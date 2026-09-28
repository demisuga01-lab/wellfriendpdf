//! Analytic breakpoints for the piecewise-affine function subset exported as
//! gradient stops. Nonlinear exponents still require the exact PostScript path.
use super::{push_domain_boundary_offset, VectorShadingFunction, VectorType2Function};
use crate::render::{function::domain_position, shading::shading_domain_value};

pub(super) const MAX_STOPS: usize = 65_536;

pub(super) fn add_offsets(
    function: &VectorShadingFunction,
    shading_domain: [f64; 2],
    offsets: &mut Vec<f64>,
) -> Option<()> {
    match function {
        VectorShadingFunction::Type2(function) => {
            for input in type2_knots(function, None)? {
                push_domain_boundary_offset(shading_domain, input, offsets)?;
            }
        }
        VectorShadingFunction::Stitching(function) => {
            for boundary in function.domain {
                push_domain_boundary_offset(shading_domain, boundary, offsets)?;
            }
            for segment in &function.segments {
                crate::cancel::check_current_cancel("vector gradient breakpoints").ok()?;
                for boundary in segment.input_domain {
                    push_domain_boundary_offset(shading_domain, boundary, offsets)?;
                }
                if segment.encode[0] == segment.encode[1]
                    || segment.input_domain[0] == segment.input_domain[1]
                {
                    continue;
                }
                for child_input in type2_knots(&segment.function, function.range.as_deref())? {
                    let low = segment.encode[0].min(segment.encode[1]);
                    let high = segment.encode[0].max(segment.encode[1]);
                    if child_input <= low || child_input >= high {
                        continue;
                    }
                    let fraction = domain_position(child_input, low, high)?;
                    let fraction = if segment.encode[0] < segment.encode[1] {
                        fraction
                    } else {
                        1.0 - fraction
                    };
                    let input = shading_domain_value(
                        segment.input_domain[0],
                        segment.input_domain[1],
                        fraction,
                    );
                    push_domain_boundary_offset(shading_domain, input, offsets)?;
                }
            }
        }
    }
    Some(())
}

fn type2_knots(
    function: &VectorType2Function,
    parent_range: Option<&[[f64; 2]]>,
) -> Option<Vec<f64>> {
    // Domain clipping is another piecewise-affine transition, even without Range.
    let mut knots = function.domain.to_vec();
    if function.n != 1.0 {
        return Some(knots);
    }
    for (channel, (&start, &end)) in function.c0.iter().zip(&function.c1).enumerate() {
        crate::cancel::check_current_cancel("vector gradient clipping").ok()?;
        if start == end {
            continue;
        }
        // Clamping is monotonic; every bend of the composed child/parent/device
        // clamps must occur at one of these raw-value thresholds. Extra knots
        // hidden by an earlier clamp are harmless and are deduplicated upstream.
        let mut thresholds = vec![0.0, 1.0];
        for ranges in [function.range.as_deref(), parent_range]
            .into_iter()
            .flatten()
        {
            thresholds.extend_from_slice(ranges.get(channel)?);
        }
        for threshold in thresholds {
            let numerator = threshold - start;
            let denominator = end - start;
            let input = if numerator.is_finite() && denominator.is_finite() {
                numerator / denominator
            } else {
                (threshold * 0.5 - start * 0.5) / (end * 0.5 - start * 0.5)
            };
            if input.is_finite() && input > function.domain[0] && input < function.domain[1] {
                knots.push(input);
            }
        }
    }
    knots.sort_by(f64::total_cmp);
    knots.dedup();
    Some(knots)
}

#[cfg(test)]
#[path = "vector_function_stops_tests.rs"]
mod tests;
