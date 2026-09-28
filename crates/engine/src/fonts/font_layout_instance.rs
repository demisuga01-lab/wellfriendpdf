//! One source-bound layout stage. This is deliberately not a publishable static
//! font: the source-bound TrueType caller supplies exact outline point domains;
//! the remaining sfnt stages must join before a public instance can be returned.
use super::{
    base_instance, gdef_instance, gpos_instance, jstf_instance, layout_instance,
    position_instance::{fail, relative, Resolver},
    variation_store::{bytes, u16_at, u32_at, ItemVariationStore},
};
use crate::Result;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) struct LayoutStage {
    /// Replacements only, applied with the other font stages at final publication.
    pub tables: BTreeMap<[u8; 4], Vec<u8>>,
    pub selected_features: BTreeMap<[u8; 4], Option<u32>>,
    pub substituted_features: BTreeMap<[u8; 4], Vec<u16>>,
    pub resolved_adjustments: usize,
    pub retained_device_hints: usize,
    pub evaluated_delta_cells: usize,
    pub distinct_variation_indices: usize,
    pub contour_point_references: usize,
    /// Owner occurrences checked against exact generated contour point domains.
    /// Unlike the relocation count above, this includes shared-record uses.
    pub checked_point_references: usize,
    /// Isolated/CFF stages with no point authority must not claim validation.
    pub unchecked_point_references: usize,
    pub retained_gdef_store: bool,
    pub opaque_layout_sources: Vec<[u8; 4]>,
    pub retired_variation_stores: Vec<[u8; 4]>,
}
fn lookup_count(data: Option<&Arc<[u8]>>) -> Result<usize> {
    let Some(data) = data else {
        return Ok(0);
    };
    let size = match u32_at(data, 0)? {
        0x10000 => 10,
        0x10001 => 14,
        _ => return Err(fail("unsupported layout version for JSTF references")),
    };
    bytes(data, 0, size)?;
    let at = relative(data, 0, usize::from(u16_at(data, 8)?))?;
    if at < size {
        return Err(fail("JSTF referenced LookupList overlaps header"));
    }
    let count = usize::from(u16_at(data, at)?);
    bytes(data, at, 2 + count * 2)?;
    Ok(count)
}
#[cfg(test)]
pub(crate) fn freeze(
    tables: &BTreeMap<[u8; 4], Arc<[u8]>>,
    coordinates: &[ttf_parser::NormalizedCoordinate],
) -> Result<LayoutStage> {
    freeze_with_points(tables, coordinates, None)
}
pub(crate) fn freeze_with_points(
    tables: &BTreeMap<[u8; 4], Arc<[u8]>>,
    coordinates: &[ttf_parser::NormalizedCoordinate],
    point_counts: Option<Arc<[u16]>>,
) -> Result<LayoutStage> {
    crate::cancel::check_current_cancel("font layout preparation")?;
    if let Some(points) = &point_counts {
        if points.len() > usize::from(u16::MAX) {
            return Err(fail("outline point authority exceeds maxp glyph domain"));
        }
        if let Some(maxp) = tables.get(b"maxp") {
            if usize::from(u16_at(maxp, 4)?) != points.len() {
                return Err(fail("outline point authority disagrees with maxp"));
            }
        }
    }
    let store = if let Some(data) = tables.get(b"GDEF") {
        let (_, offset) = gdef_instance::header(data)?;
        if offset != 0 {
            let store = ItemVariationStore::parse(Arc::clone(data), offset..data.len())?;
            store.require_short_deltas()?;
            Some(store)
        } else {
            None
        }
    } else {
        None
    };
    let prepared = store
        .as_ref()
        .map(|store| store.prepare(coordinates))
        .transpose()?;
    let mut resolver = Resolver::new(prepared.as_ref());
    resolver.point_counts = point_counts.clone();
    let mut stage = LayoutStage {
        tables: BTreeMap::new(),
        selected_features: BTreeMap::new(),
        substituted_features: BTreeMap::new(),
        resolved_adjustments: 0,
        retained_device_hints: 0,
        evaluated_delta_cells: 0,
        distinct_variation_indices: 0,
        contour_point_references: 0,
        checked_point_references: 0,
        unchecked_point_references: 0,
        retained_gdef_store: false,
        opaque_layout_sources: Vec::new(),
        retired_variation_stores: Vec::new(),
    };
    for tag in [*b"GSUB", *b"GPOS"] {
        let Some(data) = tables.get(&tag) else {
            continue;
        };
        let frozen = if tag == *b"GPOS" {
            let gpos = gpos_instance::freeze(data, coordinates, &mut resolver)?;
            stage.contour_point_references += gpos.contour_point_anchors;
            gpos.layout
        } else {
            layout_instance::freeze(data, tag, coordinates)?
        };
        stage.selected_features.insert(tag, frozen.selected_record);
        stage
            .substituted_features
            .insert(tag, frozen.substituted_features);
        if frozen.retained_source_block {
            stage.opaque_layout_sources.push(tag);
        }
        stage.tables.insert(tag, frozen.bytes);
    }
    if let Some(data) = tables.get(b"JSTF") {
        let counts = [
            lookup_count(tables.get(b"GSUB"))?,
            lookup_count(tables.get(b"GPOS"))?,
        ];
        let glyphs = tables
            .get(b"maxp")
            .map(|data| u16_at(data, 4).map(usize::from))
            .transpose()?
            .unwrap_or_else(|| point_counts.as_ref().map_or(65536, |points| points.len()));
        let jstf = jstf_instance::freeze(data, &mut resolver, counts, glyphs)?;
        stage.contour_point_references += jstf.contour_point_references;
        stage.tables.insert(*b"JSTF", jstf.bytes);
    }
    // GPOS and JSTF have both completed, including all unreachable-by-feature
    // lookups. Only now retire their shared source store when rebuilding GDEF.
    if let Some(data) = tables.get(b"GDEF") {
        let gdef = gdef_instance::freeze(data, &mut resolver, false)?;
        stage.retained_gdef_store = gdef.retained_store;
        if store.is_some() {
            stage.retired_variation_stores.push(*b"GDEF");
        }
        stage.contour_point_references += gdef.contour_point_references;
        stage.tables.insert(*b"GDEF", gdef.bytes);
    }
    stage.resolved_adjustments = resolver.resolved;
    stage.retained_device_hints = resolver.hints;
    stage.evaluated_delta_cells = resolver.evaluated_delta_cells;
    stage.distinct_variation_indices = resolver.distinct_variation_indices();
    stage.checked_point_references = resolver.checked_point_references;
    stage.unchecked_point_references = resolver.unchecked_point_references;
    if let Some(data) = tables.get(b"BASE") {
        let base = base_instance::freeze_with_points(Arc::clone(data), coordinates, point_counts)?;
        stage.resolved_adjustments += base.resolved_adjustments;
        stage.retained_device_hints += base.retained_device_hints;
        stage.evaluated_delta_cells += base.evaluated_delta_cells;
        stage.distinct_variation_indices += base.distinct_variation_indices;
        stage.contour_point_references += base.contour_point_references;
        stage.checked_point_references += base.checked_point_references;
        stage.unchecked_point_references += base.unchecked_point_references;
        if base.retired_store {
            stage.retired_variation_stores.push(*b"BASE");
        }
        stage.tables.insert(*b"BASE", base.bytes);
    }
    crate::cancel::check_current_cancel("font layout stage publication")?;
    Ok(stage)
}

#[cfg(test)]
#[path = "font_layout_instance_tests.rs"]
mod tests;
