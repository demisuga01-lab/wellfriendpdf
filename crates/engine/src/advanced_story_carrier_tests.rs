//! Regression source only. No PDF generation, extraction or rendering was run.
use super::*;
use crate::fonts::WritingMode;

const OWNER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn line(text: &str, mode: WritingMode) -> StoryPaintLine {
    StoryPaintLine {
        writing_mode: mode,
        text: text.into(),
        x: 30.0,
        baseline: 100.0,
        width: 120.0,
        font_size: 12.0,
        font_index: 0,
        font_spans: vec![],
        style_spans: vec![],
        tab_segments: vec![],
        tab_decorations: vec![],
        rgb: [0.0; 3],
        rtl: false,
        bidi: None,
        shaping: Default::default(),
        tag_owner: None,
        artifact: false,
    }
}

fn input() -> Vec<u8> {
    use crate::authoring::{FontFace, PageSize, PdfBuilder, TextStyle};
    let mut builder = PdfBuilder::new();
    builder
        .add_page(PageSize::custom(200.0, 200.0))
        .draw_text(
            "OLD",
            10.0,
            50.0,
            &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
        )
        .unwrap();
    builder.to_bytes().unwrap()
}

fn replace(input: &[u8], lines: &[StoryPaintLine], owner: Option<&StoryFrameBinding>) -> Vec<u8> {
    replace_story_frame(
        input,
        1,
        [0, 3],
        [10.0, 10.0, 190.0, 190.0],
        lines,
        &[],
        &[],
        false,
        OWNER,
        owner,
    )
    .unwrap()
    .bytes
}

fn owner_program(engine: &ContentEngine) -> (StoryOwnedRange, PdfObject, Vec<u8>) {
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let owner = locate_story_frame(reader, &page.contents, OWNER)
        .unwrap()
        .unwrap();
    let object = reader.get_object(owner.stream.0, owner.stream.1).unwrap();
    let decoded =
        decode_stream_lossless_with_limits(&object, reader, &DecodeLimits::default()).unwrap();
    (owner, object, decoded.data)
}

#[test]
fn shaped_run_receipt_binds_font_program_and_per_occurrence_glyph_state() {
    let lines = vec![line("A", WritingMode::HorizontalTb)];
    let glyph = GeneratedGlyph {
        cid: 1,
        gid: 7,
        logical_byte_start: 0,
        visual_unicode: "A".into(),
        to_unicode: Some("A".into()),
        advance: 600.0,
        offset_x: 1.5,
        offset_y: -0.25,
        orientation: VerticalGlyphOrientation::Upright,
        cross_advance: 0.0,
        font_width: 610.0,
        bounds: Some([0.0, -10.0, 590.0, 700.0]),
    };
    let fonts = vec![crate::editing_transactions::ApprovedFontAsset {
        lookup_name: "Fixture".into(),
        bytes: vec![1, 2, 3],
    }];
    let painted = vec![vec![StoryPaintedRun {
        font_index: 0,
        style_index: 0,
        inline_origin: None,
        tab_field: None,
        glyphs: vec![glyph.clone()],
    }]];
    let baseline = story_shape_model_sha256(&lines, &painted, &fonts).unwrap();
    assert_eq!(baseline.len(), 64);
    assert_eq!(
        baseline,
        story_shape_model_sha256(&lines, &painted, &fonts).unwrap()
    );

    let mut changed_glyph = glyph;
    changed_glyph.advance += 0.5;
    assert_ne!(
        baseline,
        story_shape_model_sha256(
            &lines,
            &vec![vec![StoryPaintedRun {
                font_index: 0,
                style_index: 0,
                inline_origin: None,
                tab_field: None,
                glyphs: vec![changed_glyph],
            }]],
            &fonts,
        )
        .unwrap()
    );
    let mut changed_font = fonts;
    changed_font[0].bytes.push(4);
    assert_ne!(
        baseline,
        story_shape_model_sha256(&lines, &painted, &changed_font).unwrap()
    );
}

#[test]
fn carrier_encoding_is_exact_bounded_and_not_a_general_invisible_text_route() {
    assert_eq!(
        encode("\r\n\u{000b}\u{000c}\u{0085}\u{2028}\u{2029}").unwrap(),
        "0001000200030004000500060007"
    );
    assert_eq!(encode("").unwrap(), "");
    for text in ["word", " \n", "\u{00a0}"] {
        assert!(encode(text).is_err());
        assert!(!needs_carrier(&line(text, WritingMode::HorizontalTb)));
    }
    assert!(encode("\t").is_ok());
    assert!(needs_carrier(&line("\t", WritingMode::HorizontalTb)));
    assert_eq!(encode("\u{200d}").unwrap(), "0013");
    assert!(encode(&"\n".repeat(4_000_001)).is_err());
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel.scope(|| encode("\n")).is_err());
}

#[test]
fn story_tab_decorations_serialize_as_artifacts_and_not_logical_glyphs() {
    let mut decorated = line("A\tB", WritingMode::HorizontalTb);
    decorated.tab_segments = vec![
        StoryTabSegment {
            range: [0, 1],
            origin: 0.0,
            width: 10.0,
            leading_pad: 0.0,
        },
        StoryTabSegment {
            range: [2, 3],
            origin: 60.0,
            width: 10.0,
            leading_pad: 0.0,
        },
    ];
    decorated.tab_decorations = vec![
        crate::fonts::tab_stops::PositionedTabDecoration::Leader {
            from: 10.0,
            to: 60.0,
            leader: crate::fonts::tab_stops::TabLeader::Solid,
        },
        crate::fonts::tab_stops::PositionedTabDecoration::Bar { position: 60.0 },
    ];
    super::super::validate_story_tab_segments(&decorated, &decorated.text).unwrap();
    let program = super::super::serialize_story_tab_decorations(&decorated).unwrap();
    assert!(program.starts_with("/Artifact BMC\nq\n"));
    assert!(program.ends_with("Q\nEMC\n"));
    assert_eq!(program.matches(" l S\n").count(), 2);
    assert!(!program.contains("ActualText"));
    assert!(!program.contains(" Tj"));

    let paint = super::super::story_paint_model_sha256(&[decorated.clone()], &[]).unwrap();
    decorated.tab_decorations[0] = crate::fonts::tab_stops::PositionedTabDecoration::Leader {
        from: 10.0,
        to: 60.0,
        leader: crate::fonts::tab_stops::TabLeader::Dashes,
    };
    assert_ne!(
        paint,
        super::super::story_paint_model_sha256(&[decorated.clone()], &[]).unwrap()
    );

    decorated.text = "AB".into();
    assert!(super::super::validate_story_tab_segments(&decorated, &decorated.text).is_err());
}

#[test]
fn empty_content_needs_no_carrier_and_object_overflow_is_an_error() {
    let mut resources = crate::PdfDictionary::empty();
    let mut updates = vec![];
    let none = CarrierFonts::prepare(&[], OWNER, 10, &mut resources, &mut updates).unwrap();
    assert!(none.names.iter().all(Option::is_none));
    assert!(resources.is_empty());
    assert!(updates.is_empty());
    assert!(CarrierFonts::prepare(
        &[line("\n", WritingMode::HorizontalTb)],
        OWNER,
        u32::MAX,
        &mut resources,
        &mut updates
    )
    .is_err());
    assert!(resources.is_empty());
    assert!(updates.is_empty());
}

#[test]
fn every_blank_separator_has_real_source_codes_zero_advance_and_correct_writing_mode() {
    let input = input();
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        for separator in [
            "\t", "\r", "\n", "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
        ] {
            let output = replace(&input, &[line(separator, mode)], None);
            let engine = ContentEngine::open_bytes(output.clone()).unwrap();
            let chunks = engine.collect_page_text_chunks(1).unwrap();
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].text, separator);
            assert_eq!(chunks[0].width, 0.0);
            assert_eq!(chunks[0].is_vertical, mode.is_vertical());
            assert!(!chunks[0].is_invisible);
            assert!(chunks[0].is_actual_text);
            assert_eq!(engine.get_page_text(1).unwrap(), separator);
            let model = analyze_multi_run_text_range(&output, 1).unwrap();
            assert_eq!(model.logical_text, separator);
            assert_eq!(model.source_spans.len(), 1);
            assert_eq!(
                model.source_spans[0].logical_range,
                [0, separator.chars().count()]
            );
            assert_eq!(model.source_spans[0].text_render_mode, 0);
            assert!(output.starts_with(&input)); // ordinary edit, not sanitizing redaction
        }
    }
}

#[test]
fn carrier_to_unicode_survives_without_actual_text_and_uses_an_empty_outline() {
    let output = replace(&input(), &[line("\r\n", WritingMode::VerticalRl)], None);
    let engine = ContentEngine::open_bytes(output).unwrap();
    let (owner, _, raw) = owner_program(&engine);
    let mut operations = crate::content::ContentParser::parse(&raw[owner.range]).unwrap();
    for op in &mut operations {
        if op.operator == "BDC" {
            for operand in &mut op.operands {
                if let crate::content::Operand::Dictionary(entries) = operand {
                    entries.retain(|(key, _)| key != "ActualText");
                }
            }
        }
    }
    let resources = engine.get_page_resources(1).unwrap();
    let reader = engine.document().reader();
    for dict in resources.fonts.values().filter(|dict| is_font(dict)) {
        let program = crate::fonts::provider::embedded_program(reader, dict).unwrap();
        let face = ttf_parser::Face::parse(&program, 0).unwrap();
        let space = face.glyph_index(' ').unwrap();
        assert_ne!(space.0, 0);
        assert!(face.glyph_bounding_box(space).is_none());
        let resolver = FontResolver::new(dict, reader);
        assert!(resolver.is_vertical());
        for cid in 1..=7 {
            assert_eq!(resolver.glyph_width(cid), 0.0);
            assert_eq!(resolver.vertical_metrics(cid), (0.0, 0.0, 0.0));
        }
    }
    let mut collector = crate::text::TextCollector::new(resources, reader);
    let chunks = collector
        .collect_scoped(
            &operations,
            &Default::default(),
            &crate::CancelToken::none(),
        )
        .unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].chunk.text, "\r\n");
    assert!(!chunks[0].chunk.is_actual_text);
}

#[test]
fn verification_cannot_be_satisfied_by_a_duplicate_outside_the_owned_frame() {
    let lines = [line("\n", WritingMode::HorizontalTb)];
    let output = replace(&input(), &lines, None);
    let engine = ContentEngine::open_bytes(output).unwrap();
    let (owner, object, raw) = owner_program(&engine);
    let mut changed = String::from_utf8(raw).unwrap();
    let begin = changed[owner.range.clone()].find("q\n").unwrap() + owner.range.start;
    let outside = changed[begin..owner.range.end]
        .strip_suffix("EMC")
        .unwrap()
        .to_owned();
    assert!(changed[owner.range.clone()].contains("<0002> Tj"));
    let showing = owner.range.start + changed[owner.range.clone()].find("<0002> Tj").unwrap();
    changed.replace_range(showing..showing + "<0002> Tj".len(), "<> Tj");
    changed.push_str(&outside); // same decoded text and font, wrong source owner
    let mut dict = object.as_stream().unwrap().0.clone();
    dict.remove("Filter");
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(changed.len() as i64));
    let modified = write_incremental_update(
        engine.document().reader(),
        vec![IncrementalObject {
            number: owner.stream.0,
            generation: owner.stream.1,
            object: PdfObject::Stream {
                dict,
                raw: changed.into_bytes(),
            },
        }],
    )
    .unwrap();
    let modified = ContentEngine::open_bytes(modified).unwrap();
    assert!(modified
        .collect_page_text_chunks(1)
        .unwrap()
        .iter()
        .any(|c| c.text == "\n"));
    assert!(verify(&modified, 1, OWNER, &lines).is_err());
}

#[test]
fn repeated_rewrites_retire_carrier_fonts_and_empty_frames_remain_refillable() {
    let mut output = input();
    let mut binding = None;
    for text in ["\n", "\r\n", "\u{2028}", "\u{200d}", "\u{e0100}", "", "\n"] {
        let lines = if text.is_empty() {
            vec![]
        } else {
            vec![line(text, WritingMode::HorizontalTb)]
        };
        output = replace(&output, &lines, binding.as_ref());
        binding = bind_story_frame(&output, 1, OWNER).unwrap();
        let rebound = binding.as_ref().unwrap();
        assert_eq!(rebound.paint_sha256.as_deref().map(str::len), Some(64));
        assert_eq!(rebound.shape_sha256.as_deref().map(str::len), Some(64));
        let engine = ContentEngine::open_bytes(output.clone()).unwrap();
        let resources = engine.get_page_resources(1).unwrap();
        assert_eq!(
            resources
                .fonts
                .values()
                .filter(|dict| is_font(dict))
                .count(),
            usize::from(!text.is_empty())
        );
        assert_eq!(
            engine
                .collect_page_text_chunks(1)
                .unwrap()
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>(),
            text
        );
    }
}

#[test]
fn paragraph_markers_and_artifact_wrappers_still_contain_the_carrier() {
    for artifact in [false, true] {
        let mut marked = line("\n", WritingMode::HorizontalTb);
        marked.tag_owner = Some("b".repeat(64));
        marked.artifact = artifact;
        let output = replace(&input(), &[marked.clone()], None);
        let engine = ContentEngine::open_bytes(output).unwrap();
        let (owner, _, raw) = owner_program(&engine);
        let program = std::str::from_utf8(&raw[owner.range]).unwrap();
        assert!(program.contains("/WFStoryParagraph"));
        assert_eq!(program.contains("/Artifact BMC"), artifact);
        verify(&engine, 1, OWNER, &[marked]).unwrap();
    }
}

#[test]
fn actual_text_does_not_hide_a_damaged_carrier_to_unicode_mapping() {
    let lines = [line("\n", WritingMode::HorizontalTb)];
    let engine = ContentEngine::open_bytes(replace(&input(), &lines, None)).unwrap();
    let resources = engine.get_page_resources(1).unwrap();
    let font = resources.fonts.values().find(|dict| is_font(dict)).unwrap();
    let (number, generation) = font.get_reference("ToUnicode").unwrap();
    let object = engine
        .document()
        .reader()
        .get_object(number, generation)
        .unwrap();
    let decoded = decode_stream_lossless_with_limits(
        &object,
        engine.document().reader(),
        &DecodeLimits::default(),
    )
    .unwrap();
    let map = String::from_utf8(decoded.data).unwrap();
    assert!(map.contains("<0002> <000A>"));
    let map = map.replacen("<0002> <000A>", "<0002> <0058>", 1);
    let mut dict = object.as_stream().unwrap().0.clone();
    dict.remove("Filter");
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(map.len() as i64));
    let changed = write_incremental_update(
        engine.document().reader(),
        vec![IncrementalObject {
            number,
            generation,
            object: PdfObject::Stream {
                dict,
                raw: map.into_bytes(),
            },
        }],
    )
    .unwrap();
    let changed = ContentEngine::open_bytes(changed).unwrap();
    assert_eq!(changed.collect_page_text_chunks(1).unwrap()[0].text, "\n");
    assert!(verify(&changed, 1, OWNER, &lines).is_err());
}

#[test]
fn default_ignorable_story_lines_have_exact_source_codes_in_all_writing_modes() {
    for mode in [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
    ] {
        let lines = [
            line("\u{200d}\u{fe0f}\r\n", mode),
            line("\u{e0100}\u{2067}\u{2069}", mode),
        ];
        let output = replace(&input(), &lines, None);
        let engine = ContentEngine::open_bytes(output.clone()).unwrap();
        verify(&engine, 1, OWNER, &lines).unwrap();
        let expected = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<String>();
        assert_eq!(
            analyze_multi_run_text_range(&output, 1)
                .unwrap()
                .logical_text,
            expected
        );
        let (owner, _, raw) = owner_program(&engine);
        let mut operations = crate::content::ContentParser::parse(&raw[owner.range]).unwrap();
        for operation in &mut operations {
            for operand in &mut operation.operands {
                if let crate::content::Operand::Dictionary(entries) = operand {
                    entries.retain(|(key, _)| key != "ActualText");
                }
            }
        }
        let mut collector = crate::text::TextCollector::new(
            engine.get_page_resources(1).unwrap(),
            engine.document().reader(),
        );
        let chunks = collector.collect(&operations);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<String>(),
            expected
        );
        assert!(chunks
            .iter()
            .all(|chunk| chunk.width == 0.0 && chunk.is_vertical == mode.is_vertical()));
    }
}

#[test]
fn actual_text_cannot_conceal_wrong_valid_carrier_codes() {
    let lines = [line("\u{200d}", WritingMode::HorizontalTb)];
    let engine = ContentEngine::open_bytes(replace(&input(), &lines, None)).unwrap();
    let (owner, object, raw) = owner_program(&engine);
    let program = String::from_utf8(raw).unwrap();
    assert!(program.contains("<0013> Tj"));
    let changed = program.replacen("<0013> Tj", "<0002> Tj", 1);
    let mut dict = object.as_stream().unwrap().0.clone();
    dict.remove("Filter");
    dict.remove("DecodeParms");
    dict.insert("Length", PdfObject::Integer(changed.len() as i64));
    let bytes = write_incremental_update(
        engine.document().reader(),
        vec![IncrementalObject {
            number: owner.stream.0,
            generation: owner.stream.1,
            object: PdfObject::Stream {
                dict,
                raw: changed.into_bytes(),
            },
        }],
    )
    .unwrap();
    let reopened = ContentEngine::open_bytes(bytes).unwrap();
    assert_eq!(
        reopened.collect_page_text_chunks(1).unwrap()[0].text,
        "\u{200d}"
    );
    assert!(verify(&reopened, 1, OWNER, &lines).is_err());
}

#[test]
fn inline_logical_glyphs_use_empty_outlines_without_spacing() {
    let face = ttf_parser::Face::parse(get_fallback_font("Symbol").unwrap(), 0).unwrap();
    let glyphs = inline_glyphs("\u{200d}\u{e0100}", &face).unwrap();
    assert_eq!(glyphs.len(), 2);
    assert_eq!(glyphs[1].logical_byte_start, 3);
    assert!(glyphs
        .iter()
        .all(|glyph| glyph.advance == 0.0 && glyph.font_width == 0.0 && glyph.bounds.is_none()));
    assert_eq!(
        glyphs
            .iter()
            .filter_map(|glyph| glyph.to_unicode.as_deref())
            .collect::<String>(),
        "\u{200d}\u{e0100}"
    );
    assert!(inline_glyphs("A", &face).is_err());
    let repeated = inline_glyphs(&"\u{200d}".repeat(65_536), &face).unwrap();
    assert_eq!(repeated.len(), 65_536);
    assert!(repeated.iter().all(|glyph| glyph.cid == 0x0013));
}
