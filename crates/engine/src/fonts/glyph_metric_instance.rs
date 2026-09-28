//! Stage per-glyph static metrics against already-instanced outline geometry.
//! This is an internal part of the complete font transaction, not a font output.
use super::{
    glyph_metric_variations,
    variation_store::{bytes, round_i32, u16_at, u32_at},
};
use crate::{Result, WellfriendError};
use std::{collections::BTreeMap, sync::Arc};

type Tag = [u8; 4];
type Tables = BTreeMap<Tag, Arc<[u8]>>;
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("glyph metric instance: {message}"))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutlineKind {
    TrueType,
    Cff,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PhantomDeltas {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlyphGeometry {
    /// Source metric basis and bounds of the actual outline to serialize. For
    /// TrueType, the source header defines base phantom coordinates even when
    /// its bounds need repair; CFF uses the resolved default outline bounds.
    /// None means a structurally confirmed outline-less glyph.
    pub default: Option<ttf_parser::Rect>,
    pub instance: Option<ttf_parser::Rect>,
    /// gvar phantom deltas, not absolute phantom coordinates. Required for a
    /// TrueType glyph when the source has gvar, even for an empty glyph.
    pub phantom: Option<PhantomDeltas>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Metric {
    pub advance: u16,
    pub bearing: i16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RedundantMetricDifference {
    pub glyph: u16,
    pub vertical: bool,
    pub field: &'static str,
    pub declared: i32,
    pub derived: i32,
}
pub(crate) struct GlyphMetricStage {
    pub tables: BTreeMap<Tag, Vec<u8>>,
    pub horizontal: Vec<Metric>,
    pub vertical: Option<Vec<Metric>>,
    pub origins: Option<Vec<i16>>,
    /// Optional HVAR/VVAR redundant bearings can disagree with rounded outlines.
    /// These are explicit receipts, not a claim of exact metric preservation.
    /// The complete-font transaction must decide its fidelity policy before save.
    pub differences: Vec<RedundantMetricDifference>,
    pub global_changes: Vec<super::mvar_instance::MetricChange>,
    pub ignored_global_tags: Vec<Tag>,
    pub synchronized_hhea: bool,
}
fn signed(data: &[u8], at: usize) -> Result<i32> {
    Ok(i32::from(i16::from_be_bytes(
        bytes(data, at, 2)?.try_into().unwrap(),
    )))
}
fn i16_value(n: f64) -> Result<i16> {
    i16::try_from(round_i32(n)?).map_err(|_| fail("signed glyph metric overflow"))
}
fn metric(advance: f64, bearing: f64) -> Result<Metric> {
    Ok(Metric {
        advance: u16::try_from(round_i32(advance)?).map_err(|_| fail("glyph advance overflow"))?,
        bearing: i16_value(bearing)?,
    })
}
fn source_metrics(tables: &Tables, vertical: bool, count: usize) -> Result<Option<Vec<Metric>>> {
    let (header, metrics) = if vertical {
        (*b"vhea", *b"vmtx")
    } else {
        (*b"hhea", *b"hmtx")
    };
    match (tables.get(&header), tables.get(&metrics)) {
        (None, None) if vertical => Ok(None),
        (Some(head), Some(data)) => {
            bytes(head, 0, 36)?;
            if !matches!(u32_at(head, 0)?, 0x10000 | 0x11000)
                || (!vertical && u32_at(head, 0)? != 0x10000)
            {
                return Err(fail("unsupported metric header version"));
            }
            if u16_at(head, 32)? != 0 {
                return Err(fail("unsupported metric data format"));
            }
            let long = usize::from(u16_at(head, 34)?);
            if long == 0 || long > count {
                return Err(fail("invalid long-metric count"));
            }
            bytes(data, 0, long * 4 + (count - long) * 2)?;
            let mut result = Vec::with_capacity(count);
            for gid in 0..count {
                if gid % 256 == 0 {
                    crate::cancel::check_current_cancel("source glyph metrics")?;
                }
                let advance = u16_at(data, gid.min(long - 1) * 4)?;
                let at = if gid < long {
                    gid * 4 + 2
                } else {
                    long * 4 + (gid - long) * 2
                };
                result.push(Metric {
                    advance,
                    bearing: signed(data, at)? as i16,
                });
            }
            Ok(Some(result))
        }
        _ => Err(fail("missing paired metric/header tables")),
    }
}
fn source_origins(tables: &Tables, count: usize) -> Result<Option<Vec<i16>>> {
    let Some(data) = tables.get(b"VORG") else {
        return Ok(None);
    };
    if u32_at(data, 0)? != 0x10000 {
        return Err(fail("unsupported VORG version"));
    }
    let default = signed(data, 4)? as i16;
    let records = usize::from(u16_at(data, 6)?);
    let mut result = vec![default; count];
    let mut previous = None;
    for (i, record) in bytes(data, 8, records * 4)?.chunks_exact(4).enumerate() {
        if i % 256 == 0 {
            crate::cancel::check_current_cancel("source vertical origins")?;
        }
        let gid = usize::from(u16_at(record, 0)?);
        if gid >= count || previous.is_some_and(|last| last >= gid) {
            return Err(fail(
                "VORG glyph identities must be unique, ordered and in range",
            ));
        }
        result[gid] = signed(record, 2)? as i16;
        previous = Some(gid);
    }
    Ok(Some(result))
}
fn dimensions(bounds: Option<ttf_parser::Rect>, vertical: bool) -> (f64, f64) {
    bounds.map_or((0., 0.), |r| {
        if vertical {
            (f64::from(r.y_max), f64::from(r.y_max) - f64::from(r.y_min))
        } else {
            (f64::from(r.x_min), f64::from(r.x_max) - f64::from(r.x_min))
        }
    })
}
fn add_difference(
    out: &mut Vec<RedundantMetricDifference>,
    gid: usize,
    vertical: bool,
    field: &'static str,
    declared: f64,
    derived: f64,
) -> Result<()> {
    let declared = round_i32(declared)?;
    let derived = round_i32(derived)?;
    if declared != derived {
        out.push(RedundantMetricDifference {
            glyph: gid as u16,
            vertical,
            field,
            declared,
            derived,
        });
    }
    Ok(())
}

/// Geometry is in stable source GID order, produced by the same font transaction.
/// This function never guesses phantom deltas or treats a failed outline as blank.
pub(crate) fn freeze(
    tables: &Tables,
    coordinates: &[ttf_parser::NormalizedCoordinate],
    kind: OutlineKind,
    geometry: &[GlyphGeometry],
) -> Result<GlyphMetricStage> {
    // Apply font-wide fields first, so rebuilding hhea/vhea extrema cannot
    // overwrite selected-instance ascenders or caret metrics. Nothing publishes
    // if the later per-glyph pass fails.
    let global = super::mvar_instance::freeze(tables, coordinates)?;
    let mut working = tables.clone();
    // Only these two global targets are read by the per-glyph writer. Other
    // changed tables move into the final stage without another full-table copy.
    for (tag, data) in &global.tables {
        if matches!(tag, b"hhea" | b"vhea") {
            if data.len() > 1024 * 1024 {
                return Err(WellfriendError::ResourceLimit(
                    "metric header exceeds 1 MiB".into(),
                ));
            }
            working.insert(*tag, Arc::from(data.as_slice()));
        }
    }
    let mut stage = freeze_glyphs(&working, coordinates, kind, geometry)?;
    for (tag, data) in global.tables {
        stage.tables.entry(tag).or_insert(data);
    }
    stage.global_changes = global.changes;
    stage.ignored_global_tags = global.ignored_tags;
    stage.synchronized_hhea = global.synchronized_hhea;
    crate::cancel::check_current_cancel("combined metric stage publication")?;
    Ok(stage)
}

fn freeze_glyphs(
    tables: &Tables,
    coordinates: &[ttf_parser::NormalizedCoordinate],
    kind: OutlineKind,
    geometry: &[GlyphGeometry],
) -> Result<GlyphMetricStage> {
    crate::cancel::check_current_cancel("glyph metric instancing")?;
    let maxp = tables.get(b"maxp").ok_or_else(|| fail("missing maxp"))?;
    let count = usize::from(u16_at(maxp, 4)?);
    if count == 0 || geometry.len() != count {
        return Err(fail("geometry does not cover the source glyph identities"));
    }
    let horizontal =
        source_metrics(tables, false, count)?.ok_or_else(|| fail("missing horizontal metrics"))?;
    let vertical = source_metrics(tables, true, count)?;
    let gvar = kind == OutlineKind::TrueType && tables.contains_key(b"gvar");
    let origins = if kind == OutlineKind::Cff {
        source_origins(tables, count)?
    } else {
        None
    };
    if vertical.is_none() && (tables.contains_key(b"VVAR") || origins.is_some()) {
        return Err(fail(
            "vertical variation/origin data requires vertical metrics",
        ));
    }
    let hvar = tables
        .get(b"HVAR")
        .map(|data| {
            glyph_metric_variations::resolve(Arc::clone(data), false, coordinates, count as u16)
        })
        .transpose()?;
    let vvar = tables
        .get(b"VVAR")
        .map(|data| {
            glyph_metric_variations::resolve(Arc::clone(data), true, coordinates, count as u16)
        })
        .transpose()?;
    let mut result = GlyphMetricStage {
        tables: BTreeMap::new(),
        horizontal: Vec::with_capacity(count),
        vertical: vertical.as_ref().map(|_| Vec::with_capacity(count)),
        origins: (kind == OutlineKind::Cff && vertical.is_some())
            .then(|| Vec::with_capacity(count)),
        differences: Vec::new(),
        global_changes: Vec::new(),
        ignored_global_tags: Vec::new(),
        synchronized_hhea: false,
    };
    for (gid, geo) in geometry.iter().enumerate() {
        crate::cancel::check_current_cancel("instanced glyph metric row")?;
        for r in [geo.default, geo.instance].into_iter().flatten() {
            if r.x_min > r.x_max || r.y_min > r.y_max {
                return Err(fail("unordered glyph bounds"));
            }
        }
        let phantom = if gvar {
            geo.phantom
                .ok_or_else(|| fail("gvar glyph lacks resolved phantom deltas"))?
        } else {
            PhantomDeltas::default()
        };
        if ![phantom.left, phantom.right, phantom.top, phantom.bottom]
            .iter()
            .all(|n| n.is_finite())
        {
            return Err(fail("non-finite phantom delta"));
        }
        let old = horizontal[gid];
        let h = hvar.as_ref().map(|rows| rows[gid]);
        let (old_min, old_width) = dimensions(geo.default, false);
        let (new_min, new_width) = dimensions(geo.instance, false);
        let advance =
            f64::from(old.advance) + h.map_or(phantom.right - phantom.left, |d| d.advance);
        let bearing = if let Some(delta) = h.and_then(|d| d.leading) {
            f64::from(old.bearing) + delta
        } else if kind == OutlineKind::Cff {
            new_min
        } else {
            f64::from(old.bearing) + new_min - old_min - phantom.left
        };
        let new = metric(advance, bearing)?;
        if let Some(delta) = h {
            if gvar {
                add_difference(
                    &mut result.differences,
                    gid,
                    false,
                    "advance",
                    advance,
                    f64::from(old.advance) + phantom.right - phantom.left,
                )?;
            }
            if delta.leading.is_some() && (gvar || kind == OutlineKind::Cff) {
                let derived = if kind == OutlineKind::Cff {
                    new_min
                } else {
                    f64::from(old.bearing) + new_min - old_min - phantom.left
                };
                add_difference(
                    &mut result.differences,
                    gid,
                    false,
                    "leading_bearing",
                    bearing,
                    derived,
                )?;
            }
        }
        if let Some(delta) = h.and_then(|d| d.trailing) {
            add_difference(
                &mut result.differences,
                gid,
                false,
                "trailing_bearing",
                f64::from(old.advance) - f64::from(old.bearing) - old_width + delta,
                f64::from(new.advance) - f64::from(new.bearing) - new_width,
            )?;
        }
        result.horizontal.push(new);
        if let Some(old_rows) = &vertical {
            let old = old_rows[gid];
            let v = vvar.as_ref().map(|rows| rows[gid]);
            let (old_top, old_height) = dimensions(geo.default, true);
            let (new_top, new_height) = dimensions(geo.instance, true);
            let advance =
                f64::from(old.advance) + v.map_or(phantom.top - phantom.bottom, |d| d.advance);
            let declared_bearing = v
                .and_then(|d| d.leading)
                .map(|delta| f64::from(old.bearing) + delta);
            let base_origin = origins
                .as_ref()
                .map_or(old_top + f64::from(old.bearing), |rows| {
                    f64::from(rows[gid])
                });
            let (bearing, origin) = if kind == OutlineKind::Cff
                && (origins.is_some() || v.and_then(|d| d.origin).is_some())
            {
                let origin = base_origin + v.and_then(|d| d.origin).unwrap_or(0.);
                (origin - new_top, origin)
            } else {
                let bearing = declared_bearing.unwrap_or_else(|| {
                    if kind == OutlineKind::Cff {
                        f64::from(old.bearing)
                    } else {
                        f64::from(old.bearing) + old_top - new_top + phantom.top
                    }
                });
                (bearing, new_top + bearing)
            };
            let new = metric(advance, bearing)?;
            if gvar && v.is_some() {
                add_difference(
                    &mut result.differences,
                    gid,
                    true,
                    "advance",
                    advance,
                    f64::from(old.advance) + phantom.top - phantom.bottom,
                )?;
                if let Some(declared) = declared_bearing {
                    add_difference(
                        &mut result.differences,
                        gid,
                        true,
                        "leading_bearing",
                        declared,
                        f64::from(old.bearing) + old_top - new_top + phantom.top,
                    )?;
                }
            }
            if let Some(declared) = declared_bearing {
                add_difference(
                    &mut result.differences,
                    gid,
                    true,
                    "leading_bearing",
                    declared,
                    f64::from(new.bearing),
                )?;
            }
            if let Some(delta) = v.and_then(|d| d.trailing) {
                add_difference(
                    &mut result.differences,
                    gid,
                    true,
                    "trailing_bearing",
                    f64::from(old.advance) - f64::from(old.bearing) - old_height + delta,
                    f64::from(new.advance) - f64::from(new.bearing) - new_height,
                )?;
            }
            result.vertical.as_mut().unwrap().push(new);
            if let Some(origins) = result.origins.as_mut() {
                origins.push(i16_value(origin)?);
            }
        }
    }
    write_metrics(
        tables,
        &mut result.tables,
        false,
        &result.horizontal,
        geometry,
    )?;
    if let Some(rows) = &result.vertical {
        write_metrics(tables, &mut result.tables, true, rows, geometry)?;
    }
    if let Some(origins) = &result.origins {
        result.tables.insert(*b"VORG", write_origins(origins)?);
    }
    crate::cancel::check_current_cancel("glyph metric stage publication")?;
    Ok(result)
}

fn write_metrics(
    tables: &Tables,
    output: &mut BTreeMap<Tag, Vec<u8>>,
    vertical: bool,
    rows: &[Metric],
    geometry: &[GlyphGeometry],
) -> Result<()> {
    let (header_tag, metrics_tag) = if vertical {
        (*b"vhea", *b"vmtx")
    } else {
        (*b"hhea", *b"hmtx")
    };
    let source = tables
        .get(&header_tag)
        .ok_or_else(|| fail("missing metric header"))?;
    if source.len() > 1024 * 1024 {
        return Err(WellfriendError::ResourceLimit(
            "metric header exceeds 1 MiB".into(),
        ));
    }
    let mut header = source.to_vec();
    // Compress only the identical-advance suffix; preserve every GID and bearing.
    let mut long = rows.len();
    while long > 1 && rows[long - 2].advance == rows[long - 1].advance {
        long -= 1;
    }
    let mut out = Vec::with_capacity(long * 4 + (rows.len() - long) * 2);
    let mut advance_max = 0;
    let mut leading_min: Option<i16> = None;
    let mut trailing_min: Option<i16> = None;
    let mut extent_max: Option<i16> = None;
    for (gid, row) in rows.iter().enumerate() {
        if gid % 256 == 0 {
            crate::cancel::check_current_cancel("metric table serialization")?;
        }
        if gid < long {
            out.extend_from_slice(&row.advance.to_be_bytes());
        }
        out.extend_from_slice(&row.bearing.to_be_bytes());
        advance_max = advance_max.max(row.advance);
        if geometry[gid].instance.is_some() {
            let (_, span) = dimensions(geometry[gid].instance, vertical);
            let extent = i16_value(f64::from(row.bearing) + span)?;
            let trailing = i16_value(f64::from(row.advance) - f64::from(extent))?;
            leading_min = Some(leading_min.map_or(row.bearing, |n| n.min(row.bearing)));
            trailing_min = Some(trailing_min.map_or(trailing, |n| n.min(trailing)));
            extent_max = Some(extent_max.map_or(extent, |n| n.max(extent)));
        }
    }
    for (at, value) in [
        (10, advance_max),
        (12, leading_min.unwrap_or(0) as u16),
        (14, trailing_min.unwrap_or(0) as u16),
        (16, extent_max.unwrap_or(0) as u16),
        (34, long as u16),
    ] {
        header[at..at + 2].copy_from_slice(&value.to_be_bytes());
    }
    output.insert(header_tag, header);
    output.insert(metrics_tag, out);
    Ok(())
}
fn write_origins(origins: &[i16]) -> Result<Vec<u8>> {
    let mut counts = BTreeMap::<i16, usize>::new();
    for (i, origin) in origins.iter().enumerate() {
        if i % 256 == 0 {
            crate::cancel::check_current_cancel("vertical origin grouping")?;
        }
        *counts.entry(*origin).or_default() += 1;
    }
    let default = counts
        .iter()
        .max_by_key(|(origin, count)| (**count, std::cmp::Reverse(**origin)))
        .map(|(origin, _)| *origin)
        .ok_or_else(|| fail("empty origin list"))?;
    let records = origins.iter().filter(|origin| **origin != default).count();
    let mut out = vec![0, 1, 0, 0];
    out.extend_from_slice(&default.to_be_bytes());
    out.extend_from_slice(&(records as u16).to_be_bytes());
    for (gid, origin) in origins.iter().enumerate() {
        if gid % 256 == 0 {
            crate::cancel::check_current_cancel("vertical origin serialization")?;
        }
        if *origin != default {
            out.extend_from_slice(&(gid as u16).to_be_bytes());
            out.extend_from_slice(&origin.to_be_bytes());
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "glyph_metric_instance_tests.rs"]
mod tests;
