//! Overflow policies: text that does not fit is refused (the default),
//! shrunk, continued past its box or margin, or continued on the next page,
//! and no text operation ever loses the user's words (QF-2576).

use serde_json::{json, Value};
use wellfriendpdf_editor::{apply_operations, ApplyOutcome, EditorError, MemoryFonts, OperationList};
use wellfriendpdf_engine::{EditorGlyph, PageTextLayout};

const GELASIO_BOLD: &[u8] = include_bytes!("fonts/Gelasio-Bold.ttf");
const SANS: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Regular.ttf");
const WIKIPEDIA: &[u8] = include_bytes!("../../../tests/corpus/pdfs/pdfjs/pdfjs_wikipedia.pdf");
const PROJECT: &str = "The project was created to provide";

fn library() -> MemoryFonts {
    let mut fonts = MemoryFonts::new();
    fonts.insert("gelasio-bold", GELASIO_BOLD.to_vec());
    fonts.insert("liberation-sans-regular", SANS.to_vec());
    fonts
}

fn apply(source: &[u8], operations: Value) -> Result<ApplyOutcome, EditorError> {
    let list: OperationList = serde_json::from_value(json!({ "version": 1, "operations": operations })).unwrap();
    apply_operations(source, &list, &library(), false)
}

fn layout(bytes: &[u8], page: usize) -> PageTextLayout {
    PageTextLayout::from_bytes(bytes, page).unwrap()
}

/// With `WF_EDITOR_TEST_OUT` set, write `bytes` there for a visual check.
fn keep(name: &str, bytes: &[u8]) {
    if let Ok(dir) = std::env::var("WF_EDITOR_TEST_OUT") {
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.pdf")), bytes).unwrap();
    }
}

fn paragraph(layout: &PageTextLayout, needle: &str) -> usize {
    layout
        .paragraphs
        .iter()
        .position(|p| layout.text_in(p.range).contains(needle))
        .unwrap_or_else(|| panic!("no paragraph contains {needle:?}"))
}

/// Whitespace-free text, as the overflow check compares it.
fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn visible(glyph: &EditorGlyph) -> bool {
    !glyph.text.trim().is_empty() && !glyph.invisible
}

fn intersects(a: [f64; 4], b: [f64; 4]) -> bool {
    a[0] < b[2] - 0.01 && b[0] < a[2] - 0.01 && a[1] < b[3] - 0.01 && b[1] < a[3] - 0.01
}

/// Visible glyphs of paragraph `index` that intersect another paragraph's.
fn collisions(layout: &PageTextLayout, index: usize) -> Vec<(String, String)> {
    let lines = layout.paragraphs[index].lines;
    let mine: Vec<_> = layout.glyphs.iter().filter(|g| visible(g) && g.line >= lines[0] && g.line < lines[1]).collect();
    let mut hits = Vec::new();
    for other in layout.glyphs.iter().filter(|g| visible(g) && (g.line < lines[0] || g.line >= lines[1])) {
        for glyph in &mine {
            if intersects(glyph.bbox, other.bbox) {
                hits.push((glyph.text.clone(), other.text.clone()));
            }
        }
    }
    hits
}

fn sizes_of(layout: &PageTextLayout, index: usize) -> Vec<f64> {
    let lines = layout.paragraphs[index].lines;
    layout.glyphs.iter().filter(|g| visible(g) && g.line >= lines[0] && g.line < lines[1]).map(|g| g.font_size).collect()
}

fn restyle_op(source: &[u8], page: usize, needle: &str, size: f64, overflow: Option<&str>) -> Value {
    let before = layout(source, page);
    let p = &before.paragraphs[paragraph(&before, needle)];
    let mut operation = json!({ "op": "restyleText", "page": page, "start": p.range[0], "end": p.range[1],
        "expected": before.text_in(p.range), "font": "gelasio-bold", "size": size });
    if let Some(policy) = overflow {
        operation["overflow"] = json!(policy);
    }
    operation
}

fn overflow_details(error: &EditorError) -> &serde_json::Map<String, Value> {
    assert_eq!(error.code, "EDITOR_TEXT_OVERFLOW", "{error}");
    error.details.as_ref().and_then(Value::as_object).unwrap_or_else(|| panic!("no details: {error:?}"))
}

/// A Letter document in Liberation Sans 11 pt. Page 1: a title at the top,
/// a body paragraph, and a closing paragraph near the bottom margin (72 pt).
/// Page 2 (when `pages` is 2): a paragraph at the top.
fn near_bottom(pages: usize) -> Vec<u8> {
    use wellfriendpdf_engine::{AuthorPageSize, ParagraphStyle, PdfBuilder, TextStyle};
    let mut builder = PdfBuilder::new();
    let sans = builder.register_font_bytes("LiberationSans", SANS.to_vec()).unwrap();
    let body = TextStyle::new(sans, 11.0);
    let spacing = ParagraphStyle::new().line_height(1.2);
    let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
    page.draw_text("Overflow test page", 72.0, 706.0, &TextStyle::new(sans, 18.0)).unwrap();
    page.draw_paragraph(
        "This first paragraph sits near the top of the page. It has a few lines of ordinary text so the page has a body, a column width and a line pitch for the edits below to follow.",
        72.0, 670.0, 468.0, &body, &spacing,
    )
    .unwrap();
    page.draw_paragraph(
        "Closing paragraph near the bottom margin of the page. Any growth of this paragraph has no text below it to push and only the bottom margin left to use before the page edge.",
        72.0, 110.0, 468.0, &body, &spacing,
    )
    .unwrap();
    if pages > 1 {
        let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
        page.draw_paragraph(
            "Second page paragraph at the top of the column. It moves down when text from the previous page continues here.",
            72.0, 706.0, 468.0, &body, &spacing,
        )
        .unwrap();
        page.draw_paragraph("A later paragraph on the second page.", 72.0, 640.0, 468.0, &body, &spacing).unwrap();
    }
    builder.to_bytes().unwrap()
}

const CLOSING: &str = "Closing paragraph near the bottom";

// ---- refuse (the default) ----

#[test]
fn an_overflowing_restyle_is_refused_and_the_document_is_unchanged() {
    let operation = restyle_op(WIKIPEDIA, 1, PROJECT, 13.0, None);
    assert!(operation.get("overflow").is_none(), "refuse is the default");
    let error = apply(WIKIPEDIA, json!([operation])).unwrap_err();
    let details = overflow_details(&error);
    assert_eq!(error.operation, Some(0));
    assert_eq!(details["policy"], "refuse");
    let (missing, total) = (details["words"].as_u64().unwrap(), details["totalWords"].as_u64().unwrap());
    assert!(missing > 0 && missing < total, "{details:?}");
    assert!(details["characters"].as_u64().unwrap() > missing);
    assert_eq!(details["nextPage"], false, "a one-page document has no next page");
    assert!(details["continue"].is_boolean());
    assert!(error.message.contains("words") && error.message.contains("Nothing was changed"), "{}", error.message);
    // An edit before it in the same list does not reach the document either.
    let first = json!({ "op": "addText", "page": 1, "rect": { "x": 40, "y": 740, "width": 200, "height": 30 },
        "text": "Kept only with the restyle", "font": "liberation-sans-regular", "size": 10, "color": { "r": 0, "g": 0, "b": 0 } });
    let error = apply(WIKIPEDIA, json!([first, restyle_op(WIKIPEDIA, 1, PROJECT, 13.0, None)])).unwrap_err();
    assert_eq!((error.code.as_str(), error.operation), ("EDITOR_TEXT_OVERFLOW", Some(1)));
}

/// The regression: an overflowing restyle used to keep the words that fit
/// and leave the rest out of the page, with only a note in the report.
#[test]
fn overflow_never_drops_the_users_words() {
    let before = layout(WIKIPEDIA, 1);
    let expected = before.text_in(before.paragraphs[paragraph(&before, PROJECT)].range);
    for size in [13.0, 18.0] {
        for policy in [None, Some("continue")] {
            match apply(WIKIPEDIA, json!([restyle_op(WIKIPEDIA, 1, PROJECT, size, policy)])) {
                Ok(outcome) => {
                    let after = layout(&outcome.bytes, 1);
                    assert!(squash(&after.text).contains(&squash(&expected)), "{size} pt {policy:?}: words were dropped");
                }
                Err(error) => assert_eq!(error.code, "EDITOR_TEXT_OVERFLOW", "{size} pt {policy:?}: {error}"),
            }
        }
    }
}

#[test]
fn a_restyle_that_fits_needs_no_policy() {
    let outcome = apply(WIKIPEDIA, json!([restyle_op(WIKIPEDIA, 1, PROJECT, 11.5, None)])).unwrap();
    let after = layout(&outcome.bytes, 1);
    let text = after.text_in(after.paragraphs[paragraph(&after, PROJECT)].range);
    assert!(text.ends_with("for fast rendering speeds.[9]"), "{text:?}");
    assert!(outcome.reports[0].overflow.is_none());
}

// ---- shrink ----

#[test]
fn shrink_sets_every_word_at_the_largest_size_that_fits() {
    let before = layout(WIKIPEDIA, 1);
    let expected = before.text_in(before.paragraphs[paragraph(&before, PROJECT)].range);
    let outcome = apply(WIKIPEDIA, json!([restyle_op(WIKIPEDIA, 1, PROJECT, 13.0, Some("shrink"))])).unwrap();
    keep("overflow-shrink-wikipedia", &outcome.bytes);
    let report = &outcome.reports[0];
    let used = report.overflow.as_ref().and_then(|o| o.size).expect("the report names the size used");
    assert!((4.0..13.0).contains(&used), "{used}");
    assert!(report.notes.iter().any(|n| n.contains("Shrunk to")), "{:?}", report.notes);
    let after = layout(&outcome.bytes, 1);
    assert!(squash(&after.text).contains(&squash(&expected)), "every word is on the page");
    let restyled = paragraph(&after, PROJECT);
    assert!(sizes_of(&after, restyled).iter().all(|s| (s - used).abs() < 0.2), "{:?}", sizes_of(&after, restyled));
    assert!(collisions(&after, restyled).is_empty(), "{:?}", collisions(&after, restyled));
    // A quarter point more would not have fitted.
    let larger = apply(WIKIPEDIA, json!([restyle_op(WIKIPEDIA, 1, PROJECT, used + 0.25, None)]));
    assert_eq!(larger.unwrap_err().code, "EDITOR_TEXT_OVERFLOW");
    // The server replays the same list to the same bytes.
    let again = apply(WIKIPEDIA, json!([restyle_op(WIKIPEDIA, 1, PROJECT, 13.0, Some("shrink"))])).unwrap();
    assert_eq!(again.bytes, outcome.bytes);
}

// ---- continue ----

#[test]
fn continue_keeps_the_size_and_runs_into_the_bottom_margin() {
    let source = near_bottom(1);
    let refused = apply(&source, json!([restyle_op(&source, 1, CLOSING, 16.0, None)])).unwrap_err();
    let details = overflow_details(&refused);
    assert_eq!(details["continue"], true, "{details:?}");
    let outcome = apply(&source, json!([restyle_op(&source, 1, CLOSING, 16.0, Some("continue"))])).unwrap();
    keep("overflow-continue", &outcome.bytes);
    let before = layout(&source, 1);
    let after = layout(&outcome.bytes, 1);
    let restyled = paragraph(&after, CLOSING);
    assert!(sizes_of(&after, restyled).iter().all(|s| (s - 16.0).abs() < 0.2), "the size is kept");
    let original = before.paragraphs[paragraph(&before, CLOSING)].bbox;
    let grown = after.paragraphs[restyled].bbox;
    assert!(grown[1] < 72.0 && grown[1] < original[1] - 20.0, "past the margin: {grown:?}");
    assert!(after.glyphs.iter().filter(|g| visible(g)).all(|g| g.bbox[1] >= 0.0), "still on the page");
    let expected = before.text_in(before.paragraphs[paragraph(&before, CLOSING)].range);
    assert!(squash(&after.text).contains(&squash(&expected)));
    assert!(collisions(&after, restyled).is_empty());
    assert_eq!(outcome.reports[0].overflow.as_ref().map(|o| o.policy), Some(wellfriendpdf_editor::ops::TextOverflow::Continue));
    // So large it does not fit even at the page edge: still refused.
    let error = apply(&source, json!([restyle_op(&source, 1, CLOSING, 60.0, Some("continue"))])).unwrap_err();
    assert_eq!(overflow_details(&error)["policy"], "continue");
}

// ---- next page ----

#[test]
fn next_page_moves_the_rest_to_the_top_of_the_column_overleaf() {
    let source = near_bottom(2);
    let refused = apply(&source, json!([restyle_op(&source, 1, CLOSING, 16.0, None)])).unwrap_err();
    assert_eq!(overflow_details(&refused)["nextPage"], true);
    let outcome = apply(&source, json!([restyle_op(&source, 1, CLOSING, 16.0, Some("nextPage"))])).unwrap();
    keep("overflow-next-page", &outcome.bytes);
    let report = &outcome.reports[0];
    let placed = report.overflow.as_ref().expect("the report says where the rest went");
    assert_eq!(placed.next_page, Some(2));
    let moved = placed.moved_words.unwrap();
    let before = layout(&source, 1);
    let all = words(&before.text_in(before.paragraphs[paragraph(&before, CLOSING)].range));
    assert!(moved > 0 && moved < all.len(), "{moved} of {}", all.len());
    let first = layout(&outcome.bytes, 1);
    let second = layout(&outcome.bytes, 2);
    // Page 1 ends with the words that fit, inside the margin and at 16 pt.
    let kept = all[..all.len() - moved].join(" ");
    assert!(squash(&first.text).contains(&squash(&kept)), "{:?}", first.text);
    let restyled = paragraph(&first, CLOSING);
    assert!(first.paragraphs[restyled].bbox[1] >= 50.0, "{:?}", first.paragraphs[restyled].bbox);
    assert!(sizes_of(&first, restyled).iter().all(|s| (s - 16.0).abs() < 0.2));
    // Page 2 starts with the rest, at the top of the same column ...
    let rest = all[all.len() - moved..].join(" ");
    assert!(squash(&second.text).contains(&squash(&rest)), "{:?}", second.text);
    let continued = paragraph(&second, &all[all.len() - moved]);
    let top = second.paragraphs[continued].bbox;
    assert!(top[3] > 700.0 && (top[0] - 72.0).abs() < 1.0, "{top:?}");
    // ... and the column text there moved down below it, all of it kept.
    let old = layout(&source, 2);
    for needle in ["Second page paragraph", "A later paragraph"] {
        let a = old.paragraphs[paragraph(&old, needle)].bbox;
        let b = second.paragraphs[paragraph(&second, needle)].bbox;
        assert!(b[3] < a[3] - 10.0 && b[3] < top[1], "{needle}: {a:?} -> {b:?}");
        assert!(squash(&second.text).contains(&squash(&old.text_in(old.paragraphs[paragraph(&old, needle)].range))));
    }
    assert!(collisions(&second, continued).is_empty(), "{:?}", collisions(&second, continued));
}

#[test]
fn next_page_needs_a_next_page() {
    // Wikipedia has one page; with a blank page inserted after it the rest
    // of the restyled paragraph continues there.
    let error = apply(WIKIPEDIA, json!([restyle_op(WIKIPEDIA, 1, PROJECT, 13.0, Some("nextPage"))])).unwrap_err();
    assert_eq!(error.code, "EDITOR_TEXT_OVERFLOW");
    let outcome = apply(
        WIKIPEDIA,
        json!([{ "op": "insertBlankPage", "after": 1, "width": 612, "height": 792 }, restyle_op(WIKIPEDIA, 1, PROJECT, 13.0, Some("nextPage"))]),
    )
    .unwrap();
    keep("overflow-next-page-wikipedia", &outcome.bytes);
    let placed = outcome.reports[1].overflow.as_ref().unwrap();
    assert_eq!(placed.next_page, Some(2));
    let before = layout(WIKIPEDIA, 1);
    let expected = before.text_in(before.paragraphs[paragraph(&before, PROJECT)].range);
    // (Chrome's lines join without a space, so the split is the report's.)
    let split = outcome.reports[1].continued.clone().unwrap();
    assert_eq!(words(&split.rest).len(), placed.moved_words.unwrap());
    assert_eq!(squash(&format!("{}{}", split.kept, split.rest)), squash(&expected));
    let second = layout(&outcome.bytes, 2);
    assert!(squash(&second.text).contains(&squash(&split.rest)), "{:?}", second.text);
    let top = second.paragraphs[paragraph(&second, split.rest.split_whitespace().next().unwrap())].bbox;
    assert!(top[3] > 700.0 && (top[0] - 35.5).abs() < 1.0, "the top of the left column: {top:?}");
    // The other paragraphs of page 1 are all still there.
    let first = layout(&outcome.bytes, 1);
    for p in &before.paragraphs {
        let text = before.text_in(p.range);
        if !text.starts_with(PROJECT) {
            assert!(squash(&first.text).contains(&squash(&text)), "{text:?} kept");
        }
    }
}

// ---- replaceText ----

fn insertion(source: &[u8], extra: &str, overflow: Option<&str>) -> Value {
    let before = layout(source, 1);
    let p = &before.paragraphs[paragraph(&before, CLOSING)];
    let end = p.range[1];
    let last = before.text_in([end - 1, end]);
    let mut operation = json!({ "op": "replaceText", "page": 1, "start": end - 1, "end": end, "expected": last,
        "text": format!("{last} {extra}"), "fallbackFont": "liberation-sans-regular" });
    if let Some(policy) = overflow {
        operation["overflow"] = json!(policy);
    }
    operation
}

#[test]
fn a_paragraph_that_grows_past_the_margin_follows_its_policy() {
    let source = near_bottom(2);
    let extra = "These added sentences make the closing paragraph longer than the room above the margin. ".repeat(3);
    let extra = extra.trim();
    let before = layout(&source, 1);
    let wanted = {
        let p = &before.paragraphs[paragraph(&before, CLOSING)];
        format!("{} {extra}", before.text_in(p.range))
    };
    // Refused by default, with the facts the editor shows.
    let error = apply(&source, json!([insertion(&source, extra, None)])).unwrap_err();
    let details = overflow_details(&error);
    assert!(details["words"].as_u64().unwrap() > 0);
    assert_eq!(details["nextPage"], true, "{details:?}");
    // Continue: the source font and size, into the bottom margin.
    let outcome = apply(&source, json!([insertion(&source, extra, Some("continue"))])).unwrap();
    let after = layout(&outcome.bytes, 1);
    assert!(squash(&after.text).contains(&squash(&wanted)));
    assert!(after.glyphs.iter().filter(|g| visible(g)).all(|g| g.bbox[1] >= 0.0));
    // Shrink: every word at a smaller size.
    let outcome = apply(&source, json!([insertion(&source, extra, Some("shrink"))])).unwrap();
    keep("overflow-replace-shrink", &outcome.bytes);
    let used = outcome.reports[0].overflow.as_ref().and_then(|o| o.size).unwrap();
    assert!(used < 11.0, "{used}");
    let after = layout(&outcome.bytes, 1);
    assert!(squash(&after.text).contains(&squash(&wanted)));
    let shrunk = paragraph(&after, CLOSING);
    assert!(after.paragraphs[shrunk].bbox[1] >= 50.0, "{:?}", after.paragraphs[shrunk].bbox);
    // Next page: the words that fit stay, the rest continues on page 2.
    let outcome = apply(&source, json!([insertion(&source, extra, Some("nextPage"))])).unwrap();
    keep("overflow-replace-next-page", &outcome.bytes);
    let placed = outcome.reports[0].overflow.as_ref().unwrap();
    assert_eq!(placed.next_page, Some(2));
    let all = words(&wanted);
    let moved = placed.moved_words.unwrap();
    let rest = all[all.len() - moved..].join(" ");
    assert!(squash(&layout(&outcome.bytes, 2).text).contains(&squash(&rest)));
    assert!(squash(&layout(&outcome.bytes, 1).text).contains(&squash(&all[..all.len() - moved].join(" "))));
    // The editor sends typing at the end of a paragraph as a pure insertion.
    let end = before.paragraphs[paragraph(&before, CLOSING)].range[1];
    let append = |overflow: &str| json!({ "op": "replaceText", "page": 1, "start": end, "end": end, "expected": "",
        "text": format!(" {extra}"), "fallbackFont": "liberation-sans-regular", "overflow": overflow });
    let error = apply(&source, json!([append("refuse")])).unwrap_err();
    let details = overflow_details(&error);
    assert!(details["words"].as_u64().unwrap() > 0 && details["nextPage"] == true, "{details:?}");
    for policy in ["shrink", "continue", "nextPage"] {
        let outcome = apply(&source, json!([append(policy)])).unwrap_or_else(|e| panic!("{policy}: {e}"));
        let mut text = squash(&layout(&outcome.bytes, 1).text);
        if policy == "nextPage" {
            text.push_str(&squash(&layout(&outcome.bytes, 2).text));
        }
        for word in words(&wanted) {
            assert!(text.contains(&squash(&word)), "{policy}: {word:?} kept");
        }
    }
}

// ---- addText ----

fn text_box(text: &str, overflow: Option<&str>) -> Value {
    // The width of the page's column, so the rest can start that column on
    // the next page.
    let mut operation = json!({ "op": "addText", "page": 1, "rect": { "x": 72, "y": 300, "width": 468, "height": 30 },
        "text": text, "font": "liberation-sans-regular", "size": 12, "color": { "r": 0, "g": 0, "b": 0 } });
    if let Some(policy) = overflow {
        operation["overflow"] = json!(policy);
    }
    operation
}

#[test]
fn an_added_text_box_keeps_its_words() {
    let source = near_bottom(2);
    let text = "An added text box with more words than its box can hold at twelve points, so it overflows the bottom of the box it was drawn in. A second sentence makes sure that it needs at least three lines at the width of the column, and a third one adds a little more.";
    let error = apply(&source, json!([text_box(text, None)])).unwrap_err();
    let details = overflow_details(&error);
    assert_eq!((details["continue"].clone(), details["nextPage"].clone()), (json!(true), json!(true)));
    // Short text fits without a policy.
    apply(&source, json!([text_box("Fits in its box.", None)])).unwrap();
    // Continue: past the box, at 12 pt.
    let outcome = apply(&source, json!([text_box(text, Some("continue"))])).unwrap();
    let after = layout(&outcome.bytes, 1);
    assert!(squash(&after.text).contains(&squash(text)));
    assert!(outcome.reports[0].notes.iter().any(|n| n.contains("past the bottom of its box")));
    // Shrink: inside the box.
    let outcome = apply(&source, json!([text_box(text, Some("shrink"))])).unwrap();
    let used = outcome.reports[0].overflow.as_ref().and_then(|o| o.size).unwrap();
    assert!((4.0..12.0).contains(&used), "{used}");
    let after = layout(&outcome.bytes, 1);
    assert!(squash(&after.text).contains(&squash(text)));
    // (The page's own text is far above and below the box.)
    let added: Vec<_> = after.glyphs.iter().filter(|g| visible(g) && g.bbox[1] > 150.0 && g.bbox[3] < 500.0).collect();
    assert!(!added.is_empty());
    assert!(added.iter().all(|g| g.bbox[1] >= 300.0 - 0.3 && (g.font_size - used).abs() < 0.2), "inside the box");
    // Next page: the rest starts the column on page 2.
    let outcome = apply(&source, json!([text_box(text, Some("nextPage"))])).unwrap();
    let moved = outcome.reports[0].overflow.as_ref().unwrap().moved_words.unwrap();
    let all = words(text);
    assert!(squash(&layout(&outcome.bytes, 2).text).contains(&squash(&all[all.len() - moved..].join(" "))));
    assert!(squash(&layout(&outcome.bytes, 1).text).contains(&squash(&all[..all.len() - moved].join(" "))));
}
