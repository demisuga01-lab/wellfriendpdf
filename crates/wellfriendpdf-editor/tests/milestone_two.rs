//! Milestone 2 operations: restyling existing text, paragraph detection,
//! page structure, annotations, image crop/opacity/PNG replacement, form
//! filling and CMYK/spot colours. Each test edits, saves and reopens.

use base64::Engine as _;
use serde_json::json;
use wellfriendpdf_editor::{apply_operations, page_objects, MemoryFonts, OperationList};
use wellfriendpdf_engine::{
    AuthorPageSize, Color, ContentEngine, GraphicsStyle, PageTextLayout, PdfBuilder, TextStyle,
};

const SANS: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Regular.ttf");
const SANS_BOLD: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Bold.ttf");
const SERIF: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Regular.ttf");
const SERIF_BOLD: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Bold.ttf");
const FORM: &[u8] = include_bytes!("../../../tests/corpus/pdfs/pdfjs/form_two_pages.pdf");
const WIKIPEDIA: &[u8] = include_bytes!("../../../tests/corpus/pdfs/pdfjs/pdfjs_wikipedia.pdf");

fn library() -> MemoryFonts {
    let mut fonts = MemoryFonts::new();
    fonts.insert("liberation-sans-regular", SANS.to_vec());
    fonts.insert("liberation-sans-bold", SANS_BOLD.to_vec());
    fonts.insert("liberation-serif-regular", SERIF.to_vec());
    fonts.insert("liberation-serif-bold", SERIF_BOLD.to_vec());
    fonts
}

fn list(operations: serde_json::Value) -> OperationList {
    serde_json::from_value(json!({ "version": 1, "operations": operations })).unwrap()
}

fn layout(bytes: &[u8], page: usize) -> PageTextLayout {
    PageTextLayout::from_bytes(bytes, page).unwrap()
}

fn find(layout: &PageTextLayout, needle: &str) -> (usize, usize) {
    let text: Vec<char> = layout.text.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    let start = (0..=text.len() - needle.len())
        .find(|&i| text[i..i + needle.len()] == needle[..])
        .unwrap_or_else(|| panic!("{needle:?} not in {:?}", layout.text));
    (start, start + needle.len())
}

/// A heading close above two body lines, and a two-cell row.
fn report_pdf(pages: usize) -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    let serif = builder.register_font_bytes("Body-Serif", SERIF.to_vec()).unwrap();
    let bold = builder.register_font_bytes("Head-Bold", SANS_BOLD.to_vec()).unwrap();
    for index in 0..pages {
        let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
        page.draw_text(format!("Annual Summary {}", index + 1), 72.0, 720.0, &TextStyle::new(bold, 18.0)).unwrap();
        page.draw_text("Revenue grew in every region this year.", 72.0, 700.0, &TextStyle::new(serif, 12.0)).unwrap();
        page.draw_text("Costs fell slightly over the same period.", 72.0, 686.0, &TextStyle::new(serif, 12.0)).unwrap();
        page.draw_text("Region", 72.0, 600.0, &TextStyle::new(serif, 12.0)).unwrap();
        page.draw_text("North", 260.0, 600.0, &TextStyle::new(serif, 12.0)).unwrap();
        page.draw_rect(72.0, 420.0, 120.0, 60.0, &GraphicsStyle::fill(Color::device_rgb(0.2, 0.4, 0.8)));
    }
    builder.to_bytes().unwrap()
}

fn png(r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
    let raw = wellfriendpdf_engine::RawImage {
        width: 8,
        height: 8,
        channels: 4,
        bits_per_sample: 8,
        pixels: [r, g, b, a].repeat(64),
    };
    wellfriendpdf_engine::ImageEncoder::encode_png(&raw).unwrap()
}

fn with_image() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    let image = builder.add_rgb_image(4, 4, [200u8, 40, 40].repeat(16)).unwrap();
    let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
    page.draw_image(image, 100.0, 500.0, 200.0, 150.0);
    builder.to_bytes().unwrap()
}

#[test]
fn headings_and_table_cells_are_their_own_paragraphs() {
    let source = report_pdf(1);
    let layout = layout(&source, 1);
    let paragraph_text = |i: usize| -> String {
        let p = &layout.paragraphs[i];
        layout.text.chars().skip(p.range[0]).take(p.range[1] - p.range[0]).collect()
    };
    let texts: Vec<String> = (0..layout.paragraphs.len()).map(paragraph_text).collect();
    assert!(texts.iter().any(|t| t == "Annual Summary 1"), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("Revenue grew") && t.contains("Costs fell")), "body lines join: {texts:?}");
    assert!(texts.iter().any(|t| t == "Region") && texts.iter().any(|t| t == "North"), "cells split: {texts:?}");
}

#[test]
fn existing_text_is_restyled_in_place() {
    // A Chrome-printed page: paragraphs set in embedded Georgia subsets.
    let source = WIKIPEDIA;
    let before = layout(source, 1);
    let paragraph = before
        .paragraphs
        .iter()
        .find(|p| before.text_in(p.range).starts_with("The project was created"))
        .unwrap();
    let (start, end) = (paragraph.range[0], paragraph.range[1]);
    let expected = before.text_in(paragraph.range);

    // Restyling applies to whole paragraphs.
    let partial = list(json!([{ "op": "restyleText", "page": 1, "start": start, "end": start + 7,
        "expected": before.text_in([start, start + 7]), "font": "liberation-serif-bold", "size": 12.0 }]));
    assert_eq!(apply_operations(source, &partial, &library(), false).unwrap_err().code, "EDITOR_RESTYLE_NEEDS_PARAGRAPH");

    let operations = list(json!([{ "op": "restyleText", "page": 1, "start": start, "end": end, "expected": expected,
        "font": "liberation-serif-bold", "size": 12.0, "color": { "r": 0.8, "g": 0.1, "b": 0.1 } }]));
    let outcome = apply_operations(source, &operations, &library(), false).unwrap();
    let after = layout(&outcome.bytes, 1);
    // Tagged (marked-content) text is re-set as new text: the words read the
    // same, with the line breaks the reader saw as spaces.
    let reading = "The project was created to provide a way for viewing PDF documents natively";
    assert!(after.text.contains(reading), "restyling keeps the text: {}", after.text);
    assert!(after.text.contains("History and application"), "other text untouched");
    let (s, e) = find(&after, reading);
    let sizes: Vec<f64> = after.glyphs.iter().filter(|g| g.range[0] >= s && g.range[1] <= e && !g.text.trim().is_empty()).map(|g| g.font_size).collect();
    assert!(sizes.iter().all(|size| (size - 12.0).abs() < 0.2), "size: {sizes:?}");
    let report = &outcome.reports[0];
    assert!(!report.notes.is_empty(), "{report:?}");
    let fonts = page_objects(&outcome.bytes, 1).unwrap().fonts;
    assert!(fonts.iter().any(|f| f.base_font.ends_with("LiberationSerif-Bold")), "{fonts:?}");
    let pdf = lopdf::Document::load_mem(&outcome.bytes).unwrap();
    let content = String::from_utf8_lossy(&pdf.get_page_content(pdf.get_pages()[&1])).to_string();
    assert!(content.contains("0.8 0.1 0.1 rg"), "colour set");
    // Restyling is per paragraph: part of a line is refused, never half-done.
    let (ws, we) = find(&before, "sandboxed");
    let inline = list(json!([{ "op": "restyleText", "page": 1, "start": ws, "end": we, "expected": "sandboxed",
        "font": "liberation-serif-bold", "color": { "c": 0.0, "m": 0.0, "y": 0.0, "k": 1.0 } }]));
    assert_eq!(apply_operations(source, &inline, &library(), false).unwrap_err().code, "EDITOR_RESTYLE_NEEDS_PARAGRAPH");
}

#[test]
fn pages_are_rotated_moved_deleted_and_inserted() {
    let source = report_pdf(3);
    let other = report_pdf(2);
    let operations = list(json!([
        { "op": "rotatePage", "page": 2, "rotation": 90 },
        { "op": "movePage", "page": 3, "to": 1 },
        { "op": "insertBlankPage", "after": 3, "width": 595.0, "height": 842.0 },
        { "op": "insertPages", "after": 1, "pdf": base64::engine::general_purpose::STANDARD.encode(&other), "pages": [2] },
        { "op": "deletePage", "page": 5 }
    ]));
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    let engine = ContentEngine::open_bytes(outcome.bytes.clone()).unwrap();
    // 3 pages, moved 3 to the front, a blank after page 3 (4 pages),
    // other's page 2 after page 1 (5), page 5 (the blank) deleted.
    assert_eq!(engine.page_count().unwrap(), 4);
    let titles: Vec<String> = (1..=4)
        .map(|p| {
            let text = layout(&outcome.bytes, p).text;
            text.split("Revenue").next().unwrap_or("").to_owned()
        })
        .collect();
    assert_eq!(titles, ["Annual Summary 3", "Annual Summary 2", "Annual Summary 1", "Annual Summary 2"]);
    // The rotated page kept its rotation through the reorder.
    let pdf = lopdf::Document::load_mem(&outcome.bytes).unwrap();
    let rotated = pdf.get_pages()[&4];
    assert_eq!(pdf.get_dictionary(rotated).unwrap().get(b"Rotate").unwrap().as_i64().unwrap(), 90);
}

#[test]
fn annotations_carry_appearances() {
    let source = report_pdf(1);
    let before = layout(&source, 1);
    let (start, end) = find(&before, "Costs fell");
    let boxes: Vec<[f64; 4]> = before.glyphs.iter().filter(|g| g.range[0] >= start && g.range[1] <= end).map(|g| g.bbox).collect();
    let quad = [
        boxes.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min),
        boxes.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
        boxes.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max),
        boxes.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max),
    ];
    let yellow = json!({ "r": 1.0, "g": 0.85, "b": 0.0 });
    let operations = list(json!([
        { "op": "addAnnotation", "page": 1, "kind": "highlight", "rect": { "x": quad[0], "y": quad[1], "width": quad[2] - quad[0], "height": quad[3] - quad[1] }, "quads": [quad], "color": yellow, "contents": "Check this" },
        { "op": "addAnnotation", "page": 1, "kind": "underline", "rect": { "x": quad[0], "y": quad[1], "width": 10.0, "height": 10.0 }, "quads": [quad], "color": { "r": 0.0, "g": 0.4, "b": 1.0 } },
        { "op": "addAnnotation", "page": 1, "kind": "strikeout", "rect": { "x": quad[0], "y": quad[1], "width": 10.0, "height": 10.0 }, "quads": [quad], "color": { "r": 1.0, "g": 0.0, "b": 0.0 } },
        { "op": "addAnnotation", "page": 1, "kind": "note", "rect": { "x": 400.0, "y": 700.0, "width": 20.0, "height": 20.0 }, "color": yellow, "contents": "Reviewed", "author": "Asha Rao" },
        { "op": "addAnnotation", "page": 1, "kind": "ink", "rect": { "x": 300.0, "y": 300.0, "width": 10.0, "height": 10.0 }, "ink": [[[300.0, 300.0], [340.0, 330.0], [380.0, 300.0]]], "color": { "r": 0.0, "g": 0.0, "b": 0.0 } },
        { "op": "addAnnotation", "page": 1, "kind": "stamp", "rect": { "x": 380.0, "y": 500.0, "width": 160.0, "height": 50.0 }, "label": "Approved", "color": { "r": 0.0, "g": 0.5, "b": 0.2 } }
    ]));
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    let pdf = lopdf::Document::load_mem(&outcome.bytes).unwrap();
    let page = pdf.get_pages()[&1];
    let annots = pdf.get_dictionary(page).unwrap().get(b"Annots").unwrap().as_array().unwrap().clone();
    assert_eq!(annots.len(), 6);
    let subtypes: Vec<String> = annots
        .iter()
        .map(|a| {
            let d = pdf.get_dictionary(a.as_reference().unwrap()).unwrap();
            assert!(d.has(b"AP"), "every annotation has an appearance");
            String::from_utf8_lossy(d.get(b"Subtype").unwrap().as_name().unwrap()).into_owned()
        })
        .collect();
    assert_eq!(subtypes, ["Highlight", "Underline", "StrikeOut", "Text", "Ink", "Stamp"]);
    // Text is untouched: annotations are not content.
    assert_eq!(layout(&outcome.bytes, 1).text, before.text);
    let again = apply_operations(&source, &operations, &library(), false).unwrap();
    assert_eq!(again.bytes, outcome.bytes, "deterministic");
    // The engine draws the appearances: inside the stamp border is green.
    let raw = ContentEngine::open_bytes(outcome.bytes.clone()).unwrap().render_page(1, 72).unwrap().into_raw_image();
    let at = |x: f64, y: f64| {
        let i = ((792.0 - y) as usize * raw.width as usize + x as usize) * raw.channels as usize;
        (raw.pixels[i], raw.pixels[i + 1], raw.pixels[i + 2])
    };
    let stamp_edge = at(381.5, 525.0);
    assert!(stamp_edge.1 > stamp_edge.0 && stamp_edge.1 > stamp_edge.2, "stamp border drawn: {stamp_edge:?}");
}

#[test]
fn images_are_cropped_faded_and_replaced_with_png() {
    let source = with_image();
    let objects = page_objects(&source, 1).unwrap();
    let image = &objects.images[0];
    let selector = serde_json::to_value(&image.placement.selector).unwrap();
    let crop = list(json!([{ "op": "cropImage", "selector": selector, "crop": { "left": 0.25, "bottom": 0.0, "right": 1.0, "top": 0.75 } }]));
    let cropped = apply_operations(&source, &crop, &library(), false).unwrap();
    assert!(page_objects(&cropped.bytes, 1).unwrap().images[0].placement.crop.is_some());

    let fade = list(json!([{ "op": "setImageOpacity", "selector": selector, "opacity": 0.4 }]));
    let faded = apply_operations(&source, &fade, &library(), false).unwrap();
    assert!((page_objects(&faded.bytes, 1).unwrap().images[0].placement.opacity - 0.4).abs() < 0.01);

    let replacement = image.image.as_ref().unwrap().selector.clone();
    let swap = list(json!([{ "op": "replaceImage", "selector": replacement,
        "image": { "mimeType": "image/png", "base64": base64::engine::general_purpose::STANDARD.encode(png(10, 160, 60, 128)) } }]));
    let swapped = apply_operations(&source, &swap, &library(), false).unwrap();
    let pdf = lopdf::Document::load_mem(&swapped.bytes).unwrap();
    let id = (replacement.image_object.number, replacement.image_object.generation);
    let stream = pdf.get_object(id).unwrap().as_stream().unwrap();
    assert_eq!(stream.dict.get(b"Width").unwrap().as_i64().unwrap(), 8);
    assert!(stream.dict.has(b"SMask"), "PNG alpha is kept as a soft mask");
    // The unchanged placement now paints the new, half-transparent green.
    let raw = ContentEngine::open_bytes(swapped.bytes.clone()).unwrap().render_page(1, 72).unwrap().into_raw_image();
    let at = |x: usize, y: usize| {
        let i = (y * raw.width as usize + x) * raw.channels as usize;
        (raw.pixels[i], raw.pixels[i + 1], raw.pixels[i + 2])
    };
    let (r, g, b) = at(200, 792 - 575);
    assert!(g > r.saturating_add(40) && g > b, "green over white: {:?}", (r, g, b));
}

#[test]
fn form_fields_are_filled_with_appearances() {
    let objects = page_objects(FORM, 1).unwrap();
    let field = objects
        .forms
        .iter()
        .find(|f| format!("{:?}", f.kind).to_lowercase().contains("text") && f.value_editable)
        .unwrap_or_else(|| panic!("no editable text field: {:?}", objects.forms.iter().map(|f| (&f.name, &f.kind)).collect::<Vec<_>>()));
    let operations = list(json!([{ "op": "setFormField", "name": field.name, "value": "Asha Rao" }]));
    let outcome = apply_operations(FORM, &operations, &library(), true).unwrap();
    let after = page_objects(&outcome.bytes, 1).unwrap();
    let filled = after.forms.iter().find(|f| f.name == field.name).unwrap();
    assert_eq!(filled.value.as_deref(), Some("Asha Rao"));
}

#[test]
fn cmyk_and_spot_colours_are_written_as_such() {
    let source = report_pdf(1);
    let operations = list(json!([
        { "op": "addShape", "page": 1, "shape": "rectangle", "rect": { "x": 300.0, "y": 300.0, "width": 80.0, "height": 40.0 },
          "fill": { "c": 0.0, "m": 0.9, "y": 0.8, "k": 0.1 } },
        { "op": "addShape", "page": 1, "shape": "rectangle", "rect": { "x": 400.0, "y": 300.0, "width": 80.0, "height": 40.0 },
          "fill": { "spot": "PANTONE 186 C", "tint": 0.5, "c": 0.0, "m": 1.0, "y": 0.81, "k": 0.04 } },
        { "op": "addText", "page": 1, "rect": { "x": 72.0, "y": 200.0, "width": 300.0, "height": 30.0 }, "text": "Spot heading",
          "font": "liberation-sans-bold", "size": 16.0, "color": { "spot": "Brand Green", "c": 0.8, "m": 0.0, "y": 0.9, "k": 0.1 } }
    ]));
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    let pdf = lopdf::Document::load_mem(&outcome.bytes).unwrap();
    let page = pdf.get_pages()[&1];
    let content = String::from_utf8_lossy(&pdf.get_page_content(page)).to_string();
    assert!(content.contains("0 0.9 0.8 0.1 k"), "CMYK fill: {content}");
    assert!(content.contains(" scn"), "spot fill uses its separation");
    let separations: Vec<String> = pdf
        .objects
        .values()
        .filter_map(|o| o.as_array().ok())
        .filter(|a| a.first().and_then(|n| n.as_name().ok()) == Some(b"Separation".as_slice()))
        .map(|a| String::from_utf8_lossy(a[1].as_name().unwrap()).into_owned())
        .collect();
    assert!(separations.iter().any(|n| n.contains("PANTONE")), "{separations:?}");
    assert!(separations.iter().any(|n| n.contains("Brand")), "{separations:?}");
    assert!(layout(&outcome.bytes, 1).text.contains("Spot heading"));
}
