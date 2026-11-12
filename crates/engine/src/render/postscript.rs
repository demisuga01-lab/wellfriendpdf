//! PostScript / EPS vector output backend (`pdftops` / `pdftocairo -ps`/`-eps`
//! equivalent).
//!
//! # Design — a third output of the SAME interpretation as SVG
//!
//! This is a **sibling renderer**, exactly like [`crate::render::svg`]: it does
//! NOT introduce a third independent content-stream walker. It reuses the same
//! drawing-operation source and primitives the SVG sink uses:
//!
//! - [`GraphicsState`] for every state operator (`cm`, `q`/`Q`, `Tf`, `Td`,
//!   colour ops, …) — identical interpretation to raster/SVG, for free.
//! - [`flatten_path`] for user→device geometry. As in the SVG sink, paths are
//!   produced in **device-pixel space** (top-left origin, y-down — the same
//!   space the raster output lives in), so the emitted PostScript rasterises
//!   pixel-for-pixel like the raster render.
//! - the shared [`glyph_outline`](crate::render::glyph_outline) /
//!   [`text_decode`](crate::render::text_decode) helpers for glyph lookup.
//!
//! # Curves, fonts and images (`pdftops` parity)
//!
//! - Paths keep their Bézier segments (`curveto`); the PDF flatness travels
//!   as `setflat`.
//! - Filled text is real PostScript text in embedded fonts — TrueType as
//!   Type 42, every other outline font as a Type 1 font built from its glyph
//!   outlines — subset to the glyphs the document uses, defined once in the
//!   document setup and shown with `xshow`/`xyshow` at each glyph's exact
//!   origin, so it stays searchable (see `postscript_fonts.rs`). Type 3 and
//!   unparsable fonts, clip/stroke render modes and pattern-filled text stay
//!   filled outlines.
//! - Image samples are Flate-compressed at LanguageLevel 3 (RunLength at
//!   Level 2) inside ASCII85; DCT (JPEG) and CCITT fax images pass through
//!   with their original compression.
//!
//! # Device-pixel coordinates in a bottom-left PostScript world
//!
//! PostScript's default user space has its origin at the **bottom-left** with
//! y increasing upward, whereas our flattened device coordinates have the
//! origin at the **top-left** with y increasing downward (image space). Rather
//! than re-derive a second set of coordinates, the page prologue installs a
//! single flip — `0 <height> translate  1 -1 scale` — so that emitting the
//! device-space polylines verbatim places them correctly on the PostScript
//! page. Every path, glyph outline and clip therefore shares one coordinate
//! convention with the raster and SVG backends.
//!
//! # Per-page vector-vs-raster decision (the rasterize-embed fallback)
//!
//! Pages using only operations PostScript represents natively here — paths,
//! text-as-outlines, solid fills/strokes, clipping — are emitted as **true
//! vector PostScript**. Pages using operations not faithfully expressible here
//! (unsafe Form XObjects, unsupported shadings, tiling/shading patterns, soft
//! masks, non-trivial blend modes) fall back to embedding the **whole page as one rasterised
//! image** drawn with the PostScript `image`/`colorimage` operator
//! (pixel-identical to the raster render).
//!
//! # Regional image fallback (RB-14)
//!
//! Pages with simple affine Image XObject `Do` operations, complete inline
//! images/masks, vector-safe Form XObjects, simple axial/radial shadings, or
//! safe opaque line-style/font/default-state ExtGState dictionaries now use a **regional
//! fallback**: local raster assets are bounded `colorimage` regions, and
//! shadings are native LanguageLevel 3 `shfill` dictionaries, while surrounding
//! vector content (paths, text, clips) is preserved as native PostScript. This
//! avoids whole-page rasterization for common mixed
//! vector/local-resource pages. Eligibility is determined by
//! [`crate::render::vector_fallback::classify_page_for_vector_output`].

use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::{Color, ColorSpace, GraphicsState};
use crate::engine::{ContentEngine, PageResources};
use crate::error::{Result, WellfriendError};
use crate::filters::DecodeLimits;
use crate::images::decoder::{ImageDecoder, RawImage};
use crate::images::locator::ImageReference;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::color::{ColorSpaceHandler, RenderColor};
use crate::render::glyph_outline::{font_size_scale, get_upem};
use crate::render::line::DashState;
use crate::render::path::{
    device_curve_path, effective_device_flatness, flatten_path, flatten_path_device_transform,
    stroke_flat_path, CompactNum as PsNum, FillRule, FlatPath, Path, PathSegment,
};
use crate::render::text_decode::{
    decoded_glyph_strict_horizontal_advance, decoded_glyph_strict_outline, get_font_bytes,
    try_decode_text_bytes, DecodedGlyph,
};
use crate::render::transform::{Transform2D, Viewport};
use crate::render::vector_fallback::{
    classify_page_for_postscript_output_with_reader, classify_scoped_postscript_vector_output,
    decode_inline_image_region, ensure_regional_raw_image, ensure_regional_stencil_mask,
    image_device_placement, inline_mask_sample_paints, load_vector_form_program,
    load_vector_shading_for_postscript_output, load_vector_shading_pattern_for_postscript_output,
    load_vector_tiling_pattern, resolved_regional_image_color_space_override,
    stencil_mask_paints_ones, vector_uncolored_tiling_paint_color, VectorFallbackDecision,
    VectorPostScriptAlternateColorSpace, VectorPostScriptCmykStitchingFunction,
    VectorPostScriptNamedColorFamily, VectorPostScriptShadingColorSpace,
    VectorPostScriptShadingFunction, VectorPostScriptStitchingFunction,
    VectorPostScriptTintStitchingFunction, VectorPostScriptType2CmykArrayFunction,
    VectorPostScriptType2CmykFunction, VectorPostScriptType2ComponentFunction,
    VectorPostScriptType2Function, VectorPostScriptType2RgbArrayFunction, VectorShading,
    VectorShadingStop, VectorTilingPatternPaintType, VectorTilingPatternProgram,
    MAX_PATTERN_STENCIL_CLIP_RECTS, MAX_VECTOR_FORM_DEPTH, MAX_VECTOR_TILING_PATTERN_CELLS,
};

#[path = "postscript_fonts.rs"]
mod fonts;

pub use fonts::PsFontResources;
use fonts::{PsFontKind, PsGlyphSource, PsPageText};

/// PostScript output options (`pdftops -level2` / `-level3`, font handling).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PsOptions {
    /// Target PostScript LanguageLevel, 2 or 3 (default 3). Level 3 output
    /// compresses image samples with `/FlateDecode`; Level 2 output uses
    /// `/RunLengthDecode`. Both wrap the bytes in ASCII85 and pass DCT
    /// (JPEG) images through unchanged.
    pub language_level: u8,
    /// Embed the PDF's fonts (Type 42 for TrueType, Type 1 for other outline
    /// fonts) and write searchable text (default `true`). When `false`, text
    /// is drawn as filled glyph outlines.
    pub embed_fonts: bool,
    /// Refuse whole-page raster fallback instead of embedding a rasterized
    /// page (default `false`); see [`render_page_ps_strict`].
    pub strict: bool,
}

impl Default for PsOptions {
    fn default() -> Self {
        Self {
            language_level: 3,
            embed_fonts: true,
            strict: false,
        }
    }
}

/// How image samples are compressed inside the ASCII85 wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PsImageFilter {
    /// LanguageLevel 3 `/FlateDecode` (zlib).
    Flate,
    /// LanguageLevel 2 `/RunLengthDecode`.
    RunLength,
}

impl PsImageFilter {
    fn for_options(options: &PsOptions) -> Self {
        if options.language_level >= 3 {
            PsImageFilter::Flate
        } else {
            PsImageFilter::RunLength
        }
    }
}

/// A single rendered PostScript page body plus the flag indicating whether it
/// was emitted as true vector PostScript or as a rasterize-and-embed fallback.
pub struct PsPage {
    /// The PostScript page body: everything between the per-page `%%Page:`
    /// comment's setup and the trailing `showpage`, including the `gsave`/
    /// coordinate flip prologue and the matching `grestore`.
    pub body: String,
    /// Page width in device pixels (the `%%BoundingBox` width / `showpage`
    /// media size for this page).
    pub width: u32,
    /// Page height in device pixels.
    pub height: u32,
    /// PostScript points (1/72 inch) per device pixel: `72 / dpi`. The body
    /// draws in device pixels and the prologue scales it by this factor, so
    /// the printed page has the PDF page's physical size at any DPI.
    pub points_per_pixel: f64,
    /// True when the whole page was embedded as a raster image because it used
    /// operations the vector sink cannot express natively.
    pub is_rasterized: bool,
    /// True when the page used regional vector fallback: vector content is
    /// preserved natively around bounded images or vector-safe Form subprograms.
    pub has_regional_images: bool,
    /// Embedded fonts the body's text refers to. [`assemble_ps_document`] and
    /// [`assemble_eps_document`] define them once in the document setup; a
    /// page body with fonts is not self-contained without that setup.
    pub font_resources: PsFontResources,
}

pub fn render_page_ps(engine: &ContentEngine, page_number: usize, dpi: u32) -> Result<PsPage> {
    render_page_ps_with_options(engine, page_number, dpi, &PsOptions::default())
}

/// Render a single page to PostScript with explicit [`PsOptions`].
pub fn render_page_ps_with_options(
    engine: &ContentEngine,
    page_number: usize,
    dpi: u32,
    options: &PsOptions,
) -> Result<PsPage> {
    let policy = if options.strict {
        PsWholePageRasterPolicy::Refuse
    } else {
        PsWholePageRasterPolicy::Allow
    };
    render_page_ps_with_policy(engine, page_number, dpi, policy, options)
}

/// Render a single page to PostScript without silently rasterizing the whole
/// page when the vector sink cannot represent the page exactly.
pub fn render_page_ps_strict(
    engine: &ContentEngine,
    page_number: usize,
    dpi: u32,
) -> Result<PsPage> {
    let options = PsOptions {
        strict: true,
        ..PsOptions::default()
    };
    render_page_ps_with_options(engine, page_number, dpi, &options)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PsWholePageRasterPolicy {
    Allow,
    Refuse,
}

/// Render a single page to a PostScript page body. Uses the shared fallback
/// classifier to decide: pure vector, regional image fallback, or whole-page
/// raster only when the caller selected compatibility raster fallback.
fn render_page_ps_with_policy(
    engine: &ContentEngine,
    page_number: usize,
    dpi: u32,
    whole_page_policy: PsWholePageRasterPolicy,
    options: &PsOptions,
) -> Result<PsPage> {
    let viewport = engine.page_viewport(page_number, dpi)?;
    let ops = engine.get_page_content(page_number)?;
    let resources = engine.get_page_resources(page_number)?;

    let decision = classify_page_for_postscript_output_with_reader(
        &ops,
        &resources,
        viewport.scale,
        engine.document().reader(),
    );

    match decision {
        VectorFallbackDecision::WholePageRaster { reason } => match whole_page_policy {
            PsWholePageRasterPolicy::Allow => {
                rasterized_page(engine, page_number, &viewport, options)
            }
            PsWholePageRasterPolicy::Refuse => Err(WellfriendError::UnsupportedFeature(format!(
                "strict PostScript vector output refuses whole-page raster fallback: {reason}"
            ))),
        },
        VectorFallbackDecision::PureVector => render_vector_ps(
            engine,
            &ops,
            &resources,
            &viewport,
            RegionalVectorResources::empty(),
            options,
        ),
        VectorFallbackDecision::RegionalImageFallback {
            ref image_names,
            inline_image_count,
            ref form_names,
            ref shading_names,
        } => {
            match render_vector_ps(
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
                options,
            ) {
                Ok(page) => Ok(page),
                Err(WellfriendError::UnsupportedFeature(_)) => match whole_page_policy {
                    PsWholePageRasterPolicy::Allow => rasterized_page(engine, page_number, &viewport, options),
                    PsWholePageRasterPolicy::Refuse => Err(WellfriendError::UnsupportedFeature(
                        "strict PostScript vector output refuses regional replay fallback to whole-page raster"
                            .to_string(),
                    )),
                },
                Err(err) => Err(err),
            }
        }
    }
}

/// Render a page as native vector PostScript, optionally embedding regional
/// raster images for named Image XObjects.
fn render_vector_ps(
    engine: &ContentEngine,
    ops: &[ContentOperation],
    resources: &PageResources,
    viewport: &Viewport,
    regional: RegionalVectorResources<'_>,
    options: &PsOptions,
) -> Result<PsPage> {
    let (w, h) = (viewport.width_px, viewport.height_px);
    let points_per_pixel = viewport_points_per_pixel(viewport);
    let mut sink = PsSink::new(w, h)
        .with_points_per_pixel(points_per_pixel)
        .with_image_filter(PsImageFilter::for_options(options));
    let mut state = PsRenderState {
        engine,
        page_resources: resources.clone(),
        font_bytes_cache: std::collections::HashMap::new(),
        font_resolver_cache: std::collections::HashMap::new(),
        resources: resources.clone(),
        viewport: viewport.clone(),
        gs: GraphicsState::default(),
        path: Path::new(),
        pending_clip: None,
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
        embed_fonts: options.embed_fonts,
        text_font_cache: std::collections::HashMap::new(),
        text_run: None,
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
    let font_resources = std::mem::take(&mut sink.text.resources);
    Ok(PsPage {
        body: sink.finish(),
        width: w,
        height: h,
        points_per_pixel,
        is_rasterized: false,
        has_regional_images: has_regional,
        font_resources,
    })
}

/// PostScript points per device pixel for a viewport (`72 / dpi`).
fn viewport_points_per_pixel(viewport: &Viewport) -> f64 {
    if viewport.dpi > 0 {
        72.0 / f64::from(viewport.dpi)
    } else {
        1.0
    }
}

impl PsPage {
    /// Page size in PostScript points.
    pub fn size_points(&self) -> (f64, f64) {
        let scale = if self.points_per_pixel.is_finite() && self.points_per_pixel > 0.0 {
            self.points_per_pixel
        } else {
            1.0
        };
        (
            f64::from(self.width) * scale,
            f64::from(self.height) * scale,
        )
    }
}

/// Format a PostScript number with at most six decimals and no trailing zeros.
fn ps_number(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// Device-space geometry of one painted path: Bézier curves kept as is, or
/// the flattened polyline when the curve form cannot be produced.
enum DevicePath {
    Curves(Path),
    Flat(FlatPath),
}

impl DevicePath {
    fn new(path: &Path, ctm: &Transform2D, viewport: &Viewport, flatness: f64) -> Self {
        match device_curve_path(path, ctm, viewport) {
            Some(curves) => DevicePath::Curves(curves),
            None => DevicePath::Flat(flatten_path(path, ctm, viewport, flatness)),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            DevicePath::Curves(path) => path.is_empty(),
            DevicePath::Flat(flat) => flat.subpaths.iter().all(|s| s.is_empty()),
        }
    }
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
    reader: &PdfReader,
) -> Result<ImageReference> {
    let context = format!("regional PS image /{name}");
    let filter = extract_image_filter_names(dict, &context)?;
    let color_space = if is_mask {
        "DeviceGray".to_string()
    } else {
        extract_image_color_space_name(dict, reader, &context)?
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

fn extract_image_color_space_name(
    dict: &PdfDictionary,
    reader: &PdfReader,
    context: &str,
) -> Result<String> {
    let source = dict
        .get("ColorSpace")
        .or_else(|| dict.get("CS"))
        .ok_or_else(|| WellfriendError::MalformedPdf(format!("{context} missing /ColorSpace")))?;
    let resolved = match source {
        PdfObject::Reference { .. } => reader.resolve(source.clone()).map_err(|err| {
            WellfriendError::MalformedPdf(format!("{context} failed to resolve /ColorSpace: {err}"))
        })?,
        other => other.clone(),
    };
    match &resolved {
        PdfObject::Name(name) => Ok(canonical_image_color_space_name(name)),
        PdfObject::Array(items) => items
            .first()
            .and_then(PdfObject::as_name)
            .map(canonical_image_color_space_name)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(format!("{context} malformed /ColorSpace array"))
            }),
        _ => Err(WellfriendError::MalformedPdf(format!(
            "{context} /ColorSpace is not a name or array"
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

/// Emit a page as a single embedded raster image drawn with `colorimage`
/// (the fallback). The RGB samples are emitted as ASCII-hex so the output is a
/// pure 7-bit-clean conforming PostScript stream (no binary).
fn rasterized_page(
    engine: &ContentEngine,
    page_number: usize,
    viewport: &Viewport,
    options: &PsOptions,
) -> Result<PsPage> {
    let buf = engine.render_page(page_number, viewport.dpi)?;
    let raw = buf.to_raw_image(); // 3 channels, 8-bit
    let (w, h) = (viewport.width_px, viewport.height_px);

    let points_per_pixel = viewport_points_per_pixel(viewport);
    let mut sink = PsSink::new(w, h)
        .with_points_per_pixel(points_per_pixel)
        .with_image_filter(PsImageFilter::for_options(options));
    sink.emit_raster_image(&raw.pixels, w, h);
    Ok(PsPage {
        body: sink.finish(),
        width: w,
        height: h,
        points_per_pixel,
        is_rasterized: true,
        has_regional_images: false,
        font_resources: PsFontResources::default(),
    })
}

/// Assemble one or more [`PsPage`] bodies into a complete, DSC-conformant
/// multi-page PostScript document (`%!PS-Adobe-3.0`).
pub fn assemble_ps_document(pages: &[PsPage]) -> String {
    let mut out = String::new();
    out.push_str("%!PS-Adobe-3.0\n");
    out.push_str("%%Creator: Wellfriend PDF SDK Toolkit\n");
    let language_level = pages
        .iter()
        .map(|page| body_language_level(&page.body))
        .max()
        .unwrap_or(2);
    out.push_str(&format!("%%LanguageLevel: {language_level}\n"));
    let fonts = fonts::merge_page_fonts(pages.iter().map(|page| &page.font_resources));
    push_supplied_fonts_comment(&fonts, &mut out);
    // The bounding box of a multi-page document is the union; DSC permits the
    // largest page. We report the max width/height across pages.
    // Sizes are PostScript points (1/72 inch), not device pixels, so the
    // printed page keeps the PDF page's physical size at any render DPI.
    let max_w = pages
        .iter()
        .map(|p| p.size_points().0)
        .fold(0.0_f64, f64::max);
    let max_h = pages
        .iter()
        .map(|p| p.size_points().1)
        .fold(0.0_f64, f64::max);
    out.push_str(&format!(
        "%%BoundingBox: 0 0 {} {}\n",
        max_w.ceil() as u64,
        max_h.ceil() as u64
    ));
    out.push_str(&format!(
        "%%HiResBoundingBox: 0 0 {} {}\n",
        ps_number(max_w),
        ps_number(max_h)
    ));
    out.push_str(&format!("%%Pages: {}\n", pages.len()));
    out.push_str("%%EndComments\n");
    push_prolog_and_setup(&fonts, &mut out);

    for (idx, page) in pages.iter().enumerate() {
        let n = idx + 1;
        out.push_str(&format!("%%Page: {n} {n}\n"));
        let (page_w, page_h) = page.size_points();
        out.push_str(&format!(
            "%%PageBoundingBox: 0 0 {} {}\n",
            page_w.ceil() as u64,
            page_h.ceil() as u64
        ));
        // Each page sets its own media size so viewers/printers select the
        // right page geometry.
        out.push_str(&format!(
            "<< /PageSize [{} {}] >> setpagedevice\n",
            ps_number(page_w),
            ps_number(page_h)
        ));
        out.push_str("%%BeginPageSetup\n");
        out.push_str("%%EndPageSetup\n");
        out.push_str(&page.body);
        out.push_str("showpage\n");
    }

    out.push_str("%%Trailer\n");
    out.push_str("%%EOF\n");
    out
}

/// Assemble a single page into a conforming EPS (`%!PS-Adobe-3.0 EPSF-3.0`)
/// document with a precise `%%BoundingBox` and no `setpagedevice`/`showpage`
/// global-state changes (EPS conformance: an EPS must not call `setpagedevice`
/// or rely on a `showpage`, so it can be embedded inside another document).
pub fn assemble_eps_document(page: &PsPage) -> String {
    let mut out = String::new();
    out.push_str("%!PS-Adobe-3.0 EPSF-3.0\n");
    out.push_str("%%Creator: Wellfriend PDF SDK Toolkit\n");
    let language_level = body_language_level(&page.body);
    out.push_str(&format!("%%LanguageLevel: {language_level}\n"));
    let fonts = fonts::merge_page_fonts([&page.font_resources]);
    push_supplied_fonts_comment(&fonts, &mut out);
    let (page_w, page_h) = page.size_points();
    out.push_str(&format!(
        "%%BoundingBox: 0 0 {} {}\n",
        page_w.ceil() as u64,
        page_h.ceil() as u64
    ));
    out.push_str(&format!(
        "%%HiResBoundingBox: 0 0 {} {}\n",
        ps_number(page_w),
        ps_number(page_h)
    ));
    out.push_str("%%EndComments\n");
    push_prolog_and_setup(&fonts, &mut out);
    // The page body is wrapped in its own gsave/grestore (added by PsSink), so
    // it does not leak graphics state to an embedding document. No showpage.
    out.push_str(&page.body);
    out.push_str("%%EOF\n");
    out
}

/// LanguageLevel a page body needs: 3 for `shfill` or `/FlateDecode`.
fn body_language_level(body: &str) -> u8 {
    if body.contains("shfill") || body.contains("/FlateDecode") {
        3
    } else {
        2
    }
}

/// `%%DocumentSuppliedResources` for the embedded fonts.
fn push_supplied_fonts_comment(fonts: &[fonts::PsFontProgram], out: &mut String) {
    for (index, font) in fonts.iter().enumerate() {
        let lead = if index == 0 {
            "%%DocumentSuppliedResources:"
        } else {
            "%%+"
        };
        out.push_str(&format!("{lead} font {}\n", font.base_name));
    }
}

/// The prolog (Wellfriend procedures) and the setup defining every embedded
/// font once for the whole document.
fn push_prolog_and_setup(fonts: &[fonts::PsFontProgram], out: &mut String) {
    out.push_str("%%BeginProlog\n");
    if !fonts.is_empty() {
        out.push_str("%%BeginResource: procset WellfriendFonts\n");
        out.push_str(fonts::PS_FONT_PROLOG);
        out.push_str("%%EndResource\n");
    }
    out.push_str("%%EndProlog\n");
    out.push_str("%%BeginSetup\n");
    fonts::write_font_definitions(fonts, out);
    out.push_str("%%EndSetup\n");
}

/// Accumulates PostScript operators for one page body and emits it wrapped in a
/// `gsave` + coordinate-flip prologue and a matching `grestore`.
struct PsSink {
    height: u32,
    body: String,
    points_per_pixel: f64,
    image_filter: PsImageFilter,
    /// Embedded fonts and page-local re-encodings used by the page's text.
    text: PsPageText,
    /// Font instance and matrix last selected with `wfsf`, while no
    /// `grestore` can have restored an earlier font.
    current_font: Option<(usize, [f64; 4])>,
}

impl PsSink {
    fn new(_width: u32, height: u32) -> Self {
        Self {
            height,
            body: String::new(),
            points_per_pixel: 1.0,
            image_filter: PsImageFilter::Flate,
            text: PsPageText::default(),
            current_font: None,
        }
    }

    fn with_image_filter(mut self, image_filter: PsImageFilter) -> Self {
        self.image_filter = image_filter;
        self
    }

    fn with_points_per_pixel(mut self, points_per_pixel: f64) -> Self {
        if points_per_pixel.is_finite() && points_per_pixel > 0.0 {
            self.points_per_pixel = points_per_pixel;
        }
        self
    }

    fn push_line(&mut self, line: &str) {
        if line.starts_with("grestore") {
            self.current_font = None;
        }
        self.body.push_str(line);
        self.body.push('\n');
    }

    /// Append the device-space path `d` as PostScript path-construction
    /// operators (`moveto`/`lineto`/`closepath`).
    fn append_path(&mut self, flat: &FlatPath) {
        for (sp, closed) in flat.subpaths.iter().zip(flat.closed.iter()) {
            let mut iter = sp.iter();
            let Some(first) = iter.next() else { continue };
            self.body
                .push_str(&format!("{:.2} {:.2} moveto\n", first.0, first.1));
            for p in iter {
                self.body
                    .push_str(&format!("{:.2} {:.2} lineto\n", p.0, p.1));
            }
            if *closed {
                self.body.push_str("closepath\n");
            }
        }
    }

    /// Append a device-space path keeping its Bézier segments as `curveto`
    /// (`pdftops` behaviour) instead of a flattened polyline.
    fn append_curve_path(&mut self, path: &Path) {
        use std::fmt::Write as _;
        let body = &mut self.body;
        for seg in &path.segments {
            match *seg {
                PathSegment::MoveTo(x, y) => {
                    let _ = writeln!(body, "{} {} moveto", PsNum(x), PsNum(y));
                }
                PathSegment::LineTo(x, y) => {
                    let _ = writeln!(body, "{} {} lineto", PsNum(x), PsNum(y));
                }
                PathSegment::CubicTo {
                    cp1x,
                    cp1y,
                    cp2x,
                    cp2y,
                    x,
                    y,
                } => {
                    let _ = writeln!(
                        body,
                        "{} {} {} {} {} {} curveto",
                        PsNum(cp1x),
                        PsNum(cp1y),
                        PsNum(cp2x),
                        PsNum(cp2y),
                        PsNum(x),
                        PsNum(y)
                    );
                }
                PathSegment::ClosePath => body.push_str("closepath\n"),
            }
        }
    }

    /// Append the geometry of a vector path: curves when every point maps to
    /// a finite device coordinate, otherwise the flattened polyline.
    fn append_device_path(&mut self, path: &DevicePath) {
        match path {
            DevicePath::Curves(path) => self.append_curve_path(path),
            DevicePath::Flat(flat) => self.append_path(flat),
        }
    }

    /// Emit a whole-page raster image using `colorimage`. The pixel data is
    /// 3-channel RGB, row-major top-to-bottom (image space). Because the page
    /// prologue already flips to top-left/y-down device space, the image is
    /// drawn with the standard top-down `[w 0 0 h 0 0]` matrix mapped into a
    /// unit square positioned at the page extent.
    fn emit_raster_image(&mut self, rgb: &[u8], w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        // Position the unit image square over the full device-space page box.
        self.push_line(&format!("{w} {h} scale"));
        // The samples are ASCII85-encoded (25% overhead instead of 100% for
        // hex) and read through an `/ASCII85Decode` filter named `picstr`.
        self.push_ascii85_image_data(
            "picstr",
            &format!("{w} {h} 8 [{w} 0 0 {h} 0 0] picstr false 3 colorimage"),
            rgb,
        );
    }

    /// Emit an image operator reading its samples from an `/ASCII85Decode`
    /// filter over `currentfile`, followed by the encoded data.
    ///
    /// The filter is bound to `source` and the image operator runs inside an
    /// executed procedure that flushes the filter afterwards, so the `~>`
    /// end-of-data marker is always consumed before the interpreter resumes
    /// scanning the page body.
    fn push_ascii85_image_data(&mut self, source: &str, image_operator: &str, data: &[u8]) {
        // Samples are compressed (Flate at LanguageLevel 3, RunLength at
        // Level 2) inside the ASCII85 text wrapper. `source` reads the
        // decompressed samples; `{source}a85` is the ASCII85 layer, flushed
        // last so its `~>` end marker is consumed whatever the compression
        // filter read ahead.
        let (filter, compressed) = match self.image_filter {
            PsImageFilter::Flate => ("FlateDecode", zlib_compress(data)),
            PsImageFilter::RunLength => ("RunLengthDecode", run_length_encode(data)),
        };
        self.push_line(&format!(
            "/{source}a85 currentfile /ASCII85Decode filter def"
        ));
        self.push_line(&format!("/{source} {source}a85 /{filter} filter def"));
        self.push_line(&format!(
            "{{ {image_operator} {source} flushfile {source}a85 flushfile }} exec"
        ));
        self.body.push_str(&ascii85_encode(&compressed));
        self.body.push('\n');
    }

    /// Finish the page body: wrap it with `gsave`, the top-left device-space
    /// coordinate flip, and `grestore`.
    fn finish(self) -> String {
        let mut out = String::new();
        out.push_str("gsave\n");
        // The body draws in device pixels; scale them to PostScript points.
        if (self.points_per_pixel - 1.0).abs() > 1e-12 {
            let s = ps_number(self.points_per_pixel);
            out.push_str(&format!("{s} {s} scale\n"));
        }
        // Map PostScript bottom-left/y-up to our top-left/y-down device space.
        out.push_str(&format!("0 {} translate\n", self.height));
        out.push_str("1 -1 scale\n");
        // Page-local re-encodings of the embedded fonts the text uses.
        out.push_str(&self.text.setup());
        out.push_str(&self.body);
        out.push_str("grestore\n");
        out
    }
}

/// PostScript sibling of `RenderState`/`SvgRenderState`: same interpretation,
/// PostScript emission.
struct PsRenderState<'a> {
    /// Font programs already resolved for the current `resources` scope, by
    /// font resource name. Cleared whenever `resources` is replaced so a name
    /// never resolves against a different Form/page resource dictionary.
    font_bytes_cache: std::collections::HashMap<String, Option<std::sync::Arc<[u8]>>>,
    font_resolver_cache:
        std::collections::HashMap<String, std::sync::Arc<crate::fonts::FontResolver>>,
    engine: &'a ContentEngine,
    resources: PageResources,
    page_resources: PageResources,
    viewport: Viewport,
    gs: GraphicsState,
    path: Path,
    pending_clip: Option<FillRule>,
    sink: &'a mut PsSink,
    /// Names of Image XObjects eligible for regional embedding.
    regional_image_names: Vec<String>,
    /// Remaining inline image sequences eligible for regional embedding.
    regional_inline_image_count: usize,
    /// Names of vector-safe Form XObjects eligible for native replay.
    regional_form_names: Vec<String>,
    /// Names of simple axial/radial shading resources eligible for native shfill.
    regional_shading_names: Vec<String>,
    pending_inline_params: Option<Vec<Operand>>,
    form_depth: usize,
    form_object_stack: Vec<(u32, u16)>,
    text_clip_path: Option<FlatPath>,
    fatal_error: Option<WellfriendError>,
    /// Draw ordinary filled text with embedded fonts ([`PsOptions::embed_fonts`]).
    embed_fonts: bool,
    /// Embedding decision per font resource name for the current `resources`
    /// scope (`None`: the program cannot be embedded; outlines are drawn).
    text_font_cache: std::collections::HashMap<String, Option<std::rc::Rc<PsTextFont>>>,
    /// Glyphs of the current text-showing operator not yet written.
    text_run: Option<PsTextRun>,
}

/// An embeddable font program as the page's text uses it.
struct PsTextFont {
    base_name: String,
    kind: PsFontKind,
    /// Glyph count of a TrueType program (valid glyph ids are below it).
    num_glyphs: u16,
}

/// Consecutive glyphs sharing one font instance, matrix and colour, written
/// as a single `xshow` / `xyshow`.
struct PsTextRun {
    instance: usize,
    /// Em-space to device-space linear matrix (`makefont` matrix).
    matrix: [f64; 4],
    color: RenderColor,
    codes: Vec<u8>,
    /// Device-space glyph origins.
    origins: Vec<(f64, f64)>,
}

fn same_font_matrix(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0))
}

impl PsRenderState<'_> {
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
                self.fill_path(rule, true);
                self.stroke_path();
                self.apply_pending_clip();
                self.finish_path();
            }
            "b" | "b*" => {
                self.path.close();
                let rule = if op.operator == "b*" {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                };
                self.fill_path(rule, true);
                self.stroke_path();
                self.apply_pending_clip();
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
                self.sink.push_line("gsave");
                self.gs.process(op);
            }
            "Q" => {
                self.gs.process(op);
                self.sink.push_line("grestore");
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
            // Safety: "Do" | "sh" | "gs" are classified by vector_fallback before dispatch.
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
                        } else if crate::content::state::ext_g_state_has_non_identity_transfer(dict)
                        {
                            // The vector PostScript sink emits no transfer curves.
                            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                                "{label} non-identity transfer function is not representable in PostScript output"
                            )));
                        }
                    }
                }
                self.gs.process(op);
            }
            "ID" => {
                if self.regional_inline_image_count > 0 {
                    self.pending_inline_params = Some(op.operands.to_vec());
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
    /// XObject through the engine's decoder and embeds it as a bounded
    /// `colorimage` region at its correct device-space position.
    fn emit_regional_image(&mut self, name: &str) -> Result<()> {
        if ps_alpha_is_fully_transparent(self.gs.fill_alpha as f32) {
            return Ok(());
        }

        let placement = match image_device_placement(&self.gs, &self.viewport) {
            Some(placement) => placement,
            None => {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "regional PS image /{name} placement is unresolvable"
                )))
            }
        };

        let (obj_num, gen_num) = match self.resources.xobjects.get(name) {
            Some(&(o, g)) => (o, g),
            None => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "regional PS image /{name} is missing from XObject resources"
                )))
            }
        };

        let reader = self.engine.document().reader();
        let (dict, encoded) = match reader.get_object(obj_num, gen_num) {
            Ok(crate::object::PdfObject::Stream { dict, raw }) => (dict, raw),
            Ok(other) => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "regional PS image /{name} resolved to {}, expected stream",
                    other.variant_name()
                )))
            }
            Err(err) => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "regional PS image /{name} failed to resolve: {err}"
                )))
            }
        };

        let context = format!("regional PS image /{name}");
        let is_mask = regional_image_bool(&dict, "ImageMask", "IM", &context)?;
        let _ = regional_image_bool(&dict, "Interpolate", "I", &context)?;
        let image_ref = regional_image_reference(name, obj_num, gen_num, &dict, is_mask, reader)?;
        if !is_mask
            && self.emit_dct_passthrough_region(
                &dict,
                &encoded,
                placement.transform,
                placement.bounds,
                &context,
            )?
        {
            return Ok(());
        }
        if !is_mask
            && self.emit_ccitt_passthrough_region(
                &dict,
                &encoded,
                placement.transform,
                placement.bounds,
                &context,
            )?
        {
            return Ok(());
        }
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
                    Some(color_space_obj),
                )
            }
            None => self.engine.decode_image(&image_ref),
        }
        .map_err(|err| {
            WellfriendError::UnsupportedFeature(format!(
                "regional PS image /{name} decode failed: {err}"
            ))
        })?;
        if is_mask {
            if self.emit_pattern_stencil_mask_region(
                &raw,
                placement.transform,
                stencil_mask_paints_ones(&dict)?,
                "image-mask",
            )? {
                return Ok(());
            }
            self.emit_stencil_mask_region(
                &raw,
                placement.transform,
                stencil_mask_paints_ones(&dict)?,
            )?;
            return Ok(());
        }

        let context = format!("regional PS image /{name}");
        // A soft-masked image keeps its transparency; fractional alpha then
        // refuses regional colorimage (PostScript has no soft masks), so the
        // page takes the raster fallback as with `pdftops`.
        let raw =
            crate::render::svg::apply_image_soft_mask(&dict, &image_ref, raw, reader, &context)?;
        self.emit_colorimage_region(
            &raw,
            placement.transform,
            placement.bounds,
            &context,
            "image-xobject",
        )
    }

    /// Handle a complete `BI`/`ID`/`EI` inline image sequence approved by the
    /// shared regional fallback classifier.
    fn emit_inline_image_region(&mut self, params: &[Operand], data: &[u8]) -> Result<()> {
        if ps_alpha_is_fully_transparent(self.gs.fill_alpha as f32) {
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
                "regional PS inline image decode failed: {err}"
            ))
        })?;
        if region.is_mask {
            if self.emit_pattern_stencil_mask_region(
                &region.raw,
                region.placement.transform,
                region.mask_paints_ones,
                "inline-image-mask",
            )? {
                return Ok(());
            }
            self.emit_stencil_mask_region(
                &region.raw,
                region.placement.transform,
                region.mask_paints_ones,
            )?;
            return Ok(());
        }
        let raw = region.raw;
        self.emit_colorimage_region(
            &raw,
            region.placement.transform,
            region.placement.bounds,
            "regional PS inline image",
            "inline-image",
        )
    }

    fn emit_colorimage_region(
        &mut self,
        raw: &RawImage,
        transform: [f64; 6],
        bounds: [f64; 4],
        context: &str,
        region_kind: &str,
    ) -> Result<()> {
        ensure_regional_raw_image(raw, context)?;
        let alpha_clip = match regional_rgba_alpha_state(raw) {
            Some(RegionalRgbaAlphaState::FullyTransparent) => return Ok(()),
            Some(RegionalRgbaAlphaState::Binary) => Some(regional_binary_alpha_clip_path(
                raw,
                transform,
                self.gs.path_flatness_tolerance(),
                context,
            )?),
            Some(RegionalRgbaAlphaState::Fractional) => {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "{context}: regional PS colorimage cannot represent fractional alpha"
                )))
            }
            Some(RegionalRgbaAlphaState::Opaque) | None => None,
        };
        let allow_binary_alpha = alpha_clip.is_some();
        let rgb = regional_colorimage_rgb_with_alpha_policy(raw, context, allow_binary_alpha)?;
        let iw = raw.width;
        let ih = raw.height;

        self.sink.push_line(&ps_region_comment(region_kind, bounds));
        self.sink.push_line("gsave");
        if let Some(flat) = alpha_clip.as_ref() {
            self.sink.push_line("newpath");
            self.sink.append_path(flat);
            self.sink.push_line("clip");
            self.sink.push_line("newpath");
        }
        self.sink.push_line(&ps_concat_matrix(transform));
        self.sink.push_ascii85_image_data(
            "regionpicstr",
            &format!("{iw} {ih} 8 [{iw} 0 0 {ih} 0 0] regionpicstr false 3 colorimage"),
            &rgb,
        );
        self.sink.push_line("grestore");
        Ok(())
    }

    /// Pass an opaque DCT-encoded (JPEG) image XObject through to the
    /// PostScript output unchanged, decoded by the printer's `/DCTDecode`
    /// filter instead of being expanded to uncompressed samples. Applies to
    /// 8-bit DeviceGray, DeviceRGB and DeviceCMYK JPEGs without /Decode,
    /// /SMask or /Mask whose frame component count matches the colour
    /// space. Returns `false` (emitting nothing) when the image does not
    /// qualify.
    fn emit_dct_passthrough_region(
        &mut self,
        dict: &PdfDictionary,
        encoded: &[u8],
        transform: [f64; 6],
        bounds: [f64; 4],
        context: &str,
    ) -> Result<bool> {
        if !ps_alpha_is_opaque(self.gs.fill_alpha as f32)
            || dict.contains_key("SMask")
            || dict.contains_key("Mask")
            || dict.contains_key("Decode")
            || dict.contains_key("D")
            || dict.contains_key("SMaskInData")
        {
            return Ok(false);
        }
        let filters = extract_image_filter_names(dict, context)?;
        if !matches!(filters.as_slice(), [filter] if filter == "DCTDecode" || filter == "DCT") {
            return Ok(false);
        }
        if dict
            .get("DecodeParms")
            .or_else(|| dict.get("DP"))
            .is_some_and(|params| !matches!(params, PdfObject::Null))
        {
            return Ok(false);
        }
        let bits = dict
            .get("BitsPerComponent")
            .or_else(|| dict.get("BPC"))
            .and_then(PdfObject::as_number);
        if bits.is_some_and(|bits| bits != 8.0) {
            return Ok(false);
        }
        let reader = self.engine.document().reader();
        let Ok(color_space) = extract_image_color_space_name(dict, reader, context) else {
            return Ok(false);
        };
        let color_space = match color_space.as_str() {
            "DeviceGray" | "DeviceRGB" | "DeviceCMYK" => color_space,
            name => match self.resources.color_spaces.get(name).map(|object| {
                reader
                    .resolve(object.clone())
                    .unwrap_or_else(|_| object.clone())
            }) {
                Some(PdfObject::Name(resolved)) => canonical_image_color_space_name(&resolved),
                _ => return Ok(false),
            },
        };
        let (components, decode) = match color_space.as_str() {
            "DeviceGray" => (1, "[0 1]"),
            "DeviceRGB" => (3, "[0 1 0 1 0 1]"),
            "DeviceCMYK" => (4, "[0 1 0 1 0 1 0 1]"),
            _ => return Ok(false),
        };
        if jpeg_frame_components(encoded) != Some(components) {
            return Ok(false);
        }
        let context_width = regional_image_positive_u32(dict, "Width", "W", context)?;
        let context_height = regional_image_positive_u32(dict, "Height", "H", context)?;
        let (iw, ih) = (context_width, context_height);

        self.sink
            .push_line(&ps_region_comment("image-xobject", bounds));
        self.sink
            .push_line("% WellfriendImagePassthrough filter=DCTDecode");
        self.sink.push_line("gsave");
        self.sink.push_line(&ps_concat_matrix(transform));
        self.sink
            .push_line(&format!("/{color_space} setcolorspace"));
        self.sink
            .push_line("/regionpicstr currentfile /ASCII85Decode filter def");
        self.sink
            .push_line("/regiondctsrc regionpicstr /DCTDecode filter def");
        self.sink.push_line(&format!(
            "{{ << /ImageType 1 /Width {iw} /Height {ih} /BitsPerComponent 8 /Decode {decode} /ImageMatrix [{iw} 0 0 {ih} 0 0] /DataSource regiondctsrc >> image regionpicstr flushfile }} exec"
        ));
        self.sink.body.push_str(&ascii85_encode(encoded));
        self.sink.body.push('\n');
        self.sink.push_line("grestore");
        Ok(true)
    }

    /// Pass a 1-bit DeviceGray CCITT Group 3/4 image XObject through to the
    /// PostScript `/CCITTFaxDecode` filter (LanguageLevel 2) with the PDF's
    /// own decode parameters, instead of expanding it to samples. Applies to
    /// opaque images without /Decode, /SMask or /Mask whose /Columns match
    /// the width. Returns `false` (emitting nothing) when the image does not
    /// qualify.
    fn emit_ccitt_passthrough_region(
        &mut self,
        dict: &PdfDictionary,
        encoded: &[u8],
        transform: [f64; 6],
        bounds: [f64; 4],
        context: &str,
    ) -> Result<bool> {
        if !ps_alpha_is_opaque(self.gs.fill_alpha as f32)
            || dict.contains_key("SMask")
            || dict.contains_key("Mask")
            || dict.contains_key("Decode")
            || dict.contains_key("D")
        {
            return Ok(false);
        }
        let filters = extract_image_filter_names(dict, context)?;
        if !matches!(filters.as_slice(), [filter] if filter == "CCITTFaxDecode" || filter == "CCF")
        {
            return Ok(false);
        }
        if dict
            .get("BitsPerComponent")
            .or_else(|| dict.get("BPC"))
            .and_then(PdfObject::as_number)
            .is_some_and(|bits| bits != 1.0)
        {
            return Ok(false);
        }
        let reader = self.engine.document().reader();
        let Ok(color_space) = extract_image_color_space_name(dict, reader, context) else {
            return Ok(false);
        };
        let color_space = match color_space.as_str() {
            "DeviceGray" => color_space,
            name => match self.resources.color_spaces.get(name).map(|object| {
                reader
                    .resolve(object.clone())
                    .unwrap_or_else(|_| object.clone())
            }) {
                Some(PdfObject::Name(resolved)) => canonical_image_color_space_name(&resolved),
                _ => return Ok(false),
            },
        };
        if color_space != "DeviceGray" {
            return Ok(false);
        }
        let iw = regional_image_positive_u32(dict, "Width", "W", context)?;
        let ih = regional_image_positive_u32(dict, "Height", "H", context)?;
        let params = match dict.get("DecodeParms").or_else(|| dict.get("DP")) {
            None | Some(PdfObject::Null) => PdfDictionary::empty(),
            Some(object) => match reader.resolve(object.clone()) {
                Ok(PdfObject::Dictionary(params)) => params,
                Ok(PdfObject::Array(items)) if items.len() == 1 => {
                    match reader.resolve(items[0].clone()) {
                        Ok(PdfObject::Dictionary(params)) => params,
                        Ok(PdfObject::Null) => PdfDictionary::empty(),
                        _ => return Ok(false),
                    }
                }
                _ => return Ok(false),
            },
        };
        let mut ps_params = String::from("<<");
        let mut columns = 1728.0;
        for (key, value) in params.iter() {
            let value = reader
                .resolve(value.clone())
                .unwrap_or_else(|_| value.clone());
            match (key.as_str(), value) {
                (
                    "K" | "Columns" | "Rows" | "DamagedRowsBeforeError",
                    PdfObject::Integer(number),
                ) => {
                    if key == "Columns" {
                        columns = number as f64;
                    }
                    ps_params.push_str(&format!(" /{key} {number}"));
                }
                (
                    "EndOfLine" | "EncodedByteAlign" | "EndOfBlock" | "BlackIs1",
                    PdfObject::Boolean(flag),
                ) => ps_params.push_str(&format!(" /{key} {flag}")),
                _ => return Ok(false),
            }
        }
        ps_params.push_str(" >>");
        if columns != f64::from(iw) {
            return Ok(false);
        }

        self.sink
            .push_line(&ps_region_comment("image-xobject", bounds));
        self.sink
            .push_line("% WellfriendImagePassthrough filter=CCITTFaxDecode");
        self.sink.push_line("gsave");
        self.sink.push_line(&ps_concat_matrix(transform));
        self.sink.push_line("/DeviceGray setcolorspace");
        self.sink
            .push_line("/regionpicstr currentfile /ASCII85Decode filter def");
        self.sink.push_line(&format!(
            "/regionfaxsrc regionpicstr {ps_params} /CCITTFaxDecode filter def"
        ));
        self.sink.push_line(&format!(
            "{{ << /ImageType 1 /Width {iw} /Height {ih} /BitsPerComponent 1 /Decode [0 1] /ImageMatrix [{iw} 0 0 {ih} 0 0] /DataSource regionfaxsrc >> image regionpicstr flushfile }} exec"
        ));
        self.sink.body.push_str(&ascii85_encode(encoded));
        self.sink.body.push('\n');
        self.sink.push_line("grestore");
        Ok(true)
    }

    fn emit_stencil_mask_region(
        &mut self,
        raw: &RawImage,
        transform: [f64; 6],
        paint_ones: bool,
    ) -> Result<()> {
        let iw = raw.width;
        let ih = raw.height;
        if iw == 0 || ih == 0 {
            return Err(WellfriendError::MalformedPdf(
                "regional PS stencil mask has zero dimensions".to_string(),
            ));
        }
        ensure_regional_stencil_mask(raw, "regional PS stencil mask")?;
        let row_bytes = (iw as usize).div_ceil(8);
        let Some((color, alpha)) =
            self.current_fill_color_or_fatal("PostScript stencil-mask fill color")
        else {
            return Ok(());
        };
        if ps_alpha_is_fully_transparent(alpha) {
            return Ok(());
        }
        self.sink.push_line("gsave");
        self.sink.push_line(&ps_concat_matrix(transform));
        self.emit_setcolor(color, alpha);
        let channels = raw.channels as usize;
        let mut packed_rows = Vec::with_capacity(row_bytes * ih as usize);
        for row in 0..ih as usize {
            for byte_idx in 0..row_bytes {
                let mut packed = 0u8;
                for bit in 0..8 {
                    let x = byte_idx * 8 + bit;
                    if x >= iw as usize {
                        continue;
                    }
                    let offset = (row * iw as usize + x) * channels;
                    let sample = raw.pixels[offset];
                    if inline_mask_sample_paints(sample, paint_ones) {
                        packed |= 0x80 >> bit;
                    }
                }
                packed_rows.push(packed);
            }
        }
        self.sink.push_ascii85_image_data(
            "regionmaskstr",
            &format!("{iw} {ih} true [{iw} 0 0 {ih} 0 0] regionmaskstr imagemask"),
            &packed_rows,
        );
        self.sink.push_line("grestore");
        Ok(())
    }

    fn emit_pattern_stencil_mask_region(
        &mut self,
        raw: &RawImage,
        transform: [f64; 6],
        paint_ones: bool,
        stage: &str,
    ) -> Result<bool> {
        let Some(pattern_name) = self.gs.fill_pattern_name.clone() else {
            return Ok(false);
        };
        if ps_alpha_is_fully_transparent(self.gs.fill_alpha as f32) {
            return Ok(true);
        }
        let flat = self.stencil_mask_clip_path(raw, transform, paint_ones, stage)?;
        if self.emit_tiling_pattern_clip(
            Some(pattern_name.clone()),
            &flat,
            FillRule::NonZero,
            self.gs.fill_alpha as f32,
            false,
            stage,
        )? {
            return Ok(true);
        }
        self.emit_shading_pattern_clip(Some(pattern_name), &flat, FillRule::NonZero, stage);
        Ok(true)
    }

    fn stencil_mask_clip_path(
        &self,
        raw: &RawImage,
        transform: [f64; 6],
        paint_ones: bool,
        stage: &str,
    ) -> Result<FlatPath> {
        ensure_regional_stencil_mask(raw, "regional PS stencil mask")?;
        let iw = raw.width as usize;
        let ih = raw.height as usize;
        let channels = raw.channels as usize;
        let mut painted = 0usize;
        let mut path = Path::new();
        for row in 0..ih {
            for col in 0..iw {
                let sample = raw.pixels[(row * iw + col) * channels];
                if !inline_mask_sample_paints(sample, paint_ones) {
                    continue;
                }
                painted = painted.saturating_add(1);
                if painted > MAX_PATTERN_STENCIL_CLIP_RECTS {
                    return Err(WellfriendError::UnsupportedFeature(format!(
                        "regional PS pattern-painted stencil {stage} has {painted} painted cells, cap is {MAX_PATTERN_STENCIL_CLIP_RECTS}"
                    )));
                }
                let x = col as f64 / iw as f64;
                let y = row as f64 / ih as f64;
                path.rect(x, y, 1.0 / iw as f64, 1.0 / ih as f64);
            }
        }
        Ok(flatten_path_device_transform(
            &path,
            &Transform2D::from_array(transform),
            self.gs.path_flatness_tolerance(),
        ))
    }

    fn emit_shading(&mut self, name: &str) {
        if ps_alpha_is_fully_transparent(self.gs.fill_alpha as f32) {
            return;
        }
        let reader = self.engine.document().reader();
        let Some(shading) =
            load_vector_shading_for_postscript_output(&self.resources, Some(reader), name)
        else {
            return;
        };
        self.emit_vector_shading(shading, self.ctm());
    }

    fn emit_vector_shading(&mut self, shading: VectorShading, ctm: Transform2D) {
        let bbox = shading.bbox();
        match shading {
            VectorShading::Axial(shading) => {
                let [x0, y0, x1, y1] = shading.coords;
                let (dx0, dy0) = self.device_point_with(ctm, x0, y0);
                let (dx1, dy1) = self.device_point_with(ctm, x1, y1);
                let extend = ps_bool_pair(shading.extend);
                let (color_space, function) = ps_shading_color_space_and_function(
                    shading.ps_color_space.as_ref(),
                    shading.ps_function.as_ref(),
                    &shading.stops,
                );
                let domain = ps_shading_domain_clause(shading.domain, shading.ps_function.as_ref());
                self.sink.push_line("gsave");
                self.apply_shading_bbox_clip(bbox, ctm);
                self.sink.push_line(&format!(
                    "<< /ShadingType 2 /ColorSpace {color_space} /Coords [{dx0:.3} {dy0:.3} {dx1:.3} {dy1:.3}] /Extend {extend}{domain} /Function {function} >> shfill"
                ));
                self.sink.push_line("grestore");
            }
            VectorShading::Radial(shading) => {
                let [x0, y0, r0, x1, y1, r1] = shading.coords;
                let full = ctm.concat(&self.viewport.to_transform());
                if !transform_is_finite_and_invertible(full)
                    || r0 < 0.0
                    || r1 < 0.0
                    || (r1 - r0).abs() <= 1e-9
                {
                    return;
                }
                let (color_space, function) = ps_shading_color_space_and_function(
                    shading.ps_color_space.as_ref(),
                    shading.ps_function.as_ref(),
                    &shading.stops,
                );
                let extend = ps_bool_pair(shading.extend);
                let domain = ps_shading_domain_clause(shading.domain, shading.ps_function.as_ref());
                self.sink.push_line("gsave");
                self.apply_shading_bbox_clip(bbox, ctm);
                if transform_preserves_circles(full) {
                    let (dx0, dy0) = self.device_point_with(ctm, x0, y0);
                    let (dx1, dy1) = self.device_point_with(ctm, x1, y1);
                    let dr0 = self.device_radius_with(ctm, r0);
                    let dr1 = self.device_radius_with(ctm, r1);
                    if dr0 < 0.0 || dr1 < 0.0 || (dr1 - dr0).abs() <= 1e-9 {
                        self.sink.push_line("grestore");
                        return;
                    }
                    self.sink.push_line(&format!(
                        "<< /ShadingType 3 /ColorSpace {color_space} /Coords [{dx0:.3} {dy0:.3} {dr0:.3} {dx1:.3} {dy1:.3} {dr1:.3}] /Extend {extend}{domain} /Function {function} >> shfill"
                    ));
                } else {
                    self.sink.push_line(&ps_concat_matrix(full.to_array()));
                    self.sink.push_line(&format!(
                        "<< /ShadingType 3 /ColorSpace {color_space} /Coords [{x0:.3} {y0:.3} {r0:.3} {x1:.3} {y1:.3} {r1:.3}] /Extend {extend}{domain} /Function {function} >> shfill"
                    ));
                }
                self.sink.push_line("grestore");
            }
        }
    }

    fn apply_shading_bbox_clip(&mut self, bbox: Option<[f64; 4]>, ctm: Transform2D) {
        let Some(bbox) = bbox else {
            return;
        };
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
            &ctm,
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        self.sink.push_line("newpath");
        self.sink.append_path(&flat);
        self.sink.push_line("clip");
        self.sink.push_line("newpath");
    }

    fn device_radius_with(&self, ctm: Transform2D, radius: f64) -> f64 {
        let (x0, y0) = self.device_point_with(ctm, 0.0, 0.0);
        let (x1, y1) = self.device_point_with(ctm, radius, 0.0);
        ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt()
    }

    /// Replay a vector-safe Form XObject as native PostScript operators. The
    /// shared classifier rejects semantic transparency groups, unsafe nested
    /// resources, recursive programs, and non-vector-safe constructs before this
    /// path is used.
    fn emit_form_xobject(&mut self, name: &str) {
        if self.form_depth >= MAX_VECTOR_FORM_DEPTH {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                "PostScript Form /{name} nesting depth exceeded"
            )));
            return;
        }
        let reader = self.engine.document().reader();
        let Some(program) = load_vector_form_program(
            &self.resources,
            &self.page_resources,
            reader,
            name,
            &self.gs,
        ) else {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                "PostScript Form /{name} cannot be resolved in its resource scope"
            )));
            return;
        };
        let form_key = (program.object_number, program.generation_number);
        if self.form_object_stack.contains(&form_key) {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                "PostScript Form /{name} recursion cycle"
            )));
            return;
        }

        let saved_gs = self.gs.clone();
        let saved_resources = self.resources.clone();
        let saved_path = std::mem::replace(&mut self.path, Path::new());
        let saved_pending_clip = self.pending_clip.take();
        let saved_text_clip = self.text_clip_path.take();
        let saved_image_names = self.regional_image_names.clone();
        let saved_inline_count = self.regional_inline_image_count;
        let saved_form_names = self.regional_form_names.clone();
        let saved_shading_names = self.regional_shading_names.clone();
        let saved_pending_inline = self.pending_inline_params.take();

        self.resources = program.resources.clone();
        self.font_bytes_cache.clear();
        self.font_resolver_cache.clear();
        self.text_font_cache.clear();
        self.gs = program.inherited_gs.clone();
        let form_t = Transform2D::from(program.form_matrix);
        let current_t = Transform2D::from(saved_gs.ctm);
        self.gs.ctm = form_t.concat(&current_t).to_array();

        self.form_depth += 1;
        self.form_object_stack.push(form_key);

        let decision = classify_scoped_postscript_vector_output(
            &program.ops,
            &self.resources,
            &self.page_resources,
            self.viewport.scale,
            reader,
            self.gs.clone(),
            &mut self.form_object_stack,
        );

        if !matches!(decision, VectorFallbackDecision::WholePageRaster { .. }) {
            self.sink.push_line("gsave");
            if let Some(bbox) = program.bbox {
                self.apply_form_bbox_clip(bbox);
            }
            match decision {
                VectorFallbackDecision::PureVector => {
                    self.regional_image_names.clear();
                    self.regional_inline_image_count = 0;
                    self.regional_form_names.clear();
                    self.regional_shading_names.clear();
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
                }
                VectorFallbackDecision::WholePageRaster { .. } => {}
            }
            self.run(&program.ops);
            self.sink.push_line("grestore");
        } else if let VectorFallbackDecision::WholePageRaster { reason } = decision {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                "PostScript Form /{name} failed scoped vector validation: {reason}"
            )));
        }

        self.form_object_stack.pop();
        self.form_depth = self.form_depth.saturating_sub(1);
        self.gs = saved_gs;
        self.resources = saved_resources;
        self.font_bytes_cache.clear();
        self.font_resolver_cache.clear();
        self.text_font_cache.clear();
        self.path = saved_path;
        self.pending_clip = saved_pending_clip;
        self.text_clip_path = saved_text_clip;
        self.regional_image_names = saved_image_names;
        self.regional_inline_image_count = saved_inline_count;
        self.regional_form_names = saved_form_names;
        self.regional_shading_names = saved_shading_names;
        self.pending_inline_params = saved_pending_inline;
    }

    fn apply_form_bbox_clip(&mut self, bbox: [f64; 4]) {
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
        self.sink.push_line("newpath");
        self.sink.append_path(&flat);
        self.sink.push_line("clip");
        self.sink.push_line("newpath");
    }

    /// Apply a pending `W`/`W*` clip: emit the path and `clip`/`eoclip`. Because
    /// PDF clips compose with `gsave`/`grestore` (which we map to PDF `q`/`Q`),
    /// emitting the PostScript clip operator at the same point reproduces the
    /// PDF clip-stack semantics exactly.
    fn apply_pending_clip(&mut self) {
        if let Some(rule) = self.pending_clip.take() {
            if self.path.is_empty() {
                return;
            }
            let ctm = self.ctm();
            let device = DevicePath::new(
                &self.path,
                &ctm,
                &self.viewport,
                self.gs.path_flatness_tolerance(),
            );
            self.sink.append_device_path(&device);
            match rule {
                FillRule::EvenOdd => self.sink.push_line("eoclip"),
                FillRule::NonZero => self.sink.push_line("clip"),
            }
            // `clip` leaves the path defined; clear it so the current path does
            // not get re-used by a following paint operator.
            self.sink.push_line("newpath");
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
        self.sink.append_path(&flat);
        self.sink.push_line("clip");
        self.sink.push_line("newpath");
    }

    fn stroke_and_clear(&mut self) {
        self.stroke_path();
        self.apply_pending_clip();
        self.finish_path();
    }

    fn fill_and_clear(&mut self, rule: FillRule) {
        self.fill_path(rule, false);
        self.apply_pending_clip();
        self.finish_path();
    }

    fn finish_path(&mut self) {
        self.path.clear();
    }

    /// Emit a fill of the current path. When `keep_path` is true (the `B`/`b`
    /// fill-then-stroke operators) the path is preserved via `gsave`/`grestore`
    /// so the following stroke uses the same geometry.
    fn fill_path(&mut self, rule: FillRule, keep_path: bool) {
        if self.path.is_empty() {
            return;
        }
        if ps_alpha_is_fully_transparent(self.gs.fill_alpha as f32) {
            return;
        }
        let ctm = self.ctm();
        if self.gs.fill_color_space.is_pattern() {
            let flat = flatten_path(
                &self.path,
                &ctm,
                &self.viewport,
                self.gs.path_flatness_tolerance(),
            );
            if flat.subpaths.iter().all(|s| s.is_empty()) {
                return;
            }
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
            self.emit_shading_pattern_clip(self.gs.fill_pattern_name.clone(), &flat, rule, "fill");
            return;
        }
        let device = DevicePath::new(
            &self.path,
            &ctm,
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        if device.is_empty() {
            return;
        }
        let Some((color, alpha)) = self.current_fill_color_or_fatal("PostScript fill color") else {
            return;
        };
        if ps_alpha_is_fully_transparent(alpha) {
            return;
        }
        self.emit_setcolor(color, alpha);
        if keep_path {
            self.sink.push_line("gsave");
        }
        self.emit_flatness();
        self.sink.push_line("newpath");
        self.sink.append_device_path(&device);
        match rule {
            FillRule::EvenOdd => self.sink.push_line("eofill"),
            FillRule::NonZero => self.sink.push_line("fill"),
        }
        if keep_path {
            self.sink.push_line("grestore");
        }
    }

    fn stroke_path(&mut self) {
        if self.path.is_empty() {
            return;
        }
        if ps_alpha_is_fully_transparent(self.gs.stroke_alpha as f32) {
            return;
        }
        let ctm = self.ctm();
        if self.gs.stroke_color_space.is_pattern() {
            let flat = flatten_path(
                &self.path,
                &ctm,
                &self.viewport,
                self.gs.path_flatness_tolerance(),
            );
            if flat.subpaths.iter().all(|s| s.is_empty()) {
                return;
            }
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
                "stroke",
            );
            return;
        }
        let device = DevicePath::new(
            &self.path,
            &ctm,
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        if device.is_empty() {
            return;
        }
        let Some((color, alpha)) = self.current_stroke_color_or_fatal("PostScript stroke color")
        else {
            return;
        };
        if ps_alpha_is_fully_transparent(alpha) {
            return;
        }
        self.emit_setcolor(color, alpha);
        let width = self.device_line_width();
        self.sink.push_line(&format!("{width:.3} setlinewidth"));
        self.emit_line_style();
        self.emit_flatness();
        self.sink.push_line("newpath");
        self.sink.append_device_path(&device);
        self.sink.push_line("stroke");
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
                "regional PS tiling pattern {stage} requires an active pattern name"
            )));
        };
        let reader = self.engine.document().reader();
        let Some(program) = load_vector_tiling_pattern(&self.resources, reader, &pattern_name)
        else {
            return Ok(false);
        };
        if ps_alpha_is_fully_transparent(alpha) {
            return Ok(true);
        }
        if !ps_alpha_is_opaque(alpha) {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "regional PS tiling pattern {stage} cannot represent fractional alpha {alpha:.6}"
            )));
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
                        "regional PS uncolored tiling pattern {stage} /{pattern_name} requires finite gray, RGB, or CMYK caller color components"
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
                "regional PS tiling pattern {stage} /{pattern_name} requires {tile_count} visible cells, cap is {MAX_VECTOR_TILING_PATTERN_CELLS}"
            )));
        }

        self.sink.push_line("gsave");
        self.sink.push_line("newpath");
        self.sink.append_path(flat);
        match rule {
            FillRule::EvenOdd => self.sink.push_line("eoclip"),
            FillRule::NonZero => self.sink.push_line("clip"),
        }
        self.sink.push_line("newpath");
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
        self.sink.push_line("grestore");
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
        if ps_alpha_is_fully_transparent(alpha) {
            return;
        }
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
        self.emit_shading_pattern_clip(pattern_name, flat, rule, stage);
    }

    fn emit_tiling_pattern_tile(
        &mut self,
        program: &VectorTilingPatternProgram,
        tile_ctm: Transform2D,
        forced_color: Option<&Color>,
    ) {
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

        self.resources = program.resources.clone();
        self.font_bytes_cache.clear();
        self.font_resolver_cache.clear();
        self.text_font_cache.clear();
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
        let decision = classify_scoped_postscript_vector_output(
            &program.ops,
            &self.resources,
            &self.page_resources,
            self.viewport.scale,
            reader,
            self.gs.clone(),
            &mut self.form_object_stack,
        );
        let run_tile = match decision {
            VectorFallbackDecision::WholePageRaster { .. } => {
                self.record_fatal_error(WellfriendError::UnsupportedFeature(
                    "regional PS tiling pattern tile contains unsupported vector content"
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
            self.sink.push_line("gsave");
            self.apply_tiling_bbox_clip(program.bbox, tile_ctm);
            self.run(&program.ops);
            self.sink.push_line("grestore");
        }

        self.gs = saved_gs;
        self.resources = saved_resources;
        self.font_bytes_cache.clear();
        self.font_resolver_cache.clear();
        self.text_font_cache.clear();
        self.path = saved_path;
        self.pending_clip = saved_pending_clip;
        self.text_clip_path = saved_text_clip_path;
        self.regional_image_names = saved_image_names;
        self.regional_inline_image_count = saved_inline_count;
        self.regional_form_names = saved_form_names;
        self.regional_shading_names = saved_shading_names;
        self.pending_inline_params = saved_pending_inline;
    }

    fn apply_tiling_bbox_clip(&mut self, bbox: [f64; 4], tile_ctm: Transform2D) {
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
        self.sink.push_line("newpath");
        self.sink.append_path(&flat);
        self.sink.push_line("clip");
        self.sink.push_line("newpath");
    }

    fn emit_shading_pattern_clip(
        &mut self,
        pattern_name: Option<String>,
        flat: &FlatPath,
        rule: FillRule,
        stage: &str,
    ) {
        if flat.subpaths.iter().all(|s| s.is_empty()) {
            return;
        }
        let Some(pattern_name) = pattern_name else {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                "regional PS shading pattern {stage} requires an active pattern name"
            )));
            return;
        };
        let reader = self.engine.document().reader();
        let Some(pattern) = load_vector_shading_pattern_for_postscript_output(
            &self.resources,
            Some(reader),
            &pattern_name,
        ) else {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                "regional PS shading pattern /{pattern_name} is not vector-safe"
            )));
            return;
        };

        self.sink.push_line("gsave");
        self.sink.push_line("newpath");
        self.sink.append_path(flat);
        match rule {
            FillRule::EvenOdd => self.sink.push_line("eoclip"),
            FillRule::NonZero => self.sink.push_line("clip"),
        }
        self.sink.push_line("newpath");
        let pattern_ctm = Transform2D::from(pattern.matrix).concat(&self.ctm());
        self.emit_vector_shading(pattern.shading, pattern_ctm);
        self.sink.push_line("grestore");
    }

    /// Emit `setlinecap`/`setlinejoin`/`setdash` from the current graphics
    /// state, in device-pixel units (matching the flattened geometry).
    fn emit_line_style(&mut self) {
        use crate::content::state::{LineCap, LineJoin};
        let cap = match self.gs.line_cap {
            LineCap::Butt => 0,
            LineCap::Round => 1,
            LineCap::ProjectingSquare => 2,
        };
        let join = match self.gs.line_join {
            LineJoin::Miter => 0,
            LineJoin::Round => 1,
            LineJoin::Bevel => 2,
        };
        self.sink.push_line(&format!("{cap} setlinecap"));
        self.sink.push_line(&format!("{join} setlinejoin"));
        self.sink.push_line(&format!(
            "{:.3} setmiterlimit",
            self.gs.miter_limit.max(1.0)
        ));
        if self.gs.dash.pattern.is_empty() {
            self.sink.push_line("[] 0 setdash");
        } else {
            let scale = self.device_scale();
            let dashes: Vec<String> = self
                .gs
                .dash
                .pattern
                .iter()
                .map(|d| format!("{:.3}", d * scale))
                .collect();
            let phase = self.gs.dash.phase * scale;
            self.sink
                .push_line(&format!("[{}] {:.3} setdash", dashes.join(" "), phase));
        }
    }

    /// Average device scale (CTM scale * viewport scale) used for line widths
    /// and dash lengths.
    fn device_scale(&self) -> f64 {
        let ctm = self.ctm();
        let sx = (ctm.a * ctm.a + ctm.b * ctm.b).sqrt();
        let sy = (ctm.c * ctm.c + ctm.d * ctm.d).sqrt();
        let ctm_scale = ((sx * sy).abs()).sqrt().max(1e-6);
        ctm_scale * self.viewport.scale
    }

    fn device_dash_state(&self) -> DashState {
        let scale = self.device_scale();
        DashState::new(
            self.gs.dash.pattern.iter().map(|d| d * scale).collect(),
            self.gs.dash.phase * scale,
        )
    }

    fn device_line_width(&self) -> f64 {
        let w = self.gs.line_width * self.device_scale();
        if w <= 0.0 {
            1.0
        } else {
            w
        }
    }

    /// Emit `setrgbcolor`. PostScript vector output only reaches this path for
    /// opaque paint; fully transparent paint is skipped by callers, and
    /// fractional alpha remains a raster-fallback/refusal boundary.
    /// Carry a non-default PDF flatness tolerance (`i` / ExtGState `/FL`) to
    /// the PostScript `setflat` operator: curves are emitted as `curveto`, so
    /// the interpreter flattens them. Coarse requests are clamped to one
    /// device pixel as the raster renderer does.
    fn emit_flatness(&mut self) {
        // 0.5 is `GraphicsState::default().flatness`, the interpreter's own
        // tolerance when the content never sets one.
        if self.gs.flatness == 0.5 {
            return;
        }
        let flatness = effective_device_flatness(self.gs.path_flatness_tolerance());
        self.sink
            .push_line(&format!("{} setflat", ps_number(flatness)));
    }

    fn emit_setcolor(&mut self, color: RenderColor, alpha: f32) {
        let a = alpha.clamp(0.0, 1.0);
        if !ps_alpha_is_opaque(a) {
            self.record_fatal_error(WellfriendError::UnsupportedFeature(format!(
                "PostScript vector output cannot represent fractional alpha {a:.6}"
            )));
            return;
        }
        let (r, g, b) = (color.r, color.g, color.b);
        self.sink
            .push_line(&format!("{r:.4} {g:.4} {b:.4} setrgbcolor"));
    }

    /// Resolve a graphics-state colour to (`RenderColor`, alpha), mirroring the
    /// SVG sink (named colour spaces resolved through the page resources).
    fn resolve_color(&self, color: &Color, alpha: f32, role: &str) -> Result<(RenderColor, f32)> {
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
                    crate::render::colorspace::NamedColor::Color(rc) => return Ok((rc, rc.a)),
                    crate::render::colorspace::NamedColor::NoPaint => {
                        let transparent = RenderColor::transparent();
                        return Ok((transparent, transparent.a));
                    }
                    crate::render::colorspace::NamedColor::Invalid(reason) => {
                        return Err(WellfriendError::UnsupportedFeature(format!(
                            "{role} color space /{name} rejected: {reason}"
                        )));
                    }
                    crate::render::colorspace::NamedColor::Unhandled => {
                        return Err(WellfriendError::UnsupportedFeature(format!(
                            "{role} color space /{name} is unsupported for PostScript vector output"
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
        Ok((rc, rc.a))
    }

    fn resolve_color_or_fatal(
        &mut self,
        color: &Color,
        alpha: f32,
        role: &str,
    ) -> Option<(RenderColor, f32)> {
        match self.resolve_color(color, alpha, role) {
            Ok(color) => Some(color),
            Err(err) => {
                self.record_fatal_error(err);
                None
            }
        }
    }

    fn current_fill_color_or_fatal(&mut self, role: &str) -> Option<(RenderColor, f32)> {
        let color = self.gs.fill_color.clone();
        self.resolve_color_or_fatal(&color, self.gs.fill_alpha as f32, role)
    }

    fn current_stroke_color_or_fatal(&mut self, role: &str) -> Option<(RenderColor, f32)> {
        let color = self.gs.stroke_color.clone();
        self.resolve_color_or_fatal(&color, self.gs.stroke_alpha as f32, role)
    }

    // ---- text ----

    fn show_text_array(&mut self, op: &ContentOperation) {
        let Some(items) = op.operand(0).and_then(Operand::as_array) else {
            return;
        };
        for item in items {
            match item {
                Operand::String(bytes) => self.show_text_glyphs(bytes),
                Operand::Integer(v) => self.adjust_text_position(-(*v as f64)),
                Operand::Real(v) => self.adjust_text_position(-*v),
                _ => {}
            }
        }
        self.flush_text_run();
    }

    fn show_text(&mut self, bytes: &[u8]) {
        self.show_text_glyphs(bytes);
        self.flush_text_run();
    }

    fn show_text_glyphs(&mut self, bytes: &[u8]) {
        let font_name = self.gs.text.font_name.clone();
        let font_size = self.gs.text.font_size;
        if font_size <= 0.0 {
            return;
        }
        let reader = self.engine.document().reader();
        let decoded = match self.resources.fonts.get(&font_name) {
            Some(font_dict) => {
                let resolver = self
                    .font_resolver_cache
                    .entry(font_name.clone())
                    .or_insert_with(|| {
                        std::sync::Arc::new(crate::fonts::FontResolver::new(font_dict, reader))
                    })
                    .clone();
                crate::render::text_decode::try_decode_text_bytes_with_resolver(
                    bytes, font_dict, &resolver, reader,
                )
            }
            None => try_decode_text_bytes(bytes, &font_name, &self.resources, reader),
        };
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(reason) => {
                self.record_fatal_error(WellfriendError::UnsupportedFeature(reason));
                return;
            }
        };
        let font_bytes = match self.font_bytes_cache.get(&font_name) {
            Some(cached) => cached.clone(),
            None => {
                let resolved = get_font_bytes(&font_name, &self.resources, reader)
                    .map(std::sync::Arc::<[u8]>::from);
                self.font_bytes_cache
                    .insert(font_name.clone(), resolved.clone());
                resolved
            }
        };
        let font_program = font_bytes.as_deref().filter(|bytes| !bytes.is_empty());
        let upem = font_program
            .and_then(get_upem)
            .map(f64::from)
            .filter(|v| *v > 0.0)
            .unwrap_or(1000.0);

        for glyph in decoded {
            let font_advance =
                font_program.and_then(|fb| decoded_glyph_strict_horizontal_advance(&glyph, fb));
            if !matches!(self.gs.text.rendering_mode, 3) {
                let Some(fb) = font_program else {
                    self.record_fatal_error(WellfriendError::UnsupportedFeature(
                        "PostScript text font program unavailable for vector output".to_string(),
                    ));
                    return;
                };
                if self.queue_font_glyph(&font_name, fb, &glyph, upem, font_advance) {
                    if self.fatal_error.is_some() {
                        return;
                    }
                } else {
                    // Outline fallback: keep painting order with queued text.
                    self.flush_text_run();
                    let Some(outline) = decoded_glyph_strict_outline(&glyph, fb) else {
                        self.record_fatal_error(WellfriendError::UnsupportedFeature(
                            "PostScript text glyph outline unavailable for vector output"
                                .to_string(),
                        ));
                        return;
                    };
                    self.emit_glyph_outline(outline, upem);
                }
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
                            "PostScript text glyph advance unavailable for vector output"
                                .to_string(),
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

    /// Embedding decision for the font resource `font_name` (whose program is
    /// `program`), cached for the current resource scope.
    fn text_font(&mut self, font_name: &str, program: &[u8]) -> Option<std::rc::Rc<PsTextFont>> {
        if let Some(cached) = self.text_font_cache.get(font_name) {
            return cached.clone();
        }
        let font = fonts::classify_font_program(program).map(|kind| {
            let num_glyphs = match &kind {
                PsFontKind::TrueType(bytes) => ttf_parser::Face::parse(bytes, 0)
                    .map(|face| face.number_of_glyphs())
                    .unwrap_or(0),
                PsFontKind::Outline { .. } => 0,
            };
            std::rc::Rc::new(PsTextFont {
                base_name: fonts::ps_font_base_name(program),
                kind,
                num_glyphs,
            })
        });
        self.text_font_cache
            .insert(font_name.to_string(), font.clone());
        font
    }

    /// Queue `glyph` as embedded-font text (filled text in an embeddable
    /// font). Returns `false` when the glyph must be drawn as an outline
    /// instead; `true` when it was queued or legitimately paints nothing.
    fn queue_font_glyph(
        &mut self,
        font_name: &str,
        program: &[u8],
        glyph: &DecodedGlyph,
        upem: f64,
        font_advance: Option<f64>,
    ) -> bool {
        if !self.embed_fonts
            || self.gs.text.rendering_mode != 0
            || glyph.is_vertical
            || self.gs.fill_color_space.is_pattern()
        {
            return false;
        }
        let Some(font) = self.text_font(font_name, program) else {
            return false;
        };
        let (glyph_name, source) = match &font.kind {
            PsFontKind::TrueType(_) => {
                let gid = if glyph.is_gid {
                    Some(glyph.code)
                } else {
                    crate::render::glyph_outline::simple_glyph_sfnt_gid(
                        program,
                        glyph.code,
                        glyph.unicode,
                        glyph.glyph_name.as_deref(),
                    )
                };
                let Some(gid) = gid.filter(|&gid| gid < font.num_glyphs) else {
                    return false;
                };
                (
                    fonts::ps_glyph_name(glyph.unicode, gid),
                    PsGlyphSource::Gid(gid),
                )
            }
            PsFontKind::Outline { .. } => {
                let Some(outline) = decoded_glyph_strict_outline(glyph, program) else {
                    return false;
                };
                let advance = glyph
                    .width
                    .filter(|width| width.is_finite())
                    .or(font_advance)
                    .unwrap_or(0.0)
                    * upem
                    / 1000.0;
                let ident = fonts::outline_glyph_ident(outline.as_ref(), advance);
                (
                    fonts::ps_glyph_name(glyph.unicode, format_args!("{ident:x}")),
                    PsGlyphSource::Outline {
                        path: outline,
                        advance,
                    },
                )
            }
        };

        let scale = font_size_scale(self.gs.text.font_size, upem);
        let th = self.gs.text.horizontal_scaling / 100.0;
        let scale_x = scale * th;
        if scale <= 0.0 || !scale_x.is_finite() {
            // The outline path draws nothing for a degenerate size either.
            return true;
        }
        let glyph_ctm = Transform2D::scale(scale_x, scale)
            .concat(&Transform2D::translation(0.0, self.gs.text.rise))
            .concat(&Transform2D::from(self.gs.text.tm))
            .concat(&self.ctm())
            .concat(&self.viewport.to_transform());
        let [a, b, c, d, e, f] = glyph_ctm.to_array();
        let matrix = [a * upem, b * upem, c * upem, d * upem];
        if !matrix.iter().chain([&e, &f]).all(|v| v.is_finite()) {
            return false;
        }
        if matrix[0] * matrix[3] - matrix[1] * matrix[2] == 0.0 {
            // A singular text matrix paints nothing.
            return true;
        }

        let Some((color, alpha)) = self.current_fill_color_or_fatal("PostScript text fill color")
        else {
            return true;
        };
        if ps_alpha_is_fully_transparent(alpha) {
            return true;
        }
        if !ps_alpha_is_opaque(alpha) {
            // Let the outline path report the unsupported alpha.
            return false;
        }

        let preferred = u8::try_from(u32::from(glyph.unicode))
            .ok()
            .filter(|code| (0x20..0x7F).contains(code) || *code >= 0xA1);
        let (instance, code) =
            self.sink
                .text
                .code_for(&font.base_name, &font.kind, &glyph_name, preferred, || {
                    source
                });
        let continues = self.text_run.as_ref().is_some_and(|run| {
            run.instance == instance && same_font_matrix(&run.matrix, &matrix) && run.color == color
        });
        if !continues {
            self.flush_text_run();
            self.text_run = Some(PsTextRun {
                instance,
                matrix,
                color,
                codes: Vec::new(),
                origins: Vec::new(),
            });
        }
        if let Some(run) = self.text_run.as_mut() {
            run.codes.push(code);
            run.origins.push((e, f));
        }
        true
    }

    /// Write the queued text run: select the font instance under the run's
    /// matrix (when it changed), then show the codes with every glyph placed
    /// at its exact origin (`xshow` / `yshow` / `xyshow`).
    fn flush_text_run(&mut self) {
        use std::fmt::Write as _;
        let Some(run) = self.text_run.take() else {
            return;
        };
        if run.codes.is_empty() {
            return;
        }
        self.emit_setcolor(run.color, 1.0);
        if self.sink.current_font != Some((run.instance, run.matrix)) {
            let [a, b, c, d] = run.matrix;
            self.sink.push_line(&format!(
                "/{} [{} {} {} {} 0 0] wfsf",
                PsPageText::instance_name(run.instance),
                ps_number(a),
                ps_number(b),
                ps_number(c),
                ps_number(d)
            ));
            self.sink.current_font = Some((run.instance, run.matrix));
        }
        // Positions on a 1/100 device-pixel grid; displacements between the
        // rounded origins never accumulate rounding drift.
        let grid: Vec<(i64, i64)> = run
            .origins
            .iter()
            .map(|&(x, y)| ((x * 100.0).round() as i64, (y * 100.0).round() as i64))
            .collect();
        let hundredths = |v: i64| PsNum(v as f64 / 100.0);
        let mut line = format!(
            "{} {} moveto {}",
            hundredths(grid[0].0),
            hundredths(grid[0].1),
            fonts::ps_code_string(&run.codes)
        );
        if grid.len() == 1 {
            line.push_str(" show");
            self.sink.push_line(&line);
            return;
        }
        let deltas: Vec<(i64, i64)> = grid
            .windows(2)
            .map(|pair| (pair[1].0 - pair[0].0, pair[1].1 - pair[0].1))
            .chain(std::iter::once((0, 0)))
            .collect();
        let (operator, per_line) = if deltas.iter().all(|d| d.1 == 0) {
            ("xshow", 24)
        } else if deltas.iter().all(|d| d.0 == 0) {
            ("yshow", 24)
        } else {
            ("xyshow", 12)
        };
        line.push_str(" [");
        for (index, &(dx, dy)) in deltas.iter().enumerate() {
            if index > 0 && index % per_line == 0 {
                line.push('\n');
            } else if index > 0 {
                line.push(' ');
            }
            let _ = match operator {
                "xshow" => write!(line, "{}", hundredths(dx)),
                "yshow" => write!(line, "{}", hundredths(dy)),
                _ => write!(line, "{} {}", hundredths(dx), hundredths(dy)),
            };
        }
        let _ = write!(line, "] {operator}");
        self.sink.push_line(&line);
    }

    /// Emit one glyph as a filled (or stroked) PostScript path outline, in
    /// device space.
    fn emit_glyph_outline(&mut self, outline: Option<Path>, upem: f64) {
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
        let device = DevicePath::new(&glyph_path, &glyph_ctm, &self.viewport, 0.3);
        if device.is_empty() {
            return;
        }
        // Flattened outlines are only needed for text clipping and for the
        // pattern-paint clips; plain paints keep the glyph's curves.
        let needs_flat = matches!(self.gs.text.rendering_mode, 4..=7)
            || self.gs.fill_color_space.is_pattern()
            || self.gs.stroke_color_space.is_pattern();
        let flat = if needs_flat {
            flatten_path(&glyph_path, &glyph_ctm, &self.viewport, 0.3)
        } else {
            FlatPath::default()
        };
        self.append_text_clip_path(&flat);

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
                    let Some((color, a)) =
                        self.current_stroke_color_or_fatal("PostScript text stroke color")
                    else {
                        return;
                    };
                    if !ps_alpha_is_fully_transparent(a) {
                        self.emit_setcolor(color, a);
                        let w = self.device_line_width();
                        self.sink.push_line(&format!("{w:.3} setlinewidth"));
                        self.emit_line_style();
                        self.sink.push_line("newpath");
                        self.sink.append_device_path(&device);
                        self.sink.push_line("stroke");
                    }
                }
            }
            2 | 6 => {
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
                    let Some((fill_color, fill_alpha)) =
                        self.current_fill_color_or_fatal("PostScript text fill color")
                    else {
                        return;
                    };
                    if !ps_alpha_is_fully_transparent(fill_alpha) {
                        self.emit_setcolor(fill_color, fill_alpha);
                        self.sink.push_line("newpath");
                        self.sink.append_device_path(&device);
                        self.sink.push_line("fill");
                    }
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
                    self.emit_pattern_clip(
                        self.gs.stroke_pattern_name.clone(),
                        &outline,
                        FillRule::NonZero,
                        self.gs.stroke_alpha as f32,
                        true,
                        "text stroke",
                    );
                } else {
                    let Some((stroke_color, stroke_alpha)) =
                        self.current_stroke_color_or_fatal("PostScript text stroke color")
                    else {
                        return;
                    };
                    if !ps_alpha_is_fully_transparent(stroke_alpha) {
                        self.emit_setcolor(stroke_color, stroke_alpha);
                        let w = self.device_line_width();
                        self.sink.push_line(&format!("{w:.3} setlinewidth"));
                        self.emit_line_style();
                        self.sink.push_line("newpath");
                        self.sink.append_device_path(&device);
                        self.sink.push_line("stroke");
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
                    let Some((color, a)) =
                        self.current_fill_color_or_fatal("PostScript text fill color")
                    else {
                        return;
                    };
                    if !ps_alpha_is_fully_transparent(a) {
                        self.emit_setcolor(color, a);
                        self.sink.push_line("newpath");
                        self.sink.append_device_path(&device);
                        // Glyph outlines use the nonzero winding rule.
                        self.sink.push_line("fill");
                    }
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
                "PostScript text vertical advance unavailable for vector output".to_string(),
            ));
            return false;
        };
        let advance_y = crate::fonts::resolver::vertical_text_advance(
            vertical_advance,
            self.gs.text.font_size,
            self.gs.text.char_spacing,
            self.gs.text.word_spacing,
            glyph.is_space,
        );
        self.translate_text_matrix(0.0, advance_y);
        true
    }

    fn adjust_text_position(&mut self, adjustment: f64) {
        let vertical = self
            .resources
            .fonts
            .get(&self.gs.text.font_name)
            .is_some_and(|font| {
                crate::fonts::resolver::uses_vertical_writing(font, self.engine.document().reader())
            });
        let [tx, ty] = crate::fonts::resolver::text_position_adjustment(
            adjustment,
            self.gs.text.font_size,
            self.gs.text.horizontal_scaling,
            vertical,
        );
        self.translate_text_matrix(tx, ty);
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

fn ps_alpha_is_fully_transparent(alpha: f32) -> bool {
    alpha <= f32::EPSILON
}

fn ps_alpha_is_opaque(alpha: f32) -> bool {
    alpha >= 0.999
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionalRgbaAlphaState {
    Opaque,
    FullyTransparent,
    Binary,
    Fractional,
}

fn regional_rgba_alpha_state(raw: &RawImage) -> Option<RegionalRgbaAlphaState> {
    if raw.channels != 4 {
        return None;
    }
    let mut any_opaque = false;
    let mut any_transparent = false;
    for pixel in raw.pixels.as_chunks::<4>().0.iter() {
        match pixel[3] {
            255 => any_opaque = true,
            0 => any_transparent = true,
            _ => return Some(RegionalRgbaAlphaState::Fractional),
        }
    }
    if any_opaque && any_transparent {
        Some(RegionalRgbaAlphaState::Binary)
    } else if any_opaque {
        Some(RegionalRgbaAlphaState::Opaque)
    } else {
        Some(RegionalRgbaAlphaState::FullyTransparent)
    }
}

fn regional_binary_alpha_clip_path(
    raw: &RawImage,
    transform: [f64; 6],
    flatness: f64,
    context: &str,
) -> Result<FlatPath> {
    ensure_regional_raw_image(raw, context)?;
    if regional_rgba_alpha_state(raw) != Some(RegionalRgbaAlphaState::Binary) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "{context}: regional PS binary alpha clip requires mixed 0/255 alpha"
        )));
    }
    let iw = raw.width as usize;
    let ih = raw.height as usize;
    let channels = raw.channels as usize;
    let mut painted = 0usize;
    let mut path = Path::new();
    for row in 0..ih {
        for col in 0..iw {
            let alpha = raw.pixels[(row * iw + col) * channels + 3];
            if alpha != 255 {
                continue;
            }
            painted = painted.saturating_add(1);
            if painted > MAX_PATTERN_STENCIL_CLIP_RECTS {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "{context}: regional PS binary alpha clip has {painted} opaque cells, cap is {MAX_PATTERN_STENCIL_CLIP_RECTS}"
                )));
            }
            let x = col as f64 / iw as f64;
            let y = row as f64 / ih as f64;
            path.rect(x, y, 1.0 / iw as f64, 1.0 / ih as f64);
        }
    }
    Ok(flatten_path_device_transform(
        &path,
        &Transform2D::from_array(transform),
        flatness,
    ))
}

fn ps_rgb_components(color: [f32; 3]) -> String {
    format!(
        "{} {} {}",
        ps_function_number(f64::from(color[0].clamp(0.0, 1.0)), 4),
        ps_function_number(f64::from(color[1].clamp(0.0, 1.0)), 4),
        ps_function_number(f64::from(color[2].clamp(0.0, 1.0)), 4)
    )
}

fn ps_shading_color_space_and_function(
    exact_color_space: Option<&VectorPostScriptShadingColorSpace>,
    exact: Option<&VectorPostScriptShadingFunction>,
    stops: &[VectorShadingStop],
) -> (String, String) {
    if let Some(color_space) = exact_color_space {
        if let Some(function) = exact {
            return (
                ps_named_shading_color_space(color_space),
                ps_exact_shading_function(function),
            );
        }
    }
    match exact {
        Some(VectorPostScriptShadingFunction::Type2Cmyk(function)) => (
            "/DeviceCMYK".to_string(),
            ps_cmyk_exact_type2_function(function),
        ),
        Some(VectorPostScriptShadingFunction::Type2CmykArray(function)) => (
            "/DeviceCMYK".to_string(),
            ps_cmyk_exact_type2_function_array(function),
        ),
        Some(VectorPostScriptShadingFunction::StitchingCmyk(function)) => (
            "/DeviceCMYK".to_string(),
            ps_cmyk_exact_stitching_function(function),
        ),
        _ => (
            "/DeviceRGB".to_string(),
            ps_rgb_shading_function(exact, stops),
        ),
    }
}

fn ps_shading_domain_clause(
    domain: [f64; 2],
    exact: Option<&VectorPostScriptShadingFunction>,
) -> String {
    if exact.is_some() && domain != [0.0, 1.0] {
        format!(" /Domain {}", ps_function_domain(domain))
    } else {
        String::new()
    }
}

fn ps_named_shading_color_space(color_space: &VectorPostScriptShadingColorSpace) -> String {
    let alternate = match color_space.alternate {
        VectorPostScriptAlternateColorSpace::DeviceRgb => "/DeviceRGB",
        VectorPostScriptAlternateColorSpace::DeviceCmyk => "/DeviceCMYK",
    };
    let tint_transform = ps_exact_shading_function(&color_space.tint_transform);
    match color_space.family {
        VectorPostScriptNamedColorFamily::Separation => {
            let colorant = color_space
                .colorants
                .first()
                .map(|name| ps_name_literal(name))
                .unwrap_or_else(|| "/None".to_string());
            format!("[/Separation {colorant} {alternate} {tint_transform}]")
        }
        VectorPostScriptNamedColorFamily::DeviceN => {
            let colorants = color_space
                .colorants
                .iter()
                .map(|name| ps_name_literal(name))
                .collect::<Vec<_>>()
                .join(" ");
            format!("[/DeviceN [{colorants}] {alternate} {tint_transform}]")
        }
    }
}

fn ps_name_literal(name: &str) -> String {
    let mut escaped = String::from("/");
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.') {
            escaped.push(byte as char);
        } else {
            escaped.push_str(&format!("#{byte:02X}"));
        }
    }
    escaped
}

fn ps_exact_shading_function(function: &VectorPostScriptShadingFunction) -> String {
    match function {
        VectorPostScriptShadingFunction::Type2(function) => ps_rgb_exact_type2_function(function),
        VectorPostScriptShadingFunction::Type2RgbArray(function) => {
            ps_rgb_exact_type2_function_array(function)
        }
        VectorPostScriptShadingFunction::Type2Cmyk(function) => {
            ps_cmyk_exact_type2_function(function)
        }
        VectorPostScriptShadingFunction::Type2CmykArray(function) => {
            ps_cmyk_exact_type2_function_array(function)
        }
        VectorPostScriptShadingFunction::Type2Tint(function) => {
            ps_rgb_exact_type2_component_function(function)
        }
        VectorPostScriptShadingFunction::Stitching(function) => {
            ps_rgb_exact_stitching_function(function)
        }
        VectorPostScriptShadingFunction::StitchingCmyk(function) => {
            ps_cmyk_exact_stitching_function(function)
        }
        VectorPostScriptShadingFunction::StitchingTint(function) => {
            ps_tint_exact_stitching_function(function)
        }
    }
}

fn ps_rgb_shading_function(
    exact: Option<&VectorPostScriptShadingFunction>,
    stops: &[VectorShadingStop],
) -> String {
    if let Some(function) = exact {
        return ps_exact_shading_function(function);
    }
    if stops.len() <= 2 {
        let c0 = ps_rgb_components(stops.first().map(|stop| stop.rgb).unwrap_or([0.0; 3]));
        let c1 = ps_rgb_components(stops.last().map(|stop| stop.rgb).unwrap_or([0.0; 3]));
        return format!("<< /FunctionType 2 /Domain [0 1] /C0 [{c0}] /C1 [{c1}] /N 1 >>");
    }

    let mut functions = String::new();
    let mut bounds = String::new();
    let mut encode = String::new();
    for pair in stops.windows(2) {
        let c0 = ps_rgb_components(pair[0].rgb);
        let c1 = ps_rgb_components(pair[1].rgb);
        functions.push_str(&format!(
            "<< /FunctionType 2 /Domain [0 1] /C0 [{c0}] /C1 [{c1}] /N 1 >> "
        ));
        encode.push_str("0 1 ");
    }
    for stop in &stops[1..stops.len() - 1] {
        bounds.push_str(&format!(
            "{} ",
            ps_function_number(stop.offset.clamp(0.0, 1.0), 6)
        ));
    }
    format!(
        "<< /FunctionType 3 /Domain [0 1] /Functions [ {functions}] /Bounds [{bounds}] /Encode [{encode}] >>"
    )
}

fn ps_rgb_exact_type2_function(function: &VectorPostScriptType2Function) -> String {
    let c0 = ps_rgb_components_f64(function.c0);
    let c1 = ps_rgb_components_f64(function.c1);
    let range = ps_function_range(function.range.as_ref());
    format!(
        "<< /FunctionType 2 /Domain {} /C0 [{c0}] /C1 [{c1}] /N {}{range} >>",
        ps_function_domain(function.domain),
        ps_function_number(function.n, 6)
    )
}

fn ps_rgb_exact_type2_function_array(function: &VectorPostScriptType2RgbArrayFunction) -> String {
    let mut functions = String::new();
    for channel in &function.channels {
        functions.push_str(&ps_rgb_exact_type2_component_function(channel));
        functions.push(' ');
    }
    format!("[ {functions}]")
}

fn ps_rgb_exact_type2_component_function(
    function: &VectorPostScriptType2ComponentFunction,
) -> String {
    let range = ps_component_function_range(function.range);
    format!(
        "<< /FunctionType 2 /Domain {} /C0 [{}] /C1 [{}] /N {}{range} >>",
        ps_function_domain(function.domain),
        ps_function_number(function.c0, 4),
        ps_function_number(function.c1, 4),
        ps_function_number(function.n, 6)
    )
}

fn ps_cmyk_exact_type2_function(function: &VectorPostScriptType2CmykFunction) -> String {
    let c0 = ps_cmyk_components_f64(function.c0);
    let c1 = ps_cmyk_components_f64(function.c1);
    let range = ps_function_range(function.range.as_ref());
    format!(
        "<< /FunctionType 2 /Domain {} /C0 [{c0}] /C1 [{c1}] /N {}{range} >>",
        ps_function_domain(function.domain),
        ps_function_number(function.n, 6)
    )
}

fn ps_cmyk_exact_type2_function_array(function: &VectorPostScriptType2CmykArrayFunction) -> String {
    let mut functions = String::new();
    for channel in &function.channels {
        functions.push_str(&ps_rgb_exact_type2_component_function(channel));
        functions.push(' ');
    }
    format!("[ {functions}]")
}

fn ps_rgb_exact_stitching_function(function: &VectorPostScriptStitchingFunction) -> String {
    let range = ps_function_range(function.range.as_ref());
    let mut functions = String::new();
    let mut bounds = String::new();
    let mut encode = String::new();
    for segment in &function.segments {
        functions.push_str(&ps_rgb_exact_type2_function(&segment.function));
        functions.push(' ');
        encode.push_str(&format!(
            "{} {} ",
            ps_function_number(segment.encode[0], 6),
            ps_function_number(segment.encode[1], 6)
        ));
    }
    for segment in function
        .segments
        .iter()
        .take(function.segments.len().saturating_sub(1))
    {
        bounds.push_str(&format!("{} ", ps_function_number(segment.bound_end, 6)));
    }
    format!(
        "<< /FunctionType 3 /Domain {} /Functions [ {functions}] /Bounds [{bounds}] /Encode [{encode}]{range} >>",
        ps_function_domain(function.domain)
    )
}

fn ps_cmyk_exact_stitching_function(function: &VectorPostScriptCmykStitchingFunction) -> String {
    let range = ps_function_range(function.range.as_ref());
    let mut functions = String::new();
    let mut bounds = String::new();
    let mut encode = String::new();
    for segment in &function.segments {
        functions.push_str(&ps_cmyk_exact_type2_function(&segment.function));
        functions.push(' ');
        encode.push_str(&format!(
            "{} {} ",
            ps_function_number(segment.encode[0], 6),
            ps_function_number(segment.encode[1], 6)
        ));
    }
    for segment in function
        .segments
        .iter()
        .take(function.segments.len().saturating_sub(1))
    {
        bounds.push_str(&format!("{} ", ps_function_number(segment.bound_end, 6)));
    }
    format!(
        "<< /FunctionType 3 /Domain {} /Functions [ {functions}] /Bounds [{bounds}] /Encode [{encode}]{range} >>",
        ps_function_domain(function.domain)
    )
}

fn ps_tint_exact_stitching_function(function: &VectorPostScriptTintStitchingFunction) -> String {
    let range = ps_component_function_range(function.range);
    let mut functions = String::new();
    let mut bounds = String::new();
    let mut encode = String::new();
    for segment in &function.segments {
        functions.push_str(&ps_rgb_exact_type2_component_function(&segment.function));
        functions.push(' ');
        encode.push_str(&format!(
            "{} {} ",
            ps_function_number(segment.encode[0], 6),
            ps_function_number(segment.encode[1], 6)
        ));
    }
    for segment in function
        .segments
        .iter()
        .take(function.segments.len().saturating_sub(1))
    {
        bounds.push_str(&format!("{} ", ps_function_number(segment.bound_end, 6)));
    }
    format!(
        "<< /FunctionType 3 /Domain {} /Functions [ {functions}] /Bounds [{bounds}] /Encode [{encode}]{range} >>",
        ps_function_domain(function.domain)
    )
}

fn ps_function_domain(domain: [f64; 2]) -> String {
    format!(
        "[{} {}]",
        ps_function_number(domain[0], 6),
        ps_function_number(domain[1], 6)
    )
}

// Keep existing compact spellings when exact, but never round distinct function
// bounds, exponents or coefficients together. PostScript accepts real exponents.
fn ps_function_number(value: f64, precision: usize) -> String {
    let compact = format!("{value:.precision$}");
    if compact.len() <= 32 && compact.parse::<f64>().ok() == Some(value) {
        compact
    } else {
        format!("{value:e}")
    }
}

fn ps_function_range<const N: usize>(range: Option<&[[f64; 2]; N]>) -> String {
    let Some(range) = range else {
        return String::new();
    };
    let mut values = String::new();
    for pair in range {
        values.push_str(&format!(
            "{} {} ",
            ps_function_number(pair[0], 6),
            ps_function_number(pair[1], 6)
        ));
    }
    format!(" /Range [{}]", values.trim_end())
}

fn ps_component_function_range(range: Option<[f64; 2]>) -> String {
    let Some(range) = range else {
        return String::new();
    };
    ps_function_range(Some(&[range]))
}

fn ps_bool_pair(values: [bool; 2]) -> &'static str {
    match values {
        [true, true] => "[true true]",
        [true, false] => "[true false]",
        [false, true] => "[false true]",
        [false, false] => "[false false]",
    }
}

fn ps_rgb_components_f64(color: [f64; 3]) -> String {
    format!(
        "{} {} {}",
        ps_function_number(color[0], 4),
        ps_function_number(color[1], 4),
        ps_function_number(color[2], 4)
    )
}

fn ps_cmyk_components_f64(color: [f64; 4]) -> String {
    format!(
        "{} {} {} {}",
        ps_function_number(color[0], 4),
        ps_function_number(color[1], 4),
        ps_function_number(color[2], 4),
        ps_function_number(color[3], 4)
    )
}

fn ps_region_comment(kind: &str, bounds: [f64; 4]) -> String {
    format!(
        "% WellfriendRegion kind={kind} bounds={:.3} {:.3} {:.3} {:.3}",
        bounds[0], bounds[1], bounds[2], bounds[3]
    )
}

#[cfg(test)]
fn regional_colorimage_hex(raw: &RawImage, context: &str) -> Result<String> {
    regional_colorimage_hex_with_alpha_policy(raw, context, false)
}

#[cfg(test)]
fn regional_colorimage_hex_with_alpha_policy(
    raw: &RawImage,
    context: &str,
    allow_binary_alpha: bool,
) -> Result<String> {
    const HEXCHARS: &[u8; 16] = b"0123456789ABCDEF";
    let rgb = regional_colorimage_rgb_with_alpha_policy(raw, context, allow_binary_alpha)?;
    let mut hex = String::with_capacity(rgb.len() * 2 + rgb.len() / 39);
    let mut col = 0usize;
    for byte in rgb {
        hex.push(HEXCHARS[(byte >> 4) as usize] as char);
        hex.push(HEXCHARS[(byte & 0xf) as usize] as char);
        col += 2;
        if col >= 78 {
            hex.push('\n');
            col = 0;
        }
    }
    Ok(hex)
}

/// Validate a regional image for `colorimage` and return its samples as
/// 8-bit RGB, row-major top-to-bottom.
fn regional_colorimage_rgb_with_alpha_policy(
    raw: &RawImage,
    context: &str,
    allow_binary_alpha: bool,
) -> Result<Vec<u8>> {
    ensure_regional_raw_image(raw, context)?;
    let iw = raw.width as usize;
    let ih = raw.height as usize;
    let channels = raw.channels as usize;
    match regional_rgba_alpha_state(raw) {
        Some(RegionalRgbaAlphaState::Opaque) | None => {}
        Some(RegionalRgbaAlphaState::Binary) if allow_binary_alpha => {}
        Some(RegionalRgbaAlphaState::Binary) => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "{context}: regional PS colorimage requires an alpha clip for binary alpha"
            )))
        }
        Some(RegionalRgbaAlphaState::FullyTransparent) => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "{context}: regional PS colorimage has no opaque pixels"
            )))
        }
        Some(RegionalRgbaAlphaState::Fractional) => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "{context}: regional PS colorimage cannot represent fractional alpha"
            )))
        }
    }

    let mut rgb = Vec::with_capacity(iw * ih * 3);
    for row in 0..ih {
        for px in 0..iw {
            let offset = (row * iw + px) * channels;
            if channels >= 3 {
                rgb.extend_from_slice(&raw.pixels[offset..offset + 3]);
            } else {
                let v = raw.pixels[offset];
                rgb.extend_from_slice(&[v, v, v]);
            }
        }
    }
    Ok(rgb)
}

/// ASCII85-encode `data` (PostScript Level 2 `/ASCII85Decode` input),
/// wrapped to 75-column lines and terminated by the `~>` end-of-data marker.
/// Four zero bytes encode as `z`; a final partial group writes `n + 1`
/// characters.
fn ascii85_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() / 4 * 5 + data.len() / 60 + 8);
    let mut column = 0usize;
    let mut push = |out: &mut String, text: &[u8]| {
        for &byte in text {
            out.push(byte as char);
            column += 1;
            if column >= 75 {
                out.push('\n');
                column = 0;
            }
        }
    };
    let chunks = data.chunks_exact(4);
    let remainder = chunks.remainder();
    for chunk in chunks {
        let value = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if value == 0 {
            push(&mut out, b"z");
            continue;
        }
        push(&mut out, &ascii85_group(value));
    }
    if !remainder.is_empty() {
        let mut padded = [0u8; 4];
        padded[..remainder.len()].copy_from_slice(remainder);
        let group = ascii85_group(u32::from_be_bytes(padded));
        push(&mut out, &group[..remainder.len() + 1]);
    }
    out.push_str("~>");
    out
}

/// zlib (RFC 1950) stream, the format PostScript's `/FlateDecode` reads.
fn zlib_compress(data: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut encoder = flate2::write::ZlibEncoder::new(
        Vec::with_capacity(data.len() / 2 + 64),
        flate2::Compression::default(),
    );
    // Writing into a Vec cannot fail.
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

/// PostScript `/RunLengthDecode` encoding (PackBits with a 128 EOD byte).
fn run_length_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 128 + 2);
    let mut i = 0usize;
    while i < data.len() {
        // A run of at least two equal bytes (up to 128).
        let mut run = 1usize;
        while i + run < data.len() && run < 128 && data[i + run] == data[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((257 - run) as u8);
            out.push(data[i]);
            i += run;
            continue;
        }
        // Literal bytes until the next run of three or the 128-byte cap.
        let start = i;
        while i < data.len() && i - start < 128 {
            if i + 2 < data.len() && data[i] == data[i + 1] && data[i] == data[i + 2] {
                break;
            }
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend_from_slice(&data[start..i]);
    }
    out.push(128);
    out
}

fn ascii85_group(mut value: u32) -> [u8; 5] {
    let mut group = [0u8; 5];
    for slot in group.iter_mut().rev() {
        *slot = b'!' + (value % 85) as u8;
        value /= 85;
    }
    group
}

/// Number of colour components declared by a baseline/progressive JPEG
/// frame header (SOFn), or `None` when the stream has no readable frame.
pub(crate) fn jpeg_frame_components(data: &[u8]) -> Option<u8> {
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2usize;
    while pos + 4 <= data.len() {
        if data[pos] != 0xFF {
            return None;
        }
        let marker = data[pos + 1];
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([data[pos + 2], data[pos + 3]]));
        if length < 2 {
            return None;
        }
        let is_frame = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_frame {
            // FF Cn Lh Ll P Yh Yl Xh Xl Nf
            return data.get(pos + 9).copied();
        }
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        pos = pos.checked_add(2 + length)?;
    }
    None
}

fn ps_concat_matrix(transform: [f64; 6]) -> String {
    let [a, b, c, d, e, f] = transform;
    format!("[{a:.6} {b:.6} {c:.6} {d:.6} {e:.6} {f:.6}] concat")
}

#[cfg(test)]
#[path = "postscript_function_tests.rs"]
mod function_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_rect(x0: f64, y0: f64, x1: f64, y1: f64) -> FlatPath {
        FlatPath {
            subpaths: vec![vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]],
            closed: vec![true],
        }
    }

    #[test]
    fn postscript_named_color_resolution_rejects_invalid_space_locally() {
        let engine = ContentEngine::open_bytes(crate::render::shading::tests_minimal_pdf())
            .expect("minimal PDF");
        let mut resources = PageResources::default();
        resources.color_spaces.insert(
            "Bad".to_string(),
            PdfObject::Array(vec![
                PdfObject::Name("CalRGB".to_string()),
                PdfObject::Dictionary(PdfDictionary::empty()),
            ]),
        );
        let mut sink = PsSink::new(10, 10);
        let mut state = PsRenderState {
            engine: &engine,
            page_resources: resources.clone(),
            font_bytes_cache: std::collections::HashMap::new(),
            font_resolver_cache: std::collections::HashMap::new(),
            resources,
            viewport: Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            gs: GraphicsState::default(),
            path: Path::new(),
            pending_clip: None,
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
            embed_fonts: true,
            text_font_cache: std::collections::HashMap::new(),
            text_run: None,
        };
        state.gs.fill_color_space = ColorSpace::Named("Bad".to_string());
        state.gs.fill_color = Color {
            space: ColorSpace::Named("Bad".to_string()),
            components: vec![0.2, 0.4, 0.6],
        };

        assert!(state
            .current_fill_color_or_fatal("PostScript test fill color")
            .is_none());
        let err = state
            .fatal_error
            .take()
            .expect("invalid named color space should be fatal");
        assert!(
            err.to_string()
                .contains("PostScript test fill color color space /Bad rejected"),
            "{err}"
        );
    }

    #[test]
    fn postscript_device_color_resolution_rejects_malformed_state_locally() {
        let engine = ContentEngine::open_bytes(crate::render::shading::tests_minimal_pdf())
            .expect("minimal PDF");
        let mut sink = PsSink::new(10, 10);
        let mut state = PsRenderState {
            engine: &engine,
            resources: PageResources::default(),
            page_resources: PageResources::default(),
            font_bytes_cache: std::collections::HashMap::new(),
            font_resolver_cache: std::collections::HashMap::new(),
            viewport: Viewport::new([0.0, 0.0, 10.0, 10.0], 72),
            gs: GraphicsState::default(),
            path: Path::new(),
            pending_clip: None,
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
            embed_fonts: true,
            text_font_cache: std::collections::HashMap::new(),
            text_run: None,
        };
        state.gs.fill_color = Color {
            space: ColorSpace::DeviceRGB,
            components: vec![1.0, 0.0],
        };

        assert!(state
            .current_fill_color_or_fatal("PostScript test fill color")
            .is_none());
        let err = state
            .fatal_error
            .take()
            .expect("malformed device color should be fatal");
        assert!(
            err.to_string()
                .contains("PostScript test fill color color rejected: malformed DeviceRGB color"),
            "{err}"
        );
    }

    #[test]
    fn sink_emits_gsave_flip_and_grestore() {
        let mut sink = PsSink::new(100, 200);
        sink.push_line("0 0 1 setrgbcolor");
        let out = sink.finish();
        assert!(out.starts_with("gsave\n"));
        assert!(out.contains("0 200 translate\n"));
        assert!(out.contains("1 -1 scale\n"));
        assert!(out.trim_end().ends_with("grestore"));
    }

    #[test]
    fn append_path_emits_moveto_lineto_closepath() {
        let mut sink = PsSink::new(50, 50);
        sink.append_path(&flat_rect(0.0, 0.0, 10.0, 10.0));
        assert!(sink.body.contains("0.00 0.00 moveto"));
        assert!(sink.body.contains("10.00 0.00 lineto"));
        assert!(sink.body.contains("closepath"));
    }

    #[test]
    fn raster_fallback_triggers_on_unsupported_constructs() {
        use crate::render::vector_fallback::{
            classify_page_for_vector_output, VectorFallbackDecision,
        };
        let r = PageResources::default();

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

        // Form XObject → whole-page.
        let mut r_form = PageResources::default();
        r_form
            .xobject_subtypes
            .insert("Im0".to_string(), "Form".to_string());
        r_form.xobjects.insert("Im0".to_string(), (1, 0));
        let do_op = ContentOperation::new("Do", vec![Operand::Name("Im0".into())]);
        assert!(matches!(
            classify_page_for_vector_output(&[do_op], &r_form, 1.0),
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
        let do_img = ContentOperation::new("Do", vec![Operand::Name("Im0".into())]);
        assert!(matches!(
            classify_page_for_vector_output(&[cm, do_img], &r_img, 1.0),
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }

    #[test]
    fn raster_image_declares_picstr_and_colorimage() {
        let mut sink = PsSink::new(2, 1);
        // 2x1 RGB image: red, green.
        let rgb = [255, 0, 0, 0, 255, 0];
        sink.emit_raster_image(&rgb, 2, 1);
        let out = sink.finish();
        assert!(
            out.contains("/picstra85 currentfile /ASCII85Decode filter def"),
            "{out}"
        );
        assert!(
            out.contains("/picstr picstra85 /FlateDecode filter def"),
            "{out}"
        );
        assert!(
            out.contains("picstr false 3 colorimage picstr flushfile picstra85 flushfile } exec")
        );
        assert!(!out.contains("readhexstring"));
        let encoded = ascii85_encode(&zlib_compress(&rgb));
        assert!(out.contains(&encoded), "{out}");
        let mut inflated = Vec::new();
        std::io::Read::read_to_end(
            &mut flate2::read::ZlibDecoder::new(ascii85_decode_for_test(&encoded).as_slice()),
            &mut inflated,
        )
        .unwrap();
        assert_eq!(inflated, rgb);
    }

    #[test]
    fn level2_raster_image_uses_run_length_samples() {
        let mut sink = PsSink::new(2, 1).with_image_filter(PsImageFilter::RunLength);
        sink.emit_raster_image(&[7, 7, 7, 7, 7, 7], 2, 1);
        let out = sink.finish();
        assert!(
            out.contains("/picstr picstra85 /RunLengthDecode filter def"),
            "{out}"
        );
        assert!(!out.contains("FlateDecode"));
        assert_eq!(body_language_level(&out), 2);
    }

    #[test]
    fn run_length_encoding_round_trips() {
        fn decode(data: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            let mut i = 0;
            loop {
                let n = data[i];
                i += 1;
                match n {
                    128 => return out,
                    0..=127 => {
                        out.extend_from_slice(&data[i..i + usize::from(n) + 1]);
                        i += usize::from(n) + 1;
                    }
                    _ => {
                        out.extend(std::iter::repeat_n(data[i], 257 - usize::from(n)));
                        i += 1;
                    }
                }
            }
        }
        let mut sample: Vec<u8> = (0..=255).collect();
        sample.extend([9u8; 300]);
        sample.extend([1, 2, 2, 3, 3, 3, 4]);
        assert_eq!(decode(&run_length_encode(&sample)), sample);
        assert_eq!(run_length_encode(&[5; 4]), vec![253, 5, 128]);
        assert_eq!(decode(&run_length_encode(&[])), Vec::<u8>::new());
    }

    /// Reference ASCII85 decoder used to prove the encoder round-trips.
    fn ascii85_decode_for_test(text: &str) -> Vec<u8> {
        let body = text.strip_suffix("~>").expect("ASCII85 EOD marker");
        let mut out = Vec::new();
        let mut group = Vec::new();
        for ch in body.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
            if ch == b'z' && group.is_empty() {
                out.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            group.push(ch - b'!');
            if group.len() == 5 {
                let value = group
                    .iter()
                    .fold(0u64, |acc, digit| acc * 85 + u64::from(*digit));
                out.extend_from_slice(&(value as u32).to_be_bytes());
                group.clear();
            }
        }
        if !group.is_empty() {
            let kept = group.len() - 1;
            while group.len() < 5 {
                group.push(84);
            }
            let value = group
                .iter()
                .fold(0u64, |acc, digit| acc * 85 + u64::from(*digit));
            out.extend_from_slice(&(value as u32).to_be_bytes()[..kept]);
        }
        out
    }

    #[test]
    fn ascii85_encoder_matches_reference_vectors_and_round_trips() {
        assert_eq!(ascii85_encode(b"Man "), "9jqo^~>");
        assert_eq!(ascii85_encode(&[0, 0, 0, 0]), "z~>");
        assert_eq!(ascii85_encode(b""), "~>");
        let data: Vec<u8> = (0..1000u32).map(|value| (value * 37 % 251) as u8).collect();
        for len in [1, 2, 3, 4, 5, 7, 8, 63, 64, 999, 1000] {
            let encoded = ascii85_encode(&data[..len]);
            assert!(encoded.lines().all(|line| line.len() <= 75), "{encoded}");
            assert_eq!(ascii85_decode_for_test(&encoded), &data[..len]);
            // ASCII85 costs 25% instead of hex's 100%.
            assert!(encoded.len() <= len * 5 / 4 + len / 75 + 8);
        }
    }

    #[test]
    fn jpeg_frame_components_reads_sof_marker() {
        // SOI, APP0 (len 4), SOF0 with 3 components, EOI.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00,
            0x01, 0x00, 0x02, 0x03, 0x01, 0x11, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF,
            0xD9,
        ];
        assert_eq!(jpeg_frame_components(&jpeg), Some(3));
        assert_eq!(jpeg_frame_components(&[0x00, 0x01]), None);
        // A Huffman table (C4) is not a frame header.
        let no_frame = [0xFF, 0xD8, 0xFF, 0xC4, 0x00, 0x03, 0x00, 0xFF, 0xD9];
        assert_eq!(jpeg_frame_components(&no_frame), None);
    }

    #[test]
    fn regional_colorimage_hex_refuses_short_gray_buffer() {
        let raw = RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![64],
        };
        let error = regional_colorimage_hex(&raw, "regional PS test image")
            .expect_err("short regional PS colorimage data must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("invalid regional image 2x1 x1 channels"));
    }

    #[test]
    fn regional_colorimage_hex_refuses_non_opaque_rgba() {
        let raw = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 128],
        };
        let error = regional_colorimage_hex(&raw, "regional PS test image")
            .expect_err("regional PS colorimage must not drop alpha");
        assert!(matches!(error, WellfriendError::UnsupportedFeature(_)));
        assert!(format!("{error}").contains("cannot represent fractional alpha"));
    }

    #[test]
    fn regional_colorimage_hex_allows_binary_alpha_only_with_clip_policy() {
        let raw = RawImage {
            width: 2,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 0],
        };
        let error = regional_colorimage_hex(&raw, "regional PS test image")
            .expect_err("binary alpha requires an explicit clip");
        assert!(matches!(error, WellfriendError::UnsupportedFeature(_)));
        assert!(format!("{error}").contains("requires an alpha clip"));

        let clipped =
            regional_colorimage_hex_with_alpha_policy(&raw, "regional PS test image", true)
                .expect("binary alpha should be valid under alpha clip");
        assert!(clipped.contains("FF0000"));
        assert!(clipped.contains("00FF00"));
    }

    #[test]
    fn ps_rgb_shading_function_preserves_exact_type2_exponent() {
        let function = VectorPostScriptShadingFunction::Type2(VectorPostScriptType2Function {
            c0: [1.0, 0.5, 0.0],
            c1: [0.0, 0.25, 1.0],
            domain: [0.25, 0.75],
            n: 2.5,
            range: Some([[0.0, 0.9], [0.0, 0.8], [0.0, 0.7]]),
        });
        let ps = ps_rgb_shading_function(Some(&function), &[]);
        assert!(ps.contains("/FunctionType 2"), "{ps}");
        assert!(ps.contains("/Domain [0.250000 0.750000]"), "{ps}");
        assert!(ps.contains("/C0 [1.0000 0.5000 0.0000]"), "{ps}");
        assert!(ps.contains("/C1 [0.0000 0.2500 1.0000]"), "{ps}");
        assert!(ps.contains("/N 2.500000"), "{ps}");
        assert!(
            ps.contains("/Range [0.000000 0.900000 0.000000 0.800000 0.000000 0.700000]"),
            "{ps}"
        );
    }

    #[test]
    fn ps_shading_domain_clause_is_exact_function_only() {
        let function = VectorPostScriptShadingFunction::Type2(VectorPostScriptType2Function {
            c0: [1.0, 0.5, 0.0],
            c1: [0.0, 0.25, 1.0],
            domain: [0.0, 1.0],
            n: 2.5,
            range: None,
        });
        assert_eq!(
            ps_shading_domain_clause([0.25, 0.75], Some(&function)),
            " /Domain [0.250000 0.750000]"
        );
        assert_eq!(
            ps_shading_domain_clause([0.25, 0.75], None),
            "",
            "sampled fallback gradients keep normalized stop offsets instead of emitting the PDF shading domain"
        );
        assert_eq!(
            ps_shading_domain_clause([0.0, 1.0], Some(&function)),
            "",
            "default shading domains do not need an explicit PostScript clause"
        );
    }

    #[test]
    fn ps_rgb_shading_function_preserves_exact_type2_component_array() {
        let function =
            VectorPostScriptShadingFunction::Type2RgbArray(VectorPostScriptType2RgbArrayFunction {
                channels: [
                    VectorPostScriptType2ComponentFunction {
                        c0: 1.0,
                        c1: 0.0,
                        domain: [0.0, 1.0],
                        n: 1.0,
                        range: Some([0.0, 0.95]),
                    },
                    VectorPostScriptType2ComponentFunction {
                        c0: 0.5,
                        c1: 0.25,
                        domain: [0.25, 0.75],
                        n: 2.0,
                        range: None,
                    },
                    VectorPostScriptType2ComponentFunction {
                        c0: 0.0,
                        c1: 1.0,
                        domain: [0.0, 1.0],
                        n: 3.0,
                        range: Some([0.1, 1.0]),
                    },
                ],
            });
        let ps = ps_rgb_shading_function(Some(&function), &[]);
        assert!(ps.starts_with("[ << /FunctionType 2"), "{ps}");
        assert_eq!(ps.matches("/FunctionType 2").count(), 3, "{ps}");
        assert!(ps.contains("/C0 [1.0000]"), "{ps}");
        assert!(ps.contains("/C1 [0.0000]"), "{ps}");
        assert!(ps.contains("/N 1.000000"), "{ps}");
        assert!(ps.contains("/Range [0.000000 0.950000]"), "{ps}");
        assert!(ps.contains("/C0 [0.5000]"), "{ps}");
        assert!(ps.contains("/C1 [0.2500]"), "{ps}");
        assert!(ps.contains("/Domain [0.250000 0.750000]"), "{ps}");
        assert!(ps.contains("/N 2.000000"), "{ps}");
        assert!(ps.contains("/C0 [0.0000]"), "{ps}");
        assert!(ps.contains("/C1 [1.0000]"), "{ps}");
        assert!(ps.contains("/N 3.000000"), "{ps}");
        assert!(ps.contains("/Range [0.100000 1.000000]"), "{ps}");
    }

    #[test]
    fn ps_cmyk_shading_function_preserves_exact_type2_component_array() {
        let function = VectorPostScriptShadingFunction::Type2CmykArray(
            VectorPostScriptType2CmykArrayFunction {
                channels: [
                    VectorPostScriptType2ComponentFunction {
                        c0: 0.0,
                        c1: 1.0,
                        domain: [0.0, 1.0],
                        n: 1.0,
                        range: None,
                    },
                    VectorPostScriptType2ComponentFunction {
                        c0: 1.0,
                        c1: 0.0,
                        domain: [0.0, 1.0],
                        n: 2.0,
                        range: Some([0.0, 0.85]),
                    },
                    VectorPostScriptType2ComponentFunction {
                        c0: 0.5,
                        c1: 0.25,
                        domain: [0.25, 0.75],
                        n: 3.0,
                        range: None,
                    },
                    VectorPostScriptType2ComponentFunction {
                        c0: 0.0,
                        c1: 0.5,
                        domain: [0.0, 1.0],
                        n: 4.0,
                        range: Some([0.1, 0.75]),
                    },
                ],
            },
        );
        let (color_space, ps) = ps_shading_color_space_and_function(None, Some(&function), &[]);
        assert_eq!(color_space, "/DeviceCMYK");
        assert!(ps.starts_with("[ << /FunctionType 2"), "{ps}");
        assert_eq!(ps.matches("/FunctionType 2").count(), 4, "{ps}");
        assert!(ps.contains("/C0 [0.0000]"), "{ps}");
        assert!(ps.contains("/C1 [1.0000]"), "{ps}");
        assert!(ps.contains("/N 1.000000"), "{ps}");
        assert!(ps.contains("/C0 [1.0000]"), "{ps}");
        assert!(ps.contains("/C1 [0.0000]"), "{ps}");
        assert!(ps.contains("/N 2.000000"), "{ps}");
        assert!(ps.contains("/Range [0.000000 0.850000]"), "{ps}");
        assert!(ps.contains("/C0 [0.5000]"), "{ps}");
        assert!(ps.contains("/C1 [0.2500]"), "{ps}");
        assert!(ps.contains("/Domain [0.250000 0.750000]"), "{ps}");
        assert!(ps.contains("/N 3.000000"), "{ps}");
        assert!(ps.contains("/C1 [0.5000]"), "{ps}");
        assert!(ps.contains("/N 4.000000"), "{ps}");
        assert!(ps.contains("/Range [0.100000 0.750000]"), "{ps}");
    }

    #[test]
    fn ps_cmyk_shading_function_preserves_exact_type3_stitching() {
        let function =
            VectorPostScriptShadingFunction::StitchingCmyk(VectorPostScriptCmykStitchingFunction {
                domain: [0.0, 1.0],
                range: None,
                segments: vec![
                    crate::render::vector_fallback::VectorPostScriptCmykStitchingSegment {
                        bound_end: 0.5,
                        encode: [0.0, 1.0],
                        function: VectorPostScriptType2CmykFunction {
                            c0: [0.0, 1.0, 1.0, 0.0],
                            c1: [1.0, 0.0, 0.0, 0.0],
                            domain: [0.0, 1.0],
                            n: 1.0,
                            range: None,
                        },
                    },
                    crate::render::vector_fallback::VectorPostScriptCmykStitchingSegment {
                        bound_end: 1.0,
                        encode: [0.0, 1.0],
                        function: VectorPostScriptType2CmykFunction {
                            c0: [0.25, 0.25, 0.25, 0.25],
                            c1: [0.0, 0.0, 0.0, 1.0],
                            domain: [0.25, 0.75],
                            n: 2.0,
                            range: Some([[0.0, 1.0], [0.0, 0.8], [0.0, 0.6], [0.0, 0.4]]),
                        },
                    },
                ],
            });
        let (color_space, ps) = ps_shading_color_space_and_function(None, Some(&function), &[]);
        assert_eq!(color_space, "/DeviceCMYK");
        assert!(ps.contains("/FunctionType 3"), "{ps}");
        assert!(ps.contains("/Bounds [0.500000 "), "{ps}");
        assert_eq!(ps.matches("/FunctionType 2").count(), 2, "{ps}");
        assert!(ps.contains("/C0 [0.0000 1.0000 1.0000 0.0000]"), "{ps}");
        assert!(ps.contains("/C1 [1.0000 0.0000 0.0000 0.0000]"), "{ps}");
        assert!(ps.contains("/C0 [0.2500 0.2500 0.2500 0.2500]"), "{ps}");
        assert!(ps.contains("/C1 [0.0000 0.0000 0.0000 1.0000]"), "{ps}");
        assert!(ps.contains("/Domain [0.250000 0.750000]"), "{ps}");
        assert!(ps.contains("/N 2.000000"), "{ps}");
        assert!(
            ps.contains(
                "/Range [0.000000 1.000000 0.000000 0.800000 0.000000 0.600000 0.000000 0.400000]"
            ),
            "{ps}"
        );
    }

    #[test]
    fn ps_rgb_shading_function_preserves_exact_type3_stitching() {
        let function =
            VectorPostScriptShadingFunction::Stitching(VectorPostScriptStitchingFunction {
                domain: [0.0, 1.0],
                range: None,
                segments: vec![
                    crate::render::vector_fallback::VectorPostScriptStitchingSegment {
                        bound_end: 0.5,
                        encode: [0.0, 1.0],
                        function: VectorPostScriptType2Function {
                            c0: [1.0, 0.0, 0.0],
                            c1: [0.0, 1.0, 0.0],
                            domain: [0.0, 1.0],
                            n: 1.0,
                            range: None,
                        },
                    },
                    crate::render::vector_fallback::VectorPostScriptStitchingSegment {
                        bound_end: 1.0,
                        encode: [0.0, 1.0],
                        function: VectorPostScriptType2Function {
                            c0: [0.0, 0.0, 0.0],
                            c1: [0.0, 0.0, 1.0],
                            domain: [0.0, 1.0],
                            n: 1.0,
                            range: None,
                        },
                    },
                ],
            });
        let ps = ps_rgb_shading_function(Some(&function), &[]);
        assert!(ps.contains("/FunctionType 3"), "{ps}");
        assert!(ps.contains("/Bounds [0.500000 "), "{ps}");
        assert!(
            ps.contains("/Encode [0.000000 1.000000 0.000000 1.000000 "),
            "{ps}"
        );
        assert_eq!(ps.matches("/FunctionType 2").count(), 2, "{ps}");
    }

    #[test]
    fn regional_binary_alpha_clip_path_emits_only_opaque_cells() {
        let raw = RawImage {
            width: 2,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 0],
        };
        let flat = regional_binary_alpha_clip_path(
            &raw,
            [2.0, 0.0, 0.0, 1.0, 10.0, 20.0],
            0.1,
            "regional PS test image",
        )
        .expect("binary alpha clip");
        assert_eq!(flat.subpaths.len(), 1);
        let points = &flat.subpaths[0];
        assert!(points
            .iter()
            .any(|&(x, y)| (x - 10.0).abs() < 1e-9 && (y - 20.0).abs() < 1e-9));
        assert!(points
            .iter()
            .any(|&(x, y)| (x - 11.0).abs() < 1e-9 && (y - 21.0).abs() < 1e-9));
    }

    #[test]
    fn regional_rgba_alpha_state_classifies_opaque_transparent_binary_and_fractional() {
        let opaque = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 255],
        };
        assert_eq!(
            regional_rgba_alpha_state(&opaque),
            Some(RegionalRgbaAlphaState::Opaque)
        );

        let transparent = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0],
        };
        assert_eq!(
            regional_rgba_alpha_state(&transparent),
            Some(RegionalRgbaAlphaState::FullyTransparent)
        );

        let mixed = RawImage {
            width: 2,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 0],
        };
        assert_eq!(
            regional_rgba_alpha_state(&mixed),
            Some(RegionalRgbaAlphaState::Binary)
        );

        let fractional = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 128],
        };
        assert_eq!(
            regional_rgba_alpha_state(&fractional),
            Some(RegionalRgbaAlphaState::Fractional)
        );

        let rgb = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0],
        };
        assert_eq!(regional_rgba_alpha_state(&rgb), None);
    }

    #[test]
    fn regional_stencil_mask_validation_refuses_short_mask_buffer() {
        let raw = RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![255],
        };
        let error = ensure_regional_stencil_mask(&raw, "regional PS stencil mask")
            .expect_err("short regional PS stencil masks must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("invalid regional stencil mask 2x1 x1 channels"));
    }

    #[test]
    fn multipage_document_has_conforming_dsc() {
        let pages = vec![
            PsPage {
                body: "gsave\n0 100 translate\n1 -1 scale\ngrestore\n".into(),
                width: 80,
                height: 100,
                points_per_pixel: 1.0,
                is_rasterized: false,
                has_regional_images: false,
                font_resources: PsFontResources::default(),
            },
            PsPage {
                body: "gsave\n0 120 translate\n1 -1 scale\ngrestore\n".into(),
                width: 90,
                height: 120,
                points_per_pixel: 1.0,
                is_rasterized: false,
                has_regional_images: false,
                font_resources: PsFontResources::default(),
            },
        ];
        let doc = assemble_ps_document(&pages);
        assert!(doc.starts_with("%!PS-Adobe-3.0\n"));
        assert!(doc.contains("%%Pages: 2"));
        // Bounding box is the union (max) of the two pages.
        assert!(doc.contains("%%BoundingBox: 0 0 90 120"));
        assert!(doc.contains("%%Page: 1 1"));
        assert!(doc.contains("%%Page: 2 2"));
        assert_eq!(doc.matches("showpage").count(), 2);
        assert!(doc.trim_end().ends_with("%%EOF"));
    }

    #[test]
    fn page_size_is_in_points_at_any_dpi() {
        // A US Letter page rendered at 150 DPI is 1275 x 1650 device pixels;
        // the PostScript page must still be 612 x 792 points.
        let page = PsPage {
            body: "gsave\n0.48 0.48 scale\n0 1650 translate\n1 -1 scale\ngrestore\n".into(),
            width: 1275,
            height: 1650,
            points_per_pixel: 72.0 / 150.0,
            is_rasterized: false,
            has_regional_images: false,
            font_resources: PsFontResources::default(),
        };
        let doc = assemble_ps_document(std::slice::from_ref(&page));
        assert!(doc.contains("%%BoundingBox: 0 0 612 792"), "{doc}");
        assert!(
            doc.contains("<< /PageSize [612 792] >> setpagedevice"),
            "{doc}"
        );
        let eps = assemble_eps_document(&page);
        assert!(eps.contains("%%BoundingBox: 0 0 612 792"), "{eps}");
        assert!(eps.contains("%%HiResBoundingBox: 0 0 612 792"), "{eps}");
    }

    #[test]
    fn sink_scales_device_pixels_to_points() {
        let sink = PsSink::new(1275, 1650).with_points_per_pixel(72.0 / 150.0);
        let body = sink.finish();
        assert!(body.starts_with("gsave\n0.48 0.48 scale\n0 1650 translate\n1 -1 scale\n"));
    }

    #[test]
    fn eps_document_is_epsf_with_precise_bbox_and_no_showpage() {
        let page = PsPage {
            body: "gsave\n0 50 translate\n1 -1 scale\n1 0 0 setrgbcolor\ngrestore\n".into(),
            width: 42,
            height: 50,
            points_per_pixel: 1.0,
            is_rasterized: false,
            has_regional_images: false,
            font_resources: PsFontResources::default(),
        };
        let eps = assemble_eps_document(&page);
        assert!(eps.starts_with("%!PS-Adobe-3.0 EPSF-3.0\n"));
        assert!(eps.contains("%%BoundingBox: 0 0 42 50"));
        assert!(eps.contains("%%HiResBoundingBox: 0 0 42 50"));
        // EPS conformance: no setpagedevice, no showpage.
        assert!(
            !eps.contains("setpagedevice"),
            "EPS must not call setpagedevice"
        );
        assert!(!eps.contains("showpage"), "EPS must not call showpage");
        assert!(eps.trim_end().ends_with("%%EOF"));
    }
}
