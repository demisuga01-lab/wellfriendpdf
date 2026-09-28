//! PDF character codes retain both value and encoded length. CIDs/GIDs remain
//! separate 16-bit selectors. A bounded DFA decodes code-space rectangles in
//! O(input bytes), without expanding three/four-byte spaces into code tables.
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharacterCode {
    value: u32,
    length: u8,
}
impl CharacterCode {
    pub fn new(value: u32, length: u8) -> Result<Self, String> {
        if !(1..=4).contains(&length) || (length < 4 && value >= (1u32 << (u32::from(length) * 8)))
        {
            return Err("PDF character code exceeds its encoded length".into());
        }
        Ok(Self { value, length })
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if !(1..=4).contains(&bytes.len()) {
            return Err("PDF character codes require one through four bytes".into());
        }
        Self::new(
            bytes
                .iter()
                .fold(0, |value, byte| (value << 8) | u32::from(*byte)),
            bytes.len() as u8,
        )
    }
    pub fn value(self) -> u32 {
        self.value
    }
    pub fn len(self) -> u8 {
        self.length
    }
    pub fn is_empty(self) -> bool {
        false
    }
    pub fn bytes(self) -> Vec<u8> {
        self.value.to_be_bytes()[4 - usize::from(self.length)..].to_vec()
    }
    pub fn is_word_space(self) -> bool {
        self.length == 1 && self.value == 32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct CodeRange {
    pub low: CharacterCode,
    pub high: CharacterCode,
}
impl CodeRange {
    pub fn new(low: CharacterCode, high: CharacterCode) -> Result<Self, String> {
        if low.len() != high.len() || low.bytes().iter().zip(high.bytes()).any(|(a, b)| *a > b) {
            return Err("invalid per-byte PDF code-space range".into());
        }
        Ok(Self { low, high })
    }
    fn bound(self, depth: usize) -> (u8, u8) {
        let shift = 8 * (usize::from(self.low.len()) - depth - 1);
        (
            (self.low.value() >> shift) as u8,
            (self.high.value() >> shift) as u8,
        )
    }
}
const INVALID: u16 = u16::MAX;
const ACCEPT: u16 = u16::MAX - 1;

#[derive(Debug, Clone)]
pub struct CodeSpace {
    nodes: Vec<[u16; 256]>,
    ranges: Vec<CodeRange>,
    fixed: Option<u8>,
}

impl CodeSpace {
    pub fn fixed(length: u8) -> Result<Self, String> {
        let high = if length == 4 {
            u32::MAX
        } else if (1..4).contains(&length) {
            (1u32 << (u32::from(length) * 8)) - 1
        } else {
            return Err("invalid fixed PDF code length".into());
        };
        let ranges = vec![CodeRange::new(
            CharacterCode::new(0, length)?,
            CharacterCode::new(high, length)?,
        )?];
        let nodes = (0..length)
            .map(|depth| {
                [if depth + 1 == length {
                    ACCEPT
                } else {
                    u16::from(depth) + 1
                }; 256]
            })
            .collect();
        Ok(Self {
            nodes,
            ranges,
            fixed: Some(length),
        })
    }
    pub(crate) fn new(mut ranges: Vec<CodeRange>) -> Result<Self, String> {
        ranges.sort_unstable();
        ranges.dedup();
        if ranges.is_empty() || ranges.len() > 4096 {
            return Err("PDF code-space declaration budget or missing code space".into());
        }
        let fixed = ranges
            .first()
            .map(|r| r.low.len())
            .filter(|len| ranges.iter().all(|r| r.low.len() == *len));
        let mut compiler = Compiler {
            ranges: &ranges,
            nodes: Vec::new(),
            memo: BTreeMap::new(),
            work: 0,
        };
        compiler.state(0, (0..ranges.len()).collect())?;
        Ok(Self {
            nodes: compiler.nodes,
            ranges,
            fixed,
        })
    }
    pub fn fixed_length(&self) -> Option<u8> {
        self.fixed
    }
    pub(crate) fn ranges(&self) -> &[CodeRange] {
        &self.ranges
    }
    pub fn contains(&self, code: CharacterCode) -> bool {
        let storage = code.value().to_be_bytes();
        let bytes = &storage[4 - usize::from(code.len())..];
        let mut at = 0;
        self.next(bytes, &mut at)
            .is_ok_and(|parsed| parsed == code && at == bytes.len())
    }
    /// Failure does not advance the caller's byte offset or pad missing bytes.
    pub fn next(&self, bytes: &[u8], offset: &mut usize) -> Result<CharacterCode, String> {
        let start = *offset;
        let input = bytes
            .get(start..)
            .ok_or_else(|| format!("PDF character-code offset exceeds input at byte {start}"))?;
        let mut state = 0usize;
        let mut value = 0u32;
        for depth in 0..4 {
            let byte = *input.get(depth).ok_or_else(|| {
                self.fixed.map_or_else(
                    || {
                        format!(
                            "truncated PDF character code at byte {start} of {}",
                            bytes.len()
                        )
                    },
                    |length| {
                        format!(
                            "truncated PDF character code: incomplete {length}-byte character code at byte {start} of {}",
                            bytes.len()
                        )
                    },
                )
            })?;
            value = (value << 8) | u32::from(byte);
            match self.nodes[state][usize::from(byte)] {
                ACCEPT => {
                    *offset = start + depth + 1;
                    return CharacterCode::new(value, (depth + 1) as u8);
                }
                INVALID => {
                    return Err(format!(
                        "PDF character code outside code space at byte {start}"
                    ))
                }
                next => state = usize::from(next),
            }
        }
        Err("PDF character code exceeds four bytes".into())
    }
}

struct Compiler<'a> {
    ranges: &'a [CodeRange],
    nodes: Vec<[u16; 256]>,
    memo: BTreeMap<(usize, Vec<usize>), u16>,
    work: usize,
}
impl Compiler<'_> {
    fn state(&mut self, depth: usize, active: Vec<usize>) -> Result<u16, String> {
        crate::cancel::check_current_cancel("PDF code-space compilation")
            .map_err(|e| e.to_string())?;
        let key = (depth, active.clone());
        if let Some(index) = self.memo.get(&key) {
            return Ok(*index);
        }
        self.work = self.work.saturating_add(active.len().saturating_mul(256));
        if self.nodes.len() >= 4096 || self.work > 16_000_000 {
            return Err("PDF code-space DFA compilation budget".into());
        }
        let index = self.nodes.len() as u16;
        self.nodes.push([INVALID; 256]);
        self.memo.insert(key, index);
        for byte in 0u16..256 {
            let matching = active
                .iter()
                .copied()
                .filter(|id| {
                    let (lo, hi) = self.ranges[*id].bound(depth);
                    u16::from(lo) <= byte && byte <= u16::from(hi)
                })
                .collect::<Vec<_>>();
            if matching.is_empty() {
                continue;
            }
            let terminal = matching
                .iter()
                .any(|id| usize::from(self.ranges[*id].low.len()) == depth + 1);
            let longer = matching
                .iter()
                .any(|id| usize::from(self.ranges[*id].low.len()) > depth + 1);
            if terminal && longer {
                return Err("ambiguous prefix-overlapping PDF code spaces".into());
            }
            let next = if terminal {
                ACCEPT
            } else {
                self.state(depth + 1, matching)?
            };
            self.nodes[usize::from(index)][usize::from(byte)] = next;
        }
        Ok(index)
    }
}
