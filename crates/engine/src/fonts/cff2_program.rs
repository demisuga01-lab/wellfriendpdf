//! Bounded CFF2 structure and variation selection. The retained source program
//! is immutable; indices remain file-relative and each glyph owns its actual FD.
use crate::{Result, WellfriendError};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock};

#[path = "cff2_charstring.rs"]
mod charstring;
#[path = "cff2_instance.rs"]
pub(crate) mod instance;
#[cfg(test)]
#[path = "cff2_program_tests.rs"]
pub(crate) mod tests;

pub(super) fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("CFF2: {message}"))
}
pub(super) fn data(bytes: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    bytes
        .get(at..at.checked_add(len).ok_or_else(|| fail("offset overflow"))?)
        .ok_or_else(|| fail("truncated data"))
}
pub(super) fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(data(bytes, at, 2)?.try_into().unwrap()))
}
pub(super) fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(data(bytes, at, 4)?.try_into().unwrap()))
}
fn integer(value: f64) -> Result<usize> {
    if value.is_finite() && value >= 0.0 && value <= u32::MAX as f64 && value.fract() == 0.0 {
        Ok(value as usize)
    } else {
        Err(fail("expected nonnegative integer"))
    }
}

/// Return a DICT number, consuming its payload. Real nibbles are not operators.
pub(super) fn number(bytes: &[u8], at: &mut usize, op: u8, dict: bool) -> Result<Option<f64>> {
    let result = match op {
        28 => {
            let n = i16::from_be_bytes(data(bytes, *at, 2)?.try_into().unwrap());
            *at += 2;
            f64::from(n)
        }
        29 if dict => {
            let n = i32::from_be_bytes(data(bytes, *at, 4)?.try_into().unwrap());
            *at += 4;
            f64::from(n)
        }
        30 if dict => {
            let mut text = String::new();
            'real: loop {
                if text.len() > 96 {
                    return Err(fail("DICT real exceeds budget"));
                }
                let byte = *bytes
                    .get(*at)
                    .ok_or_else(|| fail("unterminated DICT real"))?;
                *at += 1;
                for (position, nibble) in [byte >> 4, byte & 15].into_iter().enumerate() {
                    match nibble {
                        0..=9 => text.push(char::from(b'0' + nibble)),
                        10 => text.push('.'),
                        11 => text.push('e'),
                        12 => text.push_str("e-"),
                        14 => text.push('-'),
                        15 => {
                            if position == 0 && byte & 15 != 15 {
                                return Err(fail("DICT real padding nibble"));
                            }
                            break 'real;
                        }
                        _ => return Err(fail("reserved DICT real nibble")),
                    }
                }
            }
            real(&text)?
        }
        32..=246 => f64::from(op) - 139.0,
        247..=254 => {
            let next = f64::from(*bytes.get(*at).ok_or_else(|| fail("truncated number"))?);
            *at += 1;
            if op < 251 {
                (f64::from(op) - 247.0) * 256.0 + next + 108.0
            } else {
                -(f64::from(op) - 251.0) * 256.0 - next - 108.0
            }
        }
        255 if !dict => {
            let n = i32::from_be_bytes(data(bytes, *at, 4)?.try_into().unwrap());
            *at += 4;
            f64::from(n) / 65536.0
        }
        _ => return Ok(None),
    };
    if !result.is_finite() {
        return Err(fail("non-finite number"));
    }
    Ok(Some(result))
}

fn real(text: &str) -> Result<f64> {
    // BCD has stricter spelling rules than Rust's floating-point parser.
    if matches!(text, "" | "." | "-" | "-.") {
        return Ok(if text.starts_with('-') { -0.0 } else { 0.0 });
    }
    let (mantissa, exponent) = text
        .split_once('e')
        .map_or((text, None), |(m, e)| (m, Some(e)));
    let unsigned = mantissa.strip_prefix('-').unwrap_or(mantissa);
    let (whole, fractional) = unsigned
        .split_once('.')
        .map_or((unsigned, None), |(w, f)| (w, Some(f)));
    if whole.is_empty() && fractional.is_none_or(str::is_empty)
        || whole.len() > 1 && whole.starts_with('0')
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fractional.is_some_and(|f| !f.bytes().all(|b| b.is_ascii_digit()))
        || exponent.is_some_and(|e| {
            let e = e.strip_prefix('-').unwrap_or(e);
            e.is_empty() || e.starts_with('0') || !e.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err(fail("invalid DICT real"));
    }
    text.parse::<f64>().map_err(|_| fail("invalid DICT real"))
}

fn dict(bytes: &[u8]) -> Result<Vec<(u16, Vec<f64>)>> {
    if bytes.len() > 1024 * 1024 {
        return Err(fail("DICT size budget"));
    }
    let mut at = 0;
    let mut steps = 0usize;
    let mut stack = Vec::new();
    let mut out = Vec::new();
    while at < bytes.len() {
        if steps.is_multiple_of(512) {
            crate::cancel::check_current_cancel("CFF2 dictionary")?;
        }
        steps += 1;
        let op = bytes[at];
        at += 1;
        if let Some(value) = number(bytes, &mut at, op, true)? {
            stack.push(value);
            if stack.len() > 513 {
                return Err(fail("DICT operand budget"));
            }
        } else {
            let key = if op == 12 {
                let n = *bytes.get(at).ok_or_else(|| fail("DICT escape"))?;
                at += 1;
                0x0c00 | u16::from(n)
            } else {
                u16::from(op)
            };
            out.push((key, std::mem::take(&mut stack)));
            if out.len() > 65536 {
                return Err(fail("DICT operator budget"));
            }
        }
    }
    if !stack.is_empty() {
        return Err(fail("trailing DICT operands"));
    }
    Ok(out)
}

fn index(bytes: &[u8], start: usize, work: &mut usize) -> Result<Vec<Range<usize>>> {
    let count = u32_at(bytes, start)? as usize;
    *work = work
        .checked_add(count)
        .ok_or_else(|| fail("INDEX work overflow"))?;
    if *work > 1_000_000 {
        return Err(WellfriendError::ResourceLimit(
            "CFF2 INDEX entry budget".into(),
        ));
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    let size = usize::from(*bytes.get(start + 4).ok_or_else(|| fail("INDEX offSize"))?);
    if !(1..=4).contains(&size) {
        return Err(fail("INDEX offSize must be 1..4"));
    }
    let offsets = data(bytes, start + 5, (count + 1) * size)?;
    let base = start + 5 + offsets.len();
    let mut previous = 1;
    let mut result = Vec::with_capacity(count);
    for (i, chunk) in offsets.chunks_exact(size).enumerate() {
        if i % 512 == 0 {
            crate::cancel::check_current_cancel("CFF2 INDEX bounds")?;
        }
        let value = chunk
            .iter()
            .fold(0usize, |v, byte| (v << 8) | usize::from(*byte));
        if value < previous || (i == 0 && value != 1) {
            return Err(fail("INDEX offset order"));
        }
        let end = base
            .checked_add(value - 1)
            .ok_or_else(|| fail("INDEX overflow"))?;
        if end > bytes.len() {
            return Err(fail("INDEX exceeds table"));
        }
        if i != 0 {
            result.push(base + previous - 1..end);
        }
        previous = value;
    }
    Ok(result)
}

struct FontDict {
    private: Range<usize>,
    local: Vec<Range<usize>>,
    vsindex: usize,
    has_vsindex: bool,
}
fn variation_store(
    source: Arc<[u8]>,
    at: usize,
) -> Result<super::variation_store::ItemVariationStore> {
    let length = usize::from(u16_at(&source, at)?);
    let base = at
        .checked_add(2)
        .ok_or_else(|| fail("VariationStore offset"))?;
    let end = if length == 65535 {
        source.len()
    } else {
        base.checked_add(length)
            .ok_or_else(|| fail("VariationStore extent"))?
    };
    let store = super::variation_store::ItemVariationStore::parse(source, base..end)?;
    store.require_cff2_regions_only()?;
    Ok(store)
}

pub(crate) struct Program {
    source: Arc<[u8]>,
    glyphs: Vec<Range<usize>>,
    globals: Vec<Range<usize>>,
    font_dicts: Vec<Range<usize>>,
    dictionaries: Vec<Arc<FontDict>>,
    selection: Vec<usize>,
    store: Option<super::variation_store::ItemVariationStore>,
    pub matrix_scale: f64,
}
impl Program {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        crate::cancel::check_current_cancel("CFF2 program")?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(WellfriendError::ResourceLimit(
                "CFF2 table exceeds 32 MiB".into(),
            ));
        }
        if data(bytes, 0, 2)? != [2, 0] {
            return Err(fail("unsupported header version"));
        }
        let header = usize::from(*bytes.get(2).ok_or_else(|| fail("header"))?);
        if header < 5 {
            return Err(fail("header size"));
        }
        let top_size = usize::from(u16_at(bytes, 3)?);
        let mut top = BTreeMap::new();
        for (key, values) in dict(data(bytes, header, top_size)?)? {
            if ![17, 24, 0x0c24, 0x0c25, 0x0c07].contains(&key) {
                continue; // Unknown DICT operators consume their operands.
            }
            if top.insert(key, values).is_some() {
                return Err(fail("duplicate Top DICT key"));
            }
        }
        let offset = |key| -> Result<Option<usize>> {
            top.get(&key)
                .map(|values| {
                    if values.len() == 1 {
                        integer(values[0])
                    } else {
                        Err(fail("Top DICT offset arity"))
                    }
                })
                .transpose()
        };
        let matrix_scale = if let Some(values) = top.get(&0x0c07) {
            if values.len() != 6
                || values[0] <= 0.0
                || values[0] != values[3]
                || [1, 2, 4, 5].iter().any(|i| values[*i] != 0.0)
            {
                return Err(fail("non-uniform CFF2 FontMatrix"));
            }
            values[0]
        } else {
            0.001
        };
        let mut work = 0;
        let globals = index(bytes, header + top_size, &mut work)?;
        let glyphs = index(
            bytes,
            offset(17)?.ok_or_else(|| fail("missing CharStrings"))?,
            &mut work,
        )?;
        if glyphs.is_empty() || glyphs.len() > 65535 {
            return Err(fail("CFF2/maxp glyph budget"));
        }
        let fds = index(
            bytes,
            offset(0x0c24)?.ok_or_else(|| fail("missing FontDICTINDEX"))?,
            &mut work,
        )?;
        if fds.is_empty() || fds.len() > 65535 {
            return Err(fail("FontDICT count budget"));
        }
        let mut dictionaries = Vec::with_capacity(fds.len());
        let mut private_cache = BTreeMap::<(usize, usize), Arc<FontDict>>::new();
        let mut dict_work = top_size;
        for range in &fds {
            crate::cancel::check_current_cancel("CFF2 private ownership")?;
            charge_dict_work(&mut dict_work, range.len())?;
            let entries = dict(&bytes[range.clone()])?
                .into_iter()
                .filter(|(key, _)| *key == 18)
                .collect::<Vec<_>>();
            if entries.len() != 1 || entries[0].0 != 18 || entries[0].1.len() != 2 {
                return Err(fail("FontDICT Private offset"));
            }
            let length = integer(entries[0].1[0])?;
            let start = integer(entries[0].1[1])?;
            if let Some(shared) = private_cache.get(&(start, length)) {
                dictionaries.push(Arc::clone(shared));
                continue;
            }
            charge_dict_work(&mut dict_work, length)?;
            let mut local = None;
            let mut vsindex = None;
            for (key, values) in dict(data(bytes, start, length)?)? {
                if key == 19 || key == 22 {
                    if values.len() != 1 {
                        return Err(fail("Private structural operand arity"));
                    }
                    let value = integer(values[0])?;
                    if key == 19 {
                        if local.replace(value).is_some() {
                            return Err(fail("duplicate LocalSubr offset"));
                        }
                    } else if vsindex.replace(value).is_some() {
                        return Err(fail("duplicate Private vsindex"));
                    }
                }
            }
            let local = local
                .map(|offset| {
                    start
                        .checked_add(offset)
                        .ok_or_else(|| fail("LocalSubr offset overflow"))
                        .and_then(|at| index(bytes, at, &mut work))
                })
                .transpose()?
                .unwrap_or_default();
            let dictionary = Arc::new(FontDict {
                private: start..start + length,
                local,
                vsindex: vsindex.unwrap_or(0),
                has_vsindex: vsindex.is_some(),
            });
            private_cache.insert((start, length), Arc::clone(&dictionary));
            dictionaries.push(dictionary);
        }
        let selection = fd_select(bytes, offset(0x0c25)?, glyphs.len(), dictionaries.len())?;
        let mut source = Vec::with_capacity(bytes.len());
        for chunk in bytes.chunks(65536) {
            crate::cancel::check_current_cancel("CFF2 source retention")?;
            source.extend_from_slice(chunk);
        }
        let source: Arc<[u8]> = source.into();
        let store = offset(24)?
            .map(|at| variation_store(Arc::clone(&source), at))
            .transpose()?;
        if dictionaries
            .iter()
            .any(|fd| fd.has_vsindex && store.as_ref().is_none_or(|s| fd.vsindex >= s.set_count()))
        {
            return Err(fail("Private vsindex outside store"));
        }
        Ok(Self {
            source,
            glyphs,
            globals,
            font_dicts: fds,
            dictionaries,
            selection,
            store,
            matrix_scale,
        })
    }
    pub fn glyph_count(&self) -> usize {
        self.glyphs.len()
    }
    pub fn outline(
        &self,
        gid: u16,
        coordinates: &[ttf_parser::NormalizedCoordinate],
        pen: &mut dyn ttf_parser::OutlineBuilder,
    ) -> Result<Option<ttf_parser::Rect>> {
        let code = charstring::freeze(self, usize::from(gid), coordinates)?;
        let bytes = charstring::glyph_font(&code)?;
        let table =
            ttf_parser::cff::Table::parse(&bytes).ok_or_else(|| fail("frozen CFF glyph parse"))?;
        let result = table.outline(ttf_parser::GlyphId(1), pen);
        crate::cancel::check_current_cancel("CFF2 outline completion")?;
        match result {
            Ok(bounds) => Ok(Some(bounds)),
            Err(ttf_parser::CFFError::ZeroBBox) => Ok(None),
            Err(error) => Err(fail(&format!("frozen glyph outline: {error:?}"))),
        }
    }
    fn weight(&self) -> usize {
        // Shared Private DICTs are retained once. Counting them once per FD can
        // itself overflow on 32-bit targets when many FDs alias one subr INDEX.
        let mut seen = std::collections::BTreeSet::new();
        let private_weight = self.dictionaries.iter().fold(0usize, |sum, fd| {
            if seen.insert(Arc::as_ptr(fd) as usize) {
                sum.saturating_add(64usize.saturating_add(fd.local.len().saturating_mul(32)))
            } else {
                sum
            }
        });
        self.source.len()
            + self.glyphs.len() * 32
            + self.globals.len() * 32
            + self.font_dicts.len() * 32
            + self.selection.len() * 8
            + self.dictionaries.len() * 16
            + private_weight
            + self.store.as_ref().map_or(0, |s| s.metadata_weight())
    }
}

fn charge_dict_work(work: &mut usize, length: usize) -> Result<()> {
    *work = work
        .checked_add(length)
        .ok_or_else(|| fail("DICT work overflow"))?;
    if *work > 16 * 1024 * 1024 {
        return Err(WellfriendError::ResourceLimit(
            "CFF2 aggregate DICT budget".into(),
        ));
    }
    Ok(())
}

fn fd_select(bytes: &[u8], offset: Option<usize>, glyphs: usize, fds: usize) -> Result<Vec<usize>> {
    let Some(at) = offset else {
        return if fds == 1 {
            Ok(vec![0; glyphs])
        } else {
            Err(fail("multiple FontDICTs require FontDICTSelect"))
        };
    };
    let format = *bytes.get(at).ok_or_else(|| fail("FontDICTSelect format"))?;
    let result = match format {
        0 => data(bytes, at + 1, glyphs)?
            .iter()
            .map(|n| usize::from(*n))
            .collect(),
        3 | 4 => {
            let wide = format == 4;
            let count = if wide {
                u32_at(bytes, at + 1)? as usize
            } else {
                usize::from(u16_at(bytes, at + 1)?)
            };
            if count == 0 || count > glyphs {
                return Err(fail("FontDICTSelect range count"));
            }
            let base = at + if wide { 5 } else { 3 };
            let stride = if wide { 6 } else { 3 };
            let records = data(bytes, base, count * stride + if wide { 4 } else { 2 })?;
            let mut out = vec![0; glyphs];
            let mut previous = 0;
            for i in 0..count {
                let start = if wide {
                    u32_at(records, i * stride)? as usize
                } else {
                    usize::from(u16_at(records, i * stride)?)
                };
                let end = if wide {
                    u32_at(records, (i + 1) * stride)? as usize
                } else {
                    usize::from(u16_at(records, (i + 1) * stride)?)
                };
                let fd = if wide {
                    usize::from(u16_at(records, i * stride + 4)?)
                } else {
                    usize::from(records[i * stride + 2])
                };
                if start != previous || start >= end || end > glyphs {
                    return Err(fail("FontDICTSelect range coverage"));
                }
                out[start..end].fill(fd);
                previous = end;
            }
            if previous != glyphs {
                return Err(fail("FontDICTSelect sentinel"));
            }
            out
        }
        _ => return Err(fail("unsupported FontDICTSelect format")),
    };
    if result.iter().any(|fd| *fd >= fds) {
        return Err(fail("FontDICTSelect index outside FontDICTINDEX"));
    }
    Ok(result)
}

type Cache = VecDeque<([u8; 32], Arc<Program>, usize)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
pub(crate) fn load(bytes: &[u8]) -> Result<Arc<Program>> {
    load_cached(bytes, CACHE.get_or_init(|| Mutex::new(VecDeque::new())))
}
fn load_cached(bytes: &[u8], cache: &Mutex<Cache>) -> Result<Arc<Program>> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(WellfriendError::ResourceLimit("CFF2 table budget".into()));
    }
    crate::cancel::check_current_cancel("CFF2 cache")?;
    let mut hash = Sha256::new();
    for chunk in bytes.chunks(65536) {
        crate::cancel::check_current_cancel("CFF2 identity")?;
        hash.update(chunk);
    }
    let key: [u8; 32] = hash.finalize().into();
    {
        let mut entries = cache.lock().map_err(|_| fail("poisoned CFF2 cache"))?;
        crate::cancel::check_current_cancel("CFF2 cache lookup")?;
        if let Some(i) = entries.iter().position(|entry| entry.0 == key) {
            let entry = entries.remove(i).unwrap();
            let value = Arc::clone(&entry.1);
            entries.push_front(entry);
            return Ok(value);
        }
    }
    let program = Arc::new(Program::parse(bytes)?);
    let weight = program.weight();
    crate::cancel::check_current_cancel("CFF2 cache publication")?;
    if weight <= 64 * 1024 * 1024 {
        let mut entries = cache.lock().map_err(|_| fail("poisoned CFF2 cache"))?;
        crate::cancel::check_current_cancel("CFF2 cache insertion")?;
        if let Some(entry) = entries.iter().find(|entry| entry.0 == key) {
            return Ok(Arc::clone(&entry.1));
        }
        while entries.len() >= 8
            || entries.iter().map(|e| e.2).sum::<usize>() + weight > 64 * 1024 * 1024
        {
            entries.pop_back();
        }
        entries.push_front((key, Arc::clone(&program), weight));
    }
    Ok(program)
}
