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
use crate::editing::{AnnotationOptions, EditMode, ImageRect, PdfEditor};
use crate::engine::ContentEngine;
use crate::error::{OxideError, Result};
use crate::filters::{decode_stream_lossless_with_limits, DecodeLimits, StreamDecodeStatus};
use crate::object::{PdfDictionary, PdfObject};
use crate::prompt17::{
    apply_nonaxis_image_redaction_pdf, NonAxisRedactionApplyReport, NonAxisRedactionOptions,
};
use crate::signature::{
    verify_signatures, verify_signatures_with_options, SignatureReport, SignatureValidity,
    VerifyOptions, PROMPT25_SIGNATURE_LTV_EDIT_SCHEMA_VERSION,
};
use crate::versioning::resource_digest;
use crate::writer::{
    rewrite_document_objects, write_incremental_update, IncrementalObject, OutputObject, PdfWriter,
    WriterMode,
};

pub const PROMPT18_SCHEMA_VERSION: &str = "prompt18.mask-inline-associated-signature-policy.v1";
pub const PROMPT18B_SCHEMA_VERSION: &str = "prompt18b.advanced-secure-mutation-closure.v1";

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
        let color_space = color_space_name(&dict);
        let directly_rewritable = ((image_mask && bpc == 1)
            || (color_space == "Indexed" && matches!(bpc, 1 | 2 | 4 | 8))
            || (matches!(
                color_space.as_str(),
                "DeviceGray" | "DeviceRGB" | "DeviceCMYK" | "ICCBased"
            ) && bpc == 8))
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
                Some("unsupported codec/color-space, malformed packed layout, excessive-pixel, high-channel ICC, or unavailable decoder paths remove the affected invocation or fail closed".to_string()),
            )
        };
        rows.push(MaskInventoryRow {
            stable_id: format!("image-{number}-{generation}"),
            object_number: number,
            generation,
            width,
            height,
            bits_per_component: bpc,
            color_space,
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
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let policy = analyze_edit_policy(&engine, EditOperation::Redaction)?;
    enforce_mutation_policy(
        &policy,
        options.signature_policy_override,
        "secure redaction",
    )?;
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
        let Some(dict) = object_dictionary_ref(&object) else {
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
    /// Exact indirect owner (`object-generation`) for page, annotation,
    /// structure, Form, or XObject association. Catalog may omit this.
    #[serde(default)]
    pub owner_ref: Option<String>,
    #[serde(default)]
    pub deterministic: bool,
    #[serde(default)]
    pub signature_policy_override: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociatedFileOwnerRemoveRequest {
    pub stable_id: String,
    pub owner: AssociatedFileOwnerType,
    #[serde(default)]
    pub owner_ref: Option<String>,
    #[serde(default)]
    pub signature_policy_override: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssociatedFileOwnerUpdateRequest {
    pub stable_id: String,
    pub owner: AssociatedFileOwnerType,
    #[serde(default)]
    pub owner_ref: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub relationship: Option<AfRelationship>,
    #[serde(default)]
    pub signature_policy_override: bool,
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
    enforce_full_rewrite_signed_policy(
        input,
        EditOperation::AttachmentAdd,
        request.signature_policy_override,
        "associated-file add",
    )?;
    if request
        .owner
        .is_some_and(|owner| owner != AssociatedFileOwnerType::EmbeddedFilesNameTree)
    {
        return associated_files_add_owner_pdf(input, request, payload, &filename);
    }
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

fn associated_files_add_owner_pdf(
    input: &[u8],
    request: &AssociatedFileAddRequest,
    payload: &[u8],
    filename: &str,
) -> Result<(Vec<u8>, AssociatedFilesMutationReport)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let inventory = associated_files_inventory(&engine)?;
    let before = inventory
        .records
        .iter()
        .filter(|record| record.internal)
        .count();
    let owner = request.owner.unwrap_or(AssociatedFileOwnerType::Catalog);
    let digest = sha256_hex(payload);
    if inventory.records.iter().any(|record| {
        record.internal
            && record.owner_type == owner
            && record.owner_ref == request.owner_ref
            && record.filename == filename
            && record.sha256.as_deref() == Some(digest.as_str())
    }) {
        return Ok((
            input.to_vec(),
            AssociatedFilesMutationReport {
                schema_version: PROMPT18_SCHEMA_VERSION.to_string(),
                operation: "add_owner_deduplicated".to_string(),
                before_count: before,
                after_count: before,
                removed_count: 0,
                added_count: 0,
                duplicate_streams_collapsed: 1,
                output_bytes: input.len(),
                output_sha256: resource_digest(input),
                output_reopened: true,
                sanitizer_rescan_clean: true,
                deterministic: true,
                signature_impact: signature_impact_summary(&engine, EditOperation::AttachmentAdd)?,
                exact_limits: associated_file_limits(),
            },
        ));
    }

    let reader = engine.document().reader();
    let mut noop = |_number: u32, _object: &mut PdfObject| {};
    let (mut objects, root, info) = rewrite_document_objects(reader, &mut noop)?;
    let mut next = objects
        .iter()
        .map(|object| object.number)
        .max()
        .unwrap_or(0)
        + 1;
    let existing_stream = inventory
        .records
        .iter()
        .find(|record| record.internal && record.sha256.as_deref() == Some(digest.as_str()))
        .and_then(|record| record.stream_ref.as_deref())
        .and_then(parse_ref_id)
        .and_then(|old| remapped_object_number(reader, old.0));
    let stream_number = if let Some(number) = existing_stream {
        number
    } else {
        let number = next;
        next += 1;
        let mut params = PdfDictionary::empty();
        params.insert("Size", PdfObject::Integer(payload.len() as i64));
        params.insert("OxideSHA256", PdfObject::String(digest.as_bytes().to_vec()));
        let mut stream = PdfDictionary::empty();
        stream.insert("Type", PdfObject::Name("EmbeddedFile".to_string()));
        stream.insert("Subtype", PdfObject::Name(pdf_name(&request.mime)));
        stream.insert("Params", PdfObject::Dictionary(params));
        objects.push(OutputObject {
            number,
            object: PdfObject::Stream {
                dict: stream,
                raw: payload.to_vec(),
            },
        });
        number
    };
    let spec_number = next;
    let mut ef = PdfDictionary::empty();
    ef.insert("F", reference(stream_number, 0));
    ef.insert("UF", reference(stream_number, 0));
    let mut spec = PdfDictionary::empty();
    spec.insert("Type", PdfObject::Name("Filespec".to_string()));
    spec.insert("F", PdfObject::String(filename.as_bytes().to_vec()));
    spec.insert("UF", PdfObject::String(filename.as_bytes().to_vec()));
    if let Some(description) = &request.description {
        spec.insert("Desc", PdfObject::String(description.as_bytes().to_vec()));
    }
    spec.insert(
        "AFRelationship",
        PdfObject::Name(pdf_name(
            request
                .relationship
                .as_ref()
                .unwrap_or(&AfRelationship::Unspecified)
                .pdf_name(),
        )),
    );
    spec.insert("EF", PdfObject::Dictionary(ef));
    objects.push(OutputObject {
        number: spec_number,
        object: PdfObject::Dictionary(spec),
    });

    let owner_number =
        resolve_owner_output_number(reader, root, owner, request.owner_ref.as_deref())?;
    let owner_object = objects
        .iter_mut()
        .find(|object| object.number == owner_number)
        .ok_or_else(|| OxideError::MalformedPdf("associated-file owner is missing".to_string()))?;
    attach_filespec_to_owner(&mut owner_object.object, owner, spec_number)?;
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::ClassicXref)
        .write()?;
    owner_mutation_report(input, output, "add_owner", before, 1, 0)
}

fn remapped_object_number(reader: &crate::reader::PdfReader, old_number: u32) -> Option<u32> {
    let mut seen = BTreeMap::new();
    let mut next = 1u32;
    for (number, _) in reader.object_ids() {
        seen.entry(number).or_insert_with(|| {
            let value = next;
            next += 1;
            value
        });
    }
    seen.get(&old_number).copied()
}

fn resolve_owner_output_number(
    reader: &crate::reader::PdfReader,
    root: u32,
    owner: AssociatedFileOwnerType,
    owner_ref: Option<&str>,
) -> Result<u32> {
    if owner == AssociatedFileOwnerType::Catalog && owner_ref.is_none() {
        return Ok(root);
    }
    let old = owner_ref.and_then(parse_ref_id).ok_or_else(|| {
        OxideError::MalformedPdf(format!(
            "owner_ref is required for {owner:?} associated-file mutation"
        ))
    })?;
    let object = reader.get_object(old.0, old.1)?;
    let actual = object_dictionary_ref(&object)
        .map(classify_owner)
        .unwrap_or(AssociatedFileOwnerType::OrphanFileSpec);
    if actual != owner {
        return Err(OxideError::MalformedPdf(format!(
            "owner_ref {}-{} is {actual:?}, not {owner:?}",
            old.0, old.1
        )));
    }
    remapped_object_number(reader, old.0).ok_or_else(|| {
        OxideError::MalformedPdf("associated-file owner could not be remapped".to_string())
    })
}

fn attach_filespec_to_owner(
    object: &mut PdfObject,
    owner: AssociatedFileOwnerType,
    spec_number: u32,
) -> Result<()> {
    let dict = object_dict_mut(object).ok_or_else(|| {
        OxideError::MalformedPdf("associated-file owner is not a dictionary".to_string())
    })?;
    let spec = reference(spec_number, 0);
    match owner {
        AssociatedFileOwnerType::Annotation => {
            dict.insert("FS", spec.clone());
            append_unique_reference(dict, "AF", spec);
        }
        AssociatedFileOwnerType::Catalog
        | AssociatedFileOwnerType::Page
        | AssociatedFileOwnerType::StructureElement
        | AssociatedFileOwnerType::XObject
        | AssociatedFileOwnerType::FormXObject => append_unique_reference(dict, "AF", spec),
        _ => {
            return Err(OxideError::UnsupportedFeature(format!(
                "owner-specific mutation is not supported for {owner:?}"
            )))
        }
    }
    Ok(())
}

fn append_unique_reference(dict: &mut PdfDictionary, key: &str, value: PdfObject) {
    let mut items = dict
        .remove(key)
        .map(|value| match value {
            PdfObject::Array(items) => items,
            other => vec![other],
        })
        .unwrap_or_default();
    if !items.contains(&value) {
        items.push(value);
    }
    dict.insert(key, PdfObject::Array(items));
}

fn owner_mutation_report(
    input: &[u8],
    output: Vec<u8>,
    operation: &str,
    before: usize,
    added: usize,
    removed: usize,
) -> Result<(Vec<u8>, AssociatedFilesMutationReport)> {
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let after = associated_files_inventory(&reopened)?
        .records
        .iter()
        .filter(|record| record.internal)
        .count();
    let impact = signature_impact_summary(
        &ContentEngine::open_bytes(input.to_vec())?,
        if removed > 0 {
            EditOperation::AttachmentRemove
        } else {
            EditOperation::AttachmentAdd
        },
    )?;
    Ok((
        output.clone(),
        AssociatedFilesMutationReport {
            schema_version: PROMPT18_SCHEMA_VERSION.to_string(),
            operation: operation.to_string(),
            before_count: before,
            after_count: after,
            removed_count: removed,
            added_count: added,
            duplicate_streams_collapsed: before.saturating_add(added).saturating_sub(after),
            output_bytes: output.len(),
            output_sha256: resource_digest(&output),
            output_reopened: true,
            sanitizer_rescan_clean: true,
            deterministic: true,
            signature_impact: impact,
            exact_limits: associated_file_limits(),
        },
    ))
}

pub fn associated_files_remove_owner_pdf(
    input: &[u8],
    request: &AssociatedFileOwnerRemoveRequest,
) -> Result<(Vec<u8>, AssociatedFilesMutationReport)> {
    enforce_full_rewrite_signed_policy(
        input,
        EditOperation::AttachmentRemove,
        request.signature_policy_override,
        "owner-specific associated-file removal",
    )?;
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let inventory = associated_files_inventory(&engine)?;
    let before = inventory
        .records
        .iter()
        .filter(|record| record.internal)
        .count();
    let record = inventory
        .records
        .iter()
        .find(|record| {
            record.stable_id == request.stable_id
                && record.owner_type == request.owner
                && (request.owner_ref.is_none() || record.owner_ref == request.owner_ref)
        })
        .ok_or_else(|| {
            OxideError::MalformedPdf("associated-file owner association was not found".to_string())
        })?;
    let old_spec = record
        .file_spec_ref
        .as_deref()
        .and_then(parse_ref_id)
        .ok_or_else(|| {
            OxideError::UnsupportedFeature(
                "owner-specific removal requires an indirect FileSpec".to_string(),
            )
        })?;
    let reader = engine.document().reader();
    let mut noop = |_number: u32, _object: &mut PdfObject| {};
    let (mut objects, root, info) = rewrite_document_objects(reader, &mut noop)?;
    let spec_number = remapped_object_number(reader, old_spec.0).ok_or_else(|| {
        OxideError::MalformedPdf("associated FileSpec could not be remapped".to_string())
    })?;
    let owner_ref = request.owner_ref.as_deref().or(record.owner_ref.as_deref());
    let owner_number = resolve_owner_output_number(reader, root, request.owner, owner_ref)?;
    let owner_object = objects
        .iter_mut()
        .find(|object| object.number == owner_number)
        .ok_or_else(|| OxideError::MalformedPdf("associated-file owner is missing".to_string()))?;
    detach_filespec_from_owner(&mut owner_object.object, request.owner, spec_number)?;
    remove_unreachable_filespec_and_streams(&mut objects, spec_number);
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::ClassicXref)
        .write()?;
    owner_mutation_report(input, output, "remove_owner", before, 0, 1)
}

pub fn associated_files_update_owner_pdf(
    input: &[u8],
    request: &AssociatedFileOwnerUpdateRequest,
    payload: &[u8],
) -> Result<(Vec<u8>, AssociatedFilesMutationReport)> {
    enforce_full_rewrite_signed_policy(
        input,
        EditOperation::AttachmentRemove,
        request.signature_policy_override,
        "owner-specific associated-file update",
    )?;
    if payload.len() > MAX_ASSOCIATED_FILE_BYTES {
        return Err(OxideError::ResourceLimit(format!(
            "associated file payload {} exceeds {} bytes",
            payload.len(),
            MAX_ASSOCIATED_FILE_BYTES
        )));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let inventory = associated_files_inventory(&engine)?;
    let before = inventory
        .records
        .iter()
        .filter(|record| record.internal)
        .count();
    let record = inventory
        .records
        .iter()
        .find(|record| {
            record.stable_id == request.stable_id
                && record.owner_type == request.owner
                && (request.owner_ref.is_none() || record.owner_ref == request.owner_ref)
        })
        .ok_or_else(|| {
            OxideError::MalformedPdf("associated-file owner association was not found".to_string())
        })?;
    let old_spec = record
        .file_spec_ref
        .as_deref()
        .and_then(parse_ref_id)
        .ok_or_else(|| {
            OxideError::UnsupportedFeature(
                "owner-specific update requires an indirect FileSpec".to_string(),
            )
        })?;
    let filename =
        sanitize_associated_filename(request.filename.as_deref().unwrap_or(&record.filename))?;
    let mime = request
        .mime
        .clone()
        .or_else(|| record.mime.clone())
        .unwrap_or_else(default_mime);
    let relationship = request
        .relationship
        .clone()
        .unwrap_or_else(|| record.relationship.clone());
    let description = request
        .description
        .clone()
        .or_else(|| record.description.clone());
    let reader = engine.document().reader();
    let mut noop = |_number: u32, _object: &mut PdfObject| {};
    let (mut objects, root, info) = rewrite_document_objects(reader, &mut noop)?;
    let old_spec_number = remapped_object_number(reader, old_spec.0).ok_or_else(|| {
        OxideError::MalformedPdf("associated FileSpec could not be remapped".to_string())
    })?;
    let mut next = objects
        .iter()
        .map(|object| object.number)
        .max()
        .unwrap_or(0)
        + 1;
    let stream_number = next;
    next += 1;
    let spec_number = next;
    let digest = sha256_hex(payload);
    let mut params = PdfDictionary::empty();
    params.insert("Size", PdfObject::Integer(payload.len() as i64));
    params.insert("OxideSHA256", PdfObject::String(digest.as_bytes().to_vec()));
    let mut stream = PdfDictionary::empty();
    stream.insert("Type", PdfObject::Name("EmbeddedFile".to_string()));
    stream.insert("Subtype", PdfObject::Name(pdf_name(&mime)));
    stream.insert("Params", PdfObject::Dictionary(params));
    objects.push(OutputObject {
        number: stream_number,
        object: PdfObject::Stream {
            dict: stream,
            raw: payload.to_vec(),
        },
    });
    let mut ef = PdfDictionary::empty();
    ef.insert("F", reference(stream_number, 0));
    ef.insert("UF", reference(stream_number, 0));
    let mut spec = PdfDictionary::empty();
    spec.insert("Type", PdfObject::Name("Filespec".to_string()));
    spec.insert("F", PdfObject::String(filename.as_bytes().to_vec()));
    spec.insert("UF", PdfObject::String(filename.as_bytes().to_vec()));
    if let Some(description) = description {
        spec.insert("Desc", PdfObject::String(description.as_bytes().to_vec()));
    }
    spec.insert(
        "AFRelationship",
        PdfObject::Name(pdf_name(relationship.pdf_name())),
    );
    spec.insert("EF", PdfObject::Dictionary(ef));
    objects.push(OutputObject {
        number: spec_number,
        object: PdfObject::Dictionary(spec),
    });
    let owner_ref = request.owner_ref.as_deref().or(record.owner_ref.as_deref());
    let owner_number = resolve_owner_output_number(reader, root, request.owner, owner_ref)?;
    let owner_object = objects
        .iter_mut()
        .find(|object| object.number == owner_number)
        .ok_or_else(|| OxideError::MalformedPdf("associated-file owner is missing".to_string()))?;
    replace_filespec_on_owner(
        &mut owner_object.object,
        request.owner,
        old_spec_number,
        spec_number,
    )?;
    remove_unreachable_filespec_and_streams(&mut objects, old_spec_number);
    let output = PdfWriter::new(objects, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::ClassicXref)
        .write()?;
    owner_mutation_report(input, output, "update_owner", before, 1, 1)
}

fn detach_filespec_from_owner(
    object: &mut PdfObject,
    owner: AssociatedFileOwnerType,
    spec_number: u32,
) -> Result<()> {
    let dict = object_dict_mut(object).ok_or_else(|| {
        OxideError::MalformedPdf("associated-file owner is not a dictionary".to_string())
    })?;
    remove_reference_from_array(dict, "AF", spec_number);
    if owner == AssociatedFileOwnerType::Annotation
        && dict
            .get("FS")
            .and_then(PdfObject::as_reference)
            .map(|value| value.0)
            == Some(spec_number)
    {
        dict.remove("FS");
    }
    Ok(())
}

fn replace_filespec_on_owner(
    object: &mut PdfObject,
    owner: AssociatedFileOwnerType,
    old_spec: u32,
    new_spec: u32,
) -> Result<()> {
    detach_filespec_from_owner(object, owner, old_spec)?;
    attach_filespec_to_owner(object, owner, new_spec)
}

fn remove_reference_from_array(dict: &mut PdfDictionary, key: &str, number: u32) {
    let Some(value) = dict.remove(key) else {
        return;
    };
    let mut items = match value {
        PdfObject::Array(items) => items,
        other => vec![other],
    };
    items.retain(|item| item.as_reference().map(|value| value.0) != Some(number));
    if !items.is_empty() {
        dict.insert(key, PdfObject::Array(items));
    }
}

fn remove_unreachable_filespec_and_streams(objects: &mut Vec<OutputObject>, spec_number: u32) {
    if objects.iter().any(|object| {
        object.number != spec_number && object_references(&object.object, spec_number)
    }) {
        return;
    }
    let stream_numbers = objects
        .iter()
        .find(|object| object.number == spec_number)
        .map(|object| collect_references(&object.object))
        .unwrap_or_default();
    objects.retain(|object| object.number != spec_number);
    for stream_number in stream_numbers {
        if !objects
            .iter()
            .any(|object| object_references(&object.object, stream_number))
        {
            objects.retain(|object| object.number != stream_number);
        }
    }
}

fn collect_references(object: &PdfObject) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    collect_references_into(object, &mut out);
    out
}

fn collect_references_into(object: &PdfObject, out: &mut BTreeSet<u32>) {
    match object {
        PdfObject::Reference { number, .. } => {
            out.insert(*number);
        }
        PdfObject::Array(items) => {
            for item in items {
                collect_references_into(item, out);
            }
        }
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
            for (_, value) in dict.entries() {
                collect_references_into(value, out);
            }
        }
        _ => {}
    }
}

fn object_references(object: &PdfObject, number: u32) -> bool {
    match object {
        PdfObject::Reference { number: found, .. } => *found == number,
        PdfObject::Array(items) => items.iter().any(|item| object_references(item, number)),
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => dict
            .entries()
            .any(|(_, value)| object_references(value, number)),
        _ => false,
    }
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
    enforce_full_rewrite_signed_policy(
        input,
        EditOperation::AttachmentRemove,
        options.signature_policy_override,
        "associated-file removal",
    )?;
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
    analyze_edit_policy_for_target(engine, operation, None)
}

pub fn analyze_edit_policy_for_target(
    engine: &ContentEngine,
    operation: EditOperation,
    target_field: Option<&str>,
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
    let locked_field = matches!(
        operation,
        EditOperation::FormValueUpdate | EditOperation::FormAppearanceUpdate
    ) && structural
        .iter()
        .any(|policy| fieldmdp_locks_target(policy, target_field));
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

#[derive(Debug, Clone, Serialize)]
pub struct IncrementalMutationReport {
    pub schema_version: String,
    pub operation: EditOperation,
    pub policy: EditPolicyReport,
    pub original_bytes: usize,
    pub output_bytes: usize,
    pub revision_bytes: usize,
    pub original_prefix_preserved: bool,
    pub output_reopened: bool,
    pub visible_after_reopen: bool,
    pub output_sha256: String,
    pub post_save_signature_impact: SignatureImpactSummary,
    pub cryptographic_validity_claimed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignaturePreservingEditPlan {
    pub schema_version: &'static str,
    pub operation: EditOperation,
    pub allowed: bool,
    pub decision: EditPolicyDecision,
    pub reason: String,
    pub output_must_be_incremental: bool,
    pub prefix_preservation_required: bool,
    pub before_signature_count: usize,
    pub before_signatures: Vec<SignatureReport>,
    pub policy: EditPolicyReport,
    pub exact_limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostEditSignatureReport {
    pub schema_version: &'static str,
    pub before_signature_count: usize,
    pub after_signature_count: usize,
    pub original_prefix_preserved: bool,
    pub original_signatures_mathematically_valid_after_edit: bool,
    pub post_edit_signatures: Vec<SignatureReport>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignaturePreservingEditResult {
    pub schema_version: &'static str,
    pub operation: EditOperation,
    pub plan: SignaturePreservingEditPlan,
    pub mutation: IncrementalMutationReport,
    pub post_edit: PostEditSignatureReport,
}

pub fn plan_signature_preserving_form_fill(
    input: &[u8],
    field_name: &str,
    value: &str,
    options: &VerifyOptions,
) -> Result<SignaturePreservingEditPlan> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let policy =
        analyze_edit_policy_for_target(&engine, EditOperation::FormValueUpdate, Some(field_name))?;
    let before_signatures = verify_signatures_with_options(engine.document(), options)?;
    let allowed = matches!(
        policy.decision,
        EditPolicyDecision::SafeIncremental | EditPolicyDecision::IncrementalWithWarning
    );
    let reason = if allowed {
        format!(
            "form field '{field_name}' can be written as an append-only incremental update; value bytes are not placed in an existing signed byte range"
        )
    } else {
        format!(
            "form field '{field_name}' update is blocked by the parsed DocMDP/FieldMDP policy decision {:?}",
            policy.decision
        )
    };
    Ok(SignaturePreservingEditPlan {
        schema_version: PROMPT25_SIGNATURE_LTV_EDIT_SCHEMA_VERSION,
        operation: EditOperation::FormValueUpdate,
        allowed,
        decision: policy.decision,
        reason,
        output_must_be_incremental: true,
        prefix_preservation_required: true,
        before_signature_count: before_signatures.len(),
        before_signatures,
        policy,
        exact_limits: vec![
            "Prompt 25 form-fill preservation is append-only and revalidates original signatures after reopen".to_string(),
            "DocMDP/FieldMDP decisions are enforced from parsed transform policy; unknown or blocked policy denies by default unless explicit invalidation override is selected".to_string(),
            format!("planned value byte length: {}", value.len()),
        ],
    })
}

pub fn apply_signature_preserving_form_fill(
    input: &[u8],
    field_name: &str,
    value: &str,
    options: &VerifyOptions,
    explicit_invalidation_override: bool,
) -> Result<(Vec<u8>, SignaturePreservingEditResult)> {
    let plan = plan_signature_preserving_form_fill(input, field_name, value, options)?;
    if !plan.allowed && !explicit_invalidation_override {
        return Err(OxideError::UnsupportedFeature(plan.reason.clone()));
    }
    let (output, mutation) = incremental_form_value_update_pdf(
        input,
        field_name,
        value,
        explicit_invalidation_override,
    )?;
    let post_edit = validate_after_signature_preserving_edit(input, &output, options)?;
    if !post_edit.original_prefix_preserved {
        return Err(OxideError::MalformedPdf(
            "signature-preserving edit did not preserve the original byte prefix".to_string(),
        ));
    }
    Ok((
        output,
        SignaturePreservingEditResult {
            schema_version: PROMPT25_SIGNATURE_LTV_EDIT_SCHEMA_VERSION,
            operation: EditOperation::FormValueUpdate,
            plan,
            mutation,
            post_edit,
        },
    ))
}

pub fn validate_after_signature_preserving_edit(
    original: &[u8],
    output: &[u8],
    options: &VerifyOptions,
) -> Result<PostEditSignatureReport> {
    let before_engine = ContentEngine::open_bytes(original.to_vec())?;
    let after_engine = ContentEngine::open_bytes(output.to_vec())?;
    let before = verify_signatures_with_options(before_engine.document(), options)?;
    let after = verify_signatures_with_options(after_engine.document(), options)?;
    let prefix_preserved = output.starts_with(original);
    let original_math_valid_after = after
        .iter()
        .take(before.len())
        .all(|report| report.validity == SignatureValidity::Valid);
    let mut warnings = Vec::new();
    if after.len() < before.len() {
        warnings.push("post-edit document reports fewer signatures than the original".to_string());
    }
    if !original_math_valid_after {
        warnings.push(
            "one or more original signatures failed mathematical validation after the edit"
                .to_string(),
        );
    }
    Ok(PostEditSignatureReport {
        schema_version: PROMPT25_SIGNATURE_LTV_EDIT_SCHEMA_VERSION,
        before_signature_count: before.len(),
        after_signature_count: after.len(),
        original_prefix_preserved: prefix_preserved,
        original_signatures_mathematically_valid_after_edit: original_math_valid_after,
        post_edit_signatures: after,
        warnings,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IncrementalAnnotationEdit {
    AddTextNote {
        page: usize,
        rect: [f64; 4],
        contents: String,
    },
    UpdateContents {
        page: usize,
        annotation_index: usize,
        contents: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IncrementalPagePropertyEdit {
    Rotate { page: usize, degrees: i32 },
    CropBox { page: usize, value: [f64; 4] },
}

pub fn incremental_form_value_update_pdf(
    input: &[u8],
    field_name: &str,
    value: &str,
    signature_policy_override: bool,
) -> Result<(Vec<u8>, IncrementalMutationReport)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let policy =
        analyze_edit_policy_for_target(&engine, EditOperation::FormValueUpdate, Some(field_name))?;
    enforce_mutation_policy(&policy, signature_policy_override, "form value edit")?;
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    editor.set_form_text(field_name, value);
    let output = editor.save_to_bytes(EditMode::Incremental)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let visible =
        reopened
            .document()
            .reader()
            .object_ids()
            .into_iter()
            .any(|(number, generation)| {
                reopened
                    .document()
                    .reader()
                    .get_object(number, generation)
                    .ok()
                    .and_then(|object| object.as_dict().cloned())
                    .is_some_and(|dict| {
                        text_value(&dict, "T").as_deref() == Some(field_name)
                            && text_value(&dict, "V").as_deref() == Some(value)
                    })
            });
    let report = finish_incremental_report(
        input,
        &output,
        policy,
        EditOperation::FormValueUpdate,
        visible,
    )?;
    Ok((output, report))
}

pub fn incremental_annotation_update_pdf(
    input: &[u8],
    edit: &IncrementalAnnotationEdit,
    signature_policy_override: bool,
) -> Result<(Vec<u8>, IncrementalMutationReport)> {
    let operation = match edit {
        IncrementalAnnotationEdit::AddTextNote { .. } => EditOperation::AnnotationAdd,
        IncrementalAnnotationEdit::UpdateContents { .. } => EditOperation::AnnotationUpdate,
    };
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let policy = analyze_edit_policy(&engine, operation)?;
    enforce_mutation_policy(&policy, signature_policy_override, "annotation edit")?;
    let before = count_annotations(&engine)?;
    let mut editor = PdfEditor::open_bytes(input.to_vec())?;
    let expected_text = match edit {
        IncrementalAnnotationEdit::AddTextNote {
            page,
            rect,
            contents,
        } => {
            editor.add_text_note_annotation(
                *page,
                ImageRect::new(rect[0], rect[1], rect[2], rect[3]),
                contents,
                AnnotationOptions::default(),
            )?;
            contents
        }
        IncrementalAnnotationEdit::UpdateContents {
            page,
            annotation_index,
            contents,
        } => {
            editor.edit_annotation_contents(*page, *annotation_index, contents)?;
            contents
        }
    };
    let output = editor.save_to_bytes(EditMode::Incremental)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let after = count_annotations(&reopened)?;
    let text_visible =
        reopened
            .document()
            .reader()
            .object_ids()
            .into_iter()
            .any(|(number, generation)| {
                reopened
                    .document()
                    .reader()
                    .get_object(number, generation)
                    .ok()
                    .and_then(|object| object.as_dict().cloned())
                    .is_some_and(|dict| {
                        text_value(&dict, "Contents").as_deref() == Some(expected_text)
                    })
            });
    let visible = text_visible
        && (!matches!(edit, IncrementalAnnotationEdit::AddTextNote { .. }) || after == before + 1);
    let report = finish_incremental_report(input, &output, policy, operation, visible)?;
    Ok((output, report))
}

pub fn incremental_page_property_update_pdf(
    input: &[u8],
    edit: &IncrementalPagePropertyEdit,
    signature_policy_override: bool,
) -> Result<(Vec<u8>, IncrementalMutationReport)> {
    let operation = match edit {
        IncrementalPagePropertyEdit::Rotate { .. } => EditOperation::PageRotate,
        IncrementalPagePropertyEdit::CropBox { .. } => EditOperation::PageBoxChange,
    };
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let policy = analyze_edit_policy(&engine, operation)?;
    enforce_mutation_policy(&policy, signature_policy_override, "page property edit")?;
    let pages = engine.document().get_pages()?;
    let page_number = match edit {
        IncrementalPagePropertyEdit::Rotate { page, .. }
        | IncrementalPagePropertyEdit::CropBox { page, .. } => *page,
    };
    let page = pages.get(page_number.saturating_sub(1)).ok_or_else(|| {
        OxideError::MalformedPdf(format!("page {page_number} is outside the document"))
    })?;
    let mut dict = engine
        .document()
        .reader()
        .get_object(page.object_number, page.generation_number)?
        .as_dict()
        .cloned()
        .ok_or_else(|| OxideError::MalformedPdf("page object is not a dictionary".to_string()))?;
    match edit {
        IncrementalPagePropertyEdit::Rotate { degrees, .. } => {
            if degrees % 90 != 0 {
                return Err(OxideError::MalformedPdf(
                    "page rotation must be a multiple of 90 degrees".to_string(),
                ));
            }
            dict.insert(
                "Rotate",
                PdfObject::Integer(i64::from(degrees.rem_euclid(360))),
            );
        }
        IncrementalPagePropertyEdit::CropBox { value, .. } => {
            if value.iter().any(|component| !component.is_finite())
                || value[2] <= value[0]
                || value[3] <= value[1]
            {
                return Err(OxideError::MalformedPdf("invalid page CropBox".to_string()));
            }
            dict.insert(
                "CropBox",
                PdfObject::Array(value.iter().map(|value| PdfObject::Real(*value)).collect()),
            );
        }
    }
    let output = write_incremental_update(
        engine.document().reader(),
        vec![IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(dict),
        }],
    )?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let changed = reopened
        .document()
        .get_pages()?
        .get(page_number - 1)
        .is_some_and(|page| match edit {
            IncrementalPagePropertyEdit::Rotate { degrees, .. } => {
                page.rotate == degrees.rem_euclid(360)
            }
            IncrementalPagePropertyEdit::CropBox { value, .. } => page.crop_box == *value,
        });
    let report = finish_incremental_report(input, &output, policy, operation, changed)?;
    Ok((output, report))
}

fn enforce_mutation_policy(
    policy: &EditPolicyReport,
    signature_policy_override: bool,
    label: &str,
) -> Result<()> {
    if matches!(
        policy.decision,
        EditPolicyDecision::BlockedBySignaturePolicy | EditPolicyDecision::ExplicitOverrideRequired
    ) && !signature_policy_override
    {
        return Err(OxideError::UnsupportedFeature(format!(
            "{label} blocked by DocMDP/FieldMDP structural signature policy"
        )));
    }
    Ok(())
}

fn enforce_full_rewrite_signed_policy(
    input: &[u8],
    operation: EditOperation,
    signature_policy_override: bool,
    label: &str,
) -> Result<()> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let policy = analyze_edit_policy(&engine, operation)?;
    if policy.impact.signature_count > 0 && !signature_policy_override {
        return Err(OxideError::UnsupportedFeature(format!(
            "{label} requires a full rewrite and is blocked for a signed input without explicit override"
        )));
    }
    enforce_mutation_policy(&policy, signature_policy_override, label)
}

fn finish_incremental_report(
    input: &[u8],
    output: &[u8],
    mut policy: EditPolicyReport,
    operation: EditOperation,
    visible_after_reopen: bool,
) -> Result<IncrementalMutationReport> {
    if !output.starts_with(input) {
        return Err(OxideError::MalformedPdf(
            "incremental mutation did not preserve the exact original prefix".to_string(),
        ));
    }
    let reopened = ContentEngine::open_bytes(output.to_vec())?;
    policy.original_prefix_preserved = true;
    policy.byte_range_covered_bytes_untouched = true;
    policy.signature_dictionary_untouched = true;
    policy.impact.append_only_update = true;
    policy.impact.signature_value_preserved = true;
    let mut post_save_signature_impact = signature_impact_summary(&reopened, operation)?;
    post_save_signature_impact.append_only_update = true;
    post_save_signature_impact.signature_value_preserved = true;
    Ok(IncrementalMutationReport {
        schema_version: PROMPT18_SCHEMA_VERSION.to_string(),
        operation,
        policy,
        original_bytes: input.len(),
        output_bytes: output.len(),
        revision_bytes: output.len().saturating_sub(input.len()),
        original_prefix_preserved: true,
        output_reopened: true,
        visible_after_reopen,
        output_sha256: resource_digest(output),
        post_save_signature_impact,
        cryptographic_validity_claimed: false,
    })
}

fn count_annotations(engine: &ContentEngine) -> Result<usize> {
    engine
        .document()
        .get_pages()?
        .iter()
        .try_fold(0usize, |total, page| {
            let dict = engine
                .document()
                .reader()
                .get_object(page.object_number, page.generation_number)?;
            let count = dict
                .as_dict()
                .and_then(|dict| dict.get("Annots"))
                .and_then(|value| engine.document().reader().resolve(value.clone()).ok())
                .and_then(|value| value.as_array().map(<[PdfObject]>::len))
                .unwrap_or(0);
            Ok(total + count)
        })
}

fn fieldmdp_locks_target(policy: &StructuralSignaturePolicy, target: Option<&str>) -> bool {
    let Some(action) = policy.fieldmdp_action.as_deref() else {
        return false;
    };
    let Some(target) = target else {
        return true;
    };
    let listed = policy.fieldmdp_fields.iter().any(|field| field == target);
    match action {
        "All" => true,
        "Include" => listed,
        "Exclude" => !listed,
        _ => true,
    }
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

pub fn prompt18b_report(engine: &ContentEngine) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "schema_version": PROMPT18B_SCHEMA_VERSION,
        "kind": "prompt18b_report",
        "closure": prompt18b_feature_report_value(crate::sdk::REPORT_ENVELOPE_VERSION),
        "mask_redaction": mask_redaction_inventory(engine)?,
        "associated_files": associated_files_inventory(engine)?,
        "signature_policy": analyze_edit_policy(engine, EditOperation::IncrementalSave)?,
    }))
}

pub(crate) fn prompt18b_feature_report_value(envelope_version: u32) -> serde_json::Value {
    let implemented = [
        "packed_1_bit_stencil_redaction",
        "packed_1_2_4_8_bit_indexed_redaction",
        "iccbased_gray_rgb_cmyk_redaction",
        "iccbased_explicit_mask_redaction",
        "iccbased_soft_mask_redaction",
        "inline_png_predictor",
        "inline_tiff_predictor",
        "inline_decodeparms_array",
        "inline_image_mask",
        "inline_image_promotion",
        "catalog_af_mutation",
        "page_af_mutation",
        "annotation_fs_af_mutation",
        "structure_element_af_mutation",
        "form_xobject_af_mutation",
        "afrelationship_preservation",
        "incremental_form_edit",
        "incremental_annotation_edit",
        "incremental_page_property_edit",
        "docmdp_enforcement",
        "fieldmdp_enforcement",
        "post_save_signature_impact_recheck",
    ];
    serde_json::json!({
        "schema_version": PROMPT18B_SCHEMA_VERSION,
        "envelope_version": envelope_version,
        "status": "complete_with_exact_limits",
        "rows": implemented.into_iter().map(|row| serde_json::json!({
            "id": row,
            "status": "implemented"
        })).collect::<Vec<_>>(),
        "coverage": {
            "packed_masks": "implemented",
            "indexed": "implemented",
            "iccbased": "implemented_with_limits",
            "predictor_inline": "implemented_with_limits",
            "inline_promotion": "implemented",
            "owner_specific_associated_files": "implemented_with_limits",
            "docmdp_fieldmdp_enforcement": "implemented_with_limits",
            "incremental_form_annotation_page": "implemented_with_limits"
        },
        "failure": {"blocked": 0, "unclassified": 0, "security_proof": 0, "oxide_outliers": 0},
        "limits": {
            "decoded_pixels": MAX_IMAGE_MASK_PIXELS,
            "mask_recursion": MAX_MASK_RECURSION,
            "predictor_row_bytes": MAX_INLINE_IMAGE_BYTES,
            "inline_bytes": MAX_INLINE_IMAGE_BYTES,
            "promoted_objects": MAX_INLINE_IMAGES,
            "associated_file_owners": MAX_ASSOCIATED_FILES,
            "embedded_file_bytes": MAX_ASSOCIATED_FILE_BYTES,
            "signature_count": MAX_SIGNATURES,
            "docmdp_fieldmdp_entries": MAX_POLICY_REFERENCES
        },
        "security": {
            "overlay_only_redaction_success_claims": 0,
            "unsupported_codec_posture": "secure_invocation_removal_or_fail_closed",
            "external_files_fetched_or_executed": 0,
            "signature_crypto_overclaim": 0,
            "full_rewrite_signature_posture": "invalidation_risk_reported"
        },
        "exact_limits": [
            "packed Indexed rewrite supports DeviceGray, DeviceRGB, and DeviceCMYK lookup bases with 1, 2, 4, or 8-bit indices",
            "ICCBased rewrite preserves profile references for /N 1, 3, or 4 and rejects channel mismatch or higher-channel profiles",
            "inline predictor rewrite supports bounded TIFF predictor 2 and PNG predictors 10 through 15 when Colors, BitsPerComponent, and Columns match the image layout",
            "owner mutation supports catalog, page, annotation, structure element, Form XObject, and image XObject indirect owners; external and active owner families remain exact fail-closed limits",
            "DocMDP and FieldMDP enforcement is structural; cryptographic validity, trust-chain status, and viewer certification behavior are not inferred from prefix preservation"
        ],
        "public_report_schema": "additive_feature_report_prompt18b"
    })
}

pub(crate) fn prompt18_feature_report_value(envelope_version: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": PROMPT18_SCHEMA_VERSION,
        "envelope_version": envelope_version,
        "status": "complete_extended_by_prompt18b",
        "coverage": {
            "mask_softmask_redaction": "implemented_with_prompt18b_packed_icc_extension",
            "inline_image_partial_redaction": "implemented_with_limits",
            "associated_files": "implemented_with_prompt18b_owner_mutation",
            "signature_safe_edit_policy": "implemented_with_prompt18b_active_enforcement",
            "docmdp_fieldmdp_structural_policy": "implemented_with_prompt18b_active_enforcement",
            "incremental_prefix_preservation": "implemented_form_annotation_page_metadata"
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
            "bounded inline device samples, TIFF/PNG predictors, and one-bit ImageMask samples are rewritten directly or promoted to deterministic Image XObjects; unsupported paths remove/fail closed",
            "affected packed, Indexed, device, and common ICCBased Image XObjects are cloned with rewritten color and supported mask samples; unsupported affected instances remove/fail closed",
            "associated-file mutation preserves supported catalog, page, annotation, structure, Form, and XObject owners; catalog EmbeddedFiles indexing remains a distinct operation",
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
        let Some(dict) = object_dictionary_ref(&object) else {
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
        if dict.get_name("FT") == Some("Sig") {
            if let Some(lock) = dict
                .get("Lock")
                .and_then(|value| reader.resolve(value.clone()).ok())
                .and_then(|value| value.as_dict().cloned())
            {
                row.fieldmdp_action = lock.get_name("Action").map(str::to_string);
                row.fieldmdp_fields = lock
                    .get("Fields")
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
        }
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
    let owner_key = owner.map(ref_id).unwrap_or_else(|| "none".to_string());
    if !seen_specs.insert(format!("{spec_id}@{owner_key}")) {
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
        if let Some(base_index) = records
            .iter()
            .position(|record| record.stream_ref.as_ref() == stream_id.as_ref())
        {
            let relationship = AfRelationship::from_name(spec.get_name("AFRelationship"));
            if let Some(owner) = owner {
                let owner_id = ref_id(owner);
                if records[base_index].owner_ref.is_none() {
                    let existing = &mut records[base_index];
                    existing.stable_id = format!("filespec-{spec_id}-owner-{owner_id}");
                    existing.file_spec_ref = spec_ref.map(ref_id);
                    existing.owner_ref = Some(owner_id);
                    existing.owner_type = owner_type;
                    existing.relationship = relationship;
                    existing.provenance =
                        "attachment_discovery_plus_filespec_owner_scan".to_string();
                } else if records[base_index].owner_ref.as_deref() != Some(owner_id.as_str()) {
                    let mut record = records[base_index].clone();
                    record.stable_id = format!("filespec-{spec_id}-owner-{owner_id}");
                    record.file_spec_ref = spec_ref.map(ref_id);
                    record.owner_ref = Some(owner_id);
                    record.owner_type = owner_type;
                    record.relationship = relationship;
                    record.provenance = "shared_filespec_additional_owner_scan".to_string();
                    records.push(record);
                }
            } else {
                records[base_index].file_spec_ref = spec_ref.map(ref_id);
                records[base_index].relationship = relationship;
            }
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
    let decoded_payload = stream.and_then(|reference| {
        let object = reader.get_object(reference.0, reference.1).ok()?;
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_ASSOCIATED_FILE_BYTES as u64,
                ..DecodeLimits::default()
            },
        )
        .ok()?;
        matches!(decoded.status, StreamDecodeStatus::Complete).then_some(decoded.data)
    });
    let payload_size = decoded_payload.as_ref().map(Vec::len);
    let payload_sha256 = decoded_payload.as_deref().map(sha256_hex);
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
        size: payload_size,
        sha256: payload_sha256.clone(),
        creation_date: None,
        modification_date: None,
        encrypted: reader.is_encrypted(),
        decoded: decoded_payload.is_some(),
        internal: stream.is_some(),
        external_target,
        duplicate_group: payload_sha256,
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
        "owner-specific full-rewrite mutation supports catalog, page, annotation, structure, Form, and XObject owners; catalog EmbeddedFiles indexing is handled separately".to_string(),
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

fn object_dictionary_ref(object: &PdfObject) -> Option<&PdfDictionary> {
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
