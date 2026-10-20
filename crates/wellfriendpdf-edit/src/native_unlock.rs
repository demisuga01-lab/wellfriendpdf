//! Native password removal; caller retains custody, policy and publication.
use crate::{
    PdfEditError, SignatureEffect, is_writer_housekeeping_object, load_checked_pdf,
    pdf_objects_semantically_equal, save_new_pdf, sha256_file, signature_summaries,
};
use lopdf::{Document, LoadOptions};
use serde::Serialize;
use std::{fs, path::Path};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockResult {
    pub output_sha256: String,
    pub output_bytes: u64,
    pub page_count: usize,
    pub signature_effect: SignatureEffect,
    pub page_content_preserved: bool,
    pub decrypted_objects_preserved: bool,
}

pub fn load(input: &Path, password: &str) -> Result<Document, PdfEditError> {
    // Supply the password at load time: an unauthenticated lopdf document may
    // contain only its encryption dictionary, with no page/object graph.
    let mut options = LoadOptions::with_max_decompressed_size(64 * 1024 * 1024);
    options.password = Some(password.to_owned());
    let mut document =
        Document::load_with_options(input, options).map_err(|_| PdfEditError::EncryptedPdf)?;
    if let Some(state) = &document.encryption_state {
        // lopdf tries an empty user password before a supplied password. The
        // original worker authenticates the supplied password even when empty
        // access is possible. Authenticate its retained verifier independently;
        // this temporary document never contains page content or is written.
        let mut verifier = Document::new();
        verifier.trailer = document.trailer.clone();
        let encryption = state.encode().map_err(|_| PdfEditError::EncryptedPdf)?;
        let id = verifier.add_object(encryption);
        verifier.trailer.set("Encrypt", id);
        verifier
            .authenticate_password(password)
            .map_err(|_| PdfEditError::EncryptedPdf)?;
    }
    if document.is_encrypted() {
        return Err(PdfEditError::EncryptedPdf);
    }
    document.encryption_state = None;
    Ok(document)
}

pub fn write(document: &mut Document, output: &Path) -> Result<UnlockResult, PdfEditError> {
    if document.is_encrypted() {
        return Err(PdfEditError::EncryptedPdf);
    }
    let source = document.objects.clone();
    let trailer = document.trailer.clone();
    let page_count = document.get_pages().len();
    if page_count == 0 || page_count > crate::MAX_PAGE_COUNT {
        return Err(PdfEditError::PageLimitExceeded);
    }
    let signature_effect = if signature_summaries(document).is_empty() {
        SignatureEffect::None
    } else {
        SignatureEffect::InvalidatesExistingSignatures
    };
    let parent = output.parent().ok_or(PdfEditError::OutputWriteFailed)?;
    let scratch = tempfile::Builder::new()
        .prefix("native-pdf-unlock-")
        .tempdir_in(parent)
        .map_err(|_| PdfEditError::OutputWriteFailed)?;
    let staged = scratch.path().join("unlocked.pdf");
    save_new_pdf(document, &staged)?;
    let verified = load_checked_pdf(&staged)?;
    let preserved = source.iter().all(|(id, value)| {
        is_writer_housekeeping_object(value)
            || value.type_name().is_ok_and(|kind| kind == b"Linearized")
            || verified
                .objects
                .get(id)
                .is_some_and(|after| pdf_objects_semantically_equal(value, after))
    }) && verified.objects.iter().all(|(id, value)| {
        source.contains_key(id) || value.type_name().is_ok_and(|kind| kind == b"XRef")
    });
    if !preserved || verified.is_encrypted() || verified.get_pages() != document.get_pages() {
        return Err(PdfEditError::ValidationFailed);
    }
    // Encryption and regenerated packing fields change; original semantic
    // trailer entries (Root, Info, ID and any extension) must retain their values.
    for (key, value) in trailer.iter() {
        let packing = key.as_slice() == b"Size"
            || trailer
                .get(b"Type")
                .and_then(lopdf::Object::as_name)
                .is_ok_and(|kind| kind == b"XRef")
                && matches!(
                    key.as_slice(),
                    b"Type" | b"W" | b"Index" | b"Length" | b"Filter"
                );
        if !packing && verified.trailer.get(key).ok() != Some(value) {
            return Err(PdfEditError::ValidationFailed);
        }
    }
    let output_sha256 = sha256_file(&staged)?;
    let output_bytes = fs::metadata(&staged)
        .map_err(|_| PdfEditError::OutputWriteFailed)?
        .len();
    // Both paths share the same owned filesystem. hard_link atomically refuses
    // an existing destination; dropping scratch removes only our staging link.
    fs::hard_link(&staged, output).map_err(|_| PdfEditError::OutputWriteFailed)?;
    Ok(UnlockResult {
        output_sha256,
        output_bytes,
        page_count,
        signature_effect,
        page_content_preserved: true,
        decrypted_objects_preserved: true,
    })
}
