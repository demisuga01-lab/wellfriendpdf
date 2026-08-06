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
//! (Form XObjects, shadings, tiling/shading patterns, soft masks, non-trivial
//! blend modes) fall back to embedding the **whole page as one rasterised
//! image** drawn with the PostScript `image`/`colorimage` operator
//! (pixel-identical to the raster render).
//!
//! # Regional image fallback (RB-14)
//!
//! Pages with simple axis-aligned Image XObject `Do` operations now use a
//! **regional fallback**: the image is embedded as a bounded `colorimage`
//! region at its correct device-space position, while surrounding vector
//! content (paths, text, clips) is preserved as native PostScript. This avoids
//! whole-page rasterization for the common "vector page with one logo/figure"
//! case. Eligibility is determined by
//! [`crate::render::vector_fallback::classify_page_for_vector_output`].

use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::{Color, ColorSpace, GraphicsState};
use crate::engine::{ContentEngine, PageResources};
use crate::error::Result;
use crate::render::color::{ColorSpaceHandler, RenderColor};
use crate::render::glyph_outline::{
    extract_glyph_path_by_gid, extract_glyph_path_for_simple, font_size_scale, get_upem,
};
use crate::render::path::{flatten_path, FillRule, FlatPath, Path};
use crate::render::text_decode::{decode_text_bytes, get_font_bytes, DecodedGlyph};
use crate::render::transform::{Transform2D, Viewport};
use crate::render::vector_fallback::{
    classify_page_for_vector_output, image_device_rect, VectorFallbackDecision,
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
    /// True when the page used regional image fallback: vector content is
    /// preserved natively around bounded embedded image regions.
    pub has_regional_images: bool,
}

/// Render a single page to a PostScript page body. Uses the shared fallback
/// classifier to decide: pure vector, regional image fallback, or whole-page
/// raster.
pub fn render_page_ps(engine: &ContentEngine, page_number: usize, dpi: u32) -> Result<PsPage> {
    let viewport = engine.page_viewport(page_number, dpi)?;
    let ops = engine.get_page_content(page_number)?;
    let resources = engine.get_page_resources(page_number)?;

    let decision = classify_page_for_vector_output(&ops, &resources, viewport.scale);

    match decision {
        VectorFallbackDecision::WholePageRaster { .. } => {
            rasterized_page(engine, page_number, &viewport)
        }
        VectorFallbackDecision::PureVector => {
            render_vector_ps(engine, &ops, &resources, &viewport, &[])
        }
        VectorFallbackDecision::RegionalImageFallback { ref image_names } => {
            render_vector_ps(engine, &ops, &resources, &viewport, image_names)
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
    regional_image_names: &[String],
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
        regional_image_names: regional_image_names.to_vec(),
    };
    state.run(ops);
    let has_regional = !regional_image_names.is_empty();
    Ok(PsPage {
        body: sink.finish(),
        width: w,
        height: h,
        is_rasterized: false,
        has_regional_images: has_regional,
    })
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
    out.push_str("%%LanguageLevel: 2\n");
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
    out.push_str("%%LanguageLevel: 2\n");
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
        let mut hex = String::with_capacity(rgb.len() * 2 + rgb.len() / 32);
        const HEXCHARS: &[u8; 16] = b"0123456789ABCDEF";
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
        self.body.contains("picstr")
    }

    /// Whether the page body emits a regional raster image (it needs the
    /// `regionpicstr` scratch string declared in the prologue).
    fn body_needs_regionpicstr(&self) -> bool {
        self.body.contains("regionpicstr")
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
        if self.body_needs_regionpicstr() {
            // Scratch string for regional images — size is max row width * 3.
            // We use a generous fixed size; PostScript `readhexstring` only reads
            // as many chars as the string length, so oversizing is safe.
            let max_row = self.width.max(4096) as usize * 3;
            out.push_str(&format!("/regionpicstr {} string def\n", max_row));
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
}

impl PsRenderState<'_> {
    fn run(&mut self, ops: &[ContentOperation]) {
        for op in ops {
            self.dispatch(op);
        }
    }

    fn ctm(&self) -> Transform2D {
        Transform2D::from(self.gs.ctm)
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
                        self.emit_regional_image(name);
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
    fn emit_regional_image(&mut self, name: &str) {
        use crate::images::locator::ImageReference;

        let page_h = self.viewport.height_px as f64;
        let rect = match image_device_rect(&self.gs, self.viewport.scale, page_h) {
            Some(r) => r,
            None => return,
        };

        let [x, y, w, h] = rect;

        let (obj_num, gen_num) = match self.resources.xobjects.get(name) {
            Some(&(o, g)) => (o, g),
            None => return,
        };

        let reader = self.engine.document().reader();
        let dict = match reader.get_object(obj_num, gen_num) {
            Ok(crate::object::PdfObject::Stream { dict, .. }) => dict,
            _ => return,
        };

        let image_ref = ImageReference {
            page_number: 0,
            xobject_name: name.to_string(),
            object_number: obj_num,
            generation_number: gen_num,
            width: dict
                .get_integer("Width")
                .or_else(|| dict.get_integer("W"))
                .unwrap_or(0)
                .max(0) as u32,
            height: dict
                .get_integer("Height")
                .or_else(|| dict.get_integer("H"))
                .unwrap_or(0)
                .max(0) as u32,
            bits_per_component: dict
                .get_integer("BitsPerComponent")
                .or_else(|| dict.get_integer("BPC"))
                .unwrap_or(8)
                .clamp(1, 16) as u8,
            color_space: "DeviceRGB".to_string(),
            filter: Vec::new(),
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };

        let raw = match self.engine.decode_image(&image_ref) {
            Ok(r) => r,
            Err(_) => return,
        };

        // Emit the image as a regional colorimage at the computed device rect.
        // PostScript coordinate system is already flipped (y-down) by our prologue,
        // so we can position directly with gsave/translate/scale.
        let iw = raw.width;
        let ih = raw.height;
        let rgb_pixels = &raw.pixels;
        let channels = raw.channels as usize;

        self.sink.push_line("gsave");
        self.sink.push_line(&format!("{x:.2} {y:.2} translate"));
        self.sink.push_line(&format!("{w:.2} {h:.2} scale"));
        self.sink
            .push_line(&format!("{iw} {ih} 8 [{iw} 0 0 {ih} 0 0]"));
        self.sink
            .push_line("{currentfile regionpicstr readhexstring pop} false 3 colorimage");

        // Emit hex RGB data.
        const HEXCHARS: &[u8; 16] = b"0123456789ABCDEF";
        let mut hex =
            String::with_capacity(iw as usize * ih as usize * 6 + iw as usize * ih as usize / 13);
        let mut col = 0usize;
        for row in 0..ih {
            for px in 0..iw {
                let offset = (row as usize * iw as usize + px as usize) * channels;
                let (r, g, b) = if channels >= 3 {
                    (
                        rgb_pixels[offset],
                        rgb_pixels[offset + 1],
                        rgb_pixels[offset + 2],
                    )
                } else {
                    // Grayscale → expand to RGB.
                    let v = rgb_pixels.get(offset).copied().unwrap_or(0);
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
        self.sink.body.push_str(&hex);
        self.sink.body.push('\n');
        self.sink.push_line("grestore");
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
            let flat = flatten_path(&self.path, &ctm, &self.viewport, 0.3);
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
        let ctm = self.ctm();
        let flat = flatten_path(&self.path, &ctm, &self.viewport, 0.3);
        if flat.subpaths.iter().all(|s| s.is_empty()) {
            return;
        }
        let (color, alpha) = self.resolve_color(&self.gs.fill_color, self.gs.fill_alpha as f32);
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
        let ctm = self.ctm();
        let flat = flatten_path(&self.path, &ctm, &self.viewport, 0.3);
        if flat.subpaths.iter().all(|s| s.is_empty()) {
            return;
        }
        let (color, alpha) = self.resolve_color(&self.gs.stroke_color, self.gs.stroke_alpha as f32);
        self.emit_setcolor(color, alpha);
        let width = self.device_line_width();
        self.sink.push_line(&format!("{width:.3} setlinewidth"));
        self.emit_line_style();
        self.sink.push_line("newpath");
        self.sink.append_path(&flat);
        self.sink.push_line("stroke");
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

    fn device_line_width(&self) -> f64 {
        let w = self.gs.line_width * self.device_scale();
        if w <= 0.0 {
            1.0
        } else {
            w
        }
    }

    /// Emit `setrgbcolor`. PostScript Level 2 has no constant-alpha operator;
    /// when alpha < 1 we approximate by blending the colour toward white (the
    /// page background), which keeps a single conforming code path. (True
    /// constant-alpha transparency would require a `.setopacityalpha`-style
    /// extension or the rasterise-embed fallback; documented as a follow-up.)
    fn emit_setcolor(&mut self, color: RenderColor, alpha: f32) {
        let a = alpha.clamp(0.0, 1.0);
        let (r, g, b) = if a >= 0.999 {
            (color.r, color.g, color.b)
        } else {
            (
                color.r * a + (1.0 - a),
                color.g * a + (1.0 - a),
                color.b * a + (1.0 - a),
            )
        };
        self.sink
            .push_line(&format!("{r:.4} {g:.4} {b:.4} setrgbcolor"));
    }

    /// Resolve a graphics-state colour to (`RenderColor`, alpha), mirroring the
    /// SVG sink (named colour spaces resolved through the page resources).
    fn resolve_color(&self, color: &Color, alpha: f32) -> (RenderColor, f32) {
        if let ColorSpace::Named(name) = &color.space {
            if let Some(space_obj) = self.resources.color_spaces.get(name) {
                let reader = self.engine.document().reader();
                if let crate::render::colorspace::NamedColor::Color(rc) =
                    crate::render::colorspace::resolve_named_color(
                        space_obj,
                        &color.components,
                        alpha,
                        reader,
                    )
                {
                    return (rc, rc.a);
                }
            }
        }
        let rc = ColorSpaceHandler::to_render_color(color, alpha);
        (rc, rc.a)
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
        let decoded = decode_text_bytes(bytes, &font_name, &self.resources, reader);
        let font_bytes = get_font_bytes(&font_name, &self.resources, reader);
        let upem = font_bytes
            .as_ref()
            .and_then(|b| get_upem(b))
            .map(f64::from)
            .filter(|v| *v > 0.0)
            .unwrap_or(1000.0);

        for glyph in decoded {
            let mut ttf_advance = None;
            if !matches!(self.gs.text.rendering_mode, 3 | 7) {
                if let Some(fb) = font_bytes.as_ref() {
                    if !fb.is_empty() {
                        ttf_advance = self.emit_glyph(&glyph, fb, upem);
                    }
                }
            }
            let advance = glyph.width.or(ttf_advance).unwrap_or(500.0);
            self.advance_decoded_text(advance, &glyph);
        }
    }

    /// Emit one glyph as a filled (or stroked) PostScript path outline, in
    /// device space. Returns the glyph's advance width (1/1000 text units).
    fn emit_glyph(&mut self, glyph: &DecodedGlyph, font_bytes: &[u8], upem: f64) -> Option<f64> {
        let (outline, advance) = if glyph.is_gid {
            extract_glyph_path_by_gid(font_bytes, glyph.code)
        } else {
            extract_glyph_path_for_simple(
                font_bytes,
                glyph.code,
                glyph.unicode,
                glyph.glyph_name.as_deref(),
            )
        };
        let Some(glyph_path) = outline else {
            return Some(advance);
        };

        let scale = font_size_scale(self.gs.text.font_size, upem);
        let th = self.gs.text.horizontal_scaling / 100.0;
        let scale_x = scale * th;
        if scale <= 0.0 || !scale_x.is_finite() {
            return Some(advance);
        }
        let glyph_ctm = Transform2D::scale(scale_x, scale)
            .concat(&Transform2D::translation(0.0, self.gs.text.rise))
            .concat(&Transform2D::from(self.gs.text.tm))
            .concat(&self.ctm());
        let flat = flatten_path(&glyph_path, &glyph_ctm, &self.viewport, 0.3);
        if flat.subpaths.iter().all(|s| s.is_empty()) {
            return Some(advance);
        }

        match self.gs.text.rendering_mode {
            1 | 5 => {
                let (color, a) =
                    self.resolve_color(&self.gs.stroke_color, self.gs.stroke_alpha as f32);
                self.emit_setcolor(color, a);
                let w = self.device_line_width();
                self.sink.push_line(&format!("{w:.3} setlinewidth"));
                self.sink.push_line("newpath");
                self.sink.append_path(&flat);
                self.sink.push_line("stroke");
            }
            _ => {
                let (color, a) = self.resolve_color(&self.gs.fill_color, self.gs.fill_alpha as f32);
                self.emit_setcolor(color, a);
                self.sink.push_line("newpath");
                self.sink.append_path(&flat);
                // Glyph outlines use the nonzero winding rule.
                self.sink.push_line("fill");
            }
        }
        Some(advance)
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

    fn advance_decoded_text(&mut self, glyph_width: f64, glyph: &DecodedGlyph) {
        if !glyph.is_vertical {
            self.advance_text(glyph_width, glyph.is_space);
            return;
        }
        let mut advance_y =
            glyph.vertical_advance.unwrap_or(-1000.0) / 1000.0 * self.gs.text.font_size;
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

        // Shading → whole-page.
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

        // Axis-aligned Image XObject → regional fallback.
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
            VectorFallbackDecision::RegionalImageFallback { .. }
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
