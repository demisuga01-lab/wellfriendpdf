//! Point/GID-preserving glyf serialization, with geometry derived from the exact
//! rounded points and component arguments being written. Not a complete font.
use super::{
    glyf_program::{
        self as program, fail, Arguments, Component, Kind, Program, MAX_DEPTH, MAX_POINTS,
    },
    glyph_metric_instance::{GlyphGeometry, Metric, PhantomDeltas},
    gvar_instance::PreparedGvar,
    tuple_variations::{Budget, Domain, TABLE_LIMIT},
    variation_store::{bytes, round_i32, u16_at},
};
use crate::{Result, WellfriendError};
use std::{collections::BTreeMap, sync::Arc};
type Tables = BTreeMap<[u8; 4], Arc<[u8]>>;
pub(crate) struct OutlineStage {
    pub tables: BTreeMap<[u8; 4], Vec<u8>>,
    pub geometry: Vec<GlyphGeometry>,
    pub point_counts: Vec<u16>,
    pub contour_counts: Vec<u16>,
    pub retained_instruction_glyphs: Vec<u16>,
    /// GID-ordered instruction ranges in the generated glyf table.
    pub instruction_ranges: Vec<(u16, std::ops::Range<usize>)>,
    pub use_my_metrics_glyphs: Vec<u16>,
    pub default_offset_components: Vec<(u16, u16)>,
    pub ignored_attachment_deltas: Vec<(u16, u16)>,
    pub repaired_default_bounds: Vec<u16>,
    pub tuple_count: usize,
    pub active_tuple_count: usize,
    pub work: usize,
}
impl OutlineStage {
    pub(crate) fn synchronize_sidebearing_flag(&mut self, metrics: &[Metric]) -> Result<()> {
        if metrics.len() != self.geometry.len() {
            return Err(fail("metric/outline identity mismatch"));
        }
        let mut same = true;
        for (index, (m, g)) in metrics.iter().zip(&self.geometry).enumerate() {
            if index % 256 == 0 {
                crate::cancel::check_current_cancel("outline sidebearing flags")?;
            }
            same &= g.instance.map_or(0, |r| r.x_min) == m.bearing;
        }
        let head = self
            .tables
            .get_mut(b"head")
            .ok_or_else(|| fail("missing staged head"))?;
        let flags = (u16_at(head, 16)? & !2) | if same { 2 } else { 0 };
        head[16..18].copy_from_slice(&flags.to_be_bytes());
        Ok(())
    }
}
struct Assembled {
    original: Vec<[f64; 2]>,
    selected: Vec<[f64; 2]>,
    contours: usize,
    depth: usize,
}
fn transform(c: &Component, point: [f64; 2]) -> [f64; 2] {
    let m = c.matrix.map(|n| f64::from(n) / 16384.);
    [
        m[0] * point[0] + m[2] * point[1],
        m[1] * point[0] + m[3] * point[1],
    ]
}
fn position(c: &Component, parent: &[[f64; 2]], child: &[[f64; 2]]) -> Result<[f64; 2]> {
    match c.arguments {
        Arguments::Offset(v) => {
            let offset = v.map(f64::from);
            Ok(if c.flags & 0x1800 == 0x800 {
                transform(c, offset)
            } else {
                offset
            })
        }
        Arguments::Points([parent_id, child_id]) => {
            // Phantom references require finalized phantom locations and hinting
            // semantics, not an assumed origin or an inserted contour point.
            let p = parent.get(usize::from(parent_id)).ok_or_else(|| fail("parent attachment is not an available explicit point; phantom/hinted attachment requires later resolution"))?;
            let q = child.get(usize::from(child_id)).ok_or_else(|| fail("child attachment is not an explicit point; phantom/hinted attachment requires later resolution"))?;
            let q = transform(c, *q);
            Ok([p[0] - q[0], p[1] - q[1]])
        }
    }
}
fn append(
    parent: &mut Vec<[f64; 2]>,
    child: &[[f64; 2]],
    c: &Component,
    budget: &mut Budget,
) -> Result<()> {
    budget.charge(child.len())?;
    if parent.len() + child.len() > usize::from(u16::MAX) {
        return Err(fail("expanded composite point count exceeds maxp"));
    }
    let offset = position(c, parent, child)?;
    for (i, point) in child.iter().enumerate() {
        if i % 256 == 0 {
            crate::cancel::check_current_cancel("glyf composite assembly")?;
        }
        let p = transform(c, *point);
        parent.push([p[0] + offset[0], p[1] + offset[1]]);
    }
    Ok(())
}
fn short(n: f64) -> Result<i16> {
    i16::try_from(round_i32(n)?).map_err(|_| fail("instanced coordinate/offset exceeds FWORD"))
}
fn same(a: Option<ttf_parser::Rect>, b: Option<ttf_parser::Rect>) -> bool {
    a.map(|r| [r.x_min, r.y_min, r.x_max, r.y_max])
        == b.map(|r| [r.x_min, r.y_min, r.x_max, r.y_max])
}
pub(crate) fn freeze(tables: &Tables, gvar: Option<&PreparedGvar>) -> Result<OutlineStage> {
    let get = |tag: &[u8; 4]| {
        tables
            .get(tag)
            .ok_or_else(|| fail("missing TrueType owner table"))
    };
    let mut budget = Budget::default();
    budget.charge(gvar.map_or(0, |g| g.directory_work))?;
    let parsed = Program::parse(
        Arc::clone(get(b"glyf")?),
        get(b"loca")?,
        get(b"head")?,
        get(b"maxp")?,
        &mut budget,
    )?;
    let order = parsed.order(&mut budget)?;
    let count = parsed.glyphs.len();
    let blank = GlyphGeometry {
        default: None,
        instance: None,
        phantom: None,
    };
    let mut stage = OutlineStage {
        tables: BTreeMap::new(),
        geometry: vec![blank; count],
        point_counts: vec![0; count],
        contour_counts: vec![0; count],
        retained_instruction_glyphs: Vec::new(),
        instruction_ranges: Vec::new(),
        use_my_metrics_glyphs: Vec::new(),
        default_offset_components: Vec::new(),
        ignored_attachment_deltas: Vec::new(),
        repaired_default_bounds: Vec::new(),
        tuple_count: 0,
        active_tuple_count: 0,
        work: 0,
    };
    let mut assembled = (0..count).map(|_| None).collect::<Vec<Option<Assembled>>>();
    let mut encoded = (0..count).map(|_| None).collect::<Vec<Option<Vec<u8>>>>();
    let mut instruction_ranges = vec![None; count];
    let mut expanded = 0usize;
    let mut output_size = 0usize;
    let mut max_simple_points = 0usize;
    let mut max_simple_contours = 0usize;
    let mut max_composite_points = 0usize;
    let mut max_composite_contours = 0usize;
    let mut max_components = 0usize;
    let mut max_depth = 0usize;
    let mut max_instructions = 0usize;
    for id in order {
        budget.charge(1)?;
        let glyph = &parsed.glyphs[id];
        let expected_points = match &glyph.kind {
            Kind::Empty => 0,
            Kind::Simple { points, .. } => points.len(),
            Kind::Composite(components) => {
                budget.charge(components.len())?;
                let mut total = 0usize;
                for (index, c) in components.iter().enumerate() {
                    if index % 256 == 0 {
                        crate::cancel::check_current_cancel("glyf expansion capacity")?;
                    }
                    let child = assembled[usize::from(c.glyph)]
                        .as_ref()
                        .ok_or_else(|| fail("component dependency not assembled"))?;
                    total = total
                        .checked_add(child.selected.len())
                        .ok_or_else(|| fail("expanded point count overflow"))?;
                    if total > usize::from(u16::MAX) {
                        return Err(fail("expanded composite point count exceeds maxp"));
                    }
                }
                total
            }
        };
        expanded += expected_points;
        if expanded > MAX_POINTS {
            return Err(WellfriendError::ResourceLimit(
                "expanded glyph point cache exceeds two million points".into(),
            ));
        }
        let domain = match &glyph.kind {
            Kind::Simple { points, ends, .. } => Domain::Simple {
                points,
                contour_ends: ends,
            },
            Kind::Composite(c) => Domain::Components(c.len()),
            Kind::Empty => Domain::Components(0),
        };
        let deltas = gvar
            .map(|g| g.resolve(id as u16, domain, &mut budget))
            .transpose()?;
        let mut phantom = None;
        if let Some(d) = &deltas {
            stage.tuple_count += d.tuples;
            stage.active_tuple_count += d.active_tuples;
            let n = d.values.len() - 4;
            phantom = Some(PhantomDeltas {
                left: d.values[n][0],
                right: d.values[n + 1][0],
                top: d.values[n + 2][1],
                bottom: d.values[n + 3][1],
            });
        }
        let delta = |i: usize| deltas.as_ref().map_or([0.; 2], |d| d.values[i]);
        let instructions = &parsed.source[glyph.instructions.clone()];
        max_instructions = max_instructions.max(instructions.len());
        if !instructions.is_empty() {
            stage.retained_instruction_glyphs.push(id as u16);
        }
        let (shape, output) = match &glyph.kind {
            Kind::Empty => (
                Assembled {
                    original: Vec::new(),
                    selected: Vec::new(),
                    contours: 0,
                    depth: 0,
                },
                Vec::new(),
            ),
            Kind::Simple {
                points,
                flags,
                ends,
            } => {
                budget.charge(points.len() * 2)?;
                let mut selected = Vec::with_capacity(points.len());
                for (i, point) in points.iter().enumerate() {
                    if i % 256 == 0 {
                        crate::cancel::check_current_cancel("glyf variation rounding")?;
                    }
                    let d = delta(i);
                    selected.push([
                        i32::from(short(f64::from(point[0]) + d[0])?),
                        i32::from(short(f64::from(point[1]) + d[1])?),
                    ]);
                }
                let selected_points = selected
                    .iter()
                    .map(|p| p.map(f64::from))
                    .collect::<Vec<_>>();
                let bounds = program::rectangle(&selected_points)?;
                let output = program::encode_simple(
                    &selected,
                    flags,
                    ends,
                    instructions,
                    bounds,
                    gvar.is_some(),
                    &mut budget,
                )?;
                max_simple_points = max_simple_points.max(points.len());
                max_simple_contours = max_simple_contours.max(ends.len());
                (
                    Assembled {
                        original: points.iter().map(|p| p.map(f64::from)).collect(),
                        selected: selected_points,
                        contours: ends.len(),
                        depth: 0,
                    },
                    output,
                )
            }
            Kind::Composite(components) => {
                let mut original = Vec::new();
                let mut selected = Vec::new();
                let mut contours = 0usize;
                let mut depth = 0usize;
                let mut emitted = Vec::with_capacity(components.len());
                if components.iter().any(|c| c.flags & 0x200 != 0) {
                    stage.use_my_metrics_glyphs.push(id as u16);
                }
                for (index, component) in components.iter().enumerate() {
                    budget.charge(1)?;
                    let child = assembled[usize::from(component.glyph)]
                        .as_ref()
                        .ok_or_else(|| fail("component dependency not assembled"))?;
                    contours += child.contours;
                    depth = depth.max(child.depth + 1);
                    if contours > usize::from(u16::MAX) {
                        return Err(fail("composite contour count exceeds maxp"));
                    }
                    if depth > MAX_DEPTH {
                        return Err(WellfriendError::ResourceLimit(
                            "component depth exceeds 64".into(),
                        ));
                    }
                    let mut c = component.clone();
                    let d = delta(index);
                    match c.arguments {
                        Arguments::Offset(v) => {
                            c.arguments = Arguments::Offset([
                                short(f64::from(v[0]) + d[0])?,
                                short(f64::from(v[1]) + d[1])?,
                            ]);
                            if c.flags & 0x1800 == 0 || c.flags & 0x1800 == 0x1800 {
                                // Make the OpenType-recommended unscaled default
                                // explicit instead of preserving platform ambiguity.
                                c.flags = (c.flags & !0x1800) | 0x1000;
                                stage
                                    .default_offset_components
                                    .push((id as u16, index as u16));
                            }
                        }
                        Arguments::Points(_) => {
                            if d != [0.; 2] {
                                stage
                                    .ignored_attachment_deltas
                                    .push((id as u16, index as u16));
                            }
                        }
                    }
                    append(&mut original, &child.original, component, &mut budget)?;
                    append(&mut selected, &child.selected, &c, &mut budget)?;
                    emitted.push(c);
                }
                let bounds = program::rectangle(&selected)?;
                let output = program::encode_composite(
                    &emitted,
                    instructions,
                    bounds,
                    gvar.is_some(),
                    &mut budget,
                )?;
                max_composite_points = max_composite_points.max(selected.len());
                max_composite_contours = max_composite_contours.max(contours);
                max_components = max_components.max(components.len());
                max_depth = max_depth.max(depth);
                (
                    Assembled {
                        original,
                        selected,
                        contours,
                        depth,
                    },
                    output,
                )
            }
        };
        let original_bounds = program::rectangle(&shape.original)?;
        let instance = program::rectangle(&shape.selected)?;
        // Base phantom coordinates are defined using the source header, not a
        // repaired header. Retain that basis when deriving changed bearings.
        let default = if shape.original.is_empty() {
            None
        } else {
            glyph.bounds
        };
        if !same(default, original_bounds) {
            stage.repaired_default_bounds.push(id as u16);
        }
        stage.geometry[id] = GlyphGeometry {
            default,
            instance,
            phantom,
        };
        stage.point_counts[id] = shape.selected.len() as u16;
        stage.contour_counts[id] = shape.contours as u16;
        if !instructions.is_empty() {
            let start = match &glyph.kind {
                Kind::Simple { ends, .. } => 12 + ends.len() * 2,
                Kind::Composite(_) => output.len() - instructions.len(),
                Kind::Empty => return Err(fail("empty glyph cannot carry instructions")),
            };
            instruction_ranges[id] = Some(start..start + instructions.len());
        }
        output_size += (output.len() + 1) & !1;
        if output_size > TABLE_LIMIT {
            return Err(WellfriendError::ResourceLimit(
                "generated glyf exceeds 64 MiB".into(),
            ));
        }
        assembled[id] = Some(shape);
        encoded[id] = Some(output);
    }
    // Keep GID order independently of the dependency traversal order.
    let mut glyf = Vec::with_capacity(output_size);
    let mut offsets = Vec::with_capacity(count + 1);
    for (id, output) in encoded.into_iter().enumerate() {
        budget.charge(1)?;
        offsets.push(glyf.len());
        if let Some(range) = instruction_ranges[id].take() {
            stage
                .instruction_ranges
                .push((id as u16, glyf.len() + range.start..glyf.len() + range.end));
        }
        for chunk in output
            .ok_or_else(|| fail("missing encoded glyph"))?
            .chunks(65536)
        {
            crate::cancel::check_current_cancel("glyf table assembly")?;
            glyf.extend(chunk);
        }
        if glyf.len() % 2 != 0 {
            glyf.push(0);
        }
    }
    offsets.push(glyf.len());
    let short_loca = glyf.len() / 2 <= usize::from(u16::MAX);
    let mut loca = Vec::with_capacity((count + 1) * if short_loca { 2 } else { 4 });
    for (index, offset) in offsets.into_iter().enumerate() {
        if index % 256 == 0 {
            crate::cancel::check_current_cancel("glyf location serialization")?;
        }
        if short_loca {
            loca.extend(((offset / 2) as u16).to_be_bytes());
        } else {
            loca.extend((offset as u32).to_be_bytes());
        }
    }
    let mut head = bytes(get(b"head")?, 0, 54)?.to_vec();
    head[8..12].fill(0);
    head[50..52].copy_from_slice(&u16::from(!short_loca).to_be_bytes());
    let mut bounds = None::<ttf_parser::Rect>;
    for (index, g) in stage.geometry.iter().enumerate() {
        if index % 256 == 0 {
            crate::cancel::check_current_cancel("global glyph bounds")?;
        }
        if let Some(b) = g.instance {
            bounds = Some(bounds.map_or(b, |a| ttf_parser::Rect {
                x_min: a.x_min.min(b.x_min),
                y_min: a.y_min.min(b.y_min),
                x_max: a.x_max.max(b.x_max),
                y_max: a.y_max.max(b.y_max),
            }));
        }
    }
    let mut bbox = Vec::new();
    program::header(&mut bbox, 0, bounds);
    head[36..44].copy_from_slice(&bbox[2..10]);
    let mut maxp = bytes(get(b"maxp")?, 0, 32)?.to_vec();
    for (at, value) in [
        (6, max_simple_points),
        (8, max_simple_contours),
        (10, max_composite_points),
        (12, max_composite_contours),
        (26, max_instructions),
        (28, max_components),
        (30, max_depth),
    ] {
        maxp[at..at + 2].copy_from_slice(
            &u16::try_from(value)
                .map_err(|_| fail("maxp field overflow"))?
                .to_be_bytes(),
        );
    }
    // Hint-program resource maxima are not guessed from byte values; retain them
    // until the instruction stage has proved any replacement requirements.
    stage.tables.insert(*b"glyf", glyf);
    stage.tables.insert(*b"loca", loca);
    stage.tables.insert(*b"head", head);
    stage.tables.insert(*b"maxp", maxp);
    budget.charge(0)?;
    stage.work = budget.work;
    Ok(stage)
}

#[cfg(test)]
#[path = "glyf_instance_tests.rs"]
mod tests;
