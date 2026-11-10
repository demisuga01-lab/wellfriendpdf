//! Synthetic bold and italic for substitute glyph outlines.
//!
//! When the face substituted for a non-embedded font lacks the requested
//! weight or slant, PDFium thickens the outline with FreeType's
//! `FT_Outline_EmboldenXY` and slants it with a shear in the glyph matrix
//! (`CFX_Face::RenderGlyph` / `LoadGlyphPath`). This module does the same to
//! an outline in font units, so the raster mask, stroked text, clipping text
//! and large outline-drawn glyphs all see one consistent shape. Advances are
//! untouched: text still advances by the PDF widths.

use crate::fonts::substitute_style::SyntheticStyle;
use crate::render::path::{Path, PathSegment};

/// One point of a contour and the role it plays in the path.
#[derive(Clone, Copy)]
enum PointRole {
    Start,
    Line,
    Control1,
    Control2,
    CurveEnd,
}

struct Contour {
    points: Vec<(f64, f64)>,
    roles: Vec<PointRole>,
    closed: bool,
}

/// Apply `style` to a glyph outline in font units (y up). `upem` is the
/// font's units per em; `vertical` slants along the vertical axis instead,
/// as PDFium does for vertical writing. Returns `None` when nothing changes.
pub(crate) fn apply_synthetic_style(
    path: &Path,
    style: SyntheticStyle,
    upem: f64,
    vertical: bool,
) -> Option<Path> {
    if style.is_none() || path.segments.is_empty() || !(upem.is_finite() && upem > 0.0) {
        return None;
    }
    let mut contours = split_contours(path);
    let shear = style.shear();
    if shear != 0.0 {
        for contour in &mut contours {
            for (x, y) in &mut contour.points {
                if vertical {
                    *y -= shear * *x;
                } else {
                    *x += shear * *y;
                }
            }
        }
    }
    let strength = style.embolden_fraction() * upem;
    if strength > 0.0 {
        embolden(&mut contours, strength);
    }
    Some(join_contours(&contours))
}

fn split_contours(path: &Path) -> Vec<Contour> {
    let mut contours = Vec::new();
    let mut current: Option<Contour> = None;
    for segment in &path.segments {
        match *segment {
            PathSegment::MoveTo(x, y) => {
                if let Some(contour) = current.take() {
                    contours.push(contour);
                }
                current = Some(Contour {
                    points: vec![(x, y)],
                    roles: vec![PointRole::Start],
                    closed: false,
                });
            }
            PathSegment::LineTo(x, y) => {
                let contour = current.get_or_insert_with(|| Contour {
                    points: vec![(x, y)],
                    roles: vec![PointRole::Start],
                    closed: false,
                });
                contour.points.push((x, y));
                contour.roles.push(PointRole::Line);
            }
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                let contour = current.get_or_insert_with(|| Contour {
                    points: vec![(cp1x, cp1y)],
                    roles: vec![PointRole::Start],
                    closed: false,
                });
                contour.points.extend([(cp1x, cp1y), (cp2x, cp2y), (x, y)]);
                contour.roles.extend([
                    PointRole::Control1,
                    PointRole::Control2,
                    PointRole::CurveEnd,
                ]);
            }
            PathSegment::ClosePath => {
                if let Some(mut contour) = current.take() {
                    contour.closed = true;
                    contours.push(contour);
                }
            }
        }
    }
    if let Some(contour) = current.take() {
        contours.push(contour);
    }
    contours
}

fn join_contours(contours: &[Contour]) -> Path {
    let mut path = Path::new();
    for contour in contours {
        let mut index = 0;
        while index < contour.points.len() {
            let (x, y) = contour.points[index];
            match contour.roles[index] {
                PointRole::Start => path.move_to(x, y),
                PointRole::Line => path.line_to(x, y),
                PointRole::Control1 if index + 2 < contour.points.len() => {
                    let (x2, y2) = contour.points[index + 1];
                    let (x3, y3) = contour.points[index + 2];
                    path.curve_to(x, y, x2, y2, x3, y3);
                    index += 2;
                }
                // A malformed curve tail: keep the point as a line.
                PointRole::Control1 | PointRole::Control2 | PointRole::CurveEnd => {
                    path.line_to(x, y)
                }
            }
            index += 1;
        }
        if contour.closed {
            path.close();
        }
    }
    path
}

/// Thicken every contour by `strength` font units in total, the way
/// `FT_Outline_EmboldenXY` does: each point moves outward along the bisector
/// of its two edges so both edges move by `strength / 2`, the move is capped
/// by the adjacent edge lengths so thin details do not explode, and the whole
/// outline then shifts by `strength / 2` right and up, so the left side bearing
/// and the baseline stay where they were and the glyph grows rightward and
/// upward.
fn embolden(contours: &mut [Contour], strength: f64) {
    let half = strength / 2.0;
    // Outer contours run clockwise (TrueType) or counter-clockwise
    // (PostScript); the sign of the total area says which, and so which side
    // of an edge is outside.
    let mut area = 0.0;
    for contour in contours.iter() {
        let points = &contour.points;
        for index in 0..points.len() {
            let (x0, y0) = points[index];
            let (x1, y1) = points[(index + 1) % points.len()];
            area += x0 * y1 - x1 * y0;
        }
    }
    if area == 0.0 || !area.is_finite() {
        return;
    }
    let clockwise = area < 0.0;

    for contour in contours.iter_mut() {
        let points = &contour.points;
        let count = points.len();
        let mut moved = Vec::with_capacity(count);
        for index in 0..count {
            let current = points[index];
            let previous = (1..count)
                .map(|step| points[(index + count - step) % count])
                .find(|point| *point != current);
            let next = (1..count)
                .map(|step| points[(index + step) % count])
                .find(|point| *point != current);
            let mut shift = (0.0, 0.0);
            if let (Some(previous), Some(next)) = (previous, next) {
                let incoming = (current.0 - previous.0, current.1 - previous.1);
                let outgoing = (next.0 - current.0, next.1 - current.1);
                let length_in = incoming.0.hypot(incoming.1);
                let length_out = outgoing.0.hypot(outgoing.1);
                let unit_in = (incoming.0 / length_in, incoming.1 / length_in);
                let unit_out = (outgoing.0 / length_out, outgoing.1 / length_out);
                let dot = unit_in.0 * unit_out.0 + unit_in.1 * unit_out.1;
                // Leave very sharp turns (over ~160 degrees) alone.
                if dot > -0.9375 {
                    let outward = |unit: (f64, f64)| {
                        if clockwise {
                            (-unit.1, unit.0)
                        } else {
                            (unit.1, -unit.0)
                        }
                    };
                    let normal_in = outward(unit_in);
                    let normal_out = outward(unit_out);
                    let bisector = (normal_in.0 + normal_out.0, normal_in.1 + normal_out.1);
                    let denominator = 1.0 + dot;
                    let mut turn = unit_out.0 * unit_in.1 - unit_out.1 * unit_in.0;
                    if clockwise {
                        turn = -turn;
                    }
                    let shortest = length_in.min(length_out);
                    let scale = if half * turn <= shortest * denominator {
                        half / denominator
                    } else {
                        shortest / turn
                    };
                    shift = (bisector.0 * scale, bisector.1 * scale);
                }
            }
            moved.push((current.0 + half + shift.0, current.1 + half + shift.1));
        }
        contour.points = moved;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::substitute_style::EmboldenTable;

    fn bounds(path: &Path) -> (f64, f64, f64, f64) {
        let mut bounds = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        let mut add = |x: f64, y: f64| {
            bounds.0 = bounds.0.min(x);
            bounds.1 = bounds.1.min(y);
            bounds.2 = bounds.2.max(x);
            bounds.3 = bounds.3.max(y);
        };
        for segment in &path.segments {
            match *segment {
                PathSegment::MoveTo(x, y) | PathSegment::LineTo(x, y) => add(x, y),
                PathSegment::CubicTo { x, y, .. } => add(x, y),
                PathSegment::ClosePath => {}
            }
        }
        bounds
    }

    /// A 100 x 1000 vertical stem, clockwise (TrueType) or not.
    fn stem(clockwise: bool) -> Path {
        let mut path = Path::new();
        let corners = [(0.0, 0.0), (0.0, 1000.0), (100.0, 1000.0), (100.0, 0.0)];
        let ordered: Vec<_> = if clockwise {
            corners.to_vec()
        } else {
            corners.iter().rev().copied().collect()
        };
        path.move_to(ordered[0].0, ordered[0].1);
        for (x, y) in &ordered[1..] {
            path.line_to(*x, *y);
        }
        path.close();
        path
    }

    const BOLD: SyntheticStyle = SyntheticStyle {
        weight: 700,
        skew: 0,
        table: EmboldenTable::Latin,
    };

    #[test]
    fn emboldening_grows_right_and_up_by_the_pdfium_strength() {
        for clockwise in [true, false] {
            let bold =
                apply_synthetic_style(&stem(clockwise), BOLD, 1000.0, false).expect("emboldened");
            let (min_x, min_y, max_x, max_y) = bounds(&bold);
            let strength = BOLD.embolden_fraction() * 1000.0;
            assert!((strength - 17.0).abs() < 0.5, "{strength}");
            // Left side bearing and baseline stay put.
            assert!(min_x.abs() < 1e-6, "{min_x}");
            assert!(min_y.abs() < 1e-6, "{min_y}");
            // The stem is `strength` wider and taller.
            assert!((max_x - (100.0 + strength)).abs() < 1e-6, "{max_x}");
            assert!((max_y - (1000.0 + strength)).abs() < 1e-6, "{max_y}");
        }
    }

    #[test]
    fn italic_shears_by_the_skew_and_keeps_the_baseline() {
        let italic = SyntheticStyle {
            weight: 0,
            skew: -21,
            table: EmboldenTable::Latin,
        };
        let slanted = apply_synthetic_style(&stem(true), italic, 1000.0, false).expect("sheared");
        let (min_x, min_y, max_x, max_y) = bounds(&slanted);
        assert!(min_x.abs() < 1e-9 && min_y.abs() < 1e-9);
        assert!((max_x - 310.0).abs() < 1e-6, "{max_x}");
        assert!((max_y - 1000.0).abs() < 1e-9);

        let vertical = apply_synthetic_style(&stem(true), italic, 1000.0, true).expect("sheared");
        let (_, min_y, max_x, _) = bounds(&vertical);
        assert!((max_x - 100.0).abs() < 1e-9);
        assert!((min_y + 21.0).abs() < 1e-6, "{min_y}");
    }

    #[test]
    fn curves_survive_and_no_style_is_a_no_op() {
        let mut path = Path::new();
        path.move_to(0.0, 0.0);
        path.curve_to(0.0, 500.0, 500.0, 500.0, 500.0, 0.0);
        path.close();
        let bold = apply_synthetic_style(&path, BOLD, 1000.0, false).expect("emboldened");
        assert_eq!(bold.segments.len(), path.segments.len());
        assert!(matches!(bold.segments[1], PathSegment::CubicTo { .. }));
        assert!(apply_synthetic_style(&path, SyntheticStyle::NONE, 1000.0, false).is_none());
    }
}
