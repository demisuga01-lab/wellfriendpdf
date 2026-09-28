//! Freeze JSTF's embedded positioning using the same GDEF-owned resolver as
//! GPOS. Priority order and external GSUB/GPOS lookup identities are retained.
use super::{
    gpos_instance,
    position_instance::{fail, relative, Resolver, Writer, LIMIT},
    variation_store::{bytes, u16_at, u32_at},
};
use crate::{Result, WellfriendError};
use std::collections::BTreeMap;

pub(crate) struct FrozenJstf {
    pub bytes: Vec<u8>,
    pub contour_point_references: usize,
}
struct Program<'a, 'r, 's, 't> {
    data: &'a [u8],
    resolver: &'r mut Resolver<'s, 't>,
    w: Writer,
    lookup_counts: [usize; 2],
    glyph_count: usize,
    pending: Vec<(usize, u16, usize)>,
}
impl Program<'_, '_, '_, '_> {
    fn child(&self, base: usize, field: usize) -> Result<usize> {
        relative(self.data, base, usize::from(u16_at(self.data, field)?))
    }
    fn tags(&mut self, at: usize, count: usize) -> Result<()> {
        self.resolver.charge(count)?;
        bytes(self.data, at, count * 6)?;
        let mut previous = None;
        for i in 0..count {
            crate::cancel::check_current_cancel("font justification tags")?;
            let tag = u32_at(self.data, at + i * 6)?;
            if previous.is_some_and(|last| last >= tag) {
                return Err(fail("unordered or duplicate JSTF tags"));
            }
            previous = Some(tag);
        }
        Ok(())
    }
    fn index_list(&mut self, source: usize, bound: usize, owner: usize) -> Result<usize> {
        let count = usize::from(u16_at(self.data, source)?);
        self.resolver.charge(count + 1)?;
        let mut previous = None;
        for item in bytes(self.data, source + 2, count * 2)?.chunks_exact(2) {
            crate::cancel::check_current_cancel("font justification index list")?;
            let index = usize::from(u16_at(item, 0)?);
            if index >= bound || previous.is_some_and(|last| last >= index) {
                return Err(fail("JSTF lookup/glyph index order or bounds"));
            }
            previous = Some(index);
        }
        self.w.leaf(self.data, source, 2 + count * 2, owner)
    }
    fn maximum(&mut self, source: usize) -> Result<usize> {
        let count = usize::from(u16_at(self.data, source)?);
        self.resolver.charge(count + 1)?;
        bytes(self.data, source, 2 + count * 2)?;
        let target = self.w.reserve(2 + count * 2)?;
        self.w.word(target, count as u16);
        for i in 0..count {
            let lookup = self.child(source, source + 2 + i * 2)?;
            let kind = u16_at(self.data, lookup)?;
            if !matches!(kind, 1..=6 | 9) {
                return Err(fail("contextual or unknown JSTF lookup"));
            }
            let flags = u16_at(self.data, lookup + 2)?;
            let subtables = usize::from(u16_at(self.data, lookup + 4)?);
            self.resolver.charge(subtables)?;
            let copied = self.w.copy(bytes(
                self.data,
                lookup,
                6 + subtables * 2 + if flags & 16 != 0 { 2 } else { 0 },
            )?)?;
            self.w.word(copied, 9);
            self.w.link(target + 2 + i * 2, target, copied)?;
            let mut extension_kind = None;
            for j in 0..subtables {
                let subtable = self.child(lookup, lookup + 6 + j * 2)?;
                let (inner, at) = if kind == 9 {
                    if u16_at(self.data, subtable)? != 1 {
                        return Err(fail("unknown JSTF extension format"));
                    }
                    let inner = u16_at(self.data, subtable + 2)?;
                    if !(1..=6).contains(&inner) || extension_kind.is_some_and(|last| last != inner)
                    {
                        return Err(fail("invalid or mixed JSTF extension kinds"));
                    }
                    extension_kind = Some(inner);
                    (
                        inner,
                        relative(
                            self.data,
                            subtable,
                            u32_at(self.data, subtable + 4)? as usize,
                        )?,
                    )
                } else {
                    (kind, subtable)
                };
                let wrapper = self.w.reserve(8)?;
                self.w.word(wrapper, 1);
                self.w.word(wrapper + 2, inner);
                self.w.link(copied + 6 + j * 2, copied, wrapper)?;
                self.pending.push((wrapper, inner, at));
            }
        }
        Ok(target)
    }
    fn priority(&mut self, source: usize) -> Result<usize> {
        self.resolver.charge(1)?;
        bytes(self.data, source, 20)?;
        let target = self.w.reserve(20)?;
        for i in 0..10 {
            if u16_at(self.data, source + i * 2)? == 0 {
                continue;
            }
            let at = self.child(source, source + i * 2)?;
            let copied = if i % 5 == 4 {
                self.maximum(at)?
            } else {
                let bound = self.lookup_counts[usize::from(i % 5 >= 2)];
                self.index_list(at, bound, target)?
            };
            self.w.link(target + i * 2, target, copied)?;
        }
        Ok(target)
    }
    fn language(&mut self, source: usize) -> Result<usize> {
        let count = usize::from(u16_at(self.data, source)?);
        self.resolver.charge(count + 1)?;
        bytes(self.data, source, 2 + count * 2)?;
        let target = self.w.reserve(2 + count * 2)?;
        self.w.word(target, count as u16);
        for i in 0..count {
            let at = self.child(source, source + 2 + i * 2)?;
            let priority = self.priority(at)?;
            self.w.link(target + 2 + i * 2, target, priority)?;
        }
        Ok(target)
    }
    fn script(&mut self, source: usize) -> Result<usize> {
        let count = usize::from(u16_at(self.data, source + 4)?);
        self.resolver.charge(1)?;
        self.tags(source + 6, count)?;
        let target = self.w.reserve(6 + count * 6)?;
        self.w.word(target + 4, count as u16);
        if u16_at(self.data, source)? != 0 {
            let at = self.child(source, source)?;
            let extenders = self.index_list(at, self.glyph_count, target)?;
            self.w.link(target, target, extenders)?;
        }
        if u16_at(self.data, source + 2)? != 0 {
            let at = self.child(source, source + 2)?;
            let language = self.language(at)?;
            self.w.link(target + 2, target, language)?;
        }
        for i in 0..count {
            let src = source + 6 + i * 6;
            let dst = target + 6 + i * 6;
            self.w.dword(dst, u32_at(self.data, src)?);
            let at = self.child(source, src + 4)?;
            let language = self.language(at)?;
            self.w.link(dst + 4, target, language)?;
        }
        Ok(target)
    }
}
pub(super) fn freeze(
    data: &[u8],
    resolver: &mut Resolver<'_, '_>,
    lookup_counts: [usize; 2],
    glyph_count: usize,
) -> Result<FrozenJstf> {
    crate::cancel::check_current_cancel("JSTF instance preparation")?;
    if data.len() > LIMIT {
        return Err(WellfriendError::ResourceLimit("JSTF exceeds 64 MiB".into()));
    }
    if u32_at(data, 0)? != 0x10000 {
        return Err(fail("unsupported JSTF version"));
    }
    let count = usize::from(u16_at(data, 4)?);
    let mut p = Program {
        data,
        resolver,
        w: Writer::new(),
        lookup_counts,
        glyph_count,
        pending: Vec::new(),
    };
    p.tags(6, count)?;
    p.w.reserve(6 + count * 6)?;
    p.w.dword(0, 0x10000);
    p.w.word(4, count as u16);
    for i in 0..count {
        let record = 6 + i * 6;
        p.w.dword(record, u32_at(data, record)?);
        let source = p.child(0, record + 4)?;
        if source < 6 + count * 6 {
            return Err(fail("JSTF script overlaps header"));
        }
        let script = p.script(source)?;
        p.w.link(record + 4, 0, script)?;
    }
    // Allocate lookup owners before their large programs; all extension links
    // point forward and aliases share exactly one rewritten program.
    let mut emitted = BTreeMap::new();
    let mut points = 0usize;
    for (wrapper, kind, at) in p.pending {
        crate::cancel::check_current_cancel("JSTF lookup publication")?;
        let target = if let Some(target) = emitted.get(&(kind, at)) {
            *target
        } else {
            let (program, references) = gpos_instance::freeze_program(data, kind, at, p.resolver)?;
            points += references;
            let target = p.w.copy(&program)?;
            emitted.insert((kind, at), target);
            target
        };
        p.w.link32(wrapper + 4, wrapper, target)?;
    }
    Ok(FrozenJstf {
        bytes: p.w.out,
        contour_point_references: points,
    })
}
