use std::io::{Cursor, Read};

use crate::error::{Result, WellfriendError};
use crate::filters::{decode_stream_lossless_reader_with_limits, DecodeLimits, StreamDecodeStatus};
use crate::images::decoder::{
    ImageDecoder, RawImage, RawImageComponentSelection, RawImageDecodeWindow,
};
use crate::images::locator::ImageReference;
use crate::object::PdfDictionary;
use crate::object::PdfObject;
use crate::reader::PdfReader;
use crate::render::cmm::ColorTransformOptions;
use crate::render::color::ColorSpaceHandler;

#[derive(Debug, Clone)]
pub struct SmaskLoader;

impl SmaskLoader {
    /// Decode and combine a soft mask for the main image, if one is present.
    pub fn load_and_combine(
        main_image: &ImageReference,
        main_raw: RawImage,
        reader: &PdfReader,
    ) -> Result<Option<RawImage>> {
        Self::load_and_combine_with_limits(main_image, main_raw, reader, &DecodeLimits::default())
    }

    pub fn load_and_combine_with_limits(
        main_image: &ImageReference,
        main_raw: RawImage,
        reader: &PdfReader,
        limits: &DecodeLimits,
    ) -> Result<Option<RawImage>> {
        Self::load_and_combine_with_limits_and_source_window(
            main_image, main_raw, reader, limits, None,
        )
    }

    pub(crate) fn load_and_combine_with_limits_and_source_window(
        main_image: &ImageReference,
        main_raw: RawImage,
        reader: &PdfReader,
        limits: &DecodeLimits,
        source_window: Option<RawImageDecodeWindow>,
    ) -> Result<Option<RawImage>> {
        let obj = reader.get_object(main_image.object_number, main_image.generation_number)?;
        let dict = match &obj {
            PdfObject::Stream { dict, .. } => dict.clone(),
            other => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "image SMask parent resolved to {}, expected Stream",
                    other.variant_name()
                )))
            }
        };

        let Some(smask_value) = dict.get("SMask") else {
            return Ok(None);
        };
        let smask_ref = match smask_value {
            PdfObject::Reference { number, generation } => (*number, *generation),
            PdfObject::Name(name) if name == "None" => return Ok(None),
            other => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "unsupported image SMask value {}",
                    other.variant_name()
                )))
            }
        };
        let (smask_dict, smask_raw_stream) = match reader.get_object(smask_ref.0, smask_ref.1)? {
            PdfObject::Stream { dict, raw } => (dict, raw),
            other => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "image SMask reference resolved to {}, expected Stream",
                    other.variant_name()
                )))
            }
        };
        let matte = smask_matte_rgb(&smask_dict, &main_image.color_space)?;

        let smask_label = format!("image SMask for {}", main_image.xobject_name);
        let smask_filter = smask_filter_names(&smask_dict, &smask_label)?;
        let smask_image_ref = ImageReference {
            page_number: main_image.page_number,
            xobject_name: format!("{}_smask", main_image.xobject_name),
            object_number: smask_ref.0,
            generation_number: smask_ref.1,
            width: smask_required_positive_u32(&smask_dict, "Width", "W", &smask_label)?,
            height: smask_required_positive_u32(&smask_dict, "Height", "H", &smask_label)?,
            bits_per_component: smask_bits_per_component(&smask_dict, &smask_label)?,
            color_space: smask_required_color_space_name(&smask_dict, &smask_label)?,
            filter: smask_filter,
            is_inline: false,
            is_mask: false,
            is_smask: true,
            inline_data: None,
        };

        let reduced_target = (source_window.is_none()
            && (main_raw.width != main_image.width || main_raw.height != main_image.height))
            .then_some((main_raw.width, main_raw.height));
        let smask_raw = match (source_window, reduced_target) {
            (Some(window), _) => {
                if !smask_source_window_is_compatible(
                    main_image,
                    &main_raw,
                    &smask_image_ref,
                    window,
                ) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "image SMask source-window decode requires matching unfiltered grayscale soft mask"
                            .to_string(),
                    ));
                }
                ImageDecoder::decode_raw_window_components_with_limits_and_color_transform_options(
                    &smask_image_ref,
                    reader,
                    None,
                    window,
                    RawImageComponentSelection::All,
                    limits,
                    ColorTransformOptions::default(),
                    None,
                )
            }
            (None, Some((target_width, target_height))) => decode_reduced_grayscale_smask(
                &smask_dict,
                smask_raw_stream,
                &smask_image_ref,
                target_width,
                target_height,
                reader,
                limits,
            ),
            (None, None) => ImageDecoder::decode_with_limits(&smask_image_ref, reader, limits),
        }
        .map_err(|err| {
                WellfriendError::MalformedPdf(format!(
                    "image SMask decode failed for {}: {err}",
                    main_image.xobject_name
                ))
            })?;

        Self::combine_rgba_with_matte(main_raw, smask_raw, matte).map(Some)
    }

    /// Combine a main image with a grayscale alpha mask into RGBA.
    pub fn combine_rgba(main: RawImage, mask: RawImage) -> Result<RawImage> {
        Self::combine_rgba_with_matte(main, mask, None)
    }

    /// Combine a main image with a grayscale alpha mask into RGBA, optionally
    /// undoing producer-side matte preblending from an image SMask `/Matte`.
    pub fn combine_rgba_with_matte(
        main: RawImage,
        mask: RawImage,
        matte: Option<[u8; 3]>,
    ) -> Result<RawImage> {
        if main.width != mask.width || main.height != mask.height {
            return Err(WellfriendError::MalformedPdf(format!(
                "image SMask dimensions {}x{} do not match image {}x{}",
                mask.width, mask.height, main.width, main.height
            )));
        }

        let pixel_count =
            usize::try_from(u64::from(main.width) * u64::from(main.height)).map_err(|_| {
                WellfriendError::MalformedPdf(format!(
                    "image SMask dimensions {}x{} overflow addressable memory",
                    main.width, main.height
                ))
            })?;
        if mask.channels != 1 {
            return Err(WellfriendError::MalformedPdf(format!(
                "image SMask must decode to one alpha channel, got {} channels",
                mask.channels
            )));
        }
        ensure_smask_buffer_len("mask", mask.pixels.len(), pixel_count)?;
        let rgb: Vec<u8> = match main.channels {
            1 => {
                ensure_smask_buffer_len("main image", main.pixels.len(), pixel_count)?;
                main.pixels.iter().flat_map(|&g| [g, g, g]).collect()
            }
            3 => {
                ensure_smask_buffer_len("main image", main.pixels.len(), pixel_count * 3)?;
                main.pixels.clone()
            }
            4 => {
                return Err(WellfriendError::UnsupportedFeature(
                    "image SMask combine does not support a pre-alpha main image".to_string(),
                ))
            }
            channels => {
                return Err(WellfriendError::MalformedPdf(format!(
                    "SMask combine: unsupported main image channel count {}",
                    channels
                )))
            }
        };

        let mut rgba = Vec::with_capacity(pixel_count * 4);
        for i in 0..pixel_count {
            let rgb_offset = i * 3;
            let r = rgb[rgb_offset];
            let g = rgb[rgb_offset + 1];
            let b = rgb[rgb_offset + 2];
            let a = mask.pixels[i];
            let (r, g, b) = match matte {
                Some(matte) => (
                    unmatte_channel(r, matte[0], a),
                    unmatte_channel(g, matte[1], a),
                    unmatte_channel(b, matte[2], a),
                ),
                None => (r, g, b),
            };
            rgba.push(r);
            rgba.push(g);
            rgba.push(b);
            rgba.push(a);
        }

        Ok(RawImage {
            width: main.width,
            height: main.height,
            channels: 4,
            bits_per_sample: 8,
            pixels: rgba,
        })
    }
}

fn decode_reduced_grayscale_smask(
    dict: &PdfDictionary,
    raw_stream: Vec<u8>,
    image: &ImageReference,
    target_width: u32,
    target_height: u32,
    reader: &PdfReader,
    limits: &DecodeLimits,
) -> Result<RawImage> {
    if image.bits_per_component != 8
        || !matches!(image.color_space.as_str(), "DeviceGray" | "G")
        || target_width == 0
        || target_height == 0
        || target_width > image.width
        || target_height > image.height
        || image.width > limits.max_image_width
        || image.height > limits.max_image_height
        || u64::from(target_width).saturating_mul(u64::from(target_height))
            > limits.max_image_pixels
        || !decode_array_is_default(dict)
    {
        return Err(WellfriendError::UnsupportedFeature(
            "image SMask reduced decode requires matching 8-bit default-decoded grayscale data"
                .to_string(),
        ));
    }

    let decoded = decode_stream_lossless_reader_with_limits(
        dict,
        Cursor::new(raw_stream),
        Some(reader),
        limits,
    )?;
    if let StreamDecodeStatus::StoppedAtImageFilter(filter) = decoded.status {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "image SMask reduced decode stopped at image filter {filter}"
        )));
    }

    let source_width = image.width as usize;
    let source_height = image.height as usize;
    let destination_width = target_width as usize;
    let destination_height = target_height as usize;
    let destination_pixels = destination_width
        .checked_mul(destination_height)
        .ok_or_else(|| WellfriendError::MalformedPdf("image SMask target size overflow".into()))?;
    let destination_bytes = destination_pixels as u64;
    if destination_bytes > limits.max_image_decoded_bytes {
        return Err(WellfriendError::ResourceLimit(format!(
            "image SMask reduced output requires {destination_bytes} bytes, exceeding limit {}",
            limits.max_image_decoded_bytes
        )));
    }

    let mut source_row = vec![0u8; source_width];
    let mut horizontal = vec![0.0_f64; destination_width];
    let mut accumulated = vec![0.0_f64; destination_pixels];
    let source_per_destination_x = source_width as f64 / destination_width as f64;
    let source_per_destination_y = source_height as f64 / destination_height as f64;
    let mut decoded_reader = decoded.reader;

    for source_y in 0..source_height {
        decoded_reader.read_exact(&mut source_row)?;
        for (destination_x, sample) in horizontal.iter_mut().enumerate() {
            let source_x0 = destination_x as f64 * source_per_destination_x;
            let source_x1 = (destination_x + 1) as f64 * source_per_destination_x;
            let first_source_x = source_x0.floor() as usize;
            let last_source_x = source_x1.ceil().min(source_width as f64) as usize;
            let mut value = 0.0;
            for source_x in first_source_x..last_source_x {
                let weight =
                    ((source_x + 1) as f64).min(source_x1) - (source_x as f64).max(source_x0);
                if weight > 0.0 {
                    value += source_row[source_x] as f64 * weight;
                }
            }
            *sample = value / source_per_destination_x;
        }

        let destination_y0 = source_y as f64 / source_per_destination_y;
        let destination_y1 = (source_y + 1) as f64 / source_per_destination_y;
        let first_destination_y = destination_y0.floor() as usize;
        let last_destination_y = destination_y1.ceil().min(destination_height as f64) as usize;
        for destination_y in first_destination_y..last_destination_y {
            let destination_source_y0 = destination_y as f64 * source_per_destination_y;
            let destination_source_y1 = (destination_y + 1) as f64 * source_per_destination_y;
            let weight = ((source_y + 1) as f64).min(destination_source_y1)
                - (source_y as f64).max(destination_source_y0);
            if weight <= 0.0 {
                continue;
            }
            let output_base = destination_y * destination_width;
            for (destination_x, sample) in horizontal.iter().enumerate() {
                accumulated[output_base + destination_x] += sample * weight;
            }
        }
    }

    let mut trailing = [0u8; 1];
    if decoded_reader.read(&mut trailing)? != 0 {
        return Err(WellfriendError::MalformedPdf(
            "image SMask decoded stream contains trailing samples".into(),
        ));
    }
    let pixels = accumulated
        .into_iter()
        .map(|value| (value / source_per_destination_y).round().clamp(0.0, 255.0) as u8)
        .collect();
    Ok(RawImage {
        width: target_width,
        height: target_height,
        channels: 1,
        bits_per_sample: 8,
        pixels,
    })
}

fn decode_array_is_default(dict: &PdfDictionary) -> bool {
    let Some(value) = dict.get("Decode").or_else(|| dict.get("D")) else {
        return true;
    };
    let Some(items) = value.as_array() else {
        return false;
    };
    items.len() == 2
        && items[0]
            .as_number()
            .is_some_and(|value| value.is_finite() && value.abs() <= f64::EPSILON)
        && items[1]
            .as_number()
            .is_some_and(|value| value.is_finite() && (value - 1.0).abs() <= f64::EPSILON)
}

fn smask_filter_names(dict: &PdfDictionary, label: &str) -> Result<Vec<String>> {
    match dict.get("Filter").or_else(|| dict.get("F")) {
        None => Ok(Vec::new()),
        Some(PdfObject::Name(name)) => Ok(vec![name.clone()]),
        Some(PdfObject::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_name().map(|name| name.to_string()).ok_or_else(|| {
                    WellfriendError::MalformedPdf(format!(
                        "{label} /Filter array contains a non-name entry"
                    ))
                })
            })
            .collect(),
        Some(other) => Err(WellfriendError::MalformedPdf(format!(
            "{label} /Filter expected Name or Array, got {}",
            other.variant_name()
        ))),
    }
}

fn smask_source_window_is_compatible(
    main_image: &ImageReference,
    main_raw: &RawImage,
    smask_image: &ImageReference,
    window: RawImageDecodeWindow,
) -> bool {
    main_image.width == smask_image.width
        && main_image.height == smask_image.height
        && main_raw.width == window.width
        && main_raw.height == window.height
        && smask_image.filter.is_empty()
        && matches!(smask_image.color_space.as_str(), "DeviceGray" | "G")
        && matches!(smask_image.bits_per_component, 1 | 2 | 4 | 8 | 16)
}

fn ensure_smask_buffer_len(role: &str, actual: usize, expected: usize) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    Err(WellfriendError::MalformedPdf(format!(
        "image SMask {role} decoded {actual} bytes, expected {expected}"
    )))
}

fn smask_matte_rgb(dict: &PdfDictionary, main_color_space: &str) -> Result<Option<[u8; 3]>> {
    let Some(matte) = dict.get("Matte") else {
        return Ok(None);
    };
    let Some(items) = matte.as_array() else {
        return Err(WellfriendError::MalformedPdf(format!(
            "image SMask /Matte must be an array, got {}",
            matte.variant_name()
        )));
    };
    let comps = items
        .iter()
        .map(|item| {
            item.as_number().ok_or_else(|| {
                WellfriendError::MalformedPdf(format!(
                    "image SMask /Matte contains {}",
                    item.variant_name()
                ))
            })
        })
        .collect::<Result<Vec<f64>>>()?;
    if comps.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "image SMask /Matte is empty".to_string(),
        ));
    }
    let Some(color) = ColorSpaceHandler::try_from_components(main_color_space, &comps, 1.0) else {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "image SMask /Matte uses unsupported color space /{main_color_space}"
        )));
    };
    let color = color.to_pixel_color();
    Ok(Some([color[0], color[1], color[2]]))
}

fn smask_required_positive_u32(
    dict: &PdfDictionary,
    key: &str,
    short_key: &str,
    label: &str,
) -> Result<u32> {
    let value = dict
        .get_integer(key)
        .or_else(|| dict.get_integer(short_key));
    match value {
        Some(number) if number > 0 => u32::try_from(number).map_err(|_| {
            WellfriendError::MalformedPdf(format!(
                "{label} /{key} exceeds renderer dimension limit"
            ))
        }),
        Some(_) => Err(WellfriendError::MalformedPdf(format!(
            "{label} /{key} must be a positive integer"
        ))),
        None if dict.contains_key(key) || dict.contains_key(short_key) => Err(
            WellfriendError::MalformedPdf(format!("{label} /{key} is not an integer")),
        ),
        None => Err(WellfriendError::MalformedPdf(format!(
            "{label} missing /{key}"
        ))),
    }
}

fn smask_bits_per_component(dict: &PdfDictionary, label: &str) -> Result<u8> {
    let value = dict
        .get_integer("BitsPerComponent")
        .or_else(|| dict.get_integer("BPC"));
    match value {
        Some(number @ (1 | 2 | 4 | 8 | 16)) => Ok(number as u8),
        Some(_) => Err(WellfriendError::MalformedPdf(format!(
            "{label} /BitsPerComponent must be one of 1, 2, 4, 8, or 16"
        ))),
        None if dict.contains_key("BitsPerComponent") || dict.contains_key("BPC") => Err(
            WellfriendError::MalformedPdf(format!("{label} /BitsPerComponent is not an integer")),
        ),
        None => Err(WellfriendError::MalformedPdf(format!(
            "{label} missing /BitsPerComponent"
        ))),
    }
}

fn smask_required_color_space_name(dict: &PdfDictionary, label: &str) -> Result<String> {
    match dict.get("ColorSpace").or_else(|| dict.get("CS")) {
        Some(PdfObject::Name(name)) => Ok(canonical_smask_color_space_name(name)),
        Some(PdfObject::Array(items)) => items
            .first()
            .and_then(PdfObject::as_name)
            .map(canonical_smask_color_space_name)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(format!("{label} has malformed /ColorSpace"))
            }),
        Some(other) => Err(WellfriendError::MalformedPdf(format!(
            "{label} /ColorSpace resolved to {}, expected Name or Array",
            other.variant_name()
        ))),
        None => Err(WellfriendError::MalformedPdf(format!(
            "{label} missing /ColorSpace"
        ))),
    }
}

fn canonical_smask_color_space_name(name: &str) -> String {
    match name {
        "G" => "DeviceGray".to_string(),
        "RGB" => "DeviceRGB".to_string(),
        "CMYK" => "DeviceCMYK".to_string(),
        other => other.to_string(),
    }
}

fn unmatte_channel(sample: u8, matte: u8, alpha: u8) -> u8 {
    let a = alpha as f32 / 255.0;
    if a <= 1e-6 {
        return 0;
    }
    if a >= 0.999 {
        return sample;
    }
    let src = sample as f32 / 255.0;
    let matte = matte as f32 / 255.0;
    (((src - matte * (1.0 - a)) / a) * 255.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_ref(width: u32, height: u32, color_space: &str) -> ImageReference {
        ImageReference {
            page_number: 1,
            xobject_name: "Im0".to_string(),
            object_number: 4,
            generation_number: 0,
            width,
            height,
            bits_per_component: 8,
            color_space: color_space.to_string(),
            filter: Vec::new(),
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        }
    }

    fn test_pdf_from_objects(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len() + 1);
        offsets.push(0usize);
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
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
    fn combine_rgba_single_fully_transparent_pixel() {
        let main = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![200, 100, 50],
        };
        let mask = RawImage {
            width: 1,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![0],
        };
        let combined = SmaskLoader::combine_rgba(main, mask).unwrap();
        assert_eq!(combined.pixels, vec![200, 100, 50, 0]);
    }

    #[test]
    fn combine_rgba_preserves_pixel_order() {
        let main = RawImage {
            width: 3,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30, 40, 50, 60, 70, 80, 90],
        };
        let mask = RawImage {
            width: 3,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![100, 150, 200],
        };
        let out = SmaskLoader::combine_rgba(main, mask).unwrap();
        assert_eq!(
            out.pixels,
            vec![10, 20, 30, 100, 40, 50, 60, 150, 70, 80, 90, 200]
        );
    }

    #[test]
    fn combine_rgba_rejects_short_mask_buffer() {
        let main = RawImage {
            width: 2,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0, 0, 0, 50, 50, 50],
        };
        let mask = RawImage {
            width: 2,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![128],
        };
        let error = SmaskLoader::combine_rgba(main, mask)
            .expect_err("short SMask alpha buffer must not be padded");
        assert!(format!("{error}").contains("image SMask mask decoded 1 bytes, expected 2"));
    }

    #[test]
    fn combine_rgba_rejects_multichannel_mask_buffer() {
        let main = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0, 0, 0],
        };
        let mask = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![128, 128, 128],
        };
        let error = SmaskLoader::combine_rgba(main, mask)
            .expect_err("multi-channel SMask must not be sampled as grayscale");
        assert!(format!("{error}").contains("one alpha channel"));
    }

    #[test]
    fn combine_rgba_rejects_prealpha_main_image() {
        let main = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![10, 20, 30, 40],
        };
        let mask = RawImage {
            width: 1,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![128],
        };
        let error = SmaskLoader::combine_rgba(main, mask)
            .expect_err("pre-alpha main image must not silently drop alpha");
        assert!(format!("{error}").contains("pre-alpha main image"));
    }

    #[test]
    fn combine_rgba_with_matte_unblends_preblended_rgb() {
        let main = RawImage {
            width: 1,
            height: 1,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![255, 128, 128],
        };
        let mask = RawImage {
            width: 1,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![128],
        };

        let out = SmaskLoader::combine_rgba_with_matte(main, mask, Some([255, 255, 255])).unwrap();

        assert_eq!(out.pixels[0], 255);
        assert!(
            out.pixels[1] <= 2 && out.pixels[2] <= 2,
            "white-matte preblend should recover red, got {:?}",
            out.pixels
        );
        assert_eq!(out.pixels[3], 128);
    }

    #[test]
    fn smask_source_window_requires_matching_unfiltered_grayscale_mask() {
        let main_ref = image_ref(4, 3, "DeviceRGB");
        let main_raw = RawImage {
            width: 2,
            height: 2,
            channels: 3,
            bits_per_sample: 8,
            pixels: vec![0; 12],
        };
        let window = RawImageDecodeWindow {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        let smask_ref = image_ref(4, 3, "DeviceGray");

        assert!(smask_source_window_is_compatible(
            &main_ref, &main_raw, &smask_ref, window
        ));

        let mut filtered = smask_ref.clone();
        filtered.filter.push("FlateDecode".to_string());
        assert!(!smask_source_window_is_compatible(
            &main_ref, &main_raw, &filtered, window
        ));

        let rgb_mask = image_ref(4, 3, "DeviceRGB");
        assert!(!smask_source_window_is_compatible(
            &main_ref, &main_raw, &rgb_mask, window
        ));

        let mismatched_size = image_ref(5, 3, "DeviceGray");
        assert!(!smask_source_window_is_compatible(
            &main_ref,
            &main_raw,
            &mismatched_size,
            window
        ));
    }

    #[test]
    fn smask_loader_crops_source_window_for_ccitt_main_image() {
        let smask_pixels = [10u8, 20, 30, 40, 50, 60, 70, 80];
        let main_stream =
            b"<< /Type /XObject /Subtype /Image /Width 8 /Height 1 /ColorSpace /DeviceGray \
              /BitsPerComponent 1 /Filter /CCITTFaxDecode /SMask 5 0 R /Length 0 >>\n\
              stream\n\nendstream"
                .to_vec();
        let mut smask_stream = format!(
            "<< /Type /XObject /Subtype /Image /Width 8 /Height 1 /ColorSpace /DeviceGray \
             /BitsPerComponent 8 /Length {} >>\nstream\n",
            smask_pixels.len()
        )
        .into_bytes();
        smask_stream.extend_from_slice(&smask_pixels);
        smask_stream.extend_from_slice(b"\nendstream");
        let reader = PdfReader::from_bytes(test_pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 8 1] >>".to_vec(),
            main_stream,
            smask_stream,
        ]))
        .expect("open SMask source-window PDF");

        let main_image = ImageReference {
            page_number: 1,
            xobject_name: "Im0".to_string(),
            object_number: 4,
            generation_number: 0,
            width: 8,
            height: 1,
            bits_per_component: 1,
            color_space: "DeviceGray".to_string(),
            filter: vec!["CCITTFaxDecode".to_string()],
            is_inline: false,
            is_mask: false,
            is_smask: false,
            inline_data: None,
        };
        let main_raw = RawImage {
            width: 3,
            height: 1,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![200, 210, 220],
        };

        let combined = SmaskLoader::load_and_combine_with_limits_and_source_window(
            &main_image,
            main_raw,
            &reader,
            &DecodeLimits::default(),
            Some(RawImageDecodeWindow {
                x: 2,
                y: 0,
                width: 3,
                height: 1,
            }),
        )
        .expect("SMask source-window combine")
        .expect("SMask should combine");

        assert_eq!(combined.width, 3);
        assert_eq!(combined.height, 1);
        assert_eq!(combined.channels, 4);
        assert_eq!(
            combined.pixels,
            vec![200, 200, 200, 30, 210, 210, 210, 40, 220, 220, 220, 50]
        );
    }

    #[test]
    fn reduced_grayscale_smask_box_filters_without_full_size_output() {
        let reader = PdfReader::from_bytes(test_pdf_from_objects(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [] /Count 0 >>".to_vec(),
        ]))
        .expect("open reduction test PDF");
        let mut dict = PdfDictionary::empty();
        dict.insert("Width", PdfObject::Integer(4));
        dict.insert("Height", PdfObject::Integer(4));
        dict.insert("BitsPerComponent", PdfObject::Integer(8));
        dict.insert("ColorSpace", PdfObject::Name("DeviceGray".to_string()));
        dict.insert("Length", PdfObject::Integer(16));
        let image = ImageReference {
            page_number: 1,
            xobject_name: "Mask".to_string(),
            object_number: 0,
            generation_number: 0,
            width: 4,
            height: 4,
            bits_per_component: 8,
            color_space: "DeviceGray".to_string(),
            filter: Vec::new(),
            is_inline: false,
            is_mask: false,
            is_smask: true,
            inline_data: None,
        };
        let pixels = vec![
            0, 255, 0, 255, 255, 0, 255, 0, 0, 255, 0, 255, 255, 0, 255, 0,
        ];

        let reduced = decode_reduced_grayscale_smask(
            &dict,
            pixels,
            &image,
            2,
            2,
            &reader,
            &DecodeLimits::default(),
        )
        .expect("streaming SMask reduction");

        assert_eq!(reduced.width, 2);
        assert_eq!(reduced.height, 2);
        assert_eq!(reduced.pixels, vec![128, 128, 128, 128]);
    }

    #[test]
    fn smask_filter_names_rejects_malformed_filter_array() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "Filter".to_string(),
            PdfObject::Array(vec![
                PdfObject::Name("FlateDecode".to_string()),
                PdfObject::Integer(7),
            ]),
        );

        let error = smask_filter_names(&dict, "image SMask /Test")
            .expect_err("malformed SMask filter array must fail");
        assert!(format!("{error}").contains("/Filter array contains a non-name entry"));
    }

    #[test]
    fn smask_matte_rgb_uses_main_image_color_space() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "Matte".to_string(),
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ]),
        );

        let matte = smask_matte_rgb(&dict, "DeviceCMYK")
            .expect("valid matte")
            .expect("CMYK matte");

        assert_eq!(matte, [255, 255, 255]);
    }

    #[test]
    fn smask_matte_rgb_rejects_empty_matte_array() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Matte".to_string(), PdfObject::Array(Vec::new()));

        let error = smask_matte_rgb(&dict, "DeviceRGB").expect_err("empty matte must fail");
        assert!(format!("{error}").contains("image SMask /Matte is empty"));
    }

    #[test]
    fn smask_matte_rgb_rejects_unsupported_color_space() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "Matte".to_string(),
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
                PdfObject::Real(0.0),
            ]),
        );

        let error =
            smask_matte_rgb(&dict, "CalRGB").expect_err("unsupported matte color must fail");
        assert!(format!("{error}").contains("image SMask /Matte uses unsupported color space"));
    }

    #[test]
    fn smask_matte_rgb_rejects_incomplete_device_components() {
        let mut dict = PdfDictionary::empty();
        dict.insert(
            "Matte".to_string(),
            PdfObject::Array(vec![PdfObject::Real(0.1), PdfObject::Real(0.2)]),
        );

        let error =
            smask_matte_rgb(&dict, "DeviceRGB").expect_err("incomplete DeviceRGB matte must fail");
        assert!(format!("{error}").contains("image SMask /Matte uses unsupported color space"));
    }

    #[test]
    fn smask_color_space_name_accepts_name_and_array_forms() {
        let mut named = PdfDictionary::empty();
        named.insert("ColorSpace", PdfObject::Name("DeviceGray".to_string()));
        assert_eq!(
            smask_required_color_space_name(&named, "image SMask /Test")
                .expect("named color space"),
            "DeviceGray"
        );

        let mut array = PdfDictionary::empty();
        array.insert(
            "CS",
            PdfObject::Array(vec![PdfObject::Name("ICCBased".to_string())]),
        );
        assert_eq!(
            smask_required_color_space_name(&array, "image SMask /Test")
                .expect("array color space"),
            "ICCBased"
        );
    }

    #[test]
    fn smask_required_positive_u32_rejects_missing_zero_and_negative_dimensions() {
        let mut dict = PdfDictionary::empty();
        dict.insert("Width", PdfObject::Integer(7));
        assert_eq!(
            smask_required_positive_u32(&dict, "Width", "W", "image SMask /Test")
                .expect("valid width"),
            7
        );

        dict.insert("Width", PdfObject::Integer(0));
        let error = smask_required_positive_u32(&dict, "Width", "W", "image SMask /Test")
            .expect_err("zero width must fail");
        assert!(format!("{error}").contains("/Width must be a positive integer"));

        dict.insert("Width", PdfObject::Integer(-1));
        let error = smask_required_positive_u32(&dict, "Width", "W", "image SMask /Test")
            .expect_err("negative width must fail");
        assert!(format!("{error}").contains("/Width must be a positive integer"));

        dict.remove("Width");
        let error = smask_required_positive_u32(&dict, "Width", "W", "image SMask /Test")
            .expect_err("missing width must fail");
        assert!(format!("{error}").contains("missing /Width"));
    }

    #[test]
    fn smask_bits_per_component_requires_declared_supported_value() {
        let mut dict = PdfDictionary::empty();
        let error = smask_bits_per_component(&dict, "image SMask /Test")
            .expect_err("missing bpc must fail");
        assert!(format!("{error}").contains("missing /BitsPerComponent"));

        dict.insert("BitsPerComponent", PdfObject::Integer(8));
        assert_eq!(
            smask_bits_per_component(&dict, "image SMask /Test").expect("valid bpc"),
            8
        );

        dict.insert("BitsPerComponent", PdfObject::Integer(20));
        let error = smask_bits_per_component(&dict, "image SMask /Test")
            .expect_err("unsupported bpc must fail");
        assert!(format!("{error}").contains("must be one of 1, 2, 4, 8, or 16"));
    }

    #[test]
    fn smask_color_space_requires_declared_name_or_array() {
        let dict = PdfDictionary::empty();
        let error = smask_required_color_space_name(&dict, "image SMask /Test")
            .expect_err("missing ColorSpace must fail");
        assert!(format!("{error}").contains("missing /ColorSpace"));

        let mut malformed = PdfDictionary::empty();
        malformed.insert("ColorSpace", PdfObject::Integer(1));
        let error = smask_required_color_space_name(&malformed, "image SMask /Test")
            .expect_err("malformed ColorSpace must fail");
        assert!(format!("{error}").contains("expected Name or Array"));
    }
}
