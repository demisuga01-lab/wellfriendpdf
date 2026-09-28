//! Shared Indexed image/paint interpretation with distinct source and rendering
//! base domains. Paint retains floating-point colours; images expand a palette.
use super::{decoder::ColorSpaceConverter, sample_decode::DecodeMap};
use crate::error::{Result, WellfriendError};
use crate::render::{
    cmm,
    colorspace::{self, NamedColor},
    default_colorspace,
};
use crate::{PdfDictionary, PdfObject, PdfReader};

fn invalid(reason: impl Into<String>) -> WellfriendError {
    WellfriendError::MalformedPdf(reason.into())
}

fn indexed(object: &PdfObject, reader: &PdfReader) -> Result<Vec<PdfObject>> {
    let object = reader.resolve(object.clone())?;
    let items = object
        .as_array()
        .ok_or_else(|| invalid("Indexed image ColorSpace is not an array"))?;
    if items.len() != 4 {
        return Err(invalid(format!(
            "Indexed image ColorSpace has {} entries, expected 4",
            items.len()
        )));
    }
    if !matches!(items[0].as_name(), Some("Indexed" | "I")) {
        return Err(invalid("image source is not an Indexed colour space"));
    }
    Ok(items.to_vec())
}

fn hival(object: &PdfObject, reader: &PdfReader) -> Result<usize> {
    let object = reader.resolve(object.clone())?;
    let value = object
        .as_integer()
        .ok_or_else(|| invalid("Indexed image ColorSpace hival is not an integer"))?;
    if !(0..=255).contains(&value) {
        return Err(invalid("Indexed image ColorSpace hival is outside 0..=255"));
    }
    Ok(value as usize)
}

fn base(object: &PdfObject, reader: &PdfReader) -> Result<PdfObject> {
    let object = reader.resolve(object.clone())?;
    let object = default_colorspace::canonical_inline(&object).map_err(invalid)?;
    let object = match &object {
        PdfObject::Array(items)
            if items.len() == 1
                && matches!(
                    items[0].as_name(),
                    Some("DeviceGray" | "DeviceRGB" | "DeviceCMYK")
                ) =>
        {
            items[0].clone()
        }
        _ => object,
    };
    if matches!(
        default_colorspace::family(&object),
        Some("Indexed" | "Pattern")
    ) {
        return Err(invalid("Indexed base cannot be Indexed or Pattern"));
    }
    Ok(object)
}

struct PreparedPalette {
    target_base: PdfObject,
    source_base: PdfObject,
    target_dict: PdfDictionary,
    ranges: Vec<(f64, f64)>,
    lookup: Vec<u8>,
    channels: usize,
    maximum: usize,
}

impl PreparedPalette {
    fn new(
        object: &PdfObject,
        source_space: Option<&PdfObject>,
        reader: &PdfReader,
    ) -> Result<Self> {
        let bound = indexed(object, reader)?;
        let source = indexed(source_space.unwrap_or(object), reader)?;
        let maximum = hival(&bound[2], reader)?;
        if hival(&source[2], reader)? != maximum {
            return Err(invalid("Indexed source/replacement hival mismatch"));
        }
        let target_base = base(&bound[1], reader)?;
        let source_base = base(&source[1], reader)?;
        let target_family = default_colorspace::family(&target_base)
            .ok_or_else(|| invalid("Indexed base has no family"))?;
        let channels =
            ColorSpaceConverter::indexed_base_channel_count(target_family, &target_base, reader)?;
        let ranges = default_colorspace::component_ranges(&source_base, reader).map_err(invalid)?;
        if ranges.len() != channels {
            return Err(invalid(
                "Indexed source/replacement base component mismatch",
            ));
        }
        let lookup = colorspace::indexed_lookup_bytes(&bound[3], reader).map_err(invalid)?;
        let expected = (maximum + 1) * channels;
        if lookup.len() != expected {
            return Err(invalid(format!(
                "Indexed image ColorSpace lookup table has {} bytes, expected {expected}",
                lookup.len()
            )));
        }
        let mut target_dict = PdfDictionary::empty();
        target_dict.insert("ColorSpace", target_base.clone());
        match target_family {
            "CalGray" => {
                cmm::try_cal_gray_params_from_image_dict(&target_dict, Some(reader))
                    .map_err(invalid)?;
            }
            "CalRGB" => {
                cmm::try_cal_rgb_params_from_image_dict(&target_dict, Some(reader))
                    .map_err(invalid)?;
            }
            "Lab" => {
                cmm::try_lab_params_from_image_dict(&target_dict, Some(reader)).map_err(invalid)?;
            }
            _ => {}
        }
        Ok(Self {
            target_base,
            source_base,
            target_dict,
            ranges,
            lookup,
            channels,
            maximum,
        })
    }

    fn values(&self, index: usize, out: &mut [f64]) -> Result<()> {
        if index > self.maximum || out.len() != self.channels {
            return Err(invalid("Indexed palette component/index range mismatch"));
        }
        for (channel, value) in out.iter_mut().enumerate() {
            let unit = f64::from(self.lookup[index * self.channels + channel]) / 255.0;
            let (lo, hi) = self.ranges[channel];
            *value = (1.0 - unit) * lo + unit * hi;
            if !value.is_finite() {
                return Err(invalid("Indexed palette interpolation is not finite"));
            }
        }
        Ok(())
    }

    fn colour(
        &self,
        index: usize,
        alpha: f32,
        reader: &PdfReader,
        options: cmm::ColorTransformOptions,
        resources: crate::render::function::FunctionResources<'_>,
    ) -> Result<NamedColor> {
        let mut components = vec![0.0; self.channels];
        self.values(index, &mut components)?;
        Ok(colorspace::resolve_named_color_with_resources(
            &self.target_base,
            Some(&self.source_base),
            &components,
            alpha,
            reader,
            options,
            resources,
        ))
    }
}

/// Paint converts only the selected entry and preserves floating-point output
/// and no-paint semantics; images use the same metadata and component mapping.
#[cfg(test)]
pub(crate) fn resolve_color(
    space: &PdfObject,
    source_space: Option<&PdfObject>,
    index: f64,
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
) -> Result<NamedColor> {
    resolve_color_with_resources(
        space,
        source_space,
        index,
        alpha,
        reader,
        options,
        crate::render::function::FunctionResources::default(),
    )
}

pub(crate) fn resolve_color_with_resources(
    space: &PdfObject,
    source_space: Option<&PdfObject>,
    index: f64,
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
    resources: crate::render::function::FunctionResources<'_>,
) -> Result<NamedColor> {
    crate::cancel::check_current_cancel("Indexed paint conversion")?;
    if !index.is_finite() || !alpha.is_finite() {
        return Err(invalid("Indexed paint index/alpha is not finite"));
    }
    let palette = PreparedPalette::new(space, source_space, reader)?;
    // ISO 32000 defines Indexed selection by rounding to the nearest integer
    // and clipping to 0..hival. Callers that require a continuous colour
    // function (notably shadings) must reject fractional results before this
    // discrete palette conversion.
    let selected = index.clamp(0.0, palette.maximum as f64).round() as usize;
    palette.colour(selected, alpha, reader, options, resources)
}

fn palette(
    dict: &PdfDictionary,
    reader: &PdfReader,
    options: super::decoder::ImageColorOptions<'_>,
    source_space: Option<&PdfObject>,
) -> Result<(Vec<u8>, u8, usize)> {
    let object = dict
        .get("ColorSpace")
        .or_else(|| dict.get("CS"))
        .ok_or_else(|| invalid("Indexed image has no ColorSpace"))?;
    let prepared = PreparedPalette::new(object, source_space, reader)?;
    let family = default_colorspace::family(&prepared.target_base)
        .ok_or_else(|| invalid("Indexed base has no family"))?;
    if family == "ICCBased" {
        let (pixels, channels) = crate::render::icc_conversion::convert_image_values(
            prepared.maximum + 1,
            prepared.channels,
            &prepared.target_dict,
            reader,
            options,
            Some(&prepared.source_base),
            |index, out| prepared.values(index, out),
        )?;
        return Ok((pixels, channels, prepared.maximum));
    }
    let channels = match family {
        "DeviceGray" => 1,
        "Separation" | "DeviceN" => 4,
        _ => 3,
    };
    let mut result = Vec::with_capacity((prepared.maximum + 1) * channels as usize);
    for index in 0..=prepared.maximum {
        crate::cancel::check_current_cancel("Indexed palette conversion")?;
        let rgba = match prepared.colour(index, 1.0, reader, options.options, options.functions)? {
            NamedColor::Color(color) => color.to_pixel_color(),
            NamedColor::NoPaint if channels == 4 => [0; 4],
            NamedColor::Invalid(reason) => {
                return Err(invalid(format!("Indexed palette conversion: {reason}")))
            }
            _ => {
                return Err(WellfriendError::UnsupportedFeature(
                    "Indexed base conversion is unavailable".into(),
                ))
            }
        };
        result.extend_from_slice(&rgba[..channels as usize]);
    }
    Ok((result, channels, prepared.maximum))
}

pub(crate) fn convert_indices<'a>(
    pixel_count: usize,
    dict: &PdfDictionary,
    reader: &PdfReader,
    options: impl Into<super::decoder::ImageColorOptions<'a>>,
    source_space: Option<&PdfObject>,
    read_index: impl Fn(usize) -> Result<f64>,
) -> Result<(Vec<u8>, u8)> {
    crate::cancel::check_current_cancel("Indexed image conversion")?;
    let (palette, channels, maximum) = palette(dict, reader, options.into(), source_space)?;
    if !matches!(channels, 1 | 3 | 4) {
        return Err(invalid("Indexed converted palette has invalid channels"));
    }
    super::decoder::ensure_indexed_palette_len(palette.len(), maximum + 1, channels as usize)?;
    let length = pixel_count
        .checked_mul(channels as usize)
        .ok_or_else(|| invalid("Indexed output size overflow"))?;
    if length as u64 > crate::filters::DecodeLimits::default().max_image_decoded_bytes {
        return Err(WellfriendError::UnsupportedFeature(
            "Indexed output exceeds byte budget".into(),
        ));
    }
    let mut output = Vec::with_capacity(length);
    for pixel in 0..pixel_count {
        if pixel % 4096 == 0 {
            crate::cancel::check_current_cancel("Indexed pixel conversion")?;
        }
        let value = read_index(pixel)?;
        if !value.is_finite() {
            return Err(invalid("Indexed pixel is not finite"));
        }
        let index = value.clamp(0.0, maximum as f64).round() as usize;
        let start = index * channels as usize;
        output.extend_from_slice(&palette[start..start + channels as usize]);
    }
    crate::cancel::check_current_cancel("Indexed image result")?;
    Ok((output, channels))
}

/// Existing byte-converter callers already expanded packed samples to 0..255.
/// Recover their original integer levels, then use the same Decode/palette path.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_normalized<'a>(
    pixels: &[u8],
    bits: u8,
    dict: &PdfDictionary,
    reader: &PdfReader,
    width: u32,
    height: u32,
    options: impl Into<super::decoder::ImageColorOptions<'a>>,
) -> Result<(Vec<u8>, u8)> {
    if !matches!(bits, 1 | 2 | 4 | 8) {
        return Err(invalid("Indexed image bit depth must be 1, 2, 4 or 8"));
    }
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| invalid("Indexed dimensions overflow"))?;
    if pixels.len() != count {
        return Err(invalid(
            "Indexed image sample count differs from dimensions",
        ));
    }
    let maximum = ((1u32 << bits) - 1) as usize;
    let map = DecodeMap::from_dictionary(dict, &[(0.0, maximum as f64)], Some(reader))?;
    convert_indices(count, dict, reader, options, None, |index| {
        let original = (usize::from(pixels[index]) * maximum + 127) / 255;
        map.component_value(0, original as f64 / maximum as f64)
    })
}

#[cfg(test)]
#[path = "indexed_samples_tests.rs"]
mod tests;
