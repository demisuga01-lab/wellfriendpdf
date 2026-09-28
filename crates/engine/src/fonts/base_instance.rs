//! Resolve every baseline/minimum/maximum owner against BASE's own store.
//! Glyph/point references survive unchanged; this does not rewrite outlines.
use super::{
    position_instance::{fail, integer, relative, signed, Device, Resolver, Writer, LIMIT},
    variation_store::{bytes, u16_at, u32_at, ItemVariationStore},
};
use crate::{Result, WellfriendError};
use std::{collections::BTreeMap, sync::Arc};

pub(crate) struct FrozenBase {
    pub bytes: Vec<u8>,
    pub resolved_adjustments: usize,
    pub retained_device_hints: usize,
    pub evaluated_delta_cells: usize,
    pub distinct_variation_indices: usize,
    pub contour_point_references: usize,
    pub checked_point_references: usize,
    pub unchecked_point_references: usize,
    pub retired_store: bool,
}
struct Program<'a, 'r, 's, 't> {
    data: &'a [u8],
    resolver: &'r mut Resolver<'s, 't>,
    w: Writer,
    coordinates: BTreeMap<usize, Vec<usize>>,
    points: usize,
}
impl Program<'_, '_, '_, '_> {
    fn child(&self, base: usize, field: usize) -> Result<usize> {
        relative(self.data, base, usize::from(u16_at(self.data, field)?))
    }
    fn tags(&mut self, at: usize, count: usize, stride: usize) -> Result<()> {
        self.resolver.charge(count)?;
        bytes(self.data, at, count * stride)?;
        let mut previous = None;
        for i in 0..count {
            crate::cancel::check_current_cancel("font baseline tags")?;
            let tag = u32_at(self.data, at + i * stride)?;
            if previous.is_some_and(|last| last >= tag) {
                return Err(fail("unordered or duplicate BASE tags"));
            }
            previous = Some(tag);
        }
        Ok(())
    }
    fn coordinate(&mut self, source: usize, owner: usize) -> Result<usize> {
        self.resolver.charge(1)?;
        let format = u16_at(self.data, source)?;
        if format == 2 {
            self.resolver.point(
                u16_at(self.data, source + 4)?,
                u16_at(self.data, source + 6)?,
            )?;
        }
        if let Some(target) = self.coordinates.get(&source).and_then(|copies| {
            copies
                .get(copies.partition_point(|at| *at <= owner))
                .copied()
                .filter(|at| *at - owner <= 65535)
        }) {
            return Ok(target);
        }
        let length = match format {
            1 => 4,
            2 => 8,
            3 => 6,
            _ => return Err(fail("unknown BaseCoord format")),
        };
        let target = self.w.copy(bytes(self.data, source, length)?)?;
        if format == 2 {
            self.points += 1;
        }
        if format == 3 {
            let offset = u16_at(self.data, source + 4)?;
            self.w.word(target + 4, 0);
            match self.resolver.device(self.data, source, offset)? {
                Device::None => {}
                Device::Delta(delta) => {
                    self.w.word(
                        target + 2,
                        integer(f64::from(signed(self.data, source + 2)?) + delta)? as u16,
                    );
                }
                Device::Hint(hint) => {
                    let at = relative(self.data, source, usize::from(offset))?;
                    let copied = self.w.leaf(self.data, at, hint.len(), target)?;
                    self.w.link(target + 4, target, copied)?;
                }
            }
        }
        self.coordinates.entry(source).or_default().push(target);
        Ok(target)
    }
    fn coordinate_link(
        &mut self,
        src: usize,
        src_field: usize,
        dst: usize,
        dst_field: usize,
        optional: bool,
    ) -> Result<()> {
        let offset = u16_at(self.data, src_field)?;
        if offset == 0 && optional {
            return Ok(());
        }
        let source = relative(self.data, src, usize::from(offset))?;
        let target = self.coordinate(source, dst)?;
        self.w.link(dst_field, dst, target)
    }
    fn values(&mut self, source: usize, tags: usize) -> Result<usize> {
        let default = usize::from(u16_at(self.data, source)?);
        let count = usize::from(u16_at(self.data, source + 2)?);
        self.resolver.charge(count + 1)?;
        if count != tags || default >= count {
            return Err(fail("BASE default/count disagrees with tag list"));
        }
        bytes(self.data, source, 4 + count * 2)?;
        let target = self.w.reserve(4 + count * 2)?;
        self.w.word(target, default as u16);
        self.w.word(target + 2, count as u16);
        for i in 0..count {
            self.coordinate_link(
                source,
                source + 4 + i * 2,
                target,
                target + 4 + i * 2,
                false,
            )?;
        }
        Ok(target)
    }
    fn minmax(&mut self, source: usize) -> Result<usize> {
        let count = usize::from(u16_at(self.data, source + 4)?);
        self.resolver.charge(1)?;
        self.tags(source + 6, count, 8)?;
        let target = self.w.reserve(6 + count * 8)?;
        self.w.word(target + 4, count as u16);
        for field in [0, 2] {
            self.coordinate_link(source, source + field, target, target + field, true)?;
        }
        for i in 0..count {
            let src = source + 6 + i * 8;
            let dst = target + 6 + i * 8;
            self.w.dword(dst, u32_at(self.data, src)?);
            // FeatMinMax offsets are relative to MinMax, not their own record.
            for field in [4, 6] {
                self.coordinate_link(source, src + field, target, dst + field, true)?;
            }
        }
        Ok(target)
    }
    fn script(&mut self, source: usize, tags: usize) -> Result<usize> {
        let count = usize::from(u16_at(self.data, source + 4)?);
        self.resolver.charge(1)?;
        self.tags(source + 6, count, 6)?;
        let target = self.w.reserve(6 + count * 6)?;
        self.w.word(target + 4, count as u16);
        if u16_at(self.data, source)? != 0 {
            let at = self.child(source, source)?;
            let values = self.values(at, tags)?;
            self.w.link(target, target, values)?;
        }
        if u16_at(self.data, source + 2)? != 0 {
            let at = self.child(source, source + 2)?;
            let minmax = self.minmax(at)?;
            self.w.link(target + 2, target, minmax)?;
        }
        for i in 0..count {
            let src = source + 6 + i * 6;
            let dst = target + 6 + i * 6;
            self.w.dword(dst, u32_at(self.data, src)?);
            let at = self.child(source, src + 4)?;
            let minmax = self.minmax(at)?;
            self.w.link(dst + 4, target, minmax)?;
        }
        Ok(target)
    }
    fn axis(&mut self, source: usize) -> Result<usize> {
        self.resolver.charge(1)?;
        bytes(self.data, source, 4)?;
        let target = self.w.reserve(4)?;
        let tags = if u16_at(self.data, source)? != 0 {
            let at = self.child(source, source)?;
            let count = usize::from(u16_at(self.data, at)?);
            self.tags(at + 2, count, 4)?;
            let copied = self.w.leaf(self.data, at, 2 + count * 4, target)?;
            self.w.link(target, target, copied)?;
            count
        } else {
            0
        };
        let list = self.child(source, source + 2)?;
        let count = usize::from(u16_at(self.data, list)?);
        self.tags(list + 2, count, 6)?;
        let output = self.w.reserve(2 + count * 6)?;
        self.w.word(output, count as u16);
        self.w.link(target + 2, target, output)?;
        for i in 0..count {
            let src = list + 2 + i * 6;
            let dst = output + 2 + i * 6;
            self.w.dword(dst, u32_at(self.data, src)?);
            let script = self.child(list, src + 4)?;
            let script = self.script(script, tags)?;
            self.w.link(dst + 4, output, script)?;
        }
        Ok(target)
    }
}
#[cfg(test)]
pub(crate) fn freeze(
    source: Arc<[u8]>,
    coordinates: &[ttf_parser::NormalizedCoordinate],
) -> Result<FrozenBase> {
    freeze_with_points(source, coordinates, None)
}
pub(super) fn freeze_with_points(
    source: Arc<[u8]>,
    coordinates: &[ttf_parser::NormalizedCoordinate],
    point_counts: Option<Arc<[u16]>>,
) -> Result<FrozenBase> {
    crate::cancel::check_current_cancel("BASE instance preparation")?;
    if source.len() > LIMIT {
        return Err(WellfriendError::ResourceLimit("BASE exceeds 64 MiB".into()));
    }
    let size = match u32_at(&source, 0)? {
        0x10000 => 8,
        0x10001 => 12,
        _ => return Err(fail("unsupported BASE version")),
    };
    bytes(&source, 0, size)?;
    let store_at = if size == 12 {
        u32_at(&source, 8)? as usize
    } else {
        0
    };
    let store = if store_at != 0 {
        if store_at < size {
            return Err(fail("BASE store overlaps header"));
        }
        let store = ItemVariationStore::parse(Arc::clone(&source), store_at..source.len())?;
        store.require_short_deltas()?;
        Some(store)
    } else {
        None
    };
    let prepared = store
        .as_ref()
        .map(|store| store.prepare(coordinates))
        .transpose()?;
    let mut resolver = Resolver::new(prepared.as_ref());
    resolver.point_counts = point_counts;
    let mut p = Program {
        data: &source,
        resolver: &mut resolver,
        w: Writer::new(),
        coordinates: BTreeMap::new(),
        points: 0,
    };
    p.w.reserve(8)?;
    p.w.dword(0, 0x10000);
    for field in [4, 6] {
        let at = usize::from(u16_at(&source, field)?);
        if at == 0 {
            continue;
        }
        if at < size {
            return Err(fail("BASE axis overlaps header"));
        }
        let axis = p.axis(at)?;
        p.w.link(field, 0, axis)?;
    }
    let points = p.points;
    let out = p.w.out;
    crate::cancel::check_current_cancel("BASE stage publication")?;
    Ok(FrozenBase {
        bytes: out,
        resolved_adjustments: resolver.resolved,
        retained_device_hints: resolver.hints,
        evaluated_delta_cells: resolver.evaluated_delta_cells,
        distinct_variation_indices: resolver.distinct_variation_indices(),
        contour_point_references: points,
        checked_point_references: resolver.checked_point_references,
        unchecked_point_references: resolver.unchecked_point_references,
        retired_store: store_at != 0,
    })
}
