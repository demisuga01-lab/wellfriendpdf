//! Display-list capture and replay for PDF rendering.
//!
//! This module captures a normalized, replayable page drawing program. Vector
//! path operations carry their draw state directly, while text, images, XObjects,
//! shadings, patterns, and transparency-sensitive operations are represented as
//! native high-level replay operations through the canonical renderer state.

use crate::cancel::CancelToken;
use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::{BlendMode, Color, ColorSpace, GraphicsState, LineCap, LineJoin};
use crate::engine::PageResources;
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};
use crate::render::buffer::{ClipMask, PixelBuffer, PixelColor, RenderMode, WHITE};
use crate::render::clip_dag::{ClipDag, ClipNode, ClipState};
use crate::render::color::ColorSpaceHandler;
use crate::render::line::DashState;
use crate::render::path::{
    axis_aligned_integer_rect, flatten_path, flatten_path_device_transform,
    rasterize_flat_alpha_mask, rasterize_path_alpha_mask, stroke_flat_path, FillRule, Path,
    PathPainter, RasterizedGlyphMask,
};
use crate::render::plan::{
    graphics_state_color_component_arity_refusal, graphics_state_operand_refusal,
    marked_content_operand_refusal, path_operand_refusal, resource_invocation_operand_refusal,
    text_operand_refusal, type3_glyph_metric_operand_refusal, GraphicsStateDescriptor,
    PatternPaintPhase, PatternPathDescriptor,
};
use crate::render::transform::{Transform2D, Viewport};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// A replayable page-level drawing program.
#[derive(Debug, Clone)]
pub struct DisplayList {
    pub viewport: Viewport,
    pub ops: Vec<DisplayOp>,
    pub stats: DisplayListStats,
    pub supported: bool,
    pub unsupported: Vec<UnsupportedRenderOp>,
}

impl DisplayList {
    pub fn is_fully_supported(&self) -> bool {
        self.supported && self.unsupported.is_empty()
    }

    pub fn native_vector_only(&self) -> bool {
        self.is_fully_supported() && !self.ops.iter().any(DisplayOp::is_native_high_level)
    }

    pub fn approximate_memory_bytes(&self) -> usize {
        let path_bytes: usize = self
            .ops
            .iter()
            .map(|op| match op {
                DisplayOp::Clip { path, .. }
                | DisplayOp::FillPath { path, .. }
                | DisplayOp::StrokePath { path, .. } => {
                    std::mem::size_of_val(path.segments.as_slice())
                }
                DisplayOp::StateOp { approx_bytes, .. } => *approx_bytes,
                DisplayOp::NativeTextOp { approx_bytes, .. }
                | DisplayOp::NativeImageXObject { approx_bytes, .. }
                | DisplayOp::NativeShadingOp { approx_bytes, .. }
                | DisplayOp::NativePatternPathOp { approx_bytes, .. }
                | DisplayOp::NativeFormXObject { approx_bytes, .. }
                | DisplayOp::NativeInlineImage { approx_bytes, .. } => *approx_bytes,
                DisplayOp::Save | DisplayOp::Restore => 0,
            })
            .sum();
        std::mem::size_of::<Self>()
            + self.ops.len() * std::mem::size_of::<DisplayOp>()
            + path_bytes
            + self.unsupported.len() * std::mem::size_of::<UnsupportedRenderOp>()
    }
}

/// Normalized display-list operation.
#[derive(Debug, Clone)]
pub enum DisplayOp {
    Save,
    Restore,
    Clip {
        path: Path,
        ctm: Transform2D,
        rule: FillRule,
        bounds: Option<RenderBounds>,
    },
    FillPath {
        path: Path,
        state: DrawState,
        rule: FillRule,
        bounds: Option<RenderBounds>,
    },
    StrokePath {
        path: Path,
        state: DrawState,
        bounds: Option<RenderBounds>,
    },
    /// Replayable graphics-state mutation needed before native high-level ops.
    ///
    /// Direct vector replay ignores this because normalized path ops already
    /// carry captured draw state. RenderState replay dispatches it before native
    /// text, image, and Form XObject operations.
    StateOp {
        state: GraphicsStateDescriptor,
        approx_bytes: usize,
    },
    /// Native replay of one text/text-state operation through the page
    /// renderer's glyph path.
    NativeTextOp {
        text: RetainedTextOp,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of an Image XObject `Do` operation.
    NativeImageXObject {
        name: String,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of a named shading `sh` operation.
    NativeShadingOp {
        name: String,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of a path paint through the page renderer's path state.
    ///
    /// This is used when canonical `RenderState` must participate in the paint
    /// operation, for example active tiling/shading patterns or an ExtGState
    /// soft mask. Direct vector replay intentionally bypasses `RenderState`, so
    /// it must not be used for those stateful cases.
    NativePatternPathOp {
        pattern: PatternPathDescriptor,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of an inline image `ID` plus payload operation.
    NativeInlineImage {
        image: RetainedInlineImage,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of a Form XObject `Do` operation.
    NativeFormXObject {
        name: String,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum RetainedTextArrayItem {
    Bytes(Vec<u8>),
    Adjustment(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RetainedTextOp {
    BeginText,
    EndText,
    SetFont {
        name: String,
        size: f64,
    },
    MoveTextPosition {
        tx: f64,
        ty: f64,
    },
    MoveTextPositionSetLeading {
        tx: f64,
        ty: f64,
    },
    SetTextMatrix {
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        e: f64,
        f: f64,
    },
    NextLine,
    SetCharSpacing(f64),
    SetWordSpacing(f64),
    SetHorizontalScaling(f64),
    SetTextLeading(f64),
    SetTextRenderingMode(i32),
    SetTextRise(f64),
    Show(Vec<u8>),
    ShowArray(Vec<RetainedTextArrayItem>),
    NextLineShow(Vec<u8>),
    SpacingNextLineShow {
        word_spacing: f64,
        char_spacing: f64,
        text: Vec<u8>,
    },
    Unsupported {
        operator: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetainedInlineImage {
    pub params: Vec<Operand>,
    pub data: Vec<u8>,
}

impl RetainedTextOp {
    pub fn from_content_operation(op: &ContentOperation) -> Self {
        if let Some(reason) = text_operand_refusal(op) {
            return Self::Unsupported {
                operator: op.operator.clone(),
                reason,
            };
        }

        match op.operator.as_str() {
            "BT" => Self::BeginText,
            "ET" => Self::EndText,
            "Tf" => Self::SetFont {
                name: op.name(0).unwrap_or("").to_string(),
                size: op.number(1).unwrap_or(0.0),
            },
            "Td" => Self::MoveTextPosition {
                tx: op.number(0).unwrap_or(0.0),
                ty: op.number(1).unwrap_or(0.0),
            },
            "TD" => Self::MoveTextPositionSetLeading {
                tx: op.number(0).unwrap_or(0.0),
                ty: op.number(1).unwrap_or(0.0),
            },
            "Tm" => Self::SetTextMatrix {
                a: op.number(0).unwrap_or(1.0),
                b: op.number(1).unwrap_or(0.0),
                c: op.number(2).unwrap_or(0.0),
                d: op.number(3).unwrap_or(1.0),
                e: op.number(4).unwrap_or(0.0),
                f: op.number(5).unwrap_or(0.0),
            },
            "T*" => Self::NextLine,
            "Tc" => Self::SetCharSpacing(op.number(0).unwrap_or(0.0)),
            "Tw" => Self::SetWordSpacing(op.number(0).unwrap_or(0.0)),
            "Tz" => Self::SetHorizontalScaling(op.number(0).unwrap_or(100.0)),
            "TL" => Self::SetTextLeading(op.number(0).unwrap_or(0.0)),
            "Tr" => Self::SetTextRenderingMode(op.number(0).unwrap_or(0.0) as i32),
            "Ts" => Self::SetTextRise(op.number(0).unwrap_or(0.0)),
            "Tj" => Self::Show(op.string_bytes(0).unwrap_or(&[]).to_vec()),
            "TJ" => Self::ShowArray(
                op.operand(0)
                    .and_then(Operand::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| match item {
                                Operand::String(bytes) => {
                                    Some(RetainedTextArrayItem::Bytes(bytes.clone()))
                                }
                                Operand::Integer(value) => {
                                    Some(RetainedTextArrayItem::Adjustment(-(*value as f64)))
                                }
                                Operand::Real(value) => {
                                    Some(RetainedTextArrayItem::Adjustment(-*value))
                                }
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
            "'" => Self::NextLineShow(op.string_bytes(0).unwrap_or(&[]).to_vec()),
            "\"" => Self::SpacingNextLineShow {
                word_spacing: op.number(0).unwrap_or(0.0),
                char_spacing: op.number(1).unwrap_or(0.0),
                text: op.string_bytes(2).unwrap_or(&[]).to_vec(),
            },
            operator => Self::Unsupported {
                operator: operator.to_string(),
                reason: format!("unsupported retained text operator '{operator}'"),
            },
        }
    }

    pub fn operator_name(&self) -> &'static str {
        match self {
            Self::BeginText => "BT",
            Self::EndText => "ET",
            Self::SetFont { .. } => "Tf",
            Self::MoveTextPosition { .. } => "Td",
            Self::MoveTextPositionSetLeading { .. } => "TD",
            Self::SetTextMatrix { .. } => "Tm",
            Self::NextLine => "T*",
            Self::SetCharSpacing(_) => "Tc",
            Self::SetWordSpacing(_) => "Tw",
            Self::SetHorizontalScaling(_) => "Tz",
            Self::SetTextLeading(_) => "TL",
            Self::SetTextRenderingMode(_) => "Tr",
            Self::SetTextRise(_) => "Ts",
            Self::Show(_) => "Tj",
            Self::ShowArray(_) => "TJ",
            Self::NextLineShow(_) => "'",
            Self::SpacingNextLineShow { .. } => "\"",
            Self::Unsupported { .. } => "unsupported-text",
        }
    }
}

impl DisplayOp {
    pub fn is_native_high_level(&self) -> bool {
        matches!(
            self,
            DisplayOp::NativeTextOp { .. }
                | DisplayOp::NativeImageXObject { .. }
                | DisplayOp::NativeShadingOp { .. }
                | DisplayOp::NativePatternPathOp { .. }
                | DisplayOp::NativeInlineImage { .. }
                | DisplayOp::NativeFormXObject { .. }
        )
    }

    pub fn bounds(&self) -> Option<RenderBounds> {
        match self {
            DisplayOp::Clip { bounds, .. }
            | DisplayOp::FillPath { bounds, .. }
            | DisplayOp::StrokePath { bounds, .. }
            | DisplayOp::NativeTextOp { bounds, .. }
            | DisplayOp::NativeImageXObject { bounds, .. }
            | DisplayOp::NativeShadingOp { bounds, .. }
            | DisplayOp::NativePatternPathOp { bounds, .. }
            | DisplayOp::NativeInlineImage { bounds, .. }
            | DisplayOp::NativeFormXObject { bounds, .. } => *bounds,
            DisplayOp::Save | DisplayOp::Restore | DisplayOp::StateOp { .. } => None,
        }
    }
}

/// Paint and geometry state needed to replay one operation.
#[derive(Debug, Clone)]
pub struct DrawState {
    pub ctm: Transform2D,
    pub fill_color: PixelColor,
    pub stroke_color: PixelColor,
    pub fill_color_explicit: bool,
    pub stroke_color_explicit: bool,
    pub fill_cmyk: Option<[f32; 4]>,
    pub stroke_cmyk: Option<[f32; 4]>,
    pub blend_mode: BlendMode,
    pub rendering_intent: String,
    pub stroke_overprint: bool,
    pub fill_overprint: bool,
    pub overprint_mode: i32,
    pub stroke_adjustment: bool,
    pub alpha_source: bool,
    pub text_knockout: bool,
    pub line_width: f64,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub miter_limit: f64,
    pub dash: DashState,
    pub flatness: f64,
}

/// Display-list feature counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayListStats {
    pub operations: usize,
    pub saves: usize,
    pub restores: usize,
    pub clips: usize,
    pub fills: usize,
    pub strokes: usize,
    pub paths: usize,
    pub path_segments: usize,
    pub text_ops: usize,
    pub image_xobjects: usize,
    pub inline_images: usize,
    pub form_xobjects: usize,
    pub shadings: usize,
    pub patterns: usize,
    pub transparency_ops: usize,
    pub requires_transparent_page_group: bool,
    pub optional_content_ops: usize,
    pub native_text_ops: usize,
    pub native_image_xobjects: usize,
    pub native_shading_ops: usize,
    pub native_pattern_path_ops: usize,
    pub native_inline_images: usize,
    pub native_form_xobjects: usize,
    pub unsupported_ops: usize,
    pub max_stack_depth: usize,
}

/// A drawing operation the current display-list subset cannot replay natively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedRenderOp {
    pub operator: String,
    pub reason: String,
}

/// Pixel-space page tile rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RenderTile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl RenderTile {
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    pub fn estimated_rgba_bytes(self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// Full-page pixel-space bounds for display-list culling.
///
/// Bounds are computed against the display list's full-page viewport, not a
/// tile-local viewport. Tile and band replay can therefore skip vector ops whose
/// retained bounds do not intersect the current viewport window, avoiding the
/// previous "execute every vector op for every tile" cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderBounds {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl RenderBounds {
    pub fn from_bbox(
        bbox: [f64; 4],
        ctm: &Transform2D,
        viewport: &Viewport,
        padding_px: f64,
    ) -> Option<Self> {
        let points = [
            ctm.transform_point(bbox[0], bbox[1]),
            ctm.transform_point(bbox[2], bbox[1]),
            ctm.transform_point(bbox[0], bbox[3]),
            ctm.transform_point(bbox[2], bbox[3]),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (x, y) in points {
            let (px, py) = viewport.page_to_pixel_f64(x, y);
            if !px.is_finite() || !py.is_finite() {
                continue;
            }
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }
        if !min_x.is_finite() || !max_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let pad = padding_px.max(0.0);
        Some(Self {
            x0: floor_i32(min_x - pad),
            y0: floor_i32(min_y - pad),
            x1: ceil_i32(max_x + pad),
            y1: ceil_i32(max_y + pad),
        })
    }

    pub fn from_unit_square(
        ctm: &Transform2D,
        viewport: &Viewport,
        padding_px: f64,
    ) -> Option<Self> {
        let points = [
            ctm.transform_point(0.0, 0.0),
            ctm.transform_point(1.0, 0.0),
            ctm.transform_point(1.0, 1.0),
            ctm.transform_point(0.0, 1.0),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (x, y) in points {
            let (px, py) = viewport.page_to_pixel_f64(x, y);
            if !px.is_finite() || !py.is_finite() {
                continue;
            }
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }
        if !min_x.is_finite() || !max_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let pad = padding_px.max(0.0);
        Some(Self {
            x0: floor_i32(min_x - pad),
            y0: floor_i32(min_y - pad),
            x1: ceil_i32(max_x + pad),
            y1: ceil_i32(max_y + pad),
        })
    }

    pub fn from_path(
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        padding_px: f64,
    ) -> Option<Self> {
        let flat = flatten_path(path, ctm, viewport, 0.5);
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for subpath in &flat.subpaths {
            for &(x, y) in subpath {
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        if !min_x.is_finite() || !max_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let pad = padding_px.max(0.0);
        Some(Self {
            x0: floor_i32(min_x - pad),
            y0: floor_i32(min_y - pad),
            x1: ceil_i32(max_x + pad),
            y1: ceil_i32(max_y + pad),
        })
    }

    pub fn intersects_viewport(&self, viewport: &Viewport) -> bool {
        let vx0 = i32_from_u32(viewport.origin_x_px);
        let vy0 = i32_from_u32(viewport.origin_y_px);
        let vx1 = i32_from_u32(viewport.origin_x_px.saturating_add(viewport.width_px));
        let vy1 = i32_from_u32(viewport.origin_y_px.saturating_add(viewport.height_px));
        self.x1 > vx0 && self.x0 < vx1 && self.y1 > vy0 && self.y0 < vy1
    }

    pub fn intersects_tile(&self, tile: RenderTile) -> bool {
        let tx0 = i32_from_u32(tile.x);
        let ty0 = i32_from_u32(tile.y);
        let tx1 = i32_from_u32(tile.x.saturating_add(tile.width));
        let ty1 = i32_from_u32(tile.y.saturating_add(tile.height));
        self.x1 > tx0 && self.x0 < tx1 && self.y1 > ty0 && self.y0 < ty1
    }

    pub fn intersect(self, other: Self) -> Option<Self> {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        let x1 = self.x1.min(other.x1);
        let y1 = self.y1.min(other.y1);
        (x1 > x0 && y1 > y0).then_some(Self { x0, y0, x1, y1 })
    }

    pub fn from_text_run(
        start_tm: [f64; 6],
        end_tm: [f64; 6],
        ctm: &Transform2D,
        viewport: &Viewport,
        font_size: f64,
        rise: f64,
        padding_px: f64,
    ) -> Option<Self> {
        if font_size <= 0.0 || !font_size.is_finite() {
            return None;
        }
        let descent = -0.30 * font_size;
        let ascent = 1.20 * font_size;
        let text_points = [
            text_matrix_point(start_tm, 0.0, rise + descent),
            text_matrix_point(start_tm, 0.0, rise + ascent),
            text_matrix_point(end_tm, 0.0, rise + descent),
            text_matrix_point(end_tm, 0.0, rise + ascent),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (x, y) in text_points {
            let (ux, uy) = ctm.transform_point(x, y);
            let (px, py) = viewport.page_to_pixel_f64(ux, uy);
            if !px.is_finite() || !py.is_finite() {
                continue;
            }
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }
        if !min_x.is_finite() || !max_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let pad = padding_px.max(0.0);
        Some(Self {
            x0: floor_i32(min_x - pad),
            y0: floor_i32(min_y - pad),
            x1: ceil_i32(max_x + pad),
            y1: ceil_i32(max_y + pad),
        })
    }
}

fn text_matrix_point(tm: [f64; 6], x: f64, y: f64) -> (f64, f64) {
    (
        tm[0].mul_add(x, tm[2].mul_add(y, tm[4])),
        tm[1].mul_add(x, tm[3].mul_add(y, tm[5])),
    )
}

fn floor_i32(value: f64) -> i32 {
    if !value.is_finite() {
        0
    } else if value <= i32::MIN as f64 {
        i32::MIN
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        value.floor() as i32
    }
}

fn ceil_i32(value: f64) -> i32 {
    if !value.is_finite() {
        0
    } else if value <= i32::MIN as f64 {
        i32::MIN
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        value.ceil() as i32
    }
}

fn i32_from_u32(value: u32) -> i32 {
    if value > i32::MAX as u32 {
        i32::MAX
    } else {
        value as i32
    }
}

fn merge_bounds(a: Option<RenderBounds>, b: Option<RenderBounds>) -> Option<RenderBounds> {
    match (a, b) {
        (Some(a), Some(b)) => Some(RenderBounds {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        }),
        (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
        (None, None) => None,
    }
}

/// Stable key for the bounded render cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderCacheKey {
    pub page_number: usize,
    pub dpi: u32,
    pub render_mode: &'static str,
    pub tile: RenderTile,
    pub visibility_fingerprint: String,
    pub prepress_fingerprint: String,
    pub document_revision: String,
    pub contract_fingerprint: String,
}

impl RenderCacheKey {
    pub fn new(page_number: usize, dpi: u32, render_mode: RenderMode, tile: RenderTile) -> Self {
        Self::new_with_visibility(page_number, dpi, render_mode, tile, "ocg:none")
    }

    pub fn new_with_visibility(
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile: RenderTile,
        visibility_fingerprint: impl Into<String>,
    ) -> Self {
        Self::new_with_visibility_and_prepress(
            page_number,
            dpi,
            render_mode,
            tile,
            visibility_fingerprint,
            "prepress:none",
        )
    }

    pub fn new_with_visibility_and_prepress(
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile: RenderTile,
        visibility_fingerprint: impl Into<String>,
        prepress_fingerprint: impl Into<String>,
    ) -> Self {
        Self::new_with_full_identity(
            page_number,
            dpi,
            render_mode,
            tile,
            visibility_fingerprint,
            prepress_fingerprint,
            "revision:legacy",
            "contract:legacy",
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_full_identity(
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile: RenderTile,
        visibility_fingerprint: impl Into<String>,
        prepress_fingerprint: impl Into<String>,
        document_revision: impl Into<String>,
        contract_fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            page_number,
            dpi,
            render_mode: render_mode.as_str(),
            tile,
            visibility_fingerprint: visibility_fingerprint.into(),
            prepress_fingerprint: prepress_fingerprint.into(),
            document_revision: document_revision.into(),
            contract_fingerprint: contract_fingerprint.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct RenderCacheMetrics {
    pub hits: usize,
    pub misses: usize,
    pub inserts: usize,
    pub evictions: usize,
    pub skipped_oversized: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone)]
struct RenderCacheEntry {
    buffer: PixelBuffer,
    bytes: usize,
    last_used: u64,
}

/// Per-document render tile cache with byte accounting.
#[derive(Debug, Clone)]
pub struct RenderCache {
    budget_bytes: usize,
    max_entry_bytes: usize,
    bytes: usize,
    clock: u64,
    entries: HashMap<RenderCacheKey, RenderCacheEntry>,
    metrics: RenderCacheMetrics,
}

impl RenderCache {
    pub fn new(budget_bytes: usize, max_entry_bytes: usize) -> Self {
        Self {
            budget_bytes,
            max_entry_bytes,
            bytes: 0,
            clock: 0,
            entries: HashMap::new(),
            metrics: RenderCacheMetrics::default(),
        }
    }

    pub fn disabled() -> Self {
        Self::new(0, 0)
    }

    pub fn metrics(&self) -> RenderCacheMetrics {
        let mut metrics = self.metrics.clone();
        metrics.bytes = self.bytes;
        metrics
    }

    pub fn get(&mut self, key: &RenderCacheKey) -> Option<PixelBuffer> {
        self.clock = self.clock.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_used = self.clock;
            self.metrics.hits += 1;
            Some(entry.buffer.clone())
        } else {
            self.metrics.misses += 1;
            None
        }
    }

    pub fn get_ref(&mut self, key: &RenderCacheKey) -> Option<&PixelBuffer> {
        self.clock = self.clock.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_used = self.clock;
            self.metrics.hits += 1;
            Some(&entry.buffer)
        } else {
            self.metrics.misses += 1;
            None
        }
    }

    pub fn insert(&mut self, key: RenderCacheKey, buffer: PixelBuffer) {
        if self.budget_bytes == 0 || self.max_entry_bytes == 0 {
            self.metrics.skipped_oversized += 1;
            return;
        }
        let bytes = buffer.width as usize * buffer.height as usize * 4;
        if bytes > self.max_entry_bytes || bytes > self.budget_bytes {
            self.metrics.skipped_oversized += 1;
            return;
        }
        self.clock = self.clock.saturating_add(1);
        if let Some(old) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        while self.bytes + bytes > self.budget_bytes {
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(removed) = self.entries.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(removed.bytes);
                self.metrics.evictions += 1;
            }
        }
        self.entries.insert(
            key,
            RenderCacheEntry {
                buffer,
                bytes,
                last_used: self.clock,
            },
        );
        self.bytes += bytes;
        self.metrics.inserts += 1;
    }

    /// Evict every cached raster artifact belonging to one of `page_numbers`.
    /// Used by dependency-driven edit invalidation; unrelated page entries keep
    /// their recency and byte charge.
    pub fn invalidate_pages(&mut self, page_numbers: &[usize]) -> usize {
        let keys: Vec<_> = self
            .entries
            .keys()
            .filter(|key| page_numbers.contains(&key.page_number))
            .cloned()
            .collect();
        let mut removed_count = 0;
        for key in keys {
            if let Some(removed) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(removed.bytes);
                self.metrics.evictions = self.metrics.evictions.saturating_add(1);
                removed_count += 1;
            }
        }
        removed_count
    }

    /// Evict exact page/tile raster artifacts without discarding unrelated
    /// tiles from the same page. This is used when the dependency graph can
    /// prove a source edit affects only spatial cache entries.
    pub fn invalidate_tiles(&mut self, page_tiles: &[(usize, RenderTile)]) -> usize {
        if page_tiles.is_empty() {
            return 0;
        }
        let keys: Vec<_> = self
            .entries
            .keys()
            .filter(|key| {
                page_tiles
                    .iter()
                    .any(|(page, tile)| key.page_number == *page && key.tile == *tile)
            })
            .cloned()
            .collect();
        let mut removed_count = 0;
        for key in keys {
            if let Some(removed) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(removed.bytes);
                self.metrics.evictions = self.metrics.evictions.saturating_add(1);
                removed_count += 1;
            }
        }
        removed_count
    }
}

/// Concrete rendering target for display-list replay.
pub trait RenderDevice {
    fn save(&mut self);
    fn restore(&mut self);
    fn clip_path(&mut self, path: &Path, ctm: &Transform2D, rule: FillRule);
    fn fill_path(&mut self, path: &Path, state: &DrawState, rule: FillRule);
    fn stroke_path(&mut self, path: &Path, state: &DrawState);
    fn supports_native_high_level_ops(&self) -> bool {
        false
    }
    fn state_op(&mut self, state: &GraphicsStateDescriptor);
    fn native_text_op(&mut self, text: &RetainedTextOp);
    fn native_image_xobject(&mut self, name: &str);
    fn native_shading_op(&mut self, name: &str);
    fn native_pattern_path_op(&mut self, pattern: &PatternPathDescriptor);
    fn native_inline_image(&mut self, image: &RetainedInlineImage);
    fn native_form_xobject(&mut self, name: &str);
}

/// CPU raster device backed by the existing [`PixelBuffer`] rasterizer.
pub struct CpuRenderDevice {
    buf: PixelBuffer,
    viewport: Viewport,
    clip_stack: Vec<Arc<ClipNode>>,
    clip_dag: ClipDag,
    current_clip: Arc<ClipNode>,
    path_fill_mask_cache: CpuPathFillMaskCache,
    path_stroke_mask_cache: CpuPathStrokeMaskCache,
}

impl CpuRenderDevice {
    pub fn new(viewport: Viewport, render_mode: RenderMode) -> Self {
        let clip_dag = ClipDag::new();
        let current_clip = clip_dag.full();
        Self {
            buf: PixelBuffer::new_filled_with_mode(
                viewport.width_px,
                viewport.height_px,
                WHITE,
                render_mode,
            ),
            viewport,
            clip_stack: Vec::new(),
            clip_dag,
            current_clip,
            path_fill_mask_cache: CpuPathFillMaskCache::default(),
            path_stroke_mask_cache: CpuPathStrokeMaskCache::default(),
        }
    }

    pub fn into_buffer(self) -> PixelBuffer {
        self.buf
    }

    fn install_clip_node(&mut self, node: Arc<ClipNode>) {
        let mask = match &node.state {
            ClipState::Full => None,
            _ => Some(
                node.materialize(self.buf.width, self.buf.height)
                    .as_ref()
                    .clone(),
            ),
        };
        self.current_clip = node;
        self.buf.restore_clip(mask);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct CpuPathFillMaskCacheKey {
    path_hash: u64,
    fill_rule: u8,
    flatness: i64,
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    frac_e: i64,
    frac_f: i64,
}

struct CpuPathMaskCache<K> {
    entries: HashMap<K, (Arc<RasterizedGlyphMask>, u64, usize)>,
    order: BTreeMap<u64, K>,
    bytes: usize,
    next_seq: u64,
}

impl<K> Default for CpuPathMaskCache<K> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            bytes: 0,
            next_seq: 0,
        }
    }
}

impl<K> CpuPathMaskCache<K>
where
    K: Clone + Eq + Hash,
{
    const MAX_ENTRIES: usize = 4096;
    const MAX_BYTES: usize = 64 * 1024 * 1024;

    fn get(&mut self, key: &K) -> Option<Arc<RasterizedGlyphMask>> {
        let (mask, old_seq, _) = self.entries.get(key)?;
        let mask = Arc::clone(mask);
        let old_seq = *old_seq;
        let new_seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.order.remove(&old_seq);
        self.order.insert(new_seq, key.clone());
        if let Some((_, seq, _)) = self.entries.get_mut(key) {
            *seq = new_seq;
        }
        Some(mask)
    }

    fn insert(&mut self, key: K, mask: Arc<RasterizedGlyphMask>) {
        let bytes = mask.approximate_bytes();
        if bytes > Self::MAX_BYTES / 4 {
            return;
        }

        if let Some((_, old_seq, old_bytes)) = self.entries.remove(&key) {
            self.order.remove(&old_seq);
            self.bytes = self.bytes.saturating_sub(old_bytes);
        }

        while self.entries.len() >= Self::MAX_ENTRIES
            || self.bytes.saturating_add(bytes) > Self::MAX_BYTES
        {
            if !self.evict_one() {
                break;
            }
        }

        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.order.insert(seq, key.clone());
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(key, (mask, seq, bytes));
    }

    fn evict_one(&mut self) -> bool {
        let Some((&lru_seq, _)) = self.order.iter().next() else {
            return false;
        };
        let Some(lru_key) = self.order.remove(&lru_seq) else {
            return false;
        };
        if let Some((_, _, bytes)) = self.entries.remove(&lru_key) {
            self.bytes = self.bytes.saturating_sub(bytes);
        }
        true
    }
}

type CpuPathFillMaskCache = CpuPathMaskCache<CpuPathFillMaskCacheKey>;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct CpuPathStrokeMaskCacheKey {
    path_hash: u64,
    width: i64,
    flatness: i64,
    cap: u8,
    join: u8,
    miter_limit: i64,
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    frac_e: i64,
    frac_f: i64,
}

type CpuPathStrokeMaskCache = CpuPathMaskCache<CpuPathStrokeMaskCacheKey>;

struct CpuPathFillMaskRequest<'a> {
    viewport: &'a Viewport,
    path: &'a Path,
    ctm: &'a Transform2D,
    rule: FillRule,
    color: PixelColor,
    flatness: f64,
}

fn cpu_paint_cached_path_fill(
    cache: &mut CpuPathFillMaskCache,
    buf: &mut PixelBuffer,
    request: CpuPathFillMaskRequest<'_>,
) -> bool {
    let CpuPathFillMaskRequest {
        viewport,
        path,
        ctm,
        rule,
        color,
        flatness,
    } = request;
    if path.segments.is_empty() || path.segments.len() > 16_384 {
        return false;
    }
    if let Some((x, y, width, height)) = axis_aligned_integer_rect(path, ctm, viewport) {
        buf.fill_rect(x, y, width, height, color);
        return true;
    }
    let device_t = ctm.concat(&viewport.to_transform());
    if !cpu_path_cache_transform_allowed(&device_t) {
        return false;
    }
    let origin_x = device_t.e.floor();
    let origin_y = device_t.f.floor();
    let normalized_t = Transform2D {
        e: device_t.e - origin_x,
        f: device_t.f - origin_y,
        ..device_t
    };
    let key = CpuPathFillMaskCacheKey {
        path_hash: cpu_hash_path_for_mask_cache(path),
        fill_rule: match rule {
            FillRule::NonZero => 0,
            FillRule::EvenOdd => 1,
        },
        flatness: cpu_quantize_mask_value(flatness),
        a: cpu_quantize_mask_value(normalized_t.a),
        b: cpu_quantize_mask_value(normalized_t.b),
        c: cpu_quantize_mask_value(normalized_t.c),
        d: cpu_quantize_mask_value(normalized_t.d),
        frac_e: cpu_quantize_mask_fraction(normalized_t.e),
        frac_f: cpu_quantize_mask_fraction(normalized_t.f),
    };
    let dx = cpu_floor_to_i32(origin_x);
    let dy = cpu_floor_to_i32(origin_y);
    if let Some(mask) = cache.get(&key) {
        mask.paint(buf, dx, dy, color);
        return true;
    }
    let Some(mask) = rasterize_path_alpha_mask(path, &normalized_t, rule, flatness) else {
        return false;
    };
    let mask = Arc::new(mask);
    mask.paint(buf, dx, dy, color);
    cache.insert(key, mask);
    true
}

#[allow(clippy::too_many_arguments)]
fn cpu_paint_cached_path_stroke(
    cache: &mut CpuPathStrokeMaskCache,
    buf: &mut PixelBuffer,
    viewport: &Viewport,
    path: &Path,
    ctm: &Transform2D,
    color: PixelColor,
    stroke_width: f64,
    dash: &DashState,
    cap: &LineCap,
    join: &LineJoin,
    miter_limit: f64,
    flatness: f64,
) -> bool {
    if path.segments.is_empty()
        || path.segments.len() > 16_384
        || !dash.is_solid()
        || stroke_width <= 0.0
        || !stroke_width.is_finite()
    {
        return false;
    }
    let device_t = ctm.concat(&viewport.to_transform());
    if !cpu_path_cache_transform_allowed(&device_t) {
        return false;
    }
    let origin_x = device_t.e.floor();
    let origin_y = device_t.f.floor();
    let normalized_t = Transform2D {
        e: device_t.e - origin_x,
        f: device_t.f - origin_y,
        ..device_t
    };
    let key = CpuPathStrokeMaskCacheKey {
        path_hash: cpu_hash_path_for_mask_cache(path),
        width: cpu_quantize_mask_value(stroke_width * device_t.scale_factor()),
        flatness: cpu_quantize_mask_value(flatness),
        cap: cpu_line_cap_id(cap),
        join: cpu_line_join_id(join),
        miter_limit: cpu_quantize_mask_value(miter_limit),
        a: cpu_quantize_mask_value(normalized_t.a),
        b: cpu_quantize_mask_value(normalized_t.b),
        c: cpu_quantize_mask_value(normalized_t.c),
        d: cpu_quantize_mask_value(normalized_t.d),
        frac_e: cpu_quantize_mask_fraction(normalized_t.e),
        frac_f: cpu_quantize_mask_fraction(normalized_t.f),
    };
    let dx = cpu_floor_to_i32(origin_x);
    let dy = cpu_floor_to_i32(origin_y);
    if let Some(mask) = cache.get(&key) {
        mask.paint(buf, dx, dy, color);
        return true;
    }
    let flat = flatten_path_device_transform(path, &normalized_t, flatness);
    let outline = stroke_flat_path(
        &flat,
        (stroke_width * normalized_t.scale_factor()).max(1.0),
        dash,
        cap.clone(),
        join.clone(),
        miter_limit,
    );
    if outline.subpaths.is_empty() {
        return true;
    }
    let Some(mask) = rasterize_flat_alpha_mask(&outline, FillRule::NonZero) else {
        return false;
    };
    let mask = Arc::new(mask);
    mask.paint(buf, dx, dy, color);
    cache.insert(key, mask);
    true
}

fn cpu_path_cache_transform_allowed(device_t: &Transform2D) -> bool {
    [
        device_t.a, device_t.b, device_t.c, device_t.d, device_t.e, device_t.f,
    ]
    .iter()
    .all(|value| value.is_finite())
        && device_t.scale_factor() > 0.0
        && device_t.scale_factor() <= 256.0
}

fn cpu_floor_to_i32(value: f64) -> i32 {
    if value <= i32::MIN as f64 {
        i32::MIN
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn cpu_line_cap_id(cap: &LineCap) -> u8 {
    match cap {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::ProjectingSquare => 2,
    }
}

fn cpu_line_join_id(join: &LineJoin) -> u8 {
    match join {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
    }
}

fn cpu_hash_path_for_mask_cache(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.segments.len().hash(&mut hasher);
    for segment in &path.segments {
        match segment {
            crate::render::path::PathSegment::MoveTo(x, y) => {
                0u8.hash(&mut hasher);
                x.to_bits().hash(&mut hasher);
                y.to_bits().hash(&mut hasher);
            }
            crate::render::path::PathSegment::LineTo(x, y) => {
                1u8.hash(&mut hasher);
                x.to_bits().hash(&mut hasher);
                y.to_bits().hash(&mut hasher);
            }
            crate::render::path::PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                2u8.hash(&mut hasher);
                cp1x.to_bits().hash(&mut hasher);
                cp1y.to_bits().hash(&mut hasher);
                cp2x.to_bits().hash(&mut hasher);
                cp2y.to_bits().hash(&mut hasher);
                x.to_bits().hash(&mut hasher);
                y.to_bits().hash(&mut hasher);
            }
            crate::render::path::PathSegment::ClosePath => {
                3u8.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

fn cpu_quantize_mask_value(value: f64) -> i64 {
    const SCALE: f64 = 64.0;
    if !value.is_finite() {
        0
    } else if value <= i64::MIN as f64 / SCALE {
        i64::MIN
    } else if value >= i64::MAX as f64 / SCALE {
        i64::MAX
    } else {
        (value * SCALE).round() as i64
    }
}

fn cpu_quantize_mask_fraction(value: f64) -> i64 {
    const SCALE: f64 = 2.0;
    if !value.is_finite() {
        0
    } else {
        (value.fract() * SCALE).round() as i64
    }
}

impl RenderDevice for CpuRenderDevice {
    fn save(&mut self) {
        self.clip_stack.push(Arc::clone(&self.current_clip));
    }

    fn restore(&mut self) {
        if let Some(saved) = self.clip_stack.pop() {
            self.install_clip_node(saved);
        } else {
            log::warn!("DisplayList CpuRenderDevice: restore with empty clip stack");
        }
    }

    fn clip_path(&mut self, path: &Path, ctm: &Transform2D, rule: FillRule) {
        let clip_node = if let Some((x, y, width, height)) =
            axis_aligned_integer_rect(path, ctm, &self.viewport)
        {
            self.clip_dag
                .rectangle(x, y, width, height, self.buf.width, self.buf.height)
        } else {
            let flat = flatten_path(path, ctm, &self.viewport, 0.5);
            let clip = ClipMask::from_path(&flat, self.buf.width, self.buf.height, rule);
            self.clip_dag.intern_mask(&clip)
        };
        let current = Arc::clone(&self.current_clip);
        let next = self.clip_dag.intersect(&current, &clip_node);
        self.install_clip_node(next);
    }

    fn fill_path(&mut self, path: &Path, state: &DrawState, rule: FillRule) {
        let saved_blend = self.buf.blend_mode;
        self.buf.blend_mode = state.blend_mode;
        if state.fill_overprint {
            if let Some(cmyk) = state.fill_cmyk {
                PathPainter::fill_device_cmyk_overprint_preview_with_flatness(
                    &mut self.buf,
                    path,
                    &state.ctm,
                    &self.viewport,
                    cmyk,
                    state.fill_color[3] as f32 / 255.0,
                    state.overprint_mode,
                    rule,
                    state.flatness,
                    false,
                );
                self.buf.blend_mode = saved_blend;
                return;
            }
        }
        if !cpu_paint_cached_path_fill(
            &mut self.path_fill_mask_cache,
            &mut self.buf,
            CpuPathFillMaskRequest {
                viewport: &self.viewport,
                path,
                ctm: &state.ctm,
                rule,
                color: state.fill_color,
                flatness: state.flatness,
            },
        ) {
            // General-path fallback is retained only for transforms/path
            // shapes outside the bounded replay-mask cache contract. Route it
            // through the same scanline-capable fast path used by the canonical
            // page renderer so CpuRenderDevice does not regress to the old
            // accumulator-heavy paint for large retained-list paths.
            let cancel = CancelToken::new();
            let _ = PathPainter::fill_fast_cancellable_with_flatness(
                &mut self.buf,
                path,
                &state.ctm,
                &self.viewport,
                state.fill_color,
                rule,
                state.flatness,
                &cancel,
            );
        }
        self.buf.blend_mode = saved_blend;
    }

    fn stroke_path(&mut self, path: &Path, state: &DrawState) {
        let saved_blend = self.buf.blend_mode;
        self.buf.blend_mode = state.blend_mode;
        if !cpu_paint_cached_path_stroke(
            &mut self.path_stroke_mask_cache,
            &mut self.buf,
            &self.viewport,
            path,
            &state.ctm,
            state.stroke_color,
            state.line_width,
            &state.dash,
            &state.line_cap,
            &state.line_join,
            state.miter_limit,
            state.flatness,
        ) {
            // General-path fallback is retained only for dash/transform/path
            // shapes outside the bounded replay-mask cache contract. Use the
            // bounded scanline-capable fast path instead of the legacy
            // accumulator-heavy stroke replay.
            let cancel = CancelToken::new();
            let _ = PathPainter::stroke_with_style_fast_cancellable_with_flatness(
                &mut self.buf,
                path,
                &state.ctm,
                &self.viewport,
                state.stroke_color,
                state.line_width,
                &state.dash,
                &state.line_cap,
                &state.line_join,
                state.miter_limit,
                state.flatness,
                &cancel,
            );
        }
        self.buf.blend_mode = saved_blend;
    }

    fn state_op(&mut self, state: &GraphicsStateDescriptor) {
        log::trace!(
            "DisplayList CpuRenderDevice ignored state op '{:?}' because vector ops carry captured state",
            state
        );
    }

    fn native_text_op(&mut self, text: &RetainedTextOp) {
        log::warn!(
            "DisplayList CpuRenderDevice cannot replay native text op '{}' without page context",
            text.operator_name()
        );
    }

    fn native_image_xobject(&mut self, name: &str) {
        log::warn!(
            "DisplayList CpuRenderDevice cannot replay native image XObject '/{}' without page context",
            name
        );
    }

    fn native_shading_op(&mut self, name: &str) {
        log::warn!(
            "DisplayList CpuRenderDevice cannot replay native shading '/{}' without page context",
            name
        );
    }

    fn native_pattern_path_op(&mut self, pattern: &PatternPathDescriptor) {
        log::warn!(
            "DisplayList CpuRenderDevice cannot replay native pattern path ({} segments, {:?}) without page context",
            pattern.path.segments.len(),
            pattern.phase
        );
    }

    fn native_inline_image(&mut self, image: &RetainedInlineImage) {
        log::warn!(
            "DisplayList CpuRenderDevice cannot replay native inline image ({} params, {} bytes) without page context",
            image.params.len(),
            image.data.len()
        );
    }

    fn native_form_xobject(&mut self, name: &str) {
        log::warn!(
            "DisplayList CpuRenderDevice cannot replay native Form XObject '/{}' without page context",
            name
        );
    }
}

pub fn replay_display_list(list: &DisplayList, device: &mut dyn RenderDevice) -> Result<()> {
    if !list.is_fully_supported() {
        let reason = display_list_unsupported_replay_reason(list);
        return Err(WellfriendError::UnsupportedFeature(format!(
            "display-list replay refused unsupported list: {reason}"
        )));
    }
    if let Some(kind) = native_high_level_replay_kind(list) {
        if !device.supports_native_high_level_ops() {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "display-list replay device cannot render native {kind} without page context"
            )));
        }
    }

    for op in &list.ops {
        match op {
            DisplayOp::Save => device.save(),
            DisplayOp::Restore => device.restore(),
            DisplayOp::Clip {
                path, ctm, rule, ..
            } => device.clip_path(path, ctm, *rule),
            DisplayOp::FillPath {
                path, state, rule, ..
            } => device.fill_path(path, state, *rule),
            DisplayOp::StrokePath { path, state, .. } => device.stroke_path(path, state),
            DisplayOp::StateOp { state, .. } => device.state_op(state),
            DisplayOp::NativeTextOp { text, .. } => device.native_text_op(text),
            DisplayOp::NativeImageXObject { name, .. } => device.native_image_xobject(name),
            DisplayOp::NativeShadingOp { name, .. } => device.native_shading_op(name),
            DisplayOp::NativePatternPathOp { pattern, .. } => {
                device.native_pattern_path_op(pattern)
            }
            DisplayOp::NativeInlineImage { image, .. } => device.native_inline_image(image),
            DisplayOp::NativeFormXObject { name, .. } => device.native_form_xobject(name),
        }
    }
    Ok(())
}

fn display_list_unsupported_replay_reason(list: &DisplayList) -> String {
    list.unsupported
        .first()
        .map(|item| format!("{}: {}", item.operator, item.reason))
        .unwrap_or_else(|| "display list is marked unsupported".to_string())
}

fn native_high_level_replay_kind(list: &DisplayList) -> Option<&'static str> {
    list.ops.iter().find_map(|op| match op {
        DisplayOp::NativeTextOp { .. } => Some("text"),
        DisplayOp::NativeImageXObject { .. } => Some("image XObject"),
        DisplayOp::NativeShadingOp { .. } => Some("shading"),
        DisplayOp::NativePatternPathOp { .. } => Some("pattern path"),
        DisplayOp::NativeInlineImage { .. } => Some("inline image"),
        DisplayOp::NativeFormXObject { .. } => Some("Form XObject"),
        _ => None,
    })
}

pub fn render_display_list(list: &DisplayList, render_mode: RenderMode) -> Result<PixelBuffer> {
    if !list.is_fully_supported() {
        let reason = display_list_unsupported_replay_reason(list);
        return Err(WellfriendError::UnsupportedFeature(format!(
            "standalone display-list CPU replay refused unsupported list: {reason}"
        )));
    }
    if let Some(kind) = native_high_level_replay_kind(list) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "standalone display-list CPU replay cannot render native {kind} without page context; use PageRenderer display-list replay"
        )));
    }
    let mut device = CpuRenderDevice::new(list.viewport.clone(), render_mode);
    replay_display_list(list, &mut device)?;
    Ok(device.into_buffer())
}

/// Capture a vector-compatible display list from decoded content operations.
pub fn build_display_list(
    ops: &[ContentOperation],
    viewport: Viewport,
    resources: &PageResources,
) -> DisplayList {
    let stats = classify_content(ops, resources);
    let mut builder = DisplayListBuilder::new(viewport, resources);
    builder.stats = stats;
    builder.dispatch_all(ops);
    builder.finish()
}

fn estimate_named_resource_bytes(name: &str) -> usize {
    std::mem::size_of::<String>() + name.len()
}

fn estimate_retained_text_op_bytes(text: &RetainedTextOp) -> usize {
    let base = std::mem::size_of::<RetainedTextOp>();
    base + match text {
        RetainedTextOp::SetFont { name, .. } => name.len(),
        RetainedTextOp::Show(bytes)
        | RetainedTextOp::NextLineShow(bytes)
        | RetainedTextOp::SpacingNextLineShow { text: bytes, .. } => bytes.len(),
        RetainedTextOp::ShowArray(items) => items
            .iter()
            .map(|item| match item {
                RetainedTextArrayItem::Bytes(bytes) => bytes.len(),
                RetainedTextArrayItem::Adjustment(_) => std::mem::size_of::<f64>(),
            })
            .sum(),
        RetainedTextOp::Unsupported { operator, reason } => operator.len() + reason.len(),
        _ => 0,
    }
}

fn estimate_inline_image_bytes(params: &[Operand], data: &[u8]) -> usize {
    std::mem::size_of::<RetainedInlineImage>()
        + params.iter().map(estimate_operand_bytes).sum::<usize>()
        + data.len()
}

fn estimate_graphics_state_descriptor_bytes(state: &GraphicsStateDescriptor) -> usize {
    let base = std::mem::size_of::<GraphicsStateDescriptor>();
    use GraphicsStateDescriptor::*;
    base + match state {
        SetDash { array, .. } => array.len() * std::mem::size_of::<f64>(),
        SetRenderingIntent(name)
        | SetStrokeColorSpace { name, .. }
        | SetFillColorSpace { name, .. }
        | ApplyExtGState { name, .. }
        | SetFont { name, .. }
        | BeginMarkedContent(name)
        | MarkedContentPoint(name) => name.len(),
        SetStrokeColor {
            components, name, ..
        }
        | SetFillColor {
            components, name, ..
        } => components.len() * std::mem::size_of::<f64>() + name.as_ref().map_or(0, String::len),
        BeginMarkedContentWithProperties { tag, properties }
        | MarkedContentPointWithProperties { tag, properties } => {
            tag.len() + estimate_marked_content_properties_bytes(properties)
        }
        Unsupported { operator } => operator.len(),
        _ => 0,
    }
}

fn estimate_marked_content_properties_bytes(
    properties: &crate::render::plan::MarkedContentProperties,
) -> usize {
    match properties {
        crate::render::plan::MarkedContentProperties::Name { name, object } => {
            name.len() + object.as_ref().map_or(0, estimate_pdf_object_bytes)
        }
        crate::render::plan::MarkedContentProperties::Inline(operands) => {
            operands.iter().map(estimate_operand_bytes).sum()
        }
    }
}

fn estimate_pattern_path_bytes(pattern: &PatternPathDescriptor) -> usize {
    std::mem::size_of::<PatternPathDescriptor>()
        + std::mem::size_of_val(pattern.path.segments.as_slice())
}

fn pattern_phase_paints_fill(phase: &PatternPaintPhase) -> bool {
    matches!(
        phase,
        PatternPaintPhase::FillNonZero
            | PatternPaintPhase::FillEvenOdd
            | PatternPaintPhase::FillStrokeNonZero
            | PatternPaintPhase::FillStrokeEvenOdd
            | PatternPaintPhase::CloseFillStrokeNonZero
            | PatternPaintPhase::CloseFillStrokeEvenOdd
    )
}

fn pattern_phase_paints_stroke(phase: &PatternPaintPhase) -> bool {
    matches!(
        phase,
        PatternPaintPhase::Stroke
            | PatternPaintPhase::CloseStroke
            | PatternPaintPhase::FillStrokeNonZero
            | PatternPaintPhase::FillStrokeEvenOdd
            | PatternPaintPhase::CloseFillStrokeNonZero
            | PatternPaintPhase::CloseFillStrokeEvenOdd
    )
}

fn color_space_object_is_pattern(object: &PdfObject) -> bool {
    match object {
        PdfObject::Name(name) => name == "Pattern",
        PdfObject::Array(items) => items
            .first()
            .and_then(PdfObject::as_name)
            .is_some_and(|name| name == "Pattern"),
        _ => false,
    }
}

fn estimate_pdf_object_bytes(object: &PdfObject) -> usize {
    match object {
        PdfObject::Null | PdfObject::Boolean(_) | PdfObject::Integer(_) | PdfObject::Real(_) => {
            std::mem::size_of::<PdfObject>()
        }
        PdfObject::String(bytes) => std::mem::size_of::<PdfObject>() + bytes.len(),
        PdfObject::Stream { dict, raw } => {
            std::mem::size_of::<PdfObject>() + estimate_pdf_dictionary_bytes(dict) + raw.len()
        }
        PdfObject::Name(name) => std::mem::size_of::<PdfObject>() + name.len(),
        PdfObject::Array(items) => {
            std::mem::size_of::<PdfObject>()
                + items.iter().map(estimate_pdf_object_bytes).sum::<usize>()
        }
        PdfObject::Dictionary(dict) => {
            std::mem::size_of::<PdfObject>() + estimate_pdf_dictionary_bytes(dict)
        }
        PdfObject::Reference { .. } => std::mem::size_of::<PdfObject>(),
    }
}

fn estimate_pdf_dictionary_bytes(dict: &crate::object::PdfDictionary) -> usize {
    dict.entries()
        .map(|(key, value)| key.len() + estimate_pdf_object_bytes(value))
        .sum()
}

fn estimate_operand_bytes(operand: &crate::content::operation::Operand) -> usize {
    use crate::content::operation::Operand;
    match operand {
        Operand::Integer(_) | Operand::Real(_) | Operand::Boolean(_) => {
            std::mem::size_of_val(operand)
        }
        Operand::Name(name) => name.len(),
        Operand::String(bytes) => bytes.len(),
        Operand::Array(items) => items.iter().map(estimate_operand_bytes).sum(),
        Operand::Dictionary(entries) => entries
            .iter()
            .map(|(key, value)| key.len().saturating_add(estimate_operand_bytes(value)))
            .sum(),
    }
}

fn classify_content(ops: &[ContentOperation], resources: &PageResources) -> DisplayListStats {
    let mut stats = DisplayListStats::default();
    let mut gs = GraphicsState::default();
    let mut pending_inline = false;
    for op in ops {
        match op.operator.as_str() {
            "Tj" | "TJ" | "'" | "\"" => stats.text_ops += 1,
            "BT" | "ET" | "Tf" | "Td" | "TD" | "Tm" | "T*" | "Tc" | "Tw" | "Tz" | "TL" | "Tr"
            | "Ts" => {}
            "Do" => match op
                .name(0)
                .and_then(|name| resources.xobject_subtypes.get(name))
                .map(String::as_str)
            {
                Some("Image") => stats.image_xobjects += 1,
                Some("Form") => {
                    stats.form_xobjects += 1;
                    if op.name(0).is_some_and(|name| {
                        resources
                            .xobject_stream_dicts
                            .get(name)
                            .is_some_and(xobject_dict_is_transparency_group)
                    }) {
                        stats.requires_transparent_page_group = true;
                    }
                }
                _ => stats.image_xobjects += 1,
            },
            "sh" => stats.shadings += 1,
            "BDC" | "DP" if marked_content_uses_optional_content(op, resources) => {
                stats.optional_content_ops += 1
            }
            "ID" => pending_inline = true,
            "inline_image_data" if pending_inline => {
                stats.inline_images += 1;
                pending_inline = false;
            }
            "gs" => {
                if let Some(name) = op.name(0) {
                    if let Some(dict) = resources.ext_g_states.get(name) {
                        if dict.get("SMask").is_some()
                            || dict.get("ca").is_some()
                            || dict.get("CA").is_some()
                            || dict.get("BM").is_some()
                        {
                            stats.transparency_ops += 1;
                        }
                        if ext_g_state_needs_transparent_page_group(dict) {
                            stats.requires_transparent_page_group = true;
                        }
                    } else {
                        stats.transparency_ops += 1;
                    }
                }
            }
            "scn" | "SCN"
                if op
                    .operands
                    .iter()
                    .any(|operand| operand.as_name().is_some()) =>
            {
                stats.patterns += 1;
            }
            _ => {}
        }
        gs.process(op);
        if gs.fill_pattern_name.is_some() || gs.stroke_pattern_name.is_some() {
            stats.patterns += 1;
        }
    }
    stats
}

fn ext_g_state_needs_transparent_page_group(dict: &PdfDictionary) -> bool {
    if ext_g_state_is_complete_no_paint(dict) {
        return false;
    }
    ["ca", "CA"].iter().any(|key| {
        dict.get(key)
            .and_then(PdfObject::as_number)
            .is_some_and(|alpha| alpha < 0.999)
    }) || match dict.get("BM") {
        Some(PdfObject::Name(name)) => name != "Normal" && name != "Compatible",
        Some(PdfObject::Array(items)) => items
            .iter()
            .filter_map(PdfObject::as_name)
            .any(|name| name != "Normal" && name != "Compatible"),
        _ => false,
    } || match dict.get("SMask") {
        Some(PdfObject::Name(name)) if name == "None" => false,
        Some(_) => true,
        None => false,
    }
}

fn ext_g_state_is_complete_no_paint(dict: &PdfDictionary) -> bool {
    match (dict.get("CA"), dict.get("ca")) {
        (Some(stroke_alpha), Some(fill_alpha)) => {
            ext_g_state_alpha_is_fully_transparent(stroke_alpha)
                && ext_g_state_alpha_is_fully_transparent(fill_alpha)
        }
        _ => false,
    }
}

fn ext_g_state_alpha_is_fully_transparent(value: &PdfObject) -> bool {
    value
        .as_number()
        .is_some_and(|alpha| alpha.is_finite() && (0.0..=f64::EPSILON).contains(&alpha))
}

fn xobject_dict_is_transparency_group(dict: &PdfDictionary) -> bool {
    matches!(
        dict.get("Group"),
        Some(PdfObject::Dictionary(group)) if group.get_name("S") == Some("Transparency")
    )
}

fn marked_content_uses_optional_content(op: &ContentOperation, resources: &PageResources) -> bool {
    let Some(property) = op.operand(1) else {
        return false;
    };
    match property {
        Operand::Name(name) => resources
            .properties
            .get(name)
            .is_some_and(pdf_object_is_optional_content_property),
        Operand::Dictionary(entries) => operand_dictionary_is_optional_content_property(entries),
        _ => false,
    }
}

fn pdf_object_is_optional_content_property(object: &crate::object::PdfObject) -> bool {
    use crate::object::PdfObject;
    match object {
        PdfObject::Dictionary(dict) => {
            matches!(dict.get_name("Type"), Some("OCG" | "OCMD")) || dict.get("OC").is_some()
        }
        PdfObject::Reference { .. } => true,
        _ => false,
    }
}

fn operand_dictionary_is_optional_content_property(entries: &[(String, Operand)]) -> bool {
    entries.iter().any(|(key, value)| {
        key == "OC"
            || (key == "Type"
                && matches!(
                    value,
                    Operand::Name(name) if name == "OCG" || name == "OCMD"
                ))
    })
}

struct DisplayListBuilder<'a> {
    viewport: Viewport,
    resources: &'a PageResources,
    gs: GraphicsState,
    path: Path,
    path_op_count: usize,
    pending_clip: Option<FillRule>,
    current_clip_bounds: Option<RenderBounds>,
    clip_bounds_stack: Vec<Option<RenderBounds>>,
    color_explicit_stack: Vec<(bool, bool)>,
    soft_mask_stack: Vec<bool>,
    active_soft_mask: bool,
    fill_color_explicit: bool,
    stroke_color_explicit: bool,
    ops: Vec<DisplayOp>,
    unsupported: Vec<UnsupportedRenderOp>,
    stats: DisplayListStats,
    inline_begin_pending: bool,
    pending_inline_params: Option<Vec<Operand>>,
    inline_data_pending_end: bool,
    text_object_active: bool,
    marked_content_depth: usize,
    compatibility_section_depth: usize,
}

impl<'a> DisplayListBuilder<'a> {
    fn new(viewport: Viewport, resources: &'a PageResources) -> Self {
        Self {
            viewport,
            resources,
            gs: GraphicsState::default(),
            path: Path::new(),
            path_op_count: 0,
            pending_clip: None,
            current_clip_bounds: None,
            clip_bounds_stack: Vec::new(),
            color_explicit_stack: Vec::new(),
            soft_mask_stack: Vec::new(),
            active_soft_mask: false,
            fill_color_explicit: false,
            stroke_color_explicit: false,
            ops: Vec::new(),
            unsupported: Vec::new(),
            stats: DisplayListStats::default(),
            inline_begin_pending: false,
            pending_inline_params: None,
            inline_data_pending_end: false,
            text_object_active: false,
            marked_content_depth: 0,
            compatibility_section_depth: 0,
        }
    }

    fn finish(mut self) -> DisplayList {
        if self.marked_content_depth > 0 {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "BMC".to_string(),
                reason: format!(
                    "malformed marked-content sequence: {} unterminated begin operator(s)",
                    self.marked_content_depth
                ),
            });
        }
        if self.compatibility_section_depth > 0 {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "BX".to_string(),
                reason: format!(
                    "malformed compatibility-section sequence: {} unterminated begin operator(s)",
                    self.compatibility_section_depth
                ),
            });
        }
        if self.inline_begin_pending {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "BI".to_string(),
                reason: "malformed inline image sequence: BI has no following ID".to_string(),
            });
        }
        if self.pending_inline_params.is_some() {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "ID".to_string(),
                reason: "malformed inline image sequence: ID has no following image data"
                    .to_string(),
            });
        }
        if self.inline_data_pending_end {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "inline_image_data".to_string(),
                reason: "malformed inline image sequence: image data has no following EI"
                    .to_string(),
            });
        }
        if self.text_object_active {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "BT".to_string(),
                reason: "malformed text-object sequence: BT has no closing ET".to_string(),
            });
        }
        if self.pending_clip.take().is_some() {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "W".to_string(),
                reason:
                    "malformed path clipping sequence: W/W* has no following path painting operator"
                        .to_string(),
            });
        }
        self.stats.operations = self.ops.len();
        self.stats.unsupported_ops = self.unsupported.len();
        let supported = self.unsupported.is_empty();
        DisplayList {
            viewport: self.viewport,
            ops: self.ops,
            stats: self.stats,
            supported,
            unsupported: self.unsupported,
        }
    }

    fn dispatch_all(&mut self, ops: &[ContentOperation]) {
        for op in ops {
            if self.inline_data_pending_end && op.operator != "EI" {
                self.inline_data_pending_end = false;
                self.unsupported.push(UnsupportedRenderOp {
                    operator: "inline_image_data".to_string(),
                    reason: "malformed inline image sequence: image data has no following EI"
                        .to_string(),
                });
            }
            if self.pending_inline_params.is_some() && op.operator != "inline_image_data" {
                self.pending_inline_params = None;
                self.unsupported.push(UnsupportedRenderOp {
                    operator: "ID".to_string(),
                    reason: "malformed inline image sequence: ID has no following image data"
                        .to_string(),
                });
            }
            if self.inline_begin_pending && op.operator != "ID" {
                self.inline_begin_pending = false;
                self.unsupported.push(UnsupportedRenderOp {
                    operator: "BI".to_string(),
                    reason: "malformed inline image sequence: BI has no following ID".to_string(),
                });
            }
            self.dispatch(op);
        }
    }

    fn note_path_op(&mut self) {
        self.path_op_count = self.path_op_count.saturating_add(1);
    }

    fn clear_path_ops(&mut self) {
        self.path_op_count = 0;
    }

    fn has_path_ops(&self) -> bool {
        self.path_op_count != 0
    }

    fn dispatch(&mut self, op: &ContentOperation) {
        match op.operator.as_str() {
            "m" => {
                if !self.validate_path_op(op) {
                    return;
                }
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    self.path.move_to(x, y);
                    self.note_path_op();
                }
            }
            "l" => {
                if !self.validate_path_op(op) {
                    return;
                }
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    self.path.line_to(x, y);
                    self.note_path_op();
                }
            }
            "c" => {
                if !self.validate_path_op(op) {
                    return;
                }
                if let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x3), Some(y3)) = (
                    op.number(0),
                    op.number(1),
                    op.number(2),
                    op.number(3),
                    op.number(4),
                    op.number(5),
                ) {
                    self.path.curve_to(x1, y1, x2, y2, x3, y3);
                    self.note_path_op();
                }
            }
            "v" => {
                if !self.validate_path_op(op) {
                    return;
                }
                if let (Some(x2), Some(y2), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    let (cx, cy) = self.path.current_point.expect("validated current point");
                    self.path.curve_to(cx, cy, x2, y2, x3, y3);
                    self.note_path_op();
                }
            }
            "y" => {
                if !self.validate_path_op(op) {
                    return;
                }
                if let (Some(x1), Some(y1), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.path.curve_to(x1, y1, x3, y3, x3, y3);
                    self.note_path_op();
                }
            }
            "h" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.path.close();
                self.note_path_op();
            }
            "re" => {
                if !self.validate_path_op(op) {
                    return;
                }
                if let (Some(x), Some(y), Some(w), Some(h)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.path.rect(x, y, w, h);
                    self.note_path_op();
                }
            }
            "S" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.stroke_and_clear(op)
            }
            "s" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.path.close();
                self.note_path_op();
                self.stroke_and_clear(op);
            }
            "f" | "F" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.fill_and_clear(op, FillRule::NonZero)
            }
            "f*" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.fill_and_clear(op, FillRule::EvenOdd)
            }
            "B" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.fill_stroke_and_clear(op, FillRule::NonZero)
            }
            "B*" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.fill_stroke_and_clear(op, FillRule::EvenOdd)
            }
            "b" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.path.close();
                self.note_path_op();
                self.fill_stroke_and_clear(op, FillRule::NonZero);
            }
            "b*" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.path.close();
                self.note_path_op();
                self.fill_stroke_and_clear(op, FillRule::EvenOdd);
            }
            "n" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.apply_pending_clip();
                self.path.clear();
                self.clear_path_ops();
            }
            "W" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.set_pending_clip(op, FillRule::NonZero);
            }
            "W*" => {
                if !self.validate_path_op(op) {
                    return;
                }
                self.set_pending_clip(op, FillRule::EvenOdd);
            }
            "q" => {
                if !self.validate_graphics_state_op(op) {
                    return;
                }
                self.ops.push(DisplayOp::Save);
                self.stats.saves += 1;
                self.clip_bounds_stack.push(self.current_clip_bounds);
                self.color_explicit_stack
                    .push((self.fill_color_explicit, self.stroke_color_explicit));
                self.soft_mask_stack.push(self.active_soft_mask);
                self.gs.process(op);
                self.stats.max_stack_depth = self.stats.max_stack_depth.max(self.gs.stack_depth());
            }
            "Q" => {
                if !self.validate_graphics_state_op(op) {
                    return;
                }
                if self.gs.stack_depth() == 0 {
                    self.reject_graphics_state_op(
                        op,
                        "malformed graphics-state operator 'Q': restore has no saved graphics state"
                            .to_string(),
                    );
                    return;
                }
                if self.clip_bounds_stack.is_empty()
                    || self.color_explicit_stack.is_empty()
                    || self.soft_mask_stack.is_empty()
                {
                    self.reject_graphics_state_op(
                        op,
                        "malformed graphics-state operator 'Q': restore side-stack state is unavailable"
                            .to_string(),
                    );
                    return;
                }
                self.gs.process(op);
                self.current_clip_bounds = self
                    .clip_bounds_stack
                    .pop()
                    .expect("side-stack guard ensures clip bounds state");
                let (fill, stroke) = self
                    .color_explicit_stack
                    .pop()
                    .expect("side-stack guard ensures color explicit state");
                self.fill_color_explicit = fill;
                self.stroke_color_explicit = stroke;
                self.active_soft_mask = self
                    .soft_mask_stack
                    .pop()
                    .expect("side-stack guard ensures soft-mask state");
                self.ops.push(DisplayOp::Restore);
                self.stats.restores += 1;
            }
            "cm" | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "G" | "g" | "RG" | "rg" | "K"
            | "k" | "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" => {
                if !self.validate_graphics_state_op(op) {
                    return;
                }
                self.gs.process(op);
                self.note_color_explicitness(op.operator.as_str());
                self.push_state_op(op);
            }
            "gs" => {
                if !self.validate_graphics_state_op(op) {
                    return;
                }
                self.apply_ext_g_state(op);
                self.push_state_op(op);
            }
            "BMC" | "BDC" | "EMC" | "MP" | "DP" | "BX" | "EX" => {
                if !self.validate_marked_content_op(op) {
                    return;
                }
                self.push_state_op(op);
            }
            "BT" | "ET" | "Tf" | "Td" | "TD" | "Tm" | "T*" | "Tc" | "Tw" | "Tz" | "TL" | "Tr"
            | "Ts" | "Tj" | "TJ" | "'" | "\"" => {
                if !self.validate_text_op(op) {
                    return;
                }
                if !self.validate_text_object_sequence(op) {
                    return;
                }
                self.push_native_text(op);
                self.gs.process(op);
            }
            "d0" | "d1" => {
                if !self.validate_type3_glyph_metric_op(op) {
                    return;
                }
                self.push_state_op(op);
            }
            "Do" => {
                if !self.validate_resource_invocation_op(op) {
                    return;
                }
                self.push_native_xobject(op);
            }
            "sh" => {
                if !self.validate_resource_invocation_op(op) {
                    return;
                }
                self.push_native_shading(op);
            }
            "BI" => {
                self.inline_begin_pending = true;
            }
            "ID" => {
                if !self.inline_begin_pending {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: "ID".to_string(),
                        reason: "malformed inline image sequence: ID has no preceding BI"
                            .to_string(),
                    });
                    return;
                }
                self.inline_begin_pending = false;
                self.pending_inline_params = Some(op.operands.clone());
            }
            "inline_image_data" => self.push_native_inline_image(op),
            "EI" => {
                if self.inline_data_pending_end {
                    self.inline_data_pending_end = false;
                } else {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: "EI".to_string(),
                        reason: "malformed inline image sequence: EI has no preceding image data"
                            .to_string(),
                    });
                }
            }
            _ => {
                self.gs.process(op);
                // Unknown or extension operators are replayed through the same
                // state-dispatch path used by immediate rendering. Unsupported
                // operators remain exact no-ops or graphics-state updates
                // according to the canonical dispatcher.
                self.push_state_op(op);
            }
        }
    }

    fn validate_graphics_state_op(&mut self, op: &ContentOperation) -> bool {
        match graphics_state_operand_refusal(op).or_else(|| {
            let (space, usage) = match op.operator.as_str() {
                "SC" | "SCN" => (&self.gs.stroke_color_space, "stroking"),
                "sc" | "scn" => (&self.gs.fill_color_space, "nonstroking"),
                _ => return None,
            };
            graphics_state_color_component_arity_refusal(op, space, usage)
        }) {
            Some(reason) => {
                self.reject_graphics_state_op(op, reason);
                false
            }
            None => true,
        }
    }

    fn reject_graphics_state_op(&mut self, op: &ContentOperation, reason: String) {
        self.unsupported.push(UnsupportedRenderOp {
            operator: op.operator.clone(),
            reason,
        });
    }

    fn validate_path_op(&mut self, op: &ContentOperation) -> bool {
        match path_operand_refusal(op, self.path.current_point.is_some()) {
            Some(reason) => {
                self.unsupported.push(UnsupportedRenderOp {
                    operator: op.operator.clone(),
                    reason,
                });
                false
            }
            None => true,
        }
    }

    fn set_pending_clip(&mut self, op: &ContentOperation, rule: FillRule) -> bool {
        if self.pending_clip.is_some() {
            self.unsupported.push(UnsupportedRenderOp {
                operator: op.operator.clone(),
                reason:
                    "malformed path clipping sequence: W/W* was not terminated before another clipping operator"
                        .to_string(),
            });
            false
        } else {
            self.pending_clip = Some(rule);
            true
        }
    }

    fn validate_text_op(&mut self, op: &ContentOperation) -> bool {
        match text_operand_refusal(op) {
            Some(reason) => {
                self.unsupported.push(UnsupportedRenderOp {
                    operator: op.operator.clone(),
                    reason,
                });
                false
            }
            None => true,
        }
    }

    fn validate_text_object_sequence(&mut self, op: &ContentOperation) -> bool {
        match op.operator.as_str() {
            "BT" => {
                if self.text_object_active {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: op.operator.clone(),
                        reason:
                            "malformed text-object sequence: nested BT inside active text object"
                                .to_string(),
                    });
                    return false;
                }
                self.text_object_active = true;
                true
            }
            "ET" => {
                if !self.text_object_active {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: op.operator.clone(),
                        reason: "malformed text-object sequence: ET has no active text object"
                            .to_string(),
                    });
                    return false;
                }
                self.text_object_active = false;
                true
            }
            "Td" | "TD" | "Tm" | "T*" | "Tj" | "TJ" | "'" | "\"" => {
                if !self.text_object_active {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: op.operator.clone(),
                        reason: format!(
                            "malformed text-object sequence: operator '{}' requires active text object",
                            op.operator
                        ),
                    });
                    return false;
                }
                true
            }
            _ => true,
        }
    }

    fn validate_marked_content_op(&mut self, op: &ContentOperation) -> bool {
        if let Some(reason) = marked_content_operand_refusal(op) {
            self.unsupported.push(UnsupportedRenderOp {
                operator: op.operator.clone(),
                reason,
            });
            return false;
        }

        match op.operator.as_str() {
            "BMC" | "BDC" => {
                self.marked_content_depth = self.marked_content_depth.saturating_add(1);
            }
            "EMC" => {
                if self.marked_content_depth == 0 {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: op.operator.clone(),
                        reason:
                            "malformed marked-content operator 'EMC': end has no active marked-content sequence"
                                .to_string(),
                    });
                    return false;
                }
                self.marked_content_depth -= 1;
            }
            "BX" => {
                self.compatibility_section_depth =
                    self.compatibility_section_depth.saturating_add(1);
            }
            "EX" => {
                if self.compatibility_section_depth == 0 {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: op.operator.clone(),
                        reason:
                            "malformed compatibility-section operator 'EX': end has no active compatibility section"
                                .to_string(),
                    });
                    return false;
                }
                self.compatibility_section_depth -= 1;
            }
            _ => {}
        }
        true
    }

    fn validate_resource_invocation_op(&mut self, op: &ContentOperation) -> bool {
        match resource_invocation_operand_refusal(op) {
            Some(reason) => {
                self.unsupported.push(UnsupportedRenderOp {
                    operator: op.operator.clone(),
                    reason,
                });
                false
            }
            None => true,
        }
    }

    fn validate_type3_glyph_metric_op(&mut self, op: &ContentOperation) -> bool {
        match type3_glyph_metric_operand_refusal(op) {
            Some(reason) => {
                self.unsupported.push(UnsupportedRenderOp {
                    operator: op.operator.clone(),
                    reason,
                });
                false
            }
            None => true,
        }
    }

    fn apply_ext_g_state(&mut self, op: &ContentOperation) {
        let Some(name) = op.name(0) else {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "gs".to_string(),
                reason: "ExtGState operator is missing its resource name".to_string(),
            });
            return;
        };
        let Some(dict) = self.resources.ext_g_states.get(name) else {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "gs".to_string(),
                reason: format!("ExtGState resource /{name} is missing"),
            });
            return;
        };
        let label = format!("ExtGState /{name}");
        if let Err(reason) = self.gs.try_apply_ext_g_state(dict, &label) {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "gs".to_string(),
                reason,
            });
            return;
        }
        // Soft masks are now represented by retaining the ExtGState operator
        // itself. Display-list replay goes through the same `RenderState`
        // dispatch path as immediate rendering, so `/SMask` Form groups,
        // transfer functions, backdrop colors, and the active clip/CTM stack are
        // applied by the canonical soft-mask implementation instead of forcing
        // the whole page back to immediate rendering.
        if let Some(smask) = dict.get("SMask") {
            self.active_soft_mask = !matches!(smask, PdfObject::Name(name) if name == "None");
        }
        if self.gs.blend_mode != BlendMode::Normal
            || self.gs.fill_alpha < 0.999
            || self.gs.stroke_alpha < 0.999
        {
            // These are represented and replayable through PixelBuffer blend
            // state, but the diagnostic counter still records the page as a
            // transparency-bearing display list for inspector users.
        }
    }

    fn note_color_explicitness(&mut self, operator: &str) {
        match operator {
            "g" | "rg" | "k" | "sc" | "scn" => self.fill_color_explicit = true,
            "G" | "RG" | "K" | "SC" | "SCN" => self.stroke_color_explicit = true,
            "cs" => self.fill_color_explicit = false,
            "CS" => self.stroke_color_explicit = false,
            _ => {}
        }
    }

    fn apply_pending_clip(&mut self) {
        if let Some(rule) = self.pending_clip.take() {
            let path = self.path.clone();
            let clip_bounds = RenderBounds::from_path(&path, &self.ctm(), &self.viewport, 0.0);
            self.current_clip_bounds = match (self.current_clip_bounds, clip_bounds) {
                (None, Some(bounds)) => Some(bounds),
                (Some(existing), Some(bounds)) => {
                    existing.intersect(bounds).or(Some(RenderBounds {
                        x0: 0,
                        y0: 0,
                        x1: 0,
                        y1: 0,
                    }))
                }
                (_, None) => Some(RenderBounds {
                    x0: 0,
                    y0: 0,
                    x1: 0,
                    y1: 0,
                }),
            };
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.clips += 1;
            self.ops.push(DisplayOp::Clip {
                path,
                ctm: self.ctm(),
                rule,
                bounds: clip_bounds,
            });
        }
    }

    fn stroke_and_clear(&mut self, paint_op: &ContentOperation) {
        self.apply_pending_clip();
        if self.active_soft_mask || self.uses_pattern_or_named_space() {
            self.push_stateful_path_run(paint_op);
            self.path.clear();
            self.clear_path_ops();
            return;
        }
        if !self.path.is_empty() {
            let path = self.path.clone();
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.strokes += 1;
            let state = self.draw_state();
            let bounds = self.path_bounds_for_stroke(&state);
            self.ops.push(DisplayOp::StrokePath {
                path,
                state,
                bounds,
            });
        }
        self.path.clear();
        self.clear_path_ops();
    }

    fn fill_and_clear(&mut self, paint_op: &ContentOperation, rule: FillRule) {
        self.apply_pending_clip();
        if self.active_soft_mask || self.uses_pattern_or_named_space() {
            self.push_stateful_path_run(paint_op);
            self.path.clear();
            self.clear_path_ops();
            return;
        }
        if !self.path.is_empty() {
            let path = self.path.clone();
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.fills += 1;
            self.ops.push(DisplayOp::FillPath {
                path,
                state: self.draw_state(),
                rule,
                bounds: self.path_bounds_for_fill(),
            });
        }
        self.path.clear();
        self.clear_path_ops();
    }

    fn fill_stroke_and_clear(&mut self, paint_op: &ContentOperation, rule: FillRule) {
        self.apply_pending_clip();
        if self.active_soft_mask || self.uses_pattern_or_named_space() {
            self.push_stateful_path_run(paint_op);
            self.path.clear();
            self.clear_path_ops();
            return;
        }
        if !self.path.is_empty() {
            let path = self.path.clone();
            let state = self.draw_state();
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.fills += 1;
            self.ops.push(DisplayOp::FillPath {
                path: path.clone(),
                state: state.clone(),
                rule,
                bounds: self.path_bounds_for_fill(),
            });
            self.stats.strokes += 1;
            let bounds = self.path_bounds_for_stroke(&state);
            self.ops.push(DisplayOp::StrokePath {
                path,
                state,
                bounds,
            });
        }
        self.path.clear();
        self.clear_path_ops();
    }

    fn push_stateful_path_run(&mut self, paint_op: &ContentOperation) {
        let Some(phase) = PatternPaintPhase::from_operator(paint_op.operator.as_str()) else {
            self.unsupported.push(UnsupportedRenderOp {
                operator: paint_op.operator.clone(),
                reason:
                    "stateful path paint operator cannot be represented as a typed pattern path"
                        .to_string(),
            });
            return;
        };
        if !self.validate_active_pattern_resources(&phase, paint_op.operator.as_str()) {
            return;
        }
        if self.path.is_empty() || !self.has_path_ops() {
            let pattern = PatternPathDescriptor {
                path: Path::new(),
                phase,
            };
            self.ops.push(DisplayOp::NativePatternPathOp {
                approx_bytes: estimate_pattern_path_bytes(&pattern),
                pattern,
                bounds: None,
            });
            return;
        }
        self.stats.path_segments += self.path.segments.len();
        self.stats.paths += 1;
        if matches!(paint_op.operator.as_str(), "S" | "s") {
            self.stats.strokes += 1;
        } else if matches!(paint_op.operator.as_str(), "B" | "B*" | "b" | "b*") {
            self.stats.fills += 1;
            self.stats.strokes += 1;
        } else {
            self.stats.fills += 1;
        }
        self.stats.native_pattern_path_ops += 1;
        let bounds = self.pattern_path_bounds(paint_op);
        let pattern = PatternPathDescriptor {
            path: self.path.clone(),
            phase,
        };
        self.ops.push(DisplayOp::NativePatternPathOp {
            approx_bytes: estimate_pattern_path_bytes(&pattern),
            pattern,
            bounds,
        });
    }

    fn validate_active_pattern_resources(
        &mut self,
        phase: &PatternPaintPhase,
        operator: &str,
    ) -> bool {
        if pattern_phase_paints_fill(phase) && self.fill_pattern_paint_active() {
            match self.gs.fill_pattern_name.as_deref() {
                Some(name) if self.resources.patterns.contains_key(name) => {}
                Some(name) => {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: operator.to_string(),
                        reason: format!("pattern fill resource /{name} is missing"),
                    });
                    return false;
                }
                None => {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: operator.to_string(),
                        reason: "pattern fill requires an active pattern name".to_string(),
                    });
                    return false;
                }
            }
        }
        if pattern_phase_paints_stroke(phase) && self.stroke_pattern_paint_active() {
            match self.gs.stroke_pattern_name.as_deref() {
                Some(name) if self.resources.patterns.contains_key(name) => {}
                Some(name) => {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: operator.to_string(),
                        reason: format!("pattern stroke resource /{name} is missing"),
                    });
                    return false;
                }
                None => {
                    self.unsupported.push(UnsupportedRenderOp {
                        operator: operator.to_string(),
                        reason: "pattern stroke requires an active pattern name".to_string(),
                    });
                    return false;
                }
            }
        }
        true
    }

    fn fill_pattern_paint_active(&self) -> bool {
        match &self.gs.fill_color_space {
            ColorSpace::Named(name) if name == "Pattern" => true,
            ColorSpace::Named(name) => self
                .resources
                .color_spaces
                .get(name)
                .is_some_and(color_space_object_is_pattern),
            _ => false,
        }
    }

    fn stroke_pattern_paint_active(&self) -> bool {
        match &self.gs.stroke_color_space {
            ColorSpace::Named(name) if name == "Pattern" => true,
            ColorSpace::Named(name) => self
                .resources
                .color_spaces
                .get(name)
                .is_some_and(color_space_object_is_pattern),
            _ => false,
        }
    }

    fn pattern_path_bounds(&self, paint_op: &ContentOperation) -> Option<RenderBounds> {
        match paint_op.operator.as_str() {
            "S" | "s" => self.path_bounds_for_stroke(&self.draw_state()),
            "B" | "B*" | "b" | "b*" => {
                let fill = self.path_bounds_for_fill();
                let stroke = self.path_bounds_for_stroke(&self.draw_state());
                merge_bounds(fill, stroke)
            }
            _ => self.path_bounds_for_fill(),
        }
    }

    fn path_bounds_for_fill(&self) -> Option<RenderBounds> {
        RenderBounds::from_path(&self.path, &self.ctm(), &self.viewport, 1.0)
    }

    fn path_bounds_for_stroke(&self, state: &DrawState) -> Option<RenderBounds> {
        let pad = state
            .line_width
            .abs()
            .max(1.0)
            .mul_add(state.ctm.scale_factor() * self.viewport.scale, 2.0);
        RenderBounds::from_path(&self.path, &state.ctm, &self.viewport, pad)
    }

    fn ctm(&self) -> Transform2D {
        Transform2D::from(self.gs.ctm)
    }

    fn draw_state(&self) -> DrawState {
        DrawState {
            ctm: self.ctm(),
            fill_color: resolve_simple_color(&self.gs.fill_color, self.gs.fill_alpha as f32),
            stroke_color: resolve_simple_color(&self.gs.stroke_color, self.gs.stroke_alpha as f32),
            fill_color_explicit: self.fill_color_explicit,
            stroke_color_explicit: self.stroke_color_explicit,
            fill_cmyk: simple_cmyk_components(&self.gs.fill_color),
            stroke_cmyk: simple_cmyk_components(&self.gs.stroke_color),
            blend_mode: self.gs.blend_mode,
            rendering_intent: self.gs.rendering_intent.clone(),
            stroke_overprint: self.gs.stroke_overprint,
            fill_overprint: self.gs.fill_overprint,
            overprint_mode: self.gs.overprint_mode,
            stroke_adjustment: self.gs.stroke_adjustment,
            alpha_source: self.gs.alpha_source,
            text_knockout: self.gs.text_knockout,
            line_width: self.gs.line_width,
            line_cap: self.gs.line_cap.clone(),
            line_join: self.gs.line_join.clone(),
            miter_limit: self.gs.miter_limit,
            dash: if self.gs.dash.pattern.is_empty() {
                DashState::solid()
            } else {
                DashState::new(self.gs.dash.pattern.clone(), self.gs.dash.phase)
            },
            flatness: self.gs.path_flatness_tolerance(),
        }
    }

    fn uses_pattern_or_named_space(&self) -> bool {
        matches!(self.gs.fill_color.space, ColorSpace::Named(_))
            || matches!(self.gs.stroke_color.space, ColorSpace::Named(_))
            || self.gs.fill_pattern_name.is_some()
            || self.gs.stroke_pattern_name.is_some()
    }

    fn push_native_text(&mut self, op: &ContentOperation) {
        self.stats.native_text_ops += 1;
        let bounds = self.text_show_bounds(op);
        let text = RetainedTextOp::from_content_operation(op);
        let approx_bytes = estimate_retained_text_op_bytes(&text);
        self.ops.push(DisplayOp::NativeTextOp {
            text,
            approx_bytes,
            bounds,
        });
    }

    fn text_show_bounds(&self, op: &ContentOperation) -> Option<RenderBounds> {
        if !matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
            return None;
        }
        let mut start = self.gs.clone();
        match op.operator.as_str() {
            "'" => start.process(&ContentOperation::new("T*", Vec::new())),
            "\"" => {
                if let Some(word_spacing) = op.number(0) {
                    start.text.word_spacing = word_spacing;
                }
                if let Some(char_spacing) = op.number(1) {
                    start.text.char_spacing = char_spacing;
                }
                start.process(&ContentOperation::new("T*", Vec::new()));
            }
            _ => {}
        }
        let start_tm = start.text.tm;
        let mut end = self.gs.clone();
        end.process(op);
        RenderBounds::from_text_run(
            start_tm,
            end.text.tm,
            &self.ctm(),
            &self.viewport,
            start.text.font_size,
            start.text.rise,
            2.0,
        )
    }

    fn push_state_op(&mut self, op: &ContentOperation) {
        let state = GraphicsStateDescriptor::compile_with_resources(op, Some(self.resources));
        self.ops.push(DisplayOp::StateOp {
            approx_bytes: estimate_graphics_state_descriptor_bytes(&state),
            state,
        });
    }

    fn push_native_xobject(&mut self, op: &ContentOperation) {
        let Some(name) = op.name(0) else {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "Do".to_string(),
                reason: "XObject operator is missing its resource name".to_string(),
            });
            return;
        };
        let subtype = self
            .resources
            .xobject_subtypes
            .get(name)
            .map(String::as_str);
        match subtype {
            Some("Image") => {
                let name = name.to_string();
                self.stats.native_image_xobjects += 1;
                self.ops.push(DisplayOp::NativeImageXObject {
                    approx_bytes: estimate_named_resource_bytes(&name),
                    name,
                    bounds: self.unit_square_bounds(),
                });
            }
            Some("Form") => {
                let name = name.to_string();
                self.stats.native_form_xobjects += 1;
                self.ops.push(DisplayOp::NativeFormXObject {
                    bounds: self.form_xobject_bounds(&name),
                    approx_bytes: estimate_named_resource_bytes(&name),
                    name,
                });
            }
            Some(other) => {
                self.unsupported.push(UnsupportedRenderOp {
                    operator: "Do".to_string(),
                    reason: format!("XObject resource /{name} has unsupported Subtype /{other}"),
                });
            }
            None => {
                let reason = if self.resources.xobjects.contains_key(name)
                    || self.resources.xobject_stream_dicts.contains_key(name)
                {
                    format!("XObject resource /{name} has no /Subtype")
                } else {
                    format!("XObject resource /{name} is missing")
                };
                self.unsupported.push(UnsupportedRenderOp {
                    operator: "Do".to_string(),
                    reason,
                });
            }
        }
    }

    fn push_native_shading(&mut self, op: &ContentOperation) {
        let Some(name) = op.name(0) else {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "sh".to_string(),
                reason: "named shading operator is missing its resource name".to_string(),
            });
            return;
        };
        if !self.resources.shadings.contains_key(name) {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "sh".to_string(),
                reason: format!("named shading resource /{name} is missing"),
            });
            return;
        }
        self.stats.native_shading_ops += 1;
        self.ops.push(DisplayOp::NativeShadingOp {
            approx_bytes: estimate_named_resource_bytes(name),
            name: name.to_string(),
            bounds: self.current_clip_bounds,
        });
    }

    fn push_native_inline_image(&mut self, data_op: &ContentOperation) {
        let Some(params) = self.pending_inline_params.take() else {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "inline_image_data".to_string(),
                reason: "malformed inline image sequence: data has no preceding ID".to_string(),
            });
            return;
        };
        let Some(data) = data_op.string_bytes(0) else {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "inline_image_data".to_string(),
                reason: "malformed inline image sequence: inline image data operand is missing"
                    .to_string(),
            });
            return;
        };
        let approx_bytes = estimate_inline_image_bytes(&params, data);
        self.stats.native_inline_images += 1;
        self.ops.push(DisplayOp::NativeInlineImage {
            image: RetainedInlineImage {
                params,
                data: data.to_vec(),
            },
            approx_bytes,
            bounds: self.unit_square_bounds(),
        });
        self.inline_data_pending_end = true;
    }

    fn unit_square_bounds(&self) -> Option<RenderBounds> {
        RenderBounds::from_unit_square(&self.ctm(), &self.viewport, 1.0)
    }

    fn form_xobject_bounds(&self, name: &str) -> Option<RenderBounds> {
        const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let bbox = *self.resources.xobject_bboxes.get(name)?;
        let matrix = self
            .resources
            .xobject_matrices
            .get(name)
            .copied()
            .unwrap_or(IDENTITY);
        let ctm = Transform2D::from(matrix).concat(&self.ctm());
        RenderBounds::from_bbox(bbox, &ctm, &self.viewport, 1.0)
    }
}

fn resolve_simple_color(color: &Color, alpha: f32) -> PixelColor {
    if matches!(color.space, ColorSpace::Named(_)) {
        return crate::render::color::RenderColor::transparent().to_pixel_color();
    }
    ColorSpaceHandler::strict_to_render_color(color, alpha)
        .unwrap_or_else(|_| crate::render::color::RenderColor::transparent())
        .to_pixel_color()
}

fn simple_cmyk_components(color: &Color) -> Option<[f32; 4]> {
    if !matches!(color.space, ColorSpace::DeviceCMYK) {
        return None;
    }
    if color.components.len() != 4 || !color.components.iter().all(|value| value.is_finite()) {
        return None;
    }
    Some([
        color.components[0].clamp(0.0, 1.0) as f32,
        color.components[1].clamp(0.0, 1.0) as f32,
        color.components[2].clamp(0.0, 1.0) as f32,
        color.components[3].clamp(0.0, 1.0) as f32,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::operation::Operand;
    use crate::object::{PdfDictionary, PdfObject};
    use crate::render::buffer::{BLACK, RED};

    fn op(operator: &str, operands: Vec<Operand>) -> ContentOperation {
        ContentOperation::new(operator, operands)
    }

    fn num(n: f64) -> Operand {
        Operand::Real(n)
    }

    fn tiny_cpu_path_mask() -> std::sync::Arc<RasterizedGlyphMask> {
        std::sync::Arc::new(
            RasterizedGlyphMask::from_alpha(0, 0, 1, 1, vec![255])
                .expect("test mask dimensions match alpha"),
        )
    }

    fn fill_mask_cache_key(id: u64) -> CpuPathFillMaskCacheKey {
        CpuPathFillMaskCacheKey {
            path_hash: id,
            fill_rule: 0,
            flatness: 0,
            a: 1000,
            b: 0,
            c: 0,
            d: 1000,
            frac_e: 0,
            frac_f: 0,
        }
    }

    fn stroke_mask_cache_key(id: u64) -> CpuPathStrokeMaskCacheKey {
        CpuPathStrokeMaskCacheKey {
            path_hash: id,
            width: 1000,
            flatness: 0,
            cap: 0,
            join: 0,
            miter_limit: 10_000,
            a: 1000,
            b: 0,
            c: 0,
            d: 1000,
            frac_e: 0,
            frac_f: 0,
        }
    }

    #[test]
    fn cpu_path_fill_mask_cache_evicts_lru_without_clearing_hot_entries() {
        let mut cache = CpuPathFillMaskCache::default();
        for id in 0..CpuPathFillMaskCache::MAX_ENTRIES {
            cache.insert(fill_mask_cache_key(id as u64), tiny_cpu_path_mask());
        }

        let hot = fill_mask_cache_key(0);
        let cold = fill_mask_cache_key(1);
        let fresh = fill_mask_cache_key(CpuPathFillMaskCache::MAX_ENTRIES as u64);
        assert!(cache.get(&hot).is_some());
        cache.insert(fresh.clone(), tiny_cpu_path_mask());

        assert!(cache.entries.contains_key(&hot));
        assert!(!cache.entries.contains_key(&cold));
        assert!(cache.entries.contains_key(&fresh));
        assert_eq!(cache.entries.len(), CpuPathFillMaskCache::MAX_ENTRIES);
        assert_eq!(cache.order.len(), cache.entries.len());
        assert_eq!(
            cache.bytes,
            cache
                .entries
                .values()
                .map(|(_, _, bytes)| *bytes)
                .sum::<usize>()
        );
    }

    #[test]
    fn cpu_path_stroke_mask_cache_evicts_lru_without_clearing_hot_entries() {
        let mut cache = CpuPathStrokeMaskCache::default();
        for id in 0..CpuPathStrokeMaskCache::MAX_ENTRIES {
            cache.insert(stroke_mask_cache_key(id as u64), tiny_cpu_path_mask());
        }

        let hot = stroke_mask_cache_key(0);
        let cold = stroke_mask_cache_key(1);
        let fresh = stroke_mask_cache_key(CpuPathStrokeMaskCache::MAX_ENTRIES as u64);
        assert!(cache.get(&hot).is_some());
        cache.insert(fresh.clone(), tiny_cpu_path_mask());

        assert!(cache.entries.contains_key(&hot));
        assert!(!cache.entries.contains_key(&cold));
        assert!(cache.entries.contains_key(&fresh));
        assert_eq!(cache.entries.len(), CpuPathStrokeMaskCache::MAX_ENTRIES);
        assert_eq!(cache.order.len(), cache.entries.len());
        assert_eq!(
            cache.bytes,
            cache
                .entries
                .values()
                .map(|(_, _, bytes)| *bytes)
                .sum::<usize>()
        );
    }

    #[test]
    fn captures_and_replays_simple_fill() {
        let ops = vec![
            op("rg", vec![num(1.0), num(0.0), num(0.0)]),
            op("re", vec![num(10.0), num(10.0), num(20.0), num(20.0)]),
            op("f", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let list = build_display_list(&ops, viewport.clone(), &PageResources::default());

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.fills, 1);
        let buf = render_display_list(&list, RenderMode::Compat).expect("vector-only list renders");
        assert_eq!(buf.get_pixel(20, 30), RED);
    }

    #[test]
    fn path_render_bounds_are_full_page_pixel_space() {
        let full = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let mut path = Path::new();
        path.rect(10.0, 10.0, 20.0, 20.0);

        let bounds = RenderBounds::from_path(&path, &Transform2D::identity(), &full, 0.0)
            .expect("rectangle should produce bounds");

        assert!(bounds.intersects_viewport(&full.pixel_window(10, 20, 5, 5)));
        assert!(!bounds.intersects_viewport(&full.pixel_window(0, 0, 5, 5)));
    }

    #[test]
    fn captured_fill_path_carries_culling_bounds() {
        let ops = vec![
            op("re", vec![num(10.0), num(10.0), num(20.0), num(20.0)]),
            op("f", vec![]),
        ];
        let full = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let list = build_display_list(&ops, full.clone(), &PageResources::default());
        let Some(DisplayOp::FillPath {
            bounds: Some(bounds),
            ..
        }) = list
            .ops
            .iter()
            .find(|op| matches!(op, DisplayOp::FillPath { .. }))
        else {
            panic!("expected captured fill path bounds");
        };

        assert!(bounds.intersects_viewport(&full.pixel_window(10, 20, 5, 5)));
        assert!(!bounds.intersects_viewport(&full.pixel_window(0, 0, 5, 5)));
    }

    #[test]
    fn captures_clip_save_restore_and_stroke() {
        let ops = vec![
            op("q", vec![]),
            op("re", vec![num(0.0), num(0.0), num(20.0), num(20.0)]),
            op("W", vec![]),
            op("n", vec![]),
            op("w", vec![num(3.0)]),
            op("m", vec![num(2.0), num(2.0)]),
            op("l", vec![num(18.0), num(18.0)]),
            op("S", vec![]),
            op("Q", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(&ops, viewport.clone(), &PageResources::default());

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.clips, 1);
        assert_eq!(list.stats.strokes, 1);
        assert_eq!(list.stats.max_stack_depth, 1);
        let buf = render_display_list(&list, RenderMode::Compat).expect("vector-only list renders");
        assert_ne!(buf.get_pixel(10, 10), WHITE);
        assert_eq!(buf.get_pixel(19, 1), WHITE);
    }

    #[test]
    fn cpu_render_device_save_restore_reuses_active_clip_dag_node() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut device = CpuRenderDevice::new(viewport, RenderMode::Compat);
        let mut first = Path::new();
        first.rect(2.0, 2.0, 12.0, 12.0);
        let mut second = Path::new();
        second.rect(8.0, 8.0, 8.0, 8.0);

        device.clip_path(&first, &Transform2D::identity(), FillRule::NonZero);
        assert!(matches!(
            device.current_clip.state,
            ClipState::Rectangle { .. }
        ));
        let first_node = Arc::clone(&device.current_clip);

        device.save();
        assert!(Arc::ptr_eq(
            device.clip_stack.last().expect("saved clip node"),
            &first_node
        ));

        device.clip_path(&second, &Transform2D::identity(), FillRule::NonZero);
        assert!(!Arc::ptr_eq(&device.current_clip, &first_node));

        device.restore();
        assert!(Arc::ptr_eq(&device.current_clip, &first_node));
        assert!(device.buf.clip_mask().is_some());
    }

    #[test]
    fn malformed_graphics_state_marks_display_list_unsupported() {
        let ops = vec![
            op("w", vec![Operand::Name("BadWidth".to_string())]),
            op("re", vec![num(0.0), num(0.0), num(20.0), num(20.0)]),
            op("S", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(&ops, viewport.clone(), &PageResources::default());

        assert!(!list.is_fully_supported());
        assert_eq!(list.unsupported.len(), 1);
        assert_eq!(list.unsupported[0].operator, "w");
        assert!(
            list.unsupported[0]
                .reason
                .contains("malformed graphics-state operator 'w'"),
            "got {}",
            list.unsupported[0].reason
        );

        let restore_underflow =
            build_display_list(&[op("Q", vec![])], viewport, &PageResources::default());
        assert!(!restore_underflow.is_fully_supported());
        assert_eq!(restore_underflow.unsupported.len(), 1);
        assert_eq!(restore_underflow.unsupported[0].operator, "Q");
        assert!(
            restore_underflow.unsupported[0]
                .reason
                .contains("restore has no saved graphics state"),
            "got {}",
            restore_underflow.unsupported[0].reason
        );
    }

    #[test]
    fn graphics_state_restore_side_stack_desync_marks_display_list_unsupported() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let resources = PageResources::default();
        let mut builder = DisplayListBuilder::new(viewport, &resources);
        builder.gs.process(&op("q", vec![]));

        builder.dispatch(&op("Q", vec![]));

        assert!(builder.unsupported.iter().any(|item| {
            item.operator == "Q" && item.reason.contains("side-stack state is unavailable")
        }));
        assert!(!builder
            .ops
            .iter()
            .any(|item| matches!(item, DisplayOp::Restore)));
    }

    #[test]
    fn malformed_device_sc_arity_marks_display_list_unsupported() {
        let ops = vec![
            op("cs", vec![Operand::Name("DeviceRGB".to_string())]),
            op("sc", vec![num(1.0), num(0.0)]),
            op("re", vec![num(0.0), num(0.0), num(20.0), num(20.0)]),
            op("f", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(!list.is_fully_supported());
        assert!(list.unsupported.iter().any(|item| {
            item.operator == "sc"
                && item
                    .reason
                    .contains("nonstroking DeviceRGB color expects exactly 3")
        }));
    }

    #[test]
    fn text_is_replayable_as_native_operation() {
        let ops = vec![
            op("BT", vec![]),
            op("Tj", vec![Operand::String(b"hello".to_vec())]),
            op("ET", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(&ops, viewport.clone(), &PageResources::default());

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.text_ops, 1);
        assert_eq!(list.stats.native_text_ops, 3);
        assert!(list.ops.iter().any(|display_op| matches!(
            display_op,
            DisplayOp::NativeTextOp {
                text: RetainedTextOp::Show(bytes),
                ..
            } if bytes == b"hello"
        )));
    }

    #[test]
    fn malformed_text_marks_display_list_unsupported() {
        let ops = vec![
            op("BT", vec![]),
            op("Tf", vec![Operand::Name("F1".to_string())]),
            op("Tj", vec![Operand::String(b"hello".to_vec())]),
            op("ET", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(&ops, viewport.clone(), &PageResources::default());

        assert!(!list.is_fully_supported());
        assert_eq!(list.unsupported.len(), 1);
        assert_eq!(list.unsupported[0].operator, "Tf");
        assert!(
            list.unsupported[0]
                .reason
                .contains("malformed text operator 'Tf'"),
            "got {}",
            list.unsupported[0].reason
        );

        for (ops, expected_operator, expected_reason) in [
            (
                vec![op("Tj", vec![Operand::String(b"hello".to_vec())])],
                "Tj",
                "requires active text object",
            ),
            (vec![op("ET", vec![])], "ET", "ET has no active text object"),
            (
                vec![op("BT", vec![]), op("BT", vec![]), op("ET", vec![])],
                "BT",
                "nested BT",
            ),
            (
                vec![
                    op("BT", vec![]),
                    op("Tj", vec![Operand::String(b"hello".to_vec())]),
                ],
                "BT",
                "BT has no closing ET",
            ),
        ] {
            let list = build_display_list(&ops, viewport.clone(), &PageResources::default());
            assert!(!list.is_fully_supported());
            assert_eq!(list.unsupported.len(), 1);
            assert_eq!(list.unsupported[0].operator, expected_operator);
            assert!(
                list.unsupported[0].reason.contains(expected_reason),
                "got {}",
                list.unsupported[0].reason
            );
        }
    }

    #[test]
    fn retained_text_conversion_rejects_malformed_operands_locally() {
        let retained = RetainedTextOp::from_content_operation(&op(
            "Tf",
            vec![Operand::Name("F1".to_string())],
        ));

        match retained {
            RetainedTextOp::Unsupported { operator, reason } => {
                assert_eq!(operator, "Tf");
                assert!(
                    reason.contains("malformed text operator 'Tf'"),
                    "got {reason}"
                );
            }
            other => panic!("malformed retained text converted to {other:?}"),
        }
    }

    #[test]
    fn malformed_type3_glyph_metric_marks_display_list_unsupported() {
        let ops = vec![op("d1", vec![num(500.0), num(0.0)])];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(!list.is_fully_supported());
        assert_eq!(list.unsupported.len(), 1);
        assert_eq!(list.unsupported[0].operator, "d1");
        assert!(
            list.unsupported[0]
                .reason
                .contains("malformed Type 3 glyph metric operator 'd1'"),
            "got {}",
            list.unsupported[0].reason
        );
    }

    #[test]
    fn malformed_inline_image_sequence_marks_display_list_unsupported() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let inline_params = || {
            vec![
                Operand::Name("Width".to_string()),
                num(1.0),
                Operand::Name("Height".to_string()),
                num(1.0),
            ]
        };

        let begin_without_id = build_display_list(
            &[op("BI", vec![])],
            viewport.clone(),
            &PageResources::default(),
        );

        assert!(!begin_without_id.is_fully_supported());
        assert!(begin_without_id.unsupported.iter().any(|item| {
            item.operator == "BI" && item.reason.contains("BI has no following ID")
        }));

        let id_without_bi = build_display_list(
            &[op("ID", inline_params())],
            viewport.clone(),
            &PageResources::default(),
        );

        assert!(!id_without_bi.is_fully_supported());
        assert!(id_without_bi.unsupported.iter().any(|item| {
            item.operator == "ID" && item.reason.contains("ID has no preceding BI")
        }));

        let data_without_id = build_display_list(
            &[op("inline_image_data", vec![Operand::String(vec![0x80])])],
            viewport.clone(),
            &PageResources::default(),
        );

        assert!(!data_without_id.is_fully_supported());
        assert!(data_without_id.unsupported.iter().any(|item| {
            item.operator == "inline_image_data" && item.reason.contains("no preceding ID")
        }));

        let unterminated_id = build_display_list(
            &[op("BI", vec![]), op("ID", inline_params())],
            viewport.clone(),
            &PageResources::default(),
        );

        assert!(!unterminated_id.is_fully_supported());
        assert!(unterminated_id.unsupported.iter().any(|item| {
            item.operator == "ID" && item.reason.contains("no following image data")
        }));

        let missing_data_operand = build_display_list(
            &[
                op("BI", vec![]),
                op("ID", inline_params()),
                op("inline_image_data", vec![]),
            ],
            viewport.clone(),
            &PageResources::default(),
        );

        assert!(!missing_data_operand.is_fully_supported());
        assert!(missing_data_operand.unsupported.iter().any(|item| {
            item.operator == "inline_image_data" && item.reason.contains("operand is missing")
        }));

        let missing_ei = build_display_list(
            &[
                op("BI", vec![]),
                op("ID", inline_params()),
                op("inline_image_data", vec![Operand::String(vec![0x80])]),
            ],
            viewport,
            &PageResources::default(),
        );

        assert!(!missing_ei.is_fully_supported());
        assert!(missing_ei.unsupported.iter().any(|item| {
            item.operator == "inline_image_data" && item.reason.contains("no following EI")
        }));
    }

    #[test]
    fn text_showing_native_op_records_tile_culling_bounds() {
        let ops = vec![
            op("BT", vec![]),
            op("Tf", vec![Operand::Name("F1".to_string()), num(12.0)]),
            op("Td", vec![num(40.0), num(40.0)]),
            op("Tj", vec![Operand::String(b"hello".to_vec())]),
            op("ET", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(&ops, viewport.clone(), &PageResources::default());
        let text_bounds = list.ops.iter().find_map(|op| match op {
            DisplayOp::NativeTextOp {
                text: RetainedTextOp::Show(_),
                bounds: Some(bounds),
                ..
            } => Some(*bounds),
            _ => None,
        });

        let bounds = text_bounds.expect("showing text should carry conservative bounds");
        assert!(bounds.intersects_viewport(&viewport));
        assert!(!bounds.intersects_viewport(&viewport.pixel_window(0, 0, 10, 10)));
    }

    #[test]
    fn text_showing_bounds_use_pre_advance_text_matrix() {
        let ops = vec![
            op("BT", vec![]),
            op("Tf", vec![Operand::Name("F1".to_string()), num(12.0)]),
            op("Td", vec![num(40.0), num(40.0)]),
            op("Tj", vec![Operand::String(b"hello".to_vec())]),
            op("ET", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(&ops, viewport.clone(), &PageResources::default());
        let text_bounds = list.ops.iter().find_map(|op| match op {
            DisplayOp::NativeTextOp {
                text: RetainedTextOp::Show(_),
                bounds: Some(bounds),
                ..
            } => Some(*bounds),
            _ => None,
        });

        let bounds = text_bounds.expect("showing text should carry conservative bounds");
        assert!(
            bounds.intersects_viewport(&viewport.pixel_window(38, 45, 4, 20)),
            "text culling bounds must include the glyph start, not only the post-showing advance"
        );
    }

    #[test]
    fn alpha_ext_gstate_does_not_force_page_compatibility_run() {
        let ops = vec![
            op("gs", vec![Operand::Name("GS1".to_string())]),
            op("rg", vec![num(1.0), num(0.0), num(0.0)]),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        let mut gs = PdfDictionary::empty();
        gs.insert("ca", PdfObject::Real(0.5));
        gs.insert("CA", PdfObject::Real(0.5));
        resources.ext_g_states.insert("GS1".to_string(), gs);

        let list = build_display_list(&ops, viewport.clone(), &resources);

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.transparency_ops, 1);
        assert!(list.stats.requires_transparent_page_group);
        assert!(matches!(list.ops[0], DisplayOp::StateOp { .. }));
        assert!(matches!(
            list.ops.iter().find(|op| matches!(op, DisplayOp::FillPath { .. })),
            Some(DisplayOp::FillPath { state, .. }) if state.fill_color[3] < 255
        ));
    }

    #[test]
    fn complete_no_paint_alpha_extgstate_does_not_require_page_group() {
        let ops = vec![op("gs", vec![Operand::Name("GS1".to_string())])];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        let mut gs = PdfDictionary::empty();
        gs.insert("ca", PdfObject::Real(0.0));
        gs.insert("CA", PdfObject::Real(0.0));
        resources.ext_g_states.insert("GS1".to_string(), gs);

        let list = build_display_list(&ops, viewport, &resources);

        assert_eq!(list.stats.transparency_ops, 1);
        assert!(!list.stats.requires_transparent_page_group);
    }

    #[test]
    fn transparent_form_xobject_sets_page_group_stat_without_raw_rescan() {
        let ops = vec![op("Do", vec![Operand::Name("Fm1".to_string())])];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        resources
            .xobject_subtypes
            .insert("Fm1".to_string(), "Form".to_string());
        let mut group = PdfDictionary::empty();
        group.insert("S", PdfObject::Name("Transparency".to_string()));
        let mut form = PdfDictionary::empty();
        form.insert("Group", PdfObject::Dictionary(group));
        resources
            .xobject_stream_dicts
            .insert("Fm1".to_string(), form);

        let list = build_display_list(&ops, viewport, &resources);

        assert_eq!(list.stats.form_xobjects, 1);
        assert!(list.stats.requires_transparent_page_group);
    }

    #[test]
    fn ordinary_marked_content_does_not_force_page_compatibility_run() {
        let ops = vec![
            op(
                "BDC",
                vec![
                    Operand::Name("Span".to_string()),
                    Operand::Dictionary(vec![(
                        "Lang".to_string(),
                        Operand::String(b"en-US".to_vec()),
                    )]),
                ],
            ),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("EMC", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);

        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.optional_content_ops, 0);
        assert!(list
            .ops
            .iter()
            .any(|op| matches!(op, DisplayOp::FillPath { .. })));
    }

    #[test]
    fn optional_content_marked_content_replays_as_state_ops_without_page_fallback() {
        let ops = vec![
            op(
                "BDC",
                vec![
                    Operand::Name("OC".to_string()),
                    Operand::Name("Layer1".to_string()),
                ],
            ),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("EMC", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        let mut ocg = PdfDictionary::empty();
        ocg.insert("Type", PdfObject::Name("OCG".to_string()));
        resources
            .properties
            .insert("Layer1".to_string(), PdfObject::Dictionary(ocg));

        let list = build_display_list(&ops, viewport, &resources);

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.optional_content_ops, 1);
        assert!(list.ops.iter().any(|op| matches!(
            op,
            DisplayOp::StateOp {
                state: GraphicsStateDescriptor::BeginMarkedContentWithProperties { tag, .. },
                ..
            } if tag == "OC"
        )));
    }

    #[test]
    fn inline_optional_content_dictionary_replays_as_state_ops_without_page_fallback() {
        let ops = vec![
            op(
                "BDC",
                vec![
                    Operand::Name("OC".to_string()),
                    Operand::Dictionary(vec![(
                        "Type".to_string(),
                        Operand::Name("OCMD".to_string()),
                    )]),
                ],
            ),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("EMC", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);

        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.optional_content_ops, 1);
    }

    #[test]
    fn malformed_marked_content_marks_display_list_unsupported() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let malformed_props = build_display_list(
            &[op("BDC", vec![Operand::Name("OC".to_string())])],
            viewport.clone(),
            &PageResources::default(),
        );

        assert!(!malformed_props.is_fully_supported());
        assert!(malformed_props.unsupported.iter().any(|item| {
            item.operator == "BDC"
                && item
                    .reason
                    .contains("malformed marked-content operator 'BDC'")
        }));

        let unmatched_end =
            build_display_list(&[op("EMC", vec![])], viewport, &PageResources::default());

        assert!(!unmatched_end.is_fully_supported());
        assert!(unmatched_end.unsupported.iter().any(|item| {
            item.operator == "EMC" && item.reason.contains("no active marked-content sequence")
        }));
    }

    #[test]
    fn unterminated_marked_content_marks_display_list_unsupported() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(
            &[op("BMC", vec![Operand::Name("Span".to_string())])],
            viewport,
            &PageResources::default(),
        );

        assert!(!list.is_fully_supported());
        assert!(list.unsupported.iter().any(|item| {
            item.operator == "BMC"
                && item
                    .reason
                    .contains("malformed marked-content sequence: 1 unterminated")
        }));
    }

    #[test]
    fn unbalanced_compatibility_section_marks_display_list_unsupported() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let unmatched_end = build_display_list(
            &[op("EX", vec![])],
            viewport.clone(),
            &PageResources::default(),
        );

        assert!(!unmatched_end.is_fully_supported());
        assert!(unmatched_end.unsupported.iter().any(|item| {
            item.operator == "EX" && item.reason.contains("no active compatibility section")
        }));

        let unterminated_begin =
            build_display_list(&[op("BX", vec![])], viewport, &PageResources::default());

        assert!(!unterminated_begin.is_fully_supported());
        assert!(unterminated_begin.unsupported.iter().any(|item| {
            item.operator == "BX"
                && item
                    .reason
                    .contains("malformed compatibility-section sequence: 1 unterminated")
        }));
    }

    #[test]
    fn malformed_path_marks_display_list_unsupported() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let missing_current_point = build_display_list(
            &[op("l", vec![num(1.0), num(1.0)])],
            viewport.clone(),
            &PageResources::default(),
        );

        assert!(!missing_current_point.is_fully_supported());
        assert!(missing_current_point.unsupported.iter().any(|item| {
            item.operator == "l" && item.reason.contains("requires an active current point")
        }));

        let operand_bearing_paint = build_display_list(
            &[
                op("m", vec![num(1.0), num(1.0)]),
                op("l", vec![num(8.0), num(8.0)]),
                op("S", vec![num(1.0)]),
            ],
            viewport.clone(),
            &PageResources::default(),
        );

        assert!(!operand_bearing_paint.is_fully_supported());
        assert!(operand_bearing_paint
            .unsupported
            .iter()
            .any(|item| { item.operator == "S" && item.reason.contains("expected no operands") }));

        let empty_clip = build_display_list(
            &[op("W", Vec::new()), op("n", Vec::new())],
            viewport.clone(),
            &PageResources::default(),
        );
        assert!(empty_clip.is_fully_supported());
        assert!(
            empty_clip
                .ops
                .iter()
                .any(|item| matches!(item, DisplayOp::Clip { path, bounds, .. } if path.is_empty() && bounds.is_none()))
        );

        let dangling_clip = build_display_list(
            &[
                op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
                op("W", Vec::new()),
            ],
            viewport.clone(),
            &PageResources::default(),
        );
        assert!(!dangling_clip.is_fully_supported());
        assert!(dangling_clip.unsupported.iter().any(|item| {
            item.operator == "W"
                && item
                    .reason
                    .contains("has no following path painting operator")
        }));

        let repeated_clip = build_display_list(
            &[
                op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
                op("W", Vec::new()),
                op("W*", Vec::new()),
                op("n", Vec::new()),
            ],
            viewport,
            &PageResources::default(),
        );
        assert!(!repeated_clip.is_fully_supported());
        assert!(repeated_clip.unsupported.iter().any(|item| {
            item.operator == "W*" && item.reason.contains("not terminated before another")
        }));
    }

    #[test]
    fn malformed_resource_invocation_marks_display_list_unsupported() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        for malformed_op in [
            op("Do", Vec::new()),
            op("Do", vec![Operand::Real(1.0)]),
            op(
                "Do",
                vec![Operand::Name("Im1".to_string()), Operand::Real(1.0)],
            ),
            op("sh", Vec::new()),
            op("sh", vec![Operand::Real(1.0)]),
            op(
                "sh",
                vec![Operand::Name("S1".to_string()), Operand::Real(1.0)],
            ),
        ] {
            let list =
                build_display_list(&[malformed_op], viewport.clone(), &PageResources::default());

            assert!(!list.is_fully_supported());
            assert!(list.unsupported.iter().any(|item| {
                item.reason.contains(&format!(
                    "malformed resource invocation operator '{}'",
                    item.operator
                ))
            }));
        }
    }

    #[test]
    fn image_xobject_native_op_records_tile_culling_bounds() {
        let ops = vec![
            op(
                "cm",
                vec![
                    num(10.0),
                    num(0.0),
                    num(0.0),
                    num(10.0),
                    num(30.0),
                    num(30.0),
                ],
            ),
            op("Do", vec![Operand::Name("Im1".to_string())]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut resources = PageResources::default();
        resources
            .xobject_subtypes
            .insert("Im1".to_string(), "Image".to_string());

        let list = build_display_list(&ops, viewport.clone(), &resources);
        let image_entry = list.ops.iter().find_map(|op| match op {
            DisplayOp::NativeImageXObject { name, bounds, .. } => Some((name, *bounds)),
            _ => None,
        });

        let (name, bounds) = image_entry.expect("native image op should carry typed name/bounds");
        assert_eq!(name, "Im1");
        let bounds = bounds.expect("native image op should carry bounds");
        assert!(bounds.intersects_viewport(&viewport));
        assert!(!bounds.intersects_viewport(&viewport.pixel_window(0, 0, 10, 10)));
    }

    #[test]
    fn form_xobject_native_op_stores_typed_resource_name() {
        let ops = vec![op("Do", vec![Operand::Name("Fm1".to_string())])];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut resources = PageResources::default();
        resources
            .xobject_subtypes
            .insert("Fm1".to_string(), "Form".to_string());

        let list = build_display_list(&ops, viewport, &resources);

        assert!(matches!(
            &list.ops[0],
            DisplayOp::NativeFormXObject { name, .. } if name == "Fm1"
        ));
    }

    #[test]
    fn malformed_xobject_subtype_marks_display_list_unsupported() {
        for (resources, expected) in [
            (PageResources::default(), "XObject resource /Xm1 is missing"),
            (
                {
                    let mut resources = PageResources::default();
                    resources.xobjects.insert("Xm1".to_string(), (5, 0));
                    resources
                        .xobject_stream_dicts
                        .insert("Xm1".to_string(), PdfDictionary::empty());
                    resources
                },
                "XObject resource /Xm1 has no /Subtype",
            ),
            (
                {
                    let mut resources = PageResources::default();
                    resources.xobjects.insert("Xm1".to_string(), (5, 0));
                    resources
                        .xobject_subtypes
                        .insert("Xm1".to_string(), "PS".to_string());
                    resources
                },
                "XObject resource /Xm1 has unsupported Subtype /PS",
            ),
        ] {
            let ops = vec![op("Do", vec![Operand::Name("Xm1".to_string())])];
            let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
            let list = build_display_list(&ops, viewport, &resources);

            assert!(!list.is_fully_supported());
            assert_eq!(list.unsupported.len(), 1);
            assert_eq!(list.unsupported[0].operator, "Do");
            assert_eq!(list.unsupported[0].reason, expected);
            assert!(
                !list
                    .ops
                    .iter()
                    .any(|op| matches!(op, DisplayOp::NativeFormXObject { .. })),
                "malformed XObject must not be retained as a native Form op"
            );
        }
    }

    #[test]
    fn named_shading_is_replayable_as_native_operation() {
        let ops = vec![op("sh", vec![Operand::Name("S1".to_string())])];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        resources.shadings.insert(
            "S1".to_string(),
            PdfObject::Dictionary(PdfDictionary::empty()),
        );

        let list = build_display_list(&ops, viewport.clone(), &resources);

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.shadings, 1);
        assert_eq!(list.stats.native_shading_ops, 1);
        assert!(matches!(
            &list.ops[0],
            DisplayOp::NativeShadingOp { name, .. } if name == "S1"
        ));
    }

    #[test]
    fn missing_named_shading_is_explicitly_unsupported() {
        let ops = vec![op("sh", vec![Operand::Name("S1".to_string())])];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);

        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(!list.is_fully_supported());
        assert_eq!(list.stats.shadings, 1);
        assert_eq!(list.stats.native_shading_ops, 0);
        assert_eq!(list.unsupported.len(), 1);
        assert_eq!(list.unsupported[0].operator, "sh");
        assert!(list.unsupported[0].reason.contains("missing"));
    }

    #[test]
    fn pattern_fill_uses_native_path_replay_when_resource_is_available() {
        let ops = vec![
            op("q", vec![]),
            op("cs", vec![Operand::Name("Pattern".to_string())]),
            op("scn", vec![Operand::Name("P1".to_string())]),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("Q", vec![]),
            op("rg", vec![num(1.0), num(0.0), num(0.0)]),
            op("re", vec![num(10.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        resources.patterns.insert(
            "P1".to_string(),
            PdfObject::Dictionary(PdfDictionary::empty()),
        );

        let list = build_display_list(&ops, viewport.clone(), &resources);

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.native_pattern_path_ops, 1);
        assert_eq!(list.stats.fills, 2);
        let pattern_bounds = list.ops.iter().find_map(|op| match op {
            DisplayOp::NativePatternPathOp { bounds, .. } => *bounds,
            _ => None,
        });
        let bounds = pattern_bounds.expect("native pattern path should carry bounds");
        assert!(bounds.intersects_viewport(&viewport));
        assert!(!bounds.intersects_viewport(&viewport.pixel_window(12, 12, 4, 4)));
        assert!(list
            .ops
            .iter()
            .any(|op| matches!(op, DisplayOp::FillPath { .. })));
    }

    #[test]
    fn missing_pattern_resource_is_explicitly_unsupported() {
        let ops = vec![
            op("q", vec![]),
            op("cs", vec![Operand::Name("Pattern".to_string())]),
            op("scn", vec![Operand::Name("P1".to_string())]),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("Q", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);

        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(!list.is_fully_supported());
        assert_eq!(list.stats.native_pattern_path_ops, 0);
        assert_eq!(list.unsupported.len(), 1);
        assert!(list.unsupported[0]
            .reason
            .contains("pattern fill resource /P1 is missing"));
    }

    #[test]
    fn stroke_color_is_replayed() {
        let ops = vec![
            op("RG", vec![num(0.0), num(0.0), num(0.0)]),
            op("w", vec![num(4.0)]),
            op("m", vec![num(5.0), num(5.0)]),
            op("l", vec![num(25.0), num(5.0)]),
            op("S", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 30.0, 30.0], 72);
        let list = build_display_list(&ops, viewport, &PageResources::default());
        let buf = render_display_list(&list, RenderMode::Compat).expect("vector-only list renders");

        assert_eq!(buf.get_pixel(10, 25), BLACK);
    }

    #[test]
    fn standalone_display_list_replay_refuses_native_high_level_ops() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = DisplayList {
            viewport,
            ops: vec![DisplayOp::NativeImageXObject {
                name: "Im1".to_string(),
                approx_bytes: 3,
                bounds: None,
            }],
            stats: DisplayListStats::default(),
            supported: true,
            unsupported: Vec::new(),
        };

        let err = render_display_list(&list, RenderMode::Compat)
            .expect_err("standalone replay must refuse page-context native ops");
        assert!(
            format!("{err}").contains("cannot render native image XObject without page context"),
            "got {err}"
        );
    }

    #[test]
    fn direct_display_list_replay_refuses_native_high_level_ops_on_default_device() {
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = DisplayList {
            viewport: viewport.clone(),
            ops: vec![DisplayOp::NativeImageXObject {
                name: "Im1".to_string(),
                approx_bytes: 3,
                bounds: None,
            }],
            stats: DisplayListStats::default(),
            supported: true,
            unsupported: Vec::new(),
        };
        let mut device = CpuRenderDevice::new(viewport, RenderMode::Compat);

        let err = replay_display_list(&list, &mut device)
            .expect_err("default CPU replay device must refuse page-context native ops");
        assert!(
            format!("{err}")
                .contains("display-list replay device cannot render native image XObject"),
            "got {err}"
        );
    }

    #[test]
    fn ext_gstate_overprint_metadata_is_captured() {
        let mut resources = PageResources::default();
        let mut gs_dict = crate::object::PdfDictionary::empty();
        gs_dict.insert("OP", crate::object::PdfObject::Boolean(true));
        gs_dict.insert("op", crate::object::PdfObject::Boolean(false));
        gs_dict.insert("OPM", crate::object::PdfObject::Integer(1));
        gs_dict.insert(
            "RI",
            crate::object::PdfObject::Name("AbsoluteColorimetric".to_string()),
        );
        resources.ext_g_states.insert("GS1".to_string(), gs_dict);
        let ops = vec![
            op("gs", vec![Operand::Name("GS1".to_string())]),
            op("re", vec![num(1.0), num(1.0), num(5.0), num(5.0)]),
            op("f", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        let list = build_display_list(&ops, viewport, &resources);

        let Some(DisplayOp::FillPath { state, .. }) = list
            .ops
            .iter()
            .find(|op| matches!(op, DisplayOp::FillPath { .. }))
        else {
            panic!("expected captured fill path");
        };
        assert!(state.stroke_overprint);
        assert!(!state.fill_overprint);
        assert_eq!(state.overprint_mode, 1);
        assert_eq!(state.rendering_intent, "AbsoluteColorimetric");
    }

    #[test]
    fn display_list_cmyk_components_require_exact_finite_state() {
        assert_eq!(
            simple_cmyk_components(&Color::device_cmyk(1.2, -0.5, 0.5, 0.0)),
            Some([1.0, 0.0, 0.5, 0.0])
        );
        assert!(simple_cmyk_components(&Color {
            space: ColorSpace::DeviceCMYK,
            components: vec![1.0, 0.0, 0.0],
        })
        .is_none());
        assert!(simple_cmyk_components(&Color {
            space: ColorSpace::DeviceCMYK,
            components: vec![1.0, 0.0, 0.0, 0.0, 0.0],
        })
        .is_none());
        assert!(simple_cmyk_components(&Color {
            space: ColorSpace::DeviceCMYK,
            components: vec![1.0, f64::NAN, 0.0, 0.0],
        })
        .is_none());
    }

    #[test]
    fn display_list_simple_color_requires_exact_finite_device_state() {
        assert_eq!(
            resolve_simple_color(
                &Color {
                    space: ColorSpace::DeviceRGB,
                    components: vec![1.0, 0.0, 0.0],
                },
                1.0,
            ),
            RED
        );
        assert_eq!(
            resolve_simple_color(
                &Color {
                    space: ColorSpace::DeviceRGB,
                    components: vec![1.0, 0.0],
                },
                1.0,
            ),
            crate::render::buffer::TRANSPARENT
        );
        assert_eq!(
            resolve_simple_color(
                &Color {
                    space: ColorSpace::DeviceCMYK,
                    components: vec![0.0, f64::NAN, 0.0, 0.0],
                },
                1.0,
            ),
            crate::render::buffer::TRANSPARENT
        );
    }

    #[test]
    fn render_cache_hits_and_evicts_by_budget() {
        let tile_a = RenderTile {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let tile_b = RenderTile {
            x: 2,
            y: 0,
            width: 2,
            height: 2,
        };
        let key_a = RenderCacheKey::new(1, 72, RenderMode::Compat, tile_a);
        let key_b = RenderCacheKey::new(1, 72, RenderMode::Compat, tile_b);
        let mut cache = RenderCache::new(16, 16);
        let mut buf = PixelBuffer::new_transparent_with_mode(2, 2, RenderMode::Compat);
        buf.set_pixel(0, 0, RED);

        cache.insert(key_a.clone(), buf.clone());
        assert!(cache.get(&key_a).is_some());
        cache.insert(
            key_b.clone(),
            PixelBuffer::new_transparent_with_mode(2, 2, RenderMode::Compat),
        );

        let metrics = cache.metrics();
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.evictions, 1);
        assert_eq!(metrics.bytes, 16);
        assert!(cache.get(&key_b).is_some());
    }

    #[test]
    fn render_cache_skips_oversized_entries() {
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        let key = RenderCacheKey::new(1, 72, RenderMode::Compat, tile);
        let mut cache = RenderCache::new(64, 64);
        cache.insert(
            key.clone(),
            PixelBuffer::new_transparent_with_mode(10, 10, RenderMode::Compat),
        );

        assert!(cache.get(&key).is_none());
        assert_eq!(cache.metrics().skipped_oversized, 1);
    }

    #[test]
    fn render_cache_invalidates_exact_tiles() {
        let tile_a = RenderTile {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let tile_b = RenderTile {
            x: 2,
            y: 0,
            width: 2,
            height: 2,
        };
        let key_a = RenderCacheKey::new(1, 72, RenderMode::Compat, tile_a);
        let key_b = RenderCacheKey::new(1, 72, RenderMode::Compat, tile_b);
        let mut cache = RenderCache::new(64, 16);
        cache.insert(
            key_a.clone(),
            PixelBuffer::new_transparent_with_mode(2, 2, RenderMode::Compat),
        );
        cache.insert(
            key_b.clone(),
            PixelBuffer::new_transparent_with_mode(2, 2, RenderMode::Compat),
        );

        assert_eq!(cache.invalidate_tiles(&[(1, tile_a)]), 1);

        assert!(cache.get(&key_a).is_none());
        assert!(cache.get(&key_b).is_some());
        assert_eq!(cache.metrics().bytes, 16);
    }

    #[test]
    fn render_cache_key_includes_visibility_fingerprint() {
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        let visible = RenderCacheKey::new_with_visibility(
            1,
            72,
            RenderMode::Compat,
            tile,
            "ocg:view:visible",
        );
        let hidden =
            RenderCacheKey::new_with_visibility(1, 72, RenderMode::Compat, tile, "ocg:view:hidden");

        assert_ne!(visible, hidden);
        let changed_revision = RenderCacheKey::new_with_full_identity(
            1,
            72,
            RenderMode::Compat,
            tile,
            "ocg:view:visible",
            "prepress:none",
            "revision:two",
            "contract:one",
        );
        let changed_contract = RenderCacheKey::new_with_full_identity(
            1,
            72,
            RenderMode::Compat,
            tile,
            "ocg:view:visible",
            "prepress:none",
            "revision:one",
            "contract:two",
        );
        let baseline = RenderCacheKey::new_with_full_identity(
            1,
            72,
            RenderMode::Compat,
            tile,
            "ocg:view:visible",
            "prepress:none",
            "revision:one",
            "contract:one",
        );
        assert_ne!(baseline, changed_revision);
        assert_ne!(baseline, changed_contract);
        let mut cache = RenderCache::new(4_000, 4_000);
        cache.insert(
            visible.clone(),
            PixelBuffer::new_transparent_with_mode(10, 10, RenderMode::Compat),
        );

        assert!(cache.get(&hidden).is_none());
        assert!(cache.get(&visible).is_some());
    }
}
