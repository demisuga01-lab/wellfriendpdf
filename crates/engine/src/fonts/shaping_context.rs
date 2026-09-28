//! Bounded non-emitting context plus an exact nearest-nontransparent joining
//! synopsis. The latter bridges arbitrarily long transparent runs inside the
//! supported paragraph budget without increasing Rustybuzz's five-scalar window.
use super::hard_break::is_hard_break as hard_boundary;
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use std::ops::Range;
#[path = "joining_properties.rs"]
mod properties;

pub const CONTEXT_SCALARS: usize = 5;
const MAX_TEXT_BYTES: usize = 4_000_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShapingContext {
    #[serde(default)]
    pub before: String,
    #[serde(default)]
    pub after: String,
    /// The actual nearest nontransparent source scalar beyond an all-transparent
    /// raw edge. Not a guessed joiner or an extra emitted character.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joining_before: Option<char>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joining_after: Option<char>,
}

fn check_range(text: &str, range: &Range<usize>) -> Result<()> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(WellfriendError::ResourceLimit(
            "shaping context input exceeds 4 MB".into(),
        ));
    }
    if range.start > range.end
        || range.end > text.len()
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return Err(WellfriendError::invalid_input(
            "invalid shaping context range",
        ));
    }
    Ok(())
}
fn needs_synopsis(raw: &str) -> bool {
    raw.chars().count() == CONTEXT_SCALARS && raw.chars().all(properties::transparent)
}
fn synopsis(raw: &str, nearest: Option<char>) -> Option<char> {
    if needs_synopsis(raw) {
        nearest.filter(|c| !hard_boundary(*c))
    } else {
        None
    }
}
fn raw_edge(chars: impl Iterator<Item = char>, reverse: bool) -> String {
    let mut edge = chars
        .take_while(|c| !hard_boundary(*c))
        .take(CONTEXT_SCALARS)
        .collect::<Vec<_>>();
    if reverse {
        edge.reverse();
    }
    edge.into_iter().collect()
}
fn scan_edge(
    chars: impl Iterator<Item = char>,
    inherited: Option<char>,
    reverse: bool,
) -> Result<(String, Option<char>)> {
    let mut edge = Vec::with_capacity(CONTEXT_SCALARS);
    let mut nearest = None;
    let mut stopped_at_break = false;
    for (index, c) in chars.enumerate() {
        if index % 1024 == 0 {
            crate::cancel::check_current_cancel("joining context scan")?;
        }
        if hard_boundary(c) {
            stopped_at_break = true;
            break;
        }
        if edge.len() < CONTEXT_SCALARS {
            edge.push(c);
        }
        if nearest.is_none() && !properties::transparent(c) {
            nearest = Some(c);
        }
        if edge.len() == CONTEXT_SCALARS && nearest.is_some() {
            break;
        }
    }
    if nearest.is_none() && !stopped_at_break {
        nearest = inherited;
    }
    if reverse {
        edge.reverse();
    }
    let raw: String = edge.into_iter().collect();
    let joining = synopsis(&raw, nearest);
    Ok((raw, joining))
}

/// A sparse index built once with paragraph bidi preparation, so each width
/// probe does not scan a long mark/control sequence again. u32 offsets cover
/// the 4 MB paragraph limit without a machine-word index per scalar.
pub(super) struct JoiningIndex<'a> {
    text: &'a str,
    nontransparent: Vec<u32>,
}
impl<'a> JoiningIndex<'a> {
    pub(super) fn new(text: &'a str) -> Result<Self> {
        check_range(text, &(0..text.len()))?;
        let mut nontransparent = Vec::new();
        for (index, (offset, c)) in text.char_indices().enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("joining paragraph index")?;
            }
            if !properties::transparent(c) || hard_boundary(c) {
                nontransparent.push(offset as u32);
            }
        }
        Ok(Self {
            text,
            nontransparent,
        })
    }
    pub(super) fn context(&self, range: Range<usize>) -> Result<ShapingContext> {
        check_range(self.text, &range)?;
        crate::cancel::check_current_cancel("indexed joining context")?;
        let before = raw_edge(self.text[..range.start].chars().rev(), true);
        let after = raw_edge(self.text[range.end..].chars(), false);
        let char_at = |offset: &u32| self.text[*offset as usize..].chars().next();
        let left = self
            .nontransparent
            .partition_point(|n| (*n as usize) < range.start);
        let right = self
            .nontransparent
            .partition_point(|n| (*n as usize) < range.end);
        let nearest_before = left
            .checked_sub(1)
            .and_then(|i| self.nontransparent.get(i))
            .and_then(char_at);
        let nearest_after = self.nontransparent.get(right).and_then(char_at);
        let joining_before = synopsis(&before, nearest_before);
        let joining_after = synopsis(&after, nearest_after);
        Ok(ShapingContext {
            before,
            after,
            joining_before,
            joining_after,
        })
    }
}

impl ShapingContext {
    pub fn is_empty(&self) -> bool {
        self.before.is_empty()
            && self.after.is_empty()
            && self.joining_before.is_none()
            && self.joining_after.is_none()
    }
    pub fn validate(&self) -> Result<()> {
        for (edge, joining) in [
            (&self.before, self.joining_before),
            (&self.after, self.joining_after),
        ] {
            if edge.len() > 4 * CONTEXT_SCALARS
                || edge.chars().count() > CONTEXT_SCALARS
                || edge.chars().any(hard_boundary)
            {
                return Err(WellfriendError::invalid_input(
                    "shaping context exceeds the pinned five-scalar edge or crosses a hard break",
                ));
            }
            if joining.is_some_and(|c| {
                !needs_synopsis(edge) || properties::transparent(c) || hard_boundary(c)
            }) {
                return Err(WellfriendError::invalid_input("joining synopsis must be nontransparent and extend a full transparent context edge"));
            }
        }
        Ok(())
    }

    /// Compose local and outer logical context, stopping at a forced break.
    /// The synopsis retains only the first character the joining state machine
    /// would observe after skipping transparent scalars.
    pub fn slice(&self, text: &str, range: Range<usize>) -> Result<Self> {
        self.validate()?;
        check_range(text, &range)?;
        crate::cancel::check_current_cancel("joining context slice")?;
        let (before, joining_before) = scan_edge(
            text[..range.start]
                .chars()
                .rev()
                .chain(self.before.chars().rev()),
            self.joining_before,
            true,
        )?;
        let (after, joining_after) = scan_edge(
            text[range.end..].chars().chain(self.after.chars()),
            self.joining_after,
            false,
        )?;
        Ok(Self {
            before,
            after,
            joining_before,
            joining_after,
        })
    }

    pub(crate) fn apply(
        &self,
        buffer: &mut rustybuzz::UnicodeBuffer,
        text: &str,
        range: Range<usize>,
    ) -> Result<()> {
        let context = self.slice(text, range)?;
        let before = match context.joining_before {
            Some(c) => std::iter::once(c)
                .chain(context.before.chars().skip(1))
                .collect::<String>(),
            None => context.before,
        };
        let after = match context.joining_after {
            Some(c) => context
                .after
                .chars()
                .take(CONTEXT_SCALARS - 1)
                .chain(std::iter::once(c))
                .collect::<String>(),
            None => context.after,
        };
        // Only one distant transparent carrier is replaced, by the actual
        // nearest nontransparent source scalar. The pinned joining machine
        // skips the remaining transparent carriers then observes that scalar.
        // Context length/presence stays unchanged for dotted-circle handling.
        // Neither context edge is added to the glyph buffer.
        buffer.set_pre_context(&before);
        buffer.set_post_context(&after);
        Ok(())
    }
}
#[cfg(test)]
#[path = "joining_synopsis_tests.rs"]
mod synopsis_tests;
#[cfg(test)]
#[path = "shaping_context_tests.rs"]
mod tests;
