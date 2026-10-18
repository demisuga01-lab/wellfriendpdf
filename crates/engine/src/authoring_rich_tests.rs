//! Tests for rich flow paragraphs.
use super::*;

fn flow() -> FlowDocument {
    FlowDocument::new(PageSize::A4, Margins::all(50.0))
}

#[test]
fn mixed_runs_wrap_and_create_uri_links() {
    let mut flow = flow();
    let body = TextStyle::standard(StandardFont::Helvetica, 11.0);
    let bold = TextStyle::standard(StandardFont::HelveticaBold, 11.0);
    let spans = vec![
        RichSpan::new("Plain words ".repeat(20), body.clone()),
        RichSpan::new("bold part", bold),
        RichSpan::new(" and ", body.clone()),
        RichSpan::new("a link", body.clone()).link("https://example.com/a b"),
        RichSpan::new(" and ", body.clone()),
        RichSpan::new("bad", body).link("javascript:alert(1)"),
    ];
    let report = flow
        .add_rich_paragraph(&spans, &RichParagraphStyle::default())
        .unwrap();
    assert!(report.lines >= 3, "{report:?}");
    assert_eq!(report.links, 1);
    assert_eq!(report.unlinked_uris, 1);
    let link = &flow.builder.pages[0].uri_links[0];
    assert_eq!(link.uri, "https://example.com/a%20b");
    let bytes = flow.into_builder().to_bytes().unwrap();
    let engine = crate::ContentEngine::open_bytes(bytes).unwrap();
    let text = engine.get_page_text(1).unwrap();
    assert!(
        text.contains("bold part") && text.contains("a link"),
        "{text}"
    );
}

#[test]
fn preserved_whitespace_and_overlong_words_break() {
    let mut flow = flow();
    let mono = TextStyle::standard(StandardFont::Courier, 10.0);
    let spans = [RichSpan::new(
        format!("    indented\n{}", "x".repeat(400)),
        mono,
    )];
    let style = RichParagraphStyle {
        preserve_whitespace: true,
        background: Some(Color::device_gray(0.9)),
        ..RichParagraphStyle::default()
    };
    let report = flow.add_rich_paragraph(&spans, &style).unwrap();
    assert!(report.lines >= 4, "{report:?}");
}

#[test]
fn invalid_style_rolls_back_nothing_and_errors() {
    let mut flow = flow();
    let style = RichParagraphStyle {
        heading_level: Some(9),
        ..RichParagraphStyle::default()
    };
    let spans = [RichSpan::new("x", TextStyle::default())];
    assert!(flow.add_rich_paragraph(&spans, &style).is_err());
    assert!(flow.builder.pages[0].commands.is_empty());
}

#[test]
fn uri_normalization_allows_only_external_schemes() {
    assert!(normalize_external_uri("mailto:a@example.com").is_some());
    assert!(normalize_external_uri("file:///etc/passwd").is_none());
    assert!(normalize_external_uri("#anchor").is_none());
}
