//! Shared vector output fallback classifier for SVG and PostScript sinks.
//!
//! This module decides whether a page must be fully rasterized, can be emitted
//! as pure vector, or can use a **regional fallback** where simple affine
//! Image XObject `Do` operations and complete non-mask inline images are
//! embedded as bounded raster regions, and simple axial/radial shadings are
//! emitted as native gradients, while surrounding vector content (paths, text,
//! clips) is preserved natively.
//!
//! # Regional fallback scope
//!
//! Regional image embedding is permitted only when:
//! - The `Do` operand names an Image XObject (not a Form XObject).
//! - The current transform matrix at the point of the `Do` is finite and
//!   non-degenerate, so the image can be embedded as a bounded transformed
//!   raster region.
//! - The image bounds are resolvable from the CTM and the image dimensions.
//!
//! # Whole-page fallback is retained when:
//! - The page uses unsafe Form XObjects (`Do` with Subtype Form).
//! - Unsupported shadings, unsupported inline images, or unsupported pattern paint appears.
//! - An unsupported ExtGState operator (`gs`) is present (soft masks,
//!   visible non-normal blend modes, non-identity transfer functions, or
//!   non-default semantic state; PostScript additionally rejects visible
//!   fractional alpha because the sink has no native transparency operator).
//! - An Image XObject carries target-incompatible alpha metadata (for example
//!   JPX internal soft-mask data for PostScript).
//! - Semantic ordering makes regional embedding unsafe (e.g., overlapping
//!   semantic transparency groups interleaving image and vector content).
//! - Any image XObject has a degenerate or unresolvable CTM at invocation.

use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::{BlendMode, Color, ColorSpace, GraphicsState, Matrix};
use crate::engine::PageResources;
use crate::error::{Result, WellfriendError};
use crate::filters::{decode_stream_lossless, StreamDecodeStatus};
use crate::images::decoder::{ImageDecoder, RawImage};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::buffer::PixelColor;
use crate::render::cmm;
use crate::render::color::ColorSpaceHandler;
use crate::render::colorspace::{
    resolve_named_color, resolve_named_color_with_options, NamedColor, MAX_DEVICEN_COMPONENTS,
};
use crate::render::plan::{
    graphics_state_operand_refusal, marked_content_operand_refusal, path_operand_refusal,
    resource_invocation_operand_refusal, text_operand_refusal, type3_glyph_metric_operand_refusal,
};
use crate::render::text_decode::{
    decoded_glyph_strict_horizontal_advance, decoded_glyph_strict_outline, get_font_bytes,
    try_decode_text_bytes,
};
use crate::render::transform::{Transform2D, Viewport};

/// Maximum vector-safe Form XObject recursion depth for SVG/PS regional
/// fallback classification and replay.
pub const MAX_VECTOR_FORM_DEPTH: usize = 8;

/// Maximum painted source cells for vectorizing a PostScript stencil mask into
/// a native clipping path before replaying a shading pattern through `shfill`.
pub(crate) const MAX_PATTERN_STENCIL_CLIP_RECTS: usize = 4096;

/// Maximum native tile replays for SVG/PS vector-safe tiling patterns.
pub(crate) const MAX_VECTOR_TILING_PATTERN_CELLS: usize = 4096;

/// Classification of a page's content for vector output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VectorFallbackDecision {
    /// Page is pure vector: no images, no unsupported operations.
    PureVector,
    /// Page contains simple affine image XObjects that can be regionally
    /// embedded, and may contain vector-safe Form XObjects or simple shadings
    /// that can be replayed natively while preserving surrounding vector
    /// content.
    RegionalImageFallback {
        /// Names of Image XObjects that will be regionally embedded.
        image_names: Vec<String>,
        /// Number of complete inline-image sequences that will be regionally
        /// embedded in stream order.
        inline_image_count: usize,
        /// Names of Form XObjects that will be replayed as native vector
        /// sub-programs by the SVG/PS sinks.
        form_names: Vec<String>,
        /// Names of simple axial/radial shading resources that will be emitted
        /// as native gradients by the SVG/PS sinks.
        shading_names: Vec<String>,
    },
    /// Page must be fully rasterized (unsupported constructs present).
    WholePageRaster {
        /// Reason the page requires full rasterization.
        reason: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VectorOutputTarget {
    Conservative,
    Svg,
    PostScript,
}

#[derive(Clone, Copy)]
struct VectorOutputContext<'a> {
    target: VectorOutputTarget,
    viewport_scale: f64,
    form_stack: &'a [(u32, u16)],
}

/// Decoded vector-safe Form XObject program used by SVG/PS sinks.
#[derive(Debug, Clone)]
pub(crate) struct VectorFormProgram {
    pub object_number: u32,
    pub generation_number: u16,
    pub form_matrix: Matrix,
    pub bbox: Option<[f64; 4]>,
    pub resources: Option<PageResources>,
    pub ops: Vec<ContentOperation>,
}

/// Result of checking a single `Do` operation for regional-embed eligibility.
#[derive(Debug, Clone)]
pub struct ImageDoClassification {
    /// The XObject resource name from the `Do` operand.
    pub name: String,
    /// Whether this is a simple affine Image XObject eligible for
    /// regional embedding.
    pub eligible: bool,
    /// If eligible, the device-space bounding box [x, y, width, height] where
    /// the image will be placed.
    pub device_rect: Option<[f64; 4]>,
}

/// Decoded inline image payload plus placement metadata for SVG/PS regional
/// fallback. These images are stream-local, so the sinks replay them in content
/// order instead of by resource name.
#[derive(Debug, Clone)]
pub(crate) struct InlineImageRegion {
    pub placement: ImageDevicePlacement,
    pub raw: RawImage,
    pub is_mask: bool,
    pub mask_paints_ones: bool,
}

/// Device-space placement for an image painted from the PDF unit square.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImageDevicePlacement {
    /// SVG/PostScript affine matrix in top-left, y-down device space.
    pub transform: [f64; 6],
    /// Axis-aligned bounding box of the transformed unit square in device
    /// pixels: [x, y, width, height].
    pub bounds: [f64; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorShadingStop {
    pub offset: f64,
    pub rgb: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorAxialShading {
    pub coords: [f64; 4],
    pub domain: [f64; 2],
    pub c0: [f32; 3],
    pub c1: [f32; 3],
    pub stops: Vec<VectorShadingStop>,
    pub ps_function: Option<VectorPostScriptShadingFunction>,
    pub ps_color_space: Option<VectorPostScriptShadingColorSpace>,
    pub extend: [bool; 2],
    pub bbox: Option<[f64; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorRadialShading {
    pub coords: [f64; 6],
    pub domain: [f64; 2],
    pub c0: [f32; 3],
    pub c1: [f32; 3],
    pub stops: Vec<VectorShadingStop>,
    pub ps_function: Option<VectorPostScriptShadingFunction>,
    pub ps_color_space: Option<VectorPostScriptShadingColorSpace>,
    pub extend: [bool; 2],
    pub bbox: Option<[f64; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum VectorShading {
    Axial(VectorAxialShading),
    Radial(VectorRadialShading),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum VectorPostScriptShadingFunction {
    Type2(VectorPostScriptType2Function),
    Type2RgbArray(VectorPostScriptType2RgbArrayFunction),
    Type2Cmyk(VectorPostScriptType2CmykFunction),
    Type2CmykArray(VectorPostScriptType2CmykArrayFunction),
    Type2Tint(VectorPostScriptType2ComponentFunction),
    Stitching(VectorPostScriptStitchingFunction),
    StitchingCmyk(VectorPostScriptCmykStitchingFunction),
    StitchingTint(VectorPostScriptTintStitchingFunction),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptShadingColorSpace {
    pub family: VectorPostScriptNamedColorFamily,
    pub colorants: Vec<String>,
    pub alternate: VectorPostScriptAlternateColorSpace,
    pub tint_transform: VectorPostScriptShadingFunction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VectorPostScriptNamedColorFamily {
    Separation,
    DeviceN,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VectorPostScriptAlternateColorSpace {
    DeviceRgb,
    DeviceCmyk,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptType2Function {
    pub c0: [f64; 3],
    pub c1: [f64; 3],
    pub domain: [f64; 2],
    pub n: f64,
    pub range: Option<[[f64; 2]; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptType2RgbArrayFunction {
    pub channels: [VectorPostScriptType2ComponentFunction; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct VectorPostScriptType2ComponentFunction {
    pub c0: f64,
    pub c1: f64,
    pub domain: [f64; 2],
    pub n: f64,
    pub range: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptType2CmykFunction {
    pub c0: [f64; 4],
    pub c1: [f64; 4],
    pub domain: [f64; 2],
    pub n: f64,
    pub range: Option<[[f64; 2]; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptType2CmykArrayFunction {
    pub channels: [VectorPostScriptType2ComponentFunction; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptStitchingFunction {
    pub domain: [f64; 2],
    pub segments: Vec<VectorPostScriptStitchingSegment>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptStitchingSegment {
    pub bound_end: f64,
    pub encode: [f64; 2],
    pub function: VectorPostScriptType2Function,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptCmykStitchingFunction {
    pub domain: [f64; 2],
    pub segments: Vec<VectorPostScriptCmykStitchingSegment>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptCmykStitchingSegment {
    pub bound_end: f64,
    pub encode: [f64; 2],
    pub function: VectorPostScriptType2CmykFunction,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptTintStitchingFunction {
    pub domain: [f64; 2],
    pub segments: Vec<VectorPostScriptTintStitchingSegment>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPostScriptTintStitchingSegment {
    pub bound_end: f64,
    pub encode: [f64; 2],
    pub function: VectorPostScriptType2ComponentFunction,
}

impl VectorShading {
    pub(crate) fn bbox(&self) -> Option<[f64; 4]> {
        match self {
            Self::Axial(shading) => shading.bbox,
            Self::Radial(shading) => shading.bbox,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VectorPatternShading {
    pub shading: VectorShading,
    pub matrix: Matrix,
}

/// Decoded vector-safe tiling pattern program used by SVG/PS sinks.
#[derive(Debug, Clone)]
pub(crate) struct VectorTilingPatternProgram {
    pub paint_type: VectorTilingPatternPaintType,
    pub matrix: Matrix,
    pub bbox: [f64; 4],
    pub x_step: f64,
    pub y_step: f64,
    pub resources: PageResources,
    pub ops: Vec<ContentOperation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VectorTilingPatternPaintType {
    Colored,
    Uncolored,
}

/// Classify a page's operations for vector output, using resource information
/// to distinguish Image XObjects from Form XObjects.
///
/// This is the authoritative fallback decision shared by SVG and PostScript
/// sinks. It replaces the older per-sink `needs_raster_fallback` functions.
pub fn classify_page_for_vector_output(
    ops: &[ContentOperation],
    resources: &PageResources,
    viewport_scale: f64,
) -> VectorFallbackDecision {
    classify_ops_for_vector_output(
        ops,
        resources,
        viewport_scale,
        None,
        GraphicsState::default(),
        VectorOutputTarget::Conservative,
        &mut Vec::new(),
    )
}

/// Form-aware vector-output classifier used by the real SVG/PS renderers.
///
/// The legacy no-reader classifier remains conservative for standalone unit
/// tests and callers that cannot inspect Form XObject streams. This variant can
/// decode vector-safe Form XObjects, recursively classify their streams under
/// the active Form matrix/resource scope, and avoid whole-page rasterization
/// when the Form content is representable by the vector sinks.
pub(crate) fn classify_page_for_svg_output_with_reader(
    ops: &[ContentOperation],
    resources: &PageResources,
    viewport_scale: f64,
    reader: &PdfReader,
) -> VectorFallbackDecision {
    classify_ops_for_vector_output(
        ops,
        resources,
        viewport_scale,
        Some(reader),
        GraphicsState::default(),
        VectorOutputTarget::Svg,
        &mut Vec::new(),
    )
}

pub(crate) fn classify_page_for_postscript_output_with_reader(
    ops: &[ContentOperation],
    resources: &PageResources,
    viewport_scale: f64,
    reader: &PdfReader,
) -> VectorFallbackDecision {
    classify_ops_for_vector_output(
        ops,
        resources,
        viewport_scale,
        Some(reader),
        GraphicsState::default(),
        VectorOutputTarget::PostScript,
        &mut Vec::new(),
    )
}

pub(crate) fn classify_scoped_svg_vector_output(
    ops: &[ContentOperation],
    resources: &PageResources,
    viewport_scale: f64,
    reader: &PdfReader,
    initial_gs: GraphicsState,
    form_stack: &mut Vec<(u32, u16)>,
) -> VectorFallbackDecision {
    classify_ops_for_vector_output(
        ops,
        resources,
        viewport_scale,
        Some(reader),
        initial_gs,
        VectorOutputTarget::Svg,
        form_stack,
    )
}

pub(crate) fn classify_scoped_postscript_vector_output(
    ops: &[ContentOperation],
    resources: &PageResources,
    viewport_scale: f64,
    reader: &PdfReader,
    initial_gs: GraphicsState,
    form_stack: &mut Vec<(u32, u16)>,
) -> VectorFallbackDecision {
    classify_ops_for_vector_output(
        ops,
        resources,
        viewport_scale,
        Some(reader),
        initial_gs,
        VectorOutputTarget::PostScript,
        form_stack,
    )
}

fn classify_ops_for_vector_output(
    ops: &[ContentOperation],
    resources: &PageResources,
    viewport_scale: f64,
    reader: Option<&PdfReader>,
    initial_gs: GraphicsState,
    target: VectorOutputTarget,
    form_stack: &mut Vec<(u32, u16)>,
) -> VectorFallbackDecision {
    let mut image_do_ops: Vec<ImageDoClassification> = Vec::new();
    let mut inline_image_count = 0usize;
    let mut form_do_names: Vec<String> = Vec::new();
    let mut shading_names: Vec<String> = Vec::new();
    let mut regional_pattern_ops = 0usize;
    let mut gs = initial_gs;
    let graphics_state_start_depth = gs.stack_depth();
    let mut inline_begin_pending = false;
    let mut pending_inline_params: Option<Vec<Operand>> = None;
    let mut inline_data_pending_end = false;
    let mut path_has_current_point = false;
    let mut path_clip_pending = false;
    let mut marked_content_depth = 0usize;
    let mut compatibility_section_depth = 0usize;
    let mut text_object_active = false;

    for op in ops {
        if inline_data_pending_end && op.operator != "EI" {
            return VectorFallbackDecision::WholePageRaster {
                reason: "unterminated inline image",
            };
        }
        if pending_inline_params.is_some() && op.operator != "inline_image_data" {
            return VectorFallbackDecision::WholePageRaster {
                reason: "unterminated inline image",
            };
        }
        if inline_begin_pending && op.operator != "ID" {
            return VectorFallbackDecision::WholePageRaster {
                reason: "unterminated inline image",
            };
        }
        if graphics_state_operand_refusal(op).is_some() {
            return VectorFallbackDecision::WholePageRaster {
                reason: "malformed graphics-state operator",
            };
        }
        if text_operand_refusal(op).is_some() {
            return VectorFallbackDecision::WholePageRaster {
                reason: "malformed text operator",
            };
        }
        if marked_content_operand_refusal(op).is_some() {
            return VectorFallbackDecision::WholePageRaster {
                reason: "malformed marked-content operator",
            };
        }
        if path_operand_refusal(op, path_has_current_point).is_some() {
            return VectorFallbackDecision::WholePageRaster {
                reason: "malformed path operator",
            };
        }
        if resource_invocation_operand_refusal(op).is_some() {
            return VectorFallbackDecision::WholePageRaster {
                reason: "malformed resource invocation",
            };
        }
        if type3_glyph_metric_operand_refusal(op).is_some() {
            return VectorFallbackDecision::WholePageRaster {
                reason: "malformed Type 3 glyph metric operator",
            };
        }
        if op.operator == "Q" && gs.stack_depth() <= graphics_state_start_depth {
            return VectorFallbackDecision::WholePageRaster {
                reason: "malformed graphics-state operator",
            };
        }
        match op.operator.as_str() {
            "BT" => {
                if text_object_active {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "malformed text-object sequence",
                    };
                }
                text_object_active = true;
            }
            "ET" => {
                if !text_object_active {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "malformed text-object sequence",
                    };
                }
                text_object_active = false;
            }
            "Td" | "TD" | "Tm" | "T*" | "Tj" | "TJ" | "'" | "\"" if !text_object_active => {
                return VectorFallbackDecision::WholePageRaster {
                    reason: "malformed text-object sequence",
                };
            }
            _ => {}
        }
        match op.operator.as_str() {
            "BMC" | "BDC" => marked_content_depth = marked_content_depth.saturating_add(1),
            "EMC" => {
                if marked_content_depth == 0 {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "malformed marked-content operator",
                    };
                }
                marked_content_depth -= 1;
            }
            "BX" => {
                compatibility_section_depth = compatibility_section_depth.saturating_add(1);
            }
            "EX" => {
                if compatibility_section_depth == 0 {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "malformed compatibility-section operator",
                    };
                }
                compatibility_section_depth -= 1;
            }
            _ => {}
        }
        match op.operator.as_str() {
            "W" | "W*" => {
                if path_clip_pending {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "malformed clipping path sequence",
                    };
                }
                path_clip_pending = true;
            }
            "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                path_clip_pending = false;
            }
            _ => {}
        }
        match op.operator.as_str() {
            // Inline image dictionaries/data are stream-local. Complete,
            // finite affine images can be embedded regionally; malformed
            // or semantically richer shapes remain a whole-page fallback.
            "BI" => inline_begin_pending = true,
            "EI" => {
                if inline_data_pending_end {
                    inline_data_pending_end = false;
                } else {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "malformed inline image",
                    };
                }
            }
            "ID" => {
                if !inline_begin_pending
                    || pending_inline_params.is_some()
                    || inline_data_pending_end
                {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "malformed inline image",
                    };
                }
                if let Some(reason) =
                    postscript_paint_state_refusal(&gs, target, PaintRoles::fill())
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                if !classify_inline_image_params(
                    &op.operands,
                    &gs,
                    resources,
                    reader,
                    VectorOutputContext {
                        target,
                        viewport_scale,
                        form_stack,
                    },
                )
                .eligible
                {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "unsupported inline image",
                    };
                }
                pending_inline_params = Some(op.operands.clone());
                inline_begin_pending = false;
            }
            "inline_image_data" => {
                let Some(params) = pending_inline_params.take() else {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "inline image data without parameters",
                    };
                };
                let Some(data) = op.string_bytes(0) else {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "inline image without data",
                    };
                };
                if !classify_inline_image_data(
                    &params,
                    data,
                    &gs,
                    resources,
                    reader,
                    VectorOutputContext {
                        target,
                        viewport_scale,
                        form_stack,
                    },
                )
                .eligible
                {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "unsupported inline image",
                    };
                }
                inline_image_count = inline_image_count.saturating_add(1);
                inline_data_pending_end = true;
            }
            "S" | "s" if stroke_paint_uses_pattern(&gs) => {
                if let Some(reason) =
                    postscript_paint_state_refusal(&gs, target, PaintRoles::stroke())
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                if vector_pattern_stroke_supported(
                    resources,
                    reader,
                    &gs,
                    VectorOutputContext {
                        target,
                        viewport_scale,
                        form_stack,
                    },
                ) {
                    regional_pattern_ops = regional_pattern_ops.saturating_add(1);
                } else {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "pattern stroke paint",
                    };
                }
            }
            "S" | "s" => {
                if let Some(reason) =
                    postscript_paint_state_refusal(&gs, target, PaintRoles::stroke())
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                if let Some(reason) = stroke_paint_color_refusal(&gs, resources, reader) {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
            }
            "f" | "F" | "f*" if fill_paint_uses_pattern(&gs) => {
                if let Some(reason) =
                    postscript_paint_state_refusal(&gs, target, PaintRoles::fill())
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                if vector_pattern_fill_supported(
                    resources,
                    reader,
                    &gs,
                    VectorOutputContext {
                        target,
                        viewport_scale,
                        form_stack,
                    },
                ) {
                    regional_pattern_ops = regional_pattern_ops.saturating_add(1);
                } else {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "pattern fill paint",
                    };
                }
            }
            "f" | "F" | "f*" => {
                if let Some(reason) =
                    postscript_paint_state_refusal(&gs, target, PaintRoles::fill())
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                if let Some(reason) = fill_paint_color_refusal(&gs, resources, reader) {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
            }
            "B" | "B*" | "b" | "b*"
                if fill_paint_uses_pattern(&gs) || stroke_paint_uses_pattern(&gs) =>
            {
                if let Some(reason) =
                    postscript_paint_state_refusal(&gs, target, PaintRoles::fill_stroke())
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                if fill_paint_uses_pattern(&gs)
                    && !vector_pattern_fill_supported(
                        resources,
                        reader,
                        &gs,
                        VectorOutputContext {
                            target,
                            viewport_scale,
                            form_stack,
                        },
                    )
                {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "pattern fill-stroke paint",
                    };
                }
                if stroke_paint_uses_pattern(&gs)
                    && !vector_pattern_stroke_supported(
                        resources,
                        reader,
                        &gs,
                        VectorOutputContext {
                            target,
                            viewport_scale,
                            form_stack,
                        },
                    )
                {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "pattern fill-stroke paint",
                    };
                }
                if !fill_paint_uses_pattern(&gs) {
                    if let Some(reason) = fill_paint_color_refusal(&gs, resources, reader) {
                        return VectorFallbackDecision::WholePageRaster { reason };
                    }
                }
                if !stroke_paint_uses_pattern(&gs) {
                    if let Some(reason) = stroke_paint_color_refusal(&gs, resources, reader) {
                        return VectorFallbackDecision::WholePageRaster { reason };
                    }
                }
                regional_pattern_ops = regional_pattern_ops.saturating_add(1);
            }
            "B" | "B*" | "b" | "b*" => {
                if let Some(reason) =
                    postscript_paint_state_refusal(&gs, target, PaintRoles::fill_stroke())
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                if let Some(reason) = fill_paint_color_refusal(&gs, resources, reader)
                    .or_else(|| stroke_paint_color_refusal(&gs, resources, reader))
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
            }
            "sh" => {
                if let Some(reason) =
                    postscript_paint_state_refusal(&gs, target, PaintRoles::fill())
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                let Some(Operand::Name(name)) = op.operands.first() else {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "shading without resolvable name",
                    };
                };
                if load_vector_shading_for_target(resources, reader, name, target)
                    .as_ref()
                    .is_some_and(|shading| vector_shading_supported_in_state(shading, &gs))
                {
                    shading_names.push(name.clone());
                } else {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "unsupported named shading",
                    };
                }
            }
            // ExtGState: only explicit no-op/default-safe state can stay
            // vector. Visible transparency, overprint, and non-identity
            // transfer functions still force the whole-page raster path.
            "gs" => {
                let Some(Operand::Name(name)) = op.operands.first() else {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "ExtGState without resolvable name",
                    };
                };
                let Some(dict) = resources.ext_g_states.get(name) else {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "unresolved ExtGState",
                    };
                };
                if !vector_ext_g_state_is_safe_for_target(dict, target) {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "unsupported ExtGState",
                    };
                }
                let label = format!("ExtGState /{name}");
                if gs.try_apply_ext_g_state(dict, &label).is_err() {
                    return VectorFallbackDecision::WholePageRaster {
                        reason: "unsupported ExtGState",
                    };
                }
            }
            // XObject invocation: classify Image vs Form.
            "Do" => {
                let name = match op.operands.first() {
                    Some(Operand::Name(n)) => n.clone(),
                    _ => {
                        return VectorFallbackDecision::WholePageRaster {
                            reason: "Do without resolvable name",
                        };
                    }
                };
                // Check subtype from resources.
                let subtype = resources.xobject_subtypes.get(&name);
                match subtype.map(|s| s.as_str()) {
                    Some("Image") => {
                        if let Some(reason) =
                            postscript_paint_state_refusal(&gs, target, PaintRoles::fill())
                        {
                            return VectorFallbackDecision::WholePageRaster { reason };
                        }
                        // Check axis-alignment of the current CTM.
                        let classification = classify_image_do(
                            &name,
                            &gs,
                            resources,
                            reader,
                            VectorOutputContext {
                                target,
                                viewport_scale,
                                form_stack,
                            },
                        );
                        if !classification.eligible {
                            return VectorFallbackDecision::WholePageRaster {
                                reason: "degenerate or unresolvable image XObject",
                            };
                        }
                        image_do_ops.push(classification);
                    }
                    Some("Form") => {
                        let Some(reader) = reader else {
                            return VectorFallbackDecision::WholePageRaster {
                                reason: "Form XObject",
                            };
                        };
                        let Some(program) = load_vector_form_program(resources, reader, &name, &gs)
                        else {
                            return VectorFallbackDecision::WholePageRaster {
                                reason: "unresolvable Form XObject",
                            };
                        };
                        let form_key = (program.object_number, program.generation_number);
                        if form_stack.len() >= MAX_VECTOR_FORM_DEPTH {
                            return VectorFallbackDecision::WholePageRaster {
                                reason: "Form XObject recursion depth",
                            };
                        }
                        if form_stack.contains(&form_key) {
                            return VectorFallbackDecision::WholePageRaster {
                                reason: "Form XObject recursion cycle",
                            };
                        }
                        let form_resources =
                            merged_vector_resources(program.resources.as_ref(), resources);
                        let mut form_gs = gs.clone();
                        let form_t = Transform2D::from(program.form_matrix);
                        let current_t = Transform2D::from(form_gs.ctm);
                        form_gs.ctm = form_t.concat(&current_t).to_array();
                        form_stack.push(form_key);
                        let decision = classify_ops_for_vector_output(
                            &program.ops,
                            &form_resources,
                            viewport_scale,
                            Some(reader),
                            form_gs,
                            target,
                            form_stack,
                        );
                        form_stack.pop();
                        if matches!(decision, VectorFallbackDecision::WholePageRaster { .. }) {
                            return VectorFallbackDecision::WholePageRaster {
                                reason: "Form XObject contains unsupported vector content",
                            };
                        }
                        form_do_names.push(name);
                    }
                    Some("PS") => {
                        return VectorFallbackDecision::WholePageRaster {
                            reason: "PostScript XObject",
                        };
                    }
                    _ => {
                        // Unknown subtype or missing — fail closed.
                        return VectorFallbackDecision::WholePageRaster {
                            reason: "XObject with unknown or missing Subtype",
                        };
                    }
                }
            }
            // Text operations: bounded scope for vector sinks.
            "Tj" | "TJ" | "'" | "\"" => {
                if let Some(reason) =
                    postscript_paint_state_refusal(&gs, target, text_paint_roles(&gs))
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                if text_paint_uses_pattern(&gs) {
                    if vector_pattern_text_supported(
                        resources,
                        reader,
                        &gs,
                        VectorOutputContext {
                            target,
                            viewport_scale,
                            form_stack,
                        },
                    ) {
                        regional_pattern_ops = regional_pattern_ops.saturating_add(1);
                    } else {
                        return VectorFallbackDecision::WholePageRaster {
                            reason: "pattern text paint",
                        };
                    }
                }
                if let Some(reason) = text_paint_color_refusal(&gs, resources, reader) {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
                if matches!(gs.text.rendering_mode, 4..=7) {
                    let Some(reader) = reader else {
                        return VectorFallbackDecision::WholePageRaster {
                            reason: "text clipping font preflight unavailable",
                        };
                    };
                    if get_font_bytes(&gs.text.font_name, resources, reader).is_none() {
                        return VectorFallbackDecision::WholePageRaster {
                            reason: "text clipping unresolved font",
                        };
                    }
                }
                if let Err(reason) =
                    text_operator_has_vector_metric_source(op, &gs, resources, reader)
                {
                    return VectorFallbackDecision::WholePageRaster { reason };
                }
            }
            _ => {}
        }
        update_vector_path_state(op, &mut path_has_current_point);
        // Track graphics state for CTM analysis.
        gs.process(op);
    }

    if pending_inline_params.is_some() || inline_begin_pending || inline_data_pending_end {
        return VectorFallbackDecision::WholePageRaster {
            reason: "unterminated inline image",
        };
    }
    if marked_content_depth > 0 {
        return VectorFallbackDecision::WholePageRaster {
            reason: "unterminated marked-content sequence",
        };
    }
    if compatibility_section_depth > 0 {
        return VectorFallbackDecision::WholePageRaster {
            reason: "unterminated compatibility-section sequence",
        };
    }
    if path_clip_pending {
        return VectorFallbackDecision::WholePageRaster {
            reason: "unterminated clipping path sequence",
        };
    }
    if text_object_active {
        return VectorFallbackDecision::WholePageRaster {
            reason: "unterminated text-object sequence",
        };
    }

    if image_do_ops.is_empty()
        && inline_image_count == 0
        && form_do_names.is_empty()
        && shading_names.is_empty()
        && regional_pattern_ops == 0
    {
        VectorFallbackDecision::PureVector
    } else {
        VectorFallbackDecision::RegionalImageFallback {
            image_names: image_do_ops.into_iter().map(|c| c.name).collect(),
            inline_image_count,
            form_names: form_do_names,
            shading_names,
        }
    }
}

fn update_vector_path_state(op: &ContentOperation, path_has_current_point: &mut bool) {
    match op.operator.as_str() {
        "m" | "l" | "c" | "v" | "y" | "re" => *path_has_current_point = true,
        "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
            *path_has_current_point = false;
        }
        _ => {}
    }
}

#[derive(Clone, Copy)]
struct PaintRoles {
    fill: bool,
    stroke: bool,
}

impl PaintRoles {
    fn fill() -> Self {
        Self {
            fill: true,
            stroke: false,
        }
    }

    fn stroke() -> Self {
        Self {
            fill: false,
            stroke: true,
        }
    }

    fn fill_stroke() -> Self {
        Self {
            fill: true,
            stroke: true,
        }
    }
}

fn text_paint_roles(gs: &GraphicsState) -> PaintRoles {
    match gs.text.rendering_mode {
        0 | 4 => PaintRoles::fill(),
        1 | 5 => PaintRoles::stroke(),
        2 | 6 => PaintRoles::fill_stroke(),
        3 | 7 => PaintRoles {
            fill: false,
            stroke: false,
        },
        _ => PaintRoles::fill_stroke(),
    }
}

fn postscript_paint_state_refusal(
    gs: &GraphicsState,
    target: VectorOutputTarget,
    roles: PaintRoles,
) -> Option<&'static str> {
    if target != VectorOutputTarget::PostScript {
        return None;
    }
    if (roles.fill && !postscript_paint_channel_is_native(gs.fill_alpha, gs.blend_mode))
        || (roles.stroke && !postscript_paint_channel_is_native(gs.stroke_alpha, gs.blend_mode))
    {
        Some("unsupported PostScript paint alpha/blend")
    } else {
        None
    }
}

fn postscript_paint_channel_is_native(alpha: f64, blend_mode: BlendMode) -> bool {
    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
        return false;
    }
    if alpha <= f64::EPSILON {
        return true;
    }
    (alpha - 1.0).abs() <= f64::EPSILON && blend_mode == BlendMode::Normal
}

fn text_operator_has_vector_metric_source(
    op: &ContentOperation,
    gs: &GraphicsState,
    resources: &PageResources,
    reader: Option<&PdfReader>,
) -> std::result::Result<(), &'static str> {
    const REASON: &str = "text metrics unavailable for vector output";
    const OUTLINE_REASON: &str = "text glyph outline unavailable for vector output";

    let Some(reader) = reader else {
        return Err(REASON);
    };
    let strings = text_operator_string_operands(op).ok_or(REASON)?;
    if strings.is_empty() {
        return Ok(());
    }
    let font_bytes = get_font_bytes(&gs.text.font_name, resources, reader);
    for bytes in strings {
        let glyphs = try_decode_text_bytes(bytes, &gs.text.font_name, resources, reader)
            .map_err(|_| REASON)?;
        for glyph in glyphs {
            if glyph.is_vertical {
                if glyph
                    .vertical_advance
                    .filter(|advance| advance.is_finite())
                    .is_none()
                {
                    return Err(REASON);
                }
            } else if glyph.width.filter(|advance| advance.is_finite()).is_none() {
                let Some(font_bytes) = font_bytes.as_deref().filter(|bytes| !bytes.is_empty())
                else {
                    return Err(REASON);
                };
                if decoded_glyph_strict_horizontal_advance(&glyph, font_bytes).is_none() {
                    return Err(REASON);
                }
            }
            if !matches!(gs.text.rendering_mode, 3) {
                let Some(font_bytes) = font_bytes.as_deref().filter(|bytes| !bytes.is_empty())
                else {
                    return Err(OUTLINE_REASON);
                };
                if decoded_glyph_strict_outline(&glyph, font_bytes).is_none() {
                    return Err(OUTLINE_REASON);
                }
            }
        }
    }
    Ok(())
}

fn text_operator_string_operands(op: &ContentOperation) -> Option<Vec<&[u8]>> {
    match op.operator.as_str() {
        "Tj" | "'" => op.string_bytes(0).map(|bytes| vec![bytes]),
        "\"" => op.string_bytes(2).map(|bytes| vec![bytes]),
        "TJ" => op.operand(0).and_then(Operand::as_array).and_then(|items| {
            let mut strings = Vec::new();
            for item in items {
                match item {
                    Operand::String(bytes) => strings.push(bytes.as_slice()),
                    Operand::Integer(_) | Operand::Real(_) => {}
                    _ => return None,
                }
            }
            Some(strings)
        }),
        _ => Some(Vec::new()),
    }
}

fn fill_paint_uses_pattern(gs: &GraphicsState) -> bool {
    gs.fill_color_space.is_pattern()
}

fn stroke_paint_uses_pattern(gs: &GraphicsState) -> bool {
    gs.stroke_color_space.is_pattern()
}

fn text_paint_uses_pattern(gs: &GraphicsState) -> bool {
    match gs.text.rendering_mode {
        1 | 5 => stroke_paint_uses_pattern(gs),
        2 | 6 => fill_paint_uses_pattern(gs) || stroke_paint_uses_pattern(gs),
        3 | 7 => false,
        _ => fill_paint_uses_pattern(gs),
    }
}

fn fill_paint_color_refusal(
    gs: &GraphicsState,
    resources: &PageResources,
    reader: Option<&PdfReader>,
) -> Option<&'static str> {
    named_paint_color_refusal(
        &gs.fill_color,
        resources,
        reader,
        "unsupported named fill color",
    )
}

fn stroke_paint_color_refusal(
    gs: &GraphicsState,
    resources: &PageResources,
    reader: Option<&PdfReader>,
) -> Option<&'static str> {
    named_paint_color_refusal(
        &gs.stroke_color,
        resources,
        reader,
        "unsupported named stroke color",
    )
}

fn text_paint_color_refusal(
    gs: &GraphicsState,
    resources: &PageResources,
    reader: Option<&PdfReader>,
) -> Option<&'static str> {
    match gs.text.rendering_mode {
        1 | 5 => stroke_paint_color_refusal(gs, resources, reader),
        2 | 6 => fill_paint_color_refusal(gs, resources, reader)
            .or_else(|| stroke_paint_color_refusal(gs, resources, reader)),
        3 | 7 => None,
        _ => fill_paint_color_refusal(gs, resources, reader),
    }
}

fn named_paint_color_refusal(
    color: &Color,
    resources: &PageResources,
    reader: Option<&PdfReader>,
    reason: &'static str,
) -> Option<&'static str> {
    let ColorSpace::Named(name) = &color.space else {
        return None;
    };
    if name == "Pattern" {
        return None;
    }
    let Some(space_obj) = resources.color_spaces.get(name) else {
        return Some(reason);
    };
    if named_paint_color_space_is_vector_safe(space_obj, &color.components, resources, reader, 0) {
        None
    } else {
        Some(reason)
    }
}

fn named_paint_color_space_is_vector_safe(
    color_space: &PdfObject,
    components: &[f64],
    resources: &PageResources,
    reader: Option<&PdfReader>,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    let resolved = match color_space {
        PdfObject::Reference { .. } => {
            let Some(reader) = reader else {
                return false;
            };
            match reader.resolve(color_space.clone()) {
                Ok(obj) => obj,
                Err(_) => return false,
            }
        }
        other => other.clone(),
    };
    match &resolved {
        PdfObject::Name(name) => {
            device_paint_color_components_are_safe(name, components)
                || resources.color_spaces.get(name).is_some_and(|space| {
                    named_paint_color_space_is_vector_safe(
                        space,
                        components,
                        resources,
                        reader,
                        depth + 1,
                    )
                })
        }
        PdfObject::Array(_) => {
            let Some(reader) = reader else {
                return false;
            };
            matches!(
                resolve_named_color(&resolved, components, 1.0, reader),
                NamedColor::Color(_) | NamedColor::NoPaint
            )
        }
        _ => false,
    }
}

fn device_paint_color_components_are_safe(space_name: &str, components: &[f64]) -> bool {
    let Some(expected) = device_paint_color_component_count(space_name) else {
        return false;
    };
    components.len() == expected
        && components.iter().all(|component| component.is_finite())
        && ColorSpaceHandler::try_from_components(space_name, components, 1.0).is_some()
}

fn device_paint_color_component_count(space_name: &str) -> Option<usize> {
    match space_name {
        "DeviceGray" | "G" => Some(1),
        "DeviceRGB" | "RGB" | "sRGB" => Some(3),
        "DeviceCMYK" | "CMYK" => Some(4),
        _ => None,
    }
}

fn vector_pattern_text_supported(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    gs: &GraphicsState,
    context: VectorOutputContext<'_>,
) -> bool {
    match gs.text.rendering_mode {
        1 | 5 => vector_pattern_stroke_supported(resources, reader, gs, context),
        2 | 6 => {
            (!fill_paint_uses_pattern(gs)
                || vector_pattern_fill_supported(resources, reader, gs, context))
                && (!stroke_paint_uses_pattern(gs)
                    || vector_pattern_stroke_supported(resources, reader, gs, context))
        }
        3 | 7 => true,
        _ => vector_pattern_fill_supported(resources, reader, gs, context),
    }
}

fn vector_pattern_fill_supported(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    gs: &GraphicsState,
    context: VectorOutputContext<'_>,
) -> bool {
    gs.fill_pattern_name.as_deref().is_some_and(|name| {
        vector_shading_pattern_supported_in_state(resources, reader, name, gs, context.target)
            || vector_tiling_pattern_supported(resources, reader, name, gs, false, context)
    })
}

fn vector_pattern_stroke_supported(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    gs: &GraphicsState,
    context: VectorOutputContext<'_>,
) -> bool {
    gs.stroke_pattern_name.as_deref().is_some_and(|name| {
        vector_shading_pattern_supported_in_state(resources, reader, name, gs, context.target)
            || vector_tiling_pattern_supported(resources, reader, name, gs, true, context)
    })
}

fn vector_shading_pattern_supported_in_state(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    name: &str,
    gs: &GraphicsState,
    target: VectorOutputTarget,
) -> bool {
    load_vector_shading_pattern_for_target(resources, reader, name, target).is_some_and(|pattern| {
        let pattern_ctm = Transform2D::from(pattern.matrix)
            .concat(&Transform2D::from(gs.ctm))
            .to_array();
        vector_shading_supported_for_ctm(&pattern.shading, pattern_ctm)
    })
}

fn vector_tiling_pattern_supported(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    name: &str,
    gs: &GraphicsState,
    use_stroke_color: bool,
    context: VectorOutputContext<'_>,
) -> bool {
    if !matches!(
        context.target,
        VectorOutputTarget::Svg | VectorOutputTarget::PostScript
    ) {
        return false;
    }
    let Some(reader) = reader else {
        return false;
    };
    let Some(program) = load_vector_tiling_pattern(resources, reader, name) else {
        return false;
    };
    match program.paint_type {
        VectorTilingPatternPaintType::Colored => {}
        VectorTilingPatternPaintType::Uncolored => {
            let paint = if use_stroke_color {
                &gs.stroke_color
            } else {
                &gs.fill_color
            };
            if vector_uncolored_tiling_paint_color(paint).is_none()
                || program
                    .ops
                    .iter()
                    .any(vector_tiling_program_op_sets_paint_color)
            {
                return false;
            }
        }
    }
    if program
        .ops
        .iter()
        .any(vector_tiling_program_op_requires_nested_pattern)
    {
        return false;
    }
    let pattern_ctm = Transform2D::from(program.matrix).concat(&Transform2D::from(gs.ctm));
    if !transform_is_finite_and_invertible(pattern_ctm) {
        return false;
    }
    let mut tile_gs = GraphicsState::default();
    tile_gs.ctm = pattern_ctm.to_array();
    let mut scoped_form_stack = context.form_stack.to_vec();
    matches!(
        classify_ops_for_vector_output(
            &program.ops,
            &program.resources,
            context.viewport_scale,
            Some(reader),
            tile_gs,
            context.target,
            &mut scoped_form_stack,
        ),
        VectorFallbackDecision::PureVector | VectorFallbackDecision::RegionalImageFallback { .. }
    )
}

fn vector_tiling_program_op_requires_nested_pattern(op: &ContentOperation) -> bool {
    match op.operator.as_str() {
        "cs" | "CS" => {
            matches!(op.operands.first(), Some(Operand::Name(name)) if name == "Pattern")
        }
        _ => false,
    }
}

fn vector_tiling_program_op_sets_paint_color(op: &ContentOperation) -> bool {
    matches!(
        op.operator.as_str(),
        "g" | "G" | "rg" | "RG" | "k" | "K" | "cs" | "CS" | "sc" | "SC" | "scn" | "SCN"
    )
}

fn vector_ext_g_state_is_safe_for_target(dict: &PdfDictionary, target: VectorOutputTarget) -> bool {
    dict.entries().all(|(key, value)| match key.as_str() {
        "Type" => matches!(value.as_name(), Some("ExtGState")),
        "LW" => value.as_number().is_some_and(|n| n.is_finite() && n >= 0.0),
        "ML" => value.as_number().is_some_and(|n| n.is_finite() && n >= 1.0),
        "FL" | "SM" => value.as_number().is_some_and(|n| n.is_finite() && n >= 0.0),
        "Font" => ext_g_state_font_is_safe(value),
        "LC" | "LJ" => matches!(value.as_integer(), Some(0..=2)),
        "D" => ext_g_state_dash_is_safe(value),
        "RI" => value.as_name().is_some(),
        "OP" | "op" => matches!(value.as_bool(), Some(false)),
        "OPM" => matches!(value.as_integer(), Some(0 | 1)),
        "SA" | "AIS" => matches!(value.as_bool(), Some(false)),
        "TK" => matches!(value.as_bool(), Some(true)),
        "CA" | "ca" => ext_g_state_alpha_is_safe(value, target),
        "BM" => ext_g_state_blend_is_safe_for_target(value, target),
        "SMask" => matches!(value.as_name(), Some("None")),
        "TR" | "TR2" => ext_g_state_transfer_is_identity(value),
        _ => false,
    })
}

fn ext_g_state_alpha_is_safe(value: &PdfObject, target: VectorOutputTarget) -> bool {
    let Some(alpha) = value.as_number() else {
        return false;
    };
    if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
        return false;
    }
    match target {
        VectorOutputTarget::Svg => true,
        VectorOutputTarget::PostScript => true,
        VectorOutputTarget::Conservative => (alpha - 1.0).abs() <= f64::EPSILON,
    }
}

fn ext_g_state_font_is_safe(value: &PdfObject) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    if items.len() != 2 {
        return false;
    }
    let Some(name) = items[0].as_name() else {
        return false;
    };
    !name.is_empty()
        && items[1]
            .as_number()
            .is_some_and(|size| size.is_finite() && size >= 0.0)
}

fn ext_g_state_dash_is_safe(value: &PdfObject) -> bool {
    let Some(items) = value.as_array() else {
        return false;
    };
    if items.len() != 2 {
        return false;
    }
    let Some(pattern_items) = items[0].as_array() else {
        return false;
    };
    if pattern_items.len() > 64 {
        return false;
    }
    let mut all_zero = !pattern_items.is_empty();
    for value in pattern_items.iter().filter_map(PdfObject::as_number) {
        if !value.is_finite() || value < 0.0 {
            return false;
        }
        all_zero &= value == 0.0;
    }
    if pattern_items.iter().any(|item| item.as_number().is_none()) || all_zero {
        return false;
    }
    items[1]
        .as_number()
        .is_some_and(|phase| phase.is_finite() && phase >= 0.0)
}

fn ext_g_state_blend_is_safe_for_target(value: &PdfObject, target: VectorOutputTarget) -> bool {
    let Some(mode) = ext_g_state_first_supported_blend(value) else {
        return false;
    };
    match target {
        VectorOutputTarget::Svg => true,
        VectorOutputTarget::PostScript => true,
        VectorOutputTarget::Conservative => mode == BlendMode::Normal,
    }
}

fn ext_g_state_first_supported_blend(value: &PdfObject) -> Option<BlendMode> {
    match value {
        PdfObject::Name(name) => BlendMode::from_supported_name(name),
        PdfObject::Array(items) => {
            if items.is_empty() {
                return None;
            }
            let mut first_supported = None;
            for item in items {
                let name = item.as_name()?;
                if first_supported.is_none() {
                    first_supported = BlendMode::from_supported_name(name);
                }
            }
            first_supported
        }
        _ => None,
    }
}

fn ext_g_state_transfer_is_identity(value: &PdfObject) -> bool {
    match value {
        PdfObject::Name(name) => name == "Identity",
        PdfObject::Array(items) if items.len() == 4 => items
            .iter()
            .all(|item| matches!(item.as_name(), Some("Identity"))),
        _ => false,
    }
}

pub(crate) fn load_vector_shading(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    name: &str,
) -> Option<VectorShading> {
    load_vector_shading_for_target(resources, reader, name, VectorOutputTarget::Svg)
}

pub(crate) fn load_vector_shading_for_postscript_output(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    name: &str,
) -> Option<VectorShading> {
    load_vector_shading_for_target(resources, reader, name, VectorOutputTarget::PostScript)
}

fn load_vector_shading_for_target(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    name: &str,
    target: VectorOutputTarget,
) -> Option<VectorShading> {
    let obj = resources.shadings.get(name)?;
    let dict = resolve_vector_dict(obj, reader)?;
    vector_shading_from_dict(resources, reader, &dict, target)
}

pub(crate) fn load_vector_shading_pattern(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    name: &str,
) -> Option<VectorPatternShading> {
    load_vector_shading_pattern_for_target(resources, reader, name, VectorOutputTarget::Svg)
}

pub(crate) fn load_vector_shading_pattern_for_postscript_output(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    name: &str,
) -> Option<VectorPatternShading> {
    load_vector_shading_pattern_for_target(resources, reader, name, VectorOutputTarget::PostScript)
}

fn load_vector_shading_pattern_for_target(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    name: &str,
    target: VectorOutputTarget,
) -> Option<VectorPatternShading> {
    let obj = resources.patterns.get(name)?;
    let pattern_dict = resolve_vector_dict(obj, reader)?;
    if pattern_dict.get_integer("PatternType")? != 2 {
        return None;
    }
    let matrix = pattern_matrix(&pattern_dict)?;
    let shading_obj = pattern_dict.get("Shading")?;
    let shading_dict = resolve_vector_dict(shading_obj, reader)?;
    let shading = vector_shading_from_dict(resources, reader, &shading_dict, target)?;
    Some(VectorPatternShading { shading, matrix })
}

pub(crate) fn load_vector_tiling_pattern(
    resources: &PageResources,
    reader: &PdfReader,
    name: &str,
) -> Option<VectorTilingPatternProgram> {
    let obj = resources.patterns.get(name)?;
    let (pattern_dict, raw) = resolve_vector_stream(obj, reader)?;
    if pattern_dict.get_integer("PatternType")? != 1 {
        return None;
    }
    let paint_type = match pattern_dict.get_integer("PaintType")? {
        1 => VectorTilingPatternPaintType::Colored,
        2 => VectorTilingPatternPaintType::Uncolored,
        _ => return None,
    };
    if !matches!(pattern_dict.get_integer("TilingType"), Some(1..=3)) {
        return None;
    }
    let bbox = vector_tiling_pattern_bbox(&pattern_dict)?;
    let x_step = vector_tiling_pattern_step(&pattern_dict, "XStep")?;
    let y_step = vector_tiling_pattern_step(&pattern_dict, "YStep")?;
    let matrix = pattern_matrix(&pattern_dict)?;
    let stream = PdfObject::Stream {
        dict: pattern_dict.clone(),
        raw,
    };
    let decoded = decode_stream_lossless(&stream, reader).ok()?;
    if decoded.status != StreamDecodeStatus::Complete {
        return None;
    }
    let ops = crate::content::ContentParser::parse(&decoded.data).ok()?;
    if paint_type == VectorTilingPatternPaintType::Uncolored
        && ops.iter().any(vector_tiling_program_op_sets_paint_color)
    {
        return None;
    }
    let pattern_resources = pattern_dict
        .get("Resources")
        .map(|obj| crate::engine::parse_resources_from_obj(obj, reader));
    let resources = merged_vector_resources(pattern_resources.as_ref(), resources);
    Some(VectorTilingPatternProgram {
        paint_type,
        matrix,
        bbox,
        x_step,
        y_step,
        resources,
        ops,
    })
}

pub(crate) fn vector_uncolored_tiling_paint_color(color: &Color) -> Option<Color> {
    if color
        .components
        .iter()
        .any(|component| !component.is_finite())
    {
        return None;
    }
    match color.components.as_slice() {
        [g] => {
            ColorSpaceHandler::try_from_components("DeviceGray", color.components.as_slice(), 1.0)?;
            Some(Color::device_gray(*g))
        }
        [r, g, b] => {
            ColorSpaceHandler::try_from_components("DeviceRGB", color.components.as_slice(), 1.0)?;
            Some(Color::device_rgb(*r, *g, *b))
        }
        [c, m, y, k] => {
            ColorSpaceHandler::try_from_components("DeviceCMYK", color.components.as_slice(), 1.0)?;
            Some(Color::device_cmyk(*c, *m, *y, *k))
        }
        _ => None,
    }
}

fn vector_shading_from_dict(
    resources: &PageResources,
    reader: Option<&PdfReader>,
    dict: &PdfDictionary,
    target: VectorOutputTarget,
) -> Option<VectorShading> {
    let shading_type = dict.get_integer("ShadingType")?;
    let bbox = match optional_strict_vector_float_array(dict, "BBox")? {
        Some(bbox)
            if bbox.len() == 4
                && (bbox[2] - bbox[0]).abs() > 1e-9
                && (bbox[3] - bbox[1]).abs() > 1e-9 =>
        {
            Some([bbox[0], bbox[1], bbox[2], bbox[3]])
        }
        Some(_) => return None,
        None => None,
    };
    let shading_domain = match optional_strict_vector_float_array(dict, "Domain")? {
        Some(domain) => vector_shading_domain(&domain)?,
        None => [0.0, 1.0],
    };
    let vector_function =
        simple_vector_shading_stops(dict, reader, resources, shading_domain, target)?;
    let stops = vector_function.stops;
    let c0 = stops.first()?.rgb;
    let c1 = stops.last()?.rgb;
    let ps_function = vector_function.ps_function;
    let ps_color_space = vector_function.ps_color_space;
    let extend = vector_function.extend;
    match shading_type {
        2 => {
            let coords = strict_vector_float_array(dict, "Coords")?;
            if coords.len() != 4 {
                return None;
            }
            if target != VectorOutputTarget::PostScript
                && extend != [true, true]
                && ((coords[2] - coords[0]).powi(2) + (coords[3] - coords[1]).powi(2)).sqrt()
                    <= 1e-9
            {
                return None;
            }
            Some(VectorShading::Axial(VectorAxialShading {
                coords: [coords[0], coords[1], coords[2], coords[3]],
                domain: shading_domain,
                c0,
                c1,
                stops,
                ps_function,
                ps_color_space,
                extend,
                bbox,
            }))
        }
        3 => {
            let coords = strict_vector_float_array(dict, "Coords")?;
            if coords.len() != 6 {
                return None;
            }
            if target != VectorOutputTarget::PostScript
                && extend != [true, true]
                && !svg_radial_extend_clip_supported(&coords)
            {
                return None;
            }
            let r0 = coords[2];
            let r1 = coords[5];
            if r0 < 0.0 || r1 < 0.0 || (r1 - r0).abs() <= 1e-9 {
                return None;
            }
            Some(VectorShading::Radial(VectorRadialShading {
                coords: [
                    coords[0], coords[1], coords[2], coords[3], coords[4], coords[5],
                ],
                domain: shading_domain,
                c0,
                c1,
                stops,
                ps_function,
                ps_color_space,
                extend,
                bbox,
            }))
        }
        _ => None,
    }
}

fn svg_radial_extend_clip_supported(coords: &[f64]) -> bool {
    if coords.len() != 6 || !coords.iter().all(|value| value.is_finite()) {
        return false;
    }
    let [x0, y0, r0, x1, y1, r1] = [
        coords[0], coords[1], coords[2], coords[3], coords[4], coords[5],
    ];
    r0 >= 0.0 && r1 > r0 && (x1 - x0).abs() <= 1e-9 && (y1 - y0).abs() <= 1e-9
}

fn vector_shading_domain(domain: &[f64]) -> Option<[f64; 2]> {
    if domain.len() != 2
        || !domain.iter().all(|value| value.is_finite())
        || (domain[0] - domain[1]).abs() <= 1e-9
    {
        return None;
    }
    Some([domain[0], domain[1]])
}

fn pattern_matrix(dict: &PdfDictionary) -> Option<Matrix> {
    let Some(value) = dict.get("Matrix") else {
        return Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    };
    let matrix = value.as_array()?;
    if matrix.len() != 6 {
        return None;
    }
    let mut values = [0.0; 6];
    for (idx, item) in matrix.iter().enumerate() {
        let value = item.as_number()?;
        if !value.is_finite() {
            return None;
        }
        values[idx] = value;
    }
    Some(values)
}

fn vector_tiling_pattern_bbox(dict: &PdfDictionary) -> Option<[f64; 4]> {
    let values = strict_vector_float_array(dict, "BBox")?;
    if values.len() != 4
        || (values[2] - values[0]).abs() <= 1e-9
        || (values[3] - values[1]).abs() <= 1e-9
    {
        return None;
    }
    Some([values[0], values[1], values[2], values[3]])
}

fn vector_tiling_pattern_step(dict: &PdfDictionary, key: &str) -> Option<f64> {
    let value = dict.get(key)?.as_number()?;
    (value.is_finite() && value > 1e-9).then_some(value)
}

struct VectorShadingStops {
    stops: Vec<VectorShadingStop>,
    ps_function: Option<VectorPostScriptShadingFunction>,
    ps_color_space: Option<VectorPostScriptShadingColorSpace>,
    extend: [bool; 2],
}

fn simple_vector_shading_stops(
    dict: &PdfDictionary,
    reader: Option<&PdfReader>,
    resources: &PageResources,
    shading_domain: [f64; 2],
    target: VectorOutputTarget,
) -> Option<VectorShadingStops> {
    let color_space = dict.get("ColorSpace").or_else(|| dict.get("CS"))?;
    let extend = vector_shading_extend(dict, target)?;
    let allow_exact_postscript = target == VectorOutputTarget::PostScript;
    let functions =
        parse_vector_shading_functions(dict.get("Function")?, reader, allow_exact_postscript)?;
    let ps_color_space = allow_exact_postscript
        .then(|| exact_postscript_named_shading_color_space(color_space, reader, resources))
        .flatten();
    let mut total_components = 0usize;
    for function in &functions {
        total_components = total_components.checked_add(function.component_count())?;
        if total_components > MAX_DEVICEN_COMPONENTS {
            return None;
        }
    }
    let mut offsets = vector_shading_stop_offsets(shading_domain, &functions)?;
    offsets.sort_by(|a, b| a.total_cmp(b));
    offsets.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
    if offsets.len() < 2 {
        return None;
    }
    let mut stops = Vec::with_capacity(offsets.len());
    for offset in offsets {
        let t = shading_domain[0] + offset * (shading_domain[1] - shading_domain[0]);
        let components = sample_vector_shading_functions(&functions, t)?;
        let rgb = vector_shading_rgb(
            color_space,
            &components,
            reader,
            resources,
            0,
            ps_color_space.is_some(),
        )?;
        stops.push(VectorShadingStop { offset, rgb });
    }
    if vector_shading_color_space_family(color_space, reader, resources, 0).as_deref()
        == Some("Indexed")
        && !vector_shading_stops_are_constant_rgb(&stops)
    {
        return None;
    }
    let ps_function = allow_exact_postscript
        .then(|| {
            exact_postscript_shading_function(
                color_space,
                &functions,
                shading_domain,
                reader,
                resources,
                ps_color_space.as_ref(),
            )
        })
        .flatten();
    if ps_color_space.is_some() && ps_function.is_none() {
        return None;
    }
    if postscript_exact_function_required(&functions) && ps_function.is_none() {
        return None;
    }
    Some(VectorShadingStops {
        stops,
        ps_function,
        ps_color_space,
        extend,
    })
}

fn exact_postscript_shading_function(
    color_space: &PdfObject,
    functions: &[VectorShadingFunction],
    shading_domain: [f64; 2],
    reader: Option<&PdfReader>,
    resources: &PageResources,
    ps_color_space: Option<&VectorPostScriptShadingColorSpace>,
) -> Option<VectorPostScriptShadingFunction> {
    if !finite_nonzero_domain(shading_domain) {
        return None;
    }
    if ps_color_space.is_some() {
        return exact_postscript_tint_shading_function(functions);
    }
    if let Some(mapping) = postscript_exact_rgb_mapping(color_space, reader, resources) {
        return match functions {
            [VectorShadingFunction::Type2(function)] => {
                exact_postscript_type2_rgb_function(function, mapping)
                    .map(VectorPostScriptShadingFunction::Type2)
            }
            [VectorShadingFunction::Stitching(function)] => {
                exact_postscript_stitching_rgb_function(function, mapping)
                    .map(VectorPostScriptShadingFunction::Stitching)
            }
            _ if matches!(mapping, PostScriptExactRgbMapping::DeviceRgb) => {
                exact_postscript_type2_function_array_rgb_function(functions)
            }
            _ => None,
        };
    }
    match vector_shading_color_space_family(color_space, reader, resources, 0)?.as_str() {
        "DeviceCMYK" | "CMYK" => match functions {
            [VectorShadingFunction::Type2(function)] => {
                exact_postscript_type2_cmyk_function(function)
                    .map(VectorPostScriptShadingFunction::Type2Cmyk)
            }
            [VectorShadingFunction::Stitching(function)] => {
                exact_postscript_stitching_cmyk_function(function)
                    .map(VectorPostScriptShadingFunction::StitchingCmyk)
            }
            _ => exact_postscript_type2_function_array_cmyk_function(functions),
        },
        _ => None,
    }
}

fn exact_postscript_tint_shading_function(
    functions: &[VectorShadingFunction],
) -> Option<VectorPostScriptShadingFunction> {
    match functions {
        [VectorShadingFunction::Type2(function)] => {
            exact_postscript_type2_component_function(function)
                .map(VectorPostScriptShadingFunction::Type2Tint)
        }
        [VectorShadingFunction::Stitching(function)] => {
            exact_postscript_stitching_tint_function(function)
                .map(VectorPostScriptShadingFunction::StitchingTint)
        }
        _ => {
            let channels = exact_type2_component_functions::<1>(functions)?;
            Some(VectorPostScriptShadingFunction::Type2Tint(channels[0]))
        }
    }
}

fn exact_postscript_named_shading_color_space(
    color_space: &PdfObject,
    reader: Option<&PdfReader>,
    resources: &PageResources,
) -> Option<VectorPostScriptShadingColorSpace> {
    let reader = reader?;
    let resolved = match color_space {
        PdfObject::Name(name) => resources.color_spaces.get(name)?,
        other => other,
    };
    let arr = resolve_vector_color_space_array(resolved, Some(reader))?;
    let family = arr.first().and_then(PdfObject::as_name)?;
    match family {
        "Separation" => {
            if arr.len() != 4 {
                return None;
            }
            let colorant = arr.get(1)?.as_name()?;
            if colorant == "None" {
                return None;
            }
            let alternate = exact_postscript_named_alternate_space(arr.get(2)?, reader)?;
            let tint_transform =
                exact_postscript_named_tint_transform(arr.get(3)?, alternate, reader)?;
            Some(VectorPostScriptShadingColorSpace {
                family: VectorPostScriptNamedColorFamily::Separation,
                colorants: vec![colorant.to_string()],
                alternate,
                tint_transform,
            })
        }
        "DeviceN" => {
            if arr.len() < 4 {
                return None;
            }
            let names = arr.get(1)?.as_array()?;
            if names.len() != 1 {
                return None;
            }
            let colorant = names[0].as_name()?;
            if colorant == "None" {
                return None;
            }
            let alternate = exact_postscript_named_alternate_space(arr.get(2)?, reader)?;
            let tint_transform =
                exact_postscript_named_tint_transform(arr.get(3)?, alternate, reader)?;
            Some(VectorPostScriptShadingColorSpace {
                family: VectorPostScriptNamedColorFamily::DeviceN,
                colorants: vec![colorant.to_string()],
                alternate,
                tint_transform,
            })
        }
        _ => None,
    }
}

fn exact_postscript_named_alternate_space(
    alternate: &PdfObject,
    reader: &PdfReader,
) -> Option<VectorPostScriptAlternateColorSpace> {
    let resolved = match alternate {
        PdfObject::Reference { .. } => reader.resolve(alternate.clone()).ok()?,
        other => other.clone(),
    };
    let family = match &resolved {
        PdfObject::Name(name) => name.as_str(),
        PdfObject::Array(items) => items.first()?.as_name()?,
        _ => return None,
    };
    match family {
        "DeviceRGB" | "RGB" | "sRGB" => Some(VectorPostScriptAlternateColorSpace::DeviceRgb),
        "DeviceCMYK" | "CMYK" => Some(VectorPostScriptAlternateColorSpace::DeviceCmyk),
        _ => None,
    }
}

fn exact_postscript_named_tint_transform(
    tint_transform: &PdfObject,
    alternate: VectorPostScriptAlternateColorSpace,
    reader: &PdfReader,
) -> Option<VectorPostScriptShadingFunction> {
    let dict = resolve_vector_dict(tint_transform, Some(reader))?;
    let function = parse_vector_shading_function(&dict, Some(reader), true)?;
    match (alternate, function) {
        (
            VectorPostScriptAlternateColorSpace::DeviceRgb,
            VectorShadingFunction::Type2(function),
        ) => exact_postscript_type2_rgb_function(&function, PostScriptExactRgbMapping::DeviceRgb)
            .map(VectorPostScriptShadingFunction::Type2),
        (
            VectorPostScriptAlternateColorSpace::DeviceRgb,
            VectorShadingFunction::Stitching(function),
        ) => {
            exact_postscript_stitching_rgb_function(&function, PostScriptExactRgbMapping::DeviceRgb)
                .map(VectorPostScriptShadingFunction::Stitching)
        }
        (
            VectorPostScriptAlternateColorSpace::DeviceCmyk,
            VectorShadingFunction::Type2(function),
        ) => exact_postscript_type2_cmyk_function(&function)
            .map(VectorPostScriptShadingFunction::Type2Cmyk),
        (
            VectorPostScriptAlternateColorSpace::DeviceCmyk,
            VectorShadingFunction::Stitching(function),
        ) => exact_postscript_stitching_cmyk_function(&function)
            .map(VectorPostScriptShadingFunction::StitchingCmyk),
    }
}

fn exact_postscript_type2_cmyk_function(
    function: &VectorType2Function,
) -> Option<VectorPostScriptType2CmykFunction> {
    if !finite_increasing_domain(function.domain) || !function.n.is_finite() || function.n < 0.0 {
        return None;
    }
    Some(VectorPostScriptType2CmykFunction {
        c0: finite_unit_cmyk(&function.c0)?,
        c1: finite_unit_cmyk(&function.c1)?,
        domain: function.domain,
        n: function.n,
        range: finite_unit_component_ranges::<4>(function.range.as_deref())?,
    })
}

fn finite_unit_cmyk(values: &[f64]) -> Option<[f64; 4]> {
    let cmyk = [
        *values.first()?,
        *values.get(1)?,
        *values.get(2)?,
        *values.get(3)?,
    ];
    (values.len() == 4
        && cmyk
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value)))
    .then_some(cmyk)
}

fn exact_postscript_type2_function_array_cmyk_function(
    functions: &[VectorShadingFunction],
) -> Option<VectorPostScriptShadingFunction> {
    let channels = exact_type2_component_functions::<4>(functions)?;
    let c0 = [
        channels[0].c0,
        channels[1].c0,
        channels[2].c0,
        channels[3].c0,
    ];
    let c1 = [
        channels[0].c1,
        channels[1].c1,
        channels[2].c1,
        channels[3].c1,
    ];
    if channels
        .iter()
        .all(|channel| (channel.n - channels[0].n).abs() <= 1e-9)
    {
        if let (Some(domain), Some(range)) = (
            component_function_domain(&channels),
            component_function_ranges(&channels),
        ) {
            return Some(VectorPostScriptShadingFunction::Type2Cmyk(
                VectorPostScriptType2CmykFunction {
                    c0,
                    c1,
                    domain,
                    n: channels[0].n,
                    range,
                },
            ));
        }
    }

    Some(VectorPostScriptShadingFunction::Type2CmykArray(
        VectorPostScriptType2CmykArrayFunction { channels },
    ))
}

#[derive(Debug, Clone, Copy)]
enum PostScriptExactRgbMapping {
    DeviceRgb,
    DeviceGray,
}

fn postscript_exact_rgb_mapping(
    color_space: &PdfObject,
    reader: Option<&PdfReader>,
    resources: &PageResources,
) -> Option<PostScriptExactRgbMapping> {
    match vector_shading_color_space_family(color_space, reader, resources, 0)?.as_str() {
        "DeviceRGB" | "RGB" => Some(PostScriptExactRgbMapping::DeviceRgb),
        "DeviceGray" | "G" => Some(PostScriptExactRgbMapping::DeviceGray),
        _ => None,
    }
}

fn exact_postscript_type2_rgb_function(
    function: &VectorType2Function,
    mapping: PostScriptExactRgbMapping,
) -> Option<VectorPostScriptType2Function> {
    if !finite_increasing_domain(function.domain) || !function.n.is_finite() || function.n < 0.0 {
        return None;
    }
    let c0 = exact_postscript_components_as_rgb(&function.c0, mapping)?;
    let c1 = exact_postscript_components_as_rgb(&function.c1, mapping)?;
    Some(VectorPostScriptType2Function {
        c0,
        c1,
        domain: function.domain,
        n: function.n,
        range: exact_postscript_rgb_ranges(function.range.as_deref(), mapping)?,
    })
}

fn exact_postscript_type2_function_array_rgb_function(
    functions: &[VectorShadingFunction],
) -> Option<VectorPostScriptShadingFunction> {
    let channels = exact_type2_component_functions::<3>(functions)?;
    let c0 = [channels[0].c0, channels[1].c0, channels[2].c0];
    let c1 = [channels[0].c1, channels[1].c1, channels[2].c1];
    let c0 = finite_unit_rgb(&c0)?;
    let c1 = finite_unit_rgb(&c1)?;
    if channels
        .iter()
        .all(|channel| (channel.n - channels[0].n).abs() <= 1e-9)
    {
        if let (Some(domain), Some(range)) = (
            component_function_domain(&channels),
            component_function_ranges(&channels),
        ) {
            return Some(VectorPostScriptShadingFunction::Type2(
                VectorPostScriptType2Function {
                    c0,
                    c1,
                    domain,
                    n: channels[0].n,
                    range,
                },
            ));
        }
    }

    Some(VectorPostScriptShadingFunction::Type2RgbArray(
        VectorPostScriptType2RgbArrayFunction {
            channels: [channels[0], channels[1], channels[2]],
        },
    ))
}

fn exact_type2_component_functions<const N: usize>(
    functions: &[VectorShadingFunction],
) -> Option<[VectorPostScriptType2ComponentFunction; N]> {
    if functions.len() != N {
        return None;
    }
    let mut channels = [VectorPostScriptType2ComponentFunction {
        c0: 0.0,
        c1: 0.0,
        domain: [0.0, 1.0],
        n: 1.0,
        range: None,
    }; N];
    for (idx, function) in functions.iter().enumerate() {
        let VectorShadingFunction::Type2(function) = function else {
            return None;
        };
        channels[idx] = exact_postscript_type2_component_function(function)?;
    }
    Some(channels)
}

fn exact_postscript_type2_component_function(
    function: &VectorType2Function,
) -> Option<VectorPostScriptType2ComponentFunction> {
    if !finite_increasing_domain(function.domain)
        || function.c0.len() != 1
        || function.c1.len() != 1
        || !function.n.is_finite()
        || function.n < 0.0
        || !(0.0..=1.0).contains(&function.c0[0])
        || !(0.0..=1.0).contains(&function.c1[0])
        || !function.c0[0].is_finite()
        || !function.c1[0].is_finite()
    {
        return None;
    }
    let range =
        finite_unit_component_ranges::<1>(function.range.as_deref())?.map(|ranges| ranges[0]);
    Some(VectorPostScriptType2ComponentFunction {
        c0: function.c0[0],
        c1: function.c1[0],
        domain: function.domain,
        n: function.n,
        range,
    })
}

fn component_function_ranges<const N: usize>(
    channels: &[VectorPostScriptType2ComponentFunction; N],
) -> Option<Option<[[f64; 2]; N]>> {
    if channels.iter().all(|channel| channel.range.is_none()) {
        return Some(None);
    }
    let mut ranges = [[0.0; 2]; N];
    for (idx, channel) in channels.iter().enumerate() {
        ranges[idx] = channel.range?;
    }
    Some(Some(ranges))
}

fn component_function_domain<const N: usize>(
    channels: &[VectorPostScriptType2ComponentFunction; N],
) -> Option<[f64; 2]> {
    let domain = channels[0].domain;
    channels
        .iter()
        .all(|channel| same_domain(channel.domain, domain))
        .then_some(domain)
}

fn exact_postscript_stitching_rgb_function(
    function: &VectorStitchingFunction,
    mapping: PostScriptExactRgbMapping,
) -> Option<VectorPostScriptStitchingFunction> {
    if !finite_increasing_domain(function.domain) || function.segments.is_empty() {
        return None;
    }
    let mut segments = Vec::with_capacity(function.segments.len());
    for segment in &function.segments {
        if !segment.input_domain.iter().all(|value| value.is_finite())
            || segment.input_domain[0] >= segment.input_domain[1]
        {
            return None;
        }
        segments.push(VectorPostScriptStitchingSegment {
            bound_end: segment.input_domain[1],
            encode: segment.encode,
            function: exact_postscript_type2_rgb_function(&segment.function, mapping)?,
        });
    }
    Some(VectorPostScriptStitchingFunction {
        domain: function.domain,
        segments,
    })
}

fn exact_postscript_stitching_tint_function(
    function: &VectorStitchingFunction,
) -> Option<VectorPostScriptTintStitchingFunction> {
    if !finite_increasing_domain(function.domain) || function.segments.is_empty() {
        return None;
    }
    let mut segments = Vec::with_capacity(function.segments.len());
    for segment in &function.segments {
        if !segment.input_domain.iter().all(|value| value.is_finite())
            || segment.input_domain[0] >= segment.input_domain[1]
        {
            return None;
        }
        segments.push(VectorPostScriptTintStitchingSegment {
            bound_end: segment.input_domain[1],
            encode: segment.encode,
            function: exact_postscript_type2_component_function(&segment.function)?,
        });
    }
    Some(VectorPostScriptTintStitchingFunction {
        domain: function.domain,
        segments,
    })
}

fn exact_postscript_stitching_cmyk_function(
    function: &VectorStitchingFunction,
) -> Option<VectorPostScriptCmykStitchingFunction> {
    if !finite_increasing_domain(function.domain) || function.segments.is_empty() {
        return None;
    }
    let mut segments = Vec::with_capacity(function.segments.len());
    for segment in &function.segments {
        if !segment.input_domain.iter().all(|value| value.is_finite())
            || segment.input_domain[0] >= segment.input_domain[1]
        {
            return None;
        }
        segments.push(VectorPostScriptCmykStitchingSegment {
            bound_end: segment.input_domain[1],
            encode: segment.encode,
            function: exact_postscript_type2_cmyk_function(&segment.function)?,
        });
    }
    Some(VectorPostScriptCmykStitchingFunction {
        domain: function.domain,
        segments,
    })
}

fn postscript_exact_function_required(functions: &[VectorShadingFunction]) -> bool {
    functions
        .iter()
        .any(VectorShadingFunction::requires_exact_postscript)
}

fn finite_nonzero_domain(domain: [f64; 2]) -> bool {
    domain[0].is_finite() && domain[1].is_finite() && (domain[0] - domain[1]).abs() > 1e-9
}

fn finite_increasing_domain(domain: [f64; 2]) -> bool {
    domain[0].is_finite() && domain[1].is_finite() && domain[0] < domain[1]
}

fn same_domain(left: [f64; 2], right: [f64; 2]) -> bool {
    (left[0] - right[0]).abs() <= 1e-9 && (left[1] - right[1]).abs() <= 1e-9
}

fn finite_unit_rgb(values: &[f64]) -> Option<[f64; 3]> {
    let rgb = [*values.first()?, *values.get(1)?, *values.get(2)?];
    rgb.iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .then_some(rgb)
}

fn finite_unit_component_ranges<const N: usize>(
    range: Option<&[[f64; 2]]>,
) -> Option<Option<[[f64; 2]; N]>> {
    let Some(range) = range else {
        return Some(None);
    };
    if range.len() != N {
        return None;
    }
    let mut exact = [[0.0; 2]; N];
    for (idx, pair) in range.iter().enumerate() {
        if !pair[0].is_finite()
            || !pair[1].is_finite()
            || pair[0] > pair[1]
            || !(0.0..=1.0).contains(&pair[0])
            || !(0.0..=1.0).contains(&pair[1])
        {
            return None;
        }
        exact[idx] = *pair;
    }
    Some(Some(exact))
}

fn exact_postscript_rgb_ranges(
    range: Option<&[[f64; 2]]>,
    mapping: PostScriptExactRgbMapping,
) -> Option<Option<[[f64; 2]; 3]>> {
    match mapping {
        PostScriptExactRgbMapping::DeviceRgb => finite_unit_component_ranges::<3>(range),
        PostScriptExactRgbMapping::DeviceGray => {
            let Some(range) = range else {
                return Some(None);
            };
            if range.len() != 1 {
                return None;
            }
            let gray_ranges = finite_unit_component_ranges::<1>(Some(range))??;
            let gray = gray_ranges[0];
            Some(Some([gray, gray, gray]))
        }
    }
}

fn exact_postscript_components_as_rgb(
    values: &[f64],
    mapping: PostScriptExactRgbMapping,
) -> Option<[f64; 3]> {
    match mapping {
        PostScriptExactRgbMapping::DeviceRgb => {
            if values.len() != 3 {
                return None;
            }
            finite_unit_rgb(values)
        }
        PostScriptExactRgbMapping::DeviceGray => {
            let gray = *values.first()?;
            (values.len() == 1 && gray.is_finite() && (0.0..=1.0).contains(&gray))
                .then_some([gray, gray, gray])
        }
    }
}

fn strict_vector_bool_pair(dict: &PdfDictionary, key: &str) -> Option<[bool; 2]> {
    let arr = dict.get(key)?.as_array()?;
    if arr.len() != 2 {
        return None;
    }
    Some([arr[0].as_bool()?, arr[1].as_bool()?])
}

fn vector_shading_extend(dict: &PdfDictionary, target: VectorOutputTarget) -> Option<[bool; 2]> {
    let extend = match dict.get("Extend") {
        Some(_) => strict_vector_bool_pair(dict, "Extend")?,
        None => [false, false],
    };
    match target {
        VectorOutputTarget::PostScript => Some(extend),
        VectorOutputTarget::Svg | VectorOutputTarget::Conservative => Some(extend),
    }
}

#[derive(Debug, Clone)]
struct VectorType2Function {
    c0: Vec<f64>,
    c1: Vec<f64>,
    domain: [f64; 2],
    n: f64,
    range: Option<Vec<[f64; 2]>>,
}

#[derive(Debug, Clone)]
struct VectorStitchingSegment {
    input_domain: [f64; 2],
    encode: [f64; 2],
    function: VectorType2Function,
}

#[derive(Debug, Clone)]
struct VectorStitchingFunction {
    domain: [f64; 2],
    segments: Vec<VectorStitchingSegment>,
    component_count: usize,
    continuous: bool,
}

#[derive(Debug, Clone)]
enum VectorShadingFunction {
    Type2(VectorType2Function),
    Stitching(VectorStitchingFunction),
}

impl VectorShadingFunction {
    fn component_count(&self) -> usize {
        match self {
            Self::Type2(function) => function.c0.len(),
            Self::Stitching(function) => function.component_count,
        }
    }

    fn add_stop_offsets(&self, shading_domain: [f64; 2], offsets: &mut Vec<f64>) -> Option<()> {
        match self {
            Self::Type2(function) => {
                for boundary in function.domain {
                    push_domain_boundary_offset(shading_domain, boundary, offsets)?;
                }
            }
            Self::Stitching(function) => {
                for boundary in function.domain {
                    push_domain_boundary_offset(shading_domain, boundary, offsets)?;
                }
                for segment in &function.segments {
                    for boundary in segment.input_domain {
                        push_domain_boundary_offset(shading_domain, boundary, offsets)?;
                    }
                }
            }
        }
        Some(())
    }

    fn sample(&self, input: f64) -> Option<Vec<f64>> {
        match self {
            Self::Type2(function) => {
                let clamped_input = input.clamp(function.domain[0], function.domain[1]);
                Some(sample_linear_type2_function(
                    &function.c0,
                    &function.c1,
                    clamped_input,
                    function.n,
                    function.range.as_deref(),
                ))
            }
            Self::Stitching(function) => function.sample(input),
        }
    }

    fn requires_exact_postscript(&self) -> bool {
        match self {
            Self::Type2(function) => (function.n - 1.0).abs() > 1e-9,
            Self::Stitching(function) => {
                !function.continuous
                    || function
                        .segments
                        .iter()
                        .any(|segment| (segment.function.n - 1.0).abs() > 1e-9)
            }
        }
    }
}

impl VectorStitchingFunction {
    fn sample(&self, input: f64) -> Option<Vec<f64>> {
        let input = input.clamp(self.domain[0], self.domain[1]);
        let segment = self
            .segments
            .iter()
            .find(|segment| input <= segment.input_domain[1] + 1e-9)
            .or_else(|| self.segments.last())?;
        sample_stitching_segment(segment, input)
    }
}

fn parse_vector_shading_functions(
    function_obj: &PdfObject,
    reader: Option<&PdfReader>,
    allow_exact_postscript: bool,
) -> Option<Vec<VectorShadingFunction>> {
    match function_obj {
        PdfObject::Array(functions) => {
            if functions.is_empty() {
                return None;
            }
            let mut parsed = Vec::with_capacity(functions.len());
            let mut total_components = 0usize;
            for item in functions {
                let function = resolve_vector_dict(item, reader)?;
                let function =
                    parse_vector_shading_function(&function, reader, allow_exact_postscript)?;
                total_components = total_components.checked_add(function.component_count())?;
                if total_components > MAX_DEVICEN_COMPONENTS {
                    return None;
                }
                parsed.push(function);
            }
            Some(parsed)
        }
        _ => {
            let function = resolve_vector_dict(function_obj, reader)?;
            Some(vec![parse_vector_shading_function(
                &function,
                reader,
                allow_exact_postscript,
            )?])
        }
    }
}

fn parse_vector_shading_function(
    function: &PdfDictionary,
    reader: Option<&PdfReader>,
    allow_exact_postscript: bool,
) -> Option<VectorShadingFunction> {
    match function.get_integer("FunctionType")? {
        2 => Some(VectorShadingFunction::Type2(parse_type2_function(
            function,
            allow_exact_postscript,
        )?)),
        3 => Some(VectorShadingFunction::Stitching(
            parse_type3_stitching_function(function, reader, allow_exact_postscript)?,
        )),
        _ => None,
    }
}

fn parse_type2_function(
    function: &PdfDictionary,
    allow_type2_exponent: bool,
) -> Option<VectorType2Function> {
    let (domain, n) = vector_type2_function_domain_and_exponent(function, allow_type2_exponent)?;
    let c0 = optional_strict_vector_float_array(function, "C0")?.unwrap_or_else(|| vec![0.0]);
    let c1 = optional_strict_vector_float_array(function, "C1")?.unwrap_or_else(|| vec![1.0]);
    if c0.is_empty() || c0.len() != c1.len() {
        return None;
    }
    let range = vector_type2_function_range(function, c0.len())?;
    Some(VectorType2Function {
        c0,
        c1,
        domain,
        n,
        range,
    })
}

fn parse_type3_stitching_function(
    function: &PdfDictionary,
    reader: Option<&PdfReader>,
    allow_discontinuous: bool,
) -> Option<VectorStitchingFunction> {
    if function.get_integer("FunctionType") != Some(3) {
        return None;
    }
    let domain = vector_stitching_domain(function)?;
    let functions = function.get("Functions")?.as_array()?;
    if functions.is_empty() {
        return None;
    }
    let bounds = optional_strict_vector_float_array(function, "Bounds")?.unwrap_or_default();
    if bounds.len() + 1 != functions.len() {
        return None;
    }
    let encode = strict_vector_float_array(function, "Encode")?;
    if encode.len() != functions.len() * 2 {
        return None;
    }

    let mut input_bounds = Vec::with_capacity(functions.len() + 1);
    input_bounds.push(domain[0]);
    let mut previous = domain[0];
    for bound in bounds {
        if bound <= previous || bound >= domain[1] || !bound.is_finite() {
            return None;
        }
        input_bounds.push(bound);
        previous = bound;
    }
    input_bounds.push(domain[1]);

    let mut segments = Vec::with_capacity(functions.len());
    let mut component_count = None;
    for (idx, item) in functions.iter().enumerate() {
        let dict = resolve_vector_dict(item, reader)?;
        let function = parse_type2_function(&dict, allow_discontinuous)?;
        let count = function.c0.len();
        if let Some(expected) = component_count {
            if expected != count {
                return None;
            }
        } else {
            component_count = Some(count);
        }
        let encoded = [encode[idx * 2], encode[idx * 2 + 1]];
        if !encoded.iter().all(|value| value.is_finite()) {
            return None;
        }
        segments.push(VectorStitchingSegment {
            input_domain: [input_bounds[idx], input_bounds[idx + 1]],
            encode: encoded,
            function,
        });
    }
    let component_count = component_count?;
    let stitching = VectorStitchingFunction {
        domain,
        segments,
        component_count,
        continuous: false,
    };
    let continuous = vector_stitching_function_is_continuous(&stitching);
    if !allow_discontinuous && !continuous {
        return None;
    }
    Some(VectorStitchingFunction {
        continuous,
        ..stitching
    })
}

fn vector_stitching_domain(function: &PdfDictionary) -> Option<[f64; 2]> {
    let domain = strict_vector_float_array(function, "Domain")?;
    if domain.len() != 2 || !domain.iter().all(|value| value.is_finite()) || domain[0] >= domain[1]
    {
        return None;
    }
    Some([domain[0], domain[1]])
}

fn vector_stitching_function_is_continuous(function: &VectorStitchingFunction) -> bool {
    function.segments.windows(2).all(|segments| {
        let Some(left) = sample_stitching_segment(&segments[0], segments[0].input_domain[1]) else {
            return false;
        };
        let Some(right) = sample_stitching_segment(&segments[1], segments[1].input_domain[0])
        else {
            return false;
        };
        left.len() == right.len()
            && left
                .iter()
                .zip(right.iter())
                .all(|(left, right)| (left - right).abs() <= 1e-7)
    })
}

fn sample_stitching_segment(segment: &VectorStitchingSegment, input: f64) -> Option<Vec<f64>> {
    let span = segment.input_domain[1] - segment.input_domain[0];
    if !span.is_finite() || span.abs() <= 1e-9 {
        return None;
    }
    let encoded = segment.encode[0]
        + (input - segment.input_domain[0]) * (segment.encode[1] - segment.encode[0]) / span;
    let encoded = encoded.clamp(segment.function.domain[0], segment.function.domain[1]);
    Some(sample_linear_type2_function(
        &segment.function.c0,
        &segment.function.c1,
        encoded,
        segment.function.n,
        segment.function.range.as_deref(),
    ))
}

fn vector_shading_stop_offsets(
    shading_domain: [f64; 2],
    functions: &[VectorShadingFunction],
) -> Option<Vec<f64>> {
    let span = shading_domain[1] - shading_domain[0];
    if !span.is_finite() || span.abs() <= 1e-9 {
        return None;
    }
    let mut offsets = vec![0.0, 1.0];
    for function in functions {
        function.add_stop_offsets(shading_domain, &mut offsets)?;
    }
    Some(offsets)
}

fn push_domain_boundary_offset(
    shading_domain: [f64; 2],
    boundary: f64,
    offsets: &mut Vec<f64>,
) -> Option<()> {
    let span = shading_domain[1] - shading_domain[0];
    if !span.is_finite() || span.abs() <= 1e-9 || !boundary.is_finite() {
        return None;
    }
    let min_t = shading_domain[0].min(shading_domain[1]);
    let max_t = shading_domain[0].max(shading_domain[1]);
    if boundary > min_t + 1e-9 && boundary < max_t - 1e-9 {
        let offset = (boundary - shading_domain[0]) / span;
        if offset.is_finite() && offset > 1e-9 && offset < 1.0 - 1e-9 {
            offsets.push(offset);
        }
    }
    Some(())
}

fn sample_vector_shading_functions(
    functions: &[VectorShadingFunction],
    input: f64,
) -> Option<Vec<f64>> {
    let mut values = Vec::new();
    for function in functions {
        if values.len() + function.component_count() > MAX_DEVICEN_COMPONENTS {
            return None;
        }
        values.extend(function.sample(input)?);
    }
    Some(values)
}

fn vector_type2_function_domain_and_exponent(
    function: &PdfDictionary,
    allow_type2_exponent: bool,
) -> Option<([f64; 2], f64)> {
    if function.get_integer("FunctionType") != Some(2) {
        return None;
    }
    let n = function.get("N").and_then(PdfObject::as_number)?;
    if !n.is_finite() || n < 0.0 || (!allow_type2_exponent && (n - 1.0).abs() > 1e-9) {
        return None;
    }
    let domain = strict_vector_float_array(function, "Domain")?;
    if domain.len() != 2 || !domain.iter().all(|value| value.is_finite()) || domain[0] >= domain[1]
    {
        return None;
    }
    Some(([domain[0], domain[1]], n))
}

fn vector_type2_function_range(
    function: &PdfDictionary,
    component_count: usize,
) -> Option<Option<Vec<[f64; 2]>>> {
    if let Some(range) = optional_strict_vector_float_array(function, "Range")? {
        if range.len() != component_count * 2 {
            return None;
        }
        let mut pairs = Vec::with_capacity(component_count);
        for chunk in range.chunks_exact(2) {
            if chunk[0] > chunk[1] {
                return None;
            }
            pairs.push([chunk[0], chunk[1]]);
        }
        return Some(Some(pairs));
    }
    Some(None)
}

fn sample_linear_type2_function(
    c0: &[f64],
    c1: &[f64],
    input: f64,
    n: f64,
    range: Option<&[[f64; 2]]>,
) -> Vec<f64> {
    let factor = input.powf(n);
    c0.iter()
        .zip(c1)
        .enumerate()
        .map(|(idx, (start, end))| {
            let value = start + factor * (end - start);
            match range {
                Some(range) => value.clamp(range[idx][0], range[idx][1]),
                None => value,
            }
        })
        .collect()
}

fn optional_strict_vector_float_array(dict: &PdfDictionary, key: &str) -> Option<Option<Vec<f64>>> {
    let Some(value) = dict.get(key) else {
        return Some(None);
    };
    let arr = value.as_array()?;
    let mut values = Vec::with_capacity(arr.len());
    for item in arr {
        let value = item.as_number()?;
        if !value.is_finite() {
            return None;
        }
        values.push(value);
    }
    Some(Some(values))
}

fn strict_vector_float_array(dict: &PdfDictionary, key: &str) -> Option<Vec<f64>> {
    optional_strict_vector_float_array(dict, key)?
}

fn vector_shading_supported_in_state(shading: &VectorShading, gs: &GraphicsState) -> bool {
    vector_shading_supported_for_ctm(shading, gs.ctm)
}

fn vector_shading_supported_for_ctm(shading: &VectorShading, ctm: Matrix) -> bool {
    match shading {
        VectorShading::Axial(_) => true,
        VectorShading::Radial(_) => transform_is_finite_and_invertible(Transform2D::from(ctm)),
    }
}

fn resolve_vector_dict(obj: &PdfObject, reader: Option<&PdfReader>) -> Option<PdfDictionary> {
    match obj {
        PdfObject::Dictionary(dict) => Some(dict.clone()),
        PdfObject::Stream { dict, .. } => Some(dict.clone()),
        PdfObject::Reference { number, generation } => {
            let reader = reader?;
            match reader.get_object(*number, *generation).ok()? {
                PdfObject::Dictionary(dict) => Some(dict),
                PdfObject::Stream { dict, .. } => Some(dict),
                _ => None,
            }
        }
        _ => None,
    }
}

fn resolve_vector_stream(obj: &PdfObject, reader: &PdfReader) -> Option<(PdfDictionary, Vec<u8>)> {
    match obj {
        PdfObject::Stream { dict, raw } => Some((dict.clone(), raw.clone())),
        PdfObject::Reference { number, generation } => {
            match reader.get_object(*number, *generation).ok()? {
                PdfObject::Stream { dict, raw } => Some((dict, raw)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn transform_is_finite_and_invertible(transform: Transform2D) -> bool {
    [
        transform.a,
        transform.b,
        transform.c,
        transform.d,
        transform.e,
        transform.f,
    ]
    .iter()
    .all(|value| value.is_finite())
        && transform.determinant().abs() > 1e-9
}

fn vector_shading_rgb(
    color_space: &PdfObject,
    components: &[f64],
    reader: Option<&PdfReader>,
    resources: &PageResources,
    depth: usize,
    allow_postscript_exact_named_tint: bool,
) -> Option<[f32; 3]> {
    if depth > 4 {
        return None;
    }
    let color_space = match color_space {
        PdfObject::Reference { .. } => reader?.resolve(color_space.clone()).ok()?,
        other => other.clone(),
    };
    match &color_space {
        PdfObject::Name(name) => vector_device_shading_rgb(name, components).or_else(|| {
            resources.color_spaces.get(name).and_then(|space| {
                vector_shading_rgb(
                    space,
                    components,
                    reader,
                    resources,
                    depth + 1,
                    allow_postscript_exact_named_tint,
                )
            })
        }),
        PdfObject::Array(items) => match items.first().and_then(PdfObject::as_name) {
            Some("CalGray") => {
                let components = exact_vector_shading_components(components, 1)?;
                vector_calibrated_shading_space_is_valid(&color_space, reader)?;
                let gray = components[0].clamp(0.0, 1.0) as f32;
                let params = cmm::cal_gray_params_from_space(&color_space, reader)?;
                Some(cmm::cal_gray_to_srgb(gray, params))
            }
            Some("CalRGB") => {
                let components = exact_vector_shading_components(components, 3)?;
                vector_calibrated_shading_space_is_valid(&color_space, reader)?;
                let params = cmm::cal_rgb_params_from_space(&color_space, reader)?;
                Some(cmm::cal_rgb_to_srgb(
                    [
                        components[0].clamp(0.0, 1.0) as f32,
                        components[1].clamp(0.0, 1.0) as f32,
                        components[2].clamp(0.0, 1.0) as f32,
                    ],
                    params,
                ))
            }
            Some("Lab") => {
                let components = exact_vector_shading_components(components, 3)?;
                vector_calibrated_shading_space_is_valid(&color_space, reader)?;
                let params = cmm::lab_params_from_space(&color_space, reader)?;
                Some(cmm::lab_to_srgb(
                    components[0] as f32,
                    components[1] as f32,
                    components[2] as f32,
                    params,
                ))
            }
            Some("ICCBased") => {
                let reader = reader?;
                let options = vector_iccbased_shading_options(vector_iccbased_component_count(
                    &color_space,
                    reader,
                )?)?;
                cmm::icc_components_to_srgb_with_options(&color_space, components, reader, options)
            }
            Some("Indexed") => {
                let reader = reader?;
                let options = vector_indexed_shading_options(&color_space, reader)?;
                match resolve_named_color_with_options(
                    &color_space,
                    components,
                    1.0,
                    reader,
                    options,
                ) {
                    NamedColor::Color(color) => Some([color.r, color.g, color.b]),
                    NamedColor::NoPaint | NamedColor::Invalid(_) | NamedColor::Unhandled => None,
                }
            }
            Some("Separation" | "DeviceN") => {
                let reader = reader?;
                if vector_named_shading_tint_is_gradient_safe(&color_space, components, reader)
                    .is_none()
                    && !allow_postscript_exact_named_tint
                {
                    return None;
                }
                match resolve_named_color(&color_space, components, 1.0, reader) {
                    NamedColor::Color(color) => Some([color.r, color.g, color.b]),
                    NamedColor::NoPaint | NamedColor::Invalid(_) | NamedColor::Unhandled => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

fn vector_named_shading_tint_is_gradient_safe(
    color_space: &PdfObject,
    components: &[f64],
    reader: &PdfReader,
) -> Option<()> {
    let arr = resolve_vector_color_space_array(color_space, Some(reader))?;
    match arr.first().and_then(PdfObject::as_name)? {
        "Separation" => {
            exact_vector_shading_components(components, 1)?;
            vector_named_shading_alternate_is_gradient_safe(arr.get(2)?, reader)?;
            vector_tint_transform_is_linear_type2(arr.get(3)?, reader)
        }
        "DeviceN" => {
            let names = arr.get(1)?.as_array()?;
            if names.len() != 1 || names[0].as_name().is_none() {
                return None;
            }
            exact_vector_shading_components(components, 1)?;
            vector_named_shading_alternate_is_gradient_safe(arr.get(2)?, reader)?;
            vector_tint_transform_is_linear_type2(arr.get(3)?, reader)
        }
        _ => None,
    }
}

fn vector_named_shading_alternate_is_gradient_safe(
    alt: &PdfObject,
    reader: &PdfReader,
) -> Option<()> {
    let resolved = match alt {
        PdfObject::Reference { .. } => reader.resolve(alt.clone()).ok()?,
        other => other.clone(),
    };
    let family = match &resolved {
        PdfObject::Name(name) => name.as_str(),
        PdfObject::Array(items) => items.first()?.as_name()?,
        _ => return None,
    };
    matches!(
        family,
        "DeviceGray" | "G" | "DeviceRGB" | "RGB" | "sRGB" | "DeviceCMYK" | "CMYK"
    )
    .then_some(())
}

fn vector_tint_transform_is_linear_type2(
    tint_transform: &PdfObject,
    reader: &PdfReader,
) -> Option<()> {
    let dict = resolve_vector_dict(tint_transform, Some(reader))?;
    parse_type2_function(&dict, false)?;
    Some(())
}

fn vector_shading_color_space_family(
    color_space: &PdfObject,
    reader: Option<&PdfReader>,
    resources: &PageResources,
    depth: usize,
) -> Option<String> {
    if depth > 4 {
        return None;
    }
    let color_space = match color_space {
        PdfObject::Reference { .. } => reader?.resolve(color_space.clone()).ok()?,
        other => other.clone(),
    };
    match color_space {
        PdfObject::Name(name) => {
            if let Some(space) = resources.color_spaces.get(&name) {
                vector_shading_color_space_family(space, reader, resources, depth + 1)
            } else {
                Some(name)
            }
        }
        PdfObject::Array(items) => items
            .first()
            .and_then(PdfObject::as_name)
            .map(str::to_string),
        _ => None,
    }
}

fn vector_shading_stops_are_constant_rgb(stops: &[VectorShadingStop]) -> bool {
    let Some(first) = stops.first().map(|stop| stop.rgb) else {
        return false;
    };
    stops.iter().all(|stop| {
        stop.rgb
            .iter()
            .zip(first.iter())
            .all(|(a, b)| (*a - *b).abs() <= 1e-6)
    })
}

fn exact_vector_shading_components(components: &[f64], expected: usize) -> Option<&[f64]> {
    (components.len() == expected && components.iter().all(|component| component.is_finite()))
        .then_some(components)
}

fn vector_iccbased_component_count(color_space: &PdfObject, reader: &PdfReader) -> Option<u8> {
    let arr = resolve_vector_color_space_array(color_space, Some(reader))?;
    if arr.len() != 2 || arr.first().and_then(PdfObject::as_name) != Some("ICCBased") {
        return None;
    }
    let profile = reader.resolve(arr.get(1)?.clone()).ok()?;
    let n = profile
        .as_stream()
        .and_then(|(dict, _)| dict.get_integer("N"))?;
    (1..=4).contains(&n).then_some(n as u8)
}

fn vector_iccbased_shading_options(component_count: u8) -> Option<cmm::ColorTransformOptions> {
    match component_count {
        1 | 3 => Some(cmm::ColorTransformOptions::default()),
        4 if cmm::native_cmm_status().available => Some(cmm::ColorTransformOptions {
            backend: cmm::ColorTransformBackend::NativeLittleCms,
            ..cmm::ColorTransformOptions::default()
        }),
        _ => None,
    }
}

fn vector_indexed_shading_options(
    color_space: &PdfObject,
    reader: &PdfReader,
) -> Option<cmm::ColorTransformOptions> {
    let arr = resolve_vector_color_space_array(color_space, Some(reader))?;
    if arr.first().and_then(PdfObject::as_name) != Some("Indexed") {
        return Some(cmm::ColorTransformOptions::default());
    }
    let Some(base) = arr.get(1) else {
        return Some(cmm::ColorTransformOptions::default());
    };
    match vector_iccbased_component_count(base, reader) {
        Some(component_count) => vector_iccbased_shading_options(component_count),
        None => Some(cmm::ColorTransformOptions::default()),
    }
}

fn vector_calibrated_shading_space_is_valid(
    color_space: &PdfObject,
    reader: Option<&PdfReader>,
) -> Option<()> {
    let arr = resolve_vector_color_space_array(color_space, reader)?;
    if arr.len() != 2 {
        return None;
    }
    let family = arr.first()?.as_name()?;
    let dict = arr
        .get(1)
        .and_then(|obj| resolve_vector_param_dict(obj, reader))?;
    match family {
        "CalGray" => {
            valid_required_xyz(&dict, "WhitePoint")?;
            valid_optional_xyz(&dict, "BlackPoint")?;
            valid_optional_positive_number(&dict, "Gamma")?;
            Some(())
        }
        "CalRGB" => {
            valid_required_xyz(&dict, "WhitePoint")?;
            valid_optional_xyz(&dict, "BlackPoint")?;
            valid_optional_positive_number_array(&dict, "Gamma", 3)?;
            valid_optional_number_array(&dict, "Matrix", 9)?;
            Some(())
        }
        "Lab" => {
            valid_required_xyz(&dict, "WhitePoint")?;
            valid_optional_xyz(&dict, "BlackPoint")?;
            valid_optional_lab_range(&dict)?;
            Some(())
        }
        _ => None,
    }
}

fn resolve_vector_color_space_array(
    color_space: &PdfObject,
    reader: Option<&PdfReader>,
) -> Option<Vec<PdfObject>> {
    let resolved = match color_space {
        PdfObject::Reference { .. } => reader?.resolve(color_space.clone()).ok()?,
        other => other.clone(),
    };
    resolved.as_array().map(|items| items.to_vec())
}

fn resolve_vector_param_dict(obj: &PdfObject, reader: Option<&PdfReader>) -> Option<PdfDictionary> {
    let resolved = match obj {
        PdfObject::Reference { .. } => reader?.resolve(obj.clone()).ok()?,
        other => other.clone(),
    };
    resolved.as_dict().cloned()
}

fn valid_required_xyz(dict: &PdfDictionary, key: &str) -> Option<()> {
    let values = numeric_array(dict.get(key)?, 3)?;
    valid_xyz(&values).then_some(())
}

fn valid_optional_xyz(dict: &PdfDictionary, key: &str) -> Option<()> {
    match dict.get(key) {
        Some(obj) => {
            let values = numeric_array(obj, 3)?;
            valid_xyz(&values).then_some(())
        }
        None => Some(()),
    }
}

fn valid_xyz(values: &[f64]) -> bool {
    values.len() == 3
        && values.iter().all(|value| value.is_finite())
        && values[0] >= 0.0
        && values[1] > 0.0
        && values[2] >= 0.0
}

fn valid_optional_positive_number(dict: &PdfDictionary, key: &str) -> Option<()> {
    match dict.get(key) {
        Some(obj) => {
            let value = obj.as_number()?;
            (value.is_finite() && value > 0.0).then_some(())
        }
        None => Some(()),
    }
}

fn valid_optional_positive_number_array(
    dict: &PdfDictionary,
    key: &str,
    expected_len: usize,
) -> Option<()> {
    match dict.get(key) {
        Some(obj) => {
            let values = numeric_array(obj, expected_len)?;
            values
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
                .then_some(())
        }
        None => Some(()),
    }
}

fn valid_optional_number_array(dict: &PdfDictionary, key: &str, expected_len: usize) -> Option<()> {
    match dict.get(key) {
        Some(obj) => {
            let values = numeric_array(obj, expected_len)?;
            values.iter().all(|value| value.is_finite()).then_some(())
        }
        None => Some(()),
    }
}

fn valid_optional_lab_range(dict: &PdfDictionary) -> Option<()> {
    match dict.get("Range") {
        Some(obj) => {
            let values = numeric_array(obj, 4)?;
            (values.iter().all(|value| value.is_finite())
                && values[0] <= values[1]
                && values[2] <= values[3])
                .then_some(())
        }
        None => Some(()),
    }
}

fn numeric_array(obj: &PdfObject, expected_len: usize) -> Option<Vec<f64>> {
    let arr = obj.as_array()?;
    if arr.len() != expected_len {
        return None;
    }
    arr.iter().map(PdfObject::as_number).collect()
}

fn vector_device_shading_rgb(color_space: &str, components: &[f64]) -> Option<[f32; 3]> {
    match color_space {
        "DeviceGray" => {
            let components = exact_vector_shading_components(components, 1)?;
            let v = components[0].clamp(0.0, 1.0) as f32;
            Some([v, v, v])
        }
        "DeviceRGB" => {
            let components = exact_vector_shading_components(components, 3)?;
            Some([
                components[0].clamp(0.0, 1.0) as f32,
                components[1].clamp(0.0, 1.0) as f32,
                components[2].clamp(0.0, 1.0) as f32,
            ])
        }
        "DeviceCMYK" => {
            let components = exact_vector_shading_components(components, 4)?;
            Some(cmm::device_cmyk_to_srgb(
                components[0].clamp(0.0, 1.0) as f32,
                components[1].clamp(0.0, 1.0) as f32,
                components[2].clamp(0.0, 1.0) as f32,
                components[3].clamp(0.0, 1.0) as f32,
            ))
        }
        _ => None,
    }
}

/// Load and parse a Form XObject program for vector sinks.
///
/// Transparency groups are accepted when either the group dictionary is an
/// explicit no-op or a conservative scan proves that isolation/knockout cannot
/// affect pixels because every paint in the form program is opaque, normal, and
/// already vector-safe. Explicit device, well-formed calibrated/profiled/indexed
/// spaces, and structurally exact tint-transform group color spaces are accepted
/// only through that same opaque/normal proof; malformed or richer non-device
/// group color spaces remain refused because they can alter compositing/color-
/// conversion semantics when transparency is observable.
pub(crate) fn load_vector_form_program(
    page_resources: &PageResources,
    reader: &PdfReader,
    name: &str,
    inherited_gs: &GraphicsState,
) -> Option<VectorFormProgram> {
    let (obj_num, gen_num) = *page_resources.xobjects.get(name)?;
    let PdfObject::Stream { dict, raw } = reader.get_object(obj_num, gen_num).ok()? else {
        return None;
    };
    if dict.get_name("Subtype") != Some("Form") {
        return None;
    }
    let bbox = extract_bbox(&dict)?;
    let stream = PdfObject::Stream {
        dict: dict.clone(),
        raw,
    };
    let decoded = decode_stream_lossless(&stream, reader).ok()?;
    if decoded.status != StreamDecodeStatus::Complete {
        return None;
    }
    let ops = crate::content::ContentParser::parse(&decoded.data).ok()?;
    let form_resources = dict
        .get("Resources")
        .map(|obj| crate::engine::parse_resources_from_obj(obj, reader));
    let scoped_resources = merged_vector_resources(form_resources.as_ref(), page_resources);
    if form_group_requires_raster(&dict, reader, &ops, &scoped_resources, inherited_gs) {
        return None;
    }
    let form_matrix = extract_form_matrix(&dict)?;
    Some(VectorFormProgram {
        object_number: obj_num,
        generation_number: gen_num,
        form_matrix,
        bbox: Some(bbox),
        resources: form_resources,
        ops,
    })
}

pub(crate) fn merged_vector_resources(
    form_res: Option<&PageResources>,
    page_res: &PageResources,
) -> PageResources {
    let Some(form_res) = form_res else {
        return page_res.clone();
    };
    let mut merged = page_res.clone();
    for (k, v) in &form_res.fonts {
        merged.fonts.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.font_references {
        merged.font_references.insert(k.clone(), *v);
    }
    for (k, v) in &form_res.xobjects {
        merged.xobjects.insert(k.clone(), *v);
    }
    for (k, v) in &form_res.xobject_subtypes {
        merged.xobject_subtypes.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.xobject_stream_dicts {
        merged.xobject_stream_dicts.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.xobject_bboxes {
        merged.xobject_bboxes.insert(k.clone(), *v);
    }
    for (k, v) in &form_res.xobject_matrices {
        merged.xobject_matrices.insert(k.clone(), *v);
    }
    for (k, v) in &form_res.color_spaces {
        merged.color_spaces.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.color_space_references {
        merged.color_space_references.insert(k.clone(), *v);
    }
    for (k, v) in &form_res.ext_g_states {
        merged.ext_g_states.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.ext_g_state_references {
        merged.ext_g_state_references.insert(k.clone(), *v);
    }
    for (k, v) in &form_res.patterns {
        merged.patterns.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.shadings {
        merged.shadings.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.properties {
        merged.properties.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.properties_references {
        merged.properties_references.insert(k.clone(), *v);
    }
    merged
}

pub(crate) fn extract_bbox(dict: &PdfDictionary) -> Option<[f64; 4]> {
    let PdfObject::Array(items) = dict.get("BBox")? else {
        return None;
    };
    if items.len() != 4 {
        return None;
    }
    let mut values = [0.0; 4];
    for (idx, item) in items.iter().enumerate() {
        let value = item.as_number()?;
        if !value.is_finite() {
            return None;
        }
        values[idx] = value;
    }
    Some(values)
}

pub(crate) fn extract_form_matrix(dict: &PdfDictionary) -> Option<Matrix> {
    let Some(value) = dict.get("Matrix") else {
        return Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    };
    let arr = value.as_array()?;
    if arr.len() != 6 {
        return None;
    }
    let mut values = [0.0; 6];
    for (idx, item) in arr.iter().enumerate() {
        let value = item.as_number()?;
        if !value.is_finite() {
            return None;
        }
        values[idx] = value;
    }
    Some(values)
}

fn form_group_requires_raster(
    dict: &PdfDictionary,
    reader: &PdfReader,
    ops: &[ContentOperation],
    resources: &PageResources,
    inherited_gs: &GraphicsState,
) -> bool {
    let Some(group) = dict.get("Group") else {
        return false;
    };
    let group = match group {
        PdfObject::Dictionary(d) => Some(d.clone()),
        PdfObject::Reference { number, generation } => {
            match reader.get_and_resolve(*number, *generation).ok() {
                Some(PdfObject::Dictionary(d)) => Some(d),
                _ => None,
            }
        }
        _ => None,
    };
    let Some(group) = group else {
        return true;
    };
    !transparency_group_is_vector_noop(&group)
        && !transparency_group_semantics_are_vector_inert(
            &group,
            ops,
            resources,
            reader,
            inherited_gs,
        )
}

fn transparency_group_is_vector_noop(group: &PdfDictionary) -> bool {
    if group.get_name("S") != Some("Transparency") {
        return false;
    }
    group.entries().all(|(key, value)| match key.as_str() {
        "Type" => matches!(value.as_name(), Some("Group")),
        "S" => matches!(value.as_name(), Some("Transparency")),
        "I" | "K" => matches!(value.as_boolean(), Some(false)),
        "CS" => false,
        _ => false,
    })
}

fn transparency_group_semantics_are_vector_inert(
    group: &PdfDictionary,
    ops: &[ContentOperation],
    resources: &PageResources,
    reader: &PdfReader,
    inherited_gs: &GraphicsState,
) -> bool {
    if group.get_name("S") != Some("Transparency") {
        return false;
    }
    let mut requires_opaque_normal_subset = false;
    for (key, value) in group.entries() {
        match key.as_str() {
            "Type" if matches!(value.as_name(), Some("Group")) => {}
            "S" if matches!(value.as_name(), Some("Transparency")) => {}
            "I" | "K" => {
                let Some(flag) = value.as_boolean() else {
                    return false;
                };
                requires_opaque_normal_subset |= flag;
            }
            "CS" => {
                if !transparency_group_color_space_is_vector_inert(value, resources, reader, 0) {
                    return false;
                }
                requires_opaque_normal_subset = true;
            }
            _ => return false,
        }
    }
    requires_opaque_normal_subset
        && form_program_is_opaque_normal_group_subset(ops, resources, inherited_gs)
}

pub(crate) fn transparency_group_color_space_is_vector_inert(
    value: &PdfObject,
    resources: &PageResources,
    reader: &PdfReader,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    let resolved = match value {
        PdfObject::Reference { .. } => match reader.resolve(value.clone()) {
            Ok(resolved) => resolved,
            Err(_) => return false,
        },
        other => other.clone(),
    };
    match &resolved {
        PdfObject::Name(name) => {
            if matches!(
                name.as_str(),
                "DeviceGray" | "G" | "DeviceRGB" | "RGB" | "sRGB" | "DeviceCMYK" | "CMYK"
            ) {
                return true;
            }
            resources.color_spaces.get(name).is_some_and(|space| {
                transparency_group_color_space_is_vector_inert(
                    space,
                    resources,
                    reader,
                    depth.saturating_add(1),
                )
            })
        }
        PdfObject::Array(items) => match items.first().and_then(PdfObject::as_name) {
            Some("DeviceGray" | "G" | "DeviceRGB" | "RGB" | "sRGB" | "DeviceCMYK" | "CMYK") => true,
            Some("CalGray" | "CalRGB" | "Lab") => {
                vector_calibrated_shading_space_is_valid(&resolved, Some(reader)).is_some()
            }
            Some("ICCBased") => vector_iccbased_component_count(&resolved, reader).is_some(),
            Some("Indexed") => {
                vector_indexed_group_color_space_is_valid(&resolved, resources, reader, depth)
            }
            Some("Separation" | "DeviceN") => {
                vector_tint_group_color_space_is_valid(&resolved, reader)
            }
            _ => false,
        },
        _ => false,
    }
}

fn vector_indexed_group_color_space_is_valid(
    color_space: &PdfObject,
    resources: &PageResources,
    reader: &PdfReader,
    depth: usize,
) -> bool {
    let Some(arr) = resolve_vector_color_space_array(color_space, Some(reader)) else {
        return false;
    };
    if arr.len() != 4 || arr.first().and_then(PdfObject::as_name) != Some("Indexed") {
        return false;
    }
    let Some(hival) = arr.get(2).and_then(PdfObject::as_integer) else {
        return false;
    };
    if !(0..=255).contains(&hival) {
        return false;
    }
    let Some(channels) = arr
        .get(1)
        .and_then(|base| vector_group_color_space_component_count(base, resources, reader, depth))
    else {
        return false;
    };
    let Some(lookup) = arr
        .get(3)
        .and_then(|lookup| vector_indexed_lookup_bytes(lookup, reader))
    else {
        return false;
    };
    let Some(entries) = (hival as usize).checked_add(1) else {
        return false;
    };
    entries
        .checked_mul(channels)
        .is_some_and(|expected| lookup.len() == expected)
}

fn vector_group_color_space_component_count(
    color_space: &PdfObject,
    resources: &PageResources,
    reader: &PdfReader,
    depth: usize,
) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    let resolved = match color_space {
        PdfObject::Reference { .. } => reader.resolve(color_space.clone()).ok()?,
        other => other.clone(),
    };
    match &resolved {
        PdfObject::Name(name) => match name.as_str() {
            "DeviceGray" | "G" => Some(1),
            "DeviceRGB" | "RGB" | "sRGB" => Some(3),
            "DeviceCMYK" | "CMYK" => Some(4),
            _ => resources.color_spaces.get(name).and_then(|space| {
                vector_group_color_space_component_count(
                    space,
                    resources,
                    reader,
                    depth.saturating_add(1),
                )
            }),
        },
        PdfObject::Array(items) => match items.first().and_then(PdfObject::as_name) {
            Some("DeviceGray" | "G") => Some(1),
            Some("DeviceRGB" | "RGB" | "sRGB") => Some(3),
            Some("DeviceCMYK" | "CMYK") => Some(4),
            Some("CalGray")
                if vector_calibrated_shading_space_is_valid(&resolved, Some(reader)).is_some() =>
            {
                Some(1)
            }
            Some("CalRGB" | "Lab")
                if vector_calibrated_shading_space_is_valid(&resolved, Some(reader)).is_some() =>
            {
                Some(3)
            }
            Some("ICCBased") => vector_iccbased_component_count(&resolved, reader).map(usize::from),
            Some("Separation") if vector_tint_group_color_space_is_valid(&resolved, reader) => {
                Some(1)
            }
            Some("DeviceN") => vector_tint_group_color_space_sample_count(&resolved)
                .filter(|_| vector_tint_group_color_space_is_valid(&resolved, reader)),
            _ => None,
        },
        _ => None,
    }
}

fn vector_tint_group_color_space_is_valid(color_space: &PdfObject, reader: &PdfReader) -> bool {
    let Some(sample_count) = vector_tint_group_color_space_sample_count(color_space) else {
        return false;
    };
    let zero = vec![0.0; sample_count];
    let full = vec![1.0; sample_count];
    vector_tint_group_sample_is_opaque(resolve_named_color(color_space, &zero, 1.0, reader))
        && vector_tint_group_sample_is_opaque(resolve_named_color(color_space, &full, 1.0, reader))
}

fn vector_tint_group_color_space_sample_count(color_space: &PdfObject) -> Option<usize> {
    let arr = color_space.as_array()?;
    match arr.first().and_then(PdfObject::as_name)? {
        "Separation" => {
            if arr.len() != 4 {
                return None;
            }
            let colorant = arr.get(1).and_then(PdfObject::as_name)?;
            if colorant == "None" {
                return None;
            }
            Some(1)
        }
        "DeviceN" => {
            if arr.len() < 4 {
                return None;
            }
            let names = arr.get(1).and_then(PdfObject::as_array)?;
            if names.is_empty()
                || names.len() > MAX_DEVICEN_COMPONENTS
                || names.iter().any(|name| name.as_name().is_none())
                || names.iter().all(|name| name.as_name() == Some("None"))
            {
                return None;
            }
            Some(names.len())
        }
        _ => None,
    }
}

fn vector_tint_group_sample_is_opaque(color: NamedColor) -> bool {
    matches!(color, NamedColor::Color(color) if color.a >= 0.999)
}

fn vector_indexed_lookup_bytes(lookup: &PdfObject, reader: &PdfReader) -> Option<Vec<u8>> {
    match lookup {
        PdfObject::String(bytes) => Some(bytes.clone()),
        PdfObject::Stream { raw, .. } => Some(raw.clone()),
        PdfObject::Reference { number, generation } => {
            match reader.get_object(*number, *generation).ok()? {
                PdfObject::String(bytes) => Some(bytes),
                PdfObject::Stream { raw, .. } => Some(raw),
                _ => None,
            }
        }
        _ => None,
    }
}

pub(crate) fn form_program_is_opaque_normal_group_subset(
    ops: &[ContentOperation],
    resources: &PageResources,
    inherited_gs: &GraphicsState,
) -> bool {
    let mut gs = inherited_gs.clone();
    for op in ops {
        match op.operator.as_str() {
            "gs" => {
                let Some(Operand::Name(name)) = op.operands.first() else {
                    return false;
                };
                let Some(dict) = resources.ext_g_states.get(name) else {
                    return false;
                };
                if !vector_ext_g_state_is_safe_for_target(dict, VectorOutputTarget::Conservative) {
                    return false;
                }
                let label = format!("ExtGState /{name}");
                if gs.try_apply_ext_g_state(dict, &label).is_err() {
                    return false;
                }
            }
            "Do" | "BI" | "ID" | "inline_image_data" | "EI" => return false,
            "sh" if !group_alpha_is_opaque(gs.fill_alpha) => {
                return false;
            }
            "S" | "s"
                if !group_alpha_is_opaque(gs.stroke_alpha) || stroke_paint_uses_pattern(&gs) =>
            {
                return false;
            }
            "f" | "F" | "f*"
                if !group_alpha_is_opaque(gs.fill_alpha) || fill_paint_uses_pattern(&gs) =>
            {
                return false;
            }
            "B" | "B*" | "b" | "b*"
                if !group_alpha_is_opaque(gs.fill_alpha)
                    || !group_alpha_is_opaque(gs.stroke_alpha)
                    || fill_paint_uses_pattern(&gs)
                    || stroke_paint_uses_pattern(&gs) =>
            {
                return false;
            }
            "Tj" | "TJ" | "'" | "\""
                if !text_group_paint_is_opaque(&gs) || text_paint_uses_pattern(&gs) =>
            {
                return false;
            }
            _ => {}
        }
        gs.process(op);
    }
    true
}

fn text_group_paint_is_opaque(gs: &GraphicsState) -> bool {
    match gs.text.rendering_mode {
        0 | 4 => group_alpha_is_opaque(gs.fill_alpha),
        1 | 5 => group_alpha_is_opaque(gs.stroke_alpha),
        2 | 6 => group_alpha_is_opaque(gs.fill_alpha) && group_alpha_is_opaque(gs.stroke_alpha),
        3 | 7 => true,
        _ => false,
    }
}

fn group_alpha_is_opaque(alpha: f64) -> bool {
    alpha.is_finite() && (alpha - 1.0).abs() <= f64::EPSILON
}

/// Check whether a `Do` invocation of a named Image XObject is eligible for
/// regional embedding: the CTM must be finite and non-degenerate so the sinks
/// can emit it as a bounded affine raster region.
fn classify_image_do(
    name: &str,
    gs: &GraphicsState,
    resources: &PageResources,
    reader: Option<&PdfReader>,
    context: VectorOutputContext<'_>,
) -> ImageDoClassification {
    let Some(bounds) = image_scaled_unit_bounds(gs, context.viewport_scale) else {
        return ImageDoClassification {
            name: name.to_string(),
            eligible: false,
            device_rect: None,
        };
    };
    if image_xobject_is_stencil_mask(name, resources)
        && !stencil_fill_paint_supported_for_target(gs, resources, reader, context)
    {
        return ImageDoClassification {
            name: name.to_string(),
            eligible: false,
            device_rect: None,
        };
    }
    if image_xobject_is_stencil_mask(name, resources)
        && context.target == VectorOutputTarget::PostScript
        && fill_paint_uses_pattern(gs)
        && !image_xobject_stencil_clip_is_bounded(name, resources)
    {
        return ImageDoClassification {
            name: name.to_string(),
            eligible: false,
            device_rect: None,
        };
    }
    if !image_xobject_metadata_supported(name, resources, reader, context) {
        return ImageDoClassification {
            name: name.to_string(),
            eligible: false,
            device_rect: None,
        };
    }
    ImageDoClassification {
        name: name.to_string(),
        eligible: true,
        device_rect: Some(bounds),
    }
}

fn image_xobject_is_stencil_mask(name: &str, resources: &PageResources) -> bool {
    resources
        .xobject_stream_dicts
        .get(name)
        .and_then(|dict| dict.get_bool("ImageMask").or_else(|| dict.get_bool("IM")))
        .unwrap_or(false)
}

fn image_xobject_metadata_supported(
    name: &str,
    resources: &PageResources,
    reader: Option<&PdfReader>,
    context: VectorOutputContext<'_>,
) -> bool {
    let Some(dict) = resources.xobject_stream_dicts.get(name) else {
        return false;
    };
    if inline_image_required_positive_u32(dict, "Width", "image XObject").is_err()
        || inline_image_required_positive_u32(dict, "Height", "image XObject").is_err()
    {
        return false;
    }
    let Some(filters) = inline_image_filter_names(dict) else {
        return false;
    };
    if !inline_filters_supported(&filters) {
        return false;
    }
    if !inline_image_bool_supported(dict, "ImageMask")
        || !inline_image_bool_supported(dict, "Interpolate")
    {
        return false;
    }
    let is_mask = inline_image_bool(dict, "ImageMask").unwrap_or(false);
    if is_mask {
        inline_image_mask_bits_per_component(dict).is_ok() && stencil_mask_paints_ones(dict).is_ok()
    } else {
        let Ok(bpc) = inline_image_bits_per_component(dict, &filters) else {
            return false;
        };
        let color_space = inline_image_effective_color_space(dict, resources, reader, false);
        image_alpha_supported_for_target(dict, &filters, context.target)
            && color_space.as_ref().is_some_and(|space| {
                inline_image_color_space_supported_for_decode(space, bpc, &filters, reader)
            })
    }
}

fn image_alpha_supported_for_target(
    _dict: &PdfDictionary,
    _filters: &[String],
    target: VectorOutputTarget,
) -> bool {
    match target {
        // PostScript cannot represent fractional alpha natively, but the
        // decoded regional emitter can preserve exact no-op, opaque, and binary
        // alpha cases and can still fall back to whole-page raster output for
        // fractional alpha after decode.
        VectorOutputTarget::PostScript => true,
        VectorOutputTarget::Svg | VectorOutputTarget::Conservative => true,
    }
}

fn image_scaled_unit_bounds(gs: &GraphicsState, viewport_scale: f64) -> Option<[f64; 4]> {
    if !viewport_scale.is_finite() || viewport_scale <= 0.0 {
        return None;
    }
    let scaled = Transform2D::scale(viewport_scale, viewport_scale);
    let transform = Transform2D::from(gs.ctm).concat(&scaled);
    image_placement_from_transform(transform).map(|placement| placement.bounds)
}

struct InlineImageClassification {
    eligible: bool,
}

#[derive(Debug, Clone)]
struct InlineImageEffectiveColorSpace {
    name: String,
    object: Option<PdfObject>,
}

fn classify_inline_image_params(
    params: &[Operand],
    gs: &GraphicsState,
    resources: &PageResources,
    reader: Option<&PdfReader>,
    context: VectorOutputContext<'_>,
) -> InlineImageClassification {
    let Ok(dict) = inline_image_params_to_dict(params) else {
        return InlineImageClassification { eligible: false };
    };
    let Some(width) = inline_image_required_positive_u32(&dict, "Width", "inline image").ok()
    else {
        return InlineImageClassification { eligible: false };
    };
    let Some(height) = inline_image_required_positive_u32(&dict, "Height", "inline image").ok()
    else {
        return InlineImageClassification { eligible: false };
    };
    if !inline_image_bool_supported(&dict, "ImageMask")
        || !inline_image_bool_supported(&dict, "Interpolate")
    {
        return InlineImageClassification { eligible: false };
    }
    let is_mask = inline_image_bool(&dict, "ImageMask").unwrap_or(false);
    let Some(filters) = inline_image_filter_names(&dict) else {
        return InlineImageClassification { eligible: false };
    };
    let bpc = match if is_mask {
        inline_image_mask_bits_per_component(&dict)
    } else {
        inline_image_bits_per_component(&dict, &filters)
    } {
        Ok(bpc) => bpc,
        Err(_) => {
            return InlineImageClassification { eligible: false };
        }
    };
    let color_space = inline_image_effective_color_space(&dict, resources, reader, is_mask);
    let color_space_ok = if is_mask {
        stencil_fill_paint_supported_for_target(gs, resources, reader, context)
            && stencil_mask_paints_ones(&dict).is_ok()
            && color_space
                .as_ref()
                .is_some_and(|space| space.name == "DeviceGray")
    } else {
        image_alpha_supported_for_target(&dict, &filters, context.target)
            && color_space.as_ref().is_some_and(|space| {
                inline_image_color_space_supported_for_decode(space, bpc, &filters, reader)
            })
    };

    InlineImageClassification {
        eligible: width > 0
            && height > 0
            && (!is_mask
                || context.target != VectorOutputTarget::PostScript
                || !fill_paint_uses_pattern(gs)
                || stencil_clip_cell_count_is_bounded(width, height))
            && color_space_ok
            && matches!(bpc, 1 | 2 | 4 | 8 | 16)
            && inline_filters_supported(&filters)
            && inline_image_decode_params(&dict).is_some()
            && image_scaled_unit_bounds(gs, context.viewport_scale).is_some(),
    }
}

fn classify_inline_image_data(
    params: &[Operand],
    data: &[u8],
    gs: &GraphicsState,
    resources: &PageResources,
    reader: Option<&PdfReader>,
    context: VectorOutputContext<'_>,
) -> InlineImageClassification {
    let base = classify_inline_image_params(params, gs, resources, reader, context);
    InlineImageClassification {
        eligible: base.eligible && !data.is_empty(),
    }
}

fn stencil_fill_paint_supported_for_target(
    gs: &GraphicsState,
    resources: &PageResources,
    reader: Option<&PdfReader>,
    context: VectorOutputContext<'_>,
) -> bool {
    if fill_paint_uses_pattern(gs) {
        matches!(
            context.target,
            VectorOutputTarget::Svg | VectorOutputTarget::PostScript
        ) && vector_pattern_fill_supported(resources, reader, gs, context)
    } else {
        fill_paint_color_refusal(gs, resources, reader).is_none()
    }
}

fn image_xobject_stencil_clip_is_bounded(name: &str, resources: &PageResources) -> bool {
    let Some(dict) = resources.xobject_stream_dicts.get(name) else {
        return false;
    };
    let Ok(width) = inline_image_required_positive_u32(dict, "Width", "image XObject") else {
        return false;
    };
    let Ok(height) = inline_image_required_positive_u32(dict, "Height", "image XObject") else {
        return false;
    };
    stencil_clip_cell_count_is_bounded(width, height)
}

fn stencil_clip_cell_count_is_bounded(width: u32, height: u32) -> bool {
    (width as usize).saturating_mul(height as usize) <= MAX_PATTERN_STENCIL_CLIP_RECTS
}

pub(crate) fn decode_inline_image_region(
    params: &[Operand],
    data: &[u8],
    gs: &GraphicsState,
    viewport: &Viewport,
    resources: &PageResources,
    reader: Option<&PdfReader>,
) -> Result<InlineImageRegion> {
    let placement = image_device_placement(gs, viewport).ok_or_else(|| {
        WellfriendError::UnsupportedFeature("inline image placement is unresolvable".to_string())
    })?;
    let dict = inline_image_params_to_dict(params)?;
    let width = inline_image_required_positive_u32(&dict, "Width", "inline image")?;
    let height = inline_image_required_positive_u32(&dict, "Height", "inline image")?;
    let is_mask = inline_image_bool_or_default(&dict, "ImageMask", false, "inline image")?;
    let _ = inline_image_bool_or_default(&dict, "Interpolate", false, "inline image")?;

    let filters = inline_image_filter_names(&dict).ok_or_else(|| {
        WellfriendError::UnsupportedFeature("unsupported inline image filter operand".to_string())
    })?;
    let bpc = if is_mask {
        inline_image_mask_bits_per_component(&dict)?
    } else {
        inline_image_bits_per_component(&dict, &filters)?
    };
    let color_space = inline_image_effective_color_space(&dict, resources, reader, is_mask);
    let color_space_ok = if is_mask {
        color_space
            .as_ref()
            .is_some_and(|space| space.name == "DeviceGray")
    } else {
        color_space.as_ref().is_some_and(|space| {
            inline_image_color_space_supported_for_decode(space, bpc, &filters, reader)
        })
    };
    if !color_space_ok {
        if !is_mask && !inline_image_contains(&dict, "ColorSpace") {
            return Err(WellfriendError::MalformedPdf(
                "inline image missing /ColorSpace".to_string(),
            ));
        }
        return Err(WellfriendError::UnsupportedFeature(
            "unsupported inline image color space".to_string(),
        ));
    }
    let color_space = color_space.expect("checked inline image color space");

    let filter_refs: Vec<&str> = filters.iter().map(String::as_str).collect();
    let decode_params = inline_image_decode_params(&dict).ok_or_else(|| {
        WellfriendError::UnsupportedFeature("unsupported inline DecodeParms operand".to_string())
    })?;
    let raw = ImageDecoder::decode_inline_with_resolved_color_space_and_param_array(
        data,
        width,
        height,
        bpc,
        &color_space.name,
        color_space.object.as_ref(),
        &filter_refs,
        &decode_params,
        &crate::filters::DecodeLimits::default(),
        reader,
        cmm::ColorTransformOptions::default(),
    )?;
    let mask_paints_ones = if is_mask {
        stencil_mask_paints_ones(&dict)?
    } else {
        true
    };
    Ok(InlineImageRegion {
        placement,
        raw,
        is_mask,
        mask_paints_ones,
    })
}

pub(crate) fn ensure_regional_raw_image(raw: &RawImage, context: &str) -> Result<()> {
    if raw.width == 0
        || raw.height == 0
        || raw.bits_per_sample != 8
        || !matches!(raw.channels, 1 | 3 | 4)
        || !raw.is_valid()
    {
        return Err(WellfriendError::MalformedPdf(format!(
            "{context}: invalid regional image {}x{} x{} channels decoded {} bytes, expected {}",
            raw.width,
            raw.height,
            raw.channels,
            raw.pixels.len(),
            raw.byte_count()
        )));
    }
    Ok(())
}

pub(crate) fn ensure_regional_stencil_mask(raw: &RawImage, context: &str) -> Result<()> {
    if raw.width == 0
        || raw.height == 0
        || raw.bits_per_sample != 8
        || raw.channels != 1
        || !raw.is_valid()
    {
        return Err(WellfriendError::MalformedPdf(format!(
            "{context}: invalid regional stencil mask {}x{} x{} channels decoded {} bytes, expected {}",
            raw.width,
            raw.height,
            raw.channels,
            raw.pixels.len(),
            raw.byte_count()
        )));
    }
    Ok(())
}

pub(crate) fn inline_stencil_mask_to_rgba(
    raw: &RawImage,
    color: PixelColor,
    paint_ones: bool,
) -> Result<RawImage> {
    ensure_regional_stencil_mask(raw, "regional inline stencil mask")?;
    let pixel_count = raw.width as usize * raw.height as usize;
    let mut pixels = Vec::with_capacity(pixel_count * 4);
    for i in 0..pixel_count {
        let sample = raw.pixels[i];
        let paint = inline_mask_sample_paints(sample, paint_ones);
        pixels.push(color[0]);
        pixels.push(color[1]);
        pixels.push(color[2]);
        pixels.push(if paint { color[3] } else { 0 });
    }
    Ok(RawImage {
        width: raw.width,
        height: raw.height,
        channels: 4,
        bits_per_sample: 8,
        pixels,
    })
}

pub(crate) fn inline_mask_sample_paints(sample: u8, paint_ones: bool) -> bool {
    if paint_ones {
        sample >= 128
    } else {
        sample < 128
    }
}

pub(crate) fn stencil_mask_paints_ones(dict: &PdfDictionary) -> Result<bool> {
    let Some(value) = inline_image_get(dict, "Decode") else {
        return Ok(true);
    };
    let Some(items) = value.as_array() else {
        return Err(WellfriendError::MalformedPdf(
            "image mask /Decode is not an array".to_string(),
        ));
    };
    decode_stencil_mask_decode_array_paints_ones(items)
}

fn decode_stencil_mask_decode_array_paints_ones(items: &[PdfObject]) -> Result<bool> {
    if items.len() != 2 {
        return Err(WellfriendError::MalformedPdf(format!(
            "image mask /Decode has {} entries, expected 2",
            items.len()
        )));
    }
    let zero = items[0].as_number().ok_or_else(|| {
        WellfriendError::MalformedPdf("image mask /Decode contains non-numeric entries".to_string())
    })?;
    let one = items[1].as_number().ok_or_else(|| {
        WellfriendError::MalformedPdf("image mask /Decode contains non-numeric entries".to_string())
    })?;
    if !zero.is_finite() || !one.is_finite() {
        return Err(WellfriendError::MalformedPdf(
            "image mask /Decode contains non-finite entries".to_string(),
        ));
    }
    Ok(one >= zero)
}

fn inline_image_effective_color_space(
    dict: &PdfDictionary,
    resources: &PageResources,
    reader: Option<&PdfReader>,
    is_mask: bool,
) -> Option<InlineImageEffectiveColorSpace> {
    let Some(color_space) = inline_image_get(dict, "ColorSpace") else {
        return is_mask.then(|| InlineImageEffectiveColorSpace {
            name: "DeviceGray".to_string(),
            object: None,
        });
    };
    inline_image_resolve_color_space(color_space, resources, reader, 0)
}

pub(crate) fn resolved_regional_image_color_space_override(
    dict: &PdfDictionary,
    resources: &PageResources,
    reader: &PdfReader,
) -> Option<(String, PdfObject)> {
    let PdfObject::Name(resource_name) = inline_image_get(dict, "ColorSpace")? else {
        return None;
    };
    let resource_obj = resources.color_spaces.get(resource_name)?.clone();
    let family = regional_image_color_space_family_name(&resource_obj, resources, reader, 0)
        .unwrap_or_else(|| canonical_regional_image_color_space_name(resource_name));
    Some((family, resource_obj))
}

fn regional_image_color_space_family_name(
    obj: &PdfObject,
    resources: &PageResources,
    reader: &PdfReader,
    depth: usize,
) -> Option<String> {
    if depth > 8 {
        return None;
    }
    let resolved = match obj {
        PdfObject::Reference { .. } => reader.resolve(obj.clone()).ok()?,
        other => other.clone(),
    };
    match resolved {
        PdfObject::Name(name) => {
            if let Some(resource_obj) = resources.color_spaces.get(&name) {
                regional_image_color_space_family_name(
                    resource_obj,
                    resources,
                    reader,
                    depth.saturating_add(1),
                )
            } else {
                Some(canonical_regional_image_color_space_name(&name))
            }
        }
        PdfObject::Array(items) => items
            .first()
            .and_then(PdfObject::as_name)
            .map(canonical_regional_image_color_space_name),
        _ => None,
    }
}

fn canonical_regional_image_color_space_name(name: &str) -> String {
    inline_image_device_color_space_name(name)
        .unwrap_or(name)
        .to_string()
}

fn inline_image_resolve_color_space(
    color_space: &PdfObject,
    resources: &PageResources,
    reader: Option<&PdfReader>,
    depth: usize,
) -> Option<InlineImageEffectiveColorSpace> {
    if depth > 4 {
        return None;
    }
    let resolved = match color_space {
        PdfObject::Reference { .. } => reader?.resolve(color_space.clone()).ok()?,
        other => other.clone(),
    };
    match &resolved {
        PdfObject::Name(name) => {
            if let Some(device) = inline_image_device_color_space_name(name) {
                return Some(InlineImageEffectiveColorSpace {
                    name: device.to_string(),
                    object: None,
                });
            }
            resources.color_spaces.get(name).and_then(|space| {
                inline_image_resolve_color_space(space, resources, reader, depth + 1)
            })
        }
        PdfObject::Array(items) => {
            let family = items.first().and_then(PdfObject::as_name)?;
            let family = inline_image_array_color_space_name(family)?;
            Some(InlineImageEffectiveColorSpace {
                name: family.to_string(),
                object: Some(resolved),
            })
        }
        _ => None,
    }
}

fn inline_image_device_color_space_name(name: &str) -> Option<&'static str> {
    match name {
        "DeviceGray" | "G" => Some("DeviceGray"),
        "DeviceRGB" | "RGB" => Some("DeviceRGB"),
        "DeviceCMYK" | "CMYK" => Some("DeviceCMYK"),
        _ => None,
    }
}

fn inline_image_array_color_space_name(name: &str) -> Option<&'static str> {
    match name {
        "CalGray" => Some("CalGray"),
        "CalRGB" => Some("CalRGB"),
        "ICCBased" => Some("ICCBased"),
        "Lab" => Some("Lab"),
        "Indexed" => Some("Indexed"),
        "Separation" => Some("Separation"),
        "DeviceN" => Some("DeviceN"),
        _ => None,
    }
}

fn inline_image_color_space_supported_for_decode(
    color_space: &InlineImageEffectiveColorSpace,
    bpc: u8,
    filters: &[String],
    reader: Option<&PdfReader>,
) -> bool {
    let has_terminal_codec = filters
        .iter()
        .any(|filter| inline_filter_is_terminal_codec(filter));
    let has_dct_terminal = inline_filters_end_with_dct(filters);
    let has_monochrome_terminal = inline_filters_end_with_monochrome_terminal(filters);
    match color_space.name.as_str() {
        "DeviceGray" => true,
        "DeviceRGB" | "DeviceCMYK" => !has_monochrome_terminal,
        "CalGray" | "CalRGB" | "ICCBased" | "Lab" => !has_terminal_codec || has_dct_terminal,
        "Indexed" => matches!(bpc, 1 | 2 | 4 | 8) && (!has_terminal_codec || has_dct_terminal),
        "Separation" => {
            (!has_terminal_codec || has_dct_terminal)
                && inline_image_tint_space_is_opaque_paintable(color_space, reader)
        }
        "DeviceN" => {
            let Some(sample_count) = inline_image_tint_space_sample_count(color_space, reader)
            else {
                return false;
            };
            (!has_terminal_codec || (has_dct_terminal && sample_count == 1))
                && inline_image_tint_space_is_opaque_paintable_for_samples(
                    color_space,
                    reader,
                    sample_count,
                )
        }
        _ => false,
    }
}

fn inline_image_tint_space_is_opaque_paintable(
    color_space: &InlineImageEffectiveColorSpace,
    reader: Option<&PdfReader>,
) -> bool {
    let Some(sample_count) = inline_image_tint_space_sample_count(color_space, reader) else {
        return false;
    };
    inline_image_tint_space_is_opaque_paintable_for_samples(color_space, reader, sample_count)
}

fn inline_image_tint_space_sample_count(
    color_space: &InlineImageEffectiveColorSpace,
    reader: Option<&PdfReader>,
) -> Option<usize> {
    reader?;
    let space_obj = color_space.object.as_ref()?;
    let arr = space_obj.as_array()?;
    match color_space.name.as_str() {
        "Separation" => {
            if arr.get(1).and_then(PdfObject::as_name) == Some("None") {
                return None;
            }
            if arr.get(2).is_none() || arr.get(3).is_none() {
                return None;
            }
            Some(1)
        }
        "DeviceN" => {
            let names = arr.get(1).and_then(PdfObject::as_array)?;
            if names.is_empty() || names.len() > MAX_DEVICEN_COMPONENTS {
                return None;
            }
            if names.iter().all(|name| name.as_name() == Some("None")) {
                return None;
            }
            if arr.get(2).is_none() || arr.get(3).is_none() {
                return None;
            }
            Some(names.len())
        }
        _ => None,
    }
}

fn inline_image_tint_space_is_opaque_paintable_for_samples(
    color_space: &InlineImageEffectiveColorSpace,
    reader: Option<&PdfReader>,
    sample_count: usize,
) -> bool {
    let Some(reader) = reader else {
        return false;
    };
    let Some(space_obj) = color_space.object.as_ref() else {
        return false;
    };
    let zero = vec![0.0; sample_count];
    let full = vec![1.0; sample_count];
    inline_named_color_sample_is_opaque(resolve_named_color(space_obj, &zero, 1.0, reader))
        && inline_named_color_sample_is_opaque(resolve_named_color(space_obj, &full, 1.0, reader))
}

fn inline_named_color_sample_is_opaque(color: NamedColor) -> bool {
    matches!(color, NamedColor::Color(color) if color.a >= 0.999)
}

fn inline_image_params_to_dict(params: &[Operand]) -> Result<PdfDictionary> {
    let mut dict = PdfDictionary::empty();
    let mut iter = params.iter().enumerate();
    while let Some((key_index, key_op)) = iter.next() {
        let Operand::Name(key) = key_op else {
            return Err(WellfriendError::MalformedPdf(format!(
                "malformed inline image parameters: key at position {key_index} is not a name"
            )));
        };
        let Some((_, value)) = iter.next() else {
            return Err(WellfriendError::MalformedPdf(format!(
                "malformed inline image parameters: /{key} has no value"
            )));
        };
        let full_key = inline_image_full_key(key);
        if dict.contains_key(full_key) {
            return Err(WellfriendError::MalformedPdf(format!(
                "malformed inline image parameters: duplicate /{full_key}"
            )));
        }
        if let Some(object) = operand_to_pdf_object(value) {
            dict.insert(full_key.to_string(), object);
        }
    }
    Ok(dict)
}

fn inline_image_required_positive_u32(dict: &PdfDictionary, key: &str, label: &str) -> Result<u32> {
    let Some(value) = inline_image_get(dict, key) else {
        return Err(WellfriendError::MalformedPdf(format!(
            "{label} missing /{key}"
        )));
    };
    let Some(number) = value.as_number() else {
        return Err(WellfriendError::MalformedPdf(format!(
            "{label} /{key} is not numeric"
        )));
    };
    if !number.is_finite() || number <= 0.0 || number.fract() != 0.0 {
        return Err(WellfriendError::MalformedPdf(format!(
            "{label} /{key} must be a positive integer"
        )));
    }
    if number > f64::from(u32::MAX) {
        return Err(WellfriendError::MalformedPdf(format!(
            "{label} /{key} exceeds renderer dimension limit"
        )));
    }
    Ok(number as u32)
}

fn inline_image_bits_per_component(dict: &PdfDictionary, filters: &[String]) -> Result<u8> {
    let Some(value) = inline_image_get(dict, "BitsPerComponent") else {
        if inline_terminal_filter_carries_sample_depth(filters) {
            return Ok(8);
        }
        return Err(WellfriendError::MalformedPdf(
            "inline image missing /BitsPerComponent".to_string(),
        ));
    };
    let Some(number) = value.as_number() else {
        return Err(WellfriendError::MalformedPdf(
            "inline image /BitsPerComponent is not numeric".to_string(),
        ));
    };
    if !number.is_finite() || number.fract() != 0.0 {
        return Err(WellfriendError::MalformedPdf(
            "inline image /BitsPerComponent must be one of 1, 2, 4, 8, or 16".to_string(),
        ));
    }
    match number as i64 {
        1 | 2 | 4 | 8 | 16 => Ok(number as u8),
        _ => Err(WellfriendError::MalformedPdf(
            "inline image /BitsPerComponent must be one of 1, 2, 4, 8, or 16".to_string(),
        )),
    }
}

fn inline_terminal_filter_carries_sample_depth(filters: &[String]) -> bool {
    inline_filters_end_with_jpx(filters)
}

fn inline_filters_end_with_jpx(filters: &[String]) -> bool {
    matches!(
        filters.last().map(String::as_str),
        Some("JPXDecode" | "JPX")
    )
}

fn inline_image_mask_bits_per_component(dict: &PdfDictionary) -> Result<u8> {
    let Some(value) = inline_image_get(dict, "BitsPerComponent") else {
        return Ok(1);
    };
    let Some(number) = value.as_number() else {
        return Err(WellfriendError::MalformedPdf(
            "inline image mask /BitsPerComponent is not numeric".to_string(),
        ));
    };
    if number.is_finite() && number.fract() == 0.0 && (number as i64) == 1 {
        Ok(1)
    } else {
        Err(WellfriendError::MalformedPdf(
            "inline image mask /BitsPerComponent must be 1".to_string(),
        ))
    }
}

fn inline_image_bool(dict: &PdfDictionary, key: &str) -> Option<bool> {
    inline_image_get(dict, key).and_then(PdfObject::as_boolean)
}

fn inline_image_bool_supported(dict: &PdfDictionary, key: &str) -> bool {
    match inline_image_get(dict, key) {
        Some(PdfObject::Boolean(_)) | None => true,
        Some(_) => false,
    }
}

fn inline_image_bool_or_default(
    dict: &PdfDictionary,
    key: &str,
    default: bool,
    context: &str,
) -> Result<bool> {
    match inline_image_get(dict, key) {
        Some(PdfObject::Boolean(value)) => Ok(*value),
        Some(_) => Err(WellfriendError::MalformedPdf(format!(
            "{context} /{key} is not boolean"
        ))),
        None => Ok(default),
    }
}

fn inline_image_contains(dict: &PdfDictionary, key: &str) -> bool {
    inline_image_get(dict, key).is_some()
}

fn inline_image_get<'a>(dict: &'a PdfDictionary, key: &str) -> Option<&'a PdfObject> {
    dict.get(key)
        .or_else(|| inline_image_short_key(key).and_then(|short| dict.get(short)))
}

fn inline_image_full_key(key: &str) -> &str {
    match key {
        "BPC" => "BitsPerComponent",
        "CS" => "ColorSpace",
        "D" => "Decode",
        "DP" => "DecodeParms",
        "F" => "Filter",
        "H" => "Height",
        "IM" => "ImageMask",
        "I" => "Interpolate",
        "W" => "Width",
        _ => key,
    }
}

fn inline_image_short_key(key: &str) -> Option<&'static str> {
    match key {
        "BitsPerComponent" => Some("BPC"),
        "ColorSpace" => Some("CS"),
        "Decode" => Some("D"),
        "DecodeParms" => Some("DP"),
        "Filter" => Some("F"),
        "Height" => Some("H"),
        "ImageMask" => Some("IM"),
        "Interpolate" => Some("I"),
        "Width" => Some("W"),
        _ => None,
    }
}

fn operand_to_pdf_object(operand: &Operand) -> Option<PdfObject> {
    match operand {
        Operand::Integer(value) => Some(PdfObject::Integer(*value)),
        Operand::Real(value) => Some(PdfObject::Real(*value)),
        Operand::Boolean(value) => Some(PdfObject::Boolean(*value)),
        Operand::Name(value) => Some(PdfObject::Name(value.clone())),
        Operand::String(value) => Some(PdfObject::String(value.clone())),
        Operand::Array(items) => Some(PdfObject::Array(
            items.iter().filter_map(operand_to_pdf_object).collect(),
        )),
        Operand::Dictionary(entries) => Some(PdfObject::Dictionary(PdfDictionary::new(
            entries
                .iter()
                .filter_map(|(key, value)| {
                    operand_to_pdf_object(value).map(|value| (key.clone(), value))
                })
                .collect(),
        ))),
    }
}

fn inline_image_filter_names(dict: &PdfDictionary) -> Option<Vec<String>> {
    match inline_image_get(dict, "Filter") {
        None => Some(Vec::new()),
        Some(PdfObject::Name(name)) => Some(vec![name.clone()]),
        Some(PdfObject::Array(items)) => items
            .iter()
            .map(|item| item.as_name().map(str::to_string))
            .collect(),
        _ => None,
    }
}

fn inline_filters_supported(filters: &[String]) -> bool {
    filters.iter().enumerate().all(|(index, filter)| {
        let known = matches!(
            filter.as_str(),
            "FlateDecode"
                | "Fl"
                | "LZWDecode"
                | "LZW"
                | "ASCIIHexDecode"
                | "AHx"
                | "ASCII85Decode"
                | "A85"
                | "RunLengthDecode"
                | "RL"
                | "DCTDecode"
                | "DCT"
                | "JPXDecode"
                | "JPX"
                | "CCITTFaxDecode"
                | "CCF"
                | "JBIG2Decode"
        );
        let terminal_codec = inline_filter_is_terminal_codec(filter);
        known && (!terminal_codec || index + 1 == filters.len())
    })
}

fn inline_filter_is_terminal_codec(filter: &str) -> bool {
    matches!(
        filter,
        "DCTDecode" | "DCT" | "JPXDecode" | "JPX" | "CCITTFaxDecode" | "CCF" | "JBIG2Decode"
    )
}

fn inline_filters_end_with_dct(filters: &[String]) -> bool {
    matches!(
        filters.last().map(String::as_str),
        Some("DCTDecode" | "DCT")
    )
}

fn inline_filters_end_with_monochrome_terminal(filters: &[String]) -> bool {
    matches!(
        filters.last().map(String::as_str),
        Some("CCITTFaxDecode" | "CCF" | "JBIG2Decode")
    )
}

fn inline_image_decode_params(dict: &PdfDictionary) -> Option<Vec<Option<PdfDictionary>>> {
    let filter_count = inline_image_filter_names(dict)?.len();
    let Some(value) = inline_image_get(dict, "DecodeParms") else {
        return Some(vec![None; filter_count]);
    };
    match value {
        PdfObject::Dictionary(params) if filter_count > 0 => {
            let mut out = vec![None; filter_count];
            out[0] = Some(params.clone());
            Some(out)
        }
        PdfObject::Array(items) if items.len() == filter_count => items
            .iter()
            .map(|item| match item {
                PdfObject::Dictionary(dict) => Some(Some(dict.clone())),
                PdfObject::Null => Some(None),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

pub(crate) fn image_device_placement(
    gs: &GraphicsState,
    viewport: &Viewport,
) -> Option<ImageDevicePlacement> {
    let transform = Transform2D::from(gs.ctm).concat(&viewport.to_transform());
    image_placement_from_transform(transform)
}

fn image_placement_from_transform(transform: Transform2D) -> Option<ImageDevicePlacement> {
    if ![
        transform.a,
        transform.b,
        transform.c,
        transform.d,
        transform.e,
        transform.f,
    ]
    .iter()
    .all(|value| value.is_finite())
        || transform.determinant().abs() < 1e-6
    {
        return None;
    }

    let corners = [
        transform.transform_point(0.0, 0.0),
        transform.transform_point(1.0, 0.0),
        transform.transform_point(0.0, 1.0),
        transform.transform_point(1.0, 1.0),
    ];
    let (mut min_x, mut min_y) = corners[0];
    let (mut max_x, mut max_y) = corners[0];
    for &(x, y) in corners.iter().skip(1) {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    let width = max_x - min_x;
    let height = max_y - min_y;
    if width < 1.0 || height < 1.0 || !width.is_finite() || !height.is_finite() {
        return None;
    }

    Some(ImageDevicePlacement {
        transform: transform.to_array(),
        bounds: [min_x, min_y, width, height],
    })
}

/// Compute the device-space rectangle for an axis-aligned image `Do` operation,
/// accounting for the PDF bottom-up to device top-down coordinate flip.
///
/// Returns `(x, y, width, height)` in device-pixel coordinates (top-left origin,
/// y-down — the same space both the SVG and PS sinks emit geometry in).
pub fn image_device_rect(
    gs: &GraphicsState,
    viewport_scale: f64,
    page_height_px: f64,
) -> Option<[f64; 4]> {
    let ctm = gs.ctm;
    let shear_threshold = 1e-6;
    if ctm[1].abs() >= shear_threshold || ctm[2].abs() >= shear_threshold {
        return None;
    }

    let w = ctm[0].abs() * viewport_scale;
    let h = ctm[3].abs() * viewport_scale;

    // The origin in PDF user space (bottom-left up). ctm[4], ctm[5] give the
    // bottom-left corner of the image placement in user space.
    // In device space (y-down from top-left), we flip:
    //   device_y = page_height_px - (user_y + image_height_in_device)
    // But we need to be careful: ctm[3] can be negative (common for images),
    // meaning the image is flipped vertically in PDF user space.
    let (user_x, user_y_bottom) = (ctm[4] * viewport_scale, ctm[5] * viewport_scale);

    // If ctm[3] < 0, the image is painted "upside down" from the PDF origin,
    // which is the normal convention (images have origin at top-left of their
    // data, ctm[3] is negative to flip them into the page's bottom-up space).
    let device_y_top = if ctm[3] < 0.0 {
        // Normal case: image data top = user_y_bottom (which is actually the
        // top of the image in device-y-down space after page flip).
        page_height_px - user_y_bottom
    } else {
        // Unusual: positive ctm[3] means the image is not flipped. The top of
        // the image in user space is user_y_bottom + h/viewport_scale * viewport_scale = user_y_bottom + h.
        page_height_px - user_y_bottom - h
    };

    let device_x = if ctm[0] < 0.0 { user_x - w } else { user_x };

    if w < 1.0
        || h < 1.0
        || !w.is_finite()
        || !h.is_finite()
        || !device_x.is_finite()
        || !device_y_top.is_finite()
    {
        return None;
    }

    Some([device_x, device_y_top, w, h])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_do_op(name: &str) -> ContentOperation {
        ContentOperation::new("Do", vec![Operand::Name(name.to_string())])
    }

    fn make_path_ops() -> Vec<ContentOperation> {
        vec![
            ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
            ContentOperation::new("l", vec![Operand::Real(100.0), Operand::Real(0.0)]),
            ContentOperation::new("l", vec![Operand::Real(100.0), Operand::Real(100.0)]),
            ContentOperation::new("f", vec![]),
        ]
    }

    fn make_stroke_path_ops() -> Vec<ContentOperation> {
        vec![
            ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
            ContentOperation::new("l", vec![Operand::Real(100.0), Operand::Real(0.0)]),
            ContentOperation::new("S", vec![]),
        ]
    }

    fn resources_with_image(name: &str) -> PageResources {
        let mut r = PageResources::default();
        r.xobject_subtypes
            .insert(name.to_string(), "Image".to_string());
        r.xobjects.insert(name.to_string(), (1, 0));
        r.xobject_stream_dicts
            .insert(name.to_string(), basic_image_xobject_dict(false));
        r
    }

    fn basic_image_xobject_dict(is_mask: bool) -> PdfDictionary {
        let mut dict = PdfDictionary::empty();
        dict.insert("Subtype", PdfObject::Name("Image".to_string()));
        dict.insert("Width", PdfObject::Integer(1));
        dict.insert("Height", PdfObject::Integer(1));
        if is_mask {
            dict.insert("ImageMask", PdfObject::Boolean(true));
            dict.insert("BitsPerComponent", PdfObject::Integer(1));
        } else {
            dict.insert("ColorSpace", PdfObject::Name("DeviceRGB".to_string()));
            dict.insert("BitsPerComponent", PdfObject::Integer(8));
        }
        dict
    }

    fn set_image_filter_and_smask_in_data(
        resources: &mut PageResources,
        name: &str,
        filter: &str,
        smask_in_data: i64,
    ) {
        let dict = resources
            .xobject_stream_dicts
            .get_mut(name)
            .expect("test image metadata exists");
        dict.insert("Filter", PdfObject::Name(filter.to_string()));
        dict.insert("SMaskInData", PdfObject::Integer(smask_in_data));
    }

    fn inline_jpx_params_with_filter_and_smask_in_data(
        filter: &str,
        smask_in_data: i64,
    ) -> Vec<Operand> {
        vec![
            Operand::Name("Width".to_string()),
            Operand::Integer(1),
            Operand::Name("Height".to_string()),
            Operand::Integer(1),
            Operand::Name("ColorSpace".to_string()),
            Operand::Name("DeviceRGB".to_string()),
            Operand::Name("Filter".to_string()),
            Operand::Name(filter.to_string()),
            Operand::Name("SMaskInData".to_string()),
            Operand::Integer(smask_in_data),
        ]
    }

    fn inline_jpx_params_with_smask_in_data(smask_in_data: i64) -> Vec<Operand> {
        inline_jpx_params_with_filter_and_smask_in_data("JPXDecode", smask_in_data)
    }

    fn inline_terminal_image_params(filter: &str, color_space: &str) -> Vec<Operand> {
        vec![
            Operand::Name("Width".to_string()),
            Operand::Integer(1),
            Operand::Name("Height".to_string()),
            Operand::Integer(1),
            Operand::Name("BitsPerComponent".to_string()),
            Operand::Integer(1),
            Operand::Name("ColorSpace".to_string()),
            Operand::Name(color_space.to_string()),
            Operand::Name("Filter".to_string()),
            Operand::Name(filter.to_string()),
        ]
    }

    fn complete_inline_image_ops(params: Vec<Operand>) -> Vec<ContentOperation> {
        vec![
            ContentOperation::new("BI", vec![]),
            ContentOperation::new("ID", params),
            ContentOperation::new("inline_image_data", vec![Operand::String(vec![0x00])]),
            ContentOperation::new("EI", vec![]),
        ]
    }

    fn resources_with_form(name: &str) -> PageResources {
        let mut r = PageResources::default();
        r.xobject_subtypes
            .insert(name.to_string(), "Form".to_string());
        r.xobjects.insert(name.to_string(), (2, 0));
        r
    }

    fn resources_with_axial_shading(name: &str, n: f64) -> PageResources {
        resources_with_axial_shading_color_space(
            name,
            n,
            PdfObject::Name("DeviceRGB".to_string()),
            vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ],
            vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
            ],
        )
    }

    fn resources_with_axial_shading_color_space(
        name: &str,
        n: f64,
        color_space: PdfObject,
        c0: Vec<PdfObject>,
        c1: Vec<PdfObject>,
    ) -> PageResources {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert("C0", PdfObject::Array(c0));
        function.insert("C1", PdfObject::Array(c1));
        function.insert("N", PdfObject::Real(n));

        let mut shading = PdfDictionary::empty();
        shading.insert("ShadingType", PdfObject::Integer(2));
        shading.insert("ColorSpace", color_space);
        shading.insert(
            "Coords",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(100.0),
                PdfObject::Real(0.0),
            ]),
        );
        shading.insert(
            "Extend",
            PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
        );
        shading.insert("Function", PdfObject::Dictionary(function));

        let mut r = PageResources::default();
        r.shadings
            .insert(name.to_string(), PdfObject::Dictionary(shading));
        r
    }

    fn indexed_rgb_space(lookup: Vec<u8>) -> PdfObject {
        PdfObject::Array(vec![
            PdfObject::Name("Indexed".to_string()),
            PdfObject::Name("DeviceRGB".to_string()),
            PdfObject::Integer(1),
            PdfObject::String(lookup),
        ])
    }

    fn test_reader() -> PdfReader {
        PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap()
    }

    #[test]
    fn vector_group_color_space_rejects_malformed_separation_colorant() {
        let reader = test_reader();
        let mut tint = PdfDictionary::empty();
        tint.insert("FunctionType", PdfObject::Integer(2));
        tint.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        tint.insert(
            "C0",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ]),
        );
        tint.insert(
            "C1",
            PdfObject::Array(vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ]),
        );
        tint.insert("N", PdfObject::Real(1.0));
        let color_space = PdfObject::Array(vec![
            PdfObject::Name("Separation".to_string()),
            PdfObject::Integer(1),
            PdfObject::Name("DeviceRGB".to_string()),
            PdfObject::Dictionary(tint),
        ]);

        assert!(
            !transparency_group_color_space_is_vector_inert(
                &color_space,
                &PageResources::default(),
                &reader,
                0,
            ),
            "malformed Separation colorant must not classify as vector-inert group /CS",
        );
    }

    fn resources_with_radial_shading(name: &str) -> PageResources {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert("C0", PdfObject::Array(vec![PdfObject::Real(0.0)]));
        function.insert("C1", PdfObject::Array(vec![PdfObject::Real(1.0)]));
        function.insert("N", PdfObject::Real(1.0));

        let mut shading = PdfDictionary::empty();
        shading.insert("ShadingType", PdfObject::Integer(3));
        shading.insert("ColorSpace", PdfObject::Name("DeviceGray".to_string()));
        shading.insert(
            "Coords",
            PdfObject::Array(vec![
                PdfObject::Real(50.0),
                PdfObject::Real(50.0),
                PdfObject::Real(0.0),
                PdfObject::Real(50.0),
                PdfObject::Real(50.0),
                PdfObject::Real(40.0),
            ]),
        );
        shading.insert(
            "Extend",
            PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
        );
        shading.insert("Function", PdfObject::Dictionary(function));

        let mut r = PageResources::default();
        r.shadings
            .insert(name.to_string(), PdfObject::Dictionary(shading));
        r
    }

    fn shading_dict_mut<'a>(resources: &'a mut PageResources, name: &str) -> &'a mut PdfDictionary {
        match resources.shadings.get_mut(name) {
            Some(PdfObject::Dictionary(dict)) => dict,
            _ => panic!("test shading {name} is missing"),
        }
    }

    fn type2_rgb_function(c0: [f64; 3], c1: [f64; 3]) -> PdfObject {
        type2_rgb_function_with_exponent(c0, c1, 1.0)
    }

    fn type2_rgb_function_with_exponent(c0: [f64; 3], c1: [f64; 3], n: f64) -> PdfObject {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert(
            "C0",
            PdfObject::Array(c0.into_iter().map(PdfObject::Real).collect()),
        );
        function.insert(
            "C1",
            PdfObject::Array(c1.into_iter().map(PdfObject::Real).collect()),
        );
        function.insert("N", PdfObject::Real(n));
        PdfObject::Dictionary(function)
    }

    fn type2_cmyk_function(c0: [f64; 4], c1: [f64; 4]) -> PdfObject {
        type2_cmyk_function_with_exponent(c0, c1, 1.0)
    }

    fn type2_cmyk_function_with_exponent(c0: [f64; 4], c1: [f64; 4], n: f64) -> PdfObject {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert(
            "C0",
            PdfObject::Array(c0.into_iter().map(PdfObject::Real).collect()),
        );
        function.insert(
            "C1",
            PdfObject::Array(c1.into_iter().map(PdfObject::Real).collect()),
        );
        function.insert("N", PdfObject::Real(n));
        PdfObject::Dictionary(function)
    }

    fn type2_component_function(c0: f64, c1: f64, n: f64) -> PdfObject {
        type2_component_function_with_range(c0, c1, n, None)
    }

    fn type2_component_function_with_range(
        c0: f64,
        c1: f64,
        n: f64,
        range: Option<[f64; 2]>,
    ) -> PdfObject {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert("C0", PdfObject::Array(vec![PdfObject::Real(c0)]));
        function.insert("C1", PdfObject::Array(vec![PdfObject::Real(c1)]));
        function.insert("N", PdfObject::Real(n));
        if let Some([min, max]) = range {
            function.insert(
                "Range",
                PdfObject::Array(vec![PdfObject::Real(min), PdfObject::Real(max)]),
            );
        }
        PdfObject::Dictionary(function)
    }

    fn type2_rgb_component_function_array(exponents: [f64; 3]) -> PdfObject {
        PdfObject::Array(vec![
            type2_component_function(1.0, 0.0, exponents[0]),
            type2_component_function(0.5, 0.25, exponents[1]),
            type2_component_function(0.0, 1.0, exponents[2]),
        ])
    }

    fn type2_rgb_component_function_array_with_ranges(
        exponents: [f64; 3],
        ranges: [Option<[f64; 2]>; 3],
    ) -> PdfObject {
        PdfObject::Array(vec![
            type2_component_function_with_range(1.0, 0.0, exponents[0], ranges[0]),
            type2_component_function_with_range(0.5, 0.25, exponents[1], ranges[1]),
            type2_component_function_with_range(0.0, 1.0, exponents[2], ranges[2]),
        ])
    }

    fn type2_cmyk_component_function_array(exponents: [f64; 4]) -> PdfObject {
        PdfObject::Array(vec![
            type2_component_function(0.0, 1.0, exponents[0]),
            type2_component_function(1.0, 0.0, exponents[1]),
            type2_component_function(0.5, 0.25, exponents[2]),
            type2_component_function(0.0, 0.5, exponents[3]),
        ])
    }

    fn type2_cmyk_component_function_array_with_ranges(
        exponents: [f64; 4],
        ranges: [Option<[f64; 2]>; 4],
    ) -> PdfObject {
        PdfObject::Array(vec![
            type2_component_function_with_range(0.0, 1.0, exponents[0], ranges[0]),
            type2_component_function_with_range(1.0, 0.0, exponents[1], ranges[1]),
            type2_component_function_with_range(0.5, 0.25, exponents[2], ranges[2]),
            type2_component_function_with_range(0.0, 0.5, exponents[3], ranges[3]),
        ])
    }

    fn type2_gray_function(c0: f64, c1: f64) -> PdfObject {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert("C0", PdfObject::Array(vec![PdfObject::Real(c0)]));
        function.insert("C1", PdfObject::Array(vec![PdfObject::Real(c1)]));
        function.insert("N", PdfObject::Real(1.0));
        PdfObject::Dictionary(function)
    }

    fn stitching_function(second_start: [f64; 3]) -> PdfObject {
        stitching_function_with_second_exponent(second_start, 1.0)
    }

    fn stitching_function_with_second_exponent(
        second_start: [f64; 3],
        second_exponent: f64,
    ) -> PdfObject {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(3));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert(
            "Functions",
            PdfObject::Array(vec![
                type2_rgb_function([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                type2_rgb_function_with_exponent(second_start, [0.0, 0.0, 1.0], second_exponent),
            ]),
        );
        function.insert("Bounds", PdfObject::Array(vec![PdfObject::Real(0.5)]));
        function.insert(
            "Encode",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
            ]),
        );
        PdfObject::Dictionary(function)
    }

    fn gray_stitching_function(second_start: f64) -> PdfObject {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(3));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert(
            "Functions",
            PdfObject::Array(vec![
                type2_gray_function(1.0, 0.0),
                type2_gray_function(second_start, 1.0),
            ]),
        );
        function.insert("Bounds", PdfObject::Array(vec![PdfObject::Real(0.5)]));
        function.insert(
            "Encode",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
            ]),
        );
        PdfObject::Dictionary(function)
    }

    fn cmyk_stitching_function_with_second_exponent(
        second_start: [f64; 4],
        second_exponent: f64,
    ) -> PdfObject {
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(3));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert(
            "Functions",
            PdfObject::Array(vec![
                type2_cmyk_function([0.0, 1.0, 1.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
                type2_cmyk_function_with_exponent(
                    second_start,
                    [0.0, 0.0, 0.0, 1.0],
                    second_exponent,
                ),
            ]),
        );
        function.insert("Bounds", PdfObject::Array(vec![PdfObject::Real(0.5)]));
        function.insert(
            "Encode",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
            ]),
        );
        PdfObject::Dictionary(function)
    }

    fn continuous_stitching_function() -> PdfObject {
        stitching_function([0.0, 1.0, 0.0])
    }

    fn discontinuous_stitching_function() -> PdfObject {
        stitching_function([0.0, 0.0, 0.0])
    }

    fn nonlinear_stitching_function() -> PdfObject {
        stitching_function_with_second_exponent([0.0, 1.0, 0.0], 2.0)
    }

    fn discontinuous_gray_stitching_function() -> PdfObject {
        gray_stitching_function(0.5)
    }

    fn discontinuous_cmyk_stitching_function() -> PdfObject {
        cmyk_stitching_function_with_second_exponent([0.25, 0.25, 0.25, 0.25], 2.0)
    }

    #[test]
    fn pure_vector_ops_classified_as_pure_vector() {
        let ops = make_path_ops();
        let r = PageResources::default();
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert_eq!(decision, VectorFallbackDecision::PureVector);
    }

    #[test]
    fn resource_named_device_rgb_fill_paint_remains_vector_safe() {
        let mut ops = vec![
            ContentOperation::new("cs", vec![Operand::Name("CS0".to_string())]),
            ContentOperation::new(
                "scn",
                vec![Operand::Real(0.2), Operand::Real(0.4), Operand::Real(0.6)],
            ),
        ];
        ops.extend(make_path_ops());

        let mut resources = PageResources::default();
        resources
            .color_spaces
            .insert("CS0".to_string(), PdfObject::Name("DeviceRGB".to_string()));

        assert_eq!(
            classify_page_for_vector_output(&ops, &resources, 1.0),
            VectorFallbackDecision::PureVector
        );
    }

    #[test]
    fn unresolved_named_fill_paint_stays_whole_page() {
        let mut ops = vec![
            ContentOperation::new("cs", vec![Operand::Name("CS0".to_string())]),
            ContentOperation::new(
                "scn",
                vec![Operand::Real(0.2), Operand::Real(0.4), Operand::Real(0.6)],
            ),
        ];
        ops.extend(make_path_ops());

        assert_eq!(
            classify_page_for_vector_output(&ops, &PageResources::default(), 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "unsupported named fill color"
            }
        );
    }

    #[test]
    fn unsupported_named_stroke_paint_stays_whole_page() {
        let ops = vec![
            ContentOperation::new("CS", vec![Operand::Name("CS0".to_string())]),
            ContentOperation::new("SCN", vec![Operand::Real(0.5)]),
            ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]),
            ContentOperation::new("l", vec![Operand::Real(100.0), Operand::Real(0.0)]),
            ContentOperation::new("S", vec![]),
        ];
        let mut resources = PageResources::default();
        resources.color_spaces.insert(
            "CS0".to_string(),
            PdfObject::Name("UnsupportedSpace".to_string()),
        );

        assert_eq!(
            classify_page_for_vector_output(&ops, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "unsupported named stroke color"
            }
        );
    }

    #[test]
    fn named_device_rgb_paint_without_components_stays_whole_page() {
        let mut ops = vec![ContentOperation::new(
            "cs",
            vec![Operand::Name("CS0".to_string())],
        )];
        ops.extend(make_path_ops());

        let mut resources = PageResources::default();
        resources
            .color_spaces
            .insert("CS0".to_string(), PdfObject::Name("DeviceRGB".to_string()));

        assert_eq!(
            classify_page_for_vector_output(&ops, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "unsupported named fill color"
            }
        );
    }

    #[test]
    fn image_xobject_with_axis_aligned_ctm_is_regional() {
        // Set up a CTM that places a 200x100 image at (50, 300) — axis-aligned.
        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(200.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(-100.0), // negative = normal image flip
                    Operand::Real(50.0),
                    Operand::Real(400.0),
                ],
            ),
            make_do_op("Im0"),
        ];
        // Add some vector ops before.
        let mut full_ops = make_path_ops();
        full_ops.extend(ops);
        let r = resources_with_image("Im0");
        let decision = classify_page_for_vector_output(&full_ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
                assert_eq!(image_names, vec!["Im0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn malformed_image_xobject_metadata_triggers_whole_page() {
        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(200.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(-100.0),
                    Operand::Real(50.0),
                    Operand::Real(400.0),
                ],
            ),
            make_do_op("Im0"),
        ];
        let cases = [
            ("missing stream dictionary", None),
            (
                "missing non-mask BitsPerComponent",
                Some({
                    let mut dict = basic_image_xobject_dict(false);
                    dict.remove("BitsPerComponent");
                    dict
                }),
            ),
            (
                "missing non-mask ColorSpace",
                Some({
                    let mut dict = basic_image_xobject_dict(false);
                    dict.remove("ColorSpace");
                    dict
                }),
            ),
            (
                "mask BitsPerComponent not 1",
                Some({
                    let mut dict = basic_image_xobject_dict(true);
                    dict.insert("BitsPerComponent", PdfObject::Integer(8));
                    dict
                }),
            ),
            (
                "mask Decode has non-numeric entry",
                Some({
                    let mut dict = basic_image_xobject_dict(true);
                    dict.insert(
                        "Decode",
                        PdfObject::Array(vec![
                            PdfObject::Name("Bad".to_string()),
                            PdfObject::Integer(1),
                        ]),
                    );
                    dict
                }),
            ),
        ];

        for (label, dict) in cases {
            let mut resources = resources_with_image("Im0");
            if let Some(dict) = dict {
                resources
                    .xobject_stream_dicts
                    .insert("Im0".to_string(), dict);
            } else {
                resources.xobject_stream_dicts.remove("Im0");
            }
            match classify_page_for_vector_output(&ops, &resources, 1.0) {
                VectorFallbackDecision::WholePageRaster { reason } => {
                    assert_eq!(
                        reason, "degenerate or unresolvable image XObject",
                        "{label}"
                    );
                }
                other => panic!("{label}: expected WholePageRaster, got {:?}", other),
            }
        }
    }

    #[test]
    fn form_xobject_triggers_whole_page() {
        let ops = vec![make_do_op("Fm0")];
        let r = resources_with_form("Fm0");
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn vector_form_matrix_extractor_rejects_malformed_present_matrix() {
        let mut dict = PdfDictionary::empty();
        assert_eq!(
            extract_form_matrix(&dict),
            Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
        );

        dict.insert(
            "Matrix",
            PdfObject::Array(vec![PdfObject::Real(1.0), PdfObject::Real(0.0)]),
        );
        assert!(
            extract_form_matrix(&dict).is_none(),
            "present malformed Form Matrix must not default to identity for vector output"
        );
    }

    #[test]
    fn vector_form_bbox_extractor_requires_exact_numeric_array() {
        let mut dict = PdfDictionary::empty();
        assert!(
            extract_bbox(&dict).is_none(),
            "missing Form BBox cannot be vector-regionally replayed"
        );

        dict.insert(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(50),
                PdfObject::Integer(50),
            ]),
        );
        assert_eq!(extract_bbox(&dict), Some([0.0, 0.0, 50.0, 50.0]));

        for bad_bbox in [
            vec![PdfObject::Integer(0), PdfObject::Integer(0)],
            vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(50),
                PdfObject::Integer(50),
                PdfObject::Integer(60),
            ],
            vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Name("Bad".to_string()),
                PdfObject::Integer(50),
            ],
        ] {
            dict.insert("BBox", PdfObject::Array(bad_bbox));
            assert!(
                extract_bbox(&dict).is_none(),
                "malformed Form BBox must not be truncated or filtered for vector output"
            );
        }
    }

    #[test]
    fn vector_pattern_matrix_extractor_rejects_malformed_present_matrix() {
        let mut dict = PdfDictionary::empty();
        assert_eq!(pattern_matrix(&dict), Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));

        dict.insert(
            "Matrix",
            PdfObject::Array(vec![PdfObject::Real(1.0), PdfObject::Real(0.0)]),
        );
        assert!(
            pattern_matrix(&dict).is_none(),
            "present malformed pattern Matrix must not default to identity for vector output"
        );
    }

    #[test]
    fn vector_iccbased_shading_options_keep_qcms_for_gray_rgb_and_gate_cmyk() {
        assert_eq!(
            vector_iccbased_shading_options(1).map(|options| options.backend),
            Some(cmm::ColorTransformBackend::PortableQcms)
        );
        assert_eq!(
            vector_iccbased_shading_options(3).map(|options| options.backend),
            Some(cmm::ColorTransformBackend::PortableQcms)
        );

        let cmyk = vector_iccbased_shading_options(4).map(|options| options.backend);
        if cmm::native_cmm_status().available {
            assert_eq!(cmyk, Some(cmm::ColorTransformBackend::NativeLittleCms));
        } else {
            assert_eq!(cmyk, None);
        }
        assert_eq!(vector_iccbased_shading_options(2), None);
    }

    #[test]
    fn vector_indexed_iccbased_shading_options_reuse_icc_backend_gate() {
        let reader = PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap();
        let mut profile_dict = PdfDictionary::empty();
        profile_dict.insert("N", PdfObject::Integer(4));
        let profile = PdfObject::Stream {
            dict: profile_dict,
            raw: Vec::new(),
        };
        let color_space = PdfObject::Array(vec![
            PdfObject::Name("Indexed".to_string()),
            PdfObject::Array(vec![PdfObject::Name("ICCBased".to_string()), profile]),
            PdfObject::Integer(0),
            PdfObject::String(vec![0, 0, 0, 0]),
        ]);
        let backend =
            vector_indexed_shading_options(&color_space, &reader).map(|options| options.backend);
        if cmm::native_cmm_status().available {
            assert_eq!(backend, Some(cmm::ColorTransformBackend::NativeLittleCms));
        } else {
            assert_eq!(backend, None);
        }

        let non_icc = PdfObject::Array(vec![
            PdfObject::Name("Indexed".to_string()),
            PdfObject::Name("DeviceRGB".to_string()),
            PdfObject::Integer(0),
            PdfObject::String(vec![0, 0, 0]),
        ]);
        assert_eq!(
            vector_indexed_shading_options(&non_icc, &reader).map(|options| options.backend),
            Some(cmm::ColorTransformBackend::PortableQcms)
        );
    }

    #[test]
    fn simple_axial_shading_is_regional_vector_output() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let r = resources_with_axial_shading("Sh0", 1.0);
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn non_unit_linear_axial_shading_domain_is_regional_vector_output() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut r, "Sh0").insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.25), PdfObject::Real(0.75)]),
        );
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn nonextended_axial_shading_is_svg_and_postscript_native() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut resources = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut resources, "Sh0").insert(
            "Extend",
            PdfObject::Array(vec![PdfObject::Boolean(false), PdfObject::Boolean(false)]),
        );
        let reader = test_reader();

        match classify_page_for_vector_output(&ops, &resources, 1.0) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "conservative vector output should keep non-extended axial shading regional: {other:?}"
            ),
        }
        match classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => {
                panic!("SVG clipPath should keep non-extended axial shading regional: {other:?}")
            }
        }
        match classify_page_for_postscript_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => {
                panic!("PostScript shfill should keep explicit /Extend false false: {other:?}")
            }
        }
        match load_vector_shading_for_postscript_output(&resources, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => {
                assert_eq!(shading.extend, [false, false]);
            }
            other => panic!(
                "expected PostScript axial shading plan with explicit extend flags: {other:?}"
            ),
        }
        match load_vector_shading(&resources, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => {
                assert_eq!(shading.extend, [false, false]);
            }
            other => {
                panic!("expected SVG axial shading plan with explicit extend flags: {other:?}")
            }
        }
    }

    #[test]
    fn absent_extend_axial_shading_defaults_to_nonextended_vector_flags() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut resources = resources_with_axial_shading("Sh0", 1.0);
        assert!(
            shading_dict_mut(&mut resources, "Sh0")
                .remove("Extend")
                .is_some(),
            "fixture should start with an explicit /Extend entry"
        );
        let reader = test_reader();

        match classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("SVG should keep absent /Extend axial shading regional: {other:?}"),
        }
        match classify_page_for_postscript_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => {
                panic!("PostScript should keep absent /Extend axial shading regional: {other:?}")
            }
        }
        match load_vector_shading(&resources, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => {
                assert_eq!(shading.extend, [false, false]);
            }
            other => panic!("expected SVG axial shading plan with default extend flags: {other:?}"),
        }
        match load_vector_shading_for_postscript_output(&resources, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => {
                assert_eq!(shading.extend, [false, false]);
            }
            other => panic!(
                "expected PostScript axial shading plan with default extend flags: {other:?}"
            ),
        }
    }

    #[test]
    fn asymmetric_extended_axial_shading_keeps_vector_extend_flags() {
        let mut resources = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut resources, "Sh0").insert(
            "Extend",
            PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(false)]),
        );
        let reader = test_reader();

        match load_vector_shading_for_postscript_output(&resources, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => {
                assert_eq!(shading.extend, [true, false]);
            }
            other => {
                panic!("expected PostScript axial shading plan with asymmetric extend flags: {other:?}")
            }
        }
        assert!(
            matches!(
                load_vector_shading(&resources, Some(&reader), "Sh0"),
                Some(VectorShading::Axial(ref shading)) if shading.extend == [true, false]
            ),
            "expected SVG axial shading plan with asymmetric extend flags"
        );
    }

    #[test]
    fn nonextended_concentric_radial_shading_is_svg_and_postscript_native() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut resources = resources_with_radial_shading("Sh0");
        shading_dict_mut(&mut resources, "Sh0").insert(
            "Extend",
            PdfObject::Array(vec![PdfObject::Boolean(false), PdfObject::Boolean(false)]),
        );
        let reader = test_reader();

        match classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => {
                panic!("SVG should keep concentric non-extended radial shading regional: {other:?}")
            }
        }
        match classify_page_for_postscript_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => {
                panic!("PostScript should keep radial /Extend flags native: {other:?}")
            }
        }
        match load_vector_shading(&resources, Some(&reader), "Sh0") {
            Some(VectorShading::Radial(shading)) => {
                assert_eq!(shading.extend, [false, false]);
            }
            other => {
                panic!("expected SVG radial shading plan with explicit extend flags: {other:?}")
            }
        }
    }

    #[test]
    fn nonextended_nonconcentric_radial_shading_stays_conservative_for_svg() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut resources = resources_with_radial_shading("Sh0");
        shading_dict_mut(&mut resources, "Sh0").insert(
            "Coords",
            PdfObject::Array(vec![
                PdfObject::Real(40.0),
                PdfObject::Real(50.0),
                PdfObject::Real(10.0),
                PdfObject::Real(55.0),
                PdfObject::Real(50.0),
                PdfObject::Real(40.0),
            ]),
        );
        shading_dict_mut(&mut resources, "Sh0").insert(
            "Extend",
            PdfObject::Array(vec![PdfObject::Boolean(false), PdfObject::Boolean(false)]),
        );
        let reader = test_reader();

        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("PostScript should preserve nonconcentric radial /Extend: {other:?}"),
        }
    }

    #[test]
    fn function_array_axial_shading_is_regional_vector_output() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 1.0);
        let component_function = |c0: f64, c1: f64| {
            let mut function = PdfDictionary::empty();
            function.insert("FunctionType", PdfObject::Integer(2));
            function.insert(
                "Domain",
                PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
            );
            function.insert("C0", PdfObject::Array(vec![PdfObject::Real(c0)]));
            function.insert("C1", PdfObject::Array(vec![PdfObject::Real(c1)]));
            function.insert("N", PdfObject::Real(1.0));
            PdfObject::Dictionary(function)
        };
        shading_dict_mut(&mut r, "Sh0").insert(
            "Function",
            PdfObject::Array(vec![
                component_function(1.0, 0.0),
                component_function(0.0, 0.0),
                component_function(0.0, 1.0),
            ]),
        );
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn stitching_function_axial_shading_is_regional_vector_output() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut r, "Sh0").insert("Function", continuous_stitching_function());
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("continuous stitching-function axial shading should stay regional vector: {other:?}"),
        }
    }

    #[test]
    fn discontinuous_stitching_function_axial_shading_stays_whole_page_fallback() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut r, "Sh0").insert("Function", discontinuous_stitching_function());
        let reader = test_reader();
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        let svg_decision = classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader);
        assert!(matches!(
            svg_decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => {
                panic!("PostScript should keep exact Type 3 stitching shadings regional: {other:?}")
            }
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Stitching(function)) => {
                    assert_eq!(function.segments.len(), 2);
                    assert!((function.segments[0].bound_end - 0.5).abs() <= 1e-9);
                }
                other => panic!("expected exact PostScript Type 3 sidecar: {other:?}"),
            },
            other => {
                panic!("expected PostScript axial shading with exact Type 3 sidecar: {other:?}")
            }
        }
    }

    #[test]
    fn nonlinear_stitching_function_axial_shading_is_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut r, "Sh0").insert("Function", nonlinear_stitching_function());
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should keep exact nonlinear Type 3 stitching shadings regional: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Stitching(function)) => {
                    assert_eq!(function.segments.len(), 2);
                    assert_eq!(function.segments[1].function.n, 2.0);
                }
                other => panic!("expected exact PostScript nonlinear Type 3 sidecar: {other:?}"),
            },
            other => panic!(
                "expected PostScript axial shading with exact nonlinear Type 3 sidecar: {other:?}"
            ),
        }
    }

    #[test]
    fn malformed_resource_invocation_stays_whole_page_vector_fallback() {
        let resources = {
            let mut resources = resources_with_image("Im0");
            let mut shading_resources = resources_with_axial_shading("Sh0", 1.0);
            resources.shadings = std::mem::take(&mut shading_resources.shadings);
            resources
        };

        for op in [
            ContentOperation::new(
                "Do",
                vec![Operand::Name("Im0".to_string()), Operand::Real(1.0)],
            ),
            ContentOperation::new(
                "sh",
                vec![Operand::Name("Sh0".to_string()), Operand::Real(1.0)],
            ),
        ] {
            assert_eq!(
                classify_page_for_vector_output(&[op], &resources, 1.0),
                VectorFallbackDecision::WholePageRaster {
                    reason: "malformed resource invocation"
                }
            );
        }
    }

    #[test]
    fn malformed_operands_stay_whole_page_vector_fallback() {
        let mut resources = PageResources::default();
        resources
            .ext_g_states
            .insert("GS0".to_string(), PdfDictionary::empty());

        for (op, reason) in [
            (
                ContentOperation::new(
                    "cm",
                    vec![
                        Operand::Real(1.0),
                        Operand::Real(0.0),
                        Operand::Real(0.0),
                        Operand::Real(1.0),
                        Operand::Real(0.0),
                        Operand::Real(0.0),
                        Operand::Real(1.0),
                    ],
                ),
                "malformed graphics-state operator",
            ),
            (
                ContentOperation::new(
                    "gs",
                    vec![Operand::Name("GS0".to_string()), Operand::Real(1.0)],
                ),
                "malformed graphics-state operator",
            ),
            (
                ContentOperation::new("Tj", vec![]),
                "malformed text operator",
            ),
            (
                ContentOperation::new("Tj", vec![Operand::String(b"H".to_vec())]),
                "malformed text-object sequence",
            ),
            (
                ContentOperation::new("ET", vec![]),
                "malformed text-object sequence",
            ),
            (
                ContentOperation::new("BMC", vec![]),
                "malformed marked-content operator",
            ),
            (
                ContentOperation::new("EMC", vec![]),
                "malformed marked-content operator",
            ),
            (
                ContentOperation::new("EX", vec![]),
                "malformed compatibility-section operator",
            ),
            (
                ContentOperation::new("Q", vec![]),
                "malformed graphics-state operator",
            ),
            (
                ContentOperation::new("l", vec![Operand::Real(1.0), Operand::Real(2.0)]),
                "malformed path operator",
            ),
            (
                ContentOperation::new(
                    "m",
                    vec![Operand::Real(1.0), Operand::Real(2.0), Operand::Real(3.0)],
                ),
                "malformed path operator",
            ),
            (
                ContentOperation::new("d1", vec![Operand::Real(600.0), Operand::Real(0.0)]),
                "malformed Type 3 glyph metric operator",
            ),
        ] {
            assert_eq!(
                classify_page_for_vector_output(&[op], &resources, 1.0),
                VectorFallbackDecision::WholePageRaster { reason }
            );
        }

        let unterminated_marked = [ContentOperation::new(
            "BMC",
            vec![Operand::Name("Span".to_string())],
        )];
        assert_eq!(
            classify_page_for_vector_output(&unterminated_marked, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "unterminated marked-content sequence"
            }
        );

        let unterminated_compatibility = [ContentOperation::new("BX", vec![])];
        assert_eq!(
            classify_page_for_vector_output(&unterminated_compatibility, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "unterminated compatibility-section sequence"
            }
        );

        let empty_clip = [
            ContentOperation::new("W", vec![]),
            ContentOperation::new("n", vec![]),
        ];
        assert_eq!(
            classify_page_for_vector_output(&empty_clip, &resources, 1.0),
            VectorFallbackDecision::PureVector
        );

        let unterminated_clip = [
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(8.0),
                    Operand::Real(8.0),
                ],
            ),
            ContentOperation::new("W", vec![]),
        ];
        assert_eq!(
            classify_page_for_vector_output(&unterminated_clip, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "unterminated clipping path sequence"
            }
        );

        let repeated_clip = [
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(1.0),
                    Operand::Real(1.0),
                    Operand::Real(8.0),
                    Operand::Real(8.0),
                ],
            ),
            ContentOperation::new("W", vec![]),
            ContentOperation::new("W*", vec![]),
            ContentOperation::new("n", vec![]),
        ];
        assert_eq!(
            classify_page_for_vector_output(&repeated_clip, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "malformed clipping path sequence"
            }
        );

        let nested_text_object = [
            ContentOperation::new("BT", vec![]),
            ContentOperation::new("BT", vec![]),
            ContentOperation::new("ET", vec![]),
        ];
        assert_eq!(
            classify_page_for_vector_output(&nested_text_object, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "malformed text-object sequence"
            }
        );

        let unterminated_text_object = [ContentOperation::new("BT", vec![])];
        assert_eq!(
            classify_page_for_vector_output(&unterminated_text_object, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster {
                reason: "unterminated text-object sequence"
            }
        );
    }

    #[test]
    fn nonlinear_axial_shading_is_postscript_regional_but_svg_whole_page() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let r = resources_with_axial_shading("Sh0", 2.0);
        let reader = test_reader();
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        let svg_decision = classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader);
        assert!(matches!(
            svg_decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => {
                panic!("PostScript should keep exact Type 2 exponent shadings regional: {other:?}")
            }
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2(function)) => {
                    assert_eq!(function.n, 2.0);
                }
                other => panic!("expected exact PostScript Type 2 sidecar: {other:?}"),
            },
            other => panic!("expected PostScript axial shading with exact /N metadata: {other:?}"),
        }
    }

    #[test]
    fn nonlinear_device_gray_axial_shading_is_postscript_regional_but_svg_whole_page() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let r = resources_with_axial_shading_color_space(
            "Sh0",
            2.0,
            PdfObject::Name("DeviceGray".to_string()),
            vec![PdfObject::Real(0.0)],
            vec![PdfObject::Real(1.0)],
        );
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should keep exact DeviceGray Type 2 exponent shadings regional: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2(function)) => {
                    assert_eq!(function.c0, [0.0, 0.0, 0.0]);
                    assert_eq!(function.c1, [1.0, 1.0, 1.0]);
                    assert_eq!(function.n, 2.0);
                }
                other => panic!("expected exact PostScript DeviceGray Type 2 sidecar: {other:?}"),
            },
            other => {
                panic!("expected PostScript axial shading with exact DeviceGray /N metadata: {other:?}")
            }
        }
    }

    #[test]
    fn nonlinear_type2_function_with_range_is_exact_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 2.5);
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        );
        function.insert(
            "C0",
            PdfObject::Array(vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.5),
                PdfObject::Real(0.0),
            ]),
        );
        function.insert(
            "C1",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.25),
                PdfObject::Real(1.0),
            ]),
        );
        function.insert("N", PdfObject::Real(2.5));
        function.insert(
            "Range",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.9),
                PdfObject::Real(0.0),
                PdfObject::Real(0.8),
                PdfObject::Real(0.0),
                PdfObject::Real(0.7),
            ]),
        );
        shading_dict_mut(&mut r, "Sh0").insert("Function", PdfObject::Dictionary(function));

        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should preserve finite Type 2 /Range shadings regionally: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2(function)) => {
                    assert_eq!(function.c0, [1.0, 0.5, 0.0]);
                    assert_eq!(function.c1, [0.0, 0.25, 1.0]);
                    assert_eq!(function.n, 2.5);
                    assert_eq!(function.range, Some([[0.0, 0.9], [0.0, 0.8], [0.0, 0.7]]));
                }
                other => panic!("expected exact PostScript Type 2 sidecar with /Range: {other:?}"),
            },
            other => {
                panic!("expected PostScript axial shading with exact Type 2 /Range metadata: {other:?}")
            }
        }
    }

    #[test]
    fn nonlinear_type2_function_with_non_unit_domain_is_exact_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 2.5);
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.25), PdfObject::Real(0.75)]),
        );
        function.insert(
            "C0",
            PdfObject::Array(vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.5),
                PdfObject::Real(0.0),
            ]),
        );
        function.insert(
            "C1",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.25),
                PdfObject::Real(1.0),
            ]),
        );
        function.insert("N", PdfObject::Real(2.5));
        shading_dict_mut(&mut r, "Sh0").insert("Function", PdfObject::Dictionary(function));

        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should preserve finite non-unit Type 2 function domains regionally: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2(function)) => {
                    assert_eq!(function.domain, [0.25, 0.75]);
                    assert_eq!(function.c0, [1.0, 0.5, 0.0]);
                    assert_eq!(function.c1, [0.0, 0.25, 1.0]);
                    assert_eq!(function.n, 2.5);
                }
                other => panic!(
                    "expected exact PostScript Type 2 sidecar with non-unit /Domain: {other:?}"
                ),
            },
            other => {
                panic!("expected PostScript axial shading with exact Type 2 /Domain metadata: {other:?}")
            }
        }
    }

    #[test]
    fn nonlinear_type2_function_with_non_unit_shading_domain_is_exact_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 2.5);
        shading_dict_mut(&mut r, "Sh0").insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(0.25), PdfObject::Real(0.75)]),
        );

        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should preserve finite non-unit shading domains regionally: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => {
                assert_eq!(shading.domain, [0.25, 0.75]);
                match shading.ps_function.as_ref() {
                    Some(VectorPostScriptShadingFunction::Type2(function)) => {
                        assert_eq!(function.domain, [0.0, 1.0]);
                        assert_eq!(function.n, 2.5);
                    }
                    other => panic!(
                        "expected exact PostScript Type 2 sidecar with shading /Domain metadata: {other:?}"
                    ),
                }
            }
            other => {
                panic!("expected PostScript axial shading with exact shading /Domain metadata: {other:?}")
            }
        }
    }

    #[test]
    fn nonlinear_device_rgb_type2_function_array_is_exact_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut r, "Sh0").insert(
            "Function",
            type2_rgb_component_function_array([2.5, 2.5, 2.5]),
        );
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should combine same-exponent RGB Type 2 arrays exactly: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2(function)) => {
                    assert_eq!(function.c0, [1.0, 0.5, 0.0]);
                    assert_eq!(function.c1, [0.0, 0.25, 1.0]);
                    assert_eq!(function.n, 2.5);
                }
                other => panic!("expected exact PostScript RGB Type 2 array sidecar: {other:?}"),
            },
            other => panic!(
                "expected PostScript axial shading with exact Type 2 array metadata: {other:?}"
            ),
        }
    }

    #[test]
    fn nonlinear_device_rgb_type2_function_array_with_mixed_exponents_is_exact_postscript_regional()
    {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut r, "Sh0").insert(
            "Function",
            type2_rgb_component_function_array([1.0, 2.0, 3.0]),
        );
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should preserve mixed-exponent RGB Type 2 arrays exactly: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2RgbArray(function)) => {
                    assert_eq!(function.channels[0].c0, 1.0);
                    assert_eq!(function.channels[0].c1, 0.0);
                    assert_eq!(function.channels[0].n, 1.0);
                    assert_eq!(function.channels[1].c0, 0.5);
                    assert_eq!(function.channels[1].c1, 0.25);
                    assert_eq!(function.channels[1].n, 2.0);
                    assert_eq!(function.channels[2].c0, 0.0);
                    assert_eq!(function.channels[2].c1, 1.0);
                    assert_eq!(function.channels[2].n, 3.0);
                }
                other => {
                    panic!("expected exact PostScript RGB Type 2 array sidecar: {other:?}")
                }
            },
            other => panic!(
                "expected PostScript axial shading with exact Type 2 array metadata: {other:?}"
            ),
        }
    }

    #[test]
    fn nonlinear_device_rgb_type2_function_array_with_ranges_is_exact_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut r, "Sh0").insert(
            "Function",
            type2_rgb_component_function_array_with_ranges(
                [1.0, 2.0, 3.0],
                [Some([0.0, 0.95]), None, Some([0.1, 1.0])],
            ),
        );
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should preserve ranged RGB Type 2 function arrays exactly: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2RgbArray(function)) => {
                    assert_eq!(function.channels[0].range, Some([0.0, 0.95]));
                    assert_eq!(function.channels[1].range, None);
                    assert_eq!(function.channels[2].range, Some([0.1, 1.0]));
                    assert_eq!(function.channels[0].n, 1.0);
                    assert_eq!(function.channels[1].n, 2.0);
                    assert_eq!(function.channels[2].n, 3.0);
                }
                other => {
                    panic!("expected exact ranged PostScript RGB Type 2 array sidecar: {other:?}")
                }
            },
            other => panic!(
                "expected PostScript axial shading with ranged Type 2 array metadata: {other:?}"
            ),
        }
    }

    #[test]
    fn discontinuous_device_gray_stitching_function_axial_shading_is_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            PdfObject::Name("DeviceGray".to_string()),
            vec![PdfObject::Real(0.0)],
            vec![PdfObject::Real(1.0)],
        );
        shading_dict_mut(&mut r, "Sh0").insert("Function", discontinuous_gray_stitching_function());
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should keep exact DeviceGray Type 3 stitching shadings regional: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Stitching(function)) => {
                    assert_eq!(function.segments.len(), 2);
                    assert_eq!(function.segments[0].function.c0, [1.0, 1.0, 1.0]);
                    assert_eq!(function.segments[1].function.c0, [0.5, 0.5, 0.5]);
                }
                other => panic!("expected exact PostScript DeviceGray Type 3 sidecar: {other:?}"),
            },
            other => panic!(
                "expected PostScript axial shading with exact DeviceGray Type 3 metadata: {other:?}"
            ),
        }
    }

    #[test]
    fn discontinuous_device_cmyk_stitching_function_axial_shading_is_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            PdfObject::Name("DeviceCMYK".to_string()),
            vec![
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
            ],
            vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
            ],
        );
        shading_dict_mut(&mut r, "Sh0").insert("Function", discontinuous_cmyk_stitching_function());
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should keep exact DeviceCMYK Type 3 stitching shadings regional: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::StitchingCmyk(function)) => {
                    assert_eq!(function.segments.len(), 2);
                    assert_eq!(function.segments[0].function.c0, [0.0, 1.0, 1.0, 0.0]);
                    assert_eq!(function.segments[0].function.c1, [1.0, 0.0, 0.0, 0.0]);
                    assert_eq!(function.segments[1].function.c0, [0.25; 4]);
                    assert_eq!(function.segments[1].function.c1, [0.0, 0.0, 0.0, 1.0]);
                    assert_eq!(function.segments[1].function.n, 2.0);
                }
                other => panic!("expected exact PostScript DeviceCMYK Type 3 sidecar: {other:?}"),
            },
            other => panic!(
                "expected PostScript axial shading with exact DeviceCMYK stitching metadata: {other:?}"
            ),
        }
    }

    #[test]
    fn nonlinear_device_cmyk_axial_shading_is_exact_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let r = resources_with_axial_shading_color_space(
            "Sh0",
            2.0,
            PdfObject::Name("DeviceCMYK".to_string()),
            vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ],
            vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ],
        );
        let reader = test_reader();
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should keep exact DeviceCMYK Type 2 exponent shadings regional: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2Cmyk(function)) => {
                    assert_eq!(function.c0, [0.0, 0.0, 0.0, 0.0]);
                    assert_eq!(function.c1, [1.0, 0.0, 0.0, 0.0]);
                    assert_eq!(function.n, 2.0);
                }
                other => panic!("expected exact PostScript DeviceCMYK Type 2 sidecar: {other:?}"),
            },
            other => {
                panic!("expected PostScript axial shading with exact DeviceCMYK /N metadata: {other:?}")
            }
        }
    }

    #[test]
    fn nonlinear_device_cmyk_type2_function_array_is_exact_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            PdfObject::Name("DeviceCMYK".to_string()),
            vec![
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(0.5),
                PdfObject::Real(0.0),
            ],
            vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.25),
                PdfObject::Real(0.5),
            ],
        );
        shading_dict_mut(&mut r, "Sh0").insert(
            "Function",
            type2_cmyk_component_function_array([2.5, 2.5, 2.5, 2.5]),
        );
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should combine same-exponent CMYK Type 2 arrays exactly: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2Cmyk(function)) => {
                    assert_eq!(function.c0, [0.0, 1.0, 0.5, 0.0]);
                    assert_eq!(function.c1, [1.0, 0.0, 0.25, 0.5]);
                    assert_eq!(function.n, 2.5);
                }
                other => panic!("expected exact PostScript CMYK Type 2 array sidecar: {other:?}"),
            },
            other => panic!(
                "expected PostScript axial shading with exact CMYK Type 2 array metadata: {other:?}"
            ),
        }
    }

    #[test]
    fn nonlinear_device_cmyk_type2_function_array_with_mixed_exponents_is_exact_postscript_regional(
    ) {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            PdfObject::Name("DeviceCMYK".to_string()),
            vec![
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(0.5),
                PdfObject::Real(0.0),
            ],
            vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.25),
                PdfObject::Real(0.5),
            ],
        );
        shading_dict_mut(&mut r, "Sh0").insert(
            "Function",
            type2_cmyk_component_function_array([1.0, 2.0, 3.0, 4.0]),
        );
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should preserve mixed-exponent CMYK Type 2 arrays exactly: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2CmykArray(function)) => {
                    assert_eq!(function.channels[0].c0, 0.0);
                    assert_eq!(function.channels[0].c1, 1.0);
                    assert_eq!(function.channels[0].n, 1.0);
                    assert_eq!(function.channels[1].c0, 1.0);
                    assert_eq!(function.channels[1].c1, 0.0);
                    assert_eq!(function.channels[1].n, 2.0);
                    assert_eq!(function.channels[2].c0, 0.5);
                    assert_eq!(function.channels[2].c1, 0.25);
                    assert_eq!(function.channels[2].n, 3.0);
                    assert_eq!(function.channels[3].c0, 0.0);
                    assert_eq!(function.channels[3].c1, 0.5);
                    assert_eq!(function.channels[3].n, 4.0);
                }
                other => panic!("expected exact PostScript CMYK Type 2 array sidecar: {other:?}"),
            },
            other => panic!(
                "expected PostScript axial shading with exact CMYK Type 2 array metadata: {other:?}"
            ),
        }
    }

    #[test]
    fn nonlinear_device_cmyk_type2_function_array_with_ranges_is_exact_postscript_regional() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            PdfObject::Name("DeviceCMYK".to_string()),
            vec![
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(0.5),
                PdfObject::Real(0.0),
            ],
            vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.25),
                PdfObject::Real(0.5),
            ],
        );
        shading_dict_mut(&mut r, "Sh0").insert(
            "Function",
            type2_cmyk_component_function_array_with_ranges(
                [1.0, 2.0, 3.0, 4.0],
                [Some([0.0, 1.0]), Some([0.0, 0.85]), None, Some([0.1, 0.75])],
            ),
        );
        let reader = test_reader();
        assert!(matches!(
            classify_page_for_vector_output(&ops, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        assert!(matches!(
            classify_page_for_svg_output_with_reader(&ops, &r, 1.0, &reader),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
        match classify_page_for_postscript_output_with_reader(&ops, &r, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!(
                "PostScript should preserve ranged CMYK Type 2 function arrays exactly: {other:?}"
            ),
        }
        match load_vector_shading_for_postscript_output(&r, Some(&reader), "Sh0") {
            Some(VectorShading::Axial(shading)) => match shading.ps_function.as_ref() {
                Some(VectorPostScriptShadingFunction::Type2CmykArray(function)) => {
                    assert_eq!(function.channels[0].range, Some([0.0, 1.0]));
                    assert_eq!(function.channels[1].range, Some([0.0, 0.85]));
                    assert_eq!(function.channels[2].range, None);
                    assert_eq!(function.channels[3].range, Some([0.1, 0.75]));
                    assert_eq!(function.channels[0].n, 1.0);
                    assert_eq!(function.channels[1].n, 2.0);
                    assert_eq!(function.channels[2].n, 3.0);
                    assert_eq!(function.channels[3].n, 4.0);
                }
                other => {
                    panic!("expected exact ranged PostScript CMYK Type 2 array sidecar: {other:?}")
                }
            },
            other => panic!(
                "expected PostScript axial shading with ranged CMYK Type 2 array metadata: {other:?}"
            ),
        }
    }

    #[test]
    fn constant_indexed_axial_shading_is_regional_vector_output() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let resources = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            indexed_rgb_space(vec![255, 0, 0, 0, 0, 255]),
            vec![PdfObject::Real(1.0)],
            vec![PdfObject::Real(1.0)],
        );
        let reader = test_reader();
        match classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("constant Indexed shading should stay regional vector: {other:?}"),
        }
    }

    #[test]
    fn postscript_declared_jpx_internal_alpha_defers_to_regional_decode() {
        let ops = vec![make_do_op("Im0")];
        let reader = test_reader();

        for smask_in_data in [1, 2] {
            let mut resources = resources_with_image("Im0");
            set_image_filter_and_smask_in_data(&mut resources, "Im0", "JPXDecode", smask_in_data);

            match classify_page_for_postscript_output_with_reader(&ops, &resources, 1.0, &reader) {
                VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
                    assert_eq!(image_names, vec!["Im0"]);
                }
                other => panic!(
                    "PostScript should defer declared JPX internal alpha /SMaskInData {smask_in_data} to regional decode: {other:?}"
                ),
            }
        }
    }

    #[test]
    fn svg_declared_jpx_internal_alpha_stays_regional_image_fallback() {
        let ops = vec![make_do_op("Im0")];
        let reader = test_reader();
        let mut resources = resources_with_image("Im0");
        set_image_filter_and_smask_in_data(&mut resources, "Im0", "JPXDecode", 1);

        match classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
                assert_eq!(image_names, vec!["Im0"]);
            }
            other => panic!(
                "SVG can embed PNG alpha, so declared JPX internal alpha should stay regional: {other:?}"
            ),
        }
    }

    #[test]
    fn postscript_jpx_without_declared_internal_alpha_stays_regional_image_fallback() {
        let ops = vec![make_do_op("Im0")];
        let reader = test_reader();
        let mut resources = resources_with_image("Im0");
        set_image_filter_and_smask_in_data(&mut resources, "Im0", "JPXDecode", 0);

        match classify_page_for_postscript_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
                assert_eq!(image_names, vec!["Im0"]);
            }
            other => panic!(
                "PostScript should keep JPX regional when /SMaskInData explicitly declares no internal alpha: {other:?}"
            ),
        }
    }

    #[test]
    fn jpx_abbreviation_image_xobject_stays_regional_vector_output() {
        let ops = vec![make_do_op("Im0")];
        let reader = test_reader();
        let mut resources = resources_with_image("Im0");
        set_image_filter_and_smask_in_data(&mut resources, "Im0", "JPX", 0);

        match classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
                assert_eq!(image_names, vec!["Im0"]);
            }
            other => panic!("SVG should accept /JPX as a JPX image filter alias: {other:?}"),
        }

        match classify_page_for_postscript_output_with_reader(&ops, &resources, 1.0, &reader) {
            VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
                assert_eq!(image_names, vec!["Im0"]);
            }
            other => panic!("PostScript should accept /JPX as a JPX image filter alias: {other:?}"),
        }
    }

    #[test]
    fn postscript_inline_jpx_declared_internal_alpha_defers_to_regional_decode() {
        let reader = test_reader();

        for smask_in_data in [1, 2] {
            let ops =
                complete_inline_image_ops(inline_jpx_params_with_smask_in_data(smask_in_data));
            match classify_page_for_postscript_output_with_reader(
                &ops,
                &PageResources::default(),
                1.0,
                &reader,
            ) {
                VectorFallbackDecision::RegionalImageFallback {
                    inline_image_count,
                    ..
                } => {
                    assert_eq!(inline_image_count, 1);
                }
                other => panic!(
                    "PostScript should defer declared inline JPX internal alpha /SMaskInData {smask_in_data} to regional decode: {other:?}"
                ),
            }
        }
    }

    #[test]
    fn jpx_abbreviation_inline_image_stays_regional_vector_output() {
        let reader = test_reader();
        let ops =
            complete_inline_image_ops(inline_jpx_params_with_filter_and_smask_in_data("JPX", 0));

        match classify_page_for_svg_output_with_reader(
            &ops,
            &PageResources::default(),
            1.0,
            &reader,
        ) {
            VectorFallbackDecision::RegionalImageFallback {
                inline_image_count, ..
            } => {
                assert_eq!(inline_image_count, 1);
            }
            other => panic!("SVG should accept inline /JPX as a JPX image filter alias: {other:?}"),
        }

        match classify_page_for_postscript_output_with_reader(
            &ops,
            &PageResources::default(),
            1.0,
            &reader,
        ) {
            VectorFallbackDecision::RegionalImageFallback {
                inline_image_count, ..
            } => {
                assert_eq!(inline_image_count, 1);
            }
            other => {
                panic!(
                    "PostScript should accept inline /JPX as a JPX image filter alias: {other:?}"
                )
            }
        }
    }

    #[test]
    fn svg_inline_jpx_declared_internal_alpha_stays_regional_image_fallback() {
        let reader = test_reader();
        let ops = complete_inline_image_ops(inline_jpx_params_with_smask_in_data(1));

        match classify_page_for_svg_output_with_reader(
            &ops,
            &PageResources::default(),
            1.0,
            &reader,
        ) {
            VectorFallbackDecision::RegionalImageFallback {
                inline_image_count,
                ..
            } => {
                assert_eq!(inline_image_count, 1);
            }
            other => panic!(
                "SVG can embed PNG alpha, so declared inline JPX internal alpha should stay regional: {other:?}"
            ),
        }
    }

    #[test]
    fn postscript_inline_jpx_without_declared_internal_alpha_stays_regional_image_fallback() {
        let reader = test_reader();
        let ops = complete_inline_image_ops(inline_jpx_params_with_smask_in_data(0));

        match classify_page_for_postscript_output_with_reader(
            &ops,
            &PageResources::default(),
            1.0,
            &reader,
        ) {
            VectorFallbackDecision::RegionalImageFallback {
                inline_image_count,
                ..
            } => {
                assert_eq!(inline_image_count, 1);
            }
            other => panic!(
                "PostScript should keep inline JPX regional when /SMaskInData explicitly declares no internal alpha: {other:?}"
            ),
        }
    }

    #[test]
    fn malformed_inline_image_parameters_stay_whole_page_fallback() {
        let reader = test_reader();
        for (params, expected) in [
            (
                vec![
                    Operand::Integer(42),
                    Operand::Name("Width".to_string()),
                    Operand::Integer(1),
                    Operand::Name("Height".to_string()),
                    Operand::Integer(1),
                    Operand::Name("BitsPerComponent".to_string()),
                    Operand::Integer(8),
                    Operand::Name("ColorSpace".to_string()),
                    Operand::Name("DeviceGray".to_string()),
                ],
                "key at position 0 is not a name",
            ),
            (
                vec![
                    Operand::Name("Width".to_string()),
                    Operand::Integer(1),
                    Operand::Name("Height".to_string()),
                    Operand::Integer(1),
                    Operand::Name("BitsPerComponent".to_string()),
                ],
                "/BitsPerComponent has no value",
            ),
            (
                vec![
                    Operand::Name("Width".to_string()),
                    Operand::Integer(1),
                    Operand::Name("Width".to_string()),
                    Operand::Integer(2),
                    Operand::Name("Height".to_string()),
                    Operand::Integer(1),
                    Operand::Name("BitsPerComponent".to_string()),
                    Operand::Integer(8),
                    Operand::Name("ColorSpace".to_string()),
                    Operand::Name("DeviceGray".to_string()),
                ],
                "duplicate /Width",
            ),
        ] {
            let err = inline_image_params_to_dict(&params)
                .expect_err("malformed inline image parameter pairs must fail");
            let message = format!("{err}");
            assert!(
                message.contains("malformed inline image parameters") && message.contains(expected),
                "expected {expected:?}, got {message}"
            );

            let ops = complete_inline_image_ops(params);
            let decision = classify_page_for_svg_output_with_reader(
                &ops,
                &PageResources::default(),
                1.0,
                &reader,
            );
            assert!(
                matches!(
                    decision,
                    VectorFallbackDecision::WholePageRaster {
                        reason: "unsupported inline image"
                    }
                ),
                "malformed inline parameters must not enter regional fallback: {decision:?}"
            );
        }
    }

    #[test]
    fn inline_monochrome_terminal_rgb_cmyk_stays_whole_page_fallback() {
        let reader = test_reader();

        for (filter, color_space) in [
            ("CCITTFaxDecode", "DeviceRGB"),
            ("CCF", "RGB"),
            ("JBIG2Decode", "DeviceCMYK"),
        ] {
            let ops = complete_inline_image_ops(inline_terminal_image_params(filter, color_space));

            let svg_decision = classify_page_for_svg_output_with_reader(
                &ops,
                &PageResources::default(),
                1.0,
                &reader,
            );
            assert!(
                matches!(svg_decision, VectorFallbackDecision::WholePageRaster { .. }),
                "SVG must not regionally emit {filter} inline image data as {color_space}: {svg_decision:?}"
            );

            let postscript_decision = classify_page_for_postscript_output_with_reader(
                &ops,
                &PageResources::default(),
                1.0,
                &reader,
            );
            assert!(
                matches!(
                    postscript_decision,
                    VectorFallbackDecision::WholePageRaster { .. }
                ),
                "PostScript must not regionally emit {filter} inline image data as {color_space}: {postscript_decision:?}"
            );
        }
    }

    #[test]
    fn inline_monochrome_terminal_device_gray_stays_regional_image_fallback() {
        let reader = test_reader();

        for filter in ["CCITTFaxDecode", "CCF", "JBIG2Decode"] {
            let ops = complete_inline_image_ops(inline_terminal_image_params(filter, "DeviceGray"));

            match classify_page_for_svg_output_with_reader(
                &ops,
                &PageResources::default(),
                1.0,
                &reader,
            ) {
                VectorFallbackDecision::RegionalImageFallback {
                    inline_image_count, ..
                } => {
                    assert_eq!(inline_image_count, 1);
                }
                other => {
                    panic!("SVG should keep {filter} DeviceGray inline images regional: {other:?}")
                }
            }

            match classify_page_for_postscript_output_with_reader(
                &ops,
                &PageResources::default(),
                1.0,
                &reader,
            ) {
                VectorFallbackDecision::RegionalImageFallback {
                    inline_image_count, ..
                } => {
                    assert_eq!(inline_image_count, 1);
                }
                other => panic!(
                    "PostScript should keep {filter} DeviceGray inline images regional: {other:?}"
                ),
            }
        }
    }

    #[test]
    fn indexed_color_transition_stays_whole_page_fallback() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let resources = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            indexed_rgb_space(vec![255, 0, 0, 0, 0, 255]),
            vec![PdfObject::Real(0.0)],
            vec![PdfObject::Real(1.0)],
        );
        let reader = test_reader();
        let decision = classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, &reader);
        assert!(
            matches!(decision, VectorFallbackDecision::WholePageRaster { .. }),
            "Indexed color transitions must not become smooth native gradients: {decision:?}"
        );
    }

    #[test]
    fn malformed_indexed_axial_shading_stays_whole_page_fallback() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let reader = test_reader();
        for (label, color_space, c0, c1) in [
            (
                "short lookup",
                indexed_rgb_space(vec![255, 0, 0]),
                vec![PdfObject::Real(0.0)],
                vec![PdfObject::Real(0.0)],
            ),
            (
                "non-integer index",
                indexed_rgb_space(vec![255, 0, 0, 0, 0, 255]),
                vec![PdfObject::Real(0.5)],
                vec![PdfObject::Real(0.5)],
            ),
        ] {
            let resources =
                resources_with_axial_shading_color_space("Sh0", 1.0, color_space, c0, c1);
            let decision = classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, &reader);
            assert!(
                matches!(decision, VectorFallbackDecision::WholePageRaster { .. }),
                "{label} must not enter native Indexed vector shading output: {decision:?}"
            );
        }
    }

    #[test]
    fn malformed_function_array_shading_stays_whole_page_fallback() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let r = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            PdfObject::Name("DeviceRGB".to_string()),
            vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Name("Bad".to_string()),
            ],
            vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
            ],
        );
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn overlong_vector_shading_components_stay_whole_page_fallback() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let valid_xyz = PdfObject::Array(vec![
            PdfObject::Real(0.9505),
            PdfObject::Real(1.0),
            PdfObject::Real(1.089),
        ]);
        let valid_lab_white = PdfObject::Array(vec![
            PdfObject::Real(0.9642),
            PdfObject::Real(1.0),
            PdfObject::Real(0.8249),
        ]);
        let cases = [
            (
                "DeviceGray",
                PdfObject::Name("DeviceGray".to_string()),
                vec![0.0, 0.5],
                vec![1.0, 0.25],
            ),
            (
                "DeviceRGB",
                PdfObject::Name("DeviceRGB".to_string()),
                vec![1.0, 0.0, 0.0, 0.25],
                vec![0.0, 0.0, 1.0, 0.75],
            ),
            (
                "DeviceCMYK",
                PdfObject::Name("DeviceCMYK".to_string()),
                vec![0.0, 1.0, 1.0, 0.0, 0.25],
                vec![1.0, 0.0, 0.0, 0.0, 0.75],
            ),
            (
                "CalGray",
                {
                    let mut params = PdfDictionary::empty();
                    params.insert("WhitePoint", valid_xyz.clone());
                    params.insert("Gamma", PdfObject::Real(1.0));
                    PdfObject::Array(vec![
                        PdfObject::Name("CalGray".to_string()),
                        PdfObject::Dictionary(params),
                    ])
                },
                vec![0.0, 0.5],
                vec![1.0, 0.25],
            ),
            (
                "CalRGB",
                {
                    let mut params = PdfDictionary::empty();
                    params.insert("WhitePoint", valid_xyz);
                    params.insert(
                        "Gamma",
                        PdfObject::Array(vec![
                            PdfObject::Real(1.0),
                            PdfObject::Real(1.0),
                            PdfObject::Real(1.0),
                        ]),
                    );
                    PdfObject::Array(vec![
                        PdfObject::Name("CalRGB".to_string()),
                        PdfObject::Dictionary(params),
                    ])
                },
                vec![1.0, 0.0, 0.0, 0.25],
                vec![0.0, 0.0, 1.0, 0.75],
            ),
            (
                "Lab",
                {
                    let mut params = PdfDictionary::empty();
                    params.insert("WhitePoint", valid_lab_white);
                    params.insert(
                        "Range",
                        PdfObject::Array(vec![
                            PdfObject::Real(-100.0),
                            PdfObject::Real(100.0),
                            PdfObject::Real(-100.0),
                            PdfObject::Real(100.0),
                        ]),
                    );
                    PdfObject::Array(vec![
                        PdfObject::Name("Lab".to_string()),
                        PdfObject::Dictionary(params),
                    ])
                },
                vec![100.0, 0.0, 0.0, 0.25],
                vec![50.0, 80.0, 60.0, 0.75],
            ),
        ];

        for (label, color_space, c0, c1) in cases {
            let resources = resources_with_axial_shading_color_space(
                "Sh0",
                1.0,
                color_space,
                c0.into_iter().map(PdfObject::Real).collect(),
                c1.into_iter().map(PdfObject::Real).collect(),
            );
            let decision = classify_page_for_vector_output(&ops, &resources, 1.0);
            assert!(
                matches!(decision, VectorFallbackDecision::WholePageRaster { .. }),
                "{label} overlong components must not enter native SVG/PS vector shading output: {decision:?}"
            );
        }
    }

    #[test]
    fn vector_calibrated_shading_rejects_overlong_color_space_array() {
        let mut params = PdfDictionary::empty();
        params.insert(
            "WhitePoint",
            PdfObject::Array(vec![
                PdfObject::Real(1.0),
                PdfObject::Real(1.0),
                PdfObject::Real(1.0),
            ]),
        );
        let color_space = PdfObject::Array(vec![
            PdfObject::Name("CalRGB".to_string()),
            PdfObject::Dictionary(params),
            PdfObject::Name("Ignored".to_string()),
        ]);

        assert!(
            vector_calibrated_shading_space_is_valid(&color_space, None).is_none(),
            "vector calibrated shading must not ignore trailing ColorSpace entries"
        );
    }

    #[test]
    fn malformed_shading_geometry_stays_whole_page_fallback() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];

        for (key, value) in [
            (
                "Coords",
                PdfObject::Array(vec![
                    PdfObject::Real(0.0),
                    PdfObject::Real(0.0),
                    PdfObject::Real(100.0),
                    PdfObject::Real(0.0),
                    PdfObject::Name("Bad".to_string()),
                ]),
            ),
            (
                "Extend",
                PdfObject::Array(vec![
                    PdfObject::Boolean(true),
                    PdfObject::Boolean(true),
                    PdfObject::Name("Bad".to_string()),
                ]),
            ),
            (
                "BBox",
                PdfObject::Array(vec![
                    PdfObject::Real(0.0),
                    PdfObject::Real(0.0),
                    PdfObject::Real(100.0),
                ]),
            ),
            ("Domain", PdfObject::Array(vec![PdfObject::Real(0.25)])),
            (
                "Domain",
                PdfObject::Array(vec![PdfObject::Real(0.75), PdfObject::Real(0.75)]),
            ),
        ] {
            let mut resources = resources_with_axial_shading("Sh0", 1.0);
            shading_dict_mut(&mut resources, "Sh0").insert(key, value);
            let decision = classify_page_for_vector_output(&ops, &resources, 1.0);
            assert!(
                matches!(decision, VectorFallbackDecision::WholePageRaster { .. }),
                "malformed or unsupported /{key} must not classify as native vector shading: {decision:?}"
            );
        }
    }

    #[test]
    fn shading_domain_outside_function_domain_is_regional_vector_output() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut resources = resources_with_axial_shading("Sh0", 1.0);
        shading_dict_mut(&mut resources, "Sh0").insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Real(-0.25), PdfObject::Real(0.75)]),
        );
        let decision = classify_page_for_vector_output(&ops, &resources, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("clipped Type 2 function-domain axial shading should stay regional vector: {other:?}"),
        }
    }

    #[test]
    fn calgray_shading_without_whitepoint_stays_whole_page_fallback() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut params = PdfDictionary::empty();
        params.insert("Gamma", PdfObject::Real(1.0));
        let r = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            PdfObject::Array(vec![
                PdfObject::Name("CalGray".to_string()),
                PdfObject::Dictionary(params),
            ]),
            vec![PdfObject::Real(0.0)],
            vec![PdfObject::Real(1.0)],
        );
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn calrgb_shading_with_malformed_gamma_stays_whole_page_fallback() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut params = PdfDictionary::empty();
        params.insert(
            "WhitePoint",
            PdfObject::Array(vec![
                PdfObject::Real(0.9505),
                PdfObject::Real(1.0),
                PdfObject::Real(1.089),
            ]),
        );
        params.insert("Gamma", PdfObject::Real(1.0));
        let r = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            PdfObject::Array(vec![
                PdfObject::Name("CalRGB".to_string()),
                PdfObject::Dictionary(params),
            ]),
            vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ],
            vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
            ],
        );
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn lab_shading_with_malformed_range_stays_whole_page_fallback() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut params = PdfDictionary::empty();
        params.insert(
            "WhitePoint",
            PdfObject::Array(vec![
                PdfObject::Real(0.9642),
                PdfObject::Real(1.0),
                PdfObject::Real(0.8249),
            ]),
        );
        params.insert(
            "Range",
            PdfObject::Array(vec![
                PdfObject::Real(100.0),
                PdfObject::Real(-100.0),
                PdfObject::Real(-100.0),
                PdfObject::Real(100.0),
            ]),
        );
        let r = resources_with_axial_shading_color_space(
            "Sh0",
            1.0,
            PdfObject::Array(vec![
                PdfObject::Name("Lab".to_string()),
                PdfObject::Dictionary(params),
            ]),
            vec![
                PdfObject::Real(100.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ],
            vec![
                PdfObject::Real(50.0),
                PdfObject::Real(80.0),
                PdfObject::Real(60.0),
            ],
        );
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn simple_radial_shading_is_regional_vector_output() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let r = resources_with_radial_shading("Sh0");
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn radial_shading_with_nonzero_start_radius_is_regional_vector_output() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_radial_shading("Sh0");
        shading_dict_mut(&mut r, "Sh0").insert(
            "Coords",
            PdfObject::Array(vec![
                PdfObject::Real(45.0),
                PdfObject::Real(50.0),
                PdfObject::Real(10.0),
                PdfObject::Real(50.0),
                PdfObject::Real(50.0),
                PdfObject::Real(40.0),
            ]),
        );
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn radial_shading_with_reversed_radii_is_regional_vector_output() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh0".to_string())],
        )];
        let mut r = resources_with_radial_shading("Sh0");
        shading_dict_mut(&mut r, "Sh0").insert(
            "Coords",
            PdfObject::Array(vec![
                PdfObject::Real(50.0),
                PdfObject::Real(50.0),
                PdfObject::Real(40.0),
                PdfObject::Real(45.0),
                PdfObject::Real(50.0),
                PdfObject::Real(10.0),
            ]),
        );
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn radial_shading_with_nonuniform_ctm_is_regional_vector_output() {
        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(2.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                    Operand::Real(1.0),
                    Operand::Real(0.0),
                    Operand::Real(0.0),
                ],
            ),
            ContentOperation::new("sh", vec![Operand::Name("Sh0".to_string())]),
        ];
        let r = resources_with_radial_shading("Sh0");
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { shading_names, .. } => {
                assert_eq!(shading_names, vec!["Sh0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn rotated_image_is_regional_affine_fallback() {
        // 45-degree rotation: ctm[1] and ctm[2] are non-zero.
        let ops = vec![
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(141.0),
                    Operand::Real(141.0),  // non-zero b = rotation
                    Operand::Real(-141.0), // non-zero c = rotation
                    Operand::Real(141.0),
                    Operand::Real(50.0),
                    Operand::Real(300.0),
                ],
            ),
            make_do_op("Im0"),
        ];
        let r = resources_with_image("Im0");
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        match decision {
            VectorFallbackDecision::RegionalImageFallback { image_names, .. } => {
                assert_eq!(image_names, vec!["Im0"]);
            }
            other => panic!("Expected RegionalImageFallback, got {:?}", other),
        }
    }

    #[test]
    fn inline_stencil_mask_to_rgba_refuses_short_mask_buffer() {
        let raw = RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![255],
        };
        let error = inline_stencil_mask_to_rgba(&raw, [255, 0, 0, 255], true)
            .expect_err("short regional stencil masks must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("invalid regional stencil mask 2x1 x1 channels"));
    }

    #[test]
    fn inline_image_triggers_whole_page() {
        let r = PageResources::default();
        let inline_params = || {
            vec![
                Operand::Name("Width".to_string()),
                Operand::Integer(1),
                Operand::Name("Height".to_string()),
                Operand::Integer(1),
                Operand::Name("ColorSpace".to_string()),
                Operand::Name("DeviceGray".to_string()),
                Operand::Name("BitsPerComponent".to_string()),
                Operand::Integer(8),
            ]
        };
        for ops in [
            vec![ContentOperation::new("BI", vec![])],
            vec![ContentOperation::new("ID", inline_params())],
            vec![ContentOperation::new(
                "inline_image_data",
                vec![Operand::String(vec![0x80])],
            )],
            vec![
                ContentOperation::new("BI", vec![]),
                ContentOperation::new("EI", vec![]),
            ],
            vec![
                ContentOperation::new("BI", vec![]),
                ContentOperation::new("ID", inline_params()),
                ContentOperation::new("inline_image_data", vec![Operand::String(vec![0x80])]),
            ],
        ] {
            let decision = classify_page_for_vector_output(&ops, &r, 1.0);
            assert!(matches!(
                decision,
                VectorFallbackDecision::WholePageRaster { .. }
            ));
        }
    }

    #[test]
    fn unresolved_gs_operator_triggers_whole_page() {
        let ops = vec![ContentOperation::new(
            "gs",
            vec![Operand::Name("GS0".into())],
        )];
        let r = PageResources::default();
        let decision = classify_page_for_vector_output(&ops, &r, 1.0);
        assert!(matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn text_clipping_render_modes_without_font_preflight_stay_whole_page_fallback() {
        for mode in 4..=7 {
            let ops = vec![
                ContentOperation::new("BT", vec![]),
                ContentOperation::new("Tr", vec![Operand::Integer(mode)]),
                ContentOperation::new("Tj", vec![Operand::String(b"H".to_vec())]),
                ContentOperation::new("ET", vec![]),
            ];
            let decision = classify_page_for_vector_output(&ops, &PageResources::default(), 1.0);
            assert!(
                matches!(
                    decision,
                    VectorFallbackDecision::WholePageRaster {
                        reason: "text clipping font preflight unavailable"
                    }
                ),
                "text clipping mode {mode} must not be emitted as incomplete vector output: {decision:?}"
            );
        }
    }

    #[test]
    fn ordinary_text_without_metric_preflight_stays_whole_page_fallback() {
        let ops = vec![
            ContentOperation::new("BT", vec![]),
            ContentOperation::new("Tj", vec![Operand::String(b"H".to_vec())]),
            ContentOperation::new("ET", vec![]),
        ];
        let decision = classify_page_for_vector_output(&ops, &PageResources::default(), 1.0);
        assert_eq!(
            decision,
            VectorFallbackDecision::WholePageRaster {
                reason: "text metrics unavailable for vector output"
            }
        );
    }

    #[test]
    fn tj_string_operand_collection_rejects_invalid_items() {
        let valid = ContentOperation::new(
            "TJ",
            vec![Operand::Array(vec![
                Operand::String(b"A".to_vec()),
                Operand::Integer(-120),
                Operand::Real(15.5),
                Operand::String(b"B".to_vec()),
            ])],
        );
        let strings = text_operator_string_operands(&valid).expect("valid TJ operands");
        assert_eq!(strings, vec![b"A".as_slice(), b"B".as_slice()]);

        let invalid = ContentOperation::new(
            "TJ",
            vec![Operand::Array(vec![
                Operand::String(b"A".to_vec()),
                Operand::Name("Bad".to_string()),
            ])],
        );
        assert!(text_operator_string_operands(&invalid).is_none());
    }

    #[test]
    fn safe_line_style_ext_gstate_remains_vector_safe() {
        let ops = vec![ContentOperation::new(
            "gs",
            vec![Operand::Name("GS0".into())],
        )];
        let mut ext = PdfDictionary::empty();
        ext.insert("Type", PdfObject::Name("ExtGState".to_string()));
        ext.insert("LW", PdfObject::Real(2.0));
        ext.insert("LC", PdfObject::Integer(1));
        ext.insert("LJ", PdfObject::Integer(2));
        ext.insert("ML", PdfObject::Real(10.0));
        ext.insert("FL", PdfObject::Real(0.5));
        ext.insert("SM", PdfObject::Real(0.02));
        ext.insert(
            "Font",
            PdfObject::Array(vec![
                PdfObject::Name("F1".to_string()),
                PdfObject::Real(12.0),
            ]),
        );
        ext.insert(
            "D",
            PdfObject::Array(vec![
                PdfObject::Array(vec![PdfObject::Real(6.0), PdfObject::Real(2.0)]),
                PdfObject::Real(1.0),
            ]),
        );
        ext.insert("RI", PdfObject::Name("RelativeColorimetric".to_string()));
        ext.insert("OP", PdfObject::Boolean(false));
        ext.insert("op", PdfObject::Boolean(false));
        ext.insert("OPM", PdfObject::Integer(1));
        ext.insert("SA", PdfObject::Boolean(false));
        ext.insert("AIS", PdfObject::Boolean(false));
        ext.insert("TK", PdfObject::Boolean(true));
        ext.insert("CA", PdfObject::Real(1.0));
        ext.insert("ca", PdfObject::Real(1.0));
        ext.insert("BM", PdfObject::Name("Normal".to_string()));
        ext.insert("SMask", PdfObject::Name("None".to_string()));
        ext.insert("TR", PdfObject::Name("Identity".to_string()));
        ext.insert("TR2", PdfObject::Name("Identity".to_string()));
        let mut resources = PageResources::default();
        resources.ext_g_states.insert("GS0".to_string(), ext);

        assert_eq!(
            classify_page_for_vector_output(&ops, &resources, 1.0),
            VectorFallbackDecision::PureVector
        );
    }

    #[test]
    fn noop_blend_and_transfer_arrays_remain_vector_safe() {
        let ops = vec![ContentOperation::new(
            "gs",
            vec![Operand::Name("GS0".into())],
        )];
        let mut ext = PdfDictionary::empty();
        ext.insert(
            "BM",
            PdfObject::Array(vec![
                PdfObject::Name("VendorMode".to_string()),
                PdfObject::Name("Compatible".to_string()),
                PdfObject::Name("Multiply".to_string()),
            ]),
        );
        ext.insert(
            "TR",
            PdfObject::Array(vec![
                PdfObject::Name("Identity".to_string()),
                PdfObject::Name("Identity".to_string()),
                PdfObject::Name("Identity".to_string()),
                PdfObject::Name("Identity".to_string()),
            ]),
        );
        ext.insert(
            "TR2",
            PdfObject::Array(vec![
                PdfObject::Name("Identity".to_string()),
                PdfObject::Name("Identity".to_string()),
                PdfObject::Name("Identity".to_string()),
                PdfObject::Name("Identity".to_string()),
            ]),
        );
        let mut resources = PageResources::default();
        resources.ext_g_states.insert("GS0".to_string(), ext);

        assert_eq!(
            classify_page_for_vector_output(&ops, &resources, 1.0),
            VectorFallbackDecision::PureVector
        );
    }

    #[test]
    fn visible_postscript_alpha_or_blend_paint_stays_whole_page() {
        for (key, value, stroke_paint) in [
            ("ca", PdfObject::Real(0.5), false),
            ("CA", PdfObject::Real(0.5), true),
            ("BM", PdfObject::Name("Multiply".to_string()), false),
            (
                "BM",
                PdfObject::Array(vec![
                    PdfObject::Name("VendorMode".to_string()),
                    PdfObject::Name("Multiply".to_string()),
                    PdfObject::Name("Normal".to_string()),
                ]),
                false,
            ),
        ] {
            let mut ops = vec![ContentOperation::new(
                "gs",
                vec![Operand::Name("GS0".into())],
            )];
            if stroke_paint {
                ops.extend(make_stroke_path_ops());
            } else {
                ops.extend(make_path_ops());
            }
            let mut ext = PdfDictionary::empty();
            ext.insert("Type", PdfObject::Name("ExtGState".to_string()));
            ext.insert(key, value);
            let mut resources = PageResources::default();
            resources.ext_g_states.insert("GS0".to_string(), ext);

            assert!(
                matches!(
                    classify_ops_for_vector_output(
                        &ops,
                        &resources,
                        1.0,
                        None,
                        GraphicsState::default(),
                        VectorOutputTarget::PostScript,
                        &mut Vec::new(),
                    ),
                    VectorFallbackDecision::WholePageRaster {
                        reason: "unsupported PostScript paint alpha/blend"
                    }
                ),
                "{key} visible PostScript paint must remain a whole-page fallback"
            );
        }
    }

    #[test]
    fn malformed_or_semantic_ext_gstate_stays_whole_page() {
        for (key, value) in [
            (
                "BM",
                PdfObject::Array(vec![
                    PdfObject::Name("Normal".to_string()),
                    PdfObject::Integer(1),
                ]),
            ),
            ("SMask", PdfObject::Dictionary(PdfDictionary::empty())),
            ("OP", PdfObject::Boolean(true)),
            ("op", PdfObject::Boolean(true)),
            ("SA", PdfObject::Boolean(true)),
            ("AIS", PdfObject::Boolean(true)),
            ("TK", PdfObject::Boolean(false)),
            ("LW", PdfObject::Real(-1.0)),
            ("LW", PdfObject::Real(f64::NAN)),
            ("ML", PdfObject::Real(0.0)),
            ("ML", PdfObject::Real(f64::NAN)),
            ("LC", PdfObject::Integer(3)),
            ("LJ", PdfObject::Integer(-1)),
            ("FL", PdfObject::Real(-1.0)),
            ("SM", PdfObject::Real(f64::NAN)),
            (
                "Font",
                PdfObject::Array(vec![
                    PdfObject::Name("F1".to_string()),
                    PdfObject::Real(f64::NAN),
                ]),
            ),
            (
                "Font",
                PdfObject::Array(vec![
                    PdfObject::String(b"not-a-name".to_vec()),
                    PdfObject::Real(12.0),
                ]),
            ),
            ("TR", PdfObject::Name("Default".to_string())),
            ("TR2", PdfObject::Name("Default".to_string())),
            (
                "TR",
                PdfObject::Array(vec![
                    PdfObject::Name("Identity".to_string()),
                    PdfObject::Name("Default".to_string()),
                    PdfObject::Name("Identity".to_string()),
                    PdfObject::Name("Identity".to_string()),
                ]),
            ),
            ("OPM", PdfObject::Integer(2)),
        ] {
            let ops = vec![ContentOperation::new(
                "gs",
                vec![Operand::Name("GS0".into())],
            )];
            let mut ext = PdfDictionary::empty();
            ext.insert("Type", PdfObject::Name("ExtGState".to_string()));
            ext.insert(key, value);
            let mut resources = PageResources::default();
            resources.ext_g_states.insert("GS0".to_string(), ext);

            assert!(
                matches!(
                    classify_page_for_vector_output(&ops, &resources, 1.0),
                    VectorFallbackDecision::WholePageRaster {
                        reason: "unsupported ExtGState"
                    }
                ),
                "{key} must remain a whole-page fallback"
            );
        }
    }

    #[test]
    fn postscript_transparent_non_normal_blend_paint_is_stateful_vector_noop() {
        let mut ext = PdfDictionary::empty();
        ext.insert("ca", PdfObject::Real(0.0));
        ext.insert("CA", PdfObject::Real(0.25));
        ext.insert("BM", PdfObject::Name("Multiply".to_string()));
        let mut resources = PageResources::default();
        resources.ext_g_states.insert("GS0".to_string(), ext);
        let mut ops = vec![ContentOperation::new(
            "gs",
            vec![Operand::Name("GS0".into())],
        )];
        ops.extend(make_path_ops());

        assert!(matches!(
            classify_ops_for_vector_output(
                &ops,
                &resources,
                1.0,
                None,
                GraphicsState::default(),
                VectorOutputTarget::PostScript,
                &mut Vec::new(),
            ),
            VectorFallbackDecision::PureVector
        ));
        assert!(matches!(
            classify_page_for_vector_output(&ops, &resources, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn postscript_stroke_noop_ignores_fractional_fill_alpha() {
        let mut ext = PdfDictionary::empty();
        ext.insert("CA", PdfObject::Real(0.0));
        ext.insert("ca", PdfObject::Real(0.25));
        ext.insert("BM", PdfObject::Name("Multiply".to_string()));
        let mut resources = PageResources::default();
        resources.ext_g_states.insert("GS0".to_string(), ext);
        let mut ops = vec![ContentOperation::new(
            "gs",
            vec![Operand::Name("GS0".into())],
        )];
        ops.extend(make_stroke_path_ops());

        assert!(matches!(
            classify_ops_for_vector_output(
                &ops,
                &resources,
                1.0,
                None,
                GraphicsState::default(),
                VectorOutputTarget::PostScript,
                &mut Vec::new(),
            ),
            VectorFallbackDecision::PureVector
        ));
    }

    #[test]
    fn postscript_visible_non_normal_blend_or_alpha_remains_conservative() {
        for (stroke_alpha, fill_alpha, stroke_paint) in [
            (Some(0.0), None, false),
            (None, Some(0.0), true),
            (Some(0.0), Some(0.25), false),
            (Some(0.25), Some(0.0), true),
            (Some(-0.1), Some(0.0), true),
        ] {
            let mut ext = PdfDictionary::empty();
            if let Some(alpha) = stroke_alpha {
                ext.insert("CA", PdfObject::Real(alpha));
            }
            if let Some(alpha) = fill_alpha {
                ext.insert("ca", PdfObject::Real(alpha));
            }
            ext.insert("BM", PdfObject::Name("Multiply".to_string()));
            let mut resources = PageResources::default();
            resources.ext_g_states.insert("GS0".to_string(), ext);
            let mut ops = vec![ContentOperation::new(
                "gs",
                vec![Operand::Name("GS0".into())],
            )];
            if stroke_paint {
                ops.extend(make_stroke_path_ops());
            } else {
                ops.extend(make_path_ops());
            }

            assert!(matches!(
                classify_ops_for_vector_output(
                    &ops,
                    &resources,
                    1.0,
                    None,
                    GraphicsState::default(),
                    VectorOutputTarget::PostScript,
                    &mut Vec::new(),
                ),
                VectorFallbackDecision::WholePageRaster { .. }
            ), "partial or malformed no-paint state must stay conservative: CA={stroke_alpha:?}, ca={fill_alpha:?}");
        }
    }

    #[test]
    fn image_device_rect_computes_correct_coordinates() {
        let mut gs = GraphicsState::default();
        // Typical image placement: 200pt wide, 100pt tall, at (50, 400) with negative ctm[3].
        gs.ctm = [200.0, 0.0, 0.0, -100.0, 50.0, 400.0];
        let scale = 1.0;
        let page_h = 800.0;
        let rect = image_device_rect(&gs, scale, page_h).unwrap();
        // device_x = 50, device_y_top = 800 - 400 = 400, w = 200, h = 100
        assert!((rect[0] - 50.0).abs() < 0.01);
        assert!((rect[1] - 400.0).abs() < 0.01);
        assert!((rect[2] - 200.0).abs() < 0.01);
        assert!((rect[3] - 100.0).abs() < 0.01);
    }
}
