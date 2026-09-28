//! Shared, bounded declarative CMap grammar. No PostScript execution and no
//! scanning for operator substrings inside strings, comments or dictionaries.
use super::character_code::{CharacterCode as Code, CodeRange, CodeSpace};
use crate::content::tokenizer::{ContentToken as Token, ContentTokenizer};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Cid,
    Unicode,
}
#[derive(Debug, Clone)]
pub(crate) struct Program {
    pub space: std::sync::Arc<CodeSpace>,
    pub cids: BTreeMap<Code, u16>,
    pub notdef: BTreeMap<Code, u16>,
    pub unicode: BTreeMap<Code, String>,
    pub identity: bool,
    pub wmode: Option<u8>,
    pub wmode_declared: bool,
    pub name: Option<String>,
    pub system: Option<super::pdf_embedding::CidSystem>,
    pub system_declared: bool,
}
impl Program {
    pub fn cid(&self, code: Code) -> u16 {
        self.cids
            .get(&code)
            .copied()
            .or_else(|| {
                self.identity
                    .then(|| u16::try_from(code.value()).ok())
                    .flatten()
            })
            .or_else(|| self.notdef.get(&code).copied())
            .unwrap_or(0)
    }
    /// Legacy repair is limited to omitted record counts/code spaces, not
    /// malformed tokens, truncated ranges or ambiguous mappings.
    pub fn parse(
        bytes: &[u8],
        kind: Kind,
        mut inherited: Option<Self>,
        repair_omissions: bool,
    ) -> Result<Self> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("CMap decoded stream budget".into());
        }
        let mut tokens = Vec::new();
        for token in ContentTokenizer::new(bytes) {
            if tokens.len() % 1024 == 0 {
                check("CMap tokenization")?;
            }
            if tokens.len() >= 1_000_000 {
                return Err("CMap token budget".into());
            }
            tokens.push(token.map_err(|e| e.to_string())?);
        }
        let mut cids = BTreeMap::new();
        let mut notdef = BTreeMap::new();
        let mut unicode = BTreeMap::new();
        let mut spaces = Vec::new();
        let mut wmode = None;
        let mut name = None;
        let mut system = None;
        let mut literals = Vec::new();
        let mut dictionaries = 0usize;
        let mut cmap_depth = 0usize;
        let mut saw_range = false;
        let mut at = 0usize;
        let mut work = 0usize;
        let mut unicode_bytes = 0usize;
        while at < tokens.len() {
            if at.is_multiple_of(256) {
                check("CMap parsing")?;
            }
            match &tokens[at] {
                Token::ArrayStart | Token::DictStart => {
                    if literals.len() >= 64 {
                        return Err("CMap metadata nesting limit".into());
                    }
                    literals.push(matches!(&tokens[at], Token::DictStart));
                    at += 1;
                    continue;
                }
                Token::ArrayEnd | Token::DictEnd => {
                    if literals.pop() != Some(matches!(&tokens[at], Token::DictEnd)) {
                        return Err("unbalanced CMap metadata".into());
                    }
                    at += 1;
                    continue;
                }
                _ if !literals.is_empty() => {
                    if matches!(&tokens[at], Token::Operator(_) | Token::InlineImageData(_)) {
                        return Err("executable CMap metadata".into());
                    }
                    at += 1;
                    continue;
                }
                Token::Name(key) if key == "CIDSystemInfo" && dictionaries == cmap_depth => {
                    let value = read_system(&tokens, at + 1)?;
                    if system
                        .replace(value.clone())
                        .is_some_and(|old| old != value)
                    {
                        return Err("conflicting CMap CIDSystemInfo".into());
                    }
                }
                Token::Name(name) if name == "WMode" && dictionaries == cmap_depth => {
                    let value = integer(tokens.get(at + 1))?;
                    let value = u8::try_from(value)
                        .ok()
                        .filter(|v| *v <= 1)
                        .ok_or("invalid CMap WMode")?;
                    if wmode.replace(value).is_some_and(|old| old != value) {
                        return Err("conflicting CMap WMode".into());
                    }
                }
                Token::Name(key) if key == "CMapName" && dictionaries == cmap_depth => {
                    let Some(Token::Name(value)) = tokens.get(at + 1) else {
                        return Err("invalid CMapName".into());
                    };
                    if name.replace(value.clone()).is_some_and(|old| old != *value) {
                        return Err("conflicting CMapName".into());
                    }
                }
                _ => {}
            }
            let Token::Operator(op) = &tokens[at] else {
                at += 1;
                continue;
            };
            match op.as_str() {
                "begin" => {
                    dictionaries += 1;
                    if dictionaries > 64 {
                        return Err("CMap dictionary nesting limit".into());
                    }
                    at += 1;
                    continue;
                }
                "end" => {
                    dictionaries = dictionaries
                        .checked_sub(1)
                        .ok_or("unbalanced CMap dictionary")?;
                    at += 1;
                    continue;
                }
                "begincmap" => {
                    cmap_depth = dictionaries;
                    at += 1;
                    continue;
                }
                "usecmap" => {
                    let name = at.checked_sub(1).and_then(|i| tokens.get(i));
                    if saw_range || dictionaries != cmap_depth {
                        return Err("late CMap inheritance".into());
                    }
                    if let Some(base) = &inherited {
                        if !matches!(name,Some(Token::Name(name)) if base.name.as_ref()==Some(name))
                        {
                            return Err("conflicting CMap inheritance declarations".into());
                        }
                    } else {
                        let Some(Token::Name(name)) = name else {
                            return Err("CMap inheritance requires a resource name".into());
                        };
                        inherited =
                            Some((*super::predefined_cmap::load_program(name, kind)?).clone());
                    }
                    at += 1;
                    continue;
                }
                "findresource" | "endcmap" | "def" | "pop" | "dict" | "currentdict"
                | "defineresource" | "dup" | "readonly" | "CMapName" => {
                    at += 1;
                    continue;
                }
                _ => {}
            }
            let (range, space, missing, unicode_record, end) = match op.as_str() {
                "begincodespacerange" => (true, true, false, false, "endcodespacerange"),
                "begincidchar" if kind == Kind::Cid => (false, false, false, false, "endcidchar"),
                "begincidrange" if kind == Kind::Cid => (true, false, false, false, "endcidrange"),
                "beginnotdefchar" if kind == Kind::Cid => {
                    (false, false, true, false, "endnotdefchar")
                }
                "beginnotdefrange" if kind == Kind::Cid => {
                    (true, false, true, false, "endnotdefrange")
                }
                "beginbfchar" if kind == Kind::Unicode => (false, false, false, true, "endbfchar"),
                "beginbfrange" if kind == Kind::Unicode => (true, false, false, true, "endbfrange"),
                _ => return Err(format!("unsupported declarative CMap operator {op}")),
            };
            if dictionaries != cmap_depth {
                return Err("CMap records outside mapping dictionary".into());
            }
            if space
                && (inherited.is_some()
                    || !cids.is_empty()
                    || !notdef.is_empty()
                    || !unicode.is_empty())
            {
                return Err("CMap code space redefined after mappings or inheritance".into());
            }
            if !space && !repair_omissions && inherited.is_none() && spaces.is_empty() {
                return Err("CMap mapping precedes its code space".into());
            }
            let count = at.checked_sub(1).and_then(|i| tokens.get(i)).and_then(|t| {
                if let Token::Integer(n) = t {
                    Some(*n)
                } else {
                    None
                }
            });
            let count = match count {
                Some(n) => Some(
                    usize::try_from(n)
                        .ok()
                        .filter(|n| *n <= 65536)
                        .ok_or("CMap record count limit")?,
                ),
                None if repair_omissions => None,
                None => return Err("missing CMap record count".into()),
            };
            at += 1;
            saw_range = true;
            let mut records = 0usize;
            while count.is_none_or(|count| records < count) {
                if count.is_none() && matches!(tokens.get(at),Some(Token::Operator(op)) if op==end)
                {
                    break;
                }
                if records >= 65536 {
                    return Err("CMap block record limit".into());
                }
                let low = code(tokens.get(at))?;
                at += 1;
                let high = if range {
                    let high = code(tokens.get(at))?;
                    at += 1;
                    high
                } else {
                    low
                };
                if high.len() != low.len() || high.value() < low.value() {
                    return Err("invalid CMap source range".into());
                }
                records += 1;
                if space {
                    spaces.push(CodeRange::new(low, high)?);
                    if spaces.len() > 4096 {
                        return Err("CMap code-space declaration limit".into());
                    }
                    continue;
                }
                let length = u64::from(high.value()) - u64::from(low.value()) + 1;
                work = work
                    .checked_add(usize::try_from(length).map_err(|_| "CMap expansion budget")?)
                    .ok_or("CMap expansion budget")?;
                if work > 1_000_000 {
                    return Err("CMap expansion budget".into());
                }
                if unicode_record {
                    let array = matches!(tokens.get(at), Some(Token::ArrayStart));
                    if array && !range {
                        return Err(
                            "ToUnicode bfchar destination must be a hexadecimal string".into()
                        );
                    }
                    let initial = if array {
                        at += 1;
                        None
                    } else {
                        let bytes = hex(tokens.get(at))?.to_vec();
                        at += 1;
                        Some(bytes)
                    };
                    for delta in 0..length as u32 {
                        if delta % 256 == 0 {
                            check("ToUnicode range expansion")?;
                        }
                        let target = if let Some(initial) = &initial {
                            increment(initial, delta)?
                        } else {
                            let target = hex(tokens.get(at))?.to_vec();
                            at += 1;
                            target
                        };
                        let target = unicode_string(&target)?;
                        unicode_bytes = unicode_bytes.saturating_add(target.len());
                        if unicode_bytes > 64 * 1024 * 1024 {
                            return Err("ToUnicode decoded text budget".into());
                        }
                        unicode.insert(Code::new(low.value() + delta, low.len())?, target);
                    }
                    if array {
                        if !matches!(tokens.get(at), Some(Token::ArrayEnd)) {
                            return Err("ToUnicode range array count mismatch".into());
                        }
                        at += 1;
                    }
                } else {
                    let cid = u16::try_from(integer(tokens.get(at))?)
                        .map_err(|_| "CID destination outside 16 bits")?;
                    at += 1;
                    let target = if missing { &mut notdef } else { &mut cids };
                    for delta in 0..length as u32 {
                        if delta % 1024 == 0 {
                            check("CID range expansion")?;
                        }
                        let mapped = if missing {
                            cid
                        } else {
                            u16::try_from(
                                u32::from(cid)
                                    .checked_add(delta)
                                    .ok_or("CID range overflow")?,
                            )
                            .map_err(|_| "CID range overflow")?
                        };
                        // CMap mappings are ordered: later definitions override
                        // earlier ranges, including inherited entries. The work
                        // budget counts assignments, not only distinct keys.
                        target.insert(Code::new(low.value() + delta, low.len())?, mapped);
                    }
                }
            }
            if !matches!(tokens.get(at),Some(Token::Operator(op)) if op==end) {
                return Err("CMap record count/end mismatch".into());
            }
            at += 1;
        }
        if !literals.is_empty() || dictionaries != 0 {
            return Err("unterminated CMap metadata".into());
        }
        if spaces.is_empty() && inherited.is_none() && repair_omissions {
            let codes = unicode
                .keys()
                .chain(cids.keys())
                .chain(notdef.keys())
                .copied()
                .collect::<BTreeSet<_>>();
            let lengths = codes.iter().map(|code| code.len()).collect::<BTreeSet<_>>();
            if lengths.len() == 1 {
                spaces = CodeSpace::fixed(*lengths.first().unwrap())?
                    .ranges()
                    .to_vec();
            } else {
                spaces = codes
                    .iter()
                    .map(|code| CodeRange::new(*code, *code))
                    .collect::<Result<_>>()?;
            }
        }
        let mut result = match inherited {
            Some(base) => base,
            None => Self {
                space: CodeSpace::new(spaces)?.into(),
                cids: BTreeMap::new(),
                notdef: BTreeMap::new(),
                unicode: BTreeMap::new(),
                identity: false,
                wmode: None,
                wmode_declared: false,
                name: None,
                system: None,
                system_declared: false,
            },
        };
        let outside_codes = cids
            .keys()
            .chain(notdef.keys())
            .chain(unicode.keys())
            .copied()
            .filter(|code| !result.space.contains(*code))
            .collect::<BTreeSet<_>>();
        if !outside_codes.is_empty() {
            if !repair_omissions {
                return Err("CMap mapping outside declared code space".into());
            }
            // ToUnicode maps in deployed PDFs sometimes declare a code-space
            // rectangle that accidentally excludes one or more mapped codes.
            // Those mappings are unambiguous evidence of their encoded length.
            // Extend the bounded DFA with exact singleton ranges rather than
            // rejecting the entire font or guessing a variable-width prefix.
            let mut repaired_ranges = result.space.ranges().to_vec();
            repaired_ranges.extend(
                outside_codes
                    .iter()
                    .copied()
                    .map(|code| CodeRange::new(code, code))
                    .collect::<Result<Vec<_>>>()?,
            );
            result.space = CodeSpace::new(repaired_ranges)?.into();
        }
        for (index, code) in cids
            .keys()
            .chain(notdef.keys())
            .chain(unicode.keys())
            .enumerate()
        {
            if index % 1024 == 0 {
                check("CMap code-space validation")?;
            }
            if !result.space.contains(*code) {
                return Err("CMap mapping outside declared code space".into());
            }
        }
        result.cids.extend(cids);
        result.notdef.extend(notdef);
        result.unicode.extend(unicode);
        if result.cids.len() + result.notdef.len() + result.unicode.len() > 1_000_000 {
            return Err("inherited CMap mapping budget".into());
        }
        let mut total = 0usize;
        for (index, text) in result.unicode.values().enumerate() {
            if index % 1024 == 0 {
                check("inherited ToUnicode budget")?;
            }
            total = total.saturating_add(text.len());
            if total > 64 * 1024 * 1024 {
                return Err("inherited ToUnicode text budget".into());
            }
        }
        result.wmode_declared = wmode.is_some();
        if wmode.is_some() {
            result.wmode = wmode;
        }
        result.name = name;
        result.system_declared = system.is_some();
        if let Some(system) = system {
            if result.system.as_ref().is_some_and(|base| {
                base.ordering.as_slice() != b"Identity"
                    && (base.registry != system.registry || base.ordering != system.ordering)
            }) {
                return Err("inherited CMap character collection mismatch".into());
            }
            result.system = Some(system);
        }
        Ok(result)
    }
}
// CMap metadata uses either a literal dictionary or the conventional
// `3 dict dup begin ... end` declaration. Read only its immediate entries;
// names in nested dictionaries and strings cannot override the collection.
fn read_system(tokens: &[Token], mut at: usize) -> Result<super::pdf_embedding::CidSystem> {
    let literal = matches!(tokens.get(at), Some(Token::DictStart));
    if literal {
        at += 1;
    } else {
        if !matches!(tokens.get(at), Some(Token::Integer(_)))
            || !matches!(tokens.get(at+1), Some(Token::Operator(op)) if op=="dict")
            || !matches!(tokens.get(at+2), Some(Token::Operator(op)) if op=="dup")
            || !matches!(tokens.get(at+3), Some(Token::Operator(op)) if op=="begin")
        {
            return Err("unsupported CMap CIDSystemInfo declaration".into());
        }
        at += 4;
    }
    let (mut registry, mut ordering, mut supplement) = (None, None, None);
    let mut depth = 1usize;
    while let Some(token) = tokens.get(at) {
        match token {
            Token::DictStart | Token::ArrayStart => depth += 1,
            Token::DictEnd | Token::ArrayEnd => depth -= 1,
            Token::Operator(op) if !literal && op == "begin" => depth += 1,
            Token::Operator(op) if !literal && op == "end" => depth -= 1,
            Token::Name(key) if depth == 1 && (key == "Registry" || key == "Ordering") => {
                let bytes = match tokens.get(at + 1) {
                    Some(Token::LiteralString(bytes) | Token::HexString(bytes))
                        if bytes.len() <= 256 =>
                    {
                        bytes.clone()
                    }
                    _ => return Err("invalid CMap character collection string".into()),
                };
                let slot = if key == "Registry" {
                    &mut registry
                } else {
                    &mut ordering
                };
                if slot.replace(bytes.clone()).is_some_and(|old| old != bytes) {
                    return Err("conflicting CMap character collection string".into());
                }
            }
            Token::Name(key) if depth == 1 && key == "Supplement" => {
                let number = u32::try_from(integer(tokens.get(at + 1))?)
                    .map_err(|_| "invalid CMap supplement")?;
                if supplement.replace(number).is_some_and(|old| old != number) {
                    return Err("conflicting CMap supplement".into());
                }
            }
            _ => {}
        }
        if depth == 0 {
            return Ok(super::pdf_embedding::CidSystem {
                registry: registry.ok_or("missing CMap Registry")?,
                ordering: ordering.ok_or("missing CMap Ordering")?,
                supplement: supplement.ok_or("missing CMap Supplement")?,
            });
        }
        if depth > 64 {
            return Err("CMap system metadata depth limit".into());
        }
        at += 1;
    }
    Err("unterminated CMap CIDSystemInfo".into())
}
fn check(label: &str) -> Result<()> {
    crate::cancel::check_current_cancel(label).map_err(|e| e.to_string())
}
fn hex(token: Option<&Token>) -> Result<&[u8]> {
    match token {
        Some(Token::HexString(bytes)) => Ok(bytes),
        _ => Err("expected CMap hexadecimal string".into()),
    }
}
fn code(token: Option<&Token>) -> Result<Code> {
    Code::from_bytes(hex(token)?)
}
fn integer(token: Option<&Token>) -> Result<i64> {
    match token {
        Some(Token::Integer(n)) => Ok(*n),
        _ => Err("expected CMap integer".into()),
    }
}
fn unicode_string(bytes: &[u8]) -> Result<String> {
    if bytes.len() > 512 {
        return Err("ToUnicode destination exceeds 512 bytes".into());
    }
    if !bytes.len().is_multiple_of(2) {
        return Err("ToUnicode destination is not UTF-16BE".into());
    }
    String::from_utf16(
        &bytes
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>(),
    )
    .map_err(|_| "invalid UTF-16BE ToUnicode destination".into())
}
fn increment(bytes: &[u8], delta: u32) -> Result<Vec<u8>> {
    let mut output = bytes.to_vec();
    if output.is_empty() {
        return Err("empty ToUnicode range destination".into());
    }
    // A bfrange string destination is a big-endian byte string.  Real Adobe
    // mapping resources cross 0xff boundaries, so addition must propagate a
    // carry through the complete destination string.  Treating only the last
    // byte as incrementable rejects valid vendored CMaps such as
    // Adobe-CNS1-UCS2.
    let mut carry = u64::from(delta);
    for byte in output.iter_mut().rev() {
        if carry == 0 {
            break;
        }
        let sum = u64::from(*byte) + (carry & 0xff);
        *byte = (sum & 0xff) as u8;
        carry = (carry >> 8) + (sum >> 8);
    }
    if carry != 0 {
        return Err("ToUnicode destination increment overflow".into());
    }
    Ok(output)
}
