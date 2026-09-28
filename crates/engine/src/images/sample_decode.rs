//! Row-aware integer samples and unquantized PDF Decode components.
//! The caller supplies defaults from the source (not a replacement) space.
use crate::error::{Result, WellfriendError};
use crate::{PdfDictionary, PdfObject, PdfReader};

fn invalid(reason: impl Into<String>) -> WellfriendError {
    WellfriendError::MalformedPdf(reason.into())
}

pub(crate) struct PackedSamples<'a> {
    data: &'a [u8],
    width: usize,
    pixels: usize,
    channels: usize,
    bits: usize,
    row_bytes: usize,
    max_sample: f64,
}

impl<'a> PackedSamples<'a> {
    pub(crate) fn new(
        data: &'a [u8],
        width: u32,
        height: u32,
        channels: usize,
        bits: u8,
    ) -> Result<Self> {
        if !matches!(bits, 1 | 2 | 4 | 8 | 16) || !(1..=16).contains(&channels) {
            return Err(invalid("invalid packed image bit depth or component count"));
        }
        let width = width as usize;
        let row_bits = width
            .checked_mul(channels)
            .and_then(|n| n.checked_mul(bits as usize))
            .ok_or_else(|| invalid("packed image row size overflow"))?;
        let row_bytes = row_bits
            .checked_add(7)
            .ok_or_else(|| invalid("packed image row padding overflow"))?
            / 8;
        let expected = row_bytes
            .checked_mul(height as usize)
            .ok_or_else(|| invalid("packed image byte size overflow"))?;
        if data.len() != expected {
            return Err(invalid(format!(
                "packed image has {} bytes, expected {expected}",
                data.len()
            )));
        }
        let pixels = width
            .checked_mul(height as usize)
            .ok_or_else(|| invalid("packed image pixel count overflow"))?;
        Ok(Self {
            data,
            width,
            pixels,
            channels,
            bits: bits as usize,
            row_bytes,
            max_sample: ((1u32 << bits) - 1) as f64,
        })
    }

    pub(crate) fn pixel_count(&self) -> usize {
        self.pixels
    }

    pub(crate) fn decode_pixel(
        &self,
        index: usize,
        map: &DecodeMap,
        out: &mut [f64],
    ) -> Result<()> {
        if index >= self.pixels || out.len() != self.channels || map.0.len() != self.channels {
            return Err(invalid("packed image pixel/component range mismatch"));
        }
        let row = index / self.width;
        let column = index % self.width;
        for (component, value) in out.iter_mut().enumerate() {
            let bit = (column * self.channels + component) * self.bits;
            let offset = row * self.row_bytes + bit / 8;
            let sample = if self.bits == 16 {
                u16::from_be_bytes([self.data[offset], self.data[offset + 1]])
            } else {
                let shift = 8 - self.bits - (bit % 8);
                (u16::from(self.data[offset]) >> shift) & ((1u16 << self.bits) - 1)
            };
            let unit = f64::from(sample) / self.max_sample;
            *value = map.component_value(component, unit)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DecodeMap(Vec<(f64, f64)>);

impl DecodeMap {
    fn has_explicit(dict: &PdfDictionary, reader: &PdfReader) -> Result<bool> {
        dict.get("Decode")
            .or_else(|| dict.get("D"))
            .map(|value| {
                reader
                    .resolve(value.clone())
                    .map(|value| !matches!(value, PdfObject::Null))
            })
            .transpose()
            .map(|value| value.unwrap_or(false))
    }
    pub(super) fn component_value(&self, component: usize, unit: f64) -> Result<f64> {
        let &(lo, hi) = self
            .0
            .get(component)
            .ok_or_else(|| invalid("image Decode component is absent"))?;
        if !unit.is_finite() || !(0.0..=1.0).contains(&unit) {
            return Err(invalid("image sample fraction is outside unit range"));
        }
        // Avoid overflow in hi-lo for opposite finite extrema.
        let value = (1.0 - unit) * lo + unit * hi;
        if !value.is_finite() {
            return Err(invalid("image Decode interpolation is not finite"));
        }
        Ok(value)
    }
    pub(crate) fn from_dictionary(
        dict: &PdfDictionary,
        defaults: &[(f64, f64)],
        reader: Option<&PdfReader>,
    ) -> Result<Self> {
        if defaults.is_empty()
            || defaults.len() > 16
            || defaults
                .iter()
                .any(|(a, b)| !a.is_finite() || !b.is_finite())
        {
            return Err(invalid("invalid image Decode defaults"));
        }
        let resolve = |object: &PdfObject| -> Result<PdfObject> {
            match reader {
                Some(reader) => reader.resolve(object.clone()),
                None if matches!(object, PdfObject::Reference { .. }) => {
                    Err(invalid("indirect image Decode requires a reader"))
                }
                None => Ok(object.clone()),
            }
        };
        let Some(object) = dict.get("Decode").or_else(|| dict.get("D")) else {
            return Ok(Self(defaults.to_vec()));
        };
        let object = resolve(object)?;
        if matches!(object, PdfObject::Null) {
            return Ok(Self(defaults.to_vec()));
        }
        let items = object
            .as_array()
            .ok_or_else(|| invalid("image Decode is not an array"))?;
        if items.len() != defaults.len() * 2 {
            return Err(invalid("image Decode component count mismatch"));
        }
        let mut pairs = Vec::with_capacity(defaults.len());
        for pair in items.chunks_exact(2) {
            let lo = resolve(&pair[0])?
                .as_number()
                .filter(|v| v.is_finite())
                .ok_or_else(|| invalid("image Decode endpoint is not finite numeric"))?;
            let hi = resolve(&pair[1])?
                .as_number()
                .filter(|v| v.is_finite())
                .ok_or_else(|| invalid("image Decode endpoint is not finite numeric"))?;
            // Descending and constant mappings are both valid PDF Decode maps.
            pairs.push((lo, hi));
        }
        Ok(Self(pairs))
    }
}

/// Preserve sample-domain values for Lab and ICC conversion. Source-space
/// defaults are distinct from a device-space replacement's component ranges.
#[allow(clippy::too_many_arguments)]
pub(crate) fn convert_component_image<'a>(
    data: &[u8],
    width: u32,
    height: u32,
    channels: usize,
    bits: u8,
    family: &str,
    dict: &PdfDictionary,
    reader: Option<&PdfReader>,
    options: impl Into<super::decoder::ImageColorOptions<'a>>,
    source_space: Option<&PdfObject>,
) -> Result<super::decoder::RawImage> {
    let options = options.into();
    use crate::render::cmm;
    crate::cancel::check_current_cancel("image component-domain decoding")?;
    let samples = PackedSamples::new(data, width, height, channels, bits)?;
    let (pixels, output_channels) = match family {
        "Indexed" => {
            if channels != 1 || !matches!(bits, 1 | 2 | 4 | 8) {
                return Err(invalid(
                    "Indexed image requires one component and 1, 2, 4 or 8 bits",
                ));
            }
            let reader = reader.ok_or_else(|| invalid("Indexed image requires a reader"))?;
            let map = DecodeMap::from_dictionary(
                dict,
                &[(0.0, ((1u32 << bits) - 1) as f64)],
                Some(reader),
            )?;
            super::indexed_samples::convert_indices(
                samples.pixel_count(),
                dict,
                reader,
                options,
                source_space,
                |index| {
                    let mut value = [0.0];
                    samples.decode_pixel(index, &map, &mut value)?;
                    Ok(value[0])
                },
            )?
        }
        "ICCBased" => {
            let reader = reader.ok_or_else(|| invalid("ICC image decoding requires a reader"))?;
            let defaults = if DecodeMap::has_explicit(dict, reader)? {
                // Only the component count is used when Decode is explicit.
                vec![(0.0, 1.0); channels]
            } else {
                source_icc_decode_defaults(
                    source_space.or_else(|| dict.get("ColorSpace").or_else(|| dict.get("CS"))),
                    channels,
                    reader,
                )?
            };
            let map = DecodeMap::from_dictionary(dict, &defaults, Some(reader))?;
            crate::render::icc_conversion::convert_image_values(
                samples.pixel_count(),
                channels,
                dict,
                reader,
                options,
                source_space.or_else(|| dict.get("ColorSpace").or_else(|| dict.get("CS"))),
                |index, out| samples.decode_pixel(index, &map, out),
            )?
        }
        "Lab" => {
            if channels != 3 {
                return Err(invalid("Lab image requires three components"));
            }
            let params = cmm::try_lab_params_from_image_dict(dict, reader).map_err(invalid)?;
            let ranges = [
                (0.0, 100.0),
                (params.range[0] as f64, params.range[1] as f64),
                (params.range[2] as f64, params.range[3] as f64),
            ];
            let map = DecodeMap::from_dictionary(dict, &ranges, reader)?;
            let length = samples
                .pixel_count()
                .checked_mul(3)
                .ok_or_else(|| invalid("Lab image output size overflow"))?;
            if length as u64 > crate::filters::DecodeLimits::default().max_image_decoded_bytes {
                return Err(WellfriendError::UnsupportedFeature(
                    "Lab image output exceeds byte budget".into(),
                ));
            }
            let mut output = Vec::with_capacity(length);
            let mut values = [0.0; 3];
            for index in 0..samples.pixel_count() {
                if index % 4096 == 0 {
                    crate::cancel::check_current_cancel("Lab image conversion")?;
                }
                samples.decode_pixel(index, &map, &mut values)?;
                for (value, (lo, hi)) in values.iter_mut().zip(ranges) {
                    *value = value.clamp(lo, hi);
                }
                let rgb =
                    cmm::lab_to_srgb(values[0] as f32, values[1] as f32, values[2] as f32, params);
                output.extend(rgb.map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8));
            }
            (output, 3)
        }
        _ => return Err(invalid("unsupported component-domain image family")),
    };
    crate::cancel::check_current_cancel("image component-domain result")?;
    Ok(super::decoder::RawImage {
        width,
        height,
        channels: output_channels,
        bits_per_sample: 8,
        pixels,
    })
}

fn source_icc_decode_defaults(
    source: Option<&PdfObject>,
    channels: usize,
    reader: &PdfReader,
) -> Result<Vec<(f64, f64)>> {
    // Standalone callers treat their supplied colour graph as the original;
    // remapping callers must retain a separate resolved source graph.
    let source = source.ok_or_else(|| invalid("ICC image source domain is unavailable"))?;
    let source = reader.resolve(source.clone())?;
    let source = crate::render::default_colorspace::canonical_inline(&source).map_err(invalid)?;
    if matches!(&source,PdfObject::Name(name) if !matches!(name.as_str(),"DeviceGray"|"DeviceRGB"|"DeviceCMYK"))
    {
        return Err(invalid(
            "ICC image source colour alias requires its selecting resource scope",
        ));
    }
    if matches!(
        crate::render::default_colorspace::family(&source),
        Some("Indexed" | "Pattern")
    ) {
        return Err(invalid(
            "Indexed/Pattern source cannot be interpreted as ICC components",
        ));
    }
    let ranges =
        crate::render::default_colorspace::component_ranges(&source, reader).map_err(invalid)?;
    if ranges.len() != channels {
        return Err(invalid("source ICC image component count mismatch"));
    }
    Ok(ranges)
}

#[cfg(test)]
#[path = "sample_decode_tests.rs"]
mod tests;
