use crate::cancel::CancelToken;
use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::{BlendMode, ColorSpace, GraphicsState, LineCap, LineJoin};
use crate::decode_scheduler::{DecodeMemoryBudget, DecodeMemoryToken};
use crate::engine::{ContentEngine, PageResources};
use crate::error::{Result, WellfriendError};
use crate::filters::DecodeLimits;
use crate::fonts::resolver::{detect_font_subtype, get_descendant_font, FontSubtype};
use crate::fonts::variations::VariationRequest;
use crate::fonts::FontResolver;
use crate::images::decoder::{ImageDecoder, RawImage};
use crate::images::locator::ImageReference;
use crate::images::SmaskLoader;
use crate::info::decode_pdf_text_string;
use crate::object::{PdfDictionary, PdfObject};
use crate::optional_content::OptionalContentContext;
use crate::prepress::{self, SeparationFramebuffer, SeparationFramebufferReport};
use crate::render::buffer::{
    AlphaMask, ClipMask, PixelBuffer, PixelColor, RenderMode, BLACK, WHITE,
};
use crate::render::clip_dag::{ClipDag, ClipNode, ClipState};
use crate::render::color::ColorSpaceHandler;
use crate::render::contract::{ObjectIdentityId, RevisionId};
use crate::render::display_list::{
    build_display_list, DisplayList, DisplayOp, RenderBounds, RenderCache, RenderCacheKey,
    RenderTile,
};
use crate::render::font_rasterizer::{get_fallback_font, FontRasterizer};
use crate::render::glyph_cache::{CachedGlyph, GlyphCache, GlyphCacheKey, GlyphCacheStats};
use crate::render::image_painter::ImagePainter;
use crate::render::invalidation::{InvalidationResult, RenderDependencyGraph};
use crate::render::line::DashState;
use crate::render::path::{
    axis_aligned_integer_rect, flatten_path, rasterize_flat_alpha_mask,
    rasterize_flat_binary_clip_mask, rasterize_glyph_alpha_mask, stroke_flat_path, FillRule,
    FlatPath, GlyphHinting, Path, PathPainter, RasterizedGlyphMask,
};
use crate::render::plan::RenderPlan;
use crate::render::shading::ShadingRenderer;
use crate::render::text_decode::{decode_text_bytes_with_resolver, DecodedGlyph};
use crate::render::transform::{Transform2D, Viewport};
use std::collections::{hash_map::DefaultHasher, HashMap, VecDeque};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

pub struct PageRenderer;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderArtifactCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub skipped_oversized: u64,
    pub entries: usize,
    pub bytes: usize,
}

fn merge_artifact_cache_counts(dst: &mut RenderArtifactCacheStats, src: RenderArtifactCacheStats) {
    dst.hits = dst.hits.saturating_add(src.hits);
    dst.misses = dst.misses.saturating_add(src.misses);
    dst.evictions = dst.evictions.saturating_add(src.evictions);
    dst.skipped_oversized = dst.skipped_oversized.saturating_add(src.skipped_oversized);
}

/// Per-document renderer scratch reused across sequential page renders.
///
/// The normal `render_page*` entry points keep the historical per-page cache
/// behavior. Callers that render many pages from one already-open document can
/// pass this cache to the `*_with_document_cache` variants to reuse font bytes,
/// font resolvers, glyph outlines/masks, Type3 glyph geometry/masks, path
/// raster masks, image/shading/Form/pattern programs, retained display lists,
/// and bounded offscreen buffers without changing rendering semantics. The
/// cache is intentionally `&mut` rather than shared behind locks: a caller may
/// keep one cache per document worker, while parallel rendering uses separate
/// caches and remains deterministic.
pub struct RenderDocumentCache {
    glyph_cache: GlyphCache,
    glyph_mask_cache: GlyphMaskCache,
    type3_mask_cache: Type3MaskCache,
    type3_rendered_cache: Type3RenderedGlyphCache,
    path_fill_mask_cache: PathFillMaskCache,
    path_stroke_mask_cache: PathStrokeMaskCache,
    font_bytes_cache: HashMap<String, Option<Arc<Vec<u8>>>>,
    font_bytes_cache_stats: RenderArtifactCacheStats,
    font_resolver_cache: HashMap<String, Arc<FontResolver>>,
    font_resolver_cache_stats: RenderArtifactCacheStats,
    type3_geometry_cache: HashMap<String, Option<Arc<Type3GlyphGeometry>>>,
    type3_charproc_cache: HashMap<String, Option<Arc<Type3CharProc>>>,
    image_xobject_cache: HashMap<String, Arc<RawImage>>,
    image_xobject_cache_order: VecDeque<String>,
    image_xobject_cache_bytes: usize,
    image_xobject_cache_stats: RenderArtifactCacheStats,
    scaled_image_cache: HashMap<String, Arc<RawImage>>,
    scaled_image_cache_order: VecDeque<String>,
    scaled_image_cache_bytes: usize,
    scaled_image_cache_stats: RenderArtifactCacheStats,
    smask_group_cache: HashMap<String, Arc<AlphaMask>>,
    smask_group_cache_order: VecDeque<String>,
    smask_group_cache_bytes: usize,
    smask_group_cache_stats: RenderArtifactCacheStats,
    shading_mesh_cache: HashMap<String, Arc<Vec<u8>>>,
    shading_mesh_cache_order: VecDeque<String>,
    shading_mesh_cache_bytes: usize,
    shading_mesh_cache_stats: RenderArtifactCacheStats,
    form_xobject_program_cache: HashMap<String, Option<Arc<FormXObjectProgram>>>,
    form_xobject_program_cache_stats: RenderArtifactCacheStats,
    tiling_pattern_program_cache: HashMap<String, Option<Arc<Vec<ContentOperation>>>>,
    tiling_pattern_program_cache_stats: RenderArtifactCacheStats,
    offscreen_buffer_pool: Vec<PixelBuffer>,
    display_list_cache: HashMap<String, Arc<DisplayList>>,
    display_list_raster_cache: RenderCache,
    transparent_page_group_cache: HashMap<String, bool>,
    document_revision: Option<RevisionId>,
    dependency_graph: Option<RenderDependencyGraph>,
}

impl RenderDocumentCache {
    pub fn new() -> Self {
        Self {
            glyph_cache: GlyphCache::with_default_capacity(),
            glyph_mask_cache: GlyphMaskCache::default(),
            type3_mask_cache: Type3MaskCache::default(),
            type3_rendered_cache: Type3RenderedGlyphCache::default(),
            path_fill_mask_cache: PathFillMaskCache::default(),
            path_stroke_mask_cache: PathStrokeMaskCache::default(),
            font_bytes_cache: HashMap::new(),
            font_bytes_cache_stats: RenderArtifactCacheStats::default(),
            font_resolver_cache: HashMap::new(),
            font_resolver_cache_stats: RenderArtifactCacheStats::default(),
            type3_geometry_cache: HashMap::new(),
            type3_charproc_cache: HashMap::new(),
            image_xobject_cache: HashMap::new(),
            image_xobject_cache_order: VecDeque::new(),
            image_xobject_cache_bytes: 0,
            image_xobject_cache_stats: RenderArtifactCacheStats::default(),
            scaled_image_cache: HashMap::new(),
            scaled_image_cache_order: VecDeque::new(),
            scaled_image_cache_bytes: 0,
            scaled_image_cache_stats: RenderArtifactCacheStats::default(),
            smask_group_cache: HashMap::new(),
            smask_group_cache_order: VecDeque::new(),
            smask_group_cache_bytes: 0,
            smask_group_cache_stats: RenderArtifactCacheStats::default(),
            shading_mesh_cache: HashMap::new(),
            shading_mesh_cache_order: VecDeque::new(),
            shading_mesh_cache_bytes: 0,
            shading_mesh_cache_stats: RenderArtifactCacheStats::default(),
            form_xobject_program_cache: HashMap::new(),
            form_xobject_program_cache_stats: RenderArtifactCacheStats::default(),
            tiling_pattern_program_cache: HashMap::new(),
            tiling_pattern_program_cache_stats: RenderArtifactCacheStats::default(),
            offscreen_buffer_pool: Vec::new(),
            display_list_cache: HashMap::new(),
            display_list_raster_cache: RenderCache::new(256 * 1024 * 1024, 64 * 1024 * 1024),
            transparent_page_group_cache: HashMap::new(),
            document_revision: None,
            dependency_graph: None,
        }
    }

    pub fn clear(&mut self) {
        self.glyph_cache.clear();
        self.glyph_mask_cache.clear();
        self.type3_mask_cache.clear();
        self.type3_rendered_cache.clear();
        self.path_fill_mask_cache.clear();
        self.path_stroke_mask_cache.clear();
        self.font_bytes_cache.clear();
        self.font_bytes_cache_stats = RenderArtifactCacheStats::default();
        self.font_resolver_cache.clear();
        self.font_resolver_cache_stats = RenderArtifactCacheStats::default();
        self.type3_geometry_cache.clear();
        self.type3_charproc_cache.clear();
        self.image_xobject_cache.clear();
        self.image_xobject_cache_order.clear();
        self.image_xobject_cache_bytes = 0;
        self.image_xobject_cache_stats = RenderArtifactCacheStats::default();
        self.scaled_image_cache.clear();
        self.scaled_image_cache_order.clear();
        self.scaled_image_cache_bytes = 0;
        self.scaled_image_cache_stats = RenderArtifactCacheStats::default();
        self.smask_group_cache.clear();
        self.smask_group_cache_order.clear();
        self.smask_group_cache_bytes = 0;
        self.smask_group_cache_stats = RenderArtifactCacheStats::default();
        self.shading_mesh_cache.clear();
        self.shading_mesh_cache_order.clear();
        self.shading_mesh_cache_bytes = 0;
        self.shading_mesh_cache_stats = RenderArtifactCacheStats::default();
        self.form_xobject_program_cache.clear();
        self.form_xobject_program_cache_stats = RenderArtifactCacheStats::default();
        self.tiling_pattern_program_cache.clear();
        self.tiling_pattern_program_cache_stats = RenderArtifactCacheStats::default();
        self.offscreen_buffer_pool.clear();
        self.display_list_cache.clear();
        self.display_list_raster_cache = RenderCache::new(256 * 1024 * 1024, 64 * 1024 * 1024);
        self.transparent_page_group_cache.clear();
        self.document_revision = None;
        self.dependency_graph = None;
    }

    fn trim_string_cache<V>(cache: &mut HashMap<String, V>, max_entries: usize) -> usize {
        if cache.len() <= max_entries {
            return 0;
        }
        let mut keys: Vec<_> = cache.keys().cloned().collect();
        keys.sort_unstable();
        let mut removed = 0;
        for key in keys
            .into_iter()
            .take(cache.len().saturating_sub(max_entries))
        {
            if cache.remove(&key).is_some() {
                removed += 1;
            }
        }
        removed
    }

    fn enforce_bounded_maps(&mut self) {
        const FONT_BYTES_BUDGET: usize = 64 * 1024 * 1024;
        const FONT_RESOLVER_ENTRIES: usize = 2_048;
        const TYPE3_PROGRAM_ENTRIES: usize = 1_024;
        const FORM_PROGRAM_ENTRIES: usize = 1_024;
        const PATTERN_PROGRAM_ENTRIES: usize = 1_024;
        const DISPLAY_LIST_ENTRIES: usize = 512;
        const TRANSPARENT_PAGE_ENTRIES: usize = 512;
        const OFFSCREEN_POOL_BYTES: usize = 64 * 1024 * 1024;

        while font_bytes_cache_bytes(&self.font_bytes_cache) > FONT_BYTES_BUDGET {
            let before = self.font_bytes_cache.len();
            Self::trim_string_cache(&mut self.font_bytes_cache, before.saturating_sub(1));
            if self.font_bytes_cache.len() == before {
                break;
            }
            self.font_bytes_cache_stats.evictions =
                self.font_bytes_cache_stats.evictions.saturating_add(1);
        }
        self.font_resolver_cache_stats.evictions = self
            .font_resolver_cache_stats
            .evictions
            .saturating_add(Self::trim_string_cache(
                &mut self.font_resolver_cache,
                FONT_RESOLVER_ENTRIES,
            ) as u64);
        Self::trim_string_cache(&mut self.type3_geometry_cache, TYPE3_PROGRAM_ENTRIES);
        Self::trim_string_cache(&mut self.type3_charproc_cache, TYPE3_PROGRAM_ENTRIES);
        self.form_xobject_program_cache_stats.evictions = self
            .form_xobject_program_cache_stats
            .evictions
            .saturating_add(Self::trim_string_cache(
                &mut self.form_xobject_program_cache,
                FORM_PROGRAM_ENTRIES,
            ) as u64);
        self.tiling_pattern_program_cache_stats.evictions = self
            .tiling_pattern_program_cache_stats
            .evictions
            .saturating_add(Self::trim_string_cache(
                &mut self.tiling_pattern_program_cache,
                PATTERN_PROGRAM_ENTRIES,
            ) as u64);
        Self::trim_string_cache(
            &mut self.transparent_page_group_cache,
            TRANSPARENT_PAGE_ENTRIES,
        );

        while self.display_list_cache.len() > DISPLAY_LIST_ENTRIES
            || self
                .display_list_cache
                .values()
                .map(|list| list.approximate_memory_bytes())
                .sum::<usize>()
                > 128 * 1024 * 1024
        {
            let before = self.display_list_cache.len();
            Self::trim_string_cache(&mut self.display_list_cache, before.saturating_sub(1));
            if self.display_list_cache.len() == before {
                break;
            }
        }
        while self.offscreen_buffer_pool_bytes() > OFFSCREEN_POOL_BYTES {
            if self.offscreen_buffer_pool.pop().is_none() {
                break;
            }
        }
    }

    pub fn bind_document_revision(&mut self, revision: RevisionId) {
        if self.document_revision == Some(revision) {
            return;
        }
        self.clear();
        self.document_revision = Some(revision);
        self.dependency_graph = Some(RenderDependencyGraph::new(revision));
    }

    pub fn document_revision(&self) -> Option<RevisionId> {
        self.document_revision
    }

    pub fn record_page_source_dependency(&mut self, page_number: usize, source: ObjectIdentityId) {
        if let Some(graph) = &mut self.dependency_graph {
            graph.record_page_source(page_number, source);
        }
    }

    pub fn record_tile_dependency(&mut self, page_number: usize, tile: RenderTile) {
        if let Some(graph) = &mut self.dependency_graph {
            graph.record_tile(page_number, tile);
        }
    }

    fn invalidate_page_artifacts(&mut self, pages: &[usize]) {
        self.display_list_cache.retain(|key, _| {
            !pages
                .iter()
                .any(|page| key.starts_with(&format!("page:{page}:")))
        });
        self.transparent_page_group_cache.retain(|key, _| {
            !pages
                .iter()
                .any(|page| key.starts_with(&format!("page:{page}:")))
        });
        self.display_list_raster_cache.invalidate_pages(pages);
    }

    pub fn invalidate_sources(
        &mut self,
        next_revision: RevisionId,
        changed_sources: &[ObjectIdentityId],
    ) -> InvalidationResult {
        let mut graph = self.dependency_graph.take().unwrap_or_else(|| {
            RenderDependencyGraph::new(self.document_revision.unwrap_or(next_revision))
        });
        let result = graph.invalidate_sources(next_revision, changed_sources);
        if result.cache_must_reset {
            self.clear();
            self.document_revision = Some(next_revision);
            self.dependency_graph = Some(RenderDependencyGraph::new(next_revision));
        } else {
            self.invalidate_page_artifacts(&result.invalidated_pages);
            self.document_revision = Some(next_revision);
            graph.reset_revision(next_revision);
            self.dependency_graph = Some(graph);
        }
        result
    }

    pub fn glyph_entries(&self) -> usize {
        self.glyph_cache.len()
    }

    pub fn glyph_cache_stats(&self) -> GlyphCacheStats {
        self.glyph_cache.stats()
    }

    pub fn glyph_mask_entries(&self) -> usize {
        self.glyph_mask_cache.len()
    }

    pub fn glyph_mask_bytes(&self) -> usize {
        self.glyph_mask_cache.bytes()
    }

    pub fn glyph_mask_cache_stats(&self) -> RenderArtifactCacheStats {
        self.glyph_mask_cache.stats()
    }

    pub fn type3_mask_entries(&self) -> usize {
        self.type3_mask_cache.len()
    }

    pub fn type3_mask_bytes(&self) -> usize {
        self.type3_mask_cache.bytes()
    }

    pub fn type3_mask_cache_stats(&self) -> RenderArtifactCacheStats {
        self.type3_mask_cache.stats()
    }

    pub fn type3_rendered_entries(&self) -> usize {
        self.type3_rendered_cache.len()
    }

    pub fn type3_rendered_bytes(&self) -> usize {
        self.type3_rendered_cache.bytes()
    }

    pub fn type3_rendered_cache_stats(&self) -> RenderArtifactCacheStats {
        self.type3_rendered_cache.stats()
    }

    pub fn path_fill_mask_entries(&self) -> usize {
        self.path_fill_mask_cache.len()
    }

    pub fn path_fill_mask_bytes(&self) -> usize {
        self.path_fill_mask_cache.bytes()
    }

    pub fn path_fill_mask_cache_stats(&self) -> RenderArtifactCacheStats {
        self.path_fill_mask_cache.stats()
    }

    pub fn path_stroke_mask_entries(&self) -> usize {
        self.path_stroke_mask_cache.len()
    }

    pub fn path_stroke_mask_bytes(&self) -> usize {
        self.path_stroke_mask_cache.bytes()
    }

    pub fn path_stroke_mask_cache_stats(&self) -> RenderArtifactCacheStats {
        self.path_stroke_mask_cache.stats()
    }

    pub fn font_byte_entries(&self) -> usize {
        self.font_bytes_cache.len()
    }

    pub fn font_bytes_cache_stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.font_bytes_cache.len(),
            bytes: font_bytes_cache_bytes(&self.font_bytes_cache),
            ..self.font_bytes_cache_stats
        }
    }

    pub fn font_resolver_entries(&self) -> usize {
        self.font_resolver_cache.len()
    }

    pub fn font_resolver_cache_stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.font_resolver_cache.len(),
            bytes: self
                .font_resolver_cache
                .len()
                .saturating_mul(std::mem::size_of::<FontResolver>()),
            ..self.font_resolver_cache_stats
        }
    }

    pub fn display_list_entries(&self) -> usize {
        self.display_list_cache.len()
    }

    pub fn image_xobject_entries(&self) -> usize {
        self.image_xobject_cache.len()
    }

    pub fn image_xobject_bytes(&self) -> usize {
        self.image_xobject_cache_bytes
    }

    pub fn image_xobject_cache_stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.image_xobject_cache.len(),
            bytes: self.image_xobject_cache_bytes,
            ..self.image_xobject_cache_stats
        }
    }

    pub fn scaled_image_entries(&self) -> usize {
        self.scaled_image_cache.len()
    }

    pub fn scaled_image_bytes(&self) -> usize {
        self.scaled_image_cache_bytes
    }

    pub fn scaled_image_cache_stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.scaled_image_cache.len(),
            bytes: self.scaled_image_cache_bytes,
            ..self.scaled_image_cache_stats
        }
    }

    pub fn smask_group_entries(&self) -> usize {
        self.smask_group_cache.len()
    }

    pub fn smask_group_cache_stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.smask_group_cache.len(),
            bytes: self.smask_group_cache_bytes,
            ..self.smask_group_cache_stats
        }
    }

    pub fn shading_mesh_entries(&self) -> usize {
        self.shading_mesh_cache.len()
    }

    pub fn shading_mesh_bytes(&self) -> usize {
        self.shading_mesh_cache_bytes
    }

    pub fn shading_mesh_cache_stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.shading_mesh_cache.len(),
            bytes: self.shading_mesh_cache_bytes,
            ..self.shading_mesh_cache_stats
        }
    }

    pub fn form_xobject_program_entries(&self) -> usize {
        self.form_xobject_program_cache.len()
    }

    pub fn form_xobject_program_cache_stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.form_xobject_program_cache.len(),
            bytes: 0,
            ..self.form_xobject_program_cache_stats
        }
    }

    pub fn tiling_pattern_program_entries(&self) -> usize {
        self.tiling_pattern_program_cache.len()
    }

    pub fn tiling_pattern_program_cache_stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.tiling_pattern_program_cache.len(),
            bytes: 0,
            ..self.tiling_pattern_program_cache_stats
        }
    }

    pub fn offscreen_buffer_pool_entries(&self) -> usize {
        self.offscreen_buffer_pool.len()
    }

    pub fn offscreen_buffer_pool_bytes(&self) -> usize {
        self.offscreen_buffer_pool
            .iter()
            .map(|buf| {
                (buf.width as usize)
                    .saturating_mul(buf.height as usize)
                    .saturating_mul(4)
            })
            .sum()
    }

    pub fn transparent_page_group_entries(&self) -> usize {
        self.transparent_page_group_cache.len()
    }

    pub fn display_list_raster_cache_metrics(
        &self,
    ) -> crate::render::display_list::RenderCacheMetrics {
        self.display_list_raster_cache.metrics()
    }

    pub(crate) fn display_list_key_with_revision(
        page_number: usize,
        dpi: u32,
        revision: impl AsRef<str>,
    ) -> String {
        format!("page:{page_number}:dpi:{dpi}:{}", revision.as_ref())
    }

    pub(crate) fn transparent_page_group_key_with_revision(
        page_number: usize,
        revision: impl AsRef<str>,
    ) -> String {
        format!("page:{page_number}:{}", revision.as_ref())
    }

    pub(crate) fn cached_display_list(&self, key: &str) -> Option<Arc<DisplayList>> {
        self.display_list_cache.get(key).cloned()
    }

    pub(crate) fn insert_display_list(
        &mut self,
        key: String,
        list: DisplayList,
    ) -> Arc<DisplayList> {
        let list = Arc::new(list);
        self.display_list_cache.insert(key, Arc::clone(&list));
        list
    }

    pub(crate) fn cached_transparent_page_group(&self, key: &str) -> Option<bool> {
        self.transparent_page_group_cache.get(key).copied()
    }

    pub(crate) fn insert_transparent_page_group(&mut self, key: String, value: bool) {
        self.transparent_page_group_cache.insert(key, value);
    }

    pub(crate) fn cached_display_list_raster(
        &mut self,
        key: &RenderCacheKey,
    ) -> Option<PixelBuffer> {
        self.display_list_raster_cache.get(key)
    }

    pub(crate) fn cached_display_list_raster_ref(
        &mut self,
        key: &RenderCacheKey,
    ) -> Option<&PixelBuffer> {
        self.display_list_raster_cache.get_ref(key)
    }

    pub(crate) fn insert_display_list_raster(&mut self, key: RenderCacheKey, buffer: PixelBuffer) {
        self.record_tile_dependency(key.page_number, key.tile);
        self.display_list_raster_cache.insert(key, buffer);
    }
}

impl Default for RenderDocumentCache {
    fn default() -> Self {
        Self::new()
    }
}

fn font_bytes_cache_bytes(cache: &HashMap<String, Option<Arc<Vec<u8>>>>) -> usize {
    cache
        .values()
        .filter_map(|bytes| bytes.as_ref())
        .map(|bytes| {
            std::mem::size_of::<Arc<Vec<u8>>>()
                .saturating_add(std::mem::size_of::<Vec<u8>>())
                .saturating_add(bytes.len())
        })
        .sum()
}

impl PageRenderer {
    fn contract_cache_key(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile: RenderTile,
        visibility_fingerprint: impl Into<String>,
        prepress_fingerprint: impl Into<String>,
    ) -> Result<RenderCacheKey> {
        let contract = engine.render_contract_for_tile(page_number, dpi, render_mode, tile)?;
        Ok(RenderCacheKey::new_with_full_identity(
            page_number,
            dpi,
            render_mode,
            tile,
            visibility_fingerprint,
            prepress_fingerprint,
            format!("{:016x}", contract.document_revision.0),
            contract.cache_fingerprint(),
        ))
    }

    fn revision_cache_key(engine: &ContentEngine) -> String {
        format!("revision:{:016x}", engine.canonical_document().revision().0)
    }

    fn render_packed_vector_plan(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        list: &DisplayList,
        render_mode: RenderMode,
    ) -> Result<PixelBuffer> {
        let contract = engine.default_render_contract(page_number, dpi, render_mode)?;
        let plan = RenderPlan::compile(list.clone(), contract)?;
        plan.execute_vector_tile(RenderTile::full(
            list.viewport.width_px,
            list.viewport.height_px,
        ))?
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "packed vector plan unexpectedly retained a native payload".to_string(),
            )
        })
    }

    /// Return a retained page display list from the caller's document cache, or
    /// build and retain it exactly once for subsequent warm replay.
    pub fn get_or_build_display_list_with_cache(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        cache: &mut RenderDocumentCache,
    ) -> Result<(Arc<DisplayList>, bool)> {
        cache.bind_document_revision(engine.canonical_document().revision());
        let page = engine.get_page(page_number)?;
        cache.record_page_source_dependency(
            page_number,
            engine
                .canonical_document()
                .page_identity_for(&page)
                .object
                .id,
        );
        let key = RenderDocumentCache::display_list_key_with_revision(
            page_number,
            dpi,
            Self::revision_cache_key(engine),
        );
        if let Some(list) = cache.cached_display_list(&key) {
            return Ok((list, true));
        }
        let list = Self::build_display_list(engine, page_number, dpi)?;
        Ok((cache.insert_display_list(key, list), false))
    }

    /// Render a single PDF page to a PixelBuffer at the given DPI.
    pub fn render_page(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
    ) -> Result<PixelBuffer> {
        Self::render_page_cancellable(engine, page_number, dpi, &CancelToken::none())
    }

    /// Render a single PDF page to a PixelBuffer with an explicit render mode.
    pub fn render_page_with_mode(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
    ) -> Result<PixelBuffer> {
        Self::render_page_cancellable_with_mode(
            engine,
            page_number,
            dpi,
            &CancelToken::none(),
            render_mode,
        )
    }

    /// Render a page, polling `cancel` periodically so a runaway content
    /// stream can be stopped from outside (e.g. a request-timeout timer).
    pub fn render_page_cancellable(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        cancel: &CancelToken,
    ) -> Result<PixelBuffer> {
        Self::render_page_cancellable_with_mode(
            engine,
            page_number,
            dpi,
            cancel,
            RenderMode::Compat,
        )
    }

    /// Render a page with cancellation and an explicit render mode.
    pub fn render_page_cancellable_with_mode(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        cancel: &CancelToken,
        render_mode: RenderMode,
    ) -> Result<PixelBuffer> {
        let ops = engine.get_page_content(page_number)?;
        let viewport = engine.page_viewport(page_number, dpi)?;
        let resources = engine.get_page_resources(page_number)?;
        let transparent_page_group = uses_top_level_transparency(&ops, &resources, engine);
        let buf = Self::initial_page_buffer(&viewport, transparent_page_group, render_mode);

        let mut state = RenderState::new(buf, viewport, resources, engine, page_number);
        state.cancel = cancel.clone();
        state.dispatch_all(&ops);
        state.check_fatal_render_error()?;
        // dispatch_all bails out early (without error) when the token trips;
        // surface that as a distinct error so the caller returns a timeout
        // response rather than a half-rendered page presented as success.
        cancel.check("page render")?;
        state.render_page_annotations();
        cancel.check("page annotation render")?;
        let mut buf = state.into_buffer();
        if transparent_page_group {
            buf.flatten_onto_background(WHITE);
        }
        Ok(buf)
    }

    /// Render a page with reusable per-document caches.
    pub fn render_page_cancellable_with_mode_and_cache(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        cancel: &CancelToken,
        render_mode: RenderMode,
        cache: &mut RenderDocumentCache,
    ) -> Result<PixelBuffer> {
        let (list, _) =
            Self::get_or_build_display_list_with_cache(engine, page_number, dpi, cache)?;
        if list.is_fully_supported() {
            return Self::render_display_list_cancellable_with_mode_and_cache(
                engine,
                page_number,
                dpi,
                list.as_ref(),
                cancel,
                render_mode,
                cache,
            );
        }

        let ops = engine.get_page_content(page_number)?;
        let viewport = engine.page_viewport(page_number, dpi)?;
        let resources = engine.get_page_resources(page_number)?;
        let transparent_page_group = uses_top_level_transparency(&ops, &resources, engine);
        let buf = Self::initial_page_buffer(&viewport, transparent_page_group, render_mode);

        let mut state = RenderState::new_with_document_cache(
            buf,
            viewport,
            resources,
            engine,
            page_number,
            cache,
        );
        state.cancel = cancel.clone();
        state.dispatch_all(&ops);
        if let Err(err) = state.check_fatal_render_error() {
            state.return_document_cache(cache);
            return Err(err);
        }
        if let Err(err) = cancel.check("page render") {
            state.return_document_cache(cache);
            return Err(err);
        }
        state.render_page_annotations();
        if let Err(err) = cancel.check("page annotation render") {
            state.return_document_cache(cache);
            return Err(err);
        }
        let mut buf = state.into_buffer_and_document_cache(cache);
        if transparent_page_group {
            buf.flatten_onto_background(WHITE);
        }
        Ok(buf)
    }

    /// Build the native retained display list for a page.
    ///
    /// The list keeps high-level text, image, shading, pattern, inline-image,
    /// Form XObject, optional-content, clip, and graphics-state operations as
    /// typed replay ops. Unsupported status is reserved for malformed or missing
    /// source evidence, not for normal PDF graphics features.
    pub fn build_display_list(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
    ) -> Result<DisplayList> {
        let ops = engine.get_page_content(page_number)?;
        let viewport = engine.page_viewport(page_number, dpi)?;
        let resources = engine.get_page_resources(page_number)?;
        Ok(build_display_list(&ops, viewport, &resources))
    }

    /// Render a page through display-list replay.
    ///
    /// Vector-only lists replay through the normalized CPU device. Pages with
    /// higher-level text/image/XObject/shading/pattern content replay through
    /// typed native ops so retained replay and cache hits use the same canonical
    /// source renderer without page-wide immediate fallback. `Ok(None)` is
    /// reserved for an explicitly unsupported display-list diagnostic.
    pub fn render_page_display_list_with_mode(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
    ) -> Result<Option<PixelBuffer>> {
        let list = Self::build_display_list(engine, page_number, dpi)?;
        if !list.is_fully_supported() {
            Ok(None)
        } else if list.native_vector_only() {
            let mut buf =
                Self::render_packed_vector_plan(engine, page_number, dpi, &list, render_mode)?;
            Self::render_annotations_into(engine, page_number, dpi, &mut buf)?;
            Ok(Some(buf))
        } else {
            Ok(Some(Self::render_display_list_cancellable_with_mode(
                engine,
                page_number,
                dpi,
                &list,
                &CancelToken::none(),
                render_mode,
            )?))
        }
    }

    /// Replay an already-built display list with cancellation.
    pub fn render_display_list_cancellable_with_mode(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        list: &DisplayList,
        cancel: &CancelToken,
        render_mode: RenderMode,
    ) -> Result<PixelBuffer> {
        cancel.check("display-list render start")?;
        if list.native_vector_only() {
            let mut buf =
                Self::render_packed_vector_plan(engine, page_number, dpi, list, render_mode)?;
            cancel.check("display-list native vector replay")?;
            Self::render_annotations_into(engine, page_number, dpi, &mut buf)?;
            cancel.check("display-list annotation render")?;
            return Ok(buf);
        }
        let viewport = engine.page_viewport(page_number, dpi)?;
        let resources = engine.get_page_resources(page_number)?;
        let transparent_page_group =
            display_list_needs_transparent_page_group(engine, page_number, &resources, list)?;
        let buf = Self::initial_page_buffer(&viewport, transparent_page_group, render_mode);
        let mut state = RenderState::new(buf, viewport, resources, engine, page_number);
        state.cancel = cancel.clone();
        state.replay_display_list(list);
        state.check_fatal_render_error()?;
        cancel.check("display-list replay")?;
        state.render_page_annotations();
        cancel.check("display-list annotation render")?;
        let mut buf = state.into_buffer();
        if transparent_page_group {
            buf.flatten_onto_background(WHITE);
        }
        Ok(buf)
    }

    /// Replay a display list with reusable per-document caches.
    pub fn render_display_list_cancellable_with_mode_and_cache(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        list: &DisplayList,
        cancel: &CancelToken,
        render_mode: RenderMode,
        cache: &mut RenderDocumentCache,
    ) -> Result<PixelBuffer> {
        cancel.check("display-list render start")?;
        let resources = engine.get_page_resources(page_number)?;
        let visibility_fingerprint = OptionalContentContext::from_document(engine.document())
            .visibility_fingerprint()
            .to_string();
        let prepress_fingerprint = prepress::cache_fingerprint_for_prepress_resources(
            resources.color_spaces.values(),
            resources.ext_g_states.values(),
        );
        let raster_key = Self::contract_cache_key(
            engine,
            page_number,
            dpi,
            render_mode,
            RenderTile::full(list.viewport.width_px, list.viewport.height_px),
            visibility_fingerprint,
            prepress_fingerprint,
        )?;
        if let Some(hit) = cache.cached_display_list_raster(&raster_key) {
            return Ok(hit);
        }

        if list.native_vector_only() {
            let mut buf =
                Self::render_packed_vector_plan(engine, page_number, dpi, list, render_mode)?;
            cancel.check("display-list native vector replay")?;
            Self::render_annotations_into(engine, page_number, dpi, &mut buf)?;
            cancel.check("display-list annotation render")?;
            cache.insert_display_list_raster(raster_key, buf.clone());
            return Ok(buf);
        }
        let viewport = engine.page_viewport(page_number, dpi)?;
        let transparent_key = RenderDocumentCache::transparent_page_group_key_with_revision(
            page_number,
            Self::revision_cache_key(engine),
        );
        let transparent_page_group = match cache.cached_transparent_page_group(&transparent_key) {
            Some(value) => value,
            None => {
                let value = display_list_needs_transparent_page_group(
                    engine,
                    page_number,
                    &resources,
                    list,
                )?;
                cache.insert_transparent_page_group(transparent_key, value);
                value
            }
        };
        let buf = Self::initial_page_buffer(&viewport, transparent_page_group, render_mode);
        let mut state = RenderState::new_with_document_cache(
            buf,
            viewport,
            resources,
            engine,
            page_number,
            cache,
        );
        state.cancel = cancel.clone();
        state.replay_display_list(list);
        if let Err(err) = state.check_fatal_render_error() {
            state.return_document_cache(cache);
            return Err(err);
        }
        if let Err(err) = cancel.check("display-list replay") {
            state.return_document_cache(cache);
            return Err(err);
        }
        state.render_page_annotations();
        if let Err(err) = cancel.check("display-list annotation render") {
            state.return_document_cache(cache);
            return Err(err);
        }
        let mut buf = state.into_buffer_and_document_cache(cache);
        if transparent_page_group {
            buf.flatten_onto_background(WHITE);
        }
        cache.insert_display_list_raster(raster_key, buf.clone());
        Ok(buf)
    }

    /// Replay a cached full-page display list into one pixel-space tile.
    ///
    /// The display list is built against the full-page viewport, while replay
    /// uses the tile-local viewport. Retained full-page bounds on vector ops let
    /// replay skip non-intersecting paths before flattening/rasterization.
    pub fn render_page_display_list_tile_cancellable_with_mode_and_cache(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        tile: RenderTile,
        cancel: &CancelToken,
        render_mode: RenderMode,
        cache: &mut RenderDocumentCache,
    ) -> Result<Option<PixelBuffer>> {
        cancel.check("display-list tile render start")?;
        cache.bind_document_revision(engine.canonical_document().revision());
        let page = engine.get_page(page_number)?;
        cache.record_page_source_dependency(
            page_number,
            engine
                .canonical_document()
                .page_identity_for(&page)
                .object
                .id,
        );
        let key = RenderDocumentCache::display_list_key_with_revision(
            page_number,
            dpi,
            Self::revision_cache_key(engine),
        );
        let list = match cache.cached_display_list(&key) {
            Some(list) => list,
            None => {
                let list = Self::build_display_list(engine, page_number, dpi)?;
                cache.insert_display_list(key, list)
            }
        };
        if !list.is_fully_supported() {
            return Ok(None);
        }

        let full_viewport = engine.page_viewport(page_number, dpi)?;
        if tile.width == 0 || tile.height == 0 {
            return Err(WellfriendError::invalid_input(
                "render tile must have non-zero width and height",
            ));
        }
        if tile.x >= full_viewport.width_px || tile.y >= full_viewport.height_px {
            return Err(WellfriendError::invalid_input(format!(
                "render tile origin ({},{}) is outside page bounds {}x{}",
                tile.x, tile.y, full_viewport.width_px, full_viewport.height_px
            )));
        }
        let end_x = tile.x.checked_add(tile.width).ok_or_else(|| {
            WellfriendError::invalid_input("render tile x range overflows".to_string())
        })?;
        let end_y = tile.y.checked_add(tile.height).ok_or_else(|| {
            WellfriendError::invalid_input("render tile y range overflows".to_string())
        })?;
        if end_x > full_viewport.width_px || end_y > full_viewport.height_px {
            return Err(WellfriendError::invalid_input(format!(
                "render tile {}x{} at {},{} exceeds page bounds {}x{}",
                tile.width,
                tile.height,
                tile.x,
                tile.y,
                full_viewport.width_px,
                full_viewport.height_px
            )));
        }

        const TILE_OVERDRAW_PX: u32 = 2;
        let expanded_tile = expand_render_tile(
            tile,
            full_viewport.width_px,
            full_viewport.height_px,
            TILE_OVERDRAW_PX,
        );
        let viewport = full_viewport.pixel_window(
            expanded_tile.x,
            expanded_tile.y,
            expanded_tile.width,
            expanded_tile.height,
        );
        if viewport.width_px == 0 || viewport.height_px == 0 {
            return Err(WellfriendError::invalid_input(
                "render tile is empty after clipping to page bounds",
            ));
        }

        let resources = engine.get_page_resources(page_number)?;
        let visibility_fingerprint = OptionalContentContext::from_document(engine.document())
            .visibility_fingerprint()
            .to_string();
        let prepress_fingerprint = prepress::cache_fingerprint_for_prepress_resources(
            resources.color_spaces.values(),
            resources.ext_g_states.values(),
        );
        let full_tile = RenderTile::full(full_viewport.width_px, full_viewport.height_px);
        let raster_key = Self::contract_cache_key(
            engine,
            page_number,
            dpi,
            render_mode,
            tile,
            visibility_fingerprint.clone(),
            prepress_fingerprint.clone(),
        )?;
        if let Some(hit) = cache.cached_display_list_raster(&raster_key) {
            return Ok(Some(hit));
        }
        if tile != full_tile {
            let full_raster_key = Self::contract_cache_key(
                engine,
                page_number,
                dpi,
                render_mode,
                full_tile,
                visibility_fingerprint,
                prepress_fingerprint,
            )?;
            if let Some(cropped) = cache
                .cached_display_list_raster_ref(&full_raster_key)
                .map(|full_page| crop_buffer(full_page, tile))
                .transpose()?
            {
                cache.insert_display_list_raster(raster_key, cropped.clone());
                return Ok(Some(cropped));
            }
        }
        let transparent_key = RenderDocumentCache::transparent_page_group_key_with_revision(
            page_number,
            Self::revision_cache_key(engine),
        );
        let transparent_page_group = match cache.cached_transparent_page_group(&transparent_key) {
            Some(value) => value,
            None => {
                let value = display_list_needs_transparent_page_group(
                    engine,
                    page_number,
                    &resources,
                    &list,
                )?;
                cache.insert_transparent_page_group(transparent_key, value);
                value
            }
        };
        let buf = Self::initial_page_buffer(&viewport, transparent_page_group, render_mode);
        let mut state = RenderState::new_with_document_cache(
            buf,
            viewport,
            resources,
            engine,
            page_number,
            cache,
        );
        state.cancel = cancel.clone();
        state.replay_display_list(&list);
        if let Err(err) = state.check_fatal_render_error() {
            state.return_document_cache(cache);
            return Err(err);
        }
        if let Err(err) = cancel.check("display-list tile replay") {
            state.return_document_cache(cache);
            return Err(err);
        }
        state.render_page_annotations();
        if let Err(err) = cancel.check("display-list tile annotation render") {
            state.return_document_cache(cache);
            return Err(err);
        }
        let mut buf = state.into_buffer_and_document_cache(cache);
        if transparent_page_group {
            buf.flatten_onto_background(WHITE);
        }
        let cropped = if expanded_tile == tile {
            buf
        } else {
            crop_buffer(
                &buf,
                RenderTile {
                    x: tile.x - expanded_tile.x,
                    y: tile.y - expanded_tile.y,
                    width: tile.width,
                    height: tile.height,
                },
            )?
        };
        cache.insert_display_list_raster(raster_key, cropped.clone());
        Ok(Some(cropped))
    }

    /// Render one page tile. The implementation is compatibility-safe: it
    /// executes the canonical page program into a tile-local viewport, with
    /// optional byte-budgeted tile caching. This preserves immediate-renderer
    /// semantics while avoiding full-page pixel allocation for progressive,
    /// viewport, and low-memory rendering.
    pub fn render_page_tile_with_mode(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        tile: RenderTile,
        render_mode: RenderMode,
        cache: Option<&mut RenderCache>,
    ) -> Result<PixelBuffer> {
        let ocg_fingerprint = OptionalContentContext::from_document(engine.document())
            .visibility_fingerprint()
            .to_string();
        let resources = engine.get_page_resources(page_number)?;
        let plate_fingerprint = prepress::cache_fingerprint_for_prepress_resources(
            resources.color_spaces.values(),
            resources.ext_g_states.values(),
        );
        let key = Self::contract_cache_key(
            engine,
            page_number,
            dpi,
            render_mode,
            tile,
            ocg_fingerprint,
            plate_fingerprint,
        )?;
        if let Some(cache) = cache {
            if let Some(hit) = cache.get(&key) {
                return Ok(hit);
            }
            let cropped = Self::render_page_tile_cancellable_with_mode(
                engine,
                page_number,
                dpi,
                tile,
                &CancelToken::none(),
                render_mode,
            )?;
            cache.insert(key, cropped.clone());
            Ok(cropped)
        } else {
            Self::render_page_tile_cancellable_with_mode(
                engine,
                page_number,
                dpi,
                tile,
                &CancelToken::none(),
                render_mode,
            )
        }
    }

    pub(crate) fn render_page_tile_cancellable_with_mode(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        tile: RenderTile,
        cancel: &CancelToken,
        render_mode: RenderMode,
    ) -> Result<PixelBuffer> {
        let mut document_cache = RenderDocumentCache::new();
        if let Some(buffer) = Self::render_page_display_list_tile_cancellable_with_mode_and_cache(
            engine,
            page_number,
            dpi,
            tile,
            cancel,
            render_mode,
            &mut document_cache,
        )? {
            return Ok(buffer);
        }
        Self::render_page_tile_immediate_cancellable_with_mode(
            engine,
            page_number,
            dpi,
            tile,
            cancel,
            render_mode,
        )
    }

    fn render_page_tile_immediate_cancellable_with_mode(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        tile: RenderTile,
        cancel: &CancelToken,
        render_mode: RenderMode,
    ) -> Result<PixelBuffer> {
        let ops = engine.get_page_content(page_number)?;
        let full_viewport = engine.page_viewport(page_number, dpi)?;
        if tile.width == 0 || tile.height == 0 {
            return Err(WellfriendError::invalid_input(
                "render tile must have non-zero width and height",
            ));
        }
        if tile.x >= full_viewport.width_px || tile.y >= full_viewport.height_px {
            return Err(WellfriendError::invalid_input(format!(
                "render tile origin ({},{}) is outside page bounds {}x{}",
                tile.x, tile.y, full_viewport.width_px, full_viewport.height_px
            )));
        }
        let end_x = tile.x.checked_add(tile.width).ok_or_else(|| {
            WellfriendError::invalid_input("render tile x range overflows".to_string())
        })?;
        let end_y = tile.y.checked_add(tile.height).ok_or_else(|| {
            WellfriendError::invalid_input("render tile y range overflows".to_string())
        })?;
        if end_x > full_viewport.width_px || end_y > full_viewport.height_px {
            return Err(WellfriendError::invalid_input(format!(
                "render tile {}x{} at {},{} exceeds page bounds {}x{}",
                tile.width,
                tile.height,
                tile.x,
                tile.y,
                full_viewport.width_px,
                full_viewport.height_px
            )));
        }
        const TILE_OVERDRAW_PX: u32 = 2;
        let expanded_tile = expand_render_tile(
            tile,
            full_viewport.width_px,
            full_viewport.height_px,
            TILE_OVERDRAW_PX,
        );
        let viewport = full_viewport.pixel_window(
            expanded_tile.x,
            expanded_tile.y,
            expanded_tile.width,
            expanded_tile.height,
        );
        if viewport.width_px == 0 || viewport.height_px == 0 {
            return Err(WellfriendError::invalid_input(
                "render tile is empty after clipping to page bounds",
            ));
        }
        let resources = engine.get_page_resources(page_number)?;
        let transparent_page_group = uses_top_level_transparency(&ops, &resources, engine);
        let buf = Self::initial_page_buffer(&viewport, transparent_page_group, render_mode);

        let mut state = RenderState::new(buf, viewport, resources, engine, page_number);
        state.cancel = cancel.clone();
        state.dispatch_all(&ops);
        state.check_fatal_render_error()?;
        cancel.check("page tile render")?;
        state.render_page_annotations();
        cancel.check("page tile annotation render")?;
        let mut buf = state.into_buffer();
        if transparent_page_group {
            buf.flatten_onto_background(WHITE);
        }
        if expanded_tile == tile {
            Ok(buf)
        } else {
            crop_buffer(
                &buf,
                RenderTile {
                    x: tile.x - expanded_tile.x,
                    y: tile.y - expanded_tile.y,
                    width: tile.width,
                    height: tile.height,
                },
            )
        }
    }

    /// Render page bands using the tile API. This gives callers a deterministic
    /// bounded-band seam while preserving compatibility renderer semantics.
    pub fn render_page_bands_with_mode(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        band_height: u32,
        render_mode: RenderMode,
    ) -> Result<Vec<PixelBuffer>> {
        let viewport = engine.page_viewport(page_number, dpi)?;
        let band_height = band_height.max(1);
        let mut bands = Vec::new();
        let mut document_cache = RenderDocumentCache::new();
        let mut y = 0u32;
        while y < viewport.height_px {
            let height = band_height.min(viewport.height_px - y);
            let tile = RenderTile {
                x: 0,
                y,
                width: viewport.width_px,
                height,
            };
            let band = match Self::render_page_display_list_tile_cancellable_with_mode_and_cache(
                engine,
                page_number,
                dpi,
                RenderTile {
                    x: 0,
                    y,
                    width: viewport.width_px,
                    height,
                },
                &CancelToken::none(),
                render_mode,
                &mut document_cache,
            )? {
                Some(buffer) => buffer,
                None => Self::render_page_tile_immediate_cancellable_with_mode(
                    engine,
                    page_number,
                    dpi,
                    tile,
                    &CancelToken::none(),
                    render_mode,
                )?,
            };
            bands.push(band);
            y += height;
        }
        Ok(bands)
    }

    /// Render-interpreter pass that returns sparse Prepress CMM/13 plate state.
    ///
    /// This follows the same content dispatch path as RGB rendering for page
    /// fill/stroke operations, and exposes plate/tint/OP/op/OPM side-channel
    /// data for supported Prepress Proofing prepress close-out cases.
    pub fn prepress_plate_report(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
    ) -> Result<SeparationFramebufferReport> {
        let ops = engine.get_page_content(page_number)?;
        let viewport = engine.page_viewport(page_number, dpi)?;
        let resources = engine.get_page_resources(page_number)?;
        let buf = Self::initial_page_buffer(&viewport, false, RenderMode::Compat);
        let mut state = RenderState::new(buf, viewport, resources, engine, page_number);
        state.dispatch_all(&ops);
        Ok(state.into_separation_framebuffer().report())
    }

    fn initial_page_buffer(
        viewport: &Viewport,
        transparent_page_group: bool,
        render_mode: RenderMode,
    ) -> PixelBuffer {
        if transparent_page_group {
            PixelBuffer::new_transparent_with_mode(
                viewport.width_px,
                viewport.height_px,
                render_mode,
            )
        } else {
            PixelBuffer::new_filled_with_mode(
                viewport.width_px,
                viewport.height_px,
                WHITE,
                render_mode,
            )
        }
    }

    fn render_annotations_into(
        engine: &ContentEngine,
        page_number: usize,
        dpi: u32,
        buf: &mut PixelBuffer,
    ) -> Result<()> {
        if !Self::page_has_annotations(engine, page_number) {
            return Ok(());
        }
        let viewport = engine.page_viewport(page_number, dpi)?;
        let resources = engine.get_page_resources(page_number)?;
        let mut state = RenderState::new(buf.clone(), viewport, resources, engine, page_number);
        state.render_page_annotations();
        *buf = state.into_buffer();
        Ok(())
    }

    fn page_has_annotations(engine: &ContentEngine, page_number: usize) -> bool {
        let reader = engine.document().reader();
        let pages = match engine.document().get_pages() {
            Ok(pages) => pages,
            Err(_) => return false,
        };
        let Some(page) = pages.get(page_number.saturating_sub(1)) else {
            return false;
        };
        let Ok(PdfObject::Dictionary(page_dict)) =
            reader.get_and_resolve(page.object_number, page.generation_number)
        else {
            return false;
        };
        let Some(annots_obj) = page_dict.get("Annots").cloned() else {
            return false;
        };
        matches!(reader.resolve(annots_obj), Ok(PdfObject::Array(items)) if !items.is_empty())
    }
}

struct RenderState<'a> {
    engine: &'a ContentEngine,
    page_number: usize,
    buf: PixelBuffer,
    viewport: Viewport,
    resources: PageResources,
    gs: GraphicsState,
    clip_stack: Vec<Arc<ClipNode>>,
    smask_stack: Vec<Option<AlphaMask>>,
    path: Path,
    pending_clip: Option<FillRule>,
    pending_text_clip: Option<ClipMask>,
    glyph_cache: GlyphCache,
    glyph_mask_cache: GlyphMaskCache,
    type3_mask_cache: Type3MaskCache,
    type3_rendered_cache: Type3RenderedGlyphCache,
    path_fill_mask_cache: PathFillMaskCache,
    path_stroke_mask_cache: PathStrokeMaskCache,
    font_bytes_cache: HashMap<String, Option<Arc<Vec<u8>>>>,
    font_bytes_cache_stats: RenderArtifactCacheStats,
    font_resolver_cache: HashMap<String, Arc<FontResolver>>,
    font_resolver_cache_stats: RenderArtifactCacheStats,
    font_resource_key_cache: HashMap<(String, usize), String>,
    type3_geometry_cache: HashMap<String, Option<Arc<Type3GlyphGeometry>>>,
    type3_charproc_cache: HashMap<String, Option<Arc<Type3CharProc>>>,
    image_xobject_cache: HashMap<String, Arc<RawImage>>,
    image_xobject_cache_order: VecDeque<String>,
    image_xobject_cache_bytes: usize,
    image_xobject_cache_stats: RenderArtifactCacheStats,
    scaled_image_cache: HashMap<String, Arc<RawImage>>,
    scaled_image_cache_order: VecDeque<String>,
    scaled_image_cache_bytes: usize,
    scaled_image_cache_stats: RenderArtifactCacheStats,
    smask_group_cache: HashMap<String, Arc<AlphaMask>>,
    smask_group_cache_order: VecDeque<String>,
    smask_group_cache_bytes: usize,
    smask_group_cache_stats: RenderArtifactCacheStats,
    shading_mesh_cache: HashMap<String, Arc<Vec<u8>>>,
    shading_mesh_cache_order: VecDeque<String>,
    shading_mesh_cache_bytes: usize,
    shading_mesh_cache_stats: RenderArtifactCacheStats,
    form_xobject_program_cache: HashMap<String, Option<Arc<FormXObjectProgram>>>,
    form_xobject_program_cache_stats: RenderArtifactCacheStats,
    tiling_pattern_program_cache: HashMap<String, Option<Arc<Vec<ContentOperation>>>>,
    tiling_pattern_program_cache_stats: RenderArtifactCacheStats,
    offscreen_buffer_pool: Vec<PixelBuffer>,
    /// Persistent clip DAG for structural sharing of clip states across
    /// save/restore cycles. Rectangle and path clips are interned so that
    /// repeated q/Q pairs share the same Arc instead of cloning masks.
    clip_dag: ClipDag,
    /// Tiling-pattern stream keys currently being replayed. PDF pattern
    /// resources can legally refer to other patterns, but real files sometimes
    /// contain accidental self-recursive pattern fills. Bound those at the
    /// source object instead of relying only on the coarse Form depth limit.
    pattern_stack: Vec<String>,
    /// Current Form XObject nesting depth, used to bound recursion.
    form_depth: usize,
    /// Source object stack for currently replayed Form XObjects. This catches
    /// direct and indirect cycles before the coarse depth guard would force
    /// repeated full-form replay attempts.
    form_object_stack: Vec<(u32, u16)>,
    /// Parameters from the most recent `ID` operator, awaiting the following
    /// `inline_image_data` so the inline image can be painted.
    pending_inline: Option<Vec<Operand>>,
    /// The CTM in effect at the start of the current content stream (the page's
    /// or a Form's). Pattern `/Matrix` values are relative to *this* default
    /// coordinate system, not the CTM at the moment of the fill.
    base_ctm: Transform2D,
    /// Cooperative cancellation flag, polled by the operator dispatch loop and
    /// the tiling-pattern tile loop so a runaway page can be stopped from
    /// outside. Child states (Form groups, soft masks) share the same token.
    cancel: CancelToken,
    /// First fatal renderer condition observed while interpreting the page.
    /// Void operator handlers record here so public page/tile APIs can return
    /// a typed error after safely unwinding their local state.
    fatal_render_error: Option<String>,
    /// Per-render decode scheduler context. Current renderer decode is
    /// synchronous for deterministic composition, but every image/stream decode
    /// still acquires a memory token and observes cancellation before work.
    decode_scheduler: RenderDecodeScheduler,
    /// Document optional-content state for the active view configuration.
    optional_content: OptionalContentContext,
    /// Visibility stack for nested BMC/BDC/EMC marked-content sections.
    oc_visibility_stack: Vec<bool>,
    oc_current_visible: bool,
    /// Sparse Prepress CMM plate framebuffer side-channel. It records Separation
    /// and DeviceN tint identity for report/proofing without changing RGB
    /// preview compositing semantics.
    separation_framebuffer: SeparationFramebuffer,
}

const RENDER_DOCUMENT_IMAGE_CACHE_MAX_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct GlyphMaskCacheKey {
    glyph: GlyphCacheKey,
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    frac_e: i64,
    frac_f: i64,
    hinting: bool,
}

#[derive(Default)]
struct GlyphMaskCache {
    entries: HashMap<GlyphMaskCacheKey, Arc<RasterizedGlyphMask>>,
    bytes: usize,
    stats: RenderArtifactCacheStats,
}

impl GlyphMaskCache {
    const MAX_ENTRIES: usize = 4096;
    const MAX_BYTES: usize = 32 * 1024 * 1024;

    fn get(&mut self, key: &GlyphMaskCacheKey) -> Option<Arc<RasterizedGlyphMask>> {
        let hit = self.entries.get(key).cloned();
        if hit.is_some() {
            self.stats.hits = self.stats.hits.saturating_add(1);
        } else {
            self.stats.misses = self.stats.misses.saturating_add(1);
        }
        hit
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn bytes(&self) -> usize {
        self.bytes
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.stats = RenderArtifactCacheStats::default();
    }

    fn stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.entries.len(),
            bytes: self.bytes,
            ..self.stats
        }
    }

    fn insert(&mut self, key: GlyphMaskCacheKey, mask: Arc<RasterizedGlyphMask>) {
        let bytes = mask.approximate_bytes();
        if bytes > Self::MAX_BYTES / 4 {
            self.stats.skipped_oversized = self.stats.skipped_oversized.saturating_add(1);
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.approximate_bytes());
        }
        if self.entries.len() >= Self::MAX_ENTRIES
            || self.bytes.saturating_add(bytes) > Self::MAX_BYTES
        {
            self.stats.evictions = self
                .stats
                .evictions
                .saturating_add(self.entries.len() as u64);
            self.entries.clear();
            self.bytes = 0;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(key, mask);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Type3MaskCacheKey {
    glyph: String,
    fill_index: u16,
    fill_rule: u8,
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    frac_e: i64,
    frac_f: i64,
}

#[derive(Default)]
struct Type3MaskCache {
    entries: HashMap<Type3MaskCacheKey, Arc<RasterizedGlyphMask>>,
    bytes: usize,
    stats: RenderArtifactCacheStats,
}

impl Type3MaskCache {
    const MAX_ENTRIES: usize = 8192;
    const MAX_BYTES: usize = 64 * 1024 * 1024;

    fn get(&mut self, key: &Type3MaskCacheKey) -> Option<Arc<RasterizedGlyphMask>> {
        let hit = self.entries.get(key).cloned();
        if hit.is_some() {
            self.stats.hits = self.stats.hits.saturating_add(1);
        } else {
            self.stats.misses = self.stats.misses.saturating_add(1);
        }
        hit
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn bytes(&self) -> usize {
        self.bytes
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.stats = RenderArtifactCacheStats::default();
    }

    fn stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.entries.len(),
            bytes: self.bytes,
            ..self.stats
        }
    }

    fn insert(&mut self, key: Type3MaskCacheKey, mask: Arc<RasterizedGlyphMask>) {
        let bytes = mask.approximate_bytes();
        if bytes > Self::MAX_BYTES / 4 {
            self.stats.skipped_oversized = self.stats.skipped_oversized.saturating_add(1);
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.approximate_bytes());
        }
        if self.entries.len() >= Self::MAX_ENTRIES
            || self.bytes.saturating_add(bytes) > Self::MAX_BYTES
        {
            self.stats.evictions = self
                .stats
                .evictions
                .saturating_add(self.entries.len() as u64);
            self.entries.clear();
            self.bytes = 0;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(key, mask);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Type3RenderedGlyphCacheKey {
    glyph: String,
    render_mode: i32,
    fill_color: PixelColor,
    stroke_color: PixelColor,
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    frac_e: i64,
    frac_f: i64,
}

#[derive(Clone)]
struct Type3RenderedGlyph {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Type3RenderedGlyph {
    fn from_buffer_with_origin(
        buf: &PixelBuffer,
        bounds: (u32, u32, u32, u32),
        full_x0: i32,
        full_y0: i32,
        origin_dx: i32,
        origin_dy: i32,
    ) -> Option<Self> {
        if buf.width == 0 || buf.height == 0 {
            return None;
        }
        let (scan_x0, scan_y0, scan_x1, scan_y1) = bounds;
        if scan_x0 > scan_x1 || scan_y0 > scan_y1 || scan_x0 >= buf.width || scan_y0 >= buf.height {
            return None;
        }
        let scan_x1 = scan_x1.min(buf.width.saturating_sub(1));
        let scan_y1 = scan_y1.min(buf.height.saturating_sub(1));
        let mut min_x = buf.width as i32;
        let mut min_y = buf.height as i32;
        let mut max_x = -1;
        let mut max_y = -1;
        for y in scan_y0 as i32..=scan_y1 as i32 {
            for x in scan_x0 as i32..=scan_x1 as i32 {
                if buf.get_pixel(x, y)[3] != 0 {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        if max_x < min_x || max_y < min_y {
            return None;
        }
        let width = (max_x - min_x + 1) as u32;
        let height = (max_y - min_y + 1) as u32;
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                pixels.extend_from_slice(&buf.get_pixel(x, y));
            }
        }
        Some(Self {
            x: full_x0.saturating_add(min_x).saturating_sub(origin_dx),
            y: full_y0.saturating_add(min_y).saturating_sub(origin_dy),
            width,
            height,
            pixels,
        })
    }

    fn paint(&self, buf: &mut PixelBuffer, dx: i32, dy: i32) {
        buf.blend_rgba_pixels_at(
            dx.saturating_add(self.x),
            dy.saturating_add(self.y),
            self.width,
            self.height,
            &self.pixels,
        );
    }

    fn approximate_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.pixels.len()
    }
}

#[derive(Default)]
struct Type3RenderedGlyphCache {
    entries: HashMap<Type3RenderedGlyphCacheKey, Arc<Type3RenderedGlyph>>,
    bytes: usize,
    stats: RenderArtifactCacheStats,
}

impl Type3RenderedGlyphCache {
    const MAX_ENTRIES: usize = 2048;
    const MAX_BYTES: usize = 96 * 1024 * 1024;

    fn get(&mut self, key: &Type3RenderedGlyphCacheKey) -> Option<Arc<Type3RenderedGlyph>> {
        let hit = self.entries.get(key).cloned();
        if hit.is_some() {
            self.stats.hits = self.stats.hits.saturating_add(1);
        } else {
            self.stats.misses = self.stats.misses.saturating_add(1);
        }
        hit
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn bytes(&self) -> usize {
        self.bytes
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.stats = RenderArtifactCacheStats::default();
    }

    fn stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.entries.len(),
            bytes: self.bytes,
            ..self.stats
        }
    }

    fn insert(&mut self, key: Type3RenderedGlyphCacheKey, glyph: Arc<Type3RenderedGlyph>) {
        let bytes = glyph.approximate_bytes();
        if bytes > Self::MAX_BYTES / 4 {
            self.stats.skipped_oversized = self.stats.skipped_oversized.saturating_add(1);
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.approximate_bytes());
        }
        if self.entries.len() >= Self::MAX_ENTRIES
            || self.bytes.saturating_add(bytes) > Self::MAX_BYTES
        {
            self.stats.evictions = self
                .stats
                .evictions
                .saturating_add(self.entries.len() as u64);
            self.entries.clear();
            self.bytes = 0;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(key, glyph);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct PathFillMaskCacheKey {
    path_hash: u64,
    fill_rule: u8,
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    frac_e: i64,
    frac_f: i64,
}

#[derive(Default)]
struct PathFillMaskCache {
    entries: HashMap<PathFillMaskCacheKey, Arc<RasterizedGlyphMask>>,
    bytes: usize,
    stats: RenderArtifactCacheStats,
}

impl PathFillMaskCache {
    const MAX_ENTRIES: usize = 8192;
    const MAX_BYTES: usize = 96 * 1024 * 1024;

    fn get(&mut self, key: &PathFillMaskCacheKey) -> Option<Arc<RasterizedGlyphMask>> {
        let hit = self.entries.get(key).cloned();
        if hit.is_some() {
            self.stats.hits = self.stats.hits.saturating_add(1);
        } else {
            self.stats.misses = self.stats.misses.saturating_add(1);
        }
        hit
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn bytes(&self) -> usize {
        self.bytes
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.stats = RenderArtifactCacheStats::default();
    }

    fn stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.entries.len(),
            bytes: self.bytes,
            ..self.stats
        }
    }

    fn insert(&mut self, key: PathFillMaskCacheKey, mask: Arc<RasterizedGlyphMask>) {
        let bytes = mask.approximate_bytes();
        if bytes > Self::MAX_BYTES / 4 {
            self.stats.skipped_oversized = self.stats.skipped_oversized.saturating_add(1);
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.approximate_bytes());
        }
        if self.entries.len() >= Self::MAX_ENTRIES
            || self.bytes.saturating_add(bytes) > Self::MAX_BYTES
        {
            self.stats.evictions = self
                .stats
                .evictions
                .saturating_add(self.entries.len() as u64);
            self.entries.clear();
            self.bytes = 0;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(key, mask);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct PathStrokeMaskCacheKey {
    path_hash: u64,
    width: i64,
    cap: u8,
    join: u8,
    miter_limit: i64,
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    frac_e: i64,
    frac_f: i64,
}

#[derive(Default)]
struct PathStrokeMaskCache {
    entries: HashMap<PathStrokeMaskCacheKey, Arc<RasterizedGlyphMask>>,
    bytes: usize,
    stats: RenderArtifactCacheStats,
}

impl PathStrokeMaskCache {
    const MAX_ENTRIES: usize = 8192;
    const MAX_BYTES: usize = 96 * 1024 * 1024;

    fn get(&mut self, key: &PathStrokeMaskCacheKey) -> Option<Arc<RasterizedGlyphMask>> {
        let hit = self.entries.get(key).cloned();
        if hit.is_some() {
            self.stats.hits = self.stats.hits.saturating_add(1);
        } else {
            self.stats.misses = self.stats.misses.saturating_add(1);
        }
        hit
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn bytes(&self) -> usize {
        self.bytes
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.stats = RenderArtifactCacheStats::default();
    }

    fn stats(&self) -> RenderArtifactCacheStats {
        RenderArtifactCacheStats {
            entries: self.entries.len(),
            bytes: self.bytes,
            ..self.stats
        }
    }

    fn insert(&mut self, key: PathStrokeMaskCacheKey, mask: Arc<RasterizedGlyphMask>) {
        let bytes = mask.approximate_bytes();
        if bytes > Self::MAX_BYTES / 4 {
            self.stats.skipped_oversized = self.stats.skipped_oversized.saturating_add(1);
            return;
        }
        if let Some(old) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.approximate_bytes());
        }
        if self.entries.len() >= Self::MAX_ENTRIES
            || self.bytes.saturating_add(bytes) > Self::MAX_BYTES
        {
            self.stats.evictions = self
                .stats
                .evictions
                .saturating_add(self.entries.len() as u64);
            self.entries.clear();
            self.bytes = 0;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(key, mask);
    }
}

#[derive(Clone)]
struct FormXObjectProgram {
    dict: PdfDictionary,
    ops: Arc<Vec<ContentOperation>>,
    form_matrix: crate::content::Matrix,
    bbox: Option<[f64; 4]>,
    resources: Option<PageResources>,
    is_transparency_group: bool,
}

#[derive(Clone, Debug)]
struct RenderDecodeScheduler {
    budget: Arc<DecodeMemoryBudget>,
    state: Arc<Mutex<RenderDecodeSchedulerState>>,
}

#[derive(Clone, Debug, Default)]
struct RenderDecodeSchedulerState {
    jobs: usize,
    rejected_jobs: usize,
    cancelled_before_decode: usize,
    failed_jobs: usize,
}

impl RenderDecodeScheduler {
    fn new(limits: &DecodeLimits) -> Self {
        Self {
            budget: Arc::new(DecodeMemoryBudget::new(
                limits.scheduler_memory_budget_bytes.max(1),
            )),
            state: Arc::new(Mutex::new(RenderDecodeSchedulerState::default())),
        }
    }

    fn run<T>(
        &self,
        estimated_bytes: u64,
        cancel: &CancelToken,
        context: &str,
        work: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        if let Ok(mut state) = self.state.lock() {
            state.jobs += 1;
        }
        if cancel.is_cancelled() {
            if let Ok(mut state) = self.state.lock() {
                state.cancelled_before_decode += 1;
            }
            return Err(WellfriendError::Cancelled(context.to_string()));
        }
        let token = match self.budget.acquire(estimated_bytes.max(1)) {
            Ok(token) => token,
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.rejected_jobs += 1;
                    state.failed_jobs += 1;
                }
                return Err(err);
            }
        };
        let result = work();
        drop(token);
        if result.is_err() {
            if let Ok(mut state) = self.state.lock() {
                state.failed_jobs += 1;
            }
        }
        result
    }

    fn reserve_memory(
        &self,
        estimated_bytes: u64,
        cancel: &CancelToken,
        context: &str,
    ) -> Result<DecodeMemoryToken> {
        if let Ok(mut state) = self.state.lock() {
            state.jobs += 1;
        }
        if cancel.is_cancelled() {
            if let Ok(mut state) = self.state.lock() {
                state.cancelled_before_decode += 1;
            }
            return Err(WellfriendError::Cancelled(context.to_string()));
        }
        match self.budget.acquire(estimated_bytes.max(1)) {
            Ok(token) => Ok(token),
            Err(err) => {
                if let Ok(mut state) = self.state.lock() {
                    state.rejected_jobs += 1;
                    state.failed_jobs += 1;
                }
                Err(err)
            }
        }
    }

    #[cfg(test)]
    fn metrics(&self) -> RendererDecodeSchedulerMetrics {
        let budget = self.budget.metrics();
        let state = self
            .state
            .lock()
            .expect("renderer decode scheduler metrics lock")
            .clone();
        RendererDecodeSchedulerMetrics {
            jobs: state.jobs,
            workers: 1,
            memory_budget_bytes: budget.memory_budget_bytes,
            peak_reserved_bytes: budget.peak_reserved_bytes,
            wait_count: budget.wait_count,
            rejected_jobs: state.rejected_jobs,
            cancelled_before_decode: state.cancelled_before_decode,
            failed_jobs: state.failed_jobs,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RendererDecodeSchedulerMetrics {
    jobs: usize,
    workers: usize,
    memory_budget_bytes: u64,
    peak_reserved_bytes: u64,
    wait_count: usize,
    rejected_jobs: usize,
    cancelled_before_decode: usize,
    failed_jobs: usize,
}

impl<'a> RenderState<'a> {
    fn new(
        buf: PixelBuffer,
        viewport: Viewport,
        resources: PageResources,
        engine: &'a ContentEngine,
        page_number: usize,
    ) -> Self {
        let mut cache = RenderDocumentCache::new();
        Self::new_with_document_cache(buf, viewport, resources, engine, page_number, &mut cache)
    }

    fn new_with_document_cache(
        buf: PixelBuffer,
        viewport: Viewport,
        resources: PageResources,
        engine: &'a ContentEngine,
        page_number: usize,
        cache: &mut RenderDocumentCache,
    ) -> Self {
        let viewport_width = viewport.width_px;
        let viewport_height = viewport.height_px;
        let image_xobject_cache_bytes = cache.image_xobject_cache_bytes;
        let scaled_image_cache_bytes = cache.scaled_image_cache_bytes;
        let smask_group_cache_bytes = cache.smask_group_cache_bytes;
        let shading_mesh_cache_bytes = cache.shading_mesh_cache_bytes;
        let font_bytes_cache_stats = cache.font_bytes_cache_stats;
        let font_resolver_cache_stats = cache.font_resolver_cache_stats;
        let image_xobject_cache_stats = cache.image_xobject_cache_stats;
        let scaled_image_cache_stats = cache.scaled_image_cache_stats;
        let smask_group_cache_stats = cache.smask_group_cache_stats;
        let shading_mesh_cache_stats = cache.shading_mesh_cache_stats;
        let form_xobject_program_cache_stats = cache.form_xobject_program_cache_stats;
        let tiling_pattern_program_cache_stats = cache.tiling_pattern_program_cache_stats;
        cache.image_xobject_cache_bytes = 0;
        cache.scaled_image_cache_bytes = 0;
        cache.smask_group_cache_bytes = 0;
        cache.shading_mesh_cache_bytes = 0;
        cache.font_bytes_cache_stats = RenderArtifactCacheStats::default();
        cache.font_resolver_cache_stats = RenderArtifactCacheStats::default();
        cache.image_xobject_cache_stats = RenderArtifactCacheStats::default();
        cache.scaled_image_cache_stats = RenderArtifactCacheStats::default();
        cache.smask_group_cache_stats = RenderArtifactCacheStats::default();
        cache.shading_mesh_cache_stats = RenderArtifactCacheStats::default();
        cache.form_xobject_program_cache_stats = RenderArtifactCacheStats::default();
        cache.tiling_pattern_program_cache_stats = RenderArtifactCacheStats::default();
        Self {
            engine,
            page_number,
            buf,
            viewport,
            resources,
            gs: GraphicsState::default(),
            clip_stack: Vec::new(),
            smask_stack: Vec::new(),
            path: Path::new(),
            pending_clip: None,
            pending_text_clip: None,
            glyph_cache: std::mem::replace(
                &mut cache.glyph_cache,
                GlyphCache::with_default_capacity(),
            ),
            glyph_mask_cache: std::mem::take(&mut cache.glyph_mask_cache),
            type3_mask_cache: std::mem::take(&mut cache.type3_mask_cache),
            type3_rendered_cache: std::mem::take(&mut cache.type3_rendered_cache),
            path_fill_mask_cache: std::mem::take(&mut cache.path_fill_mask_cache),
            path_stroke_mask_cache: std::mem::take(&mut cache.path_stroke_mask_cache),
            font_bytes_cache: std::mem::take(&mut cache.font_bytes_cache),
            font_bytes_cache_stats,
            font_resolver_cache: std::mem::take(&mut cache.font_resolver_cache),
            font_resolver_cache_stats,
            font_resource_key_cache: HashMap::new(),
            type3_geometry_cache: std::mem::take(&mut cache.type3_geometry_cache),
            type3_charproc_cache: std::mem::take(&mut cache.type3_charproc_cache),
            image_xobject_cache: std::mem::take(&mut cache.image_xobject_cache),
            image_xobject_cache_order: std::mem::take(&mut cache.image_xobject_cache_order),
            image_xobject_cache_bytes,
            image_xobject_cache_stats,
            scaled_image_cache: std::mem::take(&mut cache.scaled_image_cache),
            scaled_image_cache_order: std::mem::take(&mut cache.scaled_image_cache_order),
            scaled_image_cache_bytes,
            scaled_image_cache_stats,
            smask_group_cache: std::mem::take(&mut cache.smask_group_cache),
            smask_group_cache_order: std::mem::take(&mut cache.smask_group_cache_order),
            smask_group_cache_bytes,
            smask_group_cache_stats,
            shading_mesh_cache: std::mem::take(&mut cache.shading_mesh_cache),
            shading_mesh_cache_order: std::mem::take(&mut cache.shading_mesh_cache_order),
            shading_mesh_cache_bytes,
            shading_mesh_cache_stats,
            form_xobject_program_cache: std::mem::take(&mut cache.form_xobject_program_cache),
            form_xobject_program_cache_stats,
            tiling_pattern_program_cache: std::mem::take(&mut cache.tiling_pattern_program_cache),
            tiling_pattern_program_cache_stats,
            offscreen_buffer_pool: std::mem::take(&mut cache.offscreen_buffer_pool),
            clip_dag: ClipDag::new(),
            pattern_stack: Vec::new(),
            form_depth: 0,
            form_object_stack: Vec::new(),
            pending_inline: None,
            base_ctm: Transform2D::identity(),
            cancel: CancelToken::none(),
            fatal_render_error: None,
            decode_scheduler: RenderDecodeScheduler::new(&DecodeLimits::default()),
            optional_content: OptionalContentContext::from_document(engine.document()),
            oc_visibility_stack: Vec::new(),
            oc_current_visible: true,
            separation_framebuffer: SeparationFramebuffer::for_page(
                page_number,
                viewport_width,
                viewport_height,
            ),
        }
    }

    fn record_fatal_render_error(&mut self, reason: impl Into<String>) {
        if self.fatal_render_error.is_none() {
            self.fatal_render_error = Some(reason.into());
        }
    }

    fn check_fatal_render_error(&self) -> Result<()> {
        match &self.fatal_render_error {
            Some(reason) => Err(WellfriendError::UnsupportedFeature(reason.clone())),
            None => Ok(()),
        }
    }

    fn into_buffer(self) -> PixelBuffer {
        self.buf
    }

    fn return_document_cache(self, cache: &mut RenderDocumentCache) {
        let display_list_cache = std::mem::take(&mut cache.display_list_cache);
        let display_list_raster_cache = std::mem::replace(
            &mut cache.display_list_raster_cache,
            RenderCache::new(256 * 1024 * 1024, 64 * 1024 * 1024),
        );
        let transparent_page_group_cache = std::mem::take(&mut cache.transparent_page_group_cache);
        let document_revision = cache.document_revision;
        let dependency_graph = std::mem::take(&mut cache.dependency_graph);
        *cache = RenderDocumentCache {
            glyph_cache: self.glyph_cache,
            glyph_mask_cache: self.glyph_mask_cache,
            type3_mask_cache: self.type3_mask_cache,
            type3_rendered_cache: self.type3_rendered_cache,
            path_fill_mask_cache: self.path_fill_mask_cache,
            path_stroke_mask_cache: self.path_stroke_mask_cache,
            font_bytes_cache: self.font_bytes_cache,
            font_bytes_cache_stats: self.font_bytes_cache_stats,
            font_resolver_cache: self.font_resolver_cache,
            font_resolver_cache_stats: self.font_resolver_cache_stats,
            type3_geometry_cache: self.type3_geometry_cache,
            type3_charproc_cache: self.type3_charproc_cache,
            image_xobject_cache: self.image_xobject_cache,
            image_xobject_cache_order: self.image_xobject_cache_order,
            image_xobject_cache_bytes: self.image_xobject_cache_bytes,
            image_xobject_cache_stats: self.image_xobject_cache_stats,
            scaled_image_cache: self.scaled_image_cache,
            scaled_image_cache_order: self.scaled_image_cache_order,
            scaled_image_cache_bytes: self.scaled_image_cache_bytes,
            scaled_image_cache_stats: self.scaled_image_cache_stats,
            smask_group_cache: self.smask_group_cache,
            smask_group_cache_order: self.smask_group_cache_order,
            smask_group_cache_bytes: self.smask_group_cache_bytes,
            smask_group_cache_stats: self.smask_group_cache_stats,
            shading_mesh_cache: self.shading_mesh_cache,
            shading_mesh_cache_order: self.shading_mesh_cache_order,
            shading_mesh_cache_bytes: self.shading_mesh_cache_bytes,
            shading_mesh_cache_stats: self.shading_mesh_cache_stats,
            form_xobject_program_cache: self.form_xobject_program_cache,
            form_xobject_program_cache_stats: self.form_xobject_program_cache_stats,
            tiling_pattern_program_cache: self.tiling_pattern_program_cache,
            tiling_pattern_program_cache_stats: self.tiling_pattern_program_cache_stats,
            offscreen_buffer_pool: self.offscreen_buffer_pool,
            display_list_cache,
            display_list_raster_cache,
            transparent_page_group_cache,
            document_revision,
            dependency_graph,
        };
        cache.enforce_bounded_maps();
    }

    fn into_buffer_and_document_cache(self, cache: &mut RenderDocumentCache) -> PixelBuffer {
        let RenderState {
            buf,
            glyph_cache,
            glyph_mask_cache,
            type3_mask_cache,
            type3_rendered_cache,
            path_fill_mask_cache,
            path_stroke_mask_cache,
            font_bytes_cache,
            font_bytes_cache_stats,
            font_resolver_cache,
            font_resolver_cache_stats,
            type3_geometry_cache,
            type3_charproc_cache,
            image_xobject_cache,
            image_xobject_cache_order,
            image_xobject_cache_bytes,
            image_xobject_cache_stats,
            scaled_image_cache,
            scaled_image_cache_order,
            scaled_image_cache_bytes,
            scaled_image_cache_stats,
            smask_group_cache,
            smask_group_cache_order,
            smask_group_cache_bytes,
            smask_group_cache_stats,
            shading_mesh_cache,
            shading_mesh_cache_order,
            shading_mesh_cache_bytes,
            shading_mesh_cache_stats,
            form_xobject_program_cache,
            form_xobject_program_cache_stats,
            tiling_pattern_program_cache,
            tiling_pattern_program_cache_stats,
            offscreen_buffer_pool,
            ..
        } = self;
        let display_list_cache = std::mem::take(&mut cache.display_list_cache);
        let display_list_raster_cache = std::mem::replace(
            &mut cache.display_list_raster_cache,
            RenderCache::new(256 * 1024 * 1024, 64 * 1024 * 1024),
        );
        let transparent_page_group_cache = std::mem::take(&mut cache.transparent_page_group_cache);
        let document_revision = cache.document_revision;
        let dependency_graph = std::mem::take(&mut cache.dependency_graph);
        *cache = RenderDocumentCache {
            glyph_cache,
            glyph_mask_cache,
            type3_mask_cache,
            type3_rendered_cache,
            path_fill_mask_cache,
            path_stroke_mask_cache,
            font_bytes_cache,
            font_bytes_cache_stats,
            font_resolver_cache,
            font_resolver_cache_stats,
            type3_geometry_cache,
            type3_charproc_cache,
            image_xobject_cache,
            image_xobject_cache_order,
            image_xobject_cache_bytes,
            image_xobject_cache_stats,
            scaled_image_cache,
            scaled_image_cache_order,
            scaled_image_cache_bytes,
            scaled_image_cache_stats,
            smask_group_cache,
            smask_group_cache_order,
            smask_group_cache_bytes,
            smask_group_cache_stats,
            shading_mesh_cache,
            shading_mesh_cache_order,
            shading_mesh_cache_bytes,
            shading_mesh_cache_stats,
            form_xobject_program_cache,
            form_xobject_program_cache_stats,
            tiling_pattern_program_cache,
            tiling_pattern_program_cache_stats,
            offscreen_buffer_pool,
            display_list_cache,
            display_list_raster_cache,
            transparent_page_group_cache,
            document_revision,
            dependency_graph,
        };
        cache.enforce_bounded_maps();
        buf
    }

    fn into_separation_framebuffer(self) -> SeparationFramebuffer {
        self.separation_framebuffer
    }

    fn dispatch_all(&mut self, ops: &[ContentOperation]) {
        // Poll the cancellation flag every CANCEL_CHECK_INTERVAL operators. An
        // atomic relaxed load is cheap, but doing it per-operator on a hot path
        // with millions of trivial ops is measurable, so we amortise it. The
        // interval is small enough that even when individual operators are
        // expensive (e.g. full-page fills) the wall-clock gap between checks
        // stays short, so cancellation is observed promptly. When the token
        // trips we stop executing immediately; the entry point converts the
        // early exit into an WellfriendError::Cancelled.
        const CANCEL_CHECK_INTERVAL: usize = 64;
        for (i, op) in ops.iter().enumerate() {
            if i % CANCEL_CHECK_INTERVAL == 0 && self.cancel.is_cancelled() {
                return;
            }
            self.dispatch(op);
        }
    }

    fn scheduled_decode_stream(
        &self,
        stream_obj: &PdfObject,
        reader: &crate::reader::PdfReader,
        context: &str,
    ) -> Result<Vec<u8>> {
        let estimated = estimate_stream_decode_bytes(stream_obj);
        self.decode_scheduler
            .run(estimated, &self.cancel, context, || {
                crate::filters::decode_stream(stream_obj, reader)
            })
    }

    fn scheduled_decode_image(
        &mut self,
        image_ref: &ImageReference,
        context: &str,
    ) -> Result<Arc<RawImage>> {
        self.scheduled_decode_image_with_color_space(image_ref, None, context)
    }

    fn scheduled_decode_image_with_color_space(
        &mut self,
        image_ref: &ImageReference,
        color_space_override: Option<&(String, PdfObject)>,
        context: &str,
    ) -> Result<Arc<RawImage>> {
        let cache_key = color_space_override
            .map(|(name, obj)| image_xobject_cache_key_with_color_space(image_ref, name, obj))
            .unwrap_or_else(|| image_xobject_cache_key(image_ref));
        if self.image_xobject_cache.contains_key(&cache_key) {
            touch_image_xobject_cache_key(&mut self.image_xobject_cache_order, &cache_key);
            if let Some(cached) = self.image_xobject_cache.get(&cache_key) {
                self.image_xobject_cache_stats.hits =
                    self.image_xobject_cache_stats.hits.saturating_add(1);
                return Ok(Arc::clone(cached));
            }
        }
        self.image_xobject_cache_stats.misses =
            self.image_xobject_cache_stats.misses.saturating_add(1);
        let estimated = estimate_image_ref_decode_bytes(image_ref);
        let reader = self.engine.document().reader();
        let raw = self
            .decode_scheduler
            .run(estimated, &self.cancel, context, || {
                if let Some((color_space_name, color_space_obj)) = color_space_override {
                    ImageDecoder::decode_with_resolved_color_space(
                        image_ref,
                        reader,
                        color_space_name,
                        color_space_obj,
                    )
                } else {
                    ImageDecoder::decode(image_ref, reader)
                }
            })?;
        let raw = Arc::new(raw);
        let raw_bytes = raw.byte_count();
        insert_image_xobject_cache_entry(
            &mut self.image_xobject_cache,
            &mut self.image_xobject_cache_order,
            &mut self.image_xobject_cache_bytes,
            cache_key,
            Arc::clone(&raw),
            raw_bytes,
            RENDER_DOCUMENT_IMAGE_CACHE_MAX_BYTES,
        );
        Ok(raw)
    }

    fn cached_axis_aligned_scaled_image(
        &mut self,
        base_key: &str,
        image: &RawImage,
        target_width: u32,
        target_height: u32,
    ) -> Option<Arc<RawImage>> {
        let high_quality = self.buf.render_mode().is_high_quality();
        let cache_key = scaled_image_cache_key(base_key, target_width, target_height, high_quality);
        if self.scaled_image_cache.contains_key(&cache_key) {
            touch_scaled_image_cache_key(&mut self.scaled_image_cache_order, &cache_key);
            if let Some(cached) = self.scaled_image_cache.get(&cache_key) {
                self.scaled_image_cache_stats.hits =
                    self.scaled_image_cache_stats.hits.saturating_add(1);
                return Some(Arc::clone(cached));
            }
        }
        self.scaled_image_cache_stats.misses =
            self.scaled_image_cache_stats.misses.saturating_add(1);
        let scaled = ImagePainter::scale_axis_aligned_default_rgb(
            image,
            target_width,
            target_height,
            high_quality,
        )?;
        let scaled = Arc::new(scaled);
        insert_scaled_image_cache_entry(
            &mut self.scaled_image_cache,
            &mut self.scaled_image_cache_order,
            &mut self.scaled_image_cache_bytes,
            cache_key,
            Arc::clone(&scaled),
        );
        Some(scaled)
    }

    #[allow(clippy::too_many_arguments)]
    fn scheduled_decode_inline_image(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
        bpc: u8,
        color_space: &str,
        filters: &[&str],
        decode_params: &[Option<PdfDictionary>],
    ) -> Result<RawImage> {
        let estimated = estimate_inline_image_decode_bytes(data.len(), width, height, bpc);
        self.decode_scheduler.run(
            estimated,
            &self.cancel,
            "renderer inline image decode",
            || {
                ImageDecoder::decode_inline_with_param_array(
                    data,
                    width,
                    height,
                    bpc,
                    color_space,
                    filters,
                    decode_params,
                    &crate::filters::DecodeLimits::default(),
                )
            },
        )
    }

    fn scheduled_load_smask(
        &self,
        image_ref: &ImageReference,
        raw: RawImage,
    ) -> Result<Option<RawImage>> {
        let estimated = u64::from(image_ref.width)
            .saturating_mul(u64::from(image_ref.height))
            .saturating_mul(2)
            .max(raw.byte_count() as u64)
            .max(1);
        self.decode_scheduler.run(
            estimated,
            &self.cancel,
            "renderer soft mask image decode",
            || SmaskLoader::load_and_combine(image_ref, raw, self.engine.document().reader()),
        )
    }

    fn scheduled_load_explicit_image_mask(
        &mut self,
        image_ref: &ImageReference,
        image_dict: &PdfDictionary,
        raw: RawImage,
    ) -> Result<Option<RawImage>> {
        let Some(mask) = image_dict.get("Mask") else {
            return Ok(None);
        };
        match mask {
            PdfObject::Array(items) => Ok(apply_color_key_image_mask(
                raw,
                image_ref.bits_per_component,
                &image_ref.color_space,
                items,
            )),
            PdfObject::Reference { number, generation } => {
                let reader = self.engine.document().reader();
                let mask_obj = reader.get_object(*number, *generation)?;
                let PdfObject::Stream {
                    dict: mask_dict, ..
                } = mask_obj
                else {
                    return Ok(None);
                };
                let mask_ref = ImageReference {
                    page_number: image_ref.page_number,
                    xobject_name: format!("{}_mask", image_ref.xobject_name),
                    object_number: *number,
                    generation_number: *generation,
                    width: positive_u32(
                        mask_dict
                            .get_integer("Width")
                            .or_else(|| mask_dict.get_integer("W")),
                        image_ref.width,
                    ),
                    height: positive_u32(
                        mask_dict
                            .get_integer("Height")
                            .or_else(|| mask_dict.get_integer("H")),
                        image_ref.height,
                    ),
                    bits_per_component: mask_dict
                        .get_integer("BitsPerComponent")
                        .or_else(|| mask_dict.get_integer("BPC"))
                        .unwrap_or(1)
                        .clamp(0, 16) as u8,
                    color_space: extract_color_space_name(&mask_dict),
                    filter: extract_filter_names(&mask_dict),
                    is_inline: false,
                    is_mask: mask_dict
                        .get_bool("ImageMask")
                        .or_else(|| mask_dict.get_bool("IM"))
                        .unwrap_or(false),
                    is_smask: false,
                    inline_data: None,
                };
                let mask_raw =
                    self.scheduled_decode_image(&mask_ref, "renderer explicit image mask decode")?;
                combine_explicit_image_mask(
                    raw,
                    mask_raw.as_ref(),
                    mask_ref.is_mask,
                    image_mask_paints_ones(&mask_dict),
                )
                .map(Some)
            }
            _ => Ok(None),
        }
    }

    fn reserve_offscreen_surface(
        &self,
        width: u32,
        height: u32,
        context: &str,
    ) -> Result<DecodeMemoryToken> {
        self.decode_scheduler.reserve_memory(
            estimate_rgba_surface_bytes(width, height),
            &self.cancel,
            context,
        )
    }

    fn take_transparent_offscreen_buffer(
        &mut self,
        width: u32,
        height: u32,
        render_mode: RenderMode,
    ) -> PixelBuffer {
        if let Some(index) = self
            .offscreen_buffer_pool
            .iter()
            .position(|buf| buf.width == width && buf.height == height)
        {
            let mut buf = self.offscreen_buffer_pool.swap_remove(index);
            if buf.reset_transparent_for_reuse(width, height, render_mode) {
                return buf;
            }
        }
        PixelBuffer::new_transparent_with_mode(width, height, render_mode)
    }

    fn take_filled_offscreen_buffer(
        &mut self,
        width: u32,
        height: u32,
        color: PixelColor,
        render_mode: RenderMode,
    ) -> PixelBuffer {
        if let Some(index) = self
            .offscreen_buffer_pool
            .iter()
            .position(|buf| buf.width == width && buf.height == height)
        {
            let mut buf = self.offscreen_buffer_pool.swap_remove(index);
            if buf.reset_filled_for_reuse(width, height, color, render_mode) {
                return buf;
            }
        }
        PixelBuffer::new_filled_with_mode(width, height, color, render_mode)
    }

    fn recycle_offscreen_buffer(&mut self, mut buf: PixelBuffer) {
        const MAX_POOLED_BUFFERS: usize = 4;
        const MAX_POOLED_BYTES: usize = 64 * 1024 * 1024;
        let bytes = (buf.width as usize)
            .saturating_mul(buf.height as usize)
            .saturating_mul(4);
        if bytes == 0
            || bytes > MAX_POOLED_BYTES
            || self.offscreen_buffer_pool.len() >= MAX_POOLED_BUFFERS
        {
            return;
        }
        if !buf.reset_transparent_for_reuse(buf.width, buf.height, buf.render_mode()) {
            return;
        }
        self.offscreen_buffer_pool.push(buf);
    }

    fn absorb_child_render_caches(&mut self, child: &mut RenderState<'a>) {
        self.glyph_cache.absorb_from(std::mem::replace(
            &mut child.glyph_cache,
            GlyphCache::with_default_capacity(),
        ));

        merge_artifact_cache_counts(
            &mut self.glyph_mask_cache.stats,
            child.glyph_mask_cache.stats(),
        );
        for (key, mask) in std::mem::take(&mut child.glyph_mask_cache.entries) {
            self.glyph_mask_cache.insert(key, mask);
        }
        child.glyph_mask_cache.bytes = 0;

        merge_artifact_cache_counts(
            &mut self.type3_mask_cache.stats,
            child.type3_mask_cache.stats(),
        );
        for (key, mask) in std::mem::take(&mut child.type3_mask_cache.entries) {
            self.type3_mask_cache.insert(key, mask);
        }
        child.type3_mask_cache.bytes = 0;

        merge_artifact_cache_counts(
            &mut self.type3_rendered_cache.stats,
            child.type3_rendered_cache.stats(),
        );
        for (key, glyph) in std::mem::take(&mut child.type3_rendered_cache.entries) {
            self.type3_rendered_cache.insert(key, glyph);
        }
        child.type3_rendered_cache.bytes = 0;

        merge_artifact_cache_counts(
            &mut self.path_fill_mask_cache.stats,
            child.path_fill_mask_cache.stats(),
        );
        for (key, mask) in std::mem::take(&mut child.path_fill_mask_cache.entries) {
            self.path_fill_mask_cache.insert(key, mask);
        }
        child.path_fill_mask_cache.bytes = 0;

        merge_artifact_cache_counts(
            &mut self.path_stroke_mask_cache.stats,
            child.path_stroke_mask_cache.stats(),
        );
        for (key, mask) in std::mem::take(&mut child.path_stroke_mask_cache.entries) {
            self.path_stroke_mask_cache.insert(key, mask);
        }
        child.path_stroke_mask_cache.bytes = 0;

        self.font_bytes_cache.extend(child.font_bytes_cache.drain());
        merge_artifact_cache_counts(
            &mut self.font_bytes_cache_stats,
            child.font_bytes_cache_stats,
        );
        child.font_bytes_cache_stats = RenderArtifactCacheStats::default();
        self.font_resolver_cache
            .extend(child.font_resolver_cache.drain());
        merge_artifact_cache_counts(
            &mut self.font_resolver_cache_stats,
            child.font_resolver_cache_stats,
        );
        child.font_resolver_cache_stats = RenderArtifactCacheStats::default();
        self.font_resource_key_cache
            .extend(child.font_resource_key_cache.drain());
        self.type3_geometry_cache
            .extend(child.type3_geometry_cache.drain());
        self.type3_charproc_cache
            .extend(child.type3_charproc_cache.drain());
        for (key, mask) in std::mem::take(&mut child.smask_group_cache) {
            insert_smask_group_cache_entry(
                &mut self.smask_group_cache,
                &mut self.smask_group_cache_order,
                &mut self.smask_group_cache_bytes,
                &mut self.smask_group_cache_stats,
                key,
                mask,
            );
        }
        merge_artifact_cache_counts(
            &mut self.smask_group_cache_stats,
            child.smask_group_cache_stats,
        );
        child.smask_group_cache_stats = RenderArtifactCacheStats::default();
        child.smask_group_cache_order.clear();
        child.smask_group_cache_bytes = 0;
        self.form_xobject_program_cache
            .extend(child.form_xobject_program_cache.drain());
        merge_artifact_cache_counts(
            &mut self.form_xobject_program_cache_stats,
            child.form_xobject_program_cache_stats,
        );
        child.form_xobject_program_cache_stats = RenderArtifactCacheStats::default();
        self.tiling_pattern_program_cache
            .extend(child.tiling_pattern_program_cache.drain());
        merge_artifact_cache_counts(
            &mut self.tiling_pattern_program_cache_stats,
            child.tiling_pattern_program_cache_stats,
        );
        child.tiling_pattern_program_cache_stats = RenderArtifactCacheStats::default();

        for (key, raw) in std::mem::take(&mut child.image_xobject_cache) {
            let raw_bytes = raw.byte_count();
            insert_image_xobject_cache_entry(
                &mut self.image_xobject_cache,
                &mut self.image_xobject_cache_order,
                &mut self.image_xobject_cache_bytes,
                key,
                raw,
                raw_bytes,
                RENDER_DOCUMENT_IMAGE_CACHE_MAX_BYTES,
            );
        }
        merge_artifact_cache_counts(
            &mut self.image_xobject_cache_stats,
            child.image_xobject_cache_stats,
        );
        child.image_xobject_cache_stats = RenderArtifactCacheStats::default();
        child.image_xobject_cache_order.clear();
        child.image_xobject_cache_bytes = 0;

        for (key, raw) in std::mem::take(&mut child.scaled_image_cache) {
            insert_scaled_image_cache_entry(
                &mut self.scaled_image_cache,
                &mut self.scaled_image_cache_order,
                &mut self.scaled_image_cache_bytes,
                key,
                raw,
            );
        }
        merge_artifact_cache_counts(
            &mut self.scaled_image_cache_stats,
            child.scaled_image_cache_stats,
        );
        child.scaled_image_cache_stats = RenderArtifactCacheStats::default();
        child.scaled_image_cache_order.clear();
        child.scaled_image_cache_bytes = 0;

        for (key, bytes) in std::mem::take(&mut child.shading_mesh_cache) {
            insert_shading_mesh_cache_entry(
                &mut self.shading_mesh_cache,
                &mut self.shading_mesh_cache_order,
                &mut self.shading_mesh_cache_bytes,
                key,
                bytes,
            );
        }
        merge_artifact_cache_counts(
            &mut self.shading_mesh_cache_stats,
            child.shading_mesh_cache_stats,
        );
        child.shading_mesh_cache_stats = RenderArtifactCacheStats::default();
        child.shading_mesh_cache_order.clear();
        child.shading_mesh_cache_bytes = 0;

        for pooled in child.offscreen_buffer_pool.drain(..) {
            self.recycle_offscreen_buffer(pooled);
        }
    }

    fn shading_mesh_data(
        &mut self,
        shading_obj: &PdfObject,
        shading_dict: &PdfDictionary,
        reader: &crate::reader::PdfReader,
    ) -> Option<Arc<Vec<u8>>> {
        let st = shading_dict.get_integer("ShadingType").unwrap_or(0);
        if !(4..=7).contains(&st) {
            return None;
        }
        let cache_key = shading_mesh_cache_key(shading_obj, st);
        if let Some(key) = cache_key.as_deref() {
            if let Some(cached) = self.shading_mesh_cache.get(key) {
                touch_shading_mesh_cache_key(&mut self.shading_mesh_cache_order, key);
                self.shading_mesh_cache_stats.hits =
                    self.shading_mesh_cache_stats.hits.saturating_add(1);
                return Some(Arc::clone(cached));
            }
            self.shading_mesh_cache_stats.misses =
                self.shading_mesh_cache_stats.misses.saturating_add(1);
        }
        let (dict, raw) = resolve_to_stream(shading_obj, reader)?;
        let stream_obj = PdfObject::Stream { dict, raw };
        let bytes = self
            .scheduled_decode_stream(&stream_obj, reader, "renderer mesh shading stream decode")
            .ok()?;
        let bytes = Arc::new(bytes);
        if let Some(key) = cache_key {
            insert_shading_mesh_cache_entry(
                &mut self.shading_mesh_cache,
                &mut self.shading_mesh_cache_order,
                &mut self.shading_mesh_cache_bytes,
                key,
                Arc::clone(&bytes),
            );
        }
        Some(bytes)
    }

    fn cached_form_xobject_program(
        &mut self,
        name: &str,
        obj_num: u32,
        gen_num: u16,
    ) -> Option<Arc<FormXObjectProgram>> {
        let cache_key = form_xobject_program_cache_key(obj_num, gen_num);
        if let Some(cached) = self.form_xobject_program_cache.get(&cache_key) {
            self.form_xobject_program_cache_stats.hits =
                self.form_xobject_program_cache_stats.hits.saturating_add(1);
            return cached.as_ref().map(Arc::clone);
        }
        self.form_xobject_program_cache_stats.misses = self
            .form_xobject_program_cache_stats
            .misses
            .saturating_add(1);

        let reader = self.engine.document().reader();
        let (form_dict, raw_bytes) = match reader.get_object(obj_num, gen_num) {
            Ok(PdfObject::Stream { dict, raw }) => (dict, raw),
            Ok(_) => {
                log::warn!("PageRenderer: Form XObject '{}' is not a stream", name);
                self.form_xobject_program_cache.insert(cache_key, None);
                return None;
            }
            Err(err) => {
                log::warn!(
                    "PageRenderer: failed to fetch Form XObject '{}': {}",
                    name,
                    err
                );
                self.form_xobject_program_cache.insert(cache_key, None);
                return None;
            }
        };

        if form_dict.get_name("Subtype") != Some("Form") {
            log::debug!(
                "PageRenderer: XObject '{}' is not /Subtype /Form, skipping",
                name
            );
            self.form_xobject_program_cache.insert(cache_key, None);
            return None;
        }

        let stream_obj = PdfObject::Stream {
            dict: form_dict.clone(),
            raw: raw_bytes,
        };
        let content_bytes = match self.scheduled_decode_stream(
            &stream_obj,
            reader,
            "renderer Form XObject program stream decode",
        ) {
            Ok(bytes) => bytes,
            Err(err) => {
                log::warn!(
                    "PageRenderer: Form XObject '{}' stream decode failed: {}",
                    name,
                    err
                );
                self.form_xobject_program_cache.insert(cache_key, None);
                return None;
            }
        };
        let ops = match crate::content::ContentParser::parse(&content_bytes) {
            Ok(ops) => ops,
            Err(err) => {
                log::warn!(
                    "PageRenderer: Form XObject '{}' content parse failed: {}",
                    name,
                    err
                );
                self.form_xobject_program_cache.insert(cache_key, None);
                return None;
            }
        };
        let resources = form_dict
            .get("Resources")
            .map(|res_obj| crate::engine::parse_resources_from_obj(res_obj, reader));
        let program = Arc::new(FormXObjectProgram {
            form_matrix: extract_form_matrix(&form_dict),
            bbox: extract_bbox(&form_dict),
            is_transparency_group: is_transparency_group(&form_dict),
            dict: form_dict,
            ops: Arc::new(ops),
            resources,
        });
        self.form_xobject_program_cache
            .insert(cache_key, Some(Arc::clone(&program)));
        Some(program)
    }

    fn cached_tiling_pattern_program(
        &mut self,
        cache_key: &str,
        pat_dict: &PdfDictionary,
        raw_bytes: Vec<u8>,
    ) -> Option<Arc<Vec<ContentOperation>>> {
        if let Some(cached) = self.tiling_pattern_program_cache.get(cache_key) {
            self.tiling_pattern_program_cache_stats.hits = self
                .tiling_pattern_program_cache_stats
                .hits
                .saturating_add(1);
            return cached.as_ref().map(Arc::clone);
        }
        self.tiling_pattern_program_cache_stats.misses = self
            .tiling_pattern_program_cache_stats
            .misses
            .saturating_add(1);

        let reader = self.engine.document().reader();
        let stream_obj = PdfObject::Stream {
            dict: pat_dict.clone(),
            raw: raw_bytes,
        };
        let content_bytes = match self.scheduled_decode_stream(
            &stream_obj,
            reader,
            "renderer tiling pattern program stream decode",
        ) {
            Ok(bytes) => bytes,
            Err(err) => {
                log::warn!("tiling pattern: content decode failed: {err}");
                self.tiling_pattern_program_cache
                    .insert(cache_key.to_string(), None);
                return None;
            }
        };
        let ops = match crate::content::ContentParser::parse(&content_bytes) {
            Ok(ops) => Arc::new(ops),
            Err(err) => {
                log::warn!("tiling pattern: content parse failed: {err}");
                self.tiling_pattern_program_cache
                    .insert(cache_key.to_string(), None);
                return None;
            }
        };
        self.tiling_pattern_program_cache
            .insert(cache_key.to_string(), Some(Arc::clone(&ops)));
        Some(ops)
    }

    fn replay_display_list(&mut self, list: &DisplayList) {
        const CANCEL_CHECK_INTERVAL: usize = 64;
        for (i, op) in list.ops.iter().enumerate() {
            if i % CANCEL_CHECK_INTERVAL == 0 && self.cancel.is_cancelled() {
                return;
            }
            self.replay_display_op(op);
        }
    }

    fn replay_display_op(&mut self, op: &DisplayOp) {
        match op {
            DisplayOp::Save => {
                let node = self.clip_dag.intern_option(self.buf.clip_mask());
                self.clip_stack.push(node);
                self.smask_stack.push(self.buf.smask_mask().cloned());
                self.gs.push();
            }
            DisplayOp::Restore => {
                self.gs.pop();
                self.sync_blend_mode();
                match self.clip_stack.pop() {
                    Some(saved) => {
                        let mask = match &saved.state {
                            ClipState::Full => None,
                            _ => Some(saved.materialize(self.buf.width, self.buf.height).clone()),
                        };
                        self.buf.restore_clip(mask);
                    }
                    None => log::warn!("DisplayList replay: restore with empty clip stack"),
                }
                match self.smask_stack.pop() {
                    Some(saved) => self.buf.restore_smask(saved),
                    None => log::warn!("DisplayList replay: restore with empty SMask stack"),
                }
            }
            DisplayOp::Clip {
                path,
                ctm,
                rule,
                bounds,
            } => {
                if bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.intersects_viewport(&self.viewport))
                {
                    self.buf
                        .set_clip(ClipMask::empty(self.buf.width, self.buf.height));
                    return;
                }
                if let Some((x, y, width, height)) =
                    axis_aligned_integer_rect(path, ctm, &self.viewport)
                {
                    let clip = if x <= 0
                        && y <= 0
                        && x.saturating_add(width) >= self.buf.width as i32
                        && y.saturating_add(height) >= self.buf.height as i32
                    {
                        ClipMask::all_visible(self.buf.width, self.buf.height)
                    } else {
                        ClipMask::from_visible_rect(
                            self.buf.width,
                            self.buf.height,
                            x,
                            y,
                            width,
                            height,
                        )
                    };
                    self.buf.set_clip(clip);
                    return;
                }
                let flat = flatten_path(path, ctm, &self.viewport, 0.5);
                let clip = ClipMask::from_path(&flat, self.buf.width, self.buf.height, *rule);
                self.buf.set_clip(clip);
            }
            DisplayOp::FillPath {
                path,
                state,
                rule,
                bounds,
            } => {
                if bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.intersects_viewport(&self.viewport))
                {
                    return;
                }
                let saved_blend = self.buf.blend_mode;
                self.buf.blend_mode = state.blend_mode;
                if !paint_cached_path_fill(
                    &mut self.path_fill_mask_cache,
                    &mut self.buf,
                    &self.viewport,
                    path,
                    &state.ctm,
                    *rule,
                    state.fill_color,
                ) {
                    // Cache miss/general-path fallback only: normalized
                    // display-list replay attempts bounded path-mask reuse
                    // before direct path painting. Use the scanline-capable
                    // bounded fast path so retained replay does not fall back
                    // to accumulator-heavy rasterization on complex vector
                    // pages.
                    let _ = PathPainter::fill_fast_cancellable(
                        &mut self.buf,
                        path,
                        &state.ctm,
                        &self.viewport,
                        state.fill_color,
                        *rule,
                        &self.cancel,
                    );
                }
                self.buf.blend_mode = saved_blend;
            }
            DisplayOp::StrokePath {
                path,
                state,
                bounds,
            } => {
                if bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.intersects_viewport(&self.viewport))
                {
                    return;
                }
                let saved_blend = self.buf.blend_mode;
                self.buf.blend_mode = state.blend_mode;
                if !paint_cached_path_stroke(
                    &mut self.path_stroke_mask_cache,
                    &mut self.buf,
                    &self.viewport,
                    path,
                    &state.ctm,
                    state.stroke_color,
                    state.line_width,
                    &state.dash,
                    &state.line_cap,
                    &state.line_join,
                    state.miter_limit,
                ) {
                    // Cache miss/general-path fallback only: normalized
                    // display-list replay attempts bounded stroke-mask reuse
                    // before direct path painting. Use the scanline-capable
                    // bounded fast path so retained replay does not fall back
                    // to accumulator-heavy stroke rasterization.
                    let _ = PathPainter::stroke_with_style_fast_cancellable(
                        &mut self.buf,
                        path,
                        &state.ctm,
                        &self.viewport,
                        state.stroke_color,
                        state.line_width,
                        &state.dash,
                        &state.line_cap,
                        &state.line_join,
                        state.miter_limit,
                        &self.cancel,
                    );
                }
                self.buf.blend_mode = saved_blend;
            }
            DisplayOp::StateOp { op, .. } => self.dispatch(op),
            DisplayOp::NativeTextOp { op, bounds, .. } => {
                if bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.intersects_viewport(&self.viewport))
                    && text_showing_operator(op)
                    && !text_rendering_mode_clips(self.gs.text.rendering_mode)
                {
                    self.dispatch_text_showing_without_paint(op);
                    return;
                }
                self.replay_native_text_op(op);
            }
            DisplayOp::NativeShadingOp { op, bounds, .. } => {
                if bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.intersects_viewport(&self.viewport))
                {
                    return;
                }
                self.replay_native_shading_op(op);
            }
            DisplayOp::NativeFormXObject { op, bounds, .. } => {
                if bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.intersects_viewport(&self.viewport))
                {
                    return;
                }
                self.replay_native_xobject_op(op);
            }
            DisplayOp::NativeImageXObject { op, bounds, .. } => {
                if bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.intersects_viewport(&self.viewport))
                {
                    return;
                }
                self.replay_native_xobject_op(op);
            }
            DisplayOp::NativePatternPathOp { ops, bounds, .. } => {
                if bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.intersects_viewport(&self.viewport))
                {
                    return;
                }
                self.replay_native_pattern_path_ops(ops);
            }
            DisplayOp::NativeInlineImage { ops, bounds, .. } => {
                if bounds
                    .as_ref()
                    .is_some_and(|bounds| !bounds.intersects_viewport(&self.viewport))
                {
                    return;
                }
                self.replay_native_inline_image_ops(ops);
            }
        }
    }

    fn replay_native_xobject_op(&mut self, op: &ContentOperation) {
        if !self.oc_current_visible {
            return;
        }
        if let Some(name) = op.name(0) {
            self.handle_do(name);
        }
    }

    fn replay_native_shading_op(&mut self, op: &ContentOperation) {
        if !self.oc_current_visible {
            return;
        }
        if let Some(name) = op.name(0) {
            self.handle_sh(name.to_string());
        }
    }

    fn replay_native_text_op(&mut self, op: &ContentOperation) {
        if !self.oc_current_visible {
            return;
        }
        match op.operator.as_str() {
            "Tj" => {
                if let Some(bytes) = op.string_bytes(0) {
                    self.render_text_string(bytes);
                }
            }
            "TJ" => self.render_text_array(op),
            "'" => {
                self.move_to_next_text_line();
                if let Some(bytes) = op.string_bytes(0) {
                    self.render_text_string(bytes);
                }
            }
            "\"" => {
                if let Some(word_spacing) = op.number(0) {
                    self.gs.text.word_spacing = word_spacing;
                }
                if let Some(char_spacing) = op.number(1) {
                    self.gs.text.char_spacing = char_spacing;
                }
                self.move_to_next_text_line();
                if let Some(bytes) = op.string_bytes(2) {
                    self.render_text_string(bytes);
                }
            }
            _ => self.dispatch(op),
        }
    }

    fn replay_native_pattern_path_ops(&mut self, ops: &[ContentOperation]) {
        if !self.oc_current_visible {
            return;
        }
        if ops.is_empty() {
            return;
        }
        for op in &ops[..ops.len().saturating_sub(1)] {
            match op.operator.as_str() {
                "m" => {
                    if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                        self.path.move_to(x, y);
                    }
                }
                "l" => {
                    if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                        self.path.line_to(x, y);
                    }
                }
                "c" => {
                    if let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x3), Some(y3)) = (
                        op.number(0),
                        op.number(1),
                        op.number(2),
                        op.number(3),
                        op.number(4),
                        op.number(5),
                    ) {
                        self.path.curve_to(x1, y1, x2, y2, x3, y3);
                    }
                }
                "v" => {
                    if let (Some(x2), Some(y2), Some(x3), Some(y3)) =
                        (op.number(0), op.number(1), op.number(2), op.number(3))
                    {
                        let (cx, cy) = self.path.current_point.unwrap_or((0.0, 0.0));
                        self.path.curve_to(cx, cy, x2, y2, x3, y3);
                    }
                }
                "y" => {
                    if let (Some(x1), Some(y1), Some(x3), Some(y3)) =
                        (op.number(0), op.number(1), op.number(2), op.number(3))
                    {
                        self.path.curve_to(x1, y1, x3, y3, x3, y3);
                    }
                }
                "h" => self.path.close(),
                "re" => {
                    if let (Some(x), Some(y), Some(w), Some(h)) =
                        (op.number(0), op.number(1), op.number(2), op.number(3))
                    {
                        self.path.rect(x, y, w, h);
                    }
                }
                _ => {
                    log::debug!(
                        "Display-list native path replay skipped unexpected operator '{}'",
                        op.operator
                    );
                    self.path.clear();
                    return;
                }
            }
        }
        let paint_op = &ops[ops.len() - 1];
        match paint_op.operator.as_str() {
            "S" => self.stroke_and_clear(),
            "s" => {
                self.path.close();
                self.stroke_and_clear();
            }
            "f" | "F" => self.fill_and_clear(FillRule::NonZero),
            "f*" => self.fill_and_clear(FillRule::EvenOdd),
            "B" => self.fill_stroke_and_clear(FillRule::NonZero),
            "B*" => self.fill_stroke_and_clear(FillRule::EvenOdd),
            "b" => {
                self.path.close();
                self.fill_stroke_and_clear(FillRule::NonZero);
            }
            "b*" => {
                self.path.close();
                self.fill_stroke_and_clear(FillRule::EvenOdd);
            }
            _ => {
                log::debug!(
                    "Display-list native path replay skipped unexpected paint operator '{}'",
                    paint_op.operator
                );
                self.path.clear();
            }
        }
    }

    fn replay_native_inline_image_ops(&mut self, ops: &[ContentOperation]) {
        if !self.oc_current_visible {
            return;
        }
        for op in ops {
            match op.operator.as_str() {
                "ID" => self.pending_inline = Some(op.operands.clone()),
                "inline_image_data" => {
                    if let (Some(params), Some(bytes)) =
                        (self.pending_inline.take(), op.string_bytes(0))
                    {
                        self.paint_inline_image(&params, bytes);
                    }
                }
                _ => {
                    log::debug!(
                        "Display-list native inline-image replay ignored unexpected operator '{}'",
                        op.operator
                    );
                }
            }
        }
    }

    fn dispatch(&mut self, op: &ContentOperation) {
        match op.operator.as_str() {
            "BMC" | "BDC" => {
                self.push_optional_content_visibility(op);
                return;
            }
            "EMC" => {
                self.pop_optional_content_visibility();
                return;
            }
            _ => {}
        }
        if !self.oc_current_visible {
            return;
        }
        match op.operator.as_str() {
            "m" => {
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    self.path.move_to(x, y);
                }
            }
            "l" => {
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    self.path.line_to(x, y);
                }
            }
            "c" => {
                if let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x3), Some(y3)) = (
                    op.number(0),
                    op.number(1),
                    op.number(2),
                    op.number(3),
                    op.number(4),
                    op.number(5),
                ) {
                    self.path.curve_to(x1, y1, x2, y2, x3, y3);
                }
            }
            "v" => {
                if let (Some(x2), Some(y2), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    let (cx, cy) = self.path.current_point.unwrap_or((0.0, 0.0));
                    self.path.curve_to(cx, cy, x2, y2, x3, y3);
                }
            }
            "y" => {
                if let (Some(x1), Some(y1), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.path.curve_to(x1, y1, x3, y3, x3, y3);
                }
            }
            "h" => self.path.close(),
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.path.rect(x, y, w, h);
                }
            }
            "S" => self.stroke_and_clear(),
            "s" => {
                self.path.close();
                self.stroke_and_clear();
            }
            "f" | "F" => self.fill_and_clear(FillRule::NonZero),
            "f*" => self.fill_and_clear(FillRule::EvenOdd),
            "B" => self.fill_stroke_and_clear(FillRule::NonZero),
            "B*" => self.fill_stroke_and_clear(FillRule::EvenOdd),
            "b" => {
                self.path.close();
                self.fill_stroke_and_clear(FillRule::NonZero);
            }
            "b*" => {
                self.path.close();
                self.fill_stroke_and_clear(FillRule::EvenOdd);
            }
            "n" => {
                self.apply_pending_clip();
                self.path.clear();
            }
            "W" => self.pending_clip = Some(FillRule::NonZero),
            "W*" => self.pending_clip = Some(FillRule::EvenOdd),
            "q" => {
                let node = self.clip_dag.intern_option(self.buf.clip_mask());
                self.clip_stack.push(node);
                self.smask_stack.push(self.buf.smask_mask().cloned());
                self.gs.process(op);
            }
            "Q" => {
                self.gs.process(op);
                self.sync_blend_mode();
                match self.clip_stack.pop() {
                    Some(saved) => {
                        let mask = match &saved.state {
                            ClipState::Full => None,
                            _ => Some(saved.materialize(self.buf.width, self.buf.height).clone()),
                        };
                        self.buf.restore_clip(mask);
                    }
                    None => log::warn!("PageRenderer: Q with empty clip stack"),
                }
                match self.smask_stack.pop() {
                    Some(saved) => self.buf.restore_smask(saved),
                    None => log::warn!("PageRenderer: Q with empty SMask stack"),
                }
            }
            "Do" => {
                if let Some(name) = op.name(0) {
                    self.handle_do(name);
                }
            }
            "gs" => {
                self.gs.process(op);
                self.apply_ext_g_state(op);
            }
            "Tj" => {
                if let Some(bytes) = op.string_bytes(0) {
                    self.render_text_string(bytes);
                }
            }
            "TJ" => self.render_text_array(op),
            "'" => {
                self.move_to_next_text_line();
                if let Some(bytes) = op.string_bytes(0) {
                    self.render_text_string(bytes);
                }
            }
            "\"" => {
                if let Some(word_spacing) = op.number(0) {
                    self.gs.text.word_spacing = word_spacing;
                }
                if let Some(char_spacing) = op.number(1) {
                    self.gs.text.char_spacing = char_spacing;
                }
                self.move_to_next_text_line();
                if let Some(bytes) = op.string_bytes(2) {
                    self.render_text_string(bytes);
                }
            }
            "BT" => {
                self.pending_text_clip = None;
                self.gs.process(op);
            }
            "ET" => {
                self.apply_pending_text_clip();
                self.gs.process(op);
            }
            "Tf" | "Td" | "TD" | "Tm" | "T*" | "Tc" | "Tw" | "Tz" | "TL" | "Tr" | "Ts" | "cm"
            | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "G" | "g" | "RG" | "rg" | "K" | "k"
            | "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" => {
                self.gs.process(op);
            }
            "sh" => {
                if let Some(name) = op.name(0) {
                    self.handle_sh(name.to_string());
                }
            }
            "ID" => {
                // Stash the inline image parameters; the pixel bytes arrive in
                // the following `inline_image_data` operation.
                self.pending_inline = Some(op.operands.clone());
            }
            "inline_image_data" => {
                if let (Some(params), Some(bytes)) =
                    (self.pending_inline.take(), op.string_bytes(0))
                {
                    self.paint_inline_image(&params, bytes);
                }
            }
            "MP" | "DP" | "BX" | "EX" | "BI" | "EI" => {}
            _ => self.gs.process(op),
        }
    }

    fn dispatch_text_showing_without_paint(&mut self, op: &ContentOperation) {
        let saved_mode = self.gs.text.rendering_mode;
        self.gs.text.rendering_mode = 3;
        self.dispatch(op);
        self.gs.text.rendering_mode = saved_mode;
    }

    fn push_optional_content_visibility(&mut self, op: &ContentOperation) {
        let parent_visible = self.oc_current_visible;
        let mut visible = parent_visible;
        if op.operator == "BDC" && op.name(0) == Some("OC") {
            visible = op
                .name(1)
                .map(|name| {
                    self.optional_content.is_resource_visible(
                        name,
                        &self.resources.properties,
                        self.engine.document().reader(),
                    )
                })
                .unwrap_or(true);
            visible = parent_visible && visible;
        }
        self.oc_visibility_stack.push(parent_visible);
        self.oc_current_visible = visible;
    }

    fn pop_optional_content_visibility(&mut self) {
        self.oc_current_visible = self.oc_visibility_stack.pop().unwrap_or(true);
    }

    fn ctm(&self) -> Transform2D {
        Transform2D::from(self.gs.ctm)
    }

    /// Push the graphics-state blend mode onto the pixel buffer.
    fn sync_blend_mode(&mut self) {
        self.buf.blend_mode = self.gs.blend_mode;
    }

    fn fill_pixel_color(&self) -> PixelColor {
        self.resolve_paint_color(&self.gs.fill_color, self.gs.fill_alpha as f32)
    }

    fn stroke_pixel_color(&self) -> PixelColor {
        self.resolve_paint_color(&self.gs.stroke_color, self.gs.stroke_alpha as f32)
    }

    fn record_plate_contribution(
        &mut self,
        color: &crate::content::state::Color,
        alpha: f32,
        operation: &str,
    ) {
        let ColorSpace::Named(name) = &color.space else {
            return;
        };
        let Some(space_obj) = self.resources.color_spaces.get(name).cloned() else {
            return;
        };
        self.record_plate_contribution_for_space_obj(
            &space_obj,
            &color.components,
            alpha,
            Some(format!("page {} color space /{}", self.page_number, name)),
            operation,
        );
    }

    fn record_plate_contribution_for_space_obj(
        &mut self,
        space_obj: &PdfObject,
        components: &[f64],
        alpha: f32,
        object: Option<String>,
        operation: &str,
    ) {
        let reader = self.engine.document().reader();
        let overprint = prepress::OverprintStateModel::for_paint(
            self.gs.fill_overprint,
            self.gs.stroke_overprint,
            self.gs.overprint_mode,
            operation,
            prepress::color_space_label(space_obj, reader),
            components,
            alpha,
            object.clone(),
        );
        let contributions = prepress::plate_contributions_for_color_space_with_overprint(
            space_obj,
            components,
            alpha,
            reader,
            object,
            operation,
            Some(self.page_number),
            &overprint,
        );
        self.separation_framebuffer.record_all(contributions);
    }

    fn record_named_plate_sample(
        &mut self,
        color_space_name: &str,
        alpha: f32,
        object: String,
        operation: &str,
    ) {
        let Some(space_obj) = self.resources.color_spaces.get(color_space_name).cloned() else {
            return;
        };
        let reader = self.engine.document().reader();
        let components = plate_sample_components(&space_obj, reader);
        if components.is_empty() {
            return;
        }
        self.record_plate_contribution_for_space_obj(
            &space_obj,
            &components,
            alpha,
            Some(object),
            operation,
        );
    }

    fn record_image_plate_sample(
        &mut self,
        dict: &PdfDictionary,
        image_ref: &ImageReference,
        object: String,
        operation: &str,
    ) {
        if image_ref.is_mask {
            let fill_color_state = self.gs.fill_color.clone();
            self.record_plate_contribution(&fill_color_state, 1.0, operation);
            return;
        }
        let Some(color_space_obj) = dict.get("ColorSpace").or_else(|| dict.get("CS")) else {
            return;
        };
        match color_space_obj {
            PdfObject::Name(name) => {
                self.record_named_plate_sample(name, self.gs.fill_alpha as f32, object, operation);
            }
            PdfObject::Array(_) | PdfObject::Reference { .. } => {
                let reader = self.engine.document().reader();
                let components = plate_sample_components(color_space_obj, reader);
                if components.is_empty() {
                    return;
                }
                self.record_plate_contribution_for_space_obj(
                    color_space_obj,
                    &components,
                    self.gs.fill_alpha as f32,
                    Some(object),
                    operation,
                );
            }
            _ => {}
        }
    }

    fn record_shading_plate_sample(
        &mut self,
        shading_dict: &PdfDictionary,
        object: String,
        operation: &str,
    ) {
        let Some(color_space_obj) = shading_dict
            .get("ColorSpace")
            .or_else(|| shading_dict.get("CS"))
        else {
            return;
        };
        match color_space_obj {
            PdfObject::Name(name) => {
                self.record_named_plate_sample(name, self.gs.fill_alpha as f32, object, operation);
            }
            PdfObject::Array(_) | PdfObject::Reference { .. } => {
                let reader = self.engine.document().reader();
                let components = plate_sample_components(color_space_obj, reader);
                if components.is_empty() {
                    return;
                }
                self.record_plate_contribution_for_space_obj(
                    color_space_obj,
                    &components,
                    self.gs.fill_alpha as f32,
                    Some(object),
                    operation,
                );
            }
            _ => {}
        }
    }

    fn record_pattern_caller_plate_sample(&mut self, operation: &str) {
        let ColorSpace::Named(name) = &self.gs.fill_color.space else {
            return;
        };
        let Some(space_obj) = self.resources.color_spaces.get(name).cloned() else {
            return;
        };
        let reader = self.engine.document().reader();
        let resolved = match reader.resolve(space_obj.clone()) {
            Ok(obj) => obj,
            Err(_) => space_obj,
        };
        let PdfObject::Array(arr) = resolved else {
            return;
        };
        if arr.first().and_then(PdfObject::as_name) != Some("Pattern") {
            return;
        }
        let Some(base_space) = arr.get(1) else {
            return;
        };
        match base_space {
            PdfObject::Name(base_name) => {
                let Some(base_obj) = self.resources.color_spaces.get(base_name).cloned() else {
                    return;
                };
                self.record_plate_contribution_for_space_obj(
                    &base_obj,
                    &self.gs.fill_color.components.clone(),
                    self.gs.fill_alpha as f32,
                    Some(format!(
                        "page {} pattern base /{}",
                        self.page_number, base_name
                    )),
                    operation,
                );
            }
            PdfObject::Array(_) | PdfObject::Reference { .. } => {
                self.record_plate_contribution_for_space_obj(
                    base_space,
                    &self.gs.fill_color.components.clone(),
                    self.gs.fill_alpha as f32,
                    Some(format!(
                        "page {} pattern base color space",
                        self.page_number
                    )),
                    operation,
                );
            }
            _ => {}
        }
    }

    /// Resolve a graphics-state colour to a device pixel colour. Device spaces go
    /// straight through [`ColorSpaceHandler`]; a `Named` space is looked up in the
    /// page resources and, if it is a `/Separation` or `/DeviceN` space, its tint
    /// transform is evaluated and converted via the alternate space.
    /// `/Separation /None` (and all-`/None` DeviceN) resolve to a fully
    /// transparent colour so the paint produces no marks.
    fn resolve_paint_color(&self, color: &crate::content::state::Color, alpha: f32) -> PixelColor {
        if let ColorSpace::Named(name) = &color.space {
            if let Some(space_obj) = self.resources.color_spaces.get(name) {
                let reader = self.engine.document().reader();
                match crate::render::colorspace::resolve_named_color(
                    space_obj,
                    &color.components,
                    alpha,
                    reader,
                ) {
                    crate::render::colorspace::NamedColor::Color(rc) => return rc.to_pixel_color(),
                    crate::render::colorspace::NamedColor::NoPaint => {
                        return crate::render::color::RenderColor::transparent().to_pixel_color();
                    }
                    crate::render::colorspace::NamedColor::Unhandled => {}
                }
            }
        }
        ColorSpaceHandler::to_render_color(color, alpha).to_pixel_color()
    }

    fn dash_state(&self) -> DashState {
        if self.gs.dash.pattern.is_empty() {
            DashState::solid()
        } else {
            DashState::new(self.gs.dash.pattern.clone(), self.gs.dash.phase)
        }
    }

    fn apply_pending_clip(&mut self) {
        if let Some(rule) = self.pending_clip.take() {
            let ctm = self.ctm();
            let flat = flatten_path(&self.path, &ctm, &self.viewport, 0.5);
            let clip = ClipMask::from_path(&flat, self.buf.width, self.buf.height, rule);
            self.buf.set_clip(clip);
        }
    }

    fn apply_pending_text_clip(&mut self) {
        if let Some(clip) = self.pending_text_clip.take() {
            self.buf.set_clip(clip);
        }
    }

    fn accumulate_text_clip(&mut self, glyph_path: &Path, glyph_ctm: &Transform2D) {
        let flat = flatten_path(glyph_path, glyph_ctm, &self.viewport, 0.25);
        let clip = ClipMask::from_path(&flat, self.buf.width, self.buf.height, FillRule::NonZero);
        self.accumulate_text_clip_mask(clip);
    }

    fn accumulate_text_clip_mask(&mut self, clip: ClipMask) {
        if let Some(existing) = &mut self.pending_text_clip {
            existing.union_with(&clip);
        } else {
            self.pending_text_clip = Some(clip);
        }
    }

    fn fail_closed_text_clip(&mut self) {
        let empty = ClipMask::from_path(
            &FlatPath::default(),
            self.buf.width,
            self.buf.height,
            FillRule::NonZero,
        );
        self.pending_text_clip = Some(empty);
    }

    fn stroke_and_clear(&mut self) {
        self.apply_pending_clip();
        let ctm = self.ctm();
        let stroke_color_state = self.gs.stroke_color.clone();
        self.record_plate_contribution(&stroke_color_state, self.gs.stroke_alpha as f32, "stroke");
        if self.is_pattern_stroke() {
            if let Some(pattern_name) = self.gs.stroke_pattern_name.clone() {
                self.paint_pattern_stroke(&pattern_name);
            } else {
                log::debug!("PageRenderer: Pattern stroke color space without a pattern name");
            }
            self.path.clear();
            return;
        }
        let color = self.stroke_pixel_color();
        let width = self.gs.line_width;
        let dash = self.dash_state();
        if self.gs.stroke_overprint {
            if let Some(cmyk) = device_cmyk_components(&self.gs.stroke_color) {
                PathPainter::stroke_device_cmyk_overprint_preview(
                    &mut self.buf,
                    &self.path,
                    &ctm,
                    &self.viewport,
                    cmyk,
                    self.gs.stroke_alpha as f32,
                    self.gs.overprint_mode,
                    width,
                    &dash,
                    &self.gs.line_cap,
                    &self.gs.line_join,
                    self.gs.miter_limit,
                );
                self.path.clear();
                return;
            }
        }
        if !paint_cached_path_stroke(
            &mut self.path_stroke_mask_cache,
            &mut self.buf,
            &self.viewport,
            &self.path,
            &ctm,
            color,
            width,
            &dash,
            &self.gs.line_cap,
            &self.gs.line_join,
            self.gs.miter_limit,
        ) {
            PathPainter::stroke_with_style_fast_cancellable(
                &mut self.buf,
                &self.path,
                &ctm,
                &self.viewport,
                color,
                width,
                &dash,
                &self.gs.line_cap,
                &self.gs.line_join,
                self.gs.miter_limit,
                &self.cancel,
            );
        }
        self.path.clear();
    }

    fn fill_and_clear(&mut self, rule: FillRule) {
        self.apply_pending_clip();
        if self.is_pattern_fill() {
            if let Some(pattern_name) = self.gs.fill_pattern_name.clone() {
                self.paint_pattern_fill(rule, &pattern_name);
            } else {
                log::debug!("PageRenderer: Pattern fill color space without a pattern name");
            }
            self.path.clear();
            return;
        }
        let ctm = self.ctm();
        let fill_color_state = self.gs.fill_color.clone();
        self.record_plate_contribution(&fill_color_state, self.gs.fill_alpha as f32, "fill");
        if self.gs.fill_overprint {
            if let Some(cmyk) = device_cmyk_components(&self.gs.fill_color) {
                PathPainter::fill_device_cmyk_overprint_preview(
                    &mut self.buf,
                    &self.path,
                    &ctm,
                    &self.viewport,
                    cmyk,
                    self.gs.fill_alpha as f32,
                    self.gs.overprint_mode,
                    rule,
                );
                self.path.clear();
                return;
            }
        }
        let color = self.fill_pixel_color();
        if !paint_cached_path_fill(
            &mut self.path_fill_mask_cache,
            &mut self.buf,
            &self.viewport,
            &self.path,
            &ctm,
            rule,
            color,
        ) {
            let _ = PathPainter::fill_fast_cancellable(
                &mut self.buf,
                &self.path,
                &ctm,
                &self.viewport,
                color,
                rule,
                &self.cancel,
            );
        }
        self.path.clear();
    }

    fn fill_stroke_and_clear(&mut self, rule: FillRule) {
        self.apply_pending_clip();
        let ctm = self.ctm();
        let fill_color_state = self.gs.fill_color.clone();
        let stroke_color_state = self.gs.stroke_color.clone();
        self.record_plate_contribution(&fill_color_state, self.gs.fill_alpha as f32, "fill");
        if self.is_pattern_fill() {
            if let Some(pattern_name) = self.gs.fill_pattern_name.clone() {
                self.paint_pattern_fill(rule, &pattern_name);
            }
        } else {
            if self.gs.fill_overprint {
                if let Some(cmyk) = device_cmyk_components(&self.gs.fill_color) {
                    PathPainter::fill_device_cmyk_overprint_preview(
                        &mut self.buf,
                        &self.path,
                        &ctm,
                        &self.viewport,
                        cmyk,
                        self.gs.fill_alpha as f32,
                        self.gs.overprint_mode,
                        rule,
                    );
                } else {
                    let fill = self.fill_pixel_color();
                    if !paint_cached_path_fill(
                        &mut self.path_fill_mask_cache,
                        &mut self.buf,
                        &self.viewport,
                        &self.path,
                        &ctm,
                        rule,
                        fill,
                    ) {
                        let _ = PathPainter::fill_fast_cancellable(
                            &mut self.buf,
                            &self.path,
                            &ctm,
                            &self.viewport,
                            fill,
                            rule,
                            &self.cancel,
                        );
                    }
                }
            } else {
                let fill = self.fill_pixel_color();
                if !paint_cached_path_fill(
                    &mut self.path_fill_mask_cache,
                    &mut self.buf,
                    &self.viewport,
                    &self.path,
                    &ctm,
                    rule,
                    fill,
                ) {
                    let _ = PathPainter::fill_fast_cancellable(
                        &mut self.buf,
                        &self.path,
                        &ctm,
                        &self.viewport,
                        fill,
                        rule,
                        &self.cancel,
                    );
                }
            }
        }
        self.record_plate_contribution(&stroke_color_state, self.gs.stroke_alpha as f32, "stroke");
        if self.is_pattern_stroke() {
            if let Some(pattern_name) = self.gs.stroke_pattern_name.clone() {
                self.paint_pattern_stroke(&pattern_name);
            } else {
                log::debug!("PageRenderer: Pattern stroke color space without a pattern name");
            }
            self.path.clear();
            return;
        }
        let stroke = self.stroke_pixel_color();
        let width = self.gs.line_width;
        let dash = self.dash_state();
        if self.gs.stroke_overprint {
            if let Some(cmyk) = device_cmyk_components(&self.gs.stroke_color) {
                PathPainter::stroke_device_cmyk_overprint_preview(
                    &mut self.buf,
                    &self.path,
                    &ctm,
                    &self.viewport,
                    cmyk,
                    self.gs.stroke_alpha as f32,
                    self.gs.overprint_mode,
                    width,
                    &dash,
                    &self.gs.line_cap,
                    &self.gs.line_join,
                    self.gs.miter_limit,
                );
                self.path.clear();
                return;
            }
        }
        if !paint_cached_path_stroke(
            &mut self.path_stroke_mask_cache,
            &mut self.buf,
            &self.viewport,
            &self.path,
            &ctm,
            stroke,
            width,
            &dash,
            &self.gs.line_cap,
            &self.gs.line_join,
            self.gs.miter_limit,
        ) {
            PathPainter::stroke_with_style_fast_cancellable(
                &mut self.buf,
                &self.path,
                &ctm,
                &self.viewport,
                stroke,
                width,
                &dash,
                &self.gs.line_cap,
                &self.gs.line_join,
                self.gs.miter_limit,
                &self.cancel,
            );
        }
        self.path.clear();
    }

    /// True when the current fill color space is the Pattern space, either
    /// directly (`/Pattern cs`) or via a named resource that resolves to a
    /// `[/Pattern ...]` array (`/Cs cs` where `/Cs` is defined as a Pattern
    /// color space in the page resources).
    fn is_pattern_fill(&self) -> bool {
        match &self.gs.fill_color_space {
            ColorSpace::Named(name) if name == "Pattern" => true,
            ColorSpace::Named(name) => self.named_space_is_pattern(name),
            _ => false,
        }
    }

    fn is_pattern_stroke(&self) -> bool {
        match &self.gs.stroke_color_space {
            ColorSpace::Named(name) if name == "Pattern" => true,
            ColorSpace::Named(name) => self.named_space_is_pattern(name),
            _ => false,
        }
    }

    /// Check whether a named color-space resource resolves to a Pattern space.
    fn named_space_is_pattern(&self, name: &str) -> bool {
        let Some(obj) = self.resources.color_spaces.get(name) else {
            return false;
        };
        match obj {
            PdfObject::Name(n) => n == "Pattern",
            PdfObject::Array(arr) => arr.first().and_then(PdfObject::as_name) == Some("Pattern"),
            _ => false,
        }
    }

    fn apply_ext_g_state(&mut self, op: &ContentOperation) {
        let Some(name) = op.name(0) else {
            return;
        };
        if let Some(dict) = self.resources.ext_g_states.get(name).cloned() {
            self.gs.apply_ext_g_state(&dict);
            self.sync_blend_mode();
            self.apply_ext_g_state_smask(&dict);
        } else {
            log::warn!("PageRenderer: ExtGState '{}' not found", name);
        }
    }

    fn apply_ext_g_state_smask(&mut self, dict: &PdfDictionary) {
        let Some(smask_val) = dict.get("SMask") else {
            return;
        };
        match smask_val {
            PdfObject::Name(name) if name == "None" => self.buf.clear_smask(),
            PdfObject::Dictionary(smask_dict) => {
                let seed = smask_inline_cache_seed(smask_dict);
                self.apply_smask(smask_dict.clone(), seed);
            }
            PdfObject::Reference { number, generation } => {
                let reader = self.engine.document().reader();
                match reader.get_and_resolve(*number, *generation) {
                    Ok(PdfObject::Dictionary(smask_dict)) => {
                        let seed = format!("smask:{number}:{generation}");
                        self.apply_smask(smask_dict, Some(seed));
                    }
                    Ok(other) => log::debug!(
                        "PageRenderer: SMask reference resolved to {}, expected Dictionary",
                        other.variant_name()
                    ),
                    Err(err) => log::debug!("PageRenderer: failed to resolve SMask: {}", err),
                }
            }
            _ => log::debug!("PageRenderer: unsupported SMask value"),
        }
    }

    fn apply_smask(&mut self, smask_dict: PdfDictionary, cache_seed: Option<String>) {
        // Subtype: /Luminosity (default) converts the rendered mask group's RGB
        // to a luminance value; /Alpha uses the group's own alpha channel.
        let subtype = smask_dict.get_name("S").unwrap_or("Luminosity");
        let is_alpha = subtype == "Alpha";
        if subtype != "Luminosity" && subtype != "Alpha" {
            log::debug!(
                "PageRenderer: SMask /S '{}' is not supported; using luminosity",
                subtype
            );
        }

        let cache_key = cache_seed
            .as_deref()
            .map(|seed| self.smask_group_cache_key(seed, subtype));
        if let Some(key) = cache_key.as_deref() {
            if let Some(mask) = self.smask_group_cache.get(key) {
                touch_smask_group_cache_key(&mut self.smask_group_cache_order, key);
                self.smask_group_cache_stats.hits =
                    self.smask_group_cache_stats.hits.saturating_add(1);
                self.buf.set_smask(mask.as_ref().clone());
                return;
            }
            self.smask_group_cache_stats.misses =
                self.smask_group_cache_stats.misses.saturating_add(1);
        }

        let reader = self.engine.document().reader();
        let Some(g_obj) = smask_dict.get("G").cloned() else {
            log::debug!("PageRenderer: SMask is missing /G");
            return;
        };
        let (g_dict, g_raw) = match g_obj {
            PdfObject::Reference { number, generation } => {
                match reader.get_object(number, generation) {
                    Ok(PdfObject::Stream { dict, raw }) => (dict, raw),
                    Ok(other) => {
                        log::debug!(
                            "PageRenderer: SMask /G resolved to {}, expected Stream",
                            other.variant_name()
                        );
                        return;
                    }
                    Err(err) => {
                        log::debug!("PageRenderer: failed to resolve SMask /G: {}", err);
                        return;
                    }
                }
            }
            PdfObject::Stream { dict, raw } => (dict, raw),
            _ => {
                log::debug!("PageRenderer: SMask /G is not a Form stream");
                return;
            }
        };

        if g_dict.get_name("Subtype") != Some("Form") {
            log::debug!("PageRenderer: SMask /G is not /Subtype /Form");
            return;
        }

        let stream_obj = PdfObject::Stream {
            dict: g_dict.clone(),
            raw: g_raw,
        };
        let content_bytes = match self.scheduled_decode_stream(
            &stream_obj,
            reader,
            "renderer soft mask group stream decode",
        ) {
            Ok(bytes) => bytes,
            Err(err) => {
                log::warn!("PageRenderer: SMask /G stream decode failed: {}", err);
                return;
            }
        };

        let g_resources = if let Some(res_obj) = g_dict.get("Resources") {
            let form_res = crate::engine::parse_resources_from_obj(res_obj, reader);
            merge_resources(form_res, &self.resources)
        } else {
            self.resources.clone()
        };

        let form_matrix = extract_form_matrix(&g_dict);
        let current_ctm = Transform2D::from(self.gs.ctm);
        let form_t = Transform2D::from(form_matrix);
        let mut mask_gs = self.gs.clone();
        mask_gs.ctm = form_t.concat(&current_ctm).to_array();
        // The mask group renders from a clean compositing state.
        mask_gs.fill_alpha = 1.0;
        mask_gs.stroke_alpha = 1.0;
        mask_gs.blend_mode = BlendMode::Normal;
        let mask_base_ctm = Transform2D::from(mask_gs.ctm);
        let mask_window = match transparency_group_pixel_window(
            extract_bbox(&g_dict),
            &mask_base_ctm,
            &self.viewport,
            self.buf.clip_mask(),
        ) {
            Some(window) => window,
            None => return,
        };
        let mask_viewport = self.viewport.pixel_window(
            mask_window.x,
            mask_window.y,
            mask_window.width,
            mask_window.height,
        );

        // Backdrop initialization per subtype:
        //  - Luminosity: opaque black (mask 0 = fully masked) so areas the mask
        //    group does not paint stay masked out. /BC overrides the backdrop
        //    color (still opaque). Black-backdrop is the spec default.
        //  - Alpha: fully transparent by default, so the alpha channel reflects
        //    only what the mask group actually paints. Some producer fixtures
        //    rely on /BC making the unpainted mask backdrop opaque, but when a
        //    transfer function already maps input 0 to a visible alpha, seeding
        //    an opaque backdrop double-applies that visibility. Match Poppler's
        //    observed convention by using the opaque /BC backdrop only when
        //    /TR(0) is effectively transparent (or /TR is absent).
        let render_mode = self.buf.render_mode();
        let _surface_token = match self.reserve_offscreen_surface(
            mask_window.width,
            mask_window.height,
            "renderer soft mask offscreen surface",
        ) {
            Ok(token) => token,
            Err(err) => {
                log::warn!(
                    "PageRenderer: SMask /G offscreen surface allocation denied: {}",
                    err
                );
                return;
            }
        };
        let mut mask_buf = if is_alpha {
            if alpha_smask_uses_opaque_bc_backdrop(&smask_dict, reader) {
                self.take_filled_offscreen_buffer(
                    mask_window.width,
                    mask_window.height,
                    [0, 0, 0, 255],
                    render_mode,
                )
            } else {
                self.take_transparent_offscreen_buffer(
                    mask_window.width,
                    mask_window.height,
                    render_mode,
                )
            }
        } else {
            let bc = smask_backdrop_color(&smask_dict, &g_dict);
            self.take_filled_offscreen_buffer(
                mask_window.width,
                mask_window.height,
                bc,
                render_mode,
            )
        };
        mask_buf.blend_mode = BlendMode::Normal;

        let ops = match crate::content::ContentParser::parse(&content_bytes) {
            Ok(ops) => ops,
            Err(err) => {
                log::warn!("PageRenderer: SMask /G content parse failed: {}", err);
                return;
            }
        };
        let child_glyph_cache =
            std::mem::replace(&mut self.glyph_cache, GlyphCache::with_default_capacity());
        let child_glyph_mask_cache = std::mem::take(&mut self.glyph_mask_cache);
        let child_type3_mask_cache = std::mem::take(&mut self.type3_mask_cache);
        let child_type3_rendered_cache = std::mem::take(&mut self.type3_rendered_cache);
        let child_path_fill_mask_cache = std::mem::take(&mut self.path_fill_mask_cache);
        let child_path_stroke_mask_cache = std::mem::take(&mut self.path_stroke_mask_cache);
        let child_offscreen_buffer_pool = std::mem::take(&mut self.offscreen_buffer_pool);

        let mut mask_state = RenderState {
            engine: self.engine,
            page_number: self.page_number,
            buf: mask_buf,
            viewport: mask_viewport.clone(),
            resources: g_resources,
            gs: mask_gs,
            clip_stack: Vec::new(),
            smask_stack: Vec::new(),
            path: Path::new(),
            pending_clip: None,
            pending_text_clip: None,
            glyph_cache: child_glyph_cache,
            glyph_mask_cache: child_glyph_mask_cache,
            type3_mask_cache: child_type3_mask_cache,
            type3_rendered_cache: child_type3_rendered_cache,
            path_fill_mask_cache: child_path_fill_mask_cache,
            path_stroke_mask_cache: child_path_stroke_mask_cache,
            font_bytes_cache: self.font_bytes_cache.clone(),
            font_bytes_cache_stats: RenderArtifactCacheStats::default(),
            font_resolver_cache: self.font_resolver_cache.clone(),
            font_resolver_cache_stats: RenderArtifactCacheStats::default(),
            font_resource_key_cache: self.font_resource_key_cache.clone(),
            type3_geometry_cache: self.type3_geometry_cache.clone(),
            type3_charproc_cache: self.type3_charproc_cache.clone(),
            image_xobject_cache: self.image_xobject_cache.clone(),
            image_xobject_cache_order: self.image_xobject_cache_order.clone(),
            image_xobject_cache_bytes: self.image_xobject_cache_bytes,
            image_xobject_cache_stats: RenderArtifactCacheStats::default(),
            scaled_image_cache: self.scaled_image_cache.clone(),
            scaled_image_cache_order: self.scaled_image_cache_order.clone(),
            scaled_image_cache_bytes: self.scaled_image_cache_bytes,
            scaled_image_cache_stats: RenderArtifactCacheStats::default(),
            smask_group_cache: self.smask_group_cache.clone(),
            smask_group_cache_order: self.smask_group_cache_order.clone(),
            smask_group_cache_bytes: self.smask_group_cache_bytes,
            smask_group_cache_stats: RenderArtifactCacheStats::default(),
            shading_mesh_cache: self.shading_mesh_cache.clone(),
            shading_mesh_cache_order: self.shading_mesh_cache_order.clone(),
            shading_mesh_cache_bytes: self.shading_mesh_cache_bytes,
            shading_mesh_cache_stats: RenderArtifactCacheStats::default(),
            form_xobject_program_cache: self.form_xobject_program_cache.clone(),
            form_xobject_program_cache_stats: RenderArtifactCacheStats::default(),
            tiling_pattern_program_cache: self.tiling_pattern_program_cache.clone(),
            tiling_pattern_program_cache_stats: RenderArtifactCacheStats::default(),
            offscreen_buffer_pool: child_offscreen_buffer_pool,
            clip_dag: ClipDag::new(),
            pattern_stack: self.pattern_stack.clone(),
            form_depth: self.form_depth + 1,
            form_object_stack: self.form_object_stack.clone(),
            pending_inline: None,
            base_ctm: mask_base_ctm,
            cancel: self.cancel.clone(),
            fatal_render_error: None,
            decode_scheduler: self.decode_scheduler.clone(),
            optional_content: self.optional_content.clone(),
            oc_visibility_stack: self.oc_visibility_stack.clone(),
            oc_current_visible: self.oc_current_visible,
            separation_framebuffer: SeparationFramebuffer::for_page(
                self.page_number,
                mask_viewport.width_px,
                mask_viewport.height_px,
            ),
        };

        if let Some(bbox) = extract_bbox(&g_dict) {
            mask_state.apply_form_bbox_clip(bbox);
        }
        mask_state.dispatch_all(&ops);
        if let Some(reason) = mask_state.fatal_render_error.take() {
            self.record_fatal_render_error(reason);
        }
        self.separation_framebuffer
            .absorb(mask_state.separation_framebuffer.clone());
        self.absorb_child_render_caches(&mut mask_state);
        let mask_buf = mask_state.into_buffer();

        let window_mask = if is_alpha {
            AlphaMask::from_alpha_channel(&mask_buf)
        } else {
            AlphaMask::from_luminosity(&mask_buf)
        };
        let default_alpha = smask_default_alpha(&smask_dict, &g_dict, is_alpha, reader);
        let mut mask = window_mask.with_origin_and_outside_alpha(
            mask_window.x as i32,
            mask_window.y as i32,
            default_alpha,
        );

        // Apply the /TR transfer function if present and not the /Identity
        // no-op. All function types (0/2/3/4) are supported via the shared
        // function evaluator.
        if let Some(lut) = self.build_transfer_lut(&smask_dict) {
            mask.apply_transfer_lut(&lut);
        }

        if let Some(key) = cache_key {
            insert_smask_group_cache_entry(
                &mut self.smask_group_cache,
                &mut self.smask_group_cache_order,
                &mut self.smask_group_cache_bytes,
                &mut self.smask_group_cache_stats,
                key,
                Arc::new(mask.clone()),
            );
        }
        self.buf.set_smask(mask);
        self.recycle_offscreen_buffer(mask_buf);
    }

    fn smask_group_cache_key(&self, seed: &str, subtype: &str) -> String {
        let clip_bounds = self
            .buf
            .clip_mask()
            .and_then(ClipMask::visible_bounds)
            .map(|(x0, y0, x1, y1)| format!("{x0}:{y0}:{x1}:{y1}"))
            .unwrap_or_else(|| "clip:none".to_string());
        format!(
            "{seed}:page:{}:w:{}:h:{}:mode:{:?}:subtype:{}:ctm:{:?}:clip:{}",
            self.page_number,
            self.buf.width,
            self.buf.height,
            self.buf.render_mode(),
            subtype,
            self.gs.ctm,
            clip_bounds
        )
    }

    /// Build a 256-entry transfer LUT from an SMask `/TR` function, or `None`
    /// when /TR is absent, /Identity, or an unsupported function type. Supports
    /// Function Types 0, 2, 3, and 4 via the shared evaluator.
    fn build_transfer_lut(&self, smask_dict: &PdfDictionary) -> Option<[u8; 256]> {
        let tr = smask_dict.get("TR")?;
        // /Identity (a name) is the explicit no-op default.
        if let PdfObject::Name(name) = tr {
            if name == "Identity" {
                return None;
            }
        }
        let reader = self.engine.document().reader();
        // Probe the function once to confirm it evaluates; if not, skip.
        let probe = crate::render::shading::eval_function(tr, 0.5, reader);
        if probe.is_empty() {
            log::debug!("PageRenderer: SMask /TR is an unsupported function type; using identity");
            return None;
        }
        let mut lut = [0u8; 256];
        for (i, slot) in lut.iter_mut().enumerate() {
            let t = i as f64 / 255.0;
            let out = crate::render::shading::eval_function(tr, t, reader);
            let v = out.first().copied().unwrap_or(t).clamp(0.0, 1.0);
            *slot = (v * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        Some(lut)
    }

    /// Decode and paint an inline image (BI/ID/EI). `params` are the `ID`
    /// operands (already normalized to full key names by the content parser);
    /// `data` is the raw bytes between `ID` and `EI`.
    fn paint_inline_image(&mut self, params: &[Operand], data: &[u8]) {
        let dict = inline_params_to_map(params);

        let width = dict_int(&dict, "Width").unwrap_or(0).max(0) as u32;
        let height = dict_int(&dict, "Height").unwrap_or(0).max(0) as u32;
        if width == 0 || height == 0 {
            return;
        }
        let is_mask = dict_bool(&dict, "ImageMask").unwrap_or(false);
        let bpc = if is_mask {
            1
        } else {
            dict_int(&dict, "BitsPerComponent")
                .unwrap_or(8)
                .clamp(1, 16) as u8
        };
        let color_space = dict_name(&dict, "ColorSpace").unwrap_or("DeviceGray");
        let filters: Vec<&str> = dict_filter_list(&dict);
        let decode_params = match inline_decode_params(&dict, filters.len()) {
            Ok(params) => params,
            Err(err) => {
                log::warn!("PageRenderer: inline DecodeParms rejected: {err}");
                return;
            }
        };
        let interpolate = dict_bool(&dict, "Interpolate").unwrap_or(false);
        if is_mask {
            let fill_color_state = self.gs.fill_color.clone();
            self.record_plate_contribution(&fill_color_state, 1.0, "image_inline_stencil_mask");
        }

        // Inline image masks are stencil masks: paint the current fill color
        // through the 1-bit mask. We currently decode them as a grayscale image
        // and paint that; full stencil-color application is a follow-up.
        let raw = match self.scheduled_decode_inline_image(
            data,
            width,
            height,
            bpc,
            color_space,
            &filters,
            &decode_params,
        ) {
            Ok(raw) => raw,
            Err(err) => {
                log::warn!("PageRenderer: inline image decode failed: {}", err);
                return;
            }
        };
        let raw = if is_mask {
            let color = self.resolve_paint_color(&self.gs.fill_color, self.gs.fill_alpha as f32);
            image_mask_to_stencil_rgba(raw, color, inline_image_mask_paints_ones(&dict))
        } else {
            raw
        };

        let ctm = self.ctm();
        let paint_alpha = if is_mask {
            1.0
        } else {
            self.gs.fill_alpha as f32
        };
        ImagePainter::paint_image_with_options_and_alpha(
            &mut self.buf,
            &raw,
            &ctm,
            &self.viewport,
            interpolate,
            paint_alpha,
        );
    }

    fn handle_do(&mut self, name: &str) {
        let Some(&(obj_num, gen_num)) = self.resources.xobjects.get(name) else {
            log::warn!("PageRenderer: XObject '{}' not found in resources", name);
            return;
        };

        let reader = self.engine.document().reader();
        let obj = match reader.get_object(obj_num, gen_num) {
            Ok(obj) => obj,
            Err(err) => {
                log::warn!(
                    "PageRenderer: failed to resolve XObject '{}': {}",
                    name,
                    err
                );
                return;
            }
        };

        let PdfObject::Stream { dict, .. } = &obj else {
            log::warn!("PageRenderer: XObject '{}' is not a stream", name);
            return;
        };
        if !self
            .optional_content
            .is_object_visible(dict.get("OC"), self.engine.document().reader())
        {
            return;
        }

        match dict.get_name("Subtype") {
            Some("Image") => self.handle_do_image(name, obj_num, gen_num, dict),
            Some("Form") => self.handle_do_form(name, obj_num, gen_num),
            Some(other) => log::debug!("PageRenderer: XObject subtype '{}' not handled", other),
            None => log::warn!("PageRenderer: XObject '{}' has no /Subtype", name),
        }
    }

    fn handle_do_image(&mut self, name: &str, obj_num: u32, gen_num: u16, dict: &PdfDictionary) {
        let image_is_mask = dict
            .get_bool("ImageMask")
            .or_else(|| dict.get_bool("IM"))
            .unwrap_or(false);
        let color_space_override = if image_is_mask {
            None
        } else {
            self.resolved_image_color_space_override(dict)
        };
        let image_ref = ImageReference {
            page_number: self.page_number,
            xobject_name: name.to_string(),
            object_number: obj_num,
            generation_number: gen_num,
            width: positive_u32(
                dict.get_integer("Width").or_else(|| dict.get_integer("W")),
                1,
            ),
            height: positive_u32(
                dict.get_integer("Height").or_else(|| dict.get_integer("H")),
                1,
            ),
            bits_per_component: dict
                .get_integer("BitsPerComponent")
                .or_else(|| dict.get_integer("BPC"))
                .unwrap_or(8)
                .clamp(0, 16) as u8,
            color_space: color_space_override
                .as_ref()
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| extract_color_space_name(dict)),
            filter: extract_filter_names(dict),
            is_inline: false,
            is_mask: image_is_mask,
            is_smask: false,
            inline_data: None,
        };
        let image_cache_key = color_space_override
            .as_ref()
            .map(|(color_space_name, color_space_obj)| {
                image_xobject_cache_key_with_color_space(
                    &image_ref,
                    color_space_name,
                    color_space_obj,
                )
            })
            .unwrap_or_else(|| image_xobject_cache_key(&image_ref));
        self.record_image_plate_sample(
            dict,
            &image_ref,
            format!("image XObject /{} {} {} R", name, obj_num, gen_num),
            "image_xobject",
        );

        match self.scheduled_decode_image_with_color_space(
            &image_ref,
            color_space_override.as_ref(),
            "renderer image XObject decode",
        ) {
            Ok(raw) => {
                let owned_raw;
                let paint_raw = if image_ref.is_mask {
                    let color =
                        self.resolve_paint_color(&self.gs.fill_color, self.gs.fill_alpha as f32);
                    owned_raw = Some(image_mask_to_stencil_rgba(
                        (*raw).clone(),
                        color,
                        image_mask_paints_ones(dict),
                    ));
                    owned_raw.as_ref().expect("image mask owned raw")
                } else if dict.contains_key("SMask") {
                    match self.scheduled_load_smask(&image_ref, (*raw).clone()) {
                        Ok(Some(masked)) => {
                            owned_raw = Some(masked);
                            owned_raw.as_ref().expect("soft mask owned raw")
                        }
                        Ok(None) => raw.as_ref(),
                        Err(err) => {
                            log::warn!("PageRenderer: image '{}' SMask failed: {}", name, err);
                            raw.as_ref()
                        }
                    }
                } else if dict.contains_key("Mask") {
                    match self.scheduled_load_explicit_image_mask(&image_ref, dict, (*raw).clone())
                    {
                        Ok(Some(masked)) => {
                            owned_raw = Some(masked);
                            owned_raw.as_ref().expect("explicit mask owned raw")
                        }
                        Ok(None) => raw.as_ref(),
                        Err(err) => {
                            log::warn!(
                                "PageRenderer: image '{}' explicit Mask failed: {}",
                                name,
                                err
                            );
                            raw.as_ref()
                        }
                    }
                } else {
                    raw.as_ref()
                };
                let ctm = self.ctm();
                let smooth_jpx = image_ref.filter.iter().any(|filter| filter == "JPXDecode");
                let paint_alpha = if image_ref.is_mask {
                    1.0
                } else {
                    self.gs.fill_alpha as f32
                };
                let interpolate = image_interpolate(dict);
                if !image_ref.is_mask
                    && !dict.contains_key("SMask")
                    && !dict.contains_key("Mask")
                    && !interpolate
                    && !smooth_jpx
                {
                    if let Some(target) =
                        ImagePainter::axis_aligned_integer_target(&ctm, &self.viewport)
                    {
                        if let Some(scaled) = self.cached_axis_aligned_scaled_image(
                            &image_cache_key,
                            paint_raw,
                            target.width,
                            target.height,
                        ) {
                            if ImagePainter::paint_scaled_rgb_at_device_target(
                                &mut self.buf,
                                scaled.as_ref(),
                                target,
                                paint_alpha,
                            ) {
                                return;
                            }
                        }
                    }
                }
                if interpolate {
                    ImagePainter::paint_image_with_options_and_alpha(
                        &mut self.buf,
                        paint_raw,
                        &ctm,
                        &self.viewport,
                        true,
                        paint_alpha,
                    );
                } else if smooth_jpx {
                    ImagePainter::paint_image_with_jpx_compat_and_alpha(
                        &mut self.buf,
                        paint_raw,
                        &ctm,
                        &self.viewport,
                        paint_alpha,
                    );
                } else {
                    ImagePainter::paint_image_with_alpha(
                        &mut self.buf,
                        paint_raw,
                        &ctm,
                        &self.viewport,
                        paint_alpha,
                    );
                }
            }
            Err(err) => log::warn!("PageRenderer: image '{}' decode failed: {}", name, err),
        }
    }

    fn resolved_image_color_space_override(
        &self,
        dict: &PdfDictionary,
    ) -> Option<(String, PdfObject)> {
        let PdfObject::Name(resource_name) = dict.get("ColorSpace").or_else(|| dict.get("CS"))?
        else {
            return None;
        };
        let resource_obj = self.resources.color_spaces.get(resource_name)?.clone();
        let family = image_color_space_family_name(
            &resource_obj,
            &self.resources,
            self.engine.document().reader(),
            0,
        )
        .unwrap_or_else(|| canonical_image_color_space_name(resource_name));
        Some((family, resource_obj))
    }

    fn handle_do_form(&mut self, name: &str, obj_num: u32, gen_num: u16) {
        // Depth guard: prevent runaway recursion from malformed or cyclic PDFs.
        if self.form_depth >= 8 {
            log::warn!(
                "PageRenderer: Form XObject nesting depth limit (8) exceeded at '{}' (obj {})",
                name,
                obj_num
            );
            return;
        }
        let form_key = (obj_num, gen_num);
        if self.form_object_stack.contains(&form_key) {
            log::warn!(
                "PageRenderer: Form XObject cycle detected at '{}' (obj {} {})",
                name,
                obj_num,
                gen_num
            );
            return;
        }

        let Some(program) = self.cached_form_xobject_program(name, obj_num, gen_num) else {
            return;
        };
        let form_matrix = program.form_matrix;
        let bbox = program.bbox;
        let current_ctm = Transform2D::from(self.gs.ctm);
        let form_t = Transform2D::from(form_matrix);
        let form_ctm = form_t.concat(&current_ctm);
        if bbox.is_some_and(|bb| !form_bbox_intersects_viewport(bb, &form_ctm, &self.viewport)) {
            return;
        }

        // â”€â”€ Step 1: Fetch the Form stream object â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        // get_object already decrypts; decode_stream then decompresses.
        if program.is_transparency_group {
            self.form_object_stack.push(form_key);
            self.handle_do_form_group(
                name,
                &program.dict,
                program.ops.as_slice(),
                program.form_matrix,
                program.bbox,
                program.resources.as_ref(),
            );
            self.form_object_stack.pop();
            return;
        }

        // A /Group that is NOT /S /Transparency (e.g. some other group subtype)
        // falls through to direct rendering, which is correct for the common
        // non-transparent case. /S /Transparency groups are handled above.
        if program.dict.get("Group").is_some() {
            log::debug!(
                "PageRenderer: Form XObject '{}' has a non-transparency /Group â€” rendering directly",
                name
            );
        }

        // â”€â”€ Step 2: Extract Matrix and BBox â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        // â”€â”€ Step 3: Save graphics state, clip, and resources â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        let saved_gs = self.gs.clone();
        let node = self.clip_dag.intern_option(self.buf.clip_mask());
        self.clip_stack.push(node);
        self.smask_stack.push(self.buf.smask_mask().cloned());
        let saved_base_ctm = self.base_ctm;
        self.form_depth += 1;
        self.form_object_stack.push(form_key);

        // â”€â”€ Step 4: Apply the Form matrix to the CTM â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        // The Form /Matrix maps Form space â†’ the user space in effect at the Do.
        // concat(self, other) applies `self` first then `other`, so to apply the
        // form matrix before the current CTM we compute form_matrix.concat(ctm).
        self.gs.ctm = form_ctm.to_array();
        // Patterns referenced inside this Form are relative to the Form's own
        // default coordinate system.
        self.base_ctm = Transform2D::from(self.gs.ctm);

        // â”€â”€ Step 5: Clip to the BBox (intersected with any existing clip) â”€â”€â”€â”€
        if let Some(bb) = bbox {
            self.apply_form_bbox_clip(bb);
        }

        // â”€â”€ Step 6: Merge the Form's own resources over the page resources â”€â”€â”€
        let resource_overlay = program
            .resources
            .as_ref()
            .map(|form_res| overlay_page_resources(&mut self.resources, form_res));
        // No /Resources: keep using the inherited (page) resources already set.

        // â”€â”€ Step 7: Decode and parse the content stream â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        // Render the cached Form XObject program; stream decode and content parsing happen once per object revision.
        self.dispatch_all(program.ops.as_slice());

        // â”€â”€ Step 9: Restore the saved state â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
        if let Some(restore) = resource_overlay {
            restore.restore(&mut self.resources);
        }
        self.form_object_stack.pop();
        self.cleanup_after_form(saved_gs, saved_base_ctm);
    }

    fn handle_do_form_group(
        &mut self,
        name: &str,
        form_dict: &PdfDictionary,
        ops: &[ContentOperation],
        form_matrix: crate::content::Matrix,
        bbox: Option<[f64; 4]>,
        form_resources: Option<&PageResources>,
    ) {
        // Group flags: /I (isolated) and /K (knockout), both default false.
        let (isolated, knockout) = match transparency_group_dict(form_dict) {
            Some(group) => (group_is_isolated(group), group_is_knockout(group)),
            None => (false, false),
        };
        if knockout {
            // Knockout (/K true): interior elements should knock out the group
            // backdrop rather than accumulate. We track the flag on the group
            // RenderState and apply knockout compositing at the group's
            // initial backdrop, so overlapping interior elements replace
            // earlier group elements at covered pixels.
            log::debug!("PageRenderer: knockout transparency group '{}'", name);
        }

        // An isolated group starts from a fully transparent backdrop. A
        // non-isolated group starts from a copy of the current backdrop (the
        // page/parent buffer so far), so blend modes inside the group can
        // interact with what is already painted. We remove that backdrop
        // contribution again before compositing the group result back, so the
        // backdrop is not counted twice (PDF 32000-1 Â§11.4.8).
        let render_mode = self.buf.render_mode();
        let current_ctm = Transform2D::from(self.gs.ctm);
        let form_t = Transform2D::from(form_matrix);
        let group_ctm = form_t.concat(&current_ctm);
        let Some(group_window) =
            transparency_group_pixel_window(bbox, &group_ctm, &self.viewport, self.buf.clip_mask())
        else {
            return;
        };
        let _surface_token = match self.reserve_offscreen_surface(
            group_window.width,
            group_window.height,
            "renderer transparency group offscreen surface",
        ) {
            Ok(token) => token,
            Err(err) => {
                log::warn!(
                    "PageRenderer: transparency Form XObject '{}' offscreen surface allocation denied: {}",
                    name,
                    err
                );
                return;
            }
        };
        let group_viewport = self.viewport.pixel_window(
            group_window.x,
            group_window.y,
            group_window.width,
            group_window.height,
        );
        let backdrop_window = (!isolated)
            .then(|| {
                self.buf.copy_rect_to_new_buffer(
                    group_window.x,
                    group_window.y,
                    group_window.width,
                    group_window.height,
                )
            })
            .flatten();
        let mut group_buf = if isolated {
            self.take_transparent_offscreen_buffer(
                group_window.width,
                group_window.height,
                render_mode,
            )
        } else {
            let mut copy = backdrop_window.clone().unwrap_or_else(|| {
                PixelBuffer::new_transparent_with_mode(
                    group_window.width,
                    group_window.height,
                    render_mode,
                )
            });
            copy.clear_clip();
            copy.clear_smask();
            copy
        };
        group_buf.blend_mode = BlendMode::Normal;

        let mut group_gs = self.gs.clone();
        group_gs.ctm = group_ctm.to_array();
        // Inside the group, painting starts from a clean compositing state:
        // the group's own alpha/blend/soft-mask are applied when the *result*
        // is composited back, not to each interior element.
        group_gs.fill_alpha = 1.0;
        group_gs.stroke_alpha = 1.0;
        group_gs.blend_mode = BlendMode::Normal;
        let group_base_ctm = Transform2D::from(group_gs.ctm);

        let group_resources = if let Some(form_res) = form_resources {
            merge_resources_ref(form_res, &self.resources)
        } else {
            self.resources.clone()
        };

        let child_glyph_cache =
            std::mem::replace(&mut self.glyph_cache, GlyphCache::with_default_capacity());
        let child_glyph_mask_cache = std::mem::take(&mut self.glyph_mask_cache);
        let child_type3_mask_cache = std::mem::take(&mut self.type3_mask_cache);
        let child_type3_rendered_cache = std::mem::take(&mut self.type3_rendered_cache);
        let child_path_fill_mask_cache = std::mem::take(&mut self.path_fill_mask_cache);
        let child_path_stroke_mask_cache = std::mem::take(&mut self.path_stroke_mask_cache);
        let child_offscreen_buffer_pool = std::mem::take(&mut self.offscreen_buffer_pool);

        let mut group_state = RenderState {
            engine: self.engine,
            page_number: self.page_number,
            buf: group_buf,
            viewport: group_viewport.clone(),
            resources: group_resources,
            gs: group_gs,
            clip_stack: Vec::new(),
            smask_stack: Vec::new(),
            path: Path::new(),
            pending_clip: None,
            pending_text_clip: None,
            glyph_cache: child_glyph_cache,
            glyph_mask_cache: child_glyph_mask_cache,
            type3_mask_cache: child_type3_mask_cache,
            type3_rendered_cache: child_type3_rendered_cache,
            path_fill_mask_cache: child_path_fill_mask_cache,
            path_stroke_mask_cache: child_path_stroke_mask_cache,
            font_bytes_cache: self.font_bytes_cache.clone(),
            font_bytes_cache_stats: RenderArtifactCacheStats::default(),
            font_resolver_cache: self.font_resolver_cache.clone(),
            font_resolver_cache_stats: RenderArtifactCacheStats::default(),
            font_resource_key_cache: self.font_resource_key_cache.clone(),
            type3_geometry_cache: self.type3_geometry_cache.clone(),
            type3_charproc_cache: self.type3_charproc_cache.clone(),
            image_xobject_cache: self.image_xobject_cache.clone(),
            image_xobject_cache_order: self.image_xobject_cache_order.clone(),
            image_xobject_cache_bytes: self.image_xobject_cache_bytes,
            image_xobject_cache_stats: RenderArtifactCacheStats::default(),
            scaled_image_cache: self.scaled_image_cache.clone(),
            scaled_image_cache_order: self.scaled_image_cache_order.clone(),
            scaled_image_cache_bytes: self.scaled_image_cache_bytes,
            scaled_image_cache_stats: RenderArtifactCacheStats::default(),
            smask_group_cache: self.smask_group_cache.clone(),
            smask_group_cache_order: self.smask_group_cache_order.clone(),
            smask_group_cache_bytes: self.smask_group_cache_bytes,
            smask_group_cache_stats: RenderArtifactCacheStats::default(),
            shading_mesh_cache: self.shading_mesh_cache.clone(),
            shading_mesh_cache_order: self.shading_mesh_cache_order.clone(),
            shading_mesh_cache_bytes: self.shading_mesh_cache_bytes,
            shading_mesh_cache_stats: RenderArtifactCacheStats::default(),
            form_xobject_program_cache: self.form_xobject_program_cache.clone(),
            form_xobject_program_cache_stats: RenderArtifactCacheStats::default(),
            tiling_pattern_program_cache: self.tiling_pattern_program_cache.clone(),
            tiling_pattern_program_cache_stats: RenderArtifactCacheStats::default(),
            offscreen_buffer_pool: child_offscreen_buffer_pool,
            clip_dag: ClipDag::new(),
            pattern_stack: self.pattern_stack.clone(),
            form_depth: self.form_depth + 1,
            form_object_stack: self.form_object_stack.clone(),
            pending_inline: None,
            base_ctm: group_base_ctm,
            cancel: self.cancel.clone(),
            fatal_render_error: None,
            decode_scheduler: self.decode_scheduler.clone(),
            optional_content: self.optional_content.clone(),
            oc_visibility_stack: self.oc_visibility_stack.clone(),
            oc_current_visible: self.oc_current_visible,
            separation_framebuffer: SeparationFramebuffer::for_page(
                self.page_number,
                group_viewport.width_px,
                group_viewport.height_px,
            ),
        };

        // Carry the parent clip into the group so content is bounded the same
        // way direct rendering would be, then intersect the Form BBox.
        if let Some(clip) = self.buf.clip_mask().cloned() {
            if let Some(cropped) = clip.copy_rect_to_new_mask(
                group_window.x,
                group_window.y,
                group_window.width,
                group_window.height,
            ) {
                group_state.buf.set_clip(cropped);
            }
        }
        if let Some(bbox) = bbox {
            group_state.apply_form_bbox_clip(bbox);
        }
        if knockout {
            let knockout_backdrop = group_state.buf.clone();
            group_state.buf.set_knockout_backdrop(knockout_backdrop);
        }

        group_state.dispatch_all(ops);
        if let Some(reason) = group_state.fatal_render_error.take() {
            self.record_fatal_render_error(reason);
        }
        self.separation_framebuffer
            .absorb(group_state.separation_framebuffer.clone());
        self.absorb_child_render_caches(&mut group_state);
        let mut group_buf = group_state.into_buffer();
        group_buf.clear_knockout_backdrop();
        group_buf.clear_clip();

        // For a non-isolated group, subtract the backdrop we seeded it with so
        // it is not double-counted when we composite the group back.
        if !isolated {
            if let Some(backdrop) = backdrop_window.as_ref() {
                group_buf.remove_backdrop(backdrop);
            }
        }

        // Composite the finished group as a single unit, using the alpha /
        // blend mode / soft mask active at the point of the `Do` operator.
        let group_alpha = self.gs.fill_alpha as f32;
        let blend_mode = self.gs.blend_mode;
        let soft_mask = self.buf.smask_mask().cloned();
        self.buf.composite_from_at(
            &group_buf,
            group_window.x,
            group_window.y,
            group_alpha,
            blend_mode,
            soft_mask.as_ref(),
        );
        self.recycle_offscreen_buffer(group_buf);
    }

    /// Restore the graphics state and clip mask saved before a Form
    /// XObject was rendered, and decrement the depth counter.
    fn cleanup_after_form(&mut self, saved_gs: GraphicsState, saved_base_ctm: Transform2D) {
        self.form_depth = self.form_depth.saturating_sub(1);
        self.gs = saved_gs;
        self.base_ctm = saved_base_ctm;
        self.sync_blend_mode();
        match self.clip_stack.pop() {
            Some(saved) => {
                let mask = match &saved.state {
                    ClipState::Full => None,
                    _ => Some(saved.materialize(self.buf.width, self.buf.height).clone()),
                };
                self.buf.restore_clip(mask);
            }
            None => log::warn!("PageRenderer: Form cleanup with empty clip stack"),
        }
        match self.smask_stack.pop() {
            Some(saved) => self.buf.restore_smask(saved),
            None => log::warn!("PageRenderer: Form cleanup with empty SMask stack"),
        }
    }

    fn render_page_annotations(&mut self) {
        let reader = self.engine.document().reader();
        let pages = match self.engine.document().get_pages() {
            Ok(pages) => pages,
            Err(err) => {
                log::debug!("PageRenderer: could not load pages for annotations: {err}");
                return;
            }
        };
        let Some(page) = pages.get(self.page_number.saturating_sub(1)) else {
            return;
        };
        let page_dict = match reader.get_and_resolve(page.object_number, page.generation_number) {
            Ok(PdfObject::Dictionary(dict)) => dict,
            Ok(_) => return,
            Err(err) => {
                log::debug!("PageRenderer: could not resolve page annotations: {err}");
                return;
            }
        };
        let Some(annots_obj) = page_dict.get("Annots").cloned() else {
            return;
        };
        let annots = match reader.resolve(annots_obj) {
            Ok(PdfObject::Array(items)) => items,
            _ => return,
        };

        for (index, annot_obj) in annots.into_iter().enumerate() {
            if self.cancel.is_cancelled() {
                return;
            }
            let annot = match reader.resolve(annot_obj) {
                Ok(PdfObject::Dictionary(dict)) => dict,
                _ => continue,
            };
            if annotation_is_hidden_or_no_view(&annot) {
                continue;
            }
            if !self
                .optional_content
                .is_object_visible(annot.get("OC"), reader)
            {
                continue;
            }
            let Some(rect) = extract_rect(&annot) else {
                continue;
            };
            let Some((appearance_dict, appearance_raw)) =
                select_annotation_appearance(&annot, reader).or_else(|| {
                    synthesize_annotation_appearance(&annot, reader, self.engine, rect)
                })
            else {
                continue;
            };
            if appearance_dict.get_name("Subtype") != Some("Form") {
                continue;
            }
            self.render_annotation_appearance(
                &format!("Annot{}", index + 1),
                &appearance_dict,
                appearance_raw,
                rect,
            );
        }
    }

    fn render_annotation_appearance(
        &mut self,
        name: &str,
        form_dict: &PdfDictionary,
        raw_bytes: Vec<u8>,
        rect: [f64; 4],
    ) {
        if self.form_depth >= 8 {
            log::warn!(
                "PageRenderer: annotation appearance nesting depth limit (8) exceeded at '{}'",
                name
            );
            return;
        }

        let Some(bbox) = extract_bbox(form_dict) else {
            log::debug!(
                "PageRenderer: annotation appearance '{}' missing /BBox",
                name
            );
            return;
        };
        let Some(placement) = annotation_appearance_ctm(rect, bbox) else {
            return;
        };

        let saved_gs = self.gs.clone();
        let saved_clip = self.buf.clip_mask().cloned();
        let saved_smask = self.buf.smask_mask().cloned();
        self.buf.clear_clip();
        self.buf.clear_smask();
        self.gs = GraphicsState::default();
        self.gs.ctm = placement.to_array();
        self.sync_blend_mode();

        let stream_obj = PdfObject::Stream {
            dict: form_dict.clone(),
            raw: raw_bytes.clone(),
        };
        let reader = self.engine.document().reader();
        let content_bytes = match self.scheduled_decode_stream(
            &stream_obj,
            reader,
            "renderer annotation appearance stream decode",
        ) {
            Ok(bytes) => bytes,
            Err(err) => {
                log::warn!(
                    "PageRenderer: annotation appearance '{}' stream decode failed: {}",
                    name,
                    err
                );
                self.gs = saved_gs;
                self.buf.restore_clip(saved_clip);
                self.buf.restore_smask(saved_smask);
                self.sync_blend_mode();
                return;
            }
        };

        if is_transparency_group(form_dict) {
            let ops = match crate::content::ContentParser::parse(&content_bytes) {
                Ok(ops) => ops,
                Err(err) => {
                    log::warn!(
                        "PageRenderer: transparency annotation appearance '{}' content parse failed: {}",
                        name,
                        err
                    );
                    self.gs = saved_gs;
                    self.buf.restore_clip(saved_clip);
                    self.buf.restore_smask(saved_smask);
                    self.sync_blend_mode();
                    return;
                }
            };
            let reader = self.engine.document().reader();
            let form_resources = form_dict
                .get("Resources")
                .map(|res_obj| crate::engine::parse_resources_from_obj(res_obj, reader));
            self.handle_do_form_group(
                name,
                form_dict,
                &ops,
                extract_form_matrix(form_dict),
                extract_bbox(form_dict),
                form_resources.as_ref(),
            );
        } else {
            self.render_form_content_stream(name, form_dict, &content_bytes);
        }

        self.gs = saved_gs;
        self.buf.restore_clip(saved_clip);
        self.buf.restore_smask(saved_smask);
        self.sync_blend_mode();
    }

    fn render_form_content_stream(
        &mut self,
        name: &str,
        form_dict: &PdfDictionary,
        content_bytes: &[u8],
    ) {
        let reader = self.engine.document().reader();
        let form_matrix = extract_form_matrix(form_dict);
        let bbox = extract_bbox(form_dict);

        let saved_gs = self.gs.clone();
        let node = self.clip_dag.intern_option(self.buf.clip_mask());
        self.clip_stack.push(node);
        self.smask_stack.push(self.buf.smask_mask().cloned());
        let saved_base_ctm = self.base_ctm;
        self.form_depth += 1;

        let current_ctm = Transform2D::from(self.gs.ctm);
        let form_t = Transform2D::from(form_matrix);
        self.gs.ctm = form_t.concat(&current_ctm).to_array();
        self.base_ctm = Transform2D::from(self.gs.ctm);

        if let Some(bb) = bbox {
            self.apply_form_bbox_clip(bb);
        }

        let resource_overlay = if let Some(res_obj) = form_dict.get("Resources") {
            let form_res = crate::engine::parse_resources_from_obj(res_obj, reader);
            Some(overlay_page_resources(&mut self.resources, &form_res))
        } else {
            None
        };

        let ops = match crate::content::ContentParser::parse(content_bytes) {
            Ok(ops) => ops,
            Err(err) => {
                log::warn!(
                    "PageRenderer: Form XObject '{}' content parse failed: {}",
                    name,
                    err
                );
                if let Some(restore) = resource_overlay {
                    restore.restore(&mut self.resources);
                }
                self.cleanup_after_form(saved_gs, saved_base_ctm);
                return;
            }
        };

        self.dispatch_all(&ops);
        if let Some(restore) = resource_overlay {
            restore.restore(&mut self.resources);
        }
        self.cleanup_after_form(saved_gs, saved_base_ctm);
    }

    /// Clip subsequent painting to the Form's BBox, transformed by the current
    /// CTM. `set_clip` intersects with any existing clip, so page-level clips
    /// are preserved.
    fn apply_form_bbox_clip(&mut self, bbox: [f64; 4]) {
        let x_min = bbox[0].min(bbox[2]);
        let y_min = bbox[1].min(bbox[3]);
        let width = (bbox[2] - bbox[0]).abs();
        let height = (bbox[3] - bbox[1]).abs();
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let mut bbox_path = Path::new();
        bbox_path.rect(x_min, y_min, width, height);
        let ctm = self.ctm();
        if let Some((x, y, w, h)) =
            axis_aligned_bbox_clip_rect(bbox, &ctm, &self.viewport, self.buf.width, self.buf.height)
        {
            let clip = ClipMask::from_visible_rect(self.buf.width, self.buf.height, x, y, w, h);
            self.buf.set_clip(clip);
            return;
        }
        let flat = flatten_path(&bbox_path, &ctm, &self.viewport, 0.5);
        let clip = ClipMask::from_path(&flat, self.buf.width, self.buf.height, FillRule::NonZero);
        self.buf.set_clip(clip);
    }

    /// Handle the `sh` operator: paint the named shading over the current clip
    /// region (the entire page if no clip is set).
    fn handle_sh(&mut self, name: String) {
        let shading_obj = match self.resources.shadings.get(&name) {
            Some(obj) => obj.clone(),
            None => {
                log::warn!("sh: shading '{}' not found in resources", name);
                return;
            }
        };
        let reader = self.engine.document().reader();
        let Some(shading_dict) = resolve_to_dict(&shading_obj, reader) else {
            log::warn!("sh: shading '{}' did not resolve to a dictionary", name);
            return;
        };
        let shading_dict = self.shading_dict_with_resolved_color_space(shading_dict);
        if !self
            .optional_content
            .is_object_visible(shading_dict.get("OC"), reader)
        {
            return;
        }
        self.record_shading_plate_sample(
            &shading_dict,
            format!("page {} shading /{}", self.page_number, name),
            "shading_resource",
        );
        let ctm = self.ctm();
        let mesh_data = self.shading_mesh_data(&shading_obj, &shading_dict, reader);
        ShadingRenderer::paint(
            &shading_dict,
            &ctm,
            &self.viewport,
            &mut self.buf,
            reader,
            mesh_data.as_deref().map(Vec::as_slice),
        );
    }

    /// Paint a pattern fill for the current path. Dispatches on /PatternType.
    fn paint_pattern_fill(&mut self, rule: FillRule, pattern_name: &str) {
        let pattern_obj = match self.resources.patterns.get(pattern_name) {
            Some(obj) => obj.clone(),
            None => {
                log::warn!("pattern fill: pattern '{}' not found", pattern_name);
                return;
            }
        };
        let reader = self.engine.document().reader();
        let Some(pattern_dict) = resolve_to_dict(&pattern_obj, reader) else {
            log::warn!(
                "pattern fill: '{}' did not resolve to a dictionary",
                pattern_name
            );
            return;
        };
        if !self
            .optional_content
            .is_object_visible(pattern_dict.get("OC"), reader)
        {
            return;
        }
        self.record_pattern_caller_plate_sample("pattern_fill_caller_color");

        match pattern_dict.get_integer("PatternType").unwrap_or(0) {
            1 => self.paint_tiling_pattern_fill(rule, &pattern_obj),
            2 => self.paint_shading_pattern_fill(rule, &pattern_dict),
            other => log::debug!("pattern fill: unknown PatternType {other}"),
        }
    }

    /// Paint a pattern stroke by converting the stroked path into the same
    /// device-space clip shape used by solid strokes, then replaying the
    /// pattern through that clip. PDF pattern color spaces apply equally to
    /// `SCN` strokes and `scn` fills; keeping the stroke path source-linked here
    /// closes the prior renderer gap where patterned strokes degraded to a
    /// single fallback paint color.
    fn paint_pattern_stroke(&mut self, pattern_name: &str) {
        let pattern_obj = match self.resources.patterns.get(pattern_name) {
            Some(obj) => obj.clone(),
            None => {
                log::warn!("pattern stroke: pattern '{}' not found", pattern_name);
                return;
            }
        };
        let reader = self.engine.document().reader();
        let Some(pattern_dict) = resolve_to_dict(&pattern_obj, reader) else {
            log::warn!(
                "pattern stroke: '{}' did not resolve to a dictionary",
                pattern_name
            );
            return;
        };
        if !self
            .optional_content
            .is_object_visible(pattern_dict.get("OC"), reader)
        {
            return;
        }
        self.record_pattern_caller_plate_sample("pattern_stroke_caller_color");

        match pattern_dict.get_integer("PatternType").unwrap_or(0) {
            1 => self.paint_tiling_pattern_stroke(&pattern_obj),
            2 => self.paint_shading_pattern_stroke(&pattern_dict),
            other => log::debug!("pattern stroke: unknown PatternType {other}"),
        }
    }

    /// Paint a tiling pattern (PatternType 1) clipped to the current path.
    ///
    /// The tile content stream is rendered repeatedly across the path's
    /// device-space bounding box at `/XStep`/`/YStep` spacing, each repetition
    /// positioned via the pattern `/Matrix` (relative to the base CTM of the
    /// pattern's parent content stream) and clipped to the filled path.
    fn paint_tiling_pattern_fill(&mut self, rule: FillRule, pattern_obj: &PdfObject) {
        let path_ctm = self.ctm();
        let flat = flatten_path(&self.path, &path_ctm, &self.viewport, 0.5);
        let mut path_clip = ClipMask::from_path(&flat, self.buf.width, self.buf.height, rule);
        if let Some(existing) = self.buf.clip_mask() {
            path_clip.intersect(existing);
        }
        self.paint_tiling_pattern_with_device_clip(pattern_obj, path_clip, &flat, false);
    }

    fn paint_tiling_pattern_stroke(&mut self, pattern_obj: &PdfObject) {
        let Some((path_clip, outline)) = self.stroke_device_clip() else {
            return;
        };
        self.paint_tiling_pattern_with_device_clip(pattern_obj, path_clip, &outline, true);
    }

    fn paint_tiling_pattern_with_device_clip(
        &mut self,
        pattern_obj: &PdfObject,
        path_clip: ClipMask,
        flat: &FlatPath,
        use_stroke_color: bool,
    ) {
        if self.form_depth >= 8 {
            log::warn!("tiling pattern: nesting depth limit reached; skipping");
            return;
        }
        let reader = self.engine.document().reader();
        let (pat_dict, raw_bytes) = match resolve_to_stream(pattern_obj, reader) {
            Some(pair) => pair,
            None => {
                log::warn!("tiling pattern: did not resolve to a content stream");
                return;
            }
        };
        let bbox = match get_float_array_dict(&pat_dict, "BBox") {
            Some(b) if b.len() >= 4 => [b[0], b[1], b[2], b[3]],
            _ => {
                log::warn!("tiling pattern: missing /BBox");
                return;
            }
        };
        let x_step = pat_dict
            .get("XStep")
            .and_then(PdfObject::as_number)
            .unwrap_or(0.0);
        let y_step = pat_dict
            .get("YStep")
            .and_then(PdfObject::as_number)
            .unwrap_or(0.0);
        if x_step.abs() < 1e-6 || y_step.abs() < 1e-6 {
            log::warn!("tiling pattern: zero XStep/YStep; skipping");
            return;
        }
        let paint_type = pat_dict.get_integer("PaintType").unwrap_or(1);
        let raw_hash = fingerprint_bytes64(&raw_bytes);
        let pattern_key = tiling_pattern_stack_key(pattern_obj, &pat_dict, raw_bytes.len());
        let program_cache_key = format!("tiling-program:{pattern_key}:raw:{raw_hash:016x}");
        if self.pattern_stack.iter().any(|key| key == &pattern_key) {
            self.record_fatal_render_error(format!(
                "recursive tiling pattern resource /{pattern_key} cannot be rendered exactly"
            ));
            return;
        }

        // pattern space â†’ device. The pattern /Matrix is relative to the base
        // CTM of the parent content stream (NOT the fill-time CTM).
        let pat_matrix = match get_float_array_dict(&pat_dict, "Matrix") {
            Some(m) if m.len() >= 6 => Transform2D::from([m[0], m[1], m[2], m[3], m[4], m[5]]),
            _ => Transform2D::identity(),
        };
        let pattern_ctm = pat_matrix.concat(&self.base_ctm);

        // Determine the device-space bounding box of the filled path to bound
        // how many tiles we need; then map that back into pattern space.
        let (dx0, dy0, dx1, dy1) = path_device_bounds(flat, self.buf.width, self.buf.height);
        if dx1 < dx0 || dy1 < dy0 {
            return; // empty path
        }
        let full = pattern_ctm.concat(&self.viewport.to_transform());
        let inv = match full.inverse() {
            Some(inv) => inv,
            None => {
                log::warn!("tiling pattern: singular pattern transform");
                return;
            }
        };
        // Map the four device-bbox corners into pattern space.
        let corners = [
            inv.transform_point(dx0 as f64, dy0 as f64),
            inv.transform_point(dx1 as f64, dy0 as f64),
            inv.transform_point(dx0 as f64, dy1 as f64),
            inv.transform_point(dx1 as f64, dy1 as f64),
        ];
        let (mut pminx, mut pminy, mut pmaxx, mut pmaxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (px, py) in corners {
            pminx = pminx.min(px);
            pminy = pminy.min(py);
            pmaxx = pmaxx.max(px);
            pmaxy = pmaxy.max(py);
        }
        // Tile index range: which (i,j) translations of the tile overlap the
        // pattern-space region, accounting for the BBox extent vs the step.
        let i0 = ((pminx - bbox[2]) / x_step).floor() as i64;
        let i1 = ((pmaxx - bbox[0]) / x_step).ceil() as i64;
        let j0 = ((pminy - bbox[3]) / y_step).floor() as i64;
        let j1 = ((pmaxy - bbox[1]) / y_step).ceil() as i64;

        let tile_count = (i1 - i0 + 1).max(0) as i128 * (j1 - j0 + 1).max(0) as i128;
        const COMPAT_TILE_CAP: i128 = 4_096;
        const HIGH_QUALITY_TILE_CAP: i128 = 20_000;
        let tile_cap = if self.buf.render_mode().is_high_quality() {
            HIGH_QUALITY_TILE_CAP
        } else {
            COMPAT_TILE_CAP
        };
        if tile_count > tile_cap {
            self.record_fatal_render_error(format!(
                "tiling pattern requires {tile_count} visible cells, exceeding exact render limit {tile_cap}"
            ));
            return;
        }
        if tile_count == 0 {
            return;
        }

        let Some(ops) =
            self.cached_tiling_pattern_program(&program_cache_key, &pat_dict, raw_bytes)
        else {
            return;
        };

        let pat_resources = if let Some(res_obj) = pat_dict.get("Resources") {
            let pr = crate::engine::parse_resources_from_obj(res_obj, reader);
            merge_resources(pr, &self.resources)
        } else {
            self.resources.clone()
        };

        // For PaintType 2 (uncolored), the tile is painted in the current fill
        // color; the tile's own content stream must not set color. The fill
        // color space is the special Pattern space, so reconstruct the concrete
        // color from the numeric components recorded by `scn` (by component
        // count: 1 -> gray, 3 -> RGB, 4 -> CMYK).
        let forced_color = if paint_type == 2 {
            let color = if use_stroke_color {
                &self.gs.stroke_color
            } else {
                &self.gs.fill_color
            };
            Some(uncolored_pattern_color(color))
        } else {
            None
        };

        // Install the path clip, then render each tile (each clips additionally
        // to its own BBox). The path clip bounds the whole fill to the shape.
        let saved_clip = self.buf.clip_mask().cloned();
        self.buf.set_clip(path_clip);
        self.pattern_stack.push(pattern_key);

        for j in j0..=j1 {
            for i in i0..=i1 {
                // Each tile replays the pattern's full content stream, so even
                // under the 20k-tile cap this loop can be expensive. Poll the
                // cancellation flag once per tile (cheap relative to a tile
                // render) so a pathological pattern stops promptly on timeout.
                if self.cancel.is_cancelled() {
                    self.pattern_stack.pop();
                    self.buf.restore_clip(saved_clip);
                    return;
                }
                let translate =
                    Transform2D::new(1.0, 0.0, 0.0, 1.0, i as f64 * x_step, j as f64 * y_step);
                let tile_ctm = translate.concat(&pattern_ctm);
                self.render_pattern_tile(
                    ops.as_ref().as_slice(),
                    &pat_resources,
                    tile_ctm,
                    bbox,
                    forced_color.as_ref(),
                );
            }
        }

        self.pattern_stack.pop();
        self.buf.restore_clip(saved_clip);
    }

    /// Render a single tile of a tiling pattern at `tile_ctm`, clipped to the
    /// tile's BBox (the page-path clip is already installed on `self.buf`).
    fn render_pattern_tile(
        &mut self,
        ops: &[ContentOperation],
        resources: &PageResources,
        tile_ctm: Transform2D,
        bbox: [f64; 4],
        forced_color: Option<&(ColorSpace, crate::content::state::Color)>,
    ) {
        let saved_gs = self.gs.clone();
        let saved_resources = self.resources.clone();
        let saved_base_ctm = self.base_ctm;
        let saved_clip = self.buf.clip_mask().cloned();
        self.form_depth += 1;

        self.gs.ctm = tile_ctm.to_array();
        self.base_ctm = tile_ctm;
        self.resources = resources.clone();
        if let Some((space, color)) = forced_color {
            self.gs.fill_color_space = space.clone();
            self.gs.fill_color = color.clone();
            self.gs.stroke_color_space = space.clone();
            self.gs.stroke_color = color.clone();
        }

        // Intersect the tile BBox so tile content cannot bleed past one cell.
        let x_min = bbox[0].min(bbox[2]);
        let y_min = bbox[1].min(bbox[3]);
        let w = (bbox[2] - bbox[0]).abs();
        let h = (bbox[3] - bbox[1]).abs();
        if w > 0.0 && h > 0.0 {
            let bbox_clip = if let Some((x, y, width, height)) = axis_aligned_bbox_clip_rect(
                bbox,
                &self.ctm(),
                &self.viewport,
                self.buf.width,
                self.buf.height,
            ) {
                ClipMask::from_visible_rect(self.buf.width, self.buf.height, x, y, width, height)
            } else {
                let mut bbox_path = Path::new();
                bbox_path.rect(x_min, y_min, w, h);
                let flat = flatten_path(&bbox_path, &self.ctm(), &self.viewport, 0.5);
                ClipMask::from_path(&flat, self.buf.width, self.buf.height, FillRule::NonZero)
            };
            self.buf.set_clip(bbox_clip); // intersects with the installed path clip
        }

        self.dispatch_all(ops);

        // Restore.
        self.form_depth = self.form_depth.saturating_sub(1);
        self.gs = saved_gs;
        self.resources = saved_resources;
        self.base_ctm = saved_base_ctm;
        self.buf.restore_clip(saved_clip);
        self.sync_blend_mode();
    }

    /// Paint a shading pattern (PatternType 2) clipped to the current path.
    fn paint_shading_pattern_fill(&mut self, rule: FillRule, pattern_dict: &PdfDictionary) {
        let path_ctm = self.ctm();
        let flat = flatten_path(&self.path, &path_ctm, &self.viewport, 0.5);
        let path_clip = ClipMask::from_path(&flat, self.buf.width, self.buf.height, rule);
        self.paint_shading_pattern_with_device_clip(pattern_dict, path_clip);
    }

    fn paint_shading_pattern_stroke(&mut self, pattern_dict: &PdfDictionary) {
        let Some((path_clip, _outline)) = self.stroke_device_clip() else {
            return;
        };
        self.paint_shading_pattern_with_device_clip(pattern_dict, path_clip);
    }

    fn paint_shading_pattern_with_device_clip(
        &mut self,
        pattern_dict: &PdfDictionary,
        path_clip: ClipMask,
    ) {
        let reader = self.engine.document().reader();
        let shading_obj = match pattern_dict.get("Shading") {
            Some(obj) => obj.clone(),
            None => {
                log::warn!("shading pattern: missing /Shading entry");
                return;
            }
        };
        let Some(shading_dict) = resolve_to_dict(&shading_obj, reader) else {
            log::warn!("shading pattern: /Shading did not resolve to a dictionary");
            return;
        };
        let shading_dict = self.shading_dict_with_resolved_color_space(shading_dict);
        self.record_shading_plate_sample(
            &shading_dict,
            format!("page {} shading pattern", self.page_number),
            "pattern_shading",
        );

        // The pattern carries its own /Matrix (pattern space â†’ the default user
        // coordinate system of the pattern's parent content stream). Per PDF
        // 32000-1 Â§8.7.3.1 the pattern matrix is relative to that *base* CTM, not
        // the CTM in effect at the moment of the fill, so combine it with
        // `base_ctm` (matching the tiling-pattern path).
        let ctm = match get_float_array_dict(pattern_dict, "Matrix") {
            Some(m) if m.len() >= 6 => {
                let pat = Transform2D::from([m[0], m[1], m[2], m[3], m[4], m[5]]);
                pat.concat(&self.base_ctm)
            }
            _ => self.base_ctm,
        };

        let saved_clip = self.buf.clip_mask().cloned();
        self.buf.set_clip(path_clip); // intersects with any existing clip

        let mesh_data = self.shading_mesh_data(&shading_obj, &shading_dict, reader);
        ShadingRenderer::paint(
            &shading_dict,
            &ctm,
            &self.viewport,
            &mut self.buf,
            reader,
            mesh_data.as_deref().map(Vec::as_slice),
        );

        // Restore the exact previous clip (restore_clip sets directly).
        self.buf.restore_clip(saved_clip);
    }

    fn shading_dict_with_resolved_color_space(
        &self,
        mut shading_dict: PdfDictionary,
    ) -> PdfDictionary {
        let Some(PdfObject::Name(resource_name)) = shading_dict
            .get("ColorSpace")
            .or_else(|| shading_dict.get("CS"))
        else {
            return shading_dict;
        };
        let Some(resource_obj) = self.resources.color_spaces.get(resource_name).cloned() else {
            return shading_dict;
        };
        shading_dict.insert("ColorSpace", resource_obj);
        shading_dict
    }

    fn stroke_device_clip(&self) -> Option<(ClipMask, FlatPath)> {
        if self.path.is_empty() || self.buf.width == 0 || self.buf.height == 0 {
            return None;
        }
        let ctm = self.ctm();
        let flat = flatten_path(&self.path, &ctm, &self.viewport, 0.2);
        let width_px = (self.gs.line_width * ctm.scale_factor() * self.viewport.scale).max(1.0);
        let outline = stroke_flat_path(
            &flat,
            width_px,
            &self.dash_state(),
            self.gs.line_cap.clone(),
            self.gs.line_join.clone(),
            self.gs.miter_limit,
        );
        if outline.subpaths.is_empty() {
            return None;
        }
        // Pattern strokes need a device-space clip for the stroked outline.
        // Generate that clip directly from the already-computed stroke outline
        // using row-run scan conversion. This avoids allocating a page-sized
        // alpha buffer only to threshold it back into a binary clip.
        let mut path_clip = rasterize_flat_binary_clip_mask(
            &outline,
            self.buf.width,
            self.buf.height,
            FillRule::NonZero,
            Some(&self.cancel),
        )?;
        if let Some(existing) = self.buf.clip_mask() {
            path_clip.intersect(existing);
        }
        Some((path_clip, outline))
    }

    fn render_text_array(&mut self, op: &ContentOperation) {
        let Some(items) = op.operand(0).and_then(Operand::as_array) else {
            return;
        };
        for item in items {
            match item {
                Operand::String(bytes) => self.render_text_string(bytes),
                Operand::Integer(value) => self.adjust_text_position(-(*value as f64)),
                Operand::Real(value) => self.adjust_text_position(-*value),
                _ => {}
            }
        }
    }

    fn render_text_string(&mut self, bytes: &[u8]) {
        let font_name = self.gs.text.font_name.clone();
        let font_size = self.gs.text.font_size;
        if font_size <= 0.0 {
            return;
        }
        let font_cache_key = if let Some(font_dict) = self.resources.fonts.get(&font_name) {
            let lookup = (
                font_name.clone(),
                font_dict as *const PdfDictionary as usize,
            );
            if let Some(cached) = self.font_resource_key_cache.get(&lookup) {
                cached.clone()
            } else {
                let computed = font_resource_cache_key(&font_name, font_dict);
                self.font_resource_key_cache
                    .insert(lookup, computed.clone());
                computed
            }
        } else {
            format!("{font_name}:missing")
        };
        let font_dict = self.resources.fonts.get(&font_name).cloned();
        let decoded = if let Some(font_dict) = font_dict.as_ref() {
            let resolver = self.get_font_resolver(&font_cache_key, font_dict);
            decode_text_bytes_with_resolver(
                bytes,
                font_dict,
                &resolver,
                self.engine.document().reader(),
            )
        } else {
            crate::render::text_decode::decode_text_bytes(
                bytes,
                &font_name,
                &self.resources,
                self.engine.document().reader(),
            )
        };
        let font_subtype = font_dict
            .as_ref()
            .map(detect_font_subtype)
            .unwrap_or(FontSubtype::Unknown);
        let is_type3 = font_subtype == FontSubtype::Type3;
        let font_bytes = self.get_font_bytes(&font_name, &font_cache_key);
        let variation = font_dict
            .as_ref()
            .map(|font_dict| self.font_variation_request_from_dict(font_dict))
            .unwrap_or_else(VariationRequest::none);
        let font_hash = font_bytes
            .as_ref()
            .filter(|bytes| !bytes.is_empty())
            .map(|bytes| font_resource_glyph_cache_hash(bytes.as_slice(), &font_cache_key));
        let upem = font_bytes
            .as_ref()
            .and_then(|bytes| Self::get_upem(bytes.as_slice()))
            .map(f64::from)
            .filter(|value| *value > 0.0)
            .unwrap_or(1000.0);
        let light_hinting_supported = font_bytes
            .as_ref()
            .map(|bytes| {
                ttf_parser::Face::parse(bytes.as_slice(), 0).is_ok()
                    || crate::fonts::type1::Type1Font::is_type1(bytes.as_slice())
            })
            .unwrap_or(false);

        for (glyph_index, glyph) in decoded.into_iter().enumerate() {
            if glyph_index % 32 == 0 && self.cancel.is_cancelled() {
                return;
            }
            let mut ttf_advance = None;
            let text_mode = self.gs.text.rendering_mode;
            if should_paint_decoded_glyph(&glyph)
                && (text_rendering_mode_paints(text_mode) || text_rendering_mode_clips(text_mode))
            {
                if is_type3 {
                    if let Some(font_dict) = font_dict.as_ref() {
                        ttf_advance = self.render_type3_glyph(&font_name, font_dict, &glyph);
                    }
                    if ttf_advance.is_none() && !text_rendering_mode_clips(text_mode) {
                        self.record_fatal_render_error(format!(
                            "Type 3 glyph {} in font /{} could not be rendered through its native charproc",
                            glyph.code, font_name
                        ));
                    }
                } else if let (Some(font_bytes), Some(font_hash)) = (font_bytes.as_ref(), font_hash)
                {
                    if !font_bytes.is_empty() {
                        ttf_advance = self.render_glyph_with_cache(GlyphRenderRequest {
                            font_name: &font_name,
                            font_subtype: font_subtype.clone(),
                            code: glyph.code,
                            ch: glyph.unicode,
                            glyph_name: glyph.glyph_name.as_deref(),
                            is_gid: glyph.is_gid,
                            font_bytes: font_bytes.as_slice(),
                            font_hash,
                            variation: &variation,
                            upem,
                            light_hinting_supported,
                            offset_x: glyph.vertical_origin.map(|(vx, _)| -vx).unwrap_or(0.0),
                            offset_y: glyph.vertical_origin.map(|(_, vy)| vy).unwrap_or(0.0),
                        });
                    }
                }
            }
            let advance = glyph.width.or(ttf_advance).unwrap_or(500.0);
            self.advance_decoded_text(advance, &glyph);
        }
    }

    fn render_glyph_with_cache(&mut self, request: GlyphRenderRequest<'_>) -> Option<f64> {
        let glyph_id = crate::render::color_glyph::resolve_request_glyph_id(
            request.font_bytes,
            request.is_gid,
            request.code,
            request.ch,
            request.glyph_name,
            request.variation,
        );
        let color_mode = glyph_id
            .map(|gid| crate::render::color_glyph::color_glyph_kind(request.font_bytes, gid))
            .unwrap_or(crate::render::color_glyph::ColorGlyphKind::None)
            .cache_mode();
        let cache_key = GlyphCacheKey {
            font_hash: request.font_hash,
            variation_hash: request.variation.cache_hash(),
            code: request.code,
            is_gid: request.is_gid,
            color_mode,
        };
        let cached = self.glyph_cache.get(&cache_key).cloned();
        let cached = match cached {
            Some(cached) => cached,
            None => {
                let (path, advance_width) = if request.is_gid {
                    crate::render::glyph_outline::extract_glyph_path_by_gid_var(
                        request.font_bytes,
                        request.code,
                        request.variation,
                    )
                } else {
                    crate::render::glyph_outline::extract_glyph_path_for_simple_var(
                        request.font_bytes,
                        request.code,
                        request.ch,
                        request.glyph_name,
                        request.variation,
                    )
                };
                let cached = CachedGlyph::from_path(path, advance_width);
                self.glyph_cache.insert(cache_key.clone(), cached.clone());
                cached
            }
        };

        let advance_width = cached.advance_width;

        let scale = font_size_scale(self.gs.text.font_size, request.upem);
        let th = self.gs.text.horizontal_scaling / 100.0;
        let scale_x = scale * th;
        if scale <= 0.0 || !scale_x.is_finite() {
            return Some(advance_width);
        }

        let scale_t = Transform2D::scale(scale_x, scale);
        let offset_t = Transform2D::translation(
            request.offset_x / 1000.0 * self.gs.text.font_size * th,
            request.offset_y / 1000.0 * self.gs.text.font_size,
        );
        let rise_t = Transform2D::translation(0.0, self.gs.text.rise);
        let tm_t = Transform2D::from(self.gs.text.tm);
        let ctm = self.ctm();
        let glyph_ctm = scale_t
            .concat(&offset_t)
            .concat(&rise_t)
            .concat(&tm_t)
            .concat(&ctm);

        // Light baseline grid-fitting is bounded to normal body-text sizes in
        // `GlyphHinting::light`, and is enabled only for outline formats whose
        // charstring/outline reconstruction has passed focused parity fixtures
        // (TrueType and the Type1 body-text path).
        let glyph_pixel_size =
            self.gs.text.font_size * self.ctm().scale_factor() * self.viewport.scale;
        let glyph_hinting = if request.light_hinting_supported {
            GlyphHinting::light(glyph_pixel_size)
        } else {
            GlyphHinting::disabled()
        };

        let glyph_path = cached.path;
        if text_rendering_mode_clips(self.gs.text.rendering_mode) {
            if let Some(path) = glyph_path.as_deref() {
                self.accumulate_text_clip(path, &glyph_ctm);
            } else {
                log::debug!(
                    "PageRenderer: text clipping requested but glyph outline was unavailable: font='{}' subtype={:?} {}={} unicode=U+{:04X} glyph_name={:?}",
                    request.font_name,
                    request.font_subtype,
                    if request.is_gid { "gid" } else { "code" },
                    request.code,
                    request.ch as u32,
                    request.glyph_name
                );
                self.fail_closed_text_clip();
                return Some(advance_width);
            }
        }
        if !text_rendering_mode_paints(self.gs.text.rendering_mode) {
            return Some(advance_width);
        }

        let fill_color = self.fill_pixel_color();
        let stroke_color = self.stroke_pixel_color();
        let fill_mode = matches!(self.gs.text.rendering_mode, 0 | 2 | 4 | 6);
        let stroke_mode = matches!(self.gs.text.rendering_mode, 1 | 2 | 5 | 6);
        if fill_mode {
            let fill_color_state = self.gs.fill_color.clone();
            self.record_plate_contribution(
                &fill_color_state,
                self.gs.fill_alpha as f32,
                "text_fill",
            );
        }
        if stroke_mode {
            let stroke_color_state = self.gs.stroke_color.clone();
            self.record_plate_contribution(
                &stroke_color_state,
                self.gs.stroke_alpha as f32,
                "text_stroke",
            );
        }
        let color_fill_painted = if fill_mode {
            glyph_id
                .map(|gid| {
                    self.paint_color_glyph_fill(
                        &request,
                        gid,
                        &glyph_ctm,
                        glyph_hinting,
                        fill_color,
                        glyph_pixel_size,
                    )
                })
                .unwrap_or(false)
        } else {
            false
        };

        let Some(glyph_path) = glyph_path.as_deref() else {
            return Some(advance_width);
        };

        match self.gs.text.rendering_mode {
            0 | 4 => {
                if !color_fill_painted
                    && !self.paint_cached_glyph_fill(
                        &cache_key,
                        glyph_path,
                        &glyph_ctm,
                        fill_color,
                        glyph_hinting,
                    )
                {
                    PathPainter::fill_glyph(
                        &mut self.buf,
                        glyph_path,
                        &glyph_ctm,
                        &self.viewport,
                        fill_color,
                        FillRule::NonZero,
                        glyph_hinting,
                    );
                }
            }
            1 | 5 => {
                PathPainter::stroke_with_style_fast_cancellable(
                    &mut self.buf,
                    glyph_path,
                    &glyph_ctm,
                    &self.viewport,
                    stroke_color,
                    self.gs.line_width,
                    &DashState::solid(),
                    &LineCap::Butt,
                    &LineJoin::Miter,
                    10.0,
                    &self.cancel,
                );
            }
            2 | 6 => {
                if !color_fill_painted
                    && !self.paint_cached_glyph_fill(
                        &cache_key,
                        glyph_path,
                        &glyph_ctm,
                        fill_color,
                        glyph_hinting,
                    )
                {
                    PathPainter::fill_glyph(
                        &mut self.buf,
                        glyph_path,
                        &glyph_ctm,
                        &self.viewport,
                        fill_color,
                        FillRule::NonZero,
                        glyph_hinting,
                    );
                }
                PathPainter::stroke_with_style_fast_cancellable(
                    &mut self.buf,
                    glyph_path,
                    &glyph_ctm,
                    &self.viewport,
                    stroke_color,
                    self.gs.line_width,
                    &DashState::solid(),
                    &LineCap::Butt,
                    &LineJoin::Miter,
                    10.0,
                    &self.cancel,
                );
            }
            3 | 7 => {}
            other => log::warn!("PageRenderer: unknown text render mode {}", other),
        }
        Some(advance_width)
    }

    fn paint_cached_glyph_fill(
        &mut self,
        glyph_key: &GlyphCacheKey,
        glyph_path: &Path,
        glyph_ctm: &Transform2D,
        fill_color: PixelColor,
        glyph_hinting: GlyphHinting,
    ) -> bool {
        let device_t = glyph_ctm.concat(&self.viewport.to_transform());
        if [
            device_t.a, device_t.b, device_t.c, device_t.d, device_t.e, device_t.f,
        ]
        .iter()
        .any(|value| !value.is_finite())
        {
            return false;
        }
        if device_t.scale_factor() <= 0.0 || device_t.scale_factor() > 256.0 {
            return false;
        }
        let origin_x = device_t.e.floor();
        let origin_y = device_t.f.floor();
        let normalized_t = Transform2D {
            e: device_t.e - origin_x,
            f: device_t.f - origin_y,
            ..device_t
        };
        let key = GlyphMaskCacheKey {
            glyph: glyph_key.clone(),
            a: quantize_glyph_mask_value(normalized_t.a),
            b: quantize_glyph_mask_value(normalized_t.b),
            c: quantize_glyph_mask_value(normalized_t.c),
            d: quantize_glyph_mask_value(normalized_t.d),
            frac_e: quantize_glyph_mask_fraction(normalized_t.e),
            frac_f: quantize_glyph_mask_fraction(normalized_t.f),
            hinting: glyph_hinting.should_apply(),
        };
        let dx = if origin_x <= i32::MIN as f64 {
            i32::MIN
        } else if origin_x >= i32::MAX as f64 {
            i32::MAX
        } else {
            origin_x as i32
        };
        let dy = if origin_y <= i32::MIN as f64 {
            i32::MIN
        } else if origin_y >= i32::MAX as f64 {
            i32::MAX
        } else {
            origin_y as i32
        };
        if let Some(mask) = self.glyph_mask_cache.get(&key) {
            mask.paint(&mut self.buf, dx, dy, fill_color);
            return true;
        }
        let Some(mask) =
            rasterize_glyph_alpha_mask(glyph_path, &normalized_t, FillRule::NonZero, glyph_hinting)
        else {
            return false;
        };
        let mask = Arc::new(mask);
        mask.paint(&mut self.buf, dx, dy, fill_color);
        self.glyph_mask_cache.insert(key, mask);
        true
    }

    fn paint_color_glyph_fill(
        &mut self,
        request: &GlyphRenderRequest<'_>,
        glyph_id: ttf_parser::GlyphId,
        glyph_ctm: &Transform2D,
        glyph_hinting: GlyphHinting,
        fill_color: PixelColor,
        glyph_pixel_size: f64,
    ) -> bool {
        match crate::render::color_glyph::colr_cpal_layers(
            request.font_bytes,
            glyph_id,
            fill_color,
            fill_color[3],
            request.variation,
        ) {
            Ok(Some(layers)) => {
                let ops: Vec<_> = layers
                    .into_iter()
                    .map(|layer| crate::render::color_glyph::ColrPaintOp {
                        glyph_id: layer.glyph_id,
                        transform: layer.transform,
                        paint: crate::render::color_glyph::ColrPaint::Solid(layer.color),
                        clips: Vec::new(),
                        blend_mode: crate::render::color_glyph::ColrBlendMode::Normal,
                    })
                    .collect();
                if self.paint_colr_paint_ops(request, &ops, glyph_ctm, glyph_hinting) {
                    return true;
                }
            }
            Ok(None) => {}
            Err(_compat_err) => match crate::render::color_glyph::colr_cpal_paint_ops(
                request.font_bytes,
                glyph_id,
                fill_color,
                fill_color[3],
                request.variation,
            ) {
                Ok(Some(ops)) => {
                    if self.paint_colr_paint_ops(request, &ops, glyph_ctm, glyph_hinting) {
                        return true;
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    log::warn!(
                        "PageRenderer: COLR/CPAL glyph paint failed for font='{}' glyph={} error={}",
                        request.font_name,
                        glyph_id.0,
                        err
                    );
                    return true;
                }
            },
        }

        match crate::render::color_glyph::svg_static_glyph_paints(
            request.font_bytes,
            glyph_id,
            fill_color,
            fill_color[3],
        ) {
            Ok(Some(paths)) => {
                let mut painted = false;
                for svg_path in paths {
                    let path_ctm = svg_path.transform.concat(glyph_ctm);
                    if let Some(color) = svg_path.fill {
                        PathPainter::fill_glyph(
                            &mut self.buf,
                            &svg_path.path,
                            &path_ctm,
                            &self.viewport,
                            color,
                            FillRule::NonZero,
                            GlyphHinting::disabled(),
                        );
                        painted = true;
                    }
                    if let Some(color) = svg_path.stroke {
                        PathPainter::stroke(
                            &mut self.buf,
                            &svg_path.path,
                            &path_ctm,
                            &self.viewport,
                            color,
                            svg_path.stroke_width,
                            &DashState::solid(),
                        );
                        painted = true;
                    }
                }
                if painted {
                    return true;
                }
            }
            Ok(None) => {}
            Err(err) => {
                log::warn!(
                    "PageRenderer: SVG-in-OpenType glyph paint failed for font='{}' glyph={} error={}",
                    request.font_name,
                    glyph_id.0,
                    err
                );
                return true;
            }
        }

        let target_ppem = glyph_pixel_size.round().clamp(1.0, f64::from(u16::MAX)) as u16;
        let raster = match crate::render::color_glyph::decode_raster_glyph_image(
            request.font_bytes,
            glyph_id,
            target_ppem,
        ) {
            Ok(Some(raster)) => raster,
            Ok(None) => return false,
            Err(err) => {
                log::warn!(
                    "PageRenderer: color glyph raster decode failed for font='{}' glyph={} error={}",
                    request.font_name,
                    glyph_id.0,
                    err
                );
                return true;
            }
        };
        let units_per_pixel = request.upem / f64::from(raster.pixels_per_em);
        let image_ctm = Transform2D::scale(
            f64::from(raster.image.width) * units_per_pixel,
            f64::from(raster.image.height) * units_per_pixel,
        )
        .concat(&Transform2D::translation(
            f64::from(raster.x) * units_per_pixel,
            f64::from(raster.y) * units_per_pixel,
        ))
        .concat(glyph_ctm);
        ImagePainter::paint_image_with_alpha(
            &mut self.buf,
            &raster.image,
            &image_ctm,
            &self.viewport,
            f32::from(fill_color[3]) / 255.0,
        );
        true
    }

    fn paint_colr_paint_ops(
        &mut self,
        request: &GlyphRenderRequest<'_>,
        ops: &[crate::render::color_glyph::ColrPaintOp],
        glyph_ctm: &Transform2D,
        glyph_hinting: GlyphHinting,
    ) -> bool {
        if ops.is_empty() {
            return false;
        }
        let tile = self
            .colr_paint_ops_device_tile(request, ops, glyph_ctm)
            .unwrap_or_else(|| RenderTile::full(self.buf.width, self.buf.height));
        let token = match self.reserve_offscreen_surface(
            tile.width,
            tile.height,
            "COLRv1 glyph paint surface",
        ) {
            Ok(token) => token,
            Err(err) => {
                log::warn!(
                    "PageRenderer: COLRv1 glyph paint surface denied for font='{}' error={}",
                    request.font_name,
                    err
                );
                return true;
            }
        };
        let mut surface =
            self.take_transparent_offscreen_buffer(tile.width, tile.height, self.buf.render_mode());
        let original_viewport = self.viewport.clone();
        self.viewport = original_viewport.pixel_window(tile.x, tile.y, tile.width, tile.height);
        let mut painted = false;
        for op in ops {
            painted |=
                self.paint_colr_op_to_buffer(&mut surface, request, op, glyph_ctm, glyph_hinting);
        }
        self.viewport = original_viewport;
        drop(token);
        if painted {
            let smask = self.buf.smask_mask().cloned();
            self.buf.composite_from_at(
                &surface,
                tile.x,
                tile.y,
                1.0,
                BlendMode::Normal,
                smask.as_ref(),
            );
        }
        self.recycle_offscreen_buffer(surface);
        painted
    }

    fn colr_paint_ops_device_tile(
        &self,
        request: &GlyphRenderRequest<'_>,
        ops: &[crate::render::color_glyph::ColrPaintOp],
        glyph_ctm: &Transform2D,
    ) -> Option<RenderTile> {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for op in ops {
            let Some(path) = crate::render::color_glyph::outline_gid_path(
                request.font_bytes,
                op.glyph_id,
                request.variation,
            ) else {
                continue;
            };
            let op_ctm = op.transform.concat(glyph_ctm);
            let flat = flatten_path(&path, &op_ctm, &self.viewport, 0.2);
            let (x0, y0, x1, y1) = path_device_bounds(&flat, self.buf.width, self.buf.height);
            if x1 < x0 || y1 < y0 {
                continue;
            }
            min_x = min_x.min(x0);
            min_y = min_y.min(y0);
            max_x = max_x.max(x1);
            max_y = max_y.max(y1);
        }
        if max_x < min_x || max_y < min_y {
            return None;
        }
        let pad = 2i32;
        let x0 = min_x.saturating_sub(pad).max(0);
        let y0 = min_y.saturating_sub(pad).max(0);
        let x1 = max_x
            .saturating_add(pad)
            .min(self.buf.width.saturating_sub(1) as i32);
        let y1 = max_y
            .saturating_add(pad)
            .min(self.buf.height.saturating_sub(1) as i32);
        if x1 < x0 || y1 < y0 {
            return None;
        }
        Some(RenderTile {
            x: x0 as u32,
            y: y0 as u32,
            width: x1.saturating_sub(x0).saturating_add(1) as u32,
            height: y1.saturating_sub(y0).saturating_add(1) as u32,
        })
    }

    fn paint_colr_op_to_buffer(
        &self,
        buf: &mut PixelBuffer,
        request: &GlyphRenderRequest<'_>,
        op: &crate::render::color_glyph::ColrPaintOp,
        glyph_ctm: &Transform2D,
        glyph_hinting: GlyphHinting,
    ) -> bool {
        if colr_is_porter_duff(op.blend_mode) {
            return self.paint_colr_porter_duff_op_to_buffer(
                buf,
                request,
                op,
                glyph_ctm,
                glyph_hinting,
            );
        }
        let Some(path) = crate::render::color_glyph::outline_gid_path(
            request.font_bytes,
            op.glyph_id,
            request.variation,
        ) else {
            log::warn!(
                "PageRenderer: COLRv1 paint op missing glyph outline: font='{}' glyph={}",
                request.font_name,
                op.glyph_id
            );
            return false;
        };
        let saved_clip = buf.clip_mask().cloned();
        if !self.install_colr_clips(buf, request, &op.clips, glyph_ctm) {
            buf.restore_clip(saved_clip);
            return false;
        }
        let saved_blend = buf.blend_mode;
        buf.blend_mode = colr_blend_to_pdf(op.blend_mode);
        let op_ctm = op.transform.concat(glyph_ctm);
        self.paint_colr_paint_to_buffer(buf, &path, &op_ctm, glyph_hinting, &op.paint);
        buf.blend_mode = saved_blend;
        buf.restore_clip(saved_clip);
        true
    }

    fn paint_colr_porter_duff_op_to_buffer(
        &self,
        buf: &mut PixelBuffer,
        request: &GlyphRenderRequest<'_>,
        op: &crate::render::color_glyph::ColrPaintOp,
        glyph_ctm: &Transform2D,
        glyph_hinting: GlyphHinting,
    ) -> bool {
        let Some(path) = crate::render::color_glyph::outline_gid_path(
            request.font_bytes,
            op.glyph_id,
            request.variation,
        ) else {
            log::warn!(
                "PageRenderer: COLRv1 Porter-Duff paint op missing glyph outline: font='{}' glyph={}",
                request.font_name,
                op.glyph_id
            );
            return false;
        };
        let token = match self.reserve_offscreen_surface(
            buf.width,
            buf.height,
            "COLRv1 Porter-Duff source surface",
        ) {
            Ok(token) => token,
            Err(err) => {
                log::warn!(
                    "PageRenderer: COLRv1 Porter-Duff source surface denied for font='{}' error={}",
                    request.font_name,
                    err
                );
                return true;
            }
        };
        let mut source =
            PixelBuffer::new_transparent_with_mode(buf.width, buf.height, buf.render_mode());
        let saved_clip = source.clip_mask().cloned();
        if !self.install_colr_clips(&mut source, request, &op.clips, glyph_ctm) {
            source.restore_clip(saved_clip);
            drop(token);
            return false;
        }
        let op_ctm = op.transform.concat(glyph_ctm);
        self.paint_colr_paint_to_buffer(&mut source, &path, &op_ctm, glyph_hinting, &op.paint);
        source.restore_clip(saved_clip);
        drop(token);
        composite_colr_porter_duff(buf, &source, op.blend_mode);
        true
    }

    fn paint_colr_paint_to_buffer(
        &self,
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        glyph_hinting: GlyphHinting,
        paint: &crate::render::color_glyph::ColrPaint,
    ) {
        match paint {
            crate::render::color_glyph::ColrPaint::Solid(color) => {
                PathPainter::fill_glyph(
                    buf,
                    path,
                    ctm,
                    &self.viewport,
                    *color,
                    FillRule::NonZero,
                    glyph_hinting,
                );
            }
            crate::render::color_glyph::ColrPaint::LinearGradient { .. }
            | crate::render::color_glyph::ColrPaint::RadialGradient { .. }
            | crate::render::color_glyph::ColrPaint::SweepGradient { .. } => {
                self.fill_colr_gradient_glyph(buf, path, ctm, glyph_hinting, paint);
            }
        }
    }

    fn install_colr_clips(
        &self,
        buf: &mut PixelBuffer,
        request: &GlyphRenderRequest<'_>,
        clips: &[crate::render::color_glyph::ColrClip],
        glyph_ctm: &Transform2D,
    ) -> bool {
        for clip in clips {
            let (path, clip_t) = match clip {
                crate::render::color_glyph::ColrClip::Glyph {
                    glyph_id,
                    transform,
                } => {
                    let Some(path) = crate::render::color_glyph::outline_gid_path(
                        request.font_bytes,
                        *glyph_id,
                        request.variation,
                    ) else {
                        log::warn!(
                            "PageRenderer: COLRv1 clip missing glyph outline: font='{}' glyph={}",
                            request.font_name,
                            glyph_id
                        );
                        return false;
                    };
                    (path, transform.concat(glyph_ctm))
                }
                crate::render::color_glyph::ColrClip::Box {
                    x_min,
                    y_min,
                    x_max,
                    y_max,
                    transform,
                } => {
                    let mut path = Path::new();
                    path.rect(*x_min, *y_min, x_max - x_min, y_max - y_min);
                    (path, transform.concat(glyph_ctm))
                }
            };
            let flat = flatten_path(&path, &clip_t, &self.viewport, 0.2);
            let mask = ClipMask::from_path(&flat, buf.width, buf.height, FillRule::NonZero);
            buf.set_clip(mask);
        }
        true
    }

    fn fill_colr_gradient_glyph(
        &self,
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        glyph_hinting: GlyphHinting,
        paint: &crate::render::color_glyph::ColrPaint,
    ) {
        let device_t = ctm.concat(&self.viewport.to_transform());
        let Some(inverse_device_t) = device_t.inverse() else {
            return;
        };
        let flat = flatten_path(path, ctm, &self.viewport, 0.2);
        let (x0, y0, x1, y1) = path_device_bounds(&flat, buf.width, buf.height);
        if x1 < x0 || y1 < y0 {
            return;
        }
        let mut mask =
            PixelBuffer::new_transparent_with_mode(buf.width, buf.height, buf.render_mode());
        PathPainter::fill_glyph(
            &mut mask,
            path,
            ctm,
            &self.viewport,
            [255, 255, 255, 255],
            FillRule::NonZero,
            glyph_hinting,
        );
        for y in y0..=y1 {
            for x in x0..=x1 {
                let coverage = f32::from(mask.get_pixel(x, y)[3]) / 255.0;
                if coverage <= 0.0 {
                    continue;
                }
                let (gx, gy) = inverse_device_t.transform_point(x as f64 + 0.5, y as f64 + 0.5);
                let color = sample_colr_gradient(paint, gx, gy);
                buf.blend_pixel(x, y, color, coverage);
            }
        }
    }

    fn render_type3_glyph(
        &mut self,
        font_name: &str,
        font_dict: &PdfDictionary,
        glyph: &DecodedGlyph,
    ) -> Option<f64> {
        let fallback_name;
        let glyph_name = if let Some(name) = glyph.glyph_name.as_deref() {
            name
        } else {
            fallback_name = type3_fallback_charproc_name(glyph.unicode)?;
            fallback_name.as_str()
        };
        let text_mode = self.gs.text.rendering_mode;
        let needs_clip = text_rendering_mode_clips(text_mode);
        let paints = text_rendering_mode_paints(text_mode);
        let geometry = self.cached_type3_glyph_geometry(font_name, font_dict, glyph_name);
        if needs_clip && geometry.is_none() {
            log::debug!(
                "PageRenderer: Type3 text clipping requested but charproc '{}' did not yield supported path geometry",
                glyph_name
            );
            self.fail_closed_text_clip();
            return None;
        }

        if paints {
            if let Some(geometry) = geometry.as_ref() {
                let glyph_ctm = self.type3_glyph_ctm(font_dict);
                if needs_clip {
                    self.accumulate_type3_text_clip(geometry.as_ref(), &glyph_ctm);
                }
                self.paint_type3_geometry(geometry.as_ref(), &glyph_ctm);
                return geometry.advance_width.or(glyph.width);
            }

            if let Some(charproc) = self.cached_type3_charproc(font_name, font_dict, glyph_name) {
                if self.paint_cached_type3_rendered_charproc(
                    font_name,
                    font_dict,
                    glyph_name,
                    charproc.as_ref(),
                ) {
                    return charproc.advance_width.or(glyph.width);
                }
                if self.render_type3_charproc_full(font_dict, charproc.as_ref()) {
                    if let Some(geometry) = geometry.as_ref() {
                        self.accumulate_type3_text_clip(
                            geometry.as_ref(),
                            &self.type3_glyph_ctm(font_dict),
                        );
                    }
                    return charproc.advance_width.or(glyph.width);
                }
            }
        }

        let geometry = match geometry {
            Some(geometry) => geometry,
            None => {
                if needs_clip {
                    log::debug!(
                        "PageRenderer: Type3 text clipping requested but charproc '{}' did not yield supported path geometry",
                        glyph_name
                    );
                    self.fail_closed_text_clip();
                }
                return None;
            }
        };
        let glyph_ctm = self.type3_glyph_ctm(font_dict);
        if needs_clip {
            self.accumulate_type3_text_clip(geometry.as_ref(), &glyph_ctm);
        }
        if paints {
            self.paint_type3_geometry(geometry.as_ref(), &glyph_ctm);
        }
        geometry.advance_width
    }

    fn cached_type3_charproc(
        &mut self,
        font_name: &str,
        font_dict: &PdfDictionary,
        glyph_name: &str,
    ) -> Option<Arc<Type3CharProc>> {
        let cache_key = type3_charproc_cache_key(font_name, font_dict, glyph_name);
        if let Some(cached) = self.type3_charproc_cache.get(&cache_key) {
            return cached.clone();
        }
        let charproc = self
            .collect_type3_charproc(font_dict, glyph_name)
            .map(Arc::new);
        self.type3_charproc_cache
            .insert(cache_key, charproc.clone());
        charproc
    }

    fn collect_type3_charproc(
        &self,
        font_dict: &PdfDictionary,
        glyph_name: &str,
    ) -> Option<Type3CharProc> {
        let reader = self.engine.document().reader();
        let stream_obj = resolve_type3_charproc_object(font_dict, glyph_name, reader)?;
        let content = match self.scheduled_decode_stream(
            &stream_obj,
            reader,
            "renderer Type3 charproc recursive render",
        ) {
            Ok(content) => content,
            Err(err) => {
                log::debug!(
                    "PageRenderer: Type3 charproc '{}' decode failed for recursive render: {}",
                    glyph_name,
                    err
                );
                return None;
            }
        };
        if content.len() > TYPE3_MAX_CHARPROC_BYTES {
            log::debug!(
                "PageRenderer: Type3 charproc '{}' exceeded byte cap {} for recursive render",
                glyph_name,
                TYPE3_MAX_CHARPROC_BYTES
            );
            return None;
        }
        let ops = match crate::content::ContentParser::parse(&content) {
            Ok(ops) => ops,
            Err(err) => {
                log::debug!(
                    "PageRenderer: Type3 charproc '{}' parse failed for recursive render: {}",
                    glyph_name,
                    err
                );
                return None;
            }
        };
        if ops.len() > TYPE3_MAX_CHARPROC_OPS {
            log::debug!(
                "PageRenderer: Type3 charproc '{}' exceeded op cap {} for recursive render",
                glyph_name,
                TYPE3_MAX_CHARPROC_OPS
            );
            return None;
        }
        let advance_width = type3_advance_width_from_ops(&ops);
        let glyph_bbox = type3_glyph_bbox_from_ops(&ops);
        Some(Type3CharProc {
            ops,
            advance_width,
            glyph_bbox,
        })
    }

    fn render_type3_charproc_full(
        &mut self,
        font_dict: &PdfDictionary,
        charproc: &Type3CharProc,
    ) -> bool {
        let glyph_ctm = self.type3_glyph_ctm(font_dict);
        self.render_type3_charproc_full_with_ctm(font_dict, charproc, glyph_ctm)
    }

    fn render_type3_charproc_full_with_ctm(
        &mut self,
        font_dict: &PdfDictionary,
        charproc: &Type3CharProc,
        glyph_ctm: Transform2D,
    ) -> bool {
        if self.form_depth >= 8 {
            log::warn!("PageRenderer: Type3 charproc nesting depth limit reached; skipping");
            return false;
        }

        let reader = self.engine.document().reader();
        let type3_resources = font_dict
            .get("Resources")
            .map(|res_obj| {
                let font_res = crate::engine::parse_resources_from_obj(res_obj, reader);
                merge_resources(font_res, &self.resources)
            })
            .unwrap_or_else(|| self.resources.clone());

        let saved_gs = self.gs.clone();
        let saved_resources = self.resources.clone();
        let saved_base_ctm = self.base_ctm;
        let saved_path = std::mem::replace(&mut self.path, Path::new());
        let saved_pending_clip = self.pending_clip.take();
        let saved_pending_text_clip = self.pending_text_clip.take();
        let saved_pending_inline = self.pending_inline.take();
        let saved_clip = self.buf.clip_mask().cloned();
        let saved_smask = self.buf.smask_mask().cloned();
        let saved_clip_stack = self.clip_stack.clone();
        let saved_smask_stack = self.smask_stack.clone();
        let saved_oc_stack = self.oc_visibility_stack.clone();
        let saved_oc_current = self.oc_current_visible;

        self.form_depth += 1;
        self.resources = type3_resources;
        self.gs.ctm = glyph_ctm.to_array();
        self.base_ctm = glyph_ctm;
        self.sync_blend_mode();
        self.dispatch_all(&charproc.ops);

        self.form_depth = self.form_depth.saturating_sub(1);
        self.gs = saved_gs;
        self.resources = saved_resources;
        self.base_ctm = saved_base_ctm;
        self.path = saved_path;
        self.pending_clip = saved_pending_clip;
        self.pending_text_clip = saved_pending_text_clip;
        self.pending_inline = saved_pending_inline;
        self.clip_stack = saved_clip_stack;
        self.smask_stack = saved_smask_stack;
        self.oc_visibility_stack = saved_oc_stack;
        self.oc_current_visible = saved_oc_current;
        self.buf.restore_clip(saved_clip);
        self.buf.restore_smask(saved_smask);
        self.sync_blend_mode();
        !self.cancel.is_cancelled()
    }

    fn paint_cached_type3_rendered_charproc(
        &mut self,
        font_name: &str,
        font_dict: &PdfDictionary,
        glyph_name: &str,
        charproc: &Type3CharProc,
    ) -> bool {
        if self.buf.width == 0
            || self.buf.height == 0
            || (self.buf.width as usize).saturating_mul(self.buf.height as usize) > 4_000_000
        {
            return false;
        }
        let glyph_ctm = self.type3_glyph_ctm(font_dict);
        let Some((_normalized_t, dx, dy, transform_key)) =
            self.type3_mask_transform_parts(&glyph_ctm)
        else {
            return false;
        };
        let device_t = glyph_ctm.concat(&self.viewport.to_transform());
        let Some((full_x0, full_y0, full_x1, full_y1)) = type3_charproc_device_bounds(
            charproc,
            font_dict,
            &device_t,
            self.buf.width,
            self.buf.height,
        ) else {
            return false;
        };
        let Ok(tile_width) = u32::try_from(full_x1.saturating_sub(full_x0).saturating_add(1))
        else {
            return false;
        };
        let Ok(tile_height) = u32::try_from(full_y1.saturating_sub(full_y0).saturating_add(1))
        else {
            return false;
        };
        if tile_width == 0
            || tile_height == 0
            || (tile_width as usize).saturating_mul(tile_height as usize) > 4_000_000
        {
            return false;
        }
        let cache_key = Type3RenderedGlyphCacheKey {
            glyph: type3_charproc_cache_key(font_name, font_dict, glyph_name),
            render_mode: self.gs.text.rendering_mode,
            fill_color: self.fill_pixel_color(),
            stroke_color: self.stroke_pixel_color(),
            a: transform_key[0],
            b: transform_key[1],
            c: transform_key[2],
            d: transform_key[3],
            frac_e: transform_key[4],
            frac_f: transform_key[5],
        };
        if let Some(glyph) = self.type3_rendered_cache.get(&cache_key) {
            glyph.paint(&mut self.buf, dx, dy);
            return true;
        }

        let original_viewport = self.viewport.clone();
        let render_mode = self.buf.render_mode();
        let (Ok(tile_x), Ok(tile_y)) = (u32::try_from(full_x0), u32::try_from(full_y0)) else {
            return false;
        };
        let tile_viewport = original_viewport.pixel_window(tile_x, tile_y, tile_width, tile_height);
        let original = std::mem::replace(
            &mut self.buf,
            PixelBuffer::new_transparent_with_mode(tile_width, tile_height, render_mode),
        );
        self.viewport = tile_viewport;
        let ok = self.render_type3_charproc_full_with_ctm(font_dict, charproc, glyph_ctm);
        self.viewport = original_viewport;
        let rendered = std::mem::replace(&mut self.buf, original);
        if !ok || self.cancel.is_cancelled() {
            return false;
        }
        let Some(glyph) = Type3RenderedGlyph::from_buffer_with_origin(
            &rendered,
            (
                0,
                0,
                tile_width.saturating_sub(1),
                tile_height.saturating_sub(1),
            ),
            full_x0,
            full_y0,
            dx,
            dy,
        )
        .map(Arc::new) else {
            return false;
        };
        self.type3_rendered_cache.insert(cache_key, glyph.clone());
        glyph.paint(&mut self.buf, dx, dy);
        true
    }

    fn cached_type3_glyph_geometry(
        &mut self,
        font_name: &str,
        font_dict: &PdfDictionary,
        glyph_name: &str,
    ) -> Option<Arc<Type3GlyphGeometry>> {
        let cache_key = type3_charproc_cache_key(font_name, font_dict, glyph_name);
        if let Some(cached) = self.type3_geometry_cache.get(&cache_key) {
            return cached.clone();
        }
        let geometry = self
            .collect_type3_glyph_geometry(font_dict, glyph_name, &cache_key)
            .map(Arc::new);
        self.type3_geometry_cache
            .insert(cache_key, geometry.clone());
        geometry
    }

    fn collect_type3_glyph_geometry(
        &self,
        font_dict: &PdfDictionary,
        glyph_name: &str,
        cache_key: &str,
    ) -> Option<Type3GlyphGeometry> {
        let reader = self.engine.document().reader();
        let stream_obj = resolve_type3_charproc_object(font_dict, glyph_name, reader)?;
        let content = match self.scheduled_decode_stream(
            &stream_obj,
            reader,
            "renderer Type3 charproc clip extraction",
        ) {
            Ok(content) => content,
            Err(err) => {
                log::debug!(
                    "PageRenderer: Type3 charproc '{}' decode failed: {}",
                    glyph_name,
                    err
                );
                return None;
            }
        };
        if content.len() > TYPE3_MAX_CHARPROC_BYTES {
            log::debug!(
                "PageRenderer: Type3 charproc '{}' exceeded byte cap {}",
                glyph_name,
                TYPE3_MAX_CHARPROC_BYTES
            );
            return None;
        }
        let ops = match crate::content::ContentParser::parse(&content) {
            Ok(ops) => ops,
            Err(err) => {
                log::debug!(
                    "PageRenderer: Type3 charproc '{}' parse failed: {}",
                    glyph_name,
                    err
                );
                return None;
            }
        };
        Type3PathCollector::collect(glyph_name, &ops, cache_key)
    }

    fn type3_glyph_ctm(&self, font_dict: &PdfDictionary) -> Transform2D {
        let font_matrix = type3_font_matrix(font_dict);
        let th = self.gs.text.horizontal_scaling / 100.0;
        let scale_t = Transform2D::scale(self.gs.text.font_size * th, self.gs.text.font_size);
        let rise_t = Transform2D::translation(0.0, self.gs.text.rise);
        let tm_t = Transform2D::from(self.gs.text.tm);
        font_matrix
            .concat(&scale_t)
            .concat(&rise_t)
            .concat(&tm_t)
            .concat(&self.ctm())
    }

    fn accumulate_type3_text_clip(
        &mut self,
        geometry: &Type3GlyphGeometry,
        glyph_ctm: &Transform2D,
    ) {
        for (idx, fill) in geometry.fills.iter().enumerate() {
            if !self.accumulate_cached_type3_fill_clip(geometry, idx, fill, glyph_ctm) {
                let flat = flatten_path(&fill.path, glyph_ctm, &self.viewport, 0.25);
                let clip = ClipMask::from_path(&flat, self.buf.width, self.buf.height, fill.rule);
                self.accumulate_text_clip_mask(clip);
            }
        }
        for stroke in &geometry.strokes {
            let flat = flatten_path(&stroke.path, glyph_ctm, &self.viewport, 0.25);
            let width_px = (stroke.width * glyph_ctm.scale_factor() * self.viewport.scale).max(1.0);
            let outline = stroke_flat_path(
                &flat,
                width_px,
                &stroke.dash,
                stroke.cap.clone(),
                stroke.join.clone(),
                stroke.miter_limit,
            );
            if !outline.subpaths.is_empty() {
                let clip = ClipMask::from_path(
                    &outline,
                    self.buf.width,
                    self.buf.height,
                    FillRule::NonZero,
                );
                self.accumulate_text_clip_mask(clip);
            }
        }
    }

    fn accumulate_cached_type3_fill_clip(
        &mut self,
        geometry: &Type3GlyphGeometry,
        fill_index: usize,
        fill: &Type3Fill,
        glyph_ctm: &Transform2D,
    ) -> bool {
        let Some((mask, dx, dy)) =
            self.cached_type3_fill_mask(geometry, fill_index, fill, glyph_ctm)
        else {
            return false;
        };
        if self.pending_text_clip.is_none() {
            self.pending_text_clip = Some(ClipMask::empty(self.buf.width, self.buf.height));
        }
        if let Some(clip) = &mut self.pending_text_clip {
            mask.union_into_clip_mask(clip, dx, dy);
        }
        true
    }

    fn paint_type3_geometry(&mut self, geometry: &Type3GlyphGeometry, glyph_ctm: &Transform2D) {
        let fill_color = self.fill_pixel_color();
        let stroke_color = self.stroke_pixel_color();
        let fill_mode = matches!(self.gs.text.rendering_mode, 0 | 2 | 4 | 6);
        let stroke_mode = matches!(self.gs.text.rendering_mode, 1 | 2 | 5 | 6);
        if fill_mode {
            let fill_color_state = self.gs.fill_color.clone();
            self.record_plate_contribution(
                &fill_color_state,
                self.gs.fill_alpha as f32,
                "text_type3_fill",
            );
        }
        if stroke_mode {
            let stroke_color_state = self.gs.stroke_color.clone();
            self.record_plate_contribution(
                &stroke_color_state,
                self.gs.stroke_alpha as f32,
                "text_type3_stroke",
            );
        }
        match self.gs.text.rendering_mode {
            0 | 4 => {
                if let Some((mask, dx, dy, color)) =
                    self.cached_type3_composite_fill_mask(geometry, glyph_ctm, fill_color)
                {
                    mask.paint(&mut self.buf, dx, dy, color);
                    return;
                }
                for (idx, fill) in geometry.fills.iter().enumerate() {
                    let fill_color = type3_color_with_alpha(
                        fill.color.unwrap_or(fill_color),
                        self.gs.fill_alpha,
                    );
                    if !self.paint_type3_rect_fill(fill, glyph_ctm, fill_color)
                        && !self.paint_cached_type3_fill(geometry, idx, fill, glyph_ctm, fill_color)
                        && !PathPainter::fill_fast_cancellable(
                            &mut self.buf,
                            &fill.path,
                            glyph_ctm,
                            &self.viewport,
                            fill_color,
                            fill.rule,
                            &self.cancel,
                        )
                    {
                        return;
                    }
                }
            }
            1 | 5 => {
                for fill in &geometry.fills {
                    if !PathPainter::stroke_with_style_fast_cancellable(
                        &mut self.buf,
                        &fill.path,
                        glyph_ctm,
                        &self.viewport,
                        stroke_color,
                        self.gs.line_width,
                        &DashState::solid(),
                        &self.gs.line_cap,
                        &self.gs.line_join,
                        self.gs.miter_limit,
                        &self.cancel,
                    ) {
                        return;
                    }
                }
                self.paint_type3_strokes(geometry, glyph_ctm, stroke_color);
            }
            2 | 6 => {
                for (idx, fill) in geometry.fills.iter().enumerate() {
                    let fill_color = type3_color_with_alpha(
                        fill.color.unwrap_or(fill_color),
                        self.gs.fill_alpha,
                    );
                    if !self.paint_type3_rect_fill(fill, glyph_ctm, fill_color)
                        && !self.paint_cached_type3_fill(geometry, idx, fill, glyph_ctm, fill_color)
                        && !PathPainter::fill_fast_cancellable(
                            &mut self.buf,
                            &fill.path,
                            glyph_ctm,
                            &self.viewport,
                            fill_color,
                            fill.rule,
                            &self.cancel,
                        )
                    {
                        return;
                    }
                    if !PathPainter::stroke_with_style_fast_cancellable(
                        &mut self.buf,
                        &fill.path,
                        glyph_ctm,
                        &self.viewport,
                        stroke_color,
                        self.gs.line_width,
                        &DashState::solid(),
                        &self.gs.line_cap,
                        &self.gs.line_join,
                        self.gs.miter_limit,
                        &self.cancel,
                    ) {
                        return;
                    }
                }
                self.paint_type3_strokes(geometry, glyph_ctm, stroke_color);
            }
            3 | 7 => {}
            _ => {}
        }
    }

    fn paint_type3_rect_fill(
        &mut self,
        fill: &Type3Fill,
        glyph_ctm: &Transform2D,
        fill_color: PixelColor,
    ) -> bool {
        if fill.rule != FillRule::NonZero {
            return false;
        }
        let Some((x, y, w, h)) = axis_aligned_integer_rect(&fill.path, glyph_ctm, &self.viewport)
        else {
            return false;
        };
        self.buf.fill_rect(x, y, w, h, fill_color);
        true
    }

    fn paint_cached_type3_fill(
        &mut self,
        geometry: &Type3GlyphGeometry,
        fill_index: usize,
        fill: &Type3Fill,
        glyph_ctm: &Transform2D,
        fill_color: PixelColor,
    ) -> bool {
        let Some((mask, dx, dy)) =
            self.cached_type3_fill_mask(geometry, fill_index, fill, glyph_ctm)
        else {
            return false;
        };
        mask.paint(&mut self.buf, dx, dy, fill_color);
        true
    }

    fn cached_type3_fill_mask(
        &mut self,
        geometry: &Type3GlyphGeometry,
        fill_index: usize,
        fill: &Type3Fill,
        glyph_ctm: &Transform2D,
    ) -> Option<(Arc<RasterizedGlyphMask>, i32, i32)> {
        let (normalized_t, dx, dy, transform_key) = self.type3_mask_transform_parts(glyph_ctm)?;
        self.cached_type3_fill_mask_with_transform(
            geometry,
            fill_index,
            fill,
            &normalized_t,
            dx,
            dy,
            transform_key,
        )
    }

    fn cached_type3_composite_fill_mask(
        &mut self,
        geometry: &Type3GlyphGeometry,
        glyph_ctm: &Transform2D,
        fallback_fill_color: PixelColor,
    ) -> Option<(Arc<RasterizedGlyphMask>, i32, i32, PixelColor)> {
        if geometry.fills.len() <= 1 || !geometry.strokes.is_empty() || self.gs.fill_alpha < 0.999 {
            return None;
        }
        let color = type3_color_with_alpha(
            geometry.fills.first()?.color.unwrap_or(fallback_fill_color),
            self.gs.fill_alpha,
        );
        if geometry.fills.iter().any(|fill| {
            type3_color_with_alpha(
                fill.color.unwrap_or(fallback_fill_color),
                self.gs.fill_alpha,
            ) != color
        }) {
            return None;
        }

        let (normalized_t, dx, dy, transform_key) = self.type3_mask_transform_parts(glyph_ctm)?;
        let key = Type3MaskCacheKey {
            glyph: geometry.cache_key.clone(),
            fill_index: u16::MAX,
            fill_rule: 255,
            a: transform_key[0],
            b: transform_key[1],
            c: transform_key[2],
            d: transform_key[3],
            frac_e: transform_key[4],
            frac_f: transform_key[5],
        };
        if let Some(mask) = self.type3_mask_cache.get(&key) {
            return Some((mask, dx, dy, color));
        }

        let mut parts = Vec::with_capacity(geometry.fills.len());
        for (idx, fill) in geometry.fills.iter().enumerate() {
            let (mask, _, _) = self.cached_type3_fill_mask_with_transform(
                geometry,
                idx,
                fill,
                &normalized_t,
                dx,
                dy,
                transform_key,
            )?;
            parts.push(mask);
        }
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for mask in &parts {
            min_x = min_x.min(mask.x);
            min_y = min_y.min(mask.y);
            max_x = max_x.max(mask.x.saturating_add(mask.width as i32));
            max_y = max_y.max(mask.y.saturating_add(mask.height as i32));
        }
        if max_x <= min_x || max_y <= min_y {
            return None;
        }
        let width = (max_x - min_x) as u32;
        let height = (max_y - min_y) as u32;
        let mut alpha = vec![0u8; width as usize * height as usize];
        for mask in &parts {
            for row in 0..mask.height as usize {
                let src_base = row * mask.width as usize;
                let dst_y = (mask.y - min_y) as usize + row;
                let dst_base = dst_y * width as usize + (mask.x - min_x) as usize;
                for col in 0..mask.width as usize {
                    let value = mask.alpha_slice()[src_base + col];
                    let dst = &mut alpha[dst_base + col];
                    if value > *dst {
                        *dst = value;
                    }
                }
            }
        }
        let mask = Arc::new(RasterizedGlyphMask::from_alpha(
            min_x, min_y, width, height, alpha,
        )?);
        self.type3_mask_cache.insert(key, mask.clone());
        Some((mask, dx, dy, color))
    }

    fn type3_mask_transform_parts(
        &self,
        glyph_ctm: &Transform2D,
    ) -> Option<(Transform2D, i32, i32, [i64; 6])> {
        let device_t = glyph_ctm.concat(&self.viewport.to_transform());
        if [
            device_t.a, device_t.b, device_t.c, device_t.d, device_t.e, device_t.f,
        ]
        .iter()
        .any(|value| !value.is_finite())
        {
            return None;
        }
        if device_t.scale_factor() <= 0.0 || device_t.scale_factor() > 256.0 {
            return None;
        }
        let origin_x = device_t.e.floor();
        let origin_y = device_t.f.floor();
        let normalized_t = Transform2D {
            e: device_t.e - origin_x,
            f: device_t.f - origin_y,
            ..device_t
        };
        let dx = if origin_x <= i32::MIN as f64 {
            i32::MIN
        } else if origin_x >= i32::MAX as f64 {
            i32::MAX
        } else {
            origin_x as i32
        };
        let dy = if origin_y <= i32::MIN as f64 {
            i32::MIN
        } else if origin_y >= i32::MAX as f64 {
            i32::MAX
        } else {
            origin_y as i32
        };
        Some((
            normalized_t,
            dx,
            dy,
            [
                quantize_glyph_mask_value(normalized_t.a),
                quantize_glyph_mask_value(normalized_t.b),
                quantize_glyph_mask_value(normalized_t.c),
                quantize_glyph_mask_value(normalized_t.d),
                quantize_glyph_mask_fraction(normalized_t.e),
                quantize_glyph_mask_fraction(normalized_t.f),
            ],
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn cached_type3_fill_mask_with_transform(
        &mut self,
        geometry: &Type3GlyphGeometry,
        fill_index: usize,
        fill: &Type3Fill,
        normalized_t: &Transform2D,
        dx: i32,
        dy: i32,
        transform_key: [i64; 6],
    ) -> Option<(Arc<RasterizedGlyphMask>, i32, i32)> {
        if fill_index > u16::MAX as usize {
            return None;
        }
        let key = Type3MaskCacheKey {
            glyph: geometry.cache_key.clone(),
            fill_index: fill_index as u16,
            fill_rule: match fill.rule {
                FillRule::NonZero => 0,
                FillRule::EvenOdd => 1,
            },
            a: transform_key[0],
            b: transform_key[1],
            c: transform_key[2],
            d: transform_key[3],
            frac_e: transform_key[4],
            frac_f: transform_key[5],
        };
        if let Some(mask) = self.type3_mask_cache.get(&key) {
            return Some((mask, dx, dy));
        }
        let mask = rasterize_glyph_alpha_mask(
            &fill.path,
            normalized_t,
            fill.rule,
            GlyphHinting::disabled(),
        )?;
        let mask = Arc::new(mask);
        self.type3_mask_cache.insert(key, mask.clone());
        Some((mask, dx, dy))
    }

    fn paint_type3_strokes(
        &mut self,
        geometry: &Type3GlyphGeometry,
        glyph_ctm: &Transform2D,
        stroke_color: PixelColor,
    ) {
        for stroke in &geometry.strokes {
            let stroke_color =
                type3_color_with_alpha(stroke.color.unwrap_or(stroke_color), self.gs.stroke_alpha);
            if !PathPainter::stroke_with_style_fast_cancellable(
                &mut self.buf,
                &stroke.path,
                glyph_ctm,
                &self.viewport,
                stroke_color,
                stroke.width,
                &stroke.dash,
                &stroke.cap,
                &stroke.join,
                stroke.miter_limit,
                &self.cancel,
            ) {
                return;
            }
        }
    }

    #[cfg(test)]
    fn extract_glyph_path(font_bytes: &[u8], ch: char) -> (Option<Path>, f64) {
        crate::render::glyph_outline::extract_glyph_path_for_simple(
            font_bytes,
            glyph_cache_code(ch),
            ch,
            None,
        )
    }

    fn get_upem(font_bytes: &[u8]) -> Option<u16> {
        if let Ok(face) = ttf_parser::Face::parse(font_bytes, 0) {
            return Some(face.units_per_em());
        }
        // Bare CFF reports a 1000-unit em (FontMatrix 0.001 convention).
        if crate::render::font_rasterizer::cff_support::is_bare_cff(font_bytes) {
            return Some(crate::render::font_rasterizer::cff_support::units_per_em() as u16);
        }
        if crate::fonts::type1::Type1Font::is_type1(font_bytes) {
            return Some(crate::fonts::type1::units_per_em() as u16);
        }
        None
    }

    /// Build the variable-font [`VariationRequest`] for a font resource from its
    /// `FontDescriptor` (`/FontWeight` â†’ `wght`, `/FontStretch` â†’ `wdth`). Returns
    /// the empty request (default instance) when there is no descriptor or no
    /// non-normal weight/stretch â€” so static fonts and default-instance variable
    /// fonts keep the byte-identical pre-variation cache key and outline.
    fn font_variation_request_from_dict(&self, font_dict: &PdfDictionary) -> VariationRequest {
        let reader = self.engine.document().reader();
        // For Type0 fonts the descriptor lives on the descendant CIDFont.
        let descriptor = if detect_font_subtype(font_dict) == FontSubtype::Type0 {
            get_descendant_font(font_dict, reader).and_then(|d| resolve_descriptor(&d, reader))
        } else {
            resolve_descriptor(font_dict, reader)
        };
        let Some(descriptor) = descriptor else {
            return VariationRequest::none();
        };
        let weight = descriptor.get("FontWeight").and_then(PdfObject::as_number);
        let stretch = descriptor.get_name("FontStretch");
        VariationRequest::from_descriptor(weight, stretch)
    }

    fn get_font_bytes(&mut self, font_name: &str, cache_key: &str) -> Option<Arc<Vec<u8>>> {
        if let Some(cached) = self.font_bytes_cache.get(cache_key) {
            self.font_bytes_cache_stats.hits = self.font_bytes_cache_stats.hits.saturating_add(1);
            return cached.clone();
        }
        self.font_bytes_cache_stats.misses = self.font_bytes_cache_stats.misses.saturating_add(1);
        let resolved = self.resolve_font_bytes(font_name).map(Arc::new);
        self.font_bytes_cache
            .insert(cache_key.to_string(), resolved.clone());
        resolved
    }

    fn get_font_resolver(
        &mut self,
        cache_key: &str,
        font_dict: &PdfDictionary,
    ) -> Arc<FontResolver> {
        if let Some(cached) = self.font_resolver_cache.get(cache_key) {
            self.font_resolver_cache_stats.hits =
                self.font_resolver_cache_stats.hits.saturating_add(1);
            return Arc::clone(cached);
        }
        self.font_resolver_cache_stats.misses =
            self.font_resolver_cache_stats.misses.saturating_add(1);
        let resolver = Arc::new(FontResolver::new(
            font_dict,
            self.engine.document().reader(),
        ));
        self.font_resolver_cache
            .insert(cache_key.to_string(), Arc::clone(&resolver));
        resolver
    }

    fn resolve_font_bytes(&self, font_name: &str) -> Option<Vec<u8>> {
        let reader = self.engine.document().reader();
        if let Some(font_dict) = self.resources.fonts.get(font_name) {
            if let Some(bytes) = FontRasterizer::extract_font_bytes(font_dict, reader) {
                if !bytes.is_empty() {
                    return Some(bytes);
                }
            }
            if detect_font_subtype(font_dict) == FontSubtype::Type0 {
                if let Some(descendant_font) = get_descendant_font(font_dict, reader) {
                    if let Some(bytes) =
                        FontRasterizer::extract_font_bytes(&descendant_font, reader)
                    {
                        if !bytes.is_empty() {
                            return Some(bytes);
                        }
                    }
                }
            }
            let fallback_name = fallback_font_lookup_name(font_name, font_dict, reader);
            if let Some(bytes) = get_fallback_font(&fallback_name) {
                return Some(bytes.to_vec());
            }
        }
        get_fallback_font(font_name).map(|bytes| bytes.to_vec())
    }

    fn advance_text(&mut self, glyph_width: f64, is_space: bool) {
        let th = self.gs.text.horizontal_scaling / 100.0;
        let mut advance =
            (glyph_width / 1000.0) * self.gs.text.font_size * th + self.gs.text.char_spacing * th;
        if is_space {
            advance += self.gs.text.word_spacing * th;
        }
        self.translate_text_matrix(advance, 0.0);
    }

    fn advance_decoded_text(&mut self, glyph_width: f64, glyph: &DecodedGlyph) {
        if !glyph.is_vertical {
            self.advance_text(glyph_width, glyph.is_space);
            return;
        }
        let mut advance_y =
            glyph.vertical_advance.unwrap_or(-1000.0) / 1000.0 * self.gs.text.font_size;
        let spacing = self.gs.text.char_spacing
            + if glyph.is_space {
                self.gs.text.word_spacing
            } else {
                0.0
            };
        if spacing != 0.0 {
            let sign = if advance_y < 0.0 { -1.0 } else { 1.0 };
            advance_y += spacing * sign;
        }
        self.translate_text_matrix(0.0, advance_y);
    }

    fn adjust_text_position(&mut self, adjustment: f64) {
        let tx = adjustment / 1000.0
            * self.gs.text.font_size
            * (self.gs.text.horizontal_scaling / 100.0);
        self.translate_text_matrix(tx, 0.0);
    }

    fn move_to_next_text_line(&mut self) {
        let op = ContentOperation::new("T*", Vec::new());
        self.gs.process(&op);
    }

    fn translate_text_matrix(&mut self, tx: f64, ty: f64) {
        let mut tm = self.gs.text.tm;
        tm[4] += tm[0] * tx + tm[2] * ty;
        tm[5] += tm[1] * tx + tm[3] * ty;
        self.gs.text.tm = tm;
    }
}

struct GlyphRenderRequest<'a> {
    font_name: &'a str,
    font_subtype: FontSubtype,
    code: u16,
    ch: char,
    glyph_name: Option<&'a str>,
    is_gid: bool,
    font_bytes: &'a [u8],
    font_hash: u64,
    /// The variable-font instance to render (empty for static / default).
    variation: &'a VariationRequest,
    upem: f64,
    light_hinting_supported: bool,
    offset_x: f64,
    offset_y: f64,
}

const TYPE3_MAX_CHARPROC_BYTES: usize = 1_048_576;
const TYPE3_MAX_CHARPROC_OPS: usize = 4096;
const TYPE3_MAX_PATH_SEGMENTS: usize = 8192;
const TYPE3_MAX_Q_DEPTH: usize = 32;

#[derive(Clone)]
struct Type3Fill {
    path: Path,
    rule: FillRule,
    color: Option<PixelColor>,
}

#[derive(Clone)]
struct Type3Stroke {
    path: Path,
    width: f64,
    dash: DashState,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f64,
    color: Option<PixelColor>,
}

struct Type3GlyphGeometry {
    cache_key: String,
    fills: Vec<Type3Fill>,
    strokes: Vec<Type3Stroke>,
    advance_width: Option<f64>,
    _uses_color_state: bool,
}

struct Type3CharProc {
    ops: Vec<ContentOperation>,
    advance_width: Option<f64>,
    glyph_bbox: Option<[f64; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Type3DeviceColorSpace {
    Gray,
    Rgb,
    Cmyk,
    Unsupported,
}

#[derive(Clone)]
struct Type3CollectorState {
    ctm: Transform2D,
    line_width: f64,
    dash: DashState,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f64,
    fill_color: PixelColor,
    stroke_color: PixelColor,
    fill_color_space: Type3DeviceColorSpace,
    stroke_color_space: Type3DeviceColorSpace,
}

struct Type3PathCollector {
    glyph_name: String,
    path: Path,
    fills: Vec<Type3Fill>,
    strokes: Vec<Type3Stroke>,
    state: Type3CollectorState,
    stack: Vec<Type3CollectorState>,
    advance_width: Option<f64>,
    uses_color_state: bool,
    unsupported: Option<String>,
}

impl Type3PathCollector {
    fn collect(
        glyph_name: &str,
        ops: &[ContentOperation],
        cache_key: &str,
    ) -> Option<Type3GlyphGeometry> {
        if ops.len() > TYPE3_MAX_CHARPROC_OPS {
            log::debug!(
                "PageRenderer: Type3 charproc '{}' exceeded op cap {}",
                glyph_name,
                TYPE3_MAX_CHARPROC_OPS
            );
            return None;
        }

        let mut collector = Self {
            glyph_name: glyph_name.to_string(),
            path: Path::new(),
            fills: Vec::new(),
            strokes: Vec::new(),
            state: Type3CollectorState {
                ctm: Transform2D::identity(),
                line_width: 1.0,
                dash: DashState::solid(),
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                miter_limit: 10.0,
                fill_color: BLACK,
                stroke_color: BLACK,
                fill_color_space: Type3DeviceColorSpace::Gray,
                stroke_color_space: Type3DeviceColorSpace::Gray,
            },
            stack: Vec::new(),
            advance_width: None,
            uses_color_state: false,
            unsupported: None,
        };

        for op in ops {
            collector.dispatch(op);
            if collector.unsupported.is_some() || collector.path_too_large() {
                break;
            }
        }

        if let Some(reason) = collector.unsupported {
            log::debug!(
                "PageRenderer: Type3 charproc '{}' unsupported for clipping: {}",
                collector.glyph_name,
                reason
            );
            return None;
        }
        if collector.fills.is_empty() && collector.strokes.is_empty() {
            return None;
        }
        Some(Type3GlyphGeometry {
            cache_key: cache_key.to_string(),
            fills: collector.fills,
            strokes: collector.strokes,
            advance_width: collector.advance_width,
            _uses_color_state: collector.uses_color_state,
        })
    }

    fn dispatch(&mut self, op: &ContentOperation) {
        match op.operator.as_str() {
            "m" => {
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    let (x, y) = self.state.ctm.transform_point(x, y);
                    self.path.move_to(x, y);
                }
            }
            "l" => {
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    let (x, y) = self.state.ctm.transform_point(x, y);
                    self.path.line_to(x, y);
                }
            }
            "c" => {
                if let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x3), Some(y3)) = (
                    op.number(0),
                    op.number(1),
                    op.number(2),
                    op.number(3),
                    op.number(4),
                    op.number(5),
                ) {
                    let (x1, y1) = self.state.ctm.transform_point(x1, y1);
                    let (x2, y2) = self.state.ctm.transform_point(x2, y2);
                    let (x3, y3) = self.state.ctm.transform_point(x3, y3);
                    self.path.curve_to(x1, y1, x2, y2, x3, y3);
                }
            }
            "v" => {
                if let (Some(x2), Some(y2), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    let (cx, cy) = self.path.current_point.unwrap_or((x2, y2));
                    let (x2, y2) = self.state.ctm.transform_point(x2, y2);
                    let (x3, y3) = self.state.ctm.transform_point(x3, y3);
                    self.path.curve_to(cx, cy, x2, y2, x3, y3);
                }
            }
            "y" => {
                if let (Some(x1), Some(y1), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    let (x1, y1) = self.state.ctm.transform_point(x1, y1);
                    let (x3, y3) = self.state.ctm.transform_point(x3, y3);
                    self.path.curve_to(x1, y1, x3, y3, x3, y3);
                }
            }
            "h" => self.path.close(),
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.transformed_rect(x, y, w, h);
                }
            }
            "f" | "F" => self.collect_fill(FillRule::NonZero),
            "f*" => self.collect_fill(FillRule::EvenOdd),
            "S" => self.collect_stroke(),
            "s" => {
                self.path.close();
                self.collect_stroke();
            }
            "B" => self.collect_fill_stroke(FillRule::NonZero),
            "B*" => self.collect_fill_stroke(FillRule::EvenOdd),
            "b" => {
                self.path.close();
                self.collect_fill_stroke(FillRule::NonZero);
            }
            "b*" => {
                self.path.close();
                self.collect_fill_stroke(FillRule::EvenOdd);
            }
            "n" => self.path.clear(),
            "cm" => self.op_cm(op),
            "q" => {
                if self.stack.len() >= TYPE3_MAX_Q_DEPTH {
                    self.unsupported = Some("graphics state depth cap reached".to_string());
                } else {
                    self.stack.push(self.state.clone());
                }
            }
            "Q" => {
                if let Some(state) = self.stack.pop() {
                    self.state = state;
                }
            }
            "w" => {
                self.state.line_width = op.number(0).unwrap_or(1.0).max(0.0);
            }
            "J" => {
                self.state.cap = match op.number(0).map(|n| n as i32).unwrap_or(0) {
                    1 => LineCap::Round,
                    2 => LineCap::ProjectingSquare,
                    _ => LineCap::Butt,
                };
            }
            "j" => {
                self.state.join = match op.number(0).map(|n| n as i32).unwrap_or(0) {
                    1 => LineJoin::Round,
                    2 => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                };
            }
            "M" => {
                self.state.miter_limit = op.number(0).unwrap_or(10.0).max(1.0);
            }
            "d" => self.op_dash(op),
            "d0" | "d1" => {
                self.advance_width = op
                    .number(0)
                    .filter(|value| value.is_finite() && *value > 0.0);
            }
            "g" => {
                if let Some(gray) = op.number(0) {
                    self.state.fill_color = type3_gray_color(gray);
                    self.state.fill_color_space = Type3DeviceColorSpace::Gray;
                    self.uses_color_state = true;
                }
            }
            "G" => {
                if let Some(gray) = op.number(0) {
                    self.state.stroke_color = type3_gray_color(gray);
                    self.state.stroke_color_space = Type3DeviceColorSpace::Gray;
                    self.uses_color_state = true;
                }
            }
            "rg" => {
                if let (Some(r), Some(g), Some(b)) = (op.number(0), op.number(1), op.number(2)) {
                    self.state.fill_color = type3_rgb_color(r, g, b);
                    self.state.fill_color_space = Type3DeviceColorSpace::Rgb;
                    self.uses_color_state = true;
                }
            }
            "RG" => {
                if let (Some(r), Some(g), Some(b)) = (op.number(0), op.number(1), op.number(2)) {
                    self.state.stroke_color = type3_rgb_color(r, g, b);
                    self.state.stroke_color_space = Type3DeviceColorSpace::Rgb;
                    self.uses_color_state = true;
                }
            }
            "k" => {
                if let (Some(c), Some(m), Some(y), Some(k)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.state.fill_color = type3_cmyk_color(c, m, y, k);
                    self.state.fill_color_space = Type3DeviceColorSpace::Cmyk;
                    self.uses_color_state = true;
                }
            }
            "K" => {
                if let (Some(c), Some(m), Some(y), Some(k)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.state.stroke_color = type3_cmyk_color(c, m, y, k);
                    self.state.stroke_color_space = Type3DeviceColorSpace::Cmyk;
                    self.uses_color_state = true;
                }
            }
            "ri" | "i" => {
                self.uses_color_state = true;
            }
            "cs" => self.set_fill_color_space(op),
            "CS" => self.set_stroke_color_space(op),
            "sc" | "scn" => self.set_fill_color_from_space(op),
            "SC" | "SCN" => self.set_stroke_color_from_space(op),
            // Resource-heavy glyphs need a real recursive Type3 interpreter,
            // not a bounding-box fallback.
            "Do" | "sh" | "BI" | "ID" | "inline_image_data" | "Tj" | "TJ" | "'" | "\"" | "BT"
            | "ET" | "W" | "W*" | "gs" => {
                self.unsupported = Some(format!("operator '{}' is not path-only", op.operator));
            }
            _ => {}
        }
    }

    fn op_cm(&mut self, op: &ContentOperation) {
        let m = Transform2D::from([
            op.number(0).unwrap_or(1.0),
            op.number(1).unwrap_or(0.0),
            op.number(2).unwrap_or(0.0),
            op.number(3).unwrap_or(1.0),
            op.number(4).unwrap_or(0.0),
            op.number(5).unwrap_or(0.0),
        ]);
        self.state.ctm = m.concat(&self.state.ctm);
    }

    fn op_dash(&mut self, op: &ContentOperation) {
        let pattern = op
            .operand(0)
            .and_then(Operand::as_array)
            .map(|items| items.iter().filter_map(Operand::as_number).collect())
            .unwrap_or_default();
        let phase = op.number(1).unwrap_or(0.0);
        self.state.dash = DashState::new(pattern, phase);
    }

    fn set_fill_color_space(&mut self, op: &ContentOperation) {
        self.state.fill_color_space = type3_device_color_space(op.name(0));
        self.uses_color_state = true;
    }

    fn set_stroke_color_space(&mut self, op: &ContentOperation) {
        self.state.stroke_color_space = type3_device_color_space(op.name(0));
        self.uses_color_state = true;
    }

    fn set_fill_color_from_space(&mut self, op: &ContentOperation) {
        match type3_color_from_space(self.state.fill_color_space, op) {
            Some(color) => {
                self.state.fill_color = color;
                self.uses_color_state = true;
            }
            None => {
                self.unsupported = Some(format!(
                    "operator '{}' needs unsupported fill color space",
                    op.operator
                ));
            }
        }
    }

    fn set_stroke_color_from_space(&mut self, op: &ContentOperation) {
        match type3_color_from_space(self.state.stroke_color_space, op) {
            Some(color) => {
                self.state.stroke_color = color;
                self.uses_color_state = true;
            }
            None => {
                self.unsupported = Some(format!(
                    "operator '{}' needs unsupported stroke color space",
                    op.operator
                ));
            }
        }
    }

    fn transformed_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let points = [(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
        let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] =
            points.map(|(px, py)| self.state.ctm.transform_point(px, py));
        self.path.move_to(x0, y0);
        self.path.line_to(x1, y1);
        self.path.line_to(x2, y2);
        self.path.line_to(x3, y3);
        self.path.close();
    }

    fn collect_fill_stroke(&mut self, rule: FillRule) {
        let path = self.path.clone();
        self.collect_fill(rule);
        if !path.is_empty() {
            self.strokes.push(Type3Stroke {
                path,
                width: self.effective_line_width(),
                dash: self.state.dash.clone(),
                cap: self.state.cap.clone(),
                join: self.state.join.clone(),
                miter_limit: self.state.miter_limit,
                color: Some(self.state.stroke_color),
            });
        }
    }

    fn collect_fill(&mut self, rule: FillRule) {
        if !self.path.is_empty() {
            self.fills.push(Type3Fill {
                path: self.path.clone(),
                rule,
                color: Some(self.state.fill_color),
            });
        }
        self.path.clear();
    }

    fn collect_stroke(&mut self) {
        if !self.path.is_empty() {
            self.strokes.push(Type3Stroke {
                path: self.path.clone(),
                width: self.effective_line_width(),
                dash: self.state.dash.clone(),
                cap: self.state.cap.clone(),
                join: self.state.join.clone(),
                miter_limit: self.state.miter_limit,
                color: Some(self.state.stroke_color),
            });
        }
        self.path.clear();
    }

    fn effective_line_width(&self) -> f64 {
        (self.state.line_width * self.state.ctm.scale_factor()).max(0.0)
    }

    fn path_too_large(&mut self) -> bool {
        let segments = self.path.segments.len()
            + self
                .fills
                .iter()
                .map(|fill| fill.path.segments.len())
                .sum::<usize>()
            + self
                .strokes
                .iter()
                .map(|stroke| stroke.path.segments.len())
                .sum::<usize>();
        if segments > TYPE3_MAX_PATH_SEGMENTS {
            self.unsupported = Some(format!(
                "path segment cap {} exceeded",
                TYPE3_MAX_PATH_SEGMENTS
            ));
            true
        } else {
            false
        }
    }
}

fn type3_fallback_charproc_name(ch: char) -> Option<String> {
    match ch {
        '\u{FFFD}' | '\0' => None,
        ' ' => Some("space".to_string()),
        other if other.is_ascii_alphanumeric() => Some(other.to_string()),
        other => Some(format!("uni{:04X}", other as u32)),
    }
}

fn type3_device_color_space(name: Option<&str>) -> Type3DeviceColorSpace {
    match name {
        Some("DeviceGray" | "G") => Type3DeviceColorSpace::Gray,
        Some("DeviceRGB" | "RGB") => Type3DeviceColorSpace::Rgb,
        Some("DeviceCMYK" | "CMYK") => Type3DeviceColorSpace::Cmyk,
        _ => Type3DeviceColorSpace::Unsupported,
    }
}

fn type3_color_from_space(
    space: Type3DeviceColorSpace,
    op: &ContentOperation,
) -> Option<PixelColor> {
    match space {
        Type3DeviceColorSpace::Gray => op.number(0).map(type3_gray_color),
        Type3DeviceColorSpace::Rgb => {
            let (Some(r), Some(g), Some(b)) = (op.number(0), op.number(1), op.number(2)) else {
                return None;
            };
            Some(type3_rgb_color(r, g, b))
        }
        Type3DeviceColorSpace::Cmyk => {
            let (Some(c), Some(m), Some(y), Some(k)) =
                (op.number(0), op.number(1), op.number(2), op.number(3))
            else {
                return None;
            };
            Some(type3_cmyk_color(c, m, y, k))
        }
        Type3DeviceColorSpace::Unsupported => None,
    }
}

fn type3_gray_color(gray: f64) -> PixelColor {
    let g = unit_to_u8(gray);
    [g, g, g, 255]
}

fn type3_rgb_color(r: f64, g: f64, b: f64) -> PixelColor {
    [unit_to_u8(r), unit_to_u8(g), unit_to_u8(b), 255]
}

fn type3_cmyk_color(c: f64, m: f64, y: f64, k: f64) -> PixelColor {
    let c = c.clamp(0.0, 1.0);
    let m = m.clamp(0.0, 1.0);
    let y = y.clamp(0.0, 1.0);
    let k = k.clamp(0.0, 1.0);
    [
        unit_to_u8((1.0 - c) * (1.0 - k)),
        unit_to_u8((1.0 - m) * (1.0 - k)),
        unit_to_u8((1.0 - y) * (1.0 - k)),
        255,
    ]
}

fn type3_color_with_alpha(mut color: PixelColor, alpha: f64) -> PixelColor {
    color[3] = unit_to_u8(alpha);
    color
}

fn unit_to_u8(value: f64) -> u8 {
    if !value.is_finite() {
        0
    } else {
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    }
}

fn resolve_type3_charproc_object(
    font_dict: &PdfDictionary,
    glyph_name: &str,
    reader: &crate::reader::PdfReader,
) -> Option<PdfObject> {
    let charprocs_obj = font_dict.get("CharProcs")?.clone();
    let charprocs = match reader.resolve(charprocs_obj).ok()? {
        PdfObject::Dictionary(dict) => dict,
        _ => return None,
    };
    let glyph_obj = charprocs.get(glyph_name)?.clone();
    match reader.resolve(glyph_obj).ok()? {
        PdfObject::Stream { dict, raw } => Some(PdfObject::Stream { dict, raw }),
        _ => None,
    }
}

fn type3_font_matrix(font_dict: &PdfDictionary) -> Transform2D {
    let Some(items) = font_dict.get("FontMatrix").and_then(PdfObject::as_array) else {
        return Transform2D::scale(0.001, 0.001);
    };
    let values: Vec<f64> = items.iter().filter_map(PdfObject::as_number).collect();
    if values.len() < 6 || !values.iter().all(|value| value.is_finite()) {
        return Transform2D::scale(0.001, 0.001);
    }
    Transform2D::from([
        values[0], values[1], values[2], values[3], values[4], values[5],
    ])
}

fn type3_font_bbox(font_dict: &PdfDictionary) -> Option<[f64; 4]> {
    let items = font_dict.get("FontBBox").and_then(PdfObject::as_array)?;
    bbox_from_pdf_array(items)
}

fn bbox_from_pdf_array(items: &[PdfObject]) -> Option<[f64; 4]> {
    if items.len() < 4 {
        return None;
    }
    let mut values = [0.0; 4];
    for (idx, item) in items.iter().take(4).enumerate() {
        let value = item.as_number()?;
        if !value.is_finite() {
            return None;
        }
        values[idx] = value;
    }
    if (values[2] - values[0]).abs() <= f64::EPSILON
        || (values[3] - values[1]).abs() <= f64::EPSILON
    {
        return None;
    }
    Some(values)
}

fn type3_charproc_device_bounds(
    charproc: &Type3CharProc,
    font_dict: &PdfDictionary,
    device_t: &Transform2D,
    width: u32,
    height: u32,
) -> Option<(i32, i32, i32, i32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let bbox = charproc.glyph_bbox.or_else(|| type3_font_bbox(font_dict))?;
    let padding = (device_t.scale_factor().ceil() as i32).clamp(4, 64);
    bbox_device_bounds(bbox, device_t, width, height, padding)
}

fn bbox_device_bounds(
    bbox: [f64; 4],
    device_t: &Transform2D,
    width: u32,
    height: u32,
    padding: i32,
) -> Option<(i32, i32, i32, i32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let x0 = bbox[0].min(bbox[2]);
    let x1 = bbox[0].max(bbox[2]);
    let y0 = bbox[1].min(bbox[3]);
    let y1 = bbox[1].max(bbox[3]);
    let corners = [
        device_t.transform_point(x0, y0),
        device_t.transform_point(x1, y0),
        device_t.transform_point(x1, y1),
        device_t.transform_point(x0, y1),
    ];
    if corners
        .iter()
        .any(|(x, y)| !x.is_finite() || !y.is_finite())
    {
        return None;
    }
    let min_x = corners
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::INFINITY, f64::min);
    let max_x = corners
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = corners
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::INFINITY, f64::min);
    let max_y = corners
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);
    let page_x1 = u32_to_i32_saturating(width.saturating_sub(1));
    let page_y1 = u32_to_i32_saturating(height.saturating_sub(1));
    let left = f64_floor_to_i32_saturating(min_x).saturating_sub(padding);
    let top = f64_floor_to_i32_saturating(min_y).saturating_sub(padding);
    let right = f64_ceil_to_i32_saturating(max_x).saturating_add(padding);
    let bottom = f64_ceil_to_i32_saturating(max_y).saturating_add(padding);
    let left = left.clamp(0, page_x1);
    let top = top.clamp(0, page_y1);
    let right = right.clamp(0, page_x1);
    let bottom = bottom.clamp(0, page_y1);
    if right < left || bottom < top {
        None
    } else {
        Some((left, top, right, bottom))
    }
}

fn type3_charproc_cache_key(
    font_name: &str,
    font_dict: &PdfDictionary,
    glyph_name: &str,
) -> String {
    format!(
        "{}\0{}",
        font_resource_cache_key(font_name, font_dict),
        glyph_name
    )
}

fn type3_advance_width_from_ops(ops: &[ContentOperation]) -> Option<f64> {
    ops.iter().find_map(|op| match op.operator.as_str() {
        "d0" | "d1" => op
            .number(0)
            .filter(|value| value.is_finite() && *value > 0.0),
        _ => None,
    })
}

fn type3_glyph_bbox_from_ops(ops: &[ContentOperation]) -> Option<[f64; 4]> {
    ops.iter().find_map(|op| match op.operator.as_str() {
        "d1" => {
            let bbox = [op.number(2)?, op.number(3)?, op.number(4)?, op.number(5)?];
            if bbox.iter().all(|value| value.is_finite())
                && (bbox[2] - bbox[0]).abs() > f64::EPSILON
                && (bbox[3] - bbox[1]).abs() > f64::EPSILON
            {
                Some(bbox)
            } else {
                None
            }
        }
        _ => None,
    })
}

fn f64_floor_to_i32_saturating(value: f64) -> i32 {
    if !value.is_finite() {
        0
    } else if value <= i32::MIN as f64 {
        i32::MIN
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        value.floor() as i32
    }
}

fn f64_ceil_to_i32_saturating(value: f64) -> i32 {
    if !value.is_finite() {
        0
    } else if value <= i32::MIN as f64 {
        i32::MIN
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        value.ceil() as i32
    }
}

#[cfg(test)]
fn glyph_cache_code(ch: char) -> u16 {
    u16::try_from(ch as u32).unwrap_or(0xFFFD)
}

/// Resolve a font dict's `/FontDescriptor` (which may be an indirect reference)
/// to its dictionary.
fn resolve_descriptor(
    font_dict: &PdfDictionary,
    reader: &crate::reader::PdfReader,
) -> Option<PdfDictionary> {
    match reader
        .resolve(font_dict.get("FontDescriptor")?.clone())
        .ok()?
    {
        PdfObject::Dictionary(dict) => Some(dict),
        _ => None,
    }
}

fn fallback_font_lookup_name(
    font_name: &str,
    font_dict: &PdfDictionary,
    reader: &crate::reader::PdfReader,
) -> String {
    if let Some(name) = font_dict.get_name("BaseFont") {
        return name.to_string();
    }

    if detect_font_subtype(font_dict) == FontSubtype::Type0 {
        if let Some(descendant_font) = get_descendant_font(font_dict, reader) {
            if let Some(name) = descendant_font.get_name("BaseFont") {
                return name.to_string();
            }
            if let Some(name) = resolve_descriptor(&descendant_font, reader)
                .and_then(|descriptor| descriptor.get_name("FontName").map(|name| name.to_string()))
            {
                return name;
            }
        }
    }

    if let Some(name) = resolve_descriptor(font_dict, reader)
        .and_then(|descriptor| descriptor.get_name("FontName").map(|name| name.to_string()))
    {
        return name;
    }

    font_name.to_string()
}

fn font_resource_cache_key(font_name: &str, font_dict: &PdfDictionary) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    fnv1a_update(&mut hash, font_name.as_bytes());
    hash_pdf_dictionary(&mut hash, font_dict, 0);
    format!("{font_name}:{hash:016x}")
}

fn font_resource_glyph_cache_hash(font_bytes: &[u8], font_cache_key: &str) -> u64 {
    let mut hash = GlyphCache::hash_font_bytes(font_bytes);
    fnv1a_update(&mut hash, font_cache_key.as_bytes());
    hash
}

fn hash_pdf_dictionary(hash: &mut u64, dict: &PdfDictionary, depth: usize) {
    fnv1a_update(hash, b"<<");
    for (key, value) in dict.entries() {
        fnv1a_update(hash, key.as_bytes());
        hash_pdf_object(hash, value, depth.saturating_add(1));
    }
    fnv1a_update(hash, b">>");
}

fn hash_pdf_object(hash: &mut u64, object: &PdfObject, depth: usize) {
    if depth > 8 {
        fnv1a_update(hash, b"depth-cap");
        return;
    }
    match object {
        PdfObject::Boolean(value) => fnv1a_update(hash, if *value { b"true" } else { b"false" }),
        PdfObject::Integer(value) => {
            fnv1a_update(hash, b"int");
            fnv1a_update(hash, &value.to_le_bytes());
        }
        PdfObject::Real(value) => {
            fnv1a_update(hash, b"real");
            fnv1a_update(hash, &value.to_bits().to_le_bytes());
        }
        PdfObject::String(value) => {
            fnv1a_update(hash, b"str");
            fnv1a_update(hash, value);
        }
        PdfObject::Name(value) => {
            fnv1a_update(hash, b"name");
            fnv1a_update(hash, value.as_bytes());
        }
        PdfObject::Array(items) => {
            fnv1a_update(hash, b"[");
            for item in items {
                hash_pdf_object(hash, item, depth.saturating_add(1));
            }
            fnv1a_update(hash, b"]");
        }
        PdfObject::Dictionary(dict) => hash_pdf_dictionary(hash, dict, depth.saturating_add(1)),
        PdfObject::Stream { dict, raw } => {
            fnv1a_update(hash, b"stream");
            hash_pdf_dictionary(hash, dict, depth.saturating_add(1));
            fnv1a_update(hash, &(raw.len() as u64).to_le_bytes());
        }
        PdfObject::Null => fnv1a_update(hash, b"null"),
        PdfObject::Reference { number, generation } => {
            fnv1a_update(hash, b"ref");
            fnv1a_update(hash, &number.to_le_bytes());
            fnv1a_update(hash, &generation.to_le_bytes());
        }
    }
}

fn fnv1a_update(hash: &mut u64, bytes: &[u8]) {
    const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;
    for &byte in bytes {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

fn should_paint_decoded_glyph(glyph: &DecodedGlyph) -> bool {
    if glyph.is_gid {
        return true;
    }
    if glyph.unicode == '\u{FFFD}' {
        return glyph
            .glyph_name
            .as_deref()
            .is_some_and(|name| name != ".notdef");
    }
    let codepoint = glyph.unicode as u32;
    !(codepoint < 0x20 || codepoint == 0x7F)
}

fn text_rendering_mode_paints(mode: i32) -> bool {
    matches!(mode, 0 | 1 | 2 | 4 | 5 | 6)
}

fn text_rendering_mode_clips(mode: i32) -> bool {
    matches!(mode, 4..=7)
}

fn text_showing_operator(op: &ContentOperation) -> bool {
    matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\"")
}

fn positive_u32(value: Option<i64>, default: u32) -> u32 {
    value
        .filter(|number| *number > 0)
        .and_then(|number| u32::try_from(number).ok())
        .unwrap_or(default)
}

fn extract_color_space_name(dict: &PdfDictionary) -> String {
    match dict.get("ColorSpace").or_else(|| dict.get("CS")) {
        Some(PdfObject::Name(name)) => match name.as_str() {
            "G" => "DeviceGray".to_string(),
            "RGB" => "DeviceRGB".to_string(),
            "CMYK" => "DeviceCMYK".to_string(),
            other => other.to_string(),
        },
        Some(PdfObject::Array(items)) => items
            .first()
            .and_then(PdfObject::as_name)
            .unwrap_or("DeviceRGB")
            .to_string(),
        _ => "DeviceRGB".to_string(),
    }
}

fn canonical_image_color_space_name(name: &str) -> String {
    match name {
        "G" => "DeviceGray".to_string(),
        "RGB" => "DeviceRGB".to_string(),
        "CMYK" => "DeviceCMYK".to_string(),
        other => other.to_string(),
    }
}

fn image_color_space_family_name(
    obj: &PdfObject,
    resources: &PageResources,
    reader: &crate::reader::PdfReader,
    depth: usize,
) -> Option<String> {
    if depth > 8 {
        return None;
    }
    let resolved = match obj {
        PdfObject::Reference { .. } => reader.resolve(obj.clone()).ok()?,
        other => other.clone(),
    };
    match resolved {
        PdfObject::Name(name) => {
            if let Some(resource_obj) = resources.color_spaces.get(&name) {
                image_color_space_family_name(
                    resource_obj,
                    resources,
                    reader,
                    depth.saturating_add(1),
                )
            } else {
                Some(canonical_image_color_space_name(&name))
            }
        }
        PdfObject::Array(items) => items
            .first()
            .and_then(PdfObject::as_name)
            .map(canonical_image_color_space_name),
        _ => None,
    }
}

fn extract_filter_names(dict: &PdfDictionary) -> Vec<String> {
    match dict.get("Filter").or_else(|| dict.get("F")) {
        Some(PdfObject::Name(name)) => vec![name.clone()],
        Some(PdfObject::Array(items)) => items
            .iter()
            .filter_map(PdfObject::as_name)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn image_interpolate(dict: &PdfDictionary) -> bool {
    dict.get_bool("Interpolate")
        .or_else(|| dict.get_bool("I"))
        .unwrap_or(false)
}

fn font_size_scale(font_size: f64, upem: f64) -> f64 {
    if font_size <= 0.0 || upem <= 0.0 || !font_size.is_finite() || !upem.is_finite() {
        0.0
    } else {
        font_size / upem
    }
}

fn paint_cached_path_fill(
    cache: &mut PathFillMaskCache,
    buf: &mut PixelBuffer,
    viewport: &Viewport,
    path: &Path,
    ctm: &Transform2D,
    rule: FillRule,
    color: PixelColor,
) -> bool {
    if path.segments.is_empty() || path.segments.len() > 16_384 {
        return false;
    }
    let device_t = ctm.concat(&viewport.to_transform());
    if [
        device_t.a, device_t.b, device_t.c, device_t.d, device_t.e, device_t.f,
    ]
    .iter()
    .any(|value| !value.is_finite())
    {
        return false;
    }
    if device_t.scale_factor() <= 0.0 || device_t.scale_factor() > 256.0 {
        return false;
    }
    let origin_x = device_t.e.floor();
    let origin_y = device_t.f.floor();
    let normalized_t = Transform2D {
        e: device_t.e - origin_x,
        f: device_t.f - origin_y,
        ..device_t
    };
    let key = PathFillMaskCacheKey {
        path_hash: hash_path_for_fill_cache(path),
        fill_rule: match rule {
            FillRule::NonZero => 0,
            FillRule::EvenOdd => 1,
        },
        a: quantize_glyph_mask_value(normalized_t.a),
        b: quantize_glyph_mask_value(normalized_t.b),
        c: quantize_glyph_mask_value(normalized_t.c),
        d: quantize_glyph_mask_value(normalized_t.d),
        frac_e: quantize_glyph_mask_fraction(normalized_t.e),
        frac_f: quantize_glyph_mask_fraction(normalized_t.f),
    };
    let dx = if origin_x <= i32::MIN as f64 {
        i32::MIN
    } else if origin_x >= i32::MAX as f64 {
        i32::MAX
    } else {
        origin_x as i32
    };
    let dy = if origin_y <= i32::MIN as f64 {
        i32::MIN
    } else if origin_y >= i32::MAX as f64 {
        i32::MAX
    } else {
        origin_y as i32
    };
    if let Some(mask) = cache.get(&key) {
        mask.paint(buf, dx, dy, color);
        return true;
    }
    if let Some((x, y, width, height)) = axis_aligned_integer_rect(path, ctm, viewport) {
        buf.fill_rect(x, y, width, height, color);
        return true;
    }
    let Some(mask) =
        rasterize_glyph_alpha_mask(path, &normalized_t, rule, GlyphHinting::disabled())
    else {
        return false;
    };
    let mask = Arc::new(mask);
    mask.paint(buf, dx, dy, color);
    cache.insert(key, mask);
    true
}

#[allow(clippy::too_many_arguments)]
fn paint_cached_path_stroke(
    cache: &mut PathStrokeMaskCache,
    buf: &mut PixelBuffer,
    viewport: &Viewport,
    path: &Path,
    ctm: &Transform2D,
    color: PixelColor,
    stroke_width: f64,
    dash: &DashState,
    cap: &LineCap,
    join: &LineJoin,
    miter_limit: f64,
) -> bool {
    if path.segments.is_empty()
        || path.segments.len() > 16_384
        || !dash.is_solid()
        || stroke_width <= 0.0
        || !stroke_width.is_finite()
    {
        return false;
    }
    let device_t = ctm.concat(&viewport.to_transform());
    if [
        device_t.a, device_t.b, device_t.c, device_t.d, device_t.e, device_t.f,
    ]
    .iter()
    .any(|value| !value.is_finite())
    {
        return false;
    }
    if device_t.scale_factor() <= 0.0 || device_t.scale_factor() > 256.0 {
        return false;
    }
    let origin_x = device_t.e.floor();
    let origin_y = device_t.f.floor();
    let normalized_t = Transform2D {
        e: device_t.e - origin_x,
        f: device_t.f - origin_y,
        ..device_t
    };
    let key = PathStrokeMaskCacheKey {
        path_hash: hash_path_for_fill_cache(path),
        width: quantize_glyph_mask_value(stroke_width * device_t.scale_factor()),
        cap: line_cap_cache_id(cap),
        join: line_join_cache_id(join),
        miter_limit: quantize_glyph_mask_value(miter_limit),
        a: quantize_glyph_mask_value(normalized_t.a),
        b: quantize_glyph_mask_value(normalized_t.b),
        c: quantize_glyph_mask_value(normalized_t.c),
        d: quantize_glyph_mask_value(normalized_t.d),
        frac_e: quantize_glyph_mask_fraction(normalized_t.e),
        frac_f: quantize_glyph_mask_fraction(normalized_t.f),
    };
    let dx = if origin_x <= i32::MIN as f64 {
        i32::MIN
    } else if origin_x >= i32::MAX as f64 {
        i32::MAX
    } else {
        origin_x as i32
    };
    let dy = if origin_y <= i32::MIN as f64 {
        i32::MIN
    } else if origin_y >= i32::MAX as f64 {
        i32::MAX
    } else {
        origin_y as i32
    };
    if let Some(mask) = cache.get(&key) {
        mask.paint(buf, dx, dy, color);
        return true;
    }
    let flat = crate::render::path::flatten_path_device_transform(path, &normalized_t, 0.5);
    let width_px = (stroke_width * normalized_t.scale_factor()).max(1.0);
    let outline = stroke_flat_path(
        &flat,
        width_px,
        dash,
        cap.clone(),
        join.clone(),
        miter_limit,
    );
    if outline.subpaths.is_empty() {
        return true;
    }
    let Some(mask) = rasterize_flat_alpha_mask(&outline, FillRule::NonZero) else {
        return false;
    };
    let mask = Arc::new(mask);
    mask.paint(buf, dx, dy, color);
    cache.insert(key, mask);
    true
}

fn line_cap_cache_id(cap: &LineCap) -> u8 {
    match cap {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::ProjectingSquare => 2,
    }
}

fn line_join_cache_id(join: &LineJoin) -> u8 {
    match join {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
    }
}

fn hash_path_for_fill_cache(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.segments.len().hash(&mut hasher);
    for segment in &path.segments {
        match segment {
            crate::render::path::PathSegment::MoveTo(x, y) => {
                0u8.hash(&mut hasher);
                x.to_bits().hash(&mut hasher);
                y.to_bits().hash(&mut hasher);
            }
            crate::render::path::PathSegment::LineTo(x, y) => {
                1u8.hash(&mut hasher);
                x.to_bits().hash(&mut hasher);
                y.to_bits().hash(&mut hasher);
            }
            crate::render::path::PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                2u8.hash(&mut hasher);
                cp1x.to_bits().hash(&mut hasher);
                cp1y.to_bits().hash(&mut hasher);
                cp2x.to_bits().hash(&mut hasher);
                cp2y.to_bits().hash(&mut hasher);
                x.to_bits().hash(&mut hasher);
                y.to_bits().hash(&mut hasher);
            }
            crate::render::path::PathSegment::ClosePath => {
                3u8.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

fn quantize_glyph_mask_value(value: f64) -> i64 {
    const SCALE: f64 = 64.0;
    if !value.is_finite() {
        0
    } else if value <= i64::MIN as f64 / SCALE {
        i64::MIN
    } else if value >= i64::MAX as f64 / SCALE {
        i64::MAX
    } else {
        (value * SCALE).round() as i64
    }
}

fn quantize_glyph_mask_fraction(value: f64) -> i64 {
    const SCALE: f64 = 2.0;
    if !value.is_finite() {
        0
    } else {
        (value.fract() * SCALE).round() as i64
    }
}

fn display_list_needs_transparent_page_group(
    engine: &ContentEngine,
    page_number: usize,
    resources: &PageResources,
    list: &DisplayList,
) -> Result<bool> {
    if list.stats.transparency_ops == 0
        && list.stats.image_xobjects == 0
        && list.stats.inline_images == 0
        && list.stats.form_xobjects == 0
    {
        return Ok(false);
    }
    let ops = engine.get_page_content(page_number)?;
    Ok(uses_top_level_transparency(&ops, resources, engine))
}

fn uses_top_level_transparency(
    ops: &[ContentOperation],
    resources: &PageResources,
    engine: &ContentEngine,
) -> bool {
    for op in ops {
        match op.operator.as_str() {
            "gs" => {
                if let Some(name) = op.name(0) {
                    if resources
                        .ext_g_states
                        .get(name)
                        .is_some_and(ext_g_state_needs_transparent_backdrop)
                    {
                        return true;
                    }
                }
            }
            "Do" => {
                if let Some(name) = op.name(0) {
                    if xobject_needs_transparent_backdrop(name, resources, engine) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn ext_g_state_needs_transparent_backdrop(dict: &PdfDictionary) -> bool {
    let alpha_changed = ["ca", "CA"].iter().any(|key| {
        dict.get(key)
            .and_then(PdfObject::as_number)
            .is_some_and(|v| v < 0.999)
    });
    if alpha_changed {
        return true;
    }

    let blend_changed = match dict.get("BM") {
        Some(PdfObject::Name(name)) => name != "Normal" && name != "Compatible",
        Some(PdfObject::Array(items)) => items
            .iter()
            .filter_map(PdfObject::as_name)
            .any(|name| name != "Normal" && name != "Compatible"),
        _ => false,
    };
    if blend_changed {
        return true;
    }

    match dict.get("SMask") {
        Some(PdfObject::Name(name)) if name == "None" => false,
        Some(_) => true,
        None => false,
    }
}

fn xobject_needs_transparent_backdrop(
    name: &str,
    resources: &PageResources,
    engine: &ContentEngine,
) -> bool {
    let Some(&(obj_num, gen_num)) = resources.xobjects.get(name) else {
        return false;
    };
    match engine.document().reader().get_object(obj_num, gen_num) {
        Ok(PdfObject::Stream { dict, .. }) => is_transparency_group(&dict),
        _ => false,
    }
}

fn is_transparency_group(form_dict: &PdfDictionary) -> bool {
    matches!(
        form_dict.get("Group"),
        Some(PdfObject::Dictionary(group)) if group.get_name("S") == Some("Transparency")
    )
}

/// Collect inline image `ID` operands (alternating key/value) into a map.
/// Keys arrive already normalized to full names by the content parser.
fn inline_params_to_map(operands: &[Operand]) -> std::collections::HashMap<String, Operand> {
    let mut map = std::collections::HashMap::new();
    let mut iter = operands.iter();
    while let Some(key_op) = iter.next() {
        if let Operand::Name(key) = key_op {
            if let Some(value) = iter.next() {
                map.insert(key.clone(), value.clone());
            }
        }
    }
    map
}

fn dict_int(map: &std::collections::HashMap<String, Operand>, key: &str) -> Option<i64> {
    match map.get(key)? {
        Operand::Integer(n) => Some(*n),
        Operand::Real(r) => Some(*r as i64),
        _ => None,
    }
}

fn dict_bool(map: &std::collections::HashMap<String, Operand>, key: &str) -> Option<bool> {
    match map.get(key)? {
        Operand::Boolean(b) => Some(*b),
        _ => None,
    }
}

fn dict_name<'a>(
    map: &'a std::collections::HashMap<String, Operand>,
    key: &str,
) -> Option<&'a str> {
    match map.get(key)? {
        Operand::Name(n) => Some(n.as_str()),
        _ => None,
    }
}

/// Extract the inline image filter chain (`/Filter`), accepting a single name
/// or a name array. Returns filter names verbatim (full forms after parser
/// normalization); `decode_inline` understands them.
fn dict_filter_list(map: &std::collections::HashMap<String, Operand>) -> Vec<&str> {
    match map.get("Filter") {
        Some(Operand::Name(n)) => vec![n.as_str()],
        Some(Operand::Array(items)) => items.iter().filter_map(Operand::as_name).collect(),
        _ => Vec::new(),
    }
}

fn inline_decode_params(
    map: &std::collections::HashMap<String, Operand>,
    filter_count: usize,
) -> Result<Vec<Option<PdfDictionary>>> {
    let Some(value) = map.get("DecodeParms") else {
        return Ok(vec![None; filter_count]);
    };
    match value {
        Operand::Dictionary(entries) => {
            if filter_count == 0 {
                return Err(WellfriendError::MalformedPdf(
                    "inline DecodeParms present without Filter".to_string(),
                ));
            }
            let mut out = vec![None; filter_count];
            out[0] = Some(inline_operand_dictionary(entries)?);
            Ok(out)
        }
        Operand::Array(items) if items.len() == filter_count => items
            .iter()
            .map(|item| match item {
                Operand::Dictionary(entries) => Ok(Some(inline_operand_dictionary(entries)?)),
                _ => Err(WellfriendError::MalformedPdf(
                    "inline DecodeParms array contains a non-dictionary".to_string(),
                )),
            })
            .collect(),
        Operand::Array(items) => Err(WellfriendError::MalformedPdf(format!(
            "inline DecodeParms count {} does not match Filter count {filter_count}",
            items.len()
        ))),
        _ => Err(WellfriendError::MalformedPdf(
            "inline DecodeParms is not a dictionary or matching array".to_string(),
        )),
    }
}

fn inline_operand_dictionary(entries: &[(String, Operand)]) -> Result<PdfDictionary> {
    let mut dict = PdfDictionary::empty();
    for (key, value) in entries {
        let object = inline_operand_object(value).ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "inline DecodeParms /{key} contains an unsupported object"
            ))
        })?;
        dict.insert(key, object);
    }
    Ok(dict)
}

fn inline_operand_object(value: &Operand) -> Option<PdfObject> {
    match value {
        Operand::Integer(value) => Some(PdfObject::Integer(*value)),
        Operand::Real(value) => Some(PdfObject::Real(*value)),
        Operand::Boolean(value) => Some(PdfObject::Boolean(*value)),
        Operand::Name(value) => Some(PdfObject::Name(value.clone())),
        Operand::String(value) => Some(PdfObject::String(value.clone())),
        Operand::Array(items) => Some(PdfObject::Array(
            items
                .iter()
                .map(inline_operand_object)
                .collect::<Option<Vec<_>>>()?,
        )),
        Operand::Dictionary(entries) => Some(PdfObject::Dictionary(
            inline_operand_dictionary(entries).ok()?,
        )),
    }
}

fn image_mask_to_stencil_rgba(
    raw: crate::images::decoder::RawImage,
    color: PixelColor,
    paint_ones: bool,
) -> crate::images::decoder::RawImage {
    let pixel_count = raw.width as usize * raw.height as usize;
    let channels = raw.channels.max(1) as usize;
    let mut pixels = Vec::with_capacity(pixel_count * 4);
    for i in 0..pixel_count {
        let sample = raw.pixels.get(i * channels).copied().unwrap_or(0);
        let paint = if paint_ones {
            sample >= 128
        } else {
            sample < 128
        };
        pixels.push(color[0]);
        pixels.push(color[1]);
        pixels.push(color[2]);
        pixels.push(if paint { color[3] } else { 0 });
    }
    crate::images::decoder::RawImage {
        width: raw.width,
        height: raw.height,
        channels: 4,
        bits_per_sample: 8,
        pixels,
    }
}

fn combine_explicit_image_mask(
    main: crate::images::decoder::RawImage,
    mask: &crate::images::decoder::RawImage,
    mask_is_stencil: bool,
    paint_ones: bool,
) -> Result<crate::images::decoder::RawImage> {
    if main.width != mask.width || main.height != mask.height {
        log::warn!(
            "Explicit image /Mask dimensions {}x{} do not match image {}x{}; ignoring Mask",
            mask.width,
            mask.height,
            main.width,
            main.height
        );
        return Ok(main);
    }
    let pixel_count = main.width as usize * main.height as usize;
    let channels = main.channels.max(1) as usize;
    let mask_channels = mask.channels.max(1) as usize;
    let mut pixels = Vec::with_capacity(pixel_count * 4);
    for i in 0..pixel_count {
        let base = i * channels;
        let r = main.pixels.get(base).copied().unwrap_or(0);
        let g = if channels >= 3 {
            main.pixels.get(base + 1).copied().unwrap_or(r)
        } else {
            r
        };
        let b = if channels >= 3 {
            main.pixels.get(base + 2).copied().unwrap_or(r)
        } else {
            r
        };
        let existing_alpha = if channels >= 4 {
            main.pixels.get(base + 3).copied().unwrap_or(255)
        } else {
            255
        };
        let mbase = i * mask_channels;
        let mask_sample = if mask_channels >= 3 {
            let mr = mask.pixels.get(mbase).copied().unwrap_or(0) as u16;
            let mg = mask.pixels.get(mbase + 1).copied().unwrap_or(0) as u16;
            let mb = mask.pixels.get(mbase + 2).copied().unwrap_or(0) as u16;
            ((u32::from(mr) * 77 + u32::from(mg) * 150 + u32::from(mb) * 29 + 128) >> 8) as u8
        } else {
            mask.pixels.get(mbase).copied().unwrap_or(0)
        };
        let mask_alpha = if mask_is_stencil {
            let visible = if paint_ones {
                mask_sample >= 128
            } else {
                mask_sample < 128
            };
            if visible {
                255
            } else {
                0
            }
        } else {
            mask_sample
        };
        let alpha = ((u16::from(existing_alpha) * u16::from(mask_alpha) + 127) / 255) as u8;
        pixels.extend_from_slice(&[r, g, b, alpha]);
    }
    Ok(crate::images::decoder::RawImage {
        width: main.width,
        height: main.height,
        channels: 4,
        bits_per_sample: 8,
        pixels,
    })
}

fn apply_color_key_image_mask(
    main: crate::images::decoder::RawImage,
    bpc: u8,
    color_space: &str,
    items: &[PdfObject],
) -> Option<crate::images::decoder::RawImage> {
    let channels = match color_space {
        "DeviceGray" | "G" => 1usize,
        "DeviceRGB" | "RGB" | "sRGB" => 3usize,
        _ => return None,
    };
    if main.channels as usize != channels || items.len() < channels * 2 {
        return None;
    }
    let values = items
        .iter()
        .filter_map(PdfObject::as_number)
        .collect::<Vec<_>>();
    if values.len() < channels * 2 {
        return None;
    }
    let max_component = if bpc == 0 {
        255.0
    } else if bpc >= 8 {
        ((1u32 << bpc.min(16)) - 1) as f64
    } else {
        ((1u16 << bpc) - 1) as f64
    };
    let scale = 255.0 / max_component.max(1.0);
    let ranges = (0..channels)
        .map(|idx| {
            let lo = (values[idx * 2] * scale).round().clamp(0.0, 255.0) as u8;
            let hi = (values[idx * 2 + 1] * scale).round().clamp(0.0, 255.0) as u8;
            if lo <= hi {
                (lo, hi)
            } else {
                (hi, lo)
            }
        })
        .collect::<Vec<_>>();
    let pixel_count = main.width as usize * main.height as usize;
    let mut pixels = Vec::with_capacity(pixel_count * 4);
    for i in 0..pixel_count {
        let base = i * channels;
        let r = main.pixels.get(base).copied().unwrap_or(0);
        let g = if channels >= 3 {
            main.pixels.get(base + 1).copied().unwrap_or(r)
        } else {
            r
        };
        let b = if channels >= 3 {
            main.pixels.get(base + 2).copied().unwrap_or(r)
        } else {
            r
        };
        let masked = (0..channels).all(|ch| {
            let sample = main.pixels.get(base + ch).copied().unwrap_or(0);
            let (lo, hi) = ranges[ch];
            sample >= lo && sample <= hi
        });
        pixels.extend_from_slice(&[r, g, b, if masked { 0 } else { 255 }]);
    }
    Some(crate::images::decoder::RawImage {
        width: main.width,
        height: main.height,
        channels: 4,
        bits_per_sample: 8,
        pixels,
    })
}

fn image_mask_paints_ones(dict: &PdfDictionary) -> bool {
    dict.get("Decode")
        .and_then(PdfObject::as_array)
        .and_then(|items| decode_array_paints_ones(items.iter().filter_map(PdfObject::as_number)))
        .unwrap_or(true)
}

fn inline_image_mask_paints_ones(map: &std::collections::HashMap<String, Operand>) -> bool {
    map.get("Decode")
        .and_then(Operand::as_array)
        .and_then(|items| decode_array_paints_ones(items.iter().filter_map(Operand::as_number)))
        .unwrap_or(true)
}

fn decode_array_paints_ones(mut values: impl Iterator<Item = f64>) -> Option<bool> {
    let zero = values.next()?;
    let one = values.next()?;
    Some(one >= zero)
}

/// Determine the opaque backdrop color for a luminosity soft mask.
///
/// Defaults to black `[0,0,0,255]` (the spec default, which yields mask=0 in
/// unpainted areas). An explicit `/BC` array overrides it, interpreted in the
/// mask group's color space (`/Group /CS`) by component count: 1 â†’ gray, 3 â†’
/// RGB, 4 â†’ CMYK. The result is always opaque (alpha 255) so luminosity is
/// well-defined everywhere.
fn smask_backdrop_color(smask_dict: &PdfDictionary, g_dict: &PdfDictionary) -> PixelColor {
    let Some(bc) = smask_dict
        .get("BC")
        .and_then(PdfObject::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(PdfObject::as_number)
                .collect::<Vec<f64>>()
        })
    else {
        return [0, 0, 0, 255];
    };
    if bc.is_empty() {
        return [0, 0, 0, 255];
    }

    // Try to honor the group color space's channel count; fall back to the
    // number of /BC components.
    let space_name = g_dict
        .get("Group")
        .and_then(PdfObject::as_dict)
        .and_then(|g| g.get("CS"))
        .and_then(PdfObject::as_name)
        .map(str::to_string);

    let rc = match space_name.as_deref() {
        Some(name) => ColorSpaceHandler::from_components(name, &bc, 1.0),
        None => match bc.len() {
            1 => ColorSpaceHandler::from_components("DeviceGray", &bc, 1.0),
            4 => ColorSpaceHandler::from_components("DeviceCMYK", &bc, 1.0),
            _ => ColorSpaceHandler::from_components("DeviceRGB", &bc, 1.0),
        },
    };
    let p = rc.to_pixel_color();
    [p[0], p[1], p[2], 255]
}

fn smask_default_alpha(
    smask_dict: &PdfDictionary,
    g_dict: &PdfDictionary,
    is_alpha: bool,
    reader: &crate::reader::PdfReader,
) -> u8 {
    if is_alpha {
        if alpha_smask_uses_opaque_bc_backdrop(smask_dict, reader) {
            255
        } else {
            0
        }
    } else {
        let bc = smask_backdrop_color(smask_dict, g_dict);
        (0.299 * bc[0] as f32 + 0.587 * bc[1] as f32 + 0.114 * bc[2] as f32)
            .round()
            .clamp(0.0, 255.0) as u8
    }
}

fn alpha_smask_uses_opaque_bc_backdrop(
    smask_dict: &PdfDictionary,
    reader: &crate::reader::PdfReader,
) -> bool {
    if smask_dict.get("BC").is_none() {
        return false;
    }
    let Some(tr) = smask_dict.get("TR") else {
        return true;
    };
    if matches!(tr, PdfObject::Name(name) if name == "Identity") {
        return true;
    }
    let out = crate::render::shading::eval_function(tr, 0.0, reader);
    out.first().copied().unwrap_or(0.0) <= 0.01
}

fn smask_inline_cache_seed(smask_dict: &PdfDictionary) -> Option<String> {
    match smask_dict.get("G") {
        Some(PdfObject::Reference { number, generation }) => Some(format!(
            "smask:inline:g:{number}:{generation}:dict:{smask_dict:?}"
        )),
        _ => None,
    }
}

fn shading_mesh_cache_key(shading_obj: &PdfObject, shading_type: i64) -> Option<String> {
    match shading_obj {
        PdfObject::Reference { number, generation } => Some(format!(
            "shading-mesh:{number}:{generation}:type:{shading_type}"
        )),
        _ => None,
    }
}

fn form_xobject_program_cache_key(obj_num: u32, gen_num: u16) -> String {
    format!("form-program:{obj_num}:{gen_num}")
}

fn fingerprint_bytes64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// The `/Group` sub-dictionary of a Form XObject, if it is a transparency
/// group. Returns `None` for non-group Forms.
fn transparency_group_dict(form_dict: &PdfDictionary) -> Option<&PdfDictionary> {
    match form_dict.get("Group") {
        Some(PdfObject::Dictionary(group)) if group.get_name("S") == Some("Transparency") => {
            Some(group)
        }
        _ => None,
    }
}

/// Read the `/I` (isolated) flag of a transparency group dictionary
/// (default false).
fn group_is_isolated(group: &PdfDictionary) -> bool {
    group.get_bool("I").unwrap_or(false)
}

/// Read the `/K` (knockout) flag of a transparency group dictionary
/// (default false).
fn group_is_knockout(group: &PdfDictionary) -> bool {
    group.get_bool("K").unwrap_or(false)
}

/// Resolve a shading/pattern object (direct dict, stream, or indirect
/// reference) to its dictionary. Returns `None` for anything else.
fn resolve_to_dict(obj: &PdfObject, reader: &crate::reader::PdfReader) -> Option<PdfDictionary> {
    match obj {
        PdfObject::Dictionary(d) => Some(d.clone()),
        PdfObject::Stream { dict, .. } => Some(dict.clone()),
        PdfObject::Reference { number, generation } => {
            match reader.get_object(*number, *generation).ok()? {
                PdfObject::Dictionary(d) => Some(d),
                PdfObject::Stream { dict, .. } => Some(dict),
                _ => None,
            }
        }
        _ => None,
    }
}

fn plate_sample_components(obj: &PdfObject, reader: &crate::reader::PdfReader) -> Vec<f64> {
    let resolved = match reader.resolve(obj.clone()) {
        Ok(obj) => obj,
        Err(_) => obj.clone(),
    };
    let PdfObject::Array(arr) = resolved else {
        return Vec::new();
    };
    match arr.first().and_then(PdfObject::as_name) {
        Some("Separation") => vec![1.0],
        Some("DeviceN") => arr
            .get(1)
            .and_then(PdfObject::as_array)
            .map(|names| vec![1.0; names.len()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Reconstruct the concrete fill color for an uncolored (PaintType 2) tiling
/// pattern from the components recorded by `scn`. The fill color space is the
/// abstract Pattern space, so the concrete base space is inferred from the
/// number of numeric components.
fn uncolored_pattern_color(
    fill_color: &crate::content::state::Color,
) -> (ColorSpace, crate::content::state::Color) {
    let comps = fill_color.components.clone();
    let space = match comps.len() {
        1 => ColorSpace::DeviceGray,
        4 => ColorSpace::DeviceCMYK,
        _ => ColorSpace::DeviceRGB,
    };
    let color = crate::content::state::Color {
        space: space.clone(),
        components: comps,
    };
    (space, color)
}

fn tiling_pattern_stack_key(
    pattern_obj: &PdfObject,
    pat_dict: &PdfDictionary,
    raw_len: usize,
) -> String {
    if let PdfObject::Reference { number, generation } = pattern_obj {
        return format!("ref:{number}:{generation}");
    }
    let bbox = get_float_array_dict(pat_dict, "BBox").unwrap_or_default();
    let x_step = pat_dict
        .get("XStep")
        .and_then(PdfObject::as_number)
        .unwrap_or(0.0);
    let y_step = pat_dict
        .get("YStep")
        .and_then(PdfObject::as_number)
        .unwrap_or(0.0);
    format!("inline:{raw_len}:{x_step:.6}:{y_step:.6}:{bbox:?}")
}

/// For a mesh shading (ShadingType 4â€“7), decode and return the shading stream's
/// data (the packed vertex/patch records). Returns `None` for dictionary-only
/// shadings (Types 1â€“3) or if the object is not a stream.
fn estimate_stream_decode_bytes(stream_obj: &PdfObject) -> u64 {
    let raw_len = match stream_obj {
        PdfObject::Stream { raw, .. } => raw.len() as u64,
        _ => 1,
    };
    let limits = DecodeLimits::default();
    raw_len
        .saturating_mul(4)
        .max(raw_len)
        .max(1)
        .min(limits.max_decoded_bytes_per_stream)
}

fn estimate_image_ref_decode_bytes(image: &ImageReference) -> u64 {
    u64::from(image.width)
        .saturating_mul(u64::from(image.height))
        .saturating_mul(u64::from(image.bits_per_component.max(1)).div_ceil(8))
        .saturating_mul(estimated_image_channels(&image.color_space, image.is_mask))
        .max(1)
}

fn image_xobject_cache_key(image: &ImageReference) -> String {
    if image.is_inline {
        return format!(
            "inline:{}:{}:{}:{}",
            image.page_number,
            image.width,
            image.height,
            image
                .inline_data
                .as_ref()
                .map_or(0, |data| data.bytes.len())
        );
    }
    format!(
        "xobject:{}:{}:{}:{}:{}:{}",
        image.object_number,
        image.generation_number,
        image.width,
        image.height,
        image.bits_per_component,
        image.filter.join("+")
    )
}

fn image_xobject_cache_key_with_color_space(
    image: &ImageReference,
    color_space_name: &str,
    color_space_obj: &PdfObject,
) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    fnv1a_update(&mut hash, color_space_name.as_bytes());
    hash_pdf_object(&mut hash, color_space_obj, 0);
    format!("{}:cs:{hash:016x}", image_xobject_cache_key(image))
}

fn touch_image_xobject_cache_key(order: &mut VecDeque<String>, key: &str) {
    if let Some(pos) = order.iter().position(|item| item == key) {
        if let Some(existing) = order.remove(pos) {
            order.push_back(existing);
        }
    }
}

fn insert_image_xobject_cache_entry(
    cache: &mut HashMap<String, Arc<RawImage>>,
    order: &mut VecDeque<String>,
    cache_bytes: &mut usize,
    key: String,
    raw: Arc<RawImage>,
    raw_bytes: usize,
    max_bytes: usize,
) {
    if max_bytes == 0 || raw_bytes > max_bytes {
        return;
    }
    if let Some(previous) = cache.remove(&key) {
        *cache_bytes = cache_bytes.saturating_sub(previous.byte_count());
        if let Some(pos) = order.iter().position(|item| item == &key) {
            order.remove(pos);
        }
    }
    while cache_bytes.saturating_add(raw_bytes) > max_bytes {
        let Some(victim) = order.pop_front() else {
            break;
        };
        if let Some(removed) = cache.remove(&victim) {
            *cache_bytes = cache_bytes.saturating_sub(removed.byte_count());
        }
    }
    if cache_bytes.saturating_add(raw_bytes) <= max_bytes {
        order.push_back(key.clone());
        cache.insert(key, raw);
        *cache_bytes = cache_bytes.saturating_add(raw_bytes);
    }
}

const SCALED_IMAGE_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
const SCALED_IMAGE_CACHE_MAX_ENTRY_BYTES: usize = 32 * 1024 * 1024;

fn scaled_image_cache_key(
    base_key: &str,
    target_width: u32,
    target_height: u32,
    high_quality: bool,
) -> String {
    format!(
        "{base_key}:scaled:{}x{}:{}",
        target_width,
        target_height,
        if high_quality { "hq" } else { "compat" }
    )
}

fn touch_scaled_image_cache_key(order: &mut VecDeque<String>, key: &str) {
    if let Some(pos) = order.iter().position(|item| item == key) {
        if let Some(existing) = order.remove(pos) {
            order.push_back(existing);
        }
    }
}

fn insert_scaled_image_cache_entry(
    cache: &mut HashMap<String, Arc<RawImage>>,
    order: &mut VecDeque<String>,
    cache_bytes: &mut usize,
    key: String,
    raw: Arc<RawImage>,
) {
    let raw_bytes = raw.byte_count();
    if raw_bytes == 0
        || raw_bytes > SCALED_IMAGE_CACHE_MAX_ENTRY_BYTES
        || SCALED_IMAGE_CACHE_MAX_BYTES == 0
    {
        return;
    }
    if let Some(previous) = cache.remove(&key) {
        *cache_bytes = cache_bytes.saturating_sub(previous.byte_count());
        if let Some(pos) = order.iter().position(|item| item == &key) {
            order.remove(pos);
        }
    }
    while cache_bytes.saturating_add(raw_bytes) > SCALED_IMAGE_CACHE_MAX_BYTES {
        let Some(victim) = order.pop_front() else {
            break;
        };
        if let Some(removed) = cache.remove(&victim) {
            *cache_bytes = cache_bytes.saturating_sub(removed.byte_count());
        }
    }
    if cache_bytes.saturating_add(raw_bytes) <= SCALED_IMAGE_CACHE_MAX_BYTES {
        order.push_back(key.clone());
        cache.insert(key, raw);
        *cache_bytes = cache_bytes.saturating_add(raw_bytes);
    }
}

const SMASK_GROUP_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
const SMASK_GROUP_CACHE_MAX_ENTRY_BYTES: usize = 32 * 1024 * 1024;

fn touch_smask_group_cache_key(order: &mut VecDeque<String>, key: &str) {
    if let Some(pos) = order.iter().position(|item| item == key) {
        if let Some(existing) = order.remove(pos) {
            order.push_back(existing);
        }
    }
}

fn insert_smask_group_cache_entry(
    cache: &mut HashMap<String, Arc<AlphaMask>>,
    order: &mut VecDeque<String>,
    cache_bytes: &mut usize,
    stats: &mut RenderArtifactCacheStats,
    key: String,
    mask: Arc<AlphaMask>,
) {
    let mask_bytes = mask.approximate_bytes();
    if mask_bytes == 0
        || mask_bytes > SMASK_GROUP_CACHE_MAX_ENTRY_BYTES
        || SMASK_GROUP_CACHE_MAX_BYTES == 0
    {
        stats.skipped_oversized = stats.skipped_oversized.saturating_add(1);
        return;
    }
    if let Some(previous) = cache.remove(&key) {
        *cache_bytes = cache_bytes.saturating_sub(previous.approximate_bytes());
        if let Some(pos) = order.iter().position(|item| item == &key) {
            order.remove(pos);
        }
    }
    while cache_bytes.saturating_add(mask_bytes) > SMASK_GROUP_CACHE_MAX_BYTES {
        let Some(victim) = order.pop_front() else {
            break;
        };
        if let Some(removed) = cache.remove(&victim) {
            *cache_bytes = cache_bytes.saturating_sub(removed.approximate_bytes());
            stats.evictions = stats.evictions.saturating_add(1);
        }
    }
    if cache_bytes.saturating_add(mask_bytes) <= SMASK_GROUP_CACHE_MAX_BYTES {
        order.push_back(key.clone());
        cache.insert(key, mask);
        *cache_bytes = cache_bytes.saturating_add(mask_bytes);
    }
}

const SHADING_MESH_CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
const SHADING_MESH_CACHE_MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

fn touch_shading_mesh_cache_key(order: &mut VecDeque<String>, key: &str) {
    if let Some(pos) = order.iter().position(|item| item == key) {
        if let Some(existing) = order.remove(pos) {
            order.push_back(existing);
        }
    }
}

fn insert_shading_mesh_cache_entry(
    cache: &mut HashMap<String, Arc<Vec<u8>>>,
    order: &mut VecDeque<String>,
    cache_bytes: &mut usize,
    key: String,
    data: Arc<Vec<u8>>,
) {
    let data_bytes = data.len();
    if data_bytes == 0 || data_bytes > SHADING_MESH_CACHE_MAX_ENTRY_BYTES {
        return;
    }
    if let Some(previous) = cache.remove(&key) {
        *cache_bytes = cache_bytes.saturating_sub(previous.len());
        if let Some(pos) = order.iter().position(|item| item == &key) {
            order.remove(pos);
        }
    }
    while cache_bytes.saturating_add(data_bytes) > SHADING_MESH_CACHE_MAX_BYTES {
        let Some(victim) = order.pop_front() else {
            break;
        };
        if let Some(removed) = cache.remove(&victim) {
            *cache_bytes = cache_bytes.saturating_sub(removed.len());
        }
    }
    if cache_bytes.saturating_add(data_bytes) <= SHADING_MESH_CACHE_MAX_BYTES {
        order.push_back(key.clone());
        cache.insert(key, data);
        *cache_bytes = cache_bytes.saturating_add(data_bytes);
    }
}

fn estimate_inline_image_decode_bytes(raw_len: usize, width: u32, height: u32, bpc: u8) -> u64 {
    u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(u64::from(bpc.max(1)).div_ceil(8))
        .max(raw_len as u64)
        .max(1)
}

fn estimate_rgba_surface_bytes(width: u32, height: u32) -> u64 {
    u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(4)
        .max(1)
}

fn expand_render_tile(
    tile: RenderTile,
    page_width: u32,
    page_height: u32,
    overdraw: u32,
) -> RenderTile {
    let x = tile.x.saturating_sub(overdraw);
    let y = tile.y.saturating_sub(overdraw);
    let end_x = tile
        .x
        .saturating_add(tile.width)
        .saturating_add(overdraw)
        .min(page_width);
    let end_y = tile
        .y
        .saturating_add(tile.height)
        .saturating_add(overdraw)
        .min(page_height);
    RenderTile {
        x,
        y,
        width: end_x.saturating_sub(x),
        height: end_y.saturating_sub(y),
    }
}

fn crop_buffer(buf: &PixelBuffer, tile: RenderTile) -> Result<PixelBuffer> {
    if tile.width == 0 || tile.height == 0 {
        return Err(WellfriendError::invalid_input(
            "render crop must have non-zero width and height",
        ));
    }
    let end_x = tile.x.checked_add(tile.width).ok_or_else(|| {
        WellfriendError::invalid_input("render crop x range overflows".to_string())
    })?;
    let end_y = tile.y.checked_add(tile.height).ok_or_else(|| {
        WellfriendError::invalid_input("render crop y range overflows".to_string())
    })?;
    if end_x > buf.width || end_y > buf.height {
        return Err(WellfriendError::invalid_input(format!(
            "render crop {}x{} at {},{} exceeds buffer {}x{}",
            tile.width, tile.height, tile.x, tile.y, buf.width, buf.height
        )));
    }

    buf.copy_rect_to_new_buffer(tile.x, tile.y, tile.width, tile.height)
        .ok_or_else(|| WellfriendError::invalid_input("render tile crop failed".to_string()))
}

fn estimated_image_channels(color_space: &str, is_mask: bool) -> u64 {
    if is_mask {
        1
    } else if color_space.contains("CMYK") {
        4
    } else if color_space.contains("RGB") {
        3
    } else {
        1
    }
}

#[cfg(test)]
fn stitch_vertical_bands(bands: &[PixelBuffer], width: u32, height: u32) -> PixelBuffer {
    let mode = bands
        .first()
        .map(PixelBuffer::render_mode)
        .unwrap_or(RenderMode::Compat);
    let mut out = PixelBuffer::new_transparent_with_mode(width, height, mode);
    let mut y_offset = 0i32;
    for band in bands {
        for y in 0..band.height as i32 {
            for x in 0..band.width as i32 {
                out.set_pixel(x, y_offset + y, band.get_pixel(x, y));
            }
        }
        y_offset += band.height as i32;
    }
    out
}

fn colr_blend_to_pdf(mode: crate::render::color_glyph::ColrBlendMode) -> BlendMode {
    match mode {
        crate::render::color_glyph::ColrBlendMode::Normal => BlendMode::Normal,
        crate::render::color_glyph::ColrBlendMode::Clear
        | crate::render::color_glyph::ColrBlendMode::Source
        | crate::render::color_glyph::ColrBlendMode::Destination
        | crate::render::color_glyph::ColrBlendMode::DestinationOver
        | crate::render::color_glyph::ColrBlendMode::SourceIn
        | crate::render::color_glyph::ColrBlendMode::DestinationIn
        | crate::render::color_glyph::ColrBlendMode::SourceOut
        | crate::render::color_glyph::ColrBlendMode::DestinationOut
        | crate::render::color_glyph::ColrBlendMode::SourceAtop
        | crate::render::color_glyph::ColrBlendMode::DestinationAtop
        | crate::render::color_glyph::ColrBlendMode::Xor
        | crate::render::color_glyph::ColrBlendMode::Plus => BlendMode::Normal,
        crate::render::color_glyph::ColrBlendMode::Multiply => BlendMode::Multiply,
        crate::render::color_glyph::ColrBlendMode::Screen => BlendMode::Screen,
        crate::render::color_glyph::ColrBlendMode::Overlay => BlendMode::Overlay,
        crate::render::color_glyph::ColrBlendMode::Darken => BlendMode::Darken,
        crate::render::color_glyph::ColrBlendMode::Lighten => BlendMode::Lighten,
        crate::render::color_glyph::ColrBlendMode::ColorDodge => BlendMode::ColorDodge,
        crate::render::color_glyph::ColrBlendMode::ColorBurn => BlendMode::ColorBurn,
        crate::render::color_glyph::ColrBlendMode::HardLight => BlendMode::HardLight,
        crate::render::color_glyph::ColrBlendMode::SoftLight => BlendMode::SoftLight,
        crate::render::color_glyph::ColrBlendMode::Difference => BlendMode::Difference,
        crate::render::color_glyph::ColrBlendMode::Exclusion => BlendMode::Exclusion,
        crate::render::color_glyph::ColrBlendMode::Hue => BlendMode::Hue,
        crate::render::color_glyph::ColrBlendMode::Saturation => BlendMode::Saturation,
        crate::render::color_glyph::ColrBlendMode::Color => BlendMode::Color,
        crate::render::color_glyph::ColrBlendMode::Luminosity => BlendMode::Luminosity,
    }
}

fn colr_is_porter_duff(mode: crate::render::color_glyph::ColrBlendMode) -> bool {
    matches!(
        mode,
        crate::render::color_glyph::ColrBlendMode::Clear
            | crate::render::color_glyph::ColrBlendMode::Source
            | crate::render::color_glyph::ColrBlendMode::Destination
            | crate::render::color_glyph::ColrBlendMode::DestinationOver
            | crate::render::color_glyph::ColrBlendMode::SourceIn
            | crate::render::color_glyph::ColrBlendMode::DestinationIn
            | crate::render::color_glyph::ColrBlendMode::SourceOut
            | crate::render::color_glyph::ColrBlendMode::DestinationOut
            | crate::render::color_glyph::ColrBlendMode::SourceAtop
            | crate::render::color_glyph::ColrBlendMode::DestinationAtop
            | crate::render::color_glyph::ColrBlendMode::Xor
            | crate::render::color_glyph::ColrBlendMode::Plus
    )
}

fn composite_colr_porter_duff(
    dst: &mut PixelBuffer,
    src: &PixelBuffer,
    mode: crate::render::color_glyph::ColrBlendMode,
) {
    let w = dst.width.min(src.width) as i32;
    let h = dst.height.min(src.height) as i32;
    for y in 0..h {
        for x in 0..w {
            let src_pixel = src.get_pixel(x, y);
            if src_pixel[3] == 0 {
                continue;
            }
            let dst_pixel = dst.get_pixel(x, y);
            dst.set_pixel(
                x,
                y,
                composite_colr_porter_duff_pixel(src_pixel, dst_pixel, mode),
            );
        }
    }
}

fn composite_colr_porter_duff_pixel(
    src: PixelColor,
    dst: PixelColor,
    mode: crate::render::color_glyph::ColrBlendMode,
) -> PixelColor {
    let sa = f32::from(src[3]) / 255.0;
    let da = f32::from(dst[3]) / 255.0;
    let sc = [
        f32::from(src[0]) / 255.0,
        f32::from(src[1]) / 255.0,
        f32::from(src[2]) / 255.0,
    ];
    let dc = [
        f32::from(dst[0]) / 255.0,
        f32::from(dst[1]) / 255.0,
        f32::from(dst[2]) / 255.0,
    ];
    if mode == crate::render::color_glyph::ColrBlendMode::Plus {
        let out_a = (sa + da).min(1.0);
        let out_premul = [
            (sc[0] * sa + dc[0] * da).min(1.0),
            (sc[1] * sa + dc[1] * da).min(1.0),
            (sc[2] * sa + dc[2] * da).min(1.0),
        ];
        return colr_unpremultiply_to_pixel(out_premul, out_a);
    }

    let (src_factor, dst_factor) = match mode {
        crate::render::color_glyph::ColrBlendMode::Clear => (0.0, 0.0),
        crate::render::color_glyph::ColrBlendMode::Source => (1.0, 0.0),
        crate::render::color_glyph::ColrBlendMode::Destination => (0.0, 1.0),
        crate::render::color_glyph::ColrBlendMode::DestinationOver => (1.0 - da, 1.0),
        crate::render::color_glyph::ColrBlendMode::SourceIn => (da, 0.0),
        crate::render::color_glyph::ColrBlendMode::DestinationIn => (0.0, sa),
        crate::render::color_glyph::ColrBlendMode::SourceOut => (1.0 - da, 0.0),
        crate::render::color_glyph::ColrBlendMode::DestinationOut => (0.0, 1.0 - sa),
        crate::render::color_glyph::ColrBlendMode::SourceAtop => (da, 1.0 - sa),
        crate::render::color_glyph::ColrBlendMode::DestinationAtop => (1.0 - da, sa),
        crate::render::color_glyph::ColrBlendMode::Xor => (1.0 - da, 1.0 - sa),
        _ => (1.0, 1.0 - sa),
    };
    let out_a = sa * src_factor + da * dst_factor;
    let out_premul = [
        sc[0] * sa * src_factor + dc[0] * da * dst_factor,
        sc[1] * sa * src_factor + dc[1] * da * dst_factor,
        sc[2] * sa * src_factor + dc[2] * da * dst_factor,
    ];
    colr_unpremultiply_to_pixel(out_premul, out_a)
}

fn colr_unpremultiply_to_pixel(rgb_premul: [f32; 3], alpha: f32) -> PixelColor {
    let alpha = alpha.clamp(0.0, 1.0);
    if alpha <= 1e-6 {
        return [0, 0, 0, 0];
    }
    let to_byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    [
        to_byte((rgb_premul[0] / alpha).clamp(0.0, 1.0)),
        to_byte((rgb_premul[1] / alpha).clamp(0.0, 1.0)),
        to_byte((rgb_premul[2] / alpha).clamp(0.0, 1.0)),
        to_byte(alpha),
    ]
}

fn sample_colr_gradient(
    paint: &crate::render::color_glyph::ColrPaint,
    gx: f64,
    gy: f64,
) -> PixelColor {
    match paint {
        crate::render::color_glyph::ColrPaint::LinearGradient {
            x0,
            y0,
            x1,
            y1,
            x2,
            y2,
            extend,
            stops,
        } => {
            let _p2_finite = x2.is_finite() && y2.is_finite();
            let dx = x1 - x0;
            let dy = y1 - y0;
            let denom = dx * dx + dy * dy;
            let t = if denom <= 1e-9 {
                0.0
            } else {
                ((gx - x0) * dx + (gy - y0) * dy) / denom
            };
            sample_colr_stops(stops, normalize_colr_gradient_t(t, *extend))
        }
        crate::render::color_glyph::ColrPaint::RadialGradient {
            x0,
            y0,
            r0,
            x1,
            y1,
            r1,
            extend,
            stops,
        } => {
            let t = solve_colr_radial_t(
                ColrPoint { x: gx, y: gy },
                ColrCircle {
                    center: ColrPoint { x: *x0, y: *y0 },
                    radius: *r0,
                },
                ColrCircle {
                    center: ColrPoint { x: *x1, y: *y1 },
                    radius: *r1,
                },
            )
            .unwrap_or(0.0);
            sample_colr_stops(stops, normalize_colr_gradient_t(t, *extend))
        }
        crate::render::color_glyph::ColrPaint::SweepGradient {
            center_x,
            center_y,
            start_angle,
            end_angle,
            extend,
            stops,
        } => {
            let mut angle = (gy - center_y).atan2(gx - center_x).to_degrees();
            if angle < 0.0 {
                angle += 360.0;
            }
            let mut start = *start_angle;
            let mut end = *end_angle;
            if start < 0.0 {
                start %= 360.0;
            }
            if end <= start {
                end += 360.0;
            }
            if angle < start {
                angle += 360.0;
            }
            let denom = end - start;
            let t = if denom.abs() <= 1e-9 {
                0.0
            } else {
                (angle - start) / denom
            };
            sample_colr_stops(stops, normalize_colr_gradient_t(t, *extend))
        }
        crate::render::color_glyph::ColrPaint::Solid(color) => *color,
    }
}

#[derive(Clone, Copy)]
struct ColrPoint {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy)]
struct ColrCircle {
    center: ColrPoint,
    radius: f64,
}

fn solve_colr_radial_t(point: ColrPoint, start: ColrCircle, end: ColrCircle) -> Option<f64> {
    let ax = end.center.x - start.center.x;
    let ay = end.center.y - start.center.y;
    let ar = end.radius - start.radius;
    let dx = point.x - start.center.x;
    let dy = point.y - start.center.y;
    let aa = ax * ax + ay * ay - ar * ar;
    let bb = 2.0 * (dx * ax + dy * ay + start.radius * ar);
    let cc = dx * dx + dy * dy - start.radius * start.radius;
    if aa.abs() < 1e-10 {
        if bb.abs() < 1e-10 {
            return None;
        }
        return accept_colr_radial_t(cc / bb, ar, start.radius);
    }
    let disc = bb * bb - 4.0 * aa * cc;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let t_pos = (bb + sq) / (2.0 * aa);
    let t_neg = (bb - sq) / (2.0 * aa);
    accept_colr_radial_t(t_pos, ar, start.radius)
        .into_iter()
        .chain(accept_colr_radial_t(t_neg, ar, start.radius))
        .reduce(f64::max)
}

fn accept_colr_radial_t(t: f64, radius_delta: f64, start_radius: f64) -> Option<f64> {
    if !t.is_finite() {
        return None;
    }
    let radius = start_radius + t * radius_delta;
    if radius >= -1e-9 {
        Some(t)
    } else {
        None
    }
}

fn normalize_colr_gradient_t(
    t: f64,
    extend: crate::render::color_glyph::ColrGradientExtend,
) -> f64 {
    if !t.is_finite() {
        return 0.0;
    }
    match extend {
        crate::render::color_glyph::ColrGradientExtend::Pad => t.clamp(0.0, 1.0),
        crate::render::color_glyph::ColrGradientExtend::Repeat => t - t.floor(),
        crate::render::color_glyph::ColrGradientExtend::Reflect => {
            let whole = t.floor();
            let frac = t - whole;
            if (whole as i64).rem_euclid(2) == 0 {
                frac
            } else {
                1.0 - frac
            }
        }
    }
}

fn sample_colr_stops(stops: &[crate::render::color_glyph::ColrColorStop], t: f64) -> PixelColor {
    let Some(first) = stops.first() else {
        return [0, 0, 0, 0];
    };
    if t <= first.offset {
        return first.color;
    }
    for pair in stops.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if t <= b.offset {
            let span = (b.offset - a.offset).max(1e-9);
            let local = ((t - a.offset) / span).clamp(0.0, 1.0);
            return lerp_colr_color(a.color, b.color, local);
        }
    }
    stops.last().map(|stop| stop.color).unwrap_or(first.color)
}

fn lerp_colr_color(a: PixelColor, b: PixelColor, t: f64) -> PixelColor {
    let lerp = |ca: u8, cb: u8| -> u8 {
        (f64::from(ca) + (f64::from(cb) - f64::from(ca)) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [
        lerp(a[0], b[0]),
        lerp(a[1], b[1]),
        lerp(a[2], b[2]),
        lerp(a[3], b[3]),
    ]
}

/// Resolve a pattern/function object to its (dictionary, raw stream bytes).
/// Returns `None` if it is not a stream.
fn resolve_to_stream(
    obj: &PdfObject,
    reader: &crate::reader::PdfReader,
) -> Option<(PdfDictionary, Vec<u8>)> {
    match obj {
        PdfObject::Stream { dict, raw } => Some((dict.clone(), raw.clone())),
        PdfObject::Reference { number, generation } => {
            match reader.get_object(*number, *generation).ok()? {
                PdfObject::Stream { dict, raw } => Some((dict, raw)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Compute the integer device-space bounding box of a flattened path, clamped to
/// the buffer. Returns `(x0, y0, x1, y1)` inclusive; an empty/degenerate path
/// yields `x1 < x0`.
fn path_device_bounds(flat: &FlatPath, width: u32, height: u32) -> (i32, i32, i32, i32) {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for sub in &flat.subpaths {
        for &(x, y) in sub {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
    }
    if minx > maxx || miny > maxy {
        return (1, 1, 0, 0); // empty
    }
    let x0 = minx.floor().max(0.0) as i32;
    let y0 = miny.floor().max(0.0) as i32;
    let x1 = (maxx.ceil() as i32).min(width as i32 - 1);
    let y1 = (maxy.ceil() as i32).min(height as i32 - 1);
    (x0, y0, x1, y1)
}

/// Read a numeric array entry from a dictionary, e.g. a pattern `/Matrix`.
fn get_float_array_dict(dict: &PdfDictionary, key: &str) -> Option<Vec<f64>> {
    let arr = dict.get(key)?.as_array()?;
    let vals: Vec<f64> = arr.iter().filter_map(PdfObject::as_number).collect();
    if vals.is_empty() {
        None
    } else {
        Some(vals)
    }
}

fn annotation_is_hidden_or_no_view(dict: &PdfDictionary) -> bool {
    let flags = dict.get_integer("F").unwrap_or(0);
    const INVISIBLE: i64 = 1 << 0;
    const HIDDEN: i64 = 1 << 1;
    const NO_VIEW: i64 = 1 << 5;
    flags & (INVISIBLE | HIDDEN | NO_VIEW) != 0
}

fn select_annotation_appearance(
    annot: &PdfDictionary,
    reader: &crate::reader::PdfReader,
) -> Option<(PdfDictionary, Vec<u8>)> {
    let ap = annot.get("AP")?.clone();
    let ap = match reader.resolve(ap).ok()? {
        PdfObject::Dictionary(dict) => dict,
        _ => return None,
    };
    let normal = ap.get("N")?.clone();
    match reader.resolve(normal).ok()? {
        PdfObject::Stream { dict, raw } => Some((dict, raw)),
        PdfObject::Dictionary(states) => {
            let state_name = annot.get_name("AS").unwrap_or("Off");
            if let Some(selected) = states.get(state_name) {
                return resolve_appearance_stream(selected, reader);
            }
            if state_name != "Off" {
                if let Some(off) = states.get("Off") {
                    return resolve_appearance_stream(off, reader);
                }
            }
            states
                .entries()
                .find(|(name, _)| name.as_str() != "Off")
                .and_then(|(_, value)| resolve_appearance_stream(value, reader))
        }
        _ => None,
    }
}

const FIELD_FLAG_MULTILINE: i64 = 1 << 12;
const FIELD_FLAG_RADIO: i64 = 1 << 15;
const FIELD_FLAG_PUSHBUTTON: i64 = 1 << 16;
const FIELD_FLAG_COMBO: i64 = 1 << 17;

#[derive(Clone, Copy, Debug)]
struct DefaultAppearance {
    font_size: f64,
    color: (f64, f64, f64),
}

impl Default for DefaultAppearance {
    fn default() -> Self {
        Self {
            font_size: 10.0,
            color: (0.0, 0.0, 0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ButtonAppearanceKind {
    Checkbox,
    Radio,
    PushButton,
}

#[derive(Clone, Copy)]
struct WidgetAppearanceBox<'a> {
    width: f64,
    height: f64,
    mk: Option<&'a PdfDictionary>,
}

#[derive(Clone, Copy)]
struct TextFieldAppearance {
    default_appearance: DefaultAppearance,
    alignment: i64,
    multiline: bool,
}

struct ButtonAppearance<'a> {
    kind: ButtonAppearanceKind,
    selected: bool,
    caption: &'a str,
    caption_bytes: &'a [u8],
    default_appearance: DefaultAppearance,
}

impl<'a> WidgetAppearanceBox<'a> {
    fn new(width: f64, height: f64, mk: Option<&'a PdfDictionary>) -> Self {
        Self { width, height, mk }
    }
}

fn synthesize_annotation_appearance(
    annot: &PdfDictionary,
    reader: &crate::reader::PdfReader,
    engine: &ContentEngine,
    rect: [f64; 4],
) -> Option<(PdfDictionary, Vec<u8>)> {
    let width = (rect[2] - rect[0]).abs();
    let height = (rect[3] - rect[1]).abs();
    if width <= 0.0 || height <= 0.0 {
        return None;
    }

    if annot.get_name("Subtype") != Some("Widget") {
        if let Some(appearance) =
            synthesize_markup_annotation_appearance(annot, rect, width, height)
        {
            return Some(appearance);
        }
    }

    let field_chain = collect_field_chain(annot, reader);
    let field_type_obj = inherited_field_object(&field_chain, "FT")?;
    let field_type = field_type_obj.as_name()?.to_string();
    let acroform = resolve_acroform_dict(engine, reader);
    let need_appearances = acroform
        .as_ref()
        .and_then(|dict| dict.get_bool("NeedAppearances"))
        .unwrap_or(false);
    let field_flags = inherited_field_integer(&field_chain, "Ff").unwrap_or(0);
    let alignment = inherited_field_integer(&field_chain, "Q")
        .or_else(|| acroform.as_ref().and_then(|dict| dict.get_integer("Q")))
        .unwrap_or(0);
    let default_appearance = inherited_field_object(&field_chain, "DA")
        .and_then(|obj| object_string_bytes(&obj))
        .or_else(|| {
            acroform
                .as_ref()
                .and_then(|dict| dict.get("DA"))
                .and_then(object_string_bytes)
        })
        .as_deref()
        .map(parse_default_appearance)
        .unwrap_or_default();
    let value = inherited_field_object(&field_chain, "V");
    let options = inherited_field_object(&field_chain, "Opt");
    let mk = annot.get_dict("MK").cloned();

    let mut content = String::new();
    match field_type.as_str() {
        "Tx" => {
            let (text, text_bytes) = value.as_ref().and_then(display_text_from_object)?;
            if text.is_empty() {
                return None;
            }
            append_text_field_appearance(
                &mut content,
                WidgetAppearanceBox::new(width, height, mk.as_ref()),
                &text,
                &text_bytes,
                TextFieldAppearance {
                    default_appearance,
                    alignment,
                    multiline: field_flags & FIELD_FLAG_MULTILINE != 0,
                },
            );
        }
        "Btn" => {
            let kind = if field_flags & FIELD_FLAG_PUSHBUTTON != 0 {
                ButtonAppearanceKind::PushButton
            } else if field_flags & FIELD_FLAG_RADIO != 0 {
                ButtonAppearanceKind::Radio
            } else {
                ButtonAppearanceKind::Checkbox
            };
            let selected = button_is_selected(kind, annot, value.as_ref());
            let caption = mk
                .as_ref()
                .and_then(|dict| dict.get("CA"))
                .and_then(display_text_from_object)
                .unwrap_or_else(|| (String::new(), Vec::new()));
            if kind == ButtonAppearanceKind::PushButton && caption.0.is_empty() {
                return None;
            }
            if kind != ButtonAppearanceKind::PushButton && caption.0.chars().count() > 1 {
                return None;
            }
            if kind != ButtonAppearanceKind::PushButton && !selected {
                return None;
            }
            let has_explicit_button_chrome = mk
                .as_ref()
                .map(|dict| {
                    dict.contains_key("CA") || dict.contains_key("BG") || dict.contains_key("BC")
                })
                .unwrap_or(false);
            if kind != ButtonAppearanceKind::PushButton
                && !need_appearances
                && !has_explicit_button_chrome
            {
                return None;
            }
            if kind == ButtonAppearanceKind::Checkbox && selected {
                if let Some(label) = checkbox_label_state(annot, value.as_ref()) {
                    append_text_field_appearance(
                        &mut content,
                        WidgetAppearanceBox::new(width, height, mk.as_ref()),
                        &label,
                        label.as_bytes(),
                        TextFieldAppearance {
                            default_appearance,
                            alignment,
                            multiline: false,
                        },
                    );
                } else {
                    append_button_appearance(
                        &mut content,
                        WidgetAppearanceBox::new(width, height, mk.as_ref()),
                        ButtonAppearance {
                            kind,
                            selected,
                            caption: &caption.0,
                            caption_bytes: &caption.1,
                            default_appearance,
                        },
                    );
                }
            } else {
                append_button_appearance(
                    &mut content,
                    WidgetAppearanceBox::new(width, height, mk.as_ref()),
                    ButtonAppearance {
                        kind,
                        selected,
                        caption: &caption.0,
                        caption_bytes: &caption.1,
                        default_appearance,
                    },
                );
            }
        }
        "Ch" => {
            let selected = choice_display_text(value.as_ref(), options.as_ref())?;
            if selected.0.is_empty() {
                return None;
            }
            append_text_field_appearance(
                &mut content,
                WidgetAppearanceBox::new(width, height, mk.as_ref()),
                &selected.0,
                &selected.1,
                TextFieldAppearance {
                    default_appearance,
                    alignment,
                    multiline: field_flags & FIELD_FLAG_COMBO == 0,
                },
            );
        }
        _ => return None,
    }

    if content.is_empty() {
        return None;
    }

    let mut form = synthesized_appearance_form_dict(width, height);
    form.insert("Length", PdfObject::Integer(content.len() as i64));
    Some((form, content.into_bytes()))
}

fn synthesize_markup_annotation_appearance(
    annot: &PdfDictionary,
    rect: [f64; 4],
    width: f64,
    height: f64,
) -> Option<(PdfDictionary, Vec<u8>)> {
    let subtype = annot.get_name("Subtype")?;
    let color = annotation_rgb(annot).unwrap_or(match subtype {
        "Highlight" => (1.0, 1.0, 0.0),
        _ => (0.0, 0.0, 0.0),
    });
    let opacity = annotation_opacity(annot, if subtype == "Highlight" { 0.35 } else { 1.0 });
    let mut content = String::new();

    match subtype {
        "Highlight" => {
            for bounds in annotation_local_quad_bounds(annot, rect, width, height) {
                append_annotation_rect_fill(&mut content, bounds, color);
            }
        }
        "Underline" | "StrikeOut" | "Squiggly" => {
            for bounds in annotation_local_quad_bounds(annot, rect, width, height) {
                append_text_markup_line(&mut content, subtype, bounds, color);
            }
        }
        "Square" => {
            let _ = writeln!(
                content,
                "q /GS1 gs {} {} {} RG 1 w 0.5 0.5 {} {} re S Q",
                pdf_num(color.0),
                pdf_num(color.1),
                pdf_num(color.2),
                pdf_num((width - 1.0).max(0.0)),
                pdf_num((height - 1.0).max(0.0))
            );
        }
        "Circle" => {
            append_circle(
                &mut content,
                width * 0.5,
                height * 0.5,
                ((width.min(height) - 1.0) * 0.5).max(0.0),
                color,
                false,
            );
        }
        "Line" => {
            let line = annot.get_array("L")?;
            if line.len() < 4 {
                return None;
            }
            let x0 = line[0].as_number()? - rect[0].min(rect[2]);
            let y0 = line[1].as_number()? - rect[1].min(rect[3]);
            let x1 = line[2].as_number()? - rect[0].min(rect[2]);
            let y1 = line[3].as_number()? - rect[1].min(rect[3]);
            let _ = writeln!(
                content,
                "q /GS1 gs {} {} {} RG 1.5 w {} {} m {} {} l S Q",
                pdf_num(color.0),
                pdf_num(color.1),
                pdf_num(color.2),
                pdf_num(x0),
                pdf_num(y0),
                pdf_num(x1),
                pdf_num(y1)
            );
        }
        "Ink" => append_ink_annotation_paths(&mut content, annot, rect, color)?,
        "FreeText" => {
            let (text, text_bytes) = annot.get("Contents").and_then(display_text_from_object)?;
            append_text_field_appearance(
                &mut content,
                WidgetAppearanceBox::new(width, height, None),
                &text,
                &text_bytes,
                TextFieldAppearance {
                    default_appearance: DefaultAppearance {
                        font_size: 10.0,
                        color,
                    },
                    alignment: annot.get_integer("Q").unwrap_or(0),
                    multiline: true,
                },
            );
        }
        _ => return None,
    }

    if content.is_empty() {
        return None;
    }
    let mut form = synthesized_appearance_form_dict(width, height);
    add_synthesized_ext_g_state(
        &mut form,
        opacity,
        if subtype == "Highlight" {
            Some("Multiply")
        } else {
            None
        },
    );
    form.insert("Length", PdfObject::Integer(content.len() as i64));
    Some((form, content.into_bytes()))
}

fn add_synthesized_ext_g_state(form: &mut PdfDictionary, alpha: f64, blend_mode: Option<&str>) {
    let mut gs = PdfDictionary::empty();
    gs.insert("Type", PdfObject::Name("ExtGState".to_string()));
    gs.insert("ca", PdfObject::Real(alpha.clamp(0.0, 1.0)));
    gs.insert("CA", PdfObject::Real(alpha.clamp(0.0, 1.0)));
    if let Some(mode) = blend_mode {
        gs.insert("BM", PdfObject::Name(mode.to_string()));
    }
    let mut ext = PdfDictionary::empty();
    ext.insert("GS1", PdfObject::Dictionary(gs));
    if let Some(PdfObject::Dictionary(resources)) = form.get_mut("Resources") {
        resources.insert("ExtGState", PdfObject::Dictionary(ext));
    }
}

fn annotation_rgb(annot: &PdfDictionary) -> Option<(f64, f64, f64)> {
    let arr = annot.get_array("C")?;
    match arr.len() {
        1 => {
            let gray = clamp_unit(arr[0].as_number()?);
            Some((gray, gray, gray))
        }
        3 => Some((
            clamp_unit(arr[0].as_number()?),
            clamp_unit(arr[1].as_number()?),
            clamp_unit(arr[2].as_number()?),
        )),
        4 => Some(cmyk_to_rgb(
            arr[0].as_number()?,
            arr[1].as_number()?,
            arr[2].as_number()?,
            arr[3].as_number()?,
        )),
        _ => None,
    }
}

fn annotation_opacity(annot: &PdfDictionary, default_alpha: f64) -> f64 {
    annot
        .get("CA")
        .and_then(PdfObject::as_number)
        .unwrap_or(default_alpha)
        .clamp(0.0, 1.0)
}

fn annotation_local_quad_bounds(
    annot: &PdfDictionary,
    rect: [f64; 4],
    width: f64,
    height: f64,
) -> Vec<[f64; 4]> {
    let rect_x0 = rect[0].min(rect[2]);
    let rect_y0 = rect[1].min(rect[3]);
    let Some(quads) = annot.get_array("QuadPoints") else {
        return vec![[0.0, 0.0, width, height]];
    };
    let mut bounds = Vec::new();
    for quad in quads.chunks(8) {
        if quad.len() < 8 {
            continue;
        }
        let coords: Vec<f64> = quad.iter().filter_map(PdfObject::as_number).collect();
        if coords.len() < 8 {
            continue;
        }
        let xs = [coords[0], coords[2], coords[4], coords[6]];
        let ys = [coords[1], coords[3], coords[5], coords[7]];
        let x0 = xs.iter().copied().fold(f64::INFINITY, f64::min) - rect_x0;
        let x1 = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max) - rect_x0;
        let y0 = ys.iter().copied().fold(f64::INFINITY, f64::min) - rect_y0;
        let y1 = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max) - rect_y0;
        if x1 > x0 && y1 > y0 {
            bounds.push([
                x0.clamp(0.0, width),
                y0.clamp(0.0, height),
                x1.clamp(0.0, width),
                y1.clamp(0.0, height),
            ]);
        }
    }
    if bounds.is_empty() {
        vec![[0.0, 0.0, width, height]]
    } else {
        bounds
    }
}

fn append_annotation_rect_fill(content: &mut String, bounds: [f64; 4], color: (f64, f64, f64)) {
    let x = bounds[0].min(bounds[2]);
    let y = bounds[1].min(bounds[3]);
    let w = (bounds[2] - bounds[0]).abs();
    let h = (bounds[3] - bounds[1]).abs();
    let _ = writeln!(
        content,
        "q /GS1 gs {} {} {} rg {} {} {} {} re f Q",
        pdf_num(color.0),
        pdf_num(color.1),
        pdf_num(color.2),
        pdf_num(x),
        pdf_num(y),
        pdf_num(w),
        pdf_num(h)
    );
}

fn append_text_markup_line(
    content: &mut String,
    subtype: &str,
    bounds: [f64; 4],
    color: (f64, f64, f64),
) {
    let x0 = bounds[0].min(bounds[2]);
    let x1 = bounds[0].max(bounds[2]);
    let y0 = bounds[1].min(bounds[3]);
    let y1 = bounds[1].max(bounds[3]);
    let base_y = if subtype == "StrikeOut" {
        y0 + (y1 - y0) * 0.5
    } else {
        y0 + (y1 - y0) * 0.12
    };
    if subtype == "Squiggly" {
        let step = 4.0_f64.max((y1 - y0) * 0.2);
        let amp = ((y1 - y0) * 0.08).clamp(0.75, 2.0);
        let _ = write!(
            content,
            "q /GS1 gs {} {} {} RG 1 w {} {} m",
            pdf_num(color.0),
            pdf_num(color.1),
            pdf_num(color.2),
            pdf_num(x0),
            pdf_num(base_y)
        );
        let mut x = x0 + step;
        let mut up = true;
        while x <= x1 {
            let y = if up { base_y + amp } else { base_y - amp };
            let _ = write!(content, " {} {} l", pdf_num(x), pdf_num(y));
            x += step;
            up = !up;
        }
        let _ = writeln!(content, " S Q");
    } else {
        let _ = writeln!(
            content,
            "q /GS1 gs {} {} {} RG 1 w {} {} m {} {} l S Q",
            pdf_num(color.0),
            pdf_num(color.1),
            pdf_num(color.2),
            pdf_num(x0),
            pdf_num(base_y),
            pdf_num(x1),
            pdf_num(base_y)
        );
    }
}

fn append_ink_annotation_paths(
    content: &mut String,
    annot: &PdfDictionary,
    rect: [f64; 4],
    color: (f64, f64, f64),
) -> Option<()> {
    let rect_x0 = rect[0].min(rect[2]);
    let rect_y0 = rect[1].min(rect[3]);
    let lists = annot.get_array("InkList")?;
    let _ = write!(
        content,
        "q /GS1 gs {} {} {} RG 1.5 w",
        pdf_num(color.0),
        pdf_num(color.1),
        pdf_num(color.2)
    );
    let mut wrote = false;
    for list in lists {
        let Some(points) = list.as_array() else {
            continue;
        };
        let coords: Vec<f64> = points.iter().filter_map(PdfObject::as_number).collect();
        if coords.len() < 4 {
            continue;
        }
        let _ = write!(
            content,
            " {} {} m",
            pdf_num(coords[0] - rect_x0),
            pdf_num(coords[1] - rect_y0)
        );
        for pair in coords[2..].chunks(2) {
            if pair.len() == 2 {
                let _ = write!(
                    content,
                    " {} {} l",
                    pdf_num(pair[0] - rect_x0),
                    pdf_num(pair[1] - rect_y0)
                );
            }
        }
        wrote = true;
    }
    if wrote {
        let _ = writeln!(content, " S Q");
        Some(())
    } else {
        None
    }
}

fn collect_field_chain(
    annot: &PdfDictionary,
    reader: &crate::reader::PdfReader,
) -> Vec<PdfDictionary> {
    let mut chain = vec![annot.clone()];
    let mut parent = annot.get("Parent").cloned();
    for _ in 0..16 {
        let Some(parent_obj) = parent else {
            break;
        };
        let Ok(PdfObject::Dictionary(parent_dict)) = reader.resolve(parent_obj) else {
            break;
        };
        parent = parent_dict.get("Parent").cloned();
        chain.push(parent_dict);
    }
    chain
}

fn inherited_field_object(chain: &[PdfDictionary], key: &str) -> Option<PdfObject> {
    chain.iter().find_map(|dict| dict.get(key).cloned())
}

fn inherited_field_integer(chain: &[PdfDictionary], key: &str) -> Option<i64> {
    chain.iter().find_map(|dict| dict.get_integer(key))
}

fn resolve_acroform_dict(
    engine: &ContentEngine,
    reader: &crate::reader::PdfReader,
) -> Option<PdfDictionary> {
    let catalog = engine.document().get_catalog().ok()?;
    let acroform = catalog.get("AcroForm")?.clone();
    match reader.resolve(acroform).ok()? {
        PdfObject::Dictionary(dict) => Some(dict),
        _ => None,
    }
}

fn object_string_bytes(obj: &PdfObject) -> Option<Vec<u8>> {
    obj.as_string().map(|bytes| bytes.to_vec())
}

fn parse_default_appearance(bytes: &[u8]) -> DefaultAppearance {
    let mut appearance = DefaultAppearance::default();
    let Ok(operations) = crate::content::ContentParser::parse(bytes) else {
        return appearance;
    };
    for op in operations {
        match op.operator.as_str() {
            "Tf" => {
                if let Some(size) = op.number(1) {
                    appearance.font_size = size;
                }
            }
            "g" => {
                if let Some(gray) = op.number(0) {
                    let gray = clamp_unit(gray);
                    appearance.color = (gray, gray, gray);
                }
            }
            "rg" => {
                if let (Some(r), Some(g), Some(b)) = (op.number(0), op.number(1), op.number(2)) {
                    appearance.color = (clamp_unit(r), clamp_unit(g), clamp_unit(b));
                }
            }
            "k" => {
                if let (Some(c), Some(m), Some(y), Some(k)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    appearance.color = cmyk_to_rgb(c, m, y, k);
                }
            }
            _ => {}
        }
    }
    appearance
}

fn synthesized_appearance_form_dict(width: f64, height: f64) -> PdfDictionary {
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".to_string()));
    font.insert("Subtype", PdfObject::Name("Type1".to_string()));
    font.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));

    let mut fonts = PdfDictionary::empty();
    fonts.insert("F1", PdfObject::Dictionary(font));

    let mut resources = PdfDictionary::empty();
    resources.insert("Font", PdfObject::Dictionary(fonts));

    let mut form = PdfDictionary::empty();
    form.insert("Type", PdfObject::Name("XObject".to_string()));
    form.insert("Subtype", PdfObject::Name("Form".to_string()));
    form.insert(
        "BBox",
        PdfObject::Array(vec![
            PdfObject::Real(0.0),
            PdfObject::Real(0.0),
            PdfObject::Real(width),
            PdfObject::Real(height),
        ]),
    );
    form.insert("Resources", PdfObject::Dictionary(resources));
    form
}

fn append_text_field_appearance(
    content: &mut String,
    geometry: WidgetAppearanceBox<'_>,
    text: &str,
    text_bytes: &[u8],
    appearance: TextFieldAppearance,
) {
    append_explicit_widget_chrome(content, geometry.width, geometry.height, geometry.mk);
    if text.is_empty() {
        return;
    }

    let font_size = effective_font_size(
        appearance.default_appearance.font_size,
        text,
        geometry.width,
        geometry.height,
        appearance.multiline,
    );
    if appearance.multiline {
        let line_height = font_size * 1.2;
        let max_lines = ((geometry.height - 6.0).max(font_size) / line_height)
            .floor()
            .max(1.0) as usize;
        let mut y = (geometry.height - font_size - 3.0).max(2.0);
        for line in text.lines().take(max_lines) {
            append_text_run(
                content,
                line.as_bytes(),
                font_size,
                appearance.default_appearance.color,
                text_x_for_alignment(line, font_size, geometry.width, appearance.alignment),
                y,
            );
            y -= line_height;
            if y < 2.0 {
                break;
            }
        }
    } else {
        let literal_bytes = if text_bytes.is_empty() {
            text.as_bytes()
        } else {
            text_bytes
        };
        let y = ((geometry.height - font_size) * 0.5).max(2.0);
        append_text_run(
            content,
            literal_bytes,
            font_size,
            appearance.default_appearance.color,
            text_x_for_alignment(text, font_size, geometry.width, appearance.alignment),
            y,
        );
    }
}

fn append_button_appearance(
    content: &mut String,
    geometry: WidgetAppearanceBox<'_>,
    appearance: ButtonAppearance<'_>,
) {
    match appearance.kind {
        ButtonAppearanceKind::Checkbox => {
            append_widget_background_and_border(
                content,
                geometry.width,
                geometry.height,
                geometry.mk,
                (1.0, 1.0, 1.0),
            );
            if appearance.selected {
                let stroke = (geometry.width.min(geometry.height) * 0.09).clamp(1.2, 3.0);
                let x1 = geometry.width * 0.22;
                let y1 = geometry.height * 0.50;
                let x2 = geometry.width * 0.42;
                let y2 = geometry.height * 0.28;
                let x3 = geometry.width * 0.80;
                let y3 = geometry.height * 0.76;
                let _ = writeln!(
                    content,
                    "q 0 0 0 RG {} w {} {} m {} {} l {} {} l S Q",
                    pdf_num(stroke),
                    pdf_num(x1),
                    pdf_num(y1),
                    pdf_num(x2),
                    pdf_num(y2),
                    pdf_num(x3),
                    pdf_num(y3)
                );
            }
        }
        ButtonAppearanceKind::Radio => {
            append_widget_background(
                content,
                geometry.width,
                geometry.height,
                geometry.mk,
                (1.0, 1.0, 1.0),
            );
            let border = mk_rgb(geometry.mk, "BC").unwrap_or((0.0, 0.0, 0.0));
            append_circle(
                content,
                geometry.width * 0.5,
                geometry.height * 0.5,
                geometry.width.min(geometry.height) * 0.42,
                border,
                false,
            );
            if appearance.selected {
                append_circle(
                    content,
                    geometry.width * 0.5,
                    geometry.height * 0.5,
                    geometry.width.min(geometry.height) * 0.20,
                    border,
                    true,
                );
            }
        }
        ButtonAppearanceKind::PushButton => {
            append_widget_background_and_border(
                content,
                geometry.width,
                geometry.height,
                geometry.mk,
                (0.92, 0.92, 0.92),
            );
            if !appearance.caption.is_empty() {
                let font_size = effective_font_size(
                    appearance.default_appearance.font_size,
                    appearance.caption,
                    geometry.width,
                    geometry.height,
                    false,
                );
                let literal_bytes = if appearance.caption_bytes.is_empty() {
                    appearance.caption.as_bytes()
                } else {
                    appearance.caption_bytes
                };
                append_text_run(
                    content,
                    literal_bytes,
                    font_size,
                    appearance.default_appearance.color,
                    text_x_for_alignment(appearance.caption, font_size, geometry.width, 1),
                    ((geometry.height - font_size) * 0.5).max(2.0),
                );
            }
        }
    }
}

fn append_widget_background_and_border(
    content: &mut String,
    width: f64,
    height: f64,
    mk: Option<&PdfDictionary>,
    default_bg: (f64, f64, f64),
) {
    append_widget_background(content, width, height, mk, default_bg);
    let border = mk_rgb(mk, "BC").unwrap_or((0.0, 0.0, 0.0));
    let _ = writeln!(
        content,
        "q {} {} {} RG 1 w 0.5 0.5 {} {} re S Q",
        pdf_num(border.0),
        pdf_num(border.1),
        pdf_num(border.2),
        pdf_num((width - 1.0).max(0.0)),
        pdf_num((height - 1.0).max(0.0))
    );
}

fn append_explicit_widget_chrome(
    content: &mut String,
    width: f64,
    height: f64,
    mk: Option<&PdfDictionary>,
) {
    if let Some(bg) = mk_rgb(mk, "BG") {
        let _ = writeln!(
            content,
            "q {} {} {} rg 0 0 {} {} re f Q",
            pdf_num(bg.0),
            pdf_num(bg.1),
            pdf_num(bg.2),
            pdf_num(width),
            pdf_num(height)
        );
    }
    if let Some(border) = mk_rgb(mk, "BC") {
        let _ = writeln!(
            content,
            "q {} {} {} RG 1 w 0.5 0.5 {} {} re S Q",
            pdf_num(border.0),
            pdf_num(border.1),
            pdf_num(border.2),
            pdf_num((width - 1.0).max(0.0)),
            pdf_num((height - 1.0).max(0.0))
        );
    }
}

fn append_widget_background(
    content: &mut String,
    width: f64,
    height: f64,
    mk: Option<&PdfDictionary>,
    default_bg: (f64, f64, f64),
) {
    let bg = mk_rgb(mk, "BG").unwrap_or(default_bg);
    let _ = writeln!(
        content,
        "q {} {} {} rg 0 0 {} {} re f Q",
        pdf_num(bg.0),
        pdf_num(bg.1),
        pdf_num(bg.2),
        pdf_num(width),
        pdf_num(height)
    );
}

fn append_text_run(
    content: &mut String,
    bytes: &[u8],
    font_size: f64,
    color: (f64, f64, f64),
    x: f64,
    y: f64,
) {
    let _ = writeln!(
        content,
        "q BT /F1 {} Tf {} {} {} rg 1 0 0 1 {} {} Tm {} Tj ET Q",
        pdf_num(font_size),
        pdf_num(color.0),
        pdf_num(color.1),
        pdf_num(color.2),
        pdf_num(x),
        pdf_num(y),
        pdf_literal_bytes(bytes)
    );
}

fn append_circle(
    content: &mut String,
    cx: f64,
    cy: f64,
    radius: f64,
    color: (f64, f64, f64),
    fill: bool,
) {
    let k = radius * 0.552_284_749_830_793_6;
    let op = if fill { "f" } else { "S" };
    let color_op = if fill { "rg" } else { "RG" };
    let _ = writeln!(
        content,
        "q {} {} {} {} 1 w {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h {} Q",
        pdf_num(color.0),
        pdf_num(color.1),
        pdf_num(color.2),
        color_op,
        pdf_num(cx + radius),
        pdf_num(cy),
        pdf_num(cx + radius),
        pdf_num(cy + k),
        pdf_num(cx + k),
        pdf_num(cy + radius),
        pdf_num(cx),
        pdf_num(cy + radius),
        pdf_num(cx - k),
        pdf_num(cy + radius),
        pdf_num(cx - radius),
        pdf_num(cy + k),
        pdf_num(cx - radius),
        pdf_num(cy),
        pdf_num(cx - radius),
        pdf_num(cy - k),
        pdf_num(cx - k),
        pdf_num(cy - radius),
        pdf_num(cx),
        pdf_num(cy - radius),
        pdf_num(cx + k),
        pdf_num(cy - radius),
        pdf_num(cx + radius),
        pdf_num(cy - k),
        pdf_num(cx + radius),
        pdf_num(cy),
        op
    );
}

fn effective_font_size(
    requested: f64,
    text: &str,
    width: f64,
    height: f64,
    multiline: bool,
) -> f64 {
    let mut size = if requested > 0.0 {
        requested
    } else {
        (height * 0.55).clamp(4.0, 12.0)
    };
    size = size.min((height - 4.0).max(4.0));
    if !multiline {
        let available = (width - 6.0).max(1.0);
        while approximate_text_width(text, size) > available && size > 4.0 {
            size -= 0.5;
        }
    }
    size.max(4.0)
}

fn text_x_for_alignment(text: &str, font_size: f64, width: f64, alignment: i64) -> f64 {
    let padding = 3.0;
    let text_width = approximate_text_width(text, font_size);
    match alignment {
        1 => ((width - text_width) * 0.5).max(padding),
        2 => (width - text_width - padding).max(padding),
        _ => padding,
    }
}

fn approximate_text_width(text: &str, font_size: f64) -> f64 {
    text.chars().count() as f64 * font_size * 0.52
}

fn button_is_selected(
    kind: ButtonAppearanceKind,
    annot: &PdfDictionary,
    value: Option<&PdfObject>,
) -> bool {
    let appearance_state = annot
        .get_name("AS")
        .filter(|state| *state != "Off")
        .map(str::to_string);
    let value_state = value.and_then(object_state_name);
    match kind {
        ButtonAppearanceKind::PushButton => false,
        ButtonAppearanceKind::Checkbox => {
            appearance_state.is_some() || value_state_is_on(value_state)
        }
        ButtonAppearanceKind::Radio => match (appearance_state, value_state) {
            (Some(appearance), Some(value)) => appearance == value,
            (Some(_), None) => true,
            (None, Some(value)) => value != "Off",
            (None, None) => false,
        },
    }
}

fn checkbox_label_state(annot: &PdfDictionary, value: Option<&PdfObject>) -> Option<String> {
    let state = annot
        .get_name("AS")
        .filter(|state| *state != "Off")
        .map(str::to_string)
        .or_else(|| value.and_then(object_state_name))?;
    if is_label_like_button_state(&state) {
        Some(state)
    } else {
        None
    }
}

fn is_label_like_button_state(state: &str) -> bool {
    let normalized = state.trim();
    if matches!(normalized, "" | "Off" | "On" | "Yes" | "1") {
        return false;
    }
    normalized.chars().count() > 1 && normalized.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn value_state_is_on(value: Option<String>) -> bool {
    value
        .map(|value| !value.is_empty() && value != "Off")
        .unwrap_or(false)
}

fn object_state_name(obj: &PdfObject) -> Option<String> {
    match obj {
        PdfObject::Name(name) => Some(name.clone()),
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        _ => None,
    }
}

fn display_text_from_object(obj: &PdfObject) -> Option<(String, Vec<u8>)> {
    match obj {
        PdfObject::String(bytes) => Some((decode_pdf_text_string(bytes), bytes.clone())),
        PdfObject::Name(name) => {
            if name == "Off" {
                None
            } else {
                Some((name.clone(), name.as_bytes().to_vec()))
            }
        }
        PdfObject::Integer(value) => {
            let text = value.to_string();
            Some((text.clone(), text.into_bytes()))
        }
        PdfObject::Real(value) => {
            let text = pdf_num(*value);
            Some((text.clone(), text.into_bytes()))
        }
        PdfObject::Array(items) => {
            let values: Vec<(String, Vec<u8>)> =
                items.iter().filter_map(display_text_from_object).collect();
            if values.is_empty() {
                None
            } else {
                let text = values
                    .iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                Some((text.clone(), text.into_bytes()))
            }
        }
        _ => None,
    }
}

fn choice_display_text(
    value: Option<&PdfObject>,
    options: Option<&PdfObject>,
) -> Option<(String, Vec<u8>)> {
    value
        .and_then(display_text_from_object)
        .or_else(|| first_option_display_text(options?))
}

fn first_option_display_text(options: &PdfObject) -> Option<(String, Vec<u8>)> {
    let PdfObject::Array(items) = options else {
        return None;
    };
    items.iter().find_map(option_display_text)
}

fn option_display_text(option: &PdfObject) -> Option<(String, Vec<u8>)> {
    match option {
        PdfObject::Array(items) => items
            .get(1)
            .or_else(|| items.first())
            .and_then(display_text_from_object),
        other => display_text_from_object(other),
    }
}

fn mk_rgb(mk: Option<&PdfDictionary>, key: &str) -> Option<(f64, f64, f64)> {
    let arr = mk?.get_array(key)?;
    match arr.len() {
        1 => {
            let gray = clamp_unit(arr[0].as_number()?);
            Some((gray, gray, gray))
        }
        3 => Some((
            clamp_unit(arr[0].as_number()?),
            clamp_unit(arr[1].as_number()?),
            clamp_unit(arr[2].as_number()?),
        )),
        4 => Some(cmyk_to_rgb(
            arr[0].as_number()?,
            arr[1].as_number()?,
            arr[2].as_number()?,
            arr[3].as_number()?,
        )),
        _ => None,
    }
}

fn cmyk_to_rgb(c: f64, m: f64, y: f64, k: f64) -> (f64, f64, f64) {
    let c = clamp_unit(c);
    let m = clamp_unit(m);
    let y = clamp_unit(y);
    let k = clamp_unit(k);
    (
        clamp_unit((1.0 - c) * (1.0 - k)),
        clamp_unit((1.0 - m) * (1.0 - k)),
        clamp_unit((1.0 - y) * (1.0 - k)),
    )
}

fn device_cmyk_components(color: &crate::content::state::Color) -> Option<[f32; 4]> {
    if !matches!(color.space, ColorSpace::DeviceCMYK) {
        return None;
    }
    Some([
        color
            .components
            .first()
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0) as f32,
        color
            .components
            .get(1)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0) as f32,
        color
            .components
            .get(2)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0) as f32,
        color
            .components
            .get(3)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0) as f32,
    ])
}

fn clamp_unit(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn pdf_num(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    let mut s = format!("{value:.3}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        "0".to_string()
    } else {
        s
    }
}

fn pdf_literal_bytes(bytes: &[u8]) -> String {
    let mut out = String::from("(");
    for &byte in bytes {
        match byte {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(byte as char);
            }
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(byte as char),
            _ => {
                let _ = write!(out, "\\{byte:03o}");
            }
        }
    }
    out.push(')');
    out
}

fn resolve_appearance_stream(
    value: &PdfObject,
    reader: &crate::reader::PdfReader,
) -> Option<(PdfDictionary, Vec<u8>)> {
    match reader.resolve(value.clone()).ok()? {
        PdfObject::Stream { dict, raw } => Some((dict, raw)),
        _ => None,
    }
}

fn extract_rect(dict: &PdfDictionary) -> Option<[f64; 4]> {
    let arr = dict.get("Rect")?.as_array()?;
    if arr.len() < 4 {
        return None;
    }
    let vals: Vec<f64> = arr
        .iter()
        .take(4)
        .filter_map(PdfObject::as_number)
        .collect();
    if vals.len() < 4 {
        return None;
    }
    Some([vals[0], vals[1], vals[2], vals[3]])
}

fn annotation_appearance_ctm(rect: [f64; 4], bbox: [f64; 4]) -> Option<Transform2D> {
    let rect_x0 = rect[0].min(rect[2]);
    let rect_y0 = rect[1].min(rect[3]);
    let rect_w = (rect[2] - rect[0]).abs();
    let rect_h = (rect[3] - rect[1]).abs();
    let bbox_x0 = bbox[0].min(bbox[2]);
    let bbox_y0 = bbox[1].min(bbox[3]);
    let bbox_w = (bbox[2] - bbox[0]).abs();
    let bbox_h = (bbox[3] - bbox[1]).abs();
    if rect_w <= 0.0 || rect_h <= 0.0 || bbox_w <= 0.0 || bbox_h <= 0.0 {
        return None;
    }
    let to_origin = Transform2D::translation(-bbox_x0, -bbox_y0);
    let scale = Transform2D::scale(rect_w / bbox_w, rect_h / bbox_h);
    let to_rect = Transform2D::translation(rect_x0, rect_y0);
    Some(to_origin.concat(&scale).concat(&to_rect))
}

/// Extract a Form XObject's `/BBox` as `[x_min, y_min, x_max, y_max]`.
/// Returns `None` when absent or not a 4-number array.
fn extract_bbox(dict: &PdfDictionary) -> Option<[f64; 4]> {
    let arr = dict.get("BBox")?.as_array()?;
    if arr.len() < 4 {
        return None;
    }
    let vals: Vec<f64> = arr
        .iter()
        .take(4)
        .filter_map(PdfObject::as_number)
        .collect();
    if vals.len() < 4 {
        return None;
    }
    Some([vals[0], vals[1], vals[2], vals[3]])
}

fn form_bbox_intersects_viewport(bbox: [f64; 4], ctm: &Transform2D, viewport: &Viewport) -> bool {
    let x_min = bbox[0].min(bbox[2]);
    let y_min = bbox[1].min(bbox[3]);
    let width = (bbox[2] - bbox[0]).abs();
    let height = (bbox[3] - bbox[1]).abs();
    if width <= 0.0 || height <= 0.0 {
        return true;
    }
    let mut path = Path::new();
    path.rect(x_min, y_min, width, height);
    RenderBounds::from_path(&path, ctm, viewport, 1.0)
        .is_none_or(|bounds| bounds.intersects_viewport(viewport))
}

fn axis_aligned_bbox_clip_rect(
    bbox: [f64; 4],
    ctm: &Transform2D,
    viewport: &Viewport,
    width: u32,
    height: u32,
) -> Option<(i32, i32, i32, i32)> {
    if width == 0 || height == 0 {
        return None;
    }
    let x0 = bbox[0].min(bbox[2]);
    let x1 = bbox[0].max(bbox[2]);
    let y0 = bbox[1].min(bbox[3]);
    let y1 = bbox[1].max(bbox[3]);
    if [x0, x1, y0, y1].iter().any(|value| !value.is_finite())
        || (x1 - x0).abs() <= f64::EPSILON
        || (y1 - y0).abs() <= f64::EPSILON
    {
        return None;
    }
    let device_t = ctm.concat(&viewport.to_transform());
    let corners = [
        device_t.transform_point(x0, y0),
        device_t.transform_point(x1, y0),
        device_t.transform_point(x1, y1),
        device_t.transform_point(x0, y1),
    ];
    if corners
        .iter()
        .any(|(x, y)| !x.is_finite() || !y.is_finite())
    {
        return None;
    }
    let horizontal = |a: (f64, f64), b: (f64, f64)| (a.1 - b.1).abs() <= 1e-6;
    let vertical = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() <= 1e-6;
    let axis_aligned_edges = corners
        .iter()
        .copied()
        .zip(corners.iter().copied().cycle().skip(1))
        .take(4)
        .all(|(a, b)| horizontal(a, b) || vertical(a, b));
    if !axis_aligned_edges {
        return None;
    }
    let min_x = corners
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::INFINITY, f64::min);
    let max_x = corners
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = corners
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::INFINITY, f64::min);
    let max_y = corners
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);
    let left = f64_floor_to_i32_saturating(min_x).clamp(0, u32_to_i32_saturating(width));
    let top = f64_floor_to_i32_saturating(min_y).clamp(0, u32_to_i32_saturating(height));
    let right = f64_ceil_to_i32_saturating(max_x).clamp(0, u32_to_i32_saturating(width));
    let bottom = f64_ceil_to_i32_saturating(max_y).clamp(0, u32_to_i32_saturating(height));
    if right <= left || bottom <= top {
        None
    } else {
        Some((left, top, right - left, bottom - top))
    }
}

fn transparency_group_pixel_window(
    bbox: Option<[f64; 4]>,
    ctm: &Transform2D,
    viewport: &Viewport,
    clip: Option<&ClipMask>,
) -> Option<RenderTile> {
    let vx0 = u32_to_i32_saturating(viewport.origin_x_px);
    let vy0 = u32_to_i32_saturating(viewport.origin_y_px);
    let vx1 = u32_to_i32_saturating(viewport.origin_x_px.saturating_add(viewport.width_px));
    let vy1 = u32_to_i32_saturating(viewport.origin_y_px.saturating_add(viewport.height_px));
    let mut x0 = vx0;
    let mut y0 = vy0;
    let mut x1 = vx1;
    let mut y1 = vy1;

    if let Some(bounds) = bbox.and_then(|bbox| RenderBounds::from_bbox(bbox, ctm, viewport, 2.0)) {
        x0 = x0.max(bounds.x0);
        y0 = y0.max(bounds.y0);
        x1 = x1.min(bounds.x1);
        y1 = y1.min(bounds.y1);
    }

    if let Some(clip) = clip {
        let (cx0, cy0, cx1, cy1) = clip.visible_bounds()?;
        x0 = x0.max(vx0.saturating_add(cx0));
        y0 = y0.max(vy0.saturating_add(cy0));
        x1 = x1.min(vx0.saturating_add(cx1));
        y1 = y1.min(vy0.saturating_add(cy1));
    }

    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let local_x = x0.saturating_sub(vx0).max(0) as u32;
    let local_y = y0.saturating_sub(vy0).max(0) as u32;
    let width = (x1 - x0).max(0) as u32;
    let height = (y1 - y0).max(0) as u32;
    if width == 0 || height == 0 {
        None
    } else {
        Some(RenderTile {
            x: local_x,
            y: local_y,
            width: width.min(viewport.width_px.saturating_sub(local_x)),
            height: height.min(viewport.height_px.saturating_sub(local_y)),
        })
    }
}

fn u32_to_i32_saturating(value: u32) -> i32 {
    if value > i32::MAX as u32 {
        i32::MAX
    } else {
        value as i32
    }
}

/// Extract a Form XObject's `/Matrix`, defaulting to the identity matrix when
/// absent or malformed.
fn extract_form_matrix(dict: &PdfDictionary) -> crate::content::Matrix {
    const IDENTITY: crate::content::Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let Some(arr) = dict.get("Matrix").and_then(PdfObject::as_array) else {
        return IDENTITY;
    };
    let v: Vec<f64> = arr.iter().filter_map(PdfObject::as_number).collect();
    if v.len() < 6 {
        return IDENTITY;
    }
    [v[0], v[1], v[2], v[3], v[4], v[5]]
}

#[derive(Default)]
struct ResourceOverlayRestore {
    fonts: Vec<(String, Option<PdfDictionary>)>,
    xobjects: Vec<(String, Option<(u32, u16)>)>,
    xobject_subtypes: Vec<(String, Option<String>)>,
    xobject_bboxes: Vec<(String, Option<[f64; 4]>)>,
    xobject_matrices: Vec<(String, Option<[f64; 6]>)>,
    color_spaces: Vec<(String, Option<PdfObject>)>,
    ext_g_states: Vec<(String, Option<PdfDictionary>)>,
    patterns: Vec<(String, Option<PdfObject>)>,
    shadings: Vec<(String, Option<PdfObject>)>,
    properties: Vec<(String, Option<PdfObject>)>,
}

impl ResourceOverlayRestore {
    fn restore(self, resources: &mut PageResources) {
        restore_overlay_map(&mut resources.fonts, self.fonts);
        restore_overlay_map(&mut resources.xobjects, self.xobjects);
        restore_overlay_map(&mut resources.xobject_subtypes, self.xobject_subtypes);
        restore_overlay_map(&mut resources.xobject_bboxes, self.xobject_bboxes);
        restore_overlay_map(&mut resources.xobject_matrices, self.xobject_matrices);
        restore_overlay_map(&mut resources.color_spaces, self.color_spaces);
        restore_overlay_map(&mut resources.ext_g_states, self.ext_g_states);
        restore_overlay_map(&mut resources.patterns, self.patterns);
        restore_overlay_map(&mut resources.shadings, self.shadings);
        restore_overlay_map(&mut resources.properties, self.properties);
    }
}

fn overlay_page_resources(
    resources: &mut PageResources,
    form_res: &PageResources,
) -> ResourceOverlayRestore {
    let mut restore = ResourceOverlayRestore::default();
    overlay_resource_map(&mut resources.fonts, &form_res.fonts, &mut restore.fonts);
    overlay_resource_map(
        &mut resources.xobjects,
        &form_res.xobjects,
        &mut restore.xobjects,
    );
    overlay_resource_map(
        &mut resources.xobject_subtypes,
        &form_res.xobject_subtypes,
        &mut restore.xobject_subtypes,
    );
    overlay_resource_map(
        &mut resources.xobject_bboxes,
        &form_res.xobject_bboxes,
        &mut restore.xobject_bboxes,
    );
    overlay_resource_map(
        &mut resources.xobject_matrices,
        &form_res.xobject_matrices,
        &mut restore.xobject_matrices,
    );
    overlay_resource_map(
        &mut resources.color_spaces,
        &form_res.color_spaces,
        &mut restore.color_spaces,
    );
    overlay_resource_map(
        &mut resources.ext_g_states,
        &form_res.ext_g_states,
        &mut restore.ext_g_states,
    );
    overlay_resource_map(
        &mut resources.patterns,
        &form_res.patterns,
        &mut restore.patterns,
    );
    overlay_resource_map(
        &mut resources.shadings,
        &form_res.shadings,
        &mut restore.shadings,
    );
    overlay_resource_map(
        &mut resources.properties,
        &form_res.properties,
        &mut restore.properties,
    );
    restore
}

fn overlay_resource_map<T: Clone>(
    target: &mut HashMap<String, T>,
    source: &HashMap<String, T>,
    restore: &mut Vec<(String, Option<T>)>,
) {
    for (key, value) in source {
        restore.push((key.clone(), target.insert(key.clone(), value.clone())));
    }
}

fn restore_overlay_map<T>(target: &mut HashMap<String, T>, restore: Vec<(String, Option<T>)>) {
    for (key, previous) in restore.into_iter().rev() {
        match previous {
            Some(value) => {
                target.insert(key, value);
            }
            None => {
                target.remove(&key);
            }
        }
    }
}

fn merge_resources_ref(form_res: &PageResources, page_res: &PageResources) -> PageResources {
    let mut merged = page_res.clone();
    for (k, v) in &form_res.fonts {
        merged.fonts.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.xobjects {
        merged.xobjects.insert(k.clone(), *v);
    }
    for (k, v) in &form_res.xobject_subtypes {
        merged.xobject_subtypes.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.xobject_bboxes {
        merged.xobject_bboxes.insert(k.clone(), *v);
    }
    for (k, v) in &form_res.xobject_matrices {
        merged.xobject_matrices.insert(k.clone(), *v);
    }
    for (k, v) in &form_res.color_spaces {
        merged.color_spaces.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.ext_g_states {
        merged.ext_g_states.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.patterns {
        merged.patterns.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.shadings {
        merged.shadings.insert(k.clone(), v.clone());
    }
    for (k, v) in &form_res.properties {
        merged.properties.insert(k.clone(), v.clone());
    }
    merged
}

/// Merge a Form XObject's resources over the parent page's resources. The
/// Form's entries take priority on a name collision; names absent from the Form
/// fall through to the page's resources.
fn merge_resources(form_res: PageResources, page_res: &PageResources) -> PageResources {
    merge_resources_ref(&form_res, page_res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::images::encoder::ImageEncoder;
    use crate::render::{flatten_path, Path, PathPainter, RenderColor, BLACK, BLUE, RED, WHITE};

    fn fixture(path: &str) -> String {
        format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), path)
    }

    fn decoded_glyph(unicode: char, is_gid: bool, glyph_name: Option<&str>) -> DecodedGlyph {
        DecodedGlyph {
            code: unicode as u16,
            unicode,
            glyph_name: glyph_name.map(str::to_string),
            is_space: unicode == ' ',
            width: None,
            is_gid,
            is_vertical: false,
            vertical_advance: None,
            vertical_origin: None,
        }
    }

    fn blank_render_state(engine: &ContentEngine) -> RenderState<'_> {
        let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        let buf = PixelBuffer::new_filled_with_mode(
            viewport.width_px,
            viewport.height_px,
            WHITE,
            RenderMode::Compat,
        );
        RenderState::new(buf, viewport, PageResources::default(), engine, 1)
    }

    #[test]
    fn renderer_inline_decode_acquires_scheduler_token() {
        let engine = ContentEngine::open_path(fixture("image_only.pdf")).expect("open fixture");
        let state = blank_render_state(&engine);
        let raw = state
            .scheduled_decode_inline_image(&[128], 1, 1, 8, "DeviceGray", &[], &[])
            .expect("inline image decode");
        assert_eq!(raw.width, 1);
        let metrics = state.decode_scheduler.metrics();
        assert_eq!(metrics.jobs, 1);
        assert!(metrics.peak_reserved_bytes >= 1);
        assert_eq!(metrics.rejected_jobs, 0);
    }

    #[test]
    fn renderer_decode_scheduler_fails_closed_over_budget() {
        let engine = ContentEngine::open_path(fixture("image_only.pdf")).expect("open fixture");
        let mut state = blank_render_state(&engine);
        state.decode_scheduler = RenderDecodeScheduler::new(&DecodeLimits {
            scheduler_memory_budget_bytes: 1,
            ..DecodeLimits::default()
        });
        let err = state
            .scheduled_decode_inline_image(&[0; 16], 4, 4, 8, "DeviceGray", &[], &[])
            .expect_err("decode estimate should exceed scheduler budget");
        assert!(err.to_string().contains("exceeding scheduler budget"));
        let metrics = state.decode_scheduler.metrics();
        assert_eq!(metrics.jobs, 1);
        assert_eq!(metrics.rejected_jobs, 1);
        assert_eq!(metrics.failed_jobs, 1);
    }

    #[test]
    fn renderer_offscreen_surface_acquires_scheduler_token() {
        let engine = ContentEngine::open_path(fixture("image_only.pdf")).expect("open fixture");
        let state = blank_render_state(&engine);
        {
            let _token = state
                .reserve_offscreen_surface(10, 10, "test offscreen surface")
                .expect("10x10 RGBA surface should fit default budget");
            let metrics = state.decode_scheduler.metrics();
            assert_eq!(metrics.jobs, 1);
            assert!(metrics.peak_reserved_bytes >= 400);
            assert_eq!(metrics.rejected_jobs, 0);
        }
    }

    #[test]
    fn renderer_offscreen_surface_fails_closed_over_budget() {
        let engine = ContentEngine::open_path(fixture("image_only.pdf")).expect("open fixture");
        let mut state = blank_render_state(&engine);
        state.decode_scheduler = RenderDecodeScheduler::new(&DecodeLimits {
            scheduler_memory_budget_bytes: 399,
            ..DecodeLimits::default()
        });
        let err = match state.reserve_offscreen_surface(10, 10, "test offscreen surface") {
            Ok(_) => panic!("10x10 RGBA surface should exceed the 399-byte budget"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("exceeding scheduler budget"));
        let metrics = state.decode_scheduler.metrics();
        assert_eq!(metrics.jobs, 1);
        assert_eq!(metrics.rejected_jobs, 1);
        assert_eq!(metrics.failed_jobs, 1);
    }

    #[test]
    fn renderer_decode_scheduler_observes_cancel_before_decode() {
        let engine = ContentEngine::open_path(fixture("image_only.pdf")).expect("open fixture");
        let mut state = blank_render_state(&engine);
        state.cancel = CancelToken::new();
        state.cancel.cancel();
        let err = state
            .scheduled_decode_inline_image(&[0], 1, 1, 8, "DeviceGray", &[], &[])
            .expect_err("pre-cancelled decode should fail");
        assert!(matches!(err, WellfriendError::Cancelled(_)));
        let metrics = state.decode_scheduler.metrics();
        assert_eq!(metrics.jobs, 1);
        assert_eq!(metrics.cancelled_before_decode, 1);
    }

    #[test]
    fn nonembedded_font_resource_alias_uses_basefont_for_fallback() {
        let engine = ContentEngine::open_path(fixture("image_only.pdf")).expect("open fixture");
        let reader = engine.document().reader();
        let mut font = PdfDictionary::new(std::collections::BTreeMap::new());
        font.insert("Type", PdfObject::Name("Font".to_string()));
        font.insert("Subtype", PdfObject::Name("Type1".to_string()));
        font.insert("BaseFont", PdfObject::Name("Times-Roman".to_string()));

        let lookup_name = fallback_font_lookup_name("F1", &font, reader);
        assert_eq!(lookup_name, "Times-Roman");
        assert!(std::ptr::eq(
            get_fallback_font(&lookup_name).expect("base font fallback"),
            get_fallback_font("Times-Roman").expect("Times fallback")
        ));
        assert!(!std::ptr::eq(
            get_fallback_font(&lookup_name).expect("base font fallback"),
            get_fallback_font("F1").expect("resource alias fallback")
        ));
    }

    #[test]
    fn type0_font_resource_alias_uses_descendant_basefont_for_fallback() {
        let engine = ContentEngine::open_path(fixture("image_only.pdf")).expect("open fixture");
        let reader = engine.document().reader();
        let mut descendant = PdfDictionary::new(std::collections::BTreeMap::new());
        descendant.insert("Type", PdfObject::Name("Font".to_string()));
        descendant.insert("Subtype", PdfObject::Name("CIDFontType2".to_string()));
        descendant.insert("BaseFont", PdfObject::Name("Courier-Bold".to_string()));
        let mut font = PdfDictionary::new(std::collections::BTreeMap::new());
        font.insert("Type", PdfObject::Name("Font".to_string()));
        font.insert("Subtype", PdfObject::Name("Type0".to_string()));
        font.insert(
            "DescendantFonts",
            PdfObject::Array(vec![PdfObject::Dictionary(descendant)]),
        );

        let lookup_name = fallback_font_lookup_name("F2", &font, reader);
        assert_eq!(lookup_name, "Courier-Bold");
        assert!(std::ptr::eq(
            get_fallback_font(&lookup_name).expect("descendant fallback"),
            get_fallback_font("Courier-Bold").expect("Courier fallback")
        ));
    }

    #[test]
    fn skips_non_cid_replacement_and_control_glyph_painting() {
        assert!(!should_paint_decoded_glyph(&decoded_glyph(
            '\u{FFFD}', false, None
        )));
        assert!(!should_paint_decoded_glyph(&decoded_glyph(
            '\u{FFFD}',
            false,
            Some(".notdef")
        )));
        assert!(!should_paint_decoded_glyph(&decoded_glyph(
            '\0', false, None
        )));
        assert!(should_paint_decoded_glyph(&decoded_glyph(
            '\u{FFFD}',
            false,
            Some("G1")
        )));
        assert!(should_paint_decoded_glyph(&decoded_glyph('A', false, None)));
        assert!(should_paint_decoded_glyph(&decoded_glyph(
            '\u{FFFD}', true, None
        )));
    }

    fn simple_pdf_with_extgstate(
        content: &str,
        extgstates: &[&str],
        extgstate_resources: &str,
    ) -> Vec<u8> {
        fn add_obj(objects: &mut Vec<Vec<u8>>, body: impl AsRef<[u8]>) -> usize {
            objects.push(body.as_ref().to_vec());
            objects.len()
        }

        let mut objects = Vec::new();
        let font = add_obj(
            &mut objects,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        );
        let ext_ids: Vec<usize> = extgstates
            .iter()
            .map(|body| add_obj(&mut objects, body.as_bytes()))
            .collect();
        let stream = format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        );
        let contents = add_obj(&mut objects, stream.as_bytes());
        let page = objects.len() + 1;
        let pages = objects.len() + 2;
        let root = objects.len() + 3;
        let resources = extgstate_resources
            .replace("{font}", &font.to_string())
            .replace("{gs1}", &ext_ids[0].to_string())
            .replace("{gs2}", &ext_ids[1].to_string());
        add_obj(
            &mut objects,
            format!(
                "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 100 100] \
                 /Resources {} /Contents {} 0 R >>",
                pages, resources, contents
            )
            .as_bytes(),
        );
        add_obj(
            &mut objects,
            format!("<< /Type /Pages /Kids [{} 0 R] /Count 1 >>", page).as_bytes(),
        );
        add_obj(
            &mut objects,
            format!("<< /Type /Catalog /Pages {} 0 R >>", pages).as_bytes(),
        );

        let mut out = bytearray_pdf_header();
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
                "trailer\n<< /Size {} /Root {} 0 R >>\nstartxref\n{}\n%%EOF\n",
                objects.len() + 1,
                root,
                startxref
            )
            .as_bytes(),
        );
        out
    }

    fn bytearray_pdf_header() -> Vec<u8> {
        b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec()
    }

    fn simple_vector_pdf(content: &str) -> Vec<u8> {
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 4 0 R >>".to_vec(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content)
                .into_bytes(),
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn simple_prepress_plate_pdf() -> Vec<u8> {
        let content = "/CS1 cs 0.25 scn 10 10 20 20 re f\n/CS1 CS 0.75 SCN 40 10 m 80 10 l S\n/CS2 cs 0.20 0.80 scn 10 40 20 20 re f\n";
        let type4 = "{ 0 }";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /ColorSpace << /CS1 [/Separation /SpotOrange /DeviceRGB 5 0 R] /CS2 [/DeviceN [/Cyan /SpotGreen] /DeviceRGB 6 0 R] >> >> /Contents 4 0 R >>".to_vec(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content)
                .into_bytes(),
            b"<< /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0.5 0] /N 1 >>".to_vec(),
            format!(
                "<< /FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Length {} >>\nstream\n{}\nendstream",
                type4.len(),
                type4
            )
            .into_bytes(),
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn simple_ocg_pdf() -> Vec<u8> {
        let content = "/OC /L1 BDC 1 0 0 rg 10 10 80 80 re f EMC\n0 0 1 rg 0 0 10 10 re f\n";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R] /D << /Name (Default) /BaseState /ON /OFF [5 0 R] /Order [5 0 R] >> >> >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Properties << /L1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content)
                .into_bytes(),
            b"<< /Type /OCG /Name (Hidden Layer) /Intent /View >>".to_vec(),
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn assert_same_pixels(expected: &PixelBuffer, actual: &PixelBuffer) {
        assert_eq!(expected.width, actual.width);
        assert_eq!(expected.height, actual.height);
        for y in 0..expected.height as i32 {
            for x in 0..expected.width as i32 {
                assert_eq!(
                    expected.get_pixel(x, y),
                    actual.get_pixel(x, y),
                    "rendered pixels diverged at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn display_list_page_render_replays_vector_content() {
        let pdf = simple_vector_pdf("1 0 0 rg 10 10 40 40 re f\n0 0 0 RG 4 w 10 10 m 90 90 l S\n");
        let engine = ContentEngine::open_bytes(pdf).expect("open vector-only PDF");
        let list = engine
            .build_page_display_list(1, 72)
            .expect("build display list");

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.fills, 1);
        assert_eq!(list.stats.strokes, 1);
        assert!(list.approximate_memory_bytes() > 0);

        let via_list = engine
            .render_page_display_list_with_mode(1, 72, RenderMode::Compat)
            .expect("display-list render")
            .expect("vector page should be display-list compatible");
        assert_eq!(via_list.get_pixel(20, 70), RED);
        assert_ne!(via_list.get_pixel(50, 50), WHITE);
    }

    #[test]
    fn prepress_plate_report_records_separation_and_devicen_fill_stroke_tints() {
        let engine = ContentEngine::open_bytes(simple_prepress_plate_pdf())
            .expect("open prepress plate PDF");
        let report = engine
            .prepress_plate_report(1, 72)
            .expect("prepress plate report");
        assert_eq!(
            report.deterministic_plane_order,
            vec![
                "Cyan".to_string(),
                "SpotGreen".to_string(),
                "SpotOrange".to_string()
            ]
        );
        assert_eq!(report.plate_count, 3);
        assert_eq!(report.contribution_count, 4);
        assert!(report.scheduler_accounted);
        assert!(!report.cache_fingerprint.is_empty());
        assert!(report
            .plate_previews
            .iter()
            .any(|preview| preview.plane_name == "SpotOrange"));
    }

    #[test]
    fn optional_content_marked_content_hides_off_layer() {
        let engine = ContentEngine::open_bytes(simple_ocg_pdf()).expect("open OCG PDF");
        let list = engine
            .build_page_display_list(1, 72)
            .expect("build display list");
        assert!(!list.has_compatibility_runs());
        assert!(list.stats.optional_content_ops > 0);

        let report = OptionalContentContext::from_document(engine.document());
        assert_eq!(report.report().layers.len(), 1);
        assert!(!report.report().layers[0].default_state);

        let buf = engine.render_page(1, 72).expect("render OCG PDF");
        assert_eq!(buf.get_pixel(50, 50), WHITE);
        assert_eq!(buf.get_pixel(5, 95), BLUE);
    }

    #[test]
    fn progressive_render_resume_matches_full_page() {
        let pdf = simple_vector_pdf(
            "q 1 0 0 rg 10 10 40 40 re f Q\n\
             q 0 0 1 rg 50 50 40 40 re f Q\n\
             0 0 0 RG 3 w 0 0 m 100 100 l S\n",
        );
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let full = engine.render_page(1, 72).expect("render full page");
        let mut job = engine
            .progressive_render_job_with_mode(1, 72, 25, 25, RenderMode::Compat)
            .expect("create progressive job");

        let first = job
            .render_next(3, &CancelToken::none())
            .expect("first progressive step");
        assert_eq!(first.rendered_this_step, 3);
        assert!(first.resume_possible);
        assert!(!job.token().complete);

        while !job.is_complete() {
            job.render_next(2, &CancelToken::none())
                .expect("progressive resume step");
        }
        let progressive = job.finish().expect("completed progressive surface");
        assert_same_pixels(&full, &progressive);
    }

    #[test]
    fn progressive_resume_token_rejects_mismatched_state() {
        let pdf = simple_vector_pdf(
            "q 1 0 0 rg 10 10 40 40 re f Q\n\
             q 0 0 1 rg 50 50 40 40 re f Q\n",
        );
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let mut job = engine
            .progressive_render_job_with_mode(1, 72, 25, 25, RenderMode::Compat)
            .expect("create progressive job");

        job.render_next(2, &CancelToken::none())
            .expect("first progressive step");
        let token = job.token();
        job.validate_resume_token(&token)
            .expect("current token should validate");

        let mut bad_dpi = token.clone();
        bad_dpi.dpi = 144;
        let err = job
            .validate_resume_token(&bad_dpi)
            .expect_err("changed DPI must reject a resume token");
        assert!(format!("{err}").contains("dpi"));

        let mut bad_ocg = token.clone();
        bad_ocg.visibility_fingerprint = "ocg:view:changed".to_string();
        let err = job
            .validate_resume_token(&bad_ocg)
            .expect_err("changed OCG fingerprint must reject a resume token");
        assert!(format!("{err}").contains("visibility_fingerprint"));
    }

    #[test]
    fn progressive_cancel_report_retains_only_completed_tile_memory() {
        let pdf = simple_vector_pdf("1 0 0 rg 0 0 100 100 re f\n");
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let mut job = engine
            .progressive_render_job_with_mode(1, 72, 25, 25, RenderMode::Compat)
            .expect("create progressive job");
        let cancel = CancelToken::new();

        job.render_next(1, &CancelToken::none())
            .expect("first progressive tile");
        cancel.cancel();
        let report = job
            .render_next(4, &cancel)
            .expect("cancelled progressive step");

        assert!(report.cancelled);
        assert!(report.resume_possible);
        assert_eq!(report.completed_units, 1);
        assert_eq!(report.memory_bytes_retained, 25 * 25 * 4);
        job.validate_resume_token(&job.token())
            .expect("cancelled job should remain resumable with current token");
    }

    #[test]
    fn display_list_replay_matches_immediate_vector_render() {
        let pdf = simple_vector_pdf(
            "q 0 0 1 rg 10 10 60 60 re f Q\n\
             q 10 10 60 60 re W n 1 0 0 rg 0 0 100 100 re f Q\n\
             0 0 0 RG 6 w [8 4] 0 d 5 95 m 95 5 l S\n",
        );
        let engine = ContentEngine::open_bytes(pdf).expect("open vector-only PDF");

        let immediate = engine.render_page(1, 72).expect("immediate render");
        let replay = engine
            .render_page_display_list_with_mode(1, 72, RenderMode::Compat)
            .expect("display-list query")
            .expect("vector page should replay");

        assert_eq!(immediate.width, replay.width);
        assert_eq!(immediate.height, replay.height);
        for y in 0..immediate.height as i32 {
            for x in 0..immediate.width as i32 {
                assert_eq!(
                    immediate.get_pixel(x, y),
                    replay.get_pixel(x, y),
                    "display-list replay diverged at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn display_list_replays_text_page_through_native_ops() {
        let engine = ContentEngine::open_path(fixture("flate.pdf")).expect("open text fixture");
        let list = engine
            .build_page_display_list(1, 72)
            .expect("build display list");

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert!(list.stats.text_ops > 0);
        assert!(list.stats.native_text_ops > 0);

        let immediate = engine.render_page(1, 72).expect("immediate render");
        let replay = engine
            .render_page_display_list_with_mode(1, 72, RenderMode::Compat)
            .expect("display-list replay query")
            .expect("text page should replay through native operations");
        assert_same_pixels(&immediate, &replay);
    }

    #[test]
    fn display_list_replays_image_page_through_native_ops() {
        let engine =
            ContentEngine::open_path(fixture("image_only.pdf")).expect("open image fixture");
        let list = engine
            .build_page_display_list(1, 72)
            .expect("build display list");

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert!(list.stats.image_xobjects > 0 || list.stats.inline_images > 0);
        assert!(list.stats.native_image_xobjects > 0 || list.stats.native_inline_images > 0);

        let immediate = engine.render_page(1, 72).expect("immediate render");
        let replay = engine
            .render_page_display_list_with_mode(1, 72, RenderMode::Compat)
            .expect("display-list replay query")
            .expect("image page should replay through native operations");
        assert_same_pixels(&immediate, &replay);
    }

    #[test]
    fn display_list_tile_stitch_matches_full_page() {
        let pdf = simple_vector_pdf(
            "1 0 0 rg 0 0 50 100 re f\n\
             0 0 1 rg 50 0 50 100 re f\n\
             0 0 0 RG 3 w 5 5 m 95 95 l S\n",
        );
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let full = engine
            .render_page_display_list_with_mode(1, 72, RenderMode::Compat)
            .expect("display-list render")
            .expect("vector page should replay");
        let mut stitched = PixelBuffer::new_transparent_with_mode(100, 100, RenderMode::Compat);
        let mut document_cache = RenderDocumentCache::new();
        for tile in [
            RenderTile {
                x: 0,
                y: 0,
                width: 50,
                height: 50,
            },
            RenderTile {
                x: 50,
                y: 0,
                width: 50,
                height: 50,
            },
            RenderTile {
                x: 0,
                y: 50,
                width: 50,
                height: 50,
            },
            RenderTile {
                x: 50,
                y: 50,
                width: 50,
                height: 50,
            },
        ] {
            let piece = engine
                .render_page_display_list_tile_cancellable_with_mode_and_cache(
                    1,
                    72,
                    tile,
                    &CancelToken::none(),
                    RenderMode::Compat,
                    &mut document_cache,
                )
                .expect("render display-list tile")
                .expect("vector page should replay as a display-list tile");
            for y in 0..piece.height as i32 {
                for x in 0..piece.width as i32 {
                    stitched.set_pixel(tile.x as i32 + x, tile.y as i32 + y, piece.get_pixel(x, y));
                }
            }
        }
        assert_eq!(document_cache.display_list_entries(), 1);
        assert_eq!(document_cache.transparent_page_group_entries(), 1);
        assert_same_pixels(&full, &stitched);
    }

    #[test]
    fn display_list_tile_replay_uses_raster_cache_on_repeat_viewport() {
        let pdf = simple_vector_pdf("1 0 0 rg 0 0 100 100 re f\n0 0 1 rg 25 25 50 50 re f\n");
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let tile = RenderTile {
            x: 20,
            y: 20,
            width: 40,
            height: 40,
        };
        let mut document_cache = RenderDocumentCache::new();
        let first = engine
            .render_page_display_list_tile_cancellable_with_mode_and_cache(
                1,
                72,
                tile,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut document_cache,
            )
            .expect("first display-list tile render")
            .expect("vector page should replay");
        let after_first = document_cache.display_list_raster_cache_metrics();
        assert_eq!(after_first.inserts, 1);
        assert_eq!(after_first.hits, 0);

        let second = engine
            .render_page_display_list_tile_cancellable_with_mode_and_cache(
                1,
                72,
                tile,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut document_cache,
            )
            .expect("second display-list tile render")
            .expect("vector page should replay from tile cache");
        let after_second = document_cache.display_list_raster_cache_metrics();
        assert_eq!(after_second.inserts, 1);
        assert_eq!(after_second.hits, 1);
        assert_same_pixels(&first, &second);
    }

    #[test]
    fn display_list_tile_replay_crops_warm_full_page_raster() {
        let pdf = simple_vector_pdf("1 0 0 rg 0 0 100 100 re f\n0 0 1 rg 25 25 50 50 re f\n");
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let mut document_cache = RenderDocumentCache::new();
        let full = engine
            .render_page_display_list_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut document_cache,
            )
            .expect("full display-list render")
            .expect("vector page should replay");
        let metrics_after_full = document_cache.display_list_raster_cache_metrics();
        assert_eq!(metrics_after_full.inserts, 1);
        assert_eq!(metrics_after_full.hits, 0);

        let tile = RenderTile {
            x: 20,
            y: 20,
            width: 40,
            height: 40,
        };
        let from_warm_full = engine
            .render_page_display_list_tile_cancellable_with_mode_and_cache(
                1,
                72,
                tile,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut document_cache,
            )
            .expect("tile from warm full-page raster")
            .expect("vector page should replay from full-page cache");
        let expected = crop_buffer(&full, tile).expect("expected crop");
        let metrics_after_tile = document_cache.display_list_raster_cache_metrics();
        assert_eq!(metrics_after_tile.inserts, 2);
        assert_eq!(metrics_after_tile.hits, 1);
        assert_same_pixels(&expected, &from_warm_full);
    }

    #[test]
    fn render_document_cache_retains_display_lists_by_page_and_dpi() {
        let mut cache = RenderDocumentCache::new();
        let key = RenderDocumentCache::display_list_key_with_revision(2, 144, "revision:test");
        let list = DisplayList {
            viewport: Viewport::new([0.0, 0.0, 10.0, 10.0], 144),
            ops: Vec::new(),
            stats: Default::default(),
            supported: true,
            unsupported: Vec::new(),
        };

        let inserted = cache.insert_display_list(key.clone(), list);
        let cached = cache
            .cached_display_list(&key)
            .expect("display list should be retained");
        assert!(std::sync::Arc::ptr_eq(&inserted, &cached));
        assert_eq!(cache.display_list_entries(), 1);

        cache.clear();
        assert_eq!(cache.display_list_entries(), 0);
    }

    #[test]
    fn display_list_native_vector_fast_path_retains_display_list_cache() {
        let pdf = simple_vector_pdf("1 0 0 rg 0 0 100 100 re f\n");
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let mut cache = RenderDocumentCache::new();

        let first = engine
            .render_page_display_list_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("display-list render")
            .expect("vector page should replay");
        assert_eq!(cache.display_list_entries(), 1);
        assert_eq!(cache.transparent_page_group_entries(), 0);
        let metrics_after_first = cache.display_list_raster_cache_metrics();
        assert_eq!(metrics_after_first.inserts, 1);
        assert_eq!(metrics_after_first.hits, 0);

        let second = engine
            .render_page_display_list_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("display-list render from cache")
            .expect("vector page should replay from cache");
        assert_eq!(cache.display_list_entries(), 1);
        assert_eq!(cache.transparent_page_group_entries(), 0);
        let metrics_after_second = cache.display_list_raster_cache_metrics();
        assert_eq!(metrics_after_second.inserts, 1);
        assert_eq!(metrics_after_second.hits, 1);
        assert_same_pixels(&first, &second);
    }

    #[test]
    fn display_list_high_level_replay_caches_transparency_decision() {
        let engine = ContentEngine::open_path(fixture("flate.pdf")).expect("open text fixture");
        let mut cache = RenderDocumentCache::new();

        let first = engine
            .render_page_display_list_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("display-list render")
            .expect("text page should replay");
        assert_eq!(cache.display_list_entries(), 1);
        assert_eq!(cache.transparent_page_group_entries(), 1);
        let metrics_after_first = cache.display_list_raster_cache_metrics();
        assert_eq!(metrics_after_first.inserts, 1);
        assert_eq!(metrics_after_first.hits, 0);

        let second = engine
            .render_page_display_list_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("display-list render from cache")
            .expect("text page should replay from cache");
        assert_eq!(cache.display_list_entries(), 1);
        assert_eq!(cache.transparent_page_group_entries(), 1);
        let metrics_after_second = cache.display_list_raster_cache_metrics();
        assert_eq!(metrics_after_second.inserts, 1);
        assert_eq!(metrics_after_second.hits, 1);
        assert_same_pixels(&first, &second);
    }

    #[test]
    fn render_document_cache_retains_decoded_image_xobjects() {
        let engine =
            ContentEngine::open_path(fixture("image_only.pdf")).expect("open image fixture");
        let mut cache = RenderDocumentCache::new();

        let first = engine
            .render_page_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("first cached render");
        let cached_images = cache.image_xobject_entries();
        assert!(
            cached_images > 0,
            "image XObject render should retain decoded image data"
        );
        assert!(cache.image_xobject_bytes() > 0);

        let second = engine
            .render_page_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("second cached render");
        assert_eq!(cache.image_xobject_entries(), cached_images);
        assert_same_pixels(&first, &second);

        cache.clear();
        assert_eq!(cache.image_xobject_entries(), 0);
    }

    #[test]
    fn render_document_cache_retains_form_xobject_programs() {
        let pdf = pdf_with_repeated_form_xobject();
        let engine = ContentEngine::open_bytes(pdf).expect("open repeated Form XObject PDF");
        let mut cache = RenderDocumentCache::new();

        let first = engine
            .render_page_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("first cached form render");
        assert_eq!(
            cache.form_xobject_program_entries(),
            1,
            "repeated Form XObject should decode and parse into one cached program"
        );

        let second = engine
            .render_page_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("second cached form render");
        assert_eq!(cache.form_xobject_program_entries(), 1);
        assert_same_pixels(&first, &second);

        cache.clear();
        assert_eq!(cache.form_xobject_program_entries(), 0);
    }

    #[test]
    fn image_xobject_cache_evicts_least_recently_used_entry_by_byte_budget() {
        fn raw(bytes: usize) -> Arc<RawImage> {
            Arc::new(RawImage {
                width: bytes as u32,
                height: 1,
                channels: 1,
                bits_per_sample: 8,
                pixels: vec![0; bytes],
            })
        }

        let mut cache = HashMap::new();
        let mut order = VecDeque::new();
        let mut bytes = 0usize;

        insert_image_xobject_cache_entry(
            &mut cache,
            &mut order,
            &mut bytes,
            "a".to_string(),
            raw(10),
            10,
            20,
        );
        insert_image_xobject_cache_entry(
            &mut cache,
            &mut order,
            &mut bytes,
            "b".to_string(),
            raw(10),
            10,
            20,
        );
        touch_image_xobject_cache_key(&mut order, "a");
        insert_image_xobject_cache_entry(
            &mut cache,
            &mut order,
            &mut bytes,
            "c".to_string(),
            raw(10),
            10,
            20,
        );

        assert!(
            cache.contains_key("a"),
            "recently touched image should remain"
        );
        assert!(
            !cache.contains_key("b"),
            "least-recently-used image should evict"
        );
        assert!(cache.contains_key("c"));
        assert_eq!(bytes, 20);
        assert_eq!(
            order.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }

    #[test]
    fn shading_mesh_cache_evicts_least_recently_used_entry_by_byte_budget() {
        fn mesh(bytes: usize) -> Arc<Vec<u8>> {
            Arc::new(vec![0; bytes])
        }

        let mut cache = HashMap::new();
        let mut order = VecDeque::new();
        let mut bytes = 0usize;
        let entry = SHADING_MESH_CACHE_MAX_BYTES / 2;

        insert_shading_mesh_cache_entry(
            &mut cache,
            &mut order,
            &mut bytes,
            "a".to_string(),
            mesh(entry),
        );
        insert_shading_mesh_cache_entry(
            &mut cache,
            &mut order,
            &mut bytes,
            "b".to_string(),
            mesh(entry),
        );
        touch_shading_mesh_cache_key(&mut order, "a");
        insert_shading_mesh_cache_entry(
            &mut cache,
            &mut order,
            &mut bytes,
            "c".to_string(),
            mesh(entry),
        );

        assert!(cache.contains_key("a"));
        assert!(!cache.contains_key("b"));
        assert!(cache.contains_key("c"));
        assert_eq!(bytes, SHADING_MESH_CACHE_MAX_BYTES);
        assert_eq!(
            order.iter().map(String::as_str).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }

    #[test]
    fn display_list_band_stitch_matches_full_page() {
        let pdf = simple_vector_pdf("1 0 0 rg 0 0 100 50 re f\n0 0 1 rg 0 50 100 50 re f\n");
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let full = engine
            .render_page_display_list_with_mode(1, 72, RenderMode::Compat)
            .expect("display-list render")
            .expect("vector page should replay");
        let bands = engine
            .render_page_bands_with_mode(1, 72, 25, RenderMode::Compat)
            .expect("render bands");
        let stitched = stitch_vertical_bands(&bands, full.width, full.height);
        assert_same_pixels(&full, &stitched);
    }

    #[test]
    fn render_tile_cache_records_hit_and_budget() {
        let pdf = simple_vector_pdf("1 0 0 rg 0 0 100 100 re f\n");
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 50,
            height: 50,
        };
        let mut cache = RenderCache::new(20_000, 20_000);
        let first = engine
            .render_page_tile_with_mode(1, 72, tile, RenderMode::Compat, Some(&mut cache))
            .expect("first tile render");
        let second = engine
            .render_page_tile_with_mode(1, 72, tile, RenderMode::Compat, Some(&mut cache))
            .expect("cached tile render");

        assert_same_pixels(&first, &second);
        let metrics = cache.metrics();
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.inserts, 1);
        assert!(metrics.bytes <= 20_000);
    }

    #[test]
    fn display_list_replay_observes_pre_cancelled_token() {
        let pdf = simple_vector_pdf("1 0 0 rg 0 0 100 100 re f\n");
        let engine = ContentEngine::open_bytes(pdf).expect("open vector PDF");
        let list = engine
            .build_page_display_list(1, 72)
            .expect("build display list");
        let cancel = crate::cancel::CancelToken::new();
        cancel.cancel();

        let err = PageRenderer::render_display_list_cancellable_with_mode(
            &engine,
            1,
            72,
            &list,
            &cancel,
            RenderMode::Compat,
        )
        .expect_err("pre-cancelled replay should fail");
        assert!(matches!(err, WellfriendError::Cancelled(_)));
    }

    #[test]
    fn type0_font_decodes_two_byte_strings() {
        let bytes = [0x00u8, 0x48, 0x00, 0x69];
        let mut cmap = std::collections::HashMap::new();
        cmap.insert(72u32, 'H');
        cmap.insert(105u32, 'i');

        let chars: Vec<char> = bytes
            .chunks(2)
            .filter_map(|pair| {
                if pair.len() < 2 {
                    return None;
                }
                let cid = (u32::from(pair[0]) << 8) | u32::from(pair[1]);
                Some(cmap.get(&cid).copied().unwrap_or('\u{FFFD}'))
            })
            .collect();

        assert_eq!(chars, vec!['H', 'i']);
    }

    #[test]
    fn extract_glyph_path_by_gid_returns_positive_advance_for_gid_zero() {
        let font_bytes = get_fallback_font("Helvetica").expect("fallback font");
        let (_path, advance) =
            crate::render::glyph_outline::extract_glyph_path_by_gid(font_bytes, 0);
        assert!(advance > 0.0);
    }

    #[test]
    fn extract_glyph_path_by_gid_matches_char_lookup_for_ascii() {
        let font_bytes = get_fallback_font("Helvetica").expect("fallback font");
        let face = ttf_parser::Face::parse(font_bytes, 0).expect("parse fallback font");
        let gid_for_a = face.glyph_index('A').expect("glyph A").0;

        let (_path_by_char, adv_char) = RenderState::extract_glyph_path(font_bytes, 'A');
        let (_path_by_gid, adv_gid) =
            crate::render::glyph_outline::extract_glyph_path_by_gid(font_bytes, gid_for_a);

        assert!((adv_char - adv_gid).abs() < 1.0);
    }

    #[test]
    fn top_level_page_group_flattens_after_blending() {
        let pdf = simple_pdf_with_extgstate(
            "q /GS1 gs 1 0 0 rg 10 10 50 40 re f Q\n\
             q /GS2 gs 0 0 1 rg 35 30 50 40 re f Q",
            &[
                "<< /Type /ExtGState /ca 0.45 /CA 0.45 /BM /Multiply >>",
                "<< /Type /ExtGState /ca 0.55 /CA 0.55 /BM /Screen >>",
            ],
            "<< /Font << /F1 {font} 0 R >> /ExtGState << /GS1 {gs1} 0 R /GS2 {gs2} 0 R >> >>",
        );
        let engine = ContentEngine::open_bytes(pdf).expect("open transparency PDF");
        let buf = engine.render_page(1, 72).expect("render transparency PDF");

        assert_eq!(buf.get_pixel(5, 5), WHITE, "empty page area is white paper");
        let blue_only = buf.get_pixel(80, 40);
        assert!(
            blue_only[2] > 240 && blue_only[0] < 150 && blue_only[1] < 150,
            "Screen over initial transparent backdrop must survive final white flatten: {:?}",
            blue_only
        );
        let overlap = buf.get_pixel(45, 60);
        assert!(
            (overlap[0] as i32 - 178).abs() <= 2
                && (overlap[1] as i32 - 63).abs() <= 2
                && (overlap[2] as i32 - 203).abs() <= 2,
            "Screen over partially transparent red backdrop should match PDF blend math: {:?}",
            overlap
        );
    }

    #[test]
    fn alpha_smask_with_backdrop_color_keeps_source_visible_outside_group_paint() {
        let content = "q\n0.95 0.95 0.95 rg\n0 0 220 160 re\nf\nQ\n\
                       q\n/GS1 gs\n0.2 0.6 0.9 rg\n10 10 200 140 re\nf\nQ\n\
                       q\n0 0 0 RG\n1 w\n40 30 60 60 re\nS\nQ\n";
        let mask_content = "1 g\n40 30 60 60 re\nf\n";
        let pdf = pdf_with_alpha_smask(content, mask_content, "1", None);
        let engine = ContentEngine::open_bytes(pdf).expect("open alpha SMask PDF");
        let buf = engine.render_page(1, 72).expect("render alpha SMask PDF");

        let outside_mask_paint = buf.get_pixel(20, 20);
        assert!(
            outside_mask_paint[2] > 200
                && outside_mask_paint[0] < 80
                && outside_mask_paint[1] > 120,
            "alpha SMask /BC backdrop should make the whole blue source visible: {:?}",
            outside_mask_paint
        );
        let paper = buf.get_pixel(5, 5);
        assert!(
            (paper[0] as i32 - 242).abs() <= 2
                && (paper[1] as i32 - 242).abs() <= 2
                && (paper[2] as i32 - 242).abs() <= 2,
            "area outside the masked source should remain the gray page background: {:?}",
            paper
        );
    }

    #[test]
    fn display_list_replay_keeps_soft_mask_native_instead_of_page_fallback() {
        let content = "q\n0.95 0.95 0.95 rg\n0 0 220 160 re\nf\nQ\n\
                       q\n/GS1 gs\n0.2 0.6 0.9 rg\n10 10 200 140 re\nf\nQ\n";
        let mask_content = "1 g\n40 30 60 60 re\nf\n";
        let pdf = pdf_with_alpha_smask(content, mask_content, "1", None);
        let engine = ContentEngine::open_bytes(pdf).expect("open display-list SMask PDF");
        let list = engine
            .build_page_display_list(1, 72)
            .expect("build display-list SMask page");

        assert!(
            list.is_fully_supported(),
            "SMask ExtGState should replay through RenderState, not force immediate page fallback: {:?}",
            list.unsupported
        );

        let immediate = engine
            .render_page_cancellable_with_mode(1, 72, &CancelToken::none(), RenderMode::Compat)
            .expect("render immediate SMask page");
        let display_list = PageRenderer::render_display_list_cancellable_with_mode(
            &engine,
            1,
            72,
            &list,
            &CancelToken::none(),
            RenderMode::Compat,
        )
        .expect("render display-list SMask page");

        assert_eq!(
            immediate.to_raw_image_rgba().pixels,
            display_list.to_raw_image_rgba().pixels
        );
    }

    #[test]
    fn alpha_smask_transfer_zero_backdrop_is_not_double_applied() {
        let content = "q\n0.95 0.95 0.95 rg\n0 0 220 160 re\nf\nQ\n\
                       q\n/GS1 gs\n0.2 0.6 0.9 rg\n10 10 200 140 re\nf\nQ\n\
                       q\n0 0 0 RG\n1 w\n50 50 100 100 re\nS\nQ\n";
        let mask_content = "1 g\n50 50 100 100 re\nf\n";
        let transfer = "<< /FunctionType 2 /Domain [0 1] /C0 [0.5] /C1 [1] /N 1 /Range [0 1] >>";
        let pdf = pdf_with_alpha_smask(content, mask_content, "0.5", Some(transfer));
        let engine = ContentEngine::open_bytes(pdf).expect("open alpha SMask PDF");
        let buf = engine
            .render_page(1, 72)
            .expect("render alpha SMask transfer PDF");

        let outside_mask_paint = buf.get_pixel(20, 20);
        assert!(
            outside_mask_paint[0] > 120
                && outside_mask_paint[0] < 180
                && outside_mask_paint[1] > 180
                && outside_mask_paint[2] > 220,
            "TR(0) should provide the half-alpha backdrop, not be replaced by opaque /BC: {:?}",
            outside_mask_paint
        );
    }

    #[test]
    fn document_cache_reuses_soft_mask_groups() {
        let content = "q\n0.95 0.95 0.95 rg\n0 0 220 160 re\nf\nQ\n\
                       q\n/GS1 gs\n0.2 0.6 0.9 rg\n10 10 200 140 re\nf\nQ\n";
        let mask_content = "1 g\n40 30 60 60 re\nf\n";
        let pdf = pdf_with_alpha_smask(content, mask_content, "1", None);
        let engine = ContentEngine::open_bytes(pdf).expect("open cached SMask PDF");
        let mut cache = RenderDocumentCache::new();

        let first = engine
            .render_page_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("first cached SMask render");
        assert_eq!(cache.smask_group_entries(), 1);

        let second = engine
            .render_page_cancellable_with_mode_and_cache(
                1,
                72,
                &CancelToken::none(),
                RenderMode::Compat,
                &mut cache,
            )
            .expect("second cached SMask render");
        assert_eq!(cache.smask_group_entries(), 1);
        assert_eq!(
            first.to_raw_image_rgba().pixels,
            second.to_raw_image_rgba().pixels
        );
    }

    #[test]
    fn colored_tiling_pattern_paints_stroked_paths() {
        let pdf = pdf_with_colored_tiling_pattern_stroke();
        let engine = ContentEngine::open_bytes(pdf).expect("open pattern stroke PDF");
        let buf = engine
            .render_page(1, 72)
            .expect("render pattern stroke PDF");

        let red_band = first_pixel_in_region(&buf, 10..18, 44..57, |px| px[0] > 180 && px[2] < 120);
        assert!(
            red_band.is_some(),
            "left pattern stripe should paint the stroke red, got {:?}",
            red_band
        );
        let blue_band =
            first_pixel_in_region(&buf, 15..24, 44..57, |px| px[2] > 180 && px[0] < 120);
        assert!(
            blue_band.is_some(),
            "right pattern stripe should paint the stroke blue, got {:?}",
            blue_band
        );
    }

    #[test]
    fn tiling_pattern_inside_form_renders_tile_content() {
        let pdf = pdf_with_form_tiling_pattern_fill();
        let engine = ContentEngine::open_bytes(pdf).expect("open Form pattern PDF");
        let buf = engine
            .render_page_with_mode(1, 72, RenderMode::Compat)
            .expect("render Form pattern PDF");

        assert!(
            count_red_pixels(&buf) > 300,
            "pattern inside Form should retain red tile stripes"
        );
        assert!(
            count_blue_pixels(&buf) > 300,
            "pattern inside Form should retain blue tile stripes"
        );
    }

    #[test]
    fn recursive_tiling_pattern_returns_typed_refusal() {
        let pdf = pdf_with_recursive_tiling_pattern();
        let engine = ContentEngine::open_bytes(pdf).expect("open recursive pattern PDF");
        let error = engine
            .render_page_with_mode(1, 72, RenderMode::Compat)
            .expect_err("recursive pattern must not silently approximate to solid color");
        assert!(format!("{error}").contains("recursive tiling pattern"));
    }

    #[test]
    fn shading_resource_color_space_uses_page_color_space_map() {
        let pdf = pdf_with_resource_separation_axial_shading();
        let engine = ContentEngine::open_bytes(pdf).expect("open resource shading PDF");
        let buf = engine
            .render_page_with_mode(1, 72, RenderMode::Compat)
            .expect("render resource shading PDF");

        assert!(
            count_red_pixels(&buf) > 500,
            "named shading ColorSpace should resolve through page resources"
        );
    }

    #[test]
    fn type3_charproc_renders_resource_xobject_image() {
        let pdf = pdf_with_type3_image_charproc();
        let engine = ContentEngine::open_bytes(pdf).expect("open Type3 image charproc PDF");
        let buf = engine
            .render_page_with_mode(1, 72, RenderMode::HighQuality)
            .expect("render Type3 image charproc PDF");

        let red_pixels = count_red_pixels(&buf);
        assert!(
            red_pixels > 100,
            "resource-backed Type3 charproc should paint its image, red_pixels={red_pixels}"
        );
    }

    #[test]
    fn resource_indexed_image_color_space_drives_xobject_decode() {
        let pdf = pdf_with_resource_indexed_image_colorspace();
        let engine = ContentEngine::open_bytes(pdf).expect("open resource Indexed image PDF");
        let buf = engine
            .render_page_with_mode(1, 72, RenderMode::Compat)
            .expect("render resource Indexed image PDF");

        assert!(
            count_red_pixels(&buf) > 500,
            "first lookup-table entry should render as red"
        );
        assert!(
            count_blue_pixels(&buf) > 500,
            "second lookup-table entry should render as blue"
        );
    }

    #[test]
    fn resource_separation_image_color_space_uses_tint_transform() {
        let pdf = pdf_with_resource_separation_image_colorspace();
        let engine = ContentEngine::open_bytes(pdf).expect("open resource Separation image PDF");
        let buf = engine
            .render_page_with_mode(1, 72, RenderMode::Compat)
            .expect("render resource Separation image PDF");

        assert!(
            count_red_pixels(&buf) > 500,
            "Separation image tint transform should render high-tint samples as red"
        );
    }

    fn first_pixel_in_region(
        buf: &PixelBuffer,
        xs: std::ops::Range<i32>,
        ys: std::ops::Range<i32>,
        predicate: impl Fn(PixelColor) -> bool,
    ) -> Option<PixelColor> {
        for y in ys {
            for x in xs.clone() {
                let px = buf.get_pixel(x, y);
                if predicate(px) {
                    return Some(px);
                }
            }
        }
        None
    }

    fn pdf_with_resource_indexed_image_colorspace() -> Vec<u8> {
        let content = "q\n80 0 0 40 10 30 cm\n/Im1 Do\nQ\n";
        let image_bytes = [0u8, 1u8];
        let mut image_stream =
            b"<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /CS1 /BitsPerComponent 8 /Length 2 >>\nstream\n".to_vec();
        image_stream.extend_from_slice(&image_bytes);
        image_stream.extend_from_slice(b"\nendstream");
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /ColorSpace << /CS1 [/Indexed /DeviceRGB 1 <FF00000000FF>] >> /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content)
                .into_bytes(),
            image_stream,
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn pdf_with_resource_separation_image_colorspace() -> Vec<u8> {
        let content = "q\n80 0 0 40 10 30 cm\n/Im1 Do\nQ\n";
        let image_bytes = [0u8, 255u8];
        let mut image_stream =
            b"<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /CS1 /BitsPerComponent 8 /Length 2 >>\nstream\n".to_vec();
        image_stream.extend_from_slice(&image_bytes);
        image_stream.extend_from_slice(b"\nendstream");
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /ColorSpace << /CS1 [/Separation /SpotRed /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] >> /XObject << /Im1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", content.len(), content)
                .into_bytes(),
            image_stream,
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn pdf_with_colored_tiling_pattern_stroke() -> Vec<u8> {
        let content = "/Pattern CS /P1 SCN\n8 w\n10 50 m 90 50 l\nS\n";
        let pattern = "1 0 0 rg\n0 0 5 10 re f\n0 0 1 rg\n5 0 5 10 re f\n";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] \
              /Resources << /Pattern << /P1 5 0 R >> >> /Contents 4 0 R >>"
                .to_vec(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                content
            )
            .into_bytes(),
            format!(
                "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 \
                 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\n\
                 stream\n{}\nendstream",
                pattern.len(),
                pattern
            )
            .into_bytes(),
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn pdf_with_recursive_tiling_pattern() -> Vec<u8> {
        let content = "/Pattern cs /P1 scn\n0 0 100 100 re f\n";
        let pattern = "/Pattern cs /P1 scn\n0 0 10 10 re f\n";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] \
              /Resources << /Pattern << /P1 5 0 R >> >> /Contents 4 0 R >>"
                .to_vec(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                content
            )
            .into_bytes(),
            format!(
                "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 \
                 /BBox [0 0 10 10] /XStep 10 /YStep 10 \
                 /Resources << /Pattern << /P1 5 0 R >> >> /Length {} >>\n\
                 stream\n{}\nendstream",
                pattern.len(),
                pattern
            )
            .into_bytes(),
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn pdf_with_form_tiling_pattern_fill() -> Vec<u8> {
        let page_content = "q\n1 0 0 1 0 0 cm\n/Fm1 Do\nQ\n";
        let form_content = "/Pattern cs /P1 scn\n10 10 80 80 re f\n";
        let pattern = "1 0 0 rg\n0 0 5 10 re f\n0 0 1 rg\n5 0 5 10 re f\n";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                page_content.len(),
                page_content
            )
            .into_bytes(),
            format!(
                "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 100 100] /Resources << /Pattern << /P1 6 0 R >> >> /Length {} >>\nstream\n{}\nendstream",
                form_content.len(),
                form_content
            )
            .into_bytes(),
            format!(
                "<< /Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length {} >>\nstream\n{}\nendstream",
                pattern.len(),
                pattern
            )
            .into_bytes(),
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn pdf_with_repeated_form_xobject() -> Vec<u8> {
        let page_content = "q\n1 0 0 1 0 0 cm\n/Fm1 Do\nQ\nq\n1 0 0 1 35 35 cm\n/Fm1 Do\nQ\n";
        let form_content = "1 0 0 rg\n0 0 25 25 re f\n0 0 1 RG\n0 0 25 25 re S\n";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Fm1 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                page_content.len(),
                page_content
            )
            .into_bytes(),
            format!(
                "<< /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 25 25] /Resources << >> /Length {} >>\nstream\n{}\nendstream",
                form_content.len(),
                form_content
            )
            .into_bytes(),
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn pdf_with_resource_separation_axial_shading() -> Vec<u8> {
        let content = "q\n10 10 80 80 re W n\n/Sh1 sh\nQ\n";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /ColorSpace << /CS1 [/Separation /SpotRed /DeviceRGB << /FunctionType 2 /Domain [0 1] /Range [0 1 0 1 0 1] /C0 [1 1 1] /C1 [1 0 0] /N 1 >>] >> /Shading << /Sh1 << /ShadingType 2 /ColorSpace /CS1 /Coords [10 50 90 50] /Domain [0 1] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /Range [0 1] /C0 [0] /C1 [1] /N 1 >> >> >> >> /Contents 4 0 R >>".to_vec(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                content
            )
            .into_bytes(),
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn pdf_with_type3_image_charproc() -> Vec<u8> {
        let content = "BT /F1 60 Tf 20 20 Td (A) Tj ET";
        let charproc = "600 0 d0 q 1000 0 0 1000 0 0 cm /Im1 Do Q";
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] \
              /Resources << /Font << /F1 4 0 R >> >> /Contents 7 0 R >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /Type3 /Name /F1 /FontBBox [0 0 1000 1000] \
              /FontMatrix [0.001 0 0 0.001 0 0] /FirstChar 65 /LastChar 65 /Widths [600] \
              /Encoding << /Type /Encoding /Differences [65 /A] >> \
              /CharProcs << /A 5 0 R >> /Resources << /XObject << /Im1 6 0 R >> >> >>"
                .to_vec(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                charproc.len(),
                charproc
            )
            .into_bytes(),
            b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n\xff\x00\x00\nendstream".to_vec(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                content
            )
            .into_bytes(),
        ];
        let mut out = bytearray_pdf_header();
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
        out
    }

    fn pdf_with_alpha_smask(
        content: &str,
        mask_content: &str,
        bc: &str,
        transfer: Option<&str>,
    ) -> Vec<u8> {
        let transfer_ref = transfer.map(|_| " /TR 7 0 R").unwrap_or("");
        let mut objects = vec![
            b"<< /Pages 2 0 R /Type /Catalog >>".to_vec(),
            b"<< /Count 1 /Kids [3 0 R] /Type /Pages >>".to_vec(),
            b"<< /Contents 4 0 R /MediaBox [0 0 220 160] /Parent 2 0 R \
              /Resources << /ExtGState << /GS1 5 0 R >> >> /Type /Page >>"
                .to_vec(),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                content
            )
            .into_bytes(),
            format!(
                "<< /SMask << /BC [{}] /G 6 0 R /S /Alpha{} /Type /Mask >> /Type /ExtGState >>",
                bc, transfer_ref
            )
            .into_bytes(),
            format!(
                "<< /BBox [40 30 100 90] /FormType 1 /Group << /CS /DeviceGray /S /Transparency /Type /Group >> \
                 /Resources << >> /Subtype /Form /Type /XObject /Length {} >>\nstream\n{}\nendstream",
                mask_content.len(),
                mask_content
            )
            .into_bytes(),
        ];
        if let Some(transfer) = transfer {
            objects.push(transfer.as_bytes().to_vec());
        }

        let mut out = bytearray_pdf_header();
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
        out
    }

    #[test]
    fn annotation_appearance_stream_renders_selected_state_by_default() {
        let pdf = pdf_with_annotation_appearance(0, true, false);
        let engine = ContentEngine::open_bytes(pdf).expect("open annotation PDF");
        let buf = PageRenderer::render_page(&engine, 1, 72).expect("render annotation PDF");

        assert!(
            count_red_pixels(&buf) > 100,
            "selected /AS appearance should paint the widget"
        );
        assert_eq!(
            count_blue_pixels(&buf),
            0,
            "the /Off appearance must not be rendered when /AS selects /Yes"
        );
    }

    #[test]
    fn hidden_annotation_appearance_is_not_rendered() {
        let pdf = pdf_with_annotation_appearance(2, false, false);
        let engine = ContentEngine::open_bytes(pdf).expect("open hidden annotation PDF");
        let buf = PageRenderer::render_page(&engine, 1, 72).expect("render hidden annotation PDF");

        assert_eq!(
            count_red_pixels(&buf),
            0,
            "hidden annotations must not render their appearance streams"
        );
    }

    #[test]
    fn need_appearances_does_not_override_existing_widget_appearance() {
        let pdf = pdf_with_annotation_appearance(0, true, true);
        let engine = ContentEngine::open_bytes(pdf).expect("open NeedAppearances PDF");
        let buf = PageRenderer::render_page(&engine, 1, 72).expect("render NeedAppearances PDF");

        assert!(
            count_red_pixels(&buf) > 100,
            "a usable author-provided /AP stream should still take precedence"
        );
    }

    #[test]
    fn highlight_without_appearance_synthesizes_markup() {
        let annot = b"<< /Type /Annot /Subtype /Highlight /Rect [20 30 80 50] \
                       /QuadPoints [20 50 80 50 20 30 80 30] /C [1 1 0] /CA 0.5 >>";
        let pdf = pdf_with_form_objects(vec![annot.to_vec()], "5 0 R", "", "");
        let engine = ContentEngine::open_bytes(pdf).expect("open missing-AP highlight PDF");
        let buf = PageRenderer::render_page(&engine, 1, 72).expect("render highlight PDF");

        let yellow_pixels = count_pixels_matching(&buf, |pixel| {
            pixel[0] > 180 && pixel[1] > 180 && pixel[2] < 160
        });
        assert!(
            yellow_pixels > 100,
            "missing-AP highlight annotation should synthesize a visible markup appearance"
        );
    }

    #[test]
    fn text_widget_without_appearance_synthesizes_value_from_da() {
        let widget = b"<< /Type /Annot /Subtype /Widget /Rect [20 35 90 60] \
                       /FT /Tx /T (name) /V (Hi) /DA (/F1 14 Tf 0 0 1 rg) /Q 1 >>";
        let pdf = pdf_with_form_objects(vec![widget.to_vec()], "5 0 R", "5 0 R", "");
        let engine = ContentEngine::open_bytes(pdf).expect("open missing-AP text widget PDF");
        let buf = PageRenderer::render_page(&engine, 1, 72).expect("render text widget PDF");

        assert!(
            count_blue_pixels(&buf) > 5,
            "text widget synthesis should honor blue fill color from /DA"
        );
    }

    #[test]
    fn checkbox_without_appearance_synthesizes_checked_and_unchecked_states() {
        let checked = b"<< /Type /Annot /Subtype /Widget /Rect [25 25 55 55] \
                         /FT /Btn /T (agree) /V /Yes /AS /Yes >>";
        let unchecked = b"<< /Type /Annot /Subtype /Widget /Rect [25 25 55 55] \
                           /FT /Btn /T (agree) /V /Off /AS /Off >>";
        let checked_pdf = pdf_with_form_objects(
            vec![checked.to_vec()],
            "5 0 R",
            "5 0 R",
            "/NeedAppearances true",
        );
        let unchecked_pdf = pdf_with_form_objects(
            vec![unchecked.to_vec()],
            "5 0 R",
            "5 0 R",
            "/NeedAppearances true",
        );
        let checked_engine = ContentEngine::open_bytes(checked_pdf).expect("open checked PDF");
        let unchecked_engine =
            ContentEngine::open_bytes(unchecked_pdf).expect("open unchecked PDF");
        let checked_buf =
            PageRenderer::render_page(&checked_engine, 1, 72).expect("render checked PDF");
        let unchecked_buf =
            PageRenderer::render_page(&unchecked_engine, 1, 72).expect("render unchecked PDF");

        assert!(
            count_dark_pixels(&checked_buf) > count_dark_pixels(&unchecked_buf) + 10,
            "checked checkbox synthesis should add a visible check mark"
        );
    }

    #[test]
    fn radio_widget_without_appearance_uses_parent_value() {
        let parent = b"<< /FT /Btn /Ff 32768 /V /Choice /Kids [6 0 R] >>";
        let widget = b"<< /Type /Annot /Subtype /Widget /Parent 5 0 R \
                       /Rect [25 25 55 55] /AS /Choice >>";
        let pdf = pdf_with_form_objects(
            vec![parent.to_vec(), widget.to_vec()],
            "6 0 R",
            "5 0 R",
            "/NeedAppearances true",
        );
        let engine = ContentEngine::open_bytes(pdf).expect("open missing-AP radio PDF");
        let buf = PageRenderer::render_page(&engine, 1, 72).expect("render radio PDF");

        assert!(
            count_dark_pixels(&buf) > 140,
            "selected radio synthesis should draw both ring and inner marker"
        );
    }

    #[test]
    fn pushbutton_without_appearance_synthesizes_caption() {
        let widget = b"<< /Type /Annot /Subtype /Widget /Rect [20 35 85 60] \
                       /FT /Btn /Ff 65536 /MK << /CA (Go) /BG [0.8 0.8 0.8] >> >>";
        let pdf = pdf_with_form_objects(vec![widget.to_vec()], "5 0 R", "5 0 R", "");
        let engine = ContentEngine::open_bytes(pdf).expect("open missing-AP pushbutton PDF");
        let buf = PageRenderer::render_page(&engine, 1, 72).expect("render pushbutton PDF");

        assert!(
            count_gray_pixels(&buf) > 500 && count_dark_pixels(&buf) > 50,
            "pushbutton synthesis should draw its background, border, and caption"
        );
    }

    #[test]
    fn choice_widget_without_appearance_synthesizes_selected_value() {
        let widget = b"<< /Type /Annot /Subtype /Widget /Rect [15 35 95 60] \
                       /FT /Ch /Ff 131072 /V (Banana) /Opt [(Apple) (Banana)] \
                       /DA (/F1 12 Tf 0 g) >>";
        let pdf = pdf_with_form_objects(vec![widget.to_vec()], "5 0 R", "5 0 R", "");
        let engine = ContentEngine::open_bytes(pdf).expect("open missing-AP choice PDF");
        let buf = PageRenderer::render_page(&engine, 1, 72).expect("render choice PDF");

        assert!(
            count_dark_pixels(&buf) > 20,
            "choice synthesis should render the selected option text"
        );
    }

    #[test]
    fn hidden_widget_without_appearance_is_not_synthesized() {
        let widget = b"<< /Type /Annot /Subtype /Widget /Rect [20 35 90 60] /F 2 \
                       /FT /Tx /T (hidden) /V (Hidden) /DA (/F1 14 Tf 0 g) >>";
        let pdf = pdf_with_form_objects(vec![widget.to_vec()], "5 0 R", "5 0 R", "");
        let engine = ContentEngine::open_bytes(pdf).expect("open hidden missing-AP PDF");
        let buf = PageRenderer::render_page(&engine, 1, 72).expect("render hidden PDF");

        assert_eq!(
            count_dark_pixels(&buf),
            0,
            "hidden missing-appearance widgets must stay hidden"
        );
    }

    #[test]
    fn font_rendering_regression_check() {
        let engine = ContentEngine::open_path(fixture("flate.pdf")).expect("open flate fixture");
        let buf = engine.render_page(1, 72).expect("render page");
        let raw = buf.to_raw_image();
        let channels = raw.channels as usize;
        let non_white = raw
            .pixels
            .chunks(channels)
            .filter(|pixel| pixel[0] < 200 || pixel[1] < 200 || pixel[2] < 200)
            .count();
        assert!(non_white > 20);
    }

    fn pdf_with_annotation_appearance(
        flags: i64,
        stateful: bool,
        need_appearances: bool,
    ) -> Vec<u8> {
        fn stream(body: &str, dict_extra: &str) -> Vec<u8> {
            format!(
                "<< {} /Length {} >>\nstream\n{}\nendstream",
                dict_extra,
                body.len(),
                body
            )
            .into_bytes()
        }

        let content = stream("", "");
        let red_appearance = stream(
            "1 0 0 rg 0 0 50 50 re f\n",
            "/Type /XObject /Subtype /Form /BBox [0 0 50 50] /Resources << >>",
        );
        let blue_appearance = stream(
            "0 0 1 rg 0 0 50 50 re f\n",
            "/Type /XObject /Subtype /Form /BBox [0 0 50 50] /Resources << >>",
        );

        let annot = if stateful {
            format!(
                "<< /Type /Annot /Subtype /Widget /Rect [20 20 70 70] /F {} \
                 /AS /Yes /AP << /N << /Yes 6 0 R /Off 7 0 R >> >> >>",
                flags
            )
        } else {
            format!(
                "<< /Type /Annot /Subtype /Widget /Rect [20 20 70 70] /F {} \
                 /AP << /N 6 0 R >> >>",
                flags
            )
        };

        let catalog = if need_appearances {
            b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /NeedAppearances true >> >>".to_vec()
        } else {
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()
        };

        let objects: Vec<Vec<u8>> = vec![
            catalog,
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 4 0 R /Annots [5 0 R] >>".to_vec(),
            content,
            annot.into_bytes(),
            red_appearance,
            blue_appearance,
        ];

        let mut out = bytearray_pdf_header();
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
        out
    }

    fn pdf_with_form_objects(
        form_objects: Vec<Vec<u8>>,
        page_annots: &str,
        acroform_fields: &str,
        acroform_extra: &str,
    ) -> Vec<u8> {
        fn stream(body: &str, dict_extra: &str) -> Vec<u8> {
            format!(
                "<< {} /Length {} >>\nstream\n{}\nendstream",
                dict_extra,
                body.len(),
                body
            )
            .into_bytes()
        }

        let font_number = 5 + form_objects.len();
        let catalog = format!(
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [{}] \
             /DA (/F1 12 Tf 0 g) /DR << /Font << /F1 {} 0 R >> >> {} >> >>",
            acroform_fields, font_number, acroform_extra
        )
        .into_bytes();
        let content = stream("", "");
        let mut objects: Vec<Vec<u8>> = vec![
            catalog,
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] \
                 /Resources << >> /Contents 4 0 R /Annots [{}] >>",
                page_annots
            )
            .into_bytes(),
            content,
        ];
        objects.extend(form_objects);
        objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());

        let mut out = bytearray_pdf_header();
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
        out
    }

    fn count_red_pixels(buf: &PixelBuffer) -> usize {
        count_pixels_matching(buf, |pixel| {
            pixel[0] > 200 && pixel[1] < 80 && pixel[2] < 80
        })
    }

    fn count_blue_pixels(buf: &PixelBuffer) -> usize {
        count_pixels_matching(buf, |pixel| {
            pixel[2] > 200 && pixel[0] < 80 && pixel[1] < 80
        })
    }

    fn count_dark_pixels(buf: &PixelBuffer) -> usize {
        count_pixels_matching(buf, |pixel| pixel[0] < 80 && pixel[1] < 80 && pixel[2] < 80)
    }

    fn count_gray_pixels(buf: &PixelBuffer) -> usize {
        count_pixels_matching(buf, |pixel| {
            (pixel[0] as i16 - pixel[1] as i16).abs() < 4
                && (pixel[1] as i16 - pixel[2] as i16).abs() < 4
                && pixel[0] > 150
                && pixel[0] < 235
        })
    }

    fn count_pixels_matching(buf: &PixelBuffer, pred: impl Fn(PixelColor) -> bool) -> usize {
        let mut count = 0usize;
        for y in 0..buf.height {
            for x in 0..buf.width {
                if pred(buf.get_pixel(x as i32, y as i32)) {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn composite_group_with_full_alpha_paints_source() {
        let mut dst = PixelBuffer::new_filled(1, 1, WHITE);
        let mut src = PixelBuffer::new_transparent(1, 1);
        src.blend_pixel(0, 0, RED, 1.0);

        dst.composite_from(&src, 1.0, BlendMode::Normal, None);
        let result = dst.get_pixel(0, 0);
        assert!(result[0] > 200, "group should paint red: {:?}", result);
        assert!(result[1] < 50, "green channel should be low: {:?}", result);
    }

    #[test]
    fn composite_group_with_half_alpha_blends_with_destination() {
        let mut dst = PixelBuffer::new_filled(1, 1, WHITE);
        let mut src = PixelBuffer::new_transparent(1, 1);
        src.blend_pixel(0, 0, BLACK, 1.0);

        dst.composite_from(&src, 0.5, BlendMode::Normal, None);
        let result = dst.get_pixel(0, 0);
        assert!(
            result[0] > 100 && result[0] < 200,
            "50% black over white should be gray: {:?}",
            result
        );
    }

    #[test]
    fn is_transparency_group_detects_group_subtype() {
        let mut dict = PdfDictionary::empty();
        let mut group = PdfDictionary::empty();
        group.insert("S", PdfObject::Name("Transparency".to_string()));
        dict.insert("Group", PdfObject::Dictionary(group));
        assert!(is_transparency_group(&dict));
        assert!(!is_transparency_group(&PdfDictionary::empty()));
    }

    #[test]
    fn document_font_cache_key_includes_resource_dictionary() {
        let mut winansi = PdfDictionary::empty();
        winansi.insert("Subtype", PdfObject::Name("Type1".to_string()));
        winansi.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
        winansi.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));

        let mut macroman = PdfDictionary::empty();
        macroman.insert("Subtype", PdfObject::Name("Type1".to_string()));
        macroman.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
        macroman.insert("Encoding", PdfObject::Name("MacRomanEncoding".to_string()));

        assert_ne!(
            font_resource_cache_key("F1", &winansi),
            font_resource_cache_key("F1", &macroman)
        );
    }

    #[test]
    fn type3_charproc_cache_key_includes_resource_dictionary() {
        let mut first = PdfDictionary::empty();
        first.insert("Subtype", PdfObject::Name("Type3".to_string()));
        first.insert(
            "FontMatrix",
            PdfObject::Array(vec![
                PdfObject::Real(0.001),
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Real(0.001),
                PdfObject::Integer(0),
                PdfObject::Integer(0),
            ]),
        );

        let mut second = first.clone();
        second.insert(
            "FontMatrix",
            PdfObject::Array(vec![
                PdfObject::Real(0.002),
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Real(0.002),
                PdfObject::Integer(0),
                PdfObject::Integer(0),
            ]),
        );

        assert_ne!(
            type3_charproc_cache_key("F1", &first, "A"),
            type3_charproc_cache_key("F1", &second, "A")
        );
    }

    #[test]
    fn document_glyph_cache_hash_includes_resource_identity() {
        let font_bytes = b"same embedded font bytes";
        let base = font_resource_glyph_cache_hash(font_bytes, "F1:aaaaaaaaaaaaaaaa");
        let remapped = font_resource_glyph_cache_hash(font_bytes, "F1:bbbbbbbbbbbbbbbb");

        assert_ne!(base, remapped);
    }

    #[test]
    fn explicit_color_key_mask_makes_matching_rgb_pixels_transparent() {
        let main = crate::images::decoder::RawImage {
            width: 2,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30, 200, 210, 220],
        };
        let mask = vec![
            PdfObject::Integer(10),
            PdfObject::Integer(10),
            PdfObject::Integer(20),
            PdfObject::Integer(20),
            PdfObject::Integer(30),
            PdfObject::Integer(30),
        ];
        let out = apply_color_key_image_mask(main, 8, "DeviceRGB", &mask).expect("color key mask");
        assert_eq!(out.channels, 4);
        assert_eq!(&out.pixels[0..4], &[10, 20, 30, 0]);
        assert_eq!(&out.pixels[4..8], &[200, 210, 220, 255]);
    }

    #[test]
    fn explicit_stencil_mask_combines_as_alpha() {
        let main = crate::images::decoder::RawImage {
            width: 2,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![50, 60, 70, 80, 90, 100],
        };
        let mask = crate::images::decoder::RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![255, 0],
        };
        let out = combine_explicit_image_mask(main, &mask, true, true).expect("stencil mask");
        assert_eq!(out.channels, 4);
        assert_eq!(&out.pixels[0..4], &[50, 60, 70, 255]);
        assert_eq!(&out.pixels[4..8], &[80, 90, 100, 0]);
    }

    #[test]
    fn q_restore_restores_previous_smask_and_blend_mode() {
        let engine = ContentEngine::open_path(fixture("flate.pdf")).expect("open flate fixture");
        let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        let buf = PixelBuffer::new_filled(10, 10, WHITE);
        let mut state = RenderState::new(buf, viewport, PageResources::default(), &engine, 1);

        state.dispatch(&ContentOperation::new("q", Vec::new()));
        state.buf.set_smask(AlphaMask::all_opaque(10, 10));
        state.buf.blend_mode = BlendMode::Multiply;
        state.dispatch(&ContentOperation::new("Q", Vec::new()));

        assert!(state.buf.smask_mask().is_none());
        assert_eq!(state.buf.blend_mode, BlendMode::Normal);
    }

    #[test]
    fn clip_mask_all_visible_pixels_are_visible() {
        let clip = ClipMask::all_visible(10, 10);
        assert!(clip.is_visible(0, 0));
        assert!(clip.is_visible(9, 9));
        assert!(clip.is_visible(5, 5));
    }

    #[test]
    fn clip_mask_set_and_is_visible() {
        let mut clip = ClipMask::all_visible(10, 10);
        clip.set(5, 5, false);
        assert!(!clip.is_visible(5, 5));
        assert!(clip.is_visible(4, 5));
    }

    #[test]
    fn clip_mask_out_of_bounds_is_visible() {
        let clip = ClipMask::all_visible(10, 10);
        assert!(clip.is_visible(-1, 0));
        assert!(clip.is_visible(10, 0));
        assert!(clip.is_visible(0, -1));
        assert!(clip.is_visible(0, 10));
    }

    #[test]
    fn clip_mask_intersect_produces_and_of_two_masks() {
        let mut a = ClipMask::all_visible(4, 1);
        let mut b = ClipMask::all_visible(4, 1);
        a.set(0, 0, false);
        b.set(1, 0, false);
        a.intersect(&b);
        assert!(!a.is_visible(0, 0));
        assert!(!a.is_visible(1, 0));
        assert!(a.is_visible(2, 0));
        assert!(a.is_visible(3, 0));
    }

    #[test]
    fn clip_mask_from_path_for_simple_rectangle() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.rect(20.0, 20.0, 60.0, 60.0);
        let flat = flatten_path(&path, &ctm, &vp, 0.5);
        let clip = ClipMask::from_path(&flat, 100, 100, FillRule::NonZero);
        println!(
            "clip rect center={}, corner={}",
            clip.is_visible(50, 50),
            clip.is_visible(5, 5)
        );
        assert!(clip.is_visible(50, 50));
        assert!(!clip.is_visible(5, 5));
        assert!(!clip.is_visible(90, 90));
    }

    #[test]
    fn blend_pixel_respects_clip_mask() {
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        let mut clip = ClipMask::all_visible(10, 10);
        clip.set(5, 5, false);
        buf.set_clip(clip);
        buf.blend_pixel(5, 5, RED, 1.0);
        assert_eq!(buf.get_pixel(5, 5), WHITE);
        buf.blend_pixel(3, 3, RED, 1.0);
        assert!(buf.get_pixel(3, 3)[0] > 100);
    }

    #[test]
    fn clear_clip_restores_all_visible() {
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        let mut clip = ClipMask::all_visible(10, 10);
        clip.fill_rect(0, 0, 10, 10, false);
        buf.set_clip(clip);
        buf.blend_pixel(5, 5, RED, 1.0);
        assert_eq!(buf.get_pixel(5, 5), WHITE);
        buf.clear_clip();
        buf.blend_pixel(5, 5, RED, 1.0);
        assert!(buf.get_pixel(5, 5)[0] > 100);
    }

    #[test]
    fn clip_mask_fill_rect_marks_region_clipped() {
        let mut clip = ClipMask::all_visible(20, 20);
        clip.fill_rect(5, 5, 10, 10, false);
        assert!(!clip.is_visible(5, 5));
        assert!(!clip.is_visible(14, 14));
        assert!(clip.is_visible(4, 5));
        assert!(clip.is_visible(5, 4));
    }

    #[test]
    fn clip_mask_fill_rect_visible_restores_pixels() {
        let mut clip = ClipMask::all_visible(10, 10);
        clip.fill_rect(0, 0, 10, 10, false);
        clip.fill_rect(3, 3, 4, 4, true);
        assert!(!clip.is_visible(2, 2));
        assert!(clip.is_visible(5, 5));
    }

    #[test]
    fn clip_mask_from_path_evenodd_nested_rects() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.rect(10.0, 10.0, 80.0, 80.0);
        path.rect(30.0, 30.0, 40.0, 40.0);
        let flat = flatten_path(&path, &ctm, &vp, 0.5);
        let clip_eo = ClipMask::from_path(&flat, 100, 100, FillRule::EvenOdd);
        let clip_nz = ClipMask::from_path(&flat, 100, 100, FillRule::NonZero);
        assert!(!clip_eo.is_visible(50, 50));
        assert!(clip_nz.is_visible(50, 50));
    }

    #[test]
    fn clip_is_preserved_across_simple_paint_operations() {
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut clip = ClipMask::all_visible(100, 100);
        clip.fill_rect(50, 0, 50, 100, false);
        buf.set_clip(clip);
        buf.fill_rect(0, 0, 100, 100, RED);
        assert_eq!(buf.get_pixel(25, 50), RED);
        assert_eq!(buf.get_pixel(75, 50), WHITE);
    }

    #[test]
    fn set_clip_intersects_with_existing_clip() {
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        let mut clip1 = ClipMask::all_visible(10, 10);
        clip1.fill_rect(5, 0, 5, 10, false);
        buf.set_clip(clip1);
        let mut clip2 = ClipMask::all_visible(10, 10);
        clip2.fill_rect(0, 5, 10, 5, false);
        buf.set_clip(clip2);
        let clip = buf.clip_mask().expect("clip should be installed");
        assert!(clip.is_visible(2, 2));
        assert!(!clip.is_visible(7, 2));
        assert!(!clip.is_visible(2, 7));
        assert!(!clip.is_visible(7, 7));
    }

    #[test]
    fn q_restore_restores_previous_clip_mask() {
        let engine = ContentEngine::open_path(fixture("flate.pdf")).expect("open flate fixture");
        let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        let mut left_clip = ClipMask::all_visible(10, 10);
        left_clip.fill_rect(5, 0, 5, 10, false);
        buf.set_clip(left_clip);

        let mut state = RenderState::new(buf, viewport, PageResources::default(), &engine, 1);
        state.dispatch(&ContentOperation::new("q", Vec::new()));

        let mut top_clip = ClipMask::all_visible(10, 10);
        top_clip.fill_rect(0, 5, 10, 5, false);
        state.buf.set_clip(top_clip);
        state.dispatch(&ContentOperation::new("Q", Vec::new()));

        let clip = state.buf.clip_mask().expect("clip should be restored");
        assert!(
            clip.is_visible(2, 7),
            "left clip should restore bottom-left visibility"
        );
        assert!(
            !clip.is_visible(7, 2),
            "left clip should keep right side clipped"
        );
    }

    #[test]
    fn render_page_on_text_pdf_returns_non_trivial_buffer() {
        let engine = ContentEngine::open_path(fixture("flate.pdf")).expect("open flate fixture");
        let buf = engine.render_page(1, 72).expect("render page");
        assert_eq!(buf.width, 612);
        assert_eq!(buf.height, 792);
        let png = ImageEncoder::encode_png(&buf.to_raw_image()).expect("encode png");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn render_page_on_image_pdf_returns_modified_buffer() {
        let engine =
            ContentEngine::open_path(fixture("image_only.pdf")).expect("open image fixture");
        let buf = engine.render_page(1, 72).expect("render page");
        let any_non_white = (0..buf.height as i32)
            .flat_map(|y| (0..buf.width as i32).map(move |x| (x, y)))
            .any(|(x, y)| buf.get_pixel(x, y) != WHITE);
        assert!(any_non_white);
        let png = ImageEncoder::encode_png(&buf.to_raw_image()).expect("encode png");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(png.len() > 200);
    }

    #[test]
    fn render_page_invalid_page_returns_err() {
        let engine = ContentEngine::open_path(fixture("flate.pdf")).expect("open flate fixture");
        assert!(engine.render_page(999, 72).is_err());
    }

    #[test]
    fn fill_with_half_alpha_produces_semi_transparent_pixels() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let color = RenderColor::rgb(1.0, 0.0, 0.0)
            .with_alpha(0.5)
            .to_pixel_color();

        let mut path = Path::new();
        path.rect(20.0, 20.0, 60.0, 60.0);
        PathPainter::fill(&mut buf, &path, &ctm, &vp, color, FillRule::NonZero);

        let center = buf.get_pixel(50, 50);
        println!("half-alpha red center pixel: {:?}", center);
        assert!(center[0] > 100);
        assert!(center[1] > 50 && center[1] < 255);
        assert!(center[2] > 50 && center[2] < 255);
    }

    #[test]
    fn fill_with_opaque_alpha_produces_opaque_pixels() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let color = RenderColor::rgb(1.0, 0.0, 0.0).to_pixel_color();

        let mut path = Path::new();
        path.rect(20.0, 20.0, 60.0, 60.0);
        PathPainter::fill(&mut buf, &path, &ctm, &vp, color, FillRule::NonZero);

        let center = buf.get_pixel(50, 50);
        assert_eq!(center[0], 255);
        assert_eq!(center[1], 0);
        assert_eq!(center[2], 0);
    }

    #[test]
    fn fill_with_zero_alpha_leaves_buffer_unchanged() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let color = RenderColor::rgb(1.0, 0.0, 0.0)
            .with_alpha(0.0)
            .to_pixel_color();

        let mut path = Path::new();
        path.rect(20.0, 20.0, 60.0, 60.0);
        PathPainter::fill(&mut buf, &path, &ctm, &vp, color, FillRule::NonZero);

        assert_eq!(buf.get_pixel(50, 50), WHITE);
    }

    #[test]
    fn color_space_handler_respects_alpha_parameter() {
        let color = crate::content::state::Color {
            space: crate::content::state::ColorSpace::DeviceRGB,
            components: vec![1.0, 0.0, 0.0],
        };

        let full = ColorSpaceHandler::to_render_color(&color, 1.0);
        let half = ColorSpaceHandler::to_render_color(&color, 0.5);
        let zero = ColorSpaceHandler::to_render_color(&color, 0.0);

        assert_eq!(full.to_pixel_color()[3], 255);
        assert!((half.to_pixel_color()[3] as i32 - 128).abs() <= 1);
        assert_eq!(zero.to_pixel_color()[3], 0);
    }

    #[test]
    fn graphics_state_alpha_defaults_to_opaque() {
        let gs = GraphicsState::default();
        assert_eq!(gs.fill_alpha, 1.0);
        assert_eq!(gs.stroke_alpha, 1.0);
    }

    #[test]
    fn porter_duff_pixel_blend_matches_half_red_over_white() {
        // Half-red (rgb 128,0,0 at alpha 128/255â‰ˆ0.502) over white, composited in
        // sRGB space (matches Poppler/Splash). Each channel mixes directly:
        // R = 0.502*128 + 0.498*255 â‰ˆ 191; G = B = 0.502*0 + 0.498*255 â‰ˆ 127.
        let mut buf = PixelBuffer::new_filled(1, 1, WHITE);
        buf.blend_pixel(0, 0, [128, 0, 0, 128], 1.0);
        let pixel = buf.get_pixel(0, 0);
        println!("porter-duff half-red pixel: {:?}", pixel);
        assert!((pixel[0] as i32 - 191).abs() <= 3, "R={}", pixel[0]);
        assert!((pixel[1] as i32 - 127).abs() <= 3, "G={}", pixel[1]);
        assert!((pixel[2] as i32 - 127).abs() <= 3, "B={}", pixel[2]);
    }

    #[test]
    fn colrv1_porter_duff_pixel_modes_cover_porterduff_radial_color_glyph_set() {
        use crate::render::color_glyph::ColrBlendMode;

        let src = [200, 20, 20, 128];
        let dst = [20, 80, 200, 192];
        assert_eq!(
            composite_colr_porter_duff_pixel(src, dst, ColrBlendMode::Clear),
            [0, 0, 0, 0]
        );
        assert_eq!(
            composite_colr_porter_duff_pixel(src, dst, ColrBlendMode::Source),
            src
        );
        assert_eq!(
            composite_colr_porter_duff_pixel(src, dst, ColrBlendMode::Destination),
            dst
        );
        let plus = composite_colr_porter_duff_pixel(src, dst, ColrBlendMode::Plus);
        assert!(plus[3] >= 250, "Plus alpha should saturate: {plus:?}");
        let xor = composite_colr_porter_duff_pixel(src, dst, ColrBlendMode::Xor);
        assert!(xor[3] < plus[3], "Xor should reduce overlap alpha: {xor:?}");
    }

    #[test]
    fn colrv1_radial_solver_handles_moving_centers_exactly() {
        let t = solve_colr_radial_t(
            ColrPoint { x: 710.0, y: 500.0 },
            ColrCircle {
                center: ColrPoint { x: 400.0, y: 500.0 },
                radius: 10.0,
            },
            ColrCircle {
                center: ColrPoint { x: 700.0, y: 500.0 },
                radius: 310.0,
            },
        )
        .expect("moving-center radial root");
        assert!((t - 0.5).abs() < 1e-9, "t={t}");

        let off_axis = solve_colr_radial_t(
            ColrPoint { x: 500.0, y: 650.0 },
            ColrCircle {
                center: ColrPoint { x: 380.0, y: 430.0 },
                radius: 20.0,
            },
            ColrCircle {
                center: ColrPoint { x: 700.0, y: 620.0 },
                radius: 520.0,
            },
        )
        .expect("off-axis moving-center radial root");
        assert!(off_axis.is_finite());
        assert!(off_axis > 0.0);
    }

    #[test]
    fn alpha_composite_white_plus_half_red_is_pink() {
        // sRGB-space source-over (matches Poppler/Splash): the G/B channels land
        // at the sRGB midpoint 0.5, not the linear-light value ~0.735.
        let result = RenderColor::alpha_composite(
            RenderColor::white(),
            RenderColor::new(1.0, 0.0, 0.0, 0.5),
        );
        assert!((result.a - 1.0).abs() < 0.001);
        assert!((result.r - 1.0).abs() < 0.001);
        assert!((result.g - 0.5).abs() < 0.01, "g={}", result.g);
        assert!((result.b - 0.5).abs() < 0.01, "b={}", result.b);
    }

    #[test]
    fn blend_coverage_matches_buffer_blending_within_rounding() {
        let composited = RenderColor::alpha_composite(
            RenderColor::white(),
            RenderColor::new(1.0, 0.0, 0.0, 128.0 / 255.0),
        )
        .to_pixel_color();
        let mut buf = PixelBuffer::new_filled(1, 1, WHITE);
        buf.blend_pixel(0, 0, [255, 0, 0, 128], 1.0);
        let blended = buf.get_pixel(0, 0);

        assert!((blended[0] as i32 - composited[0] as i32).abs() <= 2);
        assert!((blended[1] as i32 - composited[1] as i32).abs() <= 2);
        assert!((blended[2] as i32 - composited[2] as i32).abs() <= 2);
    }

    #[test]
    fn transparent_stroke_over_fill_blends_border_only() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);

        let mut path = Path::new();
        path.rect(20.0, 20.0, 60.0, 60.0);
        PathPainter::fill(&mut buf, &path, &ctm, &vp, RED, FillRule::NonZero);

        let mut border = Path::new();
        border.rect(20.0, 20.0, 60.0, 60.0);
        let half_black = RenderColor::black().with_alpha(0.5).to_pixel_color();
        PathPainter::stroke(
            &mut buf,
            &border,
            &ctm,
            &vp,
            half_black,
            3.0,
            &DashState::solid(),
        );

        assert_eq!(buf.get_pixel(50, 50), RED);
        let border_pixel = buf.get_pixel(50, 20);
        assert!(border_pixel[0] < 255 || border_pixel[1] > 0 || border_pixel[2] > 0);
    }

    #[test]
    fn full_transparency_preserves_background() {
        let vp = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(50, 50, BLUE);
        let color = RenderColor::rgb(1.0, 0.0, 0.0)
            .with_alpha(0.0)
            .to_pixel_color();

        let mut path = Path::new();
        path.rect(5.0, 5.0, 40.0, 40.0);
        PathPainter::fill(&mut buf, &path, &ctm, &vp, color, FillRule::NonZero);

        for y in 0..50i32 {
            for x in 0..50i32 {
                assert_eq!(buf.get_pixel(x, y), BLUE);
            }
        }
    }

    // â”€â”€ Form XObject helper tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    fn dict_with(entries: &[(&str, PdfObject)]) -> PdfDictionary {
        PdfDictionary::new(
            entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
    }

    #[test]
    fn extract_bbox_parses_valid_array() {
        let dict = dict_with(&[(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(100.0),
                PdfObject::Real(200.0),
            ]),
        )]);
        assert_eq!(extract_bbox(&dict).unwrap(), [0.0, 0.0, 100.0, 200.0]);
    }

    #[test]
    fn extract_bbox_accepts_integer_components() {
        let dict = dict_with(&[(
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(50),
                PdfObject::Integer(50),
            ]),
        )]);
        assert_eq!(extract_bbox(&dict).unwrap(), [0.0, 0.0, 50.0, 50.0]);
    }

    #[test]
    fn extract_bbox_missing_returns_none() {
        assert!(extract_bbox(&PdfDictionary::empty()).is_none());
    }

    #[test]
    fn extract_bbox_short_array_returns_none() {
        let dict = dict_with(&[(
            "BBox",
            PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
        )]);
        assert!(extract_bbox(&dict).is_none());
    }

    #[test]
    fn form_bbox_intersection_supports_tile_culling() {
        let viewport = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let bbox = [10.0, 10.0, 30.0, 30.0];

        assert!(form_bbox_intersects_viewport(
            bbox,
            &Transform2D::identity(),
            &viewport.pixel_window(10, 20, 5, 5)
        ));
        assert!(!form_bbox_intersects_viewport(
            bbox,
            &Transform2D::identity(),
            &viewport.pixel_window(0, 0, 5, 5)
        ));
    }

    #[test]
    fn extract_form_matrix_defaults_to_identity() {
        let m = extract_form_matrix(&PdfDictionary::empty());
        assert_eq!(m, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn extract_form_matrix_parses_translation() {
        let dict = dict_with(&[(
            "Matrix",
            PdfObject::Array(vec![
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(50.0),
                PdfObject::Real(100.0),
            ]),
        )]);
        let m = extract_form_matrix(&dict);
        assert_eq!(m[4], 50.0, "e (tx) = 50");
        assert_eq!(m[5], 100.0, "f (ty) = 100");
    }

    #[test]
    fn extract_form_matrix_short_array_falls_back_to_identity() {
        let dict = dict_with(&[(
            "Matrix",
            PdfObject::Array(vec![PdfObject::Real(2.0), PdfObject::Real(0.0)]),
        )]);
        assert_eq!(extract_form_matrix(&dict), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn merge_resources_form_xobjects_override_page() {
        let mut page_res = PageResources::default();
        page_res.xobjects.insert("X1".into(), (10, 0));
        page_res.xobjects.insert("X2".into(), (11, 0));

        let mut form_res = PageResources::default();
        form_res.xobjects.insert("X1".into(), (20, 0)); // overrides X1
        form_res.xobjects.insert("X3".into(), (30, 0)); // new

        let merged = merge_resources(form_res, &page_res);
        assert_eq!(merged.xobjects["X1"], (20, 0), "Form X1 overrides page X1");
        assert_eq!(merged.xobjects["X2"], (11, 0), "page X2 inherited");
        assert_eq!(merged.xobjects["X3"], (30, 0), "Form X3 added");
    }

    #[test]
    fn merge_resources_empty_form_yields_page_resources() {
        let mut page_res = PageResources::default();
        page_res.xobjects.insert("Im1".into(), (5, 0));
        page_res.fonts.insert("F1".into(), PdfDictionary::empty());
        let merged = merge_resources(PageResources::default(), &page_res);
        assert_eq!(merged.xobjects["Im1"], (5, 0));
        assert!(merged.fonts.contains_key("F1"));
    }

    #[test]
    fn merge_resources_form_font_overrides_page_font() {
        let mut page_res = PageResources::default();
        page_res
            .fonts
            .insert("F1".into(), dict_with(&[("Tag", PdfObject::Integer(1))]));

        let mut form_res = PageResources::default();
        form_res
            .fonts
            .insert("F1".into(), dict_with(&[("Tag", PdfObject::Integer(2))]));

        let merged = merge_resources(form_res, &page_res);
        assert_eq!(
            merged.fonts["F1"].get_integer("Tag"),
            Some(2),
            "Form font F1 should override page font F1"
        );
    }
}
