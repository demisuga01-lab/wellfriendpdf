//! Canonical source identities and lazy document views used by renderer APIs.
//!
//! The PDF reader remains the single owner of document bytes. `CanonicalDocument`
//! coordinates stable identities over that source without duplicating semantic,
//! editing, OCR, or validation models during ordinary rendering.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use crate::content::ContentOperation;
use crate::document::{PdfDocument, PdfPage};
use crate::engine::{ContentEngine, PageResources};
use crate::error::Result;
use crate::render::{DisplayList, PixelBuffer, RenderMode, Viewport};

use super::contract::{ObjectIdentityId, ResourceId, RevisionId, SourceLinkId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectIdentity {
    pub id: ObjectIdentityId,
    pub number: u32,
    pub generation: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageIdentity {
    pub page_number: usize,
    pub object: ObjectIdentity,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ViewMaterializationStats {
    pub render_pages: usize,
    pub edit_pages: usize,
    pub semantic_pages: usize,
    pub validation_pages: usize,
}

#[derive(Default)]
struct ViewMaterializationCounters {
    render_pages: AtomicUsize,
    edit_pages: AtomicUsize,
    semantic_pages: AtomicUsize,
    validation_pages: AtomicUsize,
}

/// Shared, immutable identity coordinator for one opened source revision.
/// Original bytes remain owned by `PdfDocument`/`PdfReader`; this type records
/// their digest, length, xref identities, and lazily observed page identities.
#[derive(Clone)]
pub struct CanonicalDocument {
    fingerprint: [u8; 32],
    revision: RevisionId,
    original_byte_len: usize,
    object_identities: Arc<Vec<ObjectIdentity>>,
    pages: Arc<Mutex<HashMap<usize, PageIdentity>>>,
    counters: Arc<ViewMaterializationCounters>,
}

impl std::fmt::Debug for CanonicalDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanonicalDocument")
            .field("fingerprint", &self.fingerprint_hex())
            .field("revision", &self.revision)
            .field("original_byte_len", &self.original_byte_len)
            .field("object_count", &self.object_identities.len())
            .finish()
    }
}

impl CanonicalDocument {
    pub(crate) fn from_document(document: &PdfDocument) -> Self {
        let bytes = document.reader().file_bytes();
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        let revision = RevisionId(u64::from_le_bytes([
            digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
        ]));
        let object_identities = document
            .reader()
            .object_ids()
            .into_iter()
            .enumerate()
            .map(|(index, (number, generation))| ObjectIdentity {
                id: ObjectIdentityId(u32::try_from(index + 1).unwrap_or(u32::MAX)),
                number,
                generation,
            })
            .collect();
        Self {
            fingerprint: digest,
            revision,
            original_byte_len: bytes.len(),
            object_identities: Arc::new(object_identities),
            pages: Arc::new(Mutex::new(HashMap::new())),
            counters: Arc::new(ViewMaterializationCounters::default()),
        }
    }

    pub fn revision(&self) -> RevisionId {
        self.revision
    }

    pub fn original_byte_len(&self) -> usize {
        self.original_byte_len
    }

    pub fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }

    pub fn fingerprint_hex(&self) -> String {
        self.fingerprint
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub fn object_identities(&self) -> &[ObjectIdentity] {
        self.object_identities.as_slice()
    }

    pub fn view_materialization_stats(&self) -> ViewMaterializationStats {
        ViewMaterializationStats {
            render_pages: self.counters.render_pages.load(Ordering::Relaxed),
            edit_pages: self.counters.edit_pages.load(Ordering::Relaxed),
            semantic_pages: self.counters.semantic_pages.load(Ordering::Relaxed),
            validation_pages: self.counters.validation_pages.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn page_identity_for(&self, page: &PdfPage) -> PageIdentity {
        let mut pages = self.pages.lock().expect("canonical page identity mutex");
        *pages.entry(page.page_number).or_insert_with(|| PageIdentity {
            page_number: page.page_number,
            object: self
                .object_identities
                .iter()
                .copied()
                .find(|identity| {
                    identity.number == page.object_number
                        && identity.generation == page.generation_number
                })
                .unwrap_or(ObjectIdentity {
                    id: ObjectIdentityId(page.object_number),
                    number: page.object_number,
                    generation: page.generation_number,
                }),
        })
    }

    fn source_link_for_page(&self, page: &PdfPage) -> SourceLinkId {
        let identity = self.page_identity_for(page);
        SourceLinkId(identity.object.id.0)
    }

    fn resource_id_for_page(&self, page: &PdfPage) -> ResourceId {
        let identity = self.page_identity_for(page);
        ResourceId(identity.object.id.0)
    }

    fn record_render(&self) {
        self.counters.render_pages.fetch_add(1, Ordering::Relaxed);
    }

    fn record_edit(&self) {
        self.counters.edit_pages.fetch_add(1, Ordering::Relaxed);
    }

    fn record_semantic(&self) {
        self.counters.semantic_pages.fetch_add(1, Ordering::Relaxed);
    }

    fn record_validation(&self) {
        self.counters.validation_pages.fetch_add(1, Ordering::Relaxed);
    }
}

/// Lazily materialized decoded source program. Rich provenance stays in the
/// canonical view, while the renderer later compiles retained data into its own
/// packed plan.
#[derive(Clone, Debug)]
pub struct ParsedPageProgram {
    pub page: PageIdentity,
    pub source_link: SourceLinkId,
    pub resource_id: ResourceId,
    pub operations: Vec<ContentOperation>,
}

/// The rendering-only lazy view. Construction performs no page decode, OCR,
/// semantic analysis, transaction planning, or standards validation.
pub struct RenderDocumentView<'a> {
    engine: &'a ContentEngine,
}

impl<'a> RenderDocumentView<'a> {
    pub(crate) fn new(engine: &'a ContentEngine) -> Self {
        Self { engine }
    }

    pub fn canonical(&self) -> &'a CanonicalDocument {
        self.engine.canonical_document()
    }

    pub fn page_identity_for(&self, page_number: usize) -> Result<PageIdentity> {
        let page = self.engine.get_page(page_number)?;
        Ok(self.canonical().page_identity_for(&page))
    }

    pub fn viewport(&self, page_number: usize, dpi: u32) -> Result<Viewport> {
        self.canonical().record_render();
        self.engine.page_viewport(page_number, dpi)
    }

    pub fn page_resources(&self, page_number: usize) -> Result<PageResources> {
        self.canonical().record_render();
        self.engine.get_page_resources(page_number)
    }

    pub fn page_program(&self, page_number: usize) -> Result<ParsedPageProgram> {
        self.canonical().record_render();
        let page = self.engine.get_page(page_number)?;
        Ok(ParsedPageProgram {
            page: self.canonical().page_identity_for(&page),
            source_link: self.canonical().source_link_for_page(&page),
            resource_id: self.canonical().resource_id_for_page(&page),
            operations: self.engine.get_page_content(page_number)?,
        })
    }

    pub fn display_list(&self, page_number: usize, dpi: u32) -> Result<DisplayList> {
        self.canonical().record_render();
        self.engine.build_page_display_list(page_number, dpi)
    }

    pub fn render(&self, page_number: usize, dpi: u32, mode: RenderMode) -> Result<PixelBuffer> {
        self.canonical().record_render();
        self.engine.render_page_with_mode(page_number, dpi, mode)
    }
}

/// Lazy source/edit-oriented view sharing the canonical object and page IDs.
pub struct EditDocumentView<'a> {
    engine: &'a ContentEngine,
}

impl<'a> EditDocumentView<'a> {
    pub(crate) fn new(engine: &'a ContentEngine) -> Self {
        Self { engine }
    }

    pub fn canonical(&self) -> &'a CanonicalDocument {
        self.engine.canonical_document()
    }

    pub fn page_source_identity(&self, page_number: usize) -> Result<SourceLinkId> {
        self.canonical().record_edit();
        let page = self.engine.get_page(page_number)?;
        Ok(self.canonical().source_link_for_page(&page))
    }

    pub fn source_operations(&self, page_number: usize) -> Result<Vec<ContentOperation>> {
        self.canonical().record_edit();
        self.engine.get_page_content(page_number)
    }
}

/// Lazy semantic view. Semantic analysis is not constructed by `RenderDocumentView`.
pub struct SemanticDocumentView<'a> {
    engine: &'a ContentEngine,
}

impl<'a> SemanticDocumentView<'a> {
    pub(crate) fn new(engine: &'a ContentEngine) -> Self {
        Self { engine }
    }

    pub fn canonical(&self) -> &'a CanonicalDocument {
        self.engine.canonical_document()
    }

    pub fn structured_text(&self, page_number: usize) -> Result<String> {
        self.canonical().record_semantic();
        self.engine.get_page_text_structured(page_number)
    }
}

/// Lazy validation view. Validation work is performed only on explicit calls.
pub struct ValidationDocumentView<'a> {
    engine: &'a ContentEngine,
}

impl<'a> ValidationDocumentView<'a> {
    pub(crate) fn new(engine: &'a ContentEngine) -> Self {
        Self { engine }
    }

    pub fn canonical(&self) -> &'a CanonicalDocument {
        self.engine.canonical_document()
    }

    pub fn validate_page_access(&self, page_number: usize) -> Result<PageIdentity> {
        self.canonical().record_validation();
        let page = self.engine.get_page(page_number)?;
        Ok(self.canonical().page_identity_for(&page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuthorPageSize, PdfBuilder, TextStyle};

    fn engine() -> ContentEngine {
        let mut builder = PdfBuilder::new();
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("lazy view", 72.0, 720.0, &TextStyle::default())
            .expect("write test page");
        ContentEngine::open_bytes(builder.to_bytes().expect("serialize test PDF"))
            .expect("open test PDF")
    }

    #[test]
    fn constructing_render_view_does_not_materialize_other_views() {
        let engine = engine();
        let before = engine.canonical_document().view_materialization_stats();
        let view = engine.render_view();
        assert_eq!(before, view.canonical().view_materialization_stats());
        let _ = view.page_program(1).expect("render program");
        let after = engine.canonical_document().view_materialization_stats();
        assert!(after.render_pages > 0);
        assert_eq!(after.semantic_pages, 0);
        assert_eq!(after.validation_pages, 0);
        assert_eq!(after.edit_pages, 0);
    }

    #[test]
    fn edit_and_render_views_share_source_identity() {
        let engine = engine();
        let render = engine.render_view().page_program(1).expect("render program");
        let edit = engine
            .edit_view()
            .page_source_identity(1)
            .expect("edit source link");
        assert_eq!(render.source_link, edit);
        assert_eq!(render.page.object.id.0, edit.0);
    }

    #[test]
    fn changing_source_bytes_changes_canonical_revision() {
        let first = engine();
        let mut builder = PdfBuilder::new();
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("different source", 72.0, 720.0, &TextStyle::default())
            .expect("write page");
        let second = ContentEngine::open_bytes(builder.to_bytes().expect("serialize"))
            .expect("open");
        assert_ne!(
            first.canonical_document().revision(),
            second.canonical_document().revision()
        );
    }
}
