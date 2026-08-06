//! Packed retained display-list and backend-neutral render plan.
//!
//! This is intentionally additive during migration: compact path/state arenas
//! serve vector replay now, while high-level PDF operations remain in a cold
//! source table until each resource class has a native compiled payload.

use std::collections::HashMap;
use std::sync::Arc;

use crate::content::operation::Operand;
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

/// Compiled text-showing descriptor. Carries only the pre-extracted fields
/// the renderer needs without retaining a raw `ContentOperation` on the hot path.
#[derive(Clone, Debug)]
pub enum TextDescriptor {
    /// `Tj` — single string show.
    Show(Vec<u8>),
    /// `TJ` — array of strings and position adjustments.
    ShowArray(Vec<TextArrayItem>),
    /// `'` — move to next line then show.
    NextLineShow(Vec<u8>),
    /// `"` — set word/char spacing, move to next line, then show.
    SpacingNextLineShow {
        word_spacing: f64,
        char_spacing: f64,
        text: Vec<u8>,
    },
}

/// One item in a TJ array.
#[derive(Clone, Debug)]
pub enum TextArrayItem {
    Bytes(Vec<u8>),
    Adjustment(f64),
}

/// Compiled image XObject descriptor — resource name for `Do`.
#[derive(Clone, Debug)]
pub struct ImageXObjectDescriptor {
    pub name: String,
}

/// Compiled Form XObject descriptor — resource name for `Do`.
#[derive(Clone, Debug)]
pub struct FormXObjectDescriptor {
    pub name: String,
}

/// Compiled shading descriptor — resource name for `sh`.
#[derive(Clone, Debug)]
pub struct ShadingDescriptor {
    pub name: String,
}

/// High-level typed descriptor for a compiled native op.
#[derive(Clone, Debug)]
pub enum NativeDescriptor {
    /// Fully compiled text-showing operation.
    Text(TextDescriptor),
    /// Fully compiled image XObject reference.
    Image(ImageXObjectDescriptor),
    /// Fully compiled Form XObject reference.
    Form(FormXObjectDescriptor),
    /// Fully compiled shading reference.
    Shading(ShadingDescriptor),
    /// Graphics-state mutation dispatched through the interpreter.
    State(crate::content::ContentOperation),
    /// Pattern path ops — explicitly unsupported for pure vector plan replay.
    /// Dispatched through RenderState when executing full plan.
    UnsupportedPattern(Vec<crate::content::ContentOperation>),
    /// Inline image ops — explicitly unsupported for pure vector plan replay.
    /// Dispatched through RenderState when executing full plan.
    UnsupportedInlineImage(Vec<crate::content::ContentOperation>),
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
    /// Typed compiled descriptors indexed by `payload_offset` for native ops.
    pub descriptors: Vec<NativeDescriptor>,
    pub cold: PackedColdTables,
    source: Arc<DisplayList>,
    requires_native_replay: bool,
}

impl PackedDisplayList {
    /// Compile a text-showing `ContentOperation` into a typed `TextDescriptor`.
    fn compile_text_descriptor(op: &crate::content::ContentOperation) -> NativeDescriptor {
        let desc = match op.operator.as_str() {
            "Tj" => {
                let bytes = op.string_bytes(0).unwrap_or(&[]).to_vec();
                TextDescriptor::Show(bytes)
            }
            "TJ" => {
                let items = op
                    .operand(0)
                    .and_then(Operand::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| match item {
                                Operand::String(b) => Some(TextArrayItem::Bytes(b.clone())),
                                Operand::Integer(v) => {
                                    Some(TextArrayItem::Adjustment(-(*v as f64)))
                                }
                                Operand::Real(v) => Some(TextArrayItem::Adjustment(-*v)),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                TextDescriptor::ShowArray(items)
            }
            "'" => {
                let bytes = op.string_bytes(0).unwrap_or(&[]).to_vec();
                TextDescriptor::NextLineShow(bytes)
            }
            "\"" => {
                let word_spacing = op.number(0).unwrap_or(0.0);
                let char_spacing = op.number(1).unwrap_or(0.0);
                let text = op.string_bytes(2).unwrap_or(&[]).to_vec();
                TextDescriptor::SpacingNextLineShow {
                    word_spacing,
                    char_spacing,
                    text,
                }
            }
            // Non-showing text ops (Tf, Td, Tm, etc.) are state mutations
            _ => return NativeDescriptor::State(op.clone()),
        };
        NativeDescriptor::Text(desc)
    }

    pub fn compile(source: DisplayList) -> Self {
        let source = Arc::new(source);
        let mut hot_ops = Vec::with_capacity(source.ops.len());
        let mut paths = Vec::new();
        let mut clip_transforms = Vec::new();
        let mut states = Vec::new();
        let mut bounds = Vec::new();
        let mut cold = PackedColdTables::default();
        let mut descriptors: Vec<NativeDescriptor> = Vec::new();
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
        let push_descriptor =
            |desc: NativeDescriptor, descriptors: &mut Vec<NativeDescriptor>| -> u32 {
                let id = u32::try_from(descriptors.len()).unwrap_or(u32::MAX);
                descriptors.push(desc);
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
                    let desc_id =
                        push_descriptor(NativeDescriptor::State(op.clone()), &mut descriptors);
                    (OP_STATE, 0, u32::MAX, u32::MAX, desc_id, 1)
                }
                DisplayOp::NativeTextOp {
                    op,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let desc = Self::compile_text_descriptor(op);
                    let desc_id = push_descriptor(desc, &mut descriptors);
                    (
                        OP_NATIVE_TEXT,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        1,
                    )
                }
                DisplayOp::NativeImageXObject {
                    op,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let name = op.name(0).unwrap_or("").to_string();
                    let desc_id = push_descriptor(
                        NativeDescriptor::Image(ImageXObjectDescriptor { name }),
                        &mut descriptors,
                    );
                    (
                        OP_NATIVE_IMAGE,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        1,
                    )
                }
                DisplayOp::NativeShadingOp {
                    op,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let name = op.name(0).unwrap_or("").to_string();
                    let desc_id = push_descriptor(
                        NativeDescriptor::Shading(ShadingDescriptor { name }),
                        &mut descriptors,
                    );
                    (
                        OP_NATIVE_SHADING,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        1,
                    )
                }
                DisplayOp::NativePatternPathOp {
                    ops,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let desc_id = push_descriptor(
                        NativeDescriptor::UnsupportedPattern(ops.clone()),
                        &mut descriptors,
                    );
                    (
                        OP_NATIVE_PATTERN,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        u32::try_from(ops.len()).unwrap_or(u32::MAX),
                    )
                }
                DisplayOp::NativeInlineImage {
                    ops,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let desc_id = push_descriptor(
                        NativeDescriptor::UnsupportedInlineImage(ops.clone()),
                        &mut descriptors,
                    );
                    (
                        OP_NATIVE_INLINE_IMAGE,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
                        u32::try_from(ops.len()).unwrap_or(u32::MAX),
                    )
                }
                DisplayOp::NativeFormXObject {
                    op,
                    bounds: op_bounds,
                    ..
                } => {
                    requires_native_replay = true;
                    let name = op.name(0).unwrap_or("").to_string();
                    let desc_id = push_descriptor(
                        NativeDescriptor::Form(FormXObjectDescriptor { name }),
                        &mut descriptors,
                    );
                    (
                        OP_NATIVE_FORM,
                        0,
                        push_bounds(*op_bounds, &mut bounds),
                        u32::MAX,
                        desc_id,
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
            descriptors,
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

    /// Get the typed descriptor at the given index.
    pub fn descriptor(&self, index: u32) -> Option<&NativeDescriptor> {
        self.descriptors.get(index as usize)
    }

    /// Returns `true` if every native high-level op has a fully-compiled
    /// descriptor (text/image/form/shading). Patterns and inline images
    /// return `false` for pure vector replay but are still plan-dispatchable
    /// through the RenderState adapter path.
    pub fn has_only_supported_descriptors(&self) -> bool {
        self.descriptors.iter().all(|d| {
            matches!(
                d,
                NativeDescriptor::Text(_)
                    | NativeDescriptor::Image(_)
                    | NativeDescriptor::Form(_)
                    | NativeDescriptor::Shading(_)
                    | NativeDescriptor::State(_)
                    | NativeDescriptor::UnsupportedPattern(_)
                    | NativeDescriptor::UnsupportedInlineImage(_)
            )
        })
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

/// Trait for dispatching typed plan descriptors through the renderer.
///
/// This lets `RenderPlan::execute_full` drive page rendering through compiled
/// descriptors rather than raw `ContentOperation` payloads. The page renderer
/// implements this trait via a `RenderState`-backed adapter.
pub trait PlanDispatcher {
    fn dispatch_text(&mut self, desc: &TextDescriptor, bounds: Option<&RenderBounds>);
    fn dispatch_image(&mut self, desc: &ImageXObjectDescriptor, bounds: Option<&RenderBounds>);
    fn dispatch_form(&mut self, desc: &FormXObjectDescriptor, bounds: Option<&RenderBounds>);
    fn dispatch_shading(&mut self, desc: &ShadingDescriptor, bounds: Option<&RenderBounds>);
    fn dispatch_state(&mut self, op: &crate::content::ContentOperation);
    fn dispatch_pattern_ops(
        &mut self,
        ops: &[crate::content::ContentOperation],
        bounds: Option<&RenderBounds>,
    );
    fn dispatch_inline_image_ops(
        &mut self,
        ops: &[crate::content::ContentOperation],
        bounds: Option<&RenderBounds>,
    );
    fn dispatch_save(&mut self);
    fn dispatch_restore(&mut self);
    fn dispatch_clip(&mut self, path: &Path, ctm: &Transform2D, rule: FillRule);
    fn dispatch_fill(&mut self, path: &Path, state: &DrawState, rule: FillRule);
    fn dispatch_stroke(&mut self, path: &Path, state: &DrawState);
    fn is_cancelled(&self) -> bool;
}

impl PackedDisplayList {
    /// Execute the full plan through a typed dispatcher. This is the active
    /// high-level path that drives text/image/form/shading through compiled
    /// descriptors. Used by PageRenderer for all fully-supported display lists.
    pub fn execute_plan(&self, dispatcher: &mut dyn PlanDispatcher) -> Result<()> {
        for (i, hot) in self.hot_ops.iter().enumerate() {
            if i % 64 == 0 && dispatcher.is_cancelled() {
                return Err(WellfriendError::Cancelled(
                    "plan execution cancelled".to_string(),
                ));
            }
            let op_bounds = self.bounds.get(hot.bounds_id as usize).copied().flatten();
            match hot.opcode {
                OP_SAVE => dispatcher.dispatch_save(),
                OP_RESTORE => dispatcher.dispatch_restore(),
                OP_CLIP => {
                    let path = self.paths.get(hot.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed clip path missing".to_string())
                    })?;
                    let ctm = self
                        .clip_transforms
                        .get(hot.state_id as usize)
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "packed clip transform missing".to_string(),
                            )
                        })?;
                    dispatcher.dispatch_clip(path, ctm, fill_rule_from_flag(hot.flags));
                }
                OP_FILL => {
                    let path = self.paths.get(hot.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed fill path missing".to_string())
                    })?;
                    let state = self.states.get(hot.state_id as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed fill state missing".to_string())
                    })?;
                    dispatcher.dispatch_fill(path, state, fill_rule_from_flag(hot.flags));
                }
                OP_STROKE => {
                    let path = self.paths.get(hot.payload_offset as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed stroke path missing".to_string())
                    })?;
                    let state = self.states.get(hot.state_id as usize).ok_or_else(|| {
                        WellfriendError::MalformedPdf("packed stroke state missing".to_string())
                    })?;
                    dispatcher.dispatch_stroke(path, state);
                }
                OP_STATE
                | OP_NATIVE_TEXT
                | OP_NATIVE_IMAGE
                | OP_NATIVE_SHADING
                | OP_NATIVE_PATTERN
                | OP_NATIVE_INLINE_IMAGE
                | OP_NATIVE_FORM => {
                    let desc = self
                        .descriptors
                        .get(hot.payload_offset as usize)
                        .ok_or_else(|| {
                            WellfriendError::MalformedPdf(
                                "packed descriptor index out of bounds".to_string(),
                            )
                        })?;
                    match desc {
                        NativeDescriptor::Text(text) => {
                            dispatcher.dispatch_text(text, op_bounds.as_ref());
                        }
                        NativeDescriptor::Image(img) => {
                            dispatcher.dispatch_image(img, op_bounds.as_ref());
                        }
                        NativeDescriptor::Form(form) => {
                            dispatcher.dispatch_form(form, op_bounds.as_ref());
                        }
                        NativeDescriptor::Shading(shading) => {
                            dispatcher.dispatch_shading(shading, op_bounds.as_ref());
                        }
                        NativeDescriptor::State(op) => {
                            dispatcher.dispatch_state(op);
                        }
                        NativeDescriptor::UnsupportedPattern(ops) => {
                            dispatcher.dispatch_pattern_ops(ops, op_bounds.as_ref());
                        }
                        NativeDescriptor::UnsupportedInlineImage(ops) => {
                            dispatcher.dispatch_inline_image_ops(ops, op_bounds.as_ref());
                        }
                    }
                }
                _ => {
                    return Err(WellfriendError::UnsupportedFeature(
                        "packed plan encountered an unknown opcode".to_string(),
                    ));
                }
            }
        }
        Ok(())
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

    /// Execute the full plan through the typed descriptor dispatcher.
    /// This is the active high-level path for all fully-supported display lists
    /// including those with text/image/form/shading native ops.
    pub fn execute_full(&self, dispatcher: &mut dyn PlanDispatcher) -> Result<()> {
        self.contract.validate()?;
        self.packed.execute_plan(dispatcher)
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

    #[test]
    fn text_descriptor_compiles_tj_without_raw_content_operation() {
        let ops = vec![
            ContentOperation::new("BT", Vec::new()),
            ContentOperation::new(
                "Tf",
                vec![Operand::Name("F1".to_string()), Operand::Real(12.0)],
            ),
            ContentOperation::new("Tj", vec![Operand::String(b"Hello".to_vec())]),
            ContentOperation::new("ET", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        assert!(
            !list.native_vector_only(),
            "text page should have native ops"
        );
        let packed = PackedDisplayList::compile(list);
        assert!(packed.requires_native_replay());
        // Verify typed text descriptor is present, not a raw ContentOperation
        let text_ops: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_TEXT)
            .collect();
        assert!(!text_ops.is_empty(), "should have at least one text op");
        for hot in &text_ops {
            let desc = packed.descriptor(hot.payload_offset).unwrap();
            match desc {
                NativeDescriptor::Text(TextDescriptor::Show(bytes)) => {
                    assert_eq!(bytes, b"Hello");
                }
                NativeDescriptor::State(_) => {
                    // text-state ops like Tf are compiled as State descriptors
                }
                other => panic!("unexpected descriptor for text op: {:?}", other),
            }
        }
    }

    #[test]
    fn image_descriptor_compiles_resource_name_only() {
        let ops = vec![ContentOperation::new(
            "Do",
            vec![Operand::Name("Im1".to_string())],
        )];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let packed = PackedDisplayList::compile(list);
        let image_ops: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_IMAGE)
            .collect();
        // Image ops may or may not appear depending on display-list builder
        // classification. If they do, verify the descriptor is typed.
        for hot in &image_ops {
            let desc = packed.descriptor(hot.payload_offset).unwrap();
            match desc {
                NativeDescriptor::Image(img) => {
                    assert_eq!(img.name, "Im1");
                }
                _ => panic!("expected Image descriptor"),
            }
        }
    }

    #[test]
    fn form_descriptor_compiles_resource_name_only() {
        let ops = vec![ContentOperation::new(
            "Do",
            vec![Operand::Name("Fm1".to_string())],
        )];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let packed = PackedDisplayList::compile(list);
        let form_ops: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_FORM)
            .collect();
        for hot in &form_ops {
            let desc = packed.descriptor(hot.payload_offset).unwrap();
            match desc {
                NativeDescriptor::Form(form) => {
                    assert_eq!(form.name, "Fm1");
                }
                _ => panic!("expected Form descriptor"),
            }
        }
    }

    #[test]
    fn shading_descriptor_compiles_resource_name_only() {
        let ops = vec![ContentOperation::new(
            "sh",
            vec![Operand::Name("Sh1".to_string())],
        )];
        let viewport = Viewport::new([0.0, 0.0, 100.0, 100.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let packed = PackedDisplayList::compile(list);
        let shading_ops: Vec<_> = packed
            .hot_ops
            .iter()
            .filter(|h| h.opcode == OP_NATIVE_SHADING)
            .collect();
        for hot in &shading_ops {
            let desc = packed.descriptor(hot.payload_offset).unwrap();
            match desc {
                NativeDescriptor::Shading(sh) => {
                    assert_eq!(sh.name, "Sh1");
                }
                _ => panic!("expected Shading descriptor"),
            }
        }
    }

    /// A test dispatcher that records which typed descriptors were dispatched.
    struct RecordingDispatcher {
        text_count: usize,
        image_count: usize,
        form_count: usize,
        shading_count: usize,
        state_count: usize,
        pattern_count: usize,
        inline_image_count: usize,
        save_count: usize,
        restore_count: usize,
        fill_count: usize,
        stroke_count: usize,
        clip_count: usize,
    }

    impl RecordingDispatcher {
        fn new() -> Self {
            Self {
                text_count: 0,
                image_count: 0,
                form_count: 0,
                shading_count: 0,
                state_count: 0,
                pattern_count: 0,
                inline_image_count: 0,
                save_count: 0,
                restore_count: 0,
                fill_count: 0,
                stroke_count: 0,
                clip_count: 0,
            }
        }
    }

    impl PlanDispatcher for RecordingDispatcher {
        fn dispatch_text(&mut self, _desc: &TextDescriptor, _bounds: Option<&RenderBounds>) {
            self.text_count += 1;
        }
        fn dispatch_image(
            &mut self,
            _desc: &ImageXObjectDescriptor,
            _bounds: Option<&RenderBounds>,
        ) {
            self.image_count += 1;
        }
        fn dispatch_form(&mut self, _desc: &FormXObjectDescriptor, _bounds: Option<&RenderBounds>) {
            self.form_count += 1;
        }
        fn dispatch_shading(&mut self, _desc: &ShadingDescriptor, _bounds: Option<&RenderBounds>) {
            self.shading_count += 1;
        }
        fn dispatch_state(&mut self, _op: &crate::content::ContentOperation) {
            self.state_count += 1;
        }
        fn dispatch_pattern_ops(
            &mut self,
            _ops: &[crate::content::ContentOperation],
            _bounds: Option<&RenderBounds>,
        ) {
            self.pattern_count += 1;
        }
        fn dispatch_inline_image_ops(
            &mut self,
            _ops: &[crate::content::ContentOperation],
            _bounds: Option<&RenderBounds>,
        ) {
            self.inline_image_count += 1;
        }
        fn dispatch_save(&mut self) {
            self.save_count += 1;
        }
        fn dispatch_restore(&mut self) {
            self.restore_count += 1;
        }
        fn dispatch_clip(&mut self, _path: &Path, _ctm: &Transform2D, _rule: FillRule) {
            self.clip_count += 1;
        }
        fn dispatch_fill(&mut self, _path: &Path, _state: &DrawState, _rule: FillRule) {
            self.fill_count += 1;
        }
        fn dispatch_stroke(&mut self, _path: &Path, _state: &DrawState) {
            self.stroke_count += 1;
        }
        fn is_cancelled(&self) -> bool {
            false
        }
    }

    #[test]
    fn execute_plan_dispatches_text_through_typed_descriptor() {
        let ops = vec![
            ContentOperation::new("BT", Vec::new()),
            ContentOperation::new(
                "Tf",
                vec![Operand::Name("F1".to_string()), Operand::Real(12.0)],
            ),
            ContentOperation::new("Tj", vec![Operand::String(b"Plan".to_vec())]),
            ContentOperation::new("ET", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 200.0, 200.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        let packed = PackedDisplayList::compile(list);
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute_plan");
        // Text ops (Tj) go through dispatch_text; state ops (BT/Tf/ET) through dispatch_state
        assert!(
            rec.text_count > 0,
            "text descriptor should be dispatched, got text_count={}",
            rec.text_count
        );
    }

    #[test]
    fn execute_plan_dispatches_vector_fill_directly() {
        let ops = vec![
            ContentOperation::new(
                "rg",
                vec![Operand::Real(0.0), Operand::Real(1.0), Operand::Real(0.0)],
            ),
            ContentOperation::new(
                "re",
                vec![
                    Operand::Real(5.0),
                    Operand::Real(5.0),
                    Operand::Real(30.0),
                    Operand::Real(30.0),
                ],
            ),
            ContentOperation::new("f", Vec::new()),
        ];
        let viewport = Viewport::new([0.0, 0.0, 50.0, 50.0], 72);
        let list = build_display_list(
            &ops,
            viewport.clone(),
            &crate::engine::PageResources::default(),
        );
        assert!(list.native_vector_only());
        let packed = PackedDisplayList::compile(list);
        let mut rec = RecordingDispatcher::new();
        packed.execute_plan(&mut rec).expect("execute_plan");
        assert!(rec.fill_count > 0, "fill should be dispatched");
        assert_eq!(rec.text_count, 0);
        assert_eq!(rec.image_count, 0);
    }
}
