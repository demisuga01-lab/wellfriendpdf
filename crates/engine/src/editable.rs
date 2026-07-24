//! Shared editable document model for Prompt 08 conversion and editing.
//!
//! PDF content is fixed-position drawing instructions. This model is the
//! conservative bridge between Wellfriend's semantic extraction/document model and
//! outputs that need editable structure such as DOCX, PPTX, XLSX, HTML,
//! Markdown, JSON, and safe edit planning.

use serde::{Deserialize, Serialize};

use crate::engine::ContentEngine;
use crate::error::Result;
use crate::parse::{Block, BlockKind, Document, InlineSpan, ParseOptions};
use crate::versioning::resource_digest;

pub const EDITABLE_SCHEMA_VERSION: &str = "0.1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableBuildOptions {
    pub pages: Vec<usize>,
    pub include_images: bool,
    pub max_images_per_page: usize,
}

impl Default for EditableBuildOptions {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            include_images: true,
            max_images_per_page: 256,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableDocument {
    pub schema_version: String,
    pub source_document: Document,
    pub sections: Vec<EditableSection>,
    pub pages: Vec<EditablePage>,
    pub blocks: Vec<EditableBlock>,
    pub diagnostics: Vec<EditableDiagnostic>,
    pub transactions: EditTransactionLog,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableSection {
    pub id: String,
    pub title: Option<String>,
    pub page_range: [usize; 2],
    pub block_ids: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditablePage {
    pub id: String,
    pub source_page: usize,
    pub width: f64,
    pub height: f64,
    pub block_ids: Vec<String>,
    pub image_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableBlock {
    pub id: String,
    pub source_block_id: usize,
    pub page: usize,
    pub bbox: [f64; 4],
    pub reading_order: u32,
    pub role: EditableRole,
    pub edit_safety: EditSafety,
    pub confidence: f32,
    pub paragraphs: Vec<EditableParagraph>,
    pub table: Option<EditableTable>,
    pub image: Option<EditableImage>,
    pub provenance: EditableProvenance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditableRole {
    Title,
    Heading,
    Paragraph,
    List,
    Table,
    Figure,
    Caption,
    Header,
    Footer,
    PageNumber,
    Text,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditSafety {
    SafePatch,
    LocalReflowRewrite,
    PageRegenerate,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableParagraph {
    pub id: String,
    pub role: EditableRole,
    pub text: String,
    pub runs: Vec<EditableRun>,
    pub list: Option<EditableListInfo>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableRun {
    pub id: String,
    pub text: String,
    pub style: EditableTextStyle,
    pub source_span_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct EditableTextStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    pub font_family: Option<String>,
    pub font_size: Option<f64>,
    pub color: Option<String>,
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableListInfo {
    pub ordered: bool,
    pub marker: Option<String>,
    pub level: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableTable {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<EditableTableCell>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableTableCell {
    pub row: usize,
    pub col: usize,
    pub rowspan: usize,
    pub colspan: usize,
    pub text: String,
    pub is_header: bool,
    pub bbox: [f64; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableImage {
    pub id: String,
    pub source_object: Option<String>,
    pub bbox: [f64; 4],
    pub intrinsic_width: Option<u32>,
    pub intrinsic_height: Option<u32>,
    pub color_space: Option<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableProvenance {
    pub source: String,
    pub source_page: usize,
    pub source_block_id: usize,
    pub confidence: f32,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableDiagnostic {
    pub code: String,
    pub severity: EditableDiagnosticSeverity,
    pub message: String,
    pub page: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EditableDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditTransactionLog {
    pub entries: Vec<EditTransaction>,
    pub cursor: usize,
    #[serde(default)]
    pub patches: Vec<EditPatch>,
    #[serde(default)]
    pub checkpoints: Vec<EditCheckpoint>,
    #[serde(default = "default_transaction_checkpoint_cap")]
    pub max_checkpoints: usize,
}

impl Default for EditTransactionLog {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            patches: Vec::new(),
            checkpoints: Vec::new(),
            max_checkpoints: default_transaction_checkpoint_cap(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditTransaction {
    pub id: String,
    pub operation: EditOperation,
    pub before_text: Option<String>,
    pub after_text: Option<String>,
    #[serde(default)]
    pub sequence: usize,
    #[serde(default)]
    pub paragraph_id: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditPatch {
    pub sequence: usize,
    pub operation: EditOperation,
    pub target_block_id: String,
    pub target_paragraph_id: Option<String>,
    pub before_text: String,
    pub after_text: String,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditCheckpoint {
    pub sequence: usize,
    pub document_digest: String,
    pub transaction_count: usize,
    pub block_count: usize,
    pub text_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditOperation {
    ReplaceText { block_id: String },
    InsertText { block_id: String, offset: usize },
    DeleteText { block_id: String },
    ReplaceImage { image_id: String },
}

fn default_transaction_checkpoint_cap() -> usize {
    8
}

pub fn build_editable_document(
    engine: &ContentEngine,
    options: &EditableBuildOptions,
) -> Result<EditableDocument> {
    let parse_options = ParseOptions {
        pages: options.pages.clone(),
        ..ParseOptions::default()
    };
    build_editable_document_with_parse_options(engine, &parse_options, options)
}

pub fn build_editable_document_with_parse_options(
    engine: &ContentEngine,
    parse_options: &ParseOptions,
    options: &EditableBuildOptions,
) -> Result<EditableDocument> {
    let source_document = engine.parse_document(parse_options)?;
    Ok(EditableDocument::from_parse_document(
        engine,
        source_document,
        options,
    ))
}

impl EditableDocument {
    pub fn from_parse_document(
        engine: &ContentEngine,
        source_document: Document,
        options: &EditableBuildOptions,
    ) -> Self {
        let mut diagnostics = Vec::new();
        let blocks = source_document
            .body
            .iter()
            .map(block_to_editable)
            .collect::<Vec<_>>();
        let pages = source_document
            .pages
            .iter()
            .map(|page| {
                let image_ids = if options.include_images {
                    page_images(
                        engine,
                        page.number as usize,
                        options.max_images_per_page,
                        &mut diagnostics,
                    )
                } else {
                    Vec::new()
                };
                EditablePage {
                    id: format!("page-{}", page.number),
                    source_page: page.number as usize,
                    width: page.width,
                    height: page.height,
                    block_ids: page
                        .block_ids
                        .iter()
                        .map(|id| format!("block-{id}"))
                        .collect(),
                    image_ids,
                }
            })
            .collect::<Vec<_>>();
        let sections = infer_sections(&source_document, &blocks);
        Self {
            schema_version: EDITABLE_SCHEMA_VERSION.to_string(),
            source_document,
            sections,
            pages,
            blocks,
            diagnostics,
            transactions: EditTransactionLog::default(),
        }
    }

    pub fn to_parse_document(&self) -> Document {
        self.source_document.clone()
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            match block.role {
                EditableRole::Title => {
                    push_markdown_line(&mut out, &format!("# {}", block_text(block)));
                }
                EditableRole::Heading => {
                    push_markdown_line(&mut out, &format!("## {}", block_text(block)));
                }
                EditableRole::List => {
                    for para in &block.paragraphs {
                        let marker = para
                            .list
                            .as_ref()
                            .and_then(|list| list.marker.clone())
                            .unwrap_or_else(|| {
                                if para.list.as_ref().is_some_and(|list| list.ordered) {
                                    "1.".to_string()
                                } else {
                                    "-".to_string()
                                }
                            });
                        push_markdown_line(&mut out, &format!("{marker} {}", para.text));
                    }
                }
                EditableRole::Table => {
                    if let Some(table) = &block.table {
                        out.push_str(&table_to_markdown(table));
                    }
                }
                EditableRole::Figure => {
                    if let Some(image) = &block.image {
                        push_markdown_line(
                            &mut out,
                            &format!("![{}]({})", block_text(block), image.id),
                        );
                    }
                }
                EditableRole::Header | EditableRole::Footer | EditableRole::PageNumber => {}
                EditableRole::Paragraph | EditableRole::Caption | EditableRole::Text => {
                    push_markdown_line(&mut out, &block_text(block))
                }
            }
        }
        out.trim_end().to_string() + "\n"
    }

    pub fn to_semantic_html(&self) -> String {
        let mut out = String::from("<!doctype html>\n<html><head><meta charset=\"utf-8\"><title>Wellfriend Editable Model</title></head><body>\n");
        for block in &self.blocks {
            match block.role {
                EditableRole::Title => {
                    out.push_str(&format!("<h1>{}</h1>\n", html_escape(&block_text(block))))
                }
                EditableRole::Heading => {
                    out.push_str(&format!("<h2>{}</h2>\n", html_escape(&block_text(block))))
                }
                EditableRole::List => {
                    let ordered = block
                        .paragraphs
                        .first()
                        .and_then(|p| p.list.as_ref())
                        .is_some_and(|list| list.ordered);
                    out.push_str(if ordered { "<ol>\n" } else { "<ul>\n" });
                    for para in &block.paragraphs {
                        out.push_str(&format!("<li>{}</li>\n", html_escape(&para.text)));
                    }
                    out.push_str(if ordered { "</ol>\n" } else { "</ul>\n" });
                }
                EditableRole::Table => {
                    if let Some(table) = &block.table {
                        out.push_str(&table_to_html(table));
                    }
                }
                EditableRole::Header | EditableRole::Footer | EditableRole::PageNumber => {}
                _ => out.push_str(&format!("<p>{}</p>\n", html_escape(&block_text(block)))),
            }
        }
        out.push_str("</body></html>\n");
        out
    }

    pub fn replace_block_text(&mut self, block_id: &str, replacement: &str) -> bool {
        let Some(block) = self.blocks.iter_mut().find(|block| block.id == block_id) else {
            return false;
        };
        let before = block_text(block);
        block.paragraphs = vec![EditableParagraph {
            id: format!("{block_id}-p0"),
            role: block.role,
            text: replacement.to_string(),
            runs: vec![EditableRun {
                id: format!("{block_id}-r0"),
                text: replacement.to_string(),
                style: EditableTextStyle::default(),
                source_span_index: 0,
            }],
            list: None,
            confidence: block.confidence,
        }];
        self.record_text_transaction(
            EditOperation::ReplaceText {
                block_id: block_id.to_string(),
            },
            block_id,
            None,
            before,
            replacement.to_string(),
            Vec::new(),
        );
        true
    }

    pub fn replace_paragraph_text(
        &mut self,
        block_id: &str,
        paragraph_id: &str,
        replacement: &str,
    ) -> bool {
        self.splice_paragraph_text(
            block_id,
            paragraph_id,
            0,
            usize::MAX,
            replacement,
            EditOperation::ReplaceText {
                block_id: block_id.to_string(),
            },
        )
    }

    pub fn insert_paragraph_text(
        &mut self,
        block_id: &str,
        paragraph_id: &str,
        offset_chars: usize,
        insertion: &str,
    ) -> bool {
        self.splice_paragraph_text(
            block_id,
            paragraph_id,
            offset_chars,
            offset_chars,
            insertion,
            EditOperation::InsertText {
                block_id: block_id.to_string(),
                offset: offset_chars,
            },
        )
    }

    pub fn delete_paragraph_range(
        &mut self,
        block_id: &str,
        paragraph_id: &str,
        start_chars: usize,
        end_chars: usize,
    ) -> bool {
        self.splice_paragraph_text(
            block_id,
            paragraph_id,
            start_chars,
            end_chars,
            "",
            EditOperation::DeleteText {
                block_id: block_id.to_string(),
            },
        )
    }

    fn splice_paragraph_text(
        &mut self,
        block_id: &str,
        paragraph_id: &str,
        start_chars: usize,
        end_chars: usize,
        replacement: &str,
        operation: EditOperation,
    ) -> bool {
        let Some((block_idx, para_idx)) = self.find_paragraph_index(block_id, paragraph_id) else {
            return false;
        };
        let before = self.blocks[block_idx].paragraphs[para_idx].text.clone();
        let end_chars = end_chars.min(before.chars().count());
        let Some(after) = splice_chars(&before, start_chars.min(end_chars), end_chars, replacement)
        else {
            return false;
        };
        let paragraph = &mut self.blocks[block_idx].paragraphs[para_idx];
        paragraph.text = after.clone();
        paragraph.runs = rebuild_runs_for_text(&paragraph.runs, &after);
        self.record_text_transaction(
            operation,
            block_id,
            Some(paragraph_id.to_string()),
            before,
            after,
            Vec::new(),
        );
        true
    }

    pub fn undo(&mut self) -> bool {
        if self.transactions.cursor == 0 {
            return false;
        }
        self.transactions.cursor -= 1;
        let tx = self.transactions.entries[self.transactions.cursor].clone();
        self.apply_transaction_text(&tx, true)
    }

    pub fn redo(&mut self) -> bool {
        if self.transactions.cursor >= self.transactions.entries.len() {
            return false;
        }
        let tx = self.transactions.entries[self.transactions.cursor].clone();
        self.transactions.cursor += 1;
        self.apply_transaction_text(&tx, false)
    }

    fn apply_transaction_text(&mut self, tx: &EditTransaction, undo: bool) -> bool {
        let block_id = match &tx.operation {
            EditOperation::ReplaceText { block_id }
            | EditOperation::InsertText { block_id, .. }
            | EditOperation::DeleteText { block_id } => block_id,
            EditOperation::ReplaceImage { .. } => return false,
        };
        let text = if undo {
            tx.before_text.as_deref()
        } else {
            tx.after_text.as_deref()
        };
        let Some(text) = text else {
            return false;
        };
        if let Some(paragraph_id) = tx.paragraph_id.as_deref() {
            self.set_paragraph_text(block_id, paragraph_id, text)
        } else if let Some(block) = self.blocks.iter_mut().find(|block| &block.id == block_id) {
            block.paragraphs = vec![EditableParagraph {
                id: format!("{block_id}-p0"),
                role: block.role,
                text: text.to_string(),
                runs: vec![EditableRun {
                    id: format!("{block_id}-r0"),
                    text: text.to_string(),
                    style: EditableTextStyle::default(),
                    source_span_index: 0,
                }],
                list: None,
                confidence: block.confidence,
            }];
            true
        } else {
            false
        }
    }

    fn find_paragraph_index(&self, block_id: &str, paragraph_id: &str) -> Option<(usize, usize)> {
        let block_idx = self.blocks.iter().position(|block| block.id == block_id)?;
        let para_idx = self.blocks[block_idx]
            .paragraphs
            .iter()
            .position(|paragraph| paragraph.id == paragraph_id)?;
        Some((block_idx, para_idx))
    }

    fn set_paragraph_text(&mut self, block_id: &str, paragraph_id: &str, text: &str) -> bool {
        let Some((block_idx, para_idx)) = self.find_paragraph_index(block_id, paragraph_id) else {
            return false;
        };
        let paragraph = &mut self.blocks[block_idx].paragraphs[para_idx];
        paragraph.text = text.to_string();
        paragraph.runs = rebuild_runs_for_text(&paragraph.runs, text);
        true
    }

    fn record_text_transaction(
        &mut self,
        operation: EditOperation,
        block_id: &str,
        paragraph_id: Option<String>,
        before_text: String,
        after_text: String,
        diagnostics: Vec<String>,
    ) {
        self.transactions.entries.truncate(self.transactions.cursor);
        self.transactions.patches.truncate(self.transactions.cursor);
        let sequence = self.transactions.entries.len() + 1;
        let tx_id = format!("tx-{sequence}");
        self.transactions.entries.push(EditTransaction {
            id: tx_id,
            operation: operation.clone(),
            before_text: Some(before_text.clone()),
            after_text: Some(after_text.clone()),
            sequence,
            paragraph_id: paragraph_id.clone(),
            diagnostics: diagnostics.clone(),
        });
        self.transactions.patches.push(EditPatch {
            sequence,
            operation,
            target_block_id: block_id.to_string(),
            target_paragraph_id: paragraph_id,
            before_text,
            after_text,
            diagnostics,
        });
        self.transactions.cursor = self.transactions.entries.len();
        self.capture_edit_checkpoint(sequence);
    }

    fn capture_edit_checkpoint(&mut self, sequence: usize) {
        let digest_input = self
            .blocks
            .iter()
            .map(block_text)
            .collect::<Vec<_>>()
            .join("\n\x1f\n");
        let text_bytes = digest_input.len();
        self.transactions.checkpoints.push(EditCheckpoint {
            sequence,
            document_digest: resource_digest(digest_input.as_bytes()),
            transaction_count: self.transactions.entries.len(),
            block_count: self.blocks.len(),
            text_bytes,
        });
        let cap = self.transactions.max_checkpoints.max(1);
        if self.transactions.checkpoints.len() > cap {
            let drain = self.transactions.checkpoints.len() - cap;
            self.transactions.checkpoints.drain(0..drain);
        }
    }
}

fn splice_chars(
    input: &str,
    start_chars: usize,
    end_chars: usize,
    replacement: &str,
) -> Option<String> {
    if start_chars > end_chars {
        return None;
    }
    let start = char_to_byte_index(input, start_chars)?;
    let end = char_to_byte_index(input, end_chars)?;
    let mut out = String::with_capacity(input.len() + replacement.len());
    out.push_str(&input[..start]);
    out.push_str(replacement);
    out.push_str(&input[end..]);
    Some(out)
}

fn char_to_byte_index(input: &str, char_index: usize) -> Option<usize> {
    if char_index == input.chars().count() {
        return Some(input.len());
    }
    input.char_indices().nth(char_index).map(|(idx, _)| idx)
}

fn rebuild_runs_for_text(existing: &[EditableRun], text: &str) -> Vec<EditableRun> {
    if existing.is_empty() {
        return vec![EditableRun {
            id: "run-0".to_string(),
            text: text.to_string(),
            style: EditableTextStyle::default(),
            source_span_index: 0,
        }];
    }
    if existing.len() == 1 {
        let mut run = existing[0].clone();
        run.text = text.to_string();
        return vec![run];
    }
    let chars = text.chars().collect::<Vec<_>>();
    let total_chars = chars.len();
    if total_chars == 0 {
        let mut run = existing[0].clone();
        run.text.clear();
        return vec![run];
    }
    let old_total = existing
        .iter()
        .map(|run| run.text.chars().count())
        .sum::<usize>()
        .max(1);
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (idx, run) in existing.iter().enumerate() {
        let remaining_runs = existing.len() - idx;
        let remaining_chars = total_chars.saturating_sub(cursor);
        let take = if idx + 1 == existing.len() {
            remaining_chars
        } else {
            let proportional = ((run.text.chars().count() * total_chars) / old_total).max(1);
            proportional.min(remaining_chars.saturating_sub(remaining_runs - 1))
        };
        let mut rebuilt = run.clone();
        rebuilt.text = chars[cursor..cursor + take].iter().collect();
        cursor += take;
        if !rebuilt.text.is_empty() {
            out.push(rebuilt);
        }
    }
    if out.is_empty() {
        let mut run = existing[0].clone();
        run.text = text.to_string();
        out.push(run);
    }
    out
}

fn block_to_editable(block: &Block) -> EditableBlock {
    let role = block_role(block);
    let paragraphs = block_paragraphs(block, role);
    let table = block_table(block);
    let image = block_image(block);
    EditableBlock {
        id: format!("block-{}", block.id),
        source_block_id: block.id,
        page: block.page as usize,
        bbox: block.bbox,
        reading_order: block.reading_order,
        role,
        edit_safety: edit_safety_for(role),
        confidence: block.confidence,
        paragraphs,
        table,
        image,
        provenance: EditableProvenance {
            source: "parse_document".to_string(),
            source_page: block.page as usize,
            source_block_id: block.id,
            confidence: block.confidence,
            notes: Vec::new(),
        },
    }
}

fn block_role(block: &Block) -> EditableRole {
    match &block.kind {
        BlockKind::Title { .. } => EditableRole::Title,
        BlockKind::Heading { .. } => EditableRole::Heading,
        BlockKind::Paragraph { .. } => EditableRole::Paragraph,
        BlockKind::List { .. } => EditableRole::List,
        BlockKind::Figure { .. } => EditableRole::Figure,
        BlockKind::Caption { .. } => EditableRole::Caption,
        BlockKind::Table { .. } => EditableRole::Table,
        BlockKind::Header { .. } => EditableRole::Header,
        BlockKind::Footer { .. } => EditableRole::Footer,
        BlockKind::PageNumber { .. } => EditableRole::PageNumber,
        BlockKind::Text { .. } => EditableRole::Text,
    }
}

fn edit_safety_for(role: EditableRole) -> EditSafety {
    match role {
        EditableRole::Paragraph
        | EditableRole::Caption
        | EditableRole::Text
        | EditableRole::Heading
        | EditableRole::Title => EditSafety::LocalReflowRewrite,
        EditableRole::Figure | EditableRole::Table => EditSafety::PageRegenerate,
        EditableRole::Header | EditableRole::Footer | EditableRole::PageNumber => {
            EditSafety::SafePatch
        }
        EditableRole::List => EditSafety::LocalReflowRewrite,
    }
}

fn block_paragraphs(block: &Block, role: EditableRole) -> Vec<EditableParagraph> {
    match &block.kind {
        BlockKind::Title { text }
        | BlockKind::Heading { text, .. }
        | BlockKind::Paragraph { text }
        | BlockKind::Caption { text, .. }
        | BlockKind::Header { text }
        | BlockKind::Footer { text }
        | BlockKind::PageNumber { text }
        | BlockKind::Text { text } => vec![paragraph_from_spans(block, role, &text.spans, None)],
        BlockKind::List { ordered, items } => items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                paragraph_from_spans(
                    block,
                    role,
                    &item.text.spans,
                    Some(EditableListInfo {
                        ordered: *ordered,
                        marker: item.marker.clone().or_else(|| {
                            Some(if *ordered {
                                format!("{}.", idx + 1)
                            } else {
                                "-".to_string()
                            })
                        }),
                        level: 0,
                    }),
                )
            })
            .collect(),
        BlockKind::Figure { alt, .. } => alt
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .map(|text| vec![paragraph_from_text(block, role, text, None)])
            .unwrap_or_default(),
        BlockKind::Table { .. } => Vec::new(),
    }
}

fn paragraph_from_spans(
    block: &Block,
    role: EditableRole,
    spans: &[InlineSpan],
    list: Option<EditableListInfo>,
) -> EditableParagraph {
    let text = spans
        .iter()
        .map(|span| span.text.as_str())
        .collect::<String>();
    EditableParagraph {
        id: format!(
            "block-{}-p{}",
            block.id,
            list.as_ref().map(|_| 1).unwrap_or(0)
        ),
        role,
        text,
        runs: spans
            .iter()
            .enumerate()
            .map(|(idx, span)| EditableRun {
                id: format!("block-{}-r{idx}", block.id),
                text: span.text.clone(),
                style: EditableTextStyle {
                    bold: span.bold,
                    italic: span.italic,
                    link: span.link.clone(),
                    ..Default::default()
                },
                source_span_index: idx,
            })
            .collect(),
        list,
        confidence: block.confidence,
    }
}

fn paragraph_from_text(
    block: &Block,
    role: EditableRole,
    text: &str,
    list: Option<EditableListInfo>,
) -> EditableParagraph {
    paragraph_from_spans(
        block,
        role,
        &[InlineSpan {
            text: text.to_string(),
            ..Default::default()
        }],
        list,
    )
}

fn block_table(block: &Block) -> Option<EditableTable> {
    let BlockKind::Table { table, .. } = &block.kind else {
        return None;
    };
    let mut cells = Vec::new();
    if table.cells.is_empty() {
        for (row, values) in table.rows.iter().enumerate() {
            for (col, text) in values.iter().enumerate() {
                cells.push(EditableTableCell {
                    row,
                    col,
                    rowspan: 1,
                    colspan: 1,
                    text: text.clone(),
                    is_header: row == 0,
                    bbox: table.bbox,
                });
            }
        }
    } else {
        cells.extend(table.cells.iter().map(|cell| EditableTableCell {
            row: cell.row,
            col: cell.col,
            rowspan: cell.rowspan.max(1),
            colspan: cell.colspan.max(1),
            text: cell.text.clone(),
            is_header: cell.is_header,
            bbox: cell.bbox,
        }));
    }
    Some(EditableTable {
        rows: table.num_rows().max(1),
        cols: table.num_cols().max(1),
        cells,
        confidence: table.confidence as f32,
    })
}

fn block_image(block: &Block) -> Option<EditableImage> {
    let BlockKind::Figure { image, .. } = &block.kind else {
        return None;
    };
    image.as_ref().map(|image| EditableImage {
        id: format!("image-{}", image.id),
        source_object: None,
        bbox: block.bbox,
        intrinsic_width: None,
        intrinsic_height: None,
        color_space: None,
        confidence: block.confidence,
    })
}

fn page_images(
    engine: &ContentEngine,
    page: usize,
    max_images: usize,
    diagnostics: &mut Vec<EditableDiagnostic>,
) -> Vec<String> {
    let Ok((w, h)) = engine.page_dimensions(page) else {
        return Vec::new();
    };
    let Ok(region) = crate::PageRegion::new(0.0, 0.0, w.max(1.0), h.max(1.0)) else {
        return Vec::new();
    };
    let Ok(images) = engine.find_page_images_in_region(page, region) else {
        return Vec::new();
    };
    if images.len() > max_images {
        diagnostics.push(EditableDiagnostic {
            code: "editable.images.capped".to_string(),
            severity: EditableDiagnosticSeverity::Warning,
            message: format!("page {page} image list capped at {max_images}"),
            page: Some(page),
        });
    }
    images
        .into_iter()
        .take(max_images)
        .enumerate()
        .map(|(idx, _)| format!("page-{page}-image-{}", idx + 1))
        .collect()
}

fn infer_sections(document: &Document, blocks: &[EditableBlock]) -> Vec<EditableSection> {
    let mut sections = Vec::new();
    let mut current = EditableSection {
        id: "section-1".to_string(),
        title: None,
        page_range: [
            document
                .pages
                .first()
                .map(|p| p.number as usize)
                .unwrap_or(1),
            document
                .pages
                .last()
                .map(|p| p.number as usize)
                .unwrap_or(1),
        ],
        block_ids: Vec::new(),
        confidence: 0.66,
    };
    for block in blocks {
        if matches!(block.role, EditableRole::Heading | EditableRole::Title)
            && !current.block_ids.is_empty()
        {
            sections.push(current);
            current = EditableSection {
                id: format!("section-{}", sections.len() + 1),
                title: Some(block_text(block)),
                page_range: [block.page, block.page],
                block_ids: vec![block.id.clone()],
                confidence: block.confidence.max(0.7),
            };
        } else {
            if current.title.is_none()
                && matches!(block.role, EditableRole::Heading | EditableRole::Title)
            {
                current.title = Some(block_text(block));
            }
            current.page_range[1] = current.page_range[1].max(block.page);
            current.block_ids.push(block.id.clone());
        }
    }
    if !current.block_ids.is_empty() || sections.is_empty() {
        sections.push(current);
    }
    sections
}

fn block_text(block: &EditableBlock) -> String {
    block
        .paragraphs
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn push_markdown_line(out: &mut String, line: &str) {
    if line.trim().is_empty() {
        return;
    }
    out.push_str(line.trim());
    out.push_str("\n\n");
}

fn table_to_markdown(table: &EditableTable) -> String {
    let rows = table.rows.max(1);
    let cols = table.cols.max(1);
    let mut grid = vec![vec![String::new(); cols]; rows];
    for cell in &table.cells {
        if cell.row < rows && cell.col < cols {
            grid[cell.row][cell.col] = cell.text.clone();
        }
    }
    let mut out = String::new();
    for (idx, row) in grid.iter().enumerate() {
        out.push('|');
        for cell in row {
            out.push(' ');
            out.push_str(&cell.replace('|', "\\|"));
            out.push_str(" |");
        }
        out.push('\n');
        if idx == 0 {
            out.push('|');
            for _ in row {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

fn table_to_html(table: &EditableTable) -> String {
    let rows = table.rows.max(1);
    let cols = table.cols.max(1);
    let mut grid = vec![vec![String::new(); cols]; rows];
    for cell in &table.cells {
        if cell.row < rows && cell.col < cols {
            grid[cell.row][cell.col] = cell.text.clone();
        }
    }
    let mut out = String::from("<table>\n");
    for (row_idx, row) in grid.iter().enumerate() {
        out.push_str("<tr>");
        for cell in row {
            let tag = if row_idx == 0 { "th" } else { "td" };
            out.push_str(&format!("<{tag}>{}</{tag}>", html_escape(cell)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
    out
}

fn html_escape(input: &str) -> String {
    input
        .chars()
        .map(|ch| match ch {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#39;".to_string(),
            ch => ch.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::{AuthorPageSize, PdfBuilder, StandardFont, TextStyle};

    use super::*;

    fn sample_pdf() -> Vec<u8> {
        let mut doc = PdfBuilder::new();
        doc.add_page(AuthorPageSize::LETTER)
            .draw_text(
                "Heading",
                72.0,
                720.0,
                &TextStyle::standard(StandardFont::Helvetica, 18.0),
            )
            .unwrap()
            .draw_text(
                "Editable paragraph text.",
                72.0,
                690.0,
                &TextStyle::standard(StandardFont::Helvetica, 11.0),
            )
            .unwrap();
        doc.to_bytes().unwrap()
    }

    #[test]
    fn editable_model_builds_blocks_and_markdown() {
        let engine = ContentEngine::open_bytes(sample_pdf()).unwrap();
        let model = build_editable_document(&engine, &EditableBuildOptions::default()).unwrap();
        assert!(!model.blocks.is_empty());
        assert!(model.to_markdown().contains("Editable paragraph"));
        assert!(!model.sections.is_empty());
    }

    #[test]
    fn editable_transaction_undo_redo_changes_text() {
        let engine = ContentEngine::open_bytes(sample_pdf()).unwrap();
        let mut model = build_editable_document(&engine, &EditableBuildOptions::default()).unwrap();
        let id = model.blocks[0].id.clone();
        assert!(model.replace_block_text(&id, "Changed"));
        assert!(model.to_markdown().contains("Changed"));
        assert!(model.undo());
        assert!(!model.to_markdown().contains("Changed"));
        assert!(model.redo());
        assert!(model.to_markdown().contains("Changed"));
    }
}
