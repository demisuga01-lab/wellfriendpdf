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
//!   [`text_decode`](crate::render::text_decode) helpers for text-as-outlines.
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
    classify_page_for_postscript_output_with_reader, classify_scoped_postscript_vector_output,
    decode_inline_image_region, ensure_regional_raw_image, ensure_regional_stencil_mask,
    image_device_placement, inline_mask_sample_paints, load_vector_form_program,
    load_vector_shading_for_postscript_output, load_vector_shading_pattern_for_postscript_output,
    load_vector_tiling_pattern, merged_vector_resources,
    resolved_regional_image_color_space_override, stencil_mask_paints_ones,
    vector_uncolored_tiling_paint_color, VectorFallbackDecision,
    VectorPostScriptAlternateColorSpace, VectorPostScriptCmykStitchingFunction,
    VectorPostScriptNamedColorFamily, VectorPostScriptShadingColorSpace,
    VectorPostScriptShadingFunction, VectorPostScriptStitchingFunction,
    VectorPostScriptTintStitchingFunction, VectorPostScriptType2CmykArrayFunction,
    VectorPostScriptType2CmykFunction, VectorPostScriptType2ComponentFunction,
    VectorPostScriptType2Function, VectorPostScriptType2RgbArrayFunction, VectorShading,
    VectorShadingStop, VectorTilingPatternPaintType, VectorTilingPatternProgram,
    MAX_PATTERN_STENCIL_CLIP_RECTS, MAX_VECTOR_FORM_DEPTH, MAX_VECTOR_TILING_PATTERN_CELLS,
};

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
    /// True when the whole page was embedded as a raster image because it used
    /// operations the vector sink cannot express natively.
    pub is_rasterized: bool,
    /// True when the page used regional vector fallback: vector content is
    /// preserved natively around bounded images or vector-safe Form subprograms.
    pub has_regional_images: bool,
}

pub fn render_page_ps(engine: &ContentEngine, page_number: usize, dpi: u32) -> Result<PsPage> {
    render_page_ps_with_policy(engine, page_number, dpi, PsWholePageRasterPolicy::Allow)
}

/// Render a single page to PostScript without silently rasterizing the whole
/// page when the vector sink cannot represent the page exactly.
pub fn render_page_ps_strict(
    engine: &ContentEngine,
    page_number: usize,
    dpi: u32,
) -> Result<PsPage> {
    render_page_ps_with_policy(engine, page_number, dpi, PsWholePageRasterPolicy::Refuse)
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
            PsWholePageRasterPolicy::Allow => rasterized_page(engine, page_number, &viewport),
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
            ) {
                Ok(page) => Ok(page),
                Err(WellfriendError::UnsupportedFeature(_)) => match whole_page_policy {
                    PsWholePageRasterPolicy::Allow => rasterized_page(engine, page_number, &viewport),
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
) -> Result<PsPage> {
    let (w, h) = (viewport.width_px, viewport.height_px);
    let mut sink = PsSink::new(w, h);
    let mut state = PsRenderState {
        engine,
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
    Ok(PsPage {
        body: sink.finish(),
        width: w,
        height: h,
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
    let context = format!("regional PS image /{name}");
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

/// Emit a page as a single embedded raster image drawn with `colorimage`
/// (the fallback). The RGB samples are emitted as ASCII-hex so the output is a
/// pure 7-bit-clean conforming PostScript stream (no binary).
fn rasterized_page(
    engine: &ContentEngine,
    page_number: usize,
    viewport: &Viewport,
) -> Result<PsPage> {
    let buf = engine.render_page(page_number, viewport.dpi)?;
    let raw = buf.to_raw_image(); // 3 channels, 8-bit
    let (w, h) = (viewport.width_px, viewport.height_px);

    let mut sink = PsSink::new(w, h);
    sink.emit_raster_image(&raw.pixels, w, h);
    Ok(PsPage {
        body: sink.finish(),
        width: w,
        height: h,
        is_rasterized: true,
        has_regional_images: false,
    })
}

/// Assemble one or more [`PsPage`] bodies into a complete, DSC-conformant
/// multi-page PostScript document (`%!PS-Adobe-3.0`).
pub fn assemble_ps_document(pages: &[PsPage]) -> String {
    let mut out = String::new();
    out.push_str("%!PS-Adobe-3.0\n");
    out.push_str("%%Creator: Wellfriend PDF SDK Toolkit\n");
    let language_level = if pages.iter().any(|page| page.body.contains("shfill")) {
        3
    } else {
        2
    };
    out.push_str(&format!("%%LanguageLevel: {language_level}\n"));
    // The bounding box of a multi-page document is the union; DSC permits the
    // largest page. We report the max width/height across pages.
    let max_w = pages.iter().map(|p| p.width).max().unwrap_or(0);
    let max_h = pages.iter().map(|p| p.height).max().unwrap_or(0);
    out.push_str(&format!("%%BoundingBox: 0 0 {max_w} {max_h}\n"));
    out.push_str(&format!("%%Pages: {}\n", pages.len()));
    out.push_str("%%EndComments\n");
    out.push_str("%%BeginProlog\n");
    out.push_str("%%EndProlog\n");
    out.push_str("%%BeginSetup\n");
    out.push_str("%%EndSetup\n");

    for (idx, page) in pages.iter().enumerate() {
        let n = idx + 1;
        out.push_str(&format!("%%Page: {n} {n}\n"));
        out.push_str(&format!(
            "%%PageBoundingBox: 0 0 {} {}\n",
            page.width, page.height
        ));
        // Each page sets its own media size so viewers/printers select the
        // right page geometry.
        out.push_str(&format!(
            "<< /PageSize [{} {}] >> setpagedevice\n",
            page.width, page.height
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
    let language_level = if page.body.contains("shfill") { 3 } else { 2 };
    out.push_str(&format!("%%LanguageLevel: {language_level}\n"));
    out.push_str(&format!(
        "%%BoundingBox: 0 0 {} {}\n",
        page.width, page.height
    ));
    // A fractional high-resolution bounding box is identical here (integer
    // device pixels), but DSC encourages emitting it for EPS.
    out.push_str(&format!(
        "%%HiResBoundingBox: 0 0 {}.0 {}.0\n",
        page.width, page.height
    ));
    out.push_str("%%EndComments\n");
    out.push_str("%%BeginProlog\n");
    out.push_str("%%EndProlog\n");
    // The page body is wrapped in its own gsave/grestore (added by PsSink), so
    // it does not leak graphics state to an embedding document. No showpage.
    out.push_str(&page.body);
    out.push_str("%%EOF\n");
    out
}

/// Accumulates PostScript operators for one page body and emits it wrapped in a
/// `gsave` + coordinate-flip prologue and a matching `grestore`.
struct PsSink {
    width: u32,
    height: u32,
    body: String,
}

impl PsSink {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            body: String::new(),
        }
    }

    fn push_line(&mut self, line: &str) {
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
        self.push_line(&format!("{w} {h} 8 [{w} 0 0 {h} 0 0]"));
        self.push_line("{currentfile picstr readhexstring pop} false 3 colorimage");
        // `picstr` is a per-row scratch string defined in the body prologue.
        // Emit the hex sample data, wrapped to a sane line width.
        const HEXCHARS: &[u8; 16] = b"0123456789ABCDEF";
        let mut hex = String::with_capacity(rgb.len() * 2 + rgb.len() / 32);
        let mut col = 0usize;
        for &byte in rgb {
            hex.push(HEXCHARS[(byte >> 4) as usize] as char);
            hex.push(HEXCHARS[(byte & 0xf) as usize] as char);
            col += 2;
            if col >= 78 {
                hex.push('\n');
                col = 0;
            }
        }
        self.body.push_str(&hex);
        self.body.push('\n');
    }

    /// Whether the page body emits a raster image (it needs the `picstr` scratch
    /// string declared in the prologue).
    fn body_needs_picstr(&self) -> bool {
        self.body.contains("currentfile picstr readhexstring")
    }

    /// Finish the page body: wrap it with `gsave`, the top-left device-space
    /// coordinate flip, any required scratch declarations, and `grestore`.
    fn finish(self) -> String {
        let mut out = String::new();
        out.push_str("gsave\n");
        // Map PostScript bottom-left/y-up to our top-left/y-down device space.
        out.push_str(&format!("0 {} translate\n", self.height));
        out.push_str("1 -1 scale\n");
        if self.body_needs_picstr() {
            // Scratch string holding one image row (width * 3 RGB bytes).
            out.push_str(&format!("/picstr {} string def\n", self.width as usize * 3));
        }
        out.push_str(&self.body);
        out.push_str("grestore\n");
        out
    }
}

/// PostScript sibling of `RenderState`/`SvgRenderState`: same interpretation,
/// PostScript emission.
struct PsRenderState<'a> {
    engine: &'a ContentEngine,
    resources: PageResources,
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
        let dict = match reader.get_object(obj_num, gen_num) {
            Ok(crate::object::PdfObject::Stream { dict, .. }) => dict,
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
        let hex = regional_colorimage_hex_with_alpha_policy(raw, context, allow_binary_alpha)?;
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
        self.sink
            .push_line(&format!("{iw} {ih} 8 [{iw} 0 0 {ih} 0 0]"));
        let row_bytes = iw as usize * 3;
        self.sink
            .push_line(&format!("/regionpicstr {row_bytes} string def"));
        self.sink
            .push_line("{currentfile regionpicstr readhexstring pop} false 3 colorimage");

        self.sink.body.push_str(&hex);
        self.sink.body.push('\n');
        self.sink.push_line("grestore");
        Ok(())
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
        self.sink
            .push_line(&format!("{iw} {ih} true [{iw} 0 0 {ih} 0 0]"));
        self.sink
            .push_line(&format!("/regionmaskstr {row_bytes} string def"));
        self.sink
            .push_line("{currentfile regionmaskstr readhexstring pop} imagemask");

        const HEXCHARS: &[u8; 16] = b"0123456789ABCDEF";
        let channels = raw.channels as usize;
        let mut hex = String::with_capacity(row_bytes * ih as usize * 2 + ih as usize);
        let mut col = 0usize;
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
                hex.push(HEXCHARS[(packed >> 4) as usize] as char);
                hex.push(HEXCHARS[(packed & 0x0f) as usize] as char);
                col += 2;
                if col >= 78 {
                    hex.push('\n');
                    col = 0;
                }
            }
        }
        self.sink.body.push_str(&hex);
        self.sink.body.push('\n');
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
        let saved_image_names = self.regional_image_names.clone();
        let saved_inline_count = self.regional_inline_image_count;
        let saved_form_names = self.regional_form_names.clone();
        let saved_shading_names = self.regional_shading_names.clone();
        let saved_pending_inline = self.pending_inline_params.take();

        let form_resources = merged_vector_resources(program.resources.as_ref(), &self.resources);
        self.resources = form_resources;
        let form_t = Transform2D::from(program.form_matrix);
        let current_t = Transform2D::from(saved_gs.ctm);
        self.gs.ctm = form_t.concat(&current_t).to_array();

        self.form_depth += 1;
        self.form_object_stack.push(form_key);

        let decision = classify_scoped_postscript_vector_output(
            &program.ops,
            &self.resources,
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
        }

        self.form_object_stack.pop();
        self.form_depth = self.form_depth.saturating_sub(1);
        self.gs = saved_gs;
        self.resources = saved_resources;
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
            let flat = flatten_path(
                &self.path,
                &ctm,
                &self.viewport,
                self.gs.path_flatness_tolerance(),
            );
            self.sink.append_path(&flat);
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
        let flat = flatten_path(
            &self.path,
            &ctm,
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        if flat.subpaths.iter().all(|s| s.is_empty()) {
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
            self.emit_shading_pattern_clip(self.gs.fill_pattern_name.clone(), &flat, rule, "fill");
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
        self.sink.push_line("newpath");
        self.sink.append_path(&flat);
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
        let flat = flatten_path(
            &self.path,
            &ctm,
            &self.viewport,
            self.gs.path_flatness_tolerance(),
        );
        if flat.subpaths.iter().all(|s| s.is_empty()) {
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
                "stroke",
            );
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
        self.sink.push_line("newpath");
        self.sink.append_path(&flat);
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
            if !matches!(self.gs.text.rendering_mode, 3) {
                let Some(fb) = font_program else {
                    self.record_fatal_error(WellfriendError::UnsupportedFeature(
                        "PostScript text font program unavailable for vector output".to_string(),
                    ));
                    return;
                };
                if decoded_glyph_strict_outline(&glyph, fb).is_none() {
                    self.record_fatal_error(WellfriendError::UnsupportedFeature(
                        "PostScript text glyph outline unavailable for vector output".to_string(),
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

    /// Emit one glyph as a filled (or stroked) PostScript path outline, in
    /// device space.
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
        if flat.subpaths.iter().all(|s| s.is_empty()) {
            return;
        }
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
                        self.sink.append_path(&flat);
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
                        self.sink.append_path(&flat);
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
                        self.sink.append_path(&flat);
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
                        self.sink.append_path(&flat);
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
    for pixel in raw.pixels.chunks_exact(4) {
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
        "{:.4} {:.4} {:.4}",
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0)
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
    if exact.is_some() && ((domain[0]).abs() > 1e-9 || (domain[1] - 1.0).abs() > 1e-9) {
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
        bounds.push_str(&format!("{:.6} ", stop.offset.clamp(0.0, 1.0)));
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
        "<< /FunctionType 2 /Domain {} /C0 [{c0}] /C1 [{c1}] /N {:.6}{range} >>",
        ps_function_domain(function.domain),
        function.n
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
        "<< /FunctionType 2 /Domain {} /C0 [{:.4}] /C1 [{:.4}] /N {:.6}{range} >>",
        ps_function_domain(function.domain),
        function.c0.clamp(0.0, 1.0),
        function.c1.clamp(0.0, 1.0),
        function.n
    )
}

fn ps_cmyk_exact_type2_function(function: &VectorPostScriptType2CmykFunction) -> String {
    let c0 = ps_cmyk_components_f64(function.c0);
    let c1 = ps_cmyk_components_f64(function.c1);
    let range = ps_function_range(function.range.as_ref());
    format!(
        "<< /FunctionType 2 /Domain {} /C0 [{c0}] /C1 [{c1}] /N {:.6}{range} >>",
        ps_function_domain(function.domain),
        function.n
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
    let mut functions = String::new();
    let mut bounds = String::new();
    let mut encode = String::new();
    for segment in &function.segments {
        functions.push_str(&ps_rgb_exact_type2_function(&segment.function));
        functions.push(' ');
        encode.push_str(&format!(
            "{:.6} {:.6} ",
            segment.encode[0], segment.encode[1]
        ));
    }
    for segment in function
        .segments
        .iter()
        .take(function.segments.len().saturating_sub(1))
    {
        bounds.push_str(&format!("{:.6} ", segment.bound_end));
    }
    format!(
        "<< /FunctionType 3 /Domain {} /Functions [ {functions}] /Bounds [{bounds}] /Encode [{encode}] >>",
        ps_function_domain(function.domain)
    )
}

fn ps_cmyk_exact_stitching_function(function: &VectorPostScriptCmykStitchingFunction) -> String {
    let mut functions = String::new();
    let mut bounds = String::new();
    let mut encode = String::new();
    for segment in &function.segments {
        functions.push_str(&ps_cmyk_exact_type2_function(&segment.function));
        functions.push(' ');
        encode.push_str(&format!(
            "{:.6} {:.6} ",
            segment.encode[0], segment.encode[1]
        ));
    }
    for segment in function
        .segments
        .iter()
        .take(function.segments.len().saturating_sub(1))
    {
        bounds.push_str(&format!("{:.6} ", segment.bound_end));
    }
    format!(
        "<< /FunctionType 3 /Domain {} /Functions [ {functions}] /Bounds [{bounds}] /Encode [{encode}] >>",
        ps_function_domain(function.domain)
    )
}

fn ps_tint_exact_stitching_function(function: &VectorPostScriptTintStitchingFunction) -> String {
    let mut functions = String::new();
    let mut bounds = String::new();
    let mut encode = String::new();
    for segment in &function.segments {
        functions.push_str(&ps_rgb_exact_type2_component_function(&segment.function));
        functions.push(' ');
        encode.push_str(&format!(
            "{:.6} {:.6} ",
            segment.encode[0], segment.encode[1]
        ));
    }
    for segment in function
        .segments
        .iter()
        .take(function.segments.len().saturating_sub(1))
    {
        bounds.push_str(&format!("{:.6} ", segment.bound_end));
    }
    format!(
        "<< /FunctionType 3 /Domain {} /Functions [ {functions}] /Bounds [{bounds}] /Encode [{encode}] >>",
        ps_function_domain(function.domain)
    )
}

fn ps_function_domain(domain: [f64; 2]) -> String {
    format!("[{:.6} {:.6}]", domain[0], domain[1])
}

fn ps_function_range<const N: usize>(range: Option<&[[f64; 2]; N]>) -> String {
    let Some(range) = range else {
        return String::new();
    };
    let mut values = String::new();
    for pair in range {
        values.push_str(&format!(
            "{:.6} {:.6} ",
            pair[0].clamp(0.0, 1.0),
            pair[1].clamp(0.0, 1.0)
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
        "{:.4} {:.4} {:.4}",
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0)
    )
}

fn ps_cmyk_components_f64(color: [f64; 4]) -> String {
    format!(
        "{:.4} {:.4} {:.4} {:.4}",
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0),
        color[3].clamp(0.0, 1.0)
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

fn regional_colorimage_hex_with_alpha_policy(
    raw: &RawImage,
    context: &str,
    allow_binary_alpha: bool,
) -> Result<String> {
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

    const HEXCHARS: &[u8; 16] = b"0123456789ABCDEF";
    let mut hex = String::with_capacity(iw * ih * 6 + iw * ih / 13);
    let mut col = 0usize;
    for row in 0..ih {
        for px in 0..iw {
            let offset = (row * iw + px) * channels;
            let (r, g, b) = if channels >= 3 {
                (
                    raw.pixels[offset],
                    raw.pixels[offset + 1],
                    raw.pixels[offset + 2],
                )
            } else {
                let v = raw.pixels[offset];
                (v, v, v)
            };
            hex.push(HEXCHARS[(r >> 4) as usize] as char);
            hex.push(HEXCHARS[(r & 0xf) as usize] as char);
            hex.push(HEXCHARS[(g >> 4) as usize] as char);
            hex.push(HEXCHARS[(g & 0xf) as usize] as char);
            hex.push(HEXCHARS[(b >> 4) as usize] as char);
            hex.push(HEXCHARS[(b & 0xf) as usize] as char);
            col += 6;
            if col >= 78 {
                hex.push('\n');
                col = 0;
            }
        }
    }
    Ok(hex)
}

fn ps_concat_matrix(transform: [f64; 6]) -> String {
    let [a, b, c, d, e, f] = transform;
    format!("[{a:.6} {b:.6} {c:.6} {d:.6} {e:.6} {f:.6}] concat")
}

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
        sink.emit_raster_image(&[255, 0, 0, 0, 255, 0], 2, 1);
        let out = sink.finish();
        assert!(out.contains("/picstr 6 string def"), "{out}");
        assert!(out.contains("colorimage"));
        assert!(out.contains("FF0000"));
        assert!(out.contains("00FF00"));
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
                is_rasterized: false,
                has_regional_images: false,
            },
            PsPage {
                body: "gsave\n0 120 translate\n1 -1 scale\ngrestore\n".into(),
                width: 90,
                height: 120,
                is_rasterized: false,
                has_regional_images: false,
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
    fn eps_document_is_epsf_with_precise_bbox_and_no_showpage() {
        let page = PsPage {
            body: "gsave\n0 50 translate\n1 -1 scale\n1 0 0 setrgbcolor\ngrestore\n".into(),
            width: 42,
            height: 50,
            is_rasterized: false,
            has_regional_images: false,
        };
        let eps = assemble_eps_document(&page);
        assert!(eps.starts_with("%!PS-Adobe-3.0 EPSF-3.0\n"));
        assert!(eps.contains("%%BoundingBox: 0 0 42 50"));
        assert!(eps.contains("%%HiResBoundingBox: 0 0 42.0 50.0"));
        // EPS conformance: no setpagedevice, no showpage.
        assert!(
            !eps.contains("setpagedevice"),
            "EPS must not call setpagedevice"
        );
        assert!(!eps.contains("showpage"), "EPS must not call showpage");
        assert!(eps.trim_end().ends_with("%%EOF"));
    }
}
