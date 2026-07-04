use std::collections::HashSet;

use serde::Serialize;

use crate::analysis::layout::{analyze_page, BBox, LayoutConfig, PageLayout};
use crate::text::{ReadingOrderReconstructor, TextChunk};

const DEFAULT_MAX_CHUNKS_PER_PAGE: usize = 250_000;
const DEFAULT_MAX_CHARS_PER_PAGE: usize = 2_000_000;
const DEDUPE_X_TOLERANCE: f64 = 0.75;
const DEDUPE_Y_TOLERANCE: f64 = 0.75;
const DEDUPE_FONT_TOLERANCE: f64 = 0.5;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextExtractionMode {
    ExtractAllText,
    VisibleTextOnly,
    SemanticTextPreferActual,
    SearchText,
    RedactionText,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TextSemanticOptions {
    pub mode: TextExtractionMode,
    pub include_chars: bool,
    pub include_hidden: bool,
    pub deduplicate: bool,
    pub prefer_actual_text: bool,
    pub max_chunks_per_page: usize,
    pub max_chars_per_page: usize,
}

impl Default for TextSemanticOptions {
    fn default() -> Self {
        Self {
            mode: TextExtractionMode::SemanticTextPreferActual,
            include_chars: true,
            include_hidden: true,
            deduplicate: true,
            prefer_actual_text: true,
            max_chunks_per_page: DEFAULT_MAX_CHUNKS_PER_PAGE,
            max_chars_per_page: DEFAULT_MAX_CHARS_PER_PAGE,
        }
    }
}

impl TextSemanticOptions {
    pub fn visible_text() -> Self {
        Self {
            mode: TextExtractionMode::VisibleTextOnly,
            include_hidden: false,
            ..Self::default()
        }
    }

    pub fn search_text() -> Self {
        Self {
            mode: TextExtractionMode::SearchText,
            include_hidden: false,
            ..Self::default()
        }
    }

    pub fn redaction_text() -> Self {
        Self {
            mode: TextExtractionMode::RedactionText,
            include_hidden: false,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SemanticTextDirection {
    LeftToRight,
    RightToLeft,
    Vertical,
    Mixed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextRole {
    BodyText,
    Heading,
    List,
    TableCandidate,
    FigureCaption,
    Header,
    Footer,
    Footnote,
    Marginalia,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextMappingSource {
    NativePdfText,
    TaggedPdf,
    ActualText,
    ToUnicode,
    CMap,
    EncodingDifferences,
    GlyphName,
    Ocr,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TextProvenanceFlag {
    NativePdfText,
    TaggedPdf,
    ActualText,
    Ocr,
    FallbackCMap,
    FallbackGlyphName,
    SyntheticLayout,
    LowConfidenceOrder,
    Deduplicated,
    HiddenOrInvisible,
    ArtifactHeaderFooterCandidate,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextLayoutStrategy {
    TaggedPdf,
    XyCutGeometry,
    VerticalWriting,
    VisualFallback,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextDiagnostic {
    pub code: String,
    pub severity: TextDiagnosticSeverity,
    pub page: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct TextQuad {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl TextQuad {
    pub fn from_bbox(bbox: [f64; 4]) -> Self {
        Self {
            x0: bbox[0].min(bbox[2]),
            y0: bbox[1].min(bbox[3]),
            x1: bbox[0].max(bbox[2]),
            y1: bbox[1].max(bbox[3]),
        }
    }

    pub fn union(quads: &[TextQuad]) -> Option<Self> {
        let first = *quads.first()?;
        Some(quads.iter().skip(1).fold(first, |acc, q| Self {
            x0: acc.x0.min(q.x0),
            y0: acc.y0.min(q.y0),
            x1: acc.x1.max(q.x1),
            y1: acc.y1.max(q.y1),
        }))
    }

    pub fn intersects_bbox(self, bbox: BBox) -> bool {
        self.x0 < bbox.x1 && self.x1 > bbox.x0 && self.y0 < bbox.y1 && self.y1 > bbox.y0
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticChar {
    pub text: String,
    pub unicode: String,
    pub char_index: usize,
    pub chunk_index: usize,
    pub font_name: String,
    pub font_size: f64,
    pub direction: SemanticTextDirection,
    pub mapping_source: TextMappingSource,
    pub provenance: Vec<TextProvenanceFlag>,
    pub quad: TextQuad,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticWord {
    pub text: String,
    pub word_index: usize,
    pub char_range: [usize; 2],
    pub quad: TextQuad,
    pub confidence: f32,
    pub provenance: Vec<TextProvenanceFlag>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticSpan {
    pub text: String,
    pub span_index: usize,
    pub char_range: [usize; 2],
    pub quad: TextQuad,
    pub font_name: String,
    pub font_size: f64,
    pub direction: SemanticTextDirection,
    pub mapping_source: TextMappingSource,
    pub provenance: Vec<TextProvenanceFlag>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticLine {
    pub text: String,
    pub line_index: usize,
    pub role: TextRole,
    pub direction: SemanticTextDirection,
    pub words: Vec<TextSemanticWord>,
    pub spans: Vec<TextSemanticSpan>,
    pub chars: Vec<TextSemanticChar>,
    pub quad: TextQuad,
    pub confidence: f32,
    pub provenance: Vec<TextProvenanceFlag>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticParagraph {
    pub text: String,
    pub paragraph_index: usize,
    pub line_range: [usize; 2],
    pub role: TextRole,
    pub quad: TextQuad,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticBlock {
    pub text: String,
    pub block_index: usize,
    pub role: TextRole,
    pub lines: Vec<TextSemanticLine>,
    pub paragraphs: Vec<TextSemanticParagraph>,
    pub quad: TextQuad,
    pub confidence: f32,
    pub provenance: Vec<TextProvenanceFlag>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TextExtractionCounters {
    pub pages: usize,
    pub blocks: usize,
    pub lines: usize,
    pub words: usize,
    pub chars: usize,
    pub total_glyph_runs: usize,
    pub mapped_via_tounicode: usize,
    pub mapped_via_actual_text: usize,
    pub mapped_via_cmap: usize,
    pub mapped_via_encoding_differences: usize,
    pub mapped_via_glyph_name: usize,
    pub mapped_via_ocr: usize,
    pub unknown_unmapped: usize,
    pub hidden_or_invisible: usize,
    pub rtl_runs: usize,
    pub vertical_runs: usize,
    pub deduplicated_runs: usize,
    pub low_confidence_order_edges: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticPage {
    pub page: usize,
    pub page_box: [f64; 4],
    pub blocks: Vec<TextSemanticBlock>,
    pub strategy: TextLayoutStrategy,
    pub confidence: f32,
    pub counters: TextExtractionCounters,
    pub diagnostics: Vec<TextDiagnostic>,
}

impl TextSemanticPage {
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSemanticDocument {
    pub pages: Vec<TextSemanticPage>,
    pub counters: TextExtractionCounters,
    pub diagnostics: Vec<TextDiagnostic>,
}

impl TextSemanticDocument {
    pub fn text(&self) -> String {
        self.pages
            .iter()
            .map(TextSemanticPage::text)
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn search(&self, query: &str, options: &TextSearchOptions) -> Vec<TextSearchMatch> {
        search_semantic_document(self, query, options)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSearchOptions {
    pub case_sensitive: bool,
    pub normalize_ligatures: bool,
    pub ignore_hyphenation: bool,
    pub collapse_whitespace: bool,
    pub include_hidden: bool,
    pub max_matches: usize,
}

impl Default for TextSearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            normalize_ligatures: true,
            ignore_hyphenation: true,
            collapse_whitespace: true,
            include_hidden: false,
            max_matches: 10_000,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextSearchMatch {
    pub page: usize,
    pub text: String,
    pub normalized_text: String,
    pub char_range: [usize; 2],
    pub quads: Vec<TextQuad>,
    pub confidence: f32,
    pub provenance: Vec<TextProvenanceFlag>,
}

#[derive(Debug, Clone)]
struct ChunkRef {
    chunk: TextChunk,
    original_index: usize,
    bbox: TextQuad,
}

#[derive(Debug, Clone)]
struct BuiltLine {
    text: String,
    direction: SemanticTextDirection,
    words: Vec<TextSemanticWord>,
    spans: Vec<TextSemanticSpan>,
    chars: Vec<TextSemanticChar>,
    quad: TextQuad,
    provenance: Vec<TextProvenanceFlag>,
}

pub fn build_text_semantic_page(
    page: usize,
    page_box: [f64; 4],
    chunks: Vec<TextChunk>,
    options: &TextSemanticOptions,
) -> TextSemanticPage {
    let mut diagnostics = Vec::new();
    let mut counters = TextExtractionCounters {
        pages: 1,
        total_glyph_runs: chunks.len(),
        ..Default::default()
    };

    let mut working = filter_chunks(page, chunks, options, &mut counters, &mut diagnostics);
    if working.len() > options.max_chunks_per_page {
        diagnostics.push(TextDiagnostic {
            code: "text.semantic.chunk_cap".to_string(),
            severity: TextDiagnosticSeverity::Warning,
            page: Some(page),
            message: format!(
                "page has {} text runs; semantic model capped at {}",
                working.len(),
                options.max_chunks_per_page
            ),
        });
        working.truncate(options.max_chunks_per_page);
    }

    let layout_chunks: Vec<TextChunk> = working.iter().map(|r| r.chunk.clone()).collect();
    let layout = analyze_page(&layout_chunks, &LayoutConfig::default());
    let mut block_specs = layout_to_block_specs(&layout);
    append_vertical_block_specs(&mut block_specs, &layout_chunks);
    if block_specs.is_empty() && !layout_chunks.is_empty() {
        block_specs.push(fallback_block_spec(&layout_chunks));
    }

    let mut used_chars = 0usize;
    let median_font_size = median_font_size(&layout_chunks).unwrap_or(12.0);
    let page_height = (page_box[3] - page_box[1]).abs().max(1.0);
    let mut blocks = Vec::new();
    let mut global_char_index = 0usize;
    let mut global_word_index = 0usize;
    let mut global_span_index = 0usize;
    let mut line_index = 0usize;

    for (block_index, spec) in block_specs.into_iter().enumerate() {
        let role = classify_block(
            spec.bbox,
            spec.font_size,
            median_font_size,
            page_box,
            page_height,
        );
        let mut lines = Vec::new();
        for line in spec.lines {
            let candidates = chunks_for_bbox(&working, line.bbox, line.direction);
            let built = if candidates.is_empty() {
                build_line_from_text(
                    &line.text,
                    line.bbox,
                    line.direction,
                    line_index,
                    &mut global_char_index,
                    &mut global_word_index,
                    &mut global_span_index,
                    options,
                )
            } else {
                build_line_from_chunks(
                    &candidates,
                    line.bbox,
                    line.direction,
                    line_index,
                    &mut global_char_index,
                    &mut global_word_index,
                    &mut global_span_index,
                    options,
                )
            };
            used_chars += built.chars.len();
            if used_chars > options.max_chars_per_page {
                diagnostics.push(TextDiagnostic {
                    code: "text.semantic.char_cap".to_string(),
                    severity: TextDiagnosticSeverity::Warning,
                    page: Some(page),
                    message: format!(
                        "page semantic characters exceeded cap {}; remaining text omitted",
                        options.max_chars_per_page
                    ),
                });
                break;
            }
            counters.words += built.words.len();
            counters.chars += built.chars.len();
            let role = classify_line(&built.text, role, built.quad, page_box, median_font_size);
            lines.push(TextSemanticLine {
                text: built.text,
                line_index,
                role,
                direction: built.direction,
                words: built.words,
                spans: built.spans,
                chars: built.chars,
                quad: built.quad,
                confidence: if built
                    .provenance
                    .contains(&TextProvenanceFlag::SyntheticLayout)
                {
                    0.74
                } else {
                    0.86
                },
                provenance: built.provenance,
            });
            line_index += 1;
        }
        if lines.is_empty() {
            continue;
        }
        let quad = TextQuad::union(
            &lines
                .iter()
                .map(|line| line.quad)
                .collect::<Vec<TextQuad>>(),
        )
        .unwrap_or_else(|| {
            TextQuad::from_bbox([spec.bbox.x0, spec.bbox.y0, spec.bbox.x1, spec.bbox.y1])
        });
        let text = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let paragraphs = build_paragraphs(&lines, role);
        counters.lines += lines.len();
        counters.blocks += 1;
        blocks.push(TextSemanticBlock {
            text,
            block_index,
            role,
            lines,
            paragraphs,
            quad,
            confidence: if matches!(role, TextRole::Unknown) {
                0.58
            } else {
                0.78
            },
            provenance: vec![TextProvenanceFlag::SyntheticLayout],
        });
    }

    let strategy = if counters.vertical_runs > 0 && layout.blocks.is_empty() {
        TextLayoutStrategy::VerticalWriting
    } else if !layout.blocks.is_empty() {
        TextLayoutStrategy::XyCutGeometry
    } else {
        TextLayoutStrategy::VisualFallback
    };
    if matches!(strategy, TextLayoutStrategy::VisualFallback) && !working.is_empty() {
        counters.low_confidence_order_edges += 1;
        diagnostics.push(TextDiagnostic {
            code: "text.layout.low_confidence_order".to_string(),
            severity: TextDiagnosticSeverity::Warning,
            page: Some(page),
            message: "semantic model used fallback visual ordering".to_string(),
        });
    }
    if counters.hidden_or_invisible > 0 {
        diagnostics.push(TextDiagnostic {
            code: "text.visibility.hidden_or_invisible".to_string(),
            severity: TextDiagnosticSeverity::Info,
            page: Some(page),
            message: format!(
                "{} hidden or invisible text runs observed",
                counters.hidden_or_invisible
            ),
        });
    }
    if counters.mapped_via_actual_text > 0 {
        diagnostics.push(TextDiagnostic {
            code: "text.actual_text.used".to_string(),
            severity: TextDiagnosticSeverity::Info,
            page: Some(page),
            message: format!(
                "{} characters came from ActualText replacement",
                counters.mapped_via_actual_text
            ),
        });
    }

    TextSemanticPage {
        page,
        page_box,
        blocks,
        strategy,
        confidence: if counters.low_confidence_order_edges > 0 {
            0.68
        } else {
            0.84
        },
        counters,
        diagnostics,
    }
}

pub fn build_text_semantic_document(
    pages: Vec<TextSemanticPage>,
    mut diagnostics: Vec<TextDiagnostic>,
) -> TextSemanticDocument {
    let mut counters = TextExtractionCounters::default();
    for page in &pages {
        merge_counters(&mut counters, &page.counters);
        diagnostics.extend(page.diagnostics.clone());
    }
    TextSemanticDocument {
        pages,
        counters,
        diagnostics,
    }
}

fn filter_chunks(
    page: usize,
    chunks: Vec<TextChunk>,
    options: &TextSemanticOptions,
    counters: &mut TextExtractionCounters,
    diagnostics: &mut Vec<TextDiagnostic>,
) -> Vec<ChunkRef> {
    let mut out = Vec::new();
    for (idx, chunk) in chunks.into_iter().enumerate() {
        if chunk.text.is_empty() {
            continue;
        }
        if chunk.is_invisible {
            counters.hidden_or_invisible += 1;
        }
        if chunk.is_rtl {
            counters.rtl_runs += 1;
        }
        if chunk.is_vertical {
            counters.vertical_runs += 1;
        }
        if chunk.is_actual_text {
            counters.mapped_via_actual_text += chunk.text.chars().count();
        }
        if chunk.text.contains('\u{FFFD}') {
            counters.unknown_unmapped += chunk.text.matches('\u{FFFD}').count();
        }
        if chunk.is_invisible && !options.include_hidden {
            continue;
        }
        out.push(ChunkRef {
            bbox: chunk_bbox(&chunk),
            chunk,
            original_index: idx,
        });
    }

    if options.deduplicate {
        let before = out.len();
        out = deduplicate_chunks(out);
        let removed = before.saturating_sub(out.len());
        counters.deduplicated_runs += removed;
        if removed > 0 {
            diagnostics.push(TextDiagnostic {
                code: "text.dedup.removed".to_string(),
                severity: TextDiagnosticSeverity::Info,
                page: Some(page),
                message: format!("removed {removed} duplicate text runs from semantic model"),
            });
        }
    }
    out
}

fn deduplicate_chunks(chunks: Vec<ChunkRef>) -> Vec<ChunkRef> {
    let mut kept: Vec<ChunkRef> = Vec::with_capacity(chunks.len());
    'outer: for candidate in chunks {
        for existing in &kept {
            if candidate.chunk.text == existing.chunk.text
                && (candidate.chunk.x - existing.chunk.x).abs() <= DEDUPE_X_TOLERANCE
                && (candidate.chunk.y - existing.chunk.y).abs() <= DEDUPE_Y_TOLERANCE
                && (candidate.chunk.font_size - existing.chunk.font_size).abs()
                    <= DEDUPE_FONT_TOLERANCE
                && candidate.chunk.is_invisible == existing.chunk.is_invisible
            {
                continue 'outer;
            }
        }
        kept.push(candidate);
    }
    kept
}

#[derive(Debug, Clone)]
struct BlockSpec {
    bbox: BBox,
    font_size: f64,
    lines: Vec<LineSpec>,
}

#[derive(Debug, Clone)]
struct LineSpec {
    text: String,
    bbox: BBox,
    direction: SemanticTextDirection,
}

fn layout_to_block_specs(layout: &PageLayout) -> Vec<BlockSpec> {
    layout
        .blocks
        .iter()
        .map(|block| BlockSpec {
            bbox: block.bbox,
            font_size: block.font_size,
            lines: block
                .lines
                .iter()
                .map(|line| LineSpec {
                    text: line.text.clone(),
                    bbox: line.bbox,
                    direction: if line.is_rtl {
                        SemanticTextDirection::RightToLeft
                    } else {
                        SemanticTextDirection::LeftToRight
                    },
                })
                .collect(),
        })
        .collect()
}

fn append_vertical_block_specs(blocks: &mut Vec<BlockSpec>, chunks: &[TextChunk]) {
    let vertical: Vec<TextChunk> = chunks.iter().filter(|c| c.is_vertical).cloned().collect();
    if vertical.is_empty() {
        return;
    }
    let reconstructor = ReadingOrderReconstructor::new();
    let lines = reconstructor.reconstruct(vertical);
    let mut specs = Vec::new();
    for line in lines {
        let bbox = BBox {
            x0: line.x_min,
            y0: line.y,
            x1: line.x_max,
            y1: line.y + line.font_size,
        };
        specs.push(LineSpec {
            text: line.text,
            bbox,
            direction: SemanticTextDirection::Vertical,
        });
    }
    if specs.is_empty() {
        return;
    }
    let bbox = specs
        .iter()
        .map(|line| line.bbox)
        .reduce(|acc, bbox| BBox {
            x0: acc.x0.min(bbox.x0),
            y0: acc.y0.min(bbox.y0),
            x1: acc.x1.max(bbox.x1),
            y1: acc.y1.max(bbox.y1),
        })
        .unwrap_or(BBox {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        });
    blocks.push(BlockSpec {
        bbox,
        font_size: median_font_size(chunks).unwrap_or(12.0),
        lines: specs,
    });
}

fn fallback_block_spec(chunks: &[TextChunk]) -> BlockSpec {
    let reconstructor = ReadingOrderReconstructor::new();
    let lines = reconstructor.reconstruct(chunks.to_vec());
    let mut specs = Vec::new();
    for line in lines {
        let bbox = BBox {
            x0: line.x_min,
            y0: line.y,
            x1: line.x_max,
            y1: line.y + line.font_size,
        };
        specs.push(LineSpec {
            text: line.text,
            bbox,
            direction: SemanticTextDirection::LeftToRight,
        });
    }
    let bbox = chunks
        .iter()
        .map(chunk_bbox)
        .reduce(|acc, q| TextQuad {
            x0: acc.x0.min(q.x0),
            y0: acc.y0.min(q.y0),
            x1: acc.x1.max(q.x1),
            y1: acc.y1.max(q.y1),
        })
        .map(|q| BBox {
            x0: q.x0,
            y0: q.y0,
            x1: q.x1,
            y1: q.y1,
        })
        .unwrap_or(BBox {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        });
    BlockSpec {
        bbox,
        font_size: median_font_size(chunks).unwrap_or(12.0),
        lines: specs,
    }
}

fn chunks_for_bbox(
    chunks: &[ChunkRef],
    bbox: BBox,
    direction: SemanticTextDirection,
) -> Vec<ChunkRef> {
    let mut selected: Vec<ChunkRef> = chunks
        .iter()
        .filter(|candidate| {
            candidate.bbox.intersects_bbox(bbox)
                || (matches!(direction, SemanticTextDirection::Vertical)
                    && center_y(candidate.bbox, bbox))
        })
        .cloned()
        .collect();
    match direction {
        SemanticTextDirection::RightToLeft => selected.sort_by(|a, b| {
            b.chunk
                .x
                .partial_cmp(&a.chunk.x)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        SemanticTextDirection::Vertical => selected.sort_by(|a, b| {
            b.chunk
                .y
                .partial_cmp(&a.chunk.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        _ => selected.sort_by(|a, b| {
            a.chunk
                .x
                .partial_cmp(&b.chunk.x)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    }
    selected
}

fn center_y(quad: TextQuad, bbox: BBox) -> bool {
    let cy = (quad.y0 + quad.y1) / 2.0;
    cy >= bbox.y0 - 0.5 && cy <= bbox.y1 + 0.5
}

#[allow(clippy::too_many_arguments)]
fn build_line_from_text(
    text: &str,
    bbox: BBox,
    direction: SemanticTextDirection,
    line_index: usize,
    global_char_index: &mut usize,
    global_word_index: &mut usize,
    global_span_index: &mut usize,
    options: &TextSemanticOptions,
) -> BuiltLine {
    let synthetic = TextChunk {
        text: text.to_string(),
        x: bbox.x0,
        y: bbox.y0,
        font_size: (bbox.y1 - bbox.y0).max(1.0),
        font_name: String::new(),
        width: (bbox.x1 - bbox.x0).max(0.0),
        is_rtl: matches!(direction, SemanticTextDirection::RightToLeft),
        is_vertical: matches!(direction, SemanticTextDirection::Vertical),
        is_invisible: false,
        is_actual_text: false,
    };
    build_line_from_chunks(
        &[ChunkRef {
            chunk: synthetic,
            original_index: line_index,
            bbox: TextQuad::from_bbox([bbox.x0, bbox.y0, bbox.x1, bbox.y1]),
        }],
        bbox,
        direction,
        line_index,
        global_char_index,
        global_word_index,
        global_span_index,
        options,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_line_from_chunks(
    chunks: &[ChunkRef],
    bbox: BBox,
    direction: SemanticTextDirection,
    _line_index: usize,
    global_char_index: &mut usize,
    global_word_index: &mut usize,
    global_span_index: &mut usize,
    options: &TextSemanticOptions,
) -> BuiltLine {
    let mut chars = Vec::new();
    let mut spans = Vec::new();
    let mut words = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();

    for chunk_ref in chunks {
        let chunk = &chunk_ref.chunk;
        if chunk.text.trim().is_empty() {
            continue;
        }
        if !text_parts.is_empty() && needs_space(text_parts.last().unwrap(), &chunk.text, direction)
        {
            text_parts.push(" ".to_string());
            let quad = chars
                .last()
                .map(|last: &TextSemanticChar| TextQuad {
                    x0: last.quad.x1,
                    y0: last.quad.y0,
                    x1: chunk_ref.bbox.x0.max(last.quad.x1),
                    y1: last.quad.y1,
                })
                .unwrap_or(chunk_ref.bbox);
            chars.push(TextSemanticChar {
                text: " ".to_string(),
                unicode: " ".to_string(),
                char_index: *global_char_index,
                chunk_index: chunk_ref.original_index,
                font_name: chunk.font_name.clone(),
                font_size: chunk.font_size,
                direction,
                mapping_source: TextMappingSource::NativePdfText,
                provenance: vec![TextProvenanceFlag::SyntheticLayout],
                quad,
                confidence: 0.62,
            });
            *global_char_index += 1;
        }
        text_parts.push(chunk.text.clone());

        let start_char = *global_char_index;
        let chunk_chars = char_quads_for_chunk(chunk, chunk_ref.original_index, global_char_index);
        let mut span_chars = Vec::new();
        for (ch, quad, char_index) in chunk_chars {
            let mapping_source = mapping_source_for_chunk(chunk);
            let provenance = provenance_for_chunk(chunk);
            span_chars.push(TextSemanticChar {
                text: ch.to_string(),
                unicode: ch.to_string(),
                char_index,
                chunk_index: chunk_ref.original_index,
                font_name: chunk.font_name.clone(),
                font_size: chunk.font_size,
                direction: direction_for_chunk(chunk, direction),
                mapping_source,
                provenance,
                quad,
                confidence: if ch == '\u{FFFD}' { 0.1 } else { 0.82 },
            });
        }
        let end_char = *global_char_index;
        let span_quad = TextQuad::union(&span_chars.iter().map(|ch| ch.quad).collect::<Vec<_>>())
            .unwrap_or(chunk_ref.bbox);
        spans.push(TextSemanticSpan {
            text: chunk.text.clone(),
            span_index: *global_span_index,
            char_range: [start_char, end_char],
            quad: span_quad,
            font_name: chunk.font_name.clone(),
            font_size: chunk.font_size,
            direction: direction_for_chunk(chunk, direction),
            mapping_source: mapping_source_for_chunk(chunk),
            provenance: provenance_for_chunk(chunk),
            confidence: if chunk.text.contains('\u{FFFD}') {
                0.2
            } else {
                0.82
            },
        });
        *global_span_index += 1;
        chars.extend(span_chars);
    }

    let line_text = text_parts.join("").trim().to_string();
    let token_ranges = tokenize_words_from_chars(&chars);
    for (word_text, start, end) in token_ranges {
        let word_chars: Vec<TextQuad> = chars
            .iter()
            .filter(|ch| ch.char_index >= start && ch.char_index < end)
            .map(|ch| ch.quad)
            .collect();
        let quad = TextQuad::union(&word_chars)
            .unwrap_or_else(|| TextQuad::from_bbox([bbox.x0, bbox.y0, bbox.x1, bbox.y1]));
        words.push(TextSemanticWord {
            text: word_text,
            word_index: *global_word_index,
            char_range: [start, end],
            quad,
            confidence: 0.84,
            provenance: flags_union(
                &chars
                    .iter()
                    .filter(|ch| ch.char_index >= start && ch.char_index < end)
                    .flat_map(|ch| ch.provenance.iter().copied())
                    .collect::<Vec<_>>(),
            ),
        });
        *global_word_index += 1;
    }

    let line_quad = TextQuad::union(&chars.iter().map(|ch| ch.quad).collect::<Vec<_>>())
        .unwrap_or_else(|| TextQuad::from_bbox([bbox.x0, bbox.y0, bbox.x1, bbox.y1]));
    let mut provenance = flags_union(
        &spans
            .iter()
            .flat_map(|span| span.provenance.iter().copied())
            .collect::<Vec<_>>(),
    );
    provenance.push(TextProvenanceFlag::SyntheticLayout);
    provenance = flags_union(&provenance);

    BuiltLine {
        text: if line_text.is_empty() {
            chunks
                .iter()
                .map(|c| c.chunk.text.as_str())
                .collect::<Vec<_>>()
                .join("")
        } else {
            line_text
        },
        direction,
        words,
        spans,
        chars: if options.include_chars {
            chars
        } else {
            Vec::new()
        },
        quad: line_quad,
        provenance,
    }
}

fn char_quads_for_chunk(
    chunk: &TextChunk,
    chunk_index: usize,
    global_char_index: &mut usize,
) -> Vec<(char, TextQuad, usize)> {
    let chars: Vec<char> = chunk.text.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(chars.len());
    let count = chars.len().max(1) as f64;
    if chunk.is_vertical {
        let step = (chunk.width.max(chunk.font_size) / count).max(0.1);
        for (idx, ch) in chars.into_iter().enumerate() {
            let y1 = chunk.y - step * idx as f64;
            let y0 = y1 - step;
            let quad = TextQuad {
                x0: chunk.x,
                y0: y0.min(y1),
                x1: chunk.x + chunk.font_size.max(1.0),
                y1: y0.max(y1) + chunk.font_size.min(step),
            };
            out.push((ch, quad, *global_char_index));
            *global_char_index += 1;
        }
    } else {
        let width = chunk.width.max(chunk.font_size * count * 0.35);
        let step = width / count;
        for (idx, ch) in chars.into_iter().enumerate() {
            let x0 = chunk.x + step * idx as f64;
            let x1 = if idx + 1 == count as usize {
                chunk.x + width
            } else {
                x0 + step
            };
            let quad = TextQuad {
                x0,
                y0: chunk.y,
                x1,
                y1: chunk.y + chunk.font_size.max(1.0),
            };
            out.push((ch, quad, *global_char_index));
            *global_char_index += 1;
        }
    }
    let _ = chunk_index;
    out
}

fn tokenize_words_from_chars(chars: &[TextSemanticChar]) -> Vec<(String, usize, usize)> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut start = None;

    for ch in chars {
        let Some(c) = ch.text.chars().next() else {
            continue;
        };
        if c.is_whitespace() {
            flush_token(&mut tokens, &mut current, &mut start, ch.char_index);
            continue;
        }
        if is_cjk_char(c) {
            flush_token(&mut tokens, &mut current, &mut start, ch.char_index);
            tokens.push((c.to_string(), ch.char_index, ch.char_index + 1));
            continue;
        }
        if start.is_none() {
            start = Some(ch.char_index);
        }
        current.push(c);
    }
    let end = chars.last().map(|ch| ch.char_index + 1).unwrap_or(0);
    flush_token(&mut tokens, &mut current, &mut start, end);
    tokens
}

fn flush_token(
    tokens: &mut Vec<(String, usize, usize)>,
    current: &mut String,
    start: &mut Option<usize>,
    end: usize,
) {
    if !current.is_empty() {
        tokens.push((current.clone(), start.unwrap_or(end), end));
        current.clear();
    }
    *start = None;
}

fn is_cjk_char(c: char) -> bool {
    let cp = c as u32;
    matches!(
        cp,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

fn needs_space(left: &str, right: &str, direction: SemanticTextDirection) -> bool {
    if matches!(direction, SemanticTextDirection::Vertical) {
        return false;
    }
    let Some(last) = left.chars().rev().find(|c| !c.is_whitespace()) else {
        return false;
    };
    let Some(first) = right.chars().find(|c| !c.is_whitespace()) else {
        return false;
    };
    !last.is_whitespace()
        && !first.is_whitespace()
        && !is_cjk_char(last)
        && !is_cjk_char(first)
        && last != '-'
}

fn build_paragraphs(lines: &[TextSemanticLine], role: TextRole) -> Vec<TextSemanticParagraph> {
    if lines.is_empty() {
        return Vec::new();
    }
    let mut paragraphs = Vec::new();
    let mut start = 0usize;
    for idx in 1..lines.len() {
        let prev = lines[idx - 1].quad;
        let current = lines[idx].quad;
        let prev_height = (prev.y1 - prev.y0).max(1.0);
        let gap = prev.y0 - current.y1;
        let indent_delta = (current.x0 - lines[start].quad.x0).abs();
        if gap > prev_height * 0.9 || indent_delta > prev_height * 2.0 {
            push_paragraph(&mut paragraphs, lines, start, idx, role);
            start = idx;
        }
    }
    push_paragraph(&mut paragraphs, lines, start, lines.len(), role);
    paragraphs
}

fn push_paragraph(
    out: &mut Vec<TextSemanticParagraph>,
    lines: &[TextSemanticLine],
    start: usize,
    end: usize,
    role: TextRole,
) {
    let line_slice = &lines[start..end];
    let text = line_slice
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let quad = TextQuad::union(&line_slice.iter().map(|line| line.quad).collect::<Vec<_>>())
        .unwrap_or(TextQuad {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        });
    out.push(TextSemanticParagraph {
        text,
        paragraph_index: out.len(),
        line_range: [start, end],
        role,
        quad,
        confidence: 0.72,
    });
}

fn classify_block(
    bbox: BBox,
    font_size: f64,
    median_font_size: f64,
    page_box: [f64; 4],
    page_height: f64,
) -> TextRole {
    let top_band = page_box[3] - page_height * 0.08;
    let bottom_band = page_box[1] + page_height * 0.08;
    let furniture_like_height = bbox.height() <= median_font_size.max(1.0) * 2.5;
    if bbox.y1 >= top_band && furniture_like_height {
        return TextRole::Header;
    }
    if bbox.y0 <= bottom_band && furniture_like_height {
        return TextRole::Footer;
    }
    if font_size > median_font_size * 1.25 {
        return TextRole::Heading;
    }
    if font_size < median_font_size * 0.82 {
        return TextRole::Footnote;
    }
    TextRole::BodyText
}

fn classify_line(
    text: &str,
    block_role: TextRole,
    quad: TextQuad,
    page_box: [f64; 4],
    median_font_size: f64,
) -> TextRole {
    let trimmed = text.trim_start();
    if trimmed.starts_with(['-', '*', '\u{2022}']) || starts_with_numbered_list(trimmed) {
        return TextRole::List;
    }
    if trimmed.to_ascii_lowercase().starts_with("figure ")
        || trimmed.to_ascii_lowercase().starts_with("fig. ")
        || trimmed.to_ascii_lowercase().starts_with("table ")
    {
        return TextRole::FigureCaption;
    }
    if (quad.y1 - quad.y0) < median_font_size * 0.85
        && quad.y0 < page_box[1] + (page_box[3] - page_box[1]).abs() * 0.25
    {
        return TextRole::Footnote;
    }
    block_role
}

fn starts_with_numbered_list(text: &str) -> bool {
    let mut chars = text.chars().peekable();
    let mut saw_digit = false;
    while matches!(chars.peek(), Some(c) if c.is_ascii_digit()) {
        saw_digit = true;
        chars.next();
    }
    saw_digit && matches!(chars.next(), Some('.') | Some(')'))
}

fn mapping_source_for_chunk(chunk: &TextChunk) -> TextMappingSource {
    if chunk.is_actual_text {
        TextMappingSource::ActualText
    } else if chunk.is_invisible {
        TextMappingSource::Ocr
    } else if chunk.text.contains('\u{FFFD}') {
        TextMappingSource::Unknown
    } else {
        TextMappingSource::NativePdfText
    }
}

fn provenance_for_chunk(chunk: &TextChunk) -> Vec<TextProvenanceFlag> {
    let mut flags = Vec::new();
    if chunk.is_actual_text {
        flags.push(TextProvenanceFlag::ActualText);
    } else {
        flags.push(TextProvenanceFlag::NativePdfText);
    }
    if chunk.is_invisible {
        flags.push(TextProvenanceFlag::HiddenOrInvisible);
        flags.push(TextProvenanceFlag::Ocr);
    }
    flags
}

fn direction_for_chunk(
    chunk: &TextChunk,
    fallback: SemanticTextDirection,
) -> SemanticTextDirection {
    if chunk.is_vertical {
        SemanticTextDirection::Vertical
    } else if chunk.is_rtl {
        SemanticTextDirection::RightToLeft
    } else {
        fallback
    }
}

fn chunk_bbox(chunk: &TextChunk) -> TextQuad {
    if chunk.is_vertical {
        TextQuad {
            x0: chunk.x,
            y0: chunk.y - chunk.width.max(0.0),
            x1: chunk.x + chunk.font_size.max(1.0),
            y1: chunk.y + chunk.font_size.max(1.0),
        }
    } else {
        TextQuad {
            x0: chunk.x,
            y0: chunk.y,
            x1: chunk.x + chunk.width.max(0.0),
            y1: chunk.y + chunk.font_size.max(1.0),
        }
    }
}

fn median_font_size(chunks: &[TextChunk]) -> Option<f64> {
    let mut sizes: Vec<f64> = chunks
        .iter()
        .map(|chunk| chunk.font_size)
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect();
    if sizes.is_empty() {
        return None;
    }
    sizes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(sizes[sizes.len() / 2])
}

fn flags_union(flags: &[TextProvenanceFlag]) -> Vec<TextProvenanceFlag> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for flag in flags {
        if seen.insert(*flag) {
            out.push(*flag);
        }
    }
    out
}

fn merge_counters(into: &mut TextExtractionCounters, other: &TextExtractionCounters) {
    into.pages += other.pages;
    into.blocks += other.blocks;
    into.lines += other.lines;
    into.words += other.words;
    into.chars += other.chars;
    into.total_glyph_runs += other.total_glyph_runs;
    into.mapped_via_tounicode += other.mapped_via_tounicode;
    into.mapped_via_actual_text += other.mapped_via_actual_text;
    into.mapped_via_cmap += other.mapped_via_cmap;
    into.mapped_via_encoding_differences += other.mapped_via_encoding_differences;
    into.mapped_via_glyph_name += other.mapped_via_glyph_name;
    into.mapped_via_ocr += other.mapped_via_ocr;
    into.unknown_unmapped += other.unknown_unmapped;
    into.hidden_or_invisible += other.hidden_or_invisible;
    into.rtl_runs += other.rtl_runs;
    into.vertical_runs += other.vertical_runs;
    into.deduplicated_runs += other.deduplicated_runs;
    into.low_confidence_order_edges += other.low_confidence_order_edges;
}

fn search_semantic_document(
    document: &TextSemanticDocument,
    query: &str,
    options: &TextSearchOptions,
) -> Vec<TextSearchMatch> {
    let query_norm = normalize_query(query, options);
    if query_norm.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    for page in &document.pages {
        let stream = searchable_stream(page, options);
        let haystack: String = stream.iter().map(|item| item.ch).collect();
        let mut start = 0usize;
        while matches.len() < options.max_matches {
            let Some(pos) = haystack[start..].find(&query_norm) else {
                break;
            };
            let from = start + pos;
            let to = from + query_norm.len();
            let char_refs: Vec<&TextSemanticChar> = stream[from..to]
                .iter()
                .filter_map(|item| item.char_ref)
                .collect();
            if !char_refs.is_empty() {
                let mut seen = HashSet::new();
                let unique_refs: Vec<&TextSemanticChar> = char_refs
                    .into_iter()
                    .filter(|ch| seen.insert(ch.char_index))
                    .collect();
                let quads = unique_refs.iter().map(|ch| ch.quad).collect::<Vec<_>>();
                let text = unique_refs
                    .iter()
                    .map(|ch| ch.text.as_str())
                    .collect::<Vec<_>>()
                    .join("");
                let provenance = flags_union(
                    &unique_refs
                        .iter()
                        .flat_map(|ch| ch.provenance.iter().copied())
                        .collect::<Vec<_>>(),
                );
                let first = unique_refs.first().map(|ch| ch.char_index).unwrap_or(0);
                let last = unique_refs
                    .last()
                    .map(|ch| ch.char_index + 1)
                    .unwrap_or(first);
                matches.push(TextSearchMatch {
                    page: page.page,
                    text,
                    normalized_text: query_norm.clone(),
                    char_range: [first, last],
                    quads,
                    confidence: 0.86,
                    provenance,
                });
            }
            start = to.max(start + 1);
        }
    }
    matches
}

#[derive(Debug, Clone, Copy)]
struct SearchItem<'a> {
    ch: char,
    char_ref: Option<&'a TextSemanticChar>,
}

fn searchable_stream<'a>(
    page: &'a TextSemanticPage,
    options: &TextSearchOptions,
) -> Vec<SearchItem<'a>> {
    let mut raw = Vec::new();
    for block in &page.blocks {
        for line in &block.lines {
            for ch in &line.chars {
                if !options.include_hidden
                    && ch
                        .provenance
                        .contains(&TextProvenanceFlag::HiddenOrInvisible)
                {
                    continue;
                }
                raw.push(SearchItem {
                    ch: ch.text.chars().next().unwrap_or('\u{FFFD}'),
                    char_ref: Some(ch),
                });
            }
            raw.push(SearchItem {
                ch: '\n',
                char_ref: None,
            });
        }
        raw.push(SearchItem {
            ch: '\n',
            char_ref: None,
        });
    }
    normalize_stream(raw, options)
}

fn normalize_query(query: &str, options: &TextSearchOptions) -> String {
    let raw = query
        .chars()
        .map(|ch| SearchItem { ch, char_ref: None })
        .collect();
    normalize_stream(raw, options)
        .into_iter()
        .map(|item| item.ch)
        .collect()
}

fn normalize_stream<'a>(
    raw: Vec<SearchItem<'a>>,
    options: &TextSearchOptions,
) -> Vec<SearchItem<'a>> {
    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < raw.len() {
        let item = raw[idx];
        if options.ignore_hyphenation
            && item.ch == '-'
            && raw.get(idx + 1).is_some_and(|next| next.ch == '\n')
        {
            idx += 2;
            continue;
        }
        let mut chars = if options.normalize_ligatures {
            ligature_expansion(item.ch)
        } else {
            vec![item.ch]
        };
        for mut ch in chars.drain(..) {
            if options.collapse_whitespace && ch.is_whitespace() {
                ch = ' ';
                if out
                    .last()
                    .is_some_and(|prev: &SearchItem<'_>| prev.ch == ' ')
                {
                    continue;
                }
            }
            if !options.case_sensitive {
                for lower in ch.to_lowercase() {
                    out.push(SearchItem {
                        ch: lower,
                        char_ref: item.char_ref,
                    });
                }
            } else {
                out.push(SearchItem {
                    ch,
                    char_ref: item.char_ref,
                });
            }
        }
        idx += 1;
    }
    out
}

fn ligature_expansion(ch: char) -> Vec<char> {
    match ch {
        '\u{FB00}' => vec!['f', 'f'],
        '\u{FB01}' => vec!['f', 'i'],
        '\u{FB02}' => vec!['f', 'l'],
        '\u{FB03}' => vec!['f', 'f', 'i'],
        '\u{FB04}' => vec!['f', 'f', 'l'],
        '\u{FB05}' => vec!['s', 't'],
        '\u{FB06}' => vec!['s', 't'],
        _ => vec![ch],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(text: &str, x: f64, y: f64, width: f64) -> TextChunk {
        TextChunk {
            text: text.to_string(),
            x,
            y,
            font_size: 10.0,
            font_name: "F1".to_string(),
            width,
            is_rtl: false,
            is_vertical: false,
            is_invisible: false,
            is_actual_text: false,
        }
    }

    #[test]
    fn semantic_model_builds_words_and_character_quads() {
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![
                chunk("Hello", 10.0, 150.0, 30.0),
                chunk("world", 44.0, 150.0, 35.0),
            ],
            &TextSemanticOptions::default(),
        );

        assert_eq!(page.counters.words, 2);
        assert_eq!(page.blocks[0].lines[0].words[0].text, "Hello");
        assert_eq!(page.blocks[0].lines[0].words[1].text, "world");
        assert!(page.blocks[0].lines[0].words[0].quad.x1 <= 45.0);
    }

    #[test]
    fn actual_text_and_invisible_provenance_are_reported() {
        let mut actual = chunk("office", 10.0, 100.0, 50.0);
        actual.is_actual_text = true;
        let mut hidden = chunk("ocr", 10.0, 80.0, 20.0);
        hidden.is_invisible = true;
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![actual, hidden],
            &TextSemanticOptions::default(),
        );

        assert_eq!(page.counters.mapped_via_actual_text, 6);
        assert_eq!(page.counters.hidden_or_invisible, 1);
        assert!(page
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "text.actual_text.used"));
    }

    #[test]
    fn visible_text_mode_excludes_hidden_chunks() {
        let mut hidden = chunk("hidden", 10.0, 100.0, 30.0);
        hidden.is_invisible = true;
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![chunk("visible", 10.0, 130.0, 40.0), hidden],
            &TextSemanticOptions::visible_text(),
        );

        assert_eq!(page.text(), "visible");
        assert_eq!(page.counters.hidden_or_invisible, 1);
    }

    #[test]
    fn search_matches_ligatures_and_returns_quads() {
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![chunk("of\u{FB01}ce", 10.0, 100.0, 40.0)],
            &TextSemanticOptions::default(),
        );
        let doc = build_text_semantic_document(vec![page], Vec::new());
        let options = TextSearchOptions {
            case_sensitive: false,
            ..Default::default()
        };

        let matches = doc.search("office", &options);
        assert_eq!(matches.len(), 1);
        assert!(!matches[0].quads.is_empty());
    }

    #[test]
    fn search_matches_hyphenated_line_breaks() {
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![
                chunk("hyphen-", 10.0, 120.0, 45.0),
                chunk("ated", 10.0, 100.0, 25.0),
            ],
            &TextSemanticOptions::default(),
        );
        let doc = build_text_semantic_document(vec![page], Vec::new());

        let matches = doc.search("hyphenated", &TextSearchOptions::default());
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn cjk_text_is_tokenized_character_by_character() {
        let page = build_text_semantic_page(
            1,
            [0.0, 0.0, 200.0, 200.0],
            vec![chunk("\u{4F60}\u{597D}", 10.0, 100.0, 20.0)],
            &TextSemanticOptions::default(),
        );

        let words = &page.blocks[0].lines[0].words;
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "\u{4F60}");
        assert_eq!(words[1].text, "\u{597D}");
    }
}
