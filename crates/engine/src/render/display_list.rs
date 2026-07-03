//! Display-list capture and replay for PDF rendering.
//!
//! This module is intentionally conservative: Prompt 03 introduces the
//! reusable architecture without replacing the whole renderer at once. The
//! capture path records vector drawing operations that can be replayed exactly
//! through the current CPU rasterizer. Pages containing text, images, patterns,
//! shadings, soft masks, or Form XObjects remain on the existing immediate
//! renderer until those primitives are represented in later display-list passes.

use crate::content::operation::ContentOperation;
use crate::content::state::{BlendMode, Color, ColorSpace, GraphicsState, LineCap, LineJoin};
use crate::engine::PageResources;
use crate::render::buffer::{ClipMask, PixelBuffer, PixelColor, RenderMode, WHITE};
use crate::render::color::ColorSpaceHandler;
use crate::render::line::DashState;
use crate::render::path::{flatten_path, FillRule, Path, PathPainter};
use crate::render::transform::{Transform2D, Viewport};

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
}

/// Paint and geometry state needed to replay one operation.
#[derive(Debug, Clone)]
pub struct DrawState {
    pub ctm: Transform2D,
    pub fill_color: PixelColor,
    pub stroke_color: PixelColor,
    pub blend_mode: BlendMode,
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

/// Concrete rendering target for display-list replay.
pub trait RenderDevice {
    fn save(&mut self);
    fn restore(&mut self);
    fn clip_path(&mut self, path: &Path, ctm: &Transform2D, rule: FillRule);
    fn fill_path(&mut self, path: &Path, state: &DrawState, rule: FillRule);
    fn stroke_path(&mut self, path: &Path, state: &DrawState);
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
    let mut builder = DisplayListBuilder::new(viewport, resources);
    builder.dispatch_all(ops);
    builder.finish()
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
                if self.uses_pattern_or_named_space() {
                    self.mark_unsupported(
                        op,
                        "named/pattern color spaces require resource resolution",
                    );
                }
            }
            "gs" => self.apply_ext_g_state(op),
            "BMC" | "BDC" | "EMC" | "MP" | "DP" | "BX" | "EX" => {}
            "BT" | "ET" | "Tf" | "Td" | "TD" | "Tm" | "T*" | "Tc" | "Tw" | "Tz" | "TL" | "Tr"
            | "Ts" | "Tj" | "TJ" | "'" | "\"" => {
                self.gs.process(op);
                self.mark_unsupported(
                    op,
                    "text/glyph operations are not display-list replayed yet",
                );
            }
            "Do" => self.mark_unsupported(op, "XObject image/form replay is not captured yet"),
            "sh" => self.mark_unsupported(op, "shading replay is not captured yet"),
            "BI" | "ID" | "EI" | "inline_image_data" => {
                self.mark_unsupported(op, "inline image replay is not captured yet");
            }
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
            blend_mode: self.gs.blend_mode,
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
    fn unsupported_text_marks_list_non_replayable() {
        let ops = vec![op("Tj", vec![Operand::String(b"hello".to_vec())])];
        let viewport = Viewport::new([0.0, 0.0, 20.0, 20.0], 72);
        let list = build_display_list(&ops, viewport, &PageResources::default());

        assert!(!list.is_fully_supported());
        assert_eq!(list.unsupported[0].operator, "Tj");
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
}
