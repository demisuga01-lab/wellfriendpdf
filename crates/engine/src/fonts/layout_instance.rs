//! Freeze GSUB/GPOS FeatureVariations while retaining stable feature/lookup
//! indices. This does not freeze GPOS/GDEF VariationIndex values or outlines.
use super::variation_store::{bytes, u16_at, u32_at};
use crate::{Result, WellfriendError};
use std::collections::BTreeMap;
const LIMIT: usize = 64 * 1024 * 1024;
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font layout instance: {message}"))
}
fn unsupported(message: &str) -> WellfriendError {
    WellfriendError::UnsupportedFeature(format!("font layout instance: {message}"))
}
fn relative(data: &[u8], base: usize, value: usize) -> Result<usize> {
    if value == 0 {
        return Err(fail("unexpected null structural offset"));
    }
    let at = base
        .checked_add(value)
        .ok_or_else(|| fail("offset overflow"))?;
    bytes(data, at, 2)?;
    Ok(at)
}
pub(crate) struct FrozenLayout {
    pub bytes: Vec<u8>,
    pub selected_record: Option<u32>,
    pub substituted_features: Vec<u16>,
    /// Original lookup programs are retained once, with internal offsets intact.
    pub retained_source_block: bool,
}
fn conditions(
    data: &[u8],
    at: usize,
    coords: &[ttf_parser::NormalizedCoordinate],
    work: &mut usize,
) -> Result<bool> {
    let count = usize::from(u16_at(data, at)?);
    *work = work
        .checked_add(count)
        .ok_or_else(|| fail("condition work overflow"))?;
    if *work > 1_000_000 {
        return Err(WellfriendError::ResourceLimit(
            "font feature condition budget".into(),
        ));
    }
    bytes(data, at + 2, count * 4)?;
    for i in 0..count {
        if i % 256 == 0 {
            crate::cancel::check_current_cancel("font instance feature conditions")?;
        }
        let condition = relative(data, at, u32_at(data, at + 2 + i * 4)? as usize)?;
        // Unknown condition formats and invalid axes do not match (OpenType).
        if u16_at(data, condition)? != 1 {
            return Ok(false);
        }
        let axis = usize::from(u16_at(data, condition + 2)?);
        let Some(value) = coords.get(axis).map(|c| c.get()) else {
            return Ok(false);
        };
        let min = i16::from_be_bytes(bytes(data, condition + 4, 2)?.try_into().unwrap());
        let max = i16::from_be_bytes(bytes(data, condition + 6, 2)?.try_into().unwrap());
        if min < -16384 || max > 16384 || min > max {
            return Err(fail("invalid normalized feature range"));
        }
        if value < min || value > max {
            return Ok(false);
        }
    }
    Ok(true)
}
fn selection(
    data: &[u8],
    variation: usize,
    coords: &[ttf_parser::NormalizedCoordinate],
    features: usize,
) -> Result<(Option<u32>, BTreeMap<u16, usize>)> {
    if variation == 0 {
        return Ok((None, BTreeMap::new()));
    }
    if u32_at(data, variation)? != 0x00010000 {
        return Err(unsupported("FeatureVariations version"));
    }
    let count = u32_at(data, variation + 4)? as usize;
    if count > 65536 {
        return Err(WellfriendError::ResourceLimit(
            "feature variation record budget".into(),
        ));
    }
    bytes(data, variation + 8, count * 8)?;
    let mut work = 0usize;
    for i in 0..count {
        crate::cancel::check_current_cancel("font instance feature selection")?;
        let record = variation + 8 + i * 8;
        let condition = u32_at(data, record)? as usize;
        if condition != 0
            && !conditions(
                data,
                relative(data, variation, condition)?,
                coords,
                &mut work,
            )?
        {
            continue;
        }
        let substitute = u32_at(data, record + 4)? as usize;
        if substitute == 0 {
            return Ok((Some(i as u32), BTreeMap::new()));
        }
        let substitute = relative(data, variation, substitute)?;
        // An unsupported substitution version rejects this candidate, not the
        // complete search. A later supported condition may still match.
        if u32_at(data, substitute)? != 0x00010000 {
            continue;
        }
        let count = usize::from(u16_at(data, substitute + 4)?);
        bytes(data, substitute + 6, count * 6)?;
        let mut result = BTreeMap::new();
        let mut previous = None;
        for n in 0..count {
            if n % 256 == 0 {
                crate::cancel::check_current_cancel("font instance feature substitutions")?;
            }
            let record = substitute + 6 + n * 6;
            let index = u16_at(data, record)?;
            if usize::from(index) >= features || previous.is_some_and(|p| p >= index) {
                return Err(fail("unsorted, duplicate or unknown feature substitution"));
            }
            result.insert(
                index,
                relative(data, substitute, u32_at(data, record + 2)? as usize)?,
            );
            previous = Some(index);
        }
        return Ok((Some(i as u32), result));
    }
    Ok((None, BTreeMap::new()))
}
struct Writer {
    out: Vec<u8>,
}
impl Writer {
    fn reserve(&mut self, len: usize) -> Result<usize> {
        let at = self.out.len();
        let end = at
            .checked_add(len)
            .ok_or_else(|| fail("layout output overflow"))?;
        if end > LIMIT {
            return Err(WellfriendError::ResourceLimit(
                "frozen layout exceeds 64 MiB".into(),
            ));
        }
        crate::cancel::check_current_cancel("font layout allocation")?;
        self.out.resize(end, 0);
        Ok(at)
    }
    fn copy(&mut self, data: &[u8]) -> Result<usize> {
        let at = self.reserve(data.len())?;
        for (i, chunk) in data.chunks(65536).enumerate() {
            crate::cancel::check_current_cancel("font layout source copy")?;
            self.out[at + i * 65536..at + i * 65536 + chunk.len()].copy_from_slice(chunk);
        }
        Ok(at)
    }
    fn word(&mut self, at: usize, value: u16) {
        self.out[at..at + 2].copy_from_slice(&value.to_be_bytes());
    }
    fn link(&mut self, field: usize, base: usize, target: usize) -> Result<()> {
        let delta = target
            .checked_sub(base)
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| *n != 0)
            .ok_or_else(|| unsupported("rebuilt 16-bit layout offset cannot represent target"))?;
        self.word(field, delta);
        Ok(())
    }
}
fn langsys(w: &mut Writer, data: &[u8], at: usize, features: usize) -> Result<usize> {
    let count = usize::from(u16_at(data, at + 4)?);
    let source = bytes(data, at, 6 + count * 2)?;
    if u16_at(source, 0)? != 0 {
        return Err(unsupported("non-null LangSys lookup order"));
    }
    let required = u16_at(source, 2)?;
    if required != 0xffff && usize::from(required) >= features {
        return Err(fail("required feature outside feature list"));
    }
    for chunk in source[6..].chunks_exact(2) {
        if usize::from(u16::from_be_bytes(chunk.try_into().unwrap())) >= features {
            return Err(fail("language feature outside feature list"));
        }
    }
    w.copy(source)
}
fn scripts(w: &mut Writer, data: &[u8], source: usize, root: usize, features: usize) -> Result<()> {
    let count = usize::from(u16_at(data, source)?);
    let mut previous = None;
    for i in 0..count {
        let record = source + 2 + i * 6;
        let tag = u32_at(data, record)?;
        if previous.is_some_and(|p| p >= tag) {
            return Err(fail("script tags not strictly sorted"));
        }
        previous = Some(tag);
        let script = relative(data, source, usize::from(u16_at(data, record + 4)?))?;
        let langs = usize::from(u16_at(data, script + 2)?);
        let target = w.copy(bytes(data, script, 4 + langs * 6)?)?;
        w.link(root + 2 + i * 6 + 4, root, target)?;
        let default = usize::from(u16_at(data, script)?);
        if default != 0 {
            let lang = langsys(w, data, relative(data, script, default)?, features)?;
            w.link(target, target, lang)?;
        }
        let mut previous = None;
        for n in 0..langs {
            let record = script + 4 + n * 6;
            let tag = u32_at(data, record)?;
            if previous.is_some_and(|p| p >= tag) {
                return Err(fail("language tags not strictly sorted"));
            }
            previous = Some(tag);
            let lang = langsys(
                w,
                data,
                relative(data, script, usize::from(u16_at(data, record + 4)?))?,
                features,
            )?;
            w.link(target + 4 + n * 6 + 4, target, lang)?;
        }
    }
    Ok(())
}
fn parameters<'a>(data: &'a [u8], feature: usize, tag: &[u8]) -> Result<Option<&'a [u8]>> {
    let offset = usize::from(u16_at(data, feature)?);
    if offset == 0 {
        return Ok(None);
    }
    let at = relative(data, feature, offset)?;
    let suffix = if tag.len() == 4 && tag[2..].iter().all(u8::is_ascii_digit) {
        Some((tag[2] - b'0') * 10 + tag[3] - b'0')
    } else {
        None
    };
    let length = if tag == b"size" {
        10
    } else if tag.len() == 4 && &tag[..2] == b"ss" && suffix.is_some_and(|n| (1..=20).contains(&n))
    {
        if u16_at(data, at)? != 0 {
            return Err(unsupported("stylistic set parameter version"));
        }
        4
    } else if tag.len() == 4 && &tag[..2] == b"cv" && suffix.is_some_and(|n| (1..=99).contains(&n))
    {
        if u16_at(data, at)? != 0 {
            return Err(unsupported("character variant parameter version"));
        }
        14 + usize::from(u16_at(data, at + 12)?) * 3
    } else {
        return Err(unsupported(
            "unknown non-null feature parameters need an opaque relocation contract",
        ));
    };
    Ok(Some(bytes(data, at, length)?))
}
/// Freeze feature selection only. The caller must separately freeze variable
/// positioning values, font metrics and outlines before removing font axes.
pub(crate) fn freeze(
    data: &[u8],
    tag: [u8; 4],
    coordinates: &[ttf_parser::NormalizedCoordinate],
) -> Result<FrozenLayout> {
    freeze_impl(data, tag, coordinates, false, &mut |_, _| Ok(None))
}

/// Reuse the same feature/script/lookup owners when positioning programs need
/// rebuilding. Returned subtable bytes have offsets relative to their own root;
/// None retains the exact source program (used for contextual lookup dispatch).
pub(crate) fn freeze_gpos_with(
    data: &[u8],
    coordinates: &[ttf_parser::NormalizedCoordinate],
    rewrite: &mut dyn FnMut(u16, usize) -> Result<Option<Vec<u8>>>,
) -> Result<FrozenLayout> {
    freeze_impl(data, *b"GPOS", coordinates, true, rewrite)
}

fn freeze_impl(
    data: &[u8],
    tag: [u8; 4],
    coordinates: &[ttf_parser::NormalizedCoordinate],
    rebuild: bool,
    rewrite: &mut dyn FnMut(u16, usize) -> Result<Option<Vec<u8>>>,
) -> Result<FrozenLayout> {
    crate::cancel::check_current_cancel("font layout instancing")?;
    if data.len() > LIMIT || coordinates.len() > 64 {
        return Err(WellfriendError::ResourceLimit(
            "font layout input budget".into(),
        ));
    }
    let (extension, max_type) = match &tag {
        b"GSUB" => (7u16, 8u16),
        b"GPOS" => (9, 9),
        _ => return Err(fail("expected GSUB or GPOS")),
    };
    let version = u32_at(data, 0)?;
    if !matches!(version, 0x00010000 | 0x00010001) {
        return Err(unsupported("layout version"));
    }
    let script_at = relative(data, 0, usize::from(u16_at(data, 4)?))?;
    let feature_at = relative(data, 0, usize::from(u16_at(data, 6)?))?;
    let lookup_at = relative(data, 0, usize::from(u16_at(data, 8)?))?;
    let header_len = if version == 0x00010001 { 14 } else { 10 };
    if [script_at, feature_at, lookup_at]
        .iter()
        .any(|at| *at < header_len)
    {
        return Err(fail("layout root overlaps its header"));
    }
    let feature_count = usize::from(u16_at(data, feature_at)?);
    let lookup_count = usize::from(u16_at(data, lookup_at)?);
    let variation = if version == 0x00010001 {
        u32_at(data, 10)? as usize
    } else {
        0
    };
    let (selected_record, replacements) = selection(data, variation, coordinates, feature_count)?;
    if replacements.is_empty() && !rebuild {
        let mut w = Writer { out: Vec::new() };
        w.copy(data)?;
        w.out[..4].copy_from_slice(&0x00010000u32.to_be_bytes());
        if version == 0x00010001 {
            w.out[10..14].fill(0);
        }
        crate::cancel::check_current_cancel("default feature publication")?;
        return Ok(FrozenLayout {
            bytes: w.out,
            selected_record,
            substituted_features: vec![],
            retained_source_block: false,
        });
    }
    let mut w = Writer { out: vec![0; 10] };
    w.out[..4].copy_from_slice(&0x00010000u32.to_be_bytes());
    // Allocate all roots first; large lookup bodies remain in one opaque block.
    let script_count = usize::from(u16_at(data, script_at)?);
    let script_root = w.copy(bytes(data, script_at, 2 + script_count * 6)?)?;
    let feature_root = w.copy(bytes(data, feature_at, 2 + feature_count * 6)?)?;
    let lookup_root = w.copy(bytes(data, lookup_at, 2 + lookup_count * 2)?)?;
    w.link(4, 0, script_root)?;
    w.link(6, 0, feature_root)?;
    w.link(8, 0, lookup_root)?;
    scripts(&mut w, data, script_at, script_root, feature_count)?;
    let mut previous = None;
    for i in 0..feature_count {
        crate::cancel::check_current_cancel("font instance feature serialization")?;
        let record = feature_at + 2 + i * 6;
        let feature_tag = u32_at(data, record)?;
        if previous.is_some_and(|p| p > feature_tag) {
            return Err(fail("feature tags not sorted"));
        }
        previous = Some(feature_tag);
        let feature = if let Some(at) = replacements.get(&(i as u16)) {
            *at
        } else {
            relative(data, feature_at, usize::from(u16_at(data, record + 4)?))?
        };
        let count = usize::from(u16_at(data, feature + 2)?);
        let source = bytes(data, feature, 4 + count * 2)?;
        for chunk in source[4..].chunks_exact(2) {
            if usize::from(u16::from_be_bytes(chunk.try_into().unwrap())) >= lookup_count {
                return Err(fail("feature lookup outside lookup list"));
            }
        }
        let target = w.copy(source)?;
        w.link(feature_root + 2 + i * 6 + 4, feature_root, target)?;
        if let Some(params) = parameters(data, feature, &feature_tag.to_be_bytes())? {
            let params = w.copy(params)?;
            w.link(target, target, params)?;
        }
    }
    let mut pending = Vec::new();
    let mut subtables = 0usize;
    for i in 0..lookup_count {
        crate::cancel::check_current_cancel("font instance lookup ownership")?;
        let original = relative(
            data,
            lookup_at,
            usize::from(u16_at(data, lookup_at + 2 + i * 2)?),
        )?;
        let kind = u16_at(data, original)?;
        if kind == 0 || kind > max_type {
            return Err(unsupported("lookup type"));
        }
        let flags = u16_at(data, original + 2)?;
        let count = usize::from(u16_at(data, original + 4)?);
        subtables += count;
        if subtables > 65536 {
            return Err(WellfriendError::ResourceLimit(
                "layout subtable budget".into(),
            ));
        }
        let target = w.copy(bytes(
            data,
            original,
            6 + count * 2 + if flags & 16 != 0 { 2 } else { 0 },
        )?)?;
        w.word(target, extension);
        w.link(lookup_root + 2 + i * 2, lookup_root, target)?;
        pending.push((original, target, kind, count));
    }
    let mut links = Vec::new();
    for (original, target, kind, count) in pending {
        let mut extension_kind = None;
        for i in 0..count {
            let subtable = relative(
                data,
                original,
                usize::from(u16_at(data, original + 6 + i * 2)?),
            )?;
            let (kind, source) = if kind == extension {
                if u16_at(data, subtable)? != 1 {
                    return Err(unsupported("extension format"));
                }
                let inner = u16_at(data, subtable + 2)?;
                if inner == 0 || inner == extension || inner > max_type {
                    return Err(fail("invalid nested extension type"));
                }
                if extension_kind.is_some_and(|previous| previous != inner) {
                    return Err(fail("mixed lookup types within one extension lookup"));
                }
                extension_kind = Some(inner);
                (
                    inner,
                    relative(data, subtable, u32_at(data, subtable + 4)? as usize)?,
                )
            } else {
                (kind, subtable)
            };
            let wrapper = w.reserve(8)?;
            w.word(wrapper, 1);
            w.word(wrapper + 2, kind);
            w.link(target + 6 + i * 2, target, wrapper)?;
            links.push((wrapper, source, kind));
        }
    }
    let mut source_base = None;
    let mut targets = BTreeMap::new();
    for (wrapper, source, kind) in links {
        let target = if let Some(target) = targets.get(&(kind, source)) {
            *target
        } else {
            let target = if let Some(program) = rewrite(kind, source)? {
                w.copy(&program)?
            } else {
                let base = if let Some(base) = source_base {
                    base
                } else {
                    let base = w.copy(data)?;
                    source_base = Some(base);
                    base
                };
                base + source
            };
            targets.insert((kind, source), target);
            target
        };
        let offset =
            u32::try_from(target - wrapper).map_err(|_| fail("extension offset overflow"))?;
        w.out[wrapper + 4..wrapper + 8].copy_from_slice(&offset.to_be_bytes());
    }
    crate::cancel::check_current_cancel("frozen feature publication")?;
    Ok(FrozenLayout {
        bytes: w.out,
        selected_record,
        substituted_features: replacements.keys().copied().collect(),
        retained_source_block: source_base.is_some(),
    })
}

#[cfg(test)]
#[path = "layout_instance_tests.rs"]
mod tests;
