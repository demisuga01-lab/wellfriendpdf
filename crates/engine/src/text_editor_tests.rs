use super::*;
use crate::writer::{OutputObject, PdfWriter};
use crate::{PdfDictionary, PdfObject};

fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}

/// One Letter page, Helvetica (WinAnsi) as /F1, with `content`.
fn fixture(content: &[u8]) -> Vec<u8> {
    let mut catalog = PdfDictionary::empty();
    catalog.insert("Type", PdfObject::Name("Catalog".into()));
    catalog.insert("Pages", reference(2));
    let mut pages = PdfDictionary::empty();
    pages.insert("Type", PdfObject::Name("Pages".into()));
    pages.insert("Count", PdfObject::Integer(1));
    pages.insert("Kids", PdfObject::Array(vec![reference(3)]));
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".into()));
    font.insert("Subtype", PdfObject::Name("Type1".into()));
    font.insert("BaseFont", PdfObject::Name("Helvetica".into()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".into()));
    let mut fonts = PdfDictionary::empty();
    fonts.insert("F1", reference(5));
    let mut resources = PdfDictionary::empty();
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let mut page = PdfDictionary::empty();
    page.insert("Type", PdfObject::Name("Page".into()));
    page.insert("Parent", reference(2));
    page.insert(
        "MediaBox",
        PdfObject::Array(
            [0, 0, 612, 792]
                .into_iter()
                .map(PdfObject::Integer)
                .collect(),
        ),
    );
    page.insert("Resources", PdfObject::Dictionary(resources));
    page.insert("Contents", reference(4));
    let mut stream = PdfDictionary::empty();
    stream.insert("Length", PdfObject::Integer(content.len() as i64));
    PdfWriter::new(
        vec![
            OutputObject {
                number: 1,
                object: PdfObject::Dictionary(catalog),
            },
            OutputObject {
                number: 2,
                object: PdfObject::Dictionary(pages),
            },
            OutputObject {
                number: 3,
                object: PdfObject::Dictionary(page),
            },
            OutputObject {
                number: 4,
                object: PdfObject::Stream {
                    dict: stream,
                    raw: content.to_vec(),
                },
            },
            OutputObject {
                number: 5,
                object: PdfObject::Dictionary(font),
            },
        ],
        1,
    )
    .write()
    .expect("fixture")
}

const TWO_LINES: &[u8] = b"BT /F1 12 Tf 72 700 Td (Hello world) Tj 0 -14 Td (Second line) Tj ET";

fn layout() -> PageTextLayout {
    PageTextLayout::from_bytes(&fixture(TWO_LINES), 1).expect("layout")
}

#[test]
fn layout_maps_every_glyph_to_logical_text_and_lines() {
    let layout = layout();
    assert_eq!(layout.text, "Hello worldSecond line");
    assert!(layout.editable);
    assert_eq!(layout.glyphs.len(), 22);
    assert_eq!(layout.lines.len(), 2);
    assert_eq!(layout.lines[0].range, [0, 11]);
    assert_eq!(layout.lines[1].range, [11, 22]);
    assert_eq!(
        layout.paragraphs.len(),
        1,
        "a 14pt pitch at 12pt is one paragraph"
    );
    let h = &layout.glyphs[0];
    assert_eq!(h.text, "H");
    assert!((h.baseline[0][0] - 72.0).abs() < 1e-9);
    assert!((h.baseline[0][1] - 700.0).abs() < 1e-9);
    // Helvetica H advance is 722/1000 em.
    assert!((h.baseline[1][0] - (72.0 + 8.664)).abs() < 1e-6);
    assert!((h.font_size - 12.0).abs() < 1e-9);
    assert!(h.bbox[3] > 700.0 && h.bbox[1] < 700.0);
}

#[test]
fn hit_test_and_caret_follow_glyph_edges() {
    let layout = layout();
    // Right half of "H" snaps to the edge after it.
    assert_eq!(layout.hit_test([78.0, 703.0]), Some(1));
    assert_eq!(layout.hit_test([72.5, 703.0]), Some(0));
    // A click left of the second line lands at its start.
    assert_eq!(layout.hit_test([10.0, 688.0]), Some(11));
    let caret = layout.caret(0).expect("caret");
    assert!((caret.bottom[0] - 72.0).abs() < 1e-9);
    assert!(caret.top[1] > caret.bottom[1]);
    // The line-break offset draws at the end of the first line.
    let at_break = layout.caret(11).expect("caret");
    assert_eq!(at_break.line, 0);
    assert!(at_break.bottom[1] < 700.0 && at_break.bottom[1] > 695.0);
}

#[test]
fn selection_words_and_movement() {
    let layout = layout();
    assert_eq!(layout.selection_quads([6, 16]).len(), 2);
    assert_eq!(layout.text_in([6, 11]), "world");
    assert_eq!(layout.word_range(2), [0, 5]);
    assert_eq!(layout.word_range(8), [6, 11]);
    assert_eq!(layout.move_caret(2, CaretMove::WordRight), 6);
    assert_eq!(layout.move_caret(8, CaretMove::WordLeft), 6);
    assert_eq!(layout.move_caret(3, CaretMove::LineEnd), 11);
    assert_eq!(layout.move_caret(14, CaretMove::LineStart), 11);
    let down = layout.move_caret(2, CaretMove::LineDown);
    assert!((11..=14).contains(&down), "{down}");
    assert_eq!(layout.move_caret(2, CaretMove::LineUp), 0);
    assert_eq!(layout.paragraph_range(4), [0, 22]);
}

#[test]
fn device_mapping_round_trips_at_any_dpi() {
    let layout = layout();
    let pixel = layout.to_device([72.0, 700.0], 144);
    assert!((pixel[0] - 144.0).abs() < 1e-6);
    assert!((pixel[1] - 184.0).abs() < 1e-6);
    let back = layout.from_device(pixel, 144).expect("invertible");
    assert!((back[0] - 72.0).abs() < 1e-9 && (back[1] - 700.0).abs() < 1e-9);
    assert_eq!(
        layout.hit_test_device(layout.to_device([78.0, 703.0], 96), 96),
        Some(1)
    );
}

#[test]
fn session_edits_undo_and_redo() {
    let original = fixture(TWO_LINES);
    let mut session = TextEditSession::open(original.clone()).expect("session");
    let outcome = session.replace(1, [6, 11], "there").expect("replace");
    assert_eq!(outcome.caret, 11);
    assert!(outcome.dirty_region.is_some());
    let edited = session.layout(1).expect("layout").text.clone();
    assert!(edited.contains("Hello there"), "{edited}");
    assert!(!edited.contains("world"), "{edited}");
    assert!(session.undo());
    assert_eq!(session.bytes(), original.as_slice());
    assert!(session.layout(1).expect("layout").text.contains("world"));
    assert!(session.redo());
    assert!(session.layout(1).expect("layout").text.contains("there"));
    assert!(!session.redo());
}

#[test]
fn session_inserts_and_deletes_by_offset() {
    let mut session = TextEditSession::open(fixture(TWO_LINES)).expect("session");
    let inserted = session.insert(1, 5, ",").expect("insert");
    assert_eq!(inserted.caret, 6);
    assert!(session
        .layout(1)
        .expect("layout")
        .text
        .starts_with("Hello, world"));
    session.delete(1, [5, 6]).expect("delete");
    assert!(session
        .layout(1)
        .expect("layout")
        .text
        .starts_with("Hello world"));
}
