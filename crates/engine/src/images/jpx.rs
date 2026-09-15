//! JPXDecode (JPEG 2000) image decoding.
//!
//! # Approach
//!
//! JPEG 2000 is a wavelet-based codec that is substantially more complex than
//! DCT (JPEG) or even JBIG2. Rather than implement a from-scratch decoder, we
//! integrate the pure-Rust [`hayro-jpeg2000`] crate. This is the same family of
//! crates already used for CCITT (`hayro-ccitt`) and JBIG2 (`hayro-jbig2`), so
//! it satisfies the hard "no C/C++ toolchain" constraint: the crate is
//! `#![forbid(unsafe_code)]` and `no_std`-compatible, and we pull it in with
//! `default-features = false` plus `std` (no `simd`/`image` features) so there
//! is no dependency on a C compiler, cmake, or `links =` native library.
//!
//! `hayro-jpeg2000` decodes both raw JPEG 2000 codestreams (the common case for
//! PDF embedding, magic `FF 4F FF 51`) and full JP2 container files (magic
//! `00 00 00 0C 6A 50 20 20`). It selects the right path internally based on the
//! leading bytes, so the adapter does not need to detect or strip the JP2 box
//! wrapper itself.
//!
//! # Supported subset
//!
//! The underlying crate supports the vast majority of the JPEG 2000 core coding
//! system (ISO/IEC 15444-1): both the 5/3 reversible and 9/7 irreversible
//! wavelet filters, all progression orders, multiple tiles and resolution
//! levels, and palette-indexed images. It also handles several ISO/IEC 15444-2
//! color-space extensions. Color spaces surfaced here are grayscale, RGB, CMYK,
//! and ICC-based / unknown (handled by channel count). Anything the crate cannot
//! decode (e.g. progression-order changes inside tile-parts) surfaces as an
//! [`WellfriendError`] rather than a panic, matching the CCITT/JBIG2 error contract.
//!
//! [`hayro-jpeg2000`]: https://crates.io/crates/hayro-jpeg2000

use hayro_jpeg2000::{ColorSpace, DecodeSettings, Image};

use crate::error::{Result, WellfriendError};
use crate::filters::DecodeLimits;
use crate::images::decoder::{ensure_decode_budget, ColorSpaceConverter, RawImage};

pub(crate) struct DecodedJpx {
    pub raw: RawImage,
    pub original_width: u32,
    pub original_height: u32,
}

/// JPX color-space family visible from codestream/container metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JpxMetadataColorSpace {
    Gray,
    Rgb,
    Cmyk,
    Icc,
    Unknown,
}

/// Metadata that can be inspected from a JPX stream without decoding pixels.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct JpxMetadata {
    pub width: u32,
    pub height: u32,
    pub original_bit_depth: u8,
    pub color_space: JpxMetadataColorSpace,
    pub color_channels: u8,
    pub stored_channels: u8,
    pub has_alpha: bool,
}

/// Decode a PDF-embedded JPEG 2000 (`JPXDecode`) stream into a `RawImage`.
///
/// `data` is the raw stream bytes after all preceding (non-image) filters have
/// been applied. It may be either a raw J2K codestream (the common PDF case) or
/// a JP2-wrapped file; both are handled. The output is always 8-bit, with
/// channels interleaved. Grayscale and RGB images retain native color channels,
/// CMYK is converted to RGB, and JPX-internal alpha channels are preserved as
/// RGBA. PDF image soft masks are still handled separately by the SMask pipeline.
pub fn decode(data: &[u8]) -> Result<RawImage> {
    decode_with_limits(data, &DecodeLimits::default())
}

pub(crate) fn decode_with_limits(data: &[u8], limits: &DecodeLimits) -> Result<RawImage> {
    validate_jpx_limits(data, limits)?;
    decode_with_settings(data, DecodeSettings::default(), limits)
}

/// Inspect JPX dimensions, bit depth, color family, channel count, and alpha
/// state without decoding pixels.
pub fn inspect_metadata(data: &[u8]) -> Result<JpxMetadata> {
    let image = Image::new(data, &DecodeSettings::default())
        .map_err(|err| WellfriendError::MalformedPdf(format!("JPXDecode parse failed: {err}")))?;
    metadata_from_image(&image)
}

pub(crate) fn decode_with_target_resolution(
    data: &[u8],
    target_resolution: Option<(u32, u32)>,
    limits: &DecodeLimits,
) -> Result<DecodedJpx> {
    validate_jpx_limits(data, limits)?;
    let original = Image::new(data, &DecodeSettings::default())
        .map_err(|err| WellfriendError::MalformedPdf(format!("JPXDecode parse failed: {err}")))?;
    let original_width = original.width();
    let original_height = original.height();
    let settings = DecodeSettings {
        target_resolution,
        ..DecodeSettings::default()
    };
    let raw = decode_with_settings(data, settings, limits)?;
    Ok(DecodedJpx {
        raw,
        original_width,
        original_height,
    })
}

fn decode_with_settings(
    data: &[u8],
    settings: DecodeSettings,
    limits: &DecodeLimits,
) -> Result<RawImage> {
    let image = Image::new(data, &settings)
        .map_err(|err| WellfriendError::MalformedPdf(format!("JPXDecode parse failed: {err}")))?;

    let metadata = metadata_from_image(&image)?;
    let width = metadata.width;
    let height = metadata.height;
    let has_alpha = image.has_alpha();
    let color_space = image.color_space().clone();
    let color_channels = metadata.color_channels;
    let stored_channels = metadata.stored_channels;

    validate_decoded_shape(&metadata, limits)?;

    let decoded = image
        .decode()
        .map_err(|err| WellfriendError::MalformedPdf(format!("JPXDecode failed: {err}")))?;
    ensure_exact_jpx_decoded_len(width, height, stored_channels, decoded.len())?;

    let color_channels = usize::from(color_channels);
    let stored_channels = usize::from(stored_channels);

    let (color_only, alpha) = if has_alpha {
        split_trailing_alpha(&decoded, stored_channels, color_channels)
    } else {
        (decoded, None)
    };

    let (pixels, channels) = match color_space {
        ColorSpace::Gray => match alpha {
            Some(alpha) => (gray_alpha_to_rgba(width, height, &color_only, &alpha)?, 4u8),
            None => (color_only, 1u8),
        },
        ColorSpace::RGB => match alpha {
            Some(alpha) => (
                attach_alpha_to_rgb(width, height, &color_only, &alpha)?,
                4u8,
            ),
            None => (color_only, 3u8),
        },
        ColorSpace::CMYK => {
            let rgb = ColorSpaceConverter::cmyk_to_rgb(&color_only);
            match alpha {
                Some(alpha) => (attach_alpha_to_rgb(width, height, &rgb, &alpha)?, 4u8),
                None => (rgb, 3u8),
            }
        }
        // ICC-based and unknown color spaces are surfaced by their raw channel
        // count. 1 -> gray, 3 -> RGB, 4 -> treat as CMYK. Other channel counts
        // have no renderer color semantics and must fail typed instead of being
        // passed to downstream samplers that would paint fallback black pixels.
        ColorSpace::Icc { num_channels, .. } | ColorSpace::Unknown { num_channels } => {
            finish_icc_or_unknown_color_space(width, height, num_channels, color_only, alpha)?
        }
    };

    let raw = RawImage {
        width,
        height,
        channels,
        bits_per_sample: 8,
        pixels,
    };

    ensure_exact_jpx_length(raw)
}

fn validate_decoded_shape(metadata: &JpxMetadata, limits: &DecodeLimits) -> Result<()> {
    if metadata.width > limits.max_image_width || metadata.height > limits.max_image_height {
        return Err(WellfriendError::ResourceLimit(format!(
            "JPXDecode dimensions {}x{} exceed limit {}x{}",
            metadata.width, metadata.height, limits.max_image_width, limits.max_image_height
        )));
    }
    if usize::from(metadata.stored_channels) > limits.max_jpx_components {
        return Err(WellfriendError::ResourceLimit(format!(
            "JPXDecode component count {} exceeds limit {}",
            metadata.stored_channels, limits.max_jpx_components
        )));
    }
    let pixels = u64::from(metadata.width)
        .checked_mul(u64::from(metadata.height))
        .ok_or_else(|| WellfriendError::ResourceLimit("JPXDecode pixel count overflows".into()))?;
    if pixels > limits.max_image_pixels {
        return Err(WellfriendError::ResourceLimit(format!(
            "JPXDecode pixel count {pixels} exceeds limit {}",
            limits.max_image_pixels
        )));
    }
    let decoded_bytes = pixels
        .checked_mul(u64::from(metadata.stored_channels))
        .ok_or_else(|| WellfriendError::ResourceLimit("JPXDecode byte count overflows".into()))?;
    if decoded_bytes > limits.max_image_decoded_bytes {
        return Err(WellfriendError::ResourceLimit(format!(
            "JPXDecode output {decoded_bytes} bytes exceeds limit {}",
            limits.max_image_decoded_bytes
        )));
    }
    Ok(())
}

fn validate_jpx_limits(data: &[u8], limits: &DecodeLimits) -> Result<()> {
    let codestream = jpx_codestream(data).ok_or_else(|| {
        WellfriendError::MalformedPdf("JPXDecode codestream box is missing or malformed".into())
    })?;
    let mut offset = 2usize;
    let mut components = None;
    let mut tile_part_end = None;
    let mut unbounded_final_tile_part = false;
    while offset + 2 <= codestream.len() {
        if codestream[offset] != 0xff {
            return Err(WellfriendError::MalformedPdf(
                "JPXDecode marker prefix is missing".into(),
            ));
        }
        while offset < codestream.len() && codestream[offset] == 0xff {
            offset += 1;
        }
        if offset >= codestream.len() {
            break;
        }
        let marker_start = offset - 1;
        let marker = codestream[offset];
        offset += 1;
        if marker == 0xd9 {
            break;
        }
        if marker == 0x4f {
            continue;
        }
        if marker == 0x93 {
            if unbounded_final_tile_part {
                break;
            }
            let Some(end) = tile_part_end.take() else {
                return Err(WellfriendError::MalformedPdf(
                    "JPXDecode SOD appears without a bounded SOT".into(),
                ));
            };
            if end < offset || end > codestream.len() {
                return Err(WellfriendError::MalformedPdf(
                    "JPXDecode tile-part length exceeds codestream".into(),
                ));
            }
            offset = end;
            continue;
        }
        if offset + 2 > codestream.len() {
            return Err(WellfriendError::MalformedPdf(
                "JPXDecode marker segment is truncated".into(),
            ));
        }
        let length = usize::from(u16::from_be_bytes([
            codestream[offset],
            codestream[offset + 1],
        ]));
        if length < 2 || offset + length > codestream.len() {
            return Err(WellfriendError::MalformedPdf(
                "JPXDecode marker length exceeds codestream".into(),
            ));
        }
        let segment = &codestream[offset..offset + length];
        match marker {
            0x90 if segment.len() >= 10 => {
                let tile_length = usize::try_from(be_u32(segment, 4)?).map_err(|_| {
                    WellfriendError::ResourceLimit(
                        "JPXDecode tile-part length does not fit in memory".into(),
                    )
                })?;
                if tile_length == 0 {
                    tile_part_end = None;
                    unbounded_final_tile_part = true;
                    offset += length;
                    continue;
                }
                let end = marker_start.checked_add(tile_length).ok_or_else(|| {
                    WellfriendError::ResourceLimit("JPXDecode tile-part range overflows".into())
                })?;
                let minimum_end = offset.checked_add(length + 2).ok_or_else(|| {
                    WellfriendError::ResourceLimit("JPXDecode tile-part header overflows".into())
                })?;
                if end < minimum_end || end > codestream.len() {
                    return Err(WellfriendError::MalformedPdf(
                        "JPXDecode tile-part length is invalid".into(),
                    ));
                }
                tile_part_end = Some(end);
                unbounded_final_tile_part = false;
            }
            0x51 if segment.len() >= 38 => {
                let xsiz = be_u32(segment, 4)?;
                let ysiz = be_u32(segment, 8)?;
                let xosiz = be_u32(segment, 12)?;
                let yosiz = be_u32(segment, 16)?;
                let xtsiz = be_u32(segment, 20)?;
                let ytsiz = be_u32(segment, 24)?;
                let xtosiz = be_u32(segment, 28)?;
                let ytosiz = be_u32(segment, 32)?;
                if xtsiz == 0
                    || ytsiz == 0
                    || xsiz <= xosiz
                    || ysiz <= yosiz
                    || xsiz <= xtosiz
                    || ysiz <= ytosiz
                {
                    return Err(WellfriendError::MalformedPdf(
                        "JPXDecode SIZ declares invalid tile geometry".into(),
                    ));
                }
                let width = xsiz - xosiz;
                let height = ysiz - yosiz;
                if width > limits.max_image_width || height > limits.max_image_height {
                    return Err(WellfriendError::ResourceLimit(format!(
                        "JPXDecode dimensions {width}x{height} exceed limit {}x{}",
                        limits.max_image_width, limits.max_image_height
                    )));
                }
                let x_tiles = u64::from(xsiz - xtosiz).div_ceil(u64::from(xtsiz));
                let y_tiles = u64::from(ysiz - ytosiz).div_ceil(u64::from(ytsiz));
                let tiles = x_tiles.checked_mul(y_tiles).ok_or_else(|| {
                    WellfriendError::ResourceLimit("JPXDecode tile count overflows".into())
                })?;
                if tiles > limits.max_jpx_tiles as u64 {
                    return Err(WellfriendError::ResourceLimit(format!(
                        "JPXDecode tile count {tiles} exceeds limit {}",
                        limits.max_jpx_tiles
                    )));
                }
                let count = usize::from(u16::from_be_bytes([segment[36], segment[37]]));
                if count == 0 || count > limits.max_jpx_components {
                    return Err(WellfriendError::ResourceLimit(format!(
                        "JPXDecode component count {count} exceeds limit {}",
                        limits.max_jpx_components
                    )));
                }
                let pixels = u64::from(width)
                    .checked_mul(u64::from(height))
                    .ok_or_else(|| {
                        WellfriendError::ResourceLimit("JPXDecode pixel count overflows".into())
                    })?;
                if pixels > limits.max_image_pixels {
                    return Err(WellfriendError::ResourceLimit(format!(
                        "JPXDecode pixel count {pixels} exceeds limit {}",
                        limits.max_image_pixels
                    )));
                }
                let decoded_bytes = pixels.checked_mul(count as u64).ok_or_else(|| {
                    WellfriendError::ResourceLimit("JPXDecode byte count overflows".into())
                })?;
                if decoded_bytes > limits.max_image_decoded_bytes {
                    return Err(WellfriendError::ResourceLimit(format!(
                        "JPXDecode output {decoded_bytes} bytes exceeds limit {}",
                        limits.max_image_decoded_bytes
                    )));
                }
                components = Some(count);
            }
            0x52 if segment.len() >= 8 => {
                validate_resolution_levels(segment[7], limits)?;
            }
            0x53 => {
                let component_bytes = if components.unwrap_or(257) > 256 {
                    2
                } else {
                    1
                };
                let decomposition_offset = 2 + component_bytes + 1;
                if decomposition_offset < segment.len() {
                    validate_resolution_levels(segment[decomposition_offset], limits)?;
                }
            }
            _ => {}
        }
        offset += length;
    }
    Ok(())
}

fn validate_resolution_levels(decompositions: u8, limits: &DecodeLimits) -> Result<()> {
    let levels = usize::from(decompositions) + 1;
    if levels > limits.max_jpx_resolution_levels {
        return Err(WellfriendError::ResourceLimit(format!(
            "JPXDecode resolution levels {levels} exceed limit {}",
            limits.max_jpx_resolution_levels
        )));
    }
    Ok(())
}

fn jpx_codestream(data: &[u8]) -> Option<&[u8]> {
    if data.starts_with(&[0xff, 0x4f]) {
        return Some(data);
    }
    if !data.starts_with(&[0, 0, 0, 12, b'j', b'P', b' ', b' ']) {
        return None;
    }
    let mut offset = 0usize;
    while offset.checked_add(8)? <= data.len() {
        let short_len = be_u32_opt(data, offset)? as usize;
        let kind = data.get(offset + 4..offset + 8)?;
        let (header, length) = if short_len == 1 {
            let long_len = u64::from_be_bytes(data.get(offset + 8..offset + 16)?.try_into().ok()?);
            (16usize, usize::try_from(long_len).ok()?)
        } else if short_len == 0 {
            (8usize, data.len() - offset)
        } else {
            (8usize, short_len)
        };
        if length < header || offset.checked_add(length)? > data.len() {
            return None;
        }
        if kind == b"jp2c" {
            return Some(&data[offset + header..offset + length]);
        }
        offset += length;
    }
    None
}

fn be_u32(data: &[u8], offset: usize) -> Result<u32> {
    be_u32_opt(data, offset)
        .ok_or_else(|| WellfriendError::MalformedPdf("JPXDecode header is truncated".into()))
}

fn be_u32_opt(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn metadata_from_image(image: &Image<'_>) -> Result<JpxMetadata> {
    let width = image.width();
    let height = image.height();
    let color_space = image.color_space();
    let color_channels = color_space.num_channels();
    let stored_channels = color_channels
        .checked_add(u8::from(image.has_alpha()))
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "JPXDecode channel count overflows metadata storage".to_string(),
            )
        })?;
    if stored_channels == 0 {
        return Err(WellfriendError::MalformedPdf(
            "JPXDecode produced an image with zero channels".to_string(),
        ));
    }
    ensure_decode_budget(width, height, stored_channels)?;

    Ok(JpxMetadata {
        width,
        height,
        original_bit_depth: image.original_bit_depth(),
        color_space: jpx_metadata_color_space(color_space),
        color_channels,
        stored_channels,
        has_alpha: image.has_alpha(),
    })
}

fn jpx_metadata_color_space(color_space: &ColorSpace) -> JpxMetadataColorSpace {
    match color_space {
        ColorSpace::Gray => JpxMetadataColorSpace::Gray,
        ColorSpace::RGB => JpxMetadataColorSpace::Rgb,
        ColorSpace::CMYK => JpxMetadataColorSpace::Cmyk,
        ColorSpace::Icc { .. } => JpxMetadataColorSpace::Icc,
        ColorSpace::Unknown { .. } => JpxMetadataColorSpace::Unknown,
    }
}

fn ensure_exact_jpx_length(raw: RawImage) -> Result<RawImage> {
    // Guard against decode-length surprises without synthesizing replacement
    // pixels. A malformed JPX stream must fail typed instead of padding or
    // truncating output that could look visually plausible downstream.
    let expected = raw.byte_count();
    if raw.pixels.len() != expected {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {width}x{height} x{channels} channels decoded {actual} bytes, expected {expected}",
            width = raw.width,
            height = raw.height,
            channels = raw.channels,
            actual = raw.pixels.len()
        )));
    }

    Ok(raw)
}

fn ensure_exact_jpx_decoded_len(
    width: u32,
    height: u32,
    stored_channels: u8,
    actual_len: usize,
) -> Result<()> {
    let expected_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|value| value.checked_mul(stored_channels as usize))
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "JPXDecode {width}x{height} x{stored_channels} stored channels overflows"
            ))
        })?;
    if actual_len != expected_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {width}x{height} x{stored_channels} stored channels decoded {actual_len} bytes, expected {expected_len}"
        )));
    }
    Ok(())
}

fn finish_icc_or_unknown_color_space(
    width: u32,
    height: u32,
    num_channels: u8,
    color_only: Vec<u8>,
    alpha: Option<Vec<u8>>,
) -> Result<(Vec<u8>, u8)> {
    match num_channels {
        1 => match alpha {
            Some(alpha) => Ok((gray_alpha_to_rgba(width, height, &color_only, &alpha)?, 4u8)),
            None => Ok((color_only, 1u8)),
        },
        3 => match alpha {
            Some(alpha) => Ok((
                attach_alpha_to_rgb(width, height, &color_only, &alpha)?,
                4u8,
            )),
            None => Ok((color_only, 3u8)),
        },
        4 => {
            let rgb = ColorSpaceConverter::cmyk_to_rgb(&color_only);
            match alpha {
                Some(alpha) => Ok((attach_alpha_to_rgb(width, height, &rgb, &alpha)?, 4u8)),
                None => Ok((rgb, 3u8)),
            }
        }
        other => Err(WellfriendError::UnsupportedFeature(format!(
            "JPXDecode {width}x{height}: unsupported {other}-channel ICC/unknown color space"
        ))),
    }
}

/// Split a trailing alpha channel from interleaved JPX pixel data.
fn split_trailing_alpha(
    data: &[u8],
    stored_channels: usize,
    color_channels: usize,
) -> (Vec<u8>, Option<Vec<u8>>) {
    let mut color = Vec::with_capacity(data.len() / stored_channels * color_channels);
    let mut alpha = Vec::with_capacity(data.len() / stored_channels);
    for pixel in data.chunks_exact(stored_channels) {
        color.extend_from_slice(&pixel[..color_channels]);
        alpha.push(pixel[color_channels]);
    }
    (color, Some(alpha))
}

fn expected_jpx_pixels(width: u32, height: u32) -> Result<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "JPXDecode {width}x{height} pixel count overflows"
            ))
        })
}

fn ensure_jpx_channel_len(
    width: u32,
    height: u32,
    label: &str,
    expected: usize,
    actual: usize,
) -> Result<()> {
    if actual != expected {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {width}x{height} {label} decoded {actual} bytes, expected {expected}"
        )));
    }
    Ok(())
}

fn attach_alpha_to_rgb(width: u32, height: u32, rgb: &[u8], alpha: &[u8]) -> Result<Vec<u8>> {
    let expected_pixels = expected_jpx_pixels(width, height)?;
    let expected_rgb = expected_pixels.checked_mul(3).ok_or_else(|| {
        WellfriendError::MalformedPdf(format!(
            "JPXDecode {width}x{height} RGB byte count overflows"
        ))
    })?;
    ensure_jpx_channel_len(width, height, "RGB color channel", expected_rgb, rgb.len())?;
    ensure_jpx_channel_len(
        width,
        height,
        "RGB alpha channel",
        expected_pixels,
        alpha.len(),
    )?;
    let mut rgba = Vec::with_capacity(expected_pixels * 4);
    for (i, pixel) in rgb.chunks_exact(3).enumerate() {
        rgba.extend_from_slice(pixel);
        rgba.push(alpha[i]);
    }
    Ok(rgba)
}

fn gray_alpha_to_rgba(width: u32, height: u32, gray: &[u8], alpha: &[u8]) -> Result<Vec<u8>> {
    let expected_pixels = expected_jpx_pixels(width, height)?;
    ensure_jpx_channel_len(
        width,
        height,
        "Gray color channel",
        expected_pixels,
        gray.len(),
    )?;
    ensure_jpx_channel_len(
        width,
        height,
        "Gray alpha channel",
        expected_pixels,
        alpha.len(),
    )?;
    let mut rgba = Vec::with_capacity(gray.len() * 4);
    for (i, &sample) in gray.iter().enumerate() {
        rgba.extend_from_slice(&[sample, sample, sample, alpha[i]]);
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn siz_codestream(width: u32, height: u32, tile_width: u32, tile_height: u32) -> Vec<u8> {
        let mut data = vec![0xff, 0x4f, 0xff, 0x51, 0x00, 0x29, 0x00, 0x00];
        for value in [width, height, 0, 0, tile_width, tile_height, 0, 0] {
            data.extend_from_slice(&value.to_be_bytes());
        }
        data.extend_from_slice(&[0x00, 0x01, 0x07, 0x01, 0x01, 0xff, 0xd9]);
        data
    }

    #[test]
    fn malformed_codestream_returns_error() {
        let result = decode(b"not a jpeg2000 codestream");
        assert!(matches!(result, Err(WellfriendError::MalformedPdf(_))));
    }

    #[test]
    fn codestream_tile_count_is_bounded_before_codec_decode() {
        let data = siz_codestream(100, 100, 1, 1);
        let limits = DecodeLimits {
            max_jpx_tiles: 100,
            ..DecodeLimits::default()
        };
        let error = validate_jpx_limits(&data, &limits)
            .expect_err("excessive JPX tile grids must fail before codec decode");
        assert!(matches!(error, WellfriendError::ResourceLimit(_)));
        assert!(format!("{error}").contains("tile count 10000 exceeds limit 100"));
    }

    #[test]
    fn codestream_dimensions_are_bounded_before_codec_decode() {
        let data = siz_codestream(100, 100, 100, 100);
        let limits = DecodeLimits {
            max_image_width: 50,
            ..DecodeLimits::default()
        };
        let error = validate_jpx_limits(&data, &limits)
            .expect_err("oversized JPX dimensions must fail before codec decode");
        assert!(matches!(error, WellfriendError::ResourceLimit(_)));
        assert!(format!("{error}").contains("dimensions 100x100 exceed limit 50x"));
    }

    #[test]
    fn codestream_resolution_levels_are_bounded_before_codec_decode() {
        let mut data = siz_codestream(10, 10, 10, 10);
        data.truncate(data.len() - 2);
        data.extend_from_slice(&[
            0xff, 0x52, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00,
            0xff, 0xd9,
        ]);
        let limits = DecodeLimits {
            max_jpx_resolution_levels: 4,
            ..DecodeLimits::default()
        };
        let error = validate_jpx_limits(&data, &limits)
            .expect_err("excessive JPX resolution levels must fail before codec decode");
        assert!(matches!(error, WellfriendError::ResourceLimit(_)));
        assert!(
            format!("{error}").contains("resolution levels 9 exceed limit 4"),
            "unexpected JPX resolution limit error: {error}"
        );
    }

    #[test]
    fn tile_part_resolution_override_is_bounded_before_codec_decode() {
        let mut data = siz_codestream(10, 10, 10, 10);
        data.truncate(data.len() - 2);
        data.extend_from_slice(&[
            0xff, 0x90, 0x00, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x01, 0xff, 0x52,
            0x00, 0x0c, 0x00, 0x00, 0x00, 0x01, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0xff, 0x93,
            0xff, 0xd9,
        ]);
        let limits = DecodeLimits {
            max_jpx_resolution_levels: 4,
            ..DecodeLimits::default()
        };
        let error = validate_jpx_limits(&data, &limits)
            .expect_err("tile-part COD overrides must not bypass the resolution limit");
        assert!(matches!(error, WellfriendError::ResourceLimit(_)));
        assert!(format!("{error}").contains("resolution levels 9 exceed limit 4"));
    }

    #[test]
    fn malformed_metadata_inspection_returns_error_without_decode() {
        let error = inspect_metadata(b"not a jpeg2000 codestream")
            .expect_err("malformed JPX metadata inspection must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("JPXDecode parse failed"));
    }

    #[test]
    fn metadata_color_space_maps_hayro_families() {
        assert_eq!(
            jpx_metadata_color_space(&ColorSpace::Gray),
            JpxMetadataColorSpace::Gray
        );
        assert_eq!(
            jpx_metadata_color_space(&ColorSpace::RGB),
            JpxMetadataColorSpace::Rgb
        );
        assert_eq!(
            jpx_metadata_color_space(&ColorSpace::CMYK),
            JpxMetadataColorSpace::Cmyk
        );
        assert_eq!(
            jpx_metadata_color_space(&ColorSpace::Icc {
                profile: Vec::new(),
                num_channels: 3
            }),
            JpxMetadataColorSpace::Icc
        );
        assert_eq!(
            jpx_metadata_color_space(&ColorSpace::Unknown { num_channels: 2 }),
            JpxMetadataColorSpace::Unknown
        );
    }

    #[test]
    fn split_trailing_alpha_preserves_rgb_and_alpha() {
        let rgba = vec![10, 20, 30, 255, 40, 50, 60, 128];
        let (rgb, alpha) = split_trailing_alpha(&rgba, 4, 3);
        assert_eq!(rgb, vec![10, 20, 30, 40, 50, 60]);
        assert_eq!(alpha.unwrap(), vec![255, 128]);
    }

    #[test]
    fn gray_alpha_expands_to_rgba() {
        let gray_alpha = vec![100, 255, 200, 0];
        let (gray, alpha) = split_trailing_alpha(&gray_alpha, 2, 1);
        let rgba = gray_alpha_to_rgba(2, 1, &gray, &alpha.unwrap()).unwrap();
        assert_eq!(rgba, vec![100, 100, 100, 255, 200, 200, 200, 0]);
    }

    #[test]
    fn attach_alpha_to_rgb_keeps_alpha_channel() {
        let rgb = vec![1, 2, 3, 4, 5, 6];
        let alpha = vec![7, 8];
        assert_eq!(
            attach_alpha_to_rgb(2, 1, &rgb, &alpha).unwrap(),
            vec![1, 2, 3, 7, 4, 5, 6, 8]
        );
    }

    #[test]
    fn attach_alpha_to_rgb_refuses_short_alpha_channel() {
        let rgb = vec![1, 2, 3, 4, 5, 6];
        let error = attach_alpha_to_rgb(2, 1, &rgb, &[7])
            .expect_err("short JPX alpha channel must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("RGB alpha channel decoded 1 bytes, expected 2"));
    }

    #[test]
    fn attach_alpha_to_rgb_refuses_short_color_channel() {
        let error = attach_alpha_to_rgb(2, 1, &[1, 2, 3, 4, 5], &[7, 8])
            .expect_err("short JPX RGB channel must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("RGB color channel decoded 5 bytes, expected 6"));
    }

    #[test]
    fn gray_alpha_to_rgba_refuses_short_alpha_channel() {
        let error = gray_alpha_to_rgba(2, 1, &[100, 200], &[255])
            .expect_err("short JPX gray alpha channel must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("Gray alpha channel decoded 1 bytes, expected 2"));
    }

    #[test]
    fn unsupported_icc_or_unknown_channel_count_refuses_unconverted_pixels() {
        let error = finish_icc_or_unknown_color_space(1, 1, 2, vec![10, 20], None)
            .expect_err("unsupported JPX channel counts must fail typed");
        assert!(matches!(error, WellfriendError::UnsupportedFeature(_)));
        assert!(format!("{error}").contains("unsupported 2-channel ICC/unknown color space"));
    }

    #[test]
    fn jpx_decoded_len_refuses_short_stored_output() {
        let error = ensure_exact_jpx_decoded_len(2, 1, 4, 7)
            .expect_err("short stored JPX output must fail typed before alpha split");
        assert!(format!("{error}")
            .contains("JPXDecode 2x1 x4 stored channels decoded 7 bytes, expected 8"));
    }

    #[test]
    fn jpx_decoded_len_refuses_trailing_stored_output() {
        let error = ensure_exact_jpx_decoded_len(2, 1, 4, 9)
            .expect_err("trailing stored JPX output must fail typed before alpha split");
        assert!(format!("{error}")
            .contains("JPXDecode 2x1 x4 stored channels decoded 9 bytes, expected 8"));
    }

    #[test]
    fn jpx_exact_length_refuses_short_output() {
        let raw = RawImage {
            width: 2,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![1, 2, 3],
        };
        let error = ensure_exact_jpx_length(raw).expect_err("short JPX output must fail typed");
        assert!(
            format!("{error}").contains("JPXDecode 2x1 x3 channels decoded 3 bytes, expected 6")
        );
    }

    #[test]
    fn jpx_exact_length_refuses_long_output() {
        let raw = RawImage {
            width: 1,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![1, 2],
        };
        let error = ensure_exact_jpx_length(raw).expect_err("long JPX output must fail typed");
        assert!(
            format!("{error}").contains("JPXDecode 1x1 x1 channels decoded 2 bytes, expected 1")
        );
    }
}
