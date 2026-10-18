//! Native PDF/A conversion: every converted fixture must pass the SDK's own
//! clause-mapped PDF/A validator, keep its page count, metadata and text, and
//! a file that cannot be made conformant must never be returned.

use wellfriendpdf_engine::{
    convert_pdf_to_pdfa, convert_to_pdfa, encrypt_pdf, validate_pdfa, validate_pdfa_profile,
    AuthorPageSize as PageSize, ContentEngine, EncryptParams, PdfAConvertOptions, PdfAProfile,
    PdfBuilder, PdfDocument, StandardFont, StandardsValidationOptions, TextStyle,
    PDFA1_TRANSPARENCY,
};

fn options(level: &str) -> PdfAConvertOptions {
    PdfAConvertOptions::for_profile(PdfAProfile::parse(level).unwrap())
}

fn page_texts(bytes: &[u8]) -> Vec<String> {
    let engine = ContentEngine::open_bytes(bytes.to_vec()).unwrap();
    (1..=engine.page_count().unwrap())
        .map(|page| engine.get_page_text(page).unwrap())
        .collect()
}

/// Convert, then check the output with the engine's own parser and validator.
fn convert_and_check(
    source: &[u8],
    level: &str,
) -> (Vec<u8>, wellfriendpdf_engine::PdfAConvertReport) {
    let doc = PdfDocument::open_bytes(source.to_vec()).unwrap();
    let outcome = convert_pdf_to_pdfa(&doc, &options(level)).unwrap();
    let report = outcome.report.clone();
    let bytes = outcome
        .bytes
        .unwrap_or_else(|| panic!("conversion to {level} failed: {report:#?}"));
    assert!(report.ok && report.validated, "{report:#?}");
    assert_eq!(report.level, level);

    // Valid PDF that reopens with the same page count and text.
    assert!(bytes.starts_with(b"%PDF-"));
    let converted = ContentEngine::open_bytes(bytes.clone()).unwrap();
    let original = ContentEngine::open_bytes(source.to_vec()).unwrap();
    assert_eq!(
        converted.page_count().unwrap(),
        original.page_count().unwrap()
    );
    assert_eq!(
        page_texts(source),
        page_texts(&bytes),
        "text changed for {level}"
    );

    // Independent re-validation through the public clause-mapped validator.
    let profile = PdfAProfile::parse(level).unwrap();
    let validation = validate_pdfa_profile(
        &converted,
        &StandardsValidationOptions::with_target(&profile.label().to_uppercase()),
    )
    .unwrap();
    let failing: Vec<_> = validation
        .rules
        .iter()
        .filter(|r| r.status.is_failing())
        .collect();
    assert!(failing.is_empty(), "{failing:#?}");
    let legacy = validate_pdfa(converted.document(), profile).unwrap();
    assert!(legacy.compliant, "{:#?}", legacy.violations);
    (bytes, report)
}

fn standard_fonts_pdf() -> Vec<u8> {
    let mut doc = PdfBuilder::new();
    doc.set_title("Archive – Ünïcode title");
    doc.set_author("Records Team");
    let page = doc.add_page(PageSize::LETTER);
    let mut y = 720.0;
    for (font, text) in [
        (StandardFont::Helvetica, "Helvetica body text, 0123456789."),
        (StandardFont::HelveticaBold, "Helvetica Bold heading"),
        (
            StandardFont::TimesRoman,
            "Times Roman paragraph with fi and fl.",
        ),
        (StandardFont::TimesItalic, "Times Italic emphasis"),
        (StandardFont::Courier, "Courier fixed width: let x = 42;"),
        (StandardFont::CourierBold, "Courier Bold"),
    ] {
        page.draw_text(text, 72.0, y, &TextStyle::standard(font, 12.0))
            .unwrap();
        y -= 24.0;
    }
    let page = doc.add_page(PageSize::A4);
    page.draw_text(
        "Second page in Helvetica Oblique.",
        72.0,
        700.0,
        &TextStyle::standard(StandardFont::HelveticaOblique, 11.0),
    )
    .unwrap();
    doc.to_bytes().unwrap()
}

/// Minimal hand-built PDF: `objects[i]` is the body of object `i + 1`; object 1
/// is the catalog.
fn raw_pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 2 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

fn obj(text: &str) -> Vec<u8> {
    text.as_bytes().to_vec()
}

/// LZW-encode without compression (literal codes, periodic clear codes).
fn lzw_literal(data: &[u8]) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::new();
    let mut push = |code: u16| {
        for i in (0..9).rev() {
            bits.push(code >> i & 1 == 1);
        }
    };
    push(256);
    for (idx, byte) in data.iter().enumerate() {
        if idx > 0 && idx % 200 == 0 {
            push(256);
        }
        push(u16::from(*byte));
    }
    push(257);
    bits.chunks(8)
        .map(|chunk| {
            chunk
                .iter()
                .enumerate()
                .fold(0u8, |acc, (i, bit)| acc | (u8::from(*bit) << (7 - i)))
        })
        .collect()
}

/// RGB + CMYK vector content, transparency, JavaScript, LZW, annotations and
/// an embedded file in one document.
fn feature_pdf(alpha: bool) -> Vec<u8> {
    let content = b"BT /F1 18 Tf 72 700 Td (Feature fixture) Tj ET\n1 0 0 rg 72 600 100 50 re f\n0 1 0 0 k 200 600 100 50 re f\n/GS1 gs 0 0 1 rg 320 600 100 50 re f\n";
    let gs = if alpha {
        "<< /Type /ExtGState /ca 0.5 /CA 0.5 >>"
    } else {
        "<< /Type /ExtGState /LW 1 >>"
    };
    let lzw_content = b"BT /F1 12 Tf 72 560 Td (LZW encoded line) Tj ET";
    raw_pdf(&[
        obj("<< /Type /Catalog /Pages 3 0 R /OpenAction 8 0 R /Names << /JavaScript << /Names [(init) 8 0 R] >> /EmbeddedFiles << /Names [(data.txt) 11 0 R] >> >> /AA << /WC 8 0 R >> >>"),
        obj("<< /Title (Feature fixture) /Producer (hand written) /CreationDate (D:20240115103000+01'00') >>"),
        obj("<< /Type /Pages /Kids [4 0 R] /Count 1 >>"),
        obj("<< /Type /Page /Parent 3 0 R /MediaBox [0 0 612 792] /Contents [5 0 R 9 0 R] /Resources << /Font << /F1 6 0 R >> /ExtGState << /GS1 7 0 R >> >> /Annots [10 0 R 13 0 R 14 0 R 15 0 R] /AA << /O 8 0 R >> >>"),
        stream("", content),
        obj("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"),
        obj(gs),
        obj("<< /Type /Action /S /JavaScript /JS (app.alert\\('hi'\\);) >>"),
        stream("/Filter /LZWDecode", &lzw_literal(lzw_content)),
        obj("<< /Type /Annot /Subtype /Link /Rect [72 690 300 720] /Border [0 0 0] /A 8 0 R >>"),
        obj("<< /Type /Filespec /F (data.txt) /EF << /F 12 0 R >> >>"),
        stream("/Type /EmbeddedFile", b"attached data"),
        obj("<< /Type /Annot /Subtype /Square /Rect [450 600 550 650] /C [1 0 0] /F 4 >>"),
        obj("<< /Type /Annot /Subtype /Text /Rect [450 700 470 720] /F 2 /Contents (hidden note) >>"),
        obj("<< /Type /Annot /Subtype /Sound /Rect [10 10 20 20] /F 4 >>"),
    ])
}

#[test]
fn standard_fonts_are_embedded_for_every_level() {
    let source = standard_fonts_pdf();
    for level in ["1b", "2b", "2u", "3b", "3u"] {
        let (bytes, report) = convert_and_check(&source, level);
        assert!(
            report.fonts.iter().all(|f| f.embedded),
            "{:#?}",
            report.fonts
        );
        assert!(
            report.fixes.fonts_embedded >= 7,
            "{level}: {:#?}",
            report.fixes
        );
        assert!(report.fixes.output_intent_added);
        let doc = PdfDocument::open_bytes(bytes.clone()).unwrap();
        for font in wellfriendpdf_engine::fonts_report::list_fonts(&doc).unwrap() {
            assert!(font.embedded && font.to_unicode, "{font:#?}");
            assert_eq!(font.font_file_kind.as_deref(), Some("FontFile2"));
        }
        let info = wellfriendpdf_engine::DocumentInfo::gather(&doc).unwrap();
        assert_eq!(info.title.as_deref(), Some("Archive – Ünïcode title"));
        assert_eq!(info.author.as_deref(), Some("Records Team"));
        let xmp = String::from_utf8_lossy(&bytes).contains("pdfaid:part");
        assert!(xmp);
        if level == "1b" {
            assert!(bytes.starts_with(b"%PDF-1.4"));
        }
    }
}

#[test]
fn embedded_widths_keep_glyph_positions() {
    let source = standard_fonts_pdf();
    let (bytes, _) = convert_and_check(&source, "2b");
    let before = ContentEngine::open_bytes(source).unwrap();
    let after = ContentEngine::open_bytes(bytes).unwrap();
    let words = |engine: &ContentEngine| {
        engine
            .extract_page_words(1)
            .unwrap()
            .into_iter()
            .map(|w| (w.text, (w.x0 * 100.0).round(), (w.x1 * 100.0).round()))
            .collect::<Vec<_>>()
    };
    let (a, b) = (words(&before), words(&after));
    assert!(!a.is_empty());
    assert_eq!(a, b, "word boxes moved after font embedding");
}

#[test]
fn forbidden_features_are_removed_and_device_colour_is_managed() {
    let source = feature_pdf(false);
    let (bytes, report) = convert_and_check(&source, "2b");
    let fixes = &report.fixes;
    assert!(
        fixes.javascript_removed + fixes.actions_removed >= 3,
        "{fixes:#?}"
    );
    assert_eq!(fixes.lzw_streams_reencoded, 1);
    assert!(fixes.embedded_files_removed >= 1);
    assert_eq!(fixes.annotations_removed, 1, "Sound annotation");
    assert!(fixes.appearances_generated >= 2, "Square + hidden Text");
    assert!(fixes
        .default_color_spaces
        .contains(&"DefaultCMYK".to_string()));
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("/JavaScript"));
    assert!(!text.contains("LZWDecode"));
    assert!(!text.contains("/EmbeddedFiles"));
    assert!(page_texts(&bytes)[0].contains("LZW encoded line"));
}

#[test]
fn pdfa3_keeps_embedded_files_with_af_relationship() {
    let (bytes, report) = convert_and_check(&feature_pdf(false), "3b");
    assert_eq!(report.fixes.embedded_files_associated, 1);
    let doc = PdfDocument::open_bytes(bytes).unwrap();
    let catalog = doc.get_catalog().unwrap();
    assert!(catalog.get("AF").is_some());
    let attachments = wellfriendpdf_engine::attachments::list_attachments(&doc).unwrap();
    assert_eq!(attachments.len(), 1);
}

#[test]
fn transparency_converts_to_pdfa2_but_refuses_pdfa1_with_clear_code() {
    let source = feature_pdf(true);
    convert_and_check(&source, "2b");
    let doc = PdfDocument::open_bytes(source).unwrap();
    let outcome = convert_pdf_to_pdfa(&doc, &options("1b")).unwrap();
    assert!(outcome.bytes.is_none());
    assert!(!outcome.report.ok);
    assert_eq!(outcome.report.code.as_deref(), Some(PDFA1_TRANSPARENCY));
    assert!(outcome
        .report
        .failed_rules
        .iter()
        .any(|r| r.rule == "pdfa1.transparency"));
}

#[test]
fn rgb_only_content_gets_srgb_output_intent() {
    let content = b"0.2 0.4 0.6 rg 10 10 200 200 re f 1 0 0 RG 5 w 50 50 m 150 150 l S";
    let source = raw_pdf(&[
        obj("<< /Type /Catalog /Pages 3 0 R >>"),
        obj("<< /Title (RGB only) >>"),
        obj("<< /Type /Pages /Kids [4 0 R] /Count 1 >>"),
        obj("<< /Type /Page /Parent 3 0 R /MediaBox [0 0 300 300] /Contents 5 0 R /Resources << >> >>"),
        stream("", content),
    ]);
    let (_, report) = convert_and_check(&source, "2b");
    assert!(report.fixes.output_intent_added);
    assert!(report.fixes.default_color_spaces.is_empty());
    assert!(report.fonts.is_empty());
}

#[test]
fn encrypted_source_is_decrypted() {
    let source = standard_fonts_pdf();
    let engine = ContentEngine::open_bytes(source.clone()).unwrap();
    let encrypted = encrypt_pdf(
        &engine,
        &EncryptParams {
            user_password: wellfriendpdf_engine::crypto::secret_bytes(b"u".to_vec()),
            owner_password: wellfriendpdf_engine::crypto::secret_bytes(b"o".to_vec()),
            ..EncryptParams::default()
        },
    )
    .unwrap();
    let doc = PdfDocument::open_bytes_with_password(encrypted, b"u").unwrap();
    let outcome = convert_pdf_to_pdfa(&doc, &options("2b")).unwrap();
    assert!(outcome.report.ok, "{:#?}", outcome.report);
    assert!(outcome.report.fixes.decrypted);
    let bytes = outcome.bytes.unwrap();
    let reopened = PdfDocument::open_bytes(bytes.clone()).unwrap();
    assert!(!reopened.reader().is_encrypted());
    assert_eq!(page_texts(&bytes), page_texts(&source));
}

#[test]
fn legacy_api_now_embeds_fonts() {
    let doc = PdfDocument::open_bytes(standard_fonts_pdf()).unwrap();
    let bytes = convert_to_pdfa(&doc, PdfAProfile::PdfA2B).unwrap();
    let converted = PdfDocument::open_bytes(bytes).unwrap();
    assert!(
        validate_pdfa(&converted, PdfAProfile::PdfA2B)
            .unwrap()
            .compliant
    );
}

#[test]
fn unicode_level_rejects_fonts_without_unicode_mapping() {
    // A composite font with Identity-H and no ToUnicode has no Unicode mapping
    // and no way to pick substitute glyphs: the gate must refuse, not claim.
    let content = b"BT /F1 12 Tf 72 700 Td <00410042> Tj ET";
    let source = raw_pdf(&[
        obj("<< /Type /Catalog /Pages 3 0 R >>"),
        obj("<< /Title (Identity) >>"),
        obj("<< /Type /Pages /Kids [4 0 R] /Count 1 >>"),
        obj("<< /Type /Page /Parent 3 0 R /MediaBox [0 0 612 792] /Contents 5 0 R /Resources << /Font << /F1 6 0 R >> >> >>"),
        stream("", content),
        obj("<< /Type /Font /Subtype /Type0 /BaseFont /MysteryFont /Encoding /Identity-H /DescendantFonts [7 0 R] >>"),
        obj("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /MysteryFont /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 500 /FontDescriptor 8 0 R >>"),
        obj("<< /Type /FontDescriptor /FontName /MysteryFont /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>"),
    ]);
    let doc = PdfDocument::open_bytes(source).unwrap();
    let outcome = convert_pdf_to_pdfa(&doc, &options("2u")).unwrap();
    assert!(
        outcome.bytes.is_none(),
        "must not return an unvalidated file"
    );
    assert_eq!(
        outcome.report.code.as_deref(),
        Some("PDFA_CONVERSION_FAILED")
    );
    assert!(outcome
        .report
        .failed_rules
        .iter()
        .any(|r| r.rule.contains("font")));
    assert!(outcome.report.fonts.iter().any(|f| f.failure.is_some()));
}
