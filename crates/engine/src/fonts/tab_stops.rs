//! Deterministic tab-stop layout over exact logical UTF-8 ranges.
//!
//! A tab is layout state, not a font glyph or a run of guessed spaces. The
//! caller supplies the exact measurement function that final emission uses;
//! this module returns positioned source segments and never rewrites text.
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::Range;

const MAX_TAB_STOPS: usize = 256;
const MAX_TABS_PER_LINE: usize = 4096;
const EPSILON: f64 = 1e-7;
pub(crate) const TAB_STOP_LAYOUT_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabAlignment {
    #[default]
    Left,
    Right,
    Center,
    Decimal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabLeader {
    #[default]
    None,
    Dots,
    Dashes,
    Solid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabStop {
    /// Inline-axis position in PDF points relative to the line's leading edge.
    pub position: f64,
    #[serde(default)]
    pub alignment: TabAlignment,
    /// Decimal alignment character. It is meaningful only for `decimal`.
    #[serde(default = "default_decimal")]
    pub decimal: char,
    /// Optional exact decimal token. When present it replaces `decimal` for
    /// alignment and may contain multiple Unicode scalars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimal_token: Option<String>,
    /// Decorative artifact between the preceding occupied field and this
    /// field's resolved origin. It never becomes logical text.
    #[serde(default, skip_serializing_if = "TabLeader::is_none")]
    pub leader: TabLeader,
    /// Draw a perpendicular artifact rule at this exact stop when selected.
    #[serde(default, skip_serializing_if = "is_false")]
    pub bar: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl TabLeader {
    pub fn is_none(&self) -> bool {
        *self == Self::None
    }
}

fn default_decimal() -> char {
    '.'
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabStops {
    /// Sorted explicit stops. When none can accept the following field, layout
    /// continues on the declared default interval beyond the last stop.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stops: Vec<TabStop>,
    #[serde(default = "default_interval")]
    pub default_interval: f64,
}

fn default_interval() -> f64 {
    36.0
}

impl Default for TabStops {
    fn default() -> Self {
        Self {
            stops: Vec::new(),
            default_interval: default_interval(),
        }
    }
}

impl TabStops {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// True when the settings require the decoration-aware tab protocol.
    ///
    /// Keeping this decision next to the serialized model prevents saved-story
    /// and durable-history version selection from drifting apart.
    pub fn has_decorations(&self) -> bool {
        self.stops
            .iter()
            .any(|stop| !stop.leader.is_none() || stop.bar)
    }

    pub fn has_extended_decimal(&self) -> bool {
        self.stops.iter().any(|stop| stop.decimal_token.is_some())
    }

    pub fn validate(&self) -> Result<()> {
        if !self.default_interval.is_finite() || self.default_interval <= 0.0 {
            return Err(WellfriendError::invalid_input(
                "tab default interval must be finite and positive",
            ));
        }
        if self.stops.len() > MAX_TAB_STOPS {
            return Err(WellfriendError::ResourceLimit(
                "paragraph exceeds 256 explicit tab stops".into(),
            ));
        }
        let mut previous = 0.0;
        for stop in &self.stops {
            let valid_token = stop.decimal_token.as_ref().is_none_or(|token| {
                !token.is_empty()
                    && token.len() <= 64
                    && token.chars().count() <= 16
                    && token.chars().any(|value| !value.is_whitespace())
                    && !token.chars().any(|value| {
                        value.is_control()
                            || matches!(value, '\t' | '\r' | '\n')
                            || crate::fonts::hard_break::is_hard_break(value)
                    })
                    && stop.alignment == TabAlignment::Decimal
            });
            if !stop.position.is_finite()
                || stop.position <= previous
                || stop.decimal.is_control()
                || matches!(stop.decimal, '\t' | '\r' | '\n')
                || !valid_token
            {
                return Err(WellfriendError::invalid_input(
                    "tab stops require increasing positive finite positions and bounded visible decimal tokens used only by decimal alignment",
                ));
            }
            previous = stop.position;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PositionedTabSegment {
    /// Exact non-tab source range. Empty fields are retained so consecutive
    /// tabs have deterministic stop progression.
    pub range: Range<usize>,
    pub origin: f64,
    pub width: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabLinePlan {
    pub segments: Vec<PositionedTabSegment>,
    pub decorations: Vec<PositionedTabDecoration>,
    pub advance: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PositionedTabDecoration {
    Leader {
        from: f64,
        to: f64,
        leader: TabLeader,
    },
    Bar {
        position: f64,
    },
}

#[derive(Debug, Clone, Copy)]
struct FieldMetrics {
    width: f64,
    decimal_prefix: Option<f64>,
}

fn checked_width(width: f64) -> Result<f64> {
    if !width.is_finite() || width < 0.0 {
        return Err(WellfriendError::invalid_input(
            "tab field measurement must be finite and non-negative",
        ));
    }
    Ok(width)
}

fn decimal_prefix(
    text: &str,
    range: Range<usize>,
    decimal: &str,
    measure: &impl Fn(Range<usize>) -> Result<f64>,
) -> Result<Option<f64>> {
    text[range.clone()]
        .find(decimal)
        .map(|offset| checked_width(measure(range.start..range.start + offset)?))
        .transpose()
}

fn aligned_origin(stop: &TabStop, field: FieldMetrics) -> f64 {
    match stop.alignment {
        TabAlignment::Left => stop.position,
        TabAlignment::Right => stop.position - field.width,
        TabAlignment::Center => stop.position - field.width / 2.0,
        TabAlignment::Decimal => stop.position - field.decimal_prefix.unwrap_or(field.width),
    }
}

fn default_origin(current: f64, field_width: f64, interval: f64) -> Result<f64> {
    let minimum = current.max(0.0);
    let mut slot = (minimum / interval).floor() + 1.0;
    // A right/decimal field that exhausted explicit stops degrades to a left
    // default stop. It never overlaps preceding content or invents backflow.
    let mut origin = slot * interval;
    if origin + EPSILON < current {
        slot += 1.0;
        origin = slot * interval;
    }
    let end = origin + field_width;
    if !origin.is_finite() || !end.is_finite() {
        return Err(WellfriendError::ResourceLimit(
            "tab-stop progression overflow".into(),
        ));
    }
    Ok(origin)
}

/// Position every non-tab field in one already-selected, indivisible logical
/// row. The shared line breaker never soft-wraps within a tabbed hard line.
///
/// `range` must not contain a hard line/page separator. Returned ranges are
/// absolute offsets into `text`; the caller can shape and emit them without
/// reconstituting or searching the source string.
pub fn plan_line(
    text: &str,
    range: Range<usize>,
    settings: &TabStops,
    measure: impl Fn(Range<usize>) -> Result<f64>,
) -> Result<TabLinePlan> {
    settings.validate()?;
    if range.start > range.end
        || range.end > text.len()
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return Err(WellfriendError::invalid_input(
            "tab line range is not an exact UTF-8 boundary",
        ));
    }
    if text[range.clone()]
        .chars()
        .any(crate::fonts::hard_break::is_hard_break)
    {
        return Err(WellfriendError::invalid_input(
            "tab planning requires one logical line without hard separators",
        ));
    }

    let mut tabs = Vec::new();
    for (offset, _) in text[range.clone()].match_indices('\t') {
        if tabs.len() >= MAX_TABS_PER_LINE {
            return Err(WellfriendError::ResourceLimit(
                "logical line exceeds 4096 tab characters".into(),
            ));
        }
        tabs.push(range.start + offset);
    }
    if tabs.is_empty() {
        let width = checked_width(measure(range.clone())?)?;
        return Ok(TabLinePlan {
            segments: vec![PositionedTabSegment {
                range,
                origin: 0.0,
                width,
            }],
            decorations: Vec::new(),
            advance: width,
        });
    }

    let mut ranges = Vec::with_capacity(tabs.len() + 1);
    let mut start = range.start;
    for tab in tabs {
        ranges.push(start..tab);
        start = tab + 1;
    }
    ranges.push(start..range.end);

    let first_width = checked_width(measure(ranges[0].clone())?)?;
    let mut segments = vec![PositionedTabSegment {
        range: ranges[0].clone(),
        origin: 0.0,
        width: first_width,
    }];
    let mut current = first_width;
    let mut decorations = Vec::new();
    let mut explicit_cursor = 0usize;
    for field in ranges.into_iter().skip(1) {
        crate::cancel::check_current_cancel("tab-stop layout")?;
        while explicit_cursor < settings.stops.len()
            && settings.stops[explicit_cursor].position <= current + EPSILON
        {
            explicit_cursor += 1;
        }

        let mut chosen = None;
        let mut chosen_metrics = None;
        let width = checked_width(measure(field.clone())?)?;
        let mut decimal_prefixes = BTreeMap::<String, Option<f64>>::new();
        for (index, stop) in settings.stops.iter().enumerate().skip(explicit_cursor) {
            if index % 32 == 0 {
                crate::cancel::check_current_cancel("tab-stop candidate search")?;
            }
            let decimal_prefix = if stop.alignment == TabAlignment::Decimal {
                let token = stop
                    .decimal_token
                    .clone()
                    .unwrap_or_else(|| stop.decimal.to_string());
                if let Some(prefix) = decimal_prefixes.get(&token) {
                    *prefix
                } else {
                    let prefix = decimal_prefix(text, field.clone(), &token, &measure)?;
                    decimal_prefixes.insert(token, prefix);
                    prefix
                }
            } else {
                None
            };
            let metrics = FieldMetrics {
                width,
                decimal_prefix,
            };
            let origin = aligned_origin(stop, metrics);
            if origin + EPSILON >= current {
                chosen = Some((index, origin));
                chosen_metrics = Some(metrics);
                break;
            }
        }
        let (origin, metrics) = match (chosen, chosen_metrics) {
            (Some((index, origin)), Some(metrics)) => {
                let stop = &settings.stops[index];
                if !stop.leader.is_none() && origin > current + EPSILON {
                    decorations.push(PositionedTabDecoration::Leader {
                        from: current,
                        to: origin,
                        leader: stop.leader,
                    });
                }
                if stop.bar {
                    decorations.push(PositionedTabDecoration::Bar {
                        position: stop.position,
                    });
                }
                explicit_cursor = index + 1;
                (origin, metrics)
            }
            _ => {
                let metrics = FieldMetrics {
                    width,
                    decimal_prefix: None,
                };
                (
                    default_origin(current, metrics.width, settings.default_interval)?,
                    metrics,
                )
            }
        };
        let end = origin + metrics.width;
        if !end.is_finite() {
            return Err(WellfriendError::ResourceLimit(
                "tabbed line extent overflow".into(),
            ));
        }
        segments.push(PositionedTabSegment {
            range: field,
            origin,
            width: metrics.width,
        });
        current = current.max(end);
    }
    Ok(TabLinePlan {
        segments,
        decorations,
        advance: current,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monospace(range: Range<usize>) -> Result<f64> {
        Ok(range.len() as f64 * 10.0)
    }

    #[test]
    fn explicit_alignment_uses_exact_fields_without_expanding_tabs_to_spaces() {
        let text = "A\t12.5\tZ";
        let settings = TabStops {
            stops: vec![
                TabStop {
                    position: 100.0,
                    alignment: TabAlignment::Decimal,
                    decimal: '.',
                    decimal_token: None,
                    leader: TabLeader::None,
                    bar: false,
                },
                TabStop {
                    position: 160.0,
                    alignment: TabAlignment::Right,
                    decimal: '.',
                    decimal_token: None,
                    leader: TabLeader::None,
                    bar: false,
                },
            ],
            default_interval: 36.0,
        };
        let plan = plan_line(text, 0..text.len(), &settings, monospace).unwrap();
        assert_eq!(plan.segments.len(), 3);
        assert_eq!(plan.segments[0].range, 0..1);
        assert_eq!(plan.segments[1].origin, 80.0);
        assert_eq!(plan.segments[2].origin, 150.0);
        assert_eq!(plan.advance, 160.0);
    }

    #[test]
    fn wide_right_aligned_field_advances_to_a_nonoverlapping_later_stop() {
        let text = "prefix\tvery-wide";
        let settings = TabStops {
            stops: vec![TabStop {
                position: 80.0,
                alignment: TabAlignment::Right,
                decimal: '.',
                decimal_token: None,
                leader: TabLeader::None,
                bar: false,
            }],
            default_interval: 40.0,
        };
        let plan = plan_line(text, 0..text.len(), &settings, monospace).unwrap();
        assert_eq!(plan.segments[1].origin, 80.0);
        assert!(plan.segments[1].origin >= plan.segments[0].width);
    }

    #[test]
    fn candidate_search_measures_width_once_and_caches_decimal_prefixes() {
        let text = "A\t12.5";
        let field_calls = std::cell::Cell::new(0usize);
        let settings = TabStops {
            stops: vec![
                TabStop {
                    position: 20.0,
                    alignment: TabAlignment::Decimal,
                    decimal: '.',
                    decimal_token: None,
                    leader: TabLeader::None,
                    bar: false,
                },
                TabStop {
                    position: 30.0,
                    alignment: TabAlignment::Decimal,
                    decimal: '.',
                    decimal_token: None,
                    leader: TabLeader::None,
                    bar: false,
                },
                TabStop {
                    position: 100.0,
                    alignment: TabAlignment::Decimal,
                    decimal: '.',
                    decimal_token: None,
                    leader: TabLeader::None,
                    bar: false,
                },
            ],
            default_interval: 36.0,
        };
        let plan = plan_line(text, 0..text.len(), &settings, |range| {
            if range.start >= 2 {
                field_calls.set(field_calls.get() + 1);
            }
            monospace(range)
        })
        .unwrap();
        assert_eq!(field_calls.get(), 2); // full width plus one cached prefix
        assert_eq!(plan.segments[1].origin, 10.0);
    }

    #[test]
    fn explicit_stop_emits_bounded_leader_and_bar_without_decorating_defaults() {
        let text = "A\tB\tC";
        let settings = TabStops {
            stops: vec![TabStop {
                position: 60.0,
                alignment: TabAlignment::Left,
                decimal: '.',
                decimal_token: None,
                leader: TabLeader::Dots,
                bar: true,
            }],
            default_interval: 40.0,
        };
        assert!(settings.has_decorations());
        let plan = plan_line(text, 0..text.len(), &settings, monospace).unwrap();
        assert_eq!(plan.segments.len(), 3);
        assert_eq!(plan.segments[1].origin, 60.0);
        assert_eq!(
            plan.decorations,
            vec![
                PositionedTabDecoration::Leader {
                    from: 10.0,
                    to: 60.0,
                    leader: TabLeader::Dots,
                },
                PositionedTabDecoration::Bar { position: 60.0 },
            ]
        );
        assert_eq!(plan.segments[2].origin, 80.0);
        assert_eq!(plan.decorations.len(), 2);
        assert!(!TabStops::default().has_decorations());
    }

    #[test]
    fn multi_scalar_decimal_token_aligns_its_exact_prefix() {
        let text = "A\t12::50";
        let settings = TabStops {
            stops: vec![TabStop {
                position: 100.0,
                alignment: TabAlignment::Decimal,
                decimal: '.',
                decimal_token: Some("::".into()),
                leader: TabLeader::None,
                bar: false,
            }],
            default_interval: 36.0,
        };
        assert!(settings.has_extended_decimal());
        let plan = plan_line(text, 0..text.len(), &settings, monospace).unwrap();
        assert_eq!(plan.segments[1].origin, 80.0);
        assert_eq!(plan.segments[1].width, 60.0);

        let mut invalid = settings;
        invalid.stops[0].decimal_token = Some("\n".into());
        assert!(invalid.validate().is_err());
        invalid.stops[0].decimal_token = Some("::".into());
        invalid.stops[0].alignment = TabAlignment::Left;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn tab_count_is_rejected_at_the_declared_bound() {
        let text = "\t".repeat(MAX_TABS_PER_LINE + 1);
        assert!(matches!(
            plan_line(&text, 0..text.len(), &TabStops::default(), monospace),
            Err(WellfriendError::ResourceLimit(_))
        ));
    }
}
