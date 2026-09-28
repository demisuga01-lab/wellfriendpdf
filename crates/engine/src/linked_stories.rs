//! Revision-bound linked frames. Geometry/reading order is approved input, not
//! a claim that arbitrary PDF paint programs have a unique document structure.
use crate::advanced_editing::{
    analyze_multi_run_text_range, bind_story_frame, replace_story_frame, StoryFrameBinding,
    StoryPaintLine, StoryPaintStyleSpan,
};
use crate::editing_transactions::ApprovedFontAsset;
use crate::fonts::{ShapeOptions, TextDirection, TextShaper, WritingMode};
use crate::{ContentEngine, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const SAVED_STORY_SCHEMA_MAX: u32 = 8;

#[cfg(test)]
#[path = "story_figure_tests.rs"]
pub(crate) mod figure_tests;
#[path = "story_figures.rs"]
pub mod figures;
#[cfg(test)]
#[path = "story_hard_break_tests.rs"]
mod hard_break_tests;
#[path = "story_history_checkpoint.rs"]
pub mod history;
#[cfg(test)]
#[path = "story_line_break_tests.rs"]
mod line_break_tests;
#[cfg(test)]
#[path = "story_line_composition_tests.rs"]
mod line_composition_tests;
#[cfg(test)]
#[path = "story_page_break_tests.rs"]
mod page_break_tests;
#[path = "story_page_pruning.rs"]
pub mod page_pruning;
#[cfg(test)]
#[path = "story_shaping_context_tests.rs"]
mod shaping_context_tests;
#[path = "table_layout.rs"]
pub mod tables;
#[path = "story_writing_mode.rs"]
mod writing_mode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFrame {
    pub id: String,
    pub page: usize,
    pub logical_range: [usize; 2],
    pub expected_text: String,
    pub rect: [f64; 4],
    /// Pinned artwork, furniture, tables or widgets that must not be covered.
    #[serde(default)]
    pub exclusions: Vec<[f64; 4]>,
    /// Supplied by a successful save/reopen. Never synthesize this from words.
    #[serde(default)]
    pub owner: Option<StoryFrameBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryInlineStyleSpan {
    /// UTF-8 byte offsets in the paragraph, always on grapheme boundaries.
    pub logical_range: [usize; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shaping: Option<crate::fonts::shaper::OpenTypeSettings>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoryParagraph {
    pub id: String,
    pub text: String,
    pub preferred_font: String,
    pub font_size: f64,
    pub line_height: f64,
    #[serde(default)]
    pub rgb: [f64; 3],
    #[serde(default)]
    pub rtl: bool,
    #[serde(default)]
    pub keep_with_next: bool,
    #[serde(default)]
    pub keep_together: bool,
    #[serde(default)]
    pub break_before: bool,
    /// A physical-page boundary. `break_before` remains the older next-frame
    /// policy, so multi-column documents can distinguish column and page flow.
    #[serde(default, skip_serializing_if = "StoryPageBreakBefore::is_none")]
    pub page_break_before: StoryPageBreakBefore,
    #[serde(default = "two")]
    pub orphans: usize,
    #[serde(default = "two")]
    pub widows: usize,
    #[serde(default)]
    pub space_before: f64,
    #[serde(default)]
    pub space_after: f64,
    #[serde(default)]
    pub shaping: crate::fonts::shaper::OpenTypeSettings,
    /// Canonical, sorted, non-overlapping effective overrides. Empty gaps
    /// inherit the paragraph fields. Supported horizontal/vertical stories and
    /// table cells materialize these runs through the shared style itemizer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_styles: Vec<StoryInlineStyleSpan>,
    /// Logical wrap rules, shared by horizontal/vertical text and table cells.
    /// Omitted defaults retain the previous serialized story fingerprint.
    #[serde(
        default,
        skip_serializing_if = "crate::fonts::line_break_policy::LineBreakSettings::is_default"
    )]
    pub line_break: crate::fonts::line_break_policy::LineBreakSettings,
    /// Exact inline-axis tab positions. U+0009 remains logical text and is
    /// emitted as positioned fields rather than a font glyph or spaces.
    #[serde(
        default,
        skip_serializing_if = "crate::fonts::tab_stops::TabStops::is_default"
    )]
    pub tab_stops: crate::fonts::tab_stops::TabStops,
}
fn two() -> usize {
    2
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StoryPageBreakBefore {
    #[default]
    None,
    NextPage,
    NextOddPage,
    NextEvenPage,
}
impl StoryPageBreakBefore {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
    fn accepts(self, page: usize) -> bool {
        match self {
            Self::None | Self::NextPage => true,
            Self::NextOddPage => page % 2 == 1,
            Self::NextEvenPage => page.is_multiple_of(2),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryPageBreakSource {
    ParagraphPolicy,
    FormFeed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryPageBreakReceipt {
    pub paragraph_id: String,
    /// UTF-8 byte offset in the paragraph. For a paragraph policy this is zero;
    /// for form feed it identifies the exact U+000C source byte.
    pub byte_offset: usize,
    pub source: StoryPageBreakSource,
    pub policy: StoryPageBreakBefore,
    pub from_page: usize,
    pub to_page: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StoryMode {
    PreserveLayout,
    #[default]
    FlowDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedStoryRequest {
    /// Uniform story inline/block axes. All public rectangles and annotation
    /// offsets remain physical PDF coordinates; omission preserves horizontal
    /// behavior for requests saved before writing-mode support.
    /// Inline/block progression. Bidi and mixed glyph orientation remain
    /// paragraph/shaping properties; this does not rotate the entire page.
    #[serde(default)]
    pub writing_mode: WritingMode,
    pub story_id: String,
    /// SHA-256 of the exact input bytes. Source ranges are never replayed onto
    /// another revision merely because matching words still exist.
    pub input_sha256: String,
    pub frames: Vec<StoryFrame>,
    pub paragraphs: Vec<StoryParagraph>,
    pub fonts: Vec<ApprovedFontAsset>,
    #[serde(default)]
    pub annotation_anchors: Vec<crate::story_anchors::StoryAnnotationAnchor>,
    /// Native image blocks kept with their explicitly associated caption.
    #[serde(default)]
    pub figures: Vec<figures::StoryFigure>,
    /// One-shot deletion commands for exact saved figure owners. Omission from
    /// `figures` alone never authorizes removing PDF content.
    #[serde(default)]
    pub figure_removals: Vec<figures::StoryFigureRemoval>,
    /// One-shot ownership-release commands. Unlike removal, detachment leaves
    /// the exact current native image owner painted in place.
    #[serde(default)]
    pub figure_detachments: Vec<figures::StoryFigureDetachment>,
    #[serde(default)]
    pub source_tags: Option<crate::tagged_structure::story::StoryTagging>,
    /// Approved table topology using the same source frames/fonts/transaction.
    #[serde(default)]
    pub table_layout: Option<tables::TableLayout>,
    #[serde(default)]
    pub allow_font_substitution: bool,
    #[serde(default)]
    pub allow_page_creation: bool,
    /// Remove only empty pages previously created and still owned by this story.
    /// Eligibility and retained-page reasons are bound into the preview receipt.
    #[serde(default)]
    pub prune_empty_pages: bool,
    #[serde(default = "default_page_limit")]
    pub max_new_pages: usize,
    #[serde(default)]
    pub mode: StoryMode,
    #[serde(default)]
    pub signature_policy_override: bool,
}
fn default_page_limit() -> usize {
    64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFontChoice {
    #[serde(default)]
    pub font_index: usize,
    #[serde(default)]
    pub logical_byte_range: Option<[usize; 2]>,
    pub paragraph_id: String,
    pub requested: String,
    pub selected: String,
    pub font_sha256: String,
    pub substituted: bool,
    pub metric_distance: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFrameLayout {
    pub frame: StoryFrame,
    pub created: bool,
    pub lines: Vec<StoryPaintLine>,
    pub paragraph_ids: Vec<String>,
    #[serde(default)]
    pub decorations: Vec<crate::advanced_editing::StoryDecoration>,
    #[serde(default)]
    pub table_cells: Vec<tables::TableCellFragment>,
    #[serde(default)]
    pub figures: Vec<figures::StoryFigurePlacement>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedStoryPreview {
    pub input_sha256: String,
    pub story_id: String,
    pub frames: Vec<StoryFrameLayout>,
    pub font_choices: Vec<StoryFontChoice>,
    pub changed_pages: Vec<usize>,
    pub generated_pages: usize,
    /// Every authoritative physical-page transition, including U+000C. This is
    /// covered by the preview receipt and therefore cannot change at apply time.
    #[serde(default)]
    pub page_breaks: Vec<StoryPageBreakReceipt>,
    #[serde(default)]
    pub page_pruning: page_pruning::StoryPagePruning,
    #[serde(default)]
    pub anchor_moves: Vec<crate::story_anchors::StoryAnnotationMove>,
    #[serde(default)]
    pub figure_removals: Vec<figures::StoryFigureRemoval>,
    #[serde(default)]
    pub figure_detachments: Vec<figures::StoryFigureDetachment>,
    /// Page indexes after an insertion can refer to entirely different content.
    /// Tile consumers must invalidate these whole output-page rectangles.
    #[serde(default)]
    pub full_page_invalidations: Vec<(usize, [f64; 4])>,
    pub qualification: String,
    pub exact_limits: Vec<String>,
    /// Set only after successful serialization and ownership verification.
    #[serde(default)]
    pub rebound_frames: Vec<StoryFrame>,
    #[serde(default)]
    pub output_sha256: Option<String>,
    #[serde(default)]
    pub reused_paragraphs: usize,
    #[serde(skip)]
    checkpoints: Vec<LayoutCheckpoint>,
}

/// An explicit acknowledgement of one exact preview, not a reusable grant to
/// mutate a document. Any request, preview, checkpoint or undo change invalidates
/// it. Hosts must still authenticate the person issuing the acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryPreviewReceipt {
    pub revision_sha256: String,
    pub request_sha256: String,
    pub preview_sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
struct LayoutCheckpoint {
    frame_index: usize,
    y: f64,
    frames_len: usize,
    generated: usize,
    retained_lines: usize,
    retained_figures: usize,
    retained_page_breaks: usize,
}
struct LayoutSeed<'a> {
    previous: &'a LinkedStoryPreview,
    paragraph_hashes: &'a [String],
    font_indices: &'a [usize],
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn value_hash(value: &impl Serialize) -> Result<String> {
    struct DigestWriter(Sha256);
    impl std::io::Write for DigestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = DigestWriter(Sha256::new());
    serde_json::to_writer(&mut writer, value).map_err(|e| fail(&e.to_string()))?;
    Ok(format!("{:x}", writer.0.finalize()))
}
fn layout_context_key(request: &LinkedStoryRequest) -> Result<String> {
    value_hash(&(
        (
            &request.input_sha256,
            &request.story_id,
            &request.frames,
            &request.fonts,
            &request.annotation_anchors,
            &request.figures,
            &request.figure_removals,
            &request.figure_detachments,
            &request.source_tags,
        ),
        (
            &request.table_layout,
            request.allow_font_substitution,
            request.allow_page_creation,
            request.prune_empty_pages,
            request.max_new_pages,
            request.mode,
            request.writing_mode,
            request.signature_policy_override,
        ),
    ))
}
pub(crate) fn frame_key(story: &str, frame: &str) -> String {
    let mut digest = Sha256::new();
    digest.update((story.len() as u64).to_le_bytes());
    digest.update(story.as_bytes());
    digest.update(frame.as_bytes());
    format!("{:x}", digest.finalize())
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}
fn rect_valid(r: &[f64; 4]) -> bool {
    r.iter().all(|v| v.is_finite()) && r[0] < r[2] && r[1] < r[3]
}
fn overlaps(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
}

fn validate(input: &[u8], request: &LinkedStoryRequest) -> Result<()> {
    if hash(input) != request.input_sha256 {
        return Err(fail("linked story belongs to another PDF revision"));
    }
    if request.story_id.is_empty()
        || request.frames.is_empty()
        || request.frames.len() > 4096
        || request.paragraphs.len() > 100_000
        || request.fonts.len() > 128
        || request.max_new_pages > 4096
    {
        return Err(fail("linked story size/identity limits exceeded"));
    }
    if request
        .paragraphs
        .iter()
        .map(|p| p.text.len())
        .sum::<usize>()
        > 4_000_000
        || request.fonts.iter().map(|f| f.bytes.len()).sum::<usize>() > 256 * 1024 * 1024
    {
        return Err(fail("linked story text/font byte budget exceeded"));
    }
    if request.mode == StoryMode::PreserveLayout && request.frames.len() != 1 {
        return Err(fail(
            "preserve-layout mode requires exactly one approved frame",
        ));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let policy = crate::secure_mutation::analyze_edit_policy(
        &engine,
        crate::secure_mutation::EditOperation::ContentEdit,
    )?;
    if request.writing_mode == WritingMode::VerticalLr
        && crate::tagged_structure::story::active(request)
    {
        let reader = engine.document().reader();
        let catalog_version = reader
            .root_reference()
            .and_then(|(n, g)| reader.get_object(n, g).ok())
            .and_then(|v| {
                v.as_dict()
                    .and_then(|d| d.get_name("Version"))
                    .map(str::to_owned)
            });
        if reader
            .version()
            .split('.')
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0)
            < 2
            && catalog_version
                .as_deref()
                .and_then(|v| v.split('.').next())
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
                < 2
        {
            return Err(WellfriendError::UnsupportedFeature("tagged vertical-lr uses PDF 2.0 WritingMode TbLr; an explicit PDF-version migration is required before this edit".into()));
        }
    }
    if (request.allow_page_creation || request.prune_empty_pages)
        && policy.impact.signature_count > 0
        && !request.signature_policy_override
    {
        return Err(WellfriendError::UnsupportedFeature(
            "continuation-page creation/removal requires explicit signed-document rewrite approval"
                .into(),
        ));
    }
    if !request.signature_policy_override
        && !matches!(
            policy.decision,
            crate::secure_mutation::EditPolicyDecision::SafeIncremental
                | crate::secure_mutation::EditPolicyDecision::IncrementalWithWarning
        )
    {
        return Err(WellfriendError::UnsupportedFeature(
            "story requires signature-policy approval".into(),
        ));
    }
    crate::tagged_structure::story::validate_selection(input, request)?;
    figures::validate(input, request)?;
    let mut ids = BTreeSet::new();
    let mut models = BTreeMap::new();
    let owned_frames = if request.frames.iter().any(|f| f.owner.is_some()) {
        crate::advanced_editing::story_frame_inventory(&engine)?
    } else {
        BTreeMap::new()
    };
    for (index, frame) in request.frames.iter().enumerate() {
        crate::cancel::check_current_cancel("story source binding")?;
        if frame.id.is_empty()
            || !ids.insert(frame.id.clone())
            || !rect_valid(&frame.rect)
            || frame.logical_range[0] > frame.logical_range[1]
            || frame.exclusions.len() > 4096
        {
            return Err(fail("invalid story frame"));
        }
        let bounds = engine.document().get_page(frame.page)?.crop_box;
        if frame.rect[0] < bounds[0]
            || frame.rect[1] < bounds[1]
            || frame.rect[2] > bounds[2]
            || frame.rect[3] > bounds[3]
            || frame.exclusions.iter().any(|r| !rect_valid(r))
        {
            return Err(fail("story frame lies outside the visible page"));
        }
        if index > 0 && frame.page < request.frames[index - 1].page {
            return Err(fail("story frames must follow approved page order"));
        }
        for earlier in &request.frames[..index] {
            if earlier.page == frame.page
                && (overlaps(&earlier.rect, &frame.rect)
                    || earlier.owner.is_none()
                        && frame.owner.is_none()
                        && earlier.logical_range[0] < frame.logical_range[1]
                        && frame.logical_range[0] < earlier.logical_range[1]
                    || earlier.owner.is_none()
                        && frame.owner.is_none()
                        && earlier.logical_range == frame.logical_range)
            {
                return Err(fail("story frame geometry/source ranges overlap"));
            }
        }
        if let Some(owner) = &frame.owner {
            if owner.key != frame_key(&request.story_id, &frame.id)
                || owned_frames.get(&owner.key) != Some(&(frame.page, owner.clone()))
            {
                return Err(fail("saved story ownership is stale or ambiguous"));
            }
            // The owner wrapper survives empty text and canonical renumbering;
            // old logical indices are diagnostic only on this path.
            continue;
        }
        if let std::collections::btree_map::Entry::Vacant(e) = models.entry(frame.page) {
            e.insert(analyze_multi_run_text_range(input, frame.page)?);
        }
        let model = &models[&frame.page];
        if frame.logical_range[1] > model.logical_text.chars().count() {
            return Err(fail("story frame range exceeds page text"));
        }
        let actual: String = model
            .logical_text
            .chars()
            .skip(frame.logical_range[0])
            .take(frame.logical_range[1] - frame.logical_range[0])
            .collect();
        if actual != frame.expected_text {
            return Err(fail("story frame source text mismatch"));
        }
        if model.source_spans.iter().any(|s| {
            s.logical_range[0] < frame.logical_range[1]
                && frame.logical_range[0] < s.logical_range[1]
                && (s.text_render_mode >= 4
                    || (!s.flow_relocatable && !crate::tagged_structure::story::active(request)))
        }) {
            return Err(WellfriendError::UnsupportedFeature(
                "story source has clipping or unsupported marked-content ownership".into(),
            ));
        }
    }
    validate_paragraphs(request)?;
    tables::validate_source(input, request)
}

pub(crate) fn validate_paragraphs(request: &LinkedStoryRequest) -> Result<()> {
    tables::validate_topology(request)?;
    if request.paragraphs.len() > 100_000
        || request
            .paragraphs
            .iter()
            .map(|p| p.text.len())
            .sum::<usize>()
            > 4_000_000
    {
        return Err(fail("story paragraph budget exceeded"));
    }
    let mut ids = BTreeSet::new();
    let figure_captions = request
        .figures
        .iter()
        .map(|figure| figure.caption_paragraph.as_str())
        .collect::<BTreeSet<_>>();
    for (index, p) in request.paragraphs.iter().enumerate() {
        p.line_break.validate()?;
        p.tab_stops.validate()?;
        if p.id.is_empty()
            || !ids.insert(p.id.clone())
            || !p.font_size.is_finite()
            || p.font_size < 0.01
            || !p.line_height.is_finite()
            // Tight or even overlapping leading is legal typography. It may
            // be visually undesirable, but it is not malformed and style-only
            // collaborative edits must not be rejected merely because the
            // chosen font size exceeds the existing baseline distance.
            || p.line_height <= 0.0
            || p.line_height > 10_000.0
            || !p.space_before.is_finite()
            || p.space_before < 0.0
            || !p.space_after.is_finite()
            || p.space_after < 0.0
            || p.rgb
                .iter()
                .any(|c| !c.is_finite() || !(0.0..=1.0).contains(c))
            || p.orphans == 0
            || p.widows == 0
            || p.orphans > 100
            || p.widows > 100
            || p.shaping.features.len() > 64
            || p.shaping.language.as_ref().is_some_and(|s| s.len() > 128)
            || p.shaping.features.iter().any(|s| s.len() > 128)
        {
            return Err(fail("invalid paragraph style or pagination constraints"));
        }
        validate_inline_style_spans(p)?;
        let has_form_feed = p.text.chars().any(crate::fonts::hard_break::is_form_feed);
        if p.break_before && !p.page_break_before.is_none() {
            return Err(fail(
                "a paragraph cannot request both a next-frame and physical-page break",
            ));
        }
        if request.mode == StoryMode::PreserveLayout
            && (!p.page_break_before.is_none() || has_form_feed)
        {
            return Err(WellfriendError::UnsupportedFeature(
                "physical page breaks require flow-document mode".into(),
            ));
        }
        if has_form_feed && p.keep_together {
            return Err(fail(
                "keep-together conflicts with an authoritative form-feed page boundary",
            ));
        }
        if p.keep_with_next
            && p.text
                .chars()
                .next_back()
                .is_some_and(crate::fonts::hard_break::is_form_feed)
        {
            return Err(fail(
                "keep-with-next conflicts with a trailing form-feed page boundary",
            ));
        }
        if has_form_feed && figure_captions.contains(p.id.as_str()) {
            return Err(fail(
                "a figure caption cannot contain a physical-page boundary; split it into an ordinary paragraph before or after the figure",
            ));
        }
        if p.keep_with_next
            && request
                .paragraphs
                .get(index + 1)
                .is_some_and(|next| next.break_before || !next.page_break_before.is_none())
        {
            return Err(fail(
                "keep-with-next conflicts with the following explicit frame/page break",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_inline_style_spans(paragraph: &StoryParagraph) -> Result<()> {
    if paragraph.inline_styles.len() > 100_000 {
        return Err(fail("inline-style span budget exceeded"));
    }
    let grapheme_boundaries = paragraph
        .text
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(paragraph.text.len()))
        .collect::<BTreeSet<_>>();
    let mut previous_end = 0usize;
    for span in &paragraph.inline_styles {
        let [start, end] = span.logical_range;
        if start >= end
            || start < previous_end
            || end > paragraph.text.len()
            || !grapheme_boundaries.contains(&start)
            || !grapheme_boundaries.contains(&end)
            || span.preferred_font.as_ref().is_some_and(|name| {
                name.is_empty() || name.len() > 128 || name.bytes().any(|byte| byte < 0x20)
            })
            || span
                .font_size
                .is_some_and(|size| !size.is_finite() || !(0.01..=10_000.0).contains(&size))
            || span.rgb.is_some_and(|rgb| {
                rgb.iter()
                    .any(|component| !component.is_finite() || !(0.0..=1.0).contains(component))
            })
            || span.shaping.as_ref().is_some_and(|shaping| {
                shaping.features.len() > 64
                    || shaping
                        .language
                        .as_ref()
                        .is_some_and(|value| value.len() > 128)
                    || shaping.features.iter().any(|value| value.len() > 128)
            })
            || (span.preferred_font.is_none()
                && span.font_size.is_none()
                && span.rgb.is_none()
                && span.shaping.is_none())
        {
            return Err(fail("invalid inline-style span"));
        }
        previous_end = end;
    }
    Ok(())
}

fn resolve_fonts(
    input: &[u8],
    request: &LinkedStoryRequest,
) -> Result<(Vec<ApprovedFontAsset>, Vec<usize>, Vec<StoryFontChoice>)> {
    let fonts = resolve_font_pool(input, request)?;
    let (indices, choices) = choose_fonts(request, &fonts)?;
    Ok((fonts, indices, choices))
}

fn resolve_font_pool(input: &[u8], request: &LinkedStoryRequest) -> Result<Vec<ApprovedFontAsset>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let mut fonts = load_story_fonts(&engine, &request.story_id)?;
    for page in request
        .frames
        .iter()
        .map(|f| f.page)
        .collect::<BTreeSet<_>>()
    {
        let resources = engine.get_page_resources(page)?;
        for (resource, dict) in &resources.fonts {
            // Non-painting separator carriers are not substitution candidates
            // or user font assets. Their small subset is owned by the frame.
            if crate::advanced_editing::story_carriers::is_font(dict) {
                continue;
            }
            if let Some(bytes) =
                crate::fonts::provider::embedded_program(engine.document().reader(), dict)
            {
                let name = dict
                    .get("BaseFont")
                    .and_then(crate::PdfObject::as_name)
                    .unwrap_or(resource);
                let name = if name.as_bytes().get(6) == Some(&b'+') {
                    &name[7..]
                } else {
                    name
                };
                if !fonts.iter().any(|f| f.bytes == bytes) {
                    if fonts.len() >= 128
                        || fonts.iter().map(|f| f.bytes.len()).sum::<usize>() + bytes.len()
                            > 256 * 1024 * 1024
                    {
                        return Err(fail("embedded story font pool exceeds limits"));
                    }
                    fonts.push(ApprovedFontAsset {
                        lookup_name: name.into(),
                        bytes,
                    });
                }
            }
        }
    }
    fonts.extend(request.fonts.clone());
    if fonts.iter().map(|f| f.bytes.len()).sum::<usize>() > 256 * 1024 * 1024 {
        return Err(fail("resolved story font pool exceeds byte budget"));
    }
    if request.allow_font_substitution {
        for name in ["Helvetica", "Times-Roman", "Courier", "Symbol"] {
            if let Some(bytes) = crate::render::get_fallback_font(name) {
                if !fonts.iter().any(|f| f.bytes == bytes) {
                    fonts.push(ApprovedFontAsset {
                        lookup_name: name.into(),
                        bytes: bytes.to_vec(),
                    });
                }
            }
        }
    }
    if fonts.len() > 256 {
        return Err(fail("resolved font pool exceeds face limit"));
    }
    Ok(fonts)
}

#[derive(Clone, PartialEq)]
struct EffectiveTextStyle {
    range: [usize; 2],
    preferred_font: String,
    font_size: f64,
    rgb: [f64; 3],
    shaping: crate::fonts::shaper::OpenTypeSettings,
}

fn effective_text_styles(paragraph: &StoryParagraph) -> Vec<EffectiveTextStyle> {
    if paragraph.text.is_empty() {
        return vec![EffectiveTextStyle {
            range: [0, 0],
            preferred_font: paragraph.preferred_font.clone(),
            font_size: paragraph.font_size,
            rgb: paragraph.rgb,
            shaping: paragraph.shaping.clone(),
        }];
    }
    let mut boundaries = BTreeSet::from([0, paragraph.text.len()]);
    for span in &paragraph.inline_styles {
        boundaries.extend(span.logical_range);
    }
    let boundaries = boundaries.into_iter().collect::<Vec<_>>();
    let mut styles: Vec<EffectiveTextStyle> = Vec::new();
    for pair in boundaries.windows(2) {
        let range = [pair[0], pair[1]];
        let override_style = paragraph
            .inline_styles
            .iter()
            .find(|span| span.logical_range[0] <= range[0] && range[1] <= span.logical_range[1]);
        let mut style = EffectiveTextStyle {
            range,
            preferred_font: paragraph.preferred_font.clone(),
            font_size: paragraph.font_size,
            rgb: paragraph.rgb,
            shaping: paragraph.shaping.clone(),
        };
        if let Some(override_style) = override_style {
            if let Some(value) = &override_style.preferred_font {
                style.preferred_font = value.clone();
            }
            if let Some(value) = override_style.font_size {
                style.font_size = value;
            }
            if let Some(value) = override_style.rgb {
                style.rgb = value;
            }
            if let Some(value) = &override_style.shaping {
                style.shaping = value.clone();
            }
        }
        if let Some(previous) = styles.last_mut().filter(|previous| {
            previous.range[1] == style.range[0]
                && previous.preferred_font == style.preferred_font
                && previous.font_size == style.font_size
                && previous.rgb == style.rgb
                && previous.shaping == style.shaping
        }) {
            previous.range[1] = style.range[1];
        } else {
            styles.push(style);
        }
    }
    styles
}

fn line_paint_styles(
    paragraph: &StoryParagraph,
    range: std::ops::Range<usize>,
) -> Vec<StoryPaintStyleSpan> {
    if paragraph.inline_styles.is_empty() || range.is_empty() {
        return Vec::new();
    }
    effective_text_styles(paragraph)
        .into_iter()
        .filter_map(|style| {
            let start = style.range[0].max(range.start);
            let end = style.range[1].min(range.end);
            (start < end).then_some(StoryPaintStyleSpan {
                range: [start - range.start, end - range.start],
                font_size: style.font_size,
                rgb: style.rgb,
                shaping: style.shaping,
            })
        })
        .collect()
}

fn paint_style_ranges(styles: &[StoryPaintStyleSpan]) -> Vec<[usize; 2]> {
    styles.iter().map(|style| style.range).collect()
}

fn font_metric(face: &ttf_parser::Face<'_>) -> [f64; 4] {
    let upem = f64::from(face.units_per_em()).max(1.0);
    [
        f64::from(face.x_height().unwrap_or(0)) / upem,
        f64::from(face.weight().to_number()) / 1000.0,
        if face.is_italic() { 1.0 } else { 0.0 },
        ['n', 'M', '0', ' ']
            .iter()
            .filter_map(|character| face.glyph_index(*character))
            .filter_map(|glyph| face.glyph_hor_advance(glyph))
            .map(f64::from)
            .sum::<f64>()
            / upem,
    ]
}

#[allow(clippy::too_many_arguments)]
fn choose_font_segment(
    request: &LinkedStoryRequest,
    paragraph: &StoryParagraph,
    text: &str,
    style: &EffectiveTextStyle,
    global_range: Option<[usize; 2]>,
    fonts: &[ApprovedFontAsset],
    faces: &[Option<ttf_parser::Face<'_>>],
) -> Result<(usize, Vec<StoryFontChoice>)> {
    let reference = fonts
        .iter()
        .position(|font| font.lookup_name == style.preferred_font)
        .and_then(|index| faces[index].as_ref());
    let options = ShapeOptions {
        direction: Some(if paragraph.rtl {
            TextDirection::RightToLeft
        } else {
            TextDirection::LeftToRight
        }),
    };
    let logical_only = crate::fonts::logical_carrier::is_text(text);
    // Tabs are resolved by the line-layout engine. Use a byte-stable neutral
    // boundary for coverage assignment, then shape only the non-tab fields.
    let coverage_text = text.replace('\t', " ");
    let text = coverage_text.as_str();
    let mut candidates = Vec::new();
    let mut ranked = Vec::new();
    for (index, (font, face)) in fonts.iter().zip(faces).enumerate() {
        let Some(face) = face.as_ref() else {
            continue;
        };
        if font.bytes.starts_with(b"ttcf")
            || (face.tables().glyf.is_none()
                && !(font.bytes.starts_with(b"OTTO") && face.tables().cff.is_some()))
        {
            continue;
        }
        if !crate::fonts::pdf_embedding::editable_outline_embedding_allowed(face) {
            continue;
        }
        let exact = font.lookup_name == style.preferred_font;
        if !exact && !request.allow_font_substitution {
            continue;
        }
        let covered = if logical_only {
            true
        } else if request.writing_mode.is_vertical() {
            crate::fonts::vertical_fonts::covers(&font.bytes, text, options, &style.shaping)?
        } else {
            let shaped =
                TextShaper::shape_with_settings(&font.bytes, text, options, &style.shaping)?;
            !crate::fonts::shaper::has_missing_glyphs(&font.bytes, text, &shaped)?
        };
        let distance = reference
            .map(|reference| {
                font_metric(face)
                    .iter()
                    .zip(font_metric(reference))
                    .map(|(a, b)| (a - b).abs())
                    .sum::<f64>()
            })
            .unwrap_or(0.0);
        ranked.push((index, distance + if exact { 0.0 } else { 1.0 }));
        if covered {
            candidates.push((!exact, distance, index));
        }
    }
    candidates.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.total_cmp(&b.1))
            .then(a.2.cmp(&b.2))
    });
    if let Some((_, distance, index)) = candidates.first().copied() {
        return Ok((
            index,
            vec![StoryFontChoice {
                font_index: index,
                logical_byte_range: global_range,
                paragraph_id: paragraph.id.clone(),
                requested: style.preferred_font.clone(),
                selected: fonts[index].lookup_name.clone(),
                font_sha256: hash(&fonts[index].bytes),
                substituted: fonts[index].lookup_name != style.preferred_font,
                metric_distance: distance,
            }],
        ));
    }
    let spans = crate::fonts::fallback::resolve_contextual_fonts_for_mode(
        text,
        fonts,
        &ranked,
        options,
        &style.shaping,
        request.writing_mode,
    )?;
    let first = spans
        .first()
        .ok_or_else(|| fail("no approved font for empty paragraph"))?
        .font_index;
    let offset = global_range.map_or(0, |range| range[0]);
    let choices = spans
        .into_iter()
        .map(|span| {
            let index = span.font_index;
            StoryFontChoice {
                font_index: index,
                logical_byte_range: Some([span.range[0] + offset, span.range[1] + offset]),
                paragraph_id: paragraph.id.clone(),
                requested: style.preferred_font.clone(),
                selected: fonts[index].lookup_name.clone(),
                font_sha256: hash(&fonts[index].bytes),
                substituted: fonts[index].lookup_name != style.preferred_font,
                metric_distance: ranked.iter().find(|rank| rank.0 == index).unwrap().1,
            }
        })
        .collect();
    Ok((first, choices))
}

fn choose_fonts(
    request: &LinkedStoryRequest,
    fonts: &[ApprovedFontAsset],
) -> Result<(Vec<usize>, Vec<StoryFontChoice>)> {
    // Parse each immutable program once for this complete resolution pass.
    // Faces borrow the approved byte pool, so no font bytes or unbounded global
    // cache survive the request. Shaping/coverage still use their own bounded
    // caches, but ranking and embedding-policy checks no longer reparse every
    // font for every paragraph.
    let faces = fonts
        .iter()
        .enumerate()
        .map(|(index, font)| {
            if index % 32 == 0 {
                crate::cancel::check_current_cancel("story font face preparation")?;
            }
            Ok(ttf_parser::Face::parse(&font.bytes, 0).ok())
        })
        .collect::<Result<Vec<_>>>()?;
    let mut indices = Vec::new();
    let mut choices = Vec::new();
    for p in &request.paragraphs {
        crate::cancel::check_current_cancel("story font resolution")?;
        let styles = effective_text_styles(p);
        let mut first = None;
        for style in &styles {
            let explicit_range = (!p.inline_styles.is_empty()).then_some(style.range);
            let (selected, mut segment_choices) = choose_font_segment(
                request,
                p,
                &p.text[style.range[0]..style.range[1]],
                style,
                explicit_range,
                fonts,
                &faces,
            )?;
            first.get_or_insert(selected);
            choices.append(&mut segment_choices);
        }
        indices.push(first.ok_or_else(|| fail("paragraph has no effective style run"))?);
    }
    if choices
        .iter()
        .map(|c| c.font_index)
        .collect::<BTreeSet<_>>()
        .len()
        > 128
    {
        return Err(fail(
            "a saved story may own at most 128 distinct font programs",
        ));
    }
    Ok((indices, choices))
}

/// Measure exactly the resolved font runs used by final PDF emission.
fn measure_story_line(
    prepared: &crate::fonts::line_layout::PreparedParagraph<'_>,
    spans: &[crate::fonts::fallback::FontSpan],
    fonts: &[ApprovedFontAsset],
    font_metrics: &[Option<crate::fonts::line_layout::PreparedFontMetrics<'_>>],
    p: &StoryParagraph,
    writing_mode: WritingMode,
    range: std::ops::Range<usize>,
) -> Result<crate::fonts::line_layout::LineMetrics> {
    let visible = p.text[range.clone()].trim_end_matches(crate::fonts::hard_break::is_hard_break);
    let range = range.start..range.start + visible.len();
    if visible.contains('\t') {
        let plan =
            crate::fonts::tab_stops::plan_line(&p.text, range.clone(), &p.tab_stops, |segment| {
                Ok(measure_story_line(
                    prepared,
                    spans,
                    fonts,
                    font_metrics,
                    p,
                    writing_mode,
                    segment,
                )?
                .width())
            })?;
        let mut ascent = 0.0f64;
        let mut descent = 0.0f64;
        for segment in &plan.segments {
            let metrics = measure_story_line(
                prepared,
                spans,
                fonts,
                font_metrics,
                p,
                writing_mode,
                segment.range.clone(),
            )?;
            ascent = ascent.max(metrics.ascent);
            descent = descent.max(metrics.descent);
        }
        return Ok(crate::fonts::line_layout::LineMetrics {
            advance: plan.advance,
            left_pad: 0.0,
            right_pad: 0.0,
            ascent,
            descent,
        });
    }
    let bidi = prepared.bidi.line(range.clone())?;
    let style_spans = line_paint_styles(p, range.clone());
    let sliced = crate::fonts::fallback::slice_spans(spans, range);
    if writing_mode.is_vertical() {
        if !style_spans.is_empty() {
            let styled_spans = crate::fonts::fallback::intersect_styled_spans(
                &sliced,
                &paint_style_ranges(&style_spans),
                visible.len(),
            )?;
            let settings = style_spans
                .iter()
                .map(|style| style.shaping.clone())
                .collect::<Vec<_>>();
            let sizes = style_spans
                .iter()
                .map(|style| style.font_size)
                .collect::<Vec<_>>();
            let runs = crate::fonts::vertical_fonts::shape_styled_line_prepared(
                visible,
                &bidi,
                &styled_spans,
                fonts,
                &settings,
                font_metrics,
            )?;
            return crate::fonts::vertical_fonts::measure_styled_line_prepared(
                &runs,
                fonts,
                font_metrics,
                &sizes,
                writing_mode,
            );
        }
        let runs = crate::fonts::vertical_fonts::shape_line_prepared(
            visible,
            &bidi,
            &sliced,
            fonts,
            &p.shaping,
            font_metrics,
        )?;
        return crate::fonts::vertical_fonts::measure_line_prepared(
            &runs,
            fonts,
            font_metrics,
            p.font_size,
            writing_mode,
        );
    }
    if visible.is_empty() {
        let index = spans.first().map_or(0, |s| s.font_index);
        let run = TextShaper::shape_resolved(&fonts[index].bytes, visible, &bidi, &p.shaping)?;
        return font_metrics
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| fail("story line font metrics are not prepared"))?
            .measure(&run, p.font_size);
    }
    if !style_spans.is_empty() {
        let styled_spans = crate::fonts::fallback::intersect_styled_spans(
            &sliced,
            &paint_style_ranges(&style_spans),
            visible.len(),
        )?;
        let settings = style_spans
            .iter()
            .map(|style| style.shaping.clone())
            .collect::<Vec<_>>();
        let sizes = style_spans
            .iter()
            .map(|style| style.font_size)
            .collect::<Vec<_>>();
        let runs = crate::fonts::fallback::shape_styled_line(
            visible,
            &bidi,
            &styled_spans,
            fonts,
            &settings,
        )?;
        return crate::fonts::fallback::measure_styled_line_prepared(&runs, font_metrics, &sizes);
    }
    let runs = crate::fonts::fallback::shape_line(visible, &bidi, &sliced, fonts, &p.shaping)?;
    crate::fonts::fallback::measure_line_prepared(&runs, font_metrics, p.font_size)
}

pub(crate) struct StoryTabPlan {
    pub segments: Vec<crate::advanced_editing::StoryTabSegment>,
    pub decorations: Vec<crate::fonts::tab_stops::PositionedTabDecoration>,
}

pub(crate) fn story_tab_plan(
    prepared: &crate::fonts::line_layout::PreparedParagraph<'_>,
    spans: &[crate::fonts::fallback::FontSpan],
    fonts: &[ApprovedFontAsset],
    font_metrics: &[Option<crate::fonts::line_layout::PreparedFontMetrics<'_>>],
    paragraph: &StoryParagraph,
    writing_mode: WritingMode,
    range: std::ops::Range<usize>,
) -> Result<StoryTabPlan> {
    let visible_end = range.end
        - paragraph.text[range.clone()]
            .chars()
            .rev()
            .take_while(|ch| crate::fonts::hard_break::is_hard_break(*ch))
            .map(char::len_utf8)
            .sum::<usize>();
    if !paragraph.text[range.start..visible_end].contains('\t') {
        return Ok(StoryTabPlan {
            segments: Vec::new(),
            decorations: Vec::new(),
        });
    }
    let plan = crate::fonts::tab_stops::plan_line(
        &paragraph.text,
        range.start..visible_end,
        &paragraph.tab_stops,
        |segment| {
            Ok(measure_story_line(
                prepared,
                spans,
                fonts,
                font_metrics,
                paragraph,
                writing_mode,
                segment,
            )?
            .width())
        },
    )?;
    let segments = plan
        .segments
        .into_iter()
        .map(|segment| {
            let metrics = measure_story_line(
                prepared,
                spans,
                fonts,
                font_metrics,
                paragraph,
                writing_mode,
                segment.range.clone(),
            )?;
            Ok(crate::advanced_editing::StoryTabSegment {
                range: [
                    segment.range.start - range.start,
                    segment.range.end - range.start,
                ],
                origin: segment.origin,
                width: segment.width,
                leading_pad: metrics.left_pad,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(StoryTabPlan {
        segments,
        decorations: plan.decorations,
    })
}
fn break_story_lines(
    prepared: &crate::fonts::line_layout::PreparedParagraph<'_>,
    spans: &[crate::fonts::fallback::FontSpan],
    fonts: &[ApprovedFontAsset],
    font_metrics: &[Option<crate::fonts::line_layout::PreparedFontMetrics<'_>>],
    p: &StoryParagraph,
    writing_mode: WritingMode,
    from: usize,
    width: f64,
    limit: usize,
) -> Result<Vec<crate::fonts::line_layout::MeasuredLine>> {
    prepared.break_lines_measured(from, width, limit, |range| {
        Ok(
            measure_story_line(prepared, spans, fonts, font_metrics, p, writing_mode, range)?
                .width(),
        )
    })
}

fn probe_story_lines(
    prepared: &crate::fonts::line_layout::PreparedParagraph<'_>,
    spans: &[crate::fonts::fallback::FontSpan],
    fonts: &[ApprovedFontAsset],
    font_metrics: &[Option<crate::fonts::line_layout::PreparedFontMetrics<'_>>],
    p: &StoryParagraph,
    writing_mode: WritingMode,
    from: usize,
    width: f64,
    limit: usize,
) -> Result<crate::fonts::line_layout::LineBreakBatch> {
    prepared.break_lines_measured_prefix(from, width, limit, |range| {
        Ok(
            measure_story_line(prepared, spans, fonts, font_metrics, p, writing_mode, range)?
                .width(),
        )
    })
}

/// Width-only lookahead: actual height/exclusions are checked when the chosen
/// continuation is placed. Equal widths share one probe; no new page is created
/// just to ask whether its template could provide the required widow lines.
fn continuation_widows_fit(
    prepared: &crate::fonts::line_layout::PreparedParagraph<'_>,
    spans: &[crate::fonts::fallback::FontSpan],
    fonts: &[ApprovedFontAsset],
    font_metrics: &[Option<crate::fonts::line_layout::PreparedFontMetrics<'_>>],
    p: &StoryParagraph,
    writing_mode: WritingMode,
    from: usize,
    later_frames: &[StoryFrameLayout],
    continuation_width: Option<f64>,
) -> Result<bool> {
    let mut seen = BTreeSet::new();
    for width in later_frames
        .iter()
        .map(|f| f.frame.rect[2] - f.frame.rect[0])
        .chain(continuation_width)
    {
        crate::cancel::check_current_cancel("story continuation width lookahead")?;
        if !seen.insert(width.to_bits()) {
            continue;
        }
        let batch = probe_story_lines(
            prepared,
            spans,
            fonts,
            font_metrics,
            p,
            writing_mode,
            from,
            width,
            p.widows,
        )?;
        if batch.lines.len() >= p.widows {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Reserve the following orphan minimum, complete keep-together paragraph, or
/// keep-with-next chain using the same metrics/exclusions as actual pagination.
fn following_block_fits(
    request: &LinkedStoryRequest,
    start: usize,
    fonts: &[ApprovedFontAsset],
    font_metrics: &[Option<crate::fonts::line_layout::PreparedFontMetrics<'_>>],
    frame: &StoryFrame,
    mut y: f64,
    prepared: &[crate::fonts::line_layout::PreparedParagraph<'_>],
    font_spans: &[Vec<crate::fonts::fallback::FontSpan>],
    paragraph_figures: &[Option<&figures::StoryFigure>],
) -> Result<bool> {
    for index in start..request.paragraphs.len() {
        crate::cancel::check_current_cancel("story keep chain layout")?;
        let p = &request.paragraphs[index];
        if p.break_before || !p.page_break_before.is_none() {
            return Ok(false);
        }
        let figure = paragraph_figures[index];
        let reservation = match figure {
            Some(f) => match figures::reserve(frame, p, y, f)? {
                Some(r) => Some(r),
                None => return Ok(false),
            },
            None => None,
        };
        let top = reservation
            .as_ref()
            .map_or(y - p.space_before, |r| r.caption_top);
        let left = reservation
            .as_ref()
            .map_or(frame.rect[0], |r| r.caption_left);
        let width = reservation
            .as_ref()
            .map_or(frame.rect[2] - frame.rect[0], |r| r.width);
        if p.text.is_empty() {
            y = top - p.space_after;
            if p.keep_with_next {
                continue;
            } else {
                return Ok(y >= frame.rect[1]);
            }
        }
        let lookahead = (((frame.rect[3] - frame.rect[1]) / p.line_height).ceil() as usize)
            .saturating_add(p.widows)
            .saturating_add(2)
            .min(100_000);
        let batch = probe_story_lines(
            &prepared[index],
            &font_spans[index],
            fonts,
            font_metrics,
            p,
            request.writing_mode,
            0,
            width,
            lookahead,
        )?;
        let lines = batch.lines;
        let whole = p.keep_together || p.keep_with_next || figure.is_some();
        if batch.blocked.is_some() && (whole || lines.len() < p.orphans) {
            return Ok(false);
        }
        if whole
            && lines
                .last()
                .is_some_and(|line| line.bytes.end < p.text.len())
        {
            return Ok(false);
        }
        let mut ascent = 0.0f64;
        let mut descent = 0.0f64;
        for line in &lines {
            let metric = measure_story_line(
                &prepared[index],
                &font_spans[index],
                fonts,
                font_metrics,
                p,
                request.writing_mode,
                line.bytes.clone(),
            )?;
            ascent = ascent.max(metric.ascent);
            descent = descent.max(metric.descent);
        }
        let height = p.line_height.max(ascent + descent);
        let count = if whole {
            lines.len()
        } else {
            p.orphans.min(lines.len())
        };
        let mut baseline = top - ascent;
        let mut last = baseline;
        for _ in 0..count {
            let mut rows = 0;
            loop {
                if baseline - descent < frame.rect[1] - 1e-7 {
                    return Ok(false);
                }
                let bounds = [left, baseline - descent, left + width, baseline + ascent];
                if !frame.exclusions.iter().any(|r| overlaps(r, &bounds)) {
                    break;
                }
                baseline -= height;
                rows += 1;
                if rows > 100_000 {
                    return Err(fail("keep-chain exclusion budget exceeded"));
                }
                if rows % 256 == 0 {
                    crate::cancel::check_current_cancel("story keep-chain exclusions")?;
                }
            }
            last = baseline;
            baseline -= height;
        }
        y = last - (height - ascent) - p.space_after;
        if !p.keep_with_next {
            return Ok(true);
        }
    }
    Ok(true)
}

fn layout(
    request: &LinkedStoryRequest,
    fonts: &[ApprovedFontAsset],
    indices: &[usize],
    choices: Vec<StoryFontChoice>,
) -> Result<LinkedStoryPreview> {
    layout_seeded(request, fonts, indices, choices, None)
}

fn ensure_story_frame(
    request: &LinkedStoryRequest,
    frames: &mut Vec<StoryFrameLayout>,
    generated: &mut usize,
    frame_index: usize,
) -> Result<()> {
    while frame_index >= frames.len() {
        if request.mode == StoryMode::PreserveLayout
            || !request.allow_page_creation
            || *generated >= request.max_new_pages
        {
            return Err(WellfriendError::UnsupportedFeature(
                "story overflows approved frames/page budget".into(),
            ));
        }
        let mut frame = frames.last().unwrap().frame.clone();
        frame.page += 1;
        *generated += 1;
        let mut suffix = frames.len() + *generated;
        loop {
            frame.id = format!("{}:continuation:{suffix}", request.story_id);
            if !frames.iter().any(|f| f.frame.id == frame.id) {
                break;
            }
            suffix += 1;
        }
        frame.logical_range = [0, 1];
        frame.expected_text = " ".into();
        frame.exclusions.clear();
        frame.owner = None;
        frames.push(StoryFrameLayout {
            frame,
            created: true,
            lines: Vec::new(),
            paragraph_ids: Vec::new(),
            decorations: Vec::new(),
            table_cells: Vec::new(),
            figures: Vec::new(),
        });
    }
    Ok(())
}

/// Move to the first approved frame on a later physical page that satisfies the
/// requested parity. If no such approved frame exists, append bounded owned
/// continuation pages. Same-page columns are deliberately skipped.
fn advance_story_page(
    request: &LinkedStoryRequest,
    frames: &mut Vec<StoryFrameLayout>,
    generated: &mut usize,
    frame_index: usize,
    policy: StoryPageBreakBefore,
) -> Result<(usize, usize, usize)> {
    if policy.is_none() {
        return Err(fail("physical-page advance requires a page-break policy"));
    }
    let from_page = frames
        .get(frame_index)
        .ok_or_else(|| fail("page break has no current story frame"))?
        .frame
        .page;
    loop {
        if let Some((index, target)) =
            frames
                .iter()
                .enumerate()
                .skip(frame_index + 1)
                .find(|(_, candidate)| {
                    candidate.frame.page > from_page && policy.accepts(candidate.frame.page)
                })
        {
            return Ok((index, from_page, target.frame.page));
        }
        let next = frames.len();
        ensure_story_frame(request, frames, generated, next)?;
        let page = frames[next].frame.page;
        if page > from_page && policy.accepts(page) {
            return Ok((next, from_page, page));
        }
        crate::cancel::check_current_cancel("story page-break page creation")?;
    }
}

fn layout_seeded(
    request: &LinkedStoryRequest,
    fonts: &[ApprovedFontAsset],
    indices: &[usize],
    choices: Vec<StoryFontChoice>,
    seed: Option<LayoutSeed<'_>>,
) -> Result<LinkedStoryPreview> {
    writing_mode::layout(request, fonts, indices, choices, seed)
}

fn layout_seeded_flow(
    request: &LinkedStoryRequest,
    fonts: &[ApprovedFontAsset],
    indices: &[usize],
    choices: Vec<StoryFontChoice>,
    seed: Option<LayoutSeed<'_>>,
) -> Result<LinkedStoryPreview> {
    if request.table_layout.is_some() {
        return tables::layout_table(request, fonts, indices, choices);
    }
    let figures_by_caption = request
        .figures
        .iter()
        .map(|f| (f.caption_paragraph.as_str(), f))
        .collect::<BTreeMap<_, _>>();
    let paragraph_figures = request
        .paragraphs
        .iter()
        .map(|p| figures_by_caption.get(p.id.as_str()).copied())
        .collect::<Vec<_>>();
    let prepared = request
        .paragraphs
        .iter()
        .map(|p| {
            crate::fonts::line_layout::PreparedParagraph::with_break_settings(
                &p.text,
                ShapeOptions {
                    direction: Some(if p.rtl {
                        TextDirection::RightToLeft
                    } else {
                        TextDirection::LeftToRight
                    }),
                },
                &p.line_break,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let font_spans = request
        .paragraphs
        .iter()
        .enumerate()
        .map(|(index, p)| {
            let spans = choices
                .iter()
                .filter(|c| c.paragraph_id == p.id)
                .filter_map(|c| {
                    c.logical_byte_range
                        .map(|range| crate::fonts::fallback::FontSpan {
                            range,
                            font_index: c.font_index,
                        })
                })
                .collect::<Vec<_>>();
            if spans.is_empty() {
                vec![crate::fonts::fallback::FontSpan {
                    range: [0, p.text.len()],
                    font_index: indices[index],
                }]
            } else {
                spans
            }
        })
        .collect::<Vec<_>>();
    let used_font_indices = font_spans
        .iter()
        .flatten()
        .map(|span| span.font_index)
        .collect::<BTreeSet<_>>();
    let font_metrics = fonts
        .iter()
        .enumerate()
        .map(|(index, font)| {
            if !used_font_indices.contains(&index) {
                return Ok(None);
            }
            crate::cancel::check_current_cancel("story font metrics preparation")?;
            crate::fonts::line_layout::PreparedFontMetrics::new(&font.bytes).map(Some)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut frames = request
        .frames
        .iter()
        .cloned()
        .map(|frame| StoryFrameLayout {
            frame,
            created: false,
            lines: Vec::new(),
            paragraph_ids: Vec::new(),
            decorations: Vec::new(),
            table_cells: Vec::new(),
            figures: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut frame_index = 0usize;
    let mut y = frames[0].frame.rect[3];
    let mut generated = 0usize;
    let mut page_breaks = Vec::new();
    let hashes = request
        .paragraphs
        .iter()
        .map(value_hash)
        .collect::<Result<Vec<_>>>()?;
    let mut checkpoints = Vec::new();
    let mut start_paragraph = 0;
    let mut reused = 0;
    if let Some(seed) = &seed {
        let unchanged = hashes
            .iter()
            .zip(seed.paragraph_hashes)
            .zip(indices.iter().zip(seed.font_indices))
            .take_while(|((a, b), (i, j))| a == b && i == j)
            .count();
        // A changed paragraph can affect its predecessor's keep-with-next rule.
        start_paragraph = unchanged.saturating_sub(1);
        while start_paragraph > 0 && request.paragraphs[start_paragraph - 1].keep_with_next {
            start_paragraph -= 1;
        }
        if let Some(state) = seed.previous.checkpoints.get(start_paragraph) {
            frames = seed.previous.frames[..state.frames_len].to_vec();
            frame_index = state.frame_index;
            y = state.y;
            generated = state.generated;
            page_breaks.extend_from_slice(
                &seed.previous.page_breaks[..state
                    .retained_page_breaks
                    .min(seed.previous.page_breaks.len())],
            );
            for (index, frame) in frames.iter_mut().enumerate().skip(frame_index) {
                let keep = if index == frame_index {
                    state.retained_lines
                } else {
                    0
                };
                frame.lines.truncate(keep);
                frame.paragraph_ids.truncate(keep);
                frame.figures.truncate(if index == frame_index {
                    state.retained_figures
                } else {
                    0
                });
            }
            checkpoints.extend_from_slice(&seed.previous.checkpoints[..start_paragraph]);
            reused = start_paragraph;
        } else {
            start_paragraph = 0;
        }
    }
    for (paragraph_index, p) in request.paragraphs.iter().enumerate().skip(start_paragraph) {
        let state = LayoutCheckpoint {
            frame_index,
            y,
            frames_len: frames.len(),
            generated,
            retained_lines: frames.get(frame_index).map_or(0, |f| f.lines.len()),
            retained_figures: frames.get(frame_index).map_or(0, |f| f.figures.len()),
            retained_page_breaks: page_breaks.len(),
        };
        if let Some(seed) = &seed {
            if paragraph_index > start_paragraph
                && seed.previous.checkpoints.get(paragraph_index) == Some(&state)
                && hashes.get(paragraph_index..) == seed.paragraph_hashes.get(paragraph_index..)
                && indices.get(paragraph_index..) == seed.font_indices.get(paragraph_index..)
            {
                // The boundary state and all following logical/style inputs
                // converged. Copy the unchanged suffix without shaping it again.
                if let Some(current) = frames.get_mut(frame_index) {
                    let old = &seed.previous.frames[frame_index];
                    current
                        .lines
                        .extend_from_slice(&old.lines[state.retained_lines..]);
                    current
                        .paragraph_ids
                        .extend_from_slice(&old.paragraph_ids[state.retained_lines..]);
                    current
                        .figures
                        .extend_from_slice(&old.figures[state.retained_figures..]);
                }
                frames.truncate((frame_index + 1).min(frames.len()));
                frames.extend_from_slice(
                    &seed.previous.frames[(frame_index + 1).min(seed.previous.frames.len())..],
                );
                checkpoints.extend_from_slice(&seed.previous.checkpoints[paragraph_index..]);
                page_breaks.extend_from_slice(
                    &seed.previous.page_breaks[state
                        .retained_page_breaks
                        .min(seed.previous.page_breaks.len())..],
                );
                generated = seed.previous.generated_pages;
                reused += request.paragraphs.len() - paragraph_index;
                break;
            }
        }
        checkpoints.push(state);
        if p.break_before && paragraph_index > 0 {
            frame_index += 1;
            y = f64::INFINITY;
        }
        if !p.page_break_before.is_none() {
            ensure_story_frame(request, &mut frames, &mut generated, frame_index)?;
            let (target, from_page, to_page) = advance_story_page(
                request,
                &mut frames,
                &mut generated,
                frame_index,
                p.page_break_before,
            )?;
            page_breaks.push(StoryPageBreakReceipt {
                paragraph_id: p.id.clone(),
                byte_offset: 0,
                source: StoryPageBreakSource::ParagraphPolicy,
                policy: p.page_break_before,
                from_page,
                to_page,
            });
            frame_index = target;
            y = f64::INFINITY;
        }
        // Materialize explicit empty breaks too, including trailing breaks.
        if p.text.is_empty() {
            if let Some(figure) = paragraph_figures[paragraph_index] {
                loop {
                    ensure_story_frame(request, &mut frames, &mut generated, frame_index)?;
                    if !y.is_finite() {
                        y = frames[frame_index].frame.rect[3];
                    }
                    let current = &frames[frame_index];
                    if let Some(reserved) = figures::reserve(&current.frame, p, y, figure)? {
                        let after = reserved.caption_top - p.space_after;
                        if after >= current.frame.rect[1] - 1e-7
                            && (!p.keep_with_next
                                || following_block_fits(
                                    request,
                                    paragraph_index + 1,
                                    fonts,
                                    &font_metrics,
                                    &current.frame,
                                    after,
                                    &prepared,
                                    &font_spans,
                                    &paragraph_figures,
                                )?)
                        {
                            frames[frame_index].figures.push(reserved.placement);
                            y = after;
                            break;
                        }
                    }
                    if frame_index + 1 >= frames.len()
                        && current.lines.is_empty()
                        && current.figures.is_empty()
                        && current.frame.exclusions.is_empty()
                        && (y - current.frame.rect[3]).abs() < 1e-7
                    {
                        return Err(fail(
                            "image block and keep chain cannot fit a permitted empty frame",
                        ));
                    }
                    frame_index += 1;
                    y = f64::INFINITY;
                }
                continue;
            }
            ensure_story_frame(request, &mut frames, &mut generated, frame_index)?;
            if !y.is_finite() {
                y = frames[frame_index].frame.rect[3];
            }
            y -= p.space_before;
            y -= p.space_after;
            continue;
        }
        let bidi_context = &prepared[paragraph_index].bidi;
        let mut consumed = 0usize;
        // Widow/orphan minima apply within each author-declared page segment.
        // A form feed starts a new segment; it is not accidental pagination.
        let mut segment_start = 0usize;
        let mut before_applied = false;
        while consumed < p.text.len() {
            crate::cancel::check_current_cancel("linked story pagination")?;
            ensure_story_frame(request, &mut frames, &mut generated, frame_index)?;
            let continuation_width = (request.mode != StoryMode::PreserveLayout
                && request.allow_page_creation
                && generated < request.max_new_pages)
                .then(|| {
                    let r = frames.last().unwrap().frame.rect;
                    r[2] - r[0]
                });
            let (active_frames, later_frames) = frames.split_at_mut(frame_index + 1);
            let has_later_approved_frame = !later_frames.is_empty();
            let current = &mut active_frames[frame_index];
            if !y.is_finite() {
                y = current.frame.rect[3];
            }
            let figure = paragraph_figures[paragraph_index];
            let reservation = if let Some(figure) = figure {
                if consumed != 0 {
                    return Err(fail("figure caption was split across frames"));
                }
                match figures::reserve(&current.frame, p, y, figure)? {
                    Some(r) => Some(r),
                    None => {
                        if !has_later_approved_frame
                            && current.lines.is_empty()
                            && current.figures.is_empty()
                            && current.frame.exclusions.is_empty()
                            && (y - current.frame.rect[3]).abs() < 1e-7
                        {
                            return Err(fail("image block cannot fit a permitted empty frame"));
                        }
                        frame_index += 1;
                        y = f64::INFINITY;
                        continue;
                    }
                }
            } else {
                None
            };
            let top = reservation.as_ref().map_or(
                y - if !before_applied { p.space_before } else { 0.0 },
                |r| r.caption_top,
            );
            let left = reservation
                .as_ref()
                .map_or(current.frame.rect[0], |r| r.caption_left);
            let width = reservation
                .as_ref()
                .map_or(current.frame.rect[2] - current.frame.rect[0], |r| r.width);
            let lookahead = (((current.frame.rect[3] - current.frame.rect[1]) / p.line_height)
                .ceil() as usize)
                .saturating_add(p.widows)
                .saturating_add(2)
                .min(100_000);
            let batch = probe_story_lines(
                &prepared[paragraph_index],
                &font_spans[paragraph_index],
                fonts,
                &font_metrics,
                p,
                request.writing_mode,
                consumed,
                width,
                lookahead,
            )?;
            let width_blocked = batch.blocked.is_some();
            let broken = batch.lines;
            if broken.is_empty() {
                if let Some(blocked) = batch.blocked {
                    // A new continuation repeats the last template's width;
                    // only a later approved geometry can fix this first line.
                    if !has_later_approved_frame {
                        return Err(blocked.into_error());
                    }
                    frame_index += 1;
                    y = f64::INFINITY;
                    continue;
                }
                return Err(fail("nonempty story paragraph produced no line progress"));
            }
            let paragraph_complete = broken
                .last()
                .is_some_and(|line| line.bytes.end == p.text.len());
            let forced_boundary = broken
                .iter()
                .position(|line| {
                    crate::fonts::hard_break::range_ends_with_form_feed(&p.text, &line.bytes)
                })
                .map(|index| index + 1);
            let mut measured = Vec::new();
            for line in &broken {
                measured.push(measure_story_line(
                    &prepared[paragraph_index],
                    &font_spans[paragraph_index],
                    fonts,
                    &font_metrics,
                    p,
                    request.writing_mode,
                    line.bytes.clone(),
                )?);
            }
            let ascent = measured.iter().map(|m| m.ascent).fold(0.0, f64::max);
            let descent = measured.iter().map(|m| m.descent).fold(0.0, f64::max);
            let line_height = p.line_height.max(ascent + descent);
            let mut baselines = Vec::new();
            let mut baseline = top - ascent;
            let mut attempted_rows = 0usize;
            while baseline - descent >= current.frame.rect[1] - 1e-7 {
                attempted_rows += 1;
                if attempted_rows.is_multiple_of(256) {
                    crate::cancel::check_current_cancel("story exclusion layout")?;
                }
                if attempted_rows > 100_000 {
                    return Err(fail("frame line budget exceeded"));
                }
                let line_box = [left, baseline - descent, left + width, baseline + ascent];
                if !current
                    .frame
                    .exclusions
                    .iter()
                    .any(|r| overlaps(r, &line_box))
                {
                    baselines.push(baseline);
                }
                baseline -= line_height;
            }
            let mut take = broken.len().min(baselines.len());
            if (p.keep_together || figure.is_some())
                && (take < broken.len()
                    || broken.last().is_some_and(|l| l.bytes.end < p.text.len()))
            {
                take = 0;
            }
            if let Some(boundary) = forced_boundary {
                take = take.min(boundary);
            }
            if take < broken.len() || !paragraph_complete {
                let ends_at_forced_boundary = forced_boundary == Some(take);
                if !ends_at_forced_boundary && take < p.orphans && consumed == segment_start {
                    take = 0;
                }
                // Width-blocked frames can be skipped. A prefix limited by
                // lookahead or by geometry is not the end of the paragraph.
                while take > 0 && forced_boundary != Some(take) {
                    if continuation_widows_fit(
                        &prepared[paragraph_index],
                        &font_spans[paragraph_index],
                        fonts,
                        &font_metrics,
                        p,
                        request.writing_mode,
                        broken[take - 1].bytes.end,
                        later_frames,
                        continuation_width,
                    )? {
                        break;
                    }
                    take -= 1;
                    if consumed == segment_start && take < p.orphans {
                        take = 0;
                    }
                }
            }
            // A continuation may have skipped a too-short/excluded frame.
            // Recheck the actual destination, not only predicted next width.
            if consumed > segment_start
                && paragraph_complete
                && take == broken.len()
                && take < p.widows
                && forced_boundary != Some(take)
            {
                take = 0;
            }
            if p.keep_with_next
                && paragraph_complete
                && take == broken.len()
                && paragraph_index + 1 < request.paragraphs.len()
                && take > 0
                && !following_block_fits(
                    request,
                    paragraph_index + 1,
                    fonts,
                    &font_metrics,
                    &current.frame,
                    baselines[take - 1] - (line_height - ascent) - p.space_after,
                    &prepared,
                    &font_spans,
                    &paragraph_figures,
                )?
            {
                take = 0;
            }
            if take == 0 {
                if current.lines.is_empty()
                    && current.figures.is_empty()
                    && current.frame.exclusions.is_empty()
                    && !has_later_approved_frame
                    && (p.keep_together
                        || figure.is_some()
                        || broken.len() <= p.orphans
                        || width_blocked)
                    && (y - current.frame.rect[3]).abs() < 1e-7
                {
                    return Err(WellfriendError::UnsupportedFeature(
                        "paragraph constraints cannot fit an empty frame".into(),
                    ));
                }
                frame_index += 1;
                y = f64::INFINITY;
                continue;
            }
            if let Some(reserved) = reservation {
                current.figures.push(reserved.placement);
            }
            for ((line, baseline), metric) in
                broken.iter().zip(&baselines).zip(&measured).take(take)
            {
                let tab_plan = story_tab_plan(
                    &prepared[paragraph_index],
                    &font_spans[paragraph_index],
                    fonts,
                    &font_metrics,
                    p,
                    request.writing_mode,
                    line.bytes.clone(),
                )?;
                current.lines.push(StoryPaintLine {
                    writing_mode: request.writing_mode,
                    text: p.text[line.bytes.clone()].to_string(),
                    x: left + metric.left_pad,
                    baseline: *baseline,
                    width: width - metric.left_pad - metric.right_pad,
                    font_size: p.font_size,
                    font_index: indices[paragraph_index],
                    font_spans: crate::fonts::fallback::slice_spans(
                        &font_spans[paragraph_index],
                        line.bytes.clone(),
                    ),
                    style_spans: line_paint_styles(
                        p,
                        line.bytes.start
                            ..line.bytes.start
                                + p.text[line.bytes.clone()]
                                    .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                                    .len(),
                    ),
                    tab_segments: tab_plan.segments,
                    tab_decorations: tab_plan.decorations,
                    rgb: p.rgb,
                    rtl: p.rtl,
                    bidi: Some(
                        bidi_context.line(
                            line.bytes.start
                                ..line.bytes.start
                                    + p.text[line.bytes.clone()]
                                        .trim_end_matches(crate::fonts::hard_break::is_hard_break)
                                        .len(),
                        )?,
                    ),
                    shaping: p.shaping.clone(),
                    tag_owner: None,
                    artifact: false,
                });
                current.paragraph_ids.push(p.id.clone());
            }
            consumed = broken[take - 1].bytes.end;
            let form_feed_offset = crate::fonts::hard_break::range_ends_with_form_feed(
                &p.text,
                &broken[take - 1].bytes,
            )
            .then_some(broken[take - 1].bytes.end - 1);
            y = baselines[take - 1] - (line_height - ascent);
            before_applied = true;
            if let Some(byte_offset) = form_feed_offset {
                let (target, from_page, to_page) = advance_story_page(
                    request,
                    &mut frames,
                    &mut generated,
                    frame_index,
                    StoryPageBreakBefore::NextPage,
                )?;
                page_breaks.push(StoryPageBreakReceipt {
                    paragraph_id: p.id.clone(),
                    byte_offset,
                    source: StoryPageBreakSource::FormFeed,
                    policy: StoryPageBreakBefore::NextPage,
                    from_page,
                    to_page,
                });
                segment_start = consumed;
                frame_index = target;
                y = f64::INFINITY;
            } else if consumed < p.text.len() {
                frame_index += 1;
                y = f64::INFINITY;
            }
        }
        if y.is_finite() {
            y -= p.space_after;
        } else {
            // A trailing form feed owns the destination page even with no
            // visible suffix. Paragraph-after spacing belongs to that new page.
            ensure_story_frame(request, &mut frames, &mut generated, frame_index)?;
            y = frames[frame_index].frame.rect[3] - p.space_after;
        }
    }
    let changed_pages = frames
        .iter()
        .map(|f| f.frame.page)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(LinkedStoryPreview { input_sha256: request.input_sha256.clone(), story_id: request.story_id.clone(),
        frames, font_choices: choices, changed_pages, generated_pages: generated, page_breaks, page_pruning: Default::default(), anchor_moves: Vec::new(), figure_removals: Vec::new(), figure_detachments: Vec::new(), full_page_invalidations: Vec::new(),
        rebound_frames: Vec::new(), output_sha256: None, reused_paragraphs: reused, checkpoints,
        qualification: "source_implementation_only; vps_corpus_gate_pending".into(),
        exact_limits: vec!["Explicit approved frames; no automatic reading-order inference".into(),
            "U+000C and explicit next/odd/even policies advance to a later physical page, skip same-page frames and emit receipt-bound transitions; this is not a section master/header/footer system".into(),
            "Declared horizontal/vertical inline and block axes; font changes occur only at covered contextual boundaries".into(),
            "Pinned exclusions stay fixed; approved annotation anchors and paragraph-leaf tags migrate with the story; arbitrary image/table/subtree migration remains separate".into(),
            "Backward flow clears unused frames; optional pruning removes only approved empty owned continuation pages".into()] })
}

pub fn preview_linked_story(
    input: &[u8],
    request: &LinkedStoryRequest,
) -> Result<LinkedStoryPreview> {
    validate(input, request)?;
    let (fonts, indices, choices) = resolve_fonts(input, request)?;
    let mut preview = layout(request, &fonts, &indices, choices)?;
    crate::story_anchors::attach_preview(input, request, &mut preview)?;
    figures::attach_preview(input, request, &mut preview)?;
    crate::tagged_structure::story::attach_preview(request, &mut preview)?;
    let pruning = page_pruning::plan(input, request, &preview)?;
    page_pruning::project(input, request, &mut preview, pruning)?;
    Ok(preview)
}

pub fn apply_linked_story(
    input: &[u8],
    request: &LinkedStoryRequest,
) -> Result<(Vec<u8>, LinkedStoryPreview)> {
    validate(input, request)?;
    let (fonts, indices, choices) = resolve_fonts(input, request)?;
    let mut preview = layout(request, &fonts, &indices, choices)?;
    crate::story_anchors::attach_preview(input, request, &mut preview)?;
    figures::attach_preview(input, request, &mut preview)?;
    crate::tagged_structure::story::attach_preview(request, &mut preview)?;
    let pruning = page_pruning::plan(input, request, &preview)?;
    let (image_detached, image_receipt) = crate::image_fragments::stories::stage(input, request)?;
    let anchored =
        crate::story_anchors::stage_identities(input, &image_detached, &preview.anchor_moves)?;
    let isolated = clone_shared_frame_streams(&anchored, request)?;
    let detached = tables::detach_source_paint(input, &isolated, request)?;
    let mut output = crate::tagged_structure::story::prepare(&detached, request)?;
    let mut expected_frame_shapes = BTreeMap::<String, String>::new();
    let mut existing = preview
        .frames
        .iter()
        .filter(|f| !f.created)
        .cloned()
        .collect::<Vec<_>>();
    image_receipt.rebind_source_frames(&output, &mut existing)?;
    if crate::tagged_structure::story::active(request) {
        // Preflight checked the original owner hashes. Private tag detachment
        // changes only approved delimiters, so bind this prepared revision once,
        // before any frame writes (never authorize changes from earlier writes).
        let prepared = ContentEngine::open_bytes(output.clone())?;
        let bindings = crate::advanced_editing::story_frame_inventory(&prepared)?;
        for frame in &mut existing {
            if let Some(old) = &frame.frame.owner {
                let (page, binding) = bindings
                    .get(&old.key)
                    .ok_or_else(|| fail("tag preparation lost a story frame owner"))?;
                if *page != frame.frame.page {
                    return Err(fail("tag preparation moved a frame"));
                }
                frame.frame.owner = Some(binding.clone());
            }
        }
    }
    existing.sort_by_key(|f| std::cmp::Reverse((f.frame.page, f.frame.logical_range[0])));
    for frame in existing {
        crate::cancel::check_current_cancel("linked story frame transaction")?;
        let key = frame_key(&request.story_id, &frame.frame.id);
        let write = replace_story_frame(
            &output,
            frame.frame.page,
            frame.frame.logical_range,
            frame.frame.rect,
            &frame.lines,
            &frame.decorations,
            &fonts,
            request.signature_policy_override,
            &key,
            frame.frame.owner.as_ref(),
        )?;
        let expected_shape = write
            .binding
            .shape_sha256
            .clone()
            .ok_or_else(|| fail("story frame write omitted its shaped-run receipt"))?;
        if expected_frame_shapes.insert(key, expected_shape).is_some() {
            return Err(fail("duplicate story frame receipt"));
        }
        output = write.bytes;
    }
    let template = ContentEngine::open_bytes(input.to_vec())?
        .document()
        .get_page(request.frames.last().unwrap().page)?;
    let mut continuations = Vec::new();
    for frame in preview.frames.iter().filter(|f| f.created) {
        use crate::authoring::{FontFace, PageSize, PdfBuilder, TextStyle};
        let mut builder = PdfBuilder::new();
        let page = builder.add_page(PageSize::custom(
            template.media_box[2] - template.media_box[0],
            template.media_box[3] - template.media_box[1],
        ));
        page.draw_text(
            " ",
            0.0,
            12.0,
            &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
        )?;
        let blank = builder.to_bytes()?;
        let key = frame_key(&request.story_id, &frame.frame.id);
        let created = replace_story_frame(
            &blank,
            1,
            [0, 1],
            frame.frame.rect,
            &frame.lines,
            &frame.decorations,
            &fonts,
            true,
            &key,
            None,
        )?;
        let expected_shape = created
            .binding
            .shape_sha256
            .clone()
            .ok_or_else(|| fail("story continuation omitted its shaped-run receipt"))?;
        if expected_frame_shapes.insert(key, expected_shape).is_some() {
            return Err(fail("duplicate story continuation receipt"));
        }
        continuations.push(ContentEngine::open_bytes(created.bytes)?);
    }
    if !continuations.is_empty() {
        let source = ContentEngine::open_bytes(output)?;
        let geometry = Some(crate::writer::AuthoredPageGeometry {
            media_box: template.media_box,
            crop_box: template.crop_box,
            bleed_box: template.bleed_box,
            trim_box: template.trim_box,
            art_box: template.art_box,
            rotate: template.rotate,
            user_unit: template.user_unit,
        });
        let pages = continuations
            .iter()
            .map(|engine| (engine.document(), geometry))
            .collect::<Vec<_>>();
        output = crate::writer::insert_authored_pages_preserving_catalog(
            source.document(),
            &pages,
            request.frames.last().unwrap().page + 1,
        )?;
    }
    let image_output =
        crate::image_fragments::stories::finish(&output, request, &preview, &image_receipt)?;
    let (tagged_output, rebound_tags, rebound_table_tags) =
        crate::tagged_structure::story::finish(&image_output, request, &preview)?;
    output = crate::story_anchors::apply_moves(&tagged_output, &preview.anchor_moves)?;
    output = page_pruning::stamp_created(&output, request, &preview)?;
    output = page_pruning::apply(&output, request, &pruning)?;
    page_pruning::project(input, request, &mut preview, pruning)?;
    let rebound_anchors = crate::story_anchors::rebind(&output, &request.annotation_anchors)?;
    // Nothing leaves this transaction until all frames, insertions and reopen
    // have succeeded; a cancellation/error discards the private output buffer.
    crate::cancel::check_current_cancel("linked story commit")?;
    ContentEngine::open_bytes(output.clone())?;
    for frame in &preview.frames {
        let mut rebound = frame.frame.clone();
        let binding = bind_story_frame(
            &output,
            rebound.page,
            &frame_key(&request.story_id, &rebound.id),
        )?
        .ok_or_else(|| fail("saved story owner is missing"))?;
        let expected_shape = expected_frame_shapes
            .get(&binding.key)
            .ok_or_else(|| fail("saved story owner has no write receipt"))?;
        if binding.shape_sha256.as_ref() != Some(expected_shape) {
            return Err(fail(
                "saved story shaped-run receipt changed after its write",
            ));
        }
        let expected_paint =
            crate::advanced_editing::story_paint_model_sha256(&frame.lines, &frame.decorations)?;
        if binding.paint_sha256.as_deref() != Some(expected_paint.as_str()) {
            return Err(fail("saved story paint-model receipt mismatch"));
        }
        rebound.owner = Some(binding);
        rebound.expected_text = frame.lines.iter().map(|l| l.text.as_str()).collect();
        rebound.logical_range = [0, 0];
        preview.rebound_frames.push(rebound);
    }
    let used_fonts = if request.paragraphs.is_empty() {
        // An empty story is a valid reversible editing state, not a request to
        // forget its approved font collection. Retaining the bounded pool lets
        // a later refill resolve the same fonts without external installation.
        fonts.clone()
    } else {
        indices
            .iter()
            .copied()
            .chain(
                preview
                    .frames
                    .iter()
                    .flat_map(|f| &f.lines)
                    .flat_map(|l| l.font_spans.iter().map(|s| s.font_index)),
            )
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|index| fonts[index].clone())
            .collect::<Vec<_>>()
    };
    output = save_story_metadata(
        &output,
        request,
        &preview.rebound_frames,
        &used_fonts,
        &rebound_anchors,
        rebound_tags,
        rebound_table_tags,
    )?;
    preview.output_sha256 = Some(hash(&output));
    Ok((output, preview))
}

/// Clone each shared page-content occurrence under a fresh object identity.
/// Resources remain shared read-only; the frame writer installs its own page
/// font dictionary. Other pages and other uses of the same stream are untouched.
fn clone_shared_frame_streams(input: &[u8], request: &LinkedStoryRequest) -> Result<Vec<u8>> {
    use crate::writer::{write_incremental_update, IncrementalObject};
    use crate::PdfObject;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let pages = engine.document().get_pages()?;
    let selected = request
        .frames
        .iter()
        .map(|f| f.page)
        .chain(request.figures.iter().map(|f| match &f.source {
            crate::image_fragments::ImageFragmentSource::Occurrence { page, .. } => *page,
            crate::image_fragments::ImageFragmentSource::Owned { binding } => binding.page,
        }))
        .chain(request.figure_removals.iter().map(|r| r.binding.page))
        .collect::<BTreeSet<_>>();
    let mut counts = BTreeMap::new();
    for page in &pages {
        for id in &page.contents {
            *counts.entry(*id).or_insert(0usize) += 1;
        }
    }
    let mut next = reader
        .object_ids()
        .iter()
        .map(|(n, _)| *n)
        .max()
        .unwrap_or(0);
    let mut updates = Vec::new();
    for page in &pages {
        if !selected.contains(&page.page_number) {
            continue;
        }
        let mut contents = Vec::new();
        let mut changed = false;
        for &(number, generation) in &page.contents {
            crate::cancel::check_current_cancel("story occurrence cloning")?;
            let id = if counts[&(number, generation)] > 1 {
                changed = true;
                next = next
                    .checked_add(1)
                    .ok_or_else(|| fail("story object space exhausted"))?;
                updates.push(IncrementalObject {
                    number: next,
                    generation: 0,
                    object: reader.get_object(number, generation)?,
                });
                (next, 0)
            } else {
                (number, generation)
            };
            contents.push(PdfObject::Reference {
                number: id.0,
                generation: id.1,
            });
        }
        if changed {
            let mut dict = reader
                .get_object(page.object_number, page.generation_number)?
                .as_dict()
                .cloned()
                .ok_or_else(|| fail("invalid story page"))?;
            dict.insert("Contents", PdfObject::Array(contents));
            updates.push(IncrementalObject {
                number: page.object_number,
                generation: page.generation_number,
                object: PdfObject::Dictionary(dict),
            });
        }
    }
    if updates.is_empty() {
        Ok(input.to_vec())
    } else {
        write_incremental_update(reader, updates)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedLinkedStory {
    pub schema_version: u32,
    pub request: LinkedStoryRequest,
}

fn required_story_schema(request: &LinkedStoryRequest) -> u32 {
    if request
        .paragraphs
        .iter()
        .any(|paragraph| paragraph.tab_stops.has_extended_decimal())
    {
        8
    } else if request
        .paragraphs
        .iter()
        .any(|paragraph| paragraph.tab_stops.has_decorations())
    {
        7
    } else if request
        .paragraphs
        .iter()
        .any(|paragraph| paragraph.text.contains('\t') || !paragraph.tab_stops.is_default())
    {
        6
    } else if request
        .paragraphs
        .iter()
        .any(|paragraph| !paragraph.inline_styles.is_empty())
    {
        5
    } else if request.paragraphs.iter().any(|p| {
        !p.page_break_before.is_none() || p.text.chars().any(crate::fonts::hard_break::is_form_feed)
    }) {
        4
    } else if request.paragraphs.iter().any(|p| {
        p.line_break.composition == crate::fonts::line_break_policy::LineComposition::Balanced
    }) {
        3
    } else if request
        .paragraphs
        .iter()
        .any(|p| !p.line_break.is_default())
    {
        2
    } else {
        1
    }
}
fn supported_story_schema(version: u32, request: &LinkedStoryRequest) -> bool {
    version >= required_story_schema(request) && version <= SAVED_STORY_SCHEMA_MAX
}

fn read_story_metadata(engine: &ContentEngine) -> Result<BTreeMap<String, crate::PdfObject>> {
    let catalog = engine.document().get_catalog()?;
    match catalog.get("WellfriendStories") {
        None => Ok(BTreeMap::new()),
        Some(crate::PdfObject::Dictionary(dict)) => {
            Ok(dict.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        _ => Err(fail("invalid story metadata dictionary")),
    }
}

fn save_story_metadata(
    input: &[u8],
    request: &LinkedStoryRequest,
    frames: &[StoryFrame],
    fonts: &[ApprovedFontAsset],
    anchors: &[crate::story_anchors::StoryAnnotationAnchor],
    source_tags: Option<crate::tagged_structure::story::StoryTagging>,
    table_tags: Option<crate::tagged_structure::story::tables::TableTagging>,
) -> Result<Vec<u8>> {
    use crate::writer::{write_incremental_update, IncrementalObject};
    use crate::PdfObject;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let mut entries = read_story_metadata(&engine)?;
    let key = hash(request.story_id.as_bytes());
    if entries.len() >= 64 && !entries.contains_key(&key) {
        return Err(fail("saved story limit exceeded"));
    }
    // Do not clone a potentially 256 MiB font pool merely to discard it.
    let saved = LinkedStoryRequest {
        writing_mode: request.writing_mode,
        story_id: request.story_id.clone(),
        input_sha256: String::new(),
        frames: frames.to_vec(),
        paragraphs: request.paragraphs.clone(),
        fonts: Vec::new(),
        annotation_anchors: anchors.to_vec(),
        figures: figures::rebind(input, &request.figures, &request.input_sha256, false)?,
        figure_removals: Vec::new(), // consumed commands must never replay on reopen
        figure_detachments: Vec::new(), // consumed commands must never replay on reopen
        source_tags,
        table_layout: request.table_layout.clone().map(|mut table| {
            table.source_paint.clear();
            table.tagging = table_tags;
            table
        }),
        allow_font_substitution: request.allow_font_substitution,
        allow_page_creation: request.allow_page_creation,
        prune_empty_pages: request.prune_empty_pages,
        max_new_pages: request.max_new_pages,
        mode: request.mode,
        signature_policy_override: false,
    }; // font bytes are PDF resources; signature authorization is never persisted
       // Ordinary saves retain any causal history but do not falsely advance its
       // checkpoint. A later resume detects a detached model; only the approved
       // history transaction can update both the saved model and its lineage.
    let raw = history::encode_saved_metadata(&engine, saved)?;
    if raw.len() > 16 * 1024 * 1024 {
        return Err(fail("story metadata budget exceeded"));
    }
    let mut number = reader
        .object_ids()
        .iter()
        .map(|(n, _)| *n)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| fail("story object space exhausted"))?;
    let mut dict = crate::PdfDictionary::empty();
    dict.insert("Type", PdfObject::Name("WellfriendStory".into()));
    dict.insert("Length", PdfObject::Integer(raw.len() as i64));
    let mut updates = vec![IncrementalObject {
        number,
        generation: 0,
        object: PdfObject::Stream { dict, raw },
    }];
    entries.insert(
        key,
        PdfObject::Reference {
            number,
            generation: 0,
        },
    );
    let mut properties = crate::PdfDictionary::empty();
    for (key, value) in entries {
        properties.insert(key, value);
    }
    let mut catalog = engine.document().get_catalog()?;
    catalog.insert("WellfriendStories", PdfObject::Dictionary(properties));
    let mut font_stories = catalog
        .get("WellfriendStoryFonts")
        .and_then(PdfObject::as_dict)
        .cloned()
        .unwrap_or_default();
    let mut font_assets = crate::PdfDictionary::empty();
    let old_assets = font_stories
        .get(&hash(request.story_id.as_bytes()))
        .and_then(PdfObject::as_dict);
    for font in fonts {
        let digest = hash(&font.bytes);
        let asset_key = hash(format!("{}:{digest}", font.lookup_name).as_bytes());
        if let Some(old) = old_assets.and_then(|d| d.get(&asset_key)) {
            font_assets.insert(asset_key, old.clone());
            continue;
        }
        number = number
            .checked_add(1)
            .ok_or_else(|| fail("story font object space exhausted"))?;
        let mut program = crate::PdfDictionary::empty();
        program.insert("Type", PdfObject::Name("WellfriendStoryFont".into()));
        program.insert("Length", PdfObject::Integer(font.bytes.len() as i64));
        updates.push(IncrementalObject {
            number,
            generation: 0,
            object: PdfObject::Stream {
                dict: program,
                raw: font.bytes.clone(),
            },
        });
        let mut asset = crate::PdfDictionary::empty();
        asset.insert(
            "Name",
            PdfObject::String(font.lookup_name.as_bytes().to_vec()),
        );
        asset.insert("Sha256", PdfObject::String(digest.into_bytes()));
        asset.insert(
            "Program",
            PdfObject::Reference {
                number,
                generation: 0,
            },
        );
        font_assets.insert(asset_key, PdfObject::Dictionary(asset));
    }
    font_stories.insert(
        hash(request.story_id.as_bytes()),
        PdfObject::Dictionary(font_assets),
    );
    catalog.insert("WellfriendStoryFonts", PdfObject::Dictionary(font_stories));
    let root = reader
        .root_reference()
        .ok_or_else(|| fail("missing story catalog"))?;
    updates.push(IncrementalObject {
        number: root.0,
        generation: root.1,
        object: PdfObject::Dictionary(catalog),
    });
    let output = write_incremental_update(reader, updates)?;
    ContentEngine::open_bytes(output.clone())?;
    Ok(output)
}

fn load_story_fonts(engine: &ContentEngine, story_id: &str) -> Result<Vec<ApprovedFontAsset>> {
    use crate::PdfObject;
    let catalog = engine.document().get_catalog()?;
    let Some(assets) = catalog
        .get("WellfriendStoryFonts")
        .and_then(PdfObject::as_dict)
        .and_then(|d| d.get(&hash(story_id.as_bytes())))
        .and_then(PdfObject::as_dict)
    else {
        return Ok(Vec::new());
    };
    if assets.len() > 128 {
        return Err(fail("story font asset limit exceeded"));
    }
    let mut fonts = Vec::new();
    let mut budget = 0usize;
    for (_, asset) in assets.iter() {
        let dict = asset
            .as_dict()
            .ok_or_else(|| fail("invalid story font asset"))?;
        let read_string = |name| -> Result<String> {
            match dict.get(name) {
                Some(PdfObject::String(bytes)) => {
                    String::from_utf8(bytes.clone()).map_err(|_| fail("invalid font asset string"))
                }
                _ => Err(fail("missing font asset property")),
            }
        };
        let program = engine.document().reader().resolve(
            dict.get("Program")
                .cloned()
                .ok_or_else(|| fail("missing story font program"))?,
        )?;
        let decoded = crate::filters::decode_stream_lossless_with_limits(
            &program,
            engine.document().reader(),
            &crate::filters::DecodeLimits {
                max_decoded_bytes_per_stream: 128 * 1024 * 1024,
                ..Default::default()
            },
        )?;
        budget += decoded.data.len();
        if budget > 256 * 1024 * 1024
            || decoded.status != crate::filters::StreamDecodeStatus::Complete
        {
            return Err(fail("story font asset decode/budget failure"));
        }
        if hash(&decoded.data) != read_string("Sha256")? {
            return Err(fail("story font asset fingerprint mismatch"));
        }
        fonts.push(ApprovedFontAsset {
            lookup_name: read_string("Name")?,
            bytes: decoded.data,
        });
    }
    Ok(fonts)
}

/// Load explicit user-approved stories and verify every owner against current
/// content. Page numbers are resolved by marker, so unrelated page insertion
/// cannot silently redirect an edit. Modified or duplicated owners fail closed.
pub fn load_linked_stories(input: &[u8]) -> Result<Vec<SavedLinkedStory>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    load_linked_stories_in_engine(input, &engine)
}

// Callers that already opened these exact bytes can reuse the parsed document.
// Keep this private so an external host cannot pair unrelated bytes and owners.
fn load_linked_stories_in_engine(
    input: &[u8],
    engine: &ContentEngine,
) -> Result<Vec<SavedLinkedStory>> {
    let reader = engine.document().reader();
    let entries = read_story_metadata(engine)?;
    if entries.len() > 64 {
        return Err(fail("saved story limit exceeded"));
    }
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let inventory = crate::advanced_editing::story_frame_inventory(engine)?;
    let revision = hash(input);
    let mut result = Vec::new();
    let mut budget = 0usize;
    for (_, object) in entries {
        crate::cancel::check_current_cancel("story metadata loading")?;
        let object = match object {
            crate::PdfObject::Reference { number, generation } => {
                reader.get_object(number, generation)?
            }
            value => value,
        };
        let decoded = crate::filters::decode_stream_lossless_with_limits(
            &object,
            reader,
            &crate::filters::DecodeLimits {
                max_decoded_bytes_per_stream: 16 * 1024 * 1024,
                ..crate::filters::DecodeLimits::default()
            },
        )?;
        if decoded.status != crate::filters::StreamDecodeStatus::Complete {
            return Err(fail("opaque story metadata"));
        }
        budget += decoded.data.len();
        if budget > 64 * 1024 * 1024 {
            return Err(fail("story metadata budget exceeded"));
        }
        let mut saved: SavedLinkedStory =
            serde_json::from_slice(&decoded.data).map_err(|e| fail(&e.to_string()))?;
        if !supported_story_schema(saved.schema_version, &saved.request)
            || saved.request.frames.len() > 4096
            || !saved.request.figure_removals.is_empty()
            || !saved.request.figure_detachments.is_empty()
        {
            return Err(fail("unsupported story metadata"));
        }
        saved.request.input_sha256 = revision.clone();
        saved.request.signature_policy_override = false;
        saved.request.fonts.clear();
        saved.request.figures = figures::rebind(input, &saved.request.figures, &revision, true)?;
        let rebound = crate::story_anchors::rebind(input, &saved.request.annotation_anchors)?;
        if rebound
            .iter()
            .zip(&saved.request.annotation_anchors)
            .any(|(a, b)| !crate::story_anchors::same_saved_binding(a, b))
        {
            return Err(fail("saved annotation anchor geometry or group changed"));
        }
        saved.request.annotation_anchors = rebound;
        for frame in &mut saved.request.frames {
            let owner = frame
                .owner
                .as_ref()
                .ok_or_else(|| fail("saved frame has no owner"))?;
            if owner.key != frame_key(&saved.request.story_id, &frame.id) {
                return Err(fail("saved owner key mismatch"));
            }
            let (page, binding) = inventory
                .get(&owner.key)
                .ok_or_else(|| fail("saved frame is no longer present"))?;
            if binding != owner {
                return Err(fail("saved frame content changed"));
            }
            frame.page = *page;
        }
        if required_story_schema(&saved.request) >= 5
            && saved.request.frames.iter().any(|frame| {
                frame.owner.as_ref().is_none_or(|owner| {
                    owner.paint_sha256.is_none() || owner.shape_sha256.is_none()
                })
            })
        {
            return Err(fail(
                "styled story frame is missing its paint or shaped-run receipt",
            ));
        }
        // Metadata is untrusted input even when its stream and frame owners are
        // intact. Re-run every paragraph/style budget and boundary invariant;
        // deserialization alone does not validate inline UTF-8/grapheme ranges.
        validate_paragraphs(&saved.request)?;
        result.push(saved);
    }
    Ok(result)
}

/// Bounded, non-content history metadata for session hosts.
#[derive(Debug, Clone, Serialize)]
pub struct StoryHistoryStatus {
    pub undo_steps: usize,
    pub redo_steps: usize,
    pub retained_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StorySourceSecurityStatus {
    /// Hash of the exact caller-supplied bytes, before any security-envelope
    /// normalization. This is diagnostic provenance, not the editable revision.
    pub source_input_sha256: String,
    /// True when the supplied PDF was opened through a Standard security
    /// handler and therefore had an active decryption context.
    pub source_was_encrypted: bool,
    /// Encrypted inputs are converted once into an unencrypted canonical
    /// working revision. All request/receipt hashes bind this working revision.
    pub working_copy_decrypted: bool,
    /// The retained session never stores the input password and never silently
    /// promises to preserve or recreate the original encryption envelope.
    pub password_retained: bool,
    /// True only when the Standard handler authenticated an owner password.
    /// User-password sessions remain governed by the recovered `/P` mask.
    pub authenticated_as_owner: bool,
    /// Original signed PDF permission mask, absent for unencrypted inputs.
    pub permissions: Option<i32>,
    /// Whether the original authentication context permits changing document
    /// contents. Publication fails closed when this is false.
    pub modification_permitted: bool,
}

/// A caller-owned preview/checkpoint session. Preview never serializes a PDF;
/// checkpoints are explicit and undo is an exact byte preimage. Source requests
/// must be rebound to the new SHA after a checkpoint, never silently replayed.
pub struct LinkedStorySession {
    bytes: std::sync::Arc<Vec<u8>>,
    document: ContentEngine,
    source_security: StorySourceSecurityStatus,
    cached: Option<(String, LinkedStoryPreview)>,
    previous_preview: Option<LinkedStoryPreview>,
    undo: std::collections::VecDeque<std::sync::Arc<Vec<u8>>>,
    redo: std::collections::VecDeque<std::sync::Arc<Vec<u8>>>,
    prepared: Option<(String, std::sync::Arc<Vec<ApprovedFontAsset>>)>,
    paragraph_hashes: Vec<String>,
    font_indices: Vec<usize>,
    history_receipt: Option<history::HistoryPreviewReceipt>,
}

impl LinkedStorySession {
    pub fn open(bytes: Vec<u8>) -> Result<Self> {
        Self::open_with_password(bytes, b"")
    }

    /// Open a retained story session from an optionally password-protected PDF.
    ///
    /// A successfully unlocked encrypted PDF is normalized exactly once into an
    /// unencrypted working revision. Requests and receipts bind that revision,
    /// never the encrypted transport bytes. Because the working bytes are
    /// exportable without the source envelope, encrypted input requires owner
    /// authentication. The password is borrowed only for this call and is not
    /// retained by the session.
    pub fn open_with_password(bytes: Vec<u8>, password: &[u8]) -> Result<Self> {
        let source_input_sha256 = hash(&bytes);
        let opened = ContentEngine::open_bytes_with_password(bytes.clone(), password)?;
        let source_was_encrypted = opened.is_encrypted();
        let source_authority = opened.document().reader().encryption().map(|context| {
            (
                context.authenticated_as_owner,
                context.permissions,
                context.permits_content_modification(),
            )
        });
        if source_was_encrypted
            && !source_authority
                .is_some_and(|(authenticated_as_owner, _, _)| authenticated_as_owner)
        {
            return Err(fail(
                "encrypted retained editing sessions require the permissions/owner password because the exported working revision is unencrypted",
            ));
        }
        let (bytes, document) = if source_was_encrypted {
            let working = crate::utilities::decrypt_pdf(&opened)?;
            let document = ContentEngine::open_bytes(working.clone())?;
            (working, document)
        } else {
            (bytes, opened)
        };
        Ok(Self {
            bytes: std::sync::Arc::new(bytes),
            document,
            source_security: StorySourceSecurityStatus {
                source_input_sha256,
                source_was_encrypted,
                working_copy_decrypted: source_was_encrypted,
                password_retained: false,
                authenticated_as_owner: source_authority
                    .is_some_and(|(authenticated_as_owner, _, _)| authenticated_as_owner),
                permissions: source_authority.map(|(_, permissions, _)| permissions),
                modification_permitted: source_authority
                    .is_none_or(|(_, _, modification_permitted)| modification_permitted),
            },
            cached: None,
            previous_preview: None,
            undo: std::collections::VecDeque::new(),
            redo: std::collections::VecDeque::new(),
            prepared: None,
            paragraph_hashes: Vec::new(),
            font_indices: Vec::new(),
            history_receipt: None,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }
    pub fn document(&self) -> &ContentEngine {
        &self.document
    }
    pub fn source_security_status(&self) -> &StorySourceSecurityStatus {
        &self.source_security
    }
    pub fn revision_sha256(&self) -> String {
        hash(self.bytes())
    }

    /// Counts only; no preimage bytes or source text are exposed by status.
    pub fn history_status(&self) -> StoryHistoryStatus {
        StoryHistoryStatus {
            undo_steps: self.undo.len(),
            redo_steps: self.redo.len(),
            retained_bytes: self.undo.iter().chain(&self.redo).map(|b| b.len()).sum(),
        }
    }

    pub fn preview(
        &mut self,
        request: &LinkedStoryRequest,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<LinkedStoryPreview> {
        cancel.check("story preview")?;
        self.history_receipt = None;
        let key = value_hash(request)?;
        if let Some((old_key, preview)) = &self.cached {
            if *old_key == key {
                return Ok(preview.clone());
            }
        }
        let scope =
            crate::cancel::CancelToken::linked_pair(cancel, &crate::cancel::current_cancel_token());
        let context = layout_context_key(request)?;
        let reusable = self
            .prepared
            .as_ref()
            .is_some_and(|(key, _)| key == &context);
        let fonts = if reusable {
            self.prepared.as_ref().unwrap().1.clone()
        } else {
            std::sync::Arc::new(scope.scope(|| {
                validate(self.bytes(), request)?;
                resolve_font_pool(self.bytes(), request)
            })?)
        };
        let (indices, choices) = scope.scope(|| {
            validate_paragraphs(request)?;
            choose_fonts(request, &fonts)
        })?;
        let seed = if reusable && !request.prune_empty_pages {
            self.cached.as_ref().map(|(_, previous)| LayoutSeed {
                previous,
                paragraph_hashes: &self.paragraph_hashes,
                font_indices: &self.font_indices,
            })
        } else {
            None
        };
        let mut preview =
            scope.scope(|| layout_seeded(request, &fonts, &indices, choices, seed))?;
        scope
            .scope(|| crate::story_anchors::attach_preview(self.bytes(), request, &mut preview))?;
        scope.scope(|| figures::attach_preview(self.bytes(), request, &mut preview))?;
        crate::tagged_structure::story::attach_preview(request, &mut preview)?;
        let pruning = scope.scope(|| page_pruning::plan(self.bytes(), request, &preview))?;
        page_pruning::project(self.bytes(), request, &mut preview, pruning)?;
        cancel.check("story preview publication")?;
        self.prepared = Some((context, fonts));
        self.paragraph_hashes = request
            .paragraphs
            .iter()
            .map(value_hash)
            .collect::<Result<Vec<_>>>()?;
        self.font_indices = indices;
        self.previous_preview = self.cached.as_ref().map(|(_, p)| p.clone());
        self.cached = Some((key, preview.clone()));
        Ok(preview)
    }

    /// Consumers may invalidate just these page rectangles in the existing tile
    /// renderer. This is layout-diff metadata, not a claim that pixels matched.
    pub fn dirty_regions(&self) -> Vec<(usize, [f64; 4])> {
        let Some((_, current)) = &self.cached else {
            return Vec::new();
        };
        let mut dirty = current.full_page_invalidations.clone();
        for anchor in &current.anchor_moves {
            dirty.push((anchor.source_page, anchor.old_rect));
            dirty.push((anchor.target_page, anchor.new_rect));
        }
        for frame in &current.frames {
            let unchanged = self
                .previous_preview
                .as_ref()
                .and_then(|p| p.frames.iter().find(|f| f.frame.id == frame.frame.id))
                .is_some_and(|old| {
                    serde_json::to_value(old).ok() == serde_json::to_value(frame).ok()
                });
            if !unchanged {
                dirty.push((frame.frame.page, frame.frame.rect));
            }
        }
        if let Some(previous) = &self.previous_preview {
            dirty.extend_from_slice(&previous.full_page_invalidations);
            for anchor in &previous.anchor_moves {
                dirty.push((anchor.target_page, anchor.new_rect));
            }
            for frame in &previous.frames {
                if !current.frames.iter().any(|f| {
                    f.frame.id == frame.frame.id
                        && f.frame.rect == frame.frame.rect
                        && f.frame.page == frame.frame.page
                }) {
                    dirty.push((frame.frame.page, frame.frame.rect));
                }
            }
        }
        dirty
    }

    pub fn preview_receipt(&self) -> Result<StoryPreviewReceipt> {
        let (request_hash, preview) = self
            .cached
            .as_ref()
            .ok_or_else(|| fail("preview the current request before approving it"))?;
        Ok(StoryPreviewReceipt {
            revision_sha256: self.revision_sha256(),
            request_sha256: request_hash.clone(),
            preview_sha256: value_hash(preview)?,
        })
    }

    /// Binding-level checkpoint entry point. Layout is recomputed by the
    /// canonical writer, but authority cannot silently follow changed input.
    pub fn checkpoint_approved(
        &mut self,
        request: &LinkedStoryRequest,
        receipt: &StoryPreviewReceipt,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<LinkedStoryPreview> {
        cancel.check("approved story checkpoint")?;
        if self.preview_receipt()? != *receipt || value_hash(request)? != receipt.request_sha256 {
            return Err(fail(
                "story approval is stale or belongs to a different preview/request",
            ));
        }
        self.checkpoint(request, cancel)
    }

    pub fn checkpoint(
        &mut self,
        request: &LinkedStoryRequest,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<LinkedStoryPreview> {
        let scope =
            crate::cancel::CancelToken::linked_pair(cancel, &crate::cancel::current_cancel_token());
        let (output, report) = scope.scope(|| apply_linked_story(self.bytes(), request))?;
        self.publish_checkpoint(output, report, cancel)
    }

    fn publish_checkpoint(
        &mut self,
        output: Vec<u8>,
        report: LinkedStoryPreview,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<LinkedStoryPreview> {
        self.publish_bytes(output, cancel)?;
        Ok(report)
    }

    pub(super) fn publish_bytes(
        &mut self,
        output: Vec<u8>,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<bool> {
        if !self.source_security.modification_permitted {
            return Err(fail(
                "the encrypted source was opened without owner authority and its permission mask forbids changing document contents",
            ));
        }
        let document = ContentEngine::open_bytes(output.clone())?;
        cancel.check("story checkpoint publication")?;
        // Explicit memory budget; never retain unbounded whole-document history.
        while self.undo.len() >= 8
            || self.undo.iter().map(|b| b.len()).sum::<usize>() + self.bytes.len()
                > 128 * 1024 * 1024
        {
            if self.undo.pop_front().is_none() {
                break;
            }
        }
        let retained_preimage = self.bytes.len() <= 128 * 1024 * 1024;
        if retained_preimage {
            self.undo.push_back(self.bytes.clone());
        }
        self.bytes = std::sync::Arc::new(output);
        self.document = document;
        self.redo.clear();
        self.clear_preview_state();
        Ok(retained_preimage)
    }
    pub fn undo(&mut self) -> Result<bool> {
        let Some(bytes) = self.undo.back() else {
            return Ok(false);
        };
        let document = ContentEngine::open_bytes(bytes.as_ref().clone())?;
        crate::cancel::check_current_cancel("story undo publication")?;
        let target = self.undo.pop_back().expect("undo preimage exists");
        self.redo.push_back(self.bytes.clone());
        self.bytes = target;
        self.document = document;
        self.trim_history();
        self.clear_preview_state();
        Ok(true)
    }
    pub fn redo(&mut self) -> Result<bool> {
        let Some(bytes) = self.redo.back() else {
            return Ok(false);
        };
        let document = ContentEngine::open_bytes(bytes.as_ref().clone())?;
        crate::cancel::check_current_cancel("story redo publication")?;
        let target = self.redo.pop_back().expect("redo preimage exists");
        self.undo.push_back(self.bytes.clone());
        self.bytes = target;
        self.document = document;
        self.trim_history();
        self.clear_preview_state();
        Ok(true)
    }
    pub fn saved_stories(&self) -> Result<Vec<SavedLinkedStory>> {
        load_linked_stories(self.bytes())
    }
    fn clear_preview_state(&mut self) {
        self.history_receipt = None;
        self.cached = None;
        self.previous_preview = None;
        self.prepared = None;
        self.paragraph_hashes.clear();
        self.font_indices.clear();
    }
    fn trim_history(&mut self) {
        while self.undo.len() + self.redo.len() > 8
            || self
                .undo
                .iter()
                .chain(&self.redo)
                .map(|b| b.len())
                .sum::<usize>()
                > 128 * 1024 * 1024
        {
            if !self.undo.is_empty() {
                self.undo.pop_front();
            } else {
                self.redo.pop_front();
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn request(text: String) -> LinkedStoryRequest {
        LinkedStoryRequest {
            story_id: "contract-body".into(),
            writing_mode: WritingMode::HorizontalTb,
            input_sha256: String::new(),
            frames: vec![
                StoryFrame {
                    id: "first".into(),
                    page: 1,
                    logical_range: [0, 3],
                    expected_text: "OLD".into(),
                    rect: [10.0, 10.0, 190.0, 70.0],
                    exclusions: Vec::new(),
                    owner: None,
                },
                StoryFrame {
                    id: "second".into(),
                    page: 2,
                    logical_range: [0, 3],
                    expected_text: "OLD".into(),
                    rect: [10.0, 10.0, 190.0, 70.0],
                    exclusions: Vec::new(),
                    owner: None,
                },
            ],
            paragraphs: vec![StoryParagraph {
                id: "p1".into(),
                text,
                preferred_font: "Helvetica".into(),
                font_size: 12.0,
                line_height: 14.0,
                rgb: [0.0; 3],
                rtl: false,
                keep_with_next: false,
                keep_together: false,
                break_before: false,
                page_break_before: StoryPageBreakBefore::None,
                orphans: 2,
                widows: 2,
                space_before: 0.0,
                space_after: 0.0,
                shaping: Default::default(),
                inline_styles: Vec::new(),
                line_break: Default::default(),
                tab_stops: Default::default(),
            }],
            fonts: vec![ApprovedFontAsset {
                lookup_name: "Helvetica".into(),
                bytes: crate::render::get_fallback_font("Helvetica")
                    .unwrap()
                    .to_vec(),
            }],
            annotation_anchors: Vec::new(),
            figures: Vec::new(),
            figure_removals: Vec::new(),
            figure_detachments: Vec::new(),
            source_tags: None,
            table_layout: None,
            allow_font_substitution: false,
            allow_page_creation: true,
            prune_empty_pages: false,
            max_new_pages: 8,
            mode: StoryMode::FlowDocument,
            signature_policy_override: false,
        }
    }
    #[test]
    fn story_growth_and_contraction_preserve_all_logical_lines() {
        let request = request("The contract clause grows with additional words. ".repeat(12));
        let preview = layout(&request, &request.fonts, &[0], Vec::new()).unwrap();
        assert!(preview.generated_pages > 0);
        assert_eq!(
            preview
                .frames
                .iter()
                .flat_map(|f| &f.lines)
                .map(|l| l.text.as_str())
                .collect::<String>(),
            request.paragraphs[0].text
        );
        for frame in &preview.frames {
            for line in &frame.lines {
                let visible = line
                    .text
                    .trim_end_matches(crate::fonts::hard_break::is_hard_break);
                let shaped = TextShaper::shape_resolved(
                    &request.fonts[line.font_index].bytes,
                    visible,
                    line.bidi.as_ref().unwrap(),
                    &line.shaping,
                )
                .unwrap();
                let metric = crate::fonts::line_layout::measure_run(
                    &request.fonts[line.font_index].bytes,
                    &shaped,
                    line.font_size,
                )
                .unwrap();
                assert!(line.baseline - metric.descent >= frame.frame.rect[1] - 1e-7);
                assert!(line.baseline + metric.ascent <= frame.frame.rect[3] + 1e-7);
                assert!(metric.width() <= frame.frame.rect[2] - frame.frame.rect[0] + 1e-7);
            }
        }
        let short = self::request("Short clause".into());
        let short_preview = layout(&short, &short.fonts, &[0], Vec::new()).unwrap();
        assert_eq!(short_preview.generated_pages, 0);
        assert!(short_preview.frames[1].lines.is_empty());
    }
    #[test]
    fn occupied_frames_apply_reopen_and_exact_session_undo() {
        use crate::authoring::{FontFace, PageSize, PdfBuilder, TextStyle};
        let mut builder = PdfBuilder::new();
        for _ in 0..2 {
            builder
                .add_page(PageSize::custom(200.0, 200.0))
                .draw_text(
                    "OLD",
                    10.0,
                    50.0,
                    &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
                )
                .unwrap();
        }
        let input = builder.to_bytes().unwrap();
        let mut request =
            request("A replacement clause flows through both occupied frames. ".repeat(5));
        request.input_sha256 = hash(&input);
        let mut session = LinkedStorySession::open(input.clone()).unwrap();
        let cancel = crate::cancel::CancelToken::new();
        let preview = session.preview(&request, &cancel).unwrap();
        assert!(!preview.frames[1].lines.is_empty());
        let receipt = session.preview_receipt().unwrap();
        let mut changed = request.clone();
        changed.paragraphs[0]
            .text
            .push_str(" changed after preview");
        assert!(session
            .checkpoint_approved(&changed, &receipt, &cancel)
            .is_err());
        assert_eq!(session.bytes(), input.as_slice());
        session
            .checkpoint_approved(&request, &receipt, &cancel)
            .unwrap();
        assert!(session
            .checkpoint_approved(&request, &receipt, &cancel)
            .is_err());
        assert_ne!(session.bytes(), input.as_slice());
        let saved = session.saved_stories().unwrap();
        assert!(saved[0]
            .request
            .frames
            .iter()
            .all(|frame| frame.owner.is_some()));
        assert!(!session.document().get_page_text(1).unwrap().contains("OLD"));
        assert!(session.preview(&request, &cancel).is_err()); // old revision is stale
        assert!(session.undo().unwrap());
        assert_eq!(session.bytes(), input.as_slice());
        assert!(session.redo().unwrap());
        assert!(!session.document().get_page_text(1).unwrap().contains("OLD"));
        let base = session.saved_stories().unwrap().remove(0).request;
        let mut edited = base.clone();
        edited.paragraphs[0].text = "Merged replacement survives reopen.".into();
        let merge = crate::story_structure_merge::merge_story_structure(
            &crate::story_structure_merge::StoryStructureMergeRequest {
                base: base.clone(),
                branches: vec![crate::story_structure_merge::StoryStructureBranch {
                    branch_id: "offline".into(),
                    base_story_sha256: crate::story_merge::story_fingerprint(&base).unwrap(),
                    proposed: edited,
                }],
            },
        )
        .unwrap()
        .merged
        .unwrap();
        session.preview(&merge, &cancel).unwrap();
        let receipt = session.preview_receipt().unwrap();
        session
            .checkpoint_approved(&merge, &receipt, &cancel)
            .unwrap();
        assert_eq!(
            session.saved_stories().unwrap()[0].request.paragraphs[0].text,
            "Merged replacement survives reopen."
        );
    }

    pub(crate) fn two_page_input() -> Vec<u8> {
        use crate::authoring::{FontFace, PageSize, PdfBuilder, TextStyle};
        let mut builder = PdfBuilder::new();
        for _ in 0..2 {
            builder
                .add_page(PageSize::custom(200.0, 200.0))
                .draw_text(
                    "OLD",
                    10.0,
                    50.0,
                    &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
                )
                .unwrap();
        }
        builder.to_bytes().unwrap()
    }

    #[test]
    fn saved_empty_frame_can_be_refilled_without_a_source_text_anchor() {
        let input = two_page_input();
        let mut first = request(String::new());
        first.input_sha256 = hash(&input);
        let (empty, report) = apply_linked_story(&input, &first).unwrap();
        assert_eq!(report.rebound_frames.len(), 2);
        let mut saved = load_linked_stories(&empty).unwrap().remove(0).request;
        assert_eq!(saved.input_sha256, hash(&empty));
        saved.allow_font_substitution = true;
        saved.paragraphs[0].text = "A saved empty story becomes editable again. ".repeat(4);
        let (refilled, _) = apply_linked_story(&empty, &saved).unwrap();
        let reopened = ContentEngine::open_bytes(refilled.clone()).unwrap();
        assert!(reopened.get_page_text(1).unwrap().contains("saved"));
        assert!(!reopened.get_page_text(1).unwrap().contains("OLD"));
        assert_eq!(load_linked_stories(&refilled).unwrap().len(), 1);
        saved.input_sha256 = hash(&refilled);
        assert!(apply_linked_story(&refilled, &saved).is_err()); // old digest cannot be replayed
    }

    #[test]
    fn shared_page_content_is_cloned_before_story_mutation() {
        use crate::writer::{write_incremental_update, IncrementalObject};
        let input = two_page_input();
        let engine = ContentEngine::open_bytes(input).unwrap();
        let reader = engine.document().reader();
        let first = engine.document().get_page(1).unwrap();
        let second = engine.document().get_page(2).unwrap();
        let mut dict = reader
            .get_object(second.object_number, second.generation_number)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        dict.insert(
            "Contents",
            crate::PdfObject::Array(
                first
                    .contents
                    .iter()
                    .map(|&(number, generation)| crate::PdfObject::Reference { number, generation })
                    .collect(),
            ),
        );
        let shared = write_incremental_update(
            reader,
            vec![IncrementalObject {
                number: second.object_number,
                generation: second.generation_number,
                object: crate::PdfObject::Dictionary(dict),
            }],
        )
        .unwrap();
        let mut edit = request("NEW".into());
        edit.frames.truncate(1);
        edit.input_sha256 = hash(&shared);
        let (output, _) = apply_linked_story(&shared, &edit).unwrap();
        let reopened = ContentEngine::open_bytes(output).unwrap();
        assert!(reopened.get_page_text(1).unwrap().contains("NEW"));
        assert!(reopened.get_page_text(2).unwrap().contains("OLD"));
    }

    #[test]
    fn incremental_layout_converges_to_full_layout() {
        let mut original = request("ALPHA".into());
        for (id, text) in [("p2", "BETA"), ("p3", "GAMMA")] {
            let mut p = original.paragraphs[0].clone();
            p.id = id.into();
            p.text = text.into();
            original.paragraphs.push(p);
        }
        let previous = layout(&original, &original.fonts, &[0, 0, 0], Vec::new()).unwrap();
        let hashes = original
            .paragraphs
            .iter()
            .map(value_hash)
            .collect::<Result<Vec<_>>>()
            .unwrap();
        let mut changed = original.clone();
        changed.paragraphs[1].text = "ZETA".into();
        let incremental = layout_seeded(
            &changed,
            &changed.fonts,
            &[0, 0, 0],
            Vec::new(),
            Some(LayoutSeed {
                previous: &previous,
                paragraph_hashes: &hashes,
                font_indices: &[0, 0, 0],
            }),
        )
        .unwrap();
        let full = layout(&changed, &changed.fonts, &[0, 0, 0], Vec::new()).unwrap();
        assert!(incremental.reused_paragraphs > 0);
        assert_eq!(
            serde_json::to_value(&incremental.frames).unwrap(),
            serde_json::to_value(&full.frames).unwrap()
        );
    }

    #[test]
    fn widow_adjustment_does_not_create_an_orphan() {
        let mut edit = request("one\ntwo\nthree".into());
        edit.frames[0].rect[3] = 40.0; // two lines, but a 2/2 split is impossible
        let preview = layout(&edit, &edit.fonts, &[0], Vec::new()).unwrap();
        assert!(preview.frames[0].lines.is_empty());
        assert_eq!(preview.frames[1].lines.len(), 3);
    }

    #[test]
    fn keep_chain_reserves_the_following_keep_together_paragraph() {
        let mut edit = request("Heading".into());
        edit.paragraphs[0].keep_with_next = true;
        let mut second = edit.paragraphs[0].clone();
        second.id = "subheading".into();
        second.text = "Subheading".into();
        let mut body = edit.paragraphs[0].clone();
        body.id = "body".into();
        body.text = "one\ntwo\nthree".into();
        body.keep_with_next = false;
        body.keep_together = true;
        edit.paragraphs.extend([second, body]);
        edit.frames[1].rect[3] = 120.0;
        let preview = layout(&edit, &edit.fonts, &[0, 0, 0], Vec::new()).unwrap();
        assert!(preview.frames[0].lines.is_empty());
        assert_eq!(preview.frames[1].lines.len(), 5);
    }

    #[test]
    fn cancelled_preview_does_not_change_session_bytes_or_history() {
        let input = two_page_input();
        let mut edit = request("NEW".into());
        edit.input_sha256 = hash(&input);
        let mut session = LinkedStorySession::open(input.clone()).unwrap();
        let cancel = crate::cancel::CancelToken::new();
        cancel.cancel();
        assert!(session.preview(&edit, &cancel).is_err());
        assert_eq!(session.bytes(), input.as_slice());
        assert!(!session.undo().unwrap());
    }

    #[test]
    fn consecutive_empty_breaks_materialize_all_requested_frames() {
        let mut edit = request("first".into());
        edit.frames.truncate(1);
        let mut empty = edit.paragraphs[0].clone();
        empty.id = "empty".into();
        empty.text.clear();
        empty.break_before = true;
        let mut last = empty.clone();
        last.id = "last".into();
        last.text = "last".into();
        edit.paragraphs.extend([empty, last]);
        let preview = layout(&edit, &edit.fonts, &[0, 0, 0], Vec::new()).unwrap();
        assert_eq!(preview.frames.len(), 3);
        assert!(preview.frames[1].lines.is_empty());
        assert_eq!(preview.frames[2].lines[0].text, "last");
        edit.max_new_pages = 1;
        assert!(layout(&edit, &edit.fonts, &[0, 0, 0], Vec::new()).is_err());
    }

    #[test]
    fn differing_frame_widths_never_commit_a_one_line_widow() {
        let mut edit =
            request("alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu".into());
        edit.frames[0].rect = [10.0, 10.0, 95.0, 52.0];
        edit.frames[1].rect = [10.0, 10.0, 190.0, 180.0];
        let preview = layout(&edit, &edit.fonts, &[0], Vec::new()).unwrap();
        let occupied = preview
            .frames
            .iter()
            .filter(|f| !f.lines.is_empty())
            .collect::<Vec<_>>();
        if occupied.len() > 1 {
            assert!(occupied.last().unwrap().lines.len() >= 2);
        }
        assert_eq!(
            occupied
                .iter()
                .flat_map(|f| &f.lines)
                .map(|l| l.text.as_str())
                .collect::<String>(),
            edit.paragraphs[0].text
        );
    }

    #[test]
    fn story_wide_fonts_survive_subsets_and_repeated_edits_retire_resources() {
        let input = two_page_input();
        let mut edit = request("AAAA AAAA\nBBBB BBBB\nCCCC CCCC\nDDDD DDDD".into());
        edit.frames[0].rect[3] = 40.0;
        edit.frames[1].rect[3] = 40.0;
        edit.input_sha256 = hash(&input);
        let (mut output, _) = apply_linked_story(&input, &edit).unwrap();
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        assert!(saved.fonts.is_empty());
        assert!(resolve_fonts(&output, &saved).is_ok());
        for i in 0..8 {
            let mut saved = load_linked_stories(&output).unwrap().remove(0).request;
            saved.paragraphs[0].text = format!("AAAA {i}\nBBBB {i}\nCCCC {i}\nDDDD {i}");
            output = apply_linked_story(&output, &saved).unwrap().0;
            let reopened = ContentEngine::open_bytes(output.clone()).unwrap();
            for frame in &saved.frames {
                let resources = reopened.get_page_resources(frame.page).unwrap();
                let owned = resources
                    .fonts
                    .iter()
                    .filter(|(_, d)| d.contains_key("WFStoryOwner"))
                    .count();
                assert!(owned <= 1, "obsolete active story fonts accumulated");
            }
        }
    }

    #[test]
    fn inline_style_font_resolution_flows_into_native_paint_partitions() {
        let mut edit = request("AB".into());
        edit.frames.truncate(1);
        edit.paragraphs[0].preferred_font = "Helvetica".into();
        edit.paragraphs[0].inline_styles = vec![StoryInlineStyleSpan {
            logical_range: [1, 2],
            preferred_font: Some("Times-Roman".into()),
            font_size: Some(16.0),
            rgb: Some([0.8, 0.1, 0.2]),
            shaping: None,
        }];
        edit.fonts = ["Helvetica", "Times-Roman"]
            .into_iter()
            .map(|name| ApprovedFontAsset {
                lookup_name: name.into(),
                bytes: crate::render::get_fallback_font(name).unwrap().to_vec(),
            })
            .collect();
        validate_paragraphs(&edit).unwrap();
        let (indices, choices) = choose_fonts(&edit, &edit.fonts).unwrap();
        assert_eq!(indices.len(), 1);
        assert!(choices.iter().any(|choice| {
            choice.logical_byte_range == Some([0, 1]) && choice.selected == "Helvetica"
        }));
        assert!(choices.iter().any(|choice| {
            choice.logical_byte_range == Some([1, 2]) && choice.selected == "Times-Roman"
        }));
        let preview = layout(&edit, &edit.fonts, &indices, choices).unwrap();
        let line = &preview.frames[0].lines[0];
        assert_eq!(line.style_spans.len(), 2);
        assert_eq!(line.style_spans[0].range, [0, 1]);
        assert_eq!(line.style_spans[1].range, [1, 2]);
        assert_eq!(line.style_spans[1].font_size, 16.0);
        assert_eq!(line.style_spans[1].rgb, [0.8, 0.1, 0.2]);
        let input = two_page_input();
        let frame = &preview.frames[0];
        let output = replace_story_frame(
            &input,
            1,
            [0, 3],
            frame.frame.rect,
            &frame.lines,
            &frame.decorations,
            &edit.fonts,
            false,
            &frame_key(&edit.story_id, &frame.frame.id),
            None,
        )
        .unwrap()
        .bytes;
        assert!(ContentEngine::open_bytes(output)
            .unwrap()
            .get_page_text(1)
            .unwrap()
            .contains("AB"));
    }

    #[test]
    fn mixed_font_contextual_runs_use_same_measurement_and_writer() {
        let full = crate::render::get_fallback_font("Symbol").unwrap();
        let face = ttf_parser::Face::parse(full, 0).unwrap();
        let subset = |name: &str, text: &str| {
            let gids = text
                .chars()
                .filter_map(|c| face.glyph_index(c))
                .map(|g| g.0)
                .collect();
            ApprovedFontAsset {
                lookup_name: name.into(),
                bytes: crate::fonts::sfnt_subset::subset_glyf_preserving_gids(full, &gids)
                    .unwrap()
                    .bytes,
            }
        };
        let mut edit = request("ABC אבג".into());
        edit.frames.truncate(1);
        edit.allow_font_substitution = true;
        edit.paragraphs[0].preferred_font = "Latin".into();
        edit.fonts = vec![subset("Latin", "ABC "), subset("Hebrew", "אבג ")];
        let (indices, choices) = choose_fonts(&edit, &edit.fonts).unwrap();
        assert!(choices.iter().any(|c| c.selected == "Latin"));
        assert!(choices.iter().any(|c| c.selected == "Hebrew"));
        let preview = layout(&edit, &edit.fonts, &indices, choices).unwrap();
        let input = two_page_input();
        let frame = &preview.frames[0];
        let output = replace_story_frame(
            &input,
            1,
            [0, 3],
            frame.frame.rect,
            &frame.lines,
            &frame.decorations,
            &edit.fonts,
            false,
            &frame_key(&edit.story_id, &frame.frame.id),
            None,
        )
        .unwrap()
        .bytes;
        let text = ContentEngine::open_bytes(output)
            .unwrap()
            .get_page_text(1)
            .unwrap();
        assert!(text.contains("ABC אבג"), "{text}");
    }
}
