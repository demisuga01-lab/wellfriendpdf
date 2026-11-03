//! Milestone 3: restyled paragraphs push the text below them in their
//! column, text inside form XObjects is editable, and edited text reuses
//! the glyphs already embedded in the source font.

use serde_json::json;
use wellfriendpdf_editor::{apply_operations, MemoryFonts, OperationList};
use wellfriendpdf_engine::PageTextLayout;

const GELASIO_BOLD: &[u8] = include_bytes!("fonts/Gelasio-Bold.ttf");
const SANS: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Regular.ttf");
const WIKIPEDIA: &[u8] = include_bytes!("../../../tests/corpus/pdfs/pdfjs/pdfjs_wikipedia.pdf");
const TWO_COLUMNS: &[u8] = include_bytes!("../../../tests/corpus/pdfs/generated/generated_two_columns.pdf");

fn library() -> MemoryFonts {
    let mut fonts = MemoryFonts::new();
    fonts.insert("gelasio-bold", GELASIO_BOLD.to_vec());
    fonts.insert("liberation-sans-regular", SANS.to_vec());
    fonts
}

fn list(operations: serde_json::Value) -> OperationList {
    serde_json::from_value(json!({ "version": 1, "operations": operations })).unwrap()
}

fn layout(bytes: &[u8], page: usize) -> PageTextLayout {
    PageTextLayout::from_bytes(bytes, page).unwrap()
}

/// The paragraph whose text starts with `prefix`.
fn paragraph(layout: &PageTextLayout, prefix: &str) -> usize {
    layout
        .paragraphs
        .iter()
        .position(|p| layout.text_in(p.range).starts_with(prefix))
        .unwrap_or_else(|| panic!("no paragraph starts with {prefix:?}"))
}

fn restyle(source: &[u8], prefix: &str, size: f64) -> wellfriendpdf_editor::ApplyOutcome {
    let before = layout(source, 1);
    let p = &before.paragraphs[paragraph(&before, prefix)];
    let operations = list(json!([{ "op": "restyleText", "page": 1, "start": p.range[0], "end": p.range[1],
        "expected": before.text_in(p.range), "font": "gelasio-bold", "size": size }]));
    let outcome = apply_operations(source, &operations, &library(), false).unwrap();
    keep(&format!("restyle-{}", prefix.replace(' ', "-")), &outcome.bytes);
    outcome
}

/// With `WF_EDITOR_TEST_OUT` set, write `bytes` there for a visual check.
fn keep(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("WF_EDITOR_TEST_OUT") {
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.pdf")), bytes).unwrap();
    }
}

fn intersects(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] - 0.01 && b[0] < a[2] - 0.01 && a[1] < b[3] - 0.01 && b[1] < a[3] - 0.01
}

/// Every visible glyph of paragraph `index` that intersects a visible glyph
/// of another paragraph.
fn collisions(layout: &PageTextLayout, index: usize) -> Vec<(String, String)> {
    let visible = |g: &&wellfriendpdf_engine::EditorGlyph| !g.text.trim().is_empty() && !g.invisible;
    let lines = layout.paragraphs[index].lines;
    let mine: Vec<_> = layout
        .glyphs
        .iter()
        .filter(visible)
        .filter(|g| g.line >= lines[0] && g.line < lines[1])
        .collect();
    let mut hits = Vec::new();
    for other in layout.glyphs.iter().filter(visible) {
        if other.line >= lines[0] && other.line < lines[1] {
            continue;
        }
        for glyph in &mine {
            if intersects(glyph.bbox, other.bbox) {
                hits.push((glyph.text.clone(), other.text.clone()));
            }
        }
    }
    hits
}

#[test]
fn a_restyled_paragraph_pushes_the_text_below_it_in_its_column() {
    // Chrome-printed and tagged: the paragraph is re-set as new text.
    let before = layout(WIKIPEDIA, 1);
    let following = before.paragraphs[paragraph(&before, "PDF.js is used in Thunderbird")].bbox;
    let infobox = before.paragraphs[paragraph(&before, "Developer(s)")].bbox;
    let outcome = restyle(WIKIPEDIA, "The project was created to provide", 13.0);
    let after = layout(&outcome.bytes, 1);
    let restyled = paragraph(&after, "The project was created to provide");
    let hits = collisions(&after, restyled);
    assert!(hits.is_empty(), "restyled glyphs overlap other text: {hits:?}");
    // It grew and the next paragraph of its column moved down by as much.
    let grown = after.paragraphs[restyled].bbox;
    let original = before.paragraphs[paragraph(&before, "The project was created to provide")].bbox;
    assert!(grown[1] < original[1] - 20.0, "the paragraph grew: {grown:?} from {original:?}");
    let moved = after.paragraphs[paragraph(&after, "PDF.js is used in Thunderbird")].bbox;
    assert!(moved[3] < following[3] - 20.0, "following paragraph moved down: {moved:?} from {following:?}");
    assert!((moved[2] - following[2]).abs() < 0.5, "only moved vertically");
    // The gap above the moved paragraph is the source's.
    assert!(((original[1] - following[3]) - (grown[1] - moved[3])).abs() < 0.5);
    // The other column (the infobox) stays put.
    let infobox_after = after.paragraphs[paragraph(&after, "Developer(s)")].bbox;
    assert!(infobox.iter().zip(infobox_after).all(|(a, b)| (a - b).abs() < 0.01), "{infobox:?} {infobox_after:?}");
    // Every paragraph of the page still reads the same.
    for p in &before.paragraphs {
        let text = before.text_in(p.range);
        if !text.starts_with("The project was created") {
            assert!(after.text.contains(&text), "{text:?} kept");
        }
    }
    // Nothing moved out of Chrome's printable-area clip (28 pt margins):
    // the column runs out of room, so the restyled paragraph keeps its
    // size, ends where there is room and the report says what did not fit.
    let lines = after.paragraphs[restyled].lines;
    for glyph in after.glyphs.iter().filter(|g| g.line < lines[0] || g.line >= lines[1]) {
        assert!(glyph.bbox[1] >= 28.0 - 0.01, "{glyph:?} stays inside the page clip");
    }
    let notes = &outcome.reports[0].notes;
    assert!(notes.iter().any(|n| n.contains("moved down")), "{notes:?}");
    assert!(notes.iter().any(|n| n.starts_with("Overflow") && n.contains("do not fit")), "{notes:?}");
    let sizes: Vec<f64> = after
        .glyphs
        .iter()
        .filter(|g| g.line >= lines[0] && g.line < lines[1] && !g.text.trim().is_empty())
        .map(|g| g.font_size)
        .collect();
    assert!(sizes.iter().all(|s| (s - 13.0).abs() < 0.2), "{sizes:?}");
}

#[test]
fn a_restyle_with_room_keeps_every_word() {
    // A slightly larger size grows the paragraph by less than the column
    // has left.
    let before = layout(WIKIPEDIA, 1);
    let following = before.paragraphs[paragraph(&before, "PDF.js is used in Thunderbird")].bbox;
    let outcome = restyle(WIKIPEDIA, "The project was created to provide", 11.5);
    let after = layout(&outcome.bytes, 1);
    let restyled = paragraph(&after, "The project was created to provide");
    assert!(collisions(&after, restyled).is_empty(), "{:?}", collisions(&after, restyled));
    let text = after.text_in(after.paragraphs[restyled].range);
    assert!(text.ends_with("for fast rendering speeds.[9]"), "{text:?}");
    let moved = after.paragraphs[paragraph(&after, "PDF.js is used in Thunderbird")].bbox;
    assert!(moved[3] < following[3] - 1.0, "{moved:?} from {following:?}");
    let notes = &outcome.reports[0].notes;
    assert!(!notes.iter().any(|n| n.contains("Overflow")), "{notes:?}");
}

#[test]
fn pushing_moves_only_the_edited_column() {
    // Left and right column lines alternate in content order.
    let before = layout(TWO_COLUMNS, 1);
    let box_of = |l: &PageTextLayout, prefix: &str| l.paragraphs[paragraph(l, prefix)].bbox;
    let outcome = restyle(TWO_COLUMNS, "Left column line 05", 24.0);
    let after = layout(&outcome.bytes, 1);
    let restyled = paragraph(&after, "Left column line 05");
    assert!(collisions(&after, restyled).is_empty(), "{:?}", collisions(&after, restyled));
    for n in 6..=16 {
        let left = format!("Left column line {n:02}");
        let right = format!("Right column line {n:02}");
        let (a, b) = (box_of(&before, &left), box_of(&after, &left));
        assert!(b[1] < a[1] - 5.0, "{left} moved down: {a:?} -> {b:?}");
        let (a, b) = (box_of(&before, &right), box_of(&after, &right));
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01), "{right} stays: {a:?} -> {b:?}");
    }
    for n in 1..=4 {
        let left = format!("Left column line {n:02}");
        let (a, b) = (box_of(&before, &left), box_of(&after, &left));
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01), "{left} above stays");
    }
}

#[test]
fn a_paragraph_without_room_on_the_page_reports_its_overflow() {
    let before = layout(WIKIPEDIA, 1);
    let outcome = restyle(WIKIPEDIA, "According to a benchmark by Mozilla", 18.0);
    let after = layout(&outcome.bytes, 1);
    let notes = &outcome.reports[0].notes;
    assert!(notes.iter().any(|n| n.starts_with("Overflow")), "{notes:?}");
    // (The heading above it, drawn just before the re-set text, may join
    // its paragraph in the layout.)
    let restyled = after
        .paragraphs
        .iter()
        .position(|p| after.text_in(p.range).contains("According to a benchmark by Mozilla"))
        .unwrap();
    assert!(collisions(&after, restyled).is_empty(), "{:?}", collisions(&after, restyled));
    // The requested size is kept (never shrunk to fit) ...
    let lines = after.paragraphs[restyled].lines;
    let sizes: Vec<f64> = after
        .glyphs
        .iter()
        .filter(|g| g.line >= lines[0] && g.line < lines[1] && !g.text.trim().is_empty())
        .map(|g| g.font_size)
        .collect();
    assert!(sizes.iter().all(|s| (s - 18.0).abs() < 0.2), "{sizes:?}");
    // ... the paragraph below was moved but stays on the page ...
    let below = &before.paragraphs[paragraph(&before, "PDF.js supports most of the PDF")];
    let moved = after.paragraphs[paragraph(&after, "PDF.js supports most of the PDF")].bbox;
    assert!(moved[1] < below.bbox[1] - 1.0 && moved[1] >= after.page_box[1], "{moved:?}");
    assert!(after.glyphs.iter().all(|g| g.bbox[1] >= after.page_box[1] - 0.01));
    // ... and only the restyled paragraph lost the words that did not fit.
    assert!(after.text.contains(&before.text_in(below.range)));
}

// ---- Text inside Form XObjects ----

/// Two pages. Page 1 places form `Fm1` ("Shared form text ...") twice; page
/// 2 places it once more and places `Fm2` ("Single form label") once. The
/// forms are drawn from PdfBuilder pages (embedded Liberation Sans) whose
/// content and resources become the form's.
fn forms_pdf() -> Vec<u8> {
    use lopdf::{dictionary, Object, Stream};
    let mut builder = wellfriendpdf_engine::PdfBuilder::new();
    let sans = builder.register_font_bytes("Body-Sans", SANS.to_vec()).unwrap();
    let style = wellfriendpdf_engine::TextStyle::new(sans, 12.0);
    let page = builder.add_page(wellfriendpdf_engine::AuthorPageSize::custom(612.0, 792.0));
    page.draw_text("Shared form text for every placement", 10.0, 20.0, &style).unwrap();
    let page = builder.add_page(wellfriendpdf_engine::AuthorPageSize::custom(612.0, 792.0));
    page.draw_text("Single form label", 10.0, 20.0, &style).unwrap();
    let mut document = lopdf::Document::load_mem(&builder.to_bytes().unwrap()).unwrap();
    let pages = document.get_pages();
    let (first, second) = (pages[&1], pages[&2]);
    let form = |document: &mut lopdf::Document, page| {
        let content = document.get_page_content(page);
        let resources = wellfriendpdf_editor::fonts::page_resources(document, page).unwrap();
        let mut stream = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), 300.into(), 60.into()],
                "Resources" => resources,
            },
            content,
        );
        let _ = stream.compress();
        document.add_object(stream)
    };
    let shared = form(&mut document, first);
    let single = form(&mut document, second);
    let place = |document: &mut lopdf::Document, page, content: &str, xobjects: lopdf::Dictionary| {
        let contents = document.add_object(Stream::new(lopdf::Dictionary::new(), content.as_bytes().to_vec()));
        let dict = document.get_object_mut(page).unwrap().as_dict_mut().unwrap();
        dict.set("Contents", Object::Reference(contents));
        dict.set("Resources", dictionary! { "XObject" => xobjects });
    };
    place(
        &mut document,
        first,
        "q 1 0 0 1 72 600 cm /Fm1 Do Q q 1 0 0 1 72 400 cm /Fm1 Do Q",
        dictionary! { "Fm1" => shared },
    );
    place(
        &mut document,
        second,
        "q 1 0 0 1 72 700 cm /Fm1 Do Q q 1 0 0 1 72 500 cm /Fm2 Do Q",
        dictionary! { "Fm1" => shared, "Fm2" => single },
    );
    let mut out = Vec::new();
    document.save_to(&mut out).unwrap();
    out
}

fn form_text(layout: &PageTextLayout, index: usize) -> String {
    layout.text_in(layout.forms[index].range)
}

fn xobject_names(bytes: &[u8], page: u32) -> Vec<String> {
    let document = lopdf::Document::load_mem(bytes).unwrap();
    let page = document.get_pages()[&page];
    let resources = wellfriendpdf_editor::fonts::page_resources(&document, page).unwrap();
    let xobjects = match resources.get(b"XObject").unwrap() {
        lopdf::Object::Reference(id) => document.get_dictionary(*id).unwrap().clone(),
        lopdf::Object::Dictionary(dict) => dict.clone(),
        other => panic!("{other:?}"),
    };
    xobjects.iter().map(|(name, _)| String::from_utf8_lossy(name).into_owned()).collect()
}

#[test]
fn text_inside_forms_is_in_the_page_layout() {
    let source = forms_pdf();
    let page = layout(&source, 1);
    assert_eq!(page.forms.len(), 2, "{:?}", page.forms);
    assert_eq!(page.page_text_len(), 0);
    for index in 0..2 {
        assert_eq!(form_text(&page, index), "Shared form text for every placement");
        assert!(page.forms[index].editable);
    }
    // Each placement's glyphs sit where its CTM puts them, on its own line.
    let baseline = |index: usize| {
        let range = page.forms[index].range;
        let glyph = page.glyphs.iter().find(|g| g.range[0] == range[0]).unwrap();
        glyph.baseline[0]
    };
    assert!((baseline(0)[1] - 620.0).abs() < 0.5 && (baseline(0)[0] - 82.0).abs() < 0.5, "{:?}", baseline(0));
    assert!((baseline(1)[1] - 420.0).abs() < 0.5, "{:?}", baseline(1));
    assert_eq!(page.paragraphs.len(), 2, "one paragraph per placement");
    // Hit-testing finds the form text.
    let offset = page.hit_test([90.0, 624.0]).unwrap();
    assert!(page.forms[0].range[0] <= offset && offset <= page.forms[0].range[1]);
}

#[test]
fn editing_a_shared_form_copies_it_for_that_placement() {
    let source = forms_pdf();
    let before = layout(&source, 1);
    let range = before.forms[0].range;
    let (start, _) = find(&before, "form text");
    assert!(start >= range[0] && start < range[1]);
    let operations = list(json!([{ "op": "replaceText", "page": 1, "start": start, "end": start + 9,
        "expected": "form text", "text": "label", "fallbackFont": "liberation-sans-regular" }]));
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    keep("form-shared-edited", &outcome.bytes);
    let after = layout(&outcome.bytes, 1);
    assert_eq!(after.forms.len(), 2);
    assert_eq!(form_text(&after, 0), "Shared label for every placement");
    assert_eq!(form_text(&after, 1), "Shared form text for every placement", "the other placement is unchanged");
    let names = xobject_names(&outcome.bytes, 1);
    assert!(names.contains(&"Fm1e1".to_string()) && names.contains(&"Fm1".to_string()), "{names:?}");
    // The edited placement keeps its position.
    let first = after.glyphs.iter().find(|g| g.range[0] == after.forms[0].range[0]).unwrap();
    assert!((first.baseline[0][1] - 620.0).abs() < 0.5, "{:?}", first.baseline);
    // Page 2 still places the original form.
    let other = layout(&outcome.bytes, 2);
    assert_eq!(form_text(&other, 0), "Shared form text for every placement");
    let report = &outcome.reports[0];
    assert!(report.notes.iter().any(|n| n.contains("its own copy")), "{report:?}");
    let region = report.changed_region.unwrap();
    assert!(region[1] > 600.0 && region[3] < 660.0, "the region is in page space: {region:?}");
    assert_eq!(report.caret, Some(start + 5));
}

#[test]
fn a_form_placed_once_is_edited_in_place_and_restyled() {
    let source = forms_pdf();
    let before = layout(&source, 2);
    let single = before.forms.iter().position(|f| before.text_in(f.range) == "Single form label").unwrap();
    let (start, end) = find(&before, "label");
    let operations = list(json!([
        { "op": "replaceText", "page": 2, "start": start, "end": end, "expected": "label", "text": "title",
          "fallbackFont": "liberation-sans-regular" }
    ]));
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    let after = layout(&outcome.bytes, 2);
    assert_eq!(after.text_in(after.forms[single].range), "Single form title");
    assert_eq!(xobject_names(&outcome.bytes, 2), xobject_names(&source, 2), "no copy for a single placement");
    // The shared form on the same page is untouched.
    let shared = 1 - single;
    assert_eq!(after.text_in(after.forms[shared].range), "Shared form text for every placement");

    // Restyling the shared form's paragraph on page 1 (second placement).
    let page = layout(&source, 1);
    let range = page.forms[1].range;
    let operations = list(json!([{ "op": "restyleText", "page": 1, "start": range[0], "end": range[1],
        "expected": page.text_in(range), "font": "gelasio-bold", "size": 14.0 }]));
    let outcome = apply_operations(&source, &operations, &library(), false).unwrap();
    keep("form-restyled", &outcome.bytes);
    let after = layout(&outcome.bytes, 1);
    assert_eq!(after.forms.len(), 2);
    let restyled: Vec<_> = after
        .glyphs
        .iter()
        .filter(|g| g.range[0] >= after.forms[1].range[0] && g.range[1] <= after.forms[1].range[1] && !g.text.trim().is_empty())
        .collect();
    assert!(!restyled.is_empty() && restyled.iter().all(|g| (g.font_size - 14.0).abs() < 0.2), "{restyled:?}");
    assert!(restyled.iter().all(|g| g.bbox[1] > 360.0 && g.bbox[3] < 460.0), "restyled in place");
    assert_eq!(after.text_in(after.forms[1].range).split_whitespace().collect::<Vec<_>>().join(" "), "Shared form text for every placement");
    assert!(!outcome.reports[0].notes.iter().any(|n| n.contains("Overflow")), "{:?}", outcome.reports[0].notes);
    // The form's box (its clip) grew to show every restyled glyph.
    let document = lopdf::Document::load_mem(&outcome.bytes).unwrap();
    let resources = wellfriendpdf_editor::fonts::page_resources(&document, document.get_pages()[&1]).unwrap();
    let xobjects = match resources.get(b"XObject").unwrap() {
        lopdf::Object::Dictionary(dict) => dict.clone(),
        lopdf::Object::Reference(id) => document.get_dictionary(*id).unwrap().clone(),
        other => panic!("{other:?}"),
    };
    let form = xobjects.get(after.forms[1].name.as_bytes()).unwrap().as_reference().unwrap();
    let bbox: Vec<f64> = document.get_object(form).unwrap().as_stream().unwrap().dict.get(b"BBox").unwrap()
        .as_array().unwrap().iter().map(|o| o.as_float().unwrap() as f64).collect();
    for glyph in &restyled {
        let [x0, y0, x1, y1] = glyph.bbox;
        assert!(x0 - 72.0 >= bbox[0] && x1 - 72.0 <= bbox[2] && y0 - 400.0 >= bbox[1] && y1 - 400.0 <= bbox[3], "{glyph:?} inside {bbox:?}");
    }
    assert_eq!(form_text(&after, 0), "Shared form text for every placement");
    let untouched: Vec<_> = after
        .glyphs
        .iter()
        .filter(|g| g.range[0] >= after.forms[0].range[0] && g.range[1] <= after.forms[0].range[1] && !g.text.trim().is_empty())
        .collect();
    assert!(untouched.iter().all(|g| (g.font_size - 12.0).abs() < 0.01));
}

fn find(layout: &PageTextLayout, needle: &str) -> (usize, usize) {
    let text: Vec<char> = layout.text.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    let start = (0..=text.len() - needle.len())
        .find(|&i| text[i..i + needle.len()] == needle[..])
        .unwrap_or_else(|| panic!("{needle:?} not in {:?}", layout.text));
    (start, start + needle.len())
}
