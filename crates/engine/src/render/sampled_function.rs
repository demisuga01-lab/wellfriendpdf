//! Tensor-product interpolation of packed PDF function samples.
//!
//! Cubic taps are derived from the Hermite endpoint basis with centered slopes
//! (the cardinal Catmull-Rom interpolant). Missing exterior samples repeat the
//! endpoint; axes of size < 4 use the PDF-mandated linear fallback. No prefilter
//! or monotonicity limiter is applied: cubic overshoot is clipped only after
//! the caller applies Decode and Range.
use super::{max_value, read_sample, MAX_TYPE0_INTERPOLATION_DIMENSIONS, MAX_TYPE0_SAMPLE_VALUES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Order {
    Linear,
    Cubic,
}

#[derive(Clone, Copy, Default)]
struct Stencil {
    // Flat sample-point offsets and weights. Boundary aliases merge in place.
    taps: [(usize, f64); 4],
    count: usize,
}

impl Stencil {
    fn push(&mut self, index: usize, weight: f64) {
        if weight == 0.0 {
            return;
        }
        for tap in &mut self.taps[..self.count] {
            if tap.0 == index {
                tap.1 += weight;
                return;
            }
        }
        self.taps[self.count] = (index, weight);
        self.count += 1;
    }

    fn at(size: usize, coordinate: f64, stride: usize, order: Order) -> Option<Self> {
        if size == 0 || size > MAX_TYPE0_SAMPLE_VALUES || !coordinate.is_finite() {
            return None;
        }
        let coordinate = coordinate.clamp(0.0, (size - 1) as f64);
        let low = coordinate.floor() as usize;
        let t = coordinate - low as f64;
        let mut stencil = Self::default();
        if t == 0.0 || size == 1 {
            stencil.push(low.checked_mul(stride)?, 1.0);
        } else if order == Order::Linear || size < 4 {
            stencil.push(low.checked_mul(stride)?, 1.0 - t);
            stencil.push((low + 1).checked_mul(stride)?, t);
        } else {
            let t2 = t * t;
            let t3 = t2 * t;
            let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
            let h10 = t3 - 2.0 * t2 + t;
            let h01 = -2.0 * t3 + 3.0 * t2;
            let h11 = t3 - t2;
            for (index, weight) in [
                (low.saturating_sub(1), -0.5 * h10),
                (low, h00 - 0.5 * h11),
                (low + 1, h01 + 0.5 * h10),
                ((low + 2).min(size - 1), 0.5 * h11),
            ] {
                stencil.push(index.checked_mul(stride)?, weight);
            }
        }
        Some(stencil)
    }
}

#[cfg(test)]
pub(super) fn interpolate(
    samples: &[u8],
    sizes: &[usize],
    coordinates: &[f64],
    outputs: usize,
    bits: usize,
    order: Order,
) -> Option<Vec<f64>> {
    let mut remaining = super::MAX_FUNCTION_WORK;
    interpolate_with_budget(
        samples,
        sizes,
        coordinates,
        outputs,
        bits,
        order,
        &mut remaining,
    )
}

pub(super) fn interpolate_with_budget(
    samples: &[u8],
    sizes: &[usize],
    coordinates: &[f64],
    outputs: usize,
    bits: usize,
    order: Order,
    remaining: &mut usize,
) -> Option<Vec<f64>> {
    if sizes.is_empty()
        || sizes.len() > MAX_TYPE0_INTERPOLATION_DIMENSIONS
        || coordinates.len() != sizes.len()
        || outputs == 0
        || outputs > MAX_TYPE0_SAMPLE_VALUES
        || !matches!(bits, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32)
    {
        return None;
    }
    crate::cancel::check_current_cancel("sampled function interpolation").ok()?;
    let mut stencils = [Stencil::default(); MAX_TYPE0_INTERPOLATION_DIMENSIONS];
    let mut stride = 1_usize;
    let mut terms = 1_usize;
    for (axis, (&size, &coordinate)) in sizes.iter().zip(coordinates).enumerate() {
        stencils[axis] = Stencil::at(size, coordinate, stride, order)?;
        stride = stride.checked_mul(size)?;
        terms = terms.checked_mul(stencils[axis].count)?;
    }
    let values = stride.checked_mul(outputs)?;
    if values > MAX_TYPE0_SAMPLE_VALUES
        || terms.checked_mul(outputs)? > MAX_TYPE0_SAMPLE_VALUES
        || samples.len() < values.checked_mul(bits)?.div_ceil(8)
    {
        return None;
    }
    super::charge_work(
        remaining,
        terms.checked_mul(outputs.checked_add(sizes.len())?)?,
    )?;

    // Each dimension contributes at most its size (one, two or four distinct
    // samples). Thus reads cannot exceed the already bounded table value count.
    // Only two output-sized buffers and eight fixed-size stencils are allocated;
    // there is no tensor table, recursive buffer tree or per-corner allocation.
    let mut sum = vec![0.0_f64; outputs];
    let mut error = vec![0.0_f64; outputs];
    let mut selectors = [0_usize; MAX_TYPE0_INTERPOLATION_DIMENSIONS];
    let normalization = 1.0 / max_value(bits);
    let mut reads = 0_usize;
    for _ in 0..terms {
        let mut offset = 0_usize;
        let mut weight = 1.0;
        for axis in 0..sizes.len() {
            let (index, coefficient) = stencils[axis].taps[selectors[axis]];
            offset = offset.checked_add(index)?;
            weight *= coefficient;
        }
        let base = offset.checked_mul(outputs)?;
        for channel in 0..outputs {
            if reads.is_multiple_of(256) {
                crate::cancel::check_current_cancel("sampled function taps").ok()?;
            }
            reads += 1;
            let value =
                read_sample(samples, base.checked_add(channel)?, bits)? * normalization * weight;
            // Neumaier accumulation retains contributions across the signed
            // cubic lobes and nearly cancelling tensor-product terms.
            let next = sum[channel] + value;
            error[channel] += if sum[channel].abs() >= value.abs() {
                (sum[channel] - next) + value
            } else {
                (value - next) + sum[channel]
            };
            sum[channel] = next;
        }
        for axis in 0..sizes.len() {
            selectors[axis] += 1;
            if selectors[axis] < stencils[axis].count {
                break;
            }
            selectors[axis] = 0;
        }
    }
    for (value, correction) in sum.iter_mut().zip(error) {
        *value += correction;
    }
    sum.iter().all(|value| value.is_finite()).then_some(sum)
}

#[cfg(test)]
#[path = "sampled_function_tests.rs"]
mod tests;
