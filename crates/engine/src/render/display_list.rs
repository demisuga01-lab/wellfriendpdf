//! Display-list capture and replay for PDF rendering.
//!
//! This module is intentionally conservative: the normalized vector operations
//! are replayed directly through the CPU rasterizer, while higher-level content
//! categories are represented as typed native replay operations or measured
//! compatibility fallbacks. That bridge is the display-list path for text,
//! images, XObjects, shadings, patterns, and transparency until later font/color
//! passes deepen those primitives.

use crate::content::operation::ContentOperation;
use crate::content::state::{BlendMode, Color, ColorSpace, GraphicsState, LineCap, LineJoin};
use crate::engine::PageResources;
use crate::render::buffer::{ClipMask, PixelBuffer, PixelColor, RenderMode, WHITE};
use crate::render::color::ColorSpaceHandler;
use crate::render::line::DashState;
use crate::render::path::{flatten_path, FillRule, Path, PathPainter};
use crate::render::transform::{Transform2D, Viewport};
use std::collections::{BTreeMap, HashMap};

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
        self.ops
            .iter()
            .any(|op| matches!(op, DisplayOp::ContentRun { .. }))
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
                DisplayOp::ContentRun { approx_bytes, .. }
                | DisplayOp::StateOp { approx_bytes, .. } => *approx_bytes,
                DisplayOp::NativeTextOp { approx_bytes, .. }
                | DisplayOp::NativeImageXObject { approx_bytes, .. }
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
    },
    FillPath {
        path: Path,
        state: DrawState,
        rule: FillRule,
    },
    StrokePath {
        path: Path,
        state: DrawState,
    },
    /// Replayable bridge to the existing content-stream renderer.
    ///
    /// This is deliberately typed and accounted for instead of being a silent
    /// fallback. It lets the display-list path cover real pages now while
    /// keeping font/color/image semantics exactly aligned with the immediate
    /// renderer until those primitives are normalized in future passes.
    ContentRun {
        kind: DisplayRunKind,
        ops: Vec<ContentOperation>,
        approx_bytes: usize,
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
    },
    /// Native replay of an Image XObject `Do` operation.
    NativeImageXObject {
        op: ContentOperation,
        approx_bytes: usize,
    },
    /// Native replay of an inline image `ID` plus payload operation.
    NativeInlineImage {
        ops: Vec<ContentOperation>,
        approx_bytes: usize,
    },
    /// Native replay of a Form XObject `Do` operation.
    NativeFormXObject {
        op: ContentOperation,
        approx_bytes: usize,
    },
}

impl DisplayOp {
    pub fn is_native_high_level(&self) -> bool {
        matches!(
            self,
            DisplayOp::NativeTextOp { .. }
                | DisplayOp::NativeImageXObject { .. }
                | DisplayOp::NativeInlineImage { .. }
                | DisplayOp::NativeFormXObject { .. }
        )
    }
}

/// Coarse category for a compatibility content run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisplayRunKind {
    PageContent,
    Text,
    Image,
    InlineImage,
    FormXObject,
    Shading,
    Pattern,
    Transparency,
    Mixed,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// Stable key for the bounded render cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderCacheKey {
    pub page_number: usize,
    pub dpi: u32,
    pub render_mode: &'static str,
    pub tile: RenderTile,
    pub visibility_fingerprint: String,
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
        Self {
            page_number,
            dpi,
            render_mode: render_mode.as_str(),
            tile,
            visibility_fingerprint: visibility_fingerprint.into(),
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
    fn content_run(&mut self, kind: DisplayRunKind, ops: &[ContentOperation]) {
        log::warn!(
            "DisplayList device cannot replay {kind:?} compatibility run ({} ops)",
            ops.len()
        );
    }
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
        }
    }

    pub fn into_buffer(self) -> PixelBuffer {
        self.buf
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
        PathPainter::fill(
            &mut self.buf,
            path,
            &state.ctm,
            &self.viewport,
            state.fill_color,
            rule,
        );
        self.buf.blend_mode = saved_blend;
    }

    fn stroke_path(&mut self, path: &Path, state: &DrawState) {
        let saved_blend = self.buf.blend_mode;
        self.buf.blend_mode = state.blend_mode;
        PathPainter::stroke_with_style(
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
        );
        self.buf.blend_mode = saved_blend;
    }
}

pub fn replay_display_list(list: &DisplayList, device: &mut dyn RenderDevice) {
    for op in &list.ops {
        match op {
            DisplayOp::Save => device.save(),
            DisplayOp::Restore => device.restore(),
            DisplayOp::Clip { path, ctm, rule } => device.clip_path(path, ctm, *rule),
            DisplayOp::FillPath { path, state, rule } => device.fill_path(path, state, *rule),
            DisplayOp::StrokePath { path, state } => device.stroke_path(path, state),
            DisplayOp::ContentRun { kind, ops, .. } => device.content_run(*kind, ops),
            DisplayOp::StateOp { op, .. } => device.state_op(op),
            DisplayOp::NativeTextOp { op, .. } => device.native_text_op(op),
            DisplayOp::NativeImageXObject { op, .. } => device.native_image_xobject(op),
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
    if let Some(reason) = page_compatibility_fallback_reason(&stats) {
        let approx_bytes = estimate_ops_bytes(ops);
        let mut compatibility_fallback_reasons = BTreeMap::new();
        compatibility_fallback_reasons.insert(reason.to_string(), 1);
        return DisplayList {
            viewport,
            ops: vec![DisplayOp::ContentRun {
                kind: DisplayRunKind::PageContent,
                ops: ops.to_vec(),
                approx_bytes,
            }],
            stats: DisplayListStats {
                operations: 1,
                compatibility_runs: 1,
                compatibility_ops: ops.len(),
                compatibility_bytes: approx_bytes,
                compatibility_fallback_reasons,
                ..stats
            },
            supported: true,
            unsupported: Vec::new(),
        };
    }

    let mut builder = DisplayListBuilder::new(viewport, resources);
    builder.stats = stats;
    builder.dispatch_all(ops);
    builder.finish()
}

fn page_compatibility_fallback_reason(stats: &DisplayListStats) -> Option<&'static str> {
    if stats.transparency_ops > 0 {
        Some("unsupported_graphics_state")
    } else if stats.optional_content_ops > 0 {
        Some("optional_content_visibility_requires_immediate_interpreter")
    } else if stats.shadings > 0 {
        Some("unsupported_operator_shading")
    } else if stats.patterns > 0 {
        Some("unsupported_operator_pattern")
    } else {
        None
    }
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
    }
}

fn classify_content(ops: &[ContentOperation], resources: &PageResources) -> DisplayListStats {
    let mut stats = DisplayListStats::default();
    if !resources.properties.is_empty() {
        stats.optional_content_ops += resources.properties.len();
    }
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
            "BMC" | "BDC" | "EMC" => stats.optional_content_ops += 1,
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

struct DisplayListBuilder<'a> {
    viewport: Viewport,
    resources: &'a PageResources,
    gs: GraphicsState,
    path: Path,
    pending_clip: Option<FillRule>,
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
            pending_clip: None,
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
                self.ops.push(DisplayOp::Save);
                self.stats.saves += 1;
                self.gs.process(op);
                self.stats.max_stack_depth = self.stats.max_stack_depth.max(self.gs.stack_depth());
            }
            "Q" => {
                self.gs.process(op);
                self.ops.push(DisplayOp::Restore);
                self.stats.restores += 1;
            }
            "cm" | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "G" | "g" | "RG" | "rg" | "K"
            | "k" | "CS" | "cs" | "SC" | "SCN" | "sc" | "scn" => {
                self.gs.process(op);
                self.push_state_op(op);
                if self.uses_pattern_or_named_space() {
                    self.mark_unsupported(
                        op,
                        "named/pattern color spaces require resource resolution",
                    );
                }
            }
            "gs" => {
                self.apply_ext_g_state(op);
                self.push_state_op(op);
            }
            "BMC" | "BDC" | "EMC" | "MP" | "DP" | "BX" | "EX" => {}
            "BT" | "ET" | "Tf" | "Td" | "TD" | "Tm" | "T*" | "Tc" | "Tw" | "Tz" | "TL" | "Tr"
            | "Ts" | "Tj" | "TJ" | "'" | "\"" => {
                self.gs.process(op);
                self.push_native_text(op);
            }
            "Do" => self.push_native_xobject(op),
            "sh" => self.mark_unsupported(op, "shading replay is not captured yet"),
            "BI" | "EI" => {}
            "ID" => {
                self.pending_inline = Some(op.clone());
            }
            "inline_image_data" => self.push_native_inline_image(op),
            _ => {
                self.gs.process(op);
                self.mark_unsupported(op, "operator is not represented in display-list subset");
            }
        }
    }

    fn apply_ext_g_state(&mut self, op: &ContentOperation) {
        let Some(name) = op.name(0) else {
            self.mark_unsupported(op, "malformed ExtGState operator");
            return;
        };
        let Some(dict) = self.resources.ext_g_states.get(name) else {
            self.mark_unsupported(op, "ExtGState resource not found");
            return;
        };
        if dict.get("SMask").is_some() {
            self.mark_unsupported(op, "soft mask ExtGState requires group replay");
            return;
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
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.clips += 1;
            self.ops.push(DisplayOp::Clip {
                path,
                ctm: self.ctm(),
                rule,
            });
        }
    }

    fn stroke_and_clear(&mut self) {
        self.apply_pending_clip();
        if !self.path.is_empty() {
            let path = self.path.clone();
            self.stats.path_segments += path.segments.len();
            self.stats.paths += 1;
            self.stats.strokes += 1;
            self.ops.push(DisplayOp::StrokePath {
                path,
                state: self.draw_state(),
            });
        }
        self.path.clear();
    }

    fn fill_and_clear(&mut self, rule: FillRule) {
        self.apply_pending_clip();
        if self.uses_pattern_or_named_space() {
            self.mark_synthetic_unsupported("pattern/named fill cannot be replayed yet");
            self.path.clear();
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
            });
        }
        self.path.clear();
    }

    fn fill_stroke_and_clear(&mut self, rule: FillRule) {
        self.apply_pending_clip();
        if self.uses_pattern_or_named_space() {
            self.mark_synthetic_unsupported("pattern/named fill cannot be replayed yet");
            self.path.clear();
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
            });
            self.stats.strokes += 1;
            self.ops.push(DisplayOp::StrokePath { path, state });
        }
        self.path.clear();
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
        self.ops.push(DisplayOp::NativeTextOp {
            op: op.clone(),
            approx_bytes: estimate_ops_bytes(std::slice::from_ref(op)),
        });
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
                });
            }
            Some("Form") => {
                self.stats.native_form_xobjects += 1;
                self.ops.push(DisplayOp::NativeFormXObject {
                    op: op.clone(),
                    approx_bytes: estimate_ops_bytes(std::slice::from_ref(op)),
                });
            }
            _ => {
                self.push_compatibility_fallback(
                    DisplayRunKind::Mixed,
                    std::slice::from_ref(op),
                    "unsupported_xobject_subtype",
                );
            }
        }
    }

    fn push_native_inline_image(&mut self, data_op: &ContentOperation) {
        let Some(id_op) = self.pending_inline.take() else {
            self.mark_unsupported(data_op, "inline image data without ID parameters");
            return;
        };
        let ops = vec![id_op, data_op.clone()];
        let approx_bytes = estimate_ops_bytes(&ops);
        self.stats.native_inline_images += 1;
        self.ops
            .push(DisplayOp::NativeInlineImage { ops, approx_bytes });
    }

    fn push_compatibility_fallback(
        &mut self,
        kind: DisplayRunKind,
        ops: &[ContentOperation],
        reason: &str,
    ) {
        let approx_bytes = estimate_ops_bytes(ops);
        self.stats.compatibility_runs += 1;
        self.stats.compatibility_ops += ops.len();
        self.stats.compatibility_bytes += approx_bytes;
        *self
            .stats
            .compatibility_fallback_reasons
            .entry(reason.to_string())
            .or_insert(0) += 1;
        self.ops.push(DisplayOp::ContentRun {
            kind,
            ops: ops.to_vec(),
            approx_bytes,
        });
    }

    fn mark_unsupported(&mut self, op: &ContentOperation, reason: &str) {
        self.unsupported.push(UnsupportedRenderOp {
            operator: op.operator.clone(),
            reason: reason.to_string(),
        });
    }

    fn mark_synthetic_unsupported(&mut self, reason: &str) {
        self.unsupported.push(UnsupportedRenderOp {
            operator: "paint".to_string(),
            reason: reason.to_string(),
        });
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
        let mut cache = RenderCache::new(4_000, 4_000);
        cache.insert(
            visible.clone(),
            PixelBuffer::new_transparent_with_mode(10, 10, RenderMode::Compat),
        );

        assert!(cache.get(&hidden).is_none());
        assert!(cache.get(&visible).is_some());
    }
}
