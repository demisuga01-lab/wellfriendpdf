//! Source validation before the parser mutates variation coordinates. Its lazy
//! fvar/avar views alone do not establish that the requested mapping is usable.
use super::variation_store::{bytes, u16_at, u32_at};
use crate::{Result, WellfriendError};
use std::collections::BTreeSet;
use ttf_parser::{Face, Tag};

fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font coordinates: {message}"))
}

/// Returns zero for a static/zero-axis font. Only layouts the current parser
/// actually consumes are accepted; future-version records are not misstrided.
pub(super) fn validate(face: &Face<'_>) -> Result<usize> {
    let Some(raw) = face.raw_face().table(Tag::from_bytes(b"fvar")) else {
        return Ok(0);
    };
    if u32_at(raw, 0)? != 0x00010000 || u16_at(raw, 6)? != 2 {
        return Err(fail("unsupported fvar header"));
    }
    let axes = usize::from(u16_at(raw, 8)?);
    // OpenType explicitly treats a zero-axis fvar as a non-variable font.
    if axes == 0 {
        return Ok(0);
    }
    if axes > 64 {
        return Err(WellfriendError::ResourceLimit(
            "font coordinate definitions exceed 64 axes".into(),
        ));
    }
    if u16_at(raw, 10)? != 20 {
        return Err(WellfriendError::UnsupportedFeature(
            "font coordinate parser requires 20-byte fvar axis records".into(),
        ));
    }
    let start = usize::from(u16_at(raw, 4)?);
    if start < 16 {
        return Err(fail("fvar axes overlap the header"));
    }
    let records = bytes(raw, start, axes * 20)?;
    let mut tags = BTreeSet::new();
    for record in records.chunks_exact(20) {
        crate::cancel::check_current_cancel("font axis definitions")?;
        let limits =
            [4, 8, 12].map(|at| i32::from_be_bytes(record[at..at + 4].try_into().unwrap()));
        // The parser clamps invalid min/max to default, hiding malformed input.
        if limits[0] > limits[1] || limits[1] > limits[2] || !tags.insert(u32_at(record, 0)?) {
            return Err(fail("unordered axis limits or duplicate axis tags"));
        }
    }
    let instances = usize::from(u16_at(raw, 12)?);
    let size = usize::from(u16_at(raw, 14)?);
    if instances != 0 {
        if size != axes * 4 + 4 && size != axes * 4 + 6 {
            return Err(fail("invalid fvar instance record size"));
        }
        bytes(raw, start + axes * 20, instances * size)?;
    }
    if usize::from(face.variation_axes().len()) != axes {
        return Err(fail("parser did not retain the declared axes"));
    }
    if let Some(avar) = face.raw_face().table(Tag::from_bytes(b"avar")) {
        validate_avar(avar, axes)?;
    }
    Ok(axes)
}

fn validate_avar(raw: &[u8], axes: usize) -> Result<()> {
    if u32_at(raw, 0)? != 0x00010000 {
        return Err(WellfriendError::UnsupportedFeature(
            "font coordinate parser requires avar version 1.0".into(),
        ));
    }
    if u16_at(raw, 4)? != 0 || usize::from(u16_at(raw, 6)?) != axes {
        return Err(fail("avar header or axis count disagrees with fvar"));
    }
    let mut at = 8;
    for _ in 0..axes {
        let count = usize::from(u16_at(raw, at)?);
        at += 2;
        let records = bytes(raw, at, count * 4)?;
        at += count * 4;
        let mut previous = None;
        let mut anchors = [false; 3];
        for (index, record) in records.chunks_exact(4).enumerate() {
            if index % 256 == 0 {
                crate::cancel::check_current_cancel("font axis mapping")?;
            }
            let from = i16::from_be_bytes(record[..2].try_into().unwrap());
            let to = i16::from_be_bytes(record[2..].try_into().unwrap());
            if !(-16384..=16384).contains(&from)
                || !(-16384..=16384).contains(&to)
                || previous.is_some_and(|(f, t)| f >= from || t > to)
            {
                return Err(fail("unordered or out-of-domain avar mapping"));
            }
            previous = Some((from, to));
            for (index, anchor) in [-16384, 0, 16384].into_iter().enumerate() {
                if from == anchor && to == anchor {
                    anchors[index] = true;
                }
            }
        }
        // Empty maps are identity. A malformed non-empty map cannot be passed
        // through: this parser does not implement the spec's identity fallback.
        if count != 0 && anchors.iter().any(|found| !found) {
            return Err(fail("avar mapping lacks required identity anchors"));
        }
    }
    Ok(())
}
