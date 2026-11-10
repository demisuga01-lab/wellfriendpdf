use crate::cancel::CancelToken;
use crate::content::state::{LineCap, LineJoin};
use crate::render::buffer::{ClipMask, CoverageBlendMemo, PixelBuffer, PixelColor};
use crate::render::contract::ThinLineMode;
use crate::render::line::DashState;
use crate::render::transform::{Transform2D, Viewport};
use crate::render::vector_quality::{
    stroke_adjust_outline, stroke_adjusted_fill_rect, StrokePen, VectorRasterOptions,
};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

const ACCUMULATOR_MAX_CELLS: usize = 8 * 1024 * 1024;
const GLYPH_SCANLINE_ALPHA_VERTICAL_SAMPLES: usize = 4;
const PATH_SCANLINE_COLOR_VERTICAL_SAMPLES: usize = 4;
const SCANLINE_CROSSING_POOL_MAX_ENTRIES: usize = 16_384;
const SCANLINE_CROSSING_POOL_MAX_BUFFERS: usize = 16;
const SCANLINE_U16_POOL_MAX_CELLS: usize = 4096;
const SCANLINE_U16_POOL_MAX_BUFFERS: usize = 16;
const SCANLINE_EDGE_BUCKET_MAX_LINKS: usize = 2_000_000;

thread_local! {
    static SCANLINE_CROSSING_VEC_POOL: RefCell<Vec<Vec<(f64, i32)>>> = const { RefCell::new(Vec::new()) };
    static SCANLINE_U16_VEC_POOL: RefCell<Vec<Vec<u16>>> = const { RefCell::new(Vec::new()) };
    // Kept across fills: the memo resets itself when the colour or blend
    // state changes, so repeated same-colour strokes reuse their results.
    static COVERAGE_BLEND_MEMO: RefCell<Option<CoverageBlendMemo>> = const { RefCell::new(None) };
}

fn take_coverage_blend_memo() -> CoverageBlendMemo {
    COVERAGE_BLEND_MEMO
        .with(|memo| memo.borrow_mut().take())
        .unwrap_or_else(CoverageBlendMemo::new)
}

fn return_coverage_blend_memo(memo: CoverageBlendMemo) {
    COVERAGE_BLEND_MEMO.with(|slot| *slot.borrow_mut() = Some(memo));
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PathRasterStats {
    pub accumulator_allocations: u64,
    pub accumulator_reuses: u64,
    pub accumulator_cells: u64,
    pub banded_accumulator_bands: u64,
    pub scanline_fast_rows: u64,
    pub scanline_crossing_reuses: u64,
    pub scanline_span_pixels: u64,
    pub solid_run_pixels: u64,
    pub convex_fast_pixels: u64,
    pub edge_bucket_builds: u64,
    pub edge_bucket_links: u64,
    pub edge_bucket_rows: u64,
}

static PATH_RASTER_ACCUMULATOR_ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_ACCUMULATOR_REUSES: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_ACCUMULATOR_CELLS: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_BANDED_ACCUMULATOR_BANDS: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_SCANLINE_FAST_ROWS: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_SCANLINE_CROSSING_REUSES: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_SCANLINE_SPAN_PIXELS: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_SOLID_RUN_PIXELS: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_CONVEX_FAST_PIXELS: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_EDGE_BUCKET_BUILDS: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_EDGE_BUCKET_LINKS: AtomicU64 = AtomicU64::new(0);
static PATH_RASTER_EDGE_BUCKET_ROWS: AtomicU64 = AtomicU64::new(0);

pub fn path_raster_stats() -> PathRasterStats {
    PathRasterStats {
        accumulator_allocations: PATH_RASTER_ACCUMULATOR_ALLOCATIONS.load(Ordering::Relaxed),
        accumulator_reuses: PATH_RASTER_ACCUMULATOR_REUSES.load(Ordering::Relaxed),
        accumulator_cells: PATH_RASTER_ACCUMULATOR_CELLS.load(Ordering::Relaxed),
        banded_accumulator_bands: PATH_RASTER_BANDED_ACCUMULATOR_BANDS.load(Ordering::Relaxed),
        scanline_fast_rows: PATH_RASTER_SCANLINE_FAST_ROWS.load(Ordering::Relaxed),
        scanline_crossing_reuses: PATH_RASTER_SCANLINE_CROSSING_REUSES.load(Ordering::Relaxed),
        scanline_span_pixels: PATH_RASTER_SCANLINE_SPAN_PIXELS.load(Ordering::Relaxed),
        solid_run_pixels: PATH_RASTER_SOLID_RUN_PIXELS.load(Ordering::Relaxed),
        convex_fast_pixels: PATH_RASTER_CONVEX_FAST_PIXELS.load(Ordering::Relaxed),
        edge_bucket_builds: PATH_RASTER_EDGE_BUCKET_BUILDS.load(Ordering::Relaxed),
        edge_bucket_links: PATH_RASTER_EDGE_BUCKET_LINKS.load(Ordering::Relaxed),
        edge_bucket_rows: PATH_RASTER_EDGE_BUCKET_ROWS.load(Ordering::Relaxed),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PathSegment {
    MoveTo(f64, f64),
    LineTo(f64, f64),
    CubicTo {
        cp1x: f64,
        cp1y: f64,
        cp2x: f64,
        cp2y: f64,
        x: f64,
        y: f64,
    },
    ClosePath,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphHinting {
    pixel_size: f64,
}

impl GlyphHinting {
    pub fn disabled() -> Self {
        Self { pixel_size: 0.0 }
    }

    pub fn light(pixel_size: f64) -> Self {
        Self { pixel_size }
    }

    pub(crate) fn should_apply(self) -> bool {
        self.pixel_size.is_finite() && (7.0..=32.0).contains(&self.pixel_size)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    pub segments: Vec<PathSegment>,
    pub current_point: Option<(f64, f64)>,
    subpath_start: Option<(f64, f64)>,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(&mut self, x: f64, y: f64) {
        self.segments.push(PathSegment::MoveTo(x, y));
        self.current_point = Some((x, y));
        self.subpath_start = Some((x, y));
    }

    pub fn line_to(&mut self, x: f64, y: f64) {
        if self.current_point.is_none() {
            self.move_to(x, y);
            return;
        }
        self.segments.push(PathSegment::LineTo(x, y));
        self.current_point = Some((x, y));
    }

    pub fn curve_to(&mut self, cp1x: f64, cp1y: f64, cp2x: f64, cp2y: f64, x: f64, y: f64) {
        if self.current_point.is_none() {
            self.move_to(x, y);
            return;
        }
        self.segments.push(PathSegment::CubicTo {
            cp1x,
            cp1y,
            cp2x,
            cp2y,
            x,
            y,
        });
        self.current_point = Some((x, y));
    }

    pub fn close(&mut self) {
        if self.subpath_start.is_some() {
            self.segments.push(PathSegment::ClosePath);
            self.current_point = self.subpath_start;
        }
    }

    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.move_to(x, y);
        self.line_to(x + w, y);
        self.line_to(x + w, y + h);
        self.line_to(x, y + h);
        self.close();
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn clear(&mut self) {
        self.segments.clear();
        self.current_point = None;
        self.subpath_start = None;
    }
}

#[derive(Debug, Clone, Default)]
pub struct FlatPath {
    pub subpaths: Vec<Vec<(f64, f64)>>,
    pub closed: Vec<bool>,
}

/// Distance from point P to the line through A and B.
pub(crate) fn point_to_line_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-10 {
        return ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt();
    }
    ((dy * p.0 - dx * p.1 + b.0 * a.1 - b.1 * a.0) / len).abs()
}

fn midpoint(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

type CubicControlPoints = ((f64, f64), (f64, f64), (f64, f64), (f64, f64));

/// Flatten a cubic Bezier curve into endpoint/intermediate points.
pub fn flatten_cubic(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    threshold: f64,
    max_depth: u32,
    out: &mut Vec<(f64, f64)>,
) {
    let mut splits = cubic_monotonic_split_parameters(p0, p1, p2, p3);
    if splits.is_empty() {
        flatten_cubic_recursive(p0, p1, p2, p3, threshold, max_depth, out);
        return;
    }

    let mut curve = (p0, p1, p2, p3);
    let mut previous = 0.0;
    for split in splits.drain(..) {
        let denom = 1.0_f64 - previous;
        if denom <= 1e-12 {
            break;
        }
        let local_t = ((split - previous) / denom).clamp(0.0, 1.0);
        if local_t <= 1e-12 || local_t >= 1.0 - 1e-12 {
            previous = split;
            continue;
        }
        let (left, right) = split_cubic(curve, local_t);
        flatten_cubic_recursive(left.0, left.1, left.2, left.3, threshold, max_depth, out);
        curve = right;
        previous = split;
    }
    flatten_cubic_recursive(
        curve.0, curve.1, curve.2, curve.3, threshold, max_depth, out,
    );
}

fn flatten_cubic_recursive(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    threshold: f64,
    max_depth: u32,
    out: &mut Vec<(f64, f64)>,
) {
    if max_depth == 0 {
        out.push(p3);
        return;
    }

    let threshold = threshold.max(0.01);
    let d1 = point_to_line_dist(p1, p0, p3);
    let d2 = point_to_line_dist(p2, p0, p3);
    if d1 <= threshold && d2 <= threshold {
        out.push(p3);
        return;
    }

    let q01 = midpoint(p0, p1);
    let q12 = midpoint(p1, p2);
    let q23 = midpoint(p2, p3);
    let q012 = midpoint(q01, q12);
    let q123 = midpoint(q12, q23);
    let q0123 = midpoint(q012, q123);

    flatten_cubic_recursive(p0, q01, q012, q0123, threshold, max_depth - 1, out);
    flatten_cubic_recursive(q0123, q123, q23, p3, threshold, max_depth - 1, out);
}

fn split_cubic(curve: CubicControlPoints, t: f64) -> (CubicControlPoints, CubicControlPoints) {
    let (p0, p1, p2, p3) = curve;
    let q01 = lerp_point(p0, p1, t);
    let q12 = lerp_point(p1, p2, t);
    let q23 = lerp_point(p2, p3, t);
    let q012 = lerp_point(q01, q12, t);
    let q123 = lerp_point(q12, q23, t);
    let q0123 = lerp_point(q012, q123, t);
    ((p0, q01, q012, q0123), (q0123, q123, q23, p3))
}

fn lerp_point(a: (f64, f64), b: (f64, f64), t: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn cubic_monotonic_split_parameters(
    p0: (f64, f64),
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
) -> Vec<f64> {
    let mut roots = Vec::with_capacity(4);
    append_cubic_axis_derivative_roots(p0.0, p1.0, p2.0, p3.0, &mut roots);
    append_cubic_axis_derivative_roots(p0.1, p1.1, p2.1, p3.1, &mut roots);
    roots.sort_by(f64::total_cmp);
    roots.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
    roots
}

fn append_cubic_axis_derivative_roots(p0: f64, p1: f64, p2: f64, p3: f64, roots: &mut Vec<f64>) {
    if [p0, p1, p2, p3].iter().any(|v| !v.is_finite()) {
        return;
    }
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 3.0 * p0 - 6.0 * p1 + 3.0 * p2;
    let c = -3.0 * p0 + 3.0 * p1;
    let qa = 3.0 * a;
    let qb = 2.0 * b;
    let qc = c;
    let eps = 1e-12;

    if qa.abs() <= eps {
        if qb.abs() > eps {
            push_unit_interval_root(-qc / qb, roots);
        }
        return;
    }

    let discriminant = qb * qb - 4.0 * qa * qc;
    if discriminant < -eps {
        return;
    }
    if discriminant.abs() <= eps {
        push_unit_interval_root(-qb / (2.0 * qa), roots);
        return;
    }
    let sqrt_d = discriminant.sqrt();
    push_unit_interval_root((-qb - sqrt_d) / (2.0 * qa), roots);
    push_unit_interval_root((-qb + sqrt_d) / (2.0 * qa), roots);
}

fn push_unit_interval_root(t: f64, roots: &mut Vec<f64>) {
    if t.is_finite() && t > 1e-9 && t < 1.0 - 1e-9 {
        roots.push(t);
    }
}

/// Flatten a path from PDF user space to pixel-space polylines.
pub fn flatten_path(
    path: &Path,
    ctm: &Transform2D,
    viewport: &Viewport,
    bezier_threshold: f64,
) -> FlatPath {
    flatten_path_with_point_mapper(path, bezier_threshold, |x, y| {
        path_point_to_device(x, y, ctm, viewport)
    })
}

/// A device-space coordinate written for vector output (PostScript, SVG)
/// with two decimals and no trailing zeros (`12.5`, `3`, `-0.25`), without
/// allocating.
#[derive(Clone, Copy)]
pub(crate) struct CompactNum(pub(crate) f64);

impl std::fmt::Display for CompactNum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hundredths = (self.0 * 100.0).round();
        if !hundredths.is_finite() || hundredths == 0.0 {
            return f.write_str("0");
        }
        if hundredths.abs() >= 1e15 {
            return write!(f, "{:.2}", self.0);
        }
        let hundredths = hundredths as i64;
        let sign = if hundredths < 0 { "-" } else { "" };
        let magnitude = hundredths.unsigned_abs();
        let (whole, frac) = (magnitude / 100, magnitude % 100);
        match frac {
            0 => write!(f, "{sign}{whole}"),
            f2 if f2 % 10 == 0 => write!(f, "{sign}{whole}.{}", f2 / 10),
            f2 => write!(f, "{sign}{whole}.{f2:02}"),
        }
    }
}

/// Map a path from PDF user space to device-pixel space keeping its Bézier
/// segments (an affine map takes a cubic to a cubic, so nothing is lost).
/// Vector output backends emit the result as `curveto` / SVG `C` segments
/// instead of a flattened polyline. Returns `None` when any point maps to a
/// non-finite device coordinate, so the caller can fall back to
/// [`flatten_path`], which drops such points subpath by subpath.
pub(crate) fn device_curve_path(
    path: &Path,
    ctm: &Transform2D,
    viewport: &Viewport,
) -> Option<Path> {
    map_curve_path(path, |x, y| path_point_to_device(x, y, ctm, viewport))
}

fn map_curve_path<F>(path: &Path, mut to_px: F) -> Option<Path>
where
    F: FnMut(f64, f64) -> Option<(f64, f64)>,
{
    let mut out = Path::new();
    out.segments.reserve(path.segments.len());
    for seg in &path.segments {
        match *seg {
            PathSegment::MoveTo(x, y) => {
                let (x, y) = to_px(x, y)?;
                out.move_to(x, y);
            }
            PathSegment::LineTo(x, y) => {
                let (x, y) = to_px(x, y)?;
                out.line_to(x, y);
            }
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                let (ax, ay) = to_px(cp1x, cp1y)?;
                let (bx, by) = to_px(cp2x, cp2y)?;
                let (x, y) = to_px(x, y)?;
                out.curve_to(ax, ay, bx, by, x, y);
            }
            PathSegment::ClosePath => out.close(),
        }
    }
    Some(out)
}

pub(crate) fn flatten_path_device_transform(
    path: &Path,
    device_t: &Transform2D,
    bezier_threshold: f64,
) -> FlatPath {
    flatten_path_with_point_mapper(path, bezier_threshold, |x, y| {
        finite_point(device_t.transform_point(x, y))
    })
}

/// Coarsest curve-flattening tolerance, in device pixels, the rasterizer
/// accepts. Poppler/Splash fixes flatness at one pixel (Acrobat ignores the
/// PDF `i` operator too); a file asking for a coarser tolerance must not get
/// visibly faceted curves. Finer tolerances are kept.
pub(crate) const MAX_DEVICE_FLATNESS: f64 = 1.0;

/// Clamp a requested device-space flattening tolerance to at most
/// [`MAX_DEVICE_FLATNESS`]; non-finite values fall back to the half-pixel
/// default (the flattener separately enforces a 0.01 px floor).
pub(crate) fn effective_device_flatness(bezier_threshold: f64) -> f64 {
    if bezier_threshold.is_finite() {
        bezier_threshold.min(MAX_DEVICE_FLATNESS)
    } else {
        0.5
    }
}

fn flatten_path_with_point_mapper<F>(path: &Path, bezier_threshold: f64, mut to_px: F) -> FlatPath
where
    F: FnMut(f64, f64) -> Option<(f64, f64)>,
{
    let bezier_threshold = effective_device_flatness(bezier_threshold);
    let mut flat = FlatPath::default();
    let mut current_subpath = Vec::new();
    let mut current_start: Option<(f64, f64)> = None;
    let mut is_closed = false;
    let mut pen: Option<(f64, f64)> = None;

    for seg in &path.segments {
        match *seg {
            PathSegment::MoveTo(x, y) => {
                finish_flat_subpath(&mut flat, &mut current_subpath, is_closed);
                is_closed = false;
                current_start = None;
                pen = Some((x, y));
                if let Some(px) = to_px(x, y) {
                    current_start = Some(px);
                    current_subpath.push(px);
                }
            }
            PathSegment::LineTo(x, y) => {
                if let Some(px) = to_px(x, y) {
                    if current_subpath.is_empty() {
                        current_start = Some(px);
                    }
                    current_subpath.push(px);
                } else {
                    finish_flat_subpath(&mut flat, &mut current_subpath, is_closed);
                    current_start = None;
                    is_closed = false;
                }
                pen = Some((x, y));
            }
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                let p0 = pen.and_then(|(x, y)| to_px(x, y));
                let p1 = to_px(cp1x, cp1y);
                let p2 = to_px(cp2x, cp2y);
                let p3 = to_px(x, y);
                if let (Some(p0), Some(p1), Some(p2), Some(p3)) = (p0, p1, p2, p3) {
                    if current_subpath.is_empty() {
                        current_start = Some(p0);
                        current_subpath.push(p0);
                    }
                    flatten_cubic(p0, p1, p2, p3, bezier_threshold, 16, &mut current_subpath);
                } else {
                    finish_flat_subpath(&mut flat, &mut current_subpath, is_closed);
                    current_start = None;
                    is_closed = false;
                    if let Some(p3) = p3 {
                        current_start = Some(p3);
                        current_subpath.push(p3);
                    }
                }
                pen = Some((x, y));
            }
            PathSegment::ClosePath => {
                if !current_subpath.is_empty() {
                    if let Some(start) = current_start {
                        current_subpath.push(start);
                    }
                    is_closed = true;
                }
            }
        }
    }

    finish_flat_subpath(&mut flat, &mut current_subpath, is_closed);
    flat
}

fn finish_flat_subpath(
    flat: &mut FlatPath,
    current_subpath: &mut Vec<(f64, f64)>,
    is_closed: bool,
) {
    if !current_subpath.is_empty() {
        flat.subpaths.push(std::mem::take(current_subpath));
        flat.closed.push(is_closed);
    }
}

fn finite_point(point: (f64, f64)) -> Option<(f64, f64)> {
    if point.0.is_finite() && point.1.is_finite() {
        Some(point)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RasterizedGlyphMask {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// One coverage byte per pixel, or (for an LCD mask) three per pixel:
    /// the red, green and blue sub-pixel coverage, left to right.
    alpha: Vec<u8>,
    lcd: bool,
}

impl RasterizedGlyphMask {
    pub(crate) fn from_alpha(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        alpha: Vec<u8>,
    ) -> Option<Self> {
        if width == 0 || height == 0 || alpha.len() != width as usize * height as usize {
            return None;
        }
        Some(Self {
            x,
            y,
            width,
            height,
            alpha,
            lcd: false,
        })
    }

    /// Whether this mask carries separate red/green/blue sub-pixel coverage.
    pub(crate) fn is_lcd(&self) -> bool {
        self.lcd
    }

    pub(crate) fn paint(&self, buf: &mut PixelBuffer, dx: i32, dy: i32, color: PixelColor) {
        if self.lcd {
            buf.blend_lcd_coverage_mask(
                dx.saturating_add(self.x),
                dy.saturating_add(self.y),
                self.width,
                self.height,
                &self.alpha,
                color,
            );
            return;
        }
        buf.blend_alpha_mask(
            dx.saturating_add(self.x),
            dy.saturating_add(self.y),
            self.width,
            self.height,
            &self.alpha,
            color,
        );
    }

    pub(crate) fn approximate_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.alpha.len()
    }

    pub(crate) fn alpha_slice(&self) -> &[u8] {
        &self.alpha
    }

    /// Remap coverage through a vector coverage curve (`None` = linear).
    pub(crate) fn apply_coverage_lut(&mut self, lut: Option<&[u8; 256]>) {
        crate::render::vector_quality::apply_coverage_lut(&mut self.alpha, lut);
    }

    pub(crate) fn force_binary_alpha(&mut self) {
        for alpha in &mut self.alpha {
            *alpha = if *alpha >= 128 { 255 } else { 0 };
        }
    }

    pub(crate) fn union_into_clip_mask(&self, clip: &mut ClipMask, dx: i32, dy: i32) {
        let x0 = dx.saturating_add(self.x);
        let y0 = dy.saturating_add(self.y);
        clip.union_alpha_mask(x0, y0, self.width, self.height, &self.alpha);
    }
}

pub(crate) fn rasterize_glyph_alpha_mask(
    path: &Path,
    device_t: &Transform2D,
    rule: FillRule,
    hinting: GlyphHinting,
) -> Option<RasterizedGlyphMask> {
    if path.is_empty() {
        return None;
    }
    let mut flat = flatten_path_device_transform(path, device_t, 0.2);
    if hinting.should_apply() {
        light_grid_fit_flat_glyph(&mut flat, device_t);
    }
    rasterize_flat_alpha_mask(&flat, rule)
}

/// FreeType's default LCD filter (`FT_LCD_FILTER_DEFAULT`), as PDFium applies
/// it to 3x horizontal glyph bitmaps (`cfx_face.cpp`).
const LCD_FILTER_WEIGHTS: [u16; 5] = [8, 77, 86, 77, 8];

/// Rasterize a glyph outline for LCD (sub-pixel) text, the way PDFium builds
/// its LCD glyph bitmaps: the outline is scan converted at three times the
/// horizontal resolution, each 3x row is run through the 5-tap
/// [`LCD_FILTER_WEIGHTS`] filter (with two sub-pixels of padding on each
/// side), and every output pixel takes three consecutive filtered samples as
/// its red, green and blue coverage (red leftmost, the usual RGB stripe).
///
/// `phase` (0..=2) shifts the outline right by that many sub-pixels, which is
/// how PDFium positions an LCD glyph whose origin is floored to a whole
/// pixel. `device_t` maps glyph space to device pixels as for
/// [`rasterize_glyph_alpha_mask`]; light hinting runs in device pixels before
/// the horizontal scale is applied.
pub(crate) fn rasterize_glyph_lcd_mask(
    path: &Path,
    device_t: &Transform2D,
    rule: FillRule,
    hinting: GlyphHinting,
    phase: u8,
) -> Option<RasterizedGlyphMask> {
    if path.is_empty() {
        return None;
    }
    let mut flat = flatten_path_device_transform(path, device_t, 0.2);
    if hinting.should_apply() {
        light_grid_fit_flat_glyph(&mut flat, device_t);
    }
    let phase = f64::from(phase.min(2));
    for subpath in &mut flat.subpaths {
        for point in subpath {
            point.0 = point.0 * 3.0 + phase;
        }
    }
    let (sub_x0, y0, sub_w, h) = flat_alpha_mask_bounds(&flat)?;
    // Two sub-pixels of filter padding on each side, widened to whole pixels.
    let pad_start = i64::from(sub_x0) - 2;
    let pad_end = i64::from(sub_x0) + sub_w as i64 + 2;
    let px0 = pad_start.div_euclid(3);
    let px1 = (pad_end + 2).div_euclid(3);
    let width = usize::try_from(px1 - px0).ok().filter(|width| *width > 0)?;
    let x = i32::try_from(px0).ok()?;
    let sub_row = width.checked_mul(3)?;
    let offset = usize::try_from(i64::from(sub_x0) - px0 * 3).ok()?;
    if offset.checked_add(sub_w)? > sub_row || sub_row.checked_mul(h)? > ACCUMULATOR_MAX_CELLS {
        return None;
    }
    let mut coverage = vec![0u8; sub_row * h];
    let mut raw = vec![0u8; sub_row];
    scan_flat_alpha_mask_rows(
        &flat,
        rule,
        sub_x0,
        y0,
        sub_w,
        h,
        GLYPH_SCANLINE_ALPHA_VERTICAL_SAMPLES,
        |y, row| {
            raw.fill(0);
            for (dst, value) in raw[offset..offset + sub_w].iter_mut().zip(row.iter()) {
                *dst = (*value).min(255) as u8;
            }
            let start = (y - y0) as usize * sub_row;
            lcd_filter_row(&raw, &mut coverage[start..start + sub_row]);
        },
    );
    Some(RasterizedGlyphMask {
        x,
        y: y0,
        width: width as u32,
        height: h as u32,
        alpha: coverage,
        lcd: true,
    })
}

/// Apply [`LCD_FILTER_WEIGHTS`] to one row of 3x coverage, exactly as
/// PDFium's `ft_smooth_lcd_spans()` port does (saturating sum of the rounded
/// per-tap contributions).
fn lcd_filter_row(raw: &[u8], filtered: &mut [u8]) {
    let len = raw.len().min(filtered.len());
    filtered[..len].fill(0);
    for (x, &value) in raw[..len].iter().enumerate() {
        if value == 0 {
            continue;
        }
        for (tap, weight) in LCD_FILTER_WEIGHTS.iter().enumerate() {
            let Some(dst_x) = (x + tap).checked_sub(2).filter(|dst_x| *dst_x < len) else {
                continue;
            };
            let contribution = ((u16::from(value) * weight + 85) >> 8) as u8;
            filtered[dst_x] = filtered[dst_x].saturating_add(contribution);
        }
    }
}

pub struct PathPainter;

impl PathPainter {
    pub fn stroke(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
        dash: &DashState,
    ) {
        Self::stroke_internal(
            buf,
            path,
            ctm,
            viewport,
            color,
            stroke_width,
            dash,
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_with_cap(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
        dash: &DashState,
        cap: &LineCap,
    ) {
        Self::stroke_internal(
            buf,
            path,
            ctm,
            viewport,
            color,
            stroke_width,
            dash,
            cap.clone(),
            LineJoin::Miter,
            10.0,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_with_style(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
        dash: &DashState,
        cap: &LineCap,
        join: &LineJoin,
        miter_limit: f64,
    ) {
        Self::stroke_internal(
            buf,
            path,
            ctm,
            viewport,
            color,
            stroke_width,
            dash,
            cap.clone(),
            join.clone(),
            miter_limit,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_with_style_cancellable(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
        dash: &DashState,
        cap: &LineCap,
        join: &LineJoin,
        miter_limit: f64,
        cancel: &CancelToken,
    ) -> bool {
        Self::stroke_with_style_cancellable_with_flatness(
            buf,
            path,
            ctm,
            viewport,
            color,
            stroke_width,
            dash,
            cap,
            join,
            miter_limit,
            0.2,
            cancel,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_with_style_cancellable_with_flatness(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
        dash: &DashState,
        cap: &LineCap,
        join: &LineJoin,
        miter_limit: f64,
        bezier_threshold: f64,
        cancel: &CancelToken,
    ) -> bool {
        stroke_impl(
            buf,
            StrokeRequest {
                path,
                ctm,
                viewport,
                color,
                stroke_width,
                dash,
                cap,
                join,
                miter_limit,
                bezier_threshold,
            },
            StrokeScan::Antialiased,
            VectorRasterOptions::for_glyph_stroke(),
            cancel,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_with_style_fast_cancellable(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
        dash: &DashState,
        cap: &LineCap,
        join: &LineJoin,
        miter_limit: f64,
        cancel: &CancelToken,
    ) -> bool {
        Self::stroke_with_style_fast_cancellable_with_flatness(
            buf,
            path,
            ctm,
            viewport,
            color,
            stroke_width,
            dash,
            cap,
            join,
            miter_limit,
            0.5,
            cancel,
        )
    }

    /// Stroke with binary (aliased) scan conversion. Callers that want
    /// anti-aliasing use [`Self::stroke_with_style_cancellable_with_flatness`]
    /// or the crate-internal option-aware entry point.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_with_style_fast_cancellable_with_flatness(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
        dash: &DashState,
        cap: &LineCap,
        join: &LineJoin,
        miter_limit: f64,
        bezier_threshold: f64,
        cancel: &CancelToken,
    ) -> bool {
        stroke_impl(
            buf,
            StrokeRequest {
                path,
                ctm,
                viewport,
                color,
                stroke_width,
                dash,
                cap,
                join,
                miter_limit,
                bezier_threshold,
            },
            StrokeScan::Binary,
            VectorRasterOptions::for_glyph_stroke(),
            cancel,
        )
    }

    /// Stroke an ordinary (non-glyph) path with the renderer's vector
    /// raster policies: stroke adjustment, thin-line mode and coverage gamma
    /// from `options`, a transformed (user-space) pen, and anti-aliasing for
    /// every stroke shape (dashed, very large or very complex strokes
    /// included) unless `binary_alpha` asks for aliased output.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stroke_path_with_options(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
        dash: &DashState,
        cap: &LineCap,
        join: &LineJoin,
        miter_limit: f64,
        bezier_threshold: f64,
        binary_alpha: bool,
        options: VectorRasterOptions,
        cancel: &CancelToken,
    ) -> bool {
        stroke_impl(
            buf,
            StrokeRequest {
                path,
                ctm,
                viewport,
                color,
                stroke_width,
                dash,
                cap,
                join,
                miter_limit,
                bezier_threshold,
            },
            if binary_alpha {
                StrokeScan::Binary
            } else {
                StrokeScan::Antialiased
            },
            options,
            cancel,
        )
    }

    /// Fill an ordinary (non-glyph) path with the renderer's vector raster
    /// policies: stroke-adjusted rectangles and coverage gamma from
    /// `options`, and anti-aliasing for every path size and complexity unless
    /// `binary_alpha` asks for aliased output. Memory stays bounded: the
    /// anti-aliased scan converter works one device row at a time over the
    /// path's bounds clipped to the buffer and the active clip.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fill_path_with_options(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        rule: FillRule,
        bezier_threshold: f64,
        binary_alpha: bool,
        options: VectorRasterOptions,
        cancel: &CancelToken,
    ) -> bool {
        if cancel.is_cancelled() {
            return false;
        }
        if path.is_empty() || buf.width == 0 || buf.height == 0 {
            return true;
        }
        if options.stroke_adjust {
            let device_t = ctm.concat(&viewport.to_transform());
            if let Some((x, y, w, h)) = stroke_adjusted_fill_rect(path, &device_t) {
                buf.fill_rect(x, y, w, h, color);
                return true;
            }
        }
        if let Some((x, y, w, h)) = axis_aligned_integer_rect(path, ctm, viewport) {
            buf.fill_rect(x, y, w, h, color);
            return true;
        }
        let flat = flatten_path(path, ctm, viewport, bezier_threshold);
        if cancel.is_cancelled() {
            return false;
        }
        if binary_alpha {
            return fill_flat_scanline_fast(buf, &flat, color, rule, cancel);
        }
        fill_flat_antialiased(buf, &flat, color, rule, cancel, options.coverage_lut())
    }

    #[allow(clippy::too_many_arguments)]
    fn stroke_internal(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
        dash: &DashState,
        cap: LineCap,
        join: LineJoin,
        miter_limit: f64,
    ) {
        let cancel = CancelToken::new();
        let _ = stroke_impl(
            buf,
            StrokeRequest {
                path,
                ctm,
                viewport,
                color,
                stroke_width,
                dash,
                cap: &cap,
                join: &join,
                miter_limit,
                bezier_threshold: 0.2,
            },
            StrokeScan::Antialiased,
            VectorRasterOptions::for_glyph_stroke(),
            &cancel,
        );
    }

    /// Fill a path with **analytic, coverage-based antialiasing**.
    ///
    /// Each edge contributes exact signed area+cover to a pixel-local
    /// accumulation buffer (the technique used by FreeType's smooth rasteriser
    /// and `font-rs`), giving true sub-pixel coverage in BOTH axes rather than
    /// the previous hard, integer-snapped scanline spans. Glyphs route through
    /// this path too (see [`crate::render::font_rasterizer`]), so text gains the
    /// same crisp AA. Coverage is composited via [`PixelBuffer::blend_pixel`],
    /// which uses the buffer's render mode: sRGB-space for Compat, linear light
    /// for opt-in HighQuality.
    pub fn fill(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        rule: FillRule,
    ) {
        Self::fill_with_flatness(buf, path, ctm, viewport, color, rule, 0.3);
    }

    pub fn fill_with_flatness(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        rule: FillRule,
        bezier_threshold: f64,
    ) {
        if path.is_empty() || buf.width == 0 || buf.height == 0 {
            return;
        }
        if let Some((x, y, w, h)) = axis_aligned_integer_rect(path, ctm, viewport) {
            buf.fill_rect(x, y, w, h, color);
            return;
        }
        // Every path size and complexity is anti-aliased; the row-streaming
        // scan converter keeps memory bounded by the clipped path width.
        let flat = flatten_path(path, ctm, viewport, bezier_threshold);
        fill_flat_aa(buf, &flat, color, rule);
    }

    pub fn fill_cancellable(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        rule: FillRule,
        cancel: &CancelToken,
    ) -> bool {
        Self::fill_cancellable_with_flatness(buf, path, ctm, viewport, color, rule, 0.3, cancel)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_cancellable_with_flatness(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        rule: FillRule,
        bezier_threshold: f64,
        cancel: &CancelToken,
    ) -> bool {
        if cancel.is_cancelled() {
            return false;
        }
        if let Some((x, y, w, h)) = axis_aligned_integer_rect(path, ctm, viewport) {
            buf.fill_rect(x, y, w, h, color);
            return true;
        }
        let flat = flatten_path(path, ctm, viewport, bezier_threshold);
        fill_flat_aa_cancellable(buf, &flat, color, rule, cancel)
    }

    pub fn fill_fast_cancellable(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        rule: FillRule,
        cancel: &CancelToken,
    ) -> bool {
        Self::fill_fast_cancellable_with_flatness(
            buf, path, ctm, viewport, color, rule, 0.5, cancel,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_fast_cancellable_with_flatness(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        rule: FillRule,
        bezier_threshold: f64,
        cancel: &CancelToken,
    ) -> bool {
        if cancel.is_cancelled() {
            return false;
        }
        if let Some((x, y, w, h)) = axis_aligned_integer_rect(path, ctm, viewport) {
            buf.fill_rect(x, y, w, h, color);
            return true;
        }
        let flat = flatten_path(path, ctm, viewport, bezier_threshold);
        fill_flat_scanline_fast(buf, &flat, color, rule, cancel)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fill_device_cmyk_overprint_preview_with_flatness(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        cmyk: [f32; 4],
        alpha: f32,
        overprint_mode: i32,
        rule: FillRule,
        bezier_threshold: f64,
        binary_alpha: bool,
    ) {
        if path.is_empty() || buf.width == 0 || buf.height == 0 {
            return;
        }
        let flat = flatten_path(path, ctm, viewport, bezier_threshold);
        if binary_alpha {
            let _ = fill_flat_cmyk_overprint_preview_scanline(
                buf,
                &flat,
                cmyk,
                alpha,
                overprint_mode,
                rule,
            );
        } else {
            fill_flat_cmyk_overprint_preview(buf, &flat, cmyk, alpha, overprint_mode, rule);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn stroke_device_cmyk_overprint_preview_with_flatness(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        cmyk: [f32; 4],
        alpha: f32,
        overprint_mode: i32,
        stroke_width: f64,
        dash: &DashState,
        cap: &LineCap,
        join: &LineJoin,
        miter_limit: f64,
        bezier_threshold: f64,
        binary_alpha: bool,
        options: VectorRasterOptions,
    ) {
        if path.is_empty() || buf.width == 0 || buf.height == 0 {
            return;
        }
        let flat = flatten_path(path, ctm, viewport, bezier_threshold);
        let device_t = ctm.concat(&viewport.to_transform());
        let stroke = device_stroke_outline(
            &flat,
            &device_t,
            stroke_width,
            dash,
            cap,
            join,
            miter_limit,
            options,
        );
        let alpha = (f64::from(alpha) * stroke.paint.alpha_scale.min(1.0)) as f32;
        let binary_alpha = binary_alpha || stroke.paint.binary;
        let outline = stroke.outline;
        if !outline.subpaths.is_empty() {
            if binary_alpha {
                let _ = fill_flat_cmyk_overprint_preview_scanline(
                    buf,
                    &outline,
                    cmyk,
                    alpha,
                    overprint_mode,
                    FillRule::NonZero,
                );
            } else {
                fill_flat_cmyk_overprint_preview(
                    buf,
                    &outline,
                    cmyk,
                    alpha,
                    overprint_mode,
                    FillRule::NonZero,
                );
            }
        }
    }

    /// Fill a glyph outline using the shared analytic coverage rasterizer.
    ///
    /// Glyph curves use a tighter 0.2px flattening tolerance than general PDF
    /// paths. The default text path is neutral grayscale coverage composited by
    /// [`PixelBuffer::blend_pixel`] in Compat mode's sRGB byte space, matching
    /// Poppler/Splash's proof-rendering convention. Optional light grid-fitting
    /// is only applied when the caller explicitly supplies an enabled
    /// [`GlyphHinting`] value.
    pub fn fill_glyph(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        rule: FillRule,
        hinting: GlyphHinting,
    ) {
        if path.is_empty() || buf.width == 0 || buf.height == 0 {
            return;
        }
        let mut flat = flatten_path(path, ctm, viewport, 0.2);
        if hinting.should_apply() {
            let device_t = ctm.concat(&viewport.to_transform());
            light_grid_fit_flat_glyph(&mut flat, &device_t);
        }
        fill_flat_aa(buf, &flat, color, rule);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_glyph_binary_cancellable(
        buf: &mut PixelBuffer,
        path: &Path,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        rule: FillRule,
        hinting: GlyphHinting,
        cancel: &CancelToken,
    ) -> bool {
        if cancel.is_cancelled() {
            return false;
        }
        if path.is_empty() || buf.width == 0 || buf.height == 0 {
            return true;
        }
        let mut flat = flatten_path(path, ctm, viewport, 0.2);
        if hinting.should_apply() {
            let device_t = ctm.concat(&viewport.to_transform());
            light_grid_fit_flat_glyph(&mut flat, &device_t);
        }
        fill_flat_scanline_fast(buf, &flat, color, rule, cancel)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_rect(
        buf: &mut PixelBuffer,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
    ) {
        if !ctm.is_axis_aligned() {
            let mut path = Path::new();
            path.rect(x, y, w, h);
            Self::fill(buf, &path, ctm, viewport, color, FillRule::NonZero);
            return;
        }

        let (ux0, uy0) = ctm.transform_point(x, y);
        let (px0, py0) = viewport.page_to_pixel_f64(ux0, uy0);
        let (ux1, uy1) = ctm.transform_point(x + w, y + h);
        let (px1, py1) = viewport.page_to_pixel_f64(ux1, uy1);

        let rx_min = safe_ceil_i32(px0.min(px1));
        let ry_min = safe_ceil_i32(py0.min(py1));
        let rx_max = safe_floor_i32(px0.max(px1));
        let ry_max = safe_floor_i32(py0.max(py1));
        let rw = (rx_max - rx_min + 1).max(0);
        let rh = (ry_max - ry_min + 1).max(0);
        buf.fill_rect(rx_min, ry_min, rw, rh, color);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn stroke_rect(
        buf: &mut PixelBuffer,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        ctm: &Transform2D,
        viewport: &Viewport,
        color: PixelColor,
        stroke_width: f64,
    ) {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        Self::stroke(
            buf,
            &path,
            ctm,
            viewport,
            color,
            stroke_width,
            &DashState::solid(),
        );
    }
}

// ---------------------------------------------------------------------------
// Shared stroke pipeline
// ---------------------------------------------------------------------------

struct StrokeRequest<'a> {
    path: &'a Path,
    ctm: &'a Transform2D,
    viewport: &'a Viewport,
    color: PixelColor,
    stroke_width: f64,
    dash: &'a DashState,
    cap: &'a LineCap,
    join: &'a LineJoin,
    miter_limit: f64,
    bezier_threshold: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StrokeScan {
    Antialiased,
    Binary,
}

/// Paint modifiers a thin-line policy applies to one stroke.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StrokePaintPolicy {
    /// Multiplier for the paint opacity (`ThinLineMode::Shape`).
    pub(crate) alpha_scale: f64,
    /// Threshold coverage to fully opaque / transparent (`ThinLineMode::Solid`).
    pub(crate) binary: bool,
}

impl StrokePaintPolicy {
    const NORMAL: Self = Self {
        alpha_scale: 1.0,
        binary: false,
    };

    pub(crate) fn apply_to_color(self, color: PixelColor) -> PixelColor {
        if self.alpha_scale >= 1.0 {
            return color;
        }
        let alpha = (f64::from(color[3]) * self.alpha_scale.max(0.0))
            .round()
            .clamp(0.0, 255.0) as u8;
        [color[0], color[1], color[2], alpha]
    }
}

/// A device-space stroke outline plus the paint policy for it.
pub(crate) struct DeviceStrokeOutline {
    pub(crate) outline: FlatPath,
    pub(crate) paint: StrokePaintPolicy,
}

fn thin_line_paint_policy(
    mode: ThinLineMode,
    line_width: f64,
    true_device_width: f64,
) -> StrokePaintPolicy {
    let thin = true_device_width.is_nan() || true_device_width < 1.0;
    match mode {
        ThinLineMode::None => StrokePaintPolicy::NORMAL,
        ThinLineMode::Solid if thin => StrokePaintPolicy {
            alpha_scale: 1.0,
            binary: true,
        },
        ThinLineMode::Shape if thin && line_width > 0.0 && true_device_width.is_finite() => {
            StrokePaintPolicy {
                alpha_scale: true_device_width.max(1.0 / 255.0),
                binary: false,
            }
        }
        _ => StrokePaintPolicy::NORMAL,
    }
}

/// Build the device-space outline of a stroke.
///
/// `flat` is the stroke's centreline flattened in device space through
/// `device_t`; `line_width` and `dash` are in user space. Conformal device
/// transforms stroke directly in device space (exact for a round pen). Other
/// transforms stroke in pen space, so the pen is the transformed ellipse a
/// user-space line width describes rather than a circle of the average scale
/// (see [`StrokePen`]). Every stroke stays at least one device pixel thick.
/// With `options.stroke_adjust`, axis-aligned strokes are snapped to whole
/// device pixels afterwards.
#[allow(clippy::too_many_arguments)]
pub(crate) fn device_stroke_outline(
    flat: &FlatPath,
    device_t: &Transform2D,
    line_width: f64,
    dash: &DashState,
    cap: &LineCap,
    join: &LineJoin,
    miter_limit: f64,
    options: VectorRasterOptions,
) -> DeviceStrokeOutline {
    let pen = StrokePen::new(device_t, line_width);
    let mut outline = match pen {
        StrokePen::Device {
            width_px,
            dash_scale,
        } => stroke_flat_path(
            flat,
            width_px,
            &dash.scaled(dash_scale),
            cap.clone(),
            join.clone(),
            miter_limit,
        ),
        StrokePen::Transformed {
            width,
            pen_from_device,
            device_from_pen,
            ..
        } => {
            let pen_flat = transform_flat_path(flat, &pen_from_device);
            let mut outline = stroke_flat_path(
                &pen_flat,
                width,
                dash,
                cap.clone(),
                join.clone(),
                miter_limit,
            );
            transform_flat_path_in_place(&mut outline, &device_from_pen);
            outline
        }
    };
    if options.stroke_adjust && !outline.subpaths.is_empty() {
        let (half_x, half_y) = pen.half_extents();
        stroke_adjust_outline(flat, &mut outline, half_x, half_y, cap);
    }
    let paint = stroke_paint_policy(options.thin_line_mode, device_t, line_width);
    DeviceStrokeOutline { outline, paint }
}

/// Thin-line paint policy for a stroke of user-space `line_width` under
/// `device_t`.
pub(crate) fn stroke_paint_policy(
    mode: ThinLineMode,
    device_t: &Transform2D,
    line_width: f64,
) -> StrokePaintPolicy {
    thin_line_paint_policy(
        mode,
        line_width,
        StrokePen::true_device_width(device_t, line_width),
    )
}

fn transform_flat_path(flat: &FlatPath, t: &Transform2D) -> FlatPath {
    let mut mapped = flat.clone();
    transform_flat_path_in_place(&mut mapped, t);
    mapped
}

fn transform_flat_path_in_place(flat: &mut FlatPath, t: &Transform2D) {
    for subpath in &mut flat.subpaths {
        for point in subpath.iter_mut() {
            *point = t.transform_point(point.0, point.1);
        }
    }
}

/// True when the device transform scales both axes equally (rotation and
/// reflection allowed), i.e. a round user-space pen stays round.
fn device_pen_is_conformal(ctm: &Transform2D, viewport: &Viewport) -> bool {
    let t = ctm.concat(&viewport.to_transform());
    let (sx, sy) = (t.scale_x(), t.scale_y());
    let scale = sx.max(sy);
    scale.is_finite()
        && scale > 0.0
        && (sx - sy).abs() <= 1e-9 * scale
        && (t.a * t.c + t.b * t.d).abs() <= 1e-9 * scale * scale
}

/// Binary coverage curve used by `ThinLineMode::Solid`: a pixel is painted
/// fully when at least half covered.
fn binary_coverage_lut() -> &'static [u8; 256] {
    static LUT: std::sync::OnceLock<[u8; 256]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = [0u8; 256];
        for (index, slot) in lut.iter_mut().enumerate() {
            *slot = if index >= 128 { 255 } else { 0 };
        }
        lut
    })
}

fn stroke_impl(
    buf: &mut PixelBuffer,
    request: StrokeRequest<'_>,
    scan: StrokeScan,
    options: VectorRasterOptions,
    cancel: &CancelToken,
) -> bool {
    let StrokeRequest {
        path,
        ctm,
        viewport,
        color,
        stroke_width,
        dash,
        cap,
        join,
        miter_limit,
        bezier_threshold,
    } = request;
    if cancel.is_cancelled() {
        return false;
    }
    if path.is_empty() || buf.width == 0 || buf.height == 0 {
        return true;
    }
    // Exact integer-pixel fast paths. They compute the device width from the
    // average scale, so they only apply to conformal transforms, and they are
    // unaffected by stroke adjustment (their edges already sit on pixel
    // boundaries) but not by thin-line policies.
    if options.thin_line_mode == ThinLineMode::None && device_pen_is_conformal(ctm, viewport) {
        if color[3] == 255 {
            if let Some(regions) = axis_aligned_solid_rect_stroke_regions(
                path,
                ctm,
                viewport,
                stroke_width,
                dash,
                join,
                miter_limit,
            ) {
                for (x, y, w, h) in regions {
                    buf.fill_rect(x, y, w, h, color);
                }
                return true;
            }
        }
        // The hairline fast path centres a one-pixel line on a fractional
        // coordinate; stroke adjustment snaps it to a whole pixel instead.
        if !options.stroke_adjust
            && paint_axis_aligned_solid_hairline(
                buf,
                path,
                ctm,
                viewport,
                color,
                stroke_width,
                dash,
                cap,
            )
        {
            return true;
        }
        if let Some((x, y, w, h)) =
            axis_aligned_solid_stroke_rect(path, ctm, viewport, stroke_width, dash, cap)
        {
            buf.fill_rect(x, y, w, h, color);
            return true;
        }
    }
    let flat = flatten_path(path, ctm, viewport, bezier_threshold);
    if cancel.is_cancelled() {
        return false;
    }
    let device_t = ctm.concat(&viewport.to_transform());
    let stroke = device_stroke_outline(
        &flat,
        &device_t,
        stroke_width,
        dash,
        cap,
        join,
        miter_limit,
        options,
    );
    if stroke.outline.subpaths.is_empty() {
        return true;
    }
    let color = stroke.paint.apply_to_color(color);
    match scan {
        StrokeScan::Binary => {
            fill_flat_scanline_fast(buf, &stroke.outline, color, FillRule::NonZero, cancel)
        }
        StrokeScan::Antialiased => {
            let lut = if stroke.paint.binary {
                Some(binary_coverage_lut())
            } else {
                options.coverage_lut()
            };
            fill_flat_antialiased(buf, &stroke.outline, color, FillRule::NonZero, cancel, lut)
        }
    }
}

/// Anti-aliased fill of a device-space outline through the row-streaming
/// compositor (memory bounded by the clipped path width; per-row work bounded
/// by the columns the outline actually touches).
fn fill_flat_antialiased(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    color: PixelColor,
    rule: FillRule,
    cancel: &CancelToken,
    lut: Option<&[u8; 256]>,
) -> bool {
    fill_flat_with_color_compositor(buf, flat, color, rule, Some(cancel), lut)
}

/// Coverage curve for a stroke mask under `paint` and `options`.
pub(crate) fn stroke_coverage_lut(
    paint: StrokePaintPolicy,
    options: VectorRasterOptions,
) -> Option<&'static [u8; 256]> {
    if paint.binary {
        Some(binary_coverage_lut())
    } else {
        options.coverage_lut()
    }
}

// ---------------------------------------------------------------------------
// Thin-geometry rescue for the sampled scan converter
// ---------------------------------------------------------------------------

/// A subpath whose entire vertical extent lies between two adjacent
/// anti-aliasing sample lines. Point sampling would give it zero coverage, so
/// it would vanish; Splash instead includes every pixel a shape touches. The
/// rescue paints such a subpath into the row containing its vertical centre
/// with its exact horizontal coverage weighted by its height (at least 1/16
/// of a pixel, Splash's single-subsample coverage), so hairline slivers stay
/// visible with an area-proportional density.
#[derive(Clone, Copy, Debug)]
struct ThinSubpathRescue {
    index: usize,
    row: i32,
    center_y: f64,
    weight: f64,
}

const THIN_RESCUE_MIN_WEIGHT: f64 = 1.0 / 16.0;

fn thin_subpath_rescues(flat: &FlatPath, vertical_samples: usize) -> Vec<ThinSubpathRescue> {
    let samples = vertical_samples.max(1) as f64;
    let mut rescues = Vec::new();
    for (index, subpath) in flat.subpaths.iter().enumerate() {
        if subpath.len() < 2 {
            continue;
        }
        let mut y_min = f64::INFINITY;
        let mut y_max = f64::NEG_INFINITY;
        for &(x, y) in subpath {
            if x.is_finite() && y.is_finite() {
                y_min = y_min.min(y);
                y_max = y_max.max(y);
            }
        }
        if y_max <= y_min || !y_min.is_finite() || !y_max.is_finite() {
            continue;
        }
        // The first sample line at or below y_min; the subpath is sampled
        // normally when that line falls inside [y_min, y_max).
        let first_sample = ((y_min * samples - 0.5).ceil() + 0.5) / samples;
        if first_sample < y_max {
            continue;
        }
        let center_y = 0.5 * (y_min + y_max);
        rescues.push(ThinSubpathRescue {
            index,
            row: safe_floor_i32(center_y),
            center_y,
            weight: (y_max - y_min).clamp(THIN_RESCUE_MIN_WEIGHT, 1.0),
        });
    }
    rescues.sort_by_key(|rescue| rescue.row);
    rescues
}

/// Add the rescue coverage of every thin subpath centred in `row` to
/// `row_accum` (a buffer for columns `x0..x0 + w`). `cursor` walks the
/// row-sorted rescue list across sequential rows.
#[allow(clippy::too_many_arguments)]
fn add_thin_subpath_rescues_for_row(
    flat: &FlatPath,
    rescues: &[ThinSubpathRescue],
    cursor: &mut usize,
    row: i32,
    rule: FillRule,
    x0: i32,
    w: usize,
    row_accum: &mut [u16],
    row_base: usize,
    crossings: &mut Vec<(f64, i32)>,
) -> bool {
    while *cursor < rescues.len() && rescues[*cursor].row < row {
        *cursor += 1;
    }
    let mut added = false;
    let mut index = *cursor;
    while index < rescues.len() && rescues[index].row == row {
        let rescue = rescues[index];
        index += 1;
        let (Some(subpath), Some(&closed)) = (
            flat.subpaths.get(rescue.index),
            flat.closed.get(rescue.index),
        ) else {
            continue;
        };
        crossings.clear();
        collect_scanline_crossings(subpath, closed, rescue.center_y, crossings);
        if !prepare_scanline_crossings(crossings) {
            continue;
        }
        accumulate_crossing_spans(
            crossings,
            rule,
            row_accum,
            row_base,
            w,
            x0,
            255.0 * rescue.weight,
        );
        added = true;
    }
    added
}

/// Add the coverage spans of one sorted crossing list to an accumulation row.
fn accumulate_crossing_spans(
    crossings: &[(f64, i32)],
    rule: FillRule,
    accum: &mut [u16],
    row_base: usize,
    w: usize,
    x0: i32,
    sample_weight: f64,
) {
    match rule {
        FillRule::EvenOdd => {
            let mut i = 0usize;
            while i + 1 < crossings.len() {
                add_alpha_span(
                    accum,
                    row_base,
                    w,
                    crossings[i].0 - x0 as f64,
                    crossings[i + 1].0 - x0 as f64,
                    sample_weight,
                );
                i += 2;
            }
        }
        FillRule::NonZero => {
            let mut winding = 0i32;
            let mut start_x: Option<f64> = None;
            for (x, dir) in crossings.iter().copied() {
                if winding != 0 {
                    if let Some(sx) = start_x.take() {
                        add_alpha_span(
                            accum,
                            row_base,
                            w,
                            sx - x0 as f64,
                            x - x0 as f64,
                            sample_weight,
                        );
                    }
                }
                winding += dir;
                if winding != 0 {
                    start_x = Some(x);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Edge-bucket scanline antialiased fill
// ---------------------------------------------------------------------------

/// Fill a flattened path into `buf` through the bounded edge-bucket scanline
/// rasteriser. The scanline path keeps row-local scratch, uses reusable
/// crossing/span buffers, and avoids bounding-box-sized signed-area grids on
/// retained-display-list replay and general page painting.
fn fill_flat_aa(buf: &mut PixelBuffer, flat: &FlatPath, color: PixelColor, rule: FillRule) {
    let _ = fill_flat_color(buf, flat, color, rule, None);
}

#[allow(clippy::collapsible_if)]
fn fill_flat_aa_cancellable(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    color: PixelColor,
    rule: FillRule,
    cancel: &CancelToken,
) -> bool {
    fill_flat_color(buf, flat, color, rule, Some(cancel))
}

fn fill_flat_cmyk_overprint_preview(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    cmyk: [f32; 4],
    alpha: f32,
    overprint_mode: i32,
    rule: FillRule,
) {
    let _ = fill_flat_with_compositor(buf, flat, rule, None, |buf, x, y, coverage| {
        buf.blend_device_cmyk_overprint_preview(x, y, cmyk, alpha, coverage, overprint_mode);
    });
}

fn fill_flat_cmyk_overprint_preview_scanline(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    cmyk: [f32; 4],
    alpha: f32,
    overprint_mode: i32,
    rule: FillRule,
) -> bool {
    let bw = buf.width as i32;
    let bh = buf.height as i32;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for sp in &flat.subpaths {
        for &(_, y) in sp {
            if y.is_finite() {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if !min_y.is_finite() || !max_y.is_finite() {
        return true;
    }
    let y0 = safe_floor_i32(min_y).max(0);
    let y1 = safe_ceil_i32(max_y).saturating_add(1).min(bh);
    if y1 <= y0 {
        return true;
    }

    let mut crossings = take_scanline_crossing_vec();
    let h = (y1 - y0) as usize;
    let edge_buckets = build_sequential_scanline_edge_buckets(flat, y0, h);
    let mut active_edges = ScanlineActiveEdges::default();
    for y in y0..y1 {
        PATH_RASTER_SCANLINE_FAST_ROWS.fetch_add(1, Ordering::Relaxed);
        let scan_y = y as f64 + 0.5;
        crossings.clear();
        if let Some(buckets) = edge_buckets.as_ref() {
            let row = (y - y0) as usize;
            if !active_edges.collect_row(buckets, row, scan_y, &mut crossings) {
                collect_scanline_crossings_from_bucket(buckets, row, scan_y, &mut crossings);
            }
        } else {
            for (sp, &closed) in flat.subpaths.iter().zip(flat.closed.iter()) {
                collect_scanline_crossings(sp, closed, scan_y, &mut crossings);
            }
        }
        if !prepare_scanline_crossings(&mut crossings) {
            continue;
        }
        match rule {
            FillRule::EvenOdd => {
                let mut i = 0usize;
                while i + 1 < crossings.len() {
                    fill_scanline_cmyk_overprint_span(
                        buf,
                        y,
                        crossings[i].0,
                        crossings[i + 1].0,
                        bw,
                        cmyk,
                        alpha,
                        overprint_mode,
                    );
                    i += 2;
                }
            }
            FillRule::NonZero => {
                let mut winding = 0i32;
                let mut start_x: Option<f64> = None;
                for (x, dir) in crossings.iter().copied() {
                    if winding != 0 {
                        if let Some(sx) = start_x.take() {
                            fill_scanline_cmyk_overprint_span(
                                buf,
                                y,
                                sx,
                                x,
                                bw,
                                cmyk,
                                alpha,
                                overprint_mode,
                            );
                        }
                    }
                    winding += dir;
                    if winding != 0 {
                        start_x = Some(x);
                    }
                }
            }
        }
    }
    return_scanline_crossing_vec(crossings);
    true
}

fn fill_flat_color(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    color: PixelColor,
    rule: FillRule,
    cancel: Option<&CancelToken>,
) -> bool {
    fill_flat_with_color_compositor(buf, flat, color, rule, cancel, None)
}

fn fill_flat_scanline_fast(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    color: PixelColor,
    rule: FillRule,
    cancel: &CancelToken,
) -> bool {
    let bw = buf.width as i32;
    let bh = buf.height as i32;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (sp_index, sp) in flat.subpaths.iter().enumerate() {
        if sp_index % 64 == 0 && cancel.is_cancelled() {
            return false;
        }
        for &(_, y) in sp {
            if y.is_finite() {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if !min_y.is_finite() || !max_y.is_finite() {
        return true;
    }
    let y0 = safe_floor_i32(min_y).max(0);
    let y1 = safe_ceil_i32(max_y).saturating_add(1).min(bh);
    if y1 <= y0 {
        return true;
    }

    let mut crossings = take_scanline_crossing_vec();
    let h = (y1 - y0) as usize;
    let edge_buckets = build_sequential_scanline_edge_buckets(flat, y0, h);
    let mut active_edges = ScanlineActiveEdges::default();
    for y in y0..y1 {
        if (y - y0) % 16 == 0 && cancel.is_cancelled() {
            return_scanline_crossing_vec(crossings);
            return false;
        }
        PATH_RASTER_SCANLINE_FAST_ROWS.fetch_add(1, Ordering::Relaxed);
        let scan_y = y as f64 + 0.5;
        crossings.clear();
        if let Some(buckets) = edge_buckets.as_ref() {
            let row = (y - y0) as usize;
            if !active_edges.collect_row(buckets, row, scan_y, &mut crossings) {
                collect_scanline_crossings_from_bucket(buckets, row, scan_y, &mut crossings);
            }
        } else {
            for (sp, &closed) in flat.subpaths.iter().zip(flat.closed.iter()) {
                collect_scanline_crossings(sp, closed, scan_y, &mut crossings);
            }
        }
        if !prepare_scanline_crossings(&mut crossings) {
            continue;
        }
        match rule {
            FillRule::EvenOdd => {
                let mut i = 0usize;
                while i + 1 < crossings.len() {
                    fill_scanline_span(buf, y, crossings[i].0, crossings[i + 1].0, bw, color);
                    i += 2;
                }
            }
            FillRule::NonZero => {
                let mut winding = 0i32;
                let mut start_x: Option<f64> = None;
                for (x, dir) in crossings.iter().copied() {
                    if winding != 0 {
                        if let Some(sx) = start_x.take() {
                            fill_scanline_span(buf, y, sx, x, bw, color);
                        }
                    }
                    winding += dir;
                    if winding != 0 {
                        start_x = Some(x);
                    }
                }
            }
        }
    }
    return_scanline_crossing_vec(crossings);
    true
}

fn take_scanline_crossing_vec() -> Vec<(f64, i32)> {
    SCANLINE_CROSSING_VEC_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        let mut scratch = pool.pop().unwrap_or_default();
        if !scratch.is_empty() {
            scratch.clear();
        }
        if scratch.capacity() > 0 {
            PATH_RASTER_SCANLINE_CROSSING_REUSES.fetch_add(1, Ordering::Relaxed);
        }
        scratch
    })
}

fn return_scanline_crossing_vec(mut scratch: Vec<(f64, i32)>) {
    if scratch.capacity() == 0 || scratch.capacity() > SCANLINE_CROSSING_POOL_MAX_ENTRIES {
        return;
    }
    scratch.clear();
    SCANLINE_CROSSING_VEC_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < SCANLINE_CROSSING_POOL_MAX_BUFFERS {
            pool.push(scratch);
        }
    });
}

fn take_scanline_u16_vec(len: usize) -> Vec<u16> {
    if len == 0 {
        return Vec::new();
    }
    if len <= SCANLINE_U16_POOL_MAX_CELLS {
        if let Some(mut scratch) = SCANLINE_U16_VEC_POOL.with(|pool| {
            let mut pool = pool.borrow_mut();
            let pos = pool
                .iter()
                .position(|candidate| candidate.capacity() >= len)?;
            Some(pool.swap_remove(pos))
        }) {
            scratch.resize(len, 0);
            scratch.fill(0);
            return scratch;
        }
    }
    vec![0u16; len]
}

fn return_scanline_u16_vec(mut scratch: Vec<u16>) {
    if scratch.capacity() == 0 || scratch.capacity() > SCANLINE_U16_POOL_MAX_CELLS {
        return;
    }
    scratch.clear();
    SCANLINE_U16_VEC_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < SCANLINE_U16_POOL_MAX_BUFFERS {
            pool.push(scratch);
        }
    });
}

pub(crate) fn axis_aligned_integer_rect(
    path: &Path,
    ctm: &Transform2D,
    viewport: &Viewport,
) -> Option<(i32, i32, i32, i32)> {
    // Decimal PDF matrices often place an intended integral device edge a few
    // ten-thousandths of a pixel away from the integer. This tolerance stays
    // well below half an 8-bit coverage step, so snapping cannot change the
    // quantized edge alpha while avoiding a page-sized clip mask.
    const DEVICE_PIXEL_SNAP_EPSILON: f64 = 1.0e-4;

    if !ctm.is_axis_aligned() || path.segments.len() != 5 {
        return None;
    }
    let mut points = Vec::with_capacity(4);
    match path.segments.as_slice() {
        [PathSegment::MoveTo(x0, y0), PathSegment::LineTo(x1, y1), PathSegment::LineTo(x2, y2), PathSegment::LineTo(x3, y3), PathSegment::ClosePath] =>
        {
            points.push((*x0, *y0));
            points.push((*x1, *y1));
            points.push((*x2, *y2));
            points.push((*x3, *y3));
        }
        _ => return None,
    }

    let mut px_points = Vec::with_capacity(4);
    for (x, y) in points {
        let (ux, uy) = ctm.transform_point(x, y);
        let (px, py) = viewport.page_to_pixel_f64(ux, uy);
        if !px.is_finite() || !py.is_finite() {
            return None;
        }
        px_points.push((px, py));
    }

    let min_x = px_points
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::INFINITY, f64::min);
    let max_x = px_points
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = px_points
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::INFINITY, f64::min);
    let max_y = px_points
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);
    if max_x <= min_x || max_y <= min_y {
        return None;
    }
    let eps = DEVICE_PIXEL_SNAP_EPSILON;
    if px_points.iter().any(|(x, y)| {
        ((*x - min_x).abs() > eps && (*x - max_x).abs() > eps)
            || ((*y - min_y).abs() > eps && (*y - max_y).abs() > eps)
    }) {
        return None;
    }
    if [min_x, max_x, min_y, max_y]
        .iter()
        .any(|v| (v - v.round()).abs() > eps)
    {
        return None;
    }
    let x0 = min_x.round() as i32;
    let y0 = min_y.round() as i32;
    let x1 = max_x.round() as i32;
    let y1 = max_y.round() as i32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0, y0, x1 - x0, y1 - y0))
}

fn axis_aligned_solid_stroke_rect(
    path: &Path,
    ctm: &Transform2D,
    viewport: &Viewport,
    stroke_width: f64,
    dash: &DashState,
    cap: &LineCap,
) -> Option<(i32, i32, i32, i32)> {
    if !ctm.is_axis_aligned()
        || !dash.is_solid()
        || !matches!(cap, LineCap::Butt | LineCap::ProjectingSquare)
        || !stroke_width.is_finite()
    {
        return None;
    }

    let width_px = (stroke_width * ctm.scale_factor() * viewport.scale).max(1.0);
    if !width_px.is_finite() {
        return None;
    }
    let points = axis_aligned_stroke_device_points(path, ctm, viewport)?;
    let eps = 1e-7;
    let first = *points.first()?;
    let last = *points.last()?;
    let half = width_px / 2.0;
    let (min_x, min_y, max_x, max_y) = if points.iter().all(|(_, y)| (*y - first.1).abs() <= eps) {
        if !axis_values_are_monotonic(points.iter().map(|(x, _)| *x)) {
            return None;
        }
        let min_x = points.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
        let max_x = points
            .iter()
            .map(|(x, _)| *x)
            .fold(f64::NEG_INFINITY, f64::max);
        if max_x - min_x <= eps {
            return None;
        }
        let cap_pad = if matches!(cap, LineCap::ProjectingSquare) {
            half
        } else {
            0.0
        };
        (
            min_x - cap_pad,
            first.1 - half,
            max_x + cap_pad,
            first.1 + half,
        )
    } else if points.iter().all(|(x, _)| (*x - first.0).abs() <= eps) {
        if !axis_values_are_monotonic(points.iter().map(|(_, y)| *y)) {
            return None;
        }
        let min_y = points.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
        let max_y = points
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max);
        if max_y - min_y <= eps {
            return None;
        }
        let cap_pad = if matches!(cap, LineCap::ProjectingSquare) {
            half
        } else {
            0.0
        };
        (
            first.0 - half,
            min_y - cap_pad,
            first.0 + half,
            max_y + cap_pad,
        )
    } else {
        return None;
    };

    if points.len() > 2 {
        let endpoints_span = if (first.1 - last.1).abs() <= eps {
            (first.0.min(last.0), first.0.max(last.0))
        } else {
            (first.1.min(last.1), first.1.max(last.1))
        };
        let all_span = if (first.1 - last.1).abs() <= eps {
            (
                min_x
                    + if matches!(cap, LineCap::ProjectingSquare) {
                        half
                    } else {
                        0.0
                    },
                max_x
                    - if matches!(cap, LineCap::ProjectingSquare) {
                        half
                    } else {
                        0.0
                    },
            )
        } else {
            (
                min_y
                    + if matches!(cap, LineCap::ProjectingSquare) {
                        half
                    } else {
                        0.0
                    },
                max_y
                    - if matches!(cap, LineCap::ProjectingSquare) {
                        half
                    } else {
                        0.0
                    },
            )
        };
        if (endpoints_span.0 - all_span.0).abs() > eps
            || (endpoints_span.1 - all_span.1).abs() > eps
        {
            return None;
        }
    }

    if [min_x, min_y, max_x, max_y]
        .iter()
        .any(|v| !v.is_finite() || (v - v.round()).abs() > eps)
    {
        return None;
    }
    let x0 = min_x.round() as i32;
    let y0 = min_y.round() as i32;
    let x1 = max_x.round() as i32;
    let y1 = max_y.round() as i32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0, y0, x1 - x0, y1 - y0))
}

fn axis_aligned_stroke_device_points(
    path: &Path,
    ctm: &Transform2D,
    viewport: &Viewport,
) -> Option<Vec<(f64, f64)>> {
    let mut points = Vec::new();
    for segment in &path.segments {
        match *segment {
            PathSegment::MoveTo(x, y) if points.is_empty() => {
                points.push(path_point_to_device(x, y, ctm, viewport)?);
            }
            PathSegment::LineTo(x, y) if !points.is_empty() => {
                let point = path_point_to_device(x, y, ctm, viewport)?;
                if points
                    .last()
                    .is_none_or(|last| distance(*last, point) > 1e-8)
                {
                    points.push(point);
                }
            }
            _ => return None,
        }
    }
    (points.len() >= 2).then_some(points)
}

fn path_point_to_device(
    x: f64,
    y: f64,
    ctm: &Transform2D,
    viewport: &Viewport,
) -> Option<(f64, f64)> {
    let (ux, uy) = ctm.transform_point(x, y);
    let point = viewport.page_to_pixel_f64(ux, uy);
    (point.0.is_finite() && point.1.is_finite()).then_some(point)
}

fn axis_values_are_monotonic(values: impl Iterator<Item = f64>) -> bool {
    let mut last = None;
    let mut direction = 0i32;
    for value in values {
        let Some(prev) = last.replace(value) else {
            continue;
        };
        let delta = value - prev;
        if delta.abs() <= 1e-7 {
            continue;
        }
        let step = if delta > 0.0 { 1 } else { -1 };
        if direction == 0 {
            direction = step;
        } else if direction != step {
            return false;
        }
    }
    direction != 0
}

#[allow(clippy::too_many_arguments)]
fn paint_axis_aligned_solid_hairline(
    buf: &mut PixelBuffer,
    path: &Path,
    ctm: &Transform2D,
    viewport: &Viewport,
    color: PixelColor,
    stroke_width: f64,
    dash: &DashState,
    cap: &LineCap,
) -> bool {
    if !ctm.is_axis_aligned()
        || !dash.is_solid()
        || !matches!(cap, LineCap::Butt)
        || !stroke_width.is_finite()
        || stroke_width > 0.0
    {
        return false;
    }

    let Some(points) = axis_aligned_stroke_device_points(path, ctm, viewport) else {
        return false;
    };
    let eps = 1e-7;
    let first = points[0];
    if points.iter().all(|(_, y)| (*y - first.1).abs() <= eps) {
        if !axis_values_are_monotonic(points.iter().map(|(x, _)| *x)) {
            return false;
        }
        let x0 = points.iter().map(|(x, _)| *x).fold(f64::INFINITY, f64::min);
        let x1 = points
            .iter()
            .map(|(x, _)| *x)
            .fold(f64::NEG_INFINITY, f64::max);
        if (x0 - x0.round()).abs() > eps || (x1 - x1.round()).abs() > eps {
            return false;
        }
        paint_horizontal_hairline_run(buf, x0.round() as i32, x1.round() as i32, first.1, color);
        return true;
    }

    if points.iter().all(|(x, _)| (*x - first.0).abs() <= eps) {
        if !axis_values_are_monotonic(points.iter().map(|(_, y)| *y)) {
            return false;
        }
        let y0 = points.iter().map(|(_, y)| *y).fold(f64::INFINITY, f64::min);
        let y1 = points
            .iter()
            .map(|(_, y)| *y)
            .fold(f64::NEG_INFINITY, f64::max);
        if (y0 - y0.round()).abs() > eps || (y1 - y1.round()).abs() > eps {
            return false;
        }
        paint_vertical_hairline_run(buf, first.0, y0.round() as i32, y1.round() as i32, color);
        return true;
    }

    false
}

fn paint_horizontal_hairline_run(
    buf: &mut PixelBuffer,
    x0: i32,
    x1: i32,
    center_y: f64,
    color: PixelColor,
) {
    if x1 <= x0 {
        return;
    }
    let y_min = center_y - 0.5;
    let y_max = center_y + 0.5;
    let row_start = safe_floor_i32(y_min);
    let row_end = safe_ceil_i32(y_max);
    for y in row_start..row_end {
        let samples = vertical_samples_in_range(y, y_min, y_max);
        if samples == 0 {
            continue;
        }
        let alpha = sampled_span_alpha(1.0, samples);
        paint_coverage_span(buf, x0, y, x1 - x0, 1, color, alpha);
    }
}

fn paint_vertical_hairline_run(
    buf: &mut PixelBuffer,
    center_x: f64,
    y0: i32,
    y1: i32,
    color: PixelColor,
) {
    if y1 <= y0 {
        return;
    }
    let x_min = center_x - 0.5;
    let x_max = center_x + 0.5;
    let col_start = safe_floor_i32(x_min);
    let col_end = safe_ceil_i32(x_max);
    for x in col_start..col_end {
        let coverage = (x_max.min(x as f64 + 1.0) - x_min.max(x as f64)).clamp(0.0, 1.0);
        if coverage <= 0.0 {
            continue;
        }
        let alpha = sampled_span_alpha(coverage, PATH_SCANLINE_COLOR_VERTICAL_SAMPLES);
        paint_coverage_span(buf, x, y0, 1, y1 - y0, color, alpha);
    }
}

fn vertical_samples_in_range(row: i32, y_min: f64, y_max: f64) -> usize {
    let mut samples = 0usize;
    for sample in 0..PATH_SCANLINE_COLOR_VERTICAL_SAMPLES {
        let sample_y =
            row as f64 + (sample as f64 + 0.5) / PATH_SCANLINE_COLOR_VERTICAL_SAMPLES as f64;
        if sample_y >= y_min && sample_y < y_max {
            samples += 1;
        }
    }
    samples
}

fn sampled_span_alpha(horizontal_coverage: f64, vertical_samples: usize) -> u8 {
    if horizontal_coverage <= 0.0 || vertical_samples == 0 {
        return 0;
    }
    let sample_weight = 255.0 / PATH_SCANLINE_COLOR_VERTICAL_SAMPLES as f64;
    let per_sample = (horizontal_coverage.clamp(0.0, 1.0) * sample_weight)
        .round()
        .clamp(0.0, 255.0) as u16;
    per_sample.saturating_mul(vertical_samples as u16).min(255) as u8
}

fn paint_coverage_span(
    buf: &mut PixelBuffer,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: PixelColor,
    alpha: u8,
) {
    if alpha == 0 {
        return;
    }
    if alpha == 255 {
        buf.fill_rect(x, y, w, h, color);
        return;
    }
    let coverage = f32::from(alpha) / 255.0;
    for row in y..y.saturating_add(h) {
        for col in x..x.saturating_add(w) {
            buf.blend_pixel(col, row, color, coverage);
        }
    }
}

fn axis_aligned_solid_rect_stroke_regions(
    path: &Path,
    ctm: &Transform2D,
    viewport: &Viewport,
    stroke_width: f64,
    dash: &DashState,
    join: &LineJoin,
    miter_limit: f64,
) -> Option<[(i32, i32, i32, i32); 4]> {
    if !ctm.is_axis_aligned()
        || !dash.is_solid()
        || !matches!(join, LineJoin::Miter)
        || !stroke_width.is_finite()
        || miter_limit + 1e-7 < std::f64::consts::SQRT_2
    {
        return None;
    }

    let corners = match path.segments.as_slice() {
        [PathSegment::MoveTo(x0, y0), PathSegment::LineTo(x1, y1), PathSegment::LineTo(x2, y2), PathSegment::LineTo(x3, y3), PathSegment::ClosePath] => {
            [(*x0, *y0), (*x1, *y1), (*x2, *y2), (*x3, *y3)]
        }
        _ => return None,
    };

    let mut px_points = [(0.0, 0.0); 4];
    for (idx, (x, y)) in corners.into_iter().enumerate() {
        let (ux, uy) = ctm.transform_point(x, y);
        let point = viewport.page_to_pixel_f64(ux, uy);
        if !point.0.is_finite() || !point.1.is_finite() {
            return None;
        }
        px_points[idx] = point;
    }

    let min_x = px_points
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::INFINITY, f64::min);
    let max_x = px_points
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = px_points
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::INFINITY, f64::min);
    let max_y = px_points
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);
    if max_x <= min_x || max_y <= min_y {
        return None;
    }

    let eps = 1e-7;
    if px_points.iter().any(|(x, y)| {
        ((*x - min_x).abs() > eps && (*x - max_x).abs() > eps)
            || ((*y - min_y).abs() > eps && (*y - max_y).abs() > eps)
    }) {
        return None;
    }

    let width_px = (stroke_width * ctm.scale_factor() * viewport.scale).max(1.0);
    if !width_px.is_finite() {
        return None;
    }
    let half = width_px / 2.0;
    let boundaries = [
        min_x - half,
        min_x + half,
        max_x - half,
        max_x + half,
        min_y - half,
        min_y + half,
        max_y - half,
        max_y + half,
    ];
    if boundaries
        .iter()
        .any(|v| !v.is_finite() || (v - v.round()).abs() > eps)
    {
        return None;
    }

    let outer_x0 = (min_x - half).round() as i32;
    let inner_x0 = (min_x + half).round() as i32;
    let inner_x1 = (max_x - half).round() as i32;
    let outer_x1 = (max_x + half).round() as i32;
    let outer_y0 = (min_y - half).round() as i32;
    let inner_y0 = (min_y + half).round() as i32;
    let inner_y1 = (max_y - half).round() as i32;
    let outer_y1 = (max_y + half).round() as i32;

    if outer_x1 <= outer_x0
        || outer_y1 <= outer_y0
        || inner_x1 <= inner_x0
        || inner_y1 <= inner_y0
        || inner_x0 <= outer_x0
        || outer_x1 <= inner_x1
        || inner_y0 <= outer_y0
        || outer_y1 <= inner_y1
    {
        return None;
    }

    Some([
        (outer_x0, outer_y0, outer_x1 - outer_x0, inner_y0 - outer_y0),
        (outer_x0, inner_y1, outer_x1 - outer_x0, outer_y1 - inner_y1),
        (outer_x0, inner_y0, inner_x0 - outer_x0, inner_y1 - inner_y0),
        (inner_x1, inner_y0, outer_x1 - inner_x1, inner_y1 - inner_y0),
    ])
}

fn collect_scanline_crossings(
    sp: &[(f64, f64)],
    closed: bool,
    scan_y: f64,
    crossings: &mut Vec<(f64, i32)>,
) {
    if sp.len() < 2 {
        return;
    }
    for win in sp.windows(2) {
        push_scanline_crossing(win[0], win[1], scan_y, crossings);
    }
    if !closed {
        if let (Some(&first), Some(&last)) = (sp.first(), sp.last()) {
            if first != last {
                push_scanline_crossing(last, first, scan_y, crossings);
            }
        }
    }
}

fn push_scanline_crossing(
    p0: (f64, f64),
    p1: (f64, f64),
    scan_y: f64,
    crossings: &mut Vec<(f64, i32)>,
) {
    let (x0, y0) = p0;
    let (x1, y1) = p1;
    if !x0.is_finite() || !y0.is_finite() || !x1.is_finite() || !y1.is_finite() {
        return;
    }
    if (y0 - y1).abs() < 1e-12 {
        return;
    }
    let ymin = y0.min(y1);
    let ymax = y0.max(y1);
    if scan_y < ymin || scan_y >= ymax {
        return;
    }
    let t = (scan_y - y0) / (y1 - y0);
    let x = x0 + t * (x1 - x0);
    let dir = if y0 < y1 { 1 } else { -1 };
    crossings.push((x, dir));
}

#[derive(Clone, Copy)]
struct ScanlineEdge {
    p0: (f64, f64),
    ymin: f64,
    ymax: f64,
    dir: i32,
    dx_dy: f64,
}

impl ScanlineEdge {
    fn new(p0: (f64, f64), p1: (f64, f64)) -> Option<Self> {
        let (x0, y0) = p0;
        let (x1, y1) = p1;
        if !x0.is_finite() || !y0.is_finite() || !x1.is_finite() || !y1.is_finite() {
            return None;
        }
        if (y0 - y1).abs() < 1e-12 {
            return None;
        }
        Some(Self {
            p0,
            ymin: y0.min(y1),
            ymax: y0.max(y1),
            dir: if y0 < y1 { 1 } else { -1 },
            dx_dy: (x1 - x0) / (y1 - y0),
        })
    }

    fn crossing_at(self, scan_y: f64) -> Option<(f64, i32)> {
        if scan_y < self.ymin || scan_y >= self.ymax {
            return None;
        }
        Some((self.p0.0 + (scan_y - self.p0.1) * self.dx_dy, self.dir))
    }
}

struct ScanlineEdgeBuckets {
    edges: Vec<ScanlineEdge>,
    /// Exclusive end row of every edge (parallel to `edges`).
    end_rows: Vec<usize>,
    /// Per-row edge links for random row access. Only built while the total
    /// stays under [`SCANLINE_EDGE_BUCKET_MAX_LINKS`]; sequential scans use the
    /// start/end events, which are always present and O(edges + rows).
    has_row_links: bool,
    row_offsets: Vec<usize>,
    links: Vec<usize>,
    start_offsets: Vec<usize>,
    start_links: Vec<usize>,
    end_offsets: Vec<usize>,
    end_links: Vec<usize>,
}

#[cfg(test)]
fn build_scanline_edge_buckets(flat: &FlatPath, y0: i32, h: usize) -> Option<ScanlineEdgeBuckets> {
    build_scanline_edge_buckets_with(flat, y0, h, true)
}

/// Edge buckets for a scan that visits the rows in order, which only needs
/// the start/end events ([`ScanlineActiveEdges`]): the per-row random-access
/// links are not built. A non-sequential access would still be answered,
/// exactly, by testing every edge.
fn build_sequential_scanline_edge_buckets(
    flat: &FlatPath,
    y0: i32,
    h: usize,
) -> Option<ScanlineEdgeBuckets> {
    build_scanline_edge_buckets_with(flat, y0, h, false)
}

fn build_scanline_edge_buckets_with(
    flat: &FlatPath,
    y0: i32,
    h: usize,
    row_links: bool,
) -> Option<ScanlineEdgeBuckets> {
    if h == 0 {
        return None;
    }
    let mut edges = Vec::new();
    let mut spans = Vec::<(usize, usize)>::new();
    let mut links = 0usize;
    for (sp, &closed) in flat.subpaths.iter().zip(flat.closed.iter()) {
        if sp.len() < 2 {
            continue;
        }
        for win in sp.windows(2) {
            push_scanline_edge_bucket(win[0], win[1], y0, h, &mut edges, &mut spans, &mut links);
        }
        if !closed {
            if let (Some(&first), Some(&last)) = (sp.first(), sp.last()) {
                if first != last {
                    push_scanline_edge_bucket(
                        last, first, y0, h, &mut edges, &mut spans, &mut links,
                    );
                }
            }
        }
    }
    if edges.is_empty() {
        return None;
    }
    // Large or tall paths (maps, CAD drawings) would need an unbounded number
    // of per-row links. Sequential scans only need the start/end events, so
    // skip the random-access links instead of abandoning the edge buckets
    // (which used to force an O(rows x edges) brute-force scan).
    let links_fit = links <= SCANLINE_EDGE_BUCKET_MAX_LINKS;
    let has_row_links = row_links && links_fit;
    let mut row_counts = vec![0usize; if has_row_links { h } else { 0 }];
    if has_row_links {
        for &(row_start, row_end) in &spans {
            for row in &mut row_counts[row_start..row_end] {
                *row = row.saturating_add(1);
            }
        }
    }
    PATH_RASTER_EDGE_BUCKET_BUILDS.fetch_add(1, Ordering::Relaxed);
    PATH_RASTER_EDGE_BUCKET_LINKS.fetch_add(links as u64, Ordering::Relaxed);
    if has_row_links {
        PATH_RASTER_EDGE_BUCKET_ROWS.fetch_add(
            row_counts.iter().filter(|count| **count > 0).count() as u64,
            Ordering::Relaxed,
        );
    }

    let (row_offsets, flat_links) = if has_row_links {
        let mut row_offsets = vec![0usize; h + 1];
        for (idx, count) in row_counts.iter().copied().enumerate() {
            row_offsets[idx + 1] = row_offsets[idx].saturating_add(count);
        }
        let mut flat_links = vec![0usize; links];
        let mut cursor = row_offsets[..h].to_vec();
        for (edge_index, (row_start, row_end)) in spans.iter().copied().enumerate() {
            for row in row_start..row_end {
                let Some(slot) = cursor.get_mut(row) else {
                    continue;
                };
                if let Some(dst) = flat_links.get_mut(*slot) {
                    *dst = edge_index;
                }
                *slot = slot.saturating_add(1);
            }
        }
        (row_offsets, flat_links)
    } else {
        (Vec::new(), Vec::new())
    };
    let end_rows = spans.iter().map(|(_, row_end)| *row_end).collect();
    let (start_offsets, start_links) = build_scanline_edge_events(
        h,
        spans
            .iter()
            .enumerate()
            .map(|(edge_index, (row_start, _))| (edge_index, *row_start)),
    );
    let (end_offsets, end_links) = build_scanline_edge_events(
        h,
        spans
            .iter()
            .enumerate()
            .filter_map(|(edge_index, (_, row_end))| {
                (*row_end < h).then_some((edge_index, *row_end))
            }),
    );
    if !row_links && links_fit {
        // The rows with at least one edge, as the row links would count them:
        // edges started minus edges ended by each row.
        let rows = (1..=h)
            .filter(|&end| start_offsets[end] > end_offsets[end])
            .count();
        PATH_RASTER_EDGE_BUCKET_ROWS.fetch_add(rows as u64, Ordering::Relaxed);
    }

    Some(ScanlineEdgeBuckets {
        edges,
        end_rows,
        has_row_links,
        row_offsets,
        links: flat_links,
        start_offsets,
        start_links,
        end_offsets,
        end_links,
    })
}

fn build_scanline_edge_events(
    h: usize,
    events: impl Iterator<Item = (usize, usize)> + Clone,
) -> (Vec<usize>, Vec<usize>) {
    let mut row_counts = vec![0usize; h];
    for (_, row) in events.clone() {
        if let Some(count) = row_counts.get_mut(row) {
            *count = count.saturating_add(1);
        }
    }
    let mut offsets = vec![0usize; h + 1];
    for (idx, count) in row_counts.iter().copied().enumerate() {
        offsets[idx + 1] = offsets[idx].saturating_add(count);
    }
    let mut links = vec![0usize; offsets[h]];
    let mut cursor = offsets[..h].to_vec();
    for (edge_index, row) in events {
        let Some(slot) = cursor.get_mut(row) else {
            continue;
        };
        if let Some(dst) = links.get_mut(*slot) {
            *dst = edge_index;
        }
        *slot = slot.saturating_add(1);
    }
    (offsets, links)
}

fn push_scanline_edge_bucket(
    p0: (f64, f64),
    p1: (f64, f64),
    y0: i32,
    h: usize,
    edges: &mut Vec<ScanlineEdge>,
    spans: &mut Vec<(usize, usize)>,
    links: &mut usize,
) {
    let Some(edge) = ScanlineEdge::new(p0, p1) else {
        return;
    };
    let row_start = safe_floor_i32(edge.ymin - y0 as f64).max(0) as usize;
    let row_end = safe_ceil_i32(edge.ymax - y0 as f64).max(0).min(h as i32) as usize;
    if row_start >= row_end || row_start >= h {
        return;
    }
    let end = row_end.min(h);
    let span_len = end.saturating_sub(row_start);
    edges.push(edge);
    spans.push((row_start, end));
    *links = links.saturating_add(span_len);
}

fn collect_scanline_crossings_from_bucket(
    bucket: &ScanlineEdgeBuckets,
    row: usize,
    scan_y: f64,
    crossings: &mut Vec<(f64, i32)>,
) {
    if !bucket.has_row_links {
        // Random-access fallback for buckets built without per-row links.
        for (edge, &end_row) in bucket.edges.iter().zip(&bucket.end_rows) {
            if end_row > row {
                if let Some(crossing) = edge.crossing_at(scan_y) {
                    crossings.push(crossing);
                }
            }
        }
        return;
    }
    let Some(start) = bucket.row_offsets.get(row).copied() else {
        return;
    };
    let Some(end) = bucket.row_offsets.get(row + 1).copied() else {
        return;
    };
    let Some(edge_indices) = bucket.links.get(start..end) else {
        return;
    };
    for edge_index in edge_indices {
        if let Some(edge) = bucket.edges.get(*edge_index) {
            if let Some(crossing) = edge.crossing_at(scan_y) {
                crossings.push(crossing);
            }
        }
    }
}

#[derive(Default)]
struct ScanlineActiveEdges {
    next_row: usize,
    active: Vec<usize>,
}

impl ScanlineActiveEdges {
    fn advance_to_row(&mut self, buckets: &ScanlineEdgeBuckets, row: usize) -> bool {
        if row != self.next_row || row + 1 >= buckets.start_offsets.len() {
            return false;
        }
        self.remove_ended_edges(buckets, row);
        self.add_starting_edges(buckets, row);
        self.next_row = self.next_row.saturating_add(1);
        true
    }

    fn collect_current_crossings(
        &self,
        buckets: &ScanlineEdgeBuckets,
        scan_y: f64,
        crossings: &mut Vec<(f64, i32)>,
    ) {
        for edge_index in &self.active {
            if let Some(edge) = buckets.edges.get(*edge_index) {
                if let Some(crossing) = edge.crossing_at(scan_y) {
                    crossings.push(crossing);
                }
            }
        }
    }

    fn collect_row(
        &mut self,
        buckets: &ScanlineEdgeBuckets,
        row: usize,
        scan_y: f64,
        crossings: &mut Vec<(f64, i32)>,
    ) -> bool {
        if !self.advance_to_row(buckets, row) {
            return false;
        }
        self.collect_current_crossings(buckets, scan_y, crossings);
        true
    }

    fn remove_ended_edges(&mut self, buckets: &ScanlineEdgeBuckets, row: usize) {
        let Some(ended) = scanline_row_events(&buckets.end_offsets, &buckets.end_links, row) else {
            return;
        };
        if ended.is_empty() {
            return;
        }
        // O(active) per row instead of O(active x ended).
        self.active.retain(|edge_index| {
            buckets
                .end_rows
                .get(*edge_index)
                .is_some_and(|end| *end > row)
        });
    }

    fn add_starting_edges(&mut self, buckets: &ScanlineEdgeBuckets, row: usize) {
        let Some(starting) = scanline_row_events(&buckets.start_offsets, &buckets.start_links, row)
        else {
            return;
        };
        self.active.extend_from_slice(starting);
    }
}

fn scanline_row_events<'a>(
    offsets: &[usize],
    links: &'a [usize],
    row: usize,
) -> Option<&'a [usize]> {
    let start = offsets.get(row).copied()?;
    let end = offsets.get(row + 1).copied()?;
    links.get(start..end)
}

fn prepare_scanline_crossings(crossings: &mut Vec<(f64, i32)>) -> bool {
    crossings.retain(|(x, dir)| x.is_finite() && *dir != 0);
    if crossings.len() < 2 {
        return false;
    }
    sort_scanline_crossings(crossings);
    true
}

/// Sort crossings by x, then direction. Elements that compare equal under
/// this total order are bit-identical, so the unstable sort gives exactly
/// the stable sort's result (and is faster on the short lists of a row).
fn sort_scanline_crossings(crossings: &mut [(f64, i32)]) {
    crossings.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
}

fn fill_scanline_span(buf: &mut PixelBuffer, y: i32, x0: f64, x1: f64, bw: i32, color: PixelColor) {
    if x1 <= x0 {
        return;
    }
    let start = safe_ceil_i32(x0).max(0);
    let end = safe_floor_i32(x1).min(bw - 1);
    if end < start {
        return;
    }
    buf.fill_rect(start, y, end - start + 1, 1, color);
}

#[allow(clippy::too_many_arguments)]
fn fill_scanline_cmyk_overprint_span(
    buf: &mut PixelBuffer,
    y: i32,
    x0: f64,
    x1: f64,
    bw: i32,
    cmyk: [f32; 4],
    alpha: f32,
    overprint_mode: i32,
) {
    if x1 <= x0 {
        return;
    }
    let start = safe_ceil_i32(x0).max(0);
    let end = safe_floor_i32(x1).min(bw - 1);
    if end < start {
        return;
    }
    for x in start..=end {
        buf.blend_device_cmyk_overprint_preview(x, y, cmyk, alpha, 1.0, overprint_mode);
    }
}

pub(crate) fn rasterize_flat_binary_clip_mask(
    flat: &FlatPath,
    width: u32,
    height: u32,
    rule: FillRule,
    cancel: Option<&CancelToken>,
) -> Option<ClipMask> {
    if width == 0 || height == 0 {
        return Some(ClipMask::empty(width, height));
    }
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (sp_index, sp) in flat.subpaths.iter().enumerate() {
        if sp_index % 64 == 0 && cancel.is_some_and(CancelToken::is_cancelled) {
            return None;
        }
        for &(_, y) in sp {
            if y.is_finite() {
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if !min_y.is_finite() || !max_y.is_finite() {
        return Some(ClipMask::empty(width, height));
    }
    let y0 = safe_floor_i32(min_y).max(0);
    let y1 = safe_ceil_i32(max_y).saturating_add(1).min(height as i32);
    if y1 <= y0 {
        return Some(ClipMask::empty(width, height));
    }

    let mut rows = vec![Vec::<(i32, i32)>::new(); height as usize];
    let mut crossings = take_scanline_crossing_vec();
    let h = (y1 - y0) as usize;
    let edge_buckets = build_sequential_scanline_edge_buckets(flat, y0, h);
    let mut active_edges = ScanlineActiveEdges::default();
    for y in y0..y1 {
        if (y - y0) % 16 == 0 && cancel.is_some_and(CancelToken::is_cancelled) {
            return_scanline_crossing_vec(crossings);
            return None;
        }
        let scan_y = y as f64 + 0.5;
        crossings.clear();
        if let Some(buckets) = edge_buckets.as_ref() {
            let row = (y - y0) as usize;
            if !active_edges.collect_row(buckets, row, scan_y, &mut crossings) {
                collect_scanline_crossings_from_bucket(buckets, row, scan_y, &mut crossings);
            }
        } else {
            for (sp, &closed) in flat.subpaths.iter().zip(flat.closed.iter()) {
                collect_scanline_crossings(sp, closed, scan_y, &mut crossings);
            }
        }
        if !prepare_scanline_crossings(&mut crossings) {
            continue;
        }
        match rule {
            FillRule::EvenOdd => {
                let mut i = 0usize;
                while i + 1 < crossings.len() {
                    push_binary_clip_span(
                        &mut rows,
                        y,
                        crossings[i].0,
                        crossings[i + 1].0,
                        width as i32,
                    );
                    i += 2;
                }
            }
            FillRule::NonZero => {
                let mut winding = 0i32;
                let mut start_x: Option<f64> = None;
                for (x, dir) in crossings.iter().copied() {
                    if winding != 0 {
                        if let Some(sx) = start_x.take() {
                            push_binary_clip_span(&mut rows, y, sx, x, width as i32);
                        }
                    }
                    winding += dir;
                    if winding != 0 {
                        start_x = Some(x);
                    }
                }
            }
        }
    }
    return_scanline_crossing_vec(crossings);
    Some(ClipMask::from_visible_runs(width, height, rows))
}

fn push_binary_clip_span(rows: &mut [Vec<(i32, i32)>], y: i32, x0: f64, x1: f64, width: i32) {
    if x1 <= x0 || y < 0 || width <= 0 {
        return;
    }
    let Some(row) = rows.get_mut(y as usize) else {
        return;
    };
    let start = safe_ceil_i32(x0).max(0).min(width);
    let end_inclusive = safe_floor_i32(x1).max(0).min(width.saturating_sub(1));
    if end_inclusive < start {
        return;
    }
    row.push((start, end_inclusive.saturating_add(1).min(width)));
}

fn clamp_fill_bounds_to_clip(
    buf: &PixelBuffer,
    x0: &mut i32,
    y0: &mut i32,
    x1: &mut i32,
    y1: &mut i32,
) -> bool {
    let Some(clip) = buf.clip_mask() else {
        return true;
    };
    let Some((cx0, cy0, cx1, cy1)) = clip.visible_bounds() else {
        return false;
    };
    *x0 = (*x0).max(cx0);
    *y0 = (*y0).max(cy0);
    *x1 = (*x1).min(cx1);
    *y1 = (*y1).min(cy1);
    *x1 > *x0 && *y1 > *y0
}

fn fill_flat_with_compositor(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    rule: FillRule,
    cancel: Option<&CancelToken>,
    mut composite_pixel: impl FnMut(&mut PixelBuffer, i32, i32, f32),
) -> bool {
    let bw = buf.width as i32;
    let bh = buf.height as i32;

    // Device-space bounding box of all subpaths, clamped to the buffer.
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (sp_index, sp) in flat.subpaths.iter().enumerate() {
        if sp_index % 64 == 0 && cancel.is_some_and(CancelToken::is_cancelled) {
            return false;
        }
        for &(x, y) in sp {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if !min_x.is_finite() || !max_x.is_finite() {
        return true;
    }

    let mut x0 = safe_floor_i32(min_x).max(0);
    let mut y0 = safe_floor_i32(min_y).max(0);
    let mut x1 = safe_ceil_i32(max_x).saturating_add(1).min(bw);
    let mut y1 = safe_ceil_i32(max_y).saturating_add(1).min(bh);
    if !clamp_fill_bounds_to_clip(buf, &mut x0, &mut y0, &mut x1, &mut y1) {
        return true;
    }
    if x1 <= x0 || y1 <= y0 {
        return true;
    }
    let w = (x1 - x0) as usize;
    let h = (y1 - y0) as usize;
    // Guard against pathological allocation on degenerate huge geometry by
    // rasterising bounded y-bands instead of silently skipping the path.
    if w == 0 || h == 0 {
        return true;
    }
    // General page paths use the same edge-bucket scan conversion route by
    // default, avoiding bounding-box-sized f32 grids on ordinary vector
    // content and retained display-list replay.
    let rescues = thin_subpath_rescues(flat, PATH_SCANLINE_COLOR_VERTICAL_SAMPLES);
    fill_flat_with_compositor_scanline(
        buf,
        flat,
        rule,
        cancel,
        (x0, y0, x1, y1),
        PATH_SCANLINE_COLOR_VERTICAL_SAMPLES,
        &rescues,
        &mut composite_pixel,
    )
}

fn fill_flat_with_color_compositor(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    color: PixelColor,
    rule: FillRule,
    cancel: Option<&CancelToken>,
    lut: Option<&[u8; 256]>,
) -> bool {
    let bw = buf.width as i32;
    let bh = buf.height as i32;

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (sp_index, sp) in flat.subpaths.iter().enumerate() {
        if sp_index % 64 == 0 && cancel.is_some_and(CancelToken::is_cancelled) {
            return false;
        }
        for &(x, y) in sp {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if !min_x.is_finite() || !max_x.is_finite() {
        return true;
    }

    let mut x0 = safe_floor_i32(min_x).max(0);
    let mut y0 = safe_floor_i32(min_y).max(0);
    let mut x1 = safe_ceil_i32(max_x).saturating_add(1).min(bw);
    let mut y1 = safe_ceil_i32(max_y).saturating_add(1).min(bh);
    if !clamp_fill_bounds_to_clip(buf, &mut x0, &mut y0, &mut x1, &mut y1) {
        return true;
    }
    if x1 <= x0 || y1 <= y0 {
        return true;
    }
    let w = (x1 - x0) as usize;
    let h = (y1 - y0) as usize;
    if w == 0 || h == 0 {
        return true;
    }
    let rescues = thin_subpath_rescues(flat, PATH_SCANLINE_COLOR_VERTICAL_SAMPLES);
    if rescues.is_empty() {
        if let Some(handled) = fill_convex_flat_with_color_compositor_scanline(
            buf,
            flat,
            color,
            rule,
            cancel,
            (x0, y0, x1, y1),
            PATH_SCANLINE_COLOR_VERTICAL_SAMPLES,
            lut,
        ) {
            return handled;
        }
    }
    // General page paths now use the edge-bucket scanline route by default.
    // This avoids falling back to accumulator grids on the pathological vector
    // pages that motivated this closure pass while preserving cancellation and
    // bounded row-local scratch behavior.
    fill_flat_with_color_compositor_scanline(
        buf,
        flat,
        color,
        rule,
        cancel,
        (x0, y0, x1, y1),
        PATH_SCANLINE_COLOR_VERTICAL_SAMPLES,
        &rescues,
        lut,
    )
}

#[allow(clippy::too_many_arguments)]
fn fill_convex_flat_with_color_compositor_scanline(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    color: PixelColor,
    _rule: FillRule,
    cancel: Option<&CancelToken>,
    bounds: (i32, i32, i32, i32),
    vertical_samples: usize,
    lut: Option<&[u8; 256]>,
) -> Option<bool> {
    let edges = convex_flat_edges(flat)?;
    let (x0, y0, x1, y1) = bounds;
    if x1 <= x0 || y1 <= y0 || vertical_samples == 0 {
        return Some(true);
    }
    let w = (x1 - x0) as usize;
    let mut row_accum = take_scanline_u16_vec(w);
    let mut memo = take_coverage_blend_memo();
    let sample_weight = 255.0 / vertical_samples as f64;
    for y in y0..y1 {
        if (y - y0) % 16 == 0 && cancel.is_some_and(CancelToken::is_cancelled) {
            return_scanline_u16_vec(row_accum);
            return Some(false);
        }
        row_accum.fill(0);
        for sample in 0..vertical_samples {
            let scan_y = y as f64 + (sample as f64 + 0.5) / vertical_samples as f64;
            if let Some((left, right)) = convex_scanline_span(&edges, scan_y) {
                PATH_RASTER_SCANLINE_FAST_ROWS.fetch_add(1, Ordering::Relaxed);
                add_alpha_span(
                    &mut row_accum,
                    0,
                    w,
                    left - x0 as f64,
                    right - x0 as f64,
                    sample_weight,
                );
            }
        }
        paint_alpha_row_runs(buf, &row_accum, x0, y, color, lut, &mut memo, |pixels| {
            PATH_RASTER_CONVEX_FAST_PIXELS.fetch_add(pixels, Ordering::Relaxed);
        });
    }
    return_scanline_u16_vec(row_accum);
    return_coverage_blend_memo(memo);
    Some(true)
}

fn convex_flat_points(flat: &FlatPath) -> Option<Vec<(f64, f64)>> {
    if flat.subpaths.len() != 1 {
        return None;
    }
    let sp = flat.subpaths.first()?;
    let mut points = Vec::with_capacity(sp.len());
    for &point in sp {
        if !point.0.is_finite() || !point.1.is_finite() {
            return None;
        }
        if points
            .last()
            .is_none_or(|last| distance(*last, point) > 1e-8)
        {
            points.push(point);
        }
    }
    if points.len() > 1 && distance(points[0], *points.last()?) <= 1e-8 {
        points.pop();
    }
    if points.len() < 3 || !is_convex_polygon(&points) {
        return None;
    }
    Some(points)
}

fn convex_flat_edges(flat: &FlatPath) -> Option<Vec<ScanlineEdge>> {
    let points = convex_flat_points(flat)?;
    let mut edges = Vec::with_capacity(points.len());
    for index in 0..points.len() {
        if let Some(edge) = ScanlineEdge::new(points[index], points[(index + 1) % points.len()]) {
            edges.push(edge);
        }
    }
    (edges.len() >= 2).then_some(edges)
}

fn is_convex_polygon(points: &[(f64, f64)]) -> bool {
    if points.len() < 3 {
        return false;
    }
    let mut area = 0.0;
    let mut orientation = 0i32;
    for index in 0..points.len() {
        let a = points[index];
        let b = points[(index + 1) % points.len()];
        let c = points[(index + 2) % points.len()];
        area += a.0 * b.1 - a.1 * b.0;
        let turn = cross(sub(b, a), sub(c, b));
        if turn.abs() <= 1e-8 {
            continue;
        }
        let sign = if turn > 0.0 { 1 } else { -1 };
        if orientation == 0 {
            orientation = sign;
        } else if orientation != sign {
            return false;
        }
    }
    area.abs() > 1e-8 && orientation != 0
}

fn convex_scanline_span(edges: &[ScanlineEdge], scan_y: f64) -> Option<(f64, f64)> {
    let mut left = f64::INFINITY;
    let mut right = f64::NEG_INFINITY;
    let mut crossings = 0usize;
    for edge in edges {
        if let Some((x, _dir)) = edge.crossing_at(scan_y) {
            if !x.is_finite() {
                return None;
            }
            left = left.min(x);
            right = right.max(x);
            crossings += 1;
        }
    }
    (crossings >= 2 && right > left).then_some((left, right))
}

/// Pixel box `(x0, y0, width, height)` of the alpha mask
/// [`rasterize_flat_alpha_mask`] builds for `flat`, or `None` when it would
/// build none (no finite points, empty or over-budget box).
pub(crate) fn flat_alpha_mask_bounds(flat: &FlatPath) -> Option<(i32, i32, usize, usize)> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for sp in &flat.subpaths {
        for &(x, y) in sp {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if !min_x.is_finite() || !max_x.is_finite() {
        return None;
    }

    let x0 = safe_floor_i32(min_x);
    let y0 = safe_floor_i32(min_y);
    let x1 = safe_ceil_i32(max_x).saturating_add(1);
    let y1 = safe_ceil_i32(max_y).saturating_add(1);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let w = (x1 - x0) as usize;
    let h = (y1 - y0) as usize;
    let cell_count = w.saturating_mul(h);
    if w == 0 || h == 0 || cell_count > ACCUMULATOR_MAX_CELLS {
        return None;
    }
    Some((x0, y0, w, h))
}

pub(crate) fn rasterize_flat_alpha_mask(
    flat: &FlatPath,
    rule: FillRule,
) -> Option<RasterizedGlyphMask> {
    let (x0, y0, w, h) = flat_alpha_mask_bounds(flat)?;
    // Glyph, Type3, and cached path masks are a hot replay path. Use the
    // deterministic edge-bucket scanline/subsample mask for every bounded
    // alpha mask so warm display-list replay stays on row-local scratch instead
    // of accumulator-heavy raster work.
    let mut alpha = vec![0u8; w.saturating_mul(h)];
    scan_flat_alpha_mask_rows(
        flat,
        rule,
        x0,
        y0,
        w,
        h,
        GLYPH_SCANLINE_ALPHA_VERTICAL_SAMPLES,
        |y, row| {
            let start = (y - y0) as usize * w;
            for (dst, value) in alpha[start..start + w].iter_mut().zip(row.iter()) {
                *dst = (*value).min(255) as u8;
            }
        },
    );
    Some(RasterizedGlyphMask {
        x: x0,
        y: y0,
        width: w as u32,
        height: h as u32,
        alpha,
        lcd: false,
    })
}

/// Paint the alpha mask [`rasterize_flat_alpha_mask`] would build for `flat`
/// straight into `buf` at offset `(dx, dy)`, one row at a time, without
/// allocating the mask: each row is accumulated exactly as the mask row would
/// be, remapped by `map_row` (the coverage curve the caller would apply to the
/// mask) and blended with the same row compositor. Rows outside the buffer
/// are skipped, since blending them would not touch any pixel. Returns `false`
/// when no mask would be built (the caller's fallback then applies).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_flat_alpha_mask_rows(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    rule: FillRule,
    dx: i32,
    dy: i32,
    color: PixelColor,
    mut map_row: impl FnMut(&mut [u8]),
) -> bool {
    let Some((x0, y0, w, h)) = flat_alpha_mask_bounds(flat) else {
        return false;
    };
    let device_y0 = i64::from(dy) + i64::from(y0);
    let first = (-device_y0).clamp(0, h as i64) as usize;
    let last = (i64::from(buf.height) - device_y0).clamp(0, h as i64) as usize;
    let device_x0 = i64::from(dx) + i64::from(x0);
    if last <= first
        || device_x0 >= i64::from(buf.width)
        || device_x0 + w as i64 <= 0
        || color[3] == 0
    {
        return true;
    }
    let mask_x = dx.saturating_add(x0);
    let mut alpha_row = vec![0u8; w];
    scan_flat_alpha_mask_rows(
        flat,
        rule,
        x0,
        y0 + first as i32,
        w,
        last - first,
        GLYPH_SCANLINE_ALPHA_VERTICAL_SAMPLES,
        |y, row| {
            for (dst, value) in alpha_row.iter_mut().zip(row.iter()) {
                *dst = (*value).min(255) as u8;
            }
            map_row(&mut alpha_row);
            buf.blend_alpha_mask(mask_x, dy.saturating_add(y), w as u32, 1, &alpha_row, color);
        },
    );
    true
}

/// The scanline/subsample coverage of mask rows `y_start..y_start + rows`
/// (absolute rows; columns `x0..x0 + w`): `visit(y, row)` receives each
/// row's accumulated coverage. Every row depends only on the geometry, so a
/// row range scans to exactly the rows of the whole mask.
#[allow(clippy::too_many_arguments)]
fn scan_flat_alpha_mask_rows(
    flat: &FlatPath,
    rule: FillRule,
    x0: i32,
    y_start: i32,
    w: usize,
    rows: usize,
    vertical_samples: usize,
    mut visit: impl FnMut(i32, &[u16]),
) {
    if w == 0 || rows == 0 || vertical_samples == 0 {
        return;
    }
    let mut accum = take_scanline_u16_vec(w);
    let mut crossings = take_scanline_crossing_vec();
    let edge_buckets = build_sequential_scanline_edge_buckets(flat, y_start, rows);
    let mut active_edges = ScanlineActiveEdges::default();
    let sample_weight = 255.0 / vertical_samples as f64;
    let rescues = thin_subpath_rescues(flat, vertical_samples);
    let mut rescue_cursor = 0usize;
    for row in 0..rows {
        accum.fill(0);
        if !rescues.is_empty() {
            add_thin_subpath_rescues_for_row(
                flat,
                &rescues,
                &mut rescue_cursor,
                y_start.saturating_add(row as i32),
                rule,
                x0,
                w,
                &mut accum,
                0,
                &mut crossings,
            );
        }
        let mut active_ready = false;
        if let Some(buckets) = edge_buckets.as_ref() {
            active_ready = active_edges.advance_to_row(buckets, row);
        }
        for sample in 0..vertical_samples {
            let scan_y =
                y_start as f64 + row as f64 + (sample as f64 + 0.5) / vertical_samples as f64;
            crossings.clear();
            if let Some(buckets) = edge_buckets.as_ref() {
                if active_ready {
                    active_edges.collect_current_crossings(buckets, scan_y, &mut crossings);
                } else {
                    collect_scanline_crossings_from_bucket(buckets, row, scan_y, &mut crossings);
                }
            } else {
                for (sp, &closed) in flat.subpaths.iter().zip(flat.closed.iter()) {
                    collect_scanline_crossings(sp, closed, scan_y, &mut crossings);
                }
            }
            if !prepare_scanline_crossings(&mut crossings) {
                continue;
            }
            PATH_RASTER_SCANLINE_FAST_ROWS.fetch_add(1, Ordering::Relaxed);
            match rule {
                FillRule::EvenOdd => {
                    let mut i = 0usize;
                    while i + 1 < crossings.len() {
                        add_alpha_span(
                            &mut accum,
                            0,
                            w,
                            crossings[i].0 - x0 as f64,
                            crossings[i + 1].0 - x0 as f64,
                            sample_weight,
                        );
                        i += 2;
                    }
                }
                FillRule::NonZero => {
                    let mut winding = 0i32;
                    let mut start_x: Option<f64> = None;
                    for (x, dir) in crossings.iter().copied() {
                        if winding != 0 {
                            if let Some(sx) = start_x.take() {
                                add_alpha_span(
                                    &mut accum,
                                    0,
                                    w,
                                    sx - x0 as f64,
                                    x - x0 as f64,
                                    sample_weight,
                                );
                            }
                        }
                        winding += dir;
                        if winding != 0 {
                            start_x = Some(x);
                        }
                    }
                }
            }
        }
        visit(y_start.saturating_add(row as i32), &accum);
    }
    return_scanline_crossing_vec(crossings);
    return_scanline_u16_vec(accum);
}

fn add_alpha_span(
    accum: &mut [u16],
    row_base: usize,
    w: usize,
    x_start: f64,
    x_end: f64,
    sample_weight: f64,
) {
    add_alpha_span_repeated(accum, row_base, w, x_start, x_end, sample_weight, 1);
}

/// `(value.round().clamp(0.0, 255.0)) as u16`, computed without a libm call
/// (`round` is not inlined on baseline x86-64). Exact: for `0 <= value < 255`
/// the truncated integer and the fractional part are both exact, and ties
/// round away from zero as `f64::round` does; NaN gives 0 like the cast.
#[inline(always)]
fn round_coverage(value: f64) -> u16 {
    if value >= 255.0 {
        return 255;
    }
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    let whole = value as u32;
    let fraction = value - f64::from(whole);
    (whole + u32::from(fraction >= 0.5)) as u16
}

/// [`add_alpha_span`] for `repeat` identical sample lines: every cell gets
/// `repeat` times the single-line increment, which equals adding the span
/// `repeat` times (the saturating sum of non-negative increments).
#[allow(clippy::too_many_arguments)]
#[inline]
fn add_alpha_span_repeated(
    accum: &mut [u16],
    row_base: usize,
    w: usize,
    x_start: f64,
    x_end: f64,
    sample_weight: f64,
    repeat: u32,
) {
    let scaled = |add: u16| -> u16 { (u32::from(add) * repeat).min(u32::from(u16::MAX)) as u16 };
    if x_end <= x_start || !x_start.is_finite() || !x_end.is_finite() {
        return;
    }
    let start = x_start.max(0.0);
    let end = x_end.min(w as f64);
    if end <= start {
        return;
    }
    let first = safe_floor_i32(start).max(0) as usize;
    let last = (safe_ceil_i32(end).max(0) as usize).min(w);
    // Cells lying entirely inside [start, end) are covered exactly 1.0 (cell
    // bounds are small integers, so `cell_end - cell_start` is exact): they
    // all receive the same increment the per-cell formula would compute, so
    // the interior is a constant add over a slice instead of per-cell
    // floating-point work. Only the two partial end cells take the general
    // path.
    let full_lo = (safe_ceil_i32(start).max(0) as usize).clamp(first, last);
    let full_hi = (safe_floor_i32(end).max(0) as usize).clamp(full_lo, last);
    let mut add_cell = |col: usize| {
        let cell_start = col as f64;
        let cell_end = cell_start + 1.0;
        let covered = end.min(cell_end) - start.max(cell_start);
        if covered <= 0.0 {
            return;
        }
        let add = scaled(round_coverage(covered * sample_weight));
        let idx = row_base + col;
        accum[idx] = accum[idx].saturating_add(add);
    };
    for col in first..full_lo {
        add_cell(col);
    }
    for col in full_hi..last {
        add_cell(col);
    }
    if full_hi > full_lo {
        let add = scaled(round_coverage(sample_weight));
        for value in &mut accum[row_base + full_lo..row_base + full_hi] {
            *value = value.saturating_add(add);
        }
    }
}

#[cfg(test)]
fn flat_bounds(flat: &FlatPath) -> Option<(f64, f64, f64, f64)> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &(x, y) in flat.subpaths.iter().flatten() {
        if !x.is_finite() || !y.is_finite() {
            continue;
        }
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
        Some((min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

type ScanlineEdgeRowSource<'a> = (
    &'a ScanlineEdgeBuckets,
    usize,
    Option<&'a ScanlineActiveEdges>,
);

fn collect_scanline_crossings_from_row_source(
    flat: &FlatPath,
    edge_source: Option<ScanlineEdgeRowSource<'_>>,
    scan_y: f64,
    crossings: &mut Vec<(f64, i32)>,
) {
    if let Some((buckets, _row, Some(active_edges))) = edge_source {
        active_edges.collect_current_crossings(buckets, scan_y, crossings);
    } else if let Some((buckets, row, None)) = edge_source {
        collect_scanline_crossings_from_bucket(buckets, row, scan_y, crossings);
    } else {
        for (sp, &closed) in flat.subpaths.iter().zip(flat.closed.iter()) {
            collect_scanline_crossings(sp, closed, scan_y, crossings);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn accumulate_scanline_row(
    flat: &FlatPath,
    edge_source: Option<ScanlineEdgeRowSource<'_>>,
    rule: FillRule,
    scan_y: f64,
    x0: i32,
    w: usize,
    crossings: &mut Vec<(f64, i32)>,
    row_accum: &mut [u16],
) {
    crossings.clear();
    collect_scanline_crossings_from_row_source(flat, edge_source, scan_y, crossings);
    if !prepare_scanline_crossings(crossings) {
        return;
    }
    PATH_RASTER_SCANLINE_FAST_ROWS.fetch_add(1, Ordering::Relaxed);
    match rule {
        FillRule::EvenOdd => {
            let mut i = 0usize;
            while i + 1 < crossings.len() {
                add_alpha_span(
                    row_accum,
                    0,
                    w,
                    crossings[i].0 - x0 as f64,
                    crossings[i + 1].0 - x0 as f64,
                    255.0,
                );
                i += 2;
            }
        }
        FillRule::NonZero => {
            let mut winding = 0i32;
            let mut start_x: Option<f64> = None;
            for (x, dir) in crossings.iter().copied() {
                if winding != 0 {
                    if let Some(sx) = start_x.take() {
                        add_alpha_span(row_accum, 0, w, sx - x0 as f64, x - x0 as f64, 255.0);
                    }
                }
                winding += dir;
                if winding != 0 {
                    start_x = Some(x);
                }
            }
        }
    }
}

thread_local! {
    /// Scratch crossings of the next sample line in
    /// [`accumulate_scanline_row_subsampled`].
    static NEXT_SAMPLE_CROSSINGS: std::cell::RefCell<Vec<(f64, i32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// True when two unsorted crossing lists are bit-for-bit the same.
fn same_scanline_crossings(a: &[(f64, i32)], b: &[(f64, i32)]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(a, b)| a.0.to_bits() == b.0.to_bits() && a.1 == b.1)
}

/// Span accumulation for a run of identical sample lines of one pixel row.
struct SampleLineGroup {
    rule: FillRule,
    x0: i32,
    w: usize,
    sample_weight: f64,
}

impl SampleLineGroup {
    /// Add the spans of `crossings` (unsorted, as collected) `repeat` times.
    fn accumulate(
        &self,
        crossings: &mut Vec<(f64, i32)>,
        repeat: u32,
        row_accum: &mut [u16],
        touched: &mut (usize, usize),
    ) {
        let (x0, w, sample_weight) = (self.x0, self.w, self.sample_weight);
        if !prepare_scanline_crossings(crossings) {
            return;
        }
        if let (Some(first), Some(last)) = (crossings.first(), crossings.last()) {
            let lo = safe_floor_i32(first.0 - x0 as f64).clamp(0, w as i32) as usize;
            let hi = safe_ceil_i32(last.0 - x0 as f64)
                .saturating_add(1)
                .clamp(0, w as i32) as usize;
            *touched = (touched.0.min(lo), touched.1.max(hi));
        }
        PATH_RASTER_SCANLINE_FAST_ROWS.fetch_add(u64::from(repeat), Ordering::Relaxed);
        match self.rule {
            FillRule::EvenOdd => {
                let mut i = 0usize;
                while i + 1 < crossings.len() {
                    add_alpha_span_repeated(
                        row_accum,
                        0,
                        w,
                        crossings[i].0 - x0 as f64,
                        crossings[i + 1].0 - x0 as f64,
                        sample_weight,
                        repeat,
                    );
                    i += 2;
                }
            }
            FillRule::NonZero => {
                let mut winding = 0i32;
                let mut start_x: Option<f64> = None;
                for (x, dir) in crossings.iter().copied() {
                    if winding != 0 {
                        if let Some(sx) = start_x.take() {
                            add_alpha_span_repeated(
                                row_accum,
                                0,
                                w,
                                sx - x0 as f64,
                                x - x0 as f64,
                                sample_weight,
                                repeat,
                            );
                        }
                    }
                    winding += dir;
                    if winding != 0 {
                        start_x = Some(x);
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn accumulate_scanline_row_subsampled(
    flat: &FlatPath,
    edge_source: Option<ScanlineEdgeRowSource<'_>>,
    rule: FillRule,
    row_y: i32,
    x0: i32,
    w: usize,
    vertical_samples: usize,
    crossings: &mut Vec<(f64, i32)>,
    row_accum: &mut [u16],
    clear_row: bool,
) -> (usize, usize) {
    if clear_row {
        row_accum.fill(0);
    }
    if vertical_samples <= 1 {
        accumulate_scanline_row(
            flat,
            edge_source,
            rule,
            row_y as f64 + 0.5,
            x0,
            w,
            crossings,
            row_accum,
        );
        return (0, w);
    }
    // Columns this row touches: every span lies between the first and the
    // last sorted crossing of its sample line.
    let mut touched = (w, 0usize);
    let sample_scale = 1.0 / vertical_samples as f64;
    let sample_weight = 255.0 * sample_scale;
    let group = SampleLineGroup {
        rule,
        x0,
        w,
        sample_weight,
    };
    // Consecutive sample lines that cross exactly the same edges at exactly
    // the same x (vertical edges: rectangle sides, the butt ends of
    // horizontal dashes) produce the same spans. They are accumulated once
    // with a repeat count, which adds the same integer coverage as adding
    // each sample line separately.
    crossings.clear();
    collect_scanline_crossings_from_row_source(
        flat,
        edge_source,
        row_y as f64 + 0.5 * sample_scale,
        crossings,
    );
    let mut repeat = 1u32;
    NEXT_SAMPLE_CROSSINGS.with(|next| {
        let mut next = next.borrow_mut();
        for sample in 1..vertical_samples {
            let scan_y = row_y as f64 + (sample as f64 + 0.5) * sample_scale;
            next.clear();
            collect_scanline_crossings_from_row_source(flat, edge_source, scan_y, &mut next);
            if same_scanline_crossings(crossings, &next) {
                repeat += 1;
                continue;
            }
            group.accumulate(crossings, repeat, row_accum, &mut touched);
            std::mem::swap(crossings, &mut *next);
            repeat = 1;
        }
    });
    group.accumulate(crossings, repeat, row_accum, &mut touched);
    touched
}

#[allow(clippy::too_many_arguments)]
fn fill_flat_with_compositor_scanline<F>(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    rule: FillRule,
    cancel: Option<&CancelToken>,
    bounds: (i32, i32, i32, i32),
    vertical_samples: usize,
    rescues: &[ThinSubpathRescue],
    composite_pixel: &mut F,
) -> bool
where
    F: FnMut(&mut PixelBuffer, i32, i32, f32),
{
    let (x0, y0, x1, y1) = bounds;
    let w = (x1 - x0) as usize;
    if w == 0 || y1 <= y0 {
        return true;
    }
    let mut crossings = take_scanline_crossing_vec();
    let mut row_accum = take_scanline_u16_vec(w);
    let h = (y1 - y0) as usize;
    let edge_buckets = build_sequential_scanline_edge_buckets(flat, y0, h);
    let mut active_edges = ScanlineActiveEdges::default();
    let mut rescue_cursor = 0usize;
    for y in y0..y1 {
        if (y - y0) % 16 == 0 && cancel.is_some_and(CancelToken::is_cancelled) {
            return_scanline_u16_vec(row_accum);
            return_scanline_crossing_vec(crossings);
            return false;
        }
        let mut edge_source = None;
        if let Some(buckets) = edge_buckets.as_ref() {
            let row = (y - y0) as usize;
            let active = active_edges.advance_to_row(buckets, row);
            edge_source = Some((buckets, row, active.then_some(&active_edges)));
        }
        let _ = accumulate_scanline_row_subsampled(
            flat,
            edge_source,
            rule,
            y,
            x0,
            w,
            vertical_samples,
            &mut crossings,
            &mut row_accum,
            true,
        );
        if !rescues.is_empty() {
            add_thin_subpath_rescues_for_row(
                flat,
                rescues,
                &mut rescue_cursor,
                y,
                rule,
                x0,
                w,
                &mut row_accum,
                0,
                &mut crossings,
            );
        }
        for (offset, value) in row_accum.iter().copied().enumerate() {
            if value == 0 {
                continue;
            }
            let coverage = f32::from(value.min(255) as u8) / 255.0;
            composite_pixel(buf, x0 + offset as i32, y, coverage);
        }
    }
    return_scanline_u16_vec(row_accum);
    return_scanline_crossing_vec(crossings);
    true
}

#[allow(clippy::too_many_arguments)]
fn fill_flat_with_color_compositor_scanline(
    buf: &mut PixelBuffer,
    flat: &FlatPath,
    color: PixelColor,
    rule: FillRule,
    cancel: Option<&CancelToken>,
    bounds: (i32, i32, i32, i32),
    vertical_samples: usize,
    rescues: &[ThinSubpathRescue],
    lut: Option<&[u8; 256]>,
) -> bool {
    let (x0, y0, x1, y1) = bounds;
    let w = (x1 - x0) as usize;
    if w == 0 || y1 <= y0 {
        return true;
    }
    let mut crossings = take_scanline_crossing_vec();
    let mut row_accum = take_scanline_u16_vec(w);
    let mut memo = take_coverage_blend_memo();
    let h = (y1 - y0) as usize;
    let edge_buckets = build_sequential_scanline_edge_buckets(flat, y0, h);
    let mut active_edges = ScanlineActiveEdges::default();
    let mut rescue_cursor = 0usize;
    for y in y0..y1 {
        if (y - y0) % 16 == 0 && cancel.is_some_and(CancelToken::is_cancelled) {
            return_scanline_u16_vec(row_accum);
            return_scanline_crossing_vec(crossings);
            return false;
        }
        let mut edge_source = None;
        if let Some(buckets) = edge_buckets.as_ref() {
            let row = (y - y0) as usize;
            let active = active_edges.advance_to_row(buckets, row);
            edge_source = Some((buckets, row, active.then_some(&active_edges)));
        }
        // `row_accum` is all zero here: only the touched column range is
        // painted and cleared, so per-row work follows the outline's width
        // at this row rather than the whole bounding box.
        let mut touched = accumulate_scanline_row_subsampled(
            flat,
            edge_source,
            rule,
            y,
            x0,
            w,
            vertical_samples,
            &mut crossings,
            &mut row_accum,
            false,
        );
        if !rescues.is_empty()
            && add_thin_subpath_rescues_for_row(
                flat,
                rescues,
                &mut rescue_cursor,
                y,
                rule,
                x0,
                w,
                &mut row_accum,
                0,
                &mut crossings,
            )
        {
            touched = (0, w);
        }
        if touched.0 < touched.1 {
            let (lo, hi) = touched;
            paint_alpha_row_runs(
                buf,
                &row_accum[lo..hi],
                x0 + lo as i32,
                y,
                color,
                lut,
                &mut memo,
                |_| {},
            );
            row_accum[lo..hi].fill(0);
        }
    }
    return_scanline_u16_vec(row_accum);
    return_scanline_crossing_vec(crossings);
    return_coverage_blend_memo(memo);
    true
}

fn paint_alpha_row_runs(
    buf: &mut PixelBuffer,
    row_accum: &[u16],
    x0: i32,
    y: i32,
    color: PixelColor,
    lut: Option<&[u8; 256]>,
    memo: &mut CoverageBlendMemo,
    mut on_run: impl FnMut(u64),
) {
    // The raster counters are summed per row: one atomic add per row instead
    // of two per run (short dash pieces make many runs per row).
    let mut span_pixels = 0u64;
    let mut solid_pixels = 0u64;
    let mut col = 0usize;
    while col < row_accum.len() {
        let raw = row_accum[col].min(255) as u8;
        if raw == 0 {
            col += 1;
            continue;
        }
        let run_start = col;
        let mut run_end = col + 1;
        while run_end < row_accum.len() && row_accum[run_end].min(255) as u8 == raw {
            run_end += 1;
        }
        let alpha = lut.map_or(raw, |lut| lut[usize::from(raw)]);
        if alpha == 0 {
            col = run_end;
            continue;
        }
        let px0 = x0 + run_start as i32;
        let px1 = x0 + run_end as i32;
        let pixels = (px1 - px0).max(0) as u64;
        span_pixels += pixels;
        on_run(pixels);
        if alpha == 255 {
            solid_pixels += pixels;
            buf.fill_rect(px0, y, px1 - px0, 1, color);
        } else {
            buf.blend_coverage_run(px0, px1, y, color, alpha, memo);
        }
        col = run_end;
    }
    if span_pixels > 0 {
        PATH_RASTER_SCANLINE_SPAN_PIXELS.fetch_add(span_pixels, Ordering::Relaxed);
    }
    if solid_pixels > 0 {
        PATH_RASTER_SOLID_RUN_PIXELS.fetch_add(solid_pixels, Ordering::Relaxed);
    }
}

fn light_grid_fit_flat_glyph(flat: &mut FlatPath, device_t: &Transform2D) {
    if !device_t.is_axis_aligned() {
        return;
    }

    const MAX_BASELINE_SHIFT: f64 = 0.35;
    const MAX_STEM_EDGE_SHIFT: f64 = 0.18;

    let (_, baseline_y) = device_t.transform_point(0.0, 0.0);
    let baseline_shift = baseline_y.round() - baseline_y;
    if baseline_shift.abs() <= MAX_BASELINE_SHIFT {
        translate_flat(flat, 0.0, baseline_shift);
    }
    snap_near_pixel_edges(flat, MAX_STEM_EDGE_SHIFT);
}

fn translate_flat(flat: &mut FlatPath, dx: f64, dy: f64) {
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    for subpath in &mut flat.subpaths {
        for point in subpath {
            point.0 += dx;
            point.1 += dy;
        }
    }
}

fn snap_near_pixel_edges(flat: &mut FlatPath, max_shift: f64) {
    if max_shift <= 0.0 || !max_shift.is_finite() {
        return;
    }
    for subpath in &mut flat.subpaths {
        for point in subpath {
            point.0 = snap_near_integer(point.0, max_shift);
            point.1 = snap_near_integer(point.1, max_shift);
        }
    }
}

fn snap_near_integer(value: f64, max_shift: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let snapped = value.round();
    if (snapped - value).abs() <= max_shift {
        snapped
    } else {
        value
    }
}

#[derive(Debug, Clone)]
struct StrokeStyle {
    half_width: f64,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f64,
}

#[derive(Debug, Clone, Copy)]
struct StrokeSegment {
    dir: (f64, f64),
    normal: (f64, f64),
}

type StrokeOutlineContour = Vec<(f64, f64)>;
type StrokeOutlineContours = (StrokeOutlineContour, StrokeOutlineContour);

const MAX_DASH_POLYLINE_PIECES: usize = 1024;

pub(crate) fn stroke_flat_path(
    flat: &FlatPath,
    width_px: f64,
    dash: &DashState,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f64,
) -> FlatPath {
    if width_px <= 0.0 || !width_px.is_finite() {
        return FlatPath::default();
    }

    let style = StrokeStyle {
        half_width: width_px / 2.0,
        cap,
        join,
        miter_limit: miter_limit.max(1.0),
    };
    let mut outline = FlatPath::default();

    for (idx, subpath) in flat.subpaths.iter().enumerate() {
        let closed = flat.closed.get(idx).copied().unwrap_or(false);
        let Some(points) = normalize_stroke_points(subpath, closed) else {
            continue;
        };

        if closed && dash.is_solid() {
            if let Some(segments) = build_stroke_segments(&points, true) {
                if let Some((outer_side, inner_side)) =
                    stroked_closed_outline_contours(&points, &segments, &style)
                {
                    push_outline_subpath(&mut outline, outer_side);
                    push_outline_subpath(&mut outline, inner_side);
                }
            }
            continue;
        }

        let mut dashed_points = points.clone();
        if closed {
            dashed_points.push(points[0]);
        }
        for dash_piece in dash_polyline(&dashed_points, dash) {
            if matches!(style.cap, LineCap::Butt) && is_zero_dash_dot(&dash_piece) {
                // A zero-length dash with butt caps has no area (PDF 8.4.3.6).
                continue;
            }
            if let Some(poly) = stroked_polyline_outline(&dash_piece, false, &style) {
                push_outline_subpath(&mut outline, poly);
            }
        }
    }

    outline
}

fn push_outline_subpath(outline: &mut FlatPath, mut poly: Vec<(f64, f64)>) {
    if poly.len() < 3 {
        return;
    }
    if let (Some(first), Some(last)) = (poly.first().copied(), poly.last().copied()) {
        if distance(first, last) < 1e-8 {
            poly.pop();
        }
    }
    outline.subpaths.push(poly);
    // `fill_flat_aa` implicitly closes subpaths whose closed flag is false.
    outline.closed.push(false);
}

fn normalize_stroke_points(points: &[(f64, f64)], closed: bool) -> Option<Vec<(f64, f64)>> {
    let mut cleaned = Vec::with_capacity(points.len());
    for &p in points {
        if !p.0.is_finite() || !p.1.is_finite() {
            continue;
        }
        if cleaned.last().is_none_or(|&last| distance(last, p) > 1e-8) {
            cleaned.push(p);
        }
    }
    if closed && cleaned.len() >= 2 && distance(cleaned[0], *cleaned.last()?) < 1e-8 {
        cleaned.pop();
    }
    if cleaned.len() < 2 || (closed && cleaned.len() < 3) {
        None
    } else {
        Some(cleaned)
    }
}

/// Device-space length of the stand-in segment for a zero-length dash. Long
/// enough to give the stroker a direction, far too short to be visible.
const ZERO_DASH_DOT_LENGTH: f64 = 1e-4;

fn is_zero_dash_dot(piece: &[(f64, f64)]) -> bool {
    piece.len() == 2 && distance(piece[0], piece[1]) <= ZERO_DASH_DOT_LENGTH * 1.5
}

fn dash_polyline(points: &[(f64, f64)], dash: &DashState) -> Vec<Vec<(f64, f64)>> {
    if dash_polyline_would_expand_too_much(points, dash) {
        return vec![points.to_vec()];
    }

    let mut pieces = Vec::new();
    let mut current = Vec::new();
    let mut dash_state = dash.clone();

    for window in points.windows(2) {
        let p0 = window[0];
        let p1 = window[1];
        let dx = p1.0 - p0.0;
        let dy = p1.1 - p0.1;
        let seg_len = (dx * dx + dy * dy).sqrt();
        if seg_len < 1e-10 || !seg_len.is_finite() {
            continue;
        }
        let ux = dx / seg_len;
        let uy = dy / seg_len;

        for (t0, t1, drawing) in dash_state.advance(seg_len) {
            if !drawing {
                finish_dash_piece(&mut pieces, &mut current);
                continue;
            }
            if t1 - t0 <= 1e-10 {
                // Zero-length dash: a tiny segment along the path direction.
                // Its caps give the PDF result (round = dot, projecting =
                // square, butt = nothing visible).
                finish_dash_piece(&mut pieces, &mut current);
                let q = (p0.0 + ux * t0, p0.1 + uy * t0);
                pieces.push(vec![
                    q,
                    (
                        q.0 + ux * ZERO_DASH_DOT_LENGTH,
                        q.1 + uy * ZERO_DASH_DOT_LENGTH,
                    ),
                ]);
                continue;
            }

            let q0 = (p0.0 + ux * t0, p0.1 + uy * t0);
            let q1 = (p0.0 + ux * t1, p0.1 + uy * t1);
            if current
                .last()
                .is_some_and(|&last| distance(last, q0) > 1e-7)
            {
                finish_dash_piece(&mut pieces, &mut current);
            }
            if current.is_empty() {
                current.push(q0);
            }
            if current.last().is_none_or(|&last| distance(last, q1) > 1e-8) {
                current.push(q1);
            }
        }
    }

    finish_dash_piece(&mut pieces, &mut current);
    pieces
}

fn dash_polyline_would_expand_too_much(points: &[(f64, f64)], dash: &DashState) -> bool {
    if dash.is_solid() {
        return false;
    }
    let total_len = points
        .windows(2)
        .filter_map(|window| {
            let p0 = window[0];
            let p1 = window[1];
            let dx = p1.0 - p0.0;
            let dy = p1.1 - p0.1;
            let len = (dx * dx + dy * dy).sqrt();
            len.is_finite().then_some(len)
        })
        .sum::<f64>();
    dash.estimated_segment_count(total_len)
        .is_some_and(|count| count > MAX_DASH_POLYLINE_PIECES)
}

fn finish_dash_piece(pieces: &mut Vec<Vec<(f64, f64)>>, current: &mut Vec<(f64, f64)>) {
    if current.len() >= 2 {
        pieces.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

fn stroked_polyline_outline(
    points: &[(f64, f64)],
    closed: bool,
    style: &StrokeStyle,
) -> Option<Vec<(f64, f64)>> {
    let segments = build_stroke_segments(points, closed)?;
    if closed {
        let (outer_side, inner_side) = stroked_closed_outline_contours(points, &segments, style)?;
        let mut poly = outer_side;
        poly.extend(inner_side);
        Some(poly)
    } else {
        stroked_open_outline(points, &segments, style)
    }
}

fn build_stroke_segments(points: &[(f64, f64)], closed: bool) -> Option<Vec<StrokeSegment>> {
    let count = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    let mut segments = Vec::with_capacity(count);
    for i in 0..count {
        let start = points[i];
        let end = points[(i + 1) % points.len()];
        let dx = end.0 - start.0;
        let dy = end.1 - start.1;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-10 || !len.is_finite() {
            continue;
        }
        let dir = (dx / len, dy / len);
        let normal = (-dir.1, dir.0);
        segments.push(StrokeSegment { dir, normal });
    }
    if segments.is_empty() {
        None
    } else {
        Some(segments)
    }
}

fn stroked_open_outline(
    points: &[(f64, f64)],
    segments: &[StrokeSegment],
    style: &StrokeStyle,
) -> Option<Vec<(f64, f64)>> {
    let first = *segments.first()?;
    let last = *segments.last()?;
    let start_center = match style.cap {
        LineCap::ProjectingSquare => sub(points[0], scale(first.dir, style.half_width)),
        _ => points[0],
    };
    let end_center = match style.cap {
        LineCap::ProjectingSquare => add(*points.last()?, scale(last.dir, style.half_width)),
        _ => *points.last()?,
    };

    let mut left = Vec::with_capacity(points.len() + 8);
    let mut right = Vec::with_capacity(points.len() + 8);
    let start_left = offset_point(start_center, first.normal, 1.0, style.half_width);
    let start_right = offset_point(start_center, first.normal, -1.0, style.half_width);
    let end_left = offset_point(end_center, last.normal, 1.0, style.half_width);
    let end_right = offset_point(end_center, last.normal, -1.0, style.half_width);

    left.push(start_left);
    right.push(start_right);
    for i in 1..(points.len() - 1) {
        left.extend(join_points(
            points[i],
            segments[i - 1],
            segments[i],
            1.0,
            style,
        ));
        right.extend(join_points(
            points[i],
            segments[i - 1],
            segments[i],
            -1.0,
            style,
        ));
    }
    left.push(end_left);
    right.push(end_right);

    let mut poly = left;
    if matches!(style.cap, LineCap::Round) {
        poly.extend(arc_points_towards(
            end_center,
            end_left,
            end_right,
            last.dir,
            style.half_width,
        ));
    }
    poly.extend(right.into_iter().rev());
    if matches!(style.cap, LineCap::Round) {
        poly.extend(arc_points_towards(
            start_center,
            start_right,
            start_left,
            scale(first.dir, -1.0),
            style.half_width,
        ));
    }

    Some(poly)
}

fn stroked_closed_outline_contours(
    points: &[(f64, f64)],
    segments: &[StrokeSegment],
    style: &StrokeStyle,
) -> Option<StrokeOutlineContours> {
    if points.len() < 3 || segments.len() < 3 {
        return None;
    }

    let mut left = Vec::with_capacity(points.len() + 8);
    let mut right = Vec::with_capacity(points.len() + 8);
    for i in 0..points.len() {
        let prev = segments[(i + segments.len() - 1) % segments.len()];
        let next = segments[i % segments.len()];
        left.extend(join_points(points[i], prev, next, 1.0, style));
        right.extend(join_points(points[i], prev, next, -1.0, style));
    }

    right.reverse();
    Some((left, right))
}

fn join_points(
    vertex: (f64, f64),
    prev: StrokeSegment,
    next: StrokeSegment,
    side: f64,
    style: &StrokeStyle,
) -> Vec<(f64, f64)> {
    let prev_offset = offset_point(vertex, prev.normal, side, style.half_width);
    let next_offset = offset_point(vertex, next.normal, side, style.half_width);
    let intersection = line_intersection(prev_offset, prev.dir, next_offset, next.dir);

    match style.join {
        LineJoin::Miter => {
            if let Some(p) = intersection {
                if distance(vertex, p) <= style.half_width * style.miter_limit + 1e-8 {
                    return vec![p];
                }
            }
            vec![prev_offset, next_offset]
        }
        LineJoin::Bevel => vec![prev_offset, next_offset],
        LineJoin::Round => {
            let mut arc = arc_points_shortest(vertex, prev_offset, next_offset, style.half_width);
            if arc.len() < 2 {
                if let Some(p) = intersection {
                    arc.push(p);
                }
            }
            arc
        }
    }
}

fn offset_point(p: (f64, f64), normal: (f64, f64), side: f64, half_width: f64) -> (f64, f64) {
    add(p, scale(normal, side * half_width))
}

fn line_intersection(
    p: (f64, f64),
    r: (f64, f64),
    q: (f64, f64),
    s: (f64, f64),
) -> Option<(f64, f64)> {
    let denom = cross(r, s);
    if denom.abs() < 1e-10 {
        return None;
    }
    let t = cross(sub(q, p), s) / denom;
    Some(add(p, scale(r, t)))
}

fn arc_points_shortest(
    center: (f64, f64),
    from: (f64, f64),
    to: (f64, f64),
    radius: f64,
) -> Vec<(f64, f64)> {
    let a0 = angle(center, from);
    let a1 = angle(center, to);
    let mut delta = a1 - a0;
    while delta <= -std::f64::consts::PI {
        delta += std::f64::consts::TAU;
    }
    while delta > std::f64::consts::PI {
        delta -= std::f64::consts::TAU;
    }
    arc_points_with_delta(center, a0, delta, radius)
}

fn arc_points_towards(
    center: (f64, f64),
    from: (f64, f64),
    to: (f64, f64),
    desired: (f64, f64),
    radius: f64,
) -> Vec<(f64, f64)> {
    let a0 = angle(center, from);
    let a1 = angle(center, to);
    let mut ccw = a1 - a0;
    while ccw < 0.0 {
        ccw += std::f64::consts::TAU;
    }
    while ccw >= std::f64::consts::TAU {
        ccw -= std::f64::consts::TAU;
    }
    let cw = ccw - std::f64::consts::TAU;
    let ccw_mid = (a0 + ccw / 2.0).cos() * desired.0 + (a0 + ccw / 2.0).sin() * desired.1;
    let cw_mid = (a0 + cw / 2.0).cos() * desired.0 + (a0 + cw / 2.0).sin() * desired.1;
    let delta = if ccw_mid >= cw_mid { ccw } else { cw };
    arc_points_with_delta(center, a0, delta, radius)
}

fn arc_points_with_delta(
    center: (f64, f64),
    start_angle: f64,
    delta: f64,
    radius: f64,
) -> Vec<(f64, f64)> {
    if radius <= 0.0 || !radius.is_finite() || !delta.is_finite() {
        return Vec::new();
    }
    let steps = ((delta.abs() / (std::f64::consts::PI / 12.0)).ceil() as usize).max(1);
    (0..=steps)
        .map(|i| {
            let t = i as f64 / steps as f64;
            let a = start_angle + delta * t;
            (center.0 + radius * a.cos(), center.1 + radius * a.sin())
        })
        .collect()
}

fn angle(center: (f64, f64), p: (f64, f64)) -> f64 {
    (p.1 - center.1).atan2(p.0 - center.0)
}

fn add(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 - b.0, a.1 - b.1)
}

fn scale(a: (f64, f64), s: f64) -> (f64, f64) {
    (a.0 * s, a.1 * s)
}

fn cross(a: (f64, f64), b: (f64, f64)) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    (dx * dx + dy * dy).sqrt()
}

fn safe_floor_i32(value: f64) -> i32 {
    if !value.is_finite() {
        0
    } else if value <= i32::MIN as f64 {
        i32::MIN
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        // `value.floor() as i32` without the libm call: truncation rounds
        // toward zero, so step down when that landed above `value`.
        let truncated = value as i32;
        truncated - i32::from(f64::from(truncated) > value)
    }
}

fn safe_ceil_i32(value: f64) -> i32 {
    if !value.is_finite() {
        0
    } else if value <= i32::MIN as f64 {
        i32::MIN
    } else if value >= i32::MAX as f64 {
        i32::MAX
    } else {
        // `value.ceil() as i32` without the libm call.
        let truncated = value as i32;
        truncated + i32::from(f64::from(truncated) < value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::buffer::{RenderMode, BLACK, BLUE, GREEN, RED, TRANSPARENT, WHITE};

    #[test]
    fn inline_floor_ceil_and_round_match_libm() {
        let mut values = vec![
            0.0,
            -0.0,
            0.5,
            1.5,
            2.5,
            -0.5,
            -1.5,
            254.5,
            254.49999999999997,
            255.0,
            300.0,
            0.49999999999999994,
            -2147483647.5,
            2147483646.5,
            1e-300,
            -1e-300,
        ];
        let mut seed = 0x9e3779b97f4a7c15u64;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let unit = (seed >> 11) as f64 / (1u64 << 53) as f64;
            values.push(unit * 600.0 - 300.0);
            values.push((unit * 1024.0).round() / 4.0 - 128.0);
        }
        for value in values {
            assert_eq!(safe_floor_i32(value), value.floor() as i32, "floor {value}");
            assert_eq!(safe_ceil_i32(value), value.ceil() as i32, "ceil {value}");
            assert_eq!(
                round_coverage(value),
                value.round().clamp(0.0, 255.0) as u16,
                "round {value}"
            );
        }
        assert_eq!(round_coverage(f64::NAN), 0);
    }

    #[test]
    fn crossing_sort_matches_the_stable_sort() {
        let mut seed = 0x2545f4914f6cdd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for len in [0usize, 1, 2, 3, 8, 40, 300] {
            for pattern in 0..3 {
                let mut crossings: Vec<(f64, i32)> = (0..len)
                    .map(|i| {
                        let x = match pattern {
                            // Swapped pairs, as dash pieces produce.
                            0 => (i ^ 1) as f64 * 1.5,
                            // Random with many ties.
                            1 => (next() % 7) as f64,
                            _ => (next() >> 11) as f64 / 1e9,
                        };
                        (x, if next() % 2 == 0 { 1 } else { -1 })
                    })
                    .collect();
                let mut expected = crossings.clone();
                expected.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
                sort_scanline_crossings(&mut crossings);
                assert_eq!(crossings, expected, "len {len} pattern {pattern}");
            }
        }
    }

    /// The per-sample-line accumulation `accumulate_scanline_row_subsampled`
    /// did before identical sample lines were grouped.
    fn reference_row_accumulation(
        flat: &FlatPath,
        source: Option<ScanlineEdgeRowSource<'_>>,
        rule: FillRule,
        row_y: i32,
        x0: i32,
        w: usize,
    ) -> (Vec<u16>, (usize, usize)) {
        let samples = PATH_SCANLINE_COLOR_VERTICAL_SAMPLES;
        let mut accum = vec![0u16; w];
        let mut touched = (w, 0usize);
        let mut crossings = Vec::new();
        let scale = 1.0 / samples as f64;
        for sample in 0..samples {
            let scan_y = row_y as f64 + (sample as f64 + 0.5) * scale;
            crossings.clear();
            collect_scanline_crossings_from_row_source(flat, source, scan_y, &mut crossings);
            crossings.retain(|(x, dir): &(f64, i32)| x.is_finite() && *dir != 0);
            if crossings.len() < 2 {
                continue;
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            let lo = safe_floor_i32(crossings[0].0 - x0 as f64).clamp(0, w as i32) as usize;
            let hi = safe_ceil_i32(crossings[crossings.len() - 1].0 - x0 as f64)
                .saturating_add(1)
                .clamp(0, w as i32) as usize;
            touched = (touched.0.min(lo), touched.1.max(hi));
            let mut spans = Vec::new();
            match rule {
                FillRule::EvenOdd => {
                    for pair in crossings.as_chunks::<2>().0 {
                        spans.push((pair[0].0, pair[1].0));
                    }
                }
                FillRule::NonZero => {
                    let mut winding = 0;
                    let mut start = None;
                    for &(x, dir) in &crossings {
                        if winding != 0 {
                            if let Some(sx) = start.take() {
                                spans.push((sx, x));
                            }
                        }
                        winding += dir;
                        if winding != 0 {
                            start = Some(x);
                        }
                    }
                }
            }
            for (a, b) in spans {
                add_alpha_span(
                    &mut accum,
                    0,
                    w,
                    a - x0 as f64,
                    b - x0 as f64,
                    255.0 * scale,
                );
            }
        }
        (accum, touched)
    }

    #[test]
    fn grouped_sample_lines_accumulate_exactly_like_separate_ones() {
        let device = Transform2D::new(2.0, 0.0, 0.0, -2.0, 0.3, 200.7);
        let mut flats = Vec::new();
        for (dash, x1, y1) in [
            (vec![3.0, 2.0], 90.0, 10.0),
            (vec![3.0, 2.0], 90.0, 13.3),
            (vec![1.0, 1.0], 10.0, 80.0),
            (vec![6.0, 3.0], 70.0, 60.0),
        ] {
            let line = FlatPath {
                subpaths: vec![vec![
                    device.transform_point(10.1, 10.2),
                    device.transform_point(x1, y1),
                ]],
                closed: vec![false],
            };
            for cap in [LineCap::Butt, LineCap::Round, LineCap::ProjectingSquare] {
                flats.push(stroke_flat_path(
                    &line,
                    1.4,
                    &DashState::new(dash.clone(), 0.5),
                    cap,
                    LineJoin::Miter,
                    10.0,
                ));
            }
        }
        flats.push(FlatPath {
            subpaths: vec![
                vec![(2.0, 2.0), (30.5, 2.0), (30.5, 9.25), (2.0, 9.25)],
                vec![(40.25, 1.5), (60.0, 7.0), (45.0, 12.0)],
            ],
            closed: vec![true, false],
        });
        for flat in &flats {
            let (mut x_min, mut y_min, mut x_max, mut y_max) =
                (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for &(x, y) in flat.subpaths.iter().flatten() {
                x_min = x_min.min(x);
                y_min = y_min.min(y);
                x_max = x_max.max(x);
                y_max = y_max.max(y);
            }
            let x0 = x_min.floor() as i32 - 1;
            let w = (x_max.ceil() as i32 + 2 - x0) as usize;
            let (y0, y1) = (y_min.floor() as i32, y_max.ceil() as i32 + 1);
            let buckets = build_scanline_edge_buckets(flat, y0, (y1 - y0) as usize);
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let mut active = ScanlineActiveEdges::default();
                let mut crossings = Vec::new();
                for y in y0..y1 {
                    let row = (y - y0) as usize;
                    let source = buckets.as_ref().map(|buckets| {
                        let ready = active.advance_to_row(buckets, row);
                        (buckets, row, ready.then_some(&active))
                    });
                    let mut accum = vec![0u16; w];
                    let touched = accumulate_scanline_row_subsampled(
                        flat,
                        source,
                        rule,
                        y,
                        x0,
                        w,
                        PATH_SCANLINE_COLOR_VERTICAL_SAMPLES,
                        &mut crossings,
                        &mut accum,
                        false,
                    );
                    let (expected, expected_touched) =
                        reference_row_accumulation(flat, source, rule, y, x0, w);
                    assert_eq!(accum, expected, "row {y}");
                    assert_eq!(touched, expected_touched, "row {y}");
                }
            }
        }
    }

    fn assert_pixel_buffers_eq(actual: &PixelBuffer, expected: &PixelBuffer) {
        let actual_bytes = actual.rgba_bytes();
        let expected_bytes = expected.rgba_bytes();
        if actual_bytes == expected_bytes {
            return;
        }

        let mut mismatches = 0usize;
        let mut samples = Vec::new();
        for (pixel, (a, e)) in actual_bytes
            .as_chunks::<4>()
            .0
            .iter()
            .zip(expected_bytes.as_chunks::<4>().0.iter())
            .enumerate()
        {
            if a != e {
                mismatches += 1;
                if samples.len() < 12 {
                    let x = (pixel as u32 % actual.width) as i32;
                    let y = (pixel as u32 / actual.width) as i32;
                    samples.push((x, y, [a[0], a[1], a[2], a[3]], [e[0], e[1], e[2], e[3]]));
                }
            }
        }

        panic!("pixel buffers differ; mismatched pixels: {mismatches}; first samples: {samples:?}");
    }

    #[test]
    fn empty_path_is_empty() {
        assert!(Path::new().is_empty());
    }

    #[test]
    fn move_to_and_line_to() {
        let mut p = Path::new();
        p.move_to(0.0, 0.0);
        p.line_to(100.0, 0.0);
        assert!(!p.is_empty());
        assert_eq!(p.segments.len(), 2);
        assert!(matches!(p.segments[0], PathSegment::MoveTo(0.0, 0.0)));
        assert!(matches!(p.segments[1], PathSegment::LineTo(100.0, 0.0)));
    }

    #[test]
    fn rect_adds_five_segments() {
        let mut p = Path::new();
        p.rect(10.0, 20.0, 50.0, 30.0);
        assert_eq!(p.segments.len(), 5);
        assert!(matches!(p.segments[0], PathSegment::MoveTo(..)));
        assert!(matches!(p.segments[4], PathSegment::ClosePath));
    }

    #[test]
    fn clear_resets_path() {
        let mut p = Path::new();
        p.rect(0.0, 0.0, 100.0, 100.0);
        p.clear();
        assert!(p.is_empty());
        assert!(p.current_point.is_none());
    }

    #[test]
    fn close_path_sets_current_point_to_subpath_start() {
        let mut p = Path::new();
        p.move_to(50.0, 50.0);
        p.line_to(100.0, 100.0);
        p.close();
        assert_eq!(p.current_point, Some((50.0, 50.0)));
    }

    #[test]
    fn straight_line_cubic_is_not_subdivided() {
        let mut out = Vec::new();
        flatten_cubic(
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (3.0, 0.0),
            0.5,
            16,
            &mut out,
        );
        assert_eq!(out, vec![(3.0, 0.0)]);
    }

    #[test]
    fn curved_bezier_is_subdivided() {
        let mut out = Vec::new();
        flatten_cubic(
            (0.0, 1.0),
            (0.552, 1.0),
            (1.0, 0.552),
            (1.0, 0.0),
            0.05,
            16,
            &mut out,
        );
        assert!(out.len() > 1);
        let last = out.last().copied().unwrap_or((0.0, 0.0));
        assert!((last.0 - 1.0).abs() < 0.01);
        assert!(last.1.abs() < 0.01);
    }

    #[test]
    fn cubic_monotonic_split_parameters_find_axis_extrema() {
        let p0 = (0.0, 0.0);
        let p1 = (10.0, 10.0);
        let p2 = (-10.0, 10.0);
        let p3 = (0.0, 0.0);
        let roots = cubic_monotonic_split_parameters(p0, p1, p2, p3);

        assert_eq!(roots.len(), 3);
        assert!((roots[0] - 0.211_324_865_405_187_1).abs() < 1e-9);
        assert!((roots[1] - 0.5).abs() < 1e-9);
        assert!((roots[2] - 0.788_675_134_594_812_9).abs() < 1e-9);
    }

    #[test]
    fn flatten_cubic_emits_monotonic_piece_boundaries() {
        fn cubic_point(
            p0: (f64, f64),
            p1: (f64, f64),
            p2: (f64, f64),
            p3: (f64, f64),
            t: f64,
        ) -> (f64, f64) {
            let mt = 1.0 - t;
            let a = mt * mt * mt;
            let b = 3.0 * mt * mt * t;
            let c = 3.0 * mt * t * t;
            let d = t * t * t;
            (
                a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
                a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
            )
        }

        let p0 = (0.0, 0.0);
        let p1 = (10.0, 10.0);
        let p2 = (-10.0, 10.0);
        let p3 = (0.0, 0.0);
        let roots = cubic_monotonic_split_parameters(p0, p1, p2, p3);
        let mut out = Vec::new();

        flatten_cubic(p0, p1, p2, p3, 10_000.0, 16, &mut out);

        assert_eq!(out.len(), roots.len() + 1);
        for (point, root) in out.iter().zip(roots.iter()) {
            let expected = cubic_point(p0, p1, p2, p3, *root);
            assert!((point.0 - expected.0).abs() < 1e-9);
            assert!((point.1 - expected.1).abs() < 1e-9);
        }
        assert_eq!(out.last().copied(), Some(p3));
    }

    #[test]
    fn split_cubic_pieces_have_no_internal_axis_extrema() {
        let p0 = (0.0, 0.0);
        let p1 = (10.0, 10.0);
        let p2 = (-10.0, 10.0);
        let p3 = (0.0, 0.0);
        let roots = cubic_monotonic_split_parameters(p0, p1, p2, p3);
        let mut curve = (p0, p1, p2, p3);
        let mut previous = 0.0;

        for root in roots {
            let local_t = (root - previous) / (1.0 - previous);
            let (left, right) = split_cubic(curve, local_t);
            assert!(cubic_monotonic_split_parameters(left.0, left.1, left.2, left.3).is_empty());
            curve = right;
            previous = root;
        }

        assert!(cubic_monotonic_split_parameters(curve.0, curve.1, curve.2, curve.3).is_empty());
    }

    #[test]
    fn point_to_line_dist_for_known_geometry() {
        let d = point_to_line_dist((0.0, 1.0), (0.0, 0.0), (1.0, 0.0));
        assert!((d - 1.0).abs() < 1e-10);
        let d2 = point_to_line_dist((0.5, 0.0), (0.0, 0.0), (1.0, 0.0));
        assert!(d2 < 1e-10);
    }

    #[test]
    fn scanline_crossings_prepare_sorted_monotonic_active_edge_row() {
        let mut crossings = vec![
            (5.0, 1),
            (f64::NAN, 1),
            (2.0, -1),
            (5.0, -1),
            (3.0, 0),
            (4.0, 1),
        ];

        assert!(prepare_scanline_crossings(&mut crossings));
        assert_eq!(crossings, vec![(2.0, -1), (4.0, 1), (5.0, -1), (5.0, 1)]);

        crossings.clear();
        crossings.extend([(f64::INFINITY, 1), (7.0, 0), (9.0, -1)]);
        assert!(!prepare_scanline_crossings(&mut crossings));
    }

    #[test]
    fn scanline_active_edges_match_bucket_crossings_across_rows() {
        let mut flat = FlatPath::default();
        flat.subpaths.push(vec![
            (2.25, 1.0),
            (12.0, 4.25),
            (9.5, 12.0),
            (1.0, 9.75),
            (2.25, 1.0),
        ]);
        flat.closed.push(true);
        let y0 = 0;
        let h = 14;
        let buckets = build_scanline_edge_buckets(&flat, y0, h).expect("edge buckets");
        let mut active_edges = ScanlineActiveEdges::default();

        for row in 0..h {
            let scan_y = y0 as f64 + row as f64 + 0.5;
            let mut from_bucket = Vec::new();
            collect_scanline_crossings_from_bucket(&buckets, row, scan_y, &mut from_bucket);
            let bucket_has_crossings = prepare_scanline_crossings(&mut from_bucket);

            let mut from_active = Vec::new();
            assert!(active_edges.collect_row(&buckets, row, scan_y, &mut from_active));
            let active_has_crossings = prepare_scanline_crossings(&mut from_active);

            assert_eq!(active_has_crossings, bucket_has_crossings);
            assert_eq!(from_active, from_bucket);
        }
    }

    #[test]
    fn scanline_active_edges_match_bucket_crossings_across_subsamples() {
        let mut flat = FlatPath::default();
        flat.subpaths.push(vec![
            (1.5, 0.25),
            (13.0, 3.75),
            (11.25, 11.5),
            (3.0, 13.25),
            (1.5, 0.25),
        ]);
        flat.closed.push(true);
        let y0 = 0;
        let h = 15;
        let vertical_samples = 4;
        let buckets = build_scanline_edge_buckets(&flat, y0, h).expect("edge buckets");
        let mut active_edges = ScanlineActiveEdges::default();

        for row in 0..h {
            assert!(active_edges.advance_to_row(&buckets, row));
            for sample in 0..vertical_samples {
                let scan_y =
                    y0 as f64 + row as f64 + (sample as f64 + 0.5) / vertical_samples as f64;
                let mut from_bucket = Vec::new();
                collect_scanline_crossings_from_bucket(&buckets, row, scan_y, &mut from_bucket);
                let bucket_has_crossings = prepare_scanline_crossings(&mut from_bucket);

                let mut from_active = Vec::new();
                active_edges.collect_current_crossings(&buckets, scan_y, &mut from_active);
                let active_has_crossings = prepare_scanline_crossings(&mut from_active);

                assert_eq!(active_has_crossings, bucket_has_crossings);
                assert_eq!(from_active, from_bucket);
            }
        }
    }

    #[test]
    fn scanline_active_edges_require_sequential_rows() {
        let mut flat = FlatPath::default();
        flat.subpaths.push(vec![(1.0, 0.0), (5.0, 6.0), (9.0, 0.0)]);
        flat.closed.push(false);
        let buckets = build_scanline_edge_buckets(&flat, 0, 8).expect("edge buckets");
        let mut active_edges = ScanlineActiveEdges::default();
        let mut crossings = Vec::new();

        assert!(!active_edges.collect_row(&buckets, 1, 1.5, &mut crossings));
        assert!(active_edges.collect_row(&buckets, 0, 0.5, &mut crossings));
    }

    #[test]
    fn scanline_active_edges_drop_ended_edges_before_later_rows() {
        let mut flat = FlatPath::default();
        flat.subpaths.push(vec![
            (2.0, 1.0),
            (10.0, 1.0),
            (10.0, 5.0),
            (2.0, 5.0),
            (2.0, 1.0),
        ]);
        flat.closed.push(true);
        let buckets = build_scanline_edge_buckets(&flat, 0, 8).expect("edge buckets");
        let mut active_edges = ScanlineActiveEdges::default();
        let mut crossings = Vec::new();

        assert!(active_edges.collect_row(&buckets, 0, 0.5, &mut crossings));
        assert!(crossings.is_empty());

        assert!(active_edges.collect_row(&buckets, 1, 1.5, &mut crossings));
        assert_eq!(active_edges.active.len(), 2);
        assert!(prepare_scanline_crossings(&mut crossings));
        assert_eq!(crossings, vec![(2.0, -1), (10.0, 1)]);

        for row in 2..=4 {
            crossings.clear();
            assert!(active_edges.collect_row(&buckets, row, row as f64 + 0.5, &mut crossings));
            assert_eq!(active_edges.active.len(), 2);
        }

        crossings.clear();
        assert!(active_edges.collect_row(&buckets, 5, 5.5, &mut crossings));
        assert!(crossings.is_empty());
        assert!(active_edges.active.is_empty());
    }

    #[test]
    fn scanline_scratch_pools_return_empty_bounded_vectors() {
        let mut crossings = take_scanline_crossing_vec();
        crossings.extend([(1.0, 1), (2.0, -1), (3.0, 1)]);
        let crossing_capacity = crossings.capacity();
        return_scanline_crossing_vec(crossings);

        let reused_crossings = take_scanline_crossing_vec();
        assert!(reused_crossings.is_empty());
        assert!(reused_crossings.capacity() >= crossing_capacity);
        return_scanline_crossing_vec(reused_crossings);

        let mut accum = take_scanline_u16_vec(4);
        accum.extend([1, 2, 3, 4]);
        let accum_capacity = accum.capacity();
        return_scanline_u16_vec(accum);

        let reused_accum = take_scanline_u16_vec(2);
        assert_eq!(reused_accum, vec![0, 0]);
        assert!(reused_accum.capacity() >= accum_capacity);
        return_scanline_u16_vec(reused_accum);
    }

    #[test]
    fn stroke_horizontal_line_produces_dark_pixels() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut path = Path::new();
        path.move_to(10.0, 50.0);
        path.line_to(90.0, 50.0);
        PathPainter::stroke(&mut buf, &path, &ctm, &vp, BLACK, 2.0, &DashState::solid());
        let mid = buf.get_pixel(50, 50);
        assert!(mid[0] < 200, "midpoint pixel should be dark: {mid:?}");
    }

    #[test]
    fn hairline_stroke_renders_as_one_pixel() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut path = Path::new();
        path.move_to(10.0, 50.0);
        path.line_to(90.0, 50.0);

        PathPainter::stroke(&mut buf, &path, &ctm, &vp, BLACK, 0.0, &DashState::solid());

        let mid = buf.get_pixel(50, 50);
        assert!(mid[0] < 200, "hairline stroke should be visible: {mid:?}");
    }

    #[test]
    fn axis_aligned_horizontal_hairline_fast_path_matches_outline_fill() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(10.0, 50.0);
        path.line_to(90.0, 50.0);

        let mut fast = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke(&mut fast, &path, &ctm, &vp, BLACK, 0.0, &DashState::solid());

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let outline = stroke_flat_path(
            &flat,
            1.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
        fill_flat_aa(&mut reference, &outline, BLACK, FillRule::NonZero);

        assert_pixel_buffers_eq(&fast, &reference);
    }

    #[test]
    fn axis_aligned_vertical_hairline_fast_path_matches_outline_fill() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(50.0, 10.0);
        path.line_to(50.0, 90.0);

        let mut fast = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke(&mut fast, &path, &ctm, &vp, BLACK, 0.0, &DashState::solid());

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let outline = stroke_flat_path(
            &flat,
            1.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
        fill_flat_aa(&mut reference, &outline, BLACK, FillRule::NonZero);

        assert_pixel_buffers_eq(&fast, &reference);
    }

    #[test]
    fn translucent_axis_aligned_hairline_fast_path_matches_outline_fill() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let color = [0, 0, 0, 128];
        let mut path = Path::new();
        path.move_to(10.0, 50.0);
        path.line_to(90.0, 50.0);

        let mut fast = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke(&mut fast, &path, &ctm, &vp, color, 0.0, &DashState::solid());

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let outline = stroke_flat_path(
            &flat,
            1.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
        fill_flat_aa(&mut reference, &outline, color, FillRule::NonZero);

        assert_pixel_buffers_eq(&fast, &reference);
    }

    #[test]
    fn axis_aligned_polyline_hairline_fast_path_matches_outline_fill() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(10.0, 50.0);
        path.line_to(35.0, 50.0);
        path.line_to(90.0, 50.0);

        let mut fast = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke(&mut fast, &path, &ctm, &vp, BLACK, 0.0, &DashState::solid());

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let outline = stroke_flat_path(
            &flat,
            1.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
        fill_flat_aa(&mut reference, &outline, BLACK, FillRule::NonZero);

        assert_pixel_buffers_eq(&fast, &reference);
    }

    #[test]
    fn axis_aligned_hairline_fast_path_rejects_semantic_cases() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(10.0, 50.0);
        path.line_to(90.0, 50.0);

        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        assert!(!paint_axis_aligned_solid_hairline(
            &mut buf,
            &path,
            &ctm,
            &vp,
            BLACK,
            0.0,
            &DashState::new(vec![2.0, 1.0], 0.0),
            &LineCap::Butt,
        ));
        assert!(!paint_axis_aligned_solid_hairline(
            &mut buf,
            &path,
            &ctm,
            &vp,
            BLACK,
            0.0,
            &DashState::solid(),
            &LineCap::Round,
        ));
        assert!(!paint_axis_aligned_solid_hairline(
            &mut buf,
            &path,
            &ctm,
            &vp,
            BLACK,
            1.0,
            &DashState::solid(),
            &LineCap::Butt,
        ));

        path.clear();
        path.move_to(10.25, 50.0);
        path.line_to(90.0, 50.0);
        assert!(!paint_axis_aligned_solid_hairline(
            &mut buf,
            &path,
            &ctm,
            &vp,
            BLACK,
            0.0,
            &DashState::solid(),
            &LineCap::Butt,
        ));

        path.clear();
        path.move_to(10.0, 50.0);
        path.line_to(35.0, 50.0);
        path.line_to(20.0, 50.0);
        assert!(!paint_axis_aligned_solid_hairline(
            &mut buf,
            &path,
            &ctm,
            &vp,
            BLACK,
            0.0,
            &DashState::solid(),
            &LineCap::Butt,
        ));

        path.clear();
        path.move_to(10.0, 50.0);
        path.line_to(35.0, 50.0);
        path.line_to(35.0, 70.0);
        assert!(!paint_axis_aligned_solid_hairline(
            &mut buf,
            &path,
            &ctm,
            &vp,
            BLACK,
            0.0,
            &DashState::solid(),
            &LineCap::Butt,
        ));
    }

    #[test]
    fn axis_aligned_solid_stroke_rect_detects_integer_device_spans() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut horizontal = Path::new();
        horizontal.move_to(10.0, 50.0);
        horizontal.line_to(90.0, 50.0);
        assert_eq!(
            axis_aligned_solid_stroke_rect(
                &horizontal,
                &ctm,
                &vp,
                2.0,
                &DashState::solid(),
                &LineCap::Butt,
            ),
            Some((10, 49, 80, 2))
        );

        let mut vertical = Path::new();
        vertical.move_to(50.0, 10.0);
        vertical.line_to(50.0, 90.0);
        assert_eq!(
            axis_aligned_solid_stroke_rect(
                &vertical,
                &ctm,
                &vp,
                2.0,
                &DashState::solid(),
                &LineCap::Butt,
            ),
            Some((49, 10, 2, 80))
        );
    }

    #[test]
    fn axis_aligned_projecting_square_stroke_rect_extends_integer_device_spans() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut horizontal = Path::new();
        horizontal.move_to(10.0, 50.0);
        horizontal.line_to(90.0, 50.0);
        assert_eq!(
            axis_aligned_solid_stroke_rect(
                &horizontal,
                &ctm,
                &vp,
                2.0,
                &DashState::solid(),
                &LineCap::ProjectingSquare,
            ),
            Some((9, 49, 82, 2))
        );

        let mut vertical = Path::new();
        vertical.move_to(50.0, 10.0);
        vertical.line_to(50.0, 90.0);
        assert_eq!(
            axis_aligned_solid_stroke_rect(
                &vertical,
                &ctm,
                &vp,
                2.0,
                &DashState::solid(),
                &LineCap::ProjectingSquare,
            ),
            Some((49, 9, 2, 82))
        );
    }

    #[test]
    fn axis_aligned_solid_stroke_rect_detects_monotonic_collinear_polyline() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut horizontal = Path::new();
        horizontal.move_to(10.0, 50.0);
        horizontal.line_to(35.0, 50.0);
        horizontal.line_to(90.0, 50.0);
        assert_eq!(
            axis_aligned_solid_stroke_rect(
                &horizontal,
                &ctm,
                &vp,
                2.0,
                &DashState::solid(),
                &LineCap::Butt,
            ),
            Some((10, 49, 80, 2))
        );

        let mut vertical = Path::new();
        vertical.move_to(50.0, 10.0);
        vertical.line_to(50.0, 45.0);
        vertical.line_to(50.0, 90.0);
        assert_eq!(
            axis_aligned_solid_stroke_rect(
                &vertical,
                &ctm,
                &vp,
                2.0,
                &DashState::solid(),
                &LineCap::ProjectingSquare,
            ),
            Some((49, 9, 2, 82))
        );
    }

    #[test]
    fn axis_aligned_solid_stroke_rect_rejects_fractional_or_semantic_cases() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(10.0, 50.25);
        path.line_to(90.0, 50.25);
        assert!(axis_aligned_solid_stroke_rect(
            &path,
            &ctm,
            &vp,
            2.0,
            &DashState::solid(),
            &LineCap::Butt,
        )
        .is_none());

        path.clear();
        path.move_to(10.0, 50.0);
        path.line_to(90.0, 50.0);
        assert!(axis_aligned_solid_stroke_rect(
            &path,
            &ctm,
            &vp,
            2.0,
            &DashState::solid(),
            &LineCap::ProjectingSquare,
        )
        .is_some());
        assert!(axis_aligned_solid_stroke_rect(
            &path,
            &ctm,
            &vp,
            2.0,
            &DashState::new(vec![2.0, 1.0], 0.0),
            &LineCap::Butt,
        )
        .is_none());
        assert!(axis_aligned_solid_stroke_rect(
            &path,
            &ctm,
            &vp,
            2.0,
            &DashState::solid(),
            &LineCap::Round,
        )
        .is_none());

        path.clear();
        path.move_to(10.0, 50.0);
        path.line_to(35.0, 50.0);
        path.line_to(20.0, 50.0);
        assert!(axis_aligned_solid_stroke_rect(
            &path,
            &ctm,
            &vp,
            2.0,
            &DashState::solid(),
            &LineCap::Butt,
        )
        .is_none());

        path.clear();
        path.move_to(10.0, 50.0);
        path.line_to(35.0, 50.0);
        path.line_to(35.0, 70.0);
        assert!(axis_aligned_solid_stroke_rect(
            &path,
            &ctm,
            &vp,
            2.0,
            &DashState::solid(),
            &LineCap::Butt,
        )
        .is_none());
    }

    #[test]
    fn integer_axis_aligned_butt_stroke_fast_path_matches_outline_fill() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(10.0, 50.0);
        path.line_to(90.0, 50.0);

        let mut fast = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke(&mut fast, &path, &ctm, &vp, BLACK, 2.0, &DashState::solid());

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let outline = stroke_flat_path(
            &flat,
            2.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
        fill_flat_aa(&mut reference, &outline, BLACK, FillRule::NonZero);

        assert_pixel_buffers_eq(&fast, &reference);
    }

    #[test]
    fn integer_axis_aligned_projecting_square_stroke_fast_path_matches_outline_fill() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(10.0, 50.0);
        path.line_to(90.0, 50.0);

        let mut fast = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke_with_cap(
            &mut fast,
            &path,
            &ctm,
            &vp,
            BLACK,
            2.0,
            &DashState::solid(),
            &LineCap::ProjectingSquare,
        );

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let outline = stroke_flat_path(
            &flat,
            2.0,
            &DashState::solid(),
            LineCap::ProjectingSquare,
            LineJoin::Miter,
            10.0,
        );
        fill_flat_aa(&mut reference, &outline, BLACK, FillRule::NonZero);

        assert_pixel_buffers_eq(&fast, &reference);
    }

    #[test]
    fn integer_axis_aligned_polyline_stroke_fast_path_matches_outline_fill() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(10.0, 50.0);
        path.line_to(35.0, 50.0);
        path.line_to(90.0, 50.0);

        let mut fast = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke(&mut fast, &path, &ctm, &vp, BLACK, 2.0, &DashState::solid());

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let outline = stroke_flat_path(
            &flat,
            2.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
        fill_flat_aa(&mut reference, &outline, BLACK, FillRule::NonZero);

        assert_pixel_buffers_eq(&fast, &reference);
    }

    #[test]
    fn fill_rectangle_produces_filled_region() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut path = Path::new();
        path.rect(20.0, 20.0, 60.0, 60.0);
        PathPainter::fill(&mut buf, &path, &ctm, &vp, RED, FillRule::NonZero);
        let center = buf.get_pixel(50, 50);
        println!("nonzero fill center: {:?}", center);
        assert_eq!(center, RED);
        assert_eq!(buf.get_pixel(5, 5), WHITE);
    }

    #[test]
    fn integer_axis_aligned_rect_uses_span_fill_fast_path() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut path = Path::new();
        path.rect(10.0, 10.0, 20.0, 15.0);

        let rect = axis_aligned_integer_rect(&path, &ctm, &vp).expect("integer device rect");
        assert_eq!(rect, (10, 75, 20, 15));

        PathPainter::fill(&mut buf, &path, &ctm, &vp, BLUE, FillRule::EvenOdd);
        assert_eq!(buf.get_pixel(10, 75), BLUE);
        assert_eq!(buf.get_pixel(29, 89), BLUE);
        assert_eq!(buf.get_pixel(30, 89), WHITE);
    }

    #[test]
    fn decimal_matrix_noise_keeps_integer_rect_fast_path() {
        let vp = Viewport::new([0.0, 0.0, 720.0, 405.0], 72);
        let ctm = Transform2D::new(0.001968504, 0.0, 0.0, 0.001968504, 0.0, 0.0);
        let mut path = Path::new();
        path.rect(0.0, 0.0, 365_760.0, 205_740.0);

        assert_eq!(
            axis_aligned_integer_rect(&path, &ctm, &vp),
            Some((0, 0, 720, 405))
        );
    }

    #[test]
    fn axis_aligned_solid_rect_stroke_regions_detects_integer_miter_border() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.rect(20.0, 20.0, 60.0, 60.0);

        assert_eq!(
            axis_aligned_solid_rect_stroke_regions(
                &path,
                &ctm,
                &vp,
                2.0,
                &DashState::solid(),
                &LineJoin::Miter,
                10.0,
            ),
            Some([
                (19, 19, 62, 2),
                (19, 79, 62, 2),
                (19, 21, 2, 58),
                (79, 21, 2, 58)
            ])
        );
    }

    #[test]
    fn axis_aligned_solid_rect_stroke_regions_rejects_non_exact_or_semantic_cases() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.rect(20.25, 20.0, 60.0, 60.0);
        assert!(axis_aligned_solid_rect_stroke_regions(
            &path,
            &ctm,
            &vp,
            2.0,
            &DashState::solid(),
            &LineJoin::Miter,
            10.0,
        )
        .is_none());

        path.clear();
        path.rect(20.0, 20.0, 60.0, 60.0);
        assert!(axis_aligned_solid_rect_stroke_regions(
            &path,
            &ctm,
            &vp,
            2.0,
            &DashState::new(vec![2.0, 1.0], 0.0),
            &LineJoin::Miter,
            10.0,
        )
        .is_none());
        assert!(axis_aligned_solid_rect_stroke_regions(
            &path,
            &ctm,
            &vp,
            2.0,
            &DashState::solid(),
            &LineJoin::Bevel,
            10.0,
        )
        .is_none());
        assert!(axis_aligned_solid_rect_stroke_regions(
            &path,
            &ctm,
            &vp,
            70.0,
            &DashState::solid(),
            &LineJoin::Miter,
            10.0,
        )
        .is_none());
    }

    #[test]
    fn integer_axis_aligned_rect_stroke_fast_path_matches_outline_fill() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let color = BLACK;
        let mut path = Path::new();
        path.rect(20.0, 20.0, 60.0, 60.0);

        let mut fast = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke_rect(&mut fast, 20.0, 20.0, 60.0, 60.0, &ctm, &vp, color, 2.0);

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let outline = stroke_flat_path(
            &flat,
            2.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
        fill_flat_aa(&mut reference, &outline, color, FillRule::NonZero);

        assert_pixel_buffers_eq(&fast, &reference);
    }

    #[test]
    fn translucent_axis_aligned_rect_stroke_keeps_outline_blending_path() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let color = [0, 0, 0, 128];
        let mut path = Path::new();
        path.rect(20.0, 20.0, 60.0, 60.0);

        let mut rendered = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke_rect(&mut rendered, 20.0, 20.0, 60.0, 60.0, &ctm, &vp, color, 2.0);

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let outline = stroke_flat_path(
            &flat,
            2.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
        fill_flat_aa(&mut reference, &outline, color, FillRule::NonZero);

        assert_pixel_buffers_eq(&rendered, &reference);
    }

    #[test]
    fn fractional_rect_keeps_analytic_coverage_path() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.rect(10.25, 10.0, 20.0, 15.0);
        assert!(axis_aligned_integer_rect(&path, &ctm, &vp).is_none());
    }

    #[test]
    fn aa_color_compositor_matches_generic_pixel_compositor() {
        let mut flat = FlatPath::default();
        flat.subpaths
            .push(vec![(2.25, 2.0), (14.75, 4.25), (8.5, 13.75), (2.25, 2.0)]);
        flat.closed.push(true);
        let color = [15, 120, 240, 180];
        let mut fast = PixelBuffer::new_filled(20, 20, WHITE);
        let mut reference = PixelBuffer::new_filled(20, 20, WHITE);

        assert!(fill_flat_color(
            &mut fast,
            &flat,
            color,
            FillRule::NonZero,
            None,
        ));
        assert!(fill_flat_with_compositor(
            &mut reference,
            &flat,
            FillRule::NonZero,
            None,
            |buf, x, y, coverage| {
                buf.blend_pixel(x, y, color, coverage);
            },
        ));

        assert_eq!(fast.rgba_bytes(), reference.rgba_bytes());
    }

    #[test]
    fn fill_rect_fast_path_fills_correct_pixels() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::fill_rect(&mut buf, 10.0, 10.0, 80.0, 80.0, &ctm, &vp, BLUE);
        assert_eq!(buf.get_pixel(50, 50), BLUE);
        assert_eq!(buf.get_pixel(5, 5), WHITE);
    }

    #[test]
    fn evenodd_and_nonzero_fill_both_paint_pixels() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf_eo = PixelBuffer::new_filled(100, 100, WHITE);
        let mut buf_nz = PixelBuffer::new_filled(100, 100, WHITE);
        let mut path = Path::new();
        path.rect(10.0, 10.0, 50.0, 50.0);
        path.rect(30.0, 30.0, 50.0, 50.0);
        PathPainter::fill(&mut buf_eo, &path, &ctm, &vp, RED, FillRule::EvenOdd);
        PathPainter::fill(&mut buf_nz, &path, &ctm, &vp, RED, FillRule::NonZero);
        let eo_has_red = (0..100i32)
            .flat_map(|y| (0..100i32).map(move |x| (x, y)))
            .any(|(x, y)| buf_eo.get_pixel(x, y) == RED);
        let nz_has_red = (0..100i32)
            .flat_map(|y| (0..100i32).map(move |x| (x, y)))
            .any(|(x, y)| buf_nz.get_pixel(x, y) == RED);
        println!(
            "overlap eo={:?} nz={:?}",
            buf_eo.get_pixel(40, 50),
            buf_nz.get_pixel(40, 50)
        );
        assert!(eo_has_red);
        assert!(nz_has_red);
        assert_eq!(buf_eo.get_pixel(40, 50), WHITE);
        assert_eq!(buf_nz.get_pixel(40, 50), RED);
    }

    #[test]
    fn convex_polygon_fill_fast_path_matches_general_scanline() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let color = [20, 80, 220, 230];
        let mut path = Path::new();
        path.move_to(20.25, 20.25);
        path.line_to(75.5, 24.5);
        path.line_to(68.75, 72.25);
        path.line_to(25.25, 80.5);
        path.close();
        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        let Some((min_x, min_y, max_x, max_y)) = flat_bounds(&flat) else {
            panic!("convex fixture should have finite bounds");
        };
        let bounds = (
            safe_floor_i32(min_x).max(0),
            safe_floor_i32(min_y).max(0),
            safe_ceil_i32(max_x).saturating_add(1).min(100),
            safe_ceil_i32(max_y).saturating_add(1).min(100),
        );

        let mut reference = PixelBuffer::new_filled(100, 100, WHITE);
        assert!(fill_flat_with_color_compositor_scanline(
            &mut reference,
            &flat,
            color,
            FillRule::NonZero,
            None,
            bounds,
            PATH_SCANLINE_COLOR_VERTICAL_SAMPLES,
            &[],
            None,
        ));

        let before = path_raster_stats().convex_fast_pixels;
        let mut fast = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::fill(&mut fast, &path, &ctm, &vp, color, FillRule::NonZero);
        let after = path_raster_stats().convex_fast_pixels;

        assert_pixel_buffers_eq(&fast, &reference);
        assert!(after > before);
    }

    #[test]
    fn thin_subpath_between_sample_rows_is_rescued_in_streaming_and_mask_fills() {
        // Device-space sliver between y = 10.40 and 10.50: no 4x sample row
        // (.375 / .625) falls inside it.
        let flat = FlatPath {
            subpaths: vec![vec![
                (2.0, 10.4),
                (8.0, 10.4),
                (14.0, 10.4),
                (14.0, 10.5),
                (8.0, 10.5),
                (2.0, 10.5),
            ]],
            closed: vec![false],
        };
        let mut buf = PixelBuffer::new_filled(16, 16, WHITE);
        assert!(fill_flat_color(
            &mut buf,
            &flat,
            BLACK,
            FillRule::NonZero,
            None
        ));
        let value = buf.get_pixel(8, 10)[0];
        assert!(value < 255 && value > 200, "streaming rescue: {value}");
        assert_eq!(buf.get_pixel(8, 9), WHITE);

        let mask = rasterize_flat_alpha_mask(&flat, FillRule::NonZero).expect("mask");
        let row = (10 - mask.y) as usize;
        let col = (8 - mask.x) as usize;
        let alpha = mask.alpha_slice()[row * mask.width as usize + col];
        assert!(alpha > 0 && alpha < 64, "mask rescue: {alpha}");
    }

    #[test]
    fn thick_geometry_is_unaffected_by_thin_rescue() {
        assert!(thin_subpath_rescues(
            &FlatPath {
                subpaths: vec![vec![(0.0, 10.1), (4.0, 10.1), (4.0, 10.3), (0.0, 10.3)]],
                closed: vec![true],
            },
            4,
        )
        .is_empty());
        let rescues = thin_subpath_rescues(
            &FlatPath {
                subpaths: vec![vec![(0.0, 10.4), (4.0, 10.4), (4.0, 10.45), (0.0, 10.45)]],
                closed: vec![true],
            },
            4,
        );
        assert_eq!(rescues.len(), 1);
        assert_eq!(rescues[0].row, 10);
        assert!((rescues[0].weight - 1.0 / 16.0).abs() < 1e-12);
    }

    #[test]
    fn flattening_tolerance_is_clamped_to_one_device_pixel() {
        assert_eq!(effective_device_flatness(100.0), 1.0);
        assert_eq!(effective_device_flatness(0.2), 0.2);
        assert_eq!(effective_device_flatness(f64::NAN), 0.5);
        let mut path = Path::new();
        path.move_to(0.0, 0.0);
        path.curve_to(0.0, 55.0, 45.0, 100.0, 100.0, 100.0);
        let flat = flatten_path(
            &path,
            &Transform2D::identity(),
            &Viewport::new([0.0, 0.0, 200.0, 200.0], 72),
            100.0,
        );
        assert!(
            flat.subpaths[0].len() > 8,
            "coarse flatness must not facet curves"
        );
    }

    #[test]
    fn concave_polygon_fill_rejects_convex_fast_path() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(20.0, 20.0);
        path.line_to(80.0, 20.0);
        path.line_to(50.0, 50.0);
        path.line_to(80.0, 80.0);
        path.line_to(20.0, 80.0);
        path.close();

        let flat = flatten_path(&path, &ctm, &vp, 0.2);
        assert!(convex_flat_points(&flat).is_none());
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::fill(&mut buf, &path, &ctm, &vp, BLUE, FillRule::NonZero);

        assert_eq!(buf.get_pixel(35, 35), BLUE);
    }

    #[test]
    fn empty_path_does_not_panic() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        let path = Path::new();
        PathPainter::stroke(&mut buf, &path, &ctm, &vp, BLACK, 1.0, &DashState::solid());
        PathPainter::fill(&mut buf, &path, &ctm, &vp, RED, FillRule::NonZero);
        assert_eq!(buf.get_pixel(5, 5), WHITE);
    }

    #[test]
    fn curve_to_adds_cubic_segment() {
        let mut p = Path::new();
        p.move_to(0.0, 0.0);
        p.curve_to(10.0, 20.0, 30.0, 20.0, 40.0, 0.0);
        assert_eq!(p.segments.len(), 2);
        assert!(matches!(
            p.segments[1],
            PathSegment::CubicTo {
                cp1x,
                cp2x,
                x,
                ..
            } if cp1x == 10.0 && cp2x == 30.0 && x == 40.0
        ));
    }

    #[test]
    fn line_to_without_move_to_creates_implicit_move() {
        let mut p = Path::new();
        p.line_to(50.0, 50.0);
        assert!(!p.is_empty());
        assert!(p.current_point.is_some());
    }

    #[test]
    fn multiple_subpaths_in_one_path() {
        let mut p = Path::new();
        p.move_to(0.0, 0.0);
        p.line_to(50.0, 0.0);
        p.move_to(0.0, 50.0);
        p.line_to(50.0, 50.0);
        assert_eq!(p.segments.len(), 4);
    }

    #[test]
    fn flatten_path_with_pure_line_segments() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut p = Path::new();
        p.move_to(0.0, 0.0);
        p.line_to(50.0, 0.0);
        p.line_to(50.0, 50.0);
        let flat = flatten_path(&p, &ctm, &vp, 0.5);
        assert_eq!(flat.subpaths.len(), 1);
        assert_eq!(flat.subpaths[0].len(), 3);
        assert!(!flat.closed[0]);
    }

    #[test]
    fn flatten_path_closed_rect_has_closed_subpath() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut p = Path::new();
        p.rect(0.0, 0.0, 50.0, 50.0);
        let flat = flatten_path(&p, &ctm, &vp, 0.5);
        assert_eq!(flat.subpaths.len(), 1);
        assert_eq!(flat.subpaths[0].len(), 5);
        assert!(flat.closed[0]);
    }

    #[test]
    fn flatten_path_with_ctm_scale_doubles_pixel_coords() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::scale(2.0, 2.0);
        let mut p = Path::new();
        p.move_to(10.0, 10.0);
        p.line_to(20.0, 10.0);
        let flat = flatten_path(&p, &ctm, &vp, 0.5);
        let (px0, _) = flat.subpaths[0][0];
        let (px1, _) = flat.subpaths[0][1];
        assert!((px0 - 20.0).abs() < 1.0);
        assert!(((px1 - px0).abs() - 20.0).abs() < 1.0);
    }

    #[test]
    fn flatten_path_filters_nonfinite_transformed_points() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut p = Path::new();
        p.move_to(10.0, 10.0);
        p.line_to(20.0, 10.0);
        p.line_to(f64::NAN, 20.0);
        p.curve_to(30.0, 20.0, f64::INFINITY, 40.0, 50.0, 50.0);
        p.line_to(60.0, 60.0);

        let flat = flatten_path(&p, &ctm, &vp, 0.5);

        assert!(!flat.subpaths.is_empty());
        assert!(flat
            .subpaths
            .iter()
            .flatten()
            .all(|(x, y)| x.is_finite() && y.is_finite()));
    }

    #[test]
    fn flatten_path_device_transform_filters_nonfinite_points() {
        let ctm = Transform2D::identity();
        let mut p = Path::new();
        p.move_to(10.0, 10.0);
        p.line_to(f64::INFINITY, 20.0);
        p.line_to(30.0, 30.0);
        p.curve_to(40.0, 40.0, 50.0, f64::NAN, 60.0, 60.0);
        p.line_to(70.0, 70.0);

        let flat = flatten_path_device_transform(&p, &ctm, 0.2);

        assert!(!flat.subpaths.is_empty());
        assert!(flat
            .subpaths
            .iter()
            .flatten()
            .all(|(x, y)| x.is_finite() && y.is_finite()));
    }

    #[test]
    fn stroke_does_not_modify_far_corners() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut p = Path::new();
        p.move_to(40.0, 40.0);
        p.line_to(60.0, 40.0);
        PathPainter::stroke(&mut buf, &p, &ctm, &vp, BLACK, 1.0, &DashState::solid());
        assert_eq!(buf.get_pixel(0, 0), WHITE);
        assert_eq!(buf.get_pixel(99, 99), WHITE);
    }

    #[test]
    fn stroke_outline_keeps_dash_state_across_segments() {
        let points = vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)];
        let pieces = dash_polyline(&points, &DashState::new(vec![12.0, 100.0], 0.0));
        assert_eq!(pieces.len(), 1);
        let last = pieces[0].last().copied().unwrap();
        assert!(
            (last.0 - 12.0).abs() < 1e-8 && last.1.abs() < 1e-8,
            "dash should continue across segment boundary, got {last:?}"
        );
    }

    #[test]
    fn dense_dash_polyline_falls_back_before_expansion() {
        let points: Vec<(f64, f64)> = (0..256).map(|idx| (f64::from(idx), 0.0)).collect();
        let pieces = dash_polyline(&points, &DashState::new(vec![0.001, 0.001], 0.0));
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].len(), points.len());
    }

    #[test]
    fn stroke_outline_miter_and_bevel_join_geometry_differs() {
        let flat = FlatPath {
            subpaths: vec![vec![(10.0, 50.0), (50.0, 50.0), (50.0, 10.0)]],
            closed: vec![false],
        };
        let miter = stroke_flat_path(
            &flat,
            10.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Miter,
            10.0,
        );
        let bevel = stroke_flat_path(
            &flat,
            10.0,
            &DashState::solid(),
            LineCap::Butt,
            LineJoin::Bevel,
            10.0,
        );

        let miter_points = &miter.subpaths[0];
        let bevel_points = &bevel.subpaths[0];
        assert!(
            miter_points
                .iter()
                .any(|&p| distance(p, (55.0, 55.0)) < 1e-8),
            "miter join should include the offset-line intersection"
        );
        assert!(
            !bevel_points
                .iter()
                .any(|&p| distance(p, (55.0, 55.0)) < 1e-8),
            "bevel join should use edge endpoints instead of the miter point"
        );
    }

    #[test]
    fn analytic_stroke_edge_has_fractional_coverage() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut p = Path::new();
        p.move_to(10.0, 49.75);
        p.line_to(90.0, 49.75);

        PathPainter::stroke(&mut buf, &p, &ctm, &vp, BLACK, 2.0, &DashState::solid());

        let center = buf.get_pixel(50, 50);
        let edge = buf.get_pixel(50, 49);
        assert!(center[0] < 20, "stroke center should be solid: {center:?}");
        assert!(
            edge[0] > 20 && edge[0] < 230,
            "fractional stroke edge should be antialiased gray: {edge:?}"
        );
    }

    #[test]
    fn fill_rect_with_identity_ctm_uses_fast_path() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::fill_rect(&mut buf, 25.0, 25.0, 50.0, 50.0, &ctm, &vp, GREEN);
        assert_eq!(buf.get_pixel(50, 50), GREEN);
        assert_eq!(buf.get_pixel(20, 20), WHITE);
    }

    #[test]
    fn stroke_rect_produces_border_pixels() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        PathPainter::stroke_rect(&mut buf, 20.0, 20.0, 60.0, 60.0, &ctm, &vp, BLACK, 1.0);
        let top_edge = buf.get_pixel(50, 20);
        assert!(top_edge[0] < 200, "top edge should be dark: {top_edge:?}");
    }

    #[test]
    fn aa_fill_interior_is_fully_covered() {
        // A rect aligned to integer pixel boundaries: interior cells get 100%
        // coverage (exact colour), proving AA does not erode solid fills.
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut path = Path::new();
        path.rect(20.0, 20.0, 40.0, 40.0);
        PathPainter::fill(&mut buf, &path, &ctm, &vp, RED, FillRule::NonZero);
        // Center is solid red.
        assert_eq!(buf.get_pixel(40, 50), RED);
    }

    #[test]
    fn aa_fill_partial_edge_produces_intermediate_coverage() {
        // A rectangle whose right edge falls on a half-pixel (x = 30.5) must
        // produce a partially-covered column (not a hard on/off jump). With the
        // old non-AA fill that boundary column was either full or empty.
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut path = Path::new();
        // y in PDF space flips; use a tall rect with a fractional right edge.
        path.move_to(10.0, 10.0);
        path.line_to(30.5, 10.0);
        path.line_to(30.5, 90.0);
        path.line_to(10.0, 90.0);
        path.close();
        PathPainter::fill(&mut buf, &path, &ctm, &vp, BLACK, FillRule::NonZero);
        // Column 30 straddles the x=30.5 edge -> ~50% black over white -> gray.
        let edge = buf.get_pixel(30, 50);
        assert!(
            edge[0] > 60 && edge[0] < 210,
            "fractional edge column should be antialiased gray, got {edge:?}"
        );
        // Well inside is solid black; well outside is white.
        assert!(
            buf.get_pixel(20, 50)[0] < 30,
            "interior should be near-black"
        );
        assert_eq!(buf.get_pixel(40, 50), WHITE, "outside the rect stays white");
    }

    #[test]
    fn aa_fill_triangle_has_smooth_diagonal_edge() {
        // A diagonal edge should show a gradient of partial coverage along it,
        // not a staircase of full/empty pixels.
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut path = Path::new();
        path.move_to(10.0, 10.0);
        path.line_to(90.0, 10.0);
        path.line_to(10.0, 90.0);
        path.close();
        PathPainter::fill(&mut buf, &path, &ctm, &vp, BLACK, FillRule::NonZero);
        // Sample the diagonal region: count pixels with intermediate (AA) values.
        let mut partial = 0;
        for y in 0..100i32 {
            for x in 0..100i32 {
                let v = buf.get_pixel(x, y)[0];
                if v > 20 && v < 235 {
                    partial += 1;
                }
            }
        }
        assert!(
            partial > 30,
            "diagonal edge should yield many antialiased pixels, got {partial}"
        );
    }

    #[test]
    fn glyph_fill_partial_edge_produces_intermediate_coverage() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut path = Path::new();
        path.move_to(10.0, 10.0);
        path.line_to(30.5, 10.0);
        path.line_to(30.5, 90.0);
        path.line_to(10.0, 90.0);
        path.close();

        PathPainter::fill_glyph(
            &mut buf,
            &path,
            &ctm,
            &vp,
            BLACK,
            FillRule::NonZero,
            GlyphHinting::disabled(),
        );

        let edge = buf.get_pixel(30, 50);
        assert!(
            edge[0] > 60 && edge[0] < 210,
            "glyph fractional edge column should be gray, got {edge:?}"
        );
        assert!(buf.get_pixel(20, 50)[0] < 30);
        assert_eq!(buf.get_pixel(40, 50), WHITE);
    }

    #[test]
    fn glyph_grayscale_default_uses_compat_srgb_coverage_weight() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut compat = PixelBuffer::new_filled(100, 100, WHITE);
        let mut high = PixelBuffer::new_filled_with_mode(100, 100, WHITE, RenderMode::HighQuality);
        let mut path = Path::new();
        path.move_to(10.5, 10.0);
        path.line_to(20.5, 10.0);
        path.line_to(20.5, 90.0);
        path.line_to(10.5, 90.0);
        path.close();

        PathPainter::fill_glyph(
            &mut compat,
            &path,
            &ctm,
            &vp,
            BLACK,
            FillRule::NonZero,
            GlyphHinting::disabled(),
        );
        PathPainter::fill_glyph(
            &mut high,
            &path,
            &ctm,
            &vp,
            BLACK,
            FillRule::NonZero,
            GlyphHinting::disabled(),
        );

        let compat_edge = compat.get_pixel(10, 50)[0];
        let high_edge = high.get_pixel(10, 50)[0];
        assert!(
            (110..=145).contains(&compat_edge),
            "Compat grayscale text uses Poppler-style sRGB coverage, got {compat_edge}"
        );
        assert!(
            high_edge > compat_edge + 30,
            "HighQuality remains an opt-in linear-light path, compat={compat_edge}, high={high_edge}"
        );
        assert!(
            compat.get_pixel(15, 50)[0] < 20,
            "stem interior should remain solid"
        );
    }

    #[test]
    fn glyph_curve_flattening_uses_tighter_tolerance() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(10.0, 10.0);
        path.curve_to(10.0, 90.0, 90.0, 90.0, 90.0, 10.0);

        let loose = flatten_path(&path, &ctm, &vp, 0.5);
        let tight = flatten_path(&path, &ctm, &vp, 0.2);
        assert!(
            tight.subpaths[0].len() > loose.subpaths[0].len(),
            "0.2px glyph tolerance should keep more curve samples than 0.5px"
        );
    }

    #[test]
    fn path_flattening_respects_supplied_flatness_tolerance() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut path = Path::new();
        path.move_to(10.0, 10.0);
        path.curve_to(10.0, 90.0, 90.0, 90.0, 90.0, 10.0);

        let loose = flatten_path(&path, &ctm, &vp, 10.0);
        let tight = flatten_path(&path, &ctm, &vp, 0.1);
        assert!(
            tight.subpaths[0].len() > loose.subpaths[0].len(),
            "lower flatness tolerance should keep more curve samples"
        );
    }

    #[test]
    fn light_grid_fit_snaps_small_axis_aligned_baseline() {
        let mut flat = FlatPath {
            subpaths: vec![vec![
                (10.28, 5.12),
                (12.74, 5.12),
                (12.74, 20.12),
                (10.28, 20.12),
                (10.28, 5.12),
            ]],
            closed: vec![true],
        };

        light_grid_fit_flat_glyph(&mut flat, &Transform2D::translation(0.0, 0.28));

        let sp = &flat.subpaths[0];
        assert!((sp[0].0 - 10.28).abs() < 1e-10);
        assert!((sp[0].1 - 5.0).abs() < 1e-10);
        assert!((sp[2].0 - 12.74).abs() < 1e-10);
        assert!((sp[2].1 - 20.0).abs() < 1e-10);
    }

    #[test]
    fn light_grid_fit_snaps_near_pixel_stem_edges() {
        let mut flat = FlatPath {
            subpaths: vec![vec![(10.12, 9.9), (20.27, 10.22)]],
            closed: vec![false],
        };

        light_grid_fit_flat_glyph(&mut flat, &Transform2D::identity());

        assert!((flat.subpaths[0][0].0 - 10.0).abs() < 1e-10);
        assert!(
            (flat.subpaths[0][1].0 - 20.27).abs() < 1e-10,
            "points outside the bounded stem snap threshold must remain unchanged"
        );
    }

    #[test]
    fn glyph_hinting_large_display_text_is_disabled() {
        assert!(!GlyphHinting::light(48.0).should_apply());
        assert!(!GlyphHinting::light(6.5).should_apply());
        assert!(GlyphHinting::light(18.0).should_apply());
    }

    #[test]
    fn stroke_with_round_cap_draws_endpoint_pixels() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let mut p = Path::new();
        p.move_to(30.0, 50.0);
        p.line_to(70.0, 50.0);
        PathPainter::stroke_with_cap(
            &mut buf,
            &p,
            &ctm,
            &vp,
            BLACK,
            6.0,
            &DashState::solid(),
            &LineCap::Round,
        );
        assert_ne!(buf.get_pixel(30, 50), TRANSPARENT);
    }

    fn lcd_stem_path() -> Path {
        // A two-pixel-wide stem covering device x 2..4 and rows 1..3.
        let mut path = Path::new();
        path.move_to(2.0, 1.0);
        path.line_to(4.0, 1.0);
        path.line_to(4.0, 3.0);
        path.line_to(2.0, 3.0);
        path.close();
        path
    }

    #[test]
    fn lcd_glyph_mask_filters_three_x_coverage_into_rgb_triplets() {
        let mask = rasterize_glyph_lcd_mask(
            &lcd_stem_path(),
            &Transform2D::identity(),
            FillRule::NonZero,
            GlyphHinting::disabled(),
            0,
        )
        .expect("stem rasterizes");
        assert!(mask.is_lcd());
        assert_eq!((mask.x, mask.y, mask.width), (1, 1, 4));
        let row_bytes = mask.width as usize * 3;
        // Sub-pixels 6..12 are covered; the 5-tap {8, 77, 86, 77, 8} filter
        // spreads the edges over two sub-pixels on each side. Red is the
        // leftmost sub-pixel of each pixel.
        assert_eq!(
            &mask.alpha_slice()[..row_bytes],
            &[0, 8, 85, 170, 247, 255, 255, 247, 170, 85, 8, 0]
        );
        assert_eq!(
            &mask.alpha_slice()[row_bytes..2 * row_bytes],
            &mask.alpha_slice()[..row_bytes],
            "every covered row filters the same way"
        );
    }

    #[test]
    fn lcd_glyph_mask_phase_shifts_by_whole_subpixels() {
        let mask = rasterize_glyph_lcd_mask(
            &lcd_stem_path(),
            &Transform2D::identity(),
            FillRule::NonZero,
            GlyphHinting::disabled(),
            1,
        )
        .expect("stem rasterizes");
        assert_eq!((mask.x, mask.width), (1, 5));
        assert_eq!(
            &mask.alpha_slice()[..15],
            &[0, 0, 8, 85, 170, 247, 255, 255, 247, 170, 85, 8, 0, 0, 0]
        );
    }

    #[test]
    fn lcd_glyph_mask_blends_each_channel_with_its_own_coverage() {
        let mask = rasterize_glyph_lcd_mask(
            &lcd_stem_path(),
            &Transform2D::identity(),
            FillRule::NonZero,
            GlyphHinting::disabled(),
            0,
        )
        .expect("stem rasterizes");
        let mut buf = PixelBuffer::new_filled(8, 4, WHITE);
        mask.paint(&mut buf, 0, 0, BLACK);
        // Left edge: the red sub-pixel is the least covered.
        assert_eq!(buf.get_pixel(1, 1), [255, 247, 170, 255]);
        assert_eq!(buf.get_pixel(2, 1), [85, 8, 0, 255]);
        assert_eq!(buf.get_pixel(3, 1), [0, 8, 85, 255]);
        // Right edge: the blue sub-pixel is the least covered.
        assert_eq!(buf.get_pixel(4, 1), [170, 247, 255, 255]);
        assert_eq!(buf.get_pixel(0, 1), WHITE);
        assert_eq!(buf.get_pixel(5, 1), WHITE);
    }

    #[test]
    fn pdfium_text_gamma_table_darkens_partial_coverage_only() {
        let lut = crate::render::vector_quality::text_gamma_lut(
            crate::render::contract::TextGamma::Pdfium,
        )
        .expect("pdfium table");
        assert_eq!(lut[0], 0);
        assert_eq!(lut[255], 255);
        assert_eq!(lut[64], 73);
        assert_eq!(lut[128], 136);
        assert!(lut.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!((1..255).all(|value| lut[value] >= value as u8));
        assert!(crate::render::vector_quality::text_gamma_lut(
            crate::render::contract::TextGamma::Linear
        )
        .is_none());
    }
}
