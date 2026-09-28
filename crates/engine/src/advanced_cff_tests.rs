//! Source regressions only; no font/PDF workload was executed.
use super::*;
use crate::authoring::{PageSize, PdfBuilder};
use crate::fonts::pdf_embedding_fixtures::font;
use crate::fonts::resolver::get_descendant_font;

fn glyph(code: u16, gid: u16, text: &str) -> GeneratedGlyph {
    GeneratedGlyph {
        cid: code,
        gid,
        logical_byte_start: 0,
        visual_unicode: text.into(),
        to_unicode: Some(text.into()),
        advance: 900.0,
        offset_x: 0.0,
        offset_y: 0.0,
        orientation: VerticalGlyphOrientation::Upright,
        cross_advance: 0.0,
        font_width: 600.0,
        bounds: None,
    }
}

#[test]
fn generated_cff_native_cids_widths_and_unicode_survive_incremental_save() {
    for keyed in [false, true] {
        for vertical in [false, true] {
            let mut builder = PdfBuilder::new()
                .with_writer_mode(crate::writer::WriterMode::ClassicXref)
                .with_version("1.4");
            builder.add_page(PageSize::LETTER);
            let source = builder.to_bytes().unwrap();
            let engine = ContentEngine::open_bytes(source.clone()).unwrap();
            let original_reader = engine.document().reader();
            let glyphs = [
                glyph(10, 1, "A"),
                glyph(11, 1, "\u{ff21}"),
                glyph(12, 2, "B"),
            ];
            let updates = build_type0_font_objects(
                &font(keyed, 0),
                &glyphs,
                vertical,
                100,
                101,
                102,
                103,
                104,
                105,
            )
            .unwrap();
            let output = write_incremental_update(original_reader, updates).unwrap();
            assert!(output.starts_with(&source));
            let reopened = ContentEngine::open_bytes(output).unwrap();
            let reader = reopened.document().reader();
            let (root, generation) = reader.root_reference().unwrap();
            assert_eq!(
                reader
                    .get_object(root, generation)
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get_name("Version"),
                Some("1.6")
            );
            let dictionary = reader.get_object(105, 0).unwrap();
            let dictionary = dictionary.as_dict().unwrap();
            let desc = get_descendant_font(dictionary, reader).unwrap();
            assert_eq!(desc.get_name("Subtype"), Some("CIDFontType0"));
            assert!(!desc.contains_key("CIDToGIDMap"));
            let resolver = FontResolver::new(dictionary, reader);
            resolver.validate_encoding().unwrap();
            assert_eq!(
                resolver.cid_for_code(10).unwrap(),
                if keyed { 42 } else { 1 }
            );
            assert_eq!(
                resolver.cid_for_code(11).unwrap(),
                if keyed { 42 } else { 1 }
            );
            assert_eq!(resolver.glyph_width(10), 600.0);
            assert_eq!(resolver.decode_char(10), "A");
            assert_eq!(resolver.decode_char(11), "\u{ff21}");
            assert_eq!(resolver.is_vertical(), vertical);
            if vertical {
                assert_eq!(resolver.vertical_metrics(10), (-900.0, 0.0, 0.0));
            }
            let decoded = crate::render::text_decode::try_decode_text_bytes_with_resolver(
                &[0, 10, 0, 11, 0, 12],
                dictionary,
                &resolver,
                reader,
            )
            .unwrap();
            assert_eq!(
                decoded.iter().map(|g| g.code).collect::<Vec<_>>(),
                [1, 1, 2]
            );
            assert_eq!(
                decoded.iter().map(|g| g.unicode).collect::<String>(),
                "A\u{ff21}B"
            );
            let program = reader.get_object(100, 0).unwrap();
            assert_eq!(
                crate::filters::decode_stream_lossless(&program, reader)
                    .unwrap()
                    .data,
                font(keyed, 0)
            );
        }
    }
}

#[test]
fn cff_character_codes_do_not_merge_distinct_logical_occurrences() {
    let bytes = font(true, 0);
    let glyphs = [glyph(1, 1, "A"), glyph(2, 1, "\u{ff21}")];
    let objects = build_type0_font_objects(&bytes, &glyphs, false, 1, 2, 3, 4, 5, 6).unwrap();
    let PdfObject::Stream { dict, raw } = &objects.iter().find(|o| o.number == 4).unwrap().object
    else {
        panic!()
    };
    let cmap = crate::filters::decode_stream_from_dict(dict, raw).unwrap();
    let cmap = String::from_utf8(cmap).unwrap();
    assert!(cmap.contains("<0001> <0041>"));
    assert!(cmap.contains("<0002> <FF21>"));
}

#[test]
fn malformed_cff_and_conflicting_code_assignments_fail_before_serialization() {
    assert!(build_type0_font_objects(b"OTTO", &[], false, 1, 2, 3, 4, 5, 6).is_err());
    assert!(build_type0_font_objects(
        &font(true, 0),
        &[glyph(1, 1, "A"), glyph(1, 2, "B")],
        false,
        1,
        2,
        3,
        4,
        5,
        6
    )
    .is_err());
    assert!(
        build_type0_font_objects(&font(true, 2), &[glyph(1, 1, "A")], false, 1, 2, 3, 4, 5, 6)
            .is_err()
    );
}
