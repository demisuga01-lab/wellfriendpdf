//! Exact compatibility predicates for the expanded Type 2 linear subset.
//!
//! This does not replace the curve-union engine. It decides whether a linear
//! source can keep its existing program without epsilon-based classification.
//! All coordinates are bounded 16.16 values; i128 determinants are exact here.
use super::{budget, fail, valid_point, ContourReport, MAX_OUTPUT, QUANTUM};
use crate::Result;
use kurbo::{BezPath, PathEl, Point};

const MAX_WORK: usize = 16_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FixedPoint {
    x: i64,
    y: i64,
}
impl FixedPoint {
    fn from_point(p: Point) -> Result<Self> {
        valid_point(p)?;
        let x = p.x / QUANTUM;
        let y = p.y / QUANTUM;
        // No rounding: this branch must not turn a near-contact into contact.
        // The expanded charstring writer already quantizes operands to 16.16.
        if x.fract() != 0. || y.fract() != 0. {
            return Err(fail("linear contour coordinate is outside the 16.16 grid"));
        }
        Ok(Self {
            x: x as i64,
            y: y as i64,
        })
    }

    fn from_curve_point(p: Point) -> Result<Option<Self>> {
        valid_point(p)?;
        let x = p.x / QUANTUM;
        let y = p.y / QUANTUM;
        if x.fract() != 0. || y.fract() != 0. {
            return Ok(None);
        }
        Ok(Some(Self {
            x: x as i64,
            y: y as i64,
        }))
    }
}

#[derive(Clone, Copy)]
struct Edge {
    a: FixedPoint,
    b: FixedPoint,
    owner: usize,
}
struct Ring {
    start: usize,
    end: usize,
    sample: FixedPoint,
    area: i128,
}
struct Pending {
    first: FixedPoint,
    pen: FixedPoint,
    start: usize,
    axis: Option<FixedPoint>,
    collinear: bool,
}
impl Pending {
    fn see(&mut self, point: FixedPoint) {
        if let Some(axis) = self.axis {
            self.collinear &= cross(self.first, axis, point) == 0;
        } else if point != self.first {
            self.axis = Some(point);
        }
    }
}

fn cross(a: FixedPoint, b: FixedPoint, c: FixedPoint) -> i128 {
    // |coordinate| <= 2^36, |difference| <= 2^37, determinant < 2^75.
    // Even a 4096-edge signed-area sum is <= 2^85, safely within i128.
    let dx = i128::from(b.x) - i128::from(a.x);
    let dy = i128::from(b.y) - i128::from(a.y);
    dx * (i128::from(c.y) - i128::from(a.y)) - dy * (i128::from(c.x) - i128::from(a.x))
}
fn projection(a: FixedPoint, b: FixedPoint, p: FixedPoint) -> i128 {
    let dx = i128::from(b.x) - i128::from(a.x);
    let dy = i128::from(b.y) - i128::from(a.y);
    dx * (i128::from(p.x) - i128::from(a.x)) + dy * (i128::from(p.y) - i128::from(a.y))
}
fn monotone_collinear(a: FixedPoint, controls: &[FixedPoint], b: FixedPoint) -> bool {
    if a == b {
        return controls.iter().all(|point| *point == a);
    }
    let extent = projection(a, b, b);
    let mut previous = 0i128;
    for &point in controls {
        if cross(a, b, point) != 0 {
            return false;
        }
        let current = projection(a, b, point);
        if current < previous || current > extent {
            return false;
        }
        previous = current;
    }
    true
}
fn in_box(e: Edge, p: FixedPoint) -> bool {
    (e.a.x.min(e.b.x)..=e.a.x.max(e.b.x)).contains(&p.x)
        && (e.a.y.min(e.b.y)..=e.a.y.max(e.b.y)).contains(&p.y)
}
fn opposite(a: i128, b: i128) -> bool {
    (a < 0 && b > 0) || (a > 0 && b < 0)
}
fn contact(a: Edge, b: Edge) -> bool {
    let ab0 = cross(a.a, a.b, b.a);
    let ab1 = cross(a.a, a.b, b.b);
    let ba0 = cross(b.a, b.b, a.a);
    let ba1 = cross(b.a, b.b, a.b);
    (ab0 == 0 && in_box(a, b.a))
        || (ab1 == 0 && in_box(a, b.b))
        || (ba0 == 0 && in_box(b, a.a))
        || (ba1 == 0 && in_box(b, a.b))
        || (opposite(ab0, ab1) && opposite(ba0, ba1))
}
fn adjacent_overlap(a: Edge, b: Edge) -> bool {
    let (shared, p, q) = if a.a == b.a {
        (a.a, a.b, b.b)
    } else if a.a == b.b {
        (a.a, a.b, b.a)
    } else if a.b == b.a {
        (a.b, a.a, b.b)
    } else {
        (a.b, a.a, b.a)
    };
    if cross(shared, p, q) != 0 {
        return false;
    }
    // Both rays leaving the shared vertex point in the same direction only
    // when adjacent segments double back over a nonempty interval.
    (i128::from(p.x) - i128::from(shared.x)) * (i128::from(q.x) - i128::from(shared.x))
        + (i128::from(p.y) - i128::from(shared.y)) * (i128::from(q.y) - i128::from(shared.y))
        > 0
}
fn charge(report: &mut ContourReport) -> Result<()> {
    report.exact_linear_work = report
        .exact_linear_work
        .checked_add(1)
        .ok_or_else(|| budget("exact linear predicate work overflow"))?;
    if report.exact_linear_work > MAX_WORK {
        return Err(budget("exact linear predicate work"));
    }
    if report.exact_linear_work.is_multiple_of(256) {
        crate::cancel::check_current_cancel("exact linear contour predicates")?;
    }
    Ok(())
}
fn push(edges: &mut Vec<Edge>, a: FixedPoint, b: FixedPoint, owner: usize) -> Result<()> {
    if a != b {
        if edges.len() == MAX_OUTPUT {
            return Err(budget("exact linear edge capacity"));
        }
        edges.push(Edge { a, b, owner });
    }
    Ok(())
}

/// None: a curve needs the general numerical path. Some(true): unchanged
/// linear rings have unit/zero fill boundaries and no nonadjacent contacts.
/// Some(false): union/reconstruction is required (not necessarily malformed).
pub(super) fn compatible(path: &BezPath, report: &mut ContourReport) -> Result<Option<bool>> {
    crate::cancel::check_current_cancel("exact linear contour classification")?;
    if path.elements().len() > 2 * MAX_OUTPUT + 1 {
        return Err(budget("exact linear path capacity"));
    }
    let mut edges = Vec::new();
    let mut rings = Vec::<Ring>::new();
    let mut active: Option<Pending> = None;
    for element in path.iter() {
        match element {
            PathEl::MoveTo(p) => {
                if active.is_some() {
                    return Err(fail("unclosed linear contour"));
                }
                let point = FixedPoint::from_point(p)?;
                active = Some(Pending {
                    first: point,
                    pen: point,
                    start: edges.len(),
                    axis: None,
                    collinear: true,
                });
            }
            PathEl::LineTo(p) => {
                let pending = active
                    .as_mut()
                    .ok_or_else(|| fail("linear edge before move"))?;
                let next = FixedPoint::from_point(p)?;
                push(&mut edges, pending.pen, next, rings.len())?;
                pending.see(next);
                pending.pen = next;
            }
            PathEl::QuadTo(control, end) => {
                let pending = active.as_mut().ok_or_else(|| fail("curve before move"))?;
                let (Some(control), Some(next)) = (
                    FixedPoint::from_curve_point(control)?,
                    FixedPoint::from_curve_point(end)?,
                ) else {
                    return Ok(None);
                };
                if !monotone_collinear(pending.pen, &[control], next) {
                    return Ok(None);
                }
                push(&mut edges, pending.pen, next, rings.len())?;
                pending.see(control);
                pending.see(next);
                pending.pen = next;
            }
            PathEl::CurveTo(a, b, end) => {
                let pending = active.as_mut().ok_or_else(|| fail("curve before move"))?;
                let (Some(a), Some(b), Some(next)) = (
                    FixedPoint::from_curve_point(a)?,
                    FixedPoint::from_curve_point(b)?,
                    FixedPoint::from_curve_point(end)?,
                ) else {
                    return Ok(None);
                };
                if !monotone_collinear(pending.pen, &[a, b], next) {
                    return Ok(None);
                }
                push(&mut edges, pending.pen, next, rings.len())?;
                pending.see(a);
                pending.see(b);
                pending.see(next);
                pending.pen = next;
            }
            PathEl::ClosePath => {
                let pending = active
                    .take()
                    .ok_or_else(|| fail("linear close before move"))?;
                let (first, pen, start) = (pending.first, pending.pen, pending.start);
                // A closed path whose endpoints and all Bezier controls lie
                // on one line has zero winding off that line. Do not mistake
                // such harmless zero-ink programs for filled overlaps.
                if pending.collinear {
                    edges.truncate(start);
                    continue;
                }
                push(&mut edges, pen, first, rings.len())?;
                if start == edges.len() {
                    continue;
                }
                let area = edges[start..]
                    .iter()
                    .map(|e| {
                        i128::from(e.a.x) * i128::from(e.b.y)
                            - i128::from(e.a.y) * i128::from(e.b.x)
                    })
                    .sum::<i128>();
                if area == 0 {
                    return Ok(Some(false));
                }
                rings.push(Ring {
                    start,
                    end: edges.len(),
                    sample: first,
                    area,
                });
            }
        }
    }
    if active.is_some() {
        return Err(fail("unclosed linear contour"));
    }
    if edges.is_empty() {
        return Ok(Some(true));
    }

    let mut order = (0..edges.len()).collect::<Vec<_>>();
    order.sort_unstable_by_key(|i| (edges[*i].a.x.min(edges[*i].b.x), *i));
    for (position, &i) in order.iter().enumerate() {
        let a = edges[i];
        crate::cancel::check_current_cancel("exact linear intersection sweep")?;
        for &j in &order[position + 1..] {
            let b = edges[j];
            if b.a.x.min(b.b.x) > a.a.x.max(a.b.x) {
                break;
            }
            charge(report)?;
            if b.a.y.min(b.b.y) > a.a.y.max(a.b.y) || a.a.y.min(a.b.y) > b.a.y.max(b.b.y) {
                continue;
            }
            let (lo, hi) = (i.min(j), i.max(j));
            let ring = &rings[a.owner];
            let adjacent =
                a.owner == b.owner && (hi == lo + 1 || (lo == ring.start && hi + 1 == ring.end));
            if if adjacent {
                adjacent_overlap(a, b)
            } else {
                contact(a, b)
            } {
                return Ok(Some(false));
            }
        }
    }

    // With all contacts excluded, each ring is simple. Its first vertex is not
    // on another ring. Exact half-open ray crossings determine the outside
    // winding; adding this ring's orientation gives its inside winding.
    for (owner, ring) in rings.iter().enumerate() {
        let mut outside = 0i32;
        for &edge in &edges {
            if edge.owner == owner {
                continue;
            }
            charge(report)?;
            if edge.a.y <= ring.sample.y {
                if edge.b.y > ring.sample.y && cross(edge.a, edge.b, ring.sample) > 0 {
                    outside += 1;
                }
            } else if edge.b.y <= ring.sample.y && cross(edge.a, edge.b, ring.sample) < 0 {
                outside -= 1;
            }
        }
        let inside = outside + if ring.area > 0 { 1 } else { -1 };
        if !matches!((outside, inside), (0, 1) | (1, 0) | (0, -1) | (-1, 0)) {
            return Ok(Some(false));
        }
    }
    Ok(Some(true))
}

#[cfg(test)]
#[path = "cff2_linear_contours_tests.rs"]
mod tests;
