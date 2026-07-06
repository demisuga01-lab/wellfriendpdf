use serde::Serialize;

use crate::cancel::CancelToken;
use crate::engine::ContentEngine;
use crate::error::Result;
use crate::optional_content::OptionalContentContext;
use crate::render::{PageRenderer, PixelBuffer, RenderMode, RenderTile, WHITE};

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveRenderToken {
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
    pub resumable: bool,
    pub complete: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProgressiveRenderStepReport {
    pub phase: String,
    pub completed_units: usize,
    pub total_units: usize,
    pub rendered_this_step: usize,
    pub next_tile_index: usize,
    pub cancelled: bool,
    pub resume_possible: bool,
    pub memory_bytes_retained: usize,
    pub visibility_fingerprint: String,
}

pub struct ProgressiveRenderJob<'a> {
    engine: &'a ContentEngine,
    page_number: usize,
    dpi: u32,
    render_mode: RenderMode,
    tile_width: u32,
    tile_height: u32,
    page_width: u32,
    page_height: u32,
    tiles: Vec<RenderTile>,
    completed: Vec<Option<PixelBuffer>>,
    next_tile_index: usize,
    visibility_fingerprint: String,
    aborted: bool,
}

impl<'a> ProgressiveRenderJob<'a> {
    pub fn new(
        engine: &'a ContentEngine,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile_width: u32,
        tile_height: u32,
    ) -> Result<Self> {
        let viewport = engine.page_viewport(page_number, dpi)?;
        let tile_width = tile_width.max(1);
        let tile_height = tile_height.max(1);
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
            completed: vec![None; total],
            next_tile_index: 0,
            visibility_fingerprint: OptionalContentContext::from_document(engine.document())
                .visibility_fingerprint()
                .to_string(),
            aborted: false,
        })
    }

    pub fn token(&self) -> ProgressiveRenderToken {
        ProgressiveRenderToken {
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
            resumable: !self.aborted,
            complete: self.is_complete(),
        }
    }

    pub fn render_next(
        &mut self,
        max_tiles: usize,
        cancel: &CancelToken,
    ) -> Result<ProgressiveRenderStepReport> {
        let max_tiles = max_tiles.max(1);
        let mut rendered = 0;
        let mut cancelled = false;
        while self.next_tile_index < self.tiles.len() && rendered < max_tiles {
            if cancel.is_cancelled() {
                cancelled = true;
                break;
            }
            let index = self.next_tile_index;
            let tile = self.tiles[index];
            let buffer = PageRenderer::render_page_tile_with_mode(
                self.engine,
                self.page_number,
                self.dpi,
                tile,
                self.render_mode,
                None,
            )?;
            self.completed[index] = Some(buffer);
            self.next_tile_index += 1;
            rendered += 1;
        }
        Ok(ProgressiveRenderStepReport {
            phase: if self.is_complete() {
                "complete".to_string()
            } else if cancelled {
                "cancelled_resumable".to_string()
            } else {
                "rendering_tiles".to_string()
            },
            completed_units: self.completed_count(),
            total_units: self.tiles.len(),
            rendered_this_step: rendered,
            next_tile_index: self.next_tile_index,
            cancelled,
            resume_possible: !self.aborted,
            memory_bytes_retained: self.memory_bytes_retained(),
            visibility_fingerprint: self.visibility_fingerprint.clone(),
        })
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
            for y in 0..tile.height {
                for x in 0..tile.width {
                    let pixel = buffer.get_pixel(x as i32, y as i32);
                    out.set_pixel((tile.x + x) as i32, (tile.y + y) as i32, pixel);
                }
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
