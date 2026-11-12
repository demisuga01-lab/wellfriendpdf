//! Vector raster-quality policies shared by every path paint route.
//!
//! This module holds the pieces of Poppler (Splash) / PDFium raster parity
//! that are independent of a particular scan converter:
//!
//! * **Stroke adjustment** (pixel snapping). Splash hard-wires stroke
//!   adjustment on (it ignores `/SA`, citing Acrobat) and PDFium snaps thin
//!   axis-aligned lines too. Without it, table rules, cell borders and form
//!   boxes at fractional device positions render as two-pixel grey smears.
//!   Wellfriend applies it unconditionally in Compatibility mode; the
//!   geometry-exact HighQuality mode honours the graphics state's `/SA` flag
//!   instead. It is never applied to glyph outlines or stroked text.
//! * **Transformed pens.** A PDF line width is measured in user space, so a
//!   stroke under a non-uniform CTM has an elliptical pen. [`StrokePen`]
//!   strokes in pen space and maps the outline back to device space.
//! * **Contract-level options** (thin-line mode and the optional Poppler
//!   coverage gamma), carried by a scoped thread-local guard the same way the
//!   scalar-compositor contract switch is.

use std::cell::Cell;
use std::sync::OnceLock;

use crate::content::state::LineCap;
use crate::render::buffer::RenderMode;
use crate::render::contract::{
    RectSnapping, RenderProfile, StrokeAdjustment, TextGamma, ThinLineMode, VectorAntialiasGamma,
};
use crate::render::path::{FlatPath, Path, PathSegment};
use crate::render::transform::Transform2D;

thread_local! {
    static CONTRACT_VECTOR_OPTIONS: Cell<(ThinLineMode, VectorAntialiasGamma)> =
        const { Cell::new((ThinLineMode::None, VectorAntialiasGamma::Linear)) };
}

/// Scoped render-contract guard installing the thin-line mode and the vector
/// coverage curve for path paints on the current thread. Nested renders
/// (forms, groups, patterns, soft masks) run on the same thread and therefore
/// inherit the contract's choice; dropping the guard restores the previous
/// value so concurrent renders on other threads are unaffected.
pub(crate) struct VectorRasterContractGuard {
    previous: (ThinLineMode, VectorAntialiasGamma),
}

impl VectorRasterContractGuard {
    pub(crate) fn enter(thin_line_mode: ThinLineMode, gamma: VectorAntialiasGamma) -> Self {
        let previous = CONTRACT_VECTOR_OPTIONS.with(|cell| cell.replace((thin_line_mode, gamma)));
        Self { previous }
    }
}

impl Drop for VectorRasterContractGuard {
    fn drop(&mut self) {
        CONTRACT_VECTOR_OPTIONS.with(|cell| cell.set(self.previous));
    }
}

fn contract_vector_options() -> (ThinLineMode, VectorAntialiasGamma) {
    CONTRACT_VECTOR_OPTIONS.with(Cell::get)
}

thread_local! {
    static CONTRACT_TEXT_GAMMA: Cell<TextGamma> = const { Cell::new(TextGamma::Linear) };
}

/// Scoped render-contract guard installing the glyph coverage gamma for text
/// drawn from glyph masks on the current thread (see
/// [`VectorRasterContractGuard`] for the scoping rules).
pub(crate) struct TextRasterContractGuard {
    previous: TextGamma,
}

impl TextRasterContractGuard {
    pub(crate) fn enter(text_gamma: TextGamma) -> Self {
        let previous = CONTRACT_TEXT_GAMMA.with(|cell| cell.replace(text_gamma));
        Self { previous }
    }
}

impl Drop for TextRasterContractGuard {
    fn drop(&mut self) {
        CONTRACT_TEXT_GAMMA.with(|cell| cell.set(self.previous));
    }
}

/// The glyph coverage gamma installed by the active render contract.
pub(crate) fn contract_text_gamma() -> TextGamma {
    CONTRACT_TEXT_GAMMA.with(Cell::get)
}

thread_local! {
    static CONTRACT_PIXEL_SNAPPING: Cell<(RectSnapping, StrokeAdjustment)> =
        const { Cell::new((RectSnapping::Default, StrokeAdjustment::Default)) };
}

/// Scoped render-contract guard installing the filled-rectangle snapping
/// and stroke-adjustment policies for path paints on the current thread (see
/// [`VectorRasterContractGuard`] for the scoping rules).
pub(crate) struct PixelSnapContractGuard {
    previous: (RectSnapping, StrokeAdjustment),
}

impl PixelSnapContractGuard {
    pub(crate) fn enter(rect_snapping: RectSnapping, stroke_adjustment: StrokeAdjustment) -> Self {
        let previous =
            CONTRACT_PIXEL_SNAPPING.with(|cell| cell.replace((rect_snapping, stroke_adjustment)));
        Self { previous }
    }
}

impl Drop for PixelSnapContractGuard {
    fn drop(&mut self) {
        CONTRACT_PIXEL_SNAPPING.with(|cell| cell.set(self.previous));
    }
}

/// The filled-rectangle snapping installed by the active render contract.
pub(crate) fn contract_rect_snapping() -> RectSnapping {
    CONTRACT_PIXEL_SNAPPING.with(Cell::get).0
}

thread_local! {
    static INTERMEDIATE_BITMAP_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Scoped marker for content PDFium draws into an intermediate bitmap of
/// its own (a tiling-pattern cell, `CPDF_RenderTiling::DrawPatternBitmap`,
/// or a Type 3 glyph, `CPDF_Type3Cache`) rather than straight onto the
/// page: images there keep the pixel-centre placement (see
/// `ImagePainter::paint_image_with_mode`).
pub(crate) struct IntermediateBitmapGuard;

impl IntermediateBitmapGuard {
    pub(crate) fn enter() -> Self {
        INTERMEDIATE_BITMAP_DEPTH.with(|cell| cell.set(cell.get() + 1));
        Self
    }
}

impl Drop for IntermediateBitmapGuard {
    fn drop(&mut self) {
        INTERMEDIATE_BITMAP_DEPTH.with(|cell| cell.set(cell.get().saturating_sub(1)));
    }
}

/// Whether the current paint is inside a tiling-pattern cell or a Type 3
/// glyph.
pub(crate) fn in_intermediate_bitmap() -> bool {
    INTERMEDIATE_BITMAP_DEPTH.with(Cell::get) > 0
}

thread_local! {
    static CONTRACT_PROFILE: Cell<RenderProfile> = const { Cell::new(RenderProfile::Default) };
}

/// Scoped render-contract guard installing the contract's render profile on
/// the current thread, for PDFium behaviour that has no contract field of
/// its own (see [`VectorRasterContractGuard`] for the scoping rules).
pub(crate) struct ProfileContractGuard {
    previous: RenderProfile,
}

impl ProfileContractGuard {
    pub(crate) fn enter(profile: RenderProfile) -> Self {
        let previous = CONTRACT_PROFILE.with(|cell| cell.replace(profile));
        Self { previous }
    }
}

impl Drop for ProfileContractGuard {
    fn drop(&mut self) {
        CONTRACT_PROFILE.with(|cell| cell.set(self.previous));
    }
}

/// The render profile installed by the active render contract.
pub(crate) fn contract_profile() -> RenderProfile {
    CONTRACT_PROFILE.with(Cell::get)
}

/// PDFium's text gamma table (`kTextGammaAdjust` in `cfx_renderdevice.cpp`),
/// applied to glyph coverage before blending.
pub(crate) const PDFIUM_TEXT_GAMMA: [u8; 256] = [
    0, 2, 3, 4, 6, 7, 8, 10, 11, 12, 13, 15, 16, 17, 18, 19, 21, 22, 23, 24, 25, 26, 27, 29, 30,
    31, 32, 33, 34, 35, 36, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 51, 52, 53, 54, 55, 56,
    57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81,
    82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103,
    104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122,
    123, 124, 125, 126, 127, 128, 129, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140,
    141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 156, 157, 158,
    159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 174, 175, 176,
    177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 190, 191, 192, 193, 194,
    195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 204, 205, 206, 207, 208, 209, 210, 211, 212,
    213, 214, 215, 216, 217, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226, 227, 228, 228, 229,
    230, 231, 232, 233, 234, 235, 236, 237, 238, 239, 239, 240, 241, 242, 243, 244, 245, 246, 247,
    248, 249, 250, 250, 251, 252, 253, 254, 255,
];

/// Lookup table for a glyph coverage gamma, or `None` for linear coverage.
pub(crate) fn text_gamma_lut(gamma: TextGamma) -> Option<&'static [u8; 256]> {
    match gamma {
        TextGamma::Linear => None,
        TextGamma::Pdfium => Some(&PDFIUM_TEXT_GAMMA),
    }
}

/// Per-paint vector rasterization options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct VectorRasterOptions {
    pub(crate) stroke_adjust: bool,
    pub(crate) thin_line_mode: ThinLineMode,
    pub(crate) coverage_gamma: VectorAntialiasGamma,
    /// Filled-rectangle snapping rule; it only decides which fills bypass
    /// the anti-aliasing rasterizer, so it is not part of
    /// [`Self::cache_bits`].
    pub(crate) rect_snapping: RectSnapping,
}

impl VectorRasterOptions {
    /// Options for an ordinary (non-glyph) path paint.
    ///
    /// Stroke adjustment is always on in Compatibility mode (Poppler/Acrobat
    /// behaviour: `/SA` is ignored); HighQuality mode is the geometry-exact
    /// mode and only snaps when the content asks for it with `/SA true`.
    /// [`StrokeAdjustment::Off`] in the active contract switches it off.
    pub(crate) fn for_path_paint(
        render_mode: RenderMode,
        stroke_adjustment_requested: bool,
    ) -> Self {
        let (thin_line_mode, coverage_gamma) = contract_vector_options();
        let (rect_snapping, stroke_adjustment) = CONTRACT_PIXEL_SNAPPING.with(Cell::get);
        Self {
            stroke_adjust: stroke_adjustment != StrokeAdjustment::Off
                && (!render_mode.is_high_quality() || stroke_adjustment_requested),
            thin_line_mode,
            coverage_gamma,
            rect_snapping,
        }
    }

    /// Options for glyph outlines and stroked text: no stroke adjustment and
    /// no thin-line policy; the contract's coverage curve still applies to
    /// stroked text outlines (Splash fills them through the vector path).
    pub(crate) fn for_glyph_stroke() -> Self {
        let (_, coverage_gamma) = contract_vector_options();
        Self {
            stroke_adjust: false,
            thin_line_mode: ThinLineMode::None,
            coverage_gamma,
            rect_snapping: RectSnapping::Default,
        }
    }

    /// Compact identity for raster mask caches.
    pub(crate) fn cache_bits(self) -> u8 {
        let thin = match self.thin_line_mode {
            ThinLineMode::None => 0u8,
            ThinLineMode::Solid => 1,
            ThinLineMode::Shape => 2,
        };
        let gamma = match self.coverage_gamma {
            VectorAntialiasGamma::Linear => 0u8,
            VectorAntialiasGamma::Gamma15 => 1,
        };
        u8::from(self.stroke_adjust) | (thin << 1) | (gamma << 3)
    }

    pub(crate) fn coverage_lut(self) -> Option<&'static [u8; 256]> {
        coverage_gamma_lut(self.coverage_gamma)
    }
}

/// Lookup table mapping linear coverage bytes to paint opacity, or `None` for
/// the identity (linear) curve.
pub(crate) fn coverage_gamma_lut(gamma: VectorAntialiasGamma) -> Option<&'static [u8; 256]> {
    match gamma {
        VectorAntialiasGamma::Linear => None,
        VectorAntialiasGamma::Gamma15 => {
            static LUT: OnceLock<[u8; 256]> = OnceLock::new();
            Some(LUT.get_or_init(|| {
                let mut lut = [0u8; 256];
                for (index, slot) in lut.iter_mut().enumerate() {
                    let coverage = index as f64 / 255.0;
                    let mapped = (coverage.powf(1.5) * 255.0).round().clamp(0.0, 255.0) as u8;
                    // Keep every touched pixel visible: a non-zero coverage
                    // never maps to zero opacity.
                    *slot = if index > 0 { mapped.max(1) } else { 0 };
                }
                lut
            }))
        }
    }
}

pub(crate) fn apply_coverage_lut(alpha: &mut [u8], lut: Option<&[u8; 256]>) {
    if let Some(lut) = lut {
        for value in alpha {
            *value = lut[usize::from(*value)];
        }
    }
}

/// Splash's rounding for stroke-adjusted edges (`floor(x + 0.5)`).
fn snap_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Snap a pair of parallel edges to whole device pixels, keeping them at
/// least one pixel apart. Rounding both edges independently (rather than
/// rounding the width) keeps adjacent strokes and fills gap-free.
fn snap_edge_pair(e0: f64, e1: f64) -> (f64, f64) {
    let (lo, hi) = if e0 <= e1 { (e0, e1) } else { (e1, e0) };
    let s0 = snap_round(lo);
    let mut s1 = snap_round(hi);
    if s1 <= s0 {
        s1 = s0 + 1.0;
    }
    if e0 <= e1 {
        (s0, s1)
    } else {
        (s1, s0)
    }
}

/// Largest device coordinate snapped geometry may use while staying safely
/// inside `i32` pixel arithmetic.
const SNAP_COORDINATE_LIMIT: f64 = (1u32 << 30) as f64;

/// Device-space tolerance for "axis-aligned". Decimal PDF matrices place an
/// intended axis-aligned edge a few ten-thousandths of a pixel off axis.
const AXIS_EPSILON: f64 = 1.0e-4;

/// Stroke-adjusted device rectangle for a filled rectangle path.
///
/// Splash adds stroke-adjust hints to any fill consisting of a single
/// four-corner subpath (the shape `re` produces) and snaps its edges when the
/// rectangle is axis-aligned in device space. The result is `(x, y, w, h)` in
/// whole device pixels, at least one pixel in each direction, so hairline
/// rules drawn as filled rectangles never vanish or smear.
pub(crate) fn stroke_adjusted_fill_rect(
    path: &Path,
    device_t: &Transform2D,
) -> Option<(i32, i32, i32, i32)> {
    let corners = rectangle_path_corners(path)?;
    let mut device = [(0.0f64, 0.0f64); 4];
    for (slot, (x, y)) in device.iter_mut().zip(corners) {
        let point = device_t.transform_point(x, y);
        if !point.0.is_finite()
            || !point.1.is_finite()
            || point.0.abs() > SNAP_COORDINATE_LIMIT
            || point.1.abs() > SNAP_COORDINATE_LIMIT
        {
            return None;
        }
        *slot = point;
    }
    // Edges must alternate horizontal / vertical.
    let horizontal = |a: (f64, f64), b: (f64, f64)| (a.1 - b.1).abs() <= AXIS_EPSILON;
    let vertical = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() <= AXIS_EPSILON;
    let first_horizontal = horizontal(device[0], device[1])
        && vertical(device[1], device[2])
        && horizontal(device[2], device[3])
        && vertical(device[3], device[0]);
    let first_vertical = vertical(device[0], device[1])
        && horizontal(device[1], device[2])
        && vertical(device[2], device[3])
        && horizontal(device[3], device[0]);
    if !first_horizontal && !first_vertical {
        return None;
    }
    let min_x = device.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let max_x = device.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let min_y = device.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_y = device.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let (x0, x1) = snap_edge_pair(min_x, max_x);
    let (y0, y1) = snap_edge_pair(min_y, max_y);
    Some((x0 as i32, y0 as i32, (x1 - x0) as i32, (y1 - y0) as i32))
}

/// The whole-pixel rectangle `(x, y, w, h)` a fill of `path` is painted as
/// without the anti-aliasing rasterizer, if any: PDFium's rectangle fast path
/// under [`RectSnapping::Pdfium`], otherwise the stroke-adjusted rectangle
/// when stroke adjustment applies.
pub(crate) fn snapped_fill_rect(
    path: &Path,
    device_t: &Transform2D,
    options: VectorRasterOptions,
) -> Option<(i32, i32, i32, i32)> {
    match options.rect_snapping {
        RectSnapping::Pdfium => crate::render::rect_snap::pdfium_fill_rect(path, device_t),
        RectSnapping::Default if options.stroke_adjust => stroke_adjusted_fill_rect(path, device_t),
        RectSnapping::Default => None,
    }
}

/// The four corners of a path that is exactly one rectangle-shaped subpath:
/// `m l l l` optionally followed by a `l` back to the start and/or `h`.
fn rectangle_path_corners(path: &Path) -> Option<[(f64, f64); 4]> {
    let segments = path.segments.as_slice();
    let (start, rest) = match segments.split_first()? {
        (PathSegment::MoveTo(x, y), rest) => ((*x, *y), rest),
        _ => return None,
    };
    let mut points = vec![start];
    let mut closed = false;
    for segment in rest {
        match *segment {
            PathSegment::LineTo(x, y) if !closed => points.push((x, y)),
            PathSegment::ClosePath if !closed => closed = true,
            _ => return None,
        }
    }
    if points.len() == 5 && points[4] == points[0] {
        points.pop();
    }
    if points.len() != 4 {
        return None;
    }
    Some([points[0], points[1], points[2], points[3]])
}

/// Snap rules for one axis: `(source coordinate, snapped coordinate)`, sorted
/// by source coordinate.
struct AxisSnapRules {
    rules: Vec<(f64, f64)>,
}

/// How close an outline coordinate must be to a hinted edge to be snapped
/// (Splash uses the same 0.01 px window).
const SNAP_MATCH_WINDOW: f64 = 0.01;

impl AxisSnapRules {
    fn new(mut rules: Vec<(f64, f64)>) -> Self {
        rules.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { rules }
    }

    fn snap(&self, value: f64) -> f64 {
        if self.rules.is_empty() {
            return value;
        }
        let start = self
            .rules
            .partition_point(|(from, _)| *from < value - SNAP_MATCH_WINDOW);
        let mut best: Option<(f64, f64)> = None;
        for &(from, to) in &self.rules[start..] {
            if from > value + SNAP_MATCH_WINDOW {
                break;
            }
            let distance = (from - value).abs();
            if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                best = Some((distance, to));
            }
        }
        best.map_or(value, |(_, to)| to)
    }
}

/// Apply stroke adjustment to a device-space stroke outline.
///
/// Matching Splash, adjustment only applies when every segment of the
/// (flattened, device-space) centreline is horizontal or vertical; a single
/// diagonal or curved segment disables it for the whole path. Each segment's
/// two side edges are snapped to whole pixels (at least one pixel apart), and
/// butt / projecting caps of open subpaths have their end edges snapped too.
///
/// `half_extent_x` is the stroke's half thickness measured along device x
/// (the side offset of vertical segments); `half_extent_y` along device y.
pub(crate) fn stroke_adjust_outline(
    centerline: &FlatPath,
    outline: &mut FlatPath,
    half_extent_x: f64,
    half_extent_y: f64,
    cap: &LineCap,
) -> bool {
    if outline.subpaths.is_empty()
        || !half_extent_x.is_finite()
        || !half_extent_y.is_finite()
        || half_extent_x <= 0.0
        || half_extent_y <= 0.0
    {
        return false;
    }
    let mut x_rules = Vec::new();
    let mut y_rules = Vec::new();
    for (index, subpath) in centerline.subpaths.iter().enumerate() {
        let closed = centerline.closed.get(index).copied().unwrap_or(false);
        let mut points: Vec<(f64, f64)> = Vec::with_capacity(subpath.len() + 1);
        for &point in subpath {
            if !point.0.is_finite()
                || !point.1.is_finite()
                || point.0.abs() > SNAP_COORDINATE_LIMIT
                || point.1.abs() > SNAP_COORDINATE_LIMIT
            {
                return false;
            }
            if points.last().is_none_or(|last: &(f64, f64)| {
                (last.0 - point.0).abs() > 1e-9 || (last.1 - point.1).abs() > 1e-9
            }) {
                points.push(point);
            }
        }
        if closed {
            if let (Some(&first), Some(&last)) = (points.first(), points.last()) {
                if (first.0 - last.0).abs() > 1e-9 || (first.1 - last.1).abs() > 1e-9 {
                    points.push(first);
                }
            }
        }
        if points.len() < 2 {
            continue;
        }
        let mut segment_count = 0usize;
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let dx = (b.0 - a.0).abs();
            let dy = (b.1 - a.1).abs();
            if dy <= AXIS_EPSILON {
                let center = (a.1 + b.1) * 0.5;
                let (s0, s1) = snap_edge_pair(center - half_extent_y, center + half_extent_y);
                y_rules.push((center - half_extent_y, s0));
                y_rules.push((center + half_extent_y, s1));
            } else if dx <= AXIS_EPSILON {
                let center = (a.0 + b.0) * 0.5;
                let (s0, s1) = snap_edge_pair(center - half_extent_x, center + half_extent_x);
                x_rules.push((center - half_extent_x, s0));
                x_rules.push((center + half_extent_x, s1));
            } else {
                return false;
            }
            segment_count += 1;
        }
        if closed || matches!(cap, LineCap::Round) || segment_count == 0 {
            continue;
        }
        // Cap end edges of an open subpath.
        let first = (points[0], points[1]);
        let last = (points[points.len() - 2], points[points.len() - 1]);
        let projecting = matches!(cap, LineCap::ProjectingSquare);
        let cap_coordinate = |from: (f64, f64), to: (f64, f64)| -> (bool, f64) {
            // Coordinate of the cap edge at `from`, extended away from `to`.
            if (to.1 - from.1).abs() <= AXIS_EPSILON {
                let direction = (from.0 - to.0).signum();
                let extension = if projecting { half_extent_x } else { 0.0 };
                (true, from.0 + direction * extension)
            } else {
                let direction = (from.1 - to.1).signum();
                let extension = if projecting { half_extent_y } else { 0.0 };
                (false, from.1 + direction * extension)
            }
        };
        let (start_is_x, start_edge) = cap_coordinate(first.0, first.1);
        let (end_is_x, end_edge) = cap_coordinate(last.1, last.0);
        if segment_count == 1 {
            // A single segment: snap both ends as a pair so the line keeps a
            // length of at least one pixel.
            let (s0, s1) = snap_edge_pair(start_edge, end_edge);
            let rules = if start_is_x {
                &mut x_rules
            } else {
                &mut y_rules
            };
            rules.push((start_edge, s0));
            rules.push((end_edge, s1));
        } else {
            let rules = if start_is_x {
                &mut x_rules
            } else {
                &mut y_rules
            };
            rules.push((start_edge, snap_round(start_edge)));
            let rules = if end_is_x { &mut x_rules } else { &mut y_rules };
            rules.push((end_edge, snap_round(end_edge)));
        }
    }
    if x_rules.is_empty() && y_rules.is_empty() {
        return false;
    }
    let x_rules = AxisSnapRules::new(x_rules);
    let y_rules = AxisSnapRules::new(y_rules);
    for subpath in &mut outline.subpaths {
        for point in subpath.iter_mut() {
            point.0 = x_rules.snap(point.0);
            point.1 = y_rules.snap(point.1);
        }
    }
    true
}

/// Geometry of a PDF pen under a device transform.
///
/// PDF line widths are user-space lengths: a round pen of width `w` maps to an
/// ellipse under a non-uniform CTM. For conformal transforms (uniform scale
/// plus rotation/reflection) stroking directly in device space with
/// `w * scale` is exact and cheapest. Otherwise the stroke outline is built in
/// *pen space*, where the pen is a circle of diameter `w`, and mapped to
/// device space. To keep every stroke at least one device pixel thick, the
/// pen's minor axis is widened to one pixel when it would be thinner (the
/// centreline is preserved exactly because the pen-space map is applied to
/// the path and inverted on the outline).
#[derive(Clone, Copy, Debug)]
pub(crate) enum StrokePen {
    /// Stroke in device space with this device width and dash scale.
    Device { width_px: f64, dash_scale: f64 },
    /// Stroke in pen space with the user-space width `width`.
    Transformed {
        width: f64,
        pen_from_device: Transform2D,
        device_from_pen: Transform2D,
        /// Half the stroke thickness measured along device x / device y.
        half_extent_x: f64,
        half_extent_y: f64,
    },
}

impl StrokePen {
    pub(crate) fn new(device_t: &Transform2D, line_width: f64) -> Self {
        let (sx, sy) = (device_t.scale_x(), device_t.scale_y());
        let mean_scale = (sx * sy).sqrt();
        let fallback = Self::Device {
            width_px: if (line_width * mean_scale).is_finite() {
                (line_width * mean_scale).max(1.0)
            } else {
                1.0
            },
            dash_scale: mean_scale,
        };
        if line_width.is_nan() || line_width <= 0.0 || !line_width.is_finite() {
            return fallback;
        }
        let Some((sigma_max, sigma_min, v1)) = singular_values(device_t) else {
            return fallback;
        };
        if sigma_max <= 0.0 || sigma_min <= 0.0 {
            return fallback;
        }
        // Conformal (or thin in every direction): the device-space stroker is
        // exact for a circular device pen.
        if (sigma_max - sigma_min) <= 1e-9 * sigma_max || line_width * sigma_max <= 1.0 {
            return fallback;
        }
        // Widen the minor pen axis to one device pixel when needed.
        let minor_scale = if line_width * sigma_min < 1.0 {
            (1.0 / line_width) / sigma_min
        } else {
            1.0
        };
        // pen_from_user = V diag(1, 1/minor_scale) V^T (symmetric).
        let (vx, vy) = v1;
        let (wx, wy) = (-vy, vx);
        let inv_minor = 1.0 / minor_scale;
        let pen_from_user = Transform2D::new(
            vx * vx + inv_minor * wx * wx,
            vx * vy + inv_minor * wx * wy,
            vx * vy + inv_minor * wx * wy,
            vy * vy + inv_minor * wy * wy,
            0.0,
            0.0,
        );
        let user_from_pen = Transform2D::new(
            vx * vx + minor_scale * wx * wx,
            vx * vy + minor_scale * wx * wy,
            vx * vy + minor_scale * wx * wy,
            vy * vy + minor_scale * wy * wy,
            0.0,
            0.0,
        );
        let Some(user_from_device) = device_t.inverse() else {
            return fallback;
        };
        let pen_from_device = user_from_device.concat(&pen_from_user);
        let device_from_pen = user_from_pen.concat(device_t);
        let half = line_width * 0.5;
        // Support of the device pen ellipse along device x and y.
        let half_extent_x = half * device_from_pen.a.hypot(device_from_pen.c);
        let half_extent_y = half * device_from_pen.b.hypot(device_from_pen.d);
        if ![
            pen_from_device.a,
            pen_from_device.b,
            pen_from_device.c,
            pen_from_device.d,
            pen_from_device.e,
            pen_from_device.f,
            half_extent_x,
            half_extent_y,
        ]
        .iter()
        .all(|value| value.is_finite())
        {
            return fallback;
        }
        Self::Transformed {
            width: line_width,
            pen_from_device,
            device_from_pen,
            half_extent_x,
            half_extent_y,
        }
    }

    /// Effective device width used by thin-line policies (geometric mean of
    /// the pen's device extents, before the one-pixel minimum).
    pub(crate) fn true_device_width(device_t: &Transform2D, line_width: f64) -> f64 {
        line_width * (device_t.scale_x() * device_t.scale_y()).sqrt()
    }

    pub(crate) fn half_extents(self) -> (f64, f64) {
        match self {
            Self::Device { width_px, .. } => (width_px * 0.5, width_px * 0.5),
            Self::Transformed {
                half_extent_x,
                half_extent_y,
                ..
            } => (half_extent_x, half_extent_y),
        }
    }
}

/// Singular values (max, min) of the linear part of `t` and the right
/// singular vector (user-space direction) of the largest one.
fn singular_values(t: &Transform2D) -> Option<(f64, f64, (f64, f64))> {
    // M = L^T L for L = [[a, c], [b, d]] (x' = a x + c y, y' = b x + d y).
    let m11 = t.a * t.a + t.b * t.b;
    let m12 = t.a * t.c + t.b * t.d;
    let m22 = t.c * t.c + t.d * t.d;
    if ![m11, m12, m22].iter().all(|v| v.is_finite()) {
        return None;
    }
    let half_trace = 0.5 * (m11 + m22);
    let det = m11 * m22 - m12 * m12;
    let disc = (half_trace * half_trace - det).max(0.0).sqrt();
    let lambda_max = half_trace + disc;
    let lambda_min = (half_trace - disc).max(0.0);
    let v1 = if m12.abs() > 1e-12 * (m11.abs() + m22.abs()).max(1e-300) {
        let (x, y) = (lambda_max - m22, m12);
        let len = x.hypot(y);
        if len <= 0.0 || !len.is_finite() {
            return None;
        }
        (x / len, y / len)
    } else if m11 >= m22 {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    Some((lambda_max.sqrt(), lambda_min.sqrt(), v1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_edge_pair_rounds_both_edges_and_keeps_one_pixel() {
        assert_eq!(snap_edge_pair(10.1, 11.3), (10.0, 11.0));
        assert_eq!(snap_edge_pair(10.4, 11.6), (10.0, 12.0));
        assert_eq!(snap_edge_pair(10.3, 10.6), (10.0, 11.0));
        assert_eq!(snap_edge_pair(9.75, 10.25), (10.0, 11.0));
        assert_eq!(snap_edge_pair(11.0, 10.0), (11.0, 10.0));
    }

    #[test]
    fn stroke_adjusted_fill_rect_snaps_fractional_rectangles() {
        let mut path = Path::new();
        path.rect(10.3, 20.2, 30.0, 0.3);
        let rect = stroke_adjusted_fill_rect(&path, &Transform2D::identity())
            .expect("axis-aligned rectangle");
        assert_eq!(rect, (10, 20, 30, 1));

        let rotated = Transform2D::new(0.8, 0.6, -0.6, 0.8, 0.0, 0.0);
        assert!(stroke_adjusted_fill_rect(&path, &rotated).is_none());

        let mut triangle = Path::new();
        triangle.move_to(0.0, 0.0);
        triangle.line_to(10.0, 0.0);
        triangle.line_to(10.0, 10.0);
        triangle.close();
        assert!(stroke_adjusted_fill_rect(&triangle, &Transform2D::identity()).is_none());
    }

    #[test]
    fn stroke_adjust_outline_snaps_axis_aligned_strokes_only() {
        let centerline = FlatPath {
            subpaths: vec![vec![(10.2, 30.4), (50.7, 30.4)]],
            closed: vec![false],
        };
        let mut outline = FlatPath {
            subpaths: vec![vec![(10.2, 29.9), (50.7, 29.9), (50.7, 30.9), (10.2, 30.9)]],
            closed: vec![false],
        };
        assert!(stroke_adjust_outline(
            &centerline,
            &mut outline,
            0.5,
            0.5,
            &LineCap::Butt
        ));
        assert_eq!(
            outline.subpaths[0],
            vec![(10.0, 30.0), (51.0, 30.0), (51.0, 31.0), (10.0, 31.0)]
        );

        let diagonal = FlatPath {
            subpaths: vec![vec![(0.0, 0.0), (10.0, 10.0)]],
            closed: vec![false],
        };
        let mut untouched = outline.clone();
        assert!(!stroke_adjust_outline(
            &diagonal,
            &mut untouched,
            0.5,
            0.5,
            &LineCap::Butt
        ));
        assert_eq!(untouched.subpaths, outline.subpaths);
    }

    #[test]
    fn stroke_pen_is_device_space_for_conformal_transforms() {
        match StrokePen::new(&Transform2D::new(2.0, 0.0, 0.0, -2.0, 0.0, 0.0), 3.0) {
            StrokePen::Device {
                width_px,
                dash_scale,
            } => {
                assert_eq!(width_px, 6.0);
                assert_eq!(dash_scale, 2.0);
            }
            other => panic!("expected device pen, got {other:?}"),
        }
    }

    #[test]
    fn stroke_pen_tracks_non_uniform_scale_per_axis() {
        // x scaled 4x, y scaled 1x: a 2-unit pen is 8 px wide horizontally
        // and 2 px tall vertically.
        let pen = StrokePen::new(&Transform2D::new(4.0, 0.0, 0.0, 1.0, 0.0, 0.0), 2.0);
        let (hx, hy) = pen.half_extents();
        assert!((hx - 4.0).abs() < 1e-9, "{hx}");
        assert!((hy - 1.0).abs() < 1e-9, "{hy}");

        // A pen thinner than one pixel along y is widened to exactly 1 px
        // there while keeping its x extent.
        let pen = StrokePen::new(&Transform2D::new(8.0, 0.0, 0.0, 0.25, 0.0, 0.0), 1.0);
        let (hx, hy) = pen.half_extents();
        assert!((hx - 4.0).abs() < 1e-9, "{hx}");
        assert!((hy - 0.5).abs() < 1e-9, "{hy}");
    }

    #[test]
    fn gamma_lut_matches_poppler_curve_and_keeps_touched_pixels() {
        let lut = coverage_gamma_lut(VectorAntialiasGamma::Gamma15).expect("gamma lut");
        assert_eq!(lut[0], 0);
        assert_eq!(lut[255], 255);
        assert_eq!(lut[128], 91);
        assert!(lut[1] >= 1);
        assert!(coverage_gamma_lut(VectorAntialiasGamma::Linear).is_none());
    }
}
