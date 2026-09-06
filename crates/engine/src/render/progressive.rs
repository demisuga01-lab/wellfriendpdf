use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::engine::ContentEngine;
use crate::error::{Result, WellfriendError};
use crate::optional_content::OptionalContentContext;
use crate::render::{
    apply_render_invalidation_plan_json_to_cache, DisplayListStats, ExactnessPolicy,
    InvalidationResult, NativeDescriptor, PageRenderer, PixelBuffer, RenderContract,
    RenderDocumentCache, RenderMode, RenderPlan, RenderResourceBudget, RenderTile,
    RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION, WHITE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ProgressiveRenderState {
    Created,
    Preparing,
    Rendering,
    Paused,
    Completed,
    Cancelled,
    Failed,
    Closed,
}

impl ProgressiveRenderState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Preparing => "preparing",
            Self::Rendering => "rendering",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Closed => "closed",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProgressiveRenderToken {
    pub schema_version: u32,
    pub document_revision: u64,
    pub lifecycle_state: String,
    pub page_number: usize,
    pub dpi: u32,
    pub render_mode: String,
    pub tile_width: u32,
    pub tile_height: u32,
    pub page_width: u32,
    pub page_height: u32,
    pub next_tile_index: usize,
    pub total_tiles: usize,
    pub completed_tiles: usize,
    pub visibility_fingerprint: String,
    #[serde(default)]
    pub render_contract_fingerprint: String,
    #[serde(default)]
    pub publication_identity: String,
    pub viewport_hint: Option<RenderTile>,
    #[serde(default)]
    pub dirty_region: Option<RenderTile>,
    #[serde(default)]
    pub scheduler_generation: u64,
    #[serde(default)]
    pub tile_scheduler: Option<ProgressiveTileSchedulerReport>,
    pub resumable: bool,
    pub complete: bool,
}

pub const PROGRESSIVE_TILE_SCHEDULER_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressiveTileSchedulerReport {
    pub schema_version: u32,
    pub selection_mode: String,
    pub requested_tile_width: u32,
    pub requested_tile_height: u32,
    pub selected_tile_width: u32,
    pub selected_tile_height: u32,
    pub supported_tile_sizes: Vec<u32>,
    pub page_width: u32,
    pub page_height: u32,
    pub page_pixels: u64,
    pub max_temporary_bytes: u64,
    pub max_tile_pixels_by_budget: u64,
    pub render_mode: String,
    pub render_contract_fingerprint: String,
    pub execution_mode: String,
    pub backend_selection: String,
    pub print_profile: String,
    pub output_surface: String,
    pub base_target_tile_size: u32,
    pub selected_target_tile_size: u32,
    pub complexity_score: usize,
    pub complexity_tier: String,
    pub complexity_pressure_steps: u8,
    pub complexity_pressure_reasons: Vec<String>,
    pub display_operation_count: usize,
    pub hot_operation_count: usize,
    pub descriptor_arena_entries: usize,
    pub native_payload_batches: usize,
    pub compile_refusal_count: usize,
    pub image_operation_count: usize,
    pub clip_operation_count: usize,
    pub transparency_operation_count: usize,
    pub path_complexity_score: usize,
    pub selection_reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveRenderFallbackEvent {
    pub code: String,
    pub call_site: String,
    pub trigger: String,
    pub output: String,
    pub degradation: String,
    pub standard_available: bool,
    pub high_quality_available: bool,
    pub replacement: String,
    pub final_policy: String,
    pub tile: Option<RenderTile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressiveTilePriorityClass {
    Unhinted,
    Visible,
    AdjacentViewport,
    Background,
}

impl ProgressiveTilePriorityClass {
    fn sort_rank(self) -> u8 {
        match self {
            Self::Unhinted => 0,
            Self::Visible => 0,
            Self::AdjacentViewport => 1,
            Self::Background => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressiveViewerQueuePriority {
    CenterVisibleTile,
    VisibleTile,
    NearVisibleTile,
    AdjacentPagePreview,
    BackgroundPrefetch,
    Unhinted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProgressiveTilePublication {
    pub tile_index: usize,
    pub tile_order_index: usize,
    pub tile: RenderTile,
    pub priority_class: ProgressiveTilePriorityClass,
    pub publication_identity: String,
    pub tile_publication_identity: String,
    pub document_revision: u64,
    pub visibility_fingerprint: String,
    #[serde(default)]
    pub render_contract_fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveTilePublicationAcceptance {
    pub accepted: bool,
    pub reason: String,
    pub expected_publication_identity: String,
    pub supplied_publication_identity: String,
    pub expected_tile_publication_identity: String,
    pub supplied_tile_publication_identity: String,
    pub expected_document_revision: u64,
    pub supplied_document_revision: u64,
    pub expected_visibility_fingerprint: String,
    pub supplied_visibility_fingerprint: String,
    pub expected_render_contract_fingerprint: String,
    pub supplied_render_contract_fingerprint: String,
    pub tile_index: usize,
    pub tile_order_index: usize,
    pub tile: RenderTile,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveAdjacentPagePrefetch {
    pub queue_rank: usize,
    pub page_number: usize,
    pub relative_page_offset: i32,
    pub priority: ProgressiveViewerQueuePriority,
    pub prefetch_identity: String,
    pub source_publication_identity: String,
    pub document_revision: u64,
    pub visibility_fingerprint: String,
    pub render_contract_fingerprint: String,
    pub dpi: u32,
    pub render_mode: String,
    pub tile_width: u32,
    pub tile_height: u32,
    pub page_width: u32,
    pub page_height: u32,
    pub viewport_hint: Option<RenderTile>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveViewerQueueItem {
    pub queue_rank: usize,
    pub priority: ProgressiveViewerQueuePriority,
    pub page_number: usize,
    pub work_identity: String,
    pub publication_identity: String,
    pub document_revision: u64,
    pub visibility_fingerprint: String,
    pub render_contract_fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile_order_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile: Option<RenderTile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_page_offset: Option<i32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveViewerCallbackEvent {
    pub sequence: usize,
    pub callback: String,
    pub delivery_policy: String,
    pub publication_identity: String,
    pub work_identity: String,
    pub requires_publication_acceptance: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile_publication: Option<ProgressiveTilePublication>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjacent_page_prefetch: Option<ProgressiveAdjacentPagePrefetch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer_queue_item: Option<ProgressiveViewerQueueItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obsolete_publication: Option<ProgressiveObsoletePublication>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveViewerCallbackDispatchReport {
    pub lifecycle_state: String,
    pub publication_identity: String,
    pub callbacks_dispatched: usize,
    pub callbacks_suppressed: usize,
    pub suppression_reason: Option<String>,
    pub no_callback_after_terminal_state: bool,
    pub events: Vec<ProgressiveViewerCallbackEvent>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveViewerQueueExecutionItem {
    pub queue_rank: usize,
    pub page_number: usize,
    pub priority: ProgressiveViewerQueuePriority,
    pub work_identity: String,
    pub execution: String,
    pub result: String,
    pub viewer_queue_item: ProgressiveViewerQueueItem,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile_publication: Option<ProgressiveTilePublication>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveViewerQueueExecutionReport {
    pub lifecycle_state: String,
    pub publication_identity: String,
    pub requested_max_items: usize,
    pub attempted_queue_items: usize,
    pub rendered_current_page_tiles: usize,
    pub deferred_items: usize,
    pub suppressed_items: usize,
    pub terminal_suppressed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppression_reason: Option<String>,
    pub queue_before: ProgressiveRenderStepReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_step_report: Option<ProgressiveRenderStepReport>,
    pub queue_after: ProgressiveRenderStepReport,
    pub executed_items: Vec<ProgressiveViewerQueueExecutionItem>,
    pub deferred_queue_items: Vec<ProgressiveViewerQueueExecutionItem>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveAdjacentPagePrefetchExecutionReport {
    pub lifecycle_state: String,
    pub source_publication_identity: String,
    pub prefetch_identity: String,
    pub page_number: usize,
    pub relative_page_offset: i32,
    pub requested_max_tiles: usize,
    pub executed: bool,
    pub terminal_suppressed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppression_reason: Option<String>,
    pub adjacent_page_prefetch: ProgressiveAdjacentPagePrefetch,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_token: Option<ProgressiveRenderToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_step_report: Option<ProgressiveRenderStepReport>,
    pub warnings: Vec<String>,
}

pub struct ProgressiveAdjacentPagePrefetchExecution {
    pub job: Option<ProgressiveRenderJob>,
    pub report: ProgressiveAdjacentPagePrefetchExecutionReport,
}

pub trait ProgressiveViewerCallbackSink {
    fn progressive_viewer_callback(&mut self, event: &ProgressiveViewerCallbackEvent);
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveObsoletePublication {
    pub publication_identity: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile: Option<RenderTile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile_publication_identity: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveRenderStepReport {
    pub lifecycle_state: String,
    pub phase: String,
    pub completed_tiles: Vec<RenderTile>,
    pub publication_identity: String,
    pub obsolete_publications: Vec<ProgressiveObsoletePublication>,
    pub completed_tile_publications: Vec<ProgressiveTilePublication>,
    pub adjacent_page_prefetches: Vec<ProgressiveAdjacentPagePrefetch>,
    pub viewer_queue_preview: Vec<ProgressiveViewerQueueItem>,
    pub warnings: Vec<String>,
    pub fallback_events: Vec<String>,
    pub fallback_event_details: Vec<ProgressiveRenderFallbackEvent>,
    pub completed_units: usize,
    pub total_units: usize,
    pub rendered_this_step: usize,
    pub next_tile_index: usize,
    pub cancelled: bool,
    pub resume_possible: bool,
    pub memory_bytes_retained: usize,
    pub visibility_fingerprint: String,
    pub render_contract_fingerprint: String,
    pub tile_scheduler: ProgressiveTileSchedulerReport,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveRenderInvalidationReport {
    pub schema_version: u32,
    pub invalidation_plan_schema_version: String,
    pub applied: bool,
    pub invalidation: InvalidationResult,
    pub affected_current_page: bool,
    pub invalidated_tile_indices: Vec<usize>,
    pub invalidated_completed_tiles: Vec<RenderTile>,
    pub previous_publication_identity: String,
    pub current_publication_identity: String,
    pub scheduler_generation: u64,
    pub step_report: ProgressiveRenderStepReport,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveRenderContextRevisionReport {
    pub schema_version: u32,
    pub changed: bool,
    pub previous_publication_identity: String,
    pub current_publication_identity: String,
    pub previous_visibility_fingerprint: String,
    pub current_visibility_fingerprint: String,
    pub previous_render_contract_fingerprint: String,
    pub current_render_contract_fingerprint: String,
    pub scheduler_generation: u64,
    pub obsolete_publications: Vec<ProgressiveObsoletePublication>,
    pub step_report: ProgressiveRenderStepReport,
}

pub struct ProgressiveRenderJob {
    engine: ContentEngine,
    page_number: usize,
    dpi: u32,
    render_mode: RenderMode,
    base_contract: RenderContract,
    tile_width: u32,
    tile_height: u32,
    page_width: u32,
    page_height: u32,
    output_origin_x: u32,
    output_origin_y: u32,
    tiles: Vec<RenderTile>,
    tile_order: Vec<usize>,
    viewport_hint: Option<RenderTile>,
    dirty_region: Option<RenderTile>,
    adjacent_pages: Vec<ProgressiveAdjacentPageSeed>,
    scheduler_generation: u64,
    tile_scheduler: ProgressiveTileSchedulerReport,
    completed: Vec<Option<PixelBuffer>>,
    next_tile_index: usize,
    visibility_fingerprint: String,
    render_contract_fingerprint: String,
    document_cache: RenderDocumentCache,
    state: ProgressiveRenderState,
    warnings: Vec<String>,
    fallback_events: Vec<String>,
    fallback_event_details: Vec<ProgressiveRenderFallbackEvent>,
    obsolete_publications: Vec<ProgressiveObsoletePublication>,
    cancel_token: CancelToken,
    aborted: bool,
}

const ADAPTIVE_TILE_SIZES: [u32; 5] = [128, 192, 256, 384, 512];
const MAX_OBSOLETE_PUBLICATIONS: usize = 8;
const MAX_VIEWER_QUEUE_PREVIEW_ITEMS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProgressiveAdjacentPageSeed {
    page_number: usize,
    relative_page_offset: i32,
    page_width: u32,
    page_height: u32,
}

struct PreparedProgressiveRenderContract {
    page_number: usize,
    dpi: u32,
    render_mode: RenderMode,
    base_contract: RenderContract,
    tile_width: u32,
    tile_height: u32,
    page_width: u32,
    page_height: u32,
    output_origin_x: u32,
    output_origin_y: u32,
    tiles: Vec<RenderTile>,
    tile_order: Vec<usize>,
    adjacent_pages: Vec<ProgressiveAdjacentPageSeed>,
    tile_scheduler: ProgressiveTileSchedulerReport,
    visibility_fingerprint: String,
    render_contract_fingerprint: String,
}

fn prepare_progressive_render_contract(
    engine: &ContentEngine,
    contract: RenderContract,
    requested_tile_width: u32,
    requested_tile_height: u32,
    viewport_hint: Option<RenderTile>,
) -> Result<PreparedProgressiveRenderContract> {
    contract.validate()?;
    if contract.document_revision != engine.canonical_document().revision() {
        return Err(WellfriendError::invalid_input(
            "render contract belongs to a different document revision",
        ));
    }
    if contract.pixel_format != crate::render::PixelFormat::Rgba8
        || contract.alpha_mode != crate::render::AlphaMode::Premultiplied
        || contract.grayscale
        || contract.reverse_byte_order
    {
        return Err(WellfriendError::UnsupportedFeature(
            "progressive render sessions publish canonical premultiplied RGBA tiles; use caller-owned contract rendering for requested surface layout".to_string(),
        ));
    }
    let page_number = contract.page_number;
    let dpi = contract.dpi;
    let render_mode = contract.render_mode();
    let viewport = engine
        .page_viewport_for_box(page_number, dpi, contract.page_box)?
        .with_device_transform(crate::render::Transform2D::from_array(
            contract.transform.to_f64(),
        ));
    let full_tile = RenderTile::full(viewport.width_px, viewport.height_px);
    let output_region = progressive_contract_output_region(&contract, full_tile)?;
    let base_contract = contract.with_render_tile(output_region);
    let expected = engine.render_contract_for_tile_with_page_box(
        page_number,
        dpi,
        render_mode,
        output_region,
        base_contract.page_box,
    )?;
    let mut normalized = base_contract.clone();
    normalized.pixel_format = expected.pixel_format;
    normalized.alpha_mode = expected.alpha_mode;
    normalized.stride = expected.stride;
    normalized.grayscale = expected.grayscale;
    normalized.reverse_byte_order = expected.reverse_byte_order;
    let mut accepted_expected = expected;
    accepted_expected.print_profile = base_contract.print_profile;
    accepted_expected.annotations = base_contract.annotations;
    accepted_expected.forms = base_contract.forms;
    accepted_expected.execution_mode = base_contract.execution_mode;
    accepted_expected.halftone = base_contract.halftone;
    accepted_expected.background = base_contract.background;
    accepted_expected.transform = base_contract.transform;
    accepted_expected.resource_budget = base_contract.resource_budget;
    accepted_expected.exactness = base_contract.exactness;
    accepted_expected.determinism = base_contract.determinism;
    accepted_expected.rendering_intent = base_contract.rendering_intent;
    accepted_expected.color_management = base_contract.color_management;
    accepted_expected.backend = base_contract.backend;
    accepted_expected.optional_content = base_contract.optional_content.clone();
    accepted_expected.color_scheme = base_contract.color_scheme;
    accepted_expected.text_smoothing = base_contract.text_smoothing;
    accepted_expected.image_smoothing = base_contract.image_smoothing;
    accepted_expected.path_smoothing = base_contract.path_smoothing;
    accepted_expected.subpixel_text = base_contract.subpixel_text;
    if base_contract.overprint != crate::render::OverprintPolicy::PreserveSeparations {
        accepted_expected.overprint = base_contract.overprint;
    }
    if normalized != accepted_expected {
        let fields =
            crate::engine::unsupported_active_cpu_contract_fields(&normalized, &accepted_expected);
        return Err(WellfriendError::UnsupportedFeature(format!(
            "render contract fields are unsupported by the active CPU progressive renderer: {}",
            fields.join(", ")
        )));
    }
    let render_contract_fingerprint = base_contract.cache_fingerprint();
    let tile_scheduler = build_tile_scheduler_report(ProgressiveTileSchedulerInputs {
        engine,
        page_number,
        dpi,
        render_mode,
        requested_tile_width,
        requested_tile_height,
        page_width: output_region.width,
        page_height: output_region.height,
        contract: &base_contract,
    })?;
    let (tile_width, tile_height) = (
        tile_scheduler.selected_tile_width,
        tile_scheduler.selected_tile_height,
    );
    let output_end_x = output_region
        .x
        .checked_add(output_region.width)
        .ok_or_else(|| {
            WellfriendError::invalid_input("progressive contract output x range overflows")
        })?;
    let output_end_y = output_region
        .y
        .checked_add(output_region.height)
        .ok_or_else(|| {
            WellfriendError::invalid_input("progressive contract output y range overflows")
        })?;
    let mut tiles = Vec::new();
    let mut y = output_region.y;
    while y < output_end_y {
        let height = tile_height.min(output_end_y - y);
        let mut x = output_region.x;
        while x < output_end_x {
            let width = tile_width.min(output_end_x - x);
            tiles.push(RenderTile {
                x,
                y,
                width,
                height,
            });
            x += width;
        }
        y += height;
    }
    let total = tiles.len();
    let mut tile_order: Vec<usize> = (0..total).collect();
    if let Some(hint) = viewport_hint {
        tile_order.sort_by_key(|index| tile_priority_key(tiles[*index], hint));
    }
    let visibility_fingerprint = OptionalContentContext::from_document_for_state(
        engine.document(),
        &base_contract.optional_content.0,
    )
    .map_err(|err| {
        WellfriendError::UnsupportedFeature(format!(
            "render contract optional_content is unsupported: {err}"
        ))
    })?
    .visibility_fingerprint()
    .to_string();
    let adjacent_pages = adjacent_page_seeds(engine, page_number, dpi)?;
    Ok(PreparedProgressiveRenderContract {
        page_number,
        dpi,
        render_mode,
        base_contract,
        tile_width,
        tile_height,
        page_width: output_region.width,
        page_height: output_region.height,
        output_origin_x: output_region.x,
        output_origin_y: output_region.y,
        tiles,
        tile_order,
        adjacent_pages,
        tile_scheduler,
        visibility_fingerprint,
        render_contract_fingerprint,
    })
}

impl ProgressiveRenderJob {
    pub fn new(
        engine: ContentEngine,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile_width: u32,
        tile_height: u32,
    ) -> Result<Self> {
        Self::new_with_viewport_hint(
            engine,
            page_number,
            dpi,
            render_mode,
            tile_width,
            tile_height,
            None,
        )
    }

    pub fn new_with_viewport_hint(
        engine: ContentEngine,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile_width: u32,
        tile_height: u32,
        viewport_hint: Option<RenderTile>,
    ) -> Result<Self> {
        let contract = engine.default_render_contract(page_number, dpi, render_mode)?;
        Self::new_with_contract_and_viewport_hint(
            engine,
            contract,
            tile_width,
            tile_height,
            viewport_hint,
        )
    }

    pub fn new_with_contract(
        engine: ContentEngine,
        contract: RenderContract,
        tile_width: u32,
        tile_height: u32,
    ) -> Result<Self> {
        Self::new_with_contract_and_viewport_hint(engine, contract, tile_width, tile_height, None)
    }

    pub fn new_with_contract_and_viewport_hint(
        engine: ContentEngine,
        contract: RenderContract,
        tile_width: u32,
        tile_height: u32,
        viewport_hint: Option<RenderTile>,
    ) -> Result<Self> {
        let prepared = prepare_progressive_render_contract(
            &engine,
            contract,
            tile_width,
            tile_height,
            viewport_hint,
        )?;
        let total = prepared.tiles.len();
        Ok(Self {
            engine,
            page_number: prepared.page_number,
            dpi: prepared.dpi,
            render_mode: prepared.render_mode,
            base_contract: prepared.base_contract,
            tile_width: prepared.tile_width,
            tile_height: prepared.tile_height,
            page_width: prepared.page_width,
            page_height: prepared.page_height,
            output_origin_x: prepared.output_origin_x,
            output_origin_y: prepared.output_origin_y,
            tiles: prepared.tiles,
            tile_order: prepared.tile_order,
            viewport_hint,
            dirty_region: None,
            adjacent_pages: prepared.adjacent_pages,
            scheduler_generation: 0,
            tile_scheduler: prepared.tile_scheduler,
            completed: vec![None; total],
            next_tile_index: 0,
            visibility_fingerprint: prepared.visibility_fingerprint,
            render_contract_fingerprint: prepared.render_contract_fingerprint,
            document_cache: RenderDocumentCache::new(),
            state: ProgressiveRenderState::Created,
            warnings: Vec::new(),
            fallback_events: Vec::new(),
            fallback_event_details: Vec::new(),
            obsolete_publications: Vec::new(),
            cancel_token: CancelToken::new(),
            aborted: false,
        })
    }

    pub fn token(&self) -> ProgressiveRenderToken {
        ProgressiveRenderToken {
            schema_version: 1,
            document_revision: self.engine.canonical_document().revision().0,
            lifecycle_state: self.state.as_str().to_string(),
            page_number: self.page_number,
            dpi: self.dpi,
            render_mode: self.render_mode.as_str().to_string(),
            tile_width: self.tile_width,
            tile_height: self.tile_height,
            page_width: self.page_width,
            page_height: self.page_height,
            next_tile_index: self.next_tile_index,
            total_tiles: self.tiles.len(),
            completed_tiles: self.completed_count(),
            visibility_fingerprint: self.visibility_fingerprint.clone(),
            render_contract_fingerprint: self.render_contract_fingerprint.clone(),
            publication_identity: self.publication_identity(),
            viewport_hint: self.viewport_hint,
            dirty_region: self.dirty_region,
            scheduler_generation: self.scheduler_generation,
            tile_scheduler: Some(self.tile_scheduler.clone()),
            resumable: !self.aborted
                && matches!(
                    self.state,
                    ProgressiveRenderState::Created
                        | ProgressiveRenderState::Preparing
                        | ProgressiveRenderState::Rendering
                        | ProgressiveRenderState::Paused
                ),
            complete: self.is_complete(),
        }
    }

    pub fn validate_resume_token(&self, token: &ProgressiveRenderToken) -> Result<()> {
        if !token.resumable {
            return Err(WellfriendError::invalid_input(
                "progressive resume token is marked non-resumable",
            ));
        }
        let expected_mode = self.render_mode.as_str();
        let publication_identity = self.publication_identity();
        let mismatches = [
            (token.schema_version != 1).then_some("schema_version"),
            (token.document_revision != self.engine.canonical_document().revision().0)
                .then_some("document_revision"),
            matches!(
                token.lifecycle_state.as_str(),
                "cancelled" | "failed" | "closed"
            )
            .then_some("lifecycle_state"),
            (token.page_number != self.page_number).then_some("page_number"),
            (token.dpi != self.dpi).then_some("dpi"),
            (token.render_mode.as_str() != expected_mode).then_some("render_mode"),
            (token.tile_width != self.tile_width).then_some("tile_width"),
            (token.tile_height != self.tile_height).then_some("tile_height"),
            (token.page_width != self.page_width).then_some("page_width"),
            (token.page_height != self.page_height).then_some("page_height"),
            (token.next_tile_index != self.next_tile_index).then_some("next_tile_index"),
            (token.total_tiles != self.tiles.len()).then_some("total_tiles"),
            (token.completed_tiles != self.completed_count()).then_some("completed_tiles"),
            (token.visibility_fingerprint != self.visibility_fingerprint)
                .then_some("visibility_fingerprint"),
            (!token.render_contract_fingerprint.is_empty()
                && token.render_contract_fingerprint != self.render_contract_fingerprint)
                .then_some("render_contract_fingerprint"),
            (!token.publication_identity.is_empty()
                && token.publication_identity != publication_identity)
                .then_some("publication_identity"),
            (token.viewport_hint != self.viewport_hint).then_some("viewport_hint"),
            (token.dirty_region != self.dirty_region).then_some("dirty_region"),
            (token.scheduler_generation != self.scheduler_generation
                && !(token.scheduler_generation == 0 && self.scheduler_generation == 0))
                .then_some("scheduler_generation"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if mismatches.is_empty() {
            Ok(())
        } else {
            Err(WellfriendError::invalid_input(format!(
                "progressive resume token mismatch: {}",
                mismatches.join(", ")
            )))
        }
    }

    pub fn state(&self) -> ProgressiveRenderState {
        self.state
    }

    /// Return a clone of the session-owned cooperative cancellation source.
    ///
    /// Binding and server adapters keep this clone so a cancel request can be
    /// signalled while a long tile render is in progress.
    pub fn cancellation_token(&self) -> CancelToken {
        self.cancel_token.clone()
    }

    /// Signal the session-owned cancellation source without mutating retained
    /// tile state. The next in-flight or future `render_next` observes it and
    /// returns a resumable cancellation report.
    pub fn request_cancel(&self) {
        self.cancel_token.cancel();
    }

    /// Revise the visible-work hint and invalidate any already-published tile
    /// surfaces from the prior publication identity.
    ///
    /// This is a bounded obsolete-work policy for viewer-driven changes: callers
    /// can ask for a new visible region, receive a fresh publication identity,
    /// and reject tile publications associated with the identities listed in the
    /// next step report.
    pub fn revise_viewport_hint(
        &mut self,
        viewport_hint: Option<RenderTile>,
    ) -> Result<ProgressiveRenderStepReport> {
        match self.state {
            ProgressiveRenderState::Cancelled
            | ProgressiveRenderState::Failed
            | ProgressiveRenderState::Closed => {
                return Err(WellfriendError::invalid_input(
                    "terminal progressive render cannot revise its viewport hint",
                ));
            }
            ProgressiveRenderState::Completed => {
                return Err(WellfriendError::invalid_input(
                    "completed progressive render cannot revise its viewport hint",
                ));
            }
            ProgressiveRenderState::Created
            | ProgressiveRenderState::Preparing
            | ProgressiveRenderState::Rendering
            | ProgressiveRenderState::Paused => {}
        }

        if self.viewport_hint == viewport_hint {
            self.cancel_token.reset();
            return Ok(self.step_report(0, false));
        }

        let old_identity = self.publication_identity();
        let was_paused = matches!(self.state, ProgressiveRenderState::Paused);
        self.cancel_token.cancel();
        self.viewport_hint = viewport_hint;
        self.dirty_region = None;
        self.scheduler_generation = self.scheduler_generation.saturating_add(1);
        self.rebuild_tile_order();
        self.completed.fill(None);
        self.document_cache.clear();
        self.next_tile_index = 0;
        self.cancel_token.reset();
        self.record_obsolete_publication(old_identity, "viewport_hint_revised", None, None);
        self.warnings.push(
            "progressive viewport hint revised; prior tile publications are obsolete".to_string(),
        );
        self.state = if was_paused {
            ProgressiveRenderState::Paused
        } else {
            ProgressiveRenderState::Rendering
        };
        Ok(self.step_report(0, false))
    }

    /// Revise a dirty device-pixel region and reschedule only intersecting
    /// tiles while preserving already-rendered clean tile buffers.
    pub fn revise_dirty_region(
        &mut self,
        dirty_region: Option<RenderTile>,
    ) -> Result<ProgressiveRenderStepReport> {
        match self.state {
            ProgressiveRenderState::Cancelled
            | ProgressiveRenderState::Failed
            | ProgressiveRenderState::Closed => {
                return Err(WellfriendError::invalid_input(
                    "terminal progressive render cannot revise its dirty region",
                ));
            }
            ProgressiveRenderState::Created => {
                return Err(WellfriendError::invalid_input(
                    "created progressive render has no published tiles to reschedule",
                ));
            }
            ProgressiveRenderState::Completed
            | ProgressiveRenderState::Preparing
            | ProgressiveRenderState::Rendering
            | ProgressiveRenderState::Paused => {}
        }

        let Some(dirty_region) = dirty_region else {
            self.dirty_region = None;
            self.cancel_token.reset();
            return Ok(self.step_report(0, false));
        };
        if dirty_region.width == 0 || dirty_region.height == 0 {
            return Err(WellfriendError::invalid_input(
                "dirty region width and height must be greater than zero",
            ));
        }

        let dirty_tiles = self
            .tiles
            .iter()
            .enumerate()
            .filter_map(|(index, tile)| tile_intersects(*tile, dirty_region).then_some(index))
            .collect::<Vec<_>>();
        if dirty_tiles.is_empty() {
            self.dirty_region = Some(dirty_region);
            self.cancel_token.reset();
            return Ok(self.step_report(0, false));
        }

        let old_identity = self.publication_identity();
        let old_tile_order = self.tile_order.clone();
        let was_paused = matches!(self.state, ProgressiveRenderState::Paused);
        self.cancel_token.cancel();
        self.dirty_region = Some(dirty_region);
        self.scheduler_generation = self.scheduler_generation.saturating_add(1);

        for &tile_index in &dirty_tiles {
            let tile = self.tiles[tile_index];
            let old_order_index = old_tile_order
                .iter()
                .position(|index| *index == tile_index)
                .unwrap_or(tile_index);
            let old_tile_publication_identity = self.tile_publication_identity_for(
                &old_identity,
                tile_index,
                old_order_index,
                tile,
            );
            self.completed[tile_index] = None;
            self.record_obsolete_publication(
                old_identity.clone(),
                "dirty_region_revised",
                Some(tile),
                Some(old_tile_publication_identity),
            );
        }
        self.rebuild_tile_order();
        self.next_tile_index = self.first_incomplete_order_index();
        self.cancel_token.reset();
        self.warnings.push(format!(
            "progressive dirty region revised; {} tile publication(s) are obsolete",
            dirty_tiles.len()
        ));
        self.state = if was_paused {
            ProgressiveRenderState::Paused
        } else {
            ProgressiveRenderState::Rendering
        };
        Ok(self.step_report(0, false))
    }

    /// Replace the active progressive render contract and cancel all work that
    /// was scheduled for the previous contract.
    ///
    /// Unlike [`Self::revise_render_context`], this method updates rendering
    /// semantics, output region, tile grid, optional-content visibility, and
    /// scheduler state from the supplied schema-v1 contract. It is the path
    /// callers should use when a visible render-contract field changes.
    pub fn revise_render_contract(
        &mut self,
        contract: RenderContract,
    ) -> Result<ProgressiveRenderContextRevisionReport> {
        match self.state {
            ProgressiveRenderState::Cancelled
            | ProgressiveRenderState::Failed
            | ProgressiveRenderState::Closed => {
                return Err(WellfriendError::invalid_input(
                    "terminal progressive render cannot revise its render contract",
                ));
            }
            ProgressiveRenderState::Created
            | ProgressiveRenderState::Preparing
            | ProgressiveRenderState::Rendering
            | ProgressiveRenderState::Paused
            | ProgressiveRenderState::Completed => {}
        }

        let prepared = prepare_progressive_render_contract(
            &self.engine,
            contract,
            self.tile_scheduler.requested_tile_width,
            self.tile_scheduler.requested_tile_height,
            self.viewport_hint,
        )?;
        let previous_publication_identity = self.publication_identity();
        let previous_visibility_fingerprint = self.visibility_fingerprint.clone();
        let previous_render_contract_fingerprint = self.render_contract_fingerprint.clone();
        let changed = prepared.page_number != self.page_number
            || prepared.dpi != self.dpi
            || prepared.render_mode != self.render_mode
            || prepared.base_contract != self.base_contract
            || prepared.tile_width != self.tile_width
            || prepared.tile_height != self.tile_height
            || prepared.page_width != self.page_width
            || prepared.page_height != self.page_height
            || prepared.output_origin_x != self.output_origin_x
            || prepared.output_origin_y != self.output_origin_y
            || prepared.tiles != self.tiles
            || prepared.adjacent_pages != self.adjacent_pages
            || prepared.tile_scheduler != self.tile_scheduler
            || prepared.visibility_fingerprint != self.visibility_fingerprint
            || prepared.render_contract_fingerprint != self.render_contract_fingerprint;

        if changed {
            let was_paused = matches!(self.state, ProgressiveRenderState::Paused);
            let was_created = matches!(self.state, ProgressiveRenderState::Created);
            self.cancel_token.cancel();
            self.page_number = prepared.page_number;
            self.dpi = prepared.dpi;
            self.render_mode = prepared.render_mode;
            self.base_contract = prepared.base_contract;
            self.tile_width = prepared.tile_width;
            self.tile_height = prepared.tile_height;
            self.page_width = prepared.page_width;
            self.page_height = prepared.page_height;
            self.output_origin_x = prepared.output_origin_x;
            self.output_origin_y = prepared.output_origin_y;
            self.tiles = prepared.tiles;
            self.tile_order = prepared.tile_order;
            self.adjacent_pages = prepared.adjacent_pages;
            self.tile_scheduler = prepared.tile_scheduler;
            self.visibility_fingerprint = prepared.visibility_fingerprint;
            self.render_contract_fingerprint = prepared.render_contract_fingerprint;
            self.dirty_region = None;
            self.scheduler_generation = self.scheduler_generation.saturating_add(1);
            self.completed = vec![None; self.tiles.len()];
            self.document_cache.clear();
            self.next_tile_index = 0;
            self.record_obsolete_publication(
                previous_publication_identity.clone(),
                "render_contract_revised",
                None,
                None,
            );
            self.warnings.push(
                "progressive render contract revised; prior tile publications are obsolete"
                    .to_string(),
            );
            self.state = if was_paused {
                ProgressiveRenderState::Paused
            } else if was_created {
                ProgressiveRenderState::Created
            } else {
                ProgressiveRenderState::Rendering
            };
            self.cancel_token.reset();
        } else {
            self.cancel_token.reset();
        }

        let step_report = self.step_report(0, false);
        Ok(ProgressiveRenderContextRevisionReport {
            schema_version: 1,
            changed,
            previous_publication_identity,
            current_publication_identity: step_report.publication_identity.clone(),
            previous_visibility_fingerprint,
            current_visibility_fingerprint: self.visibility_fingerprint.clone(),
            previous_render_contract_fingerprint,
            current_render_contract_fingerprint: self.render_contract_fingerprint.clone(),
            scheduler_generation: self.scheduler_generation,
            obsolete_publications: self.obsolete_publications.clone(),
            step_report,
        })
    }

    /// Revise caller-visible render identity state and obsolete every retained
    /// tile publication from the prior context.
    ///
    /// This covers non-geometric render-contract or optional-content changes
    /// that a viewer can detect before presenting old surfaces. Rendering still
    /// uses this job's fixed page/dpi/mode geometry; callers that change those
    /// fields must create a new progressive session.
    pub fn revise_render_context(
        &mut self,
        render_contract_fingerprint: Option<String>,
        visibility_fingerprint: Option<String>,
    ) -> Result<ProgressiveRenderContextRevisionReport> {
        match self.state {
            ProgressiveRenderState::Cancelled
            | ProgressiveRenderState::Failed
            | ProgressiveRenderState::Closed => {
                return Err(WellfriendError::invalid_input(
                    "terminal progressive render cannot revise its render context",
                ));
            }
            ProgressiveRenderState::Created
            | ProgressiveRenderState::Preparing
            | ProgressiveRenderState::Rendering
            | ProgressiveRenderState::Paused
            | ProgressiveRenderState::Completed => {}
        }

        let next_render_contract_fingerprint = match render_contract_fingerprint {
            Some(value) => {
                validate_progressive_identity_fragment("render_contract_fingerprint", value)?
            }
            None => self.render_contract_fingerprint.clone(),
        };
        let next_visibility_fingerprint = match visibility_fingerprint {
            Some(value) => validate_progressive_identity_fragment("visibility_fingerprint", value)?,
            None => self.visibility_fingerprint.clone(),
        };

        let previous_publication_identity = self.publication_identity();
        let previous_visibility_fingerprint = self.visibility_fingerprint.clone();
        let previous_render_contract_fingerprint = self.render_contract_fingerprint.clone();
        let changed = next_render_contract_fingerprint != previous_render_contract_fingerprint
            || next_visibility_fingerprint != previous_visibility_fingerprint;

        if changed {
            let was_paused = matches!(self.state, ProgressiveRenderState::Paused);
            let was_created = matches!(self.state, ProgressiveRenderState::Created);
            self.cancel_token.cancel();
            self.render_contract_fingerprint = next_render_contract_fingerprint;
            self.visibility_fingerprint = next_visibility_fingerprint;
            self.dirty_region = None;
            self.scheduler_generation = self.scheduler_generation.saturating_add(1);
            self.rebuild_tile_order();
            self.completed.fill(None);
            self.document_cache.clear();
            self.next_tile_index = 0;
            self.record_obsolete_publication(
                previous_publication_identity.clone(),
                "render_context_revised",
                None,
                None,
            );
            self.warnings.push(
                "progressive render context revised; prior tile publications are obsolete"
                    .to_string(),
            );
            self.state = if was_paused {
                ProgressiveRenderState::Paused
            } else if was_created {
                ProgressiveRenderState::Created
            } else {
                ProgressiveRenderState::Rendering
            };
            self.cancel_token.reset();
        } else {
            self.cancel_token.reset();
        }

        let step_report = self.step_report(0, false);
        Ok(ProgressiveRenderContextRevisionReport {
            schema_version: 1,
            changed,
            previous_publication_identity,
            current_publication_identity: step_report.publication_identity.clone(),
            previous_visibility_fingerprint,
            current_visibility_fingerprint: self.visibility_fingerprint.clone(),
            previous_render_contract_fingerprint,
            current_render_contract_fingerprint: self.render_contract_fingerprint.clone(),
            scheduler_generation: self.scheduler_generation,
            obsolete_publications: self.obsolete_publications.clone(),
            step_report,
        })
    }

    pub fn pause(&mut self) -> Result<ProgressiveRenderToken> {
        match self.state {
            ProgressiveRenderState::Created
            | ProgressiveRenderState::Preparing
            | ProgressiveRenderState::Rendering => {
                self.state = ProgressiveRenderState::Paused;
                Ok(self.token())
            }
            ProgressiveRenderState::Paused => Ok(self.token()),
            ProgressiveRenderState::Completed => Err(WellfriendError::invalid_input(
                "completed progressive render cannot be paused",
            )),
            ProgressiveRenderState::Cancelled
            | ProgressiveRenderState::Failed
            | ProgressiveRenderState::Closed => Err(WellfriendError::invalid_input(
                "terminal progressive render cannot be paused",
            )),
        }
    }

    pub fn resume(&mut self, token: &ProgressiveRenderToken) -> Result<()> {
        self.validate_resume_token(token)?;
        match self.state {
            ProgressiveRenderState::Created | ProgressiveRenderState::Paused => {
                self.cancel_token.reset();
                self.state = if self.is_complete() {
                    ProgressiveRenderState::Completed
                } else {
                    ProgressiveRenderState::Rendering
                };
                Ok(())
            }
            ProgressiveRenderState::Rendering | ProgressiveRenderState::Preparing => Ok(()),
            ProgressiveRenderState::Completed => Ok(()),
            ProgressiveRenderState::Cancelled
            | ProgressiveRenderState::Failed
            | ProgressiveRenderState::Closed => Err(WellfriendError::invalid_input(
                "terminal progressive render cannot resume",
            )),
        }
    }

    /// Terminal cancellation releases temporary tile surfaces and mutable cache
    /// reservations while retaining the immutable source document.
    pub fn cancel(&mut self) {
        self.cancel_token.cancel();
        if matches!(self.state, ProgressiveRenderState::Closed) {
            return;
        }
        self.aborted = true;
        self.completed.fill(None);
        self.document_cache.clear();
        self.state = ProgressiveRenderState::Cancelled;
        self.warnings
            .push("progressive render cancelled; temporary tile surfaces released".to_string());
    }

    /// Release progressive temporary state. Calls after close are harmless.
    pub fn close(&mut self) {
        self.cancel_token.cancel();
        self.aborted = true;
        self.completed.fill(None);
        self.document_cache.clear();
        self.state = ProgressiveRenderState::Closed;
    }

    fn ensure_renderable(&self) -> Result<()> {
        match self.state {
            ProgressiveRenderState::Cancelled
            | ProgressiveRenderState::Failed
            | ProgressiveRenderState::Closed => Err(WellfriendError::invalid_input(
                "progressive render is in a terminal state",
            )),
            ProgressiveRenderState::Paused => Err(WellfriendError::invalid_input(
                "progressive render is paused; call resume with its token",
            )),
            ProgressiveRenderState::Created
            | ProgressiveRenderState::Preparing
            | ProgressiveRenderState::Rendering
            | ProgressiveRenderState::Completed => Ok(()),
        }
    }

    pub fn render_next(
        &mut self,
        max_tiles: usize,
        cancel: &CancelToken,
    ) -> Result<ProgressiveRenderStepReport> {
        self.ensure_renderable()?;
        if self.is_complete() {
            self.state = ProgressiveRenderState::Completed;
            return Ok(self.step_report(0, false));
        }

        self.state = ProgressiveRenderState::Preparing;
        let max_tiles = max_tiles.max(1);
        let render_cancel = CancelToken::linked_pair(cancel, &self.cancel_token);
        let mut rendered = 0;
        let mut cancelled = false;
        while self.next_tile_index < self.tiles.len() && rendered < max_tiles {
            if render_cancel.is_cancelled() {
                cancelled = true;
                self.state = ProgressiveRenderState::Paused;
                self.warnings.push(
                    "progressive work quantum observed cancellation and paused at a tile boundary"
                        .to_string(),
                );
                break;
            }
            self.state = ProgressiveRenderState::Rendering;
            let index = self.tile_order[self.next_tile_index];
            if self.completed[index].is_some() {
                self.next_tile_index += 1;
                continue;
            }
            let tile = self.tiles[index];
            let tile_contract = self.base_contract.with_render_tile(tile);
            let rendered_tile =
                PageRenderer::render_page_display_list_tile_cancellable_with_contract_and_cache(
                    &self.engine,
                    &tile_contract,
                    &render_cancel,
                    &mut self.document_cache,
                );
            let buffer = match rendered_tile {
                Ok(buffer) => buffer,
                Err(error) => {
                    if is_unsupported_display_list_retained_tile_refusal(&error) {
                        self.record_fallback_event(
                            unsupported_display_list_retained_tile_refusal_event(
                                tile,
                                tile_contract.exactness,
                            ),
                        );
                    }
                    self.state = ProgressiveRenderState::Failed;
                    self.warnings
                        .push(format!("progressive rendering failed: {error}"));
                    return Err(error);
                }
            };
            self.completed[index] = Some(buffer);
            self.next_tile_index += 1;
            rendered += 1;
        }
        if self.is_complete() {
            self.state = ProgressiveRenderState::Completed;
        } else if !cancelled {
            self.state = ProgressiveRenderState::Rendering;
        }
        Ok(self.step_report(rendered, cancelled))
    }

    /// Apply a source-edit render-invalidation plan to this session's retained
    /// render cache and mark any affected tile publications obsolete.
    pub fn apply_render_invalidation_plan_json(
        &mut self,
        plan_json: &str,
    ) -> Result<ProgressiveRenderInvalidationReport> {
        match self.state {
            ProgressiveRenderState::Cancelled
            | ProgressiveRenderState::Failed
            | ProgressiveRenderState::Closed => {
                return Err(WellfriendError::invalid_input(
                    "terminal progressive render cannot apply render invalidation",
                ));
            }
            ProgressiveRenderState::Created
            | ProgressiveRenderState::Preparing
            | ProgressiveRenderState::Rendering
            | ProgressiveRenderState::Paused
            | ProgressiveRenderState::Completed => {}
        }

        let previous_publication_identity = self.publication_identity();
        let was_paused = matches!(self.state, ProgressiveRenderState::Paused);
        let was_created = matches!(self.state, ProgressiveRenderState::Created);
        self.cancel_token.cancel();

        let invalidation =
            apply_render_invalidation_plan_json_to_cache(&mut self.document_cache, plan_json)?;
        let affected_current_page = self.invalidation_affects_current_page(&invalidation);
        let (invalidated_tile_indices, invalidated_completed_tiles) = self
            .invalidate_completed_tiles_for_result(&previous_publication_identity, &invalidation);

        if affected_current_page {
            self.scheduler_generation = self.scheduler_generation.saturating_add(1);
            self.rebuild_tile_order();
            self.next_tile_index = self.first_incomplete_order_index();
            self.warnings.push(format!(
                "render invalidation plan applied; {} retained tile publication(s) invalidated",
                invalidated_completed_tiles.len()
            ));
            self.state = if self.is_complete() {
                ProgressiveRenderState::Completed
            } else if was_paused {
                ProgressiveRenderState::Paused
            } else if was_created {
                ProgressiveRenderState::Created
            } else {
                ProgressiveRenderState::Rendering
            };
        } else {
            self.warnings.push(
                "render invalidation plan applied; current page retained surfaces unaffected"
                    .to_string(),
            );
        }

        self.cancel_token.reset();
        let step_report = self.step_report(0, false);
        Ok(ProgressiveRenderInvalidationReport {
            schema_version: 1,
            invalidation_plan_schema_version: RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION
                .to_string(),
            applied: true,
            invalidation,
            affected_current_page,
            invalidated_tile_indices,
            invalidated_completed_tiles,
            previous_publication_identity,
            current_publication_identity: step_report.publication_identity.clone(),
            scheduler_generation: self.scheduler_generation,
            step_report,
        })
    }

    fn invalidation_affects_current_page(&self, invalidation: &InvalidationResult) -> bool {
        invalidation.cache_must_reset
            || invalidation.invalidated_pages.contains(&self.page_number)
            || invalidation
                .invalidated_tiles
                .iter()
                .any(|(page, _tile)| *page == self.page_number)
    }

    fn invalidate_completed_tiles_for_result(
        &mut self,
        previous_publication_identity: &str,
        invalidation: &InvalidationResult,
    ) -> (Vec<usize>, Vec<RenderTile>) {
        if !self.invalidation_affects_current_page(invalidation) {
            return (Vec::new(), Vec::new());
        }

        let invalidated_tiles = invalidation
            .invalidated_tiles
            .iter()
            .filter_map(|(page, tile)| (*page == self.page_number).then_some(*tile))
            .collect::<Vec<_>>();
        let page_or_cache_reset = invalidation.cache_must_reset
            || (invalidation.invalidated_pages.contains(&self.page_number)
                && invalidated_tiles.is_empty());
        let old_tile_order = self.tile_order.clone();
        let mut invalidated_tile_indices = Vec::new();
        let mut invalidated_completed_tiles = Vec::new();

        for tile_index in 0..self.tiles.len() {
            if self.completed[tile_index].is_none() {
                continue;
            }
            let tile = self.tiles[tile_index];
            let tile_affected = page_or_cache_reset
                || invalidated_tiles
                    .iter()
                    .any(|invalidated_tile| tile_intersects(tile, *invalidated_tile));
            if !tile_affected {
                continue;
            }

            let old_order_index = old_tile_order
                .iter()
                .position(|index| *index == tile_index)
                .unwrap_or(tile_index);
            let tile_publication_identity = self.tile_publication_identity_for(
                previous_publication_identity,
                tile_index,
                old_order_index,
                tile,
            );
            self.completed[tile_index] = None;
            self.record_obsolete_publication(
                previous_publication_identity.to_string(),
                if page_or_cache_reset {
                    "render_invalidation_page_or_cache_reset"
                } else {
                    "render_invalidation_exact_tile"
                },
                Some(tile),
                Some(tile_publication_identity),
            );
            invalidated_tile_indices.push(tile_index);
            invalidated_completed_tiles.push(tile);
        }

        if page_or_cache_reset && invalidated_tile_indices.is_empty() {
            self.record_obsolete_publication(
                previous_publication_identity.to_string(),
                "render_invalidation_page_or_cache_reset",
                None,
                None,
            );
        }

        (invalidated_tile_indices, invalidated_completed_tiles)
    }

    fn step_report(
        &self,
        rendered_this_step: usize,
        cancelled: bool,
    ) -> ProgressiveRenderStepReport {
        let publication_identity = self.publication_identity();
        let completed_tiles = self
            .tiles
            .iter()
            .copied()
            .zip(self.completed.iter())
            .filter_map(|(tile, buffer)| buffer.is_some().then_some(tile))
            .collect();
        let completed_tile_publications = self.completed_tile_publications(&publication_identity);
        let adjacent_page_prefetches = self.adjacent_page_prefetches(&publication_identity);
        let viewer_queue_preview =
            self.viewer_queue_preview(&publication_identity, &adjacent_page_prefetches);
        ProgressiveRenderStepReport {
            lifecycle_state: self.state.as_str().to_string(),
            phase: if self.is_complete() {
                "complete".to_string()
            } else if cancelled {
                "cancelled_resumable".to_string()
            } else {
                "rendering_tiles".to_string()
            },
            completed_tiles,
            publication_identity,
            obsolete_publications: self.obsolete_publications.clone(),
            completed_tile_publications,
            adjacent_page_prefetches,
            viewer_queue_preview,
            warnings: self.warnings.clone(),
            fallback_events: self.fallback_events.clone(),
            fallback_event_details: self.fallback_event_details.clone(),
            completed_units: self.completed_count(),
            total_units: self.tiles.len(),
            rendered_this_step,
            next_tile_index: self.next_tile_index,
            cancelled,
            resume_possible: !self.aborted
                && matches!(
                    self.state,
                    ProgressiveRenderState::Created
                        | ProgressiveRenderState::Preparing
                        | ProgressiveRenderState::Rendering
                        | ProgressiveRenderState::Paused
                ),
            memory_bytes_retained: self.memory_bytes_retained(),
            visibility_fingerprint: self.visibility_fingerprint.clone(),
            render_contract_fingerprint: self.render_contract_fingerprint.clone(),
            tile_scheduler: self.tile_scheduler.clone(),
        }
    }

    fn record_fallback_event(&mut self, event: ProgressiveRenderFallbackEvent) {
        self.fallback_events.push(event.code.clone());
        self.fallback_event_details.push(event);
    }

    fn record_obsolete_publication(
        &mut self,
        publication_identity: String,
        reason: &str,
        tile: Option<RenderTile>,
        tile_publication_identity: Option<String>,
    ) {
        if self.obsolete_publications.len() >= MAX_OBSOLETE_PUBLICATIONS {
            self.obsolete_publications.remove(0);
        }
        self.obsolete_publications
            .push(ProgressiveObsoletePublication {
                publication_identity,
                reason: reason.to_string(),
                tile,
                tile_publication_identity,
            });
    }

    fn rebuild_tile_order(&mut self) {
        self.tile_order = (0..self.tiles.len()).collect();
        let viewport_hint = self.viewport_hint;
        let dirty_region = self.dirty_region;
        let completed = &self.completed;
        self.tile_order.sort_by_key(|index| {
            let tile = self.tiles[*index];
            tile_schedule_key(
                tile,
                *index,
                viewport_hint,
                dirty_region,
                completed[*index].is_some(),
            )
        });
    }

    pub fn publication_identity(&self) -> String {
        format!(
            "wf-progressive:v1:doc={}:rev={:016x}:page={}:dpi={}:mode={}:tile_grid={}x{}:page_px={}x{}:contract={}:visibility={}:viewport={}:dirty={}:generation={}",
            self.engine.canonical_document().fingerprint_hex(),
            self.engine.canonical_document().revision().0,
            self.page_number,
            self.dpi,
            self.render_mode.as_str(),
            self.tile_width,
            self.tile_height,
            self.page_width,
            self.page_height,
            self.render_contract_fingerprint,
            self.visibility_fingerprint,
            format_viewport_identity(self.viewport_hint),
            format_viewport_identity(self.dirty_region),
            self.scheduler_generation
        )
    }

    pub fn evaluate_tile_publication(
        &self,
        publication: &ProgressiveTilePublication,
    ) -> ProgressiveTilePublicationAcceptance {
        let expected_publication_identity = self.publication_identity();
        let expected_document_revision = self.engine.canonical_document().revision().0;
        let expected_visibility_fingerprint = self.visibility_fingerprint.clone();
        let expected_render_contract_fingerprint = self.render_contract_fingerprint.clone();
        let expected_tile_publication_identity = self.tile_publication_identity_for(
            &expected_publication_identity,
            publication.tile_index,
            publication.tile_order_index,
            publication.tile,
        );

        let reason = if self.obsolete_publications.iter().any(|obsolete| {
            obsolete.publication_identity == publication.publication_identity
                && obsolete.tile_publication_identity.as_deref()
                    == Some(publication.tile_publication_identity.as_str())
        }) {
            "obsolete_tile_publication"
        } else if self.obsolete_publications.iter().any(|obsolete| {
            obsolete.publication_identity == publication.publication_identity
                && obsolete.tile.is_none()
        }) {
            "obsolete_publication"
        } else if publication.publication_identity != expected_publication_identity {
            "publication_identity_mismatch"
        } else if publication.document_revision != expected_document_revision {
            "document_revision_mismatch"
        } else if publication.visibility_fingerprint != expected_visibility_fingerprint {
            "visibility_fingerprint_mismatch"
        } else if !publication.render_contract_fingerprint.is_empty()
            && publication.render_contract_fingerprint != expected_render_contract_fingerprint
        {
            "render_contract_fingerprint_mismatch"
        } else if self.tiles.get(publication.tile_index).copied() != Some(publication.tile) {
            "tile_identity_mismatch"
        } else if self.tile_order.get(publication.tile_order_index).copied()
            != Some(publication.tile_index)
        {
            "tile_order_mismatch"
        } else if publication.tile_publication_identity != expected_tile_publication_identity {
            "tile_publication_identity_mismatch"
        } else if self
            .completed
            .get(publication.tile_index)
            .and_then(Option::as_ref)
            .is_none()
        {
            "tile_not_currently_retained"
        } else {
            "current"
        };

        ProgressiveTilePublicationAcceptance {
            accepted: reason == "current",
            reason: reason.to_string(),
            expected_publication_identity,
            supplied_publication_identity: publication.publication_identity.clone(),
            expected_tile_publication_identity,
            supplied_tile_publication_identity: publication.tile_publication_identity.clone(),
            expected_document_revision,
            supplied_document_revision: publication.document_revision,
            expected_visibility_fingerprint,
            supplied_visibility_fingerprint: publication.visibility_fingerprint.clone(),
            expected_render_contract_fingerprint,
            supplied_render_contract_fingerprint: publication.render_contract_fingerprint.clone(),
            tile_index: publication.tile_index,
            tile_order_index: publication.tile_order_index,
            tile: publication.tile,
        }
    }

    pub fn viewer_queue_report(&self) -> ProgressiveRenderStepReport {
        self.step_report(0, false)
    }

    pub fn execute_viewer_queue(
        &mut self,
        max_items: usize,
        cancel: &CancelToken,
    ) -> Result<ProgressiveViewerQueueExecutionReport> {
        let requested_max_items = max_items.max(1);
        let queue_before = self.viewer_queue_report();
        let selected_items = queue_before
            .viewer_queue_preview
            .iter()
            .take(requested_max_items)
            .cloned()
            .collect::<Vec<_>>();
        let publication_identity = queue_before.publication_identity.clone();
        let terminal = matches!(
            self.state,
            ProgressiveRenderState::Cancelled
                | ProgressiveRenderState::Failed
                | ProgressiveRenderState::Closed
        );
        if terminal {
            let deferred_queue_items = selected_items
                .iter()
                .cloned()
                .map(|item| viewer_queue_execution_item(item, "suppressed", None))
                .collect::<Vec<_>>();
            return Ok(ProgressiveViewerQueueExecutionReport {
                lifecycle_state: self.state.as_str().to_string(),
                publication_identity,
                requested_max_items,
                attempted_queue_items: selected_items.len(),
                rendered_current_page_tiles: 0,
                deferred_items: 0,
                suppressed_items: selected_items.len(),
                terminal_suppressed: true,
                suppression_reason: Some(format!("terminal_state_{}", self.state.as_str())),
                queue_after: queue_before.clone(),
                queue_before,
                render_step_report: None,
                executed_items: Vec::new(),
                deferred_queue_items,
                warnings: vec![
                    "viewer queue execution suppressed because the progressive session is terminal"
                        .to_string(),
                ],
            });
        }

        let current_page_work = selected_items
            .iter()
            .filter(|item| item.tile.is_some())
            .count();
        let before_publications = queue_before
            .completed_tile_publications
            .iter()
            .map(|publication| publication.tile_publication_identity.clone())
            .collect::<HashSet<_>>();
        let render_step_report = if current_page_work > 0 {
            Some(self.render_next(current_page_work, cancel)?)
        } else {
            None
        };
        let queue_after = self.viewer_queue_report();
        let new_publications = render_step_report
            .as_ref()
            .map(|report| {
                report
                    .completed_tile_publications
                    .iter()
                    .filter(|publication| {
                        !before_publications.contains(&publication.tile_publication_identity)
                    })
                    .map(|publication| {
                        (
                            publication.tile_publication_identity.clone(),
                            publication.clone(),
                        )
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();

        let mut executed_items = Vec::new();
        let mut deferred_queue_items = Vec::new();
        for item in selected_items {
            if item.tile.is_none() {
                deferred_queue_items.push(viewer_queue_execution_item(
                    item,
                    "deferred_adjacent_page_prefetch_requires_page_session",
                    None,
                ));
                continue;
            }

            if let Some(publication) = new_publications.get(&item.work_identity).cloned() {
                executed_items.push(viewer_queue_execution_item(
                    item,
                    "rendered_current_page_tile",
                    Some(publication),
                ));
            } else {
                deferred_queue_items.push(viewer_queue_execution_item(
                    item,
                    "not_rendered_this_quantum",
                    None,
                ));
            }
        }

        let mut warnings = render_step_report
            .as_ref()
            .map(|report| report.warnings.clone())
            .unwrap_or_default();
        if current_page_work == 0 && !queue_before.viewer_queue_preview.is_empty() {
            warnings.push(
                "viewer queue execution found only external prefetch work in the selected quantum"
                    .to_string(),
            );
        }
        if deferred_queue_items
            .iter()
            .any(|item| item.result == "deferred_adjacent_page_prefetch_requires_page_session")
        {
            warnings.push(
                "adjacent-page prefetch items are reported for the caller to execute with a page-owned progressive session"
                    .to_string(),
            );
        }

        Ok(ProgressiveViewerQueueExecutionReport {
            lifecycle_state: queue_after.lifecycle_state.clone(),
            publication_identity,
            requested_max_items,
            attempted_queue_items: executed_items.len() + deferred_queue_items.len(),
            rendered_current_page_tiles: render_step_report
                .as_ref()
                .map(|report| report.rendered_this_step)
                .unwrap_or(0),
            deferred_items: deferred_queue_items.len(),
            suppressed_items: 0,
            terminal_suppressed: false,
            suppression_reason: None,
            queue_before,
            render_step_report,
            queue_after,
            executed_items,
            deferred_queue_items,
            warnings,
        })
    }

    pub fn execute_adjacent_page_prefetch(
        &self,
        prefetch_identity: &str,
        max_tiles: usize,
        cancel: &CancelToken,
    ) -> Result<ProgressiveAdjacentPagePrefetchExecution> {
        let requested_max_tiles = max_tiles.max(1);
        let source_publication_identity = self.publication_identity();
        let adjacent_page_prefetches = self.adjacent_page_prefetches(&source_publication_identity);
        let prefetch = adjacent_page_prefetches
            .into_iter()
            .find(|prefetch| prefetch.prefetch_identity == prefetch_identity)
            .ok_or_else(|| {
                WellfriendError::invalid_input(
                    "adjacent page prefetch identity is not current for this progressive session",
                )
            })?;

        let terminal = matches!(
            self.state,
            ProgressiveRenderState::Cancelled
                | ProgressiveRenderState::Failed
                | ProgressiveRenderState::Closed
        );
        if terminal {
            return Ok(ProgressiveAdjacentPagePrefetchExecution {
                job: None,
                report: ProgressiveAdjacentPagePrefetchExecutionReport {
                    lifecycle_state: self.state.as_str().to_string(),
                    source_publication_identity,
                    prefetch_identity: prefetch.prefetch_identity.clone(),
                    page_number: prefetch.page_number,
                    relative_page_offset: prefetch.relative_page_offset,
                    requested_max_tiles,
                    executed: false,
                    terminal_suppressed: true,
                    suppression_reason: Some(format!("terminal_state_{}", self.state.as_str())),
                    adjacent_page_prefetch: prefetch,
                    child_token: None,
                    render_step_report: None,
                    warnings: vec![
                        "adjacent-page prefetch suppressed because the source progressive session is terminal"
                            .to_string(),
                    ],
                },
            });
        }

        let mut child = ProgressiveRenderJob::new_with_viewport_hint(
            self.engine.clone(),
            prefetch.page_number,
            self.dpi,
            self.render_mode,
            self.tile_width,
            self.tile_height,
            prefetch.viewport_hint,
        )?;
        let step_report = child.render_next(requested_max_tiles, cancel)?;
        let child_token = child.token();
        let mut warnings = step_report.warnings.clone();
        warnings.push(
            "adjacent-page prefetch executed in a page-owned progressive child job".to_string(),
        );

        Ok(ProgressiveAdjacentPagePrefetchExecution {
            job: Some(child),
            report: ProgressiveAdjacentPagePrefetchExecutionReport {
                lifecycle_state: self.state.as_str().to_string(),
                source_publication_identity,
                prefetch_identity: prefetch.prefetch_identity.clone(),
                page_number: prefetch.page_number,
                relative_page_offset: prefetch.relative_page_offset,
                requested_max_tiles,
                executed: true,
                terminal_suppressed: false,
                suppression_reason: None,
                adjacent_page_prefetch: prefetch,
                child_token: Some(child_token),
                render_step_report: Some(step_report),
                warnings,
            },
        })
    }

    pub fn viewer_callback_dispatch_report(&self) -> ProgressiveViewerCallbackDispatchReport {
        let publication_identity = self.publication_identity();
        let terminal = matches!(
            self.state,
            ProgressiveRenderState::Cancelled
                | ProgressiveRenderState::Failed
                | ProgressiveRenderState::Closed
        );
        if terminal {
            return ProgressiveViewerCallbackDispatchReport {
                lifecycle_state: self.state.as_str().to_string(),
                publication_identity,
                callbacks_dispatched: 0,
                callbacks_suppressed: self.completed_count()
                    + self.obsolete_publications.len()
                    + self.adjacent_pages.len()
                    + self.tiles.len().saturating_sub(self.next_tile_index),
                suppression_reason: Some(format!("terminal_state_{}", self.state.as_str())),
                no_callback_after_terminal_state: true,
                events: Vec::new(),
            };
        }

        let report = self.step_report(0, false);
        let mut events = Vec::new();
        for obsolete in report.obsolete_publications {
            let sequence = events.len();
            events.push(ProgressiveViewerCallbackEvent {
                sequence,
                callback: "publication_obsolete".to_string(),
                delivery_policy: "deliver_before_any_new_publication_for_stale_suppression"
                    .to_string(),
                publication_identity: obsolete.publication_identity.clone(),
                work_identity: obsolete
                    .tile_publication_identity
                    .clone()
                    .unwrap_or_else(|| obsolete.publication_identity.clone()),
                requires_publication_acceptance: false,
                tile_publication: None,
                adjacent_page_prefetch: None,
                viewer_queue_item: None,
                obsolete_publication: Some(obsolete),
            });
        }
        for publication in report.completed_tile_publications {
            let sequence = events.len();
            events.push(ProgressiveViewerCallbackEvent {
                sequence,
                callback: "tile_publication_ready".to_string(),
                delivery_policy: "caller_must_evaluate_tile_publication_before_presenting_surface"
                    .to_string(),
                publication_identity: publication.publication_identity.clone(),
                work_identity: publication.tile_publication_identity.clone(),
                requires_publication_acceptance: true,
                tile_publication: Some(publication),
                adjacent_page_prefetch: None,
                viewer_queue_item: None,
                obsolete_publication: None,
            });
        }
        for prefetch in report.adjacent_page_prefetches {
            let sequence = events.len();
            events.push(ProgressiveViewerCallbackEvent {
                sequence,
                callback: "adjacent_page_prefetch_ready".to_string(),
                delivery_policy: "schedule_after_visible_and_near_visible_work".to_string(),
                publication_identity: prefetch.source_publication_identity.clone(),
                work_identity: prefetch.prefetch_identity.clone(),
                requires_publication_acceptance: false,
                tile_publication: None,
                adjacent_page_prefetch: Some(prefetch),
                viewer_queue_item: None,
                obsolete_publication: None,
            });
        }
        for item in report.viewer_queue_preview {
            let sequence = events.len();
            events.push(ProgressiveViewerCallbackEvent {
                sequence,
                callback: "viewer_queue_item_scheduled".to_string(),
                delivery_policy: "deliver_in_queue_rank_order_without_rendering".to_string(),
                publication_identity: item.publication_identity.clone(),
                work_identity: item.work_identity.clone(),
                requires_publication_acceptance: false,
                tile_publication: None,
                adjacent_page_prefetch: None,
                viewer_queue_item: Some(item),
                obsolete_publication: None,
            });
        }

        ProgressiveViewerCallbackDispatchReport {
            lifecycle_state: self.state.as_str().to_string(),
            publication_identity,
            callbacks_dispatched: events.len(),
            callbacks_suppressed: 0,
            suppression_reason: None,
            no_callback_after_terminal_state: false,
            events,
        }
    }

    pub fn dispatch_viewer_callbacks<S: ProgressiveViewerCallbackSink>(
        &self,
        sink: &mut S,
    ) -> ProgressiveViewerCallbackDispatchReport {
        let report = self.viewer_callback_dispatch_report();
        for event in &report.events {
            sink.progressive_viewer_callback(event);
        }
        report
    }

    fn completed_tile_publications(
        &self,
        publication_identity: &str,
    ) -> Vec<ProgressiveTilePublication> {
        let mut tile_order_positions = vec![usize::MAX; self.tile_order.len()];
        for (order_index, tile_index) in self.tile_order.iter().copied().enumerate() {
            if let Some(position) = tile_order_positions.get_mut(tile_index) {
                *position = order_index;
            }
        }
        let document_revision = self.engine.canonical_document().revision().0;
        self.completed
            .iter()
            .enumerate()
            .filter_map(|(tile_index, buffer)| {
                buffer.as_ref()?;
                let tile = self.tiles[tile_index];
                let tile_order_index = tile_order_positions
                    .get(tile_index)
                    .copied()
                    .filter(|position| *position != usize::MAX)
                    .unwrap_or(tile_index);
                let priority_class = self
                    .viewport_hint
                    .map(|hint| tile_priority_class(tile, hint))
                    .unwrap_or(ProgressiveTilePriorityClass::Unhinted);
                Some(ProgressiveTilePublication {
                    tile_index,
                    tile_order_index,
                    tile,
                    priority_class,
                    publication_identity: publication_identity.to_string(),
                    tile_publication_identity: self.tile_publication_identity_for(
                        publication_identity,
                        tile_index,
                        tile_order_index,
                        tile,
                    ),
                    document_revision,
                    visibility_fingerprint: self.visibility_fingerprint.clone(),
                    render_contract_fingerprint: self.render_contract_fingerprint.clone(),
                })
            })
            .collect()
    }

    fn tile_publication_identity_for(
        &self,
        publication_identity: &str,
        tile_index: usize,
        tile_order_index: usize,
        tile: RenderTile,
    ) -> String {
        format!(
            "{}:tile_index={}:tile_order_index={}:tile={}",
            publication_identity,
            tile_index,
            tile_order_index,
            format_tile_identity(tile)
        )
    }

    fn adjacent_page_prefetches(
        &self,
        publication_identity: &str,
    ) -> Vec<ProgressiveAdjacentPagePrefetch> {
        let document_revision = self.engine.canonical_document().revision().0;
        self.adjacent_pages
            .iter()
            .enumerate()
            .map(|(queue_rank, seed)| {
                let prefetch_identity = self.adjacent_page_prefetch_identity_for(
                    publication_identity,
                    seed.page_number,
                    seed.relative_page_offset,
                );
                ProgressiveAdjacentPagePrefetch {
                    queue_rank,
                    page_number: seed.page_number,
                    relative_page_offset: seed.relative_page_offset,
                    priority: ProgressiveViewerQueuePriority::AdjacentPagePreview,
                    prefetch_identity,
                    source_publication_identity: publication_identity.to_string(),
                    document_revision,
                    visibility_fingerprint: self.visibility_fingerprint.clone(),
                    render_contract_fingerprint: self.render_contract_fingerprint.clone(),
                    dpi: self.dpi,
                    render_mode: self.render_mode.as_str().to_string(),
                    tile_width: self.tile_width,
                    tile_height: self.tile_height,
                    page_width: seed.page_width,
                    page_height: seed.page_height,
                    viewport_hint: self.viewport_hint,
                }
            })
            .collect()
    }

    fn adjacent_page_prefetch_identity_for(
        &self,
        publication_identity: &str,
        page_number: usize,
        relative_page_offset: i32,
    ) -> String {
        format!(
            "{}:adjacent_page={}:offset={}",
            publication_identity, page_number, relative_page_offset
        )
    }

    fn viewer_queue_preview(
        &self,
        publication_identity: &str,
        adjacent_page_prefetches: &[ProgressiveAdjacentPagePrefetch],
    ) -> Vec<ProgressiveViewerQueueItem> {
        let mut tile_order_positions = vec![usize::MAX; self.tile_order.len()];
        for (order_index, tile_index) in self.tile_order.iter().copied().enumerate() {
            if let Some(position) = tile_order_positions.get_mut(tile_index) {
                *position = order_index;
            }
        }
        let mut visible_or_near = Vec::new();
        let mut background = Vec::new();
        let document_revision = self.engine.canonical_document().revision().0;
        let viewport_hint = self.viewport_hint;
        for tile_index in self.tile_order.iter().copied() {
            if self
                .completed
                .get(tile_index)
                .and_then(Option::as_ref)
                .is_some()
            {
                continue;
            }
            let tile = self.tiles[tile_index];
            let tile_order_index = tile_order_positions
                .get(tile_index)
                .copied()
                .filter(|position| *position != usize::MAX)
                .unwrap_or(tile_index);
            let priority = viewer_queue_priority(tile, viewport_hint);
            let item = ProgressiveViewerQueueItem {
                queue_rank: 0,
                priority,
                page_number: self.page_number,
                work_identity: self.tile_publication_identity_for(
                    publication_identity,
                    tile_index,
                    tile_order_index,
                    tile,
                ),
                publication_identity: publication_identity.to_string(),
                document_revision,
                visibility_fingerprint: self.visibility_fingerprint.clone(),
                render_contract_fingerprint: self.render_contract_fingerprint.clone(),
                tile_index: Some(tile_index),
                tile_order_index: Some(tile_order_index),
                tile: Some(tile),
                relative_page_offset: None,
            };
            match priority {
                ProgressiveViewerQueuePriority::BackgroundPrefetch => background.push(item),
                _ => visible_or_near.push(item),
            }
        }
        let reserved_for_adjacent = adjacent_page_prefetches
            .len()
            .min(MAX_VIEWER_QUEUE_PREVIEW_ITEMS);
        let reserved_for_background = usize::from(!background.is_empty());
        let current_page_capacity = MAX_VIEWER_QUEUE_PREVIEW_ITEMS
            .saturating_sub(reserved_for_adjacent + reserved_for_background);
        let mut items = visible_or_near
            .into_iter()
            .take(current_page_capacity)
            .collect::<Vec<_>>();
        if items.len() < MAX_VIEWER_QUEUE_PREVIEW_ITEMS {
            for prefetch in adjacent_page_prefetches {
                if items.len() >= MAX_VIEWER_QUEUE_PREVIEW_ITEMS {
                    break;
                }
                items.push(ProgressiveViewerQueueItem {
                    queue_rank: items.len(),
                    priority: prefetch.priority,
                    page_number: prefetch.page_number,
                    work_identity: prefetch.prefetch_identity.clone(),
                    publication_identity: publication_identity.to_string(),
                    document_revision: prefetch.document_revision,
                    visibility_fingerprint: prefetch.visibility_fingerprint.clone(),
                    render_contract_fingerprint: prefetch.render_contract_fingerprint.clone(),
                    tile_index: None,
                    tile_order_index: None,
                    tile: None,
                    relative_page_offset: Some(prefetch.relative_page_offset),
                });
            }
        }
        if items.len() < MAX_VIEWER_QUEUE_PREVIEW_ITEMS {
            items.extend(
                background
                    .into_iter()
                    .take(MAX_VIEWER_QUEUE_PREVIEW_ITEMS - items.len()),
            );
        }
        for (queue_rank, item) in items.iter_mut().enumerate() {
            item.queue_rank = queue_rank;
        }
        items
    }

    pub fn is_complete(&self) -> bool {
        self.next_tile_index >= self.tiles.len() && self.completed.iter().all(Option::is_some)
    }

    pub fn finish(&self) -> Option<PixelBuffer> {
        self.finish_checked().ok()
    }

    pub fn finish_checked(&self) -> Result<PixelBuffer> {
        if !self.is_complete() {
            return Err(WellfriendError::invalid_input(
                "progressive render cannot finish before all tiles are complete",
            ));
        }
        let mut out = PixelBuffer::new_filled_with_mode(
            self.page_width,
            self.page_height,
            WHITE,
            self.render_mode,
        );
        for (index, (tile, buffer)) in self.tiles.iter().zip(self.completed.iter()).enumerate() {
            let buffer = buffer.as_ref().ok_or_else(|| {
                WellfriendError::invalid_input(format!(
                    "progressive tile assembly missing completed tile {index}"
                ))
            })?;
            let dst_x = tile.x.checked_sub(self.output_origin_x).ok_or_else(|| {
                WellfriendError::invalid_input(format!(
                    "progressive tile assembly tile {index} begins before output origin"
                ))
            })?;
            let dst_y = tile.y.checked_sub(self.output_origin_y).ok_or_else(|| {
                WellfriendError::invalid_input(format!(
                    "progressive tile assembly tile {index} begins before output origin"
                ))
            })?;
            if !out.blit_from_buffer(buffer, dst_x, dst_y) {
                return Err(WellfriendError::invalid_input(format!(
                    "progressive tile assembly failed for tile {index} at {},{} size {}x{} into page {}x{}",
                    tile.x, tile.y, tile.width, tile.height, self.page_width, self.page_height
                )));
            }
        }
        Ok(out)
    }

    fn completed_count(&self) -> usize {
        self.completed.iter().filter(|tile| tile.is_some()).count()
    }

    fn first_incomplete_order_index(&self) -> usize {
        self.tile_order
            .iter()
            .position(|tile_index| self.completed[*tile_index].is_none())
            .unwrap_or(self.tiles.len())
    }

    fn memory_bytes_retained(&self) -> usize {
        self.completed
            .iter()
            .filter_map(|tile| tile.as_ref())
            .map(|tile| tile.width as usize * tile.height as usize * 4)
            .sum()
    }
}

fn viewer_queue_execution_item(
    item: ProgressiveViewerQueueItem,
    result: &str,
    tile_publication: Option<ProgressiveTilePublication>,
) -> ProgressiveViewerQueueExecutionItem {
    let execution = if tile_publication.is_some() {
        "render_current_page_tile"
    } else if item.tile.is_none() {
        "defer_external_prefetch"
    } else {
        "defer_current_page_tile"
    };
    ProgressiveViewerQueueExecutionItem {
        queue_rank: item.queue_rank,
        page_number: item.page_number,
        priority: item.priority,
        work_identity: item.work_identity.clone(),
        execution: execution.to_string(),
        result: result.to_string(),
        viewer_queue_item: item,
        tile_publication,
    }
}

fn progressive_contract_output_region(
    contract: &RenderContract,
    full_tile: RenderTile,
) -> Result<RenderTile> {
    let output_region = match contract.clip {
        Some(clip) => {
            if clip.x < 0 || clip.y < 0 {
                return Err(WellfriendError::invalid_input(
                    "progressive render contract clip origin must be non-negative device coordinates",
                ));
            }
            RenderTile {
                x: clip.x as u32,
                y: clip.y as u32,
                width: clip.width,
                height: clip.height,
            }
        }
        None => full_tile,
    };
    if output_region.width == 0 || output_region.height == 0 {
        return Err(WellfriendError::invalid_input(
            "progressive render contract output region must be non-zero",
        ));
    }
    if output_region.x >= full_tile.width || output_region.y >= full_tile.height {
        return Err(WellfriendError::invalid_input(
            "progressive render contract output origin lies outside the page viewport",
        ));
    }
    let end_x = output_region
        .x
        .checked_add(output_region.width)
        .ok_or_else(|| {
            WellfriendError::invalid_input("progressive render contract clip x range overflows")
        })?;
    let end_y = output_region
        .y
        .checked_add(output_region.height)
        .ok_or_else(|| {
            WellfriendError::invalid_input("progressive render contract clip y range overflows")
        })?;
    if end_x > full_tile.width || end_y > full_tile.height {
        return Err(WellfriendError::invalid_input(
            "progressive render contract output region lies outside the page viewport",
        ));
    }
    Ok(output_region)
}

/// Select a deterministic progressive tile size from the supported fixed set.
///
/// The policy is intentionally data-free and benchmark-free: it uses page
/// dimensions plus the render temporary-memory budget to pick a bounded tile
/// that keeps retained RGBA tile surfaces well below the active budget while
/// avoiding excessive scheduler fragmentation on small pages.
pub fn choose_adaptive_tile_size(
    page_width: u32,
    page_height: u32,
    budget: RenderResourceBudget,
) -> (u32, u32) {
    let page_pixels = u64::from(page_width).saturating_mul(u64::from(page_height));
    let target = if page_pixels <= 1_000_000 {
        384
    } else if page_pixels <= 4_000_000 {
        256
    } else if page_pixels <= 12_000_000 {
        192
    } else {
        128
    };
    let max_tile_pixels = budget.max_temporary_bytes.saturating_div(16).max(1);
    let selected = ADAPTIVE_TILE_SIZES
        .iter()
        .copied()
        .rev()
        .find(|size| {
            let pixels = u64::from(*size).saturating_mul(u64::from(*size));
            *size <= target && pixels <= max_tile_pixels
        })
        .unwrap_or(128);
    (selected, selected)
}

#[derive(Clone, Debug)]
struct ProgressiveTileSchedulerMetrics {
    display_operation_count: usize,
    hot_operation_count: usize,
    descriptor_arena_entries: usize,
    native_payload_batches: usize,
    compile_refusal_count: usize,
    image_operation_count: usize,
    clip_operation_count: usize,
    transparency_operation_count: usize,
    path_complexity_score: usize,
    complexity_score: usize,
}

struct ProgressiveTileSchedulerInputs<'a> {
    engine: &'a ContentEngine,
    page_number: usize,
    dpi: u32,
    render_mode: RenderMode,
    requested_tile_width: u32,
    requested_tile_height: u32,
    page_width: u32,
    page_height: u32,
    contract: &'a RenderContract,
}

fn build_tile_scheduler_report(
    input: ProgressiveTileSchedulerInputs<'_>,
) -> Result<ProgressiveTileSchedulerReport> {
    let ProgressiveTileSchedulerInputs {
        engine,
        page_number,
        dpi,
        render_mode,
        requested_tile_width,
        requested_tile_height,
        page_width,
        page_height,
        contract,
    } = input;
    let metrics = progressive_tile_scheduler_metrics(engine, page_number, dpi, contract)?;
    let budget = contract.resource_budget;
    let page_pixels = u64::from(page_width).saturating_mul(u64::from(page_height));
    let max_tile_pixels_by_budget = budget.max_temporary_bytes.saturating_div(16).max(1);
    let base_target_tile_size = adaptive_base_target_tile_size(page_pixels);
    let (complexity_tier, tier_steps) = complexity_tier_and_steps(metrics.complexity_score);
    let mut pressure_steps = tier_steps;
    let mut pressure_reasons = Vec::new();
    if tier_steps > 0 {
        pressure_reasons.push(format!("complexity_tier_{complexity_tier}"));
    }
    if render_mode.is_high_quality() {
        pressure_steps = pressure_steps.saturating_add(1);
        pressure_reasons.push("high_quality_mode".to_string());
    }
    if contract.print_profile != crate::render::PrintProfile::Display {
        pressure_steps = pressure_steps.saturating_add(1);
        pressure_reasons.push(format!("print_profile_{:?}", contract.print_profile));
    }
    if contract.backend == crate::render::BackendSelection::ScalarReference {
        pressure_steps = pressure_steps.saturating_add(1);
        pressure_reasons.push("scalar_reference_backend".to_string());
    }
    if metrics.transparency_operation_count > 0 {
        pressure_steps = pressure_steps.saturating_add(1);
        pressure_reasons.push("transparency_operations_present".to_string());
    }
    if metrics.image_operation_count >= 4 {
        pressure_steps = pressure_steps.saturating_add(1);
        pressure_reasons.push("image_density".to_string());
    }
    if metrics.clip_operation_count >= 8 {
        pressure_steps = pressure_steps.saturating_add(1);
        pressure_reasons.push("clip_complexity".to_string());
    }

    let pressure_steps = pressure_steps.min((ADAPTIVE_TILE_SIZES.len() - 1) as u8);
    let selected_target_tile_size =
        reduce_adaptive_tile_size(base_target_tile_size, pressure_steps);
    let (selected_tile_width, selected_tile_height, selection_mode, selection_reason) =
        if requested_tile_width == 0 || requested_tile_height == 0 {
            let selected = select_adaptive_tile_size_for_target(
                selected_target_tile_size,
                max_tile_pixels_by_budget,
            );
            let reason = if pressure_reasons.is_empty() {
                "adaptive_page_area_budget".to_string()
            } else {
                format!(
                    "adaptive_page_area_budget_plus_{}",
                    pressure_reasons.join("+")
                )
            };
            (selected, selected, "adaptive".to_string(), reason)
        } else {
            (
                requested_tile_width,
                requested_tile_height,
                "fixed".to_string(),
                "caller_fixed_tile_grid".to_string(),
            )
        };

    Ok(ProgressiveTileSchedulerReport {
        schema_version: PROGRESSIVE_TILE_SCHEDULER_REPORT_SCHEMA_VERSION,
        selection_mode,
        requested_tile_width,
        requested_tile_height,
        selected_tile_width,
        selected_tile_height,
        supported_tile_sizes: ADAPTIVE_TILE_SIZES.to_vec(),
        page_width,
        page_height,
        page_pixels,
        max_temporary_bytes: budget.max_temporary_bytes,
        max_tile_pixels_by_budget,
        render_mode: render_mode.as_str().to_string(),
        render_contract_fingerprint: contract.cache_fingerprint(),
        execution_mode: format!("{:?}", contract.execution_mode),
        backend_selection: format!("{:?}", contract.backend),
        print_profile: format!("{:?}", contract.print_profile),
        output_surface: format!("{:?}", contract.pixel_format),
        base_target_tile_size,
        selected_target_tile_size,
        complexity_score: metrics.complexity_score,
        complexity_tier: complexity_tier.to_string(),
        complexity_pressure_steps: pressure_steps,
        complexity_pressure_reasons: pressure_reasons,
        display_operation_count: metrics.display_operation_count,
        hot_operation_count: metrics.hot_operation_count,
        descriptor_arena_entries: metrics.descriptor_arena_entries,
        native_payload_batches: metrics.native_payload_batches,
        compile_refusal_count: metrics.compile_refusal_count,
        image_operation_count: metrics.image_operation_count,
        clip_operation_count: metrics.clip_operation_count,
        transparency_operation_count: metrics.transparency_operation_count,
        path_complexity_score: metrics.path_complexity_score,
        selection_reason,
    })
}

fn progressive_tile_scheduler_metrics(
    engine: &ContentEngine,
    page_number: usize,
    dpi: u32,
    contract: &RenderContract,
) -> Result<ProgressiveTileSchedulerMetrics> {
    let resources = engine.get_page_resources(page_number)?;
    let list = engine.build_page_display_list(page_number, dpi)?;
    let stats = list.stats.clone();
    let plan = RenderPlan::compile_with_resources(list, contract.clone(), &resources)?;
    let native_payload_batches = plan
        .batches
        .iter()
        .filter(|batch| batch.contains_native_payload)
        .count();
    let compile_refusal_count = plan
        .packed
        .descriptors
        .iter()
        .filter(|descriptor| matches!(descriptor, NativeDescriptor::CompileRefusal(_)))
        .count();
    Ok(ProgressiveTileSchedulerMetrics::from_stats(
        &stats,
        plan.packed.hot_ops.len(),
        plan.packed.descriptors.len(),
        native_payload_batches,
        compile_refusal_count,
    ))
}

impl ProgressiveTileSchedulerMetrics {
    fn from_stats(
        stats: &DisplayListStats,
        hot_operation_count: usize,
        descriptor_arena_entries: usize,
        native_payload_batches: usize,
        compile_refusal_count: usize,
    ) -> Self {
        let image_operation_count = stats
            .image_xobjects
            .saturating_add(stats.inline_images)
            .saturating_add(stats.native_image_xobjects)
            .saturating_add(stats.native_inline_images);
        let clip_operation_count = stats.clips;
        let transparency_operation_count = stats.transparency_ops;
        let path_complexity_score = stats
            .paths
            .saturating_add(stats.fills)
            .saturating_add(stats.strokes)
            .saturating_add(stats.path_segments);
        let complexity_score = stats
            .operations
            .saturating_add(path_complexity_score)
            .saturating_add(image_operation_count.saturating_mul(8))
            .saturating_add(stats.form_xobjects.saturating_mul(12))
            .saturating_add(stats.shadings.saturating_mul(10))
            .saturating_add(stats.patterns.saturating_mul(12))
            .saturating_add(transparency_operation_count.saturating_mul(16))
            .saturating_add(clip_operation_count.saturating_mul(4))
            .saturating_add(native_payload_batches.saturating_mul(6))
            .saturating_add(compile_refusal_count.saturating_mul(24));
        Self {
            display_operation_count: stats.operations,
            hot_operation_count,
            descriptor_arena_entries,
            native_payload_batches,
            compile_refusal_count,
            image_operation_count,
            clip_operation_count,
            transparency_operation_count,
            path_complexity_score,
            complexity_score,
        }
    }
}

fn adaptive_base_target_tile_size(page_pixels: u64) -> u32 {
    if page_pixels <= 1_000_000 {
        384
    } else if page_pixels <= 4_000_000 {
        256
    } else if page_pixels <= 12_000_000 {
        192
    } else {
        128
    }
}

fn select_adaptive_tile_size_for_target(target: u32, max_tile_pixels: u64) -> u32 {
    ADAPTIVE_TILE_SIZES
        .iter()
        .copied()
        .rev()
        .find(|size| {
            let pixels = u64::from(*size).saturating_mul(u64::from(*size));
            *size <= target && pixels <= max_tile_pixels
        })
        .unwrap_or(128)
}

fn reduce_adaptive_tile_size(size: u32, steps: u8) -> u32 {
    let mut selected = size;
    for _ in 0..steps {
        selected = match selected {
            512 => 384,
            384 => 256,
            256 => 192,
            192 => 128,
            _ => 128,
        };
    }
    selected
}

fn complexity_tier_and_steps(score: usize) -> (&'static str, u8) {
    if score >= 2048 {
        ("extreme", 3)
    } else if score >= 512 {
        ("high", 2)
    } else if score >= 128 {
        ("moderate", 1)
    } else {
        ("low", 0)
    }
}

fn tile_priority_key(tile: RenderTile, hint: RenderTile) -> (u8, u64, u32, u32) {
    let priority_rank = tile_viewer_priority_rank(tile, hint);
    let distance = tile_center_distance_sq(tile, hint);
    (priority_rank, distance, tile.y, tile.x)
}

fn viewer_queue_priority(
    tile: RenderTile,
    viewport_hint: Option<RenderTile>,
) -> ProgressiveViewerQueuePriority {
    let Some(hint) = viewport_hint else {
        return ProgressiveViewerQueuePriority::Unhinted;
    };
    if tile_contains_viewport_center(tile, hint) {
        return ProgressiveViewerQueuePriority::CenterVisibleTile;
    }
    match tile_priority_class(tile, hint) {
        ProgressiveTilePriorityClass::Visible => ProgressiveViewerQueuePriority::VisibleTile,
        ProgressiveTilePriorityClass::AdjacentViewport => {
            ProgressiveViewerQueuePriority::NearVisibleTile
        }
        ProgressiveTilePriorityClass::Background => {
            ProgressiveViewerQueuePriority::BackgroundPrefetch
        }
        ProgressiveTilePriorityClass::Unhinted => ProgressiveViewerQueuePriority::Unhinted,
    }
}

fn tile_schedule_key(
    tile: RenderTile,
    tile_index: usize,
    viewport_hint: Option<RenderTile>,
    dirty_region: Option<RenderTile>,
    completed: bool,
) -> (u8, u8, u64, u32, u32, usize) {
    let dirty_rank = match dirty_region {
        Some(dirty) if tile_intersects(tile, dirty) => 0,
        Some(_) if !completed => 1,
        Some(_) => 2,
        None if completed => 1,
        None => 0,
    };
    let (priority_rank, distance) = match viewport_hint {
        Some(hint) => (
            tile_viewer_priority_rank(tile, hint),
            tile_center_distance_sq(tile, hint),
        ),
        None => (ProgressiveTilePriorityClass::Unhinted.sort_rank(), 0),
    };
    (
        dirty_rank,
        priority_rank,
        distance,
        tile.y,
        tile.x,
        tile_index,
    )
}

fn tile_viewer_priority_rank(tile: RenderTile, hint: RenderTile) -> u8 {
    if tile_contains_viewport_center(tile, hint) {
        return 0;
    }
    match tile_priority_class(tile, hint) {
        ProgressiveTilePriorityClass::Visible => 1,
        ProgressiveTilePriorityClass::AdjacentViewport => 2,
        ProgressiveTilePriorityClass::Background => 3,
        ProgressiveTilePriorityClass::Unhinted => 4,
    }
}

fn tile_intersects(tile: RenderTile, region: RenderTile) -> bool {
    let tile_x1 = tile.x.saturating_add(tile.width);
    let tile_y1 = tile.y.saturating_add(tile.height);
    let region_x1 = region.x.saturating_add(region.width);
    let region_y1 = region.y.saturating_add(region.height);
    tile.x < region_x1 && region.x < tile_x1 && tile.y < region_y1 && region.y < tile_y1
}

fn tile_priority_class(tile: RenderTile, hint: RenderTile) -> ProgressiveTilePriorityClass {
    let tile_x1 = tile.x.saturating_add(tile.width);
    let tile_y1 = tile.y.saturating_add(tile.height);
    let hint_x1 = hint.x.saturating_add(hint.width);
    let hint_y1 = hint.y.saturating_add(hint.height);
    if tile_intersects(tile, hint) {
        return ProgressiveTilePriorityClass::Visible;
    }

    let adjacent_hint_x0 = hint.x.saturating_sub(tile.width);
    let adjacent_hint_y0 = hint.y.saturating_sub(tile.height);
    let adjacent_hint_x1 = hint_x1.saturating_add(tile.width);
    let adjacent_hint_y1 = hint_y1.saturating_add(tile.height);
    let adjacent = tile.x < adjacent_hint_x1
        && adjacent_hint_x0 < tile_x1
        && tile.y < adjacent_hint_y1
        && adjacent_hint_y0 < tile_y1;
    if adjacent {
        ProgressiveTilePriorityClass::AdjacentViewport
    } else {
        ProgressiveTilePriorityClass::Background
    }
}

fn tile_contains_viewport_center(tile: RenderTile, hint: RenderTile) -> bool {
    let center_x2 = u64::from(hint.x) * 2 + u64::from(hint.width);
    let center_y2 = u64::from(hint.y) * 2 + u64::from(hint.height);
    let tile_x0_2 = u64::from(tile.x) * 2;
    let tile_y0_2 = u64::from(tile.y) * 2;
    let tile_x1_2 = u64::from(tile.x.saturating_add(tile.width)) * 2;
    let tile_y1_2 = u64::from(tile.y.saturating_add(tile.height)) * 2;
    tile_x0_2 <= center_x2
        && center_x2 < tile_x1_2
        && tile_y0_2 <= center_y2
        && center_y2 < tile_y1_2
}

fn tile_center_distance_sq(tile: RenderTile, hint: RenderTile) -> u64 {
    let tile_cx = u64::from(tile.x) * 2 + u64::from(tile.width);
    let tile_cy = u64::from(tile.y) * 2 + u64::from(tile.height);
    let hint_cx = u64::from(hint.x) * 2 + u64::from(hint.width);
    let hint_cy = u64::from(hint.y) * 2 + u64::from(hint.height);
    let dx = tile_cx.abs_diff(hint_cx);
    let dy = tile_cy.abs_diff(hint_cy);
    dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy))
}

fn format_viewport_identity(viewport_hint: Option<RenderTile>) -> String {
    viewport_hint
        .map(format_tile_identity)
        .unwrap_or_else(|| "none".to_string())
}

fn format_tile_identity(tile: RenderTile) -> String {
    format!("{}:{}:{}:{}", tile.x, tile.y, tile.width, tile.height)
}

fn validate_progressive_identity_fragment(name: &str, value: String) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(WellfriendError::invalid_input(format!(
            "{name} must not be empty"
        )));
    }
    if trimmed.len() > 256 {
        return Err(WellfriendError::invalid_input(format!(
            "{name} must be 256 bytes or shorter"
        )));
    }
    if !trimmed.bytes().all(|byte| matches!(byte, 0x21..=0x7e)) {
        return Err(WellfriendError::invalid_input(format!(
            "{name} must contain only printable non-whitespace ASCII"
        )));
    }
    Ok(trimmed.to_string())
}

fn adjacent_page_seeds(
    engine: &ContentEngine,
    page_number: usize,
    dpi: u32,
) -> Result<Vec<ProgressiveAdjacentPageSeed>> {
    let page_count = engine.page_count()?;
    let mut candidates = Vec::new();
    if page_number < page_count {
        candidates.push((page_number + 1, 1));
    }
    if page_number > 1 {
        candidates.push((page_number - 1, -1));
    }
    candidates
        .into_iter()
        .map(|(candidate_page, relative_page_offset)| {
            let viewport = engine.page_viewport(candidate_page, dpi)?;
            Ok(ProgressiveAdjacentPageSeed {
                page_number: candidate_page,
                relative_page_offset,
                page_width: viewport.width_px,
                page_height: viewport.height_px,
            })
        })
        .collect()
}

fn is_unsupported_display_list_retained_tile_refusal(error: &WellfriendError) -> bool {
    matches!(
        error,
        WellfriendError::UnsupportedFeature(message)
            if message.contains("unsupported retained display-list replay")
    )
}

fn unsupported_display_list_retained_tile_refusal_event(
    tile: RenderTile,
    exactness_policy: ExactnessPolicy,
) -> ProgressiveRenderFallbackEvent {
    let exact_mode = exactness_policy == ExactnessPolicy::HighQualityExact;
    ProgressiveRenderFallbackEvent {
        code: "unsupported_display_list_retained_tile_refusal".to_string(),
        call_site: "render/progressive.rs:render_next".to_string(),
        trigger: "retained display-list tile render returned unsupported for this page tile"
            .to_string(),
        output: "typed UnsupportedFeature refusal".to_string(),
        degradation: "no tile is published when retained replay is unsupported".to_string(),
        standard_available: true,
        high_quality_available: !exact_mode,
        replacement: "use native retained-plan-supported content or select direct contract rendering explicitly".to_string(),
        final_policy: if exact_mode {
            "HighQualityExact must not publish unsupported retained replay".to_string()
        } else {
            "typed compatibility refusal for content outside native retained-plan support".to_string()
        },
        tile: Some(tile),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuthorPageSize, PdfBuilder, TextStyle};

    fn test_engine() -> ContentEngine {
        let mut builder = PdfBuilder::new();
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("progressive lifecycle", 72.0, 720.0, &TextStyle::default())
            .expect("write page");
        ContentEngine::open_bytes(builder.to_bytes().expect("serialize test PDF"))
            .expect("open test PDF")
    }

    fn unsupported_display_list_engine() -> ContentEngine {
        let content = "Q\n";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 4 0 R >>".to_vec(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content)
                .into_bytes(),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = vec![0usize];
        for (idx, obj) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
            out.extend_from_slice(obj);
            out.extend_from_slice(b"\nendobj\n");
        }
        let startxref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            out.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
                objects.len() + 1,
                startxref
            )
            .as_bytes(),
        );
        ContentEngine::open_bytes(out).expect("open unsupported display-list PDF")
    }

    fn multipage_test_engine() -> ContentEngine {
        let mut builder = PdfBuilder::new();
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("page one", 72.0, 720.0, &TextStyle::default())
            .expect("write page one");
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("page two", 72.0, 720.0, &TextStyle::default())
            .expect("write page two");
        ContentEngine::open_bytes(builder.to_bytes().expect("serialize test PDF"))
            .expect("open test PDF")
    }

    fn finish_progressive_with_quanta(
        engine: ContentEngine,
        quanta: &[usize],
        viewport_hint: Option<RenderTile>,
    ) -> (String, Vec<RenderTile>, PixelBuffer) {
        assert!(!quanta.is_empty(), "test quanta must not be empty");
        let mut job = ProgressiveRenderJob::new_with_viewport_hint(
            engine,
            1,
            72,
            RenderMode::Compat,
            64,
            64,
            viewport_hint,
        )
        .expect("create deterministic progressive job");
        let publication_identity = job.publication_identity();
        let tile_order = job
            .tile_order
            .iter()
            .copied()
            .map(|index| job.tiles[index])
            .collect::<Vec<_>>();
        let mut quantum_index = 0usize;
        while !job.is_complete() {
            let quantum = quanta[quantum_index % quanta.len()];
            let report = job
                .render_next(quantum, &CancelToken::none())
                .expect("render deterministic progressive quantum");
            assert!(
                report.rendered_this_step > 0 || job.is_complete(),
                "progressive schedule must advance or complete"
            );
            quantum_index += 1;
        }
        (
            publication_identity,
            tile_order,
            job.finish().expect("completed progressive surface"),
        )
    }

    #[test]
    fn adaptive_tile_size_is_deterministic_and_budget_bounded() {
        let default = RenderResourceBudget::default();
        assert_eq!(choose_adaptive_tile_size(800, 1000, default), (384, 384));
        assert_eq!(choose_adaptive_tile_size(2200, 1800, default), (256, 256));
        assert_eq!(choose_adaptive_tile_size(3600, 2400, default), (192, 192));
        assert_eq!(choose_adaptive_tile_size(8000, 8000, default), (128, 128));

        let tiny_budget = RenderResourceBudget {
            max_temporary_bytes: 128 * 128 * 16,
            ..RenderResourceBudget::default()
        };
        assert_eq!(
            choose_adaptive_tile_size(800, 1000, tiny_budget),
            (128, 128)
        );
    }

    #[test]
    fn zero_tile_dimension_selects_adaptive_size() {
        let engine = test_engine();
        let job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 0, 0)
            .expect("create adaptive job");
        assert!(ADAPTIVE_TILE_SIZES.contains(&job.tile_width));
        assert_eq!(job.tile_width, job.tile_height);
        let token = job.token();
        assert_eq!(token.tile_width, job.tile_width);
        let scheduler = token
            .tile_scheduler
            .as_ref()
            .expect("token carries scheduler policy");
        assert_eq!(scheduler.selection_mode, "adaptive");
        assert_eq!(scheduler.selected_tile_width, job.tile_width);
        assert_eq!(
            scheduler.render_contract_fingerprint,
            job.render_contract_fingerprint
        );
        let report_json = serde_json::to_value(job.viewer_queue_report()).expect("serialize");
        assert_eq!(report_json["tile_scheduler"]["selection_mode"], "adaptive");
        assert_eq!(
            report_json["tile_scheduler"]["selected_tile_width"],
            serde_json::json!(job.tile_width)
        );
    }

    #[test]
    fn adaptive_scheduler_report_uses_contract_and_plan_complexity_pressure() {
        let engine = test_engine();
        let job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::HighQuality, 0, 0)
            .expect("create high-quality adaptive job");
        let scheduler = &job.tile_scheduler;
        assert_eq!(scheduler.schema_version, 1);
        assert_eq!(scheduler.selection_mode, "adaptive");
        assert_eq!(scheduler.render_mode, "high");
        assert_eq!(
            scheduler.render_contract_fingerprint,
            job.render_contract_fingerprint
        );
        assert!(scheduler.hot_operation_count > 0);
        assert!(scheduler.display_operation_count > 0);
        assert!(scheduler
            .complexity_pressure_reasons
            .iter()
            .any(|reason| reason == "high_quality_mode"));
        assert!(
            scheduler.selected_target_tile_size <= scheduler.base_target_tile_size,
            "complexity/profile pressure may only keep or reduce adaptive target"
        );
    }

    #[test]
    fn lifecycle_pause_resume_cancel_and_close_are_explicit() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine.clone(), 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        assert_eq!(job.state(), ProgressiveRenderState::Created);
        let token = job.pause().expect("pause created job");
        assert_eq!(job.state(), ProgressiveRenderState::Paused);
        assert_eq!(token.lifecycle_state, "paused");
        job.resume(&token).expect("resume paused job");
        assert_eq!(job.state(), ProgressiveRenderState::Rendering);
        job.cancel();
        assert_eq!(job.state(), ProgressiveRenderState::Cancelled);
        assert!(job.render_next(1, &CancelToken::none()).is_err());
        job.close();
        assert_eq!(job.state(), ProgressiveRenderState::Closed);
        assert!(job.finish().is_none());
    }

    #[test]
    fn finish_checked_reports_incomplete_tile_assembly() {
        let engine = test_engine();
        let job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        let err = job
            .finish_checked()
            .expect_err("incomplete progressive job must be typed");
        assert!(format!("{err}").contains("cannot finish before all tiles are complete"));
        assert!(job.finish().is_none());
    }

    #[test]
    fn session_request_cancel_yields_resumable_step_and_resume_refreshes_token() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        let retained_cancel_token = job.cancellation_token();

        job.request_cancel();
        let report = job
            .render_next(4, &CancelToken::none())
            .expect("request cancellation produces a step report");
        assert!(report.cancelled);
        assert_eq!(report.phase, "cancelled_resumable");
        assert_eq!(job.state(), ProgressiveRenderState::Paused);
        assert!(report.resume_possible);

        let token = job.token();
        job.resume(&token)
            .expect("resumable cancellation token can resume");
        retained_cancel_token.cancel();
        let report = job
            .render_next(1, &CancelToken::none())
            .expect("retained token clone remains current after resume");
        assert!(report.cancelled);
        assert_eq!(report.phase, "cancelled_resumable");

        let token = job.token();
        job.resume(&token)
            .expect("resumable cancellation token can resume again");
        let report = job
            .render_next(1, &CancelToken::none())
            .expect("resume clears the retained session cancellation source");
        assert!(!report.cancelled);
        assert!(report.rendered_this_step <= 1);
    }

    #[test]
    fn viewport_hint_prioritizes_intersecting_tile() {
        let engine = test_engine();
        let hint = RenderTile {
            x: 576,
            y: 704,
            width: 32,
            height: 64,
        };
        let mut job = ProgressiveRenderJob::new_with_viewport_hint(
            engine,
            1,
            72,
            RenderMode::Compat,
            64,
            64,
            Some(hint),
        )
        .expect("create hinted job");
        let report = job
            .render_next(1, &CancelToken::none())
            .expect("render visible tile");
        assert_eq!(report.completed_tiles.len(), 1);
        let first = report.completed_tiles[0];
        assert!(
            first.x < hint.x.saturating_add(hint.width)
                && hint.x < first.x.saturating_add(first.width)
                && first.y < hint.y.saturating_add(hint.height)
                && hint.y < first.y.saturating_add(first.height),
            "first tile {first:?} must intersect hint {hint:?}"
        );
    }

    #[test]
    fn publication_identity_tracks_revision_view_and_legacy_token_compatibility() {
        let engine = test_engine();
        let unhinted = ProgressiveRenderJob::new_with_viewport_hint(
            engine.clone(),
            1,
            72,
            RenderMode::Compat,
            64,
            64,
            None,
        )
        .expect("create unhinted job");
        let hint = RenderTile {
            x: 576,
            y: 704,
            width: 32,
            height: 64,
        };
        let hinted = ProgressiveRenderJob::new_with_viewport_hint(
            engine,
            1,
            72,
            RenderMode::Compat,
            64,
            64,
            Some(hint),
        )
        .expect("create hinted job");

        let identity = hinted.publication_identity();
        assert_ne!(unhinted.publication_identity(), identity);
        assert!(identity.contains("wf-progressive:v1"));
        assert!(identity.contains("page=1"));
        assert!(identity.contains("tile_grid=64x64"));
        assert!(identity.contains("viewport=576:704:32:64"));

        let token = hinted.token();
        assert_eq!(token.publication_identity, identity);
        hinted.validate_resume_token(&token).expect("fresh token");

        let mut stale = token.clone();
        stale.publication_identity = "wf-progressive:v1:stale".to_string();
        let err = hinted
            .validate_resume_token(&stale)
            .expect_err("stale publication identity rejected");
        assert!(format!("{err}").contains("publication_identity"));

        let mut legacy = token;
        legacy.publication_identity.clear();
        hinted
            .validate_resume_token(&legacy)
            .expect("legacy token without publication identity remains resumable");
    }

    #[test]
    fn step_report_includes_publication_identity_for_completed_tiles() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        let tile_index = job.tile_order[0];
        let tile = job.tiles[tile_index];
        job.completed[tile_index] = Some(PixelBuffer::new_filled_with_mode(
            tile.width,
            tile.height,
            WHITE,
            job.render_mode,
        ));
        job.next_tile_index = 1;

        let identity = job.publication_identity();
        let report = job.step_report(1, false);
        assert_eq!(report.publication_identity, identity);
        assert_eq!(report.completed_tile_publications.len(), 1);

        let publication = &report.completed_tile_publications[0];
        assert_eq!(publication.tile_index, tile_index);
        assert_eq!(publication.tile_order_index, 0);
        assert_eq!(publication.tile, tile);
        assert_eq!(
            publication.priority_class,
            ProgressiveTilePriorityClass::Unhinted
        );
        assert_eq!(publication.publication_identity, identity);
        assert_eq!(
            publication.document_revision,
            job.engine.canonical_document().revision().0
        );
        assert_eq!(
            publication.visibility_fingerprint,
            job.visibility_fingerprint
        );
        assert!(publication
            .tile_publication_identity
            .contains(&format!("tile_index={tile_index}")));
        assert!(publication
            .tile_publication_identity
            .contains("tile=0:0:64:64"));
        let report_json = serde_json::to_value(&report).expect("serialize step report");
        assert_eq!(
            report_json["completed_tile_publications"][0]["priority_class"],
            "unhinted"
        );
    }

    #[test]
    fn viewport_hint_orders_visible_adjacent_then_background_tiles() {
        let hint = RenderTile {
            x: 128,
            y: 128,
            width: 32,
            height: 32,
        };
        let visible = RenderTile {
            x: 128,
            y: 128,
            width: 64,
            height: 64,
        };
        let adjacent = RenderTile {
            x: 192,
            y: 128,
            width: 64,
            height: 64,
        };
        let background = RenderTile {
            x: 384,
            y: 128,
            width: 64,
            height: 64,
        };

        assert_eq!(
            tile_priority_class(visible, hint),
            ProgressiveTilePriorityClass::Visible
        );
        assert_eq!(
            tile_priority_class(adjacent, hint),
            ProgressiveTilePriorityClass::AdjacentViewport
        );
        assert_eq!(
            tile_priority_class(background, hint),
            ProgressiveTilePriorityClass::Background
        );
        assert!(tile_priority_key(visible, hint) < tile_priority_key(adjacent, hint));
        assert!(tile_priority_key(adjacent, hint) < tile_priority_key(background, hint));
        assert_eq!(
            viewer_queue_priority(visible, Some(hint)),
            ProgressiveViewerQueuePriority::CenterVisibleTile
        );
        assert_eq!(
            viewer_queue_priority(adjacent, Some(hint)),
            ProgressiveViewerQueuePriority::NearVisibleTile
        );
        assert_eq!(
            viewer_queue_priority(background, Some(hint)),
            ProgressiveViewerQueuePriority::BackgroundPrefetch
        );
    }

    #[test]
    fn center_visible_tile_orders_before_other_visible_tiles() {
        let hint = RenderTile {
            x: 128,
            y: 128,
            width: 32,
            height: 32,
        };
        let visible_edge = RenderTile {
            x: 128,
            y: 128,
            width: 16,
            height: 64,
        };
        let center_visible = RenderTile {
            x: 144,
            y: 128,
            width: 16,
            height: 64,
        };

        assert_eq!(
            tile_priority_class(visible_edge, hint),
            ProgressiveTilePriorityClass::Visible
        );
        assert_eq!(
            tile_priority_class(center_visible, hint),
            ProgressiveTilePriorityClass::Visible
        );
        assert_eq!(
            viewer_queue_priority(visible_edge, Some(hint)),
            ProgressiveViewerQueuePriority::VisibleTile
        );
        assert_eq!(
            viewer_queue_priority(center_visible, Some(hint)),
            ProgressiveViewerQueuePriority::CenterVisibleTile
        );
        assert!(tile_priority_key(center_visible, hint) < tile_priority_key(visible_edge, hint));
        assert!(
            tile_schedule_key(center_visible, 1, Some(hint), None, false)
                < tile_schedule_key(visible_edge, 0, Some(hint), None, false)
        );
    }

    #[test]
    fn progressive_quantum_schedules_finish_to_identical_pixels() {
        let engine = test_engine();
        let full =
            PageRenderer::render_page_with_mode(&engine, 1, 72, RenderMode::Compat).expect("full");

        let (single_identity, single_order, single_tile) =
            finish_progressive_with_quanta(engine.clone(), &[1], None);
        let (batched_identity, batched_order, batched) =
            finish_progressive_with_quanta(engine.clone(), &[3, 2, 5], None);
        assert_eq!(single_identity, batched_identity);
        assert_eq!(single_order, batched_order);
        assert_eq!(single_tile.width, full.width);
        assert_eq!(single_tile.height, full.height);
        assert_eq!(single_tile.rgba_bytes(), full.rgba_bytes());
        assert_eq!(batched.rgba_bytes(), full.rgba_bytes());

        let hint = RenderTile {
            x: 576,
            y: 704,
            width: 32,
            height: 64,
        };
        let (hinted_identity, hinted_order, hinted) =
            finish_progressive_with_quanta(engine, &[4, 1], Some(hint));
        assert_ne!(hinted_identity, single_identity);
        assert_ne!(hinted_order, single_order);
        assert_eq!(hinted.width, full.width);
        assert_eq!(hinted.height, full.height);
        assert_eq!(hinted.rgba_bytes(), full.rgba_bytes());
    }

    #[test]
    fn progressive_concurrent_worker_jobs_finish_to_identical_pixels() {
        let engine = test_engine();
        let full =
            PageRenderer::render_page_with_mode(&engine, 1, 72, RenderMode::Compat).expect("full");
        let results = std::thread::scope(|scope| {
            let handles = [vec![1usize], vec![3usize, 2, 5]]
                .into_iter()
                .map(|quanta| {
                    let engine = engine.clone();
                    scope.spawn(move || finish_progressive_with_quanta(engine, &quanta, None))
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("progressive worker thread"))
                .collect::<Vec<_>>()
        });

        assert_eq!(results.len(), 2);
        let (_, _, first_pixels) = &results[0];
        for (identity, order, pixels) in &results {
            assert_eq!(pixels.width, full.width);
            assert_eq!(pixels.height, full.height);
            assert_eq!(pixels.rgba_bytes(), full.rgba_bytes());
            assert_eq!(pixels.rgba_bytes(), first_pixels.rgba_bytes());
            assert_eq!(identity, &results[0].0);
            assert_eq!(order, &results[0].1);
        }
    }

    #[test]
    fn step_report_includes_adjacent_page_prefetch_and_viewer_queue_preview() {
        let engine = multipage_test_engine();
        let hint = RenderTile {
            x: 128,
            y: 128,
            width: 96,
            height: 96,
        };
        let job = ProgressiveRenderJob::new_with_viewport_hint(
            engine,
            1,
            72,
            RenderMode::Compat,
            64,
            64,
            Some(hint),
        )
        .expect("create hinted multipage job");

        let report = job.step_report(0, false);
        assert_eq!(report.adjacent_page_prefetches.len(), 1);
        let adjacent = &report.adjacent_page_prefetches[0];
        assert_eq!(adjacent.page_number, 2);
        assert_eq!(adjacent.relative_page_offset, 1);
        assert_eq!(
            adjacent.priority,
            ProgressiveViewerQueuePriority::AdjacentPagePreview
        );
        assert_eq!(
            adjacent.source_publication_identity,
            report.publication_identity
        );
        assert!(adjacent
            .prefetch_identity
            .contains("adjacent_page=2:offset=1"));

        let preview = &report.viewer_queue_preview;
        assert!(preview
            .iter()
            .any(|item| item.priority == ProgressiveViewerQueuePriority::CenterVisibleTile));
        assert!(preview.iter().any(|item| {
            item.priority == ProgressiveViewerQueuePriority::AdjacentPagePreview
                && item.page_number == 2
                && item.relative_page_offset == Some(1)
        }));
        let adjacent_rank = preview
            .iter()
            .position(|item| item.priority == ProgressiveViewerQueuePriority::AdjacentPagePreview)
            .expect("adjacent prefetch appears in queue preview");
        let background_rank = preview
            .iter()
            .position(|item| item.priority == ProgressiveViewerQueuePriority::BackgroundPrefetch)
            .expect("background prefetch appears in queue preview");
        assert!(adjacent_rank < background_rank);

        let json = serde_json::to_value(&report).expect("serialize step report");
        assert_eq!(
            json["adjacent_page_prefetches"][0]["priority"],
            "adjacent_page_preview"
        );
        assert_eq!(
            json["viewer_queue_preview"][adjacent_rank]["priority"],
            "adjacent_page_preview"
        );
    }

    #[test]
    fn viewer_queue_execution_renders_current_page_and_defers_adjacent_prefetch() {
        let engine = multipage_test_engine();
        let hint = RenderTile {
            x: 128,
            y: 128,
            width: 96,
            height: 96,
        };
        let mut job = ProgressiveRenderJob::new_with_viewport_hint(
            engine,
            1,
            72,
            RenderMode::Compat,
            256,
            256,
            Some(hint),
        )
        .expect("create hinted multipage job");

        let report = job
            .execute_viewer_queue(MAX_VIEWER_QUEUE_PREVIEW_ITEMS, &CancelToken::none())
            .expect("execute viewer queue");

        assert!(!report.terminal_suppressed);
        assert_eq!(report.requested_max_items, MAX_VIEWER_QUEUE_PREVIEW_ITEMS);
        assert!(report.rendered_current_page_tiles > 0);
        assert!(
            report.queue_after.completed_units > report.queue_before.completed_units,
            "queue execution must advance retained current-page work"
        );
        assert_eq!(
            report
                .render_step_report
                .as_ref()
                .expect("render step report")
                .rendered_this_step,
            report.rendered_current_page_tiles
        );
        assert!(report.executed_items.iter().any(|item| {
            item.result == "rendered_current_page_tile" && item.tile_publication.is_some()
        }));
        assert!(report.deferred_queue_items.iter().any(|item| {
            item.result == "deferred_adjacent_page_prefetch_requires_page_session"
                && item.page_number == 2
                && item.viewer_queue_item.relative_page_offset == Some(1)
        }));
        assert!(report.warnings.iter().any(|warning| {
            warning.contains("adjacent-page prefetch items are reported for the caller")
        }));
    }

    #[test]
    fn viewer_queue_execution_suppresses_terminal_sessions() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        job.close();

        let report = job
            .execute_viewer_queue(3, &CancelToken::none())
            .expect("terminal queue execution report");

        assert!(report.terminal_suppressed);
        assert_eq!(report.lifecycle_state, "closed");
        assert_eq!(
            report.suppression_reason.as_deref(),
            Some("terminal_state_closed")
        );
        assert_eq!(report.rendered_current_page_tiles, 0);
        assert_eq!(report.suppressed_items, report.attempted_queue_items);
        assert!(report.render_step_report.is_none());
        assert!(report.executed_items.is_empty());
    }

    #[test]
    fn adjacent_page_prefetch_execution_builds_page_owned_child_job() {
        let engine = multipage_test_engine();
        let hint = RenderTile {
            x: 128,
            y: 128,
            width: 96,
            height: 96,
        };
        let job = ProgressiveRenderJob::new_with_viewport_hint(
            engine,
            1,
            72,
            RenderMode::Compat,
            256,
            256,
            Some(hint),
        )
        .expect("create hinted multipage job");
        let queue = job.viewer_queue_report();
        let prefetch_identity = queue.adjacent_page_prefetches[0].prefetch_identity.clone();

        let execution = job
            .execute_adjacent_page_prefetch(&prefetch_identity, 2, &CancelToken::none())
            .expect("execute adjacent-page prefetch");
        let report = execution.report;

        assert!(execution.job.is_some());
        assert!(report.executed);
        assert!(!report.terminal_suppressed);
        assert_eq!(report.page_number, 2);
        assert_eq!(report.relative_page_offset, 1);
        assert_eq!(report.requested_max_tiles, 2);
        assert_eq!(
            report
                .child_token
                .as_ref()
                .expect("child token")
                .page_number,
            2
        );
        assert!(
            report
                .render_step_report
                .as_ref()
                .expect("child step")
                .rendered_this_step
                > 0
        );
        assert!(report
            .warnings
            .iter()
            .any(|warning| { warning.contains("page-owned progressive child job") }));
    }

    #[test]
    fn adjacent_page_prefetch_execution_suppresses_terminal_source_session() {
        let engine = multipage_test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 256, 256)
            .expect("create multipage job");
        let prefetch_identity = job.viewer_queue_report().adjacent_page_prefetches[0]
            .prefetch_identity
            .clone();
        job.close();

        let execution = job
            .execute_adjacent_page_prefetch(&prefetch_identity, 1, &CancelToken::none())
            .expect("terminal adjacent prefetch report");
        let report = execution.report;

        assert!(execution.job.is_none());
        assert!(!report.executed);
        assert!(report.terminal_suppressed);
        assert_eq!(
            report.suppression_reason.as_deref(),
            Some("terminal_state_closed")
        );
        assert!(report.child_token.is_none());
        assert!(report.render_step_report.is_none());
    }

    #[derive(Default)]
    struct CallbackCapture {
        callbacks: Vec<String>,
    }

    impl ProgressiveViewerCallbackSink for CallbackCapture {
        fn progressive_viewer_callback(&mut self, event: &ProgressiveViewerCallbackEvent) {
            self.callbacks.push(event.callback.clone());
        }
    }

    #[test]
    fn viewer_callback_dispatch_reports_ordered_events() {
        let engine = multipage_test_engine();
        let hint = RenderTile {
            x: 128,
            y: 128,
            width: 96,
            height: 96,
        };
        let mut job = ProgressiveRenderJob::new_with_viewport_hint(
            engine,
            1,
            72,
            RenderMode::Compat,
            64,
            64,
            Some(hint),
        )
        .expect("create hinted multipage job");
        job.render_next(1, &CancelToken::none())
            .expect("render one tile");

        let mut sink = CallbackCapture::default();
        let report = job.dispatch_viewer_callbacks(&mut sink);

        assert_eq!(report.callbacks_dispatched, report.events.len());
        assert_eq!(sink.callbacks.len(), report.events.len());
        assert!(report.suppression_reason.is_none());
        assert!(report
            .events
            .iter()
            .any(|event| event.callback == "tile_publication_ready"
                && event.requires_publication_acceptance));
        assert!(report
            .events
            .iter()
            .any(|event| event.callback == "adjacent_page_prefetch_ready"));
        assert!(report
            .events
            .iter()
            .any(|event| event.callback == "viewer_queue_item_scheduled"));
        for (idx, event) in report.events.iter().enumerate() {
            assert_eq!(event.sequence, idx);
        }
    }

    #[test]
    fn viewer_callback_dispatch_suppresses_terminal_sessions() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        job.render_next(1, &CancelToken::none())
            .expect("render one tile");
        job.close();

        let mut sink = CallbackCapture::default();
        let report = job.dispatch_viewer_callbacks(&mut sink);

        assert!(report.no_callback_after_terminal_state);
        assert_eq!(report.lifecycle_state, "closed");
        assert_eq!(
            report.suppression_reason.as_deref(),
            Some("terminal_state_closed")
        );
        assert_eq!(report.callbacks_dispatched, 0);
        assert!(report.callbacks_suppressed > 0);
        assert!(report.events.is_empty());
        assert!(sink.callbacks.is_empty());
    }

    #[test]
    fn revise_viewport_hint_obsoletes_prior_publications_and_reorders_work() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        let first_report = job
            .render_next(2, &CancelToken::none())
            .expect("render first tiles");
        assert_eq!(first_report.completed_units, 2);
        assert!(first_report.obsolete_publications.is_empty());
        let old_identity = first_report.publication_identity;

        let hint = RenderTile {
            x: 576,
            y: 704,
            width: 32,
            height: 64,
        };
        let revise_report = job
            .revise_viewport_hint(Some(hint))
            .expect("revise visible hint");

        assert_eq!(revise_report.completed_units, 0);
        assert_eq!(revise_report.next_tile_index, 0);
        assert_eq!(job.memory_bytes_retained(), 0);
        assert_ne!(revise_report.publication_identity, old_identity);
        assert_eq!(revise_report.obsolete_publications.len(), 1);
        assert_eq!(
            revise_report.obsolete_publications[0].publication_identity,
            old_identity
        );
        assert_eq!(
            revise_report.obsolete_publications[0].reason,
            "viewport_hint_revised"
        );

        let report = job
            .render_next(1, &CancelToken::none())
            .expect("render visible tile after revision");
        let publication = &report.completed_tile_publications[0];
        assert_eq!(
            publication.priority_class,
            ProgressiveTilePriorityClass::Visible
        );
        let first = publication.tile;
        assert!(
            first.x < hint.x.saturating_add(hint.width)
                && hint.x < first.x.saturating_add(first.width)
                && first.y < hint.y.saturating_add(hint.height)
                && hint.y < first.y.saturating_add(first.height),
            "first revised tile {first:?} must intersect hint {hint:?}"
        );
        assert!(report
            .obsolete_publications
            .iter()
            .any(|publication| publication.reason == "viewport_hint_revised"));
        let report_json = serde_json::to_value(&report).expect("serialize revised step report");
        assert_eq!(
            report_json["completed_tile_publications"][0]["priority_class"],
            "visible"
        );
    }

    #[test]
    fn revise_dirty_region_only_requeues_intersecting_completed_tiles() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");

        for tile_index in [0usize, 1, 2] {
            let tile = job.tiles[tile_index];
            job.completed[tile_index] = Some(PixelBuffer::new_filled_with_mode(
                tile.width,
                tile.height,
                WHITE,
                job.render_mode,
            ));
        }
        job.next_tile_index = 3;
        job.state = ProgressiveRenderState::Rendering;
        let old_identity = job.publication_identity();
        let dirty_tile = job.tiles[1];
        let clean_left = job.tiles[0];
        let clean_right = job.tiles[2];
        let dirty_region = RenderTile {
            x: dirty_tile.x + 8,
            y: dirty_tile.y + 8,
            width: 8,
            height: 8,
        };

        let revise_report = job
            .revise_dirty_region(Some(dirty_region))
            .expect("revise dirty region");
        assert_eq!(revise_report.completed_units, 2);
        assert_eq!(revise_report.next_tile_index, 0);
        assert_ne!(revise_report.publication_identity, old_identity);
        assert!(revise_report.publication_identity.contains("generation=1"));
        assert_eq!(revise_report.completed_tiles, vec![clean_left, clean_right]);
        assert!(job.completed[0].is_some());
        assert!(job.completed[1].is_none());
        assert!(job.completed[2].is_some());
        assert_eq!(revise_report.obsolete_publications.len(), 1);
        let obsolete = &revise_report.obsolete_publications[0];
        assert_eq!(obsolete.publication_identity, old_identity);
        assert_eq!(obsolete.reason, "dirty_region_revised");
        assert_eq!(obsolete.tile, Some(dirty_tile));
        assert!(obsolete
            .tile_publication_identity
            .as_ref()
            .is_some_and(|identity| identity.contains("tile_index=1")));
        let mut stale_token = job.token();
        stale_token.scheduler_generation = 0;
        stale_token.publication_identity = old_identity;
        let err = job
            .validate_resume_token(&stale_token)
            .expect_err("stale dirty-region generation is rejected");
        assert!(format!("{err}").contains("scheduler_generation"));

        let report = job
            .render_next(1, &CancelToken::none())
            .expect("render dirty tile");
        assert_eq!(report.rendered_this_step, 1);
        assert_eq!(report.completed_units, 3);
        let dirty_publication = report
            .completed_tile_publications
            .iter()
            .find(|publication| publication.tile == dirty_tile)
            .expect("dirty tile publication is present after rerender");
        assert!(dirty_publication
            .tile_publication_identity
            .contains("generation=1"));
        let report_json = serde_json::to_value(&report).expect("serialize dirty step report");
        assert_eq!(
            report_json["obsolete_publications"][0]["tile"]["x"],
            serde_json::json!(dirty_tile.x)
        );
        assert!(report_json["completed_tile_publications"]
            .as_array()
            .expect("publication array")
            .iter()
            .any(|publication| publication["tile_publication_identity"]
                .as_str()
                .is_some_and(|identity| identity.contains("generation=1")
                    && publication["tile"]["x"] == serde_json::json!(dirty_tile.x))));
    }

    #[test]
    fn tile_publication_acceptance_rejects_viewport_stale_publications() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        let first_report = job
            .render_next(1, &CancelToken::none())
            .expect("render first tile");
        let publication = first_report.completed_tile_publications[0].clone();

        let accepted = job.evaluate_tile_publication(&publication);
        assert!(accepted.accepted);
        assert_eq!(accepted.reason, "current");

        let revise_report = job
            .revise_viewport_hint(Some(RenderTile {
                x: 576,
                y: 704,
                width: 32,
                height: 64,
            }))
            .expect("revise viewport");
        assert!(!revise_report.obsolete_publications.is_empty());

        let rejected = job.evaluate_tile_publication(&publication);
        assert!(!rejected.accepted);
        assert_eq!(rejected.reason, "obsolete_publication");
        assert_eq!(
            rejected.supplied_publication_identity,
            publication.publication_identity
        );
        assert_eq!(
            rejected.supplied_tile_publication_identity,
            publication.tile_publication_identity
        );
        let json = serde_json::to_value(&rejected).expect("serialize acceptance report");
        assert_eq!(json["accepted"], serde_json::json!(false));
        assert_eq!(json["reason"], serde_json::json!("obsolete_publication"));
    }

    #[test]
    fn revise_render_context_obsoletes_prior_publications() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        let first_report = job
            .render_next(1, &CancelToken::none())
            .expect("render first tile");
        let old_publication = first_report.completed_tile_publications[0].clone();
        let old_identity = first_report.publication_identity.clone();
        let old_contract = first_report.render_contract_fingerprint.clone();
        assert!(!old_contract.is_empty());
        assert_eq!(
            old_publication.render_contract_fingerprint,
            first_report.render_contract_fingerprint
        );

        let revision = job
            .revise_render_context(
                Some("contract:test-v2".to_string()),
                Some("ocg:view:manual=1".to_string()),
            )
            .expect("revise render context");

        assert!(revision.changed);
        assert_eq!(revision.previous_publication_identity, old_identity);
        assert_ne!(
            revision.current_publication_identity,
            revision.previous_publication_identity
        );
        assert_eq!(revision.previous_render_contract_fingerprint, old_contract);
        assert_eq!(
            revision.current_render_contract_fingerprint,
            "contract:test-v2"
        );
        assert_eq!(revision.current_visibility_fingerprint, "ocg:view:manual=1");
        assert_eq!(revision.scheduler_generation, 1);
        assert_eq!(revision.step_report.completed_units, 0);
        assert_eq!(revision.step_report.next_tile_index, 0);
        assert_eq!(
            revision.step_report.render_contract_fingerprint,
            "contract:test-v2"
        );
        assert_eq!(
            revision.step_report.visibility_fingerprint,
            "ocg:view:manual=1"
        );
        assert_eq!(revision.obsolete_publications.len(), 1);
        assert_eq!(
            revision.obsolete_publications[0].reason,
            "render_context_revised"
        );

        let stale = job.evaluate_tile_publication(&old_publication);
        assert!(!stale.accepted);
        assert_eq!(stale.reason, "obsolete_publication");

        let next_report = job
            .render_next(1, &CancelToken::none())
            .expect("render under revised context");
        let mut wrong_contract = next_report.completed_tile_publications[0].clone();
        wrong_contract.render_contract_fingerprint = "contract:wrong".to_string();
        let rejection = job.evaluate_tile_publication(&wrong_contract);
        assert!(!rejection.accepted);
        assert_eq!(rejection.reason, "render_contract_fingerprint_mismatch");

        let json = serde_json::to_value(&revision).expect("serialize revision report");
        assert_eq!(json["changed"], serde_json::json!(true));
        assert_eq!(
            json["step_report"]["render_contract_fingerprint"],
            serde_json::json!("contract:test-v2")
        );
    }

    #[test]
    fn tile_publication_acceptance_preserves_clean_dirty_region_tiles() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        let first_report = job
            .render_next(3, &CancelToken::none())
            .expect("render first three tiles");
        let clean_publication = first_report.completed_tile_publications[0].clone();
        let dirty_publication = first_report.completed_tile_publications[1].clone();
        let dirty_tile = dirty_publication.tile;

        let revise_report = job
            .revise_dirty_region(Some(RenderTile {
                x: dirty_tile.x + 4,
                y: dirty_tile.y + 4,
                width: 4,
                height: 4,
            }))
            .expect("revise dirty region");
        let revised_clean_publication = revise_report
            .completed_tile_publications
            .iter()
            .find(|publication| publication.tile == clean_publication.tile)
            .expect("clean retained tile is republished under the new identity");

        let clean_acceptance = job.evaluate_tile_publication(revised_clean_publication);
        assert!(clean_acceptance.accepted);
        assert_eq!(clean_acceptance.reason, "current");

        let dirty_rejection = job.evaluate_tile_publication(&dirty_publication);
        assert!(!dirty_rejection.accepted);
        assert_eq!(dirty_rejection.reason, "obsolete_tile_publication");
    }

    #[test]
    fn tile_publication_acceptance_rejects_malformed_current_publications() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        let report = job
            .render_next(1, &CancelToken::none())
            .expect("render first tile");
        let publication = report.completed_tile_publications[0].clone();

        let mut wrong_tile = publication.clone();
        wrong_tile.tile = RenderTile {
            x: publication.tile.x.saturating_add(1),
            y: publication.tile.y,
            width: publication.tile.width,
            height: publication.tile.height,
        };
        let rejection = job.evaluate_tile_publication(&wrong_tile);
        assert!(!rejection.accepted);
        assert_eq!(rejection.reason, "tile_identity_mismatch");
    }

    #[test]
    fn fallback_report_keeps_codes_and_structured_policy_details() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create job");
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        job.record_fallback_event(unsupported_display_list_retained_tile_refusal_event(
            tile,
            ExactnessPolicy::Compatibility,
        ));

        let report = job.step_report(0, false);
        assert_eq!(
            report.fallback_events,
            vec!["unsupported_display_list_retained_tile_refusal".to_string()]
        );
        assert_eq!(report.fallback_event_details.len(), 1);
        let event = &report.fallback_event_details[0];
        assert_eq!(event.code, "unsupported_display_list_retained_tile_refusal");
        assert_eq!(event.tile, Some(tile));
        assert!(event.standard_available);
        assert!(event.high_quality_available);
        assert!(event.call_site.contains("render_next"));
        assert!(event.replacement.contains("native retained tile plans"));
        assert_eq!(
            event.final_policy,
            "typed compatibility refusal for content outside native retained-plan support"
        );
    }

    #[test]
    fn progressive_contract_identity_tracks_exactness_independent_of_render_mode() {
        let engine = test_engine();
        let compat_contract = engine
            .default_render_contract(1, 72, RenderMode::Compat)
            .expect("default compatibility contract");
        let compat_job = ProgressiveRenderJob::new_with_contract(
            engine.clone(),
            compat_contract.clone(),
            64,
            64,
        )
        .expect("create compatibility contract job");

        let mut exact_contract = compat_contract;
        exact_contract.exactness = ExactnessPolicy::HighQualityExact;
        let exact_job = ProgressiveRenderJob::new_with_contract(engine, exact_contract, 64, 64)
            .expect("create exactness contract job");

        assert_eq!(compat_job.token().render_mode, "compat");
        assert_eq!(exact_job.token().render_mode, "compat");
        assert_ne!(
            compat_job.token().render_contract_fingerprint,
            exact_job.token().render_contract_fingerprint
        );
        assert_ne!(
            compat_job.tile_scheduler.render_contract_fingerprint,
            exact_job.tile_scheduler.render_contract_fingerprint
        );
    }

    #[test]
    fn progressive_contract_preserves_clipped_output_region() {
        let engine = test_engine();
        let mut contract = engine
            .default_render_contract(1, 72, RenderMode::Compat)
            .expect("default contract");
        contract.clip = Some(crate::render::DeviceClip {
            x: 128,
            y: 96,
            width: 96,
            height: 80,
        });
        contract.width = 96;
        contract.height = 80;
        contract.stride = 96 * crate::render::PixelFormat::Rgba8.bytes_per_pixel();

        let expected = engine
            .render_page_with_contract(&contract, &CancelToken::none())
            .expect("render clipped contract through ordinary path");
        let mut job = ProgressiveRenderJob::new_with_contract(engine, contract.clone(), 64, 64)
            .expect("create clipped progressive job");

        assert_eq!(job.page_width, 96);
        assert_eq!(job.page_height, 80);
        assert_eq!(job.output_origin_x, 128);
        assert_eq!(job.output_origin_y, 96);
        assert_eq!(
            job.tiles,
            vec![
                RenderTile {
                    x: 128,
                    y: 96,
                    width: 64,
                    height: 64,
                },
                RenderTile {
                    x: 192,
                    y: 96,
                    width: 32,
                    height: 64,
                },
                RenderTile {
                    x: 128,
                    y: 160,
                    width: 64,
                    height: 16,
                },
                RenderTile {
                    x: 192,
                    y: 160,
                    width: 32,
                    height: 16,
                },
            ]
        );
        while !job.is_complete() {
            job.render_next(2, &CancelToken::none())
                .expect("render clipped progressive tiles");
        }
        let actual = job.finish_checked().expect("assemble clipped surface");
        assert_eq!(actual.width, expected.width);
        assert_eq!(actual.height, expected.height);
        assert_eq!(actual.rgba_bytes(), expected.rgba_bytes());
        assert_eq!(
            job.token().render_contract_fingerprint,
            contract.cache_fingerprint()
        );
    }

    #[test]
    fn revise_render_contract_updates_rendering_contract_not_only_identity() {
        let engine = test_engine();
        let mut job = ProgressiveRenderJob::new(engine.clone(), 1, 72, RenderMode::Compat, 64, 64)
            .expect("create full-page progressive job");
        let first_report = job
            .render_next(1, &CancelToken::none())
            .expect("render first full-page tile");
        let old_publication = first_report.completed_tile_publications[0].clone();
        let old_contract = first_report.render_contract_fingerprint.clone();

        let mut contract = engine
            .default_render_contract(1, 72, RenderMode::Compat)
            .expect("default contract");
        contract.clip = Some(crate::render::DeviceClip {
            x: 128,
            y: 96,
            width: 96,
            height: 80,
        });
        contract.width = 96;
        contract.height = 80;
        contract.stride = 96 * crate::render::PixelFormat::Rgba8.bytes_per_pixel();
        let expected = engine
            .render_page_with_contract(&contract, &CancelToken::none())
            .expect("render revised contract through ordinary path");

        let revision = job
            .revise_render_contract(contract.clone())
            .expect("revise progressive render contract");

        assert!(revision.changed);
        assert_eq!(revision.previous_render_contract_fingerprint, old_contract);
        assert_eq!(
            revision.current_render_contract_fingerprint,
            contract.cache_fingerprint()
        );
        assert_eq!(revision.scheduler_generation, 1);
        assert_eq!(revision.step_report.total_units, 4);
        assert_eq!(revision.step_report.completed_units, 0);
        assert_eq!(revision.obsolete_publications.len(), 1);
        assert_eq!(
            revision.obsolete_publications[0].reason,
            "render_contract_revised"
        );
        assert_eq!(
            job.base_contract.cache_fingerprint(),
            contract.cache_fingerprint()
        );
        assert_eq!(job.page_width, 96);
        assert_eq!(job.page_height, 80);
        assert_eq!(job.output_origin_x, 128);
        assert_eq!(job.output_origin_y, 96);
        assert_eq!(
            job.tiles,
            vec![
                RenderTile {
                    x: 128,
                    y: 96,
                    width: 64,
                    height: 64,
                },
                RenderTile {
                    x: 192,
                    y: 96,
                    width: 32,
                    height: 64,
                },
                RenderTile {
                    x: 128,
                    y: 160,
                    width: 64,
                    height: 16,
                },
                RenderTile {
                    x: 192,
                    y: 160,
                    width: 32,
                    height: 16,
                },
            ]
        );

        let stale = job.evaluate_tile_publication(&old_publication);
        assert!(!stale.accepted);
        assert_eq!(stale.reason, "obsolete_publication");

        while !job.is_complete() {
            job.render_next(2, &CancelToken::none())
                .expect("render revised progressive tiles");
        }
        let actual = job.finish_checked().expect("assemble revised surface");
        assert_eq!(actual.width, expected.width);
        assert_eq!(actual.height, expected.height);
        assert_eq!(actual.rgba_bytes(), expected.rgba_bytes());
        assert_eq!(
            job.token().render_contract_fingerprint,
            contract.cache_fingerprint()
        );
    }

    #[test]
    fn progressive_contract_exactness_controls_retained_refusal_policy() {
        let engine = unsupported_display_list_engine();
        let mut contract = engine
            .default_render_contract(1, 72, RenderMode::Compat)
            .expect("default compatibility contract");
        contract.exactness = ExactnessPolicy::HighQualityExact;
        let mut job = ProgressiveRenderJob::new_with_contract(engine, contract, 64, 64)
            .expect("create exactness contract job");

        let error = job
            .render_next(1, &CancelToken::none())
            .expect_err("exactness contract must refuse unsupported retained replay");
        let message = error.to_string();
        assert!(message.contains("HighQualityExact"));

        let report = job.step_report(0, false);
        assert_eq!(
            report.fallback_events,
            vec!["unsupported_display_list_retained_tile_refusal".to_string()]
        );
        assert_eq!(report.fallback_event_details.len(), 1);
        let event = &report.fallback_event_details[0];
        assert!(event.standard_available);
        assert!(!event.high_quality_available);
        assert!(event.final_policy.contains("HighQualityExact"));
    }

    #[test]
    fn compat_progressive_refuses_unsupported_display_list_tile_without_immediate_fallback() {
        let engine = unsupported_display_list_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::Compat, 64, 64)
            .expect("create compatibility job");

        let err = job
            .render_next(1, &CancelToken::none())
            .expect_err("compat progressive tile must not use immediate fallback");
        assert!(matches!(
            err,
            WellfriendError::UnsupportedFeature(message)
                if message.contains("retained display-list tile replay")
                    && message.contains("unsupported retained display-list replay")
                    && message.contains("Q")
                    && !message.contains("HighQualityExact")
        ));
        assert_eq!(job.state(), ProgressiveRenderState::Failed);
        let report = job.viewer_queue_report();
        assert_eq!(
            report.fallback_events,
            vec!["unsupported_display_list_retained_tile_refusal".to_string()]
        );
        assert_eq!(report.fallback_event_details.len(), 1);
        let event = &report.fallback_event_details[0];
        assert_eq!(
            event.tile,
            Some(RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            })
        );
        assert!(event.standard_available);
        assert!(event.high_quality_available);
        assert!(event.final_policy.contains("typed compatibility refusal"));
    }

    #[test]
    fn high_quality_progressive_refuses_unsupported_display_list_tile() {
        let engine = unsupported_display_list_engine();
        let mut job = ProgressiveRenderJob::new(engine, 1, 72, RenderMode::HighQuality, 64, 64)
            .expect("create high-quality job");

        let err = job
            .render_next(1, &CancelToken::none())
            .expect_err("high-quality progressive tile must not use immediate fallback");
        assert!(matches!(
            err,
            WellfriendError::UnsupportedFeature(message)
                if message.contains("HighQualityExact")
                    && message.contains("unsupported retained display-list replay")
                    && message.contains("Q")
        ));
        assert_eq!(job.state(), ProgressiveRenderState::Failed);
        let report = job.viewer_queue_report();
        assert_eq!(
            report.fallback_events,
            vec!["unsupported_display_list_retained_tile_refusal".to_string()]
        );
        assert_eq!(report.fallback_event_details.len(), 1);
        let event = &report.fallback_event_details[0];
        assert_eq!(
            event.tile,
            Some(RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            })
        );
        assert!(event.standard_available);
        assert!(!event.high_quality_available);
        assert!(event.final_policy.contains("HighQualityExact"));
    }
}
