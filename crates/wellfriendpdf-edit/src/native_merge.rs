//! Native ordered page import using the existing parser and canonical writer.
use crate::{
    PdfEditError, SignatureEffect, is_writer_housekeeping_object, pdf_objects_semantically_equal,
    save_new_pdf, sha256_file, signature_summaries,
};
use lopdf::{Dictionary, Document, LoadOptions, Object};
use serde::Serialize;
use std::{fs, path::Path};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeResult {
    pub input_page_counts: Vec<usize>,
    pub output_page_count: usize,
    pub output_sha256: String,
    pub output_bytes: u64,
    pub signature_effect: SignatureEffect,
    pub page_content_preserved: bool,
    pub retained_objects_preserved: bool,
}

/// The original merge creates a fresh catalog and imports ordered page graphs;
/// it does not copy source outlines, document metadata or AcroForm catalogs.
struct MergePlan {
    document: Document,
    ordered: Vec<lopdf::ObjectId>,
    expected_contents: Vec<Vec<lopdf::ObjectId>>,
    counts: Vec<usize>,
    signature_effect: SignatureEffect,
}

fn prepare_merge(sources: Vec<Document>) -> Result<MergePlan, PdfEditError> {
    if sources.len() < 2 || sources.len() > 30 {
        return Err(PdfEditError::InvalidPageMutation);
    }
    let mut target = Document::with_version("1.3");
    let mut ordered = Vec::new();
    let mut expected_contents = Vec::new();
    let mut counts = Vec::with_capacity(sources.len());
    let mut signature_effect = SignatureEffect::None;
    for mut source in sources {
        if source.is_encrypted() {
            return Err(PdfEditError::EncryptedPdf);
        }
        let pages = source.get_pages();
        if pages.is_empty() {
            return Err(PdfEditError::PageNotFound);
        }
        if pages.len() > 2000 || ordered.len() + pages.len() > 60_000 {
            return Err(PdfEditError::PageLimitExceeded);
        }
        if !signature_summaries(&source).is_empty() {
            signature_effect = SignatureEffect::InvalidatesExistingSignatures;
        }
        counts.push(pages.len());
        let mut baked = Vec::with_capacity(pages.len());
        for id in pages.into_values() {
            let mut page = source
                .get_dictionary(id)
                .map_err(|_| PdfEditError::PageTreeUnsupported)?
                .clone();
            for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
                if !page.has(key) {
                    if let Some(value) = crate::native_organize::inherited(&source, id, key)? {
                        page.set(key.to_vec(), value);
                    }
                }
            }
            page.remove(b"Parent");
            baked.push((id, page));
        }
        for (id, page) in baked {
            source.objects.insert(id, Object::Dictionary(page));
        }
        source.renumber_objects_with(
            target
                .max_id
                .checked_add(1)
                .ok_or(PdfEditError::PageTreeUnsupported)?,
        );
        let pages = source
            .objects
            .iter()
            .filter_map(|(&id, object)| {
                object
                    .as_dict()
                    .ok()
                    .filter(|dictionary| {
                        dictionary.get(b"Type").and_then(Object::as_name).ok() == Some(b"Page")
                    })
                    .map(|_| id)
            })
            .collect::<std::collections::BTreeSet<_>>();
        // renumber_objects_with rewrites all references, including the original
        // tree's ordered Kids. Its page iterator preserves presentation order.
        let imported = source.get_pages();
        if imported.len() != *counts.last().unwrap()
            || imported.values().any(|id| !pages.contains(id))
        {
            return Err(PdfEditError::ValidationFailed);
        }
        for id in imported.into_values() {
            expected_contents.push(source.get_page_contents(id));
            ordered.push(id);
        }
        target.max_id = source.max_id;
        target.objects.append(&mut source.objects);
    }
    let pages_id = target.new_object_id();
    for &id in &ordered {
        target
            .get_dictionary_mut(id)
            .map_err(|_| PdfEditError::PageTreeUnsupported)?
            .set("Parent", pages_id);
    }
    let mut tree = Dictionary::new();
    tree.set("Type", "Pages");
    tree.set(
        "Kids",
        ordered
            .iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    tree.set("Count", ordered.len() as i64);
    target.objects.insert(pages_id, Object::Dictionary(tree));
    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = target.add_object(catalog);
    target.trailer.set("Root", catalog_id);
    target.prune_objects();
    Ok(MergePlan {
        document: target,
        ordered,
        expected_contents,
        counts,
        signature_effect,
    })
}

fn verify_plan(
    plan: &MergePlan,
    reparsed: &Document,
    expected_objects: &std::collections::BTreeMap<lopdf::ObjectId, Object>,
) -> Result<(), PdfEditError> {
    if reparsed.is_encrypted()
        || reparsed.get_pages().into_values().collect::<Vec<_>>() != plan.ordered
    {
        return Err(PdfEditError::ValidationFailed);
    }
    for (&id, expected) in plan.ordered.iter().zip(&plan.expected_contents) {
        if reparsed.get_page_contents(id) != *expected {
            return Err(PdfEditError::ValidationFailed);
        }
    }
    for (&id, expected) in expected_objects {
        if !is_writer_housekeeping_object(expected)
            && !reparsed
                .objects
                .get(&id)
                .is_some_and(|actual| pdf_objects_semantically_equal(expected, actual))
        {
            return Err(PdfEditError::ValidationFailed);
        }
    }
    let root = reparsed
        .catalog()
        .map_err(|_| PdfEditError::ValidationFailed)?;
    if root
        .iter()
        .any(|(key, _)| !matches!(key.as_slice(), b"Type" | b"Pages"))
        || reparsed.trailer.has(b"Info")
    {
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(())
}

fn merge_result(plan: MergePlan, candidate: &Path) -> Result<MergeResult, PdfEditError> {
    Ok(MergeResult {
        input_page_counts: plan.counts,
        output_page_count: plan.ordered.len(),
        output_sha256: sha256_file(candidate)?,
        output_bytes: fs::metadata(candidate)
            .map_err(|_| PdfEditError::OutputWriteFailed)?
            .len(),
        signature_effect: plan.signature_effect,
        page_content_preserved: true,
        retained_objects_preserved: true,
    })
}

pub fn merge_documents(sources: Vec<Document>, output: &Path) -> Result<MergeResult, PdfEditError> {
    if output.exists() || fs::symlink_metadata(output).is_ok() {
        return Err(PdfEditError::OutputAlreadyExists);
    }
    let mut plan = prepare_merge(sources)?;
    let expected = plan.document.objects.clone();
    save_new_pdf(&mut plan.document, output)?;
    let verify = || -> Result<(), PdfEditError> {
        let reparsed = Document::load_with_options(
            output,
            LoadOptions::with_max_decompressed_size(crate::MAX_DECOMPRESSED_STREAM_BYTES),
        )
        .map_err(|_| PdfEditError::ValidationFailed)?;
        verify_plan(&plan, &reparsed, &expected)
    };
    if let Err(error) = verify() {
        let _ = fs::remove_file(output);
        return Err(error);
    }
    merge_result(plan, output)
}

/// Verify an already-published candidate against ordered authenticated sources.
/// The native import graph is constructed in memory; no output is written and
/// no transformation, storage publication or business effect is repeated.
pub fn verify_merged_documents(
    sources: Vec<Document>,
    candidate: &Path,
) -> Result<MergeResult, PdfEditError> {
    let plan = prepare_merge(sources)?;
    let reparsed = Document::load_with_options(
        candidate,
        LoadOptions::with_max_decompressed_size(crate::MAX_DECOMPRESSED_STREAM_BYTES),
    )
    .map_err(|_| PdfEditError::ValidationFailed)?;
    verify_plan(&plan, &reparsed, &plan.document.objects)?;
    merge_result(plan, candidate)
}
