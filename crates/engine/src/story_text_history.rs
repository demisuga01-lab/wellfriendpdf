//! Revision-bound logical text history. RGA insertion trees + observed atom
//! tombstones, not PDF-byte merging. A projection is a draft, never approval.
use crate::linked_stories::{LinkedStoryRequest, StoryPageBreakBefore, StoryParagraph};
use crate::{Result, WellfriendError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use unicode_segmentation::UnicodeSegmentation;

const MAX_OPERATIONS: usize = 100_000;
const MAX_ATOMS: usize = 1_000_000;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORK: usize = 8_000_000;
const MAX_CLOCK: u64 = 9_007_199_254_740_991; // Exact in JSON/JavaScript.
const NONE: usize = usize::MAX;
pub(crate) const HISTORY_SEED_SCHEMA_MAX: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryOperationId {
    pub actor: String,
    pub sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryAtomId {
    /// None denotes an atom in this paragraph's immutable base text.
    pub operation: Option<StoryOperationId>,
    /// Unicode scalar offset, not UTF-8 bytes or UTF-16 code units.
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryAtomRange {
    pub operation: Option<StoryOperationId>,
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryTextOperation {
    pub id: StoryOperationId,
    pub lamport: u64,
    /// Complete causal version vector observed when this edit was authored.
    /// The actor's own entry is sequence - 1 (omitted for its first edit).
    #[serde(deserialize_with = "unique_context")]
    pub context: BTreeMap<String, u64>,
    pub paragraph_id: String,
    pub after: Option<StoryAtomId>,
    /// Only observed atoms are removed; unseen concurrent insertions survive.
    pub removed: Vec<StoryAtomRange>,
    /// One insertion chain, keeping a pasted run together at concurrent gaps.
    pub inserted: String,
    /// Schema-2 control event. Payload must otherwise contain no text edit.
    /// Only the target's own replica may change its activity; this is not auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<StoryOperationVisibility>,
    /// Schema-3 paragraph formatting/pagination operation. Text and style
    /// payloads are mutually exclusive; concurrent writes merge per field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paragraph_style: Option<StoryParagraphStylePatch>,
    /// Schema-4 paragraph membership/position register operation. Insertions
    /// carry the immutable paragraph preimage; later text/style edits retain
    /// the same paragraph ID through native save/reopen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paragraph_structure: Option<StoryParagraphStructurePatch>,
    /// Schema-5 formatting captured on newly inserted scalar atoms. Values are
    /// deltas from the paragraph style and survive later text reflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inserted_style: Option<StoryResolvedInlineStyle>,
    /// Schema-5 atom-targeted inline formatting. Compact targets use the same
    /// stable identities as deletion, never transient byte offsets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline_style: Option<StoryInlineStyleOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryOperationVisibility {
    pub target: StoryOperationId,
    pub active: bool,
}

/// Selective undo/redo of one original edit. Compare-and-swap is against both
/// the complete history and its current activity, never a text search.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryHistorySetActive {
    pub expected_history_sha256: String,
    pub actor: String,
    pub target: StoryOperationId,
    pub expected_active: bool,
    pub active: bool,
}

/// Atomic selective undo/redo of an explicit keyboard or editor transaction.
/// The target list is canonicalized by operation identity, must contain only
/// original edits from one replica, and is compare-and-swapped against one
/// complete history/activity preimage.  No partially applied group is returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryHistorySetManyActive {
    pub expected_history_sha256: String,
    pub actor: String,
    pub targets: Vec<StoryOperationId>,
    pub expected_active: bool,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryTextHistory {
    pub schema_version: u32,
    pub base_revision_sha256: String,
    pub base_story_sha256: String,
    pub operations: Vec<StoryTextOperation>,
}

/// Immutable text identities survive layout/source rebinding. Font assets and
/// physical source ranges are NOT copied into this seed or replayed after save.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryHistorySeed {
    pub schema_version: u32,
    pub story_id: String,
    pub base_revision_sha256: String,
    pub base_story_sha256: String,
    pub paragraphs: Vec<StorySeedParagraph>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorySeedParagraph {
    pub id: String,
    pub text: String,
    /// Schema-2 seeds preserve the exact pre-history paragraph style so a
    /// later selective undo does not use an already-styled saved projection as
    /// its baseline. Absent only in legacy schema-1 checkpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<StoryParagraphStyleSeed>,
    /// Schema-3 seeds preserve materialized inline overrides independently of
    /// the later causal operation log.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_styles: Vec<crate::linked_stories::StoryInlineStyleSpan>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryParagraphStyleSeed {
    pub preferred_font: String,
    pub font_size: f64,
    pub line_height: f64,
    pub rgb: [f64; 3],
    pub rtl: bool,
    pub keep_with_next: bool,
    pub keep_together: bool,
    pub break_before: bool,
    pub page_break_before: StoryPageBreakBefore,
    pub orphans: usize,
    pub widows: usize,
    pub space_before: f64,
    pub space_after: f64,
    pub shaping: crate::fonts::shaper::OpenTypeSettings,
    pub line_break: crate::fonts::line_break_policy::LineBreakSettings,
    #[serde(
        default,
        skip_serializing_if = "crate::fonts::tab_stops::TabStops::is_default"
    )]
    pub tab_stops: crate::fonts::tab_stops::TabStops,
}

impl StoryParagraphStyleSeed {
    fn capture(paragraph: &StoryParagraph) -> Self {
        Self {
            preferred_font: paragraph.preferred_font.clone(),
            font_size: paragraph.font_size,
            line_height: paragraph.line_height,
            rgb: paragraph.rgb,
            rtl: paragraph.rtl,
            keep_with_next: paragraph.keep_with_next,
            keep_together: paragraph.keep_together,
            break_before: paragraph.break_before,
            page_break_before: paragraph.page_break_before,
            orphans: paragraph.orphans,
            widows: paragraph.widows,
            space_before: paragraph.space_before,
            space_after: paragraph.space_after,
            shaping: paragraph.shaping.clone(),
            line_break: paragraph.line_break.clone(),
            tab_stops: paragraph.tab_stops.clone(),
        }
    }

    fn restore(&self, paragraph: &mut StoryParagraph) {
        paragraph.preferred_font = self.preferred_font.clone();
        paragraph.font_size = self.font_size;
        paragraph.line_height = self.line_height;
        paragraph.rgb = self.rgb;
        paragraph.rtl = self.rtl;
        paragraph.keep_with_next = self.keep_with_next;
        paragraph.keep_together = self.keep_together;
        paragraph.break_before = self.break_before;
        paragraph.page_break_before = self.page_break_before;
        paragraph.orphans = self.orphans;
        paragraph.widows = self.widows;
        paragraph.space_before = self.space_before;
        paragraph.space_after = self.space_after;
        paragraph.shaping = self.shaping.clone();
        paragraph.line_break = self.line_break.clone();
        paragraph.tab_stops = self.tab_stops.clone();
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryParagraphStylePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtl: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_with_next: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_together: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_before: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_break_before: Option<StoryPageBreakBefore>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orphans: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widows: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_before: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_after: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shaping: Option<crate::fonts::shaper::OpenTypeSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_break: Option<crate::fonts::line_break_policy::LineBreakSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_stops: Option<crate::fonts::tab_stops::TabStops>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryHistoryStyleEdit {
    pub expected_history_sha256: String,
    /// Hash of the complete current style-conflict report. This prevents a
    /// resolution from silently ignoring a newly arrived concurrent writer.
    pub expected_style_conflicts_sha256: String,
    pub actor: String,
    pub paragraph_id: String,
    /// Exact current values for every non-conflicting field being changed.
    /// Conflicting fields are omitted and instead bound by the report hash.
    #[serde(default)]
    pub expected: StoryParagraphStylePatch,
    pub replacement: StoryParagraphStylePatch,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryParagraphStructurePatch {
    /// Presence is a causal multi-value register. `false` is a logical delete;
    /// source removal still occurs only through the reviewed native checkpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub present: Option<bool>,
    /// A nested option is avoided so the root position is represented by an
    /// object with `after: null`, while omission means no position write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<StoryParagraphPosition>,
    /// Present only on the one immutable insertion operation that introduces
    /// this paragraph ID. It must be paired with present=true and a position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inserted_paragraph: Option<StoryParagraph>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryParagraphPosition {
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryHistoryStructureEdit {
    pub expected_history_sha256: String,
    pub expected_structure_conflicts_sha256: String,
    pub actor: String,
    pub paragraph_id: String,
    /// Required only when introducing a new paragraph ID.
    #[serde(default)]
    pub expected_absent: bool,
    #[serde(default)]
    pub expected: StoryParagraphStructurePatch,
    pub replacement: StoryParagraphStructurePatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryInlineStyleField {
    PreferredFont,
    FontSize,
    Rgb,
    Shaping,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryInlineStylePatch {
    /// Clearing a field restores inheritance from the paragraph style. A field
    /// may not be both cleared and assigned by the same operation.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub clear: BTreeSet<StoryInlineStyleField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shaping: Option<crate::fonts::shaper::OpenTypeSettings>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryResolvedInlineStyle {
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
#[serde(deny_unknown_fields)]
pub struct StoryInlineStyleOperation {
    pub targets: Vec<StoryAtomRange>,
    pub patch: StoryInlineStylePatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryHistoryInlineStyleEdit {
    pub expected_history_sha256: String,
    pub expected_inline_conflicts_sha256: String,
    pub actor: String,
    pub paragraph_id: String,
    /// UTF-8 byte range in the current projection, on grapheme boundaries.
    pub range: [usize; 2],
    pub expected_text: String,
    pub replacement: StoryInlineStylePatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryHistoryInlineStyleResolution {
    pub expected_history_sha256: String,
    pub expected_inline_conflicts_sha256: String,
    pub actor: String,
    pub paragraph_id: String,
    /// Must exactly cover every conflicting atom for the one replacement field.
    pub targets: Vec<StoryAtomRange>,
    pub replacement: StoryInlineStylePatch,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoryInlineStyleRun {
    pub paragraph_id: String,
    pub logical_range: [usize; 2],
    pub style: StoryResolvedInlineStyle,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoryInlineStyleConflictCandidate {
    pub operation: StoryOperationId,
    /// `null` means inherit the paragraph value.
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoryInlineStyleConflict {
    pub paragraph_id: String,
    pub target: StoryAtomId,
    pub field: StoryInlineStyleField,
    pub candidates: Vec<StoryInlineStyleConflictCandidate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoryStyleConflictCandidate {
    pub operation: StoryOperationId,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoryStyleConflict {
    pub paragraph_id: String,
    pub field: String,
    pub candidates: Vec<StoryStyleConflictCandidate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoryStructureConflictCandidate {
    pub operation: Option<StoryOperationId>,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoryStructureConflict {
    pub paragraph_id: String,
    pub field: String,
    pub candidates: Vec<StoryStructureConflictCandidate>,
}

pub(crate) fn seed_from_base(base: &LinkedStoryRequest) -> Result<StoryHistorySeed> {
    let header = new_history(base)?;
    let schema_version = if base
        .paragraphs
        .iter()
        .any(|paragraph| paragraph.tab_stops.has_extended_decimal())
    {
        6
    } else if base
        .paragraphs
        .iter()
        .any(|paragraph| paragraph.tab_stops.has_decorations())
    {
        5
    } else if base
        .paragraphs
        .iter()
        .any(|paragraph| !paragraph.tab_stops.is_default())
    {
        4
    } else {
        3
    };
    Ok(StoryHistorySeed {
        schema_version,
        story_id: base.story_id.clone(),
        base_revision_sha256: header.base_revision_sha256,
        base_story_sha256: header.base_story_sha256,
        paragraphs: base
            .paragraphs
            .iter()
            .map(|p| StorySeedParagraph {
                id: p.id.clone(),
                text: p.text.clone(),
                style: Some(StoryParagraphStyleSeed::capture(p)),
                inline_styles: p.inline_styles.clone(),
            })
            .collect(),
    })
}
fn seed_header(seed: &StoryHistorySeed) -> StoryTextHistory {
    StoryTextHistory {
        schema_version: 1,
        base_revision_sha256: seed.base_revision_sha256.clone(),
        base_story_sha256: seed.base_story_sha256.clone(),
        operations: Vec::new(),
    }
}
fn seed_template(
    seed: &StoryHistorySeed,
    current: &LinkedStoryRequest,
) -> Result<LinkedStoryRequest> {
    if !(1..=HISTORY_SEED_SCHEMA_MAX).contains(&seed.schema_version)
        || seed.story_id != current.story_id
        || seed.base_revision_sha256.is_empty()
        || seed.base_story_sha256.len() != 64
    {
        return Err(fail("seed schema/story identity changed"));
    }
    if seed.schema_version < 4
        && seed.paragraphs.iter().any(|paragraph| {
            paragraph
                .style
                .as_ref()
                .is_some_and(|style| !style.tab_stops.is_default())
        })
    {
        return Err(fail("tab-stop history requires seed schema 4"));
    }
    if seed.schema_version < 5
        && seed.paragraphs.iter().any(|paragraph| {
            paragraph
                .style
                .as_ref()
                .is_some_and(|style| style.tab_stops.has_decorations())
        })
    {
        return Err(fail("decorated tab-stop history requires seed schema 5"));
    }
    if seed.schema_version < 6
        && seed.paragraphs.iter().any(|paragraph| {
            paragraph
                .style
                .as_ref()
                .is_some_and(|style| style.tab_stops.has_extended_decimal())
        })
    {
        return Err(fail(
            "multi-character decimal tab history requires seed schema 6",
        ));
    }
    let mut template = current.clone();
    if seed.schema_version == 1 {
        if seed.paragraphs.len() != template.paragraphs.len() {
            return Err(fail(
                "legacy seed paragraph count changed; start an explicitly approved replacement epoch",
            ));
        }
        for (original, paragraph) in seed.paragraphs.iter().zip(&mut template.paragraphs) {
            if original.id != paragraph.id
                || original.style.is_some()
                || !original.inline_styles.is_empty()
            {
                return Err(fail("legacy seed paragraph identity/schema mismatch"));
            }
            paragraph.text = original.text.clone();
        }
    } else {
        let mut paragraphs = Vec::with_capacity(seed.paragraphs.len());
        for original in &seed.paragraphs {
            let style = original
                .style
                .as_ref()
                .ok_or_else(|| fail("seed paragraph style/schema mismatch"))?;
            let mut paragraph = StoryParagraph {
                id: original.id.clone(),
                text: original.text.clone(),
                preferred_font: String::new(),
                font_size: 1.0,
                line_height: 1.0,
                rgb: [0.0; 3],
                rtl: false,
                keep_with_next: false,
                keep_together: false,
                break_before: false,
                page_break_before: StoryPageBreakBefore::None,
                orphans: 1,
                widows: 1,
                space_before: 0.0,
                space_after: 0.0,
                shaping: Default::default(),
                inline_styles: original.inline_styles.clone(),
                line_break: Default::default(),
                tab_stops: Default::default(),
            };
            style.restore(&mut paragraph);
            paragraphs.push(paragraph);
        }
        template.paragraphs = paragraphs;
    }
    validate_base(&template)?;
    Ok(template)
}

/// Only the persisted-lineage coordinator may project a seed onto rebound
/// source owners. It verifies the last saved model, epoch and current revision
/// before invoking these helpers; ordinary histories keep their original gate.
pub(crate) fn merge_seed(
    seed: &StoryHistorySeed,
    current: &LinkedStoryRequest,
    histories: &[StoryTextHistory],
) -> Result<StoryHistoryResult> {
    if seed.schema_version < 2
        && histories
            .iter()
            .flat_map(|history| &history.operations)
            .any(|operation| {
                operation.paragraph_style.is_some()
                    || operation.paragraph_structure.is_some()
                    || operation.inline_style.is_some()
                    || operation.inserted_style.is_some()
            })
    {
        return Err(fail(
            "paragraph style/structure/inline history requires a schema-2 seed; start an explicitly approved replacement epoch",
        ));
    }
    let template = seed_template(seed, current)?;
    Ok(reconcile_bound(&template, histories, seed_header(seed))?.0)
}
pub(crate) fn edit_seed(
    seed: &StoryHistorySeed,
    current: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryEdit,
) -> Result<StoryHistoryResult> {
    reject_legacy_extended_history(seed, history)?;
    let template = seed_template(seed, current)?;
    edit_bound(&template, history, edit, Some(seed))
}

pub(crate) fn edit_style_seed(
    seed: &StoryHistorySeed,
    current: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryStyleEdit,
) -> Result<StoryHistoryResult> {
    reject_legacy_extended_history(seed, history)?;
    if seed.schema_version < 2 {
        return Err(fail(
            "paragraph-style history requires an explicitly approved replacement epoch",
        ));
    }
    let template = seed_template(seed, current)?;
    edit_style_bound(&template, history, edit, Some(seed))
}

pub(crate) fn edit_structure_seed(
    seed: &StoryHistorySeed,
    current: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryStructureEdit,
) -> Result<StoryHistoryResult> {
    reject_legacy_extended_history(seed, history)?;
    if seed.schema_version < 2 {
        return Err(fail(
            "paragraph-structure history requires an explicitly approved replacement epoch",
        ));
    }
    let template = seed_template(seed, current)?;
    edit_structure_bound(&template, history, edit, Some(seed))
}

pub(crate) fn edit_inline_style_seed(
    seed: &StoryHistorySeed,
    current: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryInlineStyleEdit,
) -> Result<StoryHistoryResult> {
    reject_legacy_extended_history(seed, history)?;
    if seed.schema_version < 2 {
        return Err(fail(
            "inline-style history requires an explicitly approved replacement epoch",
        ));
    }
    let template = seed_template(seed, current)?;
    edit_inline_style_bound(&template, history, edit, Some(seed))
}

pub(crate) fn resolve_inline_style_conflicts_seed(
    seed: &StoryHistorySeed,
    current: &LinkedStoryRequest,
    history: &StoryTextHistory,
    resolution: &StoryHistoryInlineStyleResolution,
) -> Result<StoryHistoryResult> {
    reject_legacy_extended_history(seed, history)?;
    if seed.schema_version < 2 {
        return Err(fail(
            "inline-style history requires an explicitly approved replacement epoch",
        ));
    }
    let template = seed_template(seed, current)?;
    resolve_inline_style_conflicts_bound(&template, history, resolution, Some(seed))
}
pub(crate) fn set_active_seed(
    seed: &StoryHistorySeed,
    current: &LinkedStoryRequest,
    history: &StoryTextHistory,
    change: &StoryHistorySetActive,
) -> Result<StoryHistoryResult> {
    reject_legacy_extended_history(seed, history)?;
    let template = seed_template(seed, current)?;
    set_active_bound(&template, history, change, Some(seed))
}

pub(crate) fn set_many_active_seed(
    seed: &StoryHistorySeed,
    current: &LinkedStoryRequest,
    history: &StoryTextHistory,
    change: &StoryHistorySetManyActive,
) -> Result<StoryHistoryResult> {
    reject_legacy_extended_history(seed, history)?;
    let template = seed_template(seed, current)?;
    set_many_active_bound(&template, history, change, Some(seed))
}

fn reject_legacy_extended_history(
    seed: &StoryHistorySeed,
    history: &StoryTextHistory,
) -> Result<()> {
    if seed.schema_version < 2
        && history.operations.iter().any(|operation| {
            operation.paragraph_style.is_some()
                || operation.paragraph_structure.is_some()
                || operation.inline_style.is_some()
                || operation.inserted_style.is_some()
        })
    {
        return Err(fail(
            "paragraph style/structure/inline activity requires a schema-2 seed; start an explicitly approved replacement epoch",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryHistoryEdit {
    pub expected_history_sha256: String,
    /// Host-provisioned unique replica identity. This is not authentication.
    pub actor: String,
    pub paragraph_id: String,
    /// UTF-8 byte range in the current projection, on grapheme boundaries.
    pub range: [usize; 2],
    pub expected_text: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoryHistoryResult {
    pub history: StoryTextHistory,
    pub history_sha256: String,
    pub frontier: BTreeMap<String, u64>,
    /// Missing dependencies are retained in history. No partial draft is emitted.
    pub missing_dependencies: Vec<StoryOperationId>,
    pub merged: Option<LinkedStoryRequest>,
    /// Every stable paragraph identity in the epoch, including logically
    /// deleted insertion anchors needed for conflict review and later redo.
    pub paragraph_ids: Vec<String>,
    pub atom_count: usize,
    pub tombstone_count: usize,
    /// Original edits suppressed by their latest same-replica control event.
    pub inactive_operations: Vec<StoryOperationId>,
    /// Inserted atoms hidden by inactive edits, independent of deletion count.
    pub suppressed_atom_count: usize,
    /// Concurrent, causally maximal writes with different values. No complete
    /// draft is emitted until a later operation observes and resolves them.
    pub style_conflicts: Vec<StoryStyleConflict>,
    pub style_conflicts_sha256: String,
    pub structure_conflicts: Vec<StoryStructureConflict>,
    pub structure_conflicts_sha256: String,
    /// Canonical non-overlapping UTF-8 runs. Omitted fields inherit the
    /// projected paragraph style.
    pub inline_style_runs: Vec<StoryInlineStyleRun>,
    pub inline_conflicts: Vec<StoryInlineStyleConflict>,
    pub inline_conflicts_sha256: String,
    pub limits: Vec<String>,
}

const STYLE_FIELDS: [&str; 16] = [
    "preferred_font",
    "font_size",
    "line_height",
    "rgb",
    "rtl",
    "keep_with_next",
    "keep_together",
    "break_before",
    "page_break_before",
    "orphans",
    "widows",
    "space_before",
    "space_after",
    "shaping",
    "line_break",
    "tab_stops",
];

fn patch_has(patch: &StoryParagraphStylePatch, field: &str) -> bool {
    match field {
        "preferred_font" => patch.preferred_font.is_some(),
        "font_size" => patch.font_size.is_some(),
        "line_height" => patch.line_height.is_some(),
        "rgb" => patch.rgb.is_some(),
        "rtl" => patch.rtl.is_some(),
        "keep_with_next" => patch.keep_with_next.is_some(),
        "keep_together" => patch.keep_together.is_some(),
        "break_before" => patch.break_before.is_some(),
        "page_break_before" => patch.page_break_before.is_some(),
        "orphans" => patch.orphans.is_some(),
        "widows" => patch.widows.is_some(),
        "space_before" => patch.space_before.is_some(),
        "space_after" => patch.space_after.is_some(),
        "shaping" => patch.shaping.is_some(),
        "line_break" => patch.line_break.is_some(),
        "tab_stops" => patch.tab_stops.is_some(),
        _ => false,
    }
}

fn patch_fields(patch: &StoryParagraphStylePatch) -> BTreeSet<&'static str> {
    STYLE_FIELDS
        .into_iter()
        .filter(|field| patch_has(patch, field))
        .collect()
}

fn patch_value(patch: &StoryParagraphStylePatch, field: &str) -> Result<Option<serde_json::Value>> {
    let value = match field {
        "preferred_font" => patch.preferred_font.as_ref().map(serde_json::to_value),
        "font_size" => patch.font_size.as_ref().map(serde_json::to_value),
        "line_height" => patch.line_height.as_ref().map(serde_json::to_value),
        "rgb" => patch.rgb.as_ref().map(serde_json::to_value),
        "rtl" => patch.rtl.as_ref().map(serde_json::to_value),
        "keep_with_next" => patch.keep_with_next.as_ref().map(serde_json::to_value),
        "keep_together" => patch.keep_together.as_ref().map(serde_json::to_value),
        "break_before" => patch.break_before.as_ref().map(serde_json::to_value),
        "page_break_before" => patch.page_break_before.as_ref().map(serde_json::to_value),
        "orphans" => patch.orphans.as_ref().map(serde_json::to_value),
        "widows" => patch.widows.as_ref().map(serde_json::to_value),
        "space_before" => patch.space_before.as_ref().map(serde_json::to_value),
        "space_after" => patch.space_after.as_ref().map(serde_json::to_value),
        "shaping" => patch.shaping.as_ref().map(serde_json::to_value),
        "line_break" => patch.line_break.as_ref().map(serde_json::to_value),
        "tab_stops" => patch.tab_stops.as_ref().map(serde_json::to_value),
        _ => None,
    };
    value
        .transpose()
        .map_err(|_| fail("style value serialization"))
}

fn paragraph_style_value(paragraph: &StoryParagraph, field: &str) -> Result<serde_json::Value> {
    let value = match field {
        "preferred_font" => serde_json::to_value(&paragraph.preferred_font),
        "font_size" => serde_json::to_value(paragraph.font_size),
        "line_height" => serde_json::to_value(paragraph.line_height),
        "rgb" => serde_json::to_value(paragraph.rgb),
        "rtl" => serde_json::to_value(paragraph.rtl),
        "keep_with_next" => serde_json::to_value(paragraph.keep_with_next),
        "keep_together" => serde_json::to_value(paragraph.keep_together),
        "break_before" => serde_json::to_value(paragraph.break_before),
        "page_break_before" => serde_json::to_value(paragraph.page_break_before),
        "orphans" => serde_json::to_value(paragraph.orphans),
        "widows" => serde_json::to_value(paragraph.widows),
        "space_before" => serde_json::to_value(paragraph.space_before),
        "space_after" => serde_json::to_value(paragraph.space_after),
        "shaping" => serde_json::to_value(&paragraph.shaping),
        "line_break" => serde_json::to_value(&paragraph.line_break),
        "tab_stops" => serde_json::to_value(&paragraph.tab_stops),
        _ => return Err(fail("unknown paragraph-style field")),
    };
    value.map_err(|_| fail("paragraph style serialization"))
}

fn apply_style_field(
    paragraph: &mut StoryParagraph,
    patch: &StoryParagraphStylePatch,
    field: &str,
) -> Result<()> {
    match field {
        "preferred_font" => paragraph.preferred_font = patch.preferred_font.clone().unwrap(),
        "font_size" => paragraph.font_size = patch.font_size.unwrap(),
        "line_height" => paragraph.line_height = patch.line_height.unwrap(),
        "rgb" => paragraph.rgb = patch.rgb.unwrap(),
        "rtl" => paragraph.rtl = patch.rtl.unwrap(),
        "keep_with_next" => paragraph.keep_with_next = patch.keep_with_next.unwrap(),
        "keep_together" => paragraph.keep_together = patch.keep_together.unwrap(),
        "break_before" => paragraph.break_before = patch.break_before.unwrap(),
        "page_break_before" => paragraph.page_break_before = patch.page_break_before.unwrap(),
        "orphans" => paragraph.orphans = patch.orphans.unwrap(),
        "widows" => paragraph.widows = patch.widows.unwrap(),
        "space_before" => paragraph.space_before = patch.space_before.unwrap(),
        "space_after" => paragraph.space_after = patch.space_after.unwrap(),
        "shaping" => paragraph.shaping = patch.shaping.clone().unwrap(),
        "line_break" => paragraph.line_break = patch.line_break.clone().unwrap(),
        "tab_stops" => paragraph.tab_stops = patch.tab_stops.clone().unwrap(),
        _ => return Err(fail("unknown paragraph-style field")),
    }
    Ok(())
}

fn validate_style_patch(patch: &StoryParagraphStylePatch, allow_empty: bool) -> Result<()> {
    if !allow_empty && !STYLE_FIELDS.iter().any(|field| patch_has(patch, field)) {
        return Err(fail("empty paragraph-style patch"));
    }
    if patch
        .preferred_font
        .as_ref()
        .is_some_and(|font| font.is_empty() || font.len() > 1024)
        || patch
            .font_size
            .is_some_and(|value| !value.is_finite() || !(0.01..=10_000.0).contains(&value))
        || patch
            .line_height
            .is_some_and(|value| !value.is_finite() || !(0.01..=10_000.0).contains(&value))
        || patch.rgb.is_some_and(|rgb| {
            rgb.iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        })
        || patch.orphans.is_some_and(|value| value == 0 || value > 100)
        || patch.widows.is_some_and(|value| value == 0 || value > 100)
        || patch
            .space_before
            .is_some_and(|value| !value.is_finite() || !(0.0..=1_000_000.0).contains(&value))
        || patch
            .space_after
            .is_some_and(|value| !value.is_finite() || !(0.0..=1_000_000.0).contains(&value))
        || patch.shaping.as_ref().is_some_and(|settings| {
            settings.features.len() > 64
                || settings
                    .language
                    .as_ref()
                    .is_some_and(|value| value.len() > 128)
                || settings.features.iter().any(|value| value.len() > 128)
        })
    {
        return Err(fail("invalid paragraph-style patch"));
    }
    if let Some(settings) = &patch.line_break {
        settings.validate()?;
    }
    if let Some(settings) = &patch.tab_stops {
        settings.validate()?;
    }
    Ok(())
}

const STRUCTURE_FIELDS: [&str; 2] = ["present", "position"];

fn structure_patch_has(patch: &StoryParagraphStructurePatch, field: &str) -> bool {
    match field {
        "present" => patch.present.is_some(),
        "position" => patch.position.is_some(),
        _ => false,
    }
}

fn structure_patch_fields(patch: &StoryParagraphStructurePatch) -> BTreeSet<&'static str> {
    STRUCTURE_FIELDS
        .into_iter()
        .filter(|field| structure_patch_has(patch, field))
        .collect()
}

fn structure_patch_value(
    patch: &StoryParagraphStructurePatch,
    field: &str,
) -> Result<Option<serde_json::Value>> {
    let value = match field {
        "present" => patch.present.as_ref().map(serde_json::to_value),
        "position" => patch.position.as_ref().map(serde_json::to_value),
        _ => None,
    };
    value
        .transpose()
        .map_err(|_| fail("paragraph-structure value serialization"))
}

fn validate_structure_patch(
    base: &LinkedStoryRequest,
    paragraph_id: &str,
    patch: &StoryParagraphStructurePatch,
    allow_empty: bool,
) -> Result<()> {
    if !allow_empty && structure_patch_fields(patch).is_empty() {
        return Err(fail("empty paragraph-structure patch"));
    }
    if paragraph_id.is_empty() || paragraph_id.len() > 1024 {
        return Err(fail("invalid paragraph-structure target ID"));
    }
    if let Some(position) = &patch.position {
        if position
            .after
            .as_ref()
            .is_some_and(|after| after.is_empty() || after.len() > 1024 || after == paragraph_id)
        {
            return Err(fail("invalid paragraph predecessor"));
        }
    }
    if let Some(paragraph) = &patch.inserted_paragraph {
        if paragraph.id != paragraph_id || patch.present != Some(true) || patch.position.is_none() {
            return Err(fail(
                "paragraph insertion must bind its target, present=true and initial position",
            ));
        }
        let mut candidate = base.clone();
        candidate.paragraphs.push(paragraph.clone());
        crate::linked_stories::validate_paragraphs(&candidate)?;
    }
    Ok(())
}

const INLINE_STYLE_FIELDS: [StoryInlineStyleField; 4] = [
    StoryInlineStyleField::PreferredFont,
    StoryInlineStyleField::FontSize,
    StoryInlineStyleField::Rgb,
    StoryInlineStyleField::Shaping,
];

fn inline_patch_has(patch: &StoryInlineStylePatch, field: StoryInlineStyleField) -> bool {
    patch.clear.contains(&field)
        || match field {
            StoryInlineStyleField::PreferredFont => patch.preferred_font.is_some(),
            StoryInlineStyleField::FontSize => patch.font_size.is_some(),
            StoryInlineStyleField::Rgb => patch.rgb.is_some(),
            StoryInlineStyleField::Shaping => patch.shaping.is_some(),
        }
}

fn inline_patch_fields(patch: &StoryInlineStylePatch) -> BTreeSet<StoryInlineStyleField> {
    INLINE_STYLE_FIELDS
        .into_iter()
        .filter(|field| inline_patch_has(patch, *field))
        .collect()
}

fn inline_patch_value(
    patch: &StoryInlineStylePatch,
    field: StoryInlineStyleField,
) -> Result<Option<serde_json::Value>> {
    if patch.clear.contains(&field) {
        return Ok(Some(serde_json::Value::Null));
    }
    let value = match field {
        StoryInlineStyleField::PreferredFont => {
            patch.preferred_font.as_ref().map(serde_json::to_value)
        }
        StoryInlineStyleField::FontSize => patch.font_size.as_ref().map(serde_json::to_value),
        StoryInlineStyleField::Rgb => patch.rgb.as_ref().map(serde_json::to_value),
        StoryInlineStyleField::Shaping => patch.shaping.as_ref().map(serde_json::to_value),
    };
    value
        .transpose()
        .map_err(|_| fail("inline-style value serialization"))
}

fn resolved_inline_value(
    style: &StoryResolvedInlineStyle,
    field: StoryInlineStyleField,
) -> Result<Option<serde_json::Value>> {
    let value = match field {
        StoryInlineStyleField::PreferredFont => {
            style.preferred_font.as_ref().map(serde_json::to_value)
        }
        StoryInlineStyleField::FontSize => style.font_size.as_ref().map(serde_json::to_value),
        StoryInlineStyleField::Rgb => style.rgb.as_ref().map(serde_json::to_value),
        StoryInlineStyleField::Shaping => style.shaping.as_ref().map(serde_json::to_value),
    };
    value
        .transpose()
        .map_err(|_| fail("resolved inline-style serialization"))
}

fn apply_inline_field(
    style: &mut StoryResolvedInlineStyle,
    value: &serde_json::Value,
    field: StoryInlineStyleField,
) -> Result<()> {
    match field {
        StoryInlineStyleField::PreferredFont => {
            style.preferred_font = if value.is_null() {
                None
            } else {
                Some(
                    serde_json::from_value(value.clone())
                        .map_err(|_| fail("invalid preferred-font inline value"))?,
                )
            }
        }
        StoryInlineStyleField::FontSize => {
            style.font_size = if value.is_null() {
                None
            } else {
                Some(
                    serde_json::from_value(value.clone())
                        .map_err(|_| fail("invalid font-size inline value"))?,
                )
            }
        }
        StoryInlineStyleField::Rgb => {
            style.rgb = if value.is_null() {
                None
            } else {
                Some(
                    serde_json::from_value(value.clone())
                        .map_err(|_| fail("invalid colour inline value"))?,
                )
            }
        }
        StoryInlineStyleField::Shaping => {
            style.shaping = if value.is_null() {
                None
            } else {
                Some(
                    serde_json::from_value(value.clone())
                        .map_err(|_| fail("invalid shaping inline value"))?,
                )
            }
        }
    }
    Ok(())
}

fn validate_inline_patch(patch: &StoryInlineStylePatch, allow_empty: bool) -> Result<()> {
    let assigned = [
        (
            StoryInlineStyleField::PreferredFont,
            patch.preferred_font.is_some(),
        ),
        (StoryInlineStyleField::FontSize, patch.font_size.is_some()),
        (StoryInlineStyleField::Rgb, patch.rgb.is_some()),
        (StoryInlineStyleField::Shaping, patch.shaping.is_some()),
    ];
    if assigned
        .iter()
        .any(|(field, present)| *present && patch.clear.contains(field))
    {
        return Err(fail(
            "inline-style field cannot be assigned and cleared together",
        ));
    }
    if !allow_empty && inline_patch_fields(patch).is_empty() {
        return Err(fail("empty inline-style patch"));
    }
    if patch
        .preferred_font
        .as_ref()
        .is_some_and(|font| font.is_empty() || font.len() > 1024)
        || patch
            .font_size
            .is_some_and(|value| !value.is_finite() || !(0.01..=10_000.0).contains(&value))
        || patch.rgb.is_some_and(|rgb| {
            rgb.iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        })
        || patch.shaping.as_ref().is_some_and(|settings| {
            settings.features.len() > 64
                || settings
                    .language
                    .as_ref()
                    .is_some_and(|value| value.len() > 128)
                || settings.features.iter().any(|value| value.len() > 128)
        })
    {
        return Err(fail("invalid inline-style patch"));
    }
    Ok(())
}

fn validate_resolved_inline_style(style: &StoryResolvedInlineStyle) -> Result<()> {
    validate_inline_patch(
        &StoryInlineStylePatch {
            clear: BTreeSet::new(),
            preferred_font: style.preferred_font.clone(),
            font_size: style.font_size,
            rgb: style.rgb,
            shaping: style.shaping.clone(),
        },
        true,
    )
}

fn resolved_style_is_empty(style: &StoryResolvedInlineStyle) -> bool {
    style.preferred_font.is_none()
        && style.font_size.is_none()
        && style.rgb.is_none()
        && style.shaping.is_none()
}

fn validate_inline_targets(targets: &[StoryAtomRange]) -> Result<()> {
    let mut covered = BTreeMap::<Option<StoryOperationId>, Vec<(u32, u32)>>::new();
    for target in targets {
        if target.start >= target.end {
            return Err(fail("empty/reversed inline-style atom range"));
        }
        covered
            .entry(target.operation.clone())
            .or_default()
            .push((target.start, target.end));
    }
    for ranges in covered.values_mut() {
        ranges.sort_unstable();
        for pair in ranges.windows(2) {
            if pair[0].1 > pair[1].0 {
                return Err(fail("overlapping inline-style atom targets"));
            }
        }
    }
    Ok(())
}

fn fail(s: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("story text history: {s}"))
}
fn unique_context<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<BTreeMap<String, u64>, D::Error> {
    struct ContextVisitor;
    impl<'de> serde::de::Visitor<'de> for ContextVisitor {
        type Value = BTreeMap<String, u64>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a bounded version vector with unique actor keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some((name, sequence)) = map.next_entry::<String, u64>()? {
                actor(&name).map_err(serde::de::Error::custom)?;
                if sequence == 0
                    || sequence > MAX_CLOCK
                    || result.insert(name, sequence).is_some()
                    || result.len() > 256
                {
                    return Err(serde::de::Error::custom(
                        "duplicate actor or invalid/budget-exceeding causal context",
                    ));
                }
            }
            Ok(result)
        }
    }
    decoder.deserialize_map(ContextVisitor)
}
fn actor(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
    {
        return Err(fail(
            "actor IDs must be 1..128 ASCII letters, digits, dot, colon, underscore or hyphen",
        ));
    }
    Ok(())
}
fn id(value: &StoryOperationId) -> Result<()> {
    actor(&value.actor)?;
    if value.sequence == 0 || value.sequence > MAX_CLOCK {
        return Err(fail("invalid operation sequence"));
    }
    Ok(())
}
fn spend(work: &mut usize, amount: usize) -> Result<()> {
    *work = work.saturating_add(amount);
    if *work > MAX_WORK {
        return Err(fail("history expansion/causal work budget exceeded"));
    }
    crate::cancel::check_current_cancel("story text history")
}

pub fn new_history(base: &LinkedStoryRequest) -> Result<StoryTextHistory> {
    crate::cancel::check_current_cancel("story history base")?;
    validate_base(base)?;
    Ok(StoryTextHistory {
        schema_version: 1,
        base_revision_sha256: base.input_sha256.clone(),
        base_story_sha256: crate::story_merge::story_fingerprint(base)?,
        operations: Vec::new(),
    })
}
fn validate_base(base: &LinkedStoryRequest) -> Result<()> {
    if base.paragraphs.len() > 100_000 {
        return Err(fail("paragraph budget exceeded"));
    }
    let mut ids = BTreeSet::new();
    let mut bytes = 0usize;
    let mut atoms = 0usize;
    for paragraph in &base.paragraphs {
        crate::cancel::check_current_cancel("history base paragraphs")?;
        crate::linked_stories::validate_inline_style_spans(paragraph)?;
        paragraph.tab_stops.validate()?;
        if paragraph.id.is_empty() || paragraph.id.len() > 1024 || !ids.insert(&paragraph.id) {
            return Err(fail("invalid/duplicate paragraph ID"));
        }
        bytes = bytes.saturating_add(paragraph.text.len());
        atoms = atoms.saturating_add(paragraph.text.chars().count());
        if paragraph.text.len() > 4_000_000 || bytes > MAX_BYTES || atoms > MAX_ATOMS {
            return Err(fail("base text/atom budget exceeded"));
        }
    }
    Ok(())
}

pub fn merge_histories(
    base: &LinkedStoryRequest,
    histories: &[StoryTextHistory],
) -> Result<StoryHistoryResult> {
    Ok(reconcile(base, histories)?.0)
}

/// Transport delta. The receiver joins this with its retained history; a delta
/// need not be causally complete in isolation. No actor identity is trusted as
/// authentication and no receiver acknowledgement garbage-collects history.
pub fn export_delta(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    peer: &BTreeMap<String, u64>,
) -> Result<StoryTextHistory> {
    let canonical = merge_histories(base, std::slice::from_ref(history))?.history;
    delta_from_canonical(canonical, peer)
}
pub(crate) fn delta_from_canonical(
    mut canonical: StoryTextHistory,
    peer: &BTreeMap<String, u64>,
) -> Result<StoryTextHistory> {
    if peer.len() > 256 {
        return Err(fail("peer frontier budget exceeded"));
    }
    for (name, sequence) in peer {
        actor(name)?;
        if *sequence > MAX_CLOCK {
            return Err(fail("invalid peer frontier"));
        }
    }
    canonical
        .operations
        .retain(|op| op.id.sequence > peer.get(&op.id.actor).copied().unwrap_or(0));
    Ok(canonical)
}

// Internal nodes use arena indexes, never one cloned actor string per glyph.
struct Node {
    parent: usize,
    first: usize,
    next: usize,
    scalar: Option<char>,
    removed: bool,
    origin: usize,
    offset: u32,
}
struct Origin<'a> {
    paragraph: usize,
    operation: Option<&'a StoryOperationId>,
    text: &'a str,
    rank: usize,
    start: usize,
    count: usize,
    enabled: bool,
}
struct View {
    text: String,
    visible: Vec<(usize, u32)>,
}
struct Projection {
    views: BTreeMap<String, View>,
    origins: Vec<Option<StoryOperationId>>,
}

fn reconcile(
    base: &LinkedStoryRequest,
    histories: &[StoryTextHistory],
) -> Result<(StoryHistoryResult, Option<Projection>)> {
    reconcile_bound(base, histories, new_history(base)?)
}
fn reconcile_bound(
    base: &LinkedStoryRequest,
    histories: &[StoryTextHistory],
    mut history: StoryTextHistory,
) -> Result<(StoryHistoryResult, Option<Projection>)> {
    crate::cancel::check_current_cancel("bound story text history")?;
    if histories.len() > 256 {
        return Err(fail("history branch budget exceeded"));
    }
    let mut unique = BTreeMap::<StoryOperationId, StoryTextOperation>::new();
    let mut bytes = 0usize;
    let mut unique_bytes = 0usize;
    let mut work = 0usize;
    for branch in histories {
        if !matches!(branch.schema_version, 1..=5)
            || branch.base_revision_sha256 != history.base_revision_sha256
            || branch.base_story_sha256 != history.base_story_sha256
        {
            return Err(fail("history belongs to a different PDF/story base"));
        }
        if branch.operations.len() > MAX_OPERATIONS {
            return Err(fail("operation budget exceeded"));
        }
        for operation in &branch.operations {
            spend(&mut work, 1)?;
            id(&operation.id)?;
            if let Some(style) = &operation.paragraph_style {
                if branch.schema_version < 3 {
                    return Err(fail(
                        "paragraph-style operations require history schema 3, 4 or 5",
                    ));
                }
                validate_style_patch(style, false)?;
                if !operation.inserted.is_empty()
                    || !operation.removed.is_empty()
                    || operation.after.is_some()
                    || operation.visibility.is_some()
                    || operation.paragraph_structure.is_some()
                    || operation.inserted_style.is_some()
                    || operation.inline_style.is_some()
                {
                    return Err(fail(
                        "paragraph-style operation must not contain text or visibility payloads",
                    ));
                }
            }
            if let Some(structure) = &operation.paragraph_structure {
                if branch.schema_version < 4 {
                    return Err(fail(
                        "paragraph-structure operations require history schema 4 or 5",
                    ));
                }
                validate_structure_patch(base, &operation.paragraph_id, structure, false)?;
                if !operation.inserted.is_empty()
                    || !operation.removed.is_empty()
                    || operation.after.is_some()
                    || operation.visibility.is_some()
                    || operation.paragraph_style.is_some()
                    || operation.inserted_style.is_some()
                    || operation.inline_style.is_some()
                {
                    return Err(fail(
                        "paragraph-structure operation must not contain text, style or visibility payloads",
                    ));
                }
            }
            if let Some(inline) = &operation.inline_style {
                if branch.schema_version < 5 {
                    return Err(fail("inline-style operations require history schema 5"));
                }
                validate_inline_patch(&inline.patch, false)?;
                if inline.targets.is_empty() || inline.targets.len() > MAX_ATOMS {
                    return Err(fail("inline-style target budget or shape is invalid"));
                }
                validate_inline_targets(&inline.targets)?;
                if !operation.inserted.is_empty()
                    || !operation.removed.is_empty()
                    || operation.after.is_some()
                    || operation.visibility.is_some()
                    || operation.paragraph_style.is_some()
                    || operation.paragraph_structure.is_some()
                    || operation.inserted_style.is_some()
                {
                    return Err(fail(
                        "inline-style operation must not contain text, paragraph or visibility payloads",
                    ));
                }
            }
            if let Some(inserted_style) = &operation.inserted_style {
                if branch.schema_version < 5 {
                    return Err(fail("styled insertion requires history schema 5"));
                }
                validate_resolved_inline_style(inserted_style)?;
                if operation.inserted.is_empty()
                    || operation.visibility.is_some()
                    || operation.paragraph_style.is_some()
                    || operation.paragraph_structure.is_some()
                    || operation.inline_style.is_some()
                {
                    return Err(fail(
                        "inserted inline style requires a text insertion without another payload",
                    ));
                }
            }
            if let Some(visibility) = &operation.visibility {
                if branch.schema_version < 2 {
                    return Err(fail(
                        "visibility controls require history schema 2, 3, 4 or 5",
                    ));
                }
                id(&visibility.target)?;
                if visibility.target.actor != operation.id.actor
                    || visibility.target.sequence >= operation.id.sequence
                    || !operation.inserted.is_empty()
                    || !operation.removed.is_empty()
                    || operation.after.is_some()
                    || operation.paragraph_style.is_some()
                    || operation.paragraph_structure.is_some()
                    || operation.inserted_style.is_some()
                    || operation.inline_style.is_some()
                {
                    return Err(fail(
                        "visibility control must target an earlier own edit without a text payload",
                    ));
                }
            }
            if operation.inserted.len() > 4_000_000
                || operation.paragraph_id.len() > 1024
                || operation.context.len() > 256
                || operation.removed.len() > MAX_OPERATIONS
            {
                return Err(fail("operation payload budget exceeded"));
            }
            for name in operation.context.keys() {
                actor(name)?;
            }
            if let Some(anchor) = &operation.after {
                if let Some(source) = &anchor.operation {
                    id(source)?;
                }
            }
            for range in &operation.removed {
                spend(&mut work, 1)?;
                if let Some(source) = &range.operation {
                    id(source)?;
                }
            }
            if let Some(inline) = &operation.inline_style {
                for range in &inline.targets {
                    spend(&mut work, 1)?;
                    if let Some(source) = &range.operation {
                        id(source)?;
                    }
                }
            }
            let encoded_len = serde_json::to_vec(operation)
                .map_err(|_| fail("operation serialization"))?
                .len();
            bytes = bytes.saturating_add(encoded_len);
            if bytes > 2 * MAX_BYTES {
                return Err(fail("incoming encoded operation budget exceeded"));
            }
            if let Some(previous) = unique.get(&operation.id) {
                if previous != operation {
                    return Err(fail("operation ID reused with different content"));
                }
            } else {
                unique_bytes = unique_bytes.saturating_add(encoded_len);
                if unique_bytes > MAX_BYTES {
                    return Err(fail("canonical history byte budget exceeded"));
                }
                unique.insert(operation.id.clone(), operation.clone());
                if unique.len() > MAX_OPERATIONS {
                    return Err(fail("operation budget exceeded"));
                }
            }
        }
    }
    history.operations = unique.into_values().collect();
    history.schema_version = if history
        .operations
        .iter()
        .any(|op| op.inline_style.is_some() || op.inserted_style.is_some())
    {
        5
    } else if history
        .operations
        .iter()
        .any(|op| op.paragraph_structure.is_some())
    {
        4
    } else if history
        .operations
        .iter()
        .any(|op| op.paragraph_style.is_some())
    {
        3
    } else if history.operations.iter().any(|op| op.visibility.is_some()) {
        2
    } else {
        1
    };
    let indexes = history
        .operations
        .iter()
        .enumerate()
        .map(|(i, op)| (op.id.clone(), i))
        .collect::<BTreeMap<_, _>>();
    let mut all_paragraphs = base.paragraphs.clone();
    let mut insertion_owners = BTreeMap::<String, usize>::new();
    let mut known_paragraph_ids = base
        .paragraphs
        .iter()
        .map(|paragraph| paragraph.id.clone())
        .collect::<BTreeSet<_>>();
    for (index, operation) in history.operations.iter().enumerate() {
        if let Some(paragraph) = operation
            .paragraph_structure
            .as_ref()
            .and_then(|patch| patch.inserted_paragraph.as_ref())
        {
            if !known_paragraph_ids.insert(paragraph.id.clone())
                || insertion_owners
                    .insert(paragraph.id.clone(), index)
                    .is_some()
            {
                return Err(fail(
                    "paragraph ID is already owned by the base or another insertion",
                ));
            }
            all_paragraphs.push(paragraph.clone());
        }
    }
    if all_paragraphs.len() > 100_000 {
        return Err(fail("paragraph budget exceeded"));
    }
    let paragraphs = all_paragraphs
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.clone(), i))
        .collect::<BTreeMap<_, _>>();
    let base_scalar_counts = all_paragraphs
        .iter()
        .map(|p| p.text.chars().count())
        .collect::<Vec<_>>();
    let mut missing = BTreeSet::new();
    let mut frontier = BTreeMap::<String, u64>::new();
    let mut actors = BTreeSet::new();
    for op in &history.operations {
        id(&op.id)?;
        actors.insert(&op.id.actor);
        let has_missing_context = op.context.iter().any(|(actor, sequence)| {
            !indexes.contains_key(&StoryOperationId {
                actor: actor.clone(),
                sequence: *sequence,
            })
        });
        let unknown_paragraph = !paragraphs.contains_key(&op.paragraph_id);
        if unknown_paragraph && !has_missing_context {
            return Err(fail("invalid operation paragraph"));
        }
        if op.context.len() > 256
            || op.removed.len() > MAX_ATOMS
            || op.lamport == 0
            || op.lamport > MAX_CLOCK
        {
            return Err(fail("invalid operation paragraph, clock or shape"));
        }
        if op.inserted.is_empty()
            && op.removed.is_empty()
            && op.visibility.is_none()
            && op.paragraph_style.is_none()
            && op.paragraph_structure.is_none()
            && op.inline_style.is_none()
        {
            return Err(fail("empty operation"));
        }
        if op.context.get(&op.id.actor).copied().unwrap_or(0) != op.id.sequence - 1 {
            return Err(fail("actor sequence is not causally contiguous"));
        }
        frontier
            .entry(op.id.actor.clone())
            .and_modify(|v| *v = (*v).max(op.id.sequence))
            .or_insert(op.id.sequence);
        let mut dependency_clock = 0u64;
        let mut complete_clock = true;
        for (actor_name, sequence) in &op.context {
            let dependency = StoryOperationId {
                actor: actor_name.clone(),
                sequence: *sequence,
            };
            id(&dependency)?;
            actors.insert(actor_name);
            if let Some(index) = indexes.get(&dependency) {
                let earlier = &history.operations[*index];
                dependency_clock = dependency_clock.max(earlier.lamport);
                if earlier.lamport >= op.lamport {
                    return Err(fail("causal clocks are not strictly increasing"));
                }
                for (name, value) in &earlier.context {
                    spend(&mut work, 1)?;
                    if op.context.get(name).copied().unwrap_or(0) < *value {
                        return Err(fail("operation context is not transitively closed"));
                    }
                }
            } else {
                missing.insert(dependency);
                complete_clock = false;
            }
        }
        if complete_clock && op.lamport != dependency_clock + 1 {
            return Err(fail(
                "logical clock is not the canonical successor of the observed context",
            ));
        }
        if let Some(visibility) = &op.visibility {
            let target = &visibility.target;
            if op.context.get(&target.actor).copied().unwrap_or(0) < target.sequence {
                return Err(fail("visibility target was not observed"));
            }
            if let Some(index) = indexes.get(target) {
                let original = &history.operations[*index];
                if original.visibility.is_some()
                    || original.paragraph_id != op.paragraph_id
                    || original.lamport >= op.lamport
                {
                    return Err(fail(
                        "visibility target must be an earlier original edit in the same paragraph",
                    ));
                }
            } else {
                missing.insert(target.clone());
            }
        }
        if let Some(owner) = insertion_owners.get(&op.paragraph_id) {
            let insertion = &history.operations[*owner];
            if insertion.id != op.id
                && op.context.get(&insertion.id.actor).copied().unwrap_or(0) < insertion.id.sequence
            {
                return Err(fail("operation did not observe its inserted paragraph"));
            }
        }
        if let Some(structure) = &op.paragraph_structure {
            if structure.inserted_paragraph.is_none() && unknown_paragraph && !has_missing_context {
                return Err(fail("unknown paragraph-structure target"));
            }
            if let Some(after) = structure
                .position
                .as_ref()
                .and_then(|position| position.after.as_ref())
            {
                if !paragraphs.contains_key(after) && !has_missing_context {
                    return Err(fail("unknown paragraph predecessor"));
                }
                if let Some(owner) = insertion_owners.get(after) {
                    let insertion = &history.operations[*owner];
                    if insertion.id != op.id
                        && op.context.get(&insertion.id.actor).copied().unwrap_or(0)
                            < insertion.id.sequence
                    {
                        return Err(fail(
                            "paragraph position did not observe its inserted predecessor",
                        ));
                    }
                }
            }
        }
        let inline_target_count = op
            .inline_style
            .as_ref()
            .map_or(0, |inline| inline.targets.len());
        let mut references = Vec::with_capacity(op.removed.len() + inline_target_count + 1);
        if let Some(anchor) = &op.after {
            references.push((
                &anchor.operation,
                anchor.offset,
                anchor
                    .offset
                    .checked_add(1)
                    .ok_or_else(|| fail("atom offset overflow"))?,
            ));
        }
        for range in &op.removed {
            if range.start >= range.end {
                return Err(fail("empty/reversed atom deletion range"));
            }
            references.push((&range.operation, range.start, range.end));
        }
        if let Some(inline) = &op.inline_style {
            for range in &inline.targets {
                if range.start >= range.end {
                    return Err(fail("empty/reversed inline-style atom range"));
                }
                references.push((&range.operation, range.start, range.end));
            }
        }
        for (source, _, end) in references {
            spend(&mut work, 1)?;
            if let Some(source) = source {
                id(source)?;
                if op.context.get(&source.actor).copied().unwrap_or(0) < source.sequence {
                    return Err(fail("atom reference was not observed by its operation"));
                }
                if let Some(index) = indexes.get(source) {
                    let owner = &history.operations[*index];
                    if owner.visibility.is_some()
                        || owner.paragraph_style.is_some()
                        || owner.paragraph_structure.is_some()
                        || owner.inline_style.is_some()
                        || owner.paragraph_id != op.paragraph_id
                        || owner.lamport >= op.lamport
                    {
                        return Err(fail("cross-paragraph or noncausal atom reference"));
                    }
                } else {
                    missing.insert(source.clone());
                }
            } else if !unknown_paragraph
                && end as usize > base_scalar_counts[paragraphs[&op.paragraph_id]]
            {
                return Err(fail("base atom reference out of bounds"));
            }
        }
        if missing.len() > 4096 || actors.len() > 256 {
            return Err(fail("actor or missing dependency budget exceeded"));
        }
    }
    let history_hash = crate::linked_stories::value_hash(&history)?;
    let empty_style_conflicts = Vec::<StoryStyleConflict>::new();
    let empty_structure_conflicts = Vec::<StoryStructureConflict>::new();
    let empty_inline_conflicts = Vec::<StoryInlineStyleConflict>::new();
    let mut result=StoryHistoryResult{history:history.clone(),history_sha256:history_hash,frontier,
        missing_dependencies:missing.into_iter().collect(),merged:None,
        paragraph_ids:all_paragraphs.iter().map(|paragraph| paragraph.id.clone()).collect(),
        atom_count:0,tombstone_count:0,
        inactive_operations:Vec::new(),suppressed_atom_count:0,
        style_conflicts_sha256:crate::linked_stories::value_hash(&empty_style_conflicts)?,
        style_conflicts:empty_style_conflicts,
        structure_conflicts_sha256:crate::linked_stories::value_hash(&empty_structure_conflicts)?,
        structure_conflicts:empty_structure_conflicts,
        inline_style_runs:Vec::new(),
        inline_conflicts_sha256:crate::linked_stories::value_hash(&empty_inline_conflicts)?,
        inline_conflicts:empty_inline_conflicts,
        limits:vec!["logical text convergence is not semantic agreement or PDF visual correctness; review the projection".into(),
            "actor IDs are not authenticated; the host must authorize transport, authors and imported histories".into(),
            "paragraph, membership/position and atom-targeted inline style writes use causal field registers; inline projection is logical and requires native materialization before PDF publication".into(),
            "tombstones/history retain deleted wording; not sanitizing redaction; no implicit history garbage collection".into()]};
    if !result.missing_dependencies.is_empty() {
        // An out-of-order operation is retained, but cannot advertise an actor
        // prefix we do not actually have to an incremental synchronization peer.
        result.frontier.clear();
        for op in &history.operations {
            let prefix = result.frontier.entry(op.id.actor.clone()).or_insert(0);
            if op.id.sequence == *prefix + 1 {
                *prefix += 1;
            }
        }
        result.frontier.retain(|_, sequence| *sequence > 0);
        crate::cancel::check_current_cancel("pending story history publication")?;
        return Ok((result, None));
    }

    // All controls of a target share its actor, whose sequence is contiguous
    // and causally ordered. Canonical ID order therefore chooses the latest
    // control without an arrival-order or cross-author last-writer winner.
    let mut active = vec![true; history.operations.len()];
    for operation in &history.operations {
        spend(&mut work, 1)?;
        if let Some(visibility) = &operation.visibility {
            active[indexes[&visibility.target]] = visibility.active;
        }
    }
    result.inactive_operations = history
        .operations
        .iter()
        .zip(&active)
        .filter(|(_, enabled)| !**enabled)
        .map(|(op, _)| op.id.clone())
        .collect();

    let mut order = (0..history.operations.len()).collect::<Vec<_>>();
    order.sort_by(|a, b| {
        let a = &history.operations[*a];
        let b = &history.operations[*b];
        (a.lamport, &a.id).cmp(&(b.lamport, &b.id))
    });
    let mut ranks = vec![0usize; order.len()];
    for (rank, index) in order.into_iter().enumerate() {
        ranks[index] = rank + 1;
    }
    let mut origins = all_paragraphs
        .iter()
        .enumerate()
        .map(|(paragraph, p)| Origin {
            paragraph,
            operation: None,
            text: p.text.as_str(),
            rank: 0,
            start: 0,
            count: 0,
            enabled: true,
        })
        .collect::<Vec<_>>();
    for (index, op) in history.operations.iter().enumerate() {
        origins.push(Origin {
            paragraph: paragraphs[op.paragraph_id.as_str()],
            operation: Some(&op.id),
            text: &op.inserted,
            rank: ranks[index],
            start: 0,
            count: 0,
            enabled: active[index],
        });
    }
    let base_count = all_paragraphs.len();
    let mut nodes = (0..base_count)
        .map(|_| Node {
            parent: NONE,
            first: NONE,
            next: NONE,
            scalar: None,
            removed: false,
            origin: NONE,
            offset: 0,
        })
        .collect::<Vec<_>>();
    for (origin_index, origin) in origins.iter_mut().enumerate() {
        origin.start = nodes.len();
        for (offset, scalar) in origin.text.chars().enumerate() {
            if nodes.len() - base_count >= MAX_ATOMS {
                return Err(fail("history atom budget exceeded"));
            }
            spend(&mut work, 1)?;
            nodes.push(Node {
                parent: if offset == 0 {
                    origin.paragraph
                } else {
                    nodes.len() - 1
                },
                first: NONE,
                next: NONE,
                scalar: Some(scalar),
                removed: false,
                origin: origin_index,
                offset: offset as u32,
            });
            origin.count += 1;
        }
    }
    let source_index = |paragraph: usize, operation: &Option<StoryOperationId>| -> Result<usize> {
        let index = if let Some(id) = operation {
            base_count + *indexes.get(id).ok_or_else(|| fail("missing atom origin"))?
        } else {
            paragraph
        };
        if origins[index].paragraph != paragraph {
            return Err(fail("cross-paragraph atom origin"));
        }
        Ok(index)
    };
    for (index, op) in history.operations.iter().enumerate() {
        let own = &origins[base_count + index];
        if let Some(after) = &op.after {
            let source = &origins[source_index(own.paragraph, &after.operation)?];
            if after.offset as usize >= source.count {
                return Err(fail("anchor offset out of bounds"));
            }
            if own.count > 0 {
                nodes[own.start].parent = source.start + after.offset as usize;
            }
        }
        for range in &op.removed {
            let source = &origins[source_index(own.paragraph, &range.operation)?];
            if range.end as usize > source.count {
                return Err(fail("deleted atom range out of bounds"));
            }
            spend(&mut work, (range.end - range.start) as usize)?;
            // Validate and budget inactive ranges too; undo must never hide
            // malformed references, but need not walk a no-effect deletion.
            if active[index] {
                for node in &mut nodes
                    [source.start + range.start as usize..source.start + range.end as usize]
                {
                    node.removed = true;
                }
            }
        }
    }
    // Descending insertion timestamp among siblings; chains remain anchored
    // through tombstones. Sorting indexes avoids allocating one child Vec/node.
    let mut edges = (base_count..nodes.len()).collect::<Vec<_>>();
    edges.sort_unstable_by(|a, b| {
        nodes[*a]
            .parent
            .cmp(&nodes[*b].parent)
            .then_with(|| {
                origins[nodes[*b].origin]
                    .rank
                    .cmp(&origins[nodes[*a].origin].rank)
            })
            .then_with(|| nodes[*b].offset.cmp(&nodes[*a].offset))
    });
    let mut last_parent = NONE;
    let mut previous = NONE;
    for index in edges {
        let parent = nodes[index].parent;
        if parent != last_parent {
            nodes[parent].first = index;
            last_parent = parent;
        } else {
            nodes[previous].next = index;
        }
        previous = index;
    }
    let mut views = BTreeMap::new();
    let mut merged = base.clone();
    merged.paragraphs = all_paragraphs.clone();
    let mut output_bytes = 0usize;
    for (paragraph, p) in merged.paragraphs.iter_mut().enumerate() {
        let mut view = View {
            text: String::new(),
            visible: Vec::new(),
        };
        let mut stack = Vec::new();
        if nodes[paragraph].first != NONE {
            stack.push(nodes[paragraph].first);
        }
        while let Some(index) = stack.pop() {
            spend(&mut work, 1)?;
            let node = &nodes[index];
            if node.next != NONE {
                stack.push(node.next);
            }
            if node.first != NONE {
                stack.push(node.first);
            }
            if !node.removed && origins[node.origin].enabled {
                view.text
                    .push(node.scalar.ok_or_else(|| fail("invalid text node"))?);
                view.visible.push((node.origin, node.offset));
            }
        }
        output_bytes = output_bytes.saturating_add(view.text.len());
        if view.text.len() > 4_000_000 || output_bytes > MAX_BYTES {
            return Err(fail("projected text budget exceeded"));
        }
        p.text = view.text.clone();
        views.insert(p.id.clone(), view);
    }
    // Paragraph formatting is a causal multi-value register per field. A write
    // supersedes only values it observed. Concurrent equal values converge;
    // concurrent different values remain explicit and suppress publication.
    let mut style_writes = BTreeMap::<(usize, &'static str), Vec<usize>>::new();
    for (index, operation) in history.operations.iter().enumerate() {
        if !active[index] {
            continue;
        }
        if let Some(style) = &operation.paragraph_style {
            let paragraph = paragraphs[operation.paragraph_id.as_str()];
            for field in STYLE_FIELDS {
                if patch_has(style, field) {
                    style_writes
                        .entry((paragraph, field))
                        .or_default()
                        .push(index);
                }
            }
        }
    }
    let mut style_conflicts = Vec::new();
    for ((paragraph, field), candidates) in style_writes {
        crate::cancel::check_current_cancel("story paragraph style projection")?;
        let maximal = candidates
            .iter()
            .copied()
            .filter(|candidate| {
                !candidates.iter().copied().any(|other| {
                    other != *candidate
                        && history.operations[other]
                            .context
                            .get(&history.operations[*candidate].id.actor)
                            .copied()
                            .unwrap_or(0)
                            >= history.operations[*candidate].id.sequence
                })
            })
            .collect::<Vec<_>>();
        let values = maximal
            .iter()
            .map(|index| {
                patch_value(
                    history.operations[*index].paragraph_style.as_ref().unwrap(),
                    field,
                )?
                .ok_or_else(|| fail("missing paragraph-style candidate value"))
            })
            .collect::<Result<Vec<_>>>()?;
        if values
            .first()
            .is_some_and(|first| values.iter().all(|value| value == first))
        {
            let selected = maximal
                .iter()
                .copied()
                .max_by(|a, b| {
                    let a = &history.operations[*a];
                    let b = &history.operations[*b];
                    (a.lamport, &a.id).cmp(&(b.lamport, &b.id))
                })
                .ok_or_else(|| fail("empty paragraph-style register"))?;
            apply_style_field(
                &mut merged.paragraphs[paragraph],
                history.operations[selected]
                    .paragraph_style
                    .as_ref()
                    .unwrap(),
                field,
            )?;
        } else {
            let mut candidates = maximal
                .into_iter()
                .zip(values)
                .map(|(index, value)| StoryStyleConflictCandidate {
                    operation: history.operations[index].id.clone(),
                    value,
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|a, b| a.operation.cmp(&b.operation));
            style_conflicts.push(StoryStyleConflict {
                paragraph_id: merged.paragraphs[paragraph].id.clone(),
                field: field.into(),
                candidates,
            });
        }
    }
    style_conflicts.sort_by(|a, b| (&a.paragraph_id, &a.field).cmp(&(&b.paragraph_id, &b.field)));
    result.style_conflicts_sha256 = crate::linked_stories::value_hash(&style_conflicts)?;
    result.style_conflicts = style_conflicts;

    // Inline formatting is a causal multi-value register per stable scalar atom
    // and supported field. Operations target compact atom ranges, so a text
    // insertion/deletion never retargets a mark through transient UTF-8 offsets.
    let mut inline_writes = BTreeMap::<(usize, StoryInlineStyleField), Vec<usize>>::new();
    for (index, operation) in history.operations.iter().enumerate() {
        if !active[index] {
            continue;
        }
        if let Some(style) = &operation.inserted_style {
            let origin = &origins[base_count + index];
            for field in INLINE_STYLE_FIELDS {
                if resolved_inline_value(style, field)?.is_some() {
                    for node in origin.start..origin.start + origin.count {
                        if nodes[node].removed || !origins[nodes[node].origin].enabled {
                            continue;
                        }
                        spend(&mut work, 1)?;
                        inline_writes.entry((node, field)).or_default().push(index);
                    }
                }
            }
        }
        if let Some(inline) = &operation.inline_style {
            let paragraph = paragraphs[operation.paragraph_id.as_str()];
            for range in &inline.targets {
                let origin = &origins[source_index(paragraph, &range.operation)?];
                if range.end as usize > origin.count {
                    return Err(fail("inline-style atom range out of bounds"));
                }
                for node in origin.start + range.start as usize..origin.start + range.end as usize {
                    if nodes[node].removed || !origins[nodes[node].origin].enabled {
                        continue;
                    }
                    spend(&mut work, 1)?;
                    for field in INLINE_STYLE_FIELDS {
                        if inline_patch_has(&inline.patch, field) {
                            inline_writes.entry((node, field)).or_default().push(index);
                        }
                    }
                }
            }
        }
    }
    let mut node_inline_styles = vec![StoryResolvedInlineStyle::default(); nodes.len()];
    // Materialized base spans are the immutable register baseline. They are
    // copied onto stable base atoms before causal writes; a later explicit
    // clear restores paragraph inheritance by removing that field.
    for (paragraph_index, paragraph) in all_paragraphs.iter().enumerate() {
        let origin = &origins[source_index(paragraph_index, &None)?];
        for span in &paragraph.inline_styles {
            let start = paragraph.text[..span.logical_range[0]].chars().count();
            let end = paragraph.text[..span.logical_range[1]].chars().count();
            for node in origin.start + start..origin.start + end {
                spend(&mut work, 1)?;
                let style = &mut node_inline_styles[node];
                if let Some(value) = &span.preferred_font {
                    style.preferred_font = Some(value.clone());
                }
                if let Some(value) = span.font_size {
                    style.font_size = Some(value);
                }
                if let Some(value) = span.rgb {
                    style.rgb = Some(value);
                }
                if let Some(value) = &span.shaping {
                    style.shaping = Some(value.clone());
                }
            }
        }
    }
    let mut inline_conflicts = Vec::new();
    for ((node, field), candidates) in inline_writes {
        crate::cancel::check_current_cancel("story inline-style projection")?;
        let maximal = candidates
            .iter()
            .copied()
            .filter(|candidate| {
                !candidates.iter().copied().any(|other| {
                    other != *candidate
                        && history.operations[other]
                            .context
                            .get(&history.operations[*candidate].id.actor)
                            .copied()
                            .unwrap_or(0)
                            >= history.operations[*candidate].id.sequence
                })
            })
            .collect::<Vec<_>>();
        let values = maximal
            .iter()
            .map(|index| {
                let operation = &history.operations[*index];
                if let Some(inline) = &operation.inline_style {
                    inline_patch_value(&inline.patch, field)
                } else if let Some(style) = &operation.inserted_style {
                    resolved_inline_value(style, field)
                } else {
                    Err(fail("missing inline-style candidate payload"))
                }?
                .ok_or_else(|| fail("missing inline-style candidate value"))
            })
            .collect::<Result<Vec<_>>>()?;
        if values
            .first()
            .is_some_and(|first| values.iter().all(|value| value == first))
        {
            let selected = maximal
                .iter()
                .copied()
                .max_by(|a, b| {
                    let a = &history.operations[*a];
                    let b = &history.operations[*b];
                    (a.lamport, &a.id).cmp(&(b.lamport, &b.id))
                })
                .ok_or_else(|| fail("empty inline-style register"))?;
            let value = if let Some(inline) = &history.operations[selected].inline_style {
                inline_patch_value(&inline.patch, field)?
            } else if let Some(style) = &history.operations[selected].inserted_style {
                resolved_inline_value(style, field)?
            } else {
                None
            }
            .ok_or_else(|| fail("missing selected inline-style value"))?;
            apply_inline_field(&mut node_inline_styles[node], &value, field)?;
        } else {
            if inline_conflicts.len() >= 4096 {
                return Err(fail("inline-style conflict report budget exceeded"));
            }
            let origin = &origins[nodes[node].origin];
            let mut candidates = maximal
                .into_iter()
                .zip(values)
                .map(|(index, value)| StoryInlineStyleConflictCandidate {
                    operation: history.operations[index].id.clone(),
                    value,
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|a, b| a.operation.cmp(&b.operation));
            inline_conflicts.push(StoryInlineStyleConflict {
                paragraph_id: all_paragraphs[origin.paragraph].id.clone(),
                target: StoryAtomId {
                    operation: origin.operation.cloned(),
                    offset: nodes[node].offset,
                },
                field,
                candidates,
            });
        }
    }
    inline_conflicts.sort_by(|a, b| {
        (&a.paragraph_id, &a.target, a.field).cmp(&(&b.paragraph_id, &b.target, b.field))
    });
    result.inline_conflicts_sha256 = crate::linked_stories::value_hash(&inline_conflicts)?;
    result.inline_conflicts = inline_conflicts;
    for paragraph in &all_paragraphs {
        let view = &views[paragraph.id.as_str()];
        let characters = view.text.char_indices().collect::<Vec<_>>();
        for (position, ((start, _), (origin, offset))) in
            characters.iter().zip(&view.visible).enumerate()
        {
            let end = characters
                .get(position + 1)
                .map_or(view.text.len(), |(byte, _)| *byte);
            let node = origins[*origin].start + *offset as usize;
            let style = &node_inline_styles[node];
            if resolved_style_is_empty(style) {
                continue;
            }
            if let Some(previous) = result.inline_style_runs.last_mut().filter(|run| {
                run.paragraph_id == paragraph.id
                    && run.logical_range[1] == *start
                    && &run.style == style
            }) {
                previous.logical_range[1] = end;
            } else {
                result.inline_style_runs.push(StoryInlineStyleRun {
                    paragraph_id: paragraph.id.clone(),
                    logical_range: [*start, end],
                    style: style.clone(),
                });
            }
        }
    }

    // Paragraph presence and predecessor are independent causal registers.
    // The immutable base contributes the initial ordered chain. An insertion
    // contributes the first values for its new ID; inactive insertions remain
    // tombstone anchors so later descendants keep deterministic placement.
    let mut present = (0..all_paragraphs.len())
        .map(|index| index < base.paragraphs.len())
        .collect::<Vec<_>>();
    let mut after = (0..all_paragraphs.len())
        .map(|index| {
            if index == 0 || index >= base.paragraphs.len() {
                None
            } else {
                Some(all_paragraphs[index - 1].id.clone())
            }
        })
        .collect::<Vec<_>>();
    let mut position_rank = vec![0usize; all_paragraphs.len()];
    let mut structure_writes = BTreeMap::<(usize, &'static str), Vec<usize>>::new();
    for (index, operation) in history.operations.iter().enumerate() {
        if !active[index] {
            continue;
        }
        if let Some(structure) = &operation.paragraph_structure {
            let paragraph = paragraphs[operation.paragraph_id.as_str()];
            for field in STRUCTURE_FIELDS {
                if structure_patch_has(structure, field) {
                    structure_writes
                        .entry((paragraph, field))
                        .or_default()
                        .push(index);
                }
            }
        }
    }
    let mut structure_conflicts = Vec::new();
    let mut presence_maximal = vec![Vec::<usize>::new(); all_paragraphs.len()];
    for ((paragraph, field), candidates) in structure_writes {
        crate::cancel::check_current_cancel("story paragraph structure projection")?;
        let maximal = candidates
            .iter()
            .copied()
            .filter(|candidate| {
                !candidates.iter().copied().any(|other| {
                    other != *candidate
                        && history.operations[other]
                            .context
                            .get(&history.operations[*candidate].id.actor)
                            .copied()
                            .unwrap_or(0)
                            >= history.operations[*candidate].id.sequence
                })
            })
            .collect::<Vec<_>>();
        let values = maximal
            .iter()
            .map(|index| {
                structure_patch_value(
                    history.operations[*index]
                        .paragraph_structure
                        .as_ref()
                        .unwrap(),
                    field,
                )?
                .ok_or_else(|| fail("missing paragraph-structure candidate value"))
            })
            .collect::<Result<Vec<_>>>()?;
        if values
            .first()
            .is_some_and(|first| values.iter().all(|value| value == first))
        {
            let selected = maximal
                .iter()
                .copied()
                .max_by(|a, b| {
                    let a = &history.operations[*a];
                    let b = &history.operations[*b];
                    (a.lamport, &a.id).cmp(&(b.lamport, &b.id))
                })
                .ok_or_else(|| fail("empty paragraph-structure register"))?;
            let patch = history.operations[selected]
                .paragraph_structure
                .as_ref()
                .unwrap();
            match field {
                "present" => {
                    present[paragraph] = patch.present.unwrap();
                    presence_maximal[paragraph] = maximal;
                }
                "position" => {
                    after[paragraph] = patch.position.as_ref().unwrap().after.clone();
                    position_rank[paragraph] = ranks[selected];
                }
                _ => return Err(fail("unknown paragraph-structure field")),
            }
        } else {
            let mut candidates = maximal
                .into_iter()
                .zip(values)
                .map(|(index, value)| StoryStructureConflictCandidate {
                    operation: Some(history.operations[index].id.clone()),
                    value,
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|a, b| a.operation.cmp(&b.operation));
            structure_conflicts.push(StoryStructureConflict {
                paragraph_id: all_paragraphs[paragraph].id.clone(),
                field: field.into(),
                candidates,
            });
        }
    }

    // Deleting a paragraph only wins over text/style work the delete observed.
    // A concurrent author must never have their content silently discarded.
    for (paragraph, delete_candidates) in presence_maximal.iter().enumerate() {
        if present[paragraph] || delete_candidates.is_empty() {
            continue;
        }
        let content = history
            .operations
            .iter()
            .enumerate()
            .filter(|(index, operation)| {
                active[*index]
                    && operation.paragraph_id == all_paragraphs[paragraph].id
                    && operation.visibility.is_none()
                    && operation.paragraph_structure.is_none()
                    && (!operation.inserted.is_empty()
                        || !operation.removed.is_empty()
                        || operation.paragraph_style.is_some()
                        || operation.inline_style.is_some())
            })
            .collect::<Vec<_>>();
        let mut conflicting = BTreeSet::new();
        for delete in delete_candidates {
            let deletion = &history.operations[*delete];
            for (content_index, edit) in &content {
                let deletion_observed_edit =
                    deletion.context.get(&edit.id.actor).copied().unwrap_or(0) >= edit.id.sequence;
                let edit_observed_deletion =
                    edit.context.get(&deletion.id.actor).copied().unwrap_or(0)
                        >= deletion.id.sequence;
                if edit_observed_deletion {
                    return Err(fail("content operation observes a deleted paragraph"));
                }
                if !deletion_observed_edit {
                    conflicting.insert(*content_index);
                }
            }
        }
        if !conflicting.is_empty() {
            let mut candidates = delete_candidates
                .iter()
                .map(|index| StoryStructureConflictCandidate {
                    operation: Some(history.operations[*index].id.clone()),
                    value: serde_json::json!({"present":false}),
                })
                .chain(
                    conflicting
                        .into_iter()
                        .map(|index| StoryStructureConflictCandidate {
                            operation: Some(history.operations[index].id.clone()),
                            value: serde_json::json!({"preserve_concurrent_content":true}),
                        }),
                )
                .collect::<Vec<_>>();
            candidates.sort_by(|a, b| a.operation.cmp(&b.operation));
            structure_conflicts.push(StoryStructureConflict {
                paragraph_id: all_paragraphs[paragraph].id.clone(),
                field: "present".into(),
                candidates,
            });
        }
    }

    // Detect predecessor cycles without recursion. A cycle is an explicit
    // position conflict; no partial paragraph order is published.
    let mut parent = vec![None; all_paragraphs.len()];
    for (index, predecessor) in after.iter().enumerate() {
        if let Some(predecessor) = predecessor {
            parent[index] = Some(paragraphs[predecessor]);
        }
    }
    let explicit_parent = parent
        .iter()
        .enumerate()
        .map(|(index, parent)| (position_rank[index] > 0).then_some(*parent).flatten())
        .collect::<Vec<_>>();
    let mut state = vec![0u8; all_paragraphs.len()];
    for start in 0..all_paragraphs.len() {
        if state[start] != 0 {
            continue;
        }
        let mut path = Vec::<usize>::new();
        let mut positions = BTreeMap::<usize, usize>::new();
        let mut cursor = Some(start);
        while let Some(index) = cursor {
            if state[index] == 2 {
                break;
            }
            if let Some(cycle_start) = positions.get(&index).copied() {
                for member in path[cycle_start..].iter().copied() {
                    if !structure_conflicts.iter().any(|conflict| {
                        conflict.paragraph_id == all_paragraphs[member].id
                            && conflict.field == "position"
                    }) {
                        structure_conflicts.push(StoryStructureConflict {
                            paragraph_id: all_paragraphs[member].id.clone(),
                            field: "position".into(),
                            candidates: vec![StoryStructureConflictCandidate {
                                operation: None,
                                value: serde_json::json!({"after":after[member].clone()}),
                            }],
                        });
                    }
                }
                break;
            }
            positions.insert(index, path.len());
            path.push(index);
            state[index] = 1;
            cursor = explicit_parent[index];
        }
        for index in path {
            state[index] = 2;
        }
    }
    structure_conflicts
        .sort_by(|a, b| (&a.paragraph_id, &a.field).cmp(&(&b.paragraph_id, &b.field)));
    result.structure_conflicts_sha256 = crate::linked_stories::value_hash(&structure_conflicts)?;
    result.structure_conflicts = structure_conflicts;

    if result.structure_conflicts.is_empty() {
        // Start from the immutable base order plus canonically discovered
        // insertion IDs. Apply selected position registers oldest-to-newest as
        // single-item remove/insert operations. This avoids treating the
        // predecessor relation as ownership: moving one paragraph does not drag
        // its former successors with it. Equal-gap concurrent writes still end
        // in descending causal rank because the later deterministic write is
        // inserted nearest the shared predecessor.
        let paragraph_count = all_paragraphs.len();
        let mut previous = (0..paragraph_count)
            .map(|index| index.checked_sub(1))
            .collect::<Vec<_>>();
        let mut next = (0..paragraph_count)
            .map(|index| (index + 1 < paragraph_count).then_some(index + 1))
            .collect::<Vec<_>>();
        let mut head = (paragraph_count > 0).then_some(0usize);
        let mut positioned = (0..all_paragraphs.len())
            .filter(|index| position_rank[*index] > 0)
            .collect::<Vec<_>>();
        positioned.sort_by(|a, b| {
            position_rank[*a]
                .cmp(&position_rank[*b])
                .then_with(|| all_paragraphs[*a].id.cmp(&all_paragraphs[*b].id))
        });
        for index in positioned {
            spend(&mut work, 1)?;
            let old_previous = previous[index];
            let old_next = next[index];
            if let Some(old_previous) = old_previous {
                next[old_previous] = old_next;
            } else {
                head = old_next;
            }
            if let Some(old_next) = old_next {
                previous[old_next] = old_previous;
            }
            match parent[index] {
                Some(predecessor) => {
                    let successor = next[predecessor];
                    previous[index] = Some(predecessor);
                    next[index] = successor;
                    next[predecessor] = Some(index);
                    if let Some(successor) = successor {
                        previous[successor] = Some(index);
                    }
                }
                None => {
                    previous[index] = None;
                    next[index] = head;
                    if let Some(old_head) = head {
                        previous[old_head] = Some(index);
                    }
                    head = Some(index);
                }
            }
        }
        let mut ordered = Vec::with_capacity(present.iter().filter(|value| **value).count());
        let mut visited = 0usize;
        let mut cursor = head;
        while let Some(index) = cursor {
            spend(&mut work, 1)?;
            visited += 1;
            if visited > paragraph_count {
                return Err(fail("paragraph sequence contains a cycle"));
            }
            if present[index] {
                ordered.push(merged.paragraphs[index].clone());
            }
            cursor = next[index];
        }
        if visited != paragraph_count {
            return Err(fail("paragraph sequence lost a stable identity"));
        }
        if ordered.len() != present.iter().filter(|value| **value).count() {
            return Err(fail("paragraph predecessor graph did not fully project"));
        }
        merged.paragraphs = ordered;
    }
    result.atom_count = nodes.len() - base_count;
    result.tombstone_count = nodes.iter().filter(|n| n.removed).count();
    result.suppressed_atom_count = origins
        .iter()
        .filter(|origin| !origin.enabled)
        .map(|origin| origin.count)
        .sum();
    if result.style_conflicts.is_empty()
        && result.structure_conflicts.is_empty()
        && result.inline_conflicts.is_empty()
    {
        for paragraph in &mut merged.paragraphs {
            paragraph.inline_styles = result
                .inline_style_runs
                .iter()
                .filter(|run| run.paragraph_id == paragraph.id)
                .map(|run| crate::linked_stories::StoryInlineStyleSpan {
                    logical_range: run.logical_range,
                    preferred_font: run.style.preferred_font.clone(),
                    font_size: run.style.font_size,
                    rgb: run.style.rgb,
                    shaping: run.style.shaping.clone(),
                })
                .collect();
        }
        crate::linked_stories::validate_paragraphs(&merged)?;
        result.merged = Some(merged);
    }
    crate::cancel::check_current_cancel("story history projection publication")?;
    Ok((
        result,
        Some(Projection {
            views,
            origins: origins
                .iter()
                .map(|origin| origin.operation.cloned())
                .collect(),
        }),
    ))
}

pub fn edit_history(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryEdit,
) -> Result<StoryHistoryResult> {
    edit_bound(base, history, edit, None)
}

pub fn edit_paragraph_style(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryStyleEdit,
) -> Result<StoryHistoryResult> {
    edit_style_bound(base, history, edit, None)
}

pub fn edit_paragraph_structure(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryStructureEdit,
) -> Result<StoryHistoryResult> {
    edit_structure_bound(base, history, edit, None)
}

pub fn edit_inline_style(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryInlineStyleEdit,
) -> Result<StoryHistoryResult> {
    edit_inline_style_bound(base, history, edit, None)
}

fn edit_inline_style_bound(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryInlineStyleEdit,
    seed: Option<&StoryHistorySeed>,
) -> Result<StoryHistoryResult> {
    actor(&edit.actor)?;
    validate_inline_patch(&edit.replacement, false)?;
    let (current, projection) = match seed {
        Some(seed) => reconcile_bound(base, std::slice::from_ref(history), seed_header(seed))?,
        None => reconcile(base, std::slice::from_ref(history))?,
    };
    if current.history_sha256 != edit.expected_history_sha256 {
        return Err(fail("stale inline-style history preimage"));
    }
    if current.inline_conflicts_sha256 != edit.expected_inline_conflicts_sha256 {
        return Err(fail("inline-style conflict preimage changed"));
    }
    if !current.inline_conflicts.is_empty() {
        return Err(fail(
            "resolve atom-level inline-style conflicts before authoring another mark",
        ));
    }
    if current.merged.as_ref().is_none_or(|request| {
        !request
            .paragraphs
            .iter()
            .any(|paragraph| paragraph.id == edit.paragraph_id)
    }) {
        return Err(fail("cannot style a deleted or conflicted paragraph"));
    }
    let projection = projection
        .ok_or_else(|| fail("receive missing dependencies before authoring an inline style"))?;
    let view = projection
        .views
        .get(&edit.paragraph_id)
        .ok_or_else(|| fail("unknown inline-style paragraph"))?;
    if edit.range[0] >= edit.range[1]
        || view.text.get(edit.range[0]..edit.range[1]) != Some(edit.expected_text.as_str())
    {
        return Err(fail("inline-style text preimage mismatch or empty range"));
    }
    validate_grapheme_selection(&view.text, edit.range)?;
    let targets = atom_ranges_for_selection(view, &projection, edit.range)?;
    if targets.is_empty() {
        return Err(fail("inline-style selection has no visible atoms"));
    }
    let sequence = current
        .frontier
        .get(&edit.actor)
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|value| *value <= MAX_CLOCK)
        .ok_or_else(|| fail("actor sequence exhausted"))?;
    let lamport = current
        .history
        .operations
        .iter()
        .map(|operation| operation.lamport)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|value| *value <= MAX_CLOCK)
        .ok_or_else(|| fail("logical clock exhausted"))?;
    let mut next = current.history;
    next.schema_version = next.schema_version.max(5);
    next.operations.push(StoryTextOperation {
        id: StoryOperationId {
            actor: edit.actor.clone(),
            sequence,
        },
        lamport,
        context: current.frontier,
        paragraph_id: edit.paragraph_id.clone(),
        after: None,
        removed: Vec::new(),
        inserted: String::new(),
        visibility: None,
        paragraph_style: None,
        paragraph_structure: None,
        inserted_style: None,
        inline_style: Some(StoryInlineStyleOperation {
            targets,
            patch: edit.replacement.clone(),
        }),
    });
    match seed {
        Some(seed) => Ok(reconcile_bound(base, &[next], seed_header(seed))?.0),
        None => merge_histories(base, &[next]),
    }
}

pub fn resolve_inline_style_conflicts(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    resolution: &StoryHistoryInlineStyleResolution,
) -> Result<StoryHistoryResult> {
    resolve_inline_style_conflicts_bound(base, history, resolution, None)
}

fn resolve_inline_style_conflicts_bound(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    resolution: &StoryHistoryInlineStyleResolution,
    seed: Option<&StoryHistorySeed>,
) -> Result<StoryHistoryResult> {
    actor(&resolution.actor)?;
    validate_inline_patch(&resolution.replacement, false)?;
    let fields = inline_patch_fields(&resolution.replacement);
    if fields.len() != 1 {
        return Err(fail(
            "inline-style conflict resolution must choose exactly one field",
        ));
    }
    validate_inline_targets(&resolution.targets)?;
    let current = match seed {
        Some(seed) => reconcile_bound(base, std::slice::from_ref(history), seed_header(seed))?.0,
        None => merge_histories(base, std::slice::from_ref(history))?,
    };
    if current.history_sha256 != resolution.expected_history_sha256 {
        return Err(fail("stale inline-style resolution history preimage"));
    }
    if current.inline_conflicts_sha256 != resolution.expected_inline_conflicts_sha256 {
        return Err(fail("inline-style conflict resolution preimage changed"));
    }
    let field = *fields.iter().next().unwrap();
    let expected_targets = current
        .inline_conflicts
        .iter()
        .filter(|conflict| {
            conflict.paragraph_id == resolution.paragraph_id && conflict.field == field
        })
        .map(|conflict| conflict.target.clone())
        .collect::<BTreeSet<_>>();
    if expected_targets.is_empty() || expanded_atom_ids(&resolution.targets)? != expected_targets {
        return Err(fail(
            "inline-style resolution must cover every current conflicting atom for its paragraph and field",
        ));
    }
    let sequence = current
        .frontier
        .get(&resolution.actor)
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|value| *value <= MAX_CLOCK)
        .ok_or_else(|| fail("actor sequence exhausted"))?;
    let lamport = current
        .history
        .operations
        .iter()
        .map(|operation| operation.lamport)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|value| *value <= MAX_CLOCK)
        .ok_or_else(|| fail("logical clock exhausted"))?;
    let mut next = current.history;
    next.schema_version = next.schema_version.max(5);
    next.operations.push(StoryTextOperation {
        id: StoryOperationId {
            actor: resolution.actor.clone(),
            sequence,
        },
        lamport,
        context: current.frontier,
        paragraph_id: resolution.paragraph_id.clone(),
        after: None,
        removed: Vec::new(),
        inserted: String::new(),
        visibility: None,
        paragraph_style: None,
        paragraph_structure: None,
        inserted_style: None,
        inline_style: Some(StoryInlineStyleOperation {
            targets: resolution.targets.clone(),
            patch: resolution.replacement.clone(),
        }),
    });
    match seed {
        Some(seed) => Ok(reconcile_bound(base, &[next], seed_header(seed))?.0),
        None => merge_histories(base, &[next]),
    }
}

fn expanded_atom_ids(ranges: &[StoryAtomRange]) -> Result<BTreeSet<StoryAtomId>> {
    validate_inline_targets(ranges)?;
    let mut result = BTreeSet::new();
    for range in ranges {
        for offset in range.start..range.end {
            if !result.insert(StoryAtomId {
                operation: range.operation.clone(),
                offset,
            }) {
                return Err(fail("duplicate inline-style atom target"));
            }
            if result.len() > MAX_ATOMS {
                return Err(fail("inline-style atom target budget exceeded"));
            }
        }
    }
    Ok(result)
}

fn validate_grapheme_selection(text: &str, range: [usize; 2]) -> Result<()> {
    if range[0] > range[1] || range[1] > text.len() {
        return Err(fail("selection is outside the current paragraph"));
    }
    let (start_boundary, end_boundary) = text
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .fold((false, false), |(start, end), offset| {
            (start || offset == range[0], end || offset == range[1])
        });
    if !start_boundary || !end_boundary {
        return Err(fail("selection splits a grapheme"));
    }
    Ok(())
}

fn atom_ranges_for_selection(
    view: &View,
    projection: &Projection,
    range: [usize; 2],
) -> Result<Vec<StoryAtomRange>> {
    let mut selected = Vec::<(usize, u32, u32)>::new();
    for ((byte, _), (origin, offset)) in view.text.char_indices().zip(&view.visible) {
        if byte < range[0] {
            continue;
        }
        if byte >= range[1] {
            break;
        }
        if let Some(previous) = selected
            .last_mut()
            .filter(|previous| previous.0 == *origin && previous.2 == *offset)
        {
            previous.2 += 1;
        } else {
            selected.push((*origin, *offset, *offset + 1));
        }
    }
    let targets = selected
        .into_iter()
        .map(|(origin, start, end)| {
            Ok(StoryAtomRange {
                operation: projection
                    .origins
                    .get(origin)
                    .ok_or_else(|| fail("inline-style atom origin is missing"))?
                    .clone(),
                start,
                end,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    validate_inline_targets(&targets)?;
    Ok(targets)
}

fn inherited_inline_style(
    current: &StoryHistoryResult,
    paragraph_id: &str,
    range: [usize; 2],
) -> Option<StoryResolvedInlineStyle> {
    let probe = if range[0] < range[1] || range[0] == 0 {
        range[0]
    } else {
        range[0] - 1
    };
    current
        .inline_style_runs
        .iter()
        .find(|run| {
            run.paragraph_id == paragraph_id
                && run.logical_range[0] <= probe
                && probe < run.logical_range[1]
        })
        .map(|run| run.style.clone())
}

fn edit_bound(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryEdit,
    seed: Option<&StoryHistorySeed>,
) -> Result<StoryHistoryResult> {
    actor(&edit.actor)?;
    if edit.replacement.len() > 4_000_000 || edit.expected_text.len() > 4_000_000 {
        return Err(fail("edit text budget exceeded"));
    }
    let (current, projection) = match seed {
        Some(seed) => reconcile_bound(base, std::slice::from_ref(history), seed_header(seed))?,
        None => reconcile(base, std::slice::from_ref(history))?,
    };
    if current.history_sha256 != edit.expected_history_sha256 {
        return Err(fail("stale history edit preimage"));
    }
    let projection =
        projection.ok_or_else(|| fail("receive missing dependencies before authoring an edit"))?;
    if current.merged.as_ref().is_none_or(|request| {
        !request
            .paragraphs
            .iter()
            .any(|paragraph| paragraph.id == edit.paragraph_id)
    }) {
        return Err(fail(
            "cannot edit a deleted or structurally conflicted paragraph",
        ));
    }
    let view = projection
        .views
        .get(&edit.paragraph_id)
        .ok_or_else(|| fail("unknown edit paragraph"))?;
    if view.text.get(edit.range[0]..edit.range[1]) != Some(edit.expected_text.as_str()) {
        return Err(fail("text edit preimage mismatch"));
    }
    validate_grapheme_selection(&view.text, edit.range)?;
    if edit.expected_text == edit.replacement {
        return Ok(current);
    }
    let mut after_location = None;
    let mut removed_ranges = Vec::<(usize, u32, u32)>::new();
    for ((byte, _), (origin, offset)) in view.text.char_indices().zip(&view.visible) {
        if byte < edit.range[0] {
            after_location = Some((*origin, *offset));
        } else if byte < edit.range[1] {
            if let Some(previous) = removed_ranges
                .last_mut()
                .filter(|last| last.0 == *origin && last.2 == *offset)
            {
                previous.2 += 1;
            } else {
                removed_ranges.push((*origin, *offset, *offset + 1));
            }
        } else {
            break;
        }
    }
    let after = after_location.map(|(origin, offset)| StoryAtomId {
        operation: projection.origins[origin].clone(),
        offset,
    });
    let removed = removed_ranges
        .into_iter()
        .map(|(origin, start, end)| StoryAtomRange {
            operation: projection.origins[origin].clone(),
            start,
            end,
        })
        .collect();
    let sequence = current
        .frontier
        .get(&edit.actor)
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|n| *n <= MAX_CLOCK)
        .ok_or_else(|| fail("actor sequence exhausted"))?;
    let lamport = current
        .history
        .operations
        .iter()
        .map(|op| op.lamport)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|n| *n <= MAX_CLOCK)
        .ok_or_else(|| fail("logical clock exhausted"))?;
    let inserted_style = (!edit.replacement.is_empty())
        .then(|| inherited_inline_style(&current, &edit.paragraph_id, edit.range))
        .flatten();
    let mut next = current.history;
    if inserted_style.is_some() {
        next.schema_version = next.schema_version.max(5);
    }
    next.operations.push(StoryTextOperation {
        id: StoryOperationId {
            actor: edit.actor.clone(),
            sequence,
        },
        lamport,
        context: current.frontier,
        paragraph_id: edit.paragraph_id.clone(),
        after,
        removed,
        inserted: edit.replacement.clone(),
        visibility: None,
        paragraph_style: None,
        paragraph_structure: None,
        inserted_style,
        inline_style: None,
    });
    match seed {
        Some(seed) => Ok(reconcile_bound(base, &[next], seed_header(seed))?.0),
        None => merge_histories(base, &[next]),
    }
}

fn edit_style_bound(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryStyleEdit,
    seed: Option<&StoryHistorySeed>,
) -> Result<StoryHistoryResult> {
    actor(&edit.actor)?;
    validate_style_patch(&edit.expected, true)?;
    validate_style_patch(&edit.replacement, false)?;
    let current = match seed {
        Some(seed) => reconcile_bound(base, std::slice::from_ref(history), seed_header(seed))?.0,
        None => merge_histories(base, std::slice::from_ref(history))?,
    };
    if current.history_sha256 != edit.expected_history_sha256 {
        return Err(fail("stale paragraph-style history preimage"));
    }
    if current.style_conflicts_sha256 != edit.expected_style_conflicts_sha256 {
        return Err(fail("paragraph-style conflict preimage changed"));
    }
    let paragraph_known = base
        .paragraphs
        .iter()
        .any(|paragraph| paragraph.id == edit.paragraph_id)
        || current.history.operations.iter().any(|operation| {
            operation
                .paragraph_structure
                .as_ref()
                .and_then(|patch| patch.inserted_paragraph.as_ref())
                .is_some_and(|paragraph| paragraph.id == edit.paragraph_id)
        });
    if !paragraph_known {
        return Err(fail("unknown paragraph-style target"));
    }
    let replacement_fields = patch_fields(&edit.replacement);
    let expected_fields = patch_fields(&edit.expected);
    let conflict_fields = current
        .style_conflicts
        .iter()
        .filter(|conflict| conflict.paragraph_id == edit.paragraph_id)
        .map(|conflict| conflict.field.as_str())
        .collect::<BTreeSet<_>>();
    if current.style_conflicts.is_empty() {
        if expected_fields != replacement_fields {
            return Err(fail(
                "paragraph-style expected/replacement field sets differ",
            ));
        }
        let projected = current
            .merged
            .as_ref()
            .and_then(|request| {
                request
                    .paragraphs
                    .iter()
                    .find(|candidate| candidate.id == edit.paragraph_id)
            })
            .ok_or_else(|| fail("missing paragraph-style projection"))?;
        for field in &replacement_fields {
            if patch_value(&edit.expected, field)? != Some(paragraph_style_value(projected, field)?)
            {
                return Err(fail("paragraph-style value preimage mismatch"));
            }
        }
        if edit.expected == edit.replacement {
            return Ok(current);
        }
    } else {
        if conflict_fields.is_empty() {
            return Err(fail(
                "resolve existing paragraph-style conflicts before editing another paragraph",
            ));
        }
        if !expected_fields.is_empty() || replacement_fields != conflict_fields {
            return Err(fail(
                "a conflict resolution must omit expected values and replace every conflicting field in its paragraph",
            ));
        }
    }
    // Bind the target to the immutable base even while a different paragraph's
    // conflicts suppress the full projection.
    let sequence = current
        .frontier
        .get(&edit.actor)
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|value| *value <= MAX_CLOCK)
        .ok_or_else(|| fail("actor sequence exhausted"))?;
    let lamport = current
        .history
        .operations
        .iter()
        .map(|operation| operation.lamport)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|value| *value <= MAX_CLOCK)
        .ok_or_else(|| fail("logical clock exhausted"))?;
    let mut next = current.history;
    next.schema_version = next.schema_version.max(3);
    next.operations.push(StoryTextOperation {
        id: StoryOperationId {
            actor: edit.actor.clone(),
            sequence,
        },
        lamport,
        context: current.frontier,
        paragraph_id: edit.paragraph_id.clone(),
        after: None,
        removed: Vec::new(),
        inserted: String::new(),
        visibility: None,
        paragraph_style: Some(edit.replacement.clone()),
        paragraph_structure: None,
        inserted_style: None,
        inline_style: None,
    });
    match seed {
        Some(seed) => Ok(reconcile_bound(base, &[next], seed_header(seed))?.0),
        None => merge_histories(base, &[next]),
    }
}

fn edit_structure_bound(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    edit: &StoryHistoryStructureEdit,
    seed: Option<&StoryHistorySeed>,
) -> Result<StoryHistoryResult> {
    actor(&edit.actor)?;
    validate_structure_patch(base, &edit.paragraph_id, &edit.expected, true)?;
    validate_structure_patch(base, &edit.paragraph_id, &edit.replacement, false)?;
    if edit.expected.inserted_paragraph.is_some() {
        return Err(fail(
            "paragraph-structure expected preimage cannot introduce a paragraph",
        ));
    }
    let current = match seed {
        Some(seed) => reconcile_bound(base, std::slice::from_ref(history), seed_header(seed))?.0,
        None => merge_histories(base, std::slice::from_ref(history))?,
    };
    if current.history_sha256 != edit.expected_history_sha256 {
        return Err(fail("stale paragraph-structure history preimage"));
    }
    if current.structure_conflicts_sha256 != edit.expected_structure_conflicts_sha256 {
        return Err(fail("paragraph-structure conflict preimage changed"));
    }
    let conflict_fields = current
        .structure_conflicts
        .iter()
        .filter(|conflict| conflict.paragraph_id == edit.paragraph_id)
        .map(|conflict| conflict.field.as_str())
        .collect::<BTreeSet<_>>();
    let expected_fields = structure_patch_fields(&edit.expected);
    let replacement_fields = structure_patch_fields(&edit.replacement);
    let inserted_ids = current
        .history
        .operations
        .iter()
        .filter_map(|operation| {
            operation
                .paragraph_structure
                .as_ref()
                .and_then(|patch| patch.inserted_paragraph.as_ref())
                .map(|paragraph| paragraph.id.as_str())
        })
        .collect::<BTreeSet<_>>();
    let exists_in_epoch = base
        .paragraphs
        .iter()
        .any(|paragraph| paragraph.id == edit.paragraph_id)
        || inserted_ids.contains(edit.paragraph_id.as_str());

    if current.structure_conflicts.is_empty() {
        let insertion = edit.replacement.inserted_paragraph.is_some();
        if insertion {
            if !edit.expected_absent
                || exists_in_epoch
                || !expected_fields.is_empty()
                || replacement_fields != BTreeSet::from(["present", "position"])
            {
                return Err(fail(
                    "paragraph insertion requires a new ID, expected_absent and initial presence/position",
                ));
            }
        } else {
            if edit.expected_absent || !exists_in_epoch || expected_fields != replacement_fields {
                return Err(fail(
                    "paragraph-structure expected/replacement field sets or target differ",
                ));
            }
            let projected = current
                .merged
                .as_ref()
                .ok_or_else(|| fail("missing paragraph-structure projection"))?;
            let position = projected
                .paragraphs
                .iter()
                .position(|paragraph| paragraph.id == edit.paragraph_id)
                .ok_or_else(|| fail("cannot mutate a deleted paragraph without selective undo"))?;
            for field in &expected_fields {
                let actual = match *field {
                    "present" => serde_json::json!(true),
                    "position" => serde_json::to_value(StoryParagraphPosition {
                        after: position
                            .checked_sub(1)
                            .map(|previous| projected.paragraphs[previous].id.clone()),
                    })
                    .map_err(|_| fail("paragraph position serialization"))?,
                    _ => return Err(fail("unknown paragraph-structure field")),
                };
                if structure_patch_value(&edit.expected, field)? != Some(actual) {
                    return Err(fail("paragraph-structure value preimage mismatch"));
                }
            }
            if edit.expected == edit.replacement {
                return Ok(current);
            }
        }
    } else {
        if conflict_fields.is_empty() {
            return Err(fail(
                "resolve existing paragraph-structure conflicts before editing another paragraph",
            ));
        }
        if edit.expected_absent
            || !expected_fields.is_empty()
            || edit.replacement.inserted_paragraph.is_some()
            || replacement_fields != conflict_fields
        {
            return Err(fail(
                "a structure resolution must replace every conflicting field in its paragraph",
            ));
        }
    }
    if let Some(position) = &edit.replacement.position {
        let known_after = position.after.as_ref().is_none_or(|after| {
            base.paragraphs
                .iter()
                .any(|paragraph| &paragraph.id == after)
                || inserted_ids.contains(after.as_str())
                || edit
                    .replacement
                    .inserted_paragraph
                    .as_ref()
                    .is_some_and(|paragraph| &paragraph.id == after)
        });
        if !known_after {
            return Err(fail("unknown paragraph predecessor"));
        }
    }
    let sequence = current
        .frontier
        .get(&edit.actor)
        .copied()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|value| *value <= MAX_CLOCK)
        .ok_or_else(|| fail("actor sequence exhausted"))?;
    let lamport = current
        .history
        .operations
        .iter()
        .map(|operation| operation.lamport)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|value| *value <= MAX_CLOCK)
        .ok_or_else(|| fail("logical clock exhausted"))?;
    let mut next = current.history;
    next.schema_version = next.schema_version.max(4);
    next.operations.push(StoryTextOperation {
        id: StoryOperationId {
            actor: edit.actor.clone(),
            sequence,
        },
        lamport,
        context: current.frontier,
        paragraph_id: edit.paragraph_id.clone(),
        after: None,
        removed: Vec::new(),
        inserted: String::new(),
        visibility: None,
        paragraph_style: None,
        paragraph_structure: Some(edit.replacement.clone()),
        inserted_style: None,
        inline_style: None,
    });
    match seed {
        Some(seed) => Ok(reconcile_bound(base, &[next], seed_header(seed))?.0),
        None => merge_histories(base, &[next]),
    }
}

pub fn set_operation_active(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    change: &StoryHistorySetActive,
) -> Result<StoryHistoryResult> {
    set_active_bound(base, history, change, None)
}

pub fn set_operations_active(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    change: &StoryHistorySetManyActive,
) -> Result<StoryHistoryResult> {
    set_many_active_bound(base, history, change, None)
}

fn set_active_bound(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    change: &StoryHistorySetActive,
    seed: Option<&StoryHistorySeed>,
) -> Result<StoryHistoryResult> {
    set_many_active_bound(
        base,
        history,
        &StoryHistorySetManyActive {
            expected_history_sha256: change.expected_history_sha256.clone(),
            actor: change.actor.clone(),
            targets: vec![change.target.clone()],
            expected_active: change.expected_active,
            active: change.active,
        },
        seed,
    )
}

fn set_many_active_bound(
    base: &LinkedStoryRequest,
    history: &StoryTextHistory,
    change: &StoryHistorySetManyActive,
    seed: Option<&StoryHistorySeed>,
) -> Result<StoryHistoryResult> {
    actor(&change.actor)?;
    if change.targets.is_empty() || change.targets.len() > 4096 {
        return Err(fail(
            "selective undo/redo group must contain 1..=4096 edits",
        ));
    }
    let mut requested = BTreeSet::new();
    for target in &change.targets {
        id(target)?;
        if target.actor != change.actor || !requested.insert(target.clone()) {
            return Err(fail(
                "selective undo/redo group requires unique original edits from one replica",
            ));
        }
    }
    let current = match seed {
        Some(seed) => reconcile_bound(base, std::slice::from_ref(history), seed_header(seed))?.0,
        None => merge_histories(base, std::slice::from_ref(history))?,
    };
    if current.history_sha256 != change.expected_history_sha256
        || !current.missing_dependencies.is_empty()
    {
        return Err(fail(
            "selective undo/redo requires the exact complete history",
        ));
    }
    let mut targets = Vec::with_capacity(requested.len());
    for target in requested {
        let operation = current
            .history
            .operations
            .iter()
            .find(|operation| operation.id == target)
            .filter(|operation| operation.visibility.is_none())
            .ok_or_else(|| {
                fail("select an original text/style/structure edit, not a control or absent operation")
            })?;
        let enabled = !current.inactive_operations.contains(&target);
        if enabled != change.expected_active {
            return Err(fail("selective undo/redo group activity preimage differs"));
        }
        targets.push((target, operation.paragraph_id.clone()));
    }
    if change.expected_active == change.active {
        return Ok(current);
    }
    let mut frontier = current.frontier.clone();
    let mut lamport = current
        .history
        .operations
        .iter()
        .map(|op| op.lamport)
        .max()
        .unwrap_or(0);
    let mut next = current.history;
    next.schema_version = next.schema_version.max(2);
    for (target, paragraph_id) in targets {
        crate::cancel::check_current_cancel("story text history grouped activity change")?;
        let sequence = frontier
            .get(&change.actor)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .filter(|n| *n <= MAX_CLOCK)
            .ok_or_else(|| fail("actor sequence exhausted"))?;
        lamport = lamport
            .checked_add(1)
            .filter(|clock| *clock <= MAX_CLOCK)
            .ok_or_else(|| fail("logical clock exhausted"))?;
        let context = frontier.clone();
        next.operations.push(StoryTextOperation {
            id: StoryOperationId {
                actor: change.actor.clone(),
                sequence,
            },
            lamport,
            context,
            paragraph_id,
            after: None,
            removed: Vec::new(),
            inserted: String::new(),
            visibility: Some(StoryOperationVisibility {
                target,
                active: change.active,
            }),
            paragraph_style: None,
            paragraph_structure: None,
            inserted_style: None,
            inline_style: None,
        });
        frontier.insert(change.actor.clone(), sequence);
    }
    match seed {
        Some(seed) => Ok(reconcile_bound(base, &[next], seed_header(seed))?.0),
        None => merge_histories(base, &[next]),
    }
}

#[cfg(test)]
#[path = "story_text_history_tests.rs"]
mod tests;
