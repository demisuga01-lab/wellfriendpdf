//! Explicit, curve-preserving nonzero contour normalization. This consumes only
//! our expanded CFF1 operator subset, never a second arbitrary-font parser.
use super::super::{charstring, fail, number};
use crate::{Result, WellfriendError};
use kurbo::{BezPath, PathEl, PathSeg, Point, Rect, Shape, Vec2};
use linesweeper::topology::Topology;
use serde::{Deserialize, Serialize};

const QUANTUM: f64 = 1. / 65536.;
const MAX_INPUT: usize = 2048;
const MAX_OUTPUT: usize = 4096;
const MAX_TOTAL: usize = 1_000_000;

#[path = "cff2_linear_contours.rs"]
mod linear;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContourNormalization {
    /// Numerical tolerance in unscaled font design units, not PDF points.
    pub tolerance_font_units: f64,
    /// Rewritten paths cannot retain their original hint program. Compatible
    /// source boundaries keep exact glyph/private hints without this decision.
    #[serde(default)]
    pub allow_hint_loss: bool,
}
impl ContourNormalization {
    pub(crate) fn validate(&self) -> Result<()> {
        if !self.tolerance_font_units.is_finite()
            || !(QUANTUM..=0.125).contains(&self.tolerance_font_units)
        {
            return Err(fail(
                "contour tolerance must be finite and in 1/65536..=0.125 font units",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContourReport {
    pub algorithm: String,
    pub tolerance_font_units: f64,
    pub solver_epsilon: f64,
    pub coordinate_quantum: f64,
    pub normalized_glyphs: Vec<u16>,
    #[serde(default)]
    pub preserved_glyphs: Vec<u16>,
    #[serde(default)]
    pub preserved_hint_glyphs: Vec<u16>,
    /// Straight-edge compatibility used bounded, exact 16.16 integer predicates.
    /// This is a decision method, not independent output/raster qualification.
    #[serde(default)]
    pub exact_linear_preserved_glyphs: Vec<u16>,
    /// Candidate edge-pair and containment-edge visits in the exact classifier.
    #[serde(default)]
    pub exact_linear_work: usize,
    pub dehinted_glyphs: Vec<u16>,
    /// Input path commands existed but normalized output contains no boundary.
    /// This can be legitimate cancellation/degeneracy; it is not hidden.
    pub emptied_glyphs: Vec<u16>,
    pub private_hint_dictionaries_removed: usize,
    /// A dictionary can be retained for one glyph and bypassed for another.
    #[serde(default)]
    pub private_hint_dictionaries_bypassed: usize,
    #[serde(default)]
    pub private_hint_dictionaries_retained: usize,
    pub input_segments: usize,
    pub output_segments: usize,
    pub symmetric_difference_area: f64,
    pub broadphase_pairs: usize,
    pub potential_intersections: usize,
    /// Checks use the same numerical topology library, not an independent oracle.
    pub independently_verified: bool,
}
impl ContourReport {
    pub(super) fn preserving_check() -> Self {
        Self::new(&ContourNormalization {
            tolerance_font_units: QUANTUM,
            allow_hint_loss: false,
        })
    }
    pub(super) fn new(options: &ContourNormalization) -> Self {
        Self {
            algorithm: "fixed16-i128/linear;linesweeper-0.4.0/nonzero/contours_correct".into(),
            tolerance_font_units: options.tolerance_font_units,
            solver_epsilon: options.tolerance_font_units / 8.,
            coordinate_quantum: QUANTUM,
            normalized_glyphs: Vec::new(),
            preserved_glyphs: Vec::new(),
            preserved_hint_glyphs: Vec::new(),
            exact_linear_preserved_glyphs: Vec::new(),
            exact_linear_work: 0,
            dehinted_glyphs: Vec::new(),
            emptied_glyphs: Vec::new(),
            private_hint_dictionaries_removed: 0,
            private_hint_dictionaries_bypassed: 0,
            private_hint_dictionaries_retained: 0,
            input_segments: 0,
            output_segments: 0,
            symmetric_difference_area: 0.,
            broadphase_pairs: 0,
            potential_intersections: 0,
            independently_verified: false,
        }
    }
}
fn budget(message: &str) -> WellfriendError {
    WellfriendError::ResourceLimit(format!("CFF contour processing: {message}"))
}
fn valid_point(p: Point) -> Result<()> {
    if !p.x.is_finite() || !p.y.is_finite() || p.x.abs().max(p.y.abs()) > 1_048_576. {
        return Err(fail("contour point outside finite numerical domain"));
    }
    Ok(())
}
struct Capture {
    path: BezPath,
    pen: Point,
    active: bool,
    segments: usize,
}
impl Capture {
    fn segment(&mut self) -> Result<()> {
        self.segments += 1;
        if self.segments > MAX_OUTPUT {
            return Err(budget("decoded segment count"));
        }
        if self.segments.is_multiple_of(64) {
            crate::cancel::check_current_cancel("CFF contour decoding")?;
        }
        Ok(())
    }
    fn move_by(&mut self, x: f64, y: f64) -> Result<()> {
        if self.active {
            self.path.close_path();
        }
        self.pen += Vec2::new(x, y);
        valid_point(self.pen)?;
        self.path.move_to(self.pen);
        self.active = true;
        self.segment()
    }
    fn line_by(&mut self, x: f64, y: f64) -> Result<()> {
        if !self.active {
            return Err(fail("normalized line before move"));
        }
        self.pen += Vec2::new(x, y);
        valid_point(self.pen)?;
        self.path.line_to(self.pen);
        self.segment()
    }
    fn curve_by(&mut self, a: &[f64]) -> Result<()> {
        if !self.active || a.len() != 6 {
            return Err(fail("normalized curve arity/state"));
        }
        let p1 = self.pen + Vec2::new(a[0], a[1]);
        let p2 = p1 + Vec2::new(a[2], a[3]);
        let p3 = p2 + Vec2::new(a[4], a[5]);
        for p in [p1, p2, p3] {
            valid_point(p)?;
        }
        self.path.curve_to(p1, p2, p3);
        self.pen = p3;
        self.segment()
    }
}
pub(super) fn decode(code: &[u8]) -> Result<BezPath> {
    let mut c = Capture {
        path: BezPath::new(),
        pen: Point::ZERO,
        active: false,
        segments: 0,
    };
    let mut at = 0;
    let mut stack = Vec::new();
    let mut first = true;
    let mut stems = 0usize;
    while at < code.len() {
        let op = code[at];
        at += 1;
        if let Some(n) = number(code, &mut at, op, false)? {
            stack.push(n);
            if stack.len() > 48 {
                return Err(fail("normalized operand count"));
            }
            continue;
        }
        if first {
            if stack.is_empty() {
                return Err(fail("normalized glyph lacks explicit width"));
            }
            stack.remove(0);
            first = false;
        }
        match op {
            1 | 3 | 18 | 23 => {
                if stack.len() % 2 != 0 {
                    return Err(fail("normalized stem arity"));
                }
                stems += stack.len() / 2;
            }
            19 | 20 => {
                if !stack.is_empty() || stems > 96 {
                    return Err(fail("normalized mask arity"));
                }
                at = at
                    .checked_add(stems.div_ceil(8))
                    .filter(|end| *end <= code.len())
                    .ok_or_else(|| fail("normalized mask bounds"))?;
            }
            4 if stack.len() == 1 => c.move_by(0., stack[0])?,
            22 if stack.len() == 1 => c.move_by(stack[0], 0.)?,
            21 if stack.len() == 2 => c.move_by(stack[0], stack[1])?,
            5 if !stack.is_empty() && stack.len() % 2 == 0 => {
                for p in stack.chunks_exact(2) {
                    c.line_by(p[0], p[1])?;
                }
            }
            8 if !stack.is_empty() && stack.len() % 6 == 0 => {
                for p in stack.chunks_exact(6) {
                    c.curve_by(p)?;
                }
            }
            12 => {
                let escaped = *code.get(at).ok_or_else(|| fail("normalized escape"))?;
                at += 1;
                let a = &stack;
                let curves = match (escaped, a.len()) {
                    (34, 7) => [
                        a[0], 0., a[1], a[2], a[3], 0., a[4], 0., a[5], -a[2], a[6], 0.,
                    ],
                    (35, 13) => a[..12].try_into().unwrap(),
                    (36, 9) => [
                        a[0],
                        a[1],
                        a[2],
                        a[3],
                        a[4],
                        0.,
                        a[5],
                        0.,
                        a[6],
                        a[7],
                        a[8],
                        -(a[1] + a[3] + a[7]),
                    ],
                    (37, 11) => {
                        let dx = a[0] + a[2] + a[4] + a[6] + a[8];
                        let dy = a[1] + a[3] + a[5] + a[7] + a[9];
                        let (x, y) = if dx.abs() > dy.abs() {
                            (a[10], -dy)
                        } else {
                            (-dx, a[10])
                        };
                        [
                            a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9], x, y,
                        ]
                    }
                    _ => return Err(fail("normalized flex arity")),
                };
                c.curve_by(&curves[..6])?;
                c.curve_by(&curves[6..])?;
            }
            14 if stack.is_empty() && at == code.len() => {
                if c.active {
                    c.path.close_path();
                }
                return Ok(c.path);
            }
            _ => return Err(fail("operator outside normalized CFF subset")),
        }
        stack.clear();
    }
    Err(fail("normalized glyph lacks endchar"))
}
fn quantize(p: Point) -> Result<Point> {
    valid_point(p)?;
    Ok(Point::new(
        (p.x / QUANTUM).round() * QUANTUM,
        (p.y / QUANTUM).round() * QUANTUM,
    ))
}
pub(super) fn encode(path: &BezPath, width_delta: f64) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    charstring::encode_number(&mut out, width_delta)?;
    let mut pen = Point::ZERO;
    for (i, element) in path.iter().enumerate() {
        if i % 64 == 0 {
            crate::cancel::check_current_cancel("CFF normalized path emission")?;
        }
        let (points, op): (Vec<Point>, u8) = match element {
            PathEl::MoveTo(p) => (vec![quantize(p)?], 21),
            PathEl::LineTo(p) => (vec![quantize(p)?], 5),
            PathEl::CurveTo(a, b, c) => (vec![quantize(a)?, quantize(b)?, quantize(c)?], 8),
            PathEl::QuadTo(a, b) => {
                // Elevation is exact before final 16.16 point quantization.
                let p1 = pen + (a - pen) * (2. / 3.);
                let p2 = b + (a - b) * (2. / 3.);
                (vec![quantize(p1)?, quantize(p2)?, quantize(b)?], 8)
            }
            PathEl::ClosePath => continue, // Type 2 closes on the next move/endchar; pen stays at the last point.
        };
        for point in points {
            let delta = point - pen;
            charstring::encode_number(&mut out, delta.x)?;
            charstring::encode_number(&mut out, delta.y)?;
            pen = point;
        }
        out.push(op);
        if out.len() >= 65535 {
            return Err(budget("normalized glyph bytes"));
        }
    }
    out.push(14);
    Ok(out)
}
fn finite_area(path: &BezPath) -> Result<f64> {
    let area = path.area();
    if !area.is_finite() {
        return Err(fail("non-finite contour area"));
    }
    Ok(area)
}
fn preflight(paths: &[&BezPath], eps: f64, report: &mut ContourReport) -> Result<()> {
    // A control hull contains its curve. Bound candidate pairs before entering
    // the synchronous solver; Bezout degree products bound exact crossings.
    // This is a complexity guard, not a bound on every upstream allocation.
    let mut hulls = Vec::<(Rect, usize)>::new();
    for path in paths {
        for segment in path.segments() {
            let (points, degree): (Vec<Point>, usize) = match segment {
                PathSeg::Line(s) => (vec![s.p0, s.p1], 1),
                PathSeg::Quad(s) => (vec![s.p0, s.p1, s.p2], 2),
                PathSeg::Cubic(s) => (vec![s.p0, s.p1, s.p2, s.p3], 3),
            };
            let mut rect = Rect::from_points(points[0], points[0]);
            for point in points {
                valid_point(point)?;
                rect = rect.union_pt(point);
            }
            hulls.push((rect.inflate(eps * 8., eps * 8.), degree));
        }
    }
    if hulls.len() > MAX_INPUT + MAX_OUTPUT {
        return Err(budget("topology source segment capacity"));
    }
    hulls.sort_by(|a, b| a.0.x0.total_cmp(&b.0.x0));
    let mut potential = 0usize;
    for (i, (a, degree)) in hulls.iter().enumerate() {
        crate::cancel::check_current_cancel("CFF contour broadphase")?;
        for (b, other) in &hulls[i + 1..] {
            if b.x0 > a.x1 {
                break;
            }
            report.broadphase_pairs += 1;
            if report.broadphase_pairs > 16_000_000 {
                return Err(budget("aggregate contour candidate pairs"));
            }
            if b.y0 <= a.y1 && a.y0 <= b.y1 {
                potential += degree * other;
                if potential > 250_000 {
                    return Err(budget("glyph topology intersection complexity"));
                }
            }
        }
    }
    report.potential_intersections = report
        .potential_intersections
        .checked_add(potential)
        .ok_or_else(|| budget("contour complexity overflow"))?;
    Ok(())
}
pub(super) struct GlyphOutcome {
    pub bytes: Vec<u8>,
    pub rewritten: bool,
}
fn preserve(
    code: &[u8],
    gid: u16,
    input: usize,
    has_hint_owners: bool,
    report: &mut ContourReport,
) -> Result<GlyphOutcome> {
    record_preserved(gid, input, has_hint_owners, report)?;
    Ok(GlyphOutcome {
        bytes: code.to_vec(),
        rewritten: false,
    })
}
fn record_preserved(
    gid: u16,
    input: usize,
    has_hint_owners: bool,
    report: &mut ContourReport,
) -> Result<()> {
    count_output(report, input)?;
    report.preserved_glyphs.push(gid);
    if has_hint_owners {
        report.preserved_hint_glyphs.push(gid);
    }
    Ok(())
}
fn require_hint_consent(
    gid: u16,
    has_hint_owners: bool,
    options: &ContourNormalization,
) -> Result<()> {
    if has_hint_owners && !options.allow_hint_loss {
        return Err(fail(&format!(
            "glyph {gid} requires contour rewriting and explicit hint-loss approval"
        )));
    }
    Ok(())
}
fn compatible_boundary(topology: &Topology<i32>) -> Result<bool> {
    // Inspect the surviving numerical boundary. `orig_seg` also contains
    // deleted/coincident sweep records; treating those as live edges rejects
    // ordinary cubic contours after the solver subdivides or merges them.
    // An empty surviving boundary still fails closed so equal-and-opposite
    // source contours cannot masquerade as an unchanged glyph.
    let segments = topology.segment_indices().collect::<Vec<_>>();
    if segments.is_empty() {
        return Ok(false);
    }
    let mut degree = std::collections::BTreeMap::<linesweeper::Point, usize>::new();
    for (i, index) in segments.into_iter().enumerate() {
        if i % 256 == 0 {
            crate::cancel::check_current_cancel("CFF source boundary compatibility")?;
        }
        let a = index.first_half();
        let b = a.other_half();
        let clockwise = *topology.winding_clockwise(a);
        let counter = *topology.winding_counter_clockwise(a);
        if !matches!((clockwise, counter), (0, 1) | (1, 0) | (0, -1) | (-1, 0)) {
            return Ok(false);
        }
        for point in [topology.point(a), topology.point(b)] {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(fail("non-finite topology vertex"));
            }
            let count = degree.entry(*point).or_default();
            *count += 1;
            if *count > 2 {
                return Ok(false);
            }
        }
    }
    // Each component is a simple cycle in the numerical topology. Degree-four
    // crossing/touch vertices require reconstruction even with unit winding.
    Ok(degree.values().all(|n| *n == 2))
}
fn count_output(report: &mut ContourReport, segments: usize) -> Result<()> {
    report.output_segments = report
        .output_segments
        .checked_add(segments)
        .ok_or_else(|| budget("aggregate output overflow"))?;
    if report.output_segments > MAX_TOTAL {
        return Err(budget("aggregate output segments"));
    }
    Ok(())
}
fn checked_source(code: &[u8], report: &mut ContourReport) -> Result<(BezPath, usize)> {
    crate::cancel::check_current_cancel("CFF contour inspection")?;
    let source = decode(code)?;
    let input = source.segments().count();
    if input > MAX_INPUT {
        return Err(budget("source glyph has more than 2048 segments"));
    }
    report.input_segments = report
        .input_segments
        .checked_add(input)
        .ok_or_else(|| budget("aggregate input overflow"))?;
    if report.input_segments > MAX_TOTAL {
        return Err(budget("aggregate input segments"));
    }
    Ok((source, input))
}
pub(super) fn verify_preserved(
    code: &[u8],
    gid: u16,
    report: &mut ContourReport,
    has_hint_owners: bool,
) -> Result<()> {
    let (source, input) = checked_source(code, report)?;
    if input == 0 {
        return record_preserved(gid, input, has_hint_owners, report);
    }
    let exact = linear::compatible(&source, report)?;
    let compatible = if let Some(answer) = exact {
        answer
    } else {
        preflight(&[&source], report.solver_epsilon, report)?;
        let topology = Topology::from_path(&source, report.solver_epsilon)
            .map_err(|_| fail("preserved contour is not closed"))?;
        crate::cancel::check_current_cancel("CFF preserved contour topology")?;
        compatible_boundary(&topology)?
    };
    if !compatible {
        return Err(fail(&format!("glyph {gid} cannot be preserved as checked CFF1; enable cff2_contours normalization and approve hint loss if needed")));
    }
    record_preserved(gid, input, has_hint_owners, report)?;
    if exact == Some(true) {
        report.exact_linear_preserved_glyphs.push(gid);
    }
    Ok(())
}
pub(super) fn normalize_if_needed(
    code: &[u8],
    width_delta: f64,
    gid: u16,
    options: &ContourNormalization,
    report: &mut ContourReport,
    has_hint_owners: bool,
) -> Result<GlyphOutcome> {
    options.validate()?;
    let (source, input) = checked_source(code, report)?;
    if input == 0 {
        return preserve(code, gid, input, has_hint_owners, report);
    }
    let exact_linear = linear::compatible(&source, report)?;
    if exact_linear == Some(true) {
        let outcome = preserve(code, gid, input, has_hint_owners, report)?;
        report.exact_linear_preserved_glyphs.push(gid);
        return Ok(outcome);
    }
    if exact_linear == Some(false) {
        // Do not let a tolerant topology override an exact detected contact,
        // overlap or redundant ink region. Consent can fail before the solver.
        require_hint_consent(gid, has_hint_owners, options)?;
    }
    let eps = report.solver_epsilon;
    preflight(&[&source], eps, report)?;
    // The upstream solver is synchronous. Our cancellation boundaries surround
    // each bounded glyph solve; this is not instruction-level interruption.
    let topology =
        Topology::from_path(&source, eps).map_err(|_| fail("source contour is not closed"))?;
    crate::cancel::check_current_cancel("CFF contour topology")?;
    if exact_linear.is_none() && compatible_boundary(&topology)? {
        return preserve(code, gid, input, has_hint_owners, report);
    }
    require_hint_consent(gid, has_hint_owners, options)?;
    let contours = topology.contours_correct(|w| *w != 0);
    crate::cancel::check_current_cancel("CFF contour boundary extraction")?;
    let mut output = BezPath::new();
    for contour in contours.contours() {
        if output.elements().len() + contour.path.elements().len() > MAX_OUTPUT {
            return Err(budget("normalized path elements"));
        }
        output.extend(contour.path.iter());
    }
    let bytes = encode(&output, width_delta)?;
    let saved = decode(&bytes)?;
    preflight(&[&saved], eps, report)?;
    // Recheck the actual rounded program, not only the pre-serialization paths.
    let actual =
        Topology::from_path(&saved, eps).map_err(|_| fail("saved contour is not closed"))?;
    if !actual.has_normal_contours(()) {
        return Err(fail("16.16 serialization reintroduced non-normal contours"));
    }
    crate::cancel::check_current_cancel("CFF saved contour topology")?;
    preflight(&[&source, &saved], eps, report)?;
    let compare = Topology::from_paths_binary(&source, &saved, eps)
        .map_err(|_| fail("contour comparison is not closed"))?;
    let difference = compare.contours_correct(|w| (w.shape_a != 0) != (w.shape_b != 0));
    let area = difference.contours().try_fold(0., |sum, c| {
        Ok::<_, WellfriendError>(sum + finite_area(&c.path)?.abs())
    })?;
    // This is a numerical area guard, not a certified Hausdorff/pixel bound.
    let allowance = 4. * options.tolerance_font_units * (source.perimeter(eps) + 1.);
    if !area.is_finite() || !allowance.is_finite() || area > allowance {
        return Err(fail(
            "normalized filled-area difference exceeds numerical allowance",
        ));
    }
    crate::cancel::check_current_cancel("CFF contour comparison")?;
    count_output(report, saved.segments().count())?;
    report.normalized_glyphs.push(gid);
    if saved.segments().next().is_none() {
        report.emptied_glyphs.push(gid);
    }
    report.symmetric_difference_area += area;
    if has_hint_owners {
        report.dehinted_glyphs.push(gid);
    }
    Ok(GlyphOutcome {
        bytes,
        rewritten: true,
    })
}

#[cfg(test)]
pub(super) fn normalize(
    code: &[u8],
    width_delta: f64,
    gid: u16,
    options: &ContourNormalization,
    report: &mut ContourReport,
) -> Result<Vec<u8>> {
    Ok(normalize_if_needed(code, width_delta, gid, options, report, false)?.bytes)
}

#[cfg(test)]
#[path = "cff2_contours_tests.rs"]
pub(super) mod tests;
