//! PDFium-compatible widget appearance pieces: background and border
//! painting for every `/BS /S` style, check-box and radio-button marks,
//! push-button caption and icon layout (`/MK /IF`, `/MK /TP`), the combo-box
//! drop button and `/MK /R` widget rotation.
//!
//! The geometry follows PDFium's `CPDFSDK_AppStream`
//! (`fpdfsdk/cpdfsdk_appstream.cpp`, BSD licensed): the drawings are
//! reimplemented here from that description, not copied.
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use std::fmt::Write as _;

use super::annotation_synthesis::{
    append_check_glyph, ellipse_path, n, AnnotColor, BorderKind, BorderStyle, CheckGlyph,
    Resources, TextFonts,
};

/// PDFium's control-point factor for quarter-ellipse Beziers.
const BEZIER: f64 = 0.552_284_749_8;

/// Width of PDFium's combo-box drop button, in points.
pub(crate) const DROP_BUTTON_WIDTH: f64 = 13.0;

/// PDFium's automatic font sizes (`CPVT_VariableText` font size steps).
const AUTO_FONT_SIZE_STEPS: [f64; 25] = [
    4.0, 6.0, 8.0, 9.0, 10.0, 12.0, 14.0, 18.0, 20.0, 25.0, 30.0, 35.0, 40.0, 45.0, 50.0, 55.0,
    60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 144.0,
];

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

/// PDFium `CFX_Color / 2`: every component halved; transparent becomes
/// mid grey.
fn halved(color: Option<AnnotColor>) -> AnnotColor {
    match color {
        None => AnnotColor::Rgb(0.5, 0.5, 0.5),
        Some(AnnotColor::Gray(g)) => AnnotColor::Gray(g / 2.0),
        Some(AnnotColor::Rgb(r, g, b)) => AnnotColor::Rgb(r / 2.0, g / 2.0, b / 2.0),
        Some(AnnotColor::Cmyk(c, m, y, k)) => AnnotColor::Cmyk(c / 2.0, m / 2.0, y / 2.0, k / 2.0),
    }
}

/// PDFium `CFX_Color - amount`: every component reduced (not below 0);
/// transparent becomes `1 - amount` grey.
fn reduced(color: Option<AnnotColor>, amount: f64) -> AnnotColor {
    let sub = |v: f64| (v - amount).max(0.0);
    match color {
        None => AnnotColor::Rgb(sub(1.0), sub(1.0), sub(1.0)),
        Some(AnnotColor::Gray(g)) => AnnotColor::Gray(sub(g)),
        Some(AnnotColor::Rgb(r, g, b)) => AnnotColor::Rgb(sub(r), sub(g), sub(b)),
        Some(AnnotColor::Cmyk(c, m, y, k)) => AnnotColor::Cmyk(sub(c), sub(m), sub(y), sub(k)),
    }
}

// ---------------------------------------------------------------------------
// Background and border
// ---------------------------------------------------------------------------

/// A widget's `/MK /BG` background, `/MK /BC` border colour and border
/// style (`/BS` or `/Border`).
#[derive(Clone, Debug)]
pub(crate) struct WidgetChrome {
    pub background: Option<AnnotColor>,
    pub border_color: Option<AnnotColor>,
    pub style: BorderStyle,
}

impl WidgetChrome {
    pub(crate) fn from_mk(mk: Option<&PdfDictionary>, style: BorderStyle) -> Self {
        let color = |key: &str| {
            mk.and_then(|mk| AnnotColor::from_object(mk.get(key)))
                .flatten()
        };
        Self {
            background: color("BG"),
            border_color: color("BC"),
            style,
        }
    }

    /// Painted border width: beveled and inset borders are drawn at twice
    /// the `/W` width (an outer frame plus the bevel bands).
    pub(crate) fn painted_width(&self) -> f64 {
        let width = self.style.width.max(0.0);
        match self.style.kind {
            BorderKind::Beveled | BorderKind::Inset => width * 2.0,
            _ => width,
        }
    }

    /// Whether any border pixels are painted: a border colour, or the bevel
    /// bands of a beveled or inset border (PDFium paints those even without
    /// `/BC`).
    pub(crate) fn draws_border(&self) -> bool {
        self.style.width > 0.0
            && (self.border_color.is_some()
                || matches!(self.style.kind, BorderKind::Beveled | BorderKind::Inset))
    }

    /// Distance from the widget edge to its content area.
    pub(crate) fn content_inset(&self) -> f64 {
        if self.draws_border() {
            self.painted_width()
        } else {
            0.0
        }
    }

    /// PDFium's client rectangle inset: the painted border width whether or
    /// not a border colour is set.
    pub(crate) fn client_inset(&self) -> f64 {
        self.painted_width()
    }

    fn bevel_colors(&self) -> (Option<AnnotColor>, Option<AnnotColor>) {
        match self.style.kind {
            BorderKind::Beveled => (Some(AnnotColor::WHITE), Some(halved(self.background))),
            BorderKind::Inset => (Some(AnnotColor::Gray(0.5)), Some(AnnotColor::Gray(0.75))),
            _ => (None, None),
        }
    }

    /// Rectangular background and border over `[0 0 width height]`.
    pub(crate) fn append_rect(&self, out: &mut String, width: f64, height: f64) {
        if let Some(bg) = self.background {
            let _ = writeln!(
                out,
                "q {} 0 0 {} {} re f Q",
                bg.fill_op(),
                n(width),
                n(height)
            );
        }
        let (left_top, right_bottom) = self.bevel_colors();
        append_rect_border(
            out,
            [0.0, 0.0, width, height],
            self.painted_width(),
            &BorderPaint {
                color: self.border_color,
                left_top,
                right_bottom,
                style: &self.style,
            },
        );
    }

    /// Round (radio-button circle style) background and border inside the
    /// centred square of `[0 0 width height]`, inset by 1pt like PDFium.
    pub(crate) fn append_circle(&self, out: &mut String, width: f64, height: f64) {
        let rect = deflate(center_square([0.0, 0.0, width, height]), 1.0);
        if rect[2] <= rect[0] || rect[3] <= rect[1] {
            return;
        }
        if let Some(bg) = self.background {
            let _ = write!(out, "q {} ", bg.fill_op());
            ellipse_in(out, rect);
            out.push_str("f Q\n");
        }
        let (left_top, right_bottom) = match self.style.kind {
            BorderKind::Beveled => (
                Some(AnnotColor::WHITE),
                Some(reduced(self.background, 0.25)),
            ),
            BorderKind::Inset => (Some(AnnotColor::Gray(0.5)), Some(AnnotColor::Gray(0.75))),
            _ => (None, None),
        };
        append_circle_border(
            out,
            rect,
            self.painted_width(),
            &BorderPaint {
                color: self.border_color,
                left_top,
                right_bottom,
                style: &self.style,
            },
        );
    }
}

struct BorderPaint<'a> {
    color: Option<AnnotColor>,
    left_top: Option<AnnotColor>,
    right_bottom: Option<AnnotColor>,
    style: &'a BorderStyle,
}

fn append_rect_border(out: &mut String, rect: [f64; 4], width: f64, paint: &BorderPaint<'_>) {
    if width <= 0.0 {
        return;
    }
    let [l, b, r, t] = rect;
    let hw = width / 2.0;
    let frame = |out: &mut String, color: AnnotColor, inset: f64| {
        let _ = writeln!(
            out,
            "{} {} {} {} {} re {} {} {} {} re f*",
            color.fill_op(),
            n(l),
            n(b),
            n(r - l),
            n(t - b),
            n(l + inset),
            n(b + inset),
            n((r - l - 2.0 * inset).max(0.0)),
            n((t - b - 2.0 * inset).max(0.0))
        );
    };
    let mut body = String::new();
    match paint.style.kind {
        BorderKind::Solid => {
            if let Some(color) = paint.color {
                frame(&mut body, color, width);
            }
        }
        BorderKind::Dashed => {
            if let Some(color) = paint.color {
                let _ = writeln!(
                    body,
                    "{} {} w {} {} {} {} {} re S",
                    color.stroke_op(),
                    n(width),
                    paint.style.dash_op(),
                    n(l + hw),
                    n(b + hw),
                    n((r - l - width).max(0.0)),
                    n((t - b - width).max(0.0))
                );
            }
        }
        BorderKind::Beveled | BorderKind::Inset => {
            if let Some(color) = paint.left_top {
                let _ = writeln!(
                    body,
                    "{} {} {} m {} {} l {} {} l {} {} l {} {} l {} {} l f",
                    color.fill_op(),
                    n(l + hw),
                    n(b + hw),
                    n(l + hw),
                    n(t - hw),
                    n(r - hw),
                    n(t - hw),
                    n(r - width),
                    n(t - width),
                    n(l + width),
                    n(t - width),
                    n(l + width),
                    n(b + width)
                );
            }
            if let Some(color) = paint.right_bottom {
                let _ = writeln!(
                    body,
                    "{} {} {} m {} {} l {} {} l {} {} l {} {} l {} {} l f",
                    color.fill_op(),
                    n(r - hw),
                    n(t - hw),
                    n(r - hw),
                    n(b + hw),
                    n(l + hw),
                    n(b + hw),
                    n(l + width),
                    n(b + width),
                    n(r - width),
                    n(b + width),
                    n(r - width),
                    n(t - width)
                );
            }
            if let Some(color) = paint.color {
                frame(&mut body, color, hw);
            }
        }
        BorderKind::Underline => {
            if let Some(color) = paint.color {
                let _ = writeln!(
                    body,
                    "{} {} w {} {} m {} {} l S",
                    color.stroke_op(),
                    n(width),
                    n(l),
                    n(b + hw),
                    n(r),
                    n(b + hw)
                );
            }
        }
    }
    if !body.is_empty() {
        let _ = write!(out, "q\n{body}Q\n");
    }
}

fn append_circle_border(out: &mut String, rect: [f64; 4], width: f64, paint: &BorderPaint<'_>) {
    if width <= 0.0 {
        return;
    }
    let hw = width / 2.0;
    let mut body = String::new();
    match paint.style.kind {
        BorderKind::Solid | BorderKind::Underline | BorderKind::Dashed => {
            if let Some(color) = paint.color {
                let dash = if paint.style.kind == BorderKind::Dashed {
                    paint.style.dash_op()
                } else {
                    String::new()
                };
                let _ = write!(body, "q {} w {dash} {} ", n(width), color.stroke_op());
                ellipse_in(&mut body, deflate(rect, hw));
                body.push_str("S Q\n");
            }
        }
        BorderKind::Beveled | BorderKind::Inset => {
            if let Some(color) = paint.color {
                let _ = write!(body, "q {} w {} ", n(hw), color.stroke_op());
                ellipse_in(&mut body, rect);
                body.push_str("S Q\n");
            }
            let quarter = deflate(rect, hw * 0.75);
            for (color, angle) in [
                (paint.left_top, std::f64::consts::FRAC_PI_4),
                (paint.right_bottom, std::f64::consts::PI * 1.25),
            ] {
                if let Some(color) = color {
                    let _ = write!(body, "q {} w {} ", n(hw), color.stroke_op());
                    half_ellipse(&mut body, quarter, angle);
                    body.push_str("S Q\n");
                }
            }
        }
    }
    if !body.is_empty() {
        let _ = write!(out, "q\n{body}Q\n");
    }
}

/// Upper half of the ellipse inscribed in `rect`, rotated by `angle`
/// (radians) about its centre (PDFium `GetAP_HalfCircle`).
fn half_ellipse(out: &mut String, rect: [f64; 4], angle: f64) {
    let w = rect[2] - rect[0];
    let h = rect[3] - rect[1];
    let (s, c) = angle.sin_cos();
    let _ = write!(
        out,
        "{} {} {} {} {} {} cm {} 0 m {} {} {} {} 0 {} c {} {} {} {} {} 0 c ",
        n(c),
        n(s),
        n(-s),
        n(c),
        n(rect[0] + w / 2.0),
        n(rect[1] + h / 2.0),
        n(-w / 2.0),
        n(-w / 2.0),
        n(h / 2.0 * BEZIER),
        n(-w / 2.0 * BEZIER),
        n(h / 2.0),
        n(h / 2.0),
        n(w / 2.0 * BEZIER),
        n(h / 2.0),
        n(w / 2.0),
        n(h / 2.0 * BEZIER),
        n(w / 2.0)
    );
}

fn ellipse_in(out: &mut String, rect: [f64; 4]) {
    ellipse_path(
        out,
        (rect[0] + rect[2]) / 2.0,
        (rect[1] + rect[3]) / 2.0,
        (rect[2] - rect[0]) / 2.0,
        (rect[3] - rect[1]) / 2.0,
    );
}

fn deflate(rect: [f64; 4], by: f64) -> [f64; 4] {
    [rect[0] + by, rect[1] + by, rect[2] - by, rect[3] - by]
}

/// The largest square centred in `rect`.
fn center_square(rect: [f64; 4]) -> [f64; 4] {
    let w = rect[2] - rect[0];
    let h = rect[3] - rect[1];
    let side = w.min(h);
    let x = rect[0] + (w - side) / 2.0;
    let y = rect[1] + (h - side) / 2.0;
    [x, y, x + side, y + side]
}

// ---------------------------------------------------------------------------
// Check boxes and radio buttons
// ---------------------------------------------------------------------------

/// Mark of a selected check box or radio button: a vector shape for the
/// common ZapfDingbats captions, anything else as ZapfDingbats text.
pub(crate) enum ButtonMark<'a> {
    Glyph(CheckGlyph),
    Dingbat(&'a str),
}

impl<'a> ButtonMark<'a> {
    /// The `/MK /CA` caption's mark, or `default` for an empty caption.
    pub(crate) fn from_caption(caption: &'a str, default: CheckGlyph) -> Self {
        if caption.is_empty() {
            return ButtonMark::Glyph(default);
        }
        match CheckGlyph::from_caption(caption) {
            Some(glyph) => ButtonMark::Glyph(glyph),
            None => ButtonMark::Dingbat(caption),
        }
    }
}

/// Check-box or radio-button appearance: chrome (round for the radio
/// circle style, like PDFium) and, when selected, the mark in the DA
/// colour. `font_size` is the `/DA` size (0 = fit the box).
pub(crate) struct ToggleButton<'a> {
    pub width: f64,
    pub height: f64,
    pub chrome: &'a WidgetChrome,
    pub radio: bool,
    pub selected: bool,
    pub mark: ButtonMark<'a>,
    pub color: AnnotColor,
    pub font_size: f64,
}

pub(crate) fn toggle_button_appearance(button: ToggleButton<'_>) -> (String, Resources) {
    let ToggleButton {
        width,
        height,
        chrome,
        radio,
        selected,
        mark,
        color,
        font_size,
    } = button;
    let mut out = String::new();
    let mut resources = Resources::default();
    let round = radio && matches!(mark, ButtonMark::Glyph(CheckGlyph::Circle));
    if round {
        chrome.append_circle(&mut out, width, height);
    } else {
        chrome.append_rect(&mut out, width, height);
    }
    if !selected {
        return (out, resources);
    }
    let inset = chrome.client_inset();
    let inner = (width.min(height) - 2.0 * inset).max(1.0);
    let side = if round {
        // PDFium: half of the client square for the radio dot.
        inner * 0.5
    } else if font_size > 0.0 {
        (font_size * 0.75).min(inner)
    } else {
        inner * 0.75
    };
    match mark {
        ButtonMark::Glyph(glyph) => append_check_glyph(&mut out, glyph, width, height, side, color),
        ButtonMark::Dingbat(caption) => {
            let size = side / 0.75;
            let mut font = PdfDictionary::empty();
            font.insert("Type", PdfObject::Name("Font".into()));
            font.insert("Subtype", PdfObject::Name("Type1".into()));
            font.insert("BaseFont", PdfObject::Name("ZapfDingbats".into()));
            resources.add_font("ZaDb", PdfObject::Dictionary(font));
            let mut literal = String::from("<");
            for byte in caption.bytes() {
                let _ = write!(literal, "{byte:02X}");
            }
            literal.push('>');
            let _ = writeln!(
                out,
                "q BT {} /ZaDb {} Tf {} {} Td {literal} Tj ET Q",
                color.fill_op(),
                n(size),
                n((width - size * 0.75) * 0.5),
                n((height - size * 0.7) * 0.5)
            );
        }
    }
    (out, resources)
}

/// PDFium's grey hairline rectangle for a check box or radio button whose
/// `/AP /N` has no appearance for the current state. The form's BBox is
/// one point larger on every side so the hairline is not clipped; returns
/// `(content, bbox)`.
pub(crate) fn missing_state_outline(width: f64, height: f64) -> (String, [f64; 4]) {
    (
        format!("q 0.667 G 0 w 0 0 {} {} re S Q\n", n(width), n(height)),
        [-1.0, -1.0, width + 1.0, height + 1.0],
    )
}

// ---------------------------------------------------------------------------
// Combo box drop button
// ---------------------------------------------------------------------------

/// PDFium's combo-box drop button in `rect`: a light grey beveled box with
/// a downward triangle.
pub(crate) fn append_drop_button(out: &mut String, rect: [f64; 4]) {
    let [l, b, r, t] = rect;
    if r - l <= 0.0 || t - b <= 0.0 {
        return;
    }
    let grey = AnnotColor::Rgb(220.0 / 255.0, 220.0 / 255.0, 220.0 / 255.0);
    let _ = writeln!(
        out,
        "q {} {} {} {} {} re f Q",
        grey.fill_op(),
        n(l),
        n(b),
        n(r - l),
        n(t - b)
    );
    let style = BorderStyle {
        width: 2.0,
        kind: BorderKind::Beveled,
        dash: Vec::new(),
    };
    append_rect_border(
        out,
        rect,
        2.0,
        &BorderPaint {
            color: Some(AnnotColor::BLACK),
            left_top: Some(AnnotColor::WHITE),
            right_bottom: Some(AnnotColor::Gray(0.5)),
            style: &style,
        },
    );
    if r - l > 6.0 && t - b > 6.0 {
        let cx = (l + r) / 2.0;
        let cy = (b + t) / 2.0;
        let _ = writeln!(
            out,
            "q 0 g {} {} m {} {} l {} {} l h f Q",
            n(cx - 3.0),
            n(cy + 1.5),
            n(cx + 3.0),
            n(cy + 1.5),
            n(cx),
            n(cy - 1.5)
        );
    }
}

// ---------------------------------------------------------------------------
// Push buttons
// ---------------------------------------------------------------------------

/// `/MK /TP`: where the caption goes relative to the icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptionPosition {
    CaptionOnly,
    IconOnly,
    Below,
    Above,
    Right,
    Left,
    Overlaid,
}

impl CaptionPosition {
    pub(crate) fn from_mk(mk: Option<&PdfDictionary>) -> Self {
        match mk.and_then(|mk| mk.get_integer("TP")).unwrap_or(0) {
            1 => Self::IconOnly,
            2 => Self::Below,
            3 => Self::Above,
            4 => Self::Right,
            5 => Self::Left,
            6 => Self::Overlaid,
            _ => Self::CaptionOnly,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconScale {
    Always,
    Bigger,
    Smaller,
    Never,
}

/// `/MK /IF` icon fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct IconFit {
    pub scale: IconScale,
    pub proportional: bool,
    /// Fraction of the leftover space to the left and below the icon.
    pub align: (f64, f64),
    /// `/FB`: fit the icon to the full widget, ignoring the border.
    pub full_bounds: bool,
}

impl Default for IconFit {
    fn default() -> Self {
        Self {
            scale: IconScale::Always,
            proportional: true,
            align: (0.5, 0.5),
            full_bounds: false,
        }
    }
}

impl IconFit {
    pub(crate) fn from_dict(dict: Option<&PdfDictionary>) -> Self {
        let mut fit = Self::default();
        let Some(dict) = dict else {
            return fit;
        };
        fit.scale = match dict.get_name("SW") {
            Some("B") => IconScale::Bigger,
            Some("S") => IconScale::Smaller,
            Some("N") => IconScale::Never,
            _ => IconScale::Always,
        };
        fit.proportional = dict.get_name("S") != Some("A");
        if let Some(PdfObject::Array(items)) = dict.get("A") {
            let value = |index: usize, default: f64| {
                items
                    .get(index)
                    .and_then(PdfObject::as_number)
                    .filter(|v| v.is_finite())
                    .map(|v| v.clamp(0.0, 1.0))
                    .unwrap_or(default)
            };
            fit.align = (value(0, 0.5), value(1, 0.5));
        }
        fit.full_bounds = dict.get_bool("FB").unwrap_or(false);
        fit
    }

    /// Horizontal and vertical scale for an icon of `image` size placed in
    /// a `plate` sized area.
    pub(crate) fn scale(&self, image: (f64, f64), plate: (f64, f64)) -> (f64, f64) {
        let (iw, ih) = (image.0.max(1e-6), image.1.max(1e-6));
        let (pw, ph) = plate;
        let (mut sx, mut sy) = (1.0, 1.0);
        match self.scale {
            IconScale::Always => {
                sx = pw / iw.max(1.0);
                sy = ph / ih.max(1.0);
            }
            IconScale::Bigger => {
                if pw < iw {
                    sx = pw / iw.max(1.0);
                }
                if ph < ih {
                    sy = ph / ih.max(1.0);
                }
            }
            IconScale::Smaller => {
                if pw > iw {
                    sx = pw / iw.max(1.0);
                }
                if ph > ih {
                    sy = ph / ih.max(1.0);
                }
            }
            IconScale::Never => {}
        }
        if self.proportional {
            let s = sx.min(sy);
            (s, s)
        } else {
            (sx, sy)
        }
    }
}

/// A push-button icon: the `/MK /I` XObject and its geometry.
#[derive(Clone, Debug)]
pub(crate) struct ButtonIcon {
    pub object: PdfObject,
    pub bbox: [f64; 4],
    pub matrix: [f64; 6],
}

impl ButtonIcon {
    /// Resolves `/MK /I`: a Form XObject with a valid `/BBox`, or an Image
    /// XObject (a unit square).
    pub(crate) fn from_mk(mk: Option<&PdfDictionary>, reader: &PdfReader) -> Option<Self> {
        let object = mk?.get("I")?.clone();
        let PdfObject::Stream { dict, .. } = reader.resolve(object.clone()).ok()? else {
            return None;
        };
        let numbers = |key: &str| -> Option<Vec<f64>> {
            let items = match dict.get(key)? {
                PdfObject::Array(items) => items.clone(),
                other => match reader.resolve(other.clone()).ok()? {
                    PdfObject::Array(items) => items,
                    _ => return None,
                },
            };
            items
                .iter()
                .map(|item| item.as_number().filter(|v| v.is_finite()))
                .collect()
        };
        match dict.get_name("Subtype") {
            Some("Form") => {
                let bbox = numbers("BBox").filter(|v| v.len() == 4)?;
                let bbox = [
                    bbox[0].min(bbox[2]),
                    bbox[1].min(bbox[3]),
                    bbox[0].max(bbox[2]),
                    bbox[1].max(bbox[3]),
                ];
                if bbox[2] - bbox[0] <= 0.0 || bbox[3] - bbox[1] <= 0.0 {
                    return None;
                }
                let matrix = numbers("Matrix")
                    .filter(|v| v.len() == 6)
                    .map(|v| [v[0], v[1], v[2], v[3], v[4], v[5]])
                    .filter(|m| (m[0] * m[3] - m[1] * m[2]).abs() > 1e-12)
                    .unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
                Some(Self {
                    object,
                    bbox,
                    matrix,
                })
            }
            Some("Image") => Some(Self {
                object,
                bbox: [0.0, 0.0, 1.0, 1.0],
                matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            }),
            _ => None,
        }
    }

    fn size(&self) -> (f64, f64) {
        (self.bbox[2] - self.bbox[0], self.bbox[3] - self.bbox[1])
    }
}

const ICON_RESOURCE: &str = "WFIcon";

fn invert(m: [f64; 6]) -> [f64; 6] {
    let det = m[0] * m[3] - m[1] * m[2];
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    [a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)]
}

/// Draws `icon` fitted into `plate` (`[l b r t]`), clipped to it.
fn append_icon(out: &mut String, icon: &ButtonIcon, fit: &IconFit, plate: [f64; 4]) {
    let pw = plate[2] - plate[0];
    let ph = plate[3] - plate[1];
    if pw <= 0.0 || ph <= 0.0 {
        return;
    }
    let image = icon.size();
    let (sx, sy) = fit.scale(image, (pw, ph));
    let ox = (pw - image.0 * sx) * fit.align.0;
    let oy = (ph - image.1 * sy) * fit.align.1;
    let inv = invert(icon.matrix);
    let _ = writeln!(
        out,
        "q {} {} {} {} re W n\n{} 0 0 {} {} {} cm\n1 0 0 1 {} {} cm\n{} {} {} {} {} {} cm\n0 g 0 G 1 w /{ICON_RESOURCE} Do\nQ",
        n(plate[0]),
        n(plate[1]),
        n(pw),
        n(ph),
        num(sx),
        num(sy),
        n(plate[0] + ox),
        n(plate[1] + oy),
        num(-icon.bbox[0]),
        num(-icon.bbox[1]),
        num(inv[0]),
        num(inv[1]),
        num(inv[2]),
        num(inv[3]),
        num(inv[4]),
        num(inv[5])
    );
}

/// Number formatting with more precision than `n` for scale factors.
fn num(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut s = format!("{value:.6}");
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

/// PDFium's automatic caption size: the largest size step at which the
/// single-line `text` fits `plate` (`(width, height)`).
fn auto_caption_size(fonts: &mut TextFonts, text: &str, plate: (f64, f64)) -> f64 {
    let line = (fonts.ascent() - fonts.descent()).max(0.5);
    let unit = fonts.measure(text, 1.0);
    let fits = |size: f64| unit * size <= plate.0 + 1e-6 && line * size <= plate.1 + 1e-6;
    let mut chosen = AUTO_FONT_SIZE_STEPS[0];
    for size in AUTO_FONT_SIZE_STEPS {
        if fits(size) {
            chosen = size;
        } else {
            break;
        }
    }
    chosen
}

/// Push-button appearance inputs.
pub(crate) struct PushButton<'a> {
    pub width: f64,
    pub height: f64,
    pub chrome: &'a WidgetChrome,
    pub caption: &'a str,
    pub icon: Option<&'a ButtonIcon>,
    pub fit: IconFit,
    pub position: CaptionPosition,
    pub color: AnnotColor,
    /// `/DA` font size; `None` when there is no `/DA` (PDFium uses 12).
    pub font_size: Option<f64>,
}

/// Push button like PDFium `SetAsPushButton` (normal appearance):
/// background, border, then the icon and caption laid out per `/MK /TP`
/// inside the client area (`/IF /FB` = the whole widget).
pub(crate) fn push_button_appearance(
    button: PushButton<'_>,
    mut fonts: TextFonts,
) -> (String, Resources) {
    let PushButton {
        width,
        height,
        chrome,
        caption,
        icon,
        fit,
        position,
        color,
        font_size,
    } = button;
    let mut out = String::new();
    let mut resources = Resources::default();
    chrome.append_rect(&mut out, width, height);

    let inset = if fit.full_bounds {
        0.0
    } else {
        chrome.client_inset()
    };
    let bbox = [inset, inset, width - inset, height - inset];
    if bbox[2] <= bbox[0] || bbox[3] <= bbox[1] {
        return (out, resources);
    }
    let size = font_size.unwrap_or(12.0);
    let auto = size <= 0.0;
    let line = (fonts.ascent() - fonts.descent()).max(0.5);
    // Caption content size at a fixed font size (auto: empty, as PDFium
    // measures before the caption has a plate).
    let (content_w, content_h) = if auto || caption.is_empty() {
        (0.0, 0.0)
    } else {
        (fonts.measure(caption, size), line * size)
    };
    const AUTO_SCALE: f64 = 1.0 / 3.0;
    let [l, b, r, t] = bbox;
    let (bw, bh) = (r - l, t - b);
    let empty = [0.0; 4];
    let (label, icon_rect) = match (position, icon.is_some()) {
        (CaptionPosition::CaptionOnly, _) => (bbox, empty),
        (CaptionPosition::IconOnly, _) => (empty, bbox),
        (CaptionPosition::Overlaid, _) => (bbox, bbox),
        (_, false) => (bbox, empty),
        (CaptionPosition::Below, true) => {
            let label_h = if auto { bh * AUTO_SCALE } else { content_h };
            if b + label_h > t {
                (bbox, empty)
            } else {
                ([l, b, r, b + label_h], [l, b + label_h, r, t])
            }
        }
        (CaptionPosition::Above, true) => {
            let label_h = if auto { bh * AUTO_SCALE } else { content_h };
            if b + label_h > t {
                (bbox, empty)
            } else {
                ([l, t - label_h, r, t], [l, b, r, t - label_h])
            }
        }
        (CaptionPosition::Right, true) => {
            let label_w = if auto { bw * AUTO_SCALE } else { content_w };
            if l + label_w > r {
                (bbox, empty)
            } else {
                ([r - label_w, b, r, t], [l, b, r - label_w, t])
            }
        }
        (CaptionPosition::Left, true) => {
            let label_w = if auto { bw * AUTO_SCALE } else { content_w };
            if l + label_w > r {
                (bbox, empty)
            } else {
                ([l, b, l + label_w, t], [l + label_w, b, r, t])
            }
        }
    };

    let mut body = String::new();
    if let Some(icon) = icon {
        if icon_rect[2] > icon_rect[0] && icon_rect[3] > icon_rect[1] {
            append_icon(&mut body, icon, &fit, icon_rect);
            resources.add_xobject(ICON_RESOURCE, icon.object.clone());
        }
    }
    let label_w = label[2] - label[0];
    let label_h = label[3] - label[1];
    if !caption.is_empty() && label_w > 0.0 && label_h > 0.0 {
        let size = if auto {
            auto_caption_size(&mut fonts, caption, (label_w, label_h))
        } else {
            size
        };
        let text_w = fonts.measure(caption, size);
        let x = label[0] + (label_w - text_w) / 2.0;
        let baseline = label[1] + (label_h - line * size) / 2.0 - fonts.descent() * size;
        let _ = writeln!(body, "BT {} {} {} Td", color.fill_op(), n(x), n(baseline));
        fonts.show(&mut body, caption, size);
        body.push_str("ET\n");
        fonts.add_to(&mut resources);
    }
    if !body.is_empty() {
        let _ = write!(
            out,
            "q {} {} {} {} re W n\n{body}Q\n",
            n(l),
            n(b),
            n(bw),
            n(bh)
        );
    }
    (out, resources)
}

// ---------------------------------------------------------------------------
// Rotation
// ---------------------------------------------------------------------------

/// `/MK /R` normalised to 0, 90, 180 or 270 (anything else is 0).
pub(crate) fn mk_rotation(mk: Option<&PdfDictionary>) -> u32 {
    let value = mk.and_then(|mk| mk.get_integer("R")).unwrap_or(0);
    match value.rem_euclid(360) {
        90 => 90,
        180 => 180,
        270 => 270,
        _ => 0,
    }
}

/// Layout size of a widget appearance: the rectangle's width and height,
/// swapped for 90 and 270 degree rotation.
pub(crate) fn rotated_size(rotation: u32, width: f64, height: f64) -> (f64, f64) {
    if rotation == 90 || rotation == 270 {
        (height, width)
    } else {
        (width, height)
    }
}

/// Form `/Matrix` that turns an appearance laid out in the rotated size
/// back onto a `width` x `height` widget (PDFium `CPDFSDK_Widget::GetMatrix`).
pub(crate) fn rotation_matrix(rotation: u32, width: f64, height: f64) -> Option<[f64; 6]> {
    match rotation {
        90 => Some([0.0, 1.0, -1.0, 0.0, width, 0.0]),
        180 => Some([-1.0, 0.0, 0.0, -1.0, width, height]),
        270 => Some([0.0, -1.0, 1.0, 0.0, 0.0, height]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(kind: BorderKind, width: f64) -> BorderStyle {
        BorderStyle {
            width,
            kind,
            dash: vec![3.0],
        }
    }

    #[test]
    fn beveled_border_doubles_width_and_halves_background() {
        let chrome = WidgetChrome {
            background: Some(AnnotColor::Rgb(0.8, 0.6, 0.4)),
            border_color: Some(AnnotColor::BLACK),
            style: style(BorderKind::Beveled, 1.0),
        };
        assert_eq!(chrome.painted_width(), 2.0);
        let mut out = String::new();
        chrome.append_rect(&mut out, 40.0, 20.0);
        assert!(out.contains("1 g 1 1 m"), "white top-left band: {out}");
        assert!(out.contains("0.4 0.3 0.2 rg"), "halved background: {out}");
        assert!(out.contains("f*"), "outer frame: {out}");
    }

    #[test]
    fn inset_border_uses_grey_bands_even_without_border_colour() {
        let chrome = WidgetChrome {
            background: None,
            border_color: None,
            style: style(BorderKind::Inset, 1.0),
        };
        assert!(chrome.draws_border());
        let mut out = String::new();
        chrome.append_rect(&mut out, 40.0, 20.0);
        assert!(out.contains("0.5 g"), "{out}");
        assert!(out.contains("0.75 g"), "{out}");
        assert!(!out.contains("f*"), "no frame without /BC: {out}");
    }

    #[test]
    fn dashed_and_underline_borders_stroke() {
        let mut chrome = WidgetChrome {
            background: None,
            border_color: Some(AnnotColor::Rgb(1.0, 0.0, 0.0)),
            style: BorderStyle {
                width: 2.0,
                kind: BorderKind::Dashed,
                dash: vec![4.0, 2.0],
            },
        };
        let mut out = String::new();
        chrome.append_rect(&mut out, 40.0, 20.0);
        assert!(out.contains("[4 2] 0 d"), "{out}");
        assert!(out.contains("1 1 38 18 re S"), "{out}");
        chrome.style = style(BorderKind::Underline, 2.0);
        let mut out = String::new();
        chrome.append_rect(&mut out, 40.0, 20.0);
        assert!(out.contains("0 1 m 40 1 l S"), "{out}");
    }

    #[test]
    fn icon_fit_scales_like_pdfium() {
        let fit = IconFit::default();
        assert_eq!(fit.scale((10.0, 20.0), (40.0, 40.0)), (2.0, 2.0));
        let anamorphic = IconFit {
            proportional: false,
            ..IconFit::default()
        };
        assert_eq!(anamorphic.scale((10.0, 20.0), (40.0, 40.0)), (4.0, 2.0));
        let bigger = IconFit {
            scale: IconScale::Bigger,
            ..IconFit::default()
        };
        assert_eq!(bigger.scale((10.0, 20.0), (40.0, 40.0)), (1.0, 1.0));
        assert_eq!(bigger.scale((80.0, 20.0), (40.0, 40.0)), (0.5, 0.5));
        let smaller = IconFit {
            scale: IconScale::Smaller,
            ..IconFit::default()
        };
        assert_eq!(smaller.scale((80.0, 20.0), (40.0, 40.0)), (1.0, 1.0));
        let never = IconFit {
            scale: IconScale::Never,
            ..IconFit::default()
        };
        assert_eq!(never.scale((80.0, 20.0), (40.0, 40.0)), (1.0, 1.0));
    }

    #[test]
    fn rotation_swaps_layout_size_and_builds_matrix() {
        assert_eq!(rotated_size(90, 40.0, 20.0), (20.0, 40.0));
        assert_eq!(rotated_size(180, 40.0, 20.0), (40.0, 20.0));
        assert_eq!(
            rotation_matrix(90, 40.0, 20.0),
            Some([0.0, 1.0, -1.0, 0.0, 40.0, 0.0])
        );
        assert_eq!(rotation_matrix(0, 40.0, 20.0), None);
        let mut mk = PdfDictionary::empty();
        mk.insert("R", PdfObject::Integer(-90));
        assert_eq!(mk_rotation(Some(&mk)), 270);
        mk.insert("R", PdfObject::Integer(45));
        assert_eq!(mk_rotation(Some(&mk)), 0);
    }

    #[test]
    fn drop_button_draws_grey_box_and_triangle() {
        let mut out = String::new();
        append_drop_button(&mut out, [87.0, 1.0, 100.0, 19.0]);
        assert!(out.contains("0.863 0.863 0.863 rg"), "{out}");
        assert!(
            out.contains("0 g 90.5 11.5 m 96.5 11.5 l 93.5 8.5 l h f"),
            "{out}"
        );
    }
}
