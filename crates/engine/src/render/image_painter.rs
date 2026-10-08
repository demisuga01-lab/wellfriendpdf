use crate::images::decoder::RawImage;
use crate::render::buffer::PixelBuffer;
use crate::render::transform::{Transform2D, Viewport};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SmoothMode {
    None,
    Interpolate,
    LegacyBilinear,
}

#[derive(Clone, Copy)]
struct BilinearAxisSample {
    low: usize,
    high: usize,
    fraction: f32,
}

pub struct ImagePainter;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AxisAlignedImageTarget {
    pub x_origin: i32,
    pub y_origin: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OrthogonalSourceAxis {
    X,
    Y,
}

/// Affine mapping from one device-space axis to one decoded source-image
/// axis. Orthogonal image transforms have exactly one such mapping for device
/// X and one for device Y; keeping the mapping in the retained cache target
/// lets quarter-turn rotations and reflections use the same linear-time box
/// reducer as canonical page scans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OrthogonalSourceMapping {
    pub axis: OrthogonalSourceAxis,
    pub scale: f64,
    pub offset: f64,
}

/// Device-pixel coverage and source-sampling geometry for a canonical,
/// top-down axis-aligned image draw.
///
/// Unlike [`AxisAlignedImageTarget`], this descriptor deliberately retains the
/// fractional device-space image bounds.  That phase is part of the scaled
/// image cache identity and lets scanned pages use a retained reduction even
/// when the page box and the painted image differ by a fraction of a pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AxisAlignedImageCacheTarget {
    pub paint: AxisAlignedImageTarget,
    pub device_x_min: f64,
    pub device_y_min: f64,
    pub device_width: f64,
    pub device_height: f64,
    pub device_x_source: OrthogonalSourceMapping,
    pub device_y_source: OrthogonalSourceMapping,
}

impl ImagePainter {
    /// Paint a decoded image onto the buffer.
    pub fn paint_image(
        buf: &mut PixelBuffer,
        image: &RawImage,
        ctm: &Transform2D,
        viewport: &Viewport,
    ) {
        Self::paint_image_with_options_and_alpha(buf, image, ctm, viewport, false, 1.0);
    }

    /// Paint a decoded image with an additional graphics-state alpha.
    pub fn paint_image_with_alpha(
        buf: &mut PixelBuffer,
        image: &RawImage,
        ctm: &Transform2D,
        viewport: &Viewport,
        paint_alpha: f32,
    ) {
        Self::paint_image_with_options_and_alpha(buf, image, ctm, viewport, false, paint_alpha);
    }

    /// Paint with optional `/Interpolate` smoothing when the image is magnified.
    /// PDF's default is nearest-neighbour for magnification; when interpolation
    /// is requested, smooth photographic upscaling is used instead. Downscaling
    /// always integrates the source footprint with deterministic sRGB-space area
    /// averaging, matching the default proof renderer convention.
    pub fn paint_image_with_options(
        buf: &mut PixelBuffer,
        image: &RawImage,
        ctm: &Transform2D,
        viewport: &Viewport,
        interpolate: bool,
    ) {
        Self::paint_image_with_options_and_alpha(buf, image, ctm, viewport, interpolate, 1.0);
    }

    /// Paint with optional `/Interpolate` smoothing and an additional
    /// graphics-state alpha multiplier.
    pub fn paint_image_with_options_and_alpha(
        buf: &mut PixelBuffer,
        image: &RawImage,
        ctm: &Transform2D,
        viewport: &Viewport,
        interpolate: bool,
        paint_alpha: f32,
    ) {
        let mode = if interpolate {
            SmoothMode::Interpolate
        } else {
            SmoothMode::None
        };
        Self::paint_image_with_mode(buf, image, ctm, viewport, mode, paint_alpha);
    }

    /// Return an exact integer-pixel target for a non-skewed image draw.
    ///
    /// The cached scaled-image path uses this to ensure replay paints the same
    /// device pixels as the ordinary axis-aligned renderer. Fractional origins
    /// or dimensions stay on the general sampler because caching those would
    /// require sub-pixel phase to be part of the cache key.
    #[cfg(test)]
    pub(crate) fn axis_aligned_integer_target(
        ctm: &Transform2D,
        viewport: &Viewport,
    ) -> Option<AxisAlignedImageTarget> {
        if ctm.determinant().abs() < 1e-10 {
            return None;
        }
        let combined = ctm.concat(&viewport.to_transform());
        // The scaled-image cache stores the decoded raster in device row order,
        // so it is valid only for the canonical PDF-image orientation: source
        // columns advance to the right and source rows advance downward. Page
        // rotation, reflection, and a reversed image matrix must use the affine
        // sampler, which retains that orientation explicitly.
        if !Self::is_top_down_axis_aligned(&combined) {
            return None;
        }
        let corners = [
            combined.transform_point(0.0, 0.0),
            combined.transform_point(1.0, 0.0),
            combined.transform_point(0.0, 1.0),
            combined.transform_point(1.0, 1.0),
        ];
        let (px_min, px_max, py_min, py_max) = bounding_box(&corners);
        let x0 = exact_integer_pixel(px_min)?;
        let x1 = exact_integer_pixel(px_max)?;
        let y0 = exact_integer_pixel(py_min)?;
        let y1 = exact_integer_pixel(py_max)?;
        let width = u32::try_from(x1.checked_sub(x0)?).ok()?;
        let height = u32::try_from(y1.checked_sub(y0)?).ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        Some(AxisAlignedImageTarget {
            x_origin: x0,
            y_origin: y0,
            width,
            height,
        })
    }

    /// Return the visible device-pixel target for an orthogonal image while
    /// preserving its fractional sampling phase and source orientation.
    ///
    /// A destination pixel participates exactly when its centre lies inside
    /// the transformed image. This matches `paint_axis_aligned` without
    /// including the otherwise skipped row/column at an exact upper edge.
    pub(crate) fn axis_aligned_cache_target(
        ctm: &Transform2D,
        viewport: &Viewport,
    ) -> Option<AxisAlignedImageCacheTarget> {
        if ctm.determinant().abs() < 1e-10 {
            return None;
        }
        let combined = ctm.concat(&viewport.to_transform());
        let inverse = combined.inverse()?;
        let epsilon = 1e-10;
        let source_width = 1.0;
        let source_height = 1.0;
        let (device_x_source, device_y_source) = if inverse.c.abs() <= epsilon
            && inverse.b.abs() <= epsilon
            && inverse.a.abs() > epsilon
            && inverse.d.abs() > epsilon
        {
            (
                OrthogonalSourceMapping {
                    axis: OrthogonalSourceAxis::X,
                    scale: source_width * inverse.a,
                    offset: source_width * inverse.e,
                },
                OrthogonalSourceMapping {
                    axis: OrthogonalSourceAxis::Y,
                    scale: -source_height * inverse.d,
                    offset: source_height * (1.0 - inverse.f),
                },
            )
        } else if inverse.a.abs() <= epsilon
            && inverse.d.abs() <= epsilon
            && inverse.b.abs() > epsilon
            && inverse.c.abs() > epsilon
        {
            (
                OrthogonalSourceMapping {
                    axis: OrthogonalSourceAxis::Y,
                    scale: -source_height * inverse.b,
                    offset: source_height * (1.0 - inverse.f),
                },
                OrthogonalSourceMapping {
                    axis: OrthogonalSourceAxis::X,
                    scale: source_width * inverse.c,
                    offset: source_width * inverse.e,
                },
            )
        } else {
            return None;
        };
        let corners = [
            combined.transform_point(0.0, 0.0),
            combined.transform_point(1.0, 0.0),
            combined.transform_point(0.0, 1.0),
            combined.transform_point(1.0, 1.0),
        ];
        let (px_min, px_max, py_min, py_max) = bounding_box(&corners);
        let device_width = px_max - px_min;
        let device_height = py_max - py_min;
        if !px_min.is_finite()
            || !py_min.is_finite()
            || !device_width.is_finite()
            || !device_height.is_finite()
            || device_width <= 0.0
            || device_height <= 0.0
        {
            return None;
        }

        let x0 = ceil_i32(px_min - 0.5).clamp(0, viewport.width_px as i32);
        let x_end = ceil_i32(px_max - 0.5).clamp(0, viewport.width_px as i32);
        let y0 = ceil_i32(py_min - 0.5).clamp(0, viewport.height_px as i32);
        let y_end = ceil_i32(py_max - 0.5).clamp(0, viewport.height_px as i32);
        let width = u32::try_from(x_end.checked_sub(x0)?).ok()?;
        let height = u32::try_from(y_end.checked_sub(y0)?).ok()?;
        if width == 0 || height == 0 {
            return None;
        }

        Some(AxisAlignedImageCacheTarget {
            paint: AxisAlignedImageTarget {
                x_origin: x0,
                y_origin: y0,
                width,
                height,
            },
            device_x_min: px_min,
            device_y_min: py_min,
            device_width,
            device_height,
            device_x_source,
            device_y_source,
        })
    }

    #[inline]
    fn is_top_down_axis_aligned(transform: &Transform2D) -> bool {
        transform.is_axis_aligned() && transform.a > 1e-10 && transform.d < -1e-10
    }

    /// Pre-scale an axis-aligned image to a device-size RGB image using the same
    /// default sampling decisions as [`paint_axis_aligned`].
    ///
    /// This deliberately supports only opaque 8-bit gray/RGB sources. Images
    /// with alpha, soft masks, interpolation, fractional phase, or affine
    /// transforms remain on the exact general path. The return value is a
    /// one-to-one RGB image that can be row-copied or binary-clipped quickly on
    /// every later occurrence of the same XObject/scale/render-mode key.
    #[cfg(test)]
    pub(crate) fn scale_axis_aligned_default_rgb(
        image: &RawImage,
        target_width: u32,
        target_height: u32,
        high_quality: bool,
    ) -> Option<RawImage> {
        let target = AxisAlignedImageCacheTarget {
            paint: AxisAlignedImageTarget {
                x_origin: 0,
                y_origin: 0,
                width: target_width,
                height: target_height,
            },
            device_x_min: 0.0,
            device_y_min: 0.0,
            device_width: f64::from(target_width),
            device_height: f64::from(target_height),
            device_x_source: OrthogonalSourceMapping {
                axis: OrthogonalSourceAxis::X,
                scale: 1.0 / f64::from(target_width),
                offset: 0.0,
            },
            device_y_source: OrthogonalSourceMapping {
                axis: OrthogonalSourceAxis::Y,
                scale: 1.0 / f64::from(target_height),
                offset: 0.0,
            },
        };
        Self::scale_axis_aligned_default_rgb_for_target(image, &target, high_quality)
    }

    pub(crate) fn scale_axis_aligned_default_rgb_for_target(
        image: &RawImage,
        target: &AxisAlignedImageCacheTarget,
        _high_quality: bool,
    ) -> Option<RawImage> {
        let target_width = target.paint.width;
        let target_height = target.paint.height;
        if target_width == 0
            || target_height == 0
            || image.width == 0
            || image.height == 0
            || image.bits_per_sample != 8
            || !matches!(image.channels, 1 | 3)
        {
            return None;
        }
        let pixels_len = (target_width as usize)
            .checked_mul(target_height as usize)?
            .checked_mul(3)?;
        let mut pixels = vec![0u8; pixels_len];
        let device_x_source_extent = match target.device_x_source.axis {
            OrthogonalSourceAxis::X => image.width as f64,
            OrthogonalSourceAxis::Y => image.height as f64,
        };
        let device_y_source_extent = match target.device_y_source.axis {
            OrthogonalSourceAxis::X => image.width as f64,
            OrthogonalSourceAxis::Y => image.height as f64,
        };
        let footprint_x = target.device_x_source.scale.abs() * device_x_source_extent;
        let footprint_y = target.device_y_source.scale.abs() * device_y_source_extent;
        // Bilinear sampling is not a minification filter: it observes at most
        // four source samples regardless of the source footprint. On scanned
        // pages that turns printer/scan halftones into large moire dots. Use an
        // exact source-footprint box reduction whenever both axes shrink. This
        // path is phase-correct for the device target, runs in
        // O(source pixels), and keeps only one destination row of scratch data.
        if footprint_x > 1.0 && footprint_y > 1.0 {
            return Self::box_downscale_rgb_for_target(image, target);
        }
        if target.device_x_source.axis != OrthogonalSourceAxis::X
            || target.device_y_source.axis != OrthogonalSourceAxis::Y
            || target.device_x_source.scale <= 0.0
            || target.device_y_source.scale <= 0.0
        {
            return None;
        }
        let smooth = if Self::magnifying(image, target.device_width, target.device_height) {
            SmoothMode::None
        } else {
            SmoothMode::LegacyBilinear
        };
        for y in 0..target_height as usize {
            let device_y = f64::from(target.paint.y_origin) + y as f64 + 0.5;
            let v = (device_y - target.device_y_min) / target.device_height;
            for x in 0..target_width as usize {
                let device_x = f64::from(target.paint.x_origin) + x as f64 + 0.5;
                let u = (device_x - target.device_x_min) / target.device_width;
                let sample = Self::sample(
                    image,
                    u,
                    v,
                    footprint_x,
                    footprint_y,
                    smooth,
                    footprint_x > 1.0 || footprint_y > 1.0,
                );
                let base = (y * target_width as usize + x) * 3;
                pixels[base] = sample[0];
                pixels[base + 1] = sample[1];
                pixels[base + 2] = sample[2];
            }
        }
        Some(RawImage {
            width: target_width,
            height: target_height,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        })
    }

    fn box_downscale_rgb_for_target(
        image: &RawImage,
        target: &AxisAlignedImageCacheTarget,
    ) -> Option<RawImage> {
        let target_width = target.paint.width;
        let target_height = target.paint.height;
        if target_width == 0
            || target_height == 0
            || image.bits_per_sample != 8
            || !matches!(image.channels, 1 | 3)
            || !image.is_valid()
        {
            return None;
        }

        let dst_width = target_width as usize;
        let dst_height = target_height as usize;
        let source_width = image.width as usize;
        let source_height = image.height as usize;
        let output_len = dst_width.checked_mul(dst_height)?.checked_mul(3)?;
        let row_len = dst_width.checked_mul(3)?;
        let mut pixels = vec![0u8; output_len];
        let mut accumulated = vec![0.0_f64; row_len];
        let mut normalization = vec![0.0_f64; dst_width];
        let x_spans = (0..dst_width)
            .map(|destination_x| {
                let device_x = f64::from(target.paint.x_origin) + destination_x as f64;
                orthogonal_source_box_span(device_x, device_x + 1.0, target.device_x_source, image)
            })
            .collect::<Option<Vec<_>>>()?;
        let y_spans = (0..dst_height)
            .map(|destination_y| {
                let device_y = f64::from(target.paint.y_origin) + destination_y as f64;
                orthogonal_source_box_span(device_y, device_y + 1.0, target.device_y_source, image)
            })
            .collect::<Option<Vec<_>>>()?;

        for (destination_y, y_span) in y_spans.iter().enumerate() {
            accumulated.fill(0.0);
            normalization.fill(0.0);
            if target.device_x_source.axis == OrthogonalSourceAxis::X {
                for source_y in y_span.first..y_span.end {
                    let y_weight = box_pixel_weight(source_y, y_span.start, y_span.finish);
                    if y_weight <= 0.0 {
                        continue;
                    }
                    for (destination_x, x_span) in x_spans.iter().enumerate() {
                        let output_base = destination_x * 3;

                        for source_x in x_span.first..x_span.end {
                            let x_weight = box_pixel_weight(source_x, x_span.start, x_span.finish);
                            if x_weight <= 0.0 {
                                continue;
                            }
                            let weight = x_weight * y_weight;
                            normalization[destination_x] += weight;
                            let source_base = (source_y * source_width + source_x)
                                .checked_mul(image.channels as usize)?;
                            if image.channels == 1 {
                                let gray = *image.pixels.get(source_base)? as f64;
                                accumulated[output_base] += gray * weight;
                                accumulated[output_base + 1] += gray * weight;
                                accumulated[output_base + 2] += gray * weight;
                            } else {
                                accumulated[output_base] +=
                                    *image.pixels.get(source_base)? as f64 * weight;
                                accumulated[output_base + 1] +=
                                    *image.pixels.get(source_base + 1)? as f64 * weight;
                                accumulated[output_base + 2] +=
                                    *image.pixels.get(source_base + 2)? as f64 * weight;
                            }
                        }
                    }
                }
            } else {
                // A quarter-turn maps destination X to source Y and
                // destination Y to source X. Iterate each footprint in source
                // row-major order, matching the general affine sampler's
                // accumulation order exactly at byte-rounding boundaries.
                for (destination_x, x_span) in x_spans.iter().enumerate() {
                    let output_base = destination_x * 3;
                    for source_y in x_span.first..x_span.end {
                        let x_weight = box_pixel_weight(source_y, x_span.start, x_span.finish);
                        if x_weight <= 0.0 {
                            continue;
                        }
                        for source_x in y_span.first..y_span.end {
                            let y_weight = box_pixel_weight(source_x, y_span.start, y_span.finish);
                            if y_weight <= 0.0
                                || source_x >= source_width
                                || source_y >= source_height
                            {
                                continue;
                            }
                            let weight = x_weight * y_weight;
                            normalization[destination_x] += weight;
                            let source_base = (source_y * source_width + source_x)
                                .checked_mul(image.channels as usize)?;
                            if image.channels == 1 {
                                let gray = *image.pixels.get(source_base)? as f64;
                                accumulated[output_base] += gray * weight;
                                accumulated[output_base + 1] += gray * weight;
                                accumulated[output_base + 2] += gray * weight;
                            } else {
                                accumulated[output_base] +=
                                    *image.pixels.get(source_base)? as f64 * weight;
                                accumulated[output_base + 1] +=
                                    *image.pixels.get(source_base + 1)? as f64 * weight;
                                accumulated[output_base + 2] +=
                                    *image.pixels.get(source_base + 2)? as f64 * weight;
                            }
                        }
                    }
                }
            }

            let output_row = destination_y * row_len;
            for (destination_x, total_weight) in normalization.iter().copied().enumerate() {
                if total_weight <= 0.0 {
                    continue;
                }
                let output_base = output_row + destination_x * 3;
                for channel in 0..3 {
                    pixels[output_base + channel] =
                        (accumulated[destination_x * 3 + channel] / total_weight)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                }
            }
        }

        Some(RawImage {
            width: target_width,
            height: target_height,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        })
    }

    pub(crate) fn paint_scaled_rgb_at_device_target(
        buf: &mut PixelBuffer,
        image: &RawImage,
        target: AxisAlignedImageTarget,
        paint_alpha: f32,
    ) -> bool {
        let paint_alpha = paint_alpha.clamp(0.0, 1.0);
        if paint_alpha <= 0.0 {
            return true;
        }
        if image.width != target.width
            || image.height != target.height
            || image.channels != 3
            || image.bits_per_sample != 8
            || !image.is_valid()
        {
            return false;
        }
        let (x0, x1, y0, y1) = clipped_bounds(
            buf,
            f64::from(target.x_origin),
            f64::from(target.x_origin) + f64::from(target.width),
            f64::from(target.y_origin),
            f64::from(target.y_origin) + f64::from(target.height),
        );
        if x0 > x1 || y0 > y1 {
            return true;
        }
        if paint_alpha >= 1.0 && buf.can_write_opaque_with_binary_clip() {
            return Self::paint_axis_aligned_one_to_one_rgb_runs(
                buf,
                image,
                f64::from(target.x_origin),
                f64::from(target.y_origin),
                f64::from(target.width),
                f64::from(target.height),
                x0,
                x1,
                y0,
                y1,
            );
        }
        for py in y0..=y1 {
            let sy = py - target.y_origin;
            if sy < 0 || sy >= image.height as i32 {
                continue;
            }
            for px in x0..=x1 {
                let sx = px - target.x_origin;
                if sx < 0 || sx >= image.width as i32 {
                    continue;
                }
                let sample = Self::get_pixel_channels(image, sx as usize, sy as usize);
                buf.blend_pixel(px, py, [sample[0], sample[1], sample[2], 255], paint_alpha);
            }
        }
        true
    }

    fn paint_image_with_mode(
        buf: &mut PixelBuffer,
        image: &RawImage,
        ctm: &Transform2D,
        viewport: &Viewport,
        smooth_mode: SmoothMode,
        paint_alpha: f32,
    ) {
        let paint_alpha = paint_alpha.clamp(0.0, 1.0);
        if paint_alpha <= 0.0 {
            return;
        }
        if image.width == 0 || image.height == 0 || image.channels == 0 || image.pixels.is_empty() {
            return;
        }
        if image.bits_per_sample != 8 || !matches!(image.channels, 1 | 3 | 4) || !image.is_valid() {
            log::warn!(
                "ImagePainter: invalid decoded image {}x{} x{} channels, skipping image",
                image.width,
                image.height,
                image.channels
            );
            return;
        }
        if ctm.determinant().abs() < 1e-10 {
            log::warn!("ImagePainter: singular transform, skipping image");
            return;
        }

        let vp_transform = viewport.to_transform();
        let combined = ctm.concat(&vp_transform);

        if Self::is_top_down_axis_aligned(&combined) {
            Self::paint_axis_aligned(buf, image, &combined, smooth_mode, paint_alpha);
        } else {
            Self::paint_affine(buf, image, &combined, smooth_mode, paint_alpha);
        }
    }

    /// Decide whether a paint is magnifying the source. The PDF default
    /// (`/Interpolate false`) keeps magnification nearest-neighbour so small
    /// pixel art and masks remain crisp.
    fn magnifying(image: &RawImage, dst_w: f64, dst_h: f64) -> bool {
        // Magnifying when each source pixel covers more than one destination
        // pixel on either axis (dst extent exceeds source extent).
        dst_w >= image.width as f64 && dst_h >= image.height as f64
    }

    fn sample(
        image: &RawImage,
        u: f64,
        v: f64,
        footprint_x: f64,
        footprint_y: f64,
        smooth_mode: SmoothMode,
        use_area_average: bool,
    ) -> [u8; 4] {
        if use_area_average && (footprint_x > 1.0 || footprint_y > 1.0) {
            Self::area_average_sample(image, u, v, footprint_x.max(1.0), footprint_y.max(1.0))
        } else {
            match smooth_mode {
                SmoothMode::None => Self::nearest_sample(image, u, v),
                SmoothMode::Interpolate => Self::interpolated_sample(image, u, v),
                SmoothMode::LegacyBilinear => Self::bilinear_sample(image, u, v),
            }
        }
    }

    fn paint_axis_aligned(
        buf: &mut PixelBuffer,
        image: &RawImage,
        combined: &Transform2D,
        smooth_mode: SmoothMode,
        paint_alpha: f32,
    ) {
        let corners = [
            combined.transform_point(0.0, 0.0),
            combined.transform_point(1.0, 0.0),
            combined.transform_point(0.0, 1.0),
            combined.transform_point(1.0, 1.0),
        ];
        let (px_min, px_max, py_min, py_max) = bounding_box(&corners);
        if !px_min.is_finite() || !px_max.is_finite() || !py_min.is_finite() || !py_max.is_finite()
        {
            return;
        }

        let dst_w = (px_max - px_min).max(1.0);
        let dst_h = (py_max - py_min).max(1.0);
        let footprint_x = image.width as f64 / dst_w;
        let footprint_y = image.height as f64 / dst_h;
        let smooth = if Self::magnifying(image, dst_w, dst_h) {
            smooth_mode
        } else if matches!(smooth_mode, SmoothMode::None) {
            SmoothMode::LegacyBilinear
        } else {
            smooth_mode
        };
        let use_area_average = footprint_x > 1.0 || footprint_y > 1.0;
        let (x0, x1, y0, y1) = clipped_bounds(buf, px_min, px_max, py_min, py_max);
        if x0 > x1 || y0 > y1 {
            return;
        }

        if image.channels == 3
            && image.bits_per_sample == 8
            && paint_alpha >= 1.0
            && matches!(smooth, SmoothMode::None)
            && buf.can_write_opaque_with_binary_clip()
            && Self::paint_axis_aligned_one_to_one_rgb_runs(
                buf, image, px_min, py_min, dst_w, dst_h, x0, x1, y0, y1,
            )
        {
            return;
        }

        if image.channels != 4
            && paint_alpha >= 1.0
            && matches!(smooth, SmoothMode::None)
            && footprint_x <= 1.0
            && footprint_y <= 1.0
            && (dst_w > image.width as f64 || dst_h > image.height as f64)
            && Self::paint_axis_aligned_nearest_runs(
                buf, image, px_min, py_min, dst_w, dst_h, x0, x1, y0, y1,
            )
        {
            return;
        }

        if matches!(smooth, SmoothMode::LegacyBilinear) && !use_area_average {
            Self::paint_axis_aligned_bilinear_precomputed(
                buf,
                image,
                px_min,
                py_min,
                dst_w,
                dst_h,
                x0,
                x1,
                y0,
                y1,
                paint_alpha,
            );
            return;
        }

        if image.channels != 4 && paint_alpha >= 1.0 && buf.can_write_opaque_unclipped() {
            for py in y0..=y1 {
                for px in x0..=x1 {
                    let u = (px as f64 + 0.5 - px_min) / dst_w;
                    let v = (py as f64 + 0.5 - py_min) / dst_h;
                    if !inside_unit_image_sample(u) || !inside_unit_image_sample(v) {
                        continue;
                    }
                    let sample = Self::sample(
                        image,
                        u,
                        v,
                        footprint_x,
                        footprint_y,
                        smooth,
                        use_area_average,
                    );
                    buf.write_opaque_pixel_unclipped(
                        px,
                        py,
                        [sample[0], sample[1], sample[2], 255],
                    );
                }
            }
            return;
        }

        for py in y0..=y1 {
            for px in x0..=x1 {
                let u = (px as f64 + 0.5 - px_min) / dst_w;
                let v = (py as f64 + 0.5 - py_min) / dst_h;
                if !inside_unit_image_sample(u) || !inside_unit_image_sample(v) {
                    continue;
                }
                let sample = Self::sample(
                    image,
                    u,
                    v,
                    footprint_x,
                    footprint_y,
                    smooth,
                    use_area_average,
                );
                let coverage = if image.channels == 4 {
                    sample[3] as f32 / 255.0
                } else {
                    1.0
                } * paint_alpha;
                buf.blend_pixel(px, py, [sample[0], sample[1], sample[2], 255], coverage);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_axis_aligned_bilinear_precomputed(
        buf: &mut PixelBuffer,
        image: &RawImage,
        px_min: f64,
        py_min: f64,
        dst_w: f64,
        dst_h: f64,
        x0: i32,
        x1: i32,
        y0: i32,
        y1: i32,
        paint_alpha: f32,
    ) {
        let x_samples = (x0..=x1)
            .map(|px| {
                let u = (px as f64 + 0.5 - px_min) / dst_w;
                Self::bilinear_axis_sample(u, image.width)
            })
            .collect::<Vec<_>>();
        let direct_binary_clip =
            image.channels != 4 && paint_alpha >= 1.0 && buf.can_write_opaque_with_binary_clip();
        let mut rgb_run = direct_binary_clip.then(|| Vec::with_capacity(x_samples.len() * 3));

        for py in y0..=y1 {
            let v = (py as f64 + 0.5 - py_min) / dst_h;
            let Some(y_sample) = Self::bilinear_axis_sample(v, image.height) else {
                continue;
            };
            if let Some(rgb_run) = rgb_run.as_mut() {
                let mut offset = 0usize;
                while offset < x_samples.len() {
                    while offset < x_samples.len() && x_samples[offset].is_none() {
                        offset += 1;
                    }
                    let run_start = offset;
                    rgb_run.clear();
                    while let Some(Some(x_sample)) = x_samples.get(offset) {
                        let sample = Self::bilinear_sample_precomputed(image, *x_sample, y_sample);
                        rgb_run.extend_from_slice(&sample[..3]);
                        offset += 1;
                    }
                    if !rgb_run.is_empty() {
                        buf.write_opaque_rgb_run_binary_clipped(
                            x0.saturating_add(run_start as i32),
                            py,
                            rgb_run,
                        );
                    }
                }
                continue;
            }
            for (offset, x_sample) in x_samples.iter().enumerate() {
                let Some(x_sample) = x_sample else {
                    continue;
                };
                let px = x0 + offset as i32;
                let sample = Self::bilinear_sample_precomputed(image, *x_sample, y_sample);
                let coverage = if image.channels == 4 {
                    sample[3] as f32 / 255.0
                } else {
                    1.0
                } * paint_alpha;
                buf.blend_pixel(px, py, [sample[0], sample[1], sample[2], 255], coverage);
            }
        }
    }

    #[inline]
    fn bilinear_axis_sample(normalized: f64, extent: u32) -> Option<BilinearAxisSample> {
        if extent == 0 || !inside_unit_image_sample(normalized) {
            return None;
        }
        let max = extent.saturating_sub(1) as usize;
        let source = (normalized * max as f64).clamp(0.0, max as f64);
        let low = source.floor() as usize;
        Some(BilinearAxisSample {
            low,
            high: (low + 1).min(max),
            fraction: (source - low as f64) as f32,
        })
    }

    #[inline]
    fn bilinear_sample_precomputed(
        image: &RawImage,
        x: BilinearAxisSample,
        y: BilinearAxisSample,
    ) -> [u8; 4] {
        let p00 = Self::get_pixel_channels(image, x.low, y.low);
        let p10 = Self::get_pixel_channels(image, x.high, y.low);
        let p01 = Self::get_pixel_channels(image, x.low, y.high);
        let p11 = Self::get_pixel_channels(image, x.high, y.high);
        let lerp = |v00: u8, v10: u8, v01: u8, v11: u8| -> u8 {
            let top = v00 as f32 * (1.0 - x.fraction) + v10 as f32 * x.fraction;
            let bottom = v01 as f32 * (1.0 - x.fraction) + v11 as f32 * x.fraction;
            (top * (1.0 - y.fraction) + bottom * y.fraction)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        [
            lerp(p00[0], p10[0], p01[0], p11[0]),
            lerp(p00[1], p10[1], p01[1], p11[1]),
            lerp(p00[2], p10[2], p01[2], p11[2]),
            lerp(p00[3], p10[3], p01[3], p11[3]),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_axis_aligned_one_to_one_rgb_runs(
        buf: &mut PixelBuffer,
        image: &RawImage,
        px_min: f64,
        py_min: f64,
        dst_w: f64,
        dst_h: f64,
        x0: i32,
        x1: i32,
        y0: i32,
        y1: i32,
    ) -> bool {
        if (dst_w - image.width as f64).abs() > 1e-6 || (dst_h - image.height as f64).abs() > 1e-6 {
            return false;
        }
        let Some(x_origin) = exact_integer_pixel(px_min) else {
            return false;
        };
        let Some(y_origin) = exact_integer_pixel(py_min) else {
            return false;
        };
        let stride = match (image.width as usize).checked_mul(3) {
            Some(stride) => stride,
            None => return false,
        };
        for py in y0..=y1 {
            let sy = py - y_origin;
            if sy < 0 || sy >= image.height as i32 {
                continue;
            }
            let sx0 = (x0 - x_origin).max(0).min(image.width as i32);
            let sx1 = (x1 - x_origin + 1).max(0).min(image.width as i32);
            if sx1 <= sx0 {
                continue;
            }
            let Some(row_start) = (sy as usize)
                .checked_mul(stride)
                .and_then(|row| row.checked_add(sx0 as usize * 3))
            else {
                return false;
            };
            let len = (sx1 - sx0) as usize * 3;
            let Some(row) = image.pixels.get(row_start..row_start + len) else {
                return false;
            };
            let written = buf.write_opaque_rgb_run_binary_clipped(x_origin + sx0, py, row);
            if written != (sx1 - sx0) as usize
                && buf.clip_mask().is_none_or(|clip| clip.is_all_visible())
            {
                return false;
            }
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_axis_aligned_nearest_runs(
        buf: &mut PixelBuffer,
        image: &RawImage,
        px_min: f64,
        py_min: f64,
        dst_w: f64,
        dst_h: f64,
        x0: i32,
        x1: i32,
        y0: i32,
        y1: i32,
    ) -> bool {
        if dst_w <= 0.0 || dst_h <= 0.0 || image.width == 0 || image.height == 0 {
            return false;
        }
        let source_w = image.width as f64;
        let source_h = image.height as f64;
        for py in y0..=y1 {
            let v = (py as f64 + 0.5 - py_min) / dst_h;
            if !inside_unit_image_sample(v) {
                continue;
            }
            let sy = (v * source_h).floor().clamp(0.0, source_h - 1.0) as usize;
            let mut px = x0;
            while px <= x1 {
                let u = (px as f64 + 0.5 - px_min) / dst_w;
                if !inside_unit_image_sample(u) {
                    px += 1;
                    continue;
                }
                let sx = (u * source_w).floor().clamp(0.0, source_w - 1.0) as usize;
                let mut end = px + 1;
                while end <= x1 {
                    let end_u = (end as f64 + 0.5 - px_min) / dst_w;
                    if !inside_unit_image_sample(end_u) {
                        break;
                    }
                    let end_sx = (end_u * source_w).floor().clamp(0.0, source_w - 1.0) as usize;
                    if end_sx != sx {
                        break;
                    }
                    end += 1;
                }
                let sample = Self::get_pixel_channels(image, sx, sy);
                buf.fill_rect(px, py, end - px, 1, [sample[0], sample[1], sample[2], 255]);
                px = end;
            }
        }
        true
    }

    fn paint_affine(
        buf: &mut PixelBuffer,
        image: &RawImage,
        combined: &Transform2D,
        smooth_mode: SmoothMode,
        paint_alpha: f32,
    ) {
        let inv = match combined.inverse() {
            Some(matrix) => matrix,
            None => {
                log::warn!("ImagePainter: singular transform, skipping image");
                return;
            }
        };

        let corners = [
            combined.transform_point(0.0, 0.0),
            combined.transform_point(1.0, 0.0),
            combined.transform_point(1.0, 1.0),
            combined.transform_point(0.0, 1.0),
        ];
        let (px_min, px_max, py_min, py_max) = bounding_box(&corners);
        if !px_min.is_finite() || !px_max.is_finite() || !py_min.is_finite() || !py_max.is_finite()
        {
            return;
        }

        let dst_w = (px_max - px_min).max(1.0);
        let dst_h = (py_max - py_min).max(1.0);
        let (footprint_x, footprint_y) = source_footprint_from_inverse(&inv, image);
        let smooth = if Self::magnifying(image, dst_w, dst_h) {
            smooth_mode
        } else if matches!(smooth_mode, SmoothMode::None) {
            SmoothMode::LegacyBilinear
        } else {
            smooth_mode
        };
        let use_area_average = footprint_x > 1.0 || footprint_y > 1.0;
        let (x0, x1, y0, y1) = clipped_bounds(buf, px_min, px_max, py_min, py_max);
        if x0 > x1 || y0 > y1 {
            return;
        }

        if image.channels != 4 && paint_alpha >= 1.0 && buf.can_write_opaque_unclipped() {
            for py in y0..=y1 {
                for px in x0..=x1 {
                    let (u, pdf_v) = inv.transform_point(px as f64 + 0.5, py as f64 + 0.5);
                    // PDF image samples are stored top row first while image
                    // space has its origin at the lower-left. Convert the
                    // inverse-mapped image-space ordinate to decoded row order.
                    let v = 1.0 - pdf_v;
                    if !inside_unit_image_sample(u) || !inside_unit_image_sample(v) {
                        continue;
                    }

                    let sample = Self::sample(
                        image,
                        u,
                        v,
                        footprint_x,
                        footprint_y,
                        smooth,
                        use_area_average,
                    );
                    buf.write_opaque_pixel_unclipped(
                        px,
                        py,
                        [sample[0], sample[1], sample[2], 255],
                    );
                }
            }
            return;
        }

        for py in y0..=y1 {
            for px in x0..=x1 {
                let (u, pdf_v) = inv.transform_point(px as f64 + 0.5, py as f64 + 0.5);
                let v = 1.0 - pdf_v;
                if !inside_unit_image_sample(u) || !inside_unit_image_sample(v) {
                    continue;
                }

                let sample = Self::sample(
                    image,
                    u,
                    v,
                    footprint_x,
                    footprint_y,
                    smooth,
                    use_area_average,
                );
                let coverage = if image.channels == 4 {
                    sample[3] as f32 / 255.0
                } else {
                    1.0
                } * paint_alpha;
                buf.blend_pixel(px, py, [sample[0], sample[1], sample[2], 255], coverage);
            }
        }
    }

    /// Nearest-neighbour sample at normalized coords. Used when magnifying so a
    /// small source image renders as crisp blocks (the PDF/Poppler default).
    pub fn nearest_sample(image: &RawImage, u: f64, v: f64) -> [u8; 4] {
        if image.width == 0 || image.height == 0 || image.channels == 0 || image.pixels.is_empty() {
            return [0, 0, 0, 0];
        }
        let w = image.width as f64;
        let h = image.height as f64;
        // Map [0,1) across the pixel grid and pick the covering source pixel.
        let x = (u * w).floor().clamp(0.0, w - 1.0) as usize;
        let y = (v * h).floor().clamp(0.0, h - 1.0) as usize;
        Self::get_pixel_channels(image, x, y)
    }

    /// Sample image at normalized coordinates using bilinear interpolation.
    pub fn bilinear_sample(image: &RawImage, u: f64, v: f64) -> [u8; 4] {
        if image.width == 0 || image.height == 0 || image.channels == 0 || image.pixels.is_empty() {
            return [0, 0, 0, 0];
        }

        let w = image.width as f64;
        let h = image.height as f64;
        let sx = (u * (w - 1.0)).clamp(0.0, (w - 1.0).max(0.0));
        let sy = (v * (h - 1.0)).clamp(0.0, (h - 1.0).max(0.0));

        let x0 = sx.floor() as usize;
        let y0 = sy.floor() as usize;
        let x1 = (x0 + 1).min(image.width.saturating_sub(1) as usize);
        let y1 = (y0 + 1).min(image.height.saturating_sub(1) as usize);
        let fx = (sx - x0 as f64) as f32;
        let fy = (sy - y0 as f64) as f32;

        let p00 = Self::get_pixel_channels(image, x0, y0);
        let p10 = Self::get_pixel_channels(image, x1, y0);
        let p01 = Self::get_pixel_channels(image, x0, y1);
        let p11 = Self::get_pixel_channels(image, x1, y1);

        let lerp2 = |v00: u8, v10: u8, v01: u8, v11: u8| -> u8 {
            let top = v00 as f32 * (1.0 - fx) + v10 as f32 * fx;
            let bottom = v01 as f32 * (1.0 - fx) + v11 as f32 * fx;
            (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
        };

        [
            lerp2(p00[0], p10[0], p01[0], p11[0]),
            lerp2(p00[1], p10[1], p01[1], p11[1]),
            lerp2(p00[2], p10[2], p01[2], p11[2]),
            lerp2(p00[3], p10[3], p01[3], p11[3]),
        ]
    }

    /// Source-footprint area averaging in default sRGB byte space. This is used
    /// for minification so high-resolution scans and photos are integrated
    /// instead of undersampled by a single bilinear lookup.
    pub fn area_average_sample(
        image: &RawImage,
        u: f64,
        v: f64,
        footprint_x: f64,
        footprint_y: f64,
    ) -> [u8; 4] {
        if image.width == 0 || image.height == 0 || image.channels == 0 || image.pixels.is_empty() {
            return [0, 0, 0, 0];
        }

        let w = image.width as f64;
        let h = image.height as f64;
        let cx = (u * w).clamp(0.0, w);
        let cy = (v * h).clamp(0.0, h);
        let half_w = (footprint_x.max(1e-6) * 0.5).min(w * 0.5);
        let half_h = (footprint_y.max(1e-6) * 0.5).min(h * 0.5);
        let x0 = snap_source_box_edge((cx - half_w).clamp(0.0, w), w);
        let x1 = snap_source_box_edge((cx + half_w).clamp(0.0, w), w);
        let y0 = snap_source_box_edge((cy - half_h).clamp(0.0, h), h);
        let y1 = snap_source_box_edge((cy + half_h).clamp(0.0, h), h);

        if x1 <= x0 || y1 <= y0 {
            return Self::nearest_sample(image, u, v);
        }

        let ix0 = x0.floor().max(0.0) as usize;
        let ix1 = x1.ceil().min(w) as usize;
        let iy0 = y0.floor().max(0.0) as usize;
        let iy1 = y1.ceil().min(h) as usize;

        let mut accum = [0.0_f64; 4];
        let mut total = 0.0_f64;
        for y in iy0..iy1 {
            let oy = ((y + 1) as f64).min(y1) - (y as f64).max(y0);
            if oy <= 0.0 {
                continue;
            }
            for x in ix0..ix1 {
                let ox = ((x + 1) as f64).min(x1) - (x as f64).max(x0);
                if ox <= 0.0 {
                    continue;
                }
                let weight = ox * oy;
                let px = Self::get_pixel_channels(image, x, y);
                for c in 0..4 {
                    accum[c] += px[c] as f64 * weight;
                }
                total += weight;
            }
        }

        if total <= 0.0 {
            return Self::nearest_sample(image, u, v);
        }

        [
            (accum[0] / total).round().clamp(0.0, 255.0) as u8,
            (accum[1] / total).round().clamp(0.0, 255.0) as u8,
            (accum[2] / total).round().clamp(0.0, 255.0) as u8,
            (accum[3] / total).round().clamp(0.0, 255.0) as u8,
        ]
    }

    /// Poppler-compatible smooth sample for `/Interpolate true` magnification.
    /// Uses edge-oriented bilinear coordinates, matching how Poppler spreads a
    /// 2-pixel image across the first two source-cell extents.
    pub fn interpolated_sample(image: &RawImage, u: f64, v: f64) -> [u8; 4] {
        if image.width == 0 || image.height == 0 || image.channels == 0 || image.pixels.is_empty() {
            return [0, 0, 0, 0];
        }

        let w = image.width as f64;
        let h = image.height as f64;
        let sx = (u * w).clamp(0.0, (w - 1.0).max(0.0));
        let sy = (v * h).clamp(0.0, (h - 1.0).max(0.0));
        let x0 = sx.floor() as usize;
        let y0 = sy.floor() as usize;
        let x1 = (x0 + 1).min(image.width.saturating_sub(1) as usize);
        let y1 = (y0 + 1).min(image.height.saturating_sub(1) as usize);
        let fx = sx - x0 as f64;
        let fy = sy - y0 as f64;

        let p00 = Self::get_pixel_channels(image, x0, y0);
        let p10 = Self::get_pixel_channels(image, x1, y0);
        let p01 = Self::get_pixel_channels(image, x0, y1);
        let p11 = Self::get_pixel_channels(image, x1, y1);

        let lerp2 = |v00: u8, v10: u8, v01: u8, v11: u8| -> u8 {
            let top = v00 as f64 * (1.0 - fx) + v10 as f64 * fx;
            let bottom = v01 as f64 * (1.0 - fx) + v11 as f64 * fx;
            (top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8
        };

        [
            lerp2(p00[0], p10[0], p01[0], p11[0]),
            lerp2(p00[1], p10[1], p01[1], p11[1]),
            lerp2(p00[2], p10[2], p01[2], p11[2]),
            lerp2(p00[3], p10[3], p01[3], p11[3]),
        ]
    }

    fn get_pixel_channels(image: &RawImage, x: usize, y: usize) -> [u8; 4] {
        match image.pixel(x, y) {
            [g] => {
                let g = *g;
                [g, g, g, 255]
            }
            [r, g, b] => [*r, *g, *b, 255],
            [r, g, b, a] => [*r, *g, *b, *a],
            _ => [0, 0, 0, 0],
        }
    }
}

fn source_footprint_from_inverse(inv: &Transform2D, image: &RawImage) -> (f64, f64) {
    let dx = inv.transform_vector(1.0, 0.0);
    let dy = inv.transform_vector(0.0, 1.0);
    let footprint_x = (dx.0.abs() + dy.0.abs()) * image.width as f64;
    let footprint_y = (dx.1.abs() + dy.1.abs()) * image.height as f64;
    (footprint_x.max(1e-6), footprint_y.max(1e-6))
}

#[derive(Clone, Copy, Debug)]
struct SourceBoxSpan {
    start: f64,
    finish: f64,
    first: usize,
    end: usize,
}

fn orthogonal_source_box_span(
    device_start: f64,
    device_finish: f64,
    mapping: OrthogonalSourceMapping,
    image: &RawImage,
) -> Option<SourceBoxSpan> {
    let source_extent = match mapping.axis {
        OrthogonalSourceAxis::X => image.width as usize,
        OrthogonalSourceAxis::Y => image.height as usize,
    };
    if !device_start.is_finite()
        || !device_finish.is_finite()
        || !mapping.scale.is_finite()
        || !mapping.offset.is_finite()
        || mapping.scale.abs() <= 1e-12
        || source_extent == 0
    {
        return None;
    }
    let extent = source_extent as f64;
    let first_edge = (mapping.scale * device_start + mapping.offset) * extent;
    let second_edge = (mapping.scale * device_finish + mapping.offset) * extent;
    let start = snap_source_box_edge(first_edge.min(second_edge).clamp(0.0, extent), extent);
    let finish = snap_source_box_edge(first_edge.max(second_edge).clamp(0.0, extent), extent);
    if finish <= start {
        return None;
    }
    let first = start.floor().max(0.0) as usize;
    let end = finish.ceil().min(extent) as usize;
    (end > first).then_some(SourceBoxSpan {
        start,
        finish,
        first,
        end,
    })
}

#[inline]
fn snap_source_box_edge(value: f64, extent: f64) -> f64 {
    let integer = value.round();
    let tolerance = 1e-9_f64.max(f64::EPSILON * extent.max(1.0) * 16.0);
    if (value - integer).abs() <= tolerance {
        integer.clamp(0.0, extent)
    } else {
        value
    }
}

#[inline]
fn box_pixel_weight(source_index: usize, start: f64, finish: f64) -> f64 {
    ((source_index + 1) as f64).min(finish) - (source_index as f64).max(start)
}

fn bounding_box(corners: &[(f64, f64); 4]) -> (f64, f64, f64, f64) {
    let px_min = corners
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::INFINITY, f64::min);
    let px_max = corners
        .iter()
        .map(|(x, _)| *x)
        .fold(f64::NEG_INFINITY, f64::max);
    let py_min = corners
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::INFINITY, f64::min);
    let py_max = corners
        .iter()
        .map(|(_, y)| *y)
        .fold(f64::NEG_INFINITY, f64::max);
    (px_min, px_max, py_min, py_max)
}

fn exact_integer_pixel(value: f64) -> Option<i32> {
    if !value.is_finite() {
        return None;
    }
    let rounded = value.round();
    if (value - rounded).abs() > 1e-6 || rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
        return None;
    }
    Some(rounded as i32)
}

fn clipped_bounds(
    buf: &PixelBuffer,
    px_min: f64,
    px_max: f64,
    py_min: f64,
    py_max: f64,
) -> (i32, i32, i32, i32) {
    if buf.width == 0 || buf.height == 0 {
        return (1, 0, 1, 0);
    }
    let x0 = floor_i32(px_min).max(0);
    let x1 = ceil_i32(px_max).min(buf.width as i32 - 1);
    let y0 = floor_i32(py_min).max(0);
    let y1 = ceil_i32(py_max).min(buf.height as i32 - 1);
    (x0, x1, y0, y1)
}

#[inline]
fn inside_unit_image_sample(value: f64) -> bool {
    value.is_finite() && (0.0..1.0).contains(&value)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::buffer::{ClipMask, BLACK, WHITE};

    fn rgb_2x2_image() -> RawImage {
        RawImage {
            width: 2,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
        }
    }

    #[test]
    fn bilinear_sample_on_2x2_image_corners() {
        let image = rgb_2x2_image();
        assert_eq!(
            &ImagePainter::bilinear_sample(&image, 0.0, 0.0)[..3],
            &[255, 0, 0]
        );
        assert_eq!(
            &ImagePainter::bilinear_sample(&image, 1.0, 0.0)[..3],
            &[0, 255, 0]
        );
        assert_eq!(
            &ImagePainter::bilinear_sample(&image, 0.0, 1.0)[..3],
            &[0, 0, 255]
        );
        assert_eq!(
            &ImagePainter::bilinear_sample(&image, 1.0, 1.0)[..3],
            &[255, 255, 0]
        );
    }

    #[test]
    fn bilinear_sample_at_center_blends_all_corners() {
        let image = RawImage {
            width: 2,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![200, 0, 0, 0, 200, 0, 0, 0, 200, 200, 200, 0],
        };
        let center = ImagePainter::bilinear_sample(&image, 0.5, 0.5);
        assert!((center[0] as i32 - 100).abs() <= 3);
    }

    #[test]
    fn precomputed_bilinear_coordinates_are_pixel_identical() {
        let image = rgb_2x2_image();
        for (u, v) in [
            (0.0, 0.0),
            (0.125, 0.875),
            (0.333, 0.667),
            (0.5, 0.5),
            (0.999, 0.001),
            (0.999_999, 0.999_999),
        ] {
            let x = ImagePainter::bilinear_axis_sample(u, image.width).expect("x sample");
            let y = ImagePainter::bilinear_axis_sample(v, image.height).expect("y sample");
            assert_eq!(
                ImagePainter::bilinear_sample_precomputed(&image, x, y),
                ImagePainter::bilinear_sample(&image, u, v),
                "sample mismatch at ({u}, {v})"
            );
        }
    }

    #[test]
    fn bilinear_sample_gray_image_replicates_channels() {
        let image = RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![0, 255],
        };
        let left = ImagePainter::bilinear_sample(&image, 0.0, 0.5);
        let right = ImagePainter::bilinear_sample(&image, 1.0, 0.5);
        assert_eq!(&left[..3], &[0, 0, 0]);
        assert_eq!(&right[..3], &[255, 255, 255]);
        assert_eq!(left[3], 255);
        assert_eq!(right[3], 255);
    }

    #[test]
    fn bilinear_sample_rgba_image_preserves_alpha() {
        let image = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 128],
        };
        let sample = ImagePainter::bilinear_sample(&image, 0.5, 0.5);
        assert_eq!(sample, [255, 0, 0, 128]);
    }

    #[test]
    fn area_average_downscales_checkerboard_to_gray() {
        let mut pixels = Vec::new();
        for y in 0..4 {
            for x in 0..4 {
                pixels.push(if (x + y) % 2 == 0 { 0 } else { 255 });
            }
        }
        let image = RawImage {
            width: 4,
            height: 4,
            channels: 1,
            bits_per_sample: 8,
            pixels,
        };

        let sample = ImagePainter::area_average_sample(&image, 0.5, 0.5, 4.0, 4.0);
        for channel in sample.iter().take(3) {
            assert!(
                (*channel as i32 - 128).abs() <= 1,
                "checkerboard should integrate to gray, got {sample:?}"
            );
        }
        assert_eq!(sample[3], 255);
    }

    #[test]
    fn integer_target_requires_exact_device_phase() {
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::translation(1.0, 2.0).concat(&Transform2D::scale(12.0, 8.0));
        let target = ImagePainter::axis_aligned_integer_target(&ctm, &viewport)
            .expect("integer phase target");
        assert_eq!(target.width, 12);
        assert_eq!(target.height, 8);

        let fractional =
            Transform2D::translation(1.025, 2.0).concat(&Transform2D::scale(12.0, 8.0));
        assert!(ImagePainter::axis_aligned_integer_target(&fractional, &viewport).is_none());

        let rotated_viewport = Viewport::new_rotated([0.0, 0.0, 100.0, 100.0], 72, 90);
        assert!(
            ImagePainter::axis_aligned_integer_target(&ctm, &rotated_viewport).is_none(),
            "a page-rotated image must retain its orientation through the affine sampler"
        );
        assert!(
            ImagePainter::axis_aligned_cache_target(&fractional, &viewport).is_some(),
            "fractional axis-aligned images should retain their sampling phase in cache"
        );
        assert!(
            ImagePainter::axis_aligned_cache_target(&ctm, &rotated_viewport).is_some(),
            "quarter-turn images should retain their source orientation in the cache target"
        );
    }

    #[test]
    fn rotated_cached_reduction_matches_general_sampler() {
        let viewport = Viewport::new_rotated([0.0, 0.0, 10.0, 10.0], 72, 90);
        let ctm = Transform2D::scale(10.0, 10.0);
        let mut pixels = Vec::with_capacity(20 * 20 * 3);
        for y in 0..20 {
            for x in 0..20 {
                pixels.extend_from_slice(&[(x * 11) as u8, (y * 11) as u8, ((x + y) * 5) as u8]);
            }
        }
        let image = RawImage {
            width: 20,
            height: 20,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        };
        let mut general = PixelBuffer::new_filled(10, 10, WHITE);
        ImagePainter::paint_image(&mut general, &image, &ctm, &viewport);

        let target = ImagePainter::axis_aligned_cache_target(&ctm, &viewport)
            .expect("orthogonal cache target");
        let scaled =
            ImagePainter::scale_axis_aligned_default_rgb_for_target(&image, &target, false)
                .expect("orthogonal cached reduction");
        let mut cached = PixelBuffer::new_filled(10, 10, WHITE);
        assert!(ImagePainter::paint_scaled_rgb_at_device_target(
            &mut cached,
            &scaled,
            target.paint,
            1.0,
        ));

        assert_eq!(cached.to_rgba_bytes(), general.to_rgba_bytes());
    }

    #[test]
    fn fractional_cached_reduction_matches_general_sampler() {
        let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        let ctm = Transform2D::translation(-0.08, -0.04).concat(&Transform2D::scale(10.16, 10.08));
        let mut pixels = Vec::with_capacity(20 * 20 * 3);
        for y in 0..20 {
            for x in 0..20 {
                let value: u8 = if (x + y) % 2 == 0 { 18 } else { 238 };
                pixels.extend_from_slice(&[
                    value,
                    value.saturating_add(3),
                    value.saturating_sub(3),
                ]);
            }
        }
        let image = RawImage {
            width: 20,
            height: 20,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        };
        let mut general = PixelBuffer::new_filled(10, 10, WHITE);
        ImagePainter::paint_image(&mut general, &image, &ctm, &viewport);

        let target = ImagePainter::axis_aligned_cache_target(&ctm, &viewport)
            .expect("fractional cache target");
        let scaled =
            ImagePainter::scale_axis_aligned_default_rgb_for_target(&image, &target, false)
                .expect("phase-aware reduction");
        let mut cached = PixelBuffer::new_filled(10, 10, WHITE);
        assert!(ImagePainter::paint_scaled_rgb_at_device_target(
            &mut cached,
            &scaled,
            target.paint,
            1.0,
        ));

        assert_eq!(cached.to_rgba_bytes(), general.to_rgba_bytes());
    }

    #[test]
    fn rotated_viewport_preserves_pdf_image_row_orientation() {
        // Decoded image rows are top-to-bottom: red/green, then blue/yellow.
        // A clockwise page rotation produces blue/red, then yellow/green.
        let image = rgb_2x2_image();
        let viewport = Viewport::new_rotated([0.0, 0.0, 2.0, 2.0], 72, 90);
        let ctm = Transform2D::scale(2.0, 2.0);
        let mut buf = PixelBuffer::new_filled(2, 2, WHITE);

        ImagePainter::paint_image(&mut buf, &image, &ctm, &viewport);

        assert_eq!(buf.get_pixel(0, 0), [0, 0, 255, 255]);
        assert_eq!(buf.get_pixel(1, 0), [255, 0, 0, 255]);
        assert_eq!(buf.get_pixel(0, 1), [255, 255, 0, 255]);
        assert_eq!(buf.get_pixel(1, 1), [0, 255, 0, 255]);
    }

    #[test]
    fn scale_axis_aligned_default_rgb_matches_center_samples() {
        let image = rgb_2x2_image();
        let scaled = ImagePainter::scale_axis_aligned_default_rgb(&image, 4, 4, false)
            .expect("scaled RGB image");
        assert_eq!(scaled.width, 4);
        assert_eq!(scaled.height, 4);
        assert_eq!(scaled.channels, 3);
        assert_eq!(&scaled.pixels[0..3], &[255, 0, 0]);
        let bottom_right = (3 * 4 + 3) * 3;
        assert_eq!(
            &scaled.pixels[bottom_right..bottom_right + 3],
            &[255, 255, 0]
        );
    }

    #[test]
    fn paint_scaled_rgb_at_device_target_uses_binary_clip() {
        let image = ImagePainter::scale_axis_aligned_default_rgb(&rgb_2x2_image(), 2, 2, false)
            .expect("scaled RGB image");
        let mut buf = PixelBuffer::new_filled(4, 4, WHITE);
        let mut clip = ClipMask::empty(4, 4);
        clip.set(1, 1, true);
        buf.set_clip(clip);
        let target = AxisAlignedImageTarget {
            x_origin: 1,
            y_origin: 1,
            width: 2,
            height: 2,
        };
        assert!(ImagePainter::paint_scaled_rgb_at_device_target(
            &mut buf, &image, target, 1.0
        ));
        assert_eq!(buf.get_pixel(1, 1), [255, 0, 0, 255]);
        assert_eq!(buf.get_pixel(2, 1), WHITE);
    }

    #[test]
    fn high_quality_minified_image_paint_uses_area_average() {
        let mut pixels = Vec::new();
        for y in 0..4 {
            for x in 0..4 {
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                pixels.extend_from_slice(&[v, v, v]);
            }
        }
        let image = RawImage {
            width: 4,
            height: 4,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        };
        let vp = Viewport::new([0.0, 0.0, 2.0, 2.0], 72);
        let ctm = Transform2D::new(2.0, 0.0, 0.0, 2.0, 0.0, 0.0);
        let mut buf = PixelBuffer::new_filled_with_mode(
            2,
            2,
            WHITE,
            crate::render::buffer::RenderMode::HighQuality,
        );

        ImagePainter::paint_image(&mut buf, &image, &ctm, &vp);

        for y in 0..2 {
            for x in 0..2 {
                let pixel = buf.get_pixel(x, y);
                assert!(
                    (pixel[0] as i32 - 128).abs() <= 1,
                    "downscaled pixel ({x},{y}) should be gray, got {pixel:?}"
                );
            }
        }
    }

    #[test]
    fn interpolate_true_smooths_magnified_image() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 10.0], 72);
        let ctm = Transform2D::new(100.0, 0.0, 0.0, 10.0, 0.0, 0.0);
        let image = RawImage {
            width: 2,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0, 0, 0, 255, 255, 255],
        };
        let mut crisp = PixelBuffer::new_filled(100, 10, WHITE);
        let mut smooth = PixelBuffer::new_filled(100, 10, WHITE);

        ImagePainter::paint_image(&mut crisp, &image, &ctm, &vp);
        ImagePainter::paint_image_with_options(&mut smooth, &image, &ctm, &vp, true);

        assert_eq!(
            crisp.get_pixel(49, 5)[0],
            0,
            "default /Interpolate false should stay nearest on the left of seam"
        );
        assert_eq!(
            crisp.get_pixel(50, 5)[0],
            255,
            "default /Interpolate false should stay nearest on the right of seam"
        );
        let edge = smooth.get_pixel(25, 5)[0];
        assert!(
            (32..=223).contains(&edge),
            "/Interpolate true should smooth the magnified seam, got {edge}"
        );
    }

    #[test]
    fn paint_image_places_pixels_in_correct_region() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let image = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0],
        };
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 25.0, 25.0);
        ImagePainter::paint_image(&mut buf, &image, &ctm, &vp);
        let center = buf.get_pixel(50, 50);
        println!("paint_image center pixel: {:?}", center);
        assert!(center[0] > 200);
        assert_eq!(buf.get_pixel(1, 1), WHITE);
    }

    #[test]
    fn axis_aligned_image_does_not_extend_past_unit_square() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let image = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0],
        };
        let ctm = Transform2D::new(50.0, 0.0, 0.0, 50.0, 25.0, 25.0);

        ImagePainter::paint_image(&mut buf, &image, &ctm, &vp);

        assert!(buf.get_pixel(74, 50)[0] > 200);
        assert_eq!(buf.get_pixel(75, 50), WHITE);
        assert_eq!(buf.get_pixel(50, 75), WHITE);
    }

    #[test]
    fn paint_image_with_zero_size_image_does_not_panic() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let ctm = Transform2D::identity();
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let empty = RawImage {
            width: 0,
            height: 0,
            channels: 3,
            bits_per_sample: 8,
            pixels: Vec::new(),
        };
        ImagePainter::paint_image(&mut buf, &empty, &ctm, &vp);
    }

    #[test]
    fn paint_image_with_short_buffer_does_not_synthesize_black_pixels() {
        let vp = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        let ctm = Transform2D::new(10.0, 0.0, 0.0, 10.0, 0.0, 0.0);
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        let malformed = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0],
        };

        ImagePainter::paint_image(&mut buf, &malformed, &ctm, &vp);

        assert_eq!(buf.get_pixel(5, 5), WHITE);
    }

    #[test]
    fn sample_short_buffer_is_transparent_not_black() {
        let malformed = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0],
        };

        assert_eq!(
            ImagePainter::nearest_sample(&malformed, 0.5, 0.5),
            [0, 0, 0, 0]
        );
        assert_eq!(
            ImagePainter::interpolated_sample(&malformed, 0.5, 0.5),
            [0, 0, 0, 0]
        );
    }

    #[test]
    fn sample_unsupported_channels_is_transparent_not_black() {
        let malformed = RawImage {
            width: 1,
            height: 1,
            channels: 2,
            bits_per_sample: 8,
            pixels: vec![0, 0],
        };

        assert_eq!(
            ImagePainter::nearest_sample(&malformed, 0.5, 0.5),
            [0, 0, 0, 0]
        );
    }

    #[test]
    fn paint_image_rgba_uses_alpha_channel() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut buf_opaque = PixelBuffer::new_filled(100, 100, WHITE);
        let mut buf_transp = PixelBuffer::new_filled(100, 100, WHITE);
        let ctm = Transform2D::new(100.0, 0.0, 0.0, 100.0, 0.0, 0.0);
        let opaque = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 255],
        };
        let transparent = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0],
        };
        ImagePainter::paint_image(&mut buf_opaque, &opaque, &ctm, &vp);
        ImagePainter::paint_image(&mut buf_transp, &transparent, &ctm, &vp);
        assert!(buf_opaque.get_pixel(50, 50)[0] > 200);
        assert_eq!(buf_transp.get_pixel(50, 50), WHITE);
    }

    #[test]
    fn paint_image_with_alpha_multiplies_source_coverage() {
        let vp = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
        let mut buf = PixelBuffer::new_filled(10, 10, WHITE);
        let ctm = Transform2D::new(10.0, 0.0, 0.0, 10.0, 0.0, 0.0);
        let image = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0, 0, 255],
        };

        ImagePainter::paint_image_with_alpha(&mut buf, &image, &ctm, &vp, 0.5);

        let center = buf.get_pixel(5, 5);
        assert!(
            (center[0] as i32 - 128).abs() <= 1,
            "half-alpha blue over white should retain half red, got {center:?}"
        );
        assert!(
            (center[1] as i32 - 128).abs() <= 1,
            "half-alpha blue over white should retain half green, got {center:?}"
        );
        assert_eq!(center[2], 255);
    }

    #[test]
    fn bilinear_sample_empty_image_is_transparent() {
        let image = RawImage {
            width: 0,
            height: 0,
            channels: 0,
            bits_per_sample: 8,
            pixels: Vec::new(),
        };
        assert_eq!(
            ImagePainter::bilinear_sample(&image, 0.5, 0.5),
            [0, 0, 0, 0]
        );
    }

    #[test]
    fn paint_image_singular_transform_skips_gracefully() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let image = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0, 0, 0],
        };
        let ctm = Transform2D::new(0.0, 0.0, 0.0, 0.0, 10.0, 10.0);
        ImagePainter::paint_image(&mut buf, &image, &ctm, &vp);
        assert_eq!(buf.get_pixel(10, 10), WHITE);
        assert_eq!(buf.get_pixel(0, 0), WHITE);
    }

    // Regression (Benchmark Fix B): a small image MAGNIFIED to a larger area
    // must render as crisp nearest-neighbour blocks (the PDF/Poppler default when
    // /Interpolate is absent), NOT a bilinearly-smoothed gradient. A 2x2
    // image scaled to fill a 100x100 page must have a sharp seam between cells,
    // with interior pixels exactly equal to a source pixel (no blend).
    #[test]
    fn magnified_image_uses_nearest_neighbour_blocks() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let image = RawImage {
            width: 2,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            // TL=red, TR=green, BL=blue, BR=yellow
            pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
        };
        let ctm = Transform2D::new(100.0, 0.0, 0.0, 100.0, 0.0, 0.0);
        ImagePainter::paint_image(&mut buf, &image, &ctm, &vp);
        // Deep inside each quadrant the pixel must be exactly one source colour
        // (a smoothed gradient would blend toward neighbours). Sample near each
        // corner, away from the central seam. Device y is flipped (top = y small).
        let tl = buf.get_pixel(12, 12);
        let tr = buf.get_pixel(88, 12);
        let bl = buf.get_pixel(12, 88);
        let br = buf.get_pixel(88, 88);
        // Each must be a pure primary (one channel 255, others 0) or yellow — i.e.
        // NOT a blended intermediate. Check no channel is a mid value.
        for (label, p) in [("tl", tl), ("tr", tr), ("bl", bl), ("br", br)] {
            for (c, &v) in p.iter().take(3).enumerate() {
                assert!(
                    v == 0 || v == 255,
                    "{label} channel {c} = {v}: magnified image must be crisp (0 or 255), not blended"
                );
            }
        }
    }

    #[test]
    fn magnified_nearest_run_fast_path_preserves_source_cells() {
        let mut buf = PixelBuffer::new_filled(18, 12, WHITE);
        let image = RawImage {
            width: 3,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![
                255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 0, 255, 255, 255, 0, 255,
            ],
        };

        assert!(ImagePainter::paint_axis_aligned_nearest_runs(
            &mut buf, &image, 0.0, 0.0, 18.0, 12.0, 0, 17, 0, 11,
        ));

        assert_eq!(&buf.get_pixel(0, 0)[..3], &[255, 0, 0]);
        assert_eq!(&buf.get_pixel(6, 0)[..3], &[0, 255, 0]);
        assert_eq!(&buf.get_pixel(12, 0)[..3], &[0, 0, 255]);
        assert_eq!(&buf.get_pixel(0, 6)[..3], &[255, 255, 0]);
        assert_eq!(&buf.get_pixel(6, 6)[..3], &[0, 255, 255]);
        assert_eq!(&buf.get_pixel(12, 6)[..3], &[255, 0, 255]);
    }

    #[test]
    fn one_to_one_rgb_fast_path_writes_exact_source_rows() {
        let mut buf = PixelBuffer::new_filled(6, 5, WHITE);
        let image = RawImage {
            width: 3,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![
                255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 0, 255, 255, 255, 0, 255,
            ],
        };

        assert!(ImagePainter::paint_axis_aligned_one_to_one_rgb_runs(
            &mut buf, &image, 2.0, 1.0, 3.0, 2.0, 0, 5, 0, 4,
        ));

        assert_eq!(&buf.get_pixel(2, 1)[..3], &[255, 0, 0]);
        assert_eq!(&buf.get_pixel(3, 1)[..3], &[0, 255, 0]);
        assert_eq!(&buf.get_pixel(4, 1)[..3], &[0, 0, 255]);
        assert_eq!(&buf.get_pixel(2, 2)[..3], &[255, 255, 0]);
        assert_eq!(&buf.get_pixel(3, 2)[..3], &[0, 255, 255]);
        assert_eq!(&buf.get_pixel(4, 2)[..3], &[255, 0, 255]);
        assert_eq!(buf.get_pixel(1, 1), WHITE);
        assert_eq!(buf.get_pixel(5, 2), WHITE);
    }

    #[test]
    fn one_to_one_rgb_fast_path_respects_binary_clip_runs() {
        let mut buf = PixelBuffer::new_filled(5, 4, WHITE);
        let mut clip = ClipMask::empty(5, 4);
        clip.set(2, 1, true);
        clip.set(4, 1, true);
        clip.set(3, 2, true);
        buf.set_clip(clip);
        let image = RawImage {
            width: 3,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![
                255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 0, 255, 255, 255, 0, 255,
            ],
        };

        assert!(ImagePainter::paint_axis_aligned_one_to_one_rgb_runs(
            &mut buf, &image, 2.0, 1.0, 3.0, 2.0, 0, 4, 0, 3,
        ));

        assert_eq!(&buf.get_pixel(2, 1)[..3], &[255, 0, 0]);
        assert_eq!(buf.get_pixel(3, 1), WHITE);
        assert_eq!(&buf.get_pixel(4, 1)[..3], &[0, 0, 255]);
        assert_eq!(buf.get_pixel(2, 2), WHITE);
        assert_eq!(&buf.get_pixel(3, 2)[..3], &[0, 255, 255]);
        assert_eq!(buf.get_pixel(4, 2), WHITE);
    }

    #[test]
    fn bilinear_rgb_row_fast_path_preserves_samples_and_binary_clip() {
        let mut buf = PixelBuffer::new_filled(3, 1, WHITE);
        let mut clip = ClipMask::empty(3, 1);
        clip.set(0, 0, true);
        clip.set(2, 0, true);
        buf.set_clip(clip);
        let image = RawImage {
            width: 4,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
        };

        ImagePainter::paint_axis_aligned_bilinear_precomputed(
            &mut buf, &image, 0.0, 0.0, 3.0, 1.0, 0, 2, 0, 0, 1.0,
        );

        let y_sample = ImagePainter::bilinear_axis_sample(0.5, 1).expect("y sample");
        for x in [0, 2] {
            let u = (x as f64 + 0.5) / 3.0;
            let x_sample = ImagePainter::bilinear_axis_sample(u, 4).expect("x sample");
            let expected = ImagePainter::bilinear_sample_precomputed(&image, x_sample, y_sample);
            assert_eq!(&buf.get_pixel(x, 0)[..3], &expected[..3]);
        }
        assert_eq!(buf.get_pixel(1, 0), WHITE);
    }

    #[test]
    fn paint_rotated_image_affine_path_draws_pixels() {
        let vp = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let mut buf = PixelBuffer::new_filled(100, 100, WHITE);
        let image = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: BLACK[..3].to_vec(),
        };
        let ctm = Transform2D::scale(30.0, 30.0)
            .concat(&Transform2D::rotation(std::f64::consts::FRAC_PI_4))
            .concat(&Transform2D::translation(50.0, 50.0));
        ImagePainter::paint_image(&mut buf, &image, &ctm, &vp);
        let dark = (0..100i32)
            .flat_map(|y| (0..100i32).map(move |x| (x, y)))
            .any(|(x, y)| buf.get_pixel(x, y)[0] < 200);
        assert!(dark);
    }

    #[test]
    fn box_minification_averages_halftone_without_aliasing() {
        let mut pixels = Vec::with_capacity(8 * 8);
        for y in 0..8 {
            for x in 0..8 {
                pixels.push(if (x + y) % 2 == 0 { 0 } else { 255 });
            }
        }
        let image = RawImage {
            width: 8,
            height: 8,
            channels: 1,
            bits_per_sample: 8,
            pixels,
        };

        let reduced = ImagePainter::scale_axis_aligned_default_rgb(&image, 1, 1, false)
            .expect("box-reduced RGB image");

        assert_eq!(reduced.width, 1);
        assert_eq!(reduced.height, 1);
        assert_eq!(reduced.channels, 3);
        assert_eq!(reduced.pixels, vec![128, 128, 128]);
    }
}
