//! Large documents: page windows edited on their own and spliced back as an
//! incremental update (the original bytes stay a prefix of the result).

use serde_json::json;
use wellfriendpdf_editor::protocol::{apply_windows, WindowedList};
use wellfriendpdf_editor::windows;
use wellfriendpdf_engine::{AuthorPageSize, ContentEngine, PageTextLayout, PdfBuilder, TextStyle};

const SERIF: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Regular.ttf");

fn fonts_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wf-editor-fonts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("liberation-serif-regular.ttf"), SERIF).unwrap();
    dir
}

fn book(pages: usize) -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    let serif = builder.register_font_bytes("Book-Serif", SERIF.to_vec()).unwrap();
    for index in 0..pages {
        let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
        page.draw_text(format!("Chapter {} opens here.", index + 1), 72.0, 700.0, &TextStyle::new(serif, 12.0)).unwrap();
    }
    builder.to_bytes().unwrap()
}

fn windowed(page: u32, operations: serde_json::Value) -> WindowedList {
    serde_json::from_value(json!({ "version": 1, "windows": [{ "page": page, "operations": operations }] })).unwrap()
}

#[test]
fn a_window_edit_is_appended_as_an_incremental_update() {
    let source = book(6);
    let engine = ContentEngine::open_bytes(source.clone()).unwrap();
    let index = windows::index(&engine).unwrap();
    assert_eq!(index.page_count, 6);
    let window = windows::extract(&engine, 4).unwrap();
    assert!(window.len() < source.len());
    assert!(PageTextLayout::from_bytes(&window, 1).unwrap().text.contains("Chapter 4"));

    let list = windowed(4, json!([
        { "op": "addText", "page": 1, "rect": { "x": 72.0, "y": 600.0, "width": 300.0, "height": 30.0 },
          "text": "Added in a page window", "font": "liberation-serif-regular", "size": 12.0, "color": { "r": 0.0, "g": 0.0, "b": 0.0 } }
    ]));
    let (bytes, result) = apply_windows(&source, &list, fonts_dir(), false).unwrap();
    assert_eq!(result.save_mode, "INCREMENTAL");
    assert!(bytes.starts_with(&source), "the original revision is untouched");
    assert!(bytes.len() - source.len() < source.len(), "only the edited page is appended");
    let reopened = ContentEngine::open_bytes(bytes.clone()).unwrap();
    assert_eq!(reopened.page_count().unwrap(), 6);
    for page in 1..=6 {
        let text = PageTextLayout::from_bytes(&bytes, page).unwrap().text;
        assert!(text.contains(&format!("Chapter {page} opens")), "{page}: {text}");
        assert_eq!(text.contains("Added in a page window"), page == 4, "{page}: {text}");
    }
    assert!(lopdf::Document::load_mem(&bytes).is_ok(), "a second reader opens the update");
}

#[test]
fn windows_refuse_restructuring_inside_a_window() {
    // Page structure travels in the list's `structure`, never in a window;
    // a window's operations address its own page 1.
    let source = book(2);
    for operation in [
        json!({ "op": "deletePage", "page": 1 }),
        json!({ "op": "addWhiteout", "page": 2, "rect": { "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0 } }),
    ] {
        let list = windowed(1, json!([operation]));
        assert!(apply_windows(&source, &list, fonts_dir(), false).is_err(), "{operation}");
    }
}

/// `WELLFRIENDPDF_LARGE_PDF=<path>`: index and window timings on a large file.
#[test]
fn large_file_windows_are_fast() {
    let Ok(path) = std::env::var("WELLFRIENDPDF_LARGE_PDF") else {
        eprintln!("WELLFRIENDPDF_LARGE_PDF unset; skipping");
        return;
    };
    let started = std::time::Instant::now();
    let engine = ContentEngine::open_path(&path).unwrap();
    let opened = started.elapsed();
    let index = windows::index(&engine).unwrap();
    let indexed = started.elapsed();
    let middle = (index.page_count / 2).max(1) as u32;
    let window = windows::extract(&engine, middle).unwrap();
    let extracted = started.elapsed();
    eprintln!(
        "{path}: {} pages; open {:?}, index {:?}, window of page {middle} ({} bytes) {:?}",
        index.page_count, opened, indexed, window.len(), extracted
    );
    assert!(extracted.as_secs_f64() < 10.0);
    let source = std::fs::read(&path).unwrap();
    let list = windowed(middle, json!([
        { "op": "addWhiteout", "page": 1, "rect": { "x": 20.0, "y": 20.0, "width": 40.0, "height": 20.0 } }
    ]));
    let saving = std::time::Instant::now();
    let (bytes, _) = apply_windows(&source, &list, fonts_dir(), true).unwrap();
    eprintln!("save: {:?}, appended {} bytes", saving.elapsed(), bytes.len() - source.len());
    assert!(bytes.starts_with(&source));
}
