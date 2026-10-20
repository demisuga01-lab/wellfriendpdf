//! Bulk job rotation using the existing parser, writer and preservation checks.
use crate::{
    PdfEditError, SignatureEffect, checked_metadata, is_writer_housekeeping_object,
    load_checked_pdf, page_geometry, pdf_objects_semantically_equal, save_new_pdf, sha256_file,
    signature_summaries, validate_paths,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationResult {
    pub input_sha256: String,
    pub output_sha256: String,
    pub output_bytes: u64,
    pub page_count: usize,
    pub rotated_pages: usize,
    pub signature_effect: SignatureEffect,
    pub page_content_preserved: bool,
    pub non_target_objects_preserved: bool,
}

/// `rotations` contains relative clockwise quarter-turns. Business admission and
/// any signature policy are the caller's authority; this core performs no I/O
/// outside the two supplied paths and creates no financial/document state.
pub fn rotate_pages(
    input: &Path,
    output: &Path,
    rotations: &BTreeMap<u32, i32>,
    allow_signature_invalidation: bool,
) -> Result<RotationResult, PdfEditError> {
    validate_paths(input, output)?;
    checked_metadata(input)?;
    let input_sha256 = sha256_file(input)?;
    let mut document = load_checked_pdf(input)?;
    let pages = document.get_pages();
    if pages.is_empty() || rotations.is_empty() {
        return Err(PdfEditError::InvalidPageGeometry);
    }
    let signature_effect = if signature_summaries(&document).is_empty() {
        SignatureEffect::None
    } else if allow_signature_invalidation {
        SignatureEffect::InvalidatesExistingSignatures
    } else {
        return Err(PdfEditError::SignatureConfirmationRequired);
    };
    let original_objects = document.objects.clone();
    let mut expected = BTreeMap::new();
    for (&number, &angle) in rotations {
        if !matches!(angle, 90 | 180 | 270) {
            return Err(PdfEditError::InvalidPageGeometry);
        }
        let id = *pages.get(&number).ok_or(PdfEditError::PageNotFound)?;
        let geometry = page_geometry(&document, number, id)?;
        let rotation = (geometry.rotation + angle).rem_euclid(360);
        document
            .get_dictionary_mut(id)
            .map_err(|_| PdfEditError::PageTreeUnsupported)?
            .set("Rotate", i64::from(rotation));
        expected.insert(number, (id, geometry, rotation));
    }
    // Only the selected page dictionary's Rotate key may change. In particular,
    // inherited boxes/resources and content/form/annotation objects stay intact.
    for (&id, original) in &original_objects {
        let mut reference = original.clone();
        if let Some((_, _, rotation)) = expected.values().find(|(page, _, _)| *page == id) {
            reference
                .as_dict_mut()
                .map_err(|_| PdfEditError::ValidationFailed)?
                .set("Rotate", i64::from(*rotation));
        }
        if document.objects.get(&id) != Some(&reference) {
            return Err(PdfEditError::ValidationFailed);
        }
    }
    save_new_pdf(&mut document, output)?;
    let verify = || -> Result<(), PdfEditError> {
        let reparsed = load_checked_pdf(output)?;
        if reparsed.get_pages() != pages {
            return Err(PdfEditError::ValidationFailed);
        }
        for (&number, &(id, ref before, rotation)) in &expected {
            let after = page_geometry(&reparsed, number, id)?;
            if before.media_box != after.media_box
                || before.crop_box != after.crop_box
                || after.rotation != rotation
                || document.get_page_contents(id) != reparsed.get_page_contents(id)
            {
                return Err(PdfEditError::ValidationFailed);
            }
        }
        for (&id, original) in &document.objects {
            if !is_writer_housekeeping_object(original)
                && !reparsed
                    .objects
                    .get(&id)
                    .is_some_and(|object| pdf_objects_semantically_equal(original, object))
            {
                return Err(PdfEditError::ValidationFailed);
            }
        }
        Ok(())
    };
    if let Err(error) = verify() {
        let _ = fs::remove_file(output);
        return Err(error);
    }
    Ok(RotationResult {
        input_sha256,
        output_sha256: sha256_file(output)?,
        output_bytes: fs::metadata(output)
            .map_err(|_| PdfEditError::OutputWriteFailed)?
            .len(),
        page_count: pages.len(),
        rotated_pages: rotations.len(),
        signature_effect,
        page_content_preserved: true,
        non_target_objects_preserved: true,
    })
}
