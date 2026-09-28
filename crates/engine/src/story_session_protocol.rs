//! Bounded, host-independent JSON transport for the retained story editor.
//! No filesystem/network access and no separate layout or transaction writer.
use crate::linked_stories::{LinkedStoryRequest, LinkedStorySession, StoryPreviewReceipt};
use crate::{CancelToken, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[cfg(test)]
#[path = "story_session_protocol_tests.rs"]
mod tests;

pub const MAX_INPUT_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_COMMAND_BYTES: usize = 32 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum StorySessionCommand {
    Status {},
    InspectFont {
        bytes: Vec<u8>,
    },
    PrepareFont {
        lookup_name: String,
        bytes: Vec<u8>,
        selection: crate::fonts::font_asset::FontFaceSelection,
    },
    PrepareFontInstance {
        lookup_name: String,
        bytes: Vec<u8>,
        request: crate::fonts::font_instance::FontInstanceRequest,
    },
    SavedStories {},
    Pages {},
    SourceModel {
        page: usize,
    },
    AnnotationSources {},
    ImageSources {
        page: usize,
    },
    TagSources {},
    PageGeometry {
        page: usize,
        dpi: u32,
    },
    Preview {
        request: LinkedStoryRequest,
    },
    SynchronizeTableValues {
        request: LinkedStoryRequest,
    },
    Checkpoint {
        request: LinkedStoryRequest,
        receipt: StoryPreviewReceipt,
    },
    Undo {},
    Redo {},
    MergeText {
        request: crate::story_merge::StoryMergeRequest,
    },
    MergeStructure {
        request: crate::story_structure_merge::StoryStructureMergeRequest,
    },
    ReviewStructure {
        request: crate::story_structure_merge::StoryStructureMergeRequest,
    },
    ResolveStructure {
        request: crate::story_structure_merge::StoryStructureMergeRequest,
        resolution: crate::story_structure_merge::resolution::StructureResolution,
    },
    TextHistoryNew {
        base: LinkedStoryRequest,
    },
    TextHistoryMerge {
        base: LinkedStoryRequest,
        histories: Vec<crate::story_text_history::StoryTextHistory>,
    },
    TextHistoryEdit {
        base: LinkedStoryRequest,
        history: crate::story_text_history::StoryTextHistory,
        edit: crate::story_text_history::StoryHistoryEdit,
    },
    TextHistoryStyle {
        base: LinkedStoryRequest,
        history: crate::story_text_history::StoryTextHistory,
        edit: crate::story_text_history::StoryHistoryStyleEdit,
    },
    TextHistoryStructure {
        base: LinkedStoryRequest,
        history: crate::story_text_history::StoryTextHistory,
        edit: crate::story_text_history::StoryHistoryStructureEdit,
    },
    TextHistoryInlineStyle {
        base: LinkedStoryRequest,
        history: crate::story_text_history::StoryTextHistory,
        edit: crate::story_text_history::StoryHistoryInlineStyleEdit,
    },
    TextHistoryResolveInlineStyle {
        base: LinkedStoryRequest,
        history: crate::story_text_history::StoryTextHistory,
        resolution: crate::story_text_history::StoryHistoryInlineStyleResolution,
    },
    TextHistoryDelta {
        base: LinkedStoryRequest,
        history: crate::story_text_history::StoryTextHistory,
        peer: std::collections::BTreeMap<String, u64>,
    },
    TextHistorySetActive {
        base: LinkedStoryRequest,
        history: crate::story_text_history::StoryTextHistory,
        change: crate::story_text_history::StoryHistorySetActive,
    },
    TextHistorySetManyActive {
        base: LinkedStoryRequest,
        history: crate::story_text_history::StoryTextHistory,
        change: crate::story_text_history::StoryHistorySetManyActive,
    },
    HistoryResume {
        story_id: String,
    },
    HistoryPrepare {
        source: crate::linked_stories::history::HistorySource,
    },
    HistoryJoin {
        source: crate::linked_stories::history::HistorySource,
        histories: Vec<crate::story_text_history::StoryTextHistory>,
    },
    HistoryEdit {
        source: crate::linked_stories::history::HistorySource,
        edit: crate::story_text_history::StoryHistoryEdit,
    },
    HistoryStyle {
        source: crate::linked_stories::history::HistorySource,
        edit: crate::story_text_history::StoryHistoryStyleEdit,
    },
    HistoryStructure {
        source: crate::linked_stories::history::HistorySource,
        edit: crate::story_text_history::StoryHistoryStructureEdit,
    },
    HistoryInlineStyle {
        source: crate::linked_stories::history::HistorySource,
        edit: crate::story_text_history::StoryHistoryInlineStyleEdit,
    },
    HistoryResolveInlineStyle {
        source: crate::linked_stories::history::HistorySource,
        resolution: crate::story_text_history::StoryHistoryInlineStyleResolution,
    },
    HistoryDelta {
        source: crate::linked_stories::history::HistorySource,
        peer: std::collections::BTreeMap<String, u64>,
    },
    HistorySetActive {
        source: crate::linked_stories::history::HistorySource,
        change: crate::story_text_history::StoryHistorySetActive,
    },
    HistorySetManyActive {
        source: crate::linked_stories::history::HistorySource,
        change: crate::story_text_history::StoryHistorySetManyActive,
    },
    HistoryPreview {
        source: crate::linked_stories::history::HistorySource,
    },
    HistoryCheckpoint {
        source: crate::linked_stories::history::HistorySource,
        receipt: crate::linked_stories::history::HistoryPreviewReceipt,
    },
    HistoryCompactionPlan {
        request: crate::linked_stories::history::HistoryCompactionRequest,
    },
    HistoryCompactionApply {
        request: crate::linked_stories::history::HistoryCompactionRequest,
        approved_plan_sha256: String,
    },
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| WellfriendError::invalid_input(e.to_string()))
}

pub fn open(bytes: &[u8], cancel: &CancelToken) -> Result<LinkedStorySession> {
    open_with_password(bytes, b"", cancel)
}

pub fn open_with_password(
    bytes: &[u8],
    password: &[u8],
    cancel: &CancelToken,
) -> Result<LinkedStorySession> {
    if bytes.is_empty() || bytes.len() > MAX_INPUT_BYTES {
        return Err(WellfriendError::invalid_input(
            "story input must be 1..=256 MiB",
        ));
    }
    cancel.check("story session open")?;
    let session =
        cancel.scope(|| LinkedStorySession::open_with_password(bytes.to_vec(), password))?;
    cancel.check("story session open publication")?;
    Ok(session)
}

pub fn execute_json(
    session: &mut LinkedStorySession,
    bytes: &[u8],
    cancel: &CancelToken,
) -> Result<Vec<u8>> {
    cancel.check("story session command")?;
    if bytes.len() > MAX_COMMAND_BYTES {
        return Err(WellfriendError::invalid_input(
            "story command exceeds 32 MiB",
        ));
    }
    let command: StorySessionCommand = serde_json::from_slice(bytes)
        .map_err(|e| WellfriendError::invalid_input(format!("story command: {e}")))?;
    execute(session, command, cancel)
}

pub fn execute(
    session: &mut LinkedStorySession,
    command: StorySessionCommand,
    cancel: &CancelToken,
) -> Result<Vec<u8>> {
    cancel.check("story session dispatch")?;
    // Do not turn a successfully published checkpoint into a cancellation
    // error afterward. Mutations check immediately before their publication.
    cancel.scope(|| match command {
        StorySessionCommand::Status {} => encode(&json!({
            "schema_version": 1, "revision_sha256": session.revision_sha256(),
            "line_break_policy_version": 2,
            "line_shaping_context_version": 3,
            "story_pagination_policy_version": 1,
            "tab_stop_layout_version": crate::fonts::tab_stops::TAB_STOP_LAYOUT_VERSION,
            "saved_story_schema_max": crate::linked_stories::SAVED_STORY_SCHEMA_MAX,
            "font_asset_protocol_version": 1,
            "font_instance_protocol_version": 4,
            "history_compaction_protocol_version": 1,
            "history_paragraph_style_protocol_version": 1,
            "history_paragraph_structure_protocol_version": 1,
            "history_inline_style_protocol_version": 1,
            "history_seed_schema_max": crate::story_text_history::HISTORY_SEED_SCHEMA_MAX,
            "pages": session.document().page_count()?, "byte_length": session.bytes().len(),
            "source_security": session.source_security_status(),
            "history": session.history_status(),
            "preview_receipt": session.preview_receipt().ok()
        })),
        StorySessionCommand::InspectFont { bytes } => {
            check_transport_font_size(&bytes)?;
            encode(&crate::fonts::font_asset::inspect_font_asset(&bytes)?)
        }
        StorySessionCommand::PrepareFont {
            lookup_name,
            bytes,
            selection,
        } => {
            check_transport_font_size(&bytes)?;
            let (asset, report) =
                crate::editing_transactions::ApprovedFontAsset::from_font_face_bounded(
                    lookup_name,
                    &bytes,
                    &selection,
                    4 * 1024 * 1024,
                )?;
            check_transport_font_size(&asset.bytes)?;
            encode(&json!({"asset": asset, "report": report}))
        }
        StorySessionCommand::PrepareFontInstance {
            lookup_name,
            bytes,
            request,
        } => {
            check_transport_font_size(&bytes)?;
            let (asset, report) =
                crate::editing_transactions::ApprovedFontAsset::from_font_instance_bounded(
                    lookup_name,
                    &bytes,
                    &request,
                    4 * 1024 * 1024,
                )?;
            check_transport_font_size(&asset.bytes)?;
            encode(&json!({"asset": asset, "report": report}))
        }
        StorySessionCommand::SavedStories {} => encode(&session.saved_stories()?),
        StorySessionCommand::Pages {} => encode(
            &session
                .document()
                .document()
                .get_pages()?
                .iter()
                .map(|p| {
                    json!({"page":p.page_number,"crop_box":p.crop_box,"media_box":p.media_box,
                "rotate":p.rotate,"user_unit":p.user_unit})
                })
                .collect::<Vec<_>>(),
        ),
        StorySessionCommand::SourceModel { page } => encode(
            &crate::advanced_editing::analyze_multi_run_text_range(session.bytes(), page)?,
        ),
        StorySessionCommand::AnnotationSources {} => encode(
            &crate::story_anchors::annotation_anchor_sources(session.bytes())?,
        ),
        StorySessionCommand::ImageSources { page } => encode(
            &crate::universal_editing::universal_image_occurrences_v2(session.bytes(), &[page])?,
        ),
        StorySessionCommand::TagSources {} => {
            encode(&crate::tagged_structure::story::sources(session.bytes())?)
        }
        StorySessionCommand::PageGeometry { page, dpi } => {
            check_dpi(dpi)?;
            let v = session.document().page_viewport(page, dpi)?;
            let m = v.to_transform();
            encode(&json!({"page":page,"width":v.width_px,"height":v.height_px,
                "pdf_to_device":[m.a,m.b,m.c,m.d,m.e,m.f]}))
        }
        StorySessionCommand::Preview { request } => {
            let preview = session.preview(&request, cancel)?;
            encode(
                &json!({"preview":preview,"receipt":session.preview_receipt()?,
                "dirty_regions":session.dirty_regions()}),
            )
        }
        StorySessionCommand::SynchronizeTableValues { mut request } => {
            if request.input_sha256 != session.revision_sha256() {
                return Err(WellfriendError::invalid_input(
                    "table draft differs from the open revision",
                ));
            }
            crate::linked_stories::tables::synchronize_values(&mut request)?;
            encode(&request)
        }
        StorySessionCommand::Checkpoint { request, receipt } => {
            encode(&session.checkpoint_approved(&request, &receipt, cancel)?)
        }
        StorySessionCommand::Undo {} => encode(&session.undo()?),
        StorySessionCommand::Redo {} => encode(&session.redo()?),
        StorySessionCommand::MergeText { request } => {
            if request.base.input_sha256 != session.revision_sha256() {
                return Err(WellfriendError::invalid_input(
                    "merge base differs from the open revision",
                ));
            }
            encode(&crate::story_merge::merge_story_branches(&request)?)
        }
        StorySessionCommand::MergeStructure { request } => {
            if request.base.input_sha256 != session.revision_sha256() {
                return Err(WellfriendError::invalid_input(
                    "merge base differs from the open revision",
                ));
            }
            encode(&crate::story_structure_merge::merge_story_structure(
                &request,
            )?)
        }
        StorySessionCommand::TextHistoryNew { base } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::merge_histories(&base, &[])?)
        }
        StorySessionCommand::TextHistoryMerge { base, histories } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::merge_histories(
                &base, &histories,
            )?)
        }
        StorySessionCommand::TextHistoryEdit {
            base,
            history,
            edit,
        } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::edit_history(
                &base, &history, &edit,
            )?)
        }
        StorySessionCommand::TextHistoryStyle {
            base,
            history,
            edit,
        } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::edit_paragraph_style(
                &base, &history, &edit,
            )?)
        }
        StorySessionCommand::TextHistoryStructure {
            base,
            history,
            edit,
        } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::edit_paragraph_structure(
                &base, &history, &edit,
            )?)
        }
        StorySessionCommand::TextHistoryInlineStyle {
            base,
            history,
            edit,
        } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::edit_inline_style(
                &base, &history, &edit,
            )?)
        }
        StorySessionCommand::TextHistoryResolveInlineStyle {
            base,
            history,
            resolution,
        } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::resolve_inline_style_conflicts(
                &base,
                &history,
                &resolution,
            )?)
        }
        StorySessionCommand::TextHistoryDelta {
            base,
            history,
            peer,
        } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::export_delta(
                &base, &history, &peer,
            )?)
        }
        StorySessionCommand::HistoryResume { story_id } => encode(
            &crate::linked_stories::history::resume_history(session.bytes(), &story_id)?,
        ),
        StorySessionCommand::ReviewStructure { request } => {
            check_history_revision(session, &request.base)?;
            encode(&crate::story_structure_merge::resolution::review_story_structure(&request)?)
        }
        StorySessionCommand::ResolveStructure {
            request,
            resolution,
        } => {
            check_history_revision(session, &request.base)?;
            encode(
                &crate::story_structure_merge::resolution::resolve_story_structure(
                    &request,
                    &resolution,
                )?,
            )
        }
        StorySessionCommand::TextHistorySetActive {
            base,
            history,
            change,
        } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::set_operation_active(
                &base, &history, &change,
            )?)
        }
        StorySessionCommand::TextHistorySetManyActive {
            base,
            history,
            change,
        } => {
            check_history_revision(session, &base)?;
            encode(&crate::story_text_history::set_operations_active(
                &base, &history, &change,
            )?)
        }
        StorySessionCommand::HistorySetActive { source, change } => {
            encode(&crate::linked_stories::history::set_operation_active(
                session.bytes(),
                &source,
                &change,
            )?)
        }
        StorySessionCommand::HistorySetManyActive { source, change } => {
            encode(&crate::linked_stories::history::set_operations_active(
                session.bytes(),
                &source,
                &change,
            )?)
        }
        StorySessionCommand::HistoryPrepare { source } => encode(
            &crate::linked_stories::history::prepare_history(session.bytes(), &source)?,
        ),
        StorySessionCommand::HistoryJoin { source, histories } => encode(
            &crate::linked_stories::history::join_history(session.bytes(), &source, &histories)?,
        ),
        StorySessionCommand::HistoryEdit { source, edit } => encode(
            &crate::linked_stories::history::edit_history(session.bytes(), &source, &edit)?,
        ),
        StorySessionCommand::HistoryStyle { source, edit } => encode(
            &crate::linked_stories::history::edit_history_style(session.bytes(), &source, &edit)?,
        ),
        StorySessionCommand::HistoryStructure { source, edit } => {
            encode(&crate::linked_stories::history::edit_history_structure(
                session.bytes(),
                &source,
                &edit,
            )?)
        }
        StorySessionCommand::HistoryInlineStyle { source, edit } => {
            encode(&crate::linked_stories::history::edit_history_inline_style(
                session.bytes(),
                &source,
                &edit,
            )?)
        }
        StorySessionCommand::HistoryResolveInlineStyle { source, resolution } => encode(
            &crate::linked_stories::history::resolve_history_inline_style(
                session.bytes(),
                &source,
                &resolution,
            )?,
        ),
        StorySessionCommand::HistoryDelta { source, peer } => encode(
            &crate::linked_stories::history::history_delta(session.bytes(), &source, &peer)?,
        ),
        StorySessionCommand::HistoryPreview { source } => {
            encode(&session.preview_history(&source, cancel)?)
        }
        StorySessionCommand::HistoryCheckpoint { source, receipt } => {
            encode(&session.checkpoint_history(&source, &receipt, cancel)?)
        }
        StorySessionCommand::HistoryCompactionPlan { request } => encode(
            &crate::linked_stories::history::plan_history_compaction(session.bytes(), &request)?,
        ),
        StorySessionCommand::HistoryCompactionApply {
            request,
            approved_plan_sha256,
        } => encode(&session.compact_history(&request, &approved_plan_sha256, cancel)?),
    })
}

fn check_history_revision(session: &LinkedStorySession, base: &LinkedStoryRequest) -> Result<()> {
    if base.input_sha256 != session.revision_sha256() {
        return Err(WellfriendError::invalid_input(
            "text history base differs from the open PDF revision",
        ));
    }
    Ok(())
}

// JSON represents each byte as a decimal number. Four MiB of binary data fits
// below MAX_COMMAND_BYTES even at four characters per byte, leaving headroom.
// Rust's direct preparation API independently supports the 256 MiB font budget.
fn check_transport_font_size(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > 4 * 1024 * 1024 {
        return Err(WellfriendError::invalid_input(
            "session font input/output must be 1..=4 MiB",
        ));
    }
    Ok(())
}

fn check_dpi(dpi: u32) -> Result<()> {
    if dpi == 0 || dpi > 300 {
        return Err(WellfriendError::invalid_input(
            "story preview DPI must be in 1..=300",
        ));
    }
    Ok(())
}

/// Executes only when explicitly requested by a host. Uses the canonical
/// contract renderer, including its cancellation and document identity.
pub fn render_page_png(
    session: &LinkedStorySession,
    page: usize,
    dpi: u32,
    cancel: &CancelToken,
) -> Result<Vec<u8>> {
    cancel.check("story render")?;
    check_dpi(dpi)?;
    cancel.scope(|| {
        let engine = session.document();
        let v = engine.page_viewport(page, dpi)?;
        if u64::from(v.width_px) * u64::from(v.height_px) > 16_000_000 {
            return Err(WellfriendError::invalid_input(
                "story preview exceeds 16 million pixels",
            ));
        }
        let contract =
            engine.default_render_contract(page, dpi, crate::render::RenderMode::Compat)?;
        let png = engine.render_page_png_with_contract(&contract, cancel)?;
        cancel.check("story render publication")?;
        Ok(png)
    })
}
