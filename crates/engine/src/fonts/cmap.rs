use super::character_code::{CharacterCode, CodeSpace};
use super::cmap_program::{Kind, Program};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use std::collections::{BTreeMap, HashMap};
const MAX_CMAP_MAPPINGS: usize = 65_536;

/// Length-aware ToUnicode mappings. The compatibility parser exposes no partial
/// map after malformed input; destructive consumers can inspect validate().
#[derive(Clone)]
pub struct ToUnicodeCMap {
    program: Option<std::sync::Arc<Program>>,
    error: Option<String>,
    reverse: std::sync::Arc<BTreeMap<String, Vec<CharacterCode>>>,
}
impl ToUnicodeCMap {
    pub fn parse(bytes: &[u8]) -> Self {
        match Self::try_parse(bytes) {
            Ok(map) => map,
            Err(error) => Self {
                program: None,
                error: Some(error),
                reverse: std::sync::Arc::new(BTreeMap::new()),
            },
        }
    }
    pub fn try_parse(bytes: &[u8]) -> std::result::Result<Self, String> {
        Program::parse(bytes, Kind::Unicode, None, true).and_then(Self::from_program)
    }
    pub(crate) fn load(object: &PdfObject, reader: Option<&PdfReader>) -> Self {
        match super::cmap_stream::read(object, reader, Kind::Unicode, 0)
            .and_then(Self::from_program)
        {
            Ok(map) => map,
            Err(error) => Self {
                program: None,
                error: Some(error),
                reverse: std::sync::Arc::new(BTreeMap::new()),
            },
        }
    }
    pub(crate) fn from_program(program: Program) -> std::result::Result<Self, String> {
        let mut reverse: BTreeMap<String, Vec<CharacterCode>> = BTreeMap::new();
        for (index, (code, text)) in program.unicode.iter().enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("ToUnicode reverse indexing")
                    .map_err(|e| e.to_string())?;
            }
            reverse.entry(text.clone()).or_default().push(*code);
        }
        Ok(Self {
            program: Some(std::sync::Arc::new(program)),
            error: None,
            reverse: std::sync::Arc::new(reverse),
        })
    }
    pub fn validate(&self) -> std::result::Result<(), String> {
        self.error
            .as_ref()
            .map_or(Ok(()), |error| Err(error.clone()))
    }
    pub(crate) fn failed(error: String) -> Self {
        Self {
            program: None,
            error: Some(error),
            reverse: std::sync::Arc::new(BTreeMap::new()),
        }
    }
    pub(crate) fn validate_codes(&self, space: &CodeSpace) -> std::result::Result<(), String> {
        self.validate()?;
        if let Some(program) = &self.program {
            if std::ptr::eq(program.space.as_ref(), space) {
                return Ok(());
            }
            for (index, code) in program.unicode.keys().enumerate() {
                if index % 1024 == 0 {
                    crate::cancel::check_current_cancel("ToUnicode/Encoding consistency")
                        .map_err(|e| e.to_string())?;
                }
                if !space.contains(*code) {
                    return Err("ToUnicode source code contradicts font Encoding code space".into());
                }
            }
        }
        Ok(())
    }
    pub fn lookup_code(&self, code: CharacterCode) -> Option<&str> {
        self.program
            .as_ref()?
            .unicode
            .get(&code)
            .map(String::as_str)
    }
    pub fn lookup(&self, code: u16) -> Option<&str> {
        self.lookup_code(CharacterCode::new(u32::from(code), self.code_size()).ok()?)
    }
    /// Zero denotes mixed code lengths. Use space()/the resolver byte decoder.
    pub fn code_size(&self) -> u8 {
        self.program
            .as_ref()
            .map_or(1, |program| program.space.fixed_length().unwrap_or(0))
    }
    pub fn space(&self) -> Option<&CodeSpace> {
        self.program.as_ref().map(|program| program.space.as_ref())
    }
    pub(crate) fn shared_space(&self) -> Option<std::sync::Arc<CodeSpace>> {
        self.program
            .as_ref()
            .map(|program| std::sync::Arc::clone(&program.space))
    }
    pub fn codes_for_text(&self, text: &str) -> &[CharacterCode] {
        self.reverse.get(text).map(Vec::as_slice).unwrap_or(&[])
    }
    pub(crate) fn matching_prefixes(
        &self,
        text: &str,
        work: &mut usize,
    ) -> std::result::Result<Vec<(usize, CharacterCode)>, String> {
        let Some(first) = text.chars().next() else {
            return Ok(Vec::new());
        };
        let prefix = first.to_string();
        let mut out = Vec::new();
        for (index, (logical, codes)) in self.reverse.range(prefix.clone()..).enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("ToUnicode reverse matching")
                    .map_err(|e| e.to_string())?;
            }
            if !logical.starts_with(&prefix) {
                break;
            }
            *work = work
                .saturating_add(logical.len())
                .saturating_add(codes.len());
            if *work > 16_000_000 {
                return Err("source CMap reverse matching budget".into());
            }
            if text.starts_with(logical) {
                out.extend(codes.iter().map(|code| (logical.len(), *code)));
            }
        }
        Ok(out)
    }
    pub fn is_empty(&self) -> bool {
        self.program
            .as_ref()
            .is_none_or(|program| program.unicode.is_empty())
    }
}

/// Compatibility projection for clients needing one scalar per numeric code.
/// Multi-scalar mappings and numeric values with conflicting encoded lengths
/// are omitted; the length-aware ToUnicodeCMap is the authoritative interface.
pub fn parse_to_unicode_cmap(bytes: &[u8]) -> HashMap<u32, char> {
    scalar_projection(&ToUnicodeCMap::parse(bytes))
}
fn scalar_projection(map: &ToUnicodeCMap) -> HashMap<u32, char> {
    let Some(program) = &map.program else {
        return HashMap::new();
    };
    let mut scalars: BTreeMap<u32, Option<char>> = BTreeMap::new();
    for (index, (code, text)) in program.unicode.iter().enumerate() {
        if index % 1024 == 0
            && crate::cancel::check_current_cancel("ToUnicode scalar projection").is_err()
        {
            return HashMap::new();
        }
        let mut chars = text.chars();
        let first = chars.next();
        let scalar = if chars.next().is_none() { first } else { None };
        scalars
            .entry(code.value())
            .and_modify(|old| {
                if *old != scalar {
                    *old = None
                }
            })
            .or_insert(scalar);
    }
    scalars
        .into_iter()
        .filter_map(|(code, ch)| ch.map(|ch| (code, ch)))
        .take(MAX_CMAP_MAPPINGS)
        .collect()
}
pub fn extract_to_unicode_map(
    font: &PdfDictionary,
    reader: &PdfReader,
) -> Option<HashMap<u32, char>> {
    let map = ToUnicodeCMap::load(font.get("ToUnicode")?, Some(reader));
    map.validate().ok()?;
    Some(scalar_projection(&map))
}
#[cfg(test)]
fn parse_bf_char_line(line: &str, map: &mut HashMap<u32, char>) {
    if map.len() >= MAX_CMAP_MAPPINGS {
        return;
    }
    let parsed = parse_to_unicode_cmap(format!("1 beginbfchar {line} endbfchar").as_bytes());
    for (code, ch) in parsed {
        if map.len() >= MAX_CMAP_MAPPINGS {
            break;
        }
        map.insert(code, ch);
    }
}
#[cfg(test)]
fn parse_bf_range_line(line: &str, map: &mut HashMap<u32, char>) {
    if map.len() >= MAX_CMAP_MAPPINGS {
        return;
    }
    let parsed = parse_to_unicode_cmap(format!("1 beginbfrange {line} endbfrange").as_bytes());
    for (code, ch) in parsed {
        if map.len() >= MAX_CMAP_MAPPINGS {
            break;
        }
        map.insert(code, ch);
    }
}

#[cfg(test)]
mod cid_cmap_tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn parse_bf_char_line_maps_single_character() {
        let mut map = HashMap::new();
        parse_bf_char_line("<0041> <0041>", &mut map);
        assert_eq!(map.get(&0x41), Some(&'A'));
    }

    #[test]
    fn parse_bf_range_line_contiguous_maps_sequence() {
        let mut map = HashMap::new();
        parse_bf_range_line("<0041> <0043> <0041>", &mut map);
        assert_eq!(map.get(&0x41), Some(&'A'));
        assert_eq!(map.get(&0x42), Some(&'B'));
        assert_eq!(map.get(&0x43), Some(&'C'));
    }

    #[test]
    fn parse_bf_range_line_array_maps_explicit_values() {
        let mut map = HashMap::new();
        parse_bf_range_line("<0041> <0043> [<0058> <0059> <005A>]", &mut map);
        assert_eq!(map.get(&0x41), Some(&'X'));
        assert_eq!(map.get(&0x42), Some(&'Y'));
        assert_eq!(map.get(&0x43), Some(&'Z'));
    }

    #[test]
    fn parse_to_unicode_cmap_handles_bfchar_and_bfrange() {
        let cmap_text = b"beginbfchar\n<0020> <0020>\n<002E> <002E>\nendbfchar\n\
                          beginbfrange\n<0041> <005A> <0041>\nendbfrange";
        let map = parse_to_unicode_cmap(cmap_text);
        assert_eq!(map.get(&0x20), Some(&' '));
        assert_eq!(map.get(&0x41), Some(&'A'));
        assert_eq!(map.get(&0x5A), Some(&'Z'));
        assert_eq!(map.get(&0x5B), None);
    }

    #[test]
    fn parse_to_unicode_cmap_handles_multi_byte_cids() {
        let cmap_text = b"beginbfchar\n<3042> <3042>\nendbfchar";
        let map = parse_to_unicode_cmap(cmap_text);
        assert_eq!(map.get(&0x3042), Some(&'\u{3042}'));
    }

    #[test]
    fn parse_to_unicode_cmap_empty_input_returns_empty_map() {
        let map = parse_to_unicode_cmap(b"");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_to_unicode_cmap_handles_multiple_blocks() {
        let cmap = b"\
            beginbfchar\n<0041> <0041>\nendbfchar\n\
            beginbfchar\n<0042> <0042>\nendbfchar\n\
            beginbfrange\n<0043> <0044> <0043>\nendbfrange";
        let map = parse_to_unicode_cmap(cmap);
        assert_eq!(map.len(), 4);
        assert_eq!(map.get(&0x41), Some(&'A'));
        assert_eq!(map.get(&0x44), Some(&'D'));
    }

    #[test]
    fn parse_bf_lines_stop_at_mapping_cap() {
        let mut map = HashMap::new();
        for cid in 0..MAX_CMAP_MAPPINGS as u32 {
            map.insert(cid, 'A');
        }

        parse_bf_char_line("<10000> <0042>", &mut map);
        parse_bf_range_line("<10001> <1FFFF> <0043>", &mut map);

        assert_eq!(map.len(), MAX_CMAP_MAPPINGS);
        assert_eq!(map.get(&0x10000), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_beginbfchar() {
        let cmap = b"
        /CIDInit /ProcSet findresource begin
        begincmap
        2 beginbfchar
        <41> <0041>
        <42> <0042>
        endbfchar
        endcmap end
        ";
        let parsed = ToUnicodeCMap::parse(cmap);
        assert_eq!(parsed.lookup(0x41), Some("A"));
        assert_eq!(parsed.lookup(0x42), Some("B"));
        assert_eq!(parsed.code_size(), 1);
    }

    #[test]
    fn parses_beginbfrange_scalar() {
        let cmap = b"
        begincmap
        1 beginbfrange
        <0041> <0046> <0041>
        endbfrange
        endcmap
        ";
        let parsed = ToUnicodeCMap::parse(cmap);
        assert_eq!(parsed.lookup(0x0041), Some("A"));
        assert_eq!(parsed.lookup(0x0044), Some("D"));
        assert_eq!(parsed.lookup(0x0046), Some("F"));
        assert_eq!(parsed.lookup(0x0047), None);
    }

    #[test]
    fn parses_beginbfrange_array() {
        let cmap = b"
        begincmap
        1 beginbfrange
        <20> <21> [<0048> <0049>]
        endbfrange
        endcmap
        ";
        let parsed = ToUnicodeCMap::parse(cmap);
        assert_eq!(parsed.lookup(0x20), Some("H"));
        assert_eq!(parsed.lookup(0x21), Some("I"));
        assert_eq!(parsed.code_size(), 1);
    }

    #[test]
    fn detects_two_byte_codes() {
        let cmap = b"
        begincmap
        1 beginbfchar
        <0041> <0041>
        endbfchar
        endcmap
        ";
        let parsed = ToUnicodeCMap::parse(cmap);
        assert_eq!(parsed.code_size(), 2);
        assert_eq!(parsed.lookup(0x0041), Some("A"));
    }
}
