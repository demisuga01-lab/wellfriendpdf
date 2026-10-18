//! Tests for the native converters. Every output is reopened with the engine's
//! own parser to check page count, extracted text, metadata and page geometry.
use super::*;
use crate::ContentEngine;

fn open(pdf: &[u8]) -> ContentEngine {
    assert!(pdf.starts_with(b"%PDF-"), "output is not a PDF header");
    ContentEngine::open_bytes(pdf.to_vec()).expect("engine reopens converter output")
}

fn all_text(engine: &ContentEngine) -> String {
    let pages = engine.page_count().unwrap();
    (1..=pages)
        .map(|page| engine.get_page_text(page).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

fn compact(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn media_box(engine: &ContentEngine, page: usize) -> [f64; 4] {
    engine.get_page(page).unwrap().media_box
}

// ---------------------------------------------------------------- shared

#[test]
fn error_codes_are_extracted_from_messages() {
    let error = limit_error(codes::CSV_COLUMN_LIMIT_EXCEEDED, "too many");
    assert_eq!(native_error_code(&error), "csv_column_limit_exceeded");
    assert_eq!(native_contract_code(&error), "RESOURCE_LIMIT");
    let error = WellfriendError::invalid_input("plain failure");
    assert_eq!(native_error_code(&error), "parse");
    assert_eq!(native_contract_code(&error), "INVALID_INPUT");
    let error = input_error(codes::JSON_PARSE_ERROR, "x");
    assert_eq!(native_contract_code(&error), "INVALID_JSON");
    let error = WellfriendError::UnsupportedFeature("x".into());
    assert_eq!(native_contract_code(&error), "UNSUPPORTED_INPUT");
    let error = WellfriendError::EncryptedDocument;
    assert_eq!(native_contract_code(&error), "ENCRYPTED_INPUT");
    let error = WellfriendError::Cancelled("x".into());
    assert_eq!(native_contract_code(&error), "INTERNAL");
}

#[test]
fn options_reject_unknown_fields_and_parse_page_sizes() {
    let options: TextToPdfOptions = parse_native_options(Some(
        r#"{"page_size":"legal","margins":{"top":10,"right":20,"bottom":30,"left":40}}"#,
    ))
    .unwrap();
    assert_eq!(options.page_size, NativePageSize::Legal);
    assert!(matches!(options.margins, NativeMargins::Sides { left, .. } if left == 40.0));
    // Unknown keys are ignored; the server's camelCase names are accepted.
    let options: TextToPdfOptions = parse_native_options(Some(
        r#"{"pageSize":"Letter","orientation":"Landscape","fontSize":12,"margin":20,"title":"T","author":"A","somethingElse":1}"#,
    ))
    .unwrap();
    assert_eq!(options.page_size, NativePageSize::Letter);
    assert_eq!(options.orientation, Some(NativeOrientation::Landscape));
    assert_eq!(options.font_size, 12.0);
    assert_eq!(options.margins, NativeMargins::All(20.0));
    assert_eq!(options.author.as_deref(), Some("A"));
    let csv: CsvToPdfOptions = parse_native_options(Some(
        r#"{"headerRow":false,"delimiter":";","orientation":"portrait"}"#,
    ))
    .unwrap();
    assert!(!csv.has_header);
    assert_eq!(csv.orientation, CsvOrientation::Portrait);
    let error =
        parse_native_options::<TextToPdfOptions>(Some(r#"{"fontSize":"big"}"#)).unwrap_err();
    assert_eq!(native_error_code(&error), codes::INVALID_OPTIONS);
    assert_eq!(native_contract_code(&error), "INVALID_OPTIONS");
    let error = parse_native_options::<TextToPdfOptions>(Some("[1]")).unwrap_err();
    assert_eq!(native_contract_code(&error), "INVALID_OPTIONS");
    assert_eq!(
        NativePageSize::parse("Letter"),
        Some(NativePageSize::Letter)
    );
    assert!(NativeMargins::All(500.0).resolve(PageSize::A4).is_err());
}

#[test]
fn tabs_expand_to_column_stops() {
    assert_eq!(expand_tabs("a\tb", 4), "a   b");
    assert_eq!(expand_tabs("\tx", 2), "  x");
}

#[test]
fn sanitize_replaces_uncovered_glyphs_and_counts_them() {
    let mut fonts = NativeFonts::new();
    let mut replaced = 0;
    let out = fonts.sanitize(StackKind::Sans, "a\u{4E2D}bc\u{7}\u{E0001}", &mut replaced);
    assert_eq!(out, "a\u{FFFD}bc");
    assert_eq!(replaced, 1);
}

// ---------------------------------------------------------------- text

#[test]
fn text_to_pdf_writes_selectable_unicode_text_and_metadata() {
    let input = "Hello Wellfriend\nПривет мир — Ελληνικά\nשלום עולם\n\tIndented";
    let options = TextToPdfOptions {
        title: Some("Plain text".into()),
        ..TextToPdfOptions::default()
    };
    let output = text_to_pdf_with_report(input.as_bytes(), &options).unwrap();
    assert_eq!(output.report.pages, 1);
    assert_eq!(output.report.replaced_characters, 0);
    let engine = open(&output.pdf);
    assert_eq!(engine.page_count().unwrap(), 1);
    let text = all_text(&engine);
    assert!(text.contains("Hello Wellfriend"), "{text}");
    assert!(text.contains("Привет"), "{text}");
    assert!(text.contains("Ελληνικά"), "{text}");
    let info = engine.document_info().unwrap();
    assert_eq!(info.title.as_deref(), Some("Plain text"));
    assert!(info.creator.unwrap_or_default().contains("text-to-pdf"));
    let mb = media_box(&engine, 1);
    assert!((mb[2] - PageSize::A4.width).abs() < 0.5);
}

#[test]
fn text_to_pdf_breaks_pages_and_honours_page_size_and_monospace() {
    let input = (1..=400)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let options = TextToPdfOptions {
        page_size: NativePageSize::Letter,
        monospace: true,
        font_size: 12.0,
        ..TextToPdfOptions::default()
    };
    let output = text_to_pdf_with_report(input.as_bytes(), &options).unwrap();
    assert!(output.report.pages > 5, "{:?}", output.report);
    let engine = open(&output.pdf);
    assert_eq!(engine.page_count().unwrap(), output.report.pages);
    assert!((media_box(&engine, 1)[3] - 792.0).abs() < 0.5);
    let text = all_text(&engine);
    assert!(text.contains("line 1") && text.contains("line 400"));
}

#[test]
fn text_to_pdf_form_feed_starts_a_page() {
    let output = text_to_pdf_with_report(b"first\x0csecond", &TextToPdfOptions::default()).unwrap();
    assert_eq!(output.report.pages, 2);
    let engine = open(&output.pdf);
    assert!(engine.get_page_text(2).unwrap().contains("second"));
}

#[test]
fn text_to_pdf_wrap_modes() {
    let long = "word ".repeat(60) + &"x".repeat(300);
    for wrap in [LineWrap::Word, LineWrap::Char] {
        let options = TextToPdfOptions {
            line_wrap: wrap,
            ..TextToPdfOptions::default()
        };
        let output = text_to_pdf_with_report(long.as_bytes(), &options).unwrap();
        let text = compact(&all_text(&open(&output.pdf)));
        assert!(text.contains(&"x".repeat(300)), "{wrap:?} lost text");
    }
    let options = TextToPdfOptions {
        line_wrap: LineWrap::None,
        ..TextToPdfOptions::default()
    };
    let error = text_to_pdf(long.as_bytes(), &options).unwrap_err();
    assert_eq!(native_error_code(&error), codes::TEXT_LINE_TOO_WIDE);
    assert!(text_to_pdf(b"short", &options).is_ok());
}

#[test]
fn text_to_pdf_rejects_bad_input_and_options() {
    let error = text_to_pdf(b"\xff\xfe", &TextToPdfOptions::default()).unwrap_err();
    assert_eq!(native_error_code(&error), codes::INVALID_UTF8);
    for size in [6.0, 23.0, f64::NAN] {
        let options = TextToPdfOptions {
            font_size: size,
            ..TextToPdfOptions::default()
        };
        let error = text_to_pdf(b"x", &options).unwrap_err();
        assert_eq!(native_error_code(&error), codes::FONT_SIZE_OUT_OF_RANGE);
    }
}

#[test]
fn text_to_pdf_replaces_cjk_without_failing() {
    let output =
        text_to_pdf_with_report("中文 ok".as_bytes(), &TextToPdfOptions::default()).unwrap();
    assert_eq!(output.report.replaced_characters, 2);
    assert!(all_text(&open(&output.pdf)).contains("ok"));
}

// ---------------------------------------------------------------- markdown

const MARKDOWN: &str = r#"# Report Title

Intro with **bold**, *italic*, ~~gone~~ and `code` plus a [link](https://example.com/docs).

## Lists

- first item
- second item
  1. nested one
  2. nested two

- [x] done task
- [ ] open task

> Quoted wisdom

```rust
fn main() {
    println!("hi");
}
```

| Name | Qty |
|:-----|----:|
| Apple | 3 |
| Pear | 10 |

---

![remote](https://example.com/x.png)
"#;

#[test]
fn markdown_to_pdf_renders_structure_links_and_tables() {
    let output =
        markdown_to_pdf_with_report(MARKDOWN.as_bytes(), &MarkdownToPdfOptions::default()).unwrap();
    let engine = open(&output.pdf);
    let text = all_text(&engine);
    for needle in [
        "Report Title",
        "bold",
        "italic",
        "first item",
        "nested two",
        "Quoted wisdom",
        "println!",
        "Apple",
        "Pear",
        "image not embedded",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in {text}");
    }
    let info = engine.document_info().unwrap();
    assert_eq!(info.title.as_deref(), Some("Report Title"));
    assert!(info.tagged);
    assert!(output
        .report
        .warnings
        .iter()
        .any(|warning| warning.contains("never fetched")));
    let annotations = crate::sdk::annotation_report_json(&output.pdf, None).unwrap();
    assert!(annotations.contains("Link"), "{annotations}");
    let raw = String::from_utf8_lossy(&output.pdf);
    assert!(
        raw.contains("https://example.com/docs")
            || annotations.contains("https://example.com/docs"),
        "URI action missing"
    );
}

#[test]
fn markdown_embeds_data_uri_png_and_blocks_paths_outside_base_dir() {
    // 1x1 opaque PNG.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let markdown =
        format!("Pixel:\n\n![dot](data:image/png;base64,{png})\n\n![secret](../secret.png)\n");
    let dir = tempfile::tempdir().unwrap();
    let options = MarkdownToPdfOptions {
        base_dir: Some(dir.path().to_path_buf()),
        ..MarkdownToPdfOptions::default()
    };
    let output = markdown_to_pdf_with_report(markdown.as_bytes(), &options).unwrap();
    let engine = open(&output.pdf);
    let text = all_text(&engine);
    assert!(text.contains("image not embedded: secret"), "{text}");
    assert!(!text.contains("image not embedded: dot"), "{text}");
    assert_eq!(
        output.report.warnings.len(),
        1,
        "{:?}",
        output.report.warnings
    );
}

#[test]
fn markdown_local_image_inside_base_dir_is_embedded() {
    let dir = tempfile::tempdir().unwrap();
    let png = super::markdown_pdf::decode_base64(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    )
    .unwrap();
    std::fs::write(dir.path().join("dot.png"), png).unwrap();
    let options = MarkdownToPdfOptions {
        base_dir: Some(dir.path().to_path_buf()),
        ..MarkdownToPdfOptions::default()
    };
    let output = markdown_to_pdf_with_report(b"![dot](dot.png)", &options).unwrap();
    assert!(
        output.report.warnings.is_empty(),
        "{:?}",
        output.report.warnings
    );
    // Without a base directory nothing local is read.
    let output =
        markdown_to_pdf_with_report(b"![dot](dot.png)", &MarkdownToPdfOptions::default()).unwrap();
    assert_eq!(output.report.warnings.len(), 1);
}

#[test]
fn markdown_long_document_paginates() {
    let markdown = (1..=70)
        .map(|n| format!("Paragraph number {n} with some words to wrap across the line."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let output =
        markdown_to_pdf_with_report(markdown.as_bytes(), &MarkdownToPdfOptions::default()).unwrap();
    assert!(output.report.pages >= 2);
    assert!(all_text(&open(&output.pdf)).contains("Paragraph number 70"));
}

#[test]
fn base64_decoder_handles_padding_and_rejects_garbage() {
    assert_eq!(super::markdown_pdf::decode_base64("aGk=").unwrap(), b"hi");
    assert!(super::markdown_pdf::decode_base64("a*b").is_none());
}

// ---------------------------------------------------------------- csv

#[test]
fn csv_to_pdf_repeats_headers_on_every_page() {
    let mut csv = String::from("id,name,amount\n");
    for n in 1..=150 {
        csv.push_str(&format!("{n},Item {n},{}\n", n * 3));
    }
    let output = csv_to_pdf_with_report(csv.as_bytes(), &CsvToPdfOptions::default()).unwrap();
    assert!(output.report.pages >= 3, "{:?}", output.report);
    let engine = open(&output.pdf);
    for page in 1..=engine.page_count().unwrap() {
        let text = engine.get_page_text(page).unwrap();
        assert!(text.contains("amount"), "page {page} lacks header: {text}");
    }
    assert!(all_text(&engine).contains("Item 150"));
}

#[test]
fn csv_delimiter_detection_and_option() {
    assert_eq!(csv_pdf::detect_delimiter(b"a;b;c\n1;2;3\n"), b';');
    assert_eq!(csv_pdf::detect_delimiter(b"a\tb\n1\t2\n"), b'\t');
    assert_eq!(csv_pdf::detect_delimiter(b"a|b\n1|2\n"), b'|');
    assert_eq!(csv_pdf::detect_delimiter(b"a,b\n\"1,5\",2\n"), b',');
    let options = CsvToPdfOptions {
        delimiter: Some(";".into()),
        ..CsvToPdfOptions::default()
    };
    let text = all_text(&open(&csv_to_pdf(b"x;y\nleft;right\n", &options).unwrap()));
    assert!(text.contains("left") && text.contains("right"));
    let bad = CsvToPdfOptions {
        delimiter: Some("ab".into()),
        ..CsvToPdfOptions::default()
    };
    assert_eq!(
        native_error_code(&csv_to_pdf(b"a", &bad).unwrap_err()),
        codes::INVALID_OPTIONS
    );
}

#[test]
fn csv_column_cap_is_an_explicit_error() {
    let header = (1..=12)
        .map(|n| format!("c{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let options = CsvToPdfOptions {
        max_columns: 10,
        ..CsvToPdfOptions::default()
    };
    let error = csv_to_pdf(format!("{header}\n").as_bytes(), &options).unwrap_err();
    assert_eq!(native_error_code(&error), codes::CSV_COLUMN_LIMIT_EXCEEDED);
    assert!(matches!(error, WellfriendError::ResourceLimit(_)));
}

#[test]
fn wide_csv_switches_to_landscape_and_wraps() {
    let header = (1..=14)
        .map(|n| format!("Column heading number {n}"))
        .collect::<Vec<_>>()
        .join(",");
    let row = (1..=14)
        .map(|n| format!("value {n} with extra words"))
        .collect::<Vec<_>>()
        .join(",");
    let output = csv_to_pdf_with_report(
        format!("{header}\n{row}\n").as_bytes(),
        &CsvToPdfOptions::default(),
    )
    .unwrap();
    let engine = open(&output.pdf);
    let mb = media_box(&engine, 1);
    assert!(mb[2] > mb[3], "expected landscape: {mb:?}");
    assert!(output
        .report
        .warnings
        .iter()
        .any(|w| w.contains("landscape")));
    assert!(compact(&all_text(&engine)).contains("value14"));
    let portrait = CsvToPdfOptions {
        orientation: CsvOrientation::Portrait,
        ..CsvToPdfOptions::default()
    };
    let output = csv_to_pdf(format!("{header}\n{row}\n").as_bytes(), &portrait).unwrap();
    let mb = media_box(&open(&output), 1);
    assert!(mb[2] < mb[3]);
}

#[test]
fn csv_too_wide_even_at_minimum_width_errors() {
    let header = (1..=200)
        .map(|n| format!("c{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let options = CsvToPdfOptions {
        max_columns: 200,
        font_size: 22.0,
        ..CsvToPdfOptions::default()
    };
    let error = csv_to_pdf(format!("{header}\n").as_bytes(), &options).unwrap_err();
    assert_eq!(native_error_code(&error), codes::CSV_TABLE_TOO_WIDE);
}

#[test]
fn fit_columns_shrinks_only_slack() {
    let widths = csv_pdf::fit_columns(&[100.0, 300.0], 50.0, 300.0).unwrap();
    assert!((widths.iter().sum::<f64>() - 300.0).abs() < 1e-6);
    assert!(widths[0] >= 50.0 && widths[1] > widths[0]);
    assert!(csv_pdf::fit_columns(&[100.0; 10], 50.0, 400.0).is_none());
}

#[test]
fn empty_csv_is_rejected() {
    let error = csv_to_pdf(b"\n\n", &CsvToPdfOptions::default()).unwrap_err();
    assert_eq!(native_error_code(&error), codes::EMPTY_INPUT);
}

// ---------------------------------------------------------------- json

#[test]
fn json_to_pdf_renders_tree_and_uniform_tables() {
    let json = r#"{
        "name": "Wellfriend",
        "version": 2,
        "active": true,
        "nothing": null,
        "nested": {"deep": {"leaf": "value"}},
        "tags": ["a", "b"],
        "people": [
            {"first": "Ada", "age": 36},
            {"first": "Grace", "age": 45}
        ]
    }"#;
    let output = json_to_pdf_with_report(json.as_bytes(), &JsonToPdfOptions::default()).unwrap();
    let engine = open(&output.pdf);
    let text = all_text(&engine);
    for needle in [
        "\"name\"",
        "Wellfriend",
        "\"leaf\"",
        "true",
        "null",
        "Grace",
        "45",
        "2 objects",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in {text}");
    }
    let tree = JsonToPdfOptions {
        tables: false,
        ..JsonToPdfOptions::default()
    };
    let text = all_text(&open(&json_to_pdf(json.as_bytes(), &tree).unwrap()));
    assert!(text.contains("\"first\"") && !text.contains("2 objects"));
}

#[test]
fn json_errors_are_coded() {
    let error = json_to_pdf(b"{\"a\":", &JsonToPdfOptions::default()).unwrap_err();
    assert_eq!(native_error_code(&error), codes::JSON_PARSE_ERROR);
    let error = json_to_pdf(b"  ", &JsonToPdfOptions::default()).unwrap_err();
    assert_eq!(native_error_code(&error), codes::EMPTY_INPUT);
}

#[test]
fn large_json_array_paginates() {
    let items = (0..260)
        .map(|n| format!("{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let output = json_to_pdf_with_report(
        format!("[{items}]").as_bytes(),
        &JsonToPdfOptions::default(),
    )
    .unwrap();
    assert!(output.report.pages > 3);
    assert_eq!(open(&output.pdf).page_count().unwrap(), output.report.pages);
}

// ---------------------------------------------------------------- svg

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="200" viewBox="0 0 400 200">
  <script>alert('ignored')</script>
  <rect x="10" y="10" width="380" height="180" fill="#3366cc"/>
  <text x="40" y="110" font-family="sans-serif" font-size="32" fill="white">Vector SVG</text>
  <image href="file:///etc/passwd" x="0" y="0" width="10" height="10"/>
  <image href="https://example.com/remote.png" x="0" y="0" width="10" height="10"/>
</svg>"##;

#[test]
fn svg_to_pdf_natural_size_is_vector_with_text() {
    let output = svg_to_pdf_with_report(SVG.as_bytes(), &SvgToPdfOptions::default()).unwrap();
    assert_eq!(output.report.pages, 1);
    let engine = open(&output.pdf);
    assert_eq!(engine.page_count().unwrap(), 1);
    let mb = media_box(&engine, 1);
    assert!(
        (mb[2] - 300.0).abs() < 0.5 && (mb[3] - 150.0).abs() < 0.5,
        "{mb:?}"
    );
    assert!(compact(&all_text(&engine)).contains("VectorSVG"));
    assert!(engine
        .document_info()
        .unwrap()
        .creator
        .unwrap_or_default()
        .contains("svg-to-pdf"));
}

#[test]
fn svg_to_pdf_fits_page_size() {
    let options = SvgToPdfOptions {
        page: SvgPageMode::Fit,
        page_size: NativePageSize::Letter,
        title: Some("Diagram".into()),
        ..SvgToPdfOptions::default()
    };
    let output = svg_to_pdf(SVG.as_bytes(), &options).unwrap();
    let engine = open(&output);
    let mb = media_box(&engine, 1);
    // Wider than tall: auto-oriented landscape Letter.
    assert!(
        (mb[2] - 792.0).abs() < 0.5 && (mb[3] - 612.0).abs() < 0.5,
        "{mb:?}"
    );
    assert_eq!(
        engine.document_info().unwrap().title.as_deref(),
        Some("Diagram")
    );
}

#[test]
fn svg_errors_are_coded() {
    let error = svg_to_pdf(b"<not-svg>", &SvgToPdfOptions::default()).unwrap_err();
    assert_eq!(native_error_code(&error), codes::SVG_PARSE_ERROR);
    let error = svg_to_pdf(b"   ", &SvgToPdfOptions::default()).unwrap_err();
    assert_eq!(native_error_code(&error), codes::EMPTY_INPUT);
}
