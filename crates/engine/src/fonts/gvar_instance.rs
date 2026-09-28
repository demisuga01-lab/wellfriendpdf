//! Source-owned gvar directory and tuple evaluation. This retains explicit point
//! numbering; outline serialization and TrueType instruction execution are separate.
use super::{
    tuple_variations::{self, Budget, Deltas, Domain, TABLE_LIMIT},
    variation_store::{bytes, u16_at, u32_at},
};
use crate::{Result, WellfriendError};
use std::{ops::Range, sync::Arc};

fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("gvar instance: {message}"))
}
pub(crate) struct PreparedGvar {
    data: Arc<[u8]>,
    coordinates: Vec<ttf_parser::NormalizedCoordinate>,
    shared: Vec<Vec<i16>>,
    glyphs: Vec<Range<usize>>,
    pub directory_work: usize,
}
impl PreparedGvar {
    pub(crate) fn prepare(
        data: Arc<[u8]>,
        coordinates: &[ttf_parser::NormalizedCoordinate],
        glyph_count: u16,
    ) -> Result<Self> {
        let mut budget = Budget::default();
        budget.charge(1)?;
        if data.len() > TABLE_LIMIT {
            return Err(WellfriendError::ResourceLimit("gvar exceeds 64 MiB".into()));
        }
        if coordinates.len() > 64 {
            return Err(fail("more than 64 axes"));
        }
        if u32_at(&data, 0)? != 0x0001_0000 {
            return Err(fail("unsupported table version"));
        }
        let axes = usize::from(u16_at(&data, 4)?);
        if axes != coordinates.len() {
            return Err(fail("axis count differs from selected fvar coordinates"));
        }
        let shared_count = usize::from(u16_at(&data, 6)?);
        let shared_start = u32_at(&data, 8)? as usize;
        if u16_at(&data, 12)? != glyph_count {
            return Err(fail("glyph count differs from maxp"));
        }
        let flags = u16_at(&data, 14)?;
        if flags & !1 != 0 {
            return Err(fail("reserved table flags"));
        }
        let base = u32_at(&data, 16)? as usize;
        let size = if flags & 1 == 0 { 2 } else { 4 };
        let offset_count = usize::from(glyph_count) + 1;
        let header_end = 20 + offset_count * size;
        bytes(&data, 20, offset_count * size)?;
        if base < header_end || base > data.len() {
            return Err(fail("glyph data array overlaps header or exceeds table"));
        }
        budget.charge(offset_count + shared_count * (axes + 1))?;
        let mut offsets = Vec::with_capacity(offset_count);
        for index in 0..offset_count {
            if index % 256 == 0 {
                crate::cancel::check_current_cancel("gvar directory offsets")?;
            }
            let relative = if size == 2 {
                usize::from(u16_at(&data, 20 + index * 2)?) * 2
            } else {
                u32_at(&data, 20 + index * 4)? as usize
            };
            let at = base
                .checked_add(relative)
                .ok_or_else(|| fail("glyph data offset overflow"))?;
            if at > data.len() || offsets.last().is_some_and(|previous| *previous > at) {
                return Err(fail("descending or out-of-range glyph offsets"));
            }
            offsets.push(at);
        }
        let mut shared = Vec::with_capacity(shared_count);
        if shared_count != 0 {
            let length = shared_count * axes * 2;
            let source = bytes(&data, shared_start, length)?;
            let shared_end = shared_start + length;
            if shared_start < header_end {
                return Err(fail("shared tuples overlap header"));
            }
            let first = offsets[0];
            let last = offsets[offsets.len() - 1];
            if first < last && shared_start < last && first < shared_end {
                return Err(fail("shared tuples overlap glyph variation stores"));
            }
            // Zero-axis records are empty, but retain their index identities.
            for index in 0..shared_count {
                if index % 256 == 0 {
                    crate::cancel::check_current_cancel("gvar shared tuples")?;
                }
                let tuple = bytes(source, index * axes * 2, axes * 2)?;
                let mut peak = Vec::with_capacity(axes);
                for value in tuple.chunks_exact(2) {
                    let value = i16::from_be_bytes(value.try_into().unwrap());
                    if !(-16384..=16384).contains(&value) {
                        return Err(fail("shared peak outside normalized domain"));
                    }
                    peak.push(value);
                }
                shared.push(peak);
            }
        }
        let glyphs = offsets.windows(2).map(|pair| pair[0]..pair[1]).collect();
        budget.charge(0)?;
        Ok(Self {
            data,
            coordinates: coordinates.to_vec(),
            shared,
            glyphs,
            directory_work: budget.work,
        })
    }
    pub(crate) fn resolve(
        &self,
        glyph: u16,
        domain: Domain<'_>,
        budget: &mut Budget,
    ) -> Result<Deltas> {
        budget.charge(1)?;
        if matches!(domain, Domain::Cvt(_)) {
            return Err(fail("CVT domain cannot be used for glyph deltas"));
        }
        let source = self
            .glyphs
            .get(usize::from(glyph))
            .ok_or_else(|| fail("glyph ID outside directory"))?;
        if source.is_empty() {
            let count = domain.count()?;
            budget.charge(count)?;
            return Ok(Deltas {
                values: vec![[0.; 2]; count],
                tuples: 0,
                active_tuples: 0,
            });
        }
        tuple_variations::resolve(
            &self.data[source.clone()],
            &self.coordinates,
            &self.shared,
            domain,
            budget,
        )
    }
}

#[cfg(test)]
#[path = "gvar_instance_tests.rs"]
mod tests;
