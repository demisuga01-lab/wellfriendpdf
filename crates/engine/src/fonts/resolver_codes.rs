//! Byte provenance shared by extraction, rendering and source mutation.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedCode {
    pub code: CharacterCode,
    pub byte_start: usize,
    pub byte_end: usize,
}
pub struct CodeIterator<'a> {
    resolver: &'a FontResolver,
    bytes: &'a [u8],
    offset: usize,
    count: usize,
    finished: bool,
}
impl Iterator for CodeIterator<'_> {
    type Item = std::result::Result<DecodedCode, String>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.finished || self.offset >= self.bytes.len() {
            return None;
        }
        let start = self.offset;
        let parsed = (|| {
            if self.count.is_multiple_of(256) {
                crate::cancel::check_current_cancel("PDF character decoding")
                    .map_err(|e| e.to_string())?;
            }
            let space = self.resolver.code_space.as_ref().map_err(Clone::clone)?;
            let code = space.next(self.bytes, &mut self.offset)?;
            Ok(DecodedCode {
                code,
                byte_start: start,
                byte_end: self.offset,
            })
        })();
        self.count += 1;
        if parsed.is_err() {
            self.finished = true;
        }
        Some(parsed)
    }
}
impl std::iter::FusedIterator for CodeIterator<'_> {}

impl FontResolver {
    pub fn next_code(
        &self,
        bytes: &[u8],
        offset: &mut usize,
    ) -> std::result::Result<CharacterCode, String> {
        self.code_space
            .as_ref()
            .map_err(Clone::clone)?
            .next(bytes, offset)
    }
    pub fn codes<'a>(&'a self, bytes: &'a [u8]) -> CodeIterator<'a> {
        CodeIterator {
            resolver: self,
            bytes,
            offset: 0,
            count: 0,
            finished: false,
        }
    }
    pub fn validate_source_encoding(&self) -> std::result::Result<(), String> {
        self.validate_encoding()?;
        self.unicode_encoding.as_ref().map_err(Clone::clone)?;
        Ok(())
    }
    pub fn try_decode_string(&self, bytes: &[u8]) -> std::result::Result<String, String> {
        self.validate_source_encoding()?;
        let mut result = String::new();
        for code in self.codes(bytes) {
            result.push_str(&self.decode_code(code?.code));
        }
        Ok(result)
    }
    pub(crate) fn encoding_candidates(
        &self,
        text: &str,
    ) -> std::result::Result<Vec<CharacterCode>, String> {
        self.validate_source_encoding()?;
        let space = self.code_space.as_ref().map_err(Clone::clone)?;
        if let Some(map) = &self.to_unicode {
            let candidates = map
                .codes_for_text(text)
                .iter()
                .copied()
                .filter(|code| space.contains(*code))
                .collect::<Vec<_>>();
            return Ok(candidates);
        }
        let mut candidates = Vec::new();
        // This compatibility fallback covers simple encodings and legacy
        // Identity inference. Never enumerate a 32-bit character domain.
        if self.font_type.is_cid() {
            let mut chars = text.chars();
            if let (Some(ch), None, Some(length)) =
                (chars.next(), chars.next(), space.fixed_length())
            {
                if let Ok(code) = CharacterCode::new(ch as u32, length) {
                    if space.contains(code) && self.decode_code(code) == text {
                        candidates.push(code);
                    }
                }
            }
        } else if let Some(length @ 1) = space.fixed_length() {
            let limit = 256;
            for value in 0..limit {
                if value % 1024 == 0 {
                    crate::cancel::check_current_cancel("source font reverse mapping")
                        .map_err(|e| e.to_string())?;
                }
                let code = CharacterCode::new(value, length)?;
                if space.contains(code) && self.decode_code(code) == text {
                    candidates.push(code);
                }
            }
        }
        Ok(candidates)
    }

    /// Encode through actual source mappings, including multi-scalar ligatures.
    /// Chooses a minimum-code path and reports any alternative complete path;
    /// callers still decide whether an ambiguous mapping is acceptable.
    pub fn try_encode_existing(&self, text: &str) -> std::result::Result<(Vec<u8>, bool), String> {
        self.validate_source_encoding()?;
        if text.len() > 1024 * 1024 {
            return Err("source-font encoding text budget".into());
        }
        #[derive(Clone, Copy)]
        struct Step {
            cost: usize,
            ways: u8,
            previous: usize,
            code: Option<CharacterCode>,
        }
        let mut steps = vec![None; text.len() + 1];
        steps[0] = Some(Step {
            cost: 0,
            ways: 1,
            previous: 0,
            code: None,
        });
        let mut work = 0usize;
        let mut simple = std::collections::BTreeMap::new();
        for (index, (start, ch)) in text.char_indices().enumerate() {
            if index % 256 == 0 {
                crate::cancel::check_current_cancel("source-font encoding paths")
                    .map_err(|e| e.to_string())?;
            }
            let Some(previous) = steps[start] else {
                continue;
            };
            let candidates = if let Some(map) = &self.to_unicode {
                map.matching_prefixes(&text[start..], &mut work)?
            } else {
                if let std::collections::btree_map::Entry::Vacant(e) = simple.entry(ch) {
                    e.insert(self.encoding_candidates(&ch.to_string())?);
                }
                simple[&ch]
                    .iter()
                    .map(|code| (ch.len_utf8(), *code))
                    .collect()
            };
            for (length, code) in candidates {
                let end = start + length;
                if end > text.len() || length == 0 {
                    return Err("invalid reverse CMap boundary".into());
                }
                let candidate = Step {
                    cost: previous.cost + 1,
                    ways: previous.ways,
                    previous: start,
                    code: Some(code),
                };
                match &mut steps[end] {
                    slot @ None => *slot = Some(candidate),
                    Some(old) => {
                        let ways = old.ways.saturating_add(candidate.ways).min(2);
                        if candidate.cost < old.cost {
                            *old = candidate;
                        }
                        old.ways = ways;
                    }
                }
            }
        }
        let Some(last) = steps[text.len()] else {
            return Err("existing font/CMap has no complete encoding for replacement".into());
        };
        let mut reversed = Vec::with_capacity(last.cost);
        let mut end = text.len();
        while end != 0 {
            if reversed.len() % 256 == 0 {
                crate::cancel::check_current_cancel("source-font encoding reconstruction")
                    .map_err(|e| e.to_string())?;
            }
            let step = steps[end].ok_or("incomplete reverse CMap path")?;
            reversed.push(step.code.ok_or("missing reverse CMap code")?);
            end = step.previous;
        }
        let mut bytes = Vec::new();
        for (index, code) in reversed.into_iter().rev().enumerate() {
            if index % 256 == 0 {
                crate::cancel::check_current_cancel("source-font encoding output")
                    .map_err(|e| e.to_string())?;
            }
            bytes.extend(code.bytes());
        }
        Ok((bytes, last.ways > 1))
    }
}
