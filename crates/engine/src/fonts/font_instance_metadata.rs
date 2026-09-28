//! Static-instance naming, registered-axis style fields and STAT selection.
//! Explicit caller labels replace identity names; rights/notices remain intact.
use super::{
    font_instance::{FontInstanceNaming, FontStyleLink},
    variation_store::{bytes, round_i32, u16_at, u32_at},
};
use crate::{Result, WellfriendError};
use std::collections::{BTreeMap, BTreeSet};
type Key = [u16; 4];
const MAX_NAME_MATERIALIZED_BYTES: usize = 8 * 1024 * 1024;
fn charge_names(total: &mut usize, len: usize) -> Result<()> {
    *total = total
        .checked_add(len)
        .ok_or_else(|| fail("name size overflow"))?;
    if *total > MAX_NAME_MATERIALIZED_BYTES {
        return Err(WellfriendError::ResourceLimit(
            "expanded name strings exceed 8 MiB".into(),
        ));
    }
    Ok(())
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font instance metadata: {message}"))
}
fn word(out: &mut [u8], at: usize, n: u16) {
    out[at..at + 2].copy_from_slice(&n.to_be_bytes());
}
fn dword(out: &mut [u8], at: usize, n: usize) -> Result<()> {
    out[at..at + 4].copy_from_slice(
        &u32::try_from(n)
            .map_err(|_| fail("offset overflow"))?
            .to_be_bytes(),
    );
    Ok(())
}
fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_be_bytes).collect()
}
fn identity(id: u16) -> bool {
    matches!(id, 1 | 2 | 3 | 4 | 6 | 16 | 17 | 18 | 21 | 22 | 25)
}
pub(super) struct Names {
    records: BTreeMap<Key, Vec<u8>>,
    languages: Vec<Vec<u8>>,
    version: u16,
    relocated: BTreeMap<u16, u16>,
}
impl Names {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() > 2 * 1024 * 1024 {
            return Err(WellfriendError::ResourceLimit(
                "name table exceeds 2 MiB".into(),
            ));
        }
        let version = u16_at(data, 0)?;
        let count = usize::from(u16_at(data, 2)?);
        let storage = usize::from(u16_at(data, 4)?);
        if version > 1 || count > 4096 {
            return Err(fail("unsupported name version or excessive records"));
        }
        let end = 6 + count * 12;
        bytes(data, 0, end)?;
        let mut languages = Vec::new();
        let mut materialized = 0;
        let header_end = if version == 1 {
            let n = usize::from(u16_at(data, end)?);
            if n > 256 {
                return Err(fail("language-tag count exceeds 256"));
            }
            for record in bytes(data, end + 2, n * 4)?.chunks_exact(4) {
                let value = bytes(
                    data,
                    storage + usize::from(u16_at(record, 2)?),
                    usize::from(u16_at(record, 0)?),
                )?;
                if value.len() % 2 != 0 {
                    return Err(fail("partial language-tag UTF-16 unit"));
                }
                charge_names(&mut materialized, value.len())?;
                String::from_utf16(
                    &value
                        .chunks_exact(2)
                        .map(|p| u16::from_be_bytes([p[0], p[1]]))
                        .collect::<Vec<_>>(),
                )
                .map_err(|_| fail("invalid language-tag UTF-16"))?;
                languages.push(value.to_vec());
            }
            end + 2 + n * 4
        } else {
            end
        };
        if storage < header_end {
            return Err(fail("name storage overlaps record owners"));
        }
        let mut records = BTreeMap::new();
        for record in data[6..end].chunks_exact(12) {
            crate::cancel::check_current_cancel("font name capture")?;
            let key = [
                u16_at(record, 0)?,
                u16_at(record, 2)?,
                u16_at(record, 4)?,
                u16_at(record, 6)?,
            ];
            if key[2] >= 0x8000
                && key[0] < 240
                && (version == 0 || usize::from(key[2] - 0x8000) >= languages.len())
            {
                return Err(fail("name language tag has no owner"));
            }
            let value = bytes(
                data,
                storage + usize::from(u16_at(record, 10)?),
                usize::from(u16_at(record, 8)?),
            )?;
            charge_names(&mut materialized, value.len())?;
            if records.insert(key, value.to_vec()).is_some() {
                return Err(fail("duplicate name record identity"));
            }
        }
        Ok(Self {
            records,
            languages,
            version,
            relocated: BTreeMap::new(),
        })
    }
    /// STAT can refer to an identity name that is about to change. Give its
    /// original multilingual value a private ID rather than changing its meaning.
    fn reference(&mut self, id: u16) -> Result<u16> {
        if !self.records.keys().any(|key| key[3] == id) {
            return Err(fail("STAT name reference has no string"));
        }
        if !identity(id) {
            return Ok(id);
        }
        if let Some(id) = self.relocated.get(&id) {
            return Ok(*id);
        }
        let used = self
            .records
            .keys()
            .map(|key| key[3])
            .collect::<BTreeSet<_>>();
        let next = (256..32768)
            .find(|id| !used.contains(id))
            .ok_or_else(|| fail("no private name ID remains"))?;
        let mut materialized = self.records.values().map(Vec::len).sum::<usize>()
            + self.languages.iter().map(Vec::len).sum::<usize>();
        let count = self.records.keys().filter(|key| key[3] == id).count();
        if self.records.len() + count > 4096 {
            return Err(fail("relocated name records exceed 4096"));
        }
        for (key, value) in &self.records {
            if key[3] == id {
                charge_names(&mut materialized, value.len())?;
            }
        }
        let copies = self
            .records
            .iter()
            .filter(|(key, _)| key[3] == id)
            .map(|(key, value)| {
                let mut key = *key;
                key[3] = next;
                (key, value.clone())
            })
            .collect::<Vec<_>>();
        for (key, value) in copies {
            self.records.insert(key, value);
        }
        self.relocated.insert(id, next);
        Ok(next)
    }
    fn rename(&mut self, labels: &FontInstanceNaming, unique: &str) -> Result<usize> {
        labels.validate()?;
        let before = self.records.len();
        self.records.retain(|key, _| !identity(key[3]));
        let replaced = before - self.records.len();
        let mut materialized = self.records.values().map(Vec::len).sum::<usize>()
            + self.languages.iter().map(Vec::len).sum::<usize>();
        let full = format!("{} {}", labels.family, labels.subfamily);
        for (id, value) in [
            (1, labels.legacy_family.as_str()),
            (2, labels.style_link.label()),
            (3, unique),
            (4, full.as_str()),
            (6, labels.postscript_name.as_str()),
            (16, labels.family.as_str()),
            (17, labels.subfamily.as_str()),
            (21, labels.legacy_family.as_str()),
            (22, labels.style_link.label()),
        ] {
            for (platform, encoding, language) in [(0, 4, 0), (3, 1, 0x409)] {
                charge_names(&mut materialized, value.encode_utf16().count() * 2)?;
                self.records
                    .insert([platform, encoding, language, id], utf16(value));
            }
        }
        Ok(replaced)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.records.len() > 4096 {
            return Err(fail("output name records exceed 4096"));
        }
        let header = 6
            + self.records.len() * 12
            + if self.version == 1 {
                2 + self.languages.len() * 4
            } else {
                0
            };
        let mut out = vec![0; header];
        word(&mut out, 0, self.version);
        word(&mut out, 2, self.records.len() as u16);
        word(
            &mut out,
            4,
            u16::try_from(header).map_err(|_| fail("name header offset overflow"))?,
        );
        let mut storage = Vec::new();
        let mut copies = BTreeMap::<Vec<u8>, u16>::new();
        let mut intern = |value: &[u8]| -> Result<(u16, u16)> {
            let len =
                u16::try_from(value.len()).map_err(|_| fail("name string length overflow"))?;
            let at = if let Some(at) = copies.get(value) {
                *at
            } else {
                let at = u16::try_from(storage.len())
                    .map_err(|_| fail("name storage offset overflow"))?;
                storage.extend_from_slice(value);
                copies.insert(value.to_vec(), at);
                at
            };
            Ok((len, at))
        };
        for (i, (key, value)) in self.records.iter().enumerate() {
            crate::cancel::check_current_cancel("font name serialization")?;
            let at = 6 + i * 12;
            for (j, value) in key.iter().enumerate() {
                word(&mut out, at + j * 2, *value);
            }
            let (len, offset) = intern(value)?;
            word(&mut out, at + 8, len);
            word(&mut out, at + 10, offset);
        }
        if self.version == 1 {
            let at = 6 + self.records.len() * 12;
            word(&mut out, at, self.languages.len() as u16);
            for (i, language) in self.languages.iter().enumerate() {
                let (len, offset) = intern(language)?;
                word(&mut out, at + 2 + i * 4, len);
                word(&mut out, at + 4 + i * 4, offset);
            }
        }
        out.extend(storage);
        Ok(out)
    }
}
pub(super) struct MetadataStage {
    pub tables: BTreeMap<[u8; 4], Vec<u8>>,
    pub replaced_name_records: usize,
    pub relocated_name_ids: BTreeMap<u16, u16>,
    pub kept_stat_values: usize,
    pub removed_stat_values: usize,
}
fn stat(
    data: &[u8],
    coordinates: &BTreeMap<[u8; 4], f32>,
    names: &mut Names,
) -> Result<(Vec<u8>, usize, usize)> {
    if data.len() > 4 * 1024 * 1024 {
        return Err(fail("STAT exceeds 4 MiB"));
    }
    let version = u32_at(data, 0)?;
    let header = match version {
        0x10000 => 18,
        0x10001 | 0x10002 => 20,
        _ => return Err(fail("unsupported STAT version")),
    };
    bytes(data, 0, header)?;
    let stride = usize::from(u16_at(data, 4)?);
    let axes = usize::from(u16_at(data, 6)?);
    let axis_at = u32_at(data, 8)? as usize;
    let count = usize::from(u16_at(data, 12)?);
    let values_at = u32_at(data, 14)? as usize;
    if axes > 256
        || count > 4096
        || (axes > 0 && (stride < 8 || axis_at < header))
        || (count > 0 && (axes == 0 || values_at < header))
    {
        return Err(fail("invalid STAT owners/counts"));
    }
    let mut axis_records = bytes(data, axis_at, axes * stride)?.to_vec();
    let mut tags = Vec::with_capacity(axes);
    let mut seen = BTreeSet::new();
    for record in axis_records.chunks_exact_mut(stride.max(1)) {
        crate::cancel::check_current_cancel("STAT axis binding")?;
        let tag: [u8; 4] = record[..4].try_into().unwrap();
        if !seen.insert(tag) {
            return Err(fail("duplicate STAT axis"));
        }
        tags.push(tag);
        let name = names.reference(u16_at(record, 4)?)?;
        word(record, 4, name);
    }
    if coordinates.keys().any(|tag| !seen.contains(tag)) {
        return Err(fail("STAT omits a selected fvar axis"));
    }
    let fixed = |at| -> Result<f64> {
        Ok(f64::from(i32::from_be_bytes(bytes(data, at, 4)?.try_into().unwrap())) / 65536.)
    };
    let selected = |axis: u16| -> Result<Option<f64>> {
        let tag = tags
            .get(usize::from(axis))
            .ok_or_else(|| fail("STAT axis index outside owner"))?;
        Ok(coordinates.get(tag).copied().map(f64::from))
    };
    let mut kept = Vec::new();
    for record in bytes(data, values_at, count * 2)?.chunks_exact(2) {
        crate::cancel::check_current_cancel("STAT value selection")?;
        let offset = usize::from(u16_at(record, 0)?);
        if offset < count * 2 {
            return Err(fail("STAT value overlaps its offset array"));
        }
        let at = values_at
            .checked_add(offset)
            .ok_or_else(|| fail("STAT value overflow"))?;
        let format = u16_at(data, at)?;
        let (len, matches) = match format {
            1 | 3 => {
                let coordinate = selected(u16_at(data, at + 2)?)?;
                let value = fixed(at + 8)?;
                (
                    if format == 1 { 12 } else { 16 },
                    coordinate.is_none_or(|n| n == value),
                )
            }
            2 => {
                let coordinate = selected(u16_at(data, at + 2)?)?;
                let nominal = fixed(at + 8)?;
                let min = fixed(at + 12)?;
                let max = fixed(at + 16)?;
                if min > nominal || nominal > max {
                    return Err(fail("inverted STAT value range"));
                }
                (20, coordinate.is_none_or(|n| min <= n && n <= max))
            }
            4 if version == 0x10002 => {
                let n = usize::from(u16_at(data, at + 2)?);
                if n == 0 || n > axes {
                    return Err(fail("invalid STAT combination size"));
                }
                let mut indices = BTreeSet::new();
                let mut matches = true;
                for i in 0..n {
                    let axis = u16_at(data, at + 8 + i * 6)?;
                    if !indices.insert(axis) {
                        return Err(fail("repeated STAT combination axis"));
                    }
                    let value = fixed(at + 10 + i * 6)?;
                    matches &= selected(axis)?.is_none_or(|n| n == value);
                }
                (8 + n * 6, matches)
            }
            _ => {
                return Err(WellfriendError::UnsupportedFeature(
                    "STAT value format needs an instance-aware owner".into(),
                ))
            }
        };
        let mut value = bytes(data, at, len)?.to_vec();
        let flags = u16_at(&value, 4)?;
        if flags & !3 != 0 {
            return Err(fail("unknown STAT flags"));
        }
        // Older-sibling values describe other family members, not this instance.
        if matches || flags & 1 != 0 {
            let name = names.reference(u16_at(&value, 6)?)?;
            word(&mut value, 6, name);
            kept.push(value);
        }
    }
    let mut out = data[..header].to_vec();
    if header == 20 {
        let id = names.reference(u16_at(data, 18)?)?;
        word(&mut out, 18, id);
    }
    dword(&mut out, 8, if axes == 0 { 0 } else { header })?;
    out.extend(axis_records);
    let base = out.len();
    word(&mut out, 12, kept.len() as u16);
    dword(&mut out, 14, if kept.is_empty() { 0 } else { base })?;
    out.resize(base + kept.len() * 2, 0);
    for (i, value) in kept.iter().enumerate() {
        let offset = u16::try_from(out.len() - base)
            .map_err(|_| fail("STAT value offset exceeds 16 bits"))?;
        word(&mut out, base + i * 2, offset);
        out.extend(value);
    }
    Ok((out, kept.len(), count - kept.len()))
}
pub(super) fn freeze(
    tables: &BTreeMap<[u8; 4], Vec<u8>>,
    coordinates: &BTreeMap<[u8; 4], f32>,
    naming: &FontInstanceNaming,
    unique: &str,
    advances: &[u16],
) -> Result<MetadataStage> {
    crate::cancel::check_current_cancel("font metadata preparation")?;
    if coordinates.len() > 64 || coordinates.values().any(|n| !n.is_finite()) {
        return Err(fail("invalid metadata coordinate domain"));
    }
    let mut names = if let Some(data) = tables.get(b"name") {
        Names::parse(data)?
    } else {
        Names {
            records: BTreeMap::new(),
            languages: Vec::new(),
            version: 0,
            relocated: BTreeMap::new(),
        }
    };
    let mut result = MetadataStage {
        tables: BTreeMap::new(),
        replaced_name_records: 0,
        relocated_name_ids: BTreeMap::new(),
        kept_stat_values: 0,
        removed_stat_values: 0,
    };
    if let Some(data) = tables.get(b"STAT") {
        let (stat, kept, removed) = stat(data, coordinates, &mut names)?;
        result.tables.insert(*b"STAT", stat);
        result.kept_stat_values = kept;
        result.removed_stat_values = removed;
    }
    result.replaced_name_records = names.rename(naming, unique)?;
    result.relocated_name_ids = names.relocated.clone();
    result.tables.insert(*b"name", names.encode()?);
    let bold = matches!(
        naming.style_link,
        FontStyleLink::Bold | FontStyleLink::BoldItalic
    );
    let italic = matches!(
        naming.style_link,
        FontStyleLink::Italic | FontStyleLink::BoldItalic
    );
    if let Some(data) = tables.get(b"head") {
        bytes(data, 0, 54)?;
        let mut head = data.clone();
        let mut flags = (u16_at(data, 44)? & !3) | u16::from(bold) | (u16::from(italic) << 1);
        if let Some(width) = coordinates.get(b"wdth") {
            flags = (flags & !0x60)
                | if *width < 100. {
                    0x20
                } else if *width > 100. {
                    0x40
                } else {
                    0
                };
        }
        word(&mut head, 44, flags);
        result.tables.insert(*b"head", head);
    }
    if let Some(data) = tables.get(b"OS/2") {
        bytes(data, 0, 64)?;
        let mut os2 = data.clone();
        let version = u16_at(data, 0)?;
        if version > 5 {
            return Err(fail("unsupported OS/2 version"));
        }
        if let Some(weight) = coordinates.get(b"wght") {
            if !(1.0..=1000.0).contains(weight) {
                return Err(fail("wght outside registered range"));
            }
            word(&mut os2, 4, round_i32(f64::from(*weight))? as u16);
        }
        if let Some(width) = coordinates.get(b"wdth") {
            if *width <= 0. {
                return Err(fail("nonpositive registered width"));
            }
            let classes = [50., 62.5, 75., 87.5, 100., 112.5, 125., 150., 200.];
            let at = classes.partition_point(|n| n < width).min(8);
            let value = if at == 0 {
                1.
            } else {
                at as f64
                    + (f64::from(*width) - classes[at - 1] as f64)
                        / (classes[at] - classes[at - 1]) as f64
            };
            word(&mut os2, 6, round_i32(value.clamp(1., 9.))? as u16);
        }
        let mut flags = u16_at(data, 62)? & !(1 | 32 | 64 | 256);
        flags |= u16::from(italic) | (u16::from(bold) << 5) | if !bold && !italic { 64 } else { 0 };
        if version >= 4 {
            let oblique = coordinates.get(b"slnt").is_some_and(|value| *value != 0.)
                && coordinates.get(b"ital").is_none_or(|value| *value == 0.);
            if coordinates.contains_key(b"slnt") || coordinates.contains_key(b"ital") {
                flags = (flags & !512) | if oblique { 512 } else { 0 };
            }
        }
        word(&mut os2, 62, flags);
        let widths = advances
            .iter()
            .filter(|n| **n != 0)
            .map(|n| u64::from(*n))
            .collect::<Vec<_>>();
        let average = if widths.is_empty() {
            0
        } else {
            round_i32(widths.iter().sum::<u64>() as f64 / widths.len() as f64)?
        };
        let average =
            i16::try_from(average).map_err(|_| fail("average glyph advance exceeds OS/2"))?;
        word(&mut os2, 2, average as u16);
        result.tables.insert(*b"OS/2", os2);
    }
    if let (Some(data), Some(slant)) = (tables.get(b"post"), coordinates.get(b"slnt")) {
        bytes(data, 0, 32)?;
        let mut post = data.clone();
        post[4..8].copy_from_slice(&round_i32(f64::from(*slant) * 65536.)?.to_be_bytes());
        result.tables.insert(*b"post", post);
    }
    crate::cancel::check_current_cancel("font metadata publication")?;
    Ok(result)
}

#[cfg(test)]
#[path = "font_instance_metadata_tests.rs"]
mod tests;
