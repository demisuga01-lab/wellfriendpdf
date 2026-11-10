//! The default reading-order text (`get_page_text_reading_order`, the
//! `extract-text` default) follows Poppler's `pdftotext`: per-glyph words,
//! accent merging, dehyphenation, rotation pools, fake-bold removal and the
//! Breuel block order. The previous XY-cut order stays available.

use std::path::PathBuf;

use wellfriendpdf_engine::text::FlowLayoutOptions;
use wellfriendpdf_engine::ContentEngine;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn corpus(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("corpus")
        .join("pdfs")
        .join(path)
}

/// A one-page PDF (612 × 792) with a Helvetica font `F1` and `content`.
fn page_pdf(content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, body).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

#[test]
fn tracemonkey_hyphenated_words_are_joined_and_accents_merged() {
    let engine = ContentEngine::open_path(fixture("tracemonkey.pdf")).unwrap();
    // Page 9 breaks "inter-preter" across lines in the old outputs.
    let xycut = engine.get_page_text_xycut_order(9).unwrap();
    assert!(xycut.contains("inter-\npreter"), "fixture assumption");
    let text = engine.get_page_text_reading_order(9).unwrap();
    assert!(!text.contains("inter-\npreter"), "{text}");
    assert!(text.contains("interpreter"));
    // Page 2 spells "naïve" with a dotless i and a spacing dieresis.
    let text = engine.get_page_text_reading_order(2).unwrap();
    assert!(text.contains("na\u{00EF}ve"), "accent not merged");
    assert!(!text.contains('\u{00A8}'), "spacing dieresis left over");
}

#[test]
fn tracemonkey_first_page_reads_title_then_abstract_columns() {
    let engine = ContentEngine::open_path(fixture("tracemonkey.pdf")).unwrap();
    let text = engine.get_page_text_reading_order(1).unwrap();
    let title = text.find("Trace-based Just-in-Time").expect("title");
    let abstract_ = text.find("Abstract").expect("abstract heading");
    let intro = text.find("Introduction").expect("introduction heading");
    assert!(title < abstract_ && abstract_ < intro, "{text}");
}

#[test]
fn rotated_label_is_one_line_in_its_own_pool() {
    let engine = ContentEngine::open_path(corpus("generated/generated_rotated_text.pdf")).unwrap();
    let page = engine
        .analyze_page_flows(1, &FlowLayoutOptions::default())
        .unwrap();
    let rotated: Vec<_> = page
        .flows
        .iter()
        .flat_map(|flow| flow.blocks.iter())
        .filter(|block| block.rotation != 0)
        .collect();
    assert_eq!(rotated.len(), 1, "{page:#?}");
    assert_eq!(rotated[0].rotation, 270);
    assert_eq!(rotated[0].lines.len(), 1);
    assert_eq!(rotated[0].lines[0].text, "Rotated text sample");
    assert_eq!(page.primary_rotation, 0);
}

#[test]
fn overprinted_fake_bold_is_extracted_once() {
    let content = "BT /F1 12 Tf 72 700 Td (Bold heading) Tj ET \
                   BT /F1 12 Tf 72.25 700 Td (Bold heading) Tj ET \
                   BT /F1 12 Tf 72.5 700.1 Td (Bold heading) Tj ET \
                   BT /F1 10 Tf 72 680 Td (Body text follows.) Tj ET";
    let engine = ContentEngine::open_bytes(page_pdf(content)).unwrap();
    let text = engine.get_page_text_reading_order(1).unwrap();
    assert_eq!(text.matches("Bold heading").count(), 1, "{text}");
    assert!(text.contains("Body text follows."));
    // The XY-cut order drops the copies too.
    let xycut = engine.get_page_text_xycut_order(1).unwrap();
    assert_eq!(xycut.matches("Bold heading").count(), 1, "{xycut}");
}

#[test]
fn character_spacing_is_not_part_of_the_glyph_box() {
    // Letter-spaced heading (Tc 2) and a normal line: the heading stays one
    // word sequence with its real spaces, and no spurious spaces appear.
    let content = "BT /F1 12 Tf 2 Tc 72 700 Td (WIDE TITLE) Tj ET \
                   BT /F1 12 Tf 0 Tc 72 680 Td (normal words here) Tj ET";
    let engine = ContentEngine::open_bytes(page_pdf(content)).unwrap();
    let text = engine.get_page_text_reading_order(1).unwrap();
    assert_eq!(text, "WIDE TITLE\nnormal words here");
}

#[test]
fn diagonal_text_can_be_discarded() {
    let content = "BT /F1 12 Tf 72 700 Td (Straight line) Tj ET \
                   BT /F1 12 Tf 0.7071 0.7071 -0.7071 0.7071 200 300 Tm (DRAFT) Tj ET";
    let engine = ContentEngine::open_bytes(page_pdf(content)).unwrap();
    let all = engine.get_page_text_reading_order(1).unwrap();
    assert!(all.contains("Straight line"));
    let options = FlowLayoutOptions {
        discard_diagonal: true,
        ..Default::default()
    };
    let straight = engine
        .get_page_text_reading_order_with(1, &options)
        .unwrap();
    assert_eq!(straight, "Straight line");
}
