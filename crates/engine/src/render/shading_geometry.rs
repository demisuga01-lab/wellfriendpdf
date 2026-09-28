//! Scale-aware analytic geometry for function, axial and radial shadings.
//!
//! No absolute user-space epsilon decides whether a gradient or matrix exists.
//! These are bounded floating-point calculations, not exact predicates.
use super::Transform2D;

#[cfg(test)]
#[path = "shading_geometry_tests.rs"]
mod tests;

// Recover the rounded product's residual before subtracting. All callers
// normalize operands first so these products cannot overflow.
fn difference_of_products(a: f64, b: f64, c: f64, d: f64) -> f64 {
    let cd = c * d;
    a.mul_add(b, -cd) + (-c).mul_add(d, cd)
}

fn sum_error(a: f64, b: f64, sum: f64) -> f64 {
    let b_virtual = sum - a;
    (a - (sum - b_virtual)) + (b - b_virtual)
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    let p = [a[0] * b[0], a[1] * b[1], a[2] * b[2]];
    let s = p[0] + p[1];
    let t = s + p[2];
    let error = sum_error(p[0], p[1], s)
        + sum_error(s, p[2], t)
        + a[0].mul_add(b[0], -p[0])
        + a[1].mul_add(b[1], -p[1])
        + a[2].mul_add(b[2], -p[2]);
    t + error
}

/// Interpolate a finite domain without overflowing `end - start`.
pub(crate) fn domain_value(start: f64, end: f64, s: f64) -> f64 {
    if s == 0.0 {
        start
    } else if s == 1.0 {
        end
    } else if start.is_sign_negative() == end.is_sign_negative() {
        s.mul_add(end - start, start)
    } else {
        (1.0 - s).mul_add(start, s * end)
    }
}

/// Invert the two independently normalized rows. In contrast to a determinant
/// cutoff, this preserves units, reflection and very small/large uniform scales.
/// Return None if the represented matrix is singular or its inverse is not
/// representable by finite f64 coefficients. Other rendering subsystems still
/// use the canonical Transform2D inverse; this is the analytic shading mapper.
pub(super) fn inverse(transform: &Transform2D) -> Option<Transform2D> {
    let [a, b, c, d, e, f] = transform.to_array();
    if ![a, b, c, d, e, f].iter().all(|v| v.is_finite()) {
        return None;
    }
    let sx = a.abs().max(c.abs());
    let sy = b.abs().max(d.abs());
    if sx == 0.0 || sy == 0.0 {
        return None;
    }
    let (an, bn, cn, dn) = (a / sx, b / sy, c / sx, d / sy);
    let det = difference_of_products(an, dn, bn, cn);
    if det == 0.0 {
        return None;
    }
    let divide = |n: f64, scale: f64| {
        let first = n / det;
        if first.is_finite() {
            first / scale
        } else {
            (n / scale) / det
        }
    };
    let (ia, ib, ic, id) = (
        divide(dn, sx),
        divide(-bn, sx),
        divide(-cn, sy),
        divide(an, sy),
    );
    // FMA avoids an intermediate overflow when one product cancels the other.
    let ie = (-ia).mul_add(e, -ic * f);
    let iff = (-ib).mul_add(e, -id * f);
    let values = [ia, ib, ic, id, ie, iff];
    values.iter().all(|v| v.is_finite()).then(|| values.into())
}

fn normalize<const N: usize>(values: &mut [f64; N]) -> Result<(), &'static str> {
    let scale = values.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    if !scale.is_finite() {
        return Err("nonfinite analytic shading geometry");
    }
    if scale != 0.0 {
        for value in values {
            let original = *value;
            *value /= scale;
            if original != 0.0 && *value == 0.0 {
                return Err("analytic shading geometry exceeds floating-point range");
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct AxialGeometry {
    start: [f64; 2],
    end: [f64; 2],
}

impl AxialGeometry {
    pub(super) fn new(coords: [f64; 4]) -> Result<Self, &'static str> {
        if !coords.iter().all(|v| v.is_finite()) {
            return Err("nonfinite axial coordinates");
        }
        Ok(Self {
            start: [coords[0], coords[1]],
            end: [coords[2], coords[3]],
        })
    }

    pub(super) fn parameter(
        self,
        point: (f64, f64),
        extend: [bool; 2],
    ) -> Result<Option<f64>, &'static str> {
        if !point.0.is_finite() || !point.1.is_finite() {
            return Err("nonfinite axial sample position");
        }
        if self.start == self.end {
            return Ok(None);
        }
        let local = |factor: f64| {
            [
                self.end[0] * factor - self.start[0] * factor,
                self.end[1] * factor - self.start[1] * factor,
                point.0 * factor - self.start[0] * factor,
                point.1 * factor - self.start[1] * factor,
            ]
        };
        let mut values = local(1.0);
        if !values.iter().all(|v| v.is_finite()) {
            values = local(0.5);
        }
        // Normalize the axis separately: a distant perpendicular sample must
        // not make a valid, short axis vanish when squared.
        let axis_scale = values[0].abs().max(values[1].abs());
        let point_scale = values[2].abs().max(values[3].abs());
        if axis_scale == 0.0 {
            return Err("axial axis underflows after coordinate normalization");
        }
        if point_scale == 0.0 {
            return Ok(Some(0.0));
        }
        let nx = values[0] / axis_scale;
        let ny = values[1] / axis_scale;
        let projection = dot3(
            [nx, ny, 0.0],
            [values[2] / point_scale, values[3] / point_scale, 0.0],
        );
        let unit_parameter = projection / nx.mul_add(nx, ny * ny);
        let ratio = point_scale / axis_scale;
        let s = if ratio.is_finite() {
            unit_parameter * ratio
        } else {
            (unit_parameter * point_scale) / axis_scale
        };
        if s.is_nan() {
            return Err("indeterminate axial sample parameter");
        }
        // Infinite signed projections are outside the interval and only use a
        // constant endpoint when the corresponding extension is authorized.
        Ok(accept_interval(s, extend).map(|s| s.clamp(0.0, 1.0)))
    }
}

fn accept_interval(s: f64, extend: [bool; 2]) -> Option<f64> {
    ((0.0..=1.0).contains(&s) || (s < 0.0 && extend[0]) || (s > 1.0 && extend[1])).then_some(s)
}

/// Roots of a*s*s - 2*h*s + c, with normalized finite coefficients.
/// Vieta's relation computes the cancellation-prone root from the other root.
fn roots(a: f64, h: f64, c: f64) -> [Option<f64>; 2] {
    if a == 0.0 {
        return if h != 0.0 {
            [Some((0.5 * c) / h), None]
        } else if c == 0.0 {
            // All circles on this generator reach the sample. The final
            // in-domain circle wins, including any constant end extension.
            [Some(1.0), None]
        } else {
            [None, None]
        };
    }
    let discriminant = difference_of_products(h, h, a, c);
    if discriminant < 0.0 {
        return [None, None];
    }
    let q = h + discriminant.sqrt().copysign(h);
    if q == 0.0 {
        [Some(0.0), None]
    } else {
        [Some(q / a), Some(c / q)]
    }
}

#[derive(Clone, Copy)]
pub(super) struct RadialGeometry {
    start: [f64; 3],
    end: [f64; 3],
}

impl RadialGeometry {
    pub(super) fn new(coords: [f64; 6]) -> Result<Self, &'static str> {
        if !coords.iter().all(|v| v.is_finite()) || coords[2] < 0.0 || coords[5] < 0.0 {
            return Err("radial coordinates require finite centres and nonnegative radii");
        }
        Ok(Self {
            start: [coords[0], coords[1], coords[2]],
            end: [coords[3], coords[4], coords[5]],
        })
    }

    pub(super) fn parameter(
        self,
        point: (f64, f64),
        extend: [bool; 2],
    ) -> Result<Option<f64>, &'static str> {
        if !point.0.is_finite() || !point.1.is_finite() {
            return Err("nonfinite radial sample position");
        }
        if self.start[2] == 0.0 && self.end[2] == 0.0 {
            return Ok(None);
        }
        let local = |factor: f64| {
            [
                self.end[0] * factor - self.start[0] * factor,
                self.end[1] * factor - self.start[1] * factor,
                self.end[2] * factor - self.start[2] * factor,
                point.0 * factor - self.start[0] * factor,
                point.1 * factor - self.start[1] * factor,
                self.start[2] * factor,
            ]
        };
        let mut values = local(1.0);
        if !values.iter().all(|v| v.is_finite()) {
            values = local(0.5);
        }
        normalize(&mut values)?;
        let [ax, ay, ar, dx, dy, r0] = values;
        if ax == 0.0 && ay == 0.0 && ar != 0.0 {
            // Concentric circles need no quadratic or squared-radius root.
            // In particular, the radius-zero centre is not rejected because a
            // reconstructed r0 + s*ar rounded just below zero.
            let s = (dx.hypot(dy) - r0) / ar;
            let Some(s) = accept_interval(s, extend) else {
                return Ok(None);
            };
            if !s.is_finite() {
                return Err("radial root exceeds floating-point range");
            }
            return Ok(Some(s.clamp(0.0, 1.0)));
        }
        // Do not turn a nonzero quadratic term into an apparent linear case
        // because its normalized square underflowed. A wider-exponent/exact
        // fallback is separate work; this branch reports the limit explicitly.
        if values
            .iter()
            .any(|v| *v != 0.0 && (*v * *v) < f64::MIN_POSITIVE)
        {
            return Err("radial coefficients exceed floating-point dynamic range");
        }
        let a = dot3([ax, ay, -ar], [ax, ay, ar]);
        let h = dot3([dx, dy, r0], [ax, ay, ar]);
        let c = dot3([dx, dy, -r0], [dx, dy, r0]);
        let coefficient_scale = a.abs().max(h.abs()).max(c.abs());
        let (a, h, c) = if coefficient_scale == 0.0 {
            (a, h, c)
        } else {
            (
                a / coefficient_scale,
                h / coefficient_scale,
                c / coefficient_scale,
            )
        };
        if (h != 0.0 && h * h < f64::MIN_POSITIVE)
            || (a != 0.0 && c != 0.0 && (a * c).abs() < f64::MIN_POSITIVE)
        {
            return Err("radial discriminant exceeds floating-point dynamic range");
        }
        let mut selected: Option<f64> = None;
        for s in roots(a, h, c).into_iter().flatten() {
            let Some(s) = accept_interval(s, extend) else {
                continue;
            };
            // Radius validity is checked before choosing the later circle. A
            // negative-radius root from squaring is not part of a PDF shading.
            let radius = if ar == 0.0 { r0 } else { s.mul_add(ar, r0) };
            if radius < 0.0 {
                continue;
            }
            if !s.is_finite() || !radius.is_finite() {
                return Err("radial root exceeds floating-point range");
            }
            selected = Some(selected.map_or(s, |previous| previous.max(s)));
        }
        Ok(selected.map(|s| s.clamp(0.0, 1.0)))
    }
}
