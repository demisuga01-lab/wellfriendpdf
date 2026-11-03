//! Page-parallel conversions produce the serial output.
//!
//! Every conversion (canonical parse, editable Markdown and JSON, DOCX, XLSX,
//! EPUB) runs over the whole repository corpus on a one-thread pool, where
//! every page-parallel stage takes its serial path, and on multi-thread
//! pools. The bytes must be identical. Also covered: the single-read page
//! pass falling back to the engine's own entry points, the empty structure
//! tree fallback, the scaled content-operation limit, and incremental
//! rotation.

use std::io::Write;
use std::path::{Path, PathBuf};

use wellfriendpdf_engine::editable::EditableBuildOptions;
use wellfriendpdf_engine::epub::{pdf_to_epub, EpubOptions};
use wellfriendpdf_engine::page_parallel::with_threads;
use wellfriendpdf_engine::parse::ParseOptions;
use wellfriendpdf_engine::structural::Rotation;
use wellfriendpdf_engine::{ContentEngine, DocxOptions, XlsxLayout, XlsxOptions};

fn corpus() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in [
        root.join("tests/fixtures"),
        root.join("../../tests/corpus/pdfs/existing"),
        root.join("../../tests/corpus/pdfs/generated"),
        root.join("../../tests/corpus/pdfs/pdfjs"),
    ] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "pdf") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Every conversion's bytes for one document, or the error each produced.
fn conversions(path: &Path) -> Vec<(&'static str, Result<Vec<u8>, String>)> {
    let Ok(engine) = ContentEngine::open_path(path) else {
        return Vec::new();
    };
    let err = |e: wellfriendpdf_engine::WellfriendError| e.to_string();
    let parse = engine
        .parse_document(&ParseOptions::default())
        .map(|doc| doc.to_json().into_bytes())
        .map_err(err);
    let editable = engine
        .build_editable_document(&EditableBuildOptions::default())
        .map_err(err);
    let markdown = editable
        .as_ref()
        .map(|model| model.to_markdown().into_bytes())
        .map_err(Clone::clone);
    let json = editable
        .as_ref()
        .map(|model| {
            let pretty = model.to_json_pretty().expect("editable model serializes");
            assert_eq!(
                pretty,
                serde_json::to_vec_pretty(model).unwrap(),
                "{}",
                path.display()
            );
            pretty
        })
        .map_err(Clone::clone);
    let markdown_without_images = engine
        .build_editable_document(&EditableBuildOptions {
            include_images: false,
            ..EditableBuildOptions::default()
        })
        .map(|model| model.to_markdown().into_bytes())
        .map_err(err);
    let docx = wellfriendpdf_engine::pdf_to_docx(&engine, &DocxOptions::default()).map_err(err);
    let xlsx = wellfriendpdf_engine::pdf_to_xlsx(
        &engine,
        &XlsxOptions {
            layout: XlsxLayout::Tables,
        },
    )
    .map_err(err);
    let epub = pdf_to_epub(&engine, &EpubOptions::default()).map_err(err);
    vec![
        ("parse", parse),
        ("markdown", markdown),
        ("markdown-without-images", markdown_without_images),
        ("json", json),
        ("docx", docx),
        ("xlsx", xlsx),
        ("epub", epub),
    ]
}

#[test]
fn conversions_are_identical_on_any_pool_size() {
    let files = corpus();
    assert!(files.len() >= 20, "corpus missing: {} files", files.len());
    let mut compared = 0usize;
    for path in &files {
        let serial = with_threads(Some(1), || conversions(path)).unwrap();
        for threads in [3, 8] {
            let parallel = with_threads(Some(threads), || conversions(path)).unwrap();
            assert_eq!(serial.len(), parallel.len(), "{}", path.display());
            for ((name, a), (_, b)) in serial.iter().zip(&parallel) {
                assert!(
                    a == b,
                    "{} {name}: {threads}-thread output differs from serial",
                    path.display()
                );
            }
        }
        compared += serial.len();
    }
    assert!(compared > 100, "too few conversions compared: {compared}");
}

#[test]
fn markdown_does_not_depend_on_page_image_lookup() {
    for path in corpus() {
        let outputs = conversions(&path);
        let get = |name: &str| outputs.iter().find(|(n, _)| *n == name).map(|(_, v)| v);
        if let (Some(a), Some(b)) = (get("markdown"), get("markdown-without-images")) {
            assert_eq!(a, b, "{}", path.display());
        }
    }
}

/// A minimal PDF assembled from object bodies, with a correct xref table.
fn assemble_pdf(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
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
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(b"\nendstream");
    body
}

fn text_page_pdf(catalog_extra: &str, extra_objects: &[Vec<u8>], content: &[u8]) -> Vec<u8> {
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>").into_bytes(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
        stream("", content),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    objects.extend_from_slice(extra_objects);
    assemble_pdf(&objects)
}

const TEXT: &[u8] = b"BT /F1 24 Tf 72 700 Td (Quarterly report) Tj ET\nBT /F1 11 Tf 72 660 Td (Revenue grew in every region this quarter.) Tj ET";

/// A structure tree with no content (an empty `Document` element) used to
/// make every conversion empty; the pages are now laid out instead.
#[test]
fn empty_structure_tree_falls_back_to_layout() {
    let tagged = text_page_pdf(
        "/StructTreeRoot 6 0 R /MarkInfo << /Marked true >>",
        &[
            b"<< /Type /StructTreeRoot /K 7 0 R >>".to_vec(),
            b"<< /Type /StructElem /S /Document /P 6 0 R >>".to_vec(),
        ],
        TEXT,
    );
    let untagged = text_page_pdf("", &[], TEXT);
    let markdown = |bytes: Vec<u8>| {
        ContentEngine::open_bytes(bytes)
            .unwrap()
            .build_editable_document(&EditableBuildOptions::default())
            .unwrap()
            .to_markdown()
    };
    let tagged_markdown = markdown(tagged);
    assert!(
        tagged_markdown.contains("Quarterly report"),
        "{tagged_markdown}"
    );
    assert!(
        tagged_markdown.contains("Revenue grew"),
        "{tagged_markdown}"
    );
    assert_eq!(tagged_markdown, markdown(untagged));
}

fn flate(data: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

/// A dense page whose operation count passes the fixed one-million floor
/// still parses when its stored content is large, as real map pages are.
#[test]
fn dense_page_beyond_the_fixed_floor_parses() {
    // 1.2 million path operations, stored uncompressed (stored bytes well
    // above the operation count).
    let mut content = Vec::with_capacity(16 * 1_200_000);
    for index in 0..400_000u32 {
        content.extend_from_slice(
            format!("{} {} m {} 1 l S\n", index % 600, index % 700, index % 500).as_bytes(),
        );
    }
    content.extend_from_slice(TEXT);
    let pdf = text_page_pdf("", &[], &content);
    let engine = ContentEngine::open_bytes(pdf).unwrap();
    let ops = engine.get_page_content(1).unwrap();
    assert!(ops.len() > 1_000_000, "{}", ops.len());
    let text = engine.collect_page_text_chunks(1).unwrap();
    assert!(text.iter().any(|chunk| chunk.text.contains("Quarterly")));
}

/// A decompression bomb (few stored bytes, millions of operations) keeps the
/// fixed floor and is refused.
#[test]
fn operation_bomb_is_still_refused() {
    let content = b"q Q ".repeat(600_000);
    let compressed = flate(&content);
    assert!(compressed.len() < 100_000);
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_vec(),
    ];
    objects.push(stream("/Filter /FlateDecode", &compressed));
    let engine = ContentEngine::open_bytes(assemble_pdf(&objects)).unwrap();
    let err = engine.get_page_content(1).unwrap_err().to_string();
    assert!(
        err.contains("content operation count exceeds limit 1000000"),
        "{err}"
    );
}

#[test]
fn incremental_rotation_appends_only_the_changed_pages() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tracemonkey.pdf");
    let original = std::fs::read(&path).unwrap();
    let engine = ContentEngine::open_path(&path).unwrap();
    let pages = engine.page_count().unwrap();
    let delta = wellfriendpdf_engine::rotate_pages_incremental_delta(
        &engine,
        &[2, 3],
        Rotation::Relative(90),
    )
    .unwrap();
    assert!(
        !delta.is_empty() && delta.len() < 16 * 1024,
        "{}",
        delta.len()
    );
    let mut updated = original.clone();
    updated.extend_from_slice(&delta);
    let reopened = ContentEngine::open_bytes(updated.clone()).unwrap();
    assert_eq!(reopened.page_count().unwrap(), pages);
    for page in 1..=pages {
        let expected = if page == 2 || page == 3 { 90 } else { 0 };
        assert_eq!(
            reopened.get_page(page).unwrap().rotate,
            expected,
            "page {page}"
        );
    }
    assert!(updated.starts_with(&original));
    // The same rotation as a full rewrite reads back identically.
    let full =
        wellfriendpdf_engine::rotate_pages(&engine, &[2, 3], Rotation::Relative(90)).unwrap();
    let rewritten = ContentEngine::open_bytes(full).unwrap();
    for page in 1..=pages {
        assert_eq!(
            rewritten.get_page(page).unwrap().rotate,
            reopened.get_page(page).unwrap().rotate
        );
        assert_eq!(
            rewritten.get_page_text(page).unwrap(),
            reopened.get_page_text(page).unwrap()
        );
    }
    // The in-memory incremental writer is the original followed by the delta.
    let reader = engine.document().reader();
    let page = engine.get_page(2).unwrap();
    let mut object = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap();
    if let wellfriendpdf_engine::PdfObject::Dictionary(dict) = &mut object {
        dict.insert("Rotate", wellfriendpdf_engine::PdfObject::Integer(180));
    }
    let objects = vec![wellfriendpdf_engine::IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object,
    }];
    let whole =
        wellfriendpdf_engine::writer::write_incremental_update(reader, objects.clone()).unwrap();
    let delta = wellfriendpdf_engine::incremental_update_delta(reader, objects).unwrap();
    assert_eq!(whole[..original.len()], original[..]);
    assert_eq!(&whole[original.len()..], &delta[..]);
}

/// A page image also reachable through a Form under another name, and a
/// Form image sharing the page image's name: the page's placement resolves
/// to the page's own image on every open (the resource walk used to follow a
/// randomized hash order and sometimes picked the Form's image).
#[test]
fn page_image_placements_resolve_to_the_page_resources() {
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R /Fm0 6 0 R /Fm1 6 0 R /A 6 0 R /Z 6 0 R >> >> >>".to_vec(),
        stream("", b"q 100 0 0 100 10 10 cm /Im0 Do Q /Fm0 Do"),
        stream("/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8", b"\x00"),
        stream("/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Im0 7 0 R /Alias 5 0 R >> >>", b"/Im0 Do"),
        stream("/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8", b"\xff"),
    ];
    let pdf = assemble_pdf(&objects);
    for _ in 0..24 {
        let engine = ContentEngine::open_bytes(pdf.clone()).unwrap();
        let region = wellfriendpdf_engine::PageRegion::new(0.0, 0.0, 200.0, 200.0).unwrap();
        let placed = engine.find_page_images_in_region(1, region).unwrap();
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].image.object_number, 5);
    }
}

/// Package images: a baseline JPEG is copied byte for byte, a PNG-predicted
/// Flate image is wrapped as a PNG without re-encoding, and an image placed
/// twice is stored once.
#[test]
fn epub_images_are_passed_through_and_stored_once() {
    // 4x2 RGB rows, each with the PNG "None" filter byte the predictor uses.
    let mut rows = Vec::new();
    for y in 0..2u8 {
        rows.push(0u8);
        for x in 0..4u8 {
            rows.extend_from_slice(&[x * 60, y * 120, 200]);
        }
    }
    let flate_rgb = flate(&rows);
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 90)
        .encode(&[128u8; 8 * 8 * 3], 8, 8, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /XObject << /A 5 0 R /B 6 0 R >> >> >>".to_vec(),
        stream("", b"q 20 0 0 10 10 10 cm /A Do Q q 20 0 0 10 10 100 cm /A Do Q q 8 0 0 8 100 100 cm /B Do Q"),
        stream("/Type /XObject /Subtype /Image /Width 4 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /DecodeParms << /Predictor 15 /Colors 3 /Columns 4 /BitsPerComponent 8 >>", &flate_rgb),
        stream("/Type /XObject /Subtype /Image /Width 8 /Height 8 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", &jpeg),
    ];
    let engine = ContentEngine::open_bytes(assemble_pdf(&objects)).unwrap();
    let epub = pdf_to_epub(&engine, &EpubOptions::default()).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(epub)).unwrap();
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let images: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with("OEBPS/images/"))
        .collect();
    assert_eq!(images.len(), 2, "{names:?}");
    let read = |zip: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str| {
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name(name).unwrap(), &mut out).unwrap();
        out
    };
    let jpg = images
        .iter()
        .find(|n| n.ends_with(".jpg"))
        .expect("jpeg kept");
    assert_eq!(read(&mut zip, jpg), jpeg);
    let png_name = images
        .iter()
        .find(|n| n.ends_with(".png"))
        .expect("png wrapped");
    let png = read(&mut zip, png_name);
    let decoder = png::Decoder::new(std::io::Cursor::new(&png));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (4, 2));
    let expected: Vec<u8> = rows.chunks(13).flat_map(|row| row[1..].to_vec()).collect();
    assert_eq!(&pixels[..info.buffer_size()], &expected[..]);
    // The zlib data is the stream's own.
    assert!(png.windows(flate_rgb.len()).any(|w| w == flate_rgb));
    let opf = String::from_utf8(read(&mut zip, "OEBPS/content.opf")).unwrap();
    assert!(
        opf.contains("image/jpeg") && opf.contains("image/png"),
        "{opf}"
    );
}
