//! Provenance-aware semantic chunking for Prompt 15.
//!
//! This module is additive to [`crate::chunk`]. The established chunk schema is
//! preserved for existing consumers; this versioned model adds table/cell,
//! structure-tree, MCID, ParentTree, CJK dictionary, and security evidence.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::chunk::estimate_tokens;
use crate::parse::{serialize_block_markdown, Block, BlockKind, Document};
use crate::semantic::{SemanticDocument, SemanticElement};
use crate::semantic_intelligence::{ParentTreeRecoveryReport, ParentTreeRecoveryStatus};
use crate::text::{
    segment_cjk_dictionary_text_with_provider, CjkDictionaryMetadata, CjkDictionaryProvider,
    TextQuad, TextSemanticDocument,
};

pub const ADVANCED_RAG_CHUNK_SCHEMA_VERSION: &str = "prompt15.rag_chunk.v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdvancedChunkMode {
    Hybrid,
    Page,
    Section,
    Paragraph,
    Table,
    TableRow,
    TableCell,
    FigureCaption,
    CjkTokenAware,
    SearchIndex,
}

impl AdvancedChunkMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "hybrid" => Some(Self::Hybrid),
            "page" | "pages" => Some(Self::Page),
            "section" | "sections" => Some(Self::Section),
            "paragraph" | "paragraphs" => Some(Self::Paragraph),
            "table" | "tables" => Some(Self::Table),
            "table_row" | "row" | "rows" => Some(Self::TableRow),
            "table_cell" | "cell" | "cells" => Some(Self::TableCell),
            "figure_caption" | "figure" | "figures" => Some(Self::FigureCaption),
            "cjk_token_aware" | "cjk" => Some(Self::CjkTokenAware),
            "search_index" | "search" => Some(Self::SearchIndex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TableChunkSerialization {
    Markdown,
    Json,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdvancedChunkOptions {
    pub mode: AdvancedChunkMode,
    pub target_tokens: usize,
    pub overlap_tokens: usize,
    pub include_heading_context: bool,
    pub include_furniture: bool,
    pub cjk_token_aware: bool,
    pub table_serialization: TableChunkSerialization,
}

impl Default for AdvancedChunkOptions {
    fn default() -> Self {
        Self {
            mode: AdvancedChunkMode::Hybrid,
            target_tokens: 512,
            overlap_tokens: 64,
            include_heading_context: true,
            include_furniture: false,
            cjk_token_aware: false,
            table_serialization: TableChunkSerialization::Both,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkSecurityPosture {
    pub document_state: String,
    pub sanitized: bool,
    pub redaction_applied: bool,
    pub removed_content_included: bool,
    pub hidden_content_warning: bool,
    pub active_content_warning: bool,
    pub signature_status: String,
    pub diagnostics: Vec<String>,
}

impl Default for ChunkSecurityPosture {
    fn default() -> Self {
        Self {
            document_state: "original_input_not_asserted_sanitized".to_string(),
            sanitized: false,
            redaction_applied: false,
            removed_content_included: false,
            hidden_content_warning: false,
            active_content_warning: false,
            signature_status: "not_evaluated_for_chunking".to_string(),
            diagnostics: vec![
                "Chunking original input does not assert that the document was sanitized or redacted"
                    .to_string(),
            ],
        }
    }
}

impl ChunkSecurityPosture {
    pub fn sanitized_after_redaction() -> Self {
        Self {
            document_state: "sanitized_redaction_output".to_string(),
            sanitized: true,
            redaction_applied: true,
            removed_content_included: false,
            hidden_content_warning: false,
            active_content_warning: false,
            signature_status: "signatures_may_be_invalidated_by_rewrite".to_string(),
            diagnostics: vec![
                "Chunks are built from post-redaction bytes; removed content is not reintroduced"
                    .to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RagSourceSpan {
    pub span_id: String,
    pub page: usize,
    pub block_id: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_block_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_span_index: Option<usize>,
    pub bbox: [f64; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quad: Option<[[f64; 2]; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub char_range: Option<[usize; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure_role: Option<String>,
    pub confidence: f32,
    pub provenance: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RagCitation {
    pub page: usize,
    pub bbox: [f64; 4],
    pub block_id: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
    pub source_span_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RagCjkToken {
    pub text: String,
    pub char_range: [usize; 2],
    pub byte_range: [usize; 2],
    pub language: String,
    pub confidence: f32,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RagTableFragment {
    pub table_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub associated_headers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<serde_json::Value>,
    pub merged_cell_preserved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdvancedRagChunk {
    pub chunk_id: String,
    pub index: usize,
    pub chunk_type: String,
    pub mode: AdvancedChunkMode,
    pub page_range: [usize; 2],
    pub pages: Vec<usize>,
    pub text: String,
    pub normalized_text: String,
    pub token_count_estimate: usize,
    pub stable_order: usize,
    pub stable_hash: String,
    pub confidence: f32,
    pub oversized: bool,
    pub overlap_from_previous_tokens: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_spans: Vec<RagSourceSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<RagCitation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bounding_boxes: Vec<[f64; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quads: Vec<[[f64; 2]; 4]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub block_ids: Vec<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_cell_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub figure_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caption_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub heading_section_path: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub structure_tree_path: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcids: Vec<i64>,
    pub parenttree_recovery_status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parenttree_diagnostics: Vec<String>,
    pub cjk_token_layer_enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cjk_tokens: Vec<RagCjkToken>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dictionary_packs: Vec<CjkDictionaryMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_fragments: Vec<RagTableFragment>,
    pub security: ChunkSecurityPosture,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdvancedRagChunkSet {
    pub schema_version: String,
    pub deterministic: bool,
    pub raw_text_rewritten: bool,
    pub options: AdvancedChunkOptions,
    pub title: Option<String>,
    pub dictionary_status: String,
    pub parenttree_recovery_status: String,
    pub security: ChunkSecurityPosture,
    pub chunks: Vec<AdvancedRagChunk>,
    pub diagnostics: Vec<String>,
}

#[derive(Default)]
pub struct AdvancedChunkContext<'a> {
    pub text_semantic: Option<&'a TextSemanticDocument>,
    pub semantic_document: Option<&'a SemanticDocument>,
    pub parenttree: Option<&'a ParentTreeRecoveryReport>,
    pub dictionary: Option<&'a CjkDictionaryProvider>,
    pub security: ChunkSecurityPosture,
}

#[derive(Debug, Clone)]
struct ChunkUnit {
    text: String,
    page: usize,
    bbox: [f64; 4],
    kind: String,
    block_id: usize,
    confidence: f32,
    section_path: Vec<String>,
    table_ids: Vec<String>,
    table_cell_ids: Vec<String>,
    figure_ids: Vec<String>,
    caption_ids: Vec<String>,
    table_fragments: Vec<RagTableFragment>,
    atomic: bool,
}

pub fn advanced_chunk_document(
    document: &Document,
    options: &AdvancedChunkOptions,
    context: &AdvancedChunkContext<'_>,
) -> AdvancedRagChunkSet {
    let target = options.target_tokens.max(1);
    let base_units = collect_units(document, options);
    let units = specialize_units(document, base_units, options);
    let groups = group_units(&units, options, context.dictionary, target);
    let parenttree_status = context
        .parenttree
        .map(|report| parenttree_status_name(report.status))
        .unwrap_or_else(|| "not_requested".to_string());
    let dictionary_status = context
        .dictionary
        .map(|provider| provider.report().provider_status.clone())
        .unwrap_or_else(|| "disabled".to_string());

    let mut chunks = Vec::new();
    let mut previous_group: Option<Vec<ChunkUnit>> = None;
    for (index, group) in groups.into_iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        let overlap_from_previous_tokens = actual_overlap_tokens(previous_group.as_deref(), &group);
        chunks.push(build_chunk(
            index,
            &group,
            options,
            context,
            &parenttree_status,
            overlap_from_previous_tokens,
        ));
        previous_group = Some(group);
    }
    for (index, chunk) in chunks.iter_mut().enumerate() {
        chunk.index = index;
        chunk.stable_order = index;
        chunk.stable_hash = chunk_hash(chunk);
        chunk.chunk_id = format!("chunk-{index}-{}", &chunk.stable_hash[7..23]);
    }

    AdvancedRagChunkSet {
        schema_version: ADVANCED_RAG_CHUNK_SCHEMA_VERSION.to_string(),
        deterministic: true,
        raw_text_rewritten: false,
        options: options.clone(),
        title: document.metadata.title.clone(),
        dictionary_status,
        parenttree_recovery_status: parenttree_status,
        security: context.security.clone(),
        chunks,
        diagnostics: vec![
            "Model proposals are not used to delete or rewrite deterministic chunk text"
                .to_string(),
        ],
    }
}

impl Document {
    pub fn advanced_chunks(
        &self,
        options: &AdvancedChunkOptions,
        context: &AdvancedChunkContext<'_>,
    ) -> AdvancedRagChunkSet {
        advanced_chunk_document(self, options, context)
    }
}

fn collect_units(document: &Document, options: &AdvancedChunkOptions) -> Vec<ChunkUnit> {
    let mut units = Vec::new();
    let mut headings: Vec<(u8, String)> = Vec::new();
    for block in &document.body {
        if block.kind.is_furniture() && !options.include_furniture {
            continue;
        }
        if let Some((level, text)) = heading(block) {
            headings.retain(|(existing, _)| *existing < level);
            headings.push((level, text));
        }
        if attached_caption(block, document) {
            continue;
        }
        let text = serialize_block_markdown(block, document);
        if text.trim().is_empty() {
            continue;
        }
        let section_path = headings.iter().map(|(_, text)| text.clone()).collect();
        let mut table_ids = Vec::new();
        let mut table_cell_ids = Vec::new();
        let mut figure_ids = Vec::new();
        let mut caption_ids = Vec::new();
        let mut table_fragments = Vec::new();
        let (kind, atomic) = match &block.kind {
            BlockKind::Table { table, caption } => {
                let table_id = format!("table-{}", block.id);
                table_ids.push(table_id.clone());
                table_cell_ids.extend(table_cell_ids_for(table, &table_id));
                if let Some(caption) = caption {
                    caption_ids.push(format!("caption-{caption}"));
                }
                table_fragments.push(table_fragment(
                    table,
                    table_id,
                    None,
                    None,
                    options.table_serialization,
                ));
                ("table".to_string(), true)
            }
            BlockKind::Figure { caption, .. } => {
                figure_ids.push(format!("figure-{}", block.id));
                if let Some(caption) = caption {
                    caption_ids.push(format!("caption-{caption}"));
                }
                ("figure_caption".to_string(), true)
            }
            BlockKind::Caption { .. } => {
                caption_ids.push(format!("caption-{}", block.id));
                ("caption".to_string(), false)
            }
            BlockKind::Title { .. } => ("title".to_string(), false),
            BlockKind::Heading { .. } => ("heading".to_string(), false),
            BlockKind::Paragraph { .. } => ("paragraph".to_string(), false),
            BlockKind::List { .. } => ("list".to_string(), false),
            BlockKind::Header { .. } => ("header".to_string(), false),
            BlockKind::Footer { .. } => ("footer".to_string(), false),
            BlockKind::PageNumber { .. } => ("page_number".to_string(), false),
            BlockKind::Text { .. } => ("text".to_string(), false),
        };
        units.push(ChunkUnit {
            text,
            page: block.page as usize,
            bbox: block.bbox,
            kind,
            block_id: block.id,
            confidence: block.confidence,
            section_path,
            table_ids,
            table_cell_ids,
            figure_ids,
            caption_ids,
            table_fragments,
            atomic,
        });
    }
    units
}

fn specialize_units(
    document: &Document,
    base: Vec<ChunkUnit>,
    options: &AdvancedChunkOptions,
) -> Vec<ChunkUnit> {
    match options.mode {
        AdvancedChunkMode::TableRow => table_row_units(document, &base, options),
        AdvancedChunkMode::TableCell => table_cell_units(document, &base, options),
        AdvancedChunkMode::Table => base
            .into_iter()
            .filter(|unit| unit.kind == "table")
            .collect(),
        AdvancedChunkMode::FigureCaption => base
            .into_iter()
            .filter(|unit| unit.kind == "figure_caption" || unit.kind == "caption")
            .collect(),
        AdvancedChunkMode::Paragraph => base
            .into_iter()
            .filter(|unit| matches!(unit.kind.as_str(), "paragraph" | "text" | "list"))
            .collect(),
        _ => base,
    }
}

fn table_row_units(
    document: &Document,
    base: &[ChunkUnit],
    options: &AdvancedChunkOptions,
) -> Vec<ChunkUnit> {
    let mut out = Vec::new();
    for unit in base.iter().filter(|unit| unit.kind == "table") {
        let Some(block) = document.block(unit.block_id) else {
            continue;
        };
        let BlockKind::Table { table, .. } = &block.kind else {
            continue;
        };
        let table_id = format!("table-{}", block.id);
        let headers = table_headers(table);
        for (row_index, row) in table.rows.iter().enumerate() {
            let text = row.join(" | ");
            if text.trim().is_empty() {
                continue;
            }
            let cell_ids = row
                .iter()
                .enumerate()
                .map(|(column, _)| format!("{table_id}-cell-{row_index}-{column}"))
                .collect();
            out.push(ChunkUnit {
                text,
                page: unit.page,
                bbox: row_bbox(table, row_index).unwrap_or(unit.bbox),
                kind: "table_row".to_string(),
                block_id: unit.block_id,
                confidence: unit.confidence,
                section_path: unit.section_path.clone(),
                table_ids: vec![table_id.clone()],
                table_cell_ids: cell_ids,
                figure_ids: Vec::new(),
                caption_ids: unit.caption_ids.clone(),
                table_fragments: vec![table_fragment(
                    table,
                    table_id.clone(),
                    Some(row_index),
                    None,
                    options.table_serialization,
                )
                .with_headers(headers.clone())],
                atomic: true,
            });
        }
    }
    out
}

fn table_cell_units(
    document: &Document,
    base: &[ChunkUnit],
    options: &AdvancedChunkOptions,
) -> Vec<ChunkUnit> {
    let mut out = Vec::new();
    for unit in base.iter().filter(|unit| unit.kind == "table") {
        let Some(block) = document.block(unit.block_id) else {
            continue;
        };
        let BlockKind::Table { table, .. } = &block.kind else {
            continue;
        };
        let table_id = format!("table-{}", block.id);
        let headers = table_headers(table);
        if table.cells.is_empty() {
            for (row_index, row) in table.rows.iter().enumerate() {
                for (column, text) in row.iter().enumerate() {
                    let cell_id = format!("{table_id}-cell-{row_index}-{column}");
                    out.push(cell_unit(
                        unit,
                        table,
                        table_id.clone(),
                        cell_id,
                        row_index,
                        column,
                        text.clone(),
                        unit.bbox,
                        headers.clone(),
                        options.table_serialization,
                    ));
                }
            }
        } else {
            let mut cells: Vec<_> = table.cells.iter().collect();
            cells.sort_by_key(|cell| (cell.row, cell.col));
            for cell in cells {
                let cell_id = format!("{table_id}-cell-{}-{}", cell.row, cell.col);
                out.push(cell_unit(
                    unit,
                    table,
                    table_id.clone(),
                    cell_id,
                    cell.row,
                    cell.col,
                    cell.text.clone(),
                    cell.bbox,
                    headers.clone(),
                    options.table_serialization,
                ));
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn cell_unit(
    parent: &ChunkUnit,
    table: &crate::analysis::tables::Table,
    table_id: String,
    cell_id: String,
    row: usize,
    column: usize,
    text: String,
    bbox: [f64; 4],
    headers: Vec<String>,
    serialization: TableChunkSerialization,
) -> ChunkUnit {
    ChunkUnit {
        text,
        page: parent.page,
        bbox: if bbox == [0.0; 4] { parent.bbox } else { bbox },
        kind: "table_cell".to_string(),
        block_id: parent.block_id,
        confidence: parent.confidence,
        section_path: parent.section_path.clone(),
        table_ids: vec![table_id.clone()],
        table_cell_ids: vec![cell_id.clone()],
        figure_ids: Vec::new(),
        caption_ids: parent.caption_ids.clone(),
        table_fragments: vec![table_fragment(
            table,
            table_id,
            Some(row),
            Some((cell_id, column)),
            serialization,
        )
        .with_headers(headers)],
        atomic: true,
    }
}

trait TableFragmentHeaders {
    fn with_headers(self, headers: Vec<String>) -> Self;
}

impl TableFragmentHeaders for RagTableFragment {
    fn with_headers(mut self, headers: Vec<String>) -> Self {
        self.associated_headers = headers;
        self
    }
}

fn group_units(
    units: &[ChunkUnit],
    options: &AdvancedChunkOptions,
    dictionary: Option<&CjkDictionaryProvider>,
    target: usize,
) -> Vec<Vec<ChunkUnit>> {
    match options.mode {
        AdvancedChunkMode::Page => group_by_key(units, |unit| unit.page.to_string()),
        AdvancedChunkMode::Section => group_by_key(units, |unit| unit.section_path.join("\u{1f}")),
        AdvancedChunkMode::Paragraph
        | AdvancedChunkMode::Table
        | AdvancedChunkMode::TableRow
        | AdvancedChunkMode::TableCell
        | AdvancedChunkMode::FigureCaption => {
            units.iter().cloned().map(|unit| vec![unit]).collect()
        }
        _ => pack_hybrid(units, options, dictionary, target),
    }
}

fn group_by_key<F>(units: &[ChunkUnit], key: F) -> Vec<Vec<ChunkUnit>>
where
    F: Fn(&ChunkUnit) -> String,
{
    let mut groups = Vec::new();
    let mut current_key: Option<String> = None;
    let mut current = Vec::new();
    for unit in units {
        let next_key = key(unit);
        if current_key.as_ref().is_some_and(|value| *value != next_key) && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        current_key = Some(next_key);
        current.push(unit.clone());
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn pack_hybrid(
    units: &[ChunkUnit],
    options: &AdvancedChunkOptions,
    dictionary: Option<&CjkDictionaryProvider>,
    target: usize,
) -> Vec<Vec<ChunkUnit>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    let overlap = if options.mode == AdvancedChunkMode::SearchIndex {
        0
    } else {
        options.overlap_tokens
    };
    for unit in units {
        if unit.atomic {
            flush_group(&mut groups, &mut current, overlap);
            groups.push(vec![unit.clone()]);
            continue;
        }
        let unit_tokens = estimate_tokens(&unit.text);
        if unit_tokens > target {
            flush_group(&mut groups, &mut current, overlap);
            let use_dictionary =
                options.cjk_token_aware || options.mode == AdvancedChunkMode::CjkTokenAware;
            let split =
                split_oversized_unit(unit, target, use_dictionary.then_some(dictionary).flatten());
            groups.extend(split.into_iter().map(|item| vec![item]));
            continue;
        }
        let current_tokens: usize = current
            .iter()
            .map(|item: &ChunkUnit| estimate_tokens(&item.text))
            .sum();
        if current_tokens + unit_tokens > target && !current.is_empty() {
            flush_group(&mut groups, &mut current, overlap);
        }
        current.push(unit.clone());
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn flush_group(groups: &mut Vec<Vec<ChunkUnit>>, current: &mut Vec<ChunkUnit>, overlap: usize) {
    if current.is_empty() {
        return;
    }
    groups.push(current.clone());
    if overlap == 0 {
        current.clear();
        return;
    }
    let mut carry = Vec::new();
    let mut tokens = 0usize;
    for unit in current.iter().rev() {
        if unit.atomic {
            break;
        }
        let cost = estimate_tokens(&unit.text);
        if tokens + cost > overlap && !carry.is_empty() {
            break;
        }
        carry.push(unit.clone());
        tokens += cost;
        if tokens >= overlap {
            break;
        }
    }
    carry.reverse();
    *current = carry;
}

fn split_oversized_unit(
    unit: &ChunkUnit,
    target: usize,
    dictionary: Option<&CjkDictionaryProvider>,
) -> Vec<ChunkUnit> {
    if let Some(provider) = dictionary {
        let tokens = segment_cjk_dictionary_text_with_provider(&unit.text, provider);
        if tokens.len() > 1 {
            let mut out = Vec::new();
            let mut start = 0usize;
            let mut estimated = 0usize;
            for token in &tokens {
                let cost = estimate_tokens(&token.text).max(1);
                if estimated + cost > target && token.char_range[0] > start {
                    out.push(unit_with_text(
                        unit,
                        slice_chars(&unit.text, start, token.char_range[0]),
                    ));
                    start = token.char_range[0];
                    estimated = 0;
                }
                estimated += cost;
            }
            let char_len = unit.text.chars().count();
            if start < char_len {
                out.push(unit_with_text(
                    unit,
                    slice_chars(&unit.text, start, char_len),
                ));
            }
            if out.len() > 1 {
                return out;
            }
        }
    }

    let words: Vec<&str> = unit.text.split_whitespace().collect();
    if words.len() <= 1 {
        return vec![unit.clone()];
    }
    let mut out = Vec::new();
    let mut buffer = String::new();
    for word in words {
        let candidate = if buffer.is_empty() {
            word.to_string()
        } else {
            format!("{buffer} {word}")
        };
        if estimate_tokens(&candidate) > target && !buffer.is_empty() {
            out.push(unit_with_text(unit, std::mem::take(&mut buffer)));
        }
        if !buffer.is_empty() {
            buffer.push(' ');
        }
        buffer.push_str(word);
    }
    if !buffer.is_empty() {
        out.push(unit_with_text(unit, buffer));
    }
    out
}

fn unit_with_text(unit: &ChunkUnit, text: String) -> ChunkUnit {
    let mut out = unit.clone();
    out.text = text;
    out
}

fn build_chunk(
    index: usize,
    units: &[ChunkUnit],
    options: &AdvancedChunkOptions,
    context: &AdvancedChunkContext<'_>,
    parenttree_status: &str,
    overlap_from_previous_tokens: usize,
) -> AdvancedRagChunk {
    let section_path = units
        .last()
        .map(|unit| unit.section_path.clone())
        .unwrap_or_default();
    let body = units
        .iter()
        .map(|unit| unit.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let add_heading_context = options.include_heading_context
        && section_path
            .last()
            .is_some_and(|heading| !body.starts_with(heading));
    let text = if add_heading_context {
        format!("{}\n\n{body}", section_path.join(" > "))
    } else {
        body
    };
    let pages = sorted_unique(units.iter().map(|unit| unit.page));
    let page_range = [
        pages.first().copied().unwrap_or(0),
        pages.last().copied().unwrap_or(0),
    ];
    let block_ids = sorted_unique(units.iter().map(|unit| unit.block_id));
    let bounding_boxes = sorted_unique_bbox(
        units
            .iter()
            .map(|unit| unit.bbox)
            .filter(|bbox| *bbox != [0.0; 4]),
    );
    let mut source_spans = source_spans_for_units(units, context.text_semantic);
    source_spans.sort_by(|left, right| {
        left.page
            .cmp(&right.page)
            .then_with(|| left.block_id.cmp(&right.block_id))
            .then_with(|| left.span_id.cmp(&right.span_id))
    });
    source_spans.dedup_by(|left, right| left.span_id == right.span_id);
    let mcids = sorted_unique(
        source_spans
            .iter()
            .flat_map(|span| span.mcids.iter().copied()),
    );
    let quads = source_spans
        .iter()
        .filter_map(|span| span.quad)
        .collect::<Vec<_>>();
    let citations = citations_for_spans(&source_spans);
    let structure_tree_path = structure_path_for_units(units, context.semantic_document);
    let parenttree_diagnostics = context
        .parenttree
        .map(|report| {
            report
                .diagnostics
                .iter()
                .filter(|item| {
                    item.page.is_none() || item.page.is_some_and(|page| pages.contains(&page))
                })
                .map(|item| item.code.clone())
                .collect()
        })
        .unwrap_or_default();
    let cjk_tokens = context
        .dictionary
        .map(|provider| {
            segment_cjk_dictionary_text_with_provider(&text, provider)
                .into_iter()
                .map(|token| RagCjkToken {
                    text: token.text,
                    char_range: token.char_range,
                    byte_range: token.byte_range,
                    language: token.language,
                    confidence: token.confidence,
                    source: token.source,
                })
                .collect()
        })
        .unwrap_or_default();
    let dictionary_packs = context
        .dictionary
        .map(|provider| provider.metadata().to_vec())
        .unwrap_or_default();
    let table_ids =
        sorted_unique_string(units.iter().flat_map(|unit| unit.table_ids.iter().cloned()));
    let table_cell_ids = sorted_unique_string(
        units
            .iter()
            .flat_map(|unit| unit.table_cell_ids.iter().cloned()),
    );
    let figure_ids = sorted_unique_string(
        units
            .iter()
            .flat_map(|unit| unit.figure_ids.iter().cloned()),
    );
    let caption_ids = sorted_unique_string(
        units
            .iter()
            .flat_map(|unit| unit.caption_ids.iter().cloned()),
    );
    let table_fragments = units
        .iter()
        .flat_map(|unit| unit.table_fragments.iter().cloned())
        .collect();
    let confidence = if units.is_empty() {
        0.0
    } else {
        units.iter().map(|unit| unit.confidence).sum::<f32>() / units.len() as f32
    };
    let token_count_estimate = estimate_tokens(&text);
    let chunk_type = chunk_type(units, options.mode);
    AdvancedRagChunk {
        chunk_id: String::new(),
        index,
        chunk_type,
        mode: options.mode,
        page_range,
        pages,
        normalized_text: normalize_text(&text),
        text,
        token_count_estimate,
        stable_order: index,
        stable_hash: String::new(),
        confidence,
        oversized: token_count_estimate > options.target_tokens.max(1),
        overlap_from_previous_tokens,
        source_spans,
        citations,
        bounding_boxes,
        quads,
        block_ids,
        table_ids,
        table_cell_ids,
        figure_ids,
        caption_ids,
        heading_section_path: section_path,
        structure_tree_path,
        mcids,
        parenttree_recovery_status: parenttree_status.to_string(),
        parenttree_diagnostics,
        cjk_token_layer_enabled: context.dictionary.is_some(),
        cjk_tokens,
        dictionary_packs,
        table_fragments,
        security: context.security.clone(),
        diagnostics: Vec::new(),
    }
}

fn source_spans_for_units(
    units: &[ChunkUnit],
    semantic: Option<&TextSemanticDocument>,
) -> Vec<RagSourceSpan> {
    let mut spans = Vec::new();
    for unit in units {
        spans.push(RagSourceSpan {
            span_id: format!("page-{}-block-{}", unit.page, unit.block_id),
            page: unit.page,
            block_id: unit.block_id,
            semantic_block_index: None,
            line_index: None,
            semantic_span_index: None,
            bbox: unit.bbox,
            quad: None,
            char_range: None,
            mcids: Vec::new(),
            structure_role: None,
            confidence: unit.confidence,
            provenance: vec!["canonical_document_block".to_string()],
        });
        let Some(page) =
            semantic.and_then(|document| document.pages.iter().find(|page| page.page == unit.page))
        else {
            continue;
        };
        for semantic_block in &page.blocks {
            if unit.bbox != [0.0; 4] && !bbox_intersects(unit.bbox, quad_bbox(semantic_block.quad))
            {
                continue;
            }
            for line in &semantic_block.lines {
                for span in &line.spans {
                    if unit.bbox != [0.0; 4] && !bbox_intersects(unit.bbox, quad_bbox(span.quad)) {
                        continue;
                    }
                    spans.push(RagSourceSpan {
                        span_id: format!(
                            "page-{}-block-{}-semantic-{}-line-{}-span-{}",
                            unit.page,
                            unit.block_id,
                            semantic_block.block_index,
                            line.line_index,
                            span.span_index
                        ),
                        page: unit.page,
                        block_id: unit.block_id,
                        semantic_block_index: Some(semantic_block.block_index),
                        line_index: Some(line.line_index),
                        semantic_span_index: Some(span.span_index),
                        bbox: quad_bbox(span.quad),
                        quad: Some(quad_points(span.quad)),
                        char_range: Some(span.char_range),
                        mcids: span.mcids.clone(),
                        structure_role: span.struct_role.clone(),
                        confidence: span.confidence,
                        provenance: span.provenance.iter().map(debug_name).collect(),
                    });
                }
            }
        }
    }
    spans
}

fn citations_for_spans(spans: &[RagSourceSpan]) -> Vec<RagCitation> {
    let mut grouped: BTreeMap<(usize, usize), Vec<&RagSourceSpan>> = BTreeMap::new();
    for span in spans {
        grouped
            .entry((span.page, span.block_id))
            .or_default()
            .push(span);
    }
    grouped
        .into_iter()
        .map(|((page, block_id), spans)| RagCitation {
            page,
            bbox: union_bbox(spans.iter().map(|span| span.bbox)).unwrap_or([0.0; 4]),
            block_id,
            mcids: sorted_unique(spans.iter().flat_map(|span| span.mcids.iter().copied())),
            source_span_ids: spans.iter().map(|span| span.span_id.clone()).collect(),
        })
        .collect()
}

fn structure_path_for_units(
    units: &[ChunkUnit],
    semantic: Option<&SemanticDocument>,
) -> Vec<String> {
    let Some(document) = semantic else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for unit in units {
        for element in &document.elements {
            collect_structure_paths(
                element,
                unit.page,
                unit.bbox,
                &mut Vec::new(),
                &mut candidates,
            );
        }
    }
    candidates.sort_by(|left: &Vec<String>, right: &Vec<String>| {
        right.len().cmp(&left.len()).then_with(|| left.cmp(right))
    });
    candidates.into_iter().next().unwrap_or_default()
}

fn collect_structure_paths(
    element: &SemanticElement,
    page: usize,
    bbox: [f64; 4],
    prefix: &mut Vec<String>,
    out: &mut Vec<Vec<String>>,
) {
    prefix.push(element.element_type.clone());
    let page_matches = element.page.is_none() || element.page == Some(page);
    let bbox_matches = bbox == [0.0; 4]
        || element.bbox.is_none()
        || element
            .bbox
            .is_some_and(|element_bbox| bbox_intersects(bbox, element_bbox));
    if page_matches && bbox_matches {
        out.push(prefix.clone());
    }
    for child in &element.children {
        collect_structure_paths(child, page, bbox, prefix, out);
    }
    prefix.pop();
}

fn table_fragment(
    table: &crate::analysis::tables::Table,
    table_id: String,
    row: Option<usize>,
    cell: Option<(String, usize)>,
    serialization: TableChunkSerialization,
) -> RagTableFragment {
    let (markdown, json) = match serialization {
        TableChunkSerialization::Markdown => {
            (Some(table_markdown(table, row, cell.as_ref())), None)
        }
        TableChunkSerialization::Json => (None, Some(table_json(table, row, cell.as_ref()))),
        TableChunkSerialization::Both => (
            Some(table_markdown(table, row, cell.as_ref())),
            Some(table_json(table, row, cell.as_ref())),
        ),
    };
    RagTableFragment {
        table_id,
        row,
        cell_id: cell.map(|(id, _)| id),
        associated_headers: Vec::new(),
        markdown,
        json,
        merged_cell_preserved: true,
    }
}

fn table_markdown(
    table: &crate::analysis::tables::Table,
    row: Option<usize>,
    cell: Option<&(String, usize)>,
) -> String {
    if let Some((_, column)) = cell {
        return row
            .and_then(|row| table.rows.get(row))
            .and_then(|row| row.get(*column))
            .cloned()
            .unwrap_or_default();
    }
    if let Some(row) = row {
        return table
            .rows
            .get(row)
            .map(|row| format!("| {} |", row.join(" | ")))
            .unwrap_or_default();
    }
    let mut out = String::new();
    for row in &table.rows {
        out.push_str("| ");
        out.push_str(&row.join(" | "));
        out.push_str(" |\n");
    }
    out
}

fn table_json(
    table: &crate::analysis::tables::Table,
    row: Option<usize>,
    cell: Option<&(String, usize)>,
) -> serde_json::Value {
    if let Some((cell_id, column)) = cell {
        return serde_json::json!({
            "cell_id": cell_id,
            "row": row,
            "column": column,
            "text": row.and_then(|row| table.rows.get(row)).and_then(|row| row.get(*column)),
            "origin_cell": table.cells.iter().find(|item| Some(item.row) == row && item.col == *column),
        });
    }
    if let Some(row) = row {
        return serde_json::json!({
            "row": row,
            "cells": table.rows.get(row),
            "origin_cells": table.cells.iter().filter(|item| item.row == row).collect::<Vec<_>>(),
        });
    }
    serde_json::to_value(table).unwrap_or(serde_json::Value::Null)
}

fn table_cell_ids_for(table: &crate::analysis::tables::Table, table_id: &str) -> Vec<String> {
    if table.cells.is_empty() {
        return table
            .rows
            .iter()
            .enumerate()
            .flat_map(|(row, cells)| {
                cells
                    .iter()
                    .enumerate()
                    .map(move |(column, _)| format!("{table_id}-cell-{row}-{column}"))
            })
            .collect();
    }
    let mut ids: Vec<String> = table
        .cells
        .iter()
        .map(|cell| format!("{table_id}-cell-{}-{}", cell.row, cell.col))
        .collect();
    ids.sort();
    ids
}

fn table_headers(table: &crate::analysis::tables::Table) -> Vec<String> {
    let mut headers: Vec<String> = table
        .cells
        .iter()
        .filter(|cell| cell.is_header && !cell.text.trim().is_empty())
        .map(|cell| cell.text.clone())
        .collect();
    if headers.is_empty() {
        headers = table.rows.first().cloned().unwrap_or_default();
    }
    sorted_unique_string(headers)
}

fn row_bbox(table: &crate::analysis::tables::Table, row: usize) -> Option<[f64; 4]> {
    union_bbox(
        table
            .cells
            .iter()
            .filter(|cell| cell.row == row && cell.bbox != [0.0; 4])
            .map(|cell| cell.bbox),
    )
}

fn heading(block: &Block) -> Option<(u8, String)> {
    match &block.kind {
        BlockKind::Title { text } => Some((0, text.to_plain())),
        BlockKind::Heading { level, text } => Some(((*level).max(1), text.to_plain())),
        _ => None,
    }
}

fn attached_caption(block: &Block, document: &Document) -> bool {
    let BlockKind::Caption { target, .. } = &block.kind else {
        return false;
    };
    target
        .and_then(|target| document.block(target))
        .is_some_and(|target| {
            matches!(
                target.kind,
                BlockKind::Figure { .. } | BlockKind::Table { .. }
            )
        })
}

fn chunk_type(units: &[ChunkUnit], mode: AdvancedChunkMode) -> String {
    match mode {
        AdvancedChunkMode::Page => "page".to_string(),
        AdvancedChunkMode::Section => "section".to_string(),
        AdvancedChunkMode::Paragraph => "paragraph".to_string(),
        AdvancedChunkMode::Table => "table".to_string(),
        AdvancedChunkMode::TableRow => "table_row".to_string(),
        AdvancedChunkMode::TableCell => "table_cell".to_string(),
        AdvancedChunkMode::FigureCaption => "figure_caption".to_string(),
        AdvancedChunkMode::CjkTokenAware => "cjk_token_aware".to_string(),
        AdvancedChunkMode::SearchIndex => "search_index".to_string(),
        AdvancedChunkMode::Hybrid => {
            let kinds: BTreeSet<&str> = units.iter().map(|unit| unit.kind.as_str()).collect();
            if kinds.len() == 1 {
                kinds.into_iter().next().unwrap_or("hybrid").to_string()
            } else {
                "hybrid".to_string()
            }
        }
    }
}

fn chunk_hash(chunk: &AdvancedRagChunk) -> String {
    let canonical = serde_json::json!({
        "mode": chunk.mode,
        "pages": chunk.pages,
        "text": chunk.text,
        "block_ids": chunk.block_ids,
        "table_ids": chunk.table_ids,
        "table_cell_ids": chunk.table_cell_ids,
        "figure_ids": chunk.figure_ids,
        "caption_ids": chunk.caption_ids,
        "section": chunk.heading_section_path,
        "structure_tree_path": chunk.structure_tree_path,
        "mcids": chunk.mcids,
        "source_span_ids": chunk.source_spans.iter().map(|span| &span.span_id).collect::<Vec<_>>(),
        "parenttree_recovery_status": chunk.parenttree_recovery_status,
        "dictionary_pack_hashes": chunk.dictionary_packs.iter().map(|pack| &pack.hash).collect::<Vec<_>>(),
        "security": {
            "document_state": chunk.security.document_state,
            "sanitized": chunk.security.sanitized,
            "redaction_applied": chunk.security.redaction_applied,
            "removed_content_included": chunk.security.removed_content_included,
        },
    });
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn actual_overlap_tokens(previous: Option<&[ChunkUnit]>, current: &[ChunkUnit]) -> usize {
    let Some(previous) = previous else {
        return 0;
    };
    current
        .iter()
        .take_while(|current_unit| {
            previous.iter().any(|previous_unit| {
                previous_unit.page == current_unit.page
                    && previous_unit.block_id == current_unit.block_id
                    && previous_unit.kind == current_unit.kind
                    && previous_unit.text == current_unit.text
            })
        })
        .map(|unit| estimate_tokens(&unit.text))
        .sum()
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parenttree_status_name(status: ParentTreeRecoveryStatus) -> String {
    match status {
        ParentTreeRecoveryStatus::NoTaggedEvidence => "no_tagged_evidence",
        ParentTreeRecoveryStatus::StructTreeAvailable => "struct_tree_available",
        ParentTreeRecoveryStatus::RecoveredFromParentTree => "recovered_from_parent_tree",
        ParentTreeRecoveryStatus::RecoveredWithConflicts => "recovered_with_conflicts",
        ParentTreeRecoveryStatus::RecoveredOrphansOnly => "recovered_orphans_only",
        ParentTreeRecoveryStatus::UnsupportedReportedExact => "unsupported_reported_exact",
    }
    .to_string()
}

fn quad_bbox(quad: TextQuad) -> [f64; 4] {
    [quad.x0, quad.y0, quad.x1, quad.y1]
}

fn quad_points(quad: TextQuad) -> [[f64; 2]; 4] {
    [
        [quad.x0, quad.y0],
        [quad.x1, quad.y0],
        [quad.x1, quad.y1],
        [quad.x0, quad.y1],
    ]
}

fn bbox_intersects(left: [f64; 4], right: [f64; 4]) -> bool {
    left[0] < right[2] && left[2] > right[0] && left[1] < right[3] && left[3] > right[1]
}

fn union_bbox<I>(items: I) -> Option<[f64; 4]>
where
    I: IntoIterator<Item = [f64; 4]>,
{
    let mut iter = items.into_iter().filter(|bbox| *bbox != [0.0; 4]);
    let first = iter.next()?;
    Some(iter.fold(first, |mut acc, bbox| {
        acc[0] = acc[0].min(bbox[0]);
        acc[1] = acc[1].min(bbox[1]);
        acc[2] = acc[2].max(bbox[2]);
        acc[3] = acc[3].max(bbox[3]);
        acc
    }))
}

fn sorted_unique<T, I>(items: I) -> Vec<T>
where
    T: Ord,
    I: IntoIterator<Item = T>,
{
    items
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sorted_unique_string<I>(items: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    sorted_unique(items)
}

fn sorted_unique_bbox<I>(items: I) -> Vec<[f64; 4]>
where
    I: IntoIterator<Item = [f64; 4]>,
{
    let mut out: Vec<[f64; 4]> = items.into_iter().collect();
    out.sort_by(|left, right| {
        left[0]
            .total_cmp(&right[0])
            .then_with(|| left[1].total_cmp(&right[1]))
            .then_with(|| left[2].total_cmp(&right[2]))
            .then_with(|| left[3].total_cmp(&right[3]))
    });
    out.dedup();
    out
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn debug_name<T: std::fmt::Debug>(value: &T) -> String {
    let raw = format!("{value:?}");
    let mut out = String::new();
    for (index, ch) in raw.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::tables::{Table, TableCell, TableSource};
    use crate::parse::{Block, DocumentMetadata, InlineText, Page, SourceInfo, SCHEMA_VERSION};

    fn document() -> Document {
        let table = Table {
            rows: vec![
                vec!["Name".to_string(), "Value".to_string()],
                vec!["Machine learning".to_string(), "42".to_string()],
            ],
            cells: vec![
                TableCell {
                    row: 0,
                    col: 0,
                    rowspan: 1,
                    colspan: 1,
                    text: "Name".to_string(),
                    bbox: [10.0, 10.0, 100.0, 30.0],
                    is_header: true,
                    header_scope: None,
                    nested_tables: Vec::new(),
                },
                TableCell {
                    row: 1,
                    col: 0,
                    rowspan: 1,
                    colspan: 2,
                    text: "Machine learning".to_string(),
                    bbox: [10.0, 30.0, 200.0, 50.0],
                    is_header: false,
                    header_scope: None,
                    nested_tables: Vec::new(),
                },
            ],
            header_hierarchy: Vec::new(),
            source: TableSource::Ruled,
            confidence: 0.95,
            bbox: [10.0, 10.0, 200.0, 50.0],
            notes: Vec::new(),
        };
        let blocks = vec![
            Block {
                id: 1,
                page: 1,
                bbox: [10.0, 700.0, 300.0, 730.0],
                reading_order: 0,
                confidence: 0.98,
                kind: BlockKind::Heading {
                    level: 1,
                    text: InlineText::plain("Results"),
                },
            },
            Block {
                id: 2,
                page: 1,
                bbox: [10.0, 650.0, 400.0, 690.0],
                reading_order: 1,
                confidence: 0.93,
                kind: BlockKind::Paragraph {
                    text: InlineText::plain("机器学习 improves retrieval quality."),
                },
            },
            Block {
                id: 3,
                page: 1,
                bbox: table.bbox,
                reading_order: 2,
                confidence: 0.95,
                kind: BlockKind::Table {
                    table,
                    caption: None,
                },
            },
        ];
        Document {
            schema_version: SCHEMA_VERSION.to_string(),
            metadata: DocumentMetadata {
                title: Some("Prompt 15 Fixture".to_string()),
                page_count: 1,
                ..Default::default()
            },
            source: SourceInfo::Tagged,
            body: blocks,
            pages: vec![Page {
                number: 1,
                width: 612.0,
                height: 792.0,
                source: crate::PageSource::DigitalBorn,
                classification: None,
                block_ids: vec![1, 2, 3],
            }],
        }
    }

    #[test]
    fn advanced_chunks_are_stable_and_raw_text_is_preserved() {
        let document = document();
        let provider = CjkDictionaryProvider::builtin_fixture();
        let context = AdvancedChunkContext {
            dictionary: Some(&provider),
            ..Default::default()
        };
        let options = AdvancedChunkOptions {
            cjk_token_aware: true,
            overlap_tokens: 0,
            ..Default::default()
        };
        let left = advanced_chunk_document(&document, &options, &context);
        let right = advanced_chunk_document(&document, &options, &context);
        assert_eq!(left, right);
        assert!(!left.raw_text_rewritten);
        assert!(left
            .chunks
            .iter()
            .any(|chunk| chunk.text.contains("机器学习")));
        assert!(left.chunks.iter().any(|chunk| chunk
            .cjk_tokens
            .iter()
            .any(|token| token.text == "机器学习")));
        assert!(left
            .chunks
            .iter()
            .all(|chunk| chunk.stable_hash.starts_with("sha256:")));
    }

    #[test]
    fn table_row_and_cell_modes_preserve_grid_evidence() {
        let document = document();
        let row_set = advanced_chunk_document(
            &document,
            &AdvancedChunkOptions {
                mode: AdvancedChunkMode::TableRow,
                ..Default::default()
            },
            &AdvancedChunkContext::default(),
        );
        assert_eq!(row_set.chunks.len(), 2);
        assert!(row_set
            .chunks
            .iter()
            .all(|chunk| chunk.chunk_type == "table_row" && !chunk.table_fragments.is_empty()));

        let cell_set = advanced_chunk_document(
            &document,
            &AdvancedChunkOptions {
                mode: AdvancedChunkMode::TableCell,
                ..Default::default()
            },
            &AdvancedChunkContext::default(),
        );
        assert_eq!(cell_set.chunks.len(), 2);
        assert!(cell_set
            .chunks
            .iter()
            .all(|chunk| !chunk.table_cell_ids.is_empty()));
        assert!(cell_set
            .chunks
            .iter()
            .flat_map(|chunk| &chunk.table_fragments)
            .all(|fragment| fragment.merged_cell_preserved));
    }

    #[test]
    fn sanitized_posture_never_claims_removed_content() {
        let set = advanced_chunk_document(
            &document(),
            &AdvancedChunkOptions::default(),
            &AdvancedChunkContext {
                security: ChunkSecurityPosture::sanitized_after_redaction(),
                ..Default::default()
            },
        );
        assert!(set.security.redaction_applied);
        assert!(!set.security.removed_content_included);
        assert!(set
            .chunks
            .iter()
            .all(|chunk| !chunk.security.removed_content_included));
    }

    #[test]
    fn overlap_metadata_reports_repeated_units_not_requested_overlap() {
        let set = advanced_chunk_document(
            &document(),
            &AdvancedChunkOptions {
                mode: AdvancedChunkMode::TableRow,
                overlap_tokens: 64,
                ..Default::default()
            },
            &AdvancedChunkContext::default(),
        );
        assert!(set.chunks.len() > 1);
        assert!(set
            .chunks
            .iter()
            .all(|chunk| chunk.overlap_from_previous_tokens == 0));
    }
}
