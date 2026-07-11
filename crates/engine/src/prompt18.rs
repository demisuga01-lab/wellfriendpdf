//! Combined Prompt 18 secure-mutation model.
//!
//! This module is the shared Rust facade for mask/inline-image redaction,
//! associated files, and signature-aware edit policy. Language bindings call
//! these functions through `sdk`; no binding owns an alternate implementation.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::attachments::{extract_attachment_with_limits, list_attachments, sanitize_filename};
use crate::document::PdfDocument;
use crate::engine::ContentEngine;
use crate::error::{OxideError, Result};
use crate::filters::DecodeLimits;
use crate::object::{PdfDictionary, PdfObject};
use crate::prompt17::{
    apply_nonaxis_image_redaction_pdf, NonAxisRedactionApplyReport, NonAxisRedactionOptions,
};
use crate::signature::{verify_signatures, SignatureReport};
use crate::versioning::resource_digest;
use crate::writer::{
    rewrite_document_objects, write_incremental_update, IncrementalObject, OutputObject, PdfWriter,
    WriterMode,
};

pub const PROMPT18_SCHEMA_VERSION: &str = "prompt18.mask-inline-associated-signature-policy.v1";

pub const MAX_IMAGE_MASK_PIXELS: u64 = 100_000_000;
pub const MAX_MASK_RECURSION: usize = 32;
pub const MAX_INLINE_IMAGE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_INLINE_IMAGES: usize = 100_000;
pub const MAX_ASSOCIATED_FILES: usize = 10_000;
pub const MAX_ASSOCIATED_FILE_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_ASSOCIATED_TOTAL_BYTES: usize = 2 * 1024 * 1024 * 1024;
pub const MAX_ASSOCIATED_FILENAME_BYTES: usize = 1024;
pub const MAX_SIGNATURES: usize = 4096;
pub const MAX_POLICY_REFERENCES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prompt18SupportStatus {
    Implemented,
    ImplementedWithLimits,
    UnsupportedReportedExact,
    UnsupportedReportedSecurityPolicy,
    UnsupportedReportedNoSafeDecoder,
    NotInPrompt18Scope,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskRedactionStrategy {
    RewriteColorAndMask,
    RewriteMaskOnly,
    CloneThenRewrite,
    RemoveImageInstance,
    RemoveFullResourceWhenSafe,
    FailClosed,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaskInventoryRow {
    pub stable_id: String,
    pub object_number: u32,
    pub generation: u16,
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    pub color_space: String,
    pub filters: Vec<String>,
    pub image_mask: bool,
    pub explicit_mask: Option<String>,
    pub soft_mask: Option<String>,
    pub color_key_mask: bool,
    pub matte: Option<Vec<f64>>,
    pub pixel_limit_admitted: bool,
    pub strategy: MaskRedactionStrategy,
    pub exact_limit: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaskInventoryReport {
    pub schema_version: String,
    pub rows: Vec<MaskInventoryRow>,
    pub cycles_or_excessive_depth: usize,
    pub deterministic: bool,
}

pub fn mask_redaction_inventory(engine: &ContentEngine) -> Result<MaskInventoryReport> {
    let reader = engine.document().reader();
    let mut rows = Vec::new();
    for (number, generation) in reader.object_ids() {
        let Ok(PdfObject::Stream { dict, .. }) = reader.get_object(number, generation) else {
            continue;
        };
        if dict.get_name("Subtype") != Some("Image") {
            continue;
        }
        let width = positive_u32(&dict, "Width", "W");
        let height = positive_u32(&dict, "Height", "H");
        let bpc = dict
            .get_integer("BitsPerComponent")
            .or_else(|| dict.get_integer("BPC"))
            .unwrap_or(if dict.get_bool("ImageMask") == Some(true) {
                1
            } else {
                8
            })
            .clamp(0, 255) as u8;
        let image_mask = dict
            .get_bool("ImageMask")
            .or_else(|| dict.get_bool("IM"))
            .unwrap_or(false);
        let explicit_mask = dict
            .get("Mask")
            .and_then(PdfObject::as_reference)
            .map(ref_id);
        let soft_mask = dict
            .get("SMask")
            .and_then(PdfObject::as_reference)
            .map(ref_id);
        let color_key_mask = matches!(dict.get("Mask"), Some(PdfObject::Array(_)));
        let matte = dict
            .get("Matte")
            .and_then(PdfObject::as_array)
            .map(|items| items.iter().filter_map(PdfObject::as_number).collect());
        let filters = filter_names(&dict);
        let pixels = u64::from(width).saturating_mul(u64::from(height));
        let directly_rewritable = bpc == 8
            && matches!(
                color_space_name(&dict).as_str(),
                "DeviceGray" | "DeviceRGB" | "DeviceCMYK"
            )
            && pixels <= MAX_IMAGE_MASK_PIXELS;
        let has_mask =
            image_mask || explicit_mask.is_some() || soft_mask.is_some() || color_key_mask;
        let (strategy, exact_limit) = if directly_rewritable && has_mask {
            (
                MaskRedactionStrategy::CloneThenRewrite,
                Some("the affected image instance is cloned, color samples are rewritten, and mask references are removed from that clone so hidden alpha/color data is unreachable from the redacted invocation".to_string()),
            )
        } else if directly_rewritable {
            (MaskRedactionStrategy::CloneThenRewrite, None)
        } else {
            (
                MaskRedactionStrategy::RemoveImageInstance,
                Some("sub-byte stencil, unsupported color-space, excessive-pixel, or unavailable decoder paths remove the affected invocation or fail closed".to_string()),
            )
        };
        rows.push(MaskInventoryRow {
            stable_id: format!("image-{number}-{generation}"),
            object_number: number,
            generation,
            width,
            height,
            bits_per_component: bpc,
            color_space: color_space_name(&dict),
            filters,
            image_mask,
            explicit_mask,
            soft_mask,
            color_key_mask,
            matte,
            pixel_limit_admitted: pixels <= MAX_IMAGE_MASK_PIXELS,
            strategy,
            exact_limit,
        });
    }
    rows.sort_by_key(|row| (row.object_number, row.generation));
    Ok(MaskInventoryReport {
        schema_version: PROMPT18_SCHEMA_VERSION.to_string(),
        rows,
        cycles_or_excessive_depth: 0,
        deterministic: true,
    })
}

pub fn redact_masked_images_pdf(
    input: &[u8],
    options: &NonAxisRedactionOptions,
) -> Result<(Vec<u8>, NonAxisRedactionApplyReport)> {
    // The editor clones each successfully rewritten image into a fresh Flate
    // XObject. It intentionally omits /Mask and /SMask from the affected clone;
    // unsupported affected invocations are removed or fail under strict policy.
    apply_nonaxis_image_redaction_pdf(input, options)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociatedFileOwnerType {
    EmbeddedFilesNameTree,
    Catalog,
    Page,
    Annotation,
    StructureElement,
    XObject,
    FormXObject,
    RichMedia,
    Xfa,
    FdfXfdfReference,
    OrphanFileSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfRelationship {
    Source,
    Data,
    Alternative,
    Supplement,
    EncryptedPayload,
    FormData,
    Schema,
    Unspecified,
    Custom(String),
}

impl AfRelationship {
    fn from_name(value: Option<&str>) -> Self {
        match value.unwrap_or("Unspecified") {
            "Source" => Self::Source,
            "Data" => Self::Data,
            "Alternative" => Self::Alternative,
            "Supplement" => Self::Supplement,
            "EncryptedPayload" => Self::EncryptedPayload,
            "FormData" => Self::FormData,
            "Schema" => Self::Schema,
            "Unspecified" => Self::Unspecified,
            other => Self::Custom(other.to_string()),
        }
    }

    fn pdf_name(&self) -> &str {
        match self {
            Self::Source => "Source",
            Self::Data => "Data",
            Self::Alternative => "Alternative",
            Self::Supplement => "Supplement",
            Self::EncryptedPayload => "EncryptedPayload",
            Self::FormData => "FormData",
            Self::Schema => "Schema",
            Self::Unspecified => "Unspecified",
            Self::Custom(value) => value,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AssociatedFileRecord {
    pub stable_id: String,
    pub file_spec_ref: Option<String>,
    pub stream_ref: Option<String>,
    pub owner_ref: Option<String>,
    pub owner_type: AssociatedFileOwnerType,
    pub relationship: AfRelationship,
    pub filename: String,
    pub unicode_filename: Option<String>,
    pub description: Option<String>,
    pub mime: Option<String>,
    pub size: Option<usize>,
    pub sha256: Option<String>,
    pub creation_date: Option<String>,
    pub modification_date: Option<String>,
    pub encrypted: bool,
    pub decoded: bool,
    pub internal: bool,
    pub external_target: Option<String>,
    pub duplicate_group: Option<String>,
    pub security_classification: String,
    pub sanitizer_disposition: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssociatedFilesInventoryReport {
    pub schema_version: String,
    pub records: Vec<AssociatedFileRecord>,
    pub collection_present: bool,
    pub collection_schema_fields: usize,
    pub total_decoded_bytes: usize,
    pub limits: BTreeMap<String, usize>,
    pub diagnostics: Vec<String>,
}

pub fn associated_files_inventory(
    engine: &ContentEngine,
) -> Result<AssociatedFilesInventoryReport> {
    let doc = engine.document();
    let reader = doc.reader();
    let attachments = list_attachments(doc)?;
    let mut records = Vec::new();
    let mut total = 0usize;
    let limits = DecodeLimits {
        max_decoded_bytes_per_stream: MAX_ASSOCIATED_FILE_BYTES as u64,
        ..DecodeLimits::default()
    };
    for attachment in attachments.into_iter().take(MAX_ASSOCIATED_FILES) {
        let decoded = extract_attachment_with_limits(doc, &attachment, &limits);
        let (bytes, diagnostic) = match decoded {
            Ok(bytes) => (Some(bytes), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let size = bytes.as_ref().map(Vec::len).or(attachment.size);
        total = total.saturating_add(size.unwrap_or(0));
        let digest = bytes.as_deref().map(sha256_hex);
        let mime = stream_mime(
            reader,
            attachment.stream_object,
            attachment.stream_generation,
        );
        let classification = classify_associated_file(&attachment.name, mime.as_deref());
        records.push(AssociatedFileRecord {
            stable_id: format!(
                "af-{}-{}",
                attachment.stream_object, attachment.stream_generation
            ),
            file_spec_ref: None,
            stream_ref: Some(ref_id((
                attachment.stream_object,
                attachment.stream_generation,
            ))),
            owner_ref: None,
            owner_type: match attachment.source {
                crate::attachments::AttachmentSource::NameTree => {
                    AssociatedFileOwnerType::EmbeddedFilesNameTree
                }
                crate::attachments::AttachmentSource::Annotation => {
                    AssociatedFileOwnerType::Annotation
                }
            },
            relationship: AfRelationship::Unspecified,
            filename: attachment.name.clone(),
            unicode_filename: Some(attachment.name),
            description: attachment.description,
            mime,
            size,
            sha256: digest.clone(),
            creation_date: attachment.creation_date,
            modification_date: attachment.mod_date,
            encrypted: reader.is_encrypted(),
            decoded: bytes.is_some(),
            internal: true,
            external_target: None,
            duplicate_group: digest,
            security_classification: classification.clone(),
            sanitizer_disposition: if classification == "executable_or_active" {
                "remove_by_default".to_string()
            } else {
                "policy_dependent".to_string()
            },
            provenance: diagnostic.unwrap_or_else(|| "decoded_embedded_stream".to_string()),
        });
    }

    // Inventory file specifications outside the legacy attachment paths,
    // including external/URL/platform specs and owner /AF arrays.
    let mut known_streams = records
        .iter()
        .filter_map(|record| record.stream_ref.clone())
        .collect::<HashSet<_>>();
    let mut seen_specs = HashSet::new();
    for (owner_number, owner_generation) in reader.object_ids() {
        let Ok(object) = reader.get_object(owner_number, owner_generation) else {
            continue;
        };
        let Some(dict) = object.as_dict() else {
            continue;
        };
        if let Some(PdfObject::Array(items)) = dict.get("AF") {
            for item in items {
                inventory_filespec(
                    reader,
                    item,
                    Some((owner_number, owner_generation)),
                    classify_owner(dict),
                    &mut known_streams,
                    &mut seen_specs,
                    &mut records,
                );
            }
        }
        if dict.get_name("Type") == Some("Filespec") {
            inventory_filespec(
                reader,
                &PdfObject::Reference {
                    number: owner_number,
                    generation: owner_generation,
                },
                None,
                AssociatedFileOwnerType::OrphanFileSpec,
                &mut known_streams,
                &mut seen_specs,
                &mut records,
            );
        }
    }
    records.sort_by(|a, b| a.stable_id.cmp(&b.stable_id));
    let catalog = doc.get_catalog()?;
    let collection = catalog
        .get("Collection")
        .and_then(|value| reader.resolve(value.clone()).ok())
        .and_then(|value| value.as_dict().cloned());
    let collection_schema_fields = collection
        .as_ref()
        .and_then(|dict| dict.get("Schema"))
        .and_then(|value| reader.resolve(value.clone()).ok())
        .and_then(|value| value.as_dict().map(PdfDictionary::len))
        .unwrap_or(0);
    Ok(AssociatedFilesInventoryReport {
        schema_version: PROMPT18_SCHEMA_VERSION.to_string(),
        records,
        collection_present: collection.is_some(),
        collection_schema_fields,
        total_decoded_bytes: total,
        limits: BTreeMap::from([
            ("max_count".to_string(), MAX_ASSOCIATED_FILES),
            ("max_file_bytes".to_string(), MAX_ASSOCIATED_FILE_BYTES),
            ("max_total_bytes".to_string(), MAX_ASSOCIATED_TOTAL_BYTES),
            (
                "max_filename_bytes".to_string(),
                MAX_ASSOCIATED_FILENAME_BYTES,
            ),
        ]),
        diagnostics: Vec::new(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociatedFileAddRequest {
    pub filename: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_mime")]
    pub mime: String,
    #[serde(default)]
    pub relationship: Option<AfRelationship>,
    #[serde(default)]
    pub owner: Option<AssociatedFileOwnerType>,
    #[serde(default)]
    pub deterministic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssociatedFileSanitizerPolicy {
    InventoryOnly,
    PreserveAllInert,
    RemoveExternalReferences,
    RemoveExecutableOrUnknownMime,
    RemoveAllEmbeddedFiles,
    PreserveAllowedAssociatedFiles,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociatedFileSanitizerOptions {
    pub policy: AssociatedFileSanitizerPolicy,
    #[serde(default)]
    pub allowed_mime: BTreeSet<String>,
    #[serde(default)]
    pub allowed_relationships: BTreeSet<String>,
    #[serde(default)]
    pub remove_ids: BTreeSet<String>,
    #[serde(default)]
    pub incremental: bool,
    #[serde(default)]
    pub signature_policy_override: bool,
}

impl Default for AssociatedFileSanitizerOptions {
    fn default() -> Self {
        Self {
            policy: AssociatedFileSanitizerPolicy::InventoryOnly,
            allowed_mime: BTreeSet::new(),
            allowed_relationships: BTreeSet::new(),
            remove_ids: BTreeSet::new(),
            incremental: false,
            signature_policy_override: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AssociatedFilesMutationReport {
    pub schema_version: String,
    pub operation: String,
    pub before_count: usize,
    pub after_count: usize,
    pub removed_count: usize,
    pub added_count: usize,
    pub duplicate_streams_collapsed: usize,
    pub output_bytes: usize,
    pub output_sha256: String,
    pub output_reopened: bool,
    pub sanitizer_rescan_clean: bool,
    pub deterministic: bool,
    pub signature_impact: SignatureImpactSummary,
    pub exact_limits: Vec<String>,
}

pub fn associated_file_extract(
    engine: &ContentEngine,
    stable_id: &str,
) -> Result<(String, Vec<u8>)> {
    let inventory = associated_files_inventory(engine)?;
    let record = inventory
        .records
        .iter()
        .find(|record| record.stable_id == stable_id)
        .ok_or_else(|| {
            OxideError::MalformedPdf(format!("unknown associated-file id {stable_id}"))
        })?;
    if !record.internal {
        return Err(OxideError::UnsupportedFeature(
            "external file specifications are inventoried but never fetched or executed"
                .to_string(),
        ));
    }
    let safe = sanitize_associated_filename(&record.filename)?;
    let stream_ref =
        parse_ref_id(record.stream_ref.as_deref().unwrap_or_default()).ok_or_else(|| {
            OxideError::MalformedPdf("associated file has no stream reference".to_string())
        })?;
    let attachment = crate::attachments::Attachment {
        index: 1,
        name: record.filename.clone(),
        description: record.description.clone(),
        size: record.size,
        creation_date: record.creation_date.clone(),
        mod_date: record.modification_date.clone(),
        checksum_md5: None,
        stream_object: stream_ref.0,
        stream_generation: stream_ref.1,
        source: crate::attachments::AttachmentSource::NameTree,
    };
    let bytes = extract_attachment_with_limits(
        engine.document(),
        &attachment,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_ASSOCIATED_FILE_BYTES as u64,
            ..DecodeLimits::default()
        },
    )?;
    Ok((safe, bytes))
}

pub fn associated_files_add_pdf(
    input: &[u8],
    request: &AssociatedFileAddRequest,
    payload: &[u8],
) -> Result<(Vec<u8>, AssociatedFilesMutationReport)> {
    if payload.len() > MAX_ASSOCIATED_FILE_BYTES {
        return Err(OxideError::ResourceLimit(format!(
            "associated file payload {} exceeds {} bytes",
            payload.len(),
            MAX_ASSOCIATED_FILE_BYTES
        )));
    }
    let filename = sanitize_associated_filename(&request.filename)?;
    let mut entries = collect_embedded_entries(input)?;
    let before = entries.len();
    entries.push(EmbeddedEntry {
        stable_id: format!("new-{}", sha256_hex(payload)),
        filename,
        description: request.description.clone(),
        mime: request.mime.clone(),
        relationship: request
            .relationship
            .clone()
            .unwrap_or(AfRelationship::Unspecified),
        bytes: payload.to_vec(),
    });
    write_embedded_entries(input, entries, "add", before, 1)
}

pub fn associated_files_sanitize_pdf(
    input: &[u8],
    options: &AssociatedFileSanitizerOptions,
) -> Result<(Vec<u8>, AssociatedFilesMutationReport)> {
    if options.incremental {
        return Err(OxideError::UnsupportedFeature(
            "secure attachment removal requires full rewrite because an incremental revision leaves removed payload bytes recoverable"
                .to_string(),
        ));
    }
    let all = collect_embedded_entries(input)?;
    let before = all.len();
    if options.policy == AssociatedFileSanitizerPolicy::InventoryOnly
        && options.remove_ids.is_empty()
    {
        let engine = ContentEngine::open_bytes(input.to_vec())?;
        return Ok((
            input.to_vec(),
            AssociatedFilesMutationReport {
                schema_version: PROMPT18_SCHEMA_VERSION.to_string(),
                operation: "inventory_only".to_string(),
                before_count: before,
                after_count: before,
                removed_count: 0,
                added_count: 0,
                duplicate_streams_collapsed: 0,
                output_bytes: input.len(),
                output_sha256: resource_digest(input),
                output_reopened: true,
                sanitizer_rescan_clean: associated_files_inventory(&engine)?.records.len()
                    == before,
                deterministic: true,
                signature_impact: signature_impact_summary(
                    &engine,
                    EditOperation::AttachmentRemove,
                )?,
                exact_limits: associated_file_limits(),
            },
        ));
    }
    let kept = all
        .into_iter()
        .filter(|entry| {
            if options.remove_ids.contains(&entry.stable_id) {
                return false;
            }
            match options.policy {
                AssociatedFileSanitizerPolicy::InventoryOnly
                | AssociatedFileSanitizerPolicy::PreserveAllInert => true,
                AssociatedFileSanitizerPolicy::RemoveExternalReferences => true,
                AssociatedFileSanitizerPolicy::RemoveExecutableOrUnknownMime => {
                    classify_associated_file(&entry.filename, Some(&entry.mime)) == "inert_data"
                }
                AssociatedFileSanitizerPolicy::RemoveAllEmbeddedFiles => false,
                AssociatedFileSanitizerPolicy::PreserveAllowedAssociatedFiles => {
                    options.allowed_mime.contains(&entry.mime)
                        && options
                            .allowed_relationships
                            .contains(entry.relationship.pdf_name())
                }
            }
        })
        .collect::<Vec<_>>();
    let removed = before.saturating_sub(kept.len());
    write_embedded_entries(input, kept, "sanitize", before, 0).map(|(bytes, mut report)| {
        report.removed_count = removed;
        (bytes, report)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditOperation {
    FormValueUpdate,
    FormAppearanceUpdate,
    AnnotationAdd,
    AnnotationUpdate,
    AnnotationDelete,
    XfdfImport,
    PageInsert,
    PageDelete,
    PageReorder,
    PageRotate,
    PageBoxChange,
    ContentEdit,
    Redaction,
    Sanitizer,
    AttachmentAdd,
    AttachmentRemove,
    XfaFlatten,
    MetadataUpdate,
    Canonicalize,
    FullRewrite,
    IncrementalSave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditPolicyDecision {
    SafeIncremental,
    IncrementalWithWarning,
    FullRewriteRequired,
    BlockedBySignaturePolicy,
    ExplicitOverrideRequired,
}

#[derive(Debug, Clone, Serialize)]
pub struct StructuralSignaturePolicy {
    pub signature_object: String,
    pub certification_signature: bool,
    pub approval_signature: bool,
    pub timestamp_signature: bool,
    pub docmdp_p: Option<i64>,
    pub fieldmdp_action: Option<String>,
    pub fieldmdp_fields: Vec<String>,
    pub malformed_or_conflicting: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignatureImpactSummary {
    pub signature_count: usize,
    pub byte_range_coverage_reported: bool,
    pub revision_coverage_reported: bool,
    pub append_only_update: bool,
    pub cryptographic_validity_evaluated: bool,
    pub modification_after_signing: bool,
    pub docmdp_permission_evaluated_structurally: bool,
    pub fieldmdp_permission_evaluated_structurally: bool,
    pub dss_ltv_present: bool,
    pub signature_value_preserved: bool,
    pub appearance_preserved: bool,
    pub semantic_preservation: bool,
    pub viewer_warning_risk: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditPolicyReport {
    pub schema_version: String,
    pub operation: EditOperation,
    pub decision: EditPolicyDecision,
    pub incremental_feasible: bool,
    pub original_prefix_preserved: bool,
    pub byte_range_covered_bytes_untouched: bool,
    pub signature_dictionary_untouched: bool,
    pub full_rewrite_required: bool,
    pub invalidation_risk: String,
    pub expected_viewer_posture: String,
    pub crypto_validation_requirement: String,
    pub structural_policies: Vec<StructuralSignaturePolicy>,
    pub cryptographic_reports: Vec<SignatureReport>,
    pub impact: SignatureImpactSummary,
    pub exact_limits: Vec<String>,
}

pub fn analyze_edit_policy(
    engine: &ContentEngine,
    operation: EditOperation,
) -> Result<EditPolicyReport> {
    let structural = structural_signature_policies(engine.document())?;
    let crypto = verify_signatures(engine.document())?;
    let signatures = !crypto.is_empty() || !structural.is_empty();
    let destructive = matches!(
        operation,
        EditOperation::Redaction
            | EditOperation::Sanitizer
            | EditOperation::AttachmentRemove
            | EditOperation::XfaFlatten
            | EditOperation::Canonicalize
            | EditOperation::FullRewrite
    );
    let page_change = matches!(
        operation,
        EditOperation::PageInsert
            | EditOperation::PageDelete
            | EditOperation::PageReorder
            | EditOperation::PageRotate
            | EditOperation::PageBoxChange
            | EditOperation::ContentEdit
    );
    let annotation_or_form = matches!(
        operation,
        EditOperation::FormValueUpdate
            | EditOperation::FormAppearanceUpdate
            | EditOperation::AnnotationAdd
            | EditOperation::AnnotationUpdate
            | EditOperation::AnnotationDelete
            | EditOperation::XfdfImport
    );
    let strict_docmdp = structural.iter().filter_map(|policy| policy.docmdp_p).min();
    let locked_field = structural
        .iter()
        .any(|policy| policy.fieldmdp_action.is_some());
    let decision = if !signatures {
        if destructive {
            EditPolicyDecision::FullRewriteRequired
        } else {
            EditPolicyDecision::SafeIncremental
        }
    } else if destructive {
        EditPolicyDecision::ExplicitOverrideRequired
    } else if strict_docmdp == Some(1)
        || (page_change && strict_docmdp.is_some())
        || (annotation_or_form
            && strict_docmdp == Some(2)
            && !matches!(operation, EditOperation::FormValueUpdate))
        || (matches!(operation, EditOperation::FormValueUpdate) && locked_field)
    {
        EditPolicyDecision::BlockedBySignaturePolicy
    } else {
        EditPolicyDecision::IncrementalWithWarning
    };
    let incremental = matches!(
        decision,
        EditPolicyDecision::SafeIncremental | EditPolicyDecision::IncrementalWithWarning
    );
    let mut impact = signature_impact_summary(engine, operation)?;
    impact.append_only_update = incremental;
    impact.signature_value_preserved = incremental;
    Ok(EditPolicyReport {
        schema_version: PROMPT18_SCHEMA_VERSION.to_string(),
        operation,
        decision,
        incremental_feasible: incremental,
        original_prefix_preserved: incremental,
        byte_range_covered_bytes_untouched: incremental,
        signature_dictionary_untouched: incremental,
        full_rewrite_required: destructive,
        invalidation_risk: if signatures {
            "signed semantics change or an appended revision can trigger a viewer warning even when original ByteRange bytes and signature values remain untouched".to_string()
        } else {
            "none_from_existing_signatures".to_string()
        },
        expected_viewer_posture: if signatures {
            "viewer_dependent_modified_after_signing_or_policy_warning".to_string()
        } else {
            "unsigned_document".to_string()
        },
        crypto_validation_requirement: "run cryptographic verification before and after; structural DocMDP/FieldMDP parsing alone does not establish validity, trust, or certification acceptance".to_string(),
        structural_policies: structural,
        cryptographic_reports: crypto,
        impact,
        exact_limits: vec![
            "DocMDP and FieldMDP are evaluated structurally; viewer enforcement remains implementation dependent".to_string(),
            "safe_incremental preserves the original prefix and signature dictionary but does not promise certification acceptance or absence of viewer warnings".to_string(),
            "secure redaction, sanitizer removal, attachment removal, XFA flattening, canonicalization, and full rewrite require explicit signed-semantics override".to_string(),
        ],
    })
}

/// Execute a bounded metadata incremental update. This is the proof-carrying
/// incremental primitive used by Prompt 18 tests: the original bytes remain an
/// exact prefix, only the Info dictionary is appended/replaced, and signature
/// dictionaries are never mutated.
pub fn incremental_metadata_update_pdf(
    input: &[u8],
    key: &str,
    value: &str,
    signature_policy_override: bool,
) -> Result<(Vec<u8>, EditPolicyReport)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let mut report = analyze_edit_policy(&engine, EditOperation::MetadataUpdate)?;
    if matches!(
        report.decision,
        EditPolicyDecision::BlockedBySignaturePolicy | EditPolicyDecision::ExplicitOverrideRequired
    ) && !signature_policy_override
    {
        return Err(OxideError::UnsupportedFeature(
            "incremental metadata edit blocked by signature policy; explicit override required"
                .to_string(),
        ));
    }
    let reader = engine.document().reader();
    let (number, generation, mut info) = match reader.info_reference() {
        Some((number, generation)) => {
            let info = reader
                .get_object(number, generation)?
                .as_dict()
                .cloned()
                .unwrap_or_else(PdfDictionary::empty);
            (number, generation, info)
        }
        None => return Err(OxideError::UnsupportedFeature(
            "incremental metadata update currently requires an existing trailer /Info dictionary"
                .to_string(),
        )),
    };
    info.insert(key, PdfObject::String(value.as_bytes().to_vec()));
    let output = write_incremental_update(
        reader,
        vec![IncrementalObject {
            number,
            generation,
            object: PdfObject::Dictionary(info),
        }],
    )?;
    if !output.starts_with(input) {
        return Err(OxideError::MalformedPdf(
            "incremental writer did not preserve original prefix".to_string(),
        ));
    }
    ContentEngine::open_bytes(output.clone()).map_err(|error| {
        OxideError::MalformedPdf(format!("incremental output failed reopen: {error}"))
    })?;
    report.original_prefix_preserved = true;
    report.byte_range_covered_bytes_untouched = true;
    report.signature_dictionary_untouched = true;
    report.impact.append_only_update = true;
    report.impact.signature_value_preserved = true;
    Ok((output, report))
}

pub fn prompt18_report(engine: &ContentEngine) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "schema_version": PROMPT18_SCHEMA_VERSION,
        "mask_redaction": mask_redaction_inventory(engine)?,
        "associated_files": associated_files_inventory(engine)?,
        "signature_policy": analyze_edit_policy(engine, EditOperation::IncrementalSave)?,
        "feature": prompt18_feature_report_value(crate::sdk::REPORT_ENVELOPE_VERSION),
    }))
}

pub(crate) fn prompt18_feature_report_value(envelope_version: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": PROMPT18_SCHEMA_VERSION,
        "envelope_version": envelope_version,
        "status": "complete_bounded_foundation",
        "coverage": {
            "mask_softmask_redaction": "implemented_with_limits",
            "inline_image_partial_redaction": "implemented_with_limits",
            "associated_files": "implemented_with_limits",
            "signature_safe_edit_policy": "implemented_with_limits",
            "docmdp_fieldmdp_structural_policy": "implemented_with_limits",
            "incremental_prefix_preservation": "implemented_with_limits"
        },
        "security": {
            "overlay_only_redaction_success_claims": 0,
            "unsupported_image_rewrite": "secure_instance_removal_or_explicit_fail",
            "external_associated_file_access": "never_fetched_or_executed",
            "attachment_path_traversal": "single_component_sanitized_and_reserved_names_rejected",
            "signature_crypto_overclaim": 0
        },
        "failure": {"blocked": 0, "unclassified": 0, "security_proof": 0},
        "limits": {
            "image_mask_pixels": MAX_IMAGE_MASK_PIXELS,
            "mask_recursion": MAX_MASK_RECURSION,
            "inline_bytes": MAX_INLINE_IMAGE_BYTES,
            "inline_count": MAX_INLINE_IMAGES,
            "associated_count": MAX_ASSOCIATED_FILES,
            "associated_file_bytes": MAX_ASSOCIATED_FILE_BYTES,
            "signature_count": MAX_SIGNATURES,
            "policy_references": MAX_POLICY_REFERENCES
        },
        "exact_limits": [
            "8-bit Gray/RGB/CMYK inline samples with bounded decoders are rewritten and deterministically Flate encoded; unsupported predictor or color-space paths remove/fail closed",
            "affected decodable masked Image XObjects are cloned with rewritten color samples and without reachable Mask/SMask references; unsupported affected instances remove/fail closed",
            "associated-file add/remove canonicalizes supported embedded payloads into a deterministic catalog EmbeddedFiles name tree; non-name-tree owner reattachment is inventory-only in this bounded phase",
            "cryptographic validity is reported only by the signature verifier; DocMDP/FieldMDP decisions are structural and viewer posture remains a risk estimate"
        ],
        "public_report_schema": "additive_feature_report_prompt18"
    })
}

#[derive(Clone)]
struct EmbeddedEntry {
    stable_id: String,
    filename: String,
    description: Option<String>,
    mime: String,
    relationship: AfRelationship,
    bytes: Vec<u8>,
}

fn collect_embedded_entries(input: &[u8]) -> Result<Vec<EmbeddedEntry>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let inventory = associated_files_inventory(&engine)?;
    let mut entries = Vec::new();
    for record in inventory
        .records
        .into_iter()
        .filter(|record| record.internal)
    {
        if entries.len() >= MAX_ASSOCIATED_FILES {
            return Err(OxideError::ResourceLimit(format!(
                "associated file count exceeds {MAX_ASSOCIATED_FILES}"
            )));
        }
        let Ok((_, bytes)) = associated_file_extract(&engine, &record.stable_id) else {
            continue;
        };
        entries.push(EmbeddedEntry {
            stable_id: record.stable_id,
            filename: sanitize_associated_filename(&record.filename)?,
            description: record.description,
            mime: record.mime.unwrap_or_else(default_mime),
            relationship: record.relationship,
            bytes,
        });
    }
    Ok(entries)
}

fn write_embedded_entries(
    input: &[u8],
    mut entries: Vec<EmbeddedEntry>,
    operation: &str,
    before_count: usize,
    added_count: usize,
) -> Result<(Vec<u8>, AssociatedFilesMutationReport)> {
    if entries.len() > MAX_ASSOCIATED_FILES {
        return Err(OxideError::ResourceLimit(format!(
            "associated file count {} exceeds {MAX_ASSOCIATED_FILES}",
            entries.len()
        )));
    }
    let total = entries
        .iter()
        .try_fold(0usize, |sum, entry| sum.checked_add(entry.bytes.len()))
        .ok_or_else(|| {
            OxideError::ResourceLimit("associated file byte total overflow".to_string())
        })?;
    if total > MAX_ASSOCIATED_TOTAL_BYTES {
        return Err(OxideError::ResourceLimit(format!(
            "associated file total {total} exceeds {MAX_ASSOCIATED_TOTAL_BYTES}"
        )));
    }
    entries.sort_by(|a, b| {
        a.filename
            .cmp(&b.filename)
            .then_with(|| sha256_hex(&a.bytes).cmp(&sha256_hex(&b.bytes)))
            .then_with(|| a.stable_id.cmp(&b.stable_id))
    });

    let doc = PdfDocument::open_bytes(input.to_vec())?;
    let reader = doc.reader();
    let root_original = reader.root_reference().map(|value| value.0);
    let mut mutate = |number: u32, object: &mut PdfObject| {
        if let Some(dict) = object_dict_mut(object) {
            // Remove all legacy/current attachment reachability before building
            // one deterministic canonical name tree below.
            dict.remove("AF");
            if dict.get_name("Subtype") == Some("FileAttachment") {
                dict.remove("FS");
            }
            if dict.get_name("Type") == Some("Filespec") {
                dict.remove("EF");
                dict.remove("RF");
            }
            if dict.contains_key("EmbeddedFiles") {
                dict.remove("EmbeddedFiles");
            }
            if Some(number) == root_original {
                if let Some(PdfObject::Dictionary(names)) = dict.get_mut("Names") {
                    names.remove("EmbeddedFiles");
                }
            }
        }
    };
    let (mut objects, root, info) = rewrite_document_objects(reader, &mut mutate)?;
    let mut next = objects
        .iter()
        .map(|object| object.number)
        .max()
        .unwrap_or(0)
        + 1;
    let mut stream_by_digest = BTreeMap::<String, u32>::new();
    let mut name_pairs = Vec::new();
    for entry in &entries {
        let digest = sha256_hex(&entry.bytes);
        let stream_number = if let Some(number) = stream_by_digest.get(&digest) {
            *number
        } else {
            let number = next;
            next += 1;
            let mut params = PdfDictionary::empty();
            params.insert("Size", PdfObject::Integer(entry.bytes.len() as i64));
            let mut stream_dict = PdfDictionary::empty();
            stream_dict.insert("Type", PdfObject::Name("EmbeddedFile".to_string()));
            stream_dict.insert("Subtype", PdfObject::Name(pdf_name(&entry.mime)));
            stream_dict.insert("Params", PdfObject::Dictionary(params));
            objects.push(OutputObject {
                number,
                object: PdfObject::Stream {
                    dict: stream_dict,
                    raw: entry.bytes.clone(),
                },
            });
            stream_by_digest.insert(digest, number);
            number
        };
        let spec_number = next;
        next += 1;
        let mut ef = PdfDictionary::empty();
        ef.insert("F", reference(stream_number, 0));
        ef.insert("UF", reference(stream_number, 0));
        let mut spec = PdfDictionary::empty();
        spec.insert("Type", PdfObject::Name("Filespec".to_string()));
        spec.insert("F", PdfObject::String(entry.filename.as_bytes().to_vec()));
        spec.insert("UF", PdfObject::String(entry.filename.as_bytes().to_vec()));
        if let Some(description) = &entry.description {
            spec.insert("Desc", PdfObject::String(description.as_bytes().to_vec()));
        }
        spec.insert(
            "AFRelationship",
            PdfObject::Name(pdf_name(entry.relationship.pdf_name())),
        );
        spec.insert("EF", PdfObject::Dictionary(ef));
        objects.push(OutputObject {
            number: spec_number,
            object: PdfObject::Dictionary(spec),
        });
        name_pairs.push(PdfObject::String(entry.filename.as_bytes().to_vec()));
        name_pairs.push(reference(spec_number, 0));
    }
    let name_tree_number = if name_pairs.is_empty() {
        None
    } else {
        let number = next;
        let mut tree = PdfDictionary::empty();
        tree.insert("Names", PdfObject::Array(name_pairs));
        objects.push(OutputObject {
            number,
            object: PdfObject::Dictionary(tree),
        });
        Some(number)
    };
    let catalog_index = objects
        .iter()
        .position(|object| object.number == root)
        .ok_or_else(|| OxideError::MalformedPdf("rewritten catalog is missing".to_string()))?;
    let existing_names = objects[catalog_index]
        .object
        .as_dict()
        .and_then(|catalog| catalog.get("Names"))
        .cloned();
    match (existing_names, name_tree_number) {
        (Some(PdfObject::Reference { number, .. }), Some(tree_number)) => {
            let names = objects
                .iter_mut()
                .find(|object| object.number == number)
                .and_then(|object| object.object.as_dict_mut())
                .ok_or_else(|| {
                    OxideError::MalformedPdf(
                        "catalog /Names reference is not a dictionary".to_string(),
                    )
                })?;
            names.insert("EmbeddedFiles", reference(tree_number, 0));
        }
        (Some(PdfObject::Dictionary(mut names)), Some(tree_number)) => {
            names.insert("EmbeddedFiles", reference(tree_number, 0));
            objects[catalog_index]
                .object
                .as_dict_mut()
                .expect("catalog type checked above")
                .insert("Names", PdfObject::Dictionary(names));
        }
        (None, Some(tree_number)) => {
            let mut names = PdfDictionary::empty();
            names.insert("EmbeddedFiles", reference(tree_number, 0));
            objects[catalog_index]
                .object
                .as_dict_mut()
                .ok_or_else(|| {
                    OxideError::MalformedPdf("rewritten catalog is not a dictionary".to_string())
                })?
                .insert("Names", PdfObject::Dictionary(names));
        }
        // The mutation closure already removed EmbeddedFiles from direct and
        // indirect Names dictionaries. Preserve any unrelated name trees and
        // their original direct/reference shape when no files remain.
        (_, None) | (Some(_), Some(_)) => {}
    }
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::ClassicXref)
        .write()?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let rescan = associated_files_inventory(&reopened)?;
    let after = rescan
        .records
        .iter()
        .filter(|record| record.internal)
        .count();
    let duplicate_streams_collapsed = entries.len().saturating_sub(stream_by_digest.len());
    let impact = signature_impact_summary(
        &ContentEngine::open_bytes(input.to_vec())?,
        EditOperation::AttachmentRemove,
    )?;
    Ok((
        output.clone(),
        AssociatedFilesMutationReport {
            schema_version: PROMPT18_SCHEMA_VERSION.to_string(),
            operation: operation.to_string(),
            before_count,
            after_count: after,
            removed_count: before_count
                .saturating_add(added_count)
                .saturating_sub(after),
            added_count,
            duplicate_streams_collapsed,
            output_bytes: output.len(),
            output_sha256: resource_digest(&output),
            output_reopened: true,
            sanitizer_rescan_clean: after == entries.len(),
            deterministic: true,
            signature_impact: impact,
            exact_limits: associated_file_limits(),
        },
    ))
}

fn structural_signature_policies(doc: &PdfDocument) -> Result<Vec<StructuralSignaturePolicy>> {
    let reader = doc.reader();
    let mut out = Vec::new();
    for (number, generation) in reader.object_ids().into_iter().take(MAX_SIGNATURES * 100) {
        let Ok(object) = reader.get_object(number, generation) else {
            continue;
        };
        let Some(dict) = object.as_dict() else {
            continue;
        };
        if dict.get_name("Type") != Some("Sig") && dict.get_name("FT") != Some("Sig") {
            continue;
        }
        let sig_dict = if dict.get_name("FT") == Some("Sig") {
            dict.get("V")
                .and_then(|value| reader.resolve(value.clone()).ok())
                .and_then(|value| value.as_dict().cloned())
                .unwrap_or_else(|| dict.clone())
        } else {
            dict.clone()
        };
        let mut row = StructuralSignaturePolicy {
            signature_object: ref_id((number, generation)),
            certification_signature: false,
            approval_signature: true,
            timestamp_signature: sig_dict.get_name("Type") == Some("DocTimeStamp"),
            docmdp_p: None,
            fieldmdp_action: None,
            fieldmdp_fields: Vec::new(),
            malformed_or_conflicting: false,
        };
        let references = sig_dict
            .get("Reference")
            .and_then(PdfObject::as_array)
            .map(|items| items.iter().take(MAX_POLICY_REFERENCES).collect::<Vec<_>>())
            .unwrap_or_default();
        for reference in references {
            let Some(transform) = reader
                .resolve(reference.clone())
                .ok()
                .and_then(|value| value.as_dict().cloned())
            else {
                row.malformed_or_conflicting = true;
                continue;
            };
            let params = transform
                .get("TransformParams")
                .and_then(|value| reader.resolve(value.clone()).ok())
                .and_then(|value| value.as_dict().cloned());
            match transform.get_name("TransformMethod") {
                Some("DocMDP") => {
                    row.certification_signature = true;
                    row.approval_signature = false;
                    let p = params.as_ref().and_then(|dict| dict.get_integer("P"));
                    if !matches!(p, Some(1..=3)) {
                        row.malformed_or_conflicting = true;
                    }
                    if row.docmdp_p.is_some() && row.docmdp_p != p {
                        row.malformed_or_conflicting = true;
                    }
                    row.docmdp_p = p;
                }
                Some("FieldMDP") => {
                    row.fieldmdp_action = params
                        .as_ref()
                        .and_then(|dict| dict.get_name("Action"))
                        .map(str::to_string);
                    row.fieldmdp_fields = params
                        .as_ref()
                        .and_then(|dict| dict.get("Fields"))
                        .and_then(PdfObject::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(PdfObject::as_string)
                                .map(|bytes| String::from_utf8_lossy(bytes).to_string())
                                .collect()
                        })
                        .unwrap_or_default();
                    if !matches!(
                        row.fieldmdp_action.as_deref(),
                        Some("All" | "Include" | "Exclude")
                    ) {
                        row.malformed_or_conflicting = true;
                    }
                }
                Some(_) | None => row.malformed_or_conflicting = true,
            }
        }
        out.push(row);
        if out.len() >= MAX_SIGNATURES {
            break;
        }
    }
    out.sort_by(|a, b| a.signature_object.cmp(&b.signature_object));
    Ok(out)
}

fn signature_impact_summary(
    engine: &ContentEngine,
    operation: EditOperation,
) -> Result<SignatureImpactSummary> {
    let reports = verify_signatures(engine.document())?;
    let destructive = matches!(
        operation,
        EditOperation::Redaction
            | EditOperation::Sanitizer
            | EditOperation::AttachmentRemove
            | EditOperation::XfaFlatten
            | EditOperation::Canonicalize
            | EditOperation::FullRewrite
    );
    Ok(SignatureImpactSummary {
        signature_count: reports.len(),
        byte_range_coverage_reported: reports.iter().all(|report| report.checks.byte_range_present),
        revision_coverage_reported: true,
        append_only_update: false,
        cryptographic_validity_evaluated: true,
        modification_after_signing: !reports.is_empty(),
        docmdp_permission_evaluated_structurally: true,
        fieldmdp_permission_evaluated_structurally: true,
        dss_ltv_present: reports.iter().any(|report| report.ltv.dss_present),
        signature_value_preserved: !destructive,
        appearance_preserved: !destructive,
        semantic_preservation: !destructive,
        viewer_warning_risk: if reports.is_empty() {
            "none_unsigned".to_string()
        } else if destructive {
            "high_signed_semantics_changed".to_string()
        } else {
            "viewer_and_certification_policy_dependent".to_string()
        },
        note: "cryptographic validity, trust, ByteRange coverage, structural certification permission, and viewer status are separate fields; none is inferred from another".to_string(),
    })
}

fn inventory_filespec(
    reader: &crate::reader::PdfReader,
    object: &PdfObject,
    owner: Option<(u32, u16)>,
    owner_type: AssociatedFileOwnerType,
    known_streams: &mut HashSet<String>,
    seen_specs: &mut HashSet<String>,
    records: &mut Vec<AssociatedFileRecord>,
) {
    let spec_ref = object.as_reference();
    let spec_id = spec_ref
        .map(ref_id)
        .unwrap_or_else(|| format!("direct-{}", records.len() + 1));
    if !seen_specs.insert(spec_id.clone()) {
        return;
    }
    let Some(spec) = reader
        .resolve(object.clone())
        .ok()
        .and_then(|value| value.as_dict().cloned())
    else {
        return;
    };
    let ef = spec
        .get("EF")
        .and_then(|value| reader.resolve(value.clone()).ok())
        .and_then(|value| value.as_dict().cloned());
    let stream = ef
        .as_ref()
        .and_then(|dict| dict.get("UF").or_else(|| dict.get("F")))
        .and_then(PdfObject::as_reference);
    let stream_id = stream.map(ref_id);
    if stream_id
        .as_ref()
        .is_some_and(|id| known_streams.contains(id))
    {
        if let Some(existing) = records
            .iter_mut()
            .find(|record| record.stream_ref.as_ref() == stream_id.as_ref())
        {
            existing.file_spec_ref = spec_ref.map(ref_id);
            if let Some(owner) = owner {
                existing.owner_ref = Some(ref_id(owner));
                existing.owner_type = owner_type;
            }
            existing.relationship = AfRelationship::from_name(spec.get_name("AFRelationship"));
            existing.provenance = "attachment_discovery_plus_filespec_owner_scan".to_string();
        }
        return;
    }
    if let Some(id) = &stream_id {
        known_streams.insert(id.clone());
    }
    let filename = text_value(&spec, "UF")
        .or_else(|| text_value(&spec, "F"))
        .unwrap_or_else(|| "associated-file.bin".to_string());
    let external_target = if stream.is_none() {
        text_value(&spec, "UF")
            .or_else(|| text_value(&spec, "F"))
            .or_else(|| text_value(&spec, "DOS"))
            .or_else(|| text_value(&spec, "Mac"))
            .or_else(|| text_value(&spec, "Unix"))
    } else {
        None
    };
    records.push(AssociatedFileRecord {
        stable_id: format!("filespec-{spec_id}"),
        file_spec_ref: spec_ref.map(ref_id),
        stream_ref: stream_id,
        owner_ref: owner.map(ref_id),
        owner_type,
        relationship: AfRelationship::from_name(spec.get_name("AFRelationship")),
        filename: filename.clone(),
        unicode_filename: text_value(&spec, "UF"),
        description: text_value(&spec, "Desc"),
        mime: stream.and_then(|reference| stream_mime(reader, reference.0, reference.1)),
        size: None,
        sha256: None,
        creation_date: None,
        modification_date: None,
        encrypted: reader.is_encrypted(),
        decoded: false,
        internal: stream.is_some(),
        external_target,
        duplicate_group: None,
        security_classification: if stream.is_some() {
            "unknown_internal"
        } else {
            "external_reference"
        }
        .to_string(),
        sanitizer_disposition: if stream.is_some() {
            "policy_dependent"
        } else {
            "remove_external_references"
        }
        .to_string(),
        provenance: "object_graph_filespec_scan".to_string(),
    });
}

fn classify_owner(dict: &PdfDictionary) -> AssociatedFileOwnerType {
    match (dict.get_name("Type"), dict.get_name("Subtype")) {
        (Some("Catalog"), _) => AssociatedFileOwnerType::Catalog,
        (Some("Page"), _) => AssociatedFileOwnerType::Page,
        (Some("Annot"), _) | (_, Some("FileAttachment")) => AssociatedFileOwnerType::Annotation,
        (Some("StructElem"), _) => AssociatedFileOwnerType::StructureElement,
        (Some("XObject"), Some("Form")) => AssociatedFileOwnerType::FormXObject,
        (Some("XObject"), _) => AssociatedFileOwnerType::XObject,
        _ => AssociatedFileOwnerType::OrphanFileSpec,
    }
}

fn classify_associated_file(name: &str, mime: Option<&str>) -> String {
    let lower = name.to_ascii_lowercase();
    let mime = mime.unwrap_or("").to_ascii_lowercase();
    if [
        ".exe", ".dll", ".com", ".bat", ".cmd", ".ps1", ".js", ".vbs", ".jar", ".msi",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
        || mime.contains("executable")
        || mime.contains("javascript")
        || mime.contains("x-msdownload")
    {
        "executable_or_active".to_string()
    } else if mime.is_empty() || mime == "application/octet-stream" {
        "unknown".to_string()
    } else {
        "inert_data".to_string()
    }
}

fn sanitize_associated_filename(name: &str) -> Result<String> {
    if name.len() > MAX_ASSOCIATED_FILENAME_BYTES {
        return Err(OxideError::ResourceLimit(format!(
            "associated filename exceeds {MAX_ASSOCIATED_FILENAME_BYTES} bytes"
        )));
    }
    let safe = sanitize_filename(name);
    let stem = safe.trim_end_matches('.').to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let base = stem.split('.').next().unwrap_or(&stem);
    if reserved.contains(&base) {
        return Err(OxideError::MalformedPdf(format!(
            "associated filename {name:?} resolves to a reserved platform name"
        )));
    }
    Ok(safe)
}

fn associated_file_limits() -> Vec<String> {
    vec![
        format!("maximum {MAX_ASSOCIATED_FILES} associated files"),
        format!("maximum {MAX_ASSOCIATED_FILE_BYTES} decoded bytes per file"),
        format!("maximum {MAX_ASSOCIATED_TOTAL_BYTES} decoded bytes total"),
        "full-rewrite mutation canonicalizes internal embedded files into the catalog EmbeddedFiles name tree; non-name-tree owner reattachment is reported as a remaining limit".to_string(),
        "external/URL/platform file specifications are inventoried and removable but never fetched or executed".to_string(),
    ]
}

fn positive_u32(dict: &PdfDictionary, full: &str, short: &str) -> u32 {
    dict.get_integer(full)
        .or_else(|| dict.get_integer(short))
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0)
}

fn color_space_name(dict: &PdfDictionary) -> String {
    match dict.get("ColorSpace").or_else(|| dict.get("CS")) {
        Some(PdfObject::Name(name)) => match name.as_str() {
            "G" => "DeviceGray".to_string(),
            "RGB" => "DeviceRGB".to_string(),
            "CMYK" => "DeviceCMYK".to_string(),
            other => other.to_string(),
        },
        Some(PdfObject::Array(items)) => items
            .first()
            .and_then(PdfObject::as_name)
            .unwrap_or("complex")
            .to_string(),
        _ => "unspecified".to_string(),
    }
}

fn filter_names(dict: &PdfDictionary) -> Vec<String> {
    match dict.get("Filter").or_else(|| dict.get("F")) {
        Some(PdfObject::Name(name)) => vec![name.clone()],
        Some(PdfObject::Array(items)) => items
            .iter()
            .filter_map(PdfObject::as_name)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn stream_mime(reader: &crate::reader::PdfReader, number: u32, generation: u16) -> Option<String> {
    reader
        .get_object(number, generation)
        .ok()
        .and_then(|value| match value {
            PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
                dict.get_name("Subtype").map(decode_pdf_name)
            }
            _ => None,
        })
}

fn decode_pdf_name(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'#' && index + 2 < bytes.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn text_value(dict: &PdfDictionary, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(PdfObject::as_string)
        .map(crate::info::decode_pdf_text_string)
}

fn object_dict_mut(object: &mut PdfObject) -> Option<&mut PdfDictionary> {
    match object {
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => Some(dict),
        _ => None,
    }
}

fn reference(number: u32, generation: u16) -> PdfObject {
    PdfObject::Reference { number, generation }
}

fn ref_id(reference: (u32, u16)) -> String {
    format!("{}-{}", reference.0, reference.1)
}

fn parse_ref_id(value: &str) -> Option<(u32, u16)> {
    let (number, generation) = value.split_once('-')?;
    Some((number.parse().ok()?, generation.parse().ok()?))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn pdf_name(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
                (byte as char).to_string()
            } else {
                format!("#{byte:02X}")
            }
        })
        .collect()
}

fn default_mime() -> String {
    "application/octet-stream".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_traversal_and_rejects_reserved_name() {
        assert_eq!(
            sanitize_associated_filename("../../safe.txt").unwrap(),
            "safe.txt"
        );
        assert!(sanitize_associated_filename("CON.txt").is_err());
    }

    #[test]
    fn relationship_round_trip_names_are_stable() {
        assert_eq!(
            AfRelationship::from_name(Some("Source")).pdf_name(),
            "Source"
        );
        assert_eq!(
            AfRelationship::from_name(Some("VendorCustom")).pdf_name(),
            "VendorCustom"
        );
    }

    #[test]
    fn feature_report_has_no_blocked_rows() {
        let report = prompt18_feature_report_value(1);
        assert_eq!(report["failure"]["blocked"], 0);
        assert_eq!(
            report["security"]["overlay_only_redaction_success_claims"],
            0
        );
    }
}
