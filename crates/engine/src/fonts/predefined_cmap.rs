//! Bounded predefined CMap metadata.
//!
//! Full Adobe CMap packs are large data sets. Oxide keeps the complete CMap
//! pack out of the core engine for now, but common UTF-16 predefined CMaps are
//! valuable because the PDF character code is already Unicode scalar data. This
//! module classifies those names, exposes their writing mode, and gives reports
//! a stable way to distinguish supported bounded coverage from clean
//! unsupported predefined CMaps.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredefinedCMapInfo {
    pub name: &'static str,
    pub collection: &'static str,
    pub vertical: bool,
    pub code_size: u8,
    pub unicode_preserving: bool,
}

const SUPPORTED_UTF16_CMAPS: &[PredefinedCMapInfo] = &[
    info("Identity-H", "Identity", false, true),
    info("Identity-V", "Identity", true, true),
    info("UniJIS-UTF16-H", "Adobe-Japan1", false, true),
    info("UniJIS-UTF16-V", "Adobe-Japan1", true, true),
    info("UniGB-UTF16-H", "Adobe-GB1", false, true),
    info("UniGB-UTF16-V", "Adobe-GB1", true, true),
    info("UniCNS-UTF16-H", "Adobe-CNS1", false, true),
    info("UniCNS-UTF16-V", "Adobe-CNS1", true, true),
    info("UniKS-UTF16-H", "Adobe-Korea1", false, true),
    info("UniKS-UTF16-V", "Adobe-Korea1", true, true),
];

const fn info(
    name: &'static str,
    collection: &'static str,
    vertical: bool,
    unicode_preserving: bool,
) -> PredefinedCMapInfo {
    PredefinedCMapInfo {
        name,
        collection,
        vertical,
        code_size: 2,
        unicode_preserving,
    }
}

pub fn lookup(name: &str) -> Option<PredefinedCMapInfo> {
    let clean = name.trim_start_matches('/');
    SUPPORTED_UTF16_CMAPS
        .iter()
        .copied()
        .find(|info| info.name.eq_ignore_ascii_case(clean))
}

pub fn supported_names() -> &'static [PredefinedCMapInfo] {
    SUPPORTED_UTF16_CMAPS
}

pub fn code_size_for_name(name: &str) -> Option<u8> {
    lookup(name).map(|info| info.code_size)
}

pub fn unicode_for_code(name: &str, code: u16) -> Option<String> {
    let info = lookup(name)?;
    if !info.unicode_preserving {
        return None;
    }
    char::from_u32(u32::from(code)).map(|ch| ch.to_string())
}

pub fn wmode_from_name(name: &str) -> Option<u8> {
    lookup(name).map(|info| u8::from(info.vertical))
}

pub fn is_supported_name(name: &str) -> bool {
    lookup(name).is_some()
}

pub fn looks_like_predefined_name(name: &str) -> bool {
    let clean = name.trim_start_matches('/');
    clean == "Identity-H"
        || clean == "Identity-V"
        || clean.starts_with("UniJIS-")
        || clean.starts_with("UniGB-")
        || clean.starts_with("UniCNS-")
        || clean.starts_with("UniKS-")
        || clean.starts_with("Adobe-Japan1-")
        || clean.starts_with("Adobe-GB1-")
        || clean.starts_with("Adobe-CNS1-")
        || clean.starts_with("Adobe-Korea1-")
        || clean.ends_with("-H")
        || clean.ends_with("-V")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_utf16_cmaps_are_supported_and_classified() {
        let jis = lookup("UniJIS-UTF16-V").expect("supported cmap");
        assert_eq!(jis.collection, "Adobe-Japan1");
        assert!(jis.vertical);
        assert_eq!(jis.code_size, 2);
        assert_eq!(
            unicode_for_code("UniJIS-UTF16-H", 0x65E5).as_deref(),
            Some("日")
        );
    }

    #[test]
    fn unsupported_predefined_names_are_detectable_without_claiming_support() {
        assert!(looks_like_predefined_name("90ms-RKSJ-H"));
        assert!(!is_supported_name("90ms-RKSJ-H"));
        assert_eq!(lookup("90ms-RKSJ-H"), None);
    }
}
