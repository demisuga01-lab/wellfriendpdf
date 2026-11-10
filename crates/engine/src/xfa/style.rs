//! XFA template presentation properties: fonts, paragraphs, margins,
//! borders (edges, corners), fills and captions, resolved to plain values
//! with the XFA defaults PDFium uses.

use super::form::Template;
use super::xml::XmlNode;

pub(crate) type Rgb = [u8; 3];

pub(crate) const BLACK: Rgb = [0, 0, 0];
pub(crate) const WHITE: Rgb = [255, 255, 255];

/// `"r,g,b"` (0-255) colour values.
pub(crate) fn parse_color(value: &str) -> Option<Rgb> {
    let mut parts = value
        .split([',', ' '])
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            part.trim()
                .parse::<f64>()
                .ok()
                .map(|v| v.clamp(0.0, 255.0) as u8)
        });
    let r = parts.next()??;
    let g = parts.next()??;
    let b = parts.next()??;
    Some([r, g, b])
}

fn color_of<'t>(tpl: &Template<'t>, node: Option<&'t XmlNode>) -> Option<Rgb> {
    let color = tpl.child(node?, "color")?;
    tpl.attr(color, "value").and_then(parse_color)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FontSpec {
    pub typeface: String,
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub line_through: bool,
    pub color: Rgb,
    pub baseline_shift: f64,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            typeface: "Courier".to_string(),
            size: 10.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: BLACK,
            baseline_shift: 0.0,
        }
    }
}

impl FontSpec {
    /// Reads a `<font>` element over `base` defaults.
    pub(crate) fn from_node<'t>(
        tpl: &Template<'t>,
        node: Option<&'t XmlNode>,
        base: &Self,
    ) -> Self {
        let mut spec = base.clone();
        let Some(font) = node else {
            return spec;
        };
        if let Some(typeface) = tpl.attr(font, "typeface") {
            if !typeface.trim().is_empty() {
                spec.typeface = typeface.trim().to_string();
            }
        }
        if let Some(size) = tpl.measure(font, "size") {
            if size > 0.0 && size < 2000.0 {
                spec.size = size;
            }
        }
        if let Some(weight) = tpl.attr(font, "weight") {
            spec.bold = weight == "bold";
        }
        if let Some(posture) = tpl.attr(font, "posture") {
            spec.italic = posture == "italic";
        }
        if let Some(underline) = tpl.attr(font, "underline") {
            spec.underline = underline.trim() != "0";
        }
        if let Some(line_through) = tpl.attr(font, "lineThrough") {
            spec.line_through = line_through.trim() != "0";
        }
        if let Some(shift) = tpl.measure(font, "baselineShift") {
            spec.baseline_shift = shift;
        }
        if let Some(fill) = tpl.child(font, "fill") {
            if let Some(color) = color_of(tpl, Some(fill)) {
                spec.color = color;
            }
        }
        spec
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HAlign {
    Left,
    Center,
    Right,
    Justify,
    JustifyAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ParaSpec {
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub margin_left: f64,
    pub margin_right: f64,
    pub space_above: f64,
    pub space_below: f64,
    pub text_indent: f64,
    pub line_height: f64,
}

impl Default for ParaSpec {
    fn default() -> Self {
        Self {
            h_align: HAlign::Left,
            v_align: VAlign::Top,
            margin_left: 0.0,
            margin_right: 0.0,
            space_above: 0.0,
            space_below: 0.0,
            text_indent: 0.0,
            line_height: 0.0,
        }
    }
}

pub(crate) fn parse_h_align(value: &str) -> Option<HAlign> {
    Some(match value.trim() {
        "left" | "radix" => HAlign::Left,
        "center" => HAlign::Center,
        "right" => HAlign::Right,
        "justify" => HAlign::Justify,
        "justifyAll" => HAlign::JustifyAll,
        _ => return None,
    })
}

impl ParaSpec {
    pub(crate) fn from_node<'t>(tpl: &Template<'t>, node: Option<&'t XmlNode>) -> Self {
        let mut spec = Self::default();
        let Some(para) = node else {
            return spec;
        };
        if let Some(align) = tpl.attr(para, "hAlign").and_then(parse_h_align) {
            spec.h_align = align;
        }
        spec.v_align = match tpl.attr(para, "vAlign") {
            Some("middle") => VAlign::Middle,
            Some("bottom") => VAlign::Bottom,
            _ => VAlign::Top,
        };
        let m = |name: &str| tpl.measure(para, name).unwrap_or(0.0);
        spec.margin_left = m("marginLeft");
        spec.margin_right = m("marginRight");
        spec.space_above = m("spaceAbove").max(0.0);
        spec.space_below = m("spaceBelow").max(0.0);
        spec.text_indent = m("textIndent");
        spec.line_height = m("lineHeight").max(0.0);
        spec
    }
}

/// Insets in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Insets {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Insets {
    pub(crate) fn from_margin<'t>(tpl: &Template<'t>, margin: Option<&'t XmlNode>) -> Self {
        let Some(margin) = margin else {
            return Self::default();
        };
        let m = |name: &str| tpl.measure(margin, name).unwrap_or(0.0);
        Self {
            left: m("leftInset"),
            top: m("topInset"),
            right: m("rightInset"),
            bottom: m("bottomInset"),
        }
    }

    /// Explicit insets only (`None` where the attribute is absent).
    pub(crate) fn explicit<'t>(tpl: &Template<'t>, margin: &'t XmlNode) -> [Option<f64>; 4] {
        [
            tpl.measure(margin, "leftInset"),
            tpl.measure(margin, "topInset"),
            tpl.measure(margin, "rightInset"),
            tpl.measure(margin, "bottomInset"),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrokeType {
    Solid,
    Dashed,
    Dotted,
    DashDot,
    DashDotDot,
    Lowered,
    Raised,
    Etched,
    Embossed,
}

impl StrokeType {
    fn parse(value: Option<&str>) -> Self {
        match value {
            Some("dashed") => Self::Dashed,
            Some("dotted") => Self::Dotted,
            Some("dashDot") => Self::DashDot,
            Some("dashDotDot") => Self::DashDotDot,
            Some("lowered") => Self::Lowered,
            Some("raised") => Self::Raised,
            Some("etched") => Self::Etched,
            Some("embossed") => Self::Embossed,
            _ => Self::Solid,
        }
    }

    pub(crate) fn is_3d(self) -> bool {
        matches!(
            self,
            Self::Lowered | Self::Raised | Self::Etched | Self::Embossed
        )
    }

    /// PDFium `XFA_StrokeTypeSetLineDash` with a butt cap.
    pub(crate) fn dash(self) -> Option<&'static [f64]> {
        match self {
            Self::DashDot => Some(&[4.0, 1.0, 2.0, 1.0]),
            Self::DashDotDot => Some(&[4.0, 1.0, 2.0, 1.0, 2.0, 1.0]),
            Self::Dashed => Some(&[5.0, 1.0]),
            Self::Dotted => Some(&[2.0, 1.0]),
            _ => None,
        }
    }
}

/// An `<edge>` or `<corner>`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stroke {
    pub visible: bool,
    pub thickness: f64,
    pub color: Rgb,
    pub stroke: StrokeType,
    pub round: bool,
    pub inverted: bool,
    pub radius: f64,
    pub cap: CapType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapType {
    Butt,
    Round,
    Square,
}

impl Default for Stroke {
    fn default() -> Self {
        Self {
            visible: true,
            thickness: 0.5,
            color: BLACK,
            stroke: StrokeType::Solid,
            round: false,
            inverted: false,
            radius: 0.0,
            cap: CapType::Square,
        }
    }
}

impl Stroke {
    pub(crate) fn from_node<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> Self {
        let mut stroke = Self {
            visible: !matches!(
                tpl.attr(node, "presence"),
                Some("hidden" | "invisible" | "inactive")
            ),
            ..Self::default()
        };
        if let Some(thickness) = tpl.measure(node, "thickness") {
            stroke.thickness = thickness.max(0.0);
        }
        if let Some(color) = color_of(tpl, Some(node)) {
            stroke.color = color;
        }
        stroke.stroke = StrokeType::parse(tpl.attr(node, "stroke"));
        stroke.round = tpl.attr(node, "join") == Some("round");
        stroke.inverted = tpl.attr(node, "inverted") == Some("1");
        stroke.radius = tpl.measure(node, "radius").unwrap_or(0.0);
        stroke.cap = match tpl.attr(node, "cap") {
            Some("butt") => CapType::Butt,
            Some("round") => CapType::Round,
            _ => CapType::Square,
        };
        stroke
    }

    /// PDFium `CXFA_Stroke::SameStyles` (optionally comparing radius).
    pub(crate) fn same_style(&self, other: &Self, ignore_presence: bool, corner: bool) -> bool {
        (self.thickness - other.thickness).abs() < 0.01
            && (ignore_presence || self.visible == other.visible)
            && self.stroke == other.stroke
            && self.color == other.color
            && (!corner || (self.radius - other.radius).abs() < 0.01)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinearDir {
    Right,
    Bottom,
    Left,
    Top,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FillKind {
    Solid,
    Linear { dir: LinearDir, end: Rgb },
    Radial { to_edge: bool, end: Rgb },
    Pattern { kind: PatternKind, color: Rgb },
    Stipple { rate: f64, color: Rgb },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatternKind {
    CrossHatch,
    CrossDiagonal,
    DiagonalLeft,
    DiagonalRight,
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FillSpec {
    pub color: Rgb,
    pub kind: FillKind,
}

impl FillSpec {
    pub(crate) fn from_node<'t>(tpl: &Template<'t>, fill: &'t XmlNode) -> Option<Self> {
        if matches!(
            tpl.attr(fill, "presence"),
            Some("hidden" | "invisible" | "inactive")
        ) {
            return None;
        }
        let color = color_of(tpl, Some(fill)).unwrap_or(WHITE);
        let kind_node = fill
            .children
            .iter()
            .find(|child| !matches!(child.local_name.as_str(), "color" | "extras"));
        let kind = match kind_node.map(|node| node.local_name.as_str()) {
            Some("linear") => {
                let node = kind_node?;
                FillKind::Linear {
                    dir: match tpl.attr(node, "type") {
                        Some("toBottom") => LinearDir::Bottom,
                        Some("toLeft") => LinearDir::Left,
                        Some("toTop") => LinearDir::Top,
                        _ => LinearDir::Right,
                    },
                    end: color_of(tpl, Some(node)).unwrap_or(BLACK),
                }
            }
            Some("radial") => {
                let node = kind_node?;
                FillKind::Radial {
                    to_edge: tpl.attr(node, "type") != Some("toCenter"),
                    end: color_of(tpl, Some(node)).unwrap_or(BLACK),
                }
            }
            Some("pattern") => {
                let node = kind_node?;
                FillKind::Pattern {
                    kind: match tpl.attr(node, "type") {
                        Some("crossDiagonal") => PatternKind::CrossDiagonal,
                        Some("diagonalLeft") => PatternKind::DiagonalLeft,
                        Some("diagonalRight") => PatternKind::DiagonalRight,
                        Some("horizontal") => PatternKind::Horizontal,
                        Some("vertical") => PatternKind::Vertical,
                        _ => PatternKind::CrossHatch,
                    },
                    color: color_of(tpl, Some(node)).unwrap_or(BLACK),
                }
            }
            Some("stipple") => {
                let node = kind_node?;
                FillKind::Stipple {
                    rate: tpl
                        .attr(node, "rate")
                        .and_then(|rate| rate.trim().parse::<f64>().ok())
                        .unwrap_or(50.0)
                        .clamp(0.0, 100.0),
                    color: color_of(tpl, Some(node)).unwrap_or(BLACK),
                }
            }
            _ => FillKind::Solid,
        };
        Some(Self { color, kind })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hand {
    Even,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BoxKind {
    Border,
    Rectangle,
    Arc,
}

/// A `<border>`, `<rectangle>` or `<arc>`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BoxSpec {
    pub kind: BoxKind,
    pub hand: Hand,
    /// PDFium's 8-entry stroke list: corner0, edge0 (top), corner1, edge1
    /// (right), corner2, edge2 (bottom), corner3, edge3 (left).
    pub strokes: [Stroke; 8],
    pub fill: Option<FillSpec>,
    pub circular: bool,
    pub start_angle: Option<f64>,
    pub sweep_angle: Option<f64>,
    pub margin: Insets,
}

impl BoxSpec {
    pub(crate) fn from_node<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> Option<Self> {
        if matches!(
            tpl.attr(node, "presence"),
            Some("hidden" | "invisible" | "inactive")
        ) {
            return None;
        }
        let kind = match node.local_name.as_str() {
            "rectangle" => BoxKind::Rectangle,
            "arc" => BoxKind::Arc,
            _ => BoxKind::Border,
        };
        let hand = match tpl.attr(node, "hand") {
            Some("left") => Hand::Left,
            Some("right") => Hand::Right,
            _ => Hand::Even,
        };
        let edges: Vec<&XmlNode> = node
            .children
            .iter()
            .filter(|child| child.local_name == "edge")
            .collect();
        let corners: Vec<&XmlNode> = node
            .children
            .iter()
            .filter(|child| child.local_name == "corner")
            .collect();
        let edge0 = tpl
            .child(node, "edge")
            .map(|edge| Stroke::from_node(tpl, edge))
            .unwrap_or_default();
        let corner0 = tpl
            .child(node, "corner")
            .map(|corner| Stroke::from_node(tpl, corner))
            .unwrap_or_default();
        let edge = |index: usize| edges.get(index).map(|edge| Stroke::from_node(tpl, edge));
        let corner = |index: usize| {
            corners
                .get(index)
                .map(|corner| Stroke::from_node(tpl, corner))
        };
        // GetStrokesInternal: missing edges/corners 1 and 2 copy 0; 3 copies 1.
        let e1 = edge(1).unwrap_or(edge0);
        let e2 = edge(2).unwrap_or(edge0);
        let e3 = edge(3).unwrap_or(e1);
        let c1 = corner(1).unwrap_or(corner0);
        let c2 = corner(2).unwrap_or(corner0);
        let c3 = corner(3).unwrap_or(c1);
        let fill = tpl
            .child(node, "fill")
            .and_then(|fill| FillSpec::from_node(tpl, fill));
        let angle = |name: &str| {
            tpl.attr(node, name)
                .and_then(|value| value.trim().parse::<f64>().ok())
                .filter(|value| value.is_finite())
        };
        Some(Self {
            kind,
            hand,
            strokes: [corner0, edge0, c1, e1, c2, e2, c3, e3],
            fill,
            circular: tpl.attr(node, "circular") == Some("1"),
            start_angle: angle("startAngle"),
            sweep_angle: angle("sweepAngle"),
            margin: Insets::from_margin(tpl, tpl.child(node, "margin")),
        })
    }

    pub(crate) fn edge(&self, index: usize) -> &Stroke {
        &self.strokes[index * 2 + 1]
    }

    /// PDFium `Get3DStyle`: the 3-D stroke type, visibility and thickness.
    pub(crate) fn style_3d(&self) -> Option<(StrokeType, bool, f64)> {
        if self.kind == BoxKind::Arc {
            return None;
        }
        // Style3D picks the first edge (index 1) and the next differing one.
        let mut stroke = self.strokes[1];
        if self.strokes[3].stroke != stroke.stroke {
            stroke = self.strokes[3];
        }
        stroke
            .stroke
            .is_3d()
            .then_some((stroke.stroke, stroke.visible, stroke.thickness))
    }

    /// PDFium `CXFA_Node::GetUIMargin` edge contribution for one side.
    pub(crate) fn edge_inset(&self, side: usize, three_d: bool) -> f64 {
        let stroke = self.edge(side);
        if !stroke.visible {
            return 0.0;
        }
        let mut inset = if side == 0 { 2.5 } else { 0.0 };
        inset += stroke.thickness * if three_d { 4.0 } else { 2.0 };
        inset
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptionPlacement {
    Left,
    Top,
    Right,
    Bottom,
    Inline,
}

/// Node presence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    Visible,
    Invisible,
    Hidden,
}

pub(crate) fn presence<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> Presence {
    match tpl.attr(node, "presence") {
        Some("invisible") => Presence::Invisible,
        Some("hidden" | "inactive") => Presence::Hidden,
        _ => Presence::Visible,
    }
}

/// Rotation in degrees, normalised to 0/90/180/270.
pub(crate) fn rotation<'t>(tpl: &Template<'t>, node: &'t XmlNode) -> i32 {
    let degrees = tpl
        .attr(node, "rotate")
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(0.0);
    let snapped = ((degrees / 90.0).round() as i64).rem_euclid(4) as i32;
    snapped * 90
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::xml::parse_xml;
    use crate::xfa::XfaLimits;

    #[test]
    fn border_strokes_inherit_like_pdfium() {
        let node = parse_xml(
            br#"<border hand="right"><edge thickness="2pt"><color value="255,0,0"/></edge><edge presence="hidden"/><corner radius="3pt" join="round"/><fill><linear type="toBottom"><color value="0,0,255"/></linear></fill></border>"#,
            &XfaLimits::default(),
        )
        .unwrap()
        .root;
        let tpl = Template::new(&node);
        let spec = BoxSpec::from_node(&tpl, &node).unwrap();
        assert_eq!(spec.hand, Hand::Right);
        assert_eq!(spec.edge(0).color, [255, 0, 0]);
        assert_eq!(spec.edge(0).thickness, 2.0);
        assert!(!spec.edge(1).visible);
        assert!(spec.edge(2).visible, "edge 2 copies edge 0");
        assert!(!spec.edge(3).visible, "edge 3 copies edge 1");
        assert!(spec.strokes[4].round);
        assert_eq!(spec.fill.unwrap().color, WHITE);
        assert_eq!(
            spec.fill.unwrap().kind,
            FillKind::Linear {
                dir: LinearDir::Bottom,
                end: [0, 0, 255]
            }
        );
        assert_eq!(parse_color("10, 20,30"), Some([10, 20, 30]));
    }
}
