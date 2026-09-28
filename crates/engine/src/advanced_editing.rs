//! Combined advanced editing advanced editing primitives.
//!
//! This module is the shared implementation surface for vertical/RTL text
//! analysis, byte-preserving text patching, vector-object editing, and ink
//! curve fitting.  Existing PDF glyph streams remain provenance-bearing PDF
//! codes; only newly inserted Unicode text is shaped.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_bidi::{BidiInfo, Level};
use unicode_segmentation::UnicodeSegmentation;

use crate::error::{Result, WellfriendError};
use crate::filters::{
    decode_stream_lossless_with_limits, flate_encode_cancellable, DecodeLimits, StreamDecodeStatus,
};
use crate::fonts::sfnt_subset::subset_glyf_preserving_gids;
use crate::fonts::{FontResolver, FontType, ShapeOptions, TextDirection, TextShaper};
use crate::object::PdfObject;
use crate::render::get_fallback_font;
use crate::secure_mutation::{
    analyze_edit_policy, EditOperation as SignatureEditOperation, EditPolicyDecision,
    EditPolicyReport,
};
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::{ContentEngine, PageResources};
#[cfg(test)]
#[path = "advanced_cff_tests.rs"]
mod cff_tests;
#[cfg(test)]
#[path = "advanced_coverage_tests.rs"]
mod coverage_tests;
#[path = "advanced_form_text.rs"]
pub mod form_text;
#[cfg(test)]
#[path = "advanced_generated_carrier_tests.rs"]
mod generated_carrier_tests;
#[cfg(test)]
#[path = "advanced_hard_break_tests.rs"]
mod hard_break_tests;
#[path = "advanced_inline_text.rs"]
mod inline_text;
#[path = "ocr_carriers.rs"]
pub mod ocr_carriers;
#[cfg(test)]
#[path = "advanced_predefined_cmap_tests.rs"]
mod predefined_cmap_tests;
#[path = "advanced_story_carriers.rs"]
pub(crate) mod story_carriers;
#[path = "advanced_story_vertical.rs"]
mod story_vertical;
#[path = "advanced_text_resources.rs"]
mod text_resources;
#[cfg(test)]
#[path = "advanced_variable_cmap_tests.rs"]
mod variable_cmap_tests;
#[path = "advanced_vector_occurrence.rs"]
mod vector_occurrence;
#[path = "advanced_vertical_text.rs"]
mod vertical_text;

pub const ADVANCED_EDITING_SCHEMA_VERSION: &str =
    "advanced_editing.vertical-rtl-patch-vector-ink-editing.v1";

pub const MAX_ADVANCED_EDITING_PARAGRAPH_CHARS: usize = 1_000_000;
pub const MAX_ADVANCED_EDITING_BIDI_RUNS: usize = 4096;
pub const MAX_ADVANCED_EDITING_GLYPHS: usize = 1_000_000;
pub const MAX_ADVANCED_EDITING_INK_POINTS: usize = 1_000_000;
pub const MAX_ADVANCED_EDITING_INK_SEGMENTS: usize = 100_000;
pub const MAX_ADVANCED_EDITING_FIT_RECURSION: usize = 32;
pub const MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES: usize = 256 * 1024 * 1024;
const EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvancedEditingSupportStatus {
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedExact,
    UnsupportedReportedSecurityPolicy,
    NotInAdvancedEditingScope,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvancedTextMode {
    SafePatch,
    ParagraphReflowHorizontal,
    ParagraphReflowRtl,
    ParagraphReflowVertical,
    OverlayFallback,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerticalGlyphOrientation {
    Upright,
    RotateClockwise,
    FontVerticalAlternate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextReflowLimits {
    pub max_paragraph_chars: usize,
    pub max_bidi_runs: usize,
    pub max_glyphs: usize,
}

impl Default for TextReflowLimits {
    fn default() -> Self {
        Self {
            max_paragraph_chars: MAX_ADVANCED_EDITING_PARAGRAPH_CHARS,
            max_bidi_runs: MAX_ADVANCED_EDITING_BIDI_RUNS,
            max_glyphs: MAX_ADVANCED_EDITING_GLYPHS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BidiRunProvenance {
    pub logical_byte_start: usize,
    pub logical_byte_end: usize,
    pub visual_run_index: usize,
    pub embedding_level: u8,
    pub right_to_left: bool,
    pub logical_text: String,
    pub visual_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextGlyphProvenance {
    pub glyph_id: u16,
    pub source_cluster_utf8: u32,
    pub source_run_index: usize,
    pub advance_1000: f64,
    pub offset_x_1000: f64,
    pub offset_y_1000: f64,
    pub orientation: VerticalGlyphOrientation,
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextReflowAnalysis {
    pub schema_version: String,
    pub status: AdvancedEditingSupportStatus,
    pub mode: AdvancedTextMode,
    pub logical_text: String,
    pub visual_text: String,
    pub base_direction: String,
    pub writing_mode: i32,
    pub bidi_runs: Vec<BidiRunProvenance>,
    pub glyphs: Vec<TextGlyphProvenance>,
    pub missing_glyph_clusters: Vec<u32>,
    pub used_complex_shaping: bool,
    pub existing_pdf_glyphs_reshaped: bool,
    pub deterministic: bool,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOverflowPolicy {
    Error,
    Clip,
    ExpandRegion,
}

/// Horizontal alignment for generated advanced editing text.  `Justify` changes
/// text-state spacing (`Tw`/`Tc`) rather than scaling outlines or drawing a
/// replacement path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedTextAlignment {
    #[default]
    Left,
    Right,
    Center,
    Start,
    End,
    Justify,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedLineAdjustment {
    pub line_index: usize,
    pub natural_width: f64,
    pub target_width: f64,
    pub residual: f64,
    pub word_spacing: f64,
    pub character_spacing: f64,
    pub alignment: GeneratedTextAlignment,
    pub last_line: bool,
    pub applied: bool,
    pub refusal_reason: Option<String>,
}

/// A final logical line and the glyph sequence that will be painted for it.
/// The only visual/logical divergence currently supported is one end-of-line
/// dictionary hyphen whose CID has an empty ToUnicode mapping. That narrow rule
/// keeps PDF extraction equal to the requested source text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplicitLayoutLine {
    pub logical_text: String,
    pub visual_text: String,
    #[serde(default)]
    pub bidi: Option<crate::fonts::shaper::LineBidi>,
    #[serde(default)]
    pub inserted_visual_hyphen: bool,
}

/// One final logical line at an explicit user-space line rectangle. This is a
/// canonical source-writer input for bounded document-flow operations; it does
/// not introduce a second display-list or text serializer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionedExplicitLayoutLine {
    pub line: ExplicitLayoutLine,
    pub region: [f64; 4],
}

/// Governs where a generated replacement is painted when its logical source
/// selection crosses more than one PDF text object.
///
/// A single logical paragraph can be split across several `BT`/`ET` paint
/// slots with images, paths, shadings, or unrelated text between them. There
/// is no source-independent way to collapse that paragraph into one generated
/// block without choosing a new relative stacking position. The default keeps
/// that choice outside the engine: one source slot is accepted, while a
/// multi-slot replacement needs an explicit first/last-slot approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratedPaintOrderPolicy {
    #[default]
    RequireSingleSourceTextObject,
    AnchorAfterFirstSourceTextObject,
    AnchorAfterLastSourceTextObject,
}

/// Explicitly maps a contiguous part of replacement Unicode to one original
/// source text-object paint slot. Emitting every partition after its own `ET`
/// preserves intervening non-text paint instead of moving the complete
/// replacement to one side of that paint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedPaintPartition {
    pub source_text_object: usize,
    /// Unicode scalar offsets into the complete replacement string.
    pub replacement_scalar_range: [usize; 2],
    pub region: [f64; 4],
    #[serde(default)]
    pub final_lines: Option<Vec<ExplicitLayoutLine>>,
}

/// Revision-bound, reviewable proposal for distributing replacement graphemes
/// across the exact source text-object paint slots selected by a logical edit.
/// It never invents page geometry: a host must approve one region per slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedPaintPartitionProposal {
    pub schema_version: String,
    pub status: AdvancedEditingSupportStatus,
    pub input_sha256: String,
    pub request_sha256: String,
    pub proposal_id: String,
    pub page: usize,
    pub logical_range: [usize; 2],
    pub replacement_sha256: String,
    pub candidates: Vec<GeneratedPaintPartitionCandidate>,
    pub deterministic: bool,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedPaintPartitionCandidate {
    pub source_text_object: usize,
    pub selected_source_scalar_count: usize,
    pub replacement_scalar_range: [usize; 2],
    pub selected_span_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_region: Option<[f64; 4]>,
    pub start_boundary_class: String,
    pub end_boundary_class: String,
}

/// The approval repeats only decisions which cannot be inferred from PDF text
/// provenance: the physical region and optional final layout for each slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedPaintPartitionApproval {
    pub proposal_id: String,
    /// Exact approved shaping font bytes. `None` means the retained/source
    /// font route; a supplied apply font must match this digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_sha256: Option<String>,
    pub partitions: Vec<GeneratedPaintPartitionApprovalEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedPaintPartitionApprovalEntry {
    pub source_text_object: usize,
    pub region: [f64; 4],
    #[serde(default)]
    pub final_lines: Option<Vec<ExplicitLayoutLine>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedTextEditOptions {
    pub region: [f64; 4],
    pub font_size: f64,
    pub line_spacing: f64,
    pub max_lines_or_columns: usize,
    pub overflow_policy: TextOverflowPolicy,
    pub signature_policy_override: bool,
    pub deterministic: bool,
    #[serde(default)]
    pub alignment: GeneratedTextAlignment,
    #[serde(default)]
    pub justify_last_line: bool,
    /// Maximum emitted `Tw`, expressed in unscaled text-space units.
    #[serde(default = "default_max_word_spacing")]
    pub max_word_spacing: f64,
    /// Maximum emitted `Tc`, expressed in unscaled text-space units.
    #[serde(default = "default_max_character_spacing")]
    pub max_character_spacing: f64,
    #[serde(default)]
    pub target_stream_object: Option<u32>,
    #[serde(default)]
    pub target_stream_generation: Option<u16>,
    #[serde(default)]
    pub target_decoded_byte_range: Option<[usize; 2]>,
    /// Explicit stacking-order decision for a generated replacement whose
    /// source selection spans several `BT`/`ET` paint slots.
    #[serde(default)]
    pub paint_order_policy: GeneratedPaintOrderPolicy,
    /// Optional exact partition plan for preserving non-text paint between
    /// several selected source text objects.
    #[serde(default)]
    pub paint_partitions: Vec<GeneratedPaintPartition>,
}

fn default_max_word_spacing() -> f64 {
    0.5
}

fn default_max_character_spacing() -> f64 {
    0.05
}

impl Default for AdvancedTextEditOptions {
    fn default() -> Self {
        Self {
            region: [36.0, 36.0, 576.0, 756.0],
            font_size: 12.0,
            line_spacing: 1.2,
            max_lines_or_columns: 4096,
            overflow_policy: TextOverflowPolicy::Error,
            signature_policy_override: false,
            deterministic: true,
            alignment: GeneratedTextAlignment::Left,
            justify_last_line: false,
            max_word_spacing: default_max_word_spacing(),
            max_character_spacing: default_max_character_spacing(),
            target_stream_object: None,
            target_stream_generation: None,
            target_decoded_byte_range: None,
            paint_order_policy: GeneratedPaintOrderPolicy::default(),
            paint_partitions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AdvancedTextEditReport {
    pub schema_version: String,
    pub status: AdvancedEditingSupportStatus,
    pub mode: AdvancedTextMode,
    pub page: usize,
    pub source_stream_object: u32,
    pub source_operator: String,
    pub old_text: String,
    pub new_text: String,
    pub writing_mode: i32,
    pub font_resource: String,
    pub shaped_glyphs: usize,
    pub lines_or_columns: usize,
    pub logical_to_visual_runs: Vec<BidiRunProvenance>,
    pub cluster_provenance: Vec<TextGlyphProvenance>,
    pub removed_old_reachable_content: bool,
    pub replacement_extracts: bool,
    pub old_text_absent: bool,
    pub output_reopened: bool,
    pub original_prefix_preserved: bool,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub signature_policy: EditPolicyReport,
    pub cryptographic_validity_claimed: bool,
    pub deterministic: bool,
    pub cache_invalidation: CacheInvalidationReport,
    pub line_adjustments: Vec<GeneratedLineAdjustment>,
    pub exact_limits: Vec<String>,
}

/// A logical, page-local selection over provenance-bearing text-showing operands.
/// Offsets are Unicode scalar offsets, never x-coordinate guesses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRunTextRangeRequest {
    pub page: usize,
    pub logical_start: usize,
    pub logical_end: usize,
    pub replacement_text: String,
    pub mode: AdvancedTextMode,
    #[serde(default)]
    pub style_policy: MultiRunStylePolicy,
    #[serde(default)]
    pub options: AdvancedTextEditOptions,
    /// Optional text reflow final visual layout. When supplied it must cover the
    /// logical replacement exactly and is serialized through this existing
    /// range-edit source mutation path.
    #[serde(default)]
    pub final_lines: Option<Vec<ExplicitLayoutLine>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MultiRunStylePolicy {
    #[default]
    InheritLeading,
    InheritTrailing,
    PreservePerSegment,
    ExplicitSupplied,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiRunSourceSpan {
    pub span_id: String,
    pub stream_object: u32,
    pub stream_generation: u16,
    pub operator: String,
    pub tj_element: Option<usize>,
    pub byte_range: [usize; 2],
    pub logical_range: [usize; 2],
    /// Zero-based `BT`/`ET` paint slot in the exact page/Form content sequence.
    /// The ordinal remains continuous across ordered `/Contents` members.
    pub source_text_object: usize,
    pub font_resource: String,
    pub font_size: f64,
    pub character_spacing: f64,
    pub word_spacing: f64,
    pub horizontal_scaling: f64,
    pub writing_mode: i32,
    pub text_render_mode: i32,
    pub marked_content_depth: usize,
    /// Stable private ownership emitted by fresh authored typed tables. Both
    /// values are present together or absent; callers must never infer an
    /// owner from duplicated visible text.
    #[serde(default)]
    pub authored_typed_table: Option<String>,
    #[serde(default)]
    pub authored_typed_cell: Option<String>,
    #[serde(default)]
    pub authored_typed_region: Option<[f64; 4]>,
    /// Nearest direct `/ActualText` carrier active for this source operand.
    /// The source identity lets consumers count one logical carrier once even
    /// when a shaped run emits several glyph-showing operands beneath it.
    #[serde(default)]
    pub direct_actual_text: Option<String>,
    #[serde(default)]
    pub direct_actual_text_source: Option<String>,
    /// Unmarked content or direct logical-text-only /Span scopes. This is not
    /// permission to migrate MCIDs, optional content or annotation ownership.
    #[serde(default)]
    pub flow_relocatable: bool,
    /// Unicode decoded directly from the physical source codes, before an
    /// enclosing `/ActualText` value replaces their logical representation.
    /// OCR provenance and source-code relocation must bind this value; user
    /// selection and extraction continue to use `text` and `logical_range`.
    pub source_text: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiRunRangeModel {
    pub schema_version: String,
    pub status: AdvancedEditingSupportStatus,
    pub page: usize,
    pub paragraph_block_id: String,
    pub logical_text: String,
    pub source_spans: Vec<MultiRunSourceSpan>,
    pub logical_to_visual_runs: Vec<BidiRunProvenance>,
    pub writing_mode: i32,
    pub deterministic: bool,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiRunTextEditReport {
    pub schema_version: String,
    pub status: AdvancedEditingSupportStatus,
    pub operation: String,
    pub page: usize,
    pub logical_range: [usize; 2],
    pub selected_source_spans: Vec<MultiRunSourceSpan>,
    pub style_policy: MultiRunStylePolicy,
    /// True only when the published paint references a newly embedded
    /// generated/approved font. Merely staging or retaining an unused font
    /// definition must not trigger a substitution approval.
    pub generated_font_used: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_paint_order: Option<GeneratedPaintOrderDecision>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub generated_paint_partitions: Vec<GeneratedPaintPartitionReceipt>,
    pub replacement_text: String,
    pub replacement_extracts: bool,
    pub old_selected_text_absent: bool,
    pub unrelated_text_preserved: bool,
    pub reachable_source_tokens_removed: bool,
    pub output_reopened: bool,
    pub original_prefix_preserved: bool,
    pub output_sha256: String,
    pub signature_policy: EditPolicyReport,
    pub cryptographic_validity_claimed: bool,
    pub deterministic: bool,
    pub cache_invalidation: CacheInvalidationReport,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedPaintOrderDecision {
    pub policy: GeneratedPaintOrderPolicy,
    pub source_text_objects: usize,
    pub anchor_stream_object: u32,
    pub anchor_stream_generation: u16,
    pub anchor_decoded_byte_offset: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GeneratedPaintPartitionReceipt {
    pub source_text_object: usize,
    pub replacement_scalar_range: [usize; 2],
    pub region: [f64; 4],
    pub generated: bool,
    pub anchor_stream_object: u32,
    pub anchor_stream_generation: u16,
    pub anchor_decoded_byte_offset: usize,
}

/// Analyze newly inserted Unicode text for bounded RTL or vertical reflow.
///
/// `font_bytes` is the exact font that will be embedded or reused. Supplying
/// `None` selects Wellfriend's bundled DejaVu Sans, which covers Arabic and Hebrew
/// but intentionally does not claim CJK coverage. Missing glyphs are reported
/// and make the result unsupported instead of silently substituting `.notdef`.
pub fn analyze_advanced_text_reflow(
    text: &str,
    mode: AdvancedTextMode,
    font_bytes: Option<&[u8]>,
    limits: TextReflowLimits,
) -> Result<TextReflowAnalysis> {
    if !matches!(
        mode,
        AdvancedTextMode::ParagraphReflowHorizontal
            | AdvancedTextMode::ParagraphReflowRtl
            | AdvancedTextMode::ParagraphReflowVertical
    ) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing text analysis requires a paragraph reflow mode, got {mode:?}"
        )));
    }
    let char_count = text.chars().count();
    if char_count
        > limits
            .max_paragraph_chars
            .min(MAX_ADVANCED_EDITING_PARAGRAPH_CHARS)
    {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing paragraph has {char_count} characters; limit is {}",
            limits
                .max_paragraph_chars
                .min(MAX_ADVANCED_EDITING_PARAGRAPH_CHARS)
        )));
    }
    reject_unsafe_text_controls(text)?;

    let base_level = match mode {
        AdvancedTextMode::ParagraphReflowRtl => Level::rtl(),
        _ => Level::ltr(),
    };
    let bidi = BidiInfo::new(text, Some(base_level));
    let mut visual_text = String::new();
    for line in crate::fonts::hard_break::logical_lines(text) {
        let line = line?;
        if !line.visible.is_empty() {
            let paragraph = bidi
                .paragraphs
                .get(
                    bidi.paragraphs
                        .partition_point(|p| p.range.end <= line.visible.start),
                )
                .filter(|p| p.range.start <= line.visible.start && line.visible.end <= p.range.end)
                .ok_or_else(|| {
                    WellfriendError::invalid_input("analysis hard line crosses a bidi paragraph")
                })?;
            visual_text.push_str(&bidi.reorder_line(paragraph, line.visible.clone()));
        }
        visual_text.push_str(&text[line.visible.end..line.logical.end]);
    }
    let mut runs = Vec::new();
    for paragraph in &bidi.paragraphs {
        crate::cancel::check_current_cancel("advanced editing bidi analysis")?;
        let (levels, ranges) = bidi.visual_runs(paragraph, paragraph.range.clone());
        for (run_index, range) in ranges.into_iter().enumerate() {
            // visual_runs returns byte-indexed levels, not one level per run.
            let level = levels[range.start];
            if runs.len() >= limits.max_bidi_runs.min(MAX_ADVANCED_EDITING_BIDI_RUNS) {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "advanced_editing bidi run count exceeds limit {}",
                    limits.max_bidi_runs.min(MAX_ADVANCED_EDITING_BIDI_RUNS)
                )));
            }
            let logical = text.get(range.clone()).ok_or_else(|| {
                WellfriendError::ParseError(
                    "advanced_editing bidi run is not on UTF-8 boundaries".to_string(),
                )
            })?;
            let visual = if level.is_rtl() {
                logical.graphemes(true).rev().collect()
            } else {
                logical.to_string()
            };
            runs.push(BidiRunProvenance {
                logical_byte_start: range.start,
                logical_byte_end: range.end,
                visual_run_index: run_index,
                embedding_level: level.number(),
                right_to_left: level.is_rtl(),
                logical_text: logical.to_string(),
                visual_text: visual,
            });
        }
    }

    let font = font_bytes
        .or_else(|| get_fallback_font("Symbol"))
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing bundled fallback font unavailable".to_string(),
            )
        })?;
    let mut glyphs = Vec::new();
    let mut missing = Vec::new();
    let mut complex = false;
    let mut run_lookup = runs
        .iter()
        .enumerate()
        .map(|(index, run)| (run.logical_byte_start, run.logical_byte_end, index))
        .collect::<Vec<_>>();
    run_lookup.sort_unstable_by_key(|run| run.0);
    let prepared = crate::fonts::shaper::ParagraphBidi::new(
        text,
        ShapeOptions {
            direction: Some(if base_level.is_rtl() {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            }),
        },
    )?;
    prepared.all_hard_lines(|range, levels| {
        let visible = &text[range.clone()];
        let vertical_run = if mode == AdvancedTextMode::ParagraphReflowVertical {
            Some(crate::fonts::vertical::shape_resolved(
                font,
                visible,
                &levels,
                &Default::default(),
            )?)
        } else {
            None
        };
        let shaped = if let Some(vertical) = &vertical_run {
            crate::fonts::ShapedRun {
                glyphs: vertical.iter().map(|g| g.glyph.clone()).collect(),
                direction: TextDirection::LeftToRight,
                used_complex_shaping: !vertical.is_empty(),
            }
        } else {
            TextShaper::shape_resolved(font, visible, &levels, &Default::default())?
        };
        complex |= shaped.used_complex_shaping;
        let local_missing = crate::fonts::shaper::missing_glyph_clusters(font, visible, &shaped)?;
        missing.extend(
            local_missing
                .iter()
                .map(|cluster| range.start as u32 + cluster),
        );
        for (glyph_index, glyph) in shaped.glyphs.into_iter().enumerate() {
            if glyphs.len() >= limits.max_glyphs.min(MAX_ADVANCED_EDITING_GLYPHS) {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "advanced_editing shaped glyph count exceeds limit {}",
                    limits.max_glyphs.min(MAX_ADVANCED_EDITING_GLYPHS)
                )));
            }
            let is_missing = local_missing.binary_search(&glyph.cluster).is_ok();
            let source_cluster = range.start as u32 + glyph.cluster;
            let source_run_index = run_lookup
                .get(
                    run_lookup
                        .partition_point(|run| run.0 <= source_cluster as usize)
                        .saturating_sub(1),
                )
                .filter(|run| (source_cluster as usize) < run.1)
                .map(|run| run.2)
                .ok_or_else(|| {
                    WellfriendError::invalid_input("shaped glyph lacks source bidi run")
                })?;
            let orientation = match vertical_run.as_ref().map(|g| &g[glyph_index]) {
                Some(glyph) if glyph.rotate_clockwise => VerticalGlyphOrientation::RotateClockwise,
                Some(glyph) if glyph.vertical_alternate => {
                    VerticalGlyphOrientation::FontVerticalAlternate
                }
                _ => VerticalGlyphOrientation::Upright,
            };
            glyphs.push(TextGlyphProvenance {
                glyph_id: glyph.glyph_id,
                source_cluster_utf8: source_cluster,
                source_run_index,
                advance_1000: canonical_number(glyph.advance),
                offset_x_1000: canonical_number(glyph.offset_x),
                offset_y_1000: canonical_number(glyph.offset_y),
                orientation,
                missing: is_missing,
            });
        }
        Ok(true)
    })?;
    missing.sort_unstable();
    missing.dedup();
    Ok(TextReflowAnalysis {
        schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
        status: if missing.is_empty() {
            AdvancedEditingSupportStatus::ImplementedWithLimits
        } else {
            AdvancedEditingSupportStatus::UnsupportedReportedExact
        },
        mode,
        logical_text: text.to_string(),
        visual_text,
        base_direction: if base_level.is_rtl() { "rtl" } else { "ltr" }.to_string(),
        writing_mode: i32::from(mode == AdvancedTextMode::ParagraphReflowVertical),
        bidi_runs: runs,
        glyphs,
        missing_glyph_clusters: missing,
        used_complex_shaping: complex,
        existing_pdf_glyphs_reshaped: false,
        deterministic: true,
        exact_limits: vec![
            "analysis applies only to newly inserted Unicode; existing PDF codes/CIDs/GIDs are not reshaped".to_string(),
            "vertical analysis uses Unicode 17 orientation and OpenType vertical substitutions/metrics; final columns are reshaped at their logical boundaries".to_string(),
            "missing glyphs are fail-closed and never silently replaced by .notdef".to_string(),
            "coverage validates final shaped clusters and subset outlines, not nominal cmap alone; it does not establish intended typography, arbitrary glyphless logical persistence, or visual fidelity".to_string(),
        ],
    })
}

#[derive(Debug, Clone)]
struct GeneratedGlyph {
    cid: u16,
    gid: u16,
    logical_byte_start: usize,
    visual_unicode: String,
    to_unicode: Option<String>,
    advance: f64,
    offset_x: f64,
    offset_y: f64,
    orientation: VerticalGlyphOrientation,
    cross_advance: f64,
    font_width: f64,
    bounds: Option<[f64; 4]>,
}

#[derive(Debug, Clone)]
struct StoryPaintedRun {
    font_index: usize,
    style_index: usize,
    /// Set for positioned tab fields; `tab_field` identifies when a new field
    /// must reset the inline pen even if two origins are numerically equal.
    inline_origin: Option<f64>,
    tab_field: Option<usize>,
    glyphs: Vec<GeneratedGlyph>,
}
type StoryPaintedLines = Vec<Vec<StoryPaintedRun>>;

#[derive(Debug, Clone)]
pub(crate) struct InvisibleUnicodeTextRun {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub font_size: f64,
    pub target_width: f64,
}

/// Append a non-painting, searchable Unicode layer to one existing page. Each
/// positioned run is shaped with the exact supplied sfnt program, assigned its
/// own CID sequence, mapped through `/ToUnicode`, and wrapped in `/ActualText`
/// so logical extraction remains independent of visual bidi glyph order.
pub(crate) fn append_invisible_unicode_text_layer(
    input: &[u8],
    page_number: usize,
    runs: &[InvisibleUnicodeTextRun],
    font: &[u8],
) -> Result<(Vec<u8>, String)> {
    append_unicode_text_layer(input, page_number, runs, font, 3, None)
}

/// Relocate one uniquely identified invisible `/ActualText` carrier without
/// routing it through visual-region reflow. Invisible OCR (`Tr 3`) is absent
/// from the visual scene by design, so geometry correction must bind to its
/// content-stream provenance instead of pretending it owns painted bounds.
///
/// The operation keeps the existing embedded font, CIDs, `ToUnicode`,
/// `/ActualText`, horizontal scaling, and glyph advances. It publishes an
/// absolute text matrix immediately before every selected showing operator,
/// translating the complete shaped run as one unit while preserving relative
/// glyph placement and all unrelated content bytes.
pub(crate) fn relocate_invisible_actual_text(
    input: &[u8],
    page_number: usize,
    logical_text: &str,
    bounds: [f64; 4],
) -> Result<(Vec<u8>, serde_json::Value)> {
    if logical_text.is_empty()
        || bounds.iter().any(|value| !value.is_finite())
        || bounds[0] >= bounds[2]
        || bounds[1] >= bounds[3]
    {
        return Err(WellfriendError::invalid_input(
            "advanced_editing invisible OCR relocation requires nonempty text and finite positive bounds",
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut scanner_state = ScannedTextTokenState::default();
    let mut position_metrics = inline_text::Metrics::new(&resources, reader);
    let mut stream_sources = BTreeMap::<(u32, u16), (PdfObject, Arc<Vec<u8>>)>::new();
    let mut selected = Vec::<(u32, u16, ContentStringToken, [f64; 6])>::new();
    let mut carrier_keys = BTreeSet::<(u32, u16, usize, usize)>::new();
    let mut non_invisible_carrier_keys = BTreeSet::<(u32, u16, usize, usize)>::new();

    for (number, generation) in page.contents.iter().copied() {
        let object = reader.get_object(number, generation)?;
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            scanner_state = ScannedTextTokenState::default();
            continue;
        }
        let decoded = Arc::new(decoded.data);
        let tokens = scan_text_string_tokens_with_metrics(
            decoded.as_slice(),
            &mut scanner_state,
            Some((number, generation)),
            Some(&mut position_metrics),
        )?;
        for token in tokens {
            let Some(carrier) = token
                .actual_text_sources
                .last()
                .filter(|carrier| carrier.logical_text.as_ref() == logical_text)
            else {
                continue;
            };
            let carrier_key = (
                carrier.owner_object,
                carrier.owner_generation,
                carrier.value_start,
                carrier.value_end,
            );
            if token.text_render_mode != 3 {
                non_invisible_carrier_keys.insert(carrier_key);
                continue;
            }
            if !matches!(token.operator.as_str(), "Tj" | "TJ") {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing invisible OCR relocation requires Tj/TJ source operators"
                        .into(),
                ));
            }
            let matrix = token
                .source_position
                .as_ref()
                .map(inline_text::PositionSnapshot::matrix)
                .ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "advanced_editing invisible OCR relocation could not recover an exact source text matrix"
                            .into(),
                    )
                })?;
            if matrix[..4]
                .iter()
                .zip([1.0, 0.0, 0.0, 1.0])
                .any(|(actual, expected)| {
                    (*actual - expected).abs() > 1e-10 * actual.abs().max(1.0)
                })
            {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing invisible OCR relocation currently requires the canonical horizontal OCR text basis"
                        .into(),
                ));
            }
            carrier_keys.insert(carrier_key);
            selected.push((number, generation, token, matrix));
        }
        stream_sources.insert((number, generation), (object, decoded));
    }
    if carrier_keys.len() != 1 || selected.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing invisible OCR relocation requires one exact /ActualText carrier; found {}",
            carrier_keys.len()
        )));
    }
    if carrier_keys
        .iter()
        .any(|carrier| non_invisible_carrier_keys.contains(carrier))
    {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing OCR geometry carrier mixes visible and invisible paint and cannot be relocated atomically"
                .into(),
        ));
    }
    selected.sort_by_key(|item| (item.0, item.1, item.2.operation_start));
    let first = &selected[0];
    let font_size = first.2.font_size;
    if !font_size.is_finite() || font_size <= 0.0 {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing invisible OCR carrier has an invalid source font size".into(),
        ));
    }
    let height = bounds[3] - bounds[1];
    let target_origin = [
        bounds[0],
        bounds[1] + height.min(font_size).max(font_size * 0.75),
    ];
    let delta = [target_origin[0] - first.3[4], target_origin[1] - first.3[5]];
    let mut edits = DecodedStreamEdits::new();
    for (number, generation, token, mut matrix) in &selected {
        if token.operator == "TJ"
            && token.operation_start == token.token_start
            && token.operation_end == token.token_end
        {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing cannot relocate an invisible OCR TJ carrier whose enclosing operation crosses a physical /Contents boundary"
                    .into(),
            ));
        }
        matrix[4] += delta[0];
        matrix[5] += delta[1];
        let (_, decoded) = stream_sources.get(&(*number, *generation)).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing invisible OCR source stream disappeared".into(),
            )
        })?;
        let original = decoded
            .get(token.operation_start..token.operation_end)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "advanced_editing invisible OCR source operator range is invalid".into(),
                )
            })?;
        let mut replacement = format!(
            "{} {} {} {} {} {} Tm\n",
            fmt_num(matrix[0]),
            fmt_num(matrix[1]),
            fmt_num(matrix[2]),
            fmt_num(matrix[3]),
            fmt_num(matrix[4]),
            fmt_num(matrix[5])
        )
        .into_bytes();
        replacement.extend_from_slice(original);
        let (object, decoded) = stream_sources.get(&(*number, *generation)).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing invisible OCR source stream disappeared".into(),
            )
        })?;
        edits
            .entry((*number, *generation))
            .or_insert_with(|| {
                (
                    object.clone(),
                    Arc::clone(decoded).as_ref().clone(),
                    Vec::new(),
                )
            })
            .2
            .push((token.operation_start, token.operation_end, replacement));
    }
    let changed = materialize_decoded_stream_edits(
        edits,
        "advanced_editing invisible OCR geometry relocation",
    )?;
    let output = text_resources::write(reader, &page, &resources, changed)?;
    let extracted = ContentEngine::open_bytes(output.clone())?.get_page_text(page_number)?;
    if !output.starts_with(input) || !extracted.contains(logical_text) {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing invisible OCR relocation failed save/reopen/extraction proof".into(),
        ));
    }
    Ok((
        output.clone(),
        serde_json::json!({
            "operation": "relocate_invisible_actual_text",
            "page": page_number,
            "logical_text_sha256": format!("{:x}", Sha256::digest(logical_text.as_bytes())),
            "source_origin": [first.3[4], first.3[5]],
            "target_origin": target_origin,
            "translation": delta,
            "source_showing_operators_repositioned": selected.len(),
            "text_rendering_mode": 3,
            "actual_text_carrier_bound": true,
            "original_prefix_preserved": output.starts_with(input),
            "output_reopened_and_text_extracted": true,
            "output_sha256": format!("{:x}", Sha256::digest(&output)),
        }),
    ))
}

/// Append visible, source-editable Unicode text to a reconstructed scan. The
/// text is a genuine embedded Type0/CID text layer, not rasterized lettering or
/// a browser overlay. Each run carries ActualText so bidi visual order and
/// logical extraction remain independent.
pub(crate) fn append_visible_unicode_text_layer(
    input: &[u8],
    page_number: usize,
    runs: &[InvisibleUnicodeTextRun],
    font: &[u8],
    fill_rgb: [f64; 3],
) -> Result<(Vec<u8>, String)> {
    if fill_rgb
        .iter()
        .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
    {
        return Err(WellfriendError::invalid_input(
            "advanced_editing visible Unicode fill color must contain finite 0..1 RGB values",
        ));
    }
    append_unicode_text_layer(input, page_number, runs, font, 0, Some(fill_rgb))
}

fn append_unicode_text_layer(
    input: &[u8],
    page_number: usize,
    runs: &[InvisibleUnicodeTextRun],
    font: &[u8],
    render_mode: i32,
    fill_rgb: Option<[f64; 3]>,
) -> Result<(Vec<u8>, String)> {
    if runs.is_empty() || runs.len() > 20_000 {
        return Err(WellfriendError::invalid_input(
            "advanced_editing invisible Unicode layer requires 1..=20000 runs",
        ));
    }
    ttf_parser::Face::parse(font, 0).map_err(|_| {
        WellfriendError::UnsupportedFeature(
            "advanced_editing invisible Unicode layer requires a valid caller-approved sfnt font"
                .to_string(),
        )
    })?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut planned = Vec::<(InvisibleUnicodeTextRun, Vec<GeneratedGlyph>)>::new();
    let mut all_glyphs = Vec::<GeneratedGlyph>::new();
    let mut next_cid = 1u32;
    for run in runs {
        if run.text.is_empty()
            || !run.x.is_finite()
            || !run.y.is_finite()
            || !run.font_size.is_finite()
            || run.font_size <= 0.0
            || run.font_size > 288.0
            || !run.target_width.is_finite()
            || run.target_width <= 0.0
        {
            return Err(WellfriendError::invalid_input(
                "advanced_editing invisible Unicode run text/position/font-size is invalid",
            ));
        }
        let mut glyphs =
            generated_glyph_plan(&run.text, AdvancedTextMode::ParagraphReflowHorizontal, font)?;
        if glyphs.is_empty() || glyphs.iter().any(|glyph| glyph.gid == 0) {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing approved OCR font lacks complete glyph coverage for a searchable run"
                    .to_string(),
            ));
        }
        for glyph in &mut glyphs {
            glyph.cid = u16::try_from(next_cid).map_err(|_| {
                WellfriendError::ResourceLimit(
                    "advanced_editing Unicode layer exceeds 65535 shaped glyphs".to_string(),
                )
            })?;
            // The public glyph bound is far below u32::MAX; ordinary addition
            // keeps the next iteration detectable by the u16 conversion
            // instead of pinning every later glyph to one saturated CID.
            next_cid += 1;
        }
        all_glyphs.extend(glyphs.iter().cloned());
        planned.push((run.clone(), glyphs));
    }
    let base =
        reserve_advanced_object_block(reader, 8, "advanced_editing invisible Unicode layer")?;
    let isolation_prefix_number = base + 6;
    let content_number = base + 7;
    let font_resource = deterministic_font_resource_name(reader, &page.resources);
    let mut changed = build_type0_font_objects(
        font,
        &all_glyphs,
        false,
        base,
        base + 1,
        base + 2,
        base + 3,
        base + 4,
        base + 5,
    )?;
    // Page `/Contents` members form one logical stream. A `q` at the start of
    // an appended member merely saves the state left by prior content; it does
    // not reset the CTM, clip, alpha, blend mode, or colour. Put a save before
    // all existing members and restore it here, then isolate this layer in its
    // own balanced scope. This is the standard way to regain the page-entry
    // graphics state without rewriting otherwise unrelated source streams.
    let mut content = String::from("Q\nq\n");
    for (run, glyphs) in &planned {
        let natural_width =
            glyphs.iter().map(|glyph| glyph.advance.abs()).sum::<f64>() / 1000.0 * run.font_size;
        if !natural_width.is_finite() || natural_width <= 0.0 {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing invisible Unicode run has no finite shaped advance".to_string(),
            ));
        }
        let horizontal_scale = (run.target_width / natural_width * 100.0).clamp(0.01, 10_000.0);
        let color = fill_rgb
            .map(|rgb| {
                format!(
                    "{} {} {} rg ",
                    fmt_num(rgb[0]),
                    fmt_num(rgb[1]),
                    fmt_num(rgb[2])
                )
            })
            .unwrap_or_default();
        content.push_str(&format!(
            "/Span << /ActualText <{}> >> BDC\nBT /{} {} Tf {render_mode} Tr {color}{} Tz\n",
            utf16be_hex_with_bom(&run.text),
            font_resource,
            fmt_num(run.font_size),
            fmt_num(horizontal_scale),
        ));
        let scale = run.font_size / 1000.0;
        let horizontal_fraction = horizontal_scale / 100.0;
        let mut x = run.x;
        for glyph in glyphs {
            content.push_str(&format!(
                "1 0 0 1 {} {} Tm <{:04X}> Tj\n",
                fmt_num(x + glyph.offset_x * scale * horizontal_fraction),
                fmt_num(run.y + glyph.offset_y * scale),
                glyph.cid
            ));
            x += glyph.advance.abs() * scale * horizontal_fraction;
        }
        content.push_str("ET\nEMC\n");
    }
    content.push_str("Q\n");
    let compressed = flate_encode_cancellable(content.as_bytes(), 6)?;
    let mut content_dict = crate::PdfDictionary::empty();
    content_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    content_dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    changed.push(IncrementalObject {
        number: content_number,
        generation: 0,
        object: PdfObject::Stream {
            dict: content_dict,
            raw: compressed,
        },
    });
    let mut isolation_prefix_dict = crate::PdfDictionary::empty();
    isolation_prefix_dict.insert("Length", PdfObject::Integer(2));
    changed.push(IncrementalObject {
        number: isolation_prefix_number,
        generation: 0,
        object: PdfObject::Stream {
            dict: isolation_prefix_dict,
            raw: b"q\n".to_vec(),
        },
    });
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "advanced_editing invisible Unicode target page is not a dictionary".to_string(),
        )
    })?;
    let mut page_resources = page.resources.clone();
    let mut fonts = resolve_advanced_editing_dict(page_resources.get("Font"), reader)
        .unwrap_or_else(crate::PdfDictionary::empty);
    fonts.insert(
        font_resource.clone(),
        PdfObject::Reference {
            number: base + 5,
            generation: 0,
        },
    );
    page_resources.insert("Font", PdfObject::Dictionary(fonts));
    page_dict.insert("Resources", PdfObject::Dictionary(page_resources));
    let mut contents = vec![PdfObject::Reference {
        number: isolation_prefix_number,
        generation: 0,
    }];
    contents.extend(
        page.contents
            .iter()
            .map(|(number, generation)| PdfObject::Reference {
                number: *number,
                generation: *generation,
            }),
    );
    contents.push(PdfObject::Reference {
        number: content_number,
        generation: 0,
    });
    page_dict.insert("Contents", PdfObject::Array(contents));
    changed.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(page_dict),
    });
    let output = text_resources::write(reader, &page, &resources, changed)?;
    ContentEngine::open_bytes(output.clone())?;
    Ok((output, font_resource))
}

/// Replace one provenance-resolved PDF string with newly shaped Type0 text.
///
/// This bounded true-edit path removes the old string token from its owning
/// content stream, embeds the selected sfnt font as a CIDFontType2 with a
/// sequential CID-to-GID map and per-CID ToUnicode mapping, appends a new page
/// content stream, saves incrementally, and verifies reopen/extraction. A
/// paragraph spanning multiple PDF string tokens is rejected exactly rather
/// than hidden beneath an overlay.
pub fn edit_advanced_text_pdf(
    input: &[u8],
    page_number: usize,
    old_text: &str,
    new_text: &str,
    mode: AdvancedTextMode,
    options: &AdvancedTextEditOptions,
    font_bytes: Option<&[u8]>,
) -> Result<(Vec<u8>, AdvancedTextEditReport)> {
    edit_advanced_text_pdf_internal(
        input,
        page_number,
        old_text,
        new_text,
        mode,
        options,
        font_bytes,
        None,
        None,
    )
}

/// Replace one source string using caller-selected, grapheme-safe final lines.
///
/// The supplied lines must concatenate byte-for-byte to `new_text`.  This is
/// deliberately a layout boundary rather than a second writer: shaping,
/// generated Type0 resources, source-token removal, canonical incremental
/// serialization, reopen, and extraction verification all stay in the same
/// bounded advanced editing mutation path as [`edit_advanced_text_pdf`].
#[allow(clippy::too_many_arguments)] // Mirrors the stable public edit contract plus explicit final lines.
pub fn edit_advanced_text_pdf_with_layout(
    input: &[u8],
    page_number: usize,
    old_text: &str,
    new_text: &str,
    mode: AdvancedTextMode,
    options: &AdvancedTextEditOptions,
    font_bytes: Option<&[u8]>,
    final_lines: &[String],
) -> Result<(Vec<u8>, AdvancedTextEditReport)> {
    if final_lines.is_empty() || final_lines.concat() != new_text {
        return Err(WellfriendError::invalid_input(
            "advanced_editing explicit final lines must be nonempty and concatenate exactly to replacement text",
        ));
    }
    let explicit_lines = final_lines
        .iter()
        .map(|text| ExplicitLayoutLine {
            logical_text: text.clone(),
            visual_text: text
                .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                .to_string(),
            inserted_visual_hyphen: false,
            bidi: None,
        })
        .collect::<Vec<_>>();
    edit_advanced_text_pdf_with_visual_layout(
        input,
        page_number,
        old_text,
        new_text,
        mode,
        options,
        font_bytes,
        &explicit_lines,
    )
}

/// Replace one source string with logical final lines and a narrowly permitted
/// visible end-of-line dictionary hyphen.  This is the same canonical Roadmap task
/// 20 source/token/font/writer path as [`edit_advanced_text_pdf_with_layout`];
/// it is not an overlay or a second serializer.
#[allow(clippy::too_many_arguments)]
pub fn edit_advanced_text_pdf_with_visual_layout(
    input: &[u8],
    page_number: usize,
    old_text: &str,
    new_text: &str,
    mode: AdvancedTextMode,
    options: &AdvancedTextEditOptions,
    font_bytes: Option<&[u8]>,
    final_lines: &[ExplicitLayoutLine],
) -> Result<(Vec<u8>, AdvancedTextEditReport)> {
    if final_lines.is_empty()
        || final_lines
            .iter()
            .map(|line| line.logical_text.as_str())
            .collect::<String>()
            != new_text
    {
        return Err(WellfriendError::invalid_input(
            "advanced_editing explicit logical final lines must be nonempty and concatenate exactly to replacement text",
        ));
    }
    for line in final_lines {
        let visual_base = line
            .logical_text
            .trim_end_matches(crate::fonts::hard_break::is_hard_break);
        let allowed_visual = if line.inserted_visual_hyphen {
            format!("{visual_base}-")
        } else {
            visual_base.to_string()
        };
        if line.visual_text != allowed_visual {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing visual final layout permits only trailing mandatory line separators and one end-of-line inserted dictionary hyphen"
                    .to_string(),
            ));
        }
    }
    edit_advanced_text_pdf_internal(
        input,
        page_number,
        old_text,
        new_text,
        mode,
        options,
        font_bytes,
        Some(final_lines),
        None,
    )
}

/// Replace one source token with final lines at explicit, validated line
/// rectangles. The text, shaping, Type0 subset, content-token mutation, and
/// canonical incremental writer are exactly the same advanced editing path as the
/// non-positioned variant. In vertical mode each rectangle bounds one column.
#[allow(clippy::too_many_arguments)]
pub fn edit_advanced_text_pdf_with_positioned_visual_layout(
    input: &[u8],
    page_number: usize,
    old_text: &str,
    new_text: &str,
    mode: AdvancedTextMode,
    options: &AdvancedTextEditOptions,
    font_bytes: Option<&[u8]>,
    final_lines: &[PositionedExplicitLayoutLine],
) -> Result<(Vec<u8>, AdvancedTextEditReport)> {
    let plain_lines = final_lines
        .iter()
        .map(|item| item.line.clone())
        .collect::<Vec<_>>();
    if plain_lines.is_empty()
        || plain_lines
            .iter()
            .map(|line| line.logical_text.as_str())
            .collect::<String>()
            != new_text
    {
        return Err(WellfriendError::invalid_input(
            "advanced_editing positioned final lines must be nonempty and concatenate exactly to replacement text",
        ));
    }
    for item in final_lines {
        let region = item.region;
        if region.iter().any(|value| !value.is_finite())
            || region[2] <= region[0]
            || region[3] <= region[1]
        {
            return Err(WellfriendError::invalid_input(
                "advanced_editing positioned final layout contains an invalid line rectangle",
            ));
        }
        let visual_base = item
            .line
            .logical_text
            .trim_end_matches(crate::fonts::hard_break::is_hard_break);
        let allowed_visual = if item.line.inserted_visual_hyphen {
            format!("{visual_base}-")
        } else {
            visual_base.to_string()
        };
        if item.line.visual_text != allowed_visual {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing positioned visual layout permits only trailing mandatory line separators and one end-of-line inserted dictionary hyphen"
                    .to_string(),
            ));
        }
    }
    let regions = final_lines
        .iter()
        .map(|item| item.region)
        .collect::<Vec<_>>();
    edit_advanced_text_pdf_internal(
        input,
        page_number,
        old_text,
        new_text,
        mode,
        options,
        font_bytes,
        Some(&plain_lines),
        Some(&regions),
    )
}

#[allow(clippy::too_many_arguments)]
fn edit_advanced_text_pdf_internal(
    input: &[u8],
    page_number: usize,
    old_text: &str,
    new_text: &str,
    mode: AdvancedTextMode,
    options: &AdvancedTextEditOptions,
    font_bytes: Option<&[u8]>,
    explicit_final_lines: Option<&[ExplicitLayoutLine]>,
    explicit_line_regions: Option<&[[f64; 4]]>,
) -> Result<(Vec<u8>, AdvancedTextEditReport)> {
    validate_advanced_text_options(options)?;
    if !options.paint_partitions.is_empty() {
        return Err(WellfriendError::invalid_input(
            "generated paint partitions require the page-logical multi-run edit API",
        ));
    }
    if !matches!(
        mode,
        AdvancedTextMode::ParagraphReflowHorizontal
            | AdvancedTextMode::ParagraphReflowRtl
            | AdvancedTextMode::ParagraphReflowVertical
    ) {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing serialized advanced text edit requires a paragraph reflow mode"
                .to_string(),
        ));
    }
    let font = font_bytes
        .or_else(|| get_fallback_font("Symbol"))
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing bundled shaping font unavailable".to_string(),
            )
        })?;
    let analysis_text = explicit_final_lines
        .map(|lines| {
            lines
                .iter()
                .map(|line| line.visual_text.as_str())
                .collect::<String>()
        })
        .unwrap_or_else(|| new_text.to_string());
    let analysis = analyze_advanced_text_reflow(
        &analysis_text,
        mode,
        Some(font),
        TextReflowLimits::default(),
    )?;
    if !analysis.missing_glyph_clusters.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing selected font is missing glyphs for UTF-8 clusters {:?}",
            analysis.missing_glyph_clusters
        )));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::ContentEdit)?;
    enforce_advanced_editing_signature_policy(
        &signature_policy,
        options.signature_policy_override,
        "RTL/vertical paragraph reflow",
    )?;
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut matches = Vec::new();
    let mut scanner_state = ScannedTextTokenState::default();
    let mut position_metrics = inline_text::Metrics::new(&resources, reader);
    for (stream_index, (number, generation)) in page.contents.iter().copied().enumerate() {
        let stream = reader.get_object(number, generation)?;
        let decoded_result = decode_stream_lossless_with_limits(
            &stream,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded_result.status != StreamDecodeStatus::Complete {
            // The following /Contents member may legally inherit graphics or
            // text state. Once an earlier member is opaque, that inherited
            // state is unknowable; never reuse a stale pre-stream snapshot.
            scanner_state = ScannedTextTokenState::default();
            continue;
        }
        for token in scan_text_string_tokens_with_metrics(
            &decoded_result.data,
            &mut scanner_state,
            Some((number, generation)),
            Some(&mut position_metrics),
        )? {
            let Some(font_dict) = resources.fonts.get(&token.font_name) else {
                continue;
            };
            let resolver = FontResolver::new(font_dict, reader);
            if resolver
                .try_decode_string(&token.decoded)
                .map_err(WellfriendError::UnsupportedFeature)?
                == old_text
            {
                if options
                    .target_stream_object
                    .is_some_and(|target| target != number)
                    || options
                        .target_stream_generation
                        .is_some_and(|target| target != generation)
                    || options
                        .target_decoded_byte_range
                        .is_some_and(|target| target != [token.token_start, token.token_end])
                {
                    continue;
                }
                matches.push((
                    stream_index,
                    number,
                    generation,
                    stream.clone(),
                    decoded_result.data.clone(),
                    token,
                ));
            }
        }
    }
    if matches.len() != 1 {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing bounded reflow requires old text in exactly one PDF string token; found {} occurrences",
            matches.len()
        )));
    }
    let (_stream_index, source_number, source_generation, source_object, mut source_decoded, token) =
        matches.remove(0);
    if matches!(token.text_render_mode, 4..=7) {
        return Err(WellfriendError::UnsupportedFeature(
            "clipping text requires the source-inline page-logical writer, not a relocated reflow block".into()));
    }
    if token.unresolved_actual_text
        || !token.actual_text_sources.is_empty()
        || token_has_named_actual_text(&token, &resources, reader)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing logical_actual_text_conflict: the bounded single-token writer cannot update surrounding /ActualText atomically; use the page-logical multi-run edit path"
                .to_string(),
        ));
    }
    let source_font = resources
        .fonts
        .get(&token.font_name)
        .ok_or_else(|| WellfriendError::MalformedPdf("selected source font is missing".into()))?;
    let resolver = FontResolver::new(source_font, reader);
    let (removed_start, removed_end, empty) = rewrite_source_text_destructively(
        &token,
        &resolver,
        &[],
        &token.decoded,
        &[],
        false,
        None,
    )?;
    source_decoded.splice(removed_start..removed_end, empty.clone());
    let source_compressed = flate_encode_cancellable(&source_decoded, 6)?;
    let PdfObject::Stream {
        dict: mut source_dict,
        ..
    } = source_object
    else {
        unreachable!("matched object was decoded as a stream")
    };
    source_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    source_dict.remove("DecodeParms");
    source_dict.insert("Length", PdfObject::Integer(source_compressed.len() as i64));

    let layout = match explicit_final_lines {
        Some(lines) => {
            layout_generated_explicit_lines(lines, mode, font, options, explicit_line_regions)?
        }
        None => layout_generated_logical_text(new_text, mode, font, options)?,
    };
    let glyphs = layout.iter().flatten().cloned().collect::<Vec<_>>();
    let base_object = reserve_advanced_object_block(
        reader,
        8 + story_carriers::OBJECT_COUNT,
        "advanced_editing bounded reflow",
    )?;
    let font_file_number = base_object;
    let descriptor_number = base_object + 1;
    let cid_to_gid_number = base_object + 2;
    let to_unicode_number = base_object + 3;
    let descendant_number = base_object + 4;
    let type0_number = base_object + 5;
    let isolation_prefix_number = base_object + 6;
    let content_number = base_object + 7;
    let font_resource_name = deterministic_font_resource_name(reader, &page.resources);
    let mut changed = vec![IncrementalObject {
        number: source_number,
        generation: source_generation,
        object: PdfObject::Stream {
            dict: source_dict,
            raw: source_compressed,
        },
    }];
    changed.extend(build_type0_font_objects(
        font,
        &glyphs,
        mode == AdvancedTextMode::ParagraphReflowVertical,
        font_file_number,
        descriptor_number,
        cid_to_gid_number,
        to_unicode_number,
        descendant_number,
        type0_number,
    )?);
    let (generated_content, line_adjustments) = serialize_generated_text(
        &layout,
        &font_resource_name,
        options,
        mode == AdvancedTextMode::ParagraphReflowVertical,
        explicit_line_regions,
        Some(new_text),
    )?;
    let (generated_content, logical_fonts) = story_carriers::attach_generated(
        generated_content,
        new_text,
        mode == AdvancedTextMode::ParagraphReflowVertical,
        options,
        base_object + 8,
        &mut changed,
        true,
    )?;
    let generated_content = isolated_appended_content(generated_content);
    let generated_compressed = flate_encode_cancellable(generated_content.as_bytes(), 6)?;
    let mut generated_dict = crate::PdfDictionary::empty();
    generated_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    generated_dict.insert(
        "Length",
        PdfObject::Integer(generated_compressed.len() as i64),
    );
    changed.push(IncrementalObject {
        number: content_number,
        generation: 0,
        object: PdfObject::Stream {
            dict: generated_dict,
            raw: generated_compressed,
        },
    });
    changed.push(page_graphics_state_isolation_prefix(
        isolation_prefix_number,
    ));
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "advanced_editing page object is not a dictionary".to_string(),
        )
    })?;
    let mut resource_dict = page.resources.clone();
    let mut font_resources = match resource_dict.get("Font") {
        Some(PdfObject::Dictionary(dict)) => dict.clone(),
        Some(reference @ PdfObject::Reference { .. }) => reader
            .resolve(reference.clone())?
            .as_dict()
            .cloned()
            .unwrap_or_else(crate::PdfDictionary::empty),
        _ => crate::PdfDictionary::empty(),
    };
    font_resources.insert(
        font_resource_name.clone(),
        PdfObject::Reference {
            number: type0_number,
            generation: 0,
        },
    );
    story_carriers::install(&logical_fonts, &mut font_resources)?;
    resource_dict.insert("Font", PdfObject::Dictionary(font_resources));
    page_dict.insert("Resources", PdfObject::Dictionary(resource_dict));
    anchor_generated_reflow(
        reader,
        &page.contents,
        &mut changed,
        (source_number, source_generation, token.token_start),
        content_number,
        isolation_prefix_number,
    )?;
    let contents = original_page_contents(&page.contents);
    page_dict.insert("Contents", PdfObject::Array(contents));
    changed.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(page_dict),
    });
    let output = text_resources::write(reader, &page, &resources, changed)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let extracted = reopened.get_page_text(page_number)?;
    let replacement_extracts = extracted.contains(new_text)
        || explicit_final_lines.is_some_and(|_| layout_extraction_equivalent(&extracted, new_text));
    if !logical_fonts.is_empty() {
        story_carriers::verify_generated(
            &reopened,
            &reopened.document().get_page(page_number)?,
            &logical_fonts,
            new_text,
            mode == AdvancedTextMode::ParagraphReflowVertical,
            true,
        )?;
    }
    let selected_source_removed = reopened
        .document()
        .reader()
        .get_object(source_number, source_generation)
        .ok()
        .and_then(|object| {
            decode_stream_lossless_with_limits(
                &object,
                reopened.document().reader(),
                &DecodeLimits {
                    max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                    ..DecodeLimits::default()
                },
            )
            .ok()
        })
        .filter(|decoded| decoded.status == StreamDecodeStatus::Complete)
        .and_then(|decoded| {
            decoded
                .data
                .get(removed_start..removed_start.saturating_add(empty.len()))
                .map(|bytes| bytes == empty.as_slice())
        })
        .unwrap_or(false);
    // Positional-only edits (for example, document subsystems table-cell alignment)
    // intentionally preserve the logical text sequence. In that case the
    // old-token absence proof would be tautologically impossible, so retain
    // the independent source/incremental/output checks while marking the
    // textual proof satisfied by exact logical identity.
    let old_absent = if options.target_decoded_byte_range.is_some() {
        selected_source_removed
    } else {
        old_text == new_text || !extracted.contains(old_text)
    };
    if !replacement_extracts || !old_absent || !output.starts_with(input) {
        return Err(WellfriendError::MalformedPdf(format!(
            "advanced_editing RTL/vertical edit failed proof: replacement_extracts={replacement_extracts}, old_text_absent={old_absent}, prefix_preserved={}",
            output.starts_with(input)
        )));
    }
    let before_fingerprint = format!("{:x}", Sha256::digest(input));
    let after_fingerprint = format!("{:x}", Sha256::digest(&output));
    let report = AdvancedTextEditReport {
        schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
        status: AdvancedEditingSupportStatus::Implemented,
        mode,
        page: page_number,
        source_stream_object: source_number,
        source_operator: token.operator,
        old_text: old_text.to_string(),
        new_text: new_text.to_string(),
        writing_mode: i32::from(mode == AdvancedTextMode::ParagraphReflowVertical),
        font_resource: font_resource_name,
        shaped_glyphs: glyphs.len(),
        lines_or_columns: layout.len(),
        logical_to_visual_runs: analysis.bidi_runs,
        cluster_provenance: analysis.glyphs,
        removed_old_reachable_content: true,
        replacement_extracts,
        old_text_absent: old_absent,
        output_reopened: true,
        original_prefix_preserved: output.starts_with(input),
        output_bytes: output.len(),
        output_sha256: after_fingerprint.clone(),
        signature_policy,
        cryptographic_validity_claimed: false,
        deterministic: options.deterministic,
        cache_invalidation: CacheInvalidationReport {
            text_layout: true,
            glyphs: true,
            render_tiles: true,
            vectors: true,
            annotation_appearances: false,
            semantic: true,
            search_and_rag: true,
            optional_content: false,
            writer: true,
            fingerprint_before: before_fingerprint,
            fingerprint_after: after_fingerprint,
            render_write_set_refs: Vec::new(),
            changed_object_refs: Vec::new(),
            created_object_refs: Vec::new(),
            removed_object_refs: Vec::new(),
            affected_pages: vec![page_number],
            dirty_regions: Vec::new(),
            structured_render_write_set: false,
        },
        line_adjustments,
        exact_limits: vec![
            "the bounded true-edit path currently requires the old paragraph to occupy exactly one decoded PDF string token".to_string(),
            "new Unicode is embedded as a sequential-CID Type0 font; existing source codes/CIDs/GIDs are removed, not reshaped".to_string(),
            "explicit final lines retain exact Unicode glyph mapping; validation additionally accepts line-separator-insensitive extraction equivalence when a reader materializes visual line boundaries".to_string(),
            "vertical mode uses Identity-V, top-to-bottom glyph placement, right-to-left columns, upright/rotated Unicode policy, and the selected font's glyph outlines".to_string(),
            "incremental prefix preservation is structural and does not imply cryptographic signature validity".to_string(),
        ],
    };
    Ok((output, report))
}

/// Inspect a page-local sequence of PDF text-showing operands as one logical
/// range model.  The model intentionally exposes every operator boundary so a
/// caller never has to select duplicate extracted text by position alone.
pub fn analyze_multi_run_text_range(
    input: &[u8],
    page_number: usize,
) -> Result<MultiRunRangeModel> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page = engine.document().get_page(page_number)?;
    analyze_multi_run_source(&engine, &page, ScannedTextTokenState::default())
}
pub(crate) fn analyze_multi_run_source(
    engine: &ContentEngine,
    page: &crate::document::PdfPage,
    scanner_state: ScannedTextTokenState,
) -> Result<MultiRunRangeModel> {
    let page_number = page.page_number;
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut source_spans = Vec::new();
    let mut logical_text = String::new();
    let mut logical_offset = 0usize;
    // A marked-content ActualText value replaces every painted text-showing
    // operand in its scope exactly once. Track the physical owner so a scope
    // spanning several Tj/TJ operands does not duplicate its logical value.
    let mut emitted_actual_text = BTreeSet::<(u32, u16, usize, usize)>::new();
    let mut position_metrics = inline_text::Metrics::new(&resources, reader);
    for page_token in scan_page_text_string_tokens_with_metrics(
        reader,
        &page.contents,
        scanner_state,
        &mut position_metrics,
    )? {
        let PageContentStringToken {
            stream_index,
            object: number,
            generation,
            token,
        } = page_token;
        let Some(font) = resources.fonts.get(&token.font_name) else {
            continue;
        };
        let resolver = FontResolver::new(font, reader);
        let decoded_text = resolver
            .try_decode_string(&token.decoded)
            .map_err(WellfriendError::UnsupportedFeature)?;
        let source_text = decoded_text.clone();
        let text = if let Some(source) = token.actual_text_sources.last() {
            let key = (
                source.owner_object,
                source.owner_generation,
                source.value_start,
                source.value_end,
            );
            if emitted_actual_text.insert(key) {
                source.logical_text.to_string()
            } else {
                String::new()
            }
        } else {
            decoded_text
        };
        let count = text.chars().count();
        let start = logical_offset;
        logical_offset = logical_offset.saturating_add(count);
        logical_text.push_str(&text);
        source_spans.push(MultiRunSourceSpan {
            span_id: format!("p{page_number}:s{stream_index}:o{}", token.token_start),
            stream_object: number,
            stream_generation: generation,
            operator: token.operator,
            tj_element: token.element,
            byte_range: [token.token_start, token.token_end],
            logical_range: [start, logical_offset],
            source_text_object: 0,
            font_resource: token.font_name,
            font_size: token.font_size,
            character_spacing: token.character_spacing,
            word_spacing: token.word_spacing,
            horizontal_scaling: token.horizontal_scaling,
            writing_mode: i32::from(resolver.is_vertical()),
            text_render_mode: token.text_render_mode,
            marked_content_depth: token.marked_depth,
            authored_typed_table: token
                .authored_typed_owner
                .as_ref()
                .map(|owner| owner.0.clone()),
            authored_typed_cell: token
                .authored_typed_owner
                .as_ref()
                .map(|owner| owner.1.clone()),
            authored_typed_region: token.authored_typed_region,
            direct_actual_text: token
                .actual_text_sources
                .last()
                .map(|source| source.logical_text.to_string()),
            direct_actual_text_source: token.actual_text_sources.last().map(|source| {
                format!(
                    "{}:{}:{}:{}",
                    source.owner_object,
                    source.owner_generation,
                    source.value_start,
                    source.value_end
                )
            }),
            flow_relocatable: token.flow_relocatable,
            source_text,
            text,
        });
    }
    normalize_isomorphic_actual_text_spans(&mut source_spans);
    if source_spans.len() > MAX_ADVANCED_EDITING_BIDI_RUNS {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing_closeout range span count exceeds 4096".to_string(),
        ));
    }
    assign_source_paint_slots(reader, &page.contents, &mut source_spans)?;
    let analysis = analyze_advanced_text_reflow(
        &logical_text,
        AdvancedTextMode::ParagraphReflowRtl,
        None,
        TextReflowLimits::default(),
    )?;
    let writing_mode = source_spans
        .first()
        .map(|first| {
            if source_spans
                .iter()
                .all(|span| span.writing_mode == first.writing_mode)
            {
                first.writing_mode
            } else {
                -1
            }
        })
        .unwrap_or(0);
    Ok(MultiRunRangeModel {
        schema_version: "advanced_editing_closeout.multirun-form-appearance-closure.v1".to_string(),
        status: AdvancedEditingSupportStatus::ImplementedWithLimits,
        page: page_number,
        paragraph_block_id: format!("page-{page_number}-logical-text"),
        logical_text,
        source_spans,
        logical_to_visual_runs: analysis.bidi_runs,
        writing_mode,
        deterministic: true,
        exact_limits: vec![
            "logical offsets are Unicode scalar offsets mapped to decoded PDF string-token provenance; visual-quads require a unique caller-provided span target".to_string(),
            "page-owned selections may start/end inside decoded string tokens and cross /Contents streams; the apply path preserves boundary residuals and commits all touched streams atomically".to_string(),
            "each source span reports its exact zero-based BT/ET paint-slot ordinal across the ordered page content sequence; generated edits spanning several ordinals require an explicit first/last anchor decision".to_string(),
            "Form-owned text remains separately occurrence-addressed because a shared Form mutation needs an explicit clone-one or edit-all ownership policy".to_string(),
        ],
    })
}

/// `/ActualText` replaces the text-showing operands inside its marked-content
/// scope as one logical value. When the concatenated physical source strings
/// are exactly that value, retain the stronger per-operand provenance instead
/// of assigning the entire logical range to the first operand and empty ranges
/// to every later operand. This makes an exact partial edit deterministic while
/// leaving non-isomorphic ligature/alternate descriptions indivisible.
fn normalize_isomorphic_actual_text_spans(spans: &mut [MultiRunSourceSpan]) {
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for (index, span) in spans.iter().enumerate() {
        if let Some(source) = span.direct_actual_text_source.as_ref() {
            groups.entry(source.clone()).or_default().push(index);
        }
    }
    for indices in groups.into_values() {
        let Some(first) = indices.first().copied() else {
            continue;
        };
        let Some(actual_text) = spans[first].direct_actual_text.clone() else {
            continue;
        };
        let physical_text = indices
            .iter()
            .map(|index| spans[*index].source_text.as_str())
            .collect::<String>();
        if physical_text != actual_text {
            continue;
        }
        let base = indices
            .iter()
            .map(|index| spans[*index].logical_range[0])
            .min()
            .unwrap_or(0);
        let mut cursor = base;
        for index in indices {
            let count = spans[index].source_text.chars().count();
            spans[index].logical_range = [cursor, cursor.saturating_add(count)];
            spans[index].text = spans[index].source_text.clone();
            cursor = cursor.saturating_add(count);
        }
    }
}

fn isomorphic_actual_text_sources(spans: &[MultiRunSourceSpan]) -> BTreeSet<String> {
    let mut groups = BTreeMap::<String, (String, String)>::new();
    for span in spans {
        let (Some(source), Some(actual_text)) = (
            span.direct_actual_text_source.as_ref(),
            span.direct_actual_text.as_ref(),
        ) else {
            continue;
        };
        let entry = groups
            .entry(source.clone())
            .or_insert_with(|| (actual_text.clone(), String::new()));
        entry.1.push_str(&span.source_text);
    }
    groups
        .into_iter()
        .filter_map(|(source, (actual, physical))| (actual == physical).then_some(source))
        .collect()
}

fn proposed_partition_boundary_class(text: &str, scalar: usize) -> Result<String> {
    let total = text.chars().count();
    if scalar == 0 || scalar == total {
        return Ok("replacement_edge".into());
    }
    let byte = scalar_boundary_byte(text, scalar).ok_or_else(|| {
        WellfriendError::MalformedPdf("partition proposal lost a scalar boundary".into())
    })?;
    if !is_grapheme_boundary(text, byte) {
        return Err(WellfriendError::MalformedPdf(
            "partition proposal divided a grapheme".into(),
        ));
    }
    let before = text[..byte].chars().next_back();
    let after = text[byte..].chars().next();
    if before.is_some_and(crate::fonts::hard_break::is_hard_break)
        || after.is_some_and(crate::fonts::hard_break::is_hard_break)
    {
        Ok("hard_separator".into())
    } else if before.is_some_and(char::is_whitespace) || after.is_some_and(char::is_whitespace) {
        Ok("whitespace".into())
    } else {
        Ok("contextual_requires_font_validation".into())
    }
}

/// Propose, but never silently apply, a source-slot partition. Replacement
/// graphemes are apportioned by the selected source scalar coverage of each
/// paint slot using Hamilton's largest-remainder method. The proposal is bound
/// to the exact input and request; physical regions remain explicit approval.
pub fn propose_generated_paint_partitions(
    input: &[u8],
    request: &MultiRunTextRangeRequest,
) -> Result<GeneratedPaintPartitionProposal> {
    validate_advanced_text_options(&request.options)?;
    if request.logical_start >= request.logical_end || request.replacement_text.is_empty() {
        return Err(WellfriendError::invalid_input(
            "paint partition proposals require a nonempty source selection and replacement",
        ));
    }
    if !request.options.paint_partitions.is_empty()
        || request.options.paint_order_policy
            != GeneratedPaintOrderPolicy::RequireSingleSourceTextObject
        || request.final_lines.is_some()
    {
        return Err(WellfriendError::invalid_input(
            "paint partition proposal input must not already contain partitions, a block-anchor override, or request-wide final lines",
        ));
    }
    if !matches!(
        request.mode,
        AdvancedTextMode::ParagraphReflowHorizontal
            | AdvancedTextMode::ParagraphReflowRtl
            | AdvancedTextMode::ParagraphReflowVertical
    ) {
        return Err(WellfriendError::invalid_input(
            "paint partition proposal requires a paragraph reflow mode",
        ));
    }
    let model = analyze_multi_run_text_range(input, request.page)?;
    let total_scalars = model.logical_text.chars().count();
    if request.logical_end > total_scalars {
        return Err(WellfriendError::invalid_input(
            "paint partition proposal logical range is outside the page model",
        ));
    }

    #[derive(Default)]
    struct Slot {
        source_scalars: usize,
        spans: Vec<String>,
        regions: Vec<Option<[f64; 4]>>,
    }
    let mut slots = BTreeMap::<usize, Slot>::new();
    for span in &model.source_spans {
        let start = request.logical_start.max(span.logical_range[0]);
        let end = request.logical_end.min(span.logical_range[1]);
        if start >= end {
            continue;
        }
        let slot = slots.entry(span.source_text_object).or_default();
        slot.source_scalars = slot
            .source_scalars
            .checked_add(end - start)
            .ok_or_else(|| {
                WellfriendError::ResourceLimit("partition source scalar count".into())
            })?;
        slot.spans.push(span.span_id.clone());
        slot.regions.push(span.authored_typed_region);
    }
    if slots.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "paint partition proposal selection has no provenance-bearing source spans".into(),
        ));
    }
    if slots.len() > MAX_ADVANCED_EDITING_BIDI_RUNS {
        return Err(WellfriendError::ResourceLimit(
            "paint partition proposal exceeds 4096 source text objects".into(),
        ));
    }

    let grapheme_scalar_lengths = request
        .replacement_text
        .graphemes(true)
        .map(|grapheme| grapheme.chars().count())
        .collect::<Vec<_>>();
    if grapheme_scalar_lengths.is_empty()
        || grapheme_scalar_lengths.len() > MAX_ADVANCED_EDITING_PARAGRAPH_CHARS
        || request.replacement_text.chars().count() > MAX_ADVANCED_EDITING_PARAGRAPH_CHARS
    {
        return Err(WellfriendError::ResourceLimit(
            "paint partition replacement grapheme/scalar count is outside 1..=1000000".into(),
        ));
    }
    let total_source = slots.values().try_fold(0usize, |total, slot| {
        total
            .checked_add(slot.source_scalars)
            .ok_or_else(|| WellfriendError::ResourceLimit("partition source scalar total".into()))
    })?;
    if total_source == 0 {
        return Err(WellfriendError::MalformedPdf(
            "partition proposal has zero selected source coverage".into(),
        ));
    }
    let grapheme_count = grapheme_scalar_lengths.len();
    let mut allocations = slots
        .iter()
        .map(|(&source_text_object, slot)| {
            let numerator = (grapheme_count as u128) * (slot.source_scalars as u128);
            Ok((
                source_text_object,
                usize::try_from(numerator / total_source as u128).map_err(|_| {
                    WellfriendError::ResourceLimit("partition grapheme allocation".into())
                })?,
                numerator % total_source as u128,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let assigned = allocations.iter().try_fold(0usize, |total, item| {
        total
            .checked_add(item.1)
            .ok_or_else(|| WellfriendError::ResourceLimit("partition allocation total".into()))
    })?;
    let remainder_count = grapheme_count.checked_sub(assigned).ok_or_else(|| {
        WellfriendError::MalformedPdf("partition apportionment exceeded replacement".into())
    })?;
    let mut remainder_order = (0..allocations.len()).collect::<Vec<_>>();
    remainder_order.sort_by(|&left, &right| {
        allocations[right]
            .2
            .cmp(&allocations[left].2)
            .then_with(|| allocations[left].0.cmp(&allocations[right].0))
    });
    for index in remainder_order.into_iter().take(remainder_count) {
        allocations[index].1 = allocations[index].1.checked_add(1).ok_or_else(|| {
            WellfriendError::ResourceLimit("partition remainder allocation".into())
        })?;
    }

    let mut grapheme_cursor = 0usize;
    let mut scalar_cursor = 0usize;
    let mut candidates = Vec::with_capacity(allocations.len());
    for (source_text_object, count, _) in allocations {
        let start = scalar_cursor;
        let grapheme_end = grapheme_cursor
            .checked_add(count)
            .ok_or_else(|| WellfriendError::ResourceLimit("partition grapheme cursor".into()))?;
        let assigned_graphemes = grapheme_scalar_lengths
            .get(grapheme_cursor..grapheme_end)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "partition apportionment exceeded replacement graphemes".into(),
                )
            })?;
        for length in assigned_graphemes {
            scalar_cursor = scalar_cursor.checked_add(*length).ok_or_else(|| {
                WellfriendError::ResourceLimit("partition replacement scalar cursor".into())
            })?;
        }
        grapheme_cursor = grapheme_end;
        let slot = slots.get(&source_text_object).ok_or_else(|| {
            WellfriendError::MalformedPdf("partition proposal lost its source slot".into())
        })?;
        let suggested_region = slot.regions.first().copied().flatten().filter(|region| {
            region.iter().all(|value| value.is_finite())
                && region[0] < region[2]
                && region[1] < region[3]
                && slot
                    .regions
                    .iter()
                    .all(|candidate| candidate == &Some(*region))
        });
        candidates.push(GeneratedPaintPartitionCandidate {
            source_text_object,
            selected_source_scalar_count: slot.source_scalars,
            replacement_scalar_range: [start, scalar_cursor],
            selected_span_ids: slot.spans.clone(),
            suggested_region,
            start_boundary_class: proposed_partition_boundary_class(
                &request.replacement_text,
                start,
            )?,
            end_boundary_class: proposed_partition_boundary_class(
                &request.replacement_text,
                scalar_cursor,
            )?,
        });
    }
    if grapheme_cursor != grapheme_count
        || scalar_cursor != request.replacement_text.chars().count()
    {
        return Err(WellfriendError::MalformedPdf(
            "partition proposal did not cover the replacement exactly".into(),
        ));
    }

    let input_sha256 = format!("{:x}", Sha256::digest(input));
    let request_json = serde_json::to_vec(request)
        .map_err(|error| WellfriendError::invalid_input(error.to_string()))?;
    let request_sha256 = format!("{:x}", Sha256::digest(&request_json));
    let replacement_sha256 = format!("{:x}", Sha256::digest(request.replacement_text.as_bytes()));
    let proposal_payload = serde_json::to_vec(&(
        "advanced_editing.paint-partition-proposal.v1",
        &input_sha256,
        &request_sha256,
        request.page,
        [request.logical_start, request.logical_end],
        &replacement_sha256,
        &candidates,
    ))
    .map_err(|error| WellfriendError::invalid_input(error.to_string()))?;
    let proposal_id = format!("{:x}", Sha256::digest(proposal_payload));
    Ok(GeneratedPaintPartitionProposal {
        schema_version: "advanced_editing.paint-partition-proposal.v1".into(),
        status: AdvancedEditingSupportStatus::ImplementedWithLimits,
        input_sha256,
        request_sha256,
        proposal_id,
        page: request.page,
        logical_range: [request.logical_start, request.logical_end],
        replacement_sha256,
        candidates,
        deterministic: true,
        exact_limits: vec![
            "Hamilton apportionment uses selected source scalar coverage and never claims author-intent recovery".into(),
            "candidate boundaries are grapheme-safe; the apply path still requires the approved font to reproduce the unsplit OpenType result exactly".into(),
            "regions are suggested only for one identical authored typed region across the slot; every physical region and optional final layout requires explicit approval".into(),
        ],
    })
}

/// Recompute the proposal against the current bytes, bind every approved
/// region to its exact source slot, then execute the ordinary governed edit.
pub fn apply_generated_paint_partition_proposal(
    input: &[u8],
    request: &MultiRunTextRangeRequest,
    proposal: &GeneratedPaintPartitionProposal,
    approval: &GeneratedPaintPartitionApproval,
    font_bytes: Option<&[u8]>,
) -> Result<(Vec<u8>, MultiRunTextEditReport)> {
    let canonical = propose_generated_paint_partitions(input, request)?;
    if proposal.proposal_id != canonical.proposal_id
        || proposal.input_sha256 != canonical.input_sha256
        || proposal.request_sha256 != canonical.request_sha256
        || approval.proposal_id != canonical.proposal_id
    {
        return Err(WellfriendError::invalid_input(
            "paint partition proposal or approval is stale, altered, or belongs to another request",
        ));
    }
    let supplied_font_sha256 = font_bytes.map(|font| format!("{:x}", Sha256::digest(font)));
    if approval.font_sha256 != supplied_font_sha256 {
        return Err(WellfriendError::invalid_input(
            "paint partition approval font_sha256 does not match the supplied shaping font bytes",
        ));
    }
    if approval.partitions.len() != canonical.candidates.len() {
        return Err(WellfriendError::invalid_input(
            "paint partition approval must cover every proposed source slot exactly once",
        ));
    }
    let mut approved_request = request.clone();
    approved_request.options.paint_order_policy =
        GeneratedPaintOrderPolicy::RequireSingleSourceTextObject;
    approved_request.options.paint_partitions = canonical
        .candidates
        .iter()
        .zip(&approval.partitions)
        .map(|(candidate, approved)| {
            if candidate.source_text_object != approved.source_text_object {
                return Err(WellfriendError::invalid_input(
                    "paint partition approval source-slot order differs from the proposal",
                ));
            }
            Ok(GeneratedPaintPartition {
                source_text_object: candidate.source_text_object,
                replacement_scalar_range: candidate.replacement_scalar_range,
                region: approved.region,
                final_lines: approved.final_lines.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    edit_multi_run_text_range(input, &approved_request, font_bytes)
}

/// Replace or delete a selection spanning multiple Tj/TJ/quote operands.
/// Selected source codes are physically removed from the reachable content
/// streams. Equivalent numeric `TJ` displacement retains the original text
/// advance, while the replacement is emitted once as positioned, shaped Type0
/// text with `/ActualText` for logical-order extraction. A zero-width boundary
/// is anchored to one adjacent provenance-bearing operand.
pub fn edit_multi_run_text_range(
    input: &[u8],
    request: &MultiRunTextRangeRequest,
    font_bytes: Option<&[u8]>,
) -> Result<(Vec<u8>, MultiRunTextEditReport)> {
    edit_multi_run_text_range_in_scope(input, request, font_bytes, None, None, false, false)
}

/// Relocation receipts keep exact source-font names on the origin page so a
/// later backward compaction can recreate cells which are currently painted on
/// another page. Those names have no reachable `Tf` while the receipt is
/// active, so ordinary reachability-based font retirement must treat the
/// receipt as an explicit dependency.
fn authored_relocation_receipt_fonts(
    reader: &crate::PdfReader,
    page: &crate::document::PdfPage,
) -> Result<BTreeSet<String>> {
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let dictionary = page_object.as_dict().ok_or_else(|| {
        WellfriendError::MalformedPdf("authored font owner is not a page dictionary".into())
    })?;
    let Some(marker) = dictionary.get("WFTableRelocation") else {
        return Ok(BTreeSet::new());
    };
    let marker = reader
        .resolve(marker.clone())?
        .as_dict()
        .cloned()
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "authored table relocation font receipt is not a dictionary".into(),
            )
        })?;
    let cells = marker
        .get("Cells")
        .cloned()
        .map(|value| reader.resolve(value))
        .transpose()?
        .and_then(|value| value.as_array().map(|items| items.to_vec()))
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "authored table relocation font receipt has no cell array".into(),
            )
        })?;
    if cells.is_empty() || cells.len() > 4096 {
        return Err(WellfriendError::ResourceLimit(
            "authored table relocation font receipt cell count".into(),
        ));
    }
    let mut fonts = BTreeSet::new();
    for cell in cells {
        let cell = reader.resolve(cell)?;
        let font = cell
            .as_dict()
            .and_then(|dictionary| dictionary.get_name("Font"))
            .filter(|name| !name.is_empty() && name.len() <= 255)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "authored table relocation cell has no valid font receipt".into(),
                )
            })?;
        fonts.insert(font.to_string());
    }
    Ok(fonts)
}

pub(crate) fn edit_multi_run_text_range_for_authored_owner(
    input: &[u8],
    request: &MultiRunTextRangeRequest,
    font_bytes: Option<&[u8]>,
    table_identity: &str,
    cell_identity: &str,
    force_generated_style: bool,
) -> Result<(Vec<u8>, MultiRunTextEditReport)> {
    if table_identity.is_empty()
        || table_identity.len() > 16 * 1024
        || table_identity.contains('\0')
        || cell_identity.is_empty()
        || cell_identity.len() > 16 * 1024
        || cell_identity.contains('\0')
    {
        return Err(WellfriendError::invalid_input(
            "authored typed-table source owner must be bounded and nonempty",
        ));
    }
    edit_multi_run_text_range_in_scope(
        input,
        request,
        font_bytes,
        None,
        Some((table_identity, cell_identity)),
        request.final_lines.is_some(),
        force_generated_style,
    )
}

pub(crate) fn edit_multi_run_text_range_in_scope(
    input: &[u8],
    request: &MultiRunTextRangeRequest,
    font_bytes: Option<&[u8]>,
    scope: Option<&form_text::Scope>,
    required_authored_owner: Option<(&str, &str)>,
    authored_positioned_layout: bool,
    force_generated_preserved_style: bool,
) -> Result<(Vec<u8>, MultiRunTextEditReport)> {
    validate_advanced_text_options(&request.options)?;
    if !matches!(
        request.mode,
        AdvancedTextMode::ParagraphReflowHorizontal
            | AdvancedTextMode::ParagraphReflowRtl
            | AdvancedTextMode::ParagraphReflowVertical
    ) {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing_closeout range edit requires a paragraph reflow mode".to_string(),
        ));
    }
    if request.logical_start > request.logical_end {
        return Err(WellfriendError::invalid_input(
            "advanced_editing_closeout logical range start exceeds end",
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::ContentEdit)?;
    enforce_advanced_editing_signature_policy(
        &signature_policy,
        request.options.signature_policy_override,
        "multi-run text range edit",
    )?;
    let page = match scope {
        Some(scope) => scope.source_page.clone(),
        None => engine.document().get_page(request.page)?,
    };
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let initial_scanner_state = scope.map(|scope| scope.initial.clone()).unwrap_or_default();
    let normalized_model = analyze_multi_run_source(&engine, &page, initial_scanner_state.clone())?;
    let isomorphic_actual_text = isomorphic_actual_text_sources(&normalized_model.source_spans);
    let normalized_source_spans = normalized_model
        .source_spans
        .into_iter()
        .map(|span| {
            (
                (
                    span.stream_object,
                    span.stream_generation,
                    span.byte_range[0],
                    span.byte_range[1],
                ),
                span,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut selected = Vec::<SelectedMultiRunOperand>::new();
    let mut total = 0usize;
    let mut logical_text = String::new();
    let mut insertion_before: Option<SelectedMultiRunOperand> = None;
    let mut insertion_after: Option<SelectedMultiRunOperand> = None;
    let mut insertion_inside: Option<SelectedMultiRunOperand> = None;
    let mut stream_sources = BTreeMap::<(u32, u16), (Arc<PdfObject>, Arc<Vec<u8>>)>::new();
    let mut actual_text_coverages = BTreeMap::<(u32, u16, usize, usize), ActualTextCoverage>::new();
    let mut emitted_actual_text = BTreeSet::<(u32, u16, usize, usize)>::new();
    let mut authored_page_fonts = authored_relocation_receipt_fonts(reader, &page)?;
    let mut position_metrics = inline_text::Metrics::new(&resources, reader);
    for (number, generation) in page.contents.iter().copied() {
        crate::cancel::check_current_cancel("advanced multi-run stream scan")?;
        // One immutable source object and decoded buffer are shared by every
        // selected operand in this stream. Per-operand deep clones made a
        // dense 1 MiB text stream scale toward gigabytes of temporary memory.
        let object = Arc::new(reader.get_object(number, generation)?.clone());
        let decoded_result = decode_stream_lossless_with_limits(
            object.as_ref(),
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded_result.status != StreamDecodeStatus::Complete {
            continue;
        }
        let decoded = Arc::new(decoded_result.data);
        stream_sources.insert(
            (number, generation),
            (Arc::clone(&object), Arc::clone(&decoded)),
        );
    }
    for page_token in scan_page_text_string_tokens_with_metrics(
        reader,
        &page.contents,
        initial_scanner_state,
        &mut position_metrics,
    )? {
        let PageContentStringToken {
            stream_index,
            object: number,
            generation,
            token,
        } = page_token;
        let (object, decoded) = stream_sources
            .get(&(number, generation))
            .cloned()
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "advanced multi-run source stream disappeared after page tokenization"
                        .to_string(),
                )
            })?;
        if required_authored_owner.is_some_and(|(table, _)| {
            token
                .authored_typed_owner
                .as_ref()
                .is_some_and(|owner| owner.0 == table)
        }) {
            authored_page_fonts.insert(token.font_name.clone());
        }
        let Some(font) = resources.fonts.get(&token.font_name) else {
            continue;
        };
        let resolver = FontResolver::new(font, reader);
        let decoded_text = resolver
            .try_decode_string(&token.decoded)
            .map_err(WellfriendError::UnsupportedFeature)?;
        let source_text = decoded_text.clone();
        let text = if let Some(source) = token.actual_text_sources.last() {
            let key = (
                source.owner_object,
                source.owner_generation,
                source.value_start,
                source.value_end,
            );
            if emitted_actual_text.insert(key) {
                source.logical_text.to_string()
            } else {
                String::new()
            }
        } else {
            decoded_text
        };
        let start = total;
        let end = start.saturating_add(text.chars().count());
        total = end;
        logical_text.push_str(&text);
        for source in &token.actual_text_sources {
            let key = (
                source.owner_object,
                source.owner_generation,
                source.value_start,
                source.value_end,
            );
            actual_text_coverages
                .entry(key)
                .and_modify(|coverage| {
                    coverage.logical_start = coverage.logical_start.min(start);
                    coverage.logical_end = coverage.logical_end.max(end);
                })
                .or_insert_with(|| ActualTextCoverage {
                    source: source.clone(),
                    logical_start: start,
                    logical_end: end,
                });
        }
        let mut span = MultiRunSourceSpan {
            span_id: format!("p{}:s{stream_index}:o{}", request.page, token.token_start),
            stream_object: number,
            stream_generation: generation,
            operator: token.operator.clone(),
            tj_element: token.element,
            byte_range: [token.token_start, token.token_end],
            logical_range: [start, end],
            source_text_object: 0,
            font_resource: token.font_name.clone(),
            font_size: token.font_size,
            character_spacing: token.character_spacing,
            word_spacing: token.word_spacing,
            horizontal_scaling: token.horizontal_scaling,
            writing_mode: i32::from(resolver.is_vertical()),
            text_render_mode: token.text_render_mode,
            marked_content_depth: token.marked_depth,
            authored_typed_table: token
                .authored_typed_owner
                .as_ref()
                .map(|owner| owner.0.clone()),
            authored_typed_cell: token
                .authored_typed_owner
                .as_ref()
                .map(|owner| owner.1.clone()),
            authored_typed_region: token.authored_typed_region,
            direct_actual_text: token
                .actual_text_sources
                .last()
                .map(|source| source.logical_text.to_string()),
            direct_actual_text_source: token.actual_text_sources.last().map(|source| {
                format!(
                    "{}:{}:{}:{}",
                    source.owner_object,
                    source.owner_generation,
                    source.value_start,
                    source.value_end
                )
            }),
            flow_relocatable: token.flow_relocatable,
            source_text,
            text: text.clone(),
        };
        if let Some(normalized) = normalized_source_spans.get(&(
            number,
            generation,
            span.byte_range[0],
            span.byte_range[1],
        )) {
            span.logical_range = normalized.logical_range;
            span.text = normalized.text.clone();
        }
        let [span_start, span_end] = span.logical_range;
        let owner_matches = required_authored_owner.is_none_or(|(table, cell)| {
            token
                .authored_typed_owner
                .as_ref()
                .is_some_and(|owner| owner.0 == table && owner.1 == cell)
        });
        let complete_actual_text_owner = token
            .actual_text_sources
            .last()
            .and_then(|source| {
                actual_text_coverages.get(&(
                    source.owner_object,
                    source.owner_generation,
                    source.value_start,
                    source.value_end,
                ))
            })
            .is_some_and(|coverage| {
                request.logical_start <= coverage.logical_start
                    && request.logical_end >= coverage.logical_end
            });
        if owner_matches && request.logical_start == request.logical_end {
            let candidate = || {
                (
                    number,
                    generation,
                    Arc::clone(&object),
                    Arc::clone(&decoded),
                    token.clone(),
                    span.clone(),
                )
            };
            if span_start == span_end && request.logical_start == span_start {
                // Authored empty cells deliberately retain an empty
                // text-showing operand as a zero-width provenance carrier.
                // It is a valid insertion anchor even though it contributes
                // no scalar to the page-logical text.
                insertion_inside.get_or_insert_with(candidate);
            } else if span_start < span_end && request.logical_start == span_end {
                insertion_before.get_or_insert_with(candidate);
            } else if span_start < span_end && request.logical_start == span_start {
                insertion_after.get_or_insert_with(candidate);
            } else if span_start < request.logical_start && request.logical_start < span_end {
                insertion_inside.get_or_insert_with(candidate);
            }
        }
        if owner_matches
            && request.logical_start < request.logical_end
            && ((span_start < request.logical_end && span_end > request.logical_start)
                || complete_actual_text_owner)
        {
            if selected.len() >= MAX_ADVANCED_EDITING_BIDI_RUNS {
                return Err(WellfriendError::ResourceLimit(format!(
                        "advanced_editing_closeout selected source span count exceeds {MAX_ADVANCED_EDITING_BIDI_RUNS}"
                    )));
            }
            selected.push((number, generation, object, decoded, token, span));
        }
    }
    if request.logical_end > total {
        return Err(WellfriendError::invalid_input(format!(
            "advanced_editing_closeout logical range {}..{} is outside page logical length {total}",
            request.logical_start, request.logical_end
        )));
    }
    // At an interior boundary, leading inheritance belongs to the following
    // source run and trailing inheritance belongs to the preceding run. At a
    // document edge the only adjacent provenance-bearing run is used. Explicit
    // supplied and preserve-per-segment retain the historical preceding anchor.
    let mut candidate_insertion = if request.logical_start == request.logical_end {
        if let Some(inside) = insertion_inside {
            Some(inside)
        } else {
            match request.style_policy {
                MultiRunStylePolicy::InheritLeading => insertion_after.or(insertion_before),
                MultiRunStylePolicy::InheritTrailing => insertion_before.or(insertion_after),
                _ => insertion_before.or(insertion_after),
            }
        }
    } else {
        None
    };
    if request.logical_start == request.logical_end {
        let insertion_byte = scalar_boundary_byte(&logical_text, request.logical_start)
            .ok_or_else(|| WellfriendError::invalid_input("invalid insertion scalar boundary"))?;
        if !is_grapheme_boundary(&logical_text, insertion_byte) {
            return Err(WellfriendError::invalid_input(
                "zero-width insertion must target a grapheme boundary",
            ));
        }
    }
    if request.logical_start < request.logical_end {
        selected.sort_by_key(|item| item.5.logical_range[0]);
        let _first = selected.first().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing_closeout range has no provenance-bearing source spans"
                    .to_string(),
            )
        })?;
        let mut covered_until = request.logical_start;
        for item in &selected {
            if item.5.logical_range[0] == item.5.logical_range[1] {
                // A non-isomorphic ActualText owner may cover several physical
                // Tj/TJ operands while contributing its logical value once.
                // Those later operands are still selected for source removal,
                // but do not advance logical coverage.
                continue;
            }
            let overlap_start = item.5.logical_range[0].max(request.logical_start);
            let overlap_end = item.5.logical_range[1].min(request.logical_end);
            if overlap_start > covered_until || overlap_end <= overlap_start {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing_closeout logical selection contains a provenance gap that cannot be rewritten atomically"
                        .to_string(),
                ));
            }
            covered_until = covered_until.max(overlap_end);
        }
        if covered_until != request.logical_end {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing_closeout logical selection ends outside provenance-bearing text"
                    .to_string(),
            ));
        }
    }
    if selected.is_empty() {
        candidate_insertion.as_ref().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing_closeout insertion must target a provenance-bearing token boundary".to_string(),
            )
        })?;
    }
    let paint_sources = selected
        .iter()
        .chain(candidate_insertion.as_ref())
        .map(|item| (item.0, item.1, item.4.token_start))
        .collect::<Vec<_>>();
    let paint_slots = locate_source_paint_slots(reader, &page.contents, &paint_sources)?
        .into_iter()
        .map(|(slot, source)| (source, slot))
        .collect::<BTreeMap<_, _>>();
    for item in &mut selected {
        item.5.source_text_object = *paint_slots
            .get(&(item.0, item.1, item.4.token_start))
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "selected source span lost its text-object paint slot".into(),
                )
            })?;
    }
    if let Some(item) = candidate_insertion.as_mut() {
        item.5.source_text_object = *paint_slots
            .get(&(item.0, item.1, item.4.token_start))
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "insertion source span lost its text-object paint slot".into(),
                )
            })?;
    }
    let mut named_properties = BTreeSet::<String>::new();
    for item in selected.iter() {
        named_properties.extend(item.4.named_marked_properties.iter().cloned());
    }
    if let Some(anchor) = candidate_insertion.as_ref() {
        named_properties.extend(anchor.4.named_marked_properties.iter().cloned());
    }
    for name in named_properties {
        if resolve_advanced_editing_dict(resources.properties.get(&name), reader)
            .is_some_and(|dictionary| dictionary.contains_key("ActualText"))
        {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "advanced_editing logical_actual_text_conflict: named marked-content property /{name} owns ActualText; mutate that shared property through an explicit object-graph decision"
            )));
        }
    }
    if selected.iter().any(|item| item.4.unresolved_actual_text)
        || candidate_insertion
            .as_ref()
            .is_some_and(|item| item.4.unresolved_actual_text)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing logical_actual_text_conflict: an inline /ActualText value is indirect, malformed, or otherwise not a directly rewritable string"
                .to_string(),
        ));
    }
    let old_selected = logical_text
        .chars()
        .skip(request.logical_start)
        .take(request.logical_end.saturating_sub(request.logical_start))
        .collect::<String>();
    let source_has_clipping = !request.replacement_text.is_empty()
        && selected
            .iter()
            .any(|item| matches!(item.4.text_render_mode, 4..=7));
    // Tagged replacement stays inside each original marked-content scope. This
    // is stronger than moving one MCID wrapper to an appended content stream:
    // partial operands, nested BDC/BMC scopes, and multiple owner streams keep
    // their existing ParentTree identity and source ordering without cloning
    // or inventing structure elements.
    let source_has_marked_content =
        !request.replacement_text.is_empty() && selected.iter().any(|item| item.4.marked_depth > 0);
    let source_requires_inline_replacement = source_has_clipping || source_has_marked_content;
    if !request.options.paint_partitions.is_empty() {
        if request.replacement_text.is_empty() {
            return Err(WellfriendError::invalid_input(
                "deletion-only edits do not accept generated paint partitions",
            ));
        }
        if selected.is_empty() {
            return Err(WellfriendError::invalid_input(
                "generated paint partitions require a nonempty source selection",
            ));
        }
        if source_requires_inline_replacement {
            return Err(WellfriendError::UnsupportedFeature(
                "generated paint partitions cannot move clipping or marked-content-owned text outside its original source scope"
                    .into(),
            ));
        }
        if request.final_lines.is_some() {
            return Err(WellfriendError::invalid_input(
                "use each generated paint partition's final_lines instead of the request-wide final_lines",
            ));
        }
    }
    let mut actual_text_cleanup_patches = Vec::<ActualTextCleanupPatch>::new();
    let mut actual_text_keys = BTreeSet::<(u32, u16, usize, usize)>::new();
    if request.logical_start < request.logical_end {
        for (key, coverage) in &actual_text_coverages {
            if coverage.logical_start < request.logical_end
                && coverage.logical_end > request.logical_start
            {
                actual_text_keys.insert(*key);
            }
        }
    } else if let Some(anchor) = candidate_insertion.as_ref() {
        actual_text_keys.extend(anchor.4.actual_text_sources.iter().map(|source| {
            (
                source.owner_object,
                source.owner_generation,
                source.value_start,
                source.value_end,
            )
        }));
    }
    for key in &actual_text_keys {
        let coverage = actual_text_coverages.get(key).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing active ActualText source has no logical coverage".to_string(),
            )
        })?;
        let replaces_complete_actual_text_scope = request.logical_start <= coverage.logical_start
            && request.logical_end >= coverage.logical_end;
        let source_identity = format!(
            "{}:{}:{}:{}",
            coverage.source.owner_object,
            coverage.source.owner_generation,
            coverage.source.value_start,
            coverage.source.value_end
        );
        if !isomorphic_actual_text.contains(&source_identity)
            && !replaces_complete_actual_text_scope
        {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing logical_actual_text_conflict: partial editing of non-isomorphic marked-content ActualText has no unique glyph mapping; select the complete ActualText-owned source range or use an explicit semantic object-graph rewrite"
                    .to_string(),
            ));
        }
        let owner = (
            coverage.source.owner_object,
            coverage.source.owner_generation,
        );
        stream_sources.get(&owner).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing ActualText owner stream is unavailable".to_string(),
            )
        })?;
        actual_text_cleanup_patches.push(ActualTextCleanupPatch {
            owner,
            value_start: coverage.source.value_start,
            value_end: coverage.source.value_end,
            logical_text: Arc::clone(&coverage.source.logical_text),
            logical_range: [coverage.logical_start, coverage.logical_end],
        });
    }
    if authored_positioned_layout && !request.replacement_text.is_empty() {
        let authored_selection = if selected.is_empty() {
            std::slice::from_ref(candidate_insertion.as_ref().ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "authored positioned insertion lost its empty owner carrier".into(),
                )
            })?)
        } else {
            selected.as_slice()
        };
        return edit_authored_owner_positioned_fragment(
            input,
            request,
            font_bytes,
            &page,
            &resources,
            scope,
            authored_selection,
            &stream_sources,
            &actual_text_cleanup_patches,
            &old_selected,
            signature_policy,
            force_generated_preserved_style,
            &authored_page_fonts,
        );
    }
    if selected.is_empty()
        && !request.replacement_text.is_empty()
        && matches!(
            request.style_policy,
            MultiRunStylePolicy::InheritLeading | MultiRunStylePolicy::InheritTrailing
        )
    {
        return edit_zero_width_insertion_inline_source_style(
            input,
            request,
            font_bytes,
            &page,
            &resources,
            reader,
            scope,
            candidate_insertion.as_ref().ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "source-style insertion lost its provenance anchor".into(),
                )
            })?,
            &stream_sources,
            &actual_text_cleanup_patches,
            signature_policy,
        );
    }
    let mut stream_edits = DecodedStreamEdits::new();
    add_actual_text_cleanup_edits(
        &mut stream_edits,
        &stream_sources,
        &actual_text_cleanup_patches,
    )?;
    let replacement_needs_dedicated_carrier =
        story_carriers::needs_text_carrier(&request.replacement_text);
    let source_order_append_route = !selected.is_empty()
        && !source_requires_inline_replacement
        && request.options.paint_partitions.is_empty()
        && !request.replacement_text.is_empty()
        && !replacement_needs_dedicated_carrier;
    let planned_generated_font_resource = source_order_append_route
        .then(|| deterministic_font_resource_name(reader, &page.resources));
    for (selected_index, item) in selected.iter().enumerate() {
        let font = resources.fonts.get(&item.5.font_resource).ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "advanced_editing source font /{} disappeared during range mutation",
                item.5.font_resource
            ))
        })?;
        let overlap_start = request.logical_start.max(item.5.logical_range[0]);
        let overlap_end = request.logical_end.min(item.5.logical_range[1]);
        let resolver = FontResolver::new(font, reader);
        let source_order_carrier_bytes = if selected_index == 0 && source_order_append_route {
            // The carrier needs one valid text-showing code, not a complete
            // source-font encoding of the replacement: /ActualText owns the
            // full logical value. Prefer a scalar from the new text so a
            // generated page font is not required merely for logical order.
            // Ambiguous reverse mappings are acceptable here because the code
            // is nonpainting and its Unicode is explicitly overridden.
            request.replacement_text.chars().find_map(|character| {
                encode_with_existing_font(&resolver, &character.to_string())
                    .ok()
                    .and_then(|(encoded, _ambiguous)| (!encoded.is_empty()).then_some(encoded))
            })
        } else {
            None
        };
        let complete_actual_text_owner = item
            .4
            .actual_text_sources
            .last()
            .and_then(|source| {
                actual_text_coverages.get(&(
                    source.owner_object,
                    source.owner_generation,
                    source.value_start,
                    source.value_end,
                ))
            })
            .is_some_and(|coverage| {
                request.logical_start <= coverage.logical_start
                    && request.logical_end >= coverage.logical_end
            });
        let (prefix, selected_bytes, suffix) = if complete_actual_text_owner {
            (Vec::new(), item.4.decoded.clone(), Vec::new())
        } else {
            split_selected_source_operand(
                &resolver,
                &item.4.decoded,
                item.5.logical_range,
                [overlap_start, overlap_end],
            )?
        };
        let (edit_start, edit_end, replacement) = rewrite_source_text_destructively(
            &item.4,
            &resolver,
            &prefix,
            &selected_bytes,
            &suffix,
            required_authored_owner.is_some()
                && request.replacement_text.is_empty()
                && selected_index == 0,
            if selected_index == 0 && source_order_append_route {
                Some(SourceOrderActualTextCarrier {
                    logical_text: request.replacement_text.as_str(),
                    glyph: if let Some(encoded) = source_order_carrier_bytes.as_deref() {
                        SourceOrderCarrierGlyph::SourceEncoded(encoded)
                    } else {
                        SourceOrderCarrierGlyph::Generated {
                            font_resource: planned_generated_font_resource
                                .as_deref()
                                .expect("append route reserves a generated font resource"),
                            cid: 1,
                        }
                    },
                })
            } else {
                None
            },
        )?;
        stream_edits
            .entry((item.0, item.1))
            .or_insert_with(|| (item.2.as_ref().clone(), item.3.as_ref().clone(), Vec::new()))
            .2
            .push((edit_start, edit_end, replacement));
    }
    if selected.is_empty() {
        let anchor = candidate_insertion.as_ref().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing_closeout insertion lost its source-order anchor".to_string(),
            )
        })?;
        let local_scalar = request
            .logical_start
            .checked_sub(anchor.5.logical_range[0])
            .filter(|offset| *offset <= anchor.5.text.chars().count())
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "insertion caret is outside its provenance anchor".into(),
                )
            })?;
        let local_byte = scalar_boundary_byte(&anchor.5.text, local_scalar).ok_or_else(|| {
            WellfriendError::MalformedPdf("invalid insertion anchor scalar boundary".into())
        })?;
        let actual_text = format!(
            "{}{}{}",
            &anchor.5.text[..local_byte],
            request.replacement_text,
            &anchor.5.text[local_byte..]
        );
        let (edit_start, edit_end, replacement) = rewrite_source_insertion_anchor_with_actual_text(
            &anchor.4,
            &[],
            &anchor.4.decoded,
            &[],
            &actual_text,
        )?;
        stream_edits
            .entry((anchor.0, anchor.1))
            .or_insert_with(|| {
                (
                    anchor.2.as_ref().clone(),
                    anchor.3.as_ref().clone(),
                    Vec::new(),
                )
            })
            .2
            .push((edit_start, edit_end, replacement));
    }
    let mut source_updates =
        materialize_decoded_stream_edits(stream_edits, "advanced_editing_closeout range source")?;
    if request.replacement_text.is_empty() {
        let output = form_text::write_scope_with_protected_fonts(
            reader,
            &page,
            &resources,
            source_updates,
            scope,
            &authored_page_fonts,
        )?;
        let reopened = ContentEngine::open_bytes(output.clone())?;
        let extracted = form_text::extract_scope(&reopened, request.page, scope)?;
        let old_absent = old_selected.is_empty() || !extracted.contains(&old_selected);
        // `old_absent` is intentionally a document-wide observation, not the
        // identity proof for this edit: another, unrelated occurrence may
        // contain the same Unicode.  The mutation target is proven by its
        // revision-bound stream/token provenance and the atomic source update.
        if !output.starts_with(input) {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing_closeout multi-run delete save/reopen/extract proof failed"
                    .to_string(),
            ));
        }
        return Ok((output.clone(), MultiRunTextEditReport { schema_version:"advanced_editing_closeout.multirun-form-appearance-closure.v1".to_string(), status:AdvancedEditingSupportStatus::ImplementedWithLimits, operation:"delete".to_string(), page:request.page, logical_range:[request.logical_start,request.logical_end], selected_source_spans:selected.into_iter().map(|item| item.5).collect(), style_policy:request.style_policy, generated_font_used:false, generated_paint_order:None, generated_paint_partitions:Vec::new(), replacement_text:request.replacement_text.clone(), replacement_extracts:true, old_selected_text_absent:old_absent, unrelated_text_preserved:true, reachable_source_tokens_removed:true, output_reopened:true, original_prefix_preserved:output.starts_with(input), output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:request.options.deterministic, cache_invalidation:advanced_editing_cache_invalidation(input,&output,true,false,false), exact_limits:vec!["selected source codes are absent from the current reachable stream revision; numeric TJ displacement preserves their horizontal or vertical text advance so following operands retain position".to_string(),"selection boundaries must align to complete source CMap mappings; deleting clipping text deliberately removes its selected clipping contribution".to_string(),"incremental output retains historical bytes in the earlier revision and is not a sanitizing redaction; use the full-rewrite redaction path when historical byte removal is required".to_string(),"logical/visual mapping uses bidi shaping provenance, never x-coordinate sorting; visual quad selection is accepted only after the caller resolves it to one unambiguous logical range".to_string()] }));
    }
    if (request.style_policy != MultiRunStylePolicy::ExplicitSupplied && !selected.is_empty())
        || source_requires_inline_replacement
    {
        if selected.is_empty() {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing preserve_per_segment insertion has no source style owner; use an explicit supplied or inherit style policy"
                    .to_string(),
            ));
        }
        let fallback_lines;
        let logical_lines = if let Some(lines) = request.final_lines.as_deref() {
            if lines.is_empty()
                || lines
                    .iter()
                    .map(|line| line.logical_text.as_str())
                    .collect::<String>()
                    != request.replacement_text
            {
                return Err(WellfriendError::invalid_input(
                    "advanced_editing preserve_per_segment final lines must concatenate exactly to replacement text",
                ));
            }
            let mut line_scalar_cursor = 0usize;
            for line in lines {
                let visual_base = line
                    .logical_text
                    .trim_end_matches(crate::fonts::hard_break::is_hard_break);
                let expected_visual = if line.inserted_visual_hyphen {
                    format!("{visual_base}-")
                } else {
                    visual_base.to_string()
                };
                if line.visual_text != expected_visual {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing preserve_per_segment final visual line differs from its logical text beyond one declared trailing dictionary hyphen"
                            .to_string(),
                    ));
                }
                line_scalar_cursor =
                    line_scalar_cursor.saturating_add(line.logical_text.chars().count());
                let scalar_end = line_scalar_cursor;
                let end_byte = scalar_boundary_byte(&request.replacement_text, scalar_end)
                    .ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "advanced_editing preserve_per_segment cannot map final line boundary"
                                .to_string(),
                        )
                    })?;
                if !is_grapheme_boundary(&request.replacement_text, end_byte) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing preserve_per_segment refuses a final line boundary that splits a grapheme or shaping cluster"
                            .to_string(),
                    ));
                }
            }
            lines
        } else {
            // Retain mandatory boundaries without flattening CRLF or discarding
            // blank lines. This is not implicit word wrapping or a font change.
            fallback_lines = crate::fonts::hard_break::logical_lines(&request.replacement_text)
                .map(|line| {
                    let line = line?;
                    Ok(ExplicitLayoutLine {
                        logical_text: request.replacement_text[line.logical].to_owned(),
                        visual_text: request.replacement_text[line.visible].to_owned(),
                        inserted_visual_hyphen: false,
                        bidi: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            fallback_lines.as_slice()
        };
        // Preserve source styles for replacements whose length changes without
        // flattening them to a generated Type0 font.  The style owner is
        // chosen at a *grapheme* boundary by proportional source coverage:
        // source style runs retain their order and each replacement grapheme
        // is owned by exactly one complete source grapheme.  This is a
        // deterministic editing policy, not a claim that a PDF source stores
        // an author-level style intent for newly inserted characters.  It
        // deliberately keeps the serializer scalar-oriented only after the
        // grapheme-safe ownership decision has been made.
        let mut source_styles_by_grapheme = Vec::<(PreservedTextStyle, usize)>::new();
        let mut scalar_offset = 0usize;
        if story_carriers::needs_text_carrier(&old_selected) {
            // A logical carrier may encode one grapheme (notably CRLF) as
            // several zero-advance PDF operands. Operand boundaries are not
            // style boundaries. Coalesce them only when their complete text
            // state is identical; otherwise the source does not define a
            // unique grapheme-level style and must still fail closed.
            let mut owner_style = None::<PreservedTextStyle>;
            for item in &selected {
                let mut style = preserved_style_from_token(&item.4)?;
                style.vertical = item.5.writing_mode == 1;
                if owner_style.as_ref().is_some_and(|owner| owner != &style) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing logical carrier spans incompatible source styles inside one grapheme"
                            .into(),
                    ));
                }
                owner_style.get_or_insert(style);
            }
            let owner_style = owner_style.ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "advanced_editing logical carrier has no source style owner".into(),
                )
            })?;
            source_styles_by_grapheme.extend(
                old_selected
                    .graphemes(true)
                    .map(|_| (owner_style.clone(), 0)),
            );
            scalar_offset = old_selected.chars().count();
        } else {
            for (selected_index, item) in selected.iter().enumerate() {
                let token = &item.4;
                let source_span = &item.5;
                let mut style = preserved_style_from_token(token)?;
                style.vertical = source_span.writing_mode == 1;
                let selected_start = request.logical_start.max(source_span.logical_range[0]);
                let selected_end = request.logical_end.min(source_span.logical_range[1]);
                let selected_text = source_span
                    .text
                    .chars()
                    .skip(selected_start.saturating_sub(source_span.logical_range[0]))
                    .take(selected_end.saturating_sub(selected_start))
                    .collect::<String>();
                let span_scalars = selected_text.chars().count();
                let source_boundary =
                    scalar_boundary_byte(&old_selected, scalar_offset + span_scalars).ok_or_else(
                        || {
                            WellfriendError::MalformedPdf(
                        "advanced_editing preserve_per_segment cannot map source style boundary"
                            .to_string(),
                    )
                        },
                    )?;
                if !is_grapheme_boundary(&old_selected, source_boundary) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing preserve_per_segment refuses a source style boundary inside a grapheme or shaping cluster"
                            .to_string(),
                    ));
                }
                source_styles_by_grapheme.extend(
                    selected_text
                        .graphemes(true)
                        .map(|_| (style.clone(), selected_index)),
                );
                scalar_offset = scalar_offset.saturating_add(span_scalars);
            }
        }
        if scalar_offset != old_selected.chars().count() || source_styles_by_grapheme.is_empty() {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing preserve_per_segment source spans did not cover grapheme-safe source style ownership"
                    .to_string(),
            ));
        }
        let replacement_graphemes = request.replacement_text.graphemes(true).collect::<Vec<_>>();
        if replacement_graphemes.is_empty() {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing preserve_per_segment non-delete replacement unexpectedly had no graphemes"
                    .to_string(),
            ));
        }
        let mut runs_by_scalar = Vec::<PreservedStyledRun>::new();
        let mut replacement_by_selected = vec![String::new(); selected.len()];
        let mut replacement_style_spans = Vec::<PreservedStyleSpan>::new();
        let mut replacement_byte_cursor = 0usize;
        let logical_only_replacement =
            story_carriers::needs_text_carrier(&request.replacement_text);
        let mut requires_generated_style_font = force_generated_preserved_style
            || request.style_policy == MultiRunStylePolicy::ExplicitSupplied
            || !request.options.paint_partitions.is_empty()
            || request.mode != AdvancedTextMode::ParagraphReflowHorizontal
            || logical_only_replacement
            || selected.iter().any(|item| {
                item.5.writing_mode
                    != i32::from(request.mode == AdvancedTextMode::ParagraphReflowVertical)
            })
            || contains_rtl_or_bidi_controls(&request.replacement_text)
            || request.options.alignment == GeneratedTextAlignment::Justify
            || logical_lines.iter().any(|line| line.inserted_visual_hyphen);
        for (replacement_grapheme_index, grapheme) in replacement_graphemes.iter().enumerate() {
            let source_grapheme_index = match request.style_policy {
                MultiRunStylePolicy::InheritLeading => 0,
                MultiRunStylePolicy::InheritTrailing => source_styles_by_grapheme.len() - 1,
                _ => {
                    replacement_grapheme_index.saturating_mul(source_styles_by_grapheme.len())
                        / replacement_graphemes.len()
                }
            };
            let (style, mut selected_index) = source_styles_by_grapheme
                [source_grapheme_index.min(source_styles_by_grapheme.len().saturating_sub(1))]
            .clone();
            // Inheritance chooses typography, not insertion order. The full
            // replacement stays at the first selected source occurrence.
            if matches!(
                request.style_policy,
                MultiRunStylePolicy::InheritLeading | MultiRunStylePolicy::InheritTrailing
            ) {
                selected_index = 0;
            }
            replacement_by_selected[selected_index].push_str(grapheme);
            let replacement_byte_end = replacement_byte_cursor.saturating_add(grapheme.len());
            replacement_style_spans.push(PreservedStyleSpan {
                byte_start: replacement_byte_cursor,
                byte_end: replacement_byte_end,
                style: style.clone(),
            });
            replacement_byte_cursor = replacement_byte_end;
            let Some(font_dict) = resources.fonts.get(&style.font_resource) else {
                return Err(WellfriendError::MalformedPdf(
                    "advanced_editing preserve_per_segment source font resource disappeared"
                        .to_string(),
                ));
            };
            let resolver = FontResolver::new(font_dict, reader);
            for character in grapheme.chars() {
                let text = character.to_string();
                let (encoded, ambiguous) = if requires_generated_style_font
                    || crate::fonts::hard_break::is_hard_break(character)
                {
                    (Vec::new(), false)
                } else {
                    match encode_with_existing_font(&resolver, &text) {
                        Ok((encoded, false)) => (encoded, false),
                        Ok((_, true)) | Err(_) => {
                            requires_generated_style_font = true;
                            (Vec::new(), false)
                        }
                    }
                };
                debug_assert!(!ambiguous);
                let advance = preserved_run_advance(&resolver, &encoded, &text, &style)?;
                runs_by_scalar.push(PreservedStyledRun {
                    text,
                    encoded,
                    style: style.clone(),
                    advance,
                });
            }
        }
        let clipping_inline = source_has_clipping;
        let tagged_inline = source_has_marked_content;
        // Different inherited typography must not be encoded under the first
        // operand's original font by the exact-CMap inline shortcut.
        if source_requires_inline_replacement
            && selected.len() > 1
            && matches!(
                request.style_policy,
                MultiRunStylePolicy::InheritLeading | MultiRunStylePolicy::InheritTrailing
            )
        {
            requires_generated_style_font = true;
        }
        if clipping_inline || tagged_inline {
            if !logical_only_replacement
                && request
                    .replacement_text
                    .chars()
                    .any(crate::fonts::hard_break::is_hard_break)
            {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing inline semantic replacement requires one logical line inside each original BT/ET and marked-content scope"
                        .to_string(),
                ));
            }
            if requires_generated_style_font {
                let inherited_owner =
                    if request.style_policy == MultiRunStylePolicy::InheritTrailing {
                        selected.last()
                    } else {
                        selected.first()
                    };
                let inherited_font = inherited_owner
                    .and_then(|owner| resources.fonts.get(&owner.5.font_resource))
                    .and_then(|dict| crate::fonts::provider::embedded_program(reader, dict));
                let inherited_font = inherited_font.filter(|bytes| {
                    TextShaper::shape(bytes, &request.replacement_text, ShapeOptions::default())
                        .is_ok_and(|run| {
                            crate::fonts::shaper::has_missing_glyphs(
                                bytes,
                                &request.replacement_text,
                                &run,
                            )
                            .is_ok_and(|missing| !missing)
                        })
                });
                let font = if logical_only_replacement {
                    get_fallback_font("Symbol")
                } else {
                    font_bytes
                        .or(inherited_font
                            .as_deref()
                            .filter(|bytes| ttf_parser::Face::parse(bytes, 0).is_ok()))
                        .or_else(|| get_fallback_font("Symbol"))
                }
                .ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "advanced_editing inline semantic replacement shaping font unavailable"
                            .to_string(),
                    )
                })?;
                let analysis = analyze_advanced_text_reflow(
                    &request.replacement_text,
                    request.mode,
                    Some(font),
                    TextReflowLimits::default(),
                )?;
                if !analysis.missing_glyph_clusters.is_empty() {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing approved inline semantic font lacks replacement glyph coverage"
                            .to_string(),
                    ));
                }
                let face = ttf_parser::Face::parse(font, 0).map_err(|_| {
                    WellfriendError::UnsupportedFeature(
                        "advanced_editing inline semantic replacement requires a valid sfnt font"
                            .to_string(),
                    )
                })?;
                let mut next_cid = 1u32;
                let mut planned_glyphs = Vec::<Vec<GeneratedGlyph>>::with_capacity(selected.len());
                let mut all_glyphs = Vec::<GeneratedGlyph>::new();
                for text in &replacement_by_selected {
                    let mut glyphs = if logical_only_replacement {
                        story_carriers::inline_glyphs(text, &face)?
                    } else {
                        generated_glyph_plan(text, request.mode, font)?
                    };
                    if glyphs.iter().any(|glyph| glyph.gid == 0) {
                        return Err(WellfriendError::UnsupportedFeature(
                            "advanced_editing inline semantic replacement produced a missing glyph"
                                .to_string(),
                        ));
                    }
                    if !logical_only_replacement {
                        for glyph in &mut glyphs {
                            glyph.cid = u16::try_from(next_cid).map_err(|_| {
                                WellfriendError::ResourceLimit(
                                    "advanced_editing inline semantic replacement exceeds 65535 shaped glyphs"
                                        .to_string(),
                                )
                            })?;
                            next_cid += 1;
                        }
                    }
                    all_glyphs.extend(glyphs.iter().cloned());
                    planned_glyphs.push(glyphs);
                }
                let base = reserve_advanced_object_block(
                    reader,
                    6,
                    "advanced_editing inline generated semantic font",
                )?;
                let font_resource = deterministic_font_resource_name_for_selected_owners(
                    reader,
                    &page.resources,
                    &selected,
                )?;
                let mut inline_edits = DecodedStreamEdits::new();
                add_actual_text_cleanup_edits(
                    &mut inline_edits,
                    &stream_sources,
                    &actual_text_cleanup_patches,
                )?;
                for (selected_index, item) in selected.iter().enumerate() {
                    let font_dict =
                        resources.fonts.get(&item.5.font_resource).ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "advanced_editing inline semantic source font resource disappeared"
                                    .to_string(),
                            )
                        })?;
                    let resolver = FontResolver::new(font_dict, reader);
                    let overlap_start = request.logical_start.max(item.5.logical_range[0]);
                    let overlap_end = request.logical_end.min(item.5.logical_range[1]);
                    let (prefix, selected_bytes, suffix) = split_selected_source_operand(
                        &resolver,
                        &item.4.decoded,
                        item.5.logical_range,
                        [overlap_start, overlap_end],
                    )?;
                    let (edit_start, edit_end, replacement) = rewrite_source_text_inline_generated(
                        &item.4,
                        match request.style_policy {
                            MultiRunStylePolicy::InheritLeading => &selected[0].4,
                            MultiRunStylePolicy::InheritTrailing => &selected[selected.len() - 1].4,
                            _ => &item.4,
                        },
                        &resolver,
                        &prefix,
                        &selected_bytes,
                        &replacement_by_selected[selected_index],
                        &planned_glyphs[selected_index],
                        &font_resource,
                        request.mode == AdvancedTextMode::ParagraphReflowVertical,
                        match request.style_policy {
                            MultiRunStylePolicy::InheritLeading => selected[0].5.writing_mode == 1,
                            MultiRunStylePolicy::InheritTrailing => {
                                selected[selected.len() - 1].5.writing_mode == 1
                            }
                            _ => item.5.writing_mode == 1,
                        },
                        &suffix,
                    )?;
                    inline_edits
                        .entry((item.0, item.1))
                        .or_insert_with(|| {
                            (item.2.as_ref().clone(), item.3.as_ref().clone(), Vec::new())
                        })
                        .2
                        .push((edit_start, edit_end, replacement));
                }
                source_updates = materialize_decoded_stream_edits(
                    inline_edits,
                    "advanced_editing inline generated semantic source",
                )?;
                if logical_only_replacement {
                    let mut used_codes = BTreeSet::new();
                    all_glyphs.retain(|glyph| used_codes.insert(glyph.cid));
                }
                let mut changed = build_type0_font_objects(
                    font,
                    &all_glyphs,
                    request.mode == AdvancedTextMode::ParagraphReflowVertical,
                    base,
                    base + 1,
                    base + 2,
                    base + 3,
                    base + 4,
                    base + 5,
                )?;
                if logical_only_replacement {
                    let dict = changed
                        .iter_mut()
                        .find(|object| object.number == base + 5)
                        .and_then(|object| object.object.as_dict_mut())
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "inline logical carrier font missing".into(),
                            )
                        })?;
                    dict.insert("WFLogicalLineCarrier", PdfObject::Integer(1));
                }
                changed.extend(source_updates);
                install_generated_font_in_selected_form_updates(
                    reader,
                    &selected,
                    &mut changed,
                    &font_resource,
                    base + 5,
                )?;
                let page_object = reader.get_object(page.object_number, page.generation_number)?;
                let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "advanced_editing inline semantic page object is not a dictionary"
                            .to_string(),
                    )
                })?;
                let mut page_resources = page.resources.clone();
                let mut fonts = resolve_advanced_editing_dict(page_resources.get("Font"), reader)
                    .unwrap_or_else(crate::PdfDictionary::empty);
                fonts.insert(
                    font_resource,
                    PdfObject::Reference {
                        number: base + 5,
                        generation: 0,
                    },
                );
                page_resources.insert("Font", PdfObject::Dictionary(fonts));
                page_dict.insert("Resources", PdfObject::Dictionary(page_resources));
                changed.push(IncrementalObject {
                    number: page.object_number,
                    generation: page.generation_number,
                    object: PdfObject::Dictionary(page_dict),
                });
                let output = form_text::write_scope(reader, &page, &resources, changed, scope)?;
                let reopened = ContentEngine::open_bytes(output.clone())?;
                let extracted = form_text::extract_scope(&reopened, request.page, scope)?;
                let replacement_extracts = extracted.contains(&request.replacement_text)
                    || layout_extraction_equivalent(&extracted, &request.replacement_text);
                let old_absent = old_selected.is_empty() || !extracted.contains(&old_selected);
                if !replacement_extracts || !output.starts_with(input) {
                    return Err(WellfriendError::MalformedPdf(
                        "advanced_editing generated inline semantic save/reopen/extraction proof failed"
                            .to_string(),
                    ));
                }
                return Ok((output.clone(), MultiRunTextEditReport {
                    schema_version: "advanced_editing_closeout.multirun-form-appearance-closure.v1".to_string(),
                    status: AdvancedEditingSupportStatus::ImplementedWithLimits,
                    operation: if clipping_inline {
                        "replace_shaped_clipping_text_in_source".to_string()
                    } else {
                        "replace_shaped_tagged_text_in_source".to_string()
                    },
                    page: request.page,
                    logical_range: [request.logical_start, request.logical_end],
                    selected_source_spans: selected.iter().map(|item| item.5.clone()).collect(),
                    style_policy: request.style_policy,
                    generated_font_used: true,
                    generated_paint_order: None,
                    generated_paint_partitions: Vec::new(),
                    replacement_text: request.replacement_text.clone(),
                    replacement_extracts,
                    old_selected_text_absent: old_absent,
                    unrelated_text_preserved: true,
                    reachable_source_tokens_removed: true,
                    output_reopened: true,
                    original_prefix_preserved: output.starts_with(input),
                    output_sha256: format!("{:x}", Sha256::digest(&output)),
                    signature_policy,
                    cryptographic_validity_claimed: false,
                    deterministic: request.options.deterministic,
                    cache_invalidation: advanced_editing_cache_invalidation(input, &output, true, false, false),
                    exact_limits: vec![
                        "the shaped Type0 replacement is injected inside every original BT/ET and marked-content scope, so clipping and existing MCID/ParentTree ownership observe the new glyphs without tag migration".to_string(),
                        "horizontal per-glyph GPOS offsets use TJ and text rise; upright zero-offset vertical glyphs use Identity-V; a final exact writing-axis compensation retains the original source endpoint".to_string(),
                        "the source font is restored before suffix and following operators; nested or partial marked-content scopes remain structurally unchanged and logical extraction is carried by ActualText and ToUnicode".to_string(),
                    ],
                }));
            }
            let mut inline_edits = DecodedStreamEdits::new();
            add_actual_text_cleanup_edits(
                &mut inline_edits,
                &stream_sources,
                &actual_text_cleanup_patches,
            )?;
            for (selected_index, item) in selected.iter().enumerate() {
                let font_dict = resources.fonts.get(&item.5.font_resource).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "advanced_editing inline semantic source font resource disappeared"
                            .to_string(),
                    )
                })?;
                let resolver = FontResolver::new(font_dict, reader);
                let overlap_start = request.logical_start.max(item.5.logical_range[0]);
                let overlap_end = request.logical_end.min(item.5.logical_range[1]);
                let (prefix, selected_bytes, suffix) = split_selected_source_operand(
                    &resolver,
                    &item.4.decoded,
                    item.5.logical_range,
                    [overlap_start, overlap_end],
                )?;
                let (replacement_bytes, ambiguous) =
                    encode_with_existing_font(&resolver, &replacement_by_selected[selected_index])?;
                if ambiguous {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing inline semantic replacement has more than one source-font code mapping"
                            .to_string(),
                    ));
                }
                let (edit_start, edit_end, replacement) = rewrite_source_text_inline(
                    &item.4,
                    &resolver,
                    &prefix,
                    &selected_bytes,
                    &replacement_bytes,
                    &suffix,
                )?;
                inline_edits
                    .entry((item.0, item.1))
                    .or_insert_with(|| {
                        (item.2.as_ref().clone(), item.3.as_ref().clone(), Vec::new())
                    })
                    .2
                    .push((edit_start, edit_end, replacement));
            }
            source_updates = materialize_decoded_stream_edits(
                inline_edits,
                "advanced_editing inline semantic source",
            )?;
            let output = form_text::write_scope(reader, &page, &resources, source_updates, scope)?;
            let reopened = ContentEngine::open_bytes(output.clone())?;
            let extracted = form_text::extract_scope(&reopened, request.page, scope)?;
            let replacement_extracts = extracted.contains(&request.replacement_text)
                || layout_extraction_equivalent(&extracted, &request.replacement_text);
            let old_absent = old_selected.is_empty() || !extracted.contains(&old_selected);
            if !replacement_extracts || !output.starts_with(input) {
                return Err(WellfriendError::MalformedPdf(
                    "advanced_editing inline semantic replacement save/reopen/extraction proof failed"
                        .to_string(),
                ));
            }
            return Ok((output.clone(), MultiRunTextEditReport {
                schema_version: "advanced_editing_closeout.multirun-form-appearance-closure.v1".to_string(),
                status: AdvancedEditingSupportStatus::ImplementedWithLimits,
                operation: if clipping_inline {
                    "replace_clipping_text_in_source".to_string()
                } else {
                    "replace_tagged_text_in_source".to_string()
                },
                page: request.page,
                logical_range: [request.logical_start, request.logical_end],
                selected_source_spans: selected.iter().map(|item| item.5.clone()).collect(),
                style_policy: request.style_policy,
                generated_font_used: false,
                generated_paint_order: None,
                generated_paint_partitions: Vec::new(),
                replacement_text: request.replacement_text.clone(),
                replacement_extracts,
                old_selected_text_absent: old_absent,
                unrelated_text_preserved: true,
                reachable_source_tokens_removed: true,
                output_reopened: true,
                original_prefix_preserved: output.starts_with(input),
                output_sha256: format!("{:x}", Sha256::digest(&output)),
                signature_policy,
                cryptographic_validity_claimed: false,
                deterministic: request.options.deterministic,
                cache_invalidation: advanced_editing_cache_invalidation(input, &output, true, false, false),
                exact_limits: vec![
                    "replacement glyphs remain in the original BT/ET, graphics-state, and marked-content scopes, so clipping and existing MCID ownership remain attached to the edited source".to_string(),
                    "an exact trailing TJ displacement retains the source writing-axis endpoint when replacement glyph advances differ".to_string(),
                    "the exact inline route preserves each source font and text state and therefore requires each assigned replacement scalar to have one exact source CMap code; ambiguous or missing mappings automatically use the approved generated-font route".to_string(),
                ],
            }));
        }
        if requires_generated_style_font {
            let font = font_bytes
                .or_else(|| get_fallback_font("Symbol"))
                .ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "advanced_editing preserve_per_segment shaping font unavailable"
                            .to_string(),
                    )
                })?;
            let analysis = analyze_advanced_text_reflow(
                &request.replacement_text,
                request.mode,
                Some(font),
                TextReflowLimits::default(),
            )?;
            if !analysis.missing_glyph_clusters.is_empty() {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing approved style-preserving font lacks replacement glyph coverage"
                        .to_string(),
                ));
            }
            if !request.options.paint_partitions.is_empty() {
                return edit_partitioned_generated_reflow(
                    input,
                    request,
                    font,
                    &page,
                    &resources,
                    reader,
                    scope,
                    source_updates,
                    &selected,
                    &old_selected,
                    signature_policy,
                    Some(&replacement_style_spans),
                );
            }
            let mut layout = layout_generated_explicit_lines(
                logical_lines,
                request.mode,
                font,
                &request.options,
                None,
            )?;
            let mut line_byte_base = 0usize;
            for (line, glyphs) in logical_lines.iter().zip(layout.iter_mut()) {
                for glyph in glyphs {
                    glyph.logical_byte_start =
                        glyph.logical_byte_start.saturating_add(line_byte_base);
                }
                line_byte_base = line_byte_base.saturating_add(line.logical_text.len());
            }
            let glyphs = layout.iter().flatten().cloned().collect::<Vec<_>>();
            let generated_owns_logical_text =
                !selected.is_empty() && replacement_needs_dedicated_carrier;
            if !generated_owns_logical_text && glyphs.first().map(|glyph| glyph.cid) != Some(1) {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing source-order logical replacement requires at least one generated style glyph carrier"
                        .to_string(),
                ));
            }
            let base = reserve_advanced_object_block(
                reader,
                8 + story_carriers::OBJECT_COUNT,
                "advanced_editing generated per-segment styles",
            )?;
            let isolation_prefix_number = base + 6;
            let content_number = base + 7;
            let font_resource = deterministic_font_resource_name(reader, &page.resources);
            let mut changed = source_updates;
            changed.extend(build_type0_font_objects(
                font,
                &glyphs,
                request.mode == AdvancedTextMode::ParagraphReflowVertical,
                base,
                base + 1,
                base + 2,
                base + 3,
                base + 4,
                base + 5,
            )?);
            let (generated_content, _line_adjustments) = serialize_generated_preserved_styles(
                &layout,
                &replacement_style_spans,
                &font_resource,
                &request.options,
                request.mode == AdvancedTextMode::ParagraphReflowVertical,
                None,
            )?;
            let (generated_content, logical_fonts) = story_carriers::attach_generated(
                generated_content,
                &request.replacement_text,
                request.mode == AdvancedTextMode::ParagraphReflowVertical,
                &request.options,
                base + 8,
                &mut changed,
                generated_owns_logical_text,
            )?;
            let generated_content = if generated_owns_logical_text {
                generated_content
            } else {
                wrap_generated_visual_as_artifact(generated_content)
            };
            let generated_content = isolated_appended_content(generated_content);
            let generated = flate_encode_cancellable(generated_content.as_bytes(), 6)?;
            let mut generated_dict = crate::PdfDictionary::empty();
            generated_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
            generated_dict.insert("Length", PdfObject::Integer(generated.len() as i64));
            changed.push(IncrementalObject {
                number: content_number,
                generation: 0,
                object: PdfObject::Stream {
                    dict: generated_dict,
                    raw: generated,
                },
            });
            changed.push(page_graphics_state_isolation_prefix(
                isolation_prefix_number,
            ));
            let page_object = reader.get_object(page.object_number, page.generation_number)?;
            let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "advanced_editing generated per-segment page object is not a dictionary"
                        .to_string(),
                )
            })?;
            let mut page_resources = page.resources.clone();
            let mut fonts = resolve_advanced_editing_dict(page_resources.get("Font"), reader)
                .unwrap_or_else(crate::PdfDictionary::empty);
            fonts.insert(
                font_resource,
                PdfObject::Reference {
                    number: base + 5,
                    generation: 0,
                },
            );
            story_carriers::install(&logical_fonts, &mut fonts)?;
            page_resources.insert("Font", PdfObject::Dictionary(fonts));
            page_dict.insert("Resources", PdfObject::Dictionary(page_resources));
            let paint_sources = selected
                .iter()
                .map(|item| (item.0, item.1, item.4.token_start))
                .collect::<Vec<_>>();
            let generated_paint_order = anchor_generated_reflow_for_sources(
                reader,
                &page.contents,
                &mut changed,
                &paint_sources,
                request.options.paint_order_policy,
                content_number,
                isolation_prefix_number,
            )?;
            let contents = original_page_contents(&page.contents);
            page_dict.insert("Contents", PdfObject::Array(contents));
            changed.push(IncrementalObject {
                number: page.object_number,
                generation: page.generation_number,
                object: PdfObject::Dictionary(page_dict),
            });
            let output = form_text::write_scope(reader, &page, &resources, changed, scope)?;
            let reopened = ContentEngine::open_bytes(output.clone())?;
            let extracted = form_text::extract_scope(&reopened, request.page, scope)?;
            let replacement_extracts = extracted.contains(&request.replacement_text)
                || layout_extraction_equivalent(&extracted, &request.replacement_text);
            let old_absent = old_selected.is_empty() || !extracted.contains(&old_selected);
            if !logical_fonts.is_empty() {
                story_carriers::verify_generated(
                    &reopened,
                    &form_text::output_page(&reopened, request.page, scope)?,
                    &logical_fonts,
                    &request.replacement_text,
                    request.mode == AdvancedTextMode::ParagraphReflowVertical,
                    generated_owns_logical_text,
                )?;
            }
            if !replacement_extracts || !output.starts_with(input) {
                return Err(WellfriendError::MalformedPdf(
                    "advanced_editing generated per-segment save/reopen/extraction proof failed"
                        .to_string(),
                ));
            }
            return Ok((output.clone(), MultiRunTextEditReport {
                schema_version: "advanced_editing_closeout.multirun-form-appearance-closure.v1".to_string(),
                status: AdvancedEditingSupportStatus::ImplementedWithLimits,
                operation: "replace_shaped_preserving_per_segment_styles".to_string(),
                page: request.page,
                logical_range: [request.logical_start, request.logical_end],
                selected_source_spans: selected.iter().map(|item| item.5.clone()).collect(),
                style_policy: request.style_policy,
                generated_font_used: true,
                generated_paint_order: Some(generated_paint_order),
                generated_paint_partitions: Vec::new(),
                replacement_text: request.replacement_text.clone(),
                replacement_extracts,
                old_selected_text_absent: old_absent,
                unrelated_text_preserved: true,
                reachable_source_tokens_removed: true,
                output_reopened: true,
                original_prefix_preserved: output.starts_with(input),
                output_sha256: format!("{:x}", Sha256::digest(&output)),
                signature_policy,
                cryptographic_validity_claimed: false,
                deterministic: request.options.deterministic,
                cache_invalidation: advanced_editing_cache_invalidation(input, &output, true, false, false),
                exact_limits: vec![
                    "RTL and vertical replacements are shaped with the approved embedded Type0 font while font size, spacing, scaling, rise, render mode, and exact source paint commands remain assigned per replacement grapheme".to_string(),
                    "HarfBuzz glyph offsets and bidi visual order are positioned explicitly; a nonpainting source-order ActualText carrier owns the requested logical text while the relocated generated glyphs are an Artifact".to_string(),
                    "source font-family identity is substituted only for the shaped replacement because arbitrary source PDF encodings are not shaping fonts".to_string(),
                ],
            }));
        }
        let (generated_content, _line_adjustments) = serialize_preserved_styled_runs(
            &runs_by_scalar,
            logical_lines,
            &request.options,
            request.mode,
        )?;
        let append_base = reserve_advanced_object_block(
            reader,
            2 + story_carriers::OBJECT_COUNT,
            "advanced_editing preserve_per_segment append isolation",
        )?;
        let mut changed = source_updates;
        let generated_owns_logical_text =
            !selected.is_empty() && replacement_needs_dedicated_carrier;
        let (generated_content, logical_fonts) = story_carriers::attach_generated(
            generated_content,
            &request.replacement_text,
            request.mode == AdvancedTextMode::ParagraphReflowVertical,
            &request.options,
            append_base + 2,
            &mut changed,
            generated_owns_logical_text,
        )?;
        let generated_content = if generated_owns_logical_text {
            generated_content
        } else {
            wrap_generated_visual_as_artifact(generated_content)
        };
        let generated_content = isolated_appended_content(generated_content);
        let isolation_prefix_number = append_base;
        let content_number = append_base + 1;
        let generated = flate_encode_cancellable(generated_content.as_bytes(), 6)?;
        let mut generated_dict = crate::PdfDictionary::empty();
        generated_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
        generated_dict.insert("Length", PdfObject::Integer(generated.len() as i64));
        changed.push(IncrementalObject {
            number: content_number,
            generation: 0,
            object: PdfObject::Stream {
                dict: generated_dict,
                raw: generated,
            },
        });
        changed.push(page_graphics_state_isolation_prefix(
            isolation_prefix_number,
        ));
        let page_object = reader.get_object(page.object_number, page.generation_number)?;
        let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing preserve_per_segment page object is not a dictionary".to_string(),
            )
        })?;
        if !logical_fonts.is_empty() {
            let mut page_resources = page.resources.clone();
            let mut fonts = resolve_advanced_editing_dict(page_resources.get("Font"), reader)
                .unwrap_or_else(crate::PdfDictionary::empty);
            story_carriers::install(&logical_fonts, &mut fonts)?;
            page_resources.insert("Font", PdfObject::Dictionary(fonts));
            page_dict.insert("Resources", PdfObject::Dictionary(page_resources));
        }
        let paint_sources = selected
            .iter()
            .map(|item| (item.0, item.1, item.4.token_start))
            .collect::<Vec<_>>();
        let generated_paint_order = anchor_generated_reflow_for_sources(
            reader,
            &page.contents,
            &mut changed,
            &paint_sources,
            request.options.paint_order_policy,
            content_number,
            isolation_prefix_number,
        )?;
        let contents = original_page_contents(&page.contents);
        page_dict.insert("Contents", PdfObject::Array(contents));
        changed.push(IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(page_dict),
        });
        let output = form_text::write_scope(reader, &page, &resources, changed, scope)?;
        let reopened = ContentEngine::open_bytes(output.clone())?;
        let extracted = form_text::extract_scope(&reopened, request.page, scope)?;
        let replacement_extracts = extracted.contains(&request.replacement_text)
            || layout_extraction_equivalent(&extracted, &request.replacement_text);
        let old_absent = old_selected.is_empty() || !extracted.contains(&old_selected);
        // Do not reject an otherwise exact source edit merely because the same
        // old text also exists at an unrelated occurrence on the page.
        if !logical_fonts.is_empty() {
            story_carriers::verify_generated(
                &reopened,
                &form_text::output_page(&reopened, request.page, scope)?,
                &logical_fonts,
                &request.replacement_text,
                request.mode == AdvancedTextMode::ParagraphReflowVertical,
                generated_owns_logical_text,
            )?;
        }
        if !replacement_extracts || !output.starts_with(input) {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing preserve_per_segment save/reopen/extraction proof failed"
                    .to_string(),
            ));
        }
        return Ok((output.clone(), MultiRunTextEditReport {
            schema_version: "advanced_editing_closeout.multirun-form-appearance-closure.v1".to_string(),
            status: AdvancedEditingSupportStatus::ImplementedWithLimits,
            operation: "replace_preserving_per_segment_styles".to_string(),
            page: request.page,
            logical_range: [request.logical_start, request.logical_end],
            selected_source_spans: selected.iter().map(|item| item.5.clone()).collect(),
            style_policy: request.style_policy,
            generated_font_used: false,
            generated_paint_order: Some(generated_paint_order),
            generated_paint_partitions: Vec::new(),
            replacement_text: request.replacement_text.clone(),
            replacement_extracts,
            old_selected_text_absent: old_absent,
            unrelated_text_preserved: true,
            reachable_source_tokens_removed: true,
            output_reopened: true,
            original_prefix_preserved: output.starts_with(input),
            output_sha256: format!("{:x}", Sha256::digest(&output)),
            signature_policy,
            cryptographic_validity_claimed: false,
            deterministic: request.options.deterministic,
            cache_invalidation: advanced_editing_cache_invalidation(input, &output, true, false, false),
            exact_limits: vec![
                "preserve_per_segment supports exact source CMap encoding and horizontal layout; changed-length replacements assign each complete replacement grapheme to a deterministic proportional source-style owner without flattening styles or splitting a source grapheme".to_string(),
                "font resource, font size, character/word spacing, horizontal scaling, rise, render mode, and exact source Device, calibrated, ICC, spot, DeviceN, or Pattern paint commands are replayed from each source text-showing operand".to_string(),
                "selected source codes are physically absent from reachable content; a nonpainting source-order ActualText carrier and exact TJ displacement preserve logical order and following positions while relocated visual glyphs are an Artifact".to_string(),
            ],
        }));
    }
    let font = font_bytes
        .or_else(|| get_fallback_font("Symbol"))
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing_closeout bundled shaping font unavailable".to_string(),
            )
        })?;
    let analysis_text = request
        .final_lines
        .as_ref()
        .map(|lines| {
            lines
                .iter()
                .map(|line| line.visual_text.as_str())
                .collect::<String>()
        })
        .unwrap_or_else(|| request.replacement_text.clone());
    let analysis = analyze_advanced_text_reflow(
        &analysis_text,
        request.mode,
        Some(font),
        TextReflowLimits::default(),
    )?;
    if !analysis.missing_glyph_clusters.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing_closeout replacement has missing glyph clusters in selected shaping font".to_string(),
        ));
    }
    if !request.options.paint_partitions.is_empty() {
        return edit_partitioned_generated_reflow(
            input,
            request,
            font,
            &page,
            &resources,
            reader,
            scope,
            source_updates,
            &selected,
            &old_selected,
            signature_policy,
            None,
        );
    }
    let layout = layout_generated_replacement(
        &request.replacement_text,
        request.final_lines.as_deref(),
        request.mode,
        font,
        &request.options,
    )?;
    let glyphs = layout.iter().flatten().cloned().collect::<Vec<_>>();
    let generated_owns_logical_text = !selected.is_empty() && replacement_needs_dedicated_carrier;
    if !selected.is_empty()
        && !generated_owns_logical_text
        && glyphs.first().map(|glyph| glyph.cid) != Some(1)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing source-order logical replacement requires at least one generated glyph carrier"
                .to_string(),
        ));
    }
    let base = reserve_advanced_object_block(
        reader,
        8 + story_carriers::OBJECT_COUNT,
        "advanced_editing multi-run edit",
    )?;
    let isolation_prefix_number = base + 6;
    let content_number = base + 7;
    let font_resource = match planned_generated_font_resource {
        Some(font_resource) => font_resource,
        None => deterministic_font_resource_name(reader, &page.resources),
    };
    let mut changed = source_updates;
    changed.extend(build_type0_font_objects(
        font,
        &glyphs,
        request.mode == AdvancedTextMode::ParagraphReflowVertical,
        base,
        base + 1,
        base + 2,
        base + 3,
        base + 4,
        base + 5,
    )?);
    let (generated_content, _line_adjustments) = serialize_generated_text(
        &layout,
        &font_resource,
        &request.options,
        request.mode == AdvancedTextMode::ParagraphReflowVertical,
        None,
        None,
    )?;
    let (generated_content, logical_fonts) = story_carriers::attach_generated(
        generated_content,
        &request.replacement_text,
        request.mode == AdvancedTextMode::ParagraphReflowVertical,
        &request.options,
        base + 8,
        &mut changed,
        generated_owns_logical_text,
    )?;
    // Logical ownership remains at the original source position: insertion
    // uses its existing source-order anchor, while replacement injects a new
    // empty ActualText carrier into the first selected operand. The relocated
    // Type0 glyph stream is therefore always visual-only.
    let generated_content = if generated_owns_logical_text {
        generated_content
    } else {
        wrap_generated_visual_as_artifact(generated_content)
    };
    let generated_content = isolated_appended_content(generated_content);
    let generated = flate_encode_cancellable(generated_content.as_bytes(), 6)?;
    let mut generated_dict = crate::PdfDictionary::empty();
    generated_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    generated_dict.insert("Length", PdfObject::Integer(generated.len() as i64));
    changed.push(IncrementalObject {
        number: content_number,
        generation: 0,
        object: PdfObject::Stream {
            dict: generated_dict,
            raw: generated,
        },
    });
    changed.push(page_graphics_state_isolation_prefix(
        isolation_prefix_number,
    ));
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "advanced_editing_closeout page object is not a dictionary".to_string(),
        )
    })?;
    let mut page_resources = page.resources.clone();
    let mut fonts = resolve_advanced_editing_dict(page_resources.get("Font"), reader)
        .unwrap_or_else(crate::PdfDictionary::empty);
    fonts.insert(
        font_resource,
        PdfObject::Reference {
            number: base + 5,
            generation: 0,
        },
    );
    story_carriers::install(&logical_fonts, &mut fonts)?;
    page_resources.insert("Font", PdfObject::Dictionary(fonts));
    page_dict.insert("Resources", PdfObject::Dictionary(page_resources));
    let paint_sources = selected
        .iter()
        .chain(candidate_insertion.as_ref())
        .map(|item| (item.0, item.1, item.4.token_start))
        .collect::<Vec<_>>();
    let generated_paint_order = anchor_generated_reflow_for_sources(
        reader,
        &page.contents,
        &mut changed,
        &paint_sources,
        request.options.paint_order_policy,
        content_number,
        isolation_prefix_number,
    )?;
    let contents = original_page_contents(&page.contents);
    page_dict.insert("Contents", PdfObject::Array(contents));
    changed.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(page_dict),
    });
    let output = form_text::write_scope(reader, &page, &resources, changed, scope)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let extracted = form_text::extract_scope(&reopened, request.page, scope)?;
    let replacement_extracts = request.replacement_text.is_empty()
        || extracted.contains(&request.replacement_text)
        || request
            .final_lines
            .as_ref()
            .is_some_and(|_| layout_extraction_equivalent(&extracted, &request.replacement_text));
    let old_absent = old_selected.is_empty() || !extracted.contains(&old_selected);
    // Global text absence is useful diagnostics but cannot identify the
    // selected occurrence when duplicate text exists.  Occurrence identity is
    // carried by the immutable source spans above.
    if !logical_fonts.is_empty() {
        story_carriers::verify_generated(
            &reopened,
            &form_text::output_page(&reopened, request.page, scope)?,
            &logical_fonts,
            &request.replacement_text,
            request.mode == AdvancedTextMode::ParagraphReflowVertical,
            generated_owns_logical_text,
        )?;
    }
    if !replacement_extracts || !output.starts_with(input) {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing_closeout multi-run save/reopen/extract proof failed".to_string(),
        ));
    }
    let removed_selected_source = !selected.is_empty();
    Ok((output.clone(), MultiRunTextEditReport { schema_version:"advanced_editing_closeout.multirun-form-appearance-closure.v1".to_string(), status:AdvancedEditingSupportStatus::ImplementedWithLimits, operation:if request.replacement_text.is_empty(){"delete".to_string()} else if old_selected.is_empty(){"insert".to_string()} else {"replace".to_string()}, page:request.page, logical_range:[request.logical_start,request.logical_end], selected_source_spans:selected.into_iter().map(|item| item.5).collect(), style_policy:request.style_policy, generated_font_used:true, generated_paint_order:Some(generated_paint_order), generated_paint_partitions:Vec::new(), replacement_text:request.replacement_text.clone(), replacement_extracts, old_selected_text_absent:old_absent, unrelated_text_preserved:true, reachable_source_tokens_removed:removed_selected_source, output_reopened:true, original_prefix_preserved:output.starts_with(input), output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:request.options.deterministic, cache_invalidation:advanced_editing_cache_invalidation(input,&output,true,false,false), exact_limits:vec!["logical source selections may cross decoded string-token and page-content-stream boundaries; selected source codes are removed and equivalent numeric TJ displacement preserves the original writing-axis advance".to_string(),"replacement Unicode is owned by a positioned source-order ActualText carrier while the generated Type0 glyph stream is marked as an Artifact; zero-width insertion retains its existing source-order anchor because no selected source code exists to remove".to_string(),"selection boundaries must align to complete source CMap mappings; clipping-mode deletion removes the selected clipping contribution, while nonempty clipping replacement is replayed by the preserved-style route".to_string(),"incremental editing removes selected codes from the current reachable revision but is not historical-byte sanitization".to_string()] }))
}

fn scalar_range_to_bytes(
    text: &str,
    scalar_offsets: &[usize],
    range: [usize; 2],
) -> Result<std::ops::Range<usize>> {
    let start = scalar_offsets.get(range[0]).copied().ok_or_else(|| {
        WellfriendError::invalid_input("paint partition scalar start is outside replacement text")
    })?;
    let end = scalar_offsets.get(range[1]).copied().ok_or_else(|| {
        WellfriendError::invalid_input("paint partition scalar end is outside replacement text")
    })?;
    if end > text.len() || start > end {
        return Err(WellfriendError::invalid_input(
            "paint partition scalar range is reversed or outside replacement text",
        ));
    }
    Ok(start..end)
}

fn grapheme_safe_paint_partition_boundary(
    text: &str,
    scalar_offsets: &[usize],
    scalar: usize,
) -> Result<bool> {
    let total = scalar_offsets.len().saturating_sub(1);
    if scalar == 0 || scalar == total {
        return Ok(true);
    }
    let byte = scalar_offsets.get(scalar).copied().ok_or_else(|| {
        WellfriendError::invalid_input("paint partition boundary is outside replacement text")
    })?;
    Ok(is_grapheme_boundary(text, byte))
}

/// A caller-approved paint split may change where glyphs are painted, but it
/// must not change the OpenType result. Shape each hard line once as a whole,
/// then shape the exact partition pieces with the whole paragraph's resolved
/// bidi and non-emitting joining context. A boundary is accepted only when the
/// per-piece glyph IDs, clusters, advances and offsets equal the corresponding
/// slice of the unsplit line. This catches ligatures, kerning and contextual
/// substitutions which cannot safely be divided between paint slots.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PaintPartitionShapedGlyph {
    glyph_id: u16,
    cluster: u32,
    advance: u64,
    offset_x: u64,
    offset_y: u64,
    rotate_clockwise: bool,
    vertical_alternate: bool,
    cross_advance: u64,
}

fn paint_partition_shape_signature(
    font: &[u8],
    text: &str,
    bidi: &crate::fonts::shaper::LineBidi,
    mode: AdvancedTextMode,
) -> Result<Vec<PaintPartitionShapedGlyph>> {
    if mode == AdvancedTextMode::ParagraphReflowVertical {
        return Ok(
            crate::fonts::vertical::shape_resolved(font, text, bidi, &Default::default())?
                .into_iter()
                .map(|glyph| PaintPartitionShapedGlyph {
                    glyph_id: glyph.glyph.glyph_id,
                    cluster: glyph.glyph.cluster,
                    advance: glyph.glyph.advance.to_bits(),
                    offset_x: glyph.glyph.offset_x.to_bits(),
                    offset_y: glyph.glyph.offset_y.to_bits(),
                    rotate_clockwise: glyph.rotate_clockwise,
                    vertical_alternate: glyph.vertical_alternate,
                    cross_advance: glyph.cross_advance.to_bits(),
                })
                .collect(),
        );
    }
    Ok(
        TextShaper::shape_resolved(font, text, bidi, &Default::default())?
            .glyphs
            .into_iter()
            .map(|glyph| PaintPartitionShapedGlyph {
                glyph_id: glyph.glyph_id,
                cluster: glyph.cluster,
                advance: glyph.advance.to_bits(),
                offset_x: glyph.offset_x.to_bits(),
                offset_y: glyph.offset_y.to_bits(),
                rotate_clockwise: false,
                vertical_alternate: false,
                cross_advance: 0.0f64.to_bits(),
            })
            .collect(),
    )
}

fn validate_contextual_paint_partition_boundaries(
    text: &str,
    boundary_bytes: &[usize],
    mode: AdvancedTextMode,
    font: &[u8],
) -> Result<()> {
    if text.is_empty() || boundary_bytes.is_empty() {
        return Ok(());
    }
    let shape_options = ShapeOptions {
        direction: Some(if mode == AdvancedTextMode::ParagraphReflowRtl {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        }),
    };
    let paragraph = crate::fonts::shaper::ParagraphBidi::new(text, shape_options)?;
    for hard_line in crate::fonts::hard_break::logical_lines(text) {
        crate::cancel::check_current_cancel("paint partition shaping equivalence")?;
        let line = hard_line?.visible;
        let mut cuts = boundary_bytes
            .iter()
            .copied()
            .filter(|boundary| line.start < *boundary && *boundary < line.end)
            .collect::<Vec<_>>();
        cuts.sort_unstable();
        cuts.dedup();
        if cuts.is_empty() {
            continue;
        }
        let whole_bidi = paragraph.line(line.clone())?;
        let whole = paint_partition_shape_signature(font, &text[line.clone()], &whole_bidi, mode)?;
        let mut edges = Vec::with_capacity(cuts.len() + 2);
        edges.push(line.start);
        edges.extend(cuts.iter().copied());
        edges.push(line.end);
        for edge in edges.windows(2) {
            let segment = edge[0]..edge[1];
            let segment_bidi = paragraph.line(segment.clone())?;
            let shaped =
                paint_partition_shape_signature(font, &text[segment.clone()], &segment_bidi, mode)?;
            let local_start = u32::try_from(segment.start - line.start).map_err(|_| {
                WellfriendError::ResourceLimit("paint partition line offset exceeds u32".into())
            })?;
            let local_end = u32::try_from(segment.end - line.start).map_err(|_| {
                WellfriendError::ResourceLimit("paint partition line offset exceeds u32".into())
            })?;
            let expected = whole
                .iter()
                .filter(|glyph| local_start <= glyph.cluster && glyph.cluster < local_end)
                .cloned()
                .map(|mut glyph| {
                    glyph.cluster -= local_start;
                    glyph
                })
                .collect::<Vec<_>>();
            if shaped != expected {
                let boundary = if segment.start == line.start {
                    segment.end
                } else {
                    segment.start
                };
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "generated paint partition at UTF-8 byte {boundary} divides an OpenType ligature, kerning pair, or contextual substitution"
                )));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn edit_partitioned_generated_reflow(
    input: &[u8],
    request: &MultiRunTextRangeRequest,
    font: &[u8],
    page: &crate::document::PdfPage,
    resources: &PageResources,
    reader: &crate::PdfReader,
    scope: Option<&form_text::Scope>,
    source_updates: Vec<IncrementalObject>,
    selected: &[SelectedMultiRunOperand],
    old_selected: &str,
    signature_policy: EditPolicyReport,
    preserved_styles: Option<&[PreservedStyleSpan]>,
) -> Result<(Vec<u8>, MultiRunTextEditReport)> {
    #[derive(Debug)]
    struct PreparedPartition {
        source_text_object: usize,
        replacement_scalar_range: [usize; 2],
        region: [f64; 4],
        text: String,
        options: AdvancedTextEditOptions,
        layout: Vec<Vec<GeneratedGlyph>>,
        style_spans: Vec<PreservedStyleSpan>,
        anchor: (u32, u16, usize),
    }

    let mut source_anchors = BTreeMap::<usize, (u32, u16, usize)>::new();
    for item in selected {
        source_anchors.entry(item.5.source_text_object).or_insert((
            item.0,
            item.1,
            item.4.token_start,
        ));
    }
    if source_anchors.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "partitioned generated reflow lost its source text objects".into(),
        ));
    }
    if request.options.paint_partitions.len() != source_anchors.len() {
        return Err(WellfriendError::invalid_input(format!(
            "generated paint partitions must cover every selected source text object exactly once: expected {}, received {}",
            source_anchors.len(),
            request.options.paint_partitions.len()
        )));
    }

    let mut scalar_offsets = request
        .replacement_text
        .char_indices()
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    scalar_offsets.push(request.replacement_text.len());
    let replacement_scalars = scalar_offsets.len().saturating_sub(1);
    let mut partition_byte_ranges = Vec::with_capacity(request.options.paint_partitions.len());
    let mut contextual_boundaries = Vec::new();
    let mut next_scalar = 0usize;
    let mut previous_slot = None;
    for partition in &request.options.paint_partitions {
        crate::cancel::check_current_cancel("generated paint partition validation")?;
        if previous_slot.is_some_and(|slot| slot >= partition.source_text_object) {
            return Err(WellfriendError::invalid_input(
                "generated paint partitions must be in strictly increasing source-text-object order",
            ));
        }
        if !source_anchors.contains_key(&partition.source_text_object) {
            return Err(WellfriendError::invalid_input(format!(
                "generated paint partition references unselected source text object {}",
                partition.source_text_object
            )));
        }
        let [start, end] = partition.replacement_scalar_range;
        if start != next_scalar || end < start || end > replacement_scalars {
            return Err(WellfriendError::invalid_input(
                "generated paint partition scalar ranges must be contiguous, ordered, and inside the complete replacement",
            ));
        }
        if !grapheme_safe_paint_partition_boundary(
            &request.replacement_text,
            &scalar_offsets,
            start,
        )? || !grapheme_safe_paint_partition_boundary(
            &request.replacement_text,
            &scalar_offsets,
            end,
        )? {
            return Err(WellfriendError::UnsupportedFeature(
                "generated paint partition boundary divides a grapheme cluster".into(),
            ));
        }
        let byte_range =
            scalar_range_to_bytes(&request.replacement_text, &scalar_offsets, [start, end])?;
        if end < replacement_scalars {
            contextual_boundaries.push(byte_range.end);
        }
        partition_byte_ranges.push(byte_range);
        next_scalar = end;
        previous_slot = Some(partition.source_text_object);
    }
    if next_scalar != replacement_scalars {
        return Err(WellfriendError::invalid_input(
            "generated paint partitions do not cover the complete replacement text",
        ));
    }
    validate_contextual_paint_partition_boundaries(
        &request.replacement_text,
        &contextual_boundaries,
        request.mode,
        font,
    )?;

    let mut prepared = Vec::with_capacity(request.options.paint_partitions.len());
    for (partition, byte_range) in request
        .options
        .paint_partitions
        .iter()
        .zip(partition_byte_ranges)
    {
        crate::cancel::check_current_cancel("generated paint partition preparation")?;
        let [start, end] = partition.replacement_scalar_range;
        let anchor = source_anchors
            .get(&partition.source_text_object)
            .copied()
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "validated generated paint partition lost its source anchor".into(),
                )
            })?;
        let text = request.replacement_text[byte_range.clone()].to_string();
        if text.is_empty()
            && partition
                .final_lines
                .as_ref()
                .is_some_and(|lines| !lines.is_empty())
        {
            return Err(WellfriendError::invalid_input(
                "an empty generated paint partition cannot contain final lines",
            ));
        }
        let mut options = request.options.clone();
        options.region = partition.region;
        options.paint_partitions.clear();
        validate_advanced_text_options(&options)?;
        let style_spans = preserved_styles
            .map(|spans| {
                spans
                    .iter()
                    .filter_map(|span| {
                        let overlap_start = span.byte_start.max(byte_range.start);
                        let overlap_end = span.byte_end.min(byte_range.end);
                        (overlap_start < overlap_end).then(|| PreservedStyleSpan {
                            byte_start: overlap_start - byte_range.start,
                            byte_end: overlap_end - byte_range.start,
                            style: span.style.clone(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if preserved_styles.is_some()
            && !text.is_empty()
            && (style_spans.first().is_none_or(|span| span.byte_start != 0)
                || style_spans
                    .windows(2)
                    .any(|pair| pair[0].byte_end != pair[1].byte_start)
                || style_spans
                    .last()
                    .is_none_or(|span| span.byte_end != text.len()))
        {
            return Err(WellfriendError::MalformedPdf(
                "partitioned generated style spans do not cover their replacement segment".into(),
            ));
        }
        let layout = if text.is_empty() {
            Vec::new()
        } else {
            let analysis_text = partition
                .final_lines
                .as_ref()
                .map(|lines| {
                    lines
                        .iter()
                        .map(|line| line.visual_text.as_str())
                        .collect::<String>()
                })
                .unwrap_or_else(|| text.clone());
            let analysis = analyze_advanced_text_reflow(
                &analysis_text,
                request.mode,
                Some(font),
                TextReflowLimits::default(),
            )?;
            if !analysis.missing_glyph_clusters.is_empty() {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "generated paint partition {} has missing glyph clusters {:?}",
                    partition.source_text_object, analysis.missing_glyph_clusters
                )));
            }
            layout_generated_partition_with_context(
                &request.replacement_text,
                byte_range,
                partition.final_lines.as_deref(),
                request.mode,
                font,
                &options,
            )?
        };
        prepared.push(PreparedPartition {
            source_text_object: partition.source_text_object,
            replacement_scalar_range: [start, end],
            region: partition.region,
            text,
            options,
            layout,
            style_spans,
            anchor,
        });
    }

    // Every partition is shaped independently, so its local layout starts CID
    // assignment at one. The shared embedded Type0 resource needs one global
    // collision-free code space across every emitted partition.
    let mut next_cid = 1u32;
    for partition in &mut prepared {
        for glyph in partition.layout.iter_mut().flatten() {
            glyph.cid = u16::try_from(next_cid).map_err(|_| {
                WellfriendError::ResourceLimit(
                    "partitioned generated reflow exceeds 65535 shaped glyphs".into(),
                )
            })?;
            next_cid = next_cid.checked_add(1).ok_or_else(|| {
                WellfriendError::ResourceLimit("partitioned generated CID counter".into())
            })?;
        }
    }
    let glyphs = prepared
        .iter()
        .flat_map(|partition| partition.layout.iter().flatten().cloned())
        .collect::<Vec<_>>();
    if glyphs.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "nonempty partitioned replacement produced no generated glyphs".into(),
        ));
    }
    let generated_count = u32::try_from(
        prepared
            .iter()
            .filter(|partition| !partition.text.is_empty())
            .count(),
    )
    .map_err(|_| WellfriendError::ResourceLimit("generated paint partition count".into()))?;
    let per_partition = 2u32
        .checked_add(story_carriers::OBJECT_COUNT)
        .ok_or_else(|| WellfriendError::ResourceLimit("paint partition object count".into()))?;
    let object_count = generated_count
        .checked_mul(per_partition)
        .and_then(|count| count.checked_add(6))
        .ok_or_else(|| WellfriendError::ResourceLimit("paint partition object block".into()))?;
    let base = reserve_advanced_object_block(
        reader,
        object_count,
        "advanced_editing partitioned generated reflow",
    )?;
    let reservation_end = base.checked_add(object_count).ok_or_else(|| {
        WellfriendError::ResourceLimit(
            "partitioned generated reflow needs a one-past object cursor".into(),
        )
    })?;
    let font_resource = deterministic_font_resource_name(reader, &page.resources);
    let mut changed = source_updates;
    changed.extend(build_type0_font_objects(
        font,
        &glyphs,
        request.mode == AdvancedTextMode::ParagraphReflowVertical,
        base,
        base + 1,
        base + 2,
        base + 3,
        base + 4,
        base + 5,
    )?);

    let mut cursor = base + 6;
    let mut logical_fonts = crate::PdfDictionary::empty();
    let mut verification = Vec::<(crate::PdfDictionary, String)>::new();
    let mut anchor_jobs = Vec::<(usize, (u32, u16, usize), u32, u32)>::new();
    let mut receipts = Vec::with_capacity(prepared.len());
    for partition in &prepared {
        let mut receipt = GeneratedPaintPartitionReceipt {
            source_text_object: partition.source_text_object,
            replacement_scalar_range: partition.replacement_scalar_range,
            region: partition.region,
            generated: false,
            anchor_stream_object: partition.anchor.0,
            anchor_stream_generation: partition.anchor.1,
            anchor_decoded_byte_offset: partition.anchor.2,
        };
        if !partition.text.is_empty() {
            let isolation_number = cursor;
            let content_number = cursor + 1;
            let carrier_base = cursor + 2;
            cursor = cursor.checked_add(per_partition).ok_or_else(|| {
                WellfriendError::ResourceLimit("paint partition object cursor".into())
            })?;
            let (generated_content, _) = if preserved_styles.is_some() {
                serialize_generated_preserved_styles(
                    &partition.layout,
                    &partition.style_spans,
                    &font_resource,
                    &partition.options,
                    request.mode == AdvancedTextMode::ParagraphReflowVertical,
                    Some(&partition.text),
                )?
            } else {
                serialize_generated_text(
                    &partition.layout,
                    &font_resource,
                    &partition.options,
                    request.mode == AdvancedTextMode::ParagraphReflowVertical,
                    None,
                    Some(&partition.text),
                )?
            };
            let (generated_content, segment_fonts) = story_carriers::attach_generated(
                generated_content,
                &partition.text,
                request.mode == AdvancedTextMode::ParagraphReflowVertical,
                &partition.options,
                carrier_base,
                &mut changed,
                true,
            )?;
            story_carriers::install(&segment_fonts, &mut logical_fonts)?;
            verification.push((segment_fonts, partition.text.clone()));
            let generated = flate_encode_cancellable(
                isolated_appended_content(generated_content).as_bytes(),
                6,
            )?;
            let mut generated_dict = crate::PdfDictionary::empty();
            generated_dict.insert("Filter", PdfObject::Name("FlateDecode".into()));
            generated_dict.insert("Length", PdfObject::Integer(generated.len() as i64));
            changed.push(IncrementalObject {
                number: content_number,
                generation: 0,
                object: PdfObject::Stream {
                    dict: generated_dict,
                    raw: generated,
                },
            });
            changed.push(page_graphics_state_isolation_prefix(isolation_number));
            anchor_jobs.push((
                partition.source_text_object,
                partition.anchor,
                content_number,
                isolation_number,
            ));
            receipt.generated = true;
        }
        receipts.push(receipt);
    }
    if cursor != reservation_end {
        return Err(WellfriendError::MalformedPdf(
            "partitioned generated reflow object reservation drifted".into(),
        ));
    }

    // Later insertions are applied first. Generated content contains its own
    // ET operators; descending source order keeps original ET ordinals stable
    // when several anchors share one physical content stream.
    anchor_jobs.sort_by_key(|job| std::cmp::Reverse(job.0));
    for (_, anchor, content_number, isolation_number) in anchor_jobs {
        anchor_generated_reflow(
            reader,
            &page.contents,
            &mut changed,
            anchor,
            content_number,
            isolation_number,
        )?;
    }

    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "partitioned generated reflow page object is not a dictionary".into(),
        )
    })?;
    let mut page_resources = page.resources.clone();
    let mut fonts = resolve_advanced_editing_dict(page_resources.get("Font"), reader)
        .unwrap_or_else(crate::PdfDictionary::empty);
    fonts.insert(
        font_resource,
        PdfObject::Reference {
            number: base + 5,
            generation: 0,
        },
    );
    story_carriers::install(&logical_fonts, &mut fonts)?;
    page_resources.insert("Font", PdfObject::Dictionary(fonts));
    page_dict.insert("Resources", PdfObject::Dictionary(page_resources));
    page_dict.insert(
        "Contents",
        PdfObject::Array(original_page_contents(&page.contents)),
    );
    changed.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(page_dict),
    });

    let output = form_text::write_scope(reader, page, resources, changed, scope)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let extracted = form_text::extract_scope(&reopened, request.page, scope)?;
    let replacement_extracts = extracted.contains(&request.replacement_text)
        || layout_extraction_equivalent(&extracted, &request.replacement_text);
    let old_absent = old_selected.is_empty() || !extracted.contains(old_selected);
    for (fonts, text) in &verification {
        story_carriers::verify_generated(
            &reopened,
            &form_text::output_page(&reopened, request.page, scope)?,
            fonts,
            text,
            request.mode == AdvancedTextMode::ParagraphReflowVertical,
            true,
        )?;
    }
    if !replacement_extracts || !output.starts_with(input) {
        return Err(WellfriendError::MalformedPdf(
            "partitioned generated reflow save/reopen/extraction proof failed".into(),
        ));
    }
    Ok((
        output.clone(),
        MultiRunTextEditReport {
            schema_version:
                "advanced_editing_closeout.partitioned-generated-paint-order.v1".into(),
            status: AdvancedEditingSupportStatus::ImplementedWithLimits,
            operation: if preserved_styles.is_some() {
                "replace_partitioned_preserving_source_styles".into()
            } else {
                "replace_partitioned_by_source_paint_order".into()
            },
            page: request.page,
            logical_range: [request.logical_start, request.logical_end],
            selected_source_spans: selected.iter().map(|item| item.5.clone()).collect(),
            style_policy: request.style_policy,
            generated_font_used: true,
            generated_paint_order: None,
            generated_paint_partitions: receipts,
            replacement_text: request.replacement_text.clone(),
            replacement_extracts,
            old_selected_text_absent: old_absent,
            unrelated_text_preserved: true,
            reachable_source_tokens_removed: true,
            output_reopened: true,
            original_prefix_preserved: output.starts_with(input),
            output_sha256: format!("{:x}", Sha256::digest(&output)),
            signature_policy,
            cryptographic_validity_claimed: false,
            deterministic: request.options.deterministic,
            cache_invalidation: advanced_editing_cache_invalidation(
                input, &output, true, false, false,
            ),
            exact_limits: vec![
                "each replacement segment is source-slot-bound and inserted after that exact source text object's ET; descending mutation order preserves later ET ordinals inside shared content streams".into(),
                "partition ranges cover the replacement exactly and may leave a selected source slot empty; every generated segment owns its own logical ActualText and approved region".into(),
                "partition boundaries are grapheme-safe and retain whole-paragraph bidi/joining context; the transaction refuses any split whose independently emitted glyph IDs, clusters, advances or offsets differ from the unsplit OpenType result".into(),
                "automatic and caller-supplied vertical columns retain the full paragraph context and undergo the same exact vertical shaping-equivalence validation".into(),
                "inherit-leading, inherit-trailing and preserve-per-segment policies retain grapheme-owned source size, spacing, scaling, rise, render mode and exact paint commands; the approved embedded Type0 font supplies replacement glyph outlines".into(),
                "selected current-revision source codes are removed atomically, but incremental history is not sanitizing redaction".into(),
            ],
        },
    ))
}

#[allow(clippy::too_many_arguments)]
fn edit_zero_width_insertion_inline_source_style(
    input: &[u8],
    request: &MultiRunTextRangeRequest,
    font_bytes: Option<&[u8]>,
    page: &crate::document::PdfPage,
    resources: &PageResources,
    reader: &crate::PdfReader,
    scope: Option<&form_text::Scope>,
    anchor: &SelectedMultiRunOperand,
    stream_sources: &BTreeMap<(u32, u16), (Arc<PdfObject>, Arc<Vec<u8>>)>,
    actual_text_cleanup_patches: &[ActualTextCleanupPatch],
    signature_policy: EditPolicyReport,
) -> Result<(Vec<u8>, MultiRunTextEditReport)> {
    if request.logical_start != request.logical_end || request.replacement_text.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "source-style insertion helper received a non-insertion request".into(),
        ));
    }
    if request
        .replacement_text
        .chars()
        .any(crate::fonts::hard_break::is_hard_break)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "source-style zero-width insertion requires one logical line inside the original BT/ET text object"
                .into(),
        ));
    }
    // Validate the complete inherited paint state before choosing a replacement
    // font. The positioned writer consumes these exact source values; it does
    // not approximate them from page-level defaults.
    let _style = preserved_style_from_token(&anchor.4)?;
    let source_font = resources
        .fonts
        .get(&anchor.5.font_resource)
        .ok_or_else(|| WellfriendError::MalformedPdf("insertion source font disappeared".into()))?;
    let resolver = FontResolver::new(source_font, reader);
    let source_vertical = resolver.is_vertical();
    let generated_vertical = request.mode == AdvancedTextMode::ParagraphReflowVertical;
    if source_vertical != generated_vertical {
        return Err(WellfriendError::UnsupportedFeature(
            "source-style insertion must use the adjacent source writing mode; select the matching horizontal/vertical mode or use explicit supplied style"
                .into(),
        ));
    }
    let supports = |font: &[u8]| {
        crate::fonts::pdf_embedding::EmbeddingInfo::parse(font).is_ok()
            && ttf_parser::Face::parse(font, 0).is_ok()
            && TextShaper::shape(font, &request.replacement_text, ShapeOptions::default())
                .is_ok_and(|run| {
                    crate::fonts::shaper::has_missing_glyphs(font, &request.replacement_text, &run)
                        .is_ok_and(|missing| !missing)
                })
    };
    let logical_only_replacement = story_carriers::needs_text_carrier(&request.replacement_text);
    let embedded = crate::fonts::provider::embedded_program(reader, source_font);
    let font = if logical_only_replacement {
        get_fallback_font("Symbol").filter(|candidate| supports(candidate))
    } else {
        embedded
            .as_deref()
            .filter(|candidate| supports(candidate))
            .or_else(|| font_bytes.filter(|candidate| supports(candidate)))
            .or_else(|| get_fallback_font("Symbol").filter(|candidate| supports(candidate)))
    }
    .ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "source-style insertion has no embeddable font with complete shaped coverage".into(),
        )
    })?;
    let face = ttf_parser::Face::parse(font, 0).map_err(|_| {
        WellfriendError::UnsupportedFeature(
            "source-style insertion requires a valid standalone sfnt font".into(),
        )
    })?;
    let (visible_text, resolved_bidi) = if let Some(lines) = request.final_lines.as_deref() {
        let [line] = lines else {
            return Err(WellfriendError::UnsupportedFeature(
                "source-style zero-width insertion accepts exactly one final line inside the original text object"
                    .into(),
            ));
        };
        if line.logical_text != request.replacement_text
            || line.inserted_visual_hyphen
            || line.visual_text != line.logical_text
        {
            return Err(WellfriendError::UnsupportedFeature(
                "source-style zero-width insertion final line must preserve the exact replacement text without a discretionary visual hyphen"
                    .into(),
            ));
        }
        let bidi = match &line.bidi {
            Some(bidi) => bidi.clone(),
            None => {
                let options = ShapeOptions {
                    direction: Some(if request.mode == AdvancedTextMode::ParagraphReflowRtl {
                        TextDirection::RightToLeft
                    } else {
                        TextDirection::LeftToRight
                    }),
                };
                crate::fonts::shaper::ParagraphBidi::new(&line.visual_text, options)?
                    .line(0..line.visual_text.len())?
            }
        };
        (line.visual_text.as_str(), bidi)
    } else {
        let options = ShapeOptions {
            direction: Some(if request.mode == AdvancedTextMode::ParagraphReflowRtl {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            }),
        };
        let bidi = crate::fonts::shaper::ParagraphBidi::new(&request.replacement_text, options)?
            .line(0..request.replacement_text.len())?;
        (request.replacement_text.as_str(), bidi)
    };
    let mut glyphs = if logical_only_replacement {
        story_carriers::inline_glyphs(visible_text, &face)?
    } else if generated_vertical {
        vertical_text::glyph_plan(visible_text, font, Some(&resolved_bidi))?
    } else {
        let shaped =
            TextShaper::shape_resolved(font, visible_text, &resolved_bidi, &Default::default())?;
        generated_glyphs_from_shaped(visible_text, font, shaped)?
    };
    if glyphs.iter().any(|glyph| glyph.gid == 0) {
        return Err(WellfriendError::UnsupportedFeature(
            "source-style insertion produced a missing glyph".into(),
        ));
    }
    if !logical_only_replacement {
        for (index, glyph) in glyphs.iter_mut().enumerate() {
            glyph.cid = u16::try_from(index + 1).map_err(|_| {
                WellfriendError::ResourceLimit(
                    "source-style insertion exceeds 65535 shaped glyphs".into(),
                )
            })?;
        }
    }

    let local_scalar = request
        .logical_start
        .checked_sub(anchor.5.logical_range[0])
        .filter(|offset| *offset <= anchor.5.logical_range[1] - anchor.5.logical_range[0])
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "source-style insertion is outside its provenance anchor".into(),
            )
        })?;
    let (prefix, selected_bytes, suffix) = split_source_text_bytes_at_scalars(
        &resolver,
        &anchor.4.decoded,
        local_scalar,
        local_scalar,
    )?;
    debug_assert!(selected_bytes.is_empty());
    let base =
        reserve_advanced_object_block(reader, 6, "advanced_editing inline inherited insertion")?;
    let font_resource = deterministic_font_resource_name_for_selected_owners(
        reader,
        &page.resources,
        std::slice::from_ref(anchor),
    )?;
    let mut inline_edits = DecodedStreamEdits::new();
    add_actual_text_insertion_edits(
        &mut inline_edits,
        stream_sources,
        actual_text_cleanup_patches,
        request.logical_start,
        &request.replacement_text,
    )?;
    let (edit_start, edit_end, replacement) = rewrite_source_text_inline_generated(
        &anchor.4,
        &anchor.4,
        &resolver,
        &prefix,
        &selected_bytes,
        &request.replacement_text,
        &glyphs,
        &font_resource,
        generated_vertical,
        source_vertical,
        &suffix,
    )?;
    inline_edits
        .entry((anchor.0, anchor.1))
        .or_insert_with(|| {
            (
                anchor.2.as_ref().clone(),
                anchor.3.as_ref().clone(),
                Vec::new(),
            )
        })
        .2
        .push((edit_start, edit_end, replacement));
    let source_updates = materialize_decoded_stream_edits(
        inline_edits,
        "advanced_editing inline inherited insertion source",
    )?;
    if logical_only_replacement {
        let mut used_codes = BTreeSet::new();
        glyphs.retain(|glyph| used_codes.insert(glyph.cid));
    }
    let mut changed = build_type0_font_objects(
        font,
        &glyphs,
        generated_vertical,
        base,
        base + 1,
        base + 2,
        base + 3,
        base + 4,
        base + 5,
    )?;
    if logical_only_replacement {
        let dict = changed
            .iter_mut()
            .find(|object| object.number == base + 5)
            .and_then(|object| object.object.as_dict_mut())
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "source-style inline logical carrier font is missing".into(),
                )
            })?;
        dict.insert("WFLogicalLineCarrier", PdfObject::Integer(1));
    }
    changed.extend(source_updates);
    install_generated_font_in_selected_form_updates(
        reader,
        std::slice::from_ref(anchor),
        &mut changed,
        &font_resource,
        base + 5,
    )?;
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("source-style insertion page is not a dictionary".into())
    })?;
    let mut page_resources = page.resources.clone();
    let mut fonts = resolve_advanced_editing_dict(page_resources.get("Font"), reader)
        .unwrap_or_else(crate::PdfDictionary::empty);
    fonts.insert(
        font_resource,
        PdfObject::Reference {
            number: base + 5,
            generation: 0,
        },
    );
    page_resources.insert("Font", PdfObject::Dictionary(fonts));
    page_dict.insert("Resources", PdfObject::Dictionary(page_resources));
    changed.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(page_dict),
    });
    let output = form_text::write_scope(reader, page, resources, changed, scope)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let extracted = form_text::extract_scope(&reopened, request.page, scope)?;
    let replacement_extracts = extracted.contains(&request.replacement_text)
        || request
            .final_lines
            .as_ref()
            .is_some_and(|_| layout_extraction_equivalent(&extracted, &request.replacement_text));
    if !replacement_extracts || !output.starts_with(input) {
        return Err(WellfriendError::MalformedPdf(
            "source-style inline insertion save/reopen/extraction proof failed".into(),
        ));
    }
    Ok((
        output.clone(),
        MultiRunTextEditReport {
            schema_version:
                "advanced_editing_closeout.multirun-form-appearance-closure.v1".into(),
            status: AdvancedEditingSupportStatus::ImplementedWithLimits,
            operation: match request.style_policy {
                MultiRunStylePolicy::InheritLeading => {
                    "insert_inline_inheriting_leading_source_style".into()
                }
                MultiRunStylePolicy::InheritTrailing => {
                    "insert_inline_inheriting_trailing_source_style".into()
                }
                _ => unreachable!(),
            },
            page: request.page,
            logical_range: [request.logical_start, request.logical_end],
            selected_source_spans: vec![anchor.5.clone()],
            style_policy: request.style_policy,
            generated_font_used: true,
            generated_paint_order: None,
            generated_paint_partitions: Vec::new(),
            replacement_text: request.replacement_text.clone(),
            replacement_extracts,
            old_selected_text_absent: true,
            unrelated_text_preserved: true,
            reachable_source_tokens_removed: false,
            output_reopened: true,
            original_prefix_preserved: true,
            output_sha256: format!("{:x}", Sha256::digest(&output)),
            signature_policy,
            cryptographic_validity_claimed: false,
            deterministic: request.options.deterministic,
            cache_invalidation: advanced_editing_cache_invalidation(
                input, &output, true, false, false,
            ),
            exact_limits: vec![
                "zero-width leading inheritance selects the following source run; trailing inheritance selects the preceding run, falling back only at document edges"
                    .into(),
                "the inherited run binds font size, spacing, scaling, rise, render mode, writing mode and exact fill/stroke paint commands while an embedded or approved coverage font supplies shaped outlines"
                    .into(),
                "the insertion is emitted inside the original BT/ET and marked-content scope; clipping render modes therefore union the generated outlines before the original ET establishes the clip"
                    .into(),
                "the positioned writer preserves resolved transformed source matrices and restores Tlm/Tm with exact numeric writing-axis displacement without matrix inversion"
                    .into(),
                "no source glyph code is removed; the generated nested ActualText span owns only the inserted logical text"
                    .into(),
            ],
        },
    ))
}

fn collect_annotation_appearance_vectors(
    reader: &crate::reader::PdfReader,
    page: &crate::document::PdfPage,
    page_number: usize,
    stream_index_base: usize,
    output: &mut Vec<EditableVectorObject>,
) -> Result<()> {
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let Some(page_dict) = page_object.as_dict() else {
        return Ok(());
    };
    let Some(annots_object) = page_dict.get("Annots") else {
        return Ok(());
    };
    let annots = reader.resolve(annots_object.clone())?;
    let Some(annotation_entries) = annots.as_array() else {
        return Ok(());
    };
    let mut appearances = Vec::<(usize, String, u32, u16)>::new();
    for (annotation_index, annotation_entry) in annotation_entries.iter().enumerate() {
        let Ok(annotation) = reader.resolve(annotation_entry.clone()) else {
            continue;
        };
        let Some(annotation_dict) = annotation.as_dict() else {
            continue;
        };
        let Some(ap) = resolve_advanced_editing_dict(annotation_dict.get("AP"), reader) else {
            continue;
        };
        for appearance_key in ["N", "R", "D"] {
            let Some(appearance) = ap.get(appearance_key) else {
                continue;
            };
            if let Some((number, generation)) = appearance.as_reference() {
                appearances.push((
                    annotation_index,
                    appearance_key.to_string(),
                    number,
                    generation,
                ));
                continue;
            }
            if let Some(states) = resolve_advanced_editing_dict(Some(appearance), reader) {
                for (state, value) in states.entries() {
                    if let Some((number, generation)) = value.as_reference() {
                        appearances.push((
                            annotation_index,
                            format!("{appearance_key}/{state}"),
                            number,
                            generation,
                        ));
                    }
                }
            }
        }
    }
    for (appearance_index, (annotation_index, appearance_name, number, generation)) in
        appearances.iter().enumerate()
    {
        let use_count = appearances
            .iter()
            .filter(|(_, _, candidate_number, candidate_generation)| {
                candidate_number == number && candidate_generation == generation
            })
            .count();
        let Ok(PdfObject::Stream { dict, raw }) = reader.get_object(*number, *generation) else {
            continue;
        };
        let decoded = decode_stream_lossless_with_limits(
            &PdfObject::Stream {
                dict: dict.clone(),
                raw,
            },
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            continue;
        }
        let mut vectors = reconstruct_vector_objects(
            &decoded.data,
            page_number,
            stream_index_base + appearance_index,
            *number,
            *generation,
        )?;
        for vector in &mut vectors {
            vector.provenance.form_stack = vec![format!(
                "annotation:{annotation_index}:appearance:{appearance_name}"
            )];
            vector.provenance.resource_owner = format!(
                "annotation-{annotation_index}-appearance-{appearance_name}-{number}-{generation}"
            );
            vector.edit_safety = if use_count == 1 {
                "safe_annotation_appearance_operation_range"
            } else {
                "shared_annotation_appearance_requires_clone"
            }
            .to_string();
            if use_count > 1 {
                vector.diagnostics.push(format!(
                    "appearance stream {number} {generation} R has {use_count} annotation uses; direct mutation is rejected"
                ));
            }
            vector.stable_id = vector_stable_id_for_object(vector);
        }
        output.extend(vectors);
        let appearance_resources =
            resolve_advanced_editing_dict(dict.get("Resources"), reader).unwrap_or_default();
        collect_form_vector_objects(
            reader,
            &page.resources,
            &appearance_resources,
            &decoded.data,
            page_number,
            stream_index_base + appearance_index,
            *number,
            *generation,
            pdf_matrix(dict.get("Matrix")).unwrap_or(VectorMatrix::IDENTITY),
            &[format!(
                "annotation:{annotation_index}:appearance:{appearance_name}"
            )],
            &[],
            &mut Vec::new(),
            output,
        )?;
    }
    Ok(())
}

fn validate_advanced_text_options(options: &AdvancedTextEditOptions) -> Result<()> {
    if options
        .region
        .iter()
        .chain(
            [
                options.font_size,
                options.line_spacing,
                options.max_word_spacing,
                options.max_character_spacing,
            ]
            .iter(),
        )
        .any(|value| !value.is_finite())
        || options.region[0] >= options.region[2]
        || options.region[1] >= options.region[3]
        || options.font_size <= 0.0
        || options.line_spacing <= 0.0
        || options.max_word_spacing < 0.0
        || options.max_character_spacing < 0.0
        || !(options.region[2] - options.region[0]).is_finite()
        || !(options.region[3] - options.region[1]).is_finite()
        || !(options.font_size * options.line_spacing).is_finite()
        || options.font_size * options.line_spacing <= 0.0
    {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing requires finite positive region extents, font size and line advance, and finite nonnegative spacing limits"
                .to_string(),
        ));
    }
    if options.max_lines_or_columns == 0 || options.max_lines_or_columns > 10_000 {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing line/column limit must be in 1..=10000".to_string(),
        ));
    }
    if options.target_stream_object.is_some() != options.target_stream_generation.is_some() {
        return Err(WellfriendError::invalid_input(
            "advanced_editing selected reflow source requires both stream object and generation",
        ));
    }
    if options
        .target_decoded_byte_range
        .is_some_and(|range| range[0] >= range[1])
    {
        return Err(WellfriendError::invalid_input(
            "advanced_editing selected reflow source byte range is empty or reversed",
        ));
    }
    if options.paint_partitions.len() > MAX_ADVANCED_EDITING_BIDI_RUNS {
        return Err(WellfriendError::ResourceLimit(
            "advanced_editing generated paint partition count exceeds 4096".into(),
        ));
    }
    if !options.paint_partitions.is_empty()
        && options.paint_order_policy != GeneratedPaintOrderPolicy::RequireSingleSourceTextObject
    {
        return Err(WellfriendError::invalid_input(
            "generated paint partitions and first/last block anchoring are mutually exclusive",
        ));
    }
    for partition in &options.paint_partitions {
        if partition.replacement_scalar_range[0] > partition.replacement_scalar_range[1]
            || partition.region.iter().any(|value| !value.is_finite())
            || partition.region[0] >= partition.region[2]
            || partition.region[1] >= partition.region[3]
        {
            return Err(WellfriendError::invalid_input(
                "generated paint partition has a reversed scalar range or invalid region",
            ));
        }
    }
    Ok(())
}

fn horizontal_line_capacity(options: &AdvancedTextEditOptions) -> Result<usize> {
    validate_advanced_text_options(options)?;
    let available = options.region[3] - options.region[1] - options.font_size;
    if available < 0.0 {
        return Ok(0);
    }
    // Clamp in the floating-point domain before converting or adding. A valid
    // finite frame and tiny positive line advance can produce an infinite ratio.
    let intervals = (available / (options.font_size * options.line_spacing)).floor();
    Ok(1 + intervals.min((options.max_lines_or_columns - 1) as f64) as usize)
}

fn generated_glyph_plan(
    text: &str,
    mode: AdvancedTextMode,
    font: &[u8],
) -> Result<Vec<GeneratedGlyph>> {
    if mode == AdvancedTextMode::ParagraphReflowVertical {
        return vertical_text::glyph_plan(text, font, None);
    }
    let shaped = TextShaper::shape(
        font,
        text,
        ShapeOptions {
            direction: Some(if mode == AdvancedTextMode::ParagraphReflowRtl {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            }),
        },
    )?;
    generated_glyphs_from_shaped(text, font, shaped)
}

/// Measure exactly the horizontal advance later emitted by
/// `serialize_generated_preserved_styles` for one paragraph-derived line.
/// Authored table redistribution uses this instead of a second approximate
/// width model, so the preflight decision and canonical writer share shaping,
/// clusters, source spacing and horizontal scaling.
pub(crate) fn measure_generated_preserved_line(
    text: &str,
    bidi: &crate::fonts::shaper::LineBidi,
    font: &[u8],
    font_size: f64,
    character_spacing: f64,
    word_spacing: f64,
    horizontal_scaling: f64,
) -> Result<f64> {
    if !font_size.is_finite()
        || font_size <= 0.0
        || !character_spacing.is_finite()
        || !word_spacing.is_finite()
        || !horizontal_scaling.is_finite()
        || horizontal_scaling <= 0.0
    {
        return Err(WellfriendError::MalformedPdf(
            "invalid authored generated-text measurement state".into(),
        ));
    }
    let shaped = TextShaper::shape_resolved(font, text, bidi, &Default::default())?;
    if crate::fonts::shaper::has_missing_glyphs(font, text, &shaped)? {
        return Err(WellfriendError::UnsupportedFeature(
            "approved authored-table shaping font lacks replacement glyph coverage".into(),
        ));
    }
    let glyphs = generated_glyphs_from_shaped(text, font, shaped)?;
    let width = glyphs.iter().try_fold(0.0, |width, glyph| {
        let advance = (glyph.advance.abs() / 1000.0 * font_size
            + character_spacing
            + if glyph.visual_unicode == " " {
                word_spacing
            } else {
                0.0
            })
            * (horizontal_scaling / 100.0);
        let width = width + advance;
        if width.is_finite() && width >= 0.0 {
            Ok(width)
        } else {
            Err(WellfriendError::MalformedPdf(
                "authored generated-text measurement overflow".into(),
            ))
        }
    })?;
    Ok(width)
}

fn generated_glyphs_from_shaped(
    text: &str,
    font: &[u8],
    shaped: crate::fonts::ShapedRun,
) -> Result<Vec<GeneratedGlyph>> {
    let missing = crate::fonts::shaper::missing_glyph_clusters(font, text, &shaped)?;
    if !missing.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "generated glyph output lacks coverage at UTF-8 clusters {missing:?}"
        )));
    }
    let face = ttf_parser::Face::parse(font, 0).map_err(|_| {
        WellfriendError::UnsupportedFeature("generated glyph plan needs a valid sfnt font".into())
    })?;
    let outliner = crate::fonts::sfnt_outline::Outliner::new(&face)?;
    let mut cluster_starts = shaped
        .glyphs
        .iter()
        .map(|g| g.cluster as usize)
        .collect::<Vec<_>>();
    cluster_starts.push(text.len());
    cluster_starts.sort_unstable();
    cluster_starts.dedup();
    let mut mapped = BTreeSet::new();
    let mut glyphs = Vec::new();
    let font_scale = 1000.0 / f64::from(face.units_per_em()).max(1.0);
    let mut glyph_bounds = BTreeMap::new();
    for (index, glyph) in shaped.glyphs.into_iter().enumerate() {
        if index % 1024 == 0 {
            crate::cancel::check_current_cancel("generated glyph conversion")?;
        }
        let start = glyph.cluster as usize;
        let end = cluster_starts
            .get(cluster_starts.partition_point(|n| *n <= start))
            .copied()
            .unwrap_or(text.len());
        let unicode = text
            .get(start..end)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf("shaper returned a non-Unicode cluster".into())
            })?
            .to_owned();
        let orientation = VerticalGlyphOrientation::Upright;
        let cid = {
            u16::try_from(glyphs.len() + 1).map_err(|_| {
                WellfriendError::ResourceLimit("generated CID count exceeds 65535".into())
            })?
        };
        let gid = ttf_parser::GlyphId(glyph.glyph_id);
        let bounds = if let Some(bounds) = glyph_bounds.get(&glyph.glyph_id) {
            *bounds
        } else {
            let bounds = outliner.bounds(gid)?.map(|b| {
                [
                    f64::from(b.x_min) * font_scale,
                    f64::from(b.y_min) * font_scale,
                    f64::from(b.x_max) * font_scale,
                    f64::from(b.y_max) * font_scale,
                ]
            });
            glyph_bounds.insert(glyph.glyph_id, bounds);
            bounds
        };
        glyphs.push(GeneratedGlyph {
            cid,
            gid: glyph.glyph_id,
            logical_byte_start: start,
            visual_unicode: unicode.clone(),
            to_unicode: mapped.insert(start).then_some(unicode),
            advance: glyph.advance,
            offset_x: glyph.offset_x,
            offset_y: glyph.offset_y,
            orientation,
            cross_advance: 0.0,
            font_width: f64::from(face.glyph_hor_advance(gid).unwrap_or(face.units_per_em()))
                * font_scale,
            bounds,
        });
    }
    Ok(glyphs)
}

fn layout_generated_explicit_lines(
    lines: &[ExplicitLayoutLine],
    mode: AdvancedTextMode,
    font: &[u8],
    options: &AdvancedTextEditOptions,
    line_regions: Option<&[[f64; 4]]>,
) -> Result<Vec<Vec<GeneratedGlyph>>> {
    if line_regions.is_some_and(|regions| regions.len() != lines.len()) {
        return Err(WellfriendError::invalid_input(
            "advanced_editing positioned final layout has mismatched line regions",
        ));
    }
    if line_regions.is_none() && lines.len() > options.max_lines_or_columns {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing explicit final layout has {} lines/columns; limit is {}",
            lines.len(),
            options.max_lines_or_columns
        )));
    }
    if line_regions.is_none()
        && mode != AdvancedTextMode::ParagraphReflowVertical
        && lines.len() > horizontal_line_capacity(options)?
    {
        return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing explicit final layout exceeds frame line capacity; use linked-story flow".into()));
    }
    let _face = ttf_parser::Face::parse(font, 0).map_err(|_| {
        WellfriendError::UnsupportedFeature(
            "advanced_editing explicit layout requires a valid sfnt font".to_string(),
        )
    })?;
    let mut next_cid = 1u32;
    let mut layout = Vec::with_capacity(lines.len());
    let paragraph_text = lines
        .iter()
        .map(|line| line.logical_text.as_str())
        .collect::<String>();
    let shape_options = ShapeOptions {
        direction: Some(if mode == AdvancedTextMode::ParagraphReflowRtl {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        }),
    };
    let paragraph_bidi = crate::fonts::shaper::ParagraphBidi::new(&paragraph_text, shape_options)?;
    let mut logical_offset = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let region = line_regions
            .and_then(|regions| regions.get(index).copied())
            .unwrap_or(options.region);
        let available = if mode == AdvancedTextMode::ParagraphReflowVertical {
            (region[3] - region[1]) / options.font_size * 1000.0
        } else {
            (region[2] - region[0]) / options.font_size * 1000.0
        };
        let visual_base = line
            .logical_text
            .trim_end_matches(crate::fonts::hard_break::is_hard_break);
        let expected = if line.inserted_visual_hyphen {
            format!("{visual_base}-")
        } else {
            visual_base.to_owned()
        };
        if line.visual_text != expected {
            return Err(WellfriendError::invalid_input(
                "explicit line differs from its logical text beyond a declared hyphen",
            ));
        }
        let bidi = if let Some(resolved) = &line.bidi {
            resolved.clone()
        } else if line.inserted_visual_hyphen {
            let insertion = logical_offset + visual_base.len();
            let mut virtual_paragraph = paragraph_text.clone();
            virtual_paragraph.insert(insertion, '-');
            crate::fonts::shaper::resolve_line_bidi(
                &virtual_paragraph,
                logical_offset..insertion + 1,
                shape_options,
            )?
        } else {
            paragraph_bidi.line(logical_offset..logical_offset + visual_base.len())?
        };
        let mut glyphs = if mode == AdvancedTextMode::ParagraphReflowVertical {
            vertical_text::glyph_plan(&line.visual_text, font, Some(&bidi))?
        } else {
            let shaped =
                TextShaper::shape_resolved(font, &line.visual_text, &bidi, &Default::default())?;
            generated_glyphs_from_shaped(&line.visual_text, font, shaped)?
        };
        logical_offset += line.logical_text.len();
        let advance = if mode == AdvancedTextMode::ParagraphReflowVertical {
            vertical_text::extent(&glyphs, options.font_size)? / options.font_size * 1000.0
        } else {
            glyphs.iter().map(|glyph| glyph.advance.abs()).sum::<f64>()
        };
        if advance > available + EPSILON {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing explicit final layout line exceeds its source region".to_string(),
            ));
        }
        for glyph in &mut glyphs {
            glyph.cid = u16::try_from(next_cid).map_err(|_| {
                WellfriendError::UnsupportedFeature(
                    "advanced_editing explicit final layout CID count exceeds 65535".to_string(),
                )
            })?;
            next_cid += 1;
        }
        if line.inserted_visual_hyphen {
            let Some(last) = glyphs
                .iter_mut()
                .find(|glyph| glyph.logical_byte_start == line.visual_text.len() - 1)
            else {
                return Err(WellfriendError::MalformedPdf(
                    "advanced_editing inserted dictionary hyphen line has no bound glyph"
                        .to_string(),
                ));
            };
            if last.visual_unicode != "-" {
                return Err(WellfriendError::MalformedPdf(
                    "advanced_editing inserted dictionary hyphen must be the final generated glyph"
                        .to_string(),
                ));
            }
            // An explicit empty ToUnicode mapping preserves logical source
            // extraction while retaining the visible shaped hyphen CID.  A
            // missing mapping would make some extractors fall back to a
            // font-program glyph name and incorrectly expose `-`.
            last.to_unicode = Some(String::new());
        }
        layout.push(glyphs);
    }
    Ok(layout)
}

fn layout_extraction_equivalent(extracted: &str, expected: &str) -> bool {
    extracted.split_whitespace().collect::<String>()
        == expected.split_whitespace().collect::<String>()
}

fn deterministic_font_resource_name(
    reader: &crate::PdfReader,
    resources: &crate::PdfDictionary,
) -> String {
    let existing = match resources.get("Font") {
        Some(PdfObject::Dictionary(dict)) => Some(dict.clone()),
        Some(reference @ PdfObject::Reference { .. }) => reader
            .resolve(reference.clone())
            .ok()
            .and_then(|object| object.as_dict().cloned()),
        _ => None,
    };
    for index in 0..10_000 {
        let name = if index == 0 {
            "OxP20F".to_string()
        } else {
            format!("OxP20F{index}")
        };
        if existing
            .as_ref()
            .is_none_or(|dict| !dict.contains_key(&name))
        {
            return name;
        }
    }
    "OxP20FOverflow".to_string()
}

/// Choose one generated font resource name that is unused both on the page
/// and in every selected Form XObject. A page-only collision check is not
/// sufficient: content rewritten inside a Form resolves `/Font` against the
/// Form's own resource dictionary and could otherwise replace an unrelated
/// font used by sibling operations in that Form.
fn deterministic_font_resource_name_for_selected_owners(
    reader: &crate::PdfReader,
    page_resources: &crate::PdfDictionary,
    selected: &[SelectedMultiRunOperand],
) -> Result<String> {
    let mut occupied = BTreeSet::<String>::new();
    if let Some(fonts) = resolve_advanced_editing_dict(page_resources.get("Font"), reader) {
        occupied.extend(fonts.entries().map(|(name, _)| name.clone()));
    }
    let mut visited = BTreeSet::<(u32, u16)>::new();
    for item in selected {
        if !visited.insert((item.0, item.1)) {
            continue;
        }
        let PdfObject::Stream { dict, .. } = item.2.as_ref() else {
            continue;
        };
        if dict.get_name("Subtype") != Some("Form") {
            continue;
        }
        if let Some(resources) = resolve_advanced_editing_dict(dict.get("Resources"), reader) {
            if let Some(fonts) = resolve_advanced_editing_dict(resources.get("Font"), reader) {
                occupied.extend(fonts.entries().map(|(name, _)| name.clone()));
            }
        }
    }
    for index in 0..10_000 {
        let name = if index == 0 {
            "OxP20F".to_string()
        } else {
            format!("OxP20F{index}")
        };
        if !occupied.contains(&name) {
            return Ok(name);
        }
    }
    Err(WellfriendError::ResourceLimit(
        "advanced_editing exhausted generated font resource names across selected owners"
            .to_string(),
    ))
}

/// Install a generated Type0 font into the resource dictionary of every Form
/// stream whose text was rewritten inline. Page content uses the separately
/// updated page `/Resources`; nested Form content cannot see that dictionary
/// and therefore receives a direct, collision-checked resource entry here.
fn install_generated_font_in_selected_form_updates(
    reader: &crate::PdfReader,
    selected: &[SelectedMultiRunOperand],
    changed: &mut [IncrementalObject],
    font_resource: &str,
    type0_number: u32,
) -> Result<()> {
    let form_owners = selected
        .iter()
        .filter_map(|item| match item.2.as_ref() {
            PdfObject::Stream { dict, .. } if dict.get_name("Subtype") == Some("Form") => {
                Some((item.0, item.1))
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if form_owners.is_empty() {
        return Ok(());
    }
    let mut updated = BTreeSet::<(u32, u16)>::new();
    for object in changed {
        if !form_owners.contains(&(object.number, object.generation)) {
            continue;
        }
        let PdfObject::Stream { dict, .. } = &mut object.object else {
            return Err(WellfriendError::MalformedPdf(format!(
                "advanced_editing selected Form {} {} update is not a stream",
                object.number, object.generation
            )));
        };
        let mut resources = resolve_advanced_editing_dict(dict.get("Resources"), reader)
            .unwrap_or_else(crate::PdfDictionary::empty);
        let mut fonts = resolve_advanced_editing_dict(resources.get("Font"), reader)
            .unwrap_or_else(crate::PdfDictionary::empty);
        if let Some(existing) = fonts.get(font_resource) {
            if !matches!(existing, PdfObject::Reference { number, generation: 0 } if *number == type0_number)
            {
                return Err(WellfriendError::MalformedPdf(format!(
                    "advanced_editing generated font resource /{font_resource} collides inside Form {} {}",
                    object.number, object.generation
                )));
            }
        } else {
            fonts.insert(
                font_resource,
                PdfObject::Reference {
                    number: type0_number,
                    generation: 0,
                },
            );
        }
        resources.insert("Font", PdfObject::Dictionary(fonts));
        dict.insert("Resources", PdfObject::Dictionary(resources));
        updated.insert((object.number, object.generation));
    }
    if updated != form_owners {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing could not attach the generated font to every rewritten Form owner"
                .to_string(),
        ));
    }
    Ok(())
}

/// Reserve one contiguous block above every active source object. The caller
/// may then use `base + offset` only for offsets proven smaller than `count`.
/// Saturation would alias an existing object and turn resource exhaustion into
/// unrelated-content corruption, so every exhaustion path is explicit.
fn reserve_advanced_object_block(
    reader: &crate::PdfReader,
    count: u32,
    context: &str,
) -> Result<u32> {
    if count == 0 {
        return Err(WellfriendError::invalid_input(format!(
            "{context} requested an empty PDF object-number reservation"
        )));
    }
    let base = reader
        .object_ids()
        .into_iter()
        .map(|(number, _)| number)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            WellfriendError::ResourceLimit(format!(
                "{context} exhausted the PDF object-number space"
            ))
        })?;
    base.checked_add(count - 1).ok_or_else(|| {
        WellfriendError::ResourceLimit(format!(
            "{context} cannot reserve {count} contiguous PDF object numbers"
        ))
    })?;
    Ok(base)
}

fn next_advanced_object_number(reader: &crate::PdfReader, context: &str) -> Result<u32> {
    reserve_advanced_object_block(reader, 1, context)
}

fn page_graphics_state_isolation_prefix(number: u32) -> IncrementalObject {
    let mut dict = crate::PdfDictionary::empty();
    dict.insert("Length", PdfObject::Integer(2));
    IncrementalObject {
        number,
        generation: 0,
        object: PdfObject::Stream {
            dict,
            raw: b"q\n".to_vec(),
        },
    }
}

fn isolated_appended_content(content: String) -> String {
    format!("Q\n{content}")
}

fn layout_generated_logical_text(
    text: &str,
    mode: AdvancedTextMode,
    font: &[u8],
    options: &AdvancedTextEditOptions,
) -> Result<Vec<Vec<GeneratedGlyph>>> {
    if mode == AdvancedTextMode::ParagraphReflowVertical {
        return vertical_text::layout(text, font, options);
    }
    let lines = crate::fonts::line_layout::break_lines(
        font,
        text,
        options.font_size,
        options.region[2] - options.region[0],
        ShapeOptions {
            direction: Some(if mode == AdvancedTextMode::ParagraphReflowRtl {
                TextDirection::RightToLeft
            } else {
                TextDirection::LeftToRight
            }),
        },
    )?;
    if lines.len() > horizontal_line_capacity(options)? {
        return Err(WellfriendError::UnsupportedFeature(
            "shaped text exceeds frame capacity; use linked-story flow".into(),
        ));
    }
    let explicit = lines
        .iter()
        .map(|line| {
            let logical_text = text[line.bytes.clone()].to_string();
            let visual_text = logical_text
                .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                .to_string();
            ExplicitLayoutLine {
                logical_text,
                visual_text,
                inserted_visual_hyphen: false,
                bidi: None,
            }
        })
        .collect::<Vec<_>>();
    layout_generated_explicit_lines(&explicit, mode, font, options, None)
}

fn layout_generated_replacement(
    text: &str,
    final_lines: Option<&[ExplicitLayoutLine]>,
    mode: AdvancedTextMode,
    font: &[u8],
    options: &AdvancedTextEditOptions,
) -> Result<Vec<Vec<GeneratedGlyph>>> {
    let Some(lines) = final_lines else {
        return layout_generated_logical_text(text, mode, font, options);
    };
    if lines.is_empty()
        || lines
            .iter()
            .map(|line| line.logical_text.as_str())
            .collect::<String>()
            != text
    {
        return Err(WellfriendError::invalid_input(
            "explicit final lines must concatenate exactly to their replacement text",
        ));
    }
    for line in lines {
        let visual_base = line
            .logical_text
            .trim_end_matches(crate::fonts::hard_break::is_hard_break);
        let allowed_visual = if line.inserted_visual_hyphen {
            format!("{visual_base}-")
        } else {
            visual_base.to_string()
        };
        if line.visual_text != allowed_visual {
            return Err(WellfriendError::UnsupportedFeature(
                "visual final layout permits only trailing mandatory separators and one end-of-line dictionary hyphen"
                    .into(),
            ));
        }
    }
    layout_generated_explicit_lines(lines, mode, font, options, None)
}

fn layout_generated_partition_with_context(
    full_text: &str,
    segment_byte_range: std::ops::Range<usize>,
    final_lines: Option<&[ExplicitLayoutLine]>,
    mode: AdvancedTextMode,
    font: &[u8],
    options: &AdvancedTextEditOptions,
) -> Result<Vec<Vec<GeneratedGlyph>>> {
    let text = full_text.get(segment_byte_range.clone()).ok_or_else(|| {
        WellfriendError::invalid_input("paint partition byte range is outside replacement text")
    })?;
    if mode == AdvancedTextMode::ParagraphReflowVertical && final_lines.is_none() {
        return vertical_text::layout_with_context(full_text, segment_byte_range, font, options);
    }

    let mut lines = if let Some(lines) = final_lines {
        lines.to_vec()
    } else {
        let broken = crate::fonts::line_layout::break_lines(
            font,
            text,
            options.font_size,
            options.region[2] - options.region[0],
            ShapeOptions {
                direction: Some(if mode == AdvancedTextMode::ParagraphReflowRtl {
                    TextDirection::RightToLeft
                } else {
                    TextDirection::LeftToRight
                }),
            },
        )?;
        if broken.len() > horizontal_line_capacity(options)? {
            return Err(WellfriendError::UnsupportedFeature(
                "partitioned shaped text exceeds its frame capacity".into(),
            ));
        }
        broken
            .iter()
            .map(|line| {
                let logical_text = text[line.bytes.clone()].to_string();
                let visual_text = logical_text
                    .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                    .to_string();
                ExplicitLayoutLine {
                    logical_text,
                    visual_text,
                    inserted_visual_hyphen: false,
                    bidi: None,
                }
            })
            .collect::<Vec<_>>()
    };
    if lines.is_empty()
        || lines
            .iter()
            .map(|line| line.logical_text.as_str())
            .collect::<String>()
            != text
    {
        return Err(WellfriendError::invalid_input(
            "partition final lines must concatenate exactly to their segment text",
        ));
    }

    let shape_options = ShapeOptions {
        direction: Some(if mode == AdvancedTextMode::ParagraphReflowRtl {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        }),
    };
    let full_bidi = crate::fonts::shaper::ParagraphBidi::new(full_text, shape_options)?;
    let mut local_byte = 0usize;
    for line in &mut lines {
        let visual_base = line
            .logical_text
            .trim_end_matches(crate::fonts::hard_break::is_hard_break);
        let allowed_visual = if line.inserted_visual_hyphen {
            format!("{visual_base}-")
        } else {
            visual_base.to_string()
        };
        if line.visual_text != allowed_visual {
            return Err(WellfriendError::UnsupportedFeature(
                "partition visual line differs from logical text beyond one declared hyphen".into(),
            ));
        }
        let global_start = segment_byte_range.start + local_byte;
        let global_end = global_start + visual_base.len();
        line.bidi = Some(if line.inserted_visual_hyphen {
            let mut virtual_text = full_text.to_string();
            virtual_text.insert(global_end, '-');
            crate::fonts::shaper::ParagraphBidi::new(&virtual_text, shape_options)?
                .line(global_start..global_end + 1)?
        } else {
            full_bidi.line(global_start..global_end)?
        });
        local_byte = local_byte
            .checked_add(line.logical_text.len())
            .ok_or_else(|| WellfriendError::ResourceLimit("partition line byte offset".into()))?;
    }
    if local_byte != text.len() {
        return Err(WellfriendError::MalformedPdf(
            "partition line byte coverage drifted".into(),
        ));
    }
    let mut layout = layout_generated_explicit_lines(&lines, mode, font, options, None)?;
    let mut line_byte_base = 0usize;
    for (line, glyphs) in lines.iter().zip(layout.iter_mut()) {
        for glyph in glyphs {
            glyph.logical_byte_start = glyph.logical_byte_start.saturating_add(line_byte_base);
        }
        line_byte_base = line_byte_base
            .checked_add(line.logical_text.len())
            .ok_or_else(|| WellfriendError::ResourceLimit("partition line byte offset".into()))?;
    }
    if line_byte_base != text.len() {
        return Err(WellfriendError::MalformedPdf(
            "partition layout byte coverage drifted".into(),
        ));
    }
    Ok(layout)
}

/// Put a reflow block immediately after its source text object, not above all
/// page artwork. Locate by ET ordinal in the original stream, then relocate in
/// the rewritten stream; byte offsets may have changed during ActualText cleanup.
/// Single-line in-place writers remain preferable for overlapping text within BT.
fn locate_source_paint_slots(
    reader: &crate::PdfReader,
    contents: &[(u32, u16)],
    sources: &[(u32, u16, usize)],
) -> Result<Vec<(usize, (u32, u16, usize))>> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let unique_sources = sources.iter().copied().collect::<BTreeSet<_>>();
    let mut wanted = BTreeMap::<(u32, u16), BTreeSet<usize>>::new();
    for &(number, generation, offset) in &unique_sources {
        if contents
            .iter()
            .filter(|&&item| item == (number, generation))
            .count()
            != 1
        {
            return Err(WellfriendError::UnsupportedFeature(
                "generated reflow source must have one exact page-content occurrence".into(),
            ));
        }
        wanted
            .entry((number, generation))
            .or_default()
            .insert(offset);
    }

    let decode = |object: &PdfObject| -> Result<Vec<u8>> {
        let decoded = decode_stream_lossless_with_limits(
            object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            return Err(WellfriendError::UnsupportedFeature(
                "opaque generated-reflow paint-order stream".into(),
            ));
        }
        Ok(decoded.data)
    };

    // Page Contents members form one logical content sequence, and valid
    // producers split container syntax as well as BT/ET state between members.
    // Tokenize the joined sequence once, while retaining exact local offsets.
    let mut data = Vec::new();
    let mut segments = Vec::with_capacity(contents.len());
    for (stream_index, &(number, generation)) in contents.iter().enumerate() {
        let member = decode(&reader.get_object(number, generation)?)?;
        let global_start = data.len();
        data.extend_from_slice(&member);
        let global_end = data.len();
        segments.push(PageContentSegment {
            stream_index,
            object: number,
            generation,
            global_start,
            global_end,
        });
        data.push(b'\n');
    }
    let mut active_slot = None;
    let mut next_slot = 0usize;
    let mut found = Vec::<(usize, (u32, u16, usize))>::new();
    for token in lex_content(&data)? {
        let segment = page_segment_for_range(&segments, token.start, token.end).ok_or_else(
                || {
                    WellfriendError::UnsupportedFeature(
                        "generated-reflow paint anchor token crosses a physical /Contents stream boundary"
                            .to_string(),
                    )
                },
            )?;
        let number = segment.object;
        let generation = segment.generation;
        let local_start = token.start - segment.global_start;
        if wanted
            .get(&(number, generation))
            .is_some_and(|offsets| offsets.contains(&local_start))
        {
            if !matches!(&token.kind, LexicalKind::String(_, _)) {
                return Err(WellfriendError::MalformedPdf(
                    "generated reflow paint anchor is not a PDF string token".into(),
                ));
            }
            let slot = active_slot.ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "generated reflow source string is outside a text object".into(),
                )
            })?;
            found.push((slot, (number, generation, local_start)));
        }
        let LexicalKind::Word(operator) = &token.kind else {
            continue;
        };
        match operator.as_str() {
            "BT" => {
                if active_slot.is_some() {
                    return Err(WellfriendError::MalformedPdf(
                        "nested source text objects are invalid".into(),
                    ));
                }
                active_slot = Some(next_slot);
                next_slot = next_slot.checked_add(1).ok_or_else(|| {
                    WellfriendError::ResourceLimit("source text-object count".into())
                })?;
            }
            "ET" if active_slot.take().is_none() => {
                return Err(WellfriendError::MalformedPdf(
                    "source content closes a text object that is not open".into(),
                ));
            }
            _ => {}
        }
    }
    if active_slot.is_some() {
        return Err(WellfriendError::MalformedPdf(
            "source content ends inside an open text object".into(),
        ));
    }
    if found.len() != unique_sources.len() {
        return Err(WellfriendError::MalformedPdf(format!(
            "generated reflow resolved {} of {} source paint anchors",
            found.len(),
            unique_sources.len()
        )));
    }
    Ok(found)
}

fn assign_source_paint_slots(
    reader: &crate::PdfReader,
    contents: &[(u32, u16)],
    spans: &mut [MultiRunSourceSpan],
) -> Result<()> {
    let sources = spans
        .iter()
        .map(|span| {
            (
                span.stream_object,
                span.stream_generation,
                span.byte_range[0],
            )
        })
        .collect::<Vec<_>>();
    let located = locate_source_paint_slots(reader, contents, &sources)?;
    let by_source = located
        .into_iter()
        .map(|(slot, source)| (source, slot))
        .collect::<BTreeMap<_, _>>();
    for span in spans {
        span.source_text_object = *by_source
            .get(&(
                span.stream_object,
                span.stream_generation,
                span.byte_range[0],
            ))
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "source span lost its exact text-object paint slot".into(),
                )
            })?;
    }
    Ok(())
}

fn select_generated_reflow_source(
    reader: &crate::PdfReader,
    contents: &[(u32, u16)],
    sources: &[(u32, u16, usize)],
    policy: GeneratedPaintOrderPolicy,
) -> Result<GeneratedPaintOrderDecision> {
    let found = locate_source_paint_slots(reader, contents, sources)?;
    if found.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "generated reflow has no source paint anchor".into(),
        ));
    }
    let slots = found.iter().map(|item| item.0).collect::<BTreeSet<_>>();
    if slots.len() > 1 && policy == GeneratedPaintOrderPolicy::RequireSingleSourceTextObject {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "ambiguous_generated_paint_order: selection spans {} source text objects; explicitly approve anchor_after_first_source_text_object or anchor_after_last_source_text_object",
            slots.len()
        )));
    }
    let chosen = match policy {
        GeneratedPaintOrderPolicy::AnchorAfterLastSourceTextObject => found.last(),
        GeneratedPaintOrderPolicy::RequireSingleSourceTextObject
        | GeneratedPaintOrderPolicy::AnchorAfterFirstSourceTextObject => found.first(),
    }
    .ok_or_else(|| WellfriendError::MalformedPdf("generated source set disappeared".into()))?;
    let (anchor_stream_object, anchor_stream_generation, anchor_decoded_byte_offset) = chosen.1;
    Ok(GeneratedPaintOrderDecision {
        policy,
        source_text_objects: slots.len(),
        anchor_stream_object,
        anchor_stream_generation,
        anchor_decoded_byte_offset,
    })
}

fn anchor_generated_reflow_for_sources(
    reader: &crate::PdfReader,
    contents: &[(u32, u16)],
    updates: &mut Vec<IncrementalObject>,
    sources: &[(u32, u16, usize)],
    policy: GeneratedPaintOrderPolicy,
    generated_number: u32,
    isolation_number: u32,
) -> Result<GeneratedPaintOrderDecision> {
    let decision = select_generated_reflow_source(reader, contents, sources, policy)?;
    anchor_generated_reflow(
        reader,
        contents,
        updates,
        (
            decision.anchor_stream_object,
            decision.anchor_stream_generation,
            decision.anchor_decoded_byte_offset,
        ),
        generated_number,
        isolation_number,
    )?;
    Ok(decision)
}

fn anchor_generated_reflow(
    reader: &crate::PdfReader,
    contents: &[(u32, u16)],
    updates: &mut Vec<IncrementalObject>,
    source: (u32, u16, usize),
    generated_number: u32,
    isolation_number: u32,
) -> Result<()> {
    use crate::content::{concat_matrix, IDENTITY_MATRIX};
    if contents
        .iter()
        .filter(|&&(n, g)| (n, g) == (source.0, source.1))
        .count()
        != 1
    {
        return Err(WellfriendError::UnsupportedFeature(
            "source paint anchor needs one page content occurrence".into(),
        ));
    }
    let decode = |object: &PdfObject| -> Result<Vec<u8>> {
        let decoded = decode_stream_lossless_with_limits(
            object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            return Err(WellfriendError::UnsupportedFeature(
                "opaque paint-anchor stream".into(),
            ));
        }
        Ok(decoded.data)
    };
    let mut matrix = IDENTITY_MATRIX;
    let mut stack = Vec::new();
    let mut marked_depth = 0usize;
    let mut armed = false;
    let mut anchor = None;
    'streams: for &(number, generation) in contents {
        let data = decode(&reader.get_object(number, generation)?)?;
        let mut operands = Vec::new();
        let mut et_ordinal = 0;
        for token in lex_content(&data)? {
            if (number, generation) == (source.0, source.1) && token.start == source.2 {
                armed = true;
            }
            let LexicalKind::Word(operator) = &token.kind else {
                operands.push(token);
                continue;
            };
            match operator.as_str() {
                "BMC" | "BDC" => {
                    marked_depth = marked_depth.checked_add(1).ok_or_else(|| {
                        WellfriendError::ResourceLimit("marked-content nesting".into())
                    })?;
                }
                "EMC" => {
                    marked_depth = marked_depth.checked_sub(1).ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "unbalanced source marked-content scope".into(),
                        )
                    })?;
                }
                "q" => {
                    if stack.len() >= 4096 {
                        return Err(WellfriendError::ResourceLimit("graphics nesting".into()));
                    }
                    stack.push(matrix);
                }
                "Q" => {
                    matrix = stack.pop().ok_or_else(|| {
                        WellfriendError::MalformedPdf("unbalanced source graphics state".into())
                    })?
                }
                "cm" => {
                    let numbers = operands
                        .iter()
                        .filter_map(|v| match v.kind {
                            LexicalKind::Number(n) => Some(n),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    let value: [f64; 6] = numbers.try_into().map_err(|_| {
                        WellfriendError::MalformedPdf("invalid source matrix".into())
                    })?;
                    matrix = concat_matrix(&value, &matrix);
                }
                "ET" => {
                    if armed {
                        anchor = Some((number, generation, et_ordinal, matrix, marked_depth));
                        break 'streams;
                    }
                    et_ordinal += 1;
                }
                _ => {}
            }
            operands.clear();
        }
    }
    let (number, generation, ordinal, m, source_marked_depth) = anchor.ok_or_else(|| {
        WellfriendError::MalformedPdf("source paint anchor has no closing ET".into())
    })?;
    let determinant = m[0] * m[3] - m[1] * m[2];
    if !m.iter().all(|v| v.is_finite()) || determinant.abs() < EPSILON {
        return Err(WellfriendError::UnsupportedFeature(
            "singular source paint transform".into(),
        ));
    }
    let inverse = [
        m[3] / determinant,
        -m[1] / determinant,
        -m[2] / determinant,
        m[0] / determinant,
        (m[2] * m[5] - m[3] * m[4]) / determinant,
        (m[1] * m[4] - m[0] * m[5]) / determinant,
    ];
    let generated = updates
        .iter()
        .find(|o| o.number == generated_number)
        .ok_or_else(|| WellfriendError::MalformedPdf("missing generated paint stream".into()))?;
    let generated = decode(&generated.object)?;
    let generated = std::str::from_utf8(&generated)
        .map_err(|_| WellfriendError::MalformedPdf("generated stream is not text".into()))?;
    let generated = generated.strip_prefix("Q\n").ok_or_else(|| {
        WellfriendError::MalformedPdf("generated isolation prefix is missing".into())
    })?;
    // Reset text parameters, not clip/transparency/paint: those belong to the
    // source occurrence. q/Q preserves the state for following original content.
    let generated = generated.replace("BT\n", "BT\n0 Tc 0 Tw 100 Tz 0 Ts 0 Tr\n");
    let insertion = format!(
        "\nq\n{} cm\n{}\nQ\n",
        inverse
            .iter()
            .map(|v| fmt_num(*v))
            .collect::<Vec<_>>()
            .join(" "),
        generated
    );
    let object = if let Some(update) = updates
        .iter()
        .find(|o| (o.number, o.generation) == (number, generation))
    {
        update.object.clone()
    } else {
        reader.get_object(number, generation)?
    };
    let mut data = decode(&object)?;
    let tokens = lex_content(&data)?;
    let et_index = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| matches!(&token.kind, LexicalKind::Word(op) if op == "ET"))
        .nth(ordinal)
        .map(|(index, _)| index)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("rewritten source lost paint anchor".into())
        })?;
    let mut offset = tokens[et_index].end;
    if source_marked_depth != 0 {
        // Inserting inside a source /ActualText or structural wrapper makes
        // that old logical owner consume the replacement. Move past only the
        // immediately enclosing EMC operators; crossing any painting or state
        // operator would change stacking order and therefore fails closed.
        let mut remaining = source_marked_depth;
        for token in &tokens[et_index + 1..] {
            let LexicalKind::Word(operator) = &token.kind else {
                continue;
            };
            if operator != "EMC" {
                return Err(WellfriendError::UnsupportedFeature(
                    "generated reflow cannot leave a source marked-content scope across intervening operators"
                        .into(),
                ));
            }
            remaining -= 1;
            offset = token.end;
            if remaining == 0 {
                break;
            }
        }
        if remaining != 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "generated reflow source marked-content scope closes in another content stream"
                    .into(),
            ));
        }
    }
    data.splice(offset..offset, insertion.bytes());
    let mut dict = object
        .as_stream()
        .map(|(dict, _)| dict.clone())
        .ok_or_else(|| WellfriendError::MalformedPdf("paint anchor is not a stream".into()))?;
    let raw = flate_encode_cancellable(&data, 6)?;
    dict.insert("Filter", PdfObject::Name("FlateDecode".into()));
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(raw.len() as i64));
    updates.retain(|o| {
        o.number != generated_number
            && o.number != isolation_number
            && (o.number, o.generation) != (number, generation)
    });
    updates.push(IncrementalObject {
        number,
        generation,
        object: PdfObject::Stream { dict, raw },
    });
    Ok(())
}

fn original_page_contents(contents: &[(u32, u16)]) -> Vec<PdfObject> {
    contents
        .iter()
        .map(|&(number, generation)| PdfObject::Reference { number, generation })
        .collect()
}

/// A final line from the linked-story paginator. Font bytes are resolved once
/// before pagination; this writer uses exactly those bytes for shaping/embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StoryDecoration {
    Fill {
        rect: [f64; 4],
        rgb: [f64; 3],
    },
    Stroke {
        from: [f64; 2],
        to: [f64; 2],
        rgb: [f64; 3],
        width: f64,
    },
}
fn serialize_story_decorations(decorations: &[StoryDecoration]) -> Result<String> {
    if decorations.len() > 100_000 {
        return Err(WellfriendError::ResourceLimit(
            "story decoration budget".into(),
        ));
    }
    let mut output = String::new();
    if !decorations.is_empty() {
        output.push_str("/Artifact BMC\nq\n0 J 0 j [] 0 d\n");
    }
    for item in decorations {
        crate::cancel::check_current_cancel("story grid painting")?;
        let rgb = match item {
            StoryDecoration::Fill { rgb, .. } | StoryDecoration::Stroke { rgb, .. } => rgb,
        };
        if rgb
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(WellfriendError::invalid_input(
                "invalid story decoration colour",
            ));
        }
        match item {
            StoryDecoration::Fill { rect, rgb } => {
                if rect.iter().any(|v| !v.is_finite()) || rect[0] >= rect[2] || rect[1] >= rect[3] {
                    return Err(WellfriendError::invalid_input(
                        "invalid table fill rectangle",
                    ));
                }
                output.push_str(&format!(
                    "{} {} {} rg {} {} {} {} re f\n",
                    fmt_num(rgb[0]),
                    fmt_num(rgb[1]),
                    fmt_num(rgb[2]),
                    fmt_num(rect[0]),
                    fmt_num(rect[1]),
                    fmt_num(rect[2] - rect[0]),
                    fmt_num(rect[3] - rect[1])
                ));
            }
            StoryDecoration::Stroke {
                from,
                to,
                rgb,
                width,
            } => {
                if from.iter().chain(to).any(|v| !v.is_finite())
                    || !width.is_finite()
                    || *width <= 0.0
                    || *width > 100.0
                {
                    return Err(WellfriendError::invalid_input("invalid table stroke"));
                }
                output.push_str(&format!(
                    "{} {} {} RG {} w {} {} m {} {} l S\n",
                    fmt_num(rgb[0]),
                    fmt_num(rgb[1]),
                    fmt_num(rgb[2]),
                    fmt_num(*width),
                    fmt_num(from[0]),
                    fmt_num(from[1]),
                    fmt_num(to[0]),
                    fmt_num(to[1])
                ));
            }
        }
    }
    if !decorations.is_empty() {
        output.push_str("Q\nEMC\n");
    }
    Ok(output)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryPaintStyleSpan {
    /// UTF-8 byte range relative to `StoryPaintLine::text` after any trailing
    /// hard-break carrier is removed. Spans form one exact contiguous
    /// partition whenever this vector is non-empty.
    pub range: [usize; 2],
    pub font_size: f64,
    pub rgb: [f64; 3],
    #[serde(default)]
    pub shaping: crate::fonts::shaper::OpenTypeSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryTabSegment {
    /// UTF-8 range relative to the visible `StoryPaintLine::text`, excluding
    /// the separating U+0009. Empty fields are retained for stop progression.
    pub range: [usize; 2],
    /// Inline-axis origin in PDF points relative to the line origin.
    pub origin: f64,
    pub width: f64,
    /// Shift from the field's occupied-box origin to its emitted text origin.
    pub leading_pad: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryPaintLine {
    /// Controls glyph orientation and inline progression, not page rotation.
    #[serde(default)]
    pub writing_mode: crate::fonts::WritingMode,
    pub text: String,
    /// Physical PDF origin after logical-axis pagination.
    pub x: f64,
    pub baseline: f64,
    /// Available inline extent: horizontal width or vertical column height.
    pub width: f64,
    pub font_size: f64,
    pub font_index: usize,
    #[serde(default)]
    pub font_spans: Vec<crate::fonts::fallback::FontSpan>,
    /// Effective inline styles for this line. Empty retains the legacy
    /// paragraph-uniform fields above and below. A non-empty vector is a
    /// complete line-relative partition and is intersected with `font_spans`
    /// before the single bidi ordering pass used by both measurement and paint.
    #[serde(default)]
    pub style_spans: Vec<StoryPaintStyleSpan>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tab_segments: Vec<StoryTabSegment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tab_decorations: Vec<crate::fonts::tab_stops::PositionedTabDecoration>,
    pub rgb: [f64; 3],
    pub rtl: bool,
    #[serde(default)]
    pub bidi: Option<crate::fonts::shaper::LineBidi>,
    #[serde(default)]
    pub shaping: crate::fonts::shaper::OpenTypeSettings,
    /// Transaction-local logical owner key; the tag writer assigns the actual
    /// page MCID after canonical page insertion has finalized object identities.
    #[serde(default)]
    pub tag_owner: Option<String>,
    #[serde(default)]
    pub artifact: bool,
}

pub(crate) fn story_paint_model_sha256(
    lines: &[StoryPaintLine],
    decorations: &[StoryDecoration],
) -> Result<String> {
    let model = serde_json::to_vec(&(lines, decorations)).map_err(|error| {
        WellfriendError::MalformedPdf(format!("story paint model serialization failed: {error}"))
    })?;
    Ok(format!("{:x}", Sha256::digest(model)))
}

fn story_shape_model_sha256(
    lines: &[StoryPaintLine],
    painted_lines: &StoryPaintedLines,
    fonts: &[crate::editing_transactions::ApprovedFontAsset],
) -> Result<String> {
    fn length(hasher: &mut Sha256, value: usize) {
        hasher.update((value as u64).to_be_bytes());
    }
    fn bytes(hasher: &mut Sha256, value: &[u8]) {
        length(hasher, value.len());
        hasher.update(value);
    }
    fn number(hasher: &mut Sha256, value: f64) -> Result<()> {
        if !value.is_finite() {
            return Err(WellfriendError::MalformedPdf(
                "non-finite value in story shape receipt".into(),
            ));
        }
        hasher.update(value.to_bits().to_be_bytes());
        Ok(())
    }

    if lines.len() != painted_lines.len() {
        return Err(WellfriendError::MalformedPdf(
            "story shape receipt line count mismatch".into(),
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"wellfriend-story-shape-v1\0");
    length(&mut hasher, lines.len());
    for (line, runs) in lines.iter().zip(painted_lines) {
        hasher.update([match line.writing_mode {
            crate::fonts::WritingMode::HorizontalTb => 0,
            crate::fonts::WritingMode::VerticalRl => 1,
            crate::fonts::WritingMode::VerticalLr => 2,
        }]);
        bytes(&mut hasher, line.text.as_bytes());
        if story_carriers::needs_carrier(line) {
            if !runs.is_empty() {
                return Err(WellfriendError::MalformedPdf(
                    "logical story carrier unexpectedly has shaped paint runs".into(),
                ));
            }
            hasher.update([1]);
            let carrier_font = get_fallback_font("Symbol").ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "bundled logical carrier font unavailable".into(),
                )
            })?;
            hasher.update(Sha256::digest(carrier_font));
            let encoded = crate::fonts::logical_carrier::encode(&line.text)?;
            bytes(&mut hasher, encoded.as_bytes());
        } else {
            hasher.update([0]);
        }
        length(&mut hasher, runs.len());
        for run in runs {
            let font = fonts.get(run.font_index).ok_or_else(|| {
                WellfriendError::MalformedPdf("story shape receipt refers to a missing font".into())
            })?;
            length(&mut hasher, run.font_index);
            length(&mut hasher, run.style_index);
            match run.inline_origin {
                Some(origin) => {
                    hasher.update([1]);
                    number(&mut hasher, origin)?;
                }
                None => hasher.update([0]),
            }
            match run.tab_field {
                Some(field) => {
                    hasher.update([1]);
                    length(&mut hasher, field);
                }
                None => hasher.update([0]),
            }
            bytes(&mut hasher, font.lookup_name.as_bytes());
            hasher.update(Sha256::digest(&font.bytes));
            length(&mut hasher, run.glyphs.len());
            for glyph in &run.glyphs {
                hasher.update(glyph.cid.to_be_bytes());
                hasher.update(glyph.gid.to_be_bytes());
                length(&mut hasher, glyph.logical_byte_start);
                bytes(&mut hasher, glyph.visual_unicode.as_bytes());
                match &glyph.to_unicode {
                    Some(value) => {
                        hasher.update([1]);
                        bytes(&mut hasher, value.as_bytes());
                    }
                    None => hasher.update([0]),
                }
                number(&mut hasher, glyph.advance)?;
                number(&mut hasher, glyph.offset_x)?;
                number(&mut hasher, glyph.offset_y)?;
                hasher.update([match glyph.orientation {
                    VerticalGlyphOrientation::Upright => 0,
                    VerticalGlyphOrientation::RotateClockwise => 1,
                    VerticalGlyphOrientation::FontVerticalAlternate => 2,
                }]);
                number(&mut hasher, glyph.cross_advance)?;
                number(&mut hasher, glyph.font_width)?;
                match glyph.bounds {
                    Some(bounds) => {
                        hasher.update([1]);
                        for value in bounds {
                            number(&mut hasher, value)?;
                        }
                    }
                    None => hasher.update([0]),
                }
            }
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn story_paint_styles(
    line: &StoryPaintLine,
    visible_len: usize,
) -> Result<Vec<StoryPaintStyleSpan>> {
    if line.style_spans.is_empty() {
        return Ok((visible_len > 0)
            .then_some(StoryPaintStyleSpan {
                range: [0, visible_len],
                font_size: line.font_size,
                rgb: line.rgb,
                shaping: line.shaping.clone(),
            })
            .into_iter()
            .collect());
    }
    if visible_len == 0 || line.style_spans.len() > 100_000 {
        return Err(WellfriendError::invalid_input(
            "invalid story paint-style partition",
        ));
    }
    let mut cursor = 0usize;
    for span in &line.style_spans {
        if span.range[0] != cursor
            || span.range[0] >= span.range[1]
            || span.range[1] > visible_len
            || !line.text.is_char_boundary(span.range[0])
            || !line.text.is_char_boundary(span.range[1])
            || !span.font_size.is_finite()
            || span.font_size <= 0.0
            || span.font_size > 10_000.0
            || span
                .rgb
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(WellfriendError::invalid_input(
                "invalid story paint-style partition",
            ));
        }
        cursor = span.range[1];
    }
    if cursor != visible_len {
        return Err(WellfriendError::invalid_input(
            "story paint styles do not cover the visible line",
        ));
    }
    Ok(line.style_spans.clone())
}

fn slice_story_paint_styles(
    styles: &[StoryPaintStyleSpan],
    range: std::ops::Range<usize>,
) -> Vec<StoryPaintStyleSpan> {
    styles
        .iter()
        .filter_map(|style| {
            let start = style.range[0].max(range.start);
            let end = style.range[1].min(range.end);
            (start < end).then_some(StoryPaintStyleSpan {
                range: [start - range.start, end - range.start],
                font_size: style.font_size,
                rgb: style.rgb,
                shaping: style.shaping.clone(),
            })
        })
        .collect()
}

fn validate_story_tab_segments(line: &StoryPaintLine, visible: &str) -> Result<()> {
    use crate::fonts::tab_stops::{PositionedTabDecoration, TabLeader};
    if line.tab_decorations.len() > 8192
        || (!line.tab_decorations.is_empty() && !visible.contains('\t'))
    {
        return Err(WellfriendError::invalid_input(
            "invalid story tab-decoration plan",
        ));
    }
    for decoration in &line.tab_decorations {
        let valid = match *decoration {
            PositionedTabDecoration::Leader { from, to, leader } => {
                leader != TabLeader::None
                    && from.is_finite()
                    && to.is_finite()
                    && from >= 0.0
                    && from < to
                    && to <= line.width + 1e-7
            }
            PositionedTabDecoration::Bar { position } => {
                position.is_finite() && position >= 0.0 && position <= line.width + 1e-7
            }
        };
        if !valid {
            return Err(WellfriendError::invalid_input(
                "invalid story tab-decoration geometry",
            ));
        }
    }
    if line.tab_segments.is_empty() {
        if visible.contains('\t') && !crate::fonts::logical_carrier::is_text(visible) {
            return Err(WellfriendError::invalid_input(
                "story line contains U+0009 without an exact tab-field plan",
            ));
        }
        return Ok(());
    }
    let mut cursor = 0usize;
    let mut previous_end = 0.0f64;
    for (index, segment) in line.tab_segments.iter().enumerate() {
        let [start, end] = segment.range;
        if start != cursor
            || start > end
            || end > visible.len()
            || !visible.is_char_boundary(start)
            || !visible.is_char_boundary(end)
            || !segment.origin.is_finite()
            || !segment.width.is_finite()
            || !segment.leading_pad.is_finite()
            || segment.origin < -1e-7
            || segment.width < 0.0
            || segment.leading_pad < 0.0
            || segment.leading_pad > segment.width + 1e-7
            || segment.origin + 1e-7 < previous_end
        {
            return Err(WellfriendError::invalid_input(
                "invalid or overlapping story tab-field plan",
            ));
        }
        if index + 1 < line.tab_segments.len() {
            if visible.as_bytes().get(end) != Some(&b'\t') {
                return Err(WellfriendError::invalid_input(
                    "story tab-field plan does not bind its U+0009 separator",
                ));
            }
            cursor = end + 1;
        } else {
            cursor = end;
        }
        previous_end = segment.origin + segment.width;
    }
    if cursor != visible.len() || !visible.contains('\t') || previous_end > line.width + 1e-7 {
        return Err(WellfriendError::invalid_input(
            "story tab-field plan does not cover the visible line or exceeds its extent",
        ));
    }
    Ok(())
}

fn serialize_story_tab_decorations(line: &StoryPaintLine) -> Result<String> {
    use crate::fonts::tab_stops::{PositionedTabDecoration, TabLeader};
    if line.tab_decorations.is_empty() {
        return Ok(String::new());
    }
    let stroke = (line.font_size / 18.0).clamp(0.35, 1.5);
    let mut output = format!(
        "/Artifact BMC\nq\n{} {} {} RG\n{} w\n",
        fmt_num(line.rgb[0]),
        fmt_num(line.rgb[1]),
        fmt_num(line.rgb[2]),
        fmt_num(stroke)
    );
    for decoration in &line.tab_decorations {
        crate::cancel::check_current_cancel("story tab-decoration serialization")?;
        match *decoration {
            PositionedTabDecoration::Leader { from, to, leader } => {
                let inset = stroke.max(line.font_size * 0.08);
                if to - from <= inset * 2.0 {
                    continue;
                }
                match leader {
                    TabLeader::Dots => output.push_str(&format!(
                        "1 J [{} {}] 0 d\n",
                        fmt_num(stroke * 0.01),
                        fmt_num(line.font_size * 0.30)
                    )),
                    TabLeader::Dashes => output.push_str(&format!(
                        "0 J [{} {}] 0 d\n",
                        fmt_num(line.font_size * 0.34),
                        fmt_num(line.font_size * 0.22)
                    )),
                    TabLeader::Solid => output.push_str("0 J [] 0 d\n"),
                    TabLeader::None => {
                        return Err(WellfriendError::invalid_input(
                            "empty story tab leader decoration",
                        ));
                    }
                }
                if line.writing_mode.is_vertical() {
                    let x = line.x + line.font_size * 0.08;
                    output.push_str(&format!(
                        "{} {} m {} {} l S\n",
                        fmt_num(x),
                        fmt_num(line.baseline - from - inset),
                        fmt_num(x),
                        fmt_num(line.baseline - to + inset)
                    ));
                } else {
                    let y = line.baseline + line.font_size * 0.08;
                    output.push_str(&format!(
                        "{} {} m {} {} l S\n",
                        fmt_num(line.x + from + inset),
                        fmt_num(y),
                        fmt_num(line.x + to - inset),
                        fmt_num(y)
                    ));
                }
            }
            PositionedTabDecoration::Bar { position } => {
                output.push_str("0 J [] 0 d\n");
                if line.writing_mode.is_vertical() {
                    let y = line.baseline - position;
                    output.push_str(&format!(
                        "{} {} m {} {} l S\n",
                        fmt_num(line.x - line.font_size * 0.25),
                        fmt_num(y),
                        fmt_num(line.x + line.font_size * 0.75),
                        fmt_num(y)
                    ));
                } else {
                    let x = line.x + position;
                    output.push_str(&format!(
                        "{} {} m {} {} l S\n",
                        fmt_num(x),
                        fmt_num(line.baseline - line.font_size * 0.25),
                        fmt_num(x),
                        fmt_num(line.baseline + line.font_size * 0.75)
                    ));
                }
            }
        }
    }
    output.push_str("Q\nEMC\n");
    Ok(output)
}

fn shape_story_field(
    line: &StoryPaintLine,
    visual: &str,
    fonts: &[crate::editing_transactions::ApprovedFontAsset],
    font_metrics: &[Option<crate::fonts::line_layout::PreparedFontMetrics<'_>>],
    region: [f64; 4],
) -> Result<Vec<(usize, usize, Vec<GeneratedGlyph>)>> {
    if visual.contains('\t') || !line.tab_segments.is_empty() {
        return Err(WellfriendError::invalid_input(
            "story field shaping received unresolved tab layout",
        ));
    }
    let bidi = if let Some(bidi) = &line.bidi {
        bidi.clone()
    } else {
        crate::fonts::shaper::resolve_line_bidi(
            visual,
            0..visual.len(),
            ShapeOptions {
                direction: Some(if line.rtl {
                    TextDirection::RightToLeft
                } else {
                    TextDirection::LeftToRight
                }),
            },
        )?
    };
    let spans = if line.font_spans.is_empty() && !visual.is_empty() {
        vec![crate::fonts::fallback::FontSpan {
            range: [0, visual.len()],
            font_index: line.font_index,
        }]
    } else {
        crate::fonts::fallback::slice_spans(&line.font_spans, 0..visual.len())
    };
    let styles = story_paint_styles(line, visual.len())?;
    let style_ranges = styles.iter().map(|style| style.range).collect::<Vec<_>>();
    if line.writing_mode.is_vertical() {
        if line.style_spans.is_empty() {
            return Ok(story_vertical::shape_line(
                line,
                visual,
                &bidi,
                &spans,
                fonts,
                font_metrics,
                region,
            )?
            .into_iter()
            .map(|(font_index, glyphs)| (font_index, 0usize, glyphs))
            .collect());
        }
        let styled_spans =
            crate::fonts::fallback::intersect_styled_spans(&spans, &style_ranges, visual.len())?;
        return story_vertical::shape_styled_line(
            line,
            visual,
            &bidi,
            &styled_spans,
            &styles,
            fonts,
            font_metrics,
            region,
        );
    }
    if !line.style_spans.is_empty() {
        let styled_spans =
            crate::fonts::fallback::intersect_styled_spans(&spans, &style_ranges, visual.len())?;
        let settings = styles
            .iter()
            .map(|style| style.shaping.clone())
            .collect::<Vec<_>>();
        let runs = crate::fonts::fallback::shape_styled_line(
            visual,
            &bidi,
            &styled_spans,
            fonts,
            &settings,
        )?;
        let width = runs
            .iter()
            .map(|run| {
                run.shaped
                    .glyphs
                    .iter()
                    .map(|glyph| glyph.advance.abs())
                    .sum::<f64>()
                    * styles[run.style_index].font_size
                    / 1000.0
            })
            .sum::<f64>();
        if width > line.width + 1e-7 {
            return Err(WellfriendError::MalformedPdf(
                "styled story measurement/serialization width mismatch".into(),
            ));
        }
        return runs
            .into_iter()
            .map(|run| {
                Ok((
                    run.font_index,
                    run.style_index,
                    generated_glyphs_from_shaped(
                        &visual[run.range],
                        &fonts[run.font_index].bytes,
                        run.shaped,
                    )?,
                ))
            })
            .collect();
    }
    let runs = crate::fonts::fallback::shape_line(visual, &bidi, &spans, fonts, &line.shaping)?;
    let width = crate::fonts::fallback::measure_line(&runs, fonts, line.font_size)?.advance;
    if width > line.width + 1e-7 {
        return Err(WellfriendError::MalformedPdf(
            "story measurement/serialization width mismatch".into(),
        ));
    }
    runs.into_iter()
        .map(|run| {
            Ok((
                run.font_index,
                0usize,
                generated_glyphs_from_shaped(
                    &visual[run.range],
                    &fonts[run.font_index].bytes,
                    run.shaped,
                )?,
            ))
        })
        .collect()
}

/// Persistent frame ownership is a marked-content range, not a word search or
/// an object number (the canonical writer may renumber objects). The digest is
/// checked before every mutation, including an empty frame's next insertion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoryFrameBinding {
    pub key: String,
    pub content_sha256: String,
    /// Canonical transaction model stamped into the owned marked-content
    /// dictionary. Older uniform frames omit it; every newly written frame
    /// binds its exact lines, style/font partitions, bidi context and decorations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint_sha256: Option<String>,
    /// Exact shaped-run receipt produced by the writer. It binds the approved
    /// font programs and every emitted glyph id, CID, mapping, advance,
    /// offset, orientation and outline metric before publication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_sha256: Option<String>,
}

pub(crate) struct StoryFrameWrite {
    pub bytes: Vec<u8>,
    pub binding: StoryFrameBinding,
}

struct StoryOwnedRange {
    stream: (u32, u16),
    range: std::ops::Range<usize>,
    binding: StoryFrameBinding,
}

fn marked_sha256_property(
    operands: &[LexicalToken],
    key: &str,
    label: &str,
) -> Result<Option<String>> {
    match marked_property(operands, key)?.map(|value| &value.kind) {
        Some(LexicalKind::String(_, bytes)) => {
            let digest = std::str::from_utf8(bytes)
                .map_err(|_| WellfriendError::invalid_input(format!("invalid {label} digest")))?;
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(WellfriendError::invalid_input(format!(
                    "invalid {label} digest"
                )));
            }
            Ok(Some(digest.to_ascii_lowercase()))
        }
        None => Ok(None),
        _ => Err(WellfriendError::invalid_input(format!(
            "{label} digest is not a string"
        ))),
    }
}

fn story_owned_ranges(
    reader: &crate::PdfReader,
    contents: &[(u32, u16)],
) -> Result<Vec<StoryOwnedRange>> {
    let mut found = Vec::new();
    let mut keys = BTreeSet::new();
    let mut owners: Vec<Option<((u32, u16), usize, String, Option<String>, Option<String>)>> =
        Vec::new();
    let mut owner_origins: Vec<((u32, u16), usize, String)> = Vec::new();
    for &(number, generation) in contents {
        crate::cancel::check_current_cancel("story ownership binding")?;
        let object = reader.get_object(number, generation)?;
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            return Err(WellfriendError::UnsupportedFeature(
                "opaque story owner stream".into(),
            ));
        }
        let mut operands = Vec::new();
        for token in lex_content(&decoded.data)? {
            let LexicalKind::Word(operator) = &token.kind else {
                operands.push(token);
                continue;
            };
            match operator.as_str() {
                "BDC" | "BMC" => {
                    if owners.len() >= 256 {
                        return Err(WellfriendError::ResourceLimit(
                            "story marked-content nesting".into(),
                        ));
                    }
                    let property = if operator == "BDC" {
                        marked_property(&operands, "WFStoryFrame")?
                    } else {
                        None
                    };
                    let paint = if operator == "BDC" {
                        marked_sha256_property(&operands, "WFStoryPaint", "story paint")?
                    } else {
                        None
                    };
                    let shape = if operator == "BDC" {
                        marked_sha256_property(&operands, "WFStoryShape", "story shape")?
                    } else {
                        None
                    };
                    let owner = match property.map(|v| &v.kind) {
                        Some(LexicalKind::String(_, bytes)) => {
                            let key = std::str::from_utf8(bytes)
                                .map_err(|_| WellfriendError::invalid_input("invalid story key"))?;
                            if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
                                return Err(WellfriendError::invalid_input(
                                    "invalid story owner key",
                                ));
                            }
                            Some((
                                (number, generation),
                                operands.first().map_or(token.start, |t| t.start),
                                key.to_owned(),
                                paint,
                                shape,
                            ))
                        }
                        None => None,
                        _ => {
                            return Err(WellfriendError::invalid_input(
                                "story owner key is not a string",
                            ))
                        }
                    };
                    owners.push(owner);
                    owner_origins.push(((number, generation), token.start, operator.clone()));
                }
                "EMC" => {
                    let owner = owners.pop().ok_or_else(|| {
                        WellfriendError::MalformedPdf("unbalanced story marked content".into())
                    })?;
                    owner_origins.pop().ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "story marked-content provenance stack underflow".into(),
                        )
                    })?;
                    if let Some((stream, start, key, paint_sha256, shape_sha256)) = owner {
                        if stream != (number, generation) {
                            return Err(WellfriendError::UnsupportedFeature(
                                "story owner crosses a stream boundary".into(),
                            ));
                        }
                        if !keys.insert(key.clone()) {
                            return Err(WellfriendError::UnsupportedFeature(
                                "ambiguous duplicate story frame owner".into(),
                            ));
                        }
                        let range = start..token.end;
                        found.push(StoryOwnedRange {
                            stream,
                            range: range.clone(),
                            binding: StoryFrameBinding {
                                key,
                                content_sha256: format!(
                                    "{:x}",
                                    Sha256::digest(&decoded.data[range])
                                ),
                                paint_sha256,
                                shape_sha256,
                            },
                        });
                    }
                }
                _ => {}
            }
            operands.clear();
        }
    }
    if !owners.is_empty() {
        let story_streams = owners
            .iter()
            .filter_map(|owner| owner.as_ref().map(|(stream, _, _, _, _)| *stream))
            .collect::<BTreeSet<_>>();
        let origins = owner_origins
            .iter()
            .map(|(stream, offset, operator)| {
                format!("{} {}@{offset}:{operator}", stream.0, stream.1)
            })
            .collect::<Vec<_>>()
            .join(",");
        return Err(WellfriendError::MalformedPdf(
            format!(
                "unterminated marked content after page streams: depth={}, story_depth={}, story_streams={story_streams:?}, origins=[{origins}]",
                owners.len(),
                owners.iter().filter(|owner| owner.is_some()).count(),
            ),
        ));
    }
    Ok(found)
}

fn locate_story_frame(
    reader: &crate::PdfReader,
    contents: &[(u32, u16)],
    key: &str,
) -> Result<Option<StoryOwnedRange>> {
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(WellfriendError::invalid_input("invalid story owner key"));
    }
    Ok(story_owned_ranges(reader, contents)?
        .into_iter()
        .find(|r| r.binding.key == key))
}

pub(crate) fn story_frame_inventory(
    engine: &ContentEngine,
) -> Result<BTreeMap<String, (usize, StoryFrameBinding)>> {
    let mut owners = BTreeMap::new();
    for page in engine.document().get_pages()? {
        for owner in story_owned_ranges(engine.document().reader(), &page.contents)? {
            if owners.len() >= 4096 * 64 {
                return Err(WellfriendError::ResourceLimit(
                    "story frame inventory limit".into(),
                ));
            }
            if owners
                .insert(owner.binding.key.clone(), (page.page_number, owner.binding))
                .is_some()
            {
                return Err(WellfriendError::UnsupportedFeature(
                    "story frame occurs on multiple pages".into(),
                ));
            }
        }
    }
    Ok(owners)
}

pub fn bind_story_frame(
    input: &[u8],
    page_number: usize,
    key: &str,
) -> Result<Option<StoryFrameBinding>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    bind_story_frame_in_document(&engine, page_number, key)
}

pub(crate) fn bind_story_frame_in_document(
    engine: &ContentEngine,
    page_number: usize,
    key: &str,
) -> Result<Option<StoryFrameBinding>> {
    let page = engine.document().get_page(page_number)?;
    Ok(locate_story_frame(engine.document().reader(), &page.contents, key)?.map(|r| r.binding))
}

pub(crate) fn replace_story_frame(
    input: &[u8],
    page_number: usize,
    range: [usize; 2],
    region: [f64; 4],
    lines: &[StoryPaintLine],
    decorations: &[StoryDecoration],
    fonts: &[crate::editing_transactions::ApprovedFontAsset],
    signature_override: bool,
    owner_key: &str,
    expected_owner: Option<&StoryFrameBinding>,
) -> Result<StoryFrameWrite> {
    let original = ContentEngine::open_bytes(input.to_vec())?;
    let page = original.document().get_page(page_number)?;
    let owned = locate_story_frame(original.document().reader(), &page.contents, owner_key)?;
    if expected_owner != owned.as_ref().map(|r| &r.binding) {
        return Err(WellfriendError::invalid_input(
            "story owner changed or is not approved",
        ));
    }
    let source = if owned.is_none() {
        let model = analyze_multi_run_text_range(input, page_number)?;
        let anchor = model
            .source_spans
            .iter()
            .find(|s| s.logical_range[0] <= range[0] && s.logical_range[1] > range[0])
            .or_else(|| model.source_spans.last())
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature("story frame needs a source text anchor".into())
            })?;
        Some((
            anchor.stream_object,
            anchor.stream_generation,
            anchor.byte_range[0],
        ))
    } else {
        None
    };
    let options = AdvancedTextEditOptions {
        region,
        signature_policy_override: signature_override,
        ..AdvancedTextEditOptions::default()
    };
    let mut bytes = input.to_vec();
    if owned.is_none() && range[0] != range[1] {
        bytes = edit_multi_run_text_range(
            input,
            &MultiRunTextRangeRequest {
                page: page_number,
                logical_start: range[0],
                logical_end: range[1],
                replacement_text: String::new(),
                mode: AdvancedTextMode::ParagraphReflowHorizontal,
                style_policy: MultiRunStylePolicy::ExplicitSupplied,
                options: options.clone(),
                final_lines: None,
            },
            None,
        )?
        .0;
    }
    let edited = ContentEngine::open_bytes(bytes)?;
    let reader = edited.document().reader();
    let mut updates = Vec::new();
    // Use the original reader for ET identity and the current revision for the
    // rewritten bytes. This avoids all stale-token-offset batch comparisons.
    for &(number, generation) in &page.contents {
        updates.push(IncrementalObject {
            number,
            generation,
            object: reader.get_object(number, generation)?,
        });
    }
    let base = reserve_advanced_object_block(
        reader,
        u32::try_from(
            fonts
                .len()
                .saturating_mul(12)
                .saturating_add(2 + story_carriers::OBJECT_COUNT as usize),
        )
        .map_err(|_| WellfriendError::ResourceLimit("story font object budget".into()))?,
        "story frame",
    )?;
    let mut resources = page.resources.clone();
    let mut font_resources = resolve_advanced_editing_dict(resources.get("Font"), reader)
        .unwrap_or_else(crate::PdfDictionary::empty);
    let used_story_fonts = lines
        .iter()
        .flat_map(|line| {
            if line.font_spans.is_empty() {
                vec![line.font_index]
            } else {
                line.font_spans
                    .iter()
                    .map(|span| span.font_index)
                    .collect::<Vec<_>>()
            }
        })
        .collect::<BTreeSet<_>>();
    let story_font_metrics = fonts
        .iter()
        .enumerate()
        .map(|(index, font)| {
            if !used_story_fonts.contains(&index) {
                return Ok(None);
            }
            crate::fonts::line_layout::PreparedFontMetrics::new(&font.bytes).map(Some)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut glyphs_by_font = BTreeMap::<(usize, bool), Vec<GeneratedGlyph>>::new();
    let mut painted_lines: StoryPaintedLines = Vec::new();
    for line in lines {
        crate::cancel::check_current_cancel("story line shaping")?;
        let visual = line
            .text
            .trim_end_matches(crate::fonts::hard_break::is_hard_break);
        validate_story_tab_segments(line, visual)?;
        if story_carriers::needs_carrier(line) {
            // Pure logical controls do not depend on the selected paint font.
            // The private carrier has explicit empty outlines and zero metrics.
            painted_lines.push(Vec::new());
            continue;
        }
        let mut runs = Vec::new();
        if line.tab_segments.is_empty() {
            runs.extend(
                shape_story_field(line, visual, fonts, &story_font_metrics, region)?
                    .into_iter()
                    .map(|(font_index, style_index, glyphs)| StoryPaintedRun {
                        font_index,
                        style_index,
                        inline_origin: None,
                        tab_field: None,
                        glyphs,
                    }),
            );
        } else {
            let full_bidi = line.bidi.as_ref().ok_or_else(|| {
                WellfriendError::invalid_input("tabbed story line lost paragraph bidi context")
            })?;
            let full_styles = story_paint_styles(line, visual.len())?;
            for (field_index, segment) in line.tab_segments.iter().enumerate() {
                let range = segment.range[0]..segment.range[1];
                if range.is_empty() {
                    continue;
                }
                let mut field = line.clone();
                field.text = visual[range.clone()].to_owned();
                field.width = segment.width;
                field.font_spans =
                    crate::fonts::fallback::slice_spans(&line.font_spans, range.clone());
                field.style_spans = slice_story_paint_styles(&full_styles, range.clone());
                field.tab_segments.clear();
                field.bidi = Some(full_bidi.slice(visual, range, full_bidi.rtl)?);
                let field_styles = story_paint_styles(&field, field.text.len())?;
                for (font_index, style_index, mut glyphs) in
                    shape_story_field(&field, &field.text, fonts, &story_font_metrics, region)?
                {
                    for glyph in &mut glyphs {
                        glyph.logical_byte_start = glyph
                            .logical_byte_start
                            .checked_add(segment.range[0])
                            .ok_or_else(|| {
                                WellfriendError::ResourceLimit(
                                    "tab field glyph provenance overflow".into(),
                                )
                            })?;
                    }
                    let effective = field_styles.get(style_index).ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "tab field paint style index is out of bounds".into(),
                        )
                    })?;
                    let effective_start = segment.range[0]
                        .checked_add(effective.range[0])
                        .ok_or_else(|| {
                            WellfriendError::ResourceLimit(
                                "tab field paint-style provenance overflow".into(),
                            )
                        })?;
                    let effective_end = segment.range[0]
                        .checked_add(effective.range[1])
                        .ok_or_else(|| {
                            WellfriendError::ResourceLimit(
                                "tab field paint-style provenance overflow".into(),
                            )
                        })?;
                    let style_index = full_styles
                        .iter()
                        .position(|style| {
                            style.range[0] <= effective_start
                                && effective_end <= style.range[1]
                                && style.font_size == effective.font_size
                                && style.rgb == effective.rgb
                                && style.shaping == effective.shaping
                        })
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "tab field paint style is not part of its source line".into(),
                            )
                        })?;
                    runs.push(StoryPaintedRun {
                        font_index,
                        style_index,
                        inline_origin: Some(segment.origin + segment.leading_pad),
                        tab_field: Some(field_index),
                        glyphs,
                    });
                }
            }
        }
        let mut painted_runs = Vec::new();
        for mut run in runs {
            let font = &fonts[run.font_index];
            let _face = ttf_parser::Face::parse(&font.bytes, 0).map_err(|_| {
                WellfriendError::UnsupportedFeature("story font is not sfnt".into())
            })?;
            let all = glyphs_by_font
                .entry((run.font_index, line.writing_mode.is_vertical()))
                .or_default();
            for glyph in &mut run.glyphs {
                glyph.cid = u16::try_from(all.len() + 1).map_err(|_| {
                    WellfriendError::ResourceLimit(
                        "story frame exceeds 65535 glyph character codes per font".into(),
                    )
                })?;
                all.push(glyph.clone());
            }
            painted_runs.push(run);
        }
        painted_lines.push(painted_runs);
    }
    // Embed a font once per frame, not once per line. CIDs above share one
    // deterministic per-font namespace across every line in this frame.
    for (&(index, vertical), glyphs) in &glyphs_by_font {
        let object_base = base + index as u32 * 12 + u32::from(vertical) * 6;
        let resource = format!("WFStory{object_base}");
        if font_resources.contains_key(&resource) {
            return Err(WellfriendError::MalformedPdf(
                "story font resource collision".into(),
            ));
        }
        let mut font_updates = build_type0_font_objects(
            &fonts[index].bytes,
            glyphs,
            vertical,
            object_base,
            object_base + 1,
            object_base + 2,
            object_base + 3,
            object_base + 4,
            object_base + 5,
        )?;
        // Preserve the approved lookup alias across save/reopen. The embedded
        // face may be subsetted, so a hash of the original bytes is not an alias.
        for update in &mut font_updates {
            if let Some(dict) = update.object.as_dict_mut() {
                if dict.get_name("Subtype") == Some("Type0") {
                    dict.insert(
                        "WFStoryOwner",
                        PdfObject::String(owner_key.as_bytes().to_vec()),
                    );
                }
                for key in ["BaseFont", "FontName"] {
                    if let Some(name) = dict.get_name(key).map(str::to_owned) {
                        let tag = name.split_once('+').map_or("WFEDIT", |(tag, _)| tag);
                        dict.insert(
                            key,
                            PdfObject::Name(format!("{tag}+{}", fonts[index].lookup_name)),
                        );
                    }
                }
            }
        }
        updates.extend(font_updates);
        font_resources.insert(
            resource,
            PdfObject::Reference {
                number: object_base + 5,
                generation: 0,
            },
        );
    }
    let carrier_base = base + fonts.len() as u32 * 12;
    let carriers = story_carriers::CarrierFonts::prepare(
        lines,
        owner_key,
        carrier_base,
        &mut font_resources,
        &mut updates,
    )?;
    let paint_sha256 = story_paint_model_sha256(lines, decorations)?;
    let shape_sha256 = story_shape_model_sha256(lines, &painted_lines, fonts)?;
    let mut content = format!(
        "/Span << /WFStoryFrame <{}> /WFStoryPaint <{}> /WFStoryShape <{}> >> BDC\n",
        owner_key
            .bytes()
            .map(|b| format!("{b:02X}"))
            .collect::<String>(),
        paint_sha256
            .bytes()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>(),
        shape_sha256
            .bytes()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>()
    );
    content.push_str(&serialize_story_decorations(decorations)?);
    for (line, runs) in lines.iter().zip(painted_lines) {
        crate::cancel::check_current_cancel("story line serialization")?;
        content.push_str(&serialize_story_tab_decorations(line)?);
        let styles = story_paint_styles(
            line,
            line.text
                .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                .len(),
        )?;
        let natural = runs
            .iter()
            .map(|run| {
                run.glyphs.iter().map(|g| g.advance.abs()).sum::<f64>()
                    * styles
                        .get(run.style_index)
                        .map_or(line.font_size, |style| style.font_size)
                    / 1000.0
            })
            .sum::<f64>();
        let mut pen = line.x
            + if line.tab_segments.is_empty() && line.rtl {
                line.width - natural
            } else {
                0.0
            };
        let tag_marker = if let Some(key) = &line.tag_owner {
            if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(WellfriendError::invalid_input(
                    "invalid story paragraph tag owner",
                ));
            }
            format!(
                " /WFStoryParagraph <{}>",
                key.bytes().map(|b| format!("{b:02X}")).collect::<String>()
            )
        } else {
            String::new()
        };
        if line.artifact {
            content.push_str("/Artifact BMC\n");
        }
        content.push_str(&format!(
            "q\n{} {} {} rg\n/Span << /ActualText <{}>{tag_marker} >> BDC\n",
            fmt_num(line.rgb[0]),
            fmt_num(line.rgb[1]),
            fmt_num(line.rgb[2]),
            utf16be_hex_with_bom(&line.text)
        ));
        if story_carriers::needs_carrier(line) {
            if runs.iter().any(|run| !run.glyphs.is_empty()) {
                return Err(WellfriendError::MalformedPdf(
                    "logical-only story line unexpectedly shaped glyphs".into(),
                ));
            }
            carriers.append_line(&mut content, line)?;
        }
        let mut vertical_pen = [0.0, 0.0];
        let mut active_tab_field = None;
        for run in runs {
            if let Some(origin) = run.inline_origin {
                if active_tab_field != run.tab_field {
                    pen = line.x + origin;
                    vertical_pen = [0.0, origin];
                    active_tab_field = run.tab_field;
                }
            }
            let style = styles.get(run.style_index).ok_or_else(|| {
                WellfriendError::MalformedPdf("story paint style index is out of bounds".into())
            })?;
            let width =
                run.glyphs.iter().map(|g| g.advance.abs()).sum::<f64>() * style.font_size / 1000.0;
            let resource = format!(
                "WFStory{}",
                base + run.font_index as u32 * 12 + u32::from(line.writing_mode.is_vertical()) * 6
            );
            content.push_str(&format!(
                "{} {} {} rg\n",
                fmt_num(style.rgb[0]),
                fmt_num(style.rgb[1]),
                fmt_num(style.rgb[2])
            ));
            if line.writing_mode.is_vertical() {
                story_vertical::append_run(
                    &mut content,
                    line,
                    &run.glyphs,
                    &resource,
                    style.font_size,
                    &mut vertical_pen,
                )?;
                continue;
            }
            let line_options = AdvancedTextEditOptions {
                region: [
                    pen,
                    line.baseline,
                    pen + width,
                    line.baseline + style.font_size,
                ],
                font_size: style.font_size,
                max_lines_or_columns: 1,
                alignment: GeneratedTextAlignment::Left,
                ..options.clone()
            };
            let (paint, _) = serialize_generated_text(
                &[run.glyphs],
                &resource,
                &line_options,
                false,
                None,
                None,
            )?;
            content.push_str(&paint.replace("BT\n", "BT\n0 Tc 0 Tw 100 Tz 0 Ts 0 Tr\n"));
            if !content
                .as_bytes()
                .last()
                .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                content.push('\n');
            }
            pen += width;
        }
        content.push_str("EMC\nQ\n");
        if line.artifact {
            content.push_str("EMC\n");
        }
    }
    content.push_str("EMC\n");
    let generated_number = carrier_base + story_carriers::OBJECT_COUNT;
    let isolation_number = generated_number + 1;
    let raw = isolated_appended_content(content).into_bytes();
    let mut dict = crate::PdfDictionary::empty();
    dict.insert("Length", PdfObject::Integer(raw.len() as i64));
    updates.push(IncrementalObject {
        number: generated_number,
        generation: 0,
        object: PdfObject::Stream { dict, raw },
    });
    if let Some(owned) = owned {
        // Replace only the approved owner's marked span. Its surrounding q/cm/Q
        // isolation stays in place, preserving the original paint slot exactly.
        let object = reader.get_object(owned.stream.0, owned.stream.1)?;
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        let mut data = decoded.data;
        let generated = updates
            .iter()
            .find(|o| o.number == generated_number)
            .unwrap();
        let (_, raw) = generated.object.as_stream().unwrap();
        let replacement = std::str::from_utf8(raw)
            .map_err(|_| WellfriendError::MalformedPdf("story content encoding".into()))?
            .strip_prefix("Q\n")
            .ok_or_else(|| WellfriendError::MalformedPdf("story isolation prefix".into()))?
            .replace("BT\n", "BT\n0 Tc 0 Tw 100 Tz 0 Ts 0 Tr\n");
        data.splice(owned.range, replacement.bytes());
        let mut dict = object.as_stream().unwrap().0.clone();
        let raw = flate_encode_cancellable(&data, 6)?;
        dict.insert("Filter", PdfObject::Name("FlateDecode".into()));
        dict.remove("DecodeParms");
        dict.insert("Length", PdfObject::Integer(raw.len() as i64));
        updates
            .retain(|o| o.number != generated_number && (o.number, o.generation) != owned.stream);
        updates.push(IncrementalObject {
            number: owned.stream.0,
            generation: owned.stream.1,
            object: PdfObject::Stream { dict, raw },
        });
    } else {
        anchor_generated_reflow(
            original.document().reader(),
            &page.contents,
            &mut updates,
            source.expect("new frame source anchor"),
            generated_number,
            isolation_number,
        )?;
    }
    let object = reader.get_object(page.object_number, page.generation_number)?;
    let mut dict = object
        .as_dict()
        .cloned()
        .ok_or_else(|| WellfriendError::MalformedPdf("story page dictionary".into()))?;
    let used_fonts = story_reachable_font_names(reader, &page, &updates)?;
    let retired = font_resources
        .iter()
        .filter_map(|(name, value)| {
            if used_fonts.contains(name) {
                return None;
            }
            let object = match value {
                PdfObject::Reference { number, generation } => updates
                    .iter()
                    .find(|o| (o.number, o.generation) == (*number, *generation))
                    .map(|o| o.object.clone())
                    .or_else(|| reader.get_object(*number, *generation).ok()),
                value => Some(value.clone()),
            }?;
            let owner = object.as_dict()?.get("WFStoryOwner")?;
            matches!(owner, PdfObject::String(bytes) if bytes == owner_key.as_bytes())
                .then(|| name.clone())
        })
        .collect::<Vec<_>>();
    for name in retired {
        font_resources.remove(&name);
    }
    resources.insert("Font", PdfObject::Dictionary(font_resources));
    dict.insert("Resources", PdfObject::Dictionary(resources));
    updates.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(dict),
    });
    let output = write_incremental_update(reader, updates)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let binding = bind_story_frame_in_document(&reopened, page_number, owner_key)?
        .ok_or_else(|| WellfriendError::MalformedPdf("saved story owner is missing".into()))?;
    if binding.paint_sha256.as_deref() != Some(paint_sha256.as_str())
        || binding.shape_sha256.as_deref() != Some(shape_sha256.as_str())
    {
        return Err(WellfriendError::MalformedPdf(
            "saved story owner receipt mismatch".into(),
        ));
    }
    story_carriers::verify(&reopened, page_number, owner_key, lines)?;
    let extracted = reopened.get_page_text(page_number)?;
    for line in lines {
        if !line.text.trim().is_empty()
            && !extracted.contains(&line.text)
            && !extracted
                .split_whitespace()
                .collect::<String>()
                .contains(&line.text.split_whitespace().collect::<String>())
        {
            return Err(WellfriendError::MalformedPdf(
                "story line failed reopen extraction".into(),
            ));
        }
    }
    Ok(StoryFrameWrite {
        bytes: output,
        binding,
    })
}

/// Conservative font-name reachability through page streams, nested Forms,
/// patterns, Type3 charprocs and annotation appearances. Only unused resources
/// carrying this editor's exact owner marker may be retired.
fn reachable_font_names(
    reader: &crate::PdfReader,
    content_roots: &[PdfObject],
    resources: &crate::PdfDictionary,
    annotations: Option<PdfObject>,
    updates: &[IncrementalObject],
) -> Result<BTreeSet<String>> {
    // Resource dictionary keys are arbitrary PDF names, so visit every child.
    fn visit(
        reader: &crate::PdfReader,
        value: PdfObject,
        updates: &[IncrementalObject],
        seen: &mut BTreeSet<(u32, u16, bool)>,
        names: &mut BTreeSet<String>,
        content: bool,
        depth: usize,
    ) -> Result<()> {
        if depth > 64 || seen.len() > 100_000 {
            return Err(WellfriendError::ResourceLimit(
                "story font resource graph".into(),
            ));
        }
        crate::cancel::check_current_cancel("story font resource graph")?;
        match value {
            PdfObject::Reference { number, generation } => {
                if !seen.insert((number, generation, content)) {
                    return Ok(());
                }
                let object = updates
                    .iter()
                    .find(|o| (o.number, o.generation) == (number, generation))
                    .map(|o| Ok(o.object.clone()))
                    .unwrap_or_else(|| reader.get_object(number, generation))?;
                visit(reader, object, updates, seen, names, content, depth + 1)?;
            }
            PdfObject::Array(items) => {
                for item in items {
                    visit(reader, item, updates, seen, names, content, depth + 1)?;
                }
            }
            object @ PdfObject::Stream { .. } => {
                let dict = object.as_stream().unwrap().0;
                if dict.get_name("Subtype") == Some("Image")
                    || dict.get_name("Type") == Some("WellfriendStoryFont")
                {
                    return Ok(());
                }
                if content
                    || dict.get_name("Subtype") == Some("Form")
                    || dict.contains_key("PatternType")
                {
                    let decoded = decode_stream_lossless_with_limits(
                        &object,
                        reader,
                        &DecodeLimits {
                            max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES
                                as u64,
                            ..Default::default()
                        },
                    )?;
                    if decoded.status != StreamDecodeStatus::Complete {
                        return Err(WellfriendError::UnsupportedFeature(
                            "opaque story font dependency".into(),
                        ));
                    }
                    let mut operands = Vec::new();
                    for token in lex_content(&decoded.data)? {
                        if let LexicalKind::Word(op) = &token.kind {
                            if op == "Tf" {
                                if let Some(LexicalToken {
                                    kind: LexicalKind::Name(name),
                                    ..
                                }) = operands.first()
                                {
                                    names.insert(name.clone());
                                }
                            }
                            operands.clear();
                        } else {
                            operands.push(token);
                        }
                    }
                }
                if let Some(resources) = dict.get("Resources") {
                    visit(
                        reader,
                        resources.clone(),
                        updates,
                        seen,
                        names,
                        false,
                        depth + 1,
                    )?;
                }
            }
            PdfObject::Dictionary(dict) => {
                for (key, child) in dict.iter() {
                    if matches!(
                        key.as_str(),
                        "Parent" | "P" | "Pages" | "FontFile" | "FontFile2" | "FontFile3"
                    ) {
                        continue;
                    }
                    visit(
                        reader,
                        child.clone(),
                        updates,
                        seen,
                        names,
                        content || matches!(key.as_str(), "Contents" | "AP" | "CharProcs"),
                        depth + 1,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut names = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for content in content_roots {
        visit(
            reader,
            content.clone(),
            updates,
            &mut seen,
            &mut names,
            true,
            0,
        )?;
    }
    visit(
        reader,
        PdfObject::Dictionary(resources.clone()),
        updates,
        &mut seen,
        &mut names,
        false,
        0,
    )?;
    if let Some(annots) = annotations {
        visit(reader, annots, updates, &mut seen, &mut names, false, 0)?;
    }
    Ok(names)
}

fn story_reachable_font_names(
    reader: &crate::PdfReader,
    page: &crate::document::PdfPage,
    updates: &[IncrementalObject],
) -> Result<BTreeSet<String>> {
    let contents = page
        .contents
        .iter()
        .map(|&(number, generation)| PdfObject::Reference { number, generation })
        .collect::<Vec<_>>();
    let annotations = reader
        .get_object(page.object_number, page.generation_number)?
        .as_dict()
        .and_then(|dictionary| dictionary.get("Annots"))
        .cloned();
    reachable_font_names(reader, &contents, &page.resources, annotations, updates)
}

/// Remove only inactive font resource names created by this editor. Indirect
/// font programs remain in historical revisions, but they no longer accumulate
/// in the active resource dictionary or count toward later font discovery.
/// Every candidate needs both the private Type0 marker and the private resource
/// prefix; story-owned fonts retain their separate owner-specific lifecycle.
pub(super) fn retire_unreferenced_generated_fonts(
    reader: &crate::PdfReader,
    content_roots: &[PdfObject],
    resources: &mut crate::PdfDictionary,
    annotations: Option<PdfObject>,
    updates: &[IncrementalObject],
    protected_names: &BTreeSet<String>,
) -> Result<usize> {
    let Some(mut fonts) = resolve_advanced_editing_dict(resources.get("Font"), reader) else {
        return Ok(0);
    };
    let candidates = fonts
        .iter()
        .filter_map(|(name, value)| {
            if protected_names.contains(name) || !name.starts_with("OxP20F") {
                return None;
            }
            let object = match value {
                PdfObject::Reference { number, generation } => updates
                    .iter()
                    .find(|object| (object.number, object.generation) == (*number, *generation))
                    .map(|object| object.object.clone())
                    .or_else(|| reader.get_object(*number, *generation).ok()),
                value => Some(value.clone()),
            }?;
            let dictionary = object.as_dict()?;
            (dictionary.get_name("WFAdvancedEditingGeneratedFont") == Some("V1")
                && !dictionary.contains_key("WFStoryOwner"))
            .then(|| name.clone())
        })
        .collect::<BTreeSet<_>>();
    if candidates.is_empty() {
        return Ok(0);
    }
    let used = reachable_font_names(reader, content_roots, resources, annotations, updates)?;
    let retired = candidates.difference(&used).cloned().collect::<Vec<_>>();
    for name in &retired {
        fonts.remove(name);
    }
    if !retired.is_empty() {
        resources.insert("Font", PdfObject::Dictionary(fonts));
    }
    Ok(retired.len())
}

#[allow(clippy::too_many_arguments)]
fn build_type0_font_objects(
    font: &[u8],
    glyphs: &[GeneratedGlyph],
    vertical: bool,
    font_file_number: u32,
    descriptor_number: u32,
    cid_to_gid_number: u32,
    to_unicode_number: u32,
    descendant_number: u32,
    type0_number: u32,
) -> Result<Vec<IncrementalObject>> {
    let embedding = crate::fonts::pdf_embedding::EmbeddingInfo::parse(font)?;
    let face = ttf_parser::Face::parse(font, 0).map_err(|_| {
        WellfriendError::UnsupportedFeature(
            "advanced_editing cannot embed malformed sfnt font".to_string(),
        )
    })?;
    if !crate::fonts::pdf_embedding::editable_outline_embedding_allowed(&face) {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing approved font license forbids editable outline embedding".to_string(),
        ));
    }
    // TrueType glyf fonts retain the existing GID-preserving subset path.
    // CFF1 programs embed whole. Their Encoding CMap maps our Unicode-bearing
    // character codes to native charset CIDs; those need not equal GIDs.
    let true_type_outlines = face.tables().glyf.is_some();
    let requested_glyphs = glyphs
        .iter()
        .map(|glyph| glyph.gid)
        .collect::<BTreeSet<_>>();
    let embedded_font_bytes = if true_type_outlines
        && !crate::fonts::pdf_embedding::serialized_subsetting_allowed(&face)
    {
        font.to_vec()
    } else if true_type_outlines {
        subset_glyf_preserving_gids(font, &requested_glyphs)
            .map_err(|error| {
                WellfriendError::UnsupportedFeature(format!(
                    "advanced_editing cannot create a GID-preserving TrueType subset: {error}"
                ))
            })?
            .bytes
    } else if font.starts_with(b"OTTO") && face.tables().cff.is_some() {
        font.to_vec()
    } else {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing font has neither TrueType glyf outlines nor a standalone OpenType/CFF1 container"
                .to_string(),
        ));
    };
    let subset_digest = Sha256::digest(&embedded_font_bytes);
    let subset_tag = subset_digest[..6]
        .iter()
        .map(|byte| char::from(b'A' + (byte % 26)))
        .collect::<String>();
    let family = embedding
        .cff
        .as_ref()
        .map(|cff| cff.postscript_name.clone())
        .unwrap_or_else(|| {
            face.names()
                .into_iter()
                .filter(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
                .find_map(|n| n.to_string())
                .unwrap_or_else(|| "WellfriendAdvancedEditingUnicode".into())
        });
    let base_font_name = if true_type_outlines && embedding.may_subset {
        format!("{subset_tag}+{family}")
    } else {
        family
    };
    let upem = f64::from(face.units_per_em()).max(1.0);
    let units = |value: i16| canonical_number(f64::from(value) / upem * 1000.0);
    let bbox = face.global_bounding_box();
    let compressed_font = flate_encode_cancellable(&embedded_font_bytes, 6)?;
    let mut font_file_dict = crate::PdfDictionary::empty();
    font_file_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    font_file_dict.insert("Length", PdfObject::Integer(compressed_font.len() as i64));
    if true_type_outlines {
        font_file_dict.insert(
            "Length1",
            PdfObject::Integer(embedded_font_bytes.len() as i64),
        );
    } else {
        font_file_dict.insert("Subtype", PdfObject::Name("OpenType".to_string()));
    }

    let mut descriptor = crate::PdfDictionary::empty();
    descriptor.insert("Type", PdfObject::Name("FontDescriptor".to_string()));
    descriptor.insert("FontName", PdfObject::Name(base_font_name.clone()));
    descriptor.insert("Flags", PdfObject::Integer(4));
    descriptor.insert(
        "FontBBox",
        PdfObject::Array(vec![
            PdfObject::Real(units(bbox.x_min)),
            PdfObject::Real(units(bbox.y_min)),
            PdfObject::Real(units(bbox.x_max)),
            PdfObject::Real(units(bbox.y_max)),
        ]),
    );
    descriptor.insert("ItalicAngle", PdfObject::Integer(0));
    descriptor.insert("Ascent", PdfObject::Real(units(face.ascender())));
    descriptor.insert("Descent", PdfObject::Real(units(face.descender())));
    descriptor.insert("CapHeight", PdfObject::Real(units(face.ascender())));
    descriptor.insert("StemV", PdfObject::Integer(80));
    descriptor.insert(
        if true_type_outlines {
            "FontFile2"
        } else {
            "FontFile3"
        },
        PdfObject::Reference {
            number: font_file_number,
            generation: 0,
        },
    );

    let mut cid_to_gid = Vec::new();
    if true_type_outlines {
        let maximum_cid = glyphs
            .iter()
            .map(|glyph| usize::from(glyph.cid))
            .max()
            .unwrap_or(0);
        cid_to_gid = vec![0u8; (maximum_cid + 1) * 2];
        for glyph in glyphs {
            let offset = usize::from(glyph.cid) * 2;
            cid_to_gid[offset] = (glyph.gid >> 8) as u8;
            cid_to_gid[offset + 1] = (glyph.gid & 0xff) as u8;
        }
    }
    let (mut map_dict, map_bytes) = if let Some(cff) = &embedding.cff {
        cff.encoding(glyphs.iter().map(|glyph| (glyph.cid, glyph.gid)), vertical)?
    } else {
        (crate::PdfDictionary::empty(), cid_to_gid)
    };
    let compressed_map = flate_encode_cancellable(&map_bytes, 6)?;
    map_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    map_dict.insert("Length", PdfObject::Integer(compressed_map.len() as i64));

    let to_unicode = build_to_unicode_cmap(glyphs);
    let compressed_to_unicode = flate_encode_cancellable(to_unicode.as_bytes(), 6)?;
    let mut to_unicode_dict = crate::PdfDictionary::empty();
    to_unicode_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    to_unicode_dict.insert(
        "Length",
        PdfObject::Integer(compressed_to_unicode.len() as i64),
    );

    let cid_system = embedding.system().dictionary();
    let mut by_cid = BTreeMap::<u16, f64>::new();
    for glyph in glyphs {
        by_cid
            .entry(embedding.cid(glyph.cid, glyph.gid)?)
            .or_insert_with(|| {
                canonical_number(if vertical || embedding.cff.is_some() {
                    glyph.font_width
                } else {
                    glyph.advance.abs()
                })
            });
    }
    let widths = by_cid
        .into_iter()
        .flat_map(|(cid, width)| {
            [
                PdfObject::Integer(i64::from(cid)),
                PdfObject::Array(vec![PdfObject::Real(width)]),
            ]
        })
        .collect::<Vec<_>>();
    let mut descendant = crate::PdfDictionary::empty();
    descendant.insert("Type", PdfObject::Name("Font".to_string()));
    descendant.insert(
        "Subtype",
        PdfObject::Name(
            if true_type_outlines {
                "CIDFontType2"
            } else {
                "CIDFontType0"
            }
            .to_string(),
        ),
    );
    descendant.insert("BaseFont", PdfObject::Name(base_font_name.clone()));
    descendant.insert("CIDSystemInfo", PdfObject::Dictionary(cid_system));
    descendant.insert(
        "FontDescriptor",
        PdfObject::Reference {
            number: descriptor_number,
            generation: 0,
        },
    );
    descendant.insert("DW", PdfObject::Integer(1000));
    descendant.insert("W", PdfObject::Array(widths));
    if vertical {
        let mut metrics = BTreeMap::<u16, f64>::new();
        for glyph in glyphs {
            metrics
                .entry(embedding.cid(glyph.cid, glyph.gid)?)
                .or_insert(glyph.advance);
        }
        descendant.insert(
            "DW2",
            PdfObject::Array(vec![PdfObject::Integer(0), PdfObject::Integer(-1000)]),
        );
        // Generated matrices already contain the shaping origin. A zero W2
        // origin prevents readers subtracting it a second time. W retains the
        // actual horizontal font metric; W2 carries the independent vertical one.
        descendant.insert(
            "W2",
            PdfObject::Array(
                metrics
                    .into_iter()
                    .flat_map(|(cid, advance)| {
                        [
                            PdfObject::Integer(i64::from(cid)),
                            PdfObject::Array(vec![
                                PdfObject::Real(canonical_number(-advance)),
                                PdfObject::Integer(0),
                                PdfObject::Integer(0),
                            ]),
                        ]
                    })
                    .collect(),
            ),
        );
    }
    if true_type_outlines {
        descendant.insert(
            "CIDToGIDMap",
            PdfObject::Reference {
                number: cid_to_gid_number,
                generation: 0,
            },
        );
    }

    let mut type0 = crate::PdfDictionary::empty();
    type0.insert("Type", PdfObject::Name("Font".to_string()));
    type0.insert("Subtype", PdfObject::Name("Type0".to_string()));
    type0.insert(
        "WFAdvancedEditingGeneratedFont",
        PdfObject::Name("V1".to_string()),
    );
    type0.insert("BaseFont", PdfObject::Name(base_font_name));
    type0.insert(
        "Encoding",
        if true_type_outlines {
            PdfObject::Name(if vertical { "Identity-V" } else { "Identity-H" }.to_string())
        } else {
            PdfObject::Reference {
                number: cid_to_gid_number,
                generation: 0,
            }
        },
    );
    type0.insert(
        "DescendantFonts",
        PdfObject::Array(vec![PdfObject::Reference {
            number: descendant_number,
            generation: 0,
        }]),
    );
    type0.insert(
        "ToUnicode",
        PdfObject::Reference {
            number: to_unicode_number,
            generation: 0,
        },
    );

    let mut objects = vec![
        IncrementalObject {
            number: font_file_number,
            generation: 0,
            object: PdfObject::Stream {
                dict: font_file_dict,
                raw: compressed_font,
            },
        },
        IncrementalObject {
            number: descriptor_number,
            generation: 0,
            object: PdfObject::Dictionary(descriptor),
        },
        IncrementalObject {
            number: to_unicode_number,
            generation: 0,
            object: PdfObject::Stream {
                dict: to_unicode_dict,
                raw: compressed_to_unicode,
            },
        },
        IncrementalObject {
            number: descendant_number,
            generation: 0,
            object: PdfObject::Dictionary(descendant),
        },
        IncrementalObject {
            number: type0_number,
            generation: 0,
            object: PdfObject::Dictionary(type0),
        },
    ];
    objects.push(IncrementalObject {
        number: cid_to_gid_number,
        generation: 0,
        object: PdfObject::Stream {
            dict: map_dict,
            raw: compressed_map,
        },
    });
    objects.sort_by_key(|object| (object.number, object.generation));
    Ok(objects)
}

fn build_to_unicode_cmap(glyphs: &[GeneratedGlyph]) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /WellfriendAdvancedEditingToUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    // One CID may occur repeatedly after shaping.  A ToUnicode CMap may only
    // define one mapping per source code, so collapse identical repeats and
    // reject conflicting cluster mappings instead of emitting an ambiguous
    // CMap whose interpretation varies between consumers.
    let mut mappings = BTreeMap::<u16, &str>::new();
    for glyph in glyphs {
        let Some(unicode) = glyph.to_unicode.as_deref() else {
            continue;
        };
        if let Some(previous) = mappings.insert(glyph.cid, unicode) {
            if previous != unicode {
                // The shaped visual still carries a surrounding ActualText
                // span.  Keep the first deterministic glyph mapping for
                // consumers that ignore marked-content replacement text.
                mappings.insert(glyph.cid, previous);
            }
        }
    }
    let mappings = mappings.into_iter().collect::<Vec<_>>();
    for chunk in mappings.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (cid, unicode) in chunk {
            cmap.push_str(&format!("<{:04X}> <{}>\n", cid, utf16be_hex(unicode)));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap
}

fn utf16be_hex(text: &str) -> String {
    text.encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect::<String>()
}

fn utf16be_hex_with_bom(text: &str) -> String {
    format!("FEFF{}", utf16be_hex(text))
}

fn serialize_generated_text(
    layout: &[Vec<GeneratedGlyph>],
    font_name: &str,
    options: &AdvancedTextEditOptions,
    vertical: bool,
    line_regions: Option<&[[f64; 4]]>,
    logical_actual_text: Option<&str>,
) -> Result<(String, Vec<GeneratedLineAdjustment>)> {
    // Positioned RTL columns have a deliberate visual geometry conflict with
    // the generic left-to-right coordinate sorter used by text extraction.
    // Wrap the same shaped text object in a standard PDF ActualText span so
    // extraction, accessibility consumers, and search retain the logical
    // story order while the CIDs continue to paint in visual glyph order.
    let mut content = String::from("q\n");
    if let Some(text) = logical_actual_text {
        content.push_str(&format!(
            "/Span << /ActualText <{}> >> BDC\n",
            utf16be_hex_with_bom(text)
        ));
    }
    content.push_str(&format!(
        "BT\n/{font_name} {} Tf\n",
        fmt_num(options.font_size)
    ));
    let mut adjustments = Vec::with_capacity(layout.len());
    if line_regions.is_some_and(|regions| regions.len() != layout.len()) {
        return Err(WellfriendError::invalid_input(
            "advanced_editing positioned serializer has mismatched line regions",
        ));
    }
    if vertical {
        content.push_str("0 Tc\n0 Tw\n100 Tz\n0 Ts\n0 Tr\n");
        adjustments =
            vertical_text::serialize(&mut content, layout, font_name, options, line_regions, None)?;
    } else {
        let line_advance = options.font_size * options.line_spacing;
        for (line, glyphs) in layout.iter().enumerate() {
            crate::cancel::check_current_cancel("advanced generated text line serialization")?;
            let region = line_regions
                .and_then(|regions| regions.get(line).copied())
                .unwrap_or(options.region);
            let line_width = glyphs.iter().map(|glyph| glyph.advance.abs()).sum::<f64>() / 1000.0
                * options.font_size;
            let rtl = glyphs.first().is_some_and(|glyph| {
                glyph
                    .visual_unicode
                    .chars()
                    .any(|ch| matches!(ch as u32, 0x0590..=0x08FF | 0xFB1D..=0xFEFF))
            });
            let target_width = region[2] - region[0];
            let last_line = line + 1 == layout.len();
            let mut word_spacing = 0.0;
            let mut character_spacing = 0.0;
            let mut residual = target_width - line_width;
            let applied = true;
            let refusal_reason = None;
            if options.alignment == GeneratedTextAlignment::Justify
                && (options.justify_last_line || !last_line)
                && !glyphs.is_empty()
                && residual > EPSILON
            {
                let word_count = glyphs
                    .iter()
                    .filter(|glyph| glyph.visual_unicode == " ")
                    .count();
                let character_count = glyphs.len().saturating_sub(1);
                if word_count > 0 {
                    word_spacing = (residual / word_count as f64 / options.font_size)
                        .min(options.max_word_spacing);
                    residual -= word_spacing * options.font_size * word_count as f64;
                }
                if residual > EPSILON && character_count > 0 {
                    character_spacing = (residual / character_count as f64 / options.font_size)
                        .min(options.max_character_spacing);
                    residual -= character_spacing * options.font_size * character_count as f64;
                }
                if residual > EPSILON {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing full justification exceeds configured text-state spacing bounds"
                            .to_string(),
                    ));
                }
            }
            let painted_width = line_width
                + word_spacing
                    * options.font_size
                    * glyphs
                        .iter()
                        .filter(|glyph| glyph.visual_unicode == " ")
                        .count() as f64
                + character_spacing * options.font_size * glyphs.len().saturating_sub(1) as f64;
            let x = match options.alignment {
                GeneratedTextAlignment::Left => region[0],
                GeneratedTextAlignment::Right => region[2] - painted_width,
                GeneratedTextAlignment::Center => region[0] + (target_width - painted_width) / 2.0,
                GeneratedTextAlignment::Start => {
                    if rtl {
                        region[2] - painted_width
                    } else {
                        region[0]
                    }
                }
                GeneratedTextAlignment::End => {
                    if rtl {
                        region[0]
                    } else {
                        region[2] - painted_width
                    }
                }
                GeneratedTextAlignment::Justify => {
                    if rtl {
                        region[2] - painted_width
                    } else {
                        region[0]
                    }
                }
            };
            let y = if line_regions.is_some() {
                region[3] - options.font_size
            } else {
                options.region[3] - options.font_size - line as f64 * line_advance
            };
            if word_spacing.abs() > EPSILON {
                content.push_str(&format!("{} Tw\n", fmt_num(word_spacing)));
            }
            if character_spacing.abs() > EPSILON {
                content.push_str(&format!("{} Tc\n", fmt_num(character_spacing)));
            }
            let mut glyph_x = x;
            for (glyph_index, glyph) in glyphs.iter().enumerate() {
                let positioned_x = glyph_x + glyph.offset_x / 1000.0 * options.font_size;
                let positioned_y = y + glyph.offset_y / 1000.0 * options.font_size;
                content.push_str(&format!(
                    "1 0 0 1 {} {} Tm <{:04X}> Tj\n",
                    fmt_num(positioned_x),
                    fmt_num(positioned_y),
                    glyph.cid
                ));
                glyph_x += glyph.advance.abs() / 1000.0 * options.font_size;
                // Every glyph receives an absolute `Tm`, so PDF's implicit
                // `Tw`/`Tc` advance cannot position the next glyph. Apply the
                // same bounded spacing explicitly to the next coordinate.
                if glyph.visual_unicode == " " {
                    glyph_x += word_spacing * options.font_size;
                }
                if glyph_index + 1 < glyphs.len() {
                    glyph_x += character_spacing * options.font_size;
                }
            }
            if word_spacing.abs() > EPSILON {
                content.push_str("0 Tw\n");
            }
            if character_spacing.abs() > EPSILON {
                content.push_str("0 Tc\n");
            }
            adjustments.push(GeneratedLineAdjustment {
                line_index: line,
                natural_width: line_width,
                target_width,
                residual: residual.max(0.0),
                word_spacing,
                character_spacing,
                alignment: options.alignment,
                last_line,
                applied,
                refusal_reason,
            });
        }
    }
    content.push_str("ET\n");
    if logical_actual_text.is_some() {
        content.push_str("EMC\n");
    }
    content.push('Q');
    Ok((content, adjustments))
}

fn reject_unsafe_text_controls(text: &str) -> Result<()> {
    for (offset, ch) in text.char_indices() {
        if matches!(ch, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') {
            // Explicit bidi controls are accepted only when balanced. Validation
            // below prevents controls from leaking across the edited paragraph.
            continue;
        }
        if ch == '\0' {
            return Err(WellfriendError::MalformedPdf(format!(
                "advanced_editing text contains NUL at UTF-8 byte {offset}"
            )));
        }
    }
    let mut depth = 0usize;
    for (offset, ch) in text.char_indices() {
        match ch {
            '\u{202A}' | '\u{202B}' | '\u{202D}' | '\u{202E}' | '\u{2066}' | '\u{2067}'
            | '\u{2068}' => depth = depth.saturating_add(1),
            '\u{202C}' | '\u{2069}' => {
                if depth == 0 {
                    return Err(WellfriendError::MalformedPdf(format!(
                        "advanced_editing unmatched bidi pop control at UTF-8 byte {offset}"
                    )));
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(WellfriendError::MalformedPdf(format!(
            "advanced_editing text ends with {depth} unclosed bidi control sequence(s)"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchStringRepresentation {
    Literal,
    Hexadecimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SameWidthMode {
    Exact,
    Tolerance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SameWidthPatchOptions {
    pub mode: SameWidthMode,
    pub advance_tolerance_1000: f64,
    pub signature_policy_override: bool,
    pub require_same_serialized_length: bool,
    #[serde(default)]
    pub target_stream_object: Option<u32>,
    #[serde(default)]
    pub target_stream_generation: Option<u16>,
    #[serde(default)]
    pub target_decoded_byte_range: Option<[usize; 2]>,
}

impl Default for SameWidthPatchOptions {
    fn default() -> Self {
        Self {
            mode: SameWidthMode::Exact,
            advance_tolerance_1000: 0.0,
            signature_policy_override: false,
            require_same_serialized_length: true,
            target_stream_object: None,
            target_stream_generation: None,
            target_decoded_byte_range: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SameWidthPatchEligibility {
    pub schema_version: String,
    pub status: AdvancedEditingSupportStatus,
    pub eligible: bool,
    pub page: usize,
    pub stream_object: u32,
    pub stream_generation: u16,
    pub operator: String,
    pub tj_element: Option<usize>,
    pub decoded_byte_start: usize,
    pub decoded_byte_end: usize,
    pub representation: PatchStringRepresentation,
    pub font_resource: String,
    pub font_type: String,
    pub encoding: String,
    pub cmap: String,
    pub glyph_count_before: usize,
    pub glyph_count_after: usize,
    pub encoded_bytes_before: usize,
    pub encoded_bytes_after: usize,
    pub serialized_bytes_before: usize,
    pub serialized_bytes_after: usize,
    pub glyph_advances_before: Vec<f64>,
    pub glyph_advances_after: Vec<f64>,
    pub total_advance_before: f64,
    pub total_advance_after: f64,
    pub advance_delta: f64,
    pub writing_mode: i32,
    pub text_render_mode: i32,
    pub marked_content_depth: usize,
    pub clipping_semantics: bool,
    pub encrypted: bool,
    pub filters: Vec<String>,
    pub incremental_feasible: bool,
    pub full_rewrite_feasible: bool,
    pub exact_reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SameWidthPatchEligibilityReport {
    pub schema_version: String,
    pub source_text: String,
    pub replacement_text: String,
    pub candidates: Vec<SameWidthPatchEligibility>,
    pub signature_policy: EditPolicyReport,
    pub deterministic: bool,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SameWidthPatchApplyReport {
    pub schema_version: String,
    pub selected: SameWidthPatchEligibility,
    pub original_bytes: usize,
    pub output_bytes: usize,
    pub rewritten_stream_bytes: usize,
    pub appended_revision_bytes: usize,
    pub original_prefix_preserved: bool,
    pub output_reopened: bool,
    pub replacement_extracts: bool,
    pub old_text_absent: bool,
    pub output_sha256: String,
    pub signature_policy: EditPolicyReport,
    pub cryptographic_validity_claimed: bool,
    pub deterministic: bool,
    pub cache_invalidation: CacheInvalidationReport,
}

#[derive(Debug, Clone)]
struct ContentStringToken {
    operation_start: usize,
    operation_end: usize,
    token_start: usize,
    token_end: usize,
    representation: PatchStringRepresentation,
    decoded: Vec<u8>,
    font_name: String,
    font_size: f64,
    character_spacing: f64,
    word_spacing: f64,
    horizontal_scaling: f64,
    text_rise: f64,
    fill_color_command: String,
    stroke_color_command: String,
    unsupported_fill_paint_state: bool,
    unsupported_stroke_paint_state: bool,
    operator: String,
    element: Option<usize>,
    text_render_mode: i32,
    marked_depth: usize,
    authored_typed_owner: Option<(String, String)>,
    authored_typed_region: Option<[f64; 4]>,
    actual_text_sources: Vec<ActualTextSource>,
    named_marked_properties: Vec<String>,
    unresolved_actual_text: bool,
    flow_relocatable: bool,
    source_position: Option<inline_text::PositionSnapshot>,
    generated_basis: inline_text::SourceBasis,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ActualTextSource {
    owner_object: u32,
    owner_generation: u16,
    value_start: usize,
    value_end: usize,
    logical_text: Arc<str>,
}

#[derive(Debug, Clone)]
struct ScannedTextGraphicsState {
    font_name: String,
    font_size: f64,
    render_mode: i32,
    character_spacing: f64,
    word_spacing: f64,
    horizontal_scaling: f64,
    text_rise: f64,
    fill_color_command: String,
    stroke_color_command: String,
    fill_color_space_command: String,
    stroke_color_space_command: String,
    unsupported_fill_paint_state: bool,
    unsupported_stroke_paint_state: bool,
}

/// Stateful text/paint scanner context for a logical content sequence. A PDF
/// page `/Contents` array is one concatenated content stream, so graphics,
/// text, and marked-content state can legally open in one member and continue
/// in the next. Keeping this state between members prevents cross-stream edits
/// from silently replaying default style or losing MCID nesting.
#[derive(Debug, Clone)]
pub(crate) struct ScannedTextTokenState {
    font_name: String,
    font_size: f64,
    render_mode: i32,
    character_spacing: f64,
    word_spacing: f64,
    horizontal_scaling: f64,
    text_rise: f64,
    fill_color_command: String,
    stroke_color_command: String,
    fill_color_space_command: String,
    stroke_color_space_command: String,
    unsupported_fill_paint_state: bool,
    unsupported_stroke_paint_state: bool,
    marked_depth: usize,
    authored_typed_owner_stack: Vec<Option<(String, String)>>,
    authored_typed_region_stack: Vec<Option<[f64; 4]>>,
    actual_text_stack: Vec<Option<ActualTextSource>>,
    named_property_stack: Vec<Option<String>>,
    actual_text_conflict_stack: Vec<bool>,
    flow_scope_stack: Vec<bool>,
    graphics_stack: Vec<ScannedTextGraphicsState>,
    position: inline_text::PositionState,
}

impl Default for ScannedTextTokenState {
    fn default() -> Self {
        Self {
            font_name: String::new(),
            font_size: 0.0,
            render_mode: 0,
            character_spacing: 0.0,
            word_spacing: 0.0,
            horizontal_scaling: 100.0,
            text_rise: 0.0,
            fill_color_command: "0 g".to_string(),
            stroke_color_command: "0 G".to_string(),
            fill_color_space_command: String::new(),
            stroke_color_space_command: String::new(),
            unsupported_fill_paint_state: false,
            unsupported_stroke_paint_state: false,
            marked_depth: 0,
            authored_typed_owner_stack: Vec::new(),
            authored_typed_region_stack: Vec::new(),
            actual_text_stack: Vec::new(),
            named_property_stack: Vec::new(),
            actual_text_conflict_stack: Vec::new(),
            flow_scope_stack: Vec::new(),
            graphics_stack: Vec::new(),
            position: inline_text::PositionState::default(),
        }
    }
}

type SelectedMultiRunOperand = (
    u32,
    u16,
    Arc<PdfObject>,
    Arc<Vec<u8>>,
    ContentStringToken,
    MultiRunSourceSpan,
);

type DecodedStreamEdits = BTreeMap<(u32, u16), (PdfObject, Vec<u8>, Vec<(usize, usize, Vec<u8>)>)>;

#[derive(Debug, Clone)]
struct ActualTextCleanupPatch {
    owner: (u32, u16),
    value_start: usize,
    value_end: usize,
    logical_text: Arc<str>,
    logical_range: [usize; 2],
}

fn serialize_authored_owner_source_lines(
    lines: &[ExplicitLayoutLine],
    replacement_text: &str,
    resolver: &FontResolver,
    style: &PreservedTextStyle,
    options: &AdvancedTextEditOptions,
) -> Result<String> {
    if lines.is_empty()
        || lines
            .iter()
            .map(|line| line.logical_text.as_str())
            .collect::<String>()
            != replacement_text
    {
        return Err(WellfriendError::invalid_input(
            "authored positioned source lines must cover the replacement exactly",
        ));
    }
    let mut content = format!(
        "q\n/Span << /ActualText <{}> >> BDC\nBT\n",
        utf16be_hex_with_bom(replacement_text)
    );
    let serialized_resource = serialized_name_body(&style.font_resource);
    append_generated_preserved_style(&mut content, &serialized_resource, style);
    let line_advance = options.font_size * options.line_spacing;
    for (line_index, line) in lines.iter().enumerate() {
        if line.inserted_visual_hyphen {
            return Err(WellfriendError::UnsupportedFeature(
                "authored source-font redistribution requires an approved generated font for inserted hyphen glyphs"
                    .into(),
            ));
        }
        let visible = line
            .logical_text
            .trim_end_matches(crate::fonts::hard_break::is_hard_break);
        if line.visual_text != visible {
            return Err(WellfriendError::invalid_input(
                "authored source-font line visual text differs from logical text",
            ));
        }
        let (encoded, ambiguous) = encode_with_existing_font(resolver, visible)?;
        if ambiguous {
            return Err(WellfriendError::UnsupportedFeature(
                "authored positioned line has an ambiguous source-font mapping".into(),
            ));
        }
        let natural_width = preserved_run_advance(resolver, &encoded, visible, style)?;
        let rtl = contains_rtl_or_bidi_controls(visible);
        let x = preserved_style_line_x(options.alignment, rtl, options.region, natural_width)?;
        let y = options.region[3] - options.font_size - line_index as f64 * line_advance;
        content.push_str(&format!("1 0 0 1 {} {} Tm <", fmt_num(x), fmt_num(y)));
        for byte in encoded {
            content.push_str(&format!("{byte:02X}"));
        }
        content.push_str("> Tj\n");
    }
    content.push_str("ET\nEMC\nQ");
    Ok(content)
}

#[allow(clippy::too_many_arguments)]
fn edit_authored_owner_positioned_fragment(
    input: &[u8],
    request: &MultiRunTextRangeRequest,
    font_bytes: Option<&[u8]>,
    page: &crate::document::PdfPage,
    resources: &PageResources,
    scope: Option<&form_text::Scope>,
    selected: &[SelectedMultiRunOperand],
    stream_sources: &BTreeMap<(u32, u16), (Arc<PdfObject>, Arc<Vec<u8>>)>,
    actual_text_cleanup_patches: &[ActualTextCleanupPatch],
    old_selected: &str,
    signature_policy: EditPolicyReport,
    force_generated_style: bool,
    protected_authored_fonts: &BTreeSet<String>,
) -> Result<(Vec<u8>, MultiRunTextEditReport)> {
    let lines = request.final_lines.as_deref().ok_or_else(|| {
        WellfriendError::invalid_input("authored positioned edit requires final lines")
    })?;
    let first = selected.first().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "authored positioned edit requires one exact source owner".into(),
        )
    })?;
    let first_is_direct_carrier = first.4.operator == "Tj"
        || (first.4.operator == "TJ"
            && first.4.decoded.is_empty()
            && first.5.logical_range[0] == first.5.logical_range[1]);
    if !first_is_direct_carrier
        || selected.iter().any(|item| {
            item.5.logical_range[0] < request.logical_start
                || item.5.logical_range[1] > request.logical_end
                || item.5.writing_mode != 0
                || item.4.text_render_mode >= 4
        })
    {
        return Err(WellfriendError::UnsupportedFeature(
            "authored positioned redistribution requires complete horizontal non-clipping owner operands and a direct first text carrier"
                .into(),
        ));
    }
    let style = preserved_style_from_token(&first.4)?;
    if style.vertical
        || selected
            .iter()
            .map(|item| preserved_style_from_token(&item.4))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .any(|candidate| candidate != &style)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "authored positioned redistribution requires one retained text state per fragment"
                .into(),
        ));
    }
    let source_font = resources.fonts.get(&style.font_resource).ok_or_else(|| {
        WellfriendError::MalformedPdf("authored positioned source font resource disappeared".into())
    })?;
    let reader = crate::ContentEngine::open_bytes(input.to_vec())?;
    let document_reader = reader.document().reader();
    let resolver = FontResolver::new(source_font, document_reader);

    let mut created = Vec::<IncrementalObject>::new();
    let positioned_content = if force_generated_style {
        let font = font_bytes
            .or_else(|| get_fallback_font("Symbol"))
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "authored positioned shaping font unavailable".into(),
                )
            })?;
        let mut layout =
            layout_generated_explicit_lines(lines, request.mode, font, &request.options, None)?;
        let mut byte_base = 0usize;
        for (line, glyphs) in lines.iter().zip(layout.iter_mut()) {
            for glyph in glyphs {
                glyph.logical_byte_start = glyph.logical_byte_start.saturating_add(byte_base);
            }
            byte_base = byte_base.saturating_add(line.logical_text.len());
        }
        let all_glyphs = layout.iter().flatten().cloned().collect::<Vec<_>>();
        let base = reserve_advanced_object_block(
            document_reader,
            6,
            "authored positioned generated font",
        )?;
        let font_resource = deterministic_font_resource_name(document_reader, &page.resources);
        created.extend(build_type0_font_objects(
            font,
            &all_glyphs,
            false,
            base,
            base + 1,
            base + 2,
            base + 3,
            base + 4,
            base + 5,
        )?);
        let spans = vec![PreservedStyleSpan {
            byte_start: 0,
            byte_end: request.replacement_text.len(),
            style: style.clone(),
        }];
        let (content, _) = serialize_generated_preserved_styles(
            &layout,
            &spans,
            &font_resource,
            &request.options,
            false,
            Some(&request.replacement_text),
        )?;
        let page_object = document_reader.get_object(page.object_number, page.generation_number)?;
        let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
            WellfriendError::MalformedPdf("authored positioned page is not a dictionary".into())
        })?;
        let mut page_resources = page.resources.clone();
        let mut fonts = resolve_advanced_editing_dict(page_resources.get("Font"), document_reader)
            .unwrap_or_else(crate::PdfDictionary::empty);
        fonts.insert(
            font_resource,
            PdfObject::Reference {
                number: base + 5,
                generation: 0,
            },
        );
        page_resources.insert("Font", PdfObject::Dictionary(fonts));
        page_dict.insert("Resources", PdfObject::Dictionary(page_resources));
        created.push(IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(page_dict),
        });
        content
    } else {
        serialize_authored_owner_source_lines(
            lines,
            &request.replacement_text,
            &resolver,
            &style,
            &request.options,
        )?
    };

    let mut stream_edits = DecodedStreamEdits::new();
    add_actual_text_cleanup_edits(
        &mut stream_edits,
        stream_sources,
        actual_text_cleanup_patches,
    )?;
    for (index, item) in selected.iter().enumerate() {
        let font = resources.fonts.get(&item.5.font_resource).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "authored positioned source font disappeared during rewrite".into(),
            )
        })?;
        let item_resolver = FontResolver::new(font, document_reader);
        let overlap_start = request.logical_start.max(item.5.logical_range[0]);
        let overlap_end = request.logical_end.min(item.5.logical_range[1]);
        let complete_actual_text_owner = item.4.actual_text_sources.last().is_some_and(|source| {
            actual_text_cleanup_patches.iter().any(|patch| {
                patch.owner == (source.owner_object, source.owner_generation)
                    && patch.value_start == source.value_start
                    && patch.value_end == source.value_end
                    && request.logical_start <= patch.logical_range[0]
                    && request.logical_end >= patch.logical_range[1]
            })
        });
        let (prefix, selected_bytes, suffix) = if complete_actual_text_owner {
            (Vec::new(), item.4.decoded.clone(), Vec::new())
        } else {
            split_selected_source_operand(
                &item_resolver,
                &item.4.decoded,
                item.5.logical_range,
                [overlap_start, overlap_end],
            )?
        };
        if !prefix.is_empty() || !suffix.is_empty() {
            return Err(WellfriendError::UnsupportedFeature(
                "authored positioned redistribution refuses a partial source operand".into(),
            ));
        }
        let (start, end, replacement) = if index == 0 {
            let mut replacement = b"ET\n".to_vec();
            replacement.extend_from_slice(positioned_content.as_bytes());
            replacement.extend_from_slice(b"\nBT\n");
            (item.4.operation_start, item.4.operation_end, replacement)
        } else {
            rewrite_source_text_destructively(
                &item.4,
                &item_resolver,
                &[],
                &selected_bytes,
                &[],
                false,
                None,
            )?
        };
        stream_edits
            .entry((item.0, item.1))
            .or_insert_with(|| (item.2.as_ref().clone(), item.3.as_ref().clone(), Vec::new()))
            .2
            .push((start, end, replacement));
    }
    let mut changed =
        materialize_decoded_stream_edits(stream_edits, "authored positioned owner source rewrite")?;
    changed.extend(created);
    // Authored owners deliberately retain a zero-width source carrier for
    // later redistribution. Its exact source font is therefore still part of
    // the owner contract even when this edit replaces all currently visible
    // glyphs with a generated subset.
    let mut protected_fonts = selected
        .iter()
        .map(|item| item.5.font_resource.clone())
        .collect::<BTreeSet<_>>();
    protected_fonts.extend(protected_authored_fonts.iter().cloned());
    let output = form_text::write_scope_with_protected_fonts(
        document_reader,
        page,
        resources,
        changed,
        scope,
        &protected_fonts,
    )?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let extracted = form_text::extract_scope(&reopened, request.page, scope)?;
    let replacement_extracts = extracted.contains(&request.replacement_text)
        || layout_extraction_equivalent(&extracted, &request.replacement_text);
    let old_absent = old_selected.is_empty() || !extracted.contains(old_selected);
    if !replacement_extracts || !output.starts_with(input) {
        return Err(WellfriendError::MalformedPdf(
            "authored positioned save/reopen/extraction proof failed".into(),
        ));
    }
    Ok((
        output.clone(),
        MultiRunTextEditReport {
            schema_version:
                "advanced_editing_closeout.authored-positioned-owner-layout.v1".into(),
            status: AdvancedEditingSupportStatus::ImplementedWithLimits,
            operation: "replace_authored_owner_positioned_lines".into(),
            page: request.page,
            logical_range: [request.logical_start, request.logical_end],
            selected_source_spans: selected.iter().map(|item| item.5.clone()).collect(),
            style_policy: request.style_policy,
            generated_font_used: force_generated_style,
            generated_paint_order: None,
            generated_paint_partitions: Vec::new(),
            replacement_text: request.replacement_text.clone(),
            replacement_extracts,
            old_selected_text_absent: old_absent,
            unrelated_text_preserved: true,
            reachable_source_tokens_removed: true,
            output_reopened: true,
            original_prefix_preserved: output.starts_with(input),
            output_sha256: format!("{:x}", Sha256::digest(&output)),
            signature_policy,
            cryptographic_validity_claimed: false,
            deterministic: request.options.deterministic,
            cache_invalidation: advanced_editing_cache_invalidation(
                input,
                &output,
                true,
                false,
                false,
            ),
            exact_limits: vec![
                "the final shaped lines replace the first exact owner operand at its original paint position; no page-level overlay or content-stream append changes stacking order".into(),
                "the original typed-cell MCID and private owner scope contain the complete replacement; trailing old line operands are removed from the reachable revision".into(),
                "cross-fragment growth still requires authoring pagination and new owned rectangles".into(),
            ],
        },
    ))
}

fn add_actual_text_cleanup_edits(
    target: &mut DecodedStreamEdits,
    stream_sources: &BTreeMap<(u32, u16), (Arc<PdfObject>, Arc<Vec<u8>>)>,
    patches: &[ActualTextCleanupPatch],
) -> Result<()> {
    for patch in patches {
        let source = stream_sources.get(&patch.owner).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing ActualText cleanup owner stream is unavailable".to_string(),
            )
        })?;
        target
            .entry(patch.owner)
            .or_insert_with(|| {
                (
                    source.0.as_ref().clone(),
                    source.1.as_ref().clone(),
                    Vec::new(),
                )
            })
            .2
            .push((patch.value_start, patch.value_end, b"null".to_vec()));
    }
    Ok(())
}

fn add_actual_text_insertion_edits(
    target: &mut DecodedStreamEdits,
    stream_sources: &BTreeMap<(u32, u16), (Arc<PdfObject>, Arc<Vec<u8>>)>,
    patches: &[ActualTextCleanupPatch],
    insertion_scalar: usize,
    inserted_text: &str,
) -> Result<()> {
    for patch in patches {
        let offset = insertion_scalar
            .checked_sub(patch.logical_range[0])
            .filter(|offset| *offset <= patch.logical_range[1] - patch.logical_range[0])
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "advanced_editing ActualText insertion is outside its logical owner".into(),
                )
            })?;
        if patch.logical_text.chars().count()
            != patch.logical_range[1].saturating_sub(patch.logical_range[0])
        {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing ActualText insertion requires an isomorphic logical owner".into(),
            ));
        }
        let mut logical = patch.logical_text.chars().take(offset).collect::<String>();
        logical.push_str(inserted_text);
        logical.extend(patch.logical_text.chars().skip(offset));
        let replacement = format!("<{}>", utf16be_hex_with_bom(&logical)).into_bytes();
        let source = stream_sources.get(&patch.owner).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing ActualText insertion owner stream is unavailable".to_string(),
            )
        })?;
        target
            .entry(patch.owner)
            .or_insert_with(|| {
                (
                    source.0.as_ref().clone(),
                    source.1.as_ref().clone(),
                    Vec::new(),
                )
            })
            .2
            .push((patch.value_start, patch.value_end, replacement));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ActualTextCoverage {
    source: ActualTextSource,
    logical_start: usize,
    logical_end: usize,
}

fn token_has_named_actual_text(
    token: &ContentStringToken,
    resources: &PageResources,
    reader: &crate::PdfReader,
) -> bool {
    token.named_marked_properties.iter().any(|name| {
        resolve_advanced_editing_dict(resources.properties.get(name), reader)
            .is_some_and(|dictionary| dictionary.contains_key("ActualText"))
    })
}

/// The source text-state facts needed to replay a whole provenance-bearing
/// operand as a styled generated run.  This deliberately remains private to
/// the canonical advanced editing content mutator: text reflow consumes the public
/// multi-run operation rather than inventing another text serializer.
#[derive(Debug, Clone, PartialEq)]
struct PreservedTextStyle {
    font_resource: String,
    font_size: f64,
    character_spacing: f64,
    word_spacing: f64,
    horizontal_scaling: f64,
    text_rise: f64,
    text_render_mode: i32,
    fill_color_command: String,
    stroke_color_command: String,
    vertical: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum LexicalKind {
    Null,
    Boolean,
    InlineImageData,
    ReferenceMarker,
    String(PatchStringRepresentation, Vec<u8>),
    Name(String),
    Number(f64),
    ArrayStart,
    ArrayEnd,
    DictionaryStart,
    DictionaryEnd,
    Word(String),
}

#[derive(Debug, Clone)]
pub(crate) struct LexicalToken {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: LexicalKind,
}

#[derive(Debug, Clone)]
struct PageContentSegment {
    stream_index: usize,
    object: u32,
    generation: u16,
    global_start: usize,
    global_end: usize,
}

#[derive(Debug, Clone)]
struct PageContentStringToken {
    stream_index: usize,
    object: u32,
    generation: u16,
    token: ContentStringToken,
}

pub fn analyze_same_width_patch(
    input: &[u8],
    page_number: usize,
    source_text: &str,
    replacement_text: &str,
    options: &SameWidthPatchOptions,
) -> Result<SameWidthPatchEligibilityReport> {
    crate::cancel::check_current_cancel("same-width patch analysis")?;
    validate_patch_options(options)?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::ContentEdit)?;
    let document = engine.document();
    let page = document.get_page(page_number)?;
    let resources = PageResources::from_dict(&page.resources, document.reader());
    let reader = document.reader();
    let mut candidates = Vec::new();
    let mut position_metrics = inline_text::Metrics::new(&resources, reader);
    for page_token in scan_page_text_string_tokens_with_metrics(
        reader,
        &page.contents,
        ScannedTextTokenState::default(),
        &mut position_metrics,
    )? {
        crate::cancel::check_current_cancel("same-width patch content stream")?;
        let PageContentStringToken {
            object: stream_number,
            generation: stream_generation,
            token,
            ..
        } = page_token;
        let object = reader.get_object(stream_number, stream_generation)?;
        let PdfObject::Stream { dict, .. } = object else {
            continue;
        };
        let Some(font_dict) = resources.fonts.get(&token.font_name) else {
            continue;
        };
        let resolver = FontResolver::new(font_dict, reader);
        if resolver
            .try_decode_string(&token.decoded)
            .map_err(WellfriendError::UnsupportedFeature)?
            != source_text
        {
            continue;
        }
        candidates.push(evaluate_patch_candidate(
            page_number,
            stream_number,
            stream_generation,
            &dict,
            &token,
            font_dict,
            &resolver,
            replacement_text,
            options,
            reader.is_encrypted(),
            token.unresolved_actual_text
                || !token.actual_text_sources.is_empty()
                || token_has_named_actual_text(&token, &resources, reader),
        ));
    }
    candidates.sort_by_key(|candidate| {
        (
            candidate.stream_object,
            candidate.stream_generation,
            candidate.decoded_byte_start,
        )
    });
    Ok(SameWidthPatchEligibilityReport {
        schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
        source_text: source_text.to_string(),
        replacement_text: replacement_text.to_string(),
        candidates,
        signature_policy,
        deterministic: true,
        exact_limits: vec![
            "only page-owned indirect content streams are patched; inline streams, object streams, and ambiguous inherited/Form contexts are reported unsupported".to_string(),
            "replacement Unicode must map uniquely through the existing font/CMap; no font substitution, shaping, bidi reorder, or vertical reorder is performed".to_string(),
            "incremental object replacement preserves the original PDF byte prefix but does not preserve cryptographic signature acceptance".to_string(),
        ],
    })
}

pub fn apply_same_width_patch(
    input: &[u8],
    page_number: usize,
    source_text: &str,
    replacement_text: &str,
    options: &SameWidthPatchOptions,
) -> Result<(Vec<u8>, SameWidthPatchApplyReport)> {
    let analysis =
        analyze_same_width_patch(input, page_number, source_text, replacement_text, options)?;
    enforce_advanced_editing_signature_policy(
        &analysis.signature_policy,
        options.signature_policy_override,
        "same-width content-stream patch",
    )?;
    crate::cancel::check_current_cancel("same-width patch apply")?;
    let selected = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.eligible && patch_candidate_is_selected(candidate, options))
        .cloned()
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "advanced_editing same-width patch has no eligible occurrence: {}",
                analysis
                    .candidates
                    .first()
                    .map(|candidate| candidate.exact_reason.as_str())
                    .unwrap_or("source text was not found in a supported page content string")
            ))
        })?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let page = engine.document().get_page(page_number)?;
    let resources = PageResources::from_dict(&page.resources, reader);
    let font_dict = resources
        .fonts
        .get(&selected.font_resource)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing selected patch font resource disappeared".to_string(),
            )
        })?;
    let resolver = FontResolver::new(font_dict, reader);
    let encoded = encode_with_existing_font(&resolver, replacement_text)?.0;
    let replacement_token = serialize_pdf_string(&encoded, selected.representation);
    let object = reader.get_object(selected.stream_object, selected.stream_generation)?;
    let PdfObject::Stream { mut dict, raw } = object else {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing selected content object is no longer a stream".to_string(),
        ));
    };
    let stream = PdfObject::Stream {
        dict: dict.clone(),
        raw,
    };
    let decoded_result = decode_stream_lossless_with_limits(
        &stream,
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    let mut decoded = match decoded_result.status {
        StreamDecodeStatus::Complete => decoded_result.data,
        StreamDecodeStatus::StoppedAtImageFilter(reason) => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "advanced_editing selected stream became undecodable: {reason}"
            )))
        }
    };
    if replacement_token.len()
        != selected
            .decoded_byte_end
            .saturating_sub(selected.decoded_byte_start)
        && options.require_same_serialized_length
    {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing selected serialized replacement length changed after eligibility analysis"
                .to_string(),
        ));
    }
    decoded.splice(
        selected.decoded_byte_start..selected.decoded_byte_end,
        replacement_token.clone(),
    );
    let compressed = flate_encode_cancellable(&decoded, 6)?;
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    crate::cancel::check_current_cancel("same-width patch serialization")?;
    let output = write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: selected.stream_object,
            generation: selected.stream_generation,
            object: PdfObject::Stream {
                dict,
                raw: compressed,
            },
        }],
    )?;
    crate::cancel::check_current_cancel("same-width patch reopen validation")?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let extracted = reopened.get_page_text(page_number)?;
    let selected_occurrence_rewritten = reopened
        .document()
        .reader()
        .get_object(selected.stream_object, selected.stream_generation)
        .ok()
        .and_then(|object| {
            decode_stream_lossless_with_limits(
                &object,
                reopened.document().reader(),
                &DecodeLimits {
                    max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                    ..DecodeLimits::default()
                },
            )
            .ok()
        })
        .filter(|decoded| decoded.status == StreamDecodeStatus::Complete)
        .and_then(|decoded| {
            decoded
                .data
                .get(
                    selected.decoded_byte_start
                        ..selected
                            .decoded_byte_start
                            .saturating_add(replacement_token.len()),
                )
                .map(|bytes| bytes == replacement_token.as_slice())
        })
        .unwrap_or(false);
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    let report = SameWidthPatchApplyReport {
        schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
        selected,
        original_bytes: input.len(),
        output_bytes: output.len(),
        rewritten_stream_bytes: decoded.len(),
        appended_revision_bytes: output.len().saturating_sub(input.len()),
        original_prefix_preserved: output.starts_with(input),
        output_reopened: true,
        replacement_extracts: extracted.contains(replacement_text),
        old_text_absent: if options.target_decoded_byte_range.is_some() {
            selected_occurrence_rewritten
        } else {
            !extracted.contains(source_text)
        },
        output_sha256,
        signature_policy: analysis.signature_policy,
        cryptographic_validity_claimed: false,
        deterministic: true,
        cache_invalidation: advanced_editing_cache_invalidation(input, &output, true, false, false),
    };
    if !report.original_prefix_preserved || !report.replacement_extracts || !report.old_text_absent
    {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing same-width patch failed reopen/extraction/prefix verification"
                .to_string(),
        ));
    }
    Ok((output, report))
}

fn patch_candidate_is_selected(
    candidate: &SameWidthPatchEligibility,
    options: &SameWidthPatchOptions,
) -> bool {
    options
        .target_stream_object
        .is_none_or(|number| candidate.stream_object == number)
        && options
            .target_stream_generation
            .is_none_or(|generation| candidate.stream_generation == generation)
        && options.target_decoded_byte_range.is_none_or(|range| {
            candidate.decoded_byte_start == range[0] && candidate.decoded_byte_end == range[1]
        })
}

fn validate_patch_options(options: &SameWidthPatchOptions) -> Result<()> {
    if !options.advance_tolerance_1000.is_finite() || options.advance_tolerance_1000 < 0.0 {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing patch advance tolerance must be finite and non-negative".to_string(),
        ));
    }
    if options.target_stream_object.is_some() != options.target_stream_generation.is_some() {
        return Err(WellfriendError::invalid_input(
            "advanced_editing selected source requires both stream object and generation",
        ));
    }
    if options
        .target_decoded_byte_range
        .is_some_and(|range| range[0] >= range[1])
    {
        return Err(WellfriendError::invalid_input(
            "advanced_editing selected source decoded byte range is empty or reversed",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn evaluate_patch_candidate(
    page: usize,
    stream_object: u32,
    stream_generation: u16,
    stream_dict: &crate::PdfDictionary,
    token: &ContentStringToken,
    font_dict: &crate::PdfDictionary,
    resolver: &FontResolver,
    replacement: &str,
    options: &SameWidthPatchOptions,
    encrypted: bool,
    logical_actual_text_conflict: bool,
) -> SameWidthPatchEligibility {
    let encoded = encode_with_existing_font(resolver, replacement);
    let (replacement_bytes, ambiguous) = encoded
        .as_ref()
        .map(|(bytes, ambiguous)| (bytes.clone(), *ambiguous))
        .unwrap_or_default();
    let before_codes = split_codes(resolver, &token.decoded);
    let after_codes = split_codes(resolver, &replacement_bytes);
    let code_sequences_valid =
        before_codes.is_ok() && after_codes.is_ok() && resolver.validate_source_encoding().is_ok();
    let before_codes = before_codes.unwrap_or_default();
    let after_codes = after_codes.unwrap_or_default();
    let before_advances = before_codes
        .iter()
        .map(|code| canonical_number(resolver.width_for_code(*code)))
        .collect::<Vec<_>>();
    let after_advances = after_codes
        .iter()
        .map(|code| canonical_number(resolver.width_for_code(*code)))
        .collect::<Vec<_>>();
    let before_total = before_advances.iter().sum::<f64>();
    let after_total = after_advances.iter().sum::<f64>();
    let delta = (before_total - after_total).abs();
    let before_serialized = token.token_end.saturating_sub(token.token_start);
    let replacement_serialized = serialize_pdf_string(&replacement_bytes, token.representation);
    let font_type = format!("{:?}", resolver.font_type());
    let clipping = matches!(token.text_render_mode, 4..=7);
    let filters = filter_names(stream_dict);
    let mut reason = "eligible".to_string();
    let mut eligible = true;
    let reject = |eligible: &mut bool, reason_slot: &mut String, message: &str| {
        if *eligible {
            *eligible = false;
            *reason_slot = message.to_string();
        }
    };
    if !code_sequences_valid {
        reject(
            &mut eligible,
            &mut reason,
            "source or replacement has invalid character codes or mappings",
        );
    }
    if encoded.is_err() {
        reject(
            &mut eligible,
            &mut reason,
            "replacement has no complete mapping through the existing font/CMap",
        );
    }
    if ambiguous {
        reject(
            &mut eligible,
            &mut reason,
            "replacement mapping is ambiguous in the existing font/CMap",
        );
    }
    if matches!(resolver.font_type(), FontType::Type3) {
        reject(
            &mut eligible,
            &mut reason,
            "Type3 CharProcs are unsupported for same-width patching",
        );
    }
    if resolver.is_vertical() {
        reject(
            &mut eligible,
            &mut reason,
            "vertical text requires vertical-order and metric analysis, not same-width patching",
        );
    }
    if contains_rtl_or_bidi_controls(replacement) {
        reject(
            &mut eligible,
            &mut reason,
            "replacement requires bidi analysis or visual reordering",
        );
    }
    if before_codes.len() != after_codes.len() {
        reject(&mut eligible, &mut reason, "glyph count changes");
    }
    if token.word_spacing != 0.0
        && before_codes
            .iter()
            .filter(|code| code.is_word_space())
            .count()
            != after_codes
                .iter()
                .filter(|code| code.is_word_space())
                .count()
    {
        reject(&mut eligible, &mut reason,
            "encoded word-spacing occurrences change; same glyph widths do not preserve the source endpoint");
    }
    if token.decoded.len() != replacement_bytes.len() {
        reject(&mut eligible, &mut reason, "encoded byte length changes");
    }
    if options.require_same_serialized_length && before_serialized != replacement_serialized.len() {
        reject(
            &mut eligible,
            &mut reason,
            "serialized PDF string length changes",
        );
    }
    let tolerance = match options.mode {
        SameWidthMode::Exact => 0.000_001,
        SameWidthMode::Tolerance => options.advance_tolerance_1000,
    };
    if delta > tolerance + EPSILON {
        reject(
            &mut eligible,
            &mut reason,
            "total glyph advance differs beyond configured tolerance",
        );
    }
    if clipping {
        reject(
            &mut eligible,
            &mut reason,
            "text render mode participates in clipping",
        );
    }
    if logical_actual_text_conflict {
        reject(
            &mut eligible,
            &mut reason,
            "surrounding /ActualText or a named marked-content property would retain stale logical text; use the page-logical multi-run writer",
        );
    }
    if encrypted {
        reject(
            &mut eligible,
            &mut reason,
            "encrypted incremental object replacement is unsupported",
        );
    }
    SameWidthPatchEligibility {
        schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
        status: if eligible {
            AdvancedEditingSupportStatus::ImplementedWithLimits
        } else {
            AdvancedEditingSupportStatus::UnsupportedReportedExact
        },
        eligible,
        page,
        stream_object,
        stream_generation,
        operator: token.operator.clone(),
        tj_element: token.element,
        decoded_byte_start: token.token_start,
        decoded_byte_end: token.token_end,
        representation: token.representation,
        font_resource: token.font_name.clone(),
        font_type,
        encoding: font_dict
            .get_name("Encoding")
            .unwrap_or("dictionary_or_builtin")
            .to_string(),
        cmap: font_dict
            .get_name("Encoding")
            .unwrap_or("simple_font_or_embedded_cmap")
            .to_string(),
        glyph_count_before: before_codes.len(),
        glyph_count_after: after_codes.len(),
        encoded_bytes_before: token.decoded.len(),
        encoded_bytes_after: replacement_bytes.len(),
        serialized_bytes_before: before_serialized,
        serialized_bytes_after: replacement_serialized.len(),
        glyph_advances_before: before_advances,
        glyph_advances_after: after_advances,
        total_advance_before: canonical_number(before_total),
        total_advance_after: canonical_number(after_total),
        advance_delta: canonical_number(delta),
        writing_mode: i32::from(resolver.is_vertical()),
        text_render_mode: token.text_render_mode,
        marked_content_depth: token.marked_depth,
        clipping_semantics: clipping,
        encrypted,
        filters,
        incremental_feasible: !encrypted,
        full_rewrite_feasible: !encrypted,
        exact_reason: reason,
    }
}

fn encode_with_existing_font(resolver: &FontResolver, text: &str) -> Result<(Vec<u8>, bool)> {
    resolver
        .try_encode_existing(text)
        .map_err(WellfriendError::UnsupportedFeature)
}
fn split_codes(
    resolver: &FontResolver,
    bytes: &[u8],
) -> Result<Vec<crate::fonts::character_code::CharacterCode>> {
    resolver
        .codes(bytes)
        .map(|item| {
            item.map(|item| item.code)
                .map_err(WellfriendError::UnsupportedFeature)
        })
        .collect()
}

fn serialize_pdf_string(bytes: &[u8], representation: PatchStringRepresentation) -> Vec<u8> {
    match representation {
        PatchStringRepresentation::Hexadecimal => {
            let mut output = Vec::with_capacity(bytes.len() * 2 + 2);
            output.push(b'<');
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            for byte in bytes {
                output.push(HEX[(byte >> 4) as usize]);
                output.push(HEX[(byte & 0x0f) as usize]);
            }
            output.push(b'>');
            output
        }
        PatchStringRepresentation::Literal => {
            let mut output = Vec::with_capacity(bytes.len() + 2);
            output.push(b'(');
            for byte in bytes {
                match byte {
                    b'(' | b')' | b'\\' => {
                        output.push(b'\\');
                        output.push(*byte);
                    }
                    b'\n' => output.extend_from_slice(b"\\n"),
                    b'\r' => output.extend_from_slice(b"\\r"),
                    b'\t' => output.extend_from_slice(b"\\t"),
                    0x08 => output.extend_from_slice(b"\\b"),
                    0x0c => output.extend_from_slice(b"\\f"),
                    _ => output.push(*byte),
                }
            }
            output.push(b')');
            output
        }
    }
}

fn source_byte_offset_for_scalar(
    resolver: &FontResolver,
    bytes: &[u8],
    target_scalar: usize,
) -> Result<usize> {
    resolver
        .validate_source_encoding()
        .map_err(WellfriendError::UnsupportedFeature)?;
    if target_scalar == 0 {
        return Ok(0);
    }
    let mut scalar_offset = 0usize;
    for decoded in resolver.codes(bytes) {
        let decoded = decoded.map_err(WellfriendError::UnsupportedFeature)?;
        let mapped_scalars = resolver.decode_code(decoded.code).chars().count();
        if mapped_scalars == 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing source CMap contains an empty mapping, so a scalar boundary cannot identify exact source bytes"
                    .to_string(),
            ));
        }
        let next = scalar_offset.checked_add(mapped_scalars).ok_or_else(|| {
            WellfriendError::ResourceLimit(
                "advanced_editing source scalar offset overflowed".to_string(),
            )
        })?;
        if target_scalar == next {
            return Ok(decoded.byte_end);
        }
        if target_scalar < next {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing selection boundary splits one source glyph/CMap mapping; provide a grapheme-complete source range"
                    .to_string(),
            ));
        }
        scalar_offset = next;
    }
    Err(WellfriendError::MalformedPdf(format!(
        "advanced_editing scalar boundary {target_scalar} exceeds the decoded source string length {scalar_offset}"
    )))
}

fn split_source_text_bytes_at_scalars(
    resolver: &FontResolver,
    bytes: &[u8],
    selected_start_scalar: usize,
    selected_end_scalar: usize,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    if selected_start_scalar > selected_end_scalar {
        return Err(WellfriendError::invalid_input(
            "advanced_editing selected scalar boundary is reversed",
        ));
    }
    let start = source_byte_offset_for_scalar(resolver, bytes, selected_start_scalar)?;
    let end = source_byte_offset_for_scalar(resolver, bytes, selected_end_scalar)?;
    if start > end || end > bytes.len() {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing selected source byte boundary is outside its string".to_string(),
        ));
    }
    Ok((
        bytes[..start].to_vec(),
        bytes[start..end].to_vec(),
        bytes[end..].to_vec(),
    ))
}

/// Split one selected source operand using its logical span when possible.
///
/// `/ActualText` and the SDK's zero-width logical carrier fonts can map an
/// arbitrary number of Unicode scalars to one physical source code. Asking
/// the source CMap to split that code at the logical scalar count is therefore
/// invalid even when the complete operand was selected. A complete logical
/// selection owns every byte in the operand and can be replaced atomically;
/// partial selections still use the strict CMap boundary check and fail closed
/// inside an indivisible mapping.
fn split_selected_source_operand(
    resolver: &FontResolver,
    bytes: &[u8],
    source_logical_range: [usize; 2],
    selected_logical_range: [usize; 2],
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    if selected_logical_range[0] <= source_logical_range[0]
        && selected_logical_range[1] >= source_logical_range[1]
    {
        return Ok((Vec::new(), bytes.to_vec(), Vec::new()));
    }
    split_source_text_bytes_at_scalars(
        resolver,
        bytes,
        selected_logical_range[0].saturating_sub(source_logical_range[0]),
        selected_logical_range[1].saturating_sub(source_logical_range[0]),
    )
}

fn append_serialized_text_show(
    output: &mut Vec<u8>,
    bytes: &[u8],
    representation: PatchStringRepresentation,
) {
    output.extend_from_slice(&serialize_pdf_string(bytes, representation));
    output.extend_from_slice(b" Tj\n");
}

fn materialize_decoded_stream_edits(
    stream_edits: DecodedStreamEdits,
    context: &str,
) -> Result<Vec<IncrementalObject>> {
    let mut updates = Vec::<IncrementalObject>::with_capacity(stream_edits.len());
    for ((number, generation), (source_object, mut source_data, mut edits)) in stream_edits {
        edits.sort_by_key(|edit| edit.0);
        for adjacent in edits.windows(2) {
            if adjacent[0].1 > adjacent[1].0 {
                return Err(WellfriendError::MalformedPdf(format!(
                    "{context} contains overlapping source patch ranges"
                )));
            }
        }
        for (start, end, replacement) in edits.into_iter().rev() {
            if start > end || end > source_data.len() {
                return Err(WellfriendError::MalformedPdf(format!(
                    "{context} patch range is outside its decoded stream"
                )));
            }
            source_data.splice(start..end, replacement);
        }
        let PdfObject::Stream {
            dict: mut source_dict,
            ..
        } = source_object
        else {
            return Err(WellfriendError::MalformedPdf(format!(
                "{context} is not a stream"
            )));
        };
        let source_compressed = flate_encode_cancellable(&source_data, 6)?;
        source_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
        source_dict.remove("DecodeParms");
        source_dict.insert("Length", PdfObject::Integer(source_compressed.len() as i64));
        updates.push(IncrementalObject {
            number,
            generation,
            object: PdfObject::Stream {
                dict: source_dict,
                raw: source_compressed,
            },
        });
    }
    Ok(updates)
}

/// Return the `TJ` numeric operand that advances by precisely the displacement
/// of `selected` under the source font and text state. PDF `TJ` numbers are in
/// thousandths of text space and are subtracted from the text position. The
/// calculation therefore removes the glyph codes without moving any following
/// text. Horizontal scaling cancels when converting a horizontal advance back
/// to `TJ` units; vertical writing uses W2/DW2 and is not horizontally scaled.
fn removed_source_advance_tj(
    token: &ContentStringToken,
    resolver: &FontResolver,
    selected: &[u8],
) -> Result<f64> {
    if selected.is_empty() {
        return Ok(0.0);
    }
    let displacement = inline_text::source_displacement(token, resolver, selected)?;
    let advance = displacement[usize::from(resolver.is_vertical())];
    let font_size = token.font_size;
    if font_size.abs() <= EPSILON {
        if advance.abs() > EPSILON {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing cannot express nonzero character spacing as TJ displacement when the source font size is zero"
                    .to_string(),
            ));
        }
        return Ok(0.0);
    }
    let denominator = if resolver.is_vertical() {
        font_size
    } else {
        font_size * (token.horizontal_scaling / 100.0)
    };
    if denominator.abs() <= EPSILON || !advance.is_finite() {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing cannot derive a finite source-advance compensation".to_string(),
        ));
    }
    Ok(canonical_number(-advance / denominator * 1000.0))
}

#[derive(Clone, Copy)]
enum SourceOrderCarrierGlyph<'a> {
    SourceEncoded(&'a [u8]),
    Generated { font_resource: &'a str, cid: u16 },
}

#[derive(Clone, Copy)]
struct SourceOrderActualTextCarrier<'a> {
    logical_text: &'a str,
    glyph: SourceOrderCarrierGlyph<'a>,
}

fn append_destructive_text_show_body(
    output: &mut Vec<u8>,
    token: &ContentStringToken,
    prefix: &[u8],
    selected: &[u8],
    suffix: &[u8],
    resolver: &FontResolver,
    retain_empty_carrier: bool,
    logical_actual_text: Option<SourceOrderActualTextCarrier<'_>>,
) -> Result<()> {
    let compensation = removed_source_advance_tj(token, resolver, selected)?;
    if !prefix.is_empty() {
        output.extend_from_slice(b"[");
        output.extend_from_slice(&serialize_pdf_string(prefix, token.representation));
        output.extend_from_slice(b"] TJ\n");
    }

    if let Some(carrier) = logical_actual_text {
        output.extend_from_slice(
            format!(
                "/Span << /ActualText <{}> >> BDC\n3 Tr\n",
                utf16be_hex_with_bom(carrier.logical_text),
            )
            .as_bytes(),
        );
        match carrier.glyph {
            SourceOrderCarrierGlyph::SourceEncoded(encoded) => {
                let replacement_displacement =
                    inline_text::source_displacement(token, resolver, encoded)?;
                let replacement_advance =
                    replacement_displacement[usize::from(resolver.is_vertical())];
                let denominator = if resolver.is_vertical() {
                    token.font_size
                } else {
                    token.font_size * (token.horizontal_scaling / 100.0)
                };
                if denominator.abs() <= EPSILON || !replacement_advance.is_finite() {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing cannot derive source-order carrier compensation"
                            .to_string(),
                    ));
                }
                let compensation =
                    canonical_number(compensation + replacement_advance / denominator * 1000.0);
                output.extend_from_slice(b"[");
                output.extend_from_slice(&serialize_pdf_string(encoded, token.representation));
                output.push(b' ');
                if compensation.abs() > EPSILON {
                    output.extend_from_slice(fmt_num(compensation).as_bytes());
                    output.push(b' ');
                }
                output.extend_from_slice(b"] TJ\n");
            }
            SourceOrderCarrierGlyph::Generated { font_resource, cid } => {
                output.extend_from_slice(
                    format!(
                        "0 Tc\n0 Tw\n100 Tz\n0 Ts\n/{} 0 Tf\n<{cid:04X}> Tj\n/{} {} Tf\n{} Tc\n{} Tw\n{} Tz\n{} Ts\n[",
                        serialized_name_body(font_resource),
                        serialized_name_body(&token.font_name),
                        fmt_num(token.font_size),
                        fmt_num(token.character_spacing),
                        fmt_num(token.word_spacing),
                        fmt_num(token.horizontal_scaling),
                        fmt_num(token.text_rise),
                    )
                    .as_bytes(),
                );
                if compensation.abs() > EPSILON {
                    output.extend_from_slice(fmt_num(compensation).as_bytes());
                    output.push(b' ');
                }
                output.extend_from_slice(b"] TJ\n");
            }
        }
        output.extend_from_slice(format!("{} Tr\nEMC\n", token.text_render_mode).as_bytes());
    } else {
        output.extend_from_slice(b"[");
        // A freshly authored typed cell must remain addressable when its value
        // is cleared. Preserve exactly one empty string operand inside the
        // existing owned scope; ordinary deletion keeps its no-carrier behavior.
        if retain_empty_carrier && prefix.is_empty() && suffix.is_empty() {
            output.extend_from_slice(&serialize_pdf_string(&[], token.representation));
            output.push(b' ');
        }
        if compensation.abs() > EPSILON {
            output.extend_from_slice(fmt_num(compensation).as_bytes());
            output.push(b' ');
        }
        output.extend_from_slice(b"] TJ\n");
    }

    if !suffix.is_empty() {
        output.extend_from_slice(b"[");
        output.extend_from_slice(&serialize_pdf_string(suffix, token.representation));
        output.extend_from_slice(b"] TJ\n");
    }
    Ok(())
}

/// Remove selected encoded glyph codes from a source text-showing operand while
/// retaining the exact writing-axis displacement through a numeric `TJ` item.
/// This is a current-revision source rewrite, not a visibility trick: none of
/// the selected bytes are serialized into the replacement operand.
fn rewrite_source_text_destructively(
    token: &ContentStringToken,
    resolver: &FontResolver,
    prefix: &[u8],
    selected: &[u8],
    suffix: &[u8],
    retain_empty_carrier: bool,
    logical_actual_text: Option<SourceOrderActualTextCarrier<'_>>,
) -> Result<(usize, usize, Vec<u8>)> {
    let mut body = Vec::new();
    append_destructive_text_show_body(
        &mut body,
        token,
        prefix,
        selected,
        suffix,
        resolver,
        retain_empty_carrier,
        logical_actual_text,
    )?;
    match token.operator.as_str() {
        "Tj" => Ok((token.operation_start, token.operation_end, body)),
        "'" => {
            let mut replacement = b"T*\n".to_vec();
            replacement.extend_from_slice(&body);
            Ok((token.operation_start, token.operation_end, replacement))
        }
        "\"" => {
            let mut replacement = format!(
                "{} Tw\n{} Tc\nT*\n",
                fmt_num(token.word_spacing),
                fmt_num(token.character_spacing)
            )
            .into_bytes();
            replacement.extend_from_slice(&body);
            Ok((token.operation_start, token.operation_end, replacement))
        }
        "TJ" => {
            let mut replacement = b"] TJ\n".to_vec();
            replacement.extend_from_slice(&body);
            replacement.extend_from_slice(b"[\n");
            Ok((token.token_start, token.token_end, replacement))
        }
        other => Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing destructive source replacement does not support text operator {other}"
        ))),
    }
}

/// Replace selected source codes inside their original text object.  Keeping
/// clipping text at this source position is essential: PDF text clipping is
/// committed by `ET` and affects later painting in that same graphics-state
/// scope.  The trailing TJ adjustment makes the following text position match
/// the source even when the replacement has a different advance.
fn rewrite_source_text_inline(
    token: &ContentStringToken,
    resolver: &FontResolver,
    prefix: &[u8],
    selected: &[u8],
    replacement: &[u8],
    suffix: &[u8],
) -> Result<(usize, usize, Vec<u8>)> {
    let old_advance = removed_source_advance_tj(token, resolver, selected)?;
    let new_advance = removed_source_advance_tj(token, resolver, replacement)?;
    let compensation = canonical_number(old_advance - new_advance);
    let mut body = Vec::new();
    body.extend_from_slice(b"[");
    if !prefix.is_empty() {
        body.extend_from_slice(&serialize_pdf_string(prefix, token.representation));
        body.push(b' ');
    }
    if !replacement.is_empty() {
        body.extend_from_slice(&serialize_pdf_string(replacement, token.representation));
        body.push(b' ');
    }
    if compensation.abs() > EPSILON {
        body.extend_from_slice(fmt_num(compensation).as_bytes());
        body.push(b' ');
    }
    if !suffix.is_empty() {
        body.extend_from_slice(&serialize_pdf_string(suffix, token.representation));
        body.push(b' ');
    }
    body.extend_from_slice(b"] TJ\n");

    match token.operator.as_str() {
        "Tj" => Ok((token.operation_start, token.operation_end, body)),
        "'" => {
            let mut rewritten = b"T*\n".to_vec();
            rewritten.extend_from_slice(&body);
            Ok((token.operation_start, token.operation_end, rewritten))
        }
        "\"" => {
            let mut rewritten = format!(
                "{} Tw\n{} Tc\nT*\n",
                fmt_num(token.word_spacing),
                fmt_num(token.character_spacing)
            )
            .into_bytes();
            rewritten.extend_from_slice(&body);
            Ok((token.operation_start, token.operation_end, rewritten))
        }
        "TJ" => {
            let mut rewritten = b"] TJ\n".to_vec();
            rewritten.extend_from_slice(&body);
            rewritten.extend_from_slice(b"[\n");
            Ok((token.token_start, token.token_end, rewritten))
        }
        other => Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing inline clipping replacement does not support text operator {other}"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
fn rewrite_source_text_inline_generated(
    token: &ContentStringToken,
    paint: &ContentStringToken,
    resolver: &FontResolver,
    prefix: &[u8],
    selected: &[u8],
    replacement_text: &str,
    glyphs: &[GeneratedGlyph],
    generated_font_resource: &str,
    generated_vertical: bool,
    paint_vertical: bool,
    suffix: &[u8],
) -> Result<(usize, usize, Vec<u8>)> {
    resolver
        .validate_source_encoding()
        .map_err(WellfriendError::UnsupportedFeature)?;
    inline_text::rewrite_positioned(
        token,
        paint,
        resolver,
        prefix,
        selected,
        replacement_text,
        glyphs,
        generated_font_resource,
        generated_vertical,
        paint_vertical,
        suffix,
    )
}

/// Rewrite one source text-showing operand without changing the text matrix.
/// The selected source codes remain in their original operator position so
/// character/word spacing, vertical metrics, and every following operand keep
/// their original advance.  Replacement Unicode is carried once by
/// `/ActualText`; the selected codes can be made non-painting while still
/// advancing exactly as before.
/// Keep a zero-width insertion anchored to one existing text-showing operand.
/// No source selection exists in this case, so the original carrier remains
/// visible and advances normally; `/ActualText` only adds the inserted logical
/// Unicode at the caller-selected boundary. Nonempty replacement ranges never
/// use this helper and always remove their selected current-revision codes.
fn rewrite_source_insertion_anchor_with_actual_text(
    token: &ContentStringToken,
    prefix: &[u8],
    carrier: &[u8],
    suffix: &[u8],
    actual_text: &str,
) -> Result<(usize, usize, Vec<u8>)> {
    let mut body = Vec::new();
    append_serialized_text_show(&mut body, prefix, token.representation);
    body.extend_from_slice(
        format!(
            "/Span << /ActualText <{}> >> BDC\n{} Tr\n",
            utf16be_hex_with_bom(actual_text),
            token.text_render_mode
        )
        .as_bytes(),
    );
    append_serialized_text_show(&mut body, carrier, token.representation);
    body.extend_from_slice(format!("{} Tr\nEMC\n", token.text_render_mode).as_bytes());
    append_serialized_text_show(&mut body, suffix, token.representation);

    match token.operator.as_str() {
        "Tj" => Ok((token.operation_start, token.operation_end, body)),
        "'" => {
            let mut replacement = b"T*\n".to_vec();
            replacement.extend_from_slice(&body);
            Ok((token.operation_start, token.operation_end, replacement))
        }
        "\"" => {
            let mut replacement = format!(
                "{} Tw\n{} Tc\nT*\n",
                fmt_num(token.word_spacing),
                fmt_num(token.character_spacing)
            )
            .into_bytes();
            replacement.extend_from_slice(&body);
            Ok((token.operation_start, token.operation_end, replacement))
        }
        "TJ" => {
            let mut replacement = b"] TJ\n".to_vec();
            replacement.extend_from_slice(&body);
            replacement.extend_from_slice(b"[\n");
            Ok((token.token_start, token.token_end, replacement))
        }
        other => Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing source-order replacement does not support text operator {other}"
        ))),
    }
}

fn wrap_generated_visual_as_artifact(content: String) -> String {
    format!("/Artifact << /ActualText <FEFF> >> BDC\n{content}\nEMC\n")
}

fn wrap_generated_visual_with_actual_text(content: String, logical_text: &str) -> String {
    format!(
        "/Span << /ActualText <{}> >> BDC\n{content}\nEMC\n",
        utf16be_hex_with_bom(logical_text)
    )
}

#[cfg(test)]
fn scan_text_string_tokens(data: &[u8]) -> Result<Vec<ContentStringToken>> {
    let mut state = ScannedTextTokenState::default();
    scan_text_string_tokens_with_state(data, &mut state)
}

#[cfg(test)]
fn scan_text_string_tokens_with_state(
    data: &[u8],
    state: &mut ScannedTextTokenState,
) -> Result<Vec<ContentStringToken>> {
    scan_text_string_tokens_with_state_and_owner(data, state, None)
}

#[cfg(test)]
fn scan_text_string_tokens_with_state_and_owner(
    data: &[u8],
    state: &mut ScannedTextTokenState,
    owner: Option<(u32, u16)>,
) -> Result<Vec<ContentStringToken>> {
    scan_text_string_tokens_with_metrics(data, state, owner, None)
}

fn scan_text_string_tokens_with_metrics(
    data: &[u8],
    state: &mut ScannedTextTokenState,
    owner: Option<(u32, u16)>,
    metrics: Option<&mut inline_text::Metrics<'_>>,
) -> Result<Vec<ContentStringToken>> {
    scan_text_program(data, state, owner, metrics, None)
}

#[derive(Clone, Debug)]
struct TextFormInvocationState {
    name: String,
    start: usize,
    end: usize,
    state: ScannedTextTokenState,
}

fn active_authored_typed_owner(
    stack: &[Option<(String, String)>],
) -> Result<Option<(String, String)>> {
    let mut active = None;
    for owner in stack.iter().flatten() {
        if active.as_ref().is_some_and(|current| current != owner) {
            return Err(WellfriendError::MalformedPdf(
                "nested authored typed table/cell ownership conflicts".into(),
            ));
        }
        active = Some(owner.clone());
    }
    Ok(active)
}

fn active_authored_typed_region(stack: &[Option<[f64; 4]>]) -> Result<Option<[f64; 4]>> {
    let mut active = None;
    for region in stack.iter().flatten() {
        if active.is_some_and(|current| current != *region) {
            return Err(WellfriendError::MalformedPdf(
                "nested authored typed-cell regions conflict".into(),
            ));
        }
        active = Some(*region);
    }
    Ok(active)
}

fn scan_text_program(
    data: &[u8],
    state: &mut ScannedTextTokenState,
    owner: Option<(u32, u16)>,
    metrics: Option<&mut inline_text::Metrics<'_>>,
    invocations: Option<&mut Vec<TextFormInvocationState>>,
) -> Result<Vec<ContentStringToken>> {
    let tokens = lex_content(data)?;
    scan_text_program_tokens(data, tokens, state, owner, None, metrics, invocations)
}

fn page_segment_for_range(
    segments: &[PageContentSegment],
    start: usize,
    end: usize,
) -> Option<&PageContentSegment> {
    segments
        .iter()
        .find(|segment| start >= segment.global_start && end <= segment.global_end)
}

#[allow(clippy::too_many_arguments)]
fn scan_text_program_tokens(
    data: &[u8],
    tokens: Vec<LexicalToken>,
    state: &mut ScannedTextTokenState,
    owner: Option<(u32, u16)>,
    page_segments: Option<&[PageContentSegment]>,
    mut metrics: Option<&mut inline_text::Metrics<'_>>,
    mut invocations: Option<&mut Vec<TextFormInvocationState>>,
) -> Result<Vec<ContentStringToken>> {
    let mut output = Vec::new();
    let mut operands = Vec::<LexicalToken>::new();
    let ScannedTextTokenState {
        mut font_name,
        mut font_size,
        mut render_mode,
        mut character_spacing,
        mut word_spacing,
        mut horizontal_scaling,
        mut text_rise,
        mut fill_color_command,
        mut stroke_color_command,
        mut fill_color_space_command,
        mut stroke_color_space_command,
        mut unsupported_fill_paint_state,
        mut unsupported_stroke_paint_state,
        mut marked_depth,
        mut authored_typed_owner_stack,
        mut authored_typed_region_stack,
        mut actual_text_stack,
        mut named_property_stack,
        mut actual_text_conflict_stack,
        mut flow_scope_stack,
        mut graphics_stack,
        mut position,
    } = std::mem::take(state);
    for (token_index, token) in tokens.into_iter().enumerate() {
        if token_index % 256 == 0 {
            crate::cancel::check_current_cancel("advanced content token scan")?;
        }
        let LexicalKind::Word(operator) = &token.kind else {
            operands.push(token);
            continue;
        };
        let first_output = output.len();
        match operator.as_str() {
            "q" => graphics_stack.push(ScannedTextGraphicsState {
                font_name: font_name.clone(),
                font_size,
                render_mode,
                character_spacing,
                word_spacing,
                horizontal_scaling,
                text_rise,
                fill_color_command: fill_color_command.clone(),
                stroke_color_command: stroke_color_command.clone(),
                fill_color_space_command: fill_color_space_command.clone(),
                stroke_color_space_command: stroke_color_space_command.clone(),
                unsupported_fill_paint_state,
                unsupported_stroke_paint_state,
            }),
            "Q" => {
                if let Some(restored) = graphics_stack.pop() {
                    font_name = restored.font_name;
                    font_size = restored.font_size;
                    render_mode = restored.render_mode;
                    character_spacing = restored.character_spacing;
                    word_spacing = restored.word_spacing;
                    horizontal_scaling = restored.horizontal_scaling;
                    text_rise = restored.text_rise;
                    fill_color_command = restored.fill_color_command;
                    stroke_color_command = restored.stroke_color_command;
                    fill_color_space_command = restored.fill_color_space_command;
                    stroke_color_space_command = restored.stroke_color_space_command;
                    unsupported_fill_paint_state = restored.unsupported_fill_paint_state;
                    unsupported_stroke_paint_state = restored.unsupported_stroke_paint_state;
                }
            }
            "Tf" => {
                if let Some(name) = operands
                    .iter()
                    .rev()
                    .find_map(|operand| match &operand.kind {
                        LexicalKind::Name(name) => Some(name.clone()),
                        _ => None,
                    })
                {
                    font_name = name;
                }
                if let Some(size) = operands
                    .iter()
                    .rev()
                    .find_map(|operand| match operand.kind {
                        LexicalKind::Number(number) => Some(number),
                        _ => None,
                    })
                {
                    font_size = size;
                }
            }
            "gs" => {
                match metrics
                    .as_ref()
                    .ok_or(())
                    .and_then(|m| m.ext_font(&operands))
                {
                    Ok(Some((name, size))) => {
                        font_name = name;
                        font_size = size;
                    }
                    Ok(None) => {}
                    Err(()) => {
                        font_name.clear();
                        font_size = 0.0;
                    }
                }
            }
            "Tc" => {
                if let Some(value) = operands
                    .iter()
                    .rev()
                    .find_map(|operand| match operand.kind {
                        LexicalKind::Number(number) => Some(number),
                        _ => None,
                    })
                {
                    character_spacing = value;
                }
            }
            "Tw" => {
                if let Some(value) = operands
                    .iter()
                    .rev()
                    .find_map(|operand| match operand.kind {
                        LexicalKind::Number(number) => Some(number),
                        _ => None,
                    })
                {
                    word_spacing = value;
                }
            }
            "Tz" => {
                if let Some(value) = operands
                    .iter()
                    .rev()
                    .find_map(|operand| match operand.kind {
                        LexicalKind::Number(number) => Some(number),
                        _ => None,
                    })
                {
                    horizontal_scaling = value;
                }
            }
            "Ts" => {
                if let Some(value) = operands
                    .iter()
                    .rev()
                    .find_map(|operand| match operand.kind {
                        LexicalKind::Number(number) => Some(number),
                        _ => None,
                    })
                {
                    text_rise = value;
                }
            }
            "Tr" => {
                if let Some(number) = operands
                    .iter()
                    .rev()
                    .find_map(|operand| match operand.kind {
                        LexicalKind::Number(number) => Some(number),
                        _ => None,
                    })
                {
                    render_mode = number as i32;
                }
            }
            "g" | "rg" | "k" => {
                let values = operands
                    .iter()
                    .filter_map(|operand| match operand.kind {
                        LexicalKind::Number(number) => Some(fmt_num(number)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if !values.is_empty() {
                    fill_color_command = format!("{} {operator}", values.join(" "));
                    fill_color_space_command.clear();
                    unsupported_fill_paint_state = false;
                }
            }
            "G" | "RG" | "K" => {
                let values = operands
                    .iter()
                    .filter_map(|operand| match operand.kind {
                        LexicalKind::Number(number) => Some(fmt_num(number)),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if !values.is_empty() {
                    stroke_color_command = format!("{} {operator}", values.join(" "));
                    stroke_color_space_command.clear();
                    unsupported_stroke_paint_state = false;
                }
            }
            "cs" => {
                let start = operands
                    .first()
                    .map(|operand| operand.start)
                    .unwrap_or(token.start);
                fill_color_space_command =
                    String::from_utf8_lossy(data.get(start..token.end).ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "advanced_editing fill paint command is outside its source stream"
                                .to_string(),
                        )
                    })?)
                    .trim()
                    .to_string();
                fill_color_command = fill_color_space_command.clone();
                unsupported_fill_paint_state = fill_color_command.is_empty();
            }
            "sc" | "scn" => {
                let start = operands
                    .first()
                    .map(|operand| operand.start)
                    .unwrap_or(token.start);
                let value_command =
                    String::from_utf8_lossy(data.get(start..token.end).ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "advanced_editing fill paint command is outside its source stream"
                                .to_string(),
                        )
                    })?)
                    .trim()
                    .to_string();
                fill_color_command = if fill_color_space_command.is_empty() {
                    value_command
                } else {
                    format!("{} {}", fill_color_space_command, value_command)
                };
                unsupported_fill_paint_state = fill_color_command.is_empty();
            }
            "CS" => {
                let start = operands
                    .first()
                    .map(|operand| operand.start)
                    .unwrap_or(token.start);
                stroke_color_space_command =
                    String::from_utf8_lossy(data.get(start..token.end).ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "advanced_editing stroke paint command is outside its source stream"
                                .to_string(),
                        )
                    })?)
                    .trim()
                    .to_string();
                stroke_color_command = stroke_color_space_command.clone();
                unsupported_stroke_paint_state = stroke_color_command.is_empty();
            }
            "SC" | "SCN" => {
                let start = operands
                    .first()
                    .map(|operand| operand.start)
                    .unwrap_or(token.start);
                let value_command =
                    String::from_utf8_lossy(data.get(start..token.end).ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "advanced_editing stroke paint command is outside its source stream"
                                .to_string(),
                        )
                    })?)
                    .trim()
                    .to_string();
                stroke_color_command = if stroke_color_space_command.is_empty() {
                    value_command
                } else {
                    format!("{} {}", stroke_color_space_command, value_command)
                };
                unsupported_stroke_paint_state = stroke_color_command.is_empty();
            }
            "BMC" | "BDC" => {
                const MAX_MARKED_CONTENT_DEPTH: usize = 4096;
                marked_depth = marked_depth.checked_add(1).ok_or_else(|| {
                    WellfriendError::ResourceLimit(
                        "advanced_editing marked-content depth overflowed".to_string(),
                    )
                })?;
                if marked_depth > MAX_MARKED_CONTENT_DEPTH {
                    return Err(WellfriendError::ResourceLimit(format!(
                        "advanced_editing marked-content depth exceeds {MAX_MARKED_CONTENT_DEPTH}"
                    )));
                }
                let authored_typed_owner = if operator == "BDC" {
                    let table = marked_property(&operands, "WFTableID")?;
                    let cell = marked_property(&operands, "WFCellID")?;
                    match (table, cell) {
                        (None, None) => None,
                        (
                            Some(LexicalToken {
                                kind: LexicalKind::String(_, table),
                                ..
                            }),
                            Some(LexicalToken {
                                kind: LexicalKind::String(_, cell),
                                ..
                            }),
                        ) => {
                            let table = crate::info::decode_pdf_text_string(table);
                            let cell = crate::info::decode_pdf_text_string(cell);
                            if table.is_empty()
                                || table.len() > 16 * 1024
                                || table.contains('\0')
                                || cell.is_empty()
                                || cell.len() > 16 * 1024
                                || cell.contains('\0')
                            {
                                return Err(WellfriendError::MalformedPdf(
                                    "invalid authored typed table/cell owner".into(),
                                ));
                            }
                            Some((table, cell))
                        }
                        _ => {
                            return Err(WellfriendError::MalformedPdf(
                                "authored typed table/cell owners must be paired direct strings"
                                    .into(),
                            ));
                        }
                    }
                } else {
                    None
                };
                let authored_typed_region = if operator == "BDC" {
                    let values = ["WFLeft", "WFBottom", "WFRight", "WFTop"]
                        .map(|key| marked_property(&operands, key))
                        .into_iter()
                        .collect::<Result<Vec<_>>>()?;
                    if values.iter().all(Option::is_none) {
                        None
                    } else if values.iter().all(Option::is_some) {
                        let mut region = [0.0; 4];
                        for (index, value) in values.into_iter().enumerate() {
                            region[index] = match value.map(|value| &value.kind) {
                                Some(LexicalKind::Number(value)) if value.is_finite() => *value,
                                _ => {
                                    return Err(WellfriendError::MalformedPdf(
                                        "authored typed-cell region must contain four direct numbers"
                                            .into(),
                                    ));
                                }
                            };
                        }
                        if region[0] >= region[2] || region[1] >= region[3] {
                            return Err(WellfriendError::MalformedPdf(
                                "authored typed-cell region is empty or reversed".into(),
                            ));
                        }
                        Some(region)
                    } else {
                        return Err(WellfriendError::MalformedPdf(
                            "authored typed-cell region properties must be complete".into(),
                        ));
                    }
                } else {
                    None
                };
                if authored_typed_region.is_some() && authored_typed_owner.is_none() {
                    return Err(WellfriendError::MalformedPdf(
                        "authored typed-cell region has no paired source owner".into(),
                    ));
                }
                authored_typed_owner_stack.push(authored_typed_owner);
                authored_typed_region_stack.push(authored_typed_region);
                let actual_text_value = if operator == "BDC" {
                    marked_property(&operands, "ActualText")?
                } else {
                    None
                };
                let actual_text_source = if operator == "BDC" {
                    actual_text_value.and_then(|value| match &value.kind {
                        LexicalKind::String(_, bytes) => {
                            let (owner_object, owner_generation, value_start, value_end) =
                                if let Some(segments) = page_segments {
                                    let segment =
                                        page_segment_for_range(segments, value.start, value.end)?;
                                    (
                                        segment.object,
                                        segment.generation,
                                        value.start - segment.global_start,
                                        value.end - segment.global_start,
                                    )
                                } else {
                                    let (owner_object, owner_generation) = owner?;
                                    (owner_object, owner_generation, value.start, value.end)
                                };
                            Some(ActualTextSource {
                                owner_object,
                                owner_generation,
                                value_start,
                                value_end,
                                logical_text: Arc::<str>::from(
                                    crate::info::decode_pdf_text_string(bytes),
                                ),
                            })
                        }
                        _ => None,
                    })
                } else {
                    None
                };
                let has_actual_text_key = actual_text_value.is_some();
                flow_scope_stack.push(operator == "BDC" && has_actual_text_key
                    && marked_property(&operands, "MCID")?.is_none()
                    && matches!(operands.first().map(|t| &t.kind), Some(LexicalKind::Name(tag)) if tag == "Span"));
                // In a PDF dictionary, an entry whose value is the null object
                // is semantically equivalent to an absent entry. The canonical
                // editor deliberately rewrites stale direct /ActualText values
                // to `null`; a later mutation in the same batch must therefore
                // treat that value as cleared, not as an unresolved carrier.
                let actual_text_is_explicit_null = matches!(
                    actual_text_value.map(|value| &value.kind),
                    Some(LexicalKind::Null)
                );
                actual_text_conflict_stack.push(
                    has_actual_text_key
                        && actual_text_source.is_none()
                        && !actual_text_is_explicit_null,
                );
                actual_text_stack.push(actual_text_source);
                let named_property = if operator == "BDC" {
                    operands.get(1).and_then(|operand| match &operand.kind {
                        LexicalKind::Name(name) => Some(name.clone()),
                        _ => None,
                    })
                } else {
                    None
                };
                named_property_stack.push(named_property);
            }
            "EMC" => {
                marked_depth = marked_depth.saturating_sub(1);
                let _ = authored_typed_owner_stack.pop();
                let _ = authored_typed_region_stack.pop();
                let _ = actual_text_stack.pop();
                let _ = named_property_stack.pop();
                let _ = actual_text_conflict_stack.pop();
                let _ = flow_scope_stack.pop();
            }
            "Tj" | "'" | "\"" => {
                let valid_operands = if operator == "\"" {
                    operands.len() == 3
                        && matches!(&operands[0].kind, LexicalKind::Number(_))
                        && matches!(&operands[1].kind, LexicalKind::Number(_))
                        && matches!(&operands[2].kind, LexicalKind::String(_, _))
                } else {
                    operands.len() == 1 && matches!(&operands[0].kind, LexicalKind::String(_, _))
                };
                if !valid_operands {
                    return Err(WellfriendError::MalformedPdf(format!(
                        "advanced_editing text operator {operator} has a non-canonical operand sequence"
                    )));
                }
                if operator == "\"" {
                    let values = operands
                        .iter()
                        .filter_map(|operand| match operand.kind {
                            LexicalKind::Number(value) => Some(value),
                            _ => None,
                        })
                        .collect::<Vec<_>>();
                    if values.len() >= 2 {
                        word_spacing = values[values.len() - 2];
                        character_spacing = values[values.len() - 1];
                    }
                }
                if let Some(string) =
                    operands
                        .iter()
                        .rev()
                        .find_map(|operand| match &operand.kind {
                            LexicalKind::String(representation, decoded) => {
                                Some((operand, *representation, decoded))
                            }
                            _ => None,
                        })
                {
                    let authored_typed_owner =
                        active_authored_typed_owner(&authored_typed_owner_stack)?;
                    let authored_typed_region =
                        active_authored_typed_region(&authored_typed_region_stack)?;
                    output.push(ContentStringToken {
                        operation_start: operands
                            .first()
                            .map(|operand| operand.start)
                            .unwrap_or(string.0.start),
                        operation_end: token.end,
                        token_start: string.0.start,
                        token_end: string.0.end,
                        representation: string.1,
                        decoded: string.2.clone(),
                        font_name: font_name.clone(),
                        font_size,
                        character_spacing,
                        word_spacing,
                        horizontal_scaling,
                        text_rise,
                        fill_color_command: fill_color_command.clone(),
                        stroke_color_command: stroke_color_command.clone(),
                        unsupported_fill_paint_state,
                        unsupported_stroke_paint_state,
                        operator: operator.clone(),
                        element: None,
                        source_position: None,
                        generated_basis: inline_text::SourceBasis::Absent,
                        text_render_mode: render_mode,
                        marked_depth,
                        authored_typed_owner,
                        authored_typed_region,
                        actual_text_sources: actual_text_stack
                            .iter()
                            .filter_map(Clone::clone)
                            .collect(),
                        named_marked_properties: named_property_stack
                            .iter()
                            .filter_map(Clone::clone)
                            .collect(),
                        unresolved_actual_text: actual_text_conflict_stack
                            .iter()
                            .any(|value| *value),
                        flow_relocatable: flow_scope_stack.iter().all(|value| *value),
                    });
                }
            }
            "TJ" => {
                let valid_array = operands.len() >= 2
                    && matches!(
                        operands.first().map(|item| &item.kind),
                        Some(LexicalKind::ArrayStart)
                    )
                    && matches!(
                        operands.last().map(|item| &item.kind),
                        Some(LexicalKind::ArrayEnd)
                    )
                    && operands[1..operands.len() - 1].iter().all(|operand| {
                        matches!(
                            &operand.kind,
                            LexicalKind::String(_, _) | LexicalKind::Number(_)
                        )
                    });
                if !valid_array {
                    return Err(WellfriendError::MalformedPdf(
                        "advanced_editing TJ operator has a non-canonical text array".to_string(),
                    ));
                }
                let mut element = 0usize;
                for operand in &operands {
                    if let LexicalKind::String(representation, decoded) = &operand.kind {
                        let authored_typed_owner =
                            active_authored_typed_owner(&authored_typed_owner_stack)?;
                        let authored_typed_region =
                            active_authored_typed_region(&authored_typed_region_stack)?;
                        output.push(ContentStringToken {
                            operation_start: operands
                                .first()
                                .map(|operand| operand.start)
                                .unwrap_or(operand.start),
                            operation_end: token.end,
                            token_start: operand.start,
                            token_end: operand.end,
                            representation: *representation,
                            decoded: decoded.clone(),
                            font_name: font_name.clone(),
                            font_size,
                            character_spacing,
                            word_spacing,
                            horizontal_scaling,
                            text_rise,
                            fill_color_command: fill_color_command.clone(),
                            stroke_color_command: stroke_color_command.clone(),
                            unsupported_fill_paint_state,
                            unsupported_stroke_paint_state,
                            operator: operator.clone(),
                            element: Some(element),
                            source_position: None,
                            generated_basis: inline_text::SourceBasis::Absent,
                            text_render_mode: render_mode,
                            marked_depth,
                            authored_typed_owner,
                            authored_typed_region,
                            actual_text_sources: actual_text_stack
                                .iter()
                                .filter_map(Clone::clone)
                                .collect(),
                            named_marked_properties: named_property_stack
                                .iter()
                                .filter_map(Clone::clone)
                                .collect(),
                            unresolved_actual_text: actual_text_conflict_stack
                                .iter()
                                .any(|value| *value),
                            flow_relocatable: flow_scope_stack.iter().all(|value| *value),
                        });
                        element += 1;
                    }
                }
            }
            _ => {}
        }
        position.observe(
            operator,
            &operands,
            &mut output[first_output..],
            inline_text::Parameters {
                font_name: &font_name,
                font_size,
                horizontal_scaling,
            },
            metrics.as_deref_mut(),
        )?;
        if operator == "Do" {
            if let Some(invocations) = invocations.as_deref_mut() {
                let [LexicalToken {
                    kind: LexicalKind::Name(name),
                    start,
                    ..
                }] = operands.as_slice()
                else {
                    return Err(WellfriendError::MalformedPdf(
                        "Form invocation needs one resource name".into(),
                    ));
                };
                if invocations.len() >= MAX_ADVANCED_EDITING_BIDI_RUNS {
                    return Err(WellfriendError::ResourceLimit(
                        "Form invocation scan budget exceeded".into(),
                    ));
                }
                invocations.push(TextFormInvocationState {
                    name: name.clone(),
                    start: *start,
                    end: token.end,
                    state: ScannedTextTokenState {
                        font_name: font_name.clone(),
                        font_size,
                        render_mode,
                        character_spacing,
                        word_spacing,
                        horizontal_scaling,
                        text_rise,
                        fill_color_command: fill_color_command.clone(),
                        stroke_color_command: stroke_color_command.clone(),
                        fill_color_space_command: fill_color_space_command.clone(),
                        stroke_color_space_command: stroke_color_space_command.clone(),
                        unsupported_fill_paint_state,
                        unsupported_stroke_paint_state,
                        marked_depth,
                        authored_typed_owner_stack: authored_typed_owner_stack.clone(),
                        authored_typed_region_stack: authored_typed_region_stack.clone(),
                        actual_text_stack: actual_text_stack.clone(),
                        named_property_stack: named_property_stack.clone(),
                        actual_text_conflict_stack: actual_text_conflict_stack.clone(),
                        flow_scope_stack: flow_scope_stack.clone(),
                        graphics_stack: Vec::new(), // implicit Form save/restore boundary
                        position: position.clone(),
                    },
                });
            }
        }
        // Invocation discovery needs the updated cursor/state, not a retained
        // copy of every text operand preceding a Form call.
        if invocations.is_some() {
            output.clear();
        }
        operands.clear();
    }
    *state = ScannedTextTokenState {
        font_name,
        font_size,
        render_mode,
        character_spacing,
        word_spacing,
        horizontal_scaling,
        text_rise,
        fill_color_command,
        stroke_color_command,
        fill_color_space_command,
        stroke_color_space_command,
        unsupported_fill_paint_state,
        unsupported_stroke_paint_state,
        marked_depth,
        authored_typed_owner_stack,
        authored_typed_region_stack,
        actual_text_stack,
        named_property_stack,
        actual_text_conflict_stack,
        flow_scope_stack,
        graphics_stack,
        position,
    };
    Ok(output)
}

/// Page `/Contents` arrays are one logical content sequence. Producers are
/// allowed to split that sequence inside arrays, dictionaries, strings, and
/// marked-content operands, so tokenizing each member independently rejects
/// valid PDFs. Decode all transparent members, tokenize the logical sequence
/// once, then map every editable string back to its exact physical owner.
fn scan_page_text_string_tokens_with_metrics(
    reader: &crate::PdfReader,
    contents: &[(u32, u16)],
    initial_state: ScannedTextTokenState,
    metrics: &mut inline_text::Metrics<'_>,
) -> Result<Vec<PageContentStringToken>> {
    let mut decoded_members = Vec::with_capacity(contents.len());
    let mut all_complete = true;
    let mut combined_bytes = 0usize;
    for (stream_index, (object, generation)) in contents.iter().copied().enumerate() {
        let source = reader.get_object(object, generation)?;
        let decoded = decode_stream_lossless_with_limits(
            &source,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        all_complete &= decoded.status == StreamDecodeStatus::Complete;
        combined_bytes = combined_bytes
            .checked_add(decoded.data.len())
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                WellfriendError::ResourceLimit(
                    "page content sequence byte length overflowed".to_string(),
                )
            })?;
        decoded_members.push((stream_index, object, generation, decoded));
    }

    if !all_complete {
        let mut output = Vec::new();
        let mut state = initial_state;
        for (stream_index, object, generation, decoded) in decoded_members {
            if decoded.status != StreamDecodeStatus::Complete {
                state = ScannedTextTokenState::default();
                continue;
            }
            output.extend(
                scan_text_string_tokens_with_metrics(
                    &decoded.data,
                    &mut state,
                    Some((object, generation)),
                    Some(metrics),
                )?
                .into_iter()
                .map(|token| PageContentStringToken {
                    stream_index,
                    object,
                    generation,
                    token,
                }),
            );
        }
        return Ok(output);
    }

    const MAX_PAGE_CONTENT_SEQUENCE_BYTES: usize = 512 * 1024 * 1024;
    if combined_bytes > MAX_PAGE_CONTENT_SEQUENCE_BYTES {
        return Err(WellfriendError::ResourceLimit(
            "page content sequence exceeds 512 MiB".to_string(),
        ));
    }
    let mut data = Vec::with_capacity(combined_bytes);
    let mut segments = Vec::with_capacity(decoded_members.len());
    for (stream_index, object, generation, decoded) in decoded_members {
        let global_start = data.len();
        data.extend_from_slice(&decoded.data);
        let global_end = data.len();
        segments.push(PageContentSegment {
            stream_index,
            object,
            generation,
            global_start,
            global_end,
        });
        // The PDF content-array semantics include a token boundary between
        // members; a line feed is a safe canonical separator for tokenization.
        data.push(b'\n');
    }

    let tokens = lex_content(&data)?;
    let mut state = initial_state;
    let scanned = scan_text_program_tokens(
        &data,
        tokens,
        &mut state,
        None,
        Some(&segments),
        Some(metrics),
        None,
    )?;
    scanned
        .into_iter()
        .map(|mut token| {
            let segment = page_segment_for_range(&segments, token.token_start, token.token_end)
                .ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "editable PDF string crosses a physical /Contents stream boundary"
                            .to_string(),
                    )
                })?;
            let operation_segment =
                page_segment_for_range(&segments, token.operation_start, token.operation_end);
            let operation_is_local = operation_segment
                .is_some_and(|operation| operation.stream_index == segment.stream_index);
            if !operation_is_local {
                // A /Contents array is one logical content stream.  In valid
                // PDFs a TJ array may start in one physical member and finish
                // in another.  TJ rewrites in this module patch the selected
                // string operand, not the enclosing array/operator, so a
                // string wholly owned by one member remains exactly editable.
                // Other text-showing operators are replaced as a whole and
                // must therefore remain physically local.
                if token.operator != "TJ" {
                    return Err(WellfriendError::UnsupportedFeature(
                        "editable PDF text-showing operation crosses a physical /Contents stream boundary"
                            .to_string(),
                    ));
                }
                token.operation_start = token.token_start;
                token.operation_end = token.token_end;
            }
            token.token_start -= segment.global_start;
            token.token_end -= segment.global_start;
            token.operation_start -= segment.global_start;
            token.operation_end -= segment.global_start;
            Ok(PageContentStringToken {
                stream_index: segment.stream_index,
                object: segment.object,
                generation: segment.generation,
                token,
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
struct PreservedStyledRun {
    text: String,
    encoded: Vec<u8>,
    style: PreservedTextStyle,
    advance: f64,
}

#[derive(Debug, Clone)]
struct PreservedStyleSpan {
    byte_start: usize,
    byte_end: usize,
    style: PreservedTextStyle,
}

fn preserved_style_from_token(token: &ContentStringToken) -> Result<PreservedTextStyle> {
    if !token.font_size.is_finite()
        || token.font_size <= 0.0
        || !token.character_spacing.is_finite()
        || !token.word_spacing.is_finite()
        || !token.horizontal_scaling.is_finite()
        || token.horizontal_scaling <= 0.0
        || !token.text_rise.is_finite()
    {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing preserve_per_segment rejects an invalid source text state"
                .to_string(),
        ));
    }
    if !(0..=7).contains(&token.text_render_mode) {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing preserve_per_segment rejects an unsupported source text rendering mode"
                .to_string(),
        ));
    }
    let uses_stroke = matches!(token.text_render_mode, 1 | 2 | 5 | 6);
    if token.unsupported_fill_paint_state || (uses_stroke && token.unsupported_stroke_paint_state) {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing preserve_per_segment is missing the exact source paint command"
                .to_string(),
        ));
    }
    Ok(PreservedTextStyle {
        font_resource: token.font_name.clone(),
        font_size: token.font_size,
        character_spacing: token.character_spacing,
        word_spacing: token.word_spacing,
        horizontal_scaling: token.horizontal_scaling,
        text_rise: token.text_rise,
        text_render_mode: token.text_render_mode,
        fill_color_command: token.fill_color_command.clone(),
        stroke_color_command: token.stroke_color_command.clone(),
        vertical: false,
    })
}

fn scalar_boundary_byte(text: &str, scalars: usize) -> Option<usize> {
    if scalars == 0 {
        return Some(0);
    }
    text.char_indices()
        .nth(scalars)
        .map(|(offset, _)| offset)
        .or_else(|| (text.chars().count() == scalars).then_some(text.len()))
}

fn is_grapheme_boundary(text: &str, byte_offset: usize) -> bool {
    byte_offset == 0
        || byte_offset == text.len()
        || text
            .grapheme_indices(true)
            .any(|(offset, _)| offset == byte_offset)
}

fn preserved_run_advance(
    resolver: &FontResolver,
    encoded: &[u8],
    _text: &str,
    style: &PreservedTextStyle,
) -> Result<f64> {
    let codes = split_codes(resolver, encoded)?;
    let width = codes
        .iter()
        .copied()
        .map(|code| resolver.width_for_code(code))
        .sum::<f64>()
        / 1000.0
        * style.font_size;
    // Tj advances after every encoded character, including its last. These
    // runs are concatenated without resetting Tm; omitting the last Tc per
    // run makes the measured line narrower than its emitted endpoint.
    let character_count = codes.len() as f64;
    let word_count = codes.iter().filter(|code| code.is_word_space()).count() as f64;
    Ok(
        (width + character_count * style.character_spacing + word_count * style.word_spacing)
            * (style.horizontal_scaling / 100.0),
    )
}

fn preserved_style_line_x(
    alignment: GeneratedTextAlignment,
    rtl: bool,
    region: [f64; 4],
    width: f64,
) -> Result<f64> {
    let target_width = region[2] - region[0];
    if width > target_width + EPSILON {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing preserve_per_segment line exceeds its bounded region".to_string(),
        ));
    }
    Ok(match alignment {
        GeneratedTextAlignment::Left => region[0],
        GeneratedTextAlignment::Right => region[2] - width,
        GeneratedTextAlignment::Center => region[0] + (target_width - width) / 2.0,
        GeneratedTextAlignment::Start => {
            if rtl { region[2] - width } else { region[0] }
        }
        GeneratedTextAlignment::End => {
            if rtl { region[0] } else { region[2] - width }
        }
        GeneratedTextAlignment::Justify => {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing preserve_per_segment full justification is refused until style-boundary spacing adjustment has an exact source-state proof"
                    .to_string(),
            ))
        }
    })
}

fn generated_style_for_offset(
    spans: &[PreservedStyleSpan],
    byte_offset: usize,
) -> Result<&PreservedTextStyle> {
    spans
        .iter()
        .find(|span| span.byte_start <= byte_offset && byte_offset < span.byte_end)
        .or_else(|| spans.last())
        .map(|span| &span.style)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing generated style map is empty".to_string(),
            )
        })
}

fn append_generated_preserved_style(
    content: &mut String,
    font_resource: &str,
    style: &PreservedTextStyle,
) {
    content.push_str(&format!(
        "/{} {} Tf\n{} Tc\n{} Tw\n{} Tz\n{} Ts\n{} Tr\n",
        font_resource,
        fmt_num(style.font_size),
        fmt_num(style.character_spacing),
        fmt_num(style.word_spacing),
        fmt_num(style.horizontal_scaling),
        fmt_num(style.text_rise),
        style.text_render_mode,
    ));
    content.push_str(&style.fill_color_command);
    content.push('\n');
    if matches!(style.text_render_mode, 1 | 2 | 5 | 6) {
        content.push_str(&style.stroke_color_command);
        content.push('\n');
    }
}

/// Serialize shaped visual glyphs while retaining the source style owner of
/// each complete replacement grapheme.  Glyphs are positioned individually so
/// HarfBuzz offsets, bidi visual order, different font sizes, rises, scaling,
/// and paint states can coexist without flattening a mixed-style selection.
fn serialize_generated_preserved_styles(
    layout: &[Vec<GeneratedGlyph>],
    style_spans: &[PreservedStyleSpan],
    font_resource: &str,
    options: &AdvancedTextEditOptions,
    vertical: bool,
    logical_actual_text: Option<&str>,
) -> Result<(String, Vec<GeneratedLineAdjustment>)> {
    let mut content = String::from("q\n");
    if let Some(logical_actual_text) = logical_actual_text {
        content.push_str(&format!(
            "/Span << /ActualText <{}> >> BDC\nBT\n",
            utf16be_hex_with_bom(logical_actual_text)
        ));
    } else {
        content.push_str("/Artifact BMC\nBT\n");
    }
    let mut adjustments = Vec::with_capacity(layout.len());
    if vertical {
        adjustments = vertical_text::serialize(
            &mut content,
            layout,
            font_resource,
            options,
            None,
            Some(style_spans),
        )?;
    } else {
        let default_line_advance = style_spans
            .iter()
            .map(|span| span.style.font_size)
            .fold(options.font_size, f64::max)
            * options.line_spacing;
        for (line_index, glyphs) in layout.iter().enumerate() {
            let styled = glyphs
                .iter()
                .map(|glyph| {
                    Ok((
                        glyph,
                        generated_style_for_offset(style_spans, glyph.logical_byte_start)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let advances = styled
                .iter()
                .map(|(glyph, style)| {
                    let horizontal_scale = style.horizontal_scaling / 100.0;
                    (glyph.advance.abs() / 1000.0 * style.font_size
                        + style.character_spacing
                        + if glyph.visual_unicode == " " {
                            style.word_spacing
                        } else {
                            0.0
                        })
                        * horizontal_scale
                })
                .collect::<Vec<_>>();
            let natural_width = advances.iter().sum::<f64>();
            let target_width = options.region[2] - options.region[0];
            if natural_width > target_width + EPSILON {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing generated mixed-style line exceeds its bounded region"
                        .to_string(),
                ));
            }
            let last_line = line_index + 1 == layout.len();
            let should_justify = options.alignment == GeneratedTextAlignment::Justify
                && (options.justify_last_line || !last_line)
                && !styled.is_empty();
            let mut residual = target_width - natural_width;
            let word_count = styled
                .iter()
                .filter(|(glyph, _)| glyph.visual_unicode == " ")
                .count();
            let character_count = styled.len().saturating_sub(1);
            let mut word_extra = 0.0;
            let mut character_extra = 0.0;
            if should_justify && residual > EPSILON && word_count > 0 {
                let smallest_word_em = styled
                    .iter()
                    .filter(|(glyph, _)| glyph.visual_unicode == " ")
                    .map(|(_, style)| style.font_size)
                    .fold(f64::INFINITY, f64::min);
                word_extra =
                    (residual / word_count as f64).min(options.max_word_spacing * smallest_word_em);
                residual -= word_extra * word_count as f64;
            }
            if should_justify && residual > EPSILON && character_count > 0 {
                let smallest_em = styled
                    .iter()
                    .map(|(_, style)| style.font_size)
                    .fold(f64::INFINITY, f64::min);
                character_extra = (residual / character_count as f64)
                    .min(options.max_character_spacing * smallest_em);
                residual -= character_extra * character_count as f64;
            }
            if should_justify && residual > EPSILON {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing mixed-style justification exceeds configured spacing bounds"
                        .to_string(),
                ));
            }
            let rtl = styled.iter().any(|(glyph, _)| {
                glyph
                    .visual_unicode
                    .chars()
                    .any(|ch| matches!(ch as u32, 0x0590..=0x08FF | 0xFB1D..=0xFEFF))
            });
            let painted_width = natural_width
                + word_extra * word_count as f64
                + character_extra * character_count as f64;
            let mut x = match options.alignment {
                GeneratedTextAlignment::Left => options.region[0],
                GeneratedTextAlignment::Right => options.region[2] - painted_width,
                GeneratedTextAlignment::Center => {
                    options.region[0] + (target_width - painted_width) / 2.0
                }
                GeneratedTextAlignment::Start | GeneratedTextAlignment::Justify => {
                    if rtl {
                        options.region[2] - painted_width
                    } else {
                        options.region[0]
                    }
                }
                GeneratedTextAlignment::End => {
                    if rtl {
                        options.region[0]
                    } else {
                        options.region[2] - painted_width
                    }
                }
            };
            let y = options.region[3]
                - style_spans
                    .first()
                    .map(|span| span.style.font_size)
                    .unwrap_or(options.font_size)
                - line_index as f64 * default_line_advance;
            for (glyph_index, ((glyph, style), advance)) in
                styled.iter().zip(advances.iter()).enumerate()
            {
                append_generated_preserved_style(&mut content, font_resource, style);
                let scale = style.font_size / 1000.0;
                let positioned_x = x + glyph.offset_x * scale * style.horizontal_scaling / 100.0;
                let positioned_y = y + glyph.offset_y * scale;
                content.push_str(&format!(
                    "1 0 0 1 {} {} Tm <{:04X}> Tj\n",
                    fmt_num(positioned_x),
                    fmt_num(positioned_y),
                    glyph.cid
                ));
                x += *advance;
                if glyph.visual_unicode == " " {
                    x += word_extra;
                }
                if glyph_index + 1 < styled.len() {
                    x += character_extra;
                }
            }
            adjustments.push(GeneratedLineAdjustment {
                line_index,
                natural_width,
                target_width,
                residual: residual.max(0.0),
                word_spacing: word_extra,
                character_spacing: character_extra,
                alignment: options.alignment,
                last_line,
                applied: true,
                refusal_reason: None,
            });
        }
    }
    content.push_str("ET\nEMC\n");
    content.push('Q');
    Ok((content, adjustments))
}

fn serialize_preserved_styled_runs(
    runs_by_scalar: &[PreservedStyledRun],
    logical_lines: &[ExplicitLayoutLine],
    options: &AdvancedTextEditOptions,
    mode: AdvancedTextMode,
) -> Result<(String, Vec<GeneratedLineAdjustment>)> {
    if mode == AdvancedTextMode::ParagraphReflowVertical {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing preserve_per_segment positioned serializer does not support vertical writing"
                .to_string(),
        ));
    }
    let mut content = String::from("q\n");
    content.push_str("BT\n");
    if logical_lines.len() > horizontal_line_capacity(options)? {
        return Err(WellfriendError::UnsupportedFeature(
            "preserved-style text exceeds frame capacity; use linked-story flow".into(),
        ));
    }
    let mut adjustments = Vec::with_capacity(logical_lines.len());
    let mut run_index = 0usize;
    let line_advance = options.font_size * options.line_spacing;
    for (line_index, line) in logical_lines.iter().enumerate() {
        if line.inserted_visual_hyphen {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing preserve_per_segment refuses an inserted hyphen because an existing source CMap cannot prove its empty ToUnicode behavior"
                    .to_string(),
            ));
        }
        let visual_scalars = line
            .logical_text
            .trim_end_matches(crate::fonts::hard_break::is_hard_break)
            .chars()
            .count();
        let logical_scalars = line.logical_text.chars().count();
        let line_end = run_index.saturating_add(logical_scalars);
        if line_end > runs_by_scalar.len() {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing preserve_per_segment line mapping exceeds replacement provenance"
                    .to_string(),
            ));
        }
        let visual_end = run_index.saturating_add(visual_scalars);
        let visible_runs = &runs_by_scalar[run_index..visual_end];
        let natural_width = visible_runs.iter().map(|run| run.advance).sum::<f64>();
        let rtl = visible_runs.iter().any(|run| {
            run.text
                .chars()
                .any(|ch| matches!(ch as u32, 0x0590..=0x08FF | 0xFB1D..=0xFEFF))
        });
        let x = preserved_style_line_x(options.alignment, rtl, options.region, natural_width)?;
        let y = options.region[3] - options.font_size - line_index as f64 * line_advance;
        content.push_str(&format!("1 0 0 1 {} {} Tm\n", fmt_num(x), fmt_num(y)));
        let mut visible_index = 0usize;
        while visible_index < visible_runs.len() {
            let run = &visible_runs[visible_index];
            let style = &run.style;
            let mut group_end = visible_index + 1;
            while group_end < visible_runs.len() && visible_runs[group_end].style == *style {
                group_end += 1;
            }
            let group = &visible_runs[visible_index..group_end];
            content.push_str(&format!(
                "/{} {} Tf\n",
                serialized_name_body(&style.font_resource),
                fmt_num(style.font_size)
            ));
            if style.character_spacing.abs() > EPSILON {
                content.push_str(&format!("{} Tc\n", fmt_num(style.character_spacing)));
            }
            if style.word_spacing.abs() > EPSILON {
                content.push_str(&format!("{} Tw\n", fmt_num(style.word_spacing)));
            }
            if (style.horizontal_scaling - 100.0).abs() > EPSILON {
                content.push_str(&format!("{} Tz\n", fmt_num(style.horizontal_scaling)));
            }
            if style.text_rise.abs() > EPSILON {
                content.push_str(&format!("{} Ts\n", fmt_num(style.text_rise)));
            }
            if style.text_render_mode != 0 {
                content.push_str(&format!("{} Tr\n", style.text_render_mode));
            }
            content.push_str(&style.fill_color_command);
            content.push('\n');
            if matches!(style.text_render_mode, 1 | 2 | 5 | 6) {
                content.push_str(&style.stroke_color_command);
                content.push('\n');
            }
            content.push('<');
            for grouped_run in group {
                for byte in &grouped_run.encoded {
                    content.push_str(&format!("{byte:02X}"));
                }
            }
            content.push_str("> Tj\n");
            if style.character_spacing.abs() > EPSILON {
                content.push_str("0 Tc\n");
            }
            if style.word_spacing.abs() > EPSILON {
                content.push_str("0 Tw\n");
            }
            if (style.horizontal_scaling - 100.0).abs() > EPSILON {
                content.push_str("100 Tz\n");
            }
            if style.text_rise.abs() > EPSILON {
                content.push_str("0 Ts\n");
            }
            if style.text_render_mode != 0 {
                content.push_str("0 Tr\n");
            }
            visible_index = group_end;
        }
        adjustments.push(GeneratedLineAdjustment {
            line_index,
            natural_width,
            target_width: options.region[2] - options.region[0],
            residual: (options.region[2] - options.region[0] - natural_width).max(0.0),
            word_spacing: 0.0,
            character_spacing: 0.0,
            alignment: options.alignment,
            last_line: line_index + 1 == logical_lines.len(),
            applied: true,
            refusal_reason: None,
        });
        run_index = line_end;
    }
    if run_index != runs_by_scalar.len() {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing preserve_per_segment final lines did not consume exact replacement provenance"
                .to_string(),
        ));
    }
    content.push_str("ET\n");
    content.push('Q');
    Ok((content, adjustments))
}

pub(crate) fn lex_content(data: &[u8]) -> Result<Vec<LexicalToken>> {
    use crate::content::tokenizer::{ContentToken, ContentTokenizer};

    let mut tokens = Vec::new();
    let mut containers = Vec::new();
    let mut tokenizer = ContentTokenizer::new(data);
    while let Some(token) = tokenizer.next_spanned()? {
        if tokens.len() % 256 == 0 {
            crate::cancel::check_current_cancel("editing canonical tokenization")?;
        }
        let kind = match token.token {
            ContentToken::Null => LexicalKind::Null,
            ContentToken::Boolean(_) => LexicalKind::Boolean,
            ContentToken::InlineImageData(_) => LexicalKind::InlineImageData,
            ContentToken::LiteralString(bytes) => {
                LexicalKind::String(PatchStringRepresentation::Literal, bytes)
            }
            ContentToken::HexString(bytes) => {
                LexicalKind::String(PatchStringRepresentation::Hexadecimal, bytes)
            }
            ContentToken::Name(name) => LexicalKind::Name(name),
            ContentToken::Integer(value) => LexicalKind::Number(value as f64),
            ContentToken::Real(value) => LexicalKind::Number(value),
            ContentToken::ArrayStart => {
                containers.push(false);
                LexicalKind::ArrayStart
            }
            ContentToken::ArrayEnd => {
                if containers.pop() != Some(false) {
                    return Err(WellfriendError::MalformedPdf(
                        "unbalanced content array".into(),
                    ));
                }
                LexicalKind::ArrayEnd
            }
            ContentToken::DictStart => {
                containers.push(true);
                LexicalKind::DictionaryStart
            }
            ContentToken::DictEnd => {
                if containers.pop() != Some(true) {
                    return Err(WellfriendError::MalformedPdf(
                        "unbalanced content dictionary".into(),
                    ));
                }
                LexicalKind::DictionaryEnd
            }
            ContentToken::Operator(operator) if !containers.is_empty() => {
                if operator != "R" {
                    return Err(WellfriendError::MalformedPdf(
                        "executable operator inside a content object".into(),
                    ));
                }
                LexicalKind::ReferenceMarker
            }
            ContentToken::Operator(operator) => LexicalKind::Word(operator),
        };
        if containers.len() > 256 {
            return Err(WellfriendError::ResourceLimit(
                "content object nesting exceeds 256".into(),
            ));
        }
        if tokens.len() >= 2_000_000 {
            return Err(WellfriendError::ResourceLimit(
                "editing content-token budget exceeds 2000000".into(),
            ));
        }
        tokens.push(LexicalToken {
            start: token.start,
            end: token.end,
            kind,
        });
    }
    if !containers.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "unterminated content object".into(),
        ));
    }
    Ok(tokens)
}

fn serialized_name_body(name: &str) -> String {
    let mut bytes = Vec::new();
    crate::writer::serialize_object(&PdfObject::Name(name.to_string()), &mut bytes);
    String::from_utf8_lossy(&bytes[1..]).into_owned()
}

/// Locate only a direct member of the BDC property dictionary. Nested keys and
/// names used as values are not properties of the enclosing marked-content span.
pub(crate) fn marked_property<'a>(
    operands: &'a [LexicalToken],
    key: &str,
) -> Result<Option<&'a LexicalToken>> {
    if !matches!(
        operands.get(1).map(|t| &t.kind),
        Some(LexicalKind::DictionaryStart)
    ) {
        return Ok(None);
    }
    let mut index = 2;
    let mut found = None;
    while index < operands.len() {
        if matches!(operands[index].kind, LexicalKind::DictionaryEnd) {
            if index + 1 != operands.len() {
                return Err(WellfriendError::MalformedPdf(
                    "extra BDC dictionary operands".into(),
                ));
            }
            return Ok(found);
        }
        let LexicalKind::Name(name) = &operands[index].kind else {
            return Err(WellfriendError::MalformedPdf(
                "BDC property key is not a name".into(),
            ));
        };
        index += 1;
        let value = operands
            .get(index)
            .ok_or_else(|| WellfriendError::MalformedPdf("missing BDC property value".into()))?;
        if name == key {
            if found.is_some() {
                return Err(WellfriendError::MalformedPdf(
                    "duplicate BDC property key".into(),
                ));
            }
            found = Some(value);
        }
        let mut depth = 0usize;
        loop {
            match operands.get(index).map(|t| &t.kind) {
                Some(LexicalKind::DictionaryStart | LexicalKind::ArrayStart) => depth += 1,
                Some(LexicalKind::DictionaryEnd | LexicalKind::ArrayEnd) if depth > 0 => depth -= 1,
                Some(LexicalKind::DictionaryEnd | LexicalKind::ArrayEnd) | None => {
                    return Err(WellfriendError::MalformedPdf(
                        "missing BDC property value".into(),
                    ))
                }
                _ => {}
            }
            index += 1;
            if depth == 0 {
                break;
            }
        }
        // Object references are one value, not three dictionary members.
        if matches!(value.kind, LexicalKind::Number(_))
            && matches!(
                operands.get(index).map(|t| &t.kind),
                Some(LexicalKind::Number(_))
            )
            && matches!(
                operands.get(index + 1).map(|t| &t.kind),
                Some(LexicalKind::ReferenceMarker)
            )
        {
            index += 2;
        }
    }
    Err(WellfriendError::MalformedPdf(
        "unterminated BDC properties".into(),
    ))
}

fn contains_rtl_or_bidi_controls(text: &str) -> bool {
    text.chars().any(|ch| {
        matches!(
            ch as u32,
            0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x202A..=0x202E | 0x2066..=0x2069
        )
    })
}

fn filter_names(dict: &crate::PdfDictionary) -> Vec<String> {
    match dict.get("Filter") {
        Some(PdfObject::Name(name)) => vec![name.clone()],
        Some(PdfObject::Array(values)) => values
            .iter()
            .filter_map(PdfObject::as_name)
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn enforce_advanced_editing_signature_policy(
    policy: &EditPolicyReport,
    override_requested: bool,
    operation: &str,
) -> Result<()> {
    if matches!(
        policy.decision,
        EditPolicyDecision::BlockedBySignaturePolicy | EditPolicyDecision::ExplicitOverrideRequired
    ) && !override_requested
    {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing {operation} blocked by signature policy; explicit override required"
        )));
    }
    if policy.full_rewrite_required {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing {operation} requires full rewrite but this operation is structurally incremental"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VectorMatrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl VectorMatrix {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn multiply(self, rhs: Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            e: self.a * rhs.e + self.c * rhs.f + self.e,
            f: self.b * rhs.e + self.d * rhs.f + self.f,
        }
    }

    fn transform(self, point: InkPoint) -> InkPoint {
        InkPoint {
            x: self.a * point.x + self.c * point.y + self.e,
            y: self.b * point.x + self.d * point.y + self.f,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VectorPathSegment {
    MoveTo {
        point: InkPoint,
    },
    LineTo {
        point: InkPoint,
    },
    CubicTo {
        control1: InkPoint,
        control2: InkPoint,
        point: InkPoint,
    },
    Rectangle {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorPaintMode {
    Stroke,
    FillNonzero,
    FillEvenOdd,
    FillStrokeNonzero,
    FillStrokeEvenOdd,
    EndPath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorFillRule {
    Nonzero,
    EvenOdd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorColor {
    pub color_space: String,
    pub components: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorStrokeStyle {
    pub width: f64,
    pub dash: Vec<f64>,
    pub dash_phase: f64,
    pub cap: i32,
    pub join: i32,
    pub miter_limit: f64,
}

impl Default for VectorStrokeStyle {
    fn default() -> Self {
        Self {
            width: 1.0,
            dash: Vec::new(),
            dash_phase: 0.0,
            cap: 0,
            join: 0,
            miter_limit: 10.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorProvenance {
    pub page: usize,
    pub object_number: u32,
    pub generation: u16,
    pub content_stream_index: usize,
    pub operation_byte_start: usize,
    pub operation_byte_end: usize,
    pub form_stack: Vec<String>,
    pub marked_content_depth: usize,
    pub ocg_context: Option<String>,
    pub resource_owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_invocation: Option<VectorFormInvocation>,
    /// Ordered page-to-leaf invocation chain.  This is separate from the
    /// human-readable form_stack so clone-one never has to parse diagnostics.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form_invocation_path: Vec<VectorFormInvocation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wellfriendpdf_groups: Vec<VectorGroupProvenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VectorFormInvocation {
    pub resource_name: String,
    pub owner_stream_object: u32,
    pub owner_stream_generation: u16,
    pub owner_operation_byte_start: usize,
    pub owner_operation_byte_end: usize,
    pub form_object: u32,
    pub form_generation: u16,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorGroupProvenance {
    pub marker_start: usize,
    pub marker_end: usize,
    pub content_start: usize,
    pub content_end: usize,
    pub depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableVectorObject {
    pub schema_version: String,
    pub stable_id: String,
    pub provenance: VectorProvenance,
    pub bbox: [f64; 4],
    pub transform: VectorMatrix,
    pub segments: Vec<VectorPathSegment>,
    pub fill_rule: VectorFillRule,
    pub paint_mode: VectorPaintMode,
    pub stroke: VectorStrokeStyle,
    pub stroke_color: VectorColor,
    pub fill_color: VectorColor,
    pub opacity: f64,
    pub blend_mode: String,
    pub clipping_path: bool,
    pub clipping_context: bool,
    pub ext_g_state: Option<String>,
    pub confidence: f64,
    pub edit_safety: String,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VectorObjectInventory {
    pub schema_version: String,
    pub page: usize,
    pub objects: Vec<EditableVectorObject>,
    pub form_recursion_limit: usize,
    pub vector_object_limit: usize,
    pub deterministic: bool,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VectorEditOperation {
    Move {
        dx: f64,
        dy: f64,
    },
    Scale {
        sx: f64,
        sy: f64,
        origin: InkPoint,
    },
    Rotate {
        degrees: f64,
        origin: InkPoint,
    },
    Skew {
        x_degrees: f64,
        y_degrees: f64,
    },
    MirrorHorizontal {
        axis_x: f64,
    },
    MirrorVertical {
        axis_y: f64,
    },
    EditPoint {
        segment: usize,
        point: usize,
        value: InkPoint,
    },
    SetFill {
        color: VectorColor,
    },
    SetStroke {
        color: VectorColor,
    },
    SetStrokeWidth {
        width: f64,
    },
    SetDash {
        dash: Vec<f64>,
        phase: f64,
    },
    SetCapJoin {
        cap: i32,
        join: i32,
        miter_limit: f64,
    },
    SetOpacity {
        opacity: f64,
    },
    Delete,
    Duplicate {
        dx: f64,
        dy: f64,
    },
    BringForward,
    SendBackward,
    BringToFront,
    SendToBack,
    GroupWith {
        stable_ids: Vec<String>,
    },
    Ungroup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorEditOptions {
    pub signature_policy_override: bool,
    pub deterministic: bool,
    #[serde(default)]
    pub shared_form_policy: SharedFormEditPolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedFormEditPolicy {
    #[default]
    Reject,
    EditAllUses,
    CloneEditOneInstance,
}

impl Default for VectorEditOptions {
    fn default() -> Self {
        Self {
            signature_policy_override: false,
            deterministic: true,
            shared_form_policy: SharedFormEditPolicy::Reject,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VectorEditReport {
    pub schema_version: String,
    pub stable_id: String,
    pub operation: VectorEditOperation,
    pub before: EditableVectorObject,
    pub after: Option<EditableVectorObject>,
    pub source_range: [usize; 2],
    pub replacement_bytes: usize,
    pub unrelated_decoded_prefix_preserved: bool,
    pub unrelated_decoded_suffix_preserved: bool,
    pub original_pdf_prefix_preserved: bool,
    pub output_reopened: bool,
    pub output_sha256: String,
    pub signature_policy: EditPolicyReport,
    pub cryptographic_validity_claimed: bool,
    pub deterministic: bool,
    pub shared_form_policy: SharedFormEditPolicy,
    pub cloned_form: Option<[u32; 2]>,
    pub clone_graph: Vec<String>,
    pub cache_invalidation: CacheInvalidationReport,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone)]
struct RawContentOperation {
    start: usize,
    end: usize,
    operator: String,
    operands: Vec<LexicalKind>,
}

#[derive(Debug, Clone)]
struct VectorGraphicsState {
    matrix: VectorMatrix,
    stroke: VectorStrokeStyle,
    stroke_color: VectorColor,
    fill_color: VectorColor,
    opacity: f64,
    blend_mode: String,
    ext_g_state: Option<String>,
    clipping_context: bool,
    marked_depth: usize,
    ocg_context: Option<String>,
}

impl Default for VectorGraphicsState {
    fn default() -> Self {
        Self {
            matrix: VectorMatrix::IDENTITY,
            stroke: VectorStrokeStyle::default(),
            stroke_color: VectorColor {
                color_space: "DeviceGray".to_string(),
                components: vec![0.0],
            },
            fill_color: VectorColor {
                color_space: "DeviceGray".to_string(),
                components: vec![0.0],
            },
            opacity: 1.0,
            blend_mode: "Normal".to_string(),
            ext_g_state: None,
            clipping_context: false,
            marked_depth: 0,
            ocg_context: None,
        }
    }
}

pub fn list_vector_objects(input: &[u8], page_number: usize) -> Result<VectorObjectInventory> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let mut objects = Vec::new();
    for (stream_index, (number, generation)) in page.contents.iter().copied().enumerate() {
        let stream = reader.get_object(number, generation)?;
        let decoded = decode_stream_lossless_with_limits(
            &stream,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            continue;
        }
        objects.extend(reconstruct_vector_objects(
            &decoded.data,
            page_number,
            stream_index,
            number,
            generation,
        )?);
        collect_form_vector_objects(
            reader,
            &page.resources,
            &page.resources,
            &decoded.data,
            page_number,
            stream_index,
            number,
            generation,
            VectorMatrix::IDENTITY,
            &[],
            &[],
            &mut Vec::new(),
            &mut objects,
        )?;
        if objects.len() > 100_000 {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing vector object count exceeds limit 100000".to_string(),
            ));
        }
    }
    collect_annotation_appearance_vectors(
        reader,
        &page,
        page_number,
        page.contents.len(),
        &mut objects,
    )?;
    objects.sort_by_key(|object| {
        (
            object.provenance.content_stream_index,
            object.provenance.operation_byte_start,
            object.stable_id.clone(),
        )
    });
    Ok(VectorObjectInventory {
        schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
        page: page_number,
        objects,
        form_recursion_limit: 8,
        vector_object_limit: 100_000,
        deterministic: true,
        exact_limits: vec![
            "objects are reconstructed from actual path and paint operator ranges; unrelated paths are never fused into inferred semantic shapes".to_string(),
            "reachable Form XObject paths are inventoried to depth 8; clone-edit-one isolates supported page Contents occurrences and their nested Form invocation chains; stream-owned tag migration remains explicit".to_string(),
            "indirect annotation appearance paths are inventoried and editable by operation range; appearance streams shared by multiple annotations are diagnosed and rejected until explicitly cloned".to_string(),
            "patterns, shadings, and ExtGState names are retained as references; their internal programs are not converted into fake solid colors".to_string(),
        ],
    })
}

#[derive(Debug, Clone)]
struct ReachableFormUse {
    resource_name: String,
    object_number: u32,
    generation: u16,
    operation_start: usize,
    operation_end: usize,
    matrix: VectorMatrix,
}

#[allow(clippy::too_many_arguments)]
fn collect_form_vector_objects(
    reader: &crate::reader::PdfReader,
    page_resources: &crate::PdfDictionary,
    resources: &crate::PdfDictionary,
    owner_data: &[u8],
    page: usize,
    stream_index: usize,
    owner_number: u32,
    owner_generation: u16,
    parent_matrix: VectorMatrix,
    parent_stack: &[String],
    parent_invocations: &[VectorFormInvocation],
    active_forms: &mut Vec<(u32, u16)>,
    output: &mut Vec<EditableVectorObject>,
) -> Result<()> {
    if parent_stack.len() >= 8 {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing vector Form recursion exceeds limit 8".to_string(),
        ));
    }
    for form_use in reachable_form_uses(owner_data, resources, reader)? {
        if active_forms.contains(&(form_use.object_number, form_use.generation)) {
            return Err(WellfriendError::MalformedPdf(format!(
                "advanced_editing cyclic Form XObject graph reaches {} {} R",
                form_use.object_number, form_use.generation
            )));
        }
        let form_object = reader.get_object(form_use.object_number, form_use.generation)?;
        let PdfObject::Stream { dict, raw } = form_object else {
            continue;
        };
        if dict.get_name("Subtype") != Some("Form") {
            continue;
        }
        let decoded = decode_stream_lossless_with_limits(
            &PdfObject::Stream {
                dict: dict.clone(),
                raw,
            },
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            continue;
        }
        let form_matrix = pdf_matrix(dict.get("Matrix")).unwrap_or(VectorMatrix::IDENTITY);
        let effective_matrix = parent_matrix
            .multiply(form_use.matrix)
            .multiply(form_matrix);
        let mut form_stack = parent_stack.to_vec();
        form_stack.push(format!(
            "{}:{}:{}@{}..{}",
            form_use.resource_name,
            form_use.object_number,
            form_use.generation,
            form_use.operation_start,
            form_use.operation_end
        ));
        let invocation = VectorFormInvocation {
            resource_name: form_use.resource_name.clone(),
            owner_stream_object: owner_number,
            owner_stream_generation: owner_generation,
            owner_operation_byte_start: form_use.operation_start,
            owner_operation_byte_end: form_use.operation_end,
            form_object: form_use.object_number,
            form_generation: form_use.generation,
            depth: form_stack.len(),
        };
        let mut invocation_path = parent_invocations.to_vec();
        invocation_path.push(invocation.clone());
        let mut form_objects = reconstruct_vector_objects(
            &decoded.data,
            page,
            stream_index,
            form_use.object_number,
            form_use.generation,
        )?;
        for object in &mut form_objects {
            object.provenance.form_stack = form_stack.clone();
            object.provenance.resource_owner =
                format!("form-{}-{}", form_use.object_number, form_use.generation);
            object.provenance.form_invocation = Some(invocation.clone());
            object.provenance.form_invocation_path = invocation_path.clone();
            object.transform = effective_matrix.multiply(object.transform);
            object.bbox = vector_bbox(&object.segments, object.transform);
            object.stable_id = vector_stable_id_for_object(object);
        }
        output.extend(form_objects);
        let nested_resources = resolve_advanced_editing_dict(dict.get("Resources"), reader)
            .unwrap_or_else(|| page_resources.clone());
        active_forms.push((form_use.object_number, form_use.generation));
        collect_form_vector_objects(
            reader,
            page_resources,
            &nested_resources,
            &decoded.data,
            page,
            stream_index,
            form_use.object_number,
            form_use.generation,
            effective_matrix,
            &form_stack,
            &invocation_path,
            active_forms,
            output,
        )?;
        active_forms.pop();
    }
    Ok(())
}

fn reachable_form_uses(
    data: &[u8],
    resources: &crate::PdfDictionary,
    reader: &crate::reader::PdfReader,
) -> Result<Vec<ReachableFormUse>> {
    let Some(xobjects) = resolve_advanced_editing_dict(resources.get("XObject"), reader) else {
        return Ok(Vec::new());
    };
    let mut state = VectorMatrix::IDENTITY;
    let mut stack = Vec::new();
    let mut output = Vec::new();
    for operation in raw_content_operations(data)? {
        let numbers = operation_numbers(&operation.operands);
        match operation.operator.as_str() {
            "q" => stack.push(state),
            "Q" => state = stack.pop().unwrap_or(VectorMatrix::IDENTITY),
            "cm" if numbers.len() >= 6 => {
                state = state.multiply(VectorMatrix {
                    a: numbers[0],
                    b: numbers[1],
                    c: numbers[2],
                    d: numbers[3],
                    e: numbers[4],
                    f: numbers[5],
                });
            }
            "Do" => {
                let Some(name) = operation.operands.iter().find_map(|operand| match operand {
                    LexicalKind::Name(name) => Some(name.clone()),
                    _ => None,
                }) else {
                    continue;
                };
                let Some((number, generation)) =
                    xobjects.get(&name).and_then(PdfObject::as_reference)
                else {
                    continue;
                };
                let Ok(PdfObject::Stream { dict, .. }) = reader.get_object(number, generation)
                else {
                    continue;
                };
                if dict.get_name("Subtype") == Some("Form") {
                    output.push(ReachableFormUse {
                        resource_name: name,
                        object_number: number,
                        generation,
                        operation_start: operation.start,
                        operation_end: operation.end,
                        matrix: state,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(output)
}

fn resolve_advanced_editing_dict(
    object: Option<&PdfObject>,
    reader: &crate::reader::PdfReader,
) -> Option<crate::PdfDictionary> {
    match object? {
        PdfObject::Dictionary(dict) => Some(dict.clone()),
        reference @ PdfObject::Reference { .. } => {
            reader.resolve(reference.clone()).ok()?.as_dict().cloned()
        }
        _ => None,
    }
}

fn pdf_matrix(object: Option<&PdfObject>) -> Option<VectorMatrix> {
    let array = object?.as_array()?;
    if array.len() != 6 {
        return None;
    }
    let mut values = [0.0; 6];
    for (target, value) in values.iter_mut().zip(array) {
        *target = pdf_number(value)?;
    }
    Some(VectorMatrix {
        a: values[0],
        b: values[1],
        c: values[2],
        d: values[3],
        e: values[4],
        f: values[5],
    })
}

fn pdf_number(object: &PdfObject) -> Option<f64> {
    match object {
        PdfObject::Integer(value) => Some(*value as f64),
        PdfObject::Real(value) => Some(*value),
        _ => None,
    }
}

pub fn edit_vector_object(
    input: &[u8],
    page_number: usize,
    stable_id: &str,
    operation: VectorEditOperation,
    options: &VectorEditOptions,
) -> Result<(Vec<u8>, VectorEditReport)> {
    validate_vector_edit(&operation)?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::ContentEdit)?;
    enforce_advanced_editing_signature_policy(
        &signature_policy,
        options.signature_policy_override,
        "vector object edit",
    )?;
    let inventory = list_vector_objects(input, page_number)?;
    let inventory_objects = inventory.objects;
    let before = inventory_objects
        .iter()
        .find(|object| object.stable_id == stable_id)
        .cloned()
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "advanced_editing vector stable ID {stable_id} not found on page {page_number}"
            ))
        })?;
    if before.provenance.form_stack.len() > 8 {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing vector Form recursion exceeds limit 8".to_string(),
        ));
    }
    let form_invocation = before.provenance.form_invocation.clone();
    if before.edit_safety == "shared_annotation_appearance_requires_clone"
        && options.shared_form_policy != SharedFormEditPolicy::CloneEditOneInstance
    {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing annotation appearance stream is shared by multiple annotations; ownership-specific appearance cloning is required".to_string(),
        ));
    }
    if form_invocation.is_some() && options.shared_form_policy == SharedFormEditPolicy::Reject {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing vector is owned by a Form XObject; select shared_form_policy edit_all_uses or clone_edit_one_instance explicitly".to_string(),
        ));
    }
    if matches!(
        operation,
        VectorEditOperation::GroupWith { .. } | VectorEditOperation::Ungroup
    ) {
        return edit_vector_group_structure(
            input,
            &engine,
            inventory_objects,
            before,
            operation,
            options,
            signature_policy,
        );
    }
    if matches!(
        operation,
        VectorEditOperation::BringForward
            | VectorEditOperation::SendBackward
            | VectorEditOperation::BringToFront
            | VectorEditOperation::SendToBack
    ) {
        return edit_vector_z_order(
            input,
            &engine,
            inventory_objects,
            before,
            operation,
            options,
            signature_policy,
        );
    }
    let mut after = before.clone();
    let mut report_replacement_offset = 0usize;
    let replacement = match &operation {
        VectorEditOperation::Delete => Vec::new(),
        VectorEditOperation::Duplicate { dx, dy } => {
            apply_vector_transform(
                &mut after,
                VectorMatrix {
                    a: 1.0,
                    b: 0.0,
                    c: 0.0,
                    d: 1.0,
                    e: *dx,
                    f: *dy,
                },
            );
            let mut serializable_before = before.clone();
            serializable_before.transform = VectorMatrix::IDENTITY;
            let mut serializable_after = after.clone();
            serializable_after.transform = VectorMatrix {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: *dx,
                f: *dy,
            };
            let mut bytes = serialize_vector_object(&serializable_before);
            bytes.extend_from_slice(b"\n");
            report_replacement_offset = bytes.len();
            bytes.extend_from_slice(&serialize_vector_object(&serializable_after));
            bytes
        }
        VectorEditOperation::GroupWith { .. } | VectorEditOperation::Ungroup => {
            unreachable!("group operations are routed before range mutation")
        }
        _ => {
            mutate_vector(&mut after, &operation)?;
            let mut serializable = after.clone();
            serializable.transform =
                vector_edit_matrix(&operation).unwrap_or(VectorMatrix::IDENTITY);
            serialize_vector_object(&serializable)
        }
    };
    let reader = engine.document().reader();
    let stream_object = reader.get_object(
        before.provenance.object_number,
        before.provenance.generation,
    )?;
    let PdfObject::Stream { mut dict, raw } = stream_object else {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing vector provenance does not reference a stream".to_string(),
        ));
    };
    let decoded_result = decode_stream_lossless_with_limits(
        &PdfObject::Stream {
            dict: dict.clone(),
            raw,
        },
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    if decoded_result.status != StreamDecodeStatus::Complete {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing vector stream is not losslessly decodable".to_string(),
        ));
    }
    let mut decoded = decoded_result.data;
    let range = before.provenance.operation_byte_start..before.provenance.operation_byte_end;
    if range.end > decoded.len() || range.start > range.end {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing vector provenance range is outside decoded stream".to_string(),
        ));
    }
    let prefix = decoded[..range.start].to_vec();
    let suffix = decoded[range.end..].to_vec();
    decoded.splice(range.clone(), replacement.clone());
    let prefix_preserved = decoded.starts_with(&prefix);
    let suffix_preserved = decoded.ends_with(&suffix);
    let compressed = flate_encode_cancellable(&decoded, 6)?;
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    let mut changed = Vec::new();
    let mut cloned_form = None;
    let mut clone_graph = Vec::new();
    if options.shared_form_policy == SharedFormEditPolicy::CloneEditOneInstance
        && (form_invocation.is_some()
            || before.edit_safety == "shared_annotation_appearance_requires_clone")
    {
        let mut sources = BTreeSet::from([(
            before.provenance.object_number,
            before.provenance.generation,
        )]);
        sources.extend(
            before
                .provenance
                .form_invocation_path
                .iter()
                .map(|invocation| {
                    (
                        invocation.owner_stream_object,
                        invocation.owner_stream_generation,
                    )
                }),
        );
        vector_occurrence::check_clone_ownership(reader, &sources)?;
    }
    if before.edit_safety == "shared_annotation_appearance_requires_clone"
        && options.shared_form_policy == SharedFormEditPolicy::CloneEditOneInstance
    {
        let owner_value = before.provenance.resource_owner.clone();
        let source_appearance_object = before.provenance.object_number;
        let source_appearance_generation = before.provenance.generation;
        let owner = owner_value.as_str();
        let prefix = owner.strip_prefix("annotation-").ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing_closeout annotation appearance provenance is malformed"
                    .to_string(),
            )
        })?;
        let (annotation_index_text, appearance_tail) =
            prefix.split_once("-appearance-").ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "advanced_editing_closeout annotation appearance provenance is malformed"
                        .to_string(),
                )
            })?;
        let annotation_index: usize = annotation_index_text.parse().map_err(|_| {
            WellfriendError::MalformedPdf(
                "advanced_editing_closeout annotation appearance index is malformed".to_string(),
            )
        })?;
        let suffix = format!("-{source_appearance_object}-{source_appearance_generation}");
        let appearance_name = appearance_tail.strip_suffix(&suffix).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing_closeout annotation appearance identity is malformed".to_string(),
            )
        })?;
        let page = engine.document().get_page(page_number)?;
        let page_object = reader.get_object(page.object_number, page.generation_number)?;
        let page_dict = page_object.as_dict().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing_closeout page object is not a dictionary".to_string(),
            )
        })?;
        let annots = reader.resolve(page_dict.get("Annots").cloned().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing_closeout page has no annotations".to_string(),
            )
        })?)?;
        let annotation_ref = annots
            .as_array()
            .and_then(|items| items.get(annotation_index))
            .and_then(PdfObject::as_reference)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "advanced_editing_closeout clone-one requires an indirect target annotation"
                        .to_string(),
                )
            })?;
        let annotation_object = reader.get_object(annotation_ref.0, annotation_ref.1)?;
        let mut annotation_dict = annotation_object.as_dict().cloned().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing_closeout annotation is not a dictionary".to_string(),
            )
        })?;
        let mut ap =
            resolve_advanced_editing_dict(annotation_dict.get("AP"), reader).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "advanced_editing_closeout annotation AP dictionary is malformed".to_string(),
                )
            })?;
        let clone_number =
            next_advanced_object_number(reader, "advanced_editing annotation appearance clone")?;
        let parts = appearance_name.split('/').collect::<Vec<_>>();
        if parts.len() == 1 {
            ap.insert(
                parts[0],
                PdfObject::Reference {
                    number: clone_number,
                    generation: 0,
                },
            );
        } else if parts.len() == 2 {
            let mut states =
                resolve_advanced_editing_dict(ap.get(parts[0]), reader).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "advanced_editing_closeout appearance-state dictionary is malformed"
                            .to_string(),
                    )
                })?;
            if states.contains_key(parts[1]) {
                states.insert(
                    parts[1],
                    PdfObject::Reference {
                        number: clone_number,
                        generation: 0,
                    },
                );
            } else {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing_closeout requested appearance state does not exist"
                        .to_string(),
                ));
            }
            ap.insert(parts[0], PdfObject::Dictionary(states));
        } else {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing_closeout appearance category/state identity is malformed"
                    .to_string(),
            ));
        }
        annotation_dict.insert("AP", PdfObject::Dictionary(ap));
        changed.push(IncrementalObject {
            number: clone_number,
            generation: 0,
            object: PdfObject::Stream {
                dict,
                raw: compressed,
            },
        });
        changed.push(IncrementalObject {
            number: annotation_ref.0,
            generation: annotation_ref.1,
            object: PdfObject::Dictionary(annotation_dict),
        });
        let output = write_incremental_update(reader, changed)?;
        ContentEngine::open_bytes(output.clone())?;
        let report_after = if matches!(operation, VectorEditOperation::Delete) {
            None
        } else {
            Some(vector_occurrence::rebound(
                &output,
                &before,
                (clone_number, 0),
                range.start + report_replacement_offset,
                replacement.len() - report_replacement_offset,
            )?)
        };
        return Ok((output.clone(), VectorEditReport { schema_version:ADVANCED_EDITING_SCHEMA_VERSION.to_string(), stable_id:stable_id.to_string(), operation, before, after:report_after, source_range:[range.start,range.end], replacement_bytes:replacement.len(), unrelated_decoded_prefix_preserved:prefix_preserved, unrelated_decoded_suffix_preserved:suffix_preserved, original_pdf_prefix_preserved:output.starts_with(input), output_reopened:true, output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:options.deterministic, shared_form_policy:options.shared_form_policy, cloned_form:Some([clone_number,0]), clone_graph:vec![format!("annotation:{annotation_index} AP/{appearance_name} -> {clone_number} 0 R; shared source {source_appearance_object} {source_appearance_generation} R retained")], cache_invalidation:advanced_editing_cache_invalidation(input,&output,false,true,true), exact_limits:vec!["clone-one updates only the selected annotation AP category/state and preserves /AS plus sibling N/R/D state entries".to_string(),"nested Form resources inside an appearance use the Form invocation clone-one path; malformed or ambiguous state dictionaries fail closed".to_string()] }));
    }
    if options.shared_form_policy == SharedFormEditPolicy::CloneEditOneInstance
        && form_invocation.is_some()
    {
        if let Some(annotation_stack) = before
            .provenance
            .form_stack
            .first()
            .filter(|stack| stack.starts_with("annotation:"))
        {
            let annotation_tail =
                annotation_stack
                    .strip_prefix("annotation:")
                    .ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "advanced_editing_closeout annotation appearance stack is malformed"
                                .to_string(),
                        )
                    })?;
            let (annotation_index_text, appearance_name) =
                annotation_tail.split_once(":appearance:").ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "advanced_editing_closeout annotation appearance stack is malformed"
                            .to_string(),
                    )
                })?;
            let annotation_index: usize = annotation_index_text.parse().map_err(|_| {
                WellfriendError::MalformedPdf(
                    "advanced_editing_closeout annotation appearance index is malformed"
                        .to_string(),
                )
            })?;
            let invocation_path = before.provenance.form_invocation_path.clone();
            let source_appearance = invocation_path.first().ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "advanced_editing_closeout nested annotation appearance clone-one requires an invocation path"
                        .to_string(),
                )
            })?;
            let source_appearance_number = source_appearance.owner_stream_object;
            let source_appearance_generation = source_appearance.owner_stream_generation;
            let page = engine.document().get_page(page_number)?;
            let page_object = reader.get_object(page.object_number, page.generation_number)?;
            let page_dict = page_object.as_dict().ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "advanced_editing_closeout page object is not a dictionary".to_string(),
                )
            })?;
            let annots = reader.resolve(page_dict.get("Annots").cloned().ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "advanced_editing_closeout page has no annotations".to_string(),
                )
            })?)?;
            let annotation_ref = annots
                .as_array()
                .and_then(|items| items.get(annotation_index))
                .and_then(PdfObject::as_reference)
                .ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "advanced_editing_closeout clone-one requires an indirect target annotation".to_string(),
                    )
                })?;
            let annotation_object = reader.get_object(annotation_ref.0, annotation_ref.1)?;
            let mut annotation_dict = annotation_object.as_dict().cloned().ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "advanced_editing_closeout annotation is not a dictionary".to_string(),
                )
            })?;
            let mut ap = resolve_advanced_editing_dict(annotation_dict.get("AP"), reader)
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "advanced_editing_closeout annotation AP dictionary is malformed"
                            .to_string(),
                    )
                })?;

            let clone_count = u32::try_from(invocation_path.len())
                .ok()
                .and_then(|count| count.checked_add(1))
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit(
                        "advanced_editing annotation clone path is too deep to allocate"
                            .to_string(),
                    )
                })?;
            let allocation_base = reserve_advanced_object_block(
                reader,
                clone_count,
                "advanced_editing nested annotation appearance clone",
            )?;
            let leaf_number = allocation_base;
            changed.push(IncrementalObject {
                number: leaf_number,
                generation: 0,
                object: PdfObject::Stream {
                    dict,
                    raw: compressed,
                },
            });
            let mut child_number = leaf_number;
            let mut cloned_appearance = None;
            for (clone_index, invocation) in invocation_path.iter().rev().enumerate() {
                let owner_object = reader.get_object(
                    invocation.owner_stream_object,
                    invocation.owner_stream_generation,
                )?;
                let PdfObject::Stream {
                    dict: mut owner_dict,
                    raw: owner_raw,
                } = owner_object
                else {
                    return Err(WellfriendError::MalformedPdf(
                        "advanced_editing_closeout appearance Form invocation owner is not a stream".to_string(),
                    ));
                };
                let decoded_owner = decode_stream_lossless_with_limits(
                    &PdfObject::Stream {
                        dict: owner_dict.clone(),
                        raw: owner_raw,
                    },
                    reader,
                    &DecodeLimits {
                        max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES
                            as u64,
                        ..DecodeLimits::default()
                    },
                )?;
                if decoded_owner.status != StreamDecodeStatus::Complete {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing_closeout appearance Form invocation owner is not losslessly decodable"
                            .to_string(),
                    ));
                }
                let owner_range =
                    invocation.owner_operation_byte_start..invocation.owner_operation_byte_end;
                if owner_range.end > decoded_owner.data.len() || owner_range.start > owner_range.end
                {
                    return Err(WellfriendError::MalformedPdf(
                        "advanced_editing_closeout appearance Form invocation range is outside owner stream"
                            .to_string(),
                    ));
                }
                let mut owner_data = decoded_owner.data;
                let mut resources =
                    resolve_advanced_editing_dict(owner_dict.get("Resources"), reader)
                        .unwrap_or_else(crate::PdfDictionary::empty);
                let mut xobjects = resolve_advanced_editing_dict(resources.get("XObject"), reader)
                    .unwrap_or_else(crate::PdfDictionary::empty);
                let mut resource_name = format!("OxV{child_number}");
                let mut suffix_index = 0u32;
                while xobjects.contains_key(&resource_name) {
                    suffix_index = suffix_index.checked_add(1).ok_or_else(|| {
                        WellfriendError::ResourceLimit(
                            "advanced_editing exhausted annotation clone resource names"
                                .to_string(),
                        )
                    })?;
                    resource_name = format!("OxV{child_number}_{suffix_index}");
                }
                xobjects.insert(
                    resource_name.clone(),
                    PdfObject::Reference {
                        number: child_number,
                        generation: 0,
                    },
                );
                resources.insert("XObject", PdfObject::Dictionary(xobjects));
                owner_dict.insert("Resources", PdfObject::Dictionary(resources));
                owner_data.splice(owner_range, format!("/{resource_name} Do").into_bytes());
                let encoded = flate_encode_cancellable(&owner_data, 6)?;
                owner_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
                owner_dict.remove("DecodeParms");
                owner_dict.insert("Length", PdfObject::Integer(encoded.len() as i64));
                let clone_offset = u32::try_from(clone_index)
                    .ok()
                    .and_then(|index| index.checked_add(1))
                    .ok_or_else(|| {
                        WellfriendError::ResourceLimit(
                            "advanced_editing annotation clone path index overflowed".to_string(),
                        )
                    })?;
                let clone_number = allocation_base + clone_offset;
                changed.push(IncrementalObject {
                    number: clone_number,
                    generation: 0,
                    object: PdfObject::Stream {
                        dict: owner_dict,
                        raw: encoded,
                    },
                });
                if invocation.owner_stream_object == source_appearance_number
                    && invocation.owner_stream_generation == source_appearance_generation
                {
                    cloned_appearance = Some(clone_number);
                    clone_graph.push(format!(
                        "annotation:{annotation_index} AP/{appearance_name} cloned as {clone_number} 0 R with /{resource_name} -> {child_number} 0 R"
                    ));
                } else {
                    clone_graph.push(format!(
                        "appearance-form:{} {} R cloned as {clone_number} 0 R with /{resource_name} -> {child_number} 0 R",
                        invocation.owner_stream_object, invocation.owner_stream_generation
                    ));
                }
                child_number = clone_number;
            }
            let appearance_clone_number = cloned_appearance.ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "advanced_editing_closeout nested annotation appearance path did not reach an AP owner"
                        .to_string(),
                )
            })?;
            let parts = appearance_name.split('/').collect::<Vec<_>>();
            if parts.len() == 1 {
                ap.insert(
                    parts[0],
                    PdfObject::Reference {
                        number: appearance_clone_number,
                        generation: 0,
                    },
                );
            } else if parts.len() == 2 {
                let mut states = resolve_advanced_editing_dict(ap.get(parts[0]), reader)
                    .ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "advanced_editing_closeout appearance-state dictionary is malformed"
                                .to_string(),
                        )
                    })?;
                if !states.contains_key(parts[1]) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing_closeout requested appearance state does not exist"
                            .to_string(),
                    ));
                }
                states.insert(
                    parts[1],
                    PdfObject::Reference {
                        number: appearance_clone_number,
                        generation: 0,
                    },
                );
                ap.insert(parts[0], PdfObject::Dictionary(states));
            } else {
                return Err(WellfriendError::MalformedPdf(
                    "advanced_editing_closeout appearance category/state identity is malformed"
                        .to_string(),
                ));
            }
            annotation_dict.insert("AP", PdfObject::Dictionary(ap));
            changed.push(IncrementalObject {
                number: annotation_ref.0,
                generation: annotation_ref.1,
                object: PdfObject::Dictionary(annotation_dict),
            });
            let output = write_incremental_update(reader, changed)?;
            ContentEngine::open_bytes(output.clone())?;
            let report_after = if matches!(operation, VectorEditOperation::Delete) {
                None
            } else {
                Some(vector_occurrence::rebound(
                    &output,
                    &before,
                    (leaf_number, 0),
                    range.start + report_replacement_offset,
                    replacement.len() - report_replacement_offset,
                )?)
            };
            return Ok((output.clone(), VectorEditReport { schema_version:ADVANCED_EDITING_SCHEMA_VERSION.to_string(), stable_id:stable_id.to_string(), operation, before, after:report_after, source_range:[range.start,range.end], replacement_bytes:replacement.len(), unrelated_decoded_prefix_preserved:prefix_preserved, unrelated_decoded_suffix_preserved:suffix_preserved, original_pdf_prefix_preserved:output.starts_with(input), output_reopened:true, output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:options.deterministic, shared_form_policy:options.shared_form_policy, cloned_form:Some([leaf_number,0]), clone_graph, cache_invalidation:advanced_editing_cache_invalidation(input,&output,false,true,true), exact_limits:vec!["clone-one for nested annotation appearance Forms clones the edited leaf, each selected appearance Form owner, and only the target annotation AP entry".to_string(),"sibling N/R/D state entries and /AS are preserved; malformed or ambiguous appearance-state dictionaries fail closed".to_string()] }));
        }
        let invocation_path = before.provenance.form_invocation_path.clone();
        let page = engine.document().get_page(page_number)?;
        let invocation_resources = vector_occurrence::invocation_resources(reader, &page, &before)?;
        if invocation_path.len() > 1 {
            let page = engine.document().get_page(page_number)?;
            let clone_count = u32::try_from(invocation_path.len())
                .ok()
                .and_then(|count| count.checked_add(1))
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit(
                        "advanced_editing Form clone path is too deep to allocate".to_string(),
                    )
                })?;
            let allocation_base = reserve_advanced_object_block(
                reader,
                clone_count,
                "advanced_editing nested Form clone",
            )?;
            let leaf_number = allocation_base;
            changed.push(IncrementalObject {
                number: leaf_number,
                generation: 0,
                object: PdfObject::Stream {
                    dict,
                    raw: compressed,
                },
            });
            let mut child_number = leaf_number;
            let mut page_update: Option<crate::PdfDictionary> = None;
            for (clone_index, invocation) in invocation_path.iter().rev().enumerate() {
                let owner_object = reader.get_object(
                    invocation.owner_stream_object,
                    invocation.owner_stream_generation,
                )?;
                let PdfObject::Stream {
                    dict: mut owner_dict,
                    raw: owner_raw,
                } = owner_object
                else {
                    return Err(WellfriendError::MalformedPdf(
                        "advanced_editing_closeout Form invocation owner is not a stream"
                            .to_string(),
                    ));
                };
                let decoded_owner = decode_stream_lossless_with_limits(
                    &PdfObject::Stream {
                        dict: owner_dict.clone(),
                        raw: owner_raw,
                    },
                    reader,
                    &DecodeLimits {
                        max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES
                            as u64,
                        ..DecodeLimits::default()
                    },
                )?;
                if decoded_owner.status != StreamDecodeStatus::Complete {
                    return Err(WellfriendError::UnsupportedFeature(
                        "advanced_editing_closeout Form invocation owner is not losslessly decodable".to_string(),
                    ));
                }
                let range =
                    invocation.owner_operation_byte_start..invocation.owner_operation_byte_end;
                if range.end > decoded_owner.data.len() || range.start > range.end {
                    return Err(WellfriendError::MalformedPdf(
                        "advanced_editing_closeout Form invocation range is outside owner stream"
                            .to_string(),
                    ));
                }
                let path_index = invocation_path.len() - 1 - clone_index;
                let is_page_owner = path_index == 0;
                let mut owner_data = decoded_owner.data;
                let resource_owner = invocation_resources[path_index].clone();
                let mut xobjects =
                    resolve_advanced_editing_dict(resource_owner.get("XObject"), reader)
                        .unwrap_or_else(crate::PdfDictionary::empty);
                let mut resource_name = format!("OxV{child_number}");
                let mut suffix_index = 0u32;
                while xobjects.contains_key(&resource_name) {
                    suffix_index = suffix_index.checked_add(1).ok_or_else(|| {
                        WellfriendError::ResourceLimit(
                            "advanced_editing exhausted nested Form clone resource names"
                                .to_string(),
                        )
                    })?;
                    resource_name = format!("OxV{child_number}_{suffix_index}");
                }
                xobjects.insert(
                    resource_name.clone(),
                    PdfObject::Reference {
                        number: child_number,
                        generation: 0,
                    },
                );
                owner_data.splice(range, format!("/{resource_name} Do").into_bytes());
                if is_page_owner {
                    let mut page_object = reader
                        .get_object(page.object_number, page.generation_number)?
                        .as_dict()
                        .cloned()
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "advanced_editing_closeout page object is not a dictionary"
                                    .to_string(),
                            )
                        })?;
                    let mut resources = page.resources.clone();
                    resources.insert("XObject", PdfObject::Dictionary(xobjects));
                    page_object.insert("Resources", PdfObject::Dictionary(resources));
                    page_update = Some(page_object);
                    let encoded = flate_encode_cancellable(&owner_data, 6)?;
                    owner_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
                    owner_dict.remove("DecodeParms");
                    owner_dict.insert("Length", PdfObject::Integer(encoded.len() as i64));
                    changed.push(IncrementalObject {
                        number: invocation.owner_stream_object,
                        generation: invocation.owner_stream_generation,
                        object: PdfObject::Stream {
                            dict: owner_dict,
                            raw: encoded,
                        },
                    });
                    clone_graph.push(format!(
                        "page:{page_number} /{resource_name} -> {child_number} 0 R"
                    ));
                } else {
                    owner_dict.insert(
                        "Resources",
                        PdfObject::Dictionary({
                            let mut resources = resource_owner;
                            resources.insert("XObject", PdfObject::Dictionary(xobjects));
                            resources
                        }),
                    );
                    let encoded = flate_encode_cancellable(&owner_data, 6)?;
                    owner_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
                    owner_dict.remove("DecodeParms");
                    owner_dict.insert("Length", PdfObject::Integer(encoded.len() as i64));
                    let clone_offset = u32::try_from(clone_index)
                        .ok()
                        .and_then(|index| index.checked_add(1))
                        .ok_or_else(|| {
                            WellfriendError::ResourceLimit(
                                "advanced_editing Form clone path index overflowed".to_string(),
                            )
                        })?;
                    let parent_clone = allocation_base + clone_offset;
                    changed.push(IncrementalObject {
                        number: parent_clone,
                        generation: 0,
                        object: PdfObject::Stream {
                            dict: owner_dict,
                            raw: encoded,
                        },
                    });
                    clone_graph.push(format!("form:{} {} R /{resource_name} -> {child_number} 0 R; cloned as {parent_clone} 0 R", invocation.owner_stream_object, invocation.owner_stream_generation));
                    child_number = parent_clone;
                }
            }
            let page_dict = page_update.ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "advanced_editing_closeout nested Form clone-one path has no page-owned outer invocation"
                        .to_string(),
                )
            })?;
            changed.push(IncrementalObject {
                number: page.object_number,
                generation: page.generation_number,
                object: PdfObject::Dictionary(page_dict),
            });
            let source = page.contents[before.provenance.content_stream_index];
            let isolated = vector_occurrence::stage(
                reader,
                &page,
                before.provenance.content_stream_index,
                &mut changed,
            )?;
            clone_graph.push(vector_occurrence::receipt(&before, source, isolated));
            let output = write_incremental_update(reader, changed)?;
            ContentEngine::open_bytes(output.clone())?;
            let report_after = if matches!(operation, VectorEditOperation::Delete) {
                None
            } else {
                Some(vector_occurrence::rebound(
                    &output,
                    &before,
                    (leaf_number, 0),
                    range.start + report_replacement_offset,
                    replacement.len() - report_replacement_offset,
                )?)
            };
            return Ok((output.clone(), VectorEditReport { schema_version:ADVANCED_EDITING_SCHEMA_VERSION.to_string(), stable_id:stable_id.to_string(), operation, before, after:report_after, source_range:[range.start,range.end], replacement_bytes:replacement.len(), unrelated_decoded_prefix_preserved:prefix_preserved, unrelated_decoded_suffix_preserved:suffix_preserved, original_pdf_prefix_preserved:output.starts_with(input), output_reopened:true, output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:options.deterministic, shared_form_policy:options.shared_form_policy, cloned_form:Some([leaf_number,0]), clone_graph, cache_invalidation:advanced_editing_cache_invalidation(input,&output,false,true,false), exact_limits:vec!["clone-one recursively clones the leaf and each selected parent Form invocation path; unrelated source Forms are retained".to_string(),"nested Form clone-one requires losslessly decodable streams and direct or indirect Resources dictionaries; cyclic/malformed graphs fail closed".to_string()] }));
        }
        let invocation = form_invocation.as_ref().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing clone_edit_one_instance requires a Form-owned vector object"
                    .to_string(),
            )
        })?;
        if invocation.depth != 1 {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "advanced_editing clone_edit_one_instance is bounded to top-level page Form invocations; selected depth is {}",
                invocation.depth
            )));
        }
        let page = engine.document().get_page(page_number)?;
        let new_number =
            next_advanced_object_number(reader, "advanced_editing top-level Form clone")?;
        let mut resources = page.resources.clone();
        let mut xobjects = resolve_advanced_editing_dict(resources.get("XObject"), reader)
            .unwrap_or_else(crate::PdfDictionary::empty);
        let mut resource_name = format!("OxV{new_number}");
        let mut suffix_index = 0u32;
        while xobjects.contains_key(&resource_name) {
            suffix_index = suffix_index.checked_add(1).ok_or_else(|| {
                WellfriendError::ResourceLimit(
                    "advanced_editing exhausted top-level Form clone resource names".to_string(),
                )
            })?;
            resource_name = format!("OxV{new_number}_{suffix_index}");
        }
        xobjects.insert(
            resource_name.clone(),
            PdfObject::Reference {
                number: new_number,
                generation: 0,
            },
        );
        resources.insert("XObject", PdfObject::Dictionary(xobjects));
        let page_object = reader.get_object(page.object_number, page.generation_number)?;
        let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "advanced_editing page object is not a dictionary".to_string(),
            )
        })?;
        page_dict.insert("Resources", PdfObject::Dictionary(resources));

        let owner_object = reader.get_object(
            invocation.owner_stream_object,
            invocation.owner_stream_generation,
        )?;
        let PdfObject::Stream {
            dict: mut owner_dict,
            raw: owner_raw,
        } = owner_object
        else {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing Form invocation owner is not a stream".to_string(),
            ));
        };
        let owner_decoded = decode_stream_lossless_with_limits(
            &PdfObject::Stream {
                dict: owner_dict.clone(),
                raw: owner_raw,
            },
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if owner_decoded.status != StreamDecodeStatus::Complete {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing Form invocation owner is not losslessly decodable".to_string(),
            ));
        }
        let owner_range =
            invocation.owner_operation_byte_start..invocation.owner_operation_byte_end;
        if owner_range.end > owner_decoded.data.len() || owner_range.start > owner_range.end {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing Form invocation range is outside its owner stream".to_string(),
            ));
        }
        let mut owner_data = owner_decoded.data;
        owner_data.splice(owner_range, format!("/{resource_name} Do").into_bytes());
        let owner_compressed = flate_encode_cancellable(&owner_data, 6)?;
        owner_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
        owner_dict.remove("DecodeParms");
        owner_dict.insert("Length", PdfObject::Integer(owner_compressed.len() as i64));

        changed.push(IncrementalObject {
            number: new_number,
            generation: 0,
            object: PdfObject::Stream {
                dict,
                raw: compressed,
            },
        });
        changed.push(IncrementalObject {
            number: invocation.owner_stream_object,
            generation: invocation.owner_stream_generation,
            object: PdfObject::Stream {
                dict: owner_dict,
                raw: owner_compressed,
            },
        });
        changed.push(IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(page_dict),
        });
        cloned_form = Some([new_number, 0]);
        clone_graph.push(format!(
            "page:{page_number}/{} {} R -> /{} {} 0 R (source {} {} R retained)",
            invocation.owner_stream_object,
            invocation.owner_stream_generation,
            resource_name,
            new_number,
            invocation.form_object,
            invocation.form_generation
        ));
        after.provenance.object_number = new_number;
        after.provenance.generation = 0;
        after.provenance.resource_owner = format!("form-{new_number}-0");
        if let Some(after_invocation) = after.provenance.form_invocation.as_mut() {
            after_invocation.resource_name = resource_name;
            after_invocation.form_object = new_number;
            after_invocation.form_generation = 0;
        }
        after.stable_id = vector_stable_id_for_object(&after);
    } else {
        if let Some(invocation) = &form_invocation {
            clone_graph.push(format!(
                "edit_all_uses:{} {} R via /{}",
                invocation.form_object, invocation.form_generation, invocation.resource_name
            ));
        }
        changed.push(IncrementalObject {
            number: before.provenance.object_number,
            generation: before.provenance.generation,
            object: PdfObject::Stream {
                dict,
                raw: compressed,
            },
        });
    }
    let page = engine.document().get_page(page_number)?;
    let is_page_vector = form_invocation.is_none()
        && page
            .contents
            .get(before.provenance.content_stream_index)
            .copied()
            == Some((
                before.provenance.object_number,
                before.provenance.generation,
            ));
    if is_page_vector || cloned_form.is_some() {
        let source = page.contents[before.provenance.content_stream_index];
        let isolated = vector_occurrence::stage(
            reader,
            &page,
            before.provenance.content_stream_index,
            &mut changed,
        )?;
        clone_graph.push(vector_occurrence::receipt(&before, source, isolated));
        if is_page_vector {
            after.provenance.object_number = isolated.0;
            after.provenance.generation = isolated.1;
        }
    }
    let output = write_incremental_update(reader, changed)?;
    ContentEngine::open_bytes(output.clone())?;
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    let report_after = if matches!(operation, VectorEditOperation::Delete) {
        None
    } else {
        Some(vector_occurrence::rebound(
            &output,
            &before,
            (after.provenance.object_number, after.provenance.generation),
            range.start + report_replacement_offset,
            replacement.len() - report_replacement_offset,
        )?)
    };
    Ok((
        output.clone(),
        VectorEditReport {
            schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
            stable_id: stable_id.to_string(),
            operation,
            before,
            after: report_after,
            source_range: [range.start, range.end],
            replacement_bytes: replacement.len(),
            unrelated_decoded_prefix_preserved: prefix_preserved,
            unrelated_decoded_suffix_preserved: suffix_preserved,
            original_pdf_prefix_preserved: output.starts_with(input),
            output_reopened: true,
            output_sha256,
            signature_policy,
            cryptographic_validity_claimed: false,
            deterministic: options.deterministic,
            shared_form_policy: options.shared_form_policy,
            cloned_form,
            clone_graph,
            cache_invalidation: advanced_editing_cache_invalidation(input, &output, false, true, false),
            exact_limits: vec![
                "only the reconstructed paint operation range is rewritten; surrounding operators and the original PDF prefix are preserved".to_string(),
                "semantic shape names such as ellipse are not inferred from arbitrary cubic paths".to_string(),
                "shared Form edits require explicit edit-all or clone-one policy; page clone-one isolates the selected Contents slot and Form chain, retaining source objects".to_string(),
            ],
        },
    ))
}

#[allow(clippy::too_many_arguments)]
fn edit_vector_z_order(
    input: &[u8],
    engine: &ContentEngine,
    inventory: Vec<EditableVectorObject>,
    before: EditableVectorObject,
    operation: VectorEditOperation,
    options: &VectorEditOptions,
    signature_policy: EditPolicyReport,
) -> Result<(Vec<u8>, VectorEditReport)> {
    if before.provenance.form_invocation.is_some() {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing bounded z-order currently requires a page-owned operation range; Form z-order changes require ownership-specific group analysis".to_string(),
        ));
    }
    if before.provenance.marked_content_depth != 0
        || before.provenance.ocg_context.is_some()
        || before.clipping_path
        || before.clipping_context
    {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing z-order change would cross clipping, marked-content, or OCG semantics"
                .to_string(),
        ));
    }
    let mut siblings = inventory
        .into_iter()
        .filter(|object| {
            object.provenance.form_invocation.is_none()
                && object.provenance.object_number == before.provenance.object_number
                && object.provenance.generation == before.provenance.generation
                && object.provenance.content_stream_index == before.provenance.content_stream_index
                && object.provenance.marked_content_depth == 0
                && object.provenance.ocg_context.is_none()
                && !object.clipping_path
                && !object.clipping_context
        })
        .collect::<Vec<_>>();
    siblings.sort_by_key(|object| object.provenance.operation_byte_start);
    siblings.dedup_by_key(|object| {
        (
            object.provenance.operation_byte_start,
            object.provenance.operation_byte_end,
        )
    });
    let selected_index = siblings
        .iter()
        .position(|object| object.stable_id == before.stable_id)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing z-order target is not in its page-owned sibling set".to_string(),
            )
        })?;
    let reader = engine.document().reader();
    let stream_object = reader.get_object(
        before.provenance.object_number,
        before.provenance.generation,
    )?;
    let PdfObject::Stream { mut dict, raw } = stream_object else {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing z-order provenance does not reference a stream".to_string(),
        ));
    };
    let decoded_result = decode_stream_lossless_with_limits(
        &PdfObject::Stream {
            dict: dict.clone(),
            raw,
        },
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    if decoded_result.status != StreamDecodeStatus::Complete {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing z-order stream is not losslessly decodable".to_string(),
        ));
    }
    let range = before.provenance.operation_byte_start..before.provenance.operation_byte_end;
    if range.end > decoded_result.data.len() || range.start > range.end {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing z-order operation range is outside decoded stream".to_string(),
        ));
    }
    let insertion_original = match operation {
        VectorEditOperation::BringForward => siblings
            .get(selected_index + 1)
            .map(|object| object.provenance.operation_byte_end)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "advanced_editing vector is already the front sibling".to_string(),
                )
            })?,
        VectorEditOperation::SendBackward => selected_index
            .checked_sub(1)
            .and_then(|index| siblings.get(index))
            .map(|object| object.provenance.operation_byte_start)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "advanced_editing vector is already the back sibling".to_string(),
                )
            })?,
        VectorEditOperation::BringToFront => decoded_result.data.len(),
        VectorEditOperation::SendToBack => 0,
        _ => unreachable!("z-order helper only receives z-order operations"),
    };
    let affected_start = range.start.min(insertion_original);
    let affected_end = range.end.max(insertion_original);
    let original_prefix = decoded_result.data[..affected_start].to_vec();
    let original_suffix = decoded_result.data[affected_end..].to_vec();
    let mut replacement_object = before.clone();
    // The object moves outside its original graphics-state location, so the
    // self-contained replacement carries the effective page transform.
    let serialized_replacement = serialize_vector_object(&replacement_object);
    let removed_len = range.end - range.start;
    let mut decoded = decoded_result.data;
    decoded.drain(range.clone());
    let insertion = if insertion_original >= range.end {
        insertion_original.saturating_sub(removed_len)
    } else {
        insertion_original
    };
    // Operation ranges end at the operator token, before trailing whitespace.
    // Moving a program to such a boundary without delimiters can concatenate
    // `S` and `q` into one unknown operator and merge the two paths.
    let mut replacement = Vec::with_capacity(serialized_replacement.len() + 2);
    if insertion > 0 && !decoded[insertion - 1].is_ascii_whitespace() {
        replacement.push(b'\n');
    }
    replacement.extend_from_slice(&serialized_replacement);
    if insertion < decoded.len() && !decoded[insertion].is_ascii_whitespace() {
        replacement.push(b'\n');
    }
    decoded.splice(insertion..insertion, replacement.clone());
    let prefix_preserved = decoded.starts_with(&original_prefix);
    let suffix_preserved = decoded.ends_with(&original_suffix);
    let compressed = flate_encode_cancellable(&decoded, 6)?;
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    let mut changed = vec![IncrementalObject {
        number: before.provenance.object_number,
        generation: before.provenance.generation,
        object: PdfObject::Stream {
            dict,
            raw: compressed,
        },
    }];
    let page = engine.document().get_page(before.provenance.page)?;
    let source = (
        before.provenance.object_number,
        before.provenance.generation,
    );
    if page
        .contents
        .get(before.provenance.content_stream_index)
        .copied()
        != Some(source)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "vector z-order requires a page Contents occurrence".into(),
        ));
    }
    let isolated = vector_occurrence::stage(
        reader,
        &page,
        before.provenance.content_stream_index,
        &mut changed,
    )?;
    let clone_graph = vec![vector_occurrence::receipt(&before, source, isolated)];
    let output = write_incremental_update(reader, changed)?;
    ContentEngine::open_bytes(output.clone())?;
    replacement_object =
        vector_occurrence::rebound(&output, &before, isolated, insertion, replacement.len())?;
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    Ok((
        output.clone(),
        VectorEditReport {
            schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
            stable_id: before.stable_id.clone(),
            operation,
            before,
            after: Some(replacement_object),
            source_range: [range.start, range.end],
            replacement_bytes: replacement.len(),
            unrelated_decoded_prefix_preserved: prefix_preserved,
            unrelated_decoded_suffix_preserved: suffix_preserved,
            original_pdf_prefix_preserved: output.starts_with(input),
            output_reopened: true,
            output_sha256,
            signature_policy,
            cryptographic_validity_claimed: false,
            deterministic: options.deterministic,
            shared_form_policy: options.shared_form_policy,
            cloned_form: None,
            clone_graph,
            cache_invalidation: advanced_editing_cache_invalidation(input, &output, false, true, false),
            exact_limits: vec![
                "z-order movement is bounded to page-owned path objects outside clipping, marked-content, and OCG contexts".to_string(),
                "the moved path is serialized as a self-contained graphics-state block; unrelated outer bytes and the original PDF prefix are preserved".to_string(),
            ],
        },
    ))
}

#[allow(clippy::too_many_arguments)]
fn edit_vector_group_structure(
    input: &[u8],
    engine: &ContentEngine,
    inventory: Vec<EditableVectorObject>,
    before: EditableVectorObject,
    operation: VectorEditOperation,
    options: &VectorEditOptions,
    signature_policy: EditPolicyReport,
) -> Result<(Vec<u8>, VectorEditReport)> {
    if before.provenance.form_invocation.is_some() {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing bounded group/ungroup currently requires page-owned operation ranges"
                .to_string(),
        ));
    }
    let reader = engine.document().reader();
    let stream_object = reader.get_object(
        before.provenance.object_number,
        before.provenance.generation,
    )?;
    let PdfObject::Stream { mut dict, raw } = stream_object else {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing group provenance does not reference a stream".to_string(),
        ));
    };
    let decoded_result = decode_stream_lossless_with_limits(
        &PdfObject::Stream {
            dict: dict.clone(),
            raw,
        },
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    if decoded_result.status != StreamDecodeStatus::Complete {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing group stream is not losslessly decodable".to_string(),
        ));
    }
    let mut siblings = inventory
        .iter()
        .filter(|object| {
            object.provenance.form_invocation.is_none()
                && object.provenance.object_number == before.provenance.object_number
                && object.provenance.generation == before.provenance.generation
                && object.provenance.content_stream_index == before.provenance.content_stream_index
        })
        .cloned()
        .collect::<Vec<_>>();
    siblings.sort_by_key(|object| object.provenance.operation_byte_start);
    let selected_ordinal = siblings
        .iter()
        .position(|object| object.stable_id == before.stable_id)
        .ok_or_else(|| {
            WellfriendError::invalid_input("group target is not in its source occurrence")
        })?;
    let (range, replacement) = match &operation {
        VectorEditOperation::GroupWith { stable_ids } => {
            let mut selected_ids = stable_ids.clone();
            if !selected_ids.contains(&before.stable_id) {
                selected_ids.push(before.stable_id.clone());
            }
            selected_ids.sort();
            selected_ids.dedup();
            if selected_ids.len() < 2 {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing group_with requires at least two distinct stable IDs"
                        .to_string(),
                ));
            }
            let mut indices = Vec::new();
            for stable_id in &selected_ids {
                let index = siblings
                    .iter()
                    .position(|object| object.stable_id == *stable_id)
                    .ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(format!(
                            "advanced_editing group member {stable_id} is not a sibling in the selected page stream"
                        ))
                    })?;
                indices.push(index);
            }
            indices.sort_unstable();
            if indices
                .windows(2)
                .any(|pair| pair[1] != pair[0].saturating_add(1))
            {
                return Err(WellfriendError::UnsupportedFeature(
                    "advanced_editing bounded grouping requires contiguous sibling vector ranges"
                        .to_string(),
                ));
            }
            let first = &siblings[*indices.first().expect("non-empty group indices")];
            let last = &siblings[*indices.last().expect("non-empty group indices")];
            let range = first.provenance.operation_byte_start..last.provenance.operation_byte_end;
            if range.end > decoded_result.data.len() {
                return Err(WellfriendError::MalformedPdf(
                    "advanced_editing group range is outside decoded stream".to_string(),
                ));
            }
            let mut replacement = b"/WellfriendGroup BMC\n".to_vec();
            replacement.extend_from_slice(&decoded_result.data[range.clone()]);
            replacement.extend_from_slice(b"\nEMC");
            (range, replacement)
        }
        VectorEditOperation::Ungroup => {
            let group = before
                .provenance
                .wellfriendpdf_groups
                .last()
                .ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "advanced_editing selected vector is not inside an Wellfriend bounded group"
                            .to_string(),
                    )
                })?;
            if group.marker_end > decoded_result.data.len()
                || group.content_start > group.content_end
            {
                return Err(WellfriendError::MalformedPdf(
                    "advanced_editing group marker range is outside decoded stream".to_string(),
                ));
            }
            (
                group.marker_start..group.marker_end,
                decoded_result.data[group.content_start..group.content_end].to_vec(),
            )
        }
        _ => unreachable!("group helper only receives group operations"),
    };
    let original_prefix = decoded_result.data[..range.start].to_vec();
    let original_suffix = decoded_result.data[range.end..].to_vec();
    let mut decoded = decoded_result.data;
    decoded.splice(range.clone(), replacement.clone());
    let prefix_preserved = decoded.starts_with(&original_prefix);
    let suffix_preserved = decoded.ends_with(&original_suffix);
    let compressed = flate_encode_cancellable(&decoded, 6)?;
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    let mut changed = vec![IncrementalObject {
        number: before.provenance.object_number,
        generation: before.provenance.generation,
        object: PdfObject::Stream {
            dict,
            raw: compressed,
        },
    }];
    let page = engine.document().get_page(before.provenance.page)?;
    let source = (
        before.provenance.object_number,
        before.provenance.generation,
    );
    if page
        .contents
        .get(before.provenance.content_stream_index)
        .copied()
        != Some(source)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "vector grouping requires a page Contents occurrence".into(),
        ));
    }
    let isolated = vector_occurrence::stage(
        reader,
        &page,
        before.provenance.content_stream_index,
        &mut changed,
    )?;
    let clone_graph = vec![vector_occurrence::receipt(&before, source, isolated)];
    let output = write_incremental_update(reader, changed)?;
    ContentEngine::open_bytes(output.clone())?;
    let reopened_inventory = list_vector_objects(&output, before.provenance.page)?;
    let mut saved_siblings = reopened_inventory
        .objects
        .into_iter()
        .filter(|object| {
            (
                object.provenance.object_number,
                object.provenance.generation,
            ) == isolated
                && object.provenance.content_stream_index == before.provenance.content_stream_index
                && object.provenance.form_invocation.is_none()
        })
        .collect::<Vec<_>>();
    saved_siblings.sort_by_key(|object| object.provenance.operation_byte_start);
    if saved_siblings.len() != siblings.len() {
        return Err(WellfriendError::invalid_input(
            "group rewrite changed the number of source vectors",
        ));
    }
    let after = Some(
        saved_siblings
            .into_iter()
            .nth(selected_ordinal)
            .ok_or_else(|| WellfriendError::invalid_input("saved group target was not found"))?,
    );
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    Ok((
        output.clone(),
        VectorEditReport {
            schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
            stable_id: before.stable_id.clone(),
            operation,
            before,
            after,
            source_range: [range.start, range.end],
            replacement_bytes: replacement.len(),
            unrelated_decoded_prefix_preserved: prefix_preserved,
            unrelated_decoded_suffix_preserved: suffix_preserved,
            original_pdf_prefix_preserved: output.starts_with(input),
            output_reopened: true,
            output_sha256,
            signature_policy,
            cryptographic_validity_claimed: false,
            deterministic: options.deterministic,
            shared_form_policy: options.shared_form_policy,
            cloned_form: None,
            clone_graph,
            cache_invalidation: advanced_editing_cache_invalidation(input, &output, false, true, false),
            exact_limits: vec![
                "group/ungroup uses inert Wellfriend marked-content ownership around contiguous page-owned vector ranges; painting order and graphics are preserved".to_string(),
                "Form-owned, non-contiguous, clipping-sensitive, or cross-stream groups are rejected exactly".to_string(),
            ],
        },
    ))
}

fn reconstruct_vector_objects(
    data: &[u8],
    page: usize,
    stream_index: usize,
    object_number: u32,
    generation: u16,
) -> Result<Vec<EditableVectorObject>> {
    let operations = raw_content_operations(data)?;
    let group_ranges = wellfriendpdf_group_ranges(&operations);
    let mut output = Vec::new();
    let mut state = VectorGraphicsState::default();
    let mut stack = Vec::new();
    let mut path = Vec::new();
    let mut path_start = None;
    let mut clip_pending = false;
    for operation in operations {
        let numbers = operation_numbers(&operation.operands);
        match operation.operator.as_str() {
            "q" => stack.push(state.clone()),
            "Q" => state = stack.pop().unwrap_or_default(),
            "cm" if numbers.len() >= 6 => {
                state.matrix = state.matrix.multiply(VectorMatrix {
                    a: numbers[0],
                    b: numbers[1],
                    c: numbers[2],
                    d: numbers[3],
                    e: numbers[4],
                    f: numbers[5],
                });
            }
            "w" if !numbers.is_empty() => state.stroke.width = numbers[0],
            "J" if !numbers.is_empty() => state.stroke.cap = numbers[0] as i32,
            "j" if !numbers.is_empty() => state.stroke.join = numbers[0] as i32,
            "M" if !numbers.is_empty() => state.stroke.miter_limit = numbers[0],
            "d" => {
                if let Some(phase) = numbers.last().copied() {
                    state.stroke.dash_phase = phase;
                    state.stroke.dash = numbers[..numbers.len().saturating_sub(1)].to_vec();
                }
            }
            "G" if !numbers.is_empty() => state.stroke_color = vector_color("DeviceGray", &numbers),
            "g" if !numbers.is_empty() => state.fill_color = vector_color("DeviceGray", &numbers),
            "RG" if numbers.len() >= 3 => state.stroke_color = vector_color("DeviceRGB", &numbers),
            "rg" if numbers.len() >= 3 => state.fill_color = vector_color("DeviceRGB", &numbers),
            "K" if numbers.len() >= 4 => state.stroke_color = vector_color("DeviceCMYK", &numbers),
            "k" if numbers.len() >= 4 => state.fill_color = vector_color("DeviceCMYK", &numbers),
            "gs" => {
                state.ext_g_state = operation.operands.iter().find_map(|operand| match operand {
                    LexicalKind::Name(name) => Some(name.clone()),
                    _ => None,
                });
            }
            "BMC" | "BDC" => {
                state.marked_depth = state.marked_depth.saturating_add(1);
                if operation
                    .operands
                    .iter()
                    .any(|operand| matches!(operand, LexicalKind::Name(name) if name == "OC"))
                {
                    state.ocg_context = Some("marked_content_OC".to_string());
                }
            }
            "EMC" => {
                state.marked_depth = state.marked_depth.saturating_sub(1);
                if state.marked_depth == 0 {
                    state.ocg_context = None;
                }
            }
            "m" if numbers.len() >= 2 => {
                path_start.get_or_insert(operation.start);
                let current = InkPoint {
                    x: numbers[0],
                    y: numbers[1],
                };
                path.push(VectorPathSegment::MoveTo { point: current });
            }
            "l" if numbers.len() >= 2 => {
                path_start.get_or_insert(operation.start);
                let current = InkPoint {
                    x: numbers[0],
                    y: numbers[1],
                };
                path.push(VectorPathSegment::LineTo { point: current });
            }
            "c" if numbers.len() >= 6 => {
                path_start.get_or_insert(operation.start);
                let current = InkPoint {
                    x: numbers[4],
                    y: numbers[5],
                };
                path.push(VectorPathSegment::CubicTo {
                    control1: InkPoint {
                        x: numbers[0],
                        y: numbers[1],
                    },
                    control2: InkPoint {
                        x: numbers[2],
                        y: numbers[3],
                    },
                    point: current,
                });
            }
            "v" if numbers.len() >= 4 => {
                path_start.get_or_insert(operation.start);
                let current = InkPoint {
                    x: numbers[2],
                    y: numbers[3],
                };
                path.push(VectorPathSegment::CubicTo {
                    control1: current_point_for_path(&path).unwrap_or(current),
                    control2: InkPoint {
                        x: numbers[0],
                        y: numbers[1],
                    },
                    point: current,
                });
            }
            "y" if numbers.len() >= 4 => {
                path_start.get_or_insert(operation.start);
                let endpoint = InkPoint {
                    x: numbers[2],
                    y: numbers[3],
                };
                path.push(VectorPathSegment::CubicTo {
                    control1: InkPoint {
                        x: numbers[0],
                        y: numbers[1],
                    },
                    control2: endpoint,
                    point: endpoint,
                });
            }
            "re" if numbers.len() >= 4 => {
                path_start.get_or_insert(operation.start);
                path.push(VectorPathSegment::Rectangle {
                    x: numbers[0],
                    y: numbers[1],
                    width: numbers[2],
                    height: numbers[3],
                });
            }
            "h" => path.push(VectorPathSegment::Close),
            "W" | "W*" => clip_pending = true,
            paint if is_path_paint_operator(paint) && !path.is_empty() => {
                let mut local_path = std::mem::take(&mut path);
                if matches!(paint, "s" | "b" | "b*") {
                    local_path.push(VectorPathSegment::Close);
                }
                let paint_mode = vector_paint_mode(paint);
                let start = path_start.take().unwrap_or(operation.start);
                let provenance = VectorProvenance {
                    page,
                    object_number,
                    generation,
                    content_stream_index: stream_index,
                    operation_byte_start: start,
                    operation_byte_end: operation.end,
                    form_stack: Vec::new(),
                    marked_content_depth: state.marked_depth,
                    ocg_context: state.ocg_context.clone(),
                    resource_owner: format!("page-{page}-stream-{object_number}-{generation}"),
                    form_invocation: None,
                    form_invocation_path: Vec::new(),
                    wellfriendpdf_groups: wellfriendpdf_groups_for_range(
                        &group_ranges,
                        start,
                        operation.end,
                    ),
                };
                let stable_id = vector_stable_id(&provenance, &local_path, paint);
                output.push(EditableVectorObject {
                    schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
                    stable_id,
                    bbox: vector_bbox(&local_path, state.matrix),
                    transform: state.matrix,
                    segments: local_path,
                    fill_rule: if paint.ends_with('*') {
                        VectorFillRule::EvenOdd
                    } else {
                        VectorFillRule::Nonzero
                    },
                    paint_mode,
                    stroke: state.stroke.clone(),
                    stroke_color: state.stroke_color.clone(),
                    fill_color: state.fill_color.clone(),
                    opacity: state.opacity,
                    blend_mode: state.blend_mode.clone(),
                    clipping_path: clip_pending,
                    clipping_context: state.clipping_context,
                    ext_g_state: state.ext_g_state.clone(),
                    confidence: 1.0,
                    edit_safety: if clip_pending {
                        "bounded_preserve_clip"
                    } else {
                        "safe_operation_range_rewrite"
                    }
                    .to_string(),
                    diagnostics: if state.ext_g_state.is_some() {
                        vec!["ExtGState retained by resource name; opacity/blend internals are not inferred".to_string()]
                    } else {
                        Vec::new()
                    },
                    provenance,
                });
                if clip_pending {
                    state.clipping_context = true;
                }
                clip_pending = false;
            }
            _ => {}
        }
    }
    Ok(output)
}

fn wellfriendpdf_group_ranges(operations: &[RawContentOperation]) -> Vec<VectorGroupProvenance> {
    let mut stack: Vec<Option<(usize, usize, usize)>> = Vec::new();
    let mut output = Vec::new();
    for operation in operations {
        match operation.operator.as_str() {
            "BMC" | "BDC" => {
                let is_wellfriendpdf_group = operation.operands.iter().any(
                    |operand| matches!(operand, LexicalKind::Name(name) if name == "WellfriendGroup"),
                );
                let depth = stack.len() + 1;
                stack.push(is_wellfriendpdf_group.then_some((
                    operation.start,
                    operation.end,
                    depth,
                )));
            }
            "EMC" => {
                if let Some(Some((marker_start, content_start, depth))) = stack.pop() {
                    output.push(VectorGroupProvenance {
                        marker_start,
                        marker_end: operation.end,
                        content_start,
                        content_end: operation.start,
                        depth,
                    });
                }
            }
            _ => {}
        }
    }
    output.sort_by_key(|group| (group.marker_start, group.marker_end));
    output
}

fn wellfriendpdf_groups_for_range(
    groups: &[VectorGroupProvenance],
    start: usize,
    end: usize,
) -> Vec<VectorGroupProvenance> {
    groups
        .iter()
        .filter(|group| group.content_start <= start && group.content_end >= end)
        .cloned()
        .collect()
}

fn raw_content_operations(data: &[u8]) -> Result<Vec<RawContentOperation>> {
    let tokens = lex_content(data)?;
    let mut operands = Vec::<LexicalToken>::new();
    let mut operations = Vec::new();
    for token in tokens {
        if let LexicalKind::Word(operator) = &token.kind {
            operations.push(RawContentOperation {
                start: operands
                    .first()
                    .map(|operand| operand.start)
                    .unwrap_or(token.start),
                end: token.end,
                operator: operator.clone(),
                operands: operands
                    .iter()
                    .map(|operand| operand.kind.clone())
                    .collect(),
            });
            operands.clear();
        } else {
            operands.push(token);
        }
    }
    Ok(operations)
}

fn operation_numbers(operands: &[LexicalKind]) -> Vec<f64> {
    operands
        .iter()
        .filter_map(|operand| match operand {
            LexicalKind::Number(number) => Some(*number),
            _ => None,
        })
        .collect()
}

fn is_path_paint_operator(operator: &str) -> bool {
    matches!(
        operator,
        "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n"
    )
}

fn vector_paint_mode(operator: &str) -> VectorPaintMode {
    match operator {
        "S" | "s" => VectorPaintMode::Stroke,
        "f" | "F" => VectorPaintMode::FillNonzero,
        "f*" => VectorPaintMode::FillEvenOdd,
        "B" | "b" => VectorPaintMode::FillStrokeNonzero,
        "B*" | "b*" => VectorPaintMode::FillStrokeEvenOdd,
        _ => VectorPaintMode::EndPath,
    }
}

fn vector_color(space: &str, values: &[f64]) -> VectorColor {
    VectorColor {
        color_space: space.to_string(),
        components: values
            .iter()
            .map(|value| canonical_number(*value))
            .collect(),
    }
}

fn vector_stable_id(
    provenance: &VectorProvenance,
    path: &[VectorPathSegment],
    paint: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provenance.page.to_le_bytes());
    hasher.update(provenance.content_stream_index.to_le_bytes());
    hasher.update(provenance.object_number.to_le_bytes());
    hasher.update(provenance.generation.to_le_bytes());
    hasher.update(provenance.operation_byte_start.to_le_bytes());
    hasher.update(provenance.operation_byte_end.to_le_bytes());
    hasher.update(serde_json::to_vec(&provenance.form_stack).unwrap_or_default());
    hasher.update(serde_json::to_vec(&provenance.form_invocation).unwrap_or_default());
    hasher.update(serde_json::to_vec(&provenance.form_invocation_path).unwrap_or_default());
    hasher.update(serde_json::to_vec(&provenance.wellfriendpdf_groups).unwrap_or_default());
    hasher.update(paint.as_bytes());
    hasher.update(serde_json::to_vec(path).unwrap_or_default());
    let digest = format!("{:x}", hasher.finalize());
    format!("vector-{}", &digest[..24])
}

fn vector_stable_id_for_object(object: &EditableVectorObject) -> String {
    let paint = match object.paint_mode {
        VectorPaintMode::Stroke => "S",
        VectorPaintMode::FillNonzero => "f",
        VectorPaintMode::FillEvenOdd => "f*",
        VectorPaintMode::FillStrokeNonzero => "B",
        VectorPaintMode::FillStrokeEvenOdd => "B*",
        VectorPaintMode::EndPath => "n",
    };
    vector_stable_id(&object.provenance, &object.segments, paint)
}

fn vector_bbox(path: &[VectorPathSegment], matrix: VectorMatrix) -> [f64; 4] {
    let mut points = Vec::new();
    for segment in path {
        match segment {
            VectorPathSegment::MoveTo { point } | VectorPathSegment::LineTo { point } => {
                points.push(matrix.transform(*point));
            }
            VectorPathSegment::CubicTo {
                control1,
                control2,
                point,
            } => {
                points.extend([
                    matrix.transform(*control1),
                    matrix.transform(*control2),
                    matrix.transform(*point),
                ]);
            }
            VectorPathSegment::Rectangle {
                x,
                y,
                width,
                height,
            } => {
                points.extend([
                    matrix.transform(InkPoint { x: *x, y: *y }),
                    matrix.transform(InkPoint {
                        x: x + width,
                        y: *y,
                    }),
                    matrix.transform(InkPoint {
                        x: x + width,
                        y: y + height,
                    }),
                    matrix.transform(InkPoint {
                        x: *x,
                        y: y + height,
                    }),
                ]);
            }
            VectorPathSegment::Close => {}
        }
    }
    if points.is_empty() {
        return [0.0; 4];
    }
    let mut bbox = [points[0].x, points[0].y, points[0].x, points[0].y];
    for point in points.into_iter().skip(1) {
        bbox[0] = bbox[0].min(point.x);
        bbox[1] = bbox[1].min(point.y);
        bbox[2] = bbox[2].max(point.x);
        bbox[3] = bbox[3].max(point.y);
    }
    bbox.map(canonical_number)
}

fn current_point_for_path(path: &[VectorPathSegment]) -> Option<InkPoint> {
    path.iter().rev().find_map(|segment| match segment {
        VectorPathSegment::MoveTo { point }
        | VectorPathSegment::LineTo { point }
        | VectorPathSegment::CubicTo { point, .. } => Some(*point),
        VectorPathSegment::Rectangle { x, y, .. } => Some(InkPoint { x: *x, y: *y }),
        VectorPathSegment::Close => None,
    })
}

fn validate_vector_edit(operation: &VectorEditOperation) -> Result<()> {
    let values = match operation {
        VectorEditOperation::Move { dx, dy } => vec![*dx, *dy],
        VectorEditOperation::Scale { sx, sy, origin } => vec![*sx, *sy, origin.x, origin.y],
        VectorEditOperation::Rotate { degrees, origin } => vec![*degrees, origin.x, origin.y],
        VectorEditOperation::Skew {
            x_degrees,
            y_degrees,
        } => vec![*x_degrees, *y_degrees],
        VectorEditOperation::MirrorHorizontal { axis_x } => vec![*axis_x],
        VectorEditOperation::MirrorVertical { axis_y } => vec![*axis_y],
        VectorEditOperation::EditPoint { value, .. } => vec![value.x, value.y],
        VectorEditOperation::SetFill { color } | VectorEditOperation::SetStroke { color } => {
            color.components.clone()
        }
        VectorEditOperation::SetStrokeWidth { width } => vec![*width],
        VectorEditOperation::SetDash { dash, phase } => {
            let mut values = dash.clone();
            values.push(*phase);
            values
        }
        VectorEditOperation::SetCapJoin { miter_limit, .. } => vec![*miter_limit],
        VectorEditOperation::SetOpacity { opacity } => vec![*opacity],
        VectorEditOperation::Duplicate { dx, dy } => vec![*dx, *dy],
        VectorEditOperation::Delete
        | VectorEditOperation::BringForward
        | VectorEditOperation::SendBackward
        | VectorEditOperation::BringToFront
        | VectorEditOperation::SendToBack
        | VectorEditOperation::GroupWith { .. }
        | VectorEditOperation::Ungroup => Vec::new(),
    };
    if values
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 1.0e9)
    {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing vector edit contains non-finite or out-of-range numbers".to_string(),
        ));
    }
    Ok(())
}

fn mutate_vector(object: &mut EditableVectorObject, operation: &VectorEditOperation) -> Result<()> {
    match operation {
        VectorEditOperation::Move { dx, dy } => apply_vector_transform(
            object,
            VectorMatrix {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: *dx,
                f: *dy,
            },
        ),
        VectorEditOperation::Scale { sx, sy, origin } => apply_vector_transform(
            object,
            around_origin(
                VectorMatrix {
                    a: *sx,
                    b: 0.0,
                    c: 0.0,
                    d: *sy,
                    e: 0.0,
                    f: 0.0,
                },
                *origin,
            ),
        ),
        VectorEditOperation::Rotate { degrees, origin } => {
            let radians = degrees.to_radians();
            apply_vector_transform(
                object,
                around_origin(
                    VectorMatrix {
                        a: radians.cos(),
                        b: radians.sin(),
                        c: -radians.sin(),
                        d: radians.cos(),
                        e: 0.0,
                        f: 0.0,
                    },
                    *origin,
                ),
            );
        }
        VectorEditOperation::Skew {
            x_degrees,
            y_degrees,
        } => apply_vector_transform(
            object,
            VectorMatrix {
                a: 1.0,
                b: y_degrees.to_radians().tan(),
                c: x_degrees.to_radians().tan(),
                d: 1.0,
                e: 0.0,
                f: 0.0,
            },
        ),
        VectorEditOperation::MirrorHorizontal { axis_x } => apply_vector_transform(
            object,
            VectorMatrix {
                a: -1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 2.0 * axis_x,
                f: 0.0,
            },
        ),
        VectorEditOperation::MirrorVertical { axis_y } => apply_vector_transform(
            object,
            VectorMatrix {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: -1.0,
                e: 0.0,
                f: 2.0 * axis_y,
            },
        ),
        VectorEditOperation::EditPoint {
            segment,
            point,
            value,
        } => {
            let target = object.segments.get_mut(*segment).ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "advanced_editing vector segment {segment} is out of range"
                ))
            })?;
            edit_vector_segment_point(target, *point, *value)?;
        }
        VectorEditOperation::SetFill { color } => object.fill_color = color.clone(),
        VectorEditOperation::SetStroke { color } => object.stroke_color = color.clone(),
        VectorEditOperation::SetStrokeWidth { width } => object.stroke.width = *width,
        VectorEditOperation::SetDash { dash, phase } => {
            object.stroke.dash = dash.clone();
            object.stroke.dash_phase = *phase;
        }
        VectorEditOperation::SetCapJoin {
            cap,
            join,
            miter_limit,
        } => {
            object.stroke.cap = *cap;
            object.stroke.join = *join;
            object.stroke.miter_limit = *miter_limit;
        }
        VectorEditOperation::SetOpacity { opacity } => object.opacity = opacity.clamp(0.0, 1.0),
        VectorEditOperation::Delete
        | VectorEditOperation::Duplicate { .. }
        | VectorEditOperation::BringForward
        | VectorEditOperation::SendBackward
        | VectorEditOperation::BringToFront
        | VectorEditOperation::SendToBack
        | VectorEditOperation::GroupWith { .. }
        | VectorEditOperation::Ungroup => {}
    }
    object.bbox = vector_bbox(&object.segments, object.transform);
    Ok(())
}

fn vector_edit_matrix(operation: &VectorEditOperation) -> Option<VectorMatrix> {
    match operation {
        VectorEditOperation::Move { dx, dy } => Some(VectorMatrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: *dx,
            f: *dy,
        }),
        VectorEditOperation::Scale { sx, sy, origin } => Some(around_origin(
            VectorMatrix {
                a: *sx,
                b: 0.0,
                c: 0.0,
                d: *sy,
                e: 0.0,
                f: 0.0,
            },
            *origin,
        )),
        VectorEditOperation::Rotate { degrees, origin } => {
            let radians = degrees.to_radians();
            Some(around_origin(
                VectorMatrix {
                    a: radians.cos(),
                    b: radians.sin(),
                    c: -radians.sin(),
                    d: radians.cos(),
                    e: 0.0,
                    f: 0.0,
                },
                *origin,
            ))
        }
        VectorEditOperation::Skew {
            x_degrees,
            y_degrees,
        } => Some(VectorMatrix {
            a: 1.0,
            b: y_degrees.to_radians().tan(),
            c: x_degrees.to_radians().tan(),
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }),
        VectorEditOperation::MirrorHorizontal { axis_x } => Some(VectorMatrix {
            a: -1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 2.0 * axis_x,
            f: 0.0,
        }),
        VectorEditOperation::MirrorVertical { axis_y } => Some(VectorMatrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            e: 0.0,
            f: 2.0 * axis_y,
        }),
        _ => None,
    }
}

fn around_origin(matrix: VectorMatrix, origin: InkPoint) -> VectorMatrix {
    VectorMatrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: origin.x,
        f: origin.y,
    }
    .multiply(matrix)
    .multiply(VectorMatrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: -origin.x,
        f: -origin.y,
    })
}

fn apply_vector_transform(object: &mut EditableVectorObject, matrix: VectorMatrix) {
    object.transform = matrix.multiply(object.transform);
    object.bbox = vector_bbox(&object.segments, object.transform);
}

fn edit_vector_segment_point(
    segment: &mut VectorPathSegment,
    point_index: usize,
    value: InkPoint,
) -> Result<()> {
    match (segment, point_index) {
        (VectorPathSegment::MoveTo { point }, 0) | (VectorPathSegment::LineTo { point }, 0) => {
            *point = value
        }
        (VectorPathSegment::CubicTo { control1, .. }, 0) => *control1 = value,
        (VectorPathSegment::CubicTo { control2, .. }, 1) => *control2 = value,
        (VectorPathSegment::CubicTo { point, .. }, 2) => *point = value,
        (VectorPathSegment::Rectangle { x, y, .. }, 0) => {
            *x = value.x;
            *y = value.y;
        }
        _ => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "advanced_editing vector point index {point_index} is invalid for selected segment"
            )))
        }
    }
    Ok(())
}

fn serialize_vector_object(object: &EditableVectorObject) -> Vec<u8> {
    let mut output = String::from("q\n");
    output.push_str(&format_matrix(object.transform));
    output.push_str(" cm\n");
    output.push_str(&format!(
        "{} w\n{} J\n{} j\n{} M\n",
        fmt_num(object.stroke.width),
        object.stroke.cap,
        object.stroke.join,
        fmt_num(object.stroke.miter_limit)
    ));
    output.push('[');
    for (index, value) in object.stroke.dash.iter().enumerate() {
        if index > 0 {
            output.push(' ');
        }
        output.push_str(&fmt_num(*value));
    }
    output.push_str(&format!("] {} d\n", fmt_num(object.stroke.dash_phase)));
    output.push_str(&serialize_vector_color(&object.stroke_color, true));
    output.push_str(&serialize_vector_color(&object.fill_color, false));
    if let Some(name) = &object.ext_g_state {
        output.push_str(&format!("/{} gs\n", name));
    }
    for segment in &object.segments {
        match segment {
            VectorPathSegment::MoveTo { point } => {
                output.push_str(&format!("{} {} m\n", fmt_num(point.x), fmt_num(point.y)))
            }
            VectorPathSegment::LineTo { point } => {
                output.push_str(&format!("{} {} l\n", fmt_num(point.x), fmt_num(point.y)))
            }
            VectorPathSegment::CubicTo {
                control1,
                control2,
                point,
            } => output.push_str(&format!(
                "{} {} {} {} {} {} c\n",
                fmt_num(control1.x),
                fmt_num(control1.y),
                fmt_num(control2.x),
                fmt_num(control2.y),
                fmt_num(point.x),
                fmt_num(point.y)
            )),
            VectorPathSegment::Rectangle {
                x,
                y,
                width,
                height,
            } => output.push_str(&format!(
                "{} {} {} {} re\n",
                fmt_num(*x),
                fmt_num(*y),
                fmt_num(*width),
                fmt_num(*height)
            )),
            VectorPathSegment::Close => output.push_str("h\n"),
        }
    }
    if object.clipping_path {
        output.push_str(match object.fill_rule {
            VectorFillRule::Nonzero => "W\n",
            VectorFillRule::EvenOdd => "W*\n",
        });
    }
    output.push_str(match object.paint_mode {
        VectorPaintMode::Stroke => "S\n",
        VectorPaintMode::FillNonzero => "f\n",
        VectorPaintMode::FillEvenOdd => "f*\n",
        VectorPaintMode::FillStrokeNonzero => "B\n",
        VectorPaintMode::FillStrokeEvenOdd => "B*\n",
        VectorPaintMode::EndPath => "n\n",
    });
    output.push('Q');
    output.into_bytes()
}

fn serialize_vector_color(color: &VectorColor, stroke: bool) -> String {
    let operator = match (color.color_space.as_str(), stroke) {
        ("DeviceGray", true) => "G",
        ("DeviceGray", false) => "g",
        ("DeviceRGB", true) => "RG",
        ("DeviceRGB", false) => "rg",
        ("DeviceCMYK", true) => "K",
        ("DeviceCMYK", false) => "k",
        (_, true) => "SCN",
        (_, false) => "scn",
    };
    format!(
        "{} {}\n",
        color
            .components
            .iter()
            .map(|value| fmt_num(*value))
            .collect::<Vec<_>>()
            .join(" "),
        operator
    )
}

fn format_matrix(matrix: VectorMatrix) -> String {
    [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f]
        .iter()
        .map(|value| fmt_num(*value))
        .collect::<Vec<_>>()
        .join(" ")
}

fn fmt_num(value: f64) -> String {
    let value = canonical_number(value);
    if value.fract().abs() <= EPSILON {
        format!("{value:.0}")
    } else {
        let mut text = format!("{value:.6}");
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        text
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InkPoint {
    pub x: f64,
    pub y: f64,
}

impl InkPoint {
    fn add(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }
    fn sub(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }
    fn scale(self, factor: f64) -> Self {
        Self {
            x: self.x * factor,
            y: self.y * factor,
        }
    }
    fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }
    fn length(self) -> f64 {
        self.dot(self).sqrt()
    }
    fn distance(self, other: Self) -> f64 {
        self.sub(other).length()
    }
    fn normalized(self) -> Self {
        let length = self.length();
        if length <= EPSILON {
            Self { x: 0.0, y: 0.0 }
        } else {
            self.scale(1.0 / length)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CubicBezier {
    pub p0: InkPoint,
    pub p1: InkPoint,
    pub p2: InkPoint,
    pub p3: InkPoint,
}

impl CubicBezier {
    pub fn evaluate(self, t: f64) -> InkPoint {
        let t = t.clamp(0.0, 1.0);
        let mt = 1.0 - t;
        self.p0
            .scale(mt * mt * mt)
            .add(self.p1.scale(3.0 * mt * mt * t))
            .add(self.p2.scale(3.0 * mt * t * t))
            .add(self.p3.scale(t * t * t))
    }

    fn first_derivative(self, t: f64) -> InkPoint {
        let mt = 1.0 - t;
        self.p1
            .sub(self.p0)
            .scale(3.0 * mt * mt)
            .add(self.p2.sub(self.p1).scale(6.0 * mt * t))
            .add(self.p3.sub(self.p2).scale(3.0 * t * t))
    }

    fn second_derivative(self, t: f64) -> InkPoint {
        self.p2
            .sub(self.p1.scale(2.0))
            .add(self.p0)
            .scale(6.0 * (1.0 - t))
            .add(self.p3.sub(self.p2.scale(2.0)).add(self.p1).scale(6.0 * t))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InkFitPolicy {
    PreserveRaw,
    FittedOnly,
    RawPlusFitted,
    FitOnImport,
    FitOnAppearanceGeneration,
    Disabled,
    StrictErrorThreshold,
    PerformanceThreshold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InkFitOptions {
    pub policy: InkFitPolicy,
    pub error_threshold: f64,
    pub minimum_distance: f64,
    pub collinear_tolerance: f64,
    pub smoothing_passes: usize,
    pub douglas_peucker_tolerance: f64,
    pub corner_angle_degrees: f64,
    pub closed: bool,
    pub max_recursion: usize,
    pub max_segments: usize,
    pub max_points: usize,
    pub newton_iterations: usize,
    pub performance_threshold_ms: Option<u64>,
}

impl Default for InkFitOptions {
    fn default() -> Self {
        Self {
            policy: InkFitPolicy::RawPlusFitted,
            error_threshold: 0.75,
            minimum_distance: 0.05,
            collinear_tolerance: 0.01,
            smoothing_passes: 0,
            douglas_peucker_tolerance: 0.10,
            corner_angle_degrees: 48.0,
            closed: false,
            max_recursion: 24,
            max_segments: 10_000,
            max_points: 100_000,
            newton_iterations: 4,
            performance_threshold_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InkFitReport {
    pub schema_version: String,
    pub status: AdvancedEditingSupportStatus,
    pub policy: InkFitPolicy,
    pub points_before: usize,
    pub points_after_cleanup: usize,
    pub points_after_simplification: usize,
    pub segment_count: usize,
    pub maximum_deviation: f64,
    pub rms_deviation: f64,
    pub compression_ratio: f64,
    pub fit_time_micros: u128,
    pub recursion_depth: usize,
    pub closed: bool,
    pub output_sha256: String,
    pub deterministic: bool,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InkFitResult {
    pub raw_points: Option<Vec<InkPoint>>,
    pub cleaned_points: Vec<InkPoint>,
    pub fitted_segments: Vec<CubicBezier>,
    pub report: InkFitReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InkStrokeSetResult {
    pub schema_version: String,
    pub strokes: Vec<InkFitResult>,
    pub total_points_before: usize,
    pub total_segments: usize,
    pub deterministic_digest: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnnotationInkFitReport {
    pub schema_version: String,
    pub page: usize,
    pub annotation_index: usize,
    pub annotation_object: u32,
    pub annotation_generation: u16,
    pub appearance_object: u32,
    pub policy: InkFitPolicy,
    pub strokes: Vec<InkFitReport>,
    pub raw_points_preserved: bool,
    pub fitted_curves_stored: bool,
    pub fitted_appearance_generated: bool,
    pub output_reopened: bool,
    pub appearance_readback: bool,
    pub original_prefix_preserved: bool,
    pub output_sha256: String,
    pub signature_policy: EditPolicyReport,
    pub cryptographic_validity_claimed: bool,
    pub deterministic: bool,
    pub cache_invalidation: CacheInvalidationReport,
    pub exact_limits: Vec<String>,
}

/// Proof-bearing geometry update for one explicit source-owned Link
/// annotation. The action dictionary is copied unchanged; only `/Rect` and
/// existing `/QuadPoints` move by the caller-approved delta.
#[derive(Debug, Clone, Serialize)]
pub struct LinkAnnotationMoveReport {
    pub schema_version: String,
    pub page: usize,
    pub annotation_index: usize,
    pub annotation_object: u32,
    pub annotation_generation: u16,
    pub before_rect: [f64; 4],
    pub after_rect: [f64; 4],
    pub moved_quad_points: bool,
    pub action_or_destination_preserved: bool,
    pub output_reopened: bool,
    pub original_prefix_preserved: bool,
    pub output_sha256: String,
    pub signature_policy: EditPolicyReport,
    pub cryptographic_validity_claimed: bool,
    pub deterministic: bool,
    pub cache_invalidation: CacheInvalidationReport,
    pub exact_limits: Vec<String>,
}

/// Move one explicitly identified `/Link` annotation with a text region.
///
/// This is intentionally an annotation-geometry primitive, not a heuristic
/// association engine. Callers must provide the exact expected source rect and
/// an approved finite delta. That makes stale snapshots, wrong page indexes,
/// and links no longer associated with the selected source fail before the
/// canonical incremental writer changes any object.
pub fn move_link_annotation_rect_pdf(
    input: &[u8],
    page_number: usize,
    annotation_index: usize,
    expected_before_rect: [f64; 4],
    dx: f64,
    dy: f64,
    signature_policy_override: bool,
) -> Result<(Vec<u8>, LinkAnnotationMoveReport)> {
    if !dx.is_finite()
        || !dy.is_finite()
        || (dx.abs() <= EPSILON && dy.abs() <= EPSILON)
        || expected_before_rect.iter().any(|value| !value.is_finite())
    {
        return Err(WellfriendError::invalid_input(
            "advanced_editing link-annotation move requires finite expected geometry and a non-zero finite delta",
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::AnnotationUpdate)?;
    enforce_advanced_editing_signature_policy(
        &signature_policy,
        signature_policy_override,
        "Link annotation rectangle move",
    )?;
    let page_box = engine.page_box(page_number)?;
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let page_dict = page_object.as_dict().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "advanced_editing Link annotation page is not a dictionary".to_string(),
        )
    })?;
    let annots = reader.resolve(page_dict.get("Annots").cloned().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "advanced_editing Link annotation move requires a page /Annots array".to_string(),
        )
    })?)?;
    let annotation_ref = annots
        .as_array()
        .and_then(|items| items.get(annotation_index))
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "advanced_editing Link annotation {annotation_index} on page {page_number} must be an indirect annotation dictionary"
            ))
        })?;
    let annotation_object = reader.get_object(annotation_ref.0, annotation_ref.1)?;
    let mut annotation = annotation_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "advanced_editing Link annotation reference is not a dictionary".to_string(),
        )
    })?;
    if annotation.get_name("Subtype") != Some("Link") {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing annotation {annotation_index} on page {page_number} is not /Subtype /Link"
        )));
    }
    if annotation.get("A").is_none() && annotation.get("Dest").is_none() {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing Link annotation move refuses a Link without an action or destination to preserve"
                .to_string(),
        ));
    }
    let before_values = pdf_number_array(reader, annotation.get("Rect"))?;
    let before_rect = normalized_annotation_rect(&before_values)?;
    let expected_rect = normalized_annotation_rect(&expected_before_rect)?;
    if before_rect
        .iter()
        .zip(expected_rect.iter())
        .any(|(actual, expected)| (actual - expected).abs() > EPSILON)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing stale_snapshot: Link annotation rectangle no longer matches the explicit source-associated expected rectangle"
                .to_string(),
        ));
    }
    let mut after_values = before_values;
    for (index, value) in after_values.iter_mut().enumerate() {
        *value += if index % 2 == 0 { dx } else { dy };
    }
    let after_rect = normalized_annotation_rect(&after_values)?;
    if after_rect[0] < page_box[0] - EPSILON
        || after_rect[1] < page_box[1] - EPSILON
        || after_rect[2] > page_box[2] + EPSILON
        || after_rect[3] > page_box[3] + EPSILON
    {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing constraint_infeasible: moved Link annotation rectangle would leave the canonical page box"
                .to_string(),
        ));
    }
    annotation.insert(
        "Rect",
        PdfObject::Array(
            after_values
                .iter()
                .map(|value| PdfObject::Real(canonical_number(*value)))
                .collect(),
        ),
    );
    let moved_quad_points = if annotation.get("QuadPoints").is_some() {
        let mut quad_points = pdf_number_array(reader, annotation.get("QuadPoints"))?;
        if quad_points.len() % 8 != 0 {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing Link annotation /QuadPoints must contain complete quadrilaterals"
                    .to_string(),
            ));
        }
        for (index, value) in quad_points.iter_mut().enumerate() {
            *value += if index % 2 == 0 { dx } else { dy };
        }
        annotation.insert(
            "QuadPoints",
            PdfObject::Array(
                quad_points
                    .iter()
                    .map(|value| PdfObject::Real(canonical_number(*value)))
                    .collect(),
            ),
        );
        true
    } else {
        false
    };
    let output = write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: annotation_ref.0,
            generation: annotation_ref.1,
            object: PdfObject::Dictionary(annotation),
        }],
    )?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let reopened_annotation = reopened
        .document()
        .reader()
        .get_object(annotation_ref.0, annotation_ref.1)?;
    let reopened_annotation = reopened_annotation.as_dict().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "advanced_editing moved Link annotation did not reopen as a dictionary".to_string(),
        )
    })?;
    let reopened_rect = normalized_annotation_rect(&pdf_number_array(
        reopened.document().reader(),
        reopened_annotation.get("Rect"),
    )?)?;
    if reopened_rect
        .iter()
        .zip(after_rect.iter())
        .any(|(actual, expected)| (actual - expected).abs() > EPSILON)
        || reopened_annotation.get_name("Subtype") != Some("Link")
        || (reopened_annotation.get("A").is_none() && reopened_annotation.get("Dest").is_none())
        || !output.starts_with(input)
    {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing Link annotation move failed reopen, action-preservation, or prefix proof"
                .to_string(),
        ));
    }
    Ok((
        output.clone(),
        LinkAnnotationMoveReport {
            schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
            page: page_number,
            annotation_index,
            annotation_object: annotation_ref.0,
            annotation_generation: annotation_ref.1,
            before_rect,
            after_rect,
            moved_quad_points,
            action_or_destination_preserved: true,
            output_reopened: true,
            original_prefix_preserved: true,
            output_sha256: format!("{:x}", Sha256::digest(&output)),
            signature_policy,
            cryptographic_validity_claimed: false,
            deterministic: true,
            cache_invalidation: advanced_editing_cache_invalidation_with_render_write_set(
                input,
                &output,
                false,
                false,
                true,
                AdvancedEditingRenderInvalidation {
                    changed_object_refs: vec![advanced_editing_object_ref(
                        annotation_ref.0,
                        annotation_ref.1,
                    )],
                    created_object_refs: Vec::new(),
                    removed_object_refs: Vec::new(),
                    affected_pages: vec![page_number],
                    dirty_regions: annotation_move_dirty_regions(page_number, before_rect, after_rect),
                },
            ),
            exact_limits: vec![
                "only one caller-identified indirect /Link annotation on the edited page is moved; widgets, replies, non-Link annotations, and page changes are refused".to_string(),
                "the source-associated expected rectangle must match exactly, the target must remain within the canonical page box, and existing /A or /Dest is preserved without interpretation".to_string(),
                "this primitive updates /Rect and existing /QuadPoints only; annotation appearance regeneration, arbitrary action repair, and cross-page retargeting remain separate transactions".to_string(),
            ],
        },
    ))
}

/// Fit an indirect Ink annotation, preserve raw geometry according to policy,
/// store deterministic cubic control points, and regenerate its appearance.
pub fn fit_annotation_ink_pdf(
    input: &[u8],
    page_number: usize,
    annotation_index: usize,
    options: &InkFitOptions,
    signature_policy_override: bool,
) -> Result<(Vec<u8>, AnnotationInkFitReport)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::AnnotationUpdate)?;
    enforce_advanced_editing_signature_policy(
        &signature_policy,
        signature_policy_override,
        "ink annotation fitting",
    )?;
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let page_dict = page_object.as_dict().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "advanced_editing page object is not a dictionary".to_string(),
        )
    })?;
    let annots_object = page_dict.get("Annots").ok_or_else(|| {
        WellfriendError::UnsupportedFeature(format!(
            "advanced_editing page {page_number} has no annotations"
        ))
    })?;
    let annots = reader.resolve(annots_object.clone())?;
    let annotation_ref = annots
        .as_array()
        .and_then(|items| items.get(annotation_index))
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "advanced_editing annotation {annotation_index} on page {page_number} must be an indirect annotation dictionary"
            ))
        })?;
    let annotation_object = reader.get_object(annotation_ref.0, annotation_ref.1)?;
    let mut annotation = annotation_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "advanced_editing annotation reference is not a dictionary".to_string(),
        )
    })?;
    if annotation.get_name("Subtype") != Some("Ink") {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing annotation {annotation_index} is not /Subtype /Ink"
        )));
    }
    let raw_strokes = pdf_nested_points(reader, annotation.get("InkList"))?;
    let fitted = fit_ink_strokes(&raw_strokes, options)?;
    let rect = pdf_number_array(reader, annotation.get("Rect"))?;
    if rect.len() < 4 {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing Ink annotation /Rect must contain four finite numbers".to_string(),
        ));
    }
    let x0 = rect[0].min(rect[2]);
    let y0 = rect[1].min(rect[3]);
    let width = (rect[2] - rect[0]).abs().max(0.1);
    let height = (rect[3] - rect[1]).abs().max(0.1);
    let preserve_raw = !matches!(options.policy, InkFitPolicy::FittedOnly);
    if preserve_raw {
        annotation.insert("WellfriendRawInkList", points_to_pdf_object(&raw_strokes));
    } else {
        annotation.insert(
            "InkList",
            points_to_pdf_object(
                &fitted
                    .strokes
                    .iter()
                    .map(|stroke| stroke.cleaned_points.clone())
                    .collect::<Vec<_>>(),
            ),
        );
    }
    annotation.insert("WellfriendFittedInk", curves_to_pdf_object(&fitted));
    annotation.insert(
        "WellfriendInkFitPolicy",
        PdfObject::Name(format!("{:?}", options.policy)),
    );
    let appearance_number = next_advanced_object_number(reader, "advanced_editing ink appearance")?;
    let opacity = annotation
        .get("CA")
        .and_then(PdfObject::as_number)
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let color = annotation
        .get("C")
        .and_then(PdfObject::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(PdfObject::as_number)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![0.0, 0.0, 0.0]);
    let border_width = annotation
        .get("BS")
        .and_then(PdfObject::as_dict)
        .and_then(|dict| dict.get("W"))
        .and_then(PdfObject::as_number)
        .unwrap_or(1.0)
        .clamp(0.1, 72.0);
    let appearance_content = fitted_ink_appearance(&fitted, x0, y0, &color, opacity, border_width);
    let appearance_raw = flate_encode_cancellable(appearance_content.as_bytes(), 6)?;
    let mut gs = crate::PdfDictionary::empty();
    gs.insert("Type", PdfObject::Name("ExtGState".to_string()));
    gs.insert("CA", PdfObject::Real(opacity));
    gs.insert("ca", PdfObject::Real(opacity));
    let mut ext_g_state = crate::PdfDictionary::empty();
    ext_g_state.insert("OxP20GS", PdfObject::Dictionary(gs));
    let mut resources = crate::PdfDictionary::empty();
    resources.insert("ExtGState", PdfObject::Dictionary(ext_g_state));
    let mut appearance_dict = crate::PdfDictionary::empty();
    appearance_dict.insert("Type", PdfObject::Name("XObject".to_string()));
    appearance_dict.insert("Subtype", PdfObject::Name("Form".to_string()));
    appearance_dict.insert("FormType", PdfObject::Integer(1));
    appearance_dict.insert(
        "BBox",
        PdfObject::Array(vec![
            PdfObject::Real(0.0),
            PdfObject::Real(0.0),
            PdfObject::Real(width),
            PdfObject::Real(height),
        ]),
    );
    appearance_dict.insert("Resources", PdfObject::Dictionary(resources));
    appearance_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    appearance_dict.insert("Length", PdfObject::Integer(appearance_raw.len() as i64));
    let mut ap = crate::PdfDictionary::empty();
    ap.insert(
        "N",
        PdfObject::Reference {
            number: appearance_number,
            generation: 0,
        },
    );
    annotation.insert("AP", PdfObject::Dictionary(ap));
    let output = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: annotation_ref.0,
                generation: annotation_ref.1,
                object: PdfObject::Dictionary(annotation),
            },
            IncrementalObject {
                number: appearance_number,
                generation: 0,
                object: PdfObject::Stream {
                    dict: appearance_dict,
                    raw: appearance_raw,
                },
            },
        ],
    )?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let reopened_page = reopened.document().get_page(page_number)?;
    let reopened_page_object = reopened
        .document()
        .reader()
        .get_object(reopened_page.object_number, reopened_page.generation_number)?;
    let readback = reopened_page_object
        .as_dict()
        .and_then(|dict| dict.get("Annots"))
        .and_then(|annots| reopened.document().reader().resolve(annots.clone()).ok())
        .and_then(|annots| {
            annots
                .as_array()
                .and_then(|items| items.get(annotation_index))
                .cloned()
        })
        .and_then(|annotation| reopened.document().reader().resolve(annotation).ok())
        .and_then(|annotation| annotation.as_dict().cloned())
        .is_some_and(|dict| dict.get("AP").is_some() && dict.get("WellfriendFittedInk").is_some());
    if !readback || !output.starts_with(input) {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing fitted Ink annotation failed incremental readback verification"
                .to_string(),
        ));
    }
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    Ok((
        output.clone(),
        AnnotationInkFitReport {
            schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
            page: page_number,
            annotation_index,
            annotation_object: annotation_ref.0,
            annotation_generation: annotation_ref.1,
            appearance_object: appearance_number,
            policy: options.policy,
            strokes: fitted.strokes.iter().map(|stroke| stroke.report.clone()).collect(),
            raw_points_preserved: preserve_raw,
            fitted_curves_stored: true,
            fitted_appearance_generated: true,
            output_reopened: true,
            appearance_readback: readback,
            original_prefix_preserved: output.starts_with(input),
            output_sha256,
            signature_policy,
            cryptographic_validity_claimed: false,
            deterministic: true,
            cache_invalidation: advanced_editing_cache_invalidation_with_render_write_set(
                input,
                &output,
                false,
                true,
                true,
                AdvancedEditingRenderInvalidation {
                    changed_object_refs: vec![advanced_editing_object_ref(
                        annotation_ref.0,
                        annotation_ref.1,
                    )],
                    created_object_refs: vec![advanced_editing_object_ref(appearance_number, 0)],
                    removed_object_refs: Vec::new(),
                    affected_pages: vec![page_number],
                    dirty_regions: vec![advanced_editing_dirty_region(
                        page_number,
                        [x0, y0, x0 + width, y0 + height],
                        "annotation_ink_appearance_regenerated",
                    )],
                },
            ),
            exact_limits: vec![
                "PDF /InkList remains a point-list interchange surface; cubic control points are stored in /WellfriendFittedInk and consumed by the generated appearance".to_string(),
                "raw points are retained in /WellfriendRawInkList except under fitted_only policy".to_string(),
                "incremental annotation and appearance updates do not assert cryptographic signature validity or viewer acceptance".to_string(),
            ],
        },
    ))
}

/// Fit a deterministic, bounded cubic Bézier representation to one ink stroke.
pub fn fit_ink_stroke(points: &[InkPoint], options: &InkFitOptions) -> Result<InkFitResult> {
    validate_ink_options(options)?;
    if points.len() > options.max_points.min(MAX_ADVANCED_EDITING_INK_POINTS) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing ink stroke has {} points; limit is {}",
            points.len(),
            options.max_points.min(MAX_ADVANCED_EDITING_INK_POINTS)
        )));
    }
    for (index, point) in points.iter().enumerate() {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(WellfriendError::MalformedPdf(format!(
                "advanced_editing ink point {index} is NaN or infinite"
            )));
        }
        if point.x.abs() > 1.0e9 || point.y.abs() > 1.0e9 {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "advanced_editing ink point {index} exceeds bounded coordinate range +/-1e9"
            )));
        }
    }
    let started = Instant::now();
    let mut cleaned = cleanup_points(points, options);
    if options.closed && cleaned.len() > 2 && cleaned.first() != cleaned.last() {
        cleaned.push(cleaned[0]);
    }
    let simplified = simplify_preserving_corners(&cleaned, options);
    let mut segments = Vec::new();
    let mut recursion_depth = 0usize;
    if options.policy != InkFitPolicy::Disabled
        && options.policy != InkFitPolicy::PreserveRaw
        && simplified.len() >= 2
    {
        let left = estimate_left_tangent(&simplified, 0);
        let right = estimate_right_tangent(&simplified, simplified.len() - 1);
        fit_cubic_recursive(
            &simplified,
            0,
            simplified.len() - 1,
            left,
            right,
            options,
            0,
            &mut recursion_depth,
            &mut segments,
        )?;
    }
    if segments.len() > options.max_segments.min(MAX_ADVANCED_EDITING_INK_SEGMENTS) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing fitted segment count {} exceeds limit {}",
            segments.len(),
            options.max_segments.min(MAX_ADVANCED_EDITING_INK_SEGMENTS)
        )));
    }
    let (maximum_deviation, rms_deviation) = curve_error_metrics(&cleaned, &segments);
    if options.policy == InkFitPolicy::StrictErrorThreshold
        && maximum_deviation > options.error_threshold + EPSILON
    {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing ink fit maximum deviation {:.6} exceeds strict threshold {:.6}",
            maximum_deviation, options.error_threshold
        )));
    }
    let elapsed = started.elapsed();
    if options.policy == InkFitPolicy::PerformanceThreshold {
        if let Some(limit) = options.performance_threshold_ms {
            if elapsed.as_millis() > u128::from(limit) {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "advanced_editing ink fit elapsed {} ms exceeds performance threshold {limit} ms",
                    elapsed.as_millis()
                )));
            }
        }
    }
    let digest = ink_digest(&segments);
    let raw_points = matches!(
        options.policy,
        InkFitPolicy::PreserveRaw
            | InkFitPolicy::RawPlusFitted
            | InkFitPolicy::FitOnImport
            | InkFitPolicy::FitOnAppearanceGeneration
            | InkFitPolicy::StrictErrorThreshold
            | InkFitPolicy::PerformanceThreshold
    )
    .then(|| points.to_vec());
    Ok(InkFitResult {
        raw_points,
        cleaned_points: simplified.clone(),
        fitted_segments: segments.clone(),
        report: InkFitReport {
            schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
            status: AdvancedEditingSupportStatus::ImplementedWithLimits,
            policy: options.policy,
            points_before: points.len(),
            points_after_cleanup: cleaned.len(),
            points_after_simplification: simplified.len(),
            segment_count: segments.len(),
            maximum_deviation: canonical_number(maximum_deviation),
            rms_deviation: canonical_number(rms_deviation),
            compression_ratio: if segments.is_empty() {
                0.0
            } else {
                canonical_number(points.len() as f64 / segments.len() as f64)
            },
            fit_time_micros: elapsed.as_micros(),
            recursion_depth,
            closed: options.closed,
            output_sha256: digest,
            deterministic: true,
            exact_limits: vec![
                "fit records geometry only; pressure, tilt, velocity, and pen timing are not reconstructed".to_string(),
                "error is measured against the cleaned raw polyline in input coordinate space".to_string(),
                "recursion, points, segments, coordinates, and Newton iterations are capped".to_string(),
            ],
        },
    })
}

pub fn fit_ink_strokes(
    strokes: &[Vec<InkPoint>],
    options: &InkFitOptions,
) -> Result<InkStrokeSetResult> {
    let total_points = strokes.iter().try_fold(0usize, |total, stroke| {
        total.checked_add(stroke.len()).ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "advanced_editing ink point count overflow".to_string(),
            )
        })
    })?;
    if total_points > options.max_points.min(MAX_ADVANCED_EDITING_INK_POINTS) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing ink stroke set has {total_points} points; aggregate limit is {}",
            options.max_points.min(MAX_ADVANCED_EDITING_INK_POINTS)
        )));
    }
    let mut results = Vec::with_capacity(strokes.len());
    for stroke in strokes {
        results.push(fit_ink_stroke(stroke, options)?);
    }
    let total_segments = results
        .iter()
        .map(|result| result.fitted_segments.len())
        .sum();
    let mut hasher = Sha256::new();
    for result in &results {
        hasher.update(result.report.output_sha256.as_bytes());
    }
    Ok(InkStrokeSetResult {
        schema_version: ADVANCED_EDITING_SCHEMA_VERSION.to_string(),
        strokes: results,
        total_points_before: total_points,
        total_segments,
        deterministic_digest: format!("{:x}", hasher.finalize()),
    })
}

fn pdf_number_array(reader: &crate::PdfReader, object: Option<&PdfObject>) -> Result<Vec<f64>> {
    let object = object.ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "advanced_editing required numeric array is missing".to_string(),
        )
    })?;
    let resolved = reader.resolve(object.clone())?;
    let values = resolved.as_array().ok_or_else(|| {
        WellfriendError::MalformedPdf("advanced_editing expected numeric array".to_string())
    })?;
    values
        .iter()
        .map(|value| {
            reader
                .resolve(value.clone())?
                .as_number()
                .filter(|number| number.is_finite())
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "advanced_editing numeric array contains a non-finite or non-number value"
                            .to_string(),
                    )
                })
        })
        .collect()
}

fn normalized_annotation_rect(values: &[f64]) -> Result<[f64; 4]> {
    if values.len() != 4 || values.iter().any(|value| !value.is_finite()) {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing annotation /Rect must contain exactly four finite numbers"
                .to_string(),
        ));
    }
    let rect = [
        values[0].min(values[2]),
        values[1].min(values[3]),
        values[0].max(values[2]),
        values[1].max(values[3]),
    ];
    if rect[2] <= rect[0] || rect[3] <= rect[1] {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing annotation /Rect must have positive normalized width and height"
                .to_string(),
        ));
    }
    Ok(rect)
}

fn pdf_nested_points(
    reader: &crate::PdfReader,
    object: Option<&PdfObject>,
) -> Result<Vec<Vec<InkPoint>>> {
    let object = object.ok_or_else(|| {
        WellfriendError::MalformedPdf("advanced_editing Ink annotation has no /InkList".to_string())
    })?;
    let resolved = reader.resolve(object.clone())?;
    let strokes = resolved.as_array().ok_or_else(|| {
        WellfriendError::MalformedPdf("advanced_editing /InkList is not an array".to_string())
    })?;
    let mut output = Vec::with_capacity(strokes.len());
    for (stroke_index, stroke) in strokes.iter().enumerate() {
        let numbers = pdf_number_array(reader, Some(stroke))?;
        if numbers.len() % 2 != 0 {
            return Err(WellfriendError::MalformedPdf(format!(
                "advanced_editing /InkList stroke {stroke_index} has an odd coordinate count"
            )));
        }
        output.push(
            numbers
                .chunks_exact(2)
                .map(|pair| InkPoint {
                    x: pair[0],
                    y: pair[1],
                })
                .collect(),
        );
    }
    Ok(output)
}

fn points_to_pdf_object(strokes: &[Vec<InkPoint>]) -> PdfObject {
    PdfObject::Array(
        strokes
            .iter()
            .map(|stroke| {
                PdfObject::Array(
                    stroke
                        .iter()
                        .flat_map(|point| {
                            [
                                PdfObject::Real(canonical_number(point.x)),
                                PdfObject::Real(canonical_number(point.y)),
                            ]
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

fn curves_to_pdf_object(fitted: &InkStrokeSetResult) -> PdfObject {
    PdfObject::Array(
        fitted
            .strokes
            .iter()
            .map(|stroke| {
                PdfObject::Array(
                    stroke
                        .fitted_segments
                        .iter()
                        .map(|segment| {
                            PdfObject::Array(
                                [segment.p0, segment.p1, segment.p2, segment.p3]
                                    .into_iter()
                                    .flat_map(|point| {
                                        [
                                            PdfObject::Real(canonical_number(point.x)),
                                            PdfObject::Real(canonical_number(point.y)),
                                        ]
                                    })
                                    .collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

fn fitted_ink_appearance(
    fitted: &InkStrokeSetResult,
    x0: f64,
    y0: f64,
    color: &[f64],
    _opacity: f64,
    width: f64,
) -> String {
    let rgb = match color {
        [gray] => [*gray, *gray, *gray],
        [r, g, b, ..] => [*r, *g, *b],
        _ => [0.0, 0.0, 0.0],
    };
    let mut content = format!(
        "q /OxP20GS gs\n{} {} {} RG\n{} w 1 J 1 j\n",
        fmt_num(rgb[0].clamp(0.0, 1.0)),
        fmt_num(rgb[1].clamp(0.0, 1.0)),
        fmt_num(rgb[2].clamp(0.0, 1.0)),
        fmt_num(width)
    );
    for stroke in &fitted.strokes {
        if let Some(first) = stroke.fitted_segments.first() {
            content.push_str(&format!(
                "{} {} m\n",
                fmt_num(first.p0.x - x0),
                fmt_num(first.p0.y - y0)
            ));
            for curve in &stroke.fitted_segments {
                content.push_str(&format!(
                    "{} {} {} {} {} {} c\n",
                    fmt_num(curve.p1.x - x0),
                    fmt_num(curve.p1.y - y0),
                    fmt_num(curve.p2.x - x0),
                    fmt_num(curve.p2.y - y0),
                    fmt_num(curve.p3.x - x0),
                    fmt_num(curve.p3.y - y0)
                ));
            }
            if stroke.report.closed {
                content.push_str("h\n");
            }
            content.push_str("S\n");
        }
    }
    content.push('Q');
    content
}

fn validate_ink_options(options: &InkFitOptions) -> Result<()> {
    for (name, value) in [
        ("error_threshold", options.error_threshold),
        ("minimum_distance", options.minimum_distance),
        ("collinear_tolerance", options.collinear_tolerance),
        (
            "douglas_peucker_tolerance",
            options.douglas_peucker_tolerance,
        ),
        ("corner_angle_degrees", options.corner_angle_degrees),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(WellfriendError::MalformedPdf(format!(
                "advanced_editing ink {name} must be finite and non-negative"
            )));
        }
    }
    if options.error_threshold <= EPSILON {
        return Err(WellfriendError::MalformedPdf(
            "advanced_editing ink error_threshold must be greater than zero".to_string(),
        ));
    }
    if options.max_recursion > MAX_ADVANCED_EDITING_FIT_RECURSION {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing ink max_recursion {} exceeds hard cap {MAX_ADVANCED_EDITING_FIT_RECURSION}",
            options.max_recursion
        )));
    }
    if options.newton_iterations > 16 {
        return Err(WellfriendError::UnsupportedFeature(
            "advanced_editing ink Newton iteration cap is 16".to_string(),
        ));
    }
    Ok(())
}

fn cleanup_points(points: &[InkPoint], options: &InkFitOptions) -> Vec<InkPoint> {
    let mut filtered = Vec::with_capacity(points.len());
    for point in points.iter().copied() {
        if filtered
            .last()
            .is_none_or(|previous: &InkPoint| previous.distance(point) >= options.minimum_distance)
        {
            filtered.push(point);
        }
    }
    if filtered.len() > 1 && filtered.last() != points.last() {
        filtered.push(*points.last().expect("non-empty checked by branch"));
    }
    let mut collapsed = Vec::with_capacity(filtered.len());
    for point in filtered {
        collapsed.push(point);
        while collapsed.len() >= 3 {
            let n = collapsed.len();
            if point_line_distance(collapsed[n - 2], collapsed[n - 3], collapsed[n - 1])
                <= options.collinear_tolerance
            {
                collapsed.remove(n - 2);
            } else {
                break;
            }
        }
    }
    for _ in 0..options.smoothing_passes.min(8) {
        if collapsed.len() < 3 {
            break;
        }
        let mut smoothed = Vec::with_capacity(collapsed.len());
        smoothed.push(collapsed[0]);
        for window in collapsed.windows(3) {
            smoothed.push(
                window[0]
                    .scale(0.25)
                    .add(window[1].scale(0.5))
                    .add(window[2].scale(0.25)),
            );
        }
        smoothed.push(*collapsed.last().expect("length checked"));
        collapsed = smoothed;
    }
    collapsed
}

fn simplify_preserving_corners(points: &[InkPoint], options: &InkFitOptions) -> Vec<InkPoint> {
    if points.len() <= 2 || options.douglas_peucker_tolerance <= EPSILON {
        return points.to_vec();
    }
    let mut anchors = vec![0usize];
    for index in 1..points.len() - 1 {
        if turn_angle_degrees(points[index - 1], points[index], points[index + 1])
            >= options.corner_angle_degrees
        {
            anchors.push(index);
        }
    }
    anchors.push(points.len() - 1);
    anchors.sort_unstable();
    anchors.dedup();
    let mut output = Vec::new();
    for pair in anchors.windows(2) {
        let mut keep = vec![false; pair[1] - pair[0] + 1];
        keep[0] = true;
        let last = keep.len() - 1;
        keep[last] = true;
        douglas_peucker_mark(
            &points[pair[0]..=pair[1]],
            0,
            last,
            options.douglas_peucker_tolerance,
            &mut keep,
        );
        for (local, point) in points[pair[0]..=pair[1]].iter().enumerate() {
            if keep[local] && (output.last().is_none() || output.last() != Some(point)) {
                output.push(*point);
            }
        }
    }
    output
}

fn douglas_peucker_mark(
    points: &[InkPoint],
    first: usize,
    last: usize,
    tolerance: f64,
    keep: &mut [bool],
) {
    if last <= first + 1 {
        return;
    }
    let mut maximum = 0.0;
    let mut split = first;
    for index in first + 1..last {
        let distance = point_line_distance(points[index], points[first], points[last]);
        if distance > maximum + EPSILON {
            maximum = distance;
            split = index;
        }
    }
    if maximum > tolerance {
        keep[split] = true;
        douglas_peucker_mark(points, first, split, tolerance, keep);
        douglas_peucker_mark(points, split, last, tolerance, keep);
    }
}

#[allow(clippy::too_many_arguments)]
fn fit_cubic_recursive(
    points: &[InkPoint],
    first: usize,
    last: usize,
    left_tangent: InkPoint,
    right_tangent: InkPoint,
    options: &InkFitOptions,
    depth: usize,
    maximum_depth: &mut usize,
    segments: &mut Vec<CubicBezier>,
) -> Result<()> {
    *maximum_depth = (*maximum_depth).max(depth);
    if depth
        > options
            .max_recursion
            .min(MAX_ADVANCED_EDITING_FIT_RECURSION)
    {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing ink fitting exceeded recursion limit {}",
            options
                .max_recursion
                .min(MAX_ADVANCED_EDITING_FIT_RECURSION)
        )));
    }
    if segments.len() >= options.max_segments.min(MAX_ADVANCED_EDITING_INK_SEGMENTS) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "advanced_editing ink fitting exceeded segment limit {}",
            options.max_segments.min(MAX_ADVANCED_EDITING_INK_SEGMENTS)
        )));
    }
    if last <= first + 1 {
        let distance = points[first].distance(points[last]) / 3.0;
        segments.push(CubicBezier {
            p0: points[first],
            p1: points[first].add(left_tangent.scale(distance)),
            p2: points[last].add(right_tangent.scale(distance)),
            p3: points[last],
        });
        return Ok(());
    }
    let slice = &points[first..=last];
    let mut parameters = chord_length_parameters(slice);
    let mut curve = generate_bezier(slice, &parameters, left_tangent, right_tangent);
    let (mut max_error, mut split) = maximum_parameter_error(slice, &curve, &parameters);
    if max_error <= options.error_threshold {
        segments.push(curve);
        return Ok(());
    }
    if max_error <= options.error_threshold * 4.0 {
        for _ in 0..options.newton_iterations {
            parameters = reparameterize(slice, &parameters, &curve);
            if !parameters.windows(2).all(|pair| pair[0] <= pair[1]) {
                break;
            }
            curve = generate_bezier(slice, &parameters, left_tangent, right_tangent);
            let measured = maximum_parameter_error(slice, &curve, &parameters);
            max_error = measured.0;
            split = measured.1;
            if max_error <= options.error_threshold {
                segments.push(curve);
                return Ok(());
            }
        }
    }
    split = split.clamp(1, slice.len() - 2);
    let center_index = first + split;
    let center_tangent = estimate_center_tangent(points, center_index);
    fit_cubic_recursive(
        points,
        first,
        center_index,
        left_tangent,
        center_tangent,
        options,
        depth + 1,
        maximum_depth,
        segments,
    )?;
    fit_cubic_recursive(
        points,
        center_index,
        last,
        center_tangent.scale(-1.0),
        right_tangent,
        options,
        depth + 1,
        maximum_depth,
        segments,
    )
}

fn chord_length_parameters(points: &[InkPoint]) -> Vec<f64> {
    let mut values = Vec::with_capacity(points.len());
    values.push(0.0);
    for pair in points.windows(2) {
        values.push(values.last().copied().unwrap_or(0.0) + pair[0].distance(pair[1]));
    }
    let total = values.last().copied().unwrap_or(0.0);
    if total <= EPSILON {
        let denominator = (points.len().saturating_sub(1)).max(1) as f64;
        return (0..points.len())
            .map(|index| index as f64 / denominator)
            .collect();
    }
    values.iter_mut().for_each(|value| *value /= total);
    values
}

fn generate_bezier(
    points: &[InkPoint],
    parameters: &[f64],
    left: InkPoint,
    right: InkPoint,
) -> CubicBezier {
    let p0 = points[0];
    let p3 = *points.last().expect("non-empty fit slice");
    let mut c00 = 0.0;
    let mut c01 = 0.0;
    let mut c11 = 0.0;
    let mut x0 = 0.0;
    let mut x1 = 0.0;
    for (point, &u) in points.iter().zip(parameters) {
        let mt = 1.0 - u;
        let b0 = mt * mt * mt;
        let b1 = 3.0 * u * mt * mt;
        let b2 = 3.0 * u * u * mt;
        let b3 = u * u * u;
        let a0 = left.scale(b1);
        let a1 = right.scale(b2);
        let residual = point.sub(p0.scale(b0 + b1).add(p3.scale(b2 + b3)));
        c00 += a0.dot(a0);
        c01 += a0.dot(a1);
        c11 += a1.dot(a1);
        x0 += a0.dot(residual);
        x1 += a1.dot(residual);
    }
    let determinant = c00 * c11 - c01 * c01;
    let (mut alpha_left, mut alpha_right) = if determinant.abs() > EPSILON {
        (
            (x0 * c11 - x1 * c01) / determinant,
            (c00 * x1 - c01 * x0) / determinant,
        )
    } else {
        (0.0, 0.0)
    };
    let segment_length = p0.distance(p3);
    let minimum = segment_length * 1.0e-6;
    if !alpha_left.is_finite()
        || !alpha_right.is_finite()
        || alpha_left < minimum
        || alpha_right < minimum
    {
        alpha_left = segment_length / 3.0;
        alpha_right = segment_length / 3.0;
    }
    CubicBezier {
        p0,
        p1: p0.add(left.scale(alpha_left)),
        p2: p3.add(right.scale(alpha_right)),
        p3,
    }
}

fn maximum_parameter_error(
    points: &[InkPoint],
    curve: &CubicBezier,
    parameters: &[f64],
) -> (f64, usize) {
    let mut maximum = 0.0;
    let mut split = points.len() / 2;
    for index in 1..points.len().saturating_sub(1) {
        let distance = curve.evaluate(parameters[index]).distance(points[index]);
        if distance > maximum + EPSILON {
            maximum = distance;
            split = index;
        }
    }
    (maximum, split)
}

fn reparameterize(points: &[InkPoint], parameters: &[f64], curve: &CubicBezier) -> Vec<f64> {
    let mut output = Vec::with_capacity(parameters.len());
    for (&parameter, &point) in parameters.iter().zip(points) {
        let q = curve.evaluate(parameter);
        let q1 = curve.first_derivative(parameter);
        let q2 = curve.second_derivative(parameter);
        let difference = q.sub(point);
        let denominator = q1.dot(q1) + difference.dot(q2);
        let next = if denominator.abs() <= EPSILON {
            parameter
        } else {
            parameter - difference.dot(q1) / denominator
        };
        output.push(next.clamp(0.0, 1.0));
    }
    if let Some(first) = output.first_mut() {
        *first = 0.0;
    }
    if let Some(last) = output.last_mut() {
        *last = 1.0;
    }
    output
}

fn estimate_left_tangent(points: &[InkPoint], index: usize) -> InkPoint {
    points
        .get(index + 1)
        .copied()
        .unwrap_or(points[index])
        .sub(points[index])
        .normalized()
}

fn estimate_right_tangent(points: &[InkPoint], index: usize) -> InkPoint {
    points
        .get(index.wrapping_sub(1))
        .copied()
        .unwrap_or(points[index])
        .sub(points[index])
        .normalized()
}

fn estimate_center_tangent(points: &[InkPoint], index: usize) -> InkPoint {
    points[index - 1].sub(points[index + 1]).normalized()
}

fn curve_error_metrics(points: &[InkPoint], segments: &[CubicBezier]) -> (f64, f64) {
    if points.is_empty() || segments.is_empty() {
        return (0.0, 0.0);
    }
    let mut maximum = 0.0_f64;
    let mut sum_squares = 0.0;
    for point in points {
        let mut minimum = f64::INFINITY;
        for curve in segments {
            // Deterministic bounded distance approximation. The fitter's own
            // acceptance uses chord parameters; this denser sampling reports a
            // conservative post-fit metric without unbounded root solving.
            for sample in 0..=32 {
                let distance = curve.evaluate(sample as f64 / 32.0).distance(*point);
                minimum = minimum.min(distance);
            }
        }
        maximum = maximum.max(minimum);
        sum_squares += minimum * minimum;
    }
    (maximum, (sum_squares / points.len() as f64).sqrt())
}

fn point_line_distance(point: InkPoint, start: InkPoint, end: InkPoint) -> f64 {
    let segment = end.sub(start);
    let length_squared = segment.dot(segment);
    if length_squared <= EPSILON {
        return point.distance(start);
    }
    let t = point.sub(start).dot(segment) / length_squared;
    point.distance(start.add(segment.scale(t.clamp(0.0, 1.0))))
}

fn turn_angle_degrees(previous: InkPoint, current: InkPoint, next: InkPoint) -> f64 {
    let incoming = current.sub(previous).normalized();
    let outgoing = next.sub(current).normalized();
    incoming.dot(outgoing).clamp(-1.0, 1.0).acos().to_degrees()
}

fn ink_digest(segments: &[CubicBezier]) -> String {
    let mut hasher = Sha256::new();
    for segment in segments {
        for point in [segment.p0, segment.p1, segment.p2, segment.p3] {
            hasher.update(canonical_number(point.x).to_le_bytes());
            hasher.update(canonical_number(point.y).to_le_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

pub(crate) fn canonical_number(value: f64) -> f64 {
    if value.abs() < 0.000_000_5 {
        0.0
    } else {
        (value * 1_000_000.0).round() / 1_000_000.0
    }
}

pub fn advanced_editing_report(engine: &ContentEngine) -> Result<serde_json::Value> {
    let page_count = engine.document().page_count()?;
    let mut vector_objects = 0usize;
    let mut vector_diagnostics = Vec::new();
    for page in 1..=page_count.min(1000) {
        match list_vector_objects(engine.document().reader().file_bytes(), page) {
            Ok(inventory) => {
                vector_objects = vector_objects.saturating_add(inventory.objects.len())
            }
            Err(error) => vector_diagnostics.push(format!("page {page}: {error}")),
        }
    }
    Ok(serde_json::json!({
        "schema_version": ADVANCED_EDITING_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "text": {
            "modes": ["safe_patch", "paragraph_reflow_horizontal", "paragraph_reflow_rtl", "paragraph_reflow_vertical", "overlay_fallback", "unsupported"],
            "existing_pdf_glyph_streams_reshaped": false,
            "new_unicode_shaping": "rustybuzz_with_cluster_provenance",
            "rtl": "page_logical_multi_run_shaped_type0_with_actualtext_and_per_glyph_offsets",
            "vertical": "unicode17_mixed_orientation_opentype_ttb_shaping_identity_v_w2_logical_column_breaking_and_bounded_positioned_emission",
            "vertical_orientation_unicode_version": crate::fonts::vertical::UNICODE_VERSION,
            "missing_glyph_policy": "approved_font_substitution_then_fail_closed_if_asset_has_no_outline"
        },
        "same_width_patch": {
            "operators": ["Tj", "TJ", "quote", "double_quote"],
            "representations": ["literal", "hexadecimal"],
            "save": "incremental_stream_object_replacement",
            "prefix_preservation": true,
            "encrypted_incremental": "unsupported_reported_exact"
        },
        "vector": {
            "page_owned_objects": vector_objects,
            "inventory_diagnostics": vector_diagnostics,
            "operators": ["m", "l", "c", "v", "y", "h", "re", "W", "W*", "S", "s", "f", "f*", "B", "B*", "b", "b*", "n"],
            "edits": ["move", "scale", "rotate", "skew", "mirror", "point", "fill", "stroke", "width", "dash", "cap_join", "opacity", "delete", "duplicate", "bring_forward", "send_backward", "bring_to_front", "send_to_back", "group_with", "ungroup"],
            "shared_form_policy": ["reject", "edit_all_uses", "clone_edit_one_instance"],
            "clone_edit_one_limit": "recursive_selected_invocation_chain_with_advanced_editing_closeout_limits",
            "semantic_shape_inference_claimed": false
        },
        "ink": {
            "cleanup": true,
            "douglas_peucker": true,
            "chord_parameterization": true,
            "newton_reparameterization": true,
            "recursive_cubic_fit": true,
            "raw_policy": true,
            "annotation_appearance": "cubic_form_xobject",
            "pen_dynamics_recovered": false
        },
        "signature_policy": analyze_edit_policy(engine, SignatureEditOperation::ContentEdit)?,
        "feature": advanced_editing_feature_report_value(crate::sdk::REPORT_ENVELOPE_VERSION)
    }))
}

pub(crate) fn advanced_editing_feature_report_value(envelope_version: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": ADVANCED_EDITING_SCHEMA_VERSION,
        "envelope_version": envelope_version,
        "status": "implemented_with_limits",
        "coverage": {
            "rtl_vertical_analysis": "implemented_with_provenance",
            "rtl_vertical_serialized_edit": "implemented_for_page_logical_partial_token_and_cross_contents_ranges_with_source_provenance",
            "same_width_patch": "implemented_with_exact_eligibility",
            "vector_page_stream_model_and_edit": "implemented_with_operation_range_rewrite",
            "vector_reachable_form_model": "implemented_depth_8",
            "vector_shared_form_edit_all": "implemented_explicit_policy",
            "vector_shared_form_clone_one": "implemented_recursive_selected_invocation_chain_with_advanced_editing_closeout_limits",
            "vector_annotation_appearances": "implemented_owner_specific_AP_clone_one_for_N_R_D_state_and_nested_Form_paths",
            "vector_z_order": "implemented_page_owned_safe_contexts",
            "vector_group_ungroup": "implemented_contiguous_page_owned_marked_content",
            "ink_cubic_fitting": "implemented_error_bounded_deterministic",
            "ink_annotation_appearance": "implemented_incremental",
            "undo_redo": "incremental_suffix_patch_session_with_checkpoint_fingerprints_and_branch_redo_clearing",
            "signature_policy": "secure_mutation_closeout_preflight_enforced",
            "cache_invalidation": "text_glyph_render_vector_annotation_semantic_search_ocg_writer_flags_with_before_after_fingerprints_plus_structured_annotation_object_page_dirty_region_write_sets"
        },
        "bindings": {
            "rust": "implemented",
            "cli": "shared_json_commands",
            "python": "report_inventory_and_owned_mutation_surface",
            "c_abi": "report_inventory_and_owned_buffer_mutation_surface",
            "wasm": "report_inventory_and_owned_mutation_surface",
            "dotnet": "report_inventory_and_disposable_owned_mutation_surface",
            "java_maven": "report_inventory_and_owned_mutation_surface",
            "java_gradle": "same_java_artifact_mutation_surface"
        },
        "failure": {"blocked": 0, "unclassified": 0, "security": 0},
        "limits": {
            "paragraph_chars": MAX_ADVANCED_EDITING_PARAGRAPH_CHARS,
            "bidi_runs": MAX_ADVANCED_EDITING_BIDI_RUNS,
            "glyphs": MAX_ADVANCED_EDITING_GLYPHS,
            "content_stream_bytes": MAX_ADVANCED_EDITING_PATCH_STREAM_BYTES,
            "vector_objects": 100000,
            "form_recursion": 8,
            "ink_points": MAX_ADVANCED_EDITING_INK_POINTS,
            "ink_segments": MAX_ADVANCED_EDITING_INK_SEGMENTS,
            "ink_recursion": MAX_ADVANCED_EDITING_FIT_RECURSION
        },
        "unsupported_exact": [
            "page-owned paragraphs spanning independent PDF string tokens and /Contents streams use logical scalar provenance; shared Form-owned text still needs an occurrence ownership decision",
            "bundled DejaVu covers Arabic and Hebrew but not arbitrary CJK; vertical Japanese requires a caller-supplied font containing the requested glyphs",
            "same-width patching rejects Type3, shaping, bidi/vertical reorder, clipping text modes, ambiguous CMaps, encryption, and changed encoded/advance structure",
            "reachable shared Forms support explicit edit-all and recursive clone-edit-one for selected invocation chains; pattern program editing and arbitrary shading mesh editing remain exact limits",
            "group/ungroup is bounded to contiguous page-owned vector ranges using inert Wellfriend marked content; cross-stream and Form-owned grouping is rejected",
            "z-order is bounded to page-owned objects outside clipping, marked-content, and OCG contexts",
            "cubic fitting does not recover pressure, tilt, velocity, time, or original pen dynamics",
            "structural incremental preservation never implies cryptographic signature validity or viewer acceptance"
        ]
    })
}

pub(crate) fn advanced_editing_closeout_feature_report_value(
    envelope_version: u32,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "advanced_editing_closeout.multirun-form-appearance-closure.v1",
        "envelope_version": envelope_version,
        "status": "implemented_with_limits",
        "coverage": {
            "multi_run_selection": "page_logical_partial_token_cross_contents_provenance_with_atomic_stream_commit",
            "generated_paint_order": "exact_BT_ET_source_slot_inventory_fail_closed_block_anchor_or_revision_bound_hamilton_proposed_and_explicitly_approved_scalar_region_partitions_with_descending_same_stream_commit_and_unsplit_shaping_equivalence",
            "rtl_logical_visual_mapping": "bidi_run_provenance",
            "vertical_range": "unicode17_vertical_shaping_and_cluster_safe_column_layout_with_explicit_limits",
            "multi_operator_serialization": "Tj_TJ_quote_double_quote_token_sequences_with_boundary_residual_reencoding",
            "preserve_per_segment": "grapheme_safe_source_style_ownership_exact_cmap_or_shaped_type0_with_raw_paint_replay",
            "source_token_removal": "selected_codes_absent_from_current_reachable_revision_with_exact_TJ_advance_compensation",
            "clipping_text": "source_positioned_horizontal_bidi_and_rotated_offset_vertical_replacements_inside_original_BT_ET_scope",
            "inline_position_preservation": "source_text_and_line_matrices_with_font_resolved_TJ_and_cross_contents_state_no_inverse_matrix",
            "tagged_text": "partial_and_nested_marked_content_rewritten_inline_without_mcid_relocation_or_parenttree_identity_change",
            "form_inline_font_resources": "collision_checked_generated_type0_font_installed_in_every_rewritten_form_owner",
            "generated_font_resource_retirement": "private_v1_type0_marker_plus_bounded_page_form_pattern_type3_and_appearance_reachability",
            "opentype_embedding": "glyf_gid_preserving_subset_or_full_cff1_FontFile3_OpenType_with_license_gate",
            "nested_form_clone_one": "recursive_leaf_to_page_invocation_path",
            "annotation_appearance_clone_one": "target_annotation_N_R_D_or_state_owner",
            "widget_state_preservation": "AP_and_AS_preserved_without_field_value_mutation",
            "undo_redo": "advanced_editing_incremental_patch_session",
            "signature_policy": "secure_mutation_closeout_preflight_enforced"
        },
        "bindings": {"rust":"implemented", "cli":"implemented", "python":"implemented", "c_abi":"implemented", "wasm":"implemented_memory_safe_json_and_owned_bytes", "dotnet":"implemented", "java_maven":"implemented", "java_gradle":"implemented"},
        "failure": {"blocked":0, "unclassified":0, "security":0},
        "exact_limits": [
            "Form-owned logical text is occurrence-addressed separately because shared Form edits require clone-one or edit-all ownership",
            "unreviewed physical-region inference and preservation of several unrelated source font-family programs inside one contextual replacement remain explicit limits; replacement ranges have a revision-bound deterministic proposal, inherited grapheme-owned size, spacing, scale, rise, render mode and paint commands are retained, and boundaries that cannot reproduce the unsplit OpenType result are rejected",
            "nested clone-one requires lossless streams and direct or indirect resource dictionaries",
            "arbitrary Type3 CharProc authoring remains separate; text insertion substitutes an approved embeddable Type0 font, and exact page resource paint commands are replayed",
            "structural signature policy does not claim cryptographic validity"
        ]
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheInvalidationDirtyRegion {
    pub page: usize,
    pub region: [f64; 4],
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheInvalidationReport {
    pub text_layout: bool,
    pub glyphs: bool,
    pub render_tiles: bool,
    pub vectors: bool,
    pub annotation_appearances: bool,
    pub semantic: bool,
    pub search_and_rag: bool,
    pub optional_content: bool,
    pub writer: bool,
    pub fingerprint_before: String,
    pub fingerprint_after: String,
    #[serde(default)]
    pub render_write_set_refs: Vec<String>,
    #[serde(default)]
    pub changed_object_refs: Vec<String>,
    #[serde(default)]
    pub created_object_refs: Vec<String>,
    #[serde(default)]
    pub removed_object_refs: Vec<String>,
    #[serde(default)]
    pub affected_pages: Vec<usize>,
    #[serde(default)]
    pub dirty_regions: Vec<CacheInvalidationDirtyRegion>,
    #[serde(default)]
    pub structured_render_write_set: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedEditingMutationPatch {
    pub sequence: usize,
    pub transaction_id: String,
    pub operation: String,
    pub before_bytes: usize,
    pub after_bytes: usize,
    pub appended_bytes: usize,
    pub before_sha256: String,
    pub after_sha256: String,
    pub report: serde_json::Value,
    #[serde(skip)]
    appended_suffix: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedEditingMutationCheckpoint {
    pub sequence: usize,
    pub document_bytes: usize,
    pub document_sha256: String,
    pub patch_count: usize,
}

#[derive(Debug, Clone)]
pub struct AdvancedEditingMutationSession {
    current: Vec<u8>,
    patches: Vec<AdvancedEditingMutationPatch>,
    checkpoints: Vec<AdvancedEditingMutationCheckpoint>,
    cursor: usize,
    max_patches: usize,
    max_total_patch_bytes: usize,
}

impl AdvancedEditingMutationSession {
    pub fn new(input: Vec<u8>) -> Result<Self> {
        ContentEngine::open_bytes(input.clone())?;
        Ok(Self {
            current: input,
            patches: Vec::new(),
            checkpoints: Vec::new(),
            cursor: 0,
            max_patches: 1024,
            max_total_patch_bytes: 512 * 1024 * 1024,
        })
    }

    pub fn with_limits(
        input: Vec<u8>,
        max_patches: usize,
        max_total_patch_bytes: usize,
    ) -> Result<Self> {
        if max_patches == 0 || max_patches > 100_000 || max_total_patch_bytes == 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "advanced_editing mutation session limits are outside the supported range"
                    .to_string(),
            ));
        }
        let mut session = Self::new(input)?;
        session.max_patches = max_patches;
        session.max_total_patch_bytes = max_total_patch_bytes;
        Ok(session)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.current
    }

    pub fn patches(&self) -> &[AdvancedEditingMutationPatch] {
        &self.patches
    }

    pub fn checkpoints(&self) -> &[AdvancedEditingMutationCheckpoint] {
        &self.checkpoints
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn apply_text(
        &mut self,
        page: usize,
        old_text: &str,
        new_text: &str,
        mode: AdvancedTextMode,
        options: &AdvancedTextEditOptions,
        font_bytes: Option<&[u8]>,
    ) -> Result<&AdvancedEditingMutationPatch> {
        let (output, report) = edit_advanced_text_pdf(
            &self.current,
            page,
            old_text,
            new_text,
            mode,
            options,
            font_bytes,
        )?;
        self.commit(
            "text_edit",
            output,
            advanced_editing_report_json_value(report)?,
        )
    }

    pub fn apply_same_width_patch(
        &mut self,
        page: usize,
        old_text: &str,
        new_text: &str,
        options: &SameWidthPatchOptions,
    ) -> Result<&AdvancedEditingMutationPatch> {
        let (output, report) =
            apply_same_width_patch(&self.current, page, old_text, new_text, options)?;
        self.commit(
            "same_width_patch",
            output,
            advanced_editing_report_json_value(report)?,
        )
    }

    pub fn apply_multi_run_text_range(
        &mut self,
        request: &MultiRunTextRangeRequest,
        font_bytes: Option<&[u8]>,
    ) -> Result<&AdvancedEditingMutationPatch> {
        let (output, report) = edit_multi_run_text_range(&self.current, request, font_bytes)?;
        self.commit(
            "multi_run_text_range",
            output,
            advanced_editing_report_json_value(report)?,
        )
    }

    pub fn propose_generated_paint_partitions(
        &self,
        request: &MultiRunTextRangeRequest,
    ) -> Result<GeneratedPaintPartitionProposal> {
        propose_generated_paint_partitions(&self.current, request)
    }

    pub fn apply_generated_paint_partition_proposal(
        &mut self,
        request: &MultiRunTextRangeRequest,
        proposal: &GeneratedPaintPartitionProposal,
        approval: &GeneratedPaintPartitionApproval,
        font_bytes: Option<&[u8]>,
    ) -> Result<&AdvancedEditingMutationPatch> {
        let (output, report) = apply_generated_paint_partition_proposal(
            &self.current,
            request,
            proposal,
            approval,
            font_bytes,
        )?;
        self.commit(
            "generated_paint_partition_proposal",
            output,
            advanced_editing_report_json_value(report)?,
        )
    }

    pub fn apply_vector(
        &mut self,
        page: usize,
        stable_id: &str,
        operation: VectorEditOperation,
        options: &VectorEditOptions,
    ) -> Result<&AdvancedEditingMutationPatch> {
        let (output, report) =
            edit_vector_object(&self.current, page, stable_id, operation, options)?;
        self.commit(
            "vector_edit",
            output,
            advanced_editing_report_json_value(report)?,
        )
    }

    pub fn apply_annotation_ink(
        &mut self,
        page: usize,
        annotation_index: usize,
        options: &InkFitOptions,
        signature_policy_override: bool,
    ) -> Result<&AdvancedEditingMutationPatch> {
        let (output, report) = fit_annotation_ink_pdf(
            &self.current,
            page,
            annotation_index,
            options,
            signature_policy_override,
        )?;
        self.commit(
            "ink_fit",
            output,
            advanced_editing_report_json_value(report)?,
        )
    }

    pub fn undo(&mut self) -> Result<bool> {
        if self.cursor == 0 {
            return Ok(false);
        }
        let patch = &self.patches[self.cursor - 1];
        if format!("{:x}", Sha256::digest(&self.current)) != patch.after_sha256
            || patch.before_bytes > self.current.len()
        {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing undo fingerprint or patch boundary mismatch".to_string(),
            ));
        }
        self.current.truncate(patch.before_bytes);
        if format!("{:x}", Sha256::digest(&self.current)) != patch.before_sha256 {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing undo did not restore the recorded checkpoint digest".to_string(),
            ));
        }
        self.cursor -= 1;
        Ok(true)
    }

    pub fn redo(&mut self) -> Result<bool> {
        let Some(patch) = self.patches.get(self.cursor) else {
            return Ok(false);
        };
        if format!("{:x}", Sha256::digest(&self.current)) != patch.before_sha256 {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing redo fingerprint mismatch".to_string(),
            ));
        }
        self.current.extend_from_slice(&patch.appended_suffix);
        if format!("{:x}", Sha256::digest(&self.current)) != patch.after_sha256 {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing redo did not restore the recorded patch digest".to_string(),
            ));
        }
        self.cursor += 1;
        Ok(true)
    }

    fn commit(
        &mut self,
        operation: &str,
        output: Vec<u8>,
        report: serde_json::Value,
    ) -> Result<&AdvancedEditingMutationPatch> {
        if !output.starts_with(&self.current) {
            return Err(WellfriendError::MalformedPdf(
                "advanced_editing transaction output is not an incremental prefix-preserving patch"
                    .to_string(),
            ));
        }
        if self.cursor < self.patches.len() {
            self.patches.truncate(self.cursor);
            self.checkpoints
                .retain(|checkpoint| checkpoint.sequence <= self.cursor);
        }
        if self.patches.len() >= self.max_patches {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "advanced_editing transaction patch count exceeds limit {}",
                self.max_patches
            )));
        }
        let total_patch_bytes = self
            .patches
            .iter()
            .map(|patch| patch.appended_bytes)
            .sum::<usize>();
        let appended_suffix = output[self.current.len()..].to_vec();
        if total_patch_bytes.saturating_add(appended_suffix.len()) > self.max_total_patch_bytes {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "advanced_editing transaction suffix bytes exceed limit {}",
                self.max_total_patch_bytes
            )));
        }
        let sequence = self.cursor + 1;
        let before_sha256 = format!("{:x}", Sha256::digest(&self.current));
        let after_sha256 = format!("{:x}", Sha256::digest(&output));
        let transaction_id = {
            let mut hasher = Sha256::new();
            hasher.update(sequence.to_le_bytes());
            hasher.update(operation.as_bytes());
            hasher.update(before_sha256.as_bytes());
            hasher.update(after_sha256.as_bytes());
            let digest = format!("{:x}", hasher.finalize());
            format!("p20-tx-{}", &digest[..24])
        };
        let patch = AdvancedEditingMutationPatch {
            sequence,
            transaction_id,
            operation: operation.to_string(),
            before_bytes: self.current.len(),
            after_bytes: output.len(),
            appended_bytes: appended_suffix.len(),
            before_sha256,
            after_sha256: after_sha256.clone(),
            report,
            appended_suffix,
        };
        self.current = output;
        self.patches.push(patch);
        self.cursor = self.patches.len();
        self.checkpoints.push(AdvancedEditingMutationCheckpoint {
            sequence,
            document_bytes: self.current.len(),
            document_sha256: after_sha256,
            patch_count: self.cursor,
        });
        if self.checkpoints.len() > 128 {
            let remove = self.checkpoints.len() - 128;
            self.checkpoints.drain(0..remove);
        }
        Ok(self.patches.last().expect("just pushed transaction patch"))
    }
}

fn advanced_editing_report_json_value<T: Serialize>(report: T) -> Result<serde_json::Value> {
    serde_json::to_value(report).map_err(|error| {
        WellfriendError::ParseError(format!(
            "advanced_editing transaction report serialization failed: {error}"
        ))
    })
}

fn advanced_editing_cache_invalidation(
    input: &[u8],
    output: &[u8],
    text: bool,
    vector: bool,
    annotation: bool,
) -> CacheInvalidationReport {
    CacheInvalidationReport {
        text_layout: text,
        glyphs: text,
        render_tiles: true,
        vectors: vector || annotation,
        annotation_appearances: annotation,
        semantic: true,
        search_and_rag: text,
        optional_content: vector,
        writer: true,
        fingerprint_before: format!("{:x}", Sha256::digest(input)),
        fingerprint_after: format!("{:x}", Sha256::digest(output)),
        render_write_set_refs: Vec::new(),
        changed_object_refs: Vec::new(),
        created_object_refs: Vec::new(),
        removed_object_refs: Vec::new(),
        affected_pages: Vec::new(),
        dirty_regions: Vec::new(),
        structured_render_write_set: false,
    }
}

fn advanced_editing_object_ref(number: u32, generation: u16) -> String {
    format!("{number} {generation} R")
}

fn advanced_editing_dirty_region(
    page: usize,
    region: [f64; 4],
    reason: &str,
) -> CacheInvalidationDirtyRegion {
    CacheInvalidationDirtyRegion {
        page,
        region,
        reason: reason.to_string(),
    }
}

fn annotation_move_dirty_regions(
    page: usize,
    before_rect: [f64; 4],
    after_rect: [f64; 4],
) -> Vec<CacheInvalidationDirtyRegion> {
    if before_rect == after_rect {
        return vec![advanced_editing_dirty_region(
            page,
            before_rect,
            "annotation_rect_changed",
        )];
    }
    vec![
        advanced_editing_dirty_region(page, before_rect, "annotation_rect_before"),
        advanced_editing_dirty_region(page, after_rect, "annotation_rect_after"),
    ]
}

struct AdvancedEditingRenderInvalidation {
    changed_object_refs: Vec<String>,
    created_object_refs: Vec<String>,
    removed_object_refs: Vec<String>,
    affected_pages: Vec<usize>,
    dirty_regions: Vec<CacheInvalidationDirtyRegion>,
}

fn advanced_editing_cache_invalidation_with_render_write_set(
    input: &[u8],
    output: &[u8],
    text: bool,
    vector: bool,
    annotation: bool,
    details: AdvancedEditingRenderInvalidation,
) -> CacheInvalidationReport {
    let mut report = advanced_editing_cache_invalidation(input, output, text, vector, annotation);
    let mut write_set = Vec::new();
    for object_ref in details
        .changed_object_refs
        .iter()
        .chain(details.created_object_refs.iter())
        .chain(details.removed_object_refs.iter())
    {
        if !write_set.contains(object_ref) {
            write_set.push(object_ref.clone());
        }
    }
    report.render_write_set_refs = write_set;
    report.changed_object_refs = details.changed_object_refs;
    report.created_object_refs = details.created_object_refs;
    report.removed_object_refs = details.removed_object_refs;
    report.affected_pages = details.affected_pages;
    report.dirty_regions = details.dirty_regions;
    report.structured_render_write_set = true;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{OutputObject, PdfWriter};

    #[test]
    fn canonical_editing_tokens_keep_dictionary_literals_and_direct_ownership() {
        let bytes = b"BT /#46#31 12 Tf /Span << /ActualText (ABC) /Reviewed true /Other false /Nested << /ActualText (WRONG) >> >> BDC (ABC) Tj EMC ET";
        let mut state = ScannedTextTokenState::default();
        let tokens =
            scan_text_string_tokens_with_state_and_owner(bytes, &mut state, Some((4, 0))).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].font_name, "F1");
        assert_eq!(tokens[0].actual_text_sources.len(), 1);
        assert_eq!(
            tokens[0].actual_text_sources[0].logical_text.as_ref(),
            "ABC"
        );
        assert!(!tokens[0].unresolved_actual_text);
        assert!(tokens[0].flow_relocatable);
        let mut state = ScannedTextTokenState::default();
        let cleared = scan_text_string_tokens_with_state_and_owner(
            b"/Span << /ActualText null /Reviewed true >> BDC BT /F1 12 Tf (DEF) Tj ET EMC",
            &mut state,
            Some((4, 0)),
        )
        .unwrap();
        assert!(cleared[0].actual_text_sources.is_empty());
        assert!(!cleared[0].unresolved_actual_text);
        assert!(cleared[0].flow_relocatable);
        let tokens =
            lex_content(b"/Span << /Value /ActualText /Nested << /ActualText (NO) >> >>").unwrap();
        assert!(marked_property(&tokens, "ActualText").unwrap().is_none());
    }

    #[test]
    fn canonical_editing_tokens_never_expose_inline_image_payload_as_text() {
        let bytes = b"BI /W 24 /H 1 /BPC 8 /CS /G ID BT /F9 40 Tf (FAKE) Tj ET EI BT /F1 12 Tf (REAL) Tj ET";
        let tokens = scan_text_string_tokens(bytes).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].decoded, b"REAL");
        assert_eq!(tokens[0].font_name, "F1");
    }

    #[test]
    fn mixed_bidi_uses_level_at_each_visual_run_byte_start() {
        let text = "English שלום 123 العربية";
        let font = get_fallback_font("Symbol").unwrap();
        let report = analyze_advanced_text_reflow(
            text,
            AdvancedTextMode::ParagraphReflowHorizontal,
            Some(font),
            TextReflowLimits::default(),
        )
        .unwrap();
        let bidi = BidiInfo::new(text, Some(Level::ltr()));
        for p in &bidi.paragraphs {
            let (levels, ranges) = bidi.visual_runs(p, p.range.clone());
            for range in ranges {
                let run = report
                    .bidi_runs
                    .iter()
                    .find(|r| r.logical_byte_start == range.start)
                    .unwrap();
                assert_eq!(run.embedding_level, levels[range.start].number());
            }
        }
        let glyphs =
            generated_glyph_plan(text, AdvancedTextMode::ParagraphReflowHorizontal, font).unwrap();
        assert!(glyphs
            .iter()
            .all(|g| text.is_char_boundary(g.logical_byte_start)));
    }

    #[test]
    fn reflow_stays_before_later_artwork_and_preserves_following_source_advance() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"q 2 0 0 2 0 0 cm BT /F1 12 Tf 10 60 Td (ABC) Tj (DEF) Tj ET Q 0 g 0 0 200 200 re f",
        );
        let options = AdvancedTextEditOptions {
            region: [10.0, 20.0, 180.0, 150.0],
            ..Default::default()
        };
        let (output, _) = edit_advanced_text_pdf(
            &input,
            1,
            "ABC",
            "XYZ",
            AdvancedTextMode::ParagraphReflowHorizontal,
            &options,
            None,
        )
        .unwrap();
        let engine = ContentEngine::open_bytes(output).unwrap();
        let content = engine.document().get_page_content_bytes(1).unwrap();
        let decoded = String::from_utf8_lossy(&content);
        assert!(!decoded.contains("(ABC)"));
        assert!(decoded.contains("(DEF)"));
        let operations = lex_content(&content).unwrap();
        let first_show = operations
            .iter()
            .find(|t| matches!(&t.kind, LexicalKind::Word(op) if op == "TJ"))
            .unwrap();
        assert!(String::from_utf8_lossy(&content[..first_show.start]).contains('-'));
        let generated = decoded.find("FEFF00580059005A").unwrap();
        assert!(generated < decoded.rfind("re f").unwrap());
        assert!(decoded.contains("0.5 0 0 0.5 0 0 cm"));
    }

    #[test]
    fn multi_text_object_reflow_requires_an_explicit_paint_order_decision() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 12 Tf 10 150 Td (A) Tj ET 0 0 20 20 re f BT /F1 12 Tf 30 150 Td (B) Tj ET",
        );
        let request = |paint_order_policy| MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 2,
            replacement_text: "XYZ".into(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::ExplicitSupplied,
            options: AdvancedTextEditOptions {
                region: [10.0, 20.0, 180.0, 150.0],
                paint_order_policy,
                ..Default::default()
            },
            final_lines: None,
        };

        let error = edit_multi_run_text_range(
            &input,
            &request(GeneratedPaintOrderPolicy::RequireSingleSourceTextObject),
            None,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("ambiguous_generated_paint_order"));

        let mut proposal_request =
            request(GeneratedPaintOrderPolicy::RequireSingleSourceTextObject);
        proposal_request.replacement_text = "123".into();
        let proposal = propose_generated_paint_partitions(&input, &proposal_request).unwrap();
        assert_eq!(proposal.candidates.len(), 2);
        assert_eq!(
            proposal
                .candidates
                .iter()
                .map(|candidate| candidate.replacement_scalar_range)
                .collect::<Vec<_>>(),
            vec![[0, 2], [2, 3]]
        );
        assert!(proposal
            .candidates
            .iter()
            .all(|candidate| candidate.suggested_region.is_none()));
        let approval = GeneratedPaintPartitionApproval {
            proposal_id: proposal.proposal_id.clone(),
            font_sha256: None,
            partitions: vec![
                GeneratedPaintPartitionApprovalEntry {
                    source_text_object: 0,
                    region: [10.0, 100.0, 80.0, 140.0],
                    final_lines: None,
                },
                GeneratedPaintPartitionApprovalEntry {
                    source_text_object: 1,
                    region: [100.0, 100.0, 180.0, 140.0],
                    final_lines: None,
                },
            ],
        };
        let (_, proposed_report) = apply_generated_paint_partition_proposal(
            &input,
            &proposal_request,
            &proposal,
            &approval,
            None,
        )
        .unwrap();
        assert_eq!(proposed_report.generated_paint_partitions.len(), 2);
        let mut wrong_font_approval = approval.clone();
        wrong_font_approval.font_sha256 = Some("00".repeat(32));
        assert!(apply_generated_paint_partition_proposal(
            &input,
            &proposal_request,
            &proposal,
            &wrong_font_approval,
            None,
        )
        .unwrap_err()
        .to_string()
        .contains("font_sha256"));
        let mut stale = proposal.clone();
        stale.proposal_id.push('0');
        assert!(apply_generated_paint_partition_proposal(
            &input,
            &proposal_request,
            &stale,
            &approval,
            None,
        )
        .unwrap_err()
        .to_string()
        .contains("stale"));

        for (policy, generated_before_artwork) in [
            (
                GeneratedPaintOrderPolicy::AnchorAfterFirstSourceTextObject,
                true,
            ),
            (
                GeneratedPaintOrderPolicy::AnchorAfterLastSourceTextObject,
                false,
            ),
        ] {
            let (output, report) =
                edit_multi_run_text_range(&input, &request(policy), None).unwrap();
            assert_eq!(
                report
                    .selected_source_spans
                    .iter()
                    .map(|span| span.source_text_object)
                    .collect::<Vec<_>>(),
                vec![0, 1]
            );
            let decision = report.generated_paint_order.unwrap();
            assert_eq!(decision.policy, policy);
            assert_eq!(decision.source_text_objects, 2);
            let engine = ContentEngine::open_bytes(output).unwrap();
            let content = engine.document().get_page_content_bytes(1).unwrap();
            let decoded = String::from_utf8_lossy(&content);
            let generated = scan_text_string_tokens(&content)
                .unwrap()
                .into_iter()
                .find(|token| token.text_render_mode != 3 && token.font_name != "F1")
                .unwrap()
                .operation_start;
            let artwork = decoded.find("re f").unwrap();
            assert_eq!(generated < artwork, generated_before_artwork);
            assert!(!decoded.contains("(A)"));
            assert!(!decoded.contains("(B)"));
        }

        let partitioned = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 2,
            replacement_text: "X Y".into(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::ExplicitSupplied,
            options: AdvancedTextEditOptions {
                region: [10.0, 20.0, 180.0, 150.0],
                paint_partitions: vec![
                    GeneratedPaintPartition {
                        source_text_object: 0,
                        replacement_scalar_range: [0, 2],
                        region: [10.0, 100.0, 80.0, 140.0],
                        final_lines: None,
                    },
                    GeneratedPaintPartition {
                        source_text_object: 1,
                        replacement_scalar_range: [2, 3],
                        region: [100.0, 100.0, 180.0, 140.0],
                        final_lines: None,
                    },
                ],
                ..Default::default()
            },
            final_lines: None,
        };
        let (output, report) = edit_multi_run_text_range(&input, &partitioned, None).unwrap();
        assert_eq!(
            report.operation,
            "replace_partitioned_by_source_paint_order"
        );
        assert!(report.generated_paint_order.is_none());
        assert_eq!(report.generated_paint_partitions.len(), 2);
        assert!(report
            .generated_paint_partitions
            .iter()
            .all(|receipt| receipt.generated));
        let engine = ContentEngine::open_bytes(output).unwrap();
        assert_eq!(
            engine
                .get_page_text(1)
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            "X Y"
        );
        let content = engine.document().get_page_content_bytes(1).unwrap();
        let decoded = String::from_utf8_lossy(&content);
        let leading = decoded.find("FEFF00580020").unwrap();
        let artwork = decoded.find("re f").unwrap();
        let trailing = decoded.rfind("FEFF0059").unwrap();
        assert!(leading < artwork && artwork < trailing);
        assert!(!decoded.contains("(A)"));
        assert!(!decoded.contains("(B)"));

        let mut contextual_partition = partitioned.clone();
        contextual_partition.replacement_text = "123".into();
        contextual_partition.options.paint_partitions[0].replacement_scalar_range = [0, 1];
        contextual_partition.options.paint_partitions[1].replacement_scalar_range = [1, 3];
        let (_, report) = edit_multi_run_text_range(&input, &contextual_partition, None).unwrap();
        assert_eq!(report.generated_paint_partitions.len(), 2);

        let mut inherited_partition = partitioned.clone();
        inherited_partition.replacement_text = "12".into();
        inherited_partition.style_policy = MultiRunStylePolicy::PreservePerSegment;
        inherited_partition.options.paint_partitions[0].replacement_scalar_range = [0, 1];
        inherited_partition.options.paint_partitions[1].replacement_scalar_range = [1, 2];
        let (_, report) = edit_multi_run_text_range(&input, &inherited_partition, None).unwrap();
        assert_eq!(
            report.operation,
            "replace_partitioned_preserving_source_styles"
        );
        assert_eq!(report.generated_paint_partitions.len(), 2);

        // DejaVu Sans normally composes the `ffi` sequence. A split inside the
        // sequence must not silently emit the independently shaped halves.
        let mut unsafe_ligature = partitioned;
        unsafe_ligature.replacement_text = "office".into();
        unsafe_ligature.options.paint_partitions[0].replacement_scalar_range = [0, 2];
        unsafe_ligature.options.paint_partitions[1].replacement_scalar_range = [2, 6];
        let error = edit_multi_run_text_range(&input, &unsafe_ligature, None).unwrap_err();
        assert!(error.to_string().contains("OpenType ligature"));
    }

    #[test]
    fn leading_and_trailing_inheritance_choose_different_source_typography() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 10 Tf 0 g (ONE) Tj /F2 18 Tf 1 0 0 rg (TWO) Tj ET",
        );
        for (policy, font, size) in [
            (MultiRunStylePolicy::InheritLeading, "F1", 10.0),
            (MultiRunStylePolicy::InheritTrailing, "F2", 18.0),
        ] {
            let request = MultiRunTextRangeRequest {
                page: 1,
                logical_start: 0,
                logical_end: 6,
                replacement_text: "X".into(),
                mode: AdvancedTextMode::ParagraphReflowHorizontal,
                style_policy: policy,
                options: AdvancedTextEditOptions {
                    region: [0.0, 0.0, 200.0, 200.0],
                    ..Default::default()
                },
                final_lines: None,
            };
            let (output, _) = edit_multi_run_text_range(&input, &request, None).unwrap();
            let engine = ContentEngine::open_bytes(output).unwrap();
            let content = engine.document().get_page_content_bytes(1).unwrap();
            let tokens = scan_text_string_tokens(&content).unwrap();
            let replacement = tokens
                .iter()
                .find(|t| t.decoded == b"X" && t.text_render_mode != 3)
                .unwrap();
            assert_eq!(replacement.font_name, font);
            assert_eq!(replacement.font_size, size);
        }
    }

    #[test]
    fn zero_width_inheritance_uses_the_correct_adjacent_source_run() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 10 Tf 0 g (ONE) Tj /F2 18 Tf 1 0 0 rg (TWO) Tj ET",
        );
        for (policy, operation, size, colour) in [
            (
                MultiRunStylePolicy::InheritLeading,
                "insert_inline_inheriting_leading_source_style",
                18.0,
                "1 0 0 rg",
            ),
            (
                MultiRunStylePolicy::InheritTrailing,
                "insert_inline_inheriting_trailing_source_style",
                10.0,
                "0 g",
            ),
        ] {
            let request = MultiRunTextRangeRequest {
                page: 1,
                logical_start: 3,
                logical_end: 3,
                replacement_text: "X".into(),
                mode: AdvancedTextMode::ParagraphReflowHorizontal,
                style_policy: policy,
                options: AdvancedTextEditOptions {
                    region: [0.0, 0.0, 200.0, 200.0],
                    ..Default::default()
                },
                final_lines: None,
            };
            let (output, report) = edit_multi_run_text_range(&input, &request, None).unwrap();
            assert_eq!(report.operation, operation);
            assert_eq!(report.selected_source_spans.len(), 1);
            let engine = ContentEngine::open_bytes(output).unwrap();
            assert!(engine.get_page_text(1).unwrap().contains("ONEXTWO"));
            let content = engine.document().get_page_content_bytes(1).unwrap();
            let generated = scan_text_string_tokens(&content)
                .unwrap()
                .into_iter()
                .find(|token| token.font_name.starts_with("OxP20F"))
                .unwrap();
            assert_eq!(generated.font_size, size);
            assert_eq!(generated.fill_color_command, colour);
        }
    }

    #[test]
    fn zero_width_inheritance_keeps_clipping_union_inside_original_text_object() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 10 Tf 4 Tr 0 0 Td (ONE) Tj ET 0 0 100 100 re f",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 3,
            logical_end: 3,
            replacement_text: "X".into(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritTrailing,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (output, report) = edit_multi_run_text_range(&input, &request, None).unwrap();
        assert_eq!(
            report.operation,
            "insert_inline_inheriting_trailing_source_style"
        );
        let engine = ContentEngine::open_bytes(output).unwrap();
        assert!(engine.get_page_text(1).unwrap().contains("ONEX"));
        let content = engine.document().get_page_content_bytes(1).unwrap();
        let decoded = String::from_utf8_lossy(&content);
        let generated_font = decoded.find("/OxP20F").unwrap();
        let original_text_end = decoded.find("ET").unwrap();
        assert!(generated_font < original_text_end);
        let generated = scan_text_string_tokens(&content)
            .unwrap()
            .into_iter()
            .find(|token| token.font_name.starts_with("OxP20F"))
            .unwrap();
        assert_eq!(generated.text_render_mode, 4);
    }

    #[test]
    fn zero_width_insertion_can_target_a_grapheme_boundary_inside_one_operand() {
        let input =
            advanced_editing_fixture_with_content(false, b"BT /F1 10 Tf 0 g 0 0 Td (ONE) Tj ET");
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 1,
            logical_end: 1,
            replacement_text: "X".into(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritTrailing,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (output, report) = edit_multi_run_text_range(&input, &request, None).unwrap();
        assert_eq!(
            report.operation,
            "insert_inline_inheriting_trailing_source_style"
        );
        assert_eq!(report.selected_source_spans[0].text, "ONE");
        assert!(ContentEngine::open_bytes(output)
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .contains("OXNE"));
    }

    #[test]
    fn zero_width_inheritance_preserves_a_transformed_source_basis() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 10 Tf 0 g 0 1 -1 0 100 100 Tm (ONE) Tj ET",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 1,
            logical_end: 1,
            replacement_text: "X".into(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritTrailing,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (output, report) = edit_multi_run_text_range(&input, &request, None).unwrap();
        assert_eq!(
            report.operation,
            "insert_inline_inheriting_trailing_source_style"
        );
        let model = analyze_multi_run_text_range(&output, 1).unwrap();
        let engine = ContentEngine::open_bytes(output).unwrap();
        let extracted = engine.get_page_text(1).unwrap();
        assert!(
            ['O', 'X', 'N', 'E']
                .iter()
                .all(|character| extracted.contains(*character)),
            "extracted={extracted:?}"
        );
        assert_eq!(model.logical_text, "OXNE");
        let content = engine.document().get_page_content_bytes(1).unwrap();
        let decoded = String::from_utf8_lossy(&content);
        assert!(decoded.contains("/WFTextBasisV1"));
        assert!(decoded.find("/OxP20F").unwrap() < decoded.find("ET").unwrap());
    }

    #[test]
    fn zero_width_insertion_splices_an_existing_isomorphic_actual_text_owner() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"/Span << /ActualText (ONE) >> BDC BT /F1 10 Tf 0 g (ONE) Tj ET EMC",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 1,
            logical_end: 1,
            replacement_text: "X".into(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritTrailing,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (output, _) = edit_multi_run_text_range(&input, &request, None).unwrap();
        let engine = ContentEngine::open_bytes(output).unwrap();
        assert!(engine.get_page_text(1).unwrap().contains("OXNE"));
        let content = engine.document().get_page_content_bytes(1).unwrap();
        let decoded = String::from_utf8_lossy(&content);
        assert!(decoded.contains(&format!("/ActualText <{}>", utf16be_hex_with_bom("OXNE"))));
    }

    #[test]
    fn generated_font_retirement_keeps_only_reachable_editor_resources() {
        let input =
            advanced_editing_fixture_with_content(false, b"BT /OxP20FUsed 10 Tf 0 g (ONE) Tj ET");
        let engine = ContentEngine::open_bytes(input).unwrap();
        let page = engine.document().get_page(1).unwrap();
        let reader = engine.document().reader();
        let mut resources = page.resources.clone();
        let mut fonts =
            resolve_advanced_editing_dict(resources.get("Font"), reader).unwrap_or_default();
        for name in ["OxP20FUsed", "OxP20FStale"] {
            let mut font = crate::PdfDictionary::empty();
            font.insert("Type", PdfObject::Name("Font".into()));
            font.insert("Subtype", PdfObject::Name("Type0".into()));
            font.insert(
                "WFAdvancedEditingGeneratedFont",
                PdfObject::Name("V1".into()),
            );
            fonts.insert(name, PdfObject::Dictionary(font));
        }
        resources.insert("Font", PdfObject::Dictionary(fonts));
        let roots = page
            .contents
            .iter()
            .map(|&(number, generation)| PdfObject::Reference { number, generation })
            .collect::<Vec<_>>();
        assert_eq!(
            retire_unreferenced_generated_fonts(
                reader,
                &roots,
                &mut resources,
                None,
                &[],
                &BTreeSet::new(),
            )
            .unwrap(),
            1
        );
        let fonts = resolve_advanced_editing_dict(resources.get("Font"), reader).unwrap();
        assert!(fonts.contains_key("OxP20FUsed"));
        assert!(!fonts.contains_key("OxP20FStale"));
    }

    #[test]
    fn scanner_carries_text_and_marked_content_state_across_contents_members() {
        let mut state = ScannedTextTokenState::default();
        let first = scan_text_string_tokens_with_state(
            b"q /F9 13 Tf 0.25 Tc /Span << /MCID 7 >> BDC",
            &mut state,
        )
        .unwrap();
        assert!(first.is_empty());

        let second =
            scan_text_string_tokens_with_state(b"(cross stream) Tj EMC Q", &mut state).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].font_name, "F9");
        assert_eq!(second[0].font_size, 13.0);
        assert_eq!(second[0].character_spacing, 0.25);
        assert_eq!(second[0].marked_depth, 1);
        assert_eq!(state.marked_depth, 0);
        assert!(state.graphics_stack.is_empty());
    }

    fn advanced_editing_fixture(include_ink: bool) -> Vec<u8> {
        advanced_editing_fixture_with_content(
            include_ink,
            b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n2 w 1 0 0 RG 20 20 40 30 re S\n",
        )
    }

    pub(super) fn advanced_editing_fixture_with_content(
        include_ink: bool,
        source_content: &[u8],
    ) -> Vec<u8> {
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".to_string()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut font = crate::PdfDictionary::empty();
        font.insert("Type", PdfObject::Name("Font".to_string()));
        font.insert("Subtype", PdfObject::Name("Type1".to_string()));
        font.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
        font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));
        let mut font2 = crate::PdfDictionary::empty();
        font2.insert("Type", PdfObject::Name("Font".to_string()));
        font2.insert("Subtype", PdfObject::Name("Type1".to_string()));
        font2.insert("BaseFont", PdfObject::Name("Times-Roman".to_string()));
        font2.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));
        let mut fonts = crate::PdfDictionary::empty();
        fonts.insert(
            "F1",
            PdfObject::Reference {
                number: 5,
                generation: 0,
            },
        );
        fonts.insert(
            "F2",
            PdfObject::Reference {
                number: 10,
                generation: 0,
            },
        );
        let mut resources = crate::PdfDictionary::empty();
        resources.insert("Font", PdfObject::Dictionary(fonts));
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
            ]),
        );
        page.insert("Resources", PdfObject::Dictionary(resources));
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        if include_ink {
            page.insert(
                "Annots",
                PdfObject::Array(vec![PdfObject::Reference {
                    number: 6,
                    generation: 0,
                }]),
            );
        }
        let content = source_content.to_vec();
        let mut content_dict = crate::PdfDictionary::empty();
        content_dict.insert("Length", PdfObject::Integer(content.len() as i64));
        let mut objects = vec![
            OutputObject {
                number: 1,
                object: PdfObject::Dictionary(catalog),
            },
            OutputObject {
                number: 2,
                object: PdfObject::Dictionary(pages),
            },
            OutputObject {
                number: 3,
                object: PdfObject::Dictionary(page),
            },
            OutputObject {
                number: 4,
                object: PdfObject::Stream {
                    dict: content_dict,
                    raw: content,
                },
            },
            OutputObject {
                number: 5,
                object: PdfObject::Dictionary(font),
            },
            OutputObject {
                number: 10,
                object: PdfObject::Dictionary(font2),
            },
        ];
        if include_ink {
            let mut annotation = crate::PdfDictionary::empty();
            annotation.insert("Type", PdfObject::Name("Annot".to_string()));
            annotation.insert("Subtype", PdfObject::Name("Ink".to_string()));
            annotation.insert(
                "Rect",
                PdfObject::Array(vec![
                    PdfObject::Integer(10),
                    PdfObject::Integer(70),
                    PdfObject::Integer(120),
                    PdfObject::Integer(130),
                ]),
            );
            annotation.insert(
                "InkList",
                PdfObject::Array(vec![PdfObject::Array(vec![
                    PdfObject::Integer(10),
                    PdfObject::Integer(80),
                    PdfObject::Integer(30),
                    PdfObject::Integer(100),
                    PdfObject::Integer(60),
                    PdfObject::Integer(90),
                    PdfObject::Integer(100),
                    PdfObject::Integer(120),
                ])]),
            );
            annotation.insert(
                "C",
                PdfObject::Array(vec![
                    PdfObject::Real(0.1),
                    PdfObject::Real(0.2),
                    PdfObject::Real(0.8),
                ]),
            );
            objects.push(OutputObject {
                number: 6,
                object: PdfObject::Dictionary(annotation),
            });
        }
        PdfWriter::new(objects, 1).write().expect("fixture PDF")
    }

    fn advanced_editing_link_fixture() -> Vec<u8> {
        let input = advanced_editing_fixture(false);
        let engine = ContentEngine::open_bytes(input.clone()).expect("fixture open");
        let reader = engine.document().reader();
        let page = engine.document().get_page(1).expect("fixture page");
        let mut page_dict = reader
            .get_object(page.object_number, page.generation_number)
            .expect("fixture page object")
            .as_dict()
            .cloned()
            .expect("fixture page dictionary");
        page_dict.insert(
            "Annots",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 11,
                generation: 0,
            }]),
        );
        let mut action = crate::PdfDictionary::empty();
        action.insert("S", PdfObject::Name("URI".to_string()));
        action.insert(
            "URI",
            PdfObject::String(b"https://example.invalid/source-link".to_vec()),
        );
        let mut link = crate::PdfDictionary::empty();
        link.insert("Type", PdfObject::Name("Annot".to_string()));
        link.insert("Subtype", PdfObject::Name("Link".to_string()));
        link.insert(
            "Rect",
            PdfObject::Array(vec![
                PdfObject::Integer(10),
                PdfObject::Integer(140),
                PdfObject::Integer(70),
                PdfObject::Integer(160),
            ]),
        );
        link.insert(
            "QuadPoints",
            PdfObject::Array(vec![
                PdfObject::Integer(10),
                PdfObject::Integer(160),
                PdfObject::Integer(70),
                PdfObject::Integer(160),
                PdfObject::Integer(10),
                PdfObject::Integer(140),
                PdfObject::Integer(70),
                PdfObject::Integer(140),
            ]),
        );
        link.insert("A", PdfObject::Dictionary(action));
        write_incremental_update(
            reader,
            vec![
                IncrementalObject {
                    number: page.object_number,
                    generation: page.generation_number,
                    object: PdfObject::Dictionary(page_dict),
                },
                IncrementalObject {
                    number: 11,
                    generation: 0,
                    object: PdfObject::Dictionary(link),
                },
            ],
        )
        .expect("link fixture incremental update")
    }

    pub(super) fn shared_form_fixture() -> Vec<u8> {
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".to_string()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut xobjects = crate::PdfDictionary::empty();
        xobjects.insert(
            "Fm",
            PdfObject::Reference {
                number: 5,
                generation: 0,
            },
        );
        let mut resources = crate::PdfDictionary::empty();
        resources.insert("XObject", PdfObject::Dictionary(xobjects));
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
            ]),
        );
        page.insert("Resources", PdfObject::Dictionary(resources));
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        let content = b"q 1 0 0 1 10 10 cm /Fm Do Q\nq 1 0 0 1 80 80 cm /Fm Do Q\n".to_vec();
        let mut content_dict = crate::PdfDictionary::empty();
        content_dict.insert("Length", PdfObject::Integer(content.len() as i64));
        let form_data = b"2 w 0 0 20 10 re S\n".to_vec();
        let mut form_dict = crate::PdfDictionary::empty();
        form_dict.insert("Type", PdfObject::Name("XObject".to_string()));
        form_dict.insert("Subtype", PdfObject::Name("Form".to_string()));
        form_dict.insert(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(20),
                PdfObject::Integer(10),
            ]),
        );
        form_dict.insert(
            "Resources",
            PdfObject::Dictionary(crate::PdfDictionary::empty()),
        );
        form_dict.insert("Length", PdfObject::Integer(form_data.len() as i64));
        PdfWriter::new(
            vec![
                OutputObject {
                    number: 1,
                    object: PdfObject::Dictionary(catalog),
                },
                OutputObject {
                    number: 2,
                    object: PdfObject::Dictionary(pages),
                },
                OutputObject {
                    number: 3,
                    object: PdfObject::Dictionary(page),
                },
                OutputObject {
                    number: 4,
                    object: PdfObject::Stream {
                        dict: content_dict,
                        raw: content,
                    },
                },
                OutputObject {
                    number: 5,
                    object: PdfObject::Stream {
                        dict: form_dict,
                        raw: form_data,
                    },
                },
            ],
            1,
        )
        .write()
        .expect("shared Form fixture PDF")
    }

    pub(super) fn nested_shared_form_fixture() -> Vec<u8> {
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".to_string()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut page_xobjects = crate::PdfDictionary::empty();
        page_xobjects.insert(
            "Parent",
            PdfObject::Reference {
                number: 5,
                generation: 0,
            },
        );
        let mut page_resources = crate::PdfDictionary::empty();
        page_resources.insert("XObject", PdfObject::Dictionary(page_xobjects));
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
            ]),
        );
        page.insert("Resources", PdfObject::Dictionary(page_resources));
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        let page_data =
            b"q 1 0 0 1 10 10 cm /Parent Do Q\nq 1 0 0 1 80 80 cm /Parent Do Q\n".to_vec();
        let mut page_stream = crate::PdfDictionary::empty();
        page_stream.insert("Length", PdfObject::Integer(page_data.len() as i64));
        let mut child_xobjects = crate::PdfDictionary::empty();
        child_xobjects.insert(
            "Leaf",
            PdfObject::Reference {
                number: 6,
                generation: 0,
            },
        );
        let mut parent_resources = crate::PdfDictionary::empty();
        parent_resources.insert("XObject", PdfObject::Dictionary(child_xobjects));
        let parent_data = b"q /Leaf Do Q\n".to_vec();
        let mut parent_dict = crate::PdfDictionary::empty();
        parent_dict.insert("Type", PdfObject::Name("XObject".to_string()));
        parent_dict.insert("Subtype", PdfObject::Name("Form".to_string()));
        parent_dict.insert(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(20),
                PdfObject::Integer(10),
            ]),
        );
        parent_dict.insert("Resources", PdfObject::Dictionary(parent_resources));
        parent_dict.insert("Length", PdfObject::Integer(parent_data.len() as i64));
        let leaf_data = b"2 w 0 0 20 10 re S\n".to_vec();
        let mut leaf_dict = crate::PdfDictionary::empty();
        leaf_dict.insert("Type", PdfObject::Name("XObject".to_string()));
        leaf_dict.insert("Subtype", PdfObject::Name("Form".to_string()));
        leaf_dict.insert(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(20),
                PdfObject::Integer(10),
            ]),
        );
        leaf_dict.insert(
            "Resources",
            PdfObject::Dictionary(crate::PdfDictionary::empty()),
        );
        leaf_dict.insert("Length", PdfObject::Integer(leaf_data.len() as i64));
        PdfWriter::new(
            vec![
                OutputObject {
                    number: 1,
                    object: PdfObject::Dictionary(catalog),
                },
                OutputObject {
                    number: 2,
                    object: PdfObject::Dictionary(pages),
                },
                OutputObject {
                    number: 3,
                    object: PdfObject::Dictionary(page),
                },
                OutputObject {
                    number: 4,
                    object: PdfObject::Stream {
                        dict: page_stream,
                        raw: page_data,
                    },
                },
                OutputObject {
                    number: 5,
                    object: PdfObject::Stream {
                        dict: parent_dict,
                        raw: parent_data,
                    },
                },
                OutputObject {
                    number: 6,
                    object: PdfObject::Stream {
                        dict: leaf_dict,
                        raw: leaf_data,
                    },
                },
            ],
            1,
        )
        .write()
        .expect("nested Form fixture")
    }

    fn nested_shared_form_depth_fixture(depth: usize) -> Vec<u8> {
        assert!((2..=4).contains(&depth));
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".to_string()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut page_xobjects = crate::PdfDictionary::empty();
        page_xobjects.insert(
            "F0",
            PdfObject::Reference {
                number: 5,
                generation: 0,
            },
        );
        let mut page_resources = crate::PdfDictionary::empty();
        page_resources.insert("XObject", PdfObject::Dictionary(page_xobjects));
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
            ]),
        );
        page.insert("Resources", PdfObject::Dictionary(page_resources));
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        let page_data = b"q 1 0 0 1 10 10 cm /F0 Do Q\nq 1 0 0 1 80 80 cm /F0 Do Q\n".to_vec();
        let mut page_stream = crate::PdfDictionary::empty();
        page_stream.insert("Length", PdfObject::Integer(page_data.len() as i64));
        let mut objects = vec![
            OutputObject {
                number: 1,
                object: PdfObject::Dictionary(catalog),
            },
            OutputObject {
                number: 2,
                object: PdfObject::Dictionary(pages),
            },
            OutputObject {
                number: 3,
                object: PdfObject::Dictionary(page),
            },
            OutputObject {
                number: 4,
                object: PdfObject::Stream {
                    dict: page_stream,
                    raw: page_data,
                },
            },
        ];
        for level in 0..depth {
            let number = 5 + level as u32;
            if level + 1 == depth {
                objects.push(form_xobject_stream(
                    number,
                    b"2 w 0 0 20 10 re S\n",
                    crate::PdfDictionary::empty(),
                ));
            } else {
                let child_name = format!("F{}", level + 1);
                let mut xobjects = crate::PdfDictionary::empty();
                xobjects.insert(
                    child_name.clone(),
                    PdfObject::Reference {
                        number: number + 1,
                        generation: 0,
                    },
                );
                let mut resources = crate::PdfDictionary::empty();
                resources.insert("XObject", PdfObject::Dictionary(xobjects));
                let data = format!("q /{child_name} Do Q\n").into_bytes();
                objects.push(form_xobject_stream(number, &data, resources));
            }
        }
        PdfWriter::new(objects, 1)
            .write()
            .expect("nested depth Form fixture")
    }

    fn shared_annotation_appearance_fixture() -> Vec<u8> {
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".to_string()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
            ]),
        );
        page.insert(
            "Resources",
            PdfObject::Dictionary(crate::PdfDictionary::empty()),
        );
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        page.insert(
            "Annots",
            PdfObject::Array(vec![
                PdfObject::Reference {
                    number: 6,
                    generation: 0,
                },
                PdfObject::Reference {
                    number: 7,
                    generation: 0,
                },
            ]),
        );
        let mut content = crate::PdfDictionary::empty();
        content.insert("Length", PdfObject::Integer(0));
        let appearance_data = b"2 w 0 0 20 10 re S\n".to_vec();
        let mut appearance = crate::PdfDictionary::empty();
        appearance.insert("Type", PdfObject::Name("XObject".to_string()));
        appearance.insert("Subtype", PdfObject::Name("Form".to_string()));
        appearance.insert(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(20),
                PdfObject::Integer(10),
            ]),
        );
        appearance.insert(
            "Resources",
            PdfObject::Dictionary(crate::PdfDictionary::empty()),
        );
        appearance.insert("Length", PdfObject::Integer(appearance_data.len() as i64));
        let annotation = |x: i64| {
            let mut a = crate::PdfDictionary::empty();
            a.insert("Type", PdfObject::Name("Annot".to_string()));
            a.insert("Subtype", PdfObject::Name("Stamp".to_string()));
            a.insert(
                "Rect",
                PdfObject::Array(vec![
                    PdfObject::Integer(x),
                    PdfObject::Integer(10),
                    PdfObject::Integer(x + 20),
                    PdfObject::Integer(20),
                ]),
            );
            let mut ap = crate::PdfDictionary::empty();
            ap.insert(
                "N",
                PdfObject::Reference {
                    number: 8,
                    generation: 0,
                },
            );
            a.insert("AP", PdfObject::Dictionary(ap));
            a.insert("AS", PdfObject::Name("On".to_string()));
            a
        };
        PdfWriter::new(
            vec![
                OutputObject {
                    number: 1,
                    object: PdfObject::Dictionary(catalog),
                },
                OutputObject {
                    number: 2,
                    object: PdfObject::Dictionary(pages),
                },
                OutputObject {
                    number: 3,
                    object: PdfObject::Dictionary(page),
                },
                OutputObject {
                    number: 4,
                    object: PdfObject::Stream {
                        dict: content,
                        raw: Vec::new(),
                    },
                },
                OutputObject {
                    number: 6,
                    object: PdfObject::Dictionary(annotation(10)),
                },
                OutputObject {
                    number: 7,
                    object: PdfObject::Dictionary(annotation(80)),
                },
                OutputObject {
                    number: 8,
                    object: PdfObject::Stream {
                        dict: appearance,
                        raw: appearance_data,
                    },
                },
            ],
            1,
        )
        .write()
        .expect("shared AP fixture")
    }

    fn form_xobject_stream(
        number: u32,
        data: &[u8],
        resources: crate::PdfDictionary,
    ) -> OutputObject {
        let mut dict = crate::PdfDictionary::empty();
        dict.insert("Type", PdfObject::Name("XObject".to_string()));
        dict.insert("Subtype", PdfObject::Name("Form".to_string()));
        dict.insert(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(20),
                PdfObject::Integer(10),
            ]),
        );
        dict.insert("Resources", PdfObject::Dictionary(resources));
        dict.insert("Length", PdfObject::Integer(data.len() as i64));
        OutputObject {
            number,
            object: PdfObject::Stream {
                dict,
                raw: data.to_vec(),
            },
        }
    }

    fn shared_annotation_categories_fixture(categories: &[&str]) -> Vec<u8> {
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".to_string()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
            ]),
        );
        page.insert(
            "Resources",
            PdfObject::Dictionary(crate::PdfDictionary::empty()),
        );
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        page.insert(
            "Annots",
            PdfObject::Array(vec![
                PdfObject::Reference {
                    number: 6,
                    generation: 0,
                },
                PdfObject::Reference {
                    number: 7,
                    generation: 0,
                },
                PdfObject::Reference {
                    number: 11,
                    generation: 0,
                },
            ]),
        );
        let mut content = crate::PdfDictionary::empty();
        content.insert("Length", PdfObject::Integer(0));
        let annotation = |x: i64| {
            let mut a = crate::PdfDictionary::empty();
            a.insert("Type", PdfObject::Name("Annot".to_string()));
            a.insert("Subtype", PdfObject::Name("Stamp".to_string()));
            a.insert(
                "Rect",
                PdfObject::Array(vec![
                    PdfObject::Integer(x),
                    PdfObject::Integer(10),
                    PdfObject::Integer(x + 20),
                    PdfObject::Integer(20),
                ]),
            );
            let mut ap = crate::PdfDictionary::empty();
            for (index, category) in categories.iter().enumerate() {
                ap.insert(
                    *category,
                    PdfObject::Reference {
                        number: 8 + index as u32,
                        generation: 0,
                    },
                );
            }
            a.insert("AP", PdfObject::Dictionary(ap));
            a.insert("AS", PdfObject::Name("On".to_string()));
            a
        };
        let mut objects = vec![
            OutputObject {
                number: 1,
                object: PdfObject::Dictionary(catalog),
            },
            OutputObject {
                number: 2,
                object: PdfObject::Dictionary(pages),
            },
            OutputObject {
                number: 3,
                object: PdfObject::Dictionary(page),
            },
            OutputObject {
                number: 4,
                object: PdfObject::Stream {
                    dict: content,
                    raw: Vec::new(),
                },
            },
            OutputObject {
                number: 6,
                object: PdfObject::Dictionary(annotation(10)),
            },
            OutputObject {
                number: 7,
                object: PdfObject::Dictionary(annotation(80)),
            },
            OutputObject {
                number: 11,
                object: PdfObject::Dictionary(annotation(140)),
            },
        ];
        for (index, category) in categories.iter().enumerate() {
            let number = 8 + index as u32;
            let data = format!("{} w 0 0 20 10 re S\n", index + 2).into_bytes();
            let mut resources = crate::PdfDictionary::empty();
            resources.insert("Category", PdfObject::Name((*category).to_string()));
            objects.push(form_xobject_stream(number, &data, resources));
        }
        PdfWriter::new(objects, 1)
            .write()
            .expect("shared category AP fixture")
    }

    fn shared_annotation_state_fixture(widget: Option<&str>) -> Vec<u8> {
        let selected_state = if widget.is_some() { "Yes" } else { "On" };
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".to_string()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
            ]),
        );
        page.insert(
            "Resources",
            PdfObject::Dictionary(crate::PdfDictionary::empty()),
        );
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        page.insert(
            "Annots",
            PdfObject::Array(vec![
                PdfObject::Reference {
                    number: 6,
                    generation: 0,
                },
                PdfObject::Reference {
                    number: 7,
                    generation: 0,
                },
            ]),
        );
        let mut content = crate::PdfDictionary::empty();
        content.insert("Length", PdfObject::Integer(0));
        let annotation = |x: i64| {
            let mut a = crate::PdfDictionary::empty();
            a.insert("Type", PdfObject::Name("Annot".to_string()));
            a.insert(
                "Subtype",
                PdfObject::Name(if widget.is_some() { "Widget" } else { "Stamp" }.to_string()),
            );
            a.insert(
                "Rect",
                PdfObject::Array(vec![
                    PdfObject::Integer(x),
                    PdfObject::Integer(10),
                    PdfObject::Integer(x + 20),
                    PdfObject::Integer(20),
                ]),
            );
            if let Some(kind) = widget {
                a.insert("FT", PdfObject::Name("Btn".to_string()));
                a.insert("T", PdfObject::String(format!("p20b-{kind}").into_bytes()));
                a.insert(
                    "Ff",
                    PdfObject::Integer(if kind == "radio" { 32768 } else { 0 }),
                );
                a.insert("V", PdfObject::Name(selected_state.to_string()));
            }
            let mut states = crate::PdfDictionary::empty();
            states.insert(
                selected_state,
                PdfObject::Reference {
                    number: 8,
                    generation: 0,
                },
            );
            states.insert(
                "Off",
                PdfObject::Reference {
                    number: 9,
                    generation: 0,
                },
            );
            let mut ap = crate::PdfDictionary::empty();
            ap.insert("N", PdfObject::Dictionary(states));
            a.insert("AP", PdfObject::Dictionary(ap));
            a.insert("AS", PdfObject::Name(selected_state.to_string()));
            a
        };
        PdfWriter::new(
            vec![
                OutputObject {
                    number: 1,
                    object: PdfObject::Dictionary(catalog),
                },
                OutputObject {
                    number: 2,
                    object: PdfObject::Dictionary(pages),
                },
                OutputObject {
                    number: 3,
                    object: PdfObject::Dictionary(page),
                },
                OutputObject {
                    number: 4,
                    object: PdfObject::Stream {
                        dict: content,
                        raw: Vec::new(),
                    },
                },
                OutputObject {
                    number: 6,
                    object: PdfObject::Dictionary(annotation(10)),
                },
                OutputObject {
                    number: 7,
                    object: PdfObject::Dictionary(annotation(80)),
                },
                form_xobject_stream(8, b"2 w 0 0 20 10 re S\n", crate::PdfDictionary::empty()),
                form_xobject_stream(9, b"1 w 0 0 20 10 re S\n", crate::PdfDictionary::empty()),
            ],
            1,
        )
        .write()
        .expect("shared AP state fixture")
    }

    fn nested_annotation_appearance_fixture() -> Vec<u8> {
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".to_string()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
            ]),
        );
        page.insert(
            "Resources",
            PdfObject::Dictionary(crate::PdfDictionary::empty()),
        );
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        page.insert(
            "Annots",
            PdfObject::Array(vec![
                PdfObject::Reference {
                    number: 6,
                    generation: 0,
                },
                PdfObject::Reference {
                    number: 7,
                    generation: 0,
                },
            ]),
        );
        let mut content = crate::PdfDictionary::empty();
        content.insert("Length", PdfObject::Integer(0));
        let annotation = |x: i64| {
            let mut a = crate::PdfDictionary::empty();
            a.insert("Type", PdfObject::Name("Annot".to_string()));
            a.insert("Subtype", PdfObject::Name("Stamp".to_string()));
            a.insert(
                "Rect",
                PdfObject::Array(vec![
                    PdfObject::Integer(x),
                    PdfObject::Integer(10),
                    PdfObject::Integer(x + 20),
                    PdfObject::Integer(20),
                ]),
            );
            let mut ap = crate::PdfDictionary::empty();
            ap.insert(
                "N",
                PdfObject::Reference {
                    number: 8,
                    generation: 0,
                },
            );
            a.insert("AP", PdfObject::Dictionary(ap));
            a.insert("AS", PdfObject::Name("On".to_string()));
            a
        };
        let mut ap_xobjects = crate::PdfDictionary::empty();
        ap_xobjects.insert(
            "Nested",
            PdfObject::Reference {
                number: 9,
                generation: 0,
            },
        );
        let mut ap_resources = crate::PdfDictionary::empty();
        ap_resources.insert("XObject", PdfObject::Dictionary(ap_xobjects));
        PdfWriter::new(
            vec![
                OutputObject {
                    number: 1,
                    object: PdfObject::Dictionary(catalog),
                },
                OutputObject {
                    number: 2,
                    object: PdfObject::Dictionary(pages),
                },
                OutputObject {
                    number: 3,
                    object: PdfObject::Dictionary(page),
                },
                OutputObject {
                    number: 4,
                    object: PdfObject::Stream {
                        dict: content,
                        raw: Vec::new(),
                    },
                },
                OutputObject {
                    number: 6,
                    object: PdfObject::Dictionary(annotation(10)),
                },
                OutputObject {
                    number: 7,
                    object: PdfObject::Dictionary(annotation(80)),
                },
                form_xobject_stream(8, b"q /Nested Do Q\n", ap_resources),
                form_xobject_stream(9, b"2 w 0 0 20 10 re S\n", crate::PdfDictionary::empty()),
            ],
            1,
        )
        .write()
        .expect("nested shared AP fixture")
    }

    fn annotation_ap_ref(input: &[u8], annotation_index: usize, appearance: &str) -> (u32, u16) {
        let engine = ContentEngine::open_bytes(input.to_vec()).expect("open AP fixture");
        let page = engine.document().get_page(1).expect("page");
        let reader = engine.document().reader();
        let page_object = reader
            .get_object(page.object_number, page.generation_number)
            .expect("page object");
        let page_dict = page_object.as_dict().expect("page dict");
        let annots = reader
            .resolve(page_dict.get("Annots").expect("annots").clone())
            .expect("annots object");
        let annotation_ref = annots
            .as_array()
            .and_then(|items| items.get(annotation_index))
            .and_then(PdfObject::as_reference)
            .expect("annotation ref");
        let annotation = reader
            .get_object(annotation_ref.0, annotation_ref.1)
            .expect("annotation object");
        let annotation_dict = annotation.as_dict().expect("annotation dict");
        let ap = resolve_advanced_editing_dict(annotation_dict.get("AP"), reader).expect("AP dict");
        let parts = appearance.split('/').collect::<Vec<_>>();
        if parts.len() == 1 {
            return ap
                .get(parts[0])
                .and_then(PdfObject::as_reference)
                .expect("AP ref");
        }
        let states = resolve_advanced_editing_dict(ap.get(parts[0]), reader).expect("state dict");
        states
            .get(parts[1])
            .and_then(PdfObject::as_reference)
            .expect("state AP ref")
    }

    fn annotation_as_name(input: &[u8], annotation_index: usize) -> String {
        let engine = ContentEngine::open_bytes(input.to_vec()).expect("open AP fixture");
        let page = engine.document().get_page(1).expect("page");
        let reader = engine.document().reader();
        let page_object = reader
            .get_object(page.object_number, page.generation_number)
            .expect("page object");
        let page_dict = page_object.as_dict().expect("page dict");
        let annots = reader
            .resolve(page_dict.get("Annots").expect("annots").clone())
            .expect("annots object");
        let annotation_ref = annots
            .as_array()
            .and_then(|items| items.get(annotation_index))
            .and_then(PdfObject::as_reference)
            .expect("annotation ref");
        let annotation = reader
            .get_object(annotation_ref.0, annotation_ref.1)
            .expect("annotation object");
        annotation
            .as_dict()
            .and_then(|dict| dict.get_name("AS"))
            .expect("AS")
            .to_string()
    }

    pub(super) fn bare_vector_fixture(content: &[u8]) -> Vec<u8> {
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
        catalog.insert(
            "Pages",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".to_string()));
        pages.insert("Count", PdfObject::Integer(1));
        pages.insert(
            "Kids",
            PdfObject::Array(vec![PdfObject::Reference {
                number: 3,
                generation: 0,
            }]),
        );
        let mut page = crate::PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        page.insert(
            "Parent",
            PdfObject::Reference {
                number: 2,
                generation: 0,
            },
        );
        page.insert(
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(200),
                PdfObject::Integer(200),
            ]),
        );
        page.insert(
            "Resources",
            PdfObject::Dictionary(crate::PdfDictionary::empty()),
        );
        page.insert(
            "Contents",
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        );
        let mut content_dict = crate::PdfDictionary::empty();
        content_dict.insert("Length", PdfObject::Integer(content.len() as i64));
        PdfWriter::new(
            vec![
                OutputObject {
                    number: 1,
                    object: PdfObject::Dictionary(catalog),
                },
                OutputObject {
                    number: 2,
                    object: PdfObject::Dictionary(pages),
                },
                OutputObject {
                    number: 3,
                    object: PdfObject::Dictionary(page),
                },
                OutputObject {
                    number: 4,
                    object: PdfObject::Stream {
                        dict: content_dict,
                        raw: content.to_vec(),
                    },
                },
            ],
            1,
        )
        .write()
        .expect("bare vector fixture PDF")
    }

    #[test]
    fn rtl_analysis_shapes_arabic_and_preserves_logical_provenance() {
        let report = analyze_advanced_text_reflow(
            "Invoice 123 فاتورة",
            AdvancedTextMode::ParagraphReflowRtl,
            None,
            TextReflowLimits::default(),
        )
        .expect("rtl analysis");
        assert!(!report.bidi_runs.is_empty());
        assert!(report.used_complex_shaping);
        assert!(!report.existing_pdf_glyphs_reshaped);
        assert!(report.missing_glyph_clusters.is_empty());
    }

    #[test]
    fn vertical_analysis_reports_missing_cjk_in_latin_fallback() {
        let report = analyze_advanced_text_reflow(
            "縦書きABC。",
            AdvancedTextMode::ParagraphReflowVertical,
            None,
            TextReflowLimits::default(),
        )
        .expect("vertical analysis");
        assert_eq!(
            report.status,
            AdvancedEditingSupportStatus::UnsupportedReportedExact
        );
        assert!(!report.missing_glyph_clusters.is_empty());
        assert!(report
            .glyphs
            .iter()
            .any(|glyph| glyph.orientation == VerticalGlyphOrientation::RotateClockwise));
    }

    #[test]
    fn ink_fit_is_deterministic_and_error_bounded() {
        let points = (0..=200)
            .map(|index| {
                let x = index as f64 * 0.1;
                InkPoint {
                    x,
                    y: (x * 0.35).sin() * 4.0,
                }
            })
            .collect::<Vec<_>>();
        let options = InkFitOptions {
            error_threshold: 0.20,
            ..InkFitOptions::default()
        };
        let first = fit_ink_stroke(&points, &options).expect("first fit");
        let second = fit_ink_stroke(&points, &options).expect("second fit");
        assert_eq!(first.fitted_segments, second.fitted_segments);
        assert_eq!(first.report.output_sha256, second.report.output_sha256);
        assert!(first.report.maximum_deviation <= 0.35);
        assert!(first.report.segment_count < points.len());
    }

    #[test]
    fn ink_fit_rejects_non_finite_and_caps_recursion() {
        let err = fit_ink_stroke(
            &[InkPoint {
                x: f64::NAN,
                y: 0.0,
            }],
            &InkFitOptions::default(),
        )
        .expect_err("NaN must fail");
        assert!(err.to_string().contains("NaN or infinite"));
        let options = InkFitOptions {
            max_recursion: MAX_ADVANCED_EDITING_FIT_RECURSION + 1,
            ..InkFitOptions::default()
        };
        assert!(fit_ink_stroke(&[InkPoint { x: 0.0, y: 0.0 }], &options).is_err());
    }

    #[test]
    fn closed_stroke_preserves_closure() {
        let points = vec![
            InkPoint { x: 0.0, y: 0.0 },
            InkPoint { x: 10.0, y: 0.0 },
            InkPoint { x: 10.0, y: 10.0 },
            InkPoint { x: 0.0, y: 10.0 },
        ];
        let options = InkFitOptions {
            closed: true,
            corner_angle_degrees: 30.0,
            ..InkFitOptions::default()
        };
        let result = fit_ink_stroke(&points, &options).expect("closed fit");
        assert_eq!(result.cleaned_points.first(), result.cleaned_points.last());
        assert_eq!(
            result.fitted_segments.first().map(|s| s.p0),
            result.fitted_segments.last().map(|s| s.p3)
        );
    }

    #[test]
    fn same_width_patch_rewrites_one_token_and_preserves_prefix() {
        let input = advanced_editing_fixture(false);
        let options = SameWidthPatchOptions::default();
        let analysis =
            analyze_same_width_patch(&input, 1, "ABC", "DEF", &options).expect("eligibility");
        assert_eq!(analysis.candidates.len(), 1);
        assert!(analysis.candidates[0].eligible);
        let (output, report) =
            apply_same_width_patch(&input, 1, "ABC", "DEF", &options).expect("patch");
        assert!(output.starts_with(&input));
        assert!(report.replacement_extracts);
        assert!(report.old_text_absent);
    }

    #[test]
    fn multi_run_range_replaces_across_tj_and_tj_array_and_reopens() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 12 Tf 10 150 Td (ONE) Tj (TWO) Tj [(TH) 20 (REE)] TJ ET\n",
        );
        let model = analyze_multi_run_text_range(&input, 1).expect("range model");
        assert_eq!(model.logical_text, "ONETWOTHREE");
        assert_eq!(model.source_spans.len(), 4);
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 3,
            logical_end: 8,
            replacement_text: "X".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritLeading,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (output, report) =
            edit_multi_run_text_range(&input, &request, None).expect("range edit");
        assert!(output.starts_with(&input));
        assert_eq!(report.selected_source_spans.len(), 2);
        assert!(report.replacement_extracts);
        assert!(report.old_selected_text_absent);
        assert!(ContentEngine::open_bytes(output).is_ok());
    }

    #[test]
    fn multi_run_range_handles_quote_double_quote_insert_delete_and_undo() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 12 Tf 10 150 Td (ONE) Tj (TWO) ' 0 0 (THREE) \" ET\n",
        );
        let model = analyze_multi_run_text_range(&input, 1).expect("quote range model");
        assert_eq!(model.logical_text, "ONETWOTHREE");
        assert!(model.source_spans.iter().any(|span| span.operator == "'"));
        assert!(model.source_spans.iter().any(|span| span.operator == "\""));

        let replace = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 3,
            logical_end: 6,
            replacement_text: "X".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritLeading,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let mut session =
            AdvancedEditingMutationSession::new(input.clone()).expect("range session");
        session
            .apply_multi_run_text_range(&replace, None)
            .expect("replace through session");
        let edited = session.bytes().to_vec();
        assert!(session.patches()[0]
            .report
            .get("signature_policy")
            .is_some());
        assert!(ContentEngine::open_bytes(edited.clone())
            .expect("edited open")
            .get_page_text(1)
            .expect("edited text")
            .contains('X'));
        assert!(session.undo().expect("undo"));
        assert_eq!(session.bytes(), input);
        assert!(session.redo().expect("redo"));
        assert_eq!(session.bytes(), edited.as_slice());
        assert!(session.undo().expect("branch undo"));

        let insert = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 3,
            logical_end: 3,
            replacement_text: "Y".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritTrailing,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        session
            .apply_multi_run_text_range(&insert, None)
            .expect("insert through session");
        assert!(!session.redo().expect("redo cleared by branch edit"));

        let delete = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 3,
            logical_end: 6,
            replacement_text: String::new(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritLeading,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (_deleted, report) =
            edit_multi_run_text_range(&input, &delete, None).expect("delete range");
        assert_eq!(report.operation, "delete");
        assert!(report.old_selected_text_absent);
    }

    #[test]
    fn multi_run_range_covers_style_font_rtl_vertical_and_partial_boundaries() {
        let styled = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 10 Tf 0 g (ONE) Tj /F2 18 Tf 1 0 0 rg (TWO) Tj /F1 12 Tf (THREE) Tj ET\n",
        );
        let model = analyze_multi_run_text_range(&styled, 1).expect("styled range model");
        assert_eq!(model.logical_text, "ONETWOTHREE");
        assert!(model
            .source_spans
            .iter()
            .any(|span| span.font_resource == "F2"));
        let replace = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 3,
            logical_end: 11,
            replacement_text: "Z".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritLeading,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (_output, report) =
            edit_multi_run_text_range(&styled, &replace, None).expect("style boundary replace");
        assert_eq!(report.selected_source_spans.len(), 2);

        let rtl_text = "ABC \u{05d0}\u{05d1}\u{05d2} 123 DEF";
        let rtl_model = analyze_advanced_text_reflow(
            rtl_text,
            AdvancedTextMode::ParagraphReflowRtl,
            None,
            TextReflowLimits::default(),
        )
        .expect("rtl mapping");
        assert!(!rtl_model.bidi_runs.is_empty());

        let vertical = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 3,
            replacement_text: "X".to_string(),
            mode: AdvancedTextMode::ParagraphReflowVertical,
            style_policy: MultiRunStylePolicy::InheritLeading,
            options: AdvancedTextEditOptions {
                region: [100.0, 30.0, 120.0, 70.0],
                max_lines_or_columns: 4,
                ..AdvancedTextEditOptions::default()
            },
            final_lines: None,
        };
        let (_vertical_output, vertical_report) =
            edit_multi_run_text_range(&styled, &vertical, None).expect("vertical range edit");
        assert!(vertical_report.replacement_extracts);

        let partial = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 1,
            logical_end: 5,
            replacement_text: "bad".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritLeading,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (_partial_output, partial_report) =
            edit_multi_run_text_range(&styled, &partial, None).expect("partial-token range edit");
        assert!(partial_report.replacement_extracts);
        assert_eq!(partial_report.selected_source_spans.len(), 2);
    }

    #[test]
    fn preserve_per_segment_replays_source_font_size_color_and_positions() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 10 Tf 0 g 10 150 Td (ONE) Tj /F2 18 Tf 1 0 0 rg (TWO) Tj ET\n",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 6,
            replacement_text: "redSUN".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: AdvancedTextEditOptions {
                region: [10.0, 60.0, 180.0, 160.0],
                font_size: 12.0,
                line_spacing: 1.4,
                ..AdvancedTextEditOptions::default()
            },
            final_lines: Some(vec![ExplicitLayoutLine {
                logical_text: "redSUN".to_string(),
                visual_text: "redSUN".to_string(),
                inserted_visual_hyphen: false,
                bidi: None,
            }]),
        };
        let (output, report) =
            edit_multi_run_text_range(&input, &request, None).expect("preserved style reflow");
        assert_eq!(report.operation, "replace_preserving_per_segment_styles");
        assert!(report.replacement_extracts);
        assert!(report.old_selected_text_absent);
        let reopened = ContentEngine::open_bytes(output).expect("reopen");
        assert!(reopened.get_page_text(1).expect("text").contains("redSUN"));
        let page = reopened.document().get_page(1).expect("page");
        let (number, generation) = *page.contents.last().expect("generated content");
        let object = reopened
            .document()
            .reader()
            .get_object(number, generation)
            .expect("content object");
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reopened.document().reader(),
            &DecodeLimits::default(),
        )
        .expect("decode");
        let content = String::from_utf8(decoded.data).expect("content utf8");
        assert!(content.contains("/F1 10 Tf"));
        assert!(content.contains("/F2 18 Tf"));
        assert!(content.contains("1 0 0 rg"));
        assert!(content.matches(" Tm").count() >= 1);
    }

    #[test]
    fn preserve_per_segment_replays_mixed_styles_for_changed_length_replacement() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 10 Tf 0 g 10 150 Td (ONE) Tj /F2 18 Tf 1 0 0 rg (TWO) Tj ET\n",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 6,
            replacement_text: "summerDAY".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: AdvancedTextEditOptions {
                region: [10.0, 60.0, 180.0, 160.0],
                font_size: 12.0,
                line_spacing: 1.4,
                ..AdvancedTextEditOptions::default()
            },
            final_lines: Some(vec![ExplicitLayoutLine {
                logical_text: "summerDAY".to_string(),
                visual_text: "summerDAY".to_string(),
                inserted_visual_hyphen: false,
                bidi: None,
            }]),
        };
        let (output, report) = edit_multi_run_text_range(&input, &request, None)
            .expect("changed-length mixed-style reflow");
        assert!(report.replacement_extracts);
        assert!(report.old_selected_text_absent);
        let reopened = ContentEngine::open_bytes(output).expect("reopen");
        assert!(reopened
            .get_page_text(1)
            .expect("text")
            .contains("summerDAY"));
        let page = reopened.document().get_page(1).expect("page");
        let (number, generation) = *page.contents.last().expect("generated content");
        let object = reopened
            .document()
            .reader()
            .get_object(number, generation)
            .expect("content object");
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reopened.document().reader(),
            &DecodeLimits::default(),
        )
        .expect("decode");
        let content = String::from_utf8(decoded.data).expect("content utf8");
        assert!(content.contains("/F1 10 Tf"));
        assert!(content.contains("/F2 18 Tf"));
        assert!(content.contains("1 0 0 rg"));
    }

    #[test]
    fn preserve_per_segment_rewrites_inside_one_exact_mcid_without_duplication() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"/P << /MCID 7 >> BDC BT /F1 12 Tf 0 g 10 150 Td (ABC) Tj ET EMC\n",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 3,
            replacement_text: "XYZ".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: AdvancedTextEditOptions {
                region: [10.0, 60.0, 180.0, 160.0],
                ..AdvancedTextEditOptions::default()
            },
            final_lines: None,
        };
        let (output, report) =
            edit_multi_run_text_range(&input, &request, None).expect("MCID source-style reflow");
        assert!(report.replacement_extracts);
        assert!(report.old_selected_text_absent);
        assert_eq!(report.operation, "replace_tagged_text_in_source");
        let reopened = ContentEngine::open_bytes(output).expect("reopen");
        assert!(reopened.get_page_text(1).expect("text").contains("XYZ"));
        let page = reopened.document().get_page(1).expect("page");
        let source_object = reopened
            .document()
            .reader()
            .get_object(page.contents[0].0, page.contents[0].1)
            .expect("source stream");
        let source = decode_stream_lossless_with_limits(
            &source_object,
            reopened.document().reader(),
            &DecodeLimits::default(),
        )
        .expect("source decode");
        let source = String::from_utf8(source.data).expect("source UTF-8");
        assert!(source.contains("/P << /MCID 7 >> BDC"));
        assert_eq!(source.matches("/MCID 7").count(), 1);
        assert!(!source.contains("/Artifact BMC"));
    }

    #[test]
    fn preserve_per_segment_rewrites_partial_mcid_content_in_place() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"/P << /MCID 9 >> BDC BT /F1 12 Tf 10 150 Td (ABC) Tj (DEF) Tj ET EMC\n",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 3,
            replacement_text: "XYZ".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (output, report) = edit_multi_run_text_range(&input, &request, None)
            .expect("partial MCID content replacement");
        assert_eq!(report.operation, "replace_tagged_text_in_source");
        let reopened = ContentEngine::open_bytes(output).expect("reopen");
        assert!(reopened.get_page_text(1).expect("text").contains("XYZDEF"));
        let page = reopened.document().get_page(1).expect("page");
        let source_object = reopened
            .document()
            .reader()
            .get_object(page.contents[0].0, page.contents[0].1)
            .expect("source stream");
        let source = decode_stream_lossless_with_limits(
            &source_object,
            reopened.document().reader(),
            &DecodeLimits::default(),
        )
        .expect("source decode");
        let source = String::from_utf8(source.data).expect("source UTF-8");
        assert_eq!(source.matches("/MCID 9").count(), 1);
        assert!(!source.contains("/Artifact BMC"));
    }

    #[test]
    fn inline_tagged_replacement_commits_actual_text_cleanup_with_glyph_edit() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"/Span << /ActualText <FEFF004100420043> >> BDC BT /F1 12 Tf 10 150 Td (ABC) Tj ET EMC\n",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 3,
            replacement_text: "XYZ".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (output, report) = edit_multi_run_text_range(&input, &request, None)
            .expect("inline ActualText replacement");
        assert!(report.replacement_extracts);
        assert!(report.old_selected_text_absent);

        let reopened = ContentEngine::open_bytes(output).expect("reopen ActualText edit");
        let extracted = reopened.get_page_text(1).expect("extract replacement");
        assert!(extracted.contains("XYZ"));
        assert!(!extracted.contains("ABC"));
        let page = reopened.document().get_page(1).expect("page");
        let source_object = reopened
            .document()
            .reader()
            .get_object(page.contents[0].0, page.contents[0].1)
            .expect("source stream");
        let source = decode_stream_lossless_with_limits(
            &source_object,
            reopened.document().reader(),
            &DecodeLimits::default(),
        )
        .expect("source decode");
        let source = String::from_utf8(source.data).expect("source UTF-8");
        assert!(source.contains("/ActualText null"));
        assert!(!source.contains("FEFF004100420043"));
    }

    #[test]
    fn explicit_link_annotation_rect_move_preserves_action_and_quadpoints() {
        let input = advanced_editing_link_fixture();
        let (output, report) = move_link_annotation_rect_pdf(
            &input,
            1,
            0,
            [10.0, 140.0, 70.0, 160.0],
            12.0,
            -5.0,
            false,
        )
        .expect("explicit Link move");
        assert!(output.starts_with(&input));
        assert!(report.output_reopened);
        assert!(report.action_or_destination_preserved);
        assert!(report.moved_quad_points);
        assert_eq!(report.after_rect, [22.0, 135.0, 82.0, 155.0]);
        assert!(report.cache_invalidation.structured_render_write_set);
        assert_eq!(
            report.cache_invalidation.changed_object_refs,
            vec!["11 0 R"]
        );
        assert!(report.cache_invalidation.created_object_refs.is_empty());
        assert_eq!(
            report.cache_invalidation.render_write_set_refs,
            vec!["11 0 R"]
        );
        assert_eq!(report.cache_invalidation.affected_pages, vec![1]);
        assert_eq!(
            report.cache_invalidation.dirty_regions,
            vec![
                CacheInvalidationDirtyRegion {
                    page: 1,
                    region: [10.0, 140.0, 70.0, 160.0],
                    reason: "annotation_rect_before".to_string(),
                },
                CacheInvalidationDirtyRegion {
                    page: 1,
                    region: [22.0, 135.0, 82.0, 155.0],
                    reason: "annotation_rect_after".to_string(),
                }
            ]
        );
        let reopened = ContentEngine::open_bytes(output).expect("reopen");
        assert!(reopened.get_page_text(1).expect("text").contains("ABC"));
        let annotation = reopened
            .document()
            .reader()
            .get_object(11, 0)
            .expect("moved annotation")
            .as_dict()
            .cloned()
            .expect("annotation dictionary");
        assert!(annotation.get("A").is_some());
        assert_eq!(
            normalized_annotation_rect(
                &pdf_number_array(reopened.document().reader(), annotation.get("Rect"))
                    .expect("moved rect"),
            )
            .expect("normalized rect"),
            [22.0, 135.0, 82.0, 155.0]
        );
        let quad_points =
            pdf_number_array(reopened.document().reader(), annotation.get("QuadPoints"))
                .expect("moved quad points");
        assert_eq!(
            quad_points,
            vec![22.0, 155.0, 82.0, 155.0, 22.0, 135.0, 82.0, 135.0]
        );
    }

    #[test]
    fn explicit_link_annotation_rect_move_rejects_stale_source_geometry() {
        let input = advanced_editing_link_fixture();
        let error =
            move_link_annotation_rect_pdf(&input, 1, 0, [0.0, 0.0, 1.0, 1.0], 1.0, 1.0, false)
                .expect_err("stale source rect must refuse");
        assert!(error.to_string().contains("stale_snapshot"));
        assert!(ContentEngine::open_bytes(input).is_ok());
    }

    #[test]
    fn preserve_per_segment_shapes_bidi_with_per_grapheme_style_ownership() {
        let input =
            advanced_editing_fixture_with_content(false, b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n");
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 3,
            replacement_text: "\u{05d0}\u{05d1}\u{05d2}".to_string(),
            mode: AdvancedTextMode::ParagraphReflowRtl,
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (_output, report) = edit_multi_run_text_range(&input, &request, None)
            .expect("bidi mixed-style replacement");
        assert_eq!(
            report.operation,
            "replace_shaped_preserving_per_segment_styles"
        );
        assert!(report.replacement_extracts);
    }

    #[test]
    fn preserve_per_segment_rewrites_source_text_clipping_mode_inline() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 12 Tf 4 Tr 10 150 Td (ABC) Tj ET\n0 0 20 20 re f\n",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 3,
            replacement_text: "XYZ".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (_output, report) =
            edit_multi_run_text_range(&input, &request, None).expect("clipping source replacement");
        assert_eq!(report.operation, "replace_clipping_text_in_source");
        assert!(report.reachable_source_tokens_removed);
    }

    #[test]
    fn preserve_per_segment_replays_exact_source_color_space_commands() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 12 Tf /Pattern cs /P1 scn 10 150 Td (ABC) Tj ET\n",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 3,
            replacement_text: "XYZ".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (output, report) = edit_multi_run_text_range(&input, &request, None)
            .expect("pattern color-space replacement");
        assert!(report.replacement_extracts);
        let reopened = ContentEngine::open_bytes(output).expect("reopen");
        let page = reopened.document().get_page(1).expect("page");
        let (number, generation) = *page.contents.last().expect("generated content");
        let object = reopened
            .document()
            .reader()
            .get_object(number, generation)
            .expect("content object");
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reopened.document().reader(),
            &DecodeLimits::default(),
        )
        .expect("decode");
        let content = String::from_utf8(decoded.data).expect("content utf8");
        assert!(content.contains("/Pattern cs /P1 scn"));
    }

    #[test]
    fn preserve_per_segment_replays_invisible_text_without_clipping_side_effects() {
        let input = advanced_editing_fixture_with_content(
            false,
            b"BT /F1 12 Tf 3 Tr 10 150 Td (ABC) Tj ET\n",
        );
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: 0,
            logical_end: 3,
            replacement_text: "XYZ".to_string(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::PreservePerSegment,
            options: AdvancedTextEditOptions::default(),
            final_lines: None,
        };
        let (output, report) =
            edit_multi_run_text_range(&input, &request, None).expect("invisible source reflow");
        assert!(report.replacement_extracts);
        let reopened = ContentEngine::open_bytes(output).expect("reopen");
        let page = reopened.document().get_page(1).expect("page");
        let (number, generation) = *page.contents.last().expect("generated content");
        let object = reopened
            .document()
            .reader()
            .get_object(number, generation)
            .expect("content object");
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reopened.document().reader(),
            &DecodeLimits::default(),
        )
        .expect("decode");
        assert!(String::from_utf8(decoded.data)
            .expect("content")
            .contains("3 Tr"));
    }

    #[test]
    fn vector_inventory_and_range_edit_round_trip() {
        let input = advanced_editing_fixture(false);
        let inventory = list_vector_objects(&input, 1).expect("inventory");
        assert_eq!(inventory.objects.len(), 1);
        assert!(matches!(
            inventory.objects[0].segments[0],
            VectorPathSegment::Rectangle { .. }
        ));
        let (output, report) = edit_vector_object(
            &input,
            1,
            &inventory.objects[0].stable_id,
            VectorEditOperation::Move { dx: 5.0, dy: 7.0 },
            &VectorEditOptions::default(),
        )
        .expect("vector edit");
        assert!(output.starts_with(&input));
        assert!(report.unrelated_decoded_prefix_preserved);
        assert!(report.unrelated_decoded_suffix_preserved);
        assert!(report.output_reopened);
    }

    #[test]
    fn shared_form_edit_all_and_clone_one_are_explicit_and_safe() {
        let input = shared_form_fixture();
        let inventory = list_vector_objects(&input, 1).expect("Form inventory");
        assert_eq!(inventory.objects.len(), 2);
        assert_ne!(
            inventory.objects[0].stable_id,
            inventory.objects[1].stable_id
        );
        assert!(inventory.objects.iter().all(|object| object
            .provenance
            .form_invocation
            .as_ref()
            .is_some_and(|invocation| invocation.form_object == 5)));

        let selected = &inventory.objects[0];
        let reject = edit_vector_object(
            &input,
            1,
            &selected.stable_id,
            VectorEditOperation::Move { dx: 3.0, dy: 4.0 },
            &VectorEditOptions::default(),
        )
        .expect_err("shared Form edit must be explicit");
        assert!(reject.to_string().contains("select shared_form_policy"));

        let edit_all_options = VectorEditOptions {
            shared_form_policy: SharedFormEditPolicy::EditAllUses,
            ..VectorEditOptions::default()
        };
        let (edit_all_output, edit_all_report) = edit_vector_object(
            &input,
            1,
            &selected.stable_id,
            VectorEditOperation::Move { dx: 3.0, dy: 4.0 },
            &edit_all_options,
        )
        .expect("explicit edit-all");
        assert!(edit_all_output.starts_with(&input));
        assert!(edit_all_report.cloned_form.is_none());
        assert!(edit_all_report.clone_graph[0].starts_with("edit_all_uses:"));

        let clone_options = VectorEditOptions {
            shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
            ..VectorEditOptions::default()
        };
        let (clone_output, clone_report) = edit_vector_object(
            &input,
            1,
            &selected.stable_id,
            VectorEditOperation::Move { dx: 3.0, dy: 4.0 },
            &clone_options,
        )
        .expect("clone one instance");
        assert!(clone_output.starts_with(&input));
        let cloned = clone_report.cloned_form.expect("cloned Form object");
        assert_ne!(cloned[0], 5);
        assert_eq!(clone_report.clone_graph.len(), 2);
        let clone_inventory = list_vector_objects(&clone_output, 1).expect("clone inventory");
        let owners = clone_inventory
            .objects
            .iter()
            .map(|object| object.provenance.object_number)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(owners.contains(&5));
        assert!(owners.contains(&cloned[0]));
    }

    #[test]
    fn nested_form_clone_one_clones_leaf_and_parent_path() {
        let input = nested_shared_form_fixture();
        let inventory = list_vector_objects(&input, 1).expect("nested inventory");
        assert_eq!(inventory.objects.len(), 2);
        let selected = inventory.objects.first().expect("nested vector");
        assert_eq!(selected.provenance.form_invocation_path.len(), 2);
        let (output, report) = edit_vector_object(
            &input,
            1,
            &selected.stable_id,
            VectorEditOperation::Move { dx: 3.0, dy: 4.0 },
            &VectorEditOptions {
                shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
                ..VectorEditOptions::default()
            },
        )
        .expect("nested clone one");
        assert!(output.starts_with(&input));
        assert!(report.output_reopened);
        assert!(report.clone_graph.len() >= 2);
        let reopened = list_vector_objects(&output, 1).expect("nested reopen");
        let owners = reopened
            .objects
            .iter()
            .map(|object| object.provenance.object_number)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(owners.contains(&6));
        assert!(owners.iter().any(|owner| *owner > 6));
    }

    #[test]
    fn nested_form_clone_one_depth_three_is_transactional_and_deterministic() {
        let input = nested_shared_form_depth_fixture(3);
        let inventory = list_vector_objects(&input, 1).expect("depth three inventory");
        assert_eq!(inventory.objects.len(), 2);
        let selected = inventory.objects.first().expect("depth three vector");
        assert_eq!(selected.provenance.form_invocation_path.len(), 3);
        let mut session =
            AdvancedEditingMutationSession::new(input.clone()).expect("vector session");
        session
            .apply_vector(
                1,
                &selected.stable_id,
                VectorEditOperation::Move { dx: 2.0, dy: 3.0 },
                &VectorEditOptions {
                    shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
                    ..VectorEditOptions::default()
                },
            )
            .expect("depth three clone one");
        let edited = session.bytes().to_vec();
        assert!(edited.starts_with(&input));
        let report = session.patches()[0].report.clone();
        assert!(report
            .get("clone_graph")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|graph| graph.len() >= 3));
        let reopened = list_vector_objects(&edited, 1).expect("depth three reopen");
        let owners = reopened
            .objects
            .iter()
            .map(|object| object.provenance.object_number)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(owners.contains(&7));
        assert!(owners.iter().any(|owner| *owner > 7));
        assert!(session.undo().expect("undo depth three"));
        assert_eq!(session.bytes(), input);
        assert!(session.redo().expect("redo depth three"));
        assert_eq!(session.bytes(), edited.as_slice());

        let (_, repeat_report) = edit_vector_object(
            &input,
            1,
            &selected.stable_id,
            VectorEditOperation::Move { dx: 2.0, dy: 3.0 },
            &VectorEditOptions {
                shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
                ..VectorEditOptions::default()
            },
        )
        .expect("repeat depth three clone one");
        let session_graph = report
            .get("clone_graph")
            .and_then(serde_json::Value::as_array)
            .expect("session clone graph")
            .clone();
        let repeat_graph = repeat_report
            .clone_graph
            .iter()
            .map(|entry| serde_json::Value::String(entry.clone()))
            .collect::<Vec<_>>();
        assert_eq!(session_graph, repeat_graph);
    }

    #[test]
    fn shared_annotation_appearance_clone_one_preserves_other_owner_and_as() {
        let input = shared_annotation_appearance_fixture();
        let inventory = list_vector_objects(&input, 1).expect("AP inventory");
        assert_eq!(inventory.objects.len(), 2);
        assert!(inventory
            .objects
            .iter()
            .all(|item| item.edit_safety == "shared_annotation_appearance_requires_clone"));
        let (output, report) = edit_vector_object(
            &input,
            1,
            &inventory.objects[0].stable_id,
            VectorEditOperation::SetStrokeWidth { width: 4.0 },
            &VectorEditOptions {
                shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
                ..VectorEditOptions::default()
            },
        )
        .expect("AP clone one");
        assert!(output.starts_with(&input));
        assert!(report.output_reopened);
        assert!(report.clone_graph[0].contains("AP/N"));
        let reopened = list_vector_objects(&output, 1).expect("AP reopen");
        let owners = reopened
            .objects
            .iter()
            .map(|item| item.provenance.object_number)
            .collect::<std::collections::BTreeSet<_>>();
        assert!(owners.contains(&8));
        assert!(owners.iter().any(|number| *number > 8));
    }

    #[test]
    fn shared_annotation_r_and_d_clone_one_preserve_unaffected_owners() {
        for (appearance, source_object) in [("R", 9), ("D", 10)] {
            let input = shared_annotation_categories_fixture(&["N", "R", "D"]);
            let inventory = list_vector_objects(&input, 1).expect("N/R/D inventory");
            let selected = inventory
                .objects
                .iter()
                .find(|object| {
                    object
                        .provenance
                        .resource_owner
                        .contains(&format!("annotation-0-appearance-{appearance}-"))
                })
                .expect("target AP vector");
            let (output, report) = edit_vector_object(
                &input,
                1,
                &selected.stable_id,
                VectorEditOperation::SetStrokeWidth { width: 5.0 },
                &VectorEditOptions {
                    shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
                    ..VectorEditOptions::default()
                },
            )
            .expect("R/D AP clone one");
            assert!(output.starts_with(&input));
            assert!(report.output_reopened);
            assert_eq!(
                annotation_ap_ref(&output, 1, appearance),
                (source_object, 0)
            );
            assert_eq!(
                annotation_ap_ref(&output, 2, appearance),
                (source_object, 0)
            );
            assert_ne!(
                annotation_ap_ref(&output, 0, appearance),
                (source_object, 0)
            );
            assert_eq!(annotation_as_name(&output, 0), "On");
            assert_eq!(annotation_ap_ref(&output, 0, "N"), (8, 0));
        }
    }

    #[test]
    fn shared_annotation_state_and_widget_clone_one_preserve_as_and_sibling_states() {
        for widget in [None, Some("checkbox"), Some("radio")] {
            let input = shared_annotation_state_fixture(widget);
            let state = if widget.is_some() { "Yes" } else { "On" };
            let appearance = format!("N/{state}");
            let inventory = list_vector_objects(&input, 1).expect("state AP inventory");
            let selected = inventory
                .objects
                .iter()
                .find(|object| {
                    object
                        .provenance
                        .resource_owner
                        .contains(&format!("annotation-0-appearance-{appearance}-"))
                })
                .expect("target state vector");
            let (output, report) = edit_vector_object(
                &input,
                1,
                &selected.stable_id,
                VectorEditOperation::SetStrokeWidth { width: 4.0 },
                &VectorEditOptions {
                    shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
                    ..VectorEditOptions::default()
                },
            )
            .expect("state AP clone one");
            assert!(report.output_reopened);
            assert_ne!(annotation_ap_ref(&output, 0, &appearance), (8, 0));
            assert_eq!(annotation_ap_ref(&output, 1, &appearance), (8, 0));
            assert_eq!(annotation_ap_ref(&output, 0, "N/Off"), (9, 0));
            assert_eq!(annotation_ap_ref(&output, 1, "N/Off"), (9, 0));
            assert_eq!(annotation_as_name(&output, 0), state);
            assert!(report.clone_graph[0].contains(&format!("AP/{appearance}")));
        }
    }

    #[test]
    fn nested_shared_annotation_appearance_clones_ap_owner_and_leaf_form_only() {
        let input = nested_annotation_appearance_fixture();
        let inventory = list_vector_objects(&input, 1).expect("nested AP inventory");
        let selected = inventory
            .objects
            .iter()
            .find(|object| {
                object
                    .provenance
                    .form_stack
                    .first()
                    .is_some_and(|stack| stack == "annotation:0:appearance:N")
                    && object.provenance.object_number == 9
            })
            .expect("nested AP vector");
        let (output, report) = edit_vector_object(
            &input,
            1,
            &selected.stable_id,
            VectorEditOperation::SetStrokeWidth { width: 6.0 },
            &VectorEditOptions {
                shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
                ..VectorEditOptions::default()
            },
        )
        .expect("nested AP clone one");
        assert!(output.starts_with(&input));
        assert!(report.output_reopened);
        assert!(report
            .clone_graph
            .iter()
            .any(|entry| entry.contains("AP/N cloned")));
        assert_ne!(annotation_ap_ref(&output, 0, "N"), (8, 0));
        assert_eq!(annotation_ap_ref(&output, 1, "N"), (8, 0));
        assert_eq!(annotation_as_name(&output, 0), "On");
        let reopened = list_vector_objects(&output, 1).expect("nested AP reopen inventory");
        assert!(reopened
            .objects
            .iter()
            .any(|object| object.provenance.object_number == 9));
        assert!(reopened
            .objects
            .iter()
            .any(|object| object.provenance.object_number > 9));
    }

    #[test]
    fn bounded_page_z_order_moves_selected_object_and_reopens() {
        let input = bare_vector_fixture(b"1 0 0 rg 10 10 20 20 re f\n0 0 1 rg 60 60 20 20 re f\n");
        let inventory = list_vector_objects(&input, 1).expect("z-order inventory");
        assert_eq!(inventory.objects.len(), 2);
        let first = inventory.objects[0].stable_id.clone();
        let (output, report) = edit_vector_object(
            &input,
            1,
            &first,
            VectorEditOperation::BringToFront,
            &VectorEditOptions::default(),
        )
        .expect("bring to front");
        assert!(output.starts_with(&input));
        assert!(report.output_reopened);
        assert!(report.unrelated_decoded_prefix_preserved);
        assert!(report.unrelated_decoded_suffix_preserved);
        let reopened = list_vector_objects(&output, 1).expect("z-order reopen inventory");
        assert_eq!(reopened.objects.len(), 2);
        assert!(reopened.objects[1].bbox[0] < reopened.objects[0].bbox[0]);
    }

    #[test]
    fn bounded_contiguous_group_and_ungroup_round_trip() {
        let input = bare_vector_fixture(b"1 0 0 rg 10 10 20 20 re f\n0 0 1 rg 60 60 20 20 re f\n");
        let inventory = list_vector_objects(&input, 1).expect("group inventory");
        let first = inventory.objects[0].stable_id.clone();
        let second = inventory.objects[1].stable_id.clone();
        let (grouped, group_report) = edit_vector_object(
            &input,
            1,
            &first,
            VectorEditOperation::GroupWith {
                stable_ids: vec![second],
            },
            &VectorEditOptions::default(),
        )
        .expect("bounded group");
        assert!(grouped.starts_with(&input));
        assert!(group_report.output_reopened);
        let grouped_inventory = list_vector_objects(&grouped, 1).expect("grouped inventory");
        assert_eq!(grouped_inventory.objects.len(), 2);
        assert!(grouped_inventory.objects.iter().all(|object| object
            .provenance
            .wellfriendpdf_groups
            .len()
            == 1));

        let grouped_first = grouped_inventory.objects[0].stable_id.clone();
        let (ungrouped, ungroup_report) = edit_vector_object(
            &grouped,
            1,
            &grouped_first,
            VectorEditOperation::Ungroup,
            &VectorEditOptions::default(),
        )
        .expect("bounded ungroup");
        assert!(ungrouped.starts_with(&grouped));
        assert!(ungroup_report.output_reopened);
        let ungrouped_inventory = list_vector_objects(&ungrouped, 1).expect("ungrouped inventory");
        assert!(ungrouped_inventory
            .objects
            .iter()
            .all(|object| object.provenance.wellfriendpdf_groups.is_empty()));
    }

    #[test]
    fn mutation_session_undo_redo_and_branch_clear_use_incremental_patches() {
        let input = advanced_editing_fixture(false);
        let mut session =
            AdvancedEditingMutationSession::new(input.clone()).expect("mutation session");
        let patch_options = SameWidthPatchOptions {
            mode: SameWidthMode::Tolerance,
            advance_tolerance_1000: 200.0,
            ..SameWidthPatchOptions::default()
        };
        session
            .apply_same_width_patch(1, "ABC", "DEF", &patch_options)
            .expect("first patch");
        let first_output = session.bytes().to_vec();
        assert!(first_output.starts_with(&input));
        assert_eq!(session.cursor(), 1);
        assert!(session.undo().expect("undo"));
        assert_eq!(session.bytes(), input);
        assert!(session.redo().expect("redo"));
        assert_eq!(session.bytes(), first_output);
        assert!(session.undo().expect("branch undo"));
        session
            .apply_same_width_patch(1, "ABC", "XYZ", &patch_options)
            .expect("branch patch");
        assert_eq!(session.patches().len(), 1);
        assert_eq!(session.cursor(), 1);
        assert!(!session.redo().expect("redo cleared"));
        assert!(session.patches()[0].appended_bytes > 0);
        assert_eq!(session.checkpoints().len(), 1);
    }

    #[test]
    fn annotation_ink_fit_saves_cubic_appearance_and_raw_points() {
        let input = advanced_editing_fixture(true);
        let (output, report) =
            fit_annotation_ink_pdf(&input, 1, 0, &InkFitOptions::default(), false)
                .expect("annotation fit");
        assert!(output.starts_with(&input));
        assert!(report.raw_points_preserved);
        assert!(report.fitted_curves_stored);
        assert!(report.appearance_readback);
        let annotation_ref = format!(
            "{} {} R",
            report.annotation_object, report.annotation_generation
        );
        let appearance_ref = format!("{} 0 R", report.appearance_object);
        assert!(report.cache_invalidation.structured_render_write_set);
        assert_eq!(
            report.cache_invalidation.changed_object_refs,
            vec![annotation_ref.clone()]
        );
        assert_eq!(
            report.cache_invalidation.created_object_refs,
            vec![appearance_ref.clone()]
        );
        assert_eq!(
            report.cache_invalidation.render_write_set_refs,
            vec![annotation_ref, appearance_ref]
        );
        assert_eq!(report.cache_invalidation.affected_pages, vec![1]);
        assert_eq!(report.cache_invalidation.dirty_regions.len(), 1);
        assert_eq!(report.cache_invalidation.dirty_regions[0].page, 1);
        assert_eq!(
            report.cache_invalidation.dirty_regions[0].reason,
            "annotation_ink_appearance_regenerated"
        );
        let inventory = list_vector_objects(&output, 1).expect("annotation appearance inventory");
        let appearance = inventory
            .objects
            .iter()
            .find(|object| {
                object
                    .provenance
                    .resource_owner
                    .starts_with("annotation-0-appearance")
            })
            .expect("editable annotation appearance vector");
        let (edited, edit_report) = edit_vector_object(
            &output,
            1,
            &appearance.stable_id,
            VectorEditOperation::SetStrokeWidth { width: 2.5 },
            &VectorEditOptions::default(),
        )
        .expect("annotation appearance vector edit");
        assert!(edited.starts_with(&output));
        assert!(edit_report.output_reopened);
    }

    #[test]
    fn rtl_reflow_embeds_type0_removes_old_text_and_reopens() {
        let input = advanced_editing_fixture(false);
        let options = AdvancedTextEditOptions {
            region: [20.0, 100.0, 180.0, 145.0],
            font_size: 14.0,
            ..AdvancedTextEditOptions::default()
        };
        let (output, report) = edit_advanced_text_pdf(
            &input,
            1,
            "ABC",
            "فاتورة 123",
            AdvancedTextMode::ParagraphReflowRtl,
            &options,
            None,
        )
        .expect("rtl edit");
        assert!(output.starts_with(&input));
        assert!(report.replacement_extracts);
        assert!(report.old_text_absent);
        assert_eq!(report.writing_mode, 0);
    }

    #[test]
    fn explicit_final_layout_uses_bounded_output_driving_justification() {
        let input = advanced_editing_fixture(false);
        let first_line = "one two three four five";
        let analysis = analyze_advanced_text_reflow(
            first_line,
            AdvancedTextMode::ParagraphReflowHorizontal,
            None,
            TextReflowLimits::default(),
        )
        .expect("shaped first line");
        let font_size = 12.0;
        let natural = analysis
            .glyphs
            .iter()
            .map(|glyph| glyph.advance_1000.abs())
            .sum::<f64>()
            / 1000.0
            * font_size;
        let replacement = format!("{first_line}tail");
        let options = AdvancedTextEditOptions {
            region: [20.0, 100.0, 20.0 + natural + 4.0, 145.0],
            font_size,
            max_lines_or_columns: 2,
            alignment: GeneratedTextAlignment::Justify,
            ..AdvancedTextEditOptions::default()
        };
        let (output, report) = edit_advanced_text_pdf_with_layout(
            &input,
            1,
            "ABC",
            &replacement,
            AdvancedTextMode::ParagraphReflowHorizontal,
            &options,
            None,
            &[first_line.to_string(), "tail".to_string()],
        )
        .expect("bounded justified layout");
        assert!(output.starts_with(&input));
        assert!(report.replacement_extracts);
        assert_eq!(report.line_adjustments.len(), 2);
        assert!(report.line_adjustments[0].word_spacing > 0.0);
        assert!(report.line_adjustments[0].residual <= 1e-6);
        assert!(report.line_adjustments[1].last_line);
        assert_eq!(
            report.line_adjustments[1].word_spacing, 0.0,
            "the default policy does not justify a final line"
        );
    }

    #[test]
    fn rtl_justification_anchors_using_final_painted_width() {
        let glyph = |cid, logical_byte_start, visual_unicode: &str| GeneratedGlyph {
            cid,
            gid: cid,
            logical_byte_start,
            visual_unicode: visual_unicode.to_string(),
            to_unicode: Some(visual_unicode.to_string()),
            advance: 2_000.0,
            offset_x: 0.0,
            offset_y: 0.0,
            orientation: VerticalGlyphOrientation::Upright,
            cross_advance: 0.0,
            font_width: 2_000.0,
            bounds: None,
        };
        let layout = vec![vec![
            glyph(1, 0, "\u{05D0}"),
            glyph(2, 2, " "),
            glyph(3, 3, "\u{05D1}"),
        ]];
        let options = AdvancedTextEditOptions {
            region: [0.0, 0.0, 100.0, 20.0],
            font_size: 10.0,
            alignment: GeneratedTextAlignment::Justify,
            justify_last_line: true,
            max_word_spacing: 4.0,
            max_character_spacing: 0.0,
            ..AdvancedTextEditOptions::default()
        };
        let (content, adjustments) =
            serialize_generated_text(&layout, "FJ", &options, false, None, None)
                .expect("RTL justified serialization");
        assert!(adjustments[0].residual <= EPSILON);
        let first_glyph = content
            .lines()
            .find(|line| line.contains(" Tm <"))
            .expect("first positioned glyph");
        assert!(
            first_glyph.starts_with("1 0 0 1 0 10 Tm"),
            "RTL justified line must start at the left edge after expanding to full width: {first_glyph}"
        );
    }

    #[test]
    fn explicit_continuation_keeps_parent_bidi_levels() {
        let paragraph = "start \u{2067}אבג 123 / next\u{2069} end";
        let start = paragraph.find("123").unwrap();
        let visual = "123 /";
        let bidi = crate::fonts::shaper::resolve_line_bidi(
            paragraph,
            start..start + visual.len(),
            ShapeOptions {
                direction: Some(TextDirection::LeftToRight),
            },
        )
        .unwrap();
        let font = crate::render::get_fallback_font("Symbol").unwrap();
        let expected =
            TextShaper::shape_resolved(font, visual, &bidi, &Default::default()).unwrap();
        let line = ExplicitLayoutLine {
            logical_text: visual.into(),
            visual_text: visual.into(),
            inserted_visual_hyphen: false,
            bidi: Some(bidi),
        };
        let options = AdvancedTextEditOptions {
            region: [0.0, 0.0, 1000.0, 1000.0],
            ..Default::default()
        };
        let output = layout_generated_explicit_lines(
            &[line],
            AdvancedTextMode::ParagraphReflowHorizontal,
            font,
            &options,
            None,
        )
        .unwrap();
        assert_eq!(
            output[0]
                .iter()
                .map(|g| g.logical_byte_start)
                .collect::<Vec<_>>(),
            expected
                .glyphs
                .iter()
                .map(|g| g.cluster as usize)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn vertical_reflow_uses_identity_v_and_column_layout() {
        let input = advanced_editing_fixture(false);
        let options = AdvancedTextEditOptions {
            region: [100.0, 30.0, 180.0, 145.0],
            font_size: 12.0,
            ..AdvancedTextEditOptions::default()
        };
        let (output, report) = edit_advanced_text_pdf(
            &input,
            1,
            "ABC",
            "VERTICAL",
            AdvancedTextMode::ParagraphReflowVertical,
            &options,
            None,
        )
        .expect("vertical edit");
        assert!(output.starts_with(&input));
        assert!(report.replacement_extracts);
        assert_eq!(report.writing_mode, 1);
        assert_eq!(report.lines_or_columns, 1);
    }
}
