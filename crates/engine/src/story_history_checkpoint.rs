//! Durable logical-history checkpoints over the canonical story writer. Source
//! owners rebind through saved story markers; old physical byte ranges never do.
use super::*;
use crate::story_text_history::{
    self as text_history, StoryHistoryEdit, StoryHistoryInlineStyleEdit,
    StoryHistoryInlineStyleResolution, StoryHistoryResult, StoryHistorySeed, StoryHistorySetActive,
    StoryHistoryStructureEdit, StoryHistoryStyleEdit, StoryTextHistory,
};
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::{PdfDictionary, PdfObject};

const MAX_METADATA: usize = 16 * 1024 * 1024;
const MAX_GENERATION: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistorySource {
    Start {
        base: LinkedStoryRequest,
        history: StoryTextHistory,
        #[serde(default)]
        replace_epoch: bool,
    },
    Resume {
        input_sha256: String,
        story_id: String,
        expected_checkpoint_sha256: String,
        history: StoryTextHistory,
    },
}
impl HistorySource {
    fn replace_history(&mut self, history: StoryTextHistory) {
        match self {
            Self::Start {
                history: current, ..
            }
            | Self::Resume {
                history: current, ..
            } => *current = history,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedStoryHistory {
    pub schema_version: u32,
    pub generation: u64,
    pub seed: StoryHistorySeed,
    pub seed_sha256: String,
    pub history: StoryTextHistory,
    pub history_sha256: String,
    /// Canonical hash of the raw saved model, before current-revision rebinding.
    pub saved_story_sha256: String,
    pub parent_checkpoint_sha256: Option<String>,
    pub from_revision_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryPreviewReceipt {
    pub layout: StoryPreviewReceipt,
    pub source_sha256: String,
    pub history_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreparedHistory {
    /// Normalized canonical source. Bind this exact source to preview/save.
    pub source: HistorySource,
    pub result: StoryHistoryResult,
    pub checkpoint_before: Option<String>,
    pub generation_before: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct HistoryLayoutPreview {
    pub prepared: PreparedHistory,
    pub preview: LinkedStoryPreview,
    pub receipt: HistoryPreviewReceipt,
}
#[derive(Debug, Clone, Serialize)]
pub struct HistoryCheckpointReport {
    pub story: LinkedStoryPreview,
    pub checkpoint_sha256: String,
    pub generation: u64,
    pub seed_sha256: String,
    pub history_sha256: String,
    pub output_sha256: String,
    pub same_epoch_preserved: bool,
    pub limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryCompactionRequest {
    pub input_sha256: String,
    pub story_id: String,
    pub expected_checkpoint_sha256: String,
    pub expected_history_sha256: String,
    /// Exact complete frontier acknowledged by the host for every replica in
    /// the saved epoch. This is consistency evidence, not authentication.
    pub acknowledged_frontier: BTreeMap<String, u64>,
    pub acknowledge_operation_and_undo_loss: bool,
    pub acknowledge_prior_epoch_rejected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryCompactionPlan {
    pub schema_version: u32,
    pub input_sha256: String,
    pub story_id: String,
    pub checkpoint_before: String,
    pub history_sha256: String,
    pub generation_before: u64,
    pub acknowledged_frontier: BTreeMap<String, u64>,
    pub source_operation_count: usize,
    pub source_atom_count: usize,
    pub source_tombstone_count: usize,
    pub source_inactive_operation_count: usize,
    pub plan_sha256: String,
    pub limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryCompactionReport {
    pub schema_version: u32,
    pub story_id: String,
    pub checkpoint_before: String,
    pub checkpoint_after: String,
    pub generation_before: u64,
    pub generation_after: u64,
    pub seed_before: String,
    pub seed_after: String,
    pub history_before: String,
    pub history_after: String,
    pub retired_operation_count: usize,
    pub retired_atom_count: usize,
    pub retired_tombstone_count: usize,
    pub output_sha256: String,
    pub prior_epoch_rejected: bool,
    pub exact_session_undo_available: bool,
    pub limits: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredStory {
    schema_version: u32,
    request: LinkedStoryRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text_history: Option<PersistedStoryHistory>,
}
struct Context {
    seed: StoryHistorySeed,
    current: LinkedStoryRequest,
    previous: Option<PersistedStoryHistory>,
}
fn history_error(s: &str) -> WellfriendError {
    fail(&format!("story history checkpoint: {s}"))
}

fn read_record(engine: &ContentEngine, story_id: &str) -> Result<Option<StoredStory>> {
    let entries = read_story_metadata(engine)?;
    let Some(value) = entries.get(&hash(story_id.as_bytes())) else {
        return Ok(None);
    };
    let object = engine.document().reader().resolve(value.clone())?;
    let decoded = crate::filters::decode_stream_lossless_with_limits(
        &object,
        engine.document().reader(),
        &crate::filters::DecodeLimits {
            max_decoded_bytes_per_stream: MAX_METADATA as u64,
            ..Default::default()
        },
    )?;
    if decoded.status != crate::filters::StreamDecodeStatus::Complete {
        return Err(history_error("opaque stored story"));
    }
    let record: StoredStory =
        serde_json::from_slice(&decoded.data).map_err(|e| history_error(&e.to_string()))?;
    if !supported_story_schema(record.schema_version, &record.request)
        || record.request.story_id != story_id
    {
        return Err(history_error("saved story identity/schema mismatch"));
    }
    Ok(Some(record))
}

pub(super) fn encode_saved_metadata(
    engine: &ContentEngine,
    request: LinkedStoryRequest,
) -> Result<Vec<u8>> {
    let previous = read_record(engine, &request.story_id)?.and_then(|record| record.text_history);
    let record = StoredStory {
        schema_version: required_story_schema(&request),
        request,
        text_history: previous,
    };
    bounded_metadata(&record)
}
fn bounded_metadata(record: &StoredStory) -> Result<Vec<u8>> {
    let raw = serde_json::to_vec(record).map_err(|e| history_error(&e.to_string()))?;
    if raw.len() > MAX_METADATA {
        return Err(history_error(
            "saved model and history exceed the combined 16 MiB metadata budget",
        ));
    }
    Ok(raw)
}

fn checkpoint_hash(checkpoint: &PersistedStoryHistory) -> Result<String> {
    value_hash(checkpoint)
}
fn same_text(a: &LinkedStoryRequest, b: &LinkedStoryRequest) -> bool {
    a.paragraphs == b.paragraphs
}
fn bound_record(
    input: &[u8],
    story_id: &str,
) -> Result<(
    PersistedStoryHistory,
    LinkedStoryRequest,
    StoryHistoryResult,
)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let record =
        read_record(&engine, story_id)?.ok_or_else(|| history_error("saved story is absent"))?;
    let checkpoint = record
        .text_history
        .ok_or_else(|| history_error("story has no saved causal history"))?;
    if checkpoint.schema_version != 1
        || checkpoint.generation == 0
        || checkpoint.generation > MAX_GENERATION
        || checkpoint.seed_sha256 != value_hash(&checkpoint.seed)?
        || checkpoint.history_sha256 != value_hash(&checkpoint.history)?
    {
        return Err(history_error("invalid stored checkpoint identity"));
    }
    if checkpoint.saved_story_sha256 != value_hash(&record.request)? {
        return Err(history_error("history is detached: the saved story changed outside its causal checkpoint; explicit reconciliation or a new epoch is required"));
    }
    // The canonical loader verifies every frame/image/annotation binding against
    // actual content and resolves page moves. Metadata equality alone is not a
    // license to select matching text on a newer PDF revision.
    let current = load_linked_stories_in_engine(input, &engine)?
        .into_iter()
        .find(|story| story.request.story_id == story_id)
        .ok_or_else(|| history_error("current story ownership is absent"))?
        .request;
    let projection = text_history::merge_seed(
        &checkpoint.seed,
        &current,
        std::slice::from_ref(&checkpoint.history),
    )?;
    if projection.history != checkpoint.history
        || !projection.missing_dependencies.is_empty()
        || projection
            .merged
            .as_ref()
            .is_none_or(|model| !same_text(model, &current))
    {
        return Err(history_error(
            "saved causal history does not explain the current story paragraph projection",
        ));
    }
    Ok((checkpoint, current, projection))
}

fn prepare_inner(input: &[u8], source: &HistorySource) -> Result<(PreparedHistory, Context)> {
    crate::cancel::check_current_cancel("history source binding")?;
    let revision = hash(input);
    let (context, incoming) = match source {
        HistorySource::Start {
            base,
            history,
            replace_epoch,
        } => {
            if base.input_sha256 != revision {
                return Err(history_error("new epoch base differs from current PDF"));
            }
            let engine = ContentEngine::open_bytes(input.to_vec())?;
            let previous =
                read_record(&engine, &base.story_id)?.and_then(|record| record.text_history);
            if previous.is_some() && !replace_epoch {
                return Err(history_error("starting a new epoch would replace saved history; explicit replace_epoch decision required"));
            }
            (
                Context {
                    seed: text_history::seed_from_base(base)?,
                    current: base.clone(),
                    previous,
                },
                vec![history.clone()],
            )
        }
        HistorySource::Resume {
            input_sha256,
            story_id,
            expected_checkpoint_sha256,
            history,
        } => {
            if input_sha256 != &revision {
                return Err(history_error(
                    "resume source belongs to another current PDF revision",
                ));
            }
            let (previous, current, _) = bound_record(input, story_id)?;
            if checkpoint_hash(&previous)? != *expected_checkpoint_sha256 {
                return Err(history_error("saved checkpoint compare-and-swap failed"));
            }
            let histories = vec![previous.history.clone(), history.clone()];
            (
                Context {
                    seed: previous.seed.clone(),
                    current,
                    previous: Some(previous),
                },
                histories,
            )
        }
    };
    let result = text_history::merge_seed(&context.seed, &context.current, &incoming)?;
    let mut normalized = source.clone();
    normalized.replace_history(result.history.clone());
    let checkpoint_before = context.previous.as_ref().map(checkpoint_hash).transpose()?;
    let generation_before = context
        .previous
        .as_ref()
        .map(|checkpoint| checkpoint.generation);
    Ok((
        PreparedHistory {
            source: normalized,
            result,
            checkpoint_before,
            generation_before,
        },
        context,
    ))
}

pub fn prepare_history(input: &[u8], source: &HistorySource) -> Result<PreparedHistory> {
    Ok(prepare_inner(input, source)?.0)
}
pub fn resume_history(input: &[u8], story_id: &str) -> Result<PreparedHistory> {
    crate::cancel::check_current_cancel("history resume")?;
    let (checkpoint, _, result) = bound_record(input, story_id)?;
    let checkpoint_sha256 = checkpoint_hash(&checkpoint)?;
    Ok(PreparedHistory {
        source: HistorySource::Resume {
            input_sha256: hash(input),
            story_id: story_id.into(),
            expected_checkpoint_sha256: checkpoint_sha256.clone(),
            history: checkpoint.history,
        },
        result,
        checkpoint_before: Some(checkpoint_sha256),
        generation_before: Some(checkpoint.generation),
    })
}
pub fn join_history(
    input: &[u8],
    source: &HistorySource,
    histories: &[StoryTextHistory],
) -> Result<PreparedHistory> {
    let (mut prepared, context) = prepare_inner(input, source)?;
    if histories.len() > 255 {
        return Err(history_error("incoming branch budget exceeded"));
    }
    let mut branches = Vec::with_capacity(histories.len() + 1);
    branches.push(prepared.result.history.clone());
    branches.extend_from_slice(histories);
    prepared.result = text_history::merge_seed(&context.seed, &context.current, &branches)?;
    prepared
        .source
        .replace_history(prepared.result.history.clone());
    Ok(prepared)
}
pub fn edit_history(
    input: &[u8],
    source: &HistorySource,
    edit: &StoryHistoryEdit,
) -> Result<PreparedHistory> {
    let (mut prepared, context) = prepare_inner(input, source)?;
    prepared.result = text_history::edit_seed(
        &context.seed,
        &context.current,
        &prepared.result.history,
        edit,
    )?;
    prepared
        .source
        .replace_history(prepared.result.history.clone());
    Ok(prepared)
}

pub fn edit_history_style(
    input: &[u8],
    source: &HistorySource,
    edit: &StoryHistoryStyleEdit,
) -> Result<PreparedHistory> {
    let (mut prepared, context) = prepare_inner(input, source)?;
    prepared.result = text_history::edit_style_seed(
        &context.seed,
        &context.current,
        &prepared.result.history,
        edit,
    )?;
    prepared
        .source
        .replace_history(prepared.result.history.clone());
    Ok(prepared)
}

pub fn edit_history_structure(
    input: &[u8],
    source: &HistorySource,
    edit: &StoryHistoryStructureEdit,
) -> Result<PreparedHistory> {
    let (mut prepared, context) = prepare_inner(input, source)?;
    prepared.result = text_history::edit_structure_seed(
        &context.seed,
        &context.current,
        &prepared.result.history,
        edit,
    )?;
    prepared
        .source
        .replace_history(prepared.result.history.clone());
    Ok(prepared)
}

pub fn edit_history_inline_style(
    input: &[u8],
    source: &HistorySource,
    edit: &StoryHistoryInlineStyleEdit,
) -> Result<PreparedHistory> {
    let (mut prepared, context) = prepare_inner(input, source)?;
    prepared.result = text_history::edit_inline_style_seed(
        &context.seed,
        &context.current,
        &prepared.result.history,
        edit,
    )?;
    prepared
        .source
        .replace_history(prepared.result.history.clone());
    Ok(prepared)
}

pub fn resolve_history_inline_style(
    input: &[u8],
    source: &HistorySource,
    resolution: &StoryHistoryInlineStyleResolution,
) -> Result<PreparedHistory> {
    let (mut prepared, context) = prepare_inner(input, source)?;
    prepared.result = text_history::resolve_inline_style_conflicts_seed(
        &context.seed,
        &context.current,
        &prepared.result.history,
        resolution,
    )?;
    prepared
        .source
        .replace_history(prepared.result.history.clone());
    Ok(prepared)
}
pub fn history_delta(
    input: &[u8],
    source: &HistorySource,
    peer: &BTreeMap<String, u64>,
) -> Result<StoryTextHistory> {
    text_history::delta_from_canonical(prepare_history(input, source)?.result.history, peer)
}

pub fn set_operation_active(
    input: &[u8],
    source: &HistorySource,
    change: &StoryHistorySetActive,
) -> Result<PreparedHistory> {
    let (mut prepared, context) = prepare_inner(input, source)?;
    prepared.result = text_history::set_active_seed(
        &context.seed,
        &context.current,
        &prepared.result.history,
        change,
    )?;
    prepared
        .source
        .replace_history(prepared.result.history.clone());
    Ok(prepared)
}

pub fn set_operations_active(
    input: &[u8],
    source: &HistorySource,
    change: &crate::story_text_history::StoryHistorySetManyActive,
) -> Result<PreparedHistory> {
    let (mut prepared, context) = prepare_inner(input, source)?;
    prepared.result = text_history::set_many_active_seed(
        &context.seed,
        &context.current,
        &prepared.result.history,
        change,
    )?;
    prepared
        .source
        .replace_history(prepared.result.history.clone());
    Ok(prepared)
}

fn compaction_plan_inner(
    input: &[u8],
    request: &HistoryCompactionRequest,
) -> Result<(
    HistoryCompactionPlan,
    PersistedStoryHistory,
    LinkedStoryRequest,
    StoryHistoryResult,
)> {
    crate::cancel::check_current_cancel("history compaction planning")?;
    let input_sha256 = hash(input);
    if request.input_sha256 != input_sha256 {
        return Err(history_error(
            "compaction request belongs to another PDF revision",
        ));
    }
    if request.story_id.is_empty() || request.story_id.len() > 1024 {
        return Err(history_error("invalid compaction story identity"));
    }
    if !request.acknowledge_operation_and_undo_loss || !request.acknowledge_prior_epoch_rejected {
        return Err(history_error(
            "compaction requires explicit history/undo loss and stale-epoch rejection acknowledgements",
        ));
    }
    let (checkpoint, current, result) = bound_record(input, &request.story_id)?;
    let checkpoint_before = checkpoint_hash(&checkpoint)?;
    if checkpoint_before != request.expected_checkpoint_sha256
        || checkpoint.history_sha256 != request.expected_history_sha256
    {
        return Err(history_error(
            "compaction checkpoint/history compare-and-swap failed",
        ));
    }
    if checkpoint.history.operations.is_empty() {
        return Err(history_error("saved history is already compact"));
    }
    if result.frontier != request.acknowledged_frontier {
        return Err(history_error(
            "compaction requires the exact complete saved replica frontier",
        ));
    }
    let request_sha256 = value_hash(request)?;
    let plan_sha256 = value_hash(&(
        "story-history-compaction-v1",
        &request_sha256,
        &checkpoint_before,
        checkpoint.generation,
        &checkpoint.seed_sha256,
        &checkpoint.history_sha256,
    ))?;
    let plan = HistoryCompactionPlan {
        schema_version: 1,
        input_sha256,
        story_id: request.story_id.clone(),
        checkpoint_before,
        history_sha256: checkpoint.history_sha256.clone(),
        generation_before: checkpoint.generation,
        acknowledged_frontier: result.frontier.clone(),
        source_operation_count: checkpoint.history.operations.len(),
        source_atom_count: result.atom_count,
        source_tombstone_count: result.tombstone_count,
        source_inactive_operation_count: result.inactive_operations.len(),
        plan_sha256,
        limits: vec![
            "replica frontier acknowledgements are host assertions, not authenticated signatures"
                .into(),
            "compaction creates a new epoch and permanently removes selective undo/history from the current revision; exact PDF session undo remains bounded and separate"
                .into(),
            "a replica retaining the prior epoch must not merge it into the compacted epoch; export/archive before approval if history is needed"
                .into(),
        ],
    };
    Ok((plan, checkpoint, current, result))
}

pub fn plan_history_compaction(
    input: &[u8],
    request: &HistoryCompactionRequest,
) -> Result<HistoryCompactionPlan> {
    Ok(compaction_plan_inner(input, request)?.0)
}

fn apply_history_compaction(
    input: &[u8],
    request: &HistoryCompactionRequest,
    approved_plan_sha256: &str,
) -> Result<(Vec<u8>, HistoryCompactionReport)> {
    let (plan, previous, current, previous_result) = compaction_plan_inner(input, request)?;
    if approved_plan_sha256.len() != 64
        || !approved_plan_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !plan.plan_sha256.eq_ignore_ascii_case(approved_plan_sha256)
    {
        return Err(history_error(
            "compaction requires the exact approved plan hash",
        ));
    }
    let seed = text_history::seed_from_base(&current)?;
    if seed.base_revision_sha256 != request.input_sha256 {
        return Err(history_error(
            "compaction seed did not bind the current PDF revision",
        ));
    }
    let empty = text_history::new_history(&current)?;
    let result = text_history::merge_seed(&seed, &current, std::slice::from_ref(&empty))?;
    if result.history != empty
        || !result.missing_dependencies.is_empty()
        || result
            .merged
            .as_ref()
            .is_none_or(|model| !same_text(model, &current))
    {
        return Err(history_error(
            "compaction could not construct an exact empty replacement epoch",
        ));
    }
    let context = Context {
        seed: seed.clone(),
        current: current.clone(),
        previous: Some(previous.clone()),
    };
    let prepared = PreparedHistory {
        source: HistorySource::Start {
            base: current,
            history: empty,
            replace_epoch: true,
        },
        result,
        checkpoint_before: Some(plan.checkpoint_before.clone()),
        generation_before: Some(plan.generation_before),
    };
    let (output, saved) = attach_history(input, &context, &prepared, &request.input_sha256)?;
    let checkpoint_after = checkpoint_hash(&saved)?;
    if !saved.history.operations.is_empty()
        || saved.seed_sha256 == previous.seed_sha256
        || saved.parent_checkpoint_sha256.as_deref() != Some(plan.checkpoint_before.as_str())
    {
        return Err(history_error(
            "compaction output did not establish the expected new empty epoch",
        ));
    }
    let report = HistoryCompactionReport {
        schema_version: 1,
        story_id: request.story_id.clone(),
        checkpoint_before: plan.checkpoint_before,
        checkpoint_after,
        generation_before: plan.generation_before,
        generation_after: saved.generation,
        seed_before: previous.seed_sha256,
        seed_after: saved.seed_sha256,
        history_before: previous.history_sha256,
        history_after: saved.history_sha256,
        retired_operation_count: previous.history.operations.len(),
        retired_atom_count: previous_result.atom_count,
        retired_tombstone_count: previous_result.tombstone_count,
        output_sha256: hash(&output),
        prior_epoch_rejected: true,
        // Finalized after publication because the session's bounded byte
        // history, rather than the logical transaction, owns this property.
        exact_session_undo_available: false,
        limits: plan.limits,
    };
    Ok((output, report))
}

fn attach_history(
    output: &[u8],
    context: &Context,
    prepared: &PreparedHistory,
    from_revision: &str,
) -> Result<(Vec<u8>, PersistedStoryHistory)> {
    let engine = ContentEngine::open_bytes(output.to_vec())?;
    let mut record = read_record(&engine, &context.current.story_id)?
        .ok_or_else(|| history_error("writer did not save the story model"))?;
    let model = prepared
        .result
        .merged
        .as_ref()
        .ok_or_else(|| history_error("missing dependencies prevent checkpoint"))?;
    if !same_text(&record.request, model) {
        return Err(history_error(
            "saved text differs from the approved projection",
        ));
    }
    let generation = context
        .previous
        .as_ref()
        .map(|p| p.generation)
        .unwrap_or(0)
        .checked_add(1)
        .filter(|n| *n <= MAX_GENERATION)
        .ok_or_else(|| history_error("checkpoint generation exhausted"))?;
    let checkpoint = PersistedStoryHistory {
        schema_version: 1,
        generation,
        seed: context.seed.clone(),
        seed_sha256: value_hash(&context.seed)?,
        history: prepared.result.history.clone(),
        history_sha256: prepared.result.history_sha256.clone(),
        saved_story_sha256: value_hash(&record.request)?,
        parent_checkpoint_sha256: prepared.checkpoint_before.clone(),
        from_revision_sha256: from_revision.into(),
    };
    record.text_history = Some(checkpoint.clone());
    let raw = bounded_metadata(&record)?;
    let reader = engine.document().reader();
    let size = reader
        .trailer()
        .get_integer("Size")
        .map(u32::try_from)
        .transpose()
        .map_err(|_| history_error("invalid xref Size"))?
        .unwrap_or(1);
    let number = reader
        .object_ids()
        .iter()
        .map(|id| id.0)
        .max()
        .unwrap_or(0)
        .max(size.saturating_sub(1))
        .checked_add(1)
        .ok_or_else(|| history_error("metadata object space exhausted"))?;
    let mut stream = PdfDictionary::empty();
    stream.insert("Type", PdfObject::Name("WellfriendStory".into()));
    stream.insert("Length", PdfObject::Integer(raw.len() as i64));
    let mut entries = read_story_metadata(&engine)?;
    entries.insert(
        hash(context.current.story_id.as_bytes()),
        PdfObject::Reference {
            number,
            generation: 0,
        },
    );
    let mut values = PdfDictionary::empty();
    for (key, value) in entries {
        values.insert(key, value);
    }
    let mut catalog = engine.document().get_catalog()?;
    catalog.insert("WellfriendStories", PdfObject::Dictionary(values));
    let root = reader
        .root_reference()
        .ok_or_else(|| history_error("missing catalog identity"))?;
    let bytes = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number,
                generation: 0,
                object: PdfObject::Stream { dict: stream, raw },
            },
            IncrementalObject {
                number: root.0,
                generation: root.1,
                object: PdfObject::Dictionary(catalog),
            },
        ],
    )?;
    let (saved, _, _) = bound_record(&bytes, &context.current.story_id)?;
    if checkpoint_hash(&saved)? != checkpoint_hash(&checkpoint)? {
        return Err(history_error(
            "saved history differs from the approved checkpoint",
        ));
    }
    Ok((bytes, checkpoint))
}

impl LinkedStorySession {
    pub fn compact_history(
        &mut self,
        request: &HistoryCompactionRequest,
        approved_plan_sha256: &str,
        cancel: &crate::CancelToken,
    ) -> Result<HistoryCompactionReport> {
        cancel.check("approved history compaction")?;
        let scope = crate::CancelToken::linked_pair(cancel, &crate::cancel::current_cancel_token());
        let (output, mut report) = scope
            .scope(|| apply_history_compaction(self.bytes(), request, approved_plan_sha256))?;
        report.exact_session_undo_available = self.publish_bytes(output, cancel)?;
        Ok(report)
    }

    pub fn preview_history(
        &mut self,
        source: &HistorySource,
        cancel: &crate::CancelToken,
    ) -> Result<HistoryLayoutPreview> {
        cancel.check("history layout preview")?;
        self.history_receipt = None;
        let scope = crate::CancelToken::linked_pair(cancel, &crate::cancel::current_cancel_token());
        let prepared = scope.scope(|| prepare_history(self.bytes(), source))?;
        let request = prepared
            .result
            .merged
            .as_ref()
            .ok_or_else(|| history_error("receive missing dependencies before preview"))?;
        let preview = self.preview(request, cancel)?;
        let receipt = HistoryPreviewReceipt {
            layout: self.preview_receipt()?,
            source_sha256: value_hash(&prepared.source)?,
            history_sha256: prepared.result.history_sha256.clone(),
        };
        cancel.check("history preview publication")?;
        self.history_receipt = Some(receipt.clone());
        Ok(HistoryLayoutPreview {
            prepared,
            preview,
            receipt,
        })
    }
    pub fn checkpoint_history(
        &mut self,
        source: &HistorySource,
        receipt: &HistoryPreviewReceipt,
        cancel: &crate::CancelToken,
    ) -> Result<HistoryCheckpointReport> {
        cancel.check("approved history checkpoint")?;
        if self.history_receipt.as_ref() != Some(receipt)
            || self.preview_receipt()? != receipt.layout
        {
            return Err(history_error(
                "history receipt is absent, stale or replaced by another preview",
            ));
        }
        let scope = crate::CancelToken::linked_pair(cancel, &crate::cancel::current_cancel_token());
        let (prepared, context) = scope.scope(|| prepare_inner(self.bytes(), source))?;
        let request = prepared
            .result
            .merged
            .as_ref()
            .ok_or_else(|| history_error("incomplete history cannot checkpoint"))?;
        if value_hash(&prepared.source)? != receipt.source_sha256
            || prepared.result.history_sha256 != receipt.history_sha256
            || value_hash(request)? != receipt.layout.request_sha256
        {
            return Err(history_error("history or projection changed after review"));
        }
        let from_revision = self.revision_sha256();
        let (output, mut story) = scope.scope(|| apply_linked_story(self.bytes(), request))?;
        let (output, checkpoint) =
            scope.scope(|| attach_history(&output, &context, &prepared, &from_revision))?;
        let output_sha256 = hash(&output);
        story.output_sha256 = Some(output_sha256.clone());
        let checkpoint_sha256 = checkpoint_hash(&checkpoint)?;
        let same_epoch_preserved = context
            .previous
            .as_ref()
            .is_some_and(|old| old.seed_sha256 == checkpoint.seed_sha256);
        let report = HistoryCheckpointReport {
            story: story.clone(),
            checkpoint_sha256,
            generation: checkpoint.generation,
            seed_sha256: checkpoint.seed_sha256,
            history_sha256: checkpoint.history_sha256,
            output_sha256,
            same_epoch_preserved,
            limits: vec![
                "checkpoint hashes bind source/history, not author authentication or proof of PDF correctness".into(),
                "all private writer and metadata stages publish together in one session undo step; historical PDF bytes retain deleted text".into(),
                "arbitrary external story edits detach the lineage; rebase source owners only through verified saved markers".into(),
            ],
        };
        self.publish_checkpoint(output, story, cancel)?;
        Ok(report)
    }
}

#[cfg(test)]
#[path = "story_history_checkpoint_tests.rs"]
mod tests;
