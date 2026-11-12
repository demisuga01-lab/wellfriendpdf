//! PDFium's drawing of skewed and rotated images, for the PDFium render
//! profile.
//!
//! PDFium's AGG device (`CFX_AggImageRenderer`) draws an image whose device
//! matrix has a real rotation or skew (`|b| >= 0.5` or `|c| >= 0.5` device
//! pixels, and not a quarter turn) through `CFX_ImageTransformer`: the image
//! is first resampled by `CStretchEngine` to an upright bitmap of
//! `ceil(|(a, b)|) x ceil(|(c, d)|)` pixels, and every pixel of the closest
//! integer device rectangle is then mapped back into that bitmap with a
//! 24.8 fixed-point matrix and interpolated bilinearly from its four
//! neighbours. Pixels that map outside the stretched bitmap stay
//! transparent, so the image edges are not anti-aliased. This module ports
//! that pipeline (`core/fxge/dib/cfx_imagetransformer.cpp`,
//! `cstretchengine.cpp`, `cfx_imagestretcher.cpp`) including its single
//! precision matrix arithmetic and integer rounding, so a skewed image
//! lands on the same pixels with the same colours as in PDFium.

use crate::images::decoder::RawImage;
use crate::render::buffer::PixelBuffer;
use crate::render::transform::Transform2D;

const FIXED_POINT_BITS: u32 = 16;
const FIXED_POINT_ONE: u32 = 1 << FIXED_POINT_BITS;
const BILINEAR_BASE: i32 = 256;

/// `CFX_Matrix`: single-precision affine matrix, `x' = a x + c y + e`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Matrix {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Matrix {
    const IDENTITY: Self = Self::new(1.0, 0.0, 0.0, 1.0, 0.0, 0.0);

    const fn new(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self { a, b, c, d, e, f }
    }

    /// `*this * right`: apply `self`, then `right` (`CFX_Matrix::Concat`).
    fn concat(self, r: Self) -> Self {
        Self::new(
            self.a * r.a + self.b * r.c,
            self.a * r.b + self.b * r.d,
            self.c * r.a + self.d * r.c,
            self.c * r.b + self.d * r.d,
            self.e * r.a + self.f * r.c + r.e,
            self.e * r.b + self.f * r.d + r.f,
        )
    }

    /// `CFX_Matrix::GetInverse` (identity for a singular matrix).
    fn inverse(self) -> Self {
        let i = self.a * self.d - self.b * self.c;
        if i.abs() == 0.0 {
            return Self::IDENTITY;
        }
        let j = -i;
        Self::new(
            self.d / i,
            self.b / j,
            self.c / j,
            self.a / i,
            (self.c * self.f - self.d * self.e) / i,
            (self.a * self.f - self.b * self.e) / j,
        )
    }

    fn transform(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// `CFX_Matrix::TransformRect`: bounds of the four transformed corners.
    fn transform_rect(self, rect: FloatRect) -> FloatRect {
        let points = [
            self.transform(rect.left, rect.top),
            self.transform(rect.left, rect.bottom),
            self.transform(rect.right, rect.top),
            self.transform(rect.right, rect.bottom),
        ];
        let mut out = FloatRect {
            left: points[0].0,
            bottom: points[0].1,
            right: points[0].0,
            top: points[0].1,
        };
        for &(x, y) in &points[1..] {
            out.right = out.right.max(x);
            out.left = out.left.min(x);
            out.top = out.top.max(y);
            out.bottom = out.bottom.min(y);
        }
        out
    }

    /// `CFX_Matrix::GetUnitRect`.
    fn unit_rect(self) -> FloatRect {
        self.transform_rect(FloatRect {
            left: 0.0,
            bottom: 0.0,
            right: 1.0,
            top: 1.0,
        })
    }
}

/// `CFX_FloatRect` (y up: `bottom <= top`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct FloatRect {
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
}

impl FloatRect {
    /// `CFX_FloatRect(const FX_RECT&)`.
    fn from_rect(rect: IntRect) -> Self {
        Self {
            left: rect.left as f32,
            bottom: rect.top as f32,
            right: rect.right as f32,
            top: rect.bottom as f32,
        }
    }

    /// `CFX_FloatRect::GetOuterRect`.
    fn outer_rect(self) -> IntRect {
        IntRect {
            left: saturate(self.left.floor()),
            top: saturate(self.bottom.floor()),
            right: saturate(self.right.ceil()),
            bottom: saturate(self.top.ceil()),
        }
        .normalized()
    }

    /// `CFX_FloatRect::GetClosestRect`.
    fn closest_rect(self) -> IntRect {
        let (left, right) = match_float_range(self.left, self.right);
        let (top, bottom) = match_float_range(self.bottom, self.top);
        IntRect {
            left,
            top,
            right,
            bottom,
        }
        .normalized()
    }
}

/// `MatchFloatRange` in `fx_coordinates.cpp`.
fn match_float_range(f1: f32, f2: f32) -> (i32, i32) {
    let length = (f2 - f1).ceil();
    let f1_floor = f1.floor();
    let f1_ceil = f1.ceil();
    let error1 = f1 - f1_floor + (f2 - f1_floor - length).abs();
    let error2 = f1_ceil - f1 + (f2 - f1_ceil - length).abs();
    let start = if error1 > error2 { f1_ceil } else { f1_floor };
    let end = start + length;
    let in_range =
        |value: f32| value.is_finite() && value >= i32::MIN as f32 && value < i32::MAX as f32;
    if in_range(start) && in_range(end) {
        (start as i32, end as i32)
    } else {
        (0, 0)
    }
}

fn saturate(value: f32) -> i32 {
    if value.is_nan() {
        0
    } else {
        value as i32
    }
}

/// `FX_RECT` (y down: `top <= bottom` once normalized).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct IntRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl IntRect {
    fn width(self) -> i32 {
        self.right.wrapping_sub(self.left)
    }

    fn height(self) -> i32 {
        self.bottom.wrapping_sub(self.top)
    }

    fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    fn valid(self) -> bool {
        self.right.checked_sub(self.left).is_some() && self.bottom.checked_sub(self.top).is_some()
    }

    fn normalized(mut self) -> Self {
        if self.left > self.right {
            std::mem::swap(&mut self.left, &mut self.right);
        }
        if self.top > self.bottom {
            std::mem::swap(&mut self.top, &mut self.bottom);
        }
        self
    }

    /// `FX_RECT::Intersect`.
    fn intersect(self, other: Self) -> Self {
        let a = self.normalized();
        let b = other.normalized();
        let out = Self {
            left: a.left.max(b.left),
            top: a.top.max(b.top),
            right: a.right.min(b.right),
            bottom: a.bottom.min(b.bottom),
        };
        if out.left > out.right || out.top > out.bottom {
            Self::default()
        } else {
            out
        }
    }
}

fn fx_round(value: f64) -> i32 {
    if value.is_nan() {
        0
    } else if value < f64::from(i32::MIN) {
        i32::MIN
    } else if value >= f64::from(i32::MAX) {
        i32::MAX
    } else {
        value.round() as i32
    }
}

fn fx_roundf(value: f32) -> i32 {
    if value.is_nan() {
        0
    } else if value < i32::MIN as f32 {
        i32::MIN
    } else if value >= i32::MAX as f32 {
        i32::MAX
    } else {
        value.round() as i32
    }
}

fn fixed_from_double(value: f64) -> u32 {
    fx_round(value * f64::from(FIXED_POINT_ONE)) as u32
}

fn pixel_from_fixed(fixed: u32) -> u8 {
    (fixed >> FIXED_POINT_BITS) as u8
}

/// Whether PDFium's AGG device draws an image with this device matrix
/// through `CFX_ImageTransformer`'s general (`kOther`) path rather than a
/// stretch or quarter-turn stretch.
pub(crate) fn uses_general_transform(combined: &Transform2D) -> bool {
    let m = matrix_of(combined);
    let transformed = (m.b.abs() >= 0.5 || m.a == 0.0) || (m.c.abs() >= 0.5 || m.d == 0.0);
    let quarter_turn = m.a.abs() < m.b.abs() / 20.0
        && m.d.abs() < m.c.abs() / 20.0
        && m.a.abs() < 0.5
        && m.d.abs() < 0.5;
    transformed && !quarter_turn
}

fn matrix_of(t: &Transform2D) -> Matrix {
    Matrix::new(
        t.a as f32, t.b as f32, t.c as f32, t.d as f32, t.e as f32, t.f as f32,
    )
}

/// One destination pixel's source taps (`CStretchEngine::PixelWeight`); an
/// empty weight list produces 0.
#[derive(Clone, Debug, Default)]
struct PixelWeight {
    start: i32,
    weights: Vec<u32>,
}

/// Resampling options (`FXDIB_ResampleOptions`).
#[derive(Clone, Copy, Debug, Default)]
struct ResampleOptions {
    interpolate_bilinear: bool,
    no_smoothing: bool,
}

/// `CStretchEngine::WeightTable::CalculateWeights`, for destination pixels
/// `dest_min..dest_max`.
fn calculate_weights(
    dest_len: i32,
    dest_min: i32,
    dest_max: i32,
    src_len: i32,
    src_min: i32,
    src_max: i32,
    options: ResampleOptions,
) -> Option<Vec<PixelWeight>> {
    if dest_len == 0 {
        return Some(Vec::new());
    }
    if dest_min > dest_max {
        return None;
    }
    let scale = f64::from(src_len) / f64::from(dest_len);
    let base = if dest_len < 0 {
        f64::from(src_len)
    } else {
        0.0
    };
    let range = usize::try_from(dest_max - dest_min).ok()?;
    let mut table = Vec::with_capacity(range);
    // The tap list of `SetStartEnd(start, end)`: `end - start + 1` weights,
    // none when the range is reversed.
    let taps = |start: i32, end: i32| usize::try_from(end - start + 1).unwrap_or(0);
    if options.no_smoothing || scale.abs() < 1.0 {
        for dest_pixel in dest_min..dest_max {
            let src_pos = f64::from(dest_pixel) * scale + scale / 2.0 + base;
            let weight = if options.interpolate_bilinear {
                let start = ((src_pos - 0.5).floor() as i32).max(src_min);
                let end = ((src_pos + 0.5).floor() as i32).min(src_max - 1);
                let count = taps(start, end);
                let mut weights = vec![0; count];
                if start >= end {
                    if let Some(first) = weights.first_mut() {
                        *first = FIXED_POINT_ONE;
                    }
                } else {
                    let second = fixed_from_double(src_pos - f64::from(start) - 0.5);
                    weights[0] = FIXED_POINT_ONE.wrapping_sub(second);
                    weights[1] = second;
                }
                PixelWeight { start, weights }
            } else {
                let pos = src_pos.floor() as i32;
                let start = pos.max(src_min);
                let end = pos.min(src_max - 1);
                let mut weights = vec![0; taps(start, end)];
                if let Some(first) = weights.first_mut() {
                    *first = FIXED_POINT_ONE;
                }
                PixelWeight { start, weights }
            };
            table.push(weight);
        }
        return Some(table);
    }
    for dest_pixel in dest_min..dest_max {
        let src_start = f64::from(dest_pixel) * scale + base;
        let src_end = src_start + scale;
        let mut start_i = src_start.min(src_end).floor() as i32;
        let mut end_i = src_start.max(src_end).floor() as i32;
        start_i = start_i.max(src_min);
        end_i = end_i.min(src_max - 1);
        if start_i > end_i {
            start_i = start_i.clamp(0, (src_max - 1).max(0));
            table.push(PixelWeight {
                start: start_i,
                weights: vec![0],
            });
            continue;
        }
        let mut weights = vec![0u32; taps(start_i, end_i)];
        let mut remaining = FIXED_POINT_ONE;
        let mut rounding_error = 0.0f64;
        for j in start_i..end_i {
            let mut dest_start = (f64::from(j) - base) / scale;
            let mut dest_end = (f64::from(j + 1) - base) / scale;
            if dest_start > dest_end {
                std::mem::swap(&mut dest_start, &mut dest_end);
            }
            let area_start = dest_start.max(f64::from(dest_pixel));
            let area_end = dest_end.min(f64::from(dest_pixel + 1));
            let weight = (area_end - area_start).max(0.0);
            let fixed = fixed_from_double(weight + rounding_error);
            weights[(j - start_i) as usize] = fixed;
            remaining = remaining.wrapping_sub(fixed);
            rounding_error = weight - f64::from(fixed) / f64::from(FIXED_POINT_ONE);
        }
        if remaining != 0 && remaining <= FIXED_POINT_ONE {
            if let Some(last) = weights.last_mut() {
                *last = remaining;
            }
        } else if weights.len() > 1 {
            // `RemoveLastWeightAndAdjust`: drop the last tap and add the
            // (possibly wrapped) remainder to the new last one.
            weights.pop();
            if let Some(last) = weights.last_mut() {
                *last = last.wrapping_add(remaining);
            }
        }
        table.push(PixelWeight {
            start: start_i,
            weights,
        });
    }
    Some(table)
}

/// A decoded bitmap in PDFium's channel order is not needed: every channel
/// is resampled independently, so the stretched bitmap keeps the source's
/// gray, RGB or RGBA layout.
struct Stretched {
    width: usize,
    height: usize,
    channels: usize,
    pixels: Vec<u8>,
}

/// `CStretchEngine`: resamples `image` to `dest_width x dest_height` and
/// returns the `clip` part of the result.
fn stretch(
    image: &RawImage,
    dest_width: i32,
    dest_height: i32,
    clip: IntRect,
    options: ResampleOptions,
) -> Option<Stretched> {
    let src_width = i32::try_from(image.width).ok()?;
    let src_height = i32::try_from(image.height).ok()?;
    let channels = usize::from(image.channels);
    if dest_width == 0 || dest_height == 0 || clip.is_empty() {
        return None;
    }
    let has_alpha = channels == 4;
    let mut options = options;
    if !options.no_smoothing {
        // `UseInterpolateBilinear`: integer arithmetic on the full sizes.
        let use_bilinear = !options.interpolate_bilinear
            && dest_width != 0
            && i64::from(dest_height.abs()) / 8
                < i64::from(src_width) * i64::from(src_height) / i64::from(dest_width.abs());
        if use_bilinear {
            options = ResampleOptions {
                interpolate_bilinear: true,
                no_smoothing: false,
            };
        }
    }
    let scale_x = f64::from(src_width as f32 / dest_width as f32);
    let scale_y = f64::from(src_height as f32 / dest_height as f32);
    let base_x = if dest_width > 0 {
        0.0
    } else {
        f64::from(dest_width)
    };
    let base_y = if dest_height > 0 {
        0.0
    } else {
        f64::from(dest_height)
    };
    let mut src_left = scale_x * (f64::from(clip.left) + base_x);
    let mut src_right = scale_x * (f64::from(clip.right) + base_x);
    let mut src_top = scale_y * (f64::from(clip.top) + base_y);
    let mut src_bottom = scale_y * (f64::from(clip.bottom) + base_y);
    if src_left > src_right {
        std::mem::swap(&mut src_left, &mut src_right);
    }
    if src_top > src_bottom {
        std::mem::swap(&mut src_top, &mut src_bottom);
    }
    let src_clip = IntRect {
        left: src_left.floor() as i32,
        right: src_right.ceil() as i32,
        top: src_top.floor() as i32,
        bottom: src_bottom.ceil() as i32,
    }
    .intersect(IntRect {
        left: 0,
        top: 0,
        right: src_width,
        bottom: src_height,
    });
    if src_clip.height() <= 0 {
        return None;
    }

    let dest_cols = usize::try_from(clip.width()).ok()?;
    let dest_rows = usize::try_from(clip.height()).ok()?;
    let row_bytes = dest_cols.checked_mul(channels)?;
    let horizontal = calculate_weights(
        dest_width,
        clip.left,
        clip.right,
        src_width,
        src_clip.left,
        src_clip.right,
        options,
    )?;
    let src_rows = usize::try_from(src_clip.height()).ok()?;
    let mut inter = vec![0u8; src_rows.checked_mul(row_bytes)?];
    let stride = image.row_stride();
    for (row_index, src_row) in (src_clip.top..src_clip.bottom).enumerate() {
        let row = &image.pixels[src_row as usize * stride..][..stride];
        let out = &mut inter[row_index * row_bytes..][..row_bytes];
        for (col, weight) in horizontal.iter().enumerate() {
            let start = weight.start.max(0) as usize;
            let dest = &mut out[col * channels..][..channels];
            if has_alpha {
                let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
                for (tap, &w) in weight.weights.iter().enumerate() {
                    let Some(px) = row.get((start + tap) * 4..(start + tap) * 4 + 4) else {
                        continue;
                    };
                    let pw = w.wrapping_mul(u32::from(px[3])) / 255;
                    r = r.wrapping_add(pw.wrapping_mul(u32::from(px[0])));
                    g = g.wrapping_add(pw.wrapping_mul(u32::from(px[1])));
                    b = b.wrapping_add(pw.wrapping_mul(u32::from(px[2])));
                    a = a.wrapping_add(pw);
                }
                dest[0] = pixel_from_fixed(r);
                dest[1] = pixel_from_fixed(g);
                dest[2] = pixel_from_fixed(b);
                dest[3] = pixel_from_fixed(a.wrapping_mul(255));
            } else {
                for (channel, value) in dest.iter_mut().enumerate() {
                    let mut sum = 0u32;
                    for (tap, &w) in weight.weights.iter().enumerate() {
                        let Some(&sample) = row.get((start + tap) * channels + channel) else {
                            continue;
                        };
                        sum = sum.wrapping_add(w.wrapping_mul(u32::from(sample)));
                    }
                    *value = pixel_from_fixed(sum);
                }
            }
        }
    }

    let vertical = calculate_weights(
        dest_height,
        clip.top,
        clip.bottom,
        src_height,
        src_clip.top,
        src_clip.bottom,
        options,
    )?;
    let mut pixels = vec![0u8; dest_rows.checked_mul(row_bytes)?];
    // PDFium composes every row from one reused scanline; a fully
    // transparent RGBA pixel keeps the colour the previous row left there.
    let mut scanline = vec![0u8; row_bytes];
    let mut accum = vec![0u32; row_bytes];
    for (row_index, weight) in vertical.iter().enumerate() {
        accum.iter_mut().for_each(|value| *value = 0);
        for (tap, &w) in weight.weights.iter().enumerate() {
            let inter_row = (weight.start - src_clip.top) as usize + tap;
            let Some(src) = inter.get(inter_row * row_bytes..(inter_row + 1) * row_bytes) else {
                continue;
            };
            for (acc, &sample) in accum.iter_mut().zip(src) {
                *acc = acc.wrapping_add(w.wrapping_mul(u32::from(sample)));
            }
        }
        if has_alpha {
            for (dest, acc) in scanline.chunks_exact_mut(4).zip(accum.chunks_exact(4)) {
                if acc[3] != 0 {
                    let unpremultiply =
                        |value: u32| (u64::from(value) * 255 / u64::from(acc[3])).min(255) as u8;
                    dest[0] = unpremultiply(acc[0]);
                    dest[1] = unpremultiply(acc[1]);
                    dest[2] = unpremultiply(acc[2]);
                }
                dest[3] = pixel_from_fixed(acc[3]);
            }
        } else {
            for (dest, &acc) in scanline.iter_mut().zip(&accum) {
                *dest = pixel_from_fixed(acc);
            }
        }
        pixels[row_index * row_bytes..][..row_bytes].copy_from_slice(&scanline);
    }
    Some(Stretched {
        width: dest_cols,
        height: dest_rows,
        channels,
        pixels,
    })
}

/// `CFX_BilinearMatrix`: the 24.8 fixed-point matrix of `DoBilinearLoop`.
struct BilinearMatrix {
    a: i32,
    b: i32,
    c: i32,
    d: i32,
    e: i32,
    f: i32,
}

impl BilinearMatrix {
    fn new(m: Matrix) -> Self {
        let base = BILINEAR_BASE as f32;
        Self {
            a: fx_roundf(m.a * base),
            b: fx_roundf(m.b * base),
            c: fx_roundf(m.c * base),
            d: fx_roundf(m.d * base),
            e: fx_roundf(m.e * base),
            f: fx_roundf(m.f * base),
        }
    }

    /// Source column/row and the 0..255 fractions for destination pixel
    /// `(x, y)`.
    fn transform(&self, x: i32, y: i32) -> (i32, i32, i32, i32) {
        let (x, y) = (x as f32, y as f32);
        let half = (BILINEAR_BASE / 2) as f32;
        let vx = self.a as f32 * x + self.c as f32 * y + self.e as f32 + half;
        let vy = self.b as f32 * x + self.d as f32 * y + self.f as f32 + half;
        let base = BILINEAR_BASE as f32;
        let col = saturate(vx / base);
        let row = saturate(vy / base);
        let wrap = |value: f32| {
            let mut res = saturate(value) % BILINEAR_BASE;
            if res < 0 && res > -BILINEAR_BASE {
                res += BILINEAR_BASE;
            }
            res
        };
        (col, row, wrap(vx), wrap(vy))
    }
}

/// `BilinearInterpolate` for one channel.
fn bilinear(p0: u8, p1: u8, p2: u8, p3: u8, res_x: i32, res_y: i32) -> u8 {
    let i_resx = 255 - res_x;
    let r0 = ((i32::from(p0) * i_resx + i32::from(p1) * res_x) >> 8) as u8;
    let r1 = ((i32::from(p2) * i_resx + i32::from(p3) * res_x) >> 8) as u8;
    ((i32::from(r0) * (255 - res_y) + i32::from(r1) * res_y) >> 8) as u8
}

/// Paints `image` with the device matrix `combined` the way PDFium's
/// `CFX_ImageTransformer` does for skewed and rotated images. Returns
/// `false` (nothing painted) when the matrix is one PDFium stretches
/// instead or the image is degenerate, so the caller can fall back.
pub(crate) fn paint(
    buf: &mut PixelBuffer,
    image: &RawImage,
    combined: &Transform2D,
    interpolate: bool,
    no_smoothing: bool,
    paint_alpha: f32,
) -> bool {
    if !uses_general_transform(combined)
        || !matches!(image.channels, 1 | 3 | 4)
        || image.bits_per_sample != 8
        || !image.is_valid()
    {
        return false;
    }
    let matrix = matrix_of(combined);
    let device = IntRect {
        left: 0,
        top: 0,
        right: i32::try_from(buf.width).unwrap_or(i32::MAX),
        bottom: i32::try_from(buf.height).unwrap_or(i32::MAX),
    };
    // CFX_AggImageRenderer: the device clip box within the outer rectangle.
    let unit = matrix.unit_rect();
    let clip_box = device.intersect(unit.outer_rect());
    if clip_box.is_empty() {
        return true;
    }
    // CFX_ImageTransformer::CFX_ImageTransformer, kOther.
    let result_clip = unit.closest_rect().intersect(clip_box);
    if result_clip.is_empty() {
        return true;
    }
    let stretch_width = matrix.a.hypot(matrix.b).ceil() as i32;
    let stretch_height = matrix.c.hypot(matrix.d).ceil() as i32;
    if stretch_width == 0 || stretch_height == 0 {
        return true;
    }
    let stretch_to_dest =
        Matrix::new(1.0, 0.0, 0.0, -1.0, 0.0, stretch_height as f32).concat(Matrix::new(
            matrix.a / stretch_width as f32,
            matrix.b / stretch_width as f32,
            matrix.c / stretch_height as f32,
            matrix.d / stretch_height as f32,
            matrix.e,
            matrix.f,
        ));
    let dest_to_stretch = stretch_to_dest.inverse();
    let stretch_clip = dest_to_stretch
        .transform_rect(FloatRect::from_rect(result_clip))
        .outer_rect();
    if !stretch_clip.valid() {
        return true;
    }
    let stretch_clip = stretch_clip.intersect(IntRect {
        left: 0,
        top: 0,
        right: stretch_width,
        bottom: stretch_height,
    });
    if !stretch_clip.valid() || stretch_clip.is_empty() {
        return true;
    }
    let options = ResampleOptions {
        interpolate_bilinear: interpolate && !no_smoothing,
        no_smoothing,
    };
    let Some(stretched) = stretch(image, stretch_width, stretch_height, stretch_clip, options)
    else {
        return true;
    };

    // ContinueOther: map every result pixel back into the stretched bitmap.
    let result_to_stretch = Matrix::new(
        1.0,
        0.0,
        0.0,
        1.0,
        result_clip.left as f32,
        result_clip.top as f32,
    )
    .concat(dest_to_stretch);
    let result_to_stretch = Matrix {
        e: result_to_stretch.e + -(stretch_clip.left as f32),
        f: result_to_stretch.f + -(stretch_clip.top as f32),
        ..result_to_stretch
    };
    let fixed = BilinearMatrix::new(result_to_stretch);
    let clip_width = stretch_clip.width();
    let clip_height = stretch_clip.height();
    let channels = stretched.channels;
    let pitch = stretched.width * channels;
    let paint_alpha = paint_alpha.clamp(0.0, 1.0);
    for row in 0..result_clip.height() {
        for col in 0..result_clip.width() {
            let (mut col_l, mut row_l, res_x, res_y) = fixed.transform(col, row);
            if !(col_l >= 0 && col_l <= clip_width && row_l >= 0 && row_l <= clip_height) {
                continue;
            }
            if col_l == clip_width {
                col_l -= 1;
            }
            if row_l == clip_height {
                row_l -= 1;
            }
            let mut col_r = col_l + 1;
            let mut row_r = row_l + 1;
            if col_r == clip_width {
                col_r -= 1;
            }
            if row_r == clip_height {
                row_r -= 1;
            }
            let (Ok(col_l), Ok(col_r), Ok(row_l), Ok(row_r)) = (
                usize::try_from(col_l),
                usize::try_from(col_r),
                usize::try_from(row_l),
                usize::try_from(row_r),
            ) else {
                continue;
            };
            if col_r >= stretched.width || row_r >= stretched.height {
                continue;
            }
            let sample = |channel: usize| {
                let at = |r: usize, c: usize| stretched.pixels[r * pitch + c * channels + channel];
                bilinear(
                    at(row_l, col_l),
                    at(row_l, col_r),
                    at(row_r, col_l),
                    at(row_r, col_r),
                    res_x,
                    res_y,
                )
            };
            let (rgb, alpha) = match channels {
                1 => {
                    let gray = sample(0);
                    ([gray, gray, gray], 255)
                }
                3 => ([sample(0), sample(1), sample(2)], 255),
                _ => ([sample(0), sample(1), sample(2)], sample(3)),
            };
            if alpha == 0 {
                continue;
            }
            buf.blend_pixel(
                result_clip.left + col,
                result_clip.top + row,
                [rgb[0], rgb[1], rgb[2], 255],
                f32::from(alpha) / 255.0 * paint_alpha,
            );
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_rect_matches_pdfium_rounding() {
        let rect = FloatRect {
            left: 5.7,
            bottom: 10.2,
            right: 20.6,
            top: 30.9,
        };
        // x: length ceil(14.9) = 15; starting at 5 errs 0.7 + 0.6, at 6
        // only 0.3 + 0.4. y: length 21; starting at 10 errs 0.2 + 0.1.
        assert_eq!(
            rect.closest_rect(),
            IntRect {
                left: 6,
                top: 10,
                right: 21,
                bottom: 31
            }
        );
        assert_eq!(
            rect.outer_rect(),
            IntRect {
                left: 5,
                top: 10,
                right: 21,
                bottom: 31
            }
        );
    }

    #[test]
    fn upscaling_weights_are_nearest_without_interpolation() {
        let weights = calculate_weights(8, 0, 8, 2, 0, 2, ResampleOptions::default()).unwrap();
        let starts: Vec<i32> = weights.iter().map(|w| w.start).collect();
        assert_eq!(starts, [0, 0, 0, 0, 1, 1, 1, 1]);
        assert!(weights.iter().all(|w| w.weights == [FIXED_POINT_ONE]));
    }

    #[test]
    fn downscaling_weights_sum_to_one() {
        let weights = calculate_weights(3, 0, 3, 10, 0, 10, ResampleOptions::default()).unwrap();
        for weight in &weights {
            assert_eq!(
                weight.weights.iter().sum::<u32>(),
                FIXED_POINT_ONE,
                "{weight:?}"
            );
        }
    }

    #[test]
    fn general_transform_selection_follows_the_agg_renderer() {
        let upright = Transform2D {
            a: 64.0,
            b: 0.0,
            c: 0.0,
            d: -64.0,
            e: 10.0,
            f: 80.0,
        };
        assert!(!uses_general_transform(&upright));
        let skewed = Transform2D { c: 16.0, ..upright };
        assert!(uses_general_transform(&skewed));
        let quarter_turn = Transform2D {
            a: 0.0,
            b: 64.0,
            c: 64.0,
            d: 0.0,
            e: 10.0,
            f: 10.0,
        };
        assert!(!uses_general_transform(&quarter_turn));
    }

    #[test]
    fn skewed_image_has_hard_edges_and_nearest_colours() {
        // 2x1 image, black | white, skewed: x' = 40u + 10v, y' = -20v.
        let image = RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![0, 255],
        };
        let mut buf = PixelBuffer::new_filled(80, 40, [255, 0, 0, 255]);
        let matrix = Transform2D {
            a: 40.0,
            b: 0.0,
            c: 10.0,
            d: -20.0,
            e: 10.0,
            f: 30.0,
        };
        assert!(paint(&mut buf, &image, &matrix, false, false, 1.0));
        // Inside the parallelogram: black on the left half, white on the
        // right half, with at most one interpolated column between them.
        assert_eq!(buf.get_pixel(20, 20), [0, 0, 0, 255]);
        // PDFium's BilinearInterpolate weighs with 255 - fraction out of
        // 256, so even a flat white area comes out as 253.
        assert_eq!(buf.get_pixel(45, 20), [253, 253, 253, 255]);
        // Outside it the background is untouched: no anti-aliased edge.
        assert_eq!(buf.get_pixel(5, 20), [255, 0, 0, 255]);
        let mut partial = 0;
        for y in 0..40 {
            for x in 0..80 {
                let p = buf.get_pixel(x, y);
                if p[0] == 255 && p[1] != 0 && p[1] != 255 {
                    partial += 1;
                }
            }
        }
        assert_eq!(partial, 0, "edges are not blended with the background");
    }
}
