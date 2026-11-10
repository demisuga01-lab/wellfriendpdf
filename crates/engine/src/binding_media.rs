//! Binding-neutral helpers for page media and text search.
//!
//! The C ABI, Python, WASM and CLI surfaces expose page thumbnails, single
//! image-object rendering, `pdfimages -all` style original-image extraction
//! and option-driven text search. This module keeps the image indexing, the
//! selector rules and the JSON shapes in one place so every binding reports
//! the same fields.

use serde::{Deserialize, Serialize};

use crate::error::{Result, WellfriendError};
use crate::images::decoder::PassthroughImage;
use crate::images::encoder::ImageEncoder;
use crate::images::locator::{ImageLocateOptions, ImageReference};
use crate::render::PageImageObjectSelector;
use crate::text::{TextSearchMatch, TextSearchOptions};
use crate::ContentEngine;

pub const TEXT_SEARCH_REPORT_SCHEMA_VERSION: &str = "binding_media.text_search.v1";

/// One image listed on a page, in the order used by the `index` arguments of
/// [`page_image_reference`] and the binding `extract_image_original` calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageImageInfo {
    /// Zero-based position among the page's listed images.
    pub index: usize,
    pub page: usize,
    /// XObject resource name, or a generated name for inline images.
    pub name: String,
    /// PDF object number (0 for inline images).
    pub object_number: u32,
    pub generation_number: u16,
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    pub color_space: String,
    pub filters: Vec<String>,
    pub inline: bool,
    pub mask: bool,
    pub soft_mask: bool,
}

/// A sidecar file that accompanies an original-encoding image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OriginalImageSidecarInfo {
    /// File extension: `jb2g` (JBIG2 globals) or `params` (CCITT fax2tiff
    /// options).
    pub extension: String,
    pub byte_length: usize,
}

/// Metadata for an image extracted in its original encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OriginalImageInfo {
    pub page: usize,
    pub index: usize,
    pub name: String,
    /// File extension of the bytes: `jpg`, `jp2`, `jb2e`, `ccitt` or `png`
    /// (when the image had no passthrough codec and was decoded).
    pub extension: String,
    /// Canonical PDF filter name of the codec (for example `DCTDecode`).
    pub filter: String,
    /// True when the bytes are the stored codec data rather than a PNG
    /// re-encoding.
    pub passthrough: bool,
    pub byte_length: usize,
    pub sidecars: Vec<OriginalImageSidecarInfo>,
}

impl OriginalImageInfo {
    pub fn new(page: usize, index: usize, name: &str, image: &PassthroughImage) -> Self {
        Self {
            page,
            index,
            name: name.to_string(),
            extension: image.extension.to_string(),
            filter: image.filter.clone(),
            passthrough: image.extension != "png",
            byte_length: image.bytes.len(),
            sidecars: image
                .sidecars
                .iter()
                .map(|(extension, bytes)| OriginalImageSidecarInfo {
                    extension: extension.to_string(),
                    byte_length: bytes.len(),
                })
                .collect(),
        }
    }
}

/// A text-search request: the pages to search (empty = all pages) plus the
/// [`TextSearchOptions`] fields, all optional in JSON.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TextSearchRequest {
    pub pages: Vec<usize>,
    #[serde(flatten)]
    pub options: TextSearchOptions,
}

impl TextSearchRequest {
    /// Parse a request from optional JSON; `None` or blank text yields the
    /// default request (all pages, default options).
    pub fn from_json(json: Option<&str>) -> Result<Self> {
        match json.map(str::trim).filter(|json| !json.is_empty()) {
            None => Ok(Self::default()),
            Some(json) => serde_json::from_str(json).map_err(|err| {
                WellfriendError::invalid_input(format!("invalid text search options JSON: {err}"))
            }),
        }
    }
}

/// Result of [`search_text_report`].
#[derive(Debug, Clone, Serialize)]
pub struct TextSearchReport {
    pub schema_version: &'static str,
    pub query: String,
    pub pages: Vec<usize>,
    pub options: TextSearchOptions,
    pub match_count: usize,
    pub matches: Vec<TextSearchMatch>,
}

/// Search the requested pages for `query` with the request's options.
pub fn search_text_report(
    engine: &ContentEngine,
    query: &str,
    request: &TextSearchRequest,
) -> Result<TextSearchReport> {
    if query.is_empty() {
        return Err(WellfriendError::invalid_input(
            "text search query must not be empty",
        ));
    }
    let page_count = engine.page_count()?;
    if let Some(page) = request
        .pages
        .iter()
        .find(|page| **page == 0 || **page > page_count)
    {
        return Err(WellfriendError::invalid_input(format!(
            "text search page {page} is outside 1..={page_count}"
        )));
    }
    let matches = engine.search_text(&request.pages, query, request.options.clone())?;
    Ok(TextSearchReport {
        schema_version: TEXT_SEARCH_REPORT_SCHEMA_VERSION,
        query: query.to_string(),
        pages: request.pages.clone(),
        options: request.options.clone(),
        match_count: matches.len(),
        matches,
    })
}

fn page_image_references(engine: &ContentEngine, page: usize) -> Result<Vec<ImageReference>> {
    let page_count = engine.page_count()?;
    if page == 0 || page > page_count {
        return Err(WellfriendError::invalid_input(format!(
            "page {page} is outside 1..={page_count}"
        )));
    }
    engine.find_all_images(&ImageLocateOptions {
        pages: Some(vec![page]),
        ..ImageLocateOptions::default()
    })
}

/// The images of a page (XObject and inline images; masks are not listed
/// separately), in index order.
pub fn page_image_infos(engine: &ContentEngine, page: usize) -> Result<Vec<PageImageInfo>> {
    Ok(page_image_references(engine, page)?
        .into_iter()
        .enumerate()
        .map(|(index, image)| PageImageInfo {
            index,
            page: image.page_number,
            name: image.xobject_name,
            object_number: image.object_number,
            generation_number: image.generation_number,
            width: image.width,
            height: image.height,
            bits_per_component: image.bits_per_component,
            color_space: image.color_space,
            filters: image.filter,
            inline: image.is_inline,
            mask: image.is_mask,
            soft_mask: image.is_smask,
        })
        .collect())
}

/// The `index`-th image of [`page_image_infos`].
pub fn page_image_reference(
    engine: &ContentEngine,
    page: usize,
    index: usize,
) -> Result<ImageReference> {
    let mut images = page_image_references(engine, page)?;
    let count = images.len();
    if index >= count {
        return Err(WellfriendError::invalid_input(format!(
            "image index {index} is out of range: page {page} lists {count} image(s)"
        )));
    }
    Ok(images.swap_remove(index))
}

/// Extract the `index`-th page image in its original encoding (see
/// [`ContentEngine::extract_image_original`]).
pub fn extract_page_image_original(
    engine: &ContentEngine,
    page: usize,
    index: usize,
) -> Result<(OriginalImageInfo, PassthroughImage)> {
    let image = page_image_reference(engine, page, index)?;
    let extracted = engine.extract_image_original(&image)?;
    let info = OriginalImageInfo::new(page, index, &image.xobject_name, &extracted);
    Ok((info, extracted))
}

/// The page's embedded `/Thumb` image as an RGBA PNG, or `None` when the
/// page has no thumbnail.
pub fn page_thumbnail_png(engine: &ContentEngine, page: usize) -> Result<Option<Vec<u8>>> {
    engine
        .page_thumbnail_rgba(page)?
        .map(|rgba| ImageEncoder::encode_png(&rgba))
        .transpose()
}

/// Render one image object of a page on its own (see
/// [`ContentEngine::render_page_image_object`]) as straight-alpha RGBA8:
/// `(width, height, pixels)`.
pub fn render_page_image_object_rgba(
    engine: &ContentEngine,
    page: usize,
    selector: &PageImageObjectSelector,
) -> Result<(u32, u32, Vec<u8>)> {
    let buffer = engine.render_page_image_object(page, selector)?;
    Ok((buffer.width, buffer.height, buffer.to_rgba_bytes()))
}

/// [`render_page_image_object_rgba`] encoded as an RGBA PNG.
pub fn render_page_image_object_png(
    engine: &ContentEngine,
    page: usize,
    selector: &PageImageObjectSelector,
) -> Result<Vec<u8>> {
    let buffer = engine.render_page_image_object(page, selector)?;
    ImageEncoder::encode_png(&buffer.to_raw_image_rgba())
}

/// Build a [`PageImageObjectSelector`]: a non-empty `name` selects by
/// resource name, otherwise `index` selects by top-level `Do` occurrence.
pub fn page_image_object_selector(
    index: Option<usize>,
    name: Option<&str>,
) -> Result<PageImageObjectSelector> {
    let name = name
        .map(|name| name.trim().trim_start_matches('/'))
        .filter(|name| !name.is_empty());
    match (index, name) {
        (Some(_), Some(_)) => Err(WellfriendError::invalid_input(
            "select an image object by index or by name, not both",
        )),
        (_, Some(name)) => Ok(PageImageObjectSelector::Name(name.to_string())),
        (Some(index), None) => Ok(PageImageObjectSelector::Index(index)),
        (None, None) => Ok(PageImageObjectSelector::Index(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One page with a 2x1 RGB `/Thumb`, a DCT image XObject (`/Jpg`, kept as
    /// opaque passthrough bytes), a 2x1 RGB image (`/Rgb`) and some text.
    fn media_fixture_pdf() -> Vec<u8> {
        let content = b"q 20 0 0 10 5 5 cm /Rgb Do Q q 4 0 0 4 50 50 cm /Jpg Do Q \
BT /F1 12 Tf 10 150 Td (The cat sat; concatenate the caf\xe9.) Tj ET";
        let objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Thumb 4 0 R \
/Resources << /XObject << /Rgb 5 0 R /Jpg 6 0 R >> /Font << /F1 7 0 R >> >> /Contents 8 0 R >>"
                .to_vec(),
            stream(
                "/Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
                &[255, 0, 0, 0, 0, 255],
            ),
            stream(
                "/Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB",
                &[0, 255, 0, 0, 0, 255],
            ),
            stream(
                "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /DCTDecode",
                b"\xff\xd8fake-jpeg\xff\xd9",
            ),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
                .to_vec(),
            stream("", content),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
        );
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        body
    }

    #[test]
    fn media_helpers_cover_thumbnail_image_objects_originals_and_search() {
        let engine = ContentEngine::open_bytes(media_fixture_pdf()).expect("fixture opens");

        let png = page_thumbnail_png(&engine, 1).unwrap().expect("thumbnail");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

        let (width, height, rgba) =
            render_page_image_object_rgba(&engine, 1, &PageImageObjectSelector::Index(0)).unwrap();
        assert_eq!((width, height), (20, 10));
        assert_eq!(&rgba[..4], &[0, 255, 0, 255]);
        let named = render_page_image_object_png(
            &engine,
            1,
            &PageImageObjectSelector::Name("Rgb".to_string()),
        )
        .unwrap();
        assert!(named.starts_with(b"\x89PNG\r\n\x1a\n"));

        let images = page_image_infos(&engine, 1).unwrap();
        assert_eq!(images.len(), 2);
        let jpg = images
            .iter()
            .find(|image| image.name == "Jpg")
            .expect("DCT image listed");
        let (info, original) = extract_page_image_original(&engine, 1, jpg.index).unwrap();
        assert_eq!(info.extension, "jpg");
        assert!(info.passthrough);
        assert_eq!(info.filter, "DCTDecode");
        assert_eq!(original.bytes, b"\xff\xd8fake-jpeg\xff\xd9");
        let rgb = images.iter().find(|image| image.name == "Rgb").unwrap();
        let (info, _) = extract_page_image_original(&engine, 1, rgb.index).unwrap();
        assert_eq!(info.extension, "png");
        assert!(!info.passthrough);
        assert!(page_image_reference(&engine, 1, 2).is_err());
        assert!(page_image_infos(&engine, 2).is_err());

        let all = search_text_report(&engine, "cat", &TextSearchRequest::default()).unwrap();
        assert_eq!(all.match_count, 2);
        let whole = TextSearchRequest::from_json(Some(r#"{"whole_word":true}"#)).unwrap();
        assert_eq!(
            search_text_report(&engine, "cat", &whole)
                .unwrap()
                .match_count,
            1
        );
        let accents = TextSearchRequest::from_json(Some(r#"{"ignore_accents":true}"#)).unwrap();
        assert_eq!(
            search_text_report(&engine, "cafe", &accents)
                .unwrap()
                .match_count,
            1
        );
        let bad_page = TextSearchRequest::from_json(Some(r#"{"pages":[3]}"#)).unwrap();
        assert!(search_text_report(&engine, "cat", &bad_page).is_err());
        assert!(search_text_report(&engine, "", &TextSearchRequest::default()).is_err());
    }

    #[test]
    fn selector_prefers_name_and_rejects_both() {
        assert_eq!(
            page_image_object_selector(None, Some("/Im1")).unwrap(),
            PageImageObjectSelector::Name("Im1".to_string())
        );
        assert_eq!(
            page_image_object_selector(Some(2), Some("  ")).unwrap(),
            PageImageObjectSelector::Index(2)
        );
        assert_eq!(
            page_image_object_selector(None, None).unwrap(),
            PageImageObjectSelector::Index(0)
        );
        assert!(page_image_object_selector(Some(1), Some("Im1")).is_err());
    }

    #[test]
    fn search_request_parses_pages_and_partial_options() {
        let request =
            TextSearchRequest::from_json(Some(r#"{"pages":[2],"whole_word":true}"#)).unwrap();
        assert_eq!(request.pages, vec![2]);
        assert!(request.options.whole_word);
        assert!(request.options.normalize_unicode);
        let default = TextSearchRequest::from_json(Some("  ")).unwrap();
        assert!(default.pages.is_empty());
        assert!(TextSearchRequest::from_json(Some("[1]")).is_err());
    }
}
