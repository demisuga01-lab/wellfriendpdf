//! Canonical source identities and lazy document views used by renderer APIs.
//!
//! The PDF reader remains the single owner of document bytes. `CanonicalDocument`
//! coordinates stable identities over that source without duplicating semantic,
//! editing, OCR, or validation models during ordinary rendering.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::content::{ContentOperation, Operand};
use crate::document::{PdfDocument, PdfPage};
use crate::engine::{ContentEngine, PageResources, RenderContractTelemetryReport};
use crate::error::Result;
use crate::object::PdfObject;
use crate::render::{
    DisplayList, FontSubstitutionLog, GraphicsStateDescriptor, MarkedContentProperties,
    NativeDescriptor, PageBox, PixelBuffer, RenderContract, RenderMode, RenderPlan, Viewport,
};
use crate::CancelToken;

use super::contract::{ObjectIdentityId, ResourceId, RevisionId, SourceLinkId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct ObjectIdentity {
    pub id: ObjectIdentityId,
    pub number: u32,
    pub generation: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct PageIdentity {
    pub page_number: usize,
    pub object: ObjectIdentity,
}

pub const DOCUMENT_VIEWS_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ViewMaterializationStats {
    pub render_pages: usize,
    pub edit_pages: usize,
    pub semantic_pages: usize,
    pub validation_pages: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DocumentViewBoundary {
    pub name: String,
    pub role: String,
    pub lazy_materialization_trigger: Vec<String>,
    pub active_entry_points: Vec<String>,
    pub constructs_other_views: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DocumentViewsReport {
    pub schema_version: u32,
    pub document_revision: u64,
    pub source_fingerprint: String,
    pub original_byte_len: usize,
    pub object_identity_count: usize,
    pub page_count: usize,
    pub canonical_identity_owner: String,
    pub view_construction: String,
    pub materialization: ViewMaterializationStats,
    pub views: Vec<DocumentViewBoundary>,
    pub owned_backend_plan_arena_entry_point: String,
    pub remaining_limitation: String,
}

pub const BACKEND_PLAN_ARENA_REPORT_SCHEMA_VERSION: u32 = 1;
pub const BACKEND_DOCUMENT_PLAN_ARENA_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BackendPlanResourceArenaEntries {
    pub font: usize,
    pub image: usize,
    pub form: usize,
    pub type3: usize,
    pub pattern: usize,
    pub shading: usize,
    pub group: usize,
    pub appearance: usize,
    pub color_space: usize,
    pub ext_g_state: usize,
    pub properties: usize,
    pub inline_image: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BackendPlanArenaReport {
    pub schema_version: u32,
    pub page_number: usize,
    pub document_revision: u64,
    pub render_contract_fingerprint: String,
    pub arena_kind: String,
    pub execution_mode: String,
    pub backend_selection: String,
    pub print_profile: String,
    pub output_surface: String,
    pub source_operation_count: usize,
    pub hot_operation_count: usize,
    pub folded_noop_state_ops: usize,
    pub folded_duplicate_state_ops: usize,
    pub folded_overwritten_state_ops: usize,
    pub path_arena_entries: usize,
    pub clip_transform_arena_entries: usize,
    pub state_arena_entries: usize,
    pub bounds_arena_entries: usize,
    pub descriptor_arena_entries: usize,
    pub resource_arena_entries: BackendPlanResourceArenaEntries,
    pub cold_diagnostics: usize,
    pub batch_count: usize,
    pub native_payload_batches: usize,
    pub vector_batches: usize,
    pub requires_native_replay: bool,
    pub descriptor_kinds: BTreeMap<String, usize>,
    pub compile_refusals: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BackendDocumentPlanArenaReport {
    pub schema_version: u32,
    pub document_revision: u64,
    pub source_fingerprint: String,
    pub dpi: u32,
    pub render_mode: String,
    pub arena_kind: String,
    pub print_profile: String,
    pub page_count: usize,
    pub owned_page_plan_count: usize,
    pub owns_backend_plans: bool,
    pub total_source_operation_count: usize,
    pub total_hot_operation_count: usize,
    pub total_folded_noop_state_ops: usize,
    pub total_folded_duplicate_state_ops: usize,
    pub total_folded_overwritten_state_ops: usize,
    pub total_descriptor_arena_entries: usize,
    pub total_resource_arena_entries: BackendPlanResourceArenaEntries,
    pub total_batch_count: usize,
    pub total_native_payload_batches: usize,
    pub total_vector_batches: usize,
    pub pages_requiring_native_replay: usize,
    pub descriptor_kinds: BTreeMap<String, usize>,
    pub compile_refusals: BTreeMap<String, usize>,
    pub pages: Vec<BackendPlanArenaReport>,
}

#[derive(Clone, Debug)]
pub struct BackendDocumentPagePlan {
    pub page_number: usize,
    pub page_identity: PageIdentity,
    pub source_link: SourceLinkId,
    pub resource_id: ResourceId,
    pub plan: RenderPlan,
}

#[derive(Clone, Debug)]
pub struct BackendDocumentPlanArena {
    pub document_revision: RevisionId,
    pub source_fingerprint: String,
    pub dpi: u32,
    pub render_mode: RenderMode,
    pub arena_kind: String,
    pub print_profile: String,
    pub pages: Vec<BackendDocumentPagePlan>,
}

impl BackendPlanArenaReport {
    fn from_plan(page_number: usize, document_revision: u64, plan: &RenderPlan) -> Self {
        let mut descriptor_kinds = BTreeMap::<String, usize>::new();
        let mut compile_refusals = BTreeMap::<String, usize>::new();
        let mut resource_arena_entries = BackendPlanResourceArenaEntries::default();
        for descriptor in &plan.packed.descriptors {
            let kind = native_descriptor_kind(descriptor);
            *descriptor_kinds.entry(kind.to_string()).or_insert(0) += 1;
            count_descriptor_resource_arena_entries(descriptor, &mut resource_arena_entries);
            if let NativeDescriptor::CompileRefusal(reason) = descriptor {
                *compile_refusals.entry(reason.to_string()).or_insert(0) += 1;
            }
        }
        let native_payload_batches = plan
            .batches
            .iter()
            .filter(|batch| batch.contains_native_payload)
            .count();
        Self {
            schema_version: BACKEND_PLAN_ARENA_REPORT_SCHEMA_VERSION,
            page_number,
            document_revision,
            render_contract_fingerprint: plan.contract.cache_fingerprint(),
            arena_kind: backend_arena_kind(&plan.contract).to_string(),
            execution_mode: format!("{:?}", plan.contract.execution_mode),
            backend_selection: format!("{:?}", plan.contract.backend),
            print_profile: format!("{:?}", plan.contract.print_profile),
            output_surface: backend_output_surface(&plan.contract).to_string(),
            source_operation_count: plan.packed.optimization_report.source_operation_count,
            hot_operation_count: plan.packed.hot_ops.len(),
            folded_noop_state_ops: plan.packed.optimization_report.folded_noop_state_ops,
            folded_duplicate_state_ops: plan.packed.optimization_report.folded_duplicate_state_ops,
            folded_overwritten_state_ops: plan
                .packed
                .optimization_report
                .folded_overwritten_state_ops,
            path_arena_entries: plan.packed.paths.len(),
            clip_transform_arena_entries: plan.packed.clip_transforms.len(),
            state_arena_entries: plan.packed.states.len(),
            bounds_arena_entries: plan.packed.bounds.len(),
            descriptor_arena_entries: plan.packed.descriptors.len(),
            resource_arena_entries,
            cold_diagnostics: plan.packed.cold.diagnostics.len(),
            batch_count: plan.batches.len(),
            native_payload_batches,
            vector_batches: plan.batches.len().saturating_sub(native_payload_batches),
            requires_native_replay: plan.packed.requires_native_replay(),
            descriptor_kinds,
            compile_refusals,
        }
    }
}

impl BackendDocumentPlanArena {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn plan_for_page(&self, page_number: usize) -> Option<&RenderPlan> {
        self.pages
            .iter()
            .find(|page| page.page_number == page_number)
            .map(|page| &page.plan)
    }

    pub fn report(&self) -> BackendDocumentPlanArenaReport {
        let mut descriptor_kinds = BTreeMap::<String, usize>::new();
        let mut compile_refusals = BTreeMap::<String, usize>::new();
        let mut total_source_operation_count = 0usize;
        let mut total_hot_operation_count = 0usize;
        let mut total_folded_noop_state_ops = 0usize;
        let mut total_folded_duplicate_state_ops = 0usize;
        let mut total_folded_overwritten_state_ops = 0usize;
        let mut total_descriptor_arena_entries = 0usize;
        let mut total_resource_arena_entries = BackendPlanResourceArenaEntries::default();
        let mut total_batch_count = 0usize;
        let mut total_native_payload_batches = 0usize;
        let mut total_vector_batches = 0usize;
        let mut pages_requiring_native_replay = 0usize;
        let pages = self
            .pages
            .iter()
            .map(|page| {
                let report = BackendPlanArenaReport::from_plan(
                    page.page_number,
                    self.document_revision.0,
                    &page.plan,
                );
                total_source_operation_count += report.source_operation_count;
                total_hot_operation_count += report.hot_operation_count;
                total_folded_noop_state_ops += report.folded_noop_state_ops;
                total_folded_duplicate_state_ops += report.folded_duplicate_state_ops;
                total_folded_overwritten_state_ops += report.folded_overwritten_state_ops;
                total_descriptor_arena_entries += report.descriptor_arena_entries;
                total_resource_arena_entries.add_assign(&report.resource_arena_entries);
                total_batch_count += report.batch_count;
                total_native_payload_batches += report.native_payload_batches;
                total_vector_batches += report.vector_batches;
                if report.requires_native_replay {
                    pages_requiring_native_replay += 1;
                }
                for (kind, count) in &report.descriptor_kinds {
                    *descriptor_kinds.entry(kind.clone()).or_insert(0) += *count;
                }
                for (reason, count) in &report.compile_refusals {
                    *compile_refusals.entry(reason.clone()).or_insert(0) += *count;
                }
                report
            })
            .collect::<Vec<_>>();

        BackendDocumentPlanArenaReport {
            schema_version: BACKEND_DOCUMENT_PLAN_ARENA_REPORT_SCHEMA_VERSION,
            document_revision: self.document_revision.0,
            source_fingerprint: self.source_fingerprint.clone(),
            dpi: self.dpi,
            render_mode: self.render_mode.as_str().to_string(),
            arena_kind: self.arena_kind.clone(),
            print_profile: self.print_profile.clone(),
            page_count: pages.len(),
            owned_page_plan_count: self.pages.len(),
            owns_backend_plans: true,
            total_source_operation_count,
            total_hot_operation_count,
            total_folded_noop_state_ops,
            total_folded_duplicate_state_ops,
            total_folded_overwritten_state_ops,
            total_descriptor_arena_entries,
            total_resource_arena_entries,
            total_batch_count,
            total_native_payload_batches,
            total_vector_batches,
            pages_requiring_native_replay,
            descriptor_kinds,
            compile_refusals,
            pages,
        }
    }
}

impl BackendPlanResourceArenaEntries {
    fn add_assign(&mut self, other: &Self) {
        self.font += other.font;
        self.image += other.image;
        self.form += other.form;
        self.type3 += other.type3;
        self.pattern += other.pattern;
        self.shading += other.shading;
        self.group += other.group;
        self.appearance += other.appearance;
        self.color_space += other.color_space;
        self.ext_g_state += other.ext_g_state;
        self.properties += other.properties;
        self.inline_image += other.inline_image;
    }
}

fn count_descriptor_resource_arena_entries(
    descriptor: &NativeDescriptor,
    counts: &mut BackendPlanResourceArenaEntries,
) {
    match descriptor {
        NativeDescriptor::Text(_) | NativeDescriptor::CompileRefusal(_) => {}
        NativeDescriptor::Image(desc) => {
            if let Some(handle) = &desc.handle {
                counts.image += 1;
                if handle.image_color_space.is_some() {
                    counts.color_space += 1;
                }
            }
        }
        NativeDescriptor::Form(desc) => {
            if let Some(handle) = &desc.handle {
                counts.form += 1;
                if xobject_handle_has_transparency_group(handle.stream_dict.as_ref()) {
                    counts.group += 1;
                }
            }
        }
        NativeDescriptor::Shading(desc) => {
            if desc.object.is_some() {
                counts.shading += 1;
            }
        }
        NativeDescriptor::State(state) => count_state_resource_arena_entries(state, counts),
        NativeDescriptor::Pattern(_) => counts.pattern += 1,
        NativeDescriptor::InlineImage(desc) => {
            counts.inline_image += 1;
            if desc.color_space.is_some() {
                counts.color_space += 1;
            }
        }
    }
}

fn count_state_resource_arena_entries(
    state: &GraphicsStateDescriptor,
    counts: &mut BackendPlanResourceArenaEntries,
) {
    match state {
        GraphicsStateDescriptor::SetStrokeColorSpace {
            object: Some(_), ..
        }
        | GraphicsStateDescriptor::SetFillColorSpace {
            object: Some(_), ..
        } => counts.color_space += 1,
        GraphicsStateDescriptor::SetStrokeColor {
            pattern: Some(_), ..
        }
        | GraphicsStateDescriptor::SetFillColor {
            pattern: Some(_), ..
        } => counts.pattern += 1,
        GraphicsStateDescriptor::ApplyExtGState { dict, font, .. } => {
            if dict.is_some() {
                counts.ext_g_state += 1;
            }
            if font.is_some() {
                counts.font += 1;
            }
        }
        GraphicsStateDescriptor::SetFont { dict: Some(_), .. } => counts.font += 1,
        GraphicsStateDescriptor::SetType3GlyphMetrics { .. } => counts.type3 += 1,
        GraphicsStateDescriptor::BeginMarkedContentWithProperties {
            properties:
                MarkedContentProperties::Name {
                    object: Some(_), ..
                },
            ..
        }
        | GraphicsStateDescriptor::MarkedContentPointWithProperties {
            properties:
                MarkedContentProperties::Name {
                    object: Some(_), ..
                },
            ..
        } => counts.properties += 1,
        _ => {}
    }
}

fn xobject_handle_has_transparency_group(
    stream_dict: Option<&crate::object::PdfDictionary>,
) -> bool {
    let Some(group) = stream_dict
        .and_then(|dict| dict.get("Group"))
        .and_then(PdfObject::as_dict)
    else {
        return false;
    };
    matches!(
        group.get("S").and_then(PdfObject::as_name),
        Some("Transparency")
    )
}

fn native_descriptor_kind(descriptor: &NativeDescriptor) -> &'static str {
    match descriptor {
        NativeDescriptor::Text(_) => "text",
        NativeDescriptor::Image(_) => "image",
        NativeDescriptor::Form(_) => "form",
        NativeDescriptor::Shading(_) => "shading",
        NativeDescriptor::State(_) => "state",
        NativeDescriptor::Pattern(_) => "pattern",
        NativeDescriptor::InlineImage(_) => "inline_image",
        NativeDescriptor::CompileRefusal(_) => "compile_refusal",
    }
}

fn backend_arena_kind(contract: &RenderContract) -> &'static str {
    match contract.print_profile {
        super::contract::PrintProfile::Display => "display_cpu_packed",
        super::contract::PrintProfile::Print => "print_cpu_packed",
        super::contract::PrintProfile::Proof => "proof_cpu_packed",
    }
}

fn backend_output_surface(contract: &RenderContract) -> &'static str {
    match contract.pixel_format {
        super::contract::PixelFormat::Gray8 => "gray_raster",
        super::contract::PixelFormat::Rgb8 | super::contract::PixelFormat::Bgr8 => "rgb_raster",
        super::contract::PixelFormat::Rgba8 | super::contract::PixelFormat::Bgra8 => "rgba_raster",
    }
}

impl DocumentViewsReport {
    pub(crate) fn from_engine(engine: &ContentEngine) -> Result<Self> {
        let canonical = engine.canonical_document();
        Ok(Self {
            schema_version: DOCUMENT_VIEWS_REPORT_SCHEMA_VERSION,
            document_revision: canonical.revision().0,
            source_fingerprint: canonical.fingerprint_hex(),
            original_byte_len: canonical.original_byte_len(),
            object_identity_count: canonical.object_identities().len(),
            page_count: engine.page_count()?,
            canonical_identity_owner: "PdfDocument/PdfReader bytes with CanonicalDocument identity coordinator"
                .to_string(),
            view_construction: "borrowed lazy view handles; constructing one view does not materialize another"
                .to_string(),
            materialization: canonical.view_materialization_stats(),
            views: vec![
                view_boundary(
                    "canonical",
                    "immutable source identity, revision, fingerprint, and object/page identities",
                    &[
                        "ContentEngine::canonical_document",
                        "CanonicalDocument::page_identity_for",
                    ],
                    &[
                        "CanonicalDocument::revision",
                        "CanonicalDocument::fingerprint_hex",
                        "CanonicalDocument::object_identities",
                        "CanonicalDocument::view_materialization_stats",
                    ],
                ),
                view_boundary(
                    "render",
                    "render-only preparation and retained page program/display-list/backend-plan access",
                    &[
                        "RenderDocumentView::viewport",
                        "RenderDocumentView::page_resources",
                        "RenderDocumentView::page_program",
                        "RenderDocumentView::display_list",
                        "RenderDocumentView::backend_document_plan_arena",
                        "RenderDocumentView::backend_plan_arena_report_for_contract",
                        "RenderDocumentView::render",
                        "RenderDocumentView::default_render_contract",
                        "RenderDocumentView::render_with_contract",
                        "RenderDocumentView::render_into_buffer_with_contract",
                    ],
                    &[
                        "ContentEngine::render_view",
                        "RenderDocumentView::page_identity_for",
                        "RenderDocumentView::page_program",
                        "RenderDocumentView::display_list",
                        "RenderDocumentView::backend_document_plan_arena",
                        "RenderDocumentView::backend_plan_arena_report_for_contract",
                        "RenderDocumentView::render",
                        "RenderDocumentView::default_render_contract",
                        "RenderDocumentView::render_with_contract",
                        "RenderDocumentView::render_into_buffer_with_contract",
                    ],
                ),
                view_boundary(
                    "edit",
                    "source-linked edit identity and editable content-stream operations",
                    &[
                        "EditDocumentView::page_source_identity",
                        "EditDocumentView::source_operations",
                    ],
                    &[
                        "ContentEngine::edit_view",
                        "EditDocumentView::page_source_identity",
                        "EditDocumentView::source_operations",
                    ],
                ),
                view_boundary(
                    "semantic",
                    "semantic extraction isolated from ordinary rendering",
                    &["SemanticDocumentView::structured_text"],
                    &[
                        "ContentEngine::semantic_view",
                        "SemanticDocumentView::structured_text",
                    ],
                ),
                view_boundary(
                    "validation",
                    "explicit validation/access checks isolated from ordinary rendering",
                    &["ValidationDocumentView::validate_page_access"],
                    &[
                        "ContentEngine::validation_view",
                        "ValidationDocumentView::validate_page_access",
                    ],
                ),
            ],
            owned_backend_plan_arena_entry_point:
                "RenderDocumentView::backend_document_plan_arena".to_string(),
            remaining_limitation:
                "ordinary view handles remain lazy/borrowed by design; owned CPU backend packed document arenas and explicit display/print/proof contract plan reports are available on render-view request, while GPU/debug runtime backends require future platform verification"
                    .to_string(),
        })
    }
}

fn view_boundary(
    name: &str,
    role: &str,
    lazy_materialization_trigger: &[&str],
    active_entry_points: &[&str],
) -> DocumentViewBoundary {
    DocumentViewBoundary {
        name: name.to_string(),
        role: role.to_string(),
        lazy_materialization_trigger: lazy_materialization_trigger
            .iter()
            .map(|entry| (*entry).to_string())
            .collect(),
        active_entry_points: active_entry_points
            .iter()
            .map(|entry| (*entry).to_string())
            .collect(),
        constructs_other_views: false,
    }
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
        *pages
            .entry(page.page_number)
            .or_insert_with(|| PageIdentity {
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
        self.counters
            .validation_pages
            .fetch_add(1, Ordering::Relaxed);
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
    pub compact: ParsedPageProgramArena,
    pub operations: Vec<ContentOperation>,
}

/// Source-linked cold representation of a parsed page program.
///
/// This keeps operator and operand metadata in compact arenas so renderer
/// planning can inspect program shape without walking cloned parser objects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParsedPageProgramArena {
    pub source_link: SourceLinkId,
    pub operators: Vec<String>,
    pub operations: Vec<ParsedPageProgramOp>,
    pub operands: Vec<ParsedOperandDescriptor>,
    pub estimated_source_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ParsedPageProgramOp {
    pub source_op_index: u32,
    pub operator_id: u32,
    pub operand_start: u32,
    pub operand_len: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ParsedOperandDescriptor {
    pub kind: ParsedOperandKind,
    pub byte_len: usize,
    pub nested_operands: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ParsedOperandKind {
    Integer,
    Real,
    Boolean,
    Name,
    String,
    Array,
    Dictionary,
}

impl ParsedPageProgramArena {
    pub fn build(source_link: SourceLinkId, operations: &[ContentOperation]) -> Self {
        let mut operator_ids = HashMap::<&str, u32>::new();
        let mut operators = Vec::<String>::new();
        let mut compact_ops = Vec::with_capacity(operations.len());
        let mut operands = Vec::new();
        let mut estimated_source_bytes = 0usize;

        for (source_op_index, operation) in operations.iter().enumerate() {
            let operator_id = match operator_ids.get(operation.operator.as_str()) {
                Some(id) => *id,
                None => {
                    let id = operators.len().min(u32::MAX as usize) as u32;
                    operator_ids.insert(operation.operator.as_str(), id);
                    operators.push(operation.operator.clone());
                    id
                }
            };
            let operand_start = operands.len().min(u32::MAX as usize) as u32;
            for operand in &operation.operands {
                operands.push(describe_operand(operand));
            }
            let operand_len = operation.operands.len().min(u32::MAX as usize) as u32;
            estimated_source_bytes = estimated_source_bytes
                .saturating_add(operation.operator.len())
                .saturating_add(operand_serialized_len(&operation.operands));
            compact_ops.push(ParsedPageProgramOp {
                source_op_index: source_op_index.min(u32::MAX as usize) as u32,
                operator_id,
                operand_start,
                operand_len,
            });
        }

        Self {
            source_link,
            operators,
            operations: compact_ops,
            operands,
            estimated_source_bytes,
        }
    }
}

fn describe_operand(operand: &Operand) -> ParsedOperandDescriptor {
    ParsedOperandDescriptor {
        kind: operand_kind(operand),
        byte_len: operand_byte_len(operand),
        nested_operands: nested_operand_count(operand),
    }
}

fn operand_kind(operand: &Operand) -> ParsedOperandKind {
    match operand {
        Operand::Integer(_) => ParsedOperandKind::Integer,
        Operand::Real(_) => ParsedOperandKind::Real,
        Operand::Boolean(_) => ParsedOperandKind::Boolean,
        Operand::Name(_) => ParsedOperandKind::Name,
        Operand::String(_) => ParsedOperandKind::String,
        Operand::Array(_) => ParsedOperandKind::Array,
        Operand::Dictionary(_) => ParsedOperandKind::Dictionary,
    }
}

fn operand_byte_len(operand: &Operand) -> usize {
    match operand {
        Operand::Integer(value) => value.to_string().len(),
        Operand::Real(value) => value.to_string().len(),
        Operand::Boolean(value) => {
            if *value {
                4
            } else {
                5
            }
        }
        Operand::Name(value) => 1usize.saturating_add(value.len()),
        Operand::String(value) => value.len(),
        Operand::Array(values) => operand_serialized_len(values),
        Operand::Dictionary(values) => values.iter().fold(4usize, |total, (name, value)| {
            total
                .saturating_add(1usize.saturating_add(name.len()))
                .saturating_add(operand_byte_len(value))
        }),
    }
}

fn operand_serialized_len(operands: &[Operand]) -> usize {
    operands.iter().fold(0usize, |total, operand| {
        total
            .saturating_add(1)
            .saturating_add(operand_byte_len(operand))
    })
}

fn nested_operand_count(operand: &Operand) -> usize {
    match operand {
        Operand::Array(values) => values.iter().fold(values.len(), |total, value| {
            total + nested_operand_count(value)
        }),
        Operand::Dictionary(values) => values.iter().fold(values.len(), |total, (_, value)| {
            total + nested_operand_count(value)
        }),
        _ => 0,
    }
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
        let source_link = self.canonical().source_link_for_page(&page);
        let operations = self.engine.get_page_content(page_number)?;
        Ok(ParsedPageProgram {
            page: self.canonical().page_identity_for(&page),
            source_link,
            resource_id: self.canonical().resource_id_for_page(&page),
            compact: ParsedPageProgramArena::build(source_link, &operations),
            operations,
        })
    }

    pub fn display_list(&self, page_number: usize, dpi: u32) -> Result<DisplayList> {
        self.canonical().record_render();
        self.engine.build_page_display_list(page_number, dpi)
    }

    pub fn backend_plan_arena_report(
        &self,
        page_number: usize,
        dpi: u32,
        mode: RenderMode,
    ) -> Result<BackendPlanArenaReport> {
        let contract = self
            .engine
            .default_render_contract(page_number, dpi, mode)?;
        self.backend_plan_arena_report_for_contract(contract)
    }

    pub fn backend_plan_arena_report_for_contract(
        &self,
        contract: RenderContract,
    ) -> Result<BackendPlanArenaReport> {
        self.canonical().record_render();
        contract.validate()?;
        let page_number = contract.page_number;
        let resources = self.engine.get_page_resources(page_number)?;
        let list = self
            .engine
            .build_page_display_list(page_number, contract.dpi)?;
        let plan = RenderPlan::compile_with_resources(list, contract, &resources)?;
        Ok(BackendPlanArenaReport::from_plan(
            page_number,
            self.canonical().revision().0,
            &plan,
        ))
    }

    pub fn backend_document_plan_arena(
        &self,
        dpi: u32,
        mode: RenderMode,
    ) -> Result<BackendDocumentPlanArena> {
        let page_count = self.engine.page_count()?;
        let mut pages = Vec::with_capacity(page_count);
        for page_number in 1..=page_count {
            self.canonical().record_render();
            let page = self.engine.get_page(page_number)?;
            let contract = self
                .engine
                .default_render_contract(page_number, dpi, mode)?;
            let resources = self.engine.get_page_resources(page_number)?;
            let list = self.engine.build_page_display_list(page_number, dpi)?;
            let plan = RenderPlan::compile_with_resources(list, contract, &resources)?;
            pages.push(BackendDocumentPagePlan {
                page_number,
                page_identity: self.canonical().page_identity_for(&page),
                source_link: self.canonical().source_link_for_page(&page),
                resource_id: self.canonical().resource_id_for_page(&page),
                plan,
            });
        }
        Ok(BackendDocumentPlanArena {
            document_revision: self.canonical().revision(),
            source_fingerprint: self.canonical().fingerprint_hex(),
            dpi,
            render_mode: mode,
            arena_kind: "display_cpu_packed".to_string(),
            print_profile: format!("{:?}", super::contract::PrintProfile::Display),
            pages,
        })
    }

    pub fn render(&self, page_number: usize, dpi: u32, mode: RenderMode) -> Result<PixelBuffer> {
        self.canonical().record_render();
        self.engine.render_page_with_mode(page_number, dpi, mode)
    }

    pub fn default_render_contract(
        &self,
        page_number: usize,
        dpi: u32,
        mode: RenderMode,
    ) -> Result<RenderContract> {
        self.canonical().record_render();
        self.engine.default_render_contract(page_number, dpi, mode)
    }

    pub fn default_render_contract_for_page_box(
        &self,
        page_number: usize,
        dpi: u32,
        mode: RenderMode,
        page_box: PageBox,
    ) -> Result<RenderContract> {
        self.canonical().record_render();
        self.engine
            .default_render_contract_for_page_box(page_number, dpi, mode, page_box)
    }

    pub fn render_with_contract(
        &self,
        contract: &RenderContract,
        cancel: &CancelToken,
    ) -> Result<PixelBuffer> {
        self.canonical().record_render();
        self.engine.render_page_with_contract(contract, cancel)
    }

    pub fn render_with_contract_and_font_substitution_report(
        &self,
        contract: &RenderContract,
        cancel: &CancelToken,
    ) -> Result<(PixelBuffer, FontSubstitutionLog)> {
        self.canonical().record_render();
        self.engine
            .render_page_with_contract_and_font_substitution_report(contract, cancel)
    }

    pub fn render_with_contract_and_telemetry_report(
        &self,
        contract: &RenderContract,
        cancel: &CancelToken,
    ) -> Result<(
        PixelBuffer,
        FontSubstitutionLog,
        RenderContractTelemetryReport,
    )> {
        self.canonical().record_render();
        self.engine
            .render_page_with_contract_and_telemetry_report(contract, cancel)
    }

    pub fn render_into_buffer_with_contract(
        &self,
        contract: &RenderContract,
        cancel: &CancelToken,
        output: &mut [u8],
    ) -> Result<()> {
        self.canonical().record_render();
        self.engine
            .render_page_into_buffer(contract, cancel, output)
    }

    pub fn render_into_buffer_with_font_substitution_report(
        &self,
        contract: &RenderContract,
        cancel: &CancelToken,
        output: &mut [u8],
    ) -> Result<FontSubstitutionLog> {
        self.canonical().record_render();
        self.engine
            .render_page_into_buffer_with_font_substitution_report(contract, cancel, output)
    }

    pub fn render_into_buffer_with_telemetry_report(
        &self,
        contract: &RenderContract,
        cancel: &CancelToken,
        output: &mut [u8],
    ) -> Result<(FontSubstitutionLog, RenderContractTelemetryReport)> {
        self.canonical().record_render();
        self.engine
            .render_page_into_buffer_with_telemetry_report(contract, cancel, output)
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
    use crate::object::PdfDictionary;
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

    fn engine_with_text_and_image() -> ContentEngine {
        let mut builder = PdfBuilder::new();
        let image = builder
            .add_rgb_image(2, 2, vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0])
            .expect("register test image");
        let page = builder.add_page(AuthorPageSize::LETTER);
        page.draw_text("resource arenas", 72.0, 720.0, &TextStyle::default())
            .expect("write test text");
        page.draw_image(image, 72.0, 680.0, 24.0, 24.0);
        ContentEngine::open_bytes(builder.to_bytes().expect("serialize test PDF"))
            .expect("open text and image PDF")
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
        let render = engine
            .render_view()
            .page_program(1)
            .expect("render program");
        let edit = engine
            .edit_view()
            .page_source_identity(1)
            .expect("edit source link");
        assert_eq!(render.source_link, edit);
        assert_eq!(render.page.object.id.0, edit.0);
    }

    #[test]
    fn render_page_program_exposes_compact_source_linked_arena() {
        let engine = engine();
        let program = engine
            .render_view()
            .page_program(1)
            .expect("render program");
        assert_eq!(program.compact.source_link, program.source_link);
        assert_eq!(program.compact.operations.len(), program.operations.len());
        assert!(program.compact.estimated_source_bytes > 0);
        assert!(program.compact.operators.len() <= program.operations.len());

        let operand_count = program
            .operations
            .iter()
            .map(|operation| operation.operands.len())
            .sum::<usize>();
        assert_eq!(program.compact.operands.len(), operand_count);

        for compact_op in &program.compact.operations {
            let source = &program.operations[compact_op.source_op_index as usize];
            let operator = &program.compact.operators[compact_op.operator_id as usize];
            assert_eq!(operator, &source.operator);
            assert_eq!(compact_op.operand_len as usize, source.operands.len());
            let start = compact_op.operand_start as usize;
            let end = start + compact_op.operand_len as usize;
            assert!(end <= program.compact.operands.len());
        }
    }

    #[test]
    fn render_view_exposes_backend_plan_arena_report() {
        let engine = engine();
        let report = engine
            .render_view()
            .backend_plan_arena_report(1, 72, RenderMode::Compat)
            .expect("backend plan arena report");

        assert_eq!(
            report.schema_version,
            BACKEND_PLAN_ARENA_REPORT_SCHEMA_VERSION
        );
        assert_eq!(report.page_number, 1);
        assert_eq!(
            report.document_revision,
            engine.canonical_document().revision().0
        );
        assert!(!report.render_contract_fingerprint.is_empty());
        assert_eq!(report.arena_kind, "display_cpu_packed");
        assert_eq!(report.print_profile, "Display");
        assert_eq!(report.output_surface, "rgba_raster");
        assert!(report.hot_operation_count > 0);
        assert!(report.bounds_arena_entries <= report.hot_operation_count);
        assert!(report.batch_count > 0);
        assert_eq!(
            report.batch_count,
            report.native_payload_batches + report.vector_batches
        );
        assert_eq!(
            report.descriptor_arena_entries,
            report.descriptor_kinds.values().sum::<usize>()
        );
        assert!(report.resource_arena_entries.font > 0);
        assert!(report
            .descriptor_kinds
            .keys()
            .any(|kind| kind == "text" || kind == "state"));
        assert!(report.compile_refusals.is_empty());
        let stats = engine.canonical_document().view_materialization_stats();
        assert!(stats.render_pages > 0);
        assert_eq!(stats.semantic_pages, 0);
        assert_eq!(stats.validation_pages, 0);
        assert_eq!(stats.edit_pages, 0);
    }

    #[test]
    fn render_view_exposes_print_contract_backend_plan_arena_report() {
        let engine = engine();
        let mut contract = engine
            .default_render_contract(1, 72, RenderMode::Compat)
            .expect("default contract");
        contract.print_profile = crate::render::PrintProfile::Print;

        let display = engine
            .render_view()
            .backend_plan_arena_report(1, 72, RenderMode::Compat)
            .expect("display backend plan arena report");
        let print = engine
            .render_view()
            .backend_plan_arena_report_for_contract(contract)
            .expect("print backend plan arena report");

        assert_eq!(print.page_number, 1);
        assert_eq!(print.arena_kind, "print_cpu_packed");
        assert_eq!(print.print_profile, "Print");
        assert_eq!(print.execution_mode, "Standard");
        assert_eq!(print.backend_selection, "StandardCpu");
        assert_ne!(
            display.render_contract_fingerprint,
            print.render_contract_fingerprint
        );
        assert_eq!(print.hot_operation_count, display.hot_operation_count);
    }

    #[test]
    fn backend_plan_arena_report_exposes_resource_specific_payload_counts() {
        let engine = engine_with_text_and_image();
        let report = engine
            .render_view()
            .backend_plan_arena_report(1, 72, RenderMode::Compat)
            .expect("backend plan arena report");

        assert!(report.resource_arena_entries.font > 0);
        assert!(report.resource_arena_entries.image > 0);
        assert_eq!(report.resource_arena_entries.appearance, 0);
        assert_eq!(
            report
                .descriptor_kinds
                .get("image")
                .copied()
                .unwrap_or_default(),
            report.resource_arena_entries.image
        );

        let json = serde_json::to_value(&report).expect("serialize report");
        assert_eq!(
            json["resource_arena_entries"]["font"]
                .as_u64()
                .expect("font arena count") as usize,
            report.resource_arena_entries.font
        );
        assert_eq!(
            json["resource_arena_entries"]["image"]
                .as_u64()
                .expect("image arena count") as usize,
            report.resource_arena_entries.image
        );
    }

    #[test]
    fn backend_plan_arena_counts_image_xobject_color_space_payloads() {
        let mut counts = BackendPlanResourceArenaEntries::default();
        count_descriptor_resource_arena_entries(
            &NativeDescriptor::Image(crate::render::plan::ImageXObjectDescriptor {
                name: "Im1".to_string(),
                handle: Some(crate::render::plan::ResolvedXObjectHandle {
                    name: "Im1".to_string(),
                    object_number: 7,
                    generation_number: 0,
                    subtype: Some("Image".to_string()),
                    stream_dict: Some(PdfDictionary::empty()),
                    bbox: None,
                    matrix: None,
                    image_color_space: Some(crate::render::plan::ResolvedInlineImageColorSpace {
                        name: "Cs1".to_string(),
                        object: PdfObject::Name("DeviceRGB".to_string()),
                    }),
                }),
            }),
            &mut counts,
        );

        assert_eq!(counts.image, 1);
        assert_eq!(counts.color_space, 1);
    }

    #[test]
    fn render_view_owns_backend_document_plan_arena() {
        let engine = engine();
        let arena = engine
            .render_view()
            .backend_document_plan_arena(72, RenderMode::Compat)
            .expect("backend document plan arena");
        assert_eq!(
            arena.document_revision,
            engine.canonical_document().revision()
        );
        assert_eq!(arena.render_mode, RenderMode::Compat);
        assert_eq!(arena.page_count(), engine.page_count().expect("page count"));
        assert!(arena.plan_for_page(1).is_some());
        assert_eq!(arena.pages[0].page_number, 1);
        assert_eq!(arena.pages[0].page_identity.page_number, 1);
        assert_eq!(
            arena.pages[0].source_link.0,
            arena.pages[0].page_identity.object.id.0
        );
        assert_eq!(
            arena.pages[0].resource_id.0,
            arena.pages[0].page_identity.object.id.0
        );

        let report = arena.report();
        assert_eq!(
            report.schema_version,
            BACKEND_DOCUMENT_PLAN_ARENA_REPORT_SCHEMA_VERSION
        );
        assert!(report.owns_backend_plans);
        assert_eq!(report.arena_kind, "display_cpu_packed");
        assert_eq!(report.print_profile, "Display");
        assert_eq!(report.page_count, arena.page_count());
        assert_eq!(report.owned_page_plan_count, arena.page_count());
        assert_eq!(report.pages.len(), arena.page_count());
        assert!(report.total_hot_operation_count > 0);
        assert_eq!(
            report.total_descriptor_arena_entries,
            report.descriptor_kinds.values().sum::<usize>()
        );
        assert!(report.total_resource_arena_entries.font > 0);
        assert!(report
            .descriptor_kinds
            .keys()
            .any(|kind| kind == "text" || kind == "state"));
        assert!(report.compile_refusals.is_empty());

        let stats = engine.canonical_document().view_materialization_stats();
        assert!(stats.render_pages > 0);
        assert_eq!(stats.semantic_pages, 0);
        assert_eq!(stats.validation_pages, 0);
        assert_eq!(stats.edit_pages, 0);
    }

    #[test]
    fn changing_source_bytes_changes_canonical_revision() {
        let first = engine();
        let mut builder = PdfBuilder::new();
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("different source", 72.0, 720.0, &TextStyle::default())
            .expect("write page");
        let second =
            ContentEngine::open_bytes(builder.to_bytes().expect("serialize")).expect("open");
        assert_ne!(
            first.canonical_document().revision(),
            second.canonical_document().revision()
        );
    }

    #[test]
    fn document_views_report_exposes_lazy_boundaries() {
        let engine = engine();
        let report = engine
            .document_views_report()
            .expect("document views report");
        assert_eq!(report.schema_version, DOCUMENT_VIEWS_REPORT_SCHEMA_VERSION);
        assert_eq!(report.views.len(), 5);
        assert_eq!(report.materialization, ViewMaterializationStats::default());
        assert!(report.views.iter().any(|view| view.name == "render"
            && view
                .active_entry_points
                .iter()
                .any(|entry| entry == "RenderDocumentView::page_program")));
        assert!(report.views.iter().any(|view| view.name == "render"
            && view
                .active_entry_points
                .iter()
                .any(|entry| entry == "RenderDocumentView::backend_document_plan_arena")));
        assert_eq!(
            report.owned_backend_plan_arena_entry_point,
            "RenderDocumentView::backend_document_plan_arena"
        );
        assert!(report
            .remaining_limitation
            .contains("owned CPU backend packed document arenas"));
        assert!(report.views.iter().all(|view| !view.constructs_other_views));

        let _ = engine
            .render_view()
            .page_program(1)
            .expect("render program");
        let after = engine
            .document_views_report()
            .expect("document views report after render view");
        assert!(after.materialization.render_pages > 0);
        assert_eq!(after.materialization.edit_pages, 0);
        assert_eq!(after.materialization.semantic_pages, 0);
        assert_eq!(after.materialization.validation_pages, 0);
    }

    #[test]
    fn render_view_owns_contract_render_entrypoints() {
        let engine = engine();
        let view = engine.render_view();
        let contract = view
            .default_render_contract(1, 72, RenderMode::Compat)
            .expect("render contract");
        assert_eq!(
            contract.document_revision,
            engine.canonical_document().revision()
        );

        let mut output = vec![0; contract.stride * contract.height as usize];
        view.render_into_buffer_with_contract(&contract, &CancelToken::none(), &mut output)
            .expect("caller-owned render");
        assert!(output.iter().any(|byte| *byte != 0));

        let stats = engine.canonical_document().view_materialization_stats();
        assert!(stats.render_pages >= 2);
        assert_eq!(stats.edit_pages, 0);
        assert_eq!(stats.semantic_pages, 0);
        assert_eq!(stats.validation_pages, 0);
    }
}
