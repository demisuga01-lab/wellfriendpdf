//! Offline predefined CMap resources, with exact case-sensitive names.
//! See cmaps/manifest.json for pinned source revisions, hashes and licences.
use super::character_code::CharacterCode;
use super::cmap_program::{Kind, Program, Result};
use std::sync::Arc;
#[path = "predefined_resources.rs"]
mod resources;
#[path = "predefined_unicode.rs"]
mod unicode;
pub(crate) use unicode::{font_unicode, validate_font_system};
#[cfg(test)]
#[path = "predefined_cmap_tests.rs"]
pub(crate) mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredefinedCMapInfo {
    pub name: &'static str,
    pub collection: &'static str,
    pub vertical: bool,
    /// Zero means mixed code lengths; decode through FontResolver::codes().
    pub code_size: u8,
    pub unicode_preserving: bool,
}
pub fn lookup(name: &str) -> Option<PredefinedCMapInfo> {
    let name = name.trim_start_matches('/');
    resources::METADATA
        .iter()
        .copied()
        .find(|entry| entry.name == name)
}
pub fn supported_names() -> &'static [PredefinedCMapInfo] {
    resources::METADATA
}
pub fn code_size_for_name(name: &str) -> Option<u8> {
    lookup(name).map(|entry| entry.code_size)
}
pub fn wmode_from_name(name: &str) -> Option<u8> {
    lookup(name).map(|entry| u8::from(entry.vertical))
}
pub fn is_supported_name(name: &str) -> bool {
    lookup(name).is_some()
}
pub(crate) fn load_program(name: &str, kind: Kind) -> Result<Arc<Program>> {
    resources::load(name, kind)
}

// Compatibility helper for Unicode-encoded CMaps only. This is not a CID
// decoder: Identity and legacy encodings need their character collection.
pub fn unicode_for_code(name: &str, code: u16) -> Option<String> {
    let asset = resources::asset(name, Kind::Cid)?;
    let length = match asset.unicode_encoding {
        2 | 16 => 2,
        32 => 4,
        8 => {
            if code < 256 {
                1
            } else {
                2
            }
        }
        _ => return None,
    };
    unicode_encoded(name, CharacterCode::new(u32::from(code), length).ok()?)
}
pub(super) fn unicode_encoded(name: &str, code: CharacterCode) -> Option<String> {
    let asset = resources::asset(name, Kind::Cid)?;
    decode_unicode_encoding(asset.unicode_encoding, code)
}
pub(super) fn decode_unicode_encoding(encoding: u8, code: CharacterCode) -> Option<String> {
    let bytes = code.bytes();
    let text = match encoding {
        8 => std::str::from_utf8(&bytes).ok()?.to_string(),
        2 | 16 if bytes.len().is_multiple_of(2) => String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|p| u16::from_be_bytes([p[0], p[1]]))
                .collect::<Vec<_>>(),
        )
        .ok()?,
        32 if bytes.len() == 4 => char::from_u32(code.value())?.to_string(),
        _ => return None,
    };
    (text.chars().count() == 1).then_some(text)
}
pub fn looks_like_predefined_name(name: &str) -> bool {
    let name = name.trim_start_matches('/');
    is_supported_name(name)
        || name.starts_with("Adobe-")
        || name.starts_with("Uni")
        || name.ends_with("-H")
        || name.ends_with("-V")
}
