//! PDFium's zero-area fill lines (`CFX_RenderDevice::DrawPath` and
//! `DrawZeroAreaPath`).
//!
//! A filled path that encloses no area paints nothing in a scanline filler,
//! yet PDFium keeps such fills visible: a two-point path is drawn as a
//! one-pixel line, and in every subpath the parts that only retrace themselves
//! (a palindromic point sequence, or a line that folds back over itself) are
//! stroked one pixel wide, at a quarter of the fill alpha when the subpath has
//! more than three points.
use crate::cancel::CancelToken;
use crate::content::state::{LineCap, LineJoin};
use crate::render::buffer::{PixelBuffer, PixelColor};
use crate::render::line::DashState;
use crate::render::path::{Path, PathPainter, PathSegment};
use crate::render::transform::{Transform2D, Viewport};
use crate::render::vector_quality::VectorRasterOptions;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Move,
    Line,
    Bezier,
}

/// A PDFium path point: single-precision position plus its segment kind.
#[derive(Clone, Copy, Debug)]
struct Point {
    x: f32,
    y: f32,
    kind: Kind,
}

impl Point {
    fn same(&self, other: &Point) -> bool {
        self.x == other.x && self.y == other.y
    }
}

/// Device-space line segments to stroke one pixel wide, and whether they are
/// drawn at a quarter of the fill alpha.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ZeroAreaLines {
    pub segments: Vec<[(f64, f64); 2]>,
    pub thin: bool,
}

/// What PDFium draws besides the fill itself for a fill-only path.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ZeroAreaPaint {
    /// A path of exactly a move and a line: a one-pixel line at full alpha
    /// (`DrawCosmeticLine`), instead of the fill.
    Cosmetic([(f64, f64); 2]),
    /// Lines for the zero-area parts of each subpath.
    Lines(Vec<ZeroAreaLines>),
}

/// The point list PDFium's content parser builds for `path`: a closepath adds
/// a line back to the subpath start unless the pen is already there, and a
/// trailing move is dropped.
fn points(path: &Path) -> Vec<Point> {
    let mut points: Vec<Point> = Vec::with_capacity(path.segments.len());
    let mut start = (0.0f32, 0.0f32);
    let mut current = (0.0f32, 0.0f32);
    let point = |(x, y): (f64, f64), kind| Point {
        x: x as f32,
        y: y as f32,
        kind,
    };
    for segment in &path.segments {
        match *segment {
            PathSegment::MoveTo(x, y) => {
                let p = point((x, y), Kind::Move);
                if points.last().is_some_and(|last| last.kind == Kind::Move) {
                    points.pop();
                }
                start = (p.x, p.y);
                current = start;
                points.push(p);
            }
            PathSegment::LineTo(x, y) => {
                let p = point((x, y), Kind::Line);
                current = (p.x, p.y);
                points.push(p);
            }
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                points.push(point((cp1x, cp1y), Kind::Bezier));
                points.push(point((cp2x, cp2y), Kind::Bezier));
                let p = point((x, y), Kind::Bezier);
                current = (p.x, p.y);
                points.push(p);
            }
            PathSegment::ClosePath => {
                if current != start {
                    points.push(Point {
                        x: start.0,
                        y: start.1,
                        kind: Kind::Line,
                    });
                    current = start;
                }
            }
        }
    }
    if points.last().is_some_and(|last| last.kind == Kind::Move) {
        points.pop();
    }
    points
}

/// PDFium's extra paint for a fill-only `path` whose user space maps to the
/// device through `to_device`, or `None` when there is nothing to add.
pub(crate) fn zero_area_paint(path: &Path, to_device: &Transform2D) -> Option<ZeroAreaPaint> {
    let points = points(path);
    let device = |p: &Point| to_device.transform_point(f64::from(p.x), f64::from(p.y));
    if points.len() == 2 && points[0].kind == Kind::Move && points[1].kind == Kind::Line {
        return Some(ZeroAreaPaint::Cosmetic([
            device(&points[0]),
            device(&points[1]),
        ]));
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for end in 1..=points.len() {
        if end == points.len() || points[end].kind == Kind::Move {
            if let Some(subpath) = subpath_lines(&points[start..end], to_device) {
                lines.push(subpath);
            }
            start = end;
        }
    }
    (!lines.is_empty()).then_some(ZeroAreaPaint::Lines(lines))
}

/// PDFium's `GetZeroAreaPath` for one subpath.
fn subpath_lines(points: &[Point], to_device: &Transform2D) -> Option<ZeroAreaLines> {
    if points.len() < 2 {
        return None;
    }
    let device = |p: &Point| to_device.transform_point(f64::from(p.x), f64::from(p.y));
    // CheckSimpleLinePath: A->B or A->B->A, snapped to pixel centres.
    if (points.len() == 2 || points.len() == 3)
        && points[0].kind == Kind::Move
        && points[1].kind == Kind::Line
        && (points.len() == 2 || (points[2].kind == Kind::Line && points[0].same(&points[2])))
    {
        if points[0].same(&points[1]) {
            return None;
        }
        let snap = |p: &Point| {
            let (x, y) = device(p);
            (
                (x as f32) as i32 as f64 + 0.5,
                (y as f32) as i32 as f64 + 0.5,
            )
        };
        return Some(ZeroAreaLines {
            segments: vec![[snap(&points[0]), snap(&points[1])]],
            thin: true,
        });
    }
    // CheckPalindromicPath.
    if points.len() > 3 && points.len() % 2 == 1 {
        let mid = points.len() / 2;
        let mut segments = Vec::with_capacity(mid);
        let mut palindromic = true;
        for i in 0..mid {
            let left = &points[mid - i - 1];
            let right = &points[mid + i + 1];
            if !(left.same(right) && left.kind != Kind::Bezier && right.kind != Kind::Bezier) {
                palindromic = false;
                break;
            }
            segments.push([device(&points[mid - i]), device(left)]);
        }
        if palindromic {
            return Some(ZeroAreaLines {
                segments,
                thin: true,
            });
        }
    }
    // A subpath with curves whose every point (control points included)
    // lies on one line encloses no area either. PDFium's checks above skip
    // curve points, so it only draws the parts that fold back over a line
    // (crbug.com/42270524, a PDFium bug: its expected image, like Poppler,
    // shows the whole path); the whole subpath is drawn instead.
    if points.iter().any(|p| p.kind == Kind::Bezier) && collinear(points) {
        return Some(ZeroAreaLines {
            segments: points
                .windows(2)
                .map(|pair| [device(&pair[0]), device(&pair[1])])
                .collect(),
            thin: points.len() > 3,
        });
    }
    // Lines that fold back onto the previous one.
    let mut segments = Vec::new();
    let mut i = 0;
    while i < points.len() {
        match points[i].kind {
            Kind::Move => {
                i += 1;
                continue;
            }
            Kind::Bezier => {
                i += 3;
                continue;
            }
            Kind::Line => {}
        }
        let next = &points[(i + 1) % points.len()];
        if next.kind != Kind::Line || i == 0 {
            i += 1;
            continue;
        }
        let (prev, cur) = (&points[i - 1], &points[i]);
        let vertical =
            prev.x == cur.x && cur.x == next.x && (cur.y - prev.y) * (cur.y - next.y) > 0.0;
        let horizontal =
            prev.y == cur.y && cur.y == next.y && (cur.x - prev.x) * (cur.x - next.x) > 0.0;
        let diagonal = prev.x != cur.x
            && next.x != cur.x
            && prev.y != cur.y
            && next.y != cur.y
            && (prev.y - cur.y) * (next.x - cur.x) == (next.y - cur.y) * (prev.x - cur.x);
        if vertical || horizontal || diagonal {
            let use_prev = if vertical {
                (cur.y - prev.y).abs() < (cur.y - next.y).abs()
            } else {
                (cur.x - prev.x).abs() < (cur.x - next.x).abs()
            };
            let (from, to) = if use_prev { (prev, cur) } else { (cur, next) };
            segments.push([device(from), device(to)]);
        }
        i += 1;
    }
    if segments.is_empty() {
        return None;
    }
    Some(ZeroAreaLines {
        thin: points.len() > 3,
        segments,
    })
}

/// Whether all points lie on one line through two distinct points.
fn collinear(points: &[Point]) -> bool {
    let Some(first) = points.first() else {
        return false;
    };
    let Some(other) = points.iter().find(|p| !p.same(first)) else {
        return false;
    };
    let (dx, dy) = (
        f64::from(other.x) - f64::from(first.x),
        f64::from(other.y) - f64::from(first.y),
    );
    let length = dx.hypot(dy);
    points.iter().all(|p| {
        let (px, py) = (
            f64::from(p.x) - f64::from(first.x),
            f64::from(p.y) - f64::from(first.y),
        );
        (dx * py - dy * px).abs() <= 1e-6 * length * px.hypot(py).max(1.0)
    })
}

/// PDFium's extra paint for a fill-only path in compat rendering (see
/// this module); nothing in high-quality mode.
pub(crate) fn compat_zero_area_paint(
    buf: &PixelBuffer,
    path: &Path,
    ctm: &Transform2D,
    viewport: &Viewport,
) -> Option<ZeroAreaPaint> {
    if buf.render_mode().is_high_quality() {
        return None;
    }
    zero_area_paint(path, &ctm.concat(&viewport.to_transform()))
}

/// Stroke the zero-area lines of each subpath, a quarter of the fill alpha
/// for thin ones.
pub(crate) fn paint_zero_area_lines(
    buf: &mut PixelBuffer,
    viewport: &Viewport,
    subpaths: &[ZeroAreaLines],
    color: PixelColor,
    binary_alpha: bool,
    cancel: &CancelToken,
) {
    for lines in subpaths {
        let mut line_color = color;
        if lines.thin {
            line_color[3] >>= 2;
        }
        stroke_device_hairlines(
            buf,
            viewport,
            &lines.segments,
            line_color,
            binary_alpha,
            cancel,
        );
    }
}

/// Stroke device-space segments one pixel wide (butt caps, no stroke
/// adjustment) as one paint.
pub(crate) fn stroke_device_hairlines(
    buf: &mut PixelBuffer,
    viewport: &Viewport,
    segments: &[[(f64, f64); 2]],
    color: PixelColor,
    binary_alpha: bool,
    cancel: &CancelToken,
) {
    if color[3] == 0 || !(viewport.scale > 0.0) {
        return;
    }
    let Some(device_to_page) = viewport.to_transform().inverse() else {
        return;
    };
    let mut path = Path::new();
    for [from, to] in segments {
        let (x0, y0) = device_to_page.transform_point(from.0, from.1);
        let (x1, y1) = device_to_page.transform_point(to.0, to.1);
        path.move_to(x0, y0);
        path.line_to(x1, y1);
    }
    PathPainter::stroke_path_with_options(
        buf,
        &path,
        &Transform2D::identity(),
        viewport,
        color,
        1.0 / viewport.scale,
        &DashState::solid(),
        &LineCap::Butt,
        &LineJoin::Miter,
        10.0,
        0.25,
        binary_alpha,
        VectorRasterOptions::for_glyph_stroke(),
        cancel,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(ops: &[(&str, &[f64])]) -> Path {
        let mut path = Path::new();
        for (op, v) in ops {
            match *op {
                "m" => path.move_to(v[0], v[1]),
                "l" => path.line_to(v[0], v[1]),
                "c" => path.curve_to(v[0], v[1], v[2], v[3], v[4], v[5]),
                _ => path.close(),
            }
        }
        path
    }

    #[test]
    fn collinear_subpaths_with_curves_are_drawn_whole() {
        // bug_42270524: 0,0 -> 130,130, a straight curve to 200,200, then
        // lines to 400,400 and back and forth between 300 and 400.
        let paint = zero_area_paint(
            &path(&[
                ("m", &[0.0, 0.0]),
                ("l", &[130.0, 130.0]),
                ("c", &[150.0, 150.0, 180.0, 180.0, 200.0, 200.0]),
                ("l", &[400.0, 400.0]),
                ("l", &[300.0, 300.0]),
                ("l", &[400.0, 400.0]),
                ("l", &[300.0, 300.0]),
            ]),
            &Transform2D::identity(),
        );
        let Some(ZeroAreaPaint::Lines(lines)) = paint else {
            panic!("expected zero-area lines: {paint:?}");
        };
        assert_eq!(lines.len(), 1);
        assert!(lines[0].thin);
        assert_eq!(
            lines[0].segments.first(),
            Some(&[(0.0, 0.0), (130.0, 130.0)])
        );
        assert_eq!(lines[0].segments.len(), 8);
        // A curve that bends is not zero-area.
        let bent = zero_area_paint(
            &path(&[
                ("m", &[0.0, 0.0]),
                ("c", &[0.0, 50.0, 50.0, 50.0, 50.0, 0.0]),
                ("l", &[0.0, 0.0]),
            ]),
            &Transform2D::identity(),
        );
        assert_eq!(bent, None);
    }

    #[test]
    fn two_point_paths_are_cosmetic_lines() {
        let paint = zero_area_paint(
            &path(&[("m", &[0.0, 0.0]), ("l", &[4.0, 4.0])]),
            &Transform2D::identity(),
        );
        assert_eq!(
            paint,
            Some(ZeroAreaPaint::Cosmetic([(0.0, 0.0), (4.0, 4.0)]))
        );
    }

    #[test]
    fn retraced_lines_are_snapped_thin_lines() {
        let paint = zero_area_paint(
            &path(&[("m", &[0.2, 0.7]), ("l", &[4.9, 0.7]), ("l", &[0.2, 0.7])]),
            &Transform2D::identity(),
        );
        let Some(ZeroAreaPaint::Lines(lines)) = paint else {
            panic!("{paint:?}");
        };
        assert_eq!(lines[0].segments, vec![[(0.5, 0.5), (4.5, 0.5)]]);
        assert!(lines[0].thin);
    }

    #[test]
    fn folding_lines_and_palindromes_follow_pdfium() {
        // A three-point fold is drawn at full alpha.
        let Some(ZeroAreaPaint::Lines(lines)) = zero_area_paint(
            &path(&[
                ("m", &[0.0, 0.0]),
                ("l", &[400.0, 400.0]),
                ("l", &[200.0, 200.0]),
            ]),
            &Transform2D::identity(),
        ) else {
            panic!()
        };
        assert_eq!(lines[0].segments, vec![[(400.0, 400.0), (200.0, 200.0)]]);
        assert!(!lines[0].thin);
        // m A l B l C l B l A is palindromic around C.
        let Some(ZeroAreaPaint::Lines(lines)) = zero_area_paint(
            &path(&[
                ("m", &[0.0, 0.0]),
                ("l", &[5.0, 0.0]),
                ("l", &[5.0, 5.0]),
                ("l", &[5.0, 0.0]),
                ("l", &[0.0, 0.0]),
            ]),
            &Transform2D::identity(),
        ) else {
            panic!()
        };
        assert_eq!(
            lines[0].segments,
            vec![[(5.0, 5.0), (5.0, 0.0)], [(5.0, 0.0), (0.0, 0.0)]]
        );
        assert!(lines[0].thin);
        // A closed triangle has area and adds nothing.
        assert_eq!(
            zero_area_paint(
                &path(&[
                    ("m", &[0.0, 0.0]),
                    ("l", &[5.0, 0.0]),
                    ("l", &[5.0, 5.0]),
                    ("h", &[])
                ]),
                &Transform2D::identity(),
            ),
            None
        );
    }
}
