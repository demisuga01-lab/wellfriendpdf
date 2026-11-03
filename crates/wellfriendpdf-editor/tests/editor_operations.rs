//! Editing round trips: edit, save, reopen, and check the text, the fonts
//! and the objects. Fixtures are generated "office-style" documents (subset
//! embedded TrueType fonts, an image, vector shapes), plus real PDFs from
//! `WELLFRIENDPDF_REAL_PDF_DIR` when that is set.

use base64::Engine as _;
use serde_json::json;
use wellfriendpdf_editor::{apply_operations, page_objects, MemoryFonts, OperationList};
use wellfriendpdf_engine::{
    AuthorPageSize, Color, ContentEngine, GraphicsStyle, PageTextLayout, ParagraphStyle,
    PdfBuilder, TextStyle,
};

const SANS: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Regular.ttf");
const SANS_BOLD: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Bold.ttf");
const SERIF: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Regular.ttf");

fn library() -> MemoryFonts {
    let mut fonts = MemoryFonts::new();
    fonts.insert("liberation-sans-regular", SANS.to_vec());
    fonts.insert("liberation-sans-bold", SANS_BOLD.to_vec());
    fonts.insert("liberation-serif-regular", SERIF.to_vec());
    fonts
}

fn rgb_image(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[(x * 255 / width) as u8, (y * 255 / height) as u8, 160]);
        }
    }
    pixels
}

/// A letter-style page: heading, two paragraphs, an image and a box.
fn office_pdf() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    let serif = builder
        .register_font_bytes("Office-Serif", SERIF.to_vec())
        .unwrap();
    let bold = builder
        .register_font_bytes("Office-Bold", SANS_BOLD.to_vec())
        .unwrap();
    let image = builder.add_rgb_image(64, 48, rgb_image(64, 48)).unwrap();
    let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
    page.draw_text("Quarterly Report", 72.0, 720.0, &TextStyle::new(bold, 20.0))
        .unwrap();
    page.draw_paragraph(
        "The committee reviewed the budget and approved the plan for the coming year.",
        72.0,
        680.0,
        460.0,
        &TextStyle::new(serif, 12.0).fill(Color::device_rgb(0.1, 0.1, 0.4)),
        &ParagraphStyle::new(),
    )
    .unwrap();
    page.draw_text(
        "Contact: office@example.com",
        72.0,
        640.0,
        &TextStyle::new(serif, 12.0),
    )
    .unwrap();
    page.draw_image(image, 72.0, 480.0, 128.0, 96.0);
    page.draw_rect(
        300.0,
        480.0,
        120.0,
        80.0,
        &GraphicsStyle::fill(Color::device_rgb(0.9, 0.2, 0.2)),
    );
    without_marked_content(&builder.to_bytes().unwrap())
}

/// Word and Chrome write subset Identity-H TrueType runs without a
/// marked-content /ActualText wrapper per run; the builder adds one. Strip
/// it so the fixture looks like their output.
fn without_marked_content(bytes: &[u8]) -> Vec<u8> {
    let mut document = lopdf::Document::load_mem(bytes).unwrap();
    for page in document.get_pages().into_values() {
        for id in document.get_page_contents(page) {
            let stream = document
                .get_object_mut(id)
                .unwrap()
                .as_stream_mut()
                .unwrap();
            let decoded = stream
                .decompressed_content()
                .unwrap_or_else(|_| stream.content.clone());
            let mut content = lopdf::content::Content::decode(&decoded).unwrap();
            content
                .operations
                .retain(|op| !matches!(op.operator.as_str(), "BDC" | "BMC" | "EMC"));
            stream.set_plain_content(content.encode().unwrap());
        }
    }
    let mut out = Vec::new();
    document.save_to(&mut out).unwrap();
    out
}

/// The builder's own output (with /ActualText spans): a substitute is
/// reported rather than silently claimed as the source font.
fn tagged_office_pdf() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    let serif = builder
        .register_font_bytes("Office-Serif", SERIF.to_vec())
        .unwrap();
    let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
    page.draw_text(
        "The committee reviewed the budget.",
        72.0,
        700.0,
        &TextStyle::new(serif, 12.0),
    )
    .unwrap();
    builder.to_bytes().unwrap()
}

#[test]
fn edits_inside_actual_text_spans_report_their_font() {
    let source = tagged_office_pdf();
    let before = layout(&source, 1);
    let (start, end) = find(&before, "budget");
    let operations = list(
        json!([{ "op": "replaceText", "page": 1, "start": start, "end": end, "expected": "budget",
        "text": "garden", "fallbackFont": "liberation-serif-regular" }]),
    );
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    assert!(layout(&outcome.bytes, 1)
        .text
        .contains("reviewed the garden."));
    let report = &outcome.reports[0];
    if !report.fonts_added.is_empty() {
        assert!(
            !report.notes.is_empty(),
            "a substitution is always explained: {report:?}"
        );
    }
}

fn layout(bytes: &[u8], page: usize) -> PageTextLayout {
    PageTextLayout::from_bytes(bytes, page).unwrap()
}

fn list(operations: serde_json::Value) -> OperationList {
    serde_json::from_value(json!({ "version": 1, "operations": operations })).unwrap()
}

fn find(layout: &PageTextLayout, needle: &str) -> (usize, usize) {
    let text: Vec<char> = layout.text.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    let start = (0..=text.len() - needle.len())
        .find(|&i| text[i..i + needle.len()] == needle[..])
        .unwrap_or_else(|| panic!("{:?} not in {:?}", needle, layout.text));
    (start, start + needle.len())
}

fn fonts_of(layout: &PageTextLayout, range: (usize, usize)) -> Vec<(String, i64)> {
    let mut fonts: Vec<(String, i64)> = layout
        .glyphs
        .iter()
        .filter(|g| g.range[0] >= range.0 && g.range[1] <= range.1 && !g.text.trim().is_empty())
        .map(|g| {
            (
                g.font_resource.clone(),
                (g.font_size * 100.0).round() as i64,
            )
        })
        .collect();
    fonts.dedup();
    fonts
}

#[test]
fn replaces_text_in_place_keeping_size_position_and_typeface() {
    let source = office_pdf();
    let before = layout(&source, 1);
    let (start, end) = find(&before, "budget");
    let first_glyph = before
        .glyphs
        .iter()
        .find(|g| g.range[0] == start)
        .unwrap()
        .clone();
    let source_size = fonts_of(&before, (start, end))[0].1;

    let operations = list(
        json!([{ "op": "replaceText", "page": 1, "start": start, "end": end, "expected": "budget",
        "text": "garden", "fallbackFont": "liberation-serif-regular" }]),
    );
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();

    let after = layout(&outcome.bytes, 1);
    assert!(
        after.text.contains("reviewed the garden and approved"),
        "{}",
        after.text
    );
    assert!(!after.text.contains("budget"));
    let (new_start, new_end) = find(&after, "garden");
    assert_eq!(new_start, start, "text before the edit keeps its offsets");
    let edited_fonts = fonts_of(&after, (new_start, new_end));
    assert!(
        edited_fonts.iter().all(|(_, size)| *size == source_size),
        "same size: {edited_fonts:?}"
    );
    let report = &outcome.reports[0];
    if !report.fonts_added.is_empty() {
        // The engine could not reuse this subset's program; the edit is set
        // in the library font of the same family and the UI is told so.
        let fonts = page_objects(&outcome.bytes, 1).unwrap().fonts;
        let added = fonts
            .iter()
            .find(|f| report.fonts_added.contains(&f.resource))
            .unwrap();
        assert!(added.base_font.ends_with("+LiberationSerif"), "{added:?}");
        assert!(!report.notes.is_empty());
    }
    let new_first = after
        .glyphs
        .iter()
        .find(|g| g.range[0] == new_start)
        .unwrap();
    assert!(
        (new_first.baseline[0][0] - first_glyph.baseline[0][0]).abs() < 0.5,
        "same x"
    );
    assert!(
        (new_first.baseline[0][1] - first_glyph.baseline[0][1]).abs() < 0.5,
        "same baseline"
    );
    assert!(after.text.contains("Quarterly Report"));
    assert!(after.text.contains("Contact: office@example.com"));
    assert_eq!(
        ContentEngine::open_bytes(outcome.bytes)
            .unwrap()
            .page_count()
            .unwrap(),
        1
    );
}

#[test]
fn missing_subset_glyphs_fall_back_to_the_library_font_and_say_so() {
    let source = office_pdf();
    let before = layout(&source, 1);
    let (start, end) = find(&before, "plan");
    // 'Z', 'Q' and 'X' do not occur in the serif body text, so the subset lacks them.
    let operations = list(
        json!([{ "op": "replaceText", "page": 1, "start": start, "end": end, "expected": "plan",
        "text": "ZQX plan", "fallbackFont": "liberation-serif-regular" }]),
    );
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    let after = layout(&outcome.bytes, 1);
    assert!(after.text.contains("ZQX plan"), "{}", after.text);
    let report = &outcome.reports[0];
    assert!(!report.fonts_added.is_empty(), "{report:?}");
    assert!(
        !report.notes.is_empty(),
        "the substitution is reported: {report:?}"
    );
}

#[test]
fn image_objects_are_listed_moved_resized_and_deleted() {
    let source = office_pdf();
    let objects = page_objects(&source, 1).unwrap();
    assert_eq!(objects.images.len(), 1, "{objects:?}");
    let image = &objects.images[0];
    assert!((image.bounds[0] - 72.0).abs() < 0.01 && (image.bounds[1] - 480.0).abs() < 0.01);

    let selector = serde_json::to_value(&image.placement.selector).unwrap();
    let moved = list(json!([{ "op": "transformImage", "selector": selector,
        "matrix": { "a": 256.0, "b": 0.0, "c": 0.0, "d": 192.0, "e": 100.0, "f": 300.0 } }]));
    let outcome = apply_operations(&source, &moved, &library(), false).unwrap();
    let after = page_objects(&outcome.bytes, 1).unwrap();
    assert_eq!(after.images.len(), 1);
    assert_eq!(
        after.images[0].bounds.map(|v| v.round()),
        [100.0, 300.0, 356.0, 492.0]
    );

    let deleted = list(json!([{ "op": "deleteImage", "selector": selector }]));
    let outcome = apply_operations(&source, &deleted, &library(), false).unwrap();
    assert!(page_objects(&outcome.bytes, 1).unwrap().images.is_empty());
    assert!(layout(&outcome.bytes, 1).text.contains("Quarterly Report"));
}

#[test]
fn vector_shapes_are_recoloured() {
    let source = office_pdf();
    let objects = page_objects(&source, 1).unwrap();
    let path = objects
        .vectors
        .iter()
        .find(|v| v.bounds.right - v.bounds.left > 100.0 && v.supports_fill)
        .unwrap_or_else(|| panic!("red box not editable: {:?}", objects.vectors));
    let operations = list(json!([{ "op": "editVector", "selector": path.selector,
        "mutation": { "kind": "RECOLOR", "destination_fill_color": { "red": 0.1, "green": 0.6, "blue": 0.2 }, "destination_stroke_color": null } }]));
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    let after = page_objects(&outcome.bytes, 1).unwrap();
    let recoloured = after
        .vectors
        .iter()
        .find(|v| v.bounds.right - v.bounds.left > 100.0)
        .unwrap();
    let fill = recoloured.fill_color.unwrap();
    assert!((fill.green - 0.6).abs() < 0.01, "{fill:?}");
}

#[test]
fn added_text_boxes_embed_the_library_font_as_real_text() {
    let source = office_pdf();
    let operations = list(json!([
        { "op": "addText", "page": 1, "rect": { "x": 72.0, "y": 380.0, "width": 300.0, "height": 60.0 },
          "text": "Approved by Asha Rao\nFinance lead", "font": "liberation-sans-bold", "size": 14.0,
          "color": { "r": 0.0, "g": 0.2, "b": 0.6 } },
        { "op": "addShape", "page": 1, "shape": "ellipse", "rect": { "x": 400.0, "y": 380.0, "width": 80.0, "height": 40.0 },
          "stroke": { "r": 0.0, "g": 0.0, "b": 0.0 }, "lineWidth": 2.0 },
        { "op": "addLine", "page": 1, "from": [72.0, 370.0], "to": [540.0, 370.0], "color": { "r": 0.5, "g": 0.5, "b": 0.5 } },
        { "op": "addWhiteout", "page": 1, "rect": { "x": 500.0, "y": 700.0, "width": 40.0, "height": 40.0 } },
        { "op": "addLink", "page": 1, "rect": { "x": 72.0, "y": 636.0, "width": 200.0, "height": 16.0 }, "url": "mailto:office@example.com" },
        { "op": "addImage", "page": 1, "rect": { "x": 450.0, "y": 600.0, "width": 60.0, "height": 60.0 },
          "image": { "mimeType": "image/png", "base64": base64::engine::general_purpose::STANDARD.encode(tiny_png()) } }
    ]));
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    let after = layout(&outcome.bytes, 1);
    assert!(
        after.text.contains("Approved by Asha Rao"),
        "{}",
        after.text
    );
    assert!(after.text.contains("Finance lead"));
    let fonts = page_objects(&outcome.bytes, 1).unwrap().fonts;
    assert!(
        fonts
            .iter()
            .any(|f| f.embedded && f.base_font.ends_with("+LiberationSans-Bold")),
        "the overlay embeds the library font: {fonts:?}"
    );
    // The original text is still the original text.
    assert!(after.text.contains("The committee reviewed the budget"));
    let pdf = lopdf::Document::load_mem(&outcome.bytes).unwrap();
    let page = pdf.get_pages()[&1];
    let annots = pdf
        .get_dictionary(page)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .len();
    assert_eq!(annots, 1);
}

fn tiny_png() -> Vec<u8> {
    let raw = wellfriendpdf_engine::RawImage {
        width: 4,
        height: 4,
        channels: 4,
        bits_per_sample: 8,
        pixels: [20u8, 120, 200, 255].repeat(16),
    };
    wellfriendpdf_engine::ImageEncoder::encode_png(&raw).unwrap()
}

#[test]
fn redaction_removes_the_text_not_just_covers_it() {
    let source = office_pdf();
    let before = layout(&source, 1);
    let (start, end) = find(&before, "office@example.com");
    let glyphs: Vec<_> = before
        .glyphs
        .iter()
        .filter(|g| g.range[0] >= start && g.range[1] <= end)
        .collect();
    let x0 = glyphs
        .iter()
        .map(|g| g.bbox[0])
        .fold(f64::INFINITY, f64::min);
    let y0 = glyphs
        .iter()
        .map(|g| g.bbox[1])
        .fold(f64::INFINITY, f64::min);
    let x1 = glyphs
        .iter()
        .map(|g| g.bbox[2])
        .fold(f64::NEG_INFINITY, f64::max);
    let y1 = glyphs
        .iter()
        .map(|g| g.bbox[3])
        .fold(f64::NEG_INFINITY, f64::max);
    let operations = list(json!([{ "op": "redact", "page": 1,
        "rect": { "x": x0 - 1.0, "y": y0 - 1.0, "width": x1 - x0 + 2.0, "height": y1 - y0 + 2.0 } }]));
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    let after = layout(&outcome.bytes, 1);
    assert!(!after.text.contains("office@example.com"), "{}", after.text);
    assert!(!String::from_utf8_lossy(&outcome.bytes).contains("office@example.com"));
    assert!(after.text.contains("Quarterly Report"));
    assert!(outcome.reports[0].redacted_glyphs.unwrap_or(0) >= 18);
}

#[test]
fn the_admission_rule_rejects_what_is_not_allowed() {
    let bad = [
        json!({ "version": 2, "operations": [] }),
        json!({ "version": 1, "operations": [{ "op": "addLink", "page": 1, "rect": { "x": 0, "y": 0, "width": 10, "height": 10 }, "url": "javascript:alert(1)" }] }),
        json!({ "version": 1, "operations": [{ "op": "addText", "page": 1, "rect": { "x": 0, "y": 0, "width": 10, "height": 10 }, "text": "x", "font": "../etc/passwd", "size": 12, "color": { "r": 0, "g": 0, "b": 0 } }] }),
        json!({ "version": 1, "operations": [{ "op": "addShape", "page": 0, "shape": "rectangle", "rect": { "x": 0, "y": 0, "width": 10, "height": 10 }, "fill": { "r": 0, "g": 0, "b": 0 } }] }),
        json!({ "version": 1, "operations": [{ "op": "addWhiteout", "page": 1, "rect": { "x": 1e9, "y": 0, "width": 10, "height": 10 } }] }),
        json!({ "version": 1, "operations": [{ "op": "replaceText", "page": 1, "start": 0, "end": 3, "expected": "ab", "text": "x" }] }),
        json!({ "version": 1, "operations": [{ "op": "addImage", "page": 1, "rect": { "x": 0, "y": 0, "width": 10, "height": 10 }, "image": { "mimeType": "image/png", "base64": "aGVsbG8=" } }] }),
        json!({ "version": 1, "operations": [{ "op": "unknownThing", "page": 1 }] }),
        json!({ "version": 1, "operations": [], "extra": true }),
    ];
    for case in bad {
        assert!(
            OperationList::parse(&case.to_string()).is_err(),
            "accepted {case}"
        );
    }
    // A stale selection is refused at apply time.
    let source = office_pdf();
    let stale = list(
        json!([{ "op": "replaceText", "page": 1, "start": 0, "end": 3, "expected": "Xyz", "text": "abc" }]),
    );
    let error = apply_operations(&source, &stale, &library(), false).unwrap_err();
    assert_eq!(error.code, "EDITOR_TEXT_STALE");
    assert_eq!(error.operation, Some(0));
}

#[test]
fn operations_replay_deterministically() {
    let source = office_pdf();
    let before = layout(&source, 1);
    let (start, end) = find(&before, "Quarterly");
    let operations = list(json!([
        { "op": "replaceText", "page": 1, "start": start, "end": end, "expected": "Quarterly", "text": "Annual" },
        { "op": "addWhiteout", "page": 1, "rect": { "x": 500.0, "y": 700.0, "width": 40.0, "height": 40.0 } }
    ]));
    let first = apply_operations(&source, &operations, &library(), false).unwrap();
    let second = apply_operations(&source, &operations, &library(), false).unwrap();
    assert_eq!(
        first.bytes, second.bytes,
        "the browser preview and the server replay must agree"
    );
}

/// Real PDFs: edit the first editable word on the first text page, save,
/// reopen and compare text and fonts. Set `WELLFRIENDPDF_REAL_PDF_DIR`.
#[test]
fn real_pdf_round_trips() {
    let Ok(dir) = std::env::var("WELLFRIENDPDF_REAL_PDF_DIR") else {
        eprintln!("WELLFRIENDPDF_REAL_PDF_DIR unset; skipping real PDF round trips");
        return;
    };
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
        .collect();
    entries.sort();
    let (mut edited, mut preserved, mut substituted) = (0, 0, 0);
    for path in entries {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(engine) = ContentEngine::open_bytes(bytes.clone()) else {
            continue;
        };
        let pages = engine.page_count().unwrap_or(0).min(6);
        let mut done = false;
        for page in 1..=pages {
            let Ok(before) = PageTextLayout::build(&engine, page) else {
                continue;
            };
            if !before.editable {
                continue;
            }
            // First word of 4+ ASCII letters inside one line, in one font.
            let chars: Vec<char> = before.text.chars().collect();
            let mut found = None;
            'lines: for line in &before.lines {
                let mut i = line.range[0];
                while i < line.range[1] {
                    if chars[i].is_ascii_alphabetic() {
                        let s = i;
                        while i < line.range[1] && chars[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        if i - s >= 4 && fonts_of(&before, (s, i)).len() == 1 {
                            found = Some((s, i));
                            break 'lines;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            let Some((start, end)) = found else { continue };
            let word: String = chars[start..end].iter().collect();
            // Drop the last letter: every glyph left is already in the subset.
            let replacement: String = word.chars().take(word.chars().count() - 1).collect();
            let operations = list(
                json!([{ "op": "replaceText", "page": page, "start": start, "end": end, "expected": word, "text": replacement }]),
            );
            match apply_operations(&bytes, &operations, &library(), true) {
                Ok(outcome) => {
                    let after = PageTextLayout::from_bytes(&outcome.bytes, page).unwrap();
                    assert!(
                        after.text.contains(&replacement),
                        "{}: edited text present",
                        path.display()
                    );
                    let report = &outcome.reports[0];
                    let fonts_before = fonts_of(&before, (start, end));
                    let (s2, e2) = find(&after, &replacement);
                    let fonts_after = fonts_of(&after, (s2, e2));
                    if report.fonts_added.is_empty() {
                        assert_eq!(
                            fonts_after,
                            fonts_before,
                            "{}: fonts preserved",
                            path.display()
                        );
                        preserved += 1;
                    } else {
                        // The source program could not be reused: the
                        // substitute is named and explained, never silent.
                        assert!(
                            !report.notes.is_empty(),
                            "{}: substitution explained",
                            path.display()
                        );
                        assert!(
                            fonts_after
                                .iter()
                                .all(|(_, size)| fonts_before.iter().any(|(_, s)| s == size)),
                            "{}: size kept",
                            path.display()
                        );
                        substituted += 1;
                    }
                    // Reopen: the saved bytes parse and keep their page count.
                    let reopened = ContentEngine::open_bytes(outcome.bytes.clone()).unwrap();
                    assert_eq!(reopened.page_count().unwrap(), engine.page_count().unwrap());
                    eprintln!(
                        "{}: page {page}: {word:?} -> {replacement:?} ok, fonts {:?} -> {:?}",
                        path.display(),
                        fonts_before,
                        fonts_after
                    );
                    edited += 1;
                    done = true;
                }
                Err(error) => eprintln!("{}: page {page}: refused: {error}", path.display()),
            }
            break;
        }
        if !done {
            eprintln!("{}: no editable text edit", path.display());
        }
    }
    eprintln!("real PDFs: {edited} edited, {preserved} kept the source font, {substituted} used a reported substitute");
    assert!(preserved > 0, "no real PDF kept its source font");
    assert!(edited > 0, "no real PDF could be edited");
}

#[test]
#[ignore]
fn write_office_fixture() {
    std::fs::write("/srv/editor-work/office.pdf", office_pdf()).unwrap();
}
