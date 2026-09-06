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
    decode_with_settings(data, DecodeSettings::default())
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
) -> Result<DecodedJpx> {
    let original = Image::new(data, &DecodeSettings::default())
        .map_err(|err| WellfriendError::MalformedPdf(format!("JPXDecode parse failed: {err}")))?;
    let original_width = original.width();
    let original_height = original.height();
    let settings = DecodeSettings {
        target_resolution,
        ..DecodeSettings::default()
    };
    let raw = decode_with_settings(data, settings)?;
    Ok(DecodedJpx {
        raw,
        original_width,
        original_height,
    })
}

fn decode_with_settings(data: &[u8], settings: DecodeSettings) -> Result<RawImage> {
    let image = Image::new(data, &settings)
        .map_err(|err| WellfriendError::MalformedPdf(format!("JPXDecode parse failed: {err}")))?;

    let metadata = metadata_from_image(&image)?;
    let width = metadata.width;
    let height = metadata.height;
    let has_alpha = image.has_alpha();
    let color_space = image.color_space().clone();
    let color_channels = metadata.color_channels;
    let stored_channels = metadata.stored_channels;

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

    #[test]
    fn malformed_codestream_returns_error() {
        let result = decode(b"not a jpeg2000 codestream");
        assert!(matches!(result, Err(WellfriendError::MalformedPdf(_))));
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
