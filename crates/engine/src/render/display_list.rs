//! Display-list capture and replay for PDF rendering.
//!
//! This module captures a normalized, replayable page drawing program. Vector
//! path operations carry their draw state directly, while text, images, XObjects,
//! shadings, patterns, and transparency-sensitive operations are represented as
//! native high-level replay operations through the canonical renderer state.

use crate::cancel::CancelToken;
use crate::content::operation::{ContentOperation, Operand};
use crate::content::state::{BlendMode, Color, ColorSpace, GraphicsState, LineCap, LineJoin};
use crate::engine::PageResources;
use crate::object::PdfObject;
use crate::render::buffer::{ClipMask, PixelBuffer, PixelColor, RenderMode, WHITE};
use crate::render::color::ColorSpaceHandler;
use crate::render::line::DashState;
use crate::render::path::{
    axis_aligned_integer_rect, flatten_path, flatten_path_device_transform,
    rasterize_flat_alpha_mask, rasterize_glyph_alpha_mask, stroke_flat_path, FillRule,
    GlyphHinting, Path, PathPainter, RasterizedGlyphMask,
};
use crate::render::transform::{Transform2D, Viewport};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// A replayable page-level drawing program.
#[derive(Debug, Clone)]
pub struct DisplayList {
    pub viewport: Viewport,
    pub ops: Vec<DisplayOp>,
    pub stats: DisplayListStats,
    pub supported: bool,
    pub unsupported: Vec<UnsupportedRenderOp>,
}

impl DisplayList {
    pub fn is_fully_supported(&self) -> bool {
        self.supported && self.unsupported.is_empty()
    }

    pub fn has_compatibility_runs(&self) -> bool {
        self.stats.compatibility_runs != 0
    }

    pub fn native_vector_only(&self) -> bool {
        self.is_fully_supported()
            && !self.has_compatibility_runs()
            && !self.ops.iter().any(DisplayOp::is_native_high_level)
    }

    pub fn approximate_memory_bytes(&self) -> usize {
        let path_bytes: usize = self
            .ops
            .iter()
            .map(|op| match op {
                DisplayOp::Clip { path, .. }
                | DisplayOp::FillPath { path, .. }
                | DisplayOp::StrokePath { path, .. } => {
                    std::mem::size_of_val(path.segments.as_slice())
                }
                DisplayOp::StateOp { approx_bytes, .. } => *approx_bytes,
                DisplayOp::NativeTextOp { approx_bytes, .. }
                | DisplayOp::NativeImageXObject { approx_bytes, .. }
                | DisplayOp::NativeShadingOp { approx_bytes, .. }
                | DisplayOp::NativePatternPathOp { approx_bytes, .. }
                | DisplayOp::NativeFormXObject { approx_bytes, .. }
                | DisplayOp::NativeInlineImage { approx_bytes, .. } => *approx_bytes,
                DisplayOp::Save | DisplayOp::Restore => 0,
            })
            .sum();
        std::mem::size_of::<Self>()
            + self.ops.len() * std::mem::size_of::<DisplayOp>()
            + path_bytes
            + self.unsupported.len() * std::mem::size_of::<UnsupportedRenderOp>()
    }
}

/// Normalized display-list operation.
#[derive(Debug, Clone)]
pub enum DisplayOp {
    Save,
    Restore,
    Clip {
        path: Path,
        ctm: Transform2D,
        rule: FillRule,
        bounds: Option<RenderBounds>,
    },
    FillPath {
        path: Path,
        state: DrawState,
        rule: FillRule,
        bounds: Option<RenderBounds>,
    },
    StrokePath {
        path: Path,
        state: DrawState,
        bounds: Option<RenderBounds>,
    },
    /// Replayable graphics-state mutation needed before native high-level ops.
    ///
    /// Direct vector replay ignores this because normalized path ops already
    /// carry captured draw state. RenderState replay dispatches it before native
    /// text, image, and Form XObject operations.
    StateOp {
        op: ContentOperation,
        approx_bytes: usize,
    },
    /// Native replay of one text/text-state operation through the page
    /// renderer's glyph path.
    NativeTextOp {
        op: ContentOperation,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of an Image XObject `Do` operation.
    NativeImageXObject {
        op: ContentOperation,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of a named shading `sh` operation.
    NativeShadingOp {
        op: ContentOperation,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of a path paint through the page renderer's path state.
    ///
    /// This is used when canonical `RenderState` must participate in the paint
    /// operation, for example active tiling/shading patterns or an ExtGState
    /// soft mask. Direct vector replay intentionally bypasses `RenderState`, so
    /// it must not be used for those stateful cases.
    NativePatternPathOp {
        ops: Vec<ContentOperation>,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of an inline image `ID` plus payload operation.
    NativeInlineImage {
        ops: Vec<ContentOperation>,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
    /// Native replay of a Form XObject `Do` operation.
    NativeFormXObject {
        op: ContentOperation,
        approx_bytes: usize,
        bounds: Option<RenderBounds>,
    },
}

impl DisplayOp {
    pub fn is_native_high_level(&self) -> bool {
        matches!(
            self,
            DisplayOp::NativeTextOp { .. }
                | DisplayOp::NativeImageXObject { .. }
                | DisplayOp::NativeShadingOp { .. }
                | DisplayOp::NativePatternPathOp { .. }
                | DisplayOp::NativeInlineImage { .. }
                | DisplayOp::NativeFormXObject { .. }
        )
    }
}

/// Paint and geometry state needed to replay one operation.
#[derive(Debug, Clone)]
pub struct DrawState {
    pub ctm: Transform2D,
    pub fill_color: PixelColor,
    pub stroke_color: PixelColor,
    pub fill_cmyk: Option<[f32; 4]>,
    pub stroke_cmyk: Option<[f32; 4]>,
    pub blend_mode: BlendMode,
    pub rendering_intent: String,
    pub stroke_overprint: bool,
    pub fill_overprint: bool,
    pub overprint_mode: i32,
    pub line_width: f64,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub miter_limit: f64,
    pub dash: DashState,
}

/// Display-list feature counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayListStats {
    pub operations: usize,
    pub saves: usize,
    pub restores: usize,
    pub clips: usize,
    pub fills: usize,
    pub strokes: usize,
    pub paths: usize,
    pub path_segments: usize,
    pub text_ops: usize,
    pub image_xobjects: usize,
    pub inline_images: usize,
    pub form_xobjects: usize,
    pub shadings: usize,
    pub patterns: usize,
    pub transparency_ops: usize,
    pub optional_content_ops: usize,
    pub compatibility_runs: usize,
    pub compatibility_ops: usize,
    pub compatibility_bytes: usize,
    pub compatibility_fallback_reasons: BTreeMap<String, usize>,
    pub native_text_ops: usize,
    pub native_image_xobjects: usize,
    pub native_shading_ops: usize,
    pub native_pattern_path_ops: usize,
    pub native_inline_images: usize,
    pub native_form_xobjects: usize,
    pub unsupported_ops: usize,
    pub max_stack_depth: usize,
}

/// A drawing operation the current display-list subset intentionally leaves on
/// the existing immediate renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedRenderOp {
    pub operator: String,
    pub reason: String,
}

/// Pixel-space page tile rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct RenderTile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl RenderTile {
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    pub fn estimated_rgba_bytes(self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// Full-page pixel-space bounds for display-list culling.
///
/// Bounds are computed against the display list's full-page viewport, not a
/// tile-local viewport. Tile and band replay can therefore skip vector ops whose
/// retained bounds do not intersect the current viewport window, avoiding the
/// previous "execute every vector op for every tile" cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderBounds {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl RenderBounds {
    pub fn from_bbox(
        bbox: [f64; 4],
        ctm: &Transform2D,
        viewport: &Viewport,
        padding_px: f64,
    ) -> Option<Self> {
        let points = [
            ctm.transform_point(bbox[0], bbox[1]),
            ctm.transform_point(bbox[2], bbox[1]),
            ctm.transform_point(bbox[0], bbox[3]),
            ctm.transform_point(bbox[2], bbox[3]),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (x, y) in points {
            let (px, py) = viewport.page_to_pixel_f64(x, y);
            if !px.is_finite() || !py.is_finite() {
                continue;
            }
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }
        if !min_x.is_finite() || !max_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let pad = padding_px.max(0.0);
        Some(Self {
            x0: floor_i32(min_x - pad),
            y0: floor_i32(min_y - pad),
            x1: ceil_i32(max_x + pad),
            y1: ceil_i32(max_y + pad),
        })
    }

    pub fn from_unit_square(
        ctm: &Transform2D,
        viewport: &Viewport,
        padding_px: f64,
    ) -> Option<Self> {
        let points = [
            ctm.transform_point(0.0, 0.0),
            ctm.transform_point(1.0, 0.0),
            ctm.transform_point(1.0, 1.0),
            ctm.transform_point(0.0, 1.0),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (x, y) in points {
            let (px, py) = viewport.page_to_pixel_f64(x, y);
            if !px.is_finite() || !py.is_finite() {
                continue;
            }
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }
        if !min_x.is_finite() || !max_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let pad = padding_px.max(0.0);
        Some(Self {
            x0: floor_i32(min_x - pad),
            y0: floor_i32(min_y - pad),
            x1: ceil_i32(max_x + pad),
            y1: ceil_i32(max_y + pad),
        })
    }

    pub fn from_path(
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        padding_px: f64,
    ) -> Option<Self> {
        let flat = flatten_path(path, ctm, viewport, 0.5);
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for subpath in &flat.subpaths {
            for &(x, y) in subpath {
                if !x.is_finite() || !y.is_finite() {
                    continue;
                }
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        if !min_x.is_finite() || !max_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let pad = padding_px.max(0.0);
        Some(Self {
            x0: floor_i32(min_x - pad),
            y0: floor_i32(min_y - pad),
            x1: ceil_i32(max_x + pad),
            y1: ceil_i32(max_y + pad),
        })
    }

    pub fn intersects_viewport(&self, viewport: &Viewport) -> bool {
        let vx0 = i32_from_u32(viewport.origin_x_px);
        let vy0 = i32_from_u32(viewport.origin_y_px);
        let vx1 = i32_from_u32(viewport.origin_x_px.saturating_add(viewport.width_px));
        let vy1 = i32_from_u32(viewport.origin_y_px.saturating_add(viewport.height_px));
        self.x1 > vx0 && self.x0 < vx1 && self.y1 > vy0 && self.y0 < vy1
    }

    pub fn intersect(self, other: Self) -> Option<Self> {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        let x1 = self.x1.min(other.x1);
        let y1 = self.y1.min(other.y1);
        (x1 > x0 && y1 > y0).then_some(Self { x0, y0, x1, y1 })
    }

    pub fn from_text_run(
        start_tm: [f64; 6],
        end_tm: [f64; 6],
        ctm: &Transform2D,
        viewport: &Viewport,
        font_size: f64,
        rise: f64,
        padding_px: f64,
    ) -> Option<Self> {
        if font_size <= 0.0 || !font_size.is_finite() {
            return None;
        }
        let descent = -0.30 * font_size;
        let ascent = 1.20 * font_size;
        let text_points = [
            text_matrix_point(start_tm, 0.0, rise + descent),
            text_matrix_point(start_tm, 0.0, rise + ascent),
            text_matrix_point(end_tm, 0.0, rise + descent),
            text_matrix_point(end_tm, 0.0, rise + ascent),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for (x, y) in text_points {
            let (ux, uy) = ctm.transform_point(x, y);
            let (px, py) = viewport.page_to_pixel_f64(ux, uy);
            if !px.is_finite() || !py.is_finite() {
                continue;
            }
            min_x = min_x.min(px);
            min_y = min_y.min(py);
            max_x = max_x.max(px);
            max_y = max_y.max(py);
        }
        if !min_x.is_finite() || !max_x.is_finite() || max_x <= min_x || max_y <= min_y {
            return None;
        }
        let pad = padding_px.max(0.0);
        Some(Self {
            x0: floor_i32(min_x - pad),
            y0: floor_i32(min_y - pad),
            x1: ceil_i32(max_x + pad),
            y1: ceil_i32(max_y + pad),
        })
    }
}

fn text_matrix_point(tm: [f64; 6], x: f64, y: f64) -> (f64, f64) {
    (
        tm[0].mul_add(x, tm[2].mul_add(y, tm[4])),
        tm[1].mul_add(x, tm[3].mul_add(y, tm[5])),
    )
}

fn floor_i32(value: f64) -> i32 {
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

fn ceil_i32(value: f64) -> i32 {
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

fn i32_from_u32(value: u32) -> i32 {
    if value > i32::MAX as u32 {
        i32::MAX
    } else {
        value as i32
    }
}

fn merge_bounds(a: Option<RenderBounds>, b: Option<RenderBounds>) -> Option<RenderBounds> {
    match (a, b) {
        (Some(a), Some(b)) => Some(RenderBounds {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        }),
        (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
        (None, None) => None,
    }
}

/// Stable key for the bounded render cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderCacheKey {
    pub page_number: usize,
    pub dpi: u32,
    pub render_mode: &'static str,
    pub tile: RenderTile,
    pub visibility_fingerprint: String,
    pub prepress_fingerprint: String,
    pub document_revision: String,
    pub contract_fingerprint: String,
}

impl RenderCacheKey {
    pub fn new(page_number: usize, dpi: u32, render_mode: RenderMode, tile: RenderTile) -> Self {
        Self::new_with_visibility(page_number, dpi, render_mode, tile, "ocg:none")
    }

    pub fn new_with_visibility(
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile: RenderTile,
        visibility_fingerprint: impl Into<String>,
    ) -> Self {
        Self::new_with_visibility_and_prepress(
            page_number,
            dpi,
            render_mode,
            tile,
            visibility_fingerprint,
            "prepress:none",
        )
    }

    pub fn new_with_visibility_and_prepress(
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile: RenderTile,
        visibility_fingerprint: impl Into<String>,
        prepress_fingerprint: impl Into<String>,
    ) -> Self {
        Self::new_with_full_identity(
            page_number,
            dpi,
            render_mode,
            tile,
            visibility_fingerprint,
            prepress_fingerprint,
            "revision:legacy",
            "contract:legacy",
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_full_identity(
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile: RenderTile,
        visibility_fingerprint: impl Into<String>,
        prepress_fingerprint: impl Into<String>,
        document_revision: impl Into<String>,
        contract_fingerprint: impl Into<String>,
    ) -> Self {
        Self {
            page_number,
            dpi,
            render_mode: render_mode.as_str(),
            tile,
            visibility_fingerprint: visibility_fingerprint.into(),
            prepress_fingerprint: prepress_fingerprint.into(),
            document_revision: document_revision.into(),
            contract_fingerprint: contract_fingerprint.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderCacheMetrics {
    pub hits: usize,
    pub misses: usize,
    pub inserts: usize,
    pub evictions: usize,
    pub skipped_oversized: usize,
    pub bytes: usize,
}

#[derive(Debug, Clone)]
struct RenderCacheEntry {
    buffer: PixelBuffer,
    bytes: usize,
    last_used: u64,
}

/// Per-document render tile cache with byte accounting.
#[derive(Debug, Clone)]
pub struct RenderCache {
    budget_bytes: usize,
    max_entry_bytes: usize,
    bytes: usize,
    clock: u64,
    entries: HashMap<RenderCacheKey, RenderCacheEntry>,
    metrics: RenderCacheMetrics,
}

impl RenderCache {
    pub fn new(budget_bytes: usize, max_entry_bytes: usize) -> Self {
        Self {
            budget_bytes,
            max_entry_bytes,
            bytes: 0,
            clock: 0,
            entries: HashMap::new(),
            metrics: RenderCacheMetrics::default(),
        }
    }

    pub fn disabled() -> Self {
        Self::new(0, 0)
    }

    pub fn metrics(&self) -> RenderCacheMetrics {
        let mut metrics = self.metrics.clone();
        metrics.bytes = self.bytes;
        metrics
    }

    pub fn get(&mut self, key: &RenderCacheKey) -> Option<PixelBuffer> {
        self.clock = self.clock.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_used = self.clock;
            self.metrics.hits += 1;
            Some(entry.buffer.clone())
        } else {
            self.metrics.misses += 1;
            None
        }
    }

    pub fn get_ref(&mut self, key: &RenderCacheKey) -> Option<&PixelBuffer> {
        self.clock = self.clock.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_used = self.clock;
            self.metrics.hits += 1;
            Some(&entry.buffer)
        } else {
            self.metrics.misses += 1;
            None
        }
    }

    pub fn insert(&mut self, key: RenderCacheKey, buffer: PixelBuffer) {
        if self.budget_bytes == 0 || self.max_entry_bytes == 0 {
            self.metrics.skipped_oversized += 1;
            return;
        }
        let bytes = buffer.width as usize * buffer.height as usize * 4;
        if bytes > self.max_entry_bytes || bytes > self.budget_bytes {
            self.metrics.skipped_oversized += 1;
            return;
        }
        self.clock = self.clock.saturating_add(1);
        if let Some(old) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
        while self.bytes + bytes > self.budget_bytes {
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(removed) = self.entries.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(removed.bytes);
                self.metrics.evictions += 1;
            }
        }
        self.entries.insert(
            key,
            RenderCacheEntry {
                buffer,
                bytes,
                last_used: self.clock,
            },
        );
        self.bytes += bytes;
        self.metrics.inserts += 1;
    }
}

/// Concrete rendering target for display-list replay.
pub trait RenderDevice {
    fn save(&mut self);
    fn restore(&mut self);
    fn clip_path(&mut self, path: &Path, ctm: &Transform2D, rule: FillRule);
    fn fill_path(&mut self, path: &Path, state: &DrawState, rule: FillRule);
    fn stroke_path(&mut self, path: &Path, state: &DrawState);
    fn state_op(&mut self, op: &ContentOperation) {
        log::trace!(
            "DisplayList device ignored state op '{}' because vector ops carry captured state",
            op.operator
        );
    }
    fn native_text_op(&mut self, op: &ContentOperation) {
        log::warn!(
            "DisplayList device cannot replay native text op '{}' without page context",
            op.operator
        );
    }
    fn native_image_xobject(&mut self, op: &ContentOperation) {
        log::warn!(
            "DisplayList device cannot replay native image op '{}' without page context",
            op.operator
        );
    }
    fn native_shading_op(&mut self, op: &ContentOperation) {
        log::warn!(
            "DisplayList device cannot replay native shading op '{}' without page context",
            op.operator
        );
    }
    fn native_pattern_path_op(&mut self, ops: &[ContentOperation]) {
        log::warn!(
            "DisplayList device cannot replay native pattern path ({} ops) without page context",
            ops.len()
        );
    }
    fn native_inline_image(&mut self, ops: &[ContentOperation]) {
        log::warn!(
            "DisplayList device cannot replay native inline image ({} ops) without page context",
            ops.len()
        );
    }
    fn native_form_xobject(&mut self, op: &ContentOperation) {
        log::warn!(
            "DisplayList device cannot replay native Form XObject op '{}' without page context",
            op.operator
        );
    }
}

/// CPU raster device backed by the existing [`PixelBuffer`] rasterizer.
pub struct CpuRenderDevice {
    buf: PixelBuffer,
    viewport: Viewport,
    clip_stack: Vec<Option<ClipMask>>,
    path_fill_mask_cache: CpuPathFillMaskCache,
    path_stroke_mask_cache: CpuPathStrokeMaskCache,
}

impl CpuRenderDevice {
    pub fn new(viewport: Viewport, render_mode: RenderMode) -> Self {
        Self {
            buf: PixelBuffer::new_filled_with_mode(
                viewport.width_px,
                viewport.height_px,
                WHITE,
                render_mode,
            ),
            viewport,
            clip_stack: Vec::new(),
            path_fill_mask_cache: CpuPathFillMaskCache::default(),
            path_stroke_mask_cache: CpuPathStrokeMaskCache::default(),
        }
    }

    pub fn into_buffer(self) -> PixelBuffer {
        self.buf
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct CpuPathFillMaskCacheKey {
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
struct CpuPathFillMaskCache {
    entries: HashMap<CpuPathFillMaskCacheKey, Arc<RasterizedGlyphMask>>,
    bytes: usize,
}

impl CpuPathFillMaskCache {
    const MAX_ENTRIES: usize = 4096;
    const MAX_BYTES: usize = 64 * 1024 * 1024;

    fn get(&self, key: &CpuPathFillMaskCacheKey) -> Option<Arc<RasterizedGlyphMask>> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: CpuPathFillMaskCacheKey, mask: Arc<RasterizedGlyphMask>) {
        let bytes = mask.approximate_bytes();
        if bytes > Self::MAX_BYTES / 4 {
            return;
        }
        if self.entries.len() >= Self::MAX_ENTRIES
            || self.bytes.saturating_add(bytes) > Self::MAX_BYTES
        {
            self.entries.clear();
            self.bytes = 0;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(key, mask);
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct CpuPathStrokeMaskCacheKey {
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
struct CpuPathStrokeMaskCache {
    entries: HashMap<CpuPathStrokeMaskCacheKey, Arc<RasterizedGlyphMask>>,
    bytes: usize,
}

impl CpuPathStrokeMaskCache {
    const MAX_ENTRIES: usize = 4096;
    const MAX_BYTES: usize = 64 * 1024 * 1024;

    fn get(&self, key: &CpuPathStrokeMaskCacheKey) -> Option<Arc<RasterizedGlyphMask>> {
        self.entries.get(key).cloned()
    }

    fn insert(&mut self, key: CpuPathStrokeMaskCacheKey, mask: Arc<RasterizedGlyphMask>) {
        let bytes = mask.approximate_bytes();
        if bytes > Self::MAX_BYTES / 4 {
            return;
        }
        if self.entries.len() >= Self::MAX_ENTRIES
            || self.bytes.saturating_add(bytes) > Self::MAX_BYTES
        {
            self.entries.clear();
            self.bytes = 0;
        }
        self.bytes = self.bytes.saturating_add(bytes);
        self.entries.insert(key, mask);
    }
}

fn cpu_paint_cached_path_fill(
    cache: &mut CpuPathFillMaskCache,
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
    if let Some((x, y, width, height)) = axis_aligned_integer_rect(path, ctm, viewport) {
        buf.fill_rect(x, y, width, height, color);
        return true;
    }
    let device_t = ctm.concat(&viewport.to_transform());
    if !cpu_path_cache_transform_allowed(&device_t) {
        return false;
    }
    let origin_x = device_t.e.floor();
    let origin_y = device_t.f.floor();
    let normalized_t = Transform2D {
        e: device_t.e - origin_x,
        f: device_t.f - origin_y,
        ..device_t
    };
    let key = CpuPathFillMaskCacheKey {
        path_hash: cpu_hash_path_for_mask_cache(path),
        fill_rule: match rule {
            FillRule::NonZero => 0,
            FillRule::EvenOdd => 1,
        },
        a: cpu_quantize_mask_value(normalized_t.a),
        b: cpu_quantize_mask_value(normalized_t.b),
        c: cpu_quantize_mask_value(normalized_t.c),
        d: cpu_quantize_mask_value(normalized_t.d),
        frac_e: cpu_quantize_mask_fraction(normalized_t.e),
        frac_f: cpu_quantize_mask_fraction(normalized_t.f),
    };
    let dx = cpu_floor_to_i32(origin_x);
    let dy = cpu_floor_to_i32(origin_y);
    if let Some(mask) = cache.get(&key) {
        mask.paint(buf, dx, dy, color);
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
fn cpu_paint_cached_path_stroke(
    cache: &mut CpuPathStrokeMaskCache,
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
    if !cpu_path_cache_transform_allowed(&device_t) {
        return false;
    }
    let origin_x = device_t.e.floor();
    let origin_y = device_t.f.floor();
    let normalized_t = Transform2D {
        e: device_t.e - origin_x,
        f: device_t.f - origin_y,
        ..device_t
    };
    let key = CpuPathStrokeMaskCacheKey {
        path_hash: cpu_hash_path_for_mask_cache(path),
        width: cpu_quantize_mask_value(stroke_width * device_t.scale_factor()),
        cap: cpu_line_cap_id(cap),
        join: cpu_line_join_id(join),
        miter_limit: cpu_quantize_mask_value(miter_limit),
        a: cpu_quantize_mask_value(normalized_t.a),
        b: cpu_quantize_mask_value(normalized_t.b),
        c: cpu_quantize_mask_value(normalized_t.c),
        d: cpu_quantize_mask_value(normalized_t.d),
        frac_e: cpu_quantize_mask_fraction(normalized_t.e),
        frac_f: cpu_quantize_mask_fraction(normalized_t.f),
    };
    let dx = cpu_floor_to_i32(origin_x);
    let dy = cpu_floor_to_i32(origin_y);
    if let Some(mask) = cache.get(&key) {
        mask.paint(buf, dx, dy, color);
        return true;
    }
    let flat = flatten_path_device_transform(path, &normalized_t, 0.5);
    let outline = stroke_flat_path(
        &flat,
        (stroke_width * normalized_t.scale_factor()).max(1.0),
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

fn cpu_path_cache_transform_allowed(device_t: &Transform2D) -> bool {
    [
        device_t.a, device_t.b, device_t.c, device_t.d, device_t.e, device_t.f,
    ]
    .iter()
    .all(|value| value.is_finite())
        && device_t.scale_factor() > 0.0
        && device_t.scale_factor() <= 256.0
}

fn cpu_floor_to_i32(value: f64) -> i32 {
    if value <= i32::MIN as f64 {
        i32::MIN
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn cpu_line_cap_id(cap: &LineCap) -> u8 {
    match cap {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::ProjectingSquare => 2,
    }
}

fn cpu_line_join_id(join: &LineJoin) -> u8 {
    match join {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
    }
}

fn cpu_hash_path_for_mask_cache(path: &Path) -> u64 {
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

fn cpu_quantize_mask_value(value: f64) -> i64 {
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

fn cpu_quantize_mask_fraction(value: f64) -> i64 {
    const SCALE: f64 = 2.0;
    if !value.is_finite() {
        0
    } else {
        (value.fract() * SCALE).round() as i64
    }
}

impl RenderDevice for CpuRenderDevice {
    fn save(&mut self) {
        self.clip_stack.push(self.buf.clip_mask().cloned());
    }

    fn restore(&mut self) {
        if let Some(saved) = self.clip_stack.pop() {
            self.buf.restore_clip(saved);
        } else {
            log::warn!("DisplayList CpuRenderDevice: restore with empty clip stack");
        }
    }

    fn clip_path(&mut self, path: &Path, ctm: &Transform2D, rule: FillRule) {
        if let Some((x, y, width, height)) = axis_aligned_integer_rect(path, ctm, &self.viewport) {
            self.buf.set_clip(ClipMask::from_visible_rect(
                self.buf.width,
                self.buf.height,
                x,
                y,
                width,
                height,
            ));
            return;
        }
        let flat = flatten_path(path, ctm, &self.viewport, 0.5);
        let clip = ClipMask::from_path(&flat, self.buf.width, self.buf.height, rule);
        self.buf.set_clip(clip);
    }

    fn fill_path(&mut self, path: &Path, state: &DrawState, rule: FillRule) {
        let saved_blend = self.buf.blend_mode;
        self.buf.blend_mode = state.blend_mode;
        if state.fill_overprint {
            if let Some(cmyk) = state.fill_cmyk {
                PathPainter::fill_device_cmyk_overprint_preview(
                    &mut self.buf,
                    path,
                    &state.ctm,
                    &self.viewport,
                    cmyk,
                    state.fill_color[3] as f32 / 255.0,
                    state.overprint_mode,
                    rule,
                );
                self.buf.blend_mode = saved_blend;
                return;
            }
        }
        if !cpu_paint_cached_path_fill(
            &mut self.path_fill_mask_cache,
            &mut self.buf,
            &self.viewport,
            path,
            &state.ctm,
            rule,
            state.fill_color,
        ) {
            // General-path fallback is retained only for transforms/path
            // shapes outside the bounded replay-mask cache contract. Route it
            // through the same scanline-capable fast path used by the canonical
            // page renderer so CpuRenderDevice does not regress to the old
            // accumulator-heavy paint for large retained-list paths.
            let cancel = CancelToken::new();
            let _ = PathPainter::fill_fast_cancellable(
                &mut self.buf,
                path,
                &state.ctm,
                &self.viewport,
                state.fill_color,
                rule,
                &cancel,
            );
        }
        self.buf.blend_mode = saved_blend;
    }

    fn stroke_path(&mut self, path: &Path, state: &DrawState) {
        let saved_blend = self.buf.blend_mode;
        self.buf.blend_mode = state.blend_mode;
        if !cpu_paint_cached_path_stroke(
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
            // General-path fallback is retained only for dash/transform/path
            // shapes outside the bounded replay-mask cache contract. Use the
            // bounded scanline-capable fast path instead of the legacy
            // accumulator-heavy stroke replay.
            let cancel = CancelToken::new();
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
                &cancel,
            );
        }
        self.buf.blend_mode = saved_blend;
    }
}

pub fn replay_display_list(list: &DisplayList, device: &mut dyn RenderDevice) {
    for op in &list.ops {
        match op {
            DisplayOp::Save => device.save(),
            DisplayOp::Restore => device.restore(),
            DisplayOp::Clip {
                path, ctm, rule, ..
            } => device.clip_path(path, ctm, *rule),
            DisplayOp::FillPath {
                path, state, rule, ..
            } => device.fill_path(path, state, *rule),
            DisplayOp::StrokePath { path, state, .. } => device.stroke_path(path, state),
            DisplayOp::StateOp { op, .. } => device.state_op(op),
            DisplayOp::NativeTextOp { op, .. } => device.native_text_op(op),
            DisplayOp::NativeImageXObject { op, .. } => device.native_image_xobject(op),
            DisplayOp::NativeShadingOp { op, .. } => device.native_shading_op(op),
            DisplayOp::NativePatternPathOp { ops, .. } => device.native_pattern_path_op(ops),
            DisplayOp::NativeInlineImage { ops, .. } => device.native_inline_image(ops),
            DisplayOp::NativeFormXObject { op, .. } => device.native_form_xobject(op),
        }
    }
}

pub fn render_display_list(list: &DisplayList, render_mode: RenderMode) -> PixelBuffer {
    let mut device = CpuRenderDevice::new(list.viewport.clone(), render_mode);
    replay_display_list(list, &mut device);
    device.into_buffer()
}

/// Capture a vector-compatible display list from decoded content operations.
pub fn build_display_list(
    ops: &[ContentOperation],
    viewport: Viewport,
    resources: &PageResources,
) -> DisplayList {
    let stats = classify_content(ops, resources);
    let mut builder = DisplayListBuilder::new(viewport, resources);
    builder.stats = stats;
    builder.dispatch_all(ops);
    builder.finish()
}

fn estimate_ops_bytes(ops: &[ContentOperation]) -> usize {
    ops.iter()
        .map(|op| {
            op.operator.len()
                + op.operands
                    .iter()
                    .map(estimate_operand_bytes)
                    .sum::<usize>()
                + std::mem::size_of::<ContentOperation>()
        })
        .sum()
}

fn estimate_operand_bytes(operand: &crate::content::operation::Operand) -> usize {
    use crate::content::operation::Operand;
    match operand {
        Operand::Integer(_) | Operand::Real(_) | Operand::Boolean(_) => {
            std::mem::size_of_val(operand)
        }
        Operand::Name(name) => name.len(),
        Operand::String(bytes) => bytes.len(),
        Operand::Array(items) => items.iter().map(estimate_operand_bytes).sum(),
        Operand::Dictionary(entries) => entries
            .iter()
            .map(|(key, value)| key.len().saturating_add(estimate_operand_bytes(value)))
            .sum(),
    }
}

fn classify_content(ops: &[ContentOperation], resources: &PageResources) -> DisplayListStats {
    let mut stats = DisplayListStats::default();
    let mut gs = GraphicsState::default();
    let mut pending_inline = false;
    for op in ops {
        match op.operator.as_str() {
            "Tj" | "TJ" | "'" | "\"" => stats.text_ops += 1,
            "BT" | "ET" | "Tf" | "Td" | "TD" | "Tm" | "T*" | "Tc" | "Tw" | "Tz" | "TL" | "Tr"
            | "Ts" => {}
            "Do" => match op
                .name(0)
                .and_then(|name| resources.xobject_subtypes.get(name))
                .map(String::as_str)
            {
                Some("Image") => stats.image_xobjects += 1,
                Some("Form") => stats.form_xobjects += 1,
                _ => stats.image_xobjects += 1,
            },
            "sh" => stats.shadings += 1,
            "BDC" | "DP" if marked_content_uses_optional_content(op, resources) => {
                stats.optional_content_ops += 1
            }
            "ID" => pending_inline = true,
            "inline_image_data" if pending_inline => {
                stats.inline_images += 1;
                pending_inline = false;
            }
            "gs" => {
                if let Some(name) = op.name(0) {
                    if let Some(dict) = resources.ext_g_states.get(name) {
                        if dict.get("SMask").is_some()
                            || dict.get("ca").is_some()
                            || dict.get("CA").is_some()
                            || dict.get("BM").is_some()
                        {
                            stats.transparency_ops += 1;
                        }
                    } else {
                        stats.transparency_ops += 1;
                    }
                }
            }
            "scn" | "SCN"
                if op
                    .operands
                    .iter()
                    .any(|operand| operand.as_name().is_some()) =>
            {
                stats.patterns += 1;
            }
            _ => {}
        }
        gs.process(op);
        if gs.fill_pattern_name.is_some() || gs.stroke_pattern_name.is_some() {
            stats.patterns += 1;
        }
    }
    stats
}

fn marked_content_uses_optional_content(op: &ContentOperation, resources: &PageResources) -> bool {
    let Some(property) = op.operand(1) else {
        return false;
    };
    match property {
        Operand::Name(name) => resources
            .properties
            .get(name)
            .is_some_and(pdf_object_is_optional_content_property),
        Operand::Dictionary(entries) => operand_dictionary_is_optional_content_property(entries),
        _ => false,
    }
}

fn pdf_object_is_optional_content_property(object: &crate::object::PdfObject) -> bool {
    use crate::object::PdfObject;
    match object {
        PdfObject::Dictionary(dict) => {
            matches!(dict.get_name("Type"), Some("OCG" | "OCMD")) || dict.get("OC").is_some()
        }
        PdfObject::Reference { .. } => true,
        _ => false,
    }
}

fn operand_dictionary_is_optional_content_property(entries: &[(String, Operand)]) -> bool {
    entries.iter().any(|(key, value)| {
        key == "OC"
            || (key == "Type"
                && matches!(
                    value,
                    Operand::Name(name) if name == "OCG" || name == "OCMD"
                ))
    })
}

struct DisplayListBuilder<'a> {
    viewport: Viewport,
    resources: &'a PageResources,
    gs: GraphicsState,
    path: Path,
    path_ops: Vec<ContentOperation>,
    pending_clip: Option<FillRule>,
    current_clip_bounds: Option<RenderBounds>,
    clip_bounds_stack: Vec<Option<RenderBounds>>,
    soft_mask_stack: Vec<bool>,
    active_soft_mask: bool,
    ops: Vec<DisplayOp>,
    unsupported: Vec<UnsupportedRenderOp>,
    stats: DisplayListStats,
    pending_inline: Option<ContentOperation>,
}

impl<'a> DisplayListBuilder<'a> {
    fn new(viewport: Viewport, resources: &'a PageResources) -> Self {
        Self {
            viewport,
            resources,
            gs: GraphicsState::default(),
            path: Path::new(),
            path_ops: Vec::new(),
            pending_clip: None,
            current_clip_bounds: None,
            clip_bounds_stack: Vec::new(),
            soft_mask_stack: Vec::new(),
            active_soft_mask: false,
            ops: Vec::new(),
            unsupported: Vec::new(),
            stats: DisplayListStats::default(),
            pending_inline: None,
        }
    }

    fn finish(mut self) -> DisplayList {
        self.stats.operations = self.ops.len();
        self.stats.unsupported_ops = self.unsupported.len();
        let supported = self.unsupported.is_empty();
        DisplayList {
            viewport: self.viewport,
            ops: self.ops,
            stats: self.stats,
            supported,
            unsupported: self.unsupported,
        }
    }

    fn dispatch_all(&mut self, ops: &[ContentOperation]) {
        for op in ops {
            self.dispatch(op);
        }
    }

    fn dispatch(&mut self, op: &ContentOperation) {
        match op.operator.as_str() {
            "m" => {
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    self.path.move_to(x, y);
                    self.path_ops.push(op.clone());
                }
            }
            "l" => {
                if let (Some(x), Some(y)) = (op.number(0), op.number(1)) {
                    self.path.line_to(x, y);
                    self.path_ops.push(op.clone());
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
                    self.path_ops.push(op.clone());
                }
            }
            "v" => {
                if let (Some(x2), Some(y2), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    let (cx, cy) = self.path.current_point.unwrap_or((0.0, 0.0));
                    self.path.curve_to(cx, cy, x2, y2, x3, y3);
                    self.path_ops.push(op.clone());
                }
            }
            "y" => {
                if let (Some(x1), Some(y1), Some(x3), Some(y3)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.path.curve_to(x1, y1, x3, y3, x3, y3);
                    self.path_ops.push(op.clone());
                }
            }
            "h" => {
                self.path.close();
                self.path_ops.push(op.clone());
            }
            "re" => {
                if let (Some(x), Some(y), Some(w), Some(h)) =
                    (op.number(0), op.number(1), op.number(2), op.number(3))
                {
                    self.path.rect(x, y, w, h);
                    self.path_ops.push(op.clone());
                }
            }
            "S" => self.stroke_and_clear(op),
            "s" => {
                self.path.close();
                self.path_ops.push(ContentOperation::new("h", Vec::new()));
                self.stroke_and_clear(op);
            }
            "f" | "F" => self.fill_and_clear(op, FillRule::NonZero),
            "f*" => self.fill_and_clear(op, FillRule::EvenOdd),
            "B" => self.fill_stroke_and_clear(op, FillRule::NonZero),
            "B*" => self.fill_stroke_and_clear(op, FillRule::EvenOdd),
            "b" => {
                self.path.close();
                self.path_ops.push(ContentOperation::new("h", Vec::new()));
                self.fill_stroke_and_clear(op, FillRule::NonZero);
            }
            "b*" => {
                self.path.close();
                self.path_ops.push(ContentOperation::new("h", Vec::new()));
                self.fill_stroke_and_clear(op, FillRule::EvenOdd);
            }
            "n" => {
                self.apply_pending_clip();
                self.path.clear();
                self.path_ops.clear();
            }
            "W" => self.pending_clip = Some(FillRule::NonZero),
            "W*" => self.pending_clip = Some(FillRule::EvenOdd),
            "q" => {
                self.ops.push(DisplayOp::Save);
                self.stats.saves += 1;
                self.clip_bounds_stack.push(self.current_clip_bounds);
                self.soft_mask_stack.push(self.active_soft_mask);
                self.gs.process(op);
                self.stats.max_stack_depth = self.stats.max_stack_depth.max(self.gs.stack_depth());
            }
            "Q" => {
                self.gs.process(op);
                self.current_clip_bounds = self.clip_bounds_stack.pop().unwrap_or_else(|| {
                    log::warn!("DisplayListBuilder: restore with empty clip-bounds stack");
                    None
                });
                self.active_soft_mask = self.soft_mask_stack.pop().unwrap_or_else(|| {
                    log::warn!("DisplayListBuilder: restore with empty soft-mask stack");
                    false
                });
                self.ops.push(DisplayOp::Restore);
                self.stats.restores += 1;
            }
            "cm" | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "G" | "g" | "RG" | "rg" | "K"
            | "k" | "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" => {
                self.gs.process(op);
                self.push_state_op(op);
            }
            "gs" => {
                self.apply_ext_g_state(op);
                self.push_state_op(op);
            }
            "BMC" | "BDC" | "EMC" | "MP" | "DP" | "BX" | "EX" => {
                self.push_state_op(op);
            }
            "BT" | "ET" | "Tf" | "Td" | "TD" | "Tm" | "T*" | "Tc" | "Tw" | "Tz" | "TL" | "Tr"
            | "Ts" | "Tj" | "TJ" | "'" | "\"" => {
                self.push_native_text(op);
                self.gs.process(op);
            }
            "Do" => self.push_native_xobject(op),
            "sh" => self.push_native_shading(op),
            "BI" | "EI" => {}
            "ID" => {
                self.pending_inline = Some(op.clone());
            }
            "inline_image_data" => self.push_native_inline_image(op),
            _ => {
                self.gs.process(op);
                // Unknown or extension operators are replayed through the same
                // state-dispatch path used by immediate rendering. Unsupported
                // operators remain exact no-ops or graphics-state updates
                // according to the canonical dispatcher.
                self.push_state_op(op);
            }
        }
    }

    fn apply_ext_g_state(&mut self, op: &ContentOperation) {
        let Some(name) = op.name(0) else {
            return;
        };
        let Some(dict) = self.resources.ext_g_states.get(name) else {
            return;
        };
        // Soft masks are now represented by retaining the ExtGState operator
        // itself. Display-list replay goes through the same `RenderState`
        // dispatch path as immediate rendering, so `/SMask` Form groups,
        // transfer functions, backdrop colors, and the active clip/CTM stack are
        // applied by the canonical soft-mask implementation instead of forcing
        // the whole page back to immediate rendering.
        if let Some(smask) = dict.get("SMask") {
            self.active_soft_mask = !matches!(smask, PdfObject::Name(name) if name == "None");
        }
        self.gs.apply_ext_g_state(dict);
        if self.gs.blend_mode != BlendMode::Normal
            || self.gs.fill_alpha < 0.999
            || self.gs.stroke_alpha < 0.999
        {
            // These are represented and replayable through PixelBuffer blend
            // state, but the diagnostic counter still records the page as a
            // transparency-bearing display list for inspector users.
        }
    }

    fn apply_pending_clip(&mut self) {
        if let Some(rule) = self.pending_clip.take() {
            let path = self.path.clone();
            let clip_bounds = RenderBounds::from_path(&path, &self.ctm(), &self.viewport, 0.0);
            self.current_clip_bounds = match (self.current_clip_bounds, clip_bounds) {
                (None, Some(bounds)) => Some(bounds),
                (Some(existing), Some(bounds)) => {
                    existing.intersect(bounds).or(Some(RenderBounds {
                        x0: 0,
                        y0: 0,
                        x1: 0,
                        y1: 0,
                    }))
                }
                (_, None) => Some(RenderBounds {
                    x0: 0,
                    y0: 0,
                    x1: 0,
                    y1: 0,
                }),
            };
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.clips += 1;
            self.ops.push(DisplayOp::Clip {
                path,
                ctm: self.ctm(),
                rule,
                bounds: clip_bounds,
            });
        }
    }

    fn stroke_and_clear(&mut self, paint_op: &ContentOperation) {
        self.apply_pending_clip();
        if self.active_soft_mask || self.uses_pattern_or_named_space() {
            self.push_stateful_path_run(paint_op);
            self.path.clear();
            self.path_ops.clear();
            return;
        }
        if !self.path.is_empty() {
            let path = self.path.clone();
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.strokes += 1;
            let state = self.draw_state();
            let bounds = self.path_bounds_for_stroke(&state);
            self.ops.push(DisplayOp::StrokePath {
                path,
                state,
                bounds,
            });
        }
        self.path.clear();
        self.path_ops.clear();
    }

    fn fill_and_clear(&mut self, paint_op: &ContentOperation, rule: FillRule) {
        self.apply_pending_clip();
        if self.active_soft_mask || self.uses_pattern_or_named_space() {
            self.push_stateful_path_run(paint_op);
            self.path.clear();
            self.path_ops.clear();
            return;
        }
        if !self.path.is_empty() {
            let path = self.path.clone();
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.fills += 1;
            self.ops.push(DisplayOp::FillPath {
                path,
                state: self.draw_state(),
                rule,
                bounds: self.path_bounds_for_fill(),
            });
        }
        self.path.clear();
        self.path_ops.clear();
    }

    fn fill_stroke_and_clear(&mut self, paint_op: &ContentOperation, rule: FillRule) {
        self.apply_pending_clip();
        if self.active_soft_mask || self.uses_pattern_or_named_space() {
            self.push_stateful_path_run(paint_op);
            self.path.clear();
            self.path_ops.clear();
            return;
        }
        if !self.path.is_empty() {
            let path = self.path.clone();
            let state = self.draw_state();
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.fills += 1;
            self.ops.push(DisplayOp::FillPath {
                path: path.clone(),
                state: state.clone(),
                rule,
                bounds: self.path_bounds_for_fill(),
            });
            self.stats.strokes += 1;
            let bounds = self.path_bounds_for_stroke(&state);
            self.ops.push(DisplayOp::StrokePath {
                path,
                state,
                bounds,
            });
        }
        self.path.clear();
        self.path_ops.clear();
    }

    fn push_stateful_path_run(&mut self, paint_op: &ContentOperation) {
        if self.path.is_empty() || self.path_ops.is_empty() {
            self.ops.push(DisplayOp::NativePatternPathOp {
                ops: vec![paint_op.clone()],
                approx_bytes: estimate_ops_bytes(std::slice::from_ref(paint_op)),
                bounds: None,
            });
            return;
        }
        let mut ops = self.path_ops.clone();
        ops.push(paint_op.clone());
        self.stats.path_segments += self.path.segments.len();
        self.stats.paths += 1;
        if matches!(paint_op.operator.as_str(), "S" | "s") {
            self.stats.strokes += 1;
        } else if matches!(paint_op.operator.as_str(), "B" | "B*" | "b" | "b*") {
            self.stats.fills += 1;
            self.stats.strokes += 1;
        } else {
            self.stats.fills += 1;
        }
        self.stats.native_pattern_path_ops += 1;
        let bounds = self.pattern_path_bounds(paint_op);
        self.ops.push(DisplayOp::NativePatternPathOp {
            approx_bytes: estimate_ops_bytes(&ops),
            ops,
            bounds,
        });
    }

    fn pattern_path_bounds(&self, paint_op: &ContentOperation) -> Option<RenderBounds> {
        match paint_op.operator.as_str() {
            "S" | "s" => self.path_bounds_for_stroke(&self.draw_state()),
            "B" | "B*" | "b" | "b*" => {
                let fill = self.path_bounds_for_fill();
                let stroke = self.path_bounds_for_stroke(&self.draw_state());
                merge_bounds(fill, stroke)
            }
            _ => self.path_bounds_for_fill(),
        }
    }

    fn path_bounds_for_fill(&self) -> Option<RenderBounds> {
        RenderBounds::from_path(&self.path, &self.ctm(), &self.viewport, 1.0)
    }

    fn path_bounds_for_stroke(&self, state: &DrawState) -> Option<RenderBounds> {
        let pad = state
            .line_width
            .abs()
            .max(1.0)
            .mul_add(state.ctm.scale_factor() * self.viewport.scale, 2.0);
        RenderBounds::from_path(&self.path, &state.ctm, &self.viewport, pad)
    }

    fn ctm(&self) -> Transform2D {
        Transform2D::from(self.gs.ctm)
    }

    fn draw_state(&self) -> DrawState {
        DrawState {
            ctm: self.ctm(),
            fill_color: resolve_simple_color(&self.gs.fill_color, self.gs.fill_alpha as f32),
            stroke_color: resolve_simple_color(&self.gs.stroke_color, self.gs.stroke_alpha as f32),
            fill_cmyk: simple_cmyk_components(&self.gs.fill_color),
            stroke_cmyk: simple_cmyk_components(&self.gs.stroke_color),
            blend_mode: self.gs.blend_mode,
            rendering_intent: self.gs.rendering_intent.clone(),
            stroke_overprint: self.gs.stroke_overprint,
            fill_overprint: self.gs.fill_overprint,
            overprint_mode: self.gs.overprint_mode,
            line_width: self.gs.line_width,
            line_cap: self.gs.line_cap.clone(),
            line_join: self.gs.line_join.clone(),
            miter_limit: self.gs.miter_limit,
            dash: if self.gs.dash.pattern.is_empty() {
                DashState::solid()
            } else {
                DashState::new(self.gs.dash.pattern.clone(), self.gs.dash.phase)
            },
        }
    }

    fn uses_pattern_or_named_space(&self) -> bool {
        matches!(self.gs.fill_color.space, ColorSpace::Named(_))
            || matches!(self.gs.stroke_color.space, ColorSpace::Named(_))
            || self.gs.fill_pattern_name.is_some()
            || self.gs.stroke_pattern_name.is_some()
    }

    fn push_native_text(&mut self, op: &ContentOperation) {
        self.stats.native_text_ops += 1;
        let bounds = self.text_show_bounds(op);
        self.ops.push(DisplayOp::NativeTextOp {
            op: op.clone(),
            approx_bytes: estimate_ops_bytes(std::slice::from_ref(op)),
            bounds,
        });
    }

    fn text_show_bounds(&self, op: &ContentOperation) -> Option<RenderBounds> {
        if !matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
            return None;
        }
        let mut start = self.gs.clone();
        match op.operator.as_str() {
            "'" => start.process(&ContentOperation::new("T*", Vec::new())),
            "\"" => {
                if let Some(word_spacing) = op.number(0) {
                    start.text.word_spacing = word_spacing;
                }
                if let Some(char_spacing) = op.number(1) {
                    start.text.char_spacing = char_spacing;
                }
                start.process(&ContentOperation::new("T*", Vec::new()));
            }
            _ => {}
        }
        let start_tm = start.text.tm;
        let mut end = self.gs.clone();
        end.process(op);
        RenderBounds::from_text_run(
            start_tm,
            end.text.tm,
            &self.ctm(),
            &self.viewport,
            start.text.font_size,
            start.text.rise,
            2.0,
        )
    }

    fn push_state_op(&mut self, op: &ContentOperation) {
        self.ops.push(DisplayOp::StateOp {
            op: op.clone(),
            approx_bytes: estimate_ops_bytes(std::slice::from_ref(op)),
        });
    }

    fn push_native_xobject(&mut self, op: &ContentOperation) {
        let subtype = op
            .name(0)
            .and_then(|name| self.resources.xobject_subtypes.get(name))
            .map(String::as_str);
        match subtype {
            Some("Image") => {
                self.stats.native_image_xobjects += 1;
                self.ops.push(DisplayOp::NativeImageXObject {
                    op: op.clone(),
                    approx_bytes: estimate_ops_bytes(std::slice::from_ref(op)),
                    bounds: self.unit_square_bounds(),
                });
            }
            Some("Form") => {
                self.stats.native_form_xobjects += 1;
                self.ops.push(DisplayOp::NativeFormXObject {
                    op: op.clone(),
                    approx_bytes: estimate_ops_bytes(std::slice::from_ref(op)),
                    bounds: op.name(0).and_then(|name| self.form_xobject_bounds(name)),
                });
            }
            _ => {
                self.ops.push(DisplayOp::NativeFormXObject {
                    op: op.clone(),
                    approx_bytes: estimate_ops_bytes(std::slice::from_ref(op)),
                    bounds: self.unit_square_bounds(),
                });
            }
        }
    }

    fn push_native_shading(&mut self, op: &ContentOperation) {
        let Some(name) = op.name(0) else {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "sh".to_string(),
                reason: "named shading operator is missing its resource name".to_string(),
            });
            return;
        };
        if !self.resources.shadings.contains_key(name) {
            self.unsupported.push(UnsupportedRenderOp {
                operator: "sh".to_string(),
                reason: format!("named shading resource /{name} is missing"),
            });
            return;
        }
        self.stats.native_shading_ops += 1;
        self.ops.push(DisplayOp::NativeShadingOp {
            op: op.clone(),
            approx_bytes: estimate_ops_bytes(std::slice::from_ref(op)),
            bounds: self.current_clip_bounds,
        });
    }

    fn push_native_inline_image(&mut self, data_op: &ContentOperation) {
        let Some(id_op) = self.pending_inline.take() else {
            self.ops.push(DisplayOp::NativeInlineImage {
                ops: vec![data_op.clone()],
                approx_bytes: estimate_ops_bytes(std::slice::from_ref(data_op)),
                bounds: self.unit_square_bounds(),
            });
            return;
        };
        let ops = vec![id_op, data_op.clone()];
        let approx_bytes = estimate_ops_bytes(&ops);
        self.stats.native_inline_images += 1;
        self.ops.push(DisplayOp::NativeInlineImage {
            ops,
            approx_bytes,
            bounds: self.unit_square_bounds(),
        });
    }

    fn unit_square_bounds(&self) -> Option<RenderBounds> {
        RenderBounds::from_unit_square(&self.ctm(), &self.viewport, 1.0)
    }

    fn form_xobject_bounds(&self, name: &str) -> Option<RenderBounds> {
        const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let bbox = *self.resources.xobject_bboxes.get(name)?;
        let matrix = self
            .resources
            .xobject_matrices
            .get(name)
            .copied()
            .unwrap_or(IDENTITY);
        let ctm = Transform2D::from(matrix).concat(&self.ctm());
        RenderBounds::from_bbox(bbox, &ctm, &self.viewport, 1.0)
    }
}

fn resolve_simple_color(color: &Color, alpha: f32) -> PixelColor {
    if matches!(color.space, ColorSpace::Named(_)) {
        return crate::render::color::RenderColor::transparent().to_pixel_color();
    }
    ColorSpaceHandler::to_render_color(color, alpha).to_pixel_color()
}

fn simple_cmyk_components(color: &Color) -> Option<[f32; 4]> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::operation::Operand;
    use crate::object::{PdfDictionary, PdfObject};
    use crate::render::buffer::{BLACK, RED};

    fn op(operator: &str, operands: Vec<Operand>) -> ContentOperation {
        ContentOperation::new(operator, operands)
    }

    fn num(n: f64) -> Operand {
        Operand::Real(n)
    }

    #[test]
    fn captures_and_replays_simple_fill() {
        let ops = vec![
            op("rg", vec![num(1.0), num(0.0), num(0.0)]),
            op("re", vec![num(10.0), num(10.0), num(20.0), num(20.0)]),
            op("f", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.fills, 1);
        let buf = render_display_list(&list, RenderMode::Compat);
        assert_eq!(buf.get_pixel(20, 30), RED);
    }

    #[test]
    fn path_render_bounds_are_full_page_pixel_space() {
        let full = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let mut path = Path::new();
        path.rect(10.0, 10.0, 20.0, 20.0);

        let bounds = RenderBounds::from_path(&path, &Transform2D::identity(), &full, 0.0)
            .expect("rectangle should produce bounds");

        assert!(bounds.intersects_viewport(&full.pixel_window(10, 20, 5, 5)));
        assert!(!bounds.intersects_viewport(&full.pixel_window(0, 0, 5, 5)));
    }

    #[test]
    fn captured_fill_path_carries_culling_bounds() {
        let ops = vec![
            op("re", vec![num(10.0), num(10.0), num(20.0), num(20.0)]),
            op("f", vec![]),
        ];
        let full = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let list = build_display_list(&ops, full.clone(), &PageResources::default());
        let Some(DisplayOp::FillPath {
            bounds: Some(bounds),
            ..
        }) = list
            .ops
            .iter()
            .find(|op| matches!(op, DisplayOp::FillPath { .. }))
        else {
            panic!("expected captured fill path bounds");
        };

        assert!(bounds.intersects_viewport(&full.pixel_window(10, 20, 5, 5)));
        assert!(!bounds.intersects_viewport(&full.pixel_window(0, 0, 5, 5)));
    }

    #[test]
    fn captures_clip_save_restore_and_stroke() {
        let ops = vec![
            op("q", vec![]),
            op("re", vec![num(0.0), num(0.0), num(20.0), num(20.0)]),
            op("W", vec![]),
            op("n", vec![]),
            op("w", vec![num(3.0)]),
            op("m", vec![num(2.0), num(2.0)]),
            op("l", vec![num(18.0), num(18.0)]),
            op("S", vec![]),
            op("Q", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(list.is_fully_supported());
        assert_eq!(list.stats.clips, 1);
        assert_eq!(list.stats.strokes, 1);
        assert_eq!(list.stats.max_stack_depth, 1);
        let buf = render_display_list(&list, RenderMode::Compat);
        assert_ne!(buf.get_pixel(10, 10), WHITE);
        assert_eq!(buf.get_pixel(19, 1), WHITE);
    }

    #[test]
    fn text_is_replayable_as_native_operation() {
        let ops = vec![op("Tj", vec![Operand::String(b"hello".to_vec())])];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert_eq!(list.stats.text_ops, 1);
        assert_eq!(list.stats.native_text_ops, 1);
        assert_eq!(list.stats.compatibility_runs, 0);
        assert_eq!(list.stats.compatibility_ops, 0);
        assert!(matches!(list.ops[0], DisplayOp::NativeTextOp { .. }));
    }

    #[test]
    fn text_showing_native_op_records_tile_culling_bounds() {
        let ops = vec![
            op("BT", vec![]),
            op("Tf", vec![Operand::Name("F1".to_string()), num(12.0)]),
            op("Td", vec![num(40.0), num(40.0)]),
            op("Tj", vec![Operand::String(b"hello".to_vec())]),
            op("ET", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(&ops, viewport.clone(), &PageResources::default());
        let text_bounds = list.ops.iter().find_map(|op| match op {
            DisplayOp::NativeTextOp {
                op,
                bounds: Some(bounds),
                ..
            } if op.operator == "Tj" => Some(*bounds),
            _ => None,
        });

        let bounds = text_bounds.expect("showing text should carry conservative bounds");
        assert!(bounds.intersects_viewport(&viewport));
        assert!(!bounds.intersects_viewport(&viewport.pixel_window(0, 0, 10, 10)));
    }

    #[test]
    fn text_showing_bounds_use_pre_advance_text_matrix() {
        let ops = vec![
            op("BT", vec![]),
            op("Tf", vec![Operand::Name("F1".to_string()), num(12.0)]),
            op("Td", vec![num(40.0), num(40.0)]),
            op("Tj", vec![Operand::String(b"hello".to_vec())]),
            op("ET", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(&ops, viewport.clone(), &PageResources::default());
        let text_bounds = list.ops.iter().find_map(|op| match op {
            DisplayOp::NativeTextOp {
                op,
                bounds: Some(bounds),
                ..
            } if op.operator == "Tj" => Some(*bounds),
            _ => None,
        });

        let bounds = text_bounds.expect("showing text should carry conservative bounds");
        assert!(
            bounds.intersects_viewport(&viewport.pixel_window(38, 45, 4, 20)),
            "text culling bounds must include the glyph start, not only the post-showing advance"
        );
    }

    #[test]
    fn alpha_ext_gstate_does_not_force_page_compatibility_run() {
        let ops = vec![
            op("gs", vec![Operand::Name("GS1".to_string())]),
            op("rg", vec![num(1.0), num(0.0), num(0.0)]),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        let mut gs = PdfDictionary::empty();
        gs.insert("ca", PdfObject::Real(0.5));
        gs.insert("CA", PdfObject::Real(0.5));
        resources.ext_g_states.insert("GS1".to_string(), gs);

        let list = build_display_list(&ops, viewport.clone(), &resources);

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert_eq!(list.stats.transparency_ops, 1);
        assert_eq!(list.stats.compatibility_runs, 0);
        assert!(matches!(list.ops[0], DisplayOp::StateOp { .. }));
        assert!(matches!(
            list.ops.iter().find(|op| matches!(op, DisplayOp::FillPath { .. })),
            Some(DisplayOp::FillPath { state, .. }) if state.fill_color[3] < 255
        ));
    }

    #[test]
    fn ordinary_marked_content_does_not_force_page_compatibility_run() {
        let ops = vec![
            op(
                "BDC",
                vec![
                    Operand::Name("Span".to_string()),
                    Operand::Dictionary(vec![(
                        "Lang".to_string(),
                        Operand::String(b"en-US".to_vec()),
                    )]),
                ],
            ),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("EMC", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);

        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert_eq!(list.stats.optional_content_ops, 0);
        assert_eq!(list.stats.compatibility_runs, 0);
        assert!(list
            .ops
            .iter()
            .any(|op| matches!(op, DisplayOp::FillPath { .. })));
    }

    #[test]
    fn optional_content_marked_content_replays_as_state_ops_without_page_fallback() {
        let ops = vec![
            op(
                "BDC",
                vec![
                    Operand::Name("OC".to_string()),
                    Operand::Name("Layer1".to_string()),
                ],
            ),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("EMC", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        let mut ocg = PdfDictionary::empty();
        ocg.insert("Type", PdfObject::Name("OCG".to_string()));
        resources
            .properties
            .insert("Layer1".to_string(), PdfObject::Dictionary(ocg));

        let list = build_display_list(&ops, viewport, &resources);

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert_eq!(list.stats.optional_content_ops, 1);
        assert_eq!(list.stats.compatibility_runs, 0);
        assert!(list
            .ops
            .iter()
            .any(|op| matches!(op, DisplayOp::StateOp { op, .. } if op.operator == "BDC")));
    }

    #[test]
    fn inline_optional_content_dictionary_replays_as_state_ops_without_page_fallback() {
        let ops = vec![
            op(
                "BDC",
                vec![
                    Operand::Name("OC".to_string()),
                    Operand::Dictionary(vec![(
                        "Type".to_string(),
                        Operand::Name("OCMD".to_string()),
                    )]),
                ],
            ),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("EMC", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);

        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert_eq!(list.stats.optional_content_ops, 1);
        assert_eq!(list.stats.compatibility_runs, 0);
    }

    #[test]
    fn image_xobject_native_op_records_tile_culling_bounds() {
        let ops = vec![
            op(
                "cm",
                vec![
                    num(10.0),
                    num(0.0),
                    num(0.0),
                    num(10.0),
                    num(30.0),
                    num(30.0),
                ],
            ),
            op("Do", vec![Operand::Name("Im1".to_string())]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut resources = PageResources::default();
        resources
            .xobject_subtypes
            .insert("Im1".to_string(), "Image".to_string());

        let list = build_display_list(&ops, viewport.clone(), &resources);
        let image_bounds = list.ops.iter().find_map(|op| match op {
            DisplayOp::NativeImageXObject { bounds, .. } => *bounds,
            _ => None,
        });

        let bounds = image_bounds.expect("native image op should carry bounds");
        assert!(bounds.intersects_viewport(&viewport));
        assert!(!bounds.intersects_viewport(&viewport.pixel_window(0, 0, 10, 10)));
    }

    #[test]
    fn named_shading_is_replayable_as_native_operation() {
        let ops = vec![op("sh", vec![Operand::Name("S1".to_string())])];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        resources.shadings.insert(
            "S1".to_string(),
            PdfObject::Dictionary(PdfDictionary::empty()),
        );

        let list = build_display_list(&ops, viewport.clone(), &resources);

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert_eq!(list.stats.shadings, 1);
        assert_eq!(list.stats.native_shading_ops, 1);
        assert_eq!(list.stats.compatibility_runs, 0);
        assert!(matches!(list.ops[0], DisplayOp::NativeShadingOp { .. }));
    }

    #[test]
    fn missing_named_shading_is_explicitly_unsupported() {
        let ops = vec![op("sh", vec![Operand::Name("S1".to_string())])];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);

        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(!list.is_fully_supported());
        assert_eq!(list.stats.shadings, 1);
        assert_eq!(list.stats.native_shading_ops, 0);
        assert_eq!(list.unsupported.len(), 1);
        assert_eq!(list.unsupported[0].operator, "sh");
        assert!(list.unsupported[0].reason.contains("missing"));
    }

    #[test]
    fn pattern_fill_uses_native_path_replay_when_resource_is_available() {
        let ops = vec![
            op("q", vec![]),
            op("cs", vec![Operand::Name("Pattern".to_string())]),
            op("scn", vec![Operand::Name("P1".to_string())]),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("Q", vec![]),
            op("rg", vec![num(1.0), num(0.0), num(0.0)]),
            op("re", vec![num(10.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let mut resources = PageResources::default();
        resources.patterns.insert(
            "P1".to_string(),
            PdfObject::Dictionary(PdfDictionary::empty()),
        );

        let list = build_display_list(&ops, viewport.clone(), &resources);

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert_eq!(list.stats.compatibility_runs, 0);
        assert_eq!(list.stats.native_pattern_path_ops, 1);
        assert_eq!(list.stats.fills, 2);
        let pattern_bounds = list.ops.iter().find_map(|op| match op {
            DisplayOp::NativePatternPathOp { bounds, .. } => *bounds,
            _ => None,
        });
        let bounds = pattern_bounds.expect("native pattern path should carry bounds");
        assert!(bounds.intersects_viewport(&viewport));
        assert!(!bounds.intersects_viewport(&viewport.pixel_window(12, 12, 4, 4)));
        assert!(list
            .ops
            .iter()
            .any(|op| matches!(op, DisplayOp::FillPath { .. })));
    }

    #[test]
    fn missing_pattern_resource_stays_native_and_replays_canonical_noop() {
        let ops = vec![
            op("q", vec![]),
            op("cs", vec![Operand::Name("Pattern".to_string())]),
            op("scn", vec![Operand::Name("P1".to_string())]),
            op("re", vec![num(1.0), num(1.0), num(8.0), num(8.0)]),
            op("f", vec![]),
            op("Q", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);

        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(list.is_fully_supported());
        assert!(!list.has_compatibility_runs());
        assert_eq!(list.stats.compatibility_runs, 0);
        assert_eq!(list.stats.native_pattern_path_ops, 1);
        assert!(matches!(
            list.ops
                .iter()
                .find(|op| matches!(op, DisplayOp::NativePatternPathOp { .. })),
            Some(DisplayOp::NativePatternPathOp { .. })
        ));
    }

    #[test]
    fn stroke_color_is_replayed() {
        let ops = vec![
            op("RG", vec![num(0.0), num(0.0), num(0.0)]),
            op("w", vec![num(4.0)]),
            op("m", vec![num(5.0), num(5.0)]),
            op("l", vec![num(25.0), num(5.0)]),
            op("S", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 30.0, 30.0], 72);
        let list = build_display_list(&ops, viewport, &PageResources::default());
        let buf = render_display_list(&list, RenderMode::Compat);

        assert_eq!(buf.get_pixel(10, 25), BLACK);
    }

    #[test]
    fn ext_gstate_overprint_metadata_is_captured() {
        let mut resources = PageResources::default();
        let mut gs_dict = crate::object::PdfDictionary::empty();
        gs_dict.insert("OP", crate::object::PdfObject::Boolean(true));
        gs_dict.insert("op", crate::object::PdfObject::Boolean(false));
        gs_dict.insert("OPM", crate::object::PdfObject::Integer(1));
        gs_dict.insert(
            "RI",
            crate::object::PdfObject::Name("AbsoluteColorimetric".to_string()),
        );
        resources.ext_g_states.insert("GS1".to_string(), gs_dict);
        let ops = vec![
            op("gs", vec![Operand::Name("GS1".to_string())]),
            op("re", vec![num(1.0), num(1.0), num(5.0), num(5.0)]),
            op("f", vec![]),
        ];
        let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        let list = build_display_list(&ops, viewport, &resources);

        let Some(DisplayOp::FillPath { state, .. }) = list
            .ops
            .iter()
            .find(|op| matches!(op, DisplayOp::FillPath { .. }))
        else {
            panic!("expected captured fill path");
        };
        assert!(state.stroke_overprint);
        assert!(!state.fill_overprint);
        assert_eq!(state.overprint_mode, 1);
        assert_eq!(state.rendering_intent, "AbsoluteColorimetric");
    }

    #[test]
    fn render_cache_hits_and_evicts_by_budget() {
        let tile_a = RenderTile {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let tile_b = RenderTile {
            x: 2,
            y: 0,
            width: 2,
            height: 2,
        };
        let key_a = RenderCacheKey::new(1, 72, RenderMode::Compat, tile_a);
        let key_b = RenderCacheKey::new(1, 72, RenderMode::Compat, tile_b);
        let mut cache = RenderCache::new(16, 16);
        let mut buf = PixelBuffer::new_transparent_with_mode(2, 2, RenderMode::Compat);
        buf.set_pixel(0, 0, RED);

        cache.insert(key_a.clone(), buf.clone());
        assert!(cache.get(&key_a).is_some());
        cache.insert(
            key_b.clone(),
            PixelBuffer::new_transparent_with_mode(2, 2, RenderMode::Compat),
        );

        let metrics = cache.metrics();
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.evictions, 1);
        assert_eq!(metrics.bytes, 16);
        assert!(cache.get(&key_b).is_some());
    }

    #[test]
    fn render_cache_skips_oversized_entries() {
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        let key = RenderCacheKey::new(1, 72, RenderMode::Compat, tile);
        let mut cache = RenderCache::new(64, 64);
        cache.insert(
            key.clone(),
            PixelBuffer::new_transparent_with_mode(10, 10, RenderMode::Compat),
        );

        assert!(cache.get(&key).is_none());
        assert_eq!(cache.metrics().skipped_oversized, 1);
    }

    #[test]
    fn render_cache_key_includes_visibility_fingerprint() {
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        let visible = RenderCacheKey::new_with_visibility(
            1,
            72,
            RenderMode::Compat,
            tile,
            "ocg:view:visible",
        );
        let hidden =
            RenderCacheKey::new_with_visibility(1, 72, RenderMode::Compat, tile, "ocg:view:hidden");

        assert_ne!(visible, hidden);
        let changed_revision = RenderCacheKey::new_with_full_identity(
            1,
            72,
            RenderMode::Compat,
            tile,
            "ocg:view:visible",
            "prepress:none",
            "revision:two",
            "contract:one",
        );
        let changed_contract = RenderCacheKey::new_with_full_identity(
            1,
            72,
            RenderMode::Compat,
            tile,
            "ocg:view:visible",
            "prepress:none",
            "revision:one",
            "contract:two",
        );
        let baseline = RenderCacheKey::new_with_full_identity(
            1,
            72,
            RenderMode::Compat,
            tile,
            "ocg:view:visible",
            "prepress:none",
            "revision:one",
            "contract:one",
        );
        assert_ne!(baseline, changed_revision);
        assert_ne!(baseline, changed_contract);
        let mut cache = RenderCache::new(4_000, 4_000);
        cache.insert(
            visible.clone(),
            PixelBuffer::new_transparent_with_mode(10, 10, RenderMode::Compat),
        );

        assert!(cache.get(&hidden).is_none());
        assert!(cache.get(&visible).is_some());
    }
}
