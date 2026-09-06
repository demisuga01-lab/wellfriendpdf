//! Renderer dependency graph and conservative cache invalidation.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::contract::{ObjectIdentityId, RevisionId};
use super::display_list::RenderTile;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct OrderedTile {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl From<RenderTile> for OrderedTile {
    fn from(tile: RenderTile) -> Self {
        Self {
            x: tile.x,
            y: tile.y,
            width: tile.width,
            height: tile.height,
        }
    }
}

impl From<OrderedTile> for RenderTile {
    fn from(tile: OrderedTile) -> Self {
        Self {
            x: tile.x,
            y: tile.y,
            width: tile.width,
            height: tile.height,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct OrderedPageTile {
    page_number: usize,
    tile: OrderedTile,
}

impl OrderedPageTile {
    fn new(page_number: usize, tile: RenderTile) -> Self {
        Self {
            page_number,
            tile: tile.into(),
        }
    }

    fn into_render_tuple(self) -> (usize, RenderTile) {
        (self.page_number, self.tile.into())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidationResult {
    pub previous_revision: RevisionId,
    pub current_revision: RevisionId,
    pub invalidated_pages: Vec<usize>,
    pub invalidated_tiles: Vec<(usize, RenderTile)>,
    pub cache_must_reset: bool,
}

/// Dependency graph from source identities to retained page artifacts and tiles.
/// It intentionally records conservative dependencies: an uncertain source
/// relation invalidates the full page rather than allowing a stale pixel.
#[derive(Clone, Debug)]
pub struct RenderDependencyGraph {
    revision: RevisionId,
    source_pages: BTreeMap<ObjectIdentityId, BTreeSet<usize>>,
    source_tiles: BTreeMap<ObjectIdentityId, BTreeSet<OrderedPageTile>>,
    source_dependents: BTreeMap<ObjectIdentityId, BTreeSet<ObjectIdentityId>>,
    page_tiles: BTreeMap<usize, BTreeSet<OrderedTile>>,
}

impl RenderDependencyGraph {
    pub fn new(revision: RevisionId) -> Self {
        Self {
            revision,
            source_pages: BTreeMap::new(),
            source_tiles: BTreeMap::new(),
            source_dependents: BTreeMap::new(),
            page_tiles: BTreeMap::new(),
        }
    }

    pub fn revision(&self) -> RevisionId {
        self.revision
    }

    pub fn advance_revision(&mut self, revision: RevisionId) {
        self.revision = revision;
    }

    pub fn record_page_source(&mut self, page_number: usize, source: ObjectIdentityId) {
        self.source_pages
            .entry(source)
            .or_default()
            .insert(page_number);
    }

    pub fn record_tile(&mut self, page_number: usize, tile: RenderTile) {
        self.page_tiles
            .entry(page_number)
            .or_default()
            .insert(tile.into());
    }

    pub fn record_source_tile(
        &mut self,
        page_number: usize,
        source: ObjectIdentityId,
        tile: RenderTile,
    ) {
        self.source_tiles
            .entry(source)
            .or_default()
            .insert(OrderedPageTile::new(page_number, tile));
        self.record_tile(page_number, tile);
    }

    pub fn record_source_dependency(
        &mut self,
        dependent_source: ObjectIdentityId,
        dependency_source: ObjectIdentityId,
    ) {
        if dependent_source == dependency_source {
            return;
        }
        self.source_dependents
            .entry(dependency_source)
            .or_default()
            .insert(dependent_source);
    }

    pub fn expand_changed_sources(
        &self,
        changed_sources: &[ObjectIdentityId],
    ) -> Vec<ObjectIdentityId> {
        let mut expanded = changed_sources.iter().copied().collect::<BTreeSet<_>>();
        let mut pending = expanded.iter().copied().collect::<Vec<_>>();
        while let Some(source) = pending.pop() {
            if let Some(dependents) = self.source_dependents.get(&source) {
                for dependent in dependents {
                    if expanded.insert(*dependent) {
                        pending.push(*dependent);
                    }
                }
            }
        }
        expanded.into_iter().collect()
    }

    fn page_recorded_tiles_are_covered(
        &self,
        page_number: usize,
        invalidated_tile_set: &BTreeSet<OrderedPageTile>,
    ) -> bool {
        self.page_tiles
            .get(&page_number)
            .map(|page_tiles| {
                page_tiles.iter().all(|tile| {
                    invalidated_tile_set.contains(&OrderedPageTile {
                        page_number,
                        tile: *tile,
                    })
                })
            })
            .unwrap_or(true)
    }

    fn expand_source_pages_without_full_tile_coverage(
        &self,
        source_derived_pages: &BTreeSet<usize>,
        invalidated_tile_set: &mut BTreeSet<OrderedPageTile>,
    ) {
        for page in source_derived_pages {
            if self.page_recorded_tiles_are_covered(*page, invalidated_tile_set) {
                continue;
            }
            if let Some(page_tiles) = self.page_tiles.get(page) {
                invalidated_tile_set.extend(page_tiles.iter().copied().map(|tile| {
                    OrderedPageTile {
                        page_number: *page,
                        tile,
                    }
                }));
            }
        }
    }

    pub fn invalidate_sources(
        &mut self,
        next_revision: RevisionId,
        changed_sources: &[ObjectIdentityId],
    ) -> InvalidationResult {
        self.invalidate_sources_and_pages(next_revision, changed_sources, &[])
    }

    pub fn invalidate_sources_and_pages(
        &mut self,
        next_revision: RevisionId,
        changed_sources: &[ObjectIdentityId],
        affected_pages: &[usize],
    ) -> InvalidationResult {
        self.invalidate_sources_pages_and_tiles(next_revision, changed_sources, affected_pages, &[])
    }

    pub fn invalidate_sources_pages_and_tiles(
        &mut self,
        next_revision: RevisionId,
        changed_sources: &[ObjectIdentityId],
        affected_pages: &[usize],
        affected_tiles: &[(usize, RenderTile)],
    ) -> InvalidationResult {
        let previous_revision = self.revision;
        let changed_sources = self.expand_changed_sources(changed_sources);
        let mut pages = affected_pages.iter().copied().collect::<BTreeSet<_>>();
        for source in &changed_sources {
            if let Some(source_pages) = self.source_pages.get(source) {
                pages.extend(source_pages.iter().copied());
            }
        }
        let invalidated_pages: Vec<_> = pages.into_iter().collect();
        let mut invalidated_tile_set = invalidated_pages
            .iter()
            .flat_map(|page| {
                self.page_tiles
                    .get(page)
                    .into_iter()
                    .flat_map(move |tiles| {
                        tiles.iter().copied().map(move |tile| OrderedPageTile {
                            page_number: *page,
                            tile,
                        })
                    })
            })
            .collect::<BTreeSet<_>>();
        invalidated_tile_set.extend(
            affected_tiles
                .iter()
                .map(|(page_number, tile)| OrderedPageTile::new(*page_number, *tile)),
        );
        for source in &changed_sources {
            if let Some(source_tiles) = self.source_tiles.get(source) {
                invalidated_tile_set.extend(source_tiles.iter().copied());
            }
        }
        let invalidated_tiles = invalidated_tile_set
            .into_iter()
            .map(OrderedPageTile::into_render_tuple)
            .collect::<Vec<_>>();
        self.advance_revision(next_revision);
        let cache_must_reset = previous_revision != next_revision
            && invalidated_pages.is_empty()
            && invalidated_tiles.is_empty();
        InvalidationResult {
            previous_revision,
            current_revision: next_revision,
            invalidated_pages,
            invalidated_tiles,
            cache_must_reset,
        }
    }

    pub fn invalidate_sources_page_artifacts_and_exact_tiles(
        &mut self,
        next_revision: RevisionId,
        changed_sources: &[ObjectIdentityId],
        affected_pages: &[usize],
        affected_tiles: &[(usize, RenderTile)],
    ) -> InvalidationResult {
        let previous_revision = self.revision;
        let changed_sources = self.expand_changed_sources(changed_sources);
        let mut pages = affected_pages.iter().copied().collect::<BTreeSet<_>>();
        let mut source_derived_pages = BTreeSet::new();
        for source in &changed_sources {
            if let Some(source_pages) = self.source_pages.get(source) {
                pages.extend(source_pages.iter().copied());
                source_derived_pages.extend(source_pages.iter().copied());
            }
        }
        let invalidated_pages: Vec<_> = pages.into_iter().collect();
        let mut invalidated_tile_set = affected_tiles
            .iter()
            .map(|(page_number, tile)| OrderedPageTile::new(*page_number, *tile))
            .collect::<BTreeSet<_>>();
        for source in &changed_sources {
            if let Some(source_tiles) = self.source_tiles.get(source) {
                invalidated_tile_set.extend(source_tiles.iter().copied());
            }
        }
        self.expand_source_pages_without_full_tile_coverage(
            &source_derived_pages,
            &mut invalidated_tile_set,
        );
        let invalidated_tiles = invalidated_tile_set
            .into_iter()
            .map(OrderedPageTile::into_render_tuple)
            .collect::<Vec<_>>();
        self.advance_revision(next_revision);
        let cache_must_reset = previous_revision != next_revision
            && invalidated_pages.is_empty()
            && invalidated_tiles.is_empty();
        InvalidationResult {
            previous_revision,
            current_revision: next_revision,
            invalidated_pages,
            invalidated_tiles,
            cache_must_reset,
        }
    }

    pub fn reset_revision(&mut self, revision: RevisionId) {
        self.revision = revision;
        self.source_pages.clear();
        self.source_tiles.clear();
        self.source_dependents.clear();
        self.page_tiles.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_source_invalidates_only_its_recorded_page_tiles() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        graph.record_page_source(1, ObjectIdentityId(10));
        graph.record_page_source(2, ObjectIdentityId(20));
        graph.record_tile(
            1,
            RenderTile {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            },
        );
        graph.record_tile(
            2,
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );
        let result = graph.invalidate_sources(RevisionId(2), &[ObjectIdentityId(10)]);
        assert_eq!(result.invalidated_pages, vec![1]);
        assert_eq!(result.invalidated_tiles.len(), 1);
        assert_eq!(result.invalidated_tiles[0].0, 1);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn changed_source_can_invalidate_tile_without_full_page_artifacts() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        let tile = RenderTile {
            x: 128,
            y: 0,
            width: 64,
            height: 64,
        };
        graph.record_source_tile(1, ObjectIdentityId(10), tile);

        let result = graph.invalidate_sources(RevisionId(2), &[ObjectIdentityId(10)]);

        assert!(result.invalidated_pages.is_empty());
        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn page_and_source_tile_invalidation_deduplicates_tiles() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        };
        graph.record_page_source(1, ObjectIdentityId(10));
        graph.record_source_tile(1, ObjectIdentityId(10), tile);

        let result = graph.invalidate_sources(RevisionId(2), &[ObjectIdentityId(10)]);

        assert_eq!(result.invalidated_pages, vec![1]);
        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn changed_shared_resource_invalidates_dependent_source_tiles() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        let tile = RenderTile {
            x: 16,
            y: 16,
            width: 32,
            height: 32,
        };
        graph.record_source_tile(1, ObjectIdentityId(10), tile);
        graph.record_source_dependency(ObjectIdentityId(10), ObjectIdentityId(20));

        let result = graph.invalidate_sources(RevisionId(2), &[ObjectIdentityId(20)]);

        assert!(result.invalidated_pages.is_empty());
        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn changed_shared_resource_expands_transitive_dependents() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        let tile = RenderTile {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        };
        graph.record_source_tile(1, ObjectIdentityId(10), tile);
        graph.record_source_dependency(ObjectIdentityId(10), ObjectIdentityId(20));
        graph.record_source_dependency(ObjectIdentityId(20), ObjectIdentityId(30));

        assert_eq!(
            graph.expand_changed_sources(&[ObjectIdentityId(30)]),
            vec![
                ObjectIdentityId(10),
                ObjectIdentityId(20),
                ObjectIdentityId(30)
            ]
        );

        let result = graph.invalidate_sources(RevisionId(2), &[ObjectIdentityId(30)]);

        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn explicit_affected_tile_invalidates_without_full_page_artifacts() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        let tile = RenderTile {
            x: 32,
            y: 32,
            width: 32,
            height: 32,
        };

        let result =
            graph.invalidate_sources_pages_and_tiles(RevisionId(2), &[], &[], &[(1, tile)]);

        assert!(result.invalidated_pages.is_empty());
        assert_eq!(result.invalidated_tiles, vec![(1, tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn exact_tile_mode_keeps_page_artifacts_without_expanding_all_page_tiles() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        let dirty_tile = RenderTile {
            x: 32,
            y: 0,
            width: 32,
            height: 32,
        };
        graph.record_tile(
            2,
            RenderTile {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            },
        );
        graph.record_tile(2, dirty_tile);

        let result = graph.invalidate_sources_page_artifacts_and_exact_tiles(
            RevisionId(2),
            &[],
            &[2],
            &[(2, dirty_tile)],
        );

        assert_eq!(result.invalidated_pages, vec![2]);
        assert_eq!(result.invalidated_tiles, vec![(2, dirty_tile)]);
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn exact_tile_mode_expands_source_pages_without_exact_tile_coverage() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        let page_1_a = RenderTile {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        };
        let page_1_b = RenderTile {
            x: 32,
            y: 0,
            width: 32,
            height: 32,
        };
        let page_2_dirty = RenderTile {
            x: 0,
            y: 32,
            width: 32,
            height: 32,
        };
        graph.record_page_source(1, ObjectIdentityId(10));
        graph.record_tile(1, page_1_a);
        graph.record_tile(1, page_1_b);

        let result = graph.invalidate_sources_page_artifacts_and_exact_tiles(
            RevisionId(2),
            &[ObjectIdentityId(10)],
            &[],
            &[(2, page_2_dirty)],
        );

        assert_eq!(result.invalidated_pages, vec![1]);
        assert_eq!(
            result.invalidated_tiles,
            vec![(1, page_1_a), (1, page_1_b), (2, page_2_dirty)]
        );
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn exact_tile_mode_expands_source_pages_with_partial_source_tile_coverage() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        let page_1_source_tile = RenderTile {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        };
        let page_1_recorded_tile = RenderTile {
            x: 32,
            y: 0,
            width: 32,
            height: 32,
        };
        let page_2_dirty = RenderTile {
            x: 0,
            y: 32,
            width: 32,
            height: 32,
        };
        graph.record_page_source(1, ObjectIdentityId(10));
        graph.record_source_tile(1, ObjectIdentityId(10), page_1_source_tile);
        graph.record_tile(1, page_1_recorded_tile);

        let result = graph.invalidate_sources_page_artifacts_and_exact_tiles(
            RevisionId(2),
            &[ObjectIdentityId(10)],
            &[],
            &[(2, page_2_dirty)],
        );

        assert_eq!(result.invalidated_pages, vec![1]);
        assert_eq!(
            result.invalidated_tiles,
            vec![
                (1, page_1_source_tile),
                (1, page_1_recorded_tile),
                (2, page_2_dirty),
            ]
        );
        assert!(!result.cache_must_reset);
    }

    #[test]
    fn exact_tile_mode_keeps_source_tiles_when_they_cover_recorded_page_tiles() {
        let mut graph = RenderDependencyGraph::new(RevisionId(1));
        let page_1_a = RenderTile {
            x: 0,
            y: 0,
            width: 32,
            height: 32,
        };
        let page_1_b = RenderTile {
            x: 32,
            y: 0,
            width: 32,
            height: 32,
        };
        let page_2_dirty = RenderTile {
            x: 0,
            y: 32,
            width: 32,
            height: 32,
        };
        graph.record_page_source(1, ObjectIdentityId(10));
        graph.record_source_tile(1, ObjectIdentityId(10), page_1_a);
        graph.record_source_tile(1, ObjectIdentityId(10), page_1_b);

        let result = graph.invalidate_sources_page_artifacts_and_exact_tiles(
            RevisionId(2),
            &[ObjectIdentityId(10)],
            &[],
            &[(2, page_2_dirty)],
        );

        assert_eq!(result.invalidated_pages, vec![1]);
        assert_eq!(
            result.invalidated_tiles,
            vec![(1, page_1_a), (1, page_1_b), (2, page_2_dirty)]
        );
        assert!(!result.cache_must_reset);
    }
}
