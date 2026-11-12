//! PDFium-compatible shading rasterization for the compat render mode.
//!
//! Mirrors PDFium's `cpdf_rendershading.cpp` so compat renders match the
//! reference renderer pixel for pixel:
//!
//! - axial, radial and mesh shadings with a /Function use a 256-entry colour
//!   table sampled at `t_min + (t_max - t_min) * i / 256`;
//! - analytic shadings sample at pixel corners (in double precision, so tiles
//!   match full renders) and truncate the parameter to a table index (radial
//!   shadings keep PDFium's degenerate-cone and extension rules);
//! - mesh vertex colours are converted to RGB as they are read and the RGB
//!   values are interpolated (so Lab and Indexed meshes blend in RGB);
//! - Gouraud triangles are scan-converted against integer scanlines;
//! - Coons and tensor patches are subdivided until the corner colours differ by
//!   less than 4/255 (or the patch is under 2px) and each piece is filled as a
//!   flat region bounded by its outer Bézier control points, painting every
//!   pixel the region touches.
use super::*;

const STEPS: usize = 256;
const COON_COLOR_THRESHOLD: i32 = 4;
const MAX_PATCH_DEPTH: u32 = 30;

type Steps = Vec<PixelColor>;

/// Colour conversion with PDFium's component rules: missing function outputs
/// are zero, palette indices are truncated and an out-of-range index is black
/// (`GetRGBOrZerosOnError`). A colour space that cannot convert still fails
/// the shading.
struct CompatColor<'a> {
    color_space: String,
    color_space_obj: Option<&'a PdfObject>,
    reader: &'a PdfReader,
    options: ShadingRenderOptions<'a>,
    components: usize,
    indexed_hival: Option<i64>,
}

impl<'a> CompatColor<'a> {
    fn new(
        dict: &'a PdfDictionary,
        reader: &'a PdfReader,
        options: ShadingRenderOptions<'a>,
    ) -> Option<Self> {
        let color_space = shading_color_space_name(dict)?;
        let color_space_obj = shading_color_space_object(dict);
        let components = color_space_component_count(&color_space, color_space_obj, reader)
            .unwrap_or(1)
            .max(1);
        let indexed_hival = if matches!(color_space.as_str(), "Indexed" | "I") {
            Some(
                color_space_obj
                    .and_then(PdfObject::as_array)
                    .and_then(|array| array.get(2))
                    .and_then(PdfObject::as_number)
                    .map_or(0, |value| value as i64),
            )
        } else {
            None
        };
        Some(Self {
            color_space,
            color_space_obj,
            reader,
            options,
            components,
            indexed_hival,
        })
    }

    fn rgb(&self, components: &[f64]) -> RenderColor {
        let black = RenderColor::new(0.0, 0.0, 0.0, 1.0);
        let mut values = components.to_vec();
        if values.len() < self.components {
            values.resize(self.components, 0.0);
        }
        if let Some(hival) = self.indexed_hival {
            // CPDF_IndexedCS::GetRGB truncates the index toward zero.
            let index = values[0].trunc();
            if !index.is_finite() || index < 0.0 || index > hival as f64 {
                return black;
            }
            values.truncate(1);
            values[0] = index;
        }
        components_to_render_color_with_space(
            &values,
            &self.color_space,
            self.color_space_obj,
            self.reader,
            self.options,
        )
        .unwrap_or(black)
    }
}

/// `FXSYS_roundf(c * 255)` per channel.
fn rounded_pixel(color: RenderColor) -> PixelColor {
    let byte = |c: f32| (c * 255.0).round().clamp(0.0, 255.0) as u8;
    [byte(color.r), byte(color.g), byte(color.b), byte(color.a)]
}

/// `static_cast<int32_t>(float)` as executed on x86: values outside the i32
/// range (and NaN) become `INT32_MIN`.
fn trunc_i32(value: f32) -> i32 {
    trunc_index(f64::from(value))
}

/// [`trunc_i32`] for a double-precision value.
fn trunc_index(value: f64) -> i32 {
    if value.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

/// PDFium's `GetShadingSteps`.
fn shading_steps(
    t_min: f32,
    t_max: f32,
    function: &crate::render::function::PreparedFunction,
    color: &CompatColor<'_>,
) -> Option<Steps> {
    let diff = t_max - t_min;
    let mut steps = Vec::with_capacity(STEPS);
    for i in 0..STEPS {
        let input = diff * i as f32 / STEPS as f32 + t_min;
        let components = color
            .options
            .evaluate_function(function, &[f64::from(input)]);
        if components.is_empty() {
            color
                .options
                .fail("shading function produced no components");
        }
        if color.options.failed() {
            return None;
        }
        steps.push(rounded_pixel(color.rgb(&components)));
    }
    Some(steps)
}

/// Shading parameters (0 at the start of the axis, 1 at its end) where the
/// colour function jumps: the `/Bounds` of a stitching function (or of one
/// in a per-component function array) whose neighbouring sub-functions do
/// not meet there.
///
/// PDFium's 256-entry table and pixel-corner sampling approximate a smooth
/// gradient to within a pixel; at a jump that error shows as an edge drawn
/// up to a pixel (or 1/255 of the axis) away from where the geometry puts
/// it. [`edge_index`] places such an edge like a path edge instead.
fn colour_jumps(
    dict: &PdfDictionary,
    reader: &PdfReader,
    (t_min, t_max): (f32, f32),
    function: &crate::render::function::PreparedFunction,
    color: &CompatColor<'_>,
) -> Vec<f64> {
    let (t_min, t_max) = (f64::from(t_min), f64::from(t_max));
    let span = t_max - t_min;
    if !span.is_finite() || span == 0.0 {
        return Vec::new();
    }
    let resolve = |object: &PdfObject| reader.resolve(object.clone()).ok();
    let mut functions = Vec::new();
    match dict.get("Function").and_then(resolve) {
        Some(PdfObject::Array(items)) => functions.extend(items.iter().filter_map(resolve)),
        Some(object) => functions.push(object),
        None => {}
    }
    let mut bounds = Vec::new();
    for function in &functions {
        let function = match function {
            PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => dict,
            _ => continue,
        };
        if function.get_integer("FunctionType") != Some(3) {
            continue;
        }
        if let Some(PdfObject::Array(values)) = function.get("Bounds").and_then(resolve) {
            bounds.extend(values.iter().filter_map(PdfObject::as_number));
        }
    }
    bounds.sort_by(f64::total_cmp);
    bounds.dedup();
    let colour_at = |t: f64| {
        let components = color.options.evaluate_function(function, &[t]);
        (!components.is_empty()).then(|| rounded_pixel(color.rgb(&components)))
    };
    let mut jumps = Vec::new();
    for bound in bounds {
        let s = (bound - t_min) / span;
        if !(s > 0.0 && s < 1.0) {
            continue;
        }
        let epsilon = 1e-6 * span.abs().max(bound.abs()).max(1e-6);
        let (Some(left), Some(right)) = (colour_at(bound - epsilon), colour_at(bound)) else {
            continue;
        };
        if left
            .iter()
            .zip(right.iter())
            .any(|(l, r)| l.abs_diff(*r) > 1)
        {
            jumps.push(s);
        }
    }
    jumps
}

/// The table entry for a pixel whose corner selects `index` and whose
/// centre lies at shading parameter `centre`: when a colour jump separates
/// the entry from the pixel centre, the pixel takes the closest entry on the
/// centre's side of the jump, so the jump is drawn where a path edge at the
/// same place would be (each pixel shows the colour covering its centre).
fn edge_index(index: usize, centre: f64, jumps: &[f64]) -> usize {
    let entry = index as f64 / STEPS as f64;
    for &jump in jumps {
        let entry_after = entry >= jump;
        let centre_after = centre >= jump;
        if entry_after != centre_after {
            let first_after = ((jump * STEPS as f64).ceil() as usize).min(STEPS - 1);
            return if centre_after {
                first_after
            } else {
                first_after.saturating_sub(1)
            };
        }
    }
    index
}

fn domain_pair(dict: &PdfDictionary) -> (f32, f32) {
    match get_float_array(dict, "Domain") {
        Some(values) if values.len() >= 2 => (values[0] as f32, values[1] as f32),
        _ => (0.0, 1.0),
    }
}

fn extend_pair(dict: &PdfDictionary) -> [bool; 2] {
    get_bool_pair(dict, "Extend").unwrap_or([false, false])
}

/// Pixel-space → shading-space mapping. The analytic shadings follow
/// PDFium's algorithms in double precision, so a pixel's value does not
/// depend on the render window it is drawn in.
struct PixelMatrix {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl PixelMatrix {
    fn new(transform: &Transform2D) -> Self {
        Self {
            a: transform.a,
            b: transform.b,
            c: transform.c,
            d: transform.d,
            e: transform.e,
            f: transform.f,
        }
    }

    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

fn prepare_analytic<'a>(
    dict: &'a PdfDictionary,
    ctm: &Transform2D,
    viewport: &Viewport,
    reader: &'a PdfReader,
    options: ShadingRenderOptions<'a>,
    inputs: usize,
) -> Option<(
    crate::render::function::FunctionLease,
    CompatColor<'a>,
    Transform2D,
    (f64, f64),
)> {
    let Some(function) = dict
        .get("Function")
        .and_then(|object| options.prepare_function(object, inputs, reader))
    else {
        options.fail("shading function preparation failed or exceeded limits");
        return None;
    };
    let Some(color) = CompatColor::new(dict, reader, options) else {
        options.fail("shading colour space is missing");
        return None;
    };
    // Sample in page pixel coordinates: the scratch window's offset is
    // added to each pixel, so a pixel's value does not depend on the window
    // (tile) it is rendered in.
    let page = Viewport {
        origin_x_px: 0,
        origin_y_px: 0,
        ..viewport.clone()
    };
    let Some(pixel_to_user) = ShadingRenderer::pixel_to_user(ctm, &page) else {
        options.fail("analytic shading has singular or unrepresentable coordinate mapping");
        return None;
    };
    let origin = (
        f64::from(viewport.origin_x_px),
        f64::from(viewport.origin_y_px),
    );
    Some((function, color, pixel_to_user, origin))
}

/// PDFium's `DrawAxialShading`.
pub(super) fn paint_axial(
    dict: &PdfDictionary,
    ctm: &Transform2D,
    viewport: &Viewport,
    buf: &mut PixelBuffer,
    reader: &PdfReader,
    cancel: &CancelToken,
    options: ShadingRenderOptions<'_>,
) {
    let Some(coords) = get_float_array(dict, "Coords").filter(|c| c.len() >= 4) else {
        return;
    };
    let Some((function, color, pixel_to_user, (ox, oy))) =
        prepare_analytic(dict, ctm, viewport, reader, options, 1)
    else {
        return;
    };
    let (start_x, start_y) = (coords[0], coords[1]);
    let (end_x, end_y) = (coords[2], coords[3]);
    let (t_min, t_max) = domain_pair(dict);
    let [start_extend, end_extend] = extend_pair(dict);
    let x_span = end_x - start_x;
    let y_span = end_y - start_y;
    let axis_len_square = x_span * x_span + y_span * y_span;
    if axis_len_square == 0.0 {
        return;
    }
    let Some(steps) = shading_steps(t_min, t_max, &function, &color) else {
        return;
    };
    let jumps = colour_jumps(dict, reader, (t_min, t_max), &function, &color);
    let matrix = PixelMatrix::new(&pixel_to_user);
    let delta_scale = (matrix.a * x_span + matrix.b * y_span) / axis_len_square;
    // Parameter step from a pixel's corner to its centre.
    let centre_delta =
        0.5 * delta_scale + 0.5 * (matrix.c * x_span + matrix.d * y_span) / axis_len_square;
    let Some((x0, y0, x1, y1)) = ShadingRenderer::paint_bounds(buf) else {
        return;
    };
    for row in y0..y1 {
        if cancel.is_cancelled() || options.failed() {
            return;
        }
        let (px, py) = matrix.apply(0.0, row as f64 + oy);
        let scale_0 = ((px - start_x) * x_span + (py - start_y) * y_span) / axis_len_square;
        for column in x0..x1 {
            if !options.sample_visible(column, row) {
                continue;
            }
            let scale = scale_0 + (column as f64 + ox) * delta_scale;
            let mut index = trunc_index(scale * (STEPS - 1) as f64);
            if index < 0 {
                if !start_extend {
                    continue;
                }
                index = 0;
            } else if index >= STEPS as i32 {
                if !end_extend {
                    continue;
                }
                index = STEPS as i32 - 1;
            }
            let mut index = index as usize;
            if !jumps.is_empty() {
                index = edge_index(index, scale + centre_delta, &jumps);
            }
            buf.set_pixel(column, row, steps[index]);
        }
    }
}

/// PDFium's `DrawRadialShading`.
pub(super) fn paint_radial(
    dict: &PdfDictionary,
    ctm: &Transform2D,
    viewport: &Viewport,
    buf: &mut PixelBuffer,
    reader: &PdfReader,
    cancel: &CancelToken,
    options: ShadingRenderOptions<'_>,
) {
    let Some(coords) = get_float_array(dict, "Coords").filter(|c| c.len() >= 6) else {
        return;
    };
    let Some((function, color, pixel_to_user, (ox, oy))) =
        prepare_analytic(dict, ctm, viewport, reader, options, 1)
    else {
        return;
    };
    let (start_x, start_y, start_r) = (coords[0], coords[1], coords[2]);
    let (end_x, end_y, end_r) = (coords[3], coords[4], coords[5]);
    let (t_min, t_max) = domain_pair(dict);
    let [start_extend, end_extend] = extend_pair(dict);
    let Some(steps) = shading_steps(t_min, t_max, &function, &color) else {
        return;
    };
    let jumps = colour_jumps(dict, reader, (t_min, t_max), &function, &color);
    let dx = end_x - start_x;
    let dy = end_y - start_y;
    let dr = end_r - start_r;
    let a = dx * dx + dy * dy - dr * dr;
    let float_zero = |value: f64| value < 0.0001 && value > -0.0001;
    let a_is_zero = float_zero(a);
    let decreasing = dr < 0.0 && (trunc_index(dx.hypot(dy)) as f64) < -dr;
    let matrix = PixelMatrix::new(&pixel_to_user);
    // The shading parameter at a device point, or `None` where PDFium
    // leaves the pixel unpainted.
    let parameter = |x: f64, y: f64| -> Option<f64> {
        let (px, py) = matrix.apply(x, y);
        let pos_dx = px - start_x;
        let pos_dy = py - start_y;
        let b = -2.0 * (pos_dx * dx + pos_dy * dy + start_r * dr);
        let c = pos_dx * pos_dx + pos_dy * pos_dy - start_r * start_r;
        if float_zero(b) {
            return Some((-c / a).sqrt());
        }
        if a_is_zero {
            return Some(-c / b);
        }
        let discriminant = b * b - 4.0 * (a * c);
        if discriminant < 0.0 {
            return None;
        }
        let root = discriminant.sqrt();
        let mut s1 = (-b - root) / (2.0 * a);
        let mut s2 = (-b + root) / (2.0 * a);
        if a <= 0.0 {
            std::mem::swap(&mut s1, &mut s2);
        }
        let s = if decreasing {
            if s1 >= 0.0 || start_extend {
                s1
            } else {
                s2
            }
        } else if s2 <= 1.0 || end_extend {
            s2
        } else {
            s1
        };
        (start_r + s * dr >= 0.0).then_some(s)
    };
    let Some((x0, y0, x1, y1)) = ShadingRenderer::paint_bounds(buf) else {
        return;
    };
    for row in y0..y1 {
        if cancel.is_cancelled() || options.failed() {
            return;
        }
        for column in x0..x1 {
            if !options.sample_visible(column, row) {
                continue;
            }
            let (x, y) = (column as f64 + ox, row as f64 + oy);
            let Some(s) = parameter(x, y) else {
                continue;
            };
            let mut index = trunc_index(s * (STEPS - 1) as f64);
            if index < 0 {
                if !start_extend {
                    continue;
                }
                index = 0;
            } else if index >= STEPS as i32 {
                if !end_extend {
                    continue;
                }
                index = STEPS as i32 - 1;
            }
            let mut index = index as usize;
            if !jumps.is_empty() {
                if let Some(centre) = parameter(x + 0.5, y + 0.5) {
                    index = edge_index(index, centre, &jumps);
                }
            }
            buf.set_pixel(column, row, steps[index]);
        }
    }
}

/// PDFium's `DrawFuncShading`.
pub(super) fn paint_function_based(
    dict: &PdfDictionary,
    ctm: &Transform2D,
    viewport: &Viewport,
    buf: &mut PixelBuffer,
    reader: &PdfReader,
    cancel: &CancelToken,
    options: ShadingRenderOptions<'_>,
) {
    let Some((function, color, pixel_to_user, (ox, oy))) =
        prepare_analytic(dict, ctm, viewport, reader, options, 2)
    else {
        return;
    };
    let (xmin, xmax, ymin, ymax) = match get_float_array(dict, "Domain") {
        Some(d) if d.len() >= 4 => (d[0], d[1], d[2], d[3]),
        _ => (0.0, 1.0, 0.0, 1.0),
    };
    let domain_to_target = match get_float_array(dict, "Matrix") {
        Some(m) if m.len() >= 6 => Transform2D::from([m[0], m[1], m[2], m[3], m[4], m[5]]),
        _ => Transform2D::identity(),
    };
    let Some(target_to_domain) = geometry::inverse(&domain_to_target) else {
        return;
    };
    let matrix = PixelMatrix::new(&pixel_to_user.concat(&target_to_domain));
    let Some((x0, y0, x1, y1)) = ShadingRenderer::paint_bounds(buf) else {
        return;
    };
    let byte = |c: f32| trunc_index(f64::from(c) * 255.0).clamp(0, 255) as u8;
    for row in y0..y1 {
        if cancel.is_cancelled() || options.failed() {
            return;
        }
        for column in x0..x1 {
            if !options.sample_visible(column, row) {
                continue;
            }
            let (x, y) = matrix.apply(column as f64 + ox, row as f64 + oy);
            if x < xmin || x > xmax || y < ymin || y > ymax {
                continue;
            }
            let components = options.evaluate_function(&function, &[x, y]);
            if components.is_empty() {
                options.fail("shading function produced no components");
            }
            if options.failed() {
                return;
            }
            let rgb = color.rgb(&components);
            buf.set_pixel(
                column,
                row,
                [byte(rgb.r), byte(rgb.g), byte(rgb.b), byte(rgb.a)],
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Mesh shadings
// ---------------------------------------------------------------------------

/// A mesh vertex in bitmap space with its RGB colour (or, with a /Function,
/// its colour-table index in `rgb[0]`).
#[derive(Clone, Copy, Default)]
struct Vertex {
    x: f32,
    y: f32,
    rgb: [f32; 3],
}

/// PDFium's `CPDF_MeshStream`.
struct MeshStream<'a, 'b> {
    bits: crate::render::function::BitReader<'b>,
    dec: MeshDecode,
    to_device: Transform2D,
    color: &'a CompatColor<'a>,
    function: bool,
}

impl MeshStream<'_, '_> {
    fn is_eof(&self) -> bool {
        self.bits.bits_remaining() == 0
    }
    fn can_read_flag(&self) -> bool {
        self.bits.bits_remaining() >= self.dec.bits_per_flag
    }
    fn can_read_coords(&self) -> bool {
        self.bits.bits_remaining() / 2 >= self.dec.bits_per_coord
    }
    fn can_read_color(&self) -> bool {
        self.bits.bits_remaining() / self.dec.bits_per_comp >= self.dec.n_color
    }
    fn read_flag(&mut self) -> u32 {
        self.bits.read(self.dec.bits_per_flag).unwrap_or(0) & 0x03
    }
    fn read_coords(&mut self) -> Option<(f32, f32)> {
        let max = crate::render::function::max_value(self.dec.bits_per_coord);
        let mut axis = |lo: f64, hi: f64| {
            let raw = f64::from(self.bits.read(self.dec.bits_per_coord).unwrap_or(0));
            lo + raw * (hi - lo) / max
        };
        let x = axis(self.dec.decode[0], self.dec.decode[1]);
        let y = axis(self.dec.decode[2], self.dec.decode[3]);
        let (dx, dy) = self.to_device.transform_point(x, y);
        let point = (dx as f32, dy as f32);
        if !point.0.is_finite() || !point.1.is_finite() {
            self.color.options.fail("nonfinite mesh vertex geometry");
            return None;
        }
        Some(point)
    }
    /// The vertex colour converted to RGB, or the raw parametric value when
    /// a /Function maps it later.
    fn read_color(&mut self) -> [f32; 3] {
        let max = crate::render::function::max_value(self.dec.bits_per_comp) as f32;
        let mut values = [0.0f64; crate::render::colorspace::MAX_DEVICEN_COMPONENTS];
        for (i, value) in values.iter_mut().enumerate().take(self.dec.n_color) {
            let lo = self.dec.decode[4 + 2 * i] as f32;
            let hi = self.dec.decode[5 + 2 * i] as f32;
            let raw = self.bits.read(self.dec.bits_per_comp).unwrap_or(0) as f32;
            *value = f64::from(lo + raw * (hi - lo) / max);
        }
        if self.function {
            return [values[0] as f32, 0.0, 0.0];
        }
        let rgb = self.color.rgb(&values[..self.dec.n_color]);
        [rgb.r, rgb.g, rgb.b]
    }
    /// One free-form vertex record. Unlike PDFium, a truncated record fails
    /// the whole shading rather than publishing the triangles before it.
    fn read_vertex(&mut self) -> Option<(u32, Vertex)> {
        if !self.color.options.charge_work(1) {
            return None;
        }
        if !self.can_read_flag() {
            self.color.options.fail("truncated free-form mesh vertex");
            return None;
        }
        let flag = self.read_flag();
        if !self.can_read_coords() {
            self.color.options.fail("truncated free-form mesh vertex");
            return None;
        }
        let (x, y) = self.read_coords()?;
        if !self.can_read_color() {
            self.color.options.fail("truncated free-form mesh vertex");
            return None;
        }
        let rgb = self.read_color();
        self.bits.align_to_byte();
        Some((flag, Vertex { x, y, rgb }))
    }
    fn read_vertex_row(&mut self, count: usize) -> Option<Vec<Vertex>> {
        let mut row = Vec::new();
        if row.try_reserve_exact(count).is_err() {
            self.color.options.fail("lattice row allocation failed");
            return None;
        }
        for _ in 0..count {
            if !self.color.options.charge_work(1) {
                return None;
            }
            if !self.can_read_coords() {
                self.color
                    .options
                    .fail("incomplete or invalid lattice vertex row");
                return None;
            }
            let (x, y) = self.read_coords()?;
            if !self.can_read_color() {
                self.color
                    .options
                    .fail("incomplete or invalid lattice vertex row");
                return None;
            }
            let rgb = self.read_color();
            self.bits.align_to_byte();
            row.push(Vertex { x, y, rgb });
        }
        Some(row)
    }
    fn component_range(&self) -> (f32, f32) {
        (self.dec.decode[4] as f32, self.dec.decode[5] as f32)
    }
}

/// PDFium's `ComponentToShadingIndex`.
fn shading_index(c: f32, c_min: f32, c_max: f32) -> f32 {
    if c_min == c_max {
        0.0
    } else {
        (c - c_min) / (c_max - c_min) * (STEPS - 1) as f32
    }
}

struct MeshSetup<'a> {
    color: CompatColor<'a>,
    function: Option<crate::render::function::FunctionLease>,
    dec: MeshDecode,
    to_device: Transform2D,
}

fn prepare_mesh<'a>(
    dict: &'a PdfDictionary,
    ctm: &Transform2D,
    viewport: &Viewport,
    reader: &'a PdfReader,
    options: ShadingRenderOptions<'a>,
) -> Option<MeshSetup<'a>> {
    let color = CompatColor::new(dict, reader, options)?;
    let dec = MeshDecode::from_dict(dict, &color.color_space, color.color_space_obj, reader)?;
    let function = match dict.get("Function") {
        None | Some(PdfObject::Null) => None,
        Some(object) => match options.prepare_function(object, 1, reader) {
            Some(function) => Some(function),
            None => {
                options.fail("mesh function preparation failed or exceeded limits");
                return None;
            }
        },
    };
    Some(MeshSetup {
        color,
        function,
        dec,
        to_device: ctm.concat(&viewport.to_transform()),
    })
}

fn mesh_steps(setup: &MeshSetup<'_>) -> Option<Option<Steps>> {
    match setup.function.as_deref() {
        None => Some(None),
        Some(function) => {
            let (lo, hi) = (setup.dec.decode[4] as f32, setup.dec.decode[5] as f32);
            shading_steps(lo, hi, function, &setup.color).map(Some)
        }
    }
}

/// PDFium's `GetScanlineIntersect`.
fn scanline_intersect(y: i32, first: &Vertex, second: &Vertex) -> Option<f32> {
    if first.y == second.y {
        return None;
    }
    let yf = y as f32;
    if first.y < second.y {
        if yf < first.y || yf > second.y {
            return None;
        }
    } else if yf < second.y || yf > first.y {
        return None;
    }
    Some(first.x + (second.x - first.x) * (yf - first.y) / (second.y - first.y))
}

/// PDFium's `DrawGouraud`.
fn draw_gouraud(
    buf: &mut PixelBuffer,
    triangle: &[Vertex; 3],
    steps: Option<&Steps>,
    options: ShadingRenderOptions<'_>,
) {
    let min_y = triangle.iter().map(|v| v.y).fold(triangle[0].y, f32::min);
    let max_y = triangle.iter().map(|v| v.y).fold(triangle[0].y, f32::max);
    if min_y == max_y || !min_y.is_finite() || !max_y.is_finite() {
        return;
    }
    let width = buf.width as i32;
    let height = buf.height as i32;
    let min_yi = trunc_i32(min_y.floor()).max(0);
    let max_yi = trunc_i32(max_y.ceil()).min(height - 1);
    if max_yi >= min_yi && !options.charge_work(u64::from((max_yi - min_yi + 1).unsigned_abs()) * 4)
    {
        return;
    }
    let to_byte = |value: f32| trunc_i32(value * 255.0).clamp(0, 255) as u8;
    for y in min_yi..=max_yi {
        let mut count = 0usize;
        let mut inter_x = [0.0f32; 3];
        let mut colors = [[0.0f32; 3]; 3];
        for i in 0..3 {
            let v1 = &triangle[i];
            let v2 = &triangle[(i + 1) % 3];
            let Some(x) = scanline_intersect(y, v1, v2) else {
                continue;
            };
            let y_dist = (y as f32 - v1.y) / (v2.y - v1.y);
            inter_x[count] = x;
            for k in 0..3 {
                colors[count][k] = v1.rgb[k] + (v2.rgb[k] - v1.rgb[k]) * y_dist;
            }
            count += 1;
        }
        if count != 2 {
            continue;
        }
        let (min_x, max_x, start, end) = if inter_x[0] < inter_x[1] {
            (
                trunc_i32(inter_x[0].floor()),
                trunc_i32(inter_x[1].ceil()),
                0,
                1,
            )
        } else {
            (
                trunc_i32(inter_x[1].floor()),
                trunc_i32(inter_x[0].ceil()),
                1,
                0,
            )
        };
        let start_x = min_x.clamp(0, width);
        let end_x = max_x.clamp(0, width);
        if start_x >= end_x {
            continue;
        }
        let range_x = max_x.saturating_sub(min_x) as f32;
        let unit: [f32; 3] = std::array::from_fn(|k| (colors[end][k] - colors[start][k]) / range_x);
        let diff_x = start_x.saturating_sub(min_x) as f32;
        let mut result: [f32; 3] = std::array::from_fn(|k| colors[start][k] + diff_x * unit[k]);
        for x in start_x..end_x {
            result[0] += unit[0];
            let pixel = if let Some(steps) = steps {
                steps[trunc_i32(result[0]).clamp(0, STEPS as i32 - 1) as usize]
            } else {
                result[1] += unit[1];
                result[2] += unit[2];
                [
                    to_byte(result[0]),
                    to_byte(result[1]),
                    to_byte(result[2]),
                    255,
                ]
            };
            if options.sample_visible(x, y) {
                buf.set_pixel(x, y, pixel);
            }
        }
    }
}

/// PDFium's `DrawFreeGouraudShading` and `DrawLatticeGouraudShading`.
pub(super) fn paint_gouraud_mesh(
    dict: &PdfDictionary,
    ctm: &Transform2D,
    viewport: &Viewport,
    buf: &mut PixelBuffer,
    reader: &PdfReader,
    mesh_data: Option<&[u8]>,
    options: ShadingRenderOptions<'_>,
) {
    let Some(data) = mesh_data else {
        return;
    };
    let lattice = dict.get_integer("ShadingType") == Some(5);
    let row_verts = if lattice {
        match dict.get_integer("VerticesPerRow") {
            Some(value) if value >= 2 => match usize::try_from(value) {
                Ok(value) => value,
                Err(_) => return,
            },
            _ => return,
        }
    } else {
        0
    };
    let Some(setup) = prepare_mesh(dict, ctm, viewport, reader, options) else {
        return;
    };
    let Some(steps) = mesh_steps(&setup) else {
        return;
    };
    let mut stream = MeshStream {
        bits: crate::render::function::BitReader::new(data),
        dec: setup.dec,
        to_device: setup.to_device,
        color: &setup.color,
        function: setup.function.is_some(),
    };
    let (c_min, c_max) = stream.component_range();
    let to_index = |mut vertex: Vertex| {
        if steps.is_some() {
            vertex.rgb[0] = shading_index(vertex.rgb[0], c_min, c_max);
        }
        vertex
    };
    if lattice {
        // The stream must hold whole rows; an untrusted VerticesPerRow must
        // not size allocations by itself.
        let vertex_bits =
            2 * stream.dec.bits_per_coord + stream.dec.n_color * stream.dec.bits_per_comp;
        let row_bytes = vertex_bits.div_ceil(8).saturating_mul(row_verts);
        if row_bytes == 0 || data.len() % row_bytes != 0 {
            options.fail("lattice stream does not contain complete rows");
            return;
        }
        let _row_memory = match options.reserve_mesh_storage(row_verts.saturating_mul(2)) {
            Ok(memory) => memory,
            Err(_) => {
                options.fail("lattice working-memory budget exceeded");
                return;
            }
        };
        let Some(first) = stream.read_vertex_row(row_verts) else {
            return;
        };
        let mut previous: Vec<Vertex> = first.into_iter().map(to_index).collect();
        loop {
            if options.failed() || crate::cancel::check_current_cancel("lattice mesh").is_err() {
                return;
            }
            if stream.is_eof() {
                return;
            }
            let Some(next) = stream.read_vertex_row(row_verts) else {
                return;
            };
            let next: Vec<Vertex> = next.into_iter().map(to_index).collect();
            for i in 1..row_verts {
                let mut triangle = [previous[i], next[i - 1], previous[i - 1]];
                draw_gouraud(buf, &triangle, steps.as_ref(), options);
                triangle[2] = next[i];
                draw_gouraud(buf, &triangle, steps.as_ref(), options);
            }
            previous = next;
        }
    }
    let mut triangle = [Vertex::default(); 3];
    let mut anchored = false;
    while !stream.is_eof() {
        if options.failed() || crate::cancel::check_current_cancel("free-form mesh").is_err() {
            return;
        }
        let Some((flag, vertex)) = stream.read_vertex() else {
            return;
        };
        let vertex = to_index(vertex);
        if flag == 0 {
            triangle[0] = vertex;
            for slot in triangle.iter_mut().skip(1) {
                let Some((_, next)) = stream.read_vertex() else {
                    return;
                };
                *slot = to_index(next);
            }
        } else {
            if !anchored {
                options.fail("mesh edge reuse has no preceding triangle");
                return;
            }
            if flag == 1 {
                triangle[0] = triangle[1];
            }
            triangle[1] = triangle[2];
            triangle[2] = vertex;
        }
        anchored = true;
        draw_gouraud(buf, &triangle, steps.as_ref(), options);
    }
}

// ---------------------------------------------------------------------------
// Patch meshes
// ---------------------------------------------------------------------------

type Point = (f32, f32);

#[derive(Clone, Copy, Default)]
struct BezierPatch {
    points: [[Point; 4]; 4],
}

fn mid(a: Point, b: Point) -> Point {
    (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1))
}

impl BezierPatch {
    fn bbox(&self) -> (f32, f32, f32, f32) {
        let mut bbox = (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        );
        for p in self.points.iter().flatten() {
            bbox.0 = bbox.0.min(p.0);
            bbox.1 = bbox.1.min(p.1);
            bbox.2 = bbox.2.max(p.0);
            bbox.3 = bbox.3.max(p.1);
        }
        bbox
    }

    fn is_small(&self) -> bool {
        let (x0, y0, x1, y1) = self.bbox();
        x1 - x0 < 2.0 && y1 - y0 < 2.0
    }

    fn boundary(&self) -> [Point; 13] {
        let p = &self.points;
        [
            p[0][0], p[0][1], p[0][2], p[0][3], p[1][3], p[2][3], p[3][3], p[3][2], p[3][1],
            p[3][0], p[2][0], p[1][0], p[0][0],
        ]
    }

    fn split(a: Point, b: Point, c: Point, d: Point) -> ([Point; 4], [Point; 4]) {
        let l1 = [mid(a, b), mid(b, c), mid(c, d)];
        let l2 = [mid(l1[0], l1[1]), mid(l1[1], l1[2])];
        let l3 = mid(l2[0], l2[1]);
        ([a, l1[0], l2[0], l3], [l3, l2[1], l1[2], d])
    }

    fn subdivide_vertical(&self) -> (Self, Self) {
        let mut top = Self::default();
        let mut bottom = Self::default();
        for x in 0..4 {
            let p = &self.points[x];
            let (first, second) = Self::split(p[0], p[1], p[2], p[3]);
            top.points[x] = first;
            bottom.points[x] = second;
        }
        (top, bottom)
    }

    fn subdivide_horizontal(&self) -> (Self, Self) {
        let mut left = Self::default();
        let mut right = Self::default();
        for y in 0..4 {
            let p = &self.points;
            let (first, second) = Self::split(p[0][y], p[1][y], p[2][y], p[3][y]);
            for x in 0..4 {
                left.points[x][y] = first[x];
                right.points[x][y] = second[x];
            }
        }
        (left, right)
    }
}

type CoonColor = [i32; 3];

/// PDFium's overflow-checked `Interpolate`.
fn interpolate(p1: i32, p2: i32, delta1: i32, delta2: i32, overflow: &mut bool) -> i32 {
    let value = p2
        .checked_sub(p1)
        .and_then(|v| v.checked_mul(delta1))
        .and_then(|v| v.checked_div(delta2))
        .and_then(|v| v.checked_add(p1));
    value.unwrap_or_else(|| {
        *overflow = true;
        0
    })
}

fn bi_interpol(
    colors: &[CoonColor; 4],
    x: i32,
    y: i32,
    x_scale: i32,
    y_scale: i32,
) -> Option<CoonColor> {
    let mut overflow = false;
    let mut result = [0; 3];
    for (i, value) in result.iter_mut().enumerate() {
        let x1 = interpolate(colors[0][i], colors[3][i], x, x_scale, &mut overflow);
        let x2 = interpolate(colors[1][i], colors[2][i], x, x_scale, &mut overflow);
        *value = interpolate(x1, x2, y, y_scale, &mut overflow);
    }
    (!overflow).then_some(result)
}

fn distance(a: &CoonColor, b: &CoonColor) -> i32 {
    (0..3)
        .map(|i| (a[i] - b[i]).saturating_abs())
        .max()
        .unwrap_or(0)
}

struct PatchDrawer<'a, 'b> {
    buf: &'a mut PixelBuffer,
    options: ShadingRenderOptions<'b>,
    colors: [CoonColor; 4],
    steps: Option<&'a Steps>,
    bounds: (i32, i32, i32, i32),
}

impl PatchDrawer<'_, '_> {
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        x_scale: i32,
        y_scale: i32,
        left: i32,
        bottom: i32,
        patch: &BezierPatch,
        depth: u32,
    ) {
        if self.options.failed() || depth > MAX_PATCH_DEPTH {
            return;
        }
        // Pieces wholly outside the painted area cannot touch it: the region
        // lies inside the control-point hull.
        let (bx0, by0, bx1, by1) = patch.bbox();
        let (x0, y0, x1, y1) = self.bounds;
        if !(bx1 >= x0 as f32 && bx0 <= x1 as f32 && by1 >= y0 as f32 && by0 <= y1 as f32) {
            return;
        }
        if !self.options.charge_work(1) {
            return;
        }
        let small = patch.is_small();
        let Some(c0) = bi_interpol(&self.colors, left, bottom, x_scale, y_scale) else {
            return;
        };
        let (mut d_bottom, mut d_left, mut d_top, mut d_right) = (0, 0, 0, 0);
        if !small {
            let (Some(c1), Some(c2), Some(c3)) = (
                bi_interpol(&self.colors, left, bottom + 1, x_scale, y_scale),
                bi_interpol(&self.colors, left + 1, bottom + 1, x_scale, y_scale),
                bi_interpol(&self.colors, left + 1, bottom, x_scale, y_scale),
            ) else {
                return;
            };
            d_bottom = distance(&c3, &c0);
            d_left = distance(&c1, &c0);
            d_top = distance(&c1, &c2);
            d_right = distance(&c2, &c3);
        }
        let t = COON_COLOR_THRESHOLD;
        if small || (d_bottom < t && d_left < t && d_top < t && d_right < t) {
            let color = match self.steps {
                Some(steps) => steps[c0[0].clamp(0, STEPS as i32 - 1) as usize],
                None => [
                    c0[0].clamp(0, 255) as u8,
                    c0[1].clamp(0, 255) as u8,
                    c0[2].clamp(0, 255) as u8,
                    255,
                ],
            };
            self.fill_boundary(&patch.boundary(), color);
        } else if d_bottom < t && d_top < t {
            let (top, bottom_patch) = patch.subdivide_vertical();
            let (y_scale, bottom) = (y_scale.saturating_mul(2), bottom.saturating_mul(2));
            self.draw(x_scale, y_scale, left, bottom, &top, depth + 1);
            self.draw(x_scale, y_scale, left, bottom + 1, &bottom_patch, depth + 1);
        } else if d_left < t && d_right < t {
            let (left_patch, right_patch) = patch.subdivide_horizontal();
            let (x_scale, left) = (x_scale.saturating_mul(2), left.saturating_mul(2));
            self.draw(x_scale, y_scale, left, bottom, &left_patch, depth + 1);
            self.draw(x_scale, y_scale, left + 1, bottom, &right_patch, depth + 1);
        } else {
            let (top, bottom_patch) = patch.subdivide_vertical();
            let (top_left, top_right) = top.subdivide_horizontal();
            let (bottom_left, bottom_right) = bottom_patch.subdivide_horizontal();
            let (x_scale, y_scale) = (x_scale.saturating_mul(2), y_scale.saturating_mul(2));
            let (left, bottom) = (left.saturating_mul(2), bottom.saturating_mul(2));
            self.draw(x_scale, y_scale, left, bottom, &top_left, depth + 1);
            self.draw(x_scale, y_scale, left, bottom + 1, &bottom_left, depth + 1);
            self.draw(x_scale, y_scale, left + 1, bottom, &top_right, depth + 1);
            self.draw(
                x_scale,
                y_scale,
                left + 1,
                bottom + 1,
                &bottom_right,
                depth + 1,
            );
        }
    }

    /// Fill the closed Bézier boundary (move + four cubic segments) with the
    /// nonzero rule, painting every pixel with any coverage at full opacity
    /// (AGG's `full_cover`).
    fn fill_boundary(&mut self, boundary: &[Point; 13], color: PixelColor) {
        let mut polygon = Vec::with_capacity(64);
        polygon.push(boundary[0]);
        for segment in 0..4 {
            let base = segment * 3;
            flatten_cubic(
                boundary[base],
                boundary[base + 1],
                boundary[base + 2],
                boundary[base + 3],
                &mut polygon,
            );
        }
        fill_full_cover(self.buf, &polygon, color, self.bounds, self.options);
    }
}

/// Recursive midpoint subdivision until the control points lie within a
/// quarter pixel of the chord (AGG's default curve approximation).
fn flatten_cubic(p0: Point, p1: Point, p2: Point, p3: Point, out: &mut Vec<Point>) {
    fn flat_enough(p0: Point, p1: Point, p2: Point, p3: Point) -> bool {
        let ux = 3.0 * p1.0 - 2.0 * p0.0 - p3.0;
        let uy = 3.0 * p1.1 - 2.0 * p0.1 - p3.1;
        let vx = 3.0 * p2.0 - p0.0 - 2.0 * p3.0;
        let vy = 3.0 * p2.1 - p0.1 - 2.0 * p3.1;
        // Upper bound of the squared distance between curve and chord, x16.
        (ux * ux).max(vx * vx) + (uy * uy).max(vy * vy) <= 16.0 * 0.0625
    }
    fn recurse(p0: Point, p1: Point, p2: Point, p3: Point, depth: u32, out: &mut Vec<Point>) {
        if depth >= 16 || flat_enough(p0, p1, p2, p3) {
            out.push(p3);
            return;
        }
        let (a, b) = BezierPatch::split(p0, p1, p2, p3);
        recurse(a[0], a[1], a[2], a[3], depth + 1, out);
        recurse(b[0], b[1], b[2], b[3], depth + 1, out);
    }
    recurse(p0, p1, p2, p3, 0, out);
}

/// Exact-area scanline coverage of `polygon` (implicitly closed) inside
/// `bounds`; every pixel with at least 1/256 coverage is set to `color`.
fn fill_full_cover(
    buf: &mut PixelBuffer,
    polygon: &[Point],
    color: PixelColor,
    bounds: (i32, i32, i32, i32),
    options: ShadingRenderOptions<'_>,
) {
    if polygon.len() < 3 || polygon.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
        return;
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    );
    for p in polygon {
        min_x = min_x.min(p.0);
        min_y = min_y.min(p.1);
        max_x = max_x.max(p.0);
        max_y = max_y.max(p.1);
    }
    let left = (min_x.floor() as i32).max(bounds.0);
    let right = (max_x.ceil() as i32).min(bounds.2);
    let top = (min_y.floor() as i32).max(bounds.1);
    let bottom = (max_y.ceil() as i32).min(bounds.3);
    if left >= right || top >= bottom {
        return;
    }
    let width = (right - left) as usize;
    let height = (bottom - top) as usize;
    let stride = width + 2;
    if !options.charge_work((stride * height) as u64) {
        return;
    }
    let mut area = vec![0.0f32; stride * height];
    let (lf, rf, tf) = (left as f32, right as f32, top as f32);
    // Clamping x into [left, right] preserves the winding seen by every pixel
    // inside the window; edges beyond it only shift their contribution onto
    // the window border.
    let map = |p: &Point| (p.0.clamp(lf, rf) - lf, p.1 - tf);
    for i in 0..polygon.len() {
        let a = map(&polygon[i]);
        let b = map(&polygon[(i + 1) % polygon.len()]);
        accumulate_line(&mut area, stride, height, a, b);
    }
    for row in 0..height {
        let y = top + row as i32;
        let mut acc = 0.0f32;
        for col in 0..width {
            acc += area[row * stride + col];
            if acc.abs() >= 1.0 / 256.0 - 1e-5 {
                let x = left + col as i32;
                if options.sample_visible(x, y) {
                    buf.set_pixel(x, y, color);
                }
            }
        }
    }
}

/// Signed-area accumulation of one edge (font-rs style); a prefix sum along
/// each row yields the winding-weighted coverage.
fn accumulate_line(area: &mut [f32], stride: usize, height: usize, p0: Point, p1: Point) {
    if (p0.1 - p1.1).abs() <= f32::EPSILON {
        return;
    }
    let (dir, p0, p1) = if p0.1 < p1.1 {
        (1.0f32, p0, p1)
    } else {
        (-1.0f32, p1, p0)
    };
    let dxdy = (p1.0 - p0.0) / (p1.1 - p0.1);
    let mut x = p0.0;
    if p0.1 < 0.0 {
        x -= p0.1 * dxdy;
    }
    let y_start = p0.1.max(0.0) as usize;
    let y_end = (p1.1.ceil().max(0.0) as usize).min(height);
    let max_index = stride - 1;
    for y in y_start..y_end {
        let line = y * stride;
        let dy = ((y + 1) as f32).min(p1.1) - (y as f32).max(p0.1);
        let x_next = (x + dxdy * dy).max(0.0);
        let d = dy * dir;
        let (x0, x1) = if x < x_next { (x, x_next) } else { (x_next, x) };
        let x0_floor = x0.floor();
        let x0i = x0_floor as usize;
        let x1_ceil = x1.ceil();
        let x1i = x1_ceil as usize;
        let at = |i: usize| line + i.min(max_index);
        if x1i <= x0i + 1 {
            let xmf = 0.5 * (x + x_next) - x0_floor;
            area[at(x0i)] += d - d * xmf;
            area[at(x0i + 1)] += d * xmf;
        } else {
            let s = (x1 - x0).recip();
            let x0f = x0 - x0_floor;
            let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
            let x1f = x1 - x1_ceil + 1.0;
            let am = 0.5 * s * x1f * x1f;
            area[at(x0i)] += d * a0;
            if x1i == x0i + 2 {
                area[at(x0i + 1)] += d * (1.0 - a0 - am);
            } else {
                let a1 = s * (1.5 - x0f);
                area[at(x0i + 1)] += d * (a1 - a0);
                for xi in x0i + 2..x1i - 1 {
                    area[at(xi)] += d * s;
                }
                let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                area[at(x1i - 1)] += d * (1.0 - a2 - am);
            }
            area[at(x1i)] += d * am;
        }
        x = x_next;
    }
}

/// PDFium's `DrawCoonPatchMeshes`.
pub(super) fn paint_patch_mesh(
    dict: &PdfDictionary,
    ctm: &Transform2D,
    viewport: &Viewport,
    buf: &mut PixelBuffer,
    reader: &PdfReader,
    mesh_data: Option<&[u8]>,
    options: ShadingRenderOptions<'_>,
) {
    let Some(data) = mesh_data else {
        return;
    };
    let tensor = dict.get_integer("ShadingType") == Some(7);
    let Some(setup) = prepare_mesh(dict, ctm, viewport, reader, options) else {
        return;
    };
    let Some(steps) = mesh_steps(&setup) else {
        return;
    };
    let Some(bounds) = ShadingRenderer::paint_bounds(buf) else {
        return;
    };
    let mut stream = MeshStream {
        bits: crate::render::function::BitReader::new(data),
        dec: setup.dec,
        to_device: setup.to_device,
        color: &setup.color,
        function: setup.function.is_some(),
    };
    let (c_min, c_max) = stream.component_range();
    let (width, height) = (buf.width as f32, buf.height as f32);
    let point_count = if tensor { 16 } else { 12 };
    let mut coords = [(0.0f32, 0.0f32); 16];
    let mut drawer = PatchDrawer {
        buf,
        options,
        colors: [[0; 3]; 4],
        steps: steps.as_ref(),
        bounds,
    };
    let mut patches = 0usize;
    while !stream.is_eof() {
        if options.failed() || crate::cancel::check_current_cancel("patch mesh").is_err() {
            return;
        }
        if !stream.can_read_flag() {
            options.fail("truncated patch mesh flag");
            return;
        }
        patches += 1;
        if patches > MAX_PATCH_MESH_PATCHES * 16 {
            options.fail("patch mesh exceeds patch-count budget");
            return;
        }
        let flag = stream.read_flag();
        if flag != 0 && patches == 1 {
            options.fail("invalid patch mesh edge reuse");
            return;
        }
        let (start_point, start_color) = if flag != 0 {
            let shared: [Point; 4] = std::array::from_fn(|i| coords[(flag as usize * 3 + i) % 12]);
            coords[..4].copy_from_slice(&shared);
            let shared_colors = [
                drawer.colors[flag as usize],
                drawer.colors[(flag as usize + 1) % 4],
            ];
            drawer.colors[..2].copy_from_slice(&shared_colors);
            (4, 2)
        } else {
            (0, 0)
        };
        if !options.charge_work((point_count + 4) as u64) {
            return;
        }
        for coord in coords.iter_mut().take(point_count).skip(start_point) {
            if !stream.can_read_coords() {
                options.fail("truncated patch mesh control points");
                return;
            }
            let Some(point) = stream.read_coords() else {
                return;
            };
            *coord = point;
        }
        for i in start_color..4 {
            if !stream.can_read_color() {
                options.fail("invalid or truncated patch mesh colours");
                return;
            }
            let rgb = stream.read_color();
            drawer.colors[i] = if steps.is_some() {
                [trunc_i32(shading_index(rgb[0], c_min, c_max)), 0, 0]
            } else {
                [
                    trunc_i32(rgb[0] * 255.0),
                    trunc_i32(rgb[1] * 255.0),
                    trunc_i32(rgb[2] * 255.0),
                ]
            };
        }
        let mut bbox = (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        );
        for p in &coords[..point_count] {
            bbox.0 = bbox.0.min(p.0);
            bbox.1 = bbox.1.min(p.1);
            bbox.2 = bbox.2.max(p.0);
            bbox.3 = bbox.3.max(p.1);
        }
        if bbox.2 <= 0.0 || bbox.0 >= width || bbox.3 <= 0.0 || bbox.1 >= height {
            continue;
        }
        let mut patch = BezierPatch::default();
        {
            let p = &mut patch.points;
            p[0][0] = coords[0];
            p[0][1] = coords[1];
            p[0][2] = coords[2];
            p[0][3] = coords[3];
            p[1][3] = coords[4];
            p[2][3] = coords[5];
            p[3][3] = coords[6];
            p[3][2] = coords[7];
            p[3][1] = coords[8];
            p[3][0] = coords[9];
            p[2][0] = coords[10];
            p[1][0] = coords[11];
            if tensor {
                p[1][1] = coords[12];
                p[1][2] = coords[13];
                p[2][2] = coords[14];
                p[2][1] = coords[15];
            } else {
                // ISO 32000-2 8.7.4.5.8: Coons interior controls.
                let combine = |terms: &[(f32, Point)]| {
                    let (mut x, mut y) = (0.0f32, 0.0f32);
                    for (w, q) in terms {
                        x += w * q.0;
                        y += w * q.1;
                    }
                    ((1.0 / 9.0) * x, (1.0 / 9.0) * y)
                };
                let c = *p;
                p[1][1] = combine(&[
                    (-4.0, c[0][0]),
                    (6.0, c[0][1]),
                    (6.0, c[1][0]),
                    (-2.0, c[0][3]),
                    (-2.0, c[3][0]),
                    (3.0, c[3][1]),
                    (3.0, c[1][3]),
                    (-1.0, c[3][3]),
                ]);
                p[1][2] = combine(&[
                    (-4.0, c[0][3]),
                    (6.0, c[0][2]),
                    (6.0, c[1][3]),
                    (-2.0, c[0][0]),
                    (-2.0, c[3][3]),
                    (3.0, c[3][2]),
                    (3.0, c[1][0]),
                    (-1.0, c[3][0]),
                ]);
                p[2][1] = combine(&[
                    (-4.0, c[3][0]),
                    (6.0, c[3][1]),
                    (6.0, c[2][0]),
                    (-2.0, c[3][3]),
                    (-2.0, c[0][0]),
                    (3.0, c[0][1]),
                    (3.0, c[2][3]),
                    (-1.0, c[0][3]),
                ]);
                p[2][2] = combine(&[
                    (-4.0, c[3][3]),
                    (6.0, c[3][2]),
                    (6.0, c[2][3]),
                    (-2.0, c[3][0]),
                    (-2.0, c[0][3]),
                    (3.0, c[0][2]),
                    (3.0, c[2][0]),
                    (-1.0, c[0][0]),
                ]);
            }
        }
        drawer.draw(1, 1, 0, 0, &patch, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polygon_coverage(polygon: &[Point], w: i32, h: i32) -> Vec<bool> {
        let mut buf = PixelBuffer::new_transparent(w as u32, h as u32);
        fill_full_cover(
            &mut buf,
            polygon,
            [255, 0, 0, 255],
            (0, 0, w, h),
            ShadingRenderOptions::default(),
        );
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| buf.get_pixel(x, y)[3] == 255)
            .collect()
    }

    #[test]
    fn full_cover_fill_paints_every_touched_pixel() {
        // A square from (1.5, 1.5) to (3.5, 3.5) touches pixels 1..=3.
        let covered = polygon_coverage(&[(1.5, 1.5), (3.5, 1.5), (3.5, 3.5), (1.5, 3.5)], 6, 6);
        for y in 0..6 {
            for x in 0..6 {
                let inside = (1..=3).contains(&x) && (1..=3).contains(&y);
                assert_eq!(covered[(y * 6 + x) as usize], inside, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn full_cover_fill_clamps_offscreen_edges_without_losing_winding() {
        let covered = polygon_coverage(
            &[(-50.0, -50.0), (50.0, -50.0), (50.0, 50.0), (-50.0, 50.0)],
            4,
            4,
        );
        assert!(covered.iter().all(|c| *c));
    }

    #[test]
    fn patch_color_interpolation_matches_pdfium_integer_rounding() {
        let colors = [[0, 0, 0], [255, 0, 0], [255, 255, 0], [0, 255, 0]];
        assert_eq!(bi_interpol(&colors, 1, 1, 2, 2), Some([127, 127, 0]));
        assert_eq!(bi_interpol(&colors, 1, 0, 4, 2), Some([0, 63, 0]));
        assert_eq!(distance(&[0, 10, 3], &[4, 2, 3]), 8);
        let overflowing = [[0; 3], [0; 3], [0; 3], [i32::MAX; 3]];
        assert_eq!(bi_interpol(&overflowing, 2, 1, 1, 1), None);
    }

    #[test]
    fn edge_index_moves_a_pixel_to_its_centres_side_of_a_jump() {
        // Corner entry 127 (t = 0.496) before the jump at 0.5, centre after:
        // the first entry at or past the jump (128 = 0.5 * 256).
        assert_eq!(edge_index(127, 0.5025, &[0.5]), 128);
        // Corner and centre on the same side: PDFium's entry is kept.
        assert_eq!(edge_index(126, 0.4975, &[0.5]), 126);
        assert_eq!(edge_index(129, 0.51, &[0.5]), 129);
        // Reversed axis: the corner is past the jump, the centre is not.
        assert_eq!(edge_index(128, 0.499, &[0.5]), 127);
        // The table lag band: corner t in [0.5, 0.502) still maps to 127.
        assert_eq!(edge_index(127, 0.501, &[0.5]), 128);
        assert_eq!(edge_index(40, 0.16, &[]), 40);
    }

    #[test]
    fn x86_truncation_saturates_to_int_min() {
        assert_eq!(trunc_i32(1.9), 1);
        assert_eq!(trunc_i32(-1.9), -1);
        assert_eq!(trunc_i32(f32::NAN), i32::MIN);
        assert_eq!(trunc_i32(3.0e9), i32::MIN);
    }

    #[test]
    fn gouraud_scanlines_interpolate_rgb_like_pdfium() {
        let mut buf = PixelBuffer::new_transparent(8, 8);
        let v = |x, y, r| Vertex {
            x,
            y,
            rgb: [r, 0.0, 0.0],
        };
        draw_gouraud(
            &mut buf,
            &[v(0.0, 0.0, 0.0), v(8.0, 0.0, 1.0), v(0.0, 8.0, 0.0)],
            None,
            ShadingRenderOptions::default(),
        );
        // Row 0 spans x 0..8; values advance one unit before each write.
        assert_eq!(buf.get_pixel(0, 0), [31, 0, 0, 255]);
        assert_eq!(buf.get_pixel(7, 0), [255, 0, 0, 255]);
        assert_eq!(buf.get_pixel(7, 7)[3], 0);
    }
}
