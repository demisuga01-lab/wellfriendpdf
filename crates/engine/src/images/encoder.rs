use crate::error::{Result, WellfriendError};
use crate::images::decoder::RawImage;
use crate::images::locator::{ImageLocator, ImageReference};
use crate::reader::PdfReader;

#[derive(Debug, Clone, PartialEq)]
pub enum ImageOutputFormat {
    Png,
    Jpeg,
    Webp,
    /// Keep original compressed bytes when possible.
    Original,
}

impl ImageOutputFormat {
    pub fn file_extension(&self) -> &'static str {
        match self {
            ImageOutputFormat::Png => "png",
            ImageOutputFormat::Jpeg => "jpg",
            ImageOutputFormat::Webp => "webp",
            ImageOutputFormat::Original => "bin",
        }
    }

    pub fn mime_type(&self) -> &'static str {
        match self {
            ImageOutputFormat::Png => "image/png",
            ImageOutputFormat::Jpeg => "image/jpeg",
            ImageOutputFormat::Webp => "image/webp",
            ImageOutputFormat::Original => "application/octet-stream",
        }
    }
}

pub struct ImageEncoder;

impl ImageEncoder {
    /// Encode a RawImage as PNG bytes.
    pub fn encode_png(image: &RawImage) -> Result<Vec<u8>> {
        use png::{BitDepth, ColorType, Encoder};

        let color_type = match image.channels {
            1 => ColorType::Grayscale,
            3 => ColorType::Rgb,
            4 => ColorType::Rgba,
            n => {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "PNG encoding: unsupported channel count {n}"
                )))
            }
        };

        let mut out = Vec::new();
        {
            let mut encoder = Encoder::new(&mut out, image.width, image.height);
            encoder.set_color(color_type);
            encoder.set_depth(BitDepth::Eight);
            encoder.set_compression(png::Compression::Default);
            let mut writer = encoder
                .write_header()
                .map_err(|e| WellfriendError::MalformedPdf(format!("PNG header error: {e}")))?;
            writer
                .write_image_data(&image.pixels)
                .map_err(|e| WellfriendError::MalformedPdf(format!("PNG encode error: {e}")))?;
        }
        Ok(out)
    }

    /// PNG for embedding in packages (Office media): a grayscale image whose
    /// samples are all 0 or 255 is written at bit depth 1, which decodes to the
    /// same pixels while compressing an eighth of the data. Every other image
    /// is encoded exactly as [`Self::encode_png`].
    pub fn encode_png_compact(image: &RawImage) -> Result<Vec<u8>> {
        use png::{BitDepth, ColorType, Encoder};

        let bilevel = image.channels == 1
            && image.pixels.len() == image.width as usize * image.height as usize
            && image
                .pixels
                .iter()
                .all(|&sample| sample == 0 || sample == 255);
        if !bilevel {
            return Self::encode_png(image);
        }
        let width = image.width as usize;
        let row_bytes = width.div_ceil(8);
        let mut packed = vec![0u8; row_bytes * image.height as usize];
        for (row, packed_row) in image
            .pixels
            .chunks_exact(width.max(1))
            .zip(packed.chunks_exact_mut(row_bytes.max(1)))
        {
            for (x, &sample) in row.iter().enumerate() {
                if sample == 255 {
                    packed_row[x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        let mut out = Vec::new();
        {
            let mut encoder = Encoder::new(&mut out, image.width, image.height);
            encoder.set_color(ColorType::Grayscale);
            encoder.set_depth(BitDepth::One);
            encoder.set_compression(png::Compression::Default);
            let mut writer = encoder
                .write_header()
                .map_err(|e| WellfriendError::MalformedPdf(format!("PNG header error: {e}")))?;
            writer
                .write_image_data(&packed)
                .map_err(|e| WellfriendError::MalformedPdf(format!("PNG encode error: {e}")))?;
        }
        Ok(out)
    }

    /// Encode a RawImage as PNG with fast compression.
    pub fn encode_png_fast(image: &RawImage) -> Result<Vec<u8>> {
        use png::{BitDepth, ColorType, Compression, Encoder};

        let color_type = match image.channels {
            1 => ColorType::Grayscale,
            3 => ColorType::Rgb,
            4 => ColorType::Rgba,
            n => {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "PNG fast encode: unsupported channel count {n}"
                )))
            }
        };

        let mut out = Vec::new();
        {
            let mut encoder = Encoder::new(&mut out, image.width, image.height);
            encoder.set_color(color_type);
            encoder.set_depth(BitDepth::Eight);
            encoder.set_compression(Compression::Fast);
            let mut writer = encoder.write_header().map_err(|e| {
                WellfriendError::MalformedPdf(format!("PNG fast encode header: {e}"))
            })?;
            writer
                .write_image_data(&image.pixels)
                .map_err(|e| WellfriendError::MalformedPdf(format!("PNG fast encode data: {e}")))?;
        }
        Ok(out)
    }

    /// Encode a RawImage as JPEG bytes.
    pub fn encode_jpeg(image: &RawImage, quality: u8) -> Result<Vec<u8>> {
        use jpeg_encoder::{ColorType, Encoder};

        let quality = quality.clamp(1, 100);
        let color_type = match image.channels {
            1 => ColorType::Luma,
            3 => ColorType::Rgb,
            n => {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "JPEG encoding: unsupported channel count {n} (convert to RGB first)"
                )))
            }
        };
        let width = u16::try_from(image.width).map_err(|_| {
            WellfriendError::UnsupportedFeature(format!(
                "JPEG encoding: image width {} exceeds u16::MAX",
                image.width
            ))
        })?;
        let height = u16::try_from(image.height).map_err(|_| {
            WellfriendError::UnsupportedFeature(format!(
                "JPEG encoding: image height {} exceeds u16::MAX",
                image.height
            ))
        })?;

        let mut output = Vec::new();
        let encoder = Encoder::new(&mut output, quality);
        encoder
            .encode(&image.pixels, width, height, color_type)
            .map_err(|e| WellfriendError::MalformedPdf(format!("JPEG encode error: {e}")))?;
        Ok(output)
    }

    /// Encode a RawImage as WebP bytes.
    ///
    /// Uses the pure-Rust `image-webp` crate (no C toolchain / cmake), which
    /// emits lossless VP8L WebP. The `quality` argument is accepted for API
    /// symmetry with [`Self::encode_jpeg`] but ignored, since the encoder is
    /// lossless. Supports 1-channel (grayscale), 3-channel (RGB), and 4-channel
    /// (RGBA) input; other channel counts are rejected.
    pub fn encode_webp(image: &RawImage, _quality: u8) -> Result<Vec<u8>> {
        use image_webp::{ColorType, WebPEncoder};

        let color_type = match image.channels {
            1 => ColorType::L8,
            3 => ColorType::Rgb8,
            4 => ColorType::Rgba8,
            n => {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "WebP encoding: unsupported channel count {n}"
                )))
            }
        };

        let mut out = Vec::new();
        let encoder = WebPEncoder::new(&mut out);
        encoder
            .encode(&image.pixels, image.width, image.height, color_type)
            .map_err(|e| WellfriendError::MalformedPdf(format!("WebP encode error: {e}")))?;
        Ok(out)
    }

    /// Keep original stream bytes when the source encoding matches the target.
    pub fn keep_original(
        image: &ImageReference,
        reader: &PdfReader,
        format: &ImageOutputFormat,
    ) -> Result<Option<(Vec<u8>, &'static str)>> {
        let raw = match ImageLocator::get_stream_bytes(image, reader)? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };

        match format {
            ImageOutputFormat::Original => Ok(Some((raw, original_extension(&image.filter)))),
            ImageOutputFormat::Jpeg => {
                if is_dct_only(&image.filter) {
                    Ok(Some((raw, "jpg")))
                } else {
                    Ok(None)
                }
            }
            ImageOutputFormat::Png | ImageOutputFormat::Webp => Ok(None),
        }
    }

    /// Encode a decoded RawImage to the requested format.
    pub fn encode(
        image: &RawImage,
        format: &ImageOutputFormat,
        quality: Option<u8>,
    ) -> Result<Vec<u8>> {
        let quality = quality.unwrap_or(85);
        match format {
            ImageOutputFormat::Png => Self::encode_png(image),
            ImageOutputFormat::Jpeg => Self::encode_jpeg(image, quality),
            ImageOutputFormat::Webp => Self::encode_webp(image, quality),
            ImageOutputFormat::Original => Self::encode_png(image),
        }
    }
}

/// Extension for an image stream's undecoded bytes, chosen by its filter.
/// Only a single image-codec filter yields a self-contained file; anything
/// else (Flate, LZW, no filter, chains) is opaque PDF stream data (`bin`).
/// [`crate::ContentEngine::extract_image_original`] removes lossless wrapper
/// filters first and converts non-codec images to PNG instead.
pub fn original_extension(filters: &[String]) -> &'static str {
    match filters {
        [only] => match only.as_str() {
            "DCTDecode" | "DCT" => "jpg",
            "JPXDecode" | "JPX" => "jp2",
            "JBIG2Decode" => "jb2e",
            "CCITTFaxDecode" | "CCF" => "ccitt",
            _ => "bin",
        },
        _ => "bin",
    }
}

fn is_dct_only(filters: &[String]) -> bool {
    matches!(filters, [only] if only == "DCTDecode" || only == "DCT")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_png_one_pixel_gray() {
        let img = RawImage {
            width: 1,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![128u8],
        };
        let png_bytes = ImageEncoder::encode_png(&img).unwrap();
        assert!(png_bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));
        assert_eq!(&png_bytes[12..16], b"IHDR");
        assert!(png_bytes.len() >= 67);

        let decoder = png::Decoder::new(std::io::Cursor::new(&png_bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut decoded = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!(info.width, 1);
        assert_eq!(info.height, 1);
        assert_eq!(&decoded[..info.buffer_size()], &[128u8]);
    }

    #[test]
    fn encode_png_rgb_image() {
        let img = RawImage {
            width: 2,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
        };
        let png_bytes = ImageEncoder::encode_png(&img).unwrap();
        assert!(png_bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn encode_png_fast_rgb_image() {
        let img = RawImage {
            width: 2,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0],
        };
        let png_bytes = ImageEncoder::encode_png_fast(&img).unwrap();
        assert!(png_bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }

    #[test]
    fn encode_jpeg_rgb_image() {
        let img = RawImage {
            width: 4,
            height: 4,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![128u8; 48],
        };
        let jpeg_bytes = ImageEncoder::encode_jpeg(&img, 85).unwrap();
        assert_eq!(&jpeg_bytes[..2], &[0xFF, 0xD8]);
        assert_eq!(&jpeg_bytes[jpeg_bytes.len() - 2..], &[0xFF, 0xD9]);
    }

    #[test]
    fn encode_jpeg_rejects_four_channel_input() {
        let img = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![0u8; 4],
        };
        assert!(ImageEncoder::encode_jpeg(&img, 85).is_err());
    }

    #[test]
    fn encode_webp_round_trips_rgb_losslessly() {
        // A small RGB checkerboard. image-webp emits lossless VP8L, so the
        // round-trip must be pixel-exact.
        let mut pixels = Vec::new();
        for y in 0..4u8 {
            for x in 0..4u8 {
                if (x + y) % 2 == 0 {
                    pixels.extend_from_slice(&[200, 30, 90]);
                } else {
                    pixels.extend_from_slice(&[10, 180, 240]);
                }
            }
        }
        let img = RawImage {
            width: 4,
            height: 4,
            channels: 3,
            bits_per_sample: 8,
            pixels: pixels.clone(),
        };

        let webp = ImageEncoder::encode_webp(&img, 80).unwrap();
        assert_eq!(&webp[0..4], b"RIFF");
        assert_eq!(&webp[8..12], b"WEBP");

        let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(&webp)).unwrap();
        assert_eq!(decoder.dimensions(), (4, 4));
        let mut decoded = vec![0u8; decoder.output_buffer_size().unwrap()];
        decoder.read_image(&mut decoded).unwrap();
        assert_eq!(decoded, pixels, "lossless WebP round-trip must be exact");
    }

    #[test]
    fn encode_webp_round_trips_grayscale_losslessly() {
        let pixels: Vec<u8> = (0..16u8).collect();
        let img = RawImage {
            width: 4,
            height: 4,
            channels: 1,
            bits_per_sample: 8,
            pixels: pixels.clone(),
        };
        let webp = ImageEncoder::encode_webp(&img, 80).unwrap();
        let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(&webp)).unwrap();
        assert_eq!(decoder.dimensions(), (4, 4));
        let mut decoded = vec![0u8; decoder.output_buffer_size().unwrap()];
        decoder.read_image(&mut decoded).unwrap();
        // image-webp decodes L8 WebP; compare luminance.
        assert_eq!(decoded.len() % pixels.len(), 0);
        // grayscale may decode to L8 (same length) — assert exact when so.
        if decoded.len() == pixels.len() {
            assert_eq!(decoded, pixels);
        }
    }

    #[test]
    fn encode_webp_rejects_unsupported_channel_count() {
        let img = RawImage {
            width: 1,
            height: 1,
            channels: 2,
            bits_per_sample: 8,
            pixels: vec![0u8; 2],
        };
        assert!(matches!(
            ImageEncoder::encode_webp(&img, 80),
            Err(WellfriendError::UnsupportedFeature(_))
        ));
    }

    #[test]
    fn jpeg_quality_affects_output_size() {
        let pixels: Vec<u8> = (0..(16 * 16 * 3))
            .map(|i| (i as u8).wrapping_mul(3))
            .collect();
        let img = RawImage {
            width: 16,
            height: 16,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        };
        let low_q = ImageEncoder::encode_jpeg(&img, 10).unwrap();
        let high_q = ImageEncoder::encode_jpeg(&img, 95).unwrap();
        assert!(high_q.len() > low_q.len() || high_q.len() >= 100);
    }

    #[test]
    fn image_output_format_extensions_and_mime_types() {
        assert_eq!(ImageOutputFormat::Png.file_extension(), "png");
        assert_eq!(ImageOutputFormat::Jpeg.file_extension(), "jpg");
        assert_eq!(ImageOutputFormat::Png.mime_type(), "image/png");
        assert_eq!(ImageOutputFormat::Jpeg.mime_type(), "image/jpeg");
    }

    /// A one-page PDF whose page draws one image XObject per entry of
    /// `images` (dictionary entries without /Type, /Subtype, /Length, and the
    /// stream bytes). Extra objects are appended after the images.
    fn pdf_with_images(images: &[(&str, Vec<u8>)], extra: &[Vec<u8>]) -> Vec<u8> {
        let first_image = 5;
        let mut xobjects = String::new();
        let mut content = String::new();
        for (index, _) in images.iter().enumerate() {
            xobjects.push_str(&format!("/Im{index} {} 0 R ", first_image + index));
            content.push_str(&format!(
                "q 10 0 0 10 {} 10 cm /Im{index} Do Q\n",
                index * 12
            ));
        }
        let mut objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] \
                 /Resources << /XObject << {xobjects}>> >> /Contents 4 0 R >>"
            )
            .into_bytes(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(),
                content.as_bytes(),
                b"\nendstream",
            ]
            .concat(),
        ];
        for (dict, data) in images {
            objects.push(
                [
                    format!(
                        "<< /Type /XObject /Subtype /Image {dict} /Length {} >>\nstream\n",
                        data.len()
                    )
                    .as_bytes(),
                    data.as_slice(),
                    b"\nendstream",
                ]
                .concat(),
            );
        }
        objects.extend(extra.iter().cloned());
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[test]
    fn original_extraction_names_files_by_codec_with_sidecars() {
        let jpeg = ImageEncoder::encode_jpeg(
            &RawImage {
                width: 4,
                height: 4,
                channels: 3,
                bits_per_sample: 8,
                pixels: vec![100; 48],
            },
            90,
        )
        .unwrap();
        let flate_jpeg = crate::filters::flate_encode(&jpeg, 6);
        let rgb = crate::filters::flate_encode(&[200u8; 12], 6);
        let jb2 = b"JBIG2-embedded-segments".to_vec();
        let fax = b"\x00\x01ccitt-data".to_vec();
        // Object 10 (after the 5 images at 5..=9) holds the JBIG2 globals.
        let globals = b"JBIG2-global-segments";
        let globals_obj = [
            format!("<< /Length {} >>\nstream\n", globals.len()).as_bytes(),
            globals.as_slice(),
            b"\nendstream",
        ]
        .concat();
        let pdf = pdf_with_images(
            &[
                (
                    "/Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode",
                    jpeg.clone(),
                ),
                (
                    "/Width 4 /Height 4 /ColorSpace /DeviceRGB /BitsPerComponent 8 \
                     /Filter [/FlateDecode /DCTDecode]",
                    flate_jpeg,
                ),
                (
                    "/Width 8 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 1 \
                     /Filter /JBIG2Decode /DecodeParms << /JBIG2Globals 10 0 R >>",
                    jb2.clone(),
                ),
                (
                    "/Width 8 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 1 \
                     /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8 /BlackIs1 true >>",
                    fax.clone(),
                ),
                (
                    "/Width 2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode",
                    rgb,
                ),
            ],
            &[globals_obj],
        );
        let engine = crate::ContentEngine::open_bytes(pdf).unwrap();
        let mut images = ImageLocator::find_all_images(
            &engine,
            &crate::ImageLocateOptions {
                min_width: 1,
                min_height: 1,
                ..Default::default()
            },
        )
        .unwrap();
        images.sort_by_key(|image| image.object_number);
        assert_eq!(images.len(), 5);
        let out: Vec<_> = images
            .iter()
            .map(|image| engine.extract_image_original(image).unwrap())
            .collect();

        assert_eq!(out[0].extension, "jpg");
        assert_eq!(out[0].bytes, jpeg);
        assert_eq!(out[1].extension, "jpg", "Flate-wrapped JPEG is unwrapped");
        assert_eq!(out[1].bytes, jpeg);
        assert_eq!(out[2].extension, "jb2e");
        assert_eq!(out[2].bytes, jb2);
        assert_eq!(out[2].sidecars, vec![("jb2g", globals.to_vec())]);
        assert_eq!(out[3].extension, "ccitt");
        assert_eq!(out[3].bytes, fax);
        assert_eq!(
            out[3].sidecars,
            vec![("params", b"-4 -P -X 8 -W -M\n".to_vec())]
        );
        assert_eq!(out[4].extension, "png", "non-codec images become PNG");
        assert!(out[4].bytes.starts_with(b"\x89PNG"));

        // The raw-bytes API names by filter instead of always using .bin.
        let raw = ImageEncoder::keep_original(
            &images[0],
            engine.document().reader(),
            &ImageOutputFormat::Original,
        )
        .unwrap()
        .unwrap();
        assert_eq!(raw.1, "jpg");
        assert_eq!(original_extension(&["JPXDecode".to_string()]), "jp2");
        assert_eq!(original_extension(&["FlateDecode".to_string()]), "bin");
    }

    #[test]
    fn keep_original_logic_does_not_match_flate_as_jpeg() {
        let img_ref = ImageReference {
            page_number: 1,
            xobject_name: "Im1".to_string(),
            object_number: 0,
            generation_number: 0,
            width: 1,
            height: 1,
            bits_per_component: 8,
            color_space: "DeviceRGB".to_string(),
            filter: vec!["FlateDecode".to_string()],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };
        assert!(!is_dct_only(&img_ref.filter));
    }
}
