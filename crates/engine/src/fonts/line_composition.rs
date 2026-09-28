//! Exact shortest path over declared candidates, not monotonic width search.
//! Budget exhaustion is an error, never an implicit greedy/success fallback.
use super::{LineBlock, LineBlockReason, LineBreakBatch, MeasuredLine, PreparedParagraph};
use crate::{Result, WellfriendError};
use std::{
    cmp::Ordering,
    ops::{Bound, Range},
};

const MAX_CANDIDATES: usize = 4096;
const MAX_MEASUREMENTS: usize = 131_072;
const MAX_MEASURED_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
struct Budget {
    measurements: usize,
    bytes: usize,
    // Invocation-local: never reuse metrics across another font/style/provider.
    widths: std::collections::BTreeMap<(usize, usize), f64>,
}
impl Budget {
    fn charge(&mut self, bytes: usize) -> Result<()> {
        crate::cancel::check_current_cancel("balanced paragraph composition")?;
        self.measurements = self.measurements.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.measurements > MAX_MEASUREMENTS || self.bytes > MAX_MEASURED_BYTES {
            return Err(WellfriendError::ResourceLimit("balanced paragraph measurement budget exceeded; select greedy composition or reduce paragraph complexity".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
struct Cost {
    emergency: usize,
    lines: usize,
    slack: f64,
}
impl Cost {
    fn cmp(self, other: Self) -> Ordering {
        self.emergency
            .cmp(&other.emergency)
            .then(self.lines.cmp(&other.lines))
            .then(self.slack.total_cmp(&other.slack))
    }
}
#[derive(Clone, Copy)]
struct State {
    cost: Cost,
    previous: usize,
    width: f64,
}

fn measure_checked(
    budget: &mut Budget,
    range: Range<usize>,
    measure: &impl Fn(Range<usize>) -> Result<f64>,
) -> Result<f64> {
    crate::cancel::check_current_cancel("balanced paragraph metric lookup")?;
    let key = (range.start, range.end);
    if let Some(width) = budget.widths.get(&key) {
        return Ok(*width);
    }
    budget.charge(range.len())?;
    let width = measure(range)?;
    crate::cancel::check_current_cancel("balanced paragraph measured candidate")?;
    if !width.is_finite() || width < 0.0 {
        return Err(WellfriendError::invalid_input(
            "non-finite or negative shaped line width",
        ));
    }
    budget.widths.insert(key, width);
    Ok(width)
}

fn candidates(
    paragraph: &PreparedParagraph<'_>,
    from: usize,
    stop: usize,
    emergency: bool,
) -> Result<Vec<usize>> {
    let range = (Bound::Excluded(from), Bound::Included(stop));
    let mut nodes = vec![from];
    // Both ordered streams already contain only policy-approved grapheme edges.
    // A set keeps duplicate natural/emergency edges from creating zero-length lines.
    let mut values = std::collections::BTreeSet::new();
    for n in paragraph.allowed.range(range) {
        values.insert(*n);
        if values.len() >= MAX_CANDIDATES {
            return Err(candidate_limit());
        }
    }
    if emergency {
        let start = paragraph.emergency.partition_point(|n| *n <= from);
        for n in paragraph.emergency[start..]
            .iter()
            .take_while(|n| **n < stop)
        {
            values.insert(*n);
            if values.len() >= MAX_CANDIDATES {
                return Err(candidate_limit());
            }
        }
    }
    values.insert(stop);
    if values.len() >= MAX_CANDIDATES {
        return Err(candidate_limit());
    }
    nodes.extend(values);
    Ok(nodes)
}
fn candidate_limit() -> WellfriendError {
    WellfriendError::ResourceLimit("balanced paragraph candidate budget exceeded; select greedy composition or reduce paragraph complexity".into())
}

fn solve(
    paragraph: &PreparedParagraph<'_>,
    nodes: &[usize],
    width: f64,
    budget: &mut Budget,
    measure: &impl Fn(Range<usize>) -> Result<f64>,
) -> Result<LineBreakBatch> {
    let mut best = vec![None::<State>; nodes.len()];
    best[0] = Some(State {
        cost: Cost::default(),
        previous: 0,
        width: 0.0,
    });
    for start in 0..nodes.len() - 1 {
        let Some(prior) = best[start] else { continue };
        for end in start + 1..nodes.len() {
            let measured = measure_checked(budget, nodes[start]..nodes[end], measure)?;
            // Never stop at the first over-wide candidate: substitutions,
            // kerning and marks can make a later candidate narrower.
            if measured > width + 1e-7 {
                continue;
            }
            let slack = 1.0 - (measured / width).min(1.0);
            let cost = Cost {
                emergency: prior.cost.emergency
                    + usize::from(
                        !paragraph.allowed.contains(&nodes[end]) && end + 1 != nodes.len(),
                    ),
                lines: prior.cost.lines + 1,
                slack: prior.cost.slack + slack * slack,
            };
            if best[end].is_none_or(|old| {
                cost.cmp(old.cost).is_lt() || (cost.cmp(old.cost).is_eq() && start < old.previous)
            }) {
                best[end] = Some(State {
                    cost,
                    previous: start,
                    width: measured,
                });
            }
        }
    }
    // If the full segment is infeasible, preserve the furthest reachable source
    // prefix for a later, wider approved frame. This is not complete success.
    let mut end = best.iter().rposition(Option::is_some).unwrap_or(0);
    let blocked = (end + 1 != nodes.len()).then_some(LineBlock {
        byte_start: nodes[end],
        reason: LineBlockReason::NoFittingContinuation,
    });
    let mut lines = Vec::new();
    while end != 0 {
        crate::cancel::check_current_cancel("balanced paragraph reconstruction")?;
        let state = best[end]
            .ok_or_else(|| WellfriendError::invalid_input("missing balanced predecessor"))?;
        lines.push(MeasuredLine {
            bytes: nodes[state.previous]..nodes[end],
            width: state.width,
        });
        end = state.previous;
    }
    lines.reverse();
    Ok(LineBreakBatch { lines, blocked })
}

pub(super) fn compose(
    paragraph: &PreparedParagraph<'_>,
    from: usize,
    width: f64,
    max_lines: usize,
    measure: impl Fn(Range<usize>) -> Result<f64>,
) -> Result<LineBreakBatch> {
    crate::cancel::check_current_cancel("balanced paragraph preparation")?;
    paragraph
        .boundaries
        .binary_search(&from)
        .map_err(|_| WellfriendError::invalid_input("line start divides a grapheme"))?;
    let mut start = from;
    let mut lines = Vec::new();
    let mut budget = Budget::default();
    while start < paragraph.text.len() && lines.len() < max_lines {
        let stop = paragraph
            .mandatory
            .get(paragraph.mandatory.partition_point(|n| *n <= start))
            .copied()
            .unwrap_or(paragraph.text.len());
        // A complete one-line segment is already optimal for this objective;
        // no candidate graph or monotonically increasing width assumption needed.
        let whole = measure_checked(&mut budget, start..stop, &measure)?;
        if whole <= width + 1e-7 {
            lines.push(MeasuredLine {
                bytes: start..stop,
                width: whole,
            });
            start = stop;
            continue;
        }
        let natural = candidates(paragraph, start, stop, false)?;
        let mut segment = solve(paragraph, &natural, width, &mut budget, &measure)?;
        if segment.blocked.is_some() {
            let all = candidates(paragraph, start, stop, true)?;
            if all != natural {
                segment = solve(paragraph, &all, width, &mut budget, &measure)?;
            }
        }
        let capacity = max_lines - lines.len();
        if segment.lines.len() >= capacity {
            lines.extend(segment.lines.into_iter().take(capacity));
            return Ok(LineBreakBatch {
                lines,
                blocked: None,
            });
        }
        lines.extend(segment.lines);
        if let Some(blocked) = segment.blocked {
            return Ok(LineBreakBatch {
                lines,
                blocked: Some(blocked),
            });
        }
        start = stop;
    }
    Ok(LineBreakBatch {
        lines,
        blocked: None,
    })
}

#[cfg(test)]
#[path = "line_composition_tests.rs"]
mod tests;
