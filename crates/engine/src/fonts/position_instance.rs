//! Shared ownership, checked numbers and Device/VariationIndex resolution for
//! static layout positioning. This is not a full sfnt transaction on its own.
use super::variation_store::{bytes, round_i32, u16_at, PreparedStore};
use crate::{Result, WellfriendError};
use std::{collections::BTreeMap, sync::Arc};
pub(super) const LIMIT: usize = 64 * 1024 * 1024;
pub(super) fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font position instance: {message}"))
}
pub(super) fn relative(data: &[u8], base: usize, offset: usize) -> Result<usize> {
    if offset == 0 {
        return Err(fail("unexpected null layout owner"));
    }
    let at = base
        .checked_add(offset)
        .ok_or_else(|| fail("layout offset overflow"))?;
    bytes(data, at, 2)?;
    Ok(at)
}
pub(super) fn signed(data: &[u8], at: usize) -> Result<i16> {
    Ok(i16::from_be_bytes(bytes(data, at, 2)?.try_into().unwrap()))
}
pub(super) fn integer(value: f64) -> Result<i16> {
    i16::try_from(round_i32(value)?).map_err(|_| fail("position exceeds signed 16-bit range"))
}
pub(super) enum Device<'a> {
    None,
    Delta(f64),
    Hint(&'a [u8]),
}
pub(super) struct Resolver<'s, 't> {
    pub store: Option<&'s PreparedStore<'t>>,
    pub resolved: usize,
    pub hints: usize,
    pub evaluated_delta_cells: usize,
    /// Exact expanded contour-point counts from the point-preserving writer.
    /// None is deliberately unvalidated, not an empty outline authority.
    pub point_counts: Option<Arc<[u16]>>,
    pub checked_point_references: usize,
    pub unchecked_point_references: usize,
    deltas: BTreeMap<(u16, u16), f64>,
    work: usize,
}
impl<'s, 't> Resolver<'s, 't> {
    pub fn new(store: Option<&'s PreparedStore<'t>>) -> Self {
        Self {
            store,
            resolved: 0,
            hints: 0,
            evaluated_delta_cells: 0,
            point_counts: None,
            checked_point_references: 0,
            unchecked_point_references: 0,
            deltas: BTreeMap::new(),
            work: 0,
        }
    }
    pub fn distinct_variation_indices(&self) -> usize {
        self.deltas.len()
    }
    pub fn point(&mut self, glyph: u16, point: u16) -> Result<()> {
        self.charge(1)?;
        if let Some(counts) = &self.point_counts {
            let count = counts
                .get(usize::from(glyph))
                .ok_or_else(|| fail("contour reference glyph outside outline authority"))?;
            if point >= *count {
                return Err(fail(
                    "contour reference outside its glyph's explicit points",
                ));
            }
            self.checked_point_references += 1;
        } else {
            self.unchecked_point_references += 1;
        }
        Ok(())
    }
    /// Coverage order binds each record to its glyph. Ranges are expanded once,
    /// under the same work budget, rather than rescanned per anchor/point.
    pub fn coverage_glyphs(&mut self, data: &[u8], at: usize) -> Result<Vec<u16>> {
        let (_, count) = coverage_len(data, at)?;
        self.charge(count)?;
        let mut glyphs = Vec::with_capacity(count);
        match u16_at(data, at)? {
            1 => {
                for (index, record) in bytes(data, at + 4, count * 2)?.chunks_exact(2).enumerate() {
                    if index % 256 == 0 {
                        crate::cancel::check_current_cancel("layout point coverage binding")?;
                    }
                    glyphs.push(u16_at(record, 0)?);
                }
            }
            2 => {
                let ranges = usize::from(u16_at(data, at + 2)?);
                for record in bytes(data, at + 4, ranges * 6)?.chunks_exact(6) {
                    crate::cancel::check_current_cancel("layout point coverage binding")?;
                    glyphs.extend(u16_at(record, 0)?..=u16_at(record, 2)?);
                }
            }
            _ => unreachable!(), // coverage_len validated the format.
        }
        if let (Some(counts), Some(last)) = (&self.point_counts, glyphs.last()) {
            if usize::from(*last) >= counts.len() {
                return Err(fail("coverage glyph outside outline authority"));
            }
        }
        Ok(glyphs)
    }
    fn delta(&mut self, outer: u16, inner: u16) -> Result<f64> {
        if (outer, inner) == (0xffff, 0xffff) {
            return Ok(0.);
        }
        if let Some(value) = self.deltas.get(&(outer, inner)) {
            return Ok(*value);
        }
        let store = self
            .store
            .ok_or_else(|| fail("VariationIndex has no owning variation store"))?;
        let work = store.delta_work(u32::from(outer), u32::from(inner))?;
        self.evaluated_delta_cells = self
            .evaluated_delta_cells
            .checked_add(work)
            .ok_or_else(|| fail("variation evaluation work overflow"))?;
        if self.evaluated_delta_cells > 4_000_000 || self.deltas.len() >= 65536 {
            return Err(WellfriendError::ResourceLimit(
                "position variation evaluation/cache budget".into(),
            ));
        }
        let value = store.delta(u32::from(outer), u32::from(inner))?;
        self.deltas.insert((outer, inner), value);
        Ok(value)
    }
    pub fn charge(&mut self, count: usize) -> Result<()> {
        self.work = self
            .work
            .checked_add(count)
            .ok_or_else(|| fail("position work overflow"))?;
        if self.work > 1_000_000 {
            return Err(WellfriendError::ResourceLimit(
                "layout position work budget".into(),
            ));
        }
        crate::cancel::check_current_cancel("font position evaluation")
    }
    pub fn device<'a>(&mut self, data: &'a [u8], base: usize, offset: u16) -> Result<Device<'a>> {
        self.charge(1)?;
        if offset == 0 {
            return Ok(Device::None);
        }
        let at = relative(data, base, usize::from(offset))?;
        let first = u16_at(data, at)?;
        let second = u16_at(data, at + 2)?;
        match u16_at(data, at + 4)? {
            0x8000 => {
                let delta = self.delta(first, second)?;
                self.resolved += 1;
                Ok(Device::Delta(delta))
            }
            format @ 1..=3 => {
                if first > second {
                    return Err(fail("inverted Device size range"));
                }
                let bits = 1usize << format;
                let words = ((usize::from(second - first) + 1) * bits).div_ceil(16);
                self.hints += 1;
                Ok(Device::Hint(bytes(data, at, 6 + words * 2)?))
            }
            _ => Err(fail("unknown Device/VariationIndex format")),
        }
    }
}
pub(super) struct Writer {
    pub out: Vec<u8>,
    copies: BTreeMap<(usize, usize), Vec<usize>>,
}
impl Writer {
    pub fn new() -> Self {
        Self {
            out: Vec::new(),
            copies: BTreeMap::new(),
        }
    }
    pub fn reserve(&mut self, length: usize) -> Result<usize> {
        let at = self.out.len();
        let end = at
            .checked_add(length)
            .ok_or_else(|| fail("position table size overflow"))?;
        if end > LIMIT {
            return Err(WellfriendError::ResourceLimit(
                "position table exceeds 64 MiB".into(),
            ));
        }
        crate::cancel::check_current_cancel("font position allocation")?;
        self.out.resize(end, 0);
        Ok(at)
    }
    pub fn copy(&mut self, data: &[u8]) -> Result<usize> {
        let at = self.reserve(data.len())?;
        for (i, chunk) in data.chunks(65536).enumerate() {
            crate::cancel::check_current_cancel("font position source copy")?;
            self.out[at + i * 65536..at + i * 65536 + chunk.len()].copy_from_slice(chunk);
        }
        Ok(at)
    }
    pub fn word(&mut self, at: usize, value: u16) {
        self.out[at..at + 2].copy_from_slice(&value.to_be_bytes());
    }
    pub fn dword(&mut self, at: usize, value: u32) {
        self.out[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    pub fn link32(&mut self, field: usize, base: usize, target: usize) -> Result<()> {
        let offset = target
            .checked_sub(base)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n != 0)
            .ok_or_else(|| fail("position 32-bit offset overflow"))?;
        self.dword(field, offset);
        Ok(())
    }
    pub fn link(&mut self, field: usize, base: usize, target: usize) -> Result<()> {
        let offset = target
            .checked_sub(base)
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| *n != 0)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "position graph needs 16-bit offset repacking/splitting".into(),
                )
            })?;
        self.word(field, offset);
        Ok(())
    }
    /// Share immutable leaf data only when its relocated address is representable
    /// from this owner; otherwise make a new owner-local copy.
    pub fn leaf(&mut self, data: &[u8], source: usize, len: usize, base: usize) -> Result<usize> {
        if let Some(target) = self.copies.get(&(source, len)).and_then(|copies| {
            // Addresses are append-ordered. A shared source repeatedly cloned
            // for new owners must not scan every prior copy on every lookup.
            copies
                .get(copies.partition_point(|at| *at <= base))
                .copied()
                .filter(|at| *at - base <= 65535)
        }) {
            return Ok(target);
        }
        let target = self.copy(bytes(data, source, len)?)?;
        self.copies.entry((source, len)).or_default().push(target);
        Ok(target)
    }
}

pub(super) fn coverage_len(data: &[u8], at: usize) -> Result<(usize, usize)> {
    let count = usize::from(u16_at(data, at + 2)?);
    let mut previous = None;
    let mut glyphs = 0usize;
    let length = match u16_at(data, at)? {
        1 => {
            for item in bytes(data, at + 4, count * 2)?.chunks_exact(2) {
                crate::cancel::check_current_cancel("font position coverage")?;
                let gid = u16::from_be_bytes(item.try_into().unwrap());
                if previous.is_some_and(|last| last >= gid) {
                    return Err(fail("unordered coverage glyphs"));
                }
                previous = Some(gid);
            }
            glyphs = count;
            4 + count * 2
        }
        2 => {
            for item in bytes(data, at + 4, count * 6)?.chunks_exact(6) {
                crate::cancel::check_current_cancel("font position coverage ranges")?;
                let first = u16_at(item, 0)?;
                let last = u16_at(item, 2)?;
                if first > last
                    || previous.is_some_and(|end| end >= first)
                    || usize::from(u16_at(item, 4)?) != glyphs
                {
                    return Err(fail("invalid coverage ranges or indices"));
                }
                glyphs += usize::from(last - first) + 1;
                previous = Some(last);
            }
            4 + count * 6
        }
        _ => return Err(fail("unknown coverage format")),
    };
    Ok((length, glyphs))
}
pub(super) fn class_len(data: &[u8], at: usize, classes: usize) -> Result<usize> {
    match u16_at(data, at)? {
        1 => {
            let count = usize::from(u16_at(data, at + 4)?);
            if usize::from(u16_at(data, at + 2)?) + count > 65536 {
                return Err(fail("class glyph range overflow"));
            }
            for item in bytes(data, at + 6, count * 2)?.chunks_exact(2) {
                crate::cancel::check_current_cancel("font position classes")?;
                if usize::from(u16::from_be_bytes(item.try_into().unwrap())) >= classes {
                    return Err(fail("class outside positioning matrix"));
                }
            }
            Ok(6 + count * 2)
        }
        2 => {
            let count = usize::from(u16_at(data, at + 2)?);
            let mut previous = None;
            for item in bytes(data, at + 4, count * 6)?.chunks_exact(6) {
                crate::cancel::check_current_cancel("font position class ranges")?;
                let first = u16_at(item, 0)?;
                let last = u16_at(item, 2)?;
                if first > last
                    || previous.is_some_and(|end| end >= first)
                    || usize::from(u16_at(item, 4)?) >= classes
                {
                    return Err(fail("invalid class ranges"));
                }
                previous = Some(last);
            }
            Ok(4 + count * 6)
        }
        _ => Err(fail("unknown class definition format")),
    }
}
