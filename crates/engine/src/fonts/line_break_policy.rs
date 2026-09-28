//! Explicit paragraph wrap policy, layered over the pinned Unicode line-break
//! implementation. This is not CSS layout, automatic hyphenation or full JLREQ.
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use unicode_linebreak::{break_property, BreakClass};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineBreakProfile {
    #[default]
    Unicode,
    JapaneseStrict,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmergencyWrap {
    /// Split overlong letter/number sequences and preserved space runs only at grapheme boundaries.
    /// Explicit nonbreaking controls and punctuation restrictions still win.
    #[default]
    BreakWord,
    /// An overlong unbreakable sequence requires a wider frame or a new policy.
    PreserveWords,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineComposition {
    /// Bounded local width probes, intended for responsive layout.
    #[default]
    Greedy,
    /// Whole forced-break segment DAG, with explicit work-budget errors.
    /// Prefer fewer emergency breaks, fewer lines, then squared normalized slack.
    Balanced,
}
impl LineComposition {
    fn is_greedy(&self) -> bool {
        *self == Self::Greedy
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineBreakSettings {
    #[serde(default)]
    pub profile: LineBreakProfile,
    #[serde(default)]
    pub emergency: EmergencyWrap,
    #[serde(default, skip_serializing_if = "LineComposition::is_greedy")]
    pub composition: LineComposition,
    /// Additional scalar characters prohibited at a soft line start/end.
    /// Whitespace and explicit breaks are not permitted in these sets.
    #[serde(default)]
    pub prohibit_start: String,
    #[serde(default)]
    pub prohibit_end: String,
}

#[derive(Clone, Copy)]
struct ClusterEdge {
    base: Option<char>,
    class: Option<BreakClass>,
    space: bool,
    hard: bool,
    trailing_joiner: bool,
    no_start: bool,
    no_end: bool,
}

impl LineBreakSettings {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    pub fn validate(&self) -> Result<()> {
        for values in [&self.prohibit_start, &self.prohibit_end] {
            if values.len() > 4096 || values.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return Err(WellfriendError::invalid_input(
                    "line-break prohibition sets must contain at most 4096 UTF-8 bytes of non-whitespace, non-control characters",
                ));
            }
        }
        Ok(())
    }

    /// One pass over graphemes, apart from set lookups/removals. Significant
    /// edge characters are propagated once; a long space run is never rescanned
    /// for every potential break. Mandatory source breaks always take priority.
    pub(super) fn apply(
        &self,
        text: &str,
        boundaries: &[usize],
        allowed: &mut BTreeSet<usize>,
        mandatory: &[usize],
    ) -> Result<Vec<usize>> {
        self.validate()?;
        let starts = self.prohibit_start.chars().collect::<BTreeSet<_>>();
        let ends = self.prohibit_end.chars().collect::<BTreeSet<_>>();
        let mut edges = Vec::with_capacity(boundaries.len().saturating_sub(1));
        for (index, span) in boundaries.windows(2).enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("paragraph break policy")?;
            }
            let cluster = &text[span[0]..span[1]];
            let base = cluster
                .chars()
                .find(|c| {
                    !matches!(
                        break_property(*c as u32),
                        BreakClass::CombiningMark | BreakClass::ZeroWidthJoiner
                    )
                })
                .or_else(|| cluster.chars().next());
            let class = base.map(|c| break_property(c as u32));
            let hard = class.is_some_and(super::hard_break::is_hard_break_class);
            // A trailing ZWJ belongs to the preceding grapheme (GB9), but its
            // LB8a no-break rule still applies at the following boundary. The
            // grapheme's base class alone would incorrectly allow AL / AL.
            let trailing_joiner = cluster
                .chars()
                .next_back()
                .is_some_and(|c| break_property(c as u32) == BreakClass::ZeroWidthJoiner);
            edges.push(ClusterEdge {
                base,
                class,
                space: cluster.chars().all(char::is_whitespace),
                hard,
                trailing_joiner,
                // Custom sets constrain the entire indivisible grapheme, not
                // just its base: a caller can explicitly restrict a mark too.
                no_start: cluster.chars().any(|c| starts.contains(&c)),
                no_end: cluster.chars().any(|c| ends.contains(&c)),
            });
        }
        let mut right = vec![None; edges.len() + 1];
        for index in (0..edges.len()).rev() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("paragraph right-edge policy")?;
            }
            let edge = edges[index];
            right[index] = if edge.hard {
                None
            } else if edge.space {
                right[index + 1]
            } else {
                edge.base.map(|base| (base, edge.no_start))
            };
        }
        let mut emergency = Vec::new();
        let mut left = None;
        for index in 1..boundaries.len() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("paragraph break indexing")?;
            }
            let edge = edges[index - 1];
            if edge.hard {
                left = None;
            } else if !edge.space {
                left = edge.base.map(|base| (base, edge.no_end));
            }
            let offset = boundaries[index];
            if mandatory.binary_search(&offset).is_ok() {
                continue;
            }
            let next = right[index];
            let prohibited = left.is_some_and(|(_, blocked)| blocked)
                || next.is_some_and(|(_, blocked)| blocked)
                || (self.profile == LineBreakProfile::JapaneseStrict
                    && (left.is_some_and(|(c, _)| japanese_no_end(c))
                        || next.is_some_and(|(c, _)| japanese_no_start(c))));
            if prohibited {
                allowed.remove(&offset);
                continue;
            }
            // Emergency word wrapping is deliberately narrower than CSS
            // line-break:anywhere. Never discard a WJ/GL/ZWJ or split emoji,
            // opening/closing punctuation, or a combining grapheme to make fit.
            if self.emergency == EmergencyWrap::BreakWord
                && !edge.trailing_joiner
                && (edge.class.is_some_and(word_class)
                    && edges
                        .get(index)
                        .and_then(|edge| edge.class)
                        .is_some_and(word_class)
                    || edge.class == Some(BreakClass::Space)
                        && edges.get(index).and_then(|edge| edge.class) == Some(BreakClass::Space))
            {
                emergency.push(offset);
            }
        }
        Ok(emergency)
    }
}

fn word_class(class: BreakClass) -> bool {
    matches!(
        class,
        BreakClass::Alphabetic
            | BreakClass::HebrewLetter
            | BreakClass::Numeric
            | BreakClass::ComplexContext
    )
}
fn japanese_no_start(c: char) -> bool {
    matches!(
        break_property(c as u32),
        BreakClass::ClosePunctuation
            | BreakClass::CloseParenthesis
            | BreakClass::Exclamation
            | BreakClass::NonStarter
            | BreakClass::ConditionalJapaneseStarter
            | BreakClass::Inseparable
    ) || matches!(
        c,
        '\u{2010}'
            | '\u{2013}'
            | '\u{301c}'
            | '\u{30a0}'
            | '\u{3005}'
            | '\u{303b}'
            | '\u{309d}'
            | '\u{309e}'
            | '\u{30fd}'
            | '\u{30fe}'
            | '\u{2019}'
            | '\u{201d}'
    )
}
fn japanese_no_end(c: char) -> bool {
    break_property(c as u32) == BreakClass::OpenPunctuation || matches!(c, '\u{2018}' | '\u{201c}')
}

#[cfg(test)]
#[path = "line_break_policy_tests.rs"]
mod tests;
