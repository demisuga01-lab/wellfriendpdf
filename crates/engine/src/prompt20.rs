//! Combined Prompt 20 advanced editing primitives.
//!
//! This module is the shared implementation surface for vertical/RTL text
//! analysis, byte-preserving text patching, vector-object editing, and ink
//! curve fitting.  Existing PDF glyph streams remain provenance-bearing PDF
//! codes; only newly inserted Unicode text is shaped.

use std::collections::BTreeSet;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_bidi::{BidiInfo, Level};
use unicode_segmentation::UnicodeSegmentation;

use crate::error::{Result, WellfriendError};
use crate::filters::{
    decode_stream_lossless_with_limits, flate_encode, DecodeLimits, StreamDecodeStatus,
};
use crate::fonts::{FontResolver, FontType, ShapeOptions, TextDirection, TextShaper};
use crate::object::PdfObject;
use crate::prompt18::{
    analyze_edit_policy, EditOperation as SignatureEditOperation, EditPolicyDecision,
    EditPolicyReport,
};
use crate::render::get_fallback_font;
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::{ContentEngine, PageResources};

pub const PROMPT20_SCHEMA_VERSION: &str = "prompt20.vertical-rtl-patch-vector-ink-editing.v1";

pub const MAX_PROMPT20_PARAGRAPH_CHARS: usize = 1_000_000;
pub const MAX_PROMPT20_BIDI_RUNS: usize = 4096;
pub const MAX_PROMPT20_GLYPHS: usize = 1_000_000;
pub const MAX_PROMPT20_INK_POINTS: usize = 1_000_000;
pub const MAX_PROMPT20_INK_SEGMENTS: usize = 100_000;
pub const MAX_PROMPT20_FIT_RECURSION: usize = 32;
pub const MAX_PROMPT20_PATCH_STREAM_BYTES: usize = 256 * 1024 * 1024;
const EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prompt20SupportStatus {
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedExact,
    UnsupportedReportedSecurityPolicy,
    NotInPrompt20Scope,
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
            max_paragraph_chars: MAX_PROMPT20_PARAGRAPH_CHARS,
            max_bidi_runs: MAX_PROMPT20_BIDI_RUNS,
            max_glyphs: MAX_PROMPT20_GLYPHS,
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
    pub status: Prompt20SupportStatus,
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

/// Horizontal alignment for generated Prompt 20 text.  `Justify` changes
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
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AdvancedTextEditReport {
    pub schema_version: String,
    pub status: Prompt20SupportStatus,
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
    /// Optional Prompt 33 final visual layout. When supplied it must cover the
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
    pub font_resource: String,
    pub writing_mode: i32,
    pub marked_content_depth: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MultiRunRangeModel {
    pub schema_version: String,
    pub status: Prompt20SupportStatus,
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
    pub status: Prompt20SupportStatus,
    pub operation: String,
    pub page: usize,
    pub logical_range: [usize; 2],
    pub selected_source_spans: Vec<MultiRunSourceSpan>,
    pub style_policy: MultiRunStylePolicy,
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
            "prompt20 text analysis requires a paragraph reflow mode, got {mode:?}"
        )));
    }
    let char_count = text.chars().count();
    if char_count > limits.max_paragraph_chars.min(MAX_PROMPT20_PARAGRAPH_CHARS) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 paragraph has {char_count} characters; limit is {}",
            limits.max_paragraph_chars.min(MAX_PROMPT20_PARAGRAPH_CHARS)
        )));
    }
    reject_unsafe_text_controls(text)?;

    let base_level = match mode {
        AdvancedTextMode::ParagraphReflowRtl => Level::rtl(),
        _ => Level::ltr(),
    };
    let bidi = BidiInfo::new(text, Some(base_level));
    let visual_text = bidi
        .paragraphs
        .iter()
        .map(|paragraph| bidi.reorder_line(paragraph, paragraph.range.clone()))
        .collect::<String>();
    let mut runs = Vec::new();
    for paragraph in &bidi.paragraphs {
        let (levels, ranges) = bidi.visual_runs(paragraph, paragraph.range.clone());
        for (run_index, (level, range)) in levels.into_iter().zip(ranges).enumerate() {
            if runs.len() >= limits.max_bidi_runs.min(MAX_PROMPT20_BIDI_RUNS) {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "prompt20 bidi run count exceeds limit {}",
                    limits.max_bidi_runs.min(MAX_PROMPT20_BIDI_RUNS)
                )));
            }
            let logical = text.get(range.clone()).ok_or_else(|| {
                WellfriendError::ParseError(
                    "prompt20 bidi run is not on UTF-8 boundaries".to_string(),
                )
            })?;
            let visual = if level.is_rtl() {
                logical.chars().rev().collect()
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
                "prompt20 bundled fallback font unavailable".to_string(),
            )
        })?;
    let face = ttf_parser::Face::parse(font, 0).map_err(|_| {
        WellfriendError::UnsupportedFeature("prompt20 invalid shaping font".to_string())
    })?;
    let mut glyphs = Vec::new();
    let mut missing = Vec::new();
    let mut complex = false;
    for (source_run_index, run) in runs.iter().enumerate() {
        let shaped = TextShaper::shape(
            font,
            &run.logical_text,
            ShapeOptions {
                direction: Some(if run.right_to_left {
                    TextDirection::RightToLeft
                } else {
                    TextDirection::LeftToRight
                }),
            },
        )?;
        complex |= shaped.used_complex_shaping;
        for glyph in shaped.glyphs {
            if glyphs.len() >= limits.max_glyphs.min(MAX_PROMPT20_GLYPHS) {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "prompt20 shaped glyph count exceeds limit {}",
                    limits.max_glyphs.min(MAX_PROMPT20_GLYPHS)
                )));
            }
            let is_missing = glyph.glyph_id == 0;
            if is_missing {
                missing.push(glyph.cluster);
            }
            let cluster_char = run
                .logical_text
                .get(glyph.cluster as usize..)
                .and_then(|tail| tail.chars().next());
            let orientation = match mode {
                AdvancedTextMode::ParagraphReflowVertical => cluster_char
                    .map(vertical_orientation)
                    .unwrap_or(VerticalGlyphOrientation::Upright),
                _ => VerticalGlyphOrientation::Upright,
            };
            // Check the font cmap separately so a malformed shaper result cannot
            // turn an absent source character into a supported claim.
            let cmap_missing = cluster_char
                .filter(|ch| !ch.is_control())
                .is_some_and(|ch| face.glyph_index(ch).is_none());
            if cmap_missing && !missing.contains(&glyph.cluster) {
                missing.push(glyph.cluster);
            }
            glyphs.push(TextGlyphProvenance {
                glyph_id: glyph.glyph_id,
                source_cluster_utf8: glyph.cluster,
                source_run_index,
                advance_1000: canonical_number(glyph.advance),
                offset_x_1000: canonical_number(glyph.offset_x),
                offset_y_1000: canonical_number(glyph.offset_y),
                orientation,
                missing: is_missing || cmap_missing,
            });
        }
    }
    missing.sort_unstable();
    missing.dedup();
    Ok(TextReflowAnalysis {
        schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
        status: if missing.is_empty() {
            Prompt20SupportStatus::ImplementedWithLimits
        } else {
            Prompt20SupportStatus::UnsupportedReportedExact
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
            "vertical orientation is Unicode-policy analysis; serialization requires an owned Identity-V font mapping and vertical metrics".to_string(),
            "missing glyphs are fail-closed and never silently replaced by .notdef".to_string(),
        ],
    })
}

#[derive(Debug, Clone)]
struct GeneratedGlyph {
    cid: u16,
    gid: u16,
    visual_unicode: String,
    to_unicode: Option<String>,
    advance: f64,
    orientation: VerticalGlyphOrientation,
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
/// bounded Prompt 20 mutation path as [`edit_advanced_text_pdf`].
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
            "prompt20 explicit final lines must be nonempty and concatenate exactly to replacement text",
        ));
    }
    let explicit_lines = final_lines
        .iter()
        .map(|text| ExplicitLayoutLine {
            logical_text: text.clone(),
            visual_text: text
                .trim_end_matches(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
                .to_string(),
            inserted_visual_hyphen: false,
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
/// visible end-of-line dictionary hyphen.  This is the same canonical Prompt
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
            "prompt20 explicit logical final lines must be nonempty and concatenate exactly to replacement text",
        ));
    }
    for line in final_lines {
        let visual_base = line
            .logical_text
            .trim_end_matches(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}']);
        let allowed_visual = if line.inserted_visual_hyphen {
            format!("{visual_base}-")
        } else {
            visual_base.to_string()
        };
        if line.visual_text != allowed_visual {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 visual final layout permits only trailing mandatory line separators and one end-of-line inserted dictionary hyphen"
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
/// canonical incremental writer are exactly the same Prompt 20 path as the
/// non-positioned variant. Positioning is intentionally horizontal-only until
/// the canonical vertical writer gains equivalent per-column geometry.
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
    if mode == AdvancedTextMode::ParagraphReflowVertical {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 positioned final layout does not yet support vertical columns".to_string(),
        ));
    }
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
            "prompt20 positioned final lines must be nonempty and concatenate exactly to replacement text",
        ));
    }
    for item in final_lines {
        let region = item.region;
        if region.iter().any(|value| !value.is_finite())
            || region[2] <= region[0]
            || region[3] <= region[1]
        {
            return Err(WellfriendError::invalid_input(
                "prompt20 positioned final layout contains an invalid line rectangle",
            ));
        }
        let visual_base = item
            .line
            .logical_text
            .trim_end_matches(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}']);
        let allowed_visual = if item.line.inserted_visual_hyphen {
            format!("{visual_base}-")
        } else {
            visual_base.to_string()
        };
        if item.line.visual_text != allowed_visual {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 positioned visual layout permits only trailing mandatory line separators and one end-of-line inserted dictionary hyphen"
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
    if !matches!(
        mode,
        AdvancedTextMode::ParagraphReflowHorizontal
            | AdvancedTextMode::ParagraphReflowRtl
            | AdvancedTextMode::ParagraphReflowVertical
    ) {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 serialized advanced text edit requires a paragraph reflow mode".to_string(),
        ));
    }
    let font = font_bytes
        .or_else(|| get_fallback_font("Symbol"))
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "prompt20 bundled shaping font unavailable".to_string(),
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
            "prompt20 selected font is missing glyphs for UTF-8 clusters {:?}",
            analysis.missing_glyph_clusters
        )));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::ContentEdit)?;
    enforce_prompt20_signature_policy(
        &signature_policy,
        options.signature_policy_override,
        "RTL/vertical paragraph reflow",
    )?;
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut matches = Vec::new();
    for (stream_index, (number, generation)) in page.contents.iter().copied().enumerate() {
        let stream = reader.get_object(number, generation)?;
        let decoded_result = decode_stream_lossless_with_limits(
            &stream,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded_result.status != StreamDecodeStatus::Complete {
            continue;
        }
        for token in scan_text_string_tokens(&decoded_result.data)? {
            let Some(font_dict) = resources.fonts.get(&token.font_name) else {
                continue;
            };
            let resolver = FontResolver::new(font_dict, reader);
            if resolver.decode_string(&token.decoded) == old_text {
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
            "prompt20 bounded reflow requires old text in exactly one PDF string token; found {} occurrences",
            matches.len()
        )));
    }
    let (_stream_index, source_number, source_generation, source_object, mut source_decoded, token) =
        matches.remove(0);
    let empty = serialize_pdf_string(&[], token.representation);
    source_decoded.splice(token.token_start..token.token_end, empty);
    let source_compressed = flate_encode(&source_decoded, 6);
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
        None => {
            let glyphs = generated_glyph_plan(new_text, mode, font)?;
            layout_generated_glyphs(&glyphs, mode, options)?
        }
    };
    let glyphs = layout.iter().flatten().cloned().collect::<Vec<_>>();
    let base_object = reader
        .object_ids()
        .into_iter()
        .map(|(number, _)| number)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let font_file_number = base_object;
    let descriptor_number = base_object + 1;
    let cid_to_gid_number = base_object + 2;
    let to_unicode_number = base_object + 3;
    let descendant_number = base_object + 4;
    let type0_number = base_object + 5;
    let content_number = base_object + 6;
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
        (mode == AdvancedTextMode::ParagraphReflowRtl && explicit_line_regions.is_some())
            .then_some(new_text),
    )?;
    let generated_compressed = flate_encode(generated_content.as_bytes(), 6);
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
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("prompt20 page object is not a dictionary".to_string())
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
    resource_dict.insert("Font", PdfObject::Dictionary(font_resources));
    page_dict.insert("Resources", PdfObject::Dictionary(resource_dict));
    let mut contents = page
        .contents
        .iter()
        .map(|(number, generation)| PdfObject::Reference {
            number: *number,
            generation: *generation,
        })
        .collect::<Vec<_>>();
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
    let output = write_incremental_update(reader, changed)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let extracted = reopened.get_page_text(page_number)?;
    let replacement_extracts = extracted.contains(new_text)
        || explicit_final_lines.is_some_and(|_| layout_extraction_equivalent(&extracted, new_text));
    // Positional-only edits (for example, Prompt 34 table-cell alignment)
    // intentionally preserve the logical text sequence. In that case the
    // old-token absence proof would be tautologically impossible, so retain
    // the independent source/incremental/output checks while marking the
    // textual proof satisfied by exact logical identity.
    let old_absent = old_text == new_text || !extracted.contains(old_text);
    if !replacement_extracts || !old_absent || !output.starts_with(input) {
        return Err(WellfriendError::MalformedPdf(format!(
            "prompt20 RTL/vertical edit failed proof: replacement_extracts={replacement_extracts}, old_text_absent={old_absent}, prefix_preserved={}",
            output.starts_with(input)
        )));
    }
    let before_fingerprint = format!("{:x}", Sha256::digest(input));
    let after_fingerprint = format!("{:x}", Sha256::digest(&output));
    let report = AdvancedTextEditReport {
        schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
        status: Prompt20SupportStatus::ImplementedWithLimits,
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
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut source_spans = Vec::new();
    let mut logical_text = String::new();
    let mut logical_offset = 0usize;
    for (stream_index, (number, generation)) in page.contents.iter().copied().enumerate() {
        let object = reader.get_object(number, generation)?;
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            continue;
        }
        for token in scan_text_string_tokens(&decoded.data)? {
            let Some(font) = resources.fonts.get(&token.font_name) else {
                continue;
            };
            let text = FontResolver::new(font, reader).decode_string(&token.decoded);
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
                font_resource: token.font_name,
                writing_mode: 0,
                marked_content_depth: token.marked_depth,
                text,
            });
        }
    }
    if source_spans.len() > MAX_PROMPT20_BIDI_RUNS {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20b range span count exceeds 4096".to_string(),
        ));
    }
    let analysis = analyze_advanced_text_reflow(
        &logical_text,
        AdvancedTextMode::ParagraphReflowRtl,
        None,
        TextReflowLimits::default(),
    )?;
    Ok(MultiRunRangeModel {
        schema_version: "prompt20b.multirun-form-appearance-closure.v1".to_string(),
        status: Prompt20SupportStatus::ImplementedWithLimits,
        page: page_number,
        paragraph_block_id: format!("page-{page_number}-logical-text"),
        logical_text,
        source_spans,
        logical_to_visual_runs: analysis.bidi_runs,
        writing_mode: 0,
        deterministic: true,
        exact_limits: vec![
            "logical offsets are Unicode scalar offsets mapped to decoded PDF string-token provenance; visual-quads require a unique caller-provided span target".to_string(),
            "selection must align to string-token boundaries and remain in one page content stream; partial-token, cross-stream, Form-owned, malformed-CMap, and Type3 selections fail closed".to_string(),
        ],
    })
}

/// Replace or delete a selection spanning multiple Tj/TJ/quote operands.  The
/// selected operands are removed from reachable content, while the new Unicode
/// is written as a deterministic Type0 run.  A zero-width boundary performs a
/// bounded insertion without removing an existing operand.
pub fn edit_multi_run_text_range(
    input: &[u8],
    request: &MultiRunTextRangeRequest,
    font_bytes: Option<&[u8]>,
) -> Result<(Vec<u8>, MultiRunTextEditReport)> {
    validate_advanced_text_options(&request.options)?;
    if !matches!(
        request.mode,
        AdvancedTextMode::ParagraphReflowHorizontal
            | AdvancedTextMode::ParagraphReflowRtl
            | AdvancedTextMode::ParagraphReflowVertical
    ) {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20b range edit requires a paragraph reflow mode".to_string(),
        ));
    }
    if request.logical_start > request.logical_end {
        return Err(WellfriendError::invalid_input(
            "prompt20b logical range start exceeds end",
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::ContentEdit)?;
    enforce_prompt20_signature_policy(
        &signature_policy,
        request.options.signature_policy_override,
        "multi-run text range edit",
    )?;
    let page = engine.document().get_page(request.page)?;
    let reader = engine.document().reader();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut selected = Vec::<SelectedMultiRunOperand>::new();
    let mut total = 0usize;
    let mut candidate_insertion: Option<(u32, u16, PdfObject, Vec<u8>)> = None;
    for (stream_index, (number, generation)) in page.contents.iter().copied().enumerate() {
        let object = reader.get_object(number, generation)?;
        let decoded_result = decode_stream_lossless_with_limits(
            &object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if decoded_result.status != StreamDecodeStatus::Complete {
            continue;
        }
        let decoded = decoded_result.data;
        let mut spans_here = Vec::new();
        for token in scan_text_string_tokens(&decoded)? {
            let Some(font) = resources.fonts.get(&token.font_name) else {
                continue;
            };
            let text = FontResolver::new(font, reader).decode_string(&token.decoded);
            let start = total;
            let end = start.saturating_add(text.chars().count());
            total = end;
            let span = MultiRunSourceSpan {
                span_id: format!("p{}:s{stream_index}:o{}", request.page, token.token_start),
                stream_object: number,
                stream_generation: generation,
                operator: token.operator.clone(),
                tj_element: token.element,
                byte_range: [token.token_start, token.token_end],
                logical_range: [start, end],
                font_resource: token.font_name.clone(),
                writing_mode: 0,
                marked_content_depth: token.marked_depth,
                text,
            };
            if request.logical_start == request.logical_end
                && (request.logical_start == start || request.logical_start == end)
            {
                candidate_insertion
                    .get_or_insert_with(|| (number, generation, object.clone(), decoded.clone()));
            }
            if request.logical_start < request.logical_end
                && start >= request.logical_start
                && end <= request.logical_end
            {
                spans_here.push((token, span));
            }
        }
        if !spans_here.is_empty() {
            for (token, span) in spans_here {
                selected.push((
                    number,
                    generation,
                    object.clone(),
                    decoded.clone(),
                    token,
                    span,
                ));
            }
        }
    }
    if request.logical_end > total {
        return Err(WellfriendError::invalid_input(format!(
            "prompt20b logical range {}..{} is outside page logical length {total}",
            request.logical_start, request.logical_end
        )));
    }
    if request.logical_start < request.logical_end {
        selected.sort_by_key(|item| (item.0, item.4.token_start));
        let first = selected.first().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "prompt20b range has no provenance-bearing source spans".to_string(),
            )
        })?;
        let last = selected.last().expect("nonempty");
        if first.5.logical_range[0] != request.logical_start
            || last.5.logical_range[1] != request.logical_end
            || selected
                .iter()
                .any(|item| item.0 != first.0 || item.1 != first.1)
        {
            return Err(WellfriendError::UnsupportedFeature("prompt20b selection must be contiguous token-boundary text in one content stream; cross-stream or partial-token range rejected".to_string()));
        }
    }
    let (source_number, source_generation, source_object, mut source_data) = if let Some(first) =
        selected.first()
    {
        (first.0, first.1, first.2.clone(), first.3.clone())
    } else {
        candidate_insertion.ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "prompt20b insertion must target a provenance-bearing token boundary".to_string(),
            )
        })?
    };
    let old_selected = selected
        .iter()
        .map(|item| item.5.text.as_str())
        .collect::<String>();
    if request.replacement_text.is_empty()
        && selected
            .iter()
            .any(|item| !item.4.marked_content.is_empty())
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20b deletion of marked-content text requires a structure-tree repair transaction and is refused before mutation"
                .to_string(),
        ));
    }
    let preserved_marked_content =
        if request.style_policy == MultiRunStylePolicy::PreservePerSegment {
            preserved_marked_content_wrapper(&selected, &source_data)?
        } else {
            None
        };
    for item in selected.iter().rev() {
        source_data.splice(
            item.4.token_start..item.4.token_end,
            serialize_pdf_string(&[], item.4.representation),
        );
    }
    if let Some(marked_content) = &preserved_marked_content {
        // The selected MCID moves to the generated stream.  Retag the now
        // empty source scope as an artifact so no page has two active
        // sequences claiming the same MCID.
        source_data.splice(
            marked_content.source_open_range[0]..marked_content.source_open_range[1],
            b"/Artifact BMC".iter().copied(),
        );
    }
    let PdfObject::Stream {
        dict: mut source_dict,
        ..
    } = source_object
    else {
        return Err(WellfriendError::MalformedPdf(
            "prompt20b range source is not a stream".to_string(),
        ));
    };
    let source_compressed = flate_encode(&source_data, 6);
    source_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    source_dict.remove("DecodeParms");
    source_dict.insert("Length", PdfObject::Integer(source_compressed.len() as i64));
    if request.replacement_text.is_empty() {
        let changed = vec![IncrementalObject {
            number: source_number,
            generation: source_generation,
            object: PdfObject::Stream {
                dict: source_dict,
                raw: source_compressed,
            },
        }];
        let output = write_incremental_update(reader, changed)?;
        let reopened = ContentEngine::open_bytes(output.clone())?;
        let extracted = reopened.get_page_text(request.page)?;
        let old_absent = old_selected.is_empty() || !extracted.contains(&old_selected);
        if !old_absent || !output.starts_with(input) {
            return Err(WellfriendError::MalformedPdf(
                "prompt20b multi-run delete save/reopen/extract proof failed".to_string(),
            ));
        }
        return Ok((output.clone(), MultiRunTextEditReport { schema_version:"prompt20b.multirun-form-appearance-closure.v1".to_string(), status:Prompt20SupportStatus::ImplementedWithLimits, operation:"delete".to_string(), page:request.page, logical_range:[request.logical_start,request.logical_end], selected_source_spans:selected.into_iter().map(|item| item.5).collect(), style_policy:request.style_policy, replacement_text:request.replacement_text.clone(), replacement_extracts:true, old_selected_text_absent:old_absent, unrelated_text_preserved:true, reachable_source_tokens_removed:true, output_reopened:true, original_prefix_preserved:output.starts_with(input), output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:request.options.deterministic, cache_invalidation:prompt20_cache_invalidation(input,&output,true,false,false), exact_limits:vec!["selected source spans must be contiguous token-boundary provenance in one page content stream; partial-token and cross-stream selections fail closed".to_string(),"delete removes selected provenance tokens and does not generate replacement glyph streams".to_string(),"logical/visual mapping uses bidi shaping provenance, never x-coordinate sorting; visual quad selection is accepted only after the caller resolves it to one unambiguous logical range".to_string()] }));
    }
    if request.style_policy == MultiRunStylePolicy::PreservePerSegment {
        if request.mode == AdvancedTextMode::ParagraphReflowVertical {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 preserve_per_segment supports horizontal source runs only; vertical source-run serialization is refused"
                .to_string(),
            ));
        }
        if request.mode == AdvancedTextMode::ParagraphReflowRtl
            || request
                .replacement_text
                .chars()
                .any(|character| matches!(character as u32, 0x0590..=0x08FF | 0xFB1D..=0xFEFF))
        {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 preserve_per_segment refuses RTL or mixed-bidi source runs until the canonical per-style serializer can retain final shaped visual ordering"
                    .to_string(),
            ));
        }
        if selected.is_empty() {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 preserve_per_segment insertion has no source style owner; use an explicit supplied or inherit style policy"
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
                    "prompt20 preserve_per_segment final lines must concatenate exactly to replacement text",
                ));
            }
            let mut line_scalar_cursor = 0usize;
            for line in lines {
                let visual_base = line
                    .logical_text
                    .trim_end_matches(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}']);
                if line.inserted_visual_hyphen || line.visual_text != visual_base {
                    return Err(WellfriendError::UnsupportedFeature(
                        "prompt20 preserve_per_segment supports only logical final lines without inserted visual hyphens"
                            .to_string(),
                    ));
                }
                line_scalar_cursor =
                    line_scalar_cursor.saturating_add(line.logical_text.chars().count());
                let scalar_end = line_scalar_cursor;
                let end_byte = scalar_boundary_byte(&request.replacement_text, scalar_end)
                    .ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "prompt20 preserve_per_segment cannot map final line boundary"
                                .to_string(),
                        )
                    })?;
                if !is_grapheme_boundary(&request.replacement_text, end_byte) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "prompt20 preserve_per_segment refuses a final line boundary that splits a grapheme or shaping cluster"
                            .to_string(),
                    ));
                }
            }
            lines
        } else {
            fallback_lines = vec![ExplicitLayoutLine {
                logical_text: request.replacement_text.clone(),
                visual_text: request.replacement_text.clone(),
                inserted_visual_hyphen: false,
            }];
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
        let mut source_styles_by_grapheme = Vec::<PreservedTextStyle>::new();
        let mut scalar_offset = 0usize;
        for item in &selected {
            let token = &item.4;
            let source_span = &item.5;
            let style = preserved_style_from_token(token)?;
            let span_scalars = source_span.text.chars().count();
            let source_boundary = scalar_boundary_byte(&old_selected, scalar_offset + span_scalars)
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "prompt20 preserve_per_segment cannot map source style boundary"
                            .to_string(),
                    )
                })?;
            if !is_grapheme_boundary(&old_selected, source_boundary) {
                return Err(WellfriendError::UnsupportedFeature(
                    "prompt20 preserve_per_segment refuses a source style boundary inside a grapheme or shaping cluster"
                        .to_string(),
                ));
            }
            source_styles_by_grapheme
                .extend(source_span.text.graphemes(true).map(|_| style.clone()));
            scalar_offset = scalar_offset.saturating_add(span_scalars);
        }
        if scalar_offset != old_selected.chars().count() || source_styles_by_grapheme.is_empty() {
            return Err(WellfriendError::MalformedPdf(
                "prompt20 preserve_per_segment source spans did not cover grapheme-safe source style ownership"
                    .to_string(),
            ));
        }
        let replacement_graphemes = request.replacement_text.graphemes(true).collect::<Vec<_>>();
        if replacement_graphemes.is_empty() {
            return Err(WellfriendError::MalformedPdf(
                "prompt20 preserve_per_segment non-delete replacement unexpectedly had no graphemes"
                    .to_string(),
            ));
        }
        let mut runs_by_scalar = Vec::<PreservedStyledRun>::new();
        for (replacement_grapheme_index, grapheme) in replacement_graphemes.iter().enumerate() {
            let source_grapheme_index = replacement_grapheme_index
                .saturating_mul(source_styles_by_grapheme.len())
                / replacement_graphemes.len();
            let style = source_styles_by_grapheme
                [source_grapheme_index.min(source_styles_by_grapheme.len().saturating_sub(1))]
            .clone();
            let Some(font_dict) = resources.fonts.get(&style.font_resource) else {
                return Err(WellfriendError::MalformedPdf(
                    "prompt20 preserve_per_segment source font resource disappeared".to_string(),
                ));
            };
            let resolver = FontResolver::new(font_dict, reader);
            for character in grapheme.chars() {
                let text = character.to_string();
                let (encoded, ambiguous) = encode_with_existing_font(&resolver, &text)?;
                if ambiguous {
                    return Err(WellfriendError::UnsupportedFeature(
                        "prompt20 preserve_per_segment refuses an ambiguous source CMap encoding"
                            .to_string(),
                    ));
                }
                let advance = preserved_run_advance(&resolver, &encoded, &text, &style);
                runs_by_scalar.push(PreservedStyledRun {
                    text,
                    encoded,
                    style: style.clone(),
                    advance,
                });
            }
        }
        let (generated_content, _line_adjustments) = serialize_preserved_styled_runs(
            &runs_by_scalar,
            logical_lines,
            &request.options,
            request.mode,
            preserved_marked_content
                .as_ref()
                .map(|marked_content| marked_content.opening.as_str()),
        )?;
        let content_number = reader
            .object_ids()
            .into_iter()
            .map(|(number, _)| number)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let generated = flate_encode(generated_content.as_bytes(), 6);
        let mut generated_dict = crate::PdfDictionary::empty();
        generated_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
        generated_dict.insert("Length", PdfObject::Integer(generated.len() as i64));
        let mut changed = vec![IncrementalObject {
            number: source_number,
            generation: source_generation,
            object: PdfObject::Stream {
                dict: source_dict,
                raw: source_compressed,
            },
        }];
        changed.push(IncrementalObject {
            number: content_number,
            generation: 0,
            object: PdfObject::Stream {
                dict: generated_dict,
                raw: generated,
            },
        });
        let page_object = reader.get_object(page.object_number, page.generation_number)?;
        let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "prompt20 preserve_per_segment page object is not a dictionary".to_string(),
            )
        })?;
        let mut contents = page
            .contents
            .iter()
            .map(|(number, generation)| PdfObject::Reference {
                number: *number,
                generation: *generation,
            })
            .collect::<Vec<_>>();
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
        let output = write_incremental_update(reader, changed)?;
        let reopened = ContentEngine::open_bytes(output.clone())?;
        let extracted = reopened.get_page_text(request.page)?;
        let replacement_extracts = extracted.contains(&request.replacement_text)
            || layout_extraction_equivalent(&extracted, &request.replacement_text);
        let old_absent = old_selected.is_empty() || !extracted.contains(&old_selected);
        if !replacement_extracts || !old_absent || !output.starts_with(input) {
            return Err(WellfriendError::MalformedPdf(
                "prompt20 preserve_per_segment save/reopen/extraction proof failed".to_string(),
            ));
        }
        return Ok((output.clone(), MultiRunTextEditReport {
            schema_version: "prompt20b.multirun-form-appearance-closure.v1".to_string(),
            status: Prompt20SupportStatus::ImplementedWithLimits,
            operation: "replace_preserving_per_segment_styles".to_string(),
            page: request.page,
            logical_range: [request.logical_start, request.logical_end],
            selected_source_spans: selected.iter().map(|item| item.5.clone()).collect(),
            style_policy: request.style_policy,
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
            cache_invalidation: prompt20_cache_invalidation(input, &output, true, false, false),
            exact_limits: vec![
                "preserve_per_segment supports one contiguous page content stream, exact source CMap encoding, and horizontal layout only; changed-length replacements assign each complete replacement grapheme to a deterministic proportional source-style owner without flattening styles or splitting a source grapheme".to_string(),
                "font resource, font size, character/word spacing, horizontal scaling, rise, render mode, and DeviceGray/RGB/CMYK paint state are replayed from each source text-showing operand".to_string(),
                "a single text-state-only MCID BDC containing exactly the selected source spans is moved atomically to the generated stream while the empty source wrapper is retagged Artifact; nested/partial/property-list ambiguity, inserted dictionary hyphens, RTL/vertical writing, and per-style full justification fail closed".to_string(),
            ],
        }));
    }
    let font = font_bytes
        .or_else(|| get_fallback_font("Symbol"))
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "prompt20b bundled shaping font unavailable".to_string(),
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
            "prompt20b replacement has missing glyph clusters in selected shaping font".to_string(),
        ));
    }
    let layout = if let Some(lines) = request.final_lines.as_deref() {
        if lines.is_empty()
            || lines
                .iter()
                .map(|line| line.logical_text.as_str())
                .collect::<String>()
                != request.replacement_text
        {
            return Err(WellfriendError::invalid_input(
                "prompt20b explicit final lines must concatenate exactly to replacement text",
            ));
        }
        for line in lines {
            let visual_base = line
                .logical_text
                .trim_end_matches(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}']);
            let allowed_visual = if line.inserted_visual_hyphen {
                format!("{visual_base}-")
            } else {
                visual_base.to_string()
            };
            if line.visual_text != allowed_visual {
                return Err(WellfriendError::UnsupportedFeature(
                    "prompt20b visual final layout permits only trailing mandatory separators and one end-of-line dictionary hyphen"
                        .to_string(),
                ));
            }
        }
        layout_generated_explicit_lines(lines, request.mode, font, &request.options, None)?
    } else {
        let glyphs = generated_glyph_plan(&request.replacement_text, request.mode, font)?;
        layout_generated_glyphs(&glyphs, request.mode, &request.options)?
    };
    let glyphs = layout.iter().flatten().cloned().collect::<Vec<_>>();
    let base = reader
        .object_ids()
        .into_iter()
        .map(|(n, _)| n)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let font_resource = deterministic_font_resource_name(reader, &page.resources);
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
    let generated = flate_encode(generated_content.as_bytes(), 6);
    let mut generated_dict = crate::PdfDictionary::empty();
    generated_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    generated_dict.insert("Length", PdfObject::Integer(generated.len() as i64));
    changed.push(IncrementalObject {
        number: base + 6,
        generation: 0,
        object: PdfObject::Stream {
            dict: generated_dict,
            raw: generated,
        },
    });
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("prompt20b page object is not a dictionary".to_string())
    })?;
    let mut page_resources = page.resources.clone();
    let mut fonts = resolve_prompt20_dict(page_resources.get("Font"), reader)
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
    let mut contents = page
        .contents
        .iter()
        .map(|(n, g)| PdfObject::Reference {
            number: *n,
            generation: *g,
        })
        .collect::<Vec<_>>();
    contents.push(PdfObject::Reference {
        number: base + 6,
        generation: 0,
    });
    page_dict.insert("Contents", PdfObject::Array(contents));
    changed.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: PdfObject::Dictionary(page_dict),
    });
    let output = write_incremental_update(reader, changed)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let extracted = reopened.get_page_text(request.page)?;
    let replacement_extracts = request.replacement_text.is_empty()
        || extracted.contains(&request.replacement_text)
        || request
            .final_lines
            .as_ref()
            .is_some_and(|_| layout_extraction_equivalent(&extracted, &request.replacement_text));
    let old_absent = old_selected.is_empty() || !extracted.contains(&old_selected);
    if !replacement_extracts || !old_absent || !output.starts_with(input) {
        return Err(WellfriendError::MalformedPdf(
            "prompt20b multi-run save/reopen/extract proof failed".to_string(),
        ));
    }
    Ok((output.clone(), MultiRunTextEditReport { schema_version:"prompt20b.multirun-form-appearance-closure.v1".to_string(), status:Prompt20SupportStatus::ImplementedWithLimits, operation:if request.replacement_text.is_empty(){"delete".to_string()} else if old_selected.is_empty(){"insert".to_string()} else {"replace".to_string()}, page:request.page, logical_range:[request.logical_start,request.logical_end], selected_source_spans:selected.into_iter().map(|item| item.5).collect(), style_policy:request.style_policy, replacement_text:request.replacement_text.clone(), replacement_extracts, old_selected_text_absent:old_absent, unrelated_text_preserved:true, reachable_source_tokens_removed:true, output_reopened:true, original_prefix_preserved:output.starts_with(input), output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:request.options.deterministic, cache_invalidation:prompt20_cache_invalidation(input,&output,true,false,false), exact_limits:vec!["selected source spans must be contiguous token-boundary provenance in one page content stream; partial-token and cross-stream selections fail closed".to_string(),"replacement is normalized into a deterministic generated Type0 run; preserve_per_segment requires a future per-style generated-run serializer".to_string(),"logical/visual mapping uses bidi shaping provenance, never x-coordinate sorting; visual quad selection is accepted only after the caller resolves it to one unambiguous logical range".to_string()] }))
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
        let Some(ap) = resolve_prompt20_dict(annotation_dict.get("AP"), reader) else {
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
            if let Some(states) = resolve_prompt20_dict(Some(appearance), reader) {
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
                max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
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
            resolve_prompt20_dict(dict.get("Resources"), reader).unwrap_or_default();
        collect_form_vector_objects(
            reader,
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
        .chain([options.font_size, options.line_spacing].iter())
        .any(|value| !value.is_finite())
        || options.region[0] >= options.region[2]
        || options.region[1] >= options.region[3]
        || options.font_size <= 0.0
        || options.line_spacing <= 0.0
    {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 text region/font size/line spacing must be finite and positive".to_string(),
        ));
    }
    if options.max_lines_or_columns == 0 || options.max_lines_or_columns > 10_000 {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 line/column limit must be in 1..=10000".to_string(),
        ));
    }
    Ok(())
}

fn generated_glyph_plan(
    text: &str,
    mode: AdvancedTextMode,
    font: &[u8],
) -> Result<Vec<GeneratedGlyph>> {
    let base = if mode == AdvancedTextMode::ParagraphReflowRtl {
        Level::rtl()
    } else {
        Level::ltr()
    };
    let bidi = BidiInfo::new(text, Some(base));
    let mut glyphs = Vec::new();
    for paragraph in &bidi.paragraphs {
        let (levels, ranges) = bidi.visual_runs(paragraph, paragraph.range.clone());
        for (level, range) in levels.into_iter().zip(ranges) {
            let run_text = text.get(range).ok_or_else(|| {
                WellfriendError::ParseError("prompt20 bidi run is not UTF-8 aligned".to_string())
            })?;
            let shaped = TextShaper::shape(
                font,
                run_text,
                ShapeOptions {
                    direction: Some(if level.is_rtl() {
                        TextDirection::RightToLeft
                    } else {
                        TextDirection::LeftToRight
                    }),
                },
            )?;
            let mut cluster_starts = shaped
                .glyphs
                .iter()
                .map(|glyph| glyph.cluster as usize)
                .collect::<Vec<_>>();
            cluster_starts.push(run_text.len());
            cluster_starts.sort_unstable();
            cluster_starts.dedup();
            for shaped_glyph in shaped.glyphs {
                let start = shaped_glyph.cluster as usize;
                let end = cluster_starts
                    .iter()
                    .copied()
                    .find(|cluster| *cluster > start)
                    .unwrap_or(run_text.len());
                let unicode = run_text.get(start..end).unwrap_or("\u{FFFD}").to_string();
                let orientation = if mode == AdvancedTextMode::ParagraphReflowVertical {
                    unicode
                        .chars()
                        .next()
                        .map(vertical_orientation)
                        .unwrap_or(VerticalGlyphOrientation::Upright)
                } else {
                    VerticalGlyphOrientation::Upright
                };
                let cid = u16::try_from(glyphs.len() + 1).map_err(|_| {
                    WellfriendError::UnsupportedFeature(
                        "prompt20 generated CID count exceeds 65535".to_string(),
                    )
                })?;
                glyphs.push(GeneratedGlyph {
                    cid,
                    gid: shaped_glyph.glyph_id,
                    visual_unicode: unicode.clone(),
                    to_unicode: Some(unicode),
                    advance: shaped_glyph.advance,
                    orientation,
                });
            }
        }
    }
    Ok(glyphs)
}

fn layout_generated_glyphs(
    glyphs: &[GeneratedGlyph],
    mode: AdvancedTextMode,
    options: &AdvancedTextEditOptions,
) -> Result<Vec<Vec<GeneratedGlyph>>> {
    let width_1000 = (options.region[2] - options.region[0]) / options.font_size * 1000.0;
    let height_1000 = (options.region[3] - options.region[1]) / options.font_size * 1000.0;
    let available = if mode == AdvancedTextMode::ParagraphReflowVertical {
        height_1000
    } else {
        width_1000
    };
    let mut groups = vec![Vec::new()];
    let mut advance = 0.0;
    for glyph in glyphs {
        let glyph_advance = if mode == AdvancedTextMode::ParagraphReflowVertical {
            1000.0
        } else {
            glyph.advance.abs()
        };
        if advance + glyph_advance > available && !groups.last().is_some_and(Vec::is_empty) {
            if groups.len() >= options.max_lines_or_columns {
                match options.overflow_policy {
                    TextOverflowPolicy::Error => {
                        return Err(WellfriendError::UnsupportedFeature(format!(
                            "prompt20 text overflow exceeds {} lines/columns",
                            options.max_lines_or_columns
                        )))
                    }
                    TextOverflowPolicy::Clip => break,
                    TextOverflowPolicy::ExpandRegion => {}
                }
            }
            groups.push(Vec::new());
            advance = 0.0;
        }
        groups.last_mut().expect("group exists").push(glyph.clone());
        advance += glyph_advance;
    }
    if groups.len() > options.max_lines_or_columns {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 expanded text still exceeds hard line/column limit".to_string(),
        ));
    }
    Ok(groups)
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
            "prompt20 positioned final layout has mismatched line regions",
        ));
    }
    if line_regions.is_none() && lines.len() > options.max_lines_or_columns {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 explicit final layout has {} lines/columns; limit is {}",
            lines.len(),
            options.max_lines_or_columns
        )));
    }
    let mut next_cid = 1u16;
    let mut layout = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let region = line_regions
            .and_then(|regions| regions.get(index).copied())
            .unwrap_or(options.region);
        let available = if mode == AdvancedTextMode::ParagraphReflowVertical {
            (region[3] - region[1]) / options.font_size * 1000.0
        } else {
            (region[2] - region[0]) / options.font_size * 1000.0
        };
        let mut glyphs = generated_glyph_plan(&line.visual_text, mode, font)?;
        let advance = glyphs
            .iter()
            .map(|glyph| {
                if mode == AdvancedTextMode::ParagraphReflowVertical {
                    1000.0
                } else {
                    glyph.advance.abs()
                }
            })
            .sum::<f64>();
        if advance > available + EPSILON {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 explicit final layout line exceeds its source region".to_string(),
            ));
        }
        for glyph in &mut glyphs {
            glyph.cid = next_cid;
            next_cid = next_cid.checked_add(1).ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "prompt20 explicit final layout CID count exceeds 65535".to_string(),
                )
            })?;
        }
        if line.inserted_visual_hyphen {
            let Some(last) = glyphs.last_mut() else {
                return Err(WellfriendError::MalformedPdf(
                    "prompt20 inserted dictionary hyphen line has no generated glyph".to_string(),
                ));
            };
            if last.visual_unicode != "-" {
                return Err(WellfriendError::MalformedPdf(
                    "prompt20 inserted dictionary hyphen must be the final generated glyph"
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
    let face = ttf_parser::Face::parse(font, 0).map_err(|_| {
        WellfriendError::UnsupportedFeature("prompt20 cannot embed malformed sfnt font".to_string())
    })?;
    let upem = f64::from(face.units_per_em()).max(1.0);
    let units = |value: i16| canonical_number(f64::from(value) / upem * 1000.0);
    let bbox = face.global_bounding_box();
    let compressed_font = flate_encode(font, 6);
    let mut font_file_dict = crate::PdfDictionary::empty();
    font_file_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    font_file_dict.insert("Length", PdfObject::Integer(compressed_font.len() as i64));
    font_file_dict.insert("Length1", PdfObject::Integer(font.len() as i64));

    let mut descriptor = crate::PdfDictionary::empty();
    descriptor.insert("Type", PdfObject::Name("FontDescriptor".to_string()));
    descriptor.insert(
        "FontName",
        PdfObject::Name("WellfriendPrompt20Unicode".to_string()),
    );
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
        "FontFile2",
        PdfObject::Reference {
            number: font_file_number,
            generation: 0,
        },
    );

    let mut cid_to_gid = vec![0u8; (glyphs.len() + 1) * 2];
    for glyph in glyphs {
        let offset = usize::from(glyph.cid) * 2;
        cid_to_gid[offset] = (glyph.gid >> 8) as u8;
        cid_to_gid[offset + 1] = (glyph.gid & 0xff) as u8;
    }
    let compressed_map = flate_encode(&cid_to_gid, 6);
    let mut map_dict = crate::PdfDictionary::empty();
    map_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    map_dict.insert("Length", PdfObject::Integer(compressed_map.len() as i64));

    let to_unicode = build_to_unicode_cmap(glyphs);
    let compressed_to_unicode = flate_encode(to_unicode.as_bytes(), 6);
    let mut to_unicode_dict = crate::PdfDictionary::empty();
    to_unicode_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    to_unicode_dict.insert(
        "Length",
        PdfObject::Integer(compressed_to_unicode.len() as i64),
    );

    let mut cid_system = crate::PdfDictionary::empty();
    cid_system.insert("Registry", PdfObject::String(b"Adobe".to_vec()));
    cid_system.insert("Ordering", PdfObject::String(b"Identity".to_vec()));
    cid_system.insert("Supplement", PdfObject::Integer(0));
    let widths = glyphs
        .iter()
        .map(|glyph| PdfObject::Real(canonical_number(glyph.advance.abs().max(1.0))))
        .collect::<Vec<_>>();
    let mut descendant = crate::PdfDictionary::empty();
    descendant.insert("Type", PdfObject::Name("Font".to_string()));
    descendant.insert("Subtype", PdfObject::Name("CIDFontType2".to_string()));
    descendant.insert(
        "BaseFont",
        PdfObject::Name("WellfriendPrompt20Unicode".to_string()),
    );
    descendant.insert("CIDSystemInfo", PdfObject::Dictionary(cid_system));
    descendant.insert(
        "FontDescriptor",
        PdfObject::Reference {
            number: descriptor_number,
            generation: 0,
        },
    );
    descendant.insert("DW", PdfObject::Integer(1000));
    descendant.insert(
        "W",
        PdfObject::Array(vec![PdfObject::Integer(1), PdfObject::Array(widths)]),
    );
    if vertical {
        descendant.insert(
            "DW2",
            PdfObject::Array(vec![PdfObject::Integer(880), PdfObject::Integer(-1000)]),
        );
    }
    descendant.insert(
        "CIDToGIDMap",
        PdfObject::Reference {
            number: cid_to_gid_number,
            generation: 0,
        },
    );

    let mut type0 = crate::PdfDictionary::empty();
    type0.insert("Type", PdfObject::Name("Font".to_string()));
    type0.insert("Subtype", PdfObject::Name("Type0".to_string()));
    type0.insert(
        "BaseFont",
        PdfObject::Name("WellfriendPrompt20Unicode".to_string()),
    );
    type0.insert(
        "Encoding",
        PdfObject::Name(if vertical { "Identity-V" } else { "Identity-H" }.to_string()),
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

    Ok(vec![
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
            number: cid_to_gid_number,
            generation: 0,
            object: PdfObject::Stream {
                dict: map_dict,
                raw: compressed_map,
            },
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
    ])
}

fn build_to_unicode_cmap(glyphs: &[GeneratedGlyph]) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /WellfriendPrompt20ToUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for chunk in glyphs.chunks(100) {
        let mapped = chunk
            .iter()
            .filter(|glyph| glyph.to_unicode.is_some())
            .collect::<Vec<_>>();
        if mapped.is_empty() {
            continue;
        }
        cmap.push_str(&format!("{} beginbfchar\n", mapped.len()));
        for glyph in mapped {
            cmap.push_str(&format!(
                "<{:04X}> <{}>\n",
                glyph.cid,
                utf16be_hex(glyph.to_unicode.as_deref().expect("filtered Some"))
            ));
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
            "prompt20 positioned serializer has mismatched line regions",
        ));
    }
    if vertical {
        if line_regions.is_some() {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 positioned serializer does not support vertical columns".to_string(),
            ));
        }
        if options.alignment == GeneratedTextAlignment::Justify {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 vertical full justification is unsupported until the canonical writer can emit vertical text-state spacing without changing glyph orientation"
                    .to_string(),
            ));
        }
        let column_advance = options.font_size * options.line_spacing;
        for (column, glyphs) in layout.iter().enumerate() {
            let x = options.region[2] - options.font_size - column as f64 * column_advance;
            let mut y = options.region[3] - options.font_size;
            for glyph in glyphs {
                match glyph.orientation {
                    VerticalGlyphOrientation::RotateClockwise => {
                        content.push_str(&format!(
                            "0 -1 1 0 {} {} Tm <{:04X}> Tj\n",
                            fmt_num(x),
                            fmt_num(y),
                            glyph.cid
                        ));
                    }
                    _ => {
                        content.push_str(&format!(
                            "1 0 0 1 {} {} Tm <{:04X}> Tj\n",
                            fmt_num(x),
                            fmt_num(y),
                            glyph.cid
                        ));
                    }
                }
                y -= options.font_size;
            }
            adjustments.push(GeneratedLineAdjustment {
                line_index: column,
                natural_width: glyphs.len() as f64 * options.font_size,
                target_width: options.region[3] - options.region[1],
                residual: 0.0,
                word_spacing: 0.0,
                character_spacing: 0.0,
                alignment: options.alignment,
                last_line: column + 1 == layout.len(),
                applied: true,
                refusal_reason: None,
            });
        }
    } else {
        let line_advance = options.font_size * options.line_spacing;
        for (line, glyphs) in layout.iter().enumerate() {
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
                        "prompt20 full justification exceeds configured text-state spacing bounds"
                            .to_string(),
                    ));
                }
            }
            let x = match options.alignment {
                GeneratedTextAlignment::Left => region[0],
                GeneratedTextAlignment::Right => region[2] - line_width,
                GeneratedTextAlignment::Center => region[0] + (target_width - line_width) / 2.0,
                GeneratedTextAlignment::Start => {
                    if rtl {
                        region[2] - line_width
                    } else {
                        region[0]
                    }
                }
                GeneratedTextAlignment::End => {
                    if rtl {
                        region[0]
                    } else {
                        region[2] - line_width
                    }
                }
                GeneratedTextAlignment::Justify => {
                    if rtl {
                        region[2] - line_width
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
            content.push_str(&format!("1 0 0 1 {} {} Tm <", fmt_num(x), fmt_num(y)));
            for glyph in glyphs {
                content.push_str(&format!("{:04X}", glyph.cid));
            }
            content.push_str("> Tj\n");
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
                "prompt20 text contains NUL at UTF-8 byte {offset}"
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
                        "prompt20 unmatched bidi pop control at UTF-8 byte {offset}"
                    )));
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(WellfriendError::MalformedPdf(format!(
            "prompt20 text ends with {depth} unclosed bidi control sequence(s)"
        )));
    }
    Ok(())
}

fn vertical_orientation(ch: char) -> VerticalGlyphOrientation {
    let code = ch as u32;
    if ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ':' | ';'
        )
    {
        VerticalGlyphOrientation::RotateClockwise
    } else if matches!(
        code,
        0x3001..=0x303F | 0xFE10..=0xFE1F | 0xFE30..=0xFE4F | 0xFF01..=0xFF60
    ) {
        VerticalGlyphOrientation::FontVerticalAlternate
    } else {
        VerticalGlyphOrientation::Upright
    }
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
}

impl Default for SameWidthPatchOptions {
    fn default() -> Self {
        Self {
            mode: SameWidthMode::Exact,
            advance_tolerance_1000: 0.0,
            signature_policy_override: false,
            require_same_serialized_length: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SameWidthPatchEligibility {
    pub schema_version: String,
    pub status: Prompt20SupportStatus,
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
    /// Exact open-marked-content operators active at this source operand.  The
    /// bounded multi-run serializer uses this only after proving that a single
    /// MCID-bearing BDC contains precisely the selected text-state sequence;
    /// it never guesses a tag or property list from geometry.
    marked_content: Vec<MarkedContentFrame>,
}

type SelectedMultiRunOperand = (
    u32,
    u16,
    PdfObject,
    Vec<u8>,
    ContentStringToken,
    MultiRunSourceSpan,
);

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkedContentFrame {
    open_start: usize,
    open_end: usize,
    open_operator: String,
    open_bytes: Vec<u8>,
    has_mcid: bool,
}

/// The source text-state facts needed to replay a whole provenance-bearing
/// operand as a styled generated run.  This deliberately remains private to
/// the canonical Prompt 20 content mutator: Prompt 33 consumes the public
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
}

#[derive(Debug, Clone)]
enum LexicalKind {
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
struct LexicalToken {
    start: usize,
    end: usize,
    kind: LexicalKind,
}

pub fn analyze_same_width_patch(
    input: &[u8],
    page_number: usize,
    source_text: &str,
    replacement_text: &str,
    options: &SameWidthPatchOptions,
) -> Result<SameWidthPatchEligibilityReport> {
    validate_patch_options(options)?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::ContentEdit)?;
    let document = engine.document();
    let page = document.get_page(page_number)?;
    let resources = PageResources::from_dict(&page.resources, document.reader());
    let reader = document.reader();
    let mut candidates = Vec::new();
    for (stream_number, stream_generation) in page.contents.iter().copied() {
        let object = reader.get_object(stream_number, stream_generation)?;
        let PdfObject::Stream { dict, raw } = object else {
            continue;
        };
        let stream = PdfObject::Stream {
            dict: dict.clone(),
            raw: raw.clone(),
        };
        let decoded_result = decode_stream_lossless_with_limits(
            &stream,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        let decoded = match decoded_result.status {
            StreamDecodeStatus::Complete => decoded_result.data,
            StreamDecodeStatus::StoppedAtImageFilter(reason) => {
                candidates.push(unsupported_patch_candidate(
                    page_number,
                    stream_number,
                    stream_generation,
                    filter_names(&dict),
                    format!("content stream stopped at image filter: {reason}"),
                ));
                continue;
            }
        };
        for token in scan_text_string_tokens(&decoded)? {
            let Some(font_dict) = resources.fonts.get(&token.font_name) else {
                continue;
            };
            let resolver = FontResolver::new(font_dict, reader);
            if resolver.decode_string(&token.decoded) != source_text {
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
            ));
        }
    }
    candidates.sort_by_key(|candidate| {
        (
            candidate.stream_object,
            candidate.stream_generation,
            candidate.decoded_byte_start,
        )
    });
    Ok(SameWidthPatchEligibilityReport {
        schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
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
    enforce_prompt20_signature_policy(
        &analysis.signature_policy,
        options.signature_policy_override,
        "same-width content-stream patch",
    )?;
    let selected = analysis
        .candidates
        .iter()
        .find(|candidate| candidate.eligible)
        .cloned()
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "prompt20 same-width patch has no eligible occurrence: {}",
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
                "prompt20 selected patch font resource disappeared".to_string(),
            )
        })?;
    let resolver = FontResolver::new(font_dict, reader);
    let encoded = encode_with_existing_font(&resolver, replacement_text)?.0;
    let replacement_token = serialize_pdf_string(&encoded, selected.representation);
    let object = reader.get_object(selected.stream_object, selected.stream_generation)?;
    let PdfObject::Stream { mut dict, raw } = object else {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 selected content object is no longer a stream".to_string(),
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
            max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    let mut decoded = match decoded_result.status {
        StreamDecodeStatus::Complete => decoded_result.data,
        StreamDecodeStatus::StoppedAtImageFilter(reason) => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt20 selected stream became undecodable: {reason}"
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
            "prompt20 selected serialized replacement length changed after eligibility analysis"
                .to_string(),
        ));
    }
    decoded.splice(
        selected.decoded_byte_start..selected.decoded_byte_end,
        replacement_token,
    );
    let compressed = flate_encode(&decoded, 6);
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
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
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let extracted = reopened.get_page_text(page_number)?;
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    let report = SameWidthPatchApplyReport {
        schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
        selected,
        original_bytes: input.len(),
        output_bytes: output.len(),
        rewritten_stream_bytes: decoded.len(),
        appended_revision_bytes: output.len().saturating_sub(input.len()),
        original_prefix_preserved: output.starts_with(input),
        output_reopened: true,
        replacement_extracts: extracted.contains(replacement_text),
        old_text_absent: !extracted.contains(source_text),
        output_sha256,
        signature_policy: analysis.signature_policy,
        cryptographic_validity_claimed: false,
        deterministic: true,
        cache_invalidation: prompt20_cache_invalidation(input, &output, true, false, false),
    };
    if !report.original_prefix_preserved || !report.replacement_extracts || !report.old_text_absent
    {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 same-width patch failed reopen/extraction/prefix verification".to_string(),
        ));
    }
    Ok((output, report))
}

fn validate_patch_options(options: &SameWidthPatchOptions) -> Result<()> {
    if !options.advance_tolerance_1000.is_finite() || options.advance_tolerance_1000 < 0.0 {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 patch advance tolerance must be finite and non-negative".to_string(),
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
) -> SameWidthPatchEligibility {
    let encoded = encode_with_existing_font(resolver, replacement);
    let (replacement_bytes, ambiguous) = encoded
        .as_ref()
        .map(|(bytes, ambiguous)| (bytes.clone(), *ambiguous))
        .unwrap_or_default();
    let before_codes = split_codes(&token.decoded, resolver.code_size());
    let after_codes = split_codes(&replacement_bytes, resolver.code_size());
    let before_advances = before_codes
        .iter()
        .map(|code| canonical_number(resolver.glyph_width(*code)))
        .collect::<Vec<_>>();
    let after_advances = after_codes
        .iter()
        .map(|code| canonical_number(resolver.glyph_width(*code)))
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
    if encrypted {
        reject(
            &mut eligible,
            &mut reason,
            "encrypted incremental object replacement is unsupported",
        );
    }
    SameWidthPatchEligibility {
        schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
        status: if eligible {
            Prompt20SupportStatus::ImplementedWithLimits
        } else {
            Prompt20SupportStatus::UnsupportedReportedExact
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

fn unsupported_patch_candidate(
    page: usize,
    stream_object: u32,
    stream_generation: u16,
    filters: Vec<String>,
    reason: String,
) -> SameWidthPatchEligibility {
    SameWidthPatchEligibility {
        schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
        status: Prompt20SupportStatus::UnsupportedReportedExact,
        eligible: false,
        page,
        stream_object,
        stream_generation,
        operator: "unknown".to_string(),
        tj_element: None,
        decoded_byte_start: 0,
        decoded_byte_end: 0,
        representation: PatchStringRepresentation::Literal,
        font_resource: String::new(),
        font_type: "unknown".to_string(),
        encoding: "unknown".to_string(),
        cmap: "unknown".to_string(),
        glyph_count_before: 0,
        glyph_count_after: 0,
        encoded_bytes_before: 0,
        encoded_bytes_after: 0,
        serialized_bytes_before: 0,
        serialized_bytes_after: 0,
        glyph_advances_before: Vec::new(),
        glyph_advances_after: Vec::new(),
        total_advance_before: 0.0,
        total_advance_after: 0.0,
        advance_delta: 0.0,
        writing_mode: 0,
        text_render_mode: 0,
        marked_content_depth: 0,
        clipping_semantics: false,
        encrypted: false,
        filters,
        incremental_feasible: false,
        full_rewrite_feasible: false,
        exact_reason: reason,
    }
}

fn encode_with_existing_font(resolver: &FontResolver, text: &str) -> Result<(Vec<u8>, bool)> {
    let code_size = resolver.code_size().max(1);
    let max_code = if code_size == 1 { 255u32 } else { 65_535u32 };
    let mut output = Vec::new();
    let mut ambiguous = false;
    for ch in text.chars() {
        let wanted = ch.to_string();
        let mut found = None;
        for code in 0..=max_code {
            if resolver.decode_char(code as u16) == wanted {
                if found.is_some() {
                    ambiguous = true;
                    break;
                }
                found = Some(code as u16);
            }
        }
        let code = found.ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "prompt20 existing font/CMap cannot encode U+{:04X}",
                ch as u32
            ))
        })?;
        if code_size == 2 {
            output.push((code >> 8) as u8);
        }
        output.push((code & 0xff) as u8);
    }
    Ok((output, ambiguous))
}

fn split_codes(bytes: &[u8], code_size: u8) -> Vec<u16> {
    if code_size == 2 {
        bytes
            .chunks(2)
            .map(|chunk| (u16::from(chunk[0]) << 8) | u16::from(*chunk.get(1).unwrap_or(&0)))
            .collect()
    } else {
        bytes.iter().copied().map(u16::from).collect()
    }
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

fn scan_text_string_tokens(data: &[u8]) -> Result<Vec<ContentStringToken>> {
    let tokens = lex_content(data)?;
    let mut output = Vec::new();
    let mut operands = Vec::<LexicalToken>::new();
    let mut font_name = String::new();
    let mut font_size = 0.0_f64;
    let mut render_mode = 0i32;
    let mut character_spacing = 0.0_f64;
    let mut word_spacing = 0.0_f64;
    let mut horizontal_scaling = 100.0_f64;
    let mut text_rise = 0.0_f64;
    let mut fill_color_command = "0 g".to_string();
    let mut stroke_color_command = "0 G".to_string();
    let mut unsupported_fill_paint_state = false;
    let mut unsupported_stroke_paint_state = false;
    let mut marked_depth = 0usize;
    let mut marked_content = Vec::<MarkedContentFrame>::new();
    for token in tokens {
        let LexicalKind::Word(operator) = &token.kind else {
            operands.push(token);
            continue;
        };
        match operator.as_str() {
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
                    unsupported_stroke_paint_state = false;
                }
            }
            "cs" | "sc" | "scn" => {
                // DeviceGray/RGB/CMYK device operators above are replayed
                // exactly. Other color-space/pattern/shading operations need
                // their resource identity and component semantics preserved,
                // so the bounded style serializer refuses them rather than
                // silently emitting its default DeviceGray state.
                unsupported_fill_paint_state = true;
            }
            "CS" | "SC" | "SCN" => {
                unsupported_stroke_paint_state = true;
            }
            "BMC" | "BDC" => {
                let open_start = operands
                    .first()
                    .map(|operand| operand.start)
                    .unwrap_or(token.start);
                let open_bytes = data
                    .get(open_start..token.end)
                    .ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "prompt20 marked-content opener is outside its decoded stream"
                                .to_string(),
                        )
                    })?
                    .to_vec();
                let has_mcid = operator == "BDC"
                    && open_bytes
                        .windows(b"MCID".len())
                        .any(|window| window == b"MCID");
                marked_content.push(MarkedContentFrame {
                    open_start,
                    open_end: token.end,
                    open_operator: operator.clone(),
                    open_bytes,
                    has_mcid,
                });
                marked_depth = marked_content.len();
            }
            "EMC" => {
                marked_content.pop();
                marked_depth = marked_content.len();
            }
            "Tj" | "'" | "\"" => {
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
                    output.push(ContentStringToken {
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
                        text_render_mode: render_mode,
                        marked_depth,
                        marked_content: marked_content.clone(),
                    });
                }
            }
            "TJ" => {
                let mut element = 0usize;
                for operand in &operands {
                    if let LexicalKind::String(representation, decoded) = &operand.kind {
                        output.push(ContentStringToken {
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
                            text_render_mode: render_mode,
                            marked_depth,
                            marked_content: marked_content.clone(),
                        });
                        element += 1;
                    }
                }
            }
            _ => {}
        }
        operands.clear();
    }
    Ok(output)
}

#[derive(Debug, Clone)]
struct PreservedStyledRun {
    text: String,
    encoded: Vec<u8>,
    style: PreservedTextStyle,
    advance: f64,
}

/// A single, exact BDC wrapper that can move with a generated preserved-style
/// run.  The original BDC is converted to an empty artifact BMC before the
/// generated stream receives the original raw BDC bytes, keeping the page's
/// MCID unique rather than duplicating a tagged-content identifier.
#[derive(Debug, Clone)]
struct PreservedMarkedContent {
    opening: String,
    source_open_range: [usize; 2],
}

fn preserved_marked_content_wrapper(
    selected: &[SelectedMultiRunOperand],
    source_data: &[u8],
) -> Result<Option<PreservedMarkedContent>> {
    let any_marked = selected
        .iter()
        .any(|item| !item.4.marked_content.is_empty());
    if !any_marked {
        return Ok(None);
    }
    if selected.iter().any(|item| item.4.marked_content.len() != 1) {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 preserve_per_segment supports only one shared, non-nested MCID BDC wrapper; mixed, nested, or partial marked-content selections refuse"
                .to_string(),
        ));
    }
    let frame = selected
        .first()
        .and_then(|item| item.4.marked_content.first())
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "prompt20 preserve_per_segment lost selected marked-content provenance".to_string(),
            )
        })?;
    if frame.open_operator != "BDC" || !frame.has_mcid {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 preserve_per_segment requires an exact MCID-bearing BDC wrapper; untagged BMC/property-list cases remain refused"
                .to_string(),
        ));
    }
    if selected
        .iter()
        .any(|item| item.4.marked_content.first() != Some(frame))
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 preserve_per_segment refuses a selection crossing distinct marked-content identities"
                .to_string(),
        ));
    }
    let selected_ranges = selected
        .iter()
        .map(|item| [item.4.token_start, item.4.token_end])
        .collect::<BTreeSet<_>>();
    for token in scan_text_string_tokens(source_data)? {
        if token.marked_content.first() == Some(frame)
            && !selected_ranges.contains(&[token.token_start, token.token_end])
        {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 preserve_per_segment refuses an MCID BDC containing unselected text; tag ownership would become partial"
                    .to_string(),
            ));
        }
    }
    if !mcid_frame_contains_text_state_only(source_data, frame)? {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 preserve_per_segment refuses an MCID BDC containing non-text-state painting, nesting, or an unterminated scope"
                .to_string(),
        ));
    }
    let opening = String::from_utf8(frame.open_bytes.clone()).map_err(|_| {
        WellfriendError::UnsupportedFeature(
            "prompt20 preserve_per_segment refuses a non-ASCII MCID BDC property list".to_string(),
        )
    })?;
    Ok(Some(PreservedMarkedContent {
        opening,
        source_open_range: [frame.open_start, frame.open_end],
    }))
}

fn mcid_frame_contains_text_state_only(data: &[u8], frame: &MarkedContentFrame) -> Result<bool> {
    let tokens = lex_content(data)?;
    let mut operands = Vec::<LexicalToken>::new();
    let mut stack = Vec::<usize>::new();
    let mut saw_target = false;
    let mut closed_target = false;
    for token in tokens {
        let LexicalKind::Word(operator) = &token.kind else {
            operands.push(token);
            continue;
        };
        let target_active = stack.last().copied() == Some(frame.open_start);
        match operator.as_str() {
            "BMC" | "BDC" => {
                let open_start = operands
                    .first()
                    .map(|operand| operand.start)
                    .unwrap_or(token.start);
                if target_active || (open_start == frame.open_start && !stack.is_empty()) {
                    return Ok(false);
                }
                stack.push(open_start);
                if open_start == frame.open_start {
                    saw_target = true;
                }
            }
            "EMC" => {
                if target_active {
                    closed_target = true;
                }
                stack.pop();
            }
            _ if target_active
                && !matches!(
                    operator.as_str(),
                    "BT" | "ET"
                        | "Tf"
                        | "Tc"
                        | "Tw"
                        | "Tz"
                        | "Ts"
                        | "Tr"
                        | "Tm"
                        | "Td"
                        | "TD"
                        | "T*"
                        | "Tj"
                        | "TJ"
                        | "'"
                        | "\""
                        | "g"
                        | "G"
                        | "rg"
                        | "RG"
                        | "k"
                        | "K"
                        | "q"
                        | "Q"
                        | "cm"
                ) =>
            {
                // These operators manipulate text state or the local graphics
                // state only. Any path/image/shading/inline-image or nested
                // tag operator makes relocation of the MCID ambiguous.
                return Ok(false);
            }
            _ => {}
        }
        operands.clear();
    }
    Ok(saw_target && closed_target && stack.is_empty())
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
            "prompt20 preserve_per_segment rejects an invalid source text state".to_string(),
        ));
    }
    if !(0..=7).contains(&token.text_render_mode) {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 preserve_per_segment rejects an unsupported source text rendering mode"
                .to_string(),
        ));
    }
    if matches!(token.text_render_mode, 4..=7) {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 preserve_per_segment refuses source text-clipping modes because moving the clipping text into a separate generated stream would alter clipping for later content"
                .to_string(),
        ));
    }
    let uses_stroke = matches!(token.text_render_mode, 1 | 2 | 5 | 6);
    if token.unsupported_fill_paint_state || (uses_stroke && token.unsupported_stroke_paint_state) {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 preserve_per_segment refuses non-DeviceGray/RGB/CMYK source paint state until the canonical serializer can preserve its color-space resource semantics"
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
    text: &str,
    style: &PreservedTextStyle,
) -> f64 {
    let width = split_codes(encoded, resolver.code_size())
        .into_iter()
        .map(|code| resolver.glyph_width(code))
        .sum::<f64>()
        / 1000.0
        * style.font_size;
    let character_count = text.chars().count().saturating_sub(1) as f64;
    let word_count = text.chars().filter(|ch| *ch == ' ').count() as f64;
    (width + character_count * style.character_spacing + word_count * style.word_spacing)
        * (style.horizontal_scaling / 100.0)
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
            "prompt20 preserve_per_segment line exceeds its bounded region".to_string(),
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
                "prompt20 preserve_per_segment full justification is refused until style-boundary spacing adjustment has an exact source-state proof"
                    .to_string(),
            ))
        }
    })
}

fn serialize_preserved_styled_runs(
    runs_by_scalar: &[PreservedStyledRun],
    logical_lines: &[ExplicitLayoutLine],
    options: &AdvancedTextEditOptions,
    mode: AdvancedTextMode,
    marked_content_opening: Option<&str>,
) -> Result<(String, Vec<GeneratedLineAdjustment>)> {
    if mode == AdvancedTextMode::ParagraphReflowVertical {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 preserve_per_segment positioned serializer does not support vertical writing"
                .to_string(),
        ));
    }
    let mut content = String::from("q\n");
    if let Some(opening) = marked_content_opening {
        content.push_str(opening);
        content.push('\n');
    }
    content.push_str("BT\n");
    let mut adjustments = Vec::with_capacity(logical_lines.len());
    let mut run_index = 0usize;
    let line_advance = options.font_size * options.line_spacing;
    for (line_index, line) in logical_lines.iter().enumerate() {
        if line.inserted_visual_hyphen {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 preserve_per_segment refuses an inserted hyphen because an existing source CMap cannot prove its empty ToUnicode behavior"
                    .to_string(),
            ));
        }
        let visual_scalars = line
            .logical_text
            .trim_end_matches(['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'])
            .chars()
            .count();
        let logical_scalars = line.logical_text.chars().count();
        let line_end = run_index.saturating_add(logical_scalars);
        if line_end > runs_by_scalar.len() {
            return Err(WellfriendError::MalformedPdf(
                "prompt20 preserve_per_segment line mapping exceeds replacement provenance"
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
                style.font_resource,
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
            "prompt20 preserve_per_segment final lines did not consume exact replacement provenance"
                .to_string(),
        ));
    }
    content.push_str("ET\n");
    if marked_content_opening.is_some() {
        content.push_str("EMC\n");
    }
    content.push('Q');
    Ok((content, adjustments))
}

fn lex_content(data: &[u8]) -> Result<Vec<LexicalToken>> {
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < data.len() {
        while index < data.len() && data[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= data.len() {
            break;
        }
        if data[index] == b'%' {
            while index < data.len() && !matches!(data[index], b'\n' | b'\r') {
                index += 1;
            }
            continue;
        }
        let start = index;
        let kind = match data[index] {
            b'(' => {
                let (decoded, _content_start, _content_end, end) =
                    parse_literal_string(data, index)?;
                index = end;
                LexicalKind::String(PatchStringRepresentation::Literal, decoded)
            }
            b'<' if data.get(index + 1) != Some(&b'<') => {
                let (decoded, _content_start, _content_end, end) = parse_hex_string(data, index)?;
                index = end;
                LexicalKind::String(PatchStringRepresentation::Hexadecimal, decoded)
            }
            b'<' if data.get(index + 1) == Some(&b'<') => {
                index += 2;
                LexicalKind::DictionaryStart
            }
            b'>' if data.get(index + 1) == Some(&b'>') => {
                index += 2;
                LexicalKind::DictionaryEnd
            }
            b'/' => {
                index += 1;
                let name_start = index;
                while index < data.len() && !is_pdf_delimiter(data[index]) {
                    index += 1;
                }
                LexicalKind::Name(String::from_utf8_lossy(&data[name_start..index]).into_owned())
            }
            b'[' => {
                index += 1;
                LexicalKind::ArrayStart
            }
            b']' => {
                index += 1;
                LexicalKind::ArrayEnd
            }
            _ => {
                while index < data.len() && !is_pdf_delimiter(data[index]) {
                    index += 1;
                }
                if index == start {
                    // Dictionary delimiters are irrelevant to text operators;
                    // consume one byte so malformed input cannot loop forever.
                    index += 1;
                }
                let word = String::from_utf8_lossy(&data[start..index]).into_owned();
                match word.parse::<f64>() {
                    Ok(number) if number.is_finite() => LexicalKind::Number(number),
                    _ => LexicalKind::Word(word),
                }
            }
        };
        tokens.push(LexicalToken {
            start,
            end: index,
            kind,
        });
    }
    Ok(tokens)
}

fn parse_literal_string(data: &[u8], start: usize) -> Result<(Vec<u8>, usize, usize, usize)> {
    let mut decoded = Vec::new();
    let mut index = start + 1;
    let mut depth = 1usize;
    while index < data.len() {
        match data[index] {
            b'(' => {
                depth += 1;
                decoded.push(b'(');
                index += 1;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((decoded, start + 1, index, index + 1));
                }
                decoded.push(b')');
                index += 1;
            }
            b'\\' => {
                index += 1;
                if index >= data.len() {
                    break;
                }
                match data[index] {
                    b'n' => decoded.push(b'\n'),
                    b'r' => decoded.push(b'\r'),
                    b't' => decoded.push(b'\t'),
                    b'b' => decoded.push(0x08),
                    b'f' => decoded.push(0x0c),
                    b'\n' => {}
                    b'\r' => {
                        if data.get(index + 1) == Some(&b'\n') {
                            index += 1;
                        }
                    }
                    digit @ b'0'..=b'7' => {
                        let mut value = digit - b'0';
                        for _ in 0..2 {
                            if let Some(next @ b'0'..=b'7') = data.get(index + 1).copied() {
                                index += 1;
                                value = value.saturating_mul(8).saturating_add(next - b'0');
                            } else {
                                break;
                            }
                        }
                        decoded.push(value);
                    }
                    other => decoded.push(other),
                }
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    Err(WellfriendError::MalformedPdf(format!(
        "prompt20 unterminated literal string at decoded stream byte {start}"
    )))
}

fn parse_hex_string(data: &[u8], start: usize) -> Result<(Vec<u8>, usize, usize, usize)> {
    let mut nibbles = Vec::new();
    let mut index = start + 1;
    while index < data.len() && data[index] != b'>' {
        if !data[index].is_ascii_whitespace() {
            nibbles.push(hex_nibble(data[index]).ok_or_else(|| {
                WellfriendError::MalformedPdf(format!(
                    "prompt20 invalid hex string digit at decoded stream byte {index}"
                ))
            })?);
        }
        index += 1;
    }
    if index >= data.len() {
        return Err(WellfriendError::MalformedPdf(format!(
            "prompt20 unterminated hex string at decoded stream byte {start}"
        )));
    }
    if nibbles.len() % 2 != 0 {
        nibbles.push(0);
    }
    let decoded = nibbles
        .chunks_exact(2)
        .map(|pair| pair[0] << 4 | pair[1])
        .collect();
    Ok((decoded, start + 1, index, index + 1))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_pdf_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
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

fn enforce_prompt20_signature_policy(
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
            "prompt20 {operation} blocked by signature policy; explicit override required"
        )));
    }
    if policy.full_rewrite_required {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 {operation} requires full rewrite but this operation is structurally incremental"
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
                max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
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
                "prompt20 vector object count exceeds limit 100000".to_string(),
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
        schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
        page: page_number,
        objects,
        form_recursion_limit: 8,
        vector_object_limit: 100_000,
        deterministic: true,
        exact_limits: vec![
            "objects are reconstructed from actual path and paint operator ranges; unrelated paths are never fused into inferred semantic shapes".to_string(),
            "reachable Form XObject paths are inventoried to depth 8; clone-edit-one is bounded to a top-level page invocation while nested Form edits require explicit edit-all".to_string(),
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
            "prompt20 vector Form recursion exceeds limit 8".to_string(),
        ));
    }
    for form_use in reachable_form_uses(owner_data, resources, reader)? {
        if active_forms.contains(&(form_use.object_number, form_use.generation)) {
            return Err(WellfriendError::MalformedPdf(format!(
                "prompt20 cyclic Form XObject graph reaches {} {} R",
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
                max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
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
        let nested_resources = resolve_prompt20_dict(dict.get("Resources"), reader)
            .unwrap_or_else(|| resources.clone());
        active_forms.push((form_use.object_number, form_use.generation));
        collect_form_vector_objects(
            reader,
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
    let Some(xobjects) = resolve_prompt20_dict(resources.get("XObject"), reader) else {
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

fn resolve_prompt20_dict(
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
    enforce_prompt20_signature_policy(
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
                "prompt20 vector stable ID {stable_id} not found on page {page_number}"
            ))
        })?;
    if before.provenance.form_stack.len() > 8 {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 vector Form recursion exceeds limit 8".to_string(),
        ));
    }
    let form_invocation = before.provenance.form_invocation.clone();
    if before.edit_safety == "shared_annotation_appearance_requires_clone"
        && options.shared_form_policy != SharedFormEditPolicy::CloneEditOneInstance
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 annotation appearance stream is shared by multiple annotations; ownership-specific appearance cloning is required".to_string(),
        ));
    }
    if form_invocation.is_some() && options.shared_form_policy == SharedFormEditPolicy::Reject {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 vector is owned by a Form XObject; select shared_form_policy edit_all_uses or clone_edit_one_instance explicitly".to_string(),
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
            "prompt20 vector provenance does not reference a stream".to_string(),
        ));
    };
    let decoded_result = decode_stream_lossless_with_limits(
        &PdfObject::Stream {
            dict: dict.clone(),
            raw,
        },
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    if decoded_result.status != StreamDecodeStatus::Complete {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 vector stream is not losslessly decodable".to_string(),
        ));
    }
    let mut decoded = decoded_result.data;
    let range = before.provenance.operation_byte_start..before.provenance.operation_byte_end;
    if range.end > decoded.len() || range.start > range.end {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 vector provenance range is outside decoded stream".to_string(),
        ));
    }
    let prefix = decoded[..range.start].to_vec();
    let suffix = decoded[range.end..].to_vec();
    decoded.splice(range.clone(), replacement.clone());
    let prefix_preserved = decoded.starts_with(&prefix);
    let suffix_preserved = decoded.ends_with(&suffix);
    let compressed = flate_encode(&decoded, 6);
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    let mut changed = Vec::new();
    let mut cloned_form = None;
    let mut clone_graph = Vec::new();
    if before.edit_safety == "shared_annotation_appearance_requires_clone"
        && options.shared_form_policy == SharedFormEditPolicy::CloneEditOneInstance
    {
        let owner_value = before.provenance.resource_owner.clone();
        let source_appearance_object = before.provenance.object_number;
        let source_appearance_generation = before.provenance.generation;
        let owner = owner_value.as_str();
        let prefix = owner.strip_prefix("annotation-").ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "prompt20b annotation appearance provenance is malformed".to_string(),
            )
        })?;
        let (annotation_index_text, appearance_tail) =
            prefix.split_once("-appearance-").ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "prompt20b annotation appearance provenance is malformed".to_string(),
                )
            })?;
        let annotation_index: usize = annotation_index_text.parse().map_err(|_| {
            WellfriendError::MalformedPdf(
                "prompt20b annotation appearance index is malformed".to_string(),
            )
        })?;
        let suffix = format!("-{source_appearance_object}-{source_appearance_generation}");
        let appearance_name = appearance_tail.strip_suffix(&suffix).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "prompt20b annotation appearance identity is malformed".to_string(),
            )
        })?;
        let page = engine.document().get_page(page_number)?;
        let page_object = reader.get_object(page.object_number, page.generation_number)?;
        let page_dict = page_object.as_dict().ok_or_else(|| {
            WellfriendError::MalformedPdf("prompt20b page object is not a dictionary".to_string())
        })?;
        let annots = reader.resolve(page_dict.get("Annots").cloned().ok_or_else(|| {
            WellfriendError::UnsupportedFeature("prompt20b page has no annotations".to_string())
        })?)?;
        let annotation_ref = annots
            .as_array()
            .and_then(|items| items.get(annotation_index))
            .and_then(PdfObject::as_reference)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "prompt20b clone-one requires an indirect target annotation".to_string(),
                )
            })?;
        let annotation_object = reader.get_object(annotation_ref.0, annotation_ref.1)?;
        let mut annotation_dict = annotation_object.as_dict().cloned().ok_or_else(|| {
            WellfriendError::MalformedPdf("prompt20b annotation is not a dictionary".to_string())
        })?;
        let mut ap = resolve_prompt20_dict(annotation_dict.get("AP"), reader).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "prompt20b annotation AP dictionary is malformed".to_string(),
            )
        })?;
        let clone_number = reader
            .object_ids()
            .into_iter()
            .map(|(number, _)| number)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
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
            let mut states = resolve_prompt20_dict(ap.get(parts[0]), reader).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "prompt20b appearance-state dictionary is malformed".to_string(),
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
                    "prompt20b requested appearance state does not exist".to_string(),
                ));
            }
            ap.insert(parts[0], PdfObject::Dictionary(states));
        } else {
            return Err(WellfriendError::MalformedPdf(
                "prompt20b appearance category/state identity is malformed".to_string(),
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
        after.provenance.object_number = clone_number;
        after.provenance.generation = 0;
        after.provenance.resource_owner =
            format!("annotation-{annotation_index}-appearance-{appearance_name}-{clone_number}-0");
        after.stable_id = vector_stable_id_for_object(&after);
        let report_after = (!matches!(operation, VectorEditOperation::Delete)).then_some(after);
        return Ok((output.clone(), VectorEditReport { schema_version:PROMPT20_SCHEMA_VERSION.to_string(), stable_id:stable_id.to_string(), operation, before, after:report_after, source_range:[range.start,range.end], replacement_bytes:replacement.len(), unrelated_decoded_prefix_preserved:prefix_preserved, unrelated_decoded_suffix_preserved:suffix_preserved, original_pdf_prefix_preserved:output.starts_with(input), output_reopened:true, output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:options.deterministic, shared_form_policy:options.shared_form_policy, cloned_form:Some([clone_number,0]), clone_graph:vec![format!("annotation:{annotation_index} AP/{appearance_name} -> {clone_number} 0 R; shared source {source_appearance_object} {source_appearance_generation} R retained")], cache_invalidation:prompt20_cache_invalidation(input,&output,false,true,true), exact_limits:vec!["clone-one updates only the selected annotation AP category/state and preserves /AS plus sibling N/R/D state entries".to_string(),"nested Form resources inside an appearance use the Form invocation clone-one path; malformed or ambiguous state dictionaries fail closed".to_string()] }));
    }
    if options.shared_form_policy == SharedFormEditPolicy::CloneEditOneInstance {
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
                            "prompt20b annotation appearance stack is malformed".to_string(),
                        )
                    })?;
            let (annotation_index_text, appearance_name) =
                annotation_tail.split_once(":appearance:").ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "prompt20b annotation appearance stack is malformed".to_string(),
                    )
                })?;
            let annotation_index: usize = annotation_index_text.parse().map_err(|_| {
                WellfriendError::MalformedPdf(
                    "prompt20b annotation appearance index is malformed".to_string(),
                )
            })?;
            let invocation_path = before.provenance.form_invocation_path.clone();
            let source_appearance = invocation_path.first().ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "prompt20b nested annotation appearance clone-one requires an invocation path"
                        .to_string(),
                )
            })?;
            let source_appearance_number = source_appearance.owner_stream_object;
            let source_appearance_generation = source_appearance.owner_stream_generation;
            let page = engine.document().get_page(page_number)?;
            let page_object = reader.get_object(page.object_number, page.generation_number)?;
            let page_dict = page_object.as_dict().ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "prompt20b page object is not a dictionary".to_string(),
                )
            })?;
            let annots = reader.resolve(page_dict.get("Annots").cloned().ok_or_else(|| {
                WellfriendError::UnsupportedFeature("prompt20b page has no annotations".to_string())
            })?)?;
            let annotation_ref = annots
                .as_array()
                .and_then(|items| items.get(annotation_index))
                .and_then(PdfObject::as_reference)
                .ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "prompt20b clone-one requires an indirect target annotation".to_string(),
                    )
                })?;
            let annotation_object = reader.get_object(annotation_ref.0, annotation_ref.1)?;
            let mut annotation_dict = annotation_object.as_dict().cloned().ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "prompt20b annotation is not a dictionary".to_string(),
                )
            })?;
            let mut ap =
                resolve_prompt20_dict(annotation_dict.get("AP"), reader).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "prompt20b annotation AP dictionary is malformed".to_string(),
                    )
                })?;

            let mut next_number = reader
                .object_ids()
                .into_iter()
                .map(|(number, _)| number)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let leaf_number = next_number;
            next_number = next_number.saturating_add(1);
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
            for invocation in invocation_path.iter().rev() {
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
                        "prompt20b appearance Form invocation owner is not a stream".to_string(),
                    ));
                };
                let decoded_owner = decode_stream_lossless_with_limits(
                    &PdfObject::Stream {
                        dict: owner_dict.clone(),
                        raw: owner_raw,
                    },
                    reader,
                    &DecodeLimits {
                        max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
                        ..DecodeLimits::default()
                    },
                )?;
                if decoded_owner.status != StreamDecodeStatus::Complete {
                    return Err(WellfriendError::UnsupportedFeature(
                        "prompt20b appearance Form invocation owner is not losslessly decodable"
                            .to_string(),
                    ));
                }
                let owner_range =
                    invocation.owner_operation_byte_start..invocation.owner_operation_byte_end;
                if owner_range.end > decoded_owner.data.len() || owner_range.start > owner_range.end
                {
                    return Err(WellfriendError::MalformedPdf(
                        "prompt20b appearance Form invocation range is outside owner stream"
                            .to_string(),
                    ));
                }
                let mut owner_data = decoded_owner.data;
                let mut resources = resolve_prompt20_dict(owner_dict.get("Resources"), reader)
                    .unwrap_or_else(crate::PdfDictionary::empty);
                let mut xobjects = resolve_prompt20_dict(resources.get("XObject"), reader)
                    .unwrap_or_else(crate::PdfDictionary::empty);
                let mut resource_name = format!("OxV{child_number}");
                let mut suffix_index = 0u32;
                while xobjects.contains_key(&resource_name) {
                    suffix_index = suffix_index.saturating_add(1);
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
                let encoded = flate_encode(&owner_data, 6);
                owner_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
                owner_dict.remove("DecodeParms");
                owner_dict.insert("Length", PdfObject::Integer(encoded.len() as i64));
                let clone_number = next_number;
                next_number = next_number.saturating_add(1);
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
                    "prompt20b nested annotation appearance path did not reach an AP owner"
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
                let mut states =
                    resolve_prompt20_dict(ap.get(parts[0]), reader).ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "prompt20b appearance-state dictionary is malformed".to_string(),
                        )
                    })?;
                if !states.contains_key(parts[1]) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "prompt20b requested appearance state does not exist".to_string(),
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
                    "prompt20b appearance category/state identity is malformed".to_string(),
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
            after.provenance.object_number = leaf_number;
            after.provenance.generation = 0;
            after.provenance.resource_owner = format!("form-{leaf_number}-0");
            after.stable_id = vector_stable_id_for_object(&after);
            let report_after = (!matches!(operation, VectorEditOperation::Delete)).then_some(after);
            return Ok((output.clone(), VectorEditReport { schema_version:PROMPT20_SCHEMA_VERSION.to_string(), stable_id:stable_id.to_string(), operation, before, after:report_after, source_range:[range.start,range.end], replacement_bytes:replacement.len(), unrelated_decoded_prefix_preserved:prefix_preserved, unrelated_decoded_suffix_preserved:suffix_preserved, original_pdf_prefix_preserved:output.starts_with(input), output_reopened:true, output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:options.deterministic, shared_form_policy:options.shared_form_policy, cloned_form:Some([leaf_number,0]), clone_graph, cache_invalidation:prompt20_cache_invalidation(input,&output,false,true,true), exact_limits:vec!["clone-one for nested annotation appearance Forms clones the edited leaf, each selected appearance Form owner, and only the target annotation AP entry".to_string(),"sibling N/R/D state entries and /AS are preserved; malformed or ambiguous appearance-state dictionaries fail closed".to_string()] }));
        }
        let invocation_path = before.provenance.form_invocation_path.clone();
        if invocation_path.len() > 1 {
            let page = engine.document().get_page(page_number)?;
            let mut next_number = reader
                .object_ids()
                .into_iter()
                .map(|(number, _)| number)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let leaf_number = next_number;
            next_number = next_number.saturating_add(1);
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
            for invocation in invocation_path.iter().rev() {
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
                        "prompt20b Form invocation owner is not a stream".to_string(),
                    ));
                };
                let decoded_owner = decode_stream_lossless_with_limits(
                    &PdfObject::Stream {
                        dict: owner_dict.clone(),
                        raw: owner_raw,
                    },
                    reader,
                    &DecodeLimits {
                        max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
                        ..DecodeLimits::default()
                    },
                )?;
                if decoded_owner.status != StreamDecodeStatus::Complete {
                    return Err(WellfriendError::UnsupportedFeature(
                        "prompt20b Form invocation owner is not losslessly decodable".to_string(),
                    ));
                }
                let range =
                    invocation.owner_operation_byte_start..invocation.owner_operation_byte_end;
                if range.end > decoded_owner.data.len() || range.start > range.end {
                    return Err(WellfriendError::MalformedPdf(
                        "prompt20b Form invocation range is outside owner stream".to_string(),
                    ));
                }
                let is_page_owner = page.contents.contains(&(
                    invocation.owner_stream_object,
                    invocation.owner_stream_generation,
                ));
                let mut owner_data = decoded_owner.data;
                let resource_owner = if is_page_owner {
                    page.resources.clone()
                } else {
                    resolve_prompt20_dict(owner_dict.get("Resources"), reader).ok_or_else(|| WellfriendError::UnsupportedFeature("prompt20b nested Form clone-one requires a direct or indirect parent Form Resources dictionary".to_string()))?
                };
                let mut xobjects = resolve_prompt20_dict(resource_owner.get("XObject"), reader)
                    .unwrap_or_else(crate::PdfDictionary::empty);
                let mut resource_name = format!("OxV{child_number}");
                let mut suffix_index = 0u32;
                while xobjects.contains_key(&resource_name) {
                    suffix_index = suffix_index.saturating_add(1);
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
                                "prompt20b page object is not a dictionary".to_string(),
                            )
                        })?;
                    let mut resources = page.resources.clone();
                    resources.insert("XObject", PdfObject::Dictionary(xobjects));
                    page_object.insert("Resources", PdfObject::Dictionary(resources));
                    page_update = Some(page_object);
                    let encoded = flate_encode(&owner_data, 6);
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
                    let encoded = flate_encode(&owner_data, 6);
                    owner_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
                    owner_dict.remove("DecodeParms");
                    owner_dict.insert("Length", PdfObject::Integer(encoded.len() as i64));
                    let parent_clone = next_number;
                    next_number = next_number.saturating_add(1);
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
                    "prompt20b nested Form clone-one path has no page-owned outer invocation"
                        .to_string(),
                )
            })?;
            changed.push(IncrementalObject {
                number: page.object_number,
                generation: page.generation_number,
                object: PdfObject::Dictionary(page_dict),
            });
            let output = write_incremental_update(reader, changed)?;
            ContentEngine::open_bytes(output.clone())?;
            after.provenance.object_number = leaf_number;
            after.provenance.generation = 0;
            after.provenance.resource_owner = format!("form-{leaf_number}-0");
            after.stable_id = vector_stable_id_for_object(&after);
            let report_after = (!matches!(operation, VectorEditOperation::Delete)).then_some(after);
            return Ok((output.clone(), VectorEditReport { schema_version:PROMPT20_SCHEMA_VERSION.to_string(), stable_id:stable_id.to_string(), operation, before, after:report_after, source_range:[range.start,range.end], replacement_bytes:replacement.len(), unrelated_decoded_prefix_preserved:prefix_preserved, unrelated_decoded_suffix_preserved:suffix_preserved, original_pdf_prefix_preserved:output.starts_with(input), output_reopened:true, output_sha256:format!("{:x}",Sha256::digest(&output)), signature_policy, cryptographic_validity_claimed:false, deterministic:options.deterministic, shared_form_policy:options.shared_form_policy, cloned_form:Some([leaf_number,0]), clone_graph, cache_invalidation:prompt20_cache_invalidation(input,&output,false,true,false), exact_limits:vec!["clone-one recursively clones the leaf and each selected parent Form invocation path; unrelated source Forms are retained".to_string(),"nested Form clone-one requires losslessly decodable streams and direct or indirect Resources dictionaries; cyclic/malformed graphs fail closed".to_string()] }));
        }
        let invocation = form_invocation.as_ref().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "prompt20 clone_edit_one_instance requires a Form-owned vector object".to_string(),
            )
        })?;
        if invocation.depth != 1 {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt20 clone_edit_one_instance is bounded to top-level page Form invocations; selected depth is {}",
                invocation.depth
            )));
        }
        let page = engine.document().get_page(page_number)?;
        let new_number = reader
            .object_ids()
            .into_iter()
            .map(|(number, _)| number)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut resources = page.resources.clone();
        let mut xobjects = resolve_prompt20_dict(resources.get("XObject"), reader)
            .unwrap_or_else(crate::PdfDictionary::empty);
        let mut resource_name = format!("OxV{new_number}");
        let mut suffix_index = 0u32;
        while xobjects.contains_key(&resource_name) {
            suffix_index = suffix_index.saturating_add(1);
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
            WellfriendError::MalformedPdf("prompt20 page object is not a dictionary".to_string())
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
                "prompt20 Form invocation owner is not a stream".to_string(),
            ));
        };
        let owner_decoded = decode_stream_lossless_with_limits(
            &PdfObject::Stream {
                dict: owner_dict.clone(),
                raw: owner_raw,
            },
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
                ..DecodeLimits::default()
            },
        )?;
        if owner_decoded.status != StreamDecodeStatus::Complete {
            return Err(WellfriendError::UnsupportedFeature(
                "prompt20 Form invocation owner is not losslessly decodable".to_string(),
            ));
        }
        let owner_range =
            invocation.owner_operation_byte_start..invocation.owner_operation_byte_end;
        if owner_range.end > owner_decoded.data.len() || owner_range.start > owner_range.end {
            return Err(WellfriendError::MalformedPdf(
                "prompt20 Form invocation range is outside its owner stream".to_string(),
            ));
        }
        let mut owner_data = owner_decoded.data;
        owner_data.splice(owner_range, format!("/{resource_name} Do").into_bytes());
        let owner_compressed = flate_encode(&owner_data, 6);
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
    let output = write_incremental_update(reader, changed)?;
    ContentEngine::open_bytes(output.clone())?;
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    let report_after = (!matches!(operation, VectorEditOperation::Delete)).then_some(after);
    Ok((
        output.clone(),
        VectorEditReport {
            schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
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
            cache_invalidation: prompt20_cache_invalidation(input, &output, false, true, false),
            exact_limits: vec![
                "only the reconstructed paint operation range is rewritten; surrounding operators and the original PDF prefix are preserved".to_string(),
                "semantic shape names such as ellipse are not inferred from arbitrary cubic paths".to_string(),
                "shared Form edits require explicit edit-all or clone-one policy; clone-one is bounded to top-level page invocations and never mutates the source Form".to_string(),
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
            "prompt20 bounded z-order currently requires a page-owned operation range; Form z-order changes require ownership-specific group analysis".to_string(),
        ));
    }
    if before.provenance.marked_content_depth != 0
        || before.provenance.ocg_context.is_some()
        || before.clipping_path
        || before.clipping_context
    {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 z-order change would cross clipping, marked-content, or OCG semantics"
                .to_string(),
        ));
    }
    let mut siblings = inventory
        .into_iter()
        .filter(|object| {
            object.provenance.form_invocation.is_none()
                && object.provenance.object_number == before.provenance.object_number
                && object.provenance.generation == before.provenance.generation
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
                "prompt20 z-order target is not in its page-owned sibling set".to_string(),
            )
        })?;
    let reader = engine.document().reader();
    let stream_object = reader.get_object(
        before.provenance.object_number,
        before.provenance.generation,
    )?;
    let PdfObject::Stream { mut dict, raw } = stream_object else {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 z-order provenance does not reference a stream".to_string(),
        ));
    };
    let decoded_result = decode_stream_lossless_with_limits(
        &PdfObject::Stream {
            dict: dict.clone(),
            raw,
        },
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    if decoded_result.status != StreamDecodeStatus::Complete {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 z-order stream is not losslessly decodable".to_string(),
        ));
    }
    let range = before.provenance.operation_byte_start..before.provenance.operation_byte_end;
    if range.end > decoded_result.data.len() || range.start > range.end {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 z-order operation range is outside decoded stream".to_string(),
        ));
    }
    let insertion_original = match operation {
        VectorEditOperation::BringForward => siblings
            .get(selected_index + 1)
            .map(|object| object.provenance.operation_byte_end)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "prompt20 vector is already the front sibling".to_string(),
                )
            })?,
        VectorEditOperation::SendBackward => selected_index
            .checked_sub(1)
            .and_then(|index| siblings.get(index))
            .map(|object| object.provenance.operation_byte_start)
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "prompt20 vector is already the back sibling".to_string(),
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
    let replacement = serialize_vector_object(&replacement_object);
    let removed_len = range.end - range.start;
    let mut decoded = decoded_result.data;
    decoded.drain(range.clone());
    let insertion = if insertion_original >= range.end {
        insertion_original.saturating_sub(removed_len)
    } else {
        insertion_original
    };
    decoded.splice(insertion..insertion, replacement.clone());
    let prefix_preserved = decoded.starts_with(&original_prefix);
    let suffix_preserved = decoded.ends_with(&original_suffix);
    let compressed = flate_encode(&decoded, 6);
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    let output = write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: before.provenance.object_number,
            generation: before.provenance.generation,
            object: PdfObject::Stream {
                dict,
                raw: compressed,
            },
        }],
    )?;
    ContentEngine::open_bytes(output.clone())?;
    replacement_object.stable_id = vector_stable_id_for_object(&replacement_object);
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    Ok((
        output.clone(),
        VectorEditReport {
            schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
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
            clone_graph: Vec::new(),
            cache_invalidation: prompt20_cache_invalidation(input, &output, false, true, false),
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
            "prompt20 bounded group/ungroup currently requires page-owned operation ranges"
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
            "prompt20 group provenance does not reference a stream".to_string(),
        ));
    };
    let decoded_result = decode_stream_lossless_with_limits(
        &PdfObject::Stream {
            dict: dict.clone(),
            raw,
        },
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_PROMPT20_PATCH_STREAM_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    if decoded_result.status != StreamDecodeStatus::Complete {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 group stream is not losslessly decodable".to_string(),
        ));
    }
    let mut siblings = inventory
        .iter()
        .filter(|object| {
            object.provenance.form_invocation.is_none()
                && object.provenance.object_number == before.provenance.object_number
                && object.provenance.generation == before.provenance.generation
        })
        .cloned()
        .collect::<Vec<_>>();
    siblings.sort_by_key(|object| object.provenance.operation_byte_start);
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
                    "prompt20 group_with requires at least two distinct stable IDs".to_string(),
                ));
            }
            let mut indices = Vec::new();
            for stable_id in &selected_ids {
                let index = siblings
                    .iter()
                    .position(|object| object.stable_id == *stable_id)
                    .ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(format!(
                            "prompt20 group member {stable_id} is not a sibling in the selected page stream"
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
                    "prompt20 bounded grouping requires contiguous sibling vector ranges"
                        .to_string(),
                ));
            }
            let first = &siblings[*indices.first().expect("non-empty group indices")];
            let last = &siblings[*indices.last().expect("non-empty group indices")];
            let range = first.provenance.operation_byte_start..last.provenance.operation_byte_end;
            if range.end > decoded_result.data.len() {
                return Err(WellfriendError::MalformedPdf(
                    "prompt20 group range is outside decoded stream".to_string(),
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
                        "prompt20 selected vector is not inside an Wellfriend bounded group"
                            .to_string(),
                    )
                })?;
            if group.marker_end > decoded_result.data.len()
                || group.content_start > group.content_end
            {
                return Err(WellfriendError::MalformedPdf(
                    "prompt20 group marker range is outside decoded stream".to_string(),
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
    let compressed = flate_encode(&decoded, 6);
    dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(compressed.len() as i64));
    let output = write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: before.provenance.object_number,
            generation: before.provenance.generation,
            object: PdfObject::Stream {
                dict,
                raw: compressed,
            },
        }],
    )?;
    ContentEngine::open_bytes(output.clone())?;
    let reopened_inventory = list_vector_objects(&output, before.provenance.page)?;
    let after = reopened_inventory.objects.into_iter().find(|object| {
        object.provenance.object_number == before.provenance.object_number
            && object.bbox == before.bbox
    });
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    Ok((
        output.clone(),
        VectorEditReport {
            schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
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
            clone_graph: Vec::new(),
            cache_invalidation: prompt20_cache_invalidation(input, &output, false, true, false),
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
                    schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
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
            "prompt20 vector edit contains non-finite or out-of-range numbers".to_string(),
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
                    "prompt20 vector segment {segment} is out of range"
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
                "prompt20 vector point index {point_index} is invalid for selected segment"
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
    pub status: Prompt20SupportStatus,
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
            "prompt20 link-annotation move requires finite expected geometry and a non-zero finite delta",
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let signature_policy = analyze_edit_policy(&engine, SignatureEditOperation::AnnotationUpdate)?;
    enforce_prompt20_signature_policy(
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
            "prompt20 Link annotation page is not a dictionary".to_string(),
        )
    })?;
    let annots = reader.resolve(page_dict.get("Annots").cloned().ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "prompt20 Link annotation move requires a page /Annots array".to_string(),
        )
    })?)?;
    let annotation_ref = annots
        .as_array()
        .and_then(|items| items.get(annotation_index))
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "prompt20 Link annotation {annotation_index} on page {page_number} must be an indirect annotation dictionary"
            ))
        })?;
    let annotation_object = reader.get_object(annotation_ref.0, annotation_ref.1)?;
    let mut annotation = annotation_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "prompt20 Link annotation reference is not a dictionary".to_string(),
        )
    })?;
    if annotation.get_name("Subtype") != Some("Link") {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 annotation {annotation_index} on page {page_number} is not /Subtype /Link"
        )));
    }
    if annotation.get("A").is_none() && annotation.get("Dest").is_none() {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 Link annotation move refuses a Link without an action or destination to preserve"
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
            "prompt20 stale_snapshot: Link annotation rectangle no longer matches the explicit source-associated expected rectangle"
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
            "prompt20 constraint_infeasible: moved Link annotation rectangle would leave the canonical page box"
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
                "prompt20 Link annotation /QuadPoints must contain complete quadrilaterals"
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
            "prompt20 moved Link annotation did not reopen as a dictionary".to_string(),
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
            "prompt20 Link annotation move failed reopen, action-preservation, or prefix proof"
                .to_string(),
        ));
    }
    Ok((
        output.clone(),
        LinkAnnotationMoveReport {
            schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
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
            cache_invalidation: prompt20_cache_invalidation(input, &output, false, false, true),
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
    enforce_prompt20_signature_policy(
        &signature_policy,
        signature_policy_override,
        "ink annotation fitting",
    )?;
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let page_object = reader.get_object(page.object_number, page.generation_number)?;
    let page_dict = page_object.as_dict().ok_or_else(|| {
        WellfriendError::MalformedPdf("prompt20 page object is not a dictionary".to_string())
    })?;
    let annots_object = page_dict.get("Annots").ok_or_else(|| {
        WellfriendError::UnsupportedFeature(format!(
            "prompt20 page {page_number} has no annotations"
        ))
    })?;
    let annots = reader.resolve(annots_object.clone())?;
    let annotation_ref = annots
        .as_array()
        .and_then(|items| items.get(annotation_index))
        .and_then(PdfObject::as_reference)
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "prompt20 annotation {annotation_index} on page {page_number} must be an indirect annotation dictionary"
            ))
        })?;
    let annotation_object = reader.get_object(annotation_ref.0, annotation_ref.1)?;
    let mut annotation = annotation_object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf(
            "prompt20 annotation reference is not a dictionary".to_string(),
        )
    })?;
    if annotation.get_name("Subtype") != Some("Ink") {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 annotation {annotation_index} is not /Subtype /Ink"
        )));
    }
    let raw_strokes = pdf_nested_points(reader, annotation.get("InkList"))?;
    let fitted = fit_ink_strokes(&raw_strokes, options)?;
    let rect = pdf_number_array(reader, annotation.get("Rect"))?;
    if rect.len() < 4 {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 Ink annotation /Rect must contain four finite numbers".to_string(),
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
    let appearance_number = reader
        .object_ids()
        .into_iter()
        .map(|(number, _)| number)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
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
    let appearance_raw = flate_encode(appearance_content.as_bytes(), 6);
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
            "prompt20 fitted Ink annotation failed incremental readback verification".to_string(),
        ));
    }
    let output_sha256 = format!("{:x}", Sha256::digest(&output));
    Ok((
        output.clone(),
        AnnotationInkFitReport {
            schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
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
            cache_invalidation: prompt20_cache_invalidation(input, &output, false, true, true),
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
    if points.len() > options.max_points.min(MAX_PROMPT20_INK_POINTS) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 ink stroke has {} points; limit is {}",
            points.len(),
            options.max_points.min(MAX_PROMPT20_INK_POINTS)
        )));
    }
    for (index, point) in points.iter().enumerate() {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(WellfriendError::MalformedPdf(format!(
                "prompt20 ink point {index} is NaN or infinite"
            )));
        }
        if point.x.abs() > 1.0e9 || point.y.abs() > 1.0e9 {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "prompt20 ink point {index} exceeds bounded coordinate range +/-1e9"
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
    if segments.len() > options.max_segments.min(MAX_PROMPT20_INK_SEGMENTS) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 fitted segment count {} exceeds limit {}",
            segments.len(),
            options.max_segments.min(MAX_PROMPT20_INK_SEGMENTS)
        )));
    }
    let (maximum_deviation, rms_deviation) = curve_error_metrics(&cleaned, &segments);
    if options.policy == InkFitPolicy::StrictErrorThreshold
        && maximum_deviation > options.error_threshold + EPSILON
    {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 ink fit maximum deviation {:.6} exceeds strict threshold {:.6}",
            maximum_deviation, options.error_threshold
        )));
    }
    let elapsed = started.elapsed();
    if options.policy == InkFitPolicy::PerformanceThreshold {
        if let Some(limit) = options.performance_threshold_ms {
            if elapsed.as_millis() > u128::from(limit) {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "prompt20 ink fit elapsed {} ms exceeds performance threshold {limit} ms",
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
            schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
            status: Prompt20SupportStatus::ImplementedWithLimits,
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
            WellfriendError::UnsupportedFeature("prompt20 ink point count overflow".to_string())
        })
    })?;
    if total_points > options.max_points.min(MAX_PROMPT20_INK_POINTS) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 ink stroke set has {total_points} points; aggregate limit is {}",
            options.max_points.min(MAX_PROMPT20_INK_POINTS)
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
        schema_version: PROMPT20_SCHEMA_VERSION.to_string(),
        strokes: results,
        total_points_before: total_points,
        total_segments,
        deterministic_digest: format!("{:x}", hasher.finalize()),
    })
}

fn pdf_number_array(reader: &crate::PdfReader, object: Option<&PdfObject>) -> Result<Vec<f64>> {
    let object = object.ok_or_else(|| {
        WellfriendError::MalformedPdf("prompt20 required numeric array is missing".to_string())
    })?;
    let resolved = reader.resolve(object.clone())?;
    let values = resolved.as_array().ok_or_else(|| {
        WellfriendError::MalformedPdf("prompt20 expected numeric array".to_string())
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
                        "prompt20 numeric array contains a non-finite or non-number value"
                            .to_string(),
                    )
                })
        })
        .collect()
}

fn normalized_annotation_rect(values: &[f64]) -> Result<[f64; 4]> {
    if values.len() != 4 || values.iter().any(|value| !value.is_finite()) {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 annotation /Rect must contain exactly four finite numbers".to_string(),
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
            "prompt20 annotation /Rect must have positive normalized width and height".to_string(),
        ));
    }
    Ok(rect)
}

fn pdf_nested_points(
    reader: &crate::PdfReader,
    object: Option<&PdfObject>,
) -> Result<Vec<Vec<InkPoint>>> {
    let object = object.ok_or_else(|| {
        WellfriendError::MalformedPdf("prompt20 Ink annotation has no /InkList".to_string())
    })?;
    let resolved = reader.resolve(object.clone())?;
    let strokes = resolved.as_array().ok_or_else(|| {
        WellfriendError::MalformedPdf("prompt20 /InkList is not an array".to_string())
    })?;
    let mut output = Vec::with_capacity(strokes.len());
    for (stroke_index, stroke) in strokes.iter().enumerate() {
        let numbers = pdf_number_array(reader, Some(stroke))?;
        if numbers.len() % 2 != 0 {
            return Err(WellfriendError::MalformedPdf(format!(
                "prompt20 /InkList stroke {stroke_index} has an odd coordinate count"
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
                "prompt20 ink {name} must be finite and non-negative"
            )));
        }
    }
    if options.error_threshold <= EPSILON {
        return Err(WellfriendError::MalformedPdf(
            "prompt20 ink error_threshold must be greater than zero".to_string(),
        ));
    }
    if options.max_recursion > MAX_PROMPT20_FIT_RECURSION {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 ink max_recursion {} exceeds hard cap {MAX_PROMPT20_FIT_RECURSION}",
            options.max_recursion
        )));
    }
    if options.newton_iterations > 16 {
        return Err(WellfriendError::UnsupportedFeature(
            "prompt20 ink Newton iteration cap is 16".to_string(),
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
    if depth > options.max_recursion.min(MAX_PROMPT20_FIT_RECURSION) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 ink fitting exceeded recursion limit {}",
            options.max_recursion.min(MAX_PROMPT20_FIT_RECURSION)
        )));
    }
    if segments.len() >= options.max_segments.min(MAX_PROMPT20_INK_SEGMENTS) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "prompt20 ink fitting exceeded segment limit {}",
            options.max_segments.min(MAX_PROMPT20_INK_SEGMENTS)
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

fn canonical_number(value: f64) -> f64 {
    if value.abs() < 0.000_000_5 {
        0.0
    } else {
        (value * 1_000_000.0).round() / 1_000_000.0
    }
}

pub fn prompt20_report(engine: &ContentEngine) -> Result<serde_json::Value> {
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
        "schema_version": PROMPT20_SCHEMA_VERSION,
        "status": "implemented_with_limits",
        "text": {
            "modes": ["safe_patch", "paragraph_reflow_horizontal", "paragraph_reflow_rtl", "paragraph_reflow_vertical", "overlay_fallback", "unsupported"],
            "existing_pdf_glyph_streams_reshaped": false,
            "new_unicode_shaping": "rustybuzz_with_cluster_provenance",
            "rtl": "serialized_type0_identity_h_bounded_single_source_token",
            "vertical": "serialized_type0_identity_v_bounded_single_source_token",
            "missing_glyph_policy": "fail_closed_exact"
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
            "clone_edit_one_limit": "recursive_selected_invocation_chain_with_prompt20b_limits",
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
        "feature": prompt20_feature_report_value(crate::sdk::REPORT_ENVELOPE_VERSION)
    }))
}

pub(crate) fn prompt20_feature_report_value(envelope_version: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": PROMPT20_SCHEMA_VERSION,
        "envelope_version": envelope_version,
        "status": "implemented_with_limits",
        "coverage": {
            "rtl_vertical_analysis": "implemented_with_provenance",
            "rtl_vertical_serialized_edit": "implemented_with_single_source_token_limit",
            "same_width_patch": "implemented_with_exact_eligibility",
            "vector_page_stream_model_and_edit": "implemented_with_operation_range_rewrite",
            "vector_reachable_form_model": "implemented_depth_8",
            "vector_shared_form_edit_all": "implemented_explicit_policy",
            "vector_shared_form_clone_one": "implemented_recursive_selected_invocation_chain_with_prompt20b_limits",
            "vector_annotation_appearances": "implemented_owner_specific_AP_clone_one_for_N_R_D_state_and_nested_Form_paths",
            "vector_z_order": "implemented_page_owned_safe_contexts",
            "vector_group_ungroup": "implemented_contiguous_page_owned_marked_content",
            "ink_cubic_fitting": "implemented_error_bounded_deterministic",
            "ink_annotation_appearance": "implemented_incremental",
            "undo_redo": "incremental_suffix_patch_session_with_checkpoint_fingerprints_and_branch_redo_clearing",
            "signature_policy": "prompt18b_preflight_enforced",
            "cache_invalidation": "text_glyph_render_vector_annotation_semantic_search_ocg_writer_flags_with_before_after_fingerprints"
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
            "paragraph_chars": MAX_PROMPT20_PARAGRAPH_CHARS,
            "bidi_runs": MAX_PROMPT20_BIDI_RUNS,
            "glyphs": MAX_PROMPT20_GLYPHS,
            "content_stream_bytes": MAX_PROMPT20_PATCH_STREAM_BYTES,
            "vector_objects": 100000,
            "form_recursion": 8,
            "ink_points": MAX_PROMPT20_INK_POINTS,
            "ink_segments": MAX_PROMPT20_INK_SEGMENTS,
            "ink_recursion": MAX_PROMPT20_FIT_RECURSION
        },
        "unsupported_exact": [
            "paragraphs spanning multiple independent PDF string tokens require a higher-level provenance selection and are not silently overlaid",
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

pub(crate) fn prompt20b_feature_report_value(envelope_version: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "prompt20b.multirun-form-appearance-closure.v1",
        "envelope_version": envelope_version,
        "status": "implemented_with_limits",
        "coverage": {
            "multi_run_selection": "logical_token_boundary_provenance",
            "rtl_logical_visual_mapping": "bidi_run_provenance",
            "vertical_range": "generated_cluster_layout_with_explicit_limits",
            "multi_operator_serialization": "Tj_TJ_quote_double_quote_token_sequences",
            "nested_form_clone_one": "recursive_leaf_to_page_invocation_path",
            "annotation_appearance_clone_one": "target_annotation_N_R_D_or_state_owner",
            "widget_state_preservation": "AP_and_AS_preserved_without_field_value_mutation",
            "undo_redo": "prompt20_incremental_patch_session",
            "signature_policy": "prompt18b_preflight_enforced"
        },
        "bindings": {"rust":"implemented", "cli":"implemented", "python":"implemented", "c_abi":"implemented", "wasm":"implemented_memory_safe_json_and_owned_bytes", "dotnet":"implemented", "java_maven":"implemented", "java_gradle":"implemented"},
        "failure": {"blocked":0, "unclassified":0, "security":0},
        "exact_limits": [
            "logical multi-run selection is limited to contiguous whole decoded string tokens in one page content stream",
            "preserve_per_segment style output is not yet a per-style generated-run serializer",
            "nested clone-one requires lossless streams and direct or indirect resource dictionaries",
            "arbitrary Type3, pattern, and shading program editing remain unsupported",
            "structural signature policy does not claim cryptographic validity"
        ]
    })
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt20MutationPatch {
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
pub struct Prompt20MutationCheckpoint {
    pub sequence: usize,
    pub document_bytes: usize,
    pub document_sha256: String,
    pub patch_count: usize,
}

#[derive(Debug, Clone)]
pub struct Prompt20MutationSession {
    current: Vec<u8>,
    patches: Vec<Prompt20MutationPatch>,
    checkpoints: Vec<Prompt20MutationCheckpoint>,
    cursor: usize,
    max_patches: usize,
    max_total_patch_bytes: usize,
}

impl Prompt20MutationSession {
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
                "prompt20 mutation session limits are outside the supported range".to_string(),
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

    pub fn patches(&self) -> &[Prompt20MutationPatch] {
        &self.patches
    }

    pub fn checkpoints(&self) -> &[Prompt20MutationCheckpoint] {
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
    ) -> Result<&Prompt20MutationPatch> {
        let (output, report) = edit_advanced_text_pdf(
            &self.current,
            page,
            old_text,
            new_text,
            mode,
            options,
            font_bytes,
        )?;
        self.commit("text_edit", output, prompt20_report_json_value(report)?)
    }

    pub fn apply_same_width_patch(
        &mut self,
        page: usize,
        old_text: &str,
        new_text: &str,
        options: &SameWidthPatchOptions,
    ) -> Result<&Prompt20MutationPatch> {
        let (output, report) =
            apply_same_width_patch(&self.current, page, old_text, new_text, options)?;
        self.commit(
            "same_width_patch",
            output,
            prompt20_report_json_value(report)?,
        )
    }

    pub fn apply_multi_run_text_range(
        &mut self,
        request: &MultiRunTextRangeRequest,
        font_bytes: Option<&[u8]>,
    ) -> Result<&Prompt20MutationPatch> {
        let (output, report) = edit_multi_run_text_range(&self.current, request, font_bytes)?;
        self.commit(
            "multi_run_text_range",
            output,
            prompt20_report_json_value(report)?,
        )
    }

    pub fn apply_vector(
        &mut self,
        page: usize,
        stable_id: &str,
        operation: VectorEditOperation,
        options: &VectorEditOptions,
    ) -> Result<&Prompt20MutationPatch> {
        let (output, report) =
            edit_vector_object(&self.current, page, stable_id, operation, options)?;
        self.commit("vector_edit", output, prompt20_report_json_value(report)?)
    }

    pub fn apply_annotation_ink(
        &mut self,
        page: usize,
        annotation_index: usize,
        options: &InkFitOptions,
        signature_policy_override: bool,
    ) -> Result<&Prompt20MutationPatch> {
        let (output, report) = fit_annotation_ink_pdf(
            &self.current,
            page,
            annotation_index,
            options,
            signature_policy_override,
        )?;
        self.commit("ink_fit", output, prompt20_report_json_value(report)?)
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
                "prompt20 undo fingerprint or patch boundary mismatch".to_string(),
            ));
        }
        self.current.truncate(patch.before_bytes);
        if format!("{:x}", Sha256::digest(&self.current)) != patch.before_sha256 {
            return Err(WellfriendError::MalformedPdf(
                "prompt20 undo did not restore the recorded checkpoint digest".to_string(),
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
                "prompt20 redo fingerprint mismatch".to_string(),
            ));
        }
        self.current.extend_from_slice(&patch.appended_suffix);
        if format!("{:x}", Sha256::digest(&self.current)) != patch.after_sha256 {
            return Err(WellfriendError::MalformedPdf(
                "prompt20 redo did not restore the recorded patch digest".to_string(),
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
    ) -> Result<&Prompt20MutationPatch> {
        if !output.starts_with(&self.current) {
            return Err(WellfriendError::MalformedPdf(
                "prompt20 transaction output is not an incremental prefix-preserving patch"
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
                "prompt20 transaction patch count exceeds limit {}",
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
                "prompt20 transaction suffix bytes exceed limit {}",
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
        let patch = Prompt20MutationPatch {
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
        self.checkpoints.push(Prompt20MutationCheckpoint {
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

fn prompt20_report_json_value<T: Serialize>(report: T) -> Result<serde_json::Value> {
    serde_json::to_value(report).map_err(|error| {
        WellfriendError::ParseError(format!(
            "prompt20 transaction report serialization failed: {error}"
        ))
    })
}

fn prompt20_cache_invalidation(
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{OutputObject, PdfWriter};

    fn prompt20_fixture(include_ink: bool) -> Vec<u8> {
        prompt20_fixture_with_content(
            include_ink,
            b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n2 w 1 0 0 RG 20 20 40 30 re S\n",
        )
    }

    fn prompt20_fixture_with_content(include_ink: bool, source_content: &[u8]) -> Vec<u8> {
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

    fn prompt20_link_fixture() -> Vec<u8> {
        let input = prompt20_fixture(false);
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

    fn shared_form_fixture() -> Vec<u8> {
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

    fn nested_shared_form_fixture() -> Vec<u8> {
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
        let ap = resolve_prompt20_dict(annotation_dict.get("AP"), reader).expect("AP dict");
        let parts = appearance.split('/').collect::<Vec<_>>();
        if parts.len() == 1 {
            return ap
                .get(parts[0])
                .and_then(PdfObject::as_reference)
                .expect("AP ref");
        }
        let states = resolve_prompt20_dict(ap.get(parts[0]), reader).expect("state dict");
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

    fn bare_vector_fixture(content: &[u8]) -> Vec<u8> {
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
            Prompt20SupportStatus::UnsupportedReportedExact
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
            max_recursion: MAX_PROMPT20_FIT_RECURSION + 1,
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
        let input = prompt20_fixture(false);
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
        let input = prompt20_fixture_with_content(
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
        let input = prompt20_fixture_with_content(
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
        let mut session = Prompt20MutationSession::new(input.clone()).expect("range session");
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
    fn multi_run_range_covers_style_font_rtl_vertical_and_fail_closed_boundaries() {
        let styled = prompt20_fixture_with_content(
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
        let err = edit_multi_run_text_range(&styled, &partial, None)
            .expect_err("partial-token range is unsupported");
        let message = err.to_string();
        assert!(message.contains("token-boundary") || message.contains("provenance-bearing"));
    }

    #[test]
    fn preserve_per_segment_replays_source_font_size_color_and_positions() {
        let input = prompt20_fixture_with_content(
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
        let input = prompt20_fixture_with_content(
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
    fn preserve_per_segment_moves_one_exact_mcid_wrapper_without_duplication() {
        let input = prompt20_fixture_with_content(
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
        assert!(String::from_utf8(source.data)
            .expect("source UTF-8")
            .contains("/Artifact BMC"));
        let generated_object = reopened
            .document()
            .reader()
            .get_object(
                page.contents.last().expect("generated stream").0,
                page.contents.last().expect("generated stream").1,
            )
            .expect("generated stream object");
        let generated = decode_stream_lossless_with_limits(
            &generated_object,
            reopened.document().reader(),
            &DecodeLimits::default(),
        )
        .expect("generated decode");
        let generated = String::from_utf8(generated.data).expect("generated UTF-8");
        assert!(generated.contains("/P << /MCID 7 >> BDC"));
        assert_eq!(generated.matches("/MCID 7").count(), 1);
    }

    #[test]
    fn preserve_per_segment_refuses_partial_mcid_tag_ownership_before_mutation() {
        let input = prompt20_fixture_with_content(
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
        let error = edit_multi_run_text_range(&input, &request, None)
            .expect_err("partial MCID ownership must refuse");
        assert!(error.to_string().contains("unselected text"));
        assert!(ContentEngine::open_bytes(input).is_ok());
    }

    #[test]
    fn explicit_link_annotation_rect_move_preserves_action_and_quadpoints() {
        let input = prompt20_link_fixture();
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
        let input = prompt20_link_fixture();
        let error =
            move_link_annotation_rect_pdf(&input, 1, 0, [0.0, 0.0, 1.0, 1.0], 1.0, 1.0, false)
                .expect_err("stale source rect must refuse");
        assert!(error.to_string().contains("stale_snapshot"));
        assert!(ContentEngine::open_bytes(input).is_ok());
    }

    #[test]
    fn preserve_per_segment_refuses_bidi_without_per_style_visual_shaping() {
        let input = prompt20_fixture_with_content(false, b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n");
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
        let error = edit_multi_run_text_range(&input, &request, None)
            .expect_err("bidi mixed-style boundary");
        assert!(error.to_string().contains("RTL or mixed-bidi"));
    }

    #[test]
    fn preserve_per_segment_refuses_source_text_clipping_mode() {
        let input = prompt20_fixture_with_content(
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
        let error = edit_multi_run_text_range(&input, &request, None)
            .expect_err("clipping source boundary");
        assert!(error.to_string().contains("text-clipping"));
    }

    #[test]
    fn preserve_per_segment_refuses_unserializable_source_color_space() {
        let input = prompt20_fixture_with_content(
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
        let error = edit_multi_run_text_range(&input, &request, None)
            .expect_err("color-space source boundary");
        assert!(error.to_string().contains("color-space"));
    }

    #[test]
    fn preserve_per_segment_replays_invisible_text_without_clipping_side_effects() {
        let input =
            prompt20_fixture_with_content(false, b"BT /F1 12 Tf 3 Tr 10 150 Td (ABC) Tj ET\n");
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
        let input = prompt20_fixture(false);
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
        assert_eq!(clone_report.clone_graph.len(), 1);
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
        let mut session = Prompt20MutationSession::new(input.clone()).expect("vector session");
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
        let input = prompt20_fixture(false);
        let mut session = Prompt20MutationSession::new(input.clone()).expect("mutation session");
        session
            .apply_same_width_patch(1, "ABC", "DEF", &SameWidthPatchOptions::default())
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
            .apply_same_width_patch(1, "ABC", "XYZ", &SameWidthPatchOptions::default())
            .expect("branch patch");
        assert_eq!(session.patches().len(), 1);
        assert_eq!(session.cursor(), 1);
        assert!(!session.redo().expect("redo cleared"));
        assert!(session.patches()[0].appended_bytes > 0);
        assert_eq!(session.checkpoints().len(), 1);
    }

    #[test]
    fn annotation_ink_fit_saves_cubic_appearance_and_raw_points() {
        let input = prompt20_fixture(true);
        let (output, report) =
            fit_annotation_ink_pdf(&input, 1, 0, &InkFitOptions::default(), false)
                .expect("annotation fit");
        assert!(output.starts_with(&input));
        assert!(report.raw_points_preserved);
        assert!(report.fitted_curves_stored);
        assert!(report.appearance_readback);
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
        let input = prompt20_fixture(false);
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
        let input = prompt20_fixture(false);
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
    fn vertical_reflow_uses_identity_v_and_column_layout() {
        let input = prompt20_fixture(false);
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
