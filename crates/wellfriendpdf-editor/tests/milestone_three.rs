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
