use std::fs;
use std::path::{Path, PathBuf};

use oxide_engine::{ContentEngine, PdfDocument};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn file_backed_reader_matches_in_memory_reader_for_page_content_and_text() {
    for name in ["basicapi.pdf", "form_160f.pdf", "image_only.pdf"] {
        let path = fixture(name);
        let bytes = fs::read(&path).expect("fixture is readable");

        let file_doc = PdfDocument::open_path(&path).expect("path reader opens fixture");
        let memory_doc = PdfDocument::open_bytes(bytes.clone()).expect("byte reader opens fixture");
        let file_pages = file_doc.get_pages().expect("path reader lists pages");
        let memory_pages = memory_doc.get_pages().expect("byte reader lists pages");
        assert_eq!(
            file_pages.len(),
            memory_pages.len(),
            "page count differs for {name}"
        );
        for (file_page, memory_page) in file_pages.iter().zip(&memory_pages) {
            assert_eq!(file_page.page_number, memory_page.page_number);
            assert_eq!(file_page.object_number, memory_page.object_number);
            assert_eq!(file_page.generation_number, memory_page.generation_number);
            assert_eq!(file_page.media_box, memory_page.media_box);
            assert_eq!(file_page.crop_box, memory_page.crop_box);
            assert_eq!(file_page.rotate, memory_page.rotate);
            assert_eq!(file_page.contents, memory_page.contents);
        }

        for page in 1..=file_pages.len() {
            let file_content = file_doc
                .get_page_content_bytes(page)
                .expect("path reader gets content");
            let memory_content = memory_doc
                .get_page_content_bytes(page)
                .expect("byte reader gets content");
            assert_eq!(
                file_content, memory_content,
                "content bytes differ for {name} page {page}"
            );
        }

        let file_engine = ContentEngine::open_path(&path).expect("path engine opens fixture");
        let memory_engine = ContentEngine::open_bytes(bytes).expect("byte engine opens fixture");
        assert_eq!(
            file_engine.page_count().unwrap(),
            memory_engine.page_count().unwrap()
        );
        for page in 1..=file_engine.page_count().unwrap() {
            assert_eq!(
                file_engine.get_page_text(page).unwrap(),
                memory_engine.get_page_text(page).unwrap(),
                "text differs for {name} page {page}"
            );
        }
    }
}
