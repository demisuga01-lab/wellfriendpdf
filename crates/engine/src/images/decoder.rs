use std::io::Cursor;

use crate::error::{Result, WellfriendError};
use crate::filters::{
    apply_filter_bytes_with_limits, decode_stream_lossless_with_limits, DecodeLimits,
    StreamDecodeStatus,
};
use crate::images::locator::{ImageLocator, ImageReference};
use crate::images::{ccitt, jbig2, jpx};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::cmm;

/// Decoded image data: always 8 bits per channel, channels interleaved.
#[derive(Debug, Clone)]
pub struct RawImage {
    pub width: u32,
    pub height: u32,
    /// Number of channels per pixel.
    pub channels: u8,
    /// Always 8 after decoding.
    pub bits_per_sample: u8,
    /// Raw pixel bytes.
    pub pixels: Vec<u8>,
}

impl RawImage {
    /// Total number of pixels.
    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Total bytes in the pixel buffer.
    pub fn byte_count(&self) -> usize {
        self.pixel_count() * self.channels as usize
    }

    /// Row stride in bytes.
    pub fn row_stride(&self) -> usize {
        self.width as usize * self.channels as usize
    }

    /// Get a single pixel as a slice of channel values.
    pub fn pixel(&self, x: usize, y: usize) -> &[u8] {
        let channels = self.channels as usize;
        let start = y
            .saturating_mul(self.row_stride())
            .saturating_add(x.saturating_mul(channels));
        let end = start.saturating_add(channels);
        if channels == 0 || end > self.pixels.len() {
            &[]
        } else {
            &self.pixels[start..end]
        }
    }

    /// True if the image is grayscale.
    pub fn is_grayscale(&self) -> bool {
        self.channels == 1
    }

    /// True if the image is RGB.
    pub fn is_rgb(&self) -> bool {
        self.channels == 3
    }

    /// Verify pixel buffer length matches dimensions x channels.
    pub fn is_valid(&self) -> bool {
        self.pixels.len() == self.byte_count()
            && self.width > 0
            && self.height > 0
            && self.channels > 0
    }
}

pub struct ImageDecoder;

impl ImageDecoder {
    /// Decode an image from its PDF ImageReference.
    pub fn decode(image: &ImageReference, reader: &PdfReader) -> Result<RawImage> {
        Self::decode_with_limits(image, reader, &DecodeLimits::default())
    }

    pub fn decode_with_limits(
        image: &ImageReference,
        reader: &PdfReader,
        limits: &DecodeLimits,
    ) -> Result<RawImage> {
        Self::decode_with_limits_and_color_space_override(image, reader, limits, None, None)
    }

    /// Decode an image XObject while overriding a named `/ColorSpace` resource
    /// with the already-resolved colour-space object from the active page/Form
    /// resources. This keeps the core image decoder source-linked to the real
    /// image stream while allowing render-time resource inheritance to supply
    /// CalRGB/Lab/ICCBased/Indexed/Separation/DeviceN details that are not
    /// present in the image dictionary itself.
    pub(crate) fn decode_with_resolved_color_space(
        image: &ImageReference,
        reader: &PdfReader,
        color_space_name: &str,
        color_space_obj: &PdfObject,
    ) -> Result<RawImage> {
        Self::decode_with_limits_and_color_space_override(
            image,
            reader,
            &DecodeLimits::default(),
            Some(color_space_name),
            Some(color_space_obj),
        )
    }

    fn decode_with_limits_and_color_space_override(
        image: &ImageReference,
        reader: &PdfReader,
        limits: &DecodeLimits,
        color_space_name: Option<&str>,
        color_space_obj: Option<&PdfObject>,
    ) -> Result<RawImage> {
        if image.object_number == 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "inline image decoding via decode() is not supported; use decode_inline() with the raw pixel bytes"
                    .to_string(),
            ));
        }

        let obj = reader.get_object(image.object_number, image.generation_number)?;
        let (dict, raw) = match obj {
            PdfObject::Stream { dict, raw } => (dict, raw),
            _ => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "image object {} is not a stream",
                    image.object_number
                )))
            }
        };
        let mut effective_image = image.clone();
        if let Some(name) = color_space_name {
            effective_image.color_space = name.to_string();
        }
        let mut effective_dict = dict.clone();
        if let Some(space_obj) = color_space_obj {
            effective_dict.insert("ColorSpace", space_obj.clone());
        }

        let stream_obj = PdfObject::Stream {
            dict: dict.clone(),
            raw,
        };
        let decoded = decode_stream_lossless_with_limits(&stream_obj, reader, limits)?;

        match decoded.status {
            StreamDecodeStatus::Complete => Self::build_raw_image(
                decoded.data,
                effective_image.width,
                effective_image.height,
                effective_image.bits_per_component,
                &effective_image.color_space,
                &effective_dict,
                Some(reader),
            ),
            StreamDecodeStatus::StoppedAtImageFilter(filter) => {
                Self::decode_remaining_image_filter(
                    &decoded.data,
                    &filter,
                    &effective_image,
                    reader,
                    &effective_dict,
                    limits,
                )
            }
        }
    }

    /// Decode an inline image from its raw pixel bytes and parameters.
    pub fn decode_inline(
        pixel_data: &[u8],
        width: u32,
        height: u32,
        bpc: u8,
        color_space: &str,
        filter: &[&str],
        decode_parms: Option<&PdfDictionary>,
    ) -> Result<RawImage> {
        Self::decode_inline_with_limits(
            pixel_data,
            width,
            height,
            bpc,
            color_space,
            filter,
            decode_parms,
            &DecodeLimits::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn decode_inline_with_limits(
        pixel_data: &[u8],
        width: u32,
        height: u32,
        bpc: u8,
        color_space: &str,
        filter: &[&str],
        decode_parms: Option<&PdfDictionary>,
        limits: &DecodeLimits,
    ) -> Result<RawImage> {
        let decompressed =
            Self::apply_filters_direct_with_limits(pixel_data, filter, decode_parms, limits)?;
        if let Some(last_filter) = filter.last() {
            match *last_filter {
                "DCTDecode" | "DCT" => {
                    let (mut pixels, w, h, channels) = Self::decode_jpeg_with_info(&decompressed)?;
                    let final_channels = if channels == 4 {
                        pixels = ColorSpaceConverter::cmyk_to_rgb(&pixels);
                        3
                    } else {
                        channels
                    };
                    return Ok(RawImage {
                        width: w,
                        height: h,
                        channels: final_channels,
                        bits_per_sample: 8,
                        pixels,
                    });
                }
                "CCITTFaxDecode" | "CCF" => {
                    let params = ccitt_decode_params(decode_parms, width, height)?;
                    return ccitt::decode(&decompressed, params);
                }
                "JBIG2Decode" => {
                    return jbig2::decode(&decompressed, None);
                }
                "JPXDecode" => {
                    return jpx::decode(&decompressed);
                }
                _ => {}
            }
        }

        let empty_dict = PdfDictionary::empty();
        Self::build_raw_image(
            decompressed,
            width,
            height,
            bpc,
            color_space,
            &empty_dict,
            None,
        )
    }

    /// Decode an inline image with one `/DecodeParms` entry per filter. This is
    /// used by secure mutation code, where applying predictor parameters to the
    /// wrong filter would make a sample-space rewrite unsafe.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_inline_with_param_array(
        pixel_data: &[u8],
        width: u32,
        height: u32,
        bpc: u8,
        color_space: &str,
        filters: &[&str],
        decode_params: &[Option<PdfDictionary>],
        limits: &DecodeLimits,
    ) -> Result<RawImage> {
        if decode_params.len() != filters.len() {
            return Err(WellfriendError::MalformedPdf(format!(
                "inline DecodeParms count {} does not match filter count {}",
                decode_params.len(),
                filters.len()
            )));
        }
        let mut data = pixel_data.to_vec();
        for (index, &filter) in filters.iter().enumerate() {
            let params = decode_params[index].as_ref();
            if matches!(
                filter,
                "DCTDecode" | "DCT" | "JPXDecode" | "CCITTFaxDecode" | "CCF" | "JBIG2Decode"
            ) {
                if index + 1 != filters.len() {
                    return Err(WellfriendError::UnsupportedFeature(
                        "inline image codec filter must be the final filter".to_string(),
                    ));
                }
                return match filter {
                    "DCTDecode" | "DCT" => {
                        let (mut pixels, w, h, channels) = Self::decode_jpeg_with_info(&data)?;
                        let channels = if channels == 4 {
                            pixels = ColorSpaceConverter::cmyk_to_rgb(&pixels);
                            3
                        } else {
                            channels
                        };
                        Ok(RawImage {
                            width: w,
                            height: h,
                            channels,
                            bits_per_sample: 8,
                            pixels,
                        })
                    }
                    "CCITTFaxDecode" | "CCF" => {
                        ccitt::decode(&data, ccitt_decode_params(params, width, height)?)
                    }
                    "JBIG2Decode" => jbig2::decode(&data, None),
                    "JPXDecode" => jpx::decode(&data),
                    _ => unreachable!(),
                };
            }
            data = apply_filter_bytes_with_limits(filter, &data, params, limits)?;
        }
        Self::build_raw_image(
            data,
            width,
            height,
            bpc,
            color_space,
            &PdfDictionary::empty(),
            None,
        )
    }

    /// Decode a JPEG image reference directly from its original stream bytes.
    pub fn decode_jpeg_image(image: &ImageReference, reader: &PdfReader) -> Result<RawImage> {
        let raw = ImageLocator::get_stream_bytes(image, reader)?.ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "inline JPEG images not supported via this path".to_string(),
            )
        })?;
        let obj = reader.get_object(image.object_number, image.generation_number)?;
        let dict = match obj {
            PdfObject::Stream { dict, .. } => dict,
            _ => PdfDictionary::empty(),
        };
        let (pixels, width, height, channels) = Self::decode_jpeg_with_info(&raw)?;
        Self::finish_dct_decoded_image(
            pixels,
            width,
            height,
            channels,
            &image.color_space,
            &dict,
            Some(reader),
        )
    }

    /// Decode JPEG bytes and return pixels plus width, height, channel count.
    pub fn decode_jpeg_with_info(jpeg_bytes: &[u8]) -> Result<(Vec<u8>, u32, u32, u8)> {
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(jpeg_bytes));
        decoder.read_info().map_err(|e| {
            WellfriendError::MalformedPdf(format!("JPEG metadata decode failed: {e}"))
        })?;
        let info = decoder.info().ok_or_else(|| {
            WellfriendError::MalformedPdf("JPEG decode: no metadata after read_info".to_string())
        })?;
        let channels = match info.pixel_format {
            jpeg_decoder::PixelFormat::L8 => 1,
            jpeg_decoder::PixelFormat::RGB24 => 3,
            jpeg_decoder::PixelFormat::CMYK32 => 4,
            other => {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "unsupported JPEG pixel format: {other:?}"
                )))
            }
        };
        let width = u32::from(info.width);
        let height = u32::from(info.height);
        ensure_decode_budget(width, height, channels)?;
        let max_decoded = expected_len(width, height, channels);
        decoder.set_max_decoding_buffer_size(max_decoded);
        let pixels = decoder
            .decode()
            .map_err(|e| WellfriendError::MalformedPdf(format!("JPEG decode failed: {e}")))?;
        Ok((pixels, width, height, channels))
    }

    pub(crate) fn normalise_bit_depth(
        raw: Vec<u8>,
        width: u32,
        height: u32,
        channels: u8,
        bpc: u8,
    ) -> Result<Vec<u8>> {
        match bpc {
            8 => Ok(raw),
            16 => Ok(raw.chunks(2).map(|chunk| chunk[0]).collect()),
            4 | 2 | 1 => Ok(unpack_subbyte_rows(&raw, width, height, channels, bpc)),
            other => Err(WellfriendError::UnsupportedFeature(format!(
                "unsupported bits_per_component: {other}"
            ))),
        }
    }

    #[cfg(test)]
    pub(crate) fn build_raw_image_pub(
        decompressed: Vec<u8>,
        width: u32,
        height: u32,
        bpc: u8,
        color_space: &str,
        dict: &PdfDictionary,
    ) -> Result<RawImage> {
        Self::build_raw_image(decompressed, width, height, bpc, color_space, dict, None)
    }

    fn decode_remaining_image_filter(
        data: &[u8],
        filter: &str,
        image: &ImageReference,
        reader: &PdfReader,
        dict: &PdfDictionary,
        limits: &DecodeLimits,
    ) -> Result<RawImage> {
        match filter {
            "DCTDecode" | "DCT" => {
                let (pixels, width, height, channels) = Self::decode_jpeg_with_info(data)?;
                if width != image.width || height != image.height {
                    log::warn!(
                        "JPEG image {}: dict dimensions {}x{} differ from JPEG header {}x{}; using JPEG header",
                        image.xobject_name,
                        image.width,
                        image.height,
                        width,
                        height
                    );
                }
                Self::finish_dct_decoded_image(
                    pixels,
                    width,
                    height,
                    channels,
                    &image.color_space,
                    dict,
                    Some(reader),
                )
            }
            "JPXDecode" => jpx::decode(data).map(|raw| Self::finish_jpx_decoded_image(raw, dict)),
            "CCITTFaxDecode" | "CCF" => {
                let decode_params = image_decode_params(dict, Some(reader), filter)?;
                let params =
                    ccitt_decode_params(decode_params.as_ref(), image.width, image.height)?;
                ccitt::decode(data, params)
            }
            "JBIG2Decode" => {
                let decode_params = image_decode_params(dict, Some(reader), filter)?;
                let globals = jbig2_globals(decode_params.as_ref(), reader, limits)?;
                jbig2::decode(data, globals.as_deref())
            }
            other => {
                let _ = reader;
                let _ = dict;
                Err(WellfriendError::UnsupportedFeature(format!(
                    "unknown image filter '{other}'"
                )))
            }
        }
    }

    fn apply_filters_direct_with_limits(
        raw: &[u8],
        filters: &[&str],
        decode_parms: Option<&PdfDictionary>,
        limits: &DecodeLimits,
    ) -> Result<Vec<u8>> {
        let mut data = raw.to_vec();
        for &filter in filters {
            if matches!(
                filter,
                "DCTDecode" | "DCT" | "JPXDecode" | "CCITTFaxDecode" | "CCF" | "JBIG2Decode"
            ) {
                return Ok(data);
            }
            data = apply_filter_bytes_with_limits(filter, &data, decode_parms, limits)?;
        }
        Ok(data)
    }

    fn build_raw_image(
        decompressed: Vec<u8>,
        width: u32,
        height: u32,
        bpc: u8,
        color_space: &str,
        dict: &PdfDictionary,
        reader: Option<&PdfReader>,
    ) -> Result<RawImage> {
        let raw_channels = Self::raw_channel_count(color_space, dict, reader);
        // H-4: bound the declared dimensions before allocating any pixel buffer.
        ensure_decode_budget(width, height, raw_channels.max(1))?;
        if width == 0 || height == 0 {
            return Ok(RawImage {
                width,
                height,
                channels: raw_channels.max(1),
                bits_per_sample: 8,
                pixels: Vec::new(),
            });
        }
        if decompressed.is_empty() {
            return Err(WellfriendError::MalformedPdf(format!(
                "image {width}x{height} decoded to empty pixel data"
            )));
        }

        let mut normalised =
            Self::normalise_bit_depth(decompressed, width, height, raw_channels, bpc)?;
        let is_mask = match dict.get("ImageMask").or_else(|| dict.get("IM")) {
            Some(PdfObject::Boolean(value)) => *value,
            Some(PdfObject::Name(name)) => name.eq_ignore_ascii_case("true"),
            _ => false,
        };
        if is_mask {
            let mut pixels = normalised;
            let expected_size = expected_len(width, height, 1);
            resize_mismatch(&mut pixels, width, height, expected_size);
            return Ok(RawImage {
                width,
                height,
                channels: 1,
                bits_per_sample: 8,
                pixels,
            });
        }

        if decode_array_applies_before_color_conversion(color_space) {
            Self::apply_decode_array(&mut normalised, raw_channels, dict);
        }

        let (mut pixels, channels) = match color_space {
            "DeviceGray" | "G" => (normalised, 1u8),
            "CalGray" => (
                cmm::cal_gray_bytes_to_rgb(
                    &normalised,
                    cmm::cal_gray_params_from_image_dict(dict, reader),
                ),
                3u8,
            ),
            "DeviceRGB" | "RGB" | "sRGB" => (normalised, 3u8),
            "CalRGB" => (
                cmm::cal_rgb_bytes_to_rgb(
                    &normalised,
                    cmm::cal_rgb_params_from_image_dict(dict, reader),
                ),
                3u8,
            ),
            "DeviceCMYK" | "CMYK" => (ColorSpaceConverter::cmyk_to_rgb(&normalised), 3u8),
            "Lab" => (
                cmm::lab_bytes_to_rgb(&normalised, cmm::lab_params_from_image_dict(dict, reader)),
                3u8,
            ),
            "Indexed" => {
                if let Some(reader) = reader {
                    ColorSpaceConverter::decode_indexed(&normalised, dict, reader, width, height)?
                } else {
                    log::warn!("Indexed color space without reader; using raw index bytes");
                    (normalised, 1u8)
                }
            }
            "Separation" | "DeviceN" => {
                if let Some(reader) = reader {
                    ColorSpaceConverter::tint_space_to_rgba(&normalised, dict, reader, raw_channels)
                } else {
                    log::warn!("{color_space} image color space without reader; using raw data");
                    (normalised, raw_channels)
                }
            }
            "ICCBased" => {
                if let Some(reader) = reader {
                    if let Some(converted) = cmm::icc_bytes_to_rgb(&normalised, dict, reader) {
                        converted
                    } else {
                        let n = ColorSpaceConverter::icc_channel_count(dict, reader).unwrap_or(3);
                        match n {
                            1 => (normalised, 1u8),
                            3 => (normalised, 3u8),
                            4 => (ColorSpaceConverter::cmyk_to_rgb(&normalised), 3u8),
                            _ => {
                                return Err(WellfriendError::UnsupportedFeature(format!(
                                    "ICCBased with {n} components not supported"
                                )))
                            }
                        }
                    }
                } else {
                    (normalised, 3u8)
                }
            }
            other => {
                log::warn!("build_raw_image: unhandled color space '{other}', using raw data");
                let ch = if normalised.len() == (width * height) as usize {
                    1u8
                } else {
                    3u8
                };
                (normalised, ch)
            }
        };

        let expected_size = expected_len(width, height, channels);
        resize_mismatch(&mut pixels, width, height, expected_size);

        Ok(RawImage {
            width,
            height,
            channels,
            bits_per_sample: 8,
            pixels,
        })
    }

    fn finish_dct_decoded_image(
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        channels: u8,
        color_space: &str,
        dict: &PdfDictionary,
        reader: Option<&PdfReader>,
    ) -> Result<RawImage> {
        let expected_channels = Self::raw_channel_count(color_space, dict, reader);
        if channels == expected_channels {
            return Self::build_raw_image(pixels, width, height, 8, color_space, dict, reader);
        }
        if channels == 4 {
            let mut pixels = pixels;
            Self::apply_decode_array(&mut pixels, channels, dict);
            return Ok(RawImage {
                width,
                height,
                channels: 3,
                bits_per_sample: 8,
                pixels: ColorSpaceConverter::cmyk_to_rgb(&pixels),
            });
        }
        Ok(RawImage {
            width,
            height,
            channels,
            bits_per_sample: 8,
            pixels,
        })
    }

    fn finish_jpx_decoded_image(mut raw: RawImage, dict: &PdfDictionary) -> RawImage {
        if raw.channels != 4 {
            return raw;
        }
        match dict.get_integer("SMaskInData") {
            Some(0) => {
                let mut rgb = Vec::with_capacity((raw.pixels.len() / 4).saturating_mul(3));
                for px in raw.pixels.chunks(4) {
                    rgb.extend_from_slice(&px[..3]);
                }
                raw.pixels = rgb;
                raw.channels = 3;
                raw
            }
            Some(2) => {
                for px in raw.pixels.chunks_mut(4) {
                    let alpha = u16::from(px[3]);
                    if alpha == 0 {
                        px[0] = 0;
                        px[1] = 0;
                        px[2] = 0;
                    } else if alpha < 255 {
                        for channel in &mut px[..3] {
                            let value = (u16::from(*channel) * 255 + alpha / 2) / alpha;
                            *channel = value.min(255) as u8;
                        }
                    }
                }
                raw
            }
            _ => raw,
        }
    }

    fn apply_decode_array(pixels: &mut [u8], channels: u8, dict: &PdfDictionary) {
        let Some(items) = dict.get("Decode").and_then(PdfObject::as_array) else {
            return;
        };
        let values: Vec<f64> = items.iter().filter_map(PdfObject::as_number).collect();
        let channels = channels.max(1) as usize;
        if values.len() < channels * 2 {
            log::warn!(
                "image /Decode has {} entries for {} channels; ignoring",
                values.len(),
                channels
            );
            return;
        }

        for (idx, sample) in pixels.iter_mut().enumerate() {
            let ch = idx % channels;
            let low = values[ch * 2];
            let high = values[ch * 2 + 1];
            if !low.is_finite() || !high.is_finite() {
                continue;
            }
            let unit = f64::from(*sample) / 255.0;
            let decoded = (low + unit * (high - low)).clamp(0.0, 1.0);
            *sample = (decoded * 255.0).round() as u8;
        }
    }

    fn raw_channel_count(
        color_space: &str,
        dict: &PdfDictionary,
        reader: Option<&PdfReader>,
    ) -> u8 {
        match color_space {
            "DeviceGray" | "G" | "CalGray" | "Indexed" => 1,
            "DeviceRGB" | "RGB" | "CalRGB" | "sRGB" | "Lab" => 3,
            "DeviceCMYK" | "CMYK" => 4,
            "Separation" => 1,
            "DeviceN" => dict
                .get("ColorSpace")
                .and_then(PdfObject::as_array)
                .and_then(|arr| arr.get(1))
                .and_then(PdfObject::as_array)
                .map(|names| names.len().clamp(1, 16) as u8)
                .unwrap_or(1),
            "ICCBased" => reader
                .and_then(|reader| ColorSpaceConverter::icc_channel_count(dict, reader))
                .unwrap_or(3),
            _ => 3,
        }
    }
}

pub struct ColorSpaceConverter;

impl ColorSpaceConverter {
    /// Convert source color space pixels to normalized output.
    pub fn convert(
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        source_cs: &str,
        dict: &PdfDictionary,
        reader: &PdfReader,
    ) -> Result<(Vec<u8>, u8)> {
        match source_cs {
            "DeviceGray" | "G" => Ok((pixels, 1)),
            "CalGray" => Ok((
                cmm::cal_gray_bytes_to_rgb(
                    &pixels,
                    cmm::cal_gray_params_from_image_dict(dict, Some(reader)),
                ),
                3,
            )),
            "DeviceRGB" | "RGB" | "sRGB" => Ok((pixels, 3)),
            "CalRGB" => Ok((
                cmm::cal_rgb_bytes_to_rgb(
                    &pixels,
                    cmm::cal_rgb_params_from_image_dict(dict, Some(reader)),
                ),
                3,
            )),
            "DeviceCMYK" | "CMYK" => Ok((Self::cmyk_to_rgb(&pixels), 3)),
            "ICCBased" => {
                if let Some(converted) = cmm::icc_bytes_to_rgb(&pixels, dict, reader) {
                    Ok(converted)
                } else {
                    let n = Self::icc_channel_count(dict, reader).unwrap_or(3);
                    match n {
                        1 => Ok((pixels, 1)),
                        3 => Ok((pixels, 3)),
                        4 => Ok((Self::cmyk_to_rgb(&pixels), 3)),
                        _ => Err(WellfriendError::UnsupportedFeature(format!(
                            "ICCBased with {n} components not supported"
                        ))),
                    }
                }
            }
            "Indexed" => Self::decode_indexed(&pixels, dict, reader, width, height),
            "Separation" | "DeviceN" => Ok(Self::tint_space_to_rgba(
                &pixels,
                dict,
                reader,
                Self::tint_space_channel_count(dict, source_cs),
            )),
            "Lab" => Ok((
                cmm::lab_bytes_to_rgb(&pixels, cmm::lab_params_from_image_dict(dict, Some(reader))),
                3,
            )),
            other => {
                log::warn!(
                    "unknown color space '{}', treating as RGB/Gray by channel count",
                    other
                );
                let channels = if pixels.len() == (width * height) as usize {
                    1
                } else {
                    3
                };
                Ok((pixels, channels))
            }
        }
    }

    fn tint_space_to_rgba(
        pixels: &[u8],
        dict: &PdfDictionary,
        reader: &PdfReader,
        channels: u8,
    ) -> (Vec<u8>, u8) {
        let Some(space_obj) = dict.get("ColorSpace").or_else(|| dict.get("CS")) else {
            return (pixels.to_vec(), channels.max(1));
        };
        let channels = channels.max(1) as usize;
        let mut output = Vec::with_capacity((pixels.len() / channels).saturating_mul(4));
        for chunk in pixels.chunks(channels) {
            let mut components = Vec::with_capacity(channels);
            for sample in chunk.iter().take(channels) {
                components.push(f64::from(*sample) / 255.0);
            }
            while components.len() < channels {
                components.push(0.0);
            }
            match crate::render::colorspace::resolve_named_color(
                space_obj,
                &components,
                1.0,
                reader,
            ) {
                crate::render::colorspace::NamedColor::Color(color) => {
                    output.extend_from_slice(&color.to_pixel_color());
                }
                crate::render::colorspace::NamedColor::NoPaint => {
                    output.extend_from_slice(&[0, 0, 0, 0]);
                }
                crate::render::colorspace::NamedColor::Unhandled => {
                    let fallback = chunk.first().copied().unwrap_or(0);
                    output.extend_from_slice(&[fallback, fallback, fallback, 255]);
                }
            }
        }
        (output, 4)
    }

    fn tint_space_channel_count(dict: &PdfDictionary, source_cs: &str) -> u8 {
        match source_cs {
            "DeviceN" => dict
                .get("ColorSpace")
                .and_then(PdfObject::as_array)
                .and_then(|arr| arr.get(1))
                .and_then(PdfObject::as_array)
                .map(|names| names.len().clamp(1, 16) as u8)
                .unwrap_or(1),
            _ => 1,
        }
    }

    /// Convert interleaved CMYK pixels to RGB.
    pub fn cmyk_to_rgb(pixels: &[u8]) -> Vec<u8> {
        cmm::device_cmyk_bytes_to_rgb(pixels)
    }

    fn decode_indexed(
        pixels: &[u8],
        dict: &PdfDictionary,
        reader: &PdfReader,
        _width: u32,
        _height: u32,
    ) -> Result<(Vec<u8>, u8)> {
        let cs_array = dict
            .get("ColorSpace")
            .and_then(PdfObject::as_array)
            .unwrap_or(&[]);

        if cs_array.len() < 4 {
            log::warn!("Indexed color space: missing required parameters, treating as gray");
            return Ok((pixels.to_vec(), 1));
        }

        let base_space = cs_array
            .get(1)
            .cloned()
            .unwrap_or_else(|| PdfObject::Name("DeviceRGB".to_string()));
        let base_cs = Self::color_space_family_name(&base_space, reader);
        let hival = cs_array
            .get(2)
            .and_then(PdfObject::as_integer)
            .unwrap_or(255)
            .max(0) as usize;

        let lookup = match cs_array.get(3) {
            Some(PdfObject::String(bytes)) => bytes.clone(),
            Some(PdfObject::Reference { number, generation }) => {
                match reader.get_object(*number, *generation) {
                    Ok(PdfObject::String(bytes)) => bytes,
                    Ok(PdfObject::Stream { raw, .. }) => raw,
                    _ => {
                        log::warn!("Indexed color space: lookup table reference failed");
                        return Ok((pixels.to_vec(), 1));
                    }
                }
            }
            _ => {
                log::warn!("Indexed color space: unrecognised lookup type");
                return Ok((pixels.to_vec(), 1));
            }
        };

        let base_channels = Self::indexed_base_channel_count(&base_cs, &base_space, reader);
        let expected_lookup_len = (hival + 1) * base_channels;
        if lookup.len() < expected_lookup_len {
            log::warn!(
                "Indexed color space: lookup table too short ({} < {})",
                lookup.len(),
                expected_lookup_len
            );
        }

        let mut raw_palette = Vec::with_capacity(expected_lookup_len);
        for idx in 0..=hival {
            let start = idx * base_channels;
            let end = (start + base_channels).min(lookup.len());
            if end <= start {
                raw_palette.extend(std::iter::repeat_n(0u8, base_channels));
            } else {
                raw_palette.extend_from_slice(&lookup[start..end]);
                raw_palette.extend(std::iter::repeat_n(0u8, start + base_channels - end));
            }
        }

        let mut base_dict = PdfDictionary::empty();
        base_dict.insert("ColorSpace", base_space);
        let (palette, palette_channels) = match base_cs.as_str() {
            "Indexed" => (raw_palette, base_channels as u8),
            _ => Self::convert(
                raw_palette,
                (hival + 1).min(u32::MAX as usize) as u32,
                1,
                &base_cs,
                &base_dict,
                reader,
            )?,
        };
        let palette_channels = palette_channels.max(1) as usize;
        let mut output = Vec::with_capacity(pixels.len() * palette_channels);
        for &idx in pixels {
            let idx = (idx as usize).min(hival);
            let start = idx * palette_channels;
            let end = start + palette_channels;
            if let Some(entry) = palette.get(start..end) {
                output.extend_from_slice(entry);
            } else {
                output.extend(std::iter::repeat_n(0u8, palette_channels));
            }
        }
        Ok((output, palette_channels as u8))
    }

    fn color_space_family_name(space: &PdfObject, reader: &PdfReader) -> String {
        let resolved = match space {
            PdfObject::Reference { .. } => reader
                .resolve(space.clone())
                .unwrap_or_else(|_| space.clone()),
            other => other.clone(),
        };
        match resolved {
            PdfObject::Name(name) => name,
            PdfObject::Array(items) => items
                .first()
                .and_then(PdfObject::as_name)
                .unwrap_or("DeviceRGB")
                .to_string(),
            _ => "DeviceRGB".to_string(),
        }
    }

    fn indexed_base_channel_count(
        base_cs: &str,
        base_space: &PdfObject,
        reader: &PdfReader,
    ) -> usize {
        match base_cs {
            "DeviceGray" | "G" | "CalGray" | "Separation" => 1,
            "DeviceCMYK" | "CMYK" => 4,
            "DeviceN" => Self::device_n_component_count(base_space, reader).unwrap_or(1),
            "ICCBased" => Self::icc_component_count_from_space(base_space, reader).unwrap_or(3),
            "DeviceRGB" | "RGB" | "CalRGB" | "sRGB" | "Lab" => 3,
            _ => 3,
        }
    }

    fn device_n_component_count(space: &PdfObject, reader: &PdfReader) -> Option<usize> {
        let resolved = match space {
            PdfObject::Reference { .. } => reader.resolve(space.clone()).ok()?,
            other => other.clone(),
        };
        let arr = resolved.as_array()?;
        if arr.first().and_then(PdfObject::as_name) != Some("DeviceN") {
            return None;
        }
        arr.get(1)
            .and_then(PdfObject::as_array)
            .map(|names| names.len().clamp(1, 16))
    }

    fn icc_component_count_from_space(space: &PdfObject, reader: &PdfReader) -> Option<usize> {
        let resolved = match space {
            PdfObject::Reference { .. } => reader.resolve(space.clone()).ok()?,
            other => other.clone(),
        };
        let arr = resolved.as_array()?;
        if arr.first().and_then(PdfObject::as_name) != Some("ICCBased") {
            return None;
        }
        let profile = reader.resolve(arr.get(1)?.clone()).ok()?;
        profile
            .as_stream()
            .and_then(|(dict, _)| dict.get_integer("N"))
            .map(|n| n.clamp(1, 4) as usize)
    }

    fn icc_channel_count(dict: &PdfDictionary, reader: &PdfReader) -> Option<u8> {
        cmm::icc_channel_count(dict, reader)
    }
}

fn decode_array_applies_before_color_conversion(color_space: &str) -> bool {
    !matches!(color_space, "Indexed" | "Lab")
}

fn unpack_subbyte_rows(raw: &[u8], width: u32, height: u32, channels: u8, bpc: u8) -> Vec<u8> {
    let channels = channels.max(1) as usize;
    let samples_per_row = width as usize * channels;
    let total = samples_per_row.saturating_mul(height as usize);
    let bits_per_row = samples_per_row.saturating_mul(bpc as usize);
    let bytes_per_row = bits_per_row.div_ceil(8);
    let max_value = (1u16 << bpc) - 1;
    let scale = 255u16 / max_value;
    let mask = max_value as u8;

    let mut out = Vec::with_capacity(total);
    for row in 0..height as usize {
        let row_start = row.saturating_mul(bytes_per_row);
        let row_end = row_start.saturating_add(bytes_per_row).min(raw.len());
        let row_bytes = raw.get(row_start..row_end).unwrap_or(&[]);
        for sample in 0..samples_per_row {
            let bit_offset = sample.saturating_mul(bpc as usize);
            let byte = row_bytes.get(bit_offset / 8).copied().unwrap_or(0);
            let shift = 8usize - bpc as usize - (bit_offset % 8);
            let packed = (byte >> shift) & mask;
            out.push((u16::from(packed) * scale) as u8);
        }
    }
    out
}

fn expected_len(width: u32, height: u32, channels: u8) -> usize {
    width as usize * height as usize * channels as usize
}

/// Reject an image whose declared dimensions would exceed the decode pixel
/// budget *before* any pixel buffer is allocated. This closes the decode-layer
/// OOM gap (the render-layer pixel cap does not gate embedded-image decode): a
/// few-hundred-byte stream declaring e.g. `/Width 60000 /Height 60000` is turned
/// into a clean error instead of a multi-gigabyte `Vec` reservation.
pub(crate) fn ensure_decode_budget(width: u32, height: u32, channels: u8) -> Result<()> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    let cap = crate::engine::max_decode_pixels();
    if pixels > cap {
        return Err(WellfriendError::MalformedPdf(format!(
            "image {width}x{height} = {pixels} pixels exceeds decode cap of {cap} pixels \
             (raise WELLFRIENDPDF_MAX_DECODE_PIXELS if this is a legitimate image)"
        )));
    }
    // Guard the byte product against `usize` overflow (notably on 32-bit / wasm32).
    let channels = u64::from(channels.max(1));
    if pixels
        .checked_mul(channels)
        .and_then(|bytes| usize::try_from(bytes).ok())
        .is_none()
    {
        return Err(WellfriendError::MalformedPdf(format!(
            "image {width}x{height} x{channels} channels overflows addressable memory"
        )));
    }
    Ok(())
}

fn resize_mismatch(pixels: &mut Vec<u8>, width: u32, height: u32, expected_size: usize) {
    if pixels.len() != expected_size {
        log::warn!(
            "image {}x{}: decoded {} bytes, expected {}; truncating/padding",
            width,
            height,
            pixels.len(),
            expected_size
        );
        pixels.resize(expected_size, 0);
    }
}

fn ccitt_decode_params(
    params: Option<&PdfDictionary>,
    image_width: u32,
    image_height: u32,
) -> Result<ccitt::CcittDecodeParams> {
    let default_columns = if image_width == 0 { 1728 } else { image_width };
    let columns = u32_param(params, "Columns", default_columns, false)?;
    let mut rows = u32_param(params, "Rows", image_height, true)?;
    if rows == 0 && image_height > 0 {
        rows = image_height;
    }

    Ok(ccitt::CcittDecodeParams {
        k: int_param(params, "K", 0)?,
        columns,
        rows,
        black_is_1: bool_param(params, "BlackIs1", false)?,
        encoded_byte_align: bool_param(params, "EncodedByteAlign", false)?,
        end_of_line: bool_param(params, "EndOfLine", false)?,
        end_of_block: bool_param(params, "EndOfBlock", true)?,
    })
}

fn jbig2_globals(
    params: Option<&PdfDictionary>,
    reader: &PdfReader,
    limits: &DecodeLimits,
) -> Result<Option<Vec<u8>>> {
    let Some(globals_obj) = params.and_then(|dict| dict.get("JBIG2Globals")) else {
        return Ok(None);
    };
    let globals_obj = reader.resolve(globals_obj.clone())?;
    match globals_obj {
        PdfObject::Stream { dict, raw } => {
            let stream = PdfObject::Stream { dict, raw };
            let decoded = decode_stream_lossless_with_limits(&stream, reader, limits)?;
            match decoded.status {
                StreamDecodeStatus::Complete => Ok(Some(decoded.data)),
                StreamDecodeStatus::StoppedAtImageFilter(filter) => {
                    Err(WellfriendError::UnsupportedFeature(format!(
                        "JBIG2Globals stream stopped at image filter {filter}"
                    )))
                }
            }
        }
        PdfObject::String(bytes) => Ok(Some(bytes)),
        PdfObject::Null => Ok(None),
        other => Err(WellfriendError::MalformedPdf(format!(
            "JBIG2Decode /JBIG2Globals must resolve to a stream, got {}",
            other.variant_name()
        ))),
    }
}

fn image_decode_params(
    dict: &PdfDictionary,
    reader: Option<&PdfReader>,
    target_filter: &str,
) -> Result<Option<PdfDictionary>> {
    let filters = stream_filter_names(dict, reader)?;
    let params = stream_decode_params(dict, reader, filters.len())?;
    let target_idx = filters
        .iter()
        .position(|filter| same_filter(filter, target_filter))
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "image stream filter list does not contain stopped filter {target_filter}"
            ))
        })?;

    Ok(params.get(target_idx).cloned().flatten())
}

fn stream_filter_names(dict: &PdfDictionary, reader: Option<&PdfReader>) -> Result<Vec<String>> {
    let Some(filter_obj) = dict.get("Filter").or_else(|| dict.get("F")) else {
        return Ok(Vec::new());
    };
    let filter_obj = resolved_object(filter_obj, reader)?;
    match filter_obj {
        PdfObject::Name(name) => Ok(vec![name]),
        PdfObject::Array(items) => {
            let mut names = Vec::with_capacity(items.len());
            for item in items {
                match resolved_object(&item, reader)? {
                    PdfObject::Name(name) => names.push(name),
                    other => {
                        return Err(WellfriendError::MalformedPdf(format!(
                            "filter array contains {}",
                            other.variant_name()
                        )));
                    }
                }
            }
            Ok(names)
        }
        PdfObject::Null => Ok(Vec::new()),
        other => Err(WellfriendError::MalformedPdf(format!(
            "Filter must be a name or array, got {}",
            other.variant_name()
        ))),
    }
}

fn stream_decode_params(
    dict: &PdfDictionary,
    reader: Option<&PdfReader>,
    filter_count: usize,
) -> Result<Vec<Option<PdfDictionary>>> {
    let Some(params_obj) = dict.get("DecodeParms").or_else(|| dict.get("DP")) else {
        return Ok(vec![None; filter_count]);
    };
    let params_obj = resolved_object(params_obj, reader)?;
    match params_obj {
        PdfObject::Null => Ok(vec![None; filter_count]),
        PdfObject::Dictionary(params) => {
            let mut out = vec![None; filter_count];
            if !out.is_empty() {
                out[0] = Some(params);
            }
            Ok(out)
        }
        PdfObject::Array(items) => {
            let mut out = Vec::with_capacity(filter_count);
            for item in items.into_iter().take(filter_count) {
                match resolved_object(&item, reader)? {
                    PdfObject::Null => out.push(None),
                    PdfObject::Dictionary(params) => out.push(Some(params)),
                    other => {
                        return Err(WellfriendError::MalformedPdf(format!(
                            "DecodeParms array contains {}",
                            other.variant_name()
                        )));
                    }
                }
            }
            while out.len() < filter_count {
                out.push(None);
            }
            Ok(out)
        }
        other => Err(WellfriendError::MalformedPdf(format!(
            "DecodeParms must be a dictionary or array, got {}",
            other.variant_name()
        ))),
    }
}

fn resolved_object(obj: &PdfObject, reader: Option<&PdfReader>) -> Result<PdfObject> {
    match reader {
        Some(reader) => reader.resolve(obj.clone()),
        None => Ok(obj.clone()),
    }
}

fn same_filter(a: &str, b: &str) -> bool {
    canonical_filter_name(a) == canonical_filter_name(b)
}

fn canonical_filter_name(name: &str) -> &str {
    match name {
        "DCT" => "DCTDecode",
        "CCF" => "CCITTFaxDecode",
        other => other,
    }
}

fn int_param(params: Option<&PdfDictionary>, key: &str, default: i64) -> Result<i64> {
    match params.and_then(|dict| dict.get(key)) {
        Some(PdfObject::Integer(value)) => Ok(*value),
        Some(other) => Err(WellfriendError::MalformedPdf(format!(
            "CCITTFaxDecode /{key} must be an integer, got {}",
            other.variant_name()
        ))),
        None => Ok(default),
    }
}

fn u32_param(
    params: Option<&PdfDictionary>,
    key: &str,
    default: u32,
    allow_zero: bool,
) -> Result<u32> {
    let value = int_param(params, key, i64::from(default))?;
    if value < 0 || (!allow_zero && value == 0) {
        return Err(WellfriendError::MalformedPdf(format!(
            "CCITTFaxDecode /{key} must be {}",
            if allow_zero {
                "nonnegative"
            } else {
                "positive"
            }
        )));
    }
    u32::try_from(value)
        .map_err(|_| WellfriendError::MalformedPdf(format!("CCITTFaxDecode /{key} is too large")))
}

fn bool_param(params: Option<&PdfDictionary>, key: &str, default: bool) -> Result<bool> {
    match params.and_then(|dict| dict.get(key)) {
        Some(PdfObject::Boolean(value)) => Ok(*value),
        Some(PdfObject::Name(name)) if name.eq_ignore_ascii_case("true") => Ok(true),
        Some(PdfObject::Name(name)) if name.eq_ignore_ascii_case("false") => Ok(false),
        Some(other) => Err(WellfriendError::MalformedPdf(format!(
            "CCITTFaxDecode /{key} must be a boolean, got {}",
            other.variant_name()
        ))),
        None => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::images::encoder::ImageEncoder;

    #[test]
    fn normalise_bit_depth_8_bit_passthrough() {
        let pixels = vec![100u8, 150, 200];
        let out = ImageDecoder::normalise_bit_depth(pixels.clone(), 3, 1, 1, 8).unwrap();
        assert_eq!(out, pixels);
    }

    #[test]
    fn normalise_bit_depth_16_bit_to_8_bit() {
        let pixels = vec![0xAB_u8, 0xCD, 0x00, 0xFF];
        let out = ImageDecoder::normalise_bit_depth(pixels, 2, 1, 1, 16).unwrap();
        assert_eq!(out, vec![0xAB, 0x00]);
    }

    #[test]
    fn normalise_bit_depth_4_bit() {
        let pixels = vec![0xF0_u8];
        let out = ImageDecoder::normalise_bit_depth(pixels, 2, 1, 1, 4).unwrap();
        assert_eq!(out, vec![255, 0]);
    }

    #[test]
    fn normalise_bit_depth_2_bit() {
        let pixels = vec![0b11_10_01_00_u8];
        let out = ImageDecoder::normalise_bit_depth(pixels, 4, 1, 1, 2).unwrap();
        assert_eq!(out, vec![255, 170, 85, 0]);
    }

    #[test]
    fn normalise_bit_depth_1_bit() {
        let pixels = vec![0b1010_0000_u8];
        let out = ImageDecoder::normalise_bit_depth(pixels, 8, 1, 1, 1).unwrap();
        assert_eq!(out[0], 255);
        assert_eq!(out[1], 0);
        assert_eq!(out[2], 255);
    }

    #[test]
    fn normalise_bit_depth_1_bit_respects_row_padding() {
        let pixels = vec![0b1010_0000_u8, 0b0100_0000_u8];
        let out = ImageDecoder::normalise_bit_depth(pixels, 3, 2, 1, 1).unwrap();
        assert_eq!(out, vec![255, 0, 255, 0, 255, 0]);
    }

    #[test]
    fn normalise_bit_depth_2_bit_respects_row_padding() {
        let pixels = vec![0b11_10_00_00_u8, 0b01_00_00_00_u8];
        let out = ImageDecoder::normalise_bit_depth(pixels, 2, 2, 1, 2).unwrap();
        assert_eq!(out, vec![255, 170, 85, 0]);
    }

    #[test]
    fn normalise_bit_depth_unsupported_bpc_returns_error() {
        let result = ImageDecoder::normalise_bit_depth(vec![0], 1, 1, 1, 3);
        assert!(result.is_err());
    }

    #[test]
    fn dct_finish_applies_decode_array_before_rgb_output() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "Decode",
            PdfObject::Array(vec![
                PdfObject::Integer(1),
                PdfObject::Integer(0),
                PdfObject::Integer(1),
                PdfObject::Integer(0),
                PdfObject::Integer(1),
                PdfObject::Integer(0),
            ]),
        );

        let image = ImageDecoder::finish_dct_decoded_image(
            vec![0, 128, 255],
            1,
            1,
            3,
            "DeviceRGB",
            &dict,
            None,
        )
        .unwrap();

        assert_eq!(image.channels, 3);
        assert_eq!(image.pixels, vec![255, 127, 0]);
    }

    #[test]
    fn jpx_smask_in_data_zero_drops_internal_alpha() {
        let mut dict = PdfDictionary::empty();
        dict.insert("SMaskInData", PdfObject::Integer(0));
        let raw = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30, 40],
        };

        let image = ImageDecoder::finish_jpx_decoded_image(raw, &dict);

        assert_eq!(image.channels, 3);
        assert_eq!(image.pixels, vec![10, 20, 30]);
    }

    #[test]
    fn jpx_smask_in_data_two_unpremultiplies_rgb() {
        let mut dict = PdfDictionary::empty();
        dict.insert("SMaskInData", PdfObject::Integer(2));
        let raw = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![64, 32, 16, 128],
        };

        let image = ImageDecoder::finish_jpx_decoded_image(raw, &dict);

        assert_eq!(image.channels, 4);
        assert_eq!(image.pixels, vec![128, 64, 32, 128]);
    }

    #[test]
    fn cmyk_to_rgb_pure_black() {
        let cmyk = vec![0u8, 0, 0, 255];
        assert_eq!(ColorSpaceConverter::cmyk_to_rgb(&cmyk), vec![35, 31, 32]);
    }

    #[test]
    fn cmyk_to_rgb_no_ink_is_white() {
        let cmyk = vec![0u8, 0, 0, 0];
        assert_eq!(ColorSpaceConverter::cmyk_to_rgb(&cmyk), vec![255, 255, 255]);
    }

    #[test]
    fn cmyk_to_rgb_pure_cyan() {
        let cmyk = vec![255u8, 0, 0, 0];
        let rgb = ColorSpaceConverter::cmyk_to_rgb(&cmyk);
        assert_eq!(rgb, vec![0, 173, 239]);
    }

    #[test]
    fn cmyk_to_rgb_pure_magenta() {
        let cmyk = vec![0u8, 255, 0, 0];
        let rgb = ColorSpaceConverter::cmyk_to_rgb(&cmyk);
        assert_eq!(rgb, vec![236, 0, 140]);
    }

    #[test]
    fn cmyk_to_rgb_processes_multiple_pixels() {
        let cmyk = vec![0u8, 0, 0, 0, 0, 0, 0, 255];
        let rgb = ColorSpaceConverter::cmyk_to_rgb(&cmyk);
        assert_eq!(rgb, vec![255, 255, 255, 35, 31, 32]);
    }

    #[test]
    fn indexed_cmyk_palette_converts_palette_entries() {
        let reader =
            crate::reader::PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf())
                .unwrap();
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("Indexed".to_string()),
                PdfObject::Name("DeviceCMYK".to_string()),
                PdfObject::Integer(1),
                PdfObject::String(vec![0, 0, 0, 0, 0, 0, 0, 255]),
            ]),
        );

        let (pixels, channels) =
            ColorSpaceConverter::decode_indexed(&[0, 1], &dict, &reader, 2, 1).unwrap();

        assert_eq!(channels, 3);
        assert_eq!(pixels, vec![255, 255, 255, 35, 31, 32]);
    }

    #[test]
    fn raw_image_is_valid_rejects_wrong_buffer_size() {
        let img = RawImage {
            width: 10,
            height: 10,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0u8; 100],
        };
        assert!(!img.is_valid());
    }

    #[test]
    fn raw_image_is_valid_accepts_correct_buffer() {
        let img = RawImage {
            width: 2,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0u8; 12],
        };
        assert!(img.is_valid());
        assert_eq!(img.byte_count(), 12);
        assert_eq!(img.pixel_count(), 4);
        assert_eq!(img.row_stride(), 6);
        assert!(img.is_rgb());
    }

    #[test]
    fn raw_image_pixel_accessor() {
        let img = RawImage {
            width: 2,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30, 40, 50, 60],
        };
        assert_eq!(img.pixel(0, 0), &[10, 20, 30]);
        assert_eq!(img.pixel(1, 0), &[40, 50, 60]);
        assert_eq!(img.pixel(3, 0), &[] as &[u8]);
    }

    #[test]
    fn build_raw_image_handles_device_gray() {
        let pixels = vec![0u8, 128, 255];
        let dict = PdfDictionary::empty();
        let img = ImageDecoder::build_raw_image_pub(pixels.clone(), 3, 1, 8, "DeviceGray", &dict)
            .unwrap();
        assert_eq!(img.channels, 1);
        assert_eq!(img.pixels, pixels);
        assert!(img.is_grayscale());
    }

    #[test]
    fn build_raw_image_applies_device_gray_decode_array() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "Decode",
            PdfObject::Array(vec![PdfObject::Real(1.0), PdfObject::Real(0.0)]),
        );
        let img =
            ImageDecoder::build_raw_image_pub(vec![0b1010_0000], 4, 1, 1, "DeviceGray", &dict)
                .unwrap();
        assert_eq!(img.channels, 1);
        assert_eq!(img.pixels, vec![0, 255, 0, 255]);
    }

    #[test]
    fn build_raw_image_handles_device_cmyk_to_rgb() {
        let pixels = vec![0u8, 0, 0, 0];
        let dict = PdfDictionary::empty();
        let img = ImageDecoder::build_raw_image_pub(pixels, 1, 1, 8, "DeviceCMYK", &dict).unwrap();
        assert_eq!(img.channels, 3);
        assert_eq!(img.pixels, vec![255, 255, 255]);
    }

    #[test]
    fn build_raw_image_pads_and_truncates_mismatched_buffers() {
        let dict = PdfDictionary::empty();
        let img =
            ImageDecoder::build_raw_image_pub(vec![255], 2, 1, 8, "DeviceRGB", &dict).unwrap();
        assert_eq!(img.pixels, vec![255, 0, 0, 0, 0, 0]);

        let img = ImageDecoder::build_raw_image_pub(vec![1, 2, 3, 4], 1, 1, 8, "DeviceGray", &dict)
            .unwrap();
        assert_eq!(img.pixels, vec![1]);
    }

    #[test]
    fn image_decode_params_selects_matching_filter_entry() {
        let mut ccitt_params = PdfDictionary::empty();
        ccitt_params.insert("K", PdfObject::Integer(-1));
        ccitt_params.insert("BlackIs1", PdfObject::Boolean(true));

        let mut dict = PdfDictionary::empty();
        dict.insert(
            "Filter",
            PdfObject::Array(vec![
                PdfObject::Name("FlateDecode".to_string()),
                PdfObject::Name("CCF".to_string()),
            ]),
        );
        dict.insert(
            "DecodeParms",
            PdfObject::Array(vec![
                PdfObject::Null,
                PdfObject::Dictionary(ccitt_params.clone()),
            ]),
        );

        let selected = image_decode_params(&dict, None, "CCITTFaxDecode")
            .unwrap()
            .unwrap();
        assert_eq!(selected.get_integer("K"), Some(-1));
        assert_eq!(selected.get_bool("BlackIs1"), Some(true));
    }

    #[test]
    fn ccitt_params_use_image_dimensions_for_defaults() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Rows", PdfObject::Integer(0));

        let params = ccitt_decode_params(Some(&dict), 17, 23).unwrap();
        assert_eq!(params.columns, 17);
        assert_eq!(params.rows, 23);
        assert_eq!(params.k, 0);
        assert!(!params.black_is_1);
        assert!(params.end_of_block);
    }

    #[test]
    fn jpeg_decode_round_trip() {
        let mut pixels = Vec::new();
        for y in 0..4u8 {
            for x in 0..4u8 {
                pixels.push(x * 64);
                pixels.push(y * 64);
                pixels.push(128u8);
            }
        }
        let original = RawImage {
            width: 4,
            height: 4,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        };
        let jpeg = ImageEncoder::encode_jpeg(&original, 95).unwrap();
        let (decoded_pixels, width, height, channels) =
            ImageDecoder::decode_jpeg_with_info(&jpeg).unwrap();
        assert_eq!(width, 4);
        assert_eq!(height, 4);
        assert_eq!(channels, 3);
        assert_eq!(decoded_pixels.len(), 4 * 4 * 3);
    }

    // ---- H-4/H-5/H-6: decode-layer resource caps ----

    #[test]
    fn ensure_decode_budget_rejects_oversized_dimensions() {
        // 60000 x 60000 = 3.6e9 pixels, far over the 100M default cap.
        let err = ensure_decode_budget(60_000, 60_000, 1).unwrap_err();
        assert!(
            matches!(err, WellfriendError::MalformedPdf(_)),
            "huge dimensions must be a clean MalformedPdf error, got {err:?}"
        );
        // A legitimately large image (12 MP) is well under the cap and allowed.
        assert!(ensure_decode_budget(4000, 3000, 3).is_ok());
    }

    #[test]
    fn h4_build_raw_image_rejects_decode_bomb_before_allocating() {
        // A few-hundred-byte stream declaring 60000x60000 at 1 bpc would, before
        // the cap, force a ~3.6 GB Vec::with_capacity in unpack_subbyte_rows.
        // It must now fail closed with a clean error instead of OOMing.
        let dict = PdfDictionary::empty();
        let result = ImageDecoder::build_raw_image_pub(
            vec![0u8; 64],
            60_000,
            60_000,
            1,
            "DeviceGray",
            &dict,
        );
        assert!(
            result.is_err(),
            "oversized image must be rejected before allocation"
        );
    }

    #[test]
    fn h4_legitimate_large_image_still_decodes() {
        // 1000x1000 DeviceGray (1 MP) with full data decodes fine — the cap does
        // not reject normal large-but-valid content.
        let dict = PdfDictionary::empty();
        let pixels = vec![128u8; 1000 * 1000];
        let img =
            ImageDecoder::build_raw_image_pub(pixels, 1000, 1000, 8, "DeviceGray", &dict).unwrap();
        assert_eq!(img.width, 1000);
        assert_eq!(img.height, 1000);
        assert_eq!(img.pixels.len(), 1000 * 1000);
    }

    #[test]
    fn short_subbyte_rows_are_zero_filled_without_panicking() {
        let unpacked = unpack_subbyte_rows(&[0b1010_0000], 4, 4, 1, 1);
        assert_eq!(unpacked.len(), 16);
        assert_eq!(&unpacked[..4], &[255, 0, 255, 0]);
        assert!(unpacked[4..].iter().all(|sample| *sample == 0));
    }
}
