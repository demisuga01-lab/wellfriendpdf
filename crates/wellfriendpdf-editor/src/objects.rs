//! What the editor can select on a page: image placements, vector paths and
//! the page's fonts, with the selectors the operations take.
//!
//! `images` and `vectors` are the placements the `wellfriendpdf-edit`
//! recognisers certify (they also take crop, opacity, replacement and
//! in-place recolour). `objects` lists every other image placement, inline
//! image, path group and shading of the page, inside Form XObjects too
//! (`content_objects`), which `editObject` moves, resizes, recolours and
//! deletes.

use serde::Serialize;
use wellfriend_pdf_edit::{EditableFormField, EditableImage, EditableImagePlacement, EditableVectorPath};

use crate::apply::matrix_bounds;
use crate::error::EditorError;
use crate::fonts::{page_fonts, SourceFont};
use crate::content_objects::{page_content_objects, ContentObject, ContentObjectKind};
use crate::{overlay, scratch};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageImage {
    #[serde(flatten)]
    pub placement: EditableImagePlacement,
    /// User-space bounds `[x0, y0, x1, y1]` of the placement.
    pub bounds: [f64; 4],
    /// The image XObject (for replacement), when the inspection lists it.
    pub image: Option<EditableImage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageObjects {
    pub page: u32,
    pub media_box: [f64; 4],
    pub images: Vec<PageImage>,
    pub vectors: Vec<EditableVectorPath>,
    pub fonts: Vec<SourceFont>,
    /// AcroForm fields with a widget on this page.
    pub forms: Vec<EditableFormField>,
    /// Every other image placement, inline image, path group and shading.
    pub objects: Vec<ContentObject>,
    /// More than `content_objects::MAX_PAGE_OBJECTS` objects: the first are listed.
    pub objects_truncated: bool,
    /// A content stream of the page does not parse: its objects are not listed.
    pub content_unreadable: bool,
    /// Image placements and paths on this page that cannot be selected at
    /// all (in content or forms that do not parse).
    pub unsupported_images: usize,
    pub unsupported_vectors: usize,
    pub signatures: usize,
}

pub fn page_objects(bytes: &[u8], page: u32) -> Result<PageObjects, EditorError> {
    let document = overlay::load(bytes)?;
    let page_object = overlay::page_id(&document, page)?;
    let media_box = overlay::media_box(&document, page_object)?;
    let fonts = page_fonts(&document, page);
    let content = page_content_objects(&document, page)?;
    drop(document);
    // The recognisers refuse some pages outright (a content stream they
    // cannot decode); the content reader still lists that page's objects.
    let inspection = scratch::inspect(bytes, Some(page)).ok();
    let images: Vec<PageImage> = inspection
        .iter()
        .flat_map(|inspection| {
            inspection
                .editable_image_placements
                .iter()
                .filter(|p| p.selector.page_number == page)
                .map(|placement| PageImage {
                    bounds: matrix_bounds(&placement.matrix),
                    image: inspection
                        .editable_images
                        .iter()
                        .find(|image| image.selector.image_object == placement.selector.expected_image_object)
                        .cloned(),
                    placement: placement.clone(),
                })
        })
        .collect();
    let vectors: Vec<EditableVectorPath> = inspection
        .iter()
        .flat_map(|inspection| inspection.editable_vector_paths.iter().filter(|v| v.selector.page_number == page).cloned())
        .collect();
    // The legacy counts were whole-document numbers read as arrays (always
    // 0, QF-2605); what cannot be selected now is what the content reader
    // could not read.
    let objects = uncovered(content.objects, &images, &vectors);
    Ok(PageObjects {
        page,
        media_box,
        images,
        vectors,
        fonts,
        forms: inspection
            .iter()
            .flat_map(|inspection| inspection.editable_form_fields.iter())
            .filter(|f| f.page_number == page || f.widgets.iter().any(|w| w.selector.expected_page_number == page))
            .cloned()
            .collect(),
        objects,
        objects_truncated: content.truncated,
        content_unreadable: content.content_unreadable,
        unsupported_images: content.unreadable_images,
        unsupported_vectors: content.unreadable_paths,
        signatures: inspection.as_ref().map_or(0, |i| i.signatures.len()),
    })
}

/// The content objects the certified `images` and `vectors` do not cover.
fn uncovered(objects: Vec<ContentObject>, images: &[PageImage], vectors: &[EditableVectorPath]) -> Vec<ContentObject> {
    let id = |o: &wellfriend_pdf_edit::PdfObjectId| (o.number, o.generation);
    objects
        .into_iter()
        .filter(|object| {
            let Some((stream, start, end)) = object.page_span() else {
                return true;
            };
            match object.selector.kind {
                ContentObjectKind::Image => !images.iter().any(|image| {
                    let s = &image.placement.selector;
                    id(&s.content_stream_object) == stream && s.draw_operation_index == start
                }),
                ContentObjectKind::Path => !vectors.iter().any(|vector| {
                    let s = &vector.selector;
                    id(&s.content_stream_object) == stream && s.start_operation_index < start && end < s.end_operation_index
                }),
                _ => true,
            }
        })
        .collect()
}
