pub mod buffer;
pub mod clip_dag;
pub(crate) mod cmm;
pub mod color;
pub(crate) mod color_glyph;
pub mod colorspace;
pub mod contract;
pub mod display_list;
pub mod document_view;
pub mod font_rasterizer;
pub mod font_substitution_report;
pub mod function;
pub mod glyph_cache;
pub mod glyph_outline;
pub mod image_decode_planning;
pub mod image_painter;
pub mod invalidation;
pub mod line;
pub mod page_renderer;
pub mod path;
pub mod plan;
pub mod postscript;
pub(crate) mod print_profile;
pub mod progressive;
pub mod quality;
pub mod shading;
pub mod svg;
pub mod text_decode;
pub mod transaction_invalidation;
pub mod transform;
pub mod vector_fallback;

pub use buffer::{
    pixel_buffer_allocation_stats, pixel_compositor_backend,
    pixel_compositor_detected_hardware_backend, pixel_compositor_operation_backend,
    pixel_compositor_stats, rgb, rgba, AlphaMask, ClipMask, PixelBuffer,
    PixelBufferAllocationStats, PixelColor, PixelCompositorBackend, PixelCompositorOperation,
    PixelCompositorStats, RenderMode, BLACK, BLUE, GREEN, RED, TRANSPARENT, WHITE,
};
pub use clip_dag::{ClipDag, ClipDagStats, ClipNode, ClipState};
pub use color::{ColorSpaceHandler, RenderColor};
pub use contract::{
    render_contract_field_effects, AlphaMode, AnnotationRenderPolicy, BackendSelection,
    ColorManagementPolicy, ColorScheme, CompositingPolicy, ContractColor, DeterminismPolicy,
    DeviceClip, DeviceMatrix, DisplayItemId, ExactnessPolicy, ExecutionMode, FormRenderPolicy,
    HalftonePolicy, ObjectIdentityId, OptionalContentStateId, OverprintPolicy, PageBox,
    PixelFormat, PrintProfile, RenderContract, RenderContractFieldEffect, RenderResourceBudget,
    RenderingIntent, ResourceId, RevisionId, SmoothingPolicy, SourceLinkId,
    RENDER_CONTRACT_FIELD_EFFECTS, RENDER_CONTRACT_SCHEMA_VERSION,
};
pub use display_list::{
    build_display_list, build_display_list_cancellable, render_display_list, replay_display_list,
    CpuRenderDevice, DisplayList, DisplayListStats, DisplayOp, DrawState, RenderCache,
    RenderCacheKey, RenderCacheMetrics, RenderDevice, RenderTile, UnsupportedRenderOp,
};
pub use document_view::{
    BackendDocumentPagePlan, BackendDocumentPlanArena, BackendDocumentPlanArenaReport,
    BackendPlanArenaReport, CanonicalDocument, DocumentViewBoundary, DocumentViewsReport,
    EditDocumentView, ObjectIdentity, PageIdentity, ParsedPageProgram, RenderDocumentView,
    SemanticDocumentView, ValidationDocumentView, ViewMaterializationStats,
    BACKEND_DOCUMENT_PLAN_ARENA_REPORT_SCHEMA_VERSION, BACKEND_PLAN_ARENA_REPORT_SCHEMA_VERSION,
    DOCUMENT_VIEWS_REPORT_SCHEMA_VERSION,
};
pub use font_rasterizer::{get_fallback_font, FontRasterizer};
pub use font_substitution_report::{
    classify_metric_posture, fallback_font_display_name, FontSubstitutionEvent,
    FontSubstitutionLog, FontSubstitutionMetricPosture, FontSubstitutionReason,
};
pub use glyph_cache::{CachedGlyph, GlyphCache, GlyphCacheKey, GlyphCacheStats};
pub use image_decode_planning::{
    image_decode_capabilities_for_filters, image_decode_capabilities_for_image_reference,
    ImageDecodeCapabilityReport, ImageDecodeCapabilityStatus, ImageDecodeCodec,
    ImageDecodeExecutionControlStatus, ImageDecodeUnavailableReason, ImageReductionLevel,
    ImageSourceRegion, ProgressiveImageDecodeReport, ProgressiveImageDecodeRequest,
    ProgressiveImageDecodeSession, ProgressiveImageDecodeState,
};
pub use image_painter::ImagePainter;
pub use invalidation::{InvalidationResult, RenderDependencyGraph};
pub use line::{DashState, LinePainter, WuLineRenderer};
pub use page_renderer::{PageRenderer, RenderArtifactCacheStats, RenderDocumentCache};
pub use path::{
    flatten_cubic, flatten_path, path_raster_stats, FillRule, FlatPath, Path, PathPainter,
    PathRasterStats, PathSegment,
};
pub use plan::{
    GraphicsStateDescriptor, HotDisplayOp, InlineImageDescriptor, MarkedContentProperties,
    NativeDescriptor, PackedColdTables, PackedCompileRefusal, PackedDisplayList, PatternPaintPhase,
    PatternPathDescriptor, PlanDispatcher, RenderBatch, RenderPlan, RenderSpatialIndex,
};
pub use postscript::{
    assemble_eps_document, assemble_ps_document, render_page_ps, render_page_ps_strict, PsPage,
};
pub use print_profile::{PrintProfileRefusal, PrintProfileRefusalCategory};
pub use progressive::{
    choose_adaptive_tile_size, ProgressiveAdjacentPagePrefetch,
    ProgressiveAdjacentPagePrefetchExecution, ProgressiveAdjacentPagePrefetchExecutionReport,
    ProgressiveObsoletePublication, ProgressiveRenderContextRevisionReport,
    ProgressiveRenderFallbackEvent, ProgressiveRenderInvalidationReport, ProgressiveRenderJob,
    ProgressiveRenderState, ProgressiveRenderStepReport, ProgressiveRenderToken,
    ProgressiveTilePriorityClass, ProgressiveTilePublication, ProgressiveTilePublicationAcceptance,
    ProgressiveViewerCallbackDispatchReport, ProgressiveViewerCallbackEvent,
    ProgressiveViewerCallbackSink, ProgressiveViewerQueueExecutionItem,
    ProgressiveViewerQueueExecutionReport, ProgressiveViewerQueueItem,
    ProgressiveViewerQueuePriority,
};
pub use quality::RenderQuality;
pub use shading::ShadingRenderer;
pub use svg::{render_page_svg, render_page_svg_strict, SvgPage};
pub use transaction_invalidation::{
    apply_render_invalidation_plan_json_to_cache, dirty_regions_to_render_tiles,
    map_refs_to_canonical_ids, RenderInvalidationCachePlan, RenderInvalidationPlanTile,
    TransactionInvalidationResult, TransactionWriteSet,
    RENDER_TRANSACTION_INVALIDATION_PLAN_SCHEMA_VERSION,
};
pub use transform::{Transform2D, Viewport};
pub use vector_fallback::{
    classify_page_for_vector_output, image_device_rect, VectorFallbackDecision,
};
