//! Integration tests for `edit-metadata` (Info + XMP incremental update) and
//! `pdf-to-epub` (EPUB 3 package). Outputs are reopened with the engine's own
//! parser; EPUB XML parts are checked for well-formedness with quick-xml.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use quick_xml::events::Event;
use quick_xml::Reader;
use wellfriendpdf_engine::authoring::{FlowDocument, PageSize, TableBuilder, TableColumn};
use wellfriendpdf_engine::epub::{pdf_to_epub_with_report, EpubOptions, EpubSplit};
use wellfriendpdf_engine::metadata_edit::{edit_metadata_pdf, read_metadata_views, MetadataEdit};
use wellfriendpdf_engine::secure_mutation::EditPolicyDecision;
use wellfriendpdf_engine::{
    ContentEngine, ParagraphStyle, SignatureValidity, StandardFont, TextStyle,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn authored_book() -> Vec<u8> {
    let mut flow = FlowDocument::new(PageSize::A4, wellfriendpdf_engine::Margins::all(56.0));
    flow.builder_mut()
        .set_title("Native Book")
        .set_author("Wellfriend Tests");
    let body = TextStyle::standard(StandardFont::TimesRoman, 11.0);
    let paragraph = ParagraphStyle::new();
    flow.add_heading("Chapter One", 1).unwrap();
    flow.add_paragraph(
        "The first chapter introduces the alpha paragraph with distinctive words.",
        &body,
        &paragraph,
    )
    .unwrap();
    flow.add_list(["First point", "Second point"], false, &body, &paragraph)
        .unwrap();
    let mut table = TableBuilder::new(vec![TableColumn::new(120.0), TableColumn::new(200.0)]);
    table.set_header(["Metric", "Value"]);
    table.add_row(["Speed".to_string(), "Fast".to_string()]);
    table.add_row(["Size".to_string(), "Small".to_string()]);
    flow.add_table(&table).unwrap();
    flow.add_heading("Chapter Two", 1).unwrap();
    flow.add_paragraph(
        "The second chapter closes with the omega paragraph.",
        &body,
        &paragraph,
    )
    .unwrap();
    flow.into_builder().to_bytes().unwrap()
}

/// A classic-xref PDF with no trailer /Info and an optional catalog XMP packet.
fn bare_pdf(xmp: Option<&str>) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = Vec::new();
    let catalog = if xmp.is_some() {
        "<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R >>"
    } else {
        "<< /Type /Catalog /Pages 2 0 R >>"
    };
    objects.push(catalog.as_bytes().to_vec());
    objects.push(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec());
    objects
        .push(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".to_vec());
    let content = b"BT /F1 12 Tf 20 100 Td (Hi) Tj ET";
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"\nendstream");
    objects.push(stream);
    if let Some(xmp) = xmp {
        let mut stream = format!(
            "<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n",
            xmp.len()
        )
        .into_bytes();
        stream.extend_from_slice(xmp.as_bytes());
        stream.extend_from_slice(b"\nendstream");
        objects.push(stream);
    }
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn assert_well_formed(label: &str, xml: &str) {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0i32;
    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) => depth -= 1,
            Ok(_) => {}
            Err(err) => panic!("{label} is not well-formed XML: {err}\n{xml}"),
        }
    }
    assert_eq!(depth, 0, "{label} has unbalanced elements");
}

fn xmp_of(bytes: &[u8]) -> (BTreeMap<String, String>, String) {
    let engine = ContentEngine::open_bytes(bytes.to_vec()).unwrap();
    let (info, xmp) = read_metadata_views(&engine);
    (info, xmp.expect("XMP packet present"))
}

#[test]
fn edit_metadata_updates_info_and_xmp_consistently() {
    let input = authored_book();
    let pages_before = ContentEngine::open_bytes(input.clone())
        .unwrap()
        .page_count()
        .unwrap();
    let mut custom = BTreeMap::new();
    custom.insert(
        "Department".to_string(),
        "Research & Development".to_string(),
    );
    let edit = MetadataEdit {
        title: Some("Café Résumé".to_string()),
        author: Some("Ada Lovelace; Alan Turing".to_string()),
        subject: Some("Metadata <round trip>".to_string()),
        keywords: Some("pdf, xmp, metadata".to_string()),
        creator: Some("Wellfriend Writer".to_string()),
        producer: Some("Wellfriend PDF SDK".to_string()),
        creation_date: Some("2024-05-01T10:20:30+05:30".to_string()),
        modification_date: Some("D:20250102030405Z".to_string()),
        custom,
        ..MetadataEdit::default()
    };
    let (output, report) = edit_metadata_pdf(&input, &edit).unwrap();
    assert!(
        output.starts_with(&input),
        "incremental update keeps the prefix"
    );
    assert!(report.incremental && report.original_prefix_preserved);
    assert_eq!(
        report.signature_policy_decision,
        EditPolicyDecision::SafeIncremental
    );
    assert!(!report.signatures_present);

    let engine = ContentEngine::open_bytes(output.clone()).unwrap();
    assert_eq!(engine.page_count().unwrap(), pages_before);
    let text = engine.get_page_text(1).unwrap();
    assert!(text.contains("Chapter One"), "content intact: {text}");
    let info = engine.document_info().unwrap();
    assert_eq!(info.title.as_deref(), Some("Café Résumé"));
    assert_eq!(info.author.as_deref(), Some("Ada Lovelace; Alan Turing"));
    assert_eq!(info.subject.as_deref(), Some("Metadata <round trip>"));
    assert_eq!(info.keywords.as_deref(), Some("pdf, xmp, metadata"));
    assert_eq!(info.creator.as_deref(), Some("Wellfriend Writer"));
    assert_eq!(info.producer.as_deref(), Some("Wellfriend PDF SDK"));
    assert!(info.has_xmp_metadata);

    let (raw_info, xmp) = xmp_of(&output);
    assert_eq!(raw_info["CreationDate"], "D:20240501102030+05'30'");
    assert_eq!(raw_info["ModDate"], "D:20250102030405Z");
    assert_eq!(raw_info["Department"], "Research & Development");
    assert_well_formed("xmp", &xmp);
    for expected in [
        "<rdf:li xml:lang=\"x-default\">Café Résumé</rdf:li>",
        "<rdf:li>Ada Lovelace</rdf:li>",
        "<rdf:li>Alan Turing</rdf:li>",
        "Metadata &lt;round trip&gt;",
        "<pdf:Keywords>pdf, xmp, metadata</pdf:Keywords>",
        "<rdf:li>xmp</rdf:li>",
        "<xmp:CreatorTool>Wellfriend Writer</xmp:CreatorTool>",
        "<pdf:Producer>Wellfriend PDF SDK</pdf:Producer>",
        "<xmp:CreateDate>2024-05-01T10:20:30+05:30</xmp:CreateDate>",
        "<xmp:ModifyDate>2025-01-02T03:04:05Z</xmp:ModifyDate>",
        "<pdfx:Department>Research &amp; Development</pdfx:Department>",
    ] {
        assert!(xmp.contains(expected), "XMP missing {expected}:\n{xmp}");
    }
    // The engine's own XMP field lookup agrees with Info.
    assert_eq!(
        wellfriendpdf_engine::standards_engine::xmp_lookup(&xmp, "pdf", "Producer").as_deref(),
        Some("Wellfriend PDF SDK")
    );
}

#[test]
fn second_edit_preserves_untouched_fields_and_removes_on_empty() {
    let mut custom = BTreeMap::new();
    custom.insert("Project".to_string(), "Mark2".to_string());
    let first = MetadataEdit {
        title: Some("First".to_string()),
        subject: Some("Keep me".to_string()),
        custom,
        ..MetadataEdit::default()
    };
    let (once, _) = edit_metadata_pdf(&authored_book(), &first).unwrap();
    let mut remove = BTreeMap::new();
    remove.insert("Project".to_string(), String::new());
    let second = MetadataEdit {
        title: Some("Second".to_string()),
        author: Some(String::new()),
        custom: remove,
        ..MetadataEdit::default()
    };
    let (twice, _) = edit_metadata_pdf(&once, &second).unwrap();
    assert!(twice.starts_with(&once));
    let (info, xmp) = xmp_of(&twice);
    assert_eq!(info["Title"], "Second");
    assert_eq!(info["Subject"], "Keep me");
    assert!(!info.contains_key("Author"));
    assert!(!info.contains_key("Project"));
    assert!(xmp.contains(">Second</rdf:li>"));
    assert!(xmp.contains(">Keep me</rdf:li>"));
    assert!(!xmp.contains("First"));
    assert!(
        !xmp.contains("Wellfriend Tests"),
        "removed author left XMP: {xmp}"
    );
    assert!(!xmp.contains("pdfx:Project"));
    assert_eq!(xmp.matches("<x:xmpmeta").count(), 1);
}

#[test]
fn edit_creates_info_and_xmp_when_missing_and_keeps_foreign_xmp() {
    let input = bare_pdf(None);
    let edit = MetadataEdit {
        title: Some("Created".to_string()),
        ..MetadataEdit::default()
    };
    let (output, report) = edit_metadata_pdf(&input, &edit).unwrap();
    assert!(report.info_created && report.xmp_created);
    let (info, xmp) = xmp_of(&output);
    assert_eq!(info["Title"], "Created");
    assert!(xmp.contains(">Created</rdf:li>"));

    let packet = "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><pdfaid:part>2</pdfaid:part><pdfaid:conformance>B</pdfaid:conformance><dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">Old</rdf:li></rdf:Alt></dc:title></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>";
    let (output, report) = edit_metadata_pdf(&bare_pdf(Some(packet)), &edit).unwrap();
    assert!(!report.xmp_created);
    let (_, xmp) = xmp_of(&output);
    assert_well_formed("merged xmp", &xmp);
    assert!(xmp.contains("<pdfaid:part>2</pdfaid:part>"));
    assert!(xmp.contains("<pdfaid:conformance>B</pdfaid:conformance>"));
    assert!(!xmp.contains(">Old<"));
    assert!(xmp.contains(">Created</rdf:li>"));
}

#[test]
fn edit_rejects_bad_dates_and_custom_keys() {
    let input = authored_book();
    let bad_date = MetadataEdit {
        creation_date: Some("last tuesday".to_string()),
        ..MetadataEdit::default()
    };
    assert!(edit_metadata_pdf(&input, &bad_date).is_err());
    let mut custom = BTreeMap::new();
    custom.insert("Title".to_string(), "x".to_string());
    let reserved = MetadataEdit {
        custom,
        ..MetadataEdit::default()
    };
    assert!(edit_metadata_pdf(&input, &reserved).is_err());
    assert!(edit_metadata_pdf(&input, &MetadataEdit::default()).is_err());
}

#[test]
fn edit_on_signed_document_keeps_signature_cryptographically_valid() {
    let input = fixture("sig_valid.pdf");
    let before = ContentEngine::open_bytes(input.clone())
        .unwrap()
        .verify_signatures()
        .unwrap();
    assert_eq!(before[0].validity, SignatureValidity::Valid);
    let edit = MetadataEdit {
        title: Some("Signed and retitled".to_string()),
        ..MetadataEdit::default()
    };
    let (output, report) = edit_metadata_pdf(&input, &edit).unwrap();
    assert!(output.starts_with(&input));
    assert!(report.signatures_present);
    assert_eq!(
        report.signature_policy_decision,
        EditPolicyDecision::IncrementalWithWarning
    );
    let engine = ContentEngine::open_bytes(output).unwrap();
    let after = engine.verify_signatures().unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(
        after[0].validity,
        SignatureValidity::Valid,
        "signed bytes untouched: {:#?}",
        after[0]
    );
    assert_eq!(
        engine.document_info().unwrap().title.as_deref(),
        Some("Signed and retitled")
    );
}

fn zip_entries(bytes: &[u8]) -> Vec<(String, zip::CompressionMethod, Vec<u8>, usize)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).expect("valid zip");
    let mut out = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        let extra = entry.extra_data().map_or(0, <[u8]>::len);
        out.push((entry.name().to_string(), entry.compression(), data, extra));
    }
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// Structural EPUB 3 checks: OCF layout, OPF manifest/spine integrity,
/// nav document, and XHTML well-formedness. Returns all XHTML text.
fn check_epub(bytes: &[u8]) -> String {
    assert_eq!(&bytes[0..4], b"PK\x03\x04");
    // OCF: "mimetype" must be the first local file header, stored, no extra.
    assert_eq!(&bytes[26..28], &[8, 0], "file name length of \"mimetype\"");
    assert_eq!(
        &bytes[28..30],
        &[0, 0],
        "mimetype local header has no extra field"
    );
    assert_eq!(&bytes[30..38], b"mimetype");
    assert_eq!(&bytes[38..58], b"application/epub+zip");
    let entries = zip_entries(bytes);
    let (name, method, data, extra) = &entries[0];
    assert_eq!(name, "mimetype");
    assert_eq!(*method, zip::CompressionMethod::Stored);
    assert_eq!(data, b"application/epub+zip");
    assert_eq!(*extra, 0);
    let files: BTreeMap<String, Vec<u8>> = entries
        .into_iter()
        .map(|(name, _, data, _)| (name, data))
        .collect();
    let text = |name: &str| -> String {
        String::from_utf8(
            files
                .get(name)
                .unwrap_or_else(|| panic!("missing {name}"))
                .clone(),
        )
        .unwrap()
    };
    let container = text("META-INF/container.xml");
    assert_well_formed("container.xml", &container);
    assert!(container.contains("full-path=\"OEBPS/content.opf\""));
    let opf = text("OEBPS/content.opf");
    assert_well_formed("content.opf", &opf);
    assert!(opf.contains("version=\"3.0\""));
    assert!(opf.contains("unique-identifier=\"bookid\""));
    assert!(opf.contains("<dc:identifier id=\"bookid\">urn:uuid:"));
    assert!(opf.contains("<meta property=\"dcterms:modified\">"));

    let mut manifest_ids = BTreeMap::new();
    let mut nav_items = 0;
    for item in opf.split("<item ").skip(1) {
        let tag = &item[..item.find("/>").unwrap()];
        let id = attr(tag, "id").unwrap();
        let href = attr(tag, "href").unwrap();
        assert!(attr(tag, "media-type").is_some());
        assert!(
            files.contains_key(&format!("OEBPS/{href}")),
            "manifest item {href} is in the archive"
        );
        if attr(tag, "properties").as_deref() == Some("nav") {
            nav_items += 1;
        }
        manifest_ids.insert(id, href);
    }
    assert_eq!(nav_items, 1, "exactly one nav document");
    let spine: Vec<String> = opf
        .split("<itemref ")
        .skip(1)
        .map(|item| attr(item, "idref").unwrap())
        .collect();
    assert!(!spine.is_empty());
    for idref in &spine {
        assert!(
            manifest_ids.contains_key(idref),
            "spine idref {idref} is in the manifest"
        );
    }
    // Every archived content file is declared in the manifest.
    for name in files
        .keys()
        .filter(|n| n.starts_with("OEBPS/") && *n != "OEBPS/content.opf")
    {
        let href = name.trim_start_matches("OEBPS/");
        assert!(
            manifest_ids.values().any(|h| h == href),
            "{name} is declared in the manifest"
        );
    }

    let nav = text("OEBPS/nav.xhtml");
    assert_well_formed("nav.xhtml", &nav);
    assert!(nav.contains("epub:type=\"toc\""));
    let mut all = String::new();
    for idref in &spine {
        let href = &manifest_ids[idref];
        let xhtml = text(&format!("OEBPS/{href}"));
        assert_well_formed(href, &xhtml);
        assert!(xhtml.contains("xmlns=\"http://www.w3.org/1999/xhtml\""));
        assert!(
            nav.contains(&format!("href=\"{href}\"")),
            "nav links {href}"
        );
        for img in xhtml.split("<img ").skip(1) {
            let src = attr(img, "src").unwrap();
            assert!(
                files.contains_key(&format!("OEBPS/{src}")),
                "image {src} archived"
            );
        }
        all.push_str(&xhtml);
    }
    all
}

fn run_epubcheck_if_available(label: &str, bytes: &[u8]) {
    let Ok(probe) = std::process::Command::new("epubcheck")
        .arg("--version")
        .output()
    else {
        eprintln!("epubcheck not installed; skipping external validation for {label}");
        return;
    };
    if !probe.status.success() {
        return;
    }
    let path = std::env::temp_dir().join(format!(
        "wellfriend_epub_{}_{label}.epub",
        std::process::id()
    ));
    std::fs::write(&path, bytes).unwrap();
    let out = std::process::Command::new("epubcheck")
        .arg(&path)
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&path);
    assert!(
        out.status.success(),
        "epubcheck failed for {label}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn epub_from_structured_pdf_has_valid_package_and_text() {
    let pdf = authored_book();
    let engine = ContentEngine::open_bytes(pdf).unwrap();
    let (epub, report) = pdf_to_epub_with_report(&engine, &EpubOptions::default()).unwrap();
    let text = check_epub(&epub);
    for needle in [
        "Chapter One",
        "alpha paragraph",
        "First point",
        "Metric",
        "Small",
        "Chapter Two",
        "omega paragraph",
    ] {
        assert!(text.contains(needle), "EPUB text missing {needle}");
    }
    assert_eq!(report.title, "Native Book");
    assert!(
        report.chapters.len() >= 2,
        "split on headings: {:?}",
        report.chapters
    );
    let opf = {
        let mut archive = zip::ZipArchive::new(Cursor::new(&epub)).unwrap();
        let mut s = String::new();
        archive
            .by_name("OEBPS/content.opf")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    };
    assert!(opf.contains("<dc:title>Native Book</dc:title>"));
    assert!(opf.contains("<dc:creator>Wellfriend Tests</dc:creator>"));
    // Deterministic output.
    let (again, _) = pdf_to_epub_with_report(&engine, &EpubOptions::default()).unwrap();
    assert_eq!(epub, again);
    run_epubcheck_if_available("authored", &epub);
}

#[test]
fn epub_page_split_and_images_from_real_paper() {
    let engine = ContentEngine::open_bytes(fixture("tracemonkey.pdf")).unwrap();
    let pages = engine.page_count().unwrap();
    let options = EpubOptions {
        split: EpubSplit::Pages { n: 5 },
        ..EpubOptions::default()
    };
    let (epub, report) = pdf_to_epub_with_report(&engine, &options).unwrap();
    let text = check_epub(&epub);
    assert_eq!(report.chapters.len(), pages.div_ceil(5));
    assert!(text.contains("Trace"), "paper text present");
    assert!(text.contains("<h1>") || text.contains("<h2>") || text.contains("<p>"));
    run_epubcheck_if_available("tracemonkey", &epub);
}

#[test]
fn epub_embeds_images_and_honours_include_images() {
    let engine = ContentEngine::open_bytes(fixture("image_only.pdf")).unwrap();
    let (epub, report) = pdf_to_epub_with_report(&engine, &EpubOptions::default()).unwrap();
    let text = check_epub(&epub);
    assert_eq!(report.images, 1, "image XObject embedded");
    assert!(text.contains("<img "));
    let mut archive = zip::ZipArchive::new(Cursor::new(&epub)).unwrap();
    let png_name = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .find(|name| name.ends_with(".png"))
        .expect("png in package");
    let mut png = Vec::new();
    archive
        .by_name(&png_name)
        .unwrap()
        .read_to_end(&mut png)
        .unwrap();
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']));
    run_epubcheck_if_available("image_only", &epub);

    let without = EpubOptions {
        include_images: false,
        ..EpubOptions::default()
    };
    let (epub, report) = pdf_to_epub_with_report(&engine, &without).unwrap();
    assert_eq!(report.images, 0);
    assert!(!check_epub(&epub).contains("<img "));
}
