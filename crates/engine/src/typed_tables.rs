//! Approved, source-bound table values and formulas. Fixed-grid cell editing
//! uses the linked-story writer; table topology is never inferred as authority.
use crate::linked_stories::{LinkedStoryRequest, StoryMode};
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecimalValue {
    /// Signed base-10 i128 coefficient as a string: JSON numbers must not lose
    /// integer precision in JavaScript bindings. Scale is bounded to 18.
    pub coefficient: String,
    pub scale: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Decimal {
    coefficient: i128,
    scale: u32,
}
fn fail(s: &str) -> WellfriendError {
    WellfriendError::invalid_input(s)
}
impl Decimal {
    fn parse(value: &DecimalValue) -> Result<Self> {
        if value.scale > 18 {
            return Err(fail("decimal scale exceeds 18"));
        }
        let coefficient = value
            .coefficient
            .parse::<i128>()
            .map_err(|_| fail("invalid decimal coefficient"))?;
        Ok(Self {
            coefficient,
            scale: value.scale,
        })
    }
    fn rescale(self, scale: u32) -> Result<i128> {
        self.coefficient
            .checked_mul(10i128.pow(scale - self.scale))
            .ok_or_else(|| fail("decimal overflow"))
    }
    fn add(self, other: Self, subtract: bool) -> Result<Self> {
        let scale = self.scale.max(other.scale);
        let a = self.rescale(scale)?;
        let b = other.rescale(scale)?;
        let coefficient = if subtract {
            a.checked_sub(b)
        } else {
            a.checked_add(b)
        }
        .ok_or_else(|| fail("decimal arithmetic overflow"))?;
        Ok(Self { coefficient, scale })
    }
    fn multiply(self, other: Self) -> Result<Self> {
        let mut coefficient = self
            .coefficient
            .checked_mul(other.coefficient)
            .ok_or_else(|| fail("decimal multiplication overflow"))?;
        let mut scale = self.scale + other.scale;
        while scale > 18 && coefficient % 10 == 0 {
            coefficient /= 10;
            scale -= 1;
        }
        if scale > 18 {
            return Err(fail("decimal multiplication exceeds exact supported scale"));
        }
        Ok(Self { coefficient, scale })
    }
    fn format(self, display_scale: u32) -> Result<String> {
        if display_scale > 18 {
            return Err(fail("decimal display scale exceeds 18"));
        }
        let coefficient = if display_scale >= self.scale {
            self.rescale(display_scale)?
        } else {
            let divisor = 10i128.pow(self.scale - display_scale);
            if self.coefficient % divisor != 0 {
                return Err(fail(
                    "decimal display would silently round; supply an explicitly rounded value",
                ));
            }
            self.coefficient / divisor
        };
        let negative = coefficient < 0;
        let mut digits = coefficient.unsigned_abs().to_string();
        if display_scale > 0 {
            let scale = display_scale as usize;
            if digits.len() <= scale {
                digits = format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits);
            }
            digits.insert(digits.len() - scale, '.');
        }
        Ok(format!("{}{digits}", if negative { "-" } else { "" }))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TableFormula {
    Constant {
        value: DecimalValue,
    },
    Cell {
        id: String,
    },
    Add {
        left: Box<TableFormula>,
        right: Box<TableFormula>,
    },
    Subtract {
        left: Box<TableFormula>,
        right: Box<TableFormula>,
    },
    Multiply {
        left: Box<TableFormula>,
        right: Box<TableFormula>,
    },
    Sum {
        cells: Vec<String>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TableValue {
    Text {
        text: String,
    },
    Decimal {
        value: DecimalValue,
    },
    Formula {
        expression: TableFormula,
        display_scale: u32,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedTableCell {
    pub id: String,
    pub row: usize,
    pub column: usize,
    pub value: TableValue,
    /// One source-bound frame and one styled paragraph. The writer will use
    /// the evaluated value, not any caller-supplied paragraph replacement text.
    pub binding: LinkedStoryRequest,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedEditableTable {
    pub id: String,
    pub input_sha256: String,
    pub cells: Vec<TypedTableCell>,
}
#[derive(Debug, Clone, Serialize)]
pub struct TypedTableReport {
    pub table_id: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub values: BTreeMap<String, String>,
    pub changed_pages: Vec<usize>,
    pub rebound_table: TypedEditableTable,
    pub topology: String,
}
fn references(
    expression: &TableFormula,
    out: &mut BTreeSet<String>,
    depth: usize,
    budget: &mut usize,
) -> Result<()> {
    *budget += 1;
    if depth > 64 || *budget > 100_000 {
        return Err(fail("table formula expression budget exceeded"));
    }
    match expression {
        TableFormula::Constant { value } => {
            Decimal::parse(value)?;
        }
        TableFormula::Cell { id } => {
            out.insert(id.clone());
        }
        TableFormula::Sum { cells } => {
            *budget = budget.saturating_add(cells.len());
            if *budget > 100_000 {
                return Err(fail("table formula reference budget exceeded"));
            }
            out.extend(cells.iter().cloned());
        }
        TableFormula::Add { left, right }
        | TableFormula::Subtract { left, right }
        | TableFormula::Multiply { left, right } => {
            references(left, out, depth + 1, budget)?;
            references(right, out, depth + 1, budget)?;
        }
    }
    Ok(())
}
fn evaluate(expression: &TableFormula, values: &BTreeMap<String, Decimal>) -> Result<Decimal> {
    let cell = |id: &str| {
        values
            .get(id)
            .copied()
            .ok_or_else(|| fail("formula references a missing or non-numeric cell"))
    };
    match expression {
        TableFormula::Constant { value } => Decimal::parse(value),
        TableFormula::Cell { id } => cell(id),
        TableFormula::Add { left, right } => {
            evaluate(left, values)?.add(evaluate(right, values)?, false)
        }
        TableFormula::Subtract { left, right } => {
            evaluate(left, values)?.add(evaluate(right, values)?, true)
        }
        TableFormula::Multiply { left, right } => {
            evaluate(left, values)?.multiply(evaluate(right, values)?)
        }
        TableFormula::Sum { cells } => cells.iter().try_fold(
            Decimal {
                coefficient: 0,
                scale: 0,
            },
            |sum, id| sum.add(cell(id)?, false),
        ),
    }
}
pub fn evaluate_table(table: &TypedEditableTable) -> Result<BTreeMap<String, String>> {
    evaluate_values(
        &table.id,
        &table
            .cells
            .iter()
            .map(|c| (c.id.as_str(), c.row, c.column, &c.value))
            .collect::<Vec<_>>(),
    )
}

/// Shared exact arithmetic for fixed bindings and the paginated table layout.
/// Borrow values instead of cloning a source story/font pool for every cell.
pub(crate) fn evaluate_values(
    table_id: &str,
    values: &[(&str, usize, usize, &TableValue)],
) -> Result<BTreeMap<String, String>> {
    if table_id.is_empty() || values.is_empty() || values.len() > 4096 {
        return Err(fail("typed table identity/cell budget invalid"));
    }
    let mut cells = BTreeMap::new();
    let mut positions = BTreeSet::new();
    for &(id, row, column, value) in values {
        if id.is_empty()
            || cells.insert(id.to_owned(), value).is_some()
            || !positions.insert((row, column))
        {
            return Err(fail("typed table IDs/positions must be unique"));
        }
    }
    let mut dependencies = BTreeMap::new();
    let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut budget = 0;
    for &(cell_id, _, _, value) in values {
        let mut refs = BTreeSet::new();
        if let TableValue::Formula { expression, .. } = value {
            references(expression, &mut refs, 0, &mut budget)?;
        }
        for id in &refs {
            if !cells.contains_key(id) {
                return Err(fail("table formula references unknown cell"));
            }
            dependents
                .entry(id.clone())
                .or_default()
                .push(cell_id.to_owned());
        }
        dependencies.insert(cell_id.to_owned(), refs.len());
    }
    let mut ready: BTreeSet<_> = dependencies
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut numeric = BTreeMap::new();
    let mut output = BTreeMap::new();
    while let Some(id) = ready.pop_first() {
        crate::cancel::check_current_cancel("typed table evaluation")?;
        let (number, text) = match cells[&id] {
            TableValue::Text { text } => (None, text.clone()),
            TableValue::Decimal { value } => {
                let n = Decimal::parse(value)?;
                (Some(n), n.format(value.scale)?)
            }
            TableValue::Formula {
                expression,
                display_scale,
            } => {
                let n = evaluate(expression, &numeric)?;
                (Some(n), n.format(*display_scale)?)
            }
        };
        if text.len() > 64_000 {
            return Err(fail("typed table cell text exceeds budget"));
        }
        if let Some(number) = number {
            numeric.insert(id.clone(), number);
        }
        output.insert(id.clone(), text);
        if let Some(children) = dependents.get(&id) {
            for child in children {
                let count = dependencies.get_mut(child).unwrap();
                *count -= 1;
                if *count == 0 {
                    ready.insert(child.clone());
                }
            }
        }
    }
    if output.len() != cells.len() {
        return Err(fail("cyclic table formulas require explicit correction"));
    }
    Ok(output)
}

/// Fixed-cell transaction: all preimages are validated before any private
/// mutation, edits run in descending logical order, and errors publish nothing.
pub fn apply_typed_table(
    input: &[u8],
    table: &TypedEditableTable,
) -> Result<(Vec<u8>, TypedTableReport)> {
    let digest = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
    if digest(input) != table.input_sha256 {
        return Err(fail("typed table is bound to another PDF revision"));
    }
    if table.cells.len() > 64 {
        return Err(fail(
            "fixed-grid mutation supports at most 64 owned cell stories per document",
        ));
    }
    let values = evaluate_table(table)?;
    let mut stories = BTreeSet::new();
    let mut edits = Vec::new();
    for cell in &table.cells {
        let mut binding = cell.binding.clone();
        if binding.input_sha256 != table.input_sha256
            || binding.frames.len() != 1
            || binding.paragraphs.len() != 1
            || binding.mode != StoryMode::PreserveLayout
            || binding.allow_page_creation
            || binding.table_layout.is_some()
            || !stories.insert(binding.story_id.clone())
        {
            return Err(fail("typed cells need unique one-frame preserve-layout story bindings on the exact input"));
        }
        binding.paragraphs[0].text = values[&cell.id].clone();
        crate::linked_stories::preview_linked_story(input, &binding)?;
        edits.push((cell.id.clone(), binding));
    }
    for i in 0..edits.len() {
        for j in i + 1..edits.len() {
            let a = &edits[i].1.frames[0];
            let b = &edits[j].1.frames[0];
            if a.page == b.page
                && (a.rect[0] < b.rect[2]
                    && b.rect[0] < a.rect[2]
                    && a.rect[1] < b.rect[3]
                    && b.rect[1] < a.rect[3]
                    || a.logical_range[0] < b.logical_range[1]
                        && b.logical_range[0] < a.logical_range[1])
            {
                return Err(fail(
                    "typed cell bindings overlap; merged/topology edits require separate review",
                ));
            }
        }
    }
    edits.sort_by_key(|(_, b)| std::cmp::Reverse((b.frames[0].page, b.frames[0].logical_range[0])));
    let mut output = input.to_vec();
    let mut pages = BTreeSet::new();
    for (_, binding) in &mut edits {
        binding.input_sha256 = digest(&output);
        pages.insert(binding.frames[0].page);
        output = crate::linked_stories::apply_linked_story(&output, binding)?.0;
    }
    let saved = crate::linked_stories::load_linked_stories(&output)?
        .into_iter()
        .map(|s| (s.request.story_id.clone(), s.request))
        .collect::<BTreeMap<_, _>>();
    let mut rebound = table.clone();
    rebound.input_sha256 = digest(&output);
    for cell in &mut rebound.cells {
        cell.binding = saved
            .get(&cell.binding.story_id)
            .cloned()
            .ok_or_else(|| fail("saved typed cell ownership missing"))?;
    }
    output = save_table_model(&output, &rebound)?;
    rebound.input_sha256 = digest(&output);
    for cell in &mut rebound.cells {
        cell.binding.input_sha256 = rebound.input_sha256.clone();
    }
    let report = TypedTableReport { table_id: table.id.clone(), input_sha256: table.input_sha256.clone(), output_sha256: digest(&output),
        values, changed_pages: pages.into_iter().collect(), rebound_table: rebound,
        topology: "approved fixed grid only; no inferred formulas, merged-cell restructuring, table movement or cross-page headers".into() };
    Ok((output, report))
}

fn save_table_model(input: &[u8], table: &TypedEditableTable) -> Result<Vec<u8>> {
    use crate::writer::{write_incremental_update, IncrementalObject};
    use crate::{ContentEngine, PdfDictionary, PdfObject};
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let mut catalog = engine.document().get_catalog()?;
    let mut entries = match catalog.get("WellfriendTypedTables") {
        None => PdfDictionary::empty(),
        Some(PdfObject::Dictionary(d)) => d.clone(),
        _ => return Err(fail("malformed saved typed table dictionary")),
    };
    let key = format!("{:x}", Sha256::digest(table.id.as_bytes()));
    if entries.len() >= 64 && !entries.contains_key(&key) {
        return Err(fail("saved typed table budget exceeded"));
    }
    let mut stored = table.clone();
    stored.input_sha256.clear();
    for cell in &mut stored.cells {
        cell.binding.input_sha256.clear();
        cell.binding.fonts.clear();
        cell.binding.signature_policy_override = false;
    }
    let raw = serde_json::to_vec(&stored).map_err(|e| fail(&e.to_string()))?;
    if raw.len() > 16 * 1024 * 1024 {
        return Err(fail("saved typed table exceeds 16 MiB"));
    }
    let number = reader
        .object_ids()
        .iter()
        .map(|r| r.0)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| fail("table metadata object overflow"))?;
    let mut dict = PdfDictionary::empty();
    dict.insert("Type", PdfObject::Name("WellfriendTypedTable".into()));
    dict.insert("Length", PdfObject::Integer(raw.len() as i64));
    entries.insert(
        key,
        PdfObject::Reference {
            number,
            generation: 0,
        },
    );
    catalog.insert("WellfriendTypedTables", PdfObject::Dictionary(entries));
    let root = reader
        .root_reference()
        .ok_or_else(|| fail("table metadata root missing"))?;
    write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number,
                generation: 0,
                object: PdfObject::Stream { dict, raw },
            },
            IncrementalObject {
                number: root.0,
                generation: root.1,
                object: PdfObject::Dictionary(catalog),
            },
        ],
    )
}

/// Rebind stored formulas to verified current story owners. A missing/modified
/// owner fails; matching cell words are never used as a replacement identity.
pub fn load_typed_tables(input: &[u8]) -> Result<Vec<TypedEditableTable>> {
    use crate::{ContentEngine, PdfObject};
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let catalog = engine.document().get_catalog()?;
    let entries = match catalog.get("WellfriendTypedTables") {
        None => return Ok(Vec::new()),
        Some(PdfObject::Dictionary(d)) => d,
        _ => return Err(fail("malformed typed table registry")),
    };
    if entries.len() > 64 {
        return Err(fail("saved typed table registry budget exceeded"));
    }
    let saved = crate::linked_stories::load_linked_stories(input)?
        .into_iter()
        .map(|s| (s.request.story_id.clone(), s.request))
        .collect::<BTreeMap<_, _>>();
    let mut result = Vec::new();
    let mut budget = 0usize;
    for (_, reference) in entries.iter() {
        crate::cancel::check_current_cancel("typed table reopening")?;
        let object = engine.document().reader().resolve(reference.clone())?;
        let decoded = crate::filters::decode_stream_lossless_with_limits(
            &object,
            engine.document().reader(),
            &crate::filters::DecodeLimits {
                max_decoded_bytes_per_stream: 16 * 1024 * 1024,
                ..Default::default()
            },
        )?;
        budget += decoded.data.len();
        if budget > 64 * 1024 * 1024
            || decoded.status != crate::filters::StreamDecodeStatus::Complete
        {
            return Err(fail("saved table decode budget exceeded"));
        }
        let mut table: TypedEditableTable =
            serde_json::from_slice(&decoded.data).map_err(|e| fail(&e.to_string()))?;
        let values = evaluate_table(&table)?;
        table.input_sha256 = format!("{:x}", Sha256::digest(input));
        for cell in &mut table.cells {
            let binding = saved
                .get(&cell.binding.story_id)
                .ok_or_else(|| fail("saved typed cell owner missing"))?;
            if binding.paragraphs.len() != 1
                || binding.paragraphs[0].text != values[&cell.id]
                || binding.frames.len() != 1
                || cell.binding.frames.len() != 1
                || binding.frames[0].owner != cell.binding.frames[0].owner
            {
                return Err(fail("saved typed cell was edited outside the table transaction; reconcile formulas explicitly"));
            }
            cell.binding = binding.clone();
        }
        result.push(table);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_decimal_values_never_silently_round() {
        let a = Decimal::parse(&DecimalValue {
            coefficient: "1".into(),
            scale: 1,
        })
        .unwrap();
        let b = Decimal::parse(&DecimalValue {
            coefficient: "2".into(),
            scale: 1,
        })
        .unwrap();
        assert_eq!(a.add(b, false).unwrap().format(2).unwrap(), "0.30");
        assert_eq!(a.multiply(b).unwrap().format(2).unwrap(), "0.02");
        assert!(a.multiply(b).unwrap().format(1).is_err());
        assert!(Decimal {
            coefficient: i128::MAX,
            scale: 0
        }
        .add(
            Decimal {
                coefficient: 1,
                scale: 0
            },
            false
        )
        .is_err());
    }

    #[test]
    fn formulas_are_topologically_evaluated_and_cycles_rejected() {
        let binding = || {
            serde_json::from_value(serde_json::json!({"story_id":"unused", "input_sha256":"unused", "frames":[], "paragraphs":[], "fonts":[]})).unwrap()
        };
        let mut table = TypedEditableTable {
            id: "t".into(),
            input_sha256: String::new(),
            cells: vec![
                TypedTableCell {
                    id: "a".into(),
                    row: 0,
                    column: 0,
                    value: TableValue::Decimal {
                        value: DecimalValue {
                            coefficient: "10".into(),
                            scale: 2,
                        },
                    },
                    binding: binding(),
                },
                TypedTableCell {
                    id: "b".into(),
                    row: 0,
                    column: 1,
                    value: TableValue::Formula {
                        expression: TableFormula::Add {
                            left: Box::new(TableFormula::Cell { id: "a".into() }),
                            right: Box::new(TableFormula::Constant {
                                value: DecimalValue {
                                    coefficient: "20".into(),
                                    scale: 2,
                                },
                            }),
                        },
                        display_scale: 2,
                    },
                    binding: binding(),
                },
            ],
        };
        assert_eq!(evaluate_table(&table).unwrap()["b"], "0.30");
        table.cells[0].value = TableValue::Formula {
            expression: TableFormula::Cell { id: "b".into() },
            display_scale: 2,
        };
        assert!(evaluate_table(&table).is_err());
    }
}
