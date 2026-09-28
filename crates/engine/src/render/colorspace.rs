//! Resolution of special PDF colour spaces — `/Separation` and `/DeviceN`
//! (spec §8.6.6.4) — into device RGB for the fill/stroke paint path.
//!
//! A `/Separation` colour space has one tint component; `/DeviceN` has N. Both
//! carry an *alternate* colour space (a normal space such as DeviceCMYK /
//! DeviceRGB / DeviceGray / ICCBased) and a *tint-transform function* that maps
//! the N tint values into the alternate space. To paint, we evaluate the tint
//! transform (Function Types 0/2/3/4, all supported by
//! [`crate::render::function`]) and run the resulting components through the
//! existing alternate-space → RGB conversion.
//!
//! The tint transform is evaluated with the PDF function machinery (Function
//! Types 0 and 4, in `render/function.rs`); this module wires the fill/stroke
//! colour path through it.

use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::cmm;
use crate::render::color::{ColorSpaceHandler, RenderColor};
use crate::render::function::{FunctionResources, PreparedFunction};
use std::cell::{Cell, RefCell};
use std::mem::size_of;
use std::sync::{Arc, Weak};

pub(crate) const MAX_DEVICEN_COMPONENTS: usize = 16;
pub(crate) const DEFAULT_TINT_TRANSFORM_CACHE_ENTRIES: usize = 64;
pub(crate) const DEFAULT_TINT_TRANSFORM_CACHE_BYTES: usize =
    DEFAULT_TINT_TRANSFORM_CACHE_ENTRIES * MAX_TINT_TRANSFORM_OUTPUT_COMPONENTS * size_of::<f64>();
const MAX_TINT_TRANSFORM_OUTPUT_COMPONENTS: usize = 32;

thread_local! {
    static TINT_TRANSFORM_CACHE: RefCell<TintTransformCache> =
        RefCell::new(TintTransformCache::new(DEFAULT_TINT_TRANSFORM_CACHE_ENTRIES));
}

/// Outcome of resolving a named colour space to a paint colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NamedColor {
    /// A concrete colour to paint with.
    Color(RenderColor),
    /// The colour produces no marks at all (e.g. `/Separation /None`); the
    /// caller must skip the paint operation entirely.
    NoPaint,
    /// The colour space is one this resolver understands, but the concrete
    /// definition or component vector is malformed. Callers must not substitute
    /// a default colour for this result.
    Invalid(&'static str),
    /// The named space is not a Separation/DeviceN we can resolve (caller falls
    /// back to its existing behaviour).
    Unhandled,
}

const INVALID_CAL_GRAY: &str = "malformed CalGray color space";
const INVALID_CAL_RGB: &str = "malformed CalRGB color space";
const INVALID_LAB: &str = "malformed Lab color space";
const INVALID_INDEXED: &str = "malformed Indexed color space";
const INVALID_INDEXED_COMPONENTS: &str = "malformed Indexed components";
const INVALID_TINT_COMPONENTS: &str = "malformed tint components";
const INVALID_TINT_FUNCTION: &str = "malformed tint transform";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TintTransformCacheMetrics {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
    pub admissions: usize,
    pub rejections: usize,
    pub entries: usize,
    pub max_entries: usize,
    pub bytes_used: usize,
    pub max_bytes: usize,
}

#[derive(Debug, Clone)]
struct TintTransformCacheKey {
    // Weak retains allocation identity, not the decoded graph. Its control
    // block cannot be reused while an entry exists, even after reader eviction.
    function: Weak<PreparedFunction>,
    input_count: usize,
    input_bits: [u64; MAX_DEVICEN_COMPONENTS],
}

impl PartialEq for TintTransformCacheKey {
    fn eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.function, &other.function)
            && self.input_count == other.input_count
            && self.input_bits == other.input_bits
    }
}
impl Eq for TintTransformCacheKey {}

#[derive(Debug, Clone)]
struct CachedTintTransform {
    output: Vec<f64>,
    byte_cost: usize,
}

struct TintTransformCache {
    max_entries: usize,
    max_bytes: usize,
    current_bytes: usize,
    entries: Vec<(TintTransformCacheKey, CachedTintTransform)>,
    metrics: TintTransformCacheMetrics,
}

impl TintTransformCache {
    fn new(max_entries: usize) -> Self {
        let max_entries = max_entries.max(1);
        let max_bytes = max_entries * MAX_TINT_TRANSFORM_OUTPUT_COMPONENTS * size_of::<f64>();
        Self {
            max_entries,
            max_bytes,
            current_bytes: 0,
            entries: Vec::new(),
            metrics: TintTransformCacheMetrics {
                max_entries,
                max_bytes,
                ..TintTransformCacheMetrics::default()
            },
        }
    }

    fn metrics(&self) -> TintTransformCacheMetrics {
        TintTransformCacheMetrics {
            entries: self.entries.len(),
            max_entries: self.max_entries,
            bytes_used: self.current_bytes,
            max_bytes: self.max_bytes,
            ..self.metrics
        }
    }

    fn evaluate(
        &mut self,
        tint_fn: &PdfObject,
        inputs: &[f64],
        reader: &PdfReader,
        resources: FunctionResources<'_>,
    ) -> std::result::Result<Vec<f64>, NamedColor> {
        let Some(lease) = PreparedFunction::cached_with_resources(
            tint_fn,
            inputs.len(),
            false,
            reader,
            resources,
        ) else {
            self.metrics.misses += 1;
            return Err(NamedColor::Invalid(INVALID_TINT_FUNCTION));
        };
        let function = lease.graph();
        let key = tint_transform_cache_key(function, inputs);
        if let Some(key) = key.as_ref() {
            if let Some(idx) = self
                .entries
                .iter()
                .position(|(existing, _)| existing == key)
            {
                self.metrics.hits += 1;
                let (key, cached) = self.entries.remove(idx);
                let output = cached.output.clone();
                self.entries.push((key, cached));
                return Ok(output);
            }
        }

        self.metrics.misses += 1;
        let output = function.evaluate(inputs);
        if output.is_empty() {
            return Err(NamedColor::Invalid(INVALID_TINT_FUNCTION));
        }
        if let Some(key) = key {
            self.admit(key, output.clone());
        }
        Ok(output)
    }

    fn admit(&mut self, key: TintTransformCacheKey, output: Vec<f64>) {
        let byte_cost = output.len().saturating_mul(size_of::<f64>());
        if output.len() > MAX_TINT_TRANSFORM_OUTPUT_COMPONENTS || byte_cost > self.max_bytes {
            self.metrics.rejections += 1;
            return;
        }
        while !self.entries.is_empty()
            && (self.entries.len() >= self.max_entries
                || self.current_bytes.saturating_add(byte_cost) > self.max_bytes)
        {
            let (_, evicted) = self.entries.remove(0);
            self.current_bytes = self.current_bytes.saturating_sub(evicted.byte_cost);
            self.metrics.evictions += 1;
        }
        self.current_bytes = self.current_bytes.saturating_add(byte_cost);
        self.entries
            .push((key, CachedTintTransform { output, byte_cost }));
        self.metrics.admissions += 1;
    }
}

pub(crate) fn tint_transform_cache_metrics() -> TintTransformCacheMetrics {
    TINT_TRANSFORM_CACHE.with(|cache| cache.borrow().metrics())
}

#[cfg(test)]
fn reset_tint_transform_cache_for_tests(max_entries: usize) {
    TINT_TRANSFORM_CACHE.with(|cache| {
        *cache.borrow_mut() = TintTransformCache::new(max_entries);
    });
}

/// Resolve a named colour-space *resource object* with the given tint
/// `components` into a paint colour.
///
/// `space_obj` is the already-resolved `/ColorSpace` resource entry (an array
/// like `[/Separation /Name altSpace tintFn]` or
/// `[/DeviceN [names] altSpace tintFn ...]`, or a bare family name). `reader`
/// resolves the alternate-space and tint-function indirect references.
pub fn resolve_named_color(
    space_obj: &PdfObject,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
) -> NamedColor {
    resolve_named_color_with_options(
        space_obj,
        components,
        alpha,
        reader,
        cmm::ColorTransformOptions::default(),
    )
}

pub(crate) fn resolve_named_color_with_options(
    space_obj: &PdfObject,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
) -> NamedColor {
    resolve_named_color_with_source(space_obj, None, components, alpha, reader, options)
}

thread_local! {
    static ACTIVE_COLOR_DEPTH: Cell<usize> = const { Cell::new(0) };
}

struct ColorResolutionGuard;
impl ColorResolutionGuard {
    fn enter() -> Option<Self> {
        crate::cancel::check_current_cancel("named colour conversion").ok()?;
        ACTIVE_COLOR_DEPTH.with(|depth| {
            if depth.get() >= 32 {
                return None;
            }
            depth.set(depth.get() + 1);
            Some(Self)
        })
    }
}
impl Drop for ColorResolutionGuard {
    fn drop(&mut self) {
        ACTIVE_COLOR_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Interpret components using a bound colour graph, retaining the corresponding
/// original graph for nested Indexed lookup domains. The source must be resolved
/// in the resource scope that selected it, before Default* substitutions.
pub(crate) fn resolve_named_color_with_source(
    space_obj: &PdfObject,
    source_space: Option<&PdfObject>,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
) -> NamedColor {
    resolve_named_color_with_resources(
        space_obj,
        source_space,
        components,
        alpha,
        reader,
        options,
        FunctionResources::default(),
    )
}

pub(crate) fn resolve_named_color_with_resources(
    space_obj: &PdfObject,
    source_space: Option<&PdfObject>,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
    resources: FunctionResources<'_>,
) -> NamedColor {
    let Some(_guard) = ColorResolutionGuard::enter() else {
        return NamedColor::Invalid("colour conversion cancelled or nesting exceeds 32");
    };
    if !alpha.is_finite() {
        return NamedColor::Invalid("paint alpha is not finite");
    }
    let resolved;
    let space_obj = if matches!(space_obj, PdfObject::Reference { .. }) {
        resolved = match reader.resolve(space_obj.clone()) {
            Ok(value) => value,
            Err(_) => return NamedColor::Invalid("colour space reference cannot be resolved"),
        };
        &resolved
    } else {
        space_obj
    };
    let arr = match space_obj {
        PdfObject::Array(arr) => arr.as_slice(),
        PdfObject::Name(name) => return alternate_components_to_color(name, components, alpha),
        // Non-array/non-name objects are not color spaces this resolver can use.
        _ => return NamedColor::Unhandled,
    };
    let family = match arr.first().and_then(PdfObject::as_name) {
        Some(name) => name,
        None => return NamedColor::Unhandled,
    };

    match family {
        "Separation" => resolve_separation(
            arr,
            source_space,
            components,
            alpha,
            reader,
            options,
            resources,
        ),
        "DeviceN" => resolve_device_n(
            arr,
            source_space,
            components,
            alpha,
            reader,
            options,
            resources,
        ),
        "ICCBased" => super::icc_conversion::resolve_color_with_resources(
            space_obj,
            source_space,
            components,
            alpha,
            reader,
            options,
            resources,
        )
        .unwrap_or(NamedColor::Invalid(
            "ICCBased metadata, profile or Alternate conversion rejected",
        )),
        "Lab" => {
            let Some(params) = strict_lab_params_from_space(space_obj, reader) else {
                return NamedColor::Invalid(INVALID_LAB);
            };
            if !components_have_exact_finite_count(components, 3) {
                return NamedColor::Invalid(INVALID_LAB);
            }
            let l = components[0] as f32;
            let a = components[1] as f32;
            let b = components[2] as f32;
            let [r, g, b] = cmm::lab_to_srgb(l, a, b, params);
            NamedColor::Color(RenderColor::new(r, g, b, alpha))
        }
        "CalGray" => {
            let Some(params) = strict_cal_gray_params_from_space(space_obj, reader) else {
                return NamedColor::Invalid(INVALID_CAL_GRAY);
            };
            if !components_have_exact_finite_count(components, 1) {
                return NamedColor::Invalid(INVALID_CAL_GRAY);
            }
            let gray = components[0] as f32;
            let [r, g, b] = cmm::cal_gray_to_srgb(gray, params);
            NamedColor::Color(RenderColor::new(r, g, b, alpha))
        }
        "CalRGB" => {
            let Some(params) = strict_cal_rgb_params_from_space(space_obj, reader) else {
                return NamedColor::Invalid(INVALID_CAL_RGB);
            };
            if !components_have_exact_finite_count(components, 3) {
                return NamedColor::Invalid(INVALID_CAL_RGB);
            }
            let comps = [
                components[0] as f32,
                components[1] as f32,
                components[2] as f32,
            ];
            let [r, g, b] = cmm::cal_rgb_to_srgb(comps, params);
            NamedColor::Color(RenderColor::new(r, g, b, alpha))
        }
        "Indexed" => {
            if !components_have_exact_finite_count(components, 1) {
                return NamedColor::Invalid(INVALID_INDEXED_COMPONENTS);
            }
            crate::images::indexed_samples::resolve_color_with_resources(
                space_obj,
                source_space,
                components[0],
                alpha,
                reader,
                options,
                resources,
            )
            .unwrap_or(NamedColor::Invalid(INVALID_INDEXED))
        }
        _ => NamedColor::Unhandled,
    }
}

pub(crate) fn indexed_lookup_bytes(
    lookup: &PdfObject,
    reader: &PdfReader,
) -> Result<Vec<u8>, String> {
    use crate::filters::{decode_stream_lossless_with_limits, DecodeLimits, StreamDecodeStatus};
    // At most 256 entries, each with at most 16 supported base components.
    const MAX_LOOKUP_BYTES: usize = 256 * MAX_DEVICEN_COMPONENTS;
    crate::cancel::check_current_cancel("Indexed palette decoding").map_err(|e| e.to_string())?;
    let lookup = reader.resolve(lookup.clone()).map_err(|e| e.to_string())?;
    let bytes = match lookup {
        PdfObject::String(bytes) => bytes,
        stream @ PdfObject::Stream { .. } => {
            let limits = DecodeLimits {
                // Allow bounded filter intermediates larger than the final table.
                max_decoded_bytes_per_stream: 64 * 1024,
                max_decoded_bytes_per_document: 64 * 1024,
                ..DecodeLimits::default()
            };
            let decoded = decode_stream_lossless_with_limits(&stream, reader, &limits)
                .map_err(|e| e.to_string())?;
            if !matches!(decoded.status, StreamDecodeStatus::Complete) {
                return Err("Indexed lookup uses an image filter instead of a byte stream".into());
            }
            decoded.data
        }
        _ => return Err("Indexed lookup is not a String or Stream".into()),
    };
    if bytes.len() > MAX_LOOKUP_BYTES {
        return Err("Indexed lookup exceeds the supported palette size".into());
    }
    Ok(bytes)
}

/// `[/Separation /Name altSpace tintTransform]`
fn resolve_separation(
    arr: &[PdfObject],
    source_space: Option<&PdfObject>,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
    resources: FunctionResources<'_>,
) -> NamedColor {
    // Colorant name: /None paints nothing; /All approximates as full ink.
    let colorant = arr.get(1).and_then(PdfObject::as_name);
    if colorant == Some("None") {
        return NamedColor::NoPaint;
    }
    let alt = match arr.get(2) {
        Some(obj) => obj,
        None => return NamedColor::Unhandled,
    };
    let tint_fn = match arr.get(3) {
        Some(obj) => obj,
        None => return NamedColor::Unhandled,
    };

    if !components_have_exact_finite_count(components, 1) {
        return NamedColor::Invalid(INVALID_TINT_COMPONENTS);
    }
    let tint = components[0];

    let alt_components =
        match evaluate_tint_transform_with_resources(tint_fn, &[tint], reader, resources) {
            Ok(components) => components,
            Err(invalid) => return invalid,
        };
    let source_alt = match source_alternate(source_space, "Separation", reader) {
        Ok(value) => value,
        Err(reason) => return NamedColor::Invalid(reason),
    };
    resolve_alternate_color(
        alt,
        source_alt.as_ref(),
        &alt_components,
        alpha,
        reader,
        options,
        resources,
    )
}

/// `[/DeviceN [/Name1 /Name2 ...] altSpace tintTransform attributes?]`
fn resolve_device_n(
    arr: &[PdfObject],
    source_space: Option<&PdfObject>,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
    resources: FunctionResources<'_>,
) -> NamedColor {
    let names = match arr.get(1).and_then(PdfObject::as_array) {
        Some(n) => n,
        None => return NamedColor::Unhandled,
    };
    if names.is_empty() || names.iter().any(|name| name.as_name().is_none()) {
        return NamedColor::Invalid(INVALID_TINT_COMPONENTS);
    }
    if names.len() > MAX_DEVICEN_COMPONENTS {
        return NamedColor::Unhandled;
    }
    // If every colorant is /None, the space produces no marks.
    if !names.is_empty() && names.iter().all(|n| n.as_name() == Some("None")) {
        return NamedColor::NoPaint;
    }
    let alt = match arr.get(2) {
        Some(obj) => obj,
        None => return NamedColor::Unhandled,
    };
    let tint_fn = match arr.get(3) {
        Some(obj) => obj,
        None => return NamedColor::Unhandled,
    };

    // Feed all N tint components through the multi-input tint transform.
    let n = names.len();
    if !components_have_exact_finite_count(components, n) {
        return NamedColor::Invalid(INVALID_TINT_COMPONENTS);
    }
    let alt_components =
        match evaluate_tint_transform_with_resources(tint_fn, components, reader, resources) {
            Ok(components) => components,
            Err(invalid) => return invalid,
        };
    let source_alt = match source_alternate(source_space, "DeviceN", reader) {
        Ok(value) => value,
        Err(reason) => return NamedColor::Invalid(reason),
    };
    resolve_alternate_color(
        alt,
        source_alt.as_ref(),
        &alt_components,
        alpha,
        reader,
        options,
        resources,
    )
}

fn source_alternate(
    source: Option<&PdfObject>,
    family: &str,
    reader: &PdfReader,
) -> Result<Option<PdfObject>, &'static str> {
    let Some(source) = source else {
        return Ok(None);
    };
    let source = reader
        .resolve(source.clone())
        .map_err(|_| "original colour space reference cannot be resolved")?;
    if super::default_colorspace::family(&source) != Some(family) {
        // Default* replaced this node: its alternate is a new graph, not a
        // child of the original device space.
        return Ok(None);
    }
    source
        .as_array()
        .and_then(|items| items.get(2))
        .cloned()
        .map(Some)
        .ok_or("original colour space alternate is missing")
}

#[cfg(test)]
fn evaluate_tint_transform(
    tint_fn: &PdfObject,
    inputs: &[f64],
    reader: &PdfReader,
) -> std::result::Result<Vec<f64>, NamedColor> {
    evaluate_tint_transform_with_resources(tint_fn, inputs, reader, FunctionResources::default())
}

fn evaluate_tint_transform_with_resources(
    tint_fn: &PdfObject,
    inputs: &[f64],
    reader: &PdfReader,
    resources: FunctionResources<'_>,
) -> std::result::Result<Vec<f64>, NamedColor> {
    TINT_TRANSFORM_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .evaluate(tint_fn, inputs, reader, resources)
    })
}

fn tint_transform_cache_key(
    function: &Arc<PreparedFunction>,
    inputs: &[f64],
) -> Option<TintTransformCacheKey> {
    if inputs.len() > MAX_DEVICEN_COMPONENTS || inputs.iter().any(|input| !input.is_finite()) {
        return None;
    }
    let mut input_bits = [0u64; MAX_DEVICEN_COMPONENTS];
    for (idx, input) in inputs.iter().enumerate() {
        input_bits[idx] = input.to_bits();
    }
    Some(TintTransformCacheKey {
        function: Arc::downgrade(function),
        input_count: inputs.len(),
        input_bits,
    })
}

fn resolve_alternate_color(
    alt: &PdfObject,
    source_alt: Option<&PdfObject>,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
    resources: FunctionResources<'_>,
) -> NamedColor {
    let resolved = match alt {
        PdfObject::Reference { .. } => match reader.resolve(alt.clone()) {
            Ok(value) => value,
            Err(_) => {
                return NamedColor::Invalid("alternate colour space reference cannot be resolved")
            }
        },
        other => other.clone(),
    };
    match resolve_named_color_with_resources(
        &resolved, source_alt, components, alpha, reader, options, resources,
    ) {
        NamedColor::Color(color) => return NamedColor::Color(color),
        NamedColor::NoPaint => return NamedColor::NoPaint,
        NamedColor::Invalid(reason) => return NamedColor::Invalid(reason),
        NamedColor::Unhandled => {}
    }
    let Some(alt_name) = alternate_space_name(&resolved, reader) else {
        return NamedColor::Unhandled;
    };
    alternate_components_to_color(&alt_name, components, alpha)
}

fn alternate_components_to_color(space_name: &str, components: &[f64], alpha: f32) -> NamedColor {
    match space_name {
        "DeviceGray" | "G" | "DeviceRGB" | "RGB" | "sRGB" | "DeviceCMYK" | "CMYK" => {
            match ColorSpaceHandler::try_from_components(space_name, components, alpha) {
                Some(color) => NamedColor::Color(color),
                None => NamedColor::Invalid(INVALID_TINT_COMPONENTS),
            }
        }
        "CalGray" => NamedColor::Invalid(INVALID_CAL_GRAY),
        "CalRGB" => NamedColor::Invalid(INVALID_CAL_RGB),
        "Lab" => NamedColor::Invalid(INVALID_LAB),
        _ => NamedColor::Unhandled,
    }
}

fn strict_lab_params_from_space(
    space_obj: &PdfObject,
    reader: &PdfReader,
) -> Option<cmm::LabParams> {
    let dict = strict_calibrated_param_dict(space_obj, reader, "Lab")?;
    valid_required_xyz(&dict, "WhitePoint")?;
    valid_optional_xyz(&dict, "BlackPoint")?;
    valid_optional_lab_range(&dict)?;
    cmm::lab_params_from_space(space_obj, Some(reader))
}

fn strict_cal_gray_params_from_space(
    space_obj: &PdfObject,
    reader: &PdfReader,
) -> Option<cmm::CalGrayParams> {
    let dict = strict_calibrated_param_dict(space_obj, reader, "CalGray")?;
    valid_required_xyz(&dict, "WhitePoint")?;
    valid_optional_xyz(&dict, "BlackPoint")?;
    valid_optional_positive_number(&dict, "Gamma")?;
    cmm::cal_gray_params_from_space(space_obj, Some(reader))
}

fn strict_cal_rgb_params_from_space(
    space_obj: &PdfObject,
    reader: &PdfReader,
) -> Option<cmm::CalRgbParams> {
    let dict = strict_calibrated_param_dict(space_obj, reader, "CalRGB")?;
    valid_required_xyz(&dict, "WhitePoint")?;
    valid_optional_xyz(&dict, "BlackPoint")?;
    valid_optional_positive_number_array(&dict, "Gamma", 3)?;
    valid_optional_number_array(&dict, "Matrix", 9)?;
    cmm::cal_rgb_params_from_space(space_obj, Some(reader))
}

fn strict_calibrated_param_dict(
    space_obj: &PdfObject,
    reader: &PdfReader,
    expected_family: &str,
) -> Option<PdfDictionary> {
    let resolved = match space_obj {
        PdfObject::Reference { .. } => reader.resolve(space_obj.clone()).ok()?,
        other => other.clone(),
    };
    let arr = resolved.as_array()?;
    if arr.first().and_then(PdfObject::as_name) != Some(expected_family) {
        return None;
    }
    if arr.len() != 2 {
        return None;
    }
    resolve_param_dict(arr.get(1)?, reader)
}

fn resolve_param_dict(obj: &PdfObject, reader: &PdfReader) -> Option<PdfDictionary> {
    let resolved = match obj {
        PdfObject::Reference { .. } => reader.resolve(obj.clone()).ok()?,
        other => other.clone(),
    };
    resolved.as_dict().cloned()
}

fn components_have_exact_finite_count(components: &[f64], expected_len: usize) -> bool {
    components.len() == expected_len && components.iter().all(|value| value.is_finite())
}

fn valid_required_xyz(dict: &PdfDictionary, key: &str) -> Option<()> {
    let values = numeric_array(dict.get(key)?, 3)?;
    valid_xyz(&values).then_some(())
}

fn valid_optional_xyz(dict: &PdfDictionary, key: &str) -> Option<()> {
    match dict.get(key) {
        Some(obj) => {
            let values = numeric_array(obj, 3)?;
            valid_xyz(&values).then_some(())
        }
        None => Some(()),
    }
}

fn valid_xyz(values: &[f64]) -> bool {
    values.len() == 3
        && values.iter().all(|value| value.is_finite())
        && values[0] >= 0.0
        && values[1] > 0.0
        && values[2] >= 0.0
}

fn valid_optional_positive_number(dict: &PdfDictionary, key: &str) -> Option<()> {
    match dict.get(key) {
        Some(obj) => {
            let value = obj.as_number()?;
            (value.is_finite() && value > 0.0).then_some(())
        }
        None => Some(()),
    }
}

fn valid_optional_positive_number_array(
    dict: &PdfDictionary,
    key: &str,
    expected_len: usize,
) -> Option<()> {
    match dict.get(key) {
        Some(obj) => {
            let values = numeric_array(obj, expected_len)?;
            values
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
                .then_some(())
        }
        None => Some(()),
    }
}

fn valid_optional_number_array(dict: &PdfDictionary, key: &str, expected_len: usize) -> Option<()> {
    match dict.get(key) {
        Some(obj) => {
            let values = numeric_array(obj, expected_len)?;
            values.iter().all(|value| value.is_finite()).then_some(())
        }
        None => Some(()),
    }
}

fn valid_optional_lab_range(dict: &PdfDictionary) -> Option<()> {
    match dict.get("Range") {
        Some(obj) => {
            let values = numeric_array(obj, 4)?;
            (values.iter().all(|value| value.is_finite())
                && values[0] <= values[1]
                && values[2] <= values[3])
                .then_some(())
        }
        None => Some(()),
    }
}

fn numeric_array(obj: &PdfObject, expected_len: usize) -> Option<Vec<f64>> {
    let arr = obj.as_array()?;
    if arr.len() != expected_len {
        return None;
    }
    arr.iter().map(PdfObject::as_number).collect()
}

/// Map an alternate colour-space object to the family name understood by
/// [`ColorSpaceHandler::from_components`]. ICCBased is reduced to a device space
/// by its component count (`/N`) when the full ICC stream is unavailable in this
/// spot-color shortcut.
fn alternate_space_name(alt: &PdfObject, reader: &PdfReader) -> Option<String> {
    let resolved = match alt {
        PdfObject::Reference { .. } => reader.resolve(alt.clone()).ok()?,
        other => other.clone(),
    };
    match &resolved {
        PdfObject::Name(name) => Some(name.clone()),
        PdfObject::Array(arr) => {
            let head = arr.first().and_then(PdfObject::as_name)?;
            if head == "ICCBased" {
                // Resolve the stream's /N to pick the device space.
                let n = arr
                    .get(1)
                    .and_then(|s| reader.resolve(s.clone()).ok())
                    .and_then(|obj| match obj {
                        PdfObject::Stream { dict, .. } => dict.get_integer("N"),
                        _ => None,
                    })?;
                match n {
                    1 => Some("DeviceGray".to_string()),
                    3 => Some("DeviceRGB".to_string()),
                    4 => Some("DeviceCMYK".to_string()),
                    _ => None,
                }
            } else {
                Some(head.to_string())
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{PdfDictionary, PdfObject};
    use std::collections::BTreeMap;

    fn reader() -> PdfReader {
        PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap()
    }

    fn name(s: &str) -> PdfObject {
        PdfObject::Name(s.to_string())
    }

    fn real_arr(vals: &[f64]) -> PdfObject {
        PdfObject::Array(vals.iter().map(|&v| PdfObject::Real(v)).collect())
    }

    fn dict_obj(entries: &[(&str, PdfObject)]) -> PdfObject {
        let mut map = BTreeMap::new();
        for (key, value) in entries {
            map.insert((*key).to_string(), value.clone());
        }
        PdfObject::Dictionary(PdfDictionary::new(map))
    }

    /// Type 2 tint transform: C0 -> C1 in the alternate space.
    fn type2_fn(c0: &[f64], c1: &[f64]) -> PdfObject {
        let mut m: BTreeMap<String, PdfObject> = BTreeMap::new();
        m.insert("FunctionType".into(), PdfObject::Integer(2));
        m.insert("Domain".into(), real_arr(&[0.0, 1.0]));
        m.insert("C0".into(), real_arr(c0));
        m.insert("C1".into(), real_arr(c1));
        m.insert("N".into(), PdfObject::Real(1.0));
        PdfObject::Dictionary(PdfDictionary::new(m))
    }

    #[test]
    fn named_colour_depth_and_cancellation_rejection_do_not_poison_next_paint() {
        let reader = reader();
        // Deliberately nonconforming recursive alternate nesting must fail
        // closed, not overflow the stack or leave a thread-local depth behind.
        let mut nested = name("DeviceGray");
        for _ in 0..40 {
            nested = PdfObject::Array(vec![
                name("Separation"),
                name("Ink"),
                nested,
                type2_fn(&[0.0], &[1.0]),
            ]);
        }
        assert!(matches!(
            resolve_named_color(&nested, &[0.5], 1.0, &reader),
            NamedColor::Invalid(_)
        ));
        assert_eq!(ACTIVE_COLOR_DEPTH.with(Cell::get), 0);
        let cancel = crate::cancel::CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            cancel.scope(|| resolve_named_color(&name("DeviceGray"), &[0.5], 1.0, &reader)),
            NamedColor::Invalid(_)
        ));
        assert_eq!(ACTIVE_COLOR_DEPTH.with(Cell::get), 0);
        assert!(matches!(
            resolve_named_color(&name("DeviceGray"), &[0.5], 1.0, &reader),
            NamedColor::Color(_)
        ));
    }

    fn type4_device_n_rgb_fn() -> PdfObject {
        let mut m: BTreeMap<String, PdfObject> = BTreeMap::new();
        m.insert("FunctionType".into(), PdfObject::Integer(4));
        m.insert("Domain".into(), real_arr(&[0.0, 1.0, 0.0, 1.0]));
        m.insert("Range".into(), real_arr(&[0.0, 1.0, 0.0, 1.0, 0.0, 1.0]));
        let program = b"{ 0 }".to_vec(); // a b -> a b 0
        PdfObject::Stream {
            dict: PdfDictionary::new({
                let mut mm = m.clone();
                mm.insert("Length".into(), PdfObject::Integer(program.len() as i64));
                mm
            }),
            raw: program,
        }
    }

    fn device_n_two_component_rgb_space() -> PdfObject {
        PdfObject::Array(vec![
            name("DeviceN"),
            PdfObject::Array(vec![name("Spot1"), name("Spot2")]),
            name("DeviceRGB"),
            type4_device_n_rgb_fn(),
        ])
    }

    #[test]
    fn tint_transform_cache_reuses_device_n_function_output() {
        reset_tint_transform_cache_for_tests(4);
        let r = reader();
        let space = device_n_two_component_rgb_space();
        let first = resolve_named_color(&space, &[0.25, 0.75], 1.0, &r);
        let second = resolve_named_color(&space, &[0.25, 0.75], 1.0, &r);
        assert!(matches!(first, NamedColor::Color(_)));
        assert_eq!(first, second);

        let metrics = tint_transform_cache_metrics();
        assert_eq!(metrics.misses, 1);
        assert_eq!(metrics.hits, 1);
        assert_eq!(metrics.admissions, 1);
        assert_eq!(metrics.evictions, 0);
        assert_eq!(metrics.entries, 1);
        assert_eq!(metrics.max_entries, 4);
        assert!(metrics.bytes_used > 0);
        assert_eq!(
            metrics.max_bytes,
            4 * MAX_TINT_TRANSFORM_OUTPUT_COMPONENTS * size_of::<f64>()
        );
    }

    #[test]
    fn tint_transform_cache_evicts_lru_entries() {
        reset_tint_transform_cache_for_tests(1);
        let r = reader();
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("Spot"),
            name("DeviceGray"),
            type2_fn(&[1.0], &[0.0]),
        ]);

        assert!(matches!(
            resolve_named_color(&space, &[0.25], 1.0, &r),
            NamedColor::Color(_)
        ));
        assert!(matches!(
            resolve_named_color(&space, &[0.75], 1.0, &r),
            NamedColor::Color(_)
        ));

        let metrics = tint_transform_cache_metrics();
        assert_eq!(metrics.misses, 2);
        assert_eq!(metrics.hits, 0);
        assert_eq!(metrics.admissions, 2);
        assert_eq!(metrics.evictions, 1);
        assert_eq!(metrics.entries, 1);
    }

    #[test]
    fn tint_transform_cache_does_not_admit_invalid_functions() {
        reset_tint_transform_cache_for_tests(4);
        let r = reader();
        let tint_fn = dict_obj(&[
            ("FunctionType", PdfObject::Integer(2)),
            ("C0", real_arr(&[0.0])),
            ("C1", real_arr(&[1.0])),
            ("N", PdfObject::Real(1.0)),
        ]);
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("Spot"),
            name("DeviceGray"),
            tint_fn,
        ]);

        assert_eq!(
            resolve_named_color(&space, &[0.5], 1.0, &r),
            NamedColor::Invalid(INVALID_TINT_FUNCTION)
        );
        assert_eq!(
            resolve_named_color(&space, &[0.5], 1.0, &r),
            NamedColor::Invalid(INVALID_TINT_FUNCTION)
        );

        let metrics = tint_transform_cache_metrics();
        assert_eq!(metrics.misses, 2);
        assert_eq!(metrics.hits, 0);
        assert_eq!(metrics.admissions, 0);
        assert_eq!(metrics.entries, 0);
    }

    #[test]
    fn tint_transform_cache_key_includes_document_boundary() {
        let r1 = reader();
        let mut bytes = crate::render::shading::tests_minimal_pdf();
        bytes[7] = b'5';
        let r2 = PdfReader::from_bytes(bytes).unwrap();
        let tint_fn = type2_fn(&[1.0], &[0.0]);

        let function1 = PreparedFunction::cached_single(&tint_fn, 1, &r1).unwrap();
        let function2 = PreparedFunction::cached_single(&tint_fn, 1, &r2).unwrap();
        let key1 = tint_transform_cache_key(&function1, &[0.25]).unwrap();
        let key2 = tint_transform_cache_key(&function2, &[0.25]).unwrap();

        assert_ne!(key1, key2);
        assert!(!Weak::ptr_eq(&key1.function, &key2.function));
        assert_eq!(key1.input_bits, key2.input_bits);
    }

    #[test]
    fn tint_results_do_not_hold_graphs_after_reader_drop() {
        reset_tint_transform_cache_for_tests(4);
        {
            let reader = reader();
            assert!(evaluate_tint_transform(&type2_fn(&[0.0], &[1.0]), &[0.25], &reader).is_ok());
            TINT_TRANSFORM_CACHE.with(|cache| {
                assert!(cache.borrow().entries[0].0.function.upgrade().is_some());
            });
        }
        TINT_TRANSFORM_CACHE.with(|cache| {
            assert!(cache.borrow().entries[0].0.function.upgrade().is_none());
        });
    }

    #[test]
    fn tint_keys_preserve_float_bits_and_evicted_allocation_identity() {
        let reader = reader();
        let object = type2_fn(&[0.0], &[1.0]);
        let graph = PreparedFunction::cached_single(&object, 1, &reader).unwrap();
        let old = tint_transform_cache_key(&graph, &[0.0]).unwrap();
        assert_ne!(old, tint_transform_cache_key(&graph, &[-0.0]).unwrap());
        assert!(tint_transform_cache_key(&graph, &[f64::NAN]).is_none());
        *reader.function_cache.lock().unwrap() = crate::render::function::FunctionCache::default();
        drop(graph);
        assert!(old.function.upgrade().is_none());
        let graph = PreparedFunction::cached_single(&object, 1, &reader).unwrap();
        assert_ne!(old, tint_transform_cache_key(&graph, &[0.0]).unwrap());
    }

    #[test]
    fn tint_reference_values_do_not_cross_readers() {
        use crate::render::parameter_dictionary::tests::{reader_with_objects, reference};
        reset_tint_transform_cache_for_tests(4);
        let first = reader_with_objects(&[type2_fn(&[0.0], &[1.0])]);
        let second = reader_with_objects(&[type2_fn(&[0.0], &[0.5])]);
        assert_eq!(
            evaluate_tint_transform(&reference(4), &[1.0], &first),
            Ok(vec![1.0])
        );
        assert_eq!(
            evaluate_tint_transform(&reference(4), &[1.0], &second),
            Ok(vec![0.5])
        );
        assert_eq!(
            evaluate_tint_transform(&reference(4), &[1.0], &first),
            Ok(vec![1.0])
        );
        assert_eq!(tint_transform_cache_metrics().hits, 1);
        assert_eq!(tint_transform_cache_metrics().misses, 2);
    }

    #[test]
    fn tint_output_hit_still_observes_cancellation() {
        reset_tint_transform_cache_for_tests(4);
        let reader = reader();
        let object = type2_fn(&[0.0], &[1.0]);
        assert_eq!(
            evaluate_tint_transform(&object, &[0.5], &reader),
            Ok(vec![0.5])
        );
        let cancel = crate::cancel::CancelToken::new();
        cancel.cancel();
        assert_eq!(
            cancel.scope(|| evaluate_tint_transform(&object, &[0.5], &reader)),
            Err(NamedColor::Invalid(INVALID_TINT_FUNCTION))
        );
        assert_eq!(tint_transform_cache_metrics().hits, 0);
        assert_eq!(
            evaluate_tint_transform(&object, &[0.5], &reader),
            Ok(vec![0.5])
        );
        assert_eq!(tint_transform_cache_metrics().hits, 1);
    }

    #[test]
    fn separation_tint0_is_white_cmyk() {
        // /Separation spot -> DeviceCMYK, tint 0 = all-zero CMYK = white.
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("PANTONE 286 C"),
            name("DeviceCMYK"),
            type2_fn(&[0.0, 0.0, 0.0, 0.0], &[1.0, 0.5, 0.0, 0.2]),
        ]);
        let r = resolve_named_color(&space, &[0.0], 1.0, &reader());
        match r {
            NamedColor::Color(c) => {
                assert!((c.r - 1.0).abs() < 0.01, "white R: {}", c.r);
                assert!((c.g - 1.0).abs() < 0.01, "white G: {}", c.g);
                assert!((c.b - 1.0).abs() < 0.01, "white B: {}", c.b);
            }
            other => panic!("expected Color, got {other:?}"),
        }
    }

    #[test]
    fn separation_tint1_is_alt_cmyk_full() {
        // tint 1 -> C1 = CMYK(1, 0.5, 0, 0.2), resolved through the shared
        // Poppler-like DeviceCMYK fallback.
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("PANTONE 286 C"),
            name("DeviceCMYK"),
            type2_fn(&[0.0, 0.0, 0.0, 0.0], &[1.0, 0.5, 0.0, 0.2]),
        ]);
        let r = resolve_named_color(&space, &[1.0], 1.0, &reader());
        match r {
            NamedColor::Color(c) => {
                assert!(
                    (c.r - 0.10).abs() < 0.03,
                    "R near process fallback: {}",
                    c.r
                );
                assert!(
                    (c.g - 0.37).abs() < 0.03,
                    "G near process fallback: {}",
                    c.g
                );
                assert!(
                    (c.b - 0.63).abs() < 0.03,
                    "B near process fallback: {}",
                    c.b
                );
            }
            other => panic!("expected Color, got {other:?}"),
        }
    }

    #[test]
    fn separation_none_produces_no_paint() {
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("None"),
            name("DeviceCMYK"),
            type2_fn(&[0.0, 0.0, 0.0, 0.0], &[0.0, 0.0, 0.0, 1.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[1.0], 1.0, &reader()),
            NamedColor::NoPaint
        );
    }

    #[test]
    fn separation_all_uses_supplied_tint() {
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("All"),
            name("DeviceGray"),
            // gray 1.0 at tint 0 -> 0.0 at tint 1.
            type2_fn(&[1.0], &[0.0]),
        ]);
        match resolve_named_color(&space, &[0.0], 1.0, &reader()) {
            NamedColor::Color(c) => assert!(c.r > 0.99, "All tint 0 remains white: {}", c.r),
            other => panic!("expected Color, got {other:?}"),
        }
    }

    #[test]
    fn separation_missing_tint_component_is_invalid_not_full_ink() {
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("Spot"),
            name("DeviceGray"),
            type2_fn(&[1.0], &[0.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[], 1.0, &reader()),
            NamedColor::Invalid(INVALID_TINT_COMPONENTS)
        );
    }

    #[test]
    fn separation_overlong_tint_component_is_invalid_not_truncated() {
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("Spot"),
            name("DeviceGray"),
            type2_fn(&[1.0], &[0.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.25, 0.75], 1.0, &reader()),
            NamedColor::Invalid(INVALID_TINT_COMPONENTS)
        );
    }

    #[test]
    fn separation_overlong_alternate_components_are_invalid_not_truncated() {
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("Spot"),
            name("DeviceRGB"),
            type2_fn(&[0.0, 0.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 0.5]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[1.0], 1.0, &reader()),
            NamedColor::Invalid(INVALID_TINT_COMPONENTS)
        );
    }

    #[test]
    fn separation_malformed_tint_transform_is_invalid_not_defaulted() {
        let tint_fn = dict_obj(&[
            ("FunctionType", PdfObject::Integer(2)),
            ("C0", real_arr(&[0.0, 0.0, 0.0])),
            ("C1", real_arr(&[1.0, 0.0, 0.0])),
            ("N", PdfObject::Real(1.0)),
        ]);
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("Spot"),
            name("DeviceRGB"),
            tint_fn,
        ]);
        assert_eq!(
            resolve_named_color(&space, &[1.0], 1.0, &reader()),
            NamedColor::Invalid(INVALID_TINT_FUNCTION)
        );
    }

    #[test]
    fn device_n_two_inputs_feed_tint_transform() {
        // DeviceN with 2 colorants -> DeviceRGB via a Type 4 transform that maps
        // [a b] -> [a, b, 0]. With inputs [0.25, 0.75] -> RGB(0.25, 0.75, 0).
        let space = device_n_two_component_rgb_space();
        match resolve_named_color(&space, &[0.25, 0.75], 1.0, &reader()) {
            NamedColor::Color(c) => {
                assert!((c.r - 0.25).abs() < 0.02, "R~0.25: {}", c.r);
                assert!((c.g - 0.75).abs() < 0.02, "G~0.75: {}", c.g);
                assert!(c.b < 0.02, "B~0: {}", c.b);
            }
            other => panic!("expected Color, got {other:?}"),
        }
    }

    #[test]
    fn device_n_component_cap_is_unhandled() {
        let names = (0..(MAX_DEVICEN_COMPONENTS + 1))
            .map(|i| name(&format!("Spot{i}")))
            .collect::<Vec<_>>();
        let space = PdfObject::Array(vec![
            name("DeviceN"),
            PdfObject::Array(names),
            name("DeviceRGB"),
            type2_fn(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.5; MAX_DEVICEN_COMPONENTS + 1], 1.0, &reader()),
            NamedColor::Unhandled
        );
    }

    #[test]
    fn device_n_missing_components_are_invalid_not_padded() {
        let space = PdfObject::Array(vec![
            name("DeviceN"),
            PdfObject::Array(vec![name("Spot1"), name("Spot2")]),
            name("DeviceRGB"),
            type2_fn(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.25], 1.0, &reader()),
            NamedColor::Invalid(INVALID_TINT_COMPONENTS)
        );
    }

    #[test]
    fn device_n_overlong_components_are_invalid_not_truncated() {
        let space = PdfObject::Array(vec![
            name("DeviceN"),
            PdfObject::Array(vec![name("Spot1"), name("Spot2")]),
            name("DeviceRGB"),
            type2_fn(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.25, 0.5, 0.75], 1.0, &reader()),
            NamedColor::Invalid(INVALID_TINT_COMPONENTS)
        );
    }

    #[test]
    fn device_n_single_input_tint_transform_is_invalid_for_two_colorants() {
        let space = PdfObject::Array(vec![
            name("DeviceN"),
            PdfObject::Array(vec![name("Spot1"), name("Spot2")]),
            name("DeviceRGB"),
            type2_fn(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.25, 0.75], 1.0, &reader()),
            NamedColor::Invalid(INVALID_TINT_FUNCTION)
        );
    }

    #[test]
    fn device_n_non_name_component_is_invalid_not_padded() {
        let space = PdfObject::Array(vec![
            name("DeviceN"),
            PdfObject::Array(vec![PdfObject::Integer(7)]),
            name("DeviceRGB"),
            type2_fn(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.25], 1.0, &reader()),
            NamedColor::Invalid(INVALID_TINT_COMPONENTS)
        );
    }

    #[test]
    fn calgray_without_whitepoint_is_invalid_named_color() {
        let space = PdfObject::Array(vec![name("CalGray"), dict_obj(&[])]);
        assert_eq!(
            resolve_named_color(&space, &[0.5], 1.0, &reader()),
            NamedColor::Invalid(INVALID_CAL_GRAY)
        );
    }

    #[test]
    fn calrgb_malformed_gamma_is_invalid_named_color() {
        let bad_gamma = PdfObject::Array(vec![
            PdfObject::Real(1.0),
            name("bad"),
            PdfObject::Real(1.0),
        ]);
        let space = PdfObject::Array(vec![
            name("CalRGB"),
            dict_obj(&[
                ("WhitePoint", real_arr(&[1.0, 1.0, 1.0])),
                ("Gamma", bad_gamma),
            ]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.2, 0.4, 0.6], 1.0, &reader()),
            NamedColor::Invalid(INVALID_CAL_RGB)
        );
    }

    #[test]
    fn lab_malformed_range_is_invalid_named_color() {
        let space = PdfObject::Array(vec![
            name("Lab"),
            dict_obj(&[
                ("WhitePoint", real_arr(&[1.0, 1.0, 1.0])),
                ("Range", real_arr(&[100.0, -100.0, -100.0, 100.0])),
            ]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[50.0, 0.0, 0.0], 1.0, &reader()),
            NamedColor::Invalid(INVALID_LAB)
        );
    }

    #[test]
    fn calrgb_missing_components_is_invalid_named_color() {
        let space = PdfObject::Array(vec![
            name("CalRGB"),
            dict_obj(&[("WhitePoint", real_arr(&[1.0, 1.0, 1.0]))]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.2, 0.4], 1.0, &reader()),
            NamedColor::Invalid(INVALID_CAL_RGB)
        );
    }

    #[test]
    fn calrgb_overlong_components_is_invalid_named_color() {
        let space = PdfObject::Array(vec![
            name("CalRGB"),
            dict_obj(&[("WhitePoint", real_arr(&[1.0, 1.0, 1.0]))]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.2, 0.4, 0.6, 0.8], 1.0, &reader()),
            NamedColor::Invalid(INVALID_CAL_RGB)
        );
    }

    #[test]
    fn separation_malformed_calrgb_alternate_is_invalid_named_color() {
        let bad_gamma = PdfObject::Array(vec![
            PdfObject::Real(1.0),
            name("bad"),
            PdfObject::Real(1.0),
        ]);
        let alt = PdfObject::Array(vec![
            name("CalRGB"),
            dict_obj(&[
                ("WhitePoint", real_arr(&[1.0, 1.0, 1.0])),
                ("Gamma", bad_gamma),
            ]),
        ]);
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("Spot"),
            alt,
            type2_fn(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[1.0], 1.0, &reader()),
            NamedColor::Invalid(INVALID_CAL_RGB)
        );
    }

    #[test]
    fn separation_unknown_alternate_is_unhandled_not_black() {
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("Spot"),
            name("Indexed"),
            type2_fn(&[0.0], &[1.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[1.0], 1.0, &reader()),
            NamedColor::Unhandled
        );
    }

    #[test]
    fn separation_malformed_alternate_array_is_unhandled_not_device_rgb() {
        let space = PdfObject::Array(vec![
            name("Separation"),
            name("Spot"),
            PdfObject::Array(vec![PdfObject::Integer(42)]),
            type2_fn(&[0.0, 0.0, 0.0], &[1.0, 0.0, 0.0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[1.0], 1.0, &reader()),
            NamedColor::Unhandled
        );
    }

    #[test]
    fn bare_device_rgb_resource_name_resolves_color() {
        match resolve_named_color(&name("DeviceRGB"), &[0.2, 0.4, 0.6], 1.0, &reader()) {
            NamedColor::Color(color) => {
                assert!((color.r - 0.2).abs() < 0.001);
                assert!((color.g - 0.4).abs() < 0.001);
                assert!((color.b - 0.6).abs() < 0.001);
            }
            other => panic!("expected device RGB color, got {other:?}"),
        }
    }

    #[test]
    fn bare_device_rgb_overlong_components_is_invalid_not_truncated() {
        assert_eq!(
            resolve_named_color(&name("DeviceRGB"), &[0.2, 0.4, 0.6, 0.8], 1.0, &reader()),
            NamedColor::Invalid(INVALID_TINT_COMPONENTS)
        );
    }

    #[test]
    fn bare_calrgb_family_is_invalid_not_defaulted() {
        assert_eq!(
            resolve_named_color(&name("CalRGB"), &[0.2, 0.4, 0.6], 1.0, &reader()),
            NamedColor::Invalid(INVALID_CAL_RGB)
        );
    }

    #[test]
    fn calrgb_overlong_color_space_array_is_invalid_named_color() {
        let space = PdfObject::Array(vec![
            name("CalRGB"),
            dict_obj(&[("WhitePoint", real_arr(&[1.0, 1.0, 1.0]))]),
            name("Ignored"),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.2, 0.4, 0.6], 1.0, &reader()),
            NamedColor::Invalid(INVALID_CAL_RGB)
        );
    }

    #[test]
    fn valid_calrgb_named_color_uses_calibrated_params() {
        let space = PdfObject::Array(vec![
            name("CalRGB"),
            dict_obj(&[
                ("WhitePoint", real_arr(&[1.0, 1.0, 1.0])),
                ("Gamma", real_arr(&[1.0, 1.0, 1.0])),
            ]),
        ]);
        match resolve_named_color(&space, &[0.2, 0.4, 0.6], 1.0, &reader()) {
            NamedColor::Color(color) => {
                assert!(color.r.is_finite());
                assert!(color.g.is_finite());
                assert!(color.b.is_finite());
            }
            other => panic!("expected calibrated Color, got {other:?}"),
        }
    }

    #[test]
    fn indexed_device_rgb_palette_resolves_exact_integer_index() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            name("DeviceRGB"),
            PdfObject::Integer(1),
            PdfObject::String(vec![255, 0, 0, 0, 0, 255]),
        ]);
        match resolve_named_color(&space, &[1.0], 1.0, &reader()) {
            NamedColor::Color(color) => {
                assert!(color.r < 0.01, "R: {}", color.r);
                assert!(color.g < 0.01, "G: {}", color.g);
                assert!(color.b > 0.99, "B: {}", color.b);
            }
            other => panic!("expected Indexed DeviceRGB color, got {other:?}"),
        }
    }

    #[test]
    fn indexed_device_cmyk_palette_reuses_device_cmyk_conversion() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            name("DeviceCMYK"),
            PdfObject::Integer(1),
            PdfObject::String(vec![0, 0, 0, 0, 0, 255, 255, 0]),
        ]);
        match resolve_named_color(&space, &[1.0], 1.0, &reader()) {
            NamedColor::Color(color) => {
                assert!(color.r > 0.90, "R: {}", color.r);
                assert!(color.g < 0.15, "G: {}", color.g);
                assert!(color.b < 0.18, "B: {}", color.b);
            }
            other => panic!("expected Indexed DeviceCMYK color, got {other:?}"),
        }
    }

    #[test]
    fn indexed_calgray_palette_resolves_calibrated_base() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            PdfObject::Array(vec![
                name("CalGray"),
                dict_obj(&[("WhitePoint", real_arr(&[1.0, 1.0, 1.0]))]),
            ]),
            PdfObject::Integer(0),
            PdfObject::String(vec![128]),
        ]);
        match resolve_named_color(&space, &[0.0], 1.0, &reader()) {
            NamedColor::Color(color) => {
                assert!(color.r.is_finite());
                assert!(color.g.is_finite());
                assert!(color.b.is_finite());
            }
            other => panic!("expected Indexed CalGray color, got {other:?}"),
        }
    }

    #[test]
    fn indexed_calrgb_palette_resolves_calibrated_base() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            PdfObject::Array(vec![
                name("CalRGB"),
                dict_obj(&[
                    ("WhitePoint", real_arr(&[1.0, 1.0, 1.0])),
                    ("Gamma", real_arr(&[1.0, 1.0, 1.0])),
                ]),
            ]),
            PdfObject::Integer(0),
            PdfObject::String(vec![255, 0, 0]),
        ]);
        match resolve_named_color(&space, &[0.0], 1.0, &reader()) {
            NamedColor::Color(color) => {
                assert!(color.r.is_finite());
                assert!(color.g.is_finite());
                assert!(color.b.is_finite());
            }
            other => panic!("expected Indexed CalRGB color, got {other:?}"),
        }
    }

    #[test]
    fn indexed_lab_palette_decodes_lookup_bytes_through_lab_range() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            PdfObject::Array(vec![
                name("Lab"),
                dict_obj(&[
                    ("WhitePoint", real_arr(&[1.0, 1.0, 1.0])),
                    ("Range", real_arr(&[-100.0, 100.0, -100.0, 100.0])),
                ]),
            ]),
            PdfObject::Integer(0),
            PdfObject::String(vec![255, 128, 128]),
        ]);
        match resolve_named_color(&space, &[0.0], 1.0, &reader()) {
            NamedColor::Color(color) => {
                assert!(color.r > 0.90, "R: {}", color.r);
                assert!(color.g > 0.90, "G: {}", color.g);
                assert!(color.b > 0.90, "B: {}", color.b);
            }
            other => panic!("expected Indexed Lab color, got {other:?}"),
        }
    }

    #[test]
    fn indexed_separation_palette_resolves_tint_base() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            PdfObject::Array(vec![
                name("Separation"),
                name("SpotRed"),
                name("DeviceRGB"),
                type2_fn(&[1.0, 1.0, 1.0], &[1.0, 0.0, 0.0]),
            ]),
            PdfObject::Integer(0),
            PdfObject::String(vec![255]),
        ]);
        match resolve_named_color(&space, &[0.0], 1.0, &reader()) {
            NamedColor::Color(color) => {
                assert!(color.r > 0.99, "R: {}", color.r);
                assert!(color.g < 0.01, "G: {}", color.g);
                assert!(color.b < 0.01, "B: {}", color.b);
            }
            other => panic!("expected Indexed Separation color, got {other:?}"),
        }
    }

    #[test]
    fn indexed_devicen_palette_resolves_multi_tint_base() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            device_n_two_component_rgb_space(),
            PdfObject::Integer(0),
            PdfObject::String(vec![255, 0]),
        ]);
        match resolve_named_color(&space, &[0.0], 1.0, &reader()) {
            NamedColor::Color(color) => {
                assert!(color.r > 0.99, "R: {}", color.r);
                assert!(color.g < 0.01, "G: {}", color.g);
                assert!(color.b < 0.01, "B: {}", color.b);
            }
            other => panic!("expected Indexed DeviceN color, got {other:?}"),
        }
    }

    #[test]
    fn indexed_calrgb_malformed_base_is_invalid_not_unhandled() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            PdfObject::Array(vec![name("CalRGB"), dict_obj(&[])]),
            PdfObject::Integer(0),
            PdfObject::String(vec![255, 0, 0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.0], 1.0, &reader()),
            NamedColor::Invalid(INVALID_INDEXED)
        );
    }

    #[test]
    fn indexed_components_round_half_up_and_clip_to_the_palette() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            name("DeviceRGB"),
            PdfObject::Integer(1),
            PdfObject::String(vec![255, 0, 0, 0, 0, 255]),
        ]);
        for component in [0.5, 1.5, 300.0] {
            match resolve_named_color(&space, &[component], 1.0, &reader()) {
                NamedColor::Color(color) => assert_eq!(color.to_pixel_color(), [0, 0, 255, 255]),
                other => panic!("expected rounded/clipped blue, got {other:?}"),
            }
        }
        for component in [-5.0, 0.49] {
            match resolve_named_color(&space, &[component], 1.0, &reader()) {
                NamedColor::Color(color) => assert_eq!(color.to_pixel_color(), [255, 0, 0, 255]),
                other => panic!("expected clipped red, got {other:?}"),
            }
        }
        assert_eq!(
            resolve_named_color(&space, &[f64::NAN], 1.0, &reader()),
            NamedColor::Invalid(INVALID_INDEXED_COMPONENTS)
        );
    }

    #[test]
    fn indexed_short_lookup_is_invalid_not_black() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            name("DeviceRGB"),
            PdfObject::Integer(1),
            PdfObject::String(vec![255, 0, 0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.0], 1.0, &reader()),
            NamedColor::Invalid(INVALID_INDEXED)
        );
    }

    #[test]
    fn indexed_overlong_lookup_is_invalid_not_prefix_truncated() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            name("DeviceRGB"),
            PdfObject::Integer(1),
            PdfObject::String(vec![255, 0, 0, 0, 0, 255, 0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.0], 1.0, &reader()),
            NamedColor::Invalid(INVALID_INDEXED)
        );
    }

    #[test]
    fn indexed_overlong_color_space_array_is_invalid() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            name("DeviceRGB"),
            PdfObject::Integer(1),
            PdfObject::String(vec![255, 0, 0, 0, 0, 255]),
            name("Ignored"),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.0], 1.0, &reader()),
            NamedColor::Invalid(INVALID_INDEXED)
        );
    }

    #[test]
    fn indexed_prohibited_pattern_base_is_invalid_not_black() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            name("Pattern"),
            PdfObject::Integer(0),
            PdfObject::String(vec![0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.0], 1.0, &reader()),
            NamedColor::Invalid(INVALID_INDEXED)
        );
    }

    #[test]
    fn incomplete_icc_space_is_invalid_not_an_implicit_device_fallback() {
        let space = PdfObject::Array(vec![name("ICCBased")]);
        assert!(matches!(
            resolve_named_color(&space, &[0.5], 1.0, &reader()),
            NamedColor::Invalid(_)
        ));
    }
}
