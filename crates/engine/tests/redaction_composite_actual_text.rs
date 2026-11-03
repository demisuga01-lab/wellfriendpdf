//! QF-2561: redacting text drawn with a composite (Identity-H) font inside a
//! marked-content span with /ActualText must remove the words from text
//! extraction too. The glyph rewriter records removed glyph codes, which do
//! not match the alternate text, so the span alternate text has to go.
use wellfriendpdf_engine::{
    AuthorPageSize, ContentEngine, EditMode, ImageRect, PageTextLayout, PdfBuilder, PdfEditor,
    RedactionOptions, TextStyle,
};

const SERIF: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Regular.ttf");

fn tagged_pdf() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    let serif = builder
        .register_font_bytes("Office-Serif", SERIF.to_vec())
        .unwrap();
    let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
    page.draw_text("Public heading", 72.0, 720.0, &TextStyle::new(serif, 12.0))
        .unwrap();
    page.draw_text(
        "Contact: secret@example.com",
        72.0,
        640.0,
        &TextStyle::new(serif, 12.0),
    )
    .unwrap();
    builder.to_bytes().unwrap()
}

#[test]
fn redacted_composite_text_leaves_no_alternate_text() {
    let source = tagged_pdf();
    let before = PageTextLayout::from_bytes(&source, 1).unwrap();
    assert!(
        before.text.contains("secret@example.com"),
        "{}",
        before.text
    );
    let mut editor = PdfEditor::open_bytes(source).unwrap();
    editor
        .redact(
            1,
            ImageRect::new(115.0, 636.0, 120.0, 18.0),
            RedactionOptions::default(),
        )
        .unwrap();
    let out = editor.save_to_bytes(EditMode::FullRewrite).unwrap();
    let after = PageTextLayout::from_bytes(&out, 1).unwrap();
    assert!(
        !after.text.contains("secret"),
        "layout text: {}",
        after.text
    );
    let extracted = ContentEngine::open_bytes(out.clone())
        .unwrap()
        .extract_semantic_text(&[1])
        .unwrap();
    assert!(!extracted.contains("secret"), "extracted: {extracted}");
    assert!(
        after.text.contains("Public heading"),
        "unredacted text keeps its own spans"
    );
    assert!(after.text.contains("Contact"));
}
