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
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::mem::size_of;

pub(crate) const MAX_DEVICEN_COMPONENTS: usize = 16;
pub(crate) const DEFAULT_TINT_TRANSFORM_CACHE_ENTRIES: usize = 64;
pub(crate) const DEFAULT_TINT_TRANSFORM_CACHE_BYTES: usize =
    DEFAULT_TINT_TRANSFORM_CACHE_ENTRIES * MAX_TINT_TRANSFORM_OUTPUT_COMPONENTS * size_of::<f64>();
const MAX_TINT_TRANSFORM_OUTPUT_COMPONENTS: usize = 32;
const TINT_TRANSFORM_HASH_DEPTH_LIMIT: usize = 16;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TintTransformCacheKey {
    document_hash: u64,
    document_len: usize,
    function_hash: u64,
    input_count: usize,
    input_bits: [u64; MAX_DEVICEN_COMPONENTS],
}

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
    ) -> std::result::Result<Vec<f64>, NamedColor> {
        let key = tint_transform_cache_key(tint_fn, inputs, reader);
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
        let output = evaluate_tint_transform_uncached(tint_fn, inputs, reader)?;
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
        "Separation" => resolve_separation(arr, components, alpha, reader, options),
        "DeviceN" => resolve_device_n(arr, components, alpha, reader, options),
        "ICCBased" => {
            cmm::icc_components_to_srgb_with_options(space_obj, components, reader, options)
                .map(|[r, g, b]| NamedColor::Color(RenderColor::new(r, g, b, alpha)))
                .unwrap_or(NamedColor::Unhandled)
        }
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
        "Indexed" => resolve_indexed(arr, components, alpha, reader, options),
        _ => NamedColor::Unhandled,
    }
}

fn resolve_indexed(
    arr: &[PdfObject],
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
) -> NamedColor {
    if arr.len() != 4 {
        return NamedColor::Invalid(INVALID_INDEXED);
    }
    if !components_have_exact_finite_count(components, 1) {
        return NamedColor::Invalid(INVALID_INDEXED_COMPONENTS);
    }
    let hival = match arr.get(2).and_then(PdfObject::as_integer) {
        Some(value) if value >= 0 => value as usize,
        _ => return NamedColor::Invalid(INVALID_INDEXED),
    };
    let index = match indexed_component_to_index(components[0], hival) {
        Some(index) => index,
        None => return NamedColor::Invalid(INVALID_INDEXED_COMPONENTS),
    };
    let base = match arr.get(1) {
        Some(base) => base,
        None => return NamedColor::Invalid(INVALID_INDEXED),
    };
    let channels = match indexed_base_component_count(base, reader) {
        Some(channels) => channels,
        None => return NamedColor::Unhandled,
    };
    let lookup = match arr
        .get(3)
        .and_then(|lookup| indexed_lookup_bytes(lookup, reader))
    {
        Some(lookup) => lookup,
        None => return NamedColor::Invalid(INVALID_INDEXED),
    };
    let entries = match hival.checked_add(1) {
        Some(entries) => entries,
        None => return NamedColor::Invalid(INVALID_INDEXED),
    };
    let expected = match entries.checked_mul(channels) {
        Some(expected) => expected,
        None => return NamedColor::Invalid(INVALID_INDEXED),
    };
    if lookup.len() != expected {
        return NamedColor::Invalid(INVALID_INDEXED);
    }
    let start = match index.checked_mul(channels) {
        Some(start) => start,
        None => return NamedColor::Invalid(INVALID_INDEXED),
    };
    let end = match start.checked_add(channels) {
        Some(end) if end <= lookup.len() => end,
        _ => return NamedColor::Invalid(INVALID_INDEXED),
    };
    let palette_components = match indexed_palette_components(base, &lookup[start..end], reader) {
        Some(components) => components,
        None => return NamedColor::Invalid(INVALID_INDEXED),
    };
    resolve_alternate_color(base, &palette_components, alpha, reader, options)
}

fn indexed_component_to_index(component: f64, hival: usize) -> Option<usize> {
    if !component.is_finite() || component < 0.0 || component > hival as f64 {
        return None;
    }
    let rounded = component.round();
    ((component - rounded).abs() <= 1e-9).then_some(rounded as usize)
}

fn indexed_lookup_bytes(lookup: &PdfObject, reader: &PdfReader) -> Option<Vec<u8>> {
    match lookup {
        PdfObject::String(bytes) => Some(bytes.clone()),
        PdfObject::Stream { raw, .. } => Some(raw.clone()),
        PdfObject::Reference { number, generation } => {
            match reader.get_object(*number, *generation).ok()? {
                PdfObject::String(bytes) => Some(bytes),
                PdfObject::Stream { raw, .. } => Some(raw),
                _ => None,
            }
        }
        _ => None,
    }
}

fn indexed_base_component_count(base: &PdfObject, reader: &PdfReader) -> Option<usize> {
    let resolved = match base {
        PdfObject::Reference { .. } => reader.resolve(base.clone()).ok()?,
        other => other.clone(),
    };
    match &resolved {
        PdfObject::Name(name) => indexed_base_family_component_count(name, &resolved, reader),
        PdfObject::Array(items) => {
            let head = items.first().and_then(PdfObject::as_name)?;
            indexed_base_family_component_count(head, &resolved, reader)
        }
        _ => None,
    }
}

fn indexed_base_family_component_count(
    space_name: &str,
    space: &PdfObject,
    reader: &PdfReader,
) -> Option<usize> {
    match space_name {
        "DeviceGray" | "G" | "CalGray" | "Separation" => Some(1),
        "DeviceRGB" | "RGB" | "sRGB" | "CalRGB" | "Lab" => Some(3),
        "DeviceCMYK" | "CMYK" => Some(4),
        "ICCBased" => indexed_iccbased_component_count(space, reader),
        "DeviceN" => indexed_device_n_component_count(space),
        _ => None,
    }
}

fn indexed_iccbased_component_count(space: &PdfObject, reader: &PdfReader) -> Option<usize> {
    let arr = space.as_array()?;
    if arr.first().and_then(PdfObject::as_name) != Some("ICCBased") {
        return None;
    }
    let profile = reader.resolve(arr.get(1)?.clone()).ok()?;
    profile
        .as_stream()
        .and_then(|(dict, _)| dict.get_integer("N"))
        .and_then(|n| (1..=4).contains(&n).then_some(n as usize))
}

fn indexed_device_n_component_count(space: &PdfObject) -> Option<usize> {
    let arr = space.as_array()?;
    if arr.first().and_then(PdfObject::as_name) != Some("DeviceN") {
        return None;
    }
    let names = arr.get(1)?.as_array()?;
    if names.is_empty()
        || names.len() > MAX_DEVICEN_COMPONENTS
        || names.iter().any(|name| name.as_name().is_none())
    {
        return None;
    }
    Some(names.len())
}

fn indexed_palette_components(
    base: &PdfObject,
    samples: &[u8],
    reader: &PdfReader,
) -> Option<Vec<f64>> {
    let resolved = match base {
        PdfObject::Reference { .. } => reader.resolve(base.clone()).ok()?,
        other => other.clone(),
    };
    let family = match &resolved {
        PdfObject::Name(name) => name.as_str(),
        PdfObject::Array(items) => items.first()?.as_name()?,
        _ => return None,
    };
    if family == "Lab" {
        return indexed_lab_palette_components(&resolved, samples, reader);
    }
    Some(
        samples
            .iter()
            .map(|sample| f64::from(*sample) / 255.0)
            .collect(),
    )
}

fn indexed_lab_palette_components(
    space: &PdfObject,
    samples: &[u8],
    reader: &PdfReader,
) -> Option<Vec<f64>> {
    if samples.len() != 3 {
        return None;
    }
    let params = strict_lab_params_from_space(space, reader)?;
    Some(vec![
        f64::from(samples[0]) * 100.0 / 255.0,
        f64::from(params.range[0])
            + f64::from(samples[1]) * f64::from(params.range[1] - params.range[0]) / 255.0,
        f64::from(params.range[2])
            + f64::from(samples[2]) * f64::from(params.range[3] - params.range[2]) / 255.0,
    ])
}

/// `[/Separation /Name altSpace tintTransform]`
fn resolve_separation(
    arr: &[PdfObject],
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
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

    let alt_components = match evaluate_tint_transform(tint_fn, &[tint], reader) {
        Ok(components) => components,
        Err(invalid) => return invalid,
    };
    resolve_alternate_color(alt, &alt_components, alpha, reader, options)
}

/// `[/DeviceN [/Name1 /Name2 ...] altSpace tintTransform attributes?]`
fn resolve_device_n(
    arr: &[PdfObject],
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
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
    let alt_components = match evaluate_tint_transform(tint_fn, components, reader) {
        Ok(components) => components,
        Err(invalid) => return invalid,
    };
    resolve_alternate_color(alt, &alt_components, alpha, reader, options)
}

fn evaluate_tint_transform(
    tint_fn: &PdfObject,
    inputs: &[f64],
    reader: &PdfReader,
) -> std::result::Result<Vec<f64>, NamedColor> {
    TINT_TRANSFORM_CACHE.with(|cache| cache.borrow_mut().evaluate(tint_fn, inputs, reader))
}

fn evaluate_tint_transform_uncached(
    tint_fn: &PdfObject,
    inputs: &[f64],
    reader: &PdfReader,
) -> std::result::Result<Vec<f64>, NamedColor> {
    if !tint_transform_accepts_input_count(tint_fn, inputs.len(), reader) {
        return Err(NamedColor::Invalid(INVALID_TINT_FUNCTION));
    }
    if !crate::render::function::validate_function_shape(tint_fn, inputs.len(), reader) {
        return Err(NamedColor::Invalid(INVALID_TINT_FUNCTION));
    }
    let alt_components = crate::render::function::eval_function_n(tint_fn, inputs, reader);
    if alt_components.is_empty() {
        Err(NamedColor::Invalid(INVALID_TINT_FUNCTION))
    } else {
        Ok(alt_components)
    }
}

fn tint_transform_cache_key(
    tint_fn: &PdfObject,
    inputs: &[f64],
    reader: &PdfReader,
) -> Option<TintTransformCacheKey> {
    if inputs.len() > MAX_DEVICEN_COMPONENTS || inputs.iter().any(|input| !input.is_finite()) {
        return None;
    }
    let mut input_bits = [0u64; MAX_DEVICEN_COMPONENTS];
    for (idx, input) in inputs.iter().enumerate() {
        input_bits[idx] = input.to_bits();
    }
    Some(TintTransformCacheKey {
        document_hash: stable_hash(reader.file_bytes()),
        document_len: reader.file_bytes().len(),
        function_hash: tint_function_hash(tint_fn, reader),
        input_count: inputs.len(),
        input_bits,
    })
}

fn tint_function_hash(tint_fn: &PdfObject, reader: &PdfReader) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_pdf_object_resolved(tint_fn, reader, &mut hasher, 0);
    hasher.finish()
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

fn hash_pdf_dictionary<H: Hasher>(
    dict: &PdfDictionary,
    reader: &PdfReader,
    state: &mut H,
    depth: usize,
) {
    dict.len().hash(state);
    for (key, value) in dict.entries() {
        key.hash(state);
        hash_pdf_object_resolved(value, reader, state, depth + 1);
    }
}

fn hash_pdf_object_resolved<H: Hasher>(
    object: &PdfObject,
    reader: &PdfReader,
    state: &mut H,
    depth: usize,
) {
    if depth > TINT_TRANSFORM_HASH_DEPTH_LIMIT {
        13u8.hash(state);
        return;
    }
    match object {
        PdfObject::Boolean(value) => {
            0u8.hash(state);
            value.hash(state);
        }
        PdfObject::Integer(value) => {
            1u8.hash(state);
            value.hash(state);
        }
        PdfObject::Real(value) => {
            2u8.hash(state);
            value.to_bits().hash(state);
        }
        PdfObject::String(value) => {
            3u8.hash(state);
            value.hash(state);
        }
        PdfObject::Name(value) => {
            4u8.hash(state);
            value.hash(state);
        }
        PdfObject::Array(items) => {
            5u8.hash(state);
            items.len().hash(state);
            for item in items {
                hash_pdf_object_resolved(item, reader, state, depth + 1);
            }
        }
        PdfObject::Dictionary(dict) => {
            6u8.hash(state);
            hash_pdf_dictionary(dict, reader, state, depth + 1);
        }
        PdfObject::Stream { dict, raw } => {
            7u8.hash(state);
            hash_pdf_dictionary(dict, reader, state, depth + 1);
            raw.hash(state);
        }
        PdfObject::Null => {
            8u8.hash(state);
        }
        PdfObject::Reference { number, generation } => {
            9u8.hash(state);
            number.hash(state);
            generation.hash(state);
            if depth >= TINT_TRANSFORM_HASH_DEPTH_LIMIT {
                10u8.hash(state);
                return;
            }
            match reader.resolve(object.clone()) {
                Ok(resolved) => {
                    11u8.hash(state);
                    hash_pdf_object_resolved(&resolved, reader, state, depth + 1);
                }
                Err(_) => {
                    12u8.hash(state);
                }
            }
        }
    }
}

fn tint_transform_accepts_input_count(
    tint_fn: &PdfObject,
    input_count: usize,
    reader: &PdfReader,
) -> bool {
    input_count <= 1 || !matches!(resolved_function_type(tint_fn, reader), Some(2 | 3))
}

fn resolved_function_type(func_obj: &PdfObject, reader: &PdfReader) -> Option<i64> {
    let resolved = match func_obj {
        PdfObject::Reference { .. } => reader.resolve(func_obj.clone()).ok()?,
        other => other.clone(),
    };
    match resolved {
        PdfObject::Dictionary(dict) => dict.get_integer("FunctionType"),
        PdfObject::Stream { dict, .. } => dict.get_integer("FunctionType"),
        _ => None,
    }
}

fn resolve_alternate_color(
    alt: &PdfObject,
    components: &[f64],
    alpha: f32,
    reader: &PdfReader,
    options: cmm::ColorTransformOptions,
) -> NamedColor {
    let resolved = match alt {
        PdfObject::Reference { .. } => reader.resolve(alt.clone()).unwrap_or_else(|_| alt.clone()),
        other => other.clone(),
    };
    match resolve_named_color_with_options(&resolved, components, alpha, reader, options) {
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

        let key1 = tint_transform_cache_key(&tint_fn, &[0.25], &r1).unwrap();
        let key2 = tint_transform_cache_key(&tint_fn, &[0.25], &r2).unwrap();

        assert_ne!(key1, key2);
        assert_ne!(key1.document_hash, key2.document_hash);
        assert_eq!(key1.document_len, key2.document_len);
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
            NamedColor::Invalid(INVALID_CAL_RGB)
        );
    }

    #[test]
    fn indexed_non_integer_component_is_invalid_not_rounded() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            name("DeviceRGB"),
            PdfObject::Integer(1),
            PdfObject::String(vec![255, 0, 0, 0, 0, 255]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.5], 1.0, &reader()),
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
    fn indexed_unsupported_base_is_unhandled_not_black() {
        let space = PdfObject::Array(vec![
            name("Indexed"),
            name("Pattern"),
            PdfObject::Integer(0),
            PdfObject::String(vec![0]),
        ]);
        assert_eq!(
            resolve_named_color(&space, &[0.0], 1.0, &reader()),
            NamedColor::Unhandled
        );
    }

    #[test]
    fn non_special_space_is_unhandled() {
        let space = PdfObject::Array(vec![name("ICCBased")]);
        assert_eq!(
            resolve_named_color(&space, &[0.5], 1.0, &reader()),
            NamedColor::Unhandled
        );
    }
}
