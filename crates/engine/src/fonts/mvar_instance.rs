//! Stage all registered MVAR font-wide metrics at a normalized instance. The
//! caller owns the complete font transaction and removes MVAR only at commit.
use super::variation_store::{bytes, round_i32, u16_at, ItemVariationStore};
use crate::{Result, WellfriendError};
use std::{collections::BTreeMap, sync::Arc};
type Tag = [u8; 4];
type Tables = BTreeMap<Tag, Arc<[u8]>>;
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("font metric instance: {message}"))
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetricChange {
    pub metric: Tag,
    pub table: Tag,
    pub offset: usize,
    pub before: i32,
    pub after: i32,
}
#[derive(Default)]
pub(crate) struct MetricStage {
    pub tables: BTreeMap<Tag, Vec<u8>>,
    pub changes: Vec<MetricChange>,
    /// Unrecognized/private tags have no standardized target to modify.
    pub ignored_tags: Vec<Tag>,
    pub synchronized_hhea: bool,
}
#[derive(Clone, Copy)]
struct Field {
    table: Tag,
    offset: usize,
    unsigned: bool,
}
fn field(tag: Tag) -> Option<Field> {
    let (table, offset, unsigned) = match &tag {
        b"hasc" => (*b"OS/2", 68, false),
        b"hdsc" => (*b"OS/2", 70, false),
        b"hlgp" => (*b"OS/2", 72, false),
        b"hcla" => (*b"OS/2", 74, true),
        b"hcld" => (*b"OS/2", 76, true),
        b"hcrs" => (*b"hhea", 18, false),
        b"hcrn" => (*b"hhea", 20, false),
        b"hcof" => (*b"hhea", 22, false),
        b"vasc" => (*b"vhea", 4, false),
        b"vdsc" => (*b"vhea", 6, false),
        b"vlgp" => (*b"vhea", 8, false),
        b"vcrs" => (*b"vhea", 18, false),
        b"vcrn" => (*b"vhea", 20, false),
        b"vcof" => (*b"vhea", 22, false),
        b"xhgt" => (*b"OS/2", 86, false),
        b"cpht" => (*b"OS/2", 88, false),
        b"sbxs" => (*b"OS/2", 10, false),
        b"sbys" => (*b"OS/2", 12, false),
        b"sbxo" => (*b"OS/2", 14, false),
        b"sbyo" => (*b"OS/2", 16, false),
        b"spxs" => (*b"OS/2", 18, false),
        b"spys" => (*b"OS/2", 20, false),
        b"spxo" => (*b"OS/2", 22, false),
        b"spyo" => (*b"OS/2", 24, false),
        b"strs" => (*b"OS/2", 26, false),
        b"stro" => (*b"OS/2", 28, false),
        b"unds" => (*b"post", 10, false),
        b"undo" => (*b"post", 8, false),
        _ if &tag[..3] == b"gsp" && tag[3].is_ascii_digit() => {
            (*b"gasp", 4 + usize::from(tag[3] - b'0') * 4, true)
        }
        _ => return None,
    };
    Some(Field {
        table,
        offset,
        unsigned,
    })
}
fn read(data: &[u8], field: Field) -> Result<i32> {
    let value = bytes(data, field.offset, 2)?;
    Ok(if field.unsigned {
        i32::from(u16::from_be_bytes(value.try_into().unwrap()))
    } else {
        i32::from(i16::from_be_bytes(value.try_into().unwrap()))
    })
}
fn encode(value: i32, unsigned: bool) -> Result<[u8; 2]> {
    if unsigned {
        u16::try_from(value)
            .map(u16::to_be_bytes)
            .map_err(|_| fail("unsigned metric overflow"))
    } else {
        i16::try_from(value)
            .map(i16::to_be_bytes)
            .map_err(|_| fail("signed metric overflow"))
    }
}
fn gasp(data: &[u8]) -> Result<usize> {
    if u16_at(data, 0)? > 1 {
        return Err(fail("unsupported gasp version"));
    }
    let count = usize::from(u16_at(data, 2)?);
    if count == 0 {
        return Err(fail("gasp has no terminal range"));
    }
    bytes(data, 4, count * 4)?;
    let mut previous = None;
    for i in 0..count {
        if i % 256 == 0 {
            crate::cancel::check_current_cancel("instanced gasp ranges")?;
        }
        let max = u16_at(data, 4 + i * 4)?;
        if previous.is_some_and(|p| p >= max) {
            return Err(fail("gasp ranges are not strictly increasing"));
        }
        previous = Some(max);
    }
    if previous != Some(0xffff) {
        return Err(fail("missing terminal gasp range"));
    }
    Ok(count)
}
impl MetricStage {
    fn change(&mut self, tables: &Tables, metric: Tag, field: Field, value: i32) -> Result<()> {
        let source = tables
            .get(&field.table)
            .ok_or_else(|| fail("MVAR target table is absent"))?;
        let before = read(source, field)?;
        let encoded = encode(value, field.unsigned)?;
        if before == value {
            return Ok(());
        }
        if !self.tables.contains_key(&field.table) {
            let retained = self
                .tables
                .values()
                .try_fold(source.len(), |n, t| n.checked_add(t.len()))
                .ok_or_else(|| fail("metric stage size overflow"))?;
            if retained > 64 * 1024 * 1024 {
                return Err(WellfriendError::ResourceLimit(
                    "metric stage exceeds 64 MiB".into(),
                ));
            }
            let mut copy = Vec::with_capacity(source.len());
            for chunk in source.chunks(65536) {
                crate::cancel::check_current_cancel("metric target copy")?;
                copy.extend_from_slice(chunk);
            }
            self.tables.insert(field.table, copy);
        }
        self.tables.get_mut(&field.table).unwrap()[field.offset..field.offset + 2]
            .copy_from_slice(&encoded);
        self.changes.push(MetricChange {
            metric,
            table: field.table,
            offset: field.offset,
            before,
            after: value,
        });
        Ok(())
    }
}
pub(crate) fn freeze(
    tables: &Tables,
    coordinates: &[ttf_parser::NormalizedCoordinate],
) -> Result<MetricStage> {
    crate::cancel::check_current_cancel("MVAR instancing")?;
    let Some(source) = tables.get(b"MVAR") else {
        return Ok(MetricStage::default());
    };
    if u16_at(source, 0)? != 1 || u16_at(source, 4)? != 0 {
        return Err(fail("MVAR version or reserved field"));
    }
    let stride = usize::from(u16_at(source, 6)?);
    let count = usize::from(u16_at(source, 8)?);
    let start = usize::from(u16_at(source, 10)?);
    if stride < 8 {
        return Err(fail("MVAR record is shorter than its fixed fields"));
    }
    bytes(source, 12, count * stride)?;
    if count == 0 {
        if start != 0 {
            return Err(fail("empty MVAR has a non-null store"));
        }
        return Ok(MetricStage::default());
    }
    if start < 12 + count * stride {
        return Err(fail("MVAR store overlaps value records"));
    }
    let store = ItemVariationStore::parse(Arc::clone(source), start..source.len())?;
    store.require_short_deltas()?;
    let instance = store.prepare(coordinates)?;
    let mut stage = MetricStage::default();
    let mut previous = None;
    for i in 0..count {
        crate::cancel::check_current_cancel("MVAR field resolution")?;
        let at = 12 + i * stride;
        let tag: Tag = bytes(source, at, 4)?.try_into().unwrap();
        if previous.is_some_and(|p| p >= tag) {
            return Err(fail("MVAR tags must be unique and sorted"));
        }
        previous = Some(tag);
        let Some(field) = field(tag) else {
            stage.ignored_tags.push(tag);
            continue;
        };
        let table = tables
            .get(&field.table)
            .ok_or_else(|| fail("MVAR target table is absent"))?;
        if field.table == *b"OS/2" && field.offset >= 86 && u16_at(table, 0)? < 2 {
            return Err(fail("height metric requires OS/2 version 2 or later"));
        }
        if field.table == *b"gasp" && (field.offset - 4) / 4 + 1 >= gasp(table)? {
            return Err(fail("MVAR cannot alter the terminal gasp range"));
        }
        let delta = instance.delta(
            u32::from(u16_at(source, at + 4)?),
            u32::from(u16_at(source, at + 6)?),
        )?;
        let value = round_i32(f64::from(read(table, field)?) + delta)?;
        stage.change(tables, tag, field, value)?;
    }
    if let Some(data) = stage.tables.get(b"gasp") {
        gasp(data)?;
    }
    // Preserve an existing hhea/OS2 equality, but do not invent one when the
    // original font intentionally uses different vertical metrics.
    if let (Some(os2), Some(hhea), Some(new_os2)) = (
        tables.get(b"OS/2"),
        tables.get(b"hhea"),
        stage.tables.get(b"OS/2"),
    ) {
        if os2.len() >= 74
            && hhea.len() >= 10
            && os2[68..74] == hhea[4..10]
            && os2[68..74] != new_os2[68..74]
        {
            let values = (0..3)
                .map(|i| {
                    read(
                        new_os2,
                        Field {
                            table: *b"OS/2",
                            offset: 68 + i * 2,
                            unsigned: false,
                        },
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            for (i, value) in values.into_iter().enumerate() {
                stage.change(
                    tables,
                    [*b"hasc", *b"hdsc", *b"hlgp"][i],
                    Field {
                        table: *b"hhea",
                        offset: 4 + i * 2,
                        unsigned: false,
                    },
                    value,
                )?;
            }
            stage.synchronized_hhea = true;
        }
    }
    crate::cancel::check_current_cancel("MVAR staged publication")?;
    Ok(stage)
}

#[cfg(test)]
#[path = "mvar_instance_tests.rs"]
mod tests;
