//! Replacing an image XObject with a PNG (alpha kept as a soft mask). The
//! engine's PDF builder encodes the PNG exactly as it does for added images;
//! its image stream (and soft mask) then takes the target object's number, so
//! every placement of the image shows the new picture. JPEG replacement uses
//! `wellfriendpdf-edit::replace_image`.

use lopdf::{Document, Object, ObjectId};
use wellfriend_pdf_edit::ImageSelector;
use wellfriendpdf_engine::{AuthorPageSize, PdfBuilder};

use crate::error::EditorError;
use crate::overlay::{load, save};

fn refused(message: impl Into<String>) -> EditorError {
    EditorError::new("EDITOR_IMAGE_REFUSED", message)
}

fn first_image(document: &Document) -> Option<ObjectId> {
    document.objects.iter().find_map(|(id, object)| {
        let stream = object.as_stream().ok()?;
        (stream.dict.get(b"Subtype").ok()?.as_name().ok()? == b"Image" && !is_smask_target(document, *id)).then_some(*id)
    })
}

fn is_smask_target(document: &Document, id: ObjectId) -> bool {
    document.objects.values().any(|object| {
        object
            .as_stream()
            .ok()
            .and_then(|s| s.dict.get(b"SMask").ok())
            .and_then(|m| m.as_reference().ok())
            == Some(id)
    })
}

pub fn replace_image_png(input: &[u8], selector: &ImageSelector, png: &[u8]) -> Result<Vec<u8>, EditorError> {
    let mut document = load(input)?;
    let target: ObjectId = (selector.image_object.number, selector.image_object.generation);
    let current = document
        .get_object(target)
        .and_then(Object::as_stream)
        .map_err(|_| refused("the image no longer exists"))?;
    let is_image = current.dict.get(b"Subtype").ok().and_then(|s| s.as_name().ok()) == Some(b"Image".as_slice());
    let width = current.dict.get(b"Width").ok().and_then(|w| w.as_i64().ok());
    let height = current.dict.get(b"Height").ok().and_then(|h| h.as_i64().ok());
    if !is_image
        || width != Some(i64::from(selector.expected_width))
        || height != Some(i64::from(selector.expected_height))
    {
        return Err(EditorError::new(
            "EDITOR_IMAGE_STALE",
            "the image changed since it was selected; reopen the current version",
        ));
    }

    let mut builder = PdfBuilder::new();
    let handle = builder.add_png_image(png)?;
    builder.add_page(AuthorPageSize::custom(10.0, 10.0)).draw_image(handle, 0.0, 0.0, 10.0, 10.0);
    let mut authored = load(&builder.to_bytes()?)?;
    authored.renumber_objects_with(document.max_id + 1);
    let source = first_image(&authored).ok_or_else(|| refused("the PNG could not be encoded"))?;
    let mut stream = authored
        .get_object(source)
        .and_then(Object::as_stream)
        .map_err(|_| refused("the PNG could not be encoded"))?
        .clone();
    if let Ok(mask) = stream.dict.get(b"SMask").and_then(Object::as_reference) {
        let mask_stream = authored.get_object(mask).map_err(|_| refused("the PNG mask is missing"))?.clone();
        document.objects.insert(mask, mask_stream);
        document.max_id = document.max_id.max(mask.0);
    } else {
        stream.dict.remove(b"SMask");
    }
    document.objects.insert(target, Object::Stream(stream));
    save(&mut document)
}
