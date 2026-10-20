//! Bulk page projection through the existing native parser and PDF writer.
use crate::{
    PdfEditError, SignatureEffect, checked_metadata, is_writer_housekeeping_object,
    load_checked_pdf, pdf_objects_semantically_equal, save_new_pdf, sha256_file,
    signature_summaries, validate_paths,
};
use lopdf::{Dictionary, Document, LoadOptions, Object, ObjectId};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrganizeResult {
    pub input_sha256: String,
    pub output_sha256: String,
    pub output_bytes: u64,
    pub page_count: usize,
    pub output_page_count: usize,
    pub signature_effect: SignatureEffect,
    pub page_content_preserved: bool,
    pub retained_objects_preserved: bool,
}

pub(crate) fn inherited(
    document: &Document,
    id: ObjectId,
    key: &[u8],
) -> Result<Option<Object>, PdfEditError> {
    let mut id = id;
    let mut seen = BTreeSet::new();
    for _ in 0..128 {
        if !seen.insert(id) {
            return Err(PdfEditError::PageTreeUnsupported);
        }
        let dict = document
            .get_dictionary(id)
            .map_err(|_| PdfEditError::PageTreeUnsupported)?;
        if let Ok(value) = dict.get(key) {
            return Ok(Some(value.clone()));
        }
        match dict.get(b"Parent") {
            Ok(value) => {
                id = value
                    .as_reference()
                    .map_err(|_| PdfEditError::PageTreeUnsupported)?
            }
            Err(_) => return Ok(None),
        }
    }
    Err(PdfEditError::PageTreeUnsupported)
}

fn copied_info(document: &Document) -> Result<Dictionary, PdfEditError> {
    let mut info = Dictionary::new();
    let Ok(value) = document.trailer.get(b"Info") else {
        return Ok(info);
    };
    let (_, value) = document
        .dereference(value)
        .map_err(|_| PdfEditError::MetadataUnsupported)?;
    let dict = value
        .as_dict()
        .map_err(|_| PdfEditError::MetadataUnsupported)?;
    for (key, value) in dict.iter() {
        let (_, value) = document
            .dereference(value)
            .map_err(|_| PdfEditError::MetadataUnsupported)?;
        // The original bulk operation copies docinfo through str(value).
        let value = match value {
            Object::String(_, _) => value.clone(),
            Object::Name(name) => {
                Object::string_literal(format!("/{}", String::from_utf8_lossy(name)))
            }
            Object::Integer(value) => Object::string_literal(value.to_string()),
            Object::Boolean(value) => Object::string_literal(if *value { "True" } else { "False" }),
            Object::Null => Object::string_literal("None"),
            _ => return Err(PdfEditError::MetadataUnsupported),
        };
        info.set(key.clone(), value);
    }
    Ok(info)
}

/// The website's page-copy contract creates a fresh catalog containing Pages;
/// document-wide outlines, destinations, XMP and AcroForm are not transferred.
/// The native editor's selector-bound reorder/delete APIs remain unchanged.
pub fn organize_pages(
    input: &Path,
    output: &Path,
    order: &[u32],
    allow_signature_invalidation: bool,
) -> Result<OrganizeResult, PdfEditError> {
    validate_paths(input, output)?;
    checked_metadata(input)?;
    let input_sha256 = sha256_file(input)?;
    let source = load_checked_pdf(input)?;
    project_pages(
        &source,
        output,
        order,
        &input_sha256,
        false,
        allow_signature_invalidation,
    )
}

/// Copy pages into a fresh catalog, including repeated selections used by split
/// ranges. The caller owns an absent output in its private attempt directory.
/// The source hash is the caller's already authenticated immutable snapshot.
pub fn project_pages(
    source: &Document,
    output: &Path,
    order: &[u32],
    input_sha256: &str,
    allow_repeated_pages: bool,
    allow_signature_invalidation: bool,
) -> Result<OrganizeResult, PdfEditError> {
    project_pages_with_header(
        source,
        output,
        order,
        input_sha256,
        allow_repeated_pages,
        allow_signature_invalidation,
        false,
        false,
    )
}

/// PdfWriter.add_page promotes its default 1.3 header to the source's known
/// higher header. Only the Premium OCR temporary-input caller selects this;
/// existing website/editor page projection retains its original fixed header.
pub fn project_pages_preserving_header(
    source: &Document,
    output: &Path,
    order: &[u32],
    input_sha256: &str,
    allow_repeated_pages: bool,
    allow_signature_invalidation: bool,
) -> Result<OrganizeResult, PdfEditError> {
    project_pages_with_header(
        source,
        output,
        order,
        input_sha256,
        allow_repeated_pages,
        allow_signature_invalidation,
        true,
        false,
    )
}

/// Recover only the identical bounded source projection at a fixed original
/// path. Existing targets and partial pending bytes are never replaced/deleted.
/// This does not authorize a provider call, queue adoption or new deadline.
pub fn recover_pages_preserving_header(
    source: &Document, output: &Path, order: &[u32], input_sha256: &str,
    allow_repeated_pages: bool, allow_signature_invalidation: bool,
) -> Result<OrganizeResult, PdfEditError> {
    project_pages_with_header(source, output, order, input_sha256,
        allow_repeated_pages, allow_signature_invalidation, true, true)
}

fn project_pages_with_header(
    source: &Document,
    output: &Path,
    order: &[u32],
    input_sha256: &str,
    allow_repeated_pages: bool,
    allow_signature_invalidation: bool,
    preserve_header: bool,
    recover_original: bool,
) -> Result<OrganizeResult, PdfEditError> {
    if !recover_original && (output.exists() || fs::symlink_metadata(output).is_ok()) {
        return Err(PdfEditError::OutputAlreadyExists);
    }
    if source.is_encrypted() {
        return Err(PdfEditError::EncryptedPdf);
    }
    let pages = source.get_pages();
    if pages.is_empty() || order.is_empty() {
        return Err(PdfEditError::PageNotFound);
    }
    if pages.len() > 2000 || order.len() > 1_000_000 {
        return Err(PdfEditError::PageLimitExceeded);
    }
    let mut unique = BTreeSet::new();
    if order
        .iter()
        .any(|page| !pages.contains_key(page) || !allow_repeated_pages && !unique.insert(*page))
    {
        return Err(PdfEditError::PageNotFound);
    }
    let signature_effect = if signature_summaries(source).is_empty() {
        SignatureEffect::None
    } else if allow_signature_invalidation {
        SignatureEffect::InvalidatesExistingSignatures
    } else {
        return Err(PdfEditError::SignatureConfirmationRequired);
    };
    let header = if preserve_header {
        match source.version.as_str() {
            "1.4" | "1.5" | "1.6" | "1.7" | "2.0" => source.version.as_str(),
            _ => "1.3",
        }
    } else {
        "1.3"
    };
    let mut document = Document::with_version(header);
    document.objects = source.objects.clone();
    document.max_id = source.max_id;
    // Baking inherited values prevents a reordered page from acquiring another
    // page tree's geometry/resources. Parent edges cannot retain the old tree.
    let mut projected = BTreeMap::new();
    for &id in pages.values() {
        let mut page = source
            .get_dictionary(id)
            .map_err(|_| PdfEditError::PageTreeUnsupported)?
            .clone();
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            if !page.has(key)
                && let Some(value) = inherited(source, id, key)?
            {
                page.set(key.to_vec(), value);
            }
        }
        page.remove(b"Parent");
        projected.insert(id, page);
    }
    let pages_id = document.new_object_id();
    let source_ids: Vec<_> = order.iter().map(|number| pages[number]).collect();
    let mut ids = Vec::with_capacity(source_ids.len());
    let mut copied = BTreeSet::new();
    for &source_id in &source_ids {
        let id = if copied.insert(source_id) {
            source_id
        } else {
            let mut page = projected[&source_id].clone();
            page.set("Parent", pages_id);
            document.add_object(page)
        };
        ids.push(id);
    }
    for (&id, page) in &mut projected {
        if ids.contains(&id) {
            page.set("Parent", pages_id);
        }
        document
            .objects
            .insert(id, Object::Dictionary(page.clone()));
    }
    let mut tree = Dictionary::new();
    tree.set("Type", "Pages");
    tree.set(
        "Kids",
        ids.iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    tree.set("Count", ids.len() as i64);
    document.objects.insert(pages_id, Object::Dictionary(tree));
    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let root_id = document.add_object(catalog);
    document.trailer.set("Root", root_id);
    let info = copied_info(source)?;
    if !info.is_empty() {
        let info_id = document.add_object(info);
        document.trailer.set("Info", info_id);
    }
    document.prune_objects();
    let expected_objects = document.objects.clone();
    let expected_contents: Vec<_> = source_ids
        .iter()
        .map(|id| source.get_page_contents(*id))
        .collect();
    if recover_original {
        let mut bytes = BoundedProjection { bytes: Vec::new() };
        document.save_to(&mut bytes).map_err(|_| PdfEditError::OutputWriteFailed)?;
        retain_original_projection(output, &bytes.bytes)?;
    } else {
        save_new_pdf(&mut document, output)?;
    }
    let verify = || -> Result<(), PdfEditError> {
        let reparsed = Document::load_with_options(
            output,
            LoadOptions::with_max_decompressed_size(crate::MAX_DECOMPRESSED_STREAM_BYTES),
        )
        .map_err(|_| PdfEditError::ValidationFailed)?;
        if reparsed.is_encrypted() {
            return Err(PdfEditError::ValidationFailed);
        }
        let actual_ids: Vec<_> = reparsed.get_pages().into_values().collect();
        if actual_ids != ids {
            return Err(PdfEditError::ValidationFailed);
        }
        for (&id, expected) in ids.iter().zip(&expected_contents) {
            if reparsed.get_page_contents(id) != *expected {
                return Err(PdfEditError::ValidationFailed);
            }
        }
        for (&id, expected) in &expected_objects {
            if !is_writer_housekeeping_object(expected)
                && !reparsed
                    .objects
                    .get(&id)
                    .is_some_and(|actual| pdf_objects_semantically_equal(expected, actual))
            {
                return Err(PdfEditError::ValidationFailed);
            }
        }
        Ok(())
    };
    if let Err(error) = verify() {
        if !recover_original { let _ = fs::remove_file(output); }
        return Err(error);
    }
    Ok(OrganizeResult {
        input_sha256: input_sha256.to_owned(),
        output_sha256: sha256_file(output)?,
        output_bytes: fs::metadata(output)
            .map_err(|_| PdfEditError::OutputWriteFailed)?
            .len(),
        page_count: pages.len(),
        output_page_count: ids.len(),
        signature_effect,
        page_content_preserved: true,
        retained_objects_preserved: true,
    })
}

const MAX_RECOVERY_PROJECTION_BYTES: u64 = 300 << 20;

struct BoundedProjection { bytes: Vec<u8> }
impl Write for BoundedProjection {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > MAX_RECOVERY_PROJECTION_BYTES.saturating_sub(self.bytes.len() as u64) {
            return Err(io::Error::other("bounded native projection exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl Drop for BoundedProjection { fn drop(&mut self) { self.bytes.fill(0); } }

// Positive missing-file observation only; any other metadata/read failure holds.
fn matches_original_projection(path: &Path, expected: &[u8]) -> Result<bool, PdfEditError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(PdfEditError::OutputWriteFailed),
        Ok(metadata) if !metadata.file_type().is_file() || metadata.len() != expected.len() as u64 =>
            return Err(PdfEditError::ValidationFailed),
        Ok(_) => (),
    }
    let mut file = File::open(path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let metadata = file.metadata().map_err(|_| PdfEditError::OutputWriteFailed)?;
    if !metadata.is_file() || metadata.len() != expected.len() as u64 { return Err(PdfEditError::ValidationFailed); }
    let mut buffer = [0_u8; 64 << 10];
    for chunk in expected.chunks(buffer.len()) {
        file.read_exact(&mut buffer[..chunk.len()]).map_err(|_| PdfEditError::ValidationFailed)?;
        if &buffer[..chunk.len()] != chunk { buffer.fill(0); return Err(PdfEditError::ValidationFailed); }
    }
    let mut extra = [0_u8; 1];
    let at_end = file.read(&mut extra).map_err(|_| PdfEditError::ValidationFailed)? == 0;
    buffer.fill(0);
    if !at_end { return Err(PdfEditError::ValidationFailed); }
    Ok(true)
}

fn retain_original_projection(output: &Path, expected: &[u8]) -> Result<(), PdfEditError> {
    if expected.is_empty() || expected.len() as u64 > MAX_RECOVERY_PROJECTION_BYTES { return Err(PdfEditError::OutputWriteFailed); }
    let parent = output.parent().ok_or(PdfEditError::OutputWriteFailed)?;
    // This protocol belongs only to the original Premium OCR temporary input.
    if output.file_name().and_then(|name| name.to_str()) != Some("premium-ocr-input.pdf") { return Err(PdfEditError::OutputWriteFailed); }
    let pending = parent.join("premium-ocr-input.pdf.pending");
    if matches_original_projection(output,expected)? { return Ok(()); }
    if !matches_original_projection(&pending,expected)? {
        let mut file = match File::options().write(true).create_new(true).open(&pending) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // Another actual writer won. Compare; never replace its bytes.
                if !matches_original_projection(&pending,expected)? { return Err(PdfEditError::OutputWriteFailed); }
                return link_original_projection(&pending,output,expected);
            }
            Err(_) => return Err(PdfEditError::OutputWriteFailed),
        };
        file.write_all(expected).map_err(|_| PdfEditError::OutputWriteFailed)?;
        file.sync_all().map_err(|_| PdfEditError::OutputWriteFailed)?;
    }
    if !matches_original_projection(&pending,expected)? { return Err(PdfEditError::ValidationFailed); }
    link_original_projection(&pending,output,expected)
}

fn link_original_projection(pending: &Path,output: &Path,expected: &[u8]) -> Result<(),PdfEditError> {
    match fs::hard_link(pending,output) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err(PdfEditError::OutputWriteFailed),
    }
    if !matches_original_projection(output,expected)? { return Err(PdfEditError::ValidationFailed); }
    File::options().read(true).write(true).open(output)
        .and_then(|file| file.sync_all()).map_err(|_| PdfEditError::OutputWriteFailed)?;
    #[cfg(unix)]
    File::open(output.parent().ok_or(PdfEditError::OutputWriteFailed)?)
        .and_then(|directory| directory.sync_all()).map_err(|_| PdfEditError::OutputWriteFailed)?;
    // Retain the fixed pending alias: cleanup belongs to the original source
    // owner after actual attempt retirement, never to an uncertain child exit.
    Ok(())
}
