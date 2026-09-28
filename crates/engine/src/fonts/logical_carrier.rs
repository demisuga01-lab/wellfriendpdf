//! Stable codes for non-painting logical content. The first seven codes retain
//! the original hard-line carrier format. Additional ranges match the exact
//! default-ignorable removal policy in pinned Rustybuzz 0.20.1, not a claim about
//! every Unicode version, whitespace character, or arbitrary font substitution.
use crate::{Result, WellfriendError};
use std::collections::BTreeSet;

pub(crate) const SEPARATORS: [char; 7] = [
    '\r', '\n', '\u{000b}', '\u{000c}', '\u{0085}', '\u{2028}', '\u{2029}',
];
// Append new ranges when upgrading the shaping contract; do not renumber codes.
// Source: rustybuzz-0.20.1/src/hb/unicode.rs, is_default_ignorable.
const REMOVED_RANGES: &[(u32, u32)] = &[
    (0x00ad, 0x00ad),
    (0x034f, 0x034f),
    (0x061c, 0x061c),
    (0x17b4, 0x17b5),
    (0x180b, 0x180e),
    (0x200b, 0x200f),
    (0x202a, 0x202e),
    (0x2060, 0x206f),
    (0xfe00, 0xfe0f),
    (0xfeff, 0xfeff),
    (0xfff0, 0xfff8),
    (0x1d173, 0x1d17a),
    (0xe0000, 0xe0fff),
];

pub(crate) fn code(ch: char) -> Option<u16> {
    if let Some(index) = SEPARATORS.iter().position(|candidate| *candidate == ch) {
        return Some(index as u16 + 1);
    }
    let mut base = SEPARATORS.len() as u32 + 1;
    for &(start, end) in REMOVED_RANGES {
        if (start..=end).contains(&(ch as u32)) {
            return u16::try_from(base + ch as u32 - start).ok();
        }
        base += end - start + 1;
    }
    // Tabs were added after the original separator/default-ignorable carrier
    // protocol. Assign the next free code only after walking every historical
    // range so none of those stable codes are renumbered.
    if ch == '\t' {
        return u16::try_from(base).ok();
    }
    None
}

pub(crate) fn is_scalar(ch: char) -> bool {
    code(ch).is_some()
}

pub(crate) fn is_text(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_scalar)
}

/// Always retain the original seven mappings, then include only controls used
/// by this transaction. Unused supplementary ranges do not enlarge the font.
pub(crate) fn alphabet<'a>(texts: impl IntoIterator<Item = &'a str>) -> Result<BTreeSet<char>> {
    let mut result = SEPARATORS.into_iter().collect::<BTreeSet<_>>();
    let mut bytes = 0usize;
    for text in texts {
        crate::cancel::check_current_cancel("logical carrier alphabet")?;
        bytes = bytes.checked_add(text.len()).ok_or_else(|| {
            WellfriendError::ResourceLimit("logical carrier alphabet bytes".into())
        })?;
        if bytes > 16 * 1024 * 1024 {
            return Err(WellfriendError::ResourceLimit(
                "logical carrier alphabet budget".into(),
            ));
        }
        for (index, ch) in text.chars().enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("logical carrier alphabet")?;
            }
            if !is_scalar(ch) {
                return Err(WellfriendError::invalid_input(
                    "logical carrier contains painted or unsupported text",
                ));
            }
            result.insert(ch);
        }
    }
    Ok(result)
}

pub(crate) fn encode(text: &str) -> Result<String> {
    crate::cancel::check_current_cancel("logical carrier encoding")?;
    if text.len() > 4_000_000 {
        return Err(WellfriendError::ResourceLimit(
            "logical carrier text budget".into(),
        ));
    }
    let mut result = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index % 1024 == 0 {
            crate::cancel::check_current_cancel("logical carrier encoding")?;
        }
        let code = code(ch).ok_or_else(|| {
            WellfriendError::invalid_input(
                "logical carrier can only encode hard separators and supported default ignorables",
            )
        })?;
        use std::fmt::Write;
        write!(&mut result, "{code:04X}").expect("writing to String");
    }
    Ok(result)
}

#[cfg(test)]
#[path = "logical_carrier_tests.rs"]
mod tests;
