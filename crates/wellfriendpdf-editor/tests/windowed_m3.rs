//! Large (windowed) documents, milestone 3: page structure, form filling and
//! redaction as page-scoped saves. Structure and form edits are incremental
//! updates; a redaction is a full rewrite in which the removed content (and
//! every earlier revision holding it) exists nowhere.

use lopdf::{dictionary, Dictionary, Document, Object, Stream};
use serde_json::json;
use wellfriendpdf_editor::protocol::{apply_windows, apply_windows_path, WindowedList};
use wellfriendpdf_editor::windows;
use wellfriendpdf_engine::{ContentEngine, PageTextLayout};

const SERIF: &[u8] = include_bytes!("../../wellfriendpdf-edit/fonts/LiberationSerif-Regular.ttf");

fn fonts_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wf-editor-fonts-m3-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("liberation-serif-regular.ttf"), SERIF).unwrap();
    dir
}

fn list(value: serde_json::Value) -> WindowedList {
    serde_json::from_value(value).unwrap()
}

fn content(text: &str) -> Vec<u8> {
    format!("BT /F1 14 Tf 72 700 Td ({text}) Tj ET").into_bytes()
}

/// A document whose page tree has two levels: pages 1-3 under a node that
/// rotates them by 90 degrees, pages 4-6 under a plain node.
fn nested(pages: usize) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let root = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let resources = document.add_object(dictionary! { "Font" => dictionary! { "F1" => Object::Reference(font) } });
    let first = document.new_object_id();
    let second = document.new_object_id();
    let mut kids = (Vec::new(), Vec::new());
    for index in 0..pages {
        let node = if index < pages / 2 { first } else { second };
        let stream = document.add_object(Stream::new(Dictionary::new(), content(&format!("Page {} body", index + 1))));
        let page = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => Object::Reference(node), "Contents" => Object::Reference(stream),
        });
        if index < pages / 2 {
            kids.0.push(Object::Reference(page));
        } else {
            kids.1.push(Object::Reference(page));
        }
    }
    document.objects.insert(first, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Parent" => Object::Reference(root), "Count" => kids.0.len() as i64,
        "Kids" => kids.0, "Rotate" => 90,
    }));
    document.objects.insert(second, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Parent" => Object::Reference(root), "Count" => kids.1.len() as i64, "Kids" => kids.1,
    }));
    document.objects.insert(root, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Count" => pages as i64, "Kids" => vec![Object::Reference(first), Object::Reference(second)],
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Resources" => Object::Reference(resources),
    }));
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => Object::Reference(root) });
    document.trailer.set("Root", Object::Reference(catalog));
    let mut out = Vec::new();
    document.save_to(&mut out).unwrap();
    out
}

fn texts(bytes: &[u8]) -> Vec<String> {
    let engine = ContentEngine::open_bytes(bytes.to_vec()).unwrap();
    (1..=engine.page_count().unwrap())
        .map(|page| PageTextLayout::from_bytes(bytes, page).unwrap().text.trim().to_owned())
        .collect()
}

fn rotations(bytes: &[u8]) -> Vec<i32> {
    let engine = ContentEngine::open_bytes(bytes.to_vec()).unwrap();
    engine.document().get_pages().unwrap().iter().map(|p| p.rotate).collect()
}

#[test]
fn page_structure_is_an_incremental_page_tree_edit() {
    let source = nested(6);
    assert_eq!(rotations(&source), vec![90, 90, 90, 0, 0, 0]);
    let pdf = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(nested(2))
    };
    // Delete page 2, move (old) page 6 to the front, insert a blank page
    // after page 3 and the two pages of another PDF at the end; then type
    // on the inserted blank page and on (old) page 4.
    let saved = list(json!({
        "version": 1,
        "structure": [
            { "op": "deletePage", "page": 2 },
            { "op": "movePage", "page": 5, "to": 1 },
            { "op": "insertBlankPage", "after": 3, "width": 300.0, "height": 400.0 },
            { "op": "insertPages", "after": 6, "pdf": pdf, "pages": [] }
        ],
        "windows": [
            { "insert": 0, "operations": [
                { "op": "addText", "page": 1, "rect": { "x": 20.0, "y": 300.0, "width": 200.0, "height": 30.0 },
                  "text": "Typed on the inserted page", "font": "liberation-serif-regular", "size": 12.0, "color": { "r": 0.0, "g": 0.0, "b": 0.0 } }
            ] },
            { "page": 4, "operations": [
                { "op": "addText", "page": 1, "rect": { "x": 72.0, "y": 600.0, "width": 300.0, "height": 30.0 },
                  "text": "Edited page four", "font": "liberation-serif-regular", "size": 12.0, "color": { "r": 0.0, "g": 0.0, "b": 0.0 } }
            ] }
        ]
    }));
    let (bytes, result) = apply_windows(&source, &saved, fonts_dir(), false).unwrap();
    assert_eq!(result.save_mode, "INCREMENTAL");
    assert_eq!(result.page_count, 8);
    assert_eq!(result.operation_count, 6);
    assert!(bytes.starts_with(&source), "the original revision is untouched");
    let pages = texts(&bytes);
    assert_eq!(pages.len(), 8, "{pages:?}");
    assert!(pages[0].contains("Page 6 body"), "{pages:?}");
    assert!(pages[1].contains("Page 1 body"), "{pages:?}");
    assert!(pages[2].contains("Page 3 body"), "{pages:?}");
    assert!(pages[3].contains("Typed on the inserted page"), "{pages:?}");
    assert!(pages[4].contains("Page 4 body") && pages[4].contains("Edited page four"), "{pages:?}");
    assert!(pages[5].contains("Page 5 body"), "{pages:?}");
    assert!(pages[6].contains("Page 1 body") && pages[7].contains("Page 2 body"), "{pages:?}");
    // Inherited attributes travel with a moved page: page 6 was not rotated
    // and stays unrotated next to the rotated pages; the inserted PDF's
    // first page keeps its inherited rotation; the blank page has its size.
    assert_eq!(rotations(&bytes), vec![0, 90, 90, 0, 0, 0, 90, 0]);
    let engine = ContentEngine::open_bytes(bytes.clone()).unwrap();
    let blank = &engine.document().get_pages().unwrap()[3];
    assert_eq!(blank.media_box, [0.0, 0.0, 300.0, 400.0]);
    // Only the changed nodes, pages and the new objects were appended:
    // pages 1 and 3 (unedited, still in their place) are not redefined.
    let tail = String::from_utf8_lossy(&bytes[source.len()..]).into_owned();
    let ids = Document::load_mem(&source).unwrap().get_pages();
    for page in [1, 3] {
        assert!(!tail.contains(&format!("\n{} 0 obj", ids[&page].0)), "page {page} was rewritten");
    }
    assert!(Document::load_mem(&bytes).is_ok(), "a second reader opens the update");
}

/// QF-2598: a page with its own `/Rotate 0` under a rotated `/Pages` node
/// keeps 0 after an edit in its window (the window omits a zero rotation;
/// the spliced page must state it, or it inherits its parent's 90).
#[test]
fn an_edited_page_does_not_inherit_its_parents_rotation() {
    let source = nested(4);
    assert_eq!(rotations(&source), vec![90, 90, 0, 0]);
    let rotate = list(json!({
        "version": 1,
        "windows": [{ "page": 2, "operations": [{ "op": "rotatePage", "page": 1, "rotation": 0 }] }]
    }));
    let (rotated, _) = apply_windows(&source, &rotate, fonts_dir(), false).unwrap();
    assert_eq!(rotations(&rotated), vec![90, 0, 0, 0]);
    let whiteout = list(json!({
        "version": 1,
        "windows": [{ "page": 2, "operations": [
            { "op": "addWhiteout", "page": 1, "rect": { "x": 10.0, "y": 10.0, "width": 20.0, "height": 20.0 } }
        ] }]
    }));
    let (bytes, result) = apply_windows(&rotated, &whiteout, fonts_dir(), false).unwrap();
    assert_eq!(result.save_mode, "INCREMENTAL");
    assert_eq!(rotations(&bytes), vec![90, 0, 0, 0]);
}

#[test]
fn structure_lists_are_validated() {
    let source = nested(2);
    for (structure, code) in [
        (json!([{ "op": "deletePage", "page": 3 }]), "EDITOR_PAGE_NOT_FOUND"),
        (json!([{ "op": "deletePage", "page": 1 }, { "op": "deletePage", "page": 1 }]), "EDITOR_PAGES_REFUSED"),
        (json!([{ "op": "addWhiteout", "page": 1, "rect": { "x": 0.0, "y": 0.0, "width": 9.0, "height": 9.0 } }]), "EDITOR_OPERATION_INVALID"),
    ] {
        let error = apply_windows(&source, &list(json!({ "version": 1, "structure": structure })), fonts_dir(), false).unwrap_err();
        assert_eq!(error.code, code, "{structure}");
    }
    // A window cannot restructure, and an edit on a missing insert is refused.
    let error = apply_windows(&source, &list(json!({ "version": 1, "windows": [{ "page": 1, "operations": [{ "op": "deletePage", "page": 1 }] }] })), fonts_dir(), false).unwrap_err();
    assert_eq!(error.code, "EDITOR_OPERATION_INVALID");
    let error = apply_windows(
        &source,
        &list(json!({ "version": 1, "windows": [{ "insert": 0, "operations": [{ "op": "addWhiteout", "page": 1, "rect": { "x": 0.0, "y": 0.0, "width": 9.0, "height": 9.0 } }] }] })),
        fonts_dir(),
        false,
    )
    .unwrap_err();
    assert_eq!(error.code, "EDITOR_OPERATION_INVALID");
}

/// Page 1 plain, page 2 with a field `person.name` (a widget under a
/// parent field) and a checkbox `agree` on page 1.
fn form() -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let pages = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let one = document.add_object(Stream::new(Dictionary::new(), content("Form page one")));
    let two = document.add_object(Stream::new(Dictionary::new(), content("Form page two")));
    let page_one = document.new_object_id();
    let page_two = document.new_object_id();
    let person = document.new_object_id();
    let field_name = document.new_object_id();
    let name = document.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "Parent" => Object::Reference(field_name),
        "Rect" => vec![100.into(), 500.into(), 300.into(), 520.into()], "P" => Object::Reference(page_two), "F" => 4,
    });
    document.objects.insert(field_name, Object::Dictionary(dictionary! {
        "T" => Object::string_literal("name"), "FT" => "Tx", "Parent" => Object::Reference(person),
        "Kids" => vec![Object::Reference(name)], "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
    }));
    document.objects.insert(person, Object::Dictionary(dictionary! {
        "T" => Object::string_literal("person"), "Kids" => vec![Object::Reference(field_name)],
    }));
    let off = document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 12.into(), 12.into()] }, b"".to_vec()));
    let on = document.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 12.into(), 12.into()] }, b"0 g 2 2 8 8 re f".to_vec()));
    let agree = document.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn", "T" => Object::string_literal("agree"), "V" => "Off", "AS" => "Off",
        "Rect" => vec![100.into(), 400.into(), 112.into(), 412.into()], "P" => Object::Reference(page_one),
        "AP" => dictionary! { "N" => dictionary! { "Yes" => Object::Reference(on), "Off" => Object::Reference(off) } },
    });
    let resources = dictionary! { "Font" => dictionary! { "F1" => Object::Reference(font) } };
    document.objects.insert(page_one, Object::Dictionary(dictionary! {
        "Type" => "Page", "Parent" => Object::Reference(pages), "Contents" => Object::Reference(one),
        "Resources" => resources.clone(), "Annots" => vec![Object::Reference(agree)],
    }));
    document.objects.insert(page_two, Object::Dictionary(dictionary! {
        "Type" => "Page", "Parent" => Object::Reference(pages), "Contents" => Object::Reference(two),
        "Resources" => resources, "Annots" => vec![Object::Reference(name)],
    }));
    document.objects.insert(pages, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Count" => 2, "Kids" => vec![Object::Reference(page_one), Object::Reference(page_two)],
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    }));
    let helv = document.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let acro = document.add_object(dictionary! {
        "Fields" => vec![Object::Reference(person), Object::Reference(agree)],
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => Object::Reference(helv) } },
    });
    let catalog = document.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => Object::Reference(pages), "AcroForm" => Object::Reference(acro),
    });
    document.trailer.set("Root", Object::Reference(catalog));
    let mut out = Vec::new();
    document.save_to(&mut out).unwrap();
    out
}

fn field<'a>(document: &'a Document, name: &[u8]) -> (lopdf::ObjectId, &'a Dictionary) {
    document
        .objects
        .iter()
        .find_map(|(id, object)| match object {
            Object::Dictionary(d) if d.get(b"T").and_then(Object::as_str).is_ok_and(|t| t == name) => Some((*id, d)),
            _ => None,
        })
        .unwrap()
}

#[test]
fn a_form_field_is_filled_in_place_on_its_window() {
    let source = form();
    let engine = ContentEngine::open_bytes(source.clone()).unwrap();
    // The window of page 2 carries the field ancestry and an /AcroForm.
    let window = windows::extract(&engine, 2).unwrap();
    let window_doc = Document::load_mem(&window).unwrap();
    assert!(window_doc.catalog().unwrap().has(b"AcroForm"), "the window names its fields");
    let objects = wellfriendpdf_editor::page_objects(&window, 1).unwrap();
    let names: Vec<String> = objects.forms.iter().map(|f| f.name.clone()).collect();
    assert!(names.iter().any(|n| n == "person.name"), "{names:?}");

    let saved = list(json!({
        "version": 1,
        "windows": [
            { "page": 2, "operations": [{ "op": "setFormField", "name": "person.name", "value": "Ada Lovelace", "kind": "text", "page": 1 }] },
            { "page": 1, "operations": [{ "op": "setFormField", "name": "agree", "value": "true", "kind": "checkbox", "page": 1 }] }
        ]
    }));
    let (bytes, result) = apply_windows(&source, &saved, fonts_dir(), false).unwrap();
    assert_eq!(result.save_mode, "INCREMENTAL");
    assert!(bytes.starts_with(&source));
    let before = Document::load_mem(&source).unwrap();
    let after = Document::load_mem(&bytes).unwrap();
    // The field objects keep their numbers: the field tree sees the values.
    let (name_before, _) = field(&before, b"name");
    let (name_after, name) = field(&after, b"name");
    assert_eq!(name_before, name_after, "the field is rewritten in place");
    let value = name.get(b"V").and_then(Object::as_str).map(|v| String::from_utf8_lossy(v).into_owned());
    assert_eq!(value.as_deref().ok(), Some("Ada Lovelace"));
    let widget_of = |d: &Document| {
        d.objects
            .iter()
            .find(|(_, o)| o.as_dict().is_ok_and(|w| w.get(b"Parent").and_then(Object::as_reference).ok() == Some(name_before) && w.has(b"Rect")))
            .map(|(id, o)| (*id, o.as_dict().unwrap().clone()))
            .unwrap()
    };
    let (widget_before, _) = widget_of(&before);
    let (widget_after, widget) = widget_of(&after);
    assert_eq!(widget_before, widget_after, "the widget is rewritten in place");
    assert!(widget.has(b"AP"), "the widget has a new appearance");
    assert!(widget.get(b"P").and_then(Object::as_reference).is_ok(), "the widget keeps its page link");
    let (_, agree) = field(&after, b"agree");
    assert_eq!(agree.get(b"V").and_then(Object::as_name).unwrap(), b"Yes");
    let acro = |d: &Document| d.get_dictionary(d.catalog().unwrap().get(b"AcroForm").and_then(Object::as_reference).unwrap()).unwrap().get(b"Fields").unwrap().clone();
    let (fields, fields_before) = (acro(&after), acro(&before));
    assert_eq!(fields, fields_before, "the field tree is unchanged");
    // The fill shows on page 2 (its appearance stream is regenerated).
    let text = PageTextLayout::from_bytes(&bytes, 2).unwrap().text;
    assert!(text.contains("Form page two"), "{text}");
}

fn find(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

const SECRET: &str = "SECRET-4471-ALPHA";

fn encodings() -> Vec<Vec<u8>> {
    let hex_upper: String = SECRET.bytes().map(|b| format!("{b:02X}")).collect();
    let hex_lower = hex_upper.to_lowercase();
    let mut utf16 = Vec::new();
    for unit in SECRET.encode_utf16() {
        utf16.extend_from_slice(&unit.to_be_bytes());
    }
    vec![SECRET.as_bytes().to_vec(), hex_upper.into_bytes(), hex_lower.into_bytes(), utf16]
}

/// Revision 1 shows the secret on page 1; revision 2 (an incremental update)
/// replaces page 1's content with a revised one that still shows it, drawn
/// as a hex string. The secret is also in the document information, an
/// outline title, XMP metadata, a note annotation and a text field under the
/// redaction area, and in a comment elsewhere on the page.
fn secret_document() -> (Vec<u8>, Vec<u8>) {
    let mut document = Document::with_version("1.7");
    let pages = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let mut first = Stream::new(
        Dictionary::new(),
        format!("BT /F1 18 Tf 72 700 Td (Account {SECRET}) Tj ET BT /F1 12 Tf 72 500 Td (Public paragraph stays) Tj ET").into_bytes(),
    );
    first.compress().unwrap();
    let page_one_content = document.add_object(first);
    let mut second = Stream::new(Dictionary::new(), b"BT /F1 12 Tf 72 700 Td (Second page text) Tj ET".to_vec());
    second.compress().unwrap();
    let page_two_content = document.add_object(second);
    let page_one = document.new_object_id();
    let page_two = document.new_object_id();
    let note = document.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![80.into(), 700.into(), 100.into(), 720.into()],
        "Contents" => Object::string_literal(format!("Note about {SECRET}")), "P" => Object::Reference(page_one),
    });
    let comment = document.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![500.into(), 100.into(), 520.into(), 120.into()],
        "Contents" => Object::string_literal(format!("Reviewed {SECRET} twice")), "P" => Object::Reference(page_one),
    });
    let appearance = document.add_object(Stream::new(
        dictionary! { "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 200.into(), 20.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => Object::Reference(font) } } },
        format!("/Tx BMC BT /F1 10 Tf 2 5 Td ({SECRET}) Tj ET EMC").into_bytes(),
    ));
    let account = document.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => Object::string_literal("account"),
        "V" => Object::string_literal(SECRET), "Rect" => vec![300.into(), 695.into(), 500.into(), 715.into()],
        "P" => Object::Reference(page_one), "AP" => dictionary! { "N" => Object::Reference(appearance) },
    });
    let thumb = document.add_object(Stream::new(
        dictionary! { "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
        vec![128],
    ));
    let resources = dictionary! { "Font" => dictionary! { "F1" => Object::Reference(font) } };
    document.objects.insert(page_one, Object::Dictionary(dictionary! {
        "Type" => "Page", "Parent" => Object::Reference(pages), "Contents" => Object::Reference(page_one_content),
        "Resources" => resources.clone(), "Annots" => vec![Object::Reference(note), Object::Reference(comment), Object::Reference(account)],
        "Thumb" => Object::Reference(thumb),
    }));
    document.objects.insert(page_two, Object::Dictionary(dictionary! {
        "Type" => "Page", "Parent" => Object::Reference(pages), "Contents" => Object::Reference(page_two_content),
        "Resources" => resources,
    }));
    document.objects.insert(pages, Object::Dictionary(dictionary! {
        "Type" => "Pages", "Count" => 2, "Kids" => vec![Object::Reference(page_one), Object::Reference(page_two)],
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
    }));
    let outlines = document.new_object_id();
    let item = document.add_object(dictionary! {
        "Title" => Object::string_literal(format!("{SECRET} notes")), "Parent" => Object::Reference(outlines),
        "Dest" => vec![Object::Reference(page_one), "Fit".into()],
    });
    document.objects.insert(outlines, Object::Dictionary(dictionary! {
        "Type" => "Outlines", "First" => Object::Reference(item), "Last" => Object::Reference(item), "Count" => 1,
    }));
    let metadata = document.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        format!("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><dc:description>{SECRET}</dc:description></x:xmpmeta>").into_bytes(),
    ));
    let info = document.add_object(dictionary! {
        "Title" => Object::string_literal("Quarterly"), "Subject" => Object::string_literal(format!("Report on {SECRET}")),
    });
    let acro = document.add_object(dictionary! { "Fields" => vec![Object::Reference(account)], "DA" => Object::string_literal("/F1 10 Tf 0 g") });
    let catalog = document.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => Object::Reference(pages), "Outlines" => Object::Reference(outlines),
        "Metadata" => Object::Reference(metadata), "AcroForm" => Object::Reference(acro),
    });
    document.trailer.set("Root", Object::Reference(catalog));
    document.trailer.set("Info", Object::Reference(info));
    document.trailer.set("ID", vec![Object::String(b"0123456789abcdef".to_vec(), lopdf::StringFormat::Hexadecimal); 2]);
    let mut revision_one = Vec::new();
    document.save_to(&mut revision_one).unwrap();

    // Revision 2: a new definition of page 1's content object.
    let revised: String = SECRET.bytes().map(|b| format!("{b:02X}")).collect();
    let padding = "% revised layout\n".repeat(40);
    let mut stream = Stream::new(
        Dictionary::new(),
        format!("{padding}BT /F1 18 Tf 72 700 Td (Account ) Tj <{revised}> Tj ET BT /F1 12 Tf 72 500 Td (Public paragraph stays, revised) Tj ET").into_bytes(),
    );
    stream.compress().unwrap();
    assert!(stream.dict.has(b"Filter"), "revision 2 is compressed");
    let mut body = format!("<</Length {} /Filter /FlateDecode>>\nstream\n", stream.content.len()).into_bytes();
    body.extend_from_slice(&stream.content);
    body.extend_from_slice(b"\nendstream");
    let text = String::from_utf8_lossy(&revision_one);
    let previous: usize = text.rsplit("startxref").next().unwrap().split_whitespace().next().unwrap().parse().unwrap();
    let mut out = revision_one.clone();
    let offset = out.len();
    out.extend_from_slice(format!("{} 0 obj\n", page_one_content.0).as_bytes());
    out.extend_from_slice(&body);
    out.extend_from_slice(b"\nendobj\n");
    let xref = out.len();
    out.extend_from_slice(
        format!(
            "xref\n{} 1\n{offset:010} 00000 n\r\ntrailer\n<</Size {} /Root {} 0 R /Info {} 0 R /Prev {previous}>>\nstartxref\n{xref}\n%%EOF\n",
            page_one_content.0,
            document.max_id + 1,
            catalog.0,
            info.0
        )
        .as_bytes(),
    );
    (revision_one, out)
}

/// Every place the secret could hide in `bytes`: raw, hex (both cases),
/// UTF-16BE, and the same inside every decoded stream and string.
fn leaks(bytes: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    for (index, needle) in encodings().iter().enumerate() {
        if find(bytes, needle) {
            found.push(format!("raw encoding {index}"));
        }
    }
    let document = Document::load_mem(bytes).unwrap();
    for (id, object) in &document.objects {
        if let Object::Stream(stream) = object {
            let data = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
            for (index, needle) in encodings().iter().enumerate() {
                if find(&data, needle) {
                    found.push(format!("stream {id:?} encoding {index}"));
                }
            }
        }
        let mut strings = Vec::new();
        fn collect(object: &Object, out: &mut Vec<Vec<u8>>) {
            match object {
                Object::String(bytes, _) => out.push(bytes.clone()),
                Object::Array(items) => items.iter().for_each(|i| collect(i, out)),
                Object::Dictionary(d) => d.iter().for_each(|(_, v)| collect(v, out)),
                Object::Stream(s) => s.dict.iter().for_each(|(_, v)| collect(v, out)),
                _ => {}
            }
        }
        collect(object, &mut strings);
        for string in strings {
            for (index, needle) in encodings().iter().enumerate() {
                if find(&string, needle) {
                    found.push(format!("string in {id:?} encoding {index}"));
                }
            }
        }
    }
    found
}

fn page_content(bytes: &[u8], page: u32) -> Vec<Vec<u8>> {
    let document = Document::load_mem(bytes).unwrap();
    let id = document.get_pages()[&page];
    document
        .get_page_contents(id)
        .into_iter()
        .map(|c| document.get_object(c).unwrap().as_stream().unwrap().content.clone())
        .collect()
}

#[test]
fn a_windowed_redaction_is_a_full_rewrite_without_earlier_revisions() {
    let (revision_one, source) = secret_document();
    assert!(!leaks(&source).is_empty(), "the source holds the secret");
    // Revision 1 (a prefix of the source) holds the superseded content.
    assert!(source.starts_with(&revision_one));
    let saved = list(json!({
        "version": 1,
        "windows": [{ "page": 1, "operations": [
            { "op": "redact", "page": 1, "rect": { "x": 60.0, "y": 690.0, "width": 460.0, "height": 40.0 } }
        ] }]
    }));
    let (bytes, result) = apply_windows(&source, &saved, fonts_dir(), false).unwrap();
    assert_eq!(result.save_mode, "FULL_REWRITE");
    assert!(!bytes.starts_with(&source[..source.len() / 2]), "not an update of the source");
    assert_eq!(bytes.matches_count(b"%%EOF"), 1, "one revision");
    assert!(!find(&bytes, b"/Prev"), "no earlier section is linked");
    let leaked = leaks(&bytes);
    assert!(leaked.is_empty(), "the secret survived: {leaked:?}");

    // Valid, parses with both readers and renders.
    let engine = ContentEngine::open_bytes(bytes.clone()).unwrap();
    assert_eq!(engine.page_count().unwrap(), 2);
    for page in 1..=2 {
        let rendered = engine.render_page(page, 36).unwrap();
        assert!(rendered.into_raw_image().pixels.len() > 100);
    }
    let one = PageTextLayout::from_bytes(&bytes, 1).unwrap().text;
    assert!(one.contains("Public paragraph stays, revised"), "{one}");
    assert!(!one.contains("SECRET"), "{one}");
    // The untouched page's content stream is reused byte for byte.
    assert_eq!(page_content(&bytes, 2), page_content(&source, 2));
    let document = Document::load_mem(&bytes).unwrap();
    let page_one = document.get_dictionary(document.get_pages()[&1]).unwrap();
    assert!(!page_one.has(b"Thumb"), "the old thumbnail is dropped");
    // The field under the area is gone from the field tree; the outline and
    // the document information keep their non-secret text.
    let form = document.get_dictionary(document.catalog().unwrap().get(b"AcroForm").and_then(Object::as_reference).unwrap()).unwrap();
    assert!(form.get(b"Fields").and_then(Object::as_array).unwrap().is_empty());
    let info = document.trailer.get(b"Info").and_then(Object::as_reference).unwrap();
    let subject = document.get_dictionary(info).unwrap().get(b"Subject").and_then(Object::as_str).unwrap();
    assert_eq!(subject, b"Report on ");
    let text = PageTextLayout::from_bytes(&bytes, 2).unwrap().text;
    assert!(text.contains("Second page text"));

    // A redaction together with structure edits is still one full rewrite.
    let saved = list(json!({
        "version": 1,
        "structure": [{ "op": "movePage", "page": 2, "to": 1 }],
        "windows": [{ "page": 1, "operations": [
            { "op": "redact", "page": 1, "rect": { "x": 60.0, "y": 690.0, "width": 460.0, "height": 40.0 } }
        ] }]
    }));
    let (bytes, result) = apply_windows(&source, &saved, fonts_dir(), false).unwrap();
    assert_eq!(result.save_mode, "FULL_REWRITE");
    assert!(leaks(&bytes).is_empty());
    let pages = texts(&bytes);
    assert!(pages[0].contains("Second page text") && pages[1].contains("revised"), "{pages:?}");
}

trait Count {
    fn matches_count(&self, needle: &[u8]) -> usize;
}

impl Count for Vec<u8> {
    fn matches_count(&self, needle: &[u8]) -> usize {
        self.windows(needle.len()).filter(|w| *w == needle).count()
    }
}

/// `WELLFRIENDPDF_REAL_PDF_DIR`: a windowed redaction of real files is a
/// valid full rewrite whose other pages keep their text.
#[test]
fn real_files_survive_a_windowed_redaction_rewrite() {
    let Ok(dir) = std::env::var("WELLFRIENDPDF_REAL_PDF_DIR") else {
        eprintln!("WELLFRIENDPDF_REAL_PDF_DIR unset; skipping");
        return;
    };
    let mut checked = 0;
    let mut entries: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries.into_iter().filter(|p| p.extension().is_some_and(|e| e == "pdf")) {
        let source = std::fs::read(&path).unwrap();
        let Ok(engine) = ContentEngine::open_bytes(source.clone()) else { continue };
        if engine.document().reader().is_encrypted() {
            continue;
        }
        let count = engine.page_count().unwrap();
        let page = 1u32;
        let saved = list(json!({
            "version": 1,
            "windows": [{ "page": page, "operations": [
                { "op": "redact", "page": 1, "rect": { "x": 50.0, "y": 50.0, "width": 120.0, "height": 60.0 } }
            ] }]
        }));
        let started = std::time::Instant::now();
        let (bytes, result) = match apply_windows(&source, &saved, fonts_dir(), true) {
            Ok(saved) => saved,
            Err(error) => {
                eprintln!("{}: refused: {error}", path.display());
                continue;
            }
        };
        assert_eq!(result.save_mode, "FULL_REWRITE");
        let reopened = ContentEngine::open_bytes(bytes.clone()).unwrap();
        assert_eq!(reopened.page_count().unwrap(), count, "{}", path.display());
        for other in 2..=count.min(4) {
            let before = PageTextLayout::from_bytes(&source, other).map(|l| l.text).unwrap_or_default();
            let after = PageTextLayout::from_bytes(&bytes, other).map(|l| l.text).unwrap_or_default();
            assert_eq!(before, after, "{} page {other}", path.display());
            reopened.render_page(other, 24).unwrap();
        }
        eprintln!(
            "{}: {} -> {} bytes, full rewrite in {:?}",
            path.display(),
            source.len(),
            bytes.len(),
            started.elapsed()
        );
        checked += 1;
    }
    assert!(checked > 0);
}

/// Share of dark pixels in the upper half of a page rendered at 24 dpi.
fn upper_darkness(engine: &ContentEngine, page: u32) -> f64 {
    let raw = engine.render_page(page as usize, 24).unwrap().into_raw_image();
    let (width, height, channels) = (raw.width as usize, raw.height as usize, raw.channels as usize);
    let mut dark = 0usize;
    let mut total = 0usize;
    for y in 2..height / 2 - 2 {
        for x in 2..width - 2 {
            let at = (y * width + x) * channels;
            let luminance = raw.pixels[at..at + channels.min(3)].iter().map(|v| *v as usize).sum::<usize>() / channels.min(3);
            total += 1;
            if luminance < 40 {
                dark += 1;
            }
        }
    }
    dark as f64 / total.max(1) as f64
}

/// `WELLFRIENDPDF_LARGE_PDF=<path>`: a redaction of one page of a large file
/// is a streamed full rewrite; structure edits stay incremental.
#[test]
fn large_file_redaction_rewrite_is_streamed() {
    let Ok(path) = std::env::var("WELLFRIENDPDF_LARGE_PDF") else {
        eprintln!("WELLFRIENDPDF_LARGE_PDF unset; skipping");
        return;
    };
    let path = std::path::PathBuf::from(path);
    let engine = ContentEngine::open_path(&path).unwrap();
    let count = engine.page_count().unwrap();
    let middle = (count / 2).max(1) as u32;
    let window = windows::extract(&engine, middle).unwrap();
    let layout = PageTextLayout::from_bytes(&window, 1).unwrap();
    let darkness_before = upper_darkness(&engine, middle);
    drop(engine);
    let size = std::fs::metadata(&path).unwrap().len();
    // `WELLFRIENDPDF_LARGE_SCRATCH` puts the outputs on a real disk.
    let scratch = std::env::var("WELLFRIENDPDF_LARGE_SCRATCH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(format!("wf-m3-large-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();

    let structure = list(json!({
        "version": 1,
        "structure": [{ "op": "deletePage", "page": 3 }, { "op": "movePage", "page": 5, "to": 2 },
                      { "op": "insertBlankPage", "after": 1, "width": 612.0, "height": 792.0 }]
    }));
    let output = scratch.join("structure.pdf");
    let started = std::time::Instant::now();
    let result = apply_windows_path(&path, &output, &structure, fonts_dir(), true).unwrap();
    eprintln!(
        "structure save of {} ({size} bytes): {:?}, appended {} bytes",
        path.display(),
        started.elapsed(),
        result.output_bytes - size
    );
    assert_eq!(result.save_mode, "INCREMENTAL");
    assert_eq!(result.page_count, count);
    std::fs::remove_file(&output).unwrap();

    // Redact the top half of the middle page.
    let top = layout.page_box[3];
    let rect = json!({ "x": layout.page_box[0], "y": (top + layout.page_box[1]) / 2.0, "width": layout.page_box[2] - layout.page_box[0], "height": (top - layout.page_box[1]) / 2.0 });
    let redaction = list(json!({
        "version": 1,
        "windows": [{ "page": middle, "operations": [{ "op": "redact", "page": 1, "rect": rect }] }]
    }));
    let output = scratch.join("redacted.pdf");
    let started = std::time::Instant::now();
    let result = apply_windows_path(&path, &output, &redaction, fonts_dir(), true).unwrap();
    let elapsed = started.elapsed();
    eprintln!(
        "redaction full rewrite of {} ({size} bytes, page {middle}): {:?}, output {} bytes",
        path.display(),
        elapsed,
        result.output_bytes
    );
    assert_eq!(result.save_mode, "FULL_REWRITE");
    let reopened = ContentEngine::open_path(&output).unwrap();
    assert_eq!(reopened.page_count().unwrap(), count);
    // The book is a scan: the redaction rewrites the page image, and the
    // upper half renders as the black fill; text, if any, is gone.
    let line = (top + layout.page_box[1]) / 2.0 + 2.0;
    let after_window = windows::extract(&reopened, middle).unwrap();
    let after = PageTextLayout::from_bytes(&after_window, 1).unwrap();
    let remaining = after.glyphs.iter().filter(|g| g.bbox[1] > line && !g.text.trim().is_empty()).count();
    assert_eq!(remaining, 0);
    let darkness_after = upper_darkness(&reopened, middle);
    eprintln!("upper half darkness before {darkness_before:.2}, after {darkness_after:.2}");
    assert!(darkness_after > 0.97, "the upper half is filled");
    drop(reopened);
    std::fs::remove_file(&output).unwrap();
    let _ = std::fs::remove_dir(&scratch);
    assert!(elapsed.as_secs_f64() < 60.0);
}

/// Share of pixels darker than 40 and of near-white pixels in rows
/// `[from, to)` (fractions of the height) of a page rendered at 24 dpi.
fn band(engine: &ContentEngine, page: u32, from: f64, to: f64) -> (f64, f64) {
    let raw = engine.render_page(page as usize, 24).unwrap().into_raw_image();
    let (width, height, channels) = (raw.width as usize, raw.height as usize, raw.channels as usize);
    let (mut dark, mut white, mut total) = (0usize, 0usize, 0usize);
    for y in ((height as f64 * from) as usize + 2)..((height as f64 * to) as usize).saturating_sub(2) {
        for x in 2..width - 2 {
            let at = (y * width + x) * channels;
            let luminance = raw.pixels[at..at + channels.min(3)].iter().map(|v| *v as usize).sum::<usize>() / channels.min(3);
            total += 1;
            dark += usize::from(luminance < 40);
            white += usize::from(luminance > 245);
        }
    }
    (dark as f64 / total.max(1) as f64, white as f64 / total.max(1) as f64)
}

/// QF-2599: redacting part of a scanned (JPEG) page removes only the area:
/// the scan is decoded, the area painted and a new JPEG written; the old
/// image object is gone from the rewrite. `WELLFRIENDPDF_REAL_PDF_DIR`.
#[test]
fn a_scanned_page_redaction_keeps_the_rest_of_the_scan() {
    let Ok(dir) = std::env::var("WELLFRIENDPDF_REAL_PDF_DIR") else {
        eprintln!("WELLFRIENDPDF_REAL_PDF_DIR unset; skipping");
        return;
    };
    let path = std::path::Path::new(&dir).join("Bolt_Action_Rifles_3rd_Edition-p5-7.pdf");
    let Ok(source) = std::fs::read(&path) else {
        eprintln!("{} missing; skipping", path.display());
        return;
    };
    let engine = ContentEngine::open_bytes(source.clone()).unwrap();
    let page = engine.document().get_pages().unwrap()[0].clone();
    let [x0, y0, x1, y1] = page.media_box;
    let original = Document::load_mem(&source).unwrap();
    let scans: Vec<Vec<u8>> = original
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .filter(|s| s.dict.get(b"Filter").and_then(Object::as_name).is_ok_and(|f| f == b"DCTDecode"))
        .map(|s| s.content[..s.content.len().min(4096)].to_vec())
        .collect();
    assert!(!scans.is_empty(), "the pages are JPEG scans");
    let saved = list(json!({
        "version": 1,
        "windows": [{ "page": 1, "operations": [
            { "op": "redact", "page": 1, "rect": { "x": x0, "y": (y0 + y1) / 2.0, "width": x1 - x0, "height": (y1 - y0) / 2.0 } }
        ] }]
    }));
    let (bytes, result) = apply_windows(&source, &saved, fonts_dir(), true).unwrap();
    assert_eq!(result.save_mode, "FULL_REWRITE");
    let reopened = ContentEngine::open_bytes(bytes.clone()).unwrap();
    let (upper_dark, _) = band(&reopened, 1, 0.0, 0.5);
    let (_, lower_white) = band(&reopened, 1, 0.5, 1.0);
    let (_, lower_white_before) = band(&engine, 1, 0.5, 1.0);
    eprintln!("upper dark {upper_dark:.2}; lower white {lower_white:.2} (before {lower_white_before:.2})");
    assert!(upper_dark > 0.97, "the upper half is redacted");
    // Ink below the area survives (the old fallback dropped the whole scan).
    let kept = (1.0 - lower_white) / (1.0 - lower_white_before).max(1e-6);
    assert!(kept > 0.8 && kept < 1.25, "the lower half of the scan stays ({kept:.2} of its ink)");
    // The page's old scan is not in the rewrite; the other pages' are.
    let first = &scans.iter().filter(|scan| !find(&bytes, scan)).count();
    assert_eq!(*first, 1, "exactly the redacted page's scan was replaced");
    let rewritten = Document::load_mem(&bytes).unwrap();
    let id = rewritten.get_pages()[&1];
    let resources = rewritten.get_page_resources(id).unwrap();
    assert!(resources.0.is_some() || !resources.1.is_empty());
}
