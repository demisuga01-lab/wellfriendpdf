//! PDF shading and function evaluation.
//!
//! Implements the common native shading families:
//!
//! - PDF Functions: Type 0, 2, 3, and bounded Type 4 calculator evaluation.
//! - Function-based shading (ShadingType 1).
//! - Axial and radial shadings (ShadingTypes 2 and 3).
//!
//! - Gouraud and patch mesh shadings (ShadingTypes 4-7), with bounded stream
//!   decoding, Coons surfaces, and Type 7 tensor-product interpolation.
//!
//! Rendering is pixel-by-pixel: for each device pixel we map back to user
//! space, project onto the gradient geometry to obtain the parametric value
//! `t`, evaluate the colour function, and blend. The pre-existing clip mask
//! bounds the painted region, so `sh` and shading-pattern fills only colour
//! the intended area.

use crate::cancel::CancelToken;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::render::buffer::{PixelBuffer, PixelColor};
use crate::render::cmm::ColorTransformOptions;
use crate::render::color::{ColorSpaceHandler, RenderColor};
use crate::render::transform::{Transform2D, Viewport};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const MAX_SHADING_WORK_UNITS: u64 = 64 * 1024 * 1024;

#[cfg(test)]
#[path = "shading_color_context_tests.rs"]
mod color_context_tests;

#[cfg(test)]
#[path = "function_resource_render_tests.rs"]
mod function_resource_tests;

#[path = "shading_mesh.rs"]
mod mesh;
use mesh::{MeshPaint, MeshSample, MeshVertex};

#[path = "shading_region.rs"]
mod region;
use region::{CommonEntries, PaintRegion};

#[path = "shading_geometry.rs"]
mod geometry;
pub(crate) use geometry::domain_value as shading_domain_value;
use geometry::{AxialGeometry, RadialGeometry};

const DEFAULT_SHADING_WORKING_BYTES: usize = 64 * 1024 * 1024;

/// A minimal valid PDF used by render tests that need a `PdfReader` but never
/// resolve indirect objects. Crate-visible so sibling render modules can reuse
/// it for function/shading tests.
#[cfg(test)]
pub(crate) fn tests_minimal_pdf() -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut off = [0usize; 4];
    off[1] = pdf.len();
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    off[2] = pdf.len();
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    off[3] = pdf.len();
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>\nendobj\n",
    );
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for offset in off.iter().take(4).skip(1) {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    pdf
}

// ---------------------------------------------------------------------------
// PDF function evaluation
// ---------------------------------------------------------------------------

/// Evaluate a single-input PDF function object at input `t`, returning its
/// output components. Delegates to the multi-input dispatcher in
/// [`crate::render::function`], which supports Function Types 0, 2, 3, and 4.
/// Returns an empty `Vec` for unsupported types or malformed input.
#[cfg(test)]
pub(crate) fn eval_function(func_obj: &PdfObject, t: f64, reader: &PdfReader) -> Vec<f64> {
    crate::render::function::eval_function_or_array_n(func_obj, &[t], reader)
}

/// Type 2 (exponential interpolation): `f(t) = C0 + t^N * (C1 - C0)`.
#[cfg(test)]
pub(crate) fn eval_type2(dict: &PdfDictionary, t: f64) -> Vec<f64> {
    let Some(n) = dict.get("N").and_then(PdfObject::as_number) else {
        return Vec::new();
    };
    if !n.is_finite() || !t.is_finite() {
        return Vec::new();
    }

    let Some(domain) = get_strict_float_array(dict, "Domain").filter(|values| values.len() == 2)
    else {
        return Vec::new();
    };
    let d0 = domain[0];
    let d1 = domain[1];
    if d0 > d1 || (n.fract() != 0.0 && d0 < 0.0) || (n < 0.0 && d0 <= 0.0 && d1 >= 0.0) {
        return Vec::new();
    }
    // PDF Type 2 evaluates the clipped input itself, not its normalized
    // position in Domain. C0/C1 describe x=0/x=1 even for a non-unit domain.
    let t_clamped = t.clamp(d0, d1);

    let c0 = match dict.contains_key("C0") {
        true => match get_strict_float_array(dict, "C0") {
            Some(values) if !values.is_empty() => values,
            _ => return Vec::new(),
        },
        false => vec![0.0],
    };
    let c1 = match dict.contains_key("C1") {
        true => match get_strict_float_array(dict, "C1") {
            Some(values) if !values.is_empty() => values,
            _ => return Vec::new(),
        },
        false => vec![1.0],
    };
    if c0.len() != c1.len() {
        return Vec::new();
    }
    let Some(output) = exponential_components(&c0, &c1, n, t_clamped) else {
        return Vec::new();
    };
    crate::render::function::clip_output(dict, output).unwrap_or_default()
}

/// Shared arithmetic for direct and prepared functions. Domain/shape validation
/// belongs to their callers; Range is applied after these source components.
pub(super) fn exponential_components(
    c0: &[f64],
    c1: &[f64],
    exponent: f64,
    input: f64,
) -> Option<Vec<f64>> {
    if c0.is_empty() || c0.len() != c1.len() {
        return None;
    }
    let factor = input.powf(exponent);
    if !factor.is_finite() {
        return None;
    }
    let output: Vec<f64> = (0..c0.len())
        .map(|i| {
            let v0 = c0[i];
            let v1 = c1[i];
            if (0.0..=1.0).contains(&factor) {
                geometry::domain_value(v0, v1, factor)
            } else {
                factor.mul_add(v1 - v0, v0)
            }
        })
        .collect();
    output.iter().all(|v| v.is_finite()).then_some(output)
}

/// Type 3 (stitching): selects a sub-function by breakpoint and re-encodes `t`.
#[cfg(test)]
pub(crate) fn eval_type3(dict: &PdfDictionary, t: f64, reader: &PdfReader) -> Vec<f64> {
    if !t.is_finite() {
        return Vec::new();
    }
    let Some(domain) = get_strict_float_array(dict, "Domain").filter(|values| values.len() == 2)
    else {
        return Vec::new();
    };
    let funcs = match dict.get("Functions") {
        Some(PdfObject::Array(arr)) => arr,
        _ => return Vec::new(),
    };
    if funcs.is_empty() {
        return Vec::new();
    }
    let Some(bounds) =
        get_strict_float_array(dict, "Bounds").filter(|values| values.len() + 1 == funcs.len())
    else {
        return Vec::new();
    };
    let Some(encode) =
        get_strict_float_array(dict, "Encode").filter(|values| values.len() == funcs.len() * 2)
    else {
        return Vec::new();
    };

    let d0 = domain[0];
    let d1 = domain[1];
    if d0 > d1 || (funcs.len() > 1 && d0 == d1) {
        return Vec::new();
    }
    let mut previous = d0;
    for &bound in &bounds {
        if bound <= previous || bound > d1 {
            return Vec::new();
        }
        previous = bound;
    }
    let t = t.clamp(d0, d1);

    // Find the sub-function index: first bound the value falls below.
    let idx = {
        let mut found = funcs.len() - 1;
        for (i, &bound) in bounds.iter().enumerate() {
            if t < bound {
                found = i;
                break;
            }
        }
        found.min(funcs.len() - 1)
    };

    // The segment [seg_start, seg_end) this index maps to in the domain.
    let (seg_start, seg_end) = if bounds.is_empty() {
        (d0, d1)
    } else {
        let s = if idx == 0 { d0 } else { bounds[idx - 1] };
        let e = if idx + 1 == funcs.len() {
            d1
        } else {
            bounds[idx]
        };
        (s, e)
    };

    let e0 = encode[idx * 2];
    let e1 = encode[idx * 2 + 1];
    let Some(position) = crate::render::function::domain_position(t, seg_start, seg_end) else {
        return Vec::new();
    };
    let t_enc = geometry::domain_value(e0, e1, position);

    match funcs.get(idx) {
        Some(sub) => crate::render::function::clip_output(dict, eval_function(sub, t_enc, reader))
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Read a finite numeric array from `dict[key]`, returning `None` if absent,
/// empty, or malformed.
pub(crate) fn get_float_array(dict: &PdfDictionary, key: &str) -> Option<Vec<f64>> {
    let vals = get_strict_float_array(dict, key)?;
    if vals.is_empty() {
        None
    } else {
        Some(vals)
    }
}

/// Read a 2-element boolean array (e.g. `/Extend [bool bool]`).
pub(crate) fn get_bool_pair(dict: &PdfDictionary, key: &str) -> Option<[bool; 2]> {
    let arr = dict.get(key)?.as_array()?;
    if arr.len() != 2 {
        return None;
    }
    Some([arr[0].as_bool()?, arr[1].as_bool()?])
}

fn charge_shading_work(buf: &PixelBuffer, work_budget: &AtomicU64) -> Result<(), String> {
    let Some((x_start, y_start, x_end, y_end)) = ShadingRenderer::paint_bounds(buf) else {
        return Ok(());
    };
    let width =
        u64::try_from(x_end - x_start).map_err(|_| "shading work width is invalid".to_string())?;
    let height =
        u64::try_from(y_end - y_start).map_err(|_| "shading work height is invalid".to_string())?;
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| "shading work area overflows the per-render work budget".to_string())?;
    charge_shading_units(pixels, work_budget)
}

fn charge_shading_units(units: u64, work_budget: &AtomicU64) -> Result<(), String> {
    work_budget
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
            remaining.checked_sub(units)
        })
        .map(|_| ())
        .map_err(|remaining| {
            format!(
                "shading work requires {units} units with {remaining} remaining; per-render limit is {MAX_SHADING_WORK_UNITS}"
            )
        })
}

/// Convert shading function output components to an opaque pixel colour.
#[cfg(test)]
pub(crate) fn components_to_pixel(components: &[f64], color_space: &str) -> PixelColor {
    components_to_render_color(components, color_space).to_pixel_color()
}

/// Convert shading function output components to a float render colour.
#[cfg(test)]
pub(crate) fn components_to_render_color(components: &[f64], color_space: &str) -> RenderColor {
    ColorSpaceHandler::from_components(color_space, components, 1.0)
}

fn components_to_render_color_with_space(
    components: &[f64],
    color_space: &str,
    color_space_obj: Option<&PdfObject>,
    reader: &PdfReader,
    options: ShadingRenderOptions<'_>,
) -> Option<RenderColor> {
    if matches!(color_space, "Indexed" | "I")
        && (components.len() != 1 || !components[0].is_finite() || components[0].fract() != 0.0)
    {
        options.fail("Indexed shading function produced a non-integer palette index");
        return None;
    }
    if let Some(space_obj) = color_space_obj {
        match crate::render::colorspace::resolve_named_color_with_resources(
            space_obj,
            options.source_color_space,
            components,
            1.0,
            reader,
            options.color_transform,
            options.function_resources(),
        ) {
            crate::render::colorspace::NamedColor::Color(color) => return Some(color),
            crate::render::colorspace::NamedColor::NoPaint => {
                return Some(RenderColor::transparent());
            }
            crate::render::colorspace::NamedColor::Invalid(reason) => {
                options.fail(reason);
                log::warn!("shading color-space rejected: {reason}");
                return None;
            }
            crate::render::colorspace::NamedColor::Unhandled => {
                options.fail("unsupported shading colour space");
                log::warn!("shading color-space unsupported: {color_space}");
                return None;
            }
        }
    }
    let color = ColorSpaceHandler::try_from_components(color_space, components, 1.0);
    if color.is_none() {
        options.fail("invalid shading colour components");
    }
    color
}

/// Read the shading's colour-space name. Handles both a bare name and an array
/// whose first element names the family (e.g. `[/ICCBased N 0 R]`).
fn shading_color_space_name(dict: &PdfDictionary) -> Option<String> {
    match dict.get("ColorSpace").or_else(|| dict.get("CS"))? {
        PdfObject::Name(name) => Some(name.clone()),
        PdfObject::Array(arr) => arr.first().and_then(PdfObject::as_name).map(str::to_string),
        _ => None,
    }
}

fn shading_color_space_object(dict: &PdfDictionary) -> Option<&PdfObject> {
    dict.get("ColorSpace").or_else(|| dict.get("CS"))
}

// ---------------------------------------------------------------------------
// Shading renderer
// ---------------------------------------------------------------------------

const SHADING_LUT_STEPS: usize = 4096;
const MAX_PATCH_MESH_PATCHES: usize = 4096;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ShadingRenderOptions<'a> {
    /// PDF graphics-state `/SM` smoothness tolerance, normalized to [0, 1].
    /// This colour smoothness hint is separate from patch geometry accuracy.
    smoothness_tolerance: f64,
    pub(crate) source_color_space: Option<&'a PdfObject>,
    pub(crate) color_transform: ColorTransformOptions,
    failure: Option<&'a Cell<Option<&'static str>>>,
    work_budget: Option<&'a AtomicU64>,
    working_byte_limit: usize,
    function_graph_byte_limit: usize,
    function_stream_byte_limit: usize,
    color_cache_entries: usize,
    function_cache: Option<&'a std::sync::Mutex<crate::render::function::FunctionCache>>,
    memory_budget: Option<&'a std::sync::Arc<crate::decode_scheduler::DecodeMemoryBudget>>,
    opacity: f32,
    dither_origin: (i32, i32),
    use_background: bool,
    region: Option<&'a PaintRegion>,
    sample_clip: Option<&'a crate::render::buffer::ClipMask>,
    sample_clip_origin: (i32, i32),
}

impl<'a> ShadingRenderOptions<'a> {
    pub(crate) fn new(smoothness_tolerance: f64) -> Self {
        Self {
            smoothness_tolerance: normalize_smoothness_tolerance(smoothness_tolerance),
            ..Self::default()
        }
    }
    pub(crate) fn with_color_context<'b>(
        self,
        source: Option<&'b PdfObject>,
        transform: ColorTransformOptions,
    ) -> ShadingRenderOptions<'b>
    where
        'a: 'b,
    {
        ShadingRenderOptions {
            smoothness_tolerance: self.smoothness_tolerance,
            source_color_space: source,
            color_transform: transform,
            failure: self.failure,
            work_budget: self.work_budget,
            working_byte_limit: self.working_byte_limit,
            function_graph_byte_limit: self.function_graph_byte_limit,
            function_stream_byte_limit: self.function_stream_byte_limit,
            color_cache_entries: self.color_cache_entries,
            function_cache: self.function_cache,
            memory_budget: self.memory_budget,
            opacity: self.opacity,
            dither_origin: self.dither_origin,
            use_background: self.use_background,
            region: self.region,
            sample_clip: self.sample_clip,
            sample_clip_origin: self.sample_clip_origin,
        }
    }
    pub(crate) fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
    pub(crate) fn for_pattern(mut self) -> Self {
        self.use_background = true;
        self
    }
    fn sample_visible(self, x: i32, y: i32) -> bool {
        if self.sample_clip.is_some_and(|clip| {
            clip.opacity(x + self.sample_clip_origin.0, y + self.sample_clip_origin.1) <= 0.0
        }) {
            return false;
        }
        match self.region.map(|region| region.coverage(x, y)).transpose() {
            Ok(value) => value.unwrap_or(1.0) > 0.0,
            Err(reason) => {
                self.fail(reason);
                false
            }
        }
    }
    pub(crate) fn with_working_byte_limit(mut self, bytes: usize) -> Self {
        self.working_byte_limit = bytes;
        self
    }
    pub(crate) fn with_function_graph_byte_limit(mut self, bytes: usize) -> Self {
        self.function_graph_byte_limit = bytes;
        self
    }
    pub(crate) fn with_function_stream_byte_limit(mut self, bytes: usize) -> Self {
        self.function_stream_byte_limit = bytes;
        self
    }
    pub(crate) fn with_function_cache(
        mut self,
        cache: &'a std::sync::Mutex<crate::render::function::FunctionCache>,
    ) -> Self {
        self.function_cache = Some(cache);
        self
    }
    pub(crate) fn function_resources(self) -> crate::render::function::FunctionResources<'a> {
        crate::render::function::FunctionResources {
            memory: self.memory_budget,
            max_graph_bytes: self.function_graph_byte_limit,
            max_stream_bytes: self.function_stream_byte_limit,
            cache: self.function_cache,
        }
    }
    pub(crate) fn prepare_function(
        self,
        object: &PdfObject,
        inputs: usize,
        reader: &PdfReader,
    ) -> Option<crate::render::function::FunctionLease> {
        crate::render::function::PreparedFunction::cached_with_resources(
            object,
            inputs,
            true,
            reader,
            self.function_resources(),
        )
    }
    pub(crate) fn with_memory_budget(
        mut self,
        budget: &'a std::sync::Arc<crate::decode_scheduler::DecodeMemoryBudget>,
    ) -> Self {
        self.memory_budget = Some(budget);
        self
    }
    fn reserve_bytes(
        self,
        bytes: usize,
    ) -> Result<Option<crate::decode_scheduler::DecodeMemoryToken>, String> {
        if bytes > self.working_byte_limit {
            return Err("shading working-memory budget exceeded".into());
        }
        self.memory_budget
            .map(|budget| {
                budget
                    .try_acquire(bytes as u64)
                    .map_err(|error| error.to_string())
            })
            .transpose()
    }
    fn charge_work(self, units: u64) -> bool {
        if let Some(budget) = self.work_budget {
            if charge_shading_units(units, budget).is_err() {
                self.fail("shading cumulative work budget exhausted");
                return false;
            }
        }
        true
    }
    fn evaluate_function(
        self,
        function: &crate::render::function::PreparedFunction,
        inputs: &[f64],
    ) -> Vec<f64> {
        let allowance = self.work_budget.map_or(usize::MAX, |budget| {
            usize::try_from(budget.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(usize::MAX)
        });
        let (values, consumed) = function.evaluate_metered(inputs, allowance);
        if !self.charge_work(consumed as u64) {
            return Vec::new();
        }
        values
    }
    fn reserve_mesh_storage(
        self,
        vertices: usize,
    ) -> Result<Option<crate::decode_scheduler::DecodeMemoryToken>, String> {
        let bytes = vertices
            .checked_mul(std::mem::size_of::<MeshVertex>())
            .ok_or("mesh storage size overflow")?;
        self.reserve_bytes(bytes)
    }
    fn fail(self, reason: &'static str) {
        if let Some(failure) = self.failure {
            if failure.get().is_none() {
                failure.set(Some(reason));
            }
        }
    }
    fn failed(self) -> bool {
        self.failure.is_some_and(|failure| failure.get().is_some())
    }
}

impl Default for ShadingRenderOptions<'_> {
    fn default() -> Self {
        Self {
            smoothness_tolerance: 0.0,
            source_color_space: None,
            color_transform: ColorTransformOptions::default(),
            failure: None,
            work_budget: None,
            working_byte_limit: DEFAULT_SHADING_WORKING_BYTES,
            function_graph_byte_limit: 64 * 1024 * 1024,
            function_stream_byte_limit: 16 * 1024 * 1024,
            color_cache_entries: SHADING_LUT_STEPS + 1,
            function_cache: None,
            memory_budget: None,
            opacity: 1.0,
            dither_origin: (0, 0),
            use_background: false,
            region: None,
            sample_clip: None,
            sample_clip_origin: (0, 0),
        }
    }
}

pub struct ShadingRenderer;

pub(crate) fn validate_common_shading_entries(
    dict: &PdfDictionary,
    reader: &PdfReader,
    options: ShadingRenderOptions<'_>,
) -> Result<(), String> {
    let common = CommonEntries::read(dict, reader)?;
    if let Some(background) = common.background {
        let name =
            shading_color_space_name(dict).ok_or("shading background has no colour space")?;
        let space = shading_color_space_object(dict);
        let expected = color_space_component_count(&name, space, reader)
            .ok_or("shading background component count is unknown")?;
        if background.len() != expected {
            return Err(format!(
                "shading /Background requires {expected} components, got {}",
                background.len()
            ));
        }
        if options.use_background
            && components_to_render_color_with_space(&background, &name, space, reader, options)
                .is_none()
        {
            return Err("shading background colour conversion failed".into());
        }
    }
    Ok(())
}

const BAYER_8X8: [u8; 64] = [
    0, 48, 12, 60, 3, 51, 15, 63, 32, 16, 44, 28, 35, 19, 47, 31, 8, 56, 4, 52, 11, 59, 7, 55, 40,
    24, 36, 20, 43, 27, 39, 23, 2, 50, 14, 62, 1, 49, 13, 61, 34, 18, 46, 30, 33, 17, 45, 29, 10,
    58, 6, 54, 9, 57, 5, 53, 42, 26, 38, 22, 41, 25, 37, 21,
];

#[derive(Debug, Clone)]
struct ShadingColorCache {
    entries: Vec<Option<ShadingColorCacheEntry>>,
}

#[derive(Debug, Clone, Copy)]
struct ShadingColorCacheEntry {
    bucket: usize,
    parameter: u64,
    color: RenderColor,
}

impl ShadingColorCache {
    #[cfg(test)]
    fn new() -> Self {
        Self::new_bounded(SHADING_LUT_STEPS + 1)
    }

    fn new_bounded(entries: usize) -> Self {
        Self {
            entries: vec![None; entries.min(SHADING_LUT_STEPS + 1)],
        }
    }

    #[inline]
    fn bucket(s: f64) -> usize {
        (s.clamp(0.0, 1.0) * SHADING_LUT_STEPS as f64).round() as usize
    }

    #[inline]
    fn get(&self, s: f64) -> Option<RenderColor> {
        if self.entries.is_empty() {
            return None;
        }
        let bucket = Self::bucket(s);
        self.entries
            .get(bucket % self.entries.len())
            .and_then(|entry| *entry)
            .filter(|entry| entry.bucket == bucket && entry.parameter == s.to_bits())
            .map(|entry| entry.color)
    }

    #[inline]
    fn set(&mut self, s: f64, color: RenderColor) {
        if self.entries.is_empty() {
            return;
        }
        let bucket = Self::bucket(s);
        let slot_index = bucket % self.entries.len();
        if let Some(slot) = self.entries.get_mut(slot_index) {
            *slot = Some(ShadingColorCacheEntry {
                bucket,
                parameter: s.to_bits(),
                color,
            });
        }
    }
}

#[inline]
fn ordered_dither_offset(x: i32, y: i32) -> f32 {
    let xi = x.rem_euclid(8) as usize;
    let yi = y.rem_euclid(8) as usize;
    (BAYER_8X8[yi * 8 + xi] as f32 + 0.5) / 64.0 - 0.5
}

#[inline]
fn quantize_shading_channel(value: f32, dither_offset: f32, dither: bool) -> u8 {
    let scaled = value.clamp(0.0, 1.0) * 255.0;
    let adjusted = if dither {
        scaled + dither_offset
    } else {
        scaled
    };
    adjusted.round().clamp(0.0, 255.0) as u8
}

#[inline]
fn quantize_shading_color(color: RenderColor, x: i32, y: i32, dither: bool) -> PixelColor {
    let offset = if dither {
        ordered_dither_offset(x, y)
    } else {
        0.0
    };
    [
        quantize_shading_channel(color.r, offset, dither),
        quantize_shading_channel(color.g, offset, dither),
        quantize_shading_channel(color.b, offset, dither),
        (color.a.clamp(0.0, 1.0) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn invert_transform_or_decline(transform: &Transform2D, label: &str) -> Option<Transform2D> {
    if !transform.to_array().iter().all(|value| value.is_finite()) {
        log::warn!("{label}: non-finite transform");
        return None;
    }
    let Some(inverse) = geometry::inverse(transform) else {
        log::warn!("{label}: singular or unrepresentable inverse transform");
        return None;
    };
    if !inverse.to_array().iter().all(|value| value.is_finite()) {
        log::warn!("{label}: non-finite inverse transform");
        return None;
    }
    Some(inverse)
}

impl ShadingRenderer {
    fn paint_bounds(buf: &PixelBuffer) -> Option<(i32, i32, i32, i32)> {
        match buf.clip_mask().and_then(|clip| clip.visible_bounds()) {
            Some((x0, y0, x1, y1)) => Some((
                x0.max(0).min(buf.width as i32),
                y0.max(0).min(buf.height as i32),
                x1.max(0).min(buf.width as i32),
                y1.max(0).min(buf.height as i32),
            )),
            None if buf.clip_mask().is_some() => None,
            None => Some((0, 0, buf.width as i32, buf.height as i32)),
        }
        .filter(|(x0, y0, x1, y1)| x1 > x0 && y1 > y0)
    }

    /// Paint a shading dictionary into `buf`, bounded by the buffer's current
    /// clip mask. `ctm` is the current user-space → media-box transform.
    /// `mesh_data` is the shading's decoded stream body, required for mesh
    /// shadings (Types 4–7 store vertex/patch data in the stream); it is `None`
    /// for dictionary-only shadings (Types 1–3).
    pub fn paint(
        shading_dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
        mesh_data: Option<&[u8]>,
    ) {
        Self::paint_with_options(
            shading_dict,
            ctm,
            viewport,
            buf,
            reader,
            mesh_data,
            ShadingRenderOptions::default(),
        );
    }

    pub(crate) fn paint_with_options(
        shading_dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
        mesh_data: Option<&[u8]>,
        options: ShadingRenderOptions<'_>,
    ) {
        let work_budget = AtomicU64::new(MAX_SHADING_WORK_UNITS);
        if let Err(reason) = Self::paint_with_options_cancellable(
            shading_dict,
            ctm,
            viewport,
            buf,
            reader,
            mesh_data,
            options,
            &CancelToken::none(),
            &work_budget,
        ) {
            log::warn!("{reason}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_with_options_cancellable(
        shading_dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
        mesh_data: Option<&[u8]>,
        options: ShadingRenderOptions<'_>,
        cancel: &CancelToken,
        work_budget: &AtomicU64,
    ) -> std::result::Result<(), String> {
        let linked_cancel =
            CancelToken::linked_pair(cancel, &crate::cancel::current_cancel_token());
        let cancel = &linked_cancel;
        cancel.scope(|| {
            cancel
                .check("shading start")
                .map_err(|error| error.to_string())?;
            // Resolve direct caller references once. Page rendering already supplies
            // scope-bound graphs; never remap that graph a second time here.
            let mut resolved_dict =
                crate::render::parameter_dictionary::shading(shading_dict, Some(reader))?
                    .into_owned();
            if let Some(space) = shading_color_space_object(shading_dict) {
                let resolved = crate::render::default_colorspace::bind_source(
                    space,
                    &crate::engine::PageResources::default(),
                    reader,
                )?;
                resolved_dict.insert("ColorSpace", resolved);
            }
            let shading_dict = &resolved_dict;
            let failure = Cell::new(None);
            let options = ShadingRenderOptions {
                failure: Some(&failure),
                work_budget: Some(work_budget),
                ..options
            };
            if !options.opacity.is_finite() || !(0.0..=1.0).contains(&options.opacity) {
                return Err("shading opacity must be finite and in 0..=1".into());
            }
            if let Err(reason) =
                crate::render::page_renderer::validate_shading_dictionary_for_paint_with_options(
                    shading_dict,
                    "direct shading",
                    reader,
                    options,
                )
            {
                log::warn!("{reason}");
                return Err(reason);
            }
            if matches!(shading_dict.get_integer("ShadingType"), Some(4..=7)) && mesh_data.is_none()
            {
                return Err("mesh shading requires decoded stream data".into());
            }
            if options.opacity == 0.0 {
                return Ok(());
            }
            let target = buf;
            let Some(target_bounds) = Self::paint_bounds(target) else {
                return Ok(());
            };
            let common = CommonEntries::read(shading_dict, reader)?;
            let region = PaintRegion::new(common.bbox, ctm, viewport, target_bounds)?;
            let Some((x0, y0, x1, y1)) = region.bounds() else {
                return Ok(());
            };
            let region = region.shifted(x0, y0);
            let background = if options.use_background {
                common
                    .background
                    .as_ref()
                    .map(|components| {
                        let name = shading_color_space_name(shading_dict)
                            .ok_or("shading background has no colour space")?;
                        components_to_render_color_with_space(
                            components,
                            &name,
                            shading_color_space_object(shading_dict),
                            reader,
                            options,
                        )
                        .ok_or("shading background conversion failed")
                    })
                    .transpose()?
            } else {
                None
            };
            let (width, height) = ((x1 - x0) as u32, (y1 - y0) as u32);
            let pixels = (width as usize)
                .checked_mul(height as usize)
                .ok_or("shading scratch area overflow")?;
            let scratch_bytes = pixels
                .checked_mul(4)
                .ok_or("shading scratch byte size overflow")?;
            // Fixed parser/vertex scratch plus the axial/radial colour table. This
            // bounds owned shading buffers, not all CMM/function caches or process RSS.
            let fixed_reserve = 4096usize;
            let fixed_base_bytes = scratch_bytes
                .checked_add(fixed_reserve)
                .ok_or("shading working size overflow")?;
            let available_cache_bytes = options
                .working_byte_limit
                .checked_sub(fixed_base_bytes)
                .ok_or("shading scratch exceeds working-memory budget")?;
            let cache_entry_bytes = std::mem::size_of::<Option<ShadingColorCacheEntry>>();
            let color_cache_entries =
                if matches!(shading_dict.get_integer("ShadingType"), Some(2 | 3)) {
                    (available_cache_bytes / cache_entry_bytes).min(SHADING_LUT_STEPS + 1)
                } else {
                    0
                };
            let cache_bytes = color_cache_entries
                .checked_mul(cache_entry_bytes)
                .ok_or("shading colour-cache size overflow")?;
            let base_bytes = fixed_base_bytes
                .checked_add(cache_bytes)
                .ok_or("shading working size overflow")?;
            let remaining_bytes = options
                .working_byte_limit
                .checked_sub(base_bytes)
                .ok_or("shading scratch exceeds working-memory budget")?;
            let _memory = options.reserve_bytes(base_bytes)?;
            // Reserve clear/composite work before touching the destination. Clip and
            // soft-mask coverage, blend mode and opacity are applied only once there.
            let scratch_passes = 2 + u64::from(region.has_bbox()) + u64::from(background.is_some());
            charge_shading_units(
                (pixels as u64)
                    .checked_mul(scratch_passes)
                    .ok_or("shading scratch work overflow")?,
                work_budget,
            )?;
            let mut scratch = PixelBuffer::try_new_transparent_with_mode(
                width,
                height,
                target.render_mode(),
                options.working_byte_limit,
            )?;
            let window = Viewport {
                width_px: width,
                height_px: height,
                origin_x_px: viewport
                    .origin_x_px
                    .checked_add(x0 as u32)
                    .ok_or("shading window x overflow")?,
                origin_y_px: viewport
                    .origin_y_px
                    .checked_add(y0 as u32)
                    .ok_or("shading window y overflow")?,
                ..viewport.clone()
            };
            let options = ShadingRenderOptions {
                working_byte_limit: remaining_bytes,
                color_cache_entries,
                dither_origin: (
                    (window.origin_x_px % 8) as i32,
                    (window.origin_y_px % 8) as i32,
                ),
                region: Some(&region),
                sample_clip: target.clip_mask(),
                sample_clip_origin: (x0, y0),
                ..options
            };
            let viewport = &window;
            let buf = &mut scratch;
            if let Some(color) = background {
                for y in 0..height as i32 {
                    cancel
                        .check("shading pattern background")
                        .map_err(|error| error.to_string())?;
                    for x in 0..width as i32 {
                        if options.sample_visible(x, y) {
                            buf.set_pixel(
                                x,
                                y,
                                quantize_shading_color(
                                    color,
                                    x + options.dither_origin.0,
                                    y + options.dither_origin.1,
                                    buf.render_mode().is_high_quality(),
                                ),
                            );
                        }
                    }
                }
            }
            match shading_dict.get_integer("ShadingType") {
                Some(1) => {
                    charge_shading_work(buf, work_budget)?;
                    Self::paint_function_based_cancellable(
                        shading_dict,
                        ctm,
                        viewport,
                        buf,
                        reader,
                        cancel,
                        options,
                    )
                }
                Some(2) => {
                    charge_shading_work(buf, work_budget)?;
                    Self::paint_axial_cancellable(
                        shading_dict,
                        ctm,
                        viewport,
                        buf,
                        reader,
                        cancel,
                        options,
                    )
                }
                Some(3) => {
                    charge_shading_work(buf, work_budget)?;
                    Self::paint_radial_cancellable(
                        shading_dict,
                        ctm,
                        viewport,
                        buf,
                        reader,
                        cancel,
                        options,
                    )
                }
                Some(4 | 5) => Self::paint_gouraud_mesh(
                    shading_dict,
                    ctm,
                    viewport,
                    buf,
                    reader,
                    mesh_data,
                    options,
                ),
                Some(6 | 7) => Self::paint_patch_mesh(
                    shading_dict,
                    ctm,
                    viewport,
                    buf,
                    reader,
                    mesh_data,
                    options,
                ),
                Some(other) => log::debug!("ShadingRenderer: ShadingType {other} not supported"),
                None => log::debug!("ShadingRenderer: missing ShadingType"),
            }
            cancel
                .check("shading result")
                .map_err(|error| error.to_string())?;
            if let Some(reason) = failure.get() {
                return Err(format!("shading conversion failed: {reason}"));
            }
            // End the immutable sampling borrow of target's clip before compositing.
            let opacity = options.opacity;
            for y in 0..height as i32 {
                cancel
                    .check("shading composite")
                    .map_err(|error| error.to_string())?;
                for x in 0..width as i32 {
                    let pixel = buf.get_pixel(x, y);
                    if pixel[3] != 0 {
                        let coverage = region.coverage(x, y)?;
                        target.blend_pixel_with_clip_limit(
                            x + x0,
                            y + y0,
                            pixel,
                            opacity,
                            coverage,
                        );
                    }
                }
            }
            cancel
                .check("shading composite result")
                .map_err(|error| error.to_string())?;
            Ok(())
        })
    }

    /// ShadingType 1 (function-based): color at each point (x, y) within /Domain
    /// is the result of a 2-input function, optionally pre-transformed by the
    /// shading's /Matrix. We iterate device pixels, map back to domain space, and
    /// evaluate.
    #[cfg(test)]
    fn paint_function_based(
        dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
    ) {
        Self::paint_function_based_cancellable(
            dict,
            ctm,
            viewport,
            buf,
            reader,
            &CancelToken::none(),
            ShadingRenderOptions::default(),
        );
    }

    fn paint_function_based_cancellable(
        dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
        cancel: &CancelToken,
        options: ShadingRenderOptions<'_>,
    ) {
        let func_obj = match dict.get("Function") {
            Some(f) => f,
            None => {
                log::warn!("function-based shading: missing /Function");
                return;
            }
        };
        // Domain [x0 x1 y0 y1] defaults to the unit square, but a present
        // malformed value is a local paint refusal.
        let domain = if dict.contains_key("Domain") {
            match get_float_array(dict, "Domain") {
                Some(values) if values.len() == 4 => values,
                _ => {
                    log::warn!("function-based shading: malformed /Domain");
                    return;
                }
            }
        } else {
            vec![0.0, 1.0, 0.0, 1.0]
        };
        let (dx0, dx1) = (domain[0], domain[1]);
        let (dy0, dy1) = (domain[2], domain[3]);
        // /Matrix maps domain space → the shading's target user space.
        let shading_matrix = if dict.contains_key("Matrix") {
            match get_float_array(dict, "Matrix") {
                Some(m) if m.len() == 6 => Transform2D::from([m[0], m[1], m[2], m[3], m[4], m[5]]),
                _ => {
                    log::warn!("function-based shading: malformed /Matrix");
                    return;
                }
            }
        } else {
            Transform2D::identity()
        };
        let Some(color_space) = shading_color_space_name(dict) else {
            log::warn!("function-based shading: missing or malformed /ColorSpace");
            return;
        };
        let color_space_obj = shading_color_space_object(dict);
        let dither = buf.render_mode().is_high_quality();

        // device pixel → user space → domain space.
        let Some(pixel_to_user) = Self::pixel_to_user(ctm, viewport) else {
            options.fail("analytic shading has singular or unrepresentable coordinate mapping");
            return;
        };
        let Some(function) = options.prepare_function(func_obj, 2, reader) else {
            options.fail("function-based shading function preparation failed or exceeded limits");
            return;
        };
        let Some(user_to_domain) =
            invert_transform_or_decline(&shading_matrix, "function-based shading /Matrix")
        else {
            options.fail("function shading has singular or unrepresentable domain matrix");
            return;
        };

        let Some((x_start, y_start, x_end, y_end)) = Self::paint_bounds(buf) else {
            return;
        };
        for py in y_start..y_end {
            if cancel.is_cancelled() || options.failed() {
                return;
            }
            for px in x_start..x_end {
                if !buf.clip_allows(px, py) || !options.sample_visible(px, py) {
                    continue;
                }
                let (ux, uy) = pixel_to_user.transform_point(px as f64 + 0.5, py as f64 + 0.5);
                let (mx, my) = user_to_domain.transform_point(ux, uy);
                if !mx.is_finite() || !my.is_finite() {
                    options.fail("nonfinite function-shading sample position");
                    return;
                }
                if mx < dx0.min(dx1) || mx > dx0.max(dx1) || my < dy0.min(dy1) || my > dy0.max(dy1)
                {
                    continue;
                }
                let comps = options.evaluate_function(&function, &[mx, my]);
                if comps.is_empty() {
                    options.fail("shading function produced no components");
                    return;
                }
                let Some(color) = components_to_render_color_with_space(
                    &comps,
                    &color_space,
                    color_space_obj,
                    reader,
                    options,
                ) else {
                    return;
                };
                let pixel = quantize_shading_color(
                    color,
                    px + options.dither_origin.0,
                    py + options.dither_origin.1,
                    dither,
                );
                buf.blend_pixel(px, py, pixel, 1.0);
            }
        }
    }

    /// Map a device pixel (px, py) back to user space (the space `Coords` live
    /// in). `pixel → media-box user space` is `inv_vp`; `media-box → current
    /// user space` is `inv_ctm`. Applying inv_vp first then inv_ctm gives the
    /// composite `inv_vp.concat(&inv_ctm)`.
    fn pixel_to_user(ctm: &Transform2D, viewport: &Viewport) -> Option<Transform2D> {
        let inv_ctm = invert_transform_or_decline(ctm, "shading paint CTM")?;
        let viewport_transform = viewport.to_transform();
        let inv_vp =
            invert_transform_or_decline(&viewport_transform, "shading viewport transform")?;
        let mapping = inv_vp.concat(&inv_ctm);
        mapping
            .to_array()
            .iter()
            .all(|v| v.is_finite())
            .then_some(mapping)
    }

    #[cfg(test)]
    fn paint_axial(
        dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
    ) {
        Self::paint_axial_cancellable(
            dict,
            ctm,
            viewport,
            buf,
            reader,
            &CancelToken::none(),
            ShadingRenderOptions::default(),
        );
    }

    fn paint_axial_cancellable(
        dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
        cancel: &CancelToken,
        options: ShadingRenderOptions<'_>,
    ) {
        let coords = match get_float_array(dict, "Coords") {
            Some(c) if c.len() == 4 => c,
            _ => {
                log::warn!("axial shading: missing or malformed /Coords");
                return;
            }
        };
        let (x0, y0, x1, y1) = (coords[0], coords[1], coords[2], coords[3]);
        let geometry = match AxialGeometry::new([x0, y0, x1, y1]) {
            Ok(geometry) => geometry,
            Err(reason) => {
                options.fail(reason);
                return;
            }
        };

        let extend = if dict.contains_key("Extend") {
            match get_bool_pair(dict, "Extend") {
                Some(value) => value,
                None => {
                    log::warn!("axial shading: malformed /Extend");
                    return;
                }
            }
        } else {
            [false, false]
        };
        let domain = if dict.contains_key("Domain") {
            match get_float_array(dict, "Domain") {
                Some(values) if values.len() == 2 => values,
                _ => {
                    log::warn!("axial shading: malformed /Domain");
                    return;
                }
            }
        } else {
            vec![0.0, 1.0]
        };
        let t0 = domain[0];
        let t1 = domain[1];

        let func_obj = match dict.get("Function") {
            Some(f) => f,
            None => {
                log::warn!("axial shading: missing /Function");
                return;
            }
        };
        let Some(color_space) = shading_color_space_name(dict) else {
            log::warn!("axial shading: missing or malformed /ColorSpace");
            return;
        };
        let color_space_obj = shading_color_space_object(dict);

        let Some(pixel_to_user) = Self::pixel_to_user(ctm, viewport) else {
            options.fail("analytic shading has singular or unrepresentable coordinate mapping");
            return;
        };
        let Some(function) = options.prepare_function(func_obj, 1, reader) else {
            options.fail("axial shading function preparation failed or exceeded limits");
            return;
        };
        let Some((x_start, y_start, x_end, y_end)) = Self::paint_bounds(buf) else {
            return;
        };

        // Cache high-resolution float colours to avoid re-evaluating the
        // function per pixel without prematurely stepping the gradient at 8-bit
        // output precision.
        let mut cache = ShadingColorCache::new_bounded(options.color_cache_entries);
        let dither = buf.render_mode().is_high_quality();

        for py in y_start..y_end {
            if cancel.is_cancelled() || options.failed() {
                return;
            }
            for px in x_start..x_end {
                if !buf.clip_allows(px, py) || !options.sample_visible(px, py) {
                    continue;
                }
                let (ux, uy) = pixel_to_user.transform_point(px as f64 + 0.5, py as f64 + 0.5);
                let s_clamped = match geometry.parameter((ux, uy), extend) {
                    Ok(Some(s)) => s,
                    Ok(None) => continue,
                    Err(reason) => {
                        options.fail(reason);
                        return;
                    }
                };
                let color = Self::color_for(
                    s_clamped,
                    t0,
                    t1,
                    &function,
                    &color_space,
                    color_space_obj,
                    reader,
                    &mut cache,
                    options,
                );
                if let Some(color) = color {
                    let pixel = quantize_shading_color(
                        color,
                        px + options.dither_origin.0,
                        py + options.dither_origin.1,
                        dither,
                    );
                    buf.blend_pixel(px, py, pixel, 1.0);
                }
            }
        }
    }

    #[cfg(test)]
    fn paint_radial(
        dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
    ) {
        Self::paint_radial_cancellable(
            dict,
            ctm,
            viewport,
            buf,
            reader,
            &CancelToken::none(),
            ShadingRenderOptions::default(),
        );
    }

    fn paint_radial_cancellable(
        dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
        cancel: &CancelToken,
        options: ShadingRenderOptions<'_>,
    ) {
        let coords = match get_float_array(dict, "Coords") {
            Some(c) if c.len() == 6 => c,
            _ => {
                log::warn!("radial shading: missing or malformed /Coords");
                return;
            }
        };
        let (x0, y0, r0) = (coords[0], coords[1], coords[2]);
        let (x1, y1, r1) = (coords[3], coords[4], coords[5]);
        let geometry = match RadialGeometry::new([x0, y0, r0, x1, y1, r1]) {
            Ok(geometry) => geometry,
            Err(reason) => {
                options.fail(reason);
                return;
            }
        };

        let extend = if dict.contains_key("Extend") {
            match get_bool_pair(dict, "Extend") {
                Some(value) => value,
                None => {
                    log::warn!("radial shading: malformed /Extend");
                    return;
                }
            }
        } else {
            [false, false]
        };
        let domain = if dict.contains_key("Domain") {
            match get_float_array(dict, "Domain") {
                Some(values) if values.len() == 2 => values,
                _ => {
                    log::warn!("radial shading: malformed /Domain");
                    return;
                }
            }
        } else {
            vec![0.0, 1.0]
        };
        let t0 = domain[0];
        let t1 = domain[1];

        let func_obj = match dict.get("Function") {
            Some(f) => f,
            None => {
                log::warn!("radial shading: missing /Function");
                return;
            }
        };
        let Some(color_space) = shading_color_space_name(dict) else {
            log::warn!("radial shading: missing or malformed /ColorSpace");
            return;
        };
        let color_space_obj = shading_color_space_object(dict);

        let Some(pixel_to_user) = Self::pixel_to_user(ctm, viewport) else {
            options.fail("analytic shading has singular or unrepresentable coordinate mapping");
            return;
        };
        let Some(function) = options.prepare_function(func_obj, 1, reader) else {
            options.fail("radial shading function preparation failed or exceeded limits");
            return;
        };
        let Some((x_start, y_start, x_end, y_end)) = Self::paint_bounds(buf) else {
            return;
        };
        let mut cache = ShadingColorCache::new_bounded(options.color_cache_entries);
        let dither = buf.render_mode().is_high_quality();

        for py in y_start..y_end {
            if cancel.is_cancelled() || options.failed() {
                return;
            }
            for px in x_start..x_end {
                if !buf.clip_allows(px, py) || !options.sample_visible(px, py) {
                    continue;
                }
                let (ux, uy) = pixel_to_user.transform_point(px as f64 + 0.5, py as f64 + 0.5);
                let s = match geometry.parameter((ux, uy), extend) {
                    Ok(Some(s)) => s,
                    Ok(None) => continue,
                    Err(reason) => {
                        options.fail(reason);
                        return;
                    }
                };
                let color = Self::color_for(
                    s,
                    t0,
                    t1,
                    &function,
                    &color_space,
                    color_space_obj,
                    reader,
                    &mut cache,
                    options,
                );
                if let Some(color) = color {
                    let pixel = quantize_shading_color(
                        color,
                        px + options.dither_origin.0,
                        py + options.dither_origin.1,
                        dither,
                    );
                    buf.blend_pixel(px, py, pixel, 1.0);
                }
            }
        }
    }

    /// Map parametric `s ∈ [0,1]` to a pixel colour. A bucket is only an index:
    /// reuse requires the exact parameter, including across clipping/tile order.
    #[allow(clippy::too_many_arguments)]
    fn color_for(
        s: f64,
        t0: f64,
        t1: f64,
        function: &crate::render::function::PreparedFunction,
        color_space: &str,
        color_space_obj: Option<&PdfObject>,
        reader: &PdfReader,
        cache: &mut ShadingColorCache,
        options: ShadingRenderOptions<'_>,
    ) -> Option<RenderColor> {
        if let Some(cached) = cache.get(s) {
            return Some(cached);
        }
        let t = geometry::domain_value(t0, t1, s);
        let components = options.evaluate_function(function, &[t]);
        if components.is_empty() {
            options.fail("shading function produced no components");
            return None;
        }
        let color = components_to_render_color_with_space(
            &components,
            color_space,
            color_space_obj,
            reader,
            options,
        )?;
        cache.set(s, color);
        Some(color)
    }
}

// ---------------------------------------------------------------------------
// Mesh shadings (Types 4-7): shared vertex model + Gouraud triangle rasterizer
// ---------------------------------------------------------------------------

/// Decode parameters shared by the mesh vertex stream readers.
struct MeshDecode {
    bits_per_coord: usize,
    bits_per_comp: usize,
    bits_per_flag: usize,
    /// Decode array: [xmin xmax ymin ymax c1min c1max ...].
    decode: Vec<f64>,
    /// Number of color components per vertex when colors are given directly
    /// (no /Function); 1 when a /Function maps a single parametric value.
    n_color: usize,
}

impl MeshDecode {
    fn from_dict(
        dict: &PdfDictionary,
        color_space: &str,
        color_space_obj: Option<&PdfObject>,
        reader: &PdfReader,
    ) -> Option<Self> {
        let shading_type = dict.get_integer("ShadingType")?;
        if !(4..=7).contains(&shading_type) {
            return None;
        }
        let bits_per_coord =
            mesh_integer_from_dict(dict, "BitsPerCoordinate", &[1, 2, 4, 8, 12, 16, 24, 32])?;
        let bits_per_comp =
            mesh_integer_from_dict(dict, "BitsPerComponent", &[1, 2, 4, 8, 12, 16])?;
        let bits_per_flag = if matches!(shading_type, 4 | 6 | 7) || dict.contains_key("BitsPerFlag")
        {
            mesh_integer_from_dict(dict, "BitsPerFlag", &[2, 4, 8])?
        } else {
            8
        };
        let decode = get_strict_float_array(dict, "Decode")?;
        let has_function = dict.get("Function").is_some();
        let n_color = if has_function {
            1
        } else {
            color_space_component_count(color_space, color_space_obj, reader)?
        };
        if n_color == 0
            || n_color > crate::render::colorspace::MAX_DEVICEN_COMPONENTS
            || decode.len() != 4 + 2 * n_color
        {
            return None;
        }
        Some(Self {
            bits_per_coord,
            bits_per_comp,
            bits_per_flag,
            decode,
            n_color,
        })
    }

    /// Read one coordinate pair, mapping through Decode + CTM/viewport to device
    /// space, plus the raw color components (parametric or direct).
    fn read_vertex(
        &self,
        br: &mut crate::render::function::BitReader,
        to_device: &Transform2D,
        options: ShadingRenderOptions<'_>,
    ) -> Option<MeshVertex> {
        if options.failed()
            || crate::cancel::check_current_cancel("mesh vertex conversion").is_err()
        {
            return None;
        }
        if !options.charge_work(1) {
            return None;
        }
        let xr = br.read(self.bits_per_coord)? as f64;
        let yr = br.read(self.bits_per_coord)? as f64;
        let xmax_raw = crate::render::function::max_value(self.bits_per_coord);
        let x = decode_value(xr, xmax_raw, self.decode[0], self.decode[1]);
        let y = decode_value(yr, xmax_raw, self.decode[2], self.decode[3]);
        let (dx, dy) = to_device.transform_point(x, y);
        if !dx.is_finite() || !dy.is_finite() {
            options.fail("nonfinite mesh vertex geometry");
            return None;
        }

        let cmax_raw = crate::render::function::max_value(self.bits_per_comp);
        let mut comps = Vec::with_capacity(self.n_color);
        for k in 0..self.n_color {
            let raw = br.read(self.bits_per_comp)? as f64;
            let dlo = self.decode[4 + 2 * k];
            let dhi = self.decode[5 + 2 * k];
            comps.push(decode_value(raw, cmax_raw, dlo, dhi));
        }
        let Some(sample) = MeshSample::new(&comps) else {
            options.fail("nonfinite mesh source components");
            return None;
        };
        Some(MeshVertex { dx, dy, sample })
    }
}

fn mesh_integer_from_dict(dict: &PdfDictionary, key: &str, allowed: &[i64]) -> Option<usize> {
    let value = dict.get_integer(key)?;
    if allowed.contains(&value) {
        usize::try_from(value).ok()
    } else {
        None
    }
}

fn get_strict_float_array(dict: &PdfDictionary, key: &str) -> Option<Vec<f64>> {
    let arr = dict.get(key)?.as_array()?;
    let mut vals = Vec::with_capacity(arr.len());
    for item in arr {
        let value = item.as_number()?;
        if !value.is_finite() {
            return None;
        }
        vals.push(value);
    }
    Some(vals)
}

/// Map a raw integer sample in [0, max] onto [lo, hi].
fn decode_value(raw: f64, max: f64, lo: f64, hi: f64) -> f64 {
    if max <= 0.0 {
        lo
    } else {
        lo + (raw / max) * (hi - lo)
    }
}

fn color_space_component_count(
    name: &str,
    color_space_obj: Option<&PdfObject>,
    reader: &PdfReader,
) -> Option<usize> {
    if let Some(object) = color_space_obj {
        return crate::render::default_colorspace::component_ranges(object, reader)
            .ok()
            .map(|ranges| ranges.len());
    }
    match name {
        "DeviceGray" | "G" => Some(1),
        "DeviceRGB" | "RGB" | "sRGB" => Some(3),
        "DeviceCMYK" | "CMYK" => Some(4),
        _ => None,
    }
}

impl ShadingRenderer {
    /// ShadingType 4 (free-form) and 5 (lattice-form) Gouraud triangle meshes.
    fn paint_gouraud_mesh(
        dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
        mesh_data: Option<&[u8]>,
        options: ShadingRenderOptions<'_>,
    ) {
        let shading_type = match dict.get_integer("ShadingType") {
            Some(value @ (4 | 5)) => value,
            Some(other) => {
                log::warn!("mesh shading: unsupported ShadingType {other}");
                return;
            }
            None => {
                log::warn!("mesh shading: missing ShadingType");
                return;
            }
        };
        let Some(color_space) = shading_color_space_name(dict) else {
            log::warn!("mesh shading: missing or malformed /ColorSpace");
            return;
        };
        let color_space_obj = shading_color_space_object(dict);
        let Some(dec) = MeshDecode::from_dict(dict, &color_space, color_space_obj, reader) else {
            options.fail("invalid Gouraud mesh decode metadata");
            log::warn!("mesh shading: missing BitsPerCoordinate/BitsPerComponent/Decode");
            return;
        };
        let data = match mesh_data {
            Some(d) => d,
            None => {
                log::warn!("mesh shading: vertex stream not available");
                return;
            }
        };
        let function = match dict.get("Function") {
            None | Some(PdfObject::Null) => None,
            Some(object) => match options.prepare_function(object, 1, reader) {
                Some(function) => Some(function),
                None => {
                    options.fail("mesh function preparation failed or exceeded limits");
                    return;
                }
            },
        };
        let paint = MeshPaint {
            function: function.as_deref(),
            color_space: &color_space,
            color_space_obj,
            reader,
            options,
            patch_corners: None,
        };
        let to_device = ctm.concat(&viewport.to_transform());
        let mut br = crate::render::function::BitReader::new(data);

        if shading_type == 5 {
            // Lattice-form: a grid of /VerticesPerRow columns; each 2x2 cell of
            // adjacent rows makes two triangles. No flags, no colors-as-flags.
            let per_row = match dict.get_integer("VerticesPerRow") {
                Some(value) if value >= 2 => match usize::try_from(value) {
                    Ok(value) => value,
                    Err(_) => {
                        options.fail("lattice VerticesPerRow overflows platform size");
                        log::warn!("lattice mesh: VerticesPerRow overflows usize");
                        return;
                    }
                },
                Some(_) => {
                    options.fail("lattice VerticesPerRow must be at least two");
                    log::warn!("lattice mesh: VerticesPerRow < 2");
                    return;
                }
                None => {
                    options.fail("lattice VerticesPerRow is missing");
                    log::warn!("lattice mesh: missing VerticesPerRow");
                    return;
                }
            };
            let vertex_bits = 2 * dec.bits_per_coord + dec.n_color * dec.bits_per_comp;
            let vertex_bytes = vertex_bits.div_ceil(8);
            let Some(row_bytes) = vertex_bytes.checked_mul(per_row) else {
                options.fail("lattice row size overflow");
                return;
            };
            if row_bytes == 0 || data.len() % row_bytes != 0 {
                options.fail("lattice stream does not contain complete rows");
                return;
            }
            let Some(row_vertices) = per_row.checked_mul(2) else {
                options.fail("lattice storage overflow");
                return;
            };
            let _row_memory = match options.reserve_mesh_storage(row_vertices) {
                Ok(memory) => memory,
                Err(_) => {
                    options.fail("lattice working-memory budget exceeded");
                    return;
                }
            };
            let mut prev_row: Vec<MeshVertex> = Vec::new();
            loop {
                if br.bits_remaining() == 0 {
                    break;
                }
                if options.failed()
                    || crate::cancel::check_current_cancel("lattice row conversion").is_err()
                {
                    return;
                }
                // The decoded stream must contain this row before reserving it;
                // an untrusted VerticesPerRow must not determine allocation alone.
                if br.bits_remaining() / 8 < row_bytes {
                    options.fail("incomplete lattice row");
                    return;
                }
                // Read one row.
                let mut row = Vec::new();
                if row.try_reserve_exact(per_row).is_err() {
                    options.fail("lattice row allocation failed");
                    return;
                }
                let mut complete = true;
                for _ in 0..per_row {
                    match dec.read_vertex(&mut br, &to_device, options) {
                        Some(v) => {
                            row.push(v);
                            br.align_to_byte();
                        }
                        None => {
                            complete = false;
                            break;
                        }
                    }
                }
                if !complete || row.len() < per_row {
                    options.fail("incomplete or invalid lattice vertex row");
                    return;
                }
                if !prev_row.is_empty() {
                    for c in 0..per_row - 1 {
                        // Two triangles per cell.
                        if options.failed() {
                            return;
                        }
                        mesh::fill_triangle(buf, prev_row[c], prev_row[c + 1], row[c], &paint);
                        mesh::fill_triangle(buf, prev_row[c + 1], row[c + 1], row[c], &paint);
                    }
                }
                prev_row = row;
            }
            return;
        }

        // A flag-0 record starts an independent triangle; the following
        // two vertex flags are consumed but ignored (ISO 32000-1 8.7.4.5.5).
        let mut previous: Option<[MeshVertex; 3]> = None;
        let read = |br: &mut crate::render::function::BitReader<'_>| -> Option<(u32, MeshVertex)> {
            let flag = br.read(dec.bits_per_flag)?;
            let vertex = dec.read_vertex(br, &to_device, options)?;
            br.align_to_byte();
            Some((flag, vertex))
        };
        while br.bits_remaining() > 0 {
            if options.failed()
                || crate::cancel::check_current_cancel("free-form mesh conversion").is_err()
            {
                return;
            }
            let Some((flag, vertex)) = read(&mut br) else {
                options.fail("truncated free-form mesh vertex");
                return;
            };
            let triangle = match flag {
                0 => {
                    let (Some((_, second)), Some((_, third))) = (read(&mut br), read(&mut br))
                    else {
                        options.fail("incomplete independent mesh triangle");
                        return;
                    };
                    [vertex, second, third]
                }
                1 | 2 => {
                    let Some(prior) = previous else {
                        options.fail("mesh edge reuse has no preceding triangle");
                        return;
                    };
                    if flag == 1 {
                        [prior[1], prior[2], vertex]
                    } else {
                        [prior[0], prior[2], vertex]
                    }
                }
                _ => {
                    options.fail("invalid free-form mesh edge flag");
                    return;
                }
            };
            mesh::fill_triangle(buf, triangle[0], triangle[1], triangle[2], &paint);
            previous = Some(triangle);
        }
    }

    /// ShadingType 6 (Coons) and 7 (tensor-product) patch meshes. Each patch is
    /// planned into device-adaptive, shared-edge-compatible grids. Source corner
    /// samples are evaluated bilinearly before nonlinear functions/conversion.
    /// Both patch kinds retain their bicubic geometry and source stream order.
    fn paint_patch_mesh(
        dict: &PdfDictionary,
        ctm: &Transform2D,
        viewport: &Viewport,
        buf: &mut PixelBuffer,
        reader: &PdfReader,
        mesh_data: Option<&[u8]>,
        options: ShadingRenderOptions<'_>,
    ) {
        let shading_type = match dict.get_integer("ShadingType") {
            Some(value @ (6 | 7)) => value,
            Some(other) => {
                log::warn!("patch mesh: unsupported ShadingType {other}");
                return;
            }
            None => {
                log::warn!("patch mesh: missing ShadingType");
                return;
            }
        };
        let n_points_new = if shading_type == 7 { 16 } else { 12 };
        let Some(color_space) = shading_color_space_name(dict) else {
            log::warn!("patch mesh: missing or malformed /ColorSpace");
            return;
        };
        let color_space_obj = shading_color_space_object(dict);
        let Some(dec) = MeshDecode::from_dict(dict, &color_space, color_space_obj, reader) else {
            options.fail("invalid patch mesh decode metadata");
            log::warn!("patch mesh: missing BitsPerCoordinate/BitsPerComponent/Decode");
            return;
        };
        let data = match mesh_data {
            Some(d) => d,
            None => return,
        };
        let function = match dict.get("Function") {
            None | Some(PdfObject::Null) => None,
            Some(object) => match options.prepare_function(object, 1, reader) {
                Some(function) => Some(function),
                None => {
                    options.fail("patch function preparation failed or exceeded limits");
                    return;
                }
            },
        };
        let paint = MeshPaint {
            function: function.as_deref(),
            color_space: &color_space,
            color_space_obj,
            reader,
            options,
            patch_corners: None,
        };
        let to_device = ctm.concat(&viewport.to_transform());
        let mut br = crate::render::function::BitReader::new(data);

        let coord_max = crate::render::function::max_value(dec.bits_per_coord);
        let comp_max = crate::render::function::max_value(dec.bits_per_comp);

        // Previous patch's control points (in patch/user space, pre-device) and
        // corner colors, for edge sharing (flags 1/2/3).
        let mut prev_pts: Vec<(f64, f64)> = Vec::new();
        let mut prev_cols: Vec<MeshSample> = Vec::new();
        let mut patch_count = 0usize;
        let mut patches =
            mesh::PatchBatch::new(options).with_device_bounds(Self::paint_bounds(buf));

        loop {
            if options.failed()
                || crate::cancel::check_current_cancel("patch shading conversion").is_err()
            {
                return;
            }
            if br.bits_remaining() == 0 {
                break;
            }
            if patch_count >= MAX_PATCH_MESH_PATCHES {
                options.fail("patch mesh exceeds patch-count budget");
                log::warn!(
                    "patch mesh: patch count exceeded cap {}",
                    MAX_PATCH_MESH_PATCHES
                );
                break;
            }
            if br.bits_remaining() < dec.bits_per_flag {
                options.fail("truncated patch mesh flag");
                break;
            }
            let flag = match br.read(dec.bits_per_flag) {
                Some(f) => f,
                None => break,
            };
            let new_pts_count = if flag == 0 {
                n_points_new
            } else {
                n_points_new - 4
            };
            let new_cols_count = if flag == 0 { 4 } else { 2 };
            if !options.charge_work((new_pts_count + new_cols_count) as u64) {
                return;
            }

            // Read new control points (user space).
            let mut new_pts = Vec::with_capacity(new_pts_count);
            let mut ok = true;
            for _ in 0..new_pts_count {
                let (Some(xr), Some(yr)) =
                    (br.read(dec.bits_per_coord), br.read(dec.bits_per_coord))
                else {
                    ok = false;
                    break;
                };
                let x = decode_value(xr as f64, coord_max, dec.decode[0], dec.decode[1]);
                let y = decode_value(yr as f64, coord_max, dec.decode[2], dec.decode[3]);
                new_pts.push((x, y));
            }
            if !ok {
                options.fail("truncated patch mesh control points");
                break;
            }
            // Read new corner colors.
            let mut new_cols = Vec::with_capacity(new_cols_count);
            for _ in 0..new_cols_count {
                let mut comps = Vec::with_capacity(dec.n_color);
                for k in 0..dec.n_color {
                    let Some(raw) = br.read(dec.bits_per_comp) else {
                        ok = false;
                        break;
                    };
                    let dlo = dec.decode[4 + 2 * k];
                    let dhi = dec.decode[5 + 2 * k];
                    comps.push(decode_value(raw as f64, comp_max, dlo, dhi));
                }
                if !ok {
                    break;
                }
                let Some(sample) = MeshSample::new(&comps) else {
                    ok = false;
                    break;
                };
                new_cols.push(sample);
            }
            if !ok {
                options.fail("invalid or truncated patch mesh colours");
                break;
            }
            br.align_to_byte();

            // Assemble the full 12 (Coons) control points and 4 corner colors,
            // sharing an edge from the previous patch when flag != 0.
            let (patch_pts, cols4) = match assemble_patch(
                flag,
                &new_pts,
                &new_cols,
                &prev_pts,
                &prev_cols,
                shading_type,
            ) {
                Some(v) => v,
                None => {
                    options.fail("invalid patch mesh edge reuse");
                    break;
                }
            };

            if let Err(reason) = patches.push(&patch_pts, &cols4, shading_type, &to_device) {
                options.fail(reason);
                return;
            }

            prev_pts = patch_pts;
            prev_cols = cols4;
            patch_count += 1;
        }
        if !options.failed() {
            if let Err(reason) = patches.paint(buf, &paint) {
                options.fail(reason);
            }
        }
    }
}

/// A point in patch/user space (pre-device).
type PatchPoint = (f64, f64);
/// A patch's resolved control points and 4 corner colors.
type PatchData<T> = (Vec<PatchPoint>, Vec<T>);

/// Assemble a patch's full control points and 4 corner colors, honoring
/// edge-sharing flags 1/2/3 (the new patch shares one edge with the previous
/// one). Coons patches carry 12 boundary points. Tensor patches carry those 12
/// boundary points plus the 4 interior controls required for Type 7 bicubic
/// tensor-product evaluation.
///
/// **Spec mapping (ISO 32000-1 §8.7.4.5.7, Table 85; cross-checked against
/// Apache PDFBox `Patch`/`CoonsPatch`, GSoC 2014, Apache-2.0).** The boundary
/// point order p1..p12 (0-based 0..11) traces the four cubic Bézier edges:
/// `C1`: p1 p2 p3 p4 (v at u=0); `D2`: p4 p5 p6 p7 (u at v=1);
/// `C2` reversed: p7 p8 p9 p10 (v at u=1); `D1` reversed: p10 p11 p12 p1
/// (u at v=0). Corners and their colors: p1↔c1, p4↔c2, p7↔c3, p10↔c4
/// (0-based corner indices 0, 3, 6, 9 ↔ colors 0, 1, 2, 3).
///
/// For a flagged patch (flag f), the new patch's first 4 boundary points
/// (its `p1..p4`, i.e. the shared edge) and its first 2 corner colors (`c1, c2`)
/// are taken from the *previous* patch; the stream then supplies the remaining 8
/// points (`p5..p12`) and 2 colors (`c3, c4`). The exact previous-patch indices
/// reused for each flag are:
///
/// | flag | shared points (prev idx) | shared colors (prev idx) |
/// |------|--------------------------|--------------------------|
/// | 1    | p4 p5 p6 p7   = [3,4,5,6]   | c2 c3 = [1,2] |
/// | 2    | p7 p8 p9 p10  = [6,7,8,9]   | c3 c4 = [2,3] |
/// | 3    | p10 p11 p12 p1 = [9,10,11,0] | c4 c1 = [3,0] |
fn assemble_patch<T: Copy>(
    flag: u32,
    new_pts: &[(f64, f64)],
    new_cols: &[T],
    prev_pts: &[(f64, f64)],
    prev_cols: &[T],
    shading_type: i64,
) -> Option<PatchData<T>> {
    if flag > 3 || !matches!(shading_type, 6 | 7) {
        return None;
    }
    let required_points = if shading_type == 7 { 16 } else { 12 };
    let expected_points = if flag == 0 {
        required_points
    } else {
        required_points - 4
    };
    if new_pts.len() != expected_points || new_cols.len() != if flag == 0 { 4 } else { 2 } {
        return None;
    }

    if flag == 0 {
        let pts: Vec<_> = new_pts.iter().take(required_points).copied().collect();
        if pts.len() < required_points || new_cols.len() < 4 {
            return None;
        }
        return Some((pts, new_cols.to_vec()));
    }

    // Shared-edge patches reuse one edge (4 points + 2 colors) of the previous
    // patch; the previous patch must therefore be fully formed.
    if prev_pts.len() < required_points || prev_cols.len() < 4 {
        return None;
    }
    let shared_edge: [usize; 4] = shared_edge_indices(flag);
    let shared_cols: [usize; 2] = shared_color_indices(flag);

    // new boundary/tensor = [shared edge p1..p4] ++ stream points p5...
    let mut pts = Vec::with_capacity(required_points);
    for &i in &shared_edge {
        pts.push(prev_pts[i]);
    }
    for &p in new_pts.iter().take(required_points.saturating_sub(4)) {
        pts.push(p);
    }
    if pts.len() < required_points {
        return None;
    }
    pts.truncate(required_points);

    // new corner colors = [shared c1, c2] ++ [2 new colors c3, c4].
    let mut cols = Vec::with_capacity(4);
    cols.push(prev_cols[shared_cols[0]]);
    cols.push(prev_cols[shared_cols[1]]);
    for &c in new_cols.iter().take(2) {
        cols.push(c);
    }
    if cols.len() < 4 {
        return None;
    }
    Some((pts, cols))
}

/// Previous-patch boundary-point indices reused as the new patch's shared edge
/// (p1..p4) for edge flags 1/2/3. See [`assemble_patch`] for the spec table.
fn shared_edge_indices(flag: u32) -> [usize; 4] {
    match flag {
        1 => [3, 4, 5, 6],
        2 => [6, 7, 8, 9],
        _ => [9, 10, 11, 0],
    }
}

/// Previous-patch corner-color indices reused as the new patch's shared colors
/// (c1, c2) for edge flags 1/2/3. See [`assemble_patch`] for the spec table.
fn shared_color_indices(flag: u32) -> [usize; 2] {
    match flag {
        1 => [1, 2],
        2 => [2, 3],
        _ => [3, 0],
    }
}

/// Evaluate the Coons surface position at (u, v) from the 12 boundary control
/// points. Point order follows the PDF spec boundary: p1..p12 trace the four
/// cubic Bezier edges; corners are p1 (u0,v0), p4 (u0,v1), p7 (u1,v1),
/// p10 (u1,v0).
#[cfg(test)]
fn coons_point(p: &[(f64, f64)], u: f64, v: f64) -> (f64, f64) {
    // Boundary curves (each a cubic Bezier):
    //   C1 (v at u=0): p1 p2  p3  p4
    //   C2 (v at u=1): p10 p9 p8 p7   (reversed indices for direction)
    //   D1 (u at v=0): p1 p12 p11 p10
    //   D2 (u at v=1): p4 p5  p6  p7
    let c1 = bezier(p[0], p[1], p[2], p[3], v);
    let c2 = bezier(p[9], p[8], p[7], p[6], v);
    let d1 = bezier(p[0], p[11], p[10], p[9], u);
    let d2 = bezier(p[3], p[4], p[5], p[6], u);

    // Corners.
    let p00 = p[0];
    let p01 = p[3];
    let p11 = p[6];
    let p10 = p[9];

    // Coons surface = ruled(u) + ruled(v) - bilinear(corners).
    let sx = (1.0 - u) * c1.0 + u * c2.0 + (1.0 - v) * d1.0 + v * d2.0
        - ((1.0 - u) * (1.0 - v) * p00.0
            + (1.0 - u) * v * p01.0
            + u * (1.0 - v) * p10.0
            + u * v * p11.0);
    let sy = (1.0 - u) * c1.1 + u * c2.1 + (1.0 - v) * d1.1 + v * d2.1
        - ((1.0 - u) * (1.0 - v) * p00.1
            + (1.0 - u) * v * p01.1
            + u * (1.0 - v) * p10.1
            + u * v * p11.1);
    (sx, sy)
}

fn normalize_smoothness_tolerance(smoothness_tolerance: f64) -> f64 {
    if !smoothness_tolerance.is_finite() || smoothness_tolerance < 0.0 {
        return 0.0;
    }
    smoothness_tolerance.clamp(0.0, 1.0)
}

#[cfg(test)]
fn tensor_point(p: &[(f64, f64)], u: f64, v: f64) -> (f64, f64) {
    let grid = [
        [p[0], p[1], p[2], p[3]],
        [p[11], p[12], p[13], p[4]],
        [p[10], p[15], p[14], p[5]],
        [p[9], p[8], p[7], p[6]],
    ];
    let bu = cubic_bernstein(u);
    let bv = cubic_bernstein(v);
    let mut x = 0.0;
    let mut y = 0.0;
    for i in 0..4 {
        for (j, bv_j) in bv.iter().enumerate() {
            let weight = bu[i] * *bv_j;
            x += weight * grid[i][j].0;
            y += weight * grid[i][j].1;
        }
    }
    (x, y)
}

#[cfg(test)]
fn cubic_bernstein(t: f64) -> [f64; 4] {
    let mt = 1.0 - t;
    [mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t]
}

/// Cubic Bezier interpolation of four control points at parameter t.
#[cfg(test)]
fn bezier(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let mt = 1.0 - t;
    let a = mt * mt * mt;
    let b = 3.0 * mt * mt * t;
    let c = 3.0 * mt * t * t;
    let d = t * t * t;
    (
        a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
        a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
    )
}

/// Signed area of the triangle (a, b, c) doubled (the edge function).
fn edge(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64) -> f64 {
    (cx - ax) * (by - ay) - (cy - ay) * (bx - ax)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn dict(entries: &[(&str, PdfObject)]) -> PdfDictionary {
        PdfDictionary::new(
            entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    }

    fn make_type2_dict(c0: &[f64], c1: &[f64], n: f64) -> PdfDictionary {
        dict(&[
            ("FunctionType", PdfObject::Integer(2)),
            (
                "Domain",
                PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
            ),
            (
                "C0",
                PdfObject::Array(c0.iter().map(|&v| PdfObject::Real(v)).collect()),
            ),
            (
                "C1",
                PdfObject::Array(c1.iter().map(|&v| PdfObject::Real(v)).collect()),
            ),
            ("N", PdfObject::Real(n)),
        ])
    }

    fn mesh_decode_array(values: &[f64]) -> PdfObject {
        PdfObject::Array(values.iter().map(|&value| PdfObject::Real(value)).collect())
    }

    fn red_rgb_function() -> PdfObject {
        PdfObject::Dictionary(make_type2_dict(&[1.0, 0.0, 0.0], &[1.0, 0.0, 0.0], 1.0))
    }

    #[test]
    fn shading_work_budget_is_cumulative_and_fail_closed() {
        let buf = PixelBuffer::new_filled(4, 4, crate::render::buffer::WHITE);
        let budget = AtomicU64::new(20);
        charge_shading_work(&buf, &budget).expect("first shading fits");
        let error = charge_shading_work(&buf, &budget)
            .expect_err("second shading must exceed the remaining work budget");
        assert!(error.contains("with 4 remaining"));
    }

    fn assert_shading_helper_does_not_paint(
        paint: fn(&PdfDictionary, &Transform2D, &Viewport, &mut PixelBuffer, &PdfReader),
        shading: PdfDictionary,
    ) {
        assert_shading_helper_with_ctm_does_not_paint(paint, shading, Transform2D::identity());
    }

    fn assert_shading_helper_with_ctm_does_not_paint(
        paint: fn(&PdfDictionary, &Transform2D, &Viewport, &mut PixelBuffer, &PdfReader),
        shading: PdfDictionary,
        ctm: Transform2D,
    ) {
        let reader = crate::reader::PdfReader::from_bytes(super::tests_minimal_pdf()).unwrap();
        let viewport = Viewport::new([0.0, 0.0, 4.0, 4.0], 72);
        let mut buf = PixelBuffer::new_filled(4, 4, crate::render::buffer::WHITE);

        paint(&shading, &ctm, &viewport, &mut buf, &reader);

        assert_eq!(buf.get_pixel(2, 2), crate::render::buffer::WHITE);
    }

    fn mesh_base_dict(decode: PdfObject) -> PdfDictionary {
        dict(&[
            ("ShadingType", PdfObject::Integer(4)),
            ("ColorSpace", PdfObject::Name("DeviceRGB".to_string())),
            ("BitsPerCoordinate", PdfObject::Integer(8)),
            ("BitsPerComponent", PdfObject::Integer(8)),
            ("BitsPerFlag", PdfObject::Integer(2)),
            ("Decode", decode),
        ])
    }

    #[test]
    fn type2_at_t0_returns_c0() {
        let d = make_type2_dict(&[1.0, 0.0, 0.0], &[0.0, 0.0, 1.0], 1.0);
        let r = eval_type2(&d, 0.0);
        assert!((r[0] - 1.0).abs() < 0.01);
        assert!((r[2] - 0.0).abs() < 0.01);
    }

    #[test]
    fn type2_at_t1_returns_c1() {
        let d = make_type2_dict(&[1.0, 0.0, 0.0], &[0.0, 0.0, 1.0], 1.0);
        let r = eval_type2(&d, 1.0);
        assert!((r[0] - 0.0).abs() < 0.01);
        assert!((r[2] - 1.0).abs() < 0.01);
    }

    #[test]
    fn type2_midpoint_linear() {
        let d = make_type2_dict(&[1.0, 0.0, 0.0], &[0.0, 0.0, 1.0], 1.0);
        let r = eval_type2(&d, 0.5);
        assert!((r[0] - 0.5).abs() < 0.01, "R at 0.5 = {}", r[0]);
        assert!((r[2] - 0.5).abs() < 0.01, "B at 0.5 = {}", r[2]);
    }

    #[test]
    fn type2_quadratic_exponent() {
        let d = make_type2_dict(&[0.0], &[1.0], 2.0);
        let r = eval_type2(&d, 0.5);
        assert!((r[0] - 0.25).abs() < 0.01, "quadratic at 0.5 = {}", r[0]);
    }

    #[test]
    fn mesh_decode_rejects_non_numeric_or_short_decode_arrays() {
        let non_numeric = mesh_base_dict(PdfObject::Array(vec![
            PdfObject::Real(0.0),
            PdfObject::Real(1.0),
            PdfObject::Real(0.0),
            PdfObject::Name("Bad".to_string()),
            PdfObject::Real(0.0),
            PdfObject::Real(1.0),
            PdfObject::Real(0.0),
            PdfObject::Real(1.0),
            PdfObject::Real(0.0),
            PdfObject::Real(1.0),
        ]));
        assert!(MeshDecode::from_dict(
            &non_numeric,
            "DeviceRGB",
            None,
            &PdfReader::from_bytes(tests_minimal_pdf()).unwrap()
        )
        .is_none());

        let short = mesh_base_dict(mesh_decode_array(&[0.0, 1.0, 0.0, 1.0, 0.0, 1.0]));
        assert!(MeshDecode::from_dict(
            &short,
            "DeviceRGB",
            None,
            &PdfReader::from_bytes(tests_minimal_pdf()).unwrap()
        )
        .is_none());
    }

    #[test]
    fn mesh_decode_rejects_invalid_bit_fields() {
        let mut invalid_coord = mesh_base_dict(mesh_decode_array(&[
            0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0,
        ]));
        invalid_coord.insert("BitsPerCoordinate", PdfObject::Integer(-1));
        assert!(MeshDecode::from_dict(
            &invalid_coord,
            "DeviceRGB",
            None,
            &PdfReader::from_bytes(tests_minimal_pdf()).unwrap()
        )
        .is_none());

        let mut missing_flag = mesh_base_dict(mesh_decode_array(&[
            0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0,
        ]));
        missing_flag.remove("BitsPerFlag");
        assert!(MeshDecode::from_dict(
            &missing_flag,
            "DeviceRGB",
            None,
            &PdfReader::from_bytes(tests_minimal_pdf()).unwrap()
        )
        .is_none());
    }

    #[test]
    fn mesh_decode_rejects_missing_or_non_mesh_shading_type() {
        let mut missing_type = mesh_base_dict(mesh_decode_array(&[
            0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0,
        ]));
        missing_type.remove("ShadingType");
        assert!(MeshDecode::from_dict(
            &missing_type,
            "DeviceRGB",
            None,
            &PdfReader::from_bytes(tests_minimal_pdf()).unwrap()
        )
        .is_none());

        let mut non_mesh = mesh_base_dict(mesh_decode_array(&[
            0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0,
        ]));
        non_mesh.insert("ShadingType", PdfObject::Integer(3));
        assert!(MeshDecode::from_dict(
            &non_mesh,
            "DeviceRGB",
            None,
            &PdfReader::from_bytes(tests_minimal_pdf()).unwrap()
        )
        .is_none());
    }

    #[test]
    fn mesh_decode_accepts_type5_without_flag_bits() {
        let mut lattice = mesh_base_dict(mesh_decode_array(&[
            0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0,
        ]));
        lattice.insert("ShadingType", PdfObject::Integer(5));
        lattice.remove("BitsPerFlag");
        lattice.insert("VerticesPerRow", PdfObject::Integer(2));
        assert!(MeshDecode::from_dict(
            &lattice,
            "DeviceRGB",
            None,
            &PdfReader::from_bytes(tests_minimal_pdf()).unwrap()
        )
        .is_some());
    }

    #[test]
    fn type2_input_is_clipped_to_its_declared_domain() {
        let d = make_type2_dict(&[0.9], &[0.1], 1.0);
        let r = eval_type2(&d, 2.0);
        assert!(
            (r[0] - 0.1).abs() < 1e-12,
            "input must clamp to 1, got {}",
            r[0]
        );
    }

    #[test]
    fn type2_evaluator_rejects_malformed_local_shape_fields() {
        let mut missing_domain = make_type2_dict(&[0.0], &[1.0], 1.0);
        missing_domain.remove("Domain");
        assert!(eval_type2(&missing_domain, 0.5).is_empty());

        let mut missing_exponent = make_type2_dict(&[0.0], &[1.0], 1.0);
        missing_exponent.remove("N");
        assert!(eval_type2(&missing_exponent, 0.5).is_empty());

        let mut malformed_c0 = make_type2_dict(&[0.0], &[1.0], 1.0);
        malformed_c0.insert(
            "C0",
            PdfObject::Array(vec![PdfObject::Name("Bad".to_string())]),
        );
        assert!(eval_type2(&malformed_c0, 0.5).is_empty());

        let mut mismatched_components = make_type2_dict(&[0.0], &[0.0, 1.0], 1.0);
        assert!(eval_type2(&mismatched_components, 0.5).is_empty());
        mismatched_components.insert("C0", PdfObject::Array(vec![PdfObject::Real(f64::NAN)]));
        assert!(eval_type2(&mismatched_components, 0.5).is_empty());
    }

    #[test]
    fn type3_single_subfunction_delegates() {
        // Build a reader-independent Type 3 with an inline Type 2 sub-function.
        let sub = PdfObject::Dictionary(make_type2_dict(&[1.0, 0.0, 0.0], &[0.0, 0.0, 1.0], 1.0));
        let d = dict(&[
            ("FunctionType", PdfObject::Integer(3)),
            (
                "Domain",
                PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
            ),
            ("Functions", PdfObject::Array(vec![sub])),
            ("Bounds", PdfObject::Array(vec![])),
            (
                "Encode",
                PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
            ),
        ]);
        // eval_type3 only consults `reader` for indirect sub-functions; with an
        // inline dict it is never used, so a throwaway reader suffices. We build
        // one from a trivial PDF.
        let reader = crate::reader::PdfReader::from_bytes(super::tests_minimal_pdf()).unwrap();
        let r = eval_type3(&d, 0.5, &reader);
        assert!((r[0] - 0.5).abs() < 0.01, "Type3->Type2 at 0.5: {:?}", r);
    }

    #[test]
    fn type3_evaluator_rejects_malformed_local_shape_fields() {
        let sub = PdfObject::Dictionary(make_type2_dict(&[0.0], &[1.0], 1.0));
        let reader = crate::reader::PdfReader::from_bytes(super::tests_minimal_pdf()).unwrap();
        let base = dict(&[
            ("FunctionType", PdfObject::Integer(3)),
            (
                "Domain",
                PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
            ),
            ("Functions", PdfObject::Array(vec![sub])),
            ("Bounds", PdfObject::Array(vec![])),
            (
                "Encode",
                PdfObject::Array(vec![PdfObject::Real(0.0), PdfObject::Real(1.0)]),
            ),
        ]);

        let mut missing_domain = base.clone();
        missing_domain.remove("Domain");
        assert!(eval_type3(&missing_domain, 0.5, &reader).is_empty());

        let mut malformed_bounds = base.clone();
        malformed_bounds.insert(
            "Bounds",
            PdfObject::Array(vec![PdfObject::Name("Bad".to_string())]),
        );
        assert!(eval_type3(&malformed_bounds, 0.5, &reader).is_empty());

        let mut short_encode = base;
        short_encode.insert("Encode", PdfObject::Array(vec![PdfObject::Real(0.0)]));
        assert!(eval_type3(&short_encode, 0.5, &reader).is_empty());
    }

    #[test]
    fn bool_pair_reads_extend() {
        let d = dict(&[(
            "Extend",
            PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(false)]),
        )]);
        assert_eq!(get_bool_pair(&d, "Extend").unwrap(), [true, false]);
    }

    #[test]
    fn bool_pair_missing_is_none() {
        assert!(get_bool_pair(&PdfDictionary::empty(), "Extend").is_none());
    }

    #[test]
    fn bool_pair_rejects_malformed_values() {
        let non_boolean = dict(&[(
            "Extend",
            PdfObject::Array(vec![
                PdfObject::Boolean(true),
                PdfObject::Name("Bad".to_string()),
            ]),
        )]);
        let too_long = dict(&[(
            "Extend",
            PdfObject::Array(vec![
                PdfObject::Boolean(true),
                PdfObject::Boolean(false),
                PdfObject::Boolean(true),
            ]),
        )]);

        assert!(get_bool_pair(&non_boolean, "Extend").is_none());
        assert!(get_bool_pair(&too_long, "Extend").is_none());
    }

    #[test]
    fn direct_shading_helpers_reject_malformed_local_shape_fields() {
        let function_base = dict(&[
            ("ShadingType", PdfObject::Integer(1)),
            ("ColorSpace", PdfObject::Name("DeviceRGB".to_string())),
            ("Domain", mesh_decode_array(&[0.0, 1.0, 0.0, 1.0])),
            ("Function", red_rgb_function()),
        ]);

        let mut bad_function_domain = function_base.clone();
        bad_function_domain.insert(
            "Domain",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(1.0),
                PdfObject::Real(0.0),
                PdfObject::Name("Bad".to_string()),
            ]),
        );
        assert_shading_helper_does_not_paint(
            ShadingRenderer::paint_function_based,
            bad_function_domain,
        );

        let mut bad_function_matrix = function_base.clone();
        bad_function_matrix.insert("Matrix", mesh_decode_array(&[1.0, 0.0]));
        assert_shading_helper_does_not_paint(
            ShadingRenderer::paint_function_based,
            bad_function_matrix,
        );

        let mut singular_function_matrix = function_base;
        singular_function_matrix
            .insert("Matrix", mesh_decode_array(&[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
        assert_shading_helper_does_not_paint(
            ShadingRenderer::paint_function_based,
            singular_function_matrix,
        );

        let axial_base = dict(&[
            ("ShadingType", PdfObject::Integer(2)),
            ("ColorSpace", PdfObject::Name("DeviceRGB".to_string())),
            ("Coords", mesh_decode_array(&[0.0, 2.0, 4.0, 2.0])),
            (
                "Extend",
                PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
            ),
            ("Domain", mesh_decode_array(&[0.0, 1.0])),
            ("Function", red_rgb_function()),
        ]);

        let mut bad_axial_coords = axial_base.clone();
        bad_axial_coords.insert(
            "Coords",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(2.0),
                PdfObject::Real(4.0),
                PdfObject::Real(2.0),
                PdfObject::Name("Bad".to_string()),
            ]),
        );
        assert_shading_helper_does_not_paint(ShadingRenderer::paint_axial, bad_axial_coords);

        let mut bad_axial_domain = axial_base.clone();
        bad_axial_domain.insert(
            "Domain",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Name("Bad".to_string()),
            ]),
        );
        assert_shading_helper_does_not_paint(ShadingRenderer::paint_axial, bad_axial_domain);

        let mut bad_axial_extend = axial_base;
        bad_axial_extend.insert(
            "Extend",
            PdfObject::Array(vec![
                PdfObject::Boolean(true),
                PdfObject::Name("Bad".to_string()),
            ]),
        );
        assert_shading_helper_does_not_paint(ShadingRenderer::paint_axial, bad_axial_extend);

        let radial_base = dict(&[
            ("ShadingType", PdfObject::Integer(3)),
            ("ColorSpace", PdfObject::Name("DeviceRGB".to_string())),
            ("Coords", mesh_decode_array(&[0.0, 2.0, 4.0, 4.0, 2.0, 4.0])),
            (
                "Extend",
                PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
            ),
            ("Domain", mesh_decode_array(&[0.0, 1.0])),
            ("Function", red_rgb_function()),
        ]);

        let mut bad_radial_coords = radial_base.clone();
        bad_radial_coords.insert(
            "Coords",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Real(2.0),
                PdfObject::Real(4.0),
                PdfObject::Real(4.0),
                PdfObject::Real(2.0),
                PdfObject::Real(4.0),
                PdfObject::Name("Bad".to_string()),
            ]),
        );
        assert_shading_helper_does_not_paint(ShadingRenderer::paint_radial, bad_radial_coords);

        let mut bad_radial_domain = radial_base.clone();
        bad_radial_domain.insert(
            "Domain",
            PdfObject::Array(vec![
                PdfObject::Real(0.0),
                PdfObject::Name("Bad".to_string()),
            ]),
        );
        assert_shading_helper_does_not_paint(ShadingRenderer::paint_radial, bad_radial_domain);

        let mut bad_radial_extend = radial_base;
        bad_radial_extend.insert(
            "Extend",
            PdfObject::Array(vec![
                PdfObject::Boolean(true),
                PdfObject::Name("Bad".to_string()),
            ]),
        );
        assert_shading_helper_does_not_paint(ShadingRenderer::paint_radial, bad_radial_extend);
    }

    #[test]
    fn direct_shading_helpers_reject_singular_paint_ctm() {
        let singular_ctm = Transform2D::scale(0.0, 0.0);
        let function_shading = dict(&[
            ("ShadingType", PdfObject::Integer(1)),
            ("ColorSpace", PdfObject::Name("DeviceRGB".to_string())),
            ("Domain", mesh_decode_array(&[0.0, 1.0, 0.0, 1.0])),
            ("Function", red_rgb_function()),
        ]);
        assert_shading_helper_with_ctm_does_not_paint(
            ShadingRenderer::paint_function_based,
            function_shading,
            singular_ctm,
        );

        let axial_shading = dict(&[
            ("ShadingType", PdfObject::Integer(2)),
            ("ColorSpace", PdfObject::Name("DeviceRGB".to_string())),
            ("Coords", mesh_decode_array(&[0.0, 2.0, 4.0, 2.0])),
            (
                "Extend",
                PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
            ),
            ("Domain", mesh_decode_array(&[0.0, 1.0])),
            ("Function", red_rgb_function()),
        ]);
        assert_shading_helper_with_ctm_does_not_paint(
            ShadingRenderer::paint_axial,
            axial_shading,
            singular_ctm,
        );

        let radial_shading = dict(&[
            ("ShadingType", PdfObject::Integer(3)),
            ("ColorSpace", PdfObject::Name("DeviceRGB".to_string())),
            ("Coords", mesh_decode_array(&[0.0, 2.0, 0.0, 4.0, 2.0, 4.0])),
            (
                "Extend",
                PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
            ),
            ("Domain", mesh_decode_array(&[0.0, 1.0])),
            ("Function", red_rgb_function()),
        ]);
        assert_shading_helper_with_ctm_does_not_paint(
            ShadingRenderer::paint_radial,
            radial_shading,
            singular_ctm,
        );
    }

    #[test]
    fn direct_shading_helpers_reject_unsupported_color_space_locally() {
        let shading = dict(&[
            ("ShadingType", PdfObject::Integer(2)),
            ("ColorSpace", PdfObject::Name("UnknownSpace".to_string())),
            ("Coords", mesh_decode_array(&[0.0, 2.0, 4.0, 2.0])),
            (
                "Extend",
                PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
            ),
            ("Domain", mesh_decode_array(&[0.0, 1.0])),
            ("Function", red_rgb_function()),
        ]);

        assert_shading_helper_does_not_paint(ShadingRenderer::paint_axial, shading);
    }

    #[test]
    fn direct_shading_paint_rejects_missing_color_space_instead_of_default_rgb() {
        let shading = dict(&[
            ("ShadingType", PdfObject::Integer(2)),
            ("Coords", mesh_decode_array(&[0.0, 2.0, 4.0, 2.0])),
            (
                "Extend",
                PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
            ),
            (
                "Function",
                PdfObject::Dictionary(make_type2_dict(&[1.0, 0.0, 0.0], &[1.0, 0.0, 0.0], 1.0)),
            ),
        ]);
        let reader = crate::reader::PdfReader::from_bytes(super::tests_minimal_pdf()).unwrap();
        let viewport = Viewport::new([0.0, 0.0, 4.0, 4.0], 72);
        let mut buf = PixelBuffer::new_filled(4, 4, crate::render::buffer::WHITE);

        ShadingRenderer::paint(
            &shading,
            &Transform2D::identity(),
            &viewport,
            &mut buf,
            &reader,
            None,
        );

        assert_eq!(buf.get_pixel(2, 2), crate::render::buffer::WHITE);
    }

    #[test]
    fn direct_indexed_constant_axial_shading_paints_palette_color() {
        let shading = dict(&[
            ("ShadingType", PdfObject::Integer(2)),
            ("Coords", mesh_decode_array(&[0.0, 2.0, 4.0, 2.0])),
            (
                "Extend",
                PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
            ),
            (
                "ColorSpace",
                PdfObject::Array(vec![
                    PdfObject::Name("Indexed".to_string()),
                    PdfObject::Name("DeviceRGB".to_string()),
                    PdfObject::Integer(1),
                    PdfObject::String(vec![255, 0, 0, 0, 255, 0]),
                ]),
            ),
            (
                "Function",
                PdfObject::Dictionary(make_type2_dict(&[1.0], &[1.0], 1.0)),
            ),
        ]);
        let reader = crate::reader::PdfReader::from_bytes(super::tests_minimal_pdf()).unwrap();
        let viewport = Viewport::new([0.0, 0.0, 4.0, 4.0], 72);
        let mut buf = PixelBuffer::new_filled(4, 4, crate::render::buffer::WHITE);

        ShadingRenderer::paint(
            &shading,
            &Transform2D::identity(),
            &viewport,
            &mut buf,
            &reader,
            None,
        );

        assert_eq!(buf.get_pixel(2, 2), [0, 255, 0, 255]);
    }

    #[test]
    fn direct_indexed_non_integer_shading_index_rejects_paint() {
        let shading = dict(&[
            ("ShadingType", PdfObject::Integer(2)),
            ("Coords", mesh_decode_array(&[0.0, 2.0, 4.0, 2.0])),
            (
                "Extend",
                PdfObject::Array(vec![PdfObject::Boolean(true), PdfObject::Boolean(true)]),
            ),
            (
                "ColorSpace",
                PdfObject::Array(vec![
                    PdfObject::Name("Indexed".to_string()),
                    PdfObject::Name("DeviceRGB".to_string()),
                    PdfObject::Integer(1),
                    PdfObject::String(vec![255, 0, 0, 0, 255, 0]),
                ]),
            ),
            (
                "Function",
                PdfObject::Dictionary(make_type2_dict(&[0.5], &[0.5], 1.0)),
            ),
        ]);
        let reader = crate::reader::PdfReader::from_bytes(super::tests_minimal_pdf()).unwrap();
        let viewport = Viewport::new([0.0, 0.0, 4.0, 4.0], 72);
        let mut buf = PixelBuffer::new_filled(4, 4, crate::render::buffer::WHITE);

        ShadingRenderer::paint(
            &shading,
            &Transform2D::identity(),
            &viewport,
            &mut buf,
            &reader,
            None,
        );

        assert_eq!(buf.get_pixel(2, 2), crate::render::buffer::WHITE);
    }

    #[test]
    fn components_to_pixel_rgb() {
        let c = components_to_pixel(&[1.0, 0.5, 0.0], "DeviceRGB");
        assert_eq!(c[0], 255);
        assert!((c[1] as i32 - 128).abs() <= 1, "G≈128: {}", c[1]);
        assert_eq!(c[2], 0);
        assert_eq!(c[3], 255);
    }

    #[test]
    fn components_to_pixel_gray() {
        let c = components_to_pixel(&[0.5], "DeviceGray");
        assert!((c[0] as i32 - 128).abs() <= 2, "gray≈128: {}", c[0]);
        assert_eq!(c[0], c[1]);
        assert_eq!(c[0], c[2]);
    }

    #[test]
    fn components_to_pixel_cmyk_black_and_white() {
        let black = components_to_pixel(&[0.0, 0.0, 0.0, 1.0], "DeviceCMYK");
        assert_eq!(black, [35, 31, 32, 255]);
        let white = components_to_pixel(&[0.0, 0.0, 0.0, 0.0], "DeviceCMYK");
        assert_eq!(white, [255, 255, 255, 255]);
    }

    #[test]
    fn shading_color_cache_keeps_sub_byte_gradient_steps() {
        let func = PdfObject::Dictionary(make_type2_dict(&[0.0], &[1.0], 1.0));
        let reader = crate::reader::PdfReader::from_bytes(super::tests_minimal_pdf()).unwrap();
        let func = crate::render::function::PreparedFunction::prepare(&func, 1, &reader).unwrap();
        let mut cache = ShadingColorCache::new();

        let c0 = ShadingRenderer::color_for(
            0.1000,
            0.0,
            1.0,
            &func,
            "DeviceGray",
            None,
            &reader,
            &mut cache,
            ShadingRenderOptions::default(),
        )
        .expect("color at first sample");
        let c1 = ShadingRenderer::color_for(
            0.1010,
            0.0,
            1.0,
            &func,
            "DeviceGray",
            None,
            &reader,
            &mut cache,
            ShadingRenderOptions::default(),
        )
        .expect("color at nearby sample");

        assert!(
            c1.r > c0.r && (c1.r - c0.r) > 0.0005,
            "cache should preserve sub-byte progression: {c0:?} -> {c1:?}"
        );
    }

    fn longest_run(values: &[u8]) -> usize {
        let mut longest = 0usize;
        let mut current = 0usize;
        let mut previous = None;
        for &value in values {
            if previous == Some(value) {
                current += 1;
            } else {
                current = 1;
                previous = Some(value);
            }
            longest = longest.max(current);
        }
        longest
    }

    #[test]
    fn ordered_dither_breaks_long_quantization_runs_and_is_deterministic() {
        const W: i32 = 1024;
        let plain: Vec<u8> = (0..W)
            .map(|x| {
                let t = 0.49 + 0.02 * x as f32 / (W - 1) as f32;
                quantize_shading_color(RenderColor::gray(t), x, 0, false)[0]
            })
            .collect();
        let dithered: Vec<u8> = (0..W)
            .map(|x| {
                let t = 0.49 + 0.02 * x as f32 / (W - 1) as f32;
                quantize_shading_color(RenderColor::gray(t), x, 0, true)[0]
            })
            .collect();
        let dithered_again: Vec<u8> = (0..W)
            .map(|x| {
                let t = 0.49 + 0.02 * x as f32 / (W - 1) as f32;
                quantize_shading_color(RenderColor::gray(t), x, 0, true)[0]
            })
            .collect();

        assert_eq!(dithered, dithered_again);
        assert!(
            longest_run(&plain) > 100,
            "undithered shallow gradient should visibly band"
        );
        assert!(
            longest_run(&dithered) < longest_run(&plain) / 4,
            "dithered gradient should break long runs: plain {}, dithered {}",
            longest_run(&plain),
            longest_run(&dithered)
        );
    }

    #[test]
    fn ordered_dither_does_not_texture_exact_byte_flat_colors() {
        for y in 0..8 {
            for x in 0..8 {
                assert_eq!(
                    quantize_shading_color(RenderColor::gray(128.0 / 255.0), x, y, true),
                    [128, 128, 128, 255]
                );
            }
        }
    }

    // ---- Coons/tensor shared-edge patch reconstruction ---------------------

    fn rc(r: f32, g: f32, b: f32, a: f32) -> RenderColor {
        RenderColor::new(r, g, b, a)
    }

    /// Helper: a deterministic 12-point "previous" patch where point i is
    /// (i*10, i*10) and corner colors are distinguishable. Lets us assert exact
    /// index reuse for each flag.
    fn prev_patch() -> (Vec<(f64, f64)>, Vec<RenderColor>) {
        let pts: Vec<(f64, f64)> = (0..12)
            .map(|i| (i as f64 * 10.0, i as f64 * 10.0))
            .collect();
        // 4 corner colors, each tagged in its red channel by its index.
        let cols: Vec<RenderColor> = (0..4).map(|i| rc(i as f32 / 10.0, 0.0, 0.0, 1.0)).collect();
        (pts, cols)
    }

    #[test]
    fn assemble_patch_flag0_is_independent() {
        // Flag 0: all 12 points and 4 colors come straight from the stream.
        let new_pts: Vec<(f64, f64)> = (0..12).map(|i| (i as f64, 0.0)).collect();
        let new_cols: Vec<RenderColor> = (0..4).map(|_| rc(0.0, 0.0, 0.0, 1.0)).collect();
        let (pts, cols) =
            assemble_patch(0, &new_pts, &new_cols, &[], &[], 6).expect("flag 0 must assemble");
        assert_eq!(pts.len(), 12);
        assert_eq!(cols.len(), 4);
        assert_eq!(pts[0], (0.0, 0.0));
        assert_eq!(pts[11], (11.0, 0.0));
    }

    #[test]
    fn assemble_patch_flag1_reuses_edge_p4_p7_and_colors_c2_c3() {
        let (pp, pc) = prev_patch();
        // 8 new boundary points (p5..p12) + 2 new colors (c3, c4).
        let new_pts: Vec<(f64, f64)> = (0..8).map(|i| (100.0 + i as f64, -1.0)).collect();
        let new_cols = vec![rc(0.7, 0.0, 0.0, 1.0), rc(0.8, 0.0, 0.0, 1.0)];
        let (pts, cols) =
            assemble_patch(1, &new_pts, &new_cols, &pp, &pc, 6).expect("flag 1 must assemble");
        // Shared edge p1..p4 = prev indices [3,4,5,6].
        assert_eq!(pts[0], pp[3]);
        assert_eq!(pts[1], pp[4]);
        assert_eq!(pts[2], pp[5]);
        assert_eq!(pts[3], pp[6]);
        // Remaining 8 are the new points.
        assert_eq!(pts[4], new_pts[0]);
        assert_eq!(pts[11], new_pts[7]);
        // Shared colors c1,c2 = prev colors [1,2]; new colors fill c3,c4.
        assert_eq!(cols[0], pc[1]);
        assert_eq!(cols[1], pc[2]);
        assert_eq!(cols[2], new_cols[0]);
        assert_eq!(cols[3], new_cols[1]);
    }

    #[test]
    fn assemble_patch_flag2_reuses_edge_p7_p10_and_colors_c3_c4() {
        let (pp, pc) = prev_patch();
        let new_pts: Vec<(f64, f64)> = (0..8).map(|i| (200.0 + i as f64, -2.0)).collect();
        let new_cols = vec![rc(0.6, 0.0, 0.0, 1.0), rc(0.9, 0.0, 0.0, 1.0)];
        let (pts, cols) =
            assemble_patch(2, &new_pts, &new_cols, &pp, &pc, 6).expect("flag 2 must assemble");
        assert_eq!(pts[0], pp[6]);
        assert_eq!(pts[1], pp[7]);
        assert_eq!(pts[2], pp[8]);
        assert_eq!(pts[3], pp[9]);
        assert_eq!(cols[0], pc[2]);
        assert_eq!(cols[1], pc[3]);
    }

    #[test]
    fn assemble_patch_flag3_reuses_edge_p10_p1_and_colors_c4_c1() {
        let (pp, pc) = prev_patch();
        let new_pts: Vec<(f64, f64)> = (0..8).map(|i| (300.0 + i as f64, -3.0)).collect();
        let new_cols = vec![rc(0.5, 0.0, 0.0, 1.0), rc(0.4, 0.0, 0.0, 1.0)];
        let (pts, cols) =
            assemble_patch(3, &new_pts, &new_cols, &pp, &pc, 6).expect("flag 3 must assemble");
        // Shared edge p1..p4 = prev indices [9,10,11,0] (wraps to p1).
        assert_eq!(pts[0], pp[9]);
        assert_eq!(pts[1], pp[10]);
        assert_eq!(pts[2], pp[11]);
        assert_eq!(pts[3], pp[0]);
        assert_eq!(cols[0], pc[3]);
        assert_eq!(cols[1], pc[0]);
    }

    #[test]
    fn assemble_tensor_patch_retains_interior_points() {
        // Tensor flag 0: all 16 stream points are kept so the tensor surface can
        // evaluate its four interior controls.
        let new_pts: Vec<(f64, f64)> = (0..16).map(|i| (i as f64, 0.0)).collect();
        let new_cols: Vec<RenderColor> = (0..4).map(|_| rc(0.0, 0.0, 0.0, 1.0)).collect();
        let (pts, _cols) =
            assemble_patch(0, &new_pts, &new_cols, &[], &[], 7).expect("tensor flag 0");
        assert_eq!(pts.len(), 16);
        assert_eq!(pts[11], (11.0, 0.0));
        assert_eq!(pts[15], (15.0, 0.0));
    }

    #[test]
    fn assemble_tensor_flag1_shares_edge_uses_12_new_points() {
        // Tensor flagged patch reads 12 new points (8 boundary p5..p12 + 4
        // interior), all appended after the shared edge.
        let (mut pp, pc) = prev_patch();
        pp.extend((12..16).map(|i| (i as f64 * 10.0, i as f64 * 10.0)));
        let new_pts: Vec<(f64, f64)> = (0..12).map(|i| (100.0 + i as f64, -1.0)).collect();
        let new_cols = vec![rc(0.7, 0.0, 0.0, 1.0), rc(0.8, 0.0, 0.0, 1.0)];
        let (pts, cols) =
            assemble_patch(1, &new_pts, &new_cols, &pp, &pc, 7).expect("tensor flag 1");
        assert_eq!(pts.len(), 16);
        assert_eq!(pts[0], pp[3]); // shared edge
        assert_eq!(pts[4], new_pts[0]); // first new boundary point
        assert_eq!(pts[11], new_pts[7]); // 8th new boundary point
        assert_eq!(pts[15], new_pts[11]); // 4th interior point
        assert_eq!(cols[0], pc[1]);
    }

    #[test]
    fn tensor_point_uses_interior_controls() {
        let mut pts = vec![
            (0.0, 0.0),
            (0.0, 0.3),
            (0.0, 0.7),
            (0.0, 1.0),
            (0.3, 1.0),
            (0.7, 1.0),
            (1.0, 1.0),
            (1.0, 0.7),
            (1.0, 0.3),
            (1.0, 0.0),
            (0.7, 0.0),
            (0.3, 0.0),
            (0.25, 0.85),
            (0.25, 0.95),
            (0.75, 0.95),
            (0.75, 0.85),
        ];
        let lifted = tensor_point(&pts, 0.5, 0.5);
        pts[12] = (0.25, 0.15);
        pts[13] = (0.25, 0.05);
        pts[14] = (0.75, 0.05);
        pts[15] = (0.75, 0.15);
        let lowered = tensor_point(&pts, 0.5, 0.5);
        assert!(
            (lifted.1 - lowered.1).abs() > 0.15,
            "tensor interior controls should move the surface: lifted={lifted:?} lowered={lowered:?}"
        );
    }
}
