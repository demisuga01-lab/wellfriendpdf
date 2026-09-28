//! Shared, bounded OpenType ItemVariationStore evaluation. Indices, byte ranges
//! and coordinates stay attached to the immutable source that was parsed.
use crate::{Result, WellfriendError};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font variations: {message}"))
}
pub(super) fn bytes(data: &[u8], at: usize, len: usize) -> Result<&[u8]> {
    data.get(at..at.checked_add(len).ok_or_else(|| fail("offset overflow"))?)
        .ok_or_else(|| fail("truncated data"))
}
pub(super) fn u16_at(data: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(bytes(data, at, 2)?.try_into().unwrap()))
}
pub(super) fn u32_at(data: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(bytes(data, at, 4)?.try_into().unwrap()))
}
fn coords(values: &[ttf_parser::NormalizedCoordinate], axes: usize) -> Result<()> {
    if values.len() != axes || axes > 64 {
        return Err(fail(
            "normalized coordinate count disagrees with variation store",
        ));
    }
    Ok(())
}
struct RegionList {
    axes: usize,
    count: usize,
    values: Vec<[i16; 3]>,
}
impl RegionList {
    fn parse(data: &[u8], at: usize) -> Result<Self> {
        let axes = usize::from(u16_at(data, at)?);
        let count = usize::from(u16_at(data, at + 2)?);
        if axes > 64 {
            return Err(fail("variation axis budget exceeds 64"));
        }
        let records = bytes(data, at + 4, count * axes * 6)?;
        let mut values = Vec::with_capacity(count * axes);
        for (i, record) in records.chunks_exact(6).enumerate() {
            if i % 1024 == 0 {
                crate::cancel::check_current_cancel("font variation regions")?;
            }
            let triple = [
                i16::from_be_bytes(record[0..2].try_into().unwrap()),
                i16::from_be_bytes(record[2..4].try_into().unwrap()),
                i16::from_be_bytes(record[4..6].try_into().unwrap()),
            ];
            if triple.iter().any(|v| !(-16384..=16384).contains(v)) {
                return Err(fail("region lies outside normalized coordinate domain"));
            }
            values.push(triple);
        }
        Ok(Self {
            axes,
            count,
            values,
        })
    }
    fn scalar(&self, region: usize, coordinates: &[ttf_parser::NormalizedCoordinate]) -> f64 {
        self.values[region * self.axes..(region + 1) * self.axes]
            .iter()
            .zip(coordinates)
            .fold(1., |scalar, (triple, coordinate)| {
                scalar * axis_scalar(*triple, coordinate.get())
            })
    }
}
pub(super) fn axis_scalar([start, peak, end]: [i16; 3], value: i16) -> f64 {
    // OpenType explicitly makes malformed/cross-origin/zero-peak axis regions
    // non-participating, not a source of division-by-zero or negative scalars.
    if start > peak || peak > end || peak == 0 || (start < 0 && end > 0) {
        return 1.;
    }
    if value < start || value > end {
        return 0.;
    }
    if value == peak {
        return 1.;
    }
    if value < peak {
        (f64::from(value) - f64::from(start)) / (f64::from(peak) - f64::from(start))
    } else {
        (f64::from(end) - f64::from(value)) / (f64::from(end) - f64::from(peak))
    }
}
struct DeltaSet {
    rows: Range<usize>,
    items: usize,
    words: usize,
    long: bool,
    regions: Vec<usize>,
}
pub(crate) struct ItemVariationStore {
    source: Arc<[u8]>,
    extent: Range<usize>,
    regions: RegionList,
    sets: Vec<Option<Arc<DeltaSet>>>,
}
impl ItemVariationStore {
    pub fn parse(source: Arc<[u8]>, extent: Range<usize>) -> Result<Self> {
        crate::cancel::check_current_cancel("font variation store")?;
        if extent.end < extent.start || extent.len() > 64 * 1024 * 1024 {
            return Err(WellfriendError::ResourceLimit(
                "font variation store exceeds 64 MiB".into(),
            ));
        }
        let data = bytes(&source, extent.start, extent.len())?;
        if u16_at(data, 0)? != 1 {
            return Err(fail("unsupported item variation store format"));
        }
        let region_at = u32_at(data, 2)? as usize;
        let count = usize::from(u16_at(data, 6)?);
        bytes(data, 8, count * 4)?;
        if region_at == 0 {
            return Err(fail("missing variation region list"));
        }
        let regions = RegionList::parse(data, region_at)?;
        let mut sets = Vec::with_capacity(count);
        let mut unique = BTreeMap::<usize, Arc<DeltaSet>>::new();
        let mut references = 0usize;
        let mut cells = 0usize;
        let mut rows = 0usize;
        for i in 0..count {
            crate::cancel::check_current_cancel("font variation delta directories")?;
            let at = u32_at(data, 8 + i * 4)? as usize;
            if at == 0 {
                sets.push(None);
                continue;
            }
            if let Some(set) = unique.get(&at) {
                sets.push(Some(Arc::clone(set)));
                continue;
            }
            let items = usize::from(u16_at(data, at)?);
            let packed = u16_at(data, at + 2)?;
            let words = usize::from(packed & 0x7fff);
            let long = packed & 0x8000 != 0;
            let region_count = usize::from(u16_at(data, at + 4)?);
            if words > region_count {
                return Err(fail("word delta count exceeds region count"));
            }
            references += region_count;
            cells = cells
                .checked_add(
                    items
                        .checked_mul(region_count)
                        .ok_or_else(|| fail("delta count overflow"))?,
                )
                .ok_or_else(|| fail("delta count overflow"))?;
            rows += items;
            if references > 1_000_000 || cells > 4_000_000 || rows > 1_000_000 {
                return Err(WellfriendError::ResourceLimit(
                    "font variation delta work budget".into(),
                ));
            }
            let mut selected = Vec::with_capacity(region_count);
            for chunk in bytes(data, at + 6, region_count * 2)?.chunks_exact(2) {
                let region = usize::from(u16::from_be_bytes(chunk.try_into().unwrap()));
                if region >= regions.count {
                    return Err(fail("unknown variation region"));
                }
                selected.push(region);
            }
            let row_start = at + 6 + region_count * 2;
            let row_length = (region_count + words) * if long { 2 } else { 1 };
            let row_bytes = items
                .checked_mul(row_length)
                .ok_or_else(|| fail("delta row size overflow"))?;
            bytes(data, row_start, row_bytes)?;
            let set = Arc::new(DeltaSet {
                rows: row_start..row_start + row_bytes,
                items,
                words,
                long,
                regions: selected,
            });
            unique.insert(at, Arc::clone(&set));
            sets.push(Some(set));
        }
        Ok(Self {
            source,
            extent,
            regions,
            sets,
        })
    }
    pub fn axis_count(&self) -> usize {
        self.regions.axes
    }
    pub fn set_count(&self) -> usize {
        self.sets.len()
    }
    pub fn require_short_deltas(&self) -> Result<()> {
        for (i, set) in self.sets.iter().enumerate() {
            if i % 256 == 0 {
                crate::cancel::check_current_cancel("font delta width contract")?;
            }
            if set.as_ref().is_some_and(|s| s.long) {
                return Err(fail(
                    "32-bit variation rows are not permitted in this parent table",
                ));
            }
        }
        Ok(())
    }
    pub fn require_cff2_regions_only(&self) -> Result<()> {
        for (i, set) in self.sets.iter().enumerate() {
            if i % 256 == 0 {
                crate::cancel::check_current_cancel("CFF2 variation data contract")?;
            }
            if set
                .as_ref()
                .is_some_and(|s| s.items != 0 || s.words != 0 || s.long)
            {
                return Err(fail("CFF2 variation store must not contain delta rows"));
            }
        }
        Ok(())
    }
    pub fn scalars(
        &self,
        set: usize,
        coordinates: &[ttf_parser::NormalizedCoordinate],
    ) -> Result<Vec<f64>> {
        crate::cancel::check_current_cancel("font variation scalar selection")?;
        coords(coordinates, self.axis_count())?;
        let set = self
            .sets
            .get(set)
            .ok_or_else(|| fail("variation index outside store"))?;
        let Some(set) = set else {
            return Ok(Vec::new());
        };
        let mut result = Vec::with_capacity(set.regions.len());
        for (i, region) in set.regions.iter().enumerate() {
            if i % 256 == 0 {
                crate::cancel::check_current_cancel("font variation interpolation")?;
            }
            result.push(self.regions.scalar(*region, coordinates));
        }
        Ok(result)
    }
    /// Resolve each region only once for repeated metrics/layout queries.
    pub fn prepare(
        &self,
        coordinates: &[ttf_parser::NormalizedCoordinate],
    ) -> Result<PreparedStore<'_>> {
        crate::cancel::check_current_cancel("font instance region preparation")?;
        coords(coordinates, self.axis_count())?;
        let mut scalars = Vec::with_capacity(self.regions.count);
        for region in 0..self.regions.count {
            if region % 256 == 0 {
                crate::cancel::check_current_cancel("font instance region preparation")?;
            }
            scalars.push(self.regions.scalar(region, coordinates));
        }
        Ok(PreparedStore {
            store: self,
            scalars,
        })
    }
    /// Excludes the shared source allocation, which the font owner charges once.
    pub fn metadata_weight(&self) -> usize {
        let mut seen = std::collections::BTreeSet::new();
        96 + self.regions.values.len() * 6
            + self.sets.len() * 16
            + self
                .sets
                .iter()
                .flatten()
                .filter(|s| seen.insert(Arc::as_ptr(s) as usize))
                .map(|s| 80 + s.regions.len() * 8)
                .sum::<usize>()
    }
}
pub(crate) struct PreparedStore<'a> {
    store: &'a ItemVariationStore,
    scalars: Vec<f64>,
}
impl PreparedStore<'_> {
    /// Cost of an uncached row evaluation, for callers with aggregate work limits.
    pub fn delta_work(&self, outer: u32, inner: u32) -> Result<usize> {
        if outer == 0xffff && inner == 0xffff {
            return Ok(0);
        }
        let set = self
            .store
            .sets
            .get(outer as usize)
            .ok_or_else(|| fail("outer variation index"))?;
        let Some(set) = set else {
            return Ok(0);
        };
        if inner as usize >= set.items {
            return Err(fail("inner variation index"));
        }
        Ok(set.regions.len())
    }
    pub fn delta(&self, outer: u32, inner: u32) -> Result<f64> {
        crate::cancel::check_current_cancel("font instance delta")?;
        if outer == 0xffff && inner == 0xffff {
            return Ok(0.);
        }
        let set = self
            .store
            .sets
            .get(outer as usize)
            .ok_or_else(|| fail("outer variation index"))?;
        // A NULL subtable explicitly denotes no variation for every inner index.
        let Some(set) = set else {
            return Ok(0.);
        };
        if inner as usize >= set.items {
            return Err(fail("inner variation index"));
        }
        let stride = (set.regions.len() + set.words) * if set.long { 2 } else { 1 };
        let at = self.store.extent.start + set.rows.start + inner as usize * stride;
        let row = bytes(&self.store.source, at, stride)?;
        let mut at = 0;
        let mut value = 0.;
        for (i, region) in set.regions.iter().enumerate() {
            if i % 256 == 0 {
                crate::cancel::check_current_cancel("font instance delta row")?;
            }
            let size = match (set.long, i < set.words) {
                (true, true) => 4,
                (true, false) | (false, true) => 2,
                _ => 1,
            };
            let delta = match size {
                4 => i32::from_be_bytes(row[at..at + 4].try_into().unwrap()),
                2 => i32::from(i16::from_be_bytes(row[at..at + 2].try_into().unwrap())),
                _ => i32::from(row[at] as i8),
            };
            value += f64::from(delta) * self.scalars[*region];
            at += size;
        }
        Ok(value)
    }
}

/// Packed HVAR/VVAR/COLR delta indices, with the specified last-entry extension.
pub(crate) struct DeltaSetIndexMap<'a> {
    entries: &'a [u8],
    size: usize,
    inner_bits: u32,
}
impl<'a> DeltaSetIndexMap<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let head = bytes(data, 0, 2)?;
        let (count, base) = match head[0] {
            0 => (usize::from(u16_at(data, 2)?), 4),
            1 => (u32_at(data, 2)? as usize, 6),
            _ => return Err(fail("unsupported delta index map format")),
        };
        if head[1] & 0xc0 != 0 {
            return Err(fail("reserved delta index map bits"));
        }
        if count > 1_000_000 {
            return Err(WellfriendError::ResourceLimit(
                "delta index map budget".into(),
            ));
        }
        let size = usize::from((head[1] >> 4) & 3) + 1;
        Ok(Self {
            entries: bytes(data, base, count * size)?,
            size,
            inner_bits: u32::from(head[1] & 15) + 1,
        })
    }
    pub fn get(&self, item: u32) -> Result<(u32, u32)> {
        let count = self.entries.len() / self.size;
        if count == 0 {
            return Err(fail("empty delta index map has no last entry"));
        }
        let index = (item as usize).min(count - 1);
        let value = self.entries[index * self.size..(index + 1) * self.size]
            .iter()
            .fold(0u32, |n, byte| (n << 8) | u32::from(*byte));
        Ok((
            value >> self.inner_bits,
            value & ((1 << self.inner_bits) - 1),
        ))
    }
}
/// OpenType integer rounding: half-way cases go toward positive infinity.
pub(crate) fn round_i32(value: f64) -> Result<i32> {
    let rounded = (value + 0.5).floor();
    if !value.is_finite() || rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
        return Err(fail("instanced integer is outside signed 32-bit range"));
    }
    Ok(rounded as i32)
}

#[cfg(test)]
#[path = "variation_store_tests.rs"]
mod tests;
