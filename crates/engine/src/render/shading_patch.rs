//! Device-adaptive bicubic grids with negotiated, bit-identical shared edges.
//!
//! The source mesh is bounded and planned before rasterization. Each patch has
//! two axis factors; matching cubic edges union the corresponding axes. Raising
//! factors to their component maximum closes T-junctions without forcing every
//! axis of every disconnected patch to the same resolution.
use super::*;
use crate::decode_scheduler::DecodeMemoryToken;

const MAX_AXIS_STEPS: usize = 1024;
const GEOMETRY_TOLERANCE_PX: f64 = 0.25;
type Net = [[PatchPoint; 4]; 4];
type EdgeKey = [u64; 8];

#[derive(Clone, Copy, Debug)]
struct Boundary {
    key: EdgeKey,
    controls: [PatchPoint; 4],
    reversed: bool,
    symmetric: bool,
}

impl Boundary {
    fn new(points: [PatchPoint; 4], transform: &Transform2D) -> Result<Self, &'static str> {
        let encode = |points: [PatchPoint; 4]| {
            let mut key = [0; 8];
            for (i, (x, y)) in points.into_iter().enumerate() {
                // PDF numbers do not distinguish signed zero geometrically.
                key[2 * i] = if x == 0.0 {
                    0.0f64.to_bits()
                } else {
                    x.to_bits()
                };
                key[2 * i + 1] = if y == 0.0 {
                    0.0f64.to_bits()
                } else {
                    y.to_bits()
                };
            }
            key
        };
        let forward = encode(points);
        let backward = encode([points[3], points[2], points[1], points[0]]);
        let reversed = backward < forward;
        let key = if reversed { backward } else { forward };
        let controls = std::array::from_fn(|i| {
            transform.transform_point(f64::from_bits(key[2 * i]), f64::from_bits(key[2 * i + 1]))
        });
        if controls.iter().any(|p| !finite(*p)) {
            return Err("nonfinite patch boundary transform");
        }
        Ok(Self {
            key,
            controls,
            reversed,
            symmetric: forward == backward,
        })
    }

    fn point(&self, step: usize, count: usize) -> PatchPoint {
        let mut index = if self.reversed { count - step } else { step };
        // A palindromic control polygon has no distinguishable orientation.
        // Fold its equivalent parameters to one evaluation path as well.
        if self.symmetric {
            index = index.min(count - index);
        }
        // Integer-index reversal avoids 1 - rounded(t). Dyadic grids retain
        // exact parameters; endpoints are not re-evaluated by a polynomial.
        if index == 0 {
            return self.controls[0];
        }
        if index == count {
            return self.controls[3];
        }
        if self.controls.iter().all(|p| *p == self.controls[0]) {
            return self.controls[0];
        }
        curve(self.controls, index as f64 / count as f64)
    }
}

#[derive(Clone, Debug)]
struct Patch {
    net: Net,
    edges: [Boundary; 4],
    corners: [MeshSample; 4],
    steps: usize,
    outside: bool,
}

fn finite(p: PatchPoint) -> bool {
    p.0.is_finite() && p.1.is_finite()
}
fn lerp(a: PatchPoint, b: PatchPoint, t: f64) -> PatchPoint {
    ((1.0 - t) * a.0 + t * b.0, (1.0 - t) * a.1 + t * b.1)
}
fn curve(p: [PatchPoint; 4], t: f64) -> PatchPoint {
    let a = lerp(p[0], p[1], t);
    let b = lerp(p[1], p[2], t);
    let c = lerp(p[2], p[3], t);
    lerp(lerp(a, b, t), lerp(b, c, t), t)
}
fn difference(a: PatchPoint, b: PatchPoint) -> PatchPoint {
    (a.0 - b.0, a.1 - b.1)
}
fn magnitude(p: PatchPoint) -> f64 {
    p.0.hypot(p.1)
}

/// Coons ruled surfaces minus their bilinear corner surface, degree-elevated
/// into a bicubic Bernstein control net. Boundary controls remain verbatim.
fn control_net(points: &[PatchPoint], kind: i64) -> Result<Net, &'static str> {
    let required = match kind {
        6 => 12,
        7 => 16,
        _ => return Err("invalid patch type"),
    };
    if points.len() != required || points.iter().any(|p| !finite(*p)) {
        return Err("invalid patch control points");
    }
    let p = points;
    let mut net = [[(0.0, 0.0); 4]; 4];
    net[0] = [p[0], p[1], p[2], p[3]];
    net[3] = [p[9], p[8], p[7], p[6]];
    net[1][0] = p[11];
    net[2][0] = p[10];
    net[1][3] = p[4];
    net[2][3] = p[5];
    if kind == 7 {
        net[1][1] = p[12];
        net[1][2] = p[13];
        net[2][2] = p[14];
        net[2][1] = p[15];
    } else {
        for i in 1..3 {
            for j in 1..3 {
                let u = i as f64 / 3.0;
                let v = j as f64 / 3.0;
                let across_u = lerp(net[0][j], net[3][j], u);
                let across_v = lerp(net[i][0], net[i][3], v);
                let bilinear = lerp(lerp(p[0], p[3], v), lerp(p[9], p[6], v), u);
                net[i][j] = (
                    across_v.0 + (across_u.0 - bilinear.0),
                    across_v.1 + (across_u.1 - bilinear.1),
                );
            }
        }
    }
    if net.iter().flatten().any(|p| !finite(*p)) {
        return Err("patch control-net arithmetic overflow");
    }
    Ok(net)
}

/// In exact arithmetic, Bernstein convex-hull bounds give Muu, Mvv and Muv
/// from second control differences. Bilinear grid error is at most
/// (Muu+Mvv)/(8*n*n); its split into two affine triangles adds Muv/(4*n*n).
/// This is a conservative geometric estimate, not a colour-error or floating-
/// point interval certificate. Nonfinite estimates fail rather than under-refine.
fn required_steps(net: &Net) -> Result<usize, &'static str> {
    let mut uu = 0.0f64;
    let mut vv = 0.0f64;
    let mut uv = 0.0f64;
    for i in 0..2 {
        for j in 0..4 {
            let duu = magnitude(difference(
                difference(net[i + 2][j], net[i + 1][j]),
                difference(net[i + 1][j], net[i][j]),
            ));
            let dvv = magnitude(difference(
                difference(net[j][i + 2], net[j][i + 1]),
                difference(net[j][i + 1], net[j][i]),
            ));
            if !duu.is_finite() || !dvv.is_finite() {
                return Err("nonfinite patch second difference");
            }
            uu = uu.max(duu);
            vv = vv.max(dvv);
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            let duv = magnitude(difference(
                difference(net[i + 1][j + 1], net[i + 1][j]),
                difference(net[i][j + 1], net[i][j]),
            ));
            if !duv.is_finite() {
                return Err("nonfinite patch mixed difference");
            }
            uv = uv.max(duv);
        }
    }
    let bound = (6.0 * uu + 6.0 * vv) / 8.0 + 9.0 * uv / 4.0;
    if !bound.is_finite() {
        return Err("patch geometric-error estimate overflow");
    }
    let mut count = 1usize;
    while bound / (count * count) as f64 > GEOMETRY_TOLERANCE_PX {
        if count >= MAX_AXIS_STEPS {
            return Err("patch exceeds geometric refinement budget");
        }
        count *= 2;
    }
    Ok(count)
}

#[cfg(test)]
#[path = "shading_patch_tests.rs"]
mod tests;

impl Patch {
    #[cfg(test)]
    fn new(
        points: &[PatchPoint],
        corners: &[MeshSample],
        kind: i64,
        transform: &Transform2D,
    ) -> Result<Self, &'static str> {
        Self::new_clipped(points, corners, kind, transform, None)
    }

    fn new_clipped(
        points: &[PatchPoint],
        corners: &[MeshSample],
        kind: i64,
        transform: &Transform2D,
        bounds: Option<(i32, i32, i32, i32)>,
    ) -> Result<Self, &'static str> {
        let mut net = control_net(points, kind)?;
        let corners = corners
            .try_into()
            .map_err(|_| "invalid patch corner count")?;
        for row in &mut net {
            for point in row {
                *point = transform.transform_point(point.0, point.1);
            }
        }
        if net.iter().flatten().any(|p| !finite(*p)) {
            return Err("nonfinite device patch net");
        }
        // A bicubic surface lies inside its control-net convex hull. Keep one
        // device pixel of margin; completely off-screen geometry need not
        // exhaust the refinement budget before producing no visible samples.
        let outside = bounds.is_some_and(|(x0, y0, x1, y1)| {
            net.iter().flatten().all(|p| p.0 < x0 as f64 - 1.0)
                || net.iter().flatten().all(|p| p.0 > x1 as f64 + 1.0)
                || net.iter().flatten().all(|p| p.1 < y0 as f64 - 1.0)
                || net.iter().flatten().all(|p| p.1 > y1 as f64 + 1.0)
        });
        let p = points;
        let edges = [
            Boundary::new([p[0], p[1], p[2], p[3]], transform)?,
            Boundary::new([p[3], p[4], p[5], p[6]], transform)?,
            Boundary::new([p[9], p[8], p[7], p[6]], transform)?,
            Boundary::new([p[0], p[11], p[10], p[9]], transform)?,
        ];
        Ok(Self {
            steps: if outside { 1 } else { required_steps(&net)? },
            outside,
            net,
            edges,
            corners,
        })
    }

    fn point(&self, iu: usize, iv: usize, nu: usize, nv: usize) -> PatchPoint {
        if iu == 0 {
            return self.edges[0].point(iv, nv);
        }
        if iu == nu {
            return self.edges[2].point(iv, nv);
        }
        if iv == 0 {
            return self.edges[3].point(iu, nu);
        }
        if iv == nv {
            return self.edges[1].point(iu, nu);
        }
        let v = iv as f64 / nv as f64;
        curve(self.net.map(|row| curve(row, v)), iu as f64 / nu as f64)
    }
}

#[derive(Clone, Copy)]
struct EdgeOwner {
    key: EdgeKey,
    axis: usize,
}
#[derive(Clone, Copy)]
struct Axis {
    parent: usize,
    size: usize,
    steps: usize,
}
fn root(axes: &mut [Axis], mut index: usize) -> usize {
    while axes[index].parent != index {
        let parent = axes[index].parent;
        axes[index].parent = axes[parent].parent;
        index = axes[index].parent;
    }
    index
}
fn union(axes: &mut [Axis], a: usize, b: usize) {
    let mut a = root(axes, a);
    let mut b = root(axes, b);
    if a == b {
        return;
    }
    if axes[a].size < axes[b].size {
        std::mem::swap(&mut a, &mut b);
    }
    axes[b].parent = a;
    axes[a].size += axes[b].size;
    axes[a].steps = axes[a].steps.max(axes[b].steps);
}

pub(in crate::render::shading) struct PatchBatch<'a> {
    patches: Vec<Patch>,
    memory: Option<DecodeMemoryToken>,
    reserved_bytes: usize,
    options: ShadingRenderOptions<'a>,
    bounds: Option<(i32, i32, i32, i32)>,
}

impl<'a> PatchBatch<'a> {
    pub(in crate::render::shading) fn new(options: ShadingRenderOptions<'a>) -> Self {
        Self {
            patches: Vec::new(),
            memory: None,
            reserved_bytes: 0,
            options,
            bounds: None,
        }
    }

    pub(in crate::render::shading) fn with_device_bounds(
        mut self,
        bounds: Option<(i32, i32, i32, i32)>,
    ) -> Self {
        self.bounds = bounds;
        self
    }

    pub(in crate::render::shading) fn push(
        &mut self,
        points: &[PatchPoint],
        corners: &[MeshSample],
        kind: i64,
        transform: &Transform2D,
    ) -> Result<(), &'static str> {
        if self.patches.len() >= MAX_PATCH_MESH_PATCHES {
            return Err("patch count budget exceeded");
        }
        if !self.options.charge_work(64) {
            return Err("patch planning work budget exceeded");
        }
        let patch = Patch::new_clipped(points, corners, kind, transform, self.bounds)?;
        if self.patches.len() == self.patches.capacity() {
            let capacity = self
                .patches
                .capacity()
                .max(1)
                .checked_mul(2)
                .ok_or("patch capacity overflow")?
                .min(MAX_PATCH_MESH_PATCHES);
            let bytes = capacity
                .checked_mul(std::mem::size_of::<Patch>())
                .ok_or("patch storage overflow")?;
            let peak = bytes
                .checked_add(self.reserved_bytes)
                .ok_or("patch growth storage overflow")?;
            if peak > self.options.working_byte_limit {
                return Err("patch collection working-memory budget exceeded");
            }
            if !self.options.charge_work(self.patches.len() as u64) {
                return Err("patch growth work budget exceeded");
            }
            // Reallocation can transiently own old and new arrays. Reserve both
            // explicitly and release the old token only after its buffer drops.
            let token = self
                .options
                .reserve_bytes(bytes)
                .map_err(|_| "patch collection shared-memory budget exceeded")?;
            let mut grown = Vec::new();
            grown
                .try_reserve_exact(capacity)
                .map_err(|_| "patch collection allocation failed")?;
            grown.append(&mut self.patches);
            let previous = std::mem::replace(&mut self.patches, grown);
            drop(previous);
            self.memory = token;
            self.reserved_bytes = bytes;
        }
        self.patches.push(patch);
        Ok(())
    }

    fn axes(&self) -> Result<(Vec<Axis>, Option<DecodeMemoryToken>, usize), &'static str> {
        let count = self.patches.len();
        let bytes = count
            .checked_mul(4 * std::mem::size_of::<EdgeOwner>() + 2 * std::mem::size_of::<Axis>())
            .ok_or("patch topology storage overflow")?;
        let remaining = self
            .options
            .working_byte_limit
            .checked_sub(self.reserved_bytes)
            .ok_or("patch topology working-memory budget exceeded")?;
        let options = ShadingRenderOptions {
            working_byte_limit: remaining,
            ..self.options
        };
        let token = options
            .reserve_bytes(bytes)
            .map_err(|_| "patch topology working-memory budget exceeded")?;
        let mut axes = Vec::new();
        let mut edges = Vec::new();
        axes.try_reserve_exact(2 * count)
            .map_err(|_| "patch axes allocation failed")?;
        edges
            .try_reserve_exact(4 * count)
            .map_err(|_| "patch edges allocation failed")?;
        for (index, patch) in self.patches.iter().enumerate() {
            for axis in 0..2 {
                axes.push(Axis {
                    parent: 2 * index + axis,
                    size: 1,
                    steps: patch.steps,
                });
            }
            for (edge, boundary) in patch.edges.iter().enumerate() {
                edges.push(EdgeOwner {
                    key: boundary.key,
                    axis: 2 * index + if edge % 2 == 0 { 1 } else { 0 },
                });
            }
        }
        // Comparison sort is bounded by the patch cap; charge its log-linear
        // estimate and poll cancellation before/after this noninterruptible sort.
        let sort_units =
            edges.len() as u64 * (usize::BITS - edges.len().max(1).leading_zeros()) as u64;
        if !options.charge_work(sort_units) {
            return Err("patch topology work budget exceeded");
        }
        crate::cancel::check_current_cancel("patch topology sort")
            .map_err(|_| "patch topology cancelled")?;
        edges.sort_unstable_by_key(|a| a.key);
        for pair in edges.windows(2) {
            if pair[0].key == pair[1].key {
                union(&mut axes, pair[0].axis, pair[1].axis);
            }
        }
        for index in 0..axes.len() {
            let owner = root(&mut axes, index);
            axes[index].steps = axes[owner].steps;
        }
        crate::cancel::check_current_cancel("patch topology complete")
            .map_err(|_| "patch topology cancelled")?;
        // The edge vector is released here; keep its conservative reservation
        // through rasterization so planning and raster never overcommit.
        Ok((axes, token, bytes))
    }

    pub(in crate::render::shading) fn paint(
        &self,
        buf: &mut PixelBuffer,
        paint: &MeshPaint<'_>,
    ) -> Result<(), &'static str> {
        if self.patches.is_empty() {
            return Ok(());
        }
        let (axes, _topology_memory, topology_bytes) = self.axes()?;
        let remaining = self
            .options
            .working_byte_limit
            .checked_sub(self.reserved_bytes)
            .and_then(|n| n.checked_sub(topology_bytes))
            .ok_or("patch rows working-memory budget exceeded")?;
        let options = ShadingRenderOptions {
            working_byte_limit: remaining,
            ..paint.options
        };
        for (index, patch) in self.patches.iter().enumerate() {
            crate::cancel::check_current_cancel("adaptive patch rasterization")
                .map_err(|_| "patch rasterization cancelled")?;
            if patch.outside {
                continue;
            }
            let (nu, nv) = (axes[2 * index].steps, axes[2 * index + 1].steps);
            let vertices = (nu + 1)
                .checked_mul(nv + 1)
                .ok_or("patch vertex work overflow")?;
            if !options.charge_work(vertices as u64) {
                return Err("patch vertex work budget exceeded");
            }
            let _rows = options
                .reserve_mesh_storage(2 * (nv + 1))
                .map_err(|_| "patch rows working-memory budget exceeded")?;
            let mut previous = Vec::new();
            let mut next = Vec::new();
            previous
                .try_reserve_exact(nv + 1)
                .map_err(|_| "patch row allocation failed")?;
            next.try_reserve_exact(nv + 1)
                .map_err(|_| "patch row allocation failed")?;
            let paint = MeshPaint {
                options,
                patch_corners: Some(&patch.corners),
                ..*paint
            };
            for iu in 0..=nu {
                crate::cancel::check_current_cancel("adaptive patch row")
                    .map_err(|_| "patch row cancelled")?;
                next.clear();
                for iv in 0..=nv {
                    let (dx, dy) = patch.point(iu, iv, nu, nv);
                    if !dx.is_finite() || !dy.is_finite() {
                        return Err("nonfinite adaptive patch sample");
                    }
                    let sample = MeshSample::new(&[iu as f64 / nu as f64, iv as f64 / nv as f64])
                        .ok_or("invalid adaptive patch parameter")?;
                    next.push(MeshVertex { dx, dy, sample });
                }
                if iu > 0 {
                    for iv in 0..nv {
                        fill_triangle(buf, previous[iv], next[iv], previous[iv + 1], &paint);
                        fill_triangle(buf, next[iv], next[iv + 1], previous[iv + 1], &paint);
                        if options.failed() {
                            return Err("adaptive patch rasterization failed");
                        }
                    }
                }
                std::mem::swap(&mut previous, &mut next);
            }
        }
        Ok(())
    }
}
