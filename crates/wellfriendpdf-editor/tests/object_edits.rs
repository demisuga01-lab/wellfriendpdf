//! Milestone 3: every image placement and path group is selectable and
//! editable (`content_objects`, `editObject`), and every operation kind has
//! an exact object-level inverse (`delta`).

use base64::Engine as _;
use lopdf::content::Content;
use lopdf::{dictionary, Document, Object, Stream};
use serde_json::json;
use wellfriendpdf_editor::content_objects::{page_content_objects, ContentObject, ContentObjectKind};
use wellfriendpdf_editor::delta::{self, Direction, ObjectDelta};
use wellfriendpdf_editor::{apply_operation, page_objects, EditorOperation, MemoryFonts};
use wellfriendpdf_engine::{AuthorPageSize, Color, ContentEngine, GraphicsStyle, PageTextLayout, PdfBuilder, TextStyle};

const SANS: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Regular.ttf");
const SANS_BOLD: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSans-Bold.ttf");
const SERIF: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Regular.ttf");
const SERIF_BOLD: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Bold.ttf");
const FORM: &[u8] = include_bytes!("../../../tests/corpus/pdfs/pdfjs/form_two_pages.pdf");
const WIKIPEDIA: &[u8] = include_bytes!("../../../tests/corpus/pdfs/pdfjs/pdfjs_wikipedia.pdf");

fn library() -> MemoryFonts {
    let mut fonts = MemoryFonts::new();
    fonts.insert("liberation-sans-regular", SANS.to_vec());
    fonts.insert("liberation-sans-bold", SANS_BOLD.to_vec());
    fonts.insert("liberation-serif-regular", SERIF.to_vec());
    fonts.insert("liberation-serif-bold", SERIF_BOLD.to_vec());
    fonts.insert("noto-sans-regular", SANS.to_vec());
    fonts
}

fn op(value: serde_json::Value) -> EditorOperation {
    serde_json::from_value(value).unwrap()
}

// ---- inline images survive content stream round trips ----

#[test]
fn filtered_and_named_colour_space_inline_images_round_trip() {
    let data: &[u8] = b"\xff\xd8\xff\xe0 EIx \x00\x01binary\xff\xd9";
    let mut source = b"q 10 0 0 10 0 0 cm BI /W 2 /H 2 /CS /RGB /BPC 8 /F /DCT ID ".to_vec();
    source.extend_from_slice(data);
    source.extend_from_slice(b"\nEI Q BI /W 2 /H 1 /CS [/I /RGB 1 <000000ffffff>] /BPC 8 ID \x00\x01\nEI BI /W 1 /H 1 /CS /CS0 /BPC 8 ID z EI 1 0 0 rg 0 0 5 5 re f");
    let content = Content::decode(&source).unwrap();
    let ops: Vec<&str> = content.operations.iter().map(|o| o.operator.as_str()).collect();
    assert_eq!(ops, ["q", "cm", "BI", "Q", "BI", "BI", "rg", "re", "f"]);
    let data_of = |content: &Content, index: usize| match content.operations[index].operands.first() {
        Some(Object::Stream(stream)) => stream.content.clone(),
        _ => panic!("inline image {index} dropped"),
    };
    let again = Content::decode(&content.encode().unwrap()).unwrap();
    for (index, expected) in [(2usize, data.to_vec()), (4, b"\x00\x01".to_vec()), (5, b"z".to_vec())] {
        assert_eq!(data_of(&content, index), expected);
        assert_eq!(data_of(&again, index), expected, "written back exactly");
    }
    assert_eq!(again.operations.len(), 9);
}

// ---- a page with every kind of object ----

fn image(width: i64, height: i64, rgb: [u8; 3], smask: Option<lopdf::ObjectId>) -> Stream {
    let mut dict = dictionary! {
        "Type" => "XObject", "Subtype" => "Image", "Width" => width, "Height" => height,
        "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
    };
    if let Some(mask) = smask {
        dict.set("SMask", Object::Reference(mask));
    }
    Stream::new(dict, rgb.repeat((width * height) as usize))
}

/// Inline images (unfiltered RGB and ASCIIHex grey), an image with a soft
/// mask under a clip in nested state, a pattern-filled path, a path that
/// clips and paints with a path it clips, a shading in its clip, and a form
/// (an image and a path) placed twice.
fn every_kind() -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let pages = document.new_object_id();
    let mask = document.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 2, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
        vec![255, 128, 128, 255],
    ));
    let masked = document.add_object(image(2, 2, [0, 160, 0], Some(mask)));
    let plain = document.add_object(image(2, 2, [0, 0, 200], None));
    let function = document.add_object(dictionary! { "FunctionType" => 2, "Domain" => vec![0.into(), 1.into()], "C0" => vec![1.into(), 0.into(), 0.into()], "C1" => vec![0.into(), 0.into(), 1.into()], "N" => 1 });
    let shading = document.add_object(dictionary! {
        "ShadingType" => 2, "ColorSpace" => "DeviceRGB",
        "Coords" => vec![100.into(), 0.into(), 300.into(), 0.into()], "Function" => Object::Reference(function),
    });
    let pattern = document.add_object(dictionary! { "Type" => "Pattern", "PatternType" => 2, "Shading" => Object::Reference(shading) });
    let form = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im2" => Object::Reference(plain) } },
        },
        b"q 50 0 0 50 0 0 cm /Im2 Do Q 0 0 1 rg 0 60 40 20 re f".to_vec(),
    ));
    let content = b"q 40 0 0 40 300 600 cm BI /W 2 /H 2 /CS /RGB /BPC 8 ID \xff\x00\x00\xff\x00\x00\xff\x00\x00\xff\x00\x00\nEI Q
q 30 0 0 30 400 600 cm BI /W 2 /H 2 /CS /G /BPC 8 /F /AHx ID 00ff00ff> EI Q
q 50 500 100 100 re W n q 60 0 0 60 60 520 cm /Im1 Do Q Q
/Pattern cs /P1 scn 200 500 80 60 re f
q 1 0 0 rg 300 500 50 50 re W f 0 0 1 rg 310 510 100 20 re f Q
q 100 300 200 100 re W n /Sh1 sh Q
q 1 0 0 1 50 100 cm /Fm1 Do Q q 1 0 0 1 250 100 cm /Fm1 Do Q"
        .to_vec();
    let contents = document.add_object(Stream::new(dictionary! {}, content));
    let page = document.add_object(dictionary! {
        "Type" => "Page", "Parent" => Object::Reference(pages), "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Contents" => Object::Reference(contents),
        "Resources" => dictionary! {
            "XObject" => dictionary! { "Im1" => Object::Reference(masked), "Fm1" => Object::Reference(form) },
            "Pattern" => dictionary! { "P1" => Object::Reference(pattern) },
            "Shading" => dictionary! { "Sh1" => Object::Reference(shading) },
        },
    });
    document.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }));
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => Object::Reference(pages) });
    document.trailer.set("Root", Object::Reference(catalog));
    let mut out = Vec::new();
    document.save_to(&mut out).unwrap();
    out
}

fn objects(bytes: &[u8]) -> Vec<ContentObject> {
    let document = Document::load_mem(bytes).unwrap();
    page_content_objects(&document, 1).unwrap().objects
}

fn summary(objects: &[ContentObject]) -> Vec<String> {
    objects
        .iter()
        .map(|o| format!("{:?}:{}:{}", o.selector.kind, o.selector.forms.len(), o.traits.join("+")))
        .collect()
}

fn pixel(bytes: &[u8], x: f64, y: f64) -> (u8, u8, u8) {
    let raw = ContentEngine::open_bytes(bytes.to_vec()).unwrap().render_page(1, 72).unwrap().into_raw_image();
    let i = ((792.0 - y) as usize * raw.width as usize + x as usize) * raw.channels as usize;
    (raw.pixels[i], raw.pixels[i + 1], raw.pixels[i + 2])
}

fn edit(bytes: &[u8], object: &ContentObject, edit: serde_json::Value) -> Vec<u8> {
    let operation = op(json!({ "op": "editObject", "selector": object.selector, "edit": edit }));
    apply_operation(bytes, &operation, &library(), false).unwrap().0
}

#[test]
fn every_image_placement_and_path_group_is_listed() {
    let source = every_kind();
    let listed = objects(&source);
    assert_eq!(
        summary(&listed),
        [
            "InlineImage:0:inlineImage",
            "InlineImage:0:inlineImage",
            "Image:0:softMask+clipped+nestedState",
            "Path:0:pattern",
            "Path:0:clipAndPaint",
            "Path:0:clipped",
            "Shading:0:shading+clipped",
            "Image:1:inForm",
            "Path:1:inForm",
            "Image:1:inForm",
            "Path:1:inForm",
        ]
    );
    let red = &listed[0];
    assert_eq!(red.bounds.map(f64::round), [300.0, 600.0, 340.0, 640.0]);
    assert_eq!(listed[7].bounds.map(f64::round), [50.0, 100.0, 100.0, 150.0]);
    assert_eq!(listed[9].bounds.map(f64::round), [250.0, 100.0, 300.0, 150.0]);
    // The recognisers certify none of them; page_objects lists all eleven.
    let page = page_objects(&source, 1).unwrap();
    assert_eq!(page.images.len() + page.vectors.len(), 0);
    assert_eq!(page.objects.len(), 11);
    assert_eq!((page.unsupported_images, page.unsupported_vectors), (0, 0));
}

#[test]
fn an_inline_image_moves_with_its_samples() {
    let source = every_kind();
    assert_eq!(pixel(&source, 320.0, 620.0), (255, 0, 0));
    let moved = edit(&source, &objects(&source)[0], json!({ "kind": "transform", "matrix": { "a": 1, "b": 0, "c": 0, "d": 1, "e": 150, "f": 0 } }));
    assert_eq!(pixel(&moved, 470.0, 620.0), (255, 0, 0), "the inline image is drawn at its new place");
    assert_eq!(pixel(&moved, 320.0, 620.0), (255, 255, 255), "and no longer at the old one");
    let after = objects(&moved);
    assert_eq!(after.len(), 11);
    assert_eq!(after[0].bounds.map(f64::round), [450.0, 600.0, 490.0, 640.0]);
    // The ASCIIHex inline image after it is untouched.
    assert_eq!(after[1].selector.expected_sha256, objects(&source)[1].selector.expected_sha256);

    let resized = edit(&source, &objects(&source)[1], json!({ "kind": "transform", "matrix": { "a": 2, "b": 0, "c": 0, "d": 2, "e": -400, "f": -600 } }));
    assert_eq!(objects(&resized)[1].bounds.map(f64::round), [400.0, 600.0, 460.0, 660.0]);
    let deleted = edit(&source, &objects(&source)[0], json!({ "kind": "delete" }));
    assert_eq!(objects(&deleted).len(), 10);
    assert_eq!(pixel(&deleted, 320.0, 620.0), (255, 255, 255));
}

#[test]
fn a_form_placed_twice_is_copied_for_the_edited_placement() {
    let source = every_kind();
    let second = objects(&source)[9].clone();
    let moved = edit(&source, &second, json!({ "kind": "transform", "matrix": { "a": 1, "b": 0, "c": 0, "d": 1, "e": 10, "f": 5 } }));
    let after = objects(&moved);
    assert_eq!(after.len(), 11);
    assert_eq!(after[7].bounds.map(f64::round), [50.0, 100.0, 100.0, 150.0], "the other placement keeps the original");
    assert_eq!(after[9].bounds.map(f64::round), [260.0, 105.0, 310.0, 155.0]);
    assert_ne!(after[9].selector.forms[0].object, after[7].selector.forms[0].object, "a copy of the form");
    assert_eq!(after[9].selector.forms[0].name, "Fm1c1");
    // Moving past the form box grows the copy's box so nothing is cut off.
    let far = edit(&moved, &after[9], json!({ "kind": "transform", "matrix": { "a": 1, "b": 0, "c": 0, "d": 1, "e": 0, "f": 80 } }));
    assert_eq!(pixel(&far, 285.0, 220.0), (0, 0, 200), "drawn beyond the old box");
    // Deleting the path of the first placement leaves the second alone.
    let deleted = edit(&source, &objects(&source)[8], json!({ "kind": "delete" }));
    assert_eq!(summary(&objects(&deleted)).len(), 10);
    assert_eq!(pixel(&deleted, 60.0, 170.0), (255, 255, 255));
    assert_eq!(pixel(&deleted, 260.0, 170.0), (0, 0, 255));
}

#[test]
fn a_path_that_clips_and_paints_keeps_its_clip() {
    let source = every_kind();
    let listed = objects(&source);
    let deleted = edit(&source, &listed[4], json!({ "kind": "delete" }));
    let after = objects(&deleted);
    assert_eq!(after.len(), 10);
    assert!(after[4].has("clipped"), "the blue bar is still clipped: {:?}", summary(&after));
    assert_eq!(pixel(&deleted, 340.0, 520.0), (0, 0, 255));
    assert_eq!(pixel(&deleted, 380.0, 520.0), (255, 255, 255), "outside the kept clip");
    assert_eq!(pixel(&deleted, 305.0, 545.0), (255, 255, 255), "the red fill is gone");

    let moved = edit(&source, &listed[4], json!({ "kind": "transform", "matrix": { "a": 1, "b": 0, "c": 0, "d": 1, "e": 0, "f": -100 } }));
    assert_eq!(pixel(&moved, 305.0, 445.0), (255, 0, 0), "the red square moved");
    assert_eq!(pixel(&moved, 340.0, 520.0), (0, 0, 255), "the clip it set did not");
}

#[test]
fn pattern_paths_and_shadings_are_recoloured_moved_and_deleted() {
    let source = every_kind();
    let listed = objects(&source);
    let green = edit(&source, &listed[3], json!({ "kind": "recolor", "fill": { "r": 0, "g": 1, "b": 0 } }));
    assert_eq!(pixel(&green, 240.0, 530.0), (0, 255, 0));
    assert_eq!(objects(&green)[3].fill, Some([0.0, 1.0, 0.0]));
    let cmyk = edit(&source, &listed[3], json!({ "kind": "recolor", "fill": { "c": 0, "m": 0, "y": 0, "k": 1 } }));
    let (r, g, b) = pixel(&cmyk, 240.0, 530.0);
    assert!(r < 60 && g < 60 && b < 60, "CMYK black: {:?}", (r, g, b));

    let shading = &listed[6];
    assert_eq!(shading.bounds.map(f64::round), [100.0, 300.0, 300.0, 400.0]);
    let moved = edit(&source, shading, json!({ "kind": "transform", "matrix": { "a": 1, "b": 0, "c": 0, "d": 1, "e": 0, "f": -200 } }));
    assert_eq!(objects(&moved)[6].bounds.map(f64::round), [100.0, 100.0, 300.0, 200.0], "the shading moved with its clip");
    assert_eq!(pixel(&moved, 200.0, 350.0), (255, 255, 255));
    let deleted = edit(&source, shading, json!({ "kind": "delete" }));
    assert_eq!(objects(&deleted).len(), 10);
    assert_eq!(pixel(&deleted, 200.0, 350.0), (255, 255, 255));
    // The masked image under a clip.
    let image = &listed[2];
    let moved = edit(&source, image, json!({ "kind": "transform", "matrix": { "a": 0.5, "b": 0, "c": 0, "d": 0.5, "e": 30, "f": 260 } }));
    assert_eq!(objects(&moved)[2].bounds.map(f64::round), [60.0, 520.0, 90.0, 550.0]);
}

#[test]
fn stale_and_malformed_selectors_are_refused() {
    let source = every_kind();
    let listed = objects(&source);
    let mut selector = serde_json::to_value(&listed[0].selector).unwrap();
    selector["expectedSha256"] = json!("0".repeat(64));
    let stale = op(json!({ "op": "editObject", "selector": selector, "edit": { "kind": "delete" } }));
    assert_eq!(apply_operation(&source, &stale, &library(), false).unwrap_err().code, "EDITOR_OBJECT_STALE");
    let list = |value| serde_json::from_value::<wellfriendpdf_editor::OperationList>(json!({ "version": 1, "operations": [value] })).unwrap().validate();
    let mut bad = serde_json::to_value(&listed[0].selector).unwrap();
    bad["expectedSha256"] = json!("xyz");
    assert!(list(json!({ "op": "editObject", "selector": bad, "edit": { "kind": "delete" } })).is_err());
    assert!(list(json!({ "op": "editObject", "selector": listed[0].selector, "edit": { "kind": "transform", "matrix": { "a": 0, "b": 0, "c": 0, "d": 0, "e": 0, "f": 0 } } })).is_err());
    assert!(list(json!({ "op": "editObject", "selector": listed[0].selector, "edit": { "kind": "recolor" } })).is_err());
    assert!(list(json!({ "op": "editObject", "selector": listed[0].selector, "edit": { "kind": "recolor", "fill": { "spot": "Gold", "c": 0, "m": 0.2, "y": 0.9, "k": 0 } } })).is_err());
    // Recolouring an image is refused by the engine.
    let image = op(json!({ "op": "editObject", "selector": listed[2].selector, "edit": { "kind": "recolor", "fill": { "r": 1, "g": 0, "b": 0 } } }));
    assert_eq!(apply_operation(&source, &image, &library(), false).unwrap_err().code, "EDITOR_OBJECT_REFUSED");
}

// ---- undo by object deltas: every operation kind ----

fn report_pdf(pages: usize) -> Vec<u8> {
    let mut builder = PdfBuilder::new();
    let serif = builder.register_font_bytes("Body-Serif", SERIF.to_vec()).unwrap();
    let bold = builder.register_font_bytes("Head-Bold", SANS_BOLD.to_vec()).unwrap();
    let image = builder.add_rgb_image(4, 4, [200u8, 40, 40].repeat(16)).unwrap();
    for index in 0..pages {
        let page = builder.add_page(AuthorPageSize::custom(612.0, 792.0));
        page.draw_text(format!("Annual Summary {}", index + 1), 72.0, 720.0, &TextStyle::new(bold, 18.0)).unwrap();
        page.draw_text("Revenue grew in every region this year.", 72.0, 700.0, &TextStyle::new(serif, 12.0)).unwrap();
        page.draw_text("Costs fell slightly over the same period.", 72.0, 686.0, &TextStyle::new(serif, 12.0)).unwrap();
        page.draw_rect(72.0, 420.0, 120.0, 60.0, &GraphicsStyle::fill(Color::device_rgb(0.2, 0.4, 0.8)));
        page.draw_image(image, 300.0, 400.0, 120.0, 90.0);
    }
    builder.to_bytes().unwrap()
}

fn png() -> Vec<u8> {
    let raw = wellfriendpdf_engine::RawImage { width: 8, height: 8, channels: 4, bits_per_sample: 8, pixels: [10u8, 160, 60, 200].repeat(64) };
    wellfriendpdf_engine::ImageEncoder::encode_png(&raw).unwrap()
}

fn text_range(bytes: &[u8], needle: &str) -> (usize, usize, String) {
    let layout = PageTextLayout::from_bytes(bytes, 1).unwrap();
    let text: Vec<char> = layout.text.chars().collect();
    let n: Vec<char> = needle.chars().collect();
    let start = (0..=text.len() - n.len()).find(|&i| text[i..i + n.len()] == n[..]).unwrap();
    (start, start + n.len(), needle.to_owned())
}

/// One operation of every kind in `ops.rs`, each on a document it applies to.
fn every_operation() -> Vec<(&'static str, Vec<u8>, EditorOperation)> {
    let report = report_pdf(3);
    let objects = page_objects(&report, 1).unwrap();
    let image = serde_json::to_value(&objects.images[0].placement.selector).unwrap();
    let image_object = serde_json::to_value(&objects.images[0].image.as_ref().unwrap().selector).unwrap();
    let vector = serde_json::to_value(&objects.vectors[0].selector).unwrap();
    let (start, end, expected) = text_range(&report, "region");
    let layout = PageTextLayout::from_bytes(&report, 1).unwrap();
    let paragraph = layout.paragraphs.iter().find(|p| layout.text_in(p.range).starts_with("Revenue")).unwrap();
    let kinds = every_kind();
    let kinds_objects = super_objects(&kinds);
    let png64 = base64::engine::general_purpose::STANDARD.encode(png());
    let other = base64::engine::general_purpose::STANDARD.encode(report_pdf(2));
    let field = page_objects(FORM, 1)
        .unwrap()
        .forms
        .into_iter()
        .find(|f| format!("{:?}", f.kind).to_lowercase().contains("text") && f.value_editable)
        .unwrap();
    let line = json!([{ "text": "Revenue grew", "rect": { "x": 72.0, "y": 697.0, "width": 80.0, "height": 12.0 } }]);
    let r = |page: &Vec<u8>| page.clone();
    vec![
        ("replaceText", r(&report), op(json!({ "op": "replaceText", "page": 1, "start": start, "end": end, "expected": expected, "text": "market" }))),
        ("transformImage", r(&report), op(json!({ "op": "transformImage", "selector": image, "matrix": { "a": 60.0, "b": 0.0, "c": 0.0, "d": 45.0, "e": 320.0, "f": 300.0 } }))),
        ("deleteImage", r(&report), op(json!({ "op": "deleteImage", "selector": image }))),
        ("replaceImage", r(&report), op(json!({ "op": "replaceImage", "selector": image_object, "image": { "mimeType": "image/png", "base64": png64 } }))),
        ("editVector", r(&report), op(json!({ "op": "editVector", "selector": vector, "mutation": { "kind": "RECOLOR", "destination_fill_color": { "red": 1.0, "green": 0.0, "blue": 0.0 }, "destination_stroke_color": null } }))),
        ("editObject", r(&kinds), op(json!({ "op": "editObject", "selector": kinds_objects[9].selector, "edit": { "kind": "transform", "matrix": { "a": 1, "b": 0, "c": 0, "d": 1, "e": 10, "f": 5 } } }))),
        ("addText", r(&report), op(json!({ "op": "addText", "page": 1, "rect": { "x": 72.0, "y": 300.0, "width": 300.0, "height": 40.0 }, "text": "Approved", "font": "liberation-sans-bold", "size": 14.0, "color": { "r": 0.0, "g": 0.2, "b": 0.6 } }))),
        ("addImage", r(&report), op(json!({ "op": "addImage", "page": 1, "rect": { "x": 450.0, "y": 600.0, "width": 60.0, "height": 60.0 }, "image": { "mimeType": "image/png", "base64": png64 } }))),
        ("addShape", r(&report), op(json!({ "op": "addShape", "page": 1, "shape": "ellipse", "rect": { "x": 400.0, "y": 200.0, "width": 80.0, "height": 40.0 }, "stroke": { "r": 0.0, "g": 0.0, "b": 0.0 }, "lineWidth": 2.0 }))),
        ("addLine", r(&report), op(json!({ "op": "addLine", "page": 1, "from": [72.0, 370.0], "to": [540.0, 370.0], "color": { "r": 0.5, "g": 0.5, "b": 0.5 } }))),
        ("addWhiteout", r(&report), op(json!({ "op": "addWhiteout", "page": 1, "rect": { "x": 500.0, "y": 700.0, "width": 40.0, "height": 40.0 } }))),
        ("addLink", r(&report), op(json!({ "op": "addLink", "page": 1, "rect": { "x": 72.0, "y": 636.0, "width": 200.0, "height": 16.0 }, "url": "https://example.com/" }))),
        ("redact", r(&report), op(json!({ "op": "redact", "page": 1, "rect": { "x": 70.0, "y": 684.0, "width": 250.0, "height": 12.0 } }))),
        ("restyleText", r(&report), op(json!({ "op": "restyleText", "page": 1, "start": paragraph.range[0], "end": paragraph.range[1], "expected": layout.text_in(paragraph.range), "font": "liberation-serif-bold", "size": 12.0 }))),
        ("cropImage", r(&report), op(json!({ "op": "cropImage", "selector": image, "crop": { "left": 0.25, "bottom": 0.0, "right": 1.0, "top": 0.75 } }))),
        ("setImageOpacity", r(&report), op(json!({ "op": "setImageOpacity", "selector": image, "opacity": 0.4 }))),
        ("rotatePage", r(&report), op(json!({ "op": "rotatePage", "page": 2, "rotation": 90 }))),
        ("deletePage", r(&report), op(json!({ "op": "deletePage", "page": 2 }))),
        ("movePage", r(&report), op(json!({ "op": "movePage", "page": 3, "to": 1 }))),
        ("insertBlankPage", r(&report), op(json!({ "op": "insertBlankPage", "after": 1, "width": 595.0, "height": 842.0 }))),
        ("insertPages", r(&report), op(json!({ "op": "insertPages", "after": 1, "pdf": other, "pages": [2] }))),
        ("addAnnotation", r(&report), op(json!({ "op": "addAnnotation", "page": 1, "kind": "note", "rect": { "x": 400.0, "y": 700.0, "width": 20.0, "height": 20.0 }, "color": { "r": 1.0, "g": 0.85, "b": 0.0 }, "contents": "Reviewed" }))),
        ("setFormField", FORM.to_vec(), op(json!({ "op": "setFormField", "name": field.name, "value": "Asha Rao" }))),
        ("addTextLayer", r(&report), op(json!({ "op": "addTextLayer", "page": 1, "lines": line }))),
        ("convertOcrText", r(&report), op(json!({ "op": "convertOcrText", "page": 1, "lines": line }))),
    ]
}

fn super_objects(bytes: &[u8]) -> Vec<ContentObject> {
    objects(bytes)
}

fn graph(bytes: &[u8]) -> Document {
    Document::load_mem(bytes).unwrap()
}

#[test]
fn every_operation_kind_has_an_exact_object_level_inverse() {
    let all = every_operation();
    let mut names: Vec<&str> = all.iter().map(|(name, _, _)| *name).collect();
    names.sort_unstable();
    let mut expected: Vec<&str> = [
        "replaceText", "transformImage", "deleteImage", "replaceImage", "editVector", "editObject", "addText", "addImage",
        "addShape", "addLine", "addWhiteout", "addLink", "redact", "restyleText", "cropImage", "setImageOpacity", "rotatePage",
        "deletePage", "movePage", "insertBlankPage", "insertPages", "addAnnotation", "setFormField", "addTextLayer", "convertOcrText",
    ]
    .to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected, "one case per operation kind");
    for (name, input, operation) in all {
        assert_eq!(operation.name(), name);
        let (output, _) = apply_operation(&input, &operation, &library(), true).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let computed = delta::compute(&input, &output).unwrap();
        assert!(!computed.entries.is_empty(), "{name} changed nothing");
        // The serialized delta reads back exactly.
        let bytes = computed.to_bytes();
        let read = ObjectDelta::from_bytes(&bytes).unwrap();
        assert_eq!(read.to_bytes(), bytes, "{name}: delta encoding");
        let undone = read.apply(&output, Direction::Backward).unwrap();
        let diff = delta::between(&graph(&undone), &graph(&input));
        assert!(
            diff.is_empty(),
            "{name}: undo restores the input's object graph; differs: {:?} trailer {:?} vs {:?} max {} vs {}",
            diff.entries.iter().map(|(id, a, b)| (id, a.is_some(), b.is_some())).collect::<Vec<_>>(),
            diff.trailer_before, diff.trailer_after, diff.max_before, diff.max_after
        );
        let (a, b) = (graph(&undone), graph(&input));
        assert_eq!(a.max_id, b.max_id, "{name}: the next new object gets the number the replay gives it");
        let redone = read.apply(&undone, Direction::Forward).unwrap();
        assert_eq!(graph(&redone).max_id, graph(&output).max_id, "{name}: redo numbering");
        assert!(delta::same_graph(&graph(&redone), &graph(&output)), "{name}: redo restores the output's object graph");
        // The engine renders the undone document as the input.
        let render = |b: &[u8]| ContentEngine::open_bytes(b.to_vec()).unwrap().render_page(1, 36).unwrap().into_raw_image().pixels;
        assert!(render(&undone) == render(&input), "{name}: undone page 1 renders as before");
        println!(
            "{name:<16} document {:>8} B  delta {:>7} B ({} objects)",
            output.len(),
            bytes.len(),
            read.entries.len()
        );
    }
}

#[test]
fn an_edit_after_an_undo_numbers_objects_as_the_replay_does() {
    let source = report_pdf(1);
    let first = op(json!({ "op": "addText", "page": 1, "rect": { "x": 72.0, "y": 300.0, "width": 300.0, "height": 40.0 }, "text": "One", "font": "liberation-sans-bold", "size": 14.0, "color": { "r": 0, "g": 0, "b": 0 } }));
    let second = op(json!({ "op": "addImage", "page": 1, "rect": { "x": 450.0, "y": 600.0, "width": 60.0, "height": 60.0 }, "image": { "mimeType": "image/png", "base64": base64::engine::general_purpose::STANDARD.encode(png()) } }));
    let third = op(json!({ "op": "addShape", "page": 1, "shape": "rectangle", "rect": { "x": 400.0, "y": 200.0, "width": 80.0, "height": 40.0 }, "fill": { "r": 1, "g": 0, "b": 0 } }));
    let fonts = library();
    let one = apply_operation(&source, &first, &fonts, true).unwrap().0;
    let two = apply_operation(&one, &second, &fonts, true).unwrap().0;
    let back = delta::compute(&one, &two).unwrap().apply(&two, Direction::Backward).unwrap();
    let browser = apply_operation(&back, &third, &fonts, true).unwrap().0;
    let server = apply_operation(&one, &third, &fonts, true).unwrap().0;
    let diff = delta::between(&graph(&browser), &graph(&server));
    assert!(diff.is_empty(), "the same objects under the same numbers: {:?} max {} vs {}", diff.entries.iter().map(|(id, a, b)| (id, a.is_some(), b.is_some())).collect::<Vec<_>>(), diff.max_before, diff.max_after);
}

#[test]
fn deltas_on_a_real_document_are_proportional_to_the_change() {
    let (start, end, expected) = text_range(WIKIPEDIA, "project");
    let objects = super_objects(WIKIPEDIA);
    let path = objects.iter().find(|o| o.selector.kind == ContentObjectKind::Path).unwrap();
    let operations = [
        op(json!({ "op": "replaceText", "page": 1, "start": start, "end": end, "expected": expected, "text": "projects" })),
        op(json!({ "op": "editObject", "selector": path.selector, "edit": { "kind": "transform", "matrix": { "a": 1, "b": 0, "c": 0, "d": 1, "e": 5, "f": 5 } } })),
        op(json!({ "op": "addText", "page": 1, "rect": { "x": 72.0, "y": 100.0, "width": 300.0, "height": 40.0 }, "text": "Reviewed", "font": "liberation-sans-regular", "size": 12.0, "color": { "r": 0, "g": 0, "b": 0 } })),
    ];
    let mut bytes = WIKIPEDIA.to_vec();
    let mut states = vec![bytes.clone()];
    let mut deltas = Vec::new();
    for operation in &operations {
        let (next, _) = apply_operation(&bytes, operation, &library(), true).unwrap();
        let computed = delta::compute(&bytes, &next).unwrap().to_bytes();
        println!("{:<12} snapshot {:>8} B  delta {:>7} B", operation.name(), next.len(), computed.len());
        assert!(computed.len() * 4 < next.len(), "{}: delta {} vs document {}", operation.name(), computed.len(), next.len());
        deltas.push(computed);
        bytes = next;
        states.push(bytes.clone());
    }
    // Undo all three, then redo all three.
    for (index, computed) in deltas.iter().enumerate().rev() {
        bytes = ObjectDelta::from_bytes(computed).unwrap().apply(&bytes, Direction::Backward).unwrap();
        assert!(delta::same_graph(&graph(&bytes), &graph(&states[index])));
    }
    for (index, computed) in deltas.iter().enumerate() {
        bytes = ObjectDelta::from_bytes(computed).unwrap().apply(&bytes, Direction::Forward).unwrap();
        assert!(delta::same_graph(&graph(&bytes), &graph(&states[index + 1])));
    }
}
