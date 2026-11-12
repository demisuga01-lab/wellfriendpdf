//! PDFium-compatible pixel snapping of filled rectangles
//! ([`RectSnapping::Pdfium`](crate::render::RectSnapping::Pdfium)).
//!
//! PDFium never anti-aliases a fill-only path that is a single axis-aligned
//! rectangle. `CFX_RenderDevice::DrawPath` (core/fxge/cfx_renderdevice.cpp)
//! takes the rectangle fast path whenever the stroke alpha is zero and the
//! fill options do not ask for `rect_aa` (`CPDF_RenderOptions::bRectAA`, off
//! except for uncached Type3 glyph forms in
//! `CPDF_RenderStatus::ProcessType3Text`):
//!
//! 1. `CFX_Path::GetRect(&path_matrix)` (core/fxge/cfx_path.cpp) accepts the
//!    point list when it is a rectangle — `IsRectPreTransform` on the
//!    (optionally normalized) points, then every transformed edge must keep x
//!    or y within `FXSYS_IsFloatEqual`'s 0.0001 — and returns the device
//!    bounds as a float rect.
//! 2. `GetOuterRect` takes the enclosing whole-pixel rectangle, the pixel
//!    width/height are clamped to `ceil(float extent)` (at least one pixel),
//!    and when the outer rectangle is one pixel wider (taller) than that, the
//!    column (row) on the side the rectangle covers less is dropped.
//! 3. `FillRect` composites that pixel rectangle with the fill colour,
//!    without anti-aliasing (`CFX_AggDeviceDriver::FillRect` honours the clip
//!    mask only).
//!
//! The result is that a rectangle edge always lands on a pixel boundary, so
//! bar charts and table cells never show the half-covered seams the
//! anti-aliasing rasterizer leaves between abutting fills; abutting edges
//! that round to the same pixel overlap by at most that pixel, exactly as in
//! PDFium.
//!
//! The point list is rebuilt from the retained [`Path`] the way
//! `CPDF_StreamContentParser` builds it (`AddPathPoint`, `Handle_ClosePath`,
//! `AddPathRect`), so `re`, `m l l l h` and `m l l l l` rectangles behave as
//! they do in PDFium, while open four-point paths and multi-subpath fills
//! keep the general rasterizer.

use crate::render::path::{Path, PathSegment};
use crate::render::transform::Transform2D;

/// `FXSYS_IsFloatZero`'s tolerance.
const FLOAT_EPSILON: f64 = 0.0001;

/// Device coordinates beyond this stay with the general rasterizer, keeping
/// the pixel arithmetic far inside `i32` (`FX_RECT::Valid`).
const COORDINATE_LIMIT: f32 = (1u32 << 30) as f32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointType {
    Move,
    Line,
    Bezier,
}

#[derive(Clone, Copy, Debug)]
struct Point {
    x: f32,
    y: f32,
    kind: PointType,
    close: bool,
}

/// `FXSYS_IsFloatEqual`: the float difference against a double epsilon.
fn float_equal(a: f32, b: f32) -> bool {
    let difference = f64::from(a - b);
    difference < FLOAT_EPSILON && difference > -FLOAT_EPSILON
}

fn points_float_equal(a: &Point, b: &Point) -> bool {
    float_equal(a.x, b.x) && float_equal(a.y, b.y)
}

fn xy_both_not_equal(a: (f32, f32), b: (f32, f32)) -> bool {
    !float_equal(a.0, b.0) && !float_equal(a.1, b.1)
}

/// The most points a rectangle can arrive with after normalization drops
/// collinear and repeated vertices; longer paths are not worth scanning.
const MAX_RECT_SOURCE_POINTS: usize = 64;

/// Rebuild PDFium's `CFX_Path` point list for `path`
/// (`CPDF_StreamContentParser::AddPathPoint` / `Handle_ClosePath`).
fn pdfium_points(path: &Path) -> Option<Vec<Point>> {
    if path.segments.len() > MAX_RECT_SOURCE_POINTS {
        return None;
    }
    let mut points: Vec<Point> = Vec::with_capacity(path.segments.len() + 1);
    let mut current = (0.0f32, 0.0f32);
    let mut start = (0.0f32, 0.0f32);
    let add = |points: &mut Vec<Point>,
               current: &mut (f32, f32),
               start: &mut (f32, f32),
               (x, y): (f32, f32),
               kind: PointType| {
        if kind == PointType::Move {
            if let Some(last) = points.last() {
                if !last.close && last.kind == PointType::Move && *current == (x, y) {
                    return;
                }
            }
        }
        *current = (x, y);
        if kind == PointType::Move {
            *start = (x, y);
            if let Some(last) = points.last_mut() {
                if last.kind == PointType::Move && !last.close {
                    last.x = x;
                    last.y = y;
                    return;
                }
            }
        } else if points.is_empty() {
            return;
        }
        points.push(Point {
            x,
            y,
            kind,
            close: false,
        });
    };
    for segment in &path.segments {
        match *segment {
            PathSegment::MoveTo(x, y) => add(
                &mut points,
                &mut current,
                &mut start,
                (x as f32, y as f32),
                PointType::Move,
            ),
            PathSegment::LineTo(x, y) => add(
                &mut points,
                &mut current,
                &mut start,
                (x as f32, y as f32),
                PointType::Line,
            ),
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                for point in [(cp1x, cp1y), (cp2x, cp2y), (x, y)].map(|(x, y)| (x as f32, y as f32))
                {
                    add(
                        &mut points,
                        &mut current,
                        &mut start,
                        point,
                        PointType::Bezier,
                    );
                }
            }
            PathSegment::ClosePath => {
                if points.is_empty() {
                    continue;
                }
                if start != current {
                    current = start;
                    points.push(Point {
                        x: start.0,
                        y: start.1,
                        kind: PointType::Line,
                        close: true,
                    });
                } else if let Some(last) = points.last_mut() {
                    last.close = true;
                }
            }
        }
    }
    Some(points)
}

/// `PathPointsNeedNormalization`.
fn needs_normalization(points: &[Point]) -> bool {
    points.len() > 5 || (points.len() == 5 && !points_float_equal(&points[0], &points[4]))
}

/// `IsBetweenOnAxisAlignedLine`.
fn between_on_axis_aligned_line(prev: &Point, point: &Point, next: &Point) -> bool {
    if float_equal(prev.y, point.y) && float_equal(point.y, next.y) {
        return (prev.x < point.x && point.x < next.x) || (prev.x > point.x && point.x > next.x);
    }
    if float_equal(prev.x, point.x) && float_equal(point.x, next.x) {
        return (prev.y < point.y && point.y < next.y) || (prev.y > point.y && point.y > next.y);
    }
    false
}

/// `GetNormalizedPoints`: drop repeated vertices and vertices in the middle
/// of an axis-aligned edge. An empty result means "not a rectangle".
fn normalized_points(points: &[Point]) -> Vec<Point> {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return Vec::new();
    };
    let expected = if points_float_equal(first, last) {
        5
    } else {
        4
    };
    if expected == 4 && !last.close {
        return Vec::new();
    }
    let mut normalized: Vec<Point> = Vec::with_capacity(expected);
    normalized.push(*first);
    let mut index = 1;
    while index < points.len() {
        if normalized.len() + (points.len() - index) == expected {
            normalized.extend_from_slice(&points[index..]);
            break;
        }
        let point = points[index];
        let back = *normalized.last().expect("normalized starts non-empty");
        if point.kind == PointType::Line && !back.close {
            let next = points.get(index + 1);
            if !point.close {
                if points_float_equal(&point, &back) {
                    index += 1;
                    continue;
                }
                if let Some(next) = next {
                    if next.kind == PointType::Line
                        && between_on_axis_aligned_line(&back, &point, next)
                    {
                        index += 1;
                        continue;
                    }
                }
            } else if next.is_none()
                && (points_float_equal(&point, &back)
                    || between_on_axis_aligned_line(&back, &point, first))
            {
                normalized.last_mut().expect("non-empty").close = true;
                index += 1;
                continue;
            }
        }
        if normalized.len() >= expected {
            return Vec::new();
        }
        normalized.push(point);
        index += 1;
    }
    if normalized.len() != expected {
        return Vec::new();
    }
    normalized
}

/// `IsRectPreTransform`.
fn is_rect_pre_transform(points: &[Point]) -> bool {
    if points.len() != 5 && points.len() != 4 {
        return false;
    }
    if points.len() == 4 && !points[3].close {
        return false;
    }
    if points.len() == 5 && !points_float_equal(&points[0], &points[4]) {
        return false;
    }
    if points_float_equal(&points[0], &points[2]) || points_float_equal(&points[1], &points[3]) {
        return false;
    }
    points[1..]
        .iter()
        .all(|point| point.kind == PointType::Line)
}

/// `CFX_Path::GetRect(matrix)`: the device-space float bounds
/// `(left, bottom, right, top)` (with `bottom <= top`) of a rectangle path.
fn pdfium_device_rect(path: &Path, device_t: &Transform2D) -> Option<(f32, f32, f32, f32)> {
    let points = pdfium_points(path)?;
    let points = if needs_normalization(&points) {
        normalized_points(&points)
    } else {
        points
    };
    if !is_rect_pre_transform(&points) {
        return None;
    }
    // Single precision throughout, like PDFium's CFX_PointF / CFX_Matrix, so
    // an edge a rounding error away from a pixel boundary snaps the same way.
    let [a, b, c, d, e, f] = [
        device_t.a, device_t.b, device_t.c, device_t.d, device_t.e, device_t.f,
    ]
    .map(|value| value as f32);
    let mut device = [(0.0f32, 0.0f32); 5];
    for (index, point) in points.iter().enumerate() {
        // CFX_Matrix::Transform.
        let transformed = (a * point.x + c * point.y + e, b * point.x + d * point.y + f);
        if !transformed.0.is_finite()
            || !transformed.1.is_finite()
            || transformed.0.abs() > COORDINATE_LIMIT
            || transformed.1.abs() > COORDINATE_LIMIT
        {
            return None;
        }
        device[index] = transformed;
        if index > 0 && xy_both_not_equal(device[index], device[index - 1]) {
            return None;
        }
    }
    if xy_both_not_equal(device[0], device[3]) {
        return None;
    }
    let (p0, p2) = (device[0], device[2]);
    Some((
        p0.0.min(p2.0),
        p0.1.min(p2.1),
        p0.0.max(p2.0),
        p0.1.max(p2.1),
    ))
}

/// The whole-pixel rectangle `(x, y, width, height)` PDFium fills for a
/// fill-only rectangle path under `device_t`, or `None` when PDFium would
/// rasterize the path normally (`CFX_RenderDevice::DrawPath`).
pub(crate) fn pdfium_fill_rect(
    path: &Path,
    device_t: &Transform2D,
) -> Option<(i32, i32, i32, i32)> {
    let (left, bottom, right, top) = pdfium_device_rect(path, device_t)?;
    // FX_RECT from CFX_FloatRect::GetOuterRect (device y grows downwards:
    // the float rect's `bottom` is the smaller y).
    let mut outer_left = left.floor() as i64;
    let mut outer_right = right.ceil() as i64;
    let mut outer_top = bottom.floor() as i64;
    let mut outer_bottom = top.ceil() as i64;

    let mut width = (right - left).ceil() as i64;
    if width < 1 {
        width = 1;
        if outer_left == outer_right {
            outer_right += 1;
        }
    }
    let mut height = (top - bottom).ceil() as i64;
    if height < 1 {
        height = 1;
        if outer_top == outer_bottom {
            outer_bottom += 1;
        }
    }
    if outer_right - outer_left > width {
        if left - outer_left as f32 > outer_right as f32 - right {
            outer_left += 1;
        } else {
            outer_right -= 1;
        }
    }
    if outer_bottom - outer_top > height {
        if top - outer_top as f32 > outer_bottom as f32 - bottom {
            outer_top += 1;
        } else {
            outer_bottom -= 1;
        }
    }
    Some((
        i32::try_from(outer_left).ok()?,
        i32::try_from(outer_top).ok()?,
        i32::try_from(outer_right - outer_left).ok()?,
        i32::try_from(outer_bottom - outer_top).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_path(x: f64, y: f64, w: f64, h: f64) -> Path {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        path
    }

    fn scale(s: f64) -> Transform2D {
        Transform2D {
            a: s,
            b: 0.0,
            c: 0.0,
            d: s,
            e: 0.0,
            f: 0.0,
        }
    }

    #[test]
    fn whole_pixel_rectangles_are_unchanged() {
        assert_eq!(
            pdfium_fill_rect(&rect_path(2.0, 3.0, 4.0, 5.0), &Transform2D::identity()),
            Some((2, 3, 4, 5))
        );
    }

    #[test]
    fn outer_rect_drops_the_less_covered_column_and_row() {
        // [0.3, 1.6] x [0.7, 2.2]: widths ceil(1.3) = 2 and ceil(1.5) = 2.
        // Outer x [0, 2) already has width 2. Outer y [0, 3) is one taller;
        // the top row is 0.7 uncovered, the bottom 0.8: drop the bottom.
        assert_eq!(
            pdfium_fill_rect(&rect_path(0.3, 0.7, 1.3, 1.5), &Transform2D::identity()),
            Some((0, 0, 2, 2))
        );
        // [0.6, 1.8]: width ceil(1.2) = 2, outer [0, 2) width 2 -> kept.
        // [0.6, 1.2]: width ceil(0.6) = 1, outer [0, 2) width 2: the left
        // column is 0.6 uncovered, the right 0.8 -> drop the right one.
        assert_eq!(
            pdfium_fill_rect(&rect_path(0.6, 0.0, 0.6, 1.0), &Transform2D::identity()),
            Some((0, 0, 1, 1))
        );
        // [0.9, 1.4]: left uncovered 0.9 > right uncovered 0.6 -> drop left.
        assert_eq!(
            pdfium_fill_rect(&rect_path(0.9, 0.0, 0.5, 1.0), &Transform2D::identity()),
            Some((1, 0, 1, 1))
        );
    }

    #[test]
    fn hairline_and_zero_extent_rectangles_cover_one_pixel() {
        assert_eq!(
            pdfium_fill_rect(&rect_path(5.2, 1.0, 0.1, 3.0), &Transform2D::identity()),
            Some((5, 1, 1, 3))
        );
        // Zero width on a pixel boundary still paints one column, but the
        // rectangle test rejects a degenerate (point) rectangle.
        assert!(
            pdfium_fill_rect(&rect_path(4.0, 1.0, 0.0, 0.0), &Transform2D::identity()).is_none()
        );
    }

    #[test]
    fn abutting_rectangles_leave_no_gap() {
        let t = scale(150.0 / 72.0);
        let mut previous_end = None;
        let mut x = 10.0;
        for width in [7.3, 4.15, 9.9, 3.33, 12.05] {
            let (px, _, pw, _) = pdfium_fill_rect(&rect_path(x, 0.0, width, 10.0), &t).unwrap();
            if let Some(end) = previous_end {
                assert!(px <= end, "gap between {end} and {px}");
            }
            previous_end = Some(px + pw);
            x += width;
        }
    }

    #[test]
    fn rectangle_forms_follow_the_pdfium_point_list() {
        let t = Transform2D::identity();
        // m l l l h -> the close adds the fourth edge.
        let mut closed = Path::new();
        closed.move_to(1.0, 1.0);
        closed.line_to(4.0, 1.0);
        closed.line_to(4.0, 3.0);
        closed.line_to(1.0, 3.0);
        closed.close();
        assert_eq!(pdfium_fill_rect(&closed, &t), Some((1, 1, 3, 2)));
        // m l l l l (explicitly back at the start, no h).
        let mut explicit = Path::new();
        explicit.move_to(1.0, 1.0);
        explicit.line_to(4.0, 1.0);
        explicit.line_to(4.0, 3.0);
        explicit.line_to(1.0, 3.0);
        explicit.line_to(1.0, 1.0);
        assert_eq!(pdfium_fill_rect(&explicit, &t), Some((1, 1, 3, 2)));
        // A mid-edge vertex is normalized away.
        let mut mid_edge = Path::new();
        mid_edge.move_to(1.0, 1.0);
        mid_edge.line_to(2.5, 1.0);
        mid_edge.line_to(4.0, 1.0);
        mid_edge.line_to(4.0, 3.0);
        mid_edge.line_to(1.0, 3.0);
        mid_edge.close();
        assert_eq!(pdfium_fill_rect(&mid_edge, &t), Some((1, 1, 3, 2)));
        // m l l l f (open, never back at the start) is not a rectangle.
        let mut open = Path::new();
        open.move_to(1.0, 1.0);
        open.line_to(4.0, 1.0);
        open.line_to(4.0, 3.0);
        open.line_to(1.0, 3.0);
        assert!(pdfium_fill_rect(&open, &t).is_none());
        // Two rectangles in one path keep the general rasterizer.
        let mut two = rect_path(0.0, 0.0, 2.0, 2.0);
        two.rect(5.0, 5.0, 2.0, 2.0);
        assert!(pdfium_fill_rect(&two, &t).is_none());
    }

    #[test]
    fn rotated_and_curved_paths_are_not_rectangles() {
        let rotated = Transform2D {
            a: 0.8,
            b: 0.6,
            c: -0.6,
            d: 0.8,
            e: 0.0,
            f: 0.0,
        };
        assert!(pdfium_fill_rect(&rect_path(0.0, 0.0, 5.0, 5.0), &rotated).is_none());
        // A quarter turn keeps the rectangle axis-aligned.
        let quarter = Transform2D {
            a: 0.0,
            b: 2.0,
            c: -2.0,
            d: 0.0,
            e: 20.0,
            f: 0.0,
        };
        assert_eq!(
            pdfium_fill_rect(&rect_path(1.0, 1.0, 2.0, 3.0), &quarter),
            Some((12, 2, 6, 4))
        );
        let mut curve = Path::new();
        curve.move_to(0.0, 0.0);
        curve.curve_to(1.0, 0.0, 2.0, 0.0, 3.0, 0.0);
        curve.line_to(3.0, 3.0);
        curve.line_to(0.0, 3.0);
        curve.close();
        assert!(pdfium_fill_rect(&curve, &Transform2D::identity()).is_none());
    }
}
