use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::engine::ContentEngine;
use crate::error::{Result, WellfriendError};
use crate::optional_content::OptionalContentContext;
use crate::render::{
    PageRenderer, PixelBuffer, RenderDocumentCache, RenderMode, RenderResourceBudget, RenderTile,
    WHITE,
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
    pub viewport_hint: Option<RenderTile>,
    pub resumable: bool,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveRenderStepReport {
    pub lifecycle_state: String,
    pub phase: String,
    pub completed_tiles: Vec<RenderTile>,
    pub warnings: Vec<String>,
    pub fallback_events: Vec<String>,
    pub completed_units: usize,
    pub total_units: usize,
    pub rendered_this_step: usize,
    pub next_tile_index: usize,
    pub cancelled: bool,
    pub resume_possible: bool,
    pub memory_bytes_retained: usize,
    pub visibility_fingerprint: String,
}

pub struct ProgressiveRenderJob {
    engine: ContentEngine,
    page_number: usize,
    dpi: u32,
    render_mode: RenderMode,
    tile_width: u32,
    tile_height: u32,
    page_width: u32,
    page_height: u32,
    tiles: Vec<RenderTile>,
    tile_order: Vec<usize>,
    viewport_hint: Option<RenderTile>,
    completed: Vec<Option<PixelBuffer>>,
    next_tile_index: usize,
    visibility_fingerprint: String,
    document_cache: RenderDocumentCache,
    state: ProgressiveRenderState,
    warnings: Vec<String>,
    fallback_events: Vec<String>,
    aborted: bool,
}

const ADAPTIVE_TILE_SIZES: [u32; 5] = [128, 192, 256, 384, 512];

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
        let viewport = engine.page_viewport(page_number, dpi)?;
        let (tile_width, tile_height) = if tile_width == 0 || tile_height == 0 {
            choose_adaptive_tile_size(
                viewport.width_px,
                viewport.height_px,
                RenderResourceBudget::default(),
            )
        } else {
            (tile_width, tile_height)
        };
        let mut tiles = Vec::new();
        let mut y = 0;
        while y < viewport.height_px {
            let height = tile_height.min(viewport.height_px - y);
            let mut x = 0;
            while x < viewport.width_px {
                let width = tile_width.min(viewport.width_px - x);
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
        let visibility_fingerprint = OptionalContentContext::from_document(engine.document())
            .visibility_fingerprint()
            .to_string();
        Ok(Self {
            engine,
            page_number,
            dpi,
            render_mode,
            tile_width,
            tile_height,
            page_width: viewport.width_px,
            page_height: viewport.height_px,
            tiles,
            tile_order,
            viewport_hint,
            completed: vec![None; total],
            next_tile_index: 0,
            visibility_fingerprint,
            document_cache: RenderDocumentCache::new(),
            state: ProgressiveRenderState::Created,
            warnings: Vec::new(),
            fallback_events: Vec::new(),
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
            viewport_hint: self.viewport_hint,
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
            (token.viewport_hint != self.viewport_hint).then_some("viewport_hint"),
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
        let mut rendered = 0;
        let mut cancelled = false;
        while self.next_tile_index < self.tiles.len() && rendered < max_tiles {
            if cancel.is_cancelled() {
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
            let tile = self.tiles[index];
            let rendered_tile =
                match PageRenderer::render_page_display_list_tile_cancellable_with_mode_and_cache(
                    &self.engine,
                    self.page_number,
                    self.dpi,
                    tile,
                    cancel,
                    self.render_mode,
                    &mut self.document_cache,
                ) {
                    Ok(Some(buffer)) => Ok(buffer),
                    Ok(None) => {
                        self.fallback_events
                            .push("unsupported_display_list_immediate_tile".to_string());
                        PageRenderer::render_page_tile_cancellable_with_mode(
                            &self.engine,
                            self.page_number,
                            self.dpi,
                            tile,
                            cancel,
                            self.render_mode,
                        )
                    }
                    Err(error) => Err(error),
                };
            let buffer = match rendered_tile {
                Ok(buffer) => buffer,
                Err(error) => {
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

    fn step_report(
        &self,
        rendered_this_step: usize,
        cancelled: bool,
    ) -> ProgressiveRenderStepReport {
        let completed_tiles = self
            .tiles
            .iter()
            .copied()
            .zip(self.completed.iter())
            .filter_map(|(tile, buffer)| buffer.is_some().then_some(tile))
            .collect();
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
            warnings: self.warnings.clone(),
            fallback_events: self.fallback_events.clone(),
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
        }
    }

    pub fn is_complete(&self) -> bool {
        self.next_tile_index >= self.tiles.len() && self.completed.iter().all(Option::is_some)
    }

    pub fn finish(&self) -> Option<PixelBuffer> {
        if !self.is_complete() {
            return None;
        }
        let mut out = PixelBuffer::new_filled_with_mode(
            self.page_width,
            self.page_height,
            WHITE,
            self.render_mode,
        );
        for (tile, buffer) in self.tiles.iter().zip(self.completed.iter()) {
            let buffer = buffer.as_ref()?;
            if !out.blit_from_buffer(buffer, tile.x, tile.y) {
                return None;
            }
        }
        Some(out)
    }

    fn completed_count(&self) -> usize {
        self.completed.iter().filter(|tile| tile.is_some()).count()
    }

    fn memory_bytes_retained(&self) -> usize {
        self.completed
            .iter()
            .filter_map(|tile| tile.as_ref())
            .map(|tile| tile.width as usize * tile.height as usize * 4)
            .sum()
    }
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

fn tile_priority_key(tile: RenderTile, hint: RenderTile) -> (u8, u64, u32, u32) {
    let tile_x1 = tile.x.saturating_add(tile.width);
    let tile_y1 = tile.y.saturating_add(tile.height);
    let hint_x1 = hint.x.saturating_add(hint.width);
    let hint_y1 = hint.y.saturating_add(hint.height);
    let intersects = tile.x < hint_x1 && hint.x < tile_x1 && tile.y < hint_y1 && hint.y < tile_y1;
    let tile_cx = u64::from(tile.x) * 2 + u64::from(tile.width);
    let tile_cy = u64::from(tile.y) * 2 + u64::from(tile.height);
    let hint_cx = u64::from(hint.x) * 2 + u64::from(hint.width);
    let hint_cy = u64::from(hint.y) * 2 + u64::from(hint.height);
    let dx = tile_cx.abs_diff(hint_cx);
    let dy = tile_cy.abs_diff(hint_cy);
    (
        if intersects { 0 } else { 1 },
        dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)),
        tile.y,
        tile.x,
    )
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
        assert_eq!(job.token().tile_width, job.tile_width);
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
}
