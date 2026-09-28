//! Mesh interpolation stays in source component / function-parameter space.
use super::*;

const MAX_COMPONENTS: usize = crate::render::colorspace::MAX_DEVICEN_COMPONENTS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MeshSample {
    values: [f64; MAX_COMPONENTS],
    count: usize,
}
impl MeshSample {
    pub(super) fn new(values: &[f64]) -> Option<Self> {
        if values.is_empty()
            || values.len() > MAX_COMPONENTS
            || values.iter().any(|v| !v.is_finite())
        {
            return None;
        }
        let mut result = Self {
            values: [0.0; MAX_COMPONENTS],
            count: values.len(),
        };
        result.values[..values.len()].copy_from_slice(values);
        Some(result)
    }
    fn interpolate(samples: &[(&Self, f64)]) -> Option<Self> {
        let count = samples.first()?.0.count;
        let mut result = Self {
            values: [0.0; MAX_COMPONENTS],
            count,
        };
        for (sample, weight) in samples {
            if sample.count != count || !weight.is_finite() {
                return None;
            }
            for i in 0..count {
                result.values[i] += sample.values[i] * weight;
            }
        }
        result.values[..count]
            .iter()
            .all(|v| v.is_finite())
            .then_some(result)
    }
    fn bilinear(corners: &[Self], u: f64, v: f64) -> Option<Self> {
        if corners.len() != 4 || !u.is_finite() || !v.is_finite() {
            return None;
        }
        let (u, v) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        Self::interpolate(&[
            (&corners[0], (1.0 - u) * (1.0 - v)),
            (&corners[1], (1.0 - u) * v),
            (&corners[2], u * v),
            (&corners[3], u * (1.0 - v)),
        ])
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MeshVertex {
    pub(super) dx: f64,
    pub(super) dy: f64,
    pub(super) sample: MeshSample,
}

#[derive(Clone, Copy)]
pub(super) struct MeshPaint<'a> {
    pub(super) function: Option<&'a crate::render::function::PreparedFunction>,
    pub(super) color_space: &'a str,
    pub(super) color_space_obj: Option<&'a PdfObject>,
    pub(super) reader: &'a PdfReader,
    pub(super) options: ShadingRenderOptions<'a>,
    pub(super) patch_corners: Option<&'a [MeshSample]>,
}
impl MeshPaint<'_> {
    fn convert(&self, sample: MeshSample) -> Option<RenderColor> {
        let sample = if let Some(corners) = self.patch_corners {
            if sample.count != 2 {
                self.options
                    .fail("patch vertex has no parameter coordinates");
                return None;
            }
            MeshSample::bilinear(corners, sample.values[0], sample.values[1])?
        } else {
            sample
        };
        if let Some(function) = self.function {
            if sample.count != 1 {
                self.options.fail("mesh function requires one parameter");
                return None;
            }
            let components = self
                .options
                .evaluate_function(function, &sample.values[..1]);
            if components.is_empty() {
                self.options
                    .fail("mesh function evaluation failed at interpolated sample");
                return None;
            }
            components_to_render_color_with_space(
                &components,
                self.color_space,
                self.color_space_obj,
                self.reader,
                self.options,
            )
        } else {
            components_to_render_color_with_space(
                &sample.values[..sample.count],
                self.color_space,
                self.color_space_obj,
                self.reader,
                self.options,
            )
        }
    }
}

/// Canonical endpoint ordering ensures reversed shared edges evaluate with
/// exactly opposite signs rather than two independently rounded determinants.
fn oriented_edge(a: &MeshVertex, b: &MeshVertex, x: f64, y: f64) -> f64 {
    if a.dx.total_cmp(&b.dx).then(a.dy.total_cmp(&b.dy)).is_gt() {
        -edge(b.dx, b.dy, a.dx, a.dy, x, y)
    } else {
        edge(a.dx, a.dy, b.dx, b.dy, x, y)
    }
}
fn inclusive_edge(a: &MeshVertex, b: &MeshVertex) -> bool {
    let (dx, dy) = (b.dx - a.dx, b.dy - a.dy);
    dy > 0.0 || (dy == 0.0 && dx < 0.0)
}

pub(super) fn fill_triangle(
    buf: &mut PixelBuffer,
    v0: MeshVertex,
    mut v1: MeshVertex,
    mut v2: MeshVertex,
    paint: &MeshPaint<'_>,
) {
    let options = paint.options;
    if options.failed() {
        return;
    }
    if [v0.dx, v0.dy, v1.dx, v1.dy, v2.dx, v2.dy]
        .iter()
        .any(|v| !v.is_finite())
    {
        options.fail("nonfinite mesh geometry");
        return;
    }
    let mut area = oriented_edge(&v0, &v1, v2.dx, v2.dy);
    if !area.is_finite() {
        options.fail("mesh triangle area overflow");
        return;
    }
    if area == 0.0 {
        return;
    }
    if area < 0.0 {
        std::mem::swap(&mut v1, &mut v2);
        area = -area;
    }
    let Some((clip_x0, clip_y0, clip_x1, clip_y1)) = ShadingRenderer::paint_bounds(buf) else {
        return;
    };
    let x0 = (v0.dx.min(v1.dx).min(v2.dx).floor() as i32).max(clip_x0);
    let y0 = (v0.dy.min(v1.dy).min(v2.dy).floor() as i32).max(clip_y0);
    let x1 = (v0.dx.max(v1.dx).max(v2.dx).ceil() as i32).min(clip_x1);
    let y1 = (v0.dy.max(v1.dy).max(v2.dy).ceil() as i32).min(clip_y1);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    if !options.charge_work((x1 - x0) as u64 * (y1 - y0) as u64) {
        return;
    }
    let include = [
        inclusive_edge(&v1, &v2),
        inclusive_edge(&v2, &v0),
        inclusive_edge(&v0, &v1),
    ];
    let dither = buf.render_mode().is_high_quality();
    for py in y0..y1 {
        if options.failed() || crate::cancel::check_current_cancel("mesh triangle samples").is_err()
        {
            return;
        }
        for px in x0..x1 {
            if !buf.clip_allows(px, py) || !options.sample_visible(px, py) {
                continue;
            }
            let (x, y) = (px as f64 + 0.5, py as f64 + 0.5);
            let edges = [
                oriented_edge(&v1, &v2, x, y),
                oriented_edge(&v2, &v0, x, y),
                oriented_edge(&v0, &v1, x, y),
            ];
            if edges
                .iter()
                .zip(include)
                .any(|(e, inclusive)| *e < 0.0 || (*e == 0.0 && !inclusive))
            {
                continue;
            }
            let weights = edges.map(|value| value / area);
            let total = weights.iter().sum::<f64>();
            if !total.is_finite() || total <= 0.0 {
                options.fail("invalid mesh barycentric weights");
                return;
            }
            let Some(sample) = MeshSample::interpolate(&[
                (&v0.sample, weights[0] / total),
                (&v1.sample, weights[1] / total),
                (&v2.sample, weights[2] / total),
            ]) else {
                options.fail("mesh source-component interpolation failed");
                return;
            };
            let Some(color) = paint.convert(sample) else {
                options.fail("mesh sample colour conversion failed");
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

#[path = "shading_patch.rs"]
mod patch;
pub(super) use patch::PatchBatch;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_mesh_calculators_use_parameters_and_debit_cumulative_work() {
        use crate::render::parameter_dictionary::tests::numbers;
        let reader = PdfReader::from_bytes(tests_minimal_pdf()).unwrap();
        let mut dict = PdfDictionary::empty();
        dict.insert("FunctionType", PdfObject::Integer(4));
        dict.insert("Domain", numbers(&[0.0, 1.0]));
        dict.insert("Range", numbers(&[0.0, 1.0]));
        let object = PdfObject::Stream {
            dict,
            raw: b"{ dup mul }".to_vec(),
        };
        let function =
            crate::render::function::PreparedFunction::prepare(&object, 1, &reader).unwrap();
        let failure = Cell::new(None);
        let work = AtomicU64::new(100);
        let options = ShadingRenderOptions {
            failure: Some(&failure),
            work_budget: Some(&work),
            ..Default::default()
        };
        let paint = MeshPaint {
            function: Some(&function),
            color_space: "DeviceGray",
            color_space_obj: None,
            reader: &reader,
            options,
            patch_corners: None,
        };
        let color = paint.convert(MeshSample::new(&[0.5]).unwrap()).unwrap();
        assert!((color.r - 0.25).abs() < 1e-6);
        assert!(work.load(std::sync::atomic::Ordering::Relaxed) < 100);
        work.store(0, std::sync::atomic::Ordering::Relaxed);
        assert!(paint.convert(MeshSample::new(&[0.5]).unwrap()).is_none());
        assert!(failure.get().is_some());
    }

    #[test]
    fn shared_diagonal_has_single_sample_ownership_for_both_windings() {
        use crate::render::buffer::ClipMask;
        let reader = PdfReader::from_bytes(tests_minimal_pdf()).unwrap();
        let failure = Cell::new(None);
        let work = AtomicU64::new(MAX_SHADING_WORK_UNITS);
        let options = ShadingRenderOptions {
            failure: Some(&failure),
            work_budget: Some(&work),
            ..Default::default()
        };
        let paint = MeshPaint {
            function: None,
            color_space: "DeviceGray",
            color_space_obj: None,
            reader: &reader,
            options,
            patch_corners: None,
        };
        let vertex = |dx, dy| MeshVertex {
            dx,
            dy,
            sample: MeshSample::new(&[1.0]).unwrap(),
        };
        let [a, b, c, d] = [
            vertex(0.0, 0.0),
            vertex(10.0, 0.0),
            vertex(0.0, 10.0),
            vertex(10.0, 10.0),
        ];
        for reverse_first in [false, true] {
            for reverse_second in [false, true] {
                // Deliberate half coverage exposes double hits: they produce alpha
                // 192 rather than 128. The full entry point applies clip only later.
                let mut buf = PixelBuffer::new(10, 10);
                buf.set_clip(ClipMask::from_alpha_bytes(10, 10, vec![128; 100]));
                let first = if reverse_first { [a, c, b] } else { [a, b, c] };
                let second = if reverse_second { [b, c, d] } else { [b, d, c] };
                fill_triangle(&mut buf, first[0], first[1], first[2], &paint);
                fill_triangle(&mut buf, second[0], second[1], second[2], &paint);
                assert!(failure.get().is_none());
                assert!(buf
                    .rgba_bytes()
                    .chunks_exact(4)
                    .all(|pixel| pixel[3] == 128));
            }
        }
    }

    #[test]
    fn source_samples_and_patch_records_reject_malformed_shapes() {
        assert!(MeshSample::new(&[]).is_none());
        assert!(MeshSample::new(&[0.0; MAX_COMPONENTS + 1]).is_none());
        assert!(MeshSample::new(&[f64::NAN]).is_none());
        let gray = MeshSample::new(&[0.5]).unwrap();
        let rgb = MeshSample::new(&[0.5; 3]).unwrap();
        assert!(MeshSample::interpolate(&[(&gray, 0.5), (&rgb, 0.5)]).is_none());
        assert!(MeshSample::bilinear(&[gray; 3], 0.5, 0.5).is_none());
        assert!(assemble_patch(4, &[(0.0, 0.0); 12], &[gray; 4], &[], &[], 6).is_none());
        assert!(assemble_patch(0, &[(0.0, 0.0); 13], &[gray; 4], &[], &[], 6).is_none());
        assert!(assemble_patch(0, &[(0.0, 0.0); 12], &[gray; 5], &[], &[], 6).is_none());
        assert!(assemble_patch(0, &[(0.0, 0.0); 12], &[gray; 4], &[], &[], 8).is_none());
    }
}
