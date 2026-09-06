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
use crate::render::cmm::{self, ColorTransformOptions};

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

struct DecodedJpeg {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    channels: u8,
    original_width: u32,
    original_height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RawImageDecodeWindow {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Requested raw source components for a raw-window decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RawImageComponentSelection {
    All,
    Components(Vec<u8>),
}

impl RawImageComponentSelection {
    fn selected_channel_count(&self, source_channels: u8) -> Result<u8> {
        match self {
            Self::All => Ok(source_channels.max(1)),
            Self::Components(components) => {
                validate_raw_component_selection(components, source_channels)
            }
        }
    }

    fn components(&self) -> Option<&[u8]> {
        match self {
            Self::All => None,
            Self::Components(components) => Some(components.as_slice()),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ImageColorContext<'a> {
    reader: Option<&'a PdfReader>,
    options: ColorTransformOptions,
}

impl<'a> ImageColorContext<'a> {
    fn with_reader(reader: &'a PdfReader, options: ColorTransformOptions) -> Self {
        Self {
            reader: Some(reader),
            options,
        }
    }
}

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
        Self::decode_with_limits_and_color_transform_options(
            image,
            reader,
            limits,
            ColorTransformOptions::default(),
        )
    }

    pub(crate) fn decode_with_limits_and_color_transform_options(
        image: &ImageReference,
        reader: &PdfReader,
        limits: &DecodeLimits,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        Self::decode_with_limits_and_color_space_override(
            image,
            reader,
            limits,
            None,
            None,
            color_options,
        )
    }

    pub(crate) fn decode_jpeg_scaled_with_limits_and_color_transform_options(
        image: &ImageReference,
        reader: &PdfReader,
        color_space_override: Option<&(String, PdfObject)>,
        requested_width: u32,
        requested_height: u32,
        limits: &DecodeLimits,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        if image.object_number == 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "inline JPEG scaled decode via image reference is not supported".to_string(),
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
        let mut effective_dict = dict.clone();
        if let Some((name, space_obj)) = color_space_override {
            effective_image.color_space = name.clone();
            effective_dict.insert("ColorSpace", space_obj.clone());
        }

        let stream_obj = PdfObject::Stream {
            dict: dict.clone(),
            raw,
        };
        let decoded = decode_stream_lossless_with_limits(&stream_obj, reader, limits)?;
        match decoded.status {
            StreamDecodeStatus::StoppedAtImageFilter(filter)
                if matches!(filter.as_str(), "DCTDecode" | "DCT") =>
            {
                let jpeg = Self::decode_jpeg_scaled_with_info(
                    &decoded.data,
                    requested_width,
                    requested_height,
                )?;
                ensure_dct_dimensions_match(
                    &format!("image {}", image.xobject_name),
                    effective_image.width,
                    effective_image.height,
                    jpeg.original_width,
                    jpeg.original_height,
                )?;
                Self::finish_dct_decoded_image(
                    jpeg.pixels,
                    jpeg.width,
                    jpeg.height,
                    jpeg.channels,
                    &effective_image.color_space,
                    &effective_dict,
                    ImageColorContext::with_reader(reader, color_options),
                )
            }
            StreamDecodeStatus::StoppedAtImageFilter(filter) => {
                Err(WellfriendError::UnsupportedFeature(format!(
                    "JPEG scaled decode cannot handle final image filter {filter}"
                )))
            }
            StreamDecodeStatus::Complete => Err(WellfriendError::UnsupportedFeature(
                "JPEG scaled decode requires a DCTDecode image filter".to_string(),
            )),
        }
    }

    pub(crate) fn decode_jpx_scaled_with_limits(
        image: &ImageReference,
        reader: &PdfReader,
        color_space_override: Option<&(String, PdfObject)>,
        requested_width: u32,
        requested_height: u32,
        limits: &DecodeLimits,
    ) -> Result<RawImage> {
        if image.object_number == 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "inline JPX target-resolution decode via image reference is not supported"
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
        let mut effective_dict = dict.clone();
        if let Some((_, space_obj)) = color_space_override {
            effective_dict.insert("ColorSpace", space_obj.clone());
        }

        let stream_obj = PdfObject::Stream {
            dict: dict.clone(),
            raw,
        };
        let decoded = decode_stream_lossless_with_limits(&stream_obj, reader, limits)?;
        match decoded.status {
            StreamDecodeStatus::StoppedAtImageFilter(filter)
                if matches!(filter.as_str(), "JPXDecode" | "JPX") =>
            {
                let jpx = jpx::decode_with_target_resolution(
                    &decoded.data,
                    Some((requested_width, requested_height)),
                )?;
                Self::finish_reduced_jpx_decoded_image(
                    jpx.raw,
                    image.width,
                    image.height,
                    jpx.original_width,
                    jpx.original_height,
                    &format!("image {}", image.xobject_name),
                    &effective_dict,
                )
            }
            StreamDecodeStatus::StoppedAtImageFilter(filter) => {
                Err(WellfriendError::UnsupportedFeature(format!(
                    "JPX target-resolution decode cannot handle final image filter {filter}"
                )))
            }
            StreamDecodeStatus::Complete => Err(WellfriendError::UnsupportedFeature(
                "JPX target-resolution decode requires a JPXDecode image filter".to_string(),
            )),
        }
    }

    /// Decode an image XObject while overriding a named `/ColorSpace` resource
    /// with the already-resolved colour-space object from the active page/Form
    /// resources. This keeps the core image decoder source-linked to the real
    /// image stream while allowing render-time resource inheritance to supply
    /// CalRGB/Lab/ICCBased/Indexed/Separation/DeviceN details that are not
    /// present in the image dictionary itself.
    pub(crate) fn decode_with_resolved_color_space_and_limits_and_color_transform_options(
        image: &ImageReference,
        reader: &PdfReader,
        color_space_name: &str,
        color_space_obj: &PdfObject,
        limits: &DecodeLimits,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        Self::decode_with_limits_and_color_space_override(
            image,
            reader,
            limits,
            Some(color_space_name),
            Some(color_space_obj),
            color_options,
        )
    }

    pub(crate) fn decode_ccitt_window_with_limits(
        image: &ImageReference,
        reader: &PdfReader,
        color_space_override: Option<&(String, PdfObject)>,
        window: ccitt::CcittDecodeWindow,
        limits: &DecodeLimits,
    ) -> Result<RawImage> {
        if image.object_number == 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "inline CCITT window decoding via image reference is not supported".to_string(),
            ));
        }

        let obj = reader.get_object(image.object_number, image.generation_number)?;
        let (dict, raw) = match obj {
            PdfObject::Stream { dict, raw } => (dict, raw),
            _ => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "image object {} is not a stream",
                    image.object_number
                )));
            }
        };
        let stream_obj = PdfObject::Stream {
            dict: dict.clone(),
            raw,
        };
        let decoded = decode_stream_lossless_with_limits(&stream_obj, reader, limits)?;
        match decoded.status {
            StreamDecodeStatus::StoppedAtImageFilter(filter)
                if matches!(filter.as_str(), "CCITTFaxDecode" | "CCF") =>
            {
                let effective_color_space = color_space_override
                    .map(|(name, _)| name.as_str())
                    .unwrap_or(&image.color_space);
                Self::ensure_monochrome_terminal_color_space(&filter, effective_color_space)?;
                let decode_params = image_decode_params(&dict, Some(reader), &filter)?;
                let params =
                    ccitt_decode_params(decode_params.as_ref(), image.width, image.height)?;
                ccitt::decode_window(&decoded.data, params, window)
            }
            StreamDecodeStatus::StoppedAtImageFilter(filter) => {
                Err(WellfriendError::UnsupportedFeature(format!(
                    "CCITT window decode cannot handle final image filter {filter}"
                )))
            }
            StreamDecodeStatus::Complete => Err(WellfriendError::UnsupportedFeature(
                "CCITT window decode requires a CCITTFaxDecode image filter".to_string(),
            )),
        }
    }

    pub(crate) fn decode_raw_window_with_limits_and_color_transform_options(
        image: &ImageReference,
        reader: &PdfReader,
        color_space_override: Option<&(String, PdfObject)>,
        window: RawImageDecodeWindow,
        limits: &DecodeLimits,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        Self::decode_raw_window_components_with_limits_and_color_transform_options(
            image,
            reader,
            color_space_override,
            window,
            RawImageComponentSelection::All,
            limits,
            color_options,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_raw_window_components_with_limits_and_color_transform_options(
        image: &ImageReference,
        reader: &PdfReader,
        color_space_override: Option<&(String, PdfObject)>,
        window: RawImageDecodeWindow,
        component_selection: RawImageComponentSelection,
        limits: &DecodeLimits,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        if image.object_number == 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "inline raw window decoding via image reference is not supported".to_string(),
            ));
        }
        if !image.filter.is_empty() {
            return Err(WellfriendError::UnsupportedFeature(
                "raw image window decoding requires an unfiltered image stream".to_string(),
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
        let mut effective_dict = dict.clone();
        if let Some((name, space_obj)) = color_space_override {
            effective_image.color_space = name.clone();
            effective_dict.insert("ColorSpace", space_obj.clone());
        }

        let channels = Self::raw_source_channel_count(
            effective_image.is_mask,
            &effective_image.color_space,
            &effective_dict,
            Some(reader),
        )?;
        let (cropped, selected_channels) = crop_raw_window_with_component_selection(
            &raw,
            effective_image.width,
            effective_image.height,
            channels,
            effective_image.bits_per_component,
            window,
            &component_selection,
            limits,
        )?;
        if component_selection.components().is_some() {
            return Self::build_raw_component_image(
                cropped,
                window.width,
                window.height,
                channels,
                selected_channels,
                effective_image.bits_per_component,
                &effective_image.color_space,
                &effective_dict,
                component_selection,
            );
        }
        Self::build_raw_image(
            cropped,
            window.width,
            window.height,
            effective_image.bits_per_component,
            &effective_image.color_space,
            &effective_dict,
            ImageColorContext::with_reader(reader, color_options),
        )
    }

    fn decode_with_limits_and_color_space_override(
        image: &ImageReference,
        reader: &PdfReader,
        limits: &DecodeLimits,
        color_space_name: Option<&str>,
        color_space_obj: Option<&PdfObject>,
        color_options: ColorTransformOptions,
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
                ImageColorContext::with_reader(reader, color_options),
            ),
            StreamDecodeStatus::StoppedAtImageFilter(filter) => {
                Self::decode_remaining_image_filter(
                    &decoded.data,
                    &filter,
                    &effective_image,
                    reader,
                    &effective_dict,
                    limits,
                    color_options,
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
                    let (pixels, w, h, channels) = Self::decode_jpeg_with_info(&decompressed)?;
                    ensure_dct_dimensions_match("inline image", width, height, w, h)?;
                    return Self::finish_dct_decoded_image(
                        pixels,
                        w,
                        h,
                        channels,
                        color_space,
                        &PdfDictionary::empty(),
                        ImageColorContext::default(),
                    );
                }
                "CCITTFaxDecode" | "CCF" => {
                    Self::ensure_monochrome_terminal_color_space(last_filter, color_space)?;
                    let params = ccitt_decode_params(decode_parms, width, height)?;
                    return ccitt::decode(&decompressed, params);
                }
                "JBIG2Decode" => {
                    Self::ensure_monochrome_terminal_color_space(last_filter, color_space)?;
                    return jbig2::decode(&decompressed, None);
                }
                "JPXDecode" | "JPX" => {
                    return Self::finish_jpx_decoded_image(
                        jpx::decode(&decompressed)?,
                        width,
                        height,
                        "inline image",
                        &PdfDictionary::empty(),
                    );
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
            ImageColorContext::default(),
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
        Self::decode_inline_with_resolved_color_space_and_param_array(
            pixel_data,
            width,
            height,
            bpc,
            color_space,
            None,
            filters,
            decode_params,
            limits,
            None,
            ColorTransformOptions::default(),
        )
    }

    /// Decode an inline image while preserving an already-resolved
    /// `/ColorSpace` resource object in the effective image dictionary. This
    /// lets regional SVG/PS inline-image replay use the same calibrated,
    /// Indexed, ICCBased, and tint-space conversion helpers as image XObjects
    /// without requiring inline payloads to be promoted to synthetic streams.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_inline_with_resolved_color_space_and_param_array(
        pixel_data: &[u8],
        width: u32,
        height: u32,
        bpc: u8,
        color_space: &str,
        color_space_obj: Option<&PdfObject>,
        filters: &[&str],
        decode_params: &[Option<PdfDictionary>],
        limits: &DecodeLimits,
        reader: Option<&PdfReader>,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        if decode_params.len() != filters.len() {
            return Err(WellfriendError::MalformedPdf(format!(
                "inline DecodeParms count {} does not match filter count {}",
                decode_params.len(),
                filters.len()
            )));
        }
        let mut effective_dict = PdfDictionary::empty();
        if let Some(space_obj) = color_space_obj {
            effective_dict.insert("ColorSpace", space_obj.clone());
        }
        let color_context = ImageColorContext {
            reader,
            options: color_options,
        };
        let mut data = pixel_data.to_vec();
        for (index, &filter) in filters.iter().enumerate() {
            let params = decode_params[index].as_ref();
            if matches!(
                filter,
                "DCTDecode"
                    | "DCT"
                    | "JPXDecode"
                    | "JPX"
                    | "CCITTFaxDecode"
                    | "CCF"
                    | "JBIG2Decode"
            ) {
                if index + 1 != filters.len() {
                    return Err(WellfriendError::UnsupportedFeature(
                        "inline image codec filter must be the final filter".to_string(),
                    ));
                }
                return match filter {
                    "DCTDecode" | "DCT" => {
                        let (pixels, w, h, channels) = Self::decode_jpeg_with_info(&data)?;
                        ensure_dct_dimensions_match("inline image", width, height, w, h)?;
                        Self::finish_dct_decoded_image(
                            pixels,
                            w,
                            h,
                            channels,
                            color_space,
                            &effective_dict,
                            color_context,
                        )
                    }
                    "CCITTFaxDecode" | "CCF" => {
                        Self::ensure_monochrome_terminal_color_space(filter, color_space)?;
                        ccitt::decode(&data, ccitt_decode_params(params, width, height)?)
                    }
                    "JBIG2Decode" => {
                        Self::ensure_monochrome_terminal_color_space(filter, color_space)?;
                        jbig2::decode(&data, None)
                    }
                    "JPXDecode" | "JPX" => Self::finish_jpx_decoded_image(
                        jpx::decode(&data)?,
                        width,
                        height,
                        "inline image",
                        &effective_dict,
                    ),
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
            &effective_dict,
            color_context,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_inline_raw_window_with_resolved_color_space_and_param_array(
        pixel_data: &[u8],
        width: u32,
        height: u32,
        bpc: u8,
        color_space: &str,
        color_space_obj: Option<&PdfObject>,
        filters: &[&str],
        decode_params: &[Option<PdfDictionary>],
        window: RawImageDecodeWindow,
        limits: &DecodeLimits,
        reader: Option<&PdfReader>,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        Self::decode_inline_raw_window_components_with_resolved_color_space_and_param_array(
            pixel_data,
            width,
            height,
            bpc,
            color_space,
            color_space_obj,
            filters,
            decode_params,
            window,
            RawImageComponentSelection::All,
            limits,
            reader,
            color_options,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_inline_raw_window_components_with_resolved_color_space_and_param_array(
        pixel_data: &[u8],
        width: u32,
        height: u32,
        bpc: u8,
        color_space: &str,
        color_space_obj: Option<&PdfObject>,
        filters: &[&str],
        decode_params: &[Option<PdfDictionary>],
        window: RawImageDecodeWindow,
        component_selection: RawImageComponentSelection,
        limits: &DecodeLimits,
        reader: Option<&PdfReader>,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        if decode_params.len() != filters.len() {
            return Err(WellfriendError::MalformedPdf(format!(
                "inline DecodeParms count {} does not match filter count {}",
                decode_params.len(),
                filters.len()
            )));
        }
        if !filters.is_empty() {
            return Err(WellfriendError::UnsupportedFeature(
                "inline raw window decoding requires an unfiltered inline image".to_string(),
            ));
        }
        let mut effective_dict = PdfDictionary::empty();
        if let Some(space_obj) = color_space_obj {
            effective_dict.insert("ColorSpace", space_obj.clone());
        }
        let channels = Self::raw_source_channel_count(false, color_space, &effective_dict, reader)?;
        let (cropped, selected_channels) = crop_raw_window_with_component_selection(
            pixel_data,
            width,
            height,
            channels,
            bpc,
            window,
            &component_selection,
            limits,
        )?;
        if component_selection.components().is_some() {
            return Self::build_raw_component_image(
                cropped,
                window.width,
                window.height,
                channels,
                selected_channels,
                bpc,
                color_space,
                &effective_dict,
                component_selection,
            );
        }
        Self::build_raw_image(
            cropped,
            window.width,
            window.height,
            bpc,
            color_space,
            &effective_dict,
            ImageColorContext {
                reader,
                options: color_options,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_inline_scaled_dct_with_resolved_color_space_and_param_array(
        pixel_data: &[u8],
        width: u32,
        height: u32,
        color_space: &str,
        color_space_obj: Option<&PdfObject>,
        filters: &[&str],
        decode_params: &[Option<PdfDictionary>],
        requested_width: u32,
        requested_height: u32,
        limits: &DecodeLimits,
        reader: Option<&PdfReader>,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        if decode_params.len() != filters.len() {
            return Err(WellfriendError::MalformedPdf(format!(
                "inline DecodeParms count {} does not match filter count {}",
                decode_params.len(),
                filters.len()
            )));
        }
        let mut effective_dict = PdfDictionary::empty();
        if let Some(space_obj) = color_space_obj {
            effective_dict.insert("ColorSpace", space_obj.clone());
        }
        let color_context = ImageColorContext {
            reader,
            options: color_options,
        };
        let mut data = pixel_data.to_vec();
        for (index, &filter) in filters.iter().enumerate() {
            let params = decode_params[index].as_ref();
            if matches!(
                filter,
                "DCTDecode"
                    | "DCT"
                    | "JPXDecode"
                    | "JPX"
                    | "CCITTFaxDecode"
                    | "CCF"
                    | "JBIG2Decode"
            ) {
                if index + 1 != filters.len() {
                    return Err(WellfriendError::UnsupportedFeature(
                        "inline image codec filter must be the final filter".to_string(),
                    ));
                }
                return match filter {
                    "DCTDecode" | "DCT" => {
                        let jpeg = Self::decode_jpeg_scaled_with_info(
                            &data,
                            requested_width,
                            requested_height,
                        )?;
                        ensure_dct_dimensions_match(
                            "inline image",
                            width,
                            height,
                            jpeg.original_width,
                            jpeg.original_height,
                        )?;
                        Self::finish_dct_decoded_image(
                            jpeg.pixels,
                            jpeg.width,
                            jpeg.height,
                            jpeg.channels,
                            color_space,
                            &effective_dict,
                            color_context,
                        )
                    }
                    other => Err(WellfriendError::UnsupportedFeature(format!(
                        "inline JPEG scaled decode cannot handle final image filter {other}"
                    ))),
                };
            }
            data = apply_filter_bytes_with_limits(filter, &data, params, limits)?;
        }
        Err(WellfriendError::UnsupportedFeature(
            "inline JPEG scaled decode requires a DCTDecode image filter".to_string(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_inline_scaled_jpx_with_resolved_color_space_and_param_array(
        pixel_data: &[u8],
        width: u32,
        height: u32,
        color_space_obj: Option<&PdfObject>,
        filters: &[&str],
        decode_params: &[Option<PdfDictionary>],
        requested_width: u32,
        requested_height: u32,
        limits: &DecodeLimits,
    ) -> Result<RawImage> {
        if decode_params.len() != filters.len() {
            return Err(WellfriendError::MalformedPdf(format!(
                "inline DecodeParms count {} does not match filter count {}",
                decode_params.len(),
                filters.len()
            )));
        }
        let mut effective_dict = PdfDictionary::empty();
        if let Some(space_obj) = color_space_obj {
            effective_dict.insert("ColorSpace", space_obj.clone());
        }
        let mut data = pixel_data.to_vec();
        for (index, &filter) in filters.iter().enumerate() {
            let params = decode_params[index].as_ref();
            if matches!(
                filter,
                "DCTDecode"
                    | "DCT"
                    | "JPXDecode"
                    | "JPX"
                    | "CCITTFaxDecode"
                    | "CCF"
                    | "JBIG2Decode"
            ) {
                if index + 1 != filters.len() {
                    return Err(WellfriendError::UnsupportedFeature(
                        "inline image codec filter must be the final filter".to_string(),
                    ));
                }
                return match filter {
                    "JPXDecode" | "JPX" => {
                        let jpx = jpx::decode_with_target_resolution(
                            &data,
                            Some((requested_width, requested_height)),
                        )?;
                        Self::finish_reduced_jpx_decoded_image(
                            jpx.raw,
                            width,
                            height,
                            jpx.original_width,
                            jpx.original_height,
                            "inline image",
                            &effective_dict,
                        )
                    }
                    other => Err(WellfriendError::UnsupportedFeature(format!(
                        "inline JPX target-resolution decode cannot handle final image filter {other}"
                    ))),
                };
            }
            data = apply_filter_bytes_with_limits(filter, &data, params, limits)?;
        }
        Err(WellfriendError::UnsupportedFeature(
            "inline JPX target-resolution decode requires a JPXDecode image filter".to_string(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_inline_ccitt_window_with_param_array(
        pixel_data: &[u8],
        width: u32,
        height: u32,
        color_space: &str,
        filters: &[&str],
        decode_params: &[Option<PdfDictionary>],
        window: ccitt::CcittDecodeWindow,
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
                "DCTDecode"
                    | "DCT"
                    | "JPXDecode"
                    | "JPX"
                    | "CCITTFaxDecode"
                    | "CCF"
                    | "JBIG2Decode"
            ) {
                if index + 1 != filters.len() {
                    return Err(WellfriendError::UnsupportedFeature(
                        "inline image codec filter must be the final filter".to_string(),
                    ));
                }
                return match filter {
                    "CCITTFaxDecode" | "CCF" => {
                        Self::ensure_monochrome_terminal_color_space(filter, color_space)?;
                        ccitt::decode_window(
                            &data,
                            ccitt_decode_params(params, width, height)?,
                            window,
                        )
                    }
                    other => Err(WellfriendError::UnsupportedFeature(format!(
                        "inline CCITT window decode cannot handle final image filter {other}"
                    ))),
                };
            }
            data = apply_filter_bytes_with_limits(filter, &data, params, limits)?;
        }
        Err(WellfriendError::UnsupportedFeature(
            "inline CCITT window decode requires a CCITTFaxDecode image filter".to_string(),
        ))
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
        ensure_dct_dimensions_match(
            &format!("image {}", image.xobject_name),
            image.width,
            image.height,
            width,
            height,
        )?;
        Self::finish_dct_decoded_image(
            pixels,
            width,
            height,
            channels,
            &image.color_space,
            &dict,
            ImageColorContext::with_reader(reader, ColorTransformOptions::default()),
        )
    }

    /// Decode JPEG bytes and return pixels plus width, height, channel count.
    pub fn decode_jpeg_with_info(jpeg_bytes: &[u8]) -> Result<(Vec<u8>, u32, u32, u8)> {
        let jpeg = decode_jpeg_with_requested_size(jpeg_bytes, None)?;
        Ok((jpeg.pixels, jpeg.width, jpeg.height, jpeg.channels))
    }

    fn decode_jpeg_scaled_with_info(
        jpeg_bytes: &[u8],
        requested_width: u32,
        requested_height: u32,
    ) -> Result<DecodedJpeg> {
        decode_jpeg_with_requested_size(jpeg_bytes, Some((requested_width, requested_height)))
    }
}

fn decode_jpeg_with_requested_size(
    jpeg_bytes: &[u8],
    requested_size: Option<(u32, u32)>,
) -> Result<DecodedJpeg> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(jpeg_bytes));
    decoder
        .read_info()
        .map_err(|e| WellfriendError::MalformedPdf(format!("JPEG metadata decode failed: {e}")))?;
    let info = decoder.info().ok_or_else(|| {
        WellfriendError::MalformedPdf("JPEG decode: no metadata after read_info".to_string())
    })?;
    let original_width = u32::from(info.width);
    let original_height = u32::from(info.height);
    if let Some((requested_width, requested_height)) = requested_size {
        let requested_width = requested_width.clamp(1, u32::from(u16::MAX)) as u16;
        let requested_height = requested_height.clamp(1, u32::from(u16::MAX)) as u16;
        decoder
            .scale(requested_width, requested_height)
            .map_err(|e| WellfriendError::MalformedPdf(format!("JPEG scale failed: {e}")))?;
    }
    let info = decoder.info().ok_or_else(|| {
        WellfriendError::MalformedPdf("JPEG decode: no metadata after scale".to_string())
    })?;
    let channels = jpeg_channels(info.pixel_format)?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    ensure_decode_budget(width, height, channels)?;
    let max_decoded = expected_len(width, height, channels);
    decoder.set_max_decoding_buffer_size(max_decoded);
    let pixels = decoder
        .decode()
        .map_err(|e| WellfriendError::MalformedPdf(format!("JPEG decode failed: {e}")))?;
    ensure_decoded_len(pixels.len(), width, height, channels, max_decoded)?;
    Ok(DecodedJpeg {
        pixels,
        width,
        height,
        channels,
        original_width,
        original_height,
    })
}

fn jpeg_channels(pixel_format: jpeg_decoder::PixelFormat) -> Result<u8> {
    match pixel_format {
        jpeg_decoder::PixelFormat::L8 => Ok(1),
        jpeg_decoder::PixelFormat::RGB24 => Ok(3),
        jpeg_decoder::PixelFormat::CMYK32 => Ok(4),
        other => Err(WellfriendError::UnsupportedFeature(format!(
            "unsupported JPEG pixel format: {other:?}"
        ))),
    }
}

impl ImageDecoder {
    pub(crate) fn normalise_bit_depth(
        raw: Vec<u8>,
        width: u32,
        height: u32,
        channels: u8,
        bpc: u8,
    ) -> Result<Vec<u8>> {
        match bpc {
            8 => Ok(raw),
            16 => normalise_16_bit_samples(&raw, width, height, channels),
            4 | 2 | 1 => unpack_subbyte_rows(&raw, width, height, channels, bpc),
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
        Self::build_raw_image(
            decompressed,
            width,
            height,
            bpc,
            color_space,
            dict,
            ImageColorContext::default(),
        )
    }

    fn raw_source_channel_count(
        is_mask: bool,
        color_space: &str,
        dict: &PdfDictionary,
        reader: Option<&PdfReader>,
    ) -> Result<u8> {
        if is_mask || image_mask_flag(dict)? {
            Ok(1)
        } else {
            Self::raw_channel_count(color_space, dict, reader)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_raw_component_image(
        decompressed: Vec<u8>,
        width: u32,
        height: u32,
        source_channels: u8,
        selected_channels: u8,
        bpc: u8,
        color_space: &str,
        dict: &PdfDictionary,
        component_selection: RawImageComponentSelection,
    ) -> Result<RawImage> {
        ensure_decode_budget(width, height, selected_channels.max(1))?;
        if width == 0 || height == 0 {
            return Ok(RawImage {
                width,
                height,
                channels: selected_channels.max(1),
                bits_per_sample: 8,
                pixels: Vec::new(),
            });
        }
        if decompressed.is_empty() {
            return Err(WellfriendError::MalformedPdf(format!(
                "raw image component selection {width}x{height} decoded to empty pixel data"
            )));
        }

        let mut normalised =
            Self::normalise_bit_depth(decompressed, width, height, selected_channels, bpc)?;
        let expected_size = expected_len(width, height, selected_channels);
        ensure_decoded_len(
            normalised.len(),
            width,
            height,
            selected_channels,
            expected_size,
        )?;
        if !image_mask_flag(dict)? && decode_array_applies_before_color_conversion(color_space) {
            apply_decode_array_for_component_selection(
                &mut normalised,
                source_channels,
                selected_channels,
                &component_selection,
                dict,
            )?;
        }

        Ok(RawImage {
            width,
            height,
            channels: selected_channels,
            bits_per_sample: 8,
            pixels: normalised,
        })
    }

    fn decode_remaining_image_filter(
        data: &[u8],
        filter: &str,
        image: &ImageReference,
        reader: &PdfReader,
        dict: &PdfDictionary,
        limits: &DecodeLimits,
        color_options: ColorTransformOptions,
    ) -> Result<RawImage> {
        match filter {
            "DCTDecode" | "DCT" => {
                let (pixels, width, height, channels) = Self::decode_jpeg_with_info(data)?;
                ensure_dct_dimensions_match(
                    &format!("image {}", image.xobject_name),
                    image.width,
                    image.height,
                    width,
                    height,
                )?;
                Self::finish_dct_decoded_image(
                    pixels,
                    width,
                    height,
                    channels,
                    &image.color_space,
                    dict,
                    ImageColorContext::with_reader(reader, color_options),
                )
            }
            "JPXDecode" | "JPX" => Self::finish_jpx_decoded_image(
                jpx::decode(data)?,
                image.width,
                image.height,
                &format!("image {}", image.xobject_name),
                dict,
            ),
            "CCITTFaxDecode" | "CCF" => {
                Self::ensure_monochrome_terminal_color_space(filter, &image.color_space)?;
                let decode_params = image_decode_params(dict, Some(reader), filter)?;
                let params =
                    ccitt_decode_params(decode_params.as_ref(), image.width, image.height)?;
                ccitt::decode(data, params)
            }
            "JBIG2Decode" => {
                Self::ensure_monochrome_terminal_color_space(filter, &image.color_space)?;
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

    fn ensure_monochrome_terminal_color_space(filter: &str, color_space: &str) -> Result<()> {
        if !matches!(filter, "CCITTFaxDecode" | "CCF" | "JBIG2Decode") {
            return Ok(());
        }
        if matches!(color_space, "DeviceGray" | "G") {
            return Ok(());
        }
        Err(WellfriendError::UnsupportedFeature(format!(
            "{filter} terminal image decodes monochrome samples but PDF ColorSpace /{color_space} requires color conversion"
        )))
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
                "DCTDecode"
                    | "DCT"
                    | "JPXDecode"
                    | "JPX"
                    | "CCITTFaxDecode"
                    | "CCF"
                    | "JBIG2Decode"
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
        color_context: ImageColorContext<'_>,
    ) -> Result<RawImage> {
        let reader = color_context.reader;
        let is_mask = match dict.get("ImageMask").or_else(|| dict.get("IM")) {
            Some(PdfObject::Boolean(value)) => *value,
            Some(other) => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "image /ImageMask must be a boolean, got {}",
                    other.variant_name()
                )));
            }
            _ => false,
        };
        let raw_channels = if is_mask {
            1
        } else {
            Self::raw_channel_count(color_space, dict, reader)?
        };
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
        let expected_raw_size = expected_len(width, height, raw_channels);
        ensure_decoded_len(
            normalised.len(),
            width,
            height,
            raw_channels,
            expected_raw_size,
        )?;
        if is_mask {
            let pixels = normalised;
            return Ok(RawImage {
                width,
                height,
                channels: 1,
                bits_per_sample: 8,
                pixels,
            });
        }

        if decode_array_applies_before_color_conversion(color_space) {
            Self::apply_decode_array(&mut normalised, raw_channels, dict)?;
        }

        let (pixels, channels) = match color_space {
            "DeviceGray" | "G" => (normalised, 1u8),
            "CalGray" => {
                let params = cmm::try_cal_gray_params_from_image_dict(dict, reader)
                    .map_err(WellfriendError::MalformedPdf)?;
                (cmm::cal_gray_bytes_to_rgb(&normalised, params), 3u8)
            }
            "DeviceRGB" | "RGB" | "sRGB" => (normalised, 3u8),
            "CalRGB" => {
                let params = cmm::try_cal_rgb_params_from_image_dict(dict, reader)
                    .map_err(WellfriendError::MalformedPdf)?;
                (cmm::cal_rgb_bytes_to_rgb(&normalised, params), 3u8)
            }
            "DeviceCMYK" | "CMYK" => (ColorSpaceConverter::cmyk_to_rgb(&normalised), 3u8),
            "Lab" => {
                let params = cmm::try_lab_params_from_image_dict(dict, reader)
                    .map_err(WellfriendError::MalformedPdf)?;
                (cmm::lab_bytes_to_rgb(&normalised, params), 3u8)
            }
            "Indexed" => {
                if let Some(reader) = reader {
                    ColorSpaceConverter::decode_indexed(
                        &normalised,
                        bpc,
                        dict,
                        reader,
                        width,
                        height,
                    )?
                } else {
                    return Err(WellfriendError::UnsupportedFeature(
                        "Indexed image ColorSpace requires reader-backed palette resolution"
                            .to_string(),
                    ));
                }
            }
            "Separation" | "DeviceN" => {
                if let Some(reader) = reader {
                    ColorSpaceConverter::tint_space_to_rgba(
                        &normalised,
                        dict,
                        reader,
                        raw_channels,
                        color_context.options,
                    )?
                } else {
                    return Err(WellfriendError::UnsupportedFeature(format!(
                        "{color_space} image ColorSpace requires reader-backed tint transform"
                    )));
                }
            }
            "ICCBased" => {
                if let Some(reader) = reader {
                    if let Some(converted) = cmm::icc_bytes_to_rgb_with_options(
                        &normalised,
                        dict,
                        reader,
                        color_context.options,
                    ) {
                        converted
                    } else {
                        let n = ColorSpaceConverter::icc_channel_count(dict, reader).ok_or_else(
                            || {
                                WellfriendError::UnsupportedFeature(
                                    "ICCBased image ColorSpace is missing supported channel metadata"
                                        .to_string(),
                                )
                            },
                        )?;
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
                    return Err(WellfriendError::UnsupportedFeature(
                        "ICCBased image ColorSpace requires reader-backed ICC profile".to_string(),
                    ));
                }
            }
            other => {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "unsupported image ColorSpace /{other}"
                )));
            }
        };

        let expected_size = expected_len(width, height, channels);
        ensure_decoded_len(pixels.len(), width, height, channels, expected_size)?;

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
        color_context: ImageColorContext<'_>,
    ) -> Result<RawImage> {
        let reader = color_context.reader;
        let expected_channels = Self::raw_channel_count(color_space, dict, reader)?;
        if channels == expected_channels {
            return Self::build_raw_image(
                pixels,
                width,
                height,
                8,
                color_space,
                dict,
                color_context,
            );
        }
        if channels == 4 && expected_channels == 4 {
            let mut pixels = pixels;
            ensure_decode_budget(width, height, channels)?;
            let expected_size = expected_len(width, height, channels);
            ensure_decoded_len(pixels.len(), width, height, channels, expected_size)?;
            Self::apply_decode_array(&mut pixels, channels, dict)?;
            return Ok(RawImage {
                width,
                height,
                channels: 3,
                bits_per_sample: 8,
                pixels: ColorSpaceConverter::cmyk_to_rgb(&pixels),
            });
        }
        Err(WellfriendError::UnsupportedFeature(format!(
            "DCTDecode image produced {channels} components but PDF ColorSpace /{color_space} expects {expected_channels}"
        )))
    }

    fn finish_jpx_decoded_image(
        raw: RawImage,
        declared_width: u32,
        declared_height: u32,
        label: &str,
        dict: &PdfDictionary,
    ) -> Result<RawImage> {
        ensure_jpx_dimensions_match(
            label,
            declared_width,
            declared_height,
            raw.width,
            raw.height,
        )?;
        Self::finish_jpx_decoded_samples(raw, label, dict)
    }

    fn finish_reduced_jpx_decoded_image(
        raw: RawImage,
        declared_width: u32,
        declared_height: u32,
        original_width: u32,
        original_height: u32,
        label: &str,
        dict: &PdfDictionary,
    ) -> Result<RawImage> {
        ensure_jpx_dimensions_match(
            label,
            declared_width,
            declared_height,
            original_width,
            original_height,
        )?;
        ensure_reduced_jpx_dimensions_within_original(
            label,
            original_width,
            original_height,
            raw.width,
            raw.height,
        )?;
        Self::finish_jpx_decoded_samples(raw, label, dict)
    }

    fn finish_jpx_decoded_samples(
        mut raw: RawImage,
        label: &str,
        dict: &PdfDictionary,
    ) -> Result<RawImage> {
        ensure_jpx_raw_image_invariants(label, &raw)?;
        let smask_in_data = jpx_smask_in_data_value(label, dict)?;
        if raw.channels != 4 {
            if let Some(value @ 1..=2) = smask_in_data {
                return Err(WellfriendError::MalformedPdf(format!(
                    "JPXDecode {label} declares /SMaskInData {} but decoded image has {} channels, expected an alpha channel",
                    value, raw.channels
                )));
            }
            return Ok(raw);
        }
        match smask_in_data {
            Some(0) => {
                let mut rgb = Vec::with_capacity((raw.pixels.len() / 4).saturating_mul(3));
                for px in raw.pixels.chunks(4) {
                    rgb.extend_from_slice(&px[..3]);
                }
                raw.pixels = rgb;
                raw.channels = 3;
                Ok(raw)
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
                Ok(raw)
            }
            _ => Ok(raw),
        }
    }

    fn apply_decode_array(pixels: &mut [u8], channels: u8, dict: &PdfDictionary) -> Result<()> {
        let Some(items) = dict.get("Decode").and_then(PdfObject::as_array) else {
            return Ok(());
        };
        let channels = channels.max(1) as usize;
        let expected_len = channels * 2;
        if items.len() != expected_len {
            return Err(WellfriendError::MalformedPdf(format!(
                "image /Decode has {} entries, expected {expected_len} for {channels} channels",
                items.len()
            )));
        }
        let mut values = Vec::with_capacity(items.len());
        for (idx, item) in items.iter().enumerate() {
            let Some(value) = item.as_number() else {
                return Err(WellfriendError::MalformedPdf(format!(
                    "image /Decode entry {} resolved to {}, expected Number",
                    idx + 1,
                    item.variant_name()
                )));
            };
            if !value.is_finite() {
                return Err(WellfriendError::MalformedPdf(format!(
                    "image /Decode entry {} is not finite",
                    idx + 1
                )));
            }
            values.push(value);
        }

        for (idx, sample) in pixels.iter_mut().enumerate() {
            let ch = idx % channels;
            let low = values[ch * 2];
            let high = values[ch * 2 + 1];
            let unit = f64::from(*sample) / 255.0;
            let decoded = (low + unit * (high - low)).clamp(0.0, 1.0);
            *sample = (decoded * 255.0).round() as u8;
        }
        Ok(())
    }

    fn raw_channel_count(
        color_space: &str,
        dict: &PdfDictionary,
        reader: Option<&PdfReader>,
    ) -> Result<u8> {
        match color_space {
            "DeviceGray" | "G" | "CalGray" | "Indexed" => Ok(1),
            "DeviceRGB" | "RGB" | "CalRGB" | "sRGB" | "Lab" => Ok(3),
            "DeviceCMYK" | "CMYK" => Ok(4),
            "Separation" => Ok(1),
            "DeviceN" => ColorSpaceConverter::device_n_channel_count_from_image_dict(dict, reader),
            "ICCBased" => {
                let reader = reader.ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "ICCBased image ColorSpace requires a PdfReader for channel metadata"
                            .to_string(),
                    )
                })?;
                ColorSpaceConverter::icc_channel_count(dict, reader).ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "ICCBased image ColorSpace is missing supported channel metadata"
                            .to_string(),
                    )
                })
            }
            other => Err(WellfriendError::UnsupportedFeature(format!(
                "unsupported image ColorSpace /{other}"
            ))),
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
        Self::convert_with_options(
            pixels,
            width,
            height,
            source_cs,
            dict,
            reader,
            ColorTransformOptions::default(),
        )
    }

    pub(crate) fn convert_with_options(
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        source_cs: &str,
        dict: &PdfDictionary,
        reader: &PdfReader,
        color_options: ColorTransformOptions,
    ) -> Result<(Vec<u8>, u8)> {
        match source_cs {
            "DeviceGray" | "G" => Ok((pixels, 1)),
            "CalGray" => {
                let params = cmm::try_cal_gray_params_from_image_dict(dict, Some(reader))
                    .map_err(WellfriendError::MalformedPdf)?;
                Ok((cmm::cal_gray_bytes_to_rgb(&pixels, params), 3))
            }
            "DeviceRGB" | "RGB" | "sRGB" => Ok((pixels, 3)),
            "CalRGB" => {
                let params = cmm::try_cal_rgb_params_from_image_dict(dict, Some(reader))
                    .map_err(WellfriendError::MalformedPdf)?;
                Ok((cmm::cal_rgb_bytes_to_rgb(&pixels, params), 3))
            }
            "DeviceCMYK" | "CMYK" => {
                ensure_cmyk_input_len("DeviceCMYK image ColorSpace", width, height, &pixels)?;
                Ok((Self::cmyk_to_rgb(&pixels), 3))
            }
            "ICCBased" => {
                if let Some(converted) =
                    cmm::icc_bytes_to_rgb_with_options(&pixels, dict, reader, color_options)
                {
                    Ok(converted)
                } else {
                    let n = Self::icc_channel_count(dict, reader).ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(
                            "ICCBased image ColorSpace is missing supported channel metadata"
                                .to_string(),
                        )
                    })?;
                    match n {
                        1 => Ok((pixels, 1)),
                        3 => Ok((pixels, 3)),
                        4 => {
                            ensure_cmyk_input_len(
                                "ICCBased CMYK image ColorSpace",
                                width,
                                height,
                                &pixels,
                            )?;
                            Ok((Self::cmyk_to_rgb(&pixels), 3))
                        }
                        _ => Err(WellfriendError::UnsupportedFeature(format!(
                            "ICCBased with {n} components not supported"
                        ))),
                    }
                }
            }
            "Indexed" => Self::decode_indexed(&pixels, 8, dict, reader, width, height),
            "Separation" | "DeviceN" => Self::tint_space_to_rgba(
                &pixels,
                dict,
                reader,
                Self::tint_space_channel_count(dict, source_cs, reader)?,
                color_options,
            ),
            "Lab" => {
                let params = cmm::try_lab_params_from_image_dict(dict, Some(reader))
                    .map_err(WellfriendError::MalformedPdf)?;
                Ok((cmm::lab_bytes_to_rgb(&pixels, params), 3))
            }
            other => Err(WellfriendError::UnsupportedFeature(format!(
                "unsupported image ColorSpace /{other}"
            ))),
        }
    }

    fn tint_space_to_rgba(
        pixels: &[u8],
        dict: &PdfDictionary,
        reader: &PdfReader,
        channels: u8,
        color_options: ColorTransformOptions,
    ) -> Result<(Vec<u8>, u8)> {
        let Some(space_obj) = dict.get("ColorSpace").or_else(|| dict.get("CS")) else {
            return Err(WellfriendError::UnsupportedFeature(
                "image tint ColorSpace requires a resolved ColorSpace array".to_string(),
            ));
        };
        let channels = channels.max(1) as usize;
        if !pixels.len().is_multiple_of(channels) {
            return Err(WellfriendError::MalformedPdf(format!(
                "image tint ColorSpace decoded {} bytes for {channels} components",
                pixels.len()
            )));
        }
        let family =
            Self::tint_space_family_name(space_obj).unwrap_or_else(|| "tint-space".to_string());
        let mut output = Vec::with_capacity((pixels.len() / channels).saturating_mul(4));
        for chunk in pixels.chunks_exact(channels) {
            let mut components = Vec::with_capacity(channels);
            for sample in chunk.iter().take(channels) {
                components.push(f64::from(*sample) / 255.0);
            }
            match crate::render::colorspace::resolve_named_color_with_options(
                space_obj,
                &components,
                1.0,
                reader,
                color_options,
            ) {
                crate::render::colorspace::NamedColor::Color(color) => {
                    output.extend_from_slice(&color.to_pixel_color());
                }
                crate::render::colorspace::NamedColor::NoPaint => {
                    output.extend_from_slice(&[0, 0, 0, 0]);
                }
                crate::render::colorspace::NamedColor::Invalid(reason) => {
                    return Err(WellfriendError::UnsupportedFeature(format!(
                        "invalid image ColorSpace /{family} tint transform: {reason}"
                    )));
                }
                crate::render::colorspace::NamedColor::Unhandled => {
                    return Err(WellfriendError::UnsupportedFeature(format!(
                        "unsupported image ColorSpace /{family} tint transform"
                    )));
                }
            }
        }
        Ok((output, 4))
    }

    fn tint_space_channel_count(
        dict: &PdfDictionary,
        source_cs: &str,
        reader: &PdfReader,
    ) -> Result<u8> {
        match source_cs {
            "DeviceN" => Self::device_n_channel_count_from_image_dict(dict, Some(reader)),
            _ => Ok(1),
        }
    }

    fn device_n_channel_count_from_image_dict(
        dict: &PdfDictionary,
        reader: Option<&PdfReader>,
    ) -> Result<u8> {
        let space = dict
            .get("ColorSpace")
            .or_else(|| dict.get("CS"))
            .ok_or_else(|| {
                WellfriendError::UnsupportedFeature(
                    "DeviceN image ColorSpace requires a resolved ColorSpace array".to_string(),
                )
            })?;
        Self::device_n_channel_count_from_space(space, reader)
    }

    fn device_n_channel_count_from_space(
        space: &PdfObject,
        reader: Option<&PdfReader>,
    ) -> Result<u8> {
        let resolved = match space {
            PdfObject::Reference { .. } => {
                let reader = reader.ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "DeviceN image ColorSpace reference requires a PdfReader".to_string(),
                    )
                })?;
                reader.resolve(space.clone()).map_err(|err| {
                    WellfriendError::MalformedPdf(format!(
                        "DeviceN image ColorSpace reference failed: {err}"
                    ))
                })?
            }
            other => other.clone(),
        };
        let arr = resolved.as_array().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "DeviceN image ColorSpace resolved to a non-array object".to_string(),
            )
        })?;
        if arr.first().and_then(PdfObject::as_name) != Some("DeviceN") {
            return Err(WellfriendError::MalformedPdf(
                "DeviceN image ColorSpace array has no /DeviceN family name".to_string(),
            ));
        }
        let names = arr.get(1).and_then(PdfObject::as_array).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "DeviceN image ColorSpace has no component-name array".to_string(),
            )
        })?;
        if names.is_empty() {
            return Err(WellfriendError::MalformedPdf(
                "DeviceN image ColorSpace has no components".to_string(),
            ));
        }
        if names.len() > crate::render::colorspace::MAX_DEVICEN_COMPONENTS {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "DeviceN image ColorSpace has {} components, max supported is {}",
                names.len(),
                crate::render::colorspace::MAX_DEVICEN_COMPONENTS
            )));
        }
        if names.iter().any(|name| name.as_name().is_none()) {
            return Err(WellfriendError::MalformedPdf(
                "DeviceN image ColorSpace component-name array contains a non-name entry"
                    .to_string(),
            ));
        }
        Ok(names.len() as u8)
    }

    fn tint_space_family_name(space_obj: &PdfObject) -> Option<String> {
        match space_obj {
            PdfObject::Array(items) => items
                .first()
                .and_then(PdfObject::as_name)
                .map(str::to_string),
            PdfObject::Name(name) => Some(name.clone()),
            _ => None,
        }
    }

    /// Convert interleaved CMYK pixels to RGB.
    pub fn cmyk_to_rgb(pixels: &[u8]) -> Vec<u8> {
        cmm::device_cmyk_bytes_to_rgb(pixels)
    }

    fn decode_indexed(
        pixels: &[u8],
        bits_per_component: u8,
        dict: &PdfDictionary,
        reader: &PdfReader,
        _width: u32,
        _height: u32,
    ) -> Result<(Vec<u8>, u8)> {
        let cs_array = dict
            .get("ColorSpace")
            .and_then(PdfObject::as_array)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "Indexed image ColorSpace is not an array".to_string(),
                )
            })?;

        if cs_array.len() != 4 {
            return Err(WellfriendError::MalformedPdf(format!(
                "Indexed image ColorSpace has {} entries, expected 4",
                cs_array.len()
            )));
        }

        let base_space = cs_array[1].clone();
        let base_cs = Self::color_space_family_name(&base_space, reader)?;
        let hival = cs_array[2].as_integer().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "Indexed image ColorSpace hival is not an integer".to_string(),
            )
        })?;
        if hival < 0 {
            return Err(WellfriendError::MalformedPdf(
                "Indexed image ColorSpace hival is negative".to_string(),
            ));
        }
        let hival = hival as usize;

        let lookup = match cs_array.get(3) {
            Some(PdfObject::String(bytes)) => bytes.clone(),
            Some(PdfObject::Reference { number, generation }) => {
                match reader.get_object(*number, *generation) {
                    Ok(PdfObject::String(bytes)) => bytes,
                    Ok(PdfObject::Stream { raw, .. }) => raw,
                    Ok(other) => {
                        return Err(WellfriendError::MalformedPdf(format!(
                            "Indexed image ColorSpace lookup resolved to {}, expected String or Stream",
                            other.variant_name()
                        )));
                    }
                    Err(err) => {
                        return Err(WellfriendError::MalformedPdf(format!(
                            "Indexed image ColorSpace lookup reference failed: {err}"
                        )));
                    }
                }
            }
            Some(other) => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "Indexed image ColorSpace lookup is {}, expected String, Stream, or Reference",
                    other.variant_name()
                )));
            }
            None => {
                return Err(WellfriendError::MalformedPdf(
                    "Indexed image ColorSpace is missing lookup data".to_string(),
                ));
            }
        };

        let base_channels = Self::indexed_base_channel_count(&base_cs, &base_space, reader)?;
        let expected_lookup_len = (hival + 1) * base_channels;
        if lookup.len() != expected_lookup_len {
            return Err(WellfriendError::MalformedPdf(format!(
                "Indexed image ColorSpace lookup table has {} bytes, expected {}",
                lookup.len(),
                expected_lookup_len
            )));
        }

        let raw_palette = lookup[..expected_lookup_len].to_vec();

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
        ensure_indexed_palette_len(palette.len(), hival + 1, palette_channels)?;
        let mut output = Vec::with_capacity(pixels.len() * palette_channels);
        for &sample in pixels {
            let idx = indexed_palette_index(sample, bits_per_component, hival);
            let start = idx * palette_channels;
            let end = start + palette_channels;
            output.extend_from_slice(&palette[start..end]);
        }
        Ok((output, palette_channels as u8))
    }

    fn color_space_family_name(space: &PdfObject, reader: &PdfReader) -> Result<String> {
        let resolved = match space {
            PdfObject::Reference { .. } => reader.resolve(space.clone()).map_err(|err| {
                WellfriendError::MalformedPdf(format!(
                    "Indexed image base ColorSpace reference failed: {err}"
                ))
            })?,
            other => other.clone(),
        };
        match resolved {
            PdfObject::Name(name) => Ok(name),
            PdfObject::Array(items) => items
                .first()
                .and_then(PdfObject::as_name)
                .map(str::to_string)
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "Indexed image base ColorSpace array has no family name".to_string(),
                    )
                }),
            other => Err(WellfriendError::MalformedPdf(format!(
                "Indexed image base ColorSpace resolved to {}, expected Name or Array",
                other.variant_name()
            ))),
        }
    }

    fn indexed_base_channel_count(
        base_cs: &str,
        base_space: &PdfObject,
        reader: &PdfReader,
    ) -> Result<usize> {
        match base_cs {
            "DeviceGray" | "G" | "CalGray" | "Separation" => Ok(1),
            "DeviceCMYK" | "CMYK" => Ok(4),
            "DeviceN" => {
                Self::device_n_channel_count_from_space(base_space, Some(reader)).map(usize::from)
            }
            "ICCBased" => {
                Self::icc_component_count_from_space(base_space, reader).ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(
                        "Indexed image ICCBased base ColorSpace is unsupported".to_string(),
                    )
                })
            }
            "DeviceRGB" | "RGB" | "CalRGB" | "sRGB" | "Lab" => Ok(3),
            other => Err(WellfriendError::UnsupportedFeature(format!(
                "Indexed image base ColorSpace /{other} is unsupported"
            ))),
        }
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
            .and_then(|n| (1..=4).contains(&n).then_some(n as usize))
    }

    fn icc_channel_count(dict: &PdfDictionary, reader: &PdfReader) -> Option<u8> {
        cmm::icc_channel_count(dict, reader)
    }
}

fn indexed_palette_index(sample: u8, bits_per_component: u8, hival: usize) -> usize {
    if bits_per_component < 8 {
        let max_sample = ((1usize << bits_per_component.min(7)) - 1).max(1);
        ((usize::from(sample) * max_sample + 127) / 255)
            .min(max_sample)
            .min(hival)
    } else {
        usize::from(sample).min(hival)
    }
}

fn ensure_indexed_palette_len(
    actual_len: usize,
    entries: usize,
    palette_channels: usize,
) -> Result<()> {
    let expected_len = entries.checked_mul(palette_channels).ok_or_else(|| {
        WellfriendError::MalformedPdf(format!(
            "Indexed image ColorSpace palette length overflows for {entries} entries x{palette_channels} channels"
        ))
    })?;
    if actual_len != expected_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "Indexed image ColorSpace converted palette has {actual_len} bytes, expected {expected_len}"
        )));
    }
    Ok(())
}

fn decode_array_applies_before_color_conversion(color_space: &str) -> bool {
    !matches!(color_space, "Indexed" | "Lab")
}

fn image_mask_flag(dict: &PdfDictionary) -> Result<bool> {
    match dict.get("ImageMask").or_else(|| dict.get("IM")) {
        Some(PdfObject::Boolean(value)) => Ok(*value),
        Some(other) => Err(WellfriendError::MalformedPdf(format!(
            "image /ImageMask must be a boolean, got {}",
            other.variant_name()
        ))),
        _ => Ok(false),
    }
}

fn apply_decode_array_for_component_selection(
    pixels: &mut [u8],
    source_channels: u8,
    selected_channels: u8,
    component_selection: &RawImageComponentSelection,
    dict: &PdfDictionary,
) -> Result<()> {
    let Some(items) = dict.get("Decode").and_then(PdfObject::as_array) else {
        return Ok(());
    };
    let source_channels = source_channels.max(1) as usize;
    let selected_channels = selected_channels.max(1) as usize;
    let expected_len = source_channels * 2;
    if items.len() != expected_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "image /Decode has {} entries, expected {expected_len} for {source_channels} channels",
            items.len()
        )));
    }
    let mut values = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Some(value) = item.as_number() else {
            return Err(WellfriendError::MalformedPdf(format!(
                "image /Decode entry {} resolved to {}, expected Number",
                idx + 1,
                item.variant_name()
            )));
        };
        if !value.is_finite() {
            return Err(WellfriendError::MalformedPdf(format!(
                "image /Decode entry {} is not finite",
                idx + 1
            )));
        }
        values.push(value);
    }

    let Some(components) = component_selection.components() else {
        return Ok(());
    };
    for (idx, sample) in pixels.iter_mut().enumerate() {
        let selected_channel = idx % selected_channels;
        let source_channel = usize::from(components[selected_channel]);
        let low = values[source_channel * 2];
        let high = values[source_channel * 2 + 1];
        let unit = f64::from(*sample) / 255.0;
        let decoded = (low + unit * (high - low)).clamp(0.0, 1.0);
        *sample = (decoded * 255.0).round() as u8;
    }
    Ok(())
}

fn normalise_16_bit_samples(raw: &[u8], width: u32, height: u32, channels: u8) -> Result<Vec<u8>> {
    let samples = (width as usize)
        .checked_mul(height as usize)
        .and_then(|value| value.checked_mul(channels.max(1) as usize))
        .ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "16-bit image dimensions {width}x{height} x{} channels overflow",
                channels.max(1)
            ))
        })?;
    let expected_len = samples.checked_mul(2).ok_or_else(|| {
        WellfriendError::MalformedPdf(format!(
            "16-bit image byte count for {width}x{height} x{} channels overflows",
            channels.max(1)
        ))
    })?;
    if raw.len() != expected_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "16-bit image data has {} bytes, expected {} for {}x{} x{} channels",
            raw.len(),
            expected_len,
            width,
            height,
            channels.max(1)
        )));
    }
    Ok(raw.chunks_exact(2).map(|chunk| chunk[0]).collect())
}

fn unpack_subbyte_rows(
    raw: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    bpc: u8,
) -> Result<Vec<u8>> {
    let channels = channels.max(1) as usize;
    let samples_per_row = width as usize * channels;
    let total = samples_per_row.saturating_mul(height as usize);
    let bits_per_row = samples_per_row.saturating_mul(bpc as usize);
    let bytes_per_row = bits_per_row.div_ceil(8);
    let required_len = bytes_per_row.saturating_mul(height as usize);
    if raw.len() != required_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "sub-byte image data has {} bytes, expected {} for {}x{} x{} channels at {} bpc",
            raw.len(),
            required_len,
            width,
            height,
            channels,
            bpc
        )));
    }
    let max_value = (1u16 << bpc) - 1;
    let scale = 255u16 / max_value;
    let mask = max_value as u8;

    let mut out = Vec::with_capacity(total);
    for row in 0..height as usize {
        let row_start = row.saturating_mul(bytes_per_row);
        let row_end = row_start.saturating_add(bytes_per_row);
        let row_bytes = &raw[row_start..row_end];
        for sample in 0..samples_per_row {
            let bit_offset = sample.saturating_mul(bpc as usize);
            let byte = row_bytes[bit_offset / 8];
            let shift = 8usize - bpc as usize - (bit_offset % 8);
            let packed = (byte >> shift) & mask;
            out.push((u16::from(packed) * scale) as u8);
        }
    }
    Ok(out)
}

fn expected_len(width: u32, height: u32, channels: u8) -> usize {
    width as usize * height as usize * channels as usize
}

fn validate_raw_component_selection(components: &[u8], source_channels: u8) -> Result<u8> {
    if components.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "raw image component selection requires at least one component".to_string(),
        ));
    }
    let source_channels = source_channels.max(1);
    let mut previous = None;
    for &component in components {
        if component >= source_channels {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "raw image component selection index {component} exceeds source channel count {source_channels}"
            )));
        }
        if previous.is_some_and(|last| component <= last) {
            return Err(WellfriendError::UnsupportedFeature(
                "raw image component selection must be strictly ascending and unique".to_string(),
            ));
        }
        previous = Some(component);
    }
    Ok(components.len() as u8)
}

#[allow(clippy::too_many_arguments)]
fn crop_raw_window_with_component_selection(
    raw: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    bpc: u8,
    window: RawImageDecodeWindow,
    component_selection: &RawImageComponentSelection,
    limits: &DecodeLimits,
) -> Result<(Vec<u8>, u8)> {
    let selected_channels = component_selection.selected_channel_count(channels)?;
    let Some(components) = component_selection.components() else {
        return Ok((
            crop_raw_window(raw, width, height, channels, bpc, window, limits)?,
            selected_channels,
        ));
    };

    let cropped = match bpc {
        8 => crop_raw_byte_aligned_component_window(
            raw, width, height, channels, 1, window, components, selected_channels, limits,
        )?,
        16 => crop_raw_byte_aligned_component_window(
            raw, width, height, channels, 2, window, components, selected_channels, limits,
        )?,
        1 | 2 | 4 => crop_raw_subbyte_component_window(
            raw, width, height, channels, bpc, window, components, selected_channels, limits,
        )?,
        other => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "raw image component selection supports only 1, 2, 4, 8, or 16-bit samples, got {other}"
            )))
        }
    };
    Ok((cropped, selected_channels))
}

fn crop_raw_window(
    raw: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    bpc: u8,
    window: RawImageDecodeWindow,
    limits: &DecodeLimits,
) -> Result<Vec<u8>> {
    match bpc {
        8 => crop_raw_byte_aligned_window(raw, width, height, channels, 1, window, limits),
        16 => crop_raw_byte_aligned_window(raw, width, height, channels, 2, window, limits),
        1 | 2 | 4 => crop_raw_subbyte_window(raw, width, height, channels, bpc, window, limits),
        other => Err(WellfriendError::UnsupportedFeature(format!(
            "raw image window decoding supports only 1, 2, 4, 8, or 16-bit samples, got {other}"
        ))),
    }
}

#[cfg(test)]
fn crop_raw_8bit_window(
    raw: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    window: RawImageDecodeWindow,
    limits: &DecodeLimits,
) -> Result<Vec<u8>> {
    crop_raw_byte_aligned_window(raw, width, height, channels, 1, window, limits)
}

fn crop_raw_byte_aligned_window(
    raw: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    bytes_per_sample: usize,
    window: RawImageDecodeWindow,
    limits: &DecodeLimits,
) -> Result<Vec<u8>> {
    if window.width == 0 || window.height == 0 {
        return Ok(Vec::new());
    }
    let x1 = window.x.checked_add(window.width).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image window x range overflows".to_string())
    })?;
    let y1 = window.y.checked_add(window.height).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image window y range overflows".to_string())
    })?;
    if x1 > width || y1 > height {
        return Err(WellfriendError::MalformedPdf(format!(
            "raw image window {},{} {}x{} exceeds image bounds {}x{}",
            window.x, window.y, window.width, window.height, width, height
        )));
    }
    let normalized_output_len = raw_window_normalized_output_len(window, channels)?;
    let packed_output_len = normalized_output_len
        .checked_mul(bytes_per_sample as u64)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window byte length overflows".to_string())
        })?;
    enforce_raw_window_limit(normalized_output_len.max(packed_output_len), limits)?;

    ensure_decode_budget(window.width, window.height, channels)?;

    let channels = usize::from(channels);
    let bytes_per_pixel = channels.checked_mul(bytes_per_sample).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image pixel byte length overflows".to_string())
    })?;
    let full_row_bytes = (width as usize)
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image row byte length overflows".to_string())
        })?;
    let window_row_bytes = (window.width as usize)
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window row byte length overflows".to_string())
        })?;
    let expected_full_len = full_row_bytes.checked_mul(height as usize).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image byte length overflows".to_string())
    })?;
    if raw.len() != expected_full_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "raw image stream has {} bytes, expected {expected_full_len} for {width}x{height}x{channels} at {} bpc",
            raw.len(),
            bytes_per_sample * 8
        )));
    }

    let mut cropped = Vec::with_capacity(packed_output_len as usize);
    let start_x = (window.x as usize)
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window x byte offset overflows".to_string())
        })?;
    for y in window.y..y1 {
        let row_start = (y as usize)
            .checked_mul(full_row_bytes)
            .and_then(|offset| offset.checked_add(start_x))
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "raw image window source byte offset overflows".to_string(),
                )
            })?;
        let row_end = row_start.checked_add(window_row_bytes).ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window source row overflows".to_string())
        })?;
        cropped.extend_from_slice(raw.get(row_start..row_end).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "raw image window source row exceeds stream data".to_string(),
            )
        })?);
    }
    Ok(cropped)
}

#[allow(clippy::too_many_arguments)]
fn crop_raw_byte_aligned_component_window(
    raw: &[u8],
    width: u32,
    height: u32,
    source_channels: u8,
    bytes_per_sample: usize,
    window: RawImageDecodeWindow,
    components: &[u8],
    selected_channels: u8,
    limits: &DecodeLimits,
) -> Result<Vec<u8>> {
    if window.width == 0 || window.height == 0 {
        return Ok(Vec::new());
    }
    let x1 = window.x.checked_add(window.width).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image window x range overflows".to_string())
    })?;
    let y1 = window.y.checked_add(window.height).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image window y range overflows".to_string())
    })?;
    if x1 > width || y1 > height {
        return Err(WellfriendError::MalformedPdf(format!(
            "raw image window {},{} {}x{} exceeds image bounds {}x{}",
            window.x, window.y, window.width, window.height, width, height
        )));
    }

    let normalized_output_len = raw_window_normalized_output_len(window, selected_channels)?;
    let packed_output_len = normalized_output_len
        .checked_mul(bytes_per_sample as u64)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window byte length overflows".to_string())
        })?;
    enforce_raw_window_limit(normalized_output_len.max(packed_output_len), limits)?;
    ensure_decode_budget(window.width, window.height, selected_channels)?;

    let source_channels = usize::from(source_channels.max(1));
    let bytes_per_pixel = source_channels
        .checked_mul(bytes_per_sample)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image pixel byte length overflows".to_string())
        })?;
    let full_row_bytes = (width as usize)
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image row byte length overflows".to_string())
        })?;
    let expected_full_len = full_row_bytes.checked_mul(height as usize).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image byte length overflows".to_string())
    })?;
    if raw.len() != expected_full_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "raw image stream has {} bytes, expected {expected_full_len} for {width}x{height}x{source_channels} at {} bpc",
            raw.len(),
            bytes_per_sample * 8
        )));
    }

    let mut cropped = Vec::with_capacity(packed_output_len as usize);
    for y in window.y..y1 {
        let row_start = (y as usize).checked_mul(full_row_bytes).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "raw image window source row offset overflows".to_string(),
            )
        })?;
        for x in window.x..x1 {
            let pixel_start = row_start
                .checked_add((x as usize).checked_mul(bytes_per_pixel).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "raw image window x byte offset overflows".to_string(),
                    )
                })?)
                .ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "raw image window source byte offset overflows".to_string(),
                    )
                })?;
            for &component in components {
                let component_start = pixel_start
                    .checked_add(
                        usize::from(component)
                            .checked_mul(bytes_per_sample)
                            .ok_or_else(|| {
                                WellfriendError::MalformedPdf(
                                    "raw image component byte offset overflows".to_string(),
                                )
                            })?,
                    )
                    .ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "raw image component byte offset overflows".to_string(),
                        )
                    })?;
                let component_end =
                    component_start
                        .checked_add(bytes_per_sample)
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "raw image component byte range overflows".to_string(),
                            )
                        })?;
                cropped.extend_from_slice(raw.get(component_start..component_end).ok_or_else(
                    || {
                        WellfriendError::MalformedPdf(
                            "raw image component source exceeds stream data".to_string(),
                        )
                    },
                )?);
            }
        }
    }
    Ok(cropped)
}

fn crop_raw_subbyte_window(
    raw: &[u8],
    width: u32,
    height: u32,
    channels: u8,
    bpc: u8,
    window: RawImageDecodeWindow,
    limits: &DecodeLimits,
) -> Result<Vec<u8>> {
    if window.width == 0 || window.height == 0 {
        return Ok(Vec::new());
    }
    let x1 = window.x.checked_add(window.width).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image window x range overflows".to_string())
    })?;
    let y1 = window.y.checked_add(window.height).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image window y range overflows".to_string())
    })?;
    if x1 > width || y1 > height {
        return Err(WellfriendError::MalformedPdf(format!(
            "raw image window {},{} {}x{} exceeds image bounds {}x{}",
            window.x, window.y, window.width, window.height, width, height
        )));
    }

    let normalized_output_len = raw_window_normalized_output_len(window, channels)?;
    let channels = usize::from(channels.max(1));
    let window_samples_per_row =
        (window.width as usize)
            .checked_mul(channels)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf("raw image window sample count overflows".to_string())
            })?;
    let output_row_bytes = window_samples_per_row
        .checked_mul(bpc as usize)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window bit length overflows".to_string())
        })?
        .div_ceil(8);
    let packed_output_len = output_row_bytes
        .checked_mul(window.height as usize)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window byte length overflows".to_string())
        })?;
    enforce_raw_window_limit(normalized_output_len.max(packed_output_len as u64), limits)?;
    ensure_decode_budget(window.width, window.height, channels as u8)?;

    let full_samples_per_row = (width as usize).checked_mul(channels).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image row sample count overflows".to_string())
    })?;
    let full_row_bytes = full_samples_per_row
        .checked_mul(bpc as usize)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image row bit length overflows".to_string())
        })?
        .div_ceil(8);
    let expected_full_len = full_row_bytes.checked_mul(height as usize).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image byte length overflows".to_string())
    })?;
    if raw.len() != expected_full_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "raw image stream has {} bytes, expected {expected_full_len} for {width}x{height}x{channels} at {bpc} bpc",
            raw.len()
        )));
    }

    let mut cropped = vec![0u8; packed_output_len];
    let mask = ((1u16 << bpc) - 1) as u8;
    let start_sample = (window.x as usize).checked_mul(channels).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image window x sample offset overflows".to_string())
    })?;
    for (dst_row, src_y) in (window.y..y1).enumerate() {
        let src_row_start = (src_y as usize)
            .checked_mul(full_row_bytes)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "raw image window source row offset overflows".to_string(),
                )
            })?;
        let dst_row_start = dst_row.checked_mul(output_row_bytes).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "raw image window destination row offset overflows".to_string(),
            )
        })?;
        for sample in 0..window_samples_per_row {
            let src_sample = start_sample.checked_add(sample).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "raw image window source sample offset overflows".to_string(),
                )
            })?;
            let src_bit = src_sample.checked_mul(bpc as usize).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "raw image window source bit offset overflows".to_string(),
                )
            })?;
            let src_byte = src_row_start.checked_add(src_bit / 8).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "raw image window source byte offset overflows".to_string(),
                )
            })?;
            let src_shift = 8usize - bpc as usize - (src_bit % 8);
            let packed = (raw[src_byte] >> src_shift) & mask;

            let dst_bit = sample.checked_mul(bpc as usize).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "raw image window destination bit offset overflows".to_string(),
                )
            })?;
            let dst_byte = dst_row_start.checked_add(dst_bit / 8).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "raw image window destination byte offset overflows".to_string(),
                )
            })?;
            let dst_shift = 8usize - bpc as usize - (dst_bit % 8);
            cropped[dst_byte] |= packed << dst_shift;
        }
    }
    Ok(cropped)
}

#[allow(clippy::too_many_arguments)]
fn crop_raw_subbyte_component_window(
    raw: &[u8],
    width: u32,
    height: u32,
    source_channels: u8,
    bpc: u8,
    window: RawImageDecodeWindow,
    components: &[u8],
    selected_channels: u8,
    limits: &DecodeLimits,
) -> Result<Vec<u8>> {
    if window.width == 0 || window.height == 0 {
        return Ok(Vec::new());
    }
    let x1 = window.x.checked_add(window.width).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image window x range overflows".to_string())
    })?;
    let y1 = window.y.checked_add(window.height).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image window y range overflows".to_string())
    })?;
    if x1 > width || y1 > height {
        return Err(WellfriendError::MalformedPdf(format!(
            "raw image window {},{} {}x{} exceeds image bounds {}x{}",
            window.x, window.y, window.width, window.height, width, height
        )));
    }

    let normalized_output_len = raw_window_normalized_output_len(window, selected_channels)?;
    let selected_channels_usize = usize::from(selected_channels.max(1));
    let window_samples_per_row = (window.width as usize)
        .checked_mul(selected_channels_usize)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window sample count overflows".to_string())
        })?;
    let output_row_bytes = window_samples_per_row
        .checked_mul(bpc as usize)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window bit length overflows".to_string())
        })?
        .div_ceil(8);
    let packed_output_len = output_row_bytes
        .checked_mul(window.height as usize)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window byte length overflows".to_string())
        })?;
    enforce_raw_window_limit(normalized_output_len.max(packed_output_len as u64), limits)?;
    ensure_decode_budget(window.width, window.height, selected_channels)?;

    let source_channels = usize::from(source_channels.max(1));
    let full_samples_per_row = (width as usize)
        .checked_mul(source_channels)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image row sample count overflows".to_string())
        })?;
    let full_row_bytes = full_samples_per_row
        .checked_mul(bpc as usize)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image row bit length overflows".to_string())
        })?
        .div_ceil(8);
    let expected_full_len = full_row_bytes.checked_mul(height as usize).ok_or_else(|| {
        WellfriendError::MalformedPdf("raw image byte length overflows".to_string())
    })?;
    if raw.len() != expected_full_len {
        return Err(WellfriendError::MalformedPdf(format!(
            "raw image stream has {} bytes, expected {expected_full_len} for {width}x{height}x{source_channels} at {bpc} bpc",
            raw.len()
        )));
    }

    let mut cropped = vec![0u8; packed_output_len];
    let mask = ((1u16 << bpc) - 1) as u8;
    for (dst_row, src_y) in (window.y..y1).enumerate() {
        let src_row_start = (src_y as usize)
            .checked_mul(full_row_bytes)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "raw image window source row offset overflows".to_string(),
                )
            })?;
        let dst_row_start = dst_row.checked_mul(output_row_bytes).ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "raw image window destination row offset overflows".to_string(),
            )
        })?;
        let mut dst_sample = 0usize;
        for x in window.x..x1 {
            let src_pixel_sample = (x as usize).checked_mul(source_channels).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "raw image window source sample offset overflows".to_string(),
                )
            })?;
            for &component in components {
                let src_sample = src_pixel_sample
                    .checked_add(usize::from(component))
                    .ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "raw image component source sample offset overflows".to_string(),
                        )
                    })?;
                let src_bit = src_sample.checked_mul(bpc as usize).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "raw image component source bit offset overflows".to_string(),
                    )
                })?;
                let src_byte = src_row_start.checked_add(src_bit / 8).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "raw image component source byte offset overflows".to_string(),
                    )
                })?;
                let src_shift = 8usize - bpc as usize - (src_bit % 8);
                let packed = (raw[src_byte] >> src_shift) & mask;

                let dst_bit = dst_sample.checked_mul(bpc as usize).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "raw image component destination bit offset overflows".to_string(),
                    )
                })?;
                let dst_byte = dst_row_start.checked_add(dst_bit / 8).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "raw image component destination byte offset overflows".to_string(),
                    )
                })?;
                let dst_shift = 8usize - bpc as usize - (dst_bit % 8);
                cropped[dst_byte] |= packed << dst_shift;
                dst_sample = dst_sample.checked_add(1).ok_or_else(|| {
                    WellfriendError::MalformedPdf(
                        "raw image component destination sample offset overflows".to_string(),
                    )
                })?;
            }
        }
    }
    Ok(cropped)
}

fn raw_window_normalized_output_len(window: RawImageDecodeWindow, channels: u8) -> Result<u64> {
    u64::from(window.width)
        .saturating_mul(u64::from(window.height))
        .checked_mul(u64::from(channels.max(1)))
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("raw image window byte length overflows".to_string())
        })
}

fn enforce_raw_window_limit(required_bytes: u64, limits: &DecodeLimits) -> Result<()> {
    if required_bytes > limits.max_image_decoded_bytes {
        return Err(WellfriendError::ResourceLimit(format!(
            "raw image window decode requires {required_bytes} bytes, exceeding max_image_decoded_bytes {}",
            limits.max_image_decoded_bytes
        )));
    }
    Ok(())
}

fn ensure_dct_dimensions_match(
    label: &str,
    declared_width: u32,
    declared_height: u32,
    header_width: u32,
    header_height: u32,
) -> Result<()> {
    if declared_width != header_width || declared_height != header_height {
        return Err(WellfriendError::MalformedPdf(format!(
            "DCTDecode {label} dictionary dimensions {declared_width}x{declared_height} differ from JPEG header {header_width}x{header_height}"
        )));
    }
    Ok(())
}

fn ensure_jpx_dimensions_match(
    label: &str,
    declared_width: u32,
    declared_height: u32,
    decoded_width: u32,
    decoded_height: u32,
) -> Result<()> {
    if declared_width != decoded_width || declared_height != decoded_height {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {label} dictionary dimensions {declared_width}x{declared_height} differ from decoded JPX dimensions {decoded_width}x{decoded_height}"
        )));
    }
    Ok(())
}

fn ensure_reduced_jpx_dimensions_within_original(
    label: &str,
    original_width: u32,
    original_height: u32,
    decoded_width: u32,
    decoded_height: u32,
) -> Result<()> {
    if decoded_width == 0 || decoded_height == 0 {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {label} reduced decode produced zero dimensions {decoded_width}x{decoded_height}"
        )));
    }
    if decoded_width > original_width || decoded_height > original_height {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {label} reduced decode dimensions {decoded_width}x{decoded_height} exceed codestream dimensions {original_width}x{original_height}"
        )));
    }
    Ok(())
}

fn jpx_smask_in_data_value(label: &str, dict: &PdfDictionary) -> Result<Option<i64>> {
    let Some(value) = dict.get("SMaskInData") else {
        return Ok(None);
    };
    let Some(value) = value.as_integer() else {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {label} /SMaskInData must be an integer, got {}",
            value.variant_name()
        )));
    };
    if !matches!(value, 0..=2) {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {label} /SMaskInData must be 0, 1, or 2, got {value}"
        )));
    }
    Ok(Some(value))
}

fn ensure_jpx_raw_image_invariants(label: &str, raw: &RawImage) -> Result<()> {
    if raw.bits_per_sample != 8 {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {label} decoded {} bits per sample, expected 8",
            raw.bits_per_sample
        )));
    }
    if raw.channels == 0 {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {label} decoded zero image channels"
        )));
    }
    ensure_decode_budget(raw.width, raw.height, raw.channels)?;
    let expected = expected_len(raw.width, raw.height, raw.channels);
    if raw.pixels.len() != expected {
        return Err(WellfriendError::MalformedPdf(format!(
            "JPXDecode {label} decoded {} bytes, expected {expected} for {}x{} x{} channels",
            raw.pixels.len(),
            raw.width,
            raw.height,
            raw.channels
        )));
    }
    Ok(())
}

fn ensure_cmyk_input_len(label: &str, width: u32, height: u32, pixels: &[u8]) -> Result<()> {
    ensure_decode_budget(width, height, 4)?;
    let expected = expected_len(width, height, 4);
    if pixels.len() != expected {
        return Err(WellfriendError::MalformedPdf(format!(
            "{label} decoded {} bytes, expected {expected} for {width}x{height} x4 channels",
            pixels.len()
        )));
    }
    Ok(())
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

fn ensure_decoded_len(
    actual_size: usize,
    width: u32,
    height: u32,
    channels: u8,
    expected_size: usize,
) -> Result<()> {
    if actual_size == expected_size {
        return Ok(());
    }
    Err(WellfriendError::MalformedPdf(format!(
        "image {width}x{height} x{channels} channels decoded {actual_size} bytes, expected {expected_size}"
    )))
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
        "JPX" => "JPXDecode",
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

    fn reader_with_icc_profile(profile_fields: &str) -> PdfReader {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut off = [0usize; 5];
        off[1] = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        off[2] = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        off[3] = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>\nendobj\n",
        );
        off[4] = pdf.len();
        pdf.extend_from_slice(
            format!("4 0 obj\n<< {profile_fields} /Length 4 >>\nstream\nabcd\nendstream\nendobj\n")
                .as_bytes(),
        );
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in off.iter().take(5).skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        PdfReader::from_bytes(pdf).unwrap()
    }

    fn reader_with_raw_rgb_image_object() -> PdfReader {
        let pixels = [
            0u8, 1, 2, 10, 11, 12, 20, 21, 22, 30, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62, 70,
            71, 72, 80, 81, 82, 90, 91, 92, 100, 101, 102, 110, 111, 112,
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut off = [0usize; 5];
        off[1] = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        off[2] = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        off[3] = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>\nendobj\n",
        );
        off[4] = pdf.len();
        pdf.extend_from_slice(
            format!(
                "4 0 obj\n<< /Type /XObject /Subtype /Image /Width 4 /Height 3 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Length {} >>\nstream\n",
                pixels.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&pixels);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in off.iter().take(5).skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        PdfReader::from_bytes(pdf).unwrap()
    }

    fn reader_with_raw_image_mask_object() -> PdfReader {
        let pixels = [0b1010_1010, 0b0101_0101];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut off = [0usize; 5];
        off[1] = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        off[2] = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        off[3] = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>\nendobj\n",
        );
        off[4] = pdf.len();
        pdf.extend_from_slice(
            format!(
                "4 0 obj\n<< /Type /XObject /Subtype /Image /Width 8 /Height 2 /ImageMask true /BitsPerComponent 1 /Length {} >>\nstream\n",
                pixels.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&pixels);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in off.iter().take(5).skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        PdfReader::from_bytes(pdf).unwrap()
    }

    fn icc_color_space_ref() -> PdfObject {
        PdfObject::Array(vec![
            PdfObject::Name("ICCBased".to_string()),
            PdfObject::Reference {
                number: 4,
                generation: 0,
            },
        ])
    }

    fn real_arr(vals: &[f64]) -> PdfObject {
        PdfObject::Array(vals.iter().map(|&value| PdfObject::Real(value)).collect())
    }

    fn dict_obj(entries: &[(&str, PdfObject)]) -> PdfObject {
        let mut dict = PdfDictionary::empty();
        for (key, value) in entries {
            dict.insert(*key, value.clone());
        }
        PdfObject::Dictionary(dict)
    }

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
    fn normalise_bit_depth_16_bit_rejects_short_sample() {
        let error = ImageDecoder::normalise_bit_depth(vec![0xAB], 1, 1, 1, 16)
            .expect_err("short 16-bit image sample must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("16-bit image data has 1 bytes, expected 2"));
    }

    #[test]
    fn normalise_bit_depth_16_bit_rejects_extra_sample_bytes() {
        let error = ImageDecoder::normalise_bit_depth(vec![0xAB, 0xCD, 0xEF], 1, 1, 1, 16)
            .expect_err("extra 16-bit image bytes must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("16-bit image data has 3 bytes, expected 2"));
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
    fn indexed_palette_index_maps_normalised_low_bit_samples() {
        assert_eq!(indexed_palette_index(0, 1, 1), 0);
        assert_eq!(indexed_palette_index(255, 1, 1), 1);
        assert_eq!(indexed_palette_index(85, 2, 3), 1);
        assert_eq!(indexed_palette_index(170, 2, 3), 2);
        assert_eq!(indexed_palette_index(255, 2, 3), 3);
    }

    #[test]
    fn indexed_palette_length_guard_rejects_short_converted_palette() {
        let error = ensure_indexed_palette_len(5, 2, 3)
            .expect_err("short converted Indexed palette must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("converted palette has 5 bytes, expected 6"));
    }

    #[test]
    fn indexed_palette_length_guard_rejects_long_converted_palette() {
        let error = ensure_indexed_palette_len(7, 2, 3)
            .expect_err("long converted Indexed palette must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("converted palette has 7 bytes, expected 6"));
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
            ImageColorContext::default(),
        )
        .unwrap();

        assert_eq!(image.channels, 3);
        assert_eq!(image.pixels, vec![255, 127, 0]);
    }

    #[test]
    fn dct_finish_rejects_pdf_color_space_component_mismatch() {
        let dict = PdfDictionary::empty();
        let error = ImageDecoder::finish_dct_decoded_image(
            vec![128],
            1,
            1,
            1,
            "DeviceRGB",
            &dict,
            ImageColorContext::default(),
        )
        .expect_err("DCT component mismatch must fail typed");
        assert!(matches!(error, WellfriendError::UnsupportedFeature(_)));
        assert!(format!("{error}").contains("produced 1 components"));
    }

    #[test]
    fn dct_finish_rejects_short_cmyk_before_conversion() {
        let dict = PdfDictionary::empty();
        let error = ImageDecoder::finish_dct_decoded_image(
            vec![0, 0, 0],
            1,
            1,
            4,
            "DeviceCMYK",
            &dict,
            ImageColorContext::default(),
        )
        .expect_err("DCT CMYK finalizer must not drop trailing malformed samples");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("image 1x1 x4 channels decoded 3 bytes, expected 4"),
            "unexpected DCT CMYK length error: {error}"
        );
    }

    #[test]
    fn dct_finish_cmyk_applies_decode_budget() {
        let dict = PdfDictionary::empty();
        let error = ImageDecoder::finish_dct_decoded_image(
            Vec::new(),
            60_000,
            60_000,
            4,
            "DeviceCMYK",
            &dict,
            ImageColorContext::default(),
        )
        .expect_err("DCT CMYK finalizer must apply the decode pixel budget");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("exceeds decode cap"),
            "unexpected DCT CMYK decode-budget error: {error}"
        );
    }

    #[test]
    fn monochrome_terminal_color_space_guard_accepts_device_gray() {
        ImageDecoder::ensure_monochrome_terminal_color_space("CCITTFaxDecode", "DeviceGray")
            .unwrap();
        ImageDecoder::ensure_monochrome_terminal_color_space("CCF", "G").unwrap();
        ImageDecoder::ensure_monochrome_terminal_color_space("JBIG2Decode", "DeviceGray").unwrap();
        ImageDecoder::ensure_monochrome_terminal_color_space("DCTDecode", "DeviceRGB").unwrap();
    }

    #[test]
    fn monochrome_terminal_color_space_guard_rejects_non_gray() {
        for (filter, color_space) in [
            ("CCITTFaxDecode", "DeviceRGB"),
            ("CCF", "DeviceCMYK"),
            ("JBIG2Decode", "DeviceRGB"),
        ] {
            let error = ImageDecoder::ensure_monochrome_terminal_color_space(filter, color_space)
                .expect_err("monochrome terminal image must not ignore non-gray ColorSpace");
            assert!(matches!(error, WellfriendError::UnsupportedFeature(_)));
            assert!(
                format!("{error}").contains("decodes monochrome samples"),
                "{filter} /{color_space} should explain the monochrome terminal guard: {error}"
            );
        }
    }

    #[test]
    fn inline_monochrome_terminal_decode_rejects_non_gray_before_codec() {
        for (filter, color_space) in [
            ("CCITTFaxDecode", "DeviceRGB"),
            ("JBIG2Decode", "DeviceCMYK"),
        ] {
            let error = ImageDecoder::decode_inline_with_resolved_color_space_and_param_array(
                b"not a real terminal stream",
                1,
                1,
                1,
                color_space,
                None,
                &[filter],
                &[None],
                &DecodeLimits::default(),
                None,
                ColorTransformOptions::default(),
            )
            .expect_err("non-gray monochrome terminal inline images must fail before decode");
            assert!(matches!(error, WellfriendError::UnsupportedFeature(_)));
            assert!(
                format!("{error}").contains(&format!("PDF ColorSpace /{color_space}")),
                "{filter} /{color_space} should report the declared color-space mismatch: {error}"
            );
        }
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

        let image =
            ImageDecoder::finish_jpx_decoded_image(raw, 1, 1, "inline image", &dict).unwrap();

        assert_eq!(image.channels, 3);
        assert_eq!(image.pixels, vec![10, 20, 30]);
    }

    #[test]
    fn jpx_abbreviation_is_canonical_filter_alias() {
        assert!(same_filter("JPX", "JPXDecode"));
        assert!(same_filter("JPXDecode", "JPX"));
    }

    #[test]
    fn inline_jpx_abbreviation_reaches_jpx_decoder() {
        let error = ImageDecoder::decode_inline_with_param_array(
            b"not a jpeg2000 codestream",
            1,
            1,
            8,
            "DeviceGray",
            &["JPX"],
            &[None],
            &DecodeLimits::default(),
        )
        .expect_err("JPX abbreviation should route to the JPX decoder");

        assert!(
            matches!(error, WellfriendError::MalformedPdf(_)),
            "expected JPX parser failure, got {error:?}"
        );
        assert!(
            format!("{error}").contains("JPXDecode parse failed"),
            "unexpected JPX abbreviation error: {error}"
        );
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

        let image =
            ImageDecoder::finish_jpx_decoded_image(raw, 1, 1, "inline image", &dict).unwrap();

        assert_eq!(image.channels, 4);
        assert_eq!(image.pixels, vec![128, 64, 32, 128]);
    }

    #[test]
    fn jpx_smask_in_data_requires_decoded_alpha_channel() {
        for smask_in_data in [1, 2] {
            let mut dict = PdfDictionary::empty();
            dict.insert("SMaskInData", PdfObject::Integer(smask_in_data));
            let raw = RawImage {
                width: 1,
                height: 1,
                channels: 3,
                bits_per_sample: 8,
                pixels: vec![10, 20, 30],
            };

            let error = ImageDecoder::finish_jpx_decoded_image(raw, 1, 1, "inline image", &dict)
                .expect_err("declared JPX internal alpha requires decoded alpha samples");
            assert!(matches!(error, WellfriendError::MalformedPdf(_)));
            assert!(
                format!("{error}").contains(&format!(
                    "JPXDecode inline image declares /SMaskInData {smask_in_data} but decoded image has 3 channels"
                )),
                "unexpected JPX /SMaskInData alpha-channel error: {error}"
            );
        }
    }

    #[test]
    fn jpx_smask_in_data_rejects_malformed_metadata() {
        let raw = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30, 40],
        };

        let mut invalid_value = PdfDictionary::empty();
        invalid_value.insert("SMaskInData", PdfObject::Integer(3));
        let error =
            ImageDecoder::finish_jpx_decoded_image(raw.clone(), 1, 1, "image Im0", &invalid_value)
                .expect_err("invalid JPX /SMaskInData value must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("JPXDecode image Im0 /SMaskInData must be 0, 1, or 2"),
            "unexpected JPX /SMaskInData value error: {error}"
        );

        let mut malformed_type = PdfDictionary::empty();
        malformed_type.insert("SMaskInData", PdfObject::Name("Yes".to_string()));
        let error = ImageDecoder::finish_jpx_decoded_image(raw, 1, 1, "image Im0", &malformed_type)
            .expect_err("non-integer JPX /SMaskInData value must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("JPXDecode image Im0 /SMaskInData must be an integer"),
            "unexpected JPX /SMaskInData type error: {error}"
        );
    }

    #[test]
    fn jpx_finish_rejects_pdf_dimension_mismatch() {
        let raw = RawImage {
            width: 2,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30, 40, 50, 60],
        };
        let error =
            ImageDecoder::finish_jpx_decoded_image(raw, 1, 1, "image Im0", &PdfDictionary::empty())
                .expect_err("JPX decoded dimensions must match PDF image dictionary");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains(
                "JPXDecode image Im0 dictionary dimensions 1x1 differ from decoded JPX dimensions 2x1"
            ),
            "unexpected JPX dimension mismatch error: {error}"
        );
    }

    #[test]
    fn reduced_jpx_finish_accepts_smaller_decoded_dimensions_after_original_match() {
        let raw = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30],
        };

        let image = ImageDecoder::finish_reduced_jpx_decoded_image(
            raw,
            2,
            2,
            2,
            2,
            "image Im0",
            &PdfDictionary::empty(),
        )
        .expect("reduced JPX decode should validate original dimensions, not reduced dimensions");

        assert_eq!(image.width, 1);
        assert_eq!(image.height, 1);
        assert_eq!(image.pixels, vec![10, 20, 30]);
    }

    #[test]
    fn reduced_jpx_finish_rejects_original_dimension_mismatch() {
        let raw = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30],
        };

        let error = ImageDecoder::finish_reduced_jpx_decoded_image(
            raw,
            2,
            2,
            3,
            2,
            "image Im0",
            &PdfDictionary::empty(),
        )
        .expect_err("reduced JPX decode must still validate original JPX dimensions");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains(
                "JPXDecode image Im0 dictionary dimensions 2x2 differ from decoded JPX dimensions 3x2"
            ),
            "unexpected reduced JPX original-dimension error: {error}"
        );
    }

    #[test]
    fn reduced_jpx_finish_rejects_decoded_dimensions_larger_than_original() {
        let raw = RawImage {
            width: 3,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30, 40, 50, 60, 70, 80, 90],
        };

        let error = ImageDecoder::finish_reduced_jpx_decoded_image(
            raw,
            2,
            2,
            2,
            2,
            "image Im0",
            &PdfDictionary::empty(),
        )
        .expect_err("reduced JPX decoded dimensions must not exceed original dimensions");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains(
                "JPXDecode image Im0 reduced decode dimensions 3x1 exceed codestream dimensions 2x2"
            ),
            "unexpected reduced JPX decoded-dimension error: {error}"
        );
    }

    #[test]
    fn jpx_finish_rejects_non_8_bit_raw_output() {
        let raw = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 16,
            pixels: vec![10, 20, 30],
        };

        let error =
            ImageDecoder::finish_jpx_decoded_image(raw, 1, 1, "image Im0", &PdfDictionary::empty())
                .expect_err("JPX finalizer must enforce normalized 8-bit samples");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("JPXDecode image Im0 decoded 16 bits per sample"),
            "unexpected JPX bits-per-sample error: {error}"
        );
    }

    #[test]
    fn jpx_finish_rejects_decoded_length_mismatch() {
        let raw = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![10, 20],
        };

        let error =
            ImageDecoder::finish_jpx_decoded_image(raw, 1, 1, "image Im0", &PdfDictionary::empty())
                .expect_err("JPX finalizer must enforce exact decoded byte length");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}")
                .contains("JPXDecode image Im0 decoded 2 bytes, expected 3 for 1x1 x3 channels"),
            "unexpected JPX decoded-length error: {error}"
        );
    }

    #[test]
    fn jpx_finish_applies_decode_budget() {
        let raw = RawImage {
            width: 60_000,
            height: 60_000,
            channels: 3,
            bits_per_sample: 8,
            pixels: Vec::new(),
        };

        let error = ImageDecoder::finish_jpx_decoded_image(
            raw,
            60_000,
            60_000,
            "image Im0",
            &PdfDictionary::empty(),
        )
        .expect_err("JPX finalizer must apply the decode pixel budget");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("exceeds decode cap"),
            "unexpected JPX decode-budget error: {error}"
        );
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
    fn color_space_converter_rejects_short_device_cmyk_input() {
        let reader =
            crate::reader::PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf())
                .unwrap();
        let dict = PdfDictionary::empty();

        let error = ColorSpaceConverter::convert(vec![0, 0, 0], 1, 1, "DeviceCMYK", &dict, &reader)
            .expect_err("DeviceCMYK conversion must not drop trailing malformed samples");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains(
                "DeviceCMYK image ColorSpace decoded 3 bytes, expected 4 for 1x1 x4 channels"
            ),
            "unexpected DeviceCMYK converter length error: {error}"
        );
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
            ColorSpaceConverter::decode_indexed(&[0, 1], 8, &dict, &reader, 2, 1).unwrap();

        assert_eq!(channels, 3);
        assert_eq!(pixels, vec![255, 255, 255, 35, 31, 32]);
    }

    #[test]
    fn indexed_image_rejects_overlong_lookup_table() {
        let reader =
            crate::reader::PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf())
                .unwrap();
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("Indexed".to_string()),
                PdfObject::Name("DeviceRGB".to_string()),
                PdfObject::Integer(1),
                PdfObject::String(vec![255, 0, 0, 0, 0, 255, 0]),
            ]),
        );

        let error = ColorSpaceConverter::decode_indexed(&[0, 1], 8, &dict, &reader, 2, 1)
            .expect_err("overlong Indexed lookup table must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("lookup table has 7 bytes, expected 6"),
            "unexpected Indexed lookup length error: {error}"
        );
    }

    #[test]
    fn indexed_image_rejects_overlong_color_space_array() {
        let reader =
            crate::reader::PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf())
                .unwrap();
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("Indexed".to_string()),
                PdfObject::Name("DeviceRGB".to_string()),
                PdfObject::Integer(1),
                PdfObject::String(vec![255, 0, 0, 0, 0, 255]),
                PdfObject::Name("Ignored".to_string()),
            ]),
        );

        let error = ColorSpaceConverter::decode_indexed(&[0, 1], 8, &dict, &reader, 2, 1)
            .expect_err("overlong Indexed ColorSpace array must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("Indexed image ColorSpace has 5 entries, expected 4"),
            "unexpected Indexed array arity error: {error}"
        );
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
    fn build_raw_image_uses_one_channel_for_image_mask() {
        let mut dict = PdfDictionary::empty();
        dict.insert("ImageMask", PdfObject::Boolean(true));

        let img = ImageDecoder::build_raw_image_pub(vec![0b1000_0000], 1, 1, 1, "DeviceRGB", &dict)
            .expect("image masks decode as one-channel stencils");

        assert_eq!(img.channels, 1);
        assert_eq!(img.pixels, vec![255]);
    }

    #[test]
    fn build_raw_image_rejects_malformed_decode_array() {
        for (decode, expected) in [
            (
                vec![PdfObject::Real(0.0), PdfObject::Real(1.0)],
                "image /Decode has 2 entries, expected 6",
            ),
            (
                vec![
                    PdfObject::Real(0.0),
                    PdfObject::Real(1.0),
                    PdfObject::Real(0.0),
                    PdfObject::Real(1.0),
                    PdfObject::Real(0.0),
                    PdfObject::Real(1.0),
                    PdfObject::Real(0.5),
                ],
                "image /Decode has 7 entries, expected 6",
            ),
            (
                vec![
                    PdfObject::Real(0.0),
                    PdfObject::Real(1.0),
                    PdfObject::Real(0.0),
                    PdfObject::Name("Bad".to_string()),
                    PdfObject::Real(0.0),
                    PdfObject::Real(1.0),
                ],
                "image /Decode entry 4 resolved to Name, expected Number",
            ),
        ] {
            let mut dict = PdfDictionary::empty();
            dict.insert("Decode", PdfObject::Array(decode));

            let error =
                ImageDecoder::build_raw_image_pub(vec![255, 0, 0], 1, 1, 8, "DeviceRGB", &dict)
                    .expect_err("malformed RGB Decode array must not be ignored");

            assert!(
                format!("{error}").contains(expected),
                "expected {expected:?}, got {error}"
            );
        }
    }

    #[test]
    fn build_raw_image_rejects_malformed_image_mask_boolean() {
        for key in ["ImageMask", "IM"] {
            let mut dict = PdfDictionary::empty();
            dict.insert(key, PdfObject::Name("true".to_string()));

            let error = ImageDecoder::build_raw_image_pub(vec![255], 1, 1, 8, "DeviceGray", &dict)
                .expect_err("image mask metadata must be boolean, not name-valued");

            assert!(
                format!("{error}").contains("image /ImageMask must be a boolean"),
                "{key}: got {error}"
            );
        }
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
    fn cal_gray_image_requires_parameter_array() {
        let dict = PdfDictionary::empty();
        let error = ImageDecoder::build_raw_image_pub(vec![128], 1, 1, 8, "CalGray", &dict)
            .expect_err("CalGray image must not synthesize default parameters");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("CalGray image ColorSpace is missing"));
    }

    #[test]
    fn cal_rgb_image_rejects_missing_required_white_point() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("CalRGB".to_string()),
                dict_obj(&[("Gamma", real_arr(&[1.0, 1.0, 1.0]))]),
            ]),
        );

        let error = ImageDecoder::build_raw_image_pub(vec![1, 2, 3], 1, 1, 8, "CalRGB", &dict)
            .expect_err("CalRGB image must not default missing WhitePoint");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("missing /WhitePoint"));
    }

    #[test]
    fn cal_rgb_image_rejects_malformed_gamma_instead_of_identity_default() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("CalRGB".to_string()),
                dict_obj(&[
                    ("WhitePoint", real_arr(&[1.0, 1.0, 1.0])),
                    (
                        "Gamma",
                        PdfObject::Array(vec![
                            PdfObject::Real(1.0),
                            PdfObject::Name("Bad".to_string()),
                            PdfObject::Real(1.0),
                        ]),
                    ),
                ]),
            ]),
        );

        let error = ImageDecoder::build_raw_image_pub(vec![1, 2, 3], 1, 1, 8, "CalRGB", &dict)
            .expect_err("CalRGB image must not default malformed Gamma to identity");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("/Gamma"));
    }

    #[test]
    fn cal_rgb_image_rejects_overlong_color_space_array() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("CalRGB".to_string()),
                dict_obj(&[("WhitePoint", real_arr(&[1.0, 1.0, 1.0]))]),
                PdfObject::Name("Ignored".to_string()),
            ]),
        );

        let error = ImageDecoder::build_raw_image_pub(vec![1, 2, 3], 1, 1, 8, "CalRGB", &dict)
            .expect_err("CalRGB image must not ignore trailing ColorSpace entries");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("CalRGB image ColorSpace has 3 entries, expected 2"));
    }

    #[test]
    fn lab_image_rejects_invalid_range_instead_of_defaulting() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("Lab".to_string()),
                dict_obj(&[
                    ("WhitePoint", real_arr(&[1.0, 1.0, 1.0])),
                    ("Range", real_arr(&[100.0, -100.0, -100.0, 100.0])),
                ]),
            ]),
        );

        let error = ImageDecoder::build_raw_image_pub(vec![50, 128, 128], 1, 1, 8, "Lab", &dict)
            .expect_err("Lab image must not default malformed Range");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("/Range"));
    }

    #[test]
    fn indexed_calibrated_base_rejects_missing_white_point() {
        let reader =
            crate::reader::PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf())
                .unwrap();
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("Indexed".to_string()),
                PdfObject::Array(vec![
                    PdfObject::Name("CalRGB".to_string()),
                    dict_obj(&[("Gamma", real_arr(&[1.0, 1.0, 1.0]))]),
                ]),
                PdfObject::Integer(0),
                PdfObject::String(vec![0, 0, 0]),
            ]),
        );

        let error = ColorSpaceConverter::decode_indexed(&[0], 8, &dict, &reader, 1, 1)
            .expect_err("Indexed CalRGB base must not default missing WhitePoint");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("missing /WhitePoint"));
    }

    #[test]
    fn build_raw_image_rejects_mismatched_buffers() {
        let dict = PdfDictionary::empty();
        let short = ImageDecoder::build_raw_image_pub(vec![255], 2, 1, 8, "DeviceRGB", &dict)
            .expect_err("short RGB image data must not be padded");
        assert!(format!("{short}").contains("decoded 1 bytes, expected 6"));

        let long =
            ImageDecoder::build_raw_image_pub(vec![1, 2, 3, 4], 1, 1, 8, "DeviceGray", &dict)
                .expect_err("long grayscale image data must not be truncated");
        assert!(format!("{long}").contains("decoded 4 bytes, expected 1"));
    }

    #[test]
    fn device_n_image_missing_component_names_fails_before_channel_default() {
        let dict = PdfDictionary::empty();
        let error = ImageDecoder::build_raw_image_pub(vec![128], 1, 1, 8, "DeviceN", &dict)
            .expect_err("DeviceN image must not default to one channel");
        assert!(format!("{error}").contains("requires a resolved ColorSpace array"));

        let mut empty_names = PdfDictionary::empty();
        empty_names.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("DeviceN".to_string()),
                PdfObject::Array(vec![]),
                PdfObject::Name("DeviceCMYK".to_string()),
                PdfObject::Dictionary(PdfDictionary::empty()),
            ]),
        );
        let error = ImageDecoder::build_raw_image_pub(vec![128], 1, 1, 8, "DeviceN", &empty_names)
            .expect_err("empty DeviceN component array must fail typed");
        assert!(format!("{error}").contains("has no components"));
    }

    #[test]
    fn icc_based_image_without_reader_fails_before_rgb_default() {
        let dict = PdfDictionary::empty();
        let error = ImageDecoder::build_raw_image_pub(vec![1, 2, 3], 1, 1, 8, "ICCBased", &dict)
            .expect_err("ICCBased image must not default to three channels without ICC metadata");
        assert!(format!("{error}").contains("requires a PdfReader for channel metadata"));
    }

    #[test]
    fn icc_based_image_missing_profile_n_fails_before_rgb_default() {
        let reader = reader_with_icc_profile("");
        let mut dict = PdfDictionary::empty();
        dict.insert("ColorSpace", icc_color_space_ref());

        let error = ImageDecoder::build_raw_image(
            vec![1, 2, 3],
            1,
            1,
            8,
            "ICCBased",
            &dict,
            ImageColorContext::with_reader(&reader, ColorTransformOptions::default()),
        )
        .expect_err("ICCBased image must not default missing /N to RGB");

        assert!(format!("{error}").contains("missing supported channel metadata"));
    }

    #[test]
    fn indexed_device_n_base_empty_components_fails_before_channel_clamp() {
        let reader =
            crate::reader::PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf())
                .unwrap();
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("Indexed".to_string()),
                PdfObject::Array(vec![
                    PdfObject::Name("DeviceN".to_string()),
                    PdfObject::Array(vec![]),
                    PdfObject::Name("DeviceCMYK".to_string()),
                    PdfObject::Dictionary(PdfDictionary::empty()),
                ]),
                PdfObject::Integer(0),
                PdfObject::String(vec![0]),
            ]),
        );

        let error = ColorSpaceConverter::decode_indexed(&[0], 8, &dict, &reader, 1, 1)
            .expect_err("Indexed DeviceN base must not clamp empty components to one channel");

        assert!(format!("{error}").contains("DeviceN image ColorSpace has no components"));
    }

    #[test]
    fn indexed_icc_based_base_invalid_n_fails_before_channel_clamp() {
        let reader = reader_with_icc_profile("/N 0");
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "ColorSpace",
            PdfObject::Array(vec![
                PdfObject::Name("Indexed".to_string()),
                icc_color_space_ref(),
                PdfObject::Integer(0),
                PdfObject::String(vec![0]),
            ]),
        );

        let error = ColorSpaceConverter::decode_indexed(&[0], 8, &dict, &reader, 1, 1)
            .expect_err("Indexed ICCBased base must not clamp invalid /N to one channel");

        assert!(
            format!("{error}").contains("Indexed image ICCBased base ColorSpace is unsupported")
        );
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
    fn ccitt_params_reject_name_valued_booleans() {
        for key in ["BlackIs1", "EncodedByteAlign", "EndOfLine", "EndOfBlock"] {
            let mut dict = PdfDictionary::empty();
            dict.insert(key, PdfObject::Name("true".to_string()));

            let error = ccitt_decode_params(Some(&dict), 17, 23)
                .expect_err("CCITT boolean DecodeParms must be booleans");

            assert!(
                format!("{error}").contains(&format!("CCITTFaxDecode /{key} must be a boolean")),
                "{key}: got {error}"
            );
        }
    }

    fn pack_ccitt_test_bits(bits: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut byte = 0u8;
        let mut bit_count = 0u8;
        for ch in bits.chars().filter(|ch| !ch.is_whitespace()) {
            byte <<= 1;
            if ch == '1' {
                byte |= 1;
            }
            bit_count += 1;
            if bit_count == 8 {
                out.push(byte);
                byte = 0;
                bit_count = 0;
            }
        }
        if bit_count > 0 {
            byte <<= 8 - bit_count;
            out.push(byte);
        }
        out
    }

    #[test]
    fn inline_ccitt_window_decode_uses_requested_source_region() {
        let data = pack_ccitt_test_bits("0111 10 1000 0111 10 1000 0111 10 1000");
        let mut params = PdfDictionary::empty();
        params.insert("K", PdfObject::Integer(0));
        params.insert("Columns", PdfObject::Integer(8));
        params.insert("Rows", PdfObject::Integer(3));
        params.insert("EndOfBlock", PdfObject::Boolean(false));

        let image = ImageDecoder::decode_inline_ccitt_window_with_param_array(
            &data,
            8,
            3,
            "DeviceGray",
            &["CCITTFaxDecode"],
            &[Some(params)],
            ccitt::CcittDecodeWindow {
                x: 1,
                y: 1,
                width: 5,
                height: 1,
            },
            &DecodeLimits::default(),
        )
        .unwrap();

        assert_eq!(image.width, 5);
        assert_eq!(image.height, 1);
        assert_eq!(image.channels, 1);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(image.pixels, vec![255, 0, 0, 0, 255]);
    }

    #[test]
    fn raw_xobject_window_decode_uses_requested_source_region() {
        let reader = reader_with_raw_rgb_image_object();
        let image_ref = ImageReference {
            page_number: 1,
            xobject_name: "ImRaw".to_string(),
            object_number: 4,
            generation_number: 0,
            width: 4,
            height: 3,
            bits_per_component: 8,
            color_space: "DeviceRGB".to_string(),
            filter: vec![],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };

        let image = ImageDecoder::decode_raw_window_with_limits_and_color_transform_options(
            &image_ref,
            &reader,
            None,
            RawImageDecodeWindow {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            },
            &DecodeLimits::default(),
            ColorTransformOptions::default(),
        )
        .unwrap();

        assert_eq!(image.width, 2);
        assert_eq!(image.height, 2);
        assert_eq!(image.channels, 3);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(
            image.pixels,
            vec![50, 51, 52, 60, 61, 62, 90, 91, 92, 100, 101, 102]
        );
    }

    #[test]
    fn raw_xobject_window_component_selection_applies_decode_by_source_component() {
        let pixels = [0u8, 128, 255];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut off = [0usize; 5];
        off[1] = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        off[2] = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        off[3] = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>\nendobj\n",
        );
        off[4] = pdf.len();
        pdf.extend_from_slice(
            format!(
                "4 0 obj\n<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Decode [1 0 0 1 1 0] /Length {} >>\nstream\n",
                pixels.len()
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(&pixels);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in off.iter().take(5).skip(1) {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        let reader = PdfReader::from_bytes(pdf).unwrap();
        let image_ref = ImageReference {
            page_number: 1,
            xobject_name: "ImRaw".to_string(),
            object_number: 4,
            generation_number: 0,
            width: 1,
            height: 1,
            bits_per_component: 8,
            color_space: "DeviceRGB".to_string(),
            filter: vec![],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };

        let image =
            ImageDecoder::decode_raw_window_components_with_limits_and_color_transform_options(
                &image_ref,
                &reader,
                None,
                RawImageDecodeWindow {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                RawImageComponentSelection::Components(vec![0, 2]),
                &DecodeLimits::default(),
                ColorTransformOptions::default(),
            )
            .unwrap();

        assert_eq!(image.width, 1);
        assert_eq!(image.height, 1);
        assert_eq!(image.channels, 2);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(image.pixels, vec![255, 0]);
    }

    #[test]
    fn raw_window_xobject_decode_supports_image_mask_source_region() {
        let reader = reader_with_raw_image_mask_object();
        let image_ref = ImageReference {
            page_number: 1,
            xobject_name: "ImMask".to_string(),
            object_number: 4,
            generation_number: 0,
            width: 8,
            height: 2,
            bits_per_component: 1,
            color_space: "DeviceGray".to_string(),
            filter: vec![],
            is_inline: false,
            is_mask: true,
            is_smask: false,
            inline_data: None,
        };

        let image = ImageDecoder::decode_raw_window_with_limits_and_color_transform_options(
            &image_ref,
            &reader,
            None,
            RawImageDecodeWindow {
                x: 2,
                y: 0,
                width: 4,
                height: 2,
            },
            &DecodeLimits::default(),
            ColorTransformOptions::default(),
        )
        .unwrap();

        assert_eq!(image.width, 4);
        assert_eq!(image.height, 2);
        assert_eq!(image.channels, 1);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(image.pixels, vec![255, 0, 255, 0, 0, 255, 0, 255]);
    }

    #[test]
    fn inline_raw_window_decode_uses_requested_source_region() {
        let pixels = [
            0u8, 1, 2, 10, 11, 12, 20, 21, 22, 30, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62, 70,
            71, 72, 80, 81, 82, 90, 91, 92, 100, 101, 102, 110, 111, 112,
        ];

        let image =
            ImageDecoder::decode_inline_raw_window_with_resolved_color_space_and_param_array(
                &pixels,
                4,
                3,
                8,
                "DeviceRGB",
                None,
                &[],
                &[],
                RawImageDecodeWindow {
                    x: 1,
                    y: 1,
                    width: 2,
                    height: 2,
                },
                &DecodeLimits::default(),
                None,
                ColorTransformOptions::default(),
            )
            .unwrap();

        assert_eq!(image.width, 2);
        assert_eq!(image.height, 2);
        assert_eq!(image.channels, 3);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(
            image.pixels,
            vec![50, 51, 52, 60, 61, 62, 90, 91, 92, 100, 101, 102]
        );
    }

    #[test]
    fn inline_raw_window_component_selection_uses_requested_source_region() {
        let pixels = [
            0u8, 1, 2, 10, 11, 12, 20, 21, 22, 30, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62, 70,
            71, 72, 80, 81, 82, 90, 91, 92, 100, 101, 102, 110, 111, 112,
        ];

        let image =
            ImageDecoder::decode_inline_raw_window_components_with_resolved_color_space_and_param_array(
                &pixels,
                4,
                3,
                8,
                "DeviceRGB",
                None,
                &[],
                &[],
                RawImageDecodeWindow {
                    x: 1,
                    y: 1,
                    width: 2,
                    height: 2,
                },
                RawImageComponentSelection::Components(vec![0, 2]),
                &DecodeLimits::default(),
                None,
                ColorTransformOptions::default(),
            )
            .unwrap();

        assert_eq!(image.width, 2);
        assert_eq!(image.height, 2);
        assert_eq!(image.channels, 2);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(image.pixels, vec![50, 52, 60, 62, 90, 92, 100, 102]);
    }

    #[test]
    fn inline_raw_window_subbyte_decode_uses_requested_source_region() {
        let pixels = [0b1010_1010, 0b0101_0101];

        let image =
            ImageDecoder::decode_inline_raw_window_with_resolved_color_space_and_param_array(
                &pixels,
                8,
                2,
                1,
                "DeviceGray",
                None,
                &[],
                &[],
                RawImageDecodeWindow {
                    x: 2,
                    y: 0,
                    width: 4,
                    height: 2,
                },
                &DecodeLimits::default(),
                None,
                ColorTransformOptions::default(),
            )
            .unwrap();

        assert_eq!(image.width, 4);
        assert_eq!(image.height, 2);
        assert_eq!(image.channels, 1);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(image.pixels, vec![255, 0, 255, 0, 0, 255, 0, 255]);
    }

    #[test]
    fn inline_raw_window_subbyte_rgb_decode_preserves_component_order() {
        let pixels = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB];

        let image =
            ImageDecoder::decode_inline_raw_window_with_resolved_color_space_and_param_array(
                &pixels,
                4,
                1,
                4,
                "DeviceRGB",
                None,
                &[],
                &[],
                RawImageDecodeWindow {
                    x: 1,
                    y: 0,
                    width: 2,
                    height: 1,
                },
                &DecodeLimits::default(),
                None,
                ColorTransformOptions::default(),
            )
            .unwrap();

        assert_eq!(image.width, 2);
        assert_eq!(image.height, 1);
        assert_eq!(image.channels, 3);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(image.pixels, vec![51, 68, 85, 102, 119, 136]);
    }

    #[test]
    fn inline_raw_window_subbyte_component_selection_preserves_source_order() {
        let pixels = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB];

        let image =
            ImageDecoder::decode_inline_raw_window_components_with_resolved_color_space_and_param_array(
                &pixels,
                4,
                1,
                4,
                "DeviceRGB",
                None,
                &[],
                &[],
                RawImageDecodeWindow {
                    x: 1,
                    y: 0,
                    width: 2,
                    height: 1,
                },
                RawImageComponentSelection::Components(vec![0, 2]),
                &DecodeLimits::default(),
                None,
                ColorTransformOptions::default(),
            )
            .unwrap();

        assert_eq!(image.width, 2);
        assert_eq!(image.height, 1);
        assert_eq!(image.channels, 2);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(image.pixels, vec![51, 85, 102, 136]);
    }

    #[test]
    fn inline_raw_window_16bit_decode_uses_requested_source_region() {
        let pixels = [0x00, 0x01, 0x80, 0x02, 0xFF, 0x03];

        let image =
            ImageDecoder::decode_inline_raw_window_with_resolved_color_space_and_param_array(
                &pixels,
                3,
                1,
                16,
                "DeviceGray",
                None,
                &[],
                &[],
                RawImageDecodeWindow {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                &DecodeLimits::default(),
                None,
                ColorTransformOptions::default(),
            )
            .unwrap();

        assert_eq!(image.width, 1);
        assert_eq!(image.height, 1);
        assert_eq!(image.channels, 1);
        assert_eq!(image.bits_per_sample, 8);
        assert_eq!(image.pixels, vec![128]);
    }

    #[test]
    fn raw_window_decode_rejects_overlong_source_stream() {
        let error = crop_raw_8bit_window(
            &[0, 1, 2, 3],
            1,
            1,
            3,
            RawImageDecodeWindow {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            &DecodeLimits::default(),
        )
        .expect_err("raw source-window decode must require exact source length");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("raw image stream has 4 bytes, expected 3"));
    }

    #[test]
    fn inline_raw_window_decode_rejects_overlong_source_stream() {
        let error =
            ImageDecoder::decode_inline_raw_window_with_resolved_color_space_and_param_array(
                &[0, 1, 2, 3],
                1,
                1,
                8,
                "DeviceRGB",
                None,
                &[],
                &[],
                RawImageDecodeWindow {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                &DecodeLimits::default(),
                None,
                ColorTransformOptions::default(),
            )
            .expect_err("inline raw source-window decode must require exact source length");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("raw image stream has 4 bytes, expected 3"));
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

    #[test]
    fn jpeg_scaled_decode_uses_native_reduced_idct() {
        let mut pixels = Vec::new();
        for y in 0..16u8 {
            for x in 0..16u8 {
                pixels.push(x.saturating_mul(16));
                pixels.push(y.saturating_mul(16));
                pixels.push(128u8);
            }
        }
        let original = RawImage {
            width: 16,
            height: 16,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        };
        let jpeg = ImageEncoder::encode_jpeg(&original, 95).unwrap();
        let decoded = ImageDecoder::decode_jpeg_scaled_with_info(&jpeg, 4, 4).unwrap();

        assert_eq!(decoded.original_width, 16);
        assert_eq!(decoded.original_height, 16);
        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 4);
        assert_eq!(decoded.channels, 3);
        assert_eq!(decoded.pixels.len(), 4 * 4 * 3);
    }

    #[test]
    fn scaled_dct_inline_dimension_mismatch_uses_original_jpeg_header() {
        let original = RawImage {
            width: 8,
            height: 8,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![128; 8 * 8 * 3],
        };
        let jpeg = ImageEncoder::encode_jpeg(&original, 90).unwrap();
        let error =
            ImageDecoder::decode_inline_scaled_dct_with_resolved_color_space_and_param_array(
                &jpeg,
                4,
                8,
                "DeviceRGB",
                None,
                &["DCTDecode"],
                &[None],
                2,
                2,
                &DecodeLimits::default(),
                None,
                ColorTransformOptions::default(),
            )
            .expect_err("scaled DCT path must compare PDF dimensions with original JPEG header");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("dictionary dimensions 4x8 differ from JPEG header 8x8")
        );
    }

    #[test]
    fn dct_inline_dimension_mismatch_returns_malformed_pdf() {
        let original = RawImage {
            width: 2,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
        };
        let jpeg = ImageEncoder::encode_jpeg(&original, 90).unwrap();
        let error = ImageDecoder::decode_inline_with_limits(
            &jpeg,
            1,
            2,
            8,
            "DeviceRGB",
            &["DCTDecode"],
            None,
            &DecodeLimits::default(),
        )
        .expect_err("DCT dictionary/header dimension mismatch must fail typed");

        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(
            format!("{error}").contains("dictionary dimensions 1x2 differ from JPEG header 2x2")
        );
    }

    #[test]
    fn dct_inline_rejects_declared_color_space_component_mismatch() {
        let original = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0],
        };
        let jpeg = ImageEncoder::encode_jpeg(&original, 90).unwrap();
        let error = ImageDecoder::decode_inline_with_limits(
            &jpeg,
            1,
            1,
            8,
            "DeviceCMYK",
            &["DCTDecode"],
            None,
            &DecodeLimits::default(),
        )
        .expect_err("inline DCT decode must honor declared PDF ColorSpace component count");

        assert!(matches!(error, WellfriendError::UnsupportedFeature(_)));
        assert!(
            format!("{error}").contains(
                "DCTDecode image produced 3 components but PDF ColorSpace /DeviceCMYK expects 4"
            ),
            "unexpected inline DCT component-mismatch error: {error}"
        );
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
    fn short_subbyte_rows_return_malformed_pdf() {
        let error = unpack_subbyte_rows(&[0b1010_0000], 4, 4, 1, 1)
            .expect_err("short sub-byte image rows must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("sub-byte image data has 1 bytes"));
    }

    #[test]
    fn overlong_subbyte_rows_return_malformed_pdf() {
        let error = unpack_subbyte_rows(&[0b1010_0000, 0], 4, 1, 1, 1)
            .expect_err("overlong sub-byte image rows must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("sub-byte image data has 2 bytes, expected 1"));
    }

    #[test]
    fn build_raw_image_rejects_overlong_subbyte_rows() {
        let dict = PdfDictionary::empty();
        let error =
            ImageDecoder::build_raw_image_pub(vec![0b1000_0000, 0], 1, 1, 1, "DeviceGray", &dict)
                .expect_err("overlong sub-byte raw image data must fail typed");
        assert!(matches!(error, WellfriendError::MalformedPdf(_)));
        assert!(format!("{error}").contains("sub-byte image data has 2 bytes, expected 1"));
    }
}
