use std::io::{Cursor, Read};

use wellfriendpdf_engine::{
    edit_paragraph_reflow_pdf, pdf_to_docx, resource_dedup_report, resource_digest, AuthorPageSize,
    ContentEngine, DeterministicSaveOptions, DocxLayout, DocxOptions, EditMode, EditTextStyle,
    EditableBuildOptions, ImageRect, OverlayLayer, ParagraphEditOperation, ParagraphReflowOptions,
    PdfBuilder, PdfEditor, StandardFont, TextStyle,
};
use zip::ZipArchive;

fn sample_pdf() -> Vec<u8> {
    let mut doc = PdfBuilder::new();
    doc.add_page(AuthorPageSize::LETTER)
        .draw_text(
            "Prompt 08B Heading",
            72.0,
            720.0,
            &TextStyle::standard(StandardFont::HelveticaBold, 18.0),
        )
        .unwrap()
        .draw_text(
            "Editable alpha text for paragraph editing.",
            72.0,
            690.0,
            &TextStyle::standard(StandardFont::Helvetica, 12.0),
        )
        .unwrap();
    doc.to_bytes().unwrap()
}

fn zip_entry(bytes: &[u8], name: &str) -> String {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut entry = archive.by_name(name).unwrap();
    let mut out = String::new();
    entry.read_to_string(&mut out).unwrap();
    out
}

#[test]
fn paragraph_reflow_replace_reopens_with_new_text_and_old_absent() {
    let (edited, report) = edit_paragraph_reflow_pdf(
        sample_pdf(),
        "Editable alpha text",
        ParagraphEditOperation::Replace {
            replacement: "Editable beta paragraph that wraps cleanly".to_string(),
        },
        ParagraphReflowOptions {
            bounding_region: Some(ImageRect::new(72.0, 650.0, 190.0, 72.0)),
            replacement_style: EditTextStyle::new(12.0),
            ..ParagraphReflowOptions::default()
        },
    )
    .unwrap();
    assert_eq!(report.edits, 1);
    assert_eq!(report.edit_mode, "paragraph_reflow");
    assert!(report.lines_written >= 2, "replacement should wrap");
    assert!(report.verified_old_absent);
    assert!(report.verified_new_present);
    let text = ContentEngine::open_bytes(edited)
        .unwrap()
        .get_page_text(1)
        .unwrap();
    assert!(!text.contains("Editable alpha text"));
    assert!(text.contains("Editable beta"));
    assert!(text.contains("wraps cleanly"));
}

#[test]
fn paragraph_reflow_insert_and_delete_are_undoable_in_model() {
    let engine = ContentEngine::open_bytes(sample_pdf()).unwrap();
    let mut model = engine
        .build_editable_document(&EditableBuildOptions::default())
        .unwrap();
    let block = model
        .blocks
        .iter()
        .find(|block| {
            block
                .paragraphs
                .iter()
                .any(|p| p.text.contains("Editable alpha"))
        })
        .unwrap()
        .clone();
    let paragraph_id = block.paragraphs[0].id.clone();
    assert!(model.insert_paragraph_text(&block.id, &paragraph_id, 8, " inserted"));
    assert!(model.to_markdown().contains("Editable inserted alpha"));
    assert!(model.delete_paragraph_range(&block.id, &paragraph_id, 8, 17));
    assert!(model.to_markdown().contains("Editable alpha"));
    assert_eq!(model.transactions.entries.len(), 2);
    assert_eq!(model.transactions.patches.len(), 2);
    assert!(!model.transactions.checkpoints.is_empty());
    assert!(model.undo());
    assert!(model.to_markdown().contains("Editable inserted alpha"));
    assert!(model.undo());
    assert!(!model.to_markdown().contains("Editable inserted alpha"));
    assert!(model.redo());
    assert!(model.redo());
    assert!(model.replace_paragraph_text(&block.id, &paragraph_id, "Branched paragraph"));
    assert_eq!(
        model.transactions.cursor,
        model.transactions.entries.len(),
        "branch edit should clear redo history"
    );
    let json = serde_json::to_string(&model.transactions).unwrap();
    assert!(json.contains("\"checkpoints\""));
    assert!(json.contains("\"patches\""));
}

#[test]
fn paragraph_reflow_insert_and_delete_reopen_with_expected_text() {
    let (inserted, insert_report) = edit_paragraph_reflow_pdf(
        sample_pdf(),
        "Editable alpha text",
        ParagraphEditOperation::Insert {
            offset: 8,
            text: " inserted".to_string(),
        },
        ParagraphReflowOptions {
            bounding_region: Some(ImageRect::new(72.0, 650.0, 240.0, 60.0)),
            ..ParagraphReflowOptions::default()
        },
    )
    .unwrap();
    assert!(insert_report.verified_new_present);
    assert!(insert_report.verified_old_absent);
    let inserted_text = ContentEngine::open_bytes(inserted)
        .unwrap()
        .get_page_text(1)
        .unwrap();
    assert!(inserted_text.contains("Editable inserted alpha text"));
    assert!(!inserted_text.contains("Editable alpha text for paragraph editing"));

    let (deleted, delete_report) = edit_paragraph_reflow_pdf(
        sample_pdf(),
        "Editable alpha text",
        ParagraphEditOperation::Delete { start: 9, end: 15 },
        ParagraphReflowOptions {
            bounding_region: Some(ImageRect::new(72.0, 650.0, 240.0, 60.0)),
            ..ParagraphReflowOptions::default()
        },
    )
    .unwrap();
    assert!(delete_report.verified_new_present);
    assert!(delete_report.verified_old_absent);
    let deleted_text = ContentEngine::open_bytes(deleted)
        .unwrap()
        .get_page_text(1)
        .unwrap();
    assert!(deleted_text.contains("Editable text for paragraph editing"));
    assert!(!deleted_text.contains("Editable alpha text"));
}

#[test]
fn paragraph_reflow_overflow_is_reported() {
    let err = edit_paragraph_reflow_pdf(
        sample_pdf(),
        "Editable alpha text",
        ParagraphEditOperation::Replace {
            replacement: "This replacement is intentionally far too long for one tiny line box"
                .repeat(4),
        },
        ParagraphReflowOptions {
            bounding_region: Some(ImageRect::new(72.0, 675.0, 60.0, 14.0)),
            max_lines: 1,
            ..ParagraphReflowOptions::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("paragraph reflow overflow"));
}

#[test]
fn page_faithful_docx_contains_positioned_text_boxes() {
    let engine = ContentEngine::open_bytes(sample_pdf()).unwrap();
    let flowing = pdf_to_docx(&engine, &DocxOptions::default()).unwrap();
    let faithful = pdf_to_docx(
        &engine,
        &DocxOptions {
            include_images: true,
            layout: DocxLayout::PageFaithful,
        },
    )
    .unwrap();
    let flowing_xml = zip_entry(&flowing, "word/document.xml");
    let faithful_xml = zip_entry(&faithful, "word/document.xml");
    assert!(flowing_xml.contains("<w:p"));
    assert!(faithful_xml.contains("<wp:anchor"));
    assert!(faithful_xml.contains("<wps:txbx"));
    assert!(faithful_xml.contains("Prompt 08B Heading"));
}

#[test]
fn deterministic_save_report_and_incremental_bytes_are_stable() {
    let input = sample_pdf();
    let mut a = PdfEditor::open_bytes(input.clone()).unwrap();
    a.draw_text(
        1,
        "deterministic note",
        72.0,
        620.0,
        EditTextStyle::new(10.0),
        OverlayLayer::Overlay,
    )
    .unwrap();
    let (a_bytes, a_report) = a
        .save_to_bytes_with_options(
            EditMode::Incremental,
            &DeterministicSaveOptions {
                fixed_pdf_date: Some("D:20260704000000Z".to_string()),
                ..DeterministicSaveOptions::default()
            },
        )
        .unwrap();

    let mut b = PdfEditor::open_bytes(input.clone()).unwrap();
    b.draw_text(
        1,
        "deterministic note",
        72.0,
        620.0,
        EditTextStyle::new(10.0),
        OverlayLayer::Overlay,
    )
    .unwrap();
    let (b_bytes, b_report) = b
        .save_to_bytes_with_options(
            EditMode::Incremental,
            &DeterministicSaveOptions {
                fixed_pdf_date: Some("D:20260704000000Z".to_string()),
                ..DeterministicSaveOptions::default()
            },
        )
        .unwrap();

    assert!(a_bytes.starts_with(&input));
    assert_eq!(resource_digest(&a_bytes), resource_digest(&b_bytes));
    assert_eq!(a_report.fixed_pdf_date, b_report.fixed_pdf_date);
    assert!(a_report.deterministic_resource_names);
}

#[test]
fn resource_dedup_report_is_deterministic() {
    let resources = vec![
        b"same image bytes".to_vec(),
        b"other".to_vec(),
        b"same image bytes".to_vec(),
    ];
    let a = resource_dedup_report(&resources);
    let b = resource_dedup_report(&resources);
    assert_eq!(a, b);
    assert_eq!(a.unique_count, 2);
    assert_eq!(a.duplicate_count, 1);
}
