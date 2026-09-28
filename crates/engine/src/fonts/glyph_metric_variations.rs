//! HVAR/VVAR delta resolution, retaining source ownership and glyph identities.
use super::variation_store::{bytes, u32_at, DeltaSetIndexMap, ItemVariationStore};
use crate::{Result, WellfriendError};
use std::sync::Arc;

fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("glyph metric variations: {message}"))
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Deltas {
    pub advance: f64,
    pub leading: Option<f64>,
    pub trailing: Option<f64>,
    pub origin: Option<f64>,
}

/// Resolve every requested glyph in one preparation pass. Map aliases share the
/// parsed view; the store owns the same immutable bytes, not an unrelated slice.
pub(crate) fn resolve(
    source: Arc<[u8]>,
    vertical: bool,
    coordinates: &[ttf_parser::NormalizedCoordinate],
    glyphs: u16,
) -> Result<Vec<Deltas>> {
    crate::cancel::check_current_cancel("glyph metric variation preparation")?;
    let header = if vertical { 24 } else { 20 };
    bytes(&source, 0, header)?;
    if u32_at(&source, 0)? != 0x00010000 {
        return Err(fail("unsupported HVAR/VVAR version"));
    }
    if source.len() > 64 * 1024 * 1024 {
        return Err(WellfriendError::ResourceLimit(
            "glyph metric variation table exceeds 64 MiB".into(),
        ));
    }
    let store_at = u32_at(&source, 4)? as usize;
    if store_at < header {
        return Err(fail("variation store overlaps the table header"));
    }
    let store = ItemVariationStore::parse(Arc::clone(&source), store_at..source.len())?;
    store.require_short_deltas()?;
    let instance = store.prepare(coordinates)?;
    let mut maps = std::collections::BTreeMap::new();
    let mut offsets = [0usize; 4];
    for (i, slot) in offsets
        .iter_mut()
        .enumerate()
        .take(if vertical { 4 } else { 3 })
    {
        *slot = u32_at(&source, 8 + i * 4)? as usize;
        if *slot == 0 {
            continue;
        }
        if *slot < header {
            return Err(fail("index map overlaps the table header"));
        }
        // HVAR/VVAR use format 0. Format 1 remains available in the shared core
        // for parent tables such as COLR that allow it.
        if bytes(&source, *slot, 1)?[0] != 0 {
            return Err(fail("HVAR/VVAR index map requires format 0"));
        }
        if let std::collections::btree_map::Entry::Vacant(e) = maps.entry(*slot) {
            e.insert(DeltaSetIndexMap::parse(&source[*slot..])?);
        }
    }
    let lookup = |offset: usize, gid: u16| -> Result<f64> {
        let (outer, inner) = if offset == 0 {
            (0, u32::from(gid))
        } else {
            maps.get(&offset)
                .ok_or_else(|| fail("unresolved index map"))?
                .get(u32::from(gid))?
        };
        instance.delta(outer, inner)
    };
    let mut result = Vec::with_capacity(usize::from(glyphs));
    for gid in 0..glyphs {
        crate::cancel::check_current_cancel("glyph metric variation rows")?;
        let optional = |index: usize| -> Result<Option<f64>> {
            if offsets[index] == 0 {
                Ok(None)
            } else {
                lookup(offsets[index], gid).map(Some)
            }
        };
        result.push(Deltas {
            advance: lookup(offsets[0], gid)?,
            leading: optional(1)?,
            trailing: optional(2)?,
            origin: optional(3)?,
        });
    }
    Ok(result)
}
