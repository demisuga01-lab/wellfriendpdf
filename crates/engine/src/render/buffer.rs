use crate::content::BlendMode;
use crate::images::decoder::RawImage;
use crate::render::cmm;
use crate::render::path::{FillRule, FlatPath};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, OnceLock,
};

/// Gamma-correct compositing helpers.
///
/// Antialiasing and alpha compositing are physically a mixing of *light*, and
/// light adds linearly â€” but 8-bit sRGB pixel values are **gamma-encoded**, so
/// blending them directly (the common shortcut, and what Poppler's Splash
/// backend does) mixes in the wrong space. The visible symptom is that
/// antialiased edges (especially dark text on a light background) come out too
/// dark, producing a "halo"/over-bold look. Converting sRGB â†’ linear, mixing
/// there, and converting back yields edges and transparency that are
/// measurably closer to ground truth.
///
/// The conversions use 8-bit â†’ f32 lookup tables (decode) and a 4096-entry
/// linear â†’ 8-bit table (encode), so the hot path is two table lookups per
/// channel with no `powf` calls.
#[allow(dead_code)]
pub(crate) mod gamma {
    use std::sync::OnceLock;

    fn srgb_to_linear_table() -> &'static [f32; 256] {
        static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
        TABLE.get_or_init(|| {
            let mut t = [0.0f32; 256];
            for (i, slot) in t.iter_mut().enumerate() {
                let c = i as f32 / 255.0;
                *slot = if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                };
            }
            t
        })
    }

    const ENC_SIZE: usize = 4096;

    fn linear_to_srgb_table() -> &'static [u8; ENC_SIZE] {
        static TABLE: OnceLock<[u8; ENC_SIZE]> = OnceLock::new();
        TABLE.get_or_init(|| {
            let mut t = [0u8; ENC_SIZE];
            for (i, slot) in t.iter_mut().enumerate() {
                let lin = i as f32 / (ENC_SIZE as f32 - 1.0);
                let s = if lin <= 0.003_130_8 {
                    lin * 12.92
                } else {
                    1.055 * lin.powf(1.0 / 2.4) - 0.055
                };
                *slot = (s * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            t
        })
    }

    /// Decode an 8-bit sRGB component to linear light.
    #[inline]
    pub fn to_linear(byte: u8) -> f32 {
        srgb_to_linear_table()[byte as usize]
    }

    /// Encode a linear-light value in [0, 1] back to an 8-bit sRGB component.
    #[inline]
    pub fn to_srgb(linear: f32) -> u8 {
        let idx = (linear.clamp(0.0, 1.0) * (ENC_SIZE as f32 - 1.0)).round() as usize;
        linear_to_srgb_table()[idx.min(ENC_SIZE - 1)]
    }

    /// Decode a normalised sRGB component in [0, 1] to linear light (the exact
    /// analytic transfer function â€” used where the value is already an f32, as
    /// in [`crate::render::color::RenderColor`]).
    #[inline]
    pub fn to_linear_f32(c: f32) -> f32 {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// Encode a linear-light value in [0, 1] back to a normalised sRGB f32.
    #[inline]
    pub fn to_srgb_f32(lin: f32) -> f32 {
        let lin = lin.clamp(0.0, 1.0);
        if lin <= 0.003_130_8 {
            lin * 12.92
        } else {
            1.055 * lin.powf(1.0 / 2.4) - 0.055
        }
    }
}

/// RGBA color: [R, G, B, A] each 0-255.
pub type PixelColor = [u8; 4];

pub const BLACK: PixelColor = [0, 0, 0, 255];
pub const WHITE: PixelColor = [255, 255, 255, 255];
pub const TRANSPARENT: PixelColor = [0, 0, 0, 0];
pub const RED: PixelColor = [255, 0, 0, 255];
pub const GREEN: PixelColor = [0, 255, 0, 255];
pub const BLUE: PixelColor = [0, 0, 255, 255];

/// Raster compositing mode.
///
/// `Compat` is the default Poppler/Splash-compatible path: antialiased coverage
/// and transparency are composited directly in sRGB byte space. `HighQuality`
/// keeps the same geometry and AA coverage but performs RGB compositing in
/// linear light for opt-in display fidelity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    #[default]
    Compat,
    HighQuality,
}

impl RenderMode {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "compat" | "compatible" | "poppler" | "proof" => Some(Self::Compat),
            "high" | "high-quality" | "highquality" | "hq" => Some(Self::HighQuality),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compat => "compat",
            Self::HighQuality => "high",
        }
    }

    #[inline]
    pub fn is_high_quality(self) -> bool {
        matches!(self, Self::HighQuality)
    }
}

/// Create a PixelColor with full alpha.
pub fn rgb(r: u8, g: u8, b: u8) -> PixelColor {
    [r, g, b, 255]
}

/// Create a PixelColor with specified alpha.
pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> PixelColor {
    [r, g, b, a]
}

#[derive(Debug, Clone)]
pub struct ClipMask {
    pub width: u32,
    pub height: u32,
    mask: Vec<u8>,
    solid: Option<bool>,
    partial_coverage: bool,
    run_cache: Arc<OnceLock<ClipRunCache>>,
}

#[derive(Debug, Clone, Default)]
struct ClipRunCache {
    row_offsets: Vec<usize>,
    runs: Vec<(i32, i32)>,
}

impl ClipRunCache {
    fn from_rows(rows: Vec<Vec<(i32, i32)>>) -> Self {
        let mut row_offsets = Vec::with_capacity(rows.len().saturating_add(1));
        let run_count = rows.iter().map(Vec::len).sum();
        let mut runs = Vec::with_capacity(run_count);
        row_offsets.push(0);
        for row in rows {
            runs.extend(row);
            row_offsets.push(runs.len());
        }
        Self { row_offsets, runs }
    }

    fn full_visible(width: u32, height: u32) -> Self {
        Self::from_rows(vec![vec![(0, width as i32)]; height as usize])
    }

    fn empty(height: u32) -> Self {
        let row_offsets = vec![0usize; height as usize + 1];
        Self {
            row_offsets,
            runs: Vec::new(),
        }
    }

    fn row(&self, y: usize) -> &[(i32, i32)] {
        let Some((&start, &end)) = self.row_offsets.get(y).zip(self.row_offsets.get(y + 1)) else {
            return &[];
        };
        self.runs.get(start..end).unwrap_or(&[])
    }

    fn to_rows(&self, height: u32) -> Vec<Vec<(i32, i32)>> {
        (0..height as usize).map(|y| self.row(y).to_vec()).collect()
    }
}

impl ClipMask {
    /// All-visible mask: every pixel is inside the clip.
    pub fn all_visible(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            // Solid masks are represented structurally instead of allocating a
            // full page-sized byte plane. They are materialized only when a
            // later operation needs per-pixel mutation or partial coverage.
            mask: Vec::new(),
            solid: Some(true),
            partial_coverage: false,
            run_cache: Arc::new(OnceLock::new()),
        }
    }

    /// All-clipped mask: no in-bounds pixel is visible.
    pub fn empty(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            // See `all_visible`: solid clips do not allocate a dense mask.
            mask: Vec::new(),
            solid: Some(false),
            partial_coverage: false,
            run_cache: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn from_visible_runs(
        width: u32,
        height: u32,
        mut rows: Vec<Vec<(i32, i32)>>,
    ) -> Self {
        rows.resize_with(height as usize, Vec::new);
        rows.truncate(height as usize);

        let mut saw_visible = false;
        let mut saw_clipped = false;
        let width_i32 = width as i32;
        for row in &mut rows {
            row.sort_unstable_by_key(|(start, end)| (*start, *end));
            let mut merged: Vec<(i32, i32)> = Vec::new();
            for (start, end) in row.drain(..) {
                let start = start.max(0).min(width_i32);
                let end = end.max(0).min(width_i32);
                if end <= start {
                    continue;
                }
                if let Some((_, last_end)) = merged.last_mut() {
                    if start <= *last_end {
                        *last_end = (*last_end).max(end);
                    } else {
                        merged.push((start, end));
                    }
                } else {
                    merged.push((start, end));
                }
            }
            let visible_width: i32 = merged.iter().map(|(start, end)| end - start).sum();
            saw_visible |= visible_width > 0;
            saw_clipped |= visible_width < width_i32;
            *row = merged;
        }

        if !saw_visible {
            return Self::empty(width, height);
        }
        if !saw_clipped {
            return Self::all_visible(width, height);
        }

        let lock = OnceLock::new();
        let _ = lock.set(ClipRunCache::from_rows(rows));
        Self {
            width,
            height,
            mask: Vec::new(),
            solid: None,
            partial_coverage: false,
            run_cache: Arc::new(lock),
        }
    }

    pub(crate) fn from_visible_rect(
        width: u32,
        height: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Self {
        if width == 0 || height == 0 || w <= 0 || h <= 0 {
            return Self::empty(width, height);
        }
        let x0 = x.max(0).min(width as i32);
        let y0 = y.max(0).min(height as i32);
        let x1 = x.saturating_add(w).max(0).min(width as i32);
        let y1 = y.saturating_add(h).max(0).min(height as i32);
        if x1 <= x0 || y1 <= y0 {
            return Self::empty(width, height);
        }
        if x0 == 0 && y0 == 0 && x1 == width as i32 && y1 == height as i32 {
            return Self::all_visible(width, height);
        }
        let mut rows = vec![Vec::new(); height as usize];
        for row in y0..y1 {
            rows[row as usize].push((x0, x1));
        }
        Self::from_visible_runs(width, height, rows)
    }

    fn invalidate_run_cache(&mut self) {
        self.run_cache = Arc::new(OnceLock::new());
    }

    fn dense_len(&self) -> usize {
        (self.width as usize)
            .checked_mul(self.height as usize)
            .unwrap_or(0)
    }

    fn materialize_dense_mask(&mut self) {
        let len = self.dense_len();
        if self.mask.len() == len {
            return;
        }
        let value = match self.solid {
            Some(true) => 255,
            Some(false) | None => 0,
        };
        self.mask = vec![value; len];
        if self.solid.is_none() {
            let width = self.width as usize;
            if let Some(runs) = self.run_cache.get() {
                for y in 0..self.height as usize {
                    let row_start = y.saturating_mul(width);
                    for (start, end) in runs.row(y) {
                        let start = (*start).max(0).min(self.width as i32) as usize;
                        let end = (*end).max(0).min(self.width as i32) as usize;
                        if end <= start {
                            continue;
                        }
                        let start_idx = row_start.saturating_add(start);
                        let end_idx = row_start.saturating_add(end);
                        if let Some(slice) = self.mask.get_mut(start_idx..end_idx) {
                            slice.fill(255);
                        }
                    }
                }
            }
        }
    }

    #[inline]
    pub fn is_all_visible(&self) -> bool {
        self.solid == Some(true)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.solid == Some(false)
    }

    /// Query whether pixel (x, y) is inside the clip.
    #[inline]
    pub fn is_visible(&self, x: i32, y: i32) -> bool {
        self.opacity_byte(x, y) > 0
    }

    #[inline]
    pub(crate) fn opacity_byte(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 255;
        }
        if let Some(solid) = self.solid {
            return if solid { 255 } else { 0 };
        }
        let idx = match (y as usize)
            .checked_mul(self.width as usize)
            .and_then(|row| row.checked_add(x as usize))
        {
            Some(idx) => idx,
            None => return 255,
        };
        if let Some(value) = self.mask.get(idx).copied() {
            return value;
        }
        let row_runs = self.compressed_runs().row(y as usize);
        if row_runs.iter().any(|(start, end)| x >= *start && x < *end) {
            255
        } else {
            0
        }
    }

    #[inline]
    pub(crate) fn opacity(&self, x: i32, y: i32) -> f32 {
        f32::from(self.opacity_byte(x, y)) / 255.0
    }

    #[inline]
    pub(crate) fn has_partial_coverage(&self) -> bool {
        self.partial_coverage
    }

    fn compressed_runs(&self) -> &ClipRunCache {
        self.run_cache.get_or_init(|| {
            let mut rows = Vec::with_capacity(self.height as usize);
            match self.solid {
                Some(true) => {
                    return ClipRunCache::full_visible(self.width, self.height);
                }
                Some(false) => {
                    return ClipRunCache::empty(self.height);
                }
                None => {}
            }
            for y in 0..self.height as usize {
                let row_start = y.saturating_mul(self.width as usize);
                let row_end = row_start.saturating_add(self.width as usize);
                let Some(mask_row) = self.mask.get(row_start..row_end) else {
                    rows.push(Vec::new());
                    continue;
                };
                let mut row_runs = Vec::new();
                let mut run_start: Option<i32> = None;
                for (x, value) in mask_row.iter().enumerate() {
                    let x = x as i32;
                    if *value > 0 {
                        if run_start.is_none() {
                            run_start = Some(x);
                        }
                    } else if let Some(start) = run_start.take() {
                        row_runs.push((start, x));
                    }
                }
                if let Some(start) = run_start {
                    row_runs.push((start, self.width as i32));
                }
                rows.push(row_runs);
            }
            ClipRunCache::from_rows(rows)
        })
    }

    fn binary_run_cache(&self) -> Option<&ClipRunCache> {
        if self.partial_coverage {
            return None;
        }
        Some(self.compressed_runs())
    }

    fn intersect_run_caches(
        lhs: &ClipRunCache,
        rhs: &ClipRunCache,
        width: u32,
        height: u32,
    ) -> Self {
        let mut rows = vec![Vec::new(); height as usize];
        for (y, row) in rows.iter_mut().enumerate() {
            let left = lhs.row(y);
            let right = rhs.row(y);
            let mut li = 0usize;
            let mut ri = 0usize;
            while li < left.len() && ri < right.len() {
                let (ls, le) = left[li];
                let (rs, re) = right[ri];
                let start = ls.max(rs);
                let end = le.min(re);
                if end > start {
                    row.push((start, end));
                }
                if le < re {
                    li += 1;
                } else {
                    ri += 1;
                }
            }
        }
        Self::from_visible_runs(width, height, rows)
    }

    fn union_run_caches(lhs: &ClipRunCache, rhs: &ClipRunCache, width: u32, height: u32) -> Self {
        let mut rows = vec![Vec::new(); height as usize];
        for (y, row) in rows.iter_mut().enumerate() {
            row.extend(lhs.row(y).iter().copied());
            row.extend(rhs.row(y).iter().copied());
        }
        Self::from_visible_runs(width, height, rows)
    }

    fn intersect_dense_with_binary_runs(&mut self, runs: &ClipRunCache) {
        self.materialize_dense_mask();
        let width = self.width as usize;
        for y in 0..self.height as usize {
            let row_start = y.saturating_mul(width);
            let row_end = row_start.saturating_add(width);
            let Some(row) = self.mask.get_mut(row_start..row_end) else {
                continue;
            };
            let mut cursor = 0usize;
            for (start, end) in runs.row(y) {
                let start = (*start).max(0).min(self.width as i32) as usize;
                let end = (*end).max(0).min(self.width as i32) as usize;
                if end <= start {
                    continue;
                }
                if cursor < start {
                    row[cursor..start].fill(0);
                }
                cursor = cursor.max(end);
            }
            if cursor < width {
                row[cursor..width].fill(0);
            }
        }
        self.refresh_solid_hint();
    }

    fn union_dense_with_binary_runs(&mut self, runs: &ClipRunCache) {
        self.materialize_dense_mask();
        let width = self.width as usize;
        for y in 0..self.height as usize {
            let row_start = y.saturating_mul(width);
            let row_end = row_start.saturating_add(width);
            let Some(row) = self.mask.get_mut(row_start..row_end) else {
                continue;
            };
            for (start, end) in runs.row(y) {
                let start = (*start).max(0).min(self.width as i32) as usize;
                let end = (*end).max(0).min(self.width as i32) as usize;
                if end > start {
                    row[start..end].fill(255);
                }
            }
        }
        self.refresh_solid_hint();
    }

    fn apply_rect_to_binary_runs(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, visible: bool) {
        let mut rows = self.compressed_runs().to_rows(self.height);
        rows.resize_with(self.height as usize, Vec::new);
        for row in y0..y1 {
            let row_runs = &mut rows[row as usize];
            if visible {
                row_runs.push((x0, x1));
                continue;
            }
            let mut replacement = Vec::with_capacity(row_runs.len().saturating_add(1));
            for (start, end) in row_runs.drain(..) {
                if end <= x0 || start >= x1 {
                    replacement.push((start, end));
                    continue;
                }
                if start < x0 {
                    replacement.push((start, x0));
                }
                if x1 < end {
                    replacement.push((x1, end));
                }
            }
            *row_runs = replacement;
        }
        *self = Self::from_visible_runs(self.width, self.height, rows);
    }

    /// Return the minimal exclusive pixel bounds that contain all visible
    /// samples. `None` means no pixel is visible. The result is conservative for
    /// antialiased masks because any non-zero coverage is included.
    pub(crate) fn visible_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        match self.solid {
            Some(true) => Some((0, 0, self.width as i32, self.height as i32)),
            Some(false) => None,
            None => {
                let mut x_min = self.width as i32;
                let mut y_min = self.height as i32;
                let mut x_max = 0i32;
                let mut y_max = 0i32;
                let runs = self.compressed_runs();
                for y in 0..self.height as usize {
                    let y = y as i32;
                    for (start, end) in runs.row(y as usize) {
                        if end <= start {
                            continue;
                        }
                        x_min = x_min.min(*start);
                        y_min = y_min.min(y);
                        x_max = x_max.max(*end);
                        y_max = y_max.max(y + 1);
                    }
                }
                if x_max <= x_min || y_max <= y_min {
                    None
                } else {
                    Some((x_min, y_min, x_max, y_max))
                }
            }
        }
    }

    /// Copy a rectangular clip window into a new mask using row slices.
    ///
    /// The copied mask uses destination-local coordinates. It is used by
    /// tile-local transparency/Form rendering so off-screen groups do not keep
    /// full-page clip buffers alive when only a bounded pixel window can be
    /// affected.
    pub(crate) fn copy_rect_to_new_mask(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let end_x = x.checked_add(width)?;
        let end_y = y.checked_add(height)?;
        if end_x > self.width || end_y > self.height {
            return None;
        }
        match self.solid {
            Some(true) => return Some(Self::all_visible(width, height)),
            Some(false) => return Some(Self::empty(width, height)),
            None => {}
        }

        if !self.partial_coverage {
            let mut rows = vec![Vec::new(); height as usize];
            let src_x0 = x as i32;
            let src_x1 = end_x as i32;
            for dst_y in 0..height as i32 {
                let src_y = y as i32 + dst_y;
                self.for_each_visible_run_in_span(src_y, src_x0, src_x1, |start, end| {
                    rows[dst_y as usize].push((start - src_x0, end - src_x0));
                });
            }
            return Some(Self::from_visible_runs(width, height, rows));
        }

        let mut out = Self::empty(width, height);
        out.materialize_dense_mask();
        let src_stride = self.width as usize;
        let dst_stride = width as usize;
        let src_x = x as usize;
        let mut all_visible = true;
        let mut all_empty = true;
        let mut partial_coverage = false;
        for row in 0..height as usize {
            let src_start = (y as usize + row)
                .checked_mul(src_stride)?
                .checked_add(src_x)?;
            let src_end = src_start.checked_add(dst_stride)?;
            let dst_start = row.checked_mul(dst_stride)?;
            let dst_end = dst_start.checked_add(dst_stride)?;
            let src = self.mask.get(src_start..src_end)?;
            let dst = out.mask.get_mut(dst_start..dst_end)?;
            dst.copy_from_slice(src);
            for value in src {
                all_visible &= *value == 255;
                all_empty &= *value == 0;
                partial_coverage |= *value != 0 && *value != 255;
            }
        }
        out.solid = if all_visible {
            Some(true)
        } else if all_empty {
            Some(false)
        } else {
            None
        };
        if out.solid.is_some() {
            out.mask.clear();
        }
        out.partial_coverage = out.solid.is_none() && partial_coverage;
        out.invalidate_run_cache();
        Some(out)
    }

    pub(crate) fn for_each_visible_run(&self, y: i32, max_width: i32, visit: impl FnMut(i32, i32)) {
        self.for_each_visible_run_in_span(y, 0, max_width, visit);
    }

    pub(crate) fn for_each_visible_run_in_span(
        &self,
        y: i32,
        x_start: i32,
        x_end_exclusive: i32,
        mut visit: impl FnMut(i32, i32),
    ) {
        if y < 0 || y >= self.height as i32 || x_end_exclusive <= x_start {
            return;
        }
        let x0 = x_start.max(0).min(self.width as i32);
        let x1 = x_end_exclusive.max(0).min(self.width as i32);
        if x1 <= x0 {
            return;
        }
        match self.solid {
            Some(true) => {
                visit(x0, x1);
            }
            Some(false) => {}
            None => {
                for (run_start, run_end) in self.compressed_runs().row(y as usize) {
                    let start = (*run_start).max(x0);
                    let end = (*run_end).min(x1);
                    if end > start {
                        visit(start, end);
                    }
                }
            }
        }
    }

    /// Set pixel (x, y) to visible or clipped.
    pub fn set(&mut self, x: i32, y: i32, visible: bool) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let Some(idx) = (y as usize)
            .checked_mul(self.width as usize)
            .and_then(|row| row.checked_add(x as usize))
        else {
            return;
        };
        let mut changed = false;
        self.materialize_dense_mask();
        if let Some(value) = self.mask.get_mut(idx) {
            if *value != if visible { 255 } else { 0 } {
                self.solid = None;
                changed = true;
            }
            *value = if visible { 255 } else { 0 };
        }
        if changed {
            self.invalidate_run_cache();
        }
    }

    /// Intersect this mask with another mask.
    pub fn intersect(&mut self, other: &ClipMask) {
        if self.width != other.width || self.height != other.height {
            log::warn!(
                "ClipMask::intersect size mismatch: {}x{} vs {}x{}",
                self.width,
                self.height,
                other.width,
                other.height
            );
            return;
        }
        if other.is_all_visible() || self.is_empty() {
            return;
        }
        if self.is_all_visible() {
            *self = other.clone();
            return;
        }
        if other.is_empty() {
            self.mask.clear();
            self.solid = Some(false);
            self.partial_coverage = false;
            self.invalidate_run_cache();
            return;
        }
        if let (Some(lhs), Some(rhs)) = (self.binary_run_cache(), other.binary_run_cache()) {
            *self = Self::intersect_run_caches(lhs, rhs, self.width, self.height);
            return;
        }
        if let Some(rhs) = other.binary_run_cache() {
            self.intersect_dense_with_binary_runs(rhs);
            return;
        }
        if let Some(lhs) = self.binary_run_cache() {
            let lhs = lhs.clone();
            let mut merged = other.clone();
            merged.intersect_dense_with_binary_runs(&lhs);
            *self = merged;
            return;
        }
        self.materialize_dense_mask();
        let mut all_visible = true;
        let mut all_empty = true;
        let mut partial_coverage = false;
        let width = self.width as usize;
        for (idx, a) in self.mask.iter_mut().enumerate() {
            let y = idx / width;
            let x = idx - y * width;
            let b = other.opacity_byte(x as i32, y as i32);
            *a = (*a).min(b);
            all_visible &= *a == 255;
            all_empty &= *a == 0;
            partial_coverage |= *a != 0 && *a != 255;
        }
        self.solid = if all_visible {
            Some(true)
        } else if all_empty {
            Some(false)
        } else {
            None
        };
        if self.solid.is_some() {
            self.mask.clear();
        }
        self.partial_coverage = self.solid.is_none() && partial_coverage;
        self.invalidate_run_cache();
    }

    /// Union this mask with another mask.
    pub fn union_with(&mut self, other: &ClipMask) {
        if self.width != other.width || self.height != other.height {
            log::warn!(
                "ClipMask::union_with size mismatch: {}x{} vs {}x{}",
                self.width,
                self.height,
                other.width,
                other.height
            );
            return;
        }
        if other.is_empty() || self.is_all_visible() {
            return;
        }
        if self.is_empty() {
            *self = other.clone();
            return;
        }
        if other.is_all_visible() {
            self.mask.clear();
            self.solid = Some(true);
            self.partial_coverage = false;
            self.invalidate_run_cache();
            return;
        }
        if let (Some(lhs), Some(rhs)) = (self.binary_run_cache(), other.binary_run_cache()) {
            *self = Self::union_run_caches(lhs, rhs, self.width, self.height);
            return;
        }
        if let Some(rhs) = other.binary_run_cache() {
            self.union_dense_with_binary_runs(rhs);
            return;
        }
        if let Some(lhs) = self.binary_run_cache() {
            let lhs = lhs.clone();
            let mut merged = other.clone();
            merged.union_dense_with_binary_runs(&lhs);
            *self = merged;
            return;
        }
        self.materialize_dense_mask();
        let mut all_visible = true;
        let mut all_empty = true;
        let mut partial_coverage = false;
        let width = self.width as usize;
        for (idx, a) in self.mask.iter_mut().enumerate() {
            let y = idx / width;
            let x = idx - y * width;
            let b = other.opacity_byte(x as i32, y as i32);
            *a = (*a).max(b);
            all_visible &= *a == 255;
            all_empty &= *a == 0;
            partial_coverage |= *a != 0 && *a != 255;
        }
        self.solid = if all_visible {
            Some(true)
        } else if all_empty {
            Some(false)
        } else {
            None
        };
        if self.solid.is_some() {
            self.mask.clear();
        }
        self.partial_coverage = self.solid.is_none() && partial_coverage;
        self.invalidate_run_cache();
    }

    pub(crate) fn union_alpha_mask(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        alpha: &[u8],
    ) {
        if width == 0 || height == 0 || alpha.is_empty() || self.is_all_visible() {
            return;
        }
        let x0 = x.max(0).min(self.width as i32);
        let y0 = y.max(0).min(self.height as i32);
        let x1 = x.saturating_add(width as i32).max(0).min(self.width as i32);
        let y1 = y
            .saturating_add(height as i32)
            .max(0)
            .min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        if self.solid == Some(false) {
            self.solid = None;
        }
        self.materialize_dense_mask();
        let src_stride = width as usize;
        let dst_stride = self.width as usize;
        let src_x = (x0 - x) as usize;
        let src_y = (y0 - y) as usize;
        let span = (x1 - x0) as usize;
        let mut partial_coverage = self.partial_coverage;
        for row in 0..(y1 - y0) as usize {
            let src_start = (src_y + row)
                .saturating_mul(src_stride)
                .saturating_add(src_x);
            let dst_start = (y0 as usize + row)
                .saturating_mul(dst_stride)
                .saturating_add(x0 as usize);
            let Some(src) = alpha.get(src_start..src_start.saturating_add(span)) else {
                continue;
            };
            let Some(dst) = self.mask.get_mut(dst_start..dst_start.saturating_add(span)) else {
                continue;
            };
            for (d, a) in dst.iter_mut().zip(src.iter().copied()) {
                *d = (*d).max(a);
                partial_coverage |= a != 0 && a != 255;
            }
        }
        self.solid = None;
        self.partial_coverage = partial_coverage;
        self.invalidate_run_cache();
    }

    /// Fill a rectangular mask region with visible or clipped.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, visible: bool) {
        if w <= 0 || h <= 0 {
            return;
        }
        let value = if visible { 255u8 } else { 0u8 };
        let x0 = x.max(0).min(self.width as i32);
        let y0 = y.max(0).min(self.height as i32);
        let x1 = x.saturating_add(w).max(0).min(self.width as i32);
        let y1 = y.saturating_add(h).max(0).min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        if x0 == 0 && y0 == 0 && x1 == self.width as i32 && y1 == self.height as i32 {
            self.mask.clear();
            self.solid = Some(visible);
            self.partial_coverage = false;
            self.invalidate_run_cache();
            return;
        }

        if !self.partial_coverage {
            self.apply_rect_to_binary_runs(x0, y0, x1, y1, visible);
            return;
        }

        if self.solid != Some(visible) {
            self.solid = None;
        }
        self.materialize_dense_mask();
        for row in y0..y1 {
            let start = row as usize * self.width as usize + x0 as usize;
            let end = row as usize * self.width as usize + x1 as usize;
            if let Some(slice) = self.mask.get_mut(start..end) {
                slice.fill(value);
            }
        }
        self.invalidate_run_cache();
    }

    /// Build a ClipMask from a flattened path using scanline fill.
    pub fn from_path(flat: &FlatPath, width: u32, height: u32, fill_rule: FillRule) -> Self {
        Self::scanline_fill_antialiased(flat, width, height, fill_rule)
    }

    fn refresh_solid_hint(&mut self) {
        if self.mask.is_empty() {
            if self.solid.is_some() {
                self.partial_coverage = false;
                self.invalidate_run_cache();
            }
            return;
        }
        let mut all_visible = true;
        let mut all_empty = true;
        let mut partial_coverage = false;
        for value in &self.mask {
            all_visible &= *value == 255;
            all_empty &= *value == 0;
            partial_coverage |= *value != 0 && *value != 255;
        }
        if all_visible {
            self.solid = Some(true);
        } else if all_empty {
            self.solid = Some(false);
        } else {
            self.solid = None;
        }
        if self.solid.is_some() {
            self.mask.clear();
        }
        self.partial_coverage = self.solid.is_none() && partial_coverage;
        self.invalidate_run_cache();
    }

    fn scanline_fill_antialiased(flat: &FlatPath, width: u32, height: u32, rule: FillRule) -> Self {
        let mut clip = Self::empty(width, height);

        let mut edges = Vec::new();
        for subpath in &flat.subpaths {
            for segment in subpath.windows(2) {
                let (x0, y0) = segment[0];
                let (x1, y1) = segment[1];
                if (y0 - y1).abs() < 1e-10 {
                    continue;
                }
                let (x_start, y_start, x_end, y_end) = if y0 < y1 {
                    (x0, y0, x1, y1)
                } else {
                    (x1, y1, x0, y0)
                };
                let winding = if y0 < y1 { 1 } else { -1 };
                edges.push(ClipEdge {
                    y_min: y_start,
                    y_max: y_end,
                    x_at_ymin: x_start,
                    slope: (x_end - x_start) / (y_end - y_start),
                    winding,
                });
            }
        }

        if edges.is_empty() || width == 0 || height == 0 {
            return clip;
        }

        let y_min = edges
            .iter()
            .map(|edge| floor_i32(edge.y_min))
            .min()
            .unwrap_or(0)
            .max(0);
        let y_max = edges
            .iter()
            .map(|edge| ceil_i32(edge.y_max))
            .max()
            .unwrap_or(0)
            .min(height as i32 - 1);
        if y_max < y_min {
            return clip;
        }

        // Anti-aliased clipping is important for ordinary vector clips, but a
        // few real-world files contain extremely complex clipping paths that
        // are used only as broad containment masks. The 4x4 supersampled path
        // would re-sort and walk those edge sets for every sub-scanline, which
        // can turn a single page render into a multi-minute CPU sink. Keep the
        // exact same fill rule but fall back to a binary scanline mask once the
        // edge/scanline work estimate exceeds the bounded interactive path.
        const ANTIALIAS_WORK_LIMIT: usize = 2_000_000;
        let scanlines = (y_max - y_min + 1).max(0) as usize;
        if edges.len().saturating_mul(scanlines) > ANTIALIAS_WORK_LIMIT {
            return Self::scanline_fill_binary_edges(&edges, width, height, rule, y_min, y_max);
        }

        const SAMPLES: i32 = 4;
        const SAMPLE_COUNT: u16 = (SAMPLES * SAMPLES) as u16;
        let Some(total_pixels) = (width as usize).checked_mul(height as usize) else {
            return clip;
        };
        let mut coverage = vec![0u16; total_pixels];
        let mut intersections = Vec::<(f64, i32)>::with_capacity(edges.len().min(256));
        let mut spans = Vec::<(f64, f64)>::with_capacity(edges.len().saturating_div(2).min(128));

        for y in y_min..=y_max {
            for sub_y in 0..SAMPLES {
                let y_f = y as f64 + (sub_y as f64 + 0.5) / f64::from(SAMPLES);
                intersections.clear();
                intersections.extend(edges.iter().filter_map(|edge| {
                    if edge.y_min <= y_f && y_f < edge.y_max {
                        Some((
                            edge.x_at_ymin + edge.slope * (y_f - edge.y_min),
                            edge.winding,
                        ))
                    } else {
                        None
                    }
                }));
                intersections
                    .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                fill_spans_into(&intersections, rule, &mut spans);
                for &(x0, x1) in &spans {
                    if x1 <= x0 {
                        continue;
                    }
                    let k0 = ceil_i32(x0 * f64::from(SAMPLES) - 0.5).max(0);
                    let k1 = ceil_i32(x1 * f64::from(SAMPLES) - 0.5).min(width as i32 * SAMPLES);
                    for sample_x in k0..k1 {
                        let px = sample_x / SAMPLES;
                        if px < 0 || px >= width as i32 {
                            continue;
                        }
                        let Some(idx) = (y as usize)
                            .checked_mul(width as usize)
                            .and_then(|row| row.checked_add(px as usize))
                        else {
                            continue;
                        };
                        coverage[idx] = coverage[idx].saturating_add(1).min(SAMPLE_COUNT);
                    }
                }
            }
        }

        let mut all_visible = true;
        let mut all_empty = true;
        let mut partial_coverage = false;
        clip.materialize_dense_mask();
        for (dst, samples) in clip.mask.iter_mut().zip(coverage) {
            let value = ((u32::from(samples) * 255 + u32::from(SAMPLE_COUNT / 2))
                / u32::from(SAMPLE_COUNT))
            .min(255) as u8;
            *dst = value;
            all_visible &= value == 255;
            all_empty &= value == 0;
            partial_coverage |= value != 0 && value != 255;
        }
        if all_visible {
            return Self::all_visible(width, height);
        }
        if all_empty {
            return Self::empty(width, height);
        }
        if !partial_coverage {
            let mut rows = Vec::with_capacity(height as usize);
            for y in 0..height as usize {
                let row_start = y.saturating_mul(width as usize);
                let row_end = row_start.saturating_add(width as usize);
                let Some(mask_row) = clip.mask.get(row_start..row_end) else {
                    rows.push(Vec::new());
                    continue;
                };
                let mut row_runs = Vec::new();
                let mut run_start: Option<i32> = None;
                for (x, value) in mask_row.iter().enumerate() {
                    let x = x as i32;
                    if *value > 0 {
                        if run_start.is_none() {
                            run_start = Some(x);
                        }
                    } else if let Some(start) = run_start.take() {
                        row_runs.push((start, x));
                    }
                }
                if let Some(start) = run_start {
                    row_runs.push((start, width as i32));
                }
                rows.push(row_runs);
            }
            return Self::from_visible_runs(width, height, rows);
        }
        clip.solid = None;
        clip.partial_coverage = partial_coverage;

        clip
    }

    fn scanline_fill_binary_edges(
        edges: &[ClipEdge],
        width: u32,
        height: u32,
        rule: FillRule,
        y_min: i32,
        y_max: i32,
    ) -> Self {
        if edges.is_empty() || width == 0 || height == 0 || y_max < y_min {
            return Self::empty(width, height);
        }
        let row_count = (y_max - y_min + 1) as usize;
        let mut start_counts = vec![0usize; row_count];
        let mut start_rows = Vec::<(usize, usize)>::with_capacity(edges.len());
        let mut rows = vec![Vec::<(i32, i32)>::new(); height as usize];
        for (idx, edge) in edges.iter().enumerate() {
            let start = floor_i32(edge.y_min).max(y_min);
            let end = ceil_i32(edge.y_max).min(y_max);
            if end < y_min || start > y_max {
                continue;
            }
            let row = (start - y_min) as usize;
            if let Some(count) = start_counts.get_mut(row) {
                *count = count.saturating_add(1);
                start_rows.push((idx, row));
            }
        }
        let mut start_offsets = vec![0usize; row_count + 1];
        for (idx, count) in start_counts.iter().copied().enumerate() {
            start_offsets[idx + 1] = start_offsets[idx].saturating_add(count);
        }
        let mut flat_starts = vec![0usize; start_rows.len()];
        let mut cursor = start_offsets[..row_count].to_vec();
        for (edge_idx, row) in start_rows {
            let Some(slot) = cursor.get_mut(row) else {
                continue;
            };
            if let Some(dst) = flat_starts.get_mut(*slot) {
                *dst = edge_idx;
            }
            *slot = slot.saturating_add(1);
        }
        let mut active: Vec<usize> = Vec::new();
        let mut intersections = Vec::<(f64, i32)>::with_capacity(edges.len().min(256));
        let mut spans = Vec::<(f64, f64)>::with_capacity(edges.len().saturating_div(2).min(128));
        for y in y_min..=y_max {
            let y_f = y as f64 + 0.5;
            let row = (y - y_min) as usize;
            if let (Some(start), Some(end)) = (start_offsets.get(row), start_offsets.get(row + 1)) {
                if let Some(new_edges) = flat_starts.get(*start..*end) {
                    active.extend(new_edges.iter().copied());
                }
            }
            active.retain(|idx| {
                let edge = &edges[*idx];
                edge.y_min <= y_f && y_f < edge.y_max
            });
            intersections.clear();
            intersections.extend(active.iter().map(|idx| {
                let edge = &edges[*idx];
                (
                    edge.x_at_ymin + edge.slope * (y_f - edge.y_min),
                    edge.winding,
                )
            }));
            intersections
                .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            fill_spans_into(&intersections, rule, &mut spans);
            for &(x0, x1) in &spans {
                if x1 <= x0 {
                    continue;
                }
                let x0 = ceil_i32(x0).max(0).min(width as i32);
                let x1 = ceil_i32(x1).max(0).min(width as i32);
                if x1 > x0 {
                    rows[y as usize].push((x0, x1));
                }
            }
        }
        Self::from_visible_runs(width, height, rows)
    }
}

#[derive(Debug, Clone)]
pub struct AlphaMask {
    pub width: u32,
    pub height: u32,
    origin_x: i32,
    origin_y: i32,
    outside_alpha: u8,
    data: Vec<u8>,
}

impl AlphaMask {
    pub fn filled(width: u32, height: u32, alpha: u8) -> Self {
        let len = (width as usize).checked_mul(height as usize).unwrap_or(0);
        Self {
            width,
            height,
            origin_x: 0,
            origin_y: 0,
            outside_alpha: 255,
            data: vec![alpha; len],
        }
    }

    pub fn all_opaque(width: u32, height: u32) -> Self {
        Self::filled(width, height, 255)
    }

    pub(crate) fn approximate_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.data.len()
    }

    pub fn with_origin_and_outside_alpha(
        mut self,
        origin_x: i32,
        origin_y: i32,
        outside_alpha: u8,
    ) -> Self {
        self.origin_x = origin_x;
        self.origin_y = origin_y;
        self.outside_alpha = outside_alpha;
        self
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        let local_x = x.saturating_sub(self.origin_x);
        let local_y = y.saturating_sub(self.origin_y);
        if local_x < 0
            || local_y < 0
            || local_x >= self.width as i32
            || local_y >= self.height as i32
        {
            return self.outside_alpha as f32 / 255.0;
        }
        let Some(idx) = (local_y as usize)
            .checked_mul(self.width as usize)
            .and_then(|row| row.checked_add(local_x as usize))
        else {
            return self.outside_alpha as f32 / 255.0;
        };
        self.data.get(idx).copied().unwrap_or(self.outside_alpha) as f32 / 255.0
    }

    pub fn set(&mut self, x: i32, y: i32, alpha: u8) {
        let local_x = x.saturating_sub(self.origin_x);
        let local_y = y.saturating_sub(self.origin_y);
        if local_x < 0
            || local_y < 0
            || local_x >= self.width as i32
            || local_y >= self.height as i32
        {
            return;
        }
        let Some(idx) = (local_y as usize)
            .checked_mul(self.width as usize)
            .and_then(|row| row.checked_add(local_x as usize))
        else {
            return;
        };
        if let Some(value) = self.data.get_mut(idx) {
            *value = alpha;
        }
    }

    pub fn paste_from(&mut self, src: &AlphaMask, dst_x: i32, dst_y: i32) {
        for row in 0..src.height as i32 {
            let y = dst_y.saturating_add(row);
            if y < 0 || y >= self.height as i32 {
                continue;
            }
            let x0 = dst_x.max(0);
            let x1 = dst_x
                .saturating_add(src.width as i32)
                .min(self.width as i32);
            if x1 <= x0 {
                continue;
            }
            let src_x = (x0 - dst_x) as usize;
            let width = (x1 - x0) as usize;
            let Some(src_start) = (row as usize)
                .checked_mul(src.width as usize)
                .and_then(|base| base.checked_add(src_x))
            else {
                continue;
            };
            let Some(dst_start) = (y as usize)
                .checked_mul(self.width as usize)
                .and_then(|base| base.checked_add(x0 as usize))
            else {
                continue;
            };
            let Some(src_row) = src.data.get(src_start..src_start.saturating_add(width)) else {
                continue;
            };
            let Some(dst_row) = self
                .data
                .get_mut(dst_start..dst_start.saturating_add(width))
            else {
                continue;
            };
            dst_row.copy_from_slice(src_row);
        }
    }

    /// Build a luminosity soft mask from a rendered buffer (ExtGState
    /// `/SMask /S /Luminosity`). The mask value for each pixel is the
    /// perceptual luminance of its RGB. We use Rec. 601 weights
    /// (0.299/0.587/0.114), which is what Poppler's `SplashBitmap` uses for
    /// luminosity soft masks; matching it keeps our masks PSNR-comparable.
    pub fn from_luminosity(buf: &PixelBuffer) -> Self {
        let len = (buf.width as usize)
            .checked_mul(buf.height as usize)
            .unwrap_or(0);
        let mut mask = Self {
            width: buf.width,
            height: buf.height,
            origin_x: 0,
            origin_y: 0,
            outside_alpha: 255,
            data: vec![0u8; len],
        };
        for y in 0..buf.height as i32 {
            for x in 0..buf.width as i32 {
                let p = buf.get_pixel(x, y);
                let lum = 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32;
                mask.set(x, y, lum.round().clamp(0.0, 255.0) as u8);
            }
        }
        mask
    }

    /// Build an alpha soft mask from a rendered buffer (ExtGState
    /// `/SMask /S /Alpha`). The mask value for each pixel is the buffer's own
    /// alpha channel Ã¢â‚¬â€ no luminosity conversion.
    pub fn from_alpha_channel(buf: &PixelBuffer) -> Self {
        let len = (buf.width as usize)
            .checked_mul(buf.height as usize)
            .unwrap_or(0);
        let mut mask = Self {
            width: buf.width,
            height: buf.height,
            origin_x: 0,
            origin_y: 0,
            outside_alpha: 255,
            data: vec![0u8; len],
        };
        for y in 0..buf.height as i32 {
            for x in 0..buf.width as i32 {
                mask.set(x, y, buf.get_pixel(x, y)[3]);
            }
        }
        mask
    }

    /// Remap every mask value through a transfer-function lookup table
    /// (256 entries, input index -> output byte). Used for ExtGState SMask
    /// `/TR` transfer functions.
    pub fn apply_transfer_lut(&mut self, lut: &[u8; 256]) {
        for v in self.data.iter_mut() {
            *v = lut[*v as usize];
        }
        self.outside_alpha = lut[self.outside_alpha as usize];
    }
}

#[derive(Debug, Clone)]
struct ClipEdge {
    y_min: f64,
    y_max: f64,
    x_at_ymin: f64,
    slope: f64,
    winding: i32,
}

fn fill_spans_into(intersections: &[(f64, i32)], rule: FillRule, spans: &mut Vec<(f64, f64)>) {
    spans.clear();
    match rule {
        FillRule::EvenOdd => {
            let mut iter = intersections.iter();
            while let Some((x_start, _)) = iter.next() {
                if let Some((x_end, _)) = iter.next() {
                    spans.push((*x_start, *x_end));
                }
            }
        }
        FillRule::NonZero => {
            let mut winding = 0i32;
            let mut span_start = None;
            for &(x, w) in intersections {
                let was_nonzero = winding != 0;
                winding += w;
                let is_nonzero = winding != 0;
                if !was_nonzero && is_nonzero {
                    span_start = Some(x);
                } else if was_nonzero && !is_nonzero {
                    if let Some(start) = span_start.take() {
                        spans.push((start, x));
                    }
                }
            }
        }
    }
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

fn blend_backdrop_rgb(blend_mode: BlendMode, src_rgb: [f32; 3], dst_rgb: [f32; 3]) -> [f32; 3] {
    if blend_mode.is_separable() {
        [
            blend_mode.blend_channel(src_rgb[0], dst_rgb[0]),
            blend_mode.blend_channel(src_rgb[1], dst_rgb[1]),
            blend_mode.blend_channel(src_rgb[2], dst_rgb[2]),
        ]
    } else {
        blend_mode.blend_rgb(src_rgb, dst_rgb)
    }
}

fn composite_source_over(
    src_rgb: [f32; 3],
    src_alpha: f32,
    dst_rgb: [f32; 3],
    dst_alpha: f32,
    blend_mode: BlendMode,
) -> ([f32; 3], f32) {
    let src_alpha = src_alpha.clamp(0.0, 1.0);
    let dst_alpha = dst_alpha.clamp(0.0, 1.0);
    let blended_rgb = if dst_alpha <= 1e-6 {
        src_rgb
    } else {
        blend_backdrop_rgb(blend_mode, src_rgb, dst_rgb)
    };
    let source_contribution = [
        src_rgb[0] * (1.0 - dst_alpha) + blended_rgb[0] * dst_alpha,
        src_rgb[1] * (1.0 - dst_alpha) + blended_rgb[1] * dst_alpha,
        src_rgb[2] * (1.0 - dst_alpha) + blended_rgb[2] * dst_alpha,
    ];
    let out_alpha = src_alpha + dst_alpha * (1.0 - src_alpha);

    if out_alpha < 1e-6 {
        return ([0.0, 0.0, 0.0], 0.0);
    }

    let inv_alpha = 1.0 / out_alpha;
    let out_rgb = [
        (source_contribution[0] * src_alpha + dst_rgb[0] * dst_alpha * (1.0 - src_alpha))
            * inv_alpha,
        (source_contribution[1] * src_alpha + dst_rgb[1] * dst_alpha * (1.0 - src_alpha))
            * inv_alpha,
        (source_contribution[2] * src_alpha + dst_rgb[2] * dst_alpha * (1.0 - src_alpha))
            * inv_alpha,
    ];
    (out_rgb, out_alpha)
}

#[derive(Debug)]
pub struct PixelBuffer {
    pub width: u32,
    pub height: u32,
    pub blend_mode: BlendMode,
    render_mode: RenderMode,
    data: Vec<u8>,
    clip: Option<ClipMask>,
    smask: Option<AlphaMask>,
    knockout_backdrop: Option<Box<PixelBuffer>>,
}

/// Allocation counters for renderer pixel surfaces.
///
/// The renderer closure uses these counters to prove that tile-local groups,
/// cached replay, image scaling, and scan conversion are reducing live pixel
/// memory instead of only shifting work between code paths. Counters are
/// process-global, monotonic where appropriate, and deliberately contain no
/// document data.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PixelBufferAllocationStats {
    pub allocation_count: u64,
    pub allocated_bytes: u64,
    pub live_bytes: u64,
    pub peak_live_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PixelCompositorStats {
    pub wide_solid_color_pixels: u64,
    pub wide_opaque_dst_pixels: u64,
    pub wide_uniform_alpha_pixels: u64,
    pub wide_separable_blend_pixels: u64,
    pub scalar_solid_color_pixels: u64,
    pub scalar_opaque_dst_pixels: u64,
    pub scalar_uniform_alpha_pixels: u64,
    pub scalar_separable_blend_pixels: u64,
    pub scalar_general_pixels: u64,
    pub soft_mask_opaque_dst_pixels: u64,
    pub wide_soft_mask_opaque_dst_pixels: u64,
    pub scalar_soft_mask_opaque_dst_pixels: u64,
    pub soft_mask_general_pixels: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelCompositorBackend {
    Scalar,
    PortableWide,
    Sse2,
    Avx2,
    Neon,
    WasmSimd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelCompositorOperation {
    SolidFill,
    SourceOver,
    AlphaMask,
    GlyphMask,
    SoftMask,
    SeparableBlend,
}

impl PixelCompositorOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            PixelCompositorOperation::SolidFill => "solid_fill",
            PixelCompositorOperation::SourceOver => "source_over",
            PixelCompositorOperation::AlphaMask => "alpha_mask",
            PixelCompositorOperation::GlyphMask => "glyph_mask",
            PixelCompositorOperation::SoftMask => "soft_mask",
            PixelCompositorOperation::SeparableBlend => "separable_blend",
        }
    }
}

impl PixelCompositorBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            PixelCompositorBackend::Scalar => "scalar",
            PixelCompositorBackend::PortableWide => "portable_wide",
            PixelCompositorBackend::Sse2 => "sse2",
            PixelCompositorBackend::Avx2 => "avx2",
            PixelCompositorBackend::Neon => "neon",
            PixelCompositorBackend::WasmSimd => "wasm_simd",
        }
    }
}

pub fn pixel_compositor_backend() -> PixelCompositorBackend {
    native_pixel_compositor_backend().unwrap_or(PixelCompositorBackend::PortableWide)
}

pub fn pixel_compositor_detected_hardware_backend() -> PixelCompositorBackend {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("avx2") {
            return PixelCompositorBackend::Avx2;
        }
        if std::is_x86_feature_detected!("sse2") {
            return PixelCompositorBackend::Sse2;
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        return PixelCompositorBackend::Neon;
    }
    #[cfg(all(target_arch = "arm", target_feature = "neon"))]
    {
        return PixelCompositorBackend::Neon;
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return PixelCompositorBackend::WasmSimd;
    }
    PixelCompositorBackend::Scalar
}

pub fn pixel_compositor_operation_backend(
    operation: PixelCompositorOperation,
) -> PixelCompositorBackend {
    match operation {
        PixelCompositorOperation::SolidFill
        | PixelCompositorOperation::SourceOver
        | PixelCompositorOperation::AlphaMask
        | PixelCompositorOperation::GlyphMask
        | PixelCompositorOperation::SoftMask => pixel_compositor_backend(),
        PixelCompositorOperation::SeparableBlend => PixelCompositorBackend::PortableWide,
    }
}

fn native_pixel_compositor_backend() -> Option<PixelCompositorBackend> {
    match wellfriendpdf_render_simd::active_backend() {
        wellfriendpdf_render_simd::SimdBackend::Avx2 => Some(PixelCompositorBackend::Avx2),
        wellfriendpdf_render_simd::SimdBackend::Sse2 => Some(PixelCompositorBackend::Sse2),
        wellfriendpdf_render_simd::SimdBackend::Neon => Some(PixelCompositorBackend::Neon),
        wellfriendpdf_render_simd::SimdBackend::WasmSimd => Some(PixelCompositorBackend::WasmSimd),
        wellfriendpdf_render_simd::SimdBackend::Scalar => None,
    }
}

static PIXEL_BUFFER_ALLOCATION_COUNT: AtomicU64 = AtomicU64::new(0);
static PIXEL_BUFFER_ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static PIXEL_BUFFER_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static PIXEL_BUFFER_PEAK_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_WIDE_SOLID_COLOR_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_WIDE_OPAQUE_DST_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_WIDE_UNIFORM_ALPHA_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_WIDE_SEPARABLE_BLEND_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_SCALAR_SOLID_COLOR_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_SCALAR_OPAQUE_DST_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_SCALAR_UNIFORM_ALPHA_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_SCALAR_SEPARABLE_BLEND_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_SCALAR_GENERAL_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_SOFT_MASK_OPAQUE_DST_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_WIDE_SOFT_MASK_OPAQUE_DST_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_SCALAR_SOFT_MASK_OPAQUE_DST_PIXELS: AtomicU64 = AtomicU64::new(0);
static PIXEL_COMPOSITOR_SOFT_MASK_GENERAL_PIXELS: AtomicU64 = AtomicU64::new(0);

#[inline]
fn record_pixel_buffer_alloc(bytes: usize) {
    let bytes = bytes as u64;
    PIXEL_BUFFER_ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
    PIXEL_BUFFER_ALLOCATED_BYTES.fetch_add(bytes, Ordering::Relaxed);
    let live = PIXEL_BUFFER_LIVE_BYTES.fetch_add(bytes, Ordering::Relaxed) + bytes;
    let mut observed = PIXEL_BUFFER_PEAK_LIVE_BYTES.load(Ordering::Relaxed);
    while live > observed {
        match PIXEL_BUFFER_PEAK_LIVE_BYTES.compare_exchange_weak(
            observed,
            live,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(next) => observed = next,
        }
    }
}

#[inline]
fn record_pixel_buffer_free(bytes: usize) {
    PIXEL_BUFFER_LIVE_BYTES.fetch_sub(bytes as u64, Ordering::Relaxed);
}

pub fn pixel_buffer_allocation_stats() -> PixelBufferAllocationStats {
    PixelBufferAllocationStats {
        allocation_count: PIXEL_BUFFER_ALLOCATION_COUNT.load(Ordering::Relaxed),
        allocated_bytes: PIXEL_BUFFER_ALLOCATED_BYTES.load(Ordering::Relaxed),
        live_bytes: PIXEL_BUFFER_LIVE_BYTES.load(Ordering::Relaxed),
        peak_live_bytes: PIXEL_BUFFER_PEAK_LIVE_BYTES.load(Ordering::Relaxed),
    }
}

pub fn pixel_compositor_stats() -> PixelCompositorStats {
    PixelCompositorStats {
        wide_solid_color_pixels: PIXEL_COMPOSITOR_WIDE_SOLID_COLOR_PIXELS.load(Ordering::Relaxed),
        wide_opaque_dst_pixels: PIXEL_COMPOSITOR_WIDE_OPAQUE_DST_PIXELS.load(Ordering::Relaxed),
        wide_uniform_alpha_pixels: PIXEL_COMPOSITOR_WIDE_UNIFORM_ALPHA_PIXELS
            .load(Ordering::Relaxed),
        wide_separable_blend_pixels: PIXEL_COMPOSITOR_WIDE_SEPARABLE_BLEND_PIXELS
            .load(Ordering::Relaxed),
        scalar_solid_color_pixels: PIXEL_COMPOSITOR_SCALAR_SOLID_COLOR_PIXELS
            .load(Ordering::Relaxed),
        scalar_opaque_dst_pixels: PIXEL_COMPOSITOR_SCALAR_OPAQUE_DST_PIXELS.load(Ordering::Relaxed),
        scalar_uniform_alpha_pixels: PIXEL_COMPOSITOR_SCALAR_UNIFORM_ALPHA_PIXELS
            .load(Ordering::Relaxed),
        scalar_separable_blend_pixels: PIXEL_COMPOSITOR_SCALAR_SEPARABLE_BLEND_PIXELS
            .load(Ordering::Relaxed),
        scalar_general_pixels: PIXEL_COMPOSITOR_SCALAR_GENERAL_PIXELS.load(Ordering::Relaxed),
        soft_mask_opaque_dst_pixels: PIXEL_COMPOSITOR_SOFT_MASK_OPAQUE_DST_PIXELS
            .load(Ordering::Relaxed),
        wide_soft_mask_opaque_dst_pixels: PIXEL_COMPOSITOR_WIDE_SOFT_MASK_OPAQUE_DST_PIXELS
            .load(Ordering::Relaxed),
        scalar_soft_mask_opaque_dst_pixels: PIXEL_COMPOSITOR_SCALAR_SOFT_MASK_OPAQUE_DST_PIXELS
            .load(Ordering::Relaxed),
        soft_mask_general_pixels: PIXEL_COMPOSITOR_SOFT_MASK_GENERAL_PIXELS.load(Ordering::Relaxed),
    }
}

impl Clone for PixelBuffer {
    fn clone(&self) -> Self {
        let data = self.data.clone();
        record_pixel_buffer_alloc(data.len());
        Self {
            width: self.width,
            height: self.height,
            blend_mode: self.blend_mode,
            render_mode: self.render_mode,
            data,
            clip: self.clip.clone(),
            smask: self.smask.clone(),
            knockout_backdrop: self.knockout_backdrop.clone(),
        }
    }
}

impl Drop for PixelBuffer {
    fn drop(&mut self) {
        record_pixel_buffer_free(self.data.len());
    }
}

impl PixelBuffer {
    /// Allocate a new transparent buffer.
    pub fn new(width: u32, height: u32) -> Self {
        Self::new_with_mode(width, height, RenderMode::Compat)
    }

    /// Allocate a new transparent buffer with an explicit render mode.
    pub fn new_with_mode(width: u32, height: u32, render_mode: RenderMode) -> Self {
        let len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .unwrap_or(0);
        let data = vec![0u8; len];
        record_pixel_buffer_alloc(data.len());
        Self {
            width,
            height,
            blend_mode: BlendMode::Normal,
            render_mode,
            data,
            clip: None,
            smask: None,
            knockout_backdrop: None,
        }
    }

    /// Allocate a fully transparent buffer. Used for off-screen transparency groups.
    pub fn new_transparent(width: u32, height: u32) -> Self {
        Self::new(width, height)
    }

    /// Allocate a fully transparent buffer with an explicit render mode.
    pub fn new_transparent_with_mode(width: u32, height: u32, render_mode: RenderMode) -> Self {
        Self::new_with_mode(width, height, render_mode)
    }

    /// Allocate and fill with the given color.
    pub fn new_filled(width: u32, height: u32, color: PixelColor) -> Self {
        Self::new_filled_with_mode(width, height, color, RenderMode::Compat)
    }

    /// Allocate and fill with the given color and render mode.
    pub fn new_filled_with_mode(
        width: u32,
        height: u32,
        color: PixelColor,
        render_mode: RenderMode,
    ) -> Self {
        let mut buf = Self::new_with_mode(width, height, render_mode);
        buf.fill(color);
        buf
    }

    pub fn render_mode(&self) -> RenderMode {
        self.render_mode
    }

    pub(crate) fn reset_transparent_for_reuse(
        &mut self,
        width: u32,
        height: u32,
        render_mode: RenderMode,
    ) -> bool {
        let Some(len) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
        else {
            return false;
        };
        if self.data.capacity() < len {
            return false;
        }
        self.width = width;
        self.height = height;
        self.blend_mode = BlendMode::Normal;
        self.render_mode = render_mode;
        self.data.resize(len, 0);
        self.data.fill(0);
        self.clip = None;
        self.smask = None;
        self.knockout_backdrop = None;
        true
    }

    pub(crate) fn reset_filled_for_reuse(
        &mut self,
        width: u32,
        height: u32,
        color: PixelColor,
        render_mode: RenderMode,
    ) -> bool {
        if !self.reset_transparent_for_reuse(width, height, render_mode) {
            return false;
        }
        self.fill(color);
        true
    }

    /// Copy a rectangle from this buffer into a new buffer using row slices.
    ///
    /// This is intentionally clip-independent: it is used by tile/progressive
    /// assembly after painting has already applied clipping. It avoids the
    /// previous `get_pixel`/`set_pixel` loop, which rechecked bounds and clip
    /// state for every pixel in already-rasterized rows.
    pub(crate) fn copy_rect_to_new_buffer(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let end_x = x.checked_add(width)?;
        let end_y = y.checked_add(height)?;
        if end_x > self.width || end_y > self.height {
            return None;
        }
        let mut out = PixelBuffer::new_transparent_with_mode(width, height, self.render_mode);
        let bytes_per_pixel = 4usize;
        let src_stride = self.width as usize * bytes_per_pixel;
        let dst_stride = width as usize * bytes_per_pixel;
        let src_x = x as usize * bytes_per_pixel;
        for row in 0..height as usize {
            let src_start = (y as usize + row)
                .checked_mul(src_stride)?
                .checked_add(src_x)?;
            let src_end = src_start.checked_add(dst_stride)?;
            let dst_start = row.checked_mul(dst_stride)?;
            let dst_end = dst_start.checked_add(dst_stride)?;
            out.data
                .get_mut(dst_start..dst_end)?
                .copy_from_slice(self.data.get(src_start..src_end)?);
        }
        Some(out)
    }

    /// Copy an already-rasterized source buffer into this buffer at a pixel
    /// destination using row slices. The destination clip is deliberately not
    /// applied; callers use this for assembling completed render tiles into an
    /// output surface, not for painting page content.
    pub(crate) fn blit_from_buffer(&mut self, src: &PixelBuffer, dst_x: u32, dst_y: u32) -> bool {
        let Some(end_x) = dst_x.checked_add(src.width) else {
            return false;
        };
        let Some(end_y) = dst_y.checked_add(src.height) else {
            return false;
        };
        if end_x > self.width || end_y > self.height {
            return false;
        }
        let bytes_per_pixel = 4usize;
        let dst_stride = self.width as usize * bytes_per_pixel;
        let src_stride = src.width as usize * bytes_per_pixel;
        let dst_x_bytes = dst_x as usize * bytes_per_pixel;
        for row in 0..src.height as usize {
            let src_start = row * src_stride;
            let src_end = src_start + src_stride;
            let dst_start = (dst_y as usize + row) * dst_stride + dst_x_bytes;
            let dst_end = dst_start + src_stride;
            let Some(dst_slice) = self.data.get_mut(dst_start..dst_end) else {
                return false;
            };
            let Some(src_slice) = src.data.get(src_start..src_end) else {
                return false;
            };
            dst_slice.copy_from_slice(src_slice);
        }
        true
    }

    fn pixel_index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        let idx = (y as usize)
            .checked_mul(self.width as usize)?
            .checked_add(x as usize)?
            .checked_mul(4)?;
        if idx + 3 < self.data.len() {
            Some(idx)
        } else {
            None
        }
    }

    /// Get the RGBA value of pixel (x, y). Returns transparent if out of bounds.
    pub fn get_pixel(&self, x: i32, y: i32) -> PixelColor {
        match self.pixel_index(x, y) {
            Some(idx) => [
                self.data[idx],
                self.data[idx + 1],
                self.data[idx + 2],
                self.data[idx + 3],
            ],
            None => TRANSPARENT,
        }
    }

    /// Set the RGBA value of pixel (x, y). No-op if out of bounds.
    pub fn set_pixel(&mut self, x: i32, y: i32, color: PixelColor) {
        if let Some(clip) = &self.clip {
            if !clip.is_visible(x, y) {
                return;
            }
        }
        if let Some(idx) = self.pixel_index(x, y) {
            self.data[idx] = color[0];
            self.data[idx + 1] = color[1];
            self.data[idx + 2] = color[2];
            self.data[idx + 3] = color[3];
        }
    }

    /// True when an already-computed opaque source pixel can be copied straight
    /// into the device buffer without invoking compositing, clipping, soft-mask,
    /// or knockout state. Image and glyph hot paths use this after they have
    /// already chosen a final sample color.
    #[inline]
    pub(crate) fn can_write_opaque_unclipped(&self) -> bool {
        self.blend_mode == BlendMode::Normal
            && self.smask.is_none()
            && self.knockout_backdrop.is_none()
            && self.clip.as_ref().is_none_or(ClipMask::is_all_visible)
    }

    pub(crate) fn can_write_opaque_with_binary_clip(&self) -> bool {
        self.blend_mode == BlendMode::Normal
            && self.smask.is_none()
            && self.knockout_backdrop.is_none()
            && self
                .clip
                .as_ref()
                .is_none_or(|clip| !clip.has_partial_coverage())
    }

    /// Write an opaque pixel without consulting paint-time clip/composite state.
    ///
    /// Call only after [`Self::can_write_opaque_unclipped`] has been checked.
    #[inline]
    pub(crate) fn write_opaque_pixel_unclipped(&mut self, x: i32, y: i32, color: PixelColor) {
        if let Some(idx) = self.pixel_index(x, y) {
            self.data[idx] = color[0];
            self.data[idx + 1] = color[1];
            self.data[idx + 2] = color[2];
            self.data[idx + 3] = 255;
        }
    }

    /// Write a row of 8-bit RGB samples as opaque pixels without consulting
    /// paint-time clip/composite state.
    ///
    /// Call only after [`Self::can_write_opaque_unclipped`] has been checked.
    #[inline]
    pub(crate) fn write_opaque_rgb_run_unclipped(&mut self, x: i32, y: i32, rgb: &[u8]) -> usize {
        if x < 0 || y < 0 || y >= self.height as i32 || rgb.len() < 3 {
            return 0;
        }
        let available_pixels = (self.width as i32).saturating_sub(x).max(0) as usize;
        let pixels = available_pixels.min(rgb.len() / 3);
        if pixels == 0 {
            return 0;
        }
        let Some(start) = (y as usize)
            .checked_mul(self.width as usize)
            .and_then(|row| row.checked_add(x as usize))
            .and_then(|pixel| pixel.checked_mul(4))
        else {
            return 0;
        };
        let Some(dst) = self.data.get_mut(start..start + pixels * 4) else {
            return 0;
        };
        for (out, src) in dst.chunks_exact_mut(4).zip(rgb.chunks_exact(3)) {
            out[0] = src[0];
            out[1] = src[1];
            out[2] = src[2];
            out[3] = 255;
        }
        pixels
    }

    /// Write a row of 8-bit RGB samples as opaque pixels through a binary clip.
    ///
    /// Call only after [`Self::can_write_opaque_with_binary_clip`] has been
    /// checked. Antialiased clips are deliberately excluded because this path
    /// writes pixels directly without fractional coverage.
    pub(crate) fn write_opaque_rgb_run_binary_clipped(
        &mut self,
        x: i32,
        y: i32,
        rgb: &[u8],
    ) -> usize {
        if self.clip.as_ref().is_none_or(ClipMask::is_all_visible) {
            return self.write_opaque_rgb_run_unclipped(x, y, rgb);
        }
        if y < 0 || y >= self.height as i32 || rgb.len() < 3 {
            return 0;
        }
        let pixels = rgb.len() / 3;
        if pixels == 0 {
            return 0;
        }
        let x_end = x.saturating_add(pixels as i32);
        let Some(clip) = &self.clip else {
            return self.write_opaque_rgb_run_unclipped(x, y, rgb);
        };
        if clip.is_empty() || clip.has_partial_coverage() {
            return 0;
        }
        let x0 = x.max(0).min(self.width as i32).min(clip.width as i32);
        let x1 = x_end.max(0).min(self.width as i32).min(clip.width as i32);
        if x1 <= x0 || y >= clip.height as i32 {
            return 0;
        }
        let mut written = 0usize;
        if clip.is_all_visible() {
            written = written.saturating_add(write_opaque_rgb_run_to_data(
                &mut self.data,
                self.width,
                (x0, x1),
                y,
                x,
                rgb,
            ));
            return written;
        }
        clip.for_each_visible_run_in_span(y, x0, x1, |start, end| {
            written = written.saturating_add(write_opaque_rgb_run_to_data(
                &mut self.data,
                self.width,
                (start, end),
                y,
                x,
                rgb,
            ));
        });
        written
    }

    /// Alpha-composite a color with coverage [0.0, 1.0] over the existing pixel.
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: PixelColor, coverage: f32) {
        if coverage <= 0.0 {
            return;
        }
        let clip_alpha = if let Some(clip) = &self.clip {
            let clip_alpha = clip.opacity(x, y);
            if clip_alpha <= 0.0 {
                return;
            }
            clip_alpha
        } else {
            1.0
        };
        let idx = match self.pixel_index(x, y) {
            Some(idx) => idx,
            None => return,
        };

        if color[3] == 255
            && coverage >= 1.0
            && clip_alpha >= 1.0
            && self.blend_mode == BlendMode::Normal
            && self.smask.is_none()
            && self.knockout_backdrop.is_none()
        {
            self.data[idx] = color[0];
            self.data[idx + 1] = color[1];
            self.data[idx + 2] = color[2];
            self.data[idx + 3] = 255;
            return;
        }

        let smask_alpha = self.smask.as_ref().map_or(1.0, |mask| mask.get(x, y));
        let eff_a = (color[3] as f32 / 255.0 * coverage * smask_alpha * clip_alpha).clamp(0.0, 1.0);
        if eff_a <= 0.0 {
            return;
        }

        // Compositing is done in sRGB (gamma) space â€” the channel values as
        // stored â€” to match the reference renderer (Poppler/Splash), which is the
        // visual-proof target. The source-over weighted sum and the blend-mode
        // functions operate directly on the normalised sRGB channels [0,1]. (An
        // earlier revision composited in linear light, which is arguably more
        // physically correct but diverged from Poppler on every semi-transparent
        // fill; the benchmark reference wins here.)
        let backdrop_pixel = self
            .knockout_backdrop
            .as_ref()
            .map(|backdrop| backdrop.get_pixel(x, y));
        let dst_pixel = backdrop_pixel.unwrap_or([
            self.data[idx],
            self.data[idx + 1],
            self.data[idx + 2],
            self.data[idx + 3],
        ]);
        let dst_a = dst_pixel[3] as f32 / 255.0;
        let (src_rgb, dst_rgb) = if self.render_mode.is_high_quality() {
            (
                [
                    gamma::to_linear(color[0]),
                    gamma::to_linear(color[1]),
                    gamma::to_linear(color[2]),
                ],
                [
                    gamma::to_linear(dst_pixel[0]),
                    gamma::to_linear(dst_pixel[1]),
                    gamma::to_linear(dst_pixel[2]),
                ],
            )
        } else {
            (
                [
                    color[0] as f32 / 255.0,
                    color[1] as f32 / 255.0,
                    color[2] as f32 / 255.0,
                ],
                [
                    dst_pixel[0] as f32 / 255.0,
                    dst_pixel[1] as f32 / 255.0,
                    dst_pixel[2] as f32 / 255.0,
                ],
            )
        };
        let (out_rgb, out_a) =
            composite_source_over(src_rgb, eff_a, dst_rgb, dst_a, self.blend_mode);

        if out_a < 1e-6 {
            self.data[idx] = 0;
            self.data[idx + 1] = 0;
            self.data[idx + 2] = 0;
            self.data[idx + 3] = 0;
            return;
        }

        if self.render_mode.is_high_quality() {
            self.data[idx] = gamma::to_srgb(out_rgb[0]);
            self.data[idx + 1] = gamma::to_srgb(out_rgb[1]);
            self.data[idx + 2] = gamma::to_srgb(out_rgb[2]);
        } else {
            let to_byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
            self.data[idx] = to_byte(out_rgb[0]);
            self.data[idx + 1] = to_byte(out_rgb[1]);
            self.data[idx + 2] = to_byte(out_rgb[2]);
        }
        self.data[idx + 3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
    }

    /// Composite an 8-bit alpha mask at the destination origin.
    ///
    /// Cached glyph, Type3, and stroked-path masks hit this path repeatedly in
    /// display-list replay. Keeping the mask compositor row/run based avoids
    /// re-entering the full per-pixel clip/soft-mask/blend dispatcher for the
    /// common normal-blend cases while preserving the existing `blend_pixel`
    /// fallback for partial clips, soft masks, knockout groups, and high-quality
    /// subpixel coverage.
    pub(crate) fn blend_alpha_mask(
        &mut self,
        dst_x: i32,
        dst_y: i32,
        mask_width: u32,
        mask_height: u32,
        alpha: &[u8],
        color: PixelColor,
    ) {
        if mask_width == 0
            || mask_height == 0
            || color[3] == 0
            || alpha.len() != mask_width as usize * mask_height as usize
        {
            return;
        }

        let x0 = dst_x.max(0).min(self.width as i32);
        let y0 = dst_y.max(0).min(self.height as i32);
        let x1 = dst_x
            .saturating_add(mask_width as i32)
            .max(0)
            .min(self.width as i32);
        let y1 = dst_y
            .saturating_add(mask_height as i32)
            .max(0)
            .min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        let clip_all_visible = match self.clip.as_ref() {
            None => true,
            Some(clip) if clip.is_empty() => return,
            Some(clip) if clip.is_all_visible() => true,
            _ => false,
        };
        let normal_unmasked = self.blend_mode == BlendMode::Normal
            && self.smask.is_none()
            && self.knockout_backdrop.is_none()
            && clip_all_visible;
        if normal_unmasked {
            for row in y0..y1 {
                let mask_row = (row - dst_y) as usize;
                let src_start = mask_row * mask_width as usize + (x0 - dst_x) as usize;
                let src_end = mask_row * mask_width as usize + (x1 - dst_x) as usize;
                let Some(mask_row_alpha) = alpha.get(src_start..src_end) else {
                    continue;
                };
                if !self.render_mode.is_high_quality() {
                    blend_alpha_mask_run_normal(
                        &mut self.data,
                        self.width,
                        row,
                        x0,
                        mask_row_alpha,
                        color,
                    );
                    continue;
                }
                let mut col = x0;
                let mut idx = 0usize;
                while idx < mask_row_alpha.len() {
                    let mask_alpha = mask_row_alpha[idx];
                    if mask_alpha == 0 {
                        idx += 1;
                        col += 1;
                        continue;
                    }
                    let effective_alpha =
                        ((u16::from(color[3]) * u16::from(mask_alpha) + 127) / 255) as u8;
                    if effective_alpha == 0 {
                        idx += 1;
                        col += 1;
                        continue;
                    }
                    let run_start = col;
                    let mut run_len = 1usize;
                    while idx + run_len < mask_row_alpha.len() {
                        let next_alpha = mask_row_alpha[idx + run_len];
                        let next_effective =
                            ((u16::from(color[3]) * u16::from(next_alpha) + 127) / 255) as u8;
                        if next_effective != effective_alpha {
                            break;
                        }
                        run_len += 1;
                    }
                    let run_end = run_start.saturating_add(run_len as i32);
                    if effective_alpha == 255 {
                        fill_opaque_run(&mut self.data, self.width, row, run_start, run_end, color);
                    } else if !self.render_mode.is_high_quality() {
                        let mut run_color = color;
                        run_color[3] = effective_alpha;
                        blend_normal_compat_run(
                            &mut self.data,
                            self.width,
                            row,
                            run_start,
                            run_end,
                            run_color,
                        );
                    } else {
                        for offset in 0..run_len {
                            self.blend_pixel(
                                run_start.saturating_add(offset as i32),
                                row,
                                color,
                                f32::from(mask_row_alpha[idx + offset]) / 255.0,
                            );
                        }
                    }
                    idx += run_len;
                    col = run_end;
                }
            }
            return;
        }

        let normal_binary_clip = self.blend_mode == BlendMode::Normal
            && self.smask.is_none()
            && self.knockout_backdrop.is_none()
            && !self.render_mode.is_high_quality()
            && self
                .clip
                .as_ref()
                .is_some_and(|clip| !clip.is_empty() && !clip.has_partial_coverage());
        if normal_binary_clip {
            if let Some(clip) = self.clip.clone() {
                for row in y0..y1 {
                    let mask_row = (row - dst_y) as usize;
                    clip.for_each_visible_run_in_span(row, x0, x1, |run_x0, run_x1| {
                        let src_start = mask_row * mask_width as usize + (run_x0 - dst_x) as usize;
                        let src_end = mask_row * mask_width as usize + (run_x1 - dst_x) as usize;
                        let Some(mask_row_alpha) = alpha.get(src_start..src_end) else {
                            return;
                        };
                        blend_alpha_mask_run_normal(
                            &mut self.data,
                            self.width,
                            row,
                            run_x0,
                            mask_row_alpha,
                            color,
                        );
                    });
                }
            }
            return;
        }

        for row in y0..y1 {
            let mask_row = (row - dst_y) as usize;
            let src_start = mask_row * mask_width as usize + (x0 - dst_x) as usize;
            let src_end = mask_row * mask_width as usize + (x1 - dst_x) as usize;
            let Some(mask_row_alpha) = alpha.get(src_start..src_end) else {
                continue;
            };
            for (offset, mask_alpha) in mask_row_alpha.iter().copied().enumerate() {
                if mask_alpha == 0 {
                    continue;
                }
                self.blend_pixel(
                    x0.saturating_add(offset as i32),
                    row,
                    color,
                    f32::from(mask_alpha) / 255.0,
                );
            }
        }
    }

    /// Composite a cached RGBA glyph/image fragment at a destination origin.
    ///
    /// Type3 glyph replay uses this for cached rendered glyphs. It keeps the
    /// same `blend_pixel` fallback for complex state, but routes normal Compat
    /// rows through row-slice compositors and binary-clip runs instead of
    /// dispatching one pixel at a time.
    pub(crate) fn blend_rgba_pixels_at(
        &mut self,
        dst_x: i32,
        dst_y: i32,
        src_width: u32,
        src_height: u32,
        rgba: &[u8],
    ) {
        if src_width == 0
            || src_height == 0
            || rgba.len() != src_width as usize * src_height as usize * 4
        {
            return;
        }
        let x0 = dst_x.max(0).min(self.width as i32);
        let y0 = dst_y.max(0).min(self.height as i32);
        let x1 = dst_x
            .saturating_add(src_width as i32)
            .max(0)
            .min(self.width as i32);
        let y1 = dst_y
            .saturating_add(src_height as i32)
            .max(0)
            .min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        let normal_compat = self.blend_mode == BlendMode::Normal
            && self.smask.is_none()
            && self.knockout_backdrop.is_none()
            && !self.render_mode.is_high_quality();
        if normal_compat {
            let dst_stride = self.width as usize * 4;
            let src_stride = src_width as usize * 4;
            for row in y0..y1 {
                let src_y = (row - dst_y) as usize;
                let src_x0 = (x0 - dst_x) as usize;
                let src_len = (x1 - x0) as usize * 4;
                let src_start = src_y
                    .saturating_mul(src_stride)
                    .saturating_add(src_x0.saturating_mul(4));
                let Some(src_row) = rgba.get(src_start..src_start.saturating_add(src_len)) else {
                    continue;
                };
                if row_alpha_class(src_row) == RowAlphaClass::AllTransparent {
                    continue;
                }
                match self.clip.as_ref() {
                    Some(clip) if clip.is_empty() => return,
                    None => {
                        let dst_start = row as usize * dst_stride + x0 as usize * 4;
                        if let Some(dst_row) = self
                            .data
                            .get_mut(dst_start..dst_start.saturating_add(src_len))
                        {
                            composite_normal_compat_row(dst_row, src_row, 1.0);
                        }
                    }
                    Some(clip) if clip.is_all_visible() => {
                        let dst_start = row as usize * dst_stride + x0 as usize * 4;
                        if let Some(dst_row) = self
                            .data
                            .get_mut(dst_start..dst_start.saturating_add(src_len))
                        {
                            composite_normal_compat_row(dst_row, src_row, 1.0);
                        }
                    }
                    Some(clip) if !clip.has_partial_coverage() => {
                        clip.for_each_visible_run_in_span(row, x0, x1, |run_x0, run_x1| {
                            let local_x0 = (run_x0 - x0) as usize;
                            let run_len = (run_x1 - run_x0) as usize * 4;
                            let dst_start = row as usize * dst_stride + run_x0 as usize * 4;
                            let src_start = local_x0 * 4;
                            if let (Some(dst_row), Some(src_row)) = (
                                self.data
                                    .get_mut(dst_start..dst_start.saturating_add(run_len)),
                                src_row.get(src_start..src_start.saturating_add(run_len)),
                            ) {
                                composite_normal_compat_row(dst_row, src_row, 1.0);
                            }
                        });
                    }
                    _ => break,
                }
            }
            if self
                .clip
                .as_ref()
                .is_none_or(|clip| !clip.has_partial_coverage())
            {
                return;
            }
        }

        for row in y0..y1 {
            let src_y = (row - dst_y) as usize;
            for col in x0..x1 {
                let src_x = (col - dst_x) as usize;
                let src_idx = src_y
                    .saturating_mul(src_width as usize)
                    .saturating_add(src_x)
                    .saturating_mul(4);
                let Some(src) = rgba.get(src_idx..src_idx.saturating_add(4)) else {
                    continue;
                };
                let color = [src[0], src[1], src[2], src[3]];
                if color[3] != 0 {
                    self.blend_pixel(col, row, color, 1.0);
                }
            }
        }
    }

    pub(crate) fn blend_device_cmyk_overprint_preview(
        &mut self,
        x: i32,
        y: i32,
        cmyk: [f32; 4],
        alpha: f32,
        coverage: f32,
        overprint_mode: i32,
    ) {
        if coverage <= 0.0 {
            return;
        }
        let clip_alpha = if let Some(clip) = &self.clip {
            let clip_alpha = clip.opacity(x, y);
            if clip_alpha <= 0.0 {
                return;
            }
            clip_alpha
        } else {
            1.0
        };
        let idx = match self.pixel_index(x, y) {
            Some(idx) => idx,
            None => return,
        };

        let smask_alpha = self.smask.as_ref().map_or(1.0, |mask| mask.get(x, y));
        let eff_a = (alpha * coverage * smask_alpha * clip_alpha).clamp(0.0, 1.0);
        if eff_a <= 0.0 {
            return;
        }

        let dst_rgb_srgb = [
            self.data[idx] as f32 / 255.0,
            self.data[idx + 1] as f32 / 255.0,
            self.data[idx + 2] as f32 / 255.0,
        ];
        let src_rgb_srgb =
            cmm::device_cmyk_overprint_preview_srgb(dst_rgb_srgb, cmyk, overprint_mode == 1);
        let dst_a = self.data[idx + 3] as f32 / 255.0;

        let (src_rgb, dst_rgb) = if self.render_mode.is_high_quality() {
            (
                [
                    gamma::to_linear_f32(src_rgb_srgb[0]),
                    gamma::to_linear_f32(src_rgb_srgb[1]),
                    gamma::to_linear_f32(src_rgb_srgb[2]),
                ],
                [
                    gamma::to_linear(self.data[idx]),
                    gamma::to_linear(self.data[idx + 1]),
                    gamma::to_linear(self.data[idx + 2]),
                ],
            )
        } else {
            (src_rgb_srgb, dst_rgb_srgb)
        };
        let (out_rgb, out_a) =
            composite_source_over(src_rgb, eff_a, dst_rgb, dst_a, self.blend_mode);

        if out_a < 1e-6 {
            self.data[idx] = 0;
            self.data[idx + 1] = 0;
            self.data[idx + 2] = 0;
            self.data[idx + 3] = 0;
            return;
        }

        if self.render_mode.is_high_quality() {
            self.data[idx] = gamma::to_srgb(out_rgb[0]);
            self.data[idx + 1] = gamma::to_srgb(out_rgb[1]);
            self.data[idx + 2] = gamma::to_srgb(out_rgb[2]);
        } else {
            let to_byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
            self.data[idx] = to_byte(out_rgb[0]);
            self.data[idx + 1] = to_byte(out_rgb[1]);
            self.data[idx + 2] = to_byte(out_rgb[2]);
        }
        self.data[idx + 3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
    }

    /// Fill the entire buffer with a solid color.
    pub fn fill(&mut self, color: PixelColor) {
        if color.iter().all(|component| *component == color[0]) {
            self.data.fill(color[0]);
            return;
        }
        for chunk in self.data.chunks_exact_mut(4) {
            chunk[0] = color[0];
            chunk[1] = color[1];
            chunk[2] = color[2];
            chunk[3] = color[3];
        }
    }

    /// Fill a rectangular region. Clips to buffer bounds.
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: PixelColor) {
        if w <= 0 || h <= 0 {
            return;
        }
        if color[3] == 0 {
            return;
        }
        let x0 = x.max(0).min(self.width as i32);
        let y0 = y.max(0).min(self.height as i32);
        let x1 = x.saturating_add(w).max(0).min(self.width as i32);
        let y1 = y.saturating_add(h).max(0).min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }

        if color[3] < 255
            && self.blend_mode == BlendMode::Normal
            && self.smask.is_none()
            && self.knockout_backdrop.is_none()
            && !self.render_mode.is_high_quality()
        {
            let Some(clip) = self.clip.as_ref() else {
                for row in y0..y1 {
                    blend_normal_compat_run(&mut self.data, self.width, row, x0, x1, color);
                }
                return;
            };
            if clip.is_empty() {
                return;
            }
            if clip.is_all_visible() {
                for row in y0..y1 {
                    blend_normal_compat_run(&mut self.data, self.width, row, x0, x1, color);
                }
                return;
            }
            if clip.has_partial_coverage() {
                for row in y0..y1 {
                    for col in x0..x1 {
                        self.blend_pixel(col, row, color, 1.0);
                    }
                }
                return;
            }
            for row in y0..y1 {
                clip.for_each_visible_run_in_span(row, x0, x1, |start, end| {
                    blend_normal_compat_run(&mut self.data, self.width, row, start, end, color);
                });
            }
            return;
        }

        let should_blend = color[3] < 255
            || self.blend_mode != BlendMode::Normal
            || self.smask.is_some()
            || self.knockout_backdrop.is_some();
        if should_blend {
            if color[3] == 255
                && self.blend_mode != BlendMode::Normal
                && self.blend_mode.is_separable()
                && self.smask.is_none()
                && self.knockout_backdrop.is_none()
                && !self.render_mode.is_high_quality()
            {
                let blend_mode = self.blend_mode;
                let can_fast_path = match self.clip.as_ref() {
                    None => rect_runs_have_opaque_alpha(&self.data, self.width, y0, y1, x0, x1),
                    Some(clip) if clip.is_empty() => return,
                    Some(clip) if clip.is_all_visible() => {
                        rect_runs_have_opaque_alpha(&self.data, self.width, y0, y1, x0, x1)
                    }
                    Some(clip) if !clip.has_partial_coverage() => {
                        clipped_runs_have_opaque_alpha(&self.data, self.width, clip, y0, y1, x0, x1)
                    }
                    Some(_) => false,
                };

                if can_fast_path {
                    let clip = self.clip.clone();
                    match clip.as_ref() {
                        None => {
                            for row in y0..y1 {
                                blend_separable_opaque_src_over_opaque_dst_run(
                                    &mut self.data,
                                    self.width,
                                    row,
                                    x0,
                                    x1,
                                    color,
                                    blend_mode,
                                );
                            }
                        }
                        Some(clip) if clip.is_all_visible() => {
                            for row in y0..y1 {
                                blend_separable_opaque_src_over_opaque_dst_run(
                                    &mut self.data,
                                    self.width,
                                    row,
                                    x0,
                                    x1,
                                    color,
                                    blend_mode,
                                );
                            }
                        }
                        Some(clip) => {
                            let mut runs = Vec::new();
                            for row in y0..y1 {
                                runs.clear();
                                clip.for_each_visible_run_in_span(row, x0, x1, |start, end| {
                                    runs.push((start, end));
                                });
                                for (start, end) in runs.iter().copied() {
                                    blend_separable_opaque_src_over_opaque_dst_run(
                                        &mut self.data,
                                        self.width,
                                        row,
                                        start,
                                        end,
                                        color,
                                        blend_mode,
                                    );
                                }
                            }
                        }
                    }
                    return;
                }
            }

            for row in y0..y1 {
                for col in x0..x1 {
                    self.blend_pixel(col, row, color, 1.0);
                }
            }
            return;
        }

        let Some(clip) = self.clip.as_ref() else {
            for row in y0..y1 {
                fill_opaque_run(&mut self.data, self.width, row, x0, x1, color);
            }
            return;
        };
        if clip.is_empty() {
            return;
        }
        if clip.is_all_visible() {
            for row in y0..y1 {
                fill_opaque_run(&mut self.data, self.width, row, x0, x1, color);
            }
            return;
        }
        if clip.has_partial_coverage() {
            for row in y0..y1 {
                for col in x0..x1 {
                    self.blend_pixel(col, row, color, 1.0);
                }
            }
            return;
        }

        for row in y0..y1 {
            clip.for_each_visible_run_in_span(row, x0, x1, |start, end| {
                fill_opaque_run(&mut self.data, self.width, row, start, end, color);
            });
        }
    }

    /// Intersect the existing clip with `mask`, or install it as the first clip.
    pub fn set_clip(&mut self, mask: ClipMask) {
        if let Some(existing) = &mut self.clip {
            existing.intersect(&mask);
        } else {
            self.clip = Some(mask);
        }
    }

    /// Clear clipping; all pixels become paintable.
    pub fn clear_clip(&mut self) {
        self.clip = None;
    }

    /// Directly replace the current clip without intersecting.
    pub fn replace_clip(&mut self, clip: Option<ClipMask>) {
        self.clip = clip;
    }

    /// True if a clip mask is installed.
    pub fn has_clip(&self) -> bool {
        self.clip.is_some()
    }

    /// Borrow the current clip mask, if any.
    pub fn clip_mask(&self) -> Option<&ClipMask> {
        self.clip.as_ref()
    }

    pub fn set_smask(&mut self, mask: AlphaMask) {
        self.smask = Some(mask);
    }

    pub fn clear_smask(&mut self) {
        self.smask = None;
    }

    pub fn smask_mask(&self) -> Option<&AlphaMask> {
        self.smask.as_ref()
    }

    pub(crate) fn set_knockout_backdrop(&mut self, mut backdrop: PixelBuffer) {
        backdrop.clear_knockout_backdrop();
        self.knockout_backdrop = Some(Box::new(backdrop));
    }

    pub(crate) fn clear_knockout_backdrop(&mut self) {
        self.knockout_backdrop = None;
    }

    /// True if the pixel at (x, y) is paintable under the current clip. With no
    /// clip installed every in-bounds pixel is allowed. Used by the shading
    /// renderer to skip expensive colour evaluation for clipped pixels.
    pub fn clip_allows(&self, x: i32, y: i32) -> bool {
        match &self.clip {
            Some(clip) => clip.is_visible(x, y),
            None => true,
        }
    }

    pub(crate) fn restore_clip(&mut self, clip: Option<ClipMask>) {
        self.clip = clip;
    }

    pub(crate) fn restore_smask(&mut self, smask: Option<AlphaMask>) {
        self.smask = smask;
    }

    /// Return RGB bytes, discarding alpha.
    pub fn to_rgb_bytes(&self) -> Vec<u8> {
        let pixel_count = self.width as usize * self.height as usize;
        let mut out = Vec::with_capacity(pixel_count * 3);
        for chunk in self.data.chunks_exact(4) {
            out.push(chunk[0]);
            out.push(chunk[1]);
            out.push(chunk[2]);
        }
        out
    }

    /// Flatten this straight-alpha buffer onto an opaque background.
    ///
    /// PDF pages are transparency groups: page content starts transparent, then
    /// the finished page is composited onto the output medium. PNG/JPEG render
    /// outputs use white paper as that medium, but blend modes must not see that
    /// white as their initial backdrop while the page content is still painting.
    pub fn flatten_onto_background(&mut self, background: PixelColor) {
        if background[3] == 255 && !self.render_mode.is_high_quality() {
            flatten_compat_onto_opaque_background(&mut self.data, background);
            return;
        }

        let bg_a = background[3] as f32 / 255.0;
        for chunk in self.data.chunks_exact_mut(4) {
            let src_a = chunk[3] as f32 / 255.0;
            if src_a >= 1.0 && bg_a >= 1.0 {
                chunk[3] = 255;
                continue;
            }

            let out_a = src_a + bg_a * (1.0 - src_a);
            if out_a <= 1e-6 {
                chunk.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }

            if self.render_mode.is_high_quality() {
                for c in 0..3 {
                    let src = gamma::to_linear(chunk[c]);
                    let bg = gamma::to_linear(background[c]);
                    let out = (src * src_a + bg * bg_a * (1.0 - src_a)) / out_a;
                    chunk[c] = gamma::to_srgb(out);
                }
            } else {
                for c in 0..3 {
                    let src = chunk[c] as f32 / 255.0;
                    let bg = background[c] as f32 / 255.0;
                    let out = (src * src_a + bg * bg_a * (1.0 - src_a)) / out_a;
                    chunk[c] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
            chunk[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    /// Return RGBA bytes.
    pub fn to_rgba_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }

    /// Borrow the internal RGBA bytes without allocating.
    ///
    /// This is intended for hashing, comparison, and zero-copy binding paths
    /// that do not need ownership of the pixel buffer. The byte layout is the
    /// same as [`PixelBuffer::to_rgba_bytes`].
    pub fn rgba_bytes(&self) -> &[u8] {
        &self.data
    }

    pub(crate) fn rgba_bytes_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Convert to a RawImage for use with ImageEncoder.
    pub fn to_raw_image(&self) -> RawImage {
        RawImage {
            width: self.width,
            height: self.height,
            channels: 3,
            bits_per_sample: 8,
            pixels: self.to_rgb_bytes(),
        }
    }

    /// Convert to a RawImage with alpha channel.
    pub fn to_raw_image_rgba(&self) -> RawImage {
        RawImage {
            width: self.width,
            height: self.height,
            channels: 4,
            bits_per_sample: 8,
            pixels: self.to_rgba_bytes(),
        }
    }

    /// Composite a source RGBA buffer onto this buffer.
    ///
    /// This is the primitive used to flatten a transparency-group offscreen
    /// buffer onto its parent (the page buffer, or an enclosing group). The
    /// source's own per-pixel alpha is honored, then scaled by `group_alpha`
    /// (the `/ca` or `/CA` constant active at the `Do` operator) and, if
    /// present, by the per-pixel `soft_mask` value. Blending of color channels
    /// uses `blend_mode`. `self`'s own clip mask still applies.
    ///
    /// `self` and `src` must have the same dimensions (both are page-sized in
    /// the renderer), which keeps device coordinates aligned 1:1.
    pub fn composite_from(
        &mut self,
        src: &PixelBuffer,
        group_alpha: f32,
        blend_mode: BlendMode,
        soft_mask: Option<&AlphaMask>,
    ) {
        let alpha = group_alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        if blend_mode == BlendMode::Normal
            && self.knockout_backdrop.is_none()
            && !self.render_mode.is_high_quality()
        {
            if let Some(mask) = soft_mask {
                let mask_covers_composite = mask.origin_x == 0
                    && mask.origin_y == 0
                    && mask.width >= self.width.min(src.width)
                    && mask.height >= self.height.min(src.height);
                match self.clip.as_ref() {
                    Some(clip) if clip.is_empty() => return,
                    None if mask_covers_composite => {
                        composite_normal_compat_buffer_unclipped_soft_mask(
                            &mut self.data,
                            self.width,
                            self.height,
                            src,
                            alpha,
                            mask,
                        );
                        return;
                    }
                    Some(clip) if clip.is_all_visible() && mask_covers_composite => {
                        composite_normal_compat_buffer_unclipped_soft_mask(
                            &mut self.data,
                            self.width,
                            self.height,
                            src,
                            alpha,
                            mask,
                        );
                        return;
                    }
                    Some(clip) if !clip.has_partial_coverage() && mask_covers_composite => {
                        let w = self.width.min(src.width).min(mask.width) as i32;
                        let h = self.height.min(src.height).min(mask.height) as i32;
                        for row in 0..h {
                            clip.for_each_visible_run(row, w, |start, end| {
                                composite_normal_compat_row_run_soft_mask(
                                    &mut self.data,
                                    self.width,
                                    (row, start, end),
                                    src,
                                    alpha,
                                    mask,
                                );
                            });
                        }
                        return;
                    }
                    _ => {}
                }
            } else {
                match self.clip.as_ref() {
                    Some(clip) if clip.is_empty() => return,
                    Some(clip) if clip.has_partial_coverage() => {}
                    Some(clip) if !clip.is_all_visible() => {
                        let w = self.width.min(src.width) as i32;
                        let h = self.height.min(src.height) as i32;
                        for row in 0..h {
                            clip.for_each_visible_run(row, w, |start, end| {
                                composite_normal_compat_row_run(
                                    &mut self.data,
                                    self.width,
                                    row,
                                    start,
                                    end,
                                    src,
                                    alpha,
                                );
                            });
                        }
                        return;
                    }
                    _ => {
                        composite_normal_compat_buffer_unclipped(
                            &mut self.data,
                            self.width,
                            self.height,
                            src,
                            alpha,
                        );
                        return;
                    }
                }
            }
        }

        let saved_blend = self.blend_mode;
        self.blend_mode = blend_mode;
        // The caller passes the active page soft mask as `soft_mask`; that is the
        // single source of masking for this group-flatten composite. `blend_pixel`
        // would *also* multiply by `self.smask` (the same page mask, still installed
        // on this buffer), squaring the mask (e.g. 0.5 -> 0.25) â€” confirmed against
        // Poppler/Splash, which applies the soft mask exactly once. Temporarily
        // detach `self.smask` for the duration of the composite and restore it
        // afterwards so subsequent direct paints under the same /SMask stay masked.
        let saved_smask = self.smask.take();
        let w = self.width.min(src.width) as i32;
        let h = self.height.min(src.height) as i32;
        for y in 0..h {
            for x in 0..w {
                let sp = src.get_pixel(x, y);
                if sp[3] == 0 {
                    continue;
                }
                let mask = soft_mask.map_or(1.0, |m| m.get(x, y));
                let coverage = alpha * mask;
                if coverage <= 0.0 {
                    continue;
                }
                // `blend_pixel` interprets the source's alpha (sp[3]) and the
                // coverage multiplier together, applying the buffer blend mode
                // and any installed clip; reusing it keeps a single compositing
                // code path for direct paints and group flattening.
                self.blend_pixel(x, y, sp, coverage);
            }
        }
        self.blend_mode = saved_blend;
        self.smask = saved_smask;
    }

    /// Composite a smaller, tile-local transparency-group result back into
    /// this buffer at a destination offset.
    ///
    /// This keeps the same PDF source-over/blend/clip/soft-mask semantics as
    /// [`Self::composite_from`] while avoiding a full-page off-screen buffer for
    /// bounded Form transparency groups. Parent clip and soft-mask coordinates
    /// are evaluated in destination coordinates; source samples are read from
    /// the compact group-local buffer.
    pub(crate) fn composite_from_at(
        &mut self,
        src: &PixelBuffer,
        dst_x: u32,
        dst_y: u32,
        group_alpha: f32,
        blend_mode: BlendMode,
        soft_mask: Option<&AlphaMask>,
    ) {
        if dst_x == 0 && dst_y == 0 && src.width == self.width && src.height == self.height {
            self.composite_from(src, group_alpha, blend_mode, soft_mask);
            return;
        }
        if src.width == 0 || src.height == 0 {
            return;
        }
        let width = src.width.min(self.width.saturating_sub(dst_x));
        let height = src.height.min(self.height.saturating_sub(dst_y));
        if width == 0 || height == 0 {
            return;
        }

        let alpha = group_alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }

        if blend_mode == BlendMode::Normal && self.render_mode == RenderMode::Compat {
            let saved_smask = self.smask.take();
            let dst_stride = self.width as usize * 4;
            let src_stride = src.width as usize * 4;
            for local_y in 0..height as usize {
                let dest_y = dst_y as usize + local_y;
                let src_start = local_y * src_stride;
                let src_len = width as usize * 4;
                let Some(src_row) = src.data.get(src_start..src_start + src_len) else {
                    break;
                };
                if row_alpha_class(src_row) == RowAlphaClass::AllTransparent {
                    continue;
                }
                let dest_x0 = dst_x as i32;
                let dest_x1 = dst_x.saturating_add(width) as i32;
                let dest_y_i32 = dest_y as i32;
                match (self.clip.as_ref(), soft_mask) {
                    (None, None) => {
                        let dst_start = dest_y * dst_stride + dst_x as usize * 4;
                        let Some(dst_row) = self.data.get_mut(dst_start..dst_start + src_len)
                        else {
                            break;
                        };
                        composite_normal_compat_row(dst_row, src_row, alpha);
                    }
                    (Some(clip), None)
                        if clip.is_all_visible()
                            || (clip.solid.is_none() && !clip.has_partial_coverage()) =>
                    {
                        clip.for_each_visible_run_in_span(
                            dest_y_i32,
                            dest_x0,
                            dest_x1,
                            |run_start, run_end| {
                                let local_start = run_start.saturating_sub(dest_x0) as usize;
                                let pixels = run_end.saturating_sub(run_start) as usize;
                                if pixels == 0 {
                                    return;
                                }
                                let dst_start = dest_y * dst_stride + run_start as usize * 4;
                                let src_start = local_start * 4;
                                let len = pixels * 4;
                                if let (Some(dst_run), Some(src_run)) = (
                                    self.data.get_mut(dst_start..dst_start + len),
                                    src_row.get(src_start..src_start + len),
                                ) {
                                    composite_normal_compat_row(dst_run, src_run, alpha);
                                }
                            },
                        );
                    }
                    (None, Some(mask))
                        if mask.origin_x == 0
                            && mask.origin_y == 0
                            && mask.width >= dst_x.saturating_add(width)
                            && mask.height > dest_y as u32 =>
                    {
                        let dst_start = dest_y * dst_stride + dst_x as usize * 4;
                        let Some(dst_row) = self.data.get_mut(dst_start..dst_start + src_len)
                        else {
                            break;
                        };
                        let mask_stride = mask.width as usize;
                        let mask_start = dest_y * mask_stride + dst_x as usize;
                        if let Some(mask_row) =
                            mask.data.get(mask_start..mask_start + width as usize)
                        {
                            let alpha_255 = (alpha * 255.0).round() as u16;
                            match mask_row_class(mask_row) {
                                MaskRowClass::AllTransparent => {}
                                MaskRowClass::AllOpaque => {
                                    composite_normal_compat_row(dst_row, src_row, alpha);
                                }
                                MaskRowClass::Mixed if row_alpha_is(dst_row, 255) => {
                                    composite_normal_compat_row_soft_mask_opaque_dst(
                                        dst_row, src_row, mask_row, alpha_255,
                                    );
                                }
                                MaskRowClass::Mixed => {
                                    composite_normal_compat_row_soft_mask_scalar(
                                        dst_row, src_row, mask_row, alpha_255,
                                    )
                                }
                            }
                        }
                    }
                    (Some(clip), Some(mask))
                        if !clip.has_partial_coverage()
                            && mask.origin_x == 0
                            && mask.origin_y == 0
                            && mask.width >= dst_x.saturating_add(width)
                            && mask.height > dest_y as u32 =>
                    {
                        let mask_stride = mask.width as usize;
                        clip.for_each_visible_run_in_span(
                            dest_y_i32,
                            dest_x0,
                            dest_x1,
                            |run_start, run_end| {
                                let local_start = run_start.saturating_sub(dest_x0) as usize;
                                let pixels = run_end.saturating_sub(run_start) as usize;
                                if pixels == 0 {
                                    return;
                                }
                                let dst_start = dest_y * dst_stride + run_start as usize * 4;
                                let src_start = local_start * 4;
                                let mask_start = dest_y * mask_stride + run_start as usize;
                                let len = pixels * 4;
                                if let (Some(dst_run), Some(src_run), Some(mask_run)) = (
                                    self.data.get_mut(dst_start..dst_start + len),
                                    src_row.get(src_start..src_start + len),
                                    mask.data.get(mask_start..mask_start + pixels),
                                ) {
                                    let alpha_255 = (alpha * 255.0).round() as u16;
                                    match mask_row_class(mask_run) {
                                        MaskRowClass::AllTransparent => {}
                                        MaskRowClass::AllOpaque => {
                                            composite_normal_compat_row(dst_run, src_run, alpha);
                                        }
                                        MaskRowClass::Mixed if row_alpha_is(dst_run, 255) => {
                                            composite_normal_compat_row_soft_mask_opaque_dst(
                                                dst_run, src_run, mask_run, alpha_255,
                                            );
                                        }
                                        MaskRowClass::Mixed => {
                                            composite_normal_compat_row_soft_mask_scalar(
                                                dst_run, src_run, mask_run, alpha_255,
                                            );
                                        }
                                    }
                                }
                            },
                        );
                    }
                    _ => {
                        for local_x in 0..width as i32 {
                            let dest_x = dst_x as i32 + local_x;
                            let sp = src.get_pixel(local_x, local_y as i32);
                            if sp[3] == 0 {
                                continue;
                            }
                            let mask_alpha =
                                soft_mask.map_or(1.0, |mask| mask.get(dest_x, dest_y_i32));
                            let coverage = mask_alpha;
                            self.blend_pixel(dest_x, dest_y_i32, sp, coverage * alpha);
                        }
                    }
                }
            }
            self.smask = saved_smask;
            return;
        }

        for local_y in 0..height as i32 {
            let dest_y = dst_y as i32 + local_y;
            for local_x in 0..width as i32 {
                let dest_x = dst_x as i32 + local_x;
                let sp = src.get_pixel(local_x, local_y);
                if sp[3] == 0 {
                    continue;
                }
                let clip_alpha = self
                    .clip
                    .as_ref()
                    .map_or(1.0, |clip| clip.opacity(dest_x, dest_y));
                if clip_alpha <= 0.0 {
                    continue;
                }
                let smask_alpha = soft_mask.map_or(1.0, |mask| mask.get(dest_x, dest_y));
                let eff_a =
                    (sp[3] as f32 / 255.0 * alpha * smask_alpha * clip_alpha).clamp(0.0, 1.0);
                if eff_a <= 0.0 {
                    continue;
                }
                if let Some(idx) = self.pixel_index(dest_x, dest_y) {
                    let dst_rgb = [
                        self.data[idx] as f32 / 255.0,
                        self.data[idx + 1] as f32 / 255.0,
                        self.data[idx + 2] as f32 / 255.0,
                    ];
                    let dst_a = self.data[idx + 3] as f32 / 255.0;
                    let src_rgb = [
                        sp[0] as f32 / 255.0,
                        sp[1] as f32 / 255.0,
                        sp[2] as f32 / 255.0,
                    ];
                    let blended_rgb = blend_backdrop_rgb(blend_mode, src_rgb, dst_rgb);
                    let (out_rgb, out_a) =
                        composite_source_over(blended_rgb, eff_a, dst_rgb, dst_a, blend_mode);
                    self.data[idx] = (out_rgb[0] * 255.0).round().clamp(0.0, 255.0) as u8;
                    self.data[idx + 1] = (out_rgb[1] * 255.0).round().clamp(0.0, 255.0) as u8;
                    self.data[idx + 2] = (out_rgb[2] * 255.0).round().clamp(0.0, 255.0) as u8;
                    self.data[idx + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    /// Remove a backdrop's contribution from this buffer (a non-isolated
    /// transparency-group result that was seeded with `backdrop`).
    ///
    /// A non-isolated group is rendered starting from a copy of its backdrop so
    /// that blend modes inside the group can interact with what is already
    /// painted. Before the group is composited back onto that same backdrop, the
    /// backdrop's own contribution must be removed so it is not counted twice.
    /// Per PDF 32000-1 Â§11.4.8, with group result `(Cn, Î±n)` and initial
    /// backdrop `(C0, Î±0)`:
    ///
    /// ```text
    /// C = Cn + (Cn - C0) * (Î±0 / Î±n - Î±0)   (per color channel, when Î±n > 0)
    /// ```
    ///
    /// The result alpha is left as `Î±n`; compositing back with source-over then
    /// reproduces the correct final image.
    pub fn remove_backdrop(&mut self, backdrop: &PixelBuffer) {
        let w = self.width.min(backdrop.width) as i32;
        let h = self.height.min(backdrop.height) as i32;
        for y in 0..h {
            for x in 0..w {
                let Some(idx) = self.pixel_index(x, y) else {
                    continue;
                };
                let an = self.data[idx + 3] as f32 / 255.0;
                if an <= 1e-6 {
                    continue;
                }
                let bd = backdrop.get_pixel(x, y);
                let a0 = bd[3] as f32 / 255.0;
                if a0 <= 1e-6 {
                    // No backdrop here: nothing to remove.
                    continue;
                }
                let factor = a0 / an - a0;
                for (c, &c0_byte) in bd.iter().take(3).enumerate() {
                    let cn = self.data[idx + c] as f32 / 255.0;
                    let c0 = c0_byte as f32 / 255.0;
                    let out = cn + (cn - c0) * factor;
                    self.data[idx + c] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }

    /// Composite a source buffer onto this one using "knockout" semantics:
    /// each source pixel with non-zero alpha *replaces* the destination pixel
    /// (scaled by `group_alpha`/`soft_mask`) rather than blending with it. Used
    /// for knockout transparency groups (/K true), where group elements knock
    /// out the group backdrop instead of accumulating.
    pub fn knockout_from(
        &mut self,
        src: &PixelBuffer,
        group_alpha: f32,
        soft_mask: Option<&AlphaMask>,
    ) {
        let alpha = group_alpha.clamp(0.0, 1.0);
        for y in 0..self.height.min(src.height) as i32 {
            for x in 0..self.width.min(src.width) as i32 {
                let clip_alpha = if let Some(clip) = &self.clip {
                    let clip_alpha = clip.opacity(x, y);
                    if clip_alpha <= 0.0 {
                        continue;
                    }
                    clip_alpha
                } else {
                    1.0
                };
                let sp = src.get_pixel(x, y);
                if sp[3] == 0 {
                    continue;
                }
                let mask = soft_mask.map_or(1.0, |m| m.get(x, y));
                let eff = (sp[3] as f32 / 255.0 * alpha * mask * clip_alpha).clamp(0.0, 1.0);
                if let Some(idx) = self.pixel_index(x, y) {
                    self.data[idx] = sp[0];
                    self.data[idx + 1] = sp[1];
                    self.data[idx + 2] = sp[2];
                    self.data[idx + 3] = (eff * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

fn flatten_compat_onto_opaque_background(data: &mut [u8], background: PixelColor) {
    if flatten_compat_onto_opaque_background_wide(data, background) {
        return;
    }
    for chunk in data.chunks_exact_mut(4) {
        let src_a = chunk[3];
        if src_a == 255 {
            continue;
        }
        if src_a == 0 {
            chunk.copy_from_slice(&background);
            continue;
        }
        let alpha = src_a as u16;
        let inv_alpha = 255_u16.saturating_sub(alpha);
        for channel in 0..3 {
            let src = chunk[channel] as u16;
            let bg = background[channel] as u16;
            chunk[channel] = ((src * alpha + bg * inv_alpha + 127) / 255) as u8;
        }
        chunk[3] = 255;
    }
}

fn flatten_compat_onto_opaque_background_wide(data: &mut [u8], background: PixelColor) -> bool {
    if data.len() < 8 {
        return false;
    }
    let bg = wide::u16x8::new([
        u16::from(background[0]),
        u16::from(background[1]),
        u16::from(background[2]),
        255,
        u16::from(background[0]),
        u16::from(background[1]),
        u16::from(background[2]),
        255,
    ]);
    let round = wide::u16x8::splat(128);
    let mut offset = 0usize;
    let simd_len = (data.len() / 8) * 8;
    while offset < simd_len {
        let sa0 = u16::from(data[offset + 3]);
        let sa1 = u16::from(data[offset + 7]);
        if sa0 == 255 && sa1 == 255 {
            offset += 8;
            continue;
        }
        if sa0 == 0 && sa1 == 0 {
            data[offset..offset + 4].copy_from_slice(&background);
            data[offset + 4..offset + 8].copy_from_slice(&background);
            offset += 8;
            continue;
        }
        let src = wide::u16x8::new([
            u16::from(data[offset]),
            u16::from(data[offset + 1]),
            u16::from(data[offset + 2]),
            255,
            u16::from(data[offset + 4]),
            u16::from(data[offset + 5]),
            u16::from(data[offset + 6]),
            255,
        ]);
        let inv0 = 255_u16.saturating_sub(sa0);
        let inv1 = 255_u16.saturating_sub(sa1);
        let alpha = wide::u16x8::new([sa0, sa0, sa0, 255, sa1, sa1, sa1, 255]);
        let inv_alpha = wide::u16x8::new([inv0, inv0, inv0, 0, inv1, inv1, inv1, 0]);
        let mixed = src * alpha + bg * inv_alpha + round;
        let out = ((mixed + (mixed >> 8_u32)) >> 8_u32).to_array();
        for lane in 0..8 {
            data[offset + lane] = out[lane].min(255) as u8;
        }
        data[offset + 3] = 255;
        data[offset + 7] = 255;
        offset += 8;
    }
    if offset < data.len() {
        flatten_compat_onto_opaque_background_scalar(&mut data[offset..], background);
    }
    true
}

fn flatten_compat_onto_opaque_background_scalar(data: &mut [u8], background: PixelColor) {
    for chunk in data.chunks_exact_mut(4) {
        let src_a = chunk[3];
        if src_a == 255 {
            continue;
        }
        if src_a == 0 {
            chunk.copy_from_slice(&background);
            continue;
        }
        let alpha = src_a as u16;
        let inv_alpha = 255_u16.saturating_sub(alpha);
        for channel in 0..3 {
            let src = chunk[channel] as u16;
            let bg = background[channel] as u16;
            chunk[channel] = ((src * alpha + bg * inv_alpha + 127) / 255) as u8;
        }
        chunk[3] = 255;
    }
}

fn composite_normal_compat_row_run(
    dst: &mut [u8],
    dst_width: u32,
    row: i32,
    x_start: i32,
    x_end_exclusive: i32,
    src: &PixelBuffer,
    group_alpha: f32,
) {
    if row < 0 || x_start < 0 || x_end_exclusive <= x_start {
        return;
    }
    if row >= src.height as i32 || x_start >= src.width as i32 {
        return;
    }
    let x_end_exclusive = x_end_exclusive.min(src.width as i32);
    if x_end_exclusive <= x_start {
        return;
    }
    let Some(dst_start) = (row as usize)
        .checked_mul(dst_width as usize)
        .and_then(|row_base| row_base.checked_add(x_start as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(src_start) = (row as usize)
        .checked_mul(src.width as usize)
        .and_then(|row_base| row_base.checked_add(x_start as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let len = (x_end_exclusive - x_start) as usize * 4;
    let Some(dst_row) = dst.get_mut(dst_start..dst_start + len) else {
        return;
    };
    let Some(src_row) = src.data.get(src_start..src_start + len) else {
        return;
    };
    composite_normal_compat_row(dst_row, src_row, group_alpha);
}

fn blend_alpha_mask_run_normal(
    data: &mut [u8],
    width: u32,
    row: i32,
    run_x0: i32,
    mask_alpha: &[u8],
    color: PixelColor,
) {
    if row >= 0 && run_x0 >= 0 && !mask_alpha.is_empty() {
        let Some(start) = (row as usize)
            .checked_mul(width as usize)
            .and_then(|row_base| row_base.checked_add(run_x0 as usize))
            .and_then(|pixel| pixel.checked_mul(4))
        else {
            return;
        };
        let Some(end) = start.checked_add(mask_alpha.len().saturating_mul(4)) else {
            return;
        };
        if let Some(slice) = data.get_mut(start..end) {
            if blend_alpha_mask_run_normal_opaque_dst_wide(slice, mask_alpha, color) {
                return;
            }
        }
    }

    let mut idx = 0usize;
    let mut col = run_x0;
    while idx < mask_alpha.len() {
        let alpha = mask_alpha[idx];
        if alpha == 0 {
            idx += 1;
            col += 1;
            continue;
        }
        let effective_alpha = ((u16::from(color[3]) * u16::from(alpha) + 127) / 255) as u8;
        if effective_alpha == 0 {
            idx += 1;
            col += 1;
            continue;
        }
        let mut len = 1usize;
        while idx + len < mask_alpha.len() {
            let next_alpha = mask_alpha[idx + len];
            let next_effective = ((u16::from(color[3]) * u16::from(next_alpha) + 127) / 255) as u8;
            if next_effective != effective_alpha {
                break;
            }
            len += 1;
        }
        let run_end = col.saturating_add(len as i32);
        if effective_alpha == 255 {
            fill_opaque_run(data, width, row, col, run_end, color);
        } else {
            let mut run_color = color;
            run_color[3] = effective_alpha;
            blend_normal_compat_run(data, width, row, col, run_end, run_color);
        }
        idx += len;
        col = run_end;
    }
}

fn blend_alpha_mask_run_normal_opaque_dst_wide(
    slice: &mut [u8],
    mask_alpha: &[u8],
    color: PixelColor,
) -> bool {
    let pixels = slice.chunks_exact(4).count().min(mask_alpha.len());
    if pixels < 2 || color[3] == 0 {
        return color[3] == 0;
    }
    let slice_len = pixels.saturating_mul(4);
    if !row_alpha_is(&slice[..slice_len], 255) {
        return false;
    }

    let color_alpha = u16::from(color[3]);
    let src = wide::u16x8::new([
        u16::from(color[0]),
        u16::from(color[1]),
        u16::from(color[2]),
        255,
        u16::from(color[0]),
        u16::from(color[1]),
        u16::from(color[2]),
        255,
    ]);
    let round = wide::u16x8::splat(128);
    let simd_pixels = (pixels / 2) * 2;
    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let alpha0 = ((color_alpha * u16::from(mask_alpha[pixel]) + 127) / 255).min(255);
        let alpha1 = ((color_alpha * u16::from(mask_alpha[pixel + 1]) + 127) / 255).min(255);
        if alpha0 == 0 && alpha1 == 0 {
            pixel += 2;
            continue;
        }
        let inv0 = 255_u16.saturating_sub(alpha0);
        let inv1 = 255_u16.saturating_sub(alpha1);
        let dst = wide::u16x8::new([
            u16::from(slice[offset]),
            u16::from(slice[offset + 1]),
            u16::from(slice[offset + 2]),
            255,
            u16::from(slice[offset + 4]),
            u16::from(slice[offset + 5]),
            u16::from(slice[offset + 6]),
            255,
        ]);
        let alpha = wide::u16x8::new([alpha0, alpha0, alpha0, 255, alpha1, alpha1, alpha1, 255]);
        let inv_alpha = wide::u16x8::new([inv0, inv0, inv0, 0, inv1, inv1, inv1, 0]);
        let mixed = src * alpha + dst * inv_alpha + round;
        let out = ((mixed + (mixed >> 8_u32)) >> 8_u32).to_array();
        for lane in 0..8 {
            slice[offset + lane] = out[lane].min(255) as u8;
        }
        slice[offset + 3] = 255;
        slice[offset + 7] = 255;
        pixel += 2;
    }
    if simd_pixels < pixels {
        blend_alpha_mask_run_normal_opaque_dst_scalar(
            &mut slice[simd_pixels * 4..slice_len],
            &mask_alpha[simd_pixels..pixels],
            color,
        );
    }
    PIXEL_COMPOSITOR_WIDE_OPAQUE_DST_PIXELS.fetch_add(simd_pixels as u64, Ordering::Relaxed);
    true
}

fn blend_alpha_mask_run_normal_opaque_dst_scalar(
    slice: &mut [u8],
    mask_alpha: &[u8],
    color: PixelColor,
) {
    let pixels = slice.chunks_exact_mut(4).zip(mask_alpha.iter()).count();
    if pixels == 0 || color[3] == 0 {
        return;
    }
    let color_alpha = u16::from(color[3]);
    for (chunk, mask) in slice.chunks_exact_mut(4).zip(mask_alpha.iter().copied()) {
        let alpha = ((color_alpha * u16::from(mask) + 127) / 255).min(255);
        if alpha == 0 {
            continue;
        }
        let inv_alpha = 255_u16.saturating_sub(alpha);
        for channel in 0..3 {
            let mixed = u16::from(color[channel]) * alpha + u16::from(chunk[channel]) * inv_alpha;
            chunk[channel] = ((mixed + 128 + ((mixed + 128) >> 8)) >> 8).min(255) as u8;
        }
        chunk[3] = 255;
    }
    PIXEL_COMPOSITOR_SCALAR_OPAQUE_DST_PIXELS.fetch_add(pixels as u64, Ordering::Relaxed);
}

fn fill_opaque_run(
    data: &mut [u8],
    width: u32,
    row: i32,
    x_start: i32,
    x_end_exclusive: i32,
    color: PixelColor,
) {
    if row < 0 || x_start < 0 || x_end_exclusive <= x_start {
        return;
    }
    let Some(start) = (row as usize)
        .checked_mul(width as usize)
        .and_then(|row_base| row_base.checked_add(x_start as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(end) = (row as usize)
        .checked_mul(width as usize)
        .and_then(|row_base| row_base.checked_add(x_end_exclusive as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(slice) = data.get_mut(start..end) else {
        return;
    };
    if color.iter().all(|component| *component == color[0]) {
        slice.fill(color[0]);
        return;
    }
    if fill_opaque_run_arch(slice, color) {
        return;
    }
    let _ = fill_opaque_run_word_aligned(slice, color);
}

fn fill_opaque_run_arch(slice: &mut [u8], color: PixelColor) -> bool {
    wellfriendpdf_render_simd::fill_opaque_run(slice, color)
}

fn fill_opaque_run_word_aligned(slice: &mut [u8], color: PixelColor) -> bool {
    if slice.len() < 8 {
        for chunk in slice.chunks_exact_mut(4) {
            chunk.copy_from_slice(&color);
        }
        return slice.len() >= 4;
    }
    for chunk in slice.chunks_exact_mut(4) {
        chunk.copy_from_slice(&color);
    }
    true
}

fn blend_normal_compat_run(
    data: &mut [u8],
    width: u32,
    row: i32,
    x_start: i32,
    x_end_exclusive: i32,
    color: PixelColor,
) {
    if row < 0 || x_start < 0 || x_end_exclusive <= x_start || color[3] == 0 {
        return;
    }
    let Some(start) = (row as usize)
        .checked_mul(width as usize)
        .and_then(|row_base| row_base.checked_add(x_start as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(end) = (row as usize)
        .checked_mul(width as usize)
        .and_then(|row_base| row_base.checked_add(x_end_exclusive as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(slice) = data.get_mut(start..end) else {
        return;
    };

    if row_alpha_is(slice, 255) && blend_normal_compat_opaque_dst_wide(slice, color) {
        return;
    }

    blend_normal_compat_run_scalar(slice, color);
}

fn rect_runs_have_opaque_alpha(
    data: &[u8],
    width: u32,
    y_start: i32,
    y_end_exclusive: i32,
    x_start: i32,
    x_end_exclusive: i32,
) -> bool {
    for row in y_start..y_end_exclusive {
        if !run_alpha_is(data, width, row, x_start, x_end_exclusive, 255) {
            return false;
        }
    }
    true
}

fn clipped_runs_have_opaque_alpha(
    data: &[u8],
    width: u32,
    clip: &ClipMask,
    y_start: i32,
    y_end_exclusive: i32,
    x_start: i32,
    x_end_exclusive: i32,
) -> bool {
    for row in y_start..y_end_exclusive {
        let mut row_ok = true;
        clip.for_each_visible_run_in_span(row, x_start, x_end_exclusive, |start, end| {
            if !run_alpha_is(data, width, row, start, end, 255) {
                row_ok = false;
            }
        });
        if !row_ok {
            return false;
        }
    }
    true
}

fn write_opaque_rgb_run_to_data(
    data: &mut [u8],
    dst_width: u32,
    x_range: (i32, i32),
    y: i32,
    source_x: i32,
    rgb: &[u8],
) -> usize {
    let (x_start, x_end_exclusive) = x_range;
    if y < 0 || x_start < 0 || x_end_exclusive <= x_start {
        return 0;
    }
    let src_offset = x_start.saturating_sub(source_x) as usize * 3;
    let src_len = x_end_exclusive.saturating_sub(x_start) as usize * 3;
    let Some(src_row) = rgb.get(src_offset..src_offset + src_len) else {
        return 0;
    };
    let Some(dst_start) = (y as usize)
        .checked_mul(dst_width as usize)
        .and_then(|row| row.checked_add(x_start as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return 0;
    };
    let dst_len = x_end_exclusive.saturating_sub(x_start) as usize * 4;
    let Some(dst_row) = data.get_mut(dst_start..dst_start + dst_len) else {
        return 0;
    };
    let mut written = 0usize;
    for (out, src) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(3)) {
        out[0] = src[0];
        out[1] = src[1];
        out[2] = src[2];
        out[3] = 255;
        written = written.saturating_add(1);
    }
    written
}

fn run_alpha_is(
    data: &[u8],
    width: u32,
    row: i32,
    x_start: i32,
    x_end_exclusive: i32,
    alpha: u8,
) -> bool {
    if row < 0 || x_start < 0 || x_end_exclusive < x_start {
        return false;
    }
    if x_end_exclusive == x_start {
        return true;
    }
    let Some(start) = (row as usize)
        .checked_mul(width as usize)
        .and_then(|row_base| row_base.checked_add(x_start as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return false;
    };
    let Some(end) = (row as usize)
        .checked_mul(width as usize)
        .and_then(|row_base| row_base.checked_add(x_end_exclusive as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return false;
    };
    data.get(start..end)
        .is_some_and(|slice| row_alpha_is(slice, alpha))
}

fn blend_separable_opaque_src_over_opaque_dst_run(
    data: &mut [u8],
    width: u32,
    row: i32,
    x_start: i32,
    x_end_exclusive: i32,
    color: PixelColor,
    blend_mode: BlendMode,
) {
    if row < 0 || x_start < 0 || x_end_exclusive <= x_start {
        return;
    }
    let Some(start) = (row as usize)
        .checked_mul(width as usize)
        .and_then(|row_base| row_base.checked_add(x_start as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(end) = (row as usize)
        .checked_mul(width as usize)
        .and_then(|row_base| row_base.checked_add(x_end_exclusive as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(slice) = data.get_mut(start..end) else {
        return;
    };
    if !row_alpha_is(slice, 255) {
        return;
    }
    if blend_separable_opaque_src_over_opaque_dst_wide(slice, color, blend_mode) {
        return;
    }
    blend_separable_opaque_src_over_opaque_dst_scalar(slice, color, blend_mode);
}

fn blend_separable_opaque_src_over_opaque_dst_scalar(
    slice: &mut [u8],
    color: PixelColor,
    blend_mode: BlendMode,
) {
    PIXEL_COMPOSITOR_SCALAR_SEPARABLE_BLEND_PIXELS
        .fetch_add((slice.len() / 4) as u64, Ordering::Relaxed);
    let src = [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ];
    let to_byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    for chunk in slice.chunks_exact_mut(4) {
        let dst = [
            chunk[0] as f32 / 255.0,
            chunk[1] as f32 / 255.0,
            chunk[2] as f32 / 255.0,
        ];
        chunk[0] = to_byte(blend_mode.blend_channel(src[0], dst[0]));
        chunk[1] = to_byte(blend_mode.blend_channel(src[1], dst[1]));
        chunk[2] = to_byte(blend_mode.blend_channel(src[2], dst[2]));
        chunk[3] = 255;
    }
}

fn blend_separable_opaque_src_over_opaque_dst_wide(
    slice: &mut [u8],
    color: PixelColor,
    blend_mode: BlendMode,
) -> bool {
    if slice.len() < 8 {
        return false;
    }
    if !matches!(
        blend_mode,
        BlendMode::Multiply | BlendMode::Screen | BlendMode::Darken | BlendMode::Lighten
    ) {
        return false;
    }
    let simd_len = (slice.len() / 8) * 8;
    if simd_len == 0 {
        return false;
    }
    let src = wide::u16x8::new([
        u16::from(color[0]),
        u16::from(color[1]),
        u16::from(color[2]),
        255,
        u16::from(color[0]),
        u16::from(color[1]),
        u16::from(color[2]),
        255,
    ]);
    let mut offset = 0usize;
    while offset < simd_len {
        let dst = wide::u16x8::new([
            u16::from(slice[offset]),
            u16::from(slice[offset + 1]),
            u16::from(slice[offset + 2]),
            255,
            u16::from(slice[offset + 4]),
            u16::from(slice[offset + 5]),
            u16::from(slice[offset + 6]),
            255,
        ]);
        let out = match blend_mode {
            BlendMode::Multiply => div255_round_wide(src * dst),
            BlendMode::Screen => src + dst - div255_round_wide(src * dst),
            BlendMode::Darken => min_u16x8(src, dst),
            BlendMode::Lighten => max_u16x8(src, dst),
            _ => unreachable!("blend mode filtered above"),
        }
        .to_array();
        for lane in 0..8 {
            slice[offset + lane] = out[lane].min(255) as u8;
        }
        slice[offset + 3] = 255;
        slice[offset + 7] = 255;
        offset += 8;
    }
    if offset < slice.len() {
        blend_separable_opaque_src_over_opaque_dst_scalar(&mut slice[offset..], color, blend_mode);
    }
    PIXEL_COMPOSITOR_WIDE_SEPARABLE_BLEND_PIXELS
        .fetch_add((simd_len / 4) as u64, Ordering::Relaxed);
    true
}

#[inline]
fn div255_round_wide(value: wide::u16x8) -> wide::u16x8 {
    let round = wide::u16x8::splat(128);
    let adjusted = value + round;
    (adjusted + (adjusted >> 8_u32)) >> 8_u32
}

#[inline]
fn min_u16x8(a: wide::u16x8, b: wide::u16x8) -> wide::u16x8 {
    let aa = a.to_array();
    let bb = b.to_array();
    wide::u16x8::new([
        aa[0].min(bb[0]),
        aa[1].min(bb[1]),
        aa[2].min(bb[2]),
        aa[3].min(bb[3]),
        aa[4].min(bb[4]),
        aa[5].min(bb[5]),
        aa[6].min(bb[6]),
        aa[7].min(bb[7]),
    ])
}

#[inline]
fn max_u16x8(a: wide::u16x8, b: wide::u16x8) -> wide::u16x8 {
    let aa = a.to_array();
    let bb = b.to_array();
    wide::u16x8::new([
        aa[0].max(bb[0]),
        aa[1].max(bb[1]),
        aa[2].max(bb[2]),
        aa[3].max(bb[3]),
        aa[4].max(bb[4]),
        aa[5].max(bb[5]),
        aa[6].max(bb[6]),
        aa[7].max(bb[7]),
    ])
}

fn blend_normal_compat_run_scalar(slice: &mut [u8], color: PixelColor) {
    PIXEL_COMPOSITOR_SCALAR_SOLID_COLOR_PIXELS
        .fetch_add((slice.len() / 4) as u64, Ordering::Relaxed);
    let src_a = color[3] as f32 / 255.0;
    let src_rgb = [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    ];
    let to_byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    for chunk in slice.chunks_exact_mut(4) {
        let dst_a = chunk[3] as f32 / 255.0;
        let out_a = src_a + dst_a * (1.0 - src_a);
        if out_a < 1e-6 {
            chunk.copy_from_slice(&TRANSPARENT);
            continue;
        }
        let inv_alpha = 1.0 / out_a;
        for channel in 0..3 {
            let dst = chunk[channel] as f32 / 255.0;
            let out = (src_rgb[channel] * src_a + dst * dst_a * (1.0 - src_a)) * inv_alpha;
            chunk[channel] = to_byte(out);
        }
        chunk[3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
    }
}

fn blend_normal_compat_opaque_dst_wide(slice: &mut [u8], color: PixelColor) -> bool {
    if color[3] == 0 {
        return true;
    }
    if color[3] == 255 {
        let _ = fill_opaque_run_arch(slice, color);
        return true;
    }
    if blend_normal_compat_opaque_dst_arch(slice, color) {
        return true;
    }
    blend_normal_compat_opaque_dst_portable(slice, color)
}

fn blend_normal_compat_opaque_dst_portable(slice: &mut [u8], color: PixelColor) -> bool {
    if slice.len() < 8 {
        return false;
    }

    let alpha = u16::from(color[3]);
    let inv_alpha = 255_u16.saturating_sub(alpha);
    let src = wide::u16x8::new([
        u16::from(color[0]),
        u16::from(color[1]),
        u16::from(color[2]),
        255,
        u16::from(color[0]),
        u16::from(color[1]),
        u16::from(color[2]),
        255,
    ]);
    let alpha_v = wide::u16x8::new([alpha, alpha, alpha, 255, alpha, alpha, alpha, 255]);
    let inv_v = wide::u16x8::new([
        inv_alpha, inv_alpha, inv_alpha, 0, inv_alpha, inv_alpha, inv_alpha, 0,
    ]);
    let round = wide::u16x8::splat(128);
    let mut offset = 0usize;
    let simd_len = (slice.len() / 8) * 8;
    while offset < simd_len {
        let dst = wide::u16x8::new([
            u16::from(slice[offset]),
            u16::from(slice[offset + 1]),
            u16::from(slice[offset + 2]),
            u16::from(slice[offset + 3]),
            u16::from(slice[offset + 4]),
            u16::from(slice[offset + 5]),
            u16::from(slice[offset + 6]),
            u16::from(slice[offset + 7]),
        ]);
        let mixed = src * alpha_v + dst * inv_v + round;
        let out = ((mixed + (mixed >> 8_u32)) >> 8_u32).to_array();
        for lane in 0..8 {
            slice[offset + lane] = out[lane].min(255) as u8;
        }
        slice[offset + 3] = 255;
        slice[offset + 7] = 255;
        offset += 8;
    }

    if offset < slice.len() {
        blend_normal_compat_run_scalar(&mut slice[offset..], color);
    }
    PIXEL_COMPOSITOR_WIDE_SOLID_COLOR_PIXELS.fetch_add((simd_len / 4) as u64, Ordering::Relaxed);
    true
}

fn blend_normal_compat_opaque_dst_arch(slice: &mut [u8], color: PixelColor) -> bool {
    wellfriendpdf_render_simd::blend_normal_opaque_destination(slice, color)
}

fn composite_normal_compat_buffer_unclipped(
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    src: &PixelBuffer,
    group_alpha: f32,
) {
    let width = dst_width.min(src.width) as usize;
    let height = dst_height.min(src.height) as usize;
    if width == 0 || height == 0 {
        return;
    }
    let dst_stride = dst_width as usize * 4;
    let src_stride = src.width as usize * 4;
    let alpha = group_alpha.clamp(0.0, 1.0);
    for row in 0..height {
        let Some(dst_row) = dst.get_mut(row * dst_stride..row * dst_stride + width * 4) else {
            return;
        };
        let Some(src_row) = src.data.get(row * src_stride..row * src_stride + width * 4) else {
            return;
        };
        if alpha >= 1.0 {
            match row_alpha_class(src_row) {
                RowAlphaClass::AllTransparent => continue,
                RowAlphaClass::AllOpaque => {
                    dst_row.copy_from_slice(src_row);
                    continue;
                }
                RowAlphaClass::Mixed => {}
            }
        }
        composite_normal_compat_row(dst_row, src_row, alpha);
    }
}

fn composite_normal_compat_buffer_unclipped_soft_mask(
    dst: &mut [u8],
    dst_width: u32,
    dst_height: u32,
    src: &PixelBuffer,
    group_alpha: f32,
    soft_mask: &AlphaMask,
) {
    let width = dst_width.min(src.width).min(soft_mask.width) as usize;
    let height = dst_height.min(src.height).min(soft_mask.height) as usize;
    if width == 0 || height == 0 {
        return;
    }
    let dst_stride = dst_width as usize * 4;
    let src_stride = src.width as usize * 4;
    let mask_stride = soft_mask.width as usize;
    let alpha = (group_alpha.clamp(0.0, 1.0) * 255.0).round() as u16;
    if alpha == 0 {
        return;
    }
    for row in 0..height {
        let Some(dst_row) = dst.get_mut(row * dst_stride..row * dst_stride + width * 4) else {
            return;
        };
        let Some(src_row) = src.data.get(row * src_stride..row * src_stride + width * 4) else {
            return;
        };
        let Some(mask_row) = soft_mask
            .data
            .get(row * mask_stride..row * mask_stride + width)
        else {
            return;
        };
        if row_alpha_class(src_row) == RowAlphaClass::AllTransparent {
            continue;
        }
        match mask_row_class(mask_row) {
            MaskRowClass::AllTransparent => continue,
            MaskRowClass::AllOpaque => composite_normal_compat_row(dst_row, src_row, group_alpha),
            MaskRowClass::Mixed if row_alpha_is(dst_row, 255) => {
                composite_normal_compat_row_soft_mask_opaque_dst(dst_row, src_row, mask_row, alpha);
            }
            MaskRowClass::Mixed => {
                composite_normal_compat_row_soft_mask_scalar(dst_row, src_row, mask_row, alpha);
            }
        }
    }
}

fn composite_normal_compat_row_run_soft_mask(
    dst: &mut [u8],
    dst_width: u32,
    run: (i32, i32, i32),
    src: &PixelBuffer,
    group_alpha: f32,
    soft_mask: &AlphaMask,
) {
    let (row, x_start, x_end_exclusive) = run;
    if row < 0 || x_start < 0 || x_end_exclusive <= x_start {
        return;
    }
    if row >= src.height as i32 || row >= soft_mask.height as i32 || x_start >= src.width as i32 {
        return;
    }
    let x_end_exclusive = x_end_exclusive
        .min(src.width as i32)
        .min(soft_mask.width as i32);
    if x_end_exclusive <= x_start {
        return;
    }
    let Some(dst_start) = (row as usize)
        .checked_mul(dst_width as usize)
        .and_then(|row_base| row_base.checked_add(x_start as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(src_start) = (row as usize)
        .checked_mul(src.width as usize)
        .and_then(|row_base| row_base.checked_add(x_start as usize))
        .and_then(|pixel| pixel.checked_mul(4))
    else {
        return;
    };
    let Some(mask_start) = (row as usize)
        .checked_mul(soft_mask.width as usize)
        .and_then(|row_base| row_base.checked_add(x_start as usize))
    else {
        return;
    };
    let pixels = (x_end_exclusive - x_start) as usize;
    let len = pixels * 4;
    let Some(dst_row) = dst.get_mut(dst_start..dst_start + len) else {
        return;
    };
    let Some(src_row) = src.data.get(src_start..src_start + len) else {
        return;
    };
    let Some(mask_row) = soft_mask.data.get(mask_start..mask_start + pixels) else {
        return;
    };
    let alpha = (group_alpha.clamp(0.0, 1.0) * 255.0).round() as u16;
    if alpha == 0 || row_alpha_class(src_row) == RowAlphaClass::AllTransparent {
        return;
    }
    match mask_row_class(mask_row) {
        MaskRowClass::AllTransparent => {}
        MaskRowClass::AllOpaque => composite_normal_compat_row(dst_row, src_row, group_alpha),
        MaskRowClass::Mixed if row_alpha_is(dst_row, 255) => {
            composite_normal_compat_row_soft_mask_opaque_dst(dst_row, src_row, mask_row, alpha);
        }
        MaskRowClass::Mixed => {
            composite_normal_compat_row_soft_mask_scalar(dst_row, src_row, mask_row, alpha);
        }
    }
}

fn composite_normal_compat_row_soft_mask_opaque_dst(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) {
    if composite_normal_compat_row_soft_mask_opaque_dst_wide(
        dst_row,
        src_row,
        mask_row,
        group_alpha_255,
    ) {
        return;
    }
    composite_normal_compat_row_soft_mask_opaque_dst_scalar(
        dst_row,
        src_row,
        mask_row,
        group_alpha_255,
    );
}

fn composite_normal_compat_row_soft_mask_opaque_dst_scalar(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) {
    PIXEL_COMPOSITOR_SOFT_MASK_OPAQUE_DST_PIXELS
        .fetch_add(mask_row.len() as u64, Ordering::Relaxed);
    PIXEL_COMPOSITOR_SCALAR_SOFT_MASK_OPAQUE_DST_PIXELS
        .fetch_add(mask_row.len() as u64, Ordering::Relaxed);
    for ((d, s), &mask) in dst_row
        .chunks_exact_mut(4)
        .zip(src_row.chunks_exact(4))
        .zip(mask_row.iter())
    {
        if s[3] == 0 || mask == 0 {
            continue;
        }
        let eff =
            (u32::from(s[3]) * u32::from(mask) * u32::from(group_alpha_255) + 32_512) / 65_025;
        if eff == 0 {
            continue;
        }
        if eff >= 255 {
            d.copy_from_slice(&[s[0], s[1], s[2], 255]);
            continue;
        }
        let eff = eff as u16;
        let inv = 255_u16.saturating_sub(eff);
        for channel in 0..3 {
            d[channel] =
                ((u16::from(s[channel]) * eff + u16::from(d[channel]) * inv + 127) / 255) as u8;
        }
        d[3] = 255;
    }
}

fn composite_normal_compat_row_soft_mask_opaque_dst_wide(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    if group_alpha_255 != 255 || mask_row.len() < 2 {
        return false;
    }
    let pixels = dst_row
        .chunks_exact(4)
        .zip(src_row.chunks_exact(4))
        .count()
        .min(mask_row.len());
    if composite_normal_compat_row_soft_mask_opaque_dst_arch(
        dst_row,
        src_row,
        mask_row,
        group_alpha_255,
    ) {
        PIXEL_COMPOSITOR_SOFT_MASK_OPAQUE_DST_PIXELS.fetch_add(pixels as u64, Ordering::Relaxed);
        PIXEL_COMPOSITOR_WIDE_SOFT_MASK_OPAQUE_DST_PIXELS
            .fetch_add(pixels as u64, Ordering::Relaxed);
        return true;
    }
    let simd_pixels = (pixels / 2) * 2;
    if simd_pixels == 0 {
        return false;
    }
    let round = wide::u16x8::splat(128);
    for pixel in (0..simd_pixels).step_by(2) {
        let dst_offset = pixel * 4;
        let eff0 =
            div255_round_u16(u16::from(src_row[dst_offset + 3]) * u16::from(mask_row[pixel]));
        let eff1 =
            div255_round_u16(u16::from(src_row[dst_offset + 7]) * u16::from(mask_row[pixel + 1]));
        let inv0 = 255_u16.saturating_sub(eff0);
        let inv1 = 255_u16.saturating_sub(eff1);
        let src = wide::u16x8::new([
            u16::from(src_row[dst_offset]),
            u16::from(src_row[dst_offset + 1]),
            u16::from(src_row[dst_offset + 2]),
            255,
            u16::from(src_row[dst_offset + 4]),
            u16::from(src_row[dst_offset + 5]),
            u16::from(src_row[dst_offset + 6]),
            255,
        ]);
        let dst = wide::u16x8::new([
            u16::from(dst_row[dst_offset]),
            u16::from(dst_row[dst_offset + 1]),
            u16::from(dst_row[dst_offset + 2]),
            255,
            u16::from(dst_row[dst_offset + 4]),
            u16::from(dst_row[dst_offset + 5]),
            u16::from(dst_row[dst_offset + 6]),
            255,
        ]);
        let eff = wide::u16x8::new([eff0, eff0, eff0, 255, eff1, eff1, eff1, 255]);
        let inv = wide::u16x8::new([inv0, inv0, inv0, 0, inv1, inv1, inv1, 0]);
        let mixed = src * eff + dst * inv + round;
        let out = ((mixed + (mixed >> 8_u32)) >> 8_u32).to_array();
        for lane in 0..8 {
            dst_row[dst_offset + lane] = out[lane].min(255) as u8;
        }
        dst_row[dst_offset + 3] = 255;
        dst_row[dst_offset + 7] = 255;
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        composite_normal_compat_row_soft_mask_opaque_dst_scalar(
            &mut dst_row[offset..pixels * 4],
            &src_row[offset..pixels * 4],
            &mask_row[simd_pixels..pixels],
            group_alpha_255,
        );
    }
    PIXEL_COMPOSITOR_SOFT_MASK_OPAQUE_DST_PIXELS.fetch_add(simd_pixels as u64, Ordering::Relaxed);
    PIXEL_COMPOSITOR_WIDE_SOFT_MASK_OPAQUE_DST_PIXELS
        .fetch_add(simd_pixels as u64, Ordering::Relaxed);
    true
}

fn composite_normal_compat_row_soft_mask_opaque_dst_arch(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    wellfriendpdf_render_simd::composite_soft_mask_opaque_destination(
        dst_row,
        src_row,
        mask_row,
        group_alpha_255,
    )
}

#[inline]
fn div255_round_u16(value: u16) -> u16 {
    let adjusted = value.saturating_add(128);
    (adjusted + (adjusted >> 8)) >> 8
}

fn composite_normal_compat_row_soft_mask_scalar(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) {
    PIXEL_COMPOSITOR_SOFT_MASK_GENERAL_PIXELS.fetch_add(mask_row.len() as u64, Ordering::Relaxed);
    for ((d, s), &mask) in dst_row
        .chunks_exact_mut(4)
        .zip(src_row.chunks_exact(4))
        .zip(mask_row.iter())
    {
        if s[3] == 0 || mask == 0 {
            continue;
        }
        let src_a =
            (u32::from(s[3]) * u32::from(mask) * u32::from(group_alpha_255) + 32_512) / 65_025;
        if src_a == 0 {
            continue;
        }
        if src_a >= 255 {
            d.copy_from_slice(&[s[0], s[1], s[2], 255]);
            continue;
        }
        let src_a = src_a as f32 / 255.0;
        let dst_a = d[3] as f32 / 255.0;
        let out_a = src_a + dst_a * (1.0 - src_a);
        if out_a < 1e-6 {
            d.copy_from_slice(&TRANSPARENT);
            continue;
        }
        let inv_alpha = 1.0 / out_a;
        for channel in 0..3 {
            let src_channel = s[channel] as f32 / 255.0;
            let dst_channel = d[channel] as f32 / 255.0;
            let out = (src_channel * src_a + dst_channel * dst_a * (1.0 - src_a)) * inv_alpha;
            d[channel] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        d[3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
    }
}

fn composite_normal_compat_row(dst_row: &mut [u8], src_row: &[u8], group_alpha: f32) {
    let alpha = group_alpha.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let src_alpha_class = row_alpha_class(src_row);
    if src_alpha_class == RowAlphaClass::AllTransparent {
        return;
    }
    if alpha >= 1.0 && src_alpha_class == RowAlphaClass::AllOpaque {
        let len = dst_row.len().min(src_row.len());
        dst_row[..len].copy_from_slice(&src_row[..len]);
        return;
    }
    if row_alpha_is(dst_row, 255) {
        if alpha < 1.0
            && src_alpha_class == RowAlphaClass::AllOpaque
            && composite_normal_compat_row_opaque_src_dst_uniform_alpha_wide(
                dst_row, src_row, alpha,
            )
        {
            return;
        }
        if alpha >= 1.0 && composite_normal_compat_row_opaque_dst_wide(dst_row, src_row) {
            return;
        }
    }
    let to_byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    PIXEL_COMPOSITOR_SCALAR_GENERAL_PIXELS.fetch_add(
        dst_row.chunks_exact(4).zip(src_row.chunks_exact(4)).count() as u64,
        Ordering::Relaxed,
    );
    for (d, s) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
        if s[3] == 0 {
            continue;
        }
        let src_a = (s[3] as f32 / 255.0 * alpha).clamp(0.0, 1.0);
        if src_a <= 0.0 {
            continue;
        }
        if src_a >= 1.0 {
            d.copy_from_slice(&[s[0], s[1], s[2], 255]);
            continue;
        }
        let dst_a = d[3] as f32 / 255.0;
        let out_a = src_a + dst_a * (1.0 - src_a);
        if out_a < 1e-6 {
            d.copy_from_slice(&TRANSPARENT);
            continue;
        }
        let inv_alpha = 1.0 / out_a;
        for channel in 0..3 {
            let src_channel = s[channel] as f32 / 255.0;
            let dst_channel = d[channel] as f32 / 255.0;
            let out = (src_channel * src_a + dst_channel * dst_a * (1.0 - src_a)) * inv_alpha;
            d[channel] = to_byte(out);
        }
        d[3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
    }
}

fn composite_normal_compat_row_opaque_dst_wide(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    let len = dst_row.len().min(src_row.len());
    if len < 8 {
        return false;
    }
    if composite_normal_compat_row_opaque_dst_arch(dst_row, src_row) {
        PIXEL_COMPOSITOR_WIDE_OPAQUE_DST_PIXELS.fetch_add((len / 4) as u64, Ordering::Relaxed);
        return true;
    }
    let simd_len = (len / 8) * 8;
    let round = wide::u16x8::splat(128);
    let mut offset = 0usize;
    while offset < simd_len {
        let sa0 = u16::from(src_row[offset + 3]);
        let sa1 = u16::from(src_row[offset + 7]);
        let inv0 = 255_u16.saturating_sub(sa0);
        let inv1 = 255_u16.saturating_sub(sa1);
        let src = wide::u16x8::new([
            u16::from(src_row[offset]),
            u16::from(src_row[offset + 1]),
            u16::from(src_row[offset + 2]),
            255,
            u16::from(src_row[offset + 4]),
            u16::from(src_row[offset + 5]),
            u16::from(src_row[offset + 6]),
            255,
        ]);
        let dst = wide::u16x8::new([
            u16::from(dst_row[offset]),
            u16::from(dst_row[offset + 1]),
            u16::from(dst_row[offset + 2]),
            255,
            u16::from(dst_row[offset + 4]),
            u16::from(dst_row[offset + 5]),
            u16::from(dst_row[offset + 6]),
            255,
        ]);
        let alpha = wide::u16x8::new([sa0, sa0, sa0, 255, sa1, sa1, sa1, 255]);
        let inv_alpha = wide::u16x8::new([inv0, inv0, inv0, 0, inv1, inv1, inv1, 0]);
        let mixed = src * alpha + dst * inv_alpha + round;
        let out = ((mixed + (mixed >> 8_u32)) >> 8_u32).to_array();
        for lane in 0..8 {
            dst_row[offset + lane] = out[lane].min(255) as u8;
        }
        dst_row[offset + 3] = 255;
        dst_row[offset + 7] = 255;
        offset += 8;
    }
    if offset < len {
        composite_normal_compat_row_scalar_opaque_dst(
            &mut dst_row[offset..len],
            &src_row[offset..len],
        );
    }
    PIXEL_COMPOSITOR_WIDE_OPAQUE_DST_PIXELS.fetch_add((simd_len / 4) as u64, Ordering::Relaxed);
    true
}

fn composite_normal_compat_row_opaque_dst_arch(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    wellfriendpdf_render_simd::composite_normal_opaque_destination(dst_row, src_row)
}

fn composite_normal_compat_row_opaque_src_dst_uniform_alpha_wide(
    dst_row: &mut [u8],
    src_row: &[u8],
    group_alpha: f32,
) -> bool {
    let len = dst_row.len().min(src_row.len());
    if len < 8 {
        return false;
    }
    let alpha = (group_alpha.clamp(0.0, 1.0) * 255.0).round() as u16;
    if alpha == 0 {
        return true;
    }
    if alpha == 255 {
        dst_row[..len].copy_from_slice(&src_row[..len]);
        return true;
    }
    let inv_alpha = 255_u16.saturating_sub(alpha);
    let alpha_v = wide::u16x8::new([alpha, alpha, alpha, 255, alpha, alpha, alpha, 255]);
    let inv_v = wide::u16x8::new([
        inv_alpha, inv_alpha, inv_alpha, 0, inv_alpha, inv_alpha, inv_alpha, 0,
    ]);
    let round = wide::u16x8::splat(128);
    let mut offset = 0usize;
    let simd_len = (len / 8) * 8;
    while offset < simd_len {
        let src = wide::u16x8::new([
            u16::from(src_row[offset]),
            u16::from(src_row[offset + 1]),
            u16::from(src_row[offset + 2]),
            255,
            u16::from(src_row[offset + 4]),
            u16::from(src_row[offset + 5]),
            u16::from(src_row[offset + 6]),
            255,
        ]);
        let dst = wide::u16x8::new([
            u16::from(dst_row[offset]),
            u16::from(dst_row[offset + 1]),
            u16::from(dst_row[offset + 2]),
            255,
            u16::from(dst_row[offset + 4]),
            u16::from(dst_row[offset + 5]),
            u16::from(dst_row[offset + 6]),
            255,
        ]);
        let mixed = src * alpha_v + dst * inv_v + round;
        let out = ((mixed + (mixed >> 8_u32)) >> 8_u32).to_array();
        for lane in 0..8 {
            dst_row[offset + lane] = out[lane].min(255) as u8;
        }
        dst_row[offset + 3] = 255;
        dst_row[offset + 7] = 255;
        offset += 8;
    }
    if offset < len {
        composite_normal_compat_row_scalar_opaque_src_dst_uniform_alpha(
            &mut dst_row[offset..len],
            &src_row[offset..len],
            alpha,
        );
    }
    PIXEL_COMPOSITOR_WIDE_UNIFORM_ALPHA_PIXELS.fetch_add((simd_len / 4) as u64, Ordering::Relaxed);
    true
}

fn composite_normal_compat_row_scalar_opaque_src_dst_uniform_alpha(
    dst_row: &mut [u8],
    src_row: &[u8],
    alpha: u16,
) {
    PIXEL_COMPOSITOR_SCALAR_UNIFORM_ALPHA_PIXELS.fetch_add(
        dst_row.chunks_exact(4).zip(src_row.chunks_exact(4)).count() as u64,
        Ordering::Relaxed,
    );
    let inv_alpha = 255_u16.saturating_sub(alpha);
    for (d, s) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
        for channel in 0..3 {
            let src = u16::from(s[channel]);
            let dst = u16::from(d[channel]);
            d[channel] = ((src * alpha + dst * inv_alpha + 127) / 255) as u8;
        }
        d[3] = 255;
    }
}

fn composite_normal_compat_row_scalar_opaque_dst(dst_row: &mut [u8], src_row: &[u8]) {
    PIXEL_COMPOSITOR_SCALAR_OPAQUE_DST_PIXELS.fetch_add(
        dst_row.chunks_exact(4).zip(src_row.chunks_exact(4)).count() as u64,
        Ordering::Relaxed,
    );
    for (d, s) in dst_row.chunks_exact_mut(4).zip(src_row.chunks_exact(4)) {
        let src_a = u16::from(s[3]);
        if src_a == 0 {
            continue;
        }
        if src_a == 255 {
            d.copy_from_slice(&[s[0], s[1], s[2], 255]);
            continue;
        }
        let inv_alpha = 255_u16.saturating_sub(src_a);
        for channel in 0..3 {
            let src = u16::from(s[channel]);
            let dst = u16::from(d[channel]);
            d[channel] = ((src * src_a + dst * inv_alpha + 127) / 255) as u8;
        }
        d[3] = 255;
    }
}

fn row_alpha_is(row: &[u8], alpha: u8) -> bool {
    row.chunks_exact(4).all(|pixel| pixel[3] == alpha)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowAlphaClass {
    AllTransparent,
    AllOpaque,
    Mixed,
}

fn row_alpha_class(row: &[u8]) -> RowAlphaClass {
    let mut saw_transparent = false;
    let mut saw_opaque = false;
    for alpha in row.chunks_exact(4).map(|pixel| pixel[3]) {
        match alpha {
            0 => saw_transparent = true,
            255 => saw_opaque = true,
            _ => return RowAlphaClass::Mixed,
        }
        if saw_transparent && saw_opaque {
            return RowAlphaClass::Mixed;
        }
    }
    match (saw_transparent, saw_opaque) {
        (true, false) => RowAlphaClass::AllTransparent,
        (false, true) => RowAlphaClass::AllOpaque,
        _ => RowAlphaClass::Mixed,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaskRowClass {
    AllTransparent,
    AllOpaque,
    Mixed,
}

fn mask_row_class(row: &[u8]) -> MaskRowClass {
    let mut saw_transparent = false;
    let mut saw_opaque = false;
    for &mask in row {
        match mask {
            0 => saw_transparent = true,
            255 => saw_opaque = true,
            _ => return MaskRowClass::Mixed,
        }
        if saw_transparent && saw_opaque {
            return MaskRowClass::Mixed;
        }
    }
    match (saw_transparent, saw_opaque) {
        (true, false) => MaskRowClass::AllTransparent,
        (false, true) => MaskRowClass::AllOpaque,
        _ => MaskRowClass::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_is_transparent() {
        let buf = PixelBuffer::new(4, 4);
        assert_eq!(buf.render_mode(), RenderMode::Compat);
        assert_eq!(buf.get_pixel(0, 0), TRANSPARENT);
        assert_eq!(buf.get_pixel(3, 3), TRANSPARENT);
    }

    #[test]
    fn copy_rect_to_new_buffer_uses_exact_pixel_window() {
        let mut src = PixelBuffer::new(4, 3);
        for y in 0..3 {
            for x in 0..4 {
                src.set_pixel(
                    x,
                    y,
                    [(10 + x) as u8, (20 + y) as u8, (30 + x + y) as u8, 255],
                );
            }
        }

        let cropped = src.copy_rect_to_new_buffer(1, 1, 2, 2).expect("valid crop");
        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        assert_eq!(cropped.get_pixel(0, 0), src.get_pixel(1, 1));
        assert_eq!(cropped.get_pixel(1, 0), src.get_pixel(2, 1));
        assert_eq!(cropped.get_pixel(0, 1), src.get_pixel(1, 2));
        assert_eq!(cropped.get_pixel(1, 1), src.get_pixel(2, 2));
        assert!(src.copy_rect_to_new_buffer(3, 2, 2, 1).is_none());
    }

    #[test]
    fn blit_from_buffer_copies_rows_without_paint_clip() {
        let mut src = PixelBuffer::new(2, 2);
        src.fill_rect(0, 0, 2, 2, RED);

        let mut dst = PixelBuffer::new(4, 4);
        dst.set_clip(ClipMask::empty(4, 4));
        assert!(
            dst.blit_from_buffer(&src, 1, 1),
            "assembly blit ignores paint clip"
        );
        assert_eq!(dst.get_pixel(1, 1), RED);
        assert_eq!(dst.get_pixel(2, 2), RED);
        assert_eq!(dst.get_pixel(0, 0), TRANSPARENT);
        assert!(!dst.blit_from_buffer(&src, 3, 3));
    }

    #[test]
    fn opaque_fill_rect_short_circuits_solid_clip_states() {
        let mut empty = PixelBuffer::new(3, 3);
        empty.set_clip(ClipMask::empty(3, 3));
        empty.fill_rect(0, 0, 3, 3, BLUE);
        assert_eq!(empty.get_pixel(1, 1), TRANSPARENT);

        let mut visible = PixelBuffer::new(3, 3);
        visible.set_clip(ClipMask::all_visible(3, 3));
        visible.fill_rect(0, 0, 3, 3, GREEN);
        assert_eq!(visible.get_pixel(1, 1), GREEN);
    }

    #[test]
    fn antialiased_clip_mask_preserves_fractional_edge_coverage() {
        let flat = FlatPath {
            subpaths: vec![vec![
                (0.25, 0.25),
                (4.75, 0.25),
                (4.75, 4.75),
                (0.25, 4.75),
                (0.25, 0.25),
            ]],
            closed: vec![true],
        };
        let clip = ClipMask::from_path(&flat, 6, 6, FillRule::NonZero);

        assert!(clip.has_partial_coverage());
        assert!((1..255).contains(&clip.opacity_byte(0, 2)));
        assert_eq!(clip.opacity_byte(2, 2), 255);
        assert_eq!(clip.opacity_byte(5, 2), 0);

        let mut buf = PixelBuffer::new_filled(6, 6, WHITE);
        buf.set_clip(clip);
        buf.fill_rect(0, 0, 6, 6, BLACK);

        let edge = buf.get_pixel(0, 2);
        assert!(
            edge[0] > 0 && edge[0] < 255,
            "fractional clip edge should blend instead of hard filling: {edge:?}"
        );
        assert_eq!(buf.get_pixel(2, 2), BLACK);
        assert_eq!(buf.get_pixel(5, 2), WHITE);
    }

    #[test]
    fn complex_clip_mask_uses_bounded_binary_fallback() {
        let mut subpaths = Vec::new();
        let mut closed = Vec::new();
        for i in 0..1200 {
            let x = 4.25 + f64::from(i % 24) * 2.0;
            let y = f64::from(i / 24);
            subpaths.push(vec![
                (x, y),
                (x + 1.75, y),
                (x + 1.75, y + 900.0),
                (x, y + 900.0),
                (x, y),
            ]);
            closed.push(true);
        }
        let flat = FlatPath { subpaths, closed };
        let clip = ClipMask::from_path(&flat, 64, 1024, FillRule::NonZero);

        assert!(!clip.has_partial_coverage());
        assert_eq!(clip.opacity_byte(5, 100), 255);
        assert_eq!(clip.opacity_byte(0, 100), 0);
    }

    #[test]
    fn opaque_normal_blend_pixel_uses_exact_source_fast_path() {
        let mut buf = PixelBuffer::new(2, 1);
        buf.set_pixel(0, 0, WHITE);

        assert!(buf.can_write_opaque_unclipped());
        buf.blend_pixel(0, 0, [7, 11, 13, 255], 1.0);
        assert_eq!(buf.get_pixel(0, 0), [7, 11, 13, 255]);

        buf.set_clip(ClipMask::empty(2, 1));
        assert!(!buf.can_write_opaque_unclipped());
        buf.blend_pixel(1, 0, RED, 1.0);
        assert_eq!(buf.get_pixel(1, 0), TRANSPARENT);
    }

    #[test]
    fn opaque_unclipped_writer_ignores_alpha_after_capability_check() {
        let mut buf = PixelBuffer::new(1, 1);
        assert!(buf.can_write_opaque_unclipped());
        buf.write_opaque_pixel_unclipped(0, 0, [3, 5, 7, 9]);
        assert_eq!(buf.get_pixel(0, 0), [3, 5, 7, 255]);
    }

    #[test]
    fn opaque_rgb_run_writer_expands_rgb_samples_without_blending() {
        let mut buf = PixelBuffer::new_filled(4, 1, WHITE);
        assert!(buf.can_write_opaque_unclipped());

        let written = buf.write_opaque_rgb_run_unclipped(1, 0, &[1, 2, 3, 4, 5, 6, 7, 8, 9]);

        assert_eq!(written, 3);
        assert_eq!(buf.get_pixel(0, 0), WHITE);
        assert_eq!(buf.get_pixel(1, 0), [1, 2, 3, 255]);
        assert_eq!(buf.get_pixel(2, 0), [4, 5, 6, 255]);
        assert_eq!(buf.get_pixel(3, 0), [7, 8, 9, 255]);
    }

    #[test]
    fn opaque_rgb_run_writer_respects_binary_clip_runs() {
        let mut buf = PixelBuffer::new_filled(5, 1, WHITE);
        let mut clip = ClipMask::empty(5, 1);
        clip.set(1, 0, true);
        clip.set(3, 0, true);
        buf.set_clip(clip);
        assert!(buf.can_write_opaque_with_binary_clip());
        assert!(!buf.can_write_opaque_unclipped());

        let written = buf.write_opaque_rgb_run_binary_clipped(
            0,
            0,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        );

        assert_eq!(written, 2);
        assert_eq!(buf.get_pixel(0, 0), WHITE);
        assert_eq!(buf.get_pixel(1, 0), [4, 5, 6, 255]);
        assert_eq!(buf.get_pixel(2, 0), WHITE);
        assert_eq!(buf.get_pixel(3, 0), [10, 11, 12, 255]);
        assert_eq!(buf.get_pixel(4, 0), WHITE);
    }

    #[test]
    fn uniform_full_buffer_and_row_fills_use_byte_fill_fast_path() {
        let mut buf = PixelBuffer::new(3, 2);
        buf.fill(WHITE);
        assert_eq!(buf.get_pixel(0, 0), WHITE);
        assert_eq!(buf.get_pixel(2, 1), WHITE);

        buf.fill(BLACK);
        buf.fill_rect(1, 0, 2, 1, WHITE);
        assert_eq!(buf.get_pixel(0, 0), BLACK);
        assert_eq!(buf.get_pixel(1, 0), WHITE);
        assert_eq!(buf.get_pixel(2, 0), WHITE);
        assert_eq!(buf.get_pixel(0, 1), BLACK);
    }

    #[test]
    fn translucent_normal_fill_rect_matches_pixel_blend() {
        let color = [200, 40, 20, 96];
        let mut rect = PixelBuffer::new_filled(3, 1, [10, 80, 140, 255]);
        rect.fill_rect(0, 0, 3, 1, color);

        let mut pixel = PixelBuffer::new_filled(3, 1, [10, 80, 140, 255]);
        for x in 0..3 {
            pixel.blend_pixel(x, 0, color, 1.0);
        }

        assert_eq!(
            rect.to_raw_image_rgba().pixels,
            pixel.to_raw_image_rgba().pixels
        );
    }

    #[test]
    fn wide_opaque_destination_blend_matches_pixel_blend() {
        let color = [113, 23, 211, 97];
        let mut wide_path = PixelBuffer::new_filled(7, 1, [17, 83, 149, 255]);
        wide_path.fill_rect(0, 0, 7, 1, color);

        let mut scalar_path = PixelBuffer::new_filled(7, 1, [17, 83, 149, 255]);
        for x in 0..7 {
            scalar_path.blend_pixel(x, 0, color, 1.0);
        }

        for (a, b) in wide_path
            .to_raw_image_rgba()
            .pixels
            .iter()
            .zip(scalar_path.to_raw_image_rgba().pixels.iter())
        {
            assert!((*a as i16 - *b as i16).abs() <= 1);
        }
    }

    #[test]
    fn separable_opaque_fill_rect_matches_pixel_blend() {
        let color = [180, 20, 210, 255];
        let mut rect = PixelBuffer::new_filled(5, 1, [30, 120, 200, 255]);
        rect.blend_mode = BlendMode::Multiply;
        rect.fill_rect(0, 0, 5, 1, color);

        let mut pixel = PixelBuffer::new_filled(5, 1, [30, 120, 200, 255]);
        pixel.blend_mode = BlendMode::Multiply;
        for x in 0..5 {
            pixel.blend_pixel(x, 0, color, 1.0);
        }

        assert_eq!(
            rect.to_raw_image_rgba().pixels,
            pixel.to_raw_image_rgba().pixels
        );
    }

    #[test]
    fn separable_opaque_fill_rect_binary_clip_matches_pixel_blend() {
        let color = [64, 220, 100, 255];
        let mut clip = ClipMask::empty(6, 1);
        clip.fill_rect(1, 0, 2, 1, true);
        clip.fill_rect(4, 0, 1, 1, true);

        let mut rect = PixelBuffer::new_filled(6, 1, [90, 80, 70, 255]);
        rect.blend_mode = BlendMode::Screen;
        rect.set_clip(clip.clone());
        rect.fill_rect(0, 0, 6, 1, color);

        let mut pixel = PixelBuffer::new_filled(6, 1, [90, 80, 70, 255]);
        pixel.blend_mode = BlendMode::Screen;
        pixel.set_clip(clip);
        for x in 0..6 {
            pixel.blend_pixel(x, 0, color, 1.0);
        }

        assert_eq!(
            rect.to_raw_image_rgba().pixels,
            pixel.to_raw_image_rgba().pixels
        );
    }

    #[test]
    fn normal_group_composite_fast_path_matches_pixel_blend() {
        let mut src = PixelBuffer::new_transparent(3, 1);
        src.set_pixel(0, 0, [200, 40, 20, 128]);
        src.set_pixel(1, 0, [1, 2, 3, 255]);

        let mut fast = PixelBuffer::new_filled(3, 1, [10, 80, 140, 255]);
        fast.composite_from(&src, 0.75, BlendMode::Normal, None);

        let mut expected = PixelBuffer::new_filled(3, 1, [10, 80, 140, 255]);
        for x in 0..3 {
            expected.blend_pixel(x, 0, src.get_pixel(x, 0), 0.75);
        }

        assert_eq!(
            fast.to_raw_image_rgba().pixels,
            expected.to_raw_image_rgba().pixels
        );
    }

    #[test]
    fn clipped_normal_group_composite_matches_pixel_blend() {
        let mut src = PixelBuffer::new_transparent(5, 1);
        src.set_pixel(0, 0, [200, 40, 20, 128]);
        src.set_pixel(1, 0, [1, 2, 3, 255]);
        src.set_pixel(2, 0, [40, 80, 120, 96]);
        src.set_pixel(3, 0, [70, 80, 90, 255]);
        src.set_pixel(4, 0, [11, 22, 33, 128]);

        let mut clip = ClipMask::empty(5, 1);
        clip.fill_rect(1, 0, 3, 1, true);

        let mut fast = PixelBuffer::new_filled(5, 1, [10, 80, 140, 255]);
        fast.set_clip(clip.clone());
        fast.composite_from(&src, 0.75, BlendMode::Normal, None);

        let mut expected = PixelBuffer::new_filled(5, 1, [10, 80, 140, 255]);
        expected.set_clip(clip);
        for x in 0..5 {
            expected.blend_pixel(x, 0, src.get_pixel(x, 0), 0.75);
        }

        assert_eq!(
            fast.to_raw_image_rgba().pixels,
            expected.to_raw_image_rgba().pixels
        );
    }

    #[test]
    fn clip_mask_scanline_full_rect_refreshes_solid_hint() {
        let flat = FlatPath {
            subpaths: vec![vec![
                (0.0, 0.0),
                (3.0, 0.0),
                (3.0, 2.0),
                (0.0, 2.0),
                (0.0, 0.0),
            ]],
            closed: vec![true],
        };
        let clip = ClipMask::from_path(&flat, 3, 2, FillRule::NonZero);

        assert!(clip.is_all_visible());
        assert!(clip.is_visible(0, 0));
        assert!(clip.is_visible(2, 1));
    }

    #[test]
    fn clip_mask_intersect_and_union_refresh_solid_hints() {
        let mut intersected = ClipMask::all_visible(2, 2);
        let empty = ClipMask::empty(2, 2);
        intersected.intersect(&empty);
        assert!(intersected.is_empty());

        let mut unioned = ClipMask::empty(2, 2);
        let visible = ClipMask::all_visible(2, 2);
        unioned.union_with(&visible);
        assert!(unioned.is_all_visible());
    }

    #[test]
    fn render_mode_names_parse() {
        assert_eq!(RenderMode::from_name("compat"), Some(RenderMode::Compat));
        assert_eq!(RenderMode::from_name("high"), Some(RenderMode::HighQuality));
        assert_eq!(
            RenderMode::from_name("high-quality"),
            Some(RenderMode::HighQuality)
        );
        assert_eq!(RenderMode::from_name("unknown"), None);
        assert_eq!(RenderMode::HighQuality.as_str(), "high");
    }

    #[test]
    fn set_and_get_pixel() {
        let mut buf = PixelBuffer::new(10, 10);
        buf.set_pixel(5, 5, RED);
        assert_eq!(buf.get_pixel(5, 5), RED);
        assert_eq!(buf.get_pixel(0, 0), TRANSPARENT);
    }

    #[test]
    fn out_of_bounds_set_pixel_is_no_op() {
        let mut buf = PixelBuffer::new(4, 4);
        buf.set_pixel(-1, 0, RED);
        buf.set_pixel(4, 0, RED);
        buf.set_pixel(0, -1, RED);
        buf.set_pixel(0, 4, RED);
        assert_eq!(buf.get_pixel(0, 0), TRANSPARENT);
    }

    #[test]
    fn fill() {
        let mut buf = PixelBuffer::new(3, 3);
        buf.fill(WHITE);
        assert_eq!(buf.get_pixel(0, 0), WHITE);
        assert_eq!(buf.get_pixel(2, 2), WHITE);
    }

    #[test]
    fn blend_pixel_composites_correctly() {
        let mut buf = PixelBuffer::new(1, 1);
        buf.fill(WHITE);
        buf.blend_pixel(0, 0, RED, 0.5);
        let p = buf.get_pixel(0, 0);
        assert!(p[0] >= 200);
        assert!(p[1] <= 200);
    }

    #[test]
    fn gamma_tables_round_trip_endpoints() {
        // Black and white survive the linear round trip exactly.
        assert_eq!(gamma::to_srgb(gamma::to_linear(0)), 0);
        assert_eq!(gamma::to_srgb(gamma::to_linear(255)), 255);
        // Mid-gray sRGB 128 decodes to ~0.216 linear, re-encodes back to ~128.
        let mid = gamma::to_srgb(gamma::to_linear(128));
        assert!((mid as i32 - 128).abs() <= 1, "128 round-trips, got {mid}");
        // sRGB 188 ~= 0.5 in linear light.
        assert!((gamma::to_linear(188) - 0.5).abs() < 0.02);
    }

    #[test]
    fn blend_50pct_black_over_white_is_srgb_midpoint() {
        // Compositing is done in sRGB space to match the reference renderer
        // (Poppler/Splash): 50% black over white lands at the sRGB midpoint 128,
        // NOT the linear-light value ~188. This is the deliberate
        // benchmark-matching behaviour (see the sRGB note in `blend_pixel`).
        let mut buf = PixelBuffer::new_filled(1, 1, WHITE);
        buf.blend_pixel(0, 0, BLACK, 0.5);
        let p = buf.get_pixel(0, 0);
        assert!(
            (p[0] as i32 - 128).abs() <= 2,
            "50% black over white should be ~128 (sRGB midpoint), got {}",
            p[0]
        );
    }

    #[test]
    fn high_quality_blend_50pct_black_over_white_is_linear_light_midpoint() {
        let mut buf = PixelBuffer::new_filled_with_mode(1, 1, WHITE, RenderMode::HighQuality);
        buf.blend_pixel(0, 0, BLACK, 0.5);
        let p = buf.get_pixel(0, 0);
        assert!(
            (p[0] as i32 - 188).abs() <= 2,
            "50% black over white should be ~188 in linear light, got {}",
            p[0]
        );
    }

    #[test]
    fn blend_pixel_with_zero_coverage_is_no_op() {
        let mut buf = PixelBuffer::new(1, 1);
        buf.fill(WHITE);
        buf.blend_pixel(0, 0, RED, 0.0);
        assert_eq!(buf.get_pixel(0, 0), WHITE);
    }

    #[test]
    fn to_rgb_bytes_discards_alpha() {
        let mut buf = PixelBuffer::new(2, 1);
        buf.set_pixel(0, 0, [255, 0, 0, 128]);
        buf.set_pixel(1, 0, [0, 255, 0, 255]);
        assert_eq!(buf.to_rgb_bytes(), vec![255, 0, 0, 0, 255, 0]);
    }

    #[test]
    fn flatten_onto_background_outputs_opaque_white_paper() {
        let mut buf = PixelBuffer::new_transparent(2, 1);
        buf.set_pixel(0, 0, [0, 0, 255, 128]);
        buf.flatten_onto_background(WHITE);

        assert_eq!(buf.get_pixel(1, 0), WHITE);
        let p = buf.get_pixel(0, 0);
        assert_eq!(p[3], 255);
        assert!(p[2] > 240, "blue channel stays high: {:?}", p);
        assert!(
            (p[0] as i32 - 127).abs() <= 2 && (p[1] as i32 - 127).abs() <= 2,
            "transparent blue flattens over white: {:?}",
            p
        );
    }

    #[test]
    fn flatten_compat_opaque_background_matches_source_over_math() {
        let background = [20, 40, 60, 255];
        let mut data = vec![200, 100, 50, 128, 10, 30, 70, 0, 90, 80, 70, 255];
        flatten_compat_onto_opaque_background(&mut data, background);

        let expected_channel = |src: u8, bg: u8, alpha: u8| -> u8 {
            let a = alpha as u16;
            let inv = 255_u16.saturating_sub(a);
            ((src as u16 * a + bg as u16 * inv + 127) / 255) as u8
        };
        assert_eq!(
            &data[0..4],
            &[
                expected_channel(200, 20, 128),
                expected_channel(100, 40, 128),
                expected_channel(50, 60, 128),
                255,
            ]
        );
        assert_eq!(&data[4..8], &background);
        assert_eq!(&data[8..12], &[90, 80, 70, 255]);
    }

    #[test]
    fn flatten_compat_opaque_background_wide_path_handles_mixed_alpha_and_tail() {
        let background = [11, 22, 33, 255];
        let mut data = vec![
            100, 0, 0, 32, 0, 100, 0, 64, 0, 0, 100, 128, 200, 200, 0, 192, 250, 10, 20, 255,
        ];
        let original = data.clone();
        flatten_compat_onto_opaque_background(&mut data, background);

        for (idx, (input, output)) in original
            .chunks_exact(4)
            .zip(data.chunks_exact(4))
            .enumerate()
        {
            let alpha = u16::from(input[3]);
            let inv_alpha = 255_u16.saturating_sub(alpha);
            let expected = if alpha == 255 {
                [input[0], input[1], input[2], 255]
            } else if alpha == 0 {
                background
            } else {
                [
                    ((u16::from(input[0]) * alpha + u16::from(background[0]) * inv_alpha + 127)
                        / 255) as u8,
                    ((u16::from(input[1]) * alpha + u16::from(background[1]) * inv_alpha + 127)
                        / 255) as u8,
                    ((u16::from(input[2]) * alpha + u16::from(background[2]) * inv_alpha + 127)
                        / 255) as u8,
                    255,
                ]
            };
            assert_eq!(output, expected, "pixel {idx}");
        }
    }

    #[test]
    fn to_raw_image_has_correct_dimensions_and_channels() {
        let buf = PixelBuffer::new(100, 200);
        let raw = buf.to_raw_image();
        assert_eq!(raw.width, 100);
        assert_eq!(raw.height, 200);
        assert_eq!(raw.channels, 3);
        assert_eq!(raw.pixels.len(), 100 * 200 * 3);
    }

    #[test]
    fn fill_rect_clips_correctly() {
        let mut buf = PixelBuffer::new(10, 10);
        buf.fill_rect(-5, -5, 20, 20, RED);
        for y in 0..10i32 {
            for x in 0..10i32 {
                assert_eq!(buf.get_pixel(x, y), RED);
            }
        }
    }

    #[test]
    fn fill_rect_partial_overlap() {
        let mut buf = PixelBuffer::new(10, 10);
        buf.fill_rect(5, 5, 10, 10, RED);
        assert_eq!(buf.get_pixel(5, 5), RED);
        assert_eq!(buf.get_pixel(9, 9), RED);
        assert_eq!(buf.get_pixel(4, 4), TRANSPARENT);
        assert_eq!(buf.get_pixel(4, 5), TRANSPARENT);
    }

    #[test]
    fn blend_pixel_full_opacity_replaces_pixel() {
        let mut buf = PixelBuffer::new(1, 1);
        buf.fill(WHITE);
        buf.blend_pixel(0, 0, BLACK, 1.0);
        let p = buf.get_pixel(0, 0);
        assert!(p[0] < 10);
        assert!(p[1] < 10);
        assert!(p[2] < 10);
    }

    #[test]
    fn cmyk_overprint_preview_preserves_zero_ink_channels() {
        let to_pixel = |rgb: [f32; 3]| -> PixelColor {
            [
                (rgb[0] * 255.0).round() as u8,
                (rgb[1] * 255.0).round() as u8,
                (rgb[2] * 255.0).round() as u8,
                255,
            ]
        };
        let mut buf = PixelBuffer::new(1, 1);
        buf.set_pixel(0, 0, to_pixel(cmm::device_cmyk_to_srgb(0.0, 0.0, 1.0, 0.0)));
        buf.blend_device_cmyk_overprint_preview(0, 0, [1.0, 0.0, 0.0, 0.0], 1.0, 1.0, 1);
        let expected = to_pixel(cmm::device_cmyk_to_srgb(1.0, 0.0, 1.0, 0.0));
        let actual = buf.get_pixel(0, 0);
        for i in 0..3 {
            assert!((actual[i] as i16 - expected[i] as i16).abs() <= 8);
        }
    }

    #[test]
    fn multiple_blend_operations_accumulate() {
        let mut buf = PixelBuffer::new(1, 1);
        buf.fill(WHITE);
        buf.blend_pixel(0, 0, RED, 0.5);
        buf.blend_pixel(0, 0, RED, 0.5);
        let p = buf.get_pixel(0, 0);
        assert!(p[0] >= 240);
    }

    #[test]
    fn to_raw_image_rgba_includes_alpha() {
        let mut buf = PixelBuffer::new(1, 1);
        buf.set_pixel(0, 0, [100, 150, 200, 128]);
        let raw = buf.to_raw_image_rgba();
        assert_eq!(raw.channels, 4);
        assert_eq!(&raw.pixels, &[100, 150, 200, 128]);
    }

    #[test]
    fn fill_rect_with_no_clip_uses_fast_path_correctly() {
        let mut buf = PixelBuffer::new_filled(50, 50, WHITE);
        buf.fill_rect(10, 10, 30, 30, RED);
        assert_eq!(buf.get_pixel(25, 25), RED);
        assert_eq!(buf.get_pixel(9, 10), WHITE);
        assert_eq!(buf.get_pixel(40, 10), WHITE);
    }

    #[test]
    fn fill_rect_with_clip_uses_span_path_correctly() {
        let mut buf = PixelBuffer::new_filled(20, 20, WHITE);
        let mut clip = ClipMask::all_visible(20, 20);
        clip.fill_rect(0, 0, 20, 5, false);
        clip.fill_rect(0, 15, 20, 5, false);
        buf.set_clip(clip);
        buf.fill_rect(0, 0, 20, 20, RED);
        assert_eq!(buf.get_pixel(10, 10), RED);
        assert_eq!(buf.get_pixel(10, 2), WHITE);
        assert_eq!(buf.get_pixel(10, 18), WHITE);
    }

    #[test]
    fn fill_rect_with_solid_clip_fills_entire_rect() {
        let mut buf = PixelBuffer::new_filled(20, 20, WHITE);
        let clip = ClipMask::all_visible(20, 20);
        buf.set_clip(clip);
        buf.fill_rect(5, 5, 10, 10, BLUE);
        assert_eq!(buf.get_pixel(10, 10), BLUE);
        assert_eq!(buf.get_pixel(0, 0), WHITE);
    }

    #[test]
    fn fill_rect_with_column_stripe_clip() {
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        let mut clip = ClipMask::all_visible(10, 10);
        for x in (1..10).step_by(2) {
            clip.fill_rect(x, 0, 1, 10, false);
        }
        buf.set_clip(clip);
        buf.fill_rect(0, 0, 10, 10, RED);

        assert_eq!(buf.get_pixel(0, 5), RED);
        assert_eq!(buf.get_pixel(2, 5), RED);
        assert_eq!(buf.get_pixel(1, 5), WHITE);
        assert_eq!(buf.get_pixel(3, 5), WHITE);
    }

    #[test]
    fn fill_rect_zero_dimensions_are_noop() {
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        buf.fill_rect(5, 5, 0, 5, RED);
        buf.fill_rect(5, 5, 5, 0, RED);
        buf.fill_rect(5, 5, -1, 5, RED);

        for y in 0..10i32 {
            for x in 0..10i32 {
                assert_eq!(buf.get_pixel(x, y), WHITE);
            }
        }
    }

    #[test]
    fn fill_rect_run_merging_preserves_clipped_gap() {
        let mut buf = PixelBuffer::new_filled(100, 1, WHITE);
        let mut clip = ClipMask::all_visible(100, 1);
        clip.set(50, 0, false);
        buf.set_clip(clip);
        buf.fill_rect(0, 0, 100, 1, BLUE);

        assert_eq!(buf.get_pixel(0, 0), BLUE);
        assert_eq!(buf.get_pixel(49, 0), BLUE);
        assert_eq!(buf.get_pixel(50, 0), WHITE);
        assert_eq!(buf.get_pixel(51, 0), BLUE);
        assert_eq!(buf.get_pixel(99, 0), BLUE);
    }

    #[test]
    fn clip_visible_bounds_track_sparse_coverage() {
        let empty = ClipMask::empty(8, 6);
        assert_eq!(empty.visible_bounds(), None);

        let visible = ClipMask::all_visible(8, 6);
        assert_eq!(visible.visible_bounds(), Some((0, 0, 8, 6)));

        let mut sparse = ClipMask::empty(8, 6);
        sparse.set(3, 2, true);
        sparse.set(6, 4, true);

        assert_eq!(sparse.visible_bounds(), Some((3, 2, 7, 5)));
    }

    #[test]
    fn translucent_fill_rect_binary_clip_uses_span_runs() {
        let mut clipped = PixelBuffer::new_filled(6, 1, [10, 20, 30, 255]);
        let mut expected = clipped.clone();
        let mut clip = ClipMask::empty(6, 1);
        clip.set(1, 0, true);
        clip.set(2, 0, true);
        clip.set(4, 0, true);
        clipped.set_clip(clip);
        let color = [200, 40, 20, 96];

        clipped.fill_rect(0, 0, 6, 1, color);
        for x in [1, 2, 4] {
            expected.blend_pixel(x, 0, color, 1.0);
        }

        assert_eq!(clipped.data, expected.data);
    }

    #[test]
    fn fill_rect_with_no_visible_pixels_does_nothing() {
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        let mut clip = ClipMask::all_visible(10, 10);
        clip.fill_rect(0, 0, 10, 10, false);
        buf.set_clip(clip);
        buf.fill_rect(0, 0, 10, 10, RED);

        let any_red = (0..10i32)
            .flat_map(|y| (0..10i32).map(move |x| (x, y)))
            .any(|(x, y)| {
                let pixel = buf.get_pixel(x, y);
                pixel[0] == 255 && pixel[1] == 0
            });
        assert!(!any_red);
    }

    #[test]
    fn blend_mode_channel_math_matches_pdf_modes() {
        assert_eq!(BlendMode::Normal.blend_channel(0.8, 0.3), 0.8);
        assert!((BlendMode::Multiply.blend_channel(0.8, 0.5) - 0.4).abs() < 0.001);
        assert!((BlendMode::Screen.blend_channel(0.8, 0.5) - 0.9).abs() < 0.001);
        assert!((BlendMode::Overlay.blend_channel(0.6, 0.3) - 0.36).abs() < 0.001);
        assert!((BlendMode::Overlay.blend_channel(0.6, 0.8) - 0.84).abs() < 0.001);
        assert_eq!(BlendMode::Darken.blend_channel(0.3, 0.7), 0.3);
        assert_eq!(BlendMode::Darken.blend_channel(0.7, 0.3), 0.3);
        assert_eq!(BlendMode::Lighten.blend_channel(0.3, 0.7), 0.7);
        assert_eq!(BlendMode::Lighten.blend_channel(0.7, 0.3), 0.7);
    }

    #[test]
    fn blend_mode_from_name_parses_supported_modes() {
        assert_eq!(BlendMode::from_name("Multiply"), BlendMode::Multiply);
        assert_eq!(BlendMode::from_name("Screen"), BlendMode::Screen);
        assert_eq!(BlendMode::from_name("Overlay"), BlendMode::Overlay);
        assert_eq!(BlendMode::from_name("Normal"), BlendMode::Normal);
        assert_eq!(BlendMode::from_name("Compatible"), BlendMode::Normal);
        assert_eq!(BlendMode::from_name("Unknown"), BlendMode::Normal);
    }

    /// Spec-formula exact assertions for the cases that historically diverged:
    /// Screen over white (the object vanishes â€” `B(1, cs) = 1`) AND Multiply over
    /// white (the source shows â€” `B(1, cs) = cs`), plus both over a mid-tone
    /// backdrop. Both modes must be correct *simultaneously*; this table pins
    /// that no fix to one mode can silently break the other.
    #[test]
    fn screen_and_multiply_over_white_and_midtone_both_correct() {
        let white = 1.0_f32;
        let mid = 0.5_f32;

        // Screen over white: s + 1 - s*1 = 1 for every source -> object vanishes.
        for &s in &[0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            assert!(
                (BlendMode::Screen.blend_channel(s, white) - 1.0).abs() < 1e-3,
                "Screen({s}, white=1) must be 1 (object vanishes over white)"
            );
        }
        // Multiply over white: s * 1 = s -> the source shows through.
        for &s in &[0.0_f32, 0.25, 0.5, 0.75, 1.0] {
            assert!(
                (BlendMode::Multiply.blend_channel(s, white) - s).abs() < 1e-3,
                "Multiply({s}, white=1) must equal source {s} (source shows over white)"
            );
        }
        // Mid-tone backdrop, the already-known anchors, kept here so the table
        // is the single proof of simultaneous correctness.
        assert!((BlendMode::Multiply.blend_channel(0.8, mid) - 0.4).abs() < 1e-3);
        assert!((BlendMode::Screen.blend_channel(0.8, mid) - 0.9).abs() < 1e-3);
    }

    #[test]
    fn multiply_blend_pixel_darkens_destination() {
        let mut buf = PixelBuffer::new_filled(1, 1, [200, 200, 200, 255]);
        buf.blend_mode = BlendMode::Multiply;
        buf.blend_pixel(0, 0, [128, 128, 128, 255], 1.0);
        let result = buf.get_pixel(0, 0);
        assert!(result[0] < 170, "Multiply should darken: R={}", result[0]);
    }

    #[test]
    fn screen_blend_pixel_lightens_destination() {
        let mut buf = PixelBuffer::new_filled(1, 1, [100, 100, 100, 255]);
        buf.blend_mode = BlendMode::Screen;
        buf.blend_pixel(0, 0, [100, 100, 100, 255], 1.0);
        let result = buf.get_pixel(0, 0);
        // Compositing is in sRGB space (matches Poppler). Screen in sRGB on the
        // normalised channel: 1 - (1-c)(1-c) with c = 100/255.
        let c = 100.0f32 / 255.0;
        let screened = 1.0 - (1.0 - c) * (1.0 - c);
        let expected = (screened * 255.0).round() as i32;
        assert!(result[0] > 100, "Screen must lighten: {}", result[0]);
        assert!(
            (result[0] as i32 - expected).abs() <= 2,
            "Screen blend result: {} expected: {}",
            result[0],
            expected
        );
    }

    #[test]
    fn screen_over_partially_transparent_backdrop_matches_pdf_compositing() {
        let mut buf = PixelBuffer::new_transparent(1, 1);
        buf.blend_mode = BlendMode::Multiply;
        buf.blend_pixel(0, 0, [255, 0, 0, 115], 1.0);
        buf.blend_mode = BlendMode::Screen;
        buf.blend_pixel(0, 0, [0, 0, 255, 140], 1.0);
        buf.flatten_onto_background(WHITE);

        let result = buf.get_pixel(0, 0);
        assert!(
            (result[0] as i32 - 178).abs() <= 2,
            "red channel should include uncovered source contribution: {:?}",
            result
        );
        assert!(
            (result[1] as i32 - 63).abs() <= 2 && (result[2] as i32 - 203).abs() <= 2,
            "green/blue channels should match PDF source-over blend math: {:?}",
            result
        );
    }

    #[test]
    fn transparent_paint_does_not_change_destination() {
        let mut buf = PixelBuffer::new_filled(1, 1, WHITE);
        buf.blend_pixel(0, 0, [0, 0, 0, 0], 1.0);
        assert_eq!(buf.get_pixel(0, 0), WHITE);
    }

    #[test]
    fn blend_pixel_uses_current_buffer_blend_mode() {
        let mut multiply = PixelBuffer::new_filled(1, 1, [200, 200, 200, 255]);
        multiply.blend_mode = BlendMode::Multiply;
        multiply.blend_pixel(0, 0, [128, 128, 128, 255], 1.0);
        let multiply_result = multiply.get_pixel(0, 0)[0];

        let mut normal = PixelBuffer::new_filled(1, 1, [200, 200, 200, 255]);
        normal.blend_mode = BlendMode::Normal;
        normal.blend_pixel(0, 0, [128, 128, 128, 255], 1.0);
        let normal_result = normal.get_pixel(0, 0)[0];

        assert!(
            multiply_result < normal_result,
            "Multiply({}) should be darker than Normal({})",
            multiply_result,
            normal_result
        );
    }

    #[test]
    fn alpha_mask_from_luminosity_handles_white_black_and_gray() {
        let white = AlphaMask::from_luminosity(&PixelBuffer::new_filled(1, 1, WHITE));
        assert_eq!(white.get(0, 0), 1.0);

        let black = AlphaMask::from_luminosity(&PixelBuffer::new_filled(1, 1, BLACK));
        assert!(black.get(0, 0).abs() < 0.01);

        let gray = AlphaMask::from_luminosity(&PixelBuffer::new_filled(1, 1, [128, 128, 128, 255]));
        assert!(
            (gray.get(0, 0) - 0.502).abs() < 0.01,
            "gray alpha: {}",
            gray.get(0, 0)
        );
    }

    #[test]
    fn smask_modulates_blend_pixel_alpha() {
        let mut mask = AlphaMask::all_opaque(1, 1);
        mask.set(0, 0, 128);

        let mut buf = PixelBuffer::new_filled(1, 1, WHITE);
        buf.set_smask(mask);
        buf.blend_pixel(0, 0, BLACK, 1.0);
        let result = buf.get_pixel(0, 0);
        assert!(
            result[0] > 100 && result[0] < 200,
            "50% soft mask over white should be gray-ish: {:?}",
            result
        );
    }

    #[test]
    fn composite_from_half_alpha_red_over_white_is_pink() {
        // A fully-opaque red source composited at 50% group alpha onto white
        // must produce the same pink as a 50%-alpha red paint. With sRGB-space
        // compositing (matching Poppler/Splash) the GREEN/BLUE channels mix 50%
        // of black (red's G/B = 0) with white at the sRGB midpoint 128 (see
        // `blend_50pct_black_over_white_is_srgb_midpoint`).
        let mut dst = PixelBuffer::new_filled(2, 2, WHITE);
        let src = PixelBuffer::new_filled(2, 2, RED);
        dst.composite_from(&src, 0.5, BlendMode::Normal, None);
        let p = dst.get_pixel(0, 0);
        assert_eq!(p[0], 255, "red channel stays max");
        assert!(
            (p[1] as i32 - 128).abs() <= 2,
            "green ~128 (sRGB), got {}",
            p[1]
        );
        assert!(
            (p[2] as i32 - 128).abs() <= 2,
            "blue ~128 (sRGB), got {}",
            p[2]
        );
    }

    #[test]
    fn composite_from_opaque_source_uniform_alpha_wide_row_matches_source_over_math() {
        let mut dst = PixelBuffer::new_filled(5, 1, [30, 60, 90, 255]);
        let mut src = PixelBuffer::new_transparent(5, 1);
        let samples = [
            [200, 10, 20, 255],
            [20, 200, 10, 255],
            [20, 10, 200, 255],
            [240, 240, 0, 255],
            [5, 25, 250, 255],
        ];
        for (x, sample) in samples.iter().enumerate() {
            src.set_pixel(x as i32, 0, *sample);
        }

        dst.composite_from(&src, 0.375, BlendMode::Normal, None);

        let alpha = (0.375_f32 * 255.0).round() as u16;
        let inv_alpha = 255_u16.saturating_sub(alpha);
        for (x, sample) in samples.iter().enumerate() {
            let expected = [
                ((u16::from(sample[0]) * alpha + 30 * inv_alpha + 127) / 255) as u8,
                ((u16::from(sample[1]) * alpha + 60 * inv_alpha + 127) / 255) as u8,
                ((u16::from(sample[2]) * alpha + 90 * inv_alpha + 127) / 255) as u8,
                255,
            ];
            assert_eq!(dst.get_pixel(x as i32, 0), expected);
        }
    }

    #[test]
    fn composite_from_opaque_destination_wide_row_matches_source_over_math() {
        let mut dst = PixelBuffer::new_filled(5, 1, [30, 60, 90, 255]);
        let mut src = PixelBuffer::new_transparent(5, 1);
        let samples = [
            [200, 10, 20, 0],
            [200, 10, 20, 64],
            [20, 200, 10, 127],
            [20, 10, 200, 192],
            [240, 240, 0, 255],
        ];
        for (x, sample) in samples.iter().enumerate() {
            src.set_pixel(x as i32, 0, *sample);
        }

        dst.composite_from(&src, 1.0, BlendMode::Normal, None);

        for (x, sample) in samples.iter().enumerate() {
            let alpha = u16::from(sample[3]);
            let inv_alpha = 255_u16.saturating_sub(alpha);
            let expected = [
                ((u16::from(sample[0]) * alpha + 30 * inv_alpha + 127) / 255) as u8,
                ((u16::from(sample[1]) * alpha + 60 * inv_alpha + 127) / 255) as u8,
                ((u16::from(sample[2]) * alpha + 90 * inv_alpha + 127) / 255) as u8,
                255,
            ];
            assert_eq!(dst.get_pixel(x as i32, 0), expected);
        }
    }

    #[test]
    fn row_alpha_class_distinguishes_fast_copy_and_skip_rows() {
        assert_eq!(
            row_alpha_class(&[1, 2, 3, 0, 4, 5, 6, 0]),
            RowAlphaClass::AllTransparent
        );
        assert_eq!(
            row_alpha_class(&[1, 2, 3, 255, 4, 5, 6, 255]),
            RowAlphaClass::AllOpaque
        );
        assert_eq!(
            row_alpha_class(&[1, 2, 3, 0, 4, 5, 6, 255]),
            RowAlphaClass::Mixed
        );
        assert_eq!(
            row_alpha_class(&[1, 2, 3, 128, 4, 5, 6, 255]),
            RowAlphaClass::Mixed
        );
    }

    #[test]
    fn mask_row_class_distinguishes_fast_skip_and_unmasked_rows() {
        assert_eq!(mask_row_class(&[0, 0, 0, 0]), MaskRowClass::AllTransparent);
        assert_eq!(mask_row_class(&[255, 255, 255]), MaskRowClass::AllOpaque);
        assert_eq!(mask_row_class(&[0, 255]), MaskRowClass::Mixed);
        assert_eq!(mask_row_class(&[255, 128, 255]), MaskRowClass::Mixed);
    }

    #[test]
    fn clip_mask_visible_run_iterator_reports_binary_spans() {
        let mut clip = ClipMask::empty(8, 2);
        clip.set(1, 1, true);
        clip.set(2, 1, true);
        clip.set(5, 1, true);
        clip.set(6, 1, true);

        let mut runs = Vec::new();
        clip.for_each_visible_run(1, 8, |start, end| runs.push((start, end)));

        assert_eq!(runs, vec![(1, 3), (5, 7)]);
    }

    #[test]
    fn clip_mask_visible_run_iterator_handles_solid_masks_without_scanning() {
        let clip = ClipMask::all_visible(8, 2);
        let mut runs = Vec::new();
        clip.for_each_visible_run(1, 5, |start, end| runs.push((start, end)));
        assert_eq!(runs, vec![(0, 5)]);

        let clip = ClipMask::empty(8, 2);
        let mut runs = Vec::new();
        clip.for_each_visible_run(1, 5, |start, end| runs.push((start, end)));
        assert!(runs.is_empty());
    }

    #[test]
    fn composite_from_respects_per_pixel_soft_mask() {
        // Opaque red source, full group alpha, but a soft mask that is 0 at
        // pixel (0,0) and 255 at (1,0). The masked pixel must stay white; the
        // unmasked pixel must become red.
        let mut dst = PixelBuffer::new_filled(2, 1, WHITE);
        let src = PixelBuffer::new_filled(2, 1, RED);
        let mut mask = AlphaMask::all_opaque(2, 1);
        mask.set(0, 0, 0);
        mask.set(1, 0, 255);
        dst.composite_from(&src, 1.0, BlendMode::Normal, Some(&mask));
        assert_eq!(dst.get_pixel(0, 0), WHITE, "masked-out pixel unchanged");
        assert_eq!(dst.get_pixel(1, 0), RED, "unmasked pixel fully painted");
    }

    #[test]
    fn composite_from_soft_mask_fast_path_matches_opaque_destination_math() {
        let mut dst = PixelBuffer::new_filled(4, 1, [40, 80, 120, 255]);
        let mut src = PixelBuffer::new_transparent(4, 1);
        let samples = [
            [200, 10, 20, 255],
            [10, 220, 20, 192],
            [10, 20, 240, 128],
            [250, 250, 0, 64],
        ];
        for (x, sample) in samples.iter().enumerate() {
            src.set_pixel(x as i32, 0, *sample);
        }
        let mut mask = AlphaMask::all_opaque(4, 1);
        for (x, value) in [0_u8, 80, 160, 255].into_iter().enumerate() {
            mask.set(x as i32, 0, value);
        }

        dst.composite_from(&src, 0.75, BlendMode::Normal, Some(&mask));

        let group_alpha = (0.75_f32 * 255.0).round() as u32;
        for (x, sample) in samples.iter().enumerate() {
            let mask = u32::from([0_u8, 80, 160, 255][x]);
            let eff = (u32::from(sample[3]) * mask * group_alpha + 32_512) / 65_025;
            let inv = 255_u32.saturating_sub(eff);
            let expected = [
                ((u32::from(sample[0]) * eff + 40 * inv + 127) / 255) as u8,
                ((u32::from(sample[1]) * eff + 80 * inv + 127) / 255) as u8,
                ((u32::from(sample[2]) * eff + 120 * inv + 127) / 255) as u8,
                255,
            ];
            assert_eq!(dst.get_pixel(x as i32, 0), expected);
        }
    }

    #[test]
    fn composite_from_soft_mask_all_opaque_row_matches_unmasked_path() {
        let mut masked = PixelBuffer::new_filled(4, 1, [12, 34, 56, 255]);
        let mut unmasked = masked.clone();
        let mut src = PixelBuffer::new_transparent(4, 1);
        for (x, sample) in [
            [200, 10, 20, 255],
            [10, 220, 20, 192],
            [10, 20, 240, 128],
            [250, 250, 0, 64],
        ]
        .into_iter()
        .enumerate()
        {
            src.set_pixel(x as i32, 0, sample);
        }
        let mask = AlphaMask::all_opaque(4, 1);

        masked.composite_from(&src, 0.6, BlendMode::Normal, Some(&mask));
        unmasked.composite_from(&src, 0.6, BlendMode::Normal, None);

        assert_eq!(masked.data, unmasked.data);
    }

    #[test]
    fn composite_from_soft_mask_binary_clip_all_opaque_row_matches_unmasked_run() {
        let mut masked = PixelBuffer::new_filled(5, 1, [20, 40, 60, 255]);
        let mut unmasked = masked.clone();
        let mut src = PixelBuffer::new_transparent(5, 1);
        for (x, sample) in [
            [255, 0, 0, 255],
            [0, 255, 0, 220],
            [0, 0, 255, 180],
            [255, 255, 0, 120],
            [255, 0, 255, 80],
        ]
        .into_iter()
        .enumerate()
        {
            src.set_pixel(x as i32, 0, sample);
        }
        let mask = AlphaMask::all_opaque(5, 1);
        let mut clip = ClipMask::all_visible(5, 1);
        clip.set(1, 0, false);
        clip.set(3, 0, false);
        masked.set_clip(clip.clone());
        unmasked.set_clip(clip);

        masked.composite_from(&src, 0.5, BlendMode::Normal, Some(&mask));
        unmasked.composite_from(&src, 0.5, BlendMode::Normal, None);

        assert_eq!(masked.data, unmasked.data);
        assert_eq!(masked.get_pixel(1, 0), [20, 40, 60, 255]);
        assert_eq!(masked.get_pixel(3, 0), [20, 40, 60, 255]);
    }

    #[test]
    fn composite_normal_row_copies_opaque_source_over_any_destination() {
        let src = [200, 10, 20, 255, 10, 220, 20, 255, 10, 20, 240, 255];
        let mut dst = [1, 2, 3, 0, 4, 5, 6, 128, 7, 8, 9, 255];

        composite_normal_compat_row(&mut dst, &src, 1.0);

        assert_eq!(dst, src);
    }

    #[test]
    fn composite_from_soft_mask_with_binary_clip_keeps_clipped_pixels_unchanged() {
        let mut dst = PixelBuffer::new_filled(2, 1, WHITE);
        let src = PixelBuffer::new_filled(2, 1, RED);
        let mut mask = AlphaMask::all_opaque(2, 1);
        mask.set(0, 0, 255);
        mask.set(1, 0, 255);
        let mut clip = ClipMask::all_visible(2, 1);
        clip.set(0, 0, false);
        dst.set_clip(clip);

        dst.composite_from(&src, 1.0, BlendMode::Normal, Some(&mask));

        assert_eq!(dst.get_pixel(0, 0), WHITE);
        assert_eq!(dst.get_pixel(1, 0), RED);
    }

    #[test]
    fn composite_from_soft_mask_smaller_than_source_preserves_outside_mask_semantics() {
        let mut dst = PixelBuffer::new_filled(2, 1, WHITE);
        let src = PixelBuffer::new_filled(2, 1, RED);
        let mut mask = AlphaMask::all_opaque(1, 1);
        mask.set(0, 0, 0);

        dst.composite_from(&src, 1.0, BlendMode::Normal, Some(&mask));

        assert_eq!(dst.get_pixel(0, 0), WHITE);
        assert_eq!(
            dst.get_pixel(1, 0),
            RED,
            "outside the soft-mask bounds AlphaMask::get returns fully opaque"
        );
    }

    #[test]
    fn composite_from_skips_transparent_source_pixels() {
        let mut dst = PixelBuffer::new_filled(2, 1, WHITE);
        let mut src = PixelBuffer::new_transparent(2, 1);
        src.set_pixel(1, 0, RED);
        dst.composite_from(&src, 1.0, BlendMode::Normal, None);
        assert_eq!(dst.get_pixel(0, 0), WHITE, "transparent src leaves dst");
        assert_eq!(dst.get_pixel(1, 0), RED);
    }

    #[test]
    fn normal_group_composite_copies_opaque_rows_exactly() {
        let mut dst = PixelBuffer::new_filled(3, 1, WHITE);
        let mut src = PixelBuffer::new_transparent(3, 1);
        src.set_pixel(0, 0, [10, 20, 30, 255]);
        src.set_pixel(1, 0, [40, 50, 60, 255]);
        src.set_pixel(2, 0, [70, 80, 90, 255]);

        dst.composite_from(&src, 1.0, BlendMode::Normal, None);

        assert_eq!(dst.rgba_bytes(), src.rgba_bytes());
    }

    #[test]
    fn composite_from_uses_blend_mode() {
        let mut dst = PixelBuffer::new_filled(1, 1, [200, 200, 200, 255]);
        let src = PixelBuffer::new_filled(1, 1, [128, 128, 128, 255]);
        dst.composite_from(&src, 1.0, BlendMode::Multiply, None);
        // Multiply darkens: 200/255 * 128/255 ~= 100.
        assert!(dst.get_pixel(0, 0)[0] < 170, "multiply should darken");
        // The buffer's own blend mode is restored to Normal afterwards.
        assert_eq!(dst.blend_mode, BlendMode::Normal);
    }

    #[test]
    fn knockout_from_replaces_rather_than_blends() {
        // Knockout: a semi-transparent source replaces the destination's color
        // outright (alpha scaled), it does not composite over it.
        let mut dst = PixelBuffer::new_filled(1, 1, WHITE);
        let src = PixelBuffer::new_filled(1, 1, [10, 20, 30, 128]);
        dst.knockout_from(&src, 1.0, None);
        let p = dst.get_pixel(0, 0);
        assert_eq!([p[0], p[1], p[2]], [10, 20, 30], "color replaced outright");
        assert!((p[3] as i32 - 128).abs() <= 1, "alpha scaled, got {}", p[3]);
    }

    #[test]
    fn knockout_backdrop_prevents_interior_overlap_accumulation() {
        let mut normal = PixelBuffer::new_filled(1, 1, WHITE);
        normal.blend_pixel(0, 0, [255, 0, 0, 128], 1.0);
        normal.blend_pixel(0, 0, [0, 0, 255, 128], 1.0);

        let mut knockout = PixelBuffer::new_filled(1, 1, WHITE);
        knockout.set_knockout_backdrop(PixelBuffer::new_filled(1, 1, WHITE));
        knockout.blend_pixel(0, 0, [255, 0, 0, 128], 1.0);
        knockout.blend_pixel(0, 0, [0, 0, 255, 128], 1.0);

        let normal_pixel = normal.get_pixel(0, 0);
        let knockout_pixel = knockout.get_pixel(0, 0);
        assert!(
            normal_pixel[1] < knockout_pixel[1],
            "normal overlap should accumulate over the first paint: normal={normal_pixel:?} knockout={knockout_pixel:?}"
        );
        assert!(
            (knockout_pixel[0] as i32 - 127).abs() <= 2
                && (knockout_pixel[1] as i32 - 127).abs() <= 2
                && knockout_pixel[2] == 255,
            "second knockout paint should be half-blue over white: {knockout_pixel:?}"
        );
    }

    #[test]
    fn from_alpha_channel_reads_alpha_not_luminosity() {
        let mut buf = PixelBuffer::new_transparent(2, 1);
        buf.set_pixel(0, 0, [255, 255, 255, 64]); // white but low alpha
        buf.set_pixel(1, 0, [0, 0, 0, 200]); // black but high alpha
        let mask = AlphaMask::from_alpha_channel(&buf);
        assert!((mask.get(0, 0) - 64.0 / 255.0).abs() < 0.01);
        assert!((mask.get(1, 0) - 200.0 / 255.0).abs() < 0.01);
    }

    #[test]
    fn apply_transfer_lut_remaps_mask_values() {
        let mut mask = AlphaMask::all_opaque(1, 1);
        mask.set(0, 0, 100);
        // Inversion LUT: out = 255 - in.
        let mut lut = [0u8; 256];
        for (i, v) in lut.iter_mut().enumerate() {
            *v = 255 - i as u8;
        }
        mask.apply_transfer_lut(&lut);
        assert!((mask.get(0, 0) - (155.0 / 255.0)).abs() < 0.01);
    }

    #[test]
    fn new_transparent_accumulates_alpha() {
        let mut buf = PixelBuffer::new_transparent(1, 1);
        buf.blend_pixel(0, 0, [255, 0, 0, 128], 1.0);
        let first = buf.get_pixel(0, 0);
        assert!(
            (first[3] as i32 - 128).abs() <= 1,
            "first alpha should be about 128: {:?}",
            first
        );

        buf.blend_pixel(0, 0, [255, 0, 0, 128], 1.0);
        let second = buf.get_pixel(0, 0);
        assert!(
            second[3] > first[3],
            "second semi-transparent paint should accumulate alpha: {:?} -> {:?}",
            first,
            second
        );
    }
}
