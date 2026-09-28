//! Authoring/renderer round-trip regression source; not executed.
use super::*;
use crate::fonts::pdf_embedding_fixtures::font as cff_font;
use crate::fonts::resolver::get_descendant_font;

#[test]
fn authoring_name_and_cid_keyed_cff_use_native_charset_and_reopen() {
    for keyed in [false, true] {
        for mode in [
            WriterMode::ClassicXref,
            WriterMode::XrefStream,
            WriterMode::XrefStreamWithObjStm,
        ] {
            let bytes = cff_font(keyed, 0);
            let mut doc = PdfBuilder::new().with_writer_mode(mode).with_version("1.4");
            let font = doc
                .register_font_bytes("DisplayNameNotPostScriptIdentity", bytes.clone())
                .unwrap();
            doc.add_page(PageSize::LETTER)
                .draw_text("AB A", 30.0, 700.0, &TextStyle::new(font, 12.0))
                .unwrap();
            let plan = FontBuildPlan::from_builder(&doc).unwrap();
            let output = doc.to_bytes().unwrap();
            assert!(output.starts_with(b"%PDF-1.6"));
            let engine = crate::ContentEngine::open_bytes(output).unwrap();
            let resources = engine.get_page_resources(1).unwrap();
            let reader = engine.document().reader();
            let dictionary = resources
                .fonts
                .values()
                .find(|font| font.get_name("Subtype") == Some("Type0"))
                .unwrap();
            let descendant = get_descendant_font(dictionary, reader).unwrap();
            assert_eq!(dictionary.get_name("BaseFont"), Some("WFCffFixture"));
            assert_eq!(descendant.get_name("Subtype"), Some("CIDFontType0"));
            assert!(!descendant.contains_key("CIDToGIDMap"));
            let descriptor = reader
                .resolve(descendant.get("FontDescriptor").unwrap().clone())
                .unwrap();
            let descriptor = descriptor.as_dict().unwrap();
            assert!(!descriptor.contains_key("FontFile2"));
            let program = reader
                .resolve(descriptor.get("FontFile3").unwrap().clone())
                .unwrap();
            let PdfObject::Stream { dict, .. } = &program else {
                panic!()
            };
            assert_eq!(dict.get_name("Subtype"), Some("OpenType"));
            assert_eq!(
                crate::filters::decode_stream_lossless(&program, reader)
                    .unwrap()
                    .data,
                bytes
            );
            let resolver = crate::fonts::FontResolver::new(dictionary, reader);
            resolver.validate_encoding().unwrap();
            for entry in &plan.embedded_plan(font).unwrap().entries {
                let native = if keyed {
                    [0, 42, 7, 1000][entry.glyph_id as usize]
                } else {
                    entry.glyph_id
                };
                assert_eq!(resolver.cid_for_code(entry.cid).unwrap(), native);
                assert_eq!(resolver.decode_char(entry.cid), entry.unicode);
                let glyphs = crate::render::text_decode::try_decode_text_bytes_with_resolver(
                    &entry.cid.to_be_bytes(),
                    dictionary,
                    &resolver,
                    reader,
                )
                .unwrap();
                assert_eq!(glyphs.len(), 1);
                assert!(glyphs[0].is_gid);
                assert_eq!(glyphs[0].code, entry.glyph_id);
                assert_eq!(glyphs[0].width, Some(entry.width));
                assert!(!glyphs[0].is_space, "two-byte codes do not receive Tw");
            }
            let text = engine
                .collect_page_text_chunks(1)
                .unwrap()
                .into_iter()
                .map(|c| c.text)
                .collect::<String>();
            assert_eq!(text, "AB A");
        }
    }
}

#[test]
fn cff_and_truetype_fallback_stack_share_the_existing_shaping_pipeline() {
    let mut doc = PdfBuilder::new();
    let cff = doc
        .register_font_bytes("WFCffFixture", cff_font(true, 0))
        .unwrap();
    let ttf = doc
        .register_font_bytes("Fallback", get_fallback_font("Symbol").unwrap())
        .unwrap();
    let stack = doc.register_font_stack(&[cff, ttf]).unwrap();
    let style = TextStyle::new(stack, 14.0);
    let page = doc.add_page(PageSize::LETTER);
    let preview = page.preview_font_stack("AB C AB", 400.0, &style).unwrap();
    assert!(preview[0].runs.iter().any(|run| run.font == cff));
    assert!(preview[0]
        .runs
        .iter()
        .any(|run| run.font == ttf && run.fallback));
    page.draw_text("AB C AB", 30.0, 700.0, &style).unwrap();
    let engine = crate::ContentEngine::open_bytes(doc.to_bytes().unwrap()).unwrap();
    assert_eq!(
        engine
            .collect_page_text_chunks(1)
            .unwrap()
            .into_iter()
            .map(|c| c.text)
            .collect::<String>(),
        "AB C AB"
    );
}

#[test]
fn denied_font_registration_leaves_registry_and_pages_unchanged() {
    let mut doc = PdfBuilder::new();
    doc.add_page(PageSize::LETTER);
    for rights in [2, 4, 0x208] {
        assert!(doc
            .register_font_bytes("Denied", cff_font(true, rights))
            .is_err());
    }
    assert!(doc.custom_fonts.is_empty());
    assert!(doc.pages[0].custom_fonts.is_empty());
}

#[test]
fn authoring_obeys_truetype_no_subsetting_permission() {
    let bytes = crate::fonts::pdf_embedding_fixtures::with_rights(
        get_fallback_font("Symbol").unwrap().to_vec(),
        0x108,
    );
    let mut doc = PdfBuilder::new();
    let font = doc.register_font_bytes("NoSubset", bytes.clone()).unwrap();
    doc.add_page(PageSize::LETTER)
        .draw_text("A", 30.0, 700.0, &TextStyle::new(font, 12.0))
        .unwrap();
    let plan = FontBuildPlan::from_builder(&doc).unwrap();
    let built = build_embedded_type0_font(&mut 1, font, "NoSubset", &bytes, &plan).unwrap();
    let (_, raw) = built
        .objects
        .iter()
        .find_map(|object| match &object.object {
            PdfObject::Stream { dict, raw } if dict.get_integer("Length1").is_some() => {
                Some((dict, raw))
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(*raw, bytes);
}
