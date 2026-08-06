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
    page_tiles: BTreeMap<usize, BTreeSet<OrderedTile>>,
}

impl RenderDependencyGraph {
    pub fn new(revision: RevisionId) -> Self {
        Self {
            revision,
            source_pages: BTreeMap::new(),
            page_tiles: BTreeMap::new(),
        }
    }

    pub fn revision(&self) -> RevisionId {
        self.revision
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

    pub fn invalidate_sources(
        &mut self,
        next_revision: RevisionId,
        changed_sources: &[ObjectIdentityId],
    ) -> InvalidationResult {
        let previous_revision = self.revision;
        let mut pages = BTreeSet::new();
        for source in changed_sources {
            if let Some(source_pages) = self.source_pages.get(source) {
                pages.extend(source_pages.iter().copied());
            }
        }
        let invalidated_pages: Vec<_> = pages.into_iter().collect();
        let invalidated_tiles = invalidated_pages
            .iter()
            .flat_map(|page| {
                self.page_tiles
                    .get(page)
                    .into_iter()
                    .flat_map(move |tiles| {
                        tiles.iter().copied().map(move |tile| (*page, tile.into()))
                    })
            })
            .collect();
        self.revision = next_revision;
        let cache_must_reset = previous_revision != next_revision && invalidated_pages.is_empty();
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
}
