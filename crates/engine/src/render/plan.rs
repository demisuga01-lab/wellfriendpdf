//! Packed retained display-list and backend-neutral render plan.
//!
//! This is intentionally additive during migration: compact path/state arenas
//! serve vector replay now, while high-level PDF operations remain in a cold
//! source table until each resource class has a native compiled payload.

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{Result, WellfriendError};

use super::contract::{DisplayItemId, RenderContract};
use super::display_list::{
    CpuRenderDevice, DisplayList, DisplayOp, DrawState, RenderBounds, RenderDevice, RenderTile,
};
use super::path::{FillRule, Path, PathSegment};
use super::{PixelBuffer, RenderMode, Transform2D};

const OP_SAVE: u16 = 1;
const OP_RESTORE: u16 = 2;
const OP_CLIP: u16 = 3;
const OP_FILL: u16 = 4;
const OP_STROKE: u16 = 5;
const OP_STATE: u16 = 6;
const OP_NATIVE_TEXT: u16 = 7;
const OP_NATIVE_IMAGE: u16 = 8;
const OP_NATIVE_SHADING: u16 = 9;
const OP_NATIVE_PATTERN: u16 = 10;
const OP_NATIVE_INLINE_IMAGE: u16 = 11;
const OP_NATIVE_FORM: u16 = 12;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x00000100000001b3;

fn hash_mix(hash: &mut u64, value: u64) {
    *hash ^= value;
    *hash = hash.wrapping_mul(FNV_PRIME);
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    hash_mix(hash, bytes.len() as u64);
    for byte in bytes {
        hash_mix(hash, u64::from(*byte));
    }
}

fn path_fingerprint(path: &Path) -> u64 {
    let mut hash = FNV_OFFSET;
    hash_mix(&mut hash, path.segments.len() as u64);
    for segment in &path.segments {
        match segment {
            PathSegment::MoveTo(x, y) => {
                hash_mix(&mut hash, 1);
                hash_mix(&mut hash, x.to_bits());
                hash_mix(&mut hash, y.to_bits());
            }
            PathSegment::LineTo(x, y) => {
                hash_mix(&mut hash, 2);
                hash_mix(&mut hash, x.to_bits());
                hash_mix(&mut hash, y.to_bits());
            }
            PathSegment::CubicTo {
                cp1x,
                cp1y,
                cp2x,
                cp2y,
                x,
                y,
            } => {
                hash_mix(&mut hash, 3);
                for value in [cp1x, cp1y, cp2x, cp2y, x, y] {
                    hash_mix(&mut hash, value.to_bits());
                }
            }
            PathSegment::ClosePath => hash_mix(&mut hash, 4),
        }
    }
    match path.current_point {
        Some((x, y)) => {
            hash_mix(&mut hash, 5);
            hash_mix(&mut hash, x.to_bits());
            hash_mix(&mut hash, y.to_bits());
        }
        None => hash_mix(&mut hash, 6),
    }
    hash
}

fn hash_optional_f32(hash: &mut u64, value: Option<[f32; 4]>) {
    match value {
        Some(values) => {
            hash_mix(hash, 1);
            for value in values {
                hash_mix(hash, u64::from(value.to_bits()));
            }
        }
        None => hash_mix(hash, 0),
    }
}

fn same_optional_f32(left: Option<[f32; 4]>, right: Option<[f32; 4]>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| left.to_bits() == right.to_bits()),
        (None, None) => true,
        _ => false,
    }
}

fn draw_state_fingerprint(state: &DrawState) -> u64 {
    let mut hash = FNV_OFFSET;
    for value in state.ctm.to_array() {
        hash_mix(&mut hash, value.to_bits());
    }
    for value in state.fill_color {
        hash_mix(&mut hash, u64::from(value));
    }
    for value in state.stroke_color {
        hash_mix(&mut hash, u64::from(value));
    }
    hash_optional_f32(&mut hash, state.fill_cmyk);
    hash_optional_f32(&mut hash, state.stroke_cmyk);
    hash_mix(&mut hash, state.blend_mode as u64);
    hash_bytes(&mut hash, state.rendering_intent.as_bytes());
    hash_mix(&mut hash, if state.stroke_overprint { 1 } else { 0 });
    hash_mix(&mut hash, if state.fill_overprint { 1 } else { 0 });
    hash_mix(&mut hash, state.overprint_mode as u64);
    hash_mix(&mut hash, state.line_width.to_bits());
    hash_mix(&mut hash, state.line_cap.clone() as u64);
    hash_mix(&mut hash, state.line_join.clone() as u64);
    hash_mix(&mut hash, state.miter_limit.to_bits());
    hash_mix(&mut hash, state.dash.render_cache_fingerprint());
    hash
}

fn same_draw_state(left: &DrawState, right: &DrawState) -> bool {
    left.ctm.to_array().map(f64::to_bits) == right.ctm.to_array().map(f64::to_bits)
        && left.fill_color == right.fill_color
        && left.stroke_color == right.stroke_color
        && same_optional_f32(left.fill_cmyk, right.fill_cmyk)
        && same_optional_f32(left.stroke_cmyk, right.stroke_cmyk)
        && left.blend_mode == right.blend_mode
        && left.rendering_intent == right.rendering_intent
        && left.stroke_overprint == right.stroke_overprint
        && left.fill_overprint == right.fill_overprint
        && left.overprint_mode == right.overprint_mode
        && left.line_width.to_bits() == right.line_width.to_bits()
        && left.line_cap == right.line_cap
        && left.line_join == right.line_join
        && left.miter_limit.to_bits() == right.miter_limit.to_bits()
        && left.dash.same_for_render(&right.dash)
}

/// Fixed-size hot command. Its payload indexes immutable arenas and never
/// carries a string, PDF dictionary, or `ContentOperation` directly.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotDisplayOp {
    pub opcode: u16,
    pub flags: u16,
    pub bounds_id: u32,
    pub state_id: u32,
    pub payload_offset: u32,
    pub payload_len: u32,
    pub source_link_id: u32,
}

#[derive(Clone, Debug)]
pub enum ColdPayload {
    State(crate::content::ContentOperation),
    Operation(crate::content::ContentOperation),
    Operations(Vec<crate::content::ContentOperation>),
}

#[derive(Clone, Debug, Default)]
pub struct PackedColdTables {
    pub payloads: Vec<ColdPayload>,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct PackedDisplayList {
    pub hot_ops: Vec<HotDisplayOp>,
    pub paths: Vec<Path>,
    pub clip_transforms: Vec<Transform2D>,
    pub states: Vec<DrawState>,
    pub bounds: Vec<Option<RenderBounds>>,
    pub cold: PackedColdTables,
    source: Arc<DisplayList>,
    requires_native_replay: bool,
}

impl PackedDisplayList {
    pub fn compile(source: DisplayList) -> Self {
        let source = Arc::new(source);
        let mut hot_ops = Vec::with_capacity(source.ops.len());
        let mut paths = Vec::new();
        let mut clip_transforms = Vec::new();
        let mut states = Vec::new();
        let mut bounds = Vec::new();
        let mut cold = PackedColdTables::default();
        let mut state_ids = HashMap::<u64, Vec<u32>>::new();
        let mut path_ids = HashMap::<u64, Vec<u32>>::new();
        let mut requires_native_replay = false;

        let intern_path =
            |path: &Path, paths: &mut Vec<Path>, path_ids: &mut HashMap<u64, Vec<u32>>| {
                let fingerprint = path_fingerprint(path);
                match path_ids.get(&fingerprint).and_then(|ids| {
                    ids.iter().copied().find(|id| {
                        paths
                            .get(*id as usize)
                            .is_some_and(|existing| existing == path)
                    })
                }) {
                    Some(id) => id,
                    None => {
                        let id = u32::try_from(paths.len()).unwrap_or(u32::MAX);
                        paths.push(path.clone());
                        path_ids.entry(fingerprint).or_default().push(id);
                        id
                    }
                }
            };
        let intern_state = |state: &DrawState,
                            states: &mut Vec<DrawState>,
                            state_ids: &mut HashMap<u64, Vec<u32>>| {
            let fingerprint = draw_state_fingerprint(state);
            match state_ids.get(&fingerprint).and_then(|ids| {
                ids.iter().copied().find(|id| {
                    states
                        .get(*id as usize)
                        .is_some_and(|existing| same_draw_state(existing, state))
                })
            }) {
                Some(id) => id,
                None => {
                    let id = u32::try_from(states.len()).unwrap_or(u32::MAX);
                    states.push(state.clone());
                    state_ids.entry(fingerprint).or_default().push(id);
                    id
                }
            }
        };
        let push_bounds = |value: Option<RenderBounds>, bounds: &mut Vec<Option<RenderBounds>>| {
            let id = u32::try_from(bounds.len()).unwrap_or(u32::MAX);
            bounds.push(value);
            id
        };
        let push_payload = |payload: ColdPayload, cold: &mut PackedColdTables| {
            let id = u32::try_from(cold.payloads.len()).unwrap_or(u32::MAX);
            cold.payloads.push(payload);
            id
        };

        let list_has_native_payload = source.ops.iter().any(DisplayOp::is_native_high_level);
        for (index, op) in source.ops.iter().enumerate() {
            if !list_has_native_payload && matches!(op, DisplayOp::StateOp { .. }) {
                continue;
            }
            let item = DisplayItemId(u32::try_from(index + 1).unwrap_or(u32::MAX));
            let (opcode, flags, bounds_id, state_id, payload_offset, payload_len) = match op {
                DisplayOp::Save => (OP_SAVE, 0, u32::MAX, u32::MAX, 0, 0),
                DisplayOp::Restore => (OP_RESTORE, 0, u32::MAX, u32::MAX, 0, 0),
                DisplayOp::Clip {
                    path,
                    ctm,
                    rule,
                    bounds: op_bounds,
                } => {
                    let path_id = intern_path(path, &mut paths, &mut path_ids);
                    let transform_id = u32::try_from(clip_transforms.len()).unwrap_or(u32::MAX);
                    clip_transforms.push(*ctm);
                    (
                        OP_CLIP,
                        fill_rule_flag(*rule),
                        push_bounds(*op_bounds, &mut bounds),
                        transform_id,
                        path_id,
                        1,
                    )
                }
                DisplayOp::FillPath {
                    path,
                    state,
                    rule,
                    bounds: op_bounds,
                } => (
                    OP_FILL,
                    fill_rule_flag(*rule),
                    push_bounds(*op_bounds, &mut bounds),
                    intern_state(state, &mut states, &mut state_ids),
                    intern_path(path, &mut paths, &mut path_ids),
                    1,
                ),
                DisplayOp::StrokePath {
                    path,
                    state,
                    bounds: op_bounds,
                } => (
                    OP_STROKE,
                    0,
                    push_bounds(*op_bounds, &mut bounds),
                    intern_state(state, &mut states, &mut state_ids),
                    intern_path(path, &mut paths, &mut path_ids),
                    1,
                ),
                DisplayOp::StateOp { op, .. } => {
                    requires_native_replay = true;
                    (
                        OP_STATE,
                        0,
                        u32::MAX,
                        u32::MAX,
                        push_payload(ColdPayload::State(op.clone()), &mut cold),
                        1,
                    )
                }
                DisplayOp::NativeTextOp {
                    op,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    (
                        OP_NATIVE_TEXT,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        push_payload(ColdPayload::Operation(op.clone()), &mut cold),
                        1,
                    )
                }
                DisplayOp::NativeImageXObject {
                    op,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    (
                        OP_NATIVE_IMAGE,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        push_payload(ColdPayload::Operation(op.clone()), &mut cold),
                        1,
                    )
                }
                DisplayOp::NativeShadingOp {
                    op,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    (
                        OP_NATIVE_SHADING,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        push_payload(ColdPayload::Operation(op.clone()), &mut cold),
                        1,
                    )
                }
                DisplayOp::NativePatternPathOp {
                    ops,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    (
                        OP_NATIVE_PATTERN,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        push_payload(ColdPayload::Operations(ops.clone()), &mut cold),
                        u32::try_from(ops.len()).unwrap_or(u32::MAX),
                    )
                }
                DisplayOp::NativeInlineImage {
                    ops,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    (
                        OP_NATIVE_INLINE_IMAGE,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        push_payload(ColdPayload::Operations(ops.clone()), &mut cold),
                        u32::try_from(ops.len()).unwrap_or(u32::MAX),
                    )
                }
                DisplayOp::NativeFormXObject {
                    op,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    (
                        OP_NATIVE_FORM,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        push_payload(ColdPayload::Operation(op.clone()), &mut cold),
                        1,
                    )
                }
            };
            hot_ops.push(HotDisplayOp {
                opcode,
                flags,
                bounds_id,
                state_id,
                payload_offset,
                payload_len,
                source_link_id: item.0,
            });
        }

        Self {
            hot_ops,
            paths,
            clip_transforms,
            states,
            bounds,
            cold,
            source,
            requires_native_replay,
        }
    }

    pub fn source(&self) -> &DisplayList {
        self.source.as_ref()
    }

    pub fn requires_native_replay(&self) -> bool {
        self.requires_native_replay
    }

    pub fn hot_operation_count(&self) -> usize {
        self.hot_ops.len()
    }

    pub fn replay_vector(&self, device: &mut dyn RenderDevice, selected: &[usize]) -> Result<()> {
        if self.requires_native_replay {
            return Err(WellfriendError::UnsupportedFeature(
                "packed vector replay requires a native compiled payload for high-level PDF operations".to_string(),
            ));
        }
        for &index in selected {
            let op = self.hot_ops.get(index).ok_or_else(|| {
                WellfriendError::MalformedPdf(
                    "packed display-list index was out of bounds".to_string(),
                )
            })?;
            match op.opcode {
                OP_SAVE => device.save(),
                OP_RESTORE => device.restore(),
                OP_CLIP => {
                    let path = self.paths.get(op.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed clip path missing".to_string())
                    })?;
                    let ctm = self
                        .clip_transforms
                        .get(op.state_id as usize)
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "packed clip transform missing".to_string(),
                            )
                        })?;
                    device.clip_path(path, ctm, fill_rule_from_flag(op.flags));
                }
                OP_FILL => {
                    let path = self.paths.get(op.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed fill path missing".to_string())
                    })?;
                    let state = self.states.get(op.state_id as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed fill state missing".to_string())
                    })?;
                    device.fill_path(path, state, fill_rule_from_flag(op.flags));
                }
                OP_STROKE => {
                    let path = self.paths.get(op.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed stroke path missing".to_string())
                    })?;
                    let state = self.states.get(op.state_id as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed stroke state missing".to_string())
                    })?;
                    device.stroke_path(path, state);
                }
                _ => {
                    return Err(WellfriendError::UnsupportedFeature(
                        "packed vector replay encountered a non-vector operation".to_string(),
                    ))
                }
            }
        }
        Ok(())
    }
}

fn fill_rule_flag(rule: FillRule) -> u16 {
    match rule {
        FillRule::NonZero => 0,
        FillRule::EvenOdd => 1,
    }
}

fn fill_rule_from_flag(flag: u16) -> FillRule {
    if flag & 1 == 1 {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    }
}

#[derive(Clone, Debug)]
pub struct RenderSpatialIndex {
    known: Vec<(usize, RenderBounds)>,
    unknown: Vec<usize>,
}

impl RenderSpatialIndex {
    pub fn compile(list: &PackedDisplayList) -> Self {
        let mut known = Vec::new();
        let mut unknown = Vec::new();
        for (index, hot) in list.hot_ops.iter().enumerate() {
            match list.bounds.get(hot.bounds_id as usize).copied().flatten() {
                Some(bounds) => known.push((index, bounds)),
                None => unknown.push(index),
            }
        }
        known.sort_by_key(|(_, bounds)| (bounds.y0, bounds.x0, bounds.y1, bounds.x1));
        Self { known, unknown }
    }

    pub fn query(&self, tile: RenderTile) -> Vec<usize> {
        let mut selected = self.unknown.clone();
        let tile_bounds = RenderBounds {
            x0: i32::try_from(tile.x).unwrap_or(i32::MAX),
            y0: i32::try_from(tile.y).unwrap_or(i32::MAX),
            x1: i32::try_from(tile.x.saturating_add(tile.width)).unwrap_or(i32::MAX),
            y1: i32::try_from(tile.y.saturating_add(tile.height)).unwrap_or(i32::MAX),
        };
        selected.extend(self.known.iter().filter_map(|(index, bounds)| {
            bounds.intersect(tile_bounds).is_some().then_some(*index)
        }));
        selected.sort_unstable();
        selected.dedup();
        selected
    }
}

#[derive(Clone, Debug)]
pub struct RenderBatch {
    pub first_operation: usize,
    pub operation_count: usize,
    pub contains_native_payload: bool,
}

#[derive(Clone, Debug)]
pub struct RenderPlan {
    pub contract: RenderContract,
    pub packed: Arc<PackedDisplayList>,
    pub spatial_index: RenderSpatialIndex,
    pub batches: Vec<RenderBatch>,
}

impl RenderPlan {
    pub fn compile(list: DisplayList, contract: RenderContract) -> Result<Self> {
        contract.validate()?;
        let packed = Arc::new(PackedDisplayList::compile(list));
        let contains_native_payload = packed.requires_native_replay();
        let batches = (!packed.hot_ops.is_empty())
            .then_some(RenderBatch {
                first_operation: 0,
                operation_count: packed.hot_ops.len(),
                contains_native_payload,
            })
            .into_iter()
            .collect();
        let spatial_index = RenderSpatialIndex::compile(&packed);
        Ok(Self {
            contract,
            packed,
            spatial_index,
            batches,
        })
    }

    pub fn execute_vector_tile(&self, tile: RenderTile) -> Result<Option<PixelBuffer>> {
        self.contract.validate()?;
        if self.packed.requires_native_replay() {
            return Ok(None);
        }
        let selected = self.spatial_index.query(tile);
        let viewport =
            self.packed
                .source()
                .viewport
                .pixel_window(tile.x, tile.y, tile.width, tile.height);
        let mut device =
            CpuRenderDevice::new(viewport, RenderMode::from(self.contract.compositing));
        self.packed.replay_vector(&mut device, &selected)?;
        Ok(Some(device.into_buffer()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::operation::Operand;
    use crate::render::{build_display_list, render_display_list, Viewport};
    use crate::ContentOperation;

    #[test]
    fn packed_vector_plan_replays_without_cold_payload_access() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(1.0), Operand::Real(0.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(10.0),
                    Operand::Real(10.0),
                    Operand::Real(20.0),
                    Operand::Real(20.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 40.0, 40.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        assert!(list.native_vector_only());
        let expected = render_display_list(&list, RenderMode::Compat);
        let contract = RenderContract::for_viewport(
            super::super::contract::RevisionId(1),
            super::super::contract::ObjectIdentityId(1),
            1,
            &viewport,
            RenderTile::full(viewport.width_px, viewport.height_px),
            RenderMode::Compat,
        );
        let plan = RenderPlan::compile(list, contract).expect("compile plan");
        assert!(plan.packed.cold.payloads.is_empty());
        let actual = plan
            .execute_vector_tile(RenderTile::full(viewport.width_px, viewport.height_px))
            .expect("execute plan")
            .expect("vector-only plan");
        assert_eq!(expected.rgba_bytes(), actual.rgba_bytes());
    }

    #[test]
    fn spatial_index_preserves_paint_order_after_culling() {
        let bounds = RenderBounds {
            x0: 10,
            y0: 10,
            x1: 20,
            y1: 20,
        };
        let index = RenderSpatialIndex {
            known: vec![(2, bounds), (1, bounds)],
            unknown: vec![0],
        };
        assert_eq!(
            index.query(RenderTile {
                x: 10,
                y: 10,
                width: 2,
                height: 2
            }),
            vec![0, 1, 2]
        );
    }
}
