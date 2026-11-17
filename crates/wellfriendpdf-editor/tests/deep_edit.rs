//! Deep editing of complex pages (QF-5210 to QF-5214): every visible thing
//! is an object (text runs in any font kind, outlined text, images, inline
//! images, groups, layers, content in nested forms); objects move, recolour,
//! reorder and are really removed (their text no longer extracts and the
//! resources only they used leave the file); text in subset and Type 3
//! fonts is edited in place or replaced with editable text.
//!
//! The corpus (`tests/corpus/pdfs/deep-edit`, rebuilt by its
//! `make_corpus.py`) holds a book-cover logo in heavy bold coloured text in
//! each form a real file uses, a dense multi-column page and a page of
//! grouped vector art, annotations and an inline image.

use lopdf::{dictionary, Document};
use serde_json::json;
use wellfriendpdf_editor::content_objects::{page_content_objects, ContentObject, ContentObjectKind};
use wellfriendpdf_editor::delta::{self, Direction};
use wellfriendpdf_editor::{apply_operation, page_objects, EditorOperation, MemoryFonts};
use wellfriendpdf_engine::{ContentEngine, PageTextLayout};

const SANS: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Regular.ttf");
const SANS_BOLD: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Bold.ttf");
const SERIF: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Regular.ttf");
const SERIF_BOLD: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Bold.ttf");

macro_rules! corpus {
    ($name:literal) => {
        include_bytes!(concat!("../../../tests/corpus/pdfs/deep-edit/", $name, ".pdf")).as_slice()
    };
}

const BOOK: &[u8] = corpus!("book-cover");
const SUBSET: &[u8] = corpus!("logo-a1-subset-truetype");
const CID: &[u8] = corpus!("logo-a2-subset-cid");
const TYPE3: &[u8] = corpus!("logo-b-type3");
const OUTLINED: &[u8] = corpus!("logo-c-outlined");
const FORMS: &[u8] = corpus!("logo-d-form-nested");
const CLIP: &[u8] = corpus!("logo-e-clip-gradient");
const RASTER: &[u8] = corpus!("logo-f-raster");
const LAYER: &[u8] = corpus!("logo-g-layer");
const DENSE: &[u8] = corpus!("dense-multicolumn");
const VECTOR: &[u8] = corpus!("vector-annots-inline");

fn library() -> MemoryFonts {
    let mut fonts = MemoryFonts::new();
    fonts.insert("liberation-sans-regular", SANS.to_vec());
    fonts.insert("liberation-sans-bold", SANS_BOLD.to_vec());
    fonts.insert("liberation-serif-regular", SERIF.to_vec());
    fonts.insert("liberation-serif-bold", SERIF_BOLD.to_vec());
    // The closest library faces the corpus fonts match (the test library
    // stands in for them with metric-similar faces).
    fonts.insert("lato-bold", SANS_BOLD.to_vec());
    fonts.insert("noto-sans-regular", SANS.to_vec());
    fonts.insert("noto-sans-bold", SANS_BOLD.to_vec());
    fonts
}

fn op(value: serde_json::Value) -> EditorOperation {
    serde_json::from_value(value).unwrap()
}

fn objects(bytes: &[u8]) -> Vec<ContentObject> {
    page_content_objects(&Document::load_mem(bytes).unwrap(), 1).unwrap().objects
}

fn run(bytes: &[u8], operation: serde_json::Value) -> (Vec<u8>, wellfriendpdf_editor::OperationReport) {
    let operation = op(operation);
    apply_operation(bytes, &operation, &library(), true).unwrap_or_else(|e| panic!("{}: {e:?}", operation.name()))
}

fn text(bytes: &[u8]) -> String {
    PageTextLayout::from_bytes(bytes, 1).unwrap().text
}

fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn has_text(bytes: &[u8], needle: &str) -> bool {
    squash(&text(bytes)).contains(&squash(needle))
}

fn count(bytes: &[u8], needle: &[u8]) -> usize {
    bytes.windows(needle.len()).filter(|w| *w == needle).count()
}

fn pixel(bytes: &[u8], x: f64, y: f64) -> (u8, u8, u8) {
    let raw = ContentEngine::open_bytes(bytes.to_vec()).unwrap().render_page(1, 72).unwrap().into_raw_image();
    let document = Document::load_mem(bytes).unwrap();
    let page = *document.get_pages().get(&1).unwrap();
    let height = document
        .get_dictionary(page)
        .unwrap()
        .get(b"MediaBox")
        .unwrap()
        .as_array()
        .unwrap()[3]
        .as_float()
        .map(f64::from)
        .or_else(|_| document.get_dictionary(page).unwrap().get(b"MediaBox").unwrap().as_array().unwrap()[3].as_i64().map(|v| v as f64))
        .unwrap();
    let i = ((height - y) as usize * raw.width as usize + x as usize) * raw.channels as usize;
    (raw.pixels[i], raw.pixels[i + 1], raw.pixels[i + 2])
}

/// Where the first glyph showing `letter` starts and its baseline height.
fn first_glyph(bytes: &[u8], letter: char) -> (f64, f64) {
    let layout = PageTextLayout::from_bytes(bytes, 1).unwrap();
    let glyph = layout.glyphs.iter().find(|g| g.text.starts_with(letter)).unwrap_or_else(|| panic!("no {letter} glyph"));
    (glyph.baseline[0][0], glyph.baseline[0][1])
}

/// A pixel inside `part` drawn in (about) `rgb` on the page as rendered.
fn ink_point(bytes: &[u8], part: [f64; 4], rgb: [f64; 3]) -> (f64, f64) {
    let want = rgb.map(|v| v * 255.0);
    let raw = ContentEngine::open_bytes(bytes.to_vec()).unwrap().render_page(1, 72).unwrap().into_raw_image();
    let height = raw.height as f64;
    let at = |x: f64, y: f64| {
        let i = ((height - y) as usize * raw.width as usize + x as usize) * raw.channels as usize;
        (raw.pixels[i], raw.pixels[i + 1], raw.pixels[i + 2])
    };
    let mut y = part[1] + 1.0;
    while y < part[3] {
        let mut x = part[0] + 1.0;
        while x < part[2] {
            let (r, g, b) = at(x, y);
            if (f64::from(r) - want[0]).abs() < 40.0 && (f64::from(g) - want[1]).abs() < 40.0 && (f64::from(b) - want[2]).abs() < 40.0 {
                return (x, y);
            }
            x += 2.0;
        }
        y += 2.0;
    }
    panic!("no pixel of {rgb:?} in {part:?}");
}

/// The text run whose text contains `needle`.
fn text_run<'a>(listed: &'a [ContentObject], needle: &str) -> &'a ContentObject {
    listed
        .iter()
        .find(|o| o.selector.kind == ContentObjectKind::Text && o.text.as_deref().is_some_and(|t| squash(t).contains(&squash(needle))))
        .unwrap_or_else(|| panic!("no text run with {needle:?}: {:?}", listed.iter().map(|o| (o.selector.kind, o.text.clone())).collect::<Vec<_>>()))
}

fn edit(bytes: &[u8], object: &ContentObject, edit: serde_json::Value) -> (Vec<u8>, wellfriendpdf_editor::OperationReport) {
    run(bytes, json!({ "op": "editObject", "selector": object.selector, "edit": edit }))
}

const MOVE: fn() -> serde_json::Value = || json!({ "kind": "transform", "matrix": { "a": 1, "b": 0, "c": 0, "d": 1, "e": 0, "f": -150 } });

/// An edit undone by its object delta restores the input exactly.
fn undo_restores(before: &[u8], after: &[u8]) {
    let computed = delta::compute(before, after).unwrap();
    let undone = computed.apply(after, Direction::Backward).unwrap();
    assert!(
        delta::same_graph(&Document::load_mem(&undone).unwrap(), &Document::load_mem(before).unwrap()),
        "undo restores the object graph"
    );
    assert_eq!(text(&undone), text(before));
}

// ---- the book cover (the owner's case) ----

#[test]
fn the_book_cover_logo_is_a_text_object_with_its_font() {
    let listed = objects(BOOK);
    let logo = text_run(&listed, "WELLBOOK");
    let font = logo.font.as_ref().unwrap();
    assert!(font.subset && font.base_font.ends_with("Lato-Black"), "{font:?}");
    assert!(font.bold, "a black weight reads as bold");
    assert_eq!(logo.render_mode, Some(2), "filled and stroked");
    assert!(logo.has("subsetFont") && logo.has("stroked") && logo.has("clipped"), "{:?}", logo.traits);
    assert!(logo.fill.is_some() && logo.stroke.is_some());
    assert!(logo.parts.len() >= 8, "one hit box per glyph");
    // Its bounds sit on the drawn glyphs, inside the dark panel.
    let [x0, y0, x1, y1] = logo.bounds;
    assert!(x0 > 16.0 && x1 < 420.0 && y0 > 400.0 && y1 < 560.0, "{:?}", logo.bounds);
    assert!(logo.can_recolor && logo.can_reorder);
    // The page still lists the panel and the background image.
    assert!(listed.iter().any(|o| o.selector.kind == ContentObjectKind::Image));
    assert!(listed.iter().any(|o| o.selector.kind == ContentObjectKind::Path));
}

#[test]
fn the_book_cover_logo_text_is_edited_in_place() {
    let listed = objects(BOOK);
    let logo = text_run(&listed, "WELLBOOK");
    let (out, report) = run(
        BOOK,
        json!({ "op": "editObjectText", "selector": logo.selector, "text": "WELLREAD", "expected": logo.text, "fallbackFont": "lato-bold" }),
    );
    assert!(has_text(&out, "WELLREAD") && !has_text(&out, "WELLBOOK"), "{}", text(&out));
    assert!(has_text(&out, "The Long Way Round") && has_text(&out, "JANE EXAMPLE"), "the rest of the cover is untouched");
    // The subset lacks R, A and D: the closest library face is used and named.
    assert_eq!(report.fonts_added, ["lato-bold"]);
    assert!(report.notes.iter().any(|n| n.contains("subset") && n.contains("lato-bold")), "{:?}", report.notes);
    let after = objects(&out);
    let edited = text_run(&after, "WELLREAD");
    assert_eq!(edited.render_mode, Some(2), "the render mode (fill and stroke) is kept");
    assert_eq!(edited.fill, logo.fill, "the colour is kept");
    // The new letters sit on the logo's baseline and start where it started
    // (the glyph boxes differ: the library face has other ascent and descent).
    let (old_start, old_base) = first_glyph(BOOK, 'W');
    let (new_start, new_base) = first_glyph(&out, 'W');
    assert!((old_base - new_base).abs() < 0.5 && (old_start - new_start).abs() < 0.5, "same baseline: {:?} vs {:?}", (old_start, old_base), (new_start, new_base));
    assert!((edited.font.as_ref().unwrap().size - logo.font.as_ref().unwrap().size).abs() < 0.01, "same size");
    undo_restores(BOOK, &out);
}

#[test]
fn the_book_cover_logo_is_edited_from_the_paragraph_editor_too() {
    // The whole logo word, which the paragraph transaction refused on the
    // pinned engine (it does not fit its clipped line): the line's text run
    // is edited instead.
    let layout = PageTextLayout::from_bytes(BOOK, 1).unwrap();
    let chars: Vec<char> = layout.text.chars().collect();
    let needle: Vec<char> = "WELLBOOK".chars().collect();
    let start = (0..=chars.len() - needle.len()).find(|&i| chars[i..i + needle.len()] == needle[..]).unwrap();
    let (out, report) = run(
        BOOK,
        json!({ "op": "replaceText", "page": 1, "start": start, "end": start + 8, "expected": "WELLBOOK", "text": "WELLREAD", "fallbackFont": "lato-bold" }),
    );
    assert!(has_text(&out, "WELLREAD") && !has_text(&out, "WELLBOOK"), "{}", text(&out));
    assert!(has_text(&out, "The Long Way Round"));
    assert!(!report.notes.is_empty(), "the substitute font is named: {:?}", report.notes);
}

#[test]
fn the_book_cover_logo_is_recoloured_moved_reordered_and_really_deleted() {
    let listed = objects(BOOK);
    let logo = text_run(&listed, "WELLBOOK");
    let center = ink_point(BOOK, logo.parts[0], logo.fill.expect("a filled logo"));
    let before = pixel(BOOK, center.0, center.1);
    let (green, _) = edit(BOOK, logo, json!({ "kind": "recolor", "fill": { "r": 0, "g": 1, "b": 0 } }));
    let (r, g, b) = pixel(&green, center.0, center.1);
    assert!(g > 150 && r < 120 && b < 120, "the logo is green: {:?} (was {before:?})", (r, g, b));
    assert!(has_text(&green, "WELLBOOK"), "recolouring keeps the text");
    assert_eq!(text_run(&objects(&green), "JANE").fill, text_run(&listed, "JANE").fill, "text after it keeps its colour");

    let (moved, _) = edit(BOOK, logo, json!({ "kind": "transform", "matrix": { "a": 1, "b": 0, "c": 0, "d": 1, "e": 0, "f": -60 } }));
    let after = text_run(&objects(&moved), "WELLBOOK").bounds;
    assert!((after[1] - (logo.bounds[1] - 60.0)).abs() < 0.5, "{after:?}");

    for kind in ["bringToFront", "sendToBack"] {
        let (reordered, _) = edit(BOOK, logo, json!({ "kind": kind }));
        assert!(has_text(&reordered, "WELLBOOK"), "{kind}");
        let reordered_objects = objects(&reordered);
        let moved_logo = text_run(&reordered_objects, "WELLBOOK");
        assert!((moved_logo.bounds[0] - logo.bounds[0]).abs() < 0.5, "{kind}: drawn at the same place");
        undo_restores(BOOK, &reordered);
    }
    // Sent to the back, the background image now covers it.
    let (back, _) = edit(BOOK, logo, json!({ "kind": "sendToBack" }));
    assert_ne!(pixel(&back, center.0, center.1), before, "behind the background");

    let (deleted, report) = edit(BOOK, logo, json!({ "kind": "delete" }));
    assert!(!has_text(&deleted, "WELLBOOK"), "removed text does not extract: {}", text(&deleted));
    assert!(has_text(&deleted, "The Long Way Round"));
    assert_eq!(count(&deleted, b"Lato-Black"), 0, "the font only the logo used left the file");
    assert!(report.notes.iter().any(|n| n.contains("removed")), "{:?}", report.notes);
    // The title after it on the page stays where it was.
    let title = text_run(&objects(&deleted), "The Long Way Round").bounds;
    assert_eq!(title.map(f64::round), text_run(&listed, "The Long Way Round").bounds.map(f64::round));
    undo_restores(BOOK, &deleted);
}

// ---- each logo construct ----

#[test]
fn subset_truetype_and_cid_logos_keep_their_font_when_it_has_the_glyphs() {
    for (name, bytes) in [("truetype", SUBSET), ("cid", CID)] {
        let listed = objects(bytes);
        let logo = text_run(&listed, "WELLBOOK");
        // "BOWL" uses only glyphs the subset has.
        let (out, report) = run(bytes, json!({ "op": "editObjectText", "selector": logo.selector, "text": "BOWL", "fallbackFont": "lato-bold" }));
        assert!(has_text(&out, "BOWL") && !has_text(&out, "WELLBOOK"), "{name}: {}", text(&out));
        assert!(report.fonts_added.is_empty(), "{name}: the subset font is kept: {:?}", report.notes);
        // Letters the subset lacks use the closest library face.
        let (out, report) = run(bytes, json!({ "op": "editObjectText", "selector": logo.selector, "text": "NEW TITLE", "fallbackFont": "lato-bold" }));
        assert!(has_text(&out, "NEW TITLE"), "{name}");
        assert_eq!(report.fonts_added, ["lato-bold"], "{name}");
    }
}

#[test]
fn a_type3_logo_is_an_object_that_recolours_and_becomes_editable_text() {
    let listed = objects(TYPE3);
    let logo = listed
        .iter()
        .find(|o| o.selector.kind == ContentObjectKind::Text && o.has("type3"))
        .expect("the Type 3 logo is listed");
    assert!(logo.has("undecodable") && logo.text.is_none(), "its glyphs have no Unicode: {:?}", logo.traits);
    assert!(logo.bounds[1] > 580.0 && logo.bounds[3] < 680.0, "{:?}", logo.bounds);
    // d1 glyphs take the fill colour. (The SDK raster engine paints Type 3
    // glyphs without the fill colour, so the drawn state is checked here and
    // the pixels with MuPDF and pdf.js in the corpus run.)
    let (red, _) = edit(TYPE3, logo, json!({ "kind": "recolor", "fill": { "r": 1, "g": 0, "b": 0 } }));
    let recoloured = objects(&red).into_iter().find(|o| o.has("type3")).expect("still listed");
    assert_eq!(recoloured.fill, Some([1.0, 0.0, 0.0]), "the run is drawn in the new fill");
    assert_eq!(recoloured.bounds.map(f64::round), logo.bounds.map(f64::round), "in place");
    let (moved, _) = edit(TYPE3, logo, MOVE());
    assert!((objects(&moved).iter().find(|o| o.has("type3")).unwrap().bounds[1] - (logo.bounds[1] - 150.0)).abs() < 0.5);
    // Replace with editable text: it reads back and the Type 3 font is gone.
    let (out, report) = run(TYPE3, json!({ "op": "editObjectText", "selector": logo.selector, "text": "WELLBOOK", "font": "liberation-sans-bold" }));
    assert!(has_text(&out, "WELLBOOK"), "{}", text(&out));
    assert_eq!(report.fonts_added, ["liberation-sans-bold"]);
    assert_eq!(count(&out, b"/Type3"), 0, "the Type 3 font left the file");
    // Body text on a page whose other font does not decode stays editable.
    assert!(has_text(&out, "Chapter one begins here"));
    let (deleted, _) = edit(TYPE3, logo, json!({ "kind": "delete" }));
    assert_eq!(count(&deleted, b"/Type3"), 0);
    undo_restores(TYPE3, &deleted);
}

#[test]
fn body_text_on_a_page_with_an_undecodable_font_is_edited_through_its_run() {
    let layout = PageTextLayout::from_bytes(TYPE3, 1).unwrap();
    let chars: Vec<char> = layout.text.chars().collect();
    let needle: Vec<char> = "Chapter".chars().collect();
    let start = (0..=chars.len() - needle.len()).find(|&i| chars[i..i + needle.len()] == needle[..]).expect("the body text reads");
    let (out, _) = run(
        TYPE3,
        json!({ "op": "replaceText", "page": 1, "start": start, "end": start + 7, "expected": "Chapter", "text": "Section", "fallbackFont": "liberation-serif-regular" }),
    );
    assert!(has_text(&out, "Section one begins here"), "{}", text(&out));
}

#[test]
fn outlined_text_is_one_object_hit_by_its_letters_and_becomes_editable_text() {
    let listed = objects(OUTLINED);
    let logo = listed
        .iter()
        .filter(|o| o.selector.kind == ContentObjectKind::Path && o.bounds[1] > 580.0)
        .max_by(|a, b| (a.bounds[2] - a.bounds[0]).total_cmp(&(b.bounds[2] - b.bounds[0])))
        .expect("the outlined logo");
    assert!(logo.parts.len() >= 8, "one hit box per letter outline: {}", logo.parts.len());
    let (out, report) = run(OUTLINED, json!({ "op": "replaceObjectWithText", "selector": logo.selector, "text": "WELLBOOK", "font": "liberation-sans-bold" }));
    assert!(has_text(&out, "WELLBOOK"), "{}", text(&out));
    let out_objects = objects(&out);
    let words = text_run(&out_objects, "WELLBOOK");
    assert!((words.bounds[0] - logo.bounds[0]).abs() < 4.0 && (words.bounds[2] - logo.bounds[2]).abs() < 12.0, "{:?} vs {:?}", words.bounds, logo.bounds);
    assert_eq!(words.fill.map(|c| c.map(|v| (v * 10.0).round())), logo.fill.map(|c| c.map(|v| (v * 10.0).round())), "the outline's colour");
    assert!(report.notes.iter().any(|n| n.contains("editable text")));
    assert!(!objects(&out).iter().any(|o| o.selector.expected_sha256 == logo.selector.expected_sha256), "the outlines are gone");
}

#[test]
fn text_in_a_nested_form_is_edited_per_placement() {
    let listed = objects(FORMS);
    let runs: Vec<_> = listed.iter().filter(|o| o.selector.kind == ContentObjectKind::Text && o.text.as_deref() == Some("WELLBOOK")).collect();
    assert_eq!(runs.len(), 2, "the inner form is drawn twice");
    let nested = runs.iter().find(|o| o.selector.forms.len() == 2).expect("the placement inside the outer form");
    // (The engine's text layout reads one of the two placements, so the
    // placements are counted as listed objects.)
    let logos = |bytes: &[u8]| -> Vec<ContentObject> {
        objects(bytes).into_iter().filter(|o| o.selector.kind == ContentObjectKind::Text && o.text.as_deref() == Some("WELLBOOK")).collect()
    };
    let (deleted, _) = edit(FORMS, nested, json!({ "kind": "delete" }));
    let left = logos(&deleted);
    assert_eq!(left.len(), 1, "the other placement keeps its logo");
    assert_eq!(left[0].selector.forms.len(), 1, "the direct placement is the one left");
    let (recoloured, _) = edit(FORMS, nested, json!({ "kind": "recolor", "fill": { "r": 0, "g": 0, "b": 1 } }));
    let both = logos(&recoloured);
    assert_eq!(both.len(), 2);
    assert_eq!(both.iter().find(|o| o.selector.forms.len() == 2).unwrap().fill, Some([0.0, 0.0, 1.0]), "the nested placement is blue");
    assert_ne!(both.iter().find(|o| o.selector.forms.len() == 1).unwrap().fill, Some([0.0, 0.0, 1.0]), "the other keeps its colour");
    let (edited, _) = run(FORMS, json!({ "op": "editObjectText", "selector": nested.selector, "text": "WELL", "fallbackFont": "lato-bold" }));
    assert_eq!(logos(&edited).len(), 1);
    assert!(objects(&edited).iter().any(|o| o.text.as_deref() == Some("WELL") && o.selector.forms.len() == 2), "the nested placement reads WELL");
    // The outer form's panel is a separate object; the whole placement is a group.
    assert!(listed.iter().any(|o| o.selector.kind == ContentObjectKind::Path && o.selector.forms.len() == 1));
}

#[test]
fn a_gradient_seen_through_text_is_one_object_and_a_pattern_logo_recolours() {
    let listed = objects(CLIP);
    let gradient = listed.iter().find(|o| o.selector.kind == ContentObjectKind::Shading).unwrap();
    assert!(gradient.bounds[1] > 580.0 && gradient.bounds[3] < 690.0, "bounded by the letters it shows through: {:?}", gradient.bounds);
    let (deleted, _) = edit(CLIP, gradient, json!({ "kind": "delete" }));
    assert!(!has_text(&deleted, "WELLBOOK"), "the clipping text goes with the gradient");
    assert!(has_text(&deleted, "PRESS"));
    let press = text_run(&listed, "PRESS");
    assert!(press.has("pattern") && press.has("stroked"), "{:?}", press.traits);
    let (solid, _) = edit(CLIP, press, json!({ "kind": "recolor", "fill": { "r": 0, "g": 0.5, "b": 0 } }));
    assert_eq!(text_run(&objects(&solid), "PRESS").fill, Some([0.0, 0.5, 0.0]));
    let (gone, _) = edit(CLIP, press, json!({ "kind": "delete" }));
    assert!(!has_text(&gone, "PRESS") && has_text(&gone, "WELLBOOK"));
}

#[test]
fn a_raster_logo_is_deleted_with_its_image_and_mask() {
    let listed = objects(RASTER);
    let page = page_objects(RASTER, 1).unwrap();
    let image_count = count(RASTER, b"/Image");
    assert!(image_count >= 2, "the image and its soft mask");
    let selector_kind = listed.iter().find(|o| o.selector.kind == ContentObjectKind::Image);
    let (deleted, _) = match selector_kind {
        Some(image) => edit(RASTER, image, json!({ "kind": "delete" })),
        None => run(RASTER, json!({ "op": "deleteImage", "selector": page.images[0].placement.selector })),
    };
    let after = Document::load_mem(&deleted).unwrap();
    let images = after.objects.values().filter(|o| o.as_stream().is_ok_and(|s| s.dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Image".as_slice()))).count();
    assert_eq!(images, 0, "the image and its mask left the file");
    if let Some(image) = selector_kind {
        let (text_logo, _) = run(RASTER, json!({ "op": "replaceObjectWithText", "selector": image.selector, "text": "WELLBOOK", "font": "liberation-sans-bold", "color": { "r": 0.9, "g": 0.4, "b": 0.1 } }));
        assert!(has_text(&text_logo, "WELLBOOK"));
    }
}

#[test]
fn a_layer_is_one_object_and_its_content_is_removed_with_it() {
    let listed = objects(LAYER);
    let layer = listed.iter().find(|o| o.selector.kind == ContentObjectKind::Group && o.layer.as_deref() == Some("Logo layer")).expect("the layer group");
    let logo = text_run(&listed, "WELLBOOK");
    assert!(logo.has("optionalContent") && logo.layer.as_deref() == Some("Logo layer"), "{:?}", logo.traits);
    let (deleted, _) = edit(LAYER, layer, json!({ "kind": "delete" }));
    assert!(!has_text(&deleted, "WELLBOOK") && has_text(&deleted, "Chapter one"));
    assert_eq!(count(&deleted, b"Lato-Black"), 0, "the layer's font left the file");
    let (moved, _) = edit(LAYER, layer, MOVE());
    assert!(has_text(&moved, "WELLBOOK"));
}

// ---- dense text ----

#[test]
fn dense_text_runs_follow_kerning_ligatures_rotation_and_spacing() {
    let listed = objects(DENSE);
    let heading = text_run(&listed, "Quarterly Field Report");
    assert!(heading.bounds[2] - heading.bounds[0] > 150.0, "Tc, Tw and Tz widen and narrow the heading: {:?}", heading.bounds);
    let rotated = text_run(&listed, "Rotated thirty degrees");
    assert!(rotated.has("rotated"));
    let ligature = text_run(&listed, "Pfiles");
    assert_eq!(ligature.text.as_deref(), Some("Pfiles"), "the fi ligature reads as two letters");
    let vertical = text_run(&listed, "Vertical margin note");
    assert!(vertical.bounds[3] - vertical.bounds[1] > vertical.bounds[2] - vertical.bounds[0], "drawn upwards");
    // Lines of the columns are separate runs.
    let lines = listed.iter().filter(|o| o.selector.kind == ContentObjectKind::Text).count();
    assert!(lines >= 14, "{lines} runs");

    let (out, _) = run(DENSE, json!({ "op": "editObjectText", "selector": rotated.selector, "text": "Rotated forty degrees", "fallbackFont": "liberation-sans-regular" }));
    assert!(has_text(&out, "Rotated forty degrees"));
    assert!(text_run(&objects(&out), "Rotated forty degrees").has("rotated"), "the rotation is kept");

    // Deleting one line leaves every other line where it was.
    let line = listed.iter().filter(|o| o.selector.kind == ContentObjectKind::Text && o.bounds[0] < 300.0 && o.bounds[1] < 690.0 && o.bounds[1] > 600.0).nth(1).unwrap();
    let (deleted, _) = edit(DENSE, line, json!({ "kind": "delete" }));
    let removed = squash(line.text.as_deref().unwrap());
    // The same words also run across two other lines: one occurrence fewer.
    let occurrences = |bytes: &[u8]| squash(&text(bytes)).matches(removed.as_str()).count();
    assert_eq!(occurrences(&deleted) + 1, occurrences(DENSE), "the line's text is gone");
    assert!(!objects(&deleted).iter().any(|o| o.selector.kind == ContentObjectKind::Text && o.text.as_deref().map(squash) == Some(removed.clone())));
    let others = |bytes: &[u8]| -> Vec<(String, [i64; 4])> {
        objects(bytes)
            .into_iter()
            .filter(|o| o.selector.kind == ContentObjectKind::Text && o.text.as_deref().map(squash) != Some(removed.clone()))
            .map(|o| (o.text.unwrap_or_default(), o.bounds.map(|v| (v * 10.0).round() as i64)))
            .collect()
    };
    assert_eq!(others(&deleted), others(DENSE), "the other lines keep their place");
    // Moving one run out of a text object keeps the rest of that object in place.
    let (moved, _) = edit(DENSE, line, MOVE());
    let mut before_others = others(DENSE);
    let mut after_others = others(&moved);
    before_others.sort();
    after_others.sort();
    assert_eq!(after_others, before_others);
    undo_restores(DENSE, &moved);
}

// ---- vector art, annotations and an inline image ----

#[test]
fn grouped_vector_art_is_one_object_and_annotations_stay_annotations() {
    let listed = objects(VECTOR);
    let group = listed.iter().find(|o| o.selector.kind == ContentObjectKind::Group).expect("the circle, star and frame group");
    assert!(group.parts.len() >= 3);
    assert!(group.can_recolor);
    let inline = listed.iter().find(|o| o.selector.kind == ContentObjectKind::InlineImage).expect("the inline image");
    let (red, _) = edit(VECTOR, group, json!({ "kind": "recolor", "fill": { "r": 1, "g": 0, "b": 0 } }));
    let (r, g, b) = pixel(&red, 210.0, 570.0);
    assert!(r > 200 && g < 60 && b < 60, "the circle is red: {:?}", (r, g, b));
    let (deleted, _) = edit(VECTOR, inline, json!({ "kind": "delete" }));
    assert!(!objects(&deleted).iter().any(|o| o.selector.kind == ContentObjectKind::InlineImage));
    let (gone, _) = edit(VECTOR, group, json!({ "kind": "delete" }));
    assert!(!objects(&gone).iter().any(|o| o.selector.kind == ContentObjectKind::Group));
    // Annotations are not page content: deleting the rectangle under the
    // square annotation keeps the annotation.
    let panel = listed.iter().find(|o| o.selector.kind == ContentObjectKind::Path && o.bounds.map(f64::round) == [320.0, 280.0, 520.0, 360.0]).unwrap();
    let (out, _) = edit(VECTOR, panel, json!({ "kind": "delete" }));
    let document = Document::load_mem(&out).unwrap();
    let page = *document.get_pages().get(&1).unwrap();
    assert_eq!(document.get_dictionary(page).unwrap().get(b"Annots").unwrap().as_array().unwrap().len(), 2);
    let (back, _) = edit(VECTOR, group, json!({ "kind": "sendToBack" }));
    undo_restores(VECTOR, &back);
    // Replaced with text, the group's box holds the whole word (a square box
    // shrinks the size rather than letting the text run off the page).
    let (words, _) = run(VECTOR, json!({ "op": "replaceObjectWithText", "selector": group.selector, "text": "WELLFRIEND", "font": "liberation-sans-bold" }));
    let after = objects(&words);
    let placed = text_run(&after, "WELLFRIEND");
    assert!(placed.bounds[0] >= group.bounds[0] - 1.0 && placed.bounds[2] <= group.bounds[2] + 1.0, "inside the group's box: {:?} vs {:?}", placed.bounds, group.bounds);
    assert!(placed.bounds[1] >= group.bounds[1] - 40.0 && placed.bounds[3] <= group.bounds[3] + 40.0, "{:?}", placed.bounds);
}

// ---- runs inside one text object ----

#[test]
fn a_run_cut_out_of_its_text_object_leaves_the_rest_in_place() {
    let source = {
        let mut document = Document::with_version("1.7");
        let pages = document.new_object_id();
        let font = document.add_object(lopdf::dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
        let content = b"BT /F1 12 Tf 72 700 Td (Hello) Tj 120 0 Td (World) Tj 0 -14 Td (Next line) Tj (continues) ' ET".to_vec();
        let contents = document.add_object(lopdf::Stream::new(lopdf::dictionary! {}, content));
        let page = document.add_object(lopdf::dictionary! {
            "Type" => "Page", "Parent" => lopdf::Object::Reference(pages), "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => lopdf::Object::Reference(contents), "Resources" => lopdf::dictionary! { "Font" => lopdf::dictionary! { "F1" => lopdf::Object::Reference(font) } },
        });
        document.objects.insert(pages, lopdf::Object::Dictionary(lopdf::dictionary! { "Type" => "Pages", "Kids" => vec![lopdf::Object::Reference(page)], "Count" => 1 }));
        let catalog = document.add_object(lopdf::dictionary! { "Type" => "Catalog", "Pages" => lopdf::Object::Reference(pages) });
        document.trailer.set("Root", lopdf::Object::Reference(catalog));
        let mut out = Vec::new();
        document.save_to(&mut out).unwrap();
        out
    };
    let listed = objects(&source);
    let texts: Vec<_> = listed.iter().filter(|o| o.selector.kind == ContentObjectKind::Text).map(|o| o.text.clone().unwrap()).collect();
    assert_eq!(texts, ["Hello", "World", "Next line", "continues"]);
    let world = text_run(&listed, "World");
    let place = |bytes: &[u8], needle: &str| text_run(&objects(bytes), needle).bounds.map(|v| (v * 100.0).round() as i64);
    for edit_value in [json!({ "kind": "delete" }), MOVE(), json!({ "kind": "recolor", "fill": { "r": 1, "g": 0, "b": 0 } }), json!({ "kind": "bringToFront" })] {
        let (out, _) = edit(&source, world, edit_value.clone());
        for other in ["Hello", "Next line", "continues"] {
            assert_eq!(place(&out, other), place(&source, other), "{edit_value}: {other} stays");
        }
    }
    let (out, _) = run(&source, json!({ "op": "editObjectText", "selector": world.selector, "text": "Earth" }));
    assert!(text(&out).contains("Earth"));
    assert_eq!(place(&out, "Next line"), place(&source, "Next line"));
}

#[test]
fn stale_selectors_and_bad_text_edits_are_refused() {
    let listed = objects(BOOK);
    let logo = text_run(&listed, "WELLBOOK");
    let stale = op(json!({ "op": "editObjectText", "selector": logo.selector, "text": "X", "expected": "SOMETHING ELSE" }));
    assert_eq!(apply_operation(BOOK, &stale, &library(), true).unwrap_err().code, "EDITOR_OBJECT_STALE");
    let list = |value| serde_json::from_value::<wellfriendpdf_editor::OperationList>(json!({ "version": 1, "operations": [value] })).unwrap().validate();
    assert!(list(json!({ "op": "editObjectText", "selector": logo.selector, "text": "two\nlines" })).is_err());
    assert!(list(json!({ "op": "replaceObjectWithText", "selector": logo.selector, "text": "", "font": "liberation-sans-bold" })).is_err());
    assert!(list(json!({ "op": "replaceObjectWithText", "selector": logo.selector, "text": "A", "font": "Not A Font" })).is_err());
    let missing = op(json!({ "op": "replaceObjectWithText", "selector": logo.selector, "text": "A", "font": "unknown-font" }));
    assert_eq!(apply_operation(BOOK, &missing, &library(), true).unwrap_err().code, "EDITOR_FONT_UNAVAILABLE");
}
