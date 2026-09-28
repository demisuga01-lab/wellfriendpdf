//! Text writers may use a named alias for an ExtGState-only source font.
//! Publish that alias atomically with the content, without replaying gs (which
//! could also reset unrelated alpha, clipping, line or compositing state).
use super::*;
#[cfg(test)]
#[path = "advanced_ext_gstate_tests.rs"]
mod tests;

pub(super) fn write(
    reader: &crate::PdfReader,
    page: &crate::document::PdfPage,
    parsed: &PageResources,
    updates: Vec<IncrementalObject>,
) -> Result<Vec<u8>> {
    write_with_protected_fonts(reader, page, parsed, updates, &BTreeSet::new())
}

pub(super) fn write_with_protected_fonts(
    reader: &crate::PdfReader,
    page: &crate::document::PdfPage,
    parsed: &PageResources,
    mut updates: Vec<IncrementalObject>,
    protected_fonts: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    let slot = updates
        .iter()
        .position(|u| (u.number, u.generation) == (page.object_number, page.generation_number));
    let object = match slot {
        Some(index) => updates[index].object.clone(),
        None => reader.get_object(page.object_number, page.generation_number)?,
    };
    let mut dictionary = object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("text resource owner is not a page dictionary".into())
    })?;
    let mut resources = match dictionary.get("Resources") {
        Some(value) => reader
            .resolve(value.clone())?
            .as_dict()
            .cloned()
            .ok_or_else(|| {
                WellfriendError::MalformedPdf("text resource owner has invalid Resources".into())
            })?,
        None => page.resources.clone(), // inherited page resources
    };
    let materialized = crate::ext_gstate_fonts::materialize(parsed, &mut resources, reader)?;
    let content_roots = dictionary
        .get("Contents")
        .cloned()
        .map(|contents| vec![contents])
        .unwrap_or_else(|| {
            page.contents
                .iter()
                .map(|&(number, generation)| PdfObject::Reference { number, generation })
                .collect()
        });
    let annotations = dictionary.get("Annots").cloned();
    let retired = retire_unreferenced_generated_fonts(
        reader,
        &content_roots,
        &mut resources,
        annotations,
        &updates,
        protected_fonts,
    )?;
    if materialized || retired != 0 {
        dictionary.insert("Resources", PdfObject::Dictionary(resources));
        if let Some(index) = slot {
            updates[index].object = PdfObject::Dictionary(dictionary);
        } else {
            updates.push(IncrementalObject {
                number: page.object_number,
                generation: page.generation_number,
                object: PdfObject::Dictionary(dictionary),
            });
        }
    }
    write_incremental_update(reader, updates)
}
