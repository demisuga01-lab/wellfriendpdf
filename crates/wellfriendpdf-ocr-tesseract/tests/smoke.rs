//! End-to-end Tesseract tests, in the **auto-skip** style of
//! `crates/engine/tests/ocr_containment.rs` rather than blanket `#[ignore]`.
//!
//! Every test runs in the normal `cargo test` pass. When the external
//! `tesseract` binary (+ the `eng` language pack) is present they exercise real
//! recognition; when it is absent they print a clear `SKIP:` line and return so
//! the suite stays green on a machine without OCR installed. This is the honest
//! external-dependency handling the OCR seam requires: no fabricated output, no
//! silent pass that hides a real failure.
//!
//! Coverage mirrors the seam's own robustness matrix, but against the *real*
//! Tesseract backend:
//! - recognition recovers rendered text with plausible geometry (coordinate merge);
//! - the full parse path OCRs a scanned PDF into the shared document model;
//! - a multi-page scan OCRs through the backend's real `max_concurrency()`
//!   bounded window and recovers every page;
//! - policy `Off` with the real engine present is byte-identical to no engine;
//! - field extraction and chunking work source-agnostically over OCR'd text.

use std::sync::Arc;

use wellfriendpdf_engine::ocr::preprocess::{preprocess, PreprocessConfig};
use wellfriendpdf_engine::{
    ContentEngine, OcrEngine, OcrImage, OcrOptions, OcrPolicy, ParseOptions, SerializeOptions,
};
use wellfriendpdf_ocr_tesseract::TesseractEngine;

/// Minimal single-page PDF with a few lines of large Helvetica text.
fn text_pdf(lines: &[&str]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let mut add = |s: String| objects.push(s.into_bytes());

    add("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string());
    add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
        .to_string());
    let mut c = String::new();
    let mut y = 720.0;
    for line in lines {
        c.push_str(&format!(
            "BT /F1 24 Tf 1 0 0 1 72 {y:.1} Tm ({line}) Tj ET\n"
        ));
        y -= 40.0;
    }
    let content = format!("<< /Length {} >>\nstream\n{}\nendstream", c.len(), c);
    add(content);
    add(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    );

    // Serialize.
    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for off in &offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
            offsets.len() + 1,
            xref
        )
        .as_bytes(),
    );
    pdf
}

/// Whether the live Tesseract backend can be constructed here. Used to auto-skip
/// (not `#[ignore]`) the recognition tests when the binary is absent.
fn tesseract() -> Option<TesseractEngine> {
    TesseractEngine::new().ok()
}

/// Build a single-page **image-only** PDF (no text layer) embedding `gray`
/// (an 8-bit DeviceGray raster of `w`×`h`) as a raw, unfiltered image stream.
/// The classifier marks such a page `Scanned`, routing it to the OCR path — a
/// faithful stand-in for a scanned document built from real rendered pixels.
fn image_only_pdf(gray: &[u8], w: u32, h: u32) -> Vec<u8> {
    image_only_pdf_pages(&[(gray, w, h)])
}

/// Build a multi-page image-only PDF, one image per page. Every page has no text
/// layer, so all classify as `Scanned` → the OCR path.
fn image_only_pdf_pages(pages: &[(&[u8], u32, u32)]) -> Vec<u8> {
    let page_w = 612.0;
    let page_h = 792.0;
    let n = pages.len();

    // Object layout: 1=catalog, 2=pages, then per page a Page object, a content
    // stream, and an image XObject (3 objects/page), assigned contiguously.
    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    // Page kids start at object 3, one Page object every 3 slots.
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i * 3)).collect();
    objects
        .push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), n).into_bytes());
    for (i, (gray, w, h)) in pages.iter().enumerate() {
        let page_obj = 3 + i * 3;
        let content_obj = page_obj + 1;
        let img_obj = page_obj + 2;
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_w} {page_h}] \
                 /Resources << /XObject << /Im0 {img_obj} 0 R >> >> /Contents {content_obj} 0 R >>"
            )
            .into_bytes(),
        );
        let content = format!("q {page_w} 0 0 {page_h} 0 0 cm /Im0 Do Q\n");
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .into_bytes(),
        );
        let mut img = format!(
            "<< /Type /XObject /Subtype /Image /Width {w} /Height {h} \
             /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} >>\nstream\n",
            gray.len()
        )
        .into_bytes();
        img.extend_from_slice(gray);
        img.extend_from_slice(b"\nendstream");
        objects.push(img);
    }

    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.7\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for off in &offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
            offsets.len() + 1,
            xref
        )
        .as_bytes(),
    );
    pdf
}

/// Render a digital text page to a grayscale raster we can re-embed as a scan.
fn render_gray(lines: &[&str], dpi: u32) -> OcrImage {
    let digital = text_pdf(lines);
    let dengine = ContentEngine::open_bytes(digital).unwrap();
    let buf = dengine.render_page(1, dpi).unwrap();
    OcrImage::from(&buf.to_raw_image())
}

#[test]
fn ocr_recovers_rendered_text_with_plausible_geometry() {
    let Some(tess) = tesseract() else {
        eprintln!("SKIP: tesseract not installed; install it + the eng pack to run this test");
        return;
    };
    let pdf = text_pdf(&["The quick brown fox", "jumps over the lazy dog"]);
    let engine = ContentEngine::open_bytes(pdf).unwrap();
    let buffer = engine.render_page(1, 300).unwrap();
    let raw = buffer.to_raw_image();
    let gray = OcrImage::from(&raw);
    let (clean, _angle) = preprocess(&gray, &PreprocessConfig::default());
    let (img_w, img_h) = (clean.width as f64, clean.height as f64);

    let page = tess.recognize(&clean, &OcrOptions::default()).unwrap();

    let text: String = page
        .words
        .iter()
        .map(|w| w.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!(
        "tesseract {} recognized {} words (mean conf {:.2}): {text}",
        tess.version().unwrap_or_default(),
        page.words.len(),
        page.mean_confidence
    );
    // Honest, fuzzy assertion: most of the distinctive words should be present.
    let hits = ["quick", "brown", "fox", "jumps", "lazy", "dog"]
        .iter()
        .filter(|w| text.contains(*w))
        .count();
    assert!(
        hits >= 4,
        "expected to recover most words, found {hits}/6 in: {text}"
    );
    assert!(page.mean_confidence > 0.5, "mean confidence too low");

    // Geometry sanity (coordinate frame contract): every reported box is inside
    // the image, well-formed (x1>x0, y1>y0), and the first line sits in the top
    // half of the page (the text starts near the top of the rendered page).
    assert!(!page.words.is_empty());
    for w in &page.words {
        let [x0, y0, x1, y1] = w.bbox;
        assert!(x1 > x0 && y1 > y0, "degenerate box: {:?}", w.bbox);
        assert!(
            x0 >= 0.0 && y0 >= 0.0 && x1 <= img_w && y1 <= img_h,
            "box {:?} out of image bounds {img_w}x{img_h}",
            w.bbox
        );
    }
    // The topmost word should be in the upper portion of the page image.
    let min_y = page
        .words
        .iter()
        .map(|w| w.bbox[1])
        .fold(f64::MAX, f64::min);
    assert!(
        min_y < img_h * 0.5,
        "expected the first line near the top; min y={min_y}, height={img_h}"
    );
}

#[test]
fn full_parse_path_ocrs_a_scanned_pdf_with_merged_positions() {
    use wellfriendpdf_engine::SourceInfo;

    let Some(_) = tesseract() else {
        eprintln!("SKIP: tesseract not installed");
        return;
    };

    // 150 DPI keeps the embedded raster modest while staying legible.
    let gray = render_gray(
        &["Scanned Heading Here", "and some body text below it"],
        150,
    );
    let scanned = image_only_pdf(&gray.gray, gray.width, gray.height);
    let engine = ContentEngine::open_bytes(scanned).unwrap();

    // The whole wired path: classify → rasterize → preprocess → OCR → shared
    // pipeline → blocks, with the real Tesseract engine injected.
    let opts = ParseOptions {
        ocr: Some(Arc::new(TesseractEngine::new().unwrap()) as Arc<dyn OcrEngine>),
        ocr_policy: OcrPolicy::Auto,
        ocr_dpi: 300,
        ..ParseOptions::default()
    };
    let doc = engine.parse_document(&opts).unwrap();

    assert_eq!(doc.source, SourceInfo::Ocr, "scanned + OCR text → Ocr");

    // Coordinate-merge verification against REAL OCR output: the recovered blocks
    // carry positive-area PDF-space bounding boxes on the page (proof the pixel
    // boxes were mapped back through the seam, not dropped).
    let page = &doc.pages[0];
    let boxed = page
        .block_ids
        .iter()
        .filter_map(|id| doc.block(*id))
        .filter(|b| {
            let r = b.bbox;
            r[2] > r[0] && r[3] > r[1]
        })
        .count();
    assert!(
        boxed > 0,
        "OCR'd blocks must carry merged positional geometry; got {} blocks",
        page.block_ids.len()
    );

    let md = doc.to_markdown(&SerializeOptions::default());
    eprintln!("--- OCR'd scanned page → markdown ---\n{md}\n---");
    let lower = md.to_lowercase();
    let hits = ["scanned", "heading", "body", "text", "below"]
        .iter()
        .filter(|w| lower.contains(*w))
        .count();
    assert!(
        hits >= 3,
        "expected the OCR'd scanned page to recover its text, found {hits}/5:\n{md}"
    );
}

#[test]
fn multi_page_scan_ocrs_through_the_backends_concurrency_window() {
    let Some(tess) = tesseract() else {
        eprintln!("SKIP: tesseract not installed");
        return;
    };
    // The real backend advertises >1 here (CPU-tied); the parse pipeline runs a
    // bounded parallel window up to that value. This exercises that path with the
    // real engine, in the ocr_containment style.
    assert!(
        tess.max_concurrency() >= 1,
        "backend must report a real concurrency"
    );

    // Four distinct scanned pages, each with its own recoverable marker word.
    let g1 = render_gray(&["AlphaMarker one"], 150);
    let g2 = render_gray(&["BetaMarker two"], 150);
    let g3 = render_gray(&["GammaMarker three"], 150);
    let g4 = render_gray(&["DeltaMarker four"], 150);
    let pdf = image_only_pdf_pages(&[
        (&g1.gray, g1.width, g1.height),
        (&g2.gray, g2.width, g2.height),
        (&g3.gray, g3.width, g3.height),
        (&g4.gray, g4.width, g4.height),
    ]);
    let engine = ContentEngine::open_bytes(pdf).unwrap();
    let opts = ParseOptions {
        ocr: Some(Arc::new(TesseractEngine::new().unwrap()) as Arc<dyn OcrEngine>),
        ocr_policy: OcrPolicy::Auto,
        ocr_dpi: 300,
        ..ParseOptions::default()
    };
    let doc = engine.parse_document(&opts).unwrap();
    assert_eq!(doc.pages.len(), 4, "all four scanned pages present");

    let md = doc.to_markdown(&SerializeOptions::default()).to_lowercase();
    // Each page's distinctive marker should survive OCR on its own page. OCR is
    // fuzzy, so require the majority rather than all four.
    let markers = ["alpha", "beta", "gamma", "delta"];
    let hits = markers.iter().filter(|m| md.contains(*m)).count();
    assert!(
        hits >= 3,
        "expected most per-page markers recovered across the concurrent window, \
         found {hits}/4:\n{md}"
    );
}

#[test]
fn policy_off_with_real_tesseract_matches_no_engine() {
    // This does NOT require tesseract: policy Off must never invoke the backend,
    // so it is byte-identical to no engine even when one is configured. We still
    // build the engine when present to make the assertion faithful; when absent
    // we assert the invariant with the engine slot empty (still Off).
    let gray = render_gray(&["Heading", "body text"], 100);
    let pdf = image_only_pdf(&gray.gray, gray.width, gray.height);

    let plain = ContentEngine::open_bytes(pdf.clone()).unwrap();
    let plain_md = plain
        .parse_document(&ParseOptions::default())
        .unwrap()
        .to_markdown(&SerializeOptions::default());

    let with_engine = ContentEngine::open_bytes(pdf).unwrap();
    let mut opts = ParseOptions {
        ocr_policy: OcrPolicy::Off, // explicitly off despite an engine
        ocr_dpi: 300,
        ..ParseOptions::default()
    };
    if let Some(t) = tesseract() {
        opts.ocr = Some(Arc::new(t) as Arc<dyn OcrEngine>);
    }
    let off_md = with_engine
        .parse_document(&opts)
        .unwrap()
        .to_markdown(&SerializeOptions::default());

    assert_eq!(
        plain_md, off_md,
        "policy Off with a real engine must be byte-identical to no engine"
    );
}

/// **Source-agnostic KV proof**: the *same* field extractor that handles digital
/// invoices recovers fields from a SCANNED, OCR'd invoice.
#[test]
fn extract_fields_on_an_ocrd_scanned_invoice() {
    use wellfriendpdf_engine::{DocType, ExtractOptions, FieldValue};

    let Some(_) = tesseract() else {
        eprintln!("SKIP: tesseract not installed");
        return;
    };

    let gray = render_gray(
        &[
            "INVOICE",
            "Invoice Number: INV-2024-0042",
            "Date: 2024-01-15",
            "Total: $486.00",
        ],
        200,
    );
    let scanned = image_only_pdf(&gray.gray, gray.width, gray.height);
    let engine = ContentEngine::open_bytes(scanned).unwrap();

    let opts = ExtractOptions {
        doc_type: Some(DocType::Invoice),
        ocr: Some(Arc::new(TesseractEngine::new().unwrap()) as Arc<dyn OcrEngine>),
        ocr_dpi: 300,
        ..Default::default()
    };
    let result = engine.extract_fields(&opts).unwrap();
    eprintln!(
        "--- fields from OCR'd scanned invoice ---\n{}",
        result.to_json()
    );

    assert_eq!(result.doc_type, DocType::Invoice);
    let total = result.get("total");
    assert!(
        total.is_some(),
        "expected a total field from the OCR'd invoice; got fields: {:?}",
        result.fields.iter().map(|f| &f.key).collect::<Vec<_>>()
    );
    if let Some(t) = total {
        assert!(
            matches!(&t.value, FieldValue::Amount { value, .. } if (*value - 486.0).abs() < 1.0),
            "total value from OCR: {:?}",
            t.value
        );
    }
}

/// **Source-agnostic chunking proof**: the same chunker produces RAG chunks from
/// an OCR'd scanned document.
#[test]
fn chunk_an_ocrd_scanned_document() {
    use wellfriendpdf_engine::ChunkOptions;

    let Some(_) = tesseract() else {
        eprintln!("SKIP: tesseract not installed");
        return;
    };

    let gray = render_gray(
        &[
            "Introduction",
            "This document was scanned and recovered by OCR.",
            "Methods",
            "We measured several things and report them below.",
        ],
        200,
    );
    let scanned = image_only_pdf(&gray.gray, gray.width, gray.height);
    let engine = ContentEngine::open_bytes(scanned).unwrap();

    let doc = engine
        .parse_document(&ParseOptions {
            ocr: Some(Arc::new(TesseractEngine::new().unwrap()) as Arc<dyn OcrEngine>),
            ocr_policy: OcrPolicy::Auto,
            ocr_dpi: 300,
            omit_furniture: false,
            ..ParseOptions::default()
        })
        .unwrap();

    let set = doc.chunk(&ChunkOptions {
        target_tokens: 100,
        ..Default::default()
    });
    eprintln!("--- chunks from OCR'd scan ---\n{}", set.to_json());

    assert!(!set.chunks.is_empty(), "OCR'd document should chunk");
    let all: String = set.chunks.iter().map(|c| c.text.to_lowercase()).collect();
    let hits = ["introduction", "scanned", "ocr", "methods", "measured"]
        .iter()
        .filter(|w| all.contains(*w))
        .count();
    assert!(
        hits >= 3,
        "expected OCR'd text in chunks, found {hits}/5:\n{all}"
    );
}
