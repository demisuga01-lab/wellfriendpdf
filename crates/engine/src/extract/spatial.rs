//! Part B — spatial label→value pairing (the general KV engine).
//!
//! For documents without form fields (most invoices/receipts), find LABEL text
//! and pair it with the VALUE text spatially associated with it, using geometry
//! and patterns. Operates on the canonical [`crate::parse::Document`] blocks, so
//! it is **identical** for digital-born and OCR'd input.
//!
//! Pairing strategies, in priority order:
//! 1. **Inline** — `Total: $42.00` in one block: split at the colon.
//! 2. **Right-of** — the value block is on the same baseline, just to the right
//!    (or to the *left* for RTL).
//! 3. **Below** — the value block is the next line down, left-aligned with the
//!    label.
//!
//! Each pair is scored (label clarity × geometric strength × pattern match) and
//! low-scoring pairs are still emitted but flagged by a low `confidence`.

use crate::extract::value::{normalize, ValueHint};
use crate::extract::{Field, FieldSource};
use crate::parse::{Block, BlockKind, Document};

/// A flattened text fragment with geometry — one per block (plus colon-split
/// pieces). The spatial engine reasons entirely over these.
#[derive(Debug, Clone)]
struct Frag {
    text: String,
    page: u32,
    /// `[x0,y0,x1,y1]` user space, y-up.
    bbox: [f64; 4],
    confidence: f32,
}

impl Frag {
    fn cy(&self) -> f64 {
        (self.bbox[1] + self.bbox[3]) / 2.0
    }
    fn height(&self) -> f64 {
        (self.bbox[3] - self.bbox[1]).abs()
    }
    fn left(&self) -> f64 {
        self.bbox[0]
    }
    fn right(&self) -> f64 {
        self.bbox[2]
    }
}

/// Extract spatial label→value [`Field`]s from a document's body blocks.
///
/// Pairs come from two places: free text blocks (inline / right-of / below) and
/// **table cells** (a label cell pairs with the value cell in the same row, or
/// the cell below). Real invoices/receipts put most labeled fields inside an
/// (often borderless) table the layout engine recovered, so table-cell pairing
/// is essential — not an afterthought.
pub fn extract_spatial_fields(doc: &Document) -> Vec<Field> {
    let mut fields = extract_block_fields(doc);
    fields.extend(extract_table_fields(doc));
    fields
}

/// Label→value pairs from free (non-table) text blocks.
fn extract_block_fields(doc: &Document) -> Vec<Field> {
    let frags = collect_frags(doc);

    // First pass: inline "label: value" pairs within a single fragment. Some
    // generators collapse several pairs onto one line, so this handles
    // "Account: AC-1 Period: 2026..." as multiple fields.
    let mut fields = Vec::new();
    let mut consumed = vec![false; frags.len()];

    for (i, f) in frags.iter().enumerate() {
        let pairs = extract_inline_label_values(&f.text);
        if pairs.is_empty() {
            continue;
        }
        for pair in pairs {
            fields.push(make_field(&pair.label, &pair.value, f, f, GeoKind::Inline));
        }
        consumed[i] = true;
    }

    // Second pass: a fragment that is *just a label* paired with a neighbor.
    for (i, f) in frags.iter().enumerate() {
        if consumed[i] {
            continue;
        }
        if !is_label_text(&f.text) {
            continue;
        }
        let label = strip_label(&f.text);
        if label.is_empty() {
            continue;
        }
        // Find the best value neighbor that is NOT itself a label.
        if let Some((j, kind)) = best_value_neighbor(&frags, i, &consumed) {
            let v = &frags[j];
            fields.push(make_field(&label, &v.text, f, v, kind));
            consumed[i] = true;
            consumed[j] = true;
        }
    }

    fields
}

/// Label→value pairs from inside table cells (Part B.2). For each row, a label
/// cell (`Total:` / a known label phrase) pairs with the next non-empty cell to
/// its right in the same row. This recovers invoice header fields and totals
/// that the layout engine grouped into a borderless table.
fn extract_table_fields(doc: &Document) -> Vec<Field> {
    let mut fields = Vec::new();
    for b in &doc.body {
        let BlockKind::Table { table, .. } = &b.kind else {
            continue;
        };
        for row in &table.rows {
            for cell in row.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
                for pair in extract_inline_label_values(cell) {
                    fields.push(table_field(
                        &pair.label,
                        &pair.value,
                        b.page,
                        b.bbox,
                        b.confidence,
                    ));
                }
                for pair in extract_compact_label_grid(cell) {
                    fields.push(table_field(
                        &pair.label,
                        &pair.value,
                        b.page,
                        b.bbox,
                        b.confidence,
                    ));
                }
            }

            if !allow_same_row_table_pairing(row) {
                continue;
            }
            // Walk cells; when a label cell is found, the value is the next
            // non-empty cell in the same row (skipping blank grid slots).
            let mut ci = 0;
            while ci < row.len() {
                let cell = row[ci].trim();
                if !cell.is_empty() && is_label_text(cell) {
                    // Else: the value is the next non-empty cell to the right.
                    if let Some(vj) = (ci + 1..row.len()).find(|&j| !row[j].trim().is_empty()) {
                        let value = row[vj].trim();
                        if !is_label_text(value) {
                            fields.push(table_field(cell, value, b.page, b.bbox, b.confidence));
                            ci = vj + 1;
                            continue;
                        }
                    }
                }
                ci += 1;
            }
        }

        for rows in table.rows.windows(2) {
            for pair in table_below_pairs(&rows[0], &rows[1]) {
                fields.push(table_field(
                    &pair.label,
                    &pair.value,
                    b.page,
                    b.bbox,
                    b.confidence,
                ));
            }
        }
    }
    fields
}

/// Build a table-derived field. The bbox is the table block's bbox (cell-level
/// geometry is not surfaced through `Table.rows`); confidence inherits the
/// block's, scaled by the geometric strength of an in-row pairing.
fn table_field(label: &str, raw_value: &str, page: u32, bbox: [f64; 4], block_conf: f32) -> Field {
    let key = strip_label(label);
    let raw = raw_value.trim().to_string();
    let hint = hint_for_label(&key);
    let value = normalize(&raw, hint);
    let label_clarity = if label.trim_end().ends_with(':') {
        1.0
    } else {
        0.85
    };
    let pattern_match = match (&value, hint) {
        (crate::extract::FieldValue::Text { .. }, ValueHint::Any) => 0.9,
        (crate::extract::FieldValue::Text { .. }, _) => 0.6,
        _ => 1.0,
    };
    let conf = (label_clarity * 0.9 * pattern_match * block_conf.clamp(0.1, 1.0)).clamp(0.0, 1.0);
    Field {
        key,
        value,
        raw,
        page,
        bbox,
        confidence: conf,
        source: FieldSource::Spatial,
    }
}

/// Flatten the document body into geometry-bearing fragments. Furniture and
/// figures contribute nothing useful; tables are handled by the profile layer.
fn collect_frags(doc: &Document) -> Vec<Frag> {
    let mut frags = Vec::new();
    for b in &doc.body {
        let Some(text) = block_text(b) else {
            continue;
        };
        for line in text.lines() {
            let text = line.trim().to_string();
            if text.is_empty() {
                continue;
            }
            frags.push(Frag {
                text,
                page: b.page,
                bbox: b.bbox,
                confidence: b.confidence,
            });
        }
    }
    frags
}

/// The plain text of a block, for the block kinds that carry label/value text.
fn block_text(b: &Block) -> Option<String> {
    match &b.kind {
        BlockKind::Paragraph { text }
        | BlockKind::Text { text }
        | BlockKind::Heading { text, .. }
        | BlockKind::Title { text }
        | BlockKind::Caption { text, .. }
        | BlockKind::Header { text }
        | BlockKind::Footer { text } => Some(text.to_plain()),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum GeoKind {
    Inline,
    RightOf,
    Below,
    LeftOf, // RTL
}

impl GeoKind {
    /// Geometric strength component of the confidence.
    fn strength(self) -> f32 {
        match self {
            GeoKind::Inline => 1.0,
            GeoKind::RightOf | GeoKind::LeftOf => 0.9,
            GeoKind::Below => 0.75,
        }
    }
}

/// Split `"Total: $42.00"` → `("Total", "$42.00")`. Splits on the FIRST colon
/// that is followed by content. Returns `None` if there's no such colon.
#[cfg(test)]
fn split_inline_label_value(s: &str) -> Option<(String, String)> {
    extract_inline_label_values(s)
        .into_iter()
        .next()
        .map(|pair| (pair.label, pair.value))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlinePair {
    label: String,
    value: String,
}

fn extract_inline_label_values(s: &str) -> Vec<InlinePair> {
    let hits = label_colon_hits(s);
    if hits.is_empty() {
        return extract_single_inline_label_value(s).into_iter().collect();
    }

    let mut pairs = Vec::new();
    for (idx, hit) in hits.iter().enumerate() {
        let value_start = hit.end;
        let value_end = hits.get(idx + 1).map(|next| next.start).unwrap_or(s.len());
        if value_start > value_end || value_end > s.len() {
            continue;
        }
        let value = clean_inline_value_for_label(hit.label, &s[value_start..value_end]);
        if !value.is_empty() && !is_delimiter_only_label(hit.label) {
            pairs.push(InlinePair {
                label: hit.label.to_string(),
                value,
            });
        }
    }
    pairs
}

fn extract_single_inline_label_value(s: &str) -> Option<InlinePair> {
    let idx = s.find(':')?;
    let (label, rest) = s.split_at(idx);
    let label = label.trim();
    if !is_label_text(label) {
        return None;
    }
    let label = strip_label(label);
    let value = clean_inline_value_for_label(&label, &rest[1..]);
    if value.is_empty() {
        return None;
    }
    Some(InlinePair { label, value })
}

#[derive(Debug, Clone, Copy)]
struct LabelHit {
    start: usize,
    end: usize,
    label: &'static str,
}

fn label_colon_hits(s: &str) -> Vec<LabelHit> {
    let lower = s.to_ascii_lowercase();
    let mut hits = Vec::new();
    for label in LABEL_LEXICON {
        let pattern = format!("{label}:");
        let mut search_from = 0;
        while let Some(rel) = lower[search_from..].find(&pattern) {
            let start = search_from + rel;
            let end = start + pattern.len();
            if is_label_start_boundary(&lower, start) {
                hits.push(LabelHit { start, end, label });
            }
            search_from = start + 1;
        }
    }
    hits.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));

    let mut deduped: Vec<LabelHit> = Vec::new();
    for hit in hits {
        if deduped
            .last()
            .is_some_and(|prev| hit.start < prev.end || hit.start == prev.start)
        {
            continue;
        }
        deduped.push(hit);
    }
    deduped
}

fn is_label_start_boundary(s: &str, start: usize) -> bool {
    start == 0
        || s[..start]
            .chars()
            .next_back()
            .is_none_or(|ch| !ch.is_ascii_alphanumeric())
}

fn clean_inline_value(value: &str) -> String {
    let mut trimmed = value.trim();
    for marker in [" corpus document", " page "] {
        if let Some(idx) = trimmed.to_ascii_lowercase().find(marker) {
            trimmed = trimmed[..idx].trim();
        }
    }
    trimmed
        .trim_matches(|ch: char| ch == '-' || ch == '|' || ch == ';' || ch.is_whitespace())
        .trim()
        .to_string()
}

fn clean_inline_value_for_label(label: &str, value: &str) -> String {
    let cleaned = clean_inline_value(value);
    let key = label.to_ascii_lowercase();
    if key == "payment terms" {
        return String::new();
    }
    if key == "period" {
        return period_prefix(&cleaned);
    }
    if key == "account" || key == "ref" || key == "date" || key == "due" || key == "due date" {
        return first_token(&cleaned);
    }
    if key == "reference" {
        let first = first_token(&cleaned);
        if first.to_ascii_uppercase().starts_with("REF-") {
            return first;
        }
        return cleaned;
    }
    if matches!(
        key.as_str(),
        "total" | "total due" | "amount" | "amount due" | "balance" | "closing balance"
    ) {
        return amount_prefix(&cleaned);
    }
    if matches!(
        key.as_str(),
        "seat"
            | "gate"
            | "flight"
            | "class"
            | "booking"
            | "email"
            | "phone"
            | "status"
            | "priority"
    ) {
        return first_token(&cleaned);
    }
    cleaned
}

fn is_delimiter_only_label(label: &str) -> bool {
    label.eq_ignore_ascii_case("payment terms")
}

fn first_token(s: &str) -> String {
    s.split_whitespace().next().unwrap_or("").to_string()
}

fn amount_prefix(s: &str) -> String {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    if tokens.is_empty() {
        return String::new();
    }
    if tokens.len() >= 2 && looks_like_currency_code(tokens[0]) {
        return format!("{} {}", tokens[0], tokens[1]);
    }
    tokens[0].to_string()
}

fn looks_like_currency_code(token: &str) -> bool {
    token.len() == 3 && token.chars().all(|ch| ch.is_ascii_uppercase())
}

fn period_prefix(s: &str) -> String {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    if tokens.len() >= 3 && tokens[1].eq_ignore_ascii_case("to") {
        return tokens[..3].join(" ");
    }
    first_token(s)
}

/// A short label lexicon of phrases that act as labels even without a colon.
const LABEL_LEXICON: &[&str] = &[
    "account",
    "account id",
    "total",
    "subtotal",
    "tax",
    "total due",
    "amount due",
    "amount",
    "balance",
    "balance due",
    "closing balance",
    "invoice number",
    "invoice no",
    "invoice #",
    "invoice",
    "inv no",
    "inv #",
    "date",
    "invoice date",
    "due",
    "due date",
    "period",
    "order number",
    "order no",
    "order #",
    "po number",
    "po #",
    "purchase order",
    "bill to",
    "ship to",
    "sold to",
    "vendor",
    "customer",
    "account number",
    "account no",
    "reference",
    "ref",
    "document type",
    "status",
    "priority",
    "company",
    "department",
    "requested by",
    "reviewed by",
    "full name",
    "passenger",
    "from",
    "to",
    "seat",
    "gate",
    "flight",
    "class",
    "booking",
    "quantity",
    "qty",
    "unit price",
    "price",
    "description",
    "phone",
    "email",
    "name",
    "address",
    "merchant",
    "store",
    "receipt number",
    "receipt no",
    "payment",
    "payment terms",
    "discount",
    "shipping",
    "grand total",
    "net",
    "gross",
];

/// Is this text a *label*? Either it ends with a colon (after a short-ish run),
/// or it matches a known label phrase. Labels are short; long prose is not.
fn is_label_text(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    let stripped = strip_label(t);
    let words = stripped.split_whitespace().count();
    // A trailing colon is the strongest cue.
    if t.trim_end().ends_with(':') && words <= 6 && !stripped.is_empty() {
        return true;
    }
    // Otherwise a known label phrase (case-insensitive), short.
    if words <= 4 {
        let low = stripped.to_ascii_lowercase();
        return LABEL_LEXICON.iter().any(|l| low == *l);
    }
    false
}

/// Remove a trailing colon and surrounding whitespace from a label.
fn strip_label(s: &str) -> String {
    s.trim().trim_end_matches(':').trim().to_string()
}

fn allow_same_row_table_pairing(row: &[String]) -> bool {
    non_empty_indices(row).len() == 2
}

fn table_below_pairs(label_row: &[String], value_row: &[String]) -> Vec<InlinePair> {
    let non_empty = non_empty_indices(label_row);
    let label_indices: Vec<usize> = non_empty
        .iter()
        .copied()
        .filter(|&idx| is_label_text(&label_row[idx]))
        .collect();

    if label_indices.len() < 2 || label_indices.len() > 4 || label_indices.len() != non_empty.len()
    {
        return Vec::new();
    }

    let mut pairs = Vec::new();
    for idx in label_indices {
        let Some(value) = value_row.get(idx).map(|v| v.trim()) else {
            return Vec::new();
        };
        if value.is_empty() || is_label_text(value) {
            return Vec::new();
        }
        pairs.push(InlinePair {
            label: strip_label(&label_row[idx]),
            value: clean_inline_value(value),
        });
    }
    pairs
}

fn non_empty_indices(row: &[String]) -> Vec<usize> {
    row.iter()
        .enumerate()
        .filter_map(|(idx, cell)| (!cell.trim().is_empty()).then_some(idx))
        .collect()
}

fn extract_compact_label_grid(s: &str) -> Vec<InlinePair> {
    let low = s.to_ascii_lowercase();
    if !low.contains("boarding pass") {
        return Vec::new();
    }

    let tokens: Vec<&str> = s.split_whitespace().collect();
    let mut pairs = Vec::new();
    let mut idx = 0;
    while idx < tokens.len() {
        let Some((labels, value_start)) = compact_label_run(&tokens, idx) else {
            idx += 1;
            continue;
        };
        let value_end = next_compact_label_run_or_footer(&tokens, value_start);
        if value_end <= value_start {
            idx = value_start.max(idx + 1);
            continue;
        }
        let values = &tokens[value_start..value_end];
        if values.len() >= labels.len() {
            pairs.extend(partition_compact_values(&labels, values));
        }
        idx = value_end.max(value_start + 1);
    }

    if pairs.len() < 4 {
        Vec::new()
    } else {
        pairs
    }
}

fn compact_label_run(tokens: &[&str], start: usize) -> Option<(Vec<String>, usize)> {
    let mut labels = Vec::new();
    let mut idx = start;
    while idx < tokens.len() {
        let token = compact_token(tokens[idx]);
        if !COMPACT_GRID_LABELS.contains(&token.as_str()) {
            break;
        }
        labels.push(token);
        idx += 1;
    }
    (labels.len() >= 2).then_some((labels, idx))
}

fn next_compact_label_run_or_footer(tokens: &[&str], start: usize) -> usize {
    for idx in start..tokens.len() {
        let token = compact_token(tokens[idx]);
        if token == "corpus" || token == "page" {
            return idx;
        }
        if compact_label_run(tokens, idx).is_some() {
            return idx;
        }
    }
    tokens.len()
}

fn partition_compact_values(labels: &[String], values: &[&str]) -> Vec<InlinePair> {
    let mut pairs = Vec::new();
    let mut value_idx = 0;
    for (label_idx, label) in labels.iter().enumerate() {
        let remaining_labels = labels.len() - label_idx;
        let remaining_values = values.len().saturating_sub(value_idx);
        if remaining_values < remaining_labels {
            break;
        }
        let mut take = 1;
        if compact_label_can_take_extra(label) && remaining_values > remaining_labels {
            take += remaining_values - remaining_labels;
        }
        let end = (value_idx + take).min(values.len());
        pairs.push(InlinePair {
            label: label.clone(),
            value: values[value_idx..end].join(" "),
        });
        value_idx = end;
    }
    pairs
}

fn compact_label_can_take_extra(label: &str) -> bool {
    matches!(label, "passenger" | "full name" | "company" | "department")
}

fn compact_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .to_ascii_lowercase()
}

const COMPACT_GRID_LABELS: &[&str] = &[
    "passenger",
    "from",
    "to",
    "date",
    "seat",
    "gate",
    "flight",
    "class",
    "booking",
];

/// Find the best value fragment for the label at `label_idx`. Searches for a
/// right-of (same line), then below (aligned) neighbor, scoring by proximity.
/// Skips fragments that are themselves labels (so "Date:" never pairs with
/// "Total:") and fragments already consumed.
fn best_value_neighbor(
    frags: &[Frag],
    label_idx: usize,
    consumed: &[bool],
) -> Option<(usize, GeoKind)> {
    let label = &frags[label_idx];
    let line_tol = (label.height() * 0.6).max(2.0);

    let mut best: Option<(usize, GeoKind, f64)> = None; // (idx, kind, cost)

    for (j, f) in frags.iter().enumerate() {
        if j == label_idx || consumed[j] || f.page != label.page {
            continue;
        }
        if is_label_text(&f.text) {
            continue; // a value must not be another label
        }
        let same_line = (f.cy() - label.cy()).abs() <= line_tol;

        // Right-of (same line, starts after the label ends).
        if same_line && f.left() >= label.right() - line_tol {
            let cost = (f.left() - label.right()).max(0.0);
            consider(&mut best, j, GeoKind::RightOf, cost);
            continue;
        }
        // Left-of (RTL: value before the label on the same line).
        if same_line && f.right() <= label.left() + line_tol {
            let cost = (label.left() - f.right()).max(0.0) + 1000.0; // prefer right-of
            consider(&mut best, j, GeoKind::LeftOf, cost);
            continue;
        }
        // Below: next line down, left edges roughly aligned.
        let below = f.cy() < label.cy() - line_tol;
        let aligned = (f.left() - label.left()).abs() <= label.height().max(6.0) * 2.0;
        if below && aligned {
            let vgap = (label.cy() - f.cy()).max(0.0);
            // Only the *nearest* line below is a plausible value.
            let cost = vgap + 500.0; // prefer same-line neighbors over below
            consider(&mut best, j, GeoKind::Below, cost);
        }
    }
    best.map(|(j, k, _)| (j, k))
}

fn consider(best: &mut Option<(usize, GeoKind, f64)>, j: usize, kind: GeoKind, cost: f64) {
    match best {
        Some((_, _, c)) if *c <= cost => {}
        _ => *best = Some((j, kind, cost)),
    }
}

/// Build a [`Field`] from a label fragment, a value string, and the geometry of
/// the value's home fragment.
fn make_field(
    label: &str,
    raw_value: &str,
    label_frag: &Frag,
    value_frag: &Frag,
    kind: GeoKind,
) -> Field {
    let key = strip_label(label);
    let raw = raw_value.trim().to_string();
    let hint = hint_for_label(&key);
    let value = normalize(&raw, hint);

    // Confidence = label clarity × geometric strength × pattern match, scaled by
    // the value block's own (e.g. OCR) confidence.
    let label_clarity = if label.trim_end().ends_with(':') {
        1.0
    } else {
        0.8
    };
    let pattern_match = match (&value, hint) {
        (crate::extract::FieldValue::Text { .. }, ValueHint::Any) => 0.9, // text where text is fine
        (crate::extract::FieldValue::Text { .. }, _) => 0.6, // expected a type, got text
        _ => 1.0,                                            // typed value matched
    };
    let geo = kind.strength();
    let block_conf = value_frag.confidence.clamp(0.0, 1.0).max(0.1);
    let confidence = (label_clarity * geo * pattern_match * block_conf).clamp(0.0, 1.0);

    // The label fragment's geometry is not stored on the field (the value's
    // location is what matters for consumers), but it gates confidence above.
    let _ = label_frag;

    Field {
        key,
        value,
        raw,
        page: value_frag.page,
        bbox: value_frag.bbox,
        confidence,
        source: FieldSource::Spatial,
    }
}

/// Expected value type for a label, to bias normalization.
fn hint_for_label(key: &str) -> ValueHint {
    let k = key.to_ascii_lowercase();
    if k.contains("date") || k == "due" {
        ValueHint::Date
    } else if k.contains("email") {
        ValueHint::Email
    } else if k.contains("phone") || k.contains("tel") || k.contains("fax") {
        ValueHint::Phone
    } else if k.contains("total")
        || k.contains("amount")
        || k.contains("balance")
        || k.contains("subtotal")
        || k.contains("tax")
        || k.contains("price")
        || k.contains("due")
        || k.contains("payment")
        || k.contains("discount")
    {
        ValueHint::Amount
    } else if k == "qty" || k.contains("quantity") {
        ValueHint::Number
    } else {
        ValueHint::Any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_detection() {
        assert!(is_label_text("Total:"));
        assert!(is_label_text("Invoice Number:"));
        assert!(is_label_text("Total")); // lexicon
        assert!(is_label_text("Account"));
        assert!(is_label_text("Account ID"));
        assert!(is_label_text("Passenger"));
        assert!(is_label_text("Booking"));
        assert!(!is_label_text(
            "This is a long sentence of body prose, not a label"
        ));
        assert!(!is_label_text(""));
    }

    #[test]
    fn inline_split() {
        assert_eq!(
            split_inline_label_value("Total: $42.00"),
            Some(("total".into(), "$42.00".into()))
        );
        assert_eq!(split_inline_label_value("no colon here"), None);
        assert_eq!(split_inline_label_value("Trailing:"), None);
    }

    #[test]
    fn collapsed_inline_pairs_are_split_individually() {
        let pairs = extract_inline_label_values(
            "INV-1 Bill To: Papergrid Works Account: AC-492577 Due Date: 2026-07-20 \
             Reference: REF-321168 Total Due: $282,856 Payment terms: Net 30",
        );
        let got: Vec<(&str, &str)> = pairs
            .iter()
            .map(|pair| (pair.label.as_str(), pair.value.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("bill to", "Papergrid Works"),
                ("account", "AC-492577"),
                ("due date", "2026-07-20"),
                ("reference", "REF-321168"),
                ("total due", "$282,856"),
            ]
        );
    }

    #[test]
    fn table_label_rows_pair_with_value_rows_by_column() {
        let label_row = vec!["PASSENGER".into(), "FROM".into(), "TO".into(), "".into()];
        let value_row = vec![
            "Tomas Novak".into(),
            "Chennai".into(),
            "Chennai".into(),
            "".into(),
        ];
        let pairs = table_below_pairs(&label_row, &value_row);
        let got: Vec<(&str, &str)> = pairs
            .iter()
            .map(|pair| (pair.label.as_str(), pair.value.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("PASSENGER", "Tomas Novak"),
                ("FROM", "Chennai"),
                ("TO", "Chennai"),
            ]
        );
    }

    #[test]
    fn compact_boarding_pass_cell_is_partitioned() {
        let pairs = extract_compact_label_grid(
            "GEN-000015 Boarding Pass PASSENGER FROM TO Hassan Farouk Cebu Hue \
             DATE SEAT GATE 2026-01-26 16E C19 FLIGHT CLASS BOOKING RV5200 Business \
             BK-946787 corpus document 000015 page 1 of 1",
        );
        let got: Vec<(&str, &str)> = pairs
            .iter()
            .map(|pair| (pair.label.as_str(), pair.value.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("passenger", "Hassan Farouk"),
                ("from", "Cebu"),
                ("to", "Hue"),
                ("date", "2026-01-26"),
                ("seat", "16E"),
                ("gate", "C19"),
                ("flight", "RV5200"),
                ("class", "Business"),
                ("booking", "BK-946787"),
            ]
        );
    }

    #[test]
    fn hint_mapping() {
        assert_eq!(hint_for_label("Invoice Date"), ValueHint::Date);
        assert_eq!(hint_for_label("Due"), ValueHint::Date);
        assert_eq!(hint_for_label("Total"), ValueHint::Amount);
        assert_eq!(hint_for_label("Email"), ValueHint::Email);
        assert_eq!(hint_for_label("Vendor"), ValueHint::Any);
    }
}
