//! OCR text on scanned pages: an invisible text layer changes no pixel and
//! puts every word at its box; converted text keeps the scan's look (pixels
//! change only inside the converted lines) and is editable afterwards.
//!
//! The scanned page is made here: a born-digital page (paper-coloured, dark
//! brown ink, Liberation Serif) is rendered at 150 dpi, a little noise is
//! added, and the raster becomes the only content of a new page. The words
//! and boxes of the born-digital page stand in for an OCR engine's result.

use serde_json::json;
use wellfriendpdf_editor::text_layer::{OcrLine, OcrWord};
use wellfriendpdf_editor::ops::Rect;
use wellfriendpdf_editor::{apply_operations, MemoryFonts, OperationList};
use wellfriendpdf_engine::{
    AuthorPageSize, Color, ContentEngine, GraphicsStyle, PageTextLayout, PdfBuilder, TextStyle,
};

const SERIF: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Regular.ttf");
const SANS: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Regular.ttf");

const PAPER: [f64; 3] = [0.96, 0.94, 0.89];
const INK: [f64; 3] = [0.20, 0.17, 0.14];
const LINES: [(&str, f64); 6] = [
    ("Quarterly Inspection Report", 20.0),
    ("The north wing roof was inspected on 4 March.", 12.0),
    ("Two cracked tiles were replaced and the gutters", 12.0),
    ("were cleared of leaves and moss before the rain.", 12.0),
    ("No water damage was found in the attic rooms.", 12.0),
    ("Next inspection: early September, weather permitting.", 12.0),
];

fn library() -> MemoryFonts {
    let mut fonts = MemoryFonts::new();
    fonts.insert("liberation-serif-regular", SERIF.to_vec());
    fonts.insert("liberation-sans-regular", SANS.to_vec());
    // The layer's default face; any library face works for invisible text.
    fonts.insert("noto-sans-regular", SANS.to_vec());
    fonts
}

fn list(operations: serde_json::Value) -> OperationList {
    serde_json::from_value(json!({ "version": 1, "operations": operations })).unwrap()
}

/// With `WF_EDITOR_TEST_OUT` set, write `bytes` there for a visual check.
fn keep(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("WF_EDITOR_TEST_OUT") {
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.pdf")), bytes).unwrap();
    }
}

/// The born-digital page the scan is made from.
fn born_digital() -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    let serif = builder.register_font_bytes("LiberationSerif", SERIF.to_vec()).unwrap();
    let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
    let paper = GraphicsStyle { stroke: None, fill: Some(Color::device_rgb(PAPER[0], PAPER[1], PAPER[2])), ..GraphicsStyle::default() };
    page.draw_rect(0.0, 0.0, 612.0, 792.0, &paper);
    let mut y = 700.0;
    for (text, size) in LINES {
        let style = TextStyle::new(serif, size).fill(Color::device_rgb(INK[0], INK[1], INK[2]));
        page.draw_text(text, 72.0, y, &style).unwrap();
        y -= size * 2.0;
    }
    builder.to_bytes().unwrap()
}

fn render(bytes: &[u8], dpi: u32) -> (usize, usize, Vec<u8>) {
    let raw = ContentEngine::open_bytes(bytes.to_vec()).unwrap().render_page(1, dpi).unwrap().into_raw_image();
    assert_eq!(raw.channels, 3);
    (raw.width as usize, raw.height as usize, raw.pixels)
}

/// An image-only page: the born-digital page rendered at 150 dpi with a
/// little deterministic noise, placed as the page's only content.
fn scanned(source: &[u8]) -> Vec<u8> {
    let (width, height, mut pixels) = render(source, 150);
    let mut state = 0x2545_f491_u32;
    for value in pixels.iter_mut() {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let noise = (state % 7) as i32 - 3;
        *value = (i32::from(*value) + noise).clamp(0, 255) as u8;
    }
    let mut builder = PdfBuilder::new();
    let image = builder.add_rgb_image(width as u32, height as u32, pixels).unwrap();
    builder.add_page(AuthorPageSize::custom(612.0, 792.0)).draw_image(image, 0.0, 0.0, 612.0, 792.0);
    builder.to_bytes().unwrap()
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn rect(b: [f64; 4]) -> Rect {
    Rect { x: b[0], y: b[1], width: b[2] - b[0], height: b[3] - b[1] }
}

/// What an OCR engine would report for the scan: the born-digital lines and
/// words with their boxes.
fn truth(source: &[u8]) -> Vec<OcrLine> {
    let layout = PageTextLayout::from_bytes(source, 1).unwrap();
    let mut lines = Vec::new();
    for line in &layout.lines {
        let mut words: Vec<OcrWord> = Vec::new();
        let mut current: Option<(String, [f64; 4])> = None;
        for glyph in &layout.glyphs[line.glyphs[0]..line.glyphs[1]] {
            if glyph.text.trim().is_empty() {
                if let Some((text, bbox)) = current.take() {
                    words.push(OcrWord { text, rect: rect(bbox) });
                }
                continue;
            }
            current = Some(match current.take() {
                None => (glyph.text.clone(), glyph.bbox),
                Some((text, bbox)) => (text + &glyph.text, union(bbox, glyph.bbox)),
            });
        }
        if let Some((text, bbox)) = current.take() {
            words.push(OcrWord { text, rect: rect(bbox) });
        }
        if words.is_empty() {
            continue;
        }
        let bbox = words.iter().fold(None::<[f64; 4]>, |acc, w| {
            let b = [w.rect.x, w.rect.y, w.rect.x + w.rect.width, w.rect.y + w.rect.height];
            Some(acc.map_or(b, |a| union(a, b)))
        });
        let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        lines.push(OcrLine { text, rect: rect(bbox.unwrap()), words, color: None, background: None });
    }
    assert_eq!(lines.len(), LINES.len(), "{:?}", lines.iter().map(|l| &l.text).collect::<Vec<_>>());
    lines
}

fn fixture() -> (Vec<u8>, Vec<OcrLine>) {
    let source = born_digital();
    let scan = scanned(&source);
    keep("ocr-scanned-page", &scan);
    let lines = truth(&source);
    (scan, lines)
}

#[test]
fn the_fixture_is_a_scan_without_text() {
    let (scan, _) = fixture();
    let layout = PageTextLayout::from_bytes(&scan, 1).unwrap();
    assert!(layout.glyphs.iter().all(|g| g.text.trim().is_empty()), "the scan has text: {:?}", layout.text);
}

#[test]
fn an_invisible_text_layer_changes_no_pixel_and_puts_each_word_at_its_box() {
    let (scan, lines) = fixture();
    let operations = list(json!([{ "op": "addTextLayer", "page": 1, "lines": lines }]));
    let outcome = apply_operations(&scan, &operations, &library(), false).unwrap();
    keep("ocr-text-layer", &outcome.bytes);
    assert_eq!(outcome.reports[0].op, "addTextLayer");

    let (w, h, before) = render(&scan, 100);
    let (w2, h2, after) = render(&outcome.bytes, 100);
    assert_eq!((w, h), (w2, h2));
    let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    assert_eq!(changed, 0, "{changed} rendered samples changed under an invisible layer");

    let layout = PageTextLayout::from_bytes(&outcome.bytes, 1).unwrap();
    let visible: Vec<_> = layout.glyphs.iter().filter(|g| !g.text.trim().is_empty()).collect();
    assert!(!visible.is_empty());
    assert!(visible.iter().all(|g| g.invisible), "every layer glyph is rendering mode 3");
    for line in &lines {
        assert!(layout.text.contains(&line.text), "layout text lacks {:?}: {:?}", line.text, layout.text);
    }
    // Each word's glyphs sit inside (within 1.5 pt) the word's box.
    let text: Vec<char> = layout.text.chars().collect();
    let mut cursor = 0usize;
    for word in lines.iter().flat_map(|l| &l.words) {
        let target: Vec<char> = word.text.chars().collect();
        let start = (cursor..=text.len() - target.len())
            .find(|&i| text[i..i + target.len()] == target[..])
            .unwrap_or_else(|| panic!("word {:?} not found", word.text));
        cursor = start + target.len();
        let glyphs: Vec<_> = layout
            .glyphs
            .iter()
            .filter(|g| g.range[0] >= start && g.range[1] <= cursor && !g.text.trim().is_empty())
            .collect();
        assert!(!glyphs.is_empty(), "no glyphs for {:?}", word.text);
        let bbox = glyphs.iter().fold(glyphs[0].bbox, |acc, g| union(acc, g.bbox));
        let r = word.rect;
        let tolerance = 1.5;
        assert!(
            (bbox[0] - r.x).abs() < tolerance && (bbox[2] - (r.x + r.width)).abs() < tolerance,
            "word {:?} spans x {:.1}..{:.1}, box {:.1}..{:.1}",
            word.text, bbox[0], bbox[2], r.x, r.x + r.width
        );
        let centre = (bbox[1] + bbox[3]) / 2.0;
        assert!(centre > r.y - tolerance && centre < r.y + r.height + tolerance, "word {:?} is off its box vertically", word.text);
    }
}

#[test]
fn converted_text_keeps_the_scan_look_and_changes_pixels_only_inside_the_lines() {
    let (scan, lines) = fixture();
    let operations = list(json!([{ "op": "convertOcrText", "page": 1, "lines": lines }]));
    let outcome = apply_operations(&scan, &operations, &library(), false).unwrap();
    keep("ocr-converted", &outcome.bytes);

    let dpi = 100.0;
    let (w, h, before) = render(&scan, dpi as u32);
    let (_, _, after) = render(&outcome.bytes, dpi as u32);
    // Pixels of the converted lines (their boxes plus 3 pt).
    let px = dpi / 72.0;
    let inside = |x: usize, y: usize| {
        let (ux, uy) = (x as f64 / px, 792.0 - y as f64 / px);
        lines.iter().any(|l| {
            ux >= l.rect.x - 3.0 && ux <= l.rect.x + l.rect.width + 3.0 && uy >= l.rect.y - 3.0 && uy <= l.rect.y + l.rect.height + 3.0
        })
    };
    let (mut outside_changed, mut outside_total, mut inside_sum, mut inside_total) = (0usize, 0usize, 0.0f64, 0usize);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 3;
            let diff = (0..3).map(|c| (i32::from(before[i + c]) - i32::from(after[i + c])).abs()).max().unwrap();
            if inside(x, y) {
                inside_sum += f64::from(diff);
                inside_total += 1;
            } else {
                outside_total += 1;
                if diff > 8 {
                    outside_changed += 1;
                }
            }
        }
    }
    let mean_inside = inside_sum / inside_total.max(1) as f64;
    eprintln!("outside changed {outside_changed}/{outside_total}; mean |diff| inside the lines {mean_inside:.2}/255");
    assert_eq!(outside_changed, 0, "pixels outside the converted lines changed");
    assert!(mean_inside < 14.0, "the converted lines look different from the scan (mean |diff| {mean_inside:.2})");

    // The page now has real, visible text that reads as the lines did.
    let layout = PageTextLayout::from_bytes(&outcome.bytes, 1).unwrap();
    for line in &lines {
        assert!(layout.text.contains(&line.text), "layout text lacks {:?}: {:?}", line.text, layout.text);
    }
    assert!(layout.glyphs.iter().filter(|g| !g.text.trim().is_empty()).all(|g| !g.invisible));

    // And one word of it can be edited in place.
    let word = "cracked";
    let start = layout.text.find(word).map(|b| layout.text[..b].chars().count()).unwrap();
    let end = start + word.chars().count();
    let edit = list(json!([{ "op": "replaceText", "page": 1, "start": start, "end": end, "expected": word, "text": "broken" }]));
    let edited = apply_operations(&outcome.bytes, &edit, &library(), false).unwrap();
    keep("ocr-converted-edited", &edited.bytes);
    let after_edit = PageTextLayout::from_bytes(&edited.bytes, 1).unwrap();
    assert!(after_edit.text.contains("Two broken tiles were replaced"), "{:?}", after_edit.text);
}

#[test]
fn ocr_text_operations_are_validated() {
    let (scan, lines) = fixture();
    let refused = |operation: serde_json::Value| {
        let parsed: Result<OperationList, _> = serde_json::from_value(json!({ "version": 1, "operations": [operation] }));
        match parsed {
            Err(_) => true,
            Ok(list) => list.validate().is_err() || apply_operations(&scan, &list, &library(), false).is_err(),
        }
    };
    assert!(refused(json!({ "op": "addTextLayer", "page": 1, "lines": [] })));
    assert!(refused(json!({ "op": "addTextLayer", "page": 0, "lines": lines })));
    assert!(refused(json!({ "op": "addTextLayer", "page": 1, "lines": [{ "text": "a\u{7}b", "rect": { "x": 1, "y": 1, "width": 9, "height": 9 } }] })));
    assert!(refused(json!({ "op": "addTextLayer", "page": 1, "lines": [{ "text": "ab", "rect": { "x": 1, "y": 1, "width": 0, "height": 9 } }] })));
    assert!(refused(json!({ "op": "addTextLayer", "page": 1, "lines": [{ "text": "a b", "rect": { "x": 1, "y": 1, "width": 9, "height": 9 }, "words": [{ "text": "a b", "rect": { "x": 1, "y": 1, "width": 9, "height": 9 } }] }] })));
    assert!(refused(json!({ "op": "convertOcrText", "page": 1, "lines": [{ "text": "ab", "rect": { "x": 1, "y": 1, "width": 9, "height": 9 }, "color": { "c": 0, "m": 0, "y": 0, "k": 1 } }] })));
    assert!(refused(json!({ "op": "convertOcrText", "page": 1, "lines": lines, "font": "Not A Font" })));
    assert!(refused(json!({ "op": "convertOcrText", "page": 1, "lines": lines, "font": "missing-font" })));
    assert!(refused(json!({ "op": "addTextLayer", "page": 1, "lines": lines, "extra": true })));
}

/// QF-2590: text added with Add Text (drawn under an ActualText span by the
/// builder) could not be edited with Edit Text: the engine refused the page
/// as malformed. Plain-script added text now carries no ActualText span.
#[test]
fn added_text_can_be_edited_in_place() {
    let (scan, _) = fixture();
    let added = list(json!([{ "op": "addText", "page": 1, "rect": { "x": 72, "y": 100, "width": 400, "height": 40 },
        "text": "Signed off by the site manager", "font": "liberation-sans-regular", "size": 12, "color": { "r": 0, "g": 0, "b": 0 } }]));
    let outcome = apply_operations(&scan, &added, &library(), false).unwrap();
    let layout = PageTextLayout::from_bytes(&outcome.bytes, 1).unwrap();
    assert!(layout.text.contains("Signed off by the site manager"), "{:?}", layout.text);
    let start = layout.text.find("site").map(|b| layout.text[..b].chars().count()).unwrap();
    let edit = list(json!([{ "op": "replaceText", "page": 1, "start": start, "end": start + 4, "expected": "site", "text": "project" }]));
    let edited = apply_operations(&outcome.bytes, &edit, &library(), false).unwrap();
    let after = PageTextLayout::from_bytes(&edited.bytes, 1).unwrap();
    assert!(after.text.contains("Signed off by the project manager"), "{:?}", after.text);
}
