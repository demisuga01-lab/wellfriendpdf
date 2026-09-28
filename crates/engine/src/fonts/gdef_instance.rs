//! Rebuild GDEF ownership without modifying aliased source records in place.
//! The variation store is retained until all consumers (including JSTF) close.
use super::{
    position_instance::{
        class_len, coverage_len, fail, integer, relative, signed, Device, Resolver, Writer, LIMIT,
    },
    variation_store::{bytes, u16_at, u32_at},
};
use crate::{Result, WellfriendError};

pub(crate) struct FrozenGdef {
    pub bytes: Vec<u8>,
    pub retained_store: bool,
    pub contour_point_references: usize,
}
pub(super) fn header(data: &[u8]) -> Result<(usize, usize)> {
    if data.len() > LIMIT {
        return Err(WellfriendError::ResourceLimit("GDEF exceeds 64 MiB".into()));
    }
    let size = match u32_at(data, 0)? {
        0x10000 => 12,
        0x10002 => 14,
        0x10003 => 18,
        _ => return Err(fail("unsupported GDEF version")),
    };
    bytes(data, 0, size)?;
    let store = if size == 18 {
        u32_at(data, 14)? as usize
    } else {
        0
    };
    if store != 0 {
        if store < size {
            return Err(fail("GDEF store overlaps header"));
        }
        bytes(data, store, 2)?;
    }
    Ok((size, store))
}
struct Program<'a, 'r, 's, 't> {
    data: &'a [u8],
    resolver: &'r mut Resolver<'s, 't>,
    w: Writer,
    points: usize,
}
impl Program<'_, '_, '_, '_> {
    fn child(&self, base: usize, field: usize) -> Result<usize> {
        relative(self.data, base, usize::from(u16_at(self.data, field)?))
    }
    fn coverage(&mut self, source: usize, count: Option<usize>, owner: usize) -> Result<usize> {
        let (len, glyphs) = coverage_len(self.data, source)?;
        self.resolver.charge(glyphs)?;
        if count.is_some_and(|n| n != glyphs) {
            return Err(fail("GDEF coverage count mismatch"));
        }
        self.w.leaf(self.data, source, len, owner)
    }
    fn attach(&mut self, source: usize) -> Result<usize> {
        let count = usize::from(u16_at(self.data, source + 2)?);
        self.resolver.charge(count)?;
        bytes(self.data, source, 4 + count * 2)?;
        let target = self.w.reserve(4 + count * 2)?;
        self.w.word(target + 2, count as u16);
        let coverage = self.child(source, source)?;
        let glyphs = self.resolver.coverage_glyphs(self.data, coverage)?;
        if glyphs.len() != count {
            return Err(fail("attachment coverage and records disagree"));
        }
        for i in 0..count {
            let item = self.child(source, source + 4 + i * 2)?;
            let points = usize::from(u16_at(self.data, item)?);
            self.resolver.charge(points)?;
            let mut previous = None;
            for record in bytes(self.data, item + 2, points * 2)?.chunks_exact(2) {
                crate::cancel::check_current_cancel("font attachment points")?;
                let point = u16_at(record, 0)?;
                if previous.is_some_and(|n| n >= point) {
                    return Err(fail("unordered attachment points"));
                }
                previous = Some(point);
                self.resolver.point(glyphs[i], point)?;
            }
            self.points += points;
            let copied = self.w.leaf(self.data, item, 2 + points * 2, target)?;
            self.w.link(target + 4 + i * 2, target, copied)?;
        }
        let coverage = self.child(source, source)?;
        let coverage = self.coverage(coverage, Some(count), target)?;
        self.w.link(target, target, coverage)?;
        Ok(target)
    }
    fn caret(&mut self, source: usize, glyph: u16) -> Result<usize> {
        self.resolver.charge(1)?;
        let format = u16_at(self.data, source)?;
        let len = match format {
            1 | 2 => 4,
            3 => 6,
            _ => return Err(fail("unknown ligature caret format")),
        };
        let target = self.w.copy(bytes(self.data, source, len)?)?;
        if format == 2 {
            self.resolver.point(glyph, u16_at(self.data, source + 2)?)?;
            self.points += 1;
        }
        if format == 3 {
            self.w.word(target + 4, 0);
            let offset = u16_at(self.data, source + 4)?;
            match self.resolver.device(self.data, source, offset)? {
                Device::None => {}
                Device::Delta(delta) => {
                    let value = integer(f64::from(signed(self.data, source + 2)?) + delta)?;
                    self.w.word(target + 2, value as u16);
                }
                Device::Hint(hint) => {
                    let at = relative(self.data, source, usize::from(offset))?;
                    let hint = self.w.leaf(self.data, at, hint.len(), target)?;
                    self.w.link(target + 4, target, hint)?;
                }
            }
        }
        Ok(target)
    }
    fn carets(&mut self, source: usize) -> Result<usize> {
        let count = usize::from(u16_at(self.data, source + 2)?);
        self.resolver.charge(count)?;
        bytes(self.data, source, 4 + count * 2)?;
        let target = self.w.reserve(4 + count * 2)?;
        self.w.word(target + 2, count as u16);
        let coverage = self.child(source, source)?;
        let glyphs = self.resolver.coverage_glyphs(self.data, coverage)?;
        if glyphs.len() != count {
            return Err(fail("ligature coverage and caret records disagree"));
        }
        for i in 0..count {
            let glyph = self.child(source, source + 4 + i * 2)?;
            let n = usize::from(u16_at(self.data, glyph)?);
            self.resolver.charge(n)?;
            bytes(self.data, glyph, 2 + n * 2)?;
            let copied = self.w.reserve(2 + n * 2)?;
            self.w.word(copied, n as u16);
            self.w.link(target + 4 + i * 2, target, copied)?;
            for j in 0..n {
                let caret = self.child(glyph, glyph + 2 + j * 2)?;
                let caret = self.caret(caret, glyphs[i])?;
                self.w.link(copied + 2 + j * 2, copied, caret)?;
            }
        }
        let coverage = self.child(source, source)?;
        let coverage = self.coverage(coverage, Some(count), target)?;
        self.w.link(target, target, coverage)?;
        Ok(target)
    }
    fn mark_sets(&mut self, source: usize) -> Result<usize> {
        if u16_at(self.data, source)? != 1 {
            return Err(fail("unknown mark glyph sets format"));
        }
        let count = usize::from(u16_at(self.data, source + 2)?);
        self.resolver.charge(count)?;
        bytes(self.data, source, 4 + count * 4)?;
        let target = self.w.reserve(4 + count * 4)?;
        self.w.word(target, 1);
        self.w.word(target + 2, count as u16);
        for i in 0..count {
            let at = relative(
                self.data,
                source,
                u32_at(self.data, source + 4 + i * 4)? as usize,
            )?;
            let coverage = self.coverage(at, None, target)?;
            self.w.link32(target + 4 + i * 4, target, coverage)?;
        }
        Ok(target)
    }
}
pub(super) fn freeze(
    data: &[u8],
    resolver: &mut Resolver<'_, '_>,
    retain_store: bool,
) -> Result<FrozenGdef> {
    let (size, store) = header(data)?;
    let mut p = Program {
        data,
        resolver,
        w: Writer::new(),
        points: 0,
    };
    let output_size = if size == 18 && !retain_store {
        14
    } else {
        size
    };
    p.w.copy(bytes(data, 0, output_size)?)?;
    if output_size != size {
        p.w.dword(0, 0x10002);
    }
    // Rebuild all owner graphs so a source alias cannot change an unrelated table.
    p.w.out[4..].fill(0);
    for field in [4, 6, 8, 10, 12] {
        if field >= size {
            continue;
        }
        let offset = usize::from(u16_at(data, field)?);
        if offset == 0 {
            continue;
        }
        if offset < size {
            return Err(fail("GDEF child overlaps header"));
        }
        let target = match field {
            4 | 10 => {
                let len = class_len(data, offset, if field == 4 { 5 } else { 65536 })?;
                p.resolver.charge(len / 2)?;
                p.w.leaf(data, offset, len, 0)?
            }
            6 => p.attach(offset)?,
            8 => p.carets(offset)?,
            12 => p.mark_sets(offset)?,
            _ => unreachable!(),
        };
        p.w.link(field, 0, target)?;
    }
    if store != 0 && retain_store {
        let at = p.w.copy(&data[store..])?;
        p.w.link32(14, 0, at)?;
    }
    crate::cancel::check_current_cancel("GDEF instance publication")?;
    Ok(FrozenGdef {
        bytes: p.w.out,
        retained_store: store != 0 && retain_store,
        contour_point_references: p.points,
    })
}
