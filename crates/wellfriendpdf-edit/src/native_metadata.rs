//! Website metadata cleanup through the existing bounded parser and writer.
//! This bulk operation does not alter selector-bound editor XMP restrictions.
use crate::{
    PdfEditError, SignatureEffect, checked_metadata, is_writer_housekeeping_object,
    load_checked_pdf, pdf_objects_semantically_equal, save_new_pdf, sha256_file,
    signature_summaries, validate_paths,
};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub struct MetadataPlan {
    pub fields: BTreeMap<Vec<u8>, Option<Object>>,
    pub xmp: Option<Vec<u8>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataResult {
    pub input_sha256: String,
    pub output_sha256: String,
    pub output_bytes: u64,
    pub page_count: usize,
    pub signature_effect: SignatureEffect,
    pub non_metadata_objects_preserved: bool,
}

pub fn edit_website_metadata(
    input: &Path,
    output: &Path,
    plan: MetadataPlan,
    allow_signature_invalidation: bool,
) -> Result<MetadataResult, PdfEditError> {
    validate_paths(input, output)?;
    let PreparedMetadata {
        mut document,
        source,
        changed,
        page_count,
        signature_effect,
        input_sha256,
    } = prepare_website_metadata(input, &plan, allow_signature_invalidation)?;
    save_new_pdf(&mut document, output)?;
    let verified = load_checked_pdf(output)?;
    let preserved = source.iter().all(|(id, obj)| {
        changed.contains(id)
            || is_writer_housekeeping_object(obj)
            || verified
                .objects
                .get(id)
                .is_some_and(|after| pdf_objects_semantically_equal(obj, after))
    });
    let (_, info) = verified
        .dereference(
            verified
                .trailer
                .get(b"Info")
                .map_err(|_| PdfEditError::ValidationFailed)?,
        )
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let info = info.as_dict().map_err(|_| PdfEditError::ValidationFailed)?;
    let fields_verified = plan.fields.iter().all(|(key, value)| match value {
        Some(value) => info.get(key).is_ok_and(|after| after == value),
        None => info.get(key).is_err(),
    });
    if !preserved || !fields_verified || verified.get_pages().len() != page_count {
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(MetadataResult {
        input_sha256,
        output_sha256: sha256_file(output)?,
        output_bytes: fs::metadata(output)
            .map_err(|_| PdfEditError::OutputWriteFailed)?
            .len(),
        page_count,
        signature_effect,
        non_metadata_objects_preserved: preserved,
    })
}

/// Verify retained bytes against the original source and trusted metadata plan.
/// This reads both files and builds the expected objects in memory; it never
/// saves a PDF or grants job, source, publication, or settlement authority.
pub fn verify_website_metadata(
    input: &Path,
    candidate: &Path,
    plan: MetadataPlan,
    allow_signature_invalidation: bool,
) -> Result<MetadataResult, PdfEditError> {
    let input_node = checked_metadata(input)?;
    let candidate_node = checked_metadata(candidate)?;
    if input
        .canonicalize()
        .map_err(|_| PdfEditError::InputUnavailable)?
        == candidate
            .canonicalize()
            .map_err(|_| PdfEditError::InputUnavailable)?
    {
        return Err(PdfEditError::ValidationFailed);
    }
    let output_sha256 = sha256_file(candidate)?;
    let PreparedMetadata {
        document: expected,
        page_count,
        signature_effect,
        input_sha256,
        ..
    } = prepare_website_metadata(input, &plan, allow_signature_invalidation)?;
    let actual = load_checked_pdf(candidate)?;
    // The existing writer drops source packing/linearization objects and may
    // generate an XRef stream. Every semantic object, including modified Info,
    // catalog and XMP objects, must otherwise match in both directions.
    let preserved = expected.objects.iter().all(|(id, value)| {
        writer_only_object(value)
            || actual
                .objects
                .get(id)
                .is_some_and(|after| pdf_objects_semantically_equal(value, after))
    }) && actual.objects.iter().all(|(id, value)| {
        expected.objects.contains_key(id) || value.type_name().is_ok_and(|kind| kind == b"XRef")
    });
    if !preserved
        || expected.version != actual.version
        || actual.get_pages().len() != page_count
        || semantic_trailer(&expected.trailer) != semantic_trailer(&actual.trailer)
        || !unchanged_file(input, &input_node, &input_sha256)?
        || !unchanged_file(candidate, &candidate_node, &output_sha256)?
    {
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(MetadataResult {
        input_sha256,
        output_sha256,
        output_bytes: candidate_node.len(),
        page_count,
        signature_effect,
        non_metadata_objects_preserved: true,
    })
}

fn writer_only_object(value: &Object) -> bool {
    is_writer_housekeeping_object(value)
        || value.type_name().is_ok_and(|kind| kind == b"Linearized")
}

fn semantic_trailer(trailer: &Dictionary) -> Dictionary {
    let mut semantic = trailer.clone();
    semantic.remove(b"Size");
    // These are regenerated by lopdf 0.44's XRef-stream writer. Do not ignore
    // arbitrary trailer fields (especially Root, Info, ID, Encrypt or Prev).
    if trailer
        .get(b"Type")
        .and_then(Object::as_name)
        .is_ok_and(|kind| kind == b"XRef")
    {
        for key in [b"Type".as_slice(), b"W", b"Index", b"Length", b"Filter"] {
            semantic.remove(key);
        }
    }
    semantic
}

fn unchanged_file(path: &Path, before: &fs::Metadata, digest: &str) -> Result<bool, PdfEditError> {
    let after = checked_metadata(path)?;
    Ok(before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && sha256_file(path)? == digest)
}

struct PreparedMetadata {
    document: Document,
    source: BTreeMap<ObjectId, Object>,
    changed: BTreeSet<ObjectId>,
    page_count: usize,
    signature_effect: SignatureEffect,
    input_sha256: String,
}

fn prepare_website_metadata(
    input: &Path,
    plan: &MetadataPlan,
    allow_signature_invalidation: bool,
) -> Result<PreparedMetadata, PdfEditError> {
    checked_metadata(input)?;
    if plan.fields.keys().any(|key| {
        !matches!(
            key.as_slice(),
            b"Title"
                | b"Author"
                | b"Subject"
                | b"Keywords"
                | b"Creator"
                | b"Producer"
                | b"CreationDate"
                | b"ModDate"
        )
    }) || plan
        .xmp
        .as_ref()
        .is_some_and(|bytes| bytes.len() > 4 * 1024 * 1024)
    {
        return Err(PdfEditError::MetadataUnsupported);
    }
    let input_sha256 = sha256_file(input)?;
    let mut document = load_checked_pdf(input)?;
    let page_count = document.get_pages().len();
    let source = document.objects.clone();
    let signature_effect = if signature_summaries(&document).is_empty() {
        SignatureEffect::None
    } else if allow_signature_invalidation {
        SignatureEffect::InvalidatesExistingSignatures
    } else {
        return Err(PdfEditError::SignatureConfirmationRequired);
    };
    let mut changed = BTreeSet::<ObjectId>::new();
    let (info_id, mut info) = match document.trailer.get(b"Info") {
        Ok(value) => {
            let (id, obj) = document
                .dereference(value)
                .map_err(|_| PdfEditError::MetadataUnsupported)?;
            (
                id,
                obj.as_dict()
                    .cloned()
                    .map_err(|_| PdfEditError::MetadataUnsupported)?,
            )
        }
        Err(_) => (None, Dictionary::new()),
    };
    for (key, value) in &plan.fields {
        if let Some(value) = value {
            info.set(key.clone(), value.clone());
        } else {
            info.remove(key);
        }
    }
    let info_id = info_id.unwrap_or_else(|| document.new_object_id());
    document.objects.insert(info_id, Object::Dictionary(info));
    document.trailer.set("Info", info_id);
    changed.insert(info_id);
    if let Some(bytes) = plan.xmp.as_ref() {
        let catalog_id = document
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|_| PdfEditError::MetadataUnsupported)?;
        let metadata_id = document
            .get_dictionary(catalog_id)
            .ok()
            .and_then(|catalog| catalog.get(b"Metadata").ok())
            .and_then(|value| document.dereference(value).ok())
            .and_then(|(id, obj)| obj.as_stream().ok().and(id))
            .unwrap_or_else(|| document.new_object_id());
        document.objects.insert(
            metadata_id,
            Object::Stream(Stream::new(
                lopdf::dictionary! {"Type"=>"Metadata","Subtype"=>"XML"},
                bytes.clone(),
            )),
        );
        document
            .get_dictionary_mut(catalog_id)
            .map_err(|_| PdfEditError::MetadataUnsupported)?
            .set("Metadata", metadata_id);
        changed.insert(catalog_id);
        changed.insert(metadata_id);
    }
    Ok(PreparedMetadata {
        document,
        source,
        changed,
        page_count,
        signature_effect,
        input_sha256,
    })
}
