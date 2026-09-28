//! Generated-text hard-line policy, shared by layout, coverage and emission.
//! This does not interpret PDF content-stream whitespace or source glyph codes.
use crate::Result;
use std::ops::Range;
use unicode_linebreak::{break_property, BreakClass};

pub(super) fn is_hard_break_class(class: BreakClass) -> bool {
    matches!(
        class,
        BreakClass::Mandatory
            | BreakClass::CarriageReturn
            | BreakClass::LineFeed
            | BreakClass::NextLine
    )
}
pub fn is_hard_break(c: char) -> bool {
    is_hard_break_class(break_property(c as u32))
}

/// PDF story/authoring pagination treats form feed as a physical-page command,
/// not merely as another mandatory line boundary. Keep this test separate from
/// `is_hard_break`: VT, NEL, LS and PS remain line-only controls.
pub(crate) fn is_form_feed(c: char) -> bool {
    c == '\u{000c}'
}

pub(crate) fn range_ends_with_form_feed(text: &str, range: &Range<usize>) -> bool {
    text.get(range.clone())
        .is_some_and(|line| line.chars().next_back().is_some_and(is_form_feed))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HardLine {
    /// Includes its separator, without an invented empty final line.
    pub logical: Range<usize>,
    /// Excludes the separator but preserves every other source character.
    pub visible: Range<usize>,
}
pub(crate) struct HardLines<'a> {
    text: &'a str,
    cursor: usize,
}
pub(crate) fn logical_lines(text: &str) -> HardLines<'_> {
    HardLines { text, cursor: 0 }
}
impl Iterator for HardLines<'_> {
    type Item = Result<HardLine>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor == self.text.len() {
            return None;
        }
        let start = self.cursor;
        for (index, (offset, c)) in self.text[start..].char_indices().enumerate() {
            if index % 1024 == 0 {
                if let Err(error) = crate::cancel::check_current_cancel("hard line scanning") {
                    self.cursor = self.text.len();
                    return Some(Err(error));
                }
            }
            if is_hard_break(c) {
                let visible_end = start + offset;
                let mut end = visible_end + c.len_utf8();
                if c == '\r' && self.text.as_bytes().get(end) == Some(&b'\n') {
                    end += 1;
                }
                self.cursor = end;
                return Some(Ok(HardLine {
                    logical: start..end,
                    visible: start..visible_end,
                }));
            }
        }
        self.cursor = self.text.len();
        Some(Ok(HardLine {
            logical: start..self.cursor,
            visible: start..self.cursor,
        }))
    }
}

#[cfg(test)]
#[path = "hard_break_tests.rs"]
mod tests;
