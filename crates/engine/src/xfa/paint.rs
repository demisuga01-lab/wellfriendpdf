//! Paints laid-out XFA pages into PDF content streams, following PDFium's
//! XFA widget drawing (`CXFA_FFWidget`, `CXFA_FFField` caption placement,
//! `CXFA_Box`/`CXFA_Rectangle` borders with edges, corners, hand and 3-D
//! strokes, `CXFA_Fill` solid/linear/radial/pattern/stipple fills, check
//! button marks, list/combo boxes, comb fields, images, lines and arcs).
//!
//! Content is produced in XFA page space (points, y down); the caller maps
//! it onto the PDF page with a flipping `cm`. Text uses `Tm` with a
//! negative `d` so glyphs stay upright in that space.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::object::{PdfDictionary, PdfObject};
use crate::render::annotation_synthesis::{ellipse_path, n};

use super::form::{FormNode, ImageValue, NodeKind, Template};
use super::layout::{
    caption_spec_for, field_ui, node_font_for, node_margin, node_para, LayoutPage, Placed, Ui,
};
use super::locale::LocaleSet;
use super::picture::format_value_in;
use super::style::{
    presence, BoxKind, BoxSpec, CaptionPlacement, FillKind, FillSpec, HAlign, Hand, Insets,
    LinearDir, ParaSpec, PatternKind, Presence, Rgb, Stroke, StrokeType, VAlign, BLACK,
};
use super::text::{
    emit_block, layout_paragraphs, layout_paragraphs_with, plain_paragraphs, rich_paragraphs,
    FontBook, Paragraph, TextBlock,
};
use super::xml::XmlNode;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    fn deflate(self, insets: Insets) -> Self {
        Self {
            x: self.x + insets.left,
            y: self.y + insets.top,
            w: self.w - insets.left - insets.right,
            h: self.h - insets.top - insets.bottom,
        }
    }

    fn normalized(self) -> Self {
        let (x, w) = if self.w < 0.0 {
            (self.x + self.w, -self.w)
        } else {
            (self.x, self.w)
        };
        let (y, h) = if self.h < 0.0 {
            (self.y + self.h, -self.h)
        } else {
            (self.y, self.h)
        };
        Self { x, y, w, h }
    }

    fn right(&self) -> f64 {
        self.x + self.w
    }

    fn bottom(&self) -> f64 {
        self.y + self.h
    }

    fn intersect(self, other: Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }
}

fn rgb(color: Rgb) -> String {
    format!(
        "{} {} {}",
        n(f64::from(color[0]) / 255.0),
        n(f64::from(color[1]) / 255.0),
        n(f64::from(color[2]) / 255.0)
    )
}

fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let channel = |i: usize| (f64::from(a[i]) * (1.0 - t) + f64::from(b[i]) * t).round() as u8;
    [channel(0), channel(1), channel(2)]
}

fn rect_op(r: Rect) -> String {
    format!("{} {} {} {} re", n(r.x), n(r.y), n(r.w), n(r.h))
}

/// Appends an elliptic arc (y down, angles counter-clockwise in degrees).
fn arc_path(out: &mut String, r: Rect, start: f64, sweep: f64, move_to: bool) {
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    let rx = r.w / 2.0;
    let ry = r.h / 2.0;
    let point = |angle: f64| (cx + rx * angle.cos(), cy - ry * angle.sin());
    let start = start.to_radians();
    let sweep = sweep.to_radians().clamp(-2.0 * PI, 2.0 * PI);
    let segments = ((sweep.abs() / (PI / 2.0)).ceil() as usize).max(1);
    let step = sweep / segments as f64;
    let (x0, y0) = point(start);
    if move_to {
        let _ = writeln!(out, "{} {} m", n(x0), n(y0));
    } else {
        let _ = writeln!(out, "{} {} l", n(x0), n(y0));
    }
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    for index in 0..segments {
        let a0 = start + step * index as f64;
        let a1 = a0 + step;
        let (x1, y1) = point(a0);
        let (x2, y2) = point(a1);
        let c1 = (x1 - k * rx * a0.sin(), y1 - k * ry * a0.cos());
        let c2 = (x2 + k * rx * a1.sin(), y2 + k * ry * a1.cos());
        let _ = writeln!(
            out,
            "{} {} {} {} {} {} c",
            n(c1.0),
            n(c1.1),
            n(c2.0),
            n(c2.1),
            n(x2),
            n(y2)
        );
    }
}

/// Decoded RGB image.
struct RgbImage {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

const MAX_IMAGE_PIXELS: u64 = 40_000_000;

fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn decode_image(bytes: &[u8]) -> Option<RgbImage> {
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
        decoder.read_info().ok()?;
        let info = decoder.info()?;
        if u64::from(info.width) * u64::from(info.height) > MAX_IMAGE_PIXELS {
            return None;
        }
        let pixels = decoder.decode().ok()?;
        let rgb = match info.pixel_format {
            jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|v| [*v, *v, *v]).collect(),
            jpeg_decoder::PixelFormat::L16 => pixels
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|v| [v[0], v[0], v[0]])
                .collect(),
            jpeg_decoder::PixelFormat::RGB24 => pixels,
            jpeg_decoder::PixelFormat::CMYK32 => pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    // Adobe CMYK JPEGs store inverted values.
                    let k = u16::from(p[3]);
                    let conv = |c: u8| (u16::from(c) * k / 255) as u8;
                    [conv(p[0]), conv(p[1]), conv(p[2])]
                })
                .collect(),
        };
        return Some(RgbImage {
            width: u32::from(info.width),
            height: u32::from(info.height),
            rgb,
        });
    }
    if bytes.starts_with(b"\x89PNG") {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let (width, height) = (reader.info().width, reader.info().height);
        if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
            return None;
        }
        let mut buffer = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buffer).ok()?;
        let data = &buffer[..frame.buffer_size()];
        let over_white =
            |v: u8, a: u8| ((u16::from(v) * u16::from(a) + 255 * (255 - u16::from(a))) / 255) as u8;
        let rgb = match frame.color_type {
            png::ColorType::Grayscale => data.iter().flat_map(|v| [*v, *v, *v]).collect(),
            png::ColorType::GrayscaleAlpha => data
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| {
                    let v = over_white(p[0], p[1]);
                    [v, v, v]
                })
                .collect(),
            png::ColorType::Rgb => data.to_vec(),
            png::ColorType::Rgba => data
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| {
                    [
                        over_white(p[0], p[3]),
                        over_white(p[1], p[3]),
                        over_white(p[2], p[3]),
                    ]
                })
                .collect(),
            png::ColorType::Indexed => return None,
        };
        return Some(RgbImage { width, height, rgb });
    }
    if bytes.starts_with(b"BM") {
        return decode_bmp(bytes);
    }
    None
}

/// Uncompressed 24/32-bit and 8-bit palette BMPs.
fn decode_bmp(bytes: &[u8]) -> Option<RgbImage> {
    let u32_at = |offset: usize| {
        bytes
            .get(offset..offset + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let u16_at = |offset: usize| {
        bytes
            .get(offset..offset + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let data_offset = u32_at(10)? as usize;
    let header_size = u32_at(14)? as usize;
    let width = u32_at(18)? as i32;
    let raw_height = u32_at(22)? as i32;
    let bpp = u16_at(28)?;
    let compression = u32_at(30)?;
    if width <= 0 || raw_height == 0 || compression != 0 {
        return None;
    }
    let width = width as u32;
    let height = raw_height.unsigned_abs();
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return None;
    }
    let palette_offset = 14 + header_size;
    let stride = ((u64::from(width) * u64::from(bpp)).div_ceil(32) * 4) as usize;
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for row in 0..height {
        let source_row = if raw_height > 0 {
            height - 1 - row
        } else {
            row
        };
        let start = data_offset + source_row as usize * stride;
        let line = bytes.get(start..start + stride)?;
        for x in 0..width as usize {
            match bpp {
                24 => {
                    let p = line.get(x * 3..x * 3 + 3)?;
                    rgb.extend_from_slice(&[p[2], p[1], p[0]]);
                }
                32 => {
                    let p = line.get(x * 4..x * 4 + 4)?;
                    rgb.extend_from_slice(&[p[2], p[1], p[0]]);
                }
                8 => {
                    let index = *line.get(x)? as usize;
                    let p =
                        bytes.get(palette_offset + index * 4..palette_offset + index * 4 + 3)?;
                    rgb.extend_from_slice(&[p[2], p[1], p[0]]);
                }
                _ => return None,
            }
        }
    }
    Some(RgbImage { width, height, rgb })
}

/// Named images (`/Names /XFAImages`) for `href` references.
pub(crate) type NamedImages = HashMap<String, Vec<u8>>;

pub(crate) struct Painter<'a, 't, 'r> {
    tpl: &'a Template<'t>,
    book: &'a mut FontBook<'r>,
    named_images: &'a NamedImages,
    /// exclGroup member (node address) -> group value.
    excl_values: HashMap<usize, Option<String>>,
    out: String,
    shadings: Vec<(String, PdfObject)>,
    shading_counter: usize,
    image_cache: HashMap<String, Option<std::sync::Arc<(u32, u32, String)>>>,
    pub unsupported: Vec<String>,
    locales: LocaleSet,
    /// Per-page values of master-page fields set by `layout:ready`
    /// scripts: (form node address, page index) -> value.
    page_values: HashMap<(usize, usize), String>,
    page_index: usize,
}

/// What a page paint includes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaintScope {
    /// Everything: draws, container borders and fields (full XFA pages).
    All,
    /// Fields and exclusion groups only (static forms whose PDF page
    /// content already carries the boilerplate).
    FieldsOnly,
}

pub(crate) struct PaintedPage {
    pub content: String,
    pub shadings: Vec<(String, PdfObject)>,
}

fn collect_excl<'t>(node: &FormNode<'t>, out: &mut HashMap<usize, Option<String>>) {
    if node.kind == NodeKind::ExclGroup {
        for child in &node.children {
            out.insert(child as *const FormNode<'t> as usize, node.value.clone());
        }
    }
    for child in &node.children {
        collect_excl(child, out);
    }
}

impl<'a, 't, 'r> Painter<'a, 't, 'r> {
    pub(crate) fn new(
        tpl: &'a Template<'t>,
        book: &'a mut FontBook<'r>,
        named_images: &'a NamedImages,
        roots: &[&FormNode<'t>],
    ) -> Self {
        let mut excl_values = HashMap::new();
        for root in roots {
            collect_excl(root, &mut excl_values);
        }
        Self {
            tpl,
            book,
            named_images,
            excl_values,
            out: String::new(),
            shadings: Vec::new(),
            shading_counter: 0,
            image_cache: HashMap::new(),
            unsupported: Vec::new(),
            locales: LocaleSet::default(),
            page_values: HashMap::new(),
            page_index: 0,
        }
    }

    /// The document's locales (picture clauses and default formats).
    pub(crate) fn set_locales(&mut self, locales: LocaleSet) {
        self.locales = locales;
    }

    /// Per-page master-page values from `layout:ready` scripts.
    pub(crate) fn set_page_values(&mut self, values: HashMap<(usize, usize), String>) {
        self.page_values = values;
    }

    /// The node's value, with a per-page script value on master pages.
    fn node_value(&self, node: &FormNode<'t>) -> Option<String> {
        if !self.page_values.is_empty() {
            let key = (node as *const FormNode<'t> as usize, self.page_index);
            if let Some(value) = self.page_values.get(&key) {
                return Some(value.clone());
            }
        }
        node.value.clone()
    }

    /// Paints page `index` (the index selects per-page script values).
    pub(crate) fn paint_page_at(
        &mut self,
        page: &LayoutPage<'_, 't>,
        scope: PaintScope,
        index: usize,
    ) -> PaintedPage {
        self.page_index = index;
        self.out.clear();
        self.shadings.clear();
        for item in &page.items {
            self.paint_item(item, scope);
        }
        PaintedPage {
            content: std::mem::take(&mut self.out),
            shadings: std::mem::take(&mut self.shadings),
        }
    }

    fn paint_item(&mut self, item: &Placed<'_, 't>, scope: PaintScope) {
        let node = item.node;
        let tpl = self.tpl;
        if node.presence_in(tpl) != Presence::Visible {
            return;
        }
        let wanted = match scope {
            PaintScope::All => true,
            PaintScope::FieldsOnly => matches!(node.kind, NodeKind::Field | NodeKind::ExclGroup),
        };
        if !wanted || item.w <= 0.0 && item.h <= 0.0 {
            return;
        }
        // Local frame: the unrotated widget rectangle at the origin.
        let (w, h) = if item.rotate % 180 == 90 {
            (item.h, item.w)
        } else {
            (item.w, item.h)
        };
        let matrix = match item.rotate {
            90 => [0.0, -1.0, 1.0, 0.0, item.x, item.y + item.h],
            180 => [-1.0, 0.0, 0.0, -1.0, item.x + item.w, item.y + item.h],
            270 => [0.0, 1.0, -1.0, 0.0, item.x + item.w, item.y],
            _ => [1.0, 0.0, 0.0, 1.0, item.x, item.y],
        };
        let _ = writeln!(
            self.out,
            "q {} {} {} {} {} {} cm",
            n(matrix[0]),
            n(matrix[1]),
            n(matrix[2]),
            n(matrix[3]),
            n(matrix[4]),
            n(matrix[5])
        );
        let rect = Rect::new(0.0, 0.0, w, h);
        match node.kind {
            NodeKind::Subform | NodeKind::Area | NodeKind::ExclGroup => {
                self.paint_widget_border(node, rect);
            }
            NodeKind::Draw => self.paint_draw(node, rect),
            NodeKind::Field => self.paint_field(node, rect),
        }
        self.out.push_str("Q\n");
    }

    /// `CXFA_FFWidget::RenderWidget`: the node's `<border>` inside its
    /// margin, with script fill/border colour overrides.
    fn paint_widget_border(&mut self, node: &FormNode<'t>, rect: Rect) {
        let tpl = self.tpl;
        let Some(mut border) = tpl
            .child(node.tmpl, "border")
            .and_then(|border| BoxSpec::from_node(tpl, border))
        else {
            if let Some(color) = node.fill_color {
                // `fillColor` on a node without a border: a plain fill.
                let _ = writeln!(
                    self.out,
                    "{} rg {} f",
                    rgb(color),
                    rect_op(rect.normalized())
                );
            }
            return;
        };
        if let Some(color) = node.fill_color {
            border.fill = Some(FillSpec {
                color,
                kind: FillKind::Solid,
            });
        }
        if let Some(color) = node.border_color {
            for stroke in &mut border.strokes {
                stroke.color = color;
            }
        }
        let rect = rect.deflate(border.margin).normalized();
        self.draw_box(&border, rect, false);
    }

    fn paint_draw(&mut self, node: &FormNode<'t>, rect: Rect) {
        let tpl = self.tpl;
        let margin = node_margin(tpl, node.tmpl);
        let content = tpl
            .child(node.tmpl, "value")
            .and_then(|value| value.children.first());
        match content.map(|content| content.local_name.as_str()) {
            Some("line") => {
                let line = content.unwrap_or(node.tmpl);
                self.paint_line(line, rect.deflate(margin));
            }
            Some("rectangle" | "arc") => {
                let shape = content.unwrap_or(node.tmpl);
                if let Some(spec) = BoxSpec::from_node(tpl, shape) {
                    let inner = rect.deflate(margin);
                    self.draw_box(&spec, inner, false);
                }
            }
            Some("image") => {
                self.paint_widget_border(node, rect);
                if let Some(image) = &node.image {
                    let aspect = content.and_then(|c| tpl.attr(c, "aspect")).unwrap_or("fit");
                    let para = node_para(tpl, node.tmpl);
                    self.paint_image(image, rect.deflate(margin), aspect, &para);
                }
            }
            _ => {
                self.paint_widget_border(node, rect);
                let font = node_font_for(tpl, node);
                let para = node_para(tpl, node.tmpl);
                let value = self.node_value(node);
                let paragraphs = match (node.rich, &value) {
                    (Some(rich), _) => rich_paragraphs(rich, &font, &para),
                    (None, Some(text)) if !text.is_empty() => plain_paragraphs(text, &font, &para),
                    _ => return,
                };
                let inner = rect.deflate(margin);
                let block = layout_paragraphs(self.book, &paragraphs, Some(inner.w.max(1.0)));
                self.emit_text(&block, inner, para.v_align, Some(rect));
            }
        }
    }

    fn emit_text(&mut self, block: &TextBlock, rect: Rect, v_align: VAlign, clip: Option<Rect>) {
        if block.pieces.is_empty() {
            return;
        }
        let dy = match v_align {
            VAlign::Top => 0.0,
            VAlign::Middle => (rect.h - block.height) / 2.0,
            VAlign::Bottom => rect.h - block.height,
        };
        let dy = if v_align == VAlign::Top {
            dy
        } else {
            dy.max(0.0)
        };
        if let Some(clip) = clip {
            let _ = writeln!(self.out, "q {} W n", rect_op(clip.normalized()));
        }
        emit_block(self.book, &mut self.out, block, rect.x, rect.y + dy);
        if clip.is_some() {
            self.out.push_str("Q\n");
        }
    }

    /// `CXFA_FFLine::RenderWidget`.
    fn paint_line(&mut self, line: &'t XmlNode, mut rect: Rect) {
        let tpl = self.tpl;
        let edge = tpl
            .child(line, "edge")
            .map(|edge| Stroke::from_node(tpl, edge));
        if edge.is_some_and(|edge| !edge.visible) {
            return;
        }
        let (color, width, stroke, cap) = match edge {
            Some(edge) => (edge.color, edge.thickness, edge.stroke, edge.cap),
            None => (BLACK, 1.0, StrokeType::Solid, super::style::CapType::Square),
        };
        let half = width / 2.0;
        let hand = match tpl.attr(line, "hand") {
            Some("left") => Hand::Left,
            Some("right") => Hand::Right,
            _ => Hand::Even,
        };
        if rect.h < 1.0 {
            match hand {
                Hand::Left => rect.y -= half,
                Hand::Right => rect.y += half,
                Hand::Even => {}
            }
        } else if rect.w < 1.0 {
            if hand != Hand::Even {
                rect.x += half;
            }
        } else {
            match hand {
                Hand::Left => {
                    rect = rect.deflate(Insets {
                        left: -half,
                        top: -half,
                        right: -half,
                        bottom: -half,
                    })
                }
                Hand::Right => {
                    rect = rect.deflate(Insets {
                        left: half,
                        top: half,
                        right: half,
                        bottom: half,
                    })
                }
                Hand::Even => {}
            }
        }
        let slope_up = tpl.attr(line, "slope") == Some("/");
        let (x0, y0, x1, y1) = if slope_up && rect.right() > 0.0 && rect.bottom() > 0.0 {
            (rect.right(), rect.y, rect.x, rect.bottom())
        } else {
            (rect.x, rect.y, rect.right(), rect.bottom())
        };
        if width < 0.001 {
            return;
        }
        let cap_op = match cap {
            super::style::CapType::Butt => 0,
            super::style::CapType::Round => 1,
            super::style::CapType::Square => 2,
        };
        let _ = writeln!(
            self.out,
            "q {} RG {} w {} J {} {} {} m {} {} l S Q",
            rgb(color),
            n(width),
            cap_op,
            dash_op(stroke.dash(), width),
            n(x0),
            n(y0),
            n(x1),
            n(y1)
        );
    }

    // ------------------------------------------------------------------
    // Boxes
    // ------------------------------------------------------------------

    fn draw_box(&mut self, spec: &BoxSpec, rect: Rect, force_round: bool) {
        self.draw_fill(spec, rect, force_round);
        if spec.kind == BoxKind::Arc || force_round {
            self.stroke_arc(spec, rect, force_round);
        } else {
            self.stroke_rectangle(spec, rect);
        }
    }

    fn ellipse_rect(&self, spec: &BoxSpec, rect: Rect, force_round: bool) -> Rect {
        let mut a = rect.w / 2.0;
        let mut b = rect.h / 2.0;
        if spec.circular || force_round {
            a = a.min(b);
            b = a;
        }
        let cx = rect.x + rect.w / 2.0;
        let cy = rect.y + rect.h / 2.0;
        Rect::new(cx - a, cy - b, 2.0 * a, 2.0 * b)
    }

    fn arc_or_ellipse(&self, out: &mut String, spec: &BoxSpec, rect: Rect, force_round: bool) {
        let r = self.ellipse_rect(spec, rect, force_round);
        if spec.start_angle.is_none() && spec.sweep_angle.is_none() || force_round {
            ellipse_path(out, r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, r.h / 2.0);
        } else {
            arc_path(
                out,
                r,
                spec.start_angle.unwrap_or(0.0),
                spec.sweep_angle.unwrap_or(360.0),
                true,
            );
        }
    }

    fn hand_adjusted(spec: &BoxSpec, rect: Rect, thickness: f64) -> Rect {
        let half = thickness.max(0.0) / 2.0;
        match spec.hand {
            Hand::Left => rect.deflate(Insets {
                left: -half,
                top: -half,
                right: -half,
                bottom: -half,
            }),
            Hand::Right => rect.deflate(Insets {
                left: half,
                top: half,
                right: half,
                bottom: half,
            }),
            Hand::Even => rect,
        }
    }

    fn plain_rectangle(spec: &BoxSpec) -> bool {
        let s = &spec.strokes;
        let all_same = (1..8).all(|i| s[i - 1].same_style(&s[i], false, false));
        let corners_same = [2, 4, 6]
            .iter()
            .all(|&i| s[0].same_style(&s[i], true, true));
        all_same && corners_same && !s[0].inverted && !s[0].round
    }

    /// Fill (or closed outline) path of a rectangle with its corner radii.
    fn rectangle_path(&self, spec: &BoxSpec, rect: Rect, out: &mut String) {
        if Self::plain_rectangle(spec) {
            let _ = writeln!(out, "{}", rect_op(rect));
            return;
        }
        let radius = |i: usize| {
            spec.strokes[i * 2]
                .radius
                .max(0.0)
                .min(rect.w / 2.0)
                .min(rect.h / 2.0)
        };
        let round = |i: usize| spec.strokes[i * 2].round;
        let (l, t, r, b) = (rect.x, rect.y, rect.right(), rect.bottom());
        let r0 = radius(0);
        let _ = writeln!(out, "{} {} m", n(l), n(t + r0));
        // Top-left corner.
        self.corner_to(out, (l, t + r0), (l + r0, t), (l, t), round(0));
        let r1 = radius(1);
        let _ = writeln!(out, "{} {} l", n(r - r1), n(t));
        self.corner_to(out, (r - r1, t), (r, t + r1), (r, t), round(1));
        let r2 = radius(2);
        let _ = writeln!(out, "{} {} l", n(r), n(b - r2));
        self.corner_to(out, (r, b - r2), (r - r2, b), (r, b), round(2));
        let r3 = radius(3);
        let _ = writeln!(out, "{} {} l", n(l + r3), n(b));
        self.corner_to(out, (l + r3, b), (l, b - r3), (l, b), round(3));
        out.push_str("h\n");
    }

    /// From `from` (already the current point) to `to` around `corner`:
    /// a quarter ellipse for round joins, a chamfer otherwise.
    fn corner_to(
        &self,
        out: &mut String,
        from: (f64, f64),
        to: (f64, f64),
        corner: (f64, f64),
        round: bool,
    ) {
        if (from.0 - to.0).abs() < 1e-9 && (from.1 - to.1).abs() < 1e-9 {
            return;
        }
        if round {
            let k = 0.552_284_749_830_793_6;
            let c1 = (
                from.0 + (corner.0 - from.0) * k,
                from.1 + (corner.1 - from.1) * k,
            );
            let c2 = (to.0 + (corner.0 - to.0) * k, to.1 + (corner.1 - to.1) * k);
            let _ = writeln!(
                out,
                "{} {} {} {} {} {} c",
                n(c1.0),
                n(c1.1),
                n(c2.0),
                n(c2.1),
                n(to.0),
                n(to.1)
            );
        } else {
            let _ = writeln!(out, "{} {} l", n(to.0), n(to.1));
        }
    }

    fn draw_fill(&mut self, spec: &BoxSpec, rect: Rect, force_round: bool) {
        let Some(fill) = spec.fill else {
            return;
        };
        let mut path = String::new();
        if spec.kind == BoxKind::Arc || force_round {
            let rect = Self::hand_adjusted(spec, rect, spec.edge(0).thickness);
            self.arc_or_ellipse(&mut path, spec, rect, force_round);
            path.push_str("h\n");
        } else {
            self.rectangle_path(spec, rect, &mut path);
        }
        self.fill_path(&path, rect, &fill);
    }

    fn fill_path(&mut self, path: &str, rect: Rect, fill: &FillSpec) {
        match fill.kind {
            FillKind::Solid => {
                let _ = write!(self.out, "q {} rg\n{}f Q\n", rgb(fill.color), path);
            }
            FillKind::Stipple { rate, color } => {
                let blended = mix(fill.color, color, rate / 100.0);
                let _ = write!(self.out, "q {} rg\n{}f Q\n", rgb(blended), path);
            }
            FillKind::Linear { dir, end } => {
                let (x0, y0, x1, y1) = match dir {
                    LinearDir::Right => (rect.x, rect.y, rect.right(), rect.y),
                    LinearDir::Bottom => (rect.x, rect.y, rect.x, rect.bottom()),
                    LinearDir::Left => (rect.right(), rect.y, rect.x, rect.y),
                    LinearDir::Top => (rect.x, rect.bottom(), rect.x, rect.y),
                };
                let name = self.add_shading(2, vec![x0, y0, x1, y1], fill.color, end);
                let _ = write!(self.out, "q\n{}W n /{} sh Q\n", path, name);
            }
            FillKind::Radial { to_edge, end } => {
                let (start, finish) = if to_edge {
                    (fill.color, end)
                } else {
                    (end, fill.color)
                };
                let cx = rect.x + rect.w / 2.0;
                let cy = rect.y + rect.h / 2.0;
                let radius = rect.w.hypot(rect.h) / 2.0;
                let name = self.add_shading(3, vec![cx, cy, 0.0, cx, cy, radius], start, finish);
                let _ = write!(self.out, "q\n{}W n /{} sh Q\n", path, name);
            }
            FillKind::Pattern { kind, color } => {
                let _ = write!(self.out, "q {} rg\n{}f Q\n", rgb(fill.color), path);
                let _ = write!(self.out, "q\n{}W n {} RG 0.5 w\n", path, rgb(color));
                let step = 6.0;
                let span = rect.w + rect.h;
                let mut lines = String::new();
                let horizontal = matches!(kind, PatternKind::Horizontal | PatternKind::CrossHatch);
                let vertical = matches!(kind, PatternKind::Vertical | PatternKind::CrossHatch);
                let diag_right = matches!(
                    kind,
                    PatternKind::DiagonalRight | PatternKind::CrossDiagonal
                );
                let diag_left =
                    matches!(kind, PatternKind::DiagonalLeft | PatternKind::CrossDiagonal);
                let mut offset = 0.0;
                while offset <= span && lines.len() < 400_000 {
                    if horizontal && offset <= rect.h {
                        let y = rect.y + offset;
                        let _ = writeln!(
                            lines,
                            "{} {} m {} {} l",
                            n(rect.x),
                            n(y),
                            n(rect.right()),
                            n(y)
                        );
                    }
                    if vertical && offset <= rect.w {
                        let x = rect.x + offset;
                        let _ = writeln!(
                            lines,
                            "{} {} m {} {} l",
                            n(x),
                            n(rect.y),
                            n(x),
                            n(rect.bottom())
                        );
                    }
                    if diag_right {
                        let x = rect.x + offset;
                        let _ = writeln!(
                            lines,
                            "{} {} m {} {} l",
                            n(x),
                            n(rect.y),
                            n(x - rect.h),
                            n(rect.bottom())
                        );
                    }
                    if diag_left {
                        let x = rect.x + offset - rect.h;
                        let _ = writeln!(
                            lines,
                            "{} {} m {} {} l",
                            n(x),
                            n(rect.y),
                            n(x + rect.h),
                            n(rect.bottom())
                        );
                    }
                    offset += step;
                }
                self.out.push_str(&lines);
                self.out.push_str("S Q\n");
            }
        }
    }

    fn add_shading(&mut self, shading_type: i64, coords: Vec<f64>, c0: Rgb, c1: Rgb) -> String {
        self.shading_counter += 1;
        let name = format!("XSh{}", self.shading_counter);
        let color = |c: Rgb| {
            PdfObject::Array(
                c.iter()
                    .map(|v| PdfObject::Real(f64::from(*v) / 255.0))
                    .collect(),
            )
        };
        let mut function = PdfDictionary::empty();
        function.insert("FunctionType", PdfObject::Integer(2));
        function.insert(
            "Domain",
            PdfObject::Array(vec![PdfObject::Integer(0), PdfObject::Integer(1)]),
        );
        function.insert("C0", color(c0));
        function.insert("C1", color(c1));
        function.insert("N", PdfObject::Integer(1));
        let mut shading = PdfDictionary::empty();
        shading.insert("ShadingType", PdfObject::Integer(shading_type));
        shading.insert("ColorSpace", PdfObject::Name("DeviceRGB".into()));
        shading.insert(
            "Coords",
            PdfObject::Array(coords.into_iter().map(PdfObject::Real).collect()),
        );
        shading.insert("Function", PdfObject::Dictionary(function));
        shading.insert(
            "Extend",
            PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
        );
        self.shadings
            .push((name.clone(), PdfObject::Dictionary(shading)));
        name
    }

    fn stroke_arc(&mut self, spec: &BoxSpec, rect: Rect, force_round: bool) {
        let edge = *spec.edge(0);
        if !edge.visible {
            return;
        }
        let lowered = force_round
            && spec
                .style_3d()
                .is_some_and(|(_, visible, thickness)| visible && thickness >= 0.001);
        let half = (edge.thickness / 2.0).max(0.0);
        let rect = Self::hand_adjusted(spec, rect, edge.thickness);
        if !lowered {
            if half < 0.001 {
                return;
            }
            let mut path = String::new();
            self.arc_or_ellipse(&mut path, spec, rect, force_round);
            self.stroke_path(&path, &edge);
            return;
        }
        // PDFium's lowered round box: grey/white outer, dark/light inner arcs.
        let r = self.ellipse_rect(spec, rect, true);
        let inner = r.deflate(Insets {
            left: half,
            top: half,
            right: half,
            bottom: half,
        });
        for (rect, first, second) in [
            (r, [128, 128, 128], [255, 255, 255]),
            (inner, [64, 64, 64], [192, 192, 192]),
        ] {
            let mut path = String::new();
            arc_path(&mut path, rect, 135.0, 180.0, true);
            let _ = write!(self.out, "q {} RG {} w\n{}S Q\n", rgb(first), n(half), path);
            let mut path = String::new();
            arc_path(&mut path, rect, -45.0, 180.0, true);
            let _ = write!(
                self.out,
                "q {} RG {} w\n{}S Q\n",
                rgb(second),
                n(half),
                path
            );
        }
    }

    fn stroke_path(&mut self, path: &str, stroke: &Stroke) {
        if !stroke.visible || stroke.thickness < 0.001 {
            return;
        }
        let _ = write!(
            self.out,
            "q {} RG {} w 0 J 0 j {}\n{}S Q\n",
            rgb(stroke.color),
            n(stroke.thickness),
            dash_op(stroke.stroke.dash(), stroke.thickness),
            path
        );
    }

    /// `CXFA_Rectangle::Draw`.
    fn stroke_rectangle(&mut self, spec: &BoxSpec, rect: Rect) {
        if !(0..4).any(|i| spec.edge(i).visible) {
            return;
        }
        let mut r = rect;
        for (side, stroke) in [
            (0usize, spec.edge(0)),
            (1, spec.edge(1)),
            (2, spec.edge(2)),
            (3, spec.edge(3)),
        ] {
            let half = stroke.thickness.max(0.0) / 2.0;
            match (side, spec.hand) {
                (0, Hand::Left) => {
                    r.y -= half;
                    r.h += half;
                }
                (0, Hand::Right) => {
                    r.y += half;
                    r.h -= half;
                }
                (1 | 2, Hand::Left) => {
                    if side == 1 {
                        r.w += half;
                    } else {
                        r.h += half;
                    }
                }
                (1 | 2, Hand::Right) => {
                    if side == 1 {
                        r.w -= half;
                    } else {
                        r.h -= half;
                    }
                }
                (3, Hand::Left) => {
                    r.x -= half;
                    r.w += half;
                }
                (3, Hand::Right) => {
                    r.x += half;
                    r.w -= half;
                }
                _ => {}
            }
        }
        if let Some((kind, visible, thickness)) = spec.style_3d() {
            if !visible || thickness < 0.001 {
                return;
            }
            self.stroke_3d(kind, r, thickness);
            return;
        }
        if Self::plain_rectangle(spec) {
            let mut path = String::new();
            self.rectangle_path(spec, r, &mut path);
            let edge = *spec.edge(0);
            self.stroke_path(&path, &edge);
            return;
        }
        // Edges with their own styles, extended over square corners.
        let (l, t, rr, b) = (r.x, r.y, r.right(), r.bottom());
        let radius = |i: usize| {
            spec.strokes[i * 2]
                .radius
                .max(0.0)
                .min(r.w / 2.0)
                .min(r.h / 2.0)
        };
        let half = |i: usize| {
            let edge = spec.edge(i);
            if edge.visible {
                edge.thickness / 2.0
            } else {
                0.0
            }
        };
        let segments = [
            (
                (
                    l + radius(0) - if radius(0) > 0.0 { 0.0 } else { half(3) },
                    t,
                ),
                (
                    rr - radius(1) + if radius(1) > 0.0 { 0.0 } else { half(1) },
                    t,
                ),
            ),
            (
                (
                    rr,
                    t + radius(1) - if radius(1) > 0.0 { 0.0 } else { half(0) },
                ),
                (
                    rr,
                    b - radius(2) + if radius(2) > 0.0 { 0.0 } else { half(2) },
                ),
            ),
            (
                (
                    rr - radius(2) + if radius(2) > 0.0 { 0.0 } else { half(1) },
                    b,
                ),
                (
                    l + radius(3) - if radius(3) > 0.0 { 0.0 } else { half(3) },
                    b,
                ),
            ),
            (
                (
                    l,
                    b - radius(3) + if radius(3) > 0.0 { 0.0 } else { half(2) },
                ),
                (
                    l,
                    t + radius(0) - if radius(0) > 0.0 { 0.0 } else { half(0) },
                ),
            ),
        ];
        for (index, (from, to)) in segments.iter().enumerate() {
            let edge = *spec.edge(index);
            let path = format!("{} {} m {} {} l\n", n(from.0), n(from.1), n(to.0), n(to.1));
            self.stroke_path(&path, &edge);
        }
        // Rounded corners with the corner stroke.
        let corners = [
            ((l, t + radius(0)), (l + radius(0), t), (l, t)),
            ((rr - radius(1), t), (rr, t + radius(1)), (rr, t)),
            ((rr, b - radius(2)), (rr - radius(2), b), (rr, b)),
            ((l + radius(3), b), (l, b - radius(3)), (l, b)),
        ];
        for (index, (from, to, corner)) in corners.iter().enumerate() {
            if radius(index) <= 0.0 {
                continue;
            }
            let stroke = spec.strokes[index * 2];
            let mut path = format!("{} {} m\n", n(from.0), n(from.1));
            self.corner_to(&mut path, *from, *to, *corner, stroke.round);
            let mut stroke = stroke;
            stroke.thickness = stroke.thickness.min(2.0 * stroke.radius);
            self.stroke_path(&path, &stroke);
        }
    }

    fn stroke_rect_bands(&mut self, r: Rect, width: f64, top_left: Rgb, bottom_right: Rgb) {
        let (l, t, rr, b) = (r.x, r.y, r.right(), r.bottom());
        let _ = writeln!(
            self.out,
            "q {} rg {} {} m {} {} l {} {} l {} {} l {} {} l {} {} l h f Q",
            rgb(top_left),
            n(l),
            n(b),
            n(l),
            n(t),
            n(rr),
            n(t),
            n(rr - width),
            n(t + width),
            n(l + width),
            n(t + width),
            n(l + width),
            n(b - width)
        );
        let _ = writeln!(
            self.out,
            "q {} rg {} {} m {} {} l {} {} l {} {} l {} {} l {} {} l h f Q",
            rgb(bottom_right),
            n(rr),
            n(t),
            n(rr),
            n(b),
            n(l),
            n(b),
            n(l + width),
            n(b - width),
            n(rr - width),
            n(b - width),
            n(rr - width),
            n(t + width)
        );
    }

    fn stroke_3d(&mut self, kind: StrokeType, r: Rect, thickness: f64) {
        let half = thickness / 2.0;
        let inner = r.deflate(Insets {
            left: half,
            top: half,
            right: half,
            bottom: half,
        });
        match kind {
            StrokeType::Lowered | StrokeType::Raised => {
                let _ = writeln!(
                    self.out,
                    "q 0 0 0 rg {} {} f* Q",
                    rect_op(r),
                    rect_op(inner)
                );
                if kind == StrokeType::Lowered {
                    self.stroke_rect_bands(inner, half, [128, 128, 128], [192, 192, 192]);
                } else {
                    self.stroke_rect_bands(inner, half, [255, 255, 255], [128, 128, 128]);
                }
            }
            StrokeType::Etched => {
                self.stroke_rect_bands(r, thickness, [128, 128, 128], [255, 255, 255]);
                self.stroke_rect_bands(inner, half, [255, 255, 255], [128, 128, 128]);
            }
            _ => {
                self.stroke_rect_bands(r, thickness, [128, 128, 128], [0, 0, 0]);
                self.stroke_rect_bands(inner, half, [0, 0, 0], [128, 128, 128]);
            }
        }
    }

    // ------------------------------------------------------------------
    // Fields
    // ------------------------------------------------------------------

    /// `CXFA_Node::GetUIMargin`.
    fn ui_margin(&self, widget: Option<&'t XmlNode>) -> Insets {
        let tpl = self.tpl;
        let Some(widget) = widget else {
            return Insets::default();
        };
        let Some(margin) = tpl.child(widget, "margin") else {
            return Insets::default();
        };
        let border_node = tpl.child(widget, "border");
        if border_node.is_some_and(|border| presence(tpl, border) != Presence::Visible) {
            return Insets::default();
        }
        let border = border_node.and_then(|border| BoxSpec::from_node(tpl, border));
        let [left, top, right, bottom] = Insets::explicit(tpl, margin);
        let three_d = border
            .as_ref()
            .and_then(BoxSpec::style_3d)
            .is_some_and(|(_, visible, _)| visible);
        let from_edge = |side: usize| {
            border
                .as_ref()
                .map_or(0.0, |border| border.edge_inset(side, three_d))
        };
        Insets {
            left: left.unwrap_or_else(|| from_edge(3)),
            top: top.unwrap_or_else(|| from_edge(0)),
            right: right.unwrap_or_else(|| from_edge(1)),
            bottom: bottom.unwrap_or_else(|| from_edge(2)),
        }
    }

    fn paint_field(&mut self, node: &FormNode<'t>, rect: Rect) {
        let tpl = self.tpl;
        let (ui, widget) = field_ui(tpl, node.tmpl);
        self.paint_widget_border(node, rect);
        if let Ui::Check {
            round,
            size,
            ref mark,
        } = ui
        {
            self.paint_check(node, rect, widget, round, size, mark.as_deref());
            return;
        }
        let margin = node_margin(tpl, node.tmpl);
        if ui == Ui::Button {
            // CXFA_FFPushButton: the caption fills the widget inside margins.
            if let Some(caption) = caption_spec_for(tpl, node).filter(|c| c.visible) {
                let caption_rect = rect.deflate(margin).deflate(caption.margin).normalized();
                let paragraphs = caption.paragraphs();
                if !paragraphs.is_empty() {
                    let block =
                        layout_paragraphs(self.book, &paragraphs, Some(caption_rect.w.max(1.0)));
                    let clip = caption_rect.intersect(rect);
                    self.emit_text(&block, caption_rect, caption.para.v_align, Some(clip));
                }
            }
            return;
        }
        let widget_rect = rect.deflate(margin);
        let ui_margin = self.ui_margin(widget);
        let caption = caption_spec_for(tpl, node).filter(|caption| !caption.hidden);
        let mut ui_rect = widget_rect;
        let mut caption_layout = None;
        if let Some(caption) = &caption {
            let paragraphs = caption.paragraphs();
            let vertical = matches!(
                caption.placement,
                CaptionPlacement::Top | CaptionPlacement::Bottom
            );
            let mut reserve = caption.reserve.unwrap_or(-1.0);
            if reserve <= 0.0 && !paragraphs.is_empty() {
                let natural = layout_paragraphs(self.book, &paragraphs, None);
                reserve = if vertical {
                    natural.height + caption.margin.top + caption.margin.bottom
                } else {
                    natural.width + caption.margin.left + caption.margin.right
                };
            }
            let reserve = reserve.max(0.0).min(if vertical {
                widget_rect.h
            } else {
                widget_rect.w
            });
            let mut caption_rect = widget_rect;
            match caption.placement {
                CaptionPlacement::Left => {
                    caption_rect.w = reserve;
                    caption_rect.y += ui_margin.top;
                    caption_rect.h -= ui_margin.top;
                    ui_rect.x += reserve;
                    ui_rect.w -= reserve;
                }
                CaptionPlacement::Right => {
                    caption_rect.x = caption_rect.right() - reserve;
                    caption_rect.w = reserve;
                    caption_rect.y += ui_margin.top;
                    caption_rect.h -= ui_margin.top;
                    ui_rect.w -= reserve;
                }
                CaptionPlacement::Top => {
                    caption_rect.h = reserve;
                    caption_rect.x += ui_margin.left;
                    ui_rect.y += reserve;
                    ui_rect.h -= reserve;
                }
                CaptionPlacement::Bottom => {
                    caption_rect.y = caption_rect.bottom() - reserve;
                    caption_rect.h = reserve;
                    caption_rect.x += ui_margin.left;
                    ui_rect.h -= reserve;
                }
                CaptionPlacement::Inline => {}
            }
            if caption.placement != CaptionPlacement::Inline {
                let caption_rect = caption_rect.deflate(caption.margin);
                caption_layout = Some((caption_rect, paragraphs, caption.para, caption.visible));
            }
        }
        let ui_border = widget.and_then(|widget| {
            tpl.child(widget, "border")
                .and_then(|border| BoxSpec::from_node(tpl, border))
        });
        if let Some(border) = &ui_border {
            ui_rect = ui_rect.deflate(border.margin);
        }
        let ui_rect = ui_rect.normalized();
        if let Some(border) = &ui_border {
            self.draw_box(border, ui_rect, false);
        }
        if let Some((caption_rect, paragraphs, para, visible)) = caption_layout {
            if visible && !paragraphs.is_empty() {
                let block =
                    layout_paragraphs(self.book, &paragraphs, Some(caption_rect.w.max(1.0)));
                let clip = caption_rect.normalized().intersect(rect);
                self.emit_text(&block, caption_rect, para.v_align, Some(clip));
            }
        }
        match ui {
            Ui::Button | Ui::Signature => {}
            Ui::Barcode => {
                self.paint_barcode(node, widget, ui_rect.deflate(ui_margin).normalized())
            }
            Ui::Image => {
                if let Some(image) = &node.image {
                    let aspect = tpl
                        .child(node.tmpl, "value")
                        .and_then(|value| value.child("image"))
                        .and_then(|image| tpl.attr(image, "aspect"))
                        .unwrap_or("fit");
                    let para = node_para(tpl, node.tmpl);
                    self.paint_image(image, ui_rect.deflate(ui_margin), aspect, &para);
                }
            }
            Ui::Choice { list: true } => {
                self.paint_list_box(node, ui_rect.deflate(ui_margin), ui_rect)
            }
            _ => self.paint_edit(node, &ui, ui_rect, ui_margin),
        }
    }

    /// EAN-8, EAN-13 and UPC-A as PDFium draws them: bars over the full
    /// height in the quiet-zone-padded symbol, the digits in white boxes at
    /// the bottom between the guard bars (which keep their full length),
    /// the leading/trailing digits beside the bars.
    fn paint_ean(
        &mut self,
        node: &FormNode<'t>,
        spec: &super::barcode::BarcodeSpec,
        layout: &super::barcode::EanText,
        runs: &[(bool, f64)],
        text: &str,
        rect: Rect,
    ) {
        let scale = rect.w / layout.total_modules;
        let mut x = rect.x + layout.left_pad * scale;
        self.out.push_str("0 0 0 rg\n");
        for (dark, width) in runs {
            let w = width * scale;
            if *dark {
                let _ = writeln!(self.out, "{}", rect_op(Rect::new(x, rect.y, w, rect.h)));
            }
            x += w;
        }
        self.out.push_str("f\n");
        if spec.text == super::barcode::TextLocation::None || text.is_empty() {
            return;
        }
        let font = node_font_for(self.tpl, node);
        let text_h = (font.size.abs().floor() + 1.0).min(rect.h / 2.0);
        let text_y = rect.y + rect.h - text_h;
        let mut para = ParaSpec::default();
        para.h_align = HAlign::Center;
        for (range, module_x, module_w) in &layout.groups {
            let Some(digits) = text.get(range.clone()) else {
                continue;
            };
            let group_x = rect.x + module_x * scale;
            let white = Rect::new(group_x, text_y, (module_w - 0.5).max(0.0) * scale, text_h);
            let _ = writeln!(self.out, "1 1 1 rg {} f", rect_op(white));
            let paragraphs = plain_paragraphs(digits, &font, &para);
            let width = (module_w * scale).max(1.0);
            let block = layout_paragraphs(self.book, &paragraphs, Some(width));
            let text_rect = Rect::new(group_x, text_y, width, text_h);
            self.emit_text(&block, text_rect, VAlign::Top, Some(rect));
        }
    }

    /// `CXFA_FFBarcode::RenderWidget`: the symbol for the field's value on a
    /// white background, linear symbols with their human-readable text.
    fn paint_barcode(&mut self, node: &FormNode<'t>, widget: Option<&'t XmlNode>, rect: Rect) {
        use super::barcode::{self, Symbol, TextLocation};
        let tpl = self.tpl;
        let Some(widget) = widget else {
            return;
        };
        let spec = match barcode::parse_spec(tpl, widget) {
            Ok(spec) => spec,
            Err(code) => {
                self.unsupported.push(code);
                return;
            }
        };
        let Some(value) = self.node_value(node).filter(|v| !v.is_empty()) else {
            return;
        };
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let Some(symbol) = barcode::encode_for(&spec, &value, rect.w, rect.h) else {
            self.unsupported
                .push("xfa.render.barcode.unencodable".to_string());
            return;
        };
        let _ = writeln!(self.out, "q 1 1 1 rg {} f", rect_op(rect));
        match symbol {
            Symbol::Linear { runs, text } if barcode::ean_text_layout(spec.kind).is_some() => {
                let layout = barcode::ean_text_layout(spec.kind).expect("EAN layout");
                self.paint_ean(node, &spec, &layout, &runs, &text, rect);
            }
            Symbol::Linear { runs, text } => {
                let font = node_font_for(tpl, node);
                let show_text = spec.text != TextLocation::None && !text.is_empty();
                let text_h = if show_text {
                    (font.size.abs().floor() + 1.0).min(rect.h / 2.0)
                } else {
                    0.0
                };
                let text_above =
                    matches!(spec.text, TextLocation::Above | TextLocation::AboveEmbedded);
                let bar_y = if text_above { rect.y + text_h } else { rect.y };
                let bar_h = (rect.h - text_h).max(0.0);
                let total: f64 = runs.iter().map(|(_, w)| *w).sum();
                if total > 0.0 && bar_h > 0.0 {
                    let scale = rect.w / total;
                    let mut x = rect.x;
                    self.out.push_str("0 0 0 rg\n");
                    for (dark, width) in &runs {
                        let w = width * scale;
                        if *dark {
                            let _ =
                                writeln!(self.out, "{}", rect_op(Rect::new(x, bar_y, w, bar_h)));
                        }
                        x += w;
                    }
                    self.out.push_str("f\n");
                }
                if show_text {
                    let mut para = ParaSpec::default();
                    para.h_align = HAlign::Center;
                    let paragraphs = plain_paragraphs(&text, &font, &para);
                    let block = layout_paragraphs(self.book, &paragraphs, Some(rect.w.max(1.0)));
                    let text_y = if text_above {
                        rect.y
                    } else {
                        rect.y + rect.h - text_h
                    };
                    let text_rect = Rect::new(rect.x, text_y, rect.w, text_h);
                    let _ = writeln!(self.out, "1 1 1 rg {} f", rect_op(text_rect));
                    self.emit_text(&block, text_rect, VAlign::Top, Some(rect));
                }
            }
            Symbol::Matrix {
                width,
                height,
                cells,
            } => {
                let (width, height, cells) = if spec.kind == barcode::Symbology::Pdf417 {
                    barcode::landscape(width, height, cells)
                } else {
                    (width, height, cells)
                };
                let Some(layout) = barcode::matrix_modules(&spec, width, height, rect.w, rect.h)
                else {
                    self.unsupported
                        .push("xfa.render.barcode.too_small".to_string());
                    self.out.push_str("Q\n");
                    return;
                };
                let _ = writeln!(
                    self.out,
                    "{} 0 0 {} {} {} cm 0 0 0 rg",
                    n(layout.scale_x),
                    n(layout.scale_y),
                    n(rect.x),
                    n(rect.y)
                );
                for y in 0..height {
                    let row = &cells[y * width..(y + 1) * width];
                    let mut x = 0;
                    while x < width {
                        if !row[x] {
                            x += 1;
                            continue;
                        }
                        let start = x;
                        while x < width && row[x] {
                            x += 1;
                        }
                        let module = Rect::new(
                            layout.left + (start as f64 + 1.0) * layout.module_w,
                            layout.top + (y as f64 + 1.0) * layout.module_h,
                            (x - start) as f64 * layout.module_w,
                            layout.module_h,
                        );
                        let _ = writeln!(self.out, "{}", rect_op(module));
                    }
                }
                self.out.push_str("f\n");
            }
        }
        self.out.push_str("Q\n");
    }

    /// The display text of a field value: choice-list save values mapped
    /// to their display text, then the display picture applied.
    fn display_value(&self, node: &FormNode<'t>, ui: &Ui) -> Option<String> {
        let tpl = self.tpl;
        let raw = self.node_value(node)?;
        if raw.is_empty() {
            return None;
        }
        if let Ui::Choice { .. } = ui {
            let (display, save) = self.choice_items(node);
            if let Some(index) = save.iter().position(|item| *item == raw) {
                if let Some(text) = display.get(index) {
                    return Some(text.clone());
                }
            }
            return Some(raw);
        }
        let kind = tpl
            .child(node.tmpl, "value")
            .and_then(|value| value.children.first())
            .map(|child| child.local_name.as_str())
            .unwrap_or(match ui {
                Ui::Numeric => "decimal",
                Ui::DateTime => "date",
                _ => "text",
            });
        let picture = tpl
            .child(node.tmpl, "format")
            .and_then(|format| tpl.child(format, "picture"))
            .map(|picture| picture.text.trim().to_string())
            .filter(|picture| !picture.is_empty());
        let mut text = format_value_in(&raw, picture.as_deref(), kind, &self.locales, node.locale);
        if let Ui::Password(ch) = ui {
            text = std::iter::repeat_n(*ch, text.chars().count()).collect();
        }
        Some(text)
    }

    /// (display items, save items) of a choice list.
    fn choice_items(&self, node: &FormNode<'t>) -> (Vec<String>, Vec<String>) {
        if let Some(items) = &node.items {
            return items.iter().cloned().unzip();
        }
        let lists: Vec<&XmlNode> = node
            .tmpl
            .children
            .iter()
            .filter(|child| child.local_name == "items")
            .collect();
        let texts = |items: &XmlNode| -> Vec<String> {
            items
                .children
                .iter()
                .map(|item| item.text.trim().to_string())
                .collect()
        };
        match lists.as_slice() {
            [] => (Vec::new(), Vec::new()),
            [only] => (texts(only), texts(only)),
            [first, second, ..] => {
                if first.attr("save") == Some("1") {
                    (texts(second), texts(first))
                } else {
                    (texts(first), texts(second))
                }
            }
        }
    }

    fn paint_edit(&mut self, node: &FormNode<'t>, ui: &Ui, ui_rect: Rect, ui_margin: Insets) {
        let tpl = self.tpl;
        let font = node_font_for(tpl, node);
        let mut para = node_para(tpl, node.tmpl);
        let multi_line = matches!(
            ui,
            Ui::Text {
                multi_line: true,
                ..
            }
        );
        let mut engine = ui_rect.deflate(ui_margin);
        let combo = matches!(ui, Ui::Choice { list: false });
        if combo {
            // CFWL_ComboBox: a drop button on the right.
            let button = Rect::new(
                ui_rect.right() - 17.0_f64.min(ui_rect.w),
                ui_rect.y,
                17.0_f64.min(ui_rect.w),
                ui_rect.h,
            );
            let _ = writeln!(
                self.out,
                "q 0.937 0.937 0.937 rg {} f 0.5 0.5 0.5 RG 0.5 w {} S 0 0 0 rg {} {} m {} {} l {} {} l h f Q",
                rect_op(button),
                rect_op(button),
                n(button.x + button.w / 2.0 - 3.5),
                n(button.y + button.h / 2.0 - 1.75),
                n(button.x + button.w / 2.0 + 3.5),
                n(button.y + button.h / 2.0 - 1.75),
                n(button.x + button.w / 2.0),
                n(button.y + button.h / 2.0 + 1.75)
            );
            engine.w -= button.w;
        }
        let comb_cells = match ui {
            Ui::Text {
                comb: Some(cells), ..
            } => {
                let cells = if *cells > 0 {
                    *cells
                } else {
                    tpl.child(node.tmpl, "value")
                        .and_then(|value| value.children.first())
                        .and_then(|text| tpl.attr(text, "maxChars"))
                        .and_then(|max| max.trim().parse::<usize>().ok())
                        .filter(|max| *max > 0)
                        .unwrap_or(1)
                };
                Some(cells.min(4096))
            }
            _ => None,
        };
        if let Some(cells) = comb_cells {
            // CFWL_Edit::DrawContent comb separators.
            let step = engine.w / cells as f64;
            let mut path = String::new();
            let mut x = engine.x + 1.0;
            for _ in 1..cells {
                x += step;
                let _ = writeln!(
                    path,
                    "{} {} m {} {} l",
                    n(x),
                    n(ui_rect.y),
                    n(x),
                    n(ui_rect.bottom())
                );
            }
            if !path.is_empty() {
                let _ = write!(self.out, "q 0 0 0 RG 1 w\n{}S Q\n", path);
            }
        }
        let Some(text) = self.display_value(node, ui) else {
            return;
        };
        if !multi_line {
            para.margin_right = 0.0;
        }
        let line_height = if para.line_height > 0.1 {
            para.line_height
        } else {
            font.size * 1.2
        };
        let paragraphs: Vec<Paragraph> = match (node.rich, multi_line) {
            (Some(rich), true) => rich_paragraphs(rich, &font, &para),
            _ if multi_line => plain_paragraphs(&text, &font, &para),
            // A single-line edit shows one line.
            _ => plain_paragraphs(&text.replace(['\r', '\n'], " "), &font, &para),
        };
        if let Some(cells) = comb_cells {
            let block = layout_paragraphs_with(self.book, &paragraphs, None, Some(line_height));
            let step = engine.w / cells as f64;
            let dy = match para.v_align {
                VAlign::Top => 0.0,
                VAlign::Middle => ((engine.h - line_height) / 2.0).max(0.0),
                VAlign::Bottom => (engine.h - line_height).max(0.0),
            };
            let font_index = self.book.font(&font);
            let baseline = block.pieces.first().map_or(0.0, |piece| piece.y);
            for (index, ch) in text.chars().take(cells).enumerate() {
                let glyph = ch.to_string();
                let width = self.book.measure(font_index, &glyph, font.size);
                let piece = super::text::Piece {
                    x: engine.x + step * index as f64 + (step - width) / 2.0,
                    y: engine.y + dy + baseline,
                    width,
                    text: glyph,
                    font: font_index,
                    size: font.size,
                    color: font.color,
                    underline: font.underline,
                    line_through: font.line_through,
                };
                super::text::emit_piece(self.book, &mut self.out, &piece, 0.0, 0.0);
            }
            return;
        }
        let wrap = multi_line.then_some(engine.w.max(1.0));
        let block = layout_paragraphs_with(self.book, &paragraphs, wrap, Some(line_height));
        let clip = ui_rect;
        self.emit_text(&block, engine, para.v_align, Some(clip));
    }

    fn paint_list_box(&mut self, node: &FormNode<'t>, rect: Rect, clip: Rect) {
        let tpl = self.tpl;
        let font = node_font_for(tpl, node);
        let (display, save) = self.choice_items(node);
        let selected: Vec<&str> = node
            .value
            .as_deref()
            .map(|value| value.split('\n').collect())
            .unwrap_or_default();
        let line = font.size * 1.2;
        let _ = writeln!(self.out, "q {} W n", rect_op(clip));
        for (index, item) in display.iter().enumerate() {
            let top = rect.y + line * index as f64;
            if top > rect.bottom() {
                break;
            }
            let save_value = save.get(index).unwrap_or(item);
            let is_selected = selected.iter().any(|value| *value == save_value);
            let mut item_font = font.clone();
            if is_selected {
                let _ = writeln!(
                    self.out,
                    "q 0.2 0.6 1 rg {} f Q",
                    rect_op(Rect::new(rect.x, top, rect.w, line))
                );
                item_font.color = [255, 255, 255];
            }
            let paragraphs = plain_paragraphs(item, &item_font, &ParaSpec::default());
            let block = layout_paragraphs_with(self.book, &paragraphs, None, Some(line));
            emit_block(self.book, &mut self.out, &block, rect.x + 1.0, top);
        }
        self.out.push_str("Q\n");
    }

    /// `CXFA_FFCheckButton::PerformLayout` + `CFWL_CheckBoxTP` sign shapes.
    fn paint_check(
        &mut self,
        node: &FormNode<'t>,
        rect: Rect,
        widget: Option<&'t XmlNode>,
        round: bool,
        check_size: f64,
        mark: Option<&str>,
    ) {
        let tpl = self.tpl;
        let margin = node_margin(tpl, node.tmpl);
        let widget_rect = rect.deflate(margin);
        let caption = caption_spec_for(tpl, node).filter(|caption| caption.visible);
        let para = node_para(tpl, node.tmpl);
        let (mut h_align, v_align) = (para.h_align, para.v_align);
        let mut ui_rect = widget_rect;
        let mut caption_draw = None;
        match &caption {
            Some(caption) => {
                let mut caption_rect = widget_rect;
                let vertical = matches!(
                    caption.placement,
                    CaptionPlacement::Top | CaptionPlacement::Bottom
                );
                let mut reserve = caption.reserve.unwrap_or(-1.0);
                if reserve <= 0.0 {
                    reserve = if vertical {
                        widget_rect.h - check_size
                    } else {
                        widget_rect.w - check_size
                    };
                }
                match caption.placement {
                    CaptionPlacement::Left => {
                        caption_rect.w = reserve;
                        ui_rect.w -= reserve;
                        ui_rect.x += reserve;
                    }
                    CaptionPlacement::Top => {
                        caption_rect.h = reserve;
                        ui_rect.h -= reserve;
                        ui_rect.y += reserve;
                    }
                    CaptionPlacement::Right => {
                        caption_rect.x = caption_rect.right() - reserve;
                        caption_rect.w = reserve;
                        ui_rect.w -= reserve;
                    }
                    CaptionPlacement::Bottom => {
                        caption_rect.y = caption_rect.bottom() - reserve;
                        caption_rect.h = reserve;
                        ui_rect.h -= reserve;
                    }
                    CaptionPlacement::Inline => {}
                }
                caption_draw = Some((
                    caption_rect.deflate(caption.margin).normalized(),
                    caption.paragraphs(),
                    caption.para,
                ));
            }
            None => h_align = HAlign::Right,
        }
        match h_align {
            HAlign::Center => ui_rect.x += (ui_rect.w - check_size) / 2.0,
            HAlign::Right => ui_rect.x = ui_rect.right() - check_size,
            _ => {}
        }
        match v_align {
            VAlign::Middle => ui_rect.y += (ui_rect.h - check_size) / 2.0,
            VAlign::Bottom => ui_rect.y = ui_rect.bottom() - check_size,
            VAlign::Top => {}
        }
        ui_rect.w = check_size;
        ui_rect.h = check_size;
        // AddUIMargin.
        let ui_margin = self.ui_margin(widget);
        ui_rect.y -= ui_margin.top / 2.0 - ui_margin.bottom / 2.0;
        let box_rect = ui_rect;
        let ui_border = widget.and_then(|widget| {
            tpl.child(widget, "border")
                .and_then(|border| BoxSpec::from_node(tpl, border))
        });
        if let Some(border) = &ui_border {
            ui_rect = ui_rect.deflate(border.margin);
            self.draw_box(border, ui_rect.normalized(), round);
        }
        if let Some((caption_rect, paragraphs, caption_para)) = caption_draw {
            if !paragraphs.is_empty() {
                let block =
                    layout_paragraphs(self.book, &paragraphs, Some(caption_rect.w.max(1.0)));
                self.emit_text(
                    &block,
                    caption_rect,
                    caption_para.v_align,
                    Some(caption_rect.intersect(rect)),
                );
            }
        }
        // Check state.
        let items: Vec<String> = tpl
            .child(node.tmpl, "items")
            .map(|items| {
                items
                    .children
                    .iter()
                    .map(|item| item.text.trim().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let on = items.first().cloned().unwrap_or_else(|| "1".to_string());
        let neutral = items.get(2).cloned();
        let key = node as *const FormNode<'t> as usize;
        let value = match self.excl_values.get(&key) {
            Some(Some(group_value)) => Some(group_value.clone()),
            _ => self.node_value(node),
        };
        let checked = value.as_deref().map(str::trim) == Some(on.as_str());
        let is_neutral =
            !checked && neutral.is_some() && value.as_deref().map(str::trim) == neutral.as_deref();
        if !checked && !is_neutral {
            return;
        }
        let color = if is_neutral { [169, 169, 169] } else { BLACK };
        // CFWL_CheckBox box: client rect's top-left, check_size square.
        let sign = Rect::new(box_rect.x, box_rect.y, check_size, check_size);
        let sign = sign.deflate(Insets {
            left: sign.w / 4.0,
            top: sign.h / 4.0,
            right: sign.w / 4.0,
            bottom: sign.h / 4.0,
        });
        let shape = match mark {
            Some("check") => "check",
            Some("circle") => "circle",
            Some("cross") => "cross",
            Some("diamond") => "diamond",
            Some("square") => "square",
            Some("star") => "star",
            _ if round => "circle",
            _ => "cross",
        };
        self.paint_sign(shape, sign, color);
    }

    fn paint_sign(&mut self, shape: &str, r: Rect, color: Rgb) {
        let mut path = String::new();
        match shape {
            "circle" => {
                ellipse_path(
                    &mut path,
                    r.x + r.w / 2.0,
                    r.y + r.h / 2.0,
                    r.w / 2.0,
                    r.h / 2.0,
                );
                let _ = write!(self.out, "q {} rg\n{}f Q\n", rgb(color), path);
            }
            "cross" => {
                let _ = writeln!(
                    self.out,
                    "q {} RG 1 w {} {} m {} {} l {} {} m {} {} l S Q",
                    rgb(color),
                    n(r.x),
                    n(r.y),
                    n(r.right()),
                    n(r.bottom()),
                    n(r.x),
                    n(r.bottom()),
                    n(r.right()),
                    n(r.y)
                );
            }
            "diamond" => {
                let _ = writeln!(
                    self.out,
                    "q {} rg {} {} m {} {} l {} {} l {} {} l h f Q",
                    rgb(color),
                    n(r.x + r.w / 2.0),
                    n(r.y),
                    n(r.x),
                    n(r.y + r.h / 2.0),
                    n(r.x + r.w / 2.0),
                    n(r.bottom()),
                    n(r.right()),
                    n(r.y + r.h / 2.0)
                );
            }
            "square" => {
                let _ = writeln!(self.out, "q {} rg {} f Q", rgb(color), rect_op(r));
            }
            "star" => {
                let radius = (r.h) / (1.0 + (PI / 5.0).cos());
                let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                let mut points = Vec::new();
                let mut angle = PI / 10.0;
                for _ in 0..5 {
                    points.push((cx + radius * angle.cos(), cy - radius * angle.sin()));
                    angle += PI * 2.0 / 5.0;
                }
                let mut next = 0;
                let _ = write!(path, "{} {} m ", n(points[0].0), n(points[0].1));
                for _ in 0..5 {
                    next = (next + 2) % 5;
                    let _ = write!(path, "{} {} l ", n(points[next].0), n(points[next].1));
                }
                let _ = writeln!(self.out, "q {} rg {}h f Q", rgb(color), path);
            }
            _ => {
                // PDFium's check path, defined in a 100-unit box.
                let scale = r.w / 100.0;
                let p = |x: f64, y: f64| (r.x + x * scale, r.y + y * scale);
                let (w, hgt, bottom) = (100.0, -100.0, 100.0);
                let pt1 = (w / 15.0, bottom + hgt * 2.0 / 5.0);
                let pt2 = (w / 4.5, bottom + hgt / 16.0);
                let pt3 = (w / 3.0, bottom);
                let pt4 = (w * 14.0 / 15.0, bottom + hgt * 15.0 / 16.0);
                let pt5 = (w / 3.6, bottom + hgt / 3.5);
                let pt12 = (w / 7.0, bottom + hgt * 2.0 / 7.0);
                let pt21 = (w / 5.0, bottom + hgt / 5.0);
                let pt23 = (w / 4.4, bottom);
                let pt32 = (w / 4.0, bottom);
                let pt34 = (w * (1.0 / 7.0 + 7.0 / 15.0), bottom + hgt * 4.0 / 5.0);
                let pt43 = pt34;
                let pt45 = (w * 7.0 / 15.0, bottom + hgt * 8.0 / 7.0);
                let pt54 = (w / 3.4, bottom + hgt / 3.5);
                let pt51 = (w / 3.6, bottom + hgt / 4.0);
                let pt15 = (w / 3.5, bottom + hgt * 3.5 / 5.0);
                let k = 0.552_284_749_830_793_6;
                let scaled = |from: (f64, f64), to: (f64, f64)| {
                    (from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k)
                };
                let segments = [
                    (pt1, pt12, pt2, pt21),
                    (pt2, pt23, pt3, pt32),
                    (pt3, pt34, pt4, pt43),
                    (pt4, pt45, pt5, pt54),
                    (pt5, pt51, pt1, pt15),
                ];
                let start = p(pt1.0, pt1.1);
                let _ = writeln!(path, "{} {} m", n(start.0), n(start.1));
                for (a, a_ctrl, b, b_ctrl) in segments {
                    let c1 = scaled(a, a_ctrl);
                    let c2 = scaled(b, b_ctrl);
                    let (c1, c2, end) = (p(c1.0, c1.1), p(c2.0, c2.1), p(b.0, b.1));
                    let _ = writeln!(
                        path,
                        "{} {} {} {} {} {} c",
                        n(c1.0),
                        n(c1.1),
                        n(c2.0),
                        n(c2.1),
                        n(end.0),
                        n(end.1)
                    );
                }
                let _ = write!(self.out, "q {} rg\n{}f Q\n", rgb(color), path);
            }
        }
    }

    // ------------------------------------------------------------------
    // Images
    // ------------------------------------------------------------------

    fn image_bytes(&mut self, image: &ImageValue) -> Option<Vec<u8>> {
        if !image.data.is_empty() {
            return decode_base64(&image.data);
        }
        let href = image.href.as_deref()?;
        // Only images embedded in the PDF (/Names /XFAImages); external
        // references are never fetched.
        self.named_images
            .get(href)
            .or_else(|| {
                self.named_images
                    .get(href.trim_start_matches(".\\").trim_start_matches("./"))
            })
            .cloned()
    }

    fn paint_image(&mut self, image: &ImageValue, rect: Rect, aspect: &str, para: &ParaSpec) {
        let key = format!(
            "{}:{}",
            image.href.as_deref().unwrap_or(""),
            image.data.len()
        ) + &image.data.chars().take(64).collect::<String>();
        let encoded = match self.image_cache.get(&key) {
            Some(cached) => cached.clone(),
            None => {
                let encoded = self
                    .image_bytes(image)
                    .and_then(|bytes| decode_image(&bytes))
                    .map(|decoded| {
                        let mut encoder = flate2::write::ZlibEncoder::new(
                            Vec::new(),
                            flate2::Compression::default(),
                        );
                        let _ = std::io::Write::write_all(&mut encoder, &decoded.rgb);
                        let compressed = encoder.finish().unwrap_or_default();
                        let mut hex = String::with_capacity(compressed.len() * 2 + 1);
                        for byte in compressed {
                            let _ = write!(hex, "{byte:02X}");
                        }
                        hex.push('>');
                        std::sync::Arc::new((decoded.width, decoded.height, hex))
                    });
                if encoded.is_none() {
                    self.unsupported.push(if image.data.is_empty() {
                        "xfa.render.image_external_reference_not_loaded".to_string()
                    } else {
                        format!("xfa.render.image_unsupported:{}", image.content_type)
                    });
                }
                self.image_cache.insert(key, encoded.clone());
                encoded
            }
        };
        let Some(encoded) = encoded else {
            return;
        };
        let (width, height, ref hex) = *encoded;
        if width == 0 || height == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        // XFA_DrawImage: natural size at 96 dpi, then the aspect rule.
        let natural_w = f64::from(width) * 72.0 / 96.0;
        let natural_h = f64::from(height) * 72.0 / 96.0;
        let (w, h) = match aspect {
            "actual" => (natural_w, natural_h),
            "none" => (rect.w, rect.h),
            "width" => (rect.w, natural_h * rect.w / natural_w),
            "height" => (natural_w * rect.h / natural_h, rect.h),
            _ => {
                let scale = (rect.w / natural_w).min(rect.h / natural_h);
                (natural_w * scale, natural_h * scale)
            }
        };
        let x = match para.h_align {
            HAlign::Center => rect.x + (rect.w - w) / 2.0,
            HAlign::Right => rect.right() - w,
            _ => rect.x,
        };
        let y = match para.v_align {
            VAlign::Middle => rect.y + (rect.h - h) / 2.0,
            VAlign::Bottom => rect.bottom() - h,
            VAlign::Top => rect.y,
        };
        let _ = write!(
            self.out,
            "q {} W n {} 0 0 {} {} {} cm\nBI /W {} /H {} /BPC 8 /CS /RGB /F [/AHx /Fl] ID\n{}\nEI Q\n",
            rect_op(rect),
            n(w),
            n(-h),
            n(x),
            n(y + h),
            width,
            height,
            hex
        );
    }
}

/// Dash operator; like PDFium's `EnableActOnDash`, the pattern scales with
/// the line width.
fn dash_op(dash: Option<&[f64]>, width: f64) -> String {
    let scale = if width > 0.0 { width } else { 1.0 };
    match dash {
        Some(values) => {
            let parts: Vec<String> = values.iter().map(|v| n(*v * scale)).collect();
            format!("[{}] 0 d", parts.join(" "))
        }
        None => "[] 0 d".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_bmp_decode() {
        assert_eq!(decode_base64("aGVsbG8=").unwrap(), b"hello");
        // 1x1 24-bit BMP, red pixel.
        let mut bmp = vec![b'B', b'M'];
        bmp.extend_from_slice(&58u32.to_le_bytes());
        bmp.extend_from_slice(&[0, 0, 0, 0]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&1i32.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&[0, 0, 255, 0]);
        let image = decode_image(&bmp).unwrap();
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.rgb, vec![255, 0, 0]);
    }

    #[test]
    fn arc_path_ends_where_expected() {
        let mut out = String::new();
        arc_path(&mut out, Rect::new(0.0, 0.0, 20.0, 20.0), 0.0, 90.0, true);
        assert!(out.starts_with("20 10 m"));
        assert!(out.trim_end().ends_with("10 0 c"), "{out}");
    }
}
