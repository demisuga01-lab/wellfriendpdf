//! Persistent interned clip-state DAG for the raster/retained renderer.
//!
//! PDF graphics state save/restore (`q`/`Q`) frequently pushes and pops
//! identical clip states. The naive approach clones the full `ClipMask` on
//! every save, allocating dense byte planes or run-length vectors repeatedly
//! for identical geometry.
//!
//! This module provides a structural DAG of clip states where:
//! - Common states (`Full`, `Empty`, `Rectangle`) are flyweights.
//! - Path-derived clips are interned by content hash.
//! - Intersections form DAG edges, enabling structural sharing on save/restore.
//! - `Arc`-based nodes allow zero-copy push/pop on the graphics state stack.
//!
//! The DAG integrates into the active renderer path: `RenderState` uses
//! `ClipDag` to intern clip states and the clip stack holds `Arc<ClipNode>`
//! handles instead of owned `Option<ClipMask>` clones.

use std::collections::HashMap;
use std::sync::Arc;

use super::buffer::ClipMask;

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
    /// Arbitrary path-derived binary clip, stored as sorted run-length rows.
    /// The `fingerprint` is a content hash for deduplication.
    Path {
        fingerprint: u64,
        width: u32,
        height: u32,
        runs: Arc<Vec<Vec<(i32, i32)>>>,
    },
    /// Intersection of two clip states (structural sharing).
    Intersection(Arc<ClipNode>, Arc<ClipNode>),
}

/// A node in the clip DAG: a clip state plus its cached materialization.
#[derive(Debug, Clone)]
pub struct ClipNode {
    pub state: ClipState,
    /// Lazily materialized ClipMask. Populated on first use for rendering.
    materialized: std::sync::OnceLock<ClipMask>,
}

impl PartialEq for ClipNode {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl Eq for ClipNode {}

impl std::hash::Hash for ClipNode {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.state.hash(hasher);
    }
}

impl ClipNode {
    /// Create a new node with a given state.
    pub fn new(state: ClipState) -> Self {
        Self {
            state,
            materialized: std::sync::OnceLock::new(),
        }
    }

    /// Get or materialize the concrete `ClipMask` for this node at the given
    /// buffer dimensions.
    pub fn materialize(&self, width: u32, height: u32) -> &ClipMask {
        self.materialized
            .get_or_init(|| self.state.to_clip_mask(width, height))
    }

    /// Check if the materialized mask is already available without computing it.
    pub fn is_materialized(&self) -> bool {
        self.materialized.get().is_some()
    }

    /// Return approximate memory bytes held by this node (excluding Arc overhead).
    pub fn approximate_bytes(&self) -> usize {
        let state_bytes = match &self.state {
            ClipState::Full | ClipState::Empty => 0,
            ClipState::Rectangle { .. } => 16,
            ClipState::Path { runs, .. } => {
                runs.iter().map(|r| r.len() * 8 + 24).sum::<usize>() + 24
            }
            ClipState::Intersection(_, _) => 16, // two Arc ptrs
        };
        let mat_bytes = self
            .materialized
            .get()
            .map(|m| std::mem::size_of::<ClipMask>() + m.width as usize * m.height as usize)
            .unwrap_or(0);
        std::mem::size_of::<Self>() + state_bytes + mat_bytes
    }
}

impl ClipState {
    /// Materialize a concrete `ClipMask` from this DAG state.
    pub fn to_clip_mask(&self, width: u32, height: u32) -> ClipMask {
        match self {
            ClipState::Full => ClipMask::all_visible(width, height),
            ClipState::Empty => ClipMask::empty(width, height),
            ClipState::Rectangle { x, y, w, h } => {
                ClipMask::from_visible_rect(width, height, *x, *y, *w, *h)
            }
            ClipState::Path { runs, .. } => {
                ClipMask::from_visible_runs(width, height, (**runs).clone())
            }
            ClipState::Intersection(lhs, rhs) => {
                let mut left = lhs.state.to_clip_mask(width, height);
                let right = rhs.state.to_clip_mask(width, height);
                left.intersect(&right);
                left
            }
        }
    }

    /// Classify a `ClipMask` into its structural `ClipState` representation.
    ///
    /// This inspects the mask's structural hints to determine the cheapest DAG
    /// node that represents it, avoiding dense-mask materialization.
    pub fn from_clip_mask(mask: &ClipMask) -> Self {
        if mask.is_all_visible() {
            return ClipState::Full;
        }
        if mask.is_empty() {
            return ClipState::Empty;
        }
        // Try to detect a rectangle from the visible bounds and run structure.
        if let Some((x0, y0, x1, y1)) = mask.visible_bounds() {
            let w = x1 - x0;
            let h = y1 - y0;
            if Self::mask_is_solid_rect(mask, x0, y0, x1, y1) {
                return ClipState::Rectangle { x: x0, y: y0, w, h };
            }
        }
        // General path: extract runs and fingerprint them.
        let runs = Self::extract_runs(mask);
        let fingerprint = Self::fingerprint_runs(&runs, mask.width, mask.height);
        ClipState::Path {
            fingerprint,
            width: mask.width,
            height: mask.height,
            runs: Arc::new(runs),
        }
    }

    /// Check if a mask is a solid filled rectangle between the given bounds.
    fn mask_is_solid_rect(mask: &ClipMask, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
        if mask.has_partial_coverage() {
            return false;
        }
        // Verify all rows in [y0, y1) have exactly one run spanning [x0, x1)
        // and all rows outside that range are empty.
        for y in 0..mask.height as i32 {
            let mut row_visible = 0i32;
            mask.for_each_visible_run(y, mask.width as i32, |start, end| {
                row_visible += end - start;
            });
            if y >= y0 && y < y1 {
                // Row inside rect: must have exactly (x1 - x0) visible pixels
                // in a single span from x0 to x1.
                let expected = x1 - x0;
                if row_visible != expected {
                    return false;
                }
                // Verify the single span matches
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

    /// FNV-1a content hash over the run structure for deduplication.
    fn fingerprint_runs(runs: &[Vec<(i32, i32)>], width: u32, height: u32) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        let mix = |h: &mut u64, byte: u8| {
            *h ^= byte as u64;
            *h = h.wrapping_mul(0x100000001b3);
        };
        for b in width.to_le_bytes() {
            mix(&mut hash, b);
        }
        for b in height.to_le_bytes() {
            mix(&mut hash, b);
        }
        for row in runs {
            for (start, end) in row {
                for b in start.to_le_bytes() {
                    mix(&mut hash, b);
                }
                for b in end.to_le_bytes() {
                    mix(&mut hash, b);
                }
            }
            // Row separator
            mix(&mut hash, 0xFF);
        }
        hash
    }

    /// Return a structural fingerprint for this state (for interner lookup).
    pub fn fingerprint(&self) -> u64 {
        match self {
            ClipState::Full => 0x0000_0000_0000_0001,
            ClipState::Empty => 0x0000_0000_0000_0002,
            ClipState::Rectangle { x, y, w, h } => {
                let mut hash: u64 = 0xcbf29ce484222325;
                let mix = |hh: &mut u64, byte: u8| {
                    *hh ^= byte as u64;
                    *hh = hh.wrapping_mul(0x100000001b3);
                };
                mix(&mut hash, 0x03); // tag byte for Rectangle
                for b in x.to_le_bytes() {
                    mix(&mut hash, b);
                }
                for b in y.to_le_bytes() {
                    mix(&mut hash, b);
                }
                for b in w.to_le_bytes() {
                    mix(&mut hash, b);
                }
                for b in h.to_le_bytes() {
                    mix(&mut hash, b);
                }
                hash
            }
            ClipState::Path { fingerprint, .. } => *fingerprint,
            ClipState::Intersection(lhs, rhs) => {
                let mut hash: u64 = 0xcbf29ce484222325;
                let mix = |hh: &mut u64, byte: u8| {
                    *hh ^= byte as u64;
                    *hh = hh.wrapping_mul(0x100000001b3);
                };
                mix(&mut hash, 0x04); // tag byte for Intersection
                for b in lhs.state.fingerprint().to_le_bytes() {
                    mix(&mut hash, b);
                }
                for b in rhs.state.fingerprint().to_le_bytes() {
                    mix(&mut hash, b);
                }
                hash
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
    /// Interned nodes by fingerprint. Collisions are possible but harmless:
    /// the materializer produces the correct mask regardless.
    nodes: HashMap<u64, Arc<ClipNode>>,
    /// Pre-allocated flyweight nodes for Full and Empty.
    full_node: Arc<ClipNode>,
    empty_node: Arc<ClipNode>,
    /// Statistics: total intern lookups.
    pub intern_lookups: u64,
    /// Statistics: cache hits (reuse instead of new allocation).
    pub intern_hits: u64,
    /// Statistics: new nodes created.
    pub nodes_created: u64,
}

/// Statistics snapshot for the clip DAG.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClipDagStats {
    pub interned_nodes: usize,
    pub intern_lookups: u64,
    pub intern_hits: u64,
    pub nodes_created: u64,
    pub approximate_bytes: usize,
}

impl ClipDag {
    /// Create a new empty DAG with flyweight Full/Empty nodes.
    pub fn new() -> Self {
        let full_node = Arc::new(ClipNode::new(ClipState::Full));
        let empty_node = Arc::new(ClipNode::new(ClipState::Empty));
        let mut nodes = HashMap::with_capacity(64);
        nodes.insert(ClipState::Full.fingerprint(), Arc::clone(&full_node));
        nodes.insert(ClipState::Empty.fingerprint(), Arc::clone(&empty_node));
        Self {
            nodes,
            full_node,
            empty_node,
            intern_lookups: 0,
            intern_hits: 0,
            nodes_created: 0,
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
        let fp = state.fingerprint();
        if let Some(existing) = self.nodes.get(&fp) {
            if existing.state == state {
                self.intern_hits += 1;
                return Arc::clone(existing);
            }
        }
        // New node.
        self.nodes_created += 1;
        let node = Arc::new(ClipNode::new(state));
        self.nodes.insert(fp, Arc::clone(&node));
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
            let ix1 = (x1 + w1).min(x2 + w2);
            let iy1 = (y1 + h1).min(y2 + h2);
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
        // General intersection node
        let state = ClipState::Intersection(Arc::clone(lhs), Arc::clone(rhs));
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
        if x <= 0 && y <= 0 && (x + w) >= buf_w as i32 && (y + h) >= buf_h as i32 {
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
            intern_lookups: self.intern_lookups,
            intern_hits: self.intern_hits,
            nodes_created: self.nodes_created,
            approximate_bytes,
        }
    }

    /// Evict nodes that are not referenced externally (only the DAG holds them).
    /// Returns the number of evicted entries.
    pub fn evict_unused(&mut self) -> usize {
        let before = self.nodes.len();
        self.nodes.retain(|fp, node| {
            // Keep flyweights always
            *fp == ClipState::Full.fingerprint()
                || *fp == ClipState::Empty.fingerprint()
                || Arc::strong_count(node) > 1
        });
        before - self.nodes.len()
    }

    /// Reset statistics counters.
    pub fn reset_stats(&mut self) {
        self.intern_lookups = 0;
        self.intern_hits = 0;
        self.nodes_created = 0;
    }
}

impl Default for ClipDag {
    fn default() -> Self {
        Self::new()
    }
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
    fn materialize_is_lazy_and_cached() {
        let mut dag = ClipDag::new();
        let node = dag.rectangle(10, 20, 50, 30, 100, 100);
        assert!(!node.is_materialized());
        let _mask = node.materialize(100, 100);
        assert!(node.is_materialized());
        // Second call returns same reference
        let m1 = node.materialize(100, 100) as *const ClipMask;
        let m2 = node.materialize(100, 100) as *const ClipMask;
        assert_eq!(m1, m2);
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
