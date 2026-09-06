//! SVG vector output backend (`pdftocairo -svg`-equivalent).
//!
//! # Design (the feasibility-driven approach)
//!
//! The raster interpreter ([`crate::render::page_renderer`]) is tightly coupled
//! to its `PixelBuffer`, so rather than an invasive `RenderSink` trait refactor
//! (high risk to the verified raster path), this is a **sibling renderer** that
//! reuses the same geometry and state primitives:
//!
//! - [`GraphicsState`] for all state operators (`cm`, `q`/`Q`, `Tf`, `Td`,
//!   colour ops, …) — identical interpretation to raster, for free.
//! - [`flatten_path`] for user→device geometry — the SVG paths live in the SAME
//!   device-pixel coordinate space as the raster output, so rasterizing the SVG
//!   reproduces the raster image.
//! - the shared [`glyph_outline`](crate::render::glyph_outline) /
//!   [`text_decode`](crate::render::text_decode) helpers for text-as-outlines.
//!
//! # Per-page vector-vs-raster decision (the roadmap task's rasterize-embed fallback)
//!
//! Pages that use only operations SVG represents natively — paths, text, solid
//! fills/strokes, clipping, opacity — are emitted as **true scalable SVG**.
//! Pages that use operations SVG cannot faithfully express here — unsafe Form
//! XObjects, unsupported shadings, tiling/shading patterns, soft masks, or
//! target-incompatible transparency — fall
//! back to embedding the **whole page as one rasterized PNG** `<image>`
//! (pixel-identical to the raster render).
//!
//! # Regional image fallback (RB-14)
//!
//! Pages with simple affine Image XObject `Do` operations, complete inline
//! images/masks, vector-safe Form XObjects, simple axial/radial shadings, SVG
//! normal-alpha/blend ExtGState opacity, or safe line-style/font/default-state
//! ExtGState dictionaries now use a **regional
//! fallback**: local raster assets are bounded `<image>` elements and shadings
//! are native `<linearGradient>`/`<radialGradient>` fills, while surrounding
//! vector content (paths, text, clips) is preserved as native SVG elements. This
//! avoids whole-page rasterization for common mixed
//! vector/local-resource pages. The eligibility is determined by
//! [`crate::render::vector_fallback::classify_page_for_vector_output`].

use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::{BlendMode, Color, ColorSpace, GraphicsState, LineCap, LineJoin};
use crate::engine::{ContentEngine, PageResources};
use crate::error::{Result, WellfriendError};
use crate::filters::DecodeLimits;
use crate::images::decoder::ImageDecoder;
use crate::images::locator::ImageReference;
use crate::object::{PdfDictionary, PdfObject};
use crate::render::color::{ColorSpaceHandler, RenderColor};
use crate::render::glyph_outline::{font_size_scale, get_upem};
use crate::render::line::DashState;
use crate::render::path::{
    flatten_path, flatten_path_device_transform, stroke_flat_path, FillRule, FlatPath, Path,
};
use crate::render::text_decode::{
    decoded_glyph_strict_horizontal_advance, decoded_glyph_strict_outline, get_font_bytes,
    try_decode_text_bytes, DecodedGlyph,
};
use crate::render::transform::{Transform2D, Viewport};
use crate::render::vector_fallback::{
    classify_page_for_svg_output_with_reader, classify_scoped_svg_vector_output,
    decode_inline_image_region, ensure_regional_stencil_mask, image_device_placement,
    inline_stencil_mask_to_rgba, load_vector_form_program, load_vector_shading,
    load_vector_shading_pattern, load_vector_tiling_pattern, merged_vector_resources,
    resolved_regional_image_color_space_override, stencil_mask_paints_ones,
    vector_uncolored_tiling_paint_color, VectorFallbackDecision, VectorShading, VectorShadingStop,
    VectorTilingPatternPaintType, VectorTilingPatternProgram, MAX_VECTOR_FORM_DEPTH,
    MAX_VECTOR_TILING_PATTERN_CELLS,
};

/// A rendered SVG page plus a flag indicating whether it was emitted as true
/// vector SVG or as a rasterize-and-embed fallback.
pub struct SvgPage {
    /// The complete SVG document for this page.
    pub svg: String,
    /// True when the whole page was embedded as a raster image because it used
    /// operations the vector sink cannot express natively (images, shadings,
    /// patterns, forms, soft masks, blend modes).
    pub is_rasterized: bool,
    /// True when the page used regional vector fallback: vector content is
    /// preserved natively around bounded images or vector-safe Form subprograms.
    pub has_regional_images: bool,
}

pub fn render_page_svg(engine: &ContentEngine, page_number: usize, dpi: u32) -> Result<SvgPage> {
    render_page_svg_with_policy(engine, page_number, dpi, SvgWholePageRasterPolicy::Allow)
}

/// Render a single page to SVG without silently rasterizing the whole page when
/// the vector sink cannot represent the page exactly.
pub fn render_page_svg_strict(
    engine: &ContentEngine,
    page_number: usize,
    dpi: u32,
) -> Result<SvgPage> {
    render_page_svg_with_policy(engine, page_number, dpi, SvgWholePageRasterPolicy::Refuse)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SvgWholePageRasterPolicy {
    Allow,
    Refuse,
}

/// Render a single page to SVG. Uses the shared fallback classifier to decide:
/// - Pure vector: emit all content as SVG paths/text.
/// - Regional image fallback: embed affine image regions as `<image>` elements
///   while preserving surrounding vector content natively.
/// - Whole-page raster: embed the entire page as one PNG `<image>` only when
///   the caller selected compatibility raster fallback.
fn render_page_svg_with_policy(
    engine: &ContentEngine,
    page_number: usize,
    dpi: u32,
    whole_page_policy: SvgWholePageRasterPolicy,
) -> Result<SvgPage> {
    let viewport = engine.page_viewport(page_number, dpi)?;
    let ops = engine.get_page_content(page_number)?;
    let resources = engine.get_page_resources(page_number)?;

    let decision = classify_page_for_svg_output_with_reader(
        &ops,
        &resources,
        viewport.scale,
        engine.document().reader(),
    );

    match decision {
        VectorFallbackDecision::WholePageRaster { reason } => match whole_page_policy {
            SvgWholePageRasterPolicy::Allow => rasterized_page(engine, page_number, dpi, &viewport),
            SvgWholePageRasterPolicy::Refuse => Err(WellfriendError::UnsupportedFeature(format!(
                "strict SVG vector output refuses whole-page raster fallback: {reason}"
            ))),
        },
        VectorFallbackDecision::PureVector => render_vector_page(
            engine,
            &ops,
            &resources,
            &viewport,
            RegionalVectorResources::empty(),
        ),
        VectorFallbackDecision::RegionalImageFallback {
            ref image_names,
            inline_image_count,
            ref form_names,
            ref shading_names,
        } => render_vector_page(
            engine,
            &ops,
            &resources,
            &viewport,
            RegionalVectorResources {
                active: true,
                image_names,
                inline_image_count,
                form_names,
                shading_names,
            },
        ),
    }
}

/// Render a page as native SVG vector content, optionally embedding regional
/// raster images for named Image XObjects.
fn render_vector_page(
    engine: &ContentEngine,
    ops: &[ContentOperation],
    resources: &PageResources,
    viewport: &Viewport,
    regional: RegionalVectorResources<'_>,
) -> Result<SvgPage> {
    let (w, h) = (viewport.width_px, viewport.height_px);
    let mut sink = SvgSink::new(w, h);
    let mut state = SvgRenderState {
        engine,
        resources: resources.clone(),
        viewport: viewport.clone(),
        gs: GraphicsState::default(),
        path: Path::new(),
        pending_clip: None,
        clip_stack: Vec::new(),
        sink: &mut sink,
        regional_image_names: regional.image_names.to_vec(),
        regional_inline_image_count: regional.inline_image_count,
        regional_form_names: regional.form_names.to_vec(),
        regional_shading_names: regional.shading_names.to_vec(),
        pending_inline_params: None,
        form_depth: 0,
        form_object_stack: Vec::new(),
        text_clip_path: None,
        fatal_error: None,
    };
    state.run(ops);
    if let Some(err) = state.fatal_error.take() {
        return Err(err);
    }
    let has_regional = !regional.image_names.is_empty()
        || regional.inline_image_count > 0
        || !regional.form_names.is_empty()
        || !regional.shading_names.is_empty()
        || regional.active;
    Ok(SvgPage {
        svg: sink.finish(),
        is_rasterized: false,
        has_regional_images: has_regional,
    })
}

#[derive(Clone, Copy)]
struct RegionalVectorResources<'a> {
    active: bool,
    image_names: &'a [String],
    inline_image_count: usize,
    form_names: &'a [String],
    shading_names: &'a [String],
}

impl<'a> RegionalVectorResources<'a> {
    fn empty() -> Self {
        Self {
            active: false,
            image_names: &[],
            inline_image_count: 0,
            form_names: &[],
            shading_names: &[],
        }
    }
}

fn regional_image_reference(
    name: &str,
    object_number: u32,
    generation_number: u16,
    dict: &PdfDictionary,
    is_mask: bool,
) -> Result<ImageReference> {
    let context = format!("regional SVG image /{name}");
    let filter = extract_image_filter_names(dict, &context)?;
    let color_space = if is_mask {
        "DeviceGray".to_string()
    } else {
        extract_image_color_space_name(dict, &context)?
    };
    Ok(ImageReference {
        page_number: 0,
        xobject_name: name.to_string(),
        object_number,
        generation_number,
        width: regional_image_positive_u32(dict, "Width", "W", &context)?,
        height: regional_image_positive_u32(dict, "Height", "H", &context)?,
        bits_per_component: regional_image_bits_per_component(dict, is_mask, &filter, &context)?,
        color_space,
        filter,
        is_inline: false,
        is_mask,
        is_smask: false,
        inline_data: None,
    })
}

fn regional_image_positive_u32(
    dict: &PdfDictionary,
    key: &str,
    short_key: &str,
    context: &str,
) -> Result<u32> {
    match dict
        .get_integer(key)
        .or_else(|| dict.get_integer(short_key))
    {
        Some(number) if number > 0 => u32::try_from(number).map_err(|_| {
            WellfriendError::MalformedPdf(format!("{context} /{key} exceeds renderer limit"))
        }),
        Some(_) => Err(WellfriendError::MalformedPdf(format!(
            "{context} /{key} must be a positive integer"
        ))),
        None if dict.contains_key(key) || dict.contains_key(short_key) => Err(
            WellfriendError::MalformedPdf(format!("{context} /{key} is not an integer")),
        ),
        None => Err(WellfriendError::MalformedPdf(format!(
            "{context} missing /{key}"
        ))),
    }
}

fn regional_image_bool(
    dict: &PdfDictionary,
    key: &str,
    short_key: &str,
    context: &str,
) -> Result<bool> {
    if let Some(obj) = dict.get(key) {
        return obj.as_bool().ok_or_else(|| {
            WellfriendError::MalformedPdf(format!("{context} /{key} is not boolean"))
        });
    }
    if let Some(obj) = dict.get(short_key) {
        return obj.as_bool().ok_or_else(|| {
            WellfriendError::MalformedPdf(format!("{context} /{key} is not boolean"))
        });
    }
    Ok(false)
}

fn regional_image_bits_per_component(
    dict: &PdfDictionary,
    is_mask: bool,
    filters: &[String],
    context: &str,
) -> Result<u8> {
    let value = dict
        .get_integer("BitsPerComponent")
        .or_else(|| dict.get_integer("BPC"));
    if is_mask {
        return match value {
            Some(1) => Ok(1),
            None if !dict.contains_key("BitsPerComponent") && !dict.contains_key("BPC") => Ok(1),
            Some(_) => Err(WellfriendError::MalformedPdf(format!(
                "{context} /BitsPerComponent must be 1 for /ImageMask true"
            ))),
            None => Err(WellfriendError::MalformedPdf(format!(
                "{context} /BitsPerComponent is not an integer"
            ))),
        };
    }
    match value {
        Some(number @ (1 | 2 | 4 | 8 | 16)) => Ok(number as u8),
        Some(_) => Err(WellfriendError::MalformedPdf(format!(
            "{context} /BitsPerComponent must be one of 1, 2, 4, 8, or 16"
        ))),
        None if !dict.contains_key("BitsPerComponent")
            && !dict.contains_key("BPC")
            && regional_terminal_filter_carries_sample_depth(filters) =>
        {
            Ok(8)
        }
        None if dict.contains_key("BitsPerComponent") || dict.contains_key("BPC") => Err(
            WellfriendError::MalformedPdf(format!("{context} /BitsPerComponent is not an integer")),
        ),
        None => Err(WellfriendError::MalformedPdf(format!(
            "{context} missing /BitsPerComponent"
        ))),
    }
}

fn regional_terminal_filter_carries_sample_depth(filters: &[String]) -> bool {
    matches!(
        filters.last().map(String::as_str),
        Some("JPXDecode" | "JPX")
    )
}

fn extract_image_color_space_name(dict: &PdfDictionary, context: &str) -> Result<String> {
    match dict.get("ColorSpace").or_else(|| dict.get("CS")) {
        Some(PdfObject::Name(name)) => Ok(canonical_image_color_space_name(name)),
        Some(PdfObject::Array(items)) => items
            .first()
            .and_then(PdfObject::as_name)
            .map(canonical_image_color_space_name)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(format!("{context} malformed /ColorSpace array"))
            }),
        Some(_) => Err(WellfriendError::MalformedPdf(format!(
            "{context} /ColorSpace is not a name or array"
        ))),
        None => Err(WellfriendError::MalformedPdf(format!(
            "{context} missing /ColorSpace"
        ))),
    }
}

fn canonical_image_color_space_name(name: &str) -> String {
    match name {
        "G" => "DeviceGray".to_string(),
        "RGB" => "DeviceRGB".to_string(),
        "CMYK" => "DeviceCMYK".to_string(),
        other => other.to_string(),
    }
}

fn extract_image_filter_names(dict: &PdfDictionary, context: &str) -> Result<Vec<String>> {
    match dict.get("Filter").or_else(|| dict.get("F")) {
        Some(PdfObject::Name(name)) => Ok(vec![name.clone()]),
        Some(PdfObject::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_name().map(str::to_string).ok_or_else(|| {
                    WellfriendError::MalformedPdf(format!("{context} /Filter array is malformed"))
                })
            })
            .collect(),
        Some(_) => Err(WellfriendError::MalformedPdf(format!(
            "{context} /Filter is not a name or array"
        ))),
        None => Ok(Vec::new()),
    }
}

/// Emit a page as a single embedded raster `<image>` (the fallback).
fn rasterized_page(
    engine: &ContentEngine,
    page_number: usize,
    dpi: u32,
    viewport: &Viewport,
) -> Result<SvgPage> {
    use crate::images::encoder::ImageEncoder;
    let buf = engine.render_page(page_number, dpi)?;
    let png = ImageEncoder::encode_png_fast(&buf.to_raw_image())?;
    let b64 = base64_encode(&png);
    let (w, h) = (viewport.width_px, viewport.height_px);
    let svg = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n\
         <image width=\"{w}\" height=\"{h}\" xlink:href=\"data:image/png;base64,{b64}\"/>\n\
         </svg>\n"
    );
    Ok(SvgPage {
        svg,
        is_rasterized: true,
        has_regional_images: false,
    })
}

/// Accumulates SVG element strings and emits the final document.
struct SvgSink {
    width: u32,
    height: u32,
    body: String,
    /// Definitions block (clipPaths).
    defs: String,
    clip_counter: usize,
    gradient_counter: usize,
    mask_counter: usize,
}

impl SvgSink {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            body: String::new(),
            defs: String::new(),
            clip_counter: 0,
            gradient_counter: 0,
            mask_counter: 0,
        }
    }

    fn push_element(&mut self, el: &str) {
        self.body.push_str(el);
        self.body.push('\n');
    }

    /// Register a clip path (device-space polylines) and return its id.
    fn add_clip(&mut self, flat: &FlatPath, rule: FillRule, parent_clip: Option<&str>) -> String {
        let id = format!("clip{}", self.clip_counter);
        self.clip_counter += 1;
        let d = path_data(flat);
        let rule_attr = match rule {
            FillRule::EvenOdd => " clip-rule=\"evenodd\"",
            FillRule::NonZero => "",
        };
        if let Some(parent) = parent_clip {
            self.defs.push_str(&format!(
                "<clipPath id=\"{id}\"><g clip-path=\"url(#{parent})\"><path d=\"{d}\"{rule_attr}/></g></clipPath>\n"
            ));
        } else {
            self.defs.push_str(&format!(
                "<clipPath id=\"{id}\"><path d=\"{d}\"{rule_attr}/></clipPath>\n"
            ));
        }
        id
    }

    fn add_linear_gradient(
        &mut self,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        stops: &[VectorShadingStop],
    ) -> String {
        let id = format!("grad{}", self.gradient_counter);
        self.gradient_counter += 1;
        let stops = svg_gradient_stops(stops);
        self.defs.push_str(&format!(
            "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" \
             x1=\"{x0:.3}\" y1=\"{y0:.3}\" x2=\"{x1:.3}\" y2=\"{y1:.3}\">\
             {stops}\
             </linearGradient>\n"
        ));
        id
    }

    fn add_radial_gradient(
        &mut self,
        center: (f64, f64),
        radius: f64,
        focus: (f64, f64),
        focal_radius: f64,
        stops: &[VectorShadingStop],
        gradient_transform: Option<Transform2D>,
    ) -> String {
        let id = format!("grad{}", self.gradient_counter);
        self.gradient_counter += 1;
        let (cx, cy) = center;
        let (fx, fy) = focus;
        let stops = svg_gradient_stops(stops);
        let transform_attr = gradient_transform
            .map(|transform| {
                let [a, b, c, d, e, f] = transform.to_array();
                format!(" gradientTransform=\"matrix({a:.6} {b:.6} {c:.6} {d:.6} {e:.6} {f:.6})\"")
            })
            .unwrap_or_default();
        let focal_radius_attr = if focal_radius > 1e-9 {
            format!(" fr=\"{focal_radius:.3}\"")
        } else {
            String::new()
        };
        self.defs.push_str(&format!(
            "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" \
             cx=\"{cx:.3}\" cy=\"{cy:.3}\" r=\"{radius:.3}\" fx=\"{fx:.3}\" fy=\"{fy:.3}\"{focal_radius_attr}{transform_attr}>\
             {stops}\
             </radialGradient>\n"
        ));
        id
    }

    fn add_stencil_image_mask(
        &mut self,
        raw: &crate::images::decoder::RawImage,
        transform: [f64; 6],
        paint_ones: bool,
    ) -> Result<String> {
        let mask = stencil_mask_to_svg_mask_rgba(raw, paint_ones)?;
        let png = crate::images::encoder::ImageEncoder::encode_png_fast(&mask).map_err(|err| {
            WellfriendError::UnsupportedFeature(format!(
                "regional SVG stencil mask PNG encode failed: {err}"
            ))
        })?;
        let b64 = base64_encode(&png);
        let id = format!("mask{}", self.mask_counter);
        self.mask_counter += 1;
        let [a, b, c, d, e, f] = transform;
        self.defs.push_str(&format!(
            "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" maskContentUnits=\"userSpaceOnUse\">\
             <image x=\"0\" y=\"0\" width=\"1\" height=\"1\" \
             transform=\"matrix({a:.6} {b:.6} {c:.6} {d:.6} {e:.6} {f:.6})\" \
             xlink:href=\"data:image/png;base64,{b64}\"/></mask>\n"
        ));
        Ok(id)
    }

    fn finish(self) -> String {
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        // Include the xlink namespace when regional images are embedded.
        let xlink_ns = if self.body.contains("xlink:href") || self.defs.contains("xlink:href") {
            " xmlns:xlink=\"http://www.w3.org/1999/xlink\""
        } else {
            ""
        };
        out.push_str(&format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\"{xlink_ns} \
             width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n",
            self.width, self.height, self.width, self.height
        ));
        if !self.defs.is_empty() {
            out.push_str("<defs>\n");
            out.push_str(&self.defs);
            out.push_str("</defs>\n");
        }
        out.push_str(&self.body);
        out.push_str("</svg>\n");
        out
    }
}

/// SVG sibling of `RenderState`: same interpretation, vector emission.
struct SvgRenderState<'a> {
    engine: &'a ContentEngine,
    resources: PageResources,
    viewport: Viewport,
    gs: GraphicsState,
    path: Path,
    pending_clip: Option<FillRule>,
    /// Stack of active clip-path ids saved at each `q` (None = no clip).
    clip_stack: Vec<Option<String>>,
    sink: &'a mut SvgSink,
    /// Names of Image XObjects eligible for regional embedding.
    regional_image_names: Vec<String>,
    /// Remaining inline image sequences eligible for regional embedding.
    regional_inline_image_count: usize,
    /// Names of vector-safe Form XObjects eligible for native replay.
    regional_form_names: Vec<String>,
    /// Names of simple axial/radial shading resources eligible for native
    /// gradient emission.
    regional_shading_names: Vec<String>,
    pending_inline_params: Option<Vec<Operand>>,
    form_depth: usize,
    form_object_stack: Vec<(u32, u16)>,
    text_clip_path: Option<FlatPath>,
    fatal_error: Option<WellfriendError>,
}

impl SvgRenderState<'_> {
    fn run(&mut self, ops: &[ContentOperation]) {
        for op in ops {
            if self.fatal_error.is_some() {
                break;
            }
            self.dispatch(op);
        }
        if self.fatal_error.is_none() {
            self.apply_text_clip();
        }
    }

    fn record_fatal_error(&mut self, err: WellfriendError) {
        self.fatal_error = Some(err);
    }

    fn ctm(&self) -> Transform2D {
        Transform2D::from(self.gs.ctm)
    }

    fn device_point_with(&self, ctm: Transform2D, x: f64, y: f64) -> (f64, f64) {
        let (ux, uy) = ctm.transform_point(x, y);
        self.viewport.page_to_pixel_f64(ux, uy)
    }

    fn current_clip(&self) -> Option<&str> {
        self.clip_stack.last().and_then(|c| c.as_deref())
    }

    fn dispatch(&mut self, op: &ContentOperation) {
        match op.operator.as_str() {
            "m" => {
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    self.path.move_to(x, y);
                }
            }
            "l" => {
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    self.path.line_to(x, y);
                }
            }
            "c" => {
                if let (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) = (
                    op.number(0),
                    op.number(1),
                    op.number(2),
                    op.number(3),
                    op.number(4),
                    op.number(5),
                ) {
                    self.path.curve_to(a, b, c, d, e, f);
                }
            }
            "v" => {
                if let (Some(x2), Some(y2), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    let (cx, cy) = self.path.current_point.unwrap_or((0.0, 0.0));
                    self.path.curve_to(cx, cy, x2, y2, x3, y3);
                }
            }
            "y" => {
                if let (Some(x1), Some(y1), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.path.curve_to(x1, y1, x3, y3, x3, y3);
                }
            }
            "h" => self.path.close(),
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.path.rect(x, y, w, h);
                }
            }
            "S" => self.stroke_and_clear(),
            "s" => {
                self.path.close();
                self.stroke_and_clear();
            }
            "f" | "F" => self.fill_and_clear(FillRule::NonZero),
            "f*" => self.fill_and_clear(FillRule::EvenOdd),
            "B" | "B*" => {
                let rule = if op.operator == "B*" {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                };
                self.fill_path(rule);
                self.stroke_path();
                self.finish_path();
            }
            "b" | "b*" => {
                self.path.close();
                let rule = if op.operator == "b*" {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                };
                self.fill_path(rule);
                self.stroke_path();
                self.finish_path();
            }
            "n" => {
                self.apply_pending_clip();
                self.path.clear();
            }
            "W" => self.pending_clip = Some(FillRule::NonZero),
            "W*" => self.pending_clip = Some(FillRule::EvenOdd),
            "BT" => {
                self.apply_text_clip();
                self.gs.process(op);
            }
            "ET" => {
                self.apply_text_clip();
                self.gs.process(op);
            }
            "q" => {
                self.clip_stack
                    .push(self.current_clip().map(str::to_string));
                self.gs.process(op);
            }
            "Q" => {
                self.gs.process(op);
                self.clip_stack.pop();
            }
            "Tj" => {
                if let Some(bytes) = op.string_bytes(0) {
                    self.show_text(bytes);
                }
            }
            "TJ" => self.show_text_array(op),
            "'" => {
                self.next_text_line();
                if let Some(bytes) = op.string_bytes(0) {
                    self.show_text(bytes);
                }
            }
            "\"" => {
                if let Some(ws) = op.number(0) {
                    self.gs.text.word_spacing = ws;
                }
                if let Some(cs) = op.number(1) {
                    self.gs.text.char_spacing = cs;
                }
                self.next_text_line();
                if let Some(bytes) = op.string_bytes(2) {
                    self.show_text(bytes);
                }
            }
            // Regional image `Do` for eligible Image XObjects.
            "Do" => {
                if let Some(Operand::Name(name)) = op.operands.first() {
                    if self.regional_image_names.contains(name) {
                        if let Err(err) = self.emit_regional_image(name) {
                            self.record_fatal_error(err);
                        }
                    } else if self.regional_form_names.contains(name) {
                        self.emit_form_xobject(name);
                    }
                }
                // Update graphics state (Do is a no-op for gs but we call
                // process for uniformity).
                self.gs.process(op);
            }
            "sh" => {
                if let Some(Operand::Name(name)) = op.operands.first() {
                    if self.regional_shading_names.contains(name) {
                        self.emit_shading(name);
                    }
                }
                self.gs.process(op);
            }
            "gs" => {
                if let Some(Operand::Name(name)) = op.operands.first() {
                    if let Some(dict) = self.resources.ext_g_states.get(name) {
                        let label = format!("ExtGState /{name}");
                        if let Err(err) = self.gs.try_apply_ext_g_state(dict, &label) {
                            self.record_fatal_error(WellfriendError::UnsupportedFeature(err));
                        }
                    }
                }
                self.gs.process(op);
            }
            "ID" => {
                if self.regional_inline_image_count > 0 {
                    self.pending_inline_params = Some(op.operands.clone());
                }
                self.gs.process(op);
            }
            "inline_image_data" => {
                if self.regional_inline_image_count > 0 {
                    if let (Some(params), Some(bytes)) =
                        (self.pending_inline_params.take(), op.string_bytes(0))
                    {
                        if let Err(err) = self.emit_inline_image_region(&params, bytes) {
                            self.record_fatal_error(err);
                        }
                        self.regional_inline_image_count =
                            self.regional_inline_image_count.saturating_sub(1);
                    }
                }
                self.gs.process(op);
            }
            // All state operators handled identically to the raster renderer.
            _ => self.gs.process(op),
        }
    }

    /// Handle a `Do` operation for a regional image embed. Renders the image
    /// XObject through the raster renderer and embeds it as a bounded
    /// `<image>` element at the correct device-space position.
    fn emit_regional_image(&mut self, name: &str) -> Result<()> {
        use crate::images::encoder::ImageEncoder;

        let paint_alpha = self.gs.fill_alpha as f32;
        if paint_alpha <= f32::EPSILON {
            return Ok(());
        }

        let placement = match image_device_placement(&self.gs, &self.viewport) {
            Some(placement) => placement,
            None => {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "regional SVG image /{name} placement is unresolvable"
                )))
            }
        };

        // Resolve the XObject to build an ImageReference for decoding.
        let (obj_num, gen_num) = match self.resources.xobjects.get(name) {
            Some(&(o, g)) => (o, g),
            None => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "regional SVG image /{name} is missing from XObject resources"
                )))
            }
        };

        let reader = self.engine.document().reader();
        let dict = match reader.get_object(obj_num, gen_num) {
            Ok(crate::object::PdfObject::Stream { dict, .. }) => dict,
            Ok(other) => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "regional SVG image /{name} resolved to {}, expected stream",
                    other.variant_name()
                )))
            }
            Err(err) => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "regional SVG image /{name} failed to resolve: {err}"
                )))
            }
        };

        let context = format!("regional SVG image /{name}");
        let is_mask = regional_image_bool(&dict, "ImageMask", "IM", &context)?;
        let _ = regional_image_bool(&dict, "Interpolate", "I", &context)?;
        let image_ref = regional_image_reference(name, obj_num, gen_num, &dict, is_mask)?;
        let color_space_override = (!is_mask)
            .then(|| {
                resolved_regional_image_color_space_override(
                    &dict,
                    &self.resources,
                    self.engine.document().reader(),
                )
            })
            .flatten();

        let raw = match color_space_override.as_ref() {
            Some((color_space_name, color_space_obj)) => {
                ImageDecoder::decode_with_resolved_color_space_and_limits_and_color_transform_options(
                    &image_ref,
                    reader,
                    color_space_name,
                    color_space_obj,
                    &DecodeLimits::default(),
                    crate::render::cmm::ColorTransformOptions::default(),
                )
            }
            None => self.engine.decode_image(&image_ref),
        }
        .map_err(|err| {
            WellfriendError::UnsupportedFeature(format!(
                "regional SVG image /{name} decode failed: {err}"
            ))
        })?;
        let raw = if is_mask {
            if self.emit_pattern_stencil_mask_region(
                &raw,
                placement.transform,
                stencil_mask_paints_ones(&dict)?,
                "image-mask",
            )? {
                return Ok(());
            }
            let Some((_, alpha)) = self.current_fill_color_or_fatal("SVG image-mask fill color")
            else {
                return Ok(());
            };
            let Some(color) =
                self.current_fill_render_color_or_fatal(alpha, "SVG image-mask fill color")
            else {
                return Ok(());
            };
            let color = color.to_pixel_color();
            inline_stencil_mask_to_rgba(&raw, color, stencil_mask_paints_ones(&dict)?)?
        } else {
            raw
        };

        let png = ImageEncoder::encode_png_fast(&raw).map_err(|err| {
            WellfriendError::UnsupportedFeature(format!(
                "regional SVG image /{name} PNG encode failed: {err}"
            ))
        })?;

        let b64 = base64_encode(&png);
        let clip = self.clip_attr();
        let opacity = if is_mask {
            String::new()
        } else {
            opacity_attr("opacity", paint_alpha)
        };
        let bounds = regional_bounds_attr("image-xobject", placement.bounds);
        let blend = self.blend_attr();
        let [a, b, c, d, e, f] = placement.transform;
        self.sink.push_element(&format!(
            "<image x=\"0\" y=\"0\" width=\"1\" height=\"1\" \
             transform=\"matrix({a:.6} {b:.6} {c:.6} {d:.6} {e:.6} {f:.6})\" \
             xlink:href=\"data:image/png;base64,{b64}\"{bounds}{opacity}{blend}{clip}/>"
        ));
        Ok(())
    }

    fn emit_shading(&mut self, name: &str) {
        let alpha = self.gs.fill_alpha as f32;
        if alpha <= f32::EPSILON {
            return;
        }
        let reader = self.engine.document().reader();
        let Some(shading) = load_vector_shading(&self.resources, Some(reader), name) else {
            return;
        };
        let ctm = self.ctm();
        let Some(gradient) = self.add_gradient_for_shading(&shading, ctm) else {
            return;
        };
        let clip = match self.shading_clip_attr(&shading, ctm, "regional SVG shading") {
            Ok(clip) => clip,
            Err(err) => {
                self.record_fatal_error(err);
                return;
            }
        };
        let w = self.viewport.width_px;
        let h = self.viewport.height_px;
        let opacity = opacity_attr("fill-opacity", alpha);
        let blend = self.blend_attr();
        self.sink.push_element(&format!(
            "<rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" fill=\"url(#{gradient})\"{opacity}{blend}{clip}/>"
        ));
    }

    fn add_gradient_for_shading(
        &mut self,
        shading: &VectorShading,
        ctm: Transform2D,
    ) -> Option<String> {
        let gradient = match shading {
            VectorShading::Axial(shading) => {
                let [x0, y0, x1, y1] = shading.coords;
                let (dx0, dy0) = self.device_point_with(ctm, x0, y0);
                let (dx1, dy1) = self.device_point_with(ctm, x1, y1);
                self.sink
                    .add_linear_gradient(dx0, dy0, dx1, dy1, &shading.stops)
            }
            VectorShading::Radial(shading) => {
                let [x0, y0, r0, x1, y1, r1] = shading.coords;
                let full = ctm.concat(&self.viewport.to_transform());
                if !transform_is_finite_and_invertible(full)
                    || r0 < 0.0
                    || r1 < 0.0
                    || (r1 - r0).abs() <= 1e-9
                {
                    return None;
                }
                if transform_preserves_circles(full) {
                    if r1 > r0 {
                        let (fx, fy) = self.device_point_with(ctm, x0, y0);
                        let (cx, cy) = self.device_point_with(ctm, x1, y1);
                        let focal_radius = self.device_radius_with(ctm, r0);
                        let radius = self.device_radius_with(ctm, r1);
                        if focal_radius < 0.0 || radius <= focal_radius {
                            return None;
                        }
                        self.sink.add_radial_gradient(
                            (cx, cy),
                            radius,
                            (fx, fy),
                            focal_radius,
                            &shading.stops,
                            None,
                        )
                    } else {
                        let (fx, fy) = self.device_point_with(ctm, x1, y1);
                        let (cx, cy) = self.device_point_with(ctm, x0, y0);
                        let focal_radius = self.device_radius_with(ctm, r1);
                        let radius = self.device_radius_with(ctm, r0);
                        if focal_radius < 0.0 || radius <= focal_radius {
                            return None;
                        }
                        let stops = reversed_svg_gradient_stops(&shading.stops);
                        self.sink.add_radial_gradient(
                            (cx, cy),
                            radius,
                            (fx, fy),
                            focal_radius,
                            &stops,
                            None,
                        )
                    }
                } else if r1 > r0 {
                    self.sink.add_radial_gradient(
                        (x1, y1),
                        r1,
                        (x0, y0),
                        r0,
                        &shading.stops,
                        Some(full),
                    )
                } else {
                    let stops = reversed_svg_gradient_stops(&shading.stops);
                    self.sink
                        .add_radial_gradient((x0, y0), r0, (x1, y1), r1, &stops, Some(full))
                }
            }
        };
        Some(gradient)
    }

    fn emit_pattern_stencil_mask_region(
        &mut self,
        raw: &crate::images::decoder::RawImage,
        transform: [f64; 6],
        paint_ones: bool,
        stage: &str,
    ) -> Result<bool> {
        let Some(pattern_name) = self.gs.fill_pattern_name.clone() else {
            return Ok(false);
        };
        let alpha = self.gs.fill_alpha as f32;
        if alpha <= f32::EPSILON {
            return Ok(true);
        }
        if self.emit_tiling_pattern_stencil_mask_region(
            &pattern_name,
            raw,
            transform,
            paint_ones,
            alpha,
            stage,
        )? {
            return Ok(true);
        }
        let reader = self.engine.document().reader();
        let pattern = load_vector_shading_pattern(&self.resources, Some(reader), &pattern_name)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "regional SVG pattern-painted stencil {stage} /{pattern_name} is not vector-safe"
                ))
            })?;
        let pattern_ctm = Transform2D::from(pattern.matrix).concat(&self.ctm());
        let gradient = self
            .add_gradient_for_shading(&pattern.shading, pattern_ctm)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "regional SVG pattern-painted stencil {stage} gradient is unrepresentable"
                ))
            })?;
        let mask_id = self
            .sink
            .add_stencil_image_mask(raw, transform, paint_ones)?;
        let clip = self.shading_clip_attr(
            &pattern.shading,
            pattern_ctm,
            "regional SVG pattern-painted stencil shading",
        )?;
        let opacity = opacity_attr("fill-opacity", alpha);
        let blend = self.blend_attr();
        let w = self.viewport.width_px;
        let h = self.viewport.height_px;
        self.sink.push_element(&format!(
            "<rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" \
             fill=\"url(#{gradient})\" mask=\"url(#{mask_id})\"{opacity}{blend}{clip}/>"
        ));
        Ok(true)
    }

    fn emit_tiling_pattern_stencil_mask_region(
        &mut self,
        pattern_name: &str,
        raw: &crate::images::decoder::RawImage,
        transform: [f64; 6],
        paint_ones: bool,
        alpha: f32,
        stage: &str,
    ) -> Result<bool> {
        let reader = self.engine.document().reader();
        let Some(program) = load_vector_tiling_pattern(&self.resources, reader, pattern_name)
        else {
            return Ok(false);
        };
        ensure_regional_stencil_mask(raw, "regional SVG tiling-pattern stencil mask")?;
        let forced_color = match program.paint_type {
            VectorTilingPatternPaintType::Colored => None,
            VectorTilingPatternPaintType::Uncolored => {
                Some(vector_uncolored_tiling_paint_color(&self.gs.fill_color).ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(format!(
                        "regional SVG uncolored tiling-pattern stencil {stage} /{pattern_name} requires finite gray, RGB, or CMYK caller color components"
                    ))
                })?)
            }
        };

        let mut path = Path::new();
        path.rect(0.0, 0.0, 1.0, 1.0);
        let flat = flatten_path_device_transform(
            &path,
            &Transform2D::from_array(transform),
            self.gs.path_flatness_tolerance(),
        );
        if flat.subpaths.iter().all(|s| s.is_empty()) {
            return Ok(true);
        }

        let pattern_ctm = Transform2D::from(program.matrix).concat(&self.ctm());
        let Some((i0, i1, j0, j1, tile_count)) =
            tiling_pattern_tile_range(&flat, &program, pattern_ctm, &self.viewport)
        else {
            return Ok(true);
        };
        if tile_count > MAX_VECTOR_TILING_PATTERN_CELLS {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "regional SVG tiling-pattern stencil {stage} /{pattern_name} requires {tile_count} visible cells, cap is {MAX_VECTOR_TILING_PATTERN_CELLS}"
            )));
        }

        let mask_id = self
            .sink
            .add_stencil_image_mask(raw, transform, paint_ones)?;
        let clip = self.clip_attr();
        let opacity = opacity_attr("opacity", alpha);
        let blend = self.blend_attr();
        self.sink.push_element(&format!(
            "<g mask=\"url(#{mask_id})\"{opacity}{blend}{clip}>"
        ));
        for j in j0..=j1 {
            for i in i0..=i1 {
                if self.fatal_error.is_some() {
                    break;
                }
                let translate =
                    Transform2D::translation(i as f64 * program.x_step, j as f64 * program.y_step);
                let tile_ctm = translate.concat(&pattern_ctm);
                self.emit_tiling_pattern_tile(&program, tile_ctm, forced_color.as_ref());
            }
        }
        self.sink.push_element("</g>");
        Ok(true)
    }

    fn shading_clip_attr(
        &mut self,
        shading: &VectorShading,
        ctm: Transform2D,
        context: &str,
    ) -> Result<String> {
        Ok(match self.shading_clip_id(shading, ctm, context)? {
            Some(clip_id) => format!(" clip-path=\"url(#{clip_id})\""),
            None => String::new(),
        })
    }

    fn shading_clip_id(
        &mut self,
        shading: &VectorShading,
        ctm: Transform2D,
        context: &str,
    ) -> Result<Option<String>> {
        let mut clip_id = self.current_clip().map(str::to_string);
        if let Some(bbox) = shading.bbox() {
            let flat = self.shading_bbox_flat_path(bbox, ctm).ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "{context} /BBox could not be flattened"
                ))
            })?;
            clip_id = Some(
                self.sink
                    .add_clip(&flat, FillRule::NonZero, clip_id.as_deref()),
            );
        }
        if let Some((flat, rule)) =
            self.shading_extend_flat_path(shading, ctm).ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "{context} /Extend clip could not be flattened"
                ))
            })?
        {
            clip_id = Some(self.sink.add_clip(&flat, rule, clip_id.as_deref()));
        }
        Ok(clip_id)
    }

    fn shading_extend_flat_path(
        &self,
        shading: &VectorShading,
        ctm: Transform2D,
    ) -> Option<Option<(FlatPath, FillRule)>> {
        match shading {
            VectorShading::Axial(shading) => self
                .axial_extend_clip_flat_path(shading.coords, shading.extend, ctm)
                .map(|clip| clip.map(|flat| (flat, FillRule::NonZero))),
            VectorShading::Radial(shading) if shading.extend == [true, true] => Some(None),
            VectorShading::Radial(shading) => {
                self.radial_extend_clip_flat_path(shading.coords, shading.extend, ctm)
            }
        }
    }

    fn axial_extend_clip_flat_path(
        &self,
        coords: [f64; 4],
        extend: [bool; 2],
        ctm: Transform2D,
    ) -> Option<Option<FlatPath>> {
        if extend == [true, true] {
            return Some(None);
        }
        let (x0, y0) = self.device_point_with(ctm, coords[0], coords[1]);
        let (x1, y1) = self.device_point_with(ctm, coords[2], coords[3]);
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt();
        if ![x0, y0, x1, y1, dx, dy, len]
            .iter()
            .all(|value| value.is_finite())
            || len <= 1e-9
        {
            return None;
        }
        let ux = dx / len;
        let uy = dy / len;
        let px = -uy;
        let py = ux;
        let width = self.viewport.width_px as f64;
        let height = self.viewport.height_px as f64;
        let max_abs = [x0.abs(), y0.abs(), x1.abs(), y1.abs(), width, height]
            .into_iter()
            .fold(0.0_f64, f64::max);
        let span = max_abs + width.hypot(height) + len + 1024.0;
        if !span.is_finite() {
            return None;
        }
        let start = if extend[0] {
            (x0 - ux * span, y0 - uy * span)
        } else {
            (x0, y0)
        };
        let end = if extend[1] {
            (x1 + ux * span, y1 + uy * span)
        } else {
            (x1, y1)
        };
        let subpath = vec![
            (start.0 + px * span, start.1 + py * span),
            (end.0 + px * span, end.1 + py * span),
            (end.0 - px * span, end.1 - py * span),
            (start.0 - px * span, start.1 - py * span),
        ];
        subpath
            .iter()
            .all(|(x, y)| x.is_finite() && y.is_finite())
            .then_some(Some(FlatPath {
                subpaths: vec![subpath],
                closed: vec![true],
            }))
    }

    fn radial_extend_clip_flat_path(
        &self,
        coords: [f64; 6],
        extend: [bool; 2],
        ctm: Transform2D,
    ) -> Option<Option<(FlatPath, FillRule)>> {
        if extend == [true, true] {
            return Some(None);
        }
        let [x0, y0, r0, x1, y1, r1] = coords;
        let full = ctm.concat(&self.viewport.to_transform());
        if !transform_is_finite_and_invertible(full)
            || r0 < 0.0
            || r1 <= r0
            || (x1 - x0).abs() > 1e-9
            || (y1 - y0).abs() > 1e-9
        {
            return None;
        }

        let outer = self.radial_circle_subpath_device(ctm, x1, y1, r1)?;
        let mut subpaths = Vec::new();
        let mut closed = Vec::new();
        let mut rule = FillRule::NonZero;

        match extend {
            [true, false] => {
                subpaths.push(outer);
                closed.push(true);
            }
            [false, false] => {
                subpaths.push(outer);
                closed.push(true);
                if r0 > 1e-9 {
                    subpaths.push(self.radial_circle_subpath_device(ctm, x0, y0, r0)?);
                    closed.push(true);
                    rule = FillRule::EvenOdd;
                }
            }
            [false, true] => {
                if r0 <= 1e-9 {
                    return Some(None);
                }
                subpaths.push(vec![
                    (0.0, 0.0),
                    (self.viewport.width_px as f64, 0.0),
                    (
                        self.viewport.width_px as f64,
                        self.viewport.height_px as f64,
                    ),
                    (0.0, self.viewport.height_px as f64),
                ]);
                closed.push(true);
                subpaths.push(self.radial_circle_subpath_device(ctm, x0, y0, r0)?);
                closed.push(true);
                rule = FillRule::EvenOdd;
            }
            [true, true] => return Some(None),
        }

        subpaths
            .iter()
            .flatten()
            .all(|(x, y)| x.is_finite() && y.is_finite())
            .then_some(Some((FlatPath { subpaths, closed }, rule)))
    }

    fn radial_circle_subpath_device(
        &self,
        ctm: Transform2D,
        cx: f64,
        cy: f64,
        radius: f64,
    ) -> Option<Vec<(f64, f64)>> {
        if radius <= 0.0 || ![cx, cy, radius].iter().all(|value| value.is_finite()) {
            return None;
        }
        let segments = 96usize;
        let mut subpath = Vec::with_capacity(segments + 1);
        for index in 0..segments {
            let theta = std::f64::consts::TAU * index as f64 / segments as f64;
            subpath.push(self.device_point_with(
                ctm,
                cx + radius * theta.cos(),
                cy + radius * theta.sin(),
            ));
        }
        subpath.push(subpath[0]);
        Some(subpath)
    }

    fn shading_bbox_flat_path(&self, bbox: [f64; 4], ctm: Transform2D) -> Option<FlatPath> {
        let x = bbox[0].min(bbox[2]);
        let y = bbox[1].min(bbox[3]);
        let w = (bbox[2] - bbox[0]).abs();
        let h = (bbox[3] - bbox[1]).abs();
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let mut path = Path::new();
        path.rect(x, y, w, h);
        let flat = flatten_path(
            &path,
            &ctm,
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        if flat.subpaths.iter().all(|s| s.is_empty()) {
            return None;
        }
        Some(flat)
    }

    fn device_radius_with(&self, ctm: Transform2D, radius: f64) -> f64 {
        let (x0, y0) = self.device_point_with(ctm, 0.0, 0.0);
        let (x1, y1) = self.device_point_with(ctm, radius, 0.0);
        ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt()
    }

    /// Handle a complete `BI`/`ID`/`EI` inline image sequence approved by the
    /// shared regional fallback classifier.
    fn emit_inline_image_region(&mut self, params: &[Operand], data: &[u8]) -> Result<()> {
        use crate::images::encoder::ImageEncoder;

        let paint_alpha = self.gs.fill_alpha as f32;
        if paint_alpha <= f32::EPSILON {
            return Ok(());
        }

        let region = decode_inline_image_region(
            params,
            data,
            &self.gs,
            &self.viewport,
            &self.resources,
            Some(self.engine.document().reader()),
        )
        .map_err(|err| {
            WellfriendError::UnsupportedFeature(format!(
                "regional SVG inline image decode failed: {err}"
            ))
        })?;
        let raw = if region.is_mask {
            if self.emit_pattern_stencil_mask_region(
                &region.raw,
                region.placement.transform,
                region.mask_paints_ones,
                "inline-image-mask",
            )? {
                return Ok(());
            }
            let Some((_, alpha)) =
                self.current_fill_color_or_fatal("SVG inline-image-mask fill color")
            else {
                return Ok(());
            };
            let Some(color) =
                self.current_fill_render_color_or_fatal(alpha, "SVG inline-image-mask fill color")
            else {
                return Ok(());
            };
            let color = color.to_pixel_color();
            inline_stencil_mask_to_rgba(&region.raw, color, region.mask_paints_ones)?
        } else {
            region.raw
        };
        let png = ImageEncoder::encode_png_fast(&raw).map_err(|err| {
            WellfriendError::UnsupportedFeature(format!(
                "regional SVG inline image PNG encode failed: {err}"
            ))
        })?;
        let b64 = base64_encode(&png);
        let clip = self.clip_attr();
        let opacity = if region.is_mask {
            String::new()
        } else {
            opacity_attr("opacity", paint_alpha)
        };
        let bounds = regional_bounds_attr("inline-image", region.placement.bounds);
        let blend = self.blend_attr();
        let [a, b, c, d, e, f] = region.placement.transform;
        self.sink.push_element(&format!(
            "<image x=\"0\" y=\"0\" width=\"1\" height=\"1\" \
             transform=\"matrix({a:.6} {b:.6} {c:.6} {d:.6} {e:.6} {f:.6})\" \
             xlink:href=\"data:image/png;base64,{b64}\"{bounds}{opacity}{blend}{clip}/>"
        ));
        Ok(())
    }

    /// Replay a vector-safe Form XObject natively inside the SVG sink. The
    /// shared classifier has already proven the Form does not require a
    /// whole-page fallback; this method rechecks the scoped program, applies
    /// the Form matrix and BBox, merges resources, and restores every caller
    /// scope afterward.
    fn emit_form_xobject(&mut self, name: &str) {
        if self.form_depth >= MAX_VECTOR_FORM_DEPTH {
            return;
        }
        let reader = self.engine.document().reader();
        let Some(program) = load_vector_form_program(&self.resources, reader, name, &self.gs)
        else {
            return;
        };
        let form_key = (program.object_number, program.generation_number);
        if self.form_object_stack.contains(&form_key) {
            return;
        }

        let saved_gs = self.gs.clone();
        let saved_resources = self.resources.clone();
        let saved_path = self.path.clone();
        let saved_pending_clip = self.pending_clip;
        let saved_text_clip_path = self.text_clip_path.clone();
        let saved_image_names = self.regional_image_names.clone();
        let saved_inline_count = self.regional_inline_image_count;
        let saved_form_names = self.regional_form_names.clone();
        let saved_shading_names = self.regional_shading_names.clone();
        let saved_pending_inline = self.pending_inline_params.take();
        let saved_clip_len = self.clip_stack.len();

        let form_resources = merged_vector_resources(program.resources.as_ref(), &self.resources);
        self.resources = form_resources;
        let form_t = Transform2D::from(program.form_matrix);
        let current_t = Transform2D::from(saved_gs.ctm);
        self.gs.ctm = form_t.concat(&current_t).to_array();

        self.form_depth += 1;
        self.form_object_stack.push(form_key);

        let decision = classify_scoped_svg_vector_output(
            &program.ops,
            &self.resources,
            self.viewport.scale,
            reader,
            self.gs.clone(),
            &mut self.form_object_stack,
        );

        if let Some(bbox) = program.bbox {
            self.push_form_bbox_clip(bbox);
        }

        match decision {
            VectorFallbackDecision::WholePageRaster { .. } => {}
            VectorFallbackDecision::PureVector => {
                self.regional_image_names.clear();
                self.regional_inline_image_count = 0;
                self.regional_form_names.clear();
                self.regional_shading_names.clear();
                self.run(&program.ops);
            }
            VectorFallbackDecision::RegionalImageFallback {
                image_names,
                inline_image_count,
                form_names,
                shading_names,
            } => {
                self.regional_image_names = image_names;
                self.regional_inline_image_count = inline_image_count;
                self.regional_form_names = form_names;
                self.regional_shading_names = shading_names;
                self.run(&program.ops);
            }
        }

        self.form_object_stack.pop();
        self.form_depth = self.form_depth.saturating_sub(1);
        self.clip_stack.truncate(saved_clip_len);
        self.gs = saved_gs;
        self.resources = saved_resources;
        self.path = saved_path;
        self.pending_clip = saved_pending_clip;
        self.text_clip_path = saved_text_clip_path;
        self.regional_image_names = saved_image_names;
        self.regional_inline_image_count = saved_inline_count;
        self.regional_form_names = saved_form_names;
        self.regional_shading_names = saved_shading_names;
        self.pending_inline_params = saved_pending_inline;
    }

    fn push_form_bbox_clip(&mut self, bbox: [f64; 4]) {
        let x = bbox[0].min(bbox[2]);
        let y = bbox[1].min(bbox[3]);
        let w = (bbox[2] - bbox[0]).abs();
        let h = (bbox[3] - bbox[1]).abs();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let mut path = Path::new();
        path.rect(x, y, w, h);
        let flat = flatten_path(
            &path,
            &self.ctm(),
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        let parent = self.current_clip().map(str::to_string);
        let id = self
            .sink
            .add_clip(&flat, FillRule::NonZero, parent.as_deref());
        self.clip_stack.push(Some(id));
    }

    fn apply_pending_clip(&mut self) {
        if let Some(rule) = self.pending_clip.take() {
            if self.path.is_empty() {
                return;
            }
            let ctm = self.ctm();
            let flat = flatten_path(
                &self.path,
                &ctm,
                &self.viewport,
                self.gs.path_flatness_tolerance(),
            );
            let parent = self.current_clip().map(str::to_string);
            let id = self.sink.add_clip(&flat, rule, parent.as_deref());
            // PDF clips intersect. The new SVG clipPath references the previous
            // active clip when one exists, so replacing the stack top preserves
            // the composed clip identity for subsequent elements.
            if let Some(top) = self.clip_stack.last_mut() {
                *top = Some(id);
            } else {
                self.clip_stack.push(Some(id));
            }
        }
    }

    fn append_text_clip_path(&mut self, flat: &FlatPath) {
        if !matches!(self.gs.text.rendering_mode, 4..=7) {
            return;
        }
        let clip = self.text_clip_path.get_or_insert_with(FlatPath::default);
        for (subpath, closed) in flat.subpaths.iter().zip(flat.closed.iter()) {
            if subpath.is_empty() {
                continue;
            }
            clip.subpaths.push(subpath.clone());
            clip.closed.push(*closed);
        }
    }

    fn apply_text_clip(&mut self) {
        let Some(flat) = self.text_clip_path.take() else {
            return;
        };
        if flat.subpaths.iter().all(|s| s.is_empty()) {
            return;
        }
        let parent = self.current_clip().map(str::to_string);
        let id = self
            .sink
            .add_clip(&flat, FillRule::NonZero, parent.as_deref());
        if let Some(top) = self.clip_stack.last_mut() {
            *top = Some(id);
        } else {
            self.clip_stack.push(Some(id));
        }
    }

    fn stroke_and_clear(&mut self) {
        self.apply_pending_clip();
        self.stroke_path();
        self.finish_path();
    }

    fn fill_and_clear(&mut self, rule: FillRule) {
        self.apply_pending_clip();
        self.fill_path(rule);
        self.finish_path();
    }

    fn finish_path(&mut self) {
        self.path.clear();
    }

    fn fill_path(&mut self, rule: FillRule) {
        if self.path.is_empty() {
            return;
        }
        let ctm = self.ctm();
        let flat = flatten_path(
            &self.path,
            &ctm,
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        let d = path_data(&flat);
        if d.is_empty() {
            return;
        }
        if self.gs.fill_color_space.is_pattern() {
            match self.emit_tiling_pattern_clip(
                self.gs.fill_pattern_name.clone(),
                &flat,
                rule,
                self.gs.fill_alpha as f32,
                false,
                "fill",
            ) {
                Ok(true) => return,
                Ok(false) => {}
                Err(err) => {
                    self.record_fatal_error(err);
                    return;
                }
            }
            self.emit_shading_pattern_clip(
                self.gs.fill_pattern_name.clone(),
                &flat,
                rule,
                self.gs.fill_alpha as f32,
                "fill",
            );
            return;
        }
        let Some((rgb, alpha)) = self.current_fill_color_or_fatal("SVG fill color") else {
            return;
        };
        let rule_attr = match rule {
            FillRule::EvenOdd => " fill-rule=\"evenodd\"",
            FillRule::NonZero => "",
        };
        let clip = self.clip_attr();
        let opacity = opacity_attr("fill-opacity", alpha);
        let blend = self.blend_attr();
        self.sink.push_element(&format!(
            "<path d=\"{d}\" fill=\"{rgb}\"{rule_attr}{opacity}{blend}{clip}/>"
        ));
    }

    fn stroke_path(&mut self) {
        if self.path.is_empty() {
            return;
        }
        let ctm = self.ctm();
        let flat = flatten_path(
            &self.path,
            &ctm,
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        let d = path_data(&flat);
        if d.is_empty() {
            return;
        }
        if self.gs.stroke_color_space.is_pattern() {
            let outline = stroke_flat_path(
                &flat,
                self.device_line_width(),
                &self.device_dash_state(),
                self.gs.line_cap.clone(),
                self.gs.line_join.clone(),
                self.gs.miter_limit,
            );
            match self.emit_tiling_pattern_clip(
                self.gs.stroke_pattern_name.clone(),
                &outline,
                FillRule::NonZero,
                self.gs.stroke_alpha as f32,
                true,
                "stroke",
            ) {
                Ok(true) => return,
                Ok(false) => {}
                Err(err) => {
                    self.record_fatal_error(err);
                    return;
                }
            }
            self.emit_shading_pattern_clip(
                self.gs.stroke_pattern_name.clone(),
                &outline,
                FillRule::NonZero,
                self.gs.stroke_alpha as f32,
                "stroke",
            );
            return;
        }
        let Some((rgb, alpha)) = self.current_stroke_color_or_fatal("SVG stroke color") else {
            return;
        };
        // Stroke width: PDF line width is in user space; scale by the CTM and
        // the viewport scale to device pixels. A 0-width line is a 1px hairline.
        let width = self.device_line_width();
        let clip = self.clip_attr();
        let opacity = opacity_attr("stroke-opacity", alpha);
        let dash = self.dash_attr();
        let line_style = self.line_style_attrs();
        let blend = self.blend_attr();
        self.sink.push_element(&format!(
            "<path d=\"{d}\" fill=\"none\" stroke=\"{rgb}\" stroke-width=\"{width:.3}\"{opacity}{dash}{line_style}{blend}{clip}/>"
        ));
    }

    fn emit_tiling_pattern_clip(
        &mut self,
        pattern_name: Option<String>,
        flat: &FlatPath,
        rule: FillRule,
        alpha: f32,
        use_stroke_color: bool,
        stage: &str,
    ) -> Result<bool> {
        let Some(pattern_name) = pattern_name else {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "regional SVG tiling pattern {stage} requires an active pattern name"
            )));
        };
        let reader = self.engine.document().reader();
        let Some(program) = load_vector_tiling_pattern(&self.resources, reader, &pattern_name)
        else {
            return Ok(false);
        };
        if alpha <= f32::EPSILON {
            return Ok(true);
        }
        let forced_color = match program.paint_type {
            VectorTilingPatternPaintType::Colored => None,
            VectorTilingPatternPaintType::Uncolored => {
                let paint_color = if use_stroke_color {
                    &self.gs.stroke_color
                } else {
                    &self.gs.fill_color
                };
                Some(vector_uncolored_tiling_paint_color(paint_color).ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(format!(
                        "regional SVG uncolored tiling pattern {stage} /{pattern_name} requires finite gray, RGB, or CMYK caller color components"
                    ))
                })?)
            }
        };
        let pattern_ctm = Transform2D::from(program.matrix).concat(&self.ctm());
        let Some((i0, i1, j0, j1, tile_count)) =
            tiling_pattern_tile_range(flat, &program, pattern_ctm, &self.viewport)
        else {
            return Ok(true);
        };
        if tile_count > MAX_VECTOR_TILING_PATTERN_CELLS {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "regional SVG tiling pattern {stage} /{pattern_name} requires {tile_count} visible cells, cap is {MAX_VECTOR_TILING_PATTERN_CELLS}"
            )));
        }

        let parent = self.current_clip().map(str::to_string);
        let path_clip_id = self.sink.add_clip(flat, rule, parent.as_deref());
        let saved_clip_len = self.clip_stack.len();
        self.clip_stack.push(Some(path_clip_id));
        let opacity = opacity_attr("opacity", alpha);
        let blend = self.blend_attr();
        self.sink.push_element(&format!("<g{opacity}{blend}>"));
        for j in j0..=j1 {
            for i in i0..=i1 {
                if self.fatal_error.is_some() {
                    break;
                }
                let translate =
                    Transform2D::translation(i as f64 * program.x_step, j as f64 * program.y_step);
                let tile_ctm = translate.concat(&pattern_ctm);
                self.emit_tiling_pattern_tile(&program, tile_ctm, forced_color.as_ref());
            }
        }
        self.sink.push_element("</g>");
        self.clip_stack.truncate(saved_clip_len);
        Ok(true)
    }

    fn emit_pattern_clip(
        &mut self,
        pattern_name: Option<String>,
        flat: &FlatPath,
        rule: FillRule,
        alpha: f32,
        use_stroke_color: bool,
        stage: &str,
    ) {
        match self.emit_tiling_pattern_clip(
            pattern_name.clone(),
            flat,
            rule,
            alpha,
            use_stroke_color,
            stage,
        ) {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => {
                self.record_fatal_error(err);
                return;
            }
        }
        self.emit_shading_pattern_clip(pattern_name, flat, rule, alpha, stage);
    }

    fn emit_tiling_pattern_tile(
        &mut self,
        program: &VectorTilingPatternProgram,
        tile_ctm: Transform2D,
        forced_color: Option<&Color>,
    ) {
        let saved_gs = self.gs.clone();
        let saved_resources = self.resources.clone();
        let saved_image_names = self.regional_image_names.clone();
        let saved_inline_count = self.regional_inline_image_count;
        let saved_form_names = self.regional_form_names.clone();
        let saved_shading_names = self.regional_shading_names.clone();
        let saved_pending_inline = self.pending_inline_params.take();
        let saved_clip_len = self.clip_stack.len();

        self.resources = program.resources.clone();
        self.gs = GraphicsState::default();
        self.gs.ctm = tile_ctm.to_array();
        if let Some(color) = forced_color {
            self.gs.fill_color_space = color.space.clone();
            self.gs.stroke_color_space = color.space.clone();
            self.gs.fill_color = color.clone();
            self.gs.stroke_color = color.clone();
        }
        self.regional_image_names.clear();
        self.regional_inline_image_count = 0;
        self.regional_form_names.clear();
        self.regional_shading_names.clear();
        let reader = self.engine.document().reader();
        let decision = classify_scoped_svg_vector_output(
            &program.ops,
            &self.resources,
            self.viewport.scale,
            reader,
            self.gs.clone(),
            &mut self.form_object_stack,
        );
        let run_tile = match decision {
            VectorFallbackDecision::WholePageRaster { .. } => {
                self.record_fatal_error(WellfriendError::UnsupportedFeature(
                    "regional SVG tiling pattern tile contains unsupported vector content"
                        .to_string(),
                ));
                false
            }
            VectorFallbackDecision::PureVector => true,
            VectorFallbackDecision::RegionalImageFallback {
                image_names,
                inline_image_count,
                form_names,
                shading_names,
            } => {
                self.regional_image_names = image_names;
                self.regional_inline_image_count = inline_image_count;
                self.regional_form_names = form_names;
                self.regional_shading_names = shading_names;
                true
            }
        };
        if run_tile {
            self.push_tiling_bbox_clip(program.bbox, tile_ctm);
            self.run(&program.ops);
        }

        self.clip_stack.truncate(saved_clip_len);
        self.gs = saved_gs;
        self.resources = saved_resources;
        self.regional_image_names = saved_image_names;
        self.regional_inline_image_count = saved_inline_count;
        self.regional_form_names = saved_form_names;
        self.regional_shading_names = saved_shading_names;
        self.pending_inline_params = saved_pending_inline;
    }

    fn push_tiling_bbox_clip(&mut self, bbox: [f64; 4], tile_ctm: Transform2D) {
        let x = bbox[0].min(bbox[2]);
        let y = bbox[1].min(bbox[3]);
        let w = (bbox[2] - bbox[0]).abs();
        let h = (bbox[3] - bbox[1]).abs();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let mut path = Path::new();
        path.rect(x, y, w, h);
        let flat = flatten_path(
            &path,
            &tile_ctm,
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        let parent = self.current_clip().map(str::to_string);
        let id = self
            .sink
            .add_clip(&flat, FillRule::NonZero, parent.as_deref());
        self.clip_stack.push(Some(id));
    }

    fn emit_shading_pattern_clip(
        &mut self,
        pattern_name: Option<String>,
        flat: &FlatPath,
        rule: FillRule,
        alpha: f32,
        stage: &str,
    ) {
        if flat.subpaths.iter().all(|s| s.is_empty()) {
            return;
        }
        let Some(pattern_name) = pattern_name else {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                "regional SVG shading pattern {stage} requires an active pattern name"
            )));
            return;
        };
        let reader = self.engine.document().reader();
        let Some(pattern) =
            load_vector_shading_pattern(&self.resources, Some(reader), &pattern_name)
        else {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                "regional SVG shading pattern /{pattern_name} is not vector-safe"
            )));
            return;
        };
        let pattern_ctm = Transform2D::from(pattern.matrix).concat(&self.ctm());
        let Some(gradient) = self.add_gradient_for_shading(&pattern.shading, pattern_ctm) else {
            return;
        };
        let clip_parent = match self.shading_clip_id(
            &pattern.shading,
            pattern_ctm,
            "regional SVG shading pattern",
        ) {
            Ok(clip_parent) => clip_parent,
            Err(err) => {
                self.record_fatal_error(err);
                return;
            }
        };
        let clip_id = self.sink.add_clip(flat, rule, clip_parent.as_deref());
        let w = self.viewport.width_px;
        let h = self.viewport.height_px;
        let opacity = opacity_attr("fill-opacity", alpha);
        let blend = self.blend_attr();
        self.sink.push_element(&format!(
            "<rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" fill=\"url(#{gradient})\" clip-path=\"url(#{clip_id})\"{opacity}{blend}/>"
        ));
    }

    /// Approximate the device-space stroke width from the user-space line width
    /// and the current transform's average scale.
    fn device_line_width(&self) -> f64 {
        let ctm = self.ctm();
        // Average scale of the CTM (geometric mean of the two axis lengths).
        let sx = (ctm.a * ctm.a + ctm.b * ctm.b).sqrt();
        let sy = (ctm.c * ctm.c + ctm.d * ctm.d).sqrt();
        let ctm_scale = ((sx * sy).abs()).sqrt().max(1e-6);
        let vp_scale = self.viewport.scale;
        let w = self.gs.line_width * ctm_scale * vp_scale;
        if w <= 0.0 {
            1.0
        } else {
            w
        }
    }

    fn dash_attr(&self) -> String {
        if self.gs.dash.pattern.is_empty() {
            return String::new();
        }
        let scale = self.device_dash_scale();
        let dashes: Vec<String> = self
            .gs
            .dash
            .pattern
            .iter()
            .map(|d| format!("{:.3}", d * scale))
            .collect();
        let phase = self.gs.dash.phase * scale;
        format!(
            " stroke-dasharray=\"{}\" stroke-dashoffset=\"{phase:.3}\"",
            dashes.join(",")
        )
    }

    fn device_dash_state(&self) -> DashState {
        let scale = self.device_dash_scale();
        DashState::new(
            self.gs.dash.pattern.iter().map(|d| d * scale).collect(),
            self.gs.dash.phase * scale,
        )
    }

    fn device_dash_scale(&self) -> f64 {
        self.viewport.scale * {
            let ctm = self.ctm();
            ((ctm.a * ctm.a + ctm.b * ctm.b).sqrt()).max(1e-6)
        }
    }

    fn line_style_attrs(&self) -> String {
        let cap = match self.gs.line_cap {
            LineCap::Butt => "butt",
            LineCap::Round => "round",
            LineCap::ProjectingSquare => "square",
        };
        let join = match self.gs.line_join {
            LineJoin::Miter => "miter",
            LineJoin::Round => "round",
            LineJoin::Bevel => "bevel",
        };
        let miter = self.gs.miter_limit.max(1.0);
        format!(
            " stroke-linecap=\"{cap}\" stroke-linejoin=\"{join}\" stroke-miterlimit=\"{miter:.3}\""
        )
    }

    fn clip_attr(&self) -> String {
        match self.current_clip() {
            Some(id) => format!(" clip-path=\"url(#{id})\""),
            None => String::new(),
        }
    }

    fn blend_attr(&self) -> String {
        let Some(mode) = svg_blend_mode_name(self.gs.blend_mode) else {
            return String::new();
        };
        format!(" style=\"mix-blend-mode:{mode}\"")
    }

    /// Resolve a graphics-state colour to (`#rrggbb`, alpha). Pattern/unknown
    /// spaces are not reached here (those pages take the raster fallback).
    fn resolve_color(&self, color: &Color, alpha: f32, role: &str) -> Result<(String, f32)> {
        if let ColorSpace::Named(name) = &color.space {
            if let Some(space_obj) = self.resources.color_spaces.get(name) {
                let reader = self.engine.document().reader();
                let color_options = crate::render::cmm::ColorTransformOptions {
                    intent: crate::render::cmm::ColorIntent::from_pdf_name(
                        &self.gs.rendering_intent,
                    ),
                    black_point_compensation: false,
                    ..crate::render::cmm::ColorTransformOptions::default()
                };
                match crate::render::colorspace::resolve_named_color_with_options(
                    space_obj,
                    &color.components,
                    alpha,
                    reader,
                    color_options,
                ) {
                    crate::render::colorspace::NamedColor::Color(rc) => {
                        return Ok((rgb_hex(&rc), rc.a));
                    }
                    crate::render::colorspace::NamedColor::NoPaint => {
                        let transparent = RenderColor::transparent();
                        return Ok((rgb_hex(&transparent), transparent.a));
                    }
                    crate::render::colorspace::NamedColor::Invalid(reason) => {
                        return Err(WellfriendError::UnsupportedFeature(format!(
                            "{role} color space /{name} rejected: {reason}"
                        )));
                    }
                    crate::render::colorspace::NamedColor::Unhandled => {
                        return Err(WellfriendError::UnsupportedFeature(format!(
                            "{role} color space /{name} is unsupported for SVG vector output"
                        )));
                    }
                }
            } else {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "{role} color space /{name} resource is missing"
                )));
            }
        }
        let rc = ColorSpaceHandler::strict_to_render_color(color, alpha).map_err(|reason| {
            WellfriendError::UnsupportedFeature(format!("{role} color rejected: {reason}"))
        })?;
        Ok((rgb_hex(&rc), rc.a))
    }

    fn resolve_render_color(&self, color: &Color, alpha: f32, role: &str) -> Result<RenderColor> {
        if let ColorSpace::Named(name) = &color.space {
            if let Some(space_obj) = self.resources.color_spaces.get(name) {
                let reader = self.engine.document().reader();
                let color_options = crate::render::cmm::ColorTransformOptions {
                    intent: crate::render::cmm::ColorIntent::from_pdf_name(
                        &self.gs.rendering_intent,
                    ),
                    black_point_compensation: false,
                    ..crate::render::cmm::ColorTransformOptions::default()
                };
                match crate::render::colorspace::resolve_named_color_with_options(
                    space_obj,
                    &color.components,
                    alpha,
                    reader,
                    color_options,
                ) {
                    crate::render::colorspace::NamedColor::Color(rc) => return Ok(rc),
                    crate::render::colorspace::NamedColor::NoPaint => {
                        return Ok(RenderColor::transparent());
                    }
                    crate::render::colorspace::NamedColor::Invalid(reason) => {
                        return Err(WellfriendError::UnsupportedFeature(format!(
                            "{role} color space /{name} rejected: {reason}"
                        )));
                    }
                    crate::render::colorspace::NamedColor::Unhandled => {
                        return Err(WellfriendError::UnsupportedFeature(format!(
                            "{role} color space /{name} is unsupported for SVG vector output"
                        )));
                    }
                }
            } else {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "{role} color space /{name} resource is missing"
                )));
            }
        }
        ColorSpaceHandler::strict_to_render_color(color, alpha).map_err(|reason| {
            WellfriendError::UnsupportedFeature(format!("{role} color rejected: {reason}"))
        })
    }

    fn resolve_color_or_fatal(
        &mut self,
        color: &Color,
        alpha: f32,
        role: &str,
    ) -> Option<(String, f32)> {
        match self.resolve_color(color, alpha, role) {
            Ok(color) => Some(color),
            Err(err) => {
                self.record_fatal_error(err);
                None
            }
        }
    }

    fn resolve_render_color_or_fatal(
        &mut self,
        color: &Color,
        alpha: f32,
        role: &str,
    ) -> Option<RenderColor> {
        match self.resolve_render_color(color, alpha, role) {
            Ok(color) => Some(color),
            Err(err) => {
                self.record_fatal_error(err);
                None
            }
        }
    }

    fn current_fill_color_or_fatal(&mut self, role: &str) -> Option<(String, f32)> {
        let color = self.gs.fill_color.clone();
        self.resolve_color_or_fatal(&color, self.gs.fill_alpha as f32, role)
    }

    fn current_stroke_color_or_fatal(&mut self, role: &str) -> Option<(String, f32)> {
        let color = self.gs.stroke_color.clone();
        self.resolve_color_or_fatal(&color, self.gs.stroke_alpha as f32, role)
    }

    fn current_fill_render_color_or_fatal(
        &mut self,
        alpha: f32,
        role: &str,
    ) -> Option<RenderColor> {
        let color = self.gs.fill_color.clone();
        self.resolve_render_color_or_fatal(&color, alpha, role)
    }

    // ---- text ----

    fn show_text_array(&mut self, op: &ContentOperation) {
        let Some(items) = op.operand(0).and_then(Operand::as_array) else {
            return;
        };
        for item in items {
            match item {
                Operand::String(bytes) => self.show_text(bytes),
                Operand::Integer(v) => self.adjust_text_position(-(*v as f64)),
                Operand::Real(v) => self.adjust_text_position(-*v),
                _ => {}
            }
        }
    }

    fn show_text(&mut self, bytes: &[u8]) {
        let font_name = self.gs.text.font_name.clone();
        let font_size = self.gs.text.font_size;
        if font_size <= 0.0 {
            return;
        }
        let reader = self.engine.document().reader();
        let decoded = match try_decode_text_bytes(bytes, &font_name, &self.resources, reader) {
            Ok(decoded) => decoded,
            Err(reason) => {
                self.record_fatal_error(WellfriendError::UnsupportedFeature(reason));
                return;
            }
        };
        let font_bytes = get_font_bytes(&font_name, &self.resources, reader);
        let font_program = font_bytes.as_deref().filter(|bytes| !bytes.is_empty());
        let upem = font_program
            .and_then(get_upem)
            .map(f64::from)
            .filter(|v| *v > 0.0)
            .unwrap_or(1000.0);

        for glyph in decoded {
            let font_advance =
                font_program.and_then(|fb| decoded_glyph_strict_horizontal_advance(&glyph, fb));
            // Render mode 3 is invisible and not a clipping mode; skip outline emission.
            if !matches!(self.gs.text.rendering_mode, 3) {
                let Some(fb) = font_program else {
                    self.record_fatal_error(WellfriendError::UnsupportedFeature(
                        "SVG text font program unavailable for vector output".to_string(),
                    ));
                    return;
                };
                if decoded_glyph_strict_outline(&glyph, fb).is_none() {
                    self.record_fatal_error(WellfriendError::UnsupportedFeature(
                        "SVG text glyph outline unavailable for vector output".to_string(),
                    ));
                    return;
                }
                self.emit_glyph(&glyph, fb, upem);
            }
            let advance = if glyph.is_vertical {
                0.0
            } else {
                match glyph
                    .width
                    .filter(|advance| advance.is_finite())
                    .or(font_advance)
                {
                    Some(advance) => advance,
                    None => {
                        self.record_fatal_error(WellfriendError::UnsupportedFeature(
                            "SVG text glyph advance unavailable for vector output".to_string(),
                        ));
                        return;
                    }
                }
            };
            if !self.advance_decoded_text(advance, &glyph) {
                return;
            }
        }
    }

    /// Emit one glyph as an SVG `<path>` outline (text-as-outlines).
    fn emit_glyph(&mut self, glyph: &DecodedGlyph, font_bytes: &[u8], upem: f64) {
        let Some(outline) = decoded_glyph_strict_outline(glyph, font_bytes) else {
            return;
        };
        let Some(glyph_path) = outline else {
            return;
        };

        let scale = font_size_scale(self.gs.text.font_size, upem);
        let th = self.gs.text.horizontal_scaling / 100.0;
        let scale_x = scale * th;
        if scale <= 0.0 || !scale_x.is_finite() {
            return;
        }
        let glyph_ctm = Transform2D::scale(scale_x, scale)
            .concat(&Transform2D::translation(0.0, self.gs.text.rise))
            .concat(&Transform2D::from(self.gs.text.tm))
            .concat(&self.ctm());
        let flat = flatten_path(&glyph_path, &glyph_ctm, &self.viewport, 0.3);
        let d = path_data(&flat);
        if d.is_empty() {
            return;
        }
        self.append_text_clip_path(&flat);

        // Text rendering modes: 0/4 fill, 1/5 stroke, 2/6 fill+stroke.
        let clip = self.clip_attr();
        match self.gs.text.rendering_mode {
            1 | 5 => {
                if self.gs.stroke_color_space.is_pattern() {
                    let outline = stroke_flat_path(
                        &flat,
                        self.device_line_width(),
                        &self.device_dash_state(),
                        self.gs.line_cap.clone(),
                        self.gs.line_join.clone(),
                        self.gs.miter_limit,
                    );
                    self.emit_pattern_clip(
                        self.gs.stroke_pattern_name.clone(),
                        &outline,
                        FillRule::NonZero,
                        self.gs.stroke_alpha as f32,
                        true,
                        "text stroke",
                    );
                } else {
                    let Some((rgb, a)) =
                        self.current_stroke_color_or_fatal("SVG text stroke color")
                    else {
                        return;
                    };
                    let w = self.device_line_width();
                    let op = opacity_attr("stroke-opacity", a);
                    let dash = self.dash_attr();
                    let line_style = self.line_style_attrs();
                    let blend = self.blend_attr();
                    self.sink.push_element(&format!(
                        "<path d=\"{d}\" fill=\"none\" stroke=\"{rgb}\" stroke-width=\"{w:.3}\"{op}{dash}{line_style}{blend}{clip}/>"
                    ));
                }
            }
            2 | 6 => {
                let fill_is_pattern = self.gs.fill_color_space.is_pattern();
                let stroke_is_pattern = self.gs.stroke_color_space.is_pattern();
                let w = self.device_line_width();
                let dash = self.dash_attr();
                let line_style = self.line_style_attrs();
                if !fill_is_pattern && !stroke_is_pattern {
                    let Some((fill_rgb, fill_alpha)) =
                        self.current_fill_color_or_fatal("SVG text fill color")
                    else {
                        return;
                    };
                    let Some((stroke_rgb, stroke_alpha)) =
                        self.current_stroke_color_or_fatal("SVG text stroke color")
                    else {
                        return;
                    };
                    let fill_opacity = opacity_attr("fill-opacity", fill_alpha);
                    let stroke_opacity = opacity_attr("stroke-opacity", stroke_alpha);
                    let blend = self.blend_attr();
                    self.sink.push_element(&format!(
                        "<path d=\"{d}\" fill=\"{fill_rgb}\" stroke=\"{stroke_rgb}\" stroke-width=\"{w:.3}\"{fill_opacity}{stroke_opacity}{dash}{line_style}{blend}{clip}/>"
                    ));
                } else {
                    if fill_is_pattern {
                        self.emit_pattern_clip(
                            self.gs.fill_pattern_name.clone(),
                            &flat,
                            FillRule::NonZero,
                            self.gs.fill_alpha as f32,
                            false,
                            "text fill",
                        );
                    } else {
                        let Some((fill_rgb, fill_alpha)) =
                            self.current_fill_color_or_fatal("SVG text fill color")
                        else {
                            return;
                        };
                        let fill_opacity = opacity_attr("fill-opacity", fill_alpha);
                        let blend = self.blend_attr();
                        self.sink.push_element(&format!(
                            "<path d=\"{d}\" fill=\"{fill_rgb}\"{fill_opacity}{blend}{clip}/>"
                        ));
                    }
                    if stroke_is_pattern {
                        let outline = stroke_flat_path(
                            &flat,
                            w,
                            &self.device_dash_state(),
                            self.gs.line_cap.clone(),
                            self.gs.line_join.clone(),
                            self.gs.miter_limit,
                        );
                        self.emit_pattern_clip(
                            self.gs.stroke_pattern_name.clone(),
                            &outline,
                            FillRule::NonZero,
                            self.gs.stroke_alpha as f32,
                            true,
                            "text stroke",
                        );
                    } else {
                        let Some((stroke_rgb, stroke_alpha)) =
                            self.current_stroke_color_or_fatal("SVG text stroke color")
                        else {
                            return;
                        };
                        let stroke_opacity = opacity_attr("stroke-opacity", stroke_alpha);
                        let blend = self.blend_attr();
                        self.sink.push_element(&format!(
                            "<path d=\"{d}\" fill=\"none\" stroke=\"{stroke_rgb}\" stroke-width=\"{w:.3}\"{stroke_opacity}{dash}{line_style}{blend}{clip}/>"
                        ));
                    }
                }
            }
            7 => {}
            _ => {
                if self.gs.fill_color_space.is_pattern() {
                    self.emit_pattern_clip(
                        self.gs.fill_pattern_name.clone(),
                        &flat,
                        FillRule::NonZero,
                        self.gs.fill_alpha as f32,
                        false,
                        "text fill",
                    );
                } else {
                    let Some((rgb, a)) = self.current_fill_color_or_fatal("SVG text fill color")
                    else {
                        return;
                    };
                    let op = opacity_attr("fill-opacity", a);
                    let blend = self.blend_attr();
                    self.sink.push_element(&format!(
                        "<path d=\"{d}\" fill=\"{rgb}\"{op}{blend}{clip}/>"
                    ));
                }
            }
        }
    }

    fn advance_text(&mut self, glyph_width: f64, is_space: bool) {
        let th = self.gs.text.horizontal_scaling / 100.0;
        let mut advance =
            (glyph_width / 1000.0) * self.gs.text.font_size * th + self.gs.text.char_spacing * th;
        if is_space {
            advance += self.gs.text.word_spacing * th;
        }
        self.translate_text_matrix(advance, 0.0);
    }

    fn advance_decoded_text(&mut self, glyph_width: f64, glyph: &DecodedGlyph) -> bool {
        if !glyph.is_vertical {
            self.advance_text(glyph_width, glyph.is_space);
            return true;
        }
        let Some(vertical_advance) = glyph.vertical_advance.filter(|advance| advance.is_finite())
        else {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(
                "SVG text vertical advance unavailable for vector output".to_string(),
            ));
            return false;
        };
        let mut advance_y = vertical_advance / 1000.0 * self.gs.text.font_size;
        let spacing = self.gs.text.char_spacing
            + if glyph.is_space {
                self.gs.text.word_spacing
            } else {
                0.0
            };
        if spacing != 0.0 {
            let sign = if advance_y < 0.0 { -1.0 } else { 1.0 };
            advance_y += spacing * sign;
        }
        self.translate_text_matrix(0.0, advance_y);
        true
    }

    fn adjust_text_position(&mut self, adjustment: f64) {
        let tx = adjustment / 1000.0
            * self.gs.text.font_size
            * (self.gs.text.horizontal_scaling / 100.0);
        self.translate_text_matrix(tx, 0.0);
    }

    fn translate_text_matrix(&mut self, tx: f64, ty: f64) {
        let mut tm = self.gs.text.tm;
        tm[4] += tm[0] * tx + tm[2] * ty;
        tm[5] += tm[1] * tx + tm[3] * ty;
        self.gs.text.tm = tm;
    }

    fn next_text_line(&mut self) {
        let op = ContentOperation::new("T*", Vec::new());
        self.gs.process(&op);
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

fn tiling_pattern_tile_range(
    flat: &FlatPath,
    program: &VectorTilingPatternProgram,
    pattern_ctm: Transform2D,
    viewport: &Viewport,
) -> Option<(i64, i64, i64, i64, usize)> {
    let (dx0, dy0, dx1, dy1) = flat_device_bounds(flat, viewport.width_px, viewport.height_px)?;
    let full = pattern_ctm.concat(&viewport.to_transform());
    let inv = full.inverse()?;
    let corners = [
        inv.transform_point(dx0, dy0),
        inv.transform_point(dx1, dy0),
        inv.transform_point(dx0, dy1),
        inv.transform_point(dx1, dy1),
    ];
    let (mut pminx, mut pminy, mut pmaxx, mut pmaxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (px, py) in corners {
        pminx = pminx.min(px);
        pminy = pminy.min(py);
        pmaxx = pmaxx.max(px);
        pmaxy = pmaxy.max(py);
    }
    let bbox = program.bbox;
    let i0 = ((pminx - bbox[2]) / program.x_step).floor() as i64;
    let i1 = ((pmaxx - bbox[0]) / program.x_step).ceil() as i64;
    let j0 = ((pminy - bbox[3]) / program.y_step).floor() as i64;
    let j1 = ((pmaxy - bbox[1]) / program.y_step).ceil() as i64;
    let count = (i1 - i0 + 1).max(0) as u128 * (j1 - j0 + 1).max(0) as u128;
    if count == 0 || count > usize::MAX as u128 {
        return None;
    }
    Some((i0, i1, j0, j1, count as usize))
}

fn flat_device_bounds(flat: &FlatPath, width: u32, height: u32) -> Option<(f64, f64, f64, f64)> {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for subpath in &flat.subpaths {
        for &(x, y) in subpath {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
    }
    if minx > maxx || miny > maxy || width == 0 || height == 0 {
        return None;
    }
    minx = minx.floor().max(0.0);
    miny = miny.floor().max(0.0);
    maxx = maxx.ceil().min(f64::from(width));
    maxy = maxy.ceil().min(f64::from(height));
    (maxx > minx && maxy > miny).then_some((minx, miny, maxx, maxy))
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

fn transform_preserves_circles(transform: Transform2D) -> bool {
    let sx = (transform.a * transform.a + transform.b * transform.b).sqrt();
    let sy = (transform.c * transform.c + transform.d * transform.d).sqrt();
    if !sx.is_finite() || !sy.is_finite() || sx <= 1e-9 || sy <= 1e-9 {
        return false;
    }
    let dot = transform.a * transform.c + transform.b * transform.d;
    dot.is_finite() && dot.abs() <= 1e-6 && (sx - sy).abs() <= 1e-6
}

/// Build SVG path data (`M x y L x y ... Z`) from device-space flattened
/// polylines. Coordinates are emitted with modest precision.
fn path_data(flat: &FlatPath) -> String {
    let mut d = String::new();
    for (sp, closed) in flat.subpaths.iter().zip(flat.closed.iter()) {
        if sp.is_empty() {
            continue;
        }
        let mut iter = sp.iter();
        let first = iter.next().unwrap();
        d.push_str(&format!("M{:.2} {:.2}", first.0, first.1));
        for p in iter {
            d.push_str(&format!(" L{:.2} {:.2}", p.0, p.1));
        }
        if *closed {
            d.push_str(" Z");
        }
    }
    d
}

fn rgb_hex(c: &RenderColor) -> String {
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", to_u8(c.r), to_u8(c.g), to_u8(c.b))
}

fn rgb_array_hex(c: [f32; 3]) -> String {
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}", to_u8(c[0]), to_u8(c[1]), to_u8(c[2]))
}

fn svg_gradient_stops(stops: &[VectorShadingStop]) -> String {
    let mut out = String::new();
    for stop in stops {
        let offset = svg_gradient_offset(stop.offset);
        let color = rgb_array_hex(stop.rgb);
        out.push_str(&format!(
            "<stop offset=\"{offset}\" stop-color=\"{color}\"/>"
        ));
    }
    out
}

fn svg_gradient_offset(offset: f64) -> String {
    let offset = offset.clamp(0.0, 1.0);
    if offset <= 1e-9 {
        "0".to_string()
    } else if offset >= 1.0 - 1e-9 {
        "1".to_string()
    } else {
        format!("{offset:.6}")
    }
}

fn reversed_svg_gradient_stops(stops: &[VectorShadingStop]) -> Vec<VectorShadingStop> {
    let mut reversed: Vec<VectorShadingStop> = stops
        .iter()
        .map(|stop| VectorShadingStop {
            offset: 1.0 - stop.offset,
            rgb: stop.rgb,
        })
        .collect();
    reversed.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    reversed
}

/// Emit a `fill-opacity`/`stroke-opacity` attribute only when alpha < 1.
fn opacity_attr(attr: &str, alpha: f32) -> String {
    if alpha >= 0.999 {
        String::new()
    } else {
        format!(" {attr}=\"{:.3}\"", alpha.clamp(0.0, 1.0))
    }
}

fn svg_blend_mode_name(mode: BlendMode) -> Option<&'static str> {
    match mode {
        BlendMode::Normal => None,
        BlendMode::Multiply => Some("multiply"),
        BlendMode::Screen => Some("screen"),
        BlendMode::Overlay => Some("overlay"),
        BlendMode::Darken => Some("darken"),
        BlendMode::Lighten => Some("lighten"),
        BlendMode::ColorDodge => Some("color-dodge"),
        BlendMode::ColorBurn => Some("color-burn"),
        BlendMode::HardLight => Some("hard-light"),
        BlendMode::SoftLight => Some("soft-light"),
        BlendMode::Difference => Some("difference"),
        BlendMode::Exclusion => Some("exclusion"),
        BlendMode::Hue => Some("hue"),
        BlendMode::Saturation => Some("saturation"),
        BlendMode::Color => Some("color"),
        BlendMode::Luminosity => Some("luminosity"),
    }
}

fn regional_bounds_attr(kind: &str, bounds: [f64; 4]) -> String {
    format!(
        " data-wellfriend-region-kind=\"{kind}\" data-wellfriend-region-bounds=\"{:.3} {:.3} {:.3} {:.3}\"",
        bounds[0], bounds[1], bounds[2], bounds[3]
    )
}

fn stencil_mask_to_svg_mask_rgba(
    raw: &crate::images::decoder::RawImage,
    paint_ones: bool,
) -> Result<crate::images::decoder::RawImage> {
    inline_stencil_mask_to_rgba(raw, [255, 255, 255, 255], paint_ones)
}

/// Minimal, dependency-free base64 (standard alphabet) for embedding raster
/// page images as data URIs.
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_pdf() -> Vec<u8> {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut off = [0usize; 4];
        off[1] = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        off[2] = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        off[3] = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>\nendobj\n",
        );
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
        for offset in off.iter().take(4).skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        pdf
    }

    #[test]
    fn svg_named_color_resolution_rejects_invalid_space_locally() {
        let engine = ContentEngine::open_bytes(minimal_pdf()).expect("minimal PDF");
        let mut resources = PageResources::default();
        resources.color_spaces.insert(
            "Bad".to_string(),
            PdfObject::Array(vec![
                PdfObject::Name("CalRGB".to_string()),
                PdfObject::Dictionary(PdfDictionary::empty()),
            ]),
        );
        let mut sink = SvgSink::new(10, 10);
        let mut state = SvgRenderState {
            engine: &engine,
            resources,
            viewport: Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            gs: GraphicsState::default(),
            path: Path::new(),
            pending_clip: None,
            clip_stack: Vec::new(),
            sink: &mut sink,
            regional_image_names: Vec::new(),
            regional_inline_image_count: 0,
            regional_form_names: Vec::new(),
            regional_shading_names: Vec::new(),
            pending_inline_params: None,
            form_depth: 0,
            form_object_stack: Vec::new(),
            text_clip_path: None,
            fatal_error: None,
        };
        state.gs.fill_color_space = ColorSpace::Named("Bad".to_string());
        state.gs.fill_color = Color {
            space: ColorSpace::Named("Bad".to_string()),
            components: vec![0.2, 0.4, 0.6],
        };

        assert!(state
            .current_fill_color_or_fatal("SVG test fill color")
            .is_none());
        let err = state
            .fatal_error
            .take()
            .expect("invalid named color space should be fatal");
        assert!(
            err.to_string()
                .contains("SVG test fill color color space /Bad rejected"),
            "{err}"
        );
    }

    #[test]
    fn svg_device_color_resolution_rejects_malformed_state_locally() {
        let engine = ContentEngine::open_bytes(minimal_pdf()).expect("minimal PDF");
        let mut sink = SvgSink::new(10, 10);
        let mut state = SvgRenderState {
            engine: &engine,
            resources: PageResources::default(),
            viewport: Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            gs: GraphicsState::default(),
            path: Path::new(),
            pending_clip: None,
            clip_stack: Vec::new(),
            sink: &mut sink,
            regional_image_names: Vec::new(),
            regional_inline_image_count: 0,
            regional_form_names: Vec::new(),
            regional_shading_names: Vec::new(),
            pending_inline_params: None,
            form_depth: 0,
            form_object_stack: Vec::new(),
            text_clip_path: None,
            fatal_error: None,
        };
        state.gs.fill_color = Color {
            space: ColorSpace::DeviceRGB,
            components: vec![1.0, 0.0],
        };

        assert!(state
            .current_fill_color_or_fatal("SVG test fill color")
            .is_none());
        let err = state
            .fatal_error
            .take()
            .expect("malformed device color should be fatal");
        assert!(
            err.to_string()
                .contains("SVG test fill color color rejected: malformed DeviceRGB color"),
            "{err}"
        );
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn rgb_hex_formats() {
        assert_eq!(rgb_hex(&RenderColor::rgb(1.0, 0.0, 0.0)), "#FF0000");
        assert_eq!(rgb_hex(&RenderColor::rgb(0.0, 1.0, 0.0)), "#00FF00");
        assert_eq!(rgb_hex(&RenderColor::rgb(0.0, 0.0, 0.0)), "#000000");
    }

    #[test]
    fn opacity_attr_omitted_when_opaque() {
        assert_eq!(opacity_attr("fill-opacity", 1.0), "");
        assert!(opacity_attr("fill-opacity", 0.5).contains("0.500"));
    }

    #[test]
    fn shading_bbox_clip_refuses_unflattenable_geometry() {
        let engine = ContentEngine::open_bytes(minimal_pdf()).expect("minimal PDF");
        let mut sink = SvgSink::new(10, 10);
        let mut state = SvgRenderState {
            engine: &engine,
            resources: PageResources::default(),
            viewport: Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            gs: GraphicsState::default(),
            path: Path::new(),
            pending_clip: None,
            clip_stack: Vec::new(),
            sink: &mut sink,
            regional_image_names: Vec::new(),
            regional_inline_image_count: 0,
            regional_form_names: Vec::new(),
            regional_shading_names: Vec::new(),
            pending_inline_params: None,
            form_depth: 0,
            form_object_stack: Vec::new(),
            text_clip_path: None,
            fatal_error: None,
        };
        let shading = VectorShading::Axial(crate::render::vector_fallback::VectorAxialShading {
            coords: [0.0, 0.0, 5.0, 0.0],
            domain: [0.0, 1.0],
            c0: [1.0, 0.0, 0.0],
            c1: [0.0, 0.0, 1.0],
            stops: vec![
                VectorShadingStop {
                    offset: 0.0,
                    rgb: [1.0, 0.0, 0.0],
                },
                VectorShadingStop {
                    offset: 1.0,
                    rgb: [0.0, 0.0, 1.0],
                },
            ],
            ps_function: None,
            ps_color_space: None,
            extend: [true, true],
            bbox: Some([0.0, 0.0, 5.0, 5.0]),
        });
        let err = state
            .shading_clip_attr(
                &shading,
                Transform2D::new(f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0),
                "test shading",
            )
            .expect_err("non-finite BBox transform must fail closed");
        assert!(
            err.to_string()
                .contains("test shading /BBox could not be flattened"),
            "{err}"
        );
        assert!(!sink.finish().contains("<clipPath"));
    }

    #[test]
    fn raster_fallback_triggers_on_unsupported_constructs() {
        use crate::render::vector_fallback::{
            classify_page_for_vector_output, VectorFallbackDecision,
        };
        let r = PageResources::default();

        // Inline image → whole-page.
        let bi_op = ContentOperation::new("BI", vec![]);
        assert!(matches!(
            classify_page_for_vector_output(&[bi_op], &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));

        // Missing/unsupported shading → whole-page.
        let sh_op = ContentOperation::new("sh", vec![Operand::Name("Sh0".into())]);
        assert!(matches!(
            classify_page_for_vector_output(&[sh_op], &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));

        // ExtGState → whole-page.
        let gs_op = ContentOperation::new("gs", vec![Operand::Name("GS0".into())]);
        assert!(matches!(
            classify_page_for_vector_output(&[gs_op], &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));

        // Text showing outside a text object is malformed and stays fail-closed.
        let dense_text: Vec<_> = (0..128)
            .map(|_| ContentOperation::new("Tj", vec![Operand::String(vec![b'x'])]))
            .collect();
        assert!(matches!(
            classify_page_for_vector_output(&dense_text, &r, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));

        // Pure path ops → pure vector.
        let m = ContentOperation::new("m", vec![Operand::Real(0.0), Operand::Real(0.0)]);
        let f = ContentOperation::new("f", vec![]);
        assert!(matches!(
            classify_page_for_vector_output(&[m, f], &r, 1.0),
            VectorFallbackDecision::PureVector
        ));

        // Image XObject without resolvable image metadata remains conservative.
        let mut r_img = PageResources::default();
        r_img
            .xobject_subtypes
            .insert("Im0".to_string(), "Image".to_string());
        r_img.xobjects.insert("Im0".to_string(), (1, 0));
        let cm = ContentOperation::new(
            "cm",
            vec![
                Operand::Real(200.0),
                Operand::Real(0.0),
                Operand::Real(0.0),
                Operand::Real(-100.0),
                Operand::Real(50.0),
                Operand::Real(400.0),
            ],
        );
        let do_op = ContentOperation::new("Do", vec![Operand::Name("Im0".into())]);
        assert!(matches!(
            classify_page_for_vector_output(&[cm, do_op], &r_img, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));

        // Form XObject Do → whole-page.
        let mut r_form = PageResources::default();
        r_form
            .xobject_subtypes
            .insert("Fm0".to_string(), "Form".to_string());
        r_form.xobjects.insert("Fm0".to_string(), (2, 0));
        let do_form = ContentOperation::new("Do", vec![Operand::Name("Fm0".into())]);
        assert!(matches!(
            classify_page_for_vector_output(&[do_form], &r_form, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }
}
