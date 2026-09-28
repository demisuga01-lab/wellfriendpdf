//! Rebuild positioning programs with resolved variable values. Contextual
//! dispatch keeps its exact source bytes and stable lookup indices.
use super::{
    layout_instance::{self, FrozenLayout},
    position_instance::{
        class_len, coverage_len, fail, integer, relative, signed, Device, Resolver, Writer,
    },
    variation_store::{bytes, u16_at},
};
use crate::{Result, WellfriendError};
use std::collections::BTreeMap;

pub(crate) struct FrozenGpos {
    pub layout: FrozenLayout,
    /// Anchor format 2 still depends on original contour point identities.
    pub contour_point_anchors: usize,
}
fn size(format: u16) -> Result<usize> {
    if format & 0xff00 != 0 {
        return Err(fail("reserved ValueFormat bits"));
    }
    Ok(format.count_ones() as usize * 2)
}
fn expanded(format: u16) -> u16 {
    format | ((format >> 4) & 15)
}
struct Program<'a, 'r, 's, 't> {
    data: &'a [u8],
    resolver: &'r mut Resolver<'s, 't>,
    w: Writer,
    anchors: BTreeMap<usize, Vec<usize>>,
    contour_points: usize,
}
impl Program<'_, '_, '_, '_> {
    fn source(&self, base: usize, field: usize) -> Result<usize> {
        relative(self.data, base, usize::from(u16_at(self.data, field)?))
    }
    fn coverage(
        &mut self,
        root: usize,
        source_field: usize,
        dest_field: usize,
        expected: Option<usize>,
    ) -> Result<()> {
        let source = self.source(root, source_field)?;
        let (length, count) = coverage_len(self.data, source)?;
        self.resolver.charge(count)?;
        if expected.is_some_and(|n| n != count) {
            return Err(fail("coverage and positioning counts disagree"));
        }
        let target = self.w.leaf(self.data, source, length, 0)?;
        self.w.link(dest_field, 0, target)
    }
    fn device(
        &mut self,
        source_base: usize,
        source_offset: u16,
        target_field: usize,
        target_base: usize,
        value: i16,
    ) -> Result<i16> {
        match self
            .resolver
            .device(self.data, source_base, source_offset)?
        {
            Device::None => Ok(value),
            Device::Delta(delta) => integer(f64::from(value) + delta),
            Device::Hint(hint) => {
                let at = relative(self.data, source_base, usize::from(source_offset))?;
                let target = self.w.leaf(self.data, at, hint.len(), target_base)?;
                self.w.link(target_field, target_base, target)?;
                Ok(value)
            }
        }
    }
    fn value(
        &mut self,
        source_base: usize,
        at: usize,
        format: u16,
        target_base: usize,
        target: usize,
    ) -> Result<usize> {
        self.resolver.charge(1)?;
        let input_size = size(format)?;
        bytes(self.data, at, input_size)?;
        let output_format = expanded(format);
        let mut values = [0i16; 4];
        let mut offsets = [0u16; 4];
        let mut src = at;
        for (i, value) in values.iter_mut().enumerate() {
            if format & (1 << i) != 0 {
                *value = signed(self.data, src)?;
                src += 2;
            }
        }
        for (i, offset) in offsets.iter_mut().enumerate() {
            if format & (16 << i) != 0 {
                *offset = u16_at(self.data, src)?;
                src += 2;
            }
        }
        let mut coordinate_fields = [None; 4];
        let mut dst = target;
        for (i, field) in coordinate_fields.iter_mut().enumerate() {
            if output_format & (1 << i) != 0 {
                *field = Some(dst);
                dst += 2;
            }
        }
        for i in 0..4 {
            if format & (16 << i) != 0 {
                values[i] = self.device(source_base, offsets[i], dst, target_base, values[i])?;
                dst += 2;
            }
            if let Some(field) = coordinate_fields[i] {
                self.w.word(field, values[i] as u16);
            }
        }
        Ok(src)
    }
    fn anchor(&mut self, source: usize, owner: usize, glyph: u16) -> Result<usize> {
        // Validate BEFORE relocation-cache reuse: the same record can belong
        // to distinct glyphs with different point domains.
        let format = u16_at(self.data, source)?;
        if format == 2 {
            self.resolver.point(glyph, u16_at(self.data, source + 6)?)?;
        }
        if let Some(target) = self.anchors.get(&source).and_then(|copies| {
            copies
                .get(copies.partition_point(|at| *at <= owner))
                .copied()
                .filter(|at| *at - owner <= 65535)
        }) {
            return Ok(target);
        }
        self.resolver.charge(1)?;
        let length = match format {
            1 => 6,
            2 => 8,
            3 => 10,
            _ => return Err(fail("unknown anchor format")),
        };
        let target = self.w.copy(bytes(self.data, source, length)?)?;
        if format == 2 {
            self.contour_points += 1;
        }
        if format == 3 {
            self.w.out[target + 6..target + 10].fill(0);
            for axis in 0..2 {
                let value = self.device(
                    source,
                    u16_at(self.data, source + 6 + axis * 2)?,
                    target + 6 + axis * 2,
                    target,
                    signed(self.data, source + 2 + axis * 2)?,
                )?;
                self.w.word(target + 2 + axis * 2, value as u16);
            }
        }
        self.anchors.entry(source).or_default().push(target);
        Ok(target)
    }
    fn anchor_link(
        &mut self,
        source_base: usize,
        source_field: usize,
        target_base: usize,
        target_field: usize,
        required: bool,
        glyph: u16,
    ) -> Result<()> {
        let offset = u16_at(self.data, source_field)?;
        if offset == 0 {
            if required {
                return Err(fail("null required mark anchor"));
            }
            return Ok(());
        }
        let source = relative(self.data, source_base, usize::from(offset))?;
        let target = self.anchor(source, target_base, glyph)?;
        self.w.link(target_field, target_base, target)
    }
    fn single(&mut self, root: usize) -> Result<()> {
        let format = u16_at(self.data, root)?;
        let flags = u16_at(self.data, root + 4)?;
        let input_size = size(flags)?;
        let output_size = size(expanded(flags))?;
        let (header, count) = match format {
            1 => (6, 1),
            2 => (8, usize::from(u16_at(self.data, root + 6)?)),
            _ => return Err(fail("unknown SinglePos format")),
        };
        self.resolver.charge(count)?;
        bytes(self.data, root, header + count * input_size)?;
        self.w.reserve(header + count * output_size)?;
        self.w.word(0, format);
        self.w.word(4, expanded(flags));
        if format == 2 {
            self.w.word(6, count as u16);
        }
        for i in 0..count {
            self.value(
                root,
                root + header + i * input_size,
                flags,
                0,
                header + i * output_size,
            )?;
        }
        self.coverage(root, root + 2, 2, (format == 2).then_some(count))
    }
    fn pair(&mut self, root: usize) -> Result<()> {
        let format = u16_at(self.data, root)?;
        let flags = [u16_at(self.data, root + 4)?, u16_at(self.data, root + 6)?];
        let input = [size(flags[0])?, size(flags[1])?];
        let output = [size(expanded(flags[0]))?, size(expanded(flags[1]))?];
        match format {
            1 => {
                let count = usize::from(u16_at(self.data, root + 8)?);
                self.resolver.charge(count)?;
                bytes(self.data, root, 10 + count * 2)?;
                self.w.reserve(10 + count * 2)?;
                self.w.word(0, 1);
                self.w.word(8, count as u16);
                let mut sets = Vec::with_capacity(count);
                for i in 0..count {
                    let source = self.source(root, root + 10 + i * 2)?;
                    let pairs = usize::from(u16_at(self.data, source)?);
                    self.resolver.charge(pairs)?;
                    bytes(self.data, source, 2 + pairs * (2 + input[0] + input[1]))?;
                    let target = self.w.reserve(2 + pairs * (2 + output[0] + output[1]))?;
                    self.w.word(target, pairs as u16);
                    self.w.link(10 + i * 2, 0, target)?;
                    sets.push((source, target, pairs));
                }
                for (source, target, pairs) in sets {
                    let mut previous = None;
                    for i in 0..pairs {
                        let src = source + 2 + i * (2 + input[0] + input[1]);
                        let dst = target + 2 + i * (2 + output[0] + output[1]);
                        let gid = u16_at(self.data, src)?;
                        if previous.is_some_and(|last| last >= gid) {
                            return Err(fail("PairSet second glyphs are not strictly sorted"));
                        }
                        previous = Some(gid);
                        self.w.word(dst, gid);
                        self.value(source, src + 2, flags[0], target, dst + 2)?;
                        self.value(
                            source,
                            src + 2 + input[0],
                            flags[1],
                            target,
                            dst + 2 + output[0],
                        )?;
                    }
                }
                self.coverage(root, root + 2, 2, Some(count))?;
            }
            2 => {
                let rows = usize::from(u16_at(self.data, root + 12)?);
                let cols = usize::from(u16_at(self.data, root + 14)?);
                if rows == 0 || cols == 0 {
                    return Err(fail("empty PairPos class dimensions"));
                }
                let cells = rows
                    .checked_mul(cols)
                    .ok_or_else(|| fail("class matrix overflow"))?;
                self.resolver.charge(cells)?;
                bytes(self.data, root + 16, cells * (input[0] + input[1]))?;
                self.w.reserve(16 + cells * (output[0] + output[1]))?;
                self.w.word(0, 2);
                self.w.word(12, rows as u16);
                self.w.word(14, cols as u16);
                for i in 0..cells {
                    let src = root + 16 + i * (input[0] + input[1]);
                    let dst = 16 + i * (output[0] + output[1]);
                    self.value(root, src, flags[0], 0, dst)?;
                    self.value(root, src + input[0], flags[1], 0, dst + output[0])?;
                }
                self.coverage(root, root + 2, 2, None)?;
                for (field, classes) in [(8, rows), (10, cols)] {
                    let source = self.source(root, root + field)?;
                    let length = class_len(self.data, source, classes)?;
                    self.resolver.charge(length / 2)?;
                    let target = self.w.leaf(self.data, source, length, 0)?;
                    self.w.link(field, 0, target)?;
                }
            }
            _ => return Err(fail("unknown PairPos format")),
        }
        self.w.word(4, expanded(flags[0]));
        self.w.word(6, expanded(flags[1]));
        Ok(())
    }
    fn cursive(&mut self, root: usize) -> Result<()> {
        if u16_at(self.data, root)? != 1 {
            return Err(fail("unknown CursivePos format"));
        }
        let count = usize::from(u16_at(self.data, root + 4)?);
        self.resolver.charge(count)?;
        bytes(self.data, root, 6 + count * 4)?;
        self.w.reserve(6 + count * 4)?;
        self.w.word(0, 1);
        self.w.word(4, count as u16);
        let coverage = self.source(root, root + 2)?;
        let glyphs = self.resolver.coverage_glyphs(self.data, coverage)?;
        if glyphs.len() != count {
            return Err(fail("cursive coverage and anchor counts disagree"));
        }
        for i in 0..count * 2 {
            self.anchor_link(root, root + 6 + i * 2, 0, 6 + i * 2, false, glyphs[i / 2])?;
        }
        self.coverage(root, root + 2, 2, Some(count))
    }
    fn mark_array(
        &mut self,
        source: usize,
        classes: usize,
        glyphs: &[u16],
    ) -> Result<(usize, usize)> {
        let count = usize::from(u16_at(self.data, source)?);
        if count != glyphs.len() {
            return Err(fail("mark coverage and anchor counts disagree"));
        }
        self.resolver.charge(count)?;
        bytes(self.data, source, 2 + count * 4)?;
        let target = self.w.reserve(2 + count * 4)?;
        self.w.word(target, count as u16);
        for i in 0..count {
            let class = u16_at(self.data, source + 2 + i * 4)?;
            if usize::from(class) >= classes {
                return Err(fail("mark class exceeds anchor matrix"));
            }
            self.w.word(target + 2 + i * 4, class);
            self.anchor_link(
                source,
                source + 4 + i * 4,
                target,
                target + 4 + i * 4,
                true,
                glyphs[i],
            )?;
        }
        Ok((target, count))
    }
    fn matrix(
        &mut self,
        source: usize,
        classes: usize,
        glyphs: &[u16],
        ligature: bool,
    ) -> Result<(usize, usize)> {
        let rows = usize::from(u16_at(self.data, source)?);
        if (ligature && glyphs.len() != 1) || (!ligature && glyphs.len() != rows) {
            return Err(fail("base coverage and anchor rows disagree"));
        }
        let cells = rows
            .checked_mul(classes)
            .ok_or_else(|| fail("anchor matrix overflow"))?;
        self.resolver.charge(cells)?;
        bytes(self.data, source, 2 + cells * 2)?;
        let target = self.w.reserve(2 + cells * 2)?;
        self.w.word(target, rows as u16);
        for i in 0..cells {
            self.anchor_link(
                source,
                source + 2 + i * 2,
                target,
                target + 2 + i * 2,
                false,
                glyphs[if ligature { 0 } else { i / classes }],
            )?;
        }
        Ok((target, rows))
    }
    fn mark(&mut self, root: usize, kind: u16) -> Result<()> {
        if u16_at(self.data, root)? != 1 {
            return Err(fail("unknown mark positioning format"));
        }
        let classes = usize::from(u16_at(self.data, root + 6)?);
        if classes == 0 {
            return Err(fail("mark positioning has no classes"));
        }
        self.w.reserve(12)?;
        self.w.word(0, 1);
        self.w.word(6, classes as u16);
        let mark = self.source(root, root + 8)?;
        let base = self.source(root, root + 10)?;
        let mark_coverage = self.source(root, root + 2)?;
        let base_coverage = self.source(root, root + 4)?;
        let mark_glyphs = self.resolver.coverage_glyphs(self.data, mark_coverage)?;
        let base_glyphs = self.resolver.coverage_glyphs(self.data, base_coverage)?;
        let (mark, marks) = self.mark_array(mark, classes, &mark_glyphs)?;
        self.w.link(8, 0, mark)?;
        let (base, bases) = if kind == 5 {
            let count = usize::from(u16_at(self.data, base)?);
            if count != base_glyphs.len() {
                return Err(fail("ligature coverage and component matrices disagree"));
            }
            self.resolver.charge(count)?;
            bytes(self.data, base, 2 + count * 2)?;
            let target = self.w.reserve(2 + count * 2)?;
            self.w.word(target, count as u16);
            for i in 0..count {
                let source = self.source(base, base + 2 + i * 2)?;
                let (attach, _) = self.matrix(source, classes, &base_glyphs[i..i + 1], true)?;
                self.w.link(target + 2 + i * 2, target, attach)?;
            }
            (target, count)
        } else {
            self.matrix(base, classes, &base_glyphs, false)?
        };
        self.w.link(10, 0, base)?;
        self.coverage(root, root + 2, 2, Some(marks))?;
        self.coverage(root, root + 4, 4, Some(bases))
    }
}
pub(super) fn freeze(
    data: &[u8],
    coordinates: &[ttf_parser::NormalizedCoordinate],
    resolver: &mut Resolver<'_, '_>,
) -> Result<FrozenGpos> {
    let mut contour_points = 0usize;
    let layout = layout_instance::freeze_gpos_with(data, coordinates, &mut |kind, source| {
        if matches!(kind, 7 | 8) {
            return Ok(None);
        }
        let (bytes, points) = freeze_program(data, kind, source, resolver)?;
        contour_points += points;
        Ok(Some(bytes))
    })?;
    Ok(FrozenGpos {
        layout,
        contour_point_anchors: contour_points,
    })
}

/// JSTF uses these same programs but forbids contextual dispatch. Its lookup
/// header/extension owners are rebuilt by its own writer, not by a fake GPOS.
pub(super) fn freeze_program(
    data: &[u8],
    kind: u16,
    source: usize,
    resolver: &mut Resolver<'_, '_>,
) -> Result<(Vec<u8>, usize)> {
    let mut p = Program {
        data,
        resolver,
        w: Writer::new(),
        anchors: BTreeMap::new(),
        contour_points: 0,
    };
    match kind {
        1 => p.single(source)?,
        2 => p.pair(source)?,
        3 => p.cursive(source)?,
        4..=6 => p.mark(source, kind)?,
        _ => {
            return Err(WellfriendError::UnsupportedFeature(
                "unhandled or contextual embedded position program".into(),
            ))
        }
    }
    Ok((p.w.out, p.contour_points))
}
