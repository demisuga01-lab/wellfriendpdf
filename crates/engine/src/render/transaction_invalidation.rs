//! Transaction-driven narrow dependency invalidation (RB-02).
//!
//! This module bridges editing transaction reports (which carry `affected_objects`
//! and `affected_pages`) into the render cache invalidation graph. Callers invoke
//! [`TransactionWriteSet::invalidate`] after a source edit to evict only the
//! page/tile caches whose dependencies are proven affected, while unknown object
//! references trigger a conservative full reset.

use serde::{Deserialize, Serialize};

use super::contract::{ObjectIdentityId, RevisionId};
use super::invalidation::InvalidationResult;
use super::page_renderer::RenderDocumentCache;
use crate::render::document_view::ObjectIdentity;

/// A typed write-set produced by an editing transaction, suitable for driving
/// narrow cache invalidation. Callers build this from the transaction report's
/// `affected_objects` and `affected_pages`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionWriteSet {
    /// Object reference strings from the transaction report (e.g. "4 0 R").
    pub affected_object_refs: Vec<String>,
    /// Page numbers the transaction reported as affected.
    pub affected_pages: Vec<usize>,
    /// Next document revision (derived from output bytes hash).
    pub next_revision: RevisionId,
}

/// Result of applying a transaction write-set to the render cache.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransactionInvalidationResult {
    /// The underlying invalidation result from the dependency graph.
    pub invalidation: InvalidationResult,
    /// Object refs that could not be mapped to canonical IDs (triggers conservative reset).
    pub unmapped_refs: Vec<String>,
    /// Object refs that were successfully mapped.
    pub mapped_ids: Vec<ObjectIdentityId>,
}

/// Parse a PDF object reference string like "4 0 R" into (number, generation).
fn parse_object_ref(ref_str: &str) -> Option<(u32, u16)> {
    let parts: Vec<&str> = ref_str.split_whitespace().collect();
    if parts.len() >= 2 {
        let number = parts[0].parse::<u32>().ok()?;
        let generation = parts[1].parse::<u16>().ok()?;
        Some((number, generation))
    } else {
        None
    }
}

/// Map object reference strings to canonical ObjectIdentityIds using the
/// document's identity table. Returns (mapped_ids, unmapped_refs).
pub fn map_refs_to_canonical_ids(
    object_refs: &[String],
    identities: &[ObjectIdentity],
) -> (Vec<ObjectIdentityId>, Vec<String>) {
    let mut mapped = Vec::new();
    let mut unmapped = Vec::new();
    for ref_str in object_refs {
        if let Some((number, generation)) = parse_object_ref(ref_str) {
            if let Some(identity) = identities
                .iter()
                .find(|id| id.number == number && id.generation == generation)
            {
                mapped.push(identity.id);
            } else {
                unmapped.push(ref_str.clone());
            }
        } else {
            unmapped.push(ref_str.clone());
        }
    }
    (mapped, unmapped)
}

impl TransactionWriteSet {
    /// Build a write-set from an editing transaction report's fields.
    pub fn from_transaction_report(
        affected_objects: &[String],
        affected_pages: &[usize],
        next_revision: RevisionId,
    ) -> Self {
        Self {
            affected_object_refs: affected_objects.to_vec(),
            affected_pages: affected_pages.to_vec(),
            next_revision,
        }
    }

    /// Apply this write-set to a render document cache, using the provided
    /// object identity table to map refs to canonical IDs.
    ///
    /// Known dependencies are narrowly evicted. If any object refs cannot be
    /// mapped (unknown dependencies), the cache performs a conservative full
    /// reset to prevent stale pixels.
    pub fn invalidate(
        &self,
        cache: &mut RenderDocumentCache,
        identities: &[ObjectIdentity],
    ) -> TransactionInvalidationResult {
        let (mapped_ids, unmapped_refs) =
            map_refs_to_canonical_ids(&self.affected_object_refs, identities);

        // If we have unmapped refs, we cannot prove what pages they affect —
        // force a conservative full cache reset.
        if !unmapped_refs.is_empty() {
            let invalidation = cache.invalidate_sources(self.next_revision, &[]);
            // The empty changed_sources with a new revision triggers cache_must_reset
            // in the dependency graph (revision changes but no pages found).
            return TransactionInvalidationResult {
                invalidation,
                unmapped_refs,
                mapped_ids,
            };
        }

        let invalidation = cache.invalidate_sources(self.next_revision, &mapped_ids);
        TransactionInvalidationResult {
            invalidation,
            unmapped_refs,
            mapped_ids,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::display_list::RenderTile;
    use crate::render::page_renderer::RenderDocumentCache;

    fn make_identities() -> Vec<ObjectIdentity> {
        vec![
            ObjectIdentity {
                id: ObjectIdentityId(1),
                number: 1,
                generation: 0,
            },
            ObjectIdentity {
                id: ObjectIdentityId(2),
                number: 2,
                generation: 0,
            },
            ObjectIdentity {
                id: ObjectIdentityId(3),
                number: 3,
                generation: 0,
            },
            ObjectIdentity {
                id: ObjectIdentityId(4),
                number: 4,
                generation: 0,
            },
            ObjectIdentity {
                id: ObjectIdentityId(5),
                number: 5,
                generation: 0,
            },
        ]
    }

    #[test]
    fn parse_object_ref_parses_standard_format() {
        assert_eq!(parse_object_ref("4 0 R"), Some((4, 0)));
        assert_eq!(parse_object_ref("12 1 R"), Some((12, 1)));
        assert_eq!(parse_object_ref("4 0"), Some((4, 0)));
    }

    #[test]
    fn parse_object_ref_returns_none_for_invalid() {
        assert_eq!(parse_object_ref("abc"), None);
        assert_eq!(parse_object_ref(""), None);
        assert_eq!(parse_object_ref("4"), None);
    }

    #[test]
    fn map_refs_resolves_known_and_flags_unknown() {
        let identities = make_identities();
        let refs = vec![
            "4 0 R".to_string(),
            "99 0 R".to_string(), // unknown
        ];
        let (mapped, unmapped) = map_refs_to_canonical_ids(&refs, &identities);
        assert_eq!(mapped, vec![ObjectIdentityId(4)]);
        assert_eq!(unmapped, vec!["99 0 R".to_string()]);
    }

    #[test]
    fn local_edit_invalidates_page_1_retains_page_2_cache() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));

        // Record dependencies: object 4 -> page 1, object 5 -> page 2
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        cache.record_page_source_dependency(2, ObjectIdentityId(5));
        cache.record_tile_dependency(
            1,
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );
        cache.record_tile_dependency(
            2,
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );

        let identities = make_identities();

        // Transaction affects only object 4 (content stream for page 1)
        let write_set = TransactionWriteSet::from_transaction_report(
            &["4 0 R".to_string()],
            &[1],
            RevisionId(2),
        );

        let result = write_set.invalidate(&mut cache, &identities);

        // Page 1 should be invalidated
        assert!(result.invalidation.invalidated_pages.contains(&1));
        // Page 2 should NOT be invalidated
        assert!(!result.invalidation.invalidated_pages.contains(&2));
        // No unmapped refs
        assert!(result.unmapped_refs.is_empty());
        // Not a full reset
        assert!(!result.invalidation.cache_must_reset);
    }

    #[test]
    fn unknown_object_triggers_conservative_reset() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.record_page_source_dependency(1, ObjectIdentityId(4));
        cache.record_tile_dependency(
            1,
            RenderTile {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
        );

        let identities = make_identities();

        // Transaction reports an object ref that doesn't exist in our identity table
        let write_set = TransactionWriteSet::from_transaction_report(
            &["99 0 R".to_string()],
            &[1],
            RevisionId(2),
        );

        let result = write_set.invalidate(&mut cache, &identities);

        // Must do conservative reset because we can't map the ref
        assert!(result.invalidation.cache_must_reset);
        assert_eq!(result.unmapped_refs, vec!["99 0 R".to_string()]);
    }

    #[test]
    fn mixed_known_and_unknown_refs_trigger_conservative_reset() {
        let mut cache = RenderDocumentCache::new();
        cache.bind_document_revision(RevisionId(1));
        cache.record_page_source_dependency(1, ObjectIdentityId(4));

        let identities = make_identities();

        let write_set = TransactionWriteSet::from_transaction_report(
            &["4 0 R".to_string(), "unknown_ref".to_string()],
            &[1],
            RevisionId(2),
        );

        let result = write_set.invalidate(&mut cache, &identities);

        // Even though object 4 is known, the unknown ref forces conservative reset
        assert!(result.invalidation.cache_must_reset);
        assert_eq!(result.unmapped_refs, vec!["unknown_ref".to_string()]);
    }
}
