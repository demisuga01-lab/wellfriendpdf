//! Render-time appearance generation for annotations without a usable
//! `/AP /N` stream.
//!
//! Every function here builds a small Form XObject (dictionary + content
//! bytes) that the page renderer replays exactly like an author-provided
//! appearance. Nothing is written back into the document. The drawings are
//! Wellfriend's own vector designs: sticky-note / attachment / sound icons,
//! standard stamps (rounded border + bold label), line endings, cloudy
//! borders, word-wrapped FreeText and form-field text layout.
use super::widget_synthesis::{append_drop_button, WidgetChrome, DROP_BUTTON_WIDTH};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// A generated appearance plus the page-space rectangle its `/BBox` maps to.
/// `rect` differs from the annotation `/Rect` for fixed-size icons and for
/// geometry whose line endings extend beyond `/Rect`.
pub(crate) struct SynthesizedAppearance {
    pub dict: PdfDictionary,
    pub raw: Vec<u8>,
    pub rect: [f64; 4],
}

/// Nominal icon size (points) for sticky notes and fixed-size icons, like
/// Acrobat's and PDFium's 20 x 20 note.
pub(crate) const ICON_SIZE: f64 = 20.0;

pub(crate) const ANNOT_FLAG_NO_ZOOM: i64 = 1 << 3;
pub(crate) const ANNOT_FLAG_NO_ROTATE: i64 = 1 << 4;

const FIELD_FLAG_COMB: i64 = 1 << 24;

// ---------------------------------------------------------------------------
// Colours, numbers, border styles
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AnnotColor {
    Gray(f64),
    Rgb(f64, f64, f64),
    Cmyk(f64, f64, f64, f64),
}

impl AnnotColor {
    pub(crate) const BLACK: AnnotColor = AnnotColor::Gray(0.0);
    pub(crate) const WHITE: AnnotColor = AnnotColor::Gray(1.0);

    /// `None` = malformed, `Some(None)` = explicitly transparent (`[]`).
    pub(crate) fn from_array(items: &[PdfObject]) -> Option<Option<Self>> {
        let mut values = Vec::with_capacity(items.len());
        for item in items {
            let value = item.as_number()?;
            if !value.is_finite() {
                return None;
            }
            values.push(value.clamp(0.0, 1.0));
        }
        match values.as_slice() {
            [] => Some(None),
            [g] => Some(Some(AnnotColor::Gray(*g))),
            [r, g, b] => Some(Some(AnnotColor::Rgb(*r, *g, *b))),
            [c, m, y, k] => Some(Some(AnnotColor::Cmyk(*c, *m, *y, *k))),
            _ => None,
        }
    }

    pub(crate) fn from_object(obj: Option<&PdfObject>) -> Option<Option<Self>> {
        match obj {
            None => Some(None),
            Some(PdfObject::Array(items)) => Self::from_array(items),
            Some(_) => None,
        }
    }

    pub(crate) fn fill_op(&self) -> String {
        match *self {
            AnnotColor::Gray(g) => format!("{} g", n(g)),
            AnnotColor::Rgb(r, g, b) => format!("{} {} {} rg", n(r), n(g), n(b)),
            AnnotColor::Cmyk(c, m, y, k) => format!("{} {} {} {} k", n(c), n(m), n(y), n(k)),
        }
    }

    pub(crate) fn stroke_op(&self) -> String {
        match *self {
            AnnotColor::Gray(g) => format!("{} G", n(g)),
            AnnotColor::Rgb(r, g, b) => format!("{} {} {} RG", n(r), n(g), n(b)),
            AnnotColor::Cmyk(c, m, y, k) => format!("{} {} {} {} K", n(c), n(m), n(y), n(k)),
        }
    }

    /// Approximate relative luminance, used to pick legible detail colours.
    pub(crate) fn luminance(&self) -> f64 {
        let (r, g, b) = match *self {
            AnnotColor::Gray(g) => (g, g, g),
            AnnotColor::Rgb(r, g, b) => (r, g, b),
            AnnotColor::Cmyk(c, m, y, k) => (
                (1.0 - c) * (1.0 - k),
                (1.0 - m) * (1.0 - k),
                (1.0 - y) * (1.0 - k),
            ),
        };
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }
}

/// Compact PDF number formatting (at most 3 decimals, no trailing zeros).
pub(crate) fn n(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut s = format!("{value:.3}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        "0".to_string()
    } else {
        s
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BorderKind {
    Solid,
    Dashed,
    Beveled,
    Inset,
    Underline,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BorderStyle {
    pub width: f64,
    pub kind: BorderKind,
    pub dash: Vec<f64>,
}

impl BorderStyle {
    pub(crate) fn dash_op(&self) -> String {
        if self.kind == BorderKind::Dashed && !self.dash.is_empty() {
            let parts: Vec<String> = self.dash.iter().map(|v| n(*v)).collect();
            format!("[{}] 0 d", parts.join(" "))
        } else {
            "[] 0 d".to_string()
        }
    }
}

fn resolve(reader: &PdfReader, obj: &PdfObject) -> Option<PdfObject> {
    reader.resolve(obj.clone()).ok()
}

fn resolve_dict(reader: &PdfReader, obj: Option<&PdfObject>) -> Option<PdfDictionary> {
    match resolve(reader, obj?)? {
        PdfObject::Dictionary(dict) => Some(dict),
        _ => None,
    }
}

fn number(reader: &PdfReader, obj: Option<&PdfObject>) -> Option<f64> {
    let value = match obj? {
        PdfObject::Reference { .. } => resolve(reader, obj?)?.as_number()?,
        other => other.as_number()?,
    };
    value.is_finite().then_some(value)
}

fn number_array(reader: &PdfReader, obj: Option<&PdfObject>) -> Option<Vec<f64>> {
    let resolved = resolve(reader, obj?)?;
    let items = resolved.as_array()?;
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        values.push(number(reader, Some(item))?);
    }
    Some(values)
}

/// Border style from `/BS` (W, S, D) or the legacy `/Border` array.
pub(crate) fn border_style(
    annot: &PdfDictionary,
    reader: &PdfReader,
    default_width: f64,
) -> BorderStyle {
    let mut style = BorderStyle {
        width: default_width,
        kind: BorderKind::Solid,
        dash: vec![3.0],
    };
    if let Some(bs) = resolve_dict(reader, annot.get("BS")) {
        if let Some(width) = number(reader, bs.get("W")) {
            style.width = width.max(0.0);
        }
        style.kind = match bs.get_name("S") {
            Some("D") => BorderKind::Dashed,
            Some("B") => BorderKind::Beveled,
            Some("I") => BorderKind::Inset,
            Some("U") => BorderKind::Underline,
            _ => BorderKind::Solid,
        };
        if let Some(dash) = number_array(reader, bs.get("D")) {
            style.dash = sanitize_dash(dash);
        }
    } else if let Some(border) = number_array(reader, annot.get("Border")).or_else(|| {
        // /Border may carry a dash sub-array that number_array rejects.
        let resolved = resolve(reader, annot.get("Border")?)?;
        let items = resolved.as_array()?;
        let width = items.get(2).and_then(|v| number(reader, Some(v)))?;
        Some(vec![0.0, 0.0, width])
    }) {
        if let Some(width) = border.get(2) {
            style.width = width.max(0.0);
        }
        if let Some(PdfObject::Array(items)) = annot.get("Border") {
            if let Some(dash) = items.get(3).and_then(|d| number_array(reader, Some(d))) {
                let dash = sanitize_dash(dash);
                if !dash.is_empty() {
                    style.kind = BorderKind::Dashed;
                    style.dash = dash;
                }
            }
        }
    }
    if style.kind == BorderKind::Dashed && style.dash.is_empty() {
        style.dash = vec![3.0];
    }
    style
}

fn sanitize_dash(dash: Vec<f64>) -> Vec<f64> {
    if dash.iter().any(|v| !v.is_finite() || *v < 0.0) || dash.iter().all(|v| *v == 0.0) {
        return Vec::new();
    }
    dash.into_iter().take(16).collect()
}

fn opacity(annot: &PdfDictionary) -> f64 {
    annot
        .get("CA")
        .and_then(PdfObject::as_number)
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(1.0)
}

fn normalized_rect(rect: [f64; 4]) -> [f64; 4] {
    [
        rect[0].min(rect[2]),
        rect[1].min(rect[3]),
        rect[0].max(rect[2]),
        rect[1].max(rect[3]),
    ]
}

/// `/RD` inset differences `[left bottom right top]`, clamped so the inner
/// rectangle never inverts.
fn rect_differences(annot: &PdfDictionary, reader: &PdfReader, w: f64, h: f64) -> [f64; 4] {
    let Some(values) = number_array(reader, annot.get("RD")) else {
        return [0.0; 4];
    };
    if values.len() != 4 || values.iter().any(|v| *v < 0.0) {
        return [0.0; 4];
    }
    if values[0] + values[2] >= w || values[1] + values[3] >= h {
        return [0.0; 4];
    }
    [values[0], values[1], values[2], values[3]]
}

// ---------------------------------------------------------------------------
// Form assembly
// ---------------------------------------------------------------------------

#[derive(Default)]
pub(crate) struct Resources {
    fonts: BTreeMap<String, PdfObject>,
    xobjects: BTreeMap<String, PdfObject>,
    alpha: Option<f64>,
}

impl Resources {
    pub(crate) fn add_font(&mut self, name: &str, obj: PdfObject) {
        self.fonts.entry(name.to_string()).or_insert(obj);
    }

    pub(crate) fn add_xobject(&mut self, name: &str, obj: PdfObject) {
        self.xobjects.entry(name.to_string()).or_insert(obj);
    }

    pub(crate) fn into_dict(self) -> PdfDictionary {
        let mut resources = PdfDictionary::empty();
        if !self.fonts.is_empty() {
            let mut fonts = PdfDictionary::empty();
            for (name, obj) in self.fonts {
                fonts.insert(name, obj);
            }
            resources.insert("Font", PdfObject::Dictionary(fonts));
        }
        if !self.xobjects.is_empty() {
            let mut xobjects = PdfDictionary::empty();
            for (name, obj) in self.xobjects {
                xobjects.insert(name, obj);
            }
            resources.insert("XObject", PdfObject::Dictionary(xobjects));
        }
        if let Some(alpha) = self.alpha {
            let mut gs = PdfDictionary::empty();
            gs.insert("Type", PdfObject::Name("ExtGState".into()));
            gs.insert("CA", PdfObject::Real(alpha));
            gs.insert("ca", PdfObject::Real(alpha));
            let mut ext = PdfDictionary::empty();
            ext.insert("GS1", PdfObject::Dictionary(gs));
            resources.insert("ExtGState", PdfObject::Dictionary(ext));
        }
        resources
    }
}

fn finish(
    content: String,
    bbox: [f64; 4],
    rect: [f64; 4],
    mut resources: Resources,
    alpha: f64,
) -> Option<SynthesizedAppearance> {
    if content.trim().is_empty() {
        return None;
    }
    let content = if alpha < 1.0 {
        resources.alpha = Some(alpha);
        format!("q /GS1 gs\n{content}Q\n")
    } else {
        content
    };
    let mut dict = PdfDictionary::empty();
    dict.insert("Type", PdfObject::Name("XObject".into()));
    dict.insert("Subtype", PdfObject::Name("Form".into()));
    dict.insert(
        "BBox",
        PdfObject::Array(bbox.iter().map(|v| PdfObject::Real(*v)).collect()),
    );
    dict.insert("Resources", PdfObject::Dictionary(resources.into_dict()));
    dict.insert("Length", PdfObject::Integer(content.len() as i64));
    Some(SynthesizedAppearance {
        dict,
        raw: content.into_bytes(),
        rect,
    })
}

fn standard_font_dict(base: &str) -> PdfDictionary {
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".into()));
    font.insert("Subtype", PdfObject::Name("Type1".into()));
    font.insert("BaseFont", PdfObject::Name(base.into()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
    font
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

const KAPPA: f64 = 0.552_284_749_830_793_6;

pub(crate) fn ellipse_path(out: &mut String, cx: f64, cy: f64, rx: f64, ry: f64) {
    let kx = rx * KAPPA;
    let ky = ry * KAPPA;
    let _ = writeln!(
        out,
        "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h",
        n(cx + rx),
        n(cy),
        n(cx + rx),
        n(cy + ky),
        n(cx + kx),
        n(cy + ry),
        n(cx),
        n(cy + ry),
        n(cx - kx),
        n(cy + ry),
        n(cx - rx),
        n(cy + ky),
        n(cx - rx),
        n(cy),
        n(cx - rx),
        n(cy - ky),
        n(cx - kx),
        n(cy - ry),
        n(cx),
        n(cy - ry),
        n(cx + kx),
        n(cy - ry),
        n(cx + rx),
        n(cy - ky),
        n(cx + rx),
        n(cy)
    );
}

fn rounded_rect_path(out: &mut String, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let k = r * KAPPA;
    let (x1, y1) = (x + w, y + h);
    let _ = writeln!(
        out,
        "{} {} m {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c h",
        n(x + r),
        n(y),
        n(x1 - r),
        n(y),
        n(x1 - r + k),
        n(y),
        n(x1),
        n(y + r - k),
        n(x1),
        n(y + r),
        n(x1),
        n(y1 - r),
        n(x1),
        n(y1 - r + k),
        n(x1 - r + k),
        n(y1),
        n(x1 - r),
        n(y1),
        n(x + r),
        n(y1),
        n(x + r - k),
        n(y1),
        n(x),
        n(y1 - r + k),
        n(x),
        n(y1 - r),
        n(x),
        n(y + r),
        n(x),
        n(y + r - k),
        n(x + r - k),
        n(y),
        n(x + r),
        n(y)
    );
}

fn polygon_path(out: &mut String, points: &[(f64, f64)], close: bool) {
    for (index, (x, y)) in points.iter().enumerate() {
        let op = if index == 0 { "m" } else { "l" };
        let _ = write!(out, "{} {} {op} ", n(*x), n(*y));
    }
    if close {
        out.push('h');
    }
    out.push('\n');
}

/// Semicircular "cloud" bumps along a closed polygon (counter-clockwise),
/// bulging outward. `radius` is the bump radius.
fn cloud_path(out: &mut String, polygon: &[(f64, f64)], radius: f64) {
    let mut samples: Vec<(f64, f64)> = Vec::new();
    let count = polygon.len();
    for index in 0..count {
        let a = polygon[index];
        let b = polygon[(index + 1) % count];
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let steps = ((len / (2.0 * radius)).round() as usize).clamp(1, 512);
        for step in 0..steps {
            let t = step as f64 / steps as f64;
            samples.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    }
    if samples.len() < 3 {
        polygon_path(out, polygon, true);
        return;
    }
    let _ = write!(out, "{} {} m ", n(samples[0].0), n(samples[0].1));
    for index in 0..samples.len() {
        let a = samples[index];
        let b = samples[(index + 1) % samples.len()];
        let c = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
        let rho = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt() * 0.5;
        if rho <= 1e-6 {
            continue;
        }
        let u = ((a.0 - c.0) / rho, (a.1 - c.1) / rho);
        // Outward normal of a counter-clockwise edge a->b is (dy, -dx); the
        // edge direction is -u.
        let nrm = (-u.1, u.0);
        let top = (c.0 + rho * nrm.0, c.1 + rho * nrm.1);
        let k = KAPPA * rho;
        let _ = write!(
            out,
            "{} {} {} {} {} {} c {} {} {} {} {} {} c ",
            n(a.0 + k * nrm.0),
            n(a.1 + k * nrm.1),
            n(top.0 + k * u.0),
            n(top.1 + k * u.1),
            n(top.0),
            n(top.1),
            n(top.0 - k * u.0),
            n(top.1 - k * u.1),
            n(b.0 + k * nrm.0),
            n(b.1 + k * nrm.1),
            n(b.0),
            n(b.1)
        );
    }
    out.push_str("h\n");
}

fn cloud_radius(intensity: f64) -> f64 {
    // Intensity 0..2 per ISO 32000; 1 ≈ Acrobat's default bump size.
    (4.0 + 2.0 * intensity.clamp(0.0, 2.0)).max(2.0)
}

fn cloudy_intensity(annot: &PdfDictionary, reader: &PdfReader) -> Option<f64> {
    let be = resolve_dict(reader, annot.get("BE"))?;
    if be.get_name("S") != Some("C") {
        return None;
    }
    Some(number(reader, be.get("I")).unwrap_or(1.0))
}

// ---------------------------------------------------------------------------
// Line endings
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineEnding {
    None,
    Square,
    Circle,
    Diamond,
    OpenArrow,
    ClosedArrow,
    Butt,
    ROpenArrow,
    RClosedArrow,
    Slash,
}

impl LineEnding {
    pub(crate) fn from_name(name: &str) -> Self {
        match name {
            "Square" => LineEnding::Square,
            "Circle" => LineEnding::Circle,
            "Diamond" => LineEnding::Diamond,
            "OpenArrow" => LineEnding::OpenArrow,
            "ClosedArrow" => LineEnding::ClosedArrow,
            "Butt" => LineEnding::Butt,
            "ROpenArrow" => LineEnding::ROpenArrow,
            "RClosedArrow" => LineEnding::RClosedArrow,
            "Slash" => LineEnding::Slash,
            _ => LineEnding::None,
        }
    }

    /// How far the main line is pulled back from the endpoint so it does not
    /// poke through a closed ending.
    fn inset(self, size: f64) -> f64 {
        match self {
            LineEnding::Square | LineEnding::Circle | LineEnding::Diamond => size * 0.5,
            LineEnding::ClosedArrow => size * (30f64).to_radians().cos(),
            _ => 0.0,
        }
    }
}

fn line_endings(annot: &PdfDictionary, reader: &PdfReader) -> (LineEnding, LineEnding) {
    let Some(PdfObject::Array(items)) = annot.get("LE").and_then(|obj| resolve(reader, obj)) else {
        return (LineEnding::None, LineEnding::None);
    };
    let at = |index: usize| {
        items
            .get(index)
            .and_then(PdfObject::as_name)
            .map(LineEnding::from_name)
            .unwrap_or(LineEnding::None)
    };
    (at(0), at(1))
}

fn ending_size(width: f64, segment_len: f64) -> f64 {
    let size = 6.0 * width.max(1.0);
    if segment_len > 0.0 {
        size.min(segment_len * 0.5).max(width.max(1.0) * 2.0)
    } else {
        size
    }
}

/// Draws one line ending with its tip at `tip`, `dir` pointing outward along
/// the line (unit vector). Closed endings are filled with `interior` when
/// present. Extends `bounds` by the drawn geometry.
fn append_line_ending(
    out: &mut String,
    style: LineEnding,
    tip: (f64, f64),
    dir: (f64, f64),
    size: f64,
    interior: Option<AnnotColor>,
    bounds: &mut Bounds,
) {
    if style == LineEnding::None {
        return;
    }
    let (c30, s30) = ((30f64).to_radians().cos(), (30f64).to_radians().sin());
    let half = size * 0.5;
    let mut local = String::new();
    let mut closed = true;
    match style {
        LineEnding::None => return,
        LineEnding::Square => {
            let _ = writeln!(
                local,
                "{} {} {} {} re",
                n(-half),
                n(-half),
                n(size),
                n(size)
            );
        }
        LineEnding::Circle => ellipse_path(&mut local, 0.0, 0.0, half, half),
        LineEnding::Diamond => polygon_path(
            &mut local,
            &[(half, 0.0), (0.0, half), (-half, 0.0), (0.0, -half)],
            true,
        ),
        LineEnding::OpenArrow | LineEnding::ClosedArrow => {
            closed = style == LineEnding::ClosedArrow;
            polygon_path(
                &mut local,
                &[
                    (-size * c30, size * s30),
                    (0.0, 0.0),
                    (-size * c30, -size * s30),
                ],
                closed,
            );
        }
        LineEnding::ROpenArrow | LineEnding::RClosedArrow => {
            closed = style == LineEnding::RClosedArrow;
            polygon_path(
                &mut local,
                &[
                    (size * c30, size * s30),
                    (0.0, 0.0),
                    (size * c30, -size * s30),
                ],
                closed,
            );
        }
        LineEnding::Butt => {
            closed = false;
            polygon_path(&mut local, &[(0.0, half), (0.0, -half)], false);
        }
        LineEnding::Slash => {
            closed = false;
            let (dx, dy) = (half * 0.5, half * (60f64).to_radians().sin());
            polygon_path(&mut local, &[(-dx, -dy), (dx, dy)], false);
        }
    }
    let paint = match (closed, interior) {
        (true, Some(color)) => format!("{} B", color.fill_op()),
        (true, None) => "S".to_string(),
        (false, _) => "S".to_string(),
    };
    let _ = writeln!(
        out,
        "q [] 0 d {} {} {} {} {} {} cm\n{local}{paint}\nQ",
        n(dir.0),
        n(dir.1),
        n(-dir.1),
        n(dir.0),
        n(tip.0),
        n(tip.1)
    );
    bounds.add_point(tip.0, tip.1, size * 1.2);
}

#[derive(Clone, Copy)]
struct Bounds {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Bounds {
    fn new() -> Self {
        Self {
            x0: f64::INFINITY,
            y0: f64::INFINITY,
            x1: f64::NEG_INFINITY,
            y1: f64::NEG_INFINITY,
        }
    }

    fn add_point(&mut self, x: f64, y: f64, pad: f64) {
        self.x0 = self.x0.min(x - pad);
        self.y0 = self.y0.min(y - pad);
        self.x1 = self.x1.max(x + pad);
        self.y1 = self.y1.max(y + pad);
    }

    fn add_rect(&mut self, rect: [f64; 4]) {
        self.x0 = self.x0.min(rect[0]);
        self.y0 = self.y0.min(rect[1]);
        self.x1 = self.x1.max(rect[2]);
        self.y1 = self.y1.max(rect[3]);
    }

    fn to_rect(self) -> Option<[f64; 4]> {
        (self.x0.is_finite()
            && self.y0.is_finite()
            && self.x1.is_finite()
            && self.y1.is_finite()
            && self.x1 > self.x0
            && self.y1 > self.y0)
            .then_some([self.x0, self.y0, self.x1, self.y1])
    }
}

fn unit(dx: f64, dy: f64) -> Option<(f64, f64)> {
    let len = (dx * dx + dy * dy).sqrt();
    (len > 1e-9 && len.is_finite()).then(|| (dx / len, dy / len))
}

fn points_from(values: &[f64]) -> Vec<(f64, f64)> {
    values
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

// ---------------------------------------------------------------------------
// Entry point for non-widget annotations
// ---------------------------------------------------------------------------

/// Context shared by text-bearing synthesized appearances.
pub(crate) struct SynthesisContext<'a> {
    pub reader: &'a PdfReader,
    /// AcroForm `/DR`, used to resolve FreeText `/DA` fonts.
    pub acroform_dr: Option<PdfDictionary>,
}

/// Generates an appearance for a non-widget annotation. `color_default`
/// controls the colour used when `/C` is absent.
pub(crate) fn synthesize_markup(
    annot: &PdfDictionary,
    ctx: &SynthesisContext<'_>,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    let subtype = annot.get_name("Subtype")?;
    let rect = normalized_rect(rect);
    let reader = ctx.reader;
    match subtype {
        "Text" => text_icon(annot, reader, rect),
        "FileAttachment" => file_attachment_icon(annot, reader, rect),
        "Sound" => sound_icon(annot, reader, rect),
        "Stamp" => stamp(annot, reader, rect),
        "Line" => line(annot, reader, rect),
        "PolyLine" | "Polygon" => poly(annot, reader, rect, subtype == "Polygon"),
        "Square" | "Circle" => square_circle(annot, reader, rect, subtype == "Circle"),
        "Ink" => ink(annot, reader, rect),
        "Caret" => caret(annot, reader, rect),
        "FreeText" => free_text(annot, ctx, rect),
        "Link" => link_border(annot, reader, rect),
        _ => None,
    }
}

/// A Link annotation's border: with an explicit `/Border` or `/BS` of
/// positive width and a `/C` colour, the rectangle is stroked inside its
/// `/Rect` (Poppler's `AnnotLink::draw`). PDFium draws the same box, but
/// nothing for the underline style.
fn link_border(
    annot: &PdfDictionary,
    reader: &PdfReader,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    if annot.get("Border").is_none() && annot.get("BS").is_none() {
        return None;
    }
    let border = border_style(annot, reader, 1.0);
    // PDFium draws no box for an underline-style link border.
    if border.kind == BorderKind::Underline {
        return None;
    }
    let color = stroke_color(annot, None)?;
    let width = rect[2] - rect[0];
    let height = rect[3] - rect[1];
    let inset = border.width * 0.5;
    if border.width <= 0.0 || width <= border.width || height <= border.width {
        return None;
    }
    let mut out = String::from("q\n");
    stroke_setup(&mut out, color, &border);
    let _ = writeln!(
        out,
        "{} {} {} {} re S\nQ",
        n(inset),
        n(inset),
        n(width - border.width),
        n(height - border.width)
    );
    finish(
        out,
        [0.0, 0.0, width, height],
        rect,
        Resources::default(),
        opacity(annot),
    )
}

fn stroke_color(annot: &PdfDictionary, default: Option<AnnotColor>) -> Option<AnnotColor> {
    match AnnotColor::from_object(annot.get("C")) {
        Some(Some(color)) => Some(color),
        // Explicit empty /C = transparent: nothing is stroked.
        Some(None) if annot.get("C").is_some() => None,
        Some(None) => default,
        None => default,
    }
}

fn interior_color(annot: &PdfDictionary) -> Option<AnnotColor> {
    AnnotColor::from_object(annot.get("IC")).flatten()
}

// ---------------------------------------------------------------------------
// Icons (Text, FileAttachment, Sound)
// ---------------------------------------------------------------------------

/// Rect for a fixed-size icon anchored at the top-left of `/Rect`.
fn fixed_icon_rect(rect: [f64; 4]) -> [f64; 4] {
    [rect[0], rect[3] - ICON_SIZE, rect[0] + ICON_SIZE, rect[3]]
}

fn icon_colors(annot: &PdfDictionary, default: AnnotColor) -> (AnnotColor, AnnotColor, AnnotColor) {
    let fill = match AnnotColor::from_object(annot.get("C")) {
        Some(Some(color)) => color,
        _ => default,
    };
    let outline = AnnotColor::Gray(0.15);
    let detail = if fill.luminance() < 0.45 {
        AnnotColor::WHITE
    } else {
        AnnotColor::Gray(0.15)
    };
    (fill, outline, detail)
}

fn icon_flags(annot: &PdfDictionary) -> i64 {
    annot.get_integer("F").unwrap_or(0)
}

fn text_icon(
    annot: &PdfDictionary,
    reader: &PdfReader,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    let _ = reader;
    let name = annot.get_name("Name").unwrap_or("Note");
    // PDFium/Acrobat default note colour is yellow.
    let (fill, outline, detail) = icon_colors(annot, AnnotColor::Rgb(1.0, 1.0, 0.0));
    let mut out = String::new();
    let _ = writeln!(
        out,
        "q 1 j 1 J {} {} 0.8 w",
        fill.fill_op(),
        outline.stroke_op()
    );
    match name {
        "Comment" => icon_comment(&mut out, detail),
        "Key" => icon_key(&mut out),
        "Help" => icon_help(&mut out, detail),
        "NewParagraph" => icon_new_paragraph(&mut out, outline),
        "Paragraph" => icon_paragraph(&mut out),
        "Insert" => icon_insert(&mut out),
        "Check" => icon_symbol(&mut out, CheckGlyph::Check),
        "Circle" => icon_symbol(&mut out, CheckGlyph::Circle),
        "Cross" => icon_symbol(&mut out, CheckGlyph::Cross),
        "Star" => icon_symbol(&mut out, CheckGlyph::Star),
        _ => icon_note(&mut out, detail),
    }
    out.push_str("Q\n");
    // Text annotations keep a fixed icon size at the top-left corner of
    // /Rect (they behave as NoZoom), like Acrobat, PDFium and Poppler.
    let target = fixed_icon_rect(rect);
    finish(
        out,
        [0.0, 0.0, ICON_SIZE, ICON_SIZE],
        target,
        Resources::default(),
        opacity(annot),
    )
}

fn icon_note(out: &mut String, detail: AnnotColor) {
    // Sheet with a folded top-right corner and ruled lines.
    polygon_path(
        out,
        &[
            (3.5, 1.5),
            (16.5, 1.5),
            (16.5, 14.5),
            (12.5, 18.5),
            (3.5, 18.5),
        ],
        true,
    );
    out.push_str("B\n");
    polygon_path(out, &[(12.5, 18.5), (12.5, 14.5), (16.5, 14.5)], false);
    out.push_str("S\n");
    let _ = writeln!(out, "q {} 1 w 0 J", detail.stroke_op());
    for (y, x1) in [(14.0, 10.5), (11.0, 14.5), (8.0, 14.5), (5.0, 14.5)] {
        let _ = writeln!(out, "6 {} m {} {} l S", n(y), n(x1), n(y));
    }
    out.push_str("Q\n");
}

fn icon_comment(out: &mut String, detail: AnnotColor) {
    // Speech bubble with a tail at the lower left.
    let (x, y, w, h, r) = (1.5, 6.0, 17.0, 12.5, 3.0);
    let k = r * KAPPA;
    let _ = writeln!(
        out,
        "{} {} m {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c {} {} l {} {} {} {} {} {} c 9 {} l 4 1.5 l 5.5 {} l h B",
        n(x + r), n(y),
        n(x + w - r), n(y),
        n(x + w - r + k), n(y), n(x + w), n(y + r - k), n(x + w), n(y + r),
        n(x + w), n(y + h - r),
        n(x + w), n(y + h - r + k), n(x + w - r + k), n(y + h), n(x + w - r), n(y + h),
        n(x + r), n(y + h),
        n(x + r - k), n(y + h), n(x), n(y + h - r + k), n(x), n(y + h - r),
        n(x), n(y + r),
        n(x), n(y + r - k), n(x + r - k), n(y), n(x + r), n(y),
        n(y), n(y)
    );
    let _ = writeln!(
        out,
        "q {} 1 w 0 J 5 14.5 m 15 14.5 l S 5 11.5 m 15 11.5 l S 5 8.5 m 11.5 8.5 l S Q",
        detail.stroke_op()
    );
}

fn icon_key(out: &mut String) {
    // Diagonal key: round bow with a hole, shaft and two bits.
    out.push_str("q 0.7071 -0.7071 0.7071 0.7071 6.5 13.5 cm\n");
    ellipse_path(out, 0.0, 0.0, 4.5, 4.5);
    out.push_str("B\n");
    out.push_str("3.8 -1.2 10.2 2.4 re B\n");
    out.push_str("9.8 -3.8 1.6 2.6 re B\n");
    out.push_str("12.4 -3.8 1.6 2.6 re B\n");
    out.push_str("q 1 g\n");
    ellipse_path(out, -1.4, 0.0, 1.5, 1.5);
    out.push_str("B Q\nQ\n");
}

fn icon_help(out: &mut String, detail: AnnotColor) {
    ellipse_path(out, 10.0, 10.0, 8.5, 8.5);
    out.push_str("B\n");
    let _ = writeln!(
        out,
        "q {} {} 2 w 1 J 7 12.6 m 7 14.6 8.4 15.6 10 15.6 c 11.7 15.6 13 14.5 13 12.9 c 13 11 10 10.7 10 8.4 c S",
        detail.stroke_op(),
        detail.fill_op()
    );
    ellipse_path(out, 10.0, 5.0, 1.25, 1.25);
    out.push_str("f Q\n");
}

fn icon_new_paragraph(out: &mut String, letters: AnnotColor) {
    polygon_path(out, &[(10.0, 19.0), (16.5, 11.0), (3.5, 11.0)], true);
    out.push_str("B\n");
    let _ = writeln!(
        out,
        "q {} 1.4 w 0 J 0 j 4 2 m 4 8.6 l 8.4 2 l 8.4 8.6 l S 11.2 2 m 11.2 8.6 l 13.6 8.6 l 15.6 8.6 15.6 5.3 13.6 5.3 c 11.2 5.3 l S Q",
        letters.stroke_op()
    );
}

fn icon_paragraph(out: &mut String) {
    // Pilcrow: bowl plus two stems, filled with the annotation colour.
    out.push_str(
        "8.5 18.5 m 16 18.5 l 16 17 l 14.5 17 l 14.5 1.5 l 13 1.5 l 13 17 l 11.5 17 l \
         11.5 1.5 l 10 1.5 l 10 9.5 l 8.5 9.5 l 6 9.5 4 11.5 4 14 c 4 16.5 6 18.5 8.5 18.5 c h B\n",
    );
}

fn icon_insert(out: &mut String) {
    out.push_str("10 17.5 m 18.5 2.5 l 14.6 2.5 l 10 10.8 l 5.4 2.5 l 1.5 2.5 l h B\n");
}

fn icon_symbol(out: &mut String, glyph: CheckGlyph) {
    out.push_str("q 15 0 0 15 2.5 2.5 cm 0.06 w\n");
    append_check_glyph_unit(out, glyph, true);
    out.push_str("Q\n");
}

fn file_attachment_icon(
    annot: &PdfDictionary,
    reader: &PdfReader,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    let _ = reader;
    let name = annot.get_name("Name").unwrap_or("PushPin");
    let (fill, outline, detail) = icon_colors(annot, AnnotColor::Rgb(1.0, 1.0, 0.0));
    let mut out = String::new();
    let _ = writeln!(
        out,
        "q 1 j 1 J {} {} 0.8 w",
        fill.fill_op(),
        outline.stroke_op()
    );
    match name {
        "Paperclip" => {
            // Elongated wire loop drawn as a thick outlined stroke.
            let path = "8 4 m 8 15 l 8 17.5 12 17.5 12 15 c 12 5.5 l 12 3 6 3 6 5.5 c 6 13.5 l";
            let _ = writeln!(
                out,
                "q {} 3.2 w {path} S Q q {} 1.6 w {path} S Q",
                outline.stroke_op(),
                fill.stroke_op()
            );
        }
        "Graph" => {
            out.push_str("2 2 16 16 re B\n");
            let _ = writeln!(
                &mut out,
                "q {} {} 1 w 0 j 4.5 4 2.5 6 re f 8.75 4 2.5 10 re f 13 4 2.5 4 re f 3.5 4 m 16.5 4 l S Q",
                detail.fill_op(),
                detail.stroke_op()
            );
        }
        "Tag" => {
            polygon_path(
                &mut out,
                &[
                    (2.0, 10.0),
                    (8.5, 17.0),
                    (18.0, 17.0),
                    (18.0, 3.0),
                    (8.5, 3.0),
                ],
                true,
            );
            out.push_str("B\n");
            let _ = writeln!(out, "q {}", detail.fill_op());
            ellipse_path(&mut out, 7.5, 10.0, 1.4, 1.4);
            let _ = writeln!(
                &mut out,
                "f {} 1 w 11 13 m 16 13 l S 11 10 m 16 10 l S 11 7 m 16 7 l S Q",
                detail.stroke_op()
            );
        }
        _ => {
            // Push pin: needle, collar and round head along a diagonal.
            let _ = writeln!(
                out,
                "q {} 1.4 w 1 J 9.5 9.5 m 2.5 2.5 l S Q",
                outline.stroke_op()
            );
            polygon_path(
                &mut out,
                &[(7.0, 11.5), (11.5, 7.0), (13.0, 8.5), (8.5, 13.0)],
                true,
            );
            out.push_str("B\n");
            ellipse_path(&mut out, 13.5, 13.5, 4.8, 4.8);
            out.push_str("B\n");
        }
    }
    out.push_str("Q\n");
    icon_finish(annot, out, rect)
}

fn sound_icon(
    annot: &PdfDictionary,
    reader: &PdfReader,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    let _ = reader;
    let name = annot.get_name("Name").unwrap_or("Speaker");
    let (fill, outline, _) = icon_colors(annot, AnnotColor::Rgb(1.0, 1.0, 0.0));
    let mut out = String::new();
    let _ = writeln!(
        out,
        "q 1 j 1 J {} {} 0.8 w",
        fill.fill_op(),
        outline.stroke_op()
    );
    if name == "Mic" {
        // Capsule, cradle and stand.
        rounded_rect_path(&mut out, 7.0, 8.0, 6.0, 10.5, 3.0);
        out.push_str("B\n");
        let _ = writeln!(
            out,
            "q {} 1.3 w 4.5 11.5 m 4.5 7 7 5 10 5 c 13 5 15.5 7 15.5 11.5 c S 10 5 m 10 2 l S 6.5 2 m 13.5 2 l S Q",
            outline.stroke_op()
        );
    } else {
        // Speaker cone with sound waves.
        polygon_path(
            &mut out,
            &[
                (2.0, 7.0),
                (6.0, 7.0),
                (11.0, 2.5),
                (11.0, 17.5),
                (6.0, 13.0),
                (2.0, 13.0),
            ],
            true,
        );
        out.push_str("B\n");
        let _ = writeln!(
            out,
            "q {} 1.2 w 13.5 7.5 m 14.8 8.8 14.8 11.2 13.5 12.5 c S 15.8 5.2 m 18.4 7.8 18.4 12.2 15.8 14.8 c S Q",
            outline.stroke_op()
        );
    }
    out.push_str("Q\n");
    icon_finish(annot, out, rect)
}

/// Icon annotations other than Text are fitted into /Rect (Poppler), unless
/// NoZoom asks for a fixed-size icon anchored at the top-left corner.
fn icon_finish(
    annot: &PdfDictionary,
    out: String,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    let target = if icon_flags(annot) & ANNOT_FLAG_NO_ZOOM != 0 {
        fixed_icon_rect(rect)
    } else {
        rect
    };
    finish(
        out,
        [0.0, 0.0, ICON_SIZE, ICON_SIZE],
        target,
        Resources::default(),
        opacity(annot),
    )
}

// ---------------------------------------------------------------------------
// Check-box / dingbat glyphs
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CheckGlyph {
    Check,
    Circle,
    Cross,
    Diamond,
    Square,
    Star,
}

impl CheckGlyph {
    /// ZapfDingbats caption characters used by `/MK /CA`.
    pub(crate) fn from_caption(caption: &str) -> Option<Self> {
        match caption {
            "4" => Some(CheckGlyph::Check),
            "l" => Some(CheckGlyph::Circle),
            "8" => Some(CheckGlyph::Cross),
            "u" => Some(CheckGlyph::Diamond),
            "n" => Some(CheckGlyph::Square),
            "H" => Some(CheckGlyph::Star),
            _ => None,
        }
    }
}

/// Glyph drawn in the unit square with the current fill colour. When
/// `outline` is true the shape is also stroked (icons).
fn append_check_glyph_unit(out: &mut String, glyph: CheckGlyph, outline: bool) {
    let paint = if outline { "B" } else { "f" };
    match glyph {
        CheckGlyph::Check => {
            polygon_path(
                out,
                &[
                    (0.0, 0.5),
                    (0.14, 0.64),
                    (0.36, 0.40),
                    (0.86, 0.96),
                    (1.0, 0.84),
                    (0.37, 0.06),
                ],
                true,
            );
        }
        CheckGlyph::Circle => ellipse_path(out, 0.5, 0.5, 0.5, 0.5),
        CheckGlyph::Cross => {
            polygon_path(
                out,
                &[
                    (0.0, 0.12),
                    (0.12, 0.0),
                    (0.5, 0.38),
                    (0.88, 0.0),
                    (1.0, 0.12),
                    (0.62, 0.5),
                    (1.0, 0.88),
                    (0.88, 1.0),
                    (0.5, 0.62),
                    (0.12, 1.0),
                    (0.0, 0.88),
                    (0.38, 0.5),
                ],
                true,
            );
        }
        CheckGlyph::Diamond => {
            polygon_path(out, &[(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)], true)
        }
        CheckGlyph::Square => out.push_str("0.05 0.05 0.9 0.9 re\n"),
        CheckGlyph::Star => {
            let mut points = Vec::with_capacity(10);
            for index in 0..10 {
                let radius = if index % 2 == 0 { 0.52 } else { 0.21 };
                let angle = std::f64::consts::FRAC_PI_2 + index as f64 * std::f64::consts::PI / 5.0;
                points.push((0.5 + radius * angle.cos(), 0.46 + radius * angle.sin()));
            }
            polygon_path(out, &points, true);
        }
    }
    out.push_str(paint);
    out.push('\n');
}

/// Draws a check-box glyph centred in a `width` x `height` box with side
/// `side`, filled with `color`.
pub(crate) fn append_check_glyph(
    out: &mut String,
    glyph: CheckGlyph,
    width: f64,
    height: f64,
    side: f64,
    color: AnnotColor,
) {
    let x = (width - side) * 0.5;
    let y = (height - side) * 0.5;
    let _ = writeln!(
        out,
        "q {} {} 0 0 {} {} {} cm",
        color.fill_op(),
        n(side),
        n(side),
        n(x),
        n(y)
    );
    append_check_glyph_unit(out, glyph, false);
    out.push_str("Q\n");
}

// ---------------------------------------------------------------------------
// Stamps
// ---------------------------------------------------------------------------

fn stamp_label(name: &str) -> Option<(&'static str, AnnotColor)> {
    const GREEN: AnnotColor = AnnotColor::Rgb(0.13, 0.52, 0.18);
    const RED: AnnotColor = AnnotColor::Rgb(0.78, 0.1, 0.1);
    const BLUE: AnnotColor = AnnotColor::Rgb(0.12, 0.27, 0.64);
    Some(match name {
        "Approved" => ("APPROVED", GREEN),
        "Experimental" => ("EXPERIMENTAL", BLUE),
        "NotApproved" => ("NOT APPROVED", RED),
        "AsIs" => ("AS IS", RED),
        "Expired" => ("EXPIRED", RED),
        "NotForPublicRelease" => ("NOT FOR PUBLIC RELEASE", RED),
        "Confidential" => ("CONFIDENTIAL", RED),
        "Final" => ("FINAL", GREEN),
        "Sold" => ("SOLD", BLUE),
        "Departmental" => ("DEPARTMENTAL", BLUE),
        "ForComment" => ("FOR COMMENT", BLUE),
        "TopSecret" => ("TOP SECRET", RED),
        "Draft" => ("DRAFT", RED),
        "ForPublicRelease" => ("FOR PUBLIC RELEASE", GREEN),
        _ => return None,
    })
}

fn stamp(
    annot: &PdfDictionary,
    reader: &PdfReader,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    let _ = reader;
    let name = annot.get_name("Name").unwrap_or("Draft");
    let (label, color) = match stamp_label(name) {
        Some((label, color)) => (label.to_string(), color),
        None => {
            // Custom stamp names: show the name itself in /C (or red).
            let color = match AnnotColor::from_object(annot.get("C")) {
                Some(Some(color)) => color,
                _ => AnnotColor::Rgb(0.78, 0.1, 0.1),
            };
            (split_camel_case(name).to_uppercase(), color)
        }
    };
    let width = rect[2] - rect[0];
    let height = rect[3] - rect[1];
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let short = width.min(height);
    let border = (short * 0.06).clamp(0.75, 6.0);
    let radius = short * 0.22;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "q {} {} {} w",
        color.stroke_op(),
        color.fill_op(),
        n(border)
    );
    rounded_rect_path(
        &mut out,
        border * 0.5,
        border * 0.5,
        width - border,
        height - border,
        radius,
    );
    out.push_str("S\n");
    let font = SimpleFont::standard("Helvetica-Bold");
    let units = font.measure_units(&label);
    let pad = border * 2.0 + short * 0.08;
    let mut size = ((height - 2.0 * pad) * 0.72).max(1.0);
    if units > 0.0 {
        size = size.min(((width - 2.0 * pad).max(1.0)) * 1000.0 / units);
    }
    let text_width = units * size / 1000.0;
    let x = (width - text_width) * 0.5;
    let y = (height - size * 0.72) * 0.5;
    let _ = writeln!(
        out,
        "BT /WFStamp {} Tf {} {} Td {} Tj ET Q",
        n(size),
        n(x),
        n(y),
        hex_string(&font.encode_lossy(&label))
    );
    let mut resources = Resources::default();
    resources.add_font(
        "WFStamp",
        PdfObject::Dictionary(standard_font_dict("Helvetica-Bold")),
    );
    finish(
        out,
        [0.0, 0.0, width, height],
        rect,
        resources,
        opacity(annot),
    )
}

fn split_camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_lower = false;
    for ch in name.chars() {
        if ch.is_uppercase() && prev_lower {
            out.push(' ');
        }
        prev_lower = ch.is_lowercase();
        out.push(if ch == '_' || ch == '#' { ' ' } else { ch });
    }
    out
}

// ---------------------------------------------------------------------------
// Line, PolyLine, Polygon, Ink
// ---------------------------------------------------------------------------

fn stroke_setup(out: &mut String, color: AnnotColor, border: &BorderStyle) {
    let _ = writeln!(
        out,
        "{} {} w {}",
        color.stroke_op(),
        n(border.width),
        border.dash_op()
    );
}

fn geometry_finish(
    annot: &PdfDictionary,
    out: String,
    bounds: Bounds,
) -> Option<SynthesizedAppearance> {
    // Content uses page coordinates; BBox and target rect are the same
    // page-space bounds, so the placement is the identity.
    let bbox = bounds.to_rect()?;
    finish(out, bbox, bbox, Resources::default(), opacity(annot))
}

fn line(
    annot: &PdfDictionary,
    reader: &PdfReader,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    let coords = number_array(reader, annot.get("L"))?;
    if coords.len() != 4 {
        return None;
    }
    let color = stroke_color(annot, Some(AnnotColor::BLACK))?;
    let border = border_style(annot, reader, 1.0);
    if border.width <= 0.0 {
        return None;
    }
    let interior = interior_color(annot);
    let (le_start, le_end) = line_endings(annot, reader);
    let mut p1 = (coords[0], coords[1]);
    let mut p2 = (coords[2], coords[3]);
    let dir = unit(p2.0 - p1.0, p2.1 - p1.1)?;
    let normal = (-dir.1, dir.0);
    let mut bounds = Bounds::new();
    bounds.add_rect(rect);
    let mut out = String::from("q 0 j 0 J\n");
    stroke_setup(&mut out, color, &border);

    // Leader lines (dimension style): offset the line along its normal.
    let ll = number(reader, annot.get("LL")).unwrap_or(0.0);
    if ll != 0.0 {
        let lle = number(reader, annot.get("LLE")).unwrap_or(0.0).max(0.0);
        let llo = number(reader, annot.get("LLO")).unwrap_or(0.0).max(0.0);
        let sign = ll.signum();
        for p in [p1, p2] {
            let a = (p.0 + normal.0 * sign * llo, p.1 + normal.1 * sign * llo);
            let b = (
                p.0 + normal.0 * (ll + sign * lle),
                p.1 + normal.1 * (ll + sign * lle),
            );
            let _ = writeln!(out, "{} {} m {} {} l S", n(a.0), n(a.1), n(b.0), n(b.1));
            bounds.add_point(a.0, a.1, border.width);
            bounds.add_point(b.0, b.1, border.width);
        }
        p1 = (p1.0 + normal.0 * ll, p1.1 + normal.1 * ll);
        p2 = (p2.0 + normal.0 * ll, p2.1 + normal.1 * ll);
    }

    let len = ((p2.0 - p1.0).powi(2) + (p2.1 - p1.1).powi(2)).sqrt();
    let size = ending_size(border.width, len);
    let s_in = le_start.inset(size).min(len * 0.5);
    let e_in = le_end.inset(size).min(len * 0.5);
    let a = (p1.0 + dir.0 * s_in, p1.1 + dir.1 * s_in);
    let b = (p2.0 - dir.0 * e_in, p2.1 - dir.1 * e_in);
    let _ = writeln!(out, "{} {} m {} {} l S", n(a.0), n(a.1), n(b.0), n(b.1));
    bounds.add_point(p1.0, p1.1, border.width);
    bounds.add_point(p2.0, p2.1, border.width);
    append_line_ending(
        &mut out,
        le_start,
        p1,
        (-dir.0, -dir.1),
        size,
        interior,
        &mut bounds,
    );
    append_line_ending(&mut out, le_end, p2, dir, size, interior, &mut bounds);
    out.push_str("Q\n");
    geometry_finish(annot, out, bounds)
}

fn poly(
    annot: &PdfDictionary,
    reader: &PdfReader,
    rect: [f64; 4],
    closed: bool,
) -> Option<SynthesizedAppearance> {
    let vertices = number_array(reader, annot.get("Vertices"))?;
    let points = points_from(&vertices);
    if points.len() < 2 {
        return None;
    }
    let color = stroke_color(annot, Some(AnnotColor::BLACK));
    let interior = if closed { interior_color(annot) } else { None };
    let border = border_style(annot, reader, 1.0);
    let stroke = color.filter(|_| border.width > 0.0);
    if stroke.is_none() && interior.is_none() {
        return None;
    }
    let mut bounds = Bounds::new();
    bounds.add_rect(rect);
    for p in &points {
        bounds.add_point(p.0, p.1, border.width);
    }
    let mut out = String::from("q 1 j 0 J\n");
    if let Some(color) = stroke {
        stroke_setup(&mut out, color, &border);
    }
    if let Some(fill) = interior {
        let _ = writeln!(out, "{}", fill.fill_op());
    }
    let cloudy = if closed {
        cloudy_intensity(annot, reader)
    } else {
        None
    };
    if let Some(intensity) = cloudy.filter(|_| points.len() >= 3) {
        let mut ring = points.clone();
        if signed_area(&ring) < 0.0 {
            ring.reverse();
        }
        let radius = cloud_radius(intensity);
        cloud_path(&mut out, &ring, radius);
        for p in &points {
            bounds.add_point(p.0, p.1, radius * 2.0 + border.width);
        }
    } else {
        polygon_path(&mut out, &points, closed);
    }
    let paint = match (stroke.is_some(), interior.is_some()) {
        (true, true) => "B",
        (true, false) => "S",
        (false, true) => "f",
        (false, false) => "n",
    };
    let _ = writeln!(out, "{paint}");
    if !closed && stroke.is_some() {
        let (le_start, le_end) = line_endings(annot, reader);
        let ending_fill = interior_color(annot);
        if let Some(dir) = unit(points[0].0 - points[1].0, points[0].1 - points[1].1) {
            let seg = distance(points[0], points[1]);
            append_line_ending(
                &mut out,
                le_start,
                points[0],
                dir,
                ending_size(border.width, seg),
                ending_fill,
                &mut bounds,
            );
        }
        let last = points[points.len() - 1];
        let prev = points[points.len() - 2];
        if let Some(dir) = unit(last.0 - prev.0, last.1 - prev.1) {
            append_line_ending(
                &mut out,
                le_end,
                last,
                dir,
                ending_size(border.width, distance(prev, last)),
                ending_fill,
                &mut bounds,
            );
        }
    }
    out.push_str("Q\n");
    geometry_finish(annot, out, bounds)
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

fn signed_area(points: &[(f64, f64)]) -> f64 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        area += a.0 * b.1 - b.0 * a.1;
    }
    area * 0.5
}

fn ink(annot: &PdfDictionary, reader: &PdfReader, rect: [f64; 4]) -> Option<SynthesizedAppearance> {
    let lists = resolve(reader, annot.get("InkList")?)?;
    let lists = lists.as_array()?;
    let color = stroke_color(annot, Some(AnnotColor::BLACK))?;
    let border = border_style(annot, reader, 1.0);
    if border.width <= 0.0 {
        return None;
    }
    let mut bounds = Bounds::new();
    bounds.add_rect(rect);
    let mut out = String::from("q 1 j 1 J\n");
    stroke_setup(&mut out, color, &border);
    let mut wrote = false;
    for list in lists {
        let Some(values) = number_array(reader, Some(list)) else {
            continue;
        };
        let points = points_from(&values);
        if points.is_empty() {
            continue;
        }
        for p in &points {
            bounds.add_point(p.0, p.1, border.width);
        }
        append_smooth_polyline(&mut out, &points);
        out.push_str("S\n");
        wrote = true;
    }
    if !wrote {
        return None;
    }
    out.push_str("Q\n");
    geometry_finish(annot, out, bounds)
}

/// Smooths a freehand stroke with quadratic segments through the midpoints
/// of consecutive samples (expressed as cubics). A single point becomes a
/// dot via the round cap.
fn append_smooth_polyline(out: &mut String, points: &[(f64, f64)]) {
    let first = points[0];
    let _ = write!(out, "{} {} m ", n(first.0), n(first.1));
    if points.len() == 1 {
        let _ = write!(out, "{} {} l ", n(first.0), n(first.1));
        return;
    }
    if points.len() == 2 {
        let _ = write!(out, "{} {} l ", n(points[1].0), n(points[1].1));
        return;
    }
    let mid = |a: (f64, f64), b: (f64, f64)| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
    let mut current = mid(points[0], points[1]);
    let _ = write!(out, "{} {} l ", n(current.0), n(current.1));
    for index in 1..points.len() - 1 {
        let control = points[index];
        let end = mid(points[index], points[index + 1]);
        let c1 = (
            current.0 + (control.0 - current.0) * 2.0 / 3.0,
            current.1 + (control.1 - current.1) * 2.0 / 3.0,
        );
        let c2 = (
            end.0 + (control.0 - end.0) * 2.0 / 3.0,
            end.1 + (control.1 - end.1) * 2.0 / 3.0,
        );
        let _ = write!(
            out,
            "{} {} {} {} {} {} c ",
            n(c1.0),
            n(c1.1),
            n(c2.0),
            n(c2.1),
            n(end.0),
            n(end.1)
        );
        current = end;
    }
    let last = points[points.len() - 1];
    let _ = write!(out, "{} {} l ", n(last.0), n(last.1));
}

// ---------------------------------------------------------------------------
// Square / Circle / Caret
// ---------------------------------------------------------------------------

fn square_circle(
    annot: &PdfDictionary,
    reader: &PdfReader,
    rect: [f64; 4],
    circle: bool,
) -> Option<SynthesizedAppearance> {
    let width = rect[2] - rect[0];
    let height = rect[3] - rect[1];
    let color = stroke_color(annot, Some(AnnotColor::BLACK));
    let interior = interior_color(annot);
    let border = border_style(annot, reader, 1.0);
    let stroke = color.filter(|_| border.width > 0.0);
    if stroke.is_none() && interior.is_none() {
        return None;
    }
    let rd = rect_differences(annot, reader, width, height);
    let cloudy = cloudy_intensity(annot, reader);
    let inset = border.width * 0.5 + cloudy.map(cloud_radius).unwrap_or(0.0);
    let x0 = rd[0] + inset;
    let y0 = rd[1] + inset;
    let x1 = width - rd[2] - inset;
    let y1 = height - rd[3] - inset;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut out = String::from("q 1 j\n");
    if let Some(color) = stroke {
        stroke_setup(&mut out, color, &border);
    }
    if let Some(fill) = interior {
        let _ = writeln!(out, "{}", fill.fill_op());
    }
    if let Some(intensity) = cloudy {
        let radius = cloud_radius(intensity);
        let ring = if circle {
            let (cx, cy) = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
            let (rx, ry) = ((x1 - x0) * 0.5, (y1 - y0) * 0.5);
            let perimeter = std::f64::consts::PI
                * (3.0 * (rx + ry) - ((3.0 * rx + ry) * (rx + 3.0 * ry)).sqrt());
            let count = ((perimeter / (2.0 * radius)).round() as usize).clamp(6, 512);
            (0..count)
                .map(|i| {
                    let t = i as f64 / count as f64 * std::f64::consts::TAU;
                    (cx + rx * t.cos(), cy + ry * t.sin())
                })
                .collect::<Vec<_>>()
        } else {
            vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
        };
        cloud_path(&mut out, &ring, radius);
    } else if circle {
        ellipse_path(
            &mut out,
            (x0 + x1) * 0.5,
            (y0 + y1) * 0.5,
            (x1 - x0) * 0.5,
            (y1 - y0) * 0.5,
        );
    } else {
        let _ = writeln!(out, "{} {} {} {} re", n(x0), n(y0), n(x1 - x0), n(y1 - y0));
    }
    let paint = match (stroke.is_some(), interior.is_some()) {
        (true, true) => "B",
        (true, false) => "S",
        _ => "f",
    };
    let _ = writeln!(out, "{paint}\nQ");
    finish(
        out,
        [0.0, 0.0, width, height],
        rect,
        Resources::default(),
        opacity(annot),
    )
}

fn caret(
    annot: &PdfDictionary,
    reader: &PdfReader,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    let width = rect[2] - rect[0];
    let height = rect[3] - rect[1];
    let color = stroke_color(annot, Some(AnnotColor::BLACK))?;
    let rd = rect_differences(annot, reader, width, height);
    let (x0, y0) = (rd[0], rd[1]);
    let (x1, y1) = (width - rd[2], height - rd[3]);
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let mut out = String::new();
    let _ = writeln!(out, "q {}", color.fill_op());
    // A filled caret: two curved flanks meeting at the apex and a notched
    // base.
    let apex = (x0 + w * 0.5, y1);
    let notch = (x0 + w * 0.5, y0 + h * 0.25);
    let _ = writeln!(
        out,
        "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h f",
        n(x0),
        n(y0),
        n(x0 + w * 0.3),
        n(y0 + h * 0.1),
        n(apex.0 - w * 0.06),
        n(y0 + h * 0.6),
        n(apex.0),
        n(apex.1),
        n(apex.0 + w * 0.06),
        n(y0 + h * 0.6),
        n(x1 - w * 0.3),
        n(y0 + h * 0.1),
        n(x1),
        n(y0),
        n(x1 - w * 0.2),
        n(y0 + h * 0.08),
        n(notch.0 + w * 0.08),
        n(notch.1 - h * 0.03),
        n(notch.0),
        n(notch.1),
        n(notch.0 - w * 0.08),
        n(notch.1 - h * 0.03),
        n(x0 + w * 0.2),
        n(y0 + h * 0.08),
        n(x0),
        n(y0)
    );
    out.push_str("Q\n");
    finish(
        out,
        [0.0, 0.0, width, height],
        rect,
        Resources::default(),
        opacity(annot),
    )
}

// ---------------------------------------------------------------------------
// Default appearance strings and fonts
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DaSpec {
    pub font_name: Option<String>,
    pub font_size: f64,
    pub color: AnnotColor,
}

impl Default for DaSpec {
    fn default() -> Self {
        Self {
            font_name: None,
            font_size: 0.0,
            color: AnnotColor::BLACK,
        }
    }
}

/// Lenient `/DA` parse: the last Tf and colour operator win; malformed
/// operators are ignored.
pub(crate) fn parse_da(bytes: &[u8]) -> DaSpec {
    let mut da = DaSpec::default();
    let Ok(operations) = crate::content::ContentParser::parse(bytes) else {
        return da;
    };
    for op in operations {
        let numbers: Option<Vec<f64>> = op
            .operands
            .iter()
            .map(|operand| operand.as_number().filter(|v| v.is_finite()))
            .collect();
        match op.operator.as_str() {
            "Tf" => {
                if let Some(name) = op.name(0).filter(|name| !name.is_empty()) {
                    da.font_name = Some(name.to_string());
                }
                if let Some(size) = op.number(1).filter(|v| v.is_finite() && *v >= 0.0) {
                    da.font_size = size;
                }
            }
            "g" => {
                if let Some([g]) = numbers.as_deref() {
                    da.color = AnnotColor::Gray(g.clamp(0.0, 1.0));
                }
            }
            "rg" => {
                if let Some([r, g, b]) = numbers.as_deref() {
                    da.color =
                        AnnotColor::Rgb(r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
                }
            }
            "k" => {
                if let Some([c, m, y, k]) = numbers.as_deref() {
                    da.color = AnnotColor::Cmyk(
                        c.clamp(0.0, 1.0),
                        m.clamp(0.0, 1.0),
                        y.clamp(0.0, 1.0),
                        k.clamp(0.0, 1.0),
                    );
                }
            }
            _ => {}
        }
    }
    da
}

/// A single-byte font usable for text layout: reverse encoding map and
/// widths in 1/1000 em.
struct SimpleFont {
    resource_name: String,
    object: PdfObject,
    encode: BTreeMap<char, u8>,
    widths: Vec<f64>,
    ascent: f64,
    descent: f64,
}

impl SimpleFont {
    fn from_dict(
        resource_name: &str,
        object: PdfObject,
        dict: &PdfDictionary,
        reader: Option<&PdfReader>,
    ) -> Self {
        let resolver = match reader {
            Some(reader) => crate::fonts::resolver::FontResolver::new(dict, reader),
            None => crate::fonts::resolver::FontResolver::new_from_dict_only(dict),
        };
        let mut encode = BTreeMap::new();
        let mut widths = vec![0.0; 256];
        for code in (0u16..=255).rev() {
            let decoded = resolver.decode_char(code);
            let mut chars = decoded.chars();
            if let (Some(ch), None) = (chars.next(), chars.next()) {
                if ch != '\u{FFFD}' && (code >= 32 || ch == ' ') {
                    encode.insert(ch, code as u8);
                }
            }
            widths[code as usize] = resolver.glyph_width(code);
        }
        // A non-breaking space may be shown as a space.
        if let Some(code) = encode.get(&' ').copied() {
            encode.entry('\u{a0}').or_insert(code);
        }
        let (ascent, descent) = font_vertical_metrics(dict, reader);
        Self {
            resource_name: resource_name.to_string(),
            object,
            encode,
            widths,
            ascent,
            descent,
        }
    }

    fn standard(base: &str) -> Self {
        Self::standard_named(base, "WFHelv")
    }

    fn standard_named(base: &str, resource_name: &str) -> Self {
        let dict = standard_font_dict(base);
        Self::from_dict(
            resource_name,
            PdfObject::Dictionary(dict.clone()),
            &dict,
            None,
        )
    }

    fn code_width(&self, code: u8) -> f64 {
        let width = self.widths[code as usize];
        if width > 0.0 && width.is_finite() {
            width
        } else if code == b' ' {
            250.0
        } else {
            500.0
        }
    }

    fn measure_units(&self, text: &str) -> f64 {
        text.chars()
            .map(|ch| {
                self.encode
                    .get(&ch)
                    .map(|code| self.code_width(*code))
                    .unwrap_or(500.0)
            })
            .sum()
    }

    fn encode_lossy(&self, text: &str) -> Vec<u8> {
        text.chars()
            .map(|ch| self.encode.get(&ch).copied().unwrap_or(b'?'))
            .collect()
    }
}

/// A composite (`/Type0`) field font from `/DR`: characters are encoded
/// through the font's own `/ToUnicode` (or predefined Unicode CMap) in
/// reverse, looked up on demand and cached.
struct CompositeFont {
    resource_name: String,
    object: PdfObject,
    resolver: crate::fonts::resolver::FontResolver,
    ascent: f64,
    descent: f64,
    cache: std::cell::RefCell<BTreeMap<char, Option<(Vec<u8>, f64)>>>,
}

impl CompositeFont {
    fn from_dict(
        resource_name: &str,
        object: PdfObject,
        dict: &PdfDictionary,
        reader: &PdfReader,
    ) -> Option<Self> {
        let resolver = crate::fonts::resolver::FontResolver::new(dict, reader);
        // Field text is laid out horizontally.
        if resolver.is_vertical() || resolver.validate_encoding().is_err() {
            return None;
        }
        let descendant = crate::fonts::resolver::get_descendant_font(dict, reader);
        let (ascent, descent) = match descendant.as_ref() {
            Some(descendant) => font_vertical_metrics(descendant, Some(reader)),
            None => font_vertical_metrics(dict, Some(reader)),
        };
        Some(Self {
            resource_name: resource_name.to_string(),
            object,
            resolver,
            ascent,
            descent,
            cache: std::cell::RefCell::new(BTreeMap::new()),
        })
    }

    /// Code bytes and width (1/1000 em) of `ch`, `None` when the font has
    /// no code for it.
    fn lookup(&self, ch: char) -> Option<(Vec<u8>, f64)> {
        if let Some(hit) = self.cache.borrow().get(&ch) {
            return hit.clone();
        }
        let found = self
            .resolver
            .encoding_candidates(ch.encode_utf8(&mut [0; 4]))
            .ok()
            .and_then(|codes| codes.into_iter().next())
            .map(|code| {
                let width = self.resolver.width_for_code(code);
                let width = if width > 0.0 && width.is_finite() {
                    width
                } else {
                    1000.0
                };
                (code.bytes(), width)
            });
        self.cache.borrow_mut().insert(ch, found.clone());
        found
    }
}

/// The field's own font: a simple font or a composite font.
enum PrimaryFont {
    Simple(SimpleFont),
    Composite(CompositeFont),
}

impl PrimaryFont {
    fn resource_name(&self) -> &str {
        match self {
            PrimaryFont::Simple(font) => &font.resource_name,
            PrimaryFont::Composite(font) => &font.resource_name,
        }
    }

    fn object(&self) -> &PdfObject {
        match self {
            PrimaryFont::Simple(font) => &font.object,
            PrimaryFont::Composite(font) => &font.object,
        }
    }

    fn ascent(&self) -> f64 {
        match self {
            PrimaryFont::Simple(font) => font.ascent,
            PrimaryFont::Composite(font) => font.ascent,
        }
    }

    fn descent(&self) -> f64 {
        match self {
            PrimaryFont::Simple(font) => font.descent,
            PrimaryFont::Composite(font) => font.descent,
        }
    }

    /// Width in 1/1000 em of `ch`, `None` when the font cannot encode it.
    fn char_width(&self, ch: char) -> Option<f64> {
        match self {
            PrimaryFont::Simple(font) => font.encode.get(&ch).map(|code| font.code_width(*code)),
            PrimaryFont::Composite(font) => font.lookup(ch).map(|(_, width)| width),
        }
    }

    /// Encoded bytes of `text` (characters without a code become `?`).
    fn encode_lossy(&self, text: &str) -> Vec<u8> {
        match self {
            PrimaryFont::Simple(font) => text
                .chars()
                .map(|ch| font.encode.get(&ch).copied().unwrap_or(b'?'))
                .collect(),
            PrimaryFont::Composite(font) => {
                let mut bytes = Vec::new();
                for ch in text.chars() {
                    if let Some((code, _)) = font.lookup(ch).or_else(|| font.lookup('?')) {
                        bytes.extend_from_slice(&code);
                    }
                }
                bytes
            }
        }
    }
}

fn font_vertical_metrics(dict: &PdfDictionary, reader: Option<&PdfReader>) -> (f64, f64) {
    let descriptor = match (dict.get("FontDescriptor"), reader) {
        (Some(PdfObject::Dictionary(d)), _) => Some(d.clone()),
        (Some(obj), Some(reader)) => resolve_dict(reader, Some(obj)),
        _ => None,
    };
    let ascent = descriptor
        .as_ref()
        .and_then(|d| d.get("Ascent").and_then(PdfObject::as_number))
        .filter(|v| v.is_finite() && *v > 100.0 && *v < 2000.0)
        .unwrap_or(718.0);
    let descent = descriptor
        .as_ref()
        .and_then(|d| d.get("Descent").and_then(PdfObject::as_number))
        .filter(|v| v.is_finite() && *v < 0.0 && *v > -1000.0)
        .unwrap_or(-207.0);
    (ascent / 1000.0, descent / 1000.0)
}

/// Unicode fallback: a TrueType/OpenType program embedded as an Identity-H
/// composite font so values outside the field font's encoding still
/// render. The bundled DejaVu Sans comes first; installed system faces
/// cover what DejaVu lacks (CJK, Indic, Thai...).
struct UnicodeFont {
    face: Option<ttf_parser::Face<'static>>,
    bytes: &'static [u8],
    base_name: String,
    resource: String,
    /// CFF outlines: embedded as `FontFile3 /OpenType` under a
    /// `CIDFontType0`, with CIDs from a CID-keyed CFF charset.
    cff: bool,
    upem: f64,
    used: BTreeMap<u16, (char, f64)>,
}

const UNICODE_FONT_RESOURCE: &str = "WFUni";

/// At most this many installed fallback faces per text block.
const MAX_SYSTEM_FALLBACK_FONTS: usize = 4;

impl UnicodeFont {
    fn new(prefix: &str) -> Self {
        Self::from_program(
            crate::render::font_rasterizer::fallback_fonts::DEJAVU_SANS,
            "DejaVuSans",
            prefix.to_string(),
        )
    }

    fn from_program(bytes: &'static [u8], base_name: &str, resource: String) -> Self {
        let face = ttf_parser::Face::parse(bytes, 0).ok();
        let upem = face
            .as_ref()
            .map(|face| f64::from(face.units_per_em()))
            .filter(|v| *v > 0.0)
            .unwrap_or(2048.0);
        let cff = bytes.starts_with(b"OTTO");
        let base_name: String = base_name
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
            .collect();
        Self {
            face,
            bytes,
            base_name: if base_name.is_empty() {
                "WFFallback".to_string()
            } else {
                base_name
            },
            resource,
            cff,
            upem,
            used: BTreeMap::new(),
        }
    }

    /// An installed face that draws `ch`, as fallback number `index`.
    fn installed_for(ch: char, index: usize, prefix: &str) -> Option<Self> {
        if ch.is_whitespace() || ch.is_control() {
            return None;
        }
        let program = crate::fonts::installed::installed_font_covering(
            ch.encode_utf8(&mut [0; 4]),
            &crate::fonts::installed::InstalledFontStyle::default(),
        )?;
        let font = Self::from_program(
            program.bytes,
            &program.face.post_script_name,
            format!("{prefix}{}", index + 1),
        );
        font.has_glyph(ch).then_some(font)
    }

    fn has_glyph(&self, ch: char) -> bool {
        self.face
            .as_ref()
            .and_then(|face| face.glyph_index(ch))
            .is_some_and(|gid| gid.0 != 0)
    }

    /// Code (glyph id, or CID for CID-keyed CFF) and advance of `ch`.
    fn glyph(&self, ch: char) -> (u16, f64) {
        let Some(face) = self.face.as_ref() else {
            return (0, 500.0);
        };
        let gid = face.glyph_index(ch).map(|g| g.0).unwrap_or(0);
        let advance = face
            .glyph_hor_advance(ttf_parser::GlyphId(gid))
            .map(|a| f64::from(a) * 1000.0 / self.upem)
            .unwrap_or(500.0);
        let code = if self.cff {
            face.tables()
                .cff
                .and_then(|cff| cff.glyph_cid(ttf_parser::GlyphId(gid)))
                .unwrap_or(gid)
        } else {
            gid
        };
        (code, advance)
    }

    fn measure_char(&self, ch: char) -> f64 {
        self.glyph(ch).1
    }

    fn encode(&mut self, text: &str) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(text.len() * 2);
        for ch in text.chars() {
            let (code, advance) = self.glyph(ch);
            self.used.entry(code).or_insert((ch, advance));
            bytes.extend_from_slice(&code.to_be_bytes());
        }
        bytes
    }

    /// Font descriptor metrics in 1/1000 em: bbox, ascent, descent, cap
    /// height.
    fn metrics(&self) -> ([i64; 4], i64, i64, i64) {
        let Some(face) = self.face.as_ref() else {
            return ([-1021, -463, 1793, 1232], 928, -236, 729);
        };
        let scale = |v: i16| (f64::from(v) * 1000.0 / self.upem).round() as i64;
        let bbox = face.global_bounding_box();
        (
            [
                scale(bbox.x_min),
                scale(bbox.y_min),
                scale(bbox.x_max),
                scale(bbox.y_max),
            ],
            scale(face.ascender()),
            scale(face.descender()),
            face.capital_height()
                .map(scale)
                .unwrap_or_else(|| scale(face.ascender())),
        )
    }

    fn font_object(&self) -> PdfObject {
        let bytes = self.bytes;
        let mut file_dict = PdfDictionary::empty();
        file_dict.insert("Length", PdfObject::Integer(bytes.len() as i64));
        if self.cff {
            file_dict.insert("Subtype", PdfObject::Name("OpenType".into()));
        } else {
            file_dict.insert("Length1", PdfObject::Integer(bytes.len() as i64));
        }
        let (bbox, ascent, descent, cap_height) = self.metrics();
        let mut descriptor = PdfDictionary::empty();
        descriptor.insert("Type", PdfObject::Name("FontDescriptor".into()));
        descriptor.insert("FontName", PdfObject::Name(self.base_name.clone()));
        descriptor.insert("Flags", PdfObject::Integer(32));
        descriptor.insert(
            "FontBBox",
            PdfObject::Array(bbox.iter().map(|v| PdfObject::Integer(*v)).collect()),
        );
        descriptor.insert("ItalicAngle", PdfObject::Integer(0));
        descriptor.insert("Ascent", PdfObject::Integer(ascent));
        descriptor.insert("Descent", PdfObject::Integer(descent));
        descriptor.insert("CapHeight", PdfObject::Integer(cap_height));
        descriptor.insert("StemV", PdfObject::Integer(80));
        descriptor.insert(
            if self.cff { "FontFile3" } else { "FontFile2" },
            PdfObject::Stream {
                dict: file_dict,
                raw: bytes.to_vec(),
            },
        );
        let mut widths = Vec::new();
        for (code, (_, advance)) in &self.used {
            widths.push(PdfObject::Integer(i64::from(*code)));
            widths.push(PdfObject::Array(vec![PdfObject::Real(advance.round())]));
        }
        let mut system_info = PdfDictionary::empty();
        system_info.insert("Registry", PdfObject::String(b"Adobe".to_vec()));
        system_info.insert("Ordering", PdfObject::String(b"Identity".to_vec()));
        system_info.insert("Supplement", PdfObject::Integer(0));
        let mut cid_font = PdfDictionary::empty();
        cid_font.insert("Type", PdfObject::Name("Font".into()));
        cid_font.insert(
            "Subtype",
            PdfObject::Name(
                if self.cff {
                    "CIDFontType0"
                } else {
                    "CIDFontType2"
                }
                .into(),
            ),
        );
        cid_font.insert("BaseFont", PdfObject::Name(self.base_name.clone()));
        cid_font.insert("CIDSystemInfo", PdfObject::Dictionary(system_info));
        cid_font.insert("FontDescriptor", PdfObject::Dictionary(descriptor));
        if !self.cff {
            cid_font.insert("CIDToGIDMap", PdfObject::Name("Identity".into()));
        }
        cid_font.insert("DW", PdfObject::Integer(1000));
        cid_font.insert("W", PdfObject::Array(widths));
        let cmap = self.to_unicode_cmap();
        let mut cmap_dict = PdfDictionary::empty();
        cmap_dict.insert("Length", PdfObject::Integer(cmap.len() as i64));
        let mut font = PdfDictionary::empty();
        font.insert("Type", PdfObject::Name("Font".into()));
        font.insert("Subtype", PdfObject::Name("Type0".into()));
        font.insert("BaseFont", PdfObject::Name(self.base_name.clone()));
        font.insert("Encoding", PdfObject::Name("Identity-H".into()));
        font.insert(
            "DescendantFonts",
            PdfObject::Array(vec![PdfObject::Dictionary(cid_font)]),
        );
        font.insert(
            "ToUnicode",
            PdfObject::Stream {
                dict: cmap_dict,
                raw: cmap.into_bytes(),
            },
        );
        PdfObject::Dictionary(font)
    }

    fn to_unicode_cmap(&self) -> String {
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
             1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        let entries: Vec<_> = self.used.iter().filter(|(gid, _)| **gid != 0).collect();
        for chunk in entries.chunks(100) {
            let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
            for (gid, (ch, _)) in chunk {
                let mut units = [0u16; 2];
                let encoded = ch.encode_utf16(&mut units);
                let hex: String = encoded.iter().map(|u| format!("{u:04X}")).collect();
                let _ = writeln!(cmap, "<{:04X}> <{hex}>", gid);
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        cmap
    }
}

/// Which font draws a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontSlot {
    Primary,
    /// The bundled DejaVu Sans.
    Unicode,
    /// Installed fallback face number `n`.
    System(usize),
}

/// Field/annotation text font: the `/DA` font when it is a usable simple
/// or composite font, otherwise Helvetica, plus Unicode fallbacks (DejaVu
/// Sans, then installed faces) for characters the primary font cannot
/// encode — PDFium's per-character font switching.
pub(crate) struct TextFonts {
    primary: PrimaryFont,
    unicode: Option<UnicodeFont>,
    system: Vec<UnicodeFont>,
    /// Resource name of the DejaVu fallback; installed fallbacks append 1, 2...
    fallback_prefix: String,
}

impl TextFonts {
    /// Resolves `font_name` from the `/DR` dictionaries in priority order.
    pub(crate) fn resolve(
        font_name: Option<&str>,
        dr_dicts: &[PdfDictionary],
        reader: &PdfReader,
    ) -> Self {
        let primary = font_name
            .and_then(|name| {
                dr_dicts.iter().find_map(|dr| {
                    let fonts = resolve_dict(reader, dr.get("Font"))?;
                    let font_obj = fonts.get(name)?.clone();
                    let dict = match resolve(reader, &font_obj)? {
                        PdfObject::Dictionary(dict) => dict,
                        _ => return None,
                    };
                    if dict.get_name("BaseFont").is_some_and(is_symbolic_base_font) {
                        return None;
                    }
                    let resource_name = resource_safe_name(name);
                    match dict.get_name("Subtype") {
                        Some("Type1") | Some("TrueType") | Some("MMType1") => {
                            Some(PrimaryFont::Simple(SimpleFont::from_dict(
                                &resource_name,
                                font_obj,
                                &dict,
                                Some(reader),
                            )))
                        }
                        Some("Type0") => {
                            CompositeFont::from_dict(&resource_name, font_obj, &dict, reader)
                                .map(PrimaryFont::Composite)
                        }
                        _ => None,
                    }
                })
            })
            .unwrap_or_else(|| {
                PrimaryFont::Simple(SimpleFont::standard(standard_base_for(font_name)))
            });
        Self {
            primary,
            unicode: None,
            system: Vec::new(),
            fallback_prefix: UNICODE_FONT_RESOURCE.to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn helvetica() -> Self {
        Self {
            primary: PrimaryFont::Simple(SimpleFont::standard("Helvetica")),
            unicode: None,
            system: Vec::new(),
            fallback_prefix: UNICODE_FONT_RESOURCE.to_string(),
        }
    }

    /// A standard-14 font (`base`, WinAnsi) under the resource name
    /// `resource`, with its Unicode fallbacks named `<fallback_prefix>`,
    /// `<fallback_prefix>1`... so several text fonts can share one
    /// resource dictionary (XFA page painting).
    pub(crate) fn standard_named(base: &str, resource: &str, fallback_prefix: &str) -> Self {
        Self {
            primary: PrimaryFont::Simple(SimpleFont::standard_named(base, resource)),
            unicode: None,
            system: Vec::new(),
            fallback_prefix: fallback_prefix.to_string(),
        }
    }

    /// A document font (`/Type1` or `/TrueType`, usually an indirect
    /// reference in `object`) under the resource name `resource`; `None`
    /// for symbolic or non-simple fonts.
    pub(crate) fn simple_document_font(
        object: PdfObject,
        dict: &PdfDictionary,
        reader: &PdfReader,
        resource: &str,
        fallback_prefix: &str,
    ) -> Option<Self> {
        if dict.get_name("BaseFont").is_some_and(is_symbolic_base_font) {
            return None;
        }
        if !matches!(dict.get_name("Subtype"), Some("Type1" | "TrueType")) {
            return None;
        }
        let font = SimpleFont::from_dict(resource, object, dict, Some(reader));
        // A font that cannot encode basic Latin letters is unusable here.
        if !font.encode.contains_key(&'a') || !font.encode.contains_key(&'0') {
            return None;
        }
        Some(Self {
            primary: PrimaryFont::Simple(font),
            unicode: None,
            system: Vec::new(),
            fallback_prefix: fallback_prefix.to_string(),
        })
    }

    fn unicode(&mut self) -> &mut UnicodeFont {
        let prefix = &self.fallback_prefix;
        self.unicode.get_or_insert_with(|| UnicodeFont::new(prefix))
    }

    /// The font that draws `ch`: the primary font when it encodes it, else
    /// DejaVu Sans when it has a glyph, else an installed face covering it
    /// (loaded on first use), else DejaVu Sans (its missing glyph).
    fn slot(&mut self, ch: char) -> FontSlot {
        if self.primary.char_width(ch).is_some() {
            return FontSlot::Primary;
        }
        if self.unicode().has_glyph(ch) || ch.is_whitespace() || ch.is_control() {
            return FontSlot::Unicode;
        }
        if let Some(index) = self.system.iter().position(|font| font.has_glyph(ch)) {
            return FontSlot::System(index);
        }
        if self.system.len() < MAX_SYSTEM_FALLBACK_FONTS {
            if let Some(font) =
                UnicodeFont::installed_for(ch, self.system.len(), &self.fallback_prefix)
            {
                self.system.push(font);
                return FontSlot::System(self.system.len() - 1);
            }
        }
        FontSlot::Unicode
    }

    fn slot_width(&mut self, slot: FontSlot, ch: char) -> f64 {
        match slot {
            FontSlot::Primary => self.primary.char_width(ch).unwrap_or(500.0),
            FontSlot::Unicode => self.unicode().measure_char(ch),
            FontSlot::System(index) => self.system[index].measure_char(ch),
        }
    }

    /// Width of `text` at `size` points.
    pub(crate) fn measure(&mut self, text: &str, size: f64) -> f64 {
        let mut units = 0.0;
        for ch in text.chars() {
            let slot = self.slot(ch);
            units += self.slot_width(slot, ch);
        }
        units * size / 1000.0
    }

    pub(crate) fn ascent(&self) -> f64 {
        self.primary.ascent()
    }

    pub(crate) fn descent(&self) -> f64 {
        self.primary.descent()
    }

    /// Emits text-showing operators (inside BT/ET, after Td) for `text`,
    /// switching fonts for characters the primary font cannot encode.
    pub(crate) fn show(&mut self, out: &mut String, text: &str, size: f64) {
        let mut run = String::new();
        let mut run_slot = FontSlot::Primary;
        for ch in text.chars() {
            let slot = self.slot(ch);
            if !run.is_empty() && slot != run_slot {
                let taken = std::mem::take(&mut run);
                self.flush_run(out, &taken, run_slot, size);
            }
            run_slot = slot;
            run.push(ch);
        }
        self.flush_run(out, &run, run_slot, size);
    }

    fn flush_run(&mut self, out: &mut String, run: &str, slot: FontSlot, size: f64) {
        if run.is_empty() {
            return;
        }
        let (resource, bytes) = match slot {
            FontSlot::Primary => (
                self.primary.resource_name().to_string(),
                self.primary.encode_lossy(run),
            ),
            FontSlot::Unicode => {
                let font = self.unicode();
                (font.resource.clone(), font.encode(run))
            }
            FontSlot::System(index) => {
                let font = &mut self.system[index];
                (font.resource.clone(), font.encode(run))
            }
        };
        let _ = writeln!(out, "/{resource} {} Tf {} Tj", n(size), hex_string(&bytes));
    }

    pub(crate) fn add_to(&self, resources: &mut Resources) {
        resources.add_font(self.primary.resource_name(), self.primary.object().clone());
        for font in self
            .unicode
            .iter()
            .chain(self.system.iter())
            .filter(|font| !font.used.is_empty())
        {
            resources.add_font(&font.resource, font.font_object());
        }
    }
}

fn is_symbolic_base_font(base: &str) -> bool {
    let lower = base.to_ascii_lowercase();
    lower.contains("dingbat") || lower.contains("symbol") || lower.contains("wingding")
}

fn standard_base_for(font_name: Option<&str>) -> &'static str {
    let lower = font_name.unwrap_or("").to_ascii_lowercase();
    // Acrobat's conventional names: Helv/HeBo, Cour/CoBo, TiRo/TiBo.
    let bold = lower.contains("bo");
    match (lower.get(..2), bold) {
        (Some("co"), true) => "Courier-Bold",
        (Some("co"), false) => "Courier",
        (Some("ti"), true) => "Times-Bold",
        (Some("ti"), false) => "Times-Roman",
        (Some("he"), true) => "Helvetica-Bold",
        _ => "Helvetica",
    }
}

fn resource_safe_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .filter(|ch| ch.is_ascii_graphic() && !"()<>[]{}/%#".contains(*ch))
        .collect();
    if safe.is_empty() || safe.starts_with(UNICODE_FONT_RESOURCE) {
        "WFFont".to_string()
    } else {
        safe
    }
}

fn hex_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + 2);
    out.push('<');
    for byte in bytes {
        let _ = write!(out, "{byte:02X}");
    }
    out.push('>');
    out
}

// ---------------------------------------------------------------------------
// Text layout
// ---------------------------------------------------------------------------

/// Greedy word wrap honouring explicit line breaks; words wider than the
/// line are broken between characters.
pub(crate) fn wrap_lines(
    fonts: &mut TextFonts,
    text: &str,
    size: f64,
    max_width: f64,
) -> Vec<String> {
    let mut lines = Vec::new();
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    for paragraph in normalized.split('\n') {
        let mut current = String::new();
        let mut current_width = 0.0;
        for word in split_keep_spaces(paragraph) {
            let word_width = fonts.measure(&word, size);
            if current_width + word_width <= max_width
                || current.is_empty() && word.trim().is_empty()
            {
                current.push_str(&word);
                current_width += word_width;
                continue;
            }
            if word.trim().is_empty() {
                // Break at the space; it is not carried to the next line.
                lines.push(std::mem::take(&mut current));
                current_width = 0.0;
                continue;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current).trim_end().to_string());
                current_width = 0.0;
            }
            if word_width <= max_width {
                current.push_str(&word);
                current_width = word_width;
                continue;
            }
            for ch in word.chars() {
                let cw = fonts.measure(ch.encode_utf8(&mut [0; 4]), size);
                if current_width + cw > max_width && !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    current_width = 0.0;
                }
                current.push(ch);
                current_width += cw;
            }
        }
        lines.push(current.trim_end().to_string());
    }
    lines
}

fn split_keep_spaces(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_space = false;
    for ch in text.chars() {
        let space = ch == ' ' || ch == '\t';
        if space != in_space && !current.is_empty() {
            parts.push(std::mem::take(&mut current));
        }
        in_space = space;
        current.push(if ch == '\t' { ' ' } else { ch });
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn aligned_x(alignment: i64, line_width: f64, left: f64, available: f64) -> f64 {
    match alignment {
        1 => left + (available - line_width) * 0.5,
        2 => left + available - line_width,
        _ => left,
    }
}

/// Lays out `text` in the box `[left, bottom, left + width, bottom + height]`
/// top-down with word wrap. Returns the content (without BT/ET wrapping).
fn append_wrapped_text(
    out: &mut String,
    fonts: &mut TextFonts,
    text: &str,
    size: f64,
    color: AnnotColor,
    alignment: i64,
    area: [f64; 4],
) {
    let [left, bottom, width, height] = area;
    let line_height = size * 1.15;
    let lines = wrap_lines(fonts, text, size, width);
    let mut baseline = bottom + height - size * fonts.ascent().max(0.75);
    out.push_str("BT\n");
    let _ = writeln!(out, "{}", color.fill_op());
    let mut last = (0.0, 0.0);
    for line in lines {
        if baseline < bottom - size * 0.5 {
            break;
        }
        let line_width = fonts.measure(&line, size);
        let x = aligned_x(alignment, line_width, left, width);
        let _ = writeln!(out, "{} {} Td", n(x - last.0), n(baseline - last.1));
        last = (x, baseline);
        fonts.show(out, &line, size);
        baseline -= line_height;
    }
    out.push_str("ET\n");
}

/// Largest size (<= 12pt, >= 4pt) at which the wrapped text fits.
fn auto_size_multiline(fonts: &mut TextFonts, text: &str, width: f64, height: f64) -> f64 {
    let mut size = 12.0;
    while size > 4.0 {
        let lines = wrap_lines(fonts, text, size, width).len() as f64;
        if lines * size * 1.15 <= height {
            break;
        }
        size -= 0.5;
    }
    size.max(4.0)
}

// ---------------------------------------------------------------------------
// FreeText
// ---------------------------------------------------------------------------

fn free_text(
    annot: &PdfDictionary,
    ctx: &SynthesisContext<'_>,
    rect: [f64; 4],
) -> Option<SynthesizedAppearance> {
    let reader = ctx.reader;
    let width = rect[2] - rect[0];
    let height = rect[3] - rect[1];
    let contents = annot
        .get("Contents")
        .and_then(|obj| resolve(reader, obj))
        .and_then(|obj| obj.as_string().map(crate::info::decode_pdf_text_string))
        .unwrap_or_default();
    let da = annot
        .get("DA")
        .and_then(|obj| resolve(reader, obj))
        .and_then(|obj| obj.as_string().map(parse_da))
        .unwrap_or_default();
    let size = if da.font_size > 0.0 {
        da.font_size
    } else {
        10.0
    };
    let alignment = match annot.get("Q") {
        None => 0,
        Some(obj) => obj.as_integer()?,
    };
    let border = border_style(annot, reader, 1.0);
    let background = AnnotColor::from_object(annot.get("C")).flatten();
    let rd = rect_differences(annot, reader, width, height);
    let (x0, y0) = (rd[0], rd[1]);
    let (bw, bh) = (width - rd[0] - rd[2], height - rd[1] - rd[3]);
    let mut out = String::from("q\n");
    let bwidth = border.width;
    if background.is_some() || bwidth > 0.0 {
        if bwidth > 0.0 {
            // Border colour is the text colour, as Acrobat and Poppler do.
            let _ = writeln!(
                out,
                "{} {} w {}",
                da.color.stroke_op(),
                n(bwidth),
                border.dash_op()
            );
        }
        if let Some(bg) = background {
            let _ = writeln!(out, "{}", bg.fill_op());
        }
        let _ = writeln!(
            out,
            "{} {} {} {} re {}",
            n(x0 + bwidth * 0.5),
            n(y0 + bwidth * 0.5),
            n((bw - bwidth).max(0.0)),
            n((bh - bwidth).max(0.0)),
            match (background.is_some(), bwidth > 0.0) {
                (true, true) => "B",
                (true, false) => "f",
                _ => "S",
            }
        );
    }
    let mut resources = Resources::default();
    if !contents.trim().is_empty() {
        let margin = bwidth * 2.0 + 2.0;
        let area = [
            x0 + margin,
            y0 + margin,
            (bw - 2.0 * margin).max(1.0),
            (bh - 2.0 * margin).max(1.0),
        ];
        let _ = writeln!(
            out,
            "q {} {} {} {} re W n",
            n(x0 + bwidth),
            n(y0 + bwidth),
            n((bw - 2.0 * bwidth).max(0.0)),
            n((bh - 2.0 * bwidth).max(0.0))
        );
        let dr: Vec<PdfDictionary> = ctx.acroform_dr.iter().cloned().collect();
        let mut fonts = TextFonts::resolve(da.font_name.as_deref(), &dr, reader);
        append_wrapped_text(
            &mut out, &mut fonts, &contents, size, da.color, alignment, area,
        );
        out.push_str("Q\n");
        fonts.add_to(&mut resources);
    }
    out.push_str("Q\n");
    if out == "q\nQ\n" {
        return None;
    }
    finish(
        out,
        [0.0, 0.0, width, height],
        rect,
        resources,
        opacity(annot),
    )
}

// ---------------------------------------------------------------------------
// Widgets: text fields and choice fields
// ---------------------------------------------------------------------------

pub(crate) struct WidgetTextLayout<'a> {
    pub width: f64,
    pub height: f64,
    pub mk: Option<&'a PdfDictionary>,
    pub border: BorderStyle,
    pub da: DaSpec,
    pub alignment: i64,
    pub multiline: bool,
    /// `MaxLen` when the comb flag is set.
    pub comb: Option<usize>,
    /// Combo box: reserve PDFium's 13pt drop button at the right edge.
    pub drop_button: bool,
}

/// Comb cell count for a text field: requires the comb flag (bit 25), no
/// multiline/password/file-select flags and a positive `MaxLen`.
pub(crate) fn comb_cells(field_flags: i64, max_len: Option<i64>) -> Option<usize> {
    const EXCLUDED: i64 = (1 << 12) | (1 << 13) | (1 << 20);
    if field_flags & FIELD_FLAG_COMB == 0 || field_flags & EXCLUDED != 0 {
        return None;
    }
    max_len
        .filter(|len| *len > 0 && *len <= 10_000)
        .map(|len| len as usize)
}

/// Text-field appearance: MK chrome, then the value laid out with the
/// resolved font (single line, word-wrapped multiline, or comb cells), with
/// font size 0 meaning auto-fit to the real text width.
pub(crate) fn text_field_appearance(
    layout: WidgetTextLayout<'_>,
    text: &str,
    mut fonts: TextFonts,
) -> Option<(String, Resources)> {
    let WidgetTextLayout {
        width,
        height,
        mk,
        border,
        da,
        alignment,
        multiline,
        comb,
        drop_button,
    } = layout;
    let mut out = String::new();
    let chrome = WidgetChrome::from_mk(mk, border);
    chrome.append_rect(&mut out, width, height);
    let bw = chrome.content_inset();
    // The text area ends where the drop button starts.
    let width = if drop_button {
        let inset = chrome.client_inset();
        let button = [
            width - inset - DROP_BUTTON_WIDTH,
            inset,
            width - inset,
            height - inset,
        ];
        let mut drawn = String::new();
        append_drop_button(&mut drawn, button);
        out.push_str(&drawn);
        (width - (DROP_BUTTON_WIDTH + inset - bw).max(DROP_BUTTON_WIDTH)).max(2.0 * bw)
    } else {
        width
    };
    let mut resources = Resources::default();
    if text.is_empty() {
        return Some((out, resources));
    }
    let pad = bw + 2.0;
    let inner_w = (width - 2.0 * pad).max(1.0);
    let inner_h = (height - 2.0 * bw).max(1.0);
    let _ = writeln!(
        out,
        "/Tx BMC q {} {} {} {} re W n",
        n(bw),
        n(bw),
        n((width - 2.0 * bw).max(0.0)),
        n((height - 2.0 * bw).max(0.0))
    );
    let ascent = fonts.ascent();
    let descent = fonts.descent();
    if multiline {
        let size = if da.font_size > 0.0 {
            da.font_size
        } else {
            auto_size_multiline(&mut fonts, text, inner_w, inner_h - 2.0)
        };
        append_wrapped_text(
            &mut out,
            &mut fonts,
            text,
            size,
            da.color,
            alignment,
            [pad, bw + 1.0, inner_w, inner_h - 2.0],
        );
    } else if let Some(cells) = comb {
        let chars: Vec<char> = text.chars().take(cells).collect();
        let cell = (width - 2.0 * bw) / cells as f64;
        let size = if da.font_size > 0.0 {
            da.font_size
        } else {
            let widest = chars
                .iter()
                .map(|ch| fonts.measure(ch.encode_utf8(&mut [0; 4]), 1.0))
                .fold(0.0, f64::max);
            let by_width = if widest > 0.0 {
                cell * 0.9 / widest
            } else {
                12.0
            };
            ((inner_h - 2.0) / (ascent - descent).max(0.8))
                .min(by_width)
                .max(2.0)
        };
        let baseline = bw + (inner_h - size * (ascent - descent)) * 0.5 - size * descent;
        // Leading empty cells for centred/right alignment, like Acrobat.
        let skip = match alignment {
            1 => (cells - chars.len()) / 2,
            2 => cells - chars.len(),
            _ => 0,
        } as f64;
        let _ = writeln!(out, "BT {}", da.color.fill_op());
        let mut last = (0.0, 0.0);
        for (index, ch) in chars.iter().enumerate() {
            let s = ch.to_string();
            let cw = fonts.measure(&s, size);
            let x = bw + (index as f64 + skip) * cell + (cell - cw) * 0.5;
            let _ = writeln!(out, "{} {} Td", n(x - last.0), n(baseline - last.1));
            last = (x, baseline);
            fonts.show(&mut out, &s, size);
        }
        out.push_str("ET\n");
    } else {
        let line: String = text
            .chars()
            .map(|ch| if ch == '\r' || ch == '\n' { ' ' } else { ch })
            .collect();
        let size = if da.font_size > 0.0 {
            da.font_size
        } else {
            let by_height = (inner_h - 2.0) / (ascent - descent).max(0.8);
            let unit_width = fonts.measure(&line, 1.0);
            let by_width = if unit_width > 0.0 {
                inner_w / unit_width
            } else {
                by_height
            };
            by_height
                .min(by_width)
                .min(12.0f64.max(by_height.min(24.0)))
                .max(2.0)
        };
        let line_width = fonts.measure(&line, size);
        let x = aligned_x(alignment, line_width, pad, inner_w);
        let baseline = bw + (inner_h - size * (ascent - descent)) * 0.5 - size * descent;
        let _ = writeln!(out, "BT {} {} {} Td", da.color.fill_op(), n(x), n(baseline));
        fonts.show(&mut out, &line, size);
        out.push_str("ET\n");
    }
    out.push_str("Q EMC\n");
    fonts.add_to(&mut resources);
    Some((out, resources))
}

/// One list-box option: display text and whether it is selected.
pub(crate) struct ListOption {
    pub text: String,
    pub selected: bool,
}

/// List-box appearance: visible options from `top_index`, selected rows
/// highlighted like Acrobat.
pub(crate) fn list_box_appearance(
    layout: WidgetTextLayout<'_>,
    options: &[ListOption],
    top_index: usize,
    mut fonts: TextFonts,
) -> Option<(String, Resources)> {
    let WidgetTextLayout {
        width,
        height,
        mk,
        border,
        da,
        alignment,
        ..
    } = layout;
    let mut out = String::new();
    let chrome = WidgetChrome::from_mk(mk, border);
    chrome.append_rect(&mut out, width, height);
    let bw = chrome.content_inset();
    let size = if da.font_size > 0.0 {
        da.font_size
    } else {
        12.0
    };
    let line_height = size * 1.15;
    let pad = bw + 2.0;
    let inner_w = (width - 2.0 * pad).max(1.0);
    let _ = writeln!(
        out,
        "/Tx BMC q {} {} {} {} re W n",
        n(bw),
        n(bw),
        n((width - 2.0 * bw).max(0.0)),
        n((height - 2.0 * bw).max(0.0))
    );
    let mut top = height - bw;
    let descent = fonts.descent();
    let start = top_index.min(options.len().saturating_sub(1));
    for option in options.iter().skip(start) {
        if top - line_height < bw - line_height * 0.5 {
            break;
        }
        if option.selected {
            let _ = writeln!(
                out,
                "q 0.6 0.75 0.85 rg {} {} {} {} re f Q",
                n(bw),
                n(top - line_height),
                n((width - 2.0 * bw).max(0.0)),
                n(line_height)
            );
        }
        if !option.text.is_empty() {
            let line_width = fonts.measure(&option.text, size);
            let x = aligned_x(alignment, line_width, pad, inner_w);
            let baseline = top - line_height - size * descent - size * 0.075;
            let _ = writeln!(out, "BT {} {} {} Td", da.color.fill_op(), n(x), n(baseline));
            fonts.show(&mut out, &option.text, size);
            out.push_str("ET\n");
        }
        top -= line_height;
    }
    out.push_str("Q EMC\n");
    let mut resources = Resources::default();
    fonts.add_to(&mut resources);
    Some((out, resources))
}

/// Wraps widget content into a Form XObject with `bbox` and an optional
/// `/Matrix` (`/MK /R` widget rotation).
pub(crate) fn widget_form_with(
    content: String,
    bbox: [f64; 4],
    matrix: Option<[f64; 6]>,
    resources: Resources,
) -> (PdfDictionary, Vec<u8>) {
    let content = if content.trim().is_empty() {
        // An empty regenerated field: paint nothing, but replace the stale
        // appearance.
        "q Q\n".to_string()
    } else {
        content
    };
    let mut dict = PdfDictionary::empty();
    dict.insert("Type", PdfObject::Name("XObject".into()));
    dict.insert("Subtype", PdfObject::Name("Form".into()));
    dict.insert(
        "BBox",
        PdfObject::Array(bbox.iter().map(|v| PdfObject::Real(*v)).collect()),
    );
    if let Some(matrix) = matrix {
        dict.insert(
            "Matrix",
            PdfObject::Array(matrix.iter().map(|v| PdfObject::Real(*v)).collect()),
        );
    }
    dict.insert("Resources", PdfObject::Dictionary(resources.into_dict()));
    dict.insert("Length", PdfObject::Integer(content.len() as i64));
    (dict, content.into_bytes())
}

/// Un-rotation matrix for NoRotate annotations: rotates counter-clockwise
/// by the page's clockwise `/Rotate` around the top-left corner of `rect`,
/// so the appearance stays upright on screen (ISO 32000 12.5.3).
pub(crate) fn no_rotate_matrix(rect: [f64; 4], page_rotation: u32) -> Option<[f64; 6]> {
    let rotation = page_rotation % 360;
    if rotation == 0 {
        return None;
    }
    let rect = normalized_rect(rect);
    let (x, y) = (rect[0], rect[3]);
    let angle = f64::from(rotation).to_radians();
    let (c, s) = (angle.cos().round_to(1e-12), angle.sin().round_to(1e-12));
    // Counter-clockwise rotation by `angle` about (x, y).
    Some([c, s, -s, c, x - c * x + s * y, y - s * x - c * y])
}

trait RoundTo {
    fn round_to(self, eps: f64) -> f64;
}

impl RoundTo for f64 {
    fn round_to(self, eps: f64) -> f64 {
        let rounded = self.round();
        if (self - rounded).abs() < eps {
            rounded
        } else {
            self
        }
    }
}

/// Known annotation subtypes for which the legacy Invisible flag is
/// ignored, as Poppler and PDFium do (it only applies to unknown types).
pub(crate) fn is_standard_annotation_subtype(subtype: &str) -> bool {
    matches!(
        subtype,
        "Text"
            | "Link"
            | "FreeText"
            | "Line"
            | "Square"
            | "Circle"
            | "Polygon"
            | "PolyLine"
            | "Highlight"
            | "Underline"
            | "Squiggly"
            | "StrikeOut"
            | "Stamp"
            | "Caret"
            | "Ink"
            | "Popup"
            | "FileAttachment"
            | "Sound"
            | "Movie"
            | "Widget"
            | "Screen"
            | "PrinterMark"
            | "TrapNet"
            | "Watermark"
            | "3D"
            | "Redact"
            | "Projection"
            | "RichMedia"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_gray_rgb_cmyk_and_transparent() {
        assert_eq!(
            AnnotColor::from_array(&[PdfObject::Real(0.5)]),
            Some(Some(AnnotColor::Gray(0.5)))
        );
        assert_eq!(AnnotColor::from_array(&[]), Some(None));
        assert_eq!(
            AnnotColor::from_array(&[PdfObject::Integer(1), PdfObject::Integer(0)]),
            None
        );
        assert_eq!(
            AnnotColor::Cmyk(0.0, 1.0, 0.0, 0.0).fill_op(),
            "0 1 0 0 k".to_string()
        );
    }

    #[test]
    fn da_parse_reads_font_size_and_color() {
        let da = parse_da(b"/Helv 0 Tf 0 0 1 rg");
        assert_eq!(da.font_name.as_deref(), Some("Helv"));
        assert_eq!(da.font_size, 0.0);
        assert_eq!(da.color, AnnotColor::Rgb(0.0, 0.0, 1.0));
        let da = parse_da(b"0.5 g /F2 9 Tf");
        assert_eq!(da.font_size, 9.0);
        assert_eq!(da.color, AnnotColor::Gray(0.5));
    }

    #[test]
    fn helvetica_measures_with_real_metrics() {
        let mut fonts = TextFonts::helvetica();
        // Helvetica: 'i' = 222, 'W' = 944.
        let narrow = fonts.measure("iiii", 10.0);
        let wide = fonts.measure("WWWW", 10.0);
        assert!((narrow - 8.88).abs() < 0.01, "{narrow}");
        assert!((wide - 37.76).abs() < 0.01, "{wide}");
    }

    #[test]
    fn wrap_lines_breaks_words_and_honours_newlines() {
        let mut fonts = TextFonts::helvetica();
        let lines = wrap_lines(&mut fonts, "alpha beta gamma\ndelta", 10.0, 60.0);
        assert_eq!(lines, vec!["alpha beta", "gamma", "delta"]);
        let lines = wrap_lines(&mut fonts, "abcdefghijklmnop", 10.0, 30.0);
        assert!(lines.len() > 2, "{lines:?}");
        assert!(lines
            .iter()
            .all(|line| fonts.measure(line, 10.0) <= 30.0 + 1e-9));
    }

    #[test]
    fn non_latin_text_switches_to_unicode_fallback_font() {
        let mut fonts = TextFonts::helvetica();
        let mut out = String::new();
        fonts.show(&mut out, "Ab Привет", 10.0);
        assert!(out.contains("/WFHelv 10 Tf"), "{out}");
        assert!(out.contains("/WFUni 10 Tf"), "{out}");
        let mut resources = Resources::default();
        fonts.add_to(&mut resources);
        let dict = resources.into_dict();
        let font = dict.get_dict("Font").unwrap();
        let uni = font.get_dict("WFUni").unwrap();
        assert_eq!(uni.get_name("Subtype"), Some("Type0"));
        assert_eq!(uni.get_name("Encoding"), Some("Identity-H"));
    }

    #[test]
    fn comb_requires_flag_and_positive_maxlen() {
        assert_eq!(comb_cells(1 << 24, Some(6)), Some(6));
        assert_eq!(comb_cells(1 << 24, None), None);
        assert_eq!(comb_cells(0, Some(6)), None);
        assert_eq!(comb_cells((1 << 24) | (1 << 12), Some(6)), None);
    }

    #[test]
    fn no_rotate_matrix_keeps_top_left_fixed_and_counter_rotates() {
        let rect = [100.0, 200.0, 120.0, 220.0];
        assert!(no_rotate_matrix(rect, 0).is_none());
        let m = no_rotate_matrix(rect, 90).unwrap();
        let apply = |x: f64, y: f64| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        let (tx, ty) = apply(100.0, 220.0);
        assert!((tx - 100.0).abs() < 1e-9 && (ty - 220.0).abs() < 1e-9);
        // A point to the right of the anchor moves up (counter-clockwise).
        let (rx, ry) = apply(110.0, 220.0);
        assert!((rx - 100.0).abs() < 1e-9 && (ry - 230.0).abs() < 1e-9);
    }

    #[test]
    fn line_endings_parse_names() {
        assert_eq!(
            LineEnding::from_name("ClosedArrow"),
            LineEnding::ClosedArrow
        );
        assert_eq!(
            LineEnding::from_name("RClosedArrow"),
            LineEnding::RClosedArrow
        );
        assert_eq!(LineEnding::from_name("Bogus"), LineEnding::None);
    }

    #[test]
    fn check_glyph_captions_map_zapf_dingbats_codes() {
        assert_eq!(CheckGlyph::from_caption("4"), Some(CheckGlyph::Check));
        assert_eq!(CheckGlyph::from_caption("l"), Some(CheckGlyph::Circle));
        assert_eq!(CheckGlyph::from_caption("8"), Some(CheckGlyph::Cross));
        assert_eq!(CheckGlyph::from_caption("u"), Some(CheckGlyph::Diamond));
        assert_eq!(CheckGlyph::from_caption("n"), Some(CheckGlyph::Square));
        assert_eq!(CheckGlyph::from_caption("H"), Some(CheckGlyph::Star));
        assert_eq!(CheckGlyph::from_caption("x"), None);
    }
}
