//! One metadata-validated ICC/Alternate route for paint and image samples.
//! Source values are clipped, never rescaled when handed to an alternate.
use super::{
    cmm,
    colorspace::{self, NamedColor},
    default_colorspace,
};
use crate::engine::PageResources;
use crate::error::{Result, WellfriendError};
use crate::filters::{decode_stream_lossless_with_limits, DecodeLimits, StreamDecodeStatus};
use crate::{PdfDictionary, PdfObject, PdfReader};
use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AlternateMetrics {
    pub native_conversions: u64,
    pub policy_alternates: u64,
    pub unavailable_profile_alternates: u64,
    pub device_alternates: u64,
    pub rejected_conversions: u64,
}
thread_local! {
    static METRICS: Cell<AlternateMetrics> = Cell::new(AlternateMetrics::default());
    static ACTIVE_DEPTH: Cell<usize> = const {Cell::new(0)};
}
pub(crate) fn metrics() -> AlternateMetrics {
    METRICS.with(Cell::get)
}
fn count(update: impl FnOnce(&mut AlternateMetrics)) {
    METRICS.with(|cell| {
        let mut metrics = cell.get();
        update(&mut metrics);
        cell.set(metrics);
    });
}

struct ConversionGuard;
impl ConversionGuard {
    fn enter() -> Result<Self> {
        crate::cancel::check_current_cancel("ICC conversion")?;
        ACTIVE_DEPTH.with(|depth| {
            if depth.get() >= 16 {
                return Err(invalid("ICC alternate conversion nesting exceeds 16"));
            }
            depth.set(depth.get() + 1);
            Ok(Self)
        })
    }
}
impl Drop for ConversionGuard {
    fn drop(&mut self) {
        ACTIVE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}
fn invalid(reason: impl Into<String>) -> WellfriendError {
    WellfriendError::MalformedPdf(reason.into())
}

struct PreparedIcc {
    profile: Vec<u8>,
    ranges: Vec<(f64, f64)>,
    alternate: PdfObject,
    source_alternate: Option<PdfObject>,
    alternate_ranges: Vec<(f64, f64)>,
    device_alternate: bool,
    options: cmm::ColorTransformOptions,
}
impl PreparedIcc {
    fn new(
        space: &PdfObject,
        reader: &PdfReader,
        options: cmm::ColorTransformOptions,
        source_space: Option<&PdfObject>,
    ) -> Result<Self> {
        // A page renderer supplies an already scope-bound graph. Standalone
        // conversion still resolves references and rejects cycles/invalid
        // Alternate graphs using the same bounded resolver.
        let original = reader.resolve(space.clone())?;
        let bound = default_colorspace::bind(&original, &PageResources::default(), reader)
            .map_err(invalid)?;
        if default_colorspace::family(&bound) != Some("ICCBased") {
            return Err(invalid("expected ICCBased colour space"));
        }
        let profile = bound
            .as_array()
            .and_then(|a| a.get(1))
            .ok_or_else(|| invalid("ICCBased profile missing"))?;
        let PdfObject::Stream { dict, .. } = profile else {
            return Err(invalid("ICCBased profile is not a stream"));
        };
        let ranges = default_colorspace::icc_component_ranges(dict, reader).map_err(invalid)?;
        let alternate = dict
            .get("Alternate")
            .cloned()
            .ok_or_else(|| invalid("bound ICC alternate missing"))?;
        let device_alternate = matches!(
            default_colorspace::family(&alternate),
            Some("DeviceGray" | "DeviceRGB" | "DeviceCMYK")
        );
        let alternate_ranges =
            default_colorspace::component_ranges(&alternate, reader).map_err(invalid)?;
        if ranges.len() != alternate_ranges.len() {
            return Err(invalid("ICC Alternate component count mismatch"));
        }
        let source_alternate = if let Some(source) = source_space {
            let source = reader.resolve(source.clone())?;
            if default_colorspace::family(&source) == Some("ICCBased") {
                let source =
                    default_colorspace::bind_source(&source, &PageResources::default(), reader)
                        .map_err(invalid)?;
                let source_profile = source
                    .as_array()
                    .and_then(|a| a.get(1))
                    .ok_or_else(|| invalid("original ICC profile missing"))?;
                let PdfObject::Stream { dict, .. } = source_profile else {
                    return Err(invalid("original ICC profile is not a stream"));
                };
                let source_ranges =
                    default_colorspace::icc_component_ranges(dict, reader).map_err(invalid)?;
                if source_ranges.len() != ranges.len() {
                    return Err(invalid("original ICC component count mismatch"));
                }
                Some(
                    dict.get("Alternate")
                        .cloned()
                        .ok_or_else(|| invalid("original ICC alternate missing"))?,
                )
            } else {
                // A Default* replacement has no corresponding source alternate.
                None
            }
        } else {
            None
        };
        let profile = if options.backend == cmm::ColorTransformBackend::DeterministicFallback {
            Vec::new()
        } else {
            let limits = DecodeLimits {
                max_decoded_bytes_per_stream: cmm::DEFAULT_MAX_ICC_PROFILE_BYTES as u64,
                max_decoded_bytes_per_document: cmm::DEFAULT_MAX_ICC_PROFILE_BYTES as u64,
                ..DecodeLimits::default()
            };
            let decoded = decode_stream_lossless_with_limits(profile, reader, &limits)?;
            if !matches!(decoded.status, StreamDecodeStatus::Complete) {
                return Err(invalid("ICC profile stream contains an image codec filter"));
            }
            decoded.data
        };
        Ok(Self {
            profile,
            ranges,
            alternate,
            source_alternate,
            alternate_ranges,
            device_alternate,
            options,
        })
    }
    fn clipped(&self, components: &[f64]) -> Result<Vec<f64>> {
        if components.len() != self.ranges.len() || components.iter().any(|v| !v.is_finite()) {
            return Err(invalid("ICC components must be finite and match N"));
        }
        Ok(components
            .iter()
            .zip(&self.ranges)
            .map(|(v, (lo, hi))| v.clamp(*lo, *hi))
            .collect())
    }
    fn native(&self, components: &[u8]) -> Result<Option<(Vec<u8>, u8)>> {
        if self.options.backend == cmm::ColorTransformBackend::DeterministicFallback {
            return Ok(None);
        }
        crate::cancel::check_current_cancel("ICC profile transform")?;
        let output = cmm::transform_icc_profile_samples(
            &self.profile,
            self.ranges.len() as u8,
            components,
            self.options,
        );
        // A native codec call is not forcibly interruptible, but a cancelled
        // conversion must not publish its result when control returns.
        crate::cancel::check_current_cancel("ICC profile transform result")?;
        Ok(output)
    }
    fn note_alternate(&self) {
        count(|m| {
            if self.options.backend == cmm::ColorTransformBackend::DeterministicFallback {
                m.policy_alternates = m.policy_alternates.saturating_add(1);
            } else {
                m.unavailable_profile_alternates =
                    m.unavailable_profile_alternates.saturating_add(1);
            }
            if self.device_alternate {
                m.device_alternates = m.device_alternates.saturating_add(1);
            }
        });
    }
    fn alternate_color_with_resources(
        &self,
        values: &[f64],
        alpha: f32,
        reader: &PdfReader,
        resources: super::function::FunctionResources<'_>,
    ) -> Result<NamedColor> {
        let values = values
            .iter()
            .zip(&self.alternate_ranges)
            .map(|(value, (lo, hi))| value.clamp(*lo, *hi))
            .collect::<Vec<_>>();
        match colorspace::resolve_named_color_with_resources(
            &self.alternate,
            self.source_alternate.as_ref(),
            &values,
            alpha,
            reader,
            self.options,
            resources,
        ) {
            color @ (NamedColor::Color(_) | NamedColor::NoPaint) => Ok(color),
            NamedColor::Invalid(reason) => {
                Err(invalid(format!("ICC Alternate rejected: {reason}")))
            }
            NamedColor::Unhandled => Err(WellfriendError::UnsupportedFeature(
                "ICC Alternate conversion is unavailable".into(),
            )),
        }
    }
}

fn byte(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
pub(crate) fn resolve_color(
    space: &PdfObject,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
) -> Result<NamedColor> {
    resolve_color_with_source(space, None, components, alpha, reader, options)
}

#[cfg(test)]
pub(crate) fn resolve_color_with_source(
    space: &PdfObject,
    source_space: Option<&PdfObject>,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
) -> Result<NamedColor> {
    resolve_color_with_resources(
        space,
        source_space,
        components,
        alpha,
        reader,
        options,
        super::function::FunctionResources::default(),
    )
}

pub(crate) fn resolve_color_with_resources(
    space: &PdfObject,
    source_space: Option<&PdfObject>,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
    resources: super::function::FunctionResources<'_>,
) -> Result<NamedColor> {
    let result = (|| {
        let _guard = ConversionGuard::enter()?;
        if !alpha.is_finite() {
            return Err(invalid("ICC paint alpha is not finite"));
        }
        let prepared = PreparedIcc::new(space, reader, options, source_space)?;
        let values = prepared.clipped(components)?;
        let bytes = values.iter().copied().map(byte).collect::<Vec<_>>();
        if let Some((pixels, channels)) = prepared.native(&bytes)? {
            if channels != 3 || pixels.len() != 3 {
                return Err(invalid(
                    "ICC native conversion returned invalid pixel count",
                ));
            }
            count(|m| m.native_conversions = m.native_conversions.saturating_add(1));
            return Ok(NamedColor::Color(super::color::RenderColor::new(
                pixels[0] as f32 / 255.0,
                pixels[1] as f32 / 255.0,
                pixels[2] as f32 / 255.0,
                alpha,
            )));
        }
        let color = prepared.alternate_color_with_resources(&values, alpha, reader, resources)?;
        prepared.note_alternate();
        Ok(color)
    })();
    if result.is_err() {
        count(|m| m.rejected_conversions = m.rejected_conversions.saturating_add(1));
    }
    result
}

/// Input is the decoder's existing normalized eight-bit component buffer.
/// Decode-domain provenance and higher precision are separate from this route.
pub(crate) fn convert_image<'a>(
    pixels: &[u8],
    dict: &PdfDictionary,
    reader: &PdfReader,
    options: impl Into<crate::images::decoder::ImageColorOptions<'a>>,
) -> Result<(Vec<u8>, u8)> {
    let channels = match cmm::icc_channel_count(dict, reader).map(usize::from) {
        Some(channels @ (1 | 3 | 4)) if pixels.len().is_multiple_of(channels) => channels,
        _ => {
            count(|m| m.rejected_conversions = m.rejected_conversions.saturating_add(1));
            return Err(invalid("ICC image sample count does not match valid N"));
        }
    };
    convert_image_values(
        pixels.len() / channels,
        channels,
        dict,
        reader,
        options,
        None,
        |index, out| {
            for (value, sample) in out.iter_mut().zip(&pixels[index * channels..][..channels]) {
                *value = f64::from(*sample) / 255.0;
            }
            Ok(())
        },
    )
}

/// Decode-domain values are supplied lazily, retaining signed components and
/// sixteen-bit source precision until conversion. Existing CMM backends still
/// accept eight-bit input; alternate conversion receives the original f64s.
pub(crate) fn convert_image_values<'a>(
    pixel_count: usize,
    channels: usize,
    dict: &PdfDictionary,
    reader: &PdfReader,
    options: impl Into<crate::images::decoder::ImageColorOptions<'a>>,
    source_space: Option<&PdfObject>,
    read_pixel: impl Fn(usize, &mut [f64]) -> Result<()>,
) -> Result<(Vec<u8>, u8)> {
    let policy = options.into();
    let options = policy.options;
    let result = (|| {
        let _guard = ConversionGuard::enter()?;
        let space = dict
            .get("ColorSpace")
            .or_else(|| dict.get("CS"))
            .ok_or_else(|| invalid("ICCBased image has no ColorSpace"))?;
        let prepared = PreparedIcc::new(space, reader, options, source_space)?;
        let n = prepared.ranges.len();
        if channels != n {
            return Err(invalid("ICC image component count does not match N"));
        }
        let output_size = |components| -> Result<usize> {
            let size = pixel_count
                .checked_mul(components)
                .ok_or_else(|| invalid("ICC image output size overflow"))?;
            if size as u64 > DecodeLimits::default().max_image_decoded_bytes {
                return Err(WellfriendError::UnsupportedFeature(
                    "ICC image exceeds decoded-byte budget".into(),
                ));
            }
            Ok(size)
        };
        let mut values = vec![0.0; n];
        if options.backend != cmm::ColorTransformBackend::DeterministicFallback {
            let native_size = output_size(3)?;
            let mut clipped = Vec::with_capacity(output_size(n)?);
            for index in 0..pixel_count {
                if index % 4096 == 0 {
                    crate::cancel::check_current_cancel("ICC image range clipping")?;
                }
                read_pixel(index, &mut values)?;
                for (value, (lo, hi)) in values.iter().zip(&prepared.ranges) {
                    if !value.is_finite() {
                        return Err(invalid("ICC image component is not finite"));
                    }
                    clipped.push(byte(value.clamp(*lo, *hi)));
                }
            }
            if let Some(converted) = prepared.native(&clipped)? {
                if converted.1 != 3 || converted.0.len() != native_size {
                    return Err(invalid(
                        "ICC native conversion returned invalid output shape",
                    ));
                }
                count(|m| m.native_conversions = m.native_conversions.saturating_add(1));
                return Ok(converted);
            }
        }
        let expected = output_size(4)?;
        let mut output = Vec::with_capacity(expected);
        // Bounded per-conversion memoization avoids re-evaluating a spot tint
        // function for repeated colours without retaining an unbounded palette.
        let mut palette = std::collections::HashMap::<[u64; 4], [u8; 4]>::new();
        for index in 0..pixel_count {
            if index % 4096 == 0 {
                crate::cancel::check_current_cancel("ICC alternate image conversion")?;
            }
            read_pixel(index, &mut values)?;
            let mut key = [0u64; 4];
            for (component, (value, (lo, hi))) in
                values.iter_mut().zip(&prepared.ranges).enumerate()
            {
                if !value.is_finite() {
                    return Err(invalid("ICC image component is not finite"));
                }
                *value = value.clamp(*lo, *hi);
                key[component] = value.to_bits();
            }
            let rgba = if let Some(rgba) = palette.get(&key) {
                *rgba
            } else {
                let rgba = match prepared.alternate_color_with_resources(
                    &values,
                    1.0,
                    reader,
                    policy.functions,
                )? {
                    NamedColor::Color(color) => color.to_pixel_color(),
                    NamedColor::NoPaint => [0, 0, 0, 0],
                    _ => unreachable!("alternate_color accepts only colour or no paint"),
                };
                if palette.len() < 256 {
                    palette.insert(key, rgba);
                }
                rgba
            };
            output.extend_from_slice(&rgba);
        }
        crate::cancel::check_current_cancel("ICC alternate image result")?;
        prepared.note_alternate();
        Ok((output, 4))
    })();
    if result.is_err() {
        count(|m| m.rejected_conversions = m.rejected_conversions.saturating_add(1));
    }
    result
}

#[cfg(test)]
#[path = "icc_conversion_tests.rs"]
mod tests;
