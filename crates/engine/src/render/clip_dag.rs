//! Persistent interned clip-state DAG for the raster/retained renderer.
//!
//! PDF graphics state save/restore (`q`/`Q`) frequently pushes and pops
//! identical clip states. The naive approach clones the full `ClipMask` on
//! every save, allocating dense byte planes or run-length vectors repeatedly
//! for identical geometry.
//!
//! This module provides a structural DAG of clip states where:
//! - Common states (`Full`, `Empty`, `Rectangle`) are flyweights.
//! - Binary path-derived clips are preserved as sparse spans or RLE masks.
//! - Partial-coverage clips are preserved as dense alpha masks.
//! - Intersections form composite DAG edges, enabling structural sharing on save/restore.
//! - `Arc`-based nodes allow zero-copy push/pop on the graphics state stack.
//!
//! The DAG integrates into the active renderer path: `RenderState` uses
//! `ClipDag` to intern clip states and the clip stack holds `Arc<ClipNode>`
//! handles instead of owned `Option<ClipMask>` clones.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::buffer::{AlphaMask, ClipMask};

type ClipRow = Vec<(i32, i32)>;
type ClipRows = Vec<ClipRow>;
type ClipRowsRef<'a> = &'a [ClipRow];
type ClipDimensions = (u32, u32);
type MaterializedClipMaskCache = Mutex<HashMap<ClipDimensions, Arc<ClipMask>>>;

pub const DEFAULT_MAX_CLIP_DAG_NODES: usize = 4096;

/// Identity salts for a clip DAG scoped to one render contract.
///
/// The default renderer path constructs a fresh DAG per `RenderState`, so the
/// default zero identity is sufficient there. Contract-aware callers can use a
/// non-default scope to keep structurally identical clips separated across
/// document revisions, render contracts, or tile-local outputs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ClipIdentityScope {
    pub revision_identity: u64,
    pub render_contract_identity: u64,
    pub tile_identity: u64,
}

impl ClipIdentityScope {
    pub fn stable_id_for(self, state: &ClipState) -> u64 {
        let mut hash = state.fingerprint();
        for part in [
            self.revision_identity,
            self.render_contract_identity,
            self.tile_identity,
        ] {
            for byte in part.to_le_bytes() {
                ClipState::mix_byte(&mut hash, byte);
            }
        }
        hash
    }
}

/// The operation that produced a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipOperation {
    Root,
    Rectangle,
    SparseSpans,
    RleMask,
    DenseMask,
    Intersect,
}

/// Composite operation stored by a DAG edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipCompositeOp {
    Intersect,
}

/// Exclusive visible pixel bounds for a clip node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClipBounds {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

/// Source-space window used when fusing bounded alpha/image masks into the DAG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClipAlphaFusionWindow {
    pub source_width: u32,
    pub source_height: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A content-addressed clip-state node in the persistent DAG.
///
/// Variants are ordered by specificity: `Full` is the identity for intersection,
/// `Empty` is the annihilator.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClipState {
    /// Every pixel is visible (no clip installed). Identity for intersection.
    Full,
    /// No pixel is visible. Annihilator for intersection.
    Empty,
    /// Axis-aligned integer rectangle clip (very common in PDF).
    Rectangle { x: i32, y: i32, w: i32, h: i32 },
    /// Low-cardinality binary spans, stored by row without a dense byte plane.
    /// The `fingerprint` is a content hash for deduplication.
    SparseSpans {
        fingerprint: u64,
        width: u32,
        height: u32,
        rows: Arc<ClipRows>,
    },
    /// General binary run-length mask.
    RleMask {
        fingerprint: u64,
        width: u32,
        height: u32,
        runs: Arc<ClipRows>,
    },
    /// Partial-coverage clip mask that must preserve alpha coverage bytes.
    DenseMask {
        fingerprint: u64,
        width: u32,
        height: u32,
        bytes: Arc<Vec<u8>>,
    },
    /// Composite of two clip states (structural sharing).
    Composite {
        op: ClipCompositeOp,
        lhs: Arc<ClipNode>,
        rhs: Arc<ClipNode>,
    },
}

/// A node in the clip DAG: a clip state plus its cached materialization.
#[derive(Debug)]
pub struct ClipNode {
    pub state: ClipState,
    pub stable_id: u64,
    pub parent: Option<Arc<ClipNode>>,
    pub operation: ClipOperation,
    pub bounds: Option<ClipBounds>,
    pub revision_identity: u64,
    pub render_contract_identity: u64,
    pub tile_identity: u64,
    pub memory_charge: usize,
    /// Lazily materialized ClipMasks keyed by destination dimensions.
    materialized: std::sync::OnceLock<MaterializedClipMaskCache>,
}

impl PartialEq for ClipNode {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
            && self.revision_identity == other.revision_identity
            && self.render_contract_identity == other.render_contract_identity
            && self.tile_identity == other.tile_identity
    }
}

impl Eq for ClipNode {}

impl std::hash::Hash for ClipNode {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.state.hash(hasher);
        self.revision_identity.hash(hasher);
        self.render_contract_identity.hash(hasher);
        self.tile_identity.hash(hasher);
    }
}

impl ClipNode {
    /// Create a new node with a given state.
    pub fn new(state: ClipState) -> Self {
        Self::new_scoped(state, ClipIdentityScope::default())
    }

    /// Create a new node with a render identity scope.
    pub fn new_scoped(state: ClipState, identity: ClipIdentityScope) -> Self {
        let parent = state.parent();
        let operation = state.operation();
        let bounds = state.bounds();
        let memory_charge = state.memory_charge();
        let stable_id = identity.stable_id_for(&state);
        Self {
            state,
            stable_id,
            parent,
            operation,
            bounds,
            revision_identity: identity.revision_identity,
            render_contract_identity: identity.render_contract_identity,
            tile_identity: identity.tile_identity,
            memory_charge,
            materialized: std::sync::OnceLock::new(),
        }
    }

    /// Get or materialize the concrete `ClipMask` for this node at the given
    /// buffer dimensions.
    pub fn materialize(&self, width: u32, height: u32) -> Arc<ClipMask> {
        let cache = self.materialized.get_or_init(|| Mutex::new(HashMap::new()));
        let mut cache = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(mask) = cache.get(&(width, height)) {
            return Arc::clone(mask);
        }
        let mask = Arc::new(self.state.to_clip_mask(width, height));
        cache.insert((width, height), Arc::clone(&mask));
        mask
    }

    /// Materialize only a source-space pixel window into destination-local
    /// coordinates. This is used for tile/group clips so a full-page clip mask
    /// does not have to be cloned before cropping.
    pub fn materialize_window(
        &self,
        source_width: u32,
        source_height: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Option<ClipMask> {
        self.state
            .to_clip_mask_window(source_width, source_height, x, y, width, height)
    }

    /// Check if the materialized mask is already available without computing it.
    pub fn is_materialized(&self) -> bool {
        match self.materialized.get() {
            Some(cache) => !cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            None => false,
        }
    }

    /// Return approximate memory bytes held by this node (excluding Arc overhead).
    pub fn approximate_bytes(&self) -> usize {
        let mat_bytes = self
            .materialized
            .get()
            .map(|cache| {
                cache
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .values()
                    .map(|m| std::mem::size_of::<ClipMask>() + m.width as usize * m.height as usize)
                    .sum::<usize>()
            })
            .unwrap_or(0);
        std::mem::size_of::<Self>() + self.memory_charge + mat_bytes
    }
}

impl ClipBounds {
    fn from_xywh(x: i32, y: i32, w: i32, h: i32) -> Option<Self> {
        if w <= 0 || h <= 0 {
            return None;
        }
        let x1 = x.checked_add(w)?;
        let y1 = y.checked_add(h)?;
        if x1 <= x || y1 <= y {
            return None;
        }
        Some(Self {
            x0: x,
            y0: y,
            x1,
            y1,
        })
    }

    fn intersect(self, other: Self) -> Option<Self> {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        let x1 = self.x1.min(other.x1);
        let y1 = self.y1.min(other.y1);
        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some(Self { x0, y0, x1, y1 })
        }
    }
}

impl ClipState {
    /// Materialize a concrete `ClipMask` from this DAG state.
    pub fn to_clip_mask(&self, width: u32, height: u32) -> ClipMask {
        self.to_clip_mask_uncached(width, height)
    }

    fn to_clip_mask_uncached(&self, width: u32, height: u32) -> ClipMask {
        match self {
            ClipState::Full => ClipMask::all_visible(width, height),
            ClipState::Empty => ClipMask::empty(width, height),
            ClipState::Rectangle { x, y, w, h } => {
                ClipMask::from_visible_rect(width, height, *x, *y, *w, *h)
            }
            ClipState::SparseSpans { rows, .. } => {
                ClipMask::from_visible_runs(width, height, (**rows).clone())
            }
            ClipState::RleMask { runs, .. } => {
                ClipMask::from_visible_runs(width, height, (**runs).clone())
            }
            ClipState::DenseMask {
                width: mask_width,
                height: mask_height,
                bytes,
                ..
            } if *mask_width == width && *mask_height == height => {
                ClipMask::from_alpha_bytes(width, height, (**bytes).clone())
            }
            ClipState::DenseMask {
                width: mask_width,
                height: mask_height,
                bytes,
                ..
            } => Self::dense_to_dimensions(bytes, *mask_width, *mask_height, width, height)
                .unwrap_or_else(|| ClipMask::empty(width, height)),
            ClipState::Composite { op, lhs, rhs } => match op {
                ClipCompositeOp::Intersect => {
                    let mut left = lhs.state.to_clip_mask_uncached(width, height);
                    let right = rhs.state.to_clip_mask_uncached(width, height);
                    left.intersect(&right);
                    left
                }
            },
        }
    }

    pub fn to_clip_mask_window(
        &self,
        source_width: u32,
        source_height: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Option<ClipMask> {
        if width == 0 || height == 0 {
            return None;
        }
        let end_x = x.checked_add(width)?;
        let end_y = y.checked_add(height)?;
        if end_x > source_width || end_y > source_height {
            return None;
        }
        match self {
            ClipState::Full => Some(ClipMask::all_visible(width, height)),
            ClipState::Empty => Some(ClipMask::empty(width, height)),
            ClipState::Rectangle {
                x: rect_x,
                y: rect_y,
                w,
                h,
            } => {
                let rect = ClipBounds::from_xywh(*rect_x, *rect_y, *w, *h)?;
                let window = ClipBounds {
                    x0: x as i32,
                    y0: y as i32,
                    x1: end_x as i32,
                    y1: end_y as i32,
                };
                let Some(overlap) = rect.intersect(window) else {
                    return Some(ClipMask::empty(width, height));
                };
                Some(ClipMask::from_visible_rect(
                    width,
                    height,
                    overlap.x0 - window.x0,
                    overlap.y0 - window.y0,
                    overlap.x1 - overlap.x0,
                    overlap.y1 - overlap.y0,
                ))
            }
            ClipState::SparseSpans {
                width: mask_width,
                height: mask_height,
                rows,
                ..
            }
            | ClipState::RleMask {
                width: mask_width,
                height: mask_height,
                runs: rows,
                ..
            } if *mask_width == source_width && *mask_height == source_height => {
                Some(Self::rows_window(rows, x, y, width, height))
            }
            ClipState::DenseMask {
                width: mask_width,
                height: mask_height,
                bytes,
                ..
            } if *mask_width == source_width && *mask_height == source_height => {
                Self::dense_window(bytes, source_width, x, y, width, height)
            }
            ClipState::Composite { op, lhs, rhs } => match op {
                ClipCompositeOp::Intersect => {
                    let mut left =
                        lhs.materialize_window(source_width, source_height, x, y, width, height)?;
                    if left.is_empty() {
                        return Some(left);
                    }
                    if left.is_all_visible() {
                        return rhs.materialize_window(
                            source_width,
                            source_height,
                            x,
                            y,
                            width,
                            height,
                        );
                    }
                    let right =
                        rhs.materialize_window(source_width, source_height, x, y, width, height)?;
                    if right.is_empty() {
                        return Some(right);
                    }
                    if right.is_all_visible() {
                        return Some(left);
                    }
                    left.intersect(&right);
                    Some(left)
                }
            },
            _ => self
                .to_clip_mask(source_width, source_height)
                .copy_rect_to_new_mask(x, y, width, height),
        }
    }

    /// Classify a `ClipMask` into its structural `ClipState` representation.
    ///
    /// This inspects the mask's structural hints to determine the cheapest DAG
    /// node that represents it. Binary clips stay as spans/RLE rows; partial
    /// coverage clips keep their alpha bytes so antialiased coverage survives
    /// save/restore without relying on an already-materialized cache entry.
    pub fn from_clip_mask(mask: &ClipMask) -> Self {
        if mask.is_all_visible() {
            return ClipState::Full;
        }
        if mask.is_empty() {
            return ClipState::Empty;
        }
        if let Some((x0, y0, x1, y1)) = mask.visible_bounds() {
            let w = x1 - x0;
            let h = y1 - y0;
            if Self::mask_is_solid_rect(mask, x0, y0, x1, y1) {
                return ClipState::Rectangle { x: x0, y: y0, w, h };
            }
        }
        if mask.has_partial_coverage() {
            let bytes = Self::extract_alpha_bytes(mask);
            let fingerprint = Self::fingerprint_bytes(&bytes, mask.width, mask.height);
            return ClipState::DenseMask {
                fingerprint,
                width: mask.width,
                height: mask.height,
                bytes: Arc::new(bytes),
            };
        }

        let rows = Self::extract_runs(mask);
        let fingerprint = Self::fingerprint_runs(&rows, mask.width, mask.height);
        if Self::use_sparse_spans(&rows, mask.width, mask.height) {
            ClipState::SparseSpans {
                fingerprint,
                width: mask.width,
                height: mask.height,
                rows: Arc::new(rows),
            }
        } else {
            ClipState::RleMask {
                fingerprint,
                width: mask.width,
                height: mask.height,
                runs: Arc::new(rows),
            }
        }
    }

    fn parent(&self) -> Option<Arc<ClipNode>> {
        match self {
            ClipState::Composite { lhs, .. } => Some(Arc::clone(lhs)),
            _ => None,
        }
    }

    fn operation(&self) -> ClipOperation {
        match self {
            ClipState::Full | ClipState::Empty => ClipOperation::Root,
            ClipState::Rectangle { .. } => ClipOperation::Rectangle,
            ClipState::SparseSpans { .. } => ClipOperation::SparseSpans,
            ClipState::RleMask { .. } => ClipOperation::RleMask,
            ClipState::DenseMask { .. } => ClipOperation::DenseMask,
            ClipState::Composite {
                op: ClipCompositeOp::Intersect,
                ..
            } => ClipOperation::Intersect,
        }
    }

    fn bounds(&self) -> Option<ClipBounds> {
        match self {
            ClipState::Full => None,
            ClipState::Empty => None,
            ClipState::Rectangle { x, y, w, h } => ClipBounds::from_xywh(*x, *y, *w, *h),
            ClipState::SparseSpans { rows, .. } => Self::rows_bounds(rows),
            ClipState::RleMask { runs, .. } => Self::rows_bounds(runs),
            ClipState::DenseMask {
                width,
                height,
                bytes,
                ..
            } => Self::dense_bounds(bytes, *width, *height),
            ClipState::Composite { lhs, rhs, .. } => match (lhs.bounds, rhs.bounds) {
                (Some(left), Some(right)) => left.intersect(right),
                (Some(bounds), None) | (None, Some(bounds)) => Some(bounds),
                (None, None) => None,
            },
        }
    }

    fn memory_charge(&self) -> usize {
        match self {
            ClipState::Full | ClipState::Empty => 0,
            ClipState::Rectangle { .. } => std::mem::size_of::<[i32; 4]>(),
            ClipState::SparseSpans { rows, .. } => Self::rows_memory_charge(rows),
            ClipState::RleMask { runs, .. } => Self::rows_memory_charge(runs),
            ClipState::DenseMask { bytes, .. } => bytes.len(),
            ClipState::Composite { .. } => {
                std::mem::size_of::<ClipCompositeOp>() + 2 * std::mem::size_of::<Arc<ClipNode>>()
            }
        }
    }

    /// Check if a mask is a solid filled rectangle between the given bounds.
    fn mask_is_solid_rect(mask: &ClipMask, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
        if mask.has_partial_coverage() {
            return false;
        }
        for y in 0..mask.height as i32 {
            let mut row_visible = 0i32;
            mask.for_each_visible_run(y, mask.width as i32, |start, end| {
                row_visible += end - start;
            });
            if y >= y0 && y < y1 {
                let expected = x1 - x0;
                if row_visible != expected {
                    return false;
                }
                let mut span_ok = false;
                mask.for_each_visible_run_in_span(y, x0, x1, |start, end| {
                    if start == x0 && end == x1 {
                        span_ok = true;
                    }
                });
                if !span_ok {
                    return false;
                }
            } else if row_visible != 0 {
                return false;
            }
        }
        true
    }

    /// Extract run-length rows from a ClipMask.
    fn extract_runs(mask: &ClipMask) -> Vec<Vec<(i32, i32)>> {
        let mut rows = Vec::with_capacity(mask.height as usize);
        for y in 0..mask.height as i32 {
            let mut row = Vec::new();
            mask.for_each_visible_run(y, mask.width as i32, |start, end| {
                row.push((start, end));
            });
            rows.push(row);
        }
        rows
    }

    fn extract_alpha_bytes(mask: &ClipMask) -> Vec<u8> {
        let len = (mask.width as usize)
            .checked_mul(mask.height as usize)
            .unwrap_or(0);
        let mut bytes = Vec::with_capacity(len);
        for y in 0..mask.height as i32 {
            for x in 0..mask.width as i32 {
                bytes.push(mask.opacity_byte(x, y));
            }
        }
        bytes
    }

    fn use_sparse_spans(rows: &[Vec<(i32, i32)>], width: u32, height: u32) -> bool {
        let run_count = rows.iter().map(Vec::len).sum::<usize>();
        let non_empty_rows = rows.iter().filter(|row| !row.is_empty()).count();
        let visible_pixels = rows
            .iter()
            .flatten()
            .map(|(start, end)| end.saturating_sub(*start).max(0) as usize)
            .sum::<usize>();
        let total_pixels = (width as usize).saturating_mul(height as usize).max(1);
        run_count <= 32
            || visible_pixels.saturating_mul(8) <= total_pixels
            || non_empty_rows.saturating_mul(8) <= height as usize
    }

    fn binary_rows(&self) -> Option<(u32, u32, ClipRowsRef<'_>)> {
        match self {
            ClipState::SparseSpans {
                width,
                height,
                rows,
                ..
            } => Some((*width, *height, rows)),
            ClipState::RleMask {
                width,
                height,
                runs,
                ..
            } => Some((*width, *height, runs)),
            _ => None,
        }
    }

    fn intersect_structural(lhs: &Self, rhs: &Self) -> Option<Self> {
        match (lhs, rhs) {
            (ClipState::DenseMask { .. }, ClipState::Rectangle { x, y, w, h }) => {
                Self::intersect_dense_with_rect_state(lhs, *x, *y, *w, *h)
            }
            (ClipState::Rectangle { x, y, w, h }, ClipState::DenseMask { .. }) => {
                Self::intersect_dense_with_rect_state(rhs, *x, *y, *w, *h)
            }
            (ClipState::DenseMask { .. }, ClipState::DenseMask { .. }) => {
                Self::intersect_dense_states(lhs, rhs)
            }
            (
                ClipState::DenseMask { .. },
                ClipState::SparseSpans { .. } | ClipState::RleMask { .. },
            ) => Self::intersect_dense_with_rows_state(lhs, rhs),
            (
                ClipState::SparseSpans { .. } | ClipState::RleMask { .. },
                ClipState::DenseMask { .. },
            ) => Self::intersect_dense_with_rows_state(rhs, lhs),
            (
                ClipState::Rectangle { x, y, w, h },
                ClipState::SparseSpans { .. } | ClipState::RleMask { .. },
            ) => Self::intersect_rows_with_rect_state(rhs, *x, *y, *w, *h),
            (
                ClipState::SparseSpans { .. } | ClipState::RleMask { .. },
                ClipState::Rectangle { x, y, w, h },
            ) => Self::intersect_rows_with_rect_state(lhs, *x, *y, *w, *h),
            (
                ClipState::SparseSpans { .. } | ClipState::RleMask { .. },
                ClipState::SparseSpans { .. } | ClipState::RleMask { .. },
            ) => Self::intersect_binary_row_states(lhs, rhs),
            _ => None,
        }
    }

    fn fused_alpha_mask_window(
        alpha_mask: Option<&AlphaMask>,
        clip: Option<&ClipNode>,
        window: ClipAlphaFusionWindow,
    ) -> Option<Self> {
        let ClipAlphaFusionWindow {
            source_width,
            source_height,
            x,
            y,
            width,
            height,
        } = window;
        if width == 0 || height == 0 {
            return None;
        }
        let len = (width as usize).checked_mul(height as usize)?;
        let clip_window = match clip {
            Some(node) if node.state == ClipState::Empty => return Some(ClipState::Empty),
            Some(node) if node.state == ClipState::Full => None,
            Some(node) => {
                Some(node.materialize_window(source_width, source_height, x, y, width, height)?)
            }
            None => None,
        };
        let clip_window = clip_window.as_ref().filter(|mask| !mask.is_all_visible());
        if alpha_mask.is_none() && clip_window.is_none() {
            return None;
        }

        let mut bytes = vec![255; len];
        let width_usize = width as usize;
        for local_y in 0..height as i32 {
            let dest_y = y as i32 + local_y;
            let row_start = (local_y as usize).checked_mul(width_usize)?;
            for local_x in 0..width as i32 {
                let dest_x = x as i32 + local_x;
                let mut alpha = alpha_mask.map_or(255, |mask| mask.get_byte(dest_x, dest_y));
                if let Some(clip) = clip_window {
                    alpha = div255_round_u16(
                        u16::from(alpha) * u16::from(clip.opacity_byte(local_x, local_y)),
                    ) as u8;
                }
                bytes[row_start + local_x as usize] = alpha;
            }
        }
        Some(Self::state_from_alpha_bytes(width, height, bytes))
    }

    fn dense_bytes(&self) -> Option<(u32, u32, &[u8])> {
        match self {
            ClipState::DenseMask {
                width,
                height,
                bytes,
                ..
            } => Some((*width, *height, bytes.as_slice())),
            _ => None,
        }
    }

    fn intersect_dense_states(lhs: &Self, rhs: &Self) -> Option<Self> {
        let (width, height, left_bytes) = lhs.dense_bytes()?;
        let (right_width, right_height, right_bytes) = rhs.dense_bytes()?;
        if width != right_width || height != right_height {
            return None;
        }
        let expected_len = (width as usize).checked_mul(height as usize)?;
        if left_bytes.len() != expected_len || right_bytes.len() != expected_len {
            return None;
        }
        let bytes = left_bytes
            .iter()
            .zip(right_bytes.iter())
            .map(|(left, right)| (*left).min(*right))
            .collect::<Vec<_>>();
        Some(Self::state_from_alpha_bytes(width, height, bytes))
    }

    fn intersect_dense_with_rect_state(
        state: &Self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Option<Self> {
        let (width, height, bytes) = state.dense_bytes()?;
        let expected_len = (width as usize).checked_mul(height as usize)?;
        if bytes.len() != expected_len {
            return None;
        }
        let Some(rect) = ClipBounds::from_xywh(x, y, w, h) else {
            return Some(ClipState::Empty);
        };
        let x0 = rect.x0.max(0).min(width as i32);
        let y0 = rect.y0.max(0).min(height as i32);
        let x1 = rect.x1.max(0).min(width as i32);
        let y1 = rect.y1.max(0).min(height as i32);
        if x1 <= x0 || y1 <= y0 {
            return Some(ClipState::Empty);
        }

        let width_usize = width as usize;
        let mut out = vec![0; expected_len];
        for row in y0 as usize..y1 as usize {
            let start = row.checked_mul(width_usize)?.checked_add(x0 as usize)?;
            let end = row.checked_mul(width_usize)?.checked_add(x1 as usize)?;
            out.get_mut(start..end)?
                .copy_from_slice(bytes.get(start..end)?);
        }
        Some(Self::state_from_alpha_bytes(width, height, out))
    }

    fn intersect_dense_with_rows_state(dense: &Self, rows_state: &Self) -> Option<Self> {
        let (width, height, bytes) = dense.dense_bytes()?;
        let (row_width, row_height, rows) = rows_state.binary_rows()?;
        if width != row_width || height != row_height {
            return None;
        }
        let expected_len = (width as usize).checked_mul(height as usize)?;
        if bytes.len() != expected_len {
            return None;
        }

        let width_usize = width as usize;
        let mut out = vec![0; expected_len];
        for y in 0..height as usize {
            let row_start = y.checked_mul(width_usize)?;
            let Some(row) = rows.get(y) else {
                continue;
            };
            for (start, end) in row {
                let start = (*start).max(0).min(width as i32) as usize;
                let end = (*end).max(0).min(width as i32) as usize;
                if end <= start {
                    continue;
                }
                let copy_start = row_start.checked_add(start)?;
                let copy_end = row_start.checked_add(end)?;
                out.get_mut(copy_start..copy_end)?
                    .copy_from_slice(bytes.get(copy_start..copy_end)?);
            }
        }
        Some(Self::state_from_alpha_bytes(width, height, out))
    }

    fn intersect_binary_row_states(lhs: &Self, rhs: &Self) -> Option<Self> {
        let (left_width, left_height, left_rows) = lhs.binary_rows()?;
        let (right_width, right_height, right_rows) = rhs.binary_rows()?;
        if left_width != right_width || left_height != right_height {
            return None;
        }
        let rows = Self::intersect_rows(left_rows, right_rows, left_height as usize);
        Some(Self::state_from_binary_rows(left_width, left_height, rows))
    }

    fn intersect_rows_with_rect_state(
        state: &Self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> Option<Self> {
        let (width, height, rows) = state.binary_rows()?;
        let Some(rect) = ClipBounds::from_xywh(x, y, w, h) else {
            return Some(ClipState::Empty);
        };
        let x0 = rect.x0.max(0).min(width as i32);
        let y0 = rect.y0.max(0).min(height as i32);
        let x1 = rect.x1.max(0).min(width as i32);
        let y1 = rect.y1.max(0).min(height as i32);
        if x1 <= x0 || y1 <= y0 {
            return Some(ClipState::Empty);
        }

        let mut out = vec![Vec::new(); height as usize];
        for (y_index, out_row) in out
            .iter_mut()
            .enumerate()
            .take(y1 as usize)
            .skip(y0 as usize)
        {
            let Some(row) = rows.get(y_index) else {
                continue;
            };
            for (start, end) in row {
                let start = (*start).max(x0);
                let end = (*end).min(x1);
                if end > start {
                    out_row.push((start, end));
                }
            }
        }
        Some(Self::state_from_binary_rows(width, height, out))
    }

    fn state_from_alpha_bytes(width: u32, height: u32, bytes: Vec<u8>) -> Self {
        let Some(expected_len) = (width as usize).checked_mul(height as usize) else {
            return ClipState::Empty;
        };
        if expected_len == 0 || bytes.len() != expected_len {
            return ClipState::Empty;
        }

        let mut all_empty = true;
        let mut all_full = true;
        let mut partial = false;
        for byte in &bytes {
            all_empty &= *byte == 0;
            all_full &= *byte == 255;
            partial |= *byte != 0 && *byte != 255;
        }
        if all_empty {
            return ClipState::Empty;
        }
        if all_full {
            return ClipState::Full;
        }
        if !partial {
            return Self::state_from_binary_rows(
                width,
                height,
                Self::rows_from_alpha_bytes(width, height, &bytes),
            );
        }

        let fingerprint = Self::fingerprint_bytes(&bytes, width, height);
        ClipState::DenseMask {
            fingerprint,
            width,
            height,
            bytes: Arc::new(bytes),
        }
    }

    fn rows_from_alpha_bytes(width: u32, height: u32, bytes: &[u8]) -> ClipRows {
        let width_usize = width as usize;
        let mut rows = Vec::with_capacity(height as usize);
        for y in 0..height as usize {
            let row_start = y.saturating_mul(width_usize);
            let row_end = row_start.saturating_add(width_usize);
            let Some(row_bytes) = bytes.get(row_start..row_end) else {
                rows.push(Vec::new());
                continue;
            };
            let mut row = Vec::new();
            let mut run_start = None;
            for (x, byte) in row_bytes.iter().enumerate() {
                if *byte > 0 {
                    if run_start.is_none() {
                        run_start = Some(x as i32);
                    }
                } else if let Some(start) = run_start.take() {
                    row.push((start, x as i32));
                }
            }
            if let Some(start) = run_start {
                row.push((start, width as i32));
            }
            rows.push(row);
        }
        rows
    }

    fn intersect_rows(
        left_rows: ClipRowsRef<'_>,
        right_rows: ClipRowsRef<'_>,
        height: usize,
    ) -> ClipRows {
        let mut rows = vec![Vec::new(); height];
        for (y, out) in rows.iter_mut().enumerate() {
            let left = left_rows.get(y).map(Vec::as_slice).unwrap_or(&[]);
            let right = right_rows.get(y).map(Vec::as_slice).unwrap_or(&[]);
            *out = Self::intersect_run_row(left, right);
        }
        rows
    }

    fn intersect_run_row(left: &[(i32, i32)], right: &[(i32, i32)]) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        let mut l = 0usize;
        let mut r = 0usize;
        while let (Some(&(ls, le)), Some(&(rs, re))) = (left.get(l), right.get(r)) {
            let start = ls.max(rs);
            let end = le.min(re);
            if end > start {
                out.push((start, end));
            }
            if le < re {
                l += 1;
            } else {
                r += 1;
            }
        }
        out
    }

    fn rows_window(rows: ClipRowsRef<'_>, x: u32, y: u32, width: u32, height: u32) -> ClipMask {
        let x0 = x as i32;
        let x1 = x.saturating_add(width) as i32;
        let mut out = vec![Vec::new(); height as usize];
        for (local_y, out_row) in out.iter_mut().enumerate() {
            let source_y = y as usize + local_y;
            let Some(row) = rows.get(source_y) else {
                continue;
            };
            for (start, end) in row {
                let start = (*start).max(x0);
                let end = (*end).min(x1);
                if end > start {
                    out_row.push((start - x0, end - x0));
                }
            }
        }
        ClipMask::from_visible_runs(width, height, out)
    }

    fn dense_window(
        bytes: &[u8],
        source_width: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Option<ClipMask> {
        let source_stride = source_width as usize;
        let x = x as usize;
        let y = y as usize;
        let width_usize = width as usize;
        let height_usize = height as usize;
        let mut out = Vec::with_capacity(width_usize.checked_mul(height_usize)?);
        for row in 0..height_usize {
            let source_start = y
                .checked_add(row)?
                .checked_mul(source_stride)?
                .checked_add(x)?;
            let source_end = source_start.checked_add(width_usize)?;
            out.extend_from_slice(bytes.get(source_start..source_end)?);
        }
        Some(ClipMask::from_alpha_bytes(width, height, out))
    }

    fn dense_to_dimensions(
        bytes: &[u8],
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Option<ClipMask> {
        if width == 0 || height == 0 {
            return Some(ClipMask::empty(width, height));
        }
        let source_len = (source_width as usize).checked_mul(source_height as usize)?;
        if bytes.len() != source_len {
            return None;
        }
        let len = (width as usize).checked_mul(height as usize)?;
        let mut out = vec![0; len];
        let copy_width = source_width.min(width) as usize;
        let copy_height = source_height.min(height) as usize;
        let src_width = source_width as usize;
        let dst_width = width as usize;
        for row in 0..copy_height {
            let src_start = row.checked_mul(src_width)?;
            let dst_start = row.checked_mul(dst_width)?;
            let src_end = src_start.checked_add(copy_width)?;
            let dst_end = dst_start.checked_add(copy_width)?;
            out.get_mut(dst_start..dst_end)?
                .copy_from_slice(bytes.get(src_start..src_end)?);
        }
        Some(ClipMask::from_alpha_bytes(width, height, out))
    }

    fn state_from_binary_rows(width: u32, height: u32, mut rows: ClipRows) -> Self {
        rows.resize_with(height as usize, Vec::new);
        rows.truncate(height as usize);

        if rows.iter().all(Vec::is_empty) {
            return ClipState::Empty;
        }
        if width > 0
            && rows
                .iter()
                .all(|row| row.len() == 1 && row[0].0 <= 0 && row[0].1 >= width as i32)
        {
            return ClipState::Full;
        }
        if let Some(bounds) = Self::rows_bounds(&rows) {
            if Self::rows_are_solid_rect(&rows, bounds) {
                return ClipState::Rectangle {
                    x: bounds.x0,
                    y: bounds.y0,
                    w: bounds.x1 - bounds.x0,
                    h: bounds.y1 - bounds.y0,
                };
            }
        }

        let fingerprint = Self::fingerprint_runs(&rows, width, height);
        if Self::use_sparse_spans(&rows, width, height) {
            ClipState::SparseSpans {
                fingerprint,
                width,
                height,
                rows: Arc::new(rows),
            }
        } else {
            ClipState::RleMask {
                fingerprint,
                width,
                height,
                runs: Arc::new(rows),
            }
        }
    }

    fn rows_are_solid_rect(rows: ClipRowsRef<'_>, bounds: ClipBounds) -> bool {
        rows.iter().enumerate().all(|(y, row)| {
            let y = y as i32;
            if y >= bounds.y0 && y < bounds.y1 {
                row.as_slice() == [(bounds.x0, bounds.x1)]
            } else {
                row.is_empty()
            }
        })
    }

    fn rows_bounds(rows: ClipRowsRef<'_>) -> Option<ClipBounds> {
        let mut x0 = i32::MAX;
        let mut y0 = i32::MAX;
        let mut x1 = i32::MIN;
        let mut y1 = i32::MIN;
        for (y, row) in rows.iter().enumerate() {
            for (start, end) in row {
                if end <= start {
                    continue;
                }
                x0 = x0.min(*start);
                y0 = y0.min(y as i32);
                x1 = x1.max(*end);
                y1 = y1.max(y as i32 + 1);
            }
        }
        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some(ClipBounds { x0, y0, x1, y1 })
        }
    }

    fn dense_bounds(bytes: &[u8], width: u32, height: u32) -> Option<ClipBounds> {
        let mut x0 = i32::MAX;
        let mut y0 = i32::MAX;
        let mut x1 = i32::MIN;
        let mut y1 = i32::MIN;
        let width_usize = width as usize;
        for y in 0..height as usize {
            let row_start = y.saturating_mul(width_usize);
            let row_end = row_start.saturating_add(width_usize);
            let Some(row) = bytes.get(row_start..row_end) else {
                break;
            };
            for (x, value) in row.iter().enumerate() {
                if *value == 0 {
                    continue;
                }
                x0 = x0.min(x as i32);
                y0 = y0.min(y as i32);
                x1 = x1.max(x as i32 + 1);
                y1 = y1.max(y as i32 + 1);
            }
        }
        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some(ClipBounds { x0, y0, x1, y1 })
        }
    }

    fn rows_memory_charge(rows: ClipRowsRef<'_>) -> usize {
        rows.iter()
            .map(|row| std::mem::size_of::<Vec<(i32, i32)>>() + row.len() * 8)
            .sum::<usize>()
            + std::mem::size_of::<Arc<Vec<Vec<(i32, i32)>>>>()
    }

    fn fingerprint_with_tag(tag: u8, parts: impl IntoIterator<Item = u64>) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        Self::mix_byte(&mut hash, tag);
        for part in parts {
            for byte in part.to_le_bytes() {
                Self::mix_byte(&mut hash, byte);
            }
        }
        hash
    }

    #[inline]
    fn mix_byte(hash: &mut u64, byte: u8) {
        *hash ^= byte as u64;
        *hash = hash.wrapping_mul(0x100000001b3);
    }

    /// FNV-1a content hash over the run structure for deduplication.
    fn fingerprint_runs(runs: &[Vec<(i32, i32)>], width: u32, height: u32) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in width.to_le_bytes() {
            Self::mix_byte(&mut hash, b);
        }
        for b in height.to_le_bytes() {
            Self::mix_byte(&mut hash, b);
        }
        for row in runs {
            for (start, end) in row {
                for b in start.to_le_bytes() {
                    Self::mix_byte(&mut hash, b);
                }
                for b in end.to_le_bytes() {
                    Self::mix_byte(&mut hash, b);
                }
            }
            Self::mix_byte(&mut hash, 0xFF);
        }
        hash
    }

    fn fingerprint_bytes(bytes: &[u8], width: u32, height: u32) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in width.to_le_bytes() {
            Self::mix_byte(&mut hash, b);
        }
        for b in height.to_le_bytes() {
            Self::mix_byte(&mut hash, b);
        }
        for b in bytes {
            Self::mix_byte(&mut hash, *b);
        }
        hash
    }

    /// Return a structural fingerprint for this state (for interner lookup).
    pub fn fingerprint(&self) -> u64 {
        match self {
            ClipState::Full => 0x0000_0000_0000_0001,
            ClipState::Empty => 0x0000_0000_0000_0002,
            ClipState::Rectangle { x, y, w, h } => {
                Self::fingerprint_with_tag(0x03, [*x as u64, *y as u64, *w as u64, *h as u64])
            }
            ClipState::SparseSpans { fingerprint, .. } => {
                Self::fingerprint_with_tag(0x04, [*fingerprint])
            }
            ClipState::RleMask { fingerprint, .. } => {
                Self::fingerprint_with_tag(0x05, [*fingerprint])
            }
            ClipState::DenseMask { fingerprint, .. } => {
                Self::fingerprint_with_tag(0x06, [*fingerprint])
            }
            ClipState::Composite { op, lhs, rhs } => {
                let tag = match op {
                    ClipCompositeOp::Intersect => 0x07,
                };
                Self::fingerprint_with_tag(tag, [lhs.state.fingerprint(), rhs.state.fingerprint()])
            }
        }
    }
}

/// The clip DAG interner: deduplicates clip states and provides structural
/// sharing for the renderer's clip stack.
///
/// The DAG holds `Arc<ClipNode>` entries keyed by fingerprint. When the
/// renderer pushes a save, it stores a cheap `Arc::clone` of the current node
/// instead of cloning the entire `ClipMask`. Intersections create new DAG
/// edges without re-rasterizing unless the result is actually painted through.
#[derive(Debug)]
pub struct ClipDag {
    /// Interned nodes by scoped structural fingerprint.
    nodes: HashMap<u64, Arc<ClipNode>>,
    /// Maximum retained intern entries before automatic unused-node pruning.
    max_nodes: usize,
    /// Pre-allocated flyweight nodes for Full and Empty.
    full_node: Arc<ClipNode>,
    empty_node: Arc<ClipNode>,
    /// Identity scope applied to nodes interned by this DAG.
    identity_scope: ClipIdentityScope,
    /// Statistics: total intern lookups.
    pub intern_lookups: u64,
    /// Statistics: cache hits (reuse instead of new allocation).
    pub intern_hits: u64,
    /// Statistics: new nodes created.
    pub nodes_created: u64,
    /// Statistics: pruning passes, manual or automatic.
    pub pruning_passes: u64,
    /// Statistics: nodes removed by pruning.
    pub pruned_nodes: u64,
    /// Statistics: over-capacity states where all excess nodes were still live.
    pub over_capacity_referenced: u64,
}

/// Statistics snapshot for the clip DAG.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct ClipDagStats {
    pub interned_nodes: usize,
    pub max_nodes: usize,
    pub intern_lookups: u64,
    pub intern_hits: u64,
    pub nodes_created: u64,
    pub pruning_passes: u64,
    pub pruned_nodes: u64,
    pub over_capacity_referenced: u64,
    pub approximate_bytes: usize,
}

impl ClipDag {
    /// Create a new empty DAG with flyweight Full/Empty nodes.
    pub fn new() -> Self {
        Self::with_identity_scope(ClipIdentityScope::default())
    }

    /// Create a new DAG with explicit render identity salts.
    pub fn with_identity_scope(identity_scope: ClipIdentityScope) -> Self {
        Self::with_identity_scope_and_limit(identity_scope, DEFAULT_MAX_CLIP_DAG_NODES)
    }

    /// Create a new DAG with explicit render identity salts and an intern-table cap.
    pub fn with_identity_scope_and_limit(
        identity_scope: ClipIdentityScope,
        max_nodes: usize,
    ) -> Self {
        let full_node = Arc::new(ClipNode::new_scoped(ClipState::Full, identity_scope));
        let empty_node = Arc::new(ClipNode::new_scoped(ClipState::Empty, identity_scope));
        let mut nodes = HashMap::with_capacity(64);
        nodes.insert(
            Self::scoped_fingerprint_for(identity_scope, &ClipState::Full),
            Arc::clone(&full_node),
        );
        nodes.insert(
            Self::scoped_fingerprint_for(identity_scope, &ClipState::Empty),
            Arc::clone(&empty_node),
        );
        Self {
            nodes,
            max_nodes: max_nodes.max(2),
            full_node,
            empty_node,
            identity_scope,
            intern_lookups: 0,
            intern_hits: 0,
            nodes_created: 0,
            pruning_passes: 0,
            pruned_nodes: 0,
            over_capacity_referenced: 0,
        }
    }

    /// Return the canonical Full node (no clip).
    #[inline]
    pub fn full(&self) -> Arc<ClipNode> {
        Arc::clone(&self.full_node)
    }

    /// Return the canonical Empty node (fully clipped).
    #[inline]
    pub fn empty(&self) -> Arc<ClipNode> {
        Arc::clone(&self.empty_node)
    }

    /// Intern a clip state, returning a shared node. If an equivalent state
    /// already exists in the DAG, the existing Arc is reused.
    pub fn intern(&mut self, state: ClipState) -> Arc<ClipNode> {
        self.intern_lookups += 1;
        let fp = self.scoped_fingerprint(&state);
        if let Some(existing) = self.nodes.get(&fp) {
            if existing.state == state {
                self.intern_hits += 1;
                return Arc::clone(existing);
            }
        }
        // New node.
        self.nodes_created += 1;
        let node = Arc::new(ClipNode::new_scoped(state, self.identity_scope));
        self.nodes.insert(fp, Arc::clone(&node));
        self.prune_if_over_capacity();
        node
    }

    /// Intern a `ClipMask` by classifying it into a `ClipState` first.
    pub fn intern_mask(&mut self, mask: &ClipMask) -> Arc<ClipNode> {
        let state = ClipState::from_clip_mask(mask);
        self.intern(state)
    }

    /// Intern an `Option<ClipMask>` — `None` maps to the Full node.
    pub fn intern_option(&mut self, mask: Option<&ClipMask>) -> Arc<ClipNode> {
        match mask {
            None => self.full(),
            Some(m) => self.intern_mask(m),
        }
    }

    /// Fuse an alpha-bearing soft mask or image mask with an existing clip node
    /// into a destination-local DAG node for a bounded source window. The helper
    /// avoids full-page clip materialization when tile/group/image mask
    /// execution already has bounded alpha bytes.
    pub fn fuse_alpha_mask_window(
        &mut self,
        alpha_mask: Option<&AlphaMask>,
        clip: Option<&ClipNode>,
        window: ClipAlphaFusionWindow,
    ) -> Option<Arc<ClipNode>> {
        let state = ClipState::fused_alpha_mask_window(alpha_mask, clip, window)?;
        Some(self.intern(state))
    }

    /// Create an intersection node in the DAG. Applies algebraic simplification:
    /// - Full ∩ X = X
    /// - Empty ∩ X = Empty
    /// - X ∩ X = X (identity)
    pub fn intersect(&mut self, lhs: &Arc<ClipNode>, rhs: &Arc<ClipNode>) -> Arc<ClipNode> {
        // Algebraic identities
        if lhs.state == ClipState::Full {
            return Arc::clone(rhs);
        }
        if rhs.state == ClipState::Full {
            return Arc::clone(lhs);
        }
        if lhs.state == ClipState::Empty || rhs.state == ClipState::Empty {
            return self.empty();
        }
        if lhs.state == rhs.state {
            return Arc::clone(lhs);
        }
        // Rectangle ∩ Rectangle → Rectangle (if containment or simple overlap)
        if let (
            ClipState::Rectangle {
                x: x1,
                y: y1,
                w: w1,
                h: h1,
            },
            ClipState::Rectangle {
                x: x2,
                y: y2,
                w: w2,
                h: h2,
            },
        ) = (&lhs.state, &rhs.state)
        {
            let ix0 = (*x1).max(*x2);
            let iy0 = (*y1).max(*y2);
            let ix1 = (*x1).saturating_add(*w1).min((*x2).saturating_add(*w2));
            let iy1 = (*y1).saturating_add(*h1).min((*y2).saturating_add(*h2));
            if ix1 > ix0 && iy1 > iy0 {
                return self.intern(ClipState::Rectangle {
                    x: ix0,
                    y: iy0,
                    w: ix1 - ix0,
                    h: iy1 - iy0,
                });
            } else {
                return self.empty();
            }
        }
        if let Some(state) = ClipState::intersect_structural(&lhs.state, &rhs.state) {
            return self.intern(state);
        }
        let (left, right) = if lhs.state.fingerprint() <= rhs.state.fingerprint() {
            (Arc::clone(lhs), Arc::clone(rhs))
        } else {
            (Arc::clone(rhs), Arc::clone(lhs))
        };
        let state = ClipState::Composite {
            op: ClipCompositeOp::Intersect,
            lhs: left,
            rhs: right,
        };
        self.intern(state)
    }

    /// Create a rectangle node.
    pub fn rectangle(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        buf_w: u32,
        buf_h: u32,
    ) -> Arc<ClipNode> {
        if w <= 0 || h <= 0 {
            return self.empty();
        }
        if x <= 0
            && y <= 0
            && x.saturating_add(w) >= buf_w as i32
            && y.saturating_add(h) >= buf_h as i32
        {
            return self.full();
        }
        self.intern(ClipState::Rectangle { x, y, w, h })
    }

    /// Return a statistics snapshot.
    pub fn stats(&self) -> ClipDagStats {
        let approximate_bytes: usize = self
            .nodes
            .values()
            .map(|n| n.approximate_bytes())
            .sum::<usize>()
            + self.nodes.capacity() * (8 + std::mem::size_of::<Arc<ClipNode>>());
        ClipDagStats {
            interned_nodes: self.nodes.len(),
            max_nodes: self.max_nodes,
            intern_lookups: self.intern_lookups,
            intern_hits: self.intern_hits,
            nodes_created: self.nodes_created,
            pruning_passes: self.pruning_passes,
            pruned_nodes: self.pruned_nodes,
            over_capacity_referenced: self.over_capacity_referenced,
            approximate_bytes,
        }
    }

    /// Evict nodes that are not referenced externally (only the DAG holds them).
    /// Returns the number of evicted entries.
    pub fn evict_unused(&mut self) -> usize {
        self.pruning_passes = self.pruning_passes.saturating_add(1);
        let mut total_evicted = 0usize;
        loop {
            let before = self.nodes.len();
            let full_node = Arc::clone(&self.full_node);
            let empty_node = Arc::clone(&self.empty_node);
            self.nodes.retain(|_fp, node| {
                Arc::ptr_eq(node, &full_node)
                    || Arc::ptr_eq(node, &empty_node)
                    || Arc::strong_count(node) > 1
            });
            let evicted = before - self.nodes.len();
            if evicted == 0 {
                break;
            }
            total_evicted = total_evicted.saturating_add(evicted);
        }
        self.pruned_nodes = self.pruned_nodes.saturating_add(total_evicted as u64);
        total_evicted
    }

    /// Reset statistics counters.
    pub fn reset_stats(&mut self) {
        self.intern_lookups = 0;
        self.intern_hits = 0;
        self.nodes_created = 0;
        self.pruning_passes = 0;
        self.pruned_nodes = 0;
        self.over_capacity_referenced = 0;
    }

    fn prune_if_over_capacity(&mut self) {
        if self.nodes.len() <= self.max_nodes {
            return;
        }
        self.evict_unused();
        if self.nodes.len() > self.max_nodes {
            self.over_capacity_referenced = self.over_capacity_referenced.saturating_add(1);
        }
    }

    fn scoped_fingerprint(&self, state: &ClipState) -> u64 {
        Self::scoped_fingerprint_for(self.identity_scope, state)
    }

    fn scoped_fingerprint_for(identity: ClipIdentityScope, state: &ClipState) -> u64 {
        identity.stable_id_for(state)
    }
}

impl Default for ClipDag {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn div255_round_u16(value: u16) -> u16 {
    let adjusted = value.saturating_add(128);
    (adjusted + (adjusted >> 8)) >> 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_and_empty_are_flyweights() {
        let dag = ClipDag::new();
        let f1 = dag.full();
        let f2 = dag.full();
        assert!(Arc::ptr_eq(&f1, &f2));

        let e1 = dag.empty();
        let e2 = dag.empty();
        assert!(Arc::ptr_eq(&e1, &e2));
    }

    #[test]
    fn rectangle_intern_reuses_on_repeat() {
        let mut dag = ClipDag::new();
        let r1 = dag.rectangle(10, 20, 100, 200, 800, 600);
        let r2 = dag.rectangle(10, 20, 100, 200, 800, 600);
        assert!(Arc::ptr_eq(&r1, &r2));
        assert_eq!(dag.intern_hits, 1);
        assert_eq!(dag.nodes_created, 1);
    }

    #[test]
    fn different_rectangles_are_distinct() {
        let mut dag = ClipDag::new();
        let r1 = dag.rectangle(10, 20, 100, 200, 800, 600);
        let r2 = dag.rectangle(15, 20, 100, 200, 800, 600);
        assert!(!Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn full_rect_normalizes_to_full() {
        let mut dag = ClipDag::new();
        // Rectangle covering the entire buffer
        let node = dag.rectangle(0, 0, 800, 600, 800, 600);
        assert_eq!(node.state, ClipState::Full);
    }

    #[test]
    fn empty_rect_normalizes_to_empty() {
        let mut dag = ClipDag::new();
        let node = dag.rectangle(10, 20, 0, 200, 800, 600);
        assert_eq!(node.state, ClipState::Empty);
    }

    #[test]
    fn intersect_full_with_rect_returns_rect() {
        let mut dag = ClipDag::new();
        let full = dag.full();
        let rect = dag.rectangle(10, 20, 100, 200, 800, 600);
        let result = dag.intersect(&full, &rect);
        assert!(Arc::ptr_eq(&result, &rect));
    }

    #[test]
    fn intersect_empty_with_anything_returns_empty() {
        let mut dag = ClipDag::new();
        let empty = dag.empty();
        let rect = dag.rectangle(10, 20, 100, 200, 800, 600);
        let result = dag.intersect(&empty, &rect);
        assert_eq!(result.state, ClipState::Empty);
    }

    #[test]
    fn intersect_rect_with_self_returns_same() {
        let mut dag = ClipDag::new();
        let rect = dag.rectangle(10, 20, 100, 200, 800, 600);
        let result = dag.intersect(&rect, &rect);
        assert!(Arc::ptr_eq(&result, &rect));
    }

    #[test]
    fn intersect_two_rects_produces_overlap() {
        let mut dag = ClipDag::new();
        let r1 = dag.rectangle(0, 0, 100, 100, 800, 600);
        let r2 = dag.rectangle(50, 50, 100, 100, 800, 600);
        let result = dag.intersect(&r1, &r2);
        assert_eq!(
            result.state,
            ClipState::Rectangle {
                x: 50,
                y: 50,
                w: 50,
                h: 50
            }
        );
    }

    #[test]
    fn intersect_non_overlapping_rects_is_empty() {
        let mut dag = ClipDag::new();
        let r1 = dag.rectangle(0, 0, 50, 50, 800, 600);
        let r2 = dag.rectangle(100, 100, 50, 50, 800, 600);
        let result = dag.intersect(&r1, &r2);
        assert_eq!(result.state, ClipState::Empty);
    }

    #[test]
    fn materialize_full_produces_all_visible_mask() {
        let dag = ClipDag::new();
        let node = dag.full();
        let mask = node.materialize(100, 100);
        assert!(mask.is_all_visible());
    }

    #[test]
    fn materialize_empty_produces_empty_mask() {
        let dag = ClipDag::new();
        let node = dag.empty();
        let mask = node.materialize(100, 100);
        assert!(mask.is_empty());
    }

    #[test]
    fn materialize_rectangle_matches_clip_mask_from_visible_rect() {
        let mut dag = ClipDag::new();
        let node = dag.rectangle(10, 20, 50, 30, 100, 100);
        let mask = node.materialize(100, 100);
        let direct = ClipMask::from_visible_rect(100, 100, 10, 20, 50, 30);
        // Both should have same visibility at sample points
        assert!(mask.is_visible(15, 25));
        assert!(direct.is_visible(15, 25));
        assert!(!mask.is_visible(5, 5));
        assert!(!direct.is_visible(5, 5));
    }

    #[test]
    fn materialize_window_rectangle_uses_destination_local_coordinates() {
        let mut dag = ClipDag::new();
        let node = dag.rectangle(12, 22, 6, 6, 100, 100);
        let window = node
            .materialize_window(100, 100, 10, 20, 10, 10)
            .expect("valid window");

        assert!(window.is_visible(2, 2));
        assert!(window.is_visible(7, 7));
        assert!(!window.is_visible(1, 2));
        assert!(!window.is_visible(8, 7));
        assert!(!node.is_materialized());
    }

    #[test]
    fn materialize_window_sparse_rows_shift_runs_locally() {
        let mut rows = vec![Vec::new(); 20];
        rows[12].push((5, 15));
        let mask = ClipMask::from_visible_runs(20, 20, rows);
        let mut dag = ClipDag::new();
        let node = dag.intern_mask(&mask);
        let window = node
            .materialize_window(20, 20, 8, 10, 5, 5)
            .expect("valid window");

        assert!(window.is_visible(0, 2));
        assert!(window.is_visible(4, 2));
        assert!(!window.is_visible(0, 1));
        assert!(!node.is_materialized());
    }

    #[test]
    fn materialize_window_dense_alpha_preserves_partial_coverage() {
        let mask = ClipMask::from_alpha_bytes(4, 2, vec![0, 10, 20, 30, 40, 50, 60, 70]);
        let mut dag = ClipDag::new();
        let node = dag.intern_mask(&mask);
        let window = node
            .materialize_window(4, 2, 1, 0, 2, 2)
            .expect("valid window");

        assert_eq!(window.opacity_byte(0, 0), 10);
        assert_eq!(window.opacity_byte(1, 0), 20);
        assert_eq!(window.opacity_byte(0, 1), 50);
        assert_eq!(window.opacity_byte(1, 1), 60);
        assert!(!node.is_materialized());
    }

    #[test]
    fn materialize_dense_alpha_size_mismatch_preserves_overlap() {
        let mask = ClipMask::from_alpha_bytes(3, 2, vec![0, 64, 128, 192, 255, 32]);
        let mut dag = ClipDag::new();
        let node = dag.intern_mask(&mask);

        let expanded = node.materialize(5, 3);
        assert_eq!(expanded.opacity_byte(0, 0), 0);
        assert_eq!(expanded.opacity_byte(1, 0), 64);
        assert_eq!(expanded.opacity_byte(2, 0), 128);
        assert_eq!(expanded.opacity_byte(0, 1), 192);
        assert_eq!(expanded.opacity_byte(1, 1), 255);
        assert_eq!(expanded.opacity_byte(2, 1), 32);
        assert_eq!(expanded.opacity_byte(3, 1), 0);
        assert_eq!(expanded.opacity_byte(1, 2), 0);
        assert!(!expanded.is_empty());

        let shrunk = node.state.to_clip_mask(2, 1);
        assert_eq!(shrunk.opacity_byte(0, 0), 0);
        assert_eq!(shrunk.opacity_byte(1, 0), 64);
        assert!(!shrunk.is_empty());
    }

    #[test]
    fn materialize_window_composite_intersects_local_children() {
        let dense = ClipMask::from_alpha_bytes(4, 2, vec![0, 10, 20, 30, 40, 50, 60, 70]);
        let sparse = ClipMask::from_visible_runs(4, 2, vec![vec![(2, 4)], vec![(1, 3)]]);
        let mut dag = ClipDag::new();
        let dense_node = dag.intern_mask(&dense);
        let sparse_node = dag.intern_mask(&sparse);
        let composite = dag.intersect(&dense_node, &sparse_node);
        let window = composite
            .materialize_window(4, 2, 1, 0, 2, 2)
            .expect("valid window");

        assert_eq!(window.opacity_byte(0, 0), 0);
        assert_eq!(window.opacity_byte(1, 0), 20);
        assert_eq!(window.opacity_byte(0, 1), 50);
        assert_eq!(window.opacity_byte(1, 1), 60);
        assert!(!composite.is_materialized());
    }

    #[test]
    fn materialize_window_composite_short_circuits_empty_child_window() {
        let mut rows = vec![Vec::new(); 4];
        rows[3].push((0, 4));
        let sparse = ClipMask::from_visible_runs(4, 4, rows);
        let full = ClipMask::all_visible(4, 4);
        let mut dag = ClipDag::new();
        let sparse_node = dag.intern_mask(&sparse);
        let full_node = dag.intern_mask(&full);
        let composite = ClipNode::new(ClipState::Composite {
            op: ClipCompositeOp::Intersect,
            lhs: Arc::clone(&sparse_node),
            rhs: Arc::clone(&full_node),
        });

        let window = composite
            .materialize_window(4, 4, 0, 0, 2, 2)
            .expect("valid window");

        assert!(window.is_empty());
        assert!(!composite.is_materialized());
        assert!(!sparse_node.is_materialized());
        assert!(!full_node.is_materialized());
    }

    #[test]
    fn materialize_is_lazy_and_cached() {
        let mut dag = ClipDag::new();
        let node = dag.rectangle(10, 20, 50, 30, 100, 100);
        assert!(!node.is_materialized());
        let _mask = node.materialize(100, 100);
        assert!(node.is_materialized());
        let m1 = node.materialize(100, 100);
        let m2 = node.materialize(100, 100);
        assert!(Arc::ptr_eq(&m1, &m2));
    }

    #[test]
    fn materialize_cache_is_keyed_by_dimensions() {
        let mut dag = ClipDag::new();
        let node = dag.rectangle(1, 1, 4, 4, 100, 100);

        let small = node.materialize(10, 10);
        let large = node.materialize(20, 20);
        let small_again = node.materialize(10, 10);

        assert_eq!((small.width, small.height), (10, 10));
        assert_eq!((large.width, large.height), (20, 20));
        assert!(Arc::ptr_eq(&small, &small_again));
        assert!(!Arc::ptr_eq(&small, &large));
    }

    #[test]
    fn intern_mask_round_trips_full() {
        let mut dag = ClipDag::new();
        let mask = ClipMask::all_visible(200, 200);
        let node = dag.intern_mask(&mask);
        assert_eq!(node.state, ClipState::Full);
    }

    #[test]
    fn intern_mask_round_trips_empty() {
        let mut dag = ClipDag::new();
        let mask = ClipMask::empty(200, 200);
        let node = dag.intern_mask(&mask);
        assert_eq!(node.state, ClipState::Empty);
    }

    #[test]
    fn intern_mask_round_trips_rectangle() {
        let mut dag = ClipDag::new();
        let mask = ClipMask::from_visible_rect(200, 200, 10, 20, 50, 30);
        let node = dag.intern_mask(&mask);
        assert_eq!(
            node.state,
            ClipState::Rectangle {
                x: 10,
                y: 20,
                w: 50,
                h: 30
            }
        );
    }

    #[test]
    fn intern_option_none_is_full() {
        let mut dag = ClipDag::new();
        let node = dag.intern_option(None);
        assert_eq!(node.state, ClipState::Full);
    }

    #[test]
    fn save_restore_reuse_shows_no_allocation_growth() {
        let mut dag = ClipDag::new();
        // Simulate: set a clip, save 10 times, restore 10 times
        let clip = dag.rectangle(10, 10, 80, 80, 100, 100);
        let mut stack: Vec<Arc<ClipNode>> = Vec::new();
        for _ in 0..10 {
            stack.push(Arc::clone(&clip)); // save is just Arc::clone
        }
        // All stack entries point to same node — zero mask allocation
        for entry in &stack {
            assert!(Arc::ptr_eq(entry, &clip));
        }
        // Restore: just pop
        for _ in 0..10 {
            let restored = stack.pop().unwrap();
            assert!(Arc::ptr_eq(&restored, &clip));
        }
        // Only 1 rectangle node was created
        assert_eq!(dag.nodes_created, 1);
    }

    #[test]
    fn intersection_dag_node_deduplicates_on_repeat() {
        let mut dag = ClipDag::new();
        let r1 = dag.rectangle(0, 0, 100, 100, 200, 200);
        let r2 = dag.rectangle(50, 50, 100, 100, 200, 200);
        let i1 = dag.intersect(&r1, &r2);
        let i2 = dag.intersect(&r1, &r2);
        // Same fingerprint → same interned node
        assert!(Arc::ptr_eq(&i1, &i2));
    }

    #[test]
    fn evict_unused_removes_unreferenced_nodes() {
        let mut dag = ClipDag::new();
        let r1 = dag.rectangle(10, 10, 50, 50, 100, 100);
        let initial_count = dag.stats().interned_nodes;
        drop(r1);
        let evicted = dag.evict_unused();
        assert!(evicted > 0);
        assert!(dag.stats().interned_nodes < initial_count);
    }

    #[test]
    fn evict_unused_keeps_referenced_nodes() {
        let mut dag = ClipDag::new();
        let r1 = dag.rectangle(10, 10, 50, 50, 100, 100);
        let _hold = Arc::clone(&r1);
        dag.evict_unused();
        // Still interned because _hold keeps a strong reference
        let r2 = dag.rectangle(10, 10, 50, 50, 100, 100);
        assert!(Arc::ptr_eq(&r1, &r2));
    }

    #[test]
    fn evict_unused_prunes_composite_children_in_same_pass() {
        let dense_mask = ClipMask::from_alpha_bytes(3, 1, vec![0, 128, 255]);
        let sparse_mask = ClipMask::from_visible_runs(3, 1, vec![vec![(1, 3)]]);

        let mut dag = ClipDag::new();
        let dense = dag.intern_mask(&dense_mask);
        let sparse = dag.intern_mask(&sparse_mask);
        let composite = dag.intern(ClipState::Composite {
            op: ClipCompositeOp::Intersect,
            lhs: Arc::clone(&dense),
            rhs: Arc::clone(&sparse),
        });
        assert!(matches!(composite.state, ClipState::Composite { .. }));
        assert_eq!(dag.stats().interned_nodes, 5);

        drop(composite);
        drop(dense);
        drop(sparse);

        let evicted = dag.evict_unused();
        let stats = dag.stats();
        assert_eq!(evicted, 3);
        assert_eq!(stats.pruned_nodes, 3);
        assert_eq!(stats.interned_nodes, 2);
    }

    #[test]
    fn intern_prunes_unused_nodes_when_over_capacity() {
        let mut dag = ClipDag::with_identity_scope_and_limit(ClipIdentityScope::default(), 3);
        let stale = dag.rectangle(1, 1, 10, 10, 100, 100);
        drop(stale);

        let live = dag.rectangle(20, 20, 10, 10, 100, 100);
        let stats = dag.stats();
        assert!(stats.pruning_passes > 0);
        assert!(stats.pruned_nodes > 0);
        assert_eq!(stats.over_capacity_referenced, 0);
        assert!(stats.interned_nodes <= stats.max_nodes);
        let live_again = dag.rectangle(20, 20, 10, 10, 100, 100);
        assert!(Arc::ptr_eq(&live, &live_again));
    }

    #[test]
    fn intern_keeps_live_nodes_when_over_capacity() {
        let mut dag = ClipDag::with_identity_scope_and_limit(ClipIdentityScope::default(), 3);
        let first = dag.rectangle(1, 1, 10, 10, 100, 100);
        let second = dag.rectangle(20, 20, 10, 10, 100, 100);

        let stats = dag.stats();
        assert!(stats.pruning_passes > 0);
        assert_eq!(stats.pruned_nodes, 0);
        assert_eq!(stats.over_capacity_referenced, 1);
        assert!(stats.interned_nodes > stats.max_nodes);

        let first_again = dag.rectangle(1, 1, 10, 10, 100, 100);
        let second_again = dag.rectangle(20, 20, 10, 10, 100, 100);
        assert!(Arc::ptr_eq(&first, &first_again));
        assert!(Arc::ptr_eq(&second, &second_again));
    }

    #[test]
    fn narrow_allocation_path_clips_do_not_allocate_dense_mask() {
        let mut dag = ClipDag::new();
        // Create several rectangle clips and intersect them
        let r1 = dag.rectangle(10, 10, 200, 200, 400, 400);
        let r2 = dag.rectangle(50, 50, 200, 200, 400, 400);
        let _inter = dag.intersect(&r1, &r2);
        // None of these have materialized a dense mask yet
        assert!(!r1.is_materialized());
        assert!(!r2.is_materialized());
        assert!(!_inter.is_materialized());
        // Total DAG bytes should remain far below a 400x400 dense byte plane.
        let stats = dag.stats();
        let dense_mask_bytes = 400usize * 400usize;
        assert!(
            stats.approximate_bytes < dense_mask_bytes / 8,
            "DAG should remain materially narrower than dense mask allocation, got {} bytes",
            stats.approximate_bytes
        );
    }

    #[test]
    fn clip_state_from_mask_path_fingerprint_is_stable() {
        // Create a non-trivial mask from visible runs
        let runs = vec![
            vec![(5, 95)],  // row 0
            vec![(10, 90)], // row 1
            vec![(15, 85)], // row 2
        ];
        let mask = ClipMask::from_visible_runs(100, 3, runs.clone());
        let state1 = ClipState::from_clip_mask(&mask);
        let state2 = ClipState::from_clip_mask(&mask);
        assert_eq!(state1.fingerprint(), state2.fingerprint());
    }

    #[test]
    fn sparse_binary_clip_uses_sparse_spans_representation() {
        let mut rows = vec![Vec::new(); 100];
        rows[50].push((10, 20));
        rows[50].push((30, 40));
        let mask = ClipMask::from_visible_runs(100, 100, rows);
        let mut dag = ClipDag::new();
        let node = dag.intern_mask(&mask);

        match &node.state {
            ClipState::SparseSpans {
                width,
                height,
                rows,
                ..
            } => {
                assert_eq!((*width, *height), (100, 100));
                assert_eq!(rows[50], vec![(10, 20), (30, 40)]);
            }
            other => panic!("expected SparseSpans, got {other:?}"),
        }
        assert_eq!(node.operation, ClipOperation::SparseSpans);
        assert_eq!(
            node.bounds,
            Some(ClipBounds {
                x0: 10,
                y0: 50,
                x1: 40,
                y1: 51
            })
        );
        assert!(node.memory_charge < 100 * 100);
    }

    #[test]
    fn broad_binary_clip_uses_rle_mask_representation() {
        let mut rows = vec![Vec::new(); 64];
        for row in &mut rows {
            row.push((0, 8));
            row.push((16, 24));
            row.push((32, 40));
        }
        let mask = ClipMask::from_visible_runs(64, 64, rows);
        let mut dag = ClipDag::new();
        let node = dag.intern_mask(&mask);

        match &node.state {
            ClipState::RleMask {
                width,
                height,
                runs,
                ..
            } => {
                assert_eq!((*width, *height), (64, 64));
                assert_eq!(runs.len(), 64);
                assert_eq!(runs[0], vec![(0, 8), (16, 24), (32, 40)]);
            }
            other => panic!("expected RleMask, got {other:?}"),
        }
        assert_eq!(node.operation, ClipOperation::RleMask);
        assert_eq!(
            node.bounds,
            Some(ClipBounds {
                x0: 0,
                y0: 0,
                x1: 40,
                y1: 64
            })
        );
    }

    #[test]
    fn partial_coverage_clip_uses_dense_mask_and_round_trips_alpha() {
        let mask = ClipMask::from_alpha_bytes(3, 1, vec![0, 128, 255]);
        let mut dag = ClipDag::new();
        let node = dag.intern_mask(&mask);

        match &node.state {
            ClipState::DenseMask {
                width,
                height,
                bytes,
                ..
            } => {
                assert_eq!((*width, *height), (3, 1));
                assert_eq!(bytes.as_slice(), &[0, 128, 255]);
            }
            other => panic!("expected DenseMask, got {other:?}"),
        }
        assert_eq!(node.operation, ClipOperation::DenseMask);
        assert_eq!(node.materialize(3, 1).opacity_byte(1, 0), 128);
        assert_eq!(node.materialize(3, 1).opacity_byte(2, 0), 255);
    }

    #[test]
    fn sparse_rle_intersection_stays_sparse_without_materialization() {
        let mut sparse_rows = vec![Vec::new(); 64];
        sparse_rows[4].push((2, 6));
        sparse_rows[4].push((8, 12));
        let sparse_mask = ClipMask::from_visible_runs(16, 64, sparse_rows);

        let mut rle_rows = vec![Vec::new(); 64];
        for row in &mut rle_rows {
            row.push((0, 4));
            row.push((10, 14));
        }
        let rle_mask = ClipMask::from_visible_runs(16, 64, rle_rows);

        let mut dag = ClipDag::new();
        let sparse = dag.intern_mask(&sparse_mask);
        let rle = dag.intern_mask(&rle_mask);
        let intersected = dag.intersect(&sparse, &rle);

        match &intersected.state {
            ClipState::SparseSpans { rows, .. } => {
                assert_eq!(rows[4], vec![(2, 4), (10, 12)]);
                assert!(rows
                    .iter()
                    .enumerate()
                    .all(|(idx, row)| idx == 4 || row.is_empty()));
            }
            other => panic!("expected SparseSpans intersection, got {other:?}"),
        }
        assert_eq!(intersected.operation, ClipOperation::SparseSpans);
        assert!(!sparse.is_materialized());
        assert!(!rle.is_materialized());
        assert!(!intersected.is_materialized());
    }

    #[test]
    fn rle_rle_intersection_stays_rle_for_broad_rows() {
        let mut left_rows = vec![Vec::new(); 64];
        let mut right_rows = vec![Vec::new(); 64];
        for row in &mut left_rows {
            row.push((0, 8));
            row.push((16, 24));
            row.push((32, 40));
        }
        for row in &mut right_rows {
            row.push((4, 12));
            row.push((20, 28));
            row.push((36, 44));
        }
        let left_mask = ClipMask::from_visible_runs(64, 64, left_rows);
        let right_mask = ClipMask::from_visible_runs(64, 64, right_rows);

        let mut dag = ClipDag::new();
        let left = dag.intern_mask(&left_mask);
        let right = dag.intern_mask(&right_mask);
        let intersected = dag.intersect(&left, &right);

        match &intersected.state {
            ClipState::RleMask { runs, .. } => {
                assert_eq!(runs.len(), 64);
                assert_eq!(runs[0], vec![(4, 8), (20, 24), (36, 40)]);
                assert_eq!(runs[63], vec![(4, 8), (20, 24), (36, 40)]);
            }
            other => panic!("expected RleMask intersection, got {other:?}"),
        }
        assert!(!left.is_materialized());
        assert!(!right.is_materialized());
        assert!(!intersected.is_materialized());
    }

    #[test]
    fn rectangle_binary_intersection_stays_structural() {
        let mut rows = vec![Vec::new(); 16];
        rows[2].push((0, 6));
        rows[2].push((8, 12));
        let mask = ClipMask::from_visible_runs(16, 16, rows);

        let mut dag = ClipDag::new();
        let binary = dag.intern_mask(&mask);
        let rect = dag.rectangle(3, 0, 6, 16, 16, 16);
        let intersected = dag.intersect(&rect, &binary);

        match &intersected.state {
            ClipState::SparseSpans { rows, .. } => {
                assert_eq!(rows[2], vec![(3, 6), (8, 9)]);
            }
            other => panic!("expected SparseSpans intersection, got {other:?}"),
        }
        assert!(!binary.is_materialized());
        assert!(!rect.is_materialized());
        assert!(!intersected.is_materialized());
    }

    #[test]
    fn dense_binary_intersection_fuses_dense_without_materialization() {
        let dense_mask = ClipMask::from_alpha_bytes(4, 1, vec![0, 128, 255, 64]);
        let sparse_mask = ClipMask::from_visible_runs(4, 1, vec![vec![(1, 3)]]);

        let mut dag = ClipDag::new();
        let dense = dag.intern_mask(&dense_mask);
        let sparse = dag.intern_mask(&sparse_mask);
        let fused = dag.intersect(&dense, &sparse);

        match &fused.state {
            ClipState::DenseMask { bytes, .. } => {
                assert_eq!(bytes.as_slice(), &[0, 128, 255, 0]);
            }
            other => panic!("expected fused DenseMask, got {other:?}"),
        }
        assert_eq!(fused.operation, ClipOperation::DenseMask);
        assert!(fused.parent.is_none());
        assert!(!dense.is_materialized());
        assert!(!sparse.is_materialized());
        assert!(!fused.is_materialized());
        assert_eq!(fused.materialize(4, 1).opacity_byte(1, 0), 128);
        assert_eq!(fused.materialize(4, 1).opacity_byte(2, 0), 255);
        assert_eq!(fused.materialize(4, 1).opacity_byte(3, 0), 0);
    }

    #[test]
    fn dense_rectangle_intersection_fuses_dense_without_materialization() {
        let dense_mask = ClipMask::from_alpha_bytes(4, 2, vec![0, 128, 255, 64, 32, 255, 96, 0]);

        let mut dag = ClipDag::new();
        let dense = dag.intern_mask(&dense_mask);
        let rect = dag.rectangle(1, 0, 2, 2, 4, 2);
        let fused = dag.intersect(&rect, &dense);

        match &fused.state {
            ClipState::DenseMask { bytes, .. } => {
                assert_eq!(bytes.as_slice(), &[0, 128, 255, 0, 0, 255, 96, 0]);
            }
            other => panic!("expected fused DenseMask, got {other:?}"),
        }
        assert!(!dense.is_materialized());
        assert!(!rect.is_materialized());
        assert!(!fused.is_materialized());
    }

    #[test]
    fn dense_dense_intersection_fuses_min_alpha_without_materialization() {
        let left_mask = ClipMask::from_alpha_bytes(4, 1, vec![255, 128, 64, 0]);
        let right_mask = ClipMask::from_alpha_bytes(4, 1, vec![128, 255, 32, 255]);

        let mut dag = ClipDag::new();
        let left = dag.intern_mask(&left_mask);
        let right = dag.intern_mask(&right_mask);
        let fused = dag.intersect(&left, &right);

        match &fused.state {
            ClipState::DenseMask { bytes, .. } => {
                assert_eq!(bytes.as_slice(), &[128, 128, 32, 0]);
            }
            other => panic!("expected fused DenseMask, got {other:?}"),
        }
        assert!(!left.is_materialized());
        assert!(!right.is_materialized());
        assert!(!fused.is_materialized());
    }

    #[test]
    fn alpha_mask_window_fuses_with_clip_into_dense_dag_node() {
        let identity = ClipIdentityScope {
            revision_identity: 7,
            render_contract_identity: 11,
            tile_identity: 13,
        };
        let mut dag = ClipDag::with_identity_scope(identity);
        let mut alpha = AlphaMask::all_opaque(5, 2);
        alpha.set(1, 0, 128);
        alpha.set(2, 0, 64);
        alpha.set(3, 0, 255);
        let clip_mask =
            ClipMask::from_alpha_bytes(5, 2, vec![0, 255, 128, 0, 255, 255, 255, 255, 255, 255]);
        let clip = dag.intern_mask(&clip_mask);

        let fused = dag
            .fuse_alpha_mask_window(
                Some(&alpha),
                Some(clip.as_ref()),
                ClipAlphaFusionWindow {
                    source_width: 5,
                    source_height: 2,
                    x: 1,
                    y: 0,
                    width: 3,
                    height: 1,
                },
            )
            .expect("fused alpha mask window");

        match &fused.state {
            ClipState::DenseMask {
                width,
                height,
                bytes,
                ..
            } => {
                assert_eq!((*width, *height), (3, 1));
                assert_eq!(bytes.as_slice(), &[128, 32, 0]);
            }
            other => panic!("expected fused DenseMask, got {other:?}"),
        }
        assert_eq!(fused.revision_identity, identity.revision_identity);
        assert_eq!(
            fused.render_contract_identity,
            identity.render_contract_identity
        );
        assert_eq!(fused.tile_identity, identity.tile_identity);
        assert!(!clip.is_materialized());
        assert!(!fused.is_materialized());
    }

    #[test]
    fn binary_image_mask_window_simplifies_to_structural_clip_node() {
        let mut dag = ClipDag::new();
        let mut image_mask = AlphaMask::filled(4, 1, 0);
        image_mask.set(1, 0, 255);
        image_mask.set(2, 0, 255);

        let fused = dag
            .fuse_alpha_mask_window(
                Some(&image_mask),
                None,
                ClipAlphaFusionWindow {
                    source_width: 4,
                    source_height: 1,
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 1,
                },
            )
            .expect("binary image mask window");

        assert_eq!(
            fused.state,
            ClipState::Rectangle {
                x: 1,
                y: 0,
                w: 2,
                h: 1
            }
        );
        assert_eq!(fused.operation, ClipOperation::Rectangle);
        assert!(!fused.is_materialized());
    }

    #[test]
    fn alpha_mask_window_empty_clip_short_circuits_to_empty_node() {
        let mut dag = ClipDag::new();
        let alpha = AlphaMask::all_opaque(4, 1);
        let empty = dag.empty();

        let fused = dag
            .fuse_alpha_mask_window(
                Some(&alpha),
                Some(empty.as_ref()),
                ClipAlphaFusionWindow {
                    source_width: 4,
                    source_height: 1,
                    x: 0,
                    y: 0,
                    width: 4,
                    height: 1,
                },
            )
            .expect("empty clip fused node");

        assert_eq!(fused.state, ClipState::Empty);
        assert!(Arc::ptr_eq(&fused, &empty));
        assert!(!fused.is_materialized());
    }

    #[test]
    fn identity_scope_is_recorded_on_nodes() {
        let identity = ClipIdentityScope {
            revision_identity: 11,
            render_contract_identity: 22,
            tile_identity: 33,
        };
        let mut dag = ClipDag::with_identity_scope(identity);
        let rect = dag.rectangle(1, 2, 3, 4, 100, 100);

        assert_eq!(rect.revision_identity, 11);
        assert_eq!(rect.render_contract_identity, 22);
        assert_eq!(rect.tile_identity, 33);
        assert_eq!(rect.stable_id, identity.stable_id_for(&rect.state));
        assert_ne!(
            ClipNode::new(ClipState::Full),
            ClipNode::new_scoped(ClipState::Full, identity)
        );
    }

    #[test]
    fn stats_reports_node_counts_correctly() {
        let mut dag = ClipDag::new();
        dag.rectangle(10, 10, 50, 50, 100, 100);
        dag.rectangle(20, 20, 30, 30, 100, 100);
        dag.rectangle(10, 10, 50, 50, 100, 100); // hit
        let stats = dag.stats();
        assert_eq!(stats.interned_nodes, 4); // Full + Empty + 2 rects
        assert_eq!(stats.intern_hits, 1);
        assert_eq!(stats.nodes_created, 2);
    }
}
