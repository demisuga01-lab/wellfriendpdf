use std::io::{Cursor, Read};

use wellfriendpdf_engine::{
    content_defined_chunks, hamming_distance, pdf_to_docx, pdf_to_pptx, pdf_to_xlsx,
    replace_text_pdf, resource_digest, simhash_text, AuthorPageSize, ContentEngine, DocxOptions,
    EditMode, EditTextStyle, EditableBuildOptions, PdfBuilder, PdfEditor, PptxOptions,
    StandardFont, TextReplacementOptions, TextStyle, XlsxOptions,
};
use zip::ZipArchive;

fn sample_pdf() -> Vec<u8> {
    let mut doc = PdfBuilder::new();
    doc.add_page(AuthorPageSize::LETTER)
        .draw_text(
            "Advanced Rendering Heading",
            72.0,
            720.0,
            &TextStyle::standard(StandardFont::HelveticaBold, 18.0),
        )
        .unwrap()
        .draw_text(
            "Editable alpha text for conversion.",
            72.0,
            690.0,
            &TextStyle::standard(StandardFont::Helvetica, 12.0),
        )
        .unwrap()
        .draw_text(
            "List item one",
            90.0,
            660.0,
            &TextStyle::standard(StandardFont::Helvetica, 11.0),
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
fn editable_model_exports_markdown_html_and_json() {
    let engine = ContentEngine::open_bytes(sample_pdf()).unwrap();
    let model = engine
        .build_editable_document(&EditableBuildOptions::default())
        .unwrap();
    assert_eq!(model.schema_version, "0.1");
    assert!(model
        .blocks
        .iter()
        .any(|block| block_text(block).contains("Editable alpha")));
    assert!(model.to_markdown().contains("Editable alpha"));
    assert!(model.to_semantic_html().contains("<p>"));
    let json = serde_json::to_string(&model).unwrap();
    assert!(json.contains("\"schema_version\""));
}

#[test]
fn office_exports_read_back_as_openxml_packages() {
    let engine = ContentEngine::open_bytes(sample_pdf()).unwrap();
    let docx = pdf_to_docx(&engine, &DocxOptions::default()).unwrap();
    let pptx = pdf_to_pptx(&engine, &PptxOptions::default()).unwrap();
    let xlsx = pdf_to_xlsx(&engine, &XlsxOptions::default()).unwrap();

    assert!(zip_entry(&docx, "word/document.xml").contains("<w:document"));
    assert!(zip_entry(&pptx, "ppt/presentation.xml").contains("<p:presentation"));
    assert!(zip_entry(&xlsx, "xl/workbook.xml").contains("<workbook"));
}

#[test]
fn text_replacement_removes_old_text_after_reopen() {
    let input = sample_pdf();
    let (edited, report) = replace_text_pdf(
        input,
        "Editable alpha text",
        "Editable beta text",
        TextReplacementOptions {
            replacement_style: EditTextStyle::new(12.0),
            ..TextReplacementOptions::default()
        },
    )
    .unwrap();
    assert_eq!(report.replacements, 1);
    assert!(report.verified_old_absent);
    let engine = ContentEngine::open_bytes(edited).unwrap();
    let text = engine.get_page_text(1).unwrap();
    assert!(!text.contains("Editable alpha text"));
    assert!(text.contains("Editable beta text"));
}

#[test]
fn incremental_save_preserves_original_prefix_and_reopens() {
    let input = sample_pdf();
    let mut editor = PdfEditor::open_bytes(input.clone()).unwrap();
    editor
        .draw_text(
            1,
            "incremental note",
            72.0,
            620.0,
            EditTextStyle::new(10.0),
            wellfriendpdf_engine::OverlayLayer::Overlay,
        )
        .unwrap();
    let edited = editor.save_to_bytes(EditMode::Incremental).unwrap();
    assert!(edited.starts_with(&input));
    let text = ContentEngine::open_bytes(edited)
        .unwrap()
        .get_page_text(1)
        .unwrap();
    assert!(text.contains("incremental note"));
}

#[test]
fn deterministic_editing_and_versioning_helpers_are_stable() {
    let input = sample_pdf();
    let (a, _) = replace_text_pdf(
        input.clone(),
        "Editable alpha text",
        "Editable beta text",
        TextReplacementOptions::default(),
    )
    .unwrap();
    let (b, _) = replace_text_pdf(
        input,
        "Editable alpha text",
        "Editable beta text",
        TextReplacementOptions::default(),
    )
    .unwrap();
    assert_eq!(resource_digest(&a), resource_digest(&b));

    let chunks = content_defined_chunks(&a, 128, 256, 512);
    assert_eq!(
        chunks.iter().map(|chunk| chunk.length).sum::<usize>(),
        a.len()
    );
    let near_a = simhash_text("editable document model conversion output");
    let near_b = simhash_text("editable document model conversion result");
    let far = simhash_text("spot color overprint device cmyk prepress");
    assert!(hamming_distance(near_a, near_b) < hamming_distance(near_a, far));
}

fn block_text(block: &wellfriendpdf_engine::EditableBlock) -> String {
    block
        .paragraphs
        .iter()
        .map(|paragraph| paragraph.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
