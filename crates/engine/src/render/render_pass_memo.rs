//! Page-constant inputs of the renderer's program cache keys, computed once
//! per render pass.
//!
//! PDFium renders a page's content and its annotation appearance streams
//! through one `CPDF_RenderContext` (`CPDF_ProgressiveRenderer` followed by
//! `CPDF_AnnotList::DisplayAnnots` into the same context), so everything that
//! only depends on the page — its resources, its identity — is looked up
//! once. Wellfriend's annotation pass already runs on the page's render
//! state and output buffer, but every annotation appearance, soft-mask group
//! and Type3 glyph program cache key re-hashed the whole page resource
//! dictionary (fonts with their width arrays, XObject dictionaries, colour
//! spaces...), and every retained appearance plan re-loaded the page (cloning
//! its `/Annots` array) to find the page identity. On a form page with a few
//! hundred widgets that was a third of the annotation pass.
//!
//! [`RenderPassMemo`] holds those values for the life of one render pass. It
//! is shared (by `Arc`) between the page state and the nested states it
//! spawns for groups and soft masks, which render the same page with the same
//! immutable page resources.

use std::sync::OnceLock;

use crate::render::contract::ObjectIdentityId;

#[derive(Debug, Default)]
pub(crate) struct RenderPassMemo {
    /// Fingerprint of the page's (immutable) resource scope.
    page_resources_fingerprint: OnceLock<u64>,
    /// Canonical identity of the page, `None` when the page cannot be read.
    page_identity: OnceLock<Option<ObjectIdentityId>>,
}

impl RenderPassMemo {
    pub(crate) fn page_resources_fingerprint(&self, compute: impl FnOnce() -> u64) -> u64 {
        *self.page_resources_fingerprint.get_or_init(compute)
    }

    pub(crate) fn page_identity(
        &self,
        compute: impl FnOnce() -> Option<ObjectIdentityId>,
    ) -> Option<ObjectIdentityId> {
        *self.page_identity.get_or_init(compute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_computed_once() {
        let memo = RenderPassMemo::default();
        let mut calls = 0;
        assert_eq!(
            memo.page_resources_fingerprint(|| {
                calls += 1;
                7
            }),
            7
        );
        assert_eq!(memo.page_resources_fingerprint(|| unreachable!()), 7);
        assert_eq!(calls, 1);
        assert_eq!(memo.page_identity(|| None), None);
        assert_eq!(memo.page_identity(|| unreachable!()), None);
    }
}
