//! What the editor can select on a page: image placements, vector paths and
//! the page's fonts, with the selectors the operations take.

use serde::Serialize;
use wellfriend_pdf_edit::{EditableImage, EditableImagePlacement, EditableVectorPath};

use crate::apply::matrix_bounds;
use crate::error::EditorError;
use crate::fonts::{page_fonts, SourceFont};
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
    /// Objects the inspection could not certify for editing on this page.
    pub unsupported_images: usize,
    pub unsupported_vectors: usize,
    pub signatures: usize,
}

pub fn page_objects(bytes: &[u8], page: u32) -> Result<PageObjects, EditorError> {
    let document = overlay::load(bytes)?;
    let page_object = overlay::page_id(&document, page)?;
    let media_box = overlay::media_box(&document, page_object)?;
    let fonts = page_fonts(&document, page);
    drop(document);
    let inspection = scratch::inspect(bytes, Some(page))?;
    let images = inspection
        .editable_image_placements
        .iter()
        .filter(|p| p.selector.page_number == page)
        .map(|placement| PageImage {
            bounds: matrix_bounds(&placement.matrix),
            image: inspection
                .editable_images
                .iter()
                .find(|image| {
                    image.selector.image_object == placement.selector.expected_image_object
                })
                .cloned(),
            placement: placement.clone(),
        })
        .collect();
    let vectors = inspection
        .editable_vector_paths
        .iter()
        .filter(|v| v.selector.page_number == page)
        .cloned()
        .collect();
    let on_page = |value: &serde_json::Value| value["pageNumber"].as_u64() == Some(u64::from(page));
    let unsupported_images = serde_json::to_value(&inspection.unsupported_image_placements)
        .ok()
        .and_then(|v| {
            v.as_array()
                .map(|a| a.iter().filter(|v| on_page(v)).count())
        })
        .unwrap_or(0);
    let unsupported_vectors = serde_json::to_value(&inspection.unsupported_vector_path_groups)
        .ok()
        .and_then(|v| {
            v.as_array()
                .map(|a| a.iter().filter(|v| on_page(v)).count())
        })
        .unwrap_or(0);
    Ok(PageObjects {
        page,
        media_box,
        images,
        vectors,
        fonts,
        unsupported_images,
        unsupported_vectors,
        signatures: inspection.signatures.len(),
    })
}
