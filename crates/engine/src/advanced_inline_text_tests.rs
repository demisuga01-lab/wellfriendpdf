//! Regression source only. No tests or PDF workloads have been executed.
use super::*;

fn fixture() -> (ContentEngine, PageResources) {
    use crate::authoring::{PageSize, PdfBuilder};
    let mut builder = PdfBuilder::new();
    builder.add_page(PageSize::custom(200.0, 200.0));
    let engine = ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
    let mut resources = PageResources::default();
    let mut horizontal = crate::PdfDictionary::empty();
    horizontal.insert("Type", PdfObject::Name("Font".into()));
    horizontal.insert("Subtype", PdfObject::Name("Type1".into()));
    horizontal.insert("BaseFont", PdfObject::Name("Courier".into()));
    let mut descendant = crate::PdfDictionary::empty();
    descendant.insert("Subtype", PdfObject::Name("CIDFontType2".into()));
    descendant.insert("DW", PdfObject::Integer(1000));
    descendant.insert(
        "DW2",
        PdfObject::Array(vec![PdfObject::Integer(880), PdfObject::Integer(-1000)]),
    );
    let mut vertical = crate::PdfDictionary::empty();
    vertical.insert("Type", PdfObject::Name("Font".into()));
    vertical.insert("Subtype", PdfObject::Name("Type0".into()));
    vertical.insert("Encoding", PdfObject::Name("Identity-V".into()));
    vertical.insert(
        "DescendantFonts",
        PdfObject::Array(vec![PdfObject::Dictionary(descendant)]),
    );
    resources.fonts.insert("H".into(), horizontal.clone());
    resources.fonts.insert("V".into(), vertical.clone());
    // Generation widths intentionally differ from the synthetic shaping
    // advances below: explicit matrices, not a prior CID width, own placement.
    resources.fonts.insert("GH".into(), horizontal);
    resources.fonts.insert("GV".into(), vertical);
    (engine, resources)
}
fn scan(data: &[u8], metrics: &mut Metrics<'_>) -> Vec<ContentStringToken> {
    scan_text_string_tokens_with_metrics(
        data,
        &mut ScannedTextTokenState::default(),
        Some((7, 0)),
        Some(metrics),
    )
    .unwrap()
}
fn close(a: &[f64], b: &[f64]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert!((a - b).abs() < 1e-10, "{a} != {b}");
    }
}
fn same_position(a: &ContentStringToken, b: &ContentStringToken) {
    let a = a.source_position.as_ref().unwrap();
    let b = b.source_position.as_ref().unwrap();
    close(&a.line, &b.line);
    close(&a.offset, &b.offset);
    close(&a.matrix(), &b.matrix());
}
fn glyphs() -> Vec<GeneratedGlyph> {
    vec![
        GeneratedGlyph {
            cid: 200,
            gid: 1,
            logical_byte_start: 0,
            visual_unicode: "A".into(),
            to_unicode: Some("A".into()),
            advance: 730.0,
            offset_x: 120.0,
            offset_y: -380.0,
            orientation: VerticalGlyphOrientation::RotateClockwise,
            cross_advance: 13.0,
            font_width: 600.0,
            bounds: None,
        },
        GeneratedGlyph {
            cid: 201,
            gid: 2,
            logical_byte_start: 1,
            visual_unicode: "B".into(),
            to_unicode: Some("B".into()),
            advance: 1000.0,
            offset_x: -490.0,
            offset_y: -820.0,
            orientation: VerticalGlyphOrientation::Upright,
            cross_advance: -7.0,
            font_width: 610.0,
            bounds: None,
        },
    ]
}

#[test]
fn cursor_tracks_both_axes_and_numeric_tj_across_contents_members() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let mut state = ScannedTextTokenState::default();
    scan_text_string_tokens_with_metrics(
        b"BT /H 10 Tf 14 TL 1 .2 .3 1 10 20 Tm (A) Tj",
        &mut state,
        Some((7, 0)),
        Some(&mut metrics),
    )
    .unwrap();
    let tokens = scan_text_string_tokens_with_metrics(
        b"[250] TJ /V 10 Tf <0041> Tj <0042> Tj T* <0043> Tj ET",
        &mut state,
        Some((8, 0)),
        Some(&mut metrics),
    )
    .unwrap();
    close(
        &tokens[0].source_position.as_ref().unwrap().offset,
        &[3.5, 0.0],
    );
    close(
        &tokens[1].source_position.as_ref().unwrap().offset,
        &[3.5, -10.0],
    );
    close(
        &tokens[2].source_position.as_ref().unwrap().line,
        &[1.0, 0.2, 0.3, 1.0, 5.8, 6.0],
    );
    close(
        &tokens[2].source_position.as_ref().unwrap().offset,
        &[0.0, 0.0],
    );
}

#[test]
fn quote_double_quote_and_td_use_line_origin_not_current_cursor() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let tokens = scan(b"BT /V 10 Tf 12 TL 1 0 0 1 30 90 Tm <0041> Tj <0042> ' 4 2 <00200043> \" 3 -7 TD <0044> Tj T* <0045> Tj ET", &mut metrics);
    for (index, xy) in [
        [30.0, 90.0],
        [30.0, 78.0],
        [30.0, 66.0],
        [33.0, 59.0],
        [33.0, 52.0],
    ]
    .iter()
    .enumerate()
    {
        close(
            &tokens[index].source_position.as_ref().unwrap().matrix()[4..],
            xy,
        );
    }
    assert_eq!(tokens[2].word_spacing, 4.0);
    assert_eq!(tokens[2].character_spacing, 2.0);
    // A two-byte code 0020 is not the single-byte 20 to which PDF Tw applies.
    let resolver = metrics.font("V").unwrap();
    close(
        &source_displacement(&tokens[2], resolver, &tokens[2].decoded).unwrap(),
        &[0.0, -16.0],
    );
}

#[test]
fn graphics_stack_restores_text_matrices_and_leading_inside_text_object() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let tokens = scan(b"BT /V 10 Tf 12 TL 1 0 0 1 30 90 Tm <0041> Tj q 1 0 0 1 100 100 Tm 20 TL <0042> Tj Q <0043> Tj T* <0044> Tj ET", &mut metrics);
    close(
        &tokens[2].source_position.as_ref().unwrap().matrix()[4..],
        &[30.0, 80.0],
    );
    close(
        &tokens[3].source_position.as_ref().unwrap().matrix()[4..],
        &[30.0, 78.0],
    );
}

#[test]
fn unresolved_font_history_cannot_create_a_fictitious_position() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let tokens = scan(
        b"BT /Missing 10 Tf (x) Tj /V 10 Tf <0041> Tj 1 0 0 1 0 50 Tm <0042> Tj ET",
        &mut metrics,
    );
    assert!(tokens[0].source_position.is_none());
    assert!(tokens[1].source_position.is_none());
    assert!(tokens[2].source_position.is_some());
}

#[test]
fn extgstate_font_does_not_reuse_stale_tf_but_transparency_keeps_positions() {
    let (engine, mut resources) = fixture();
    let mut font_state = crate::PdfDictionary::empty();
    font_state.insert(
        "Font",
        PdfObject::Array(vec![
            PdfObject::Dictionary(resources.fonts["V"].clone()),
            PdfObject::Integer(20),
        ]),
    );
    resources
        .ext_g_states
        .insert("FontState".into(), font_state);
    resources
        .ext_g_states
        .insert("Alpha".into(), crate::PdfDictionary::empty());
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let tokens = scan(b"BT /V 10 Tf /Alpha gs <0041> Tj /FontState gs <0042> Tj /V 10 Tf <0043> Tj 1 0 0 1 0 0 Tm <0044> Tj ET", &mut metrics);
    assert!(tokens[0].source_position.is_some());
    assert!(tokens[1].source_position.is_none());
    assert!(tokens[2].source_position.is_none());
    assert!(tokens[3].source_position.is_some());
}

#[test]
fn normalized_ext_font_updates_both_writing_axes_and_q_restores_the_previous_font() {
    let (engine, mut resources) = fixture();
    let mut gs = crate::PdfDictionary::empty();
    gs.insert(
        "Font",
        PdfObject::Array(vec![PdfObject::Name("V".into()), PdfObject::Integer(20)]),
    );
    resources.ext_g_states.insert("GV".into(), gs);
    resources
        .ext_g_states
        .insert("Alpha".into(), crate::PdfDictionary::empty());
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let tokens = scan(
        b"BT /H 10 Tf 1 0 0 1 20 150 Tm (A) Tj q /GV gs <0041> Tj /Alpha gs <0042> Tj Q (B) Tj ET",
        &mut metrics,
    );
    assert_eq!(
        tokens
            .iter()
            .map(|t| t.font_name.as_str())
            .collect::<Vec<_>>(),
        ["H", "V", "V", "H"]
    );
    assert_eq!(tokens[1].font_size, 20.0);
    assert_eq!(tokens[3].font_size, 10.0);
    close(
        &tokens[1].source_position.as_ref().unwrap().offset,
        &[6.0, 0.0],
    );
    close(
        &tokens[2].source_position.as_ref().unwrap().offset,
        &[6.0, -20.0],
    );
    close(
        &tokens[3].source_position.as_ref().unwrap().offset,
        &[6.0, 0.0],
    );
}

#[test]
fn ext_font_selection_crosses_streams_and_recovers_font_but_not_an_unknown_cursor() {
    let (engine, mut resources) = fixture();
    let mut gs = crate::PdfDictionary::empty();
    gs.insert(
        "Font",
        PdfObject::Array(vec![PdfObject::Name("H".into()), PdfObject::Integer(10)]),
    );
    resources.ext_g_states.insert("G".into(), gs);
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let mut state = ScannedTextTokenState::default();
    scan_text_string_tokens_with_metrics(
        b"BT /Missing 10 Tf /G gs 1 0 0 1 20 150 Tm (A) Tj",
        &mut state,
        Some((7, 0)),
        Some(&mut metrics),
    )
    .unwrap();
    let tokens = scan_text_string_tokens_with_metrics(
        b"(B) Tj /Missing 10 Tf (X) Tj /G gs (C) Tj 1 0 0 1 20 150 Tm (D) Tj ET",
        &mut state,
        Some((8, 0)),
        Some(&mut metrics),
    )
    .unwrap();
    assert_eq!(tokens[0].font_name, "H");
    close(
        &tokens[0].source_position.as_ref().unwrap().offset,
        &[6.0, 0.0],
    );
    assert!(tokens[1].source_position.is_none());
    assert!(tokens[2].source_position.is_none());
    assert_eq!(tokens[2].font_name, "H");
    assert!(tokens[3].source_position.is_some());
}

#[test]
fn zero_size_and_zero_scale_source_advances_are_not_divided_by_zero() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let tokens = scan(
        b"BT /H 0 Tf 2 Tc (AB) Tj /V 0 Tf <0041> Tj /V 10 Tf 0 Tz <0042> Tj <0043> Tj ET",
        &mut metrics,
    );
    close(
        &tokens[1].source_position.as_ref().unwrap().offset,
        &[4.0, 0.0],
    );
    close(
        &tokens[2].source_position.as_ref().unwrap().offset,
        &[4.0, 2.0],
    );
    close(
        &tokens[3].source_position.as_ref().unwrap().offset,
        &[4.0, -6.0],
    );
}

#[test]
fn all_text_show_forms_preserve_untouched_suffix_cursor_and_next_line() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    for show in [
        "<004100420043> Tj",
        "<004100420043> '",
        "3 2 <004100420043> \"",
        "[<0030> 125 <004100420043> -80 <0031>] TJ",
    ] {
        for render_mode in 0..=7 {
            let source = format!("BT /H 10 Tf (A) Tj /V 12 Tf 14 TL 1 .25 .4 1 50 150 Tm /H 10 Tf (A) Tj /V 12 Tf 2 Tc 75 Tz 3 Ts {render_mode} Tr {show} <0044> Tj T* <0045> Tj ET");
            let before = scan(source.as_bytes(), &mut metrics);
            let target = before
                .iter()
                .find(|t| t.decoded == [0, 65, 0, 66, 0, 67])
                .unwrap();
            let replacement_glyphs = glyphs();
            let (start, end, replacement) = rewrite_positioned(
                target,
                target,
                metrics.font("V").unwrap(),
                &[0, 65],
                &[0, 66],
                "AB",
                &replacement_glyphs,
                "GV",
                true,
                true,
                &[0, 67],
            )
            .unwrap();
            let mut edited = source.into_bytes();
            edited.splice(start..end, replacement);
            let after = scan(&edited, &mut metrics);
            assert!(!after.iter().any(|t| t.decoded == [0, 66]));
            assert_eq!(
                after
                    .iter()
                    .filter(|t| t.font_name == "GV" && !t.decoded.is_empty())
                    .count(),
                2
            );
            for code in [68, 69] {
                let old = before.iter().find(|t| t.decoded == [0, code]).unwrap();
                let new = after.iter().find(|t| t.decoded == [0, code]).unwrap();
                same_position(old, new);
                assert_eq!(old.text_render_mode, new.text_render_mode);
                assert_eq!(old.text_rise, new.text_rise);
                assert_eq!(old.horizontal_scaling, new.horizontal_scaling);
            }
        }
    }
}

#[test]
fn glyph_rotation_and_offsets_compose_with_source_matrix_and_rise_once() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let tokens = scan(
        b"BT /V 10 Tf 75 Tz 3 Ts 1 .25 .4 1 50 150 Tm <0041> Tj ET",
        &mut metrics,
    );
    let token = &tokens[0];
    let (_, _, emitted) = rewrite_positioned(
        token,
        token,
        metrics.font("V").unwrap(),
        &[],
        &[0, 65],
        "AB",
        &glyphs(),
        "GV",
        true,
        true,
        &[],
    )
    .unwrap();
    let mut content = b"BT /V 10 Tf 75 Tz 3 Ts 1 .25 .4 1 50 150 Tm\n".to_vec();
    content.extend_from_slice(&emitted);
    let output = scan(&content, &mut metrics);
    let first = &output[0];
    // local = clockwise rotation with translated shaped offset and source Ts.
    let expected = concat_matrix(
        &[0.0, -1.0, 1.0, 0.0, -3.8, 2.1],
        &[1.0, 0.25, 0.4, 1.0, 50.0, 150.0],
    );
    close(&first.source_position.as_ref().unwrap().matrix(), &expected);
    assert_eq!(first.text_rise, 0.0);
}

#[test]
fn horizontal_generation_restores_mixed_axis_history_and_contextual_advances() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let source = b"BT /V 10 Tf <0041> Tj /H 10 Tf (ABC) Tj (D) Tj T* (E) Tj ET";
    let before = scan(source, &mut metrics);
    let target = before.iter().find(|t| t.decoded == b"ABC").unwrap();
    let (start, end, bytes) = rewrite_positioned(
        target,
        target,
        metrics.font("H").unwrap(),
        b"A",
        b"B",
        "AB",
        &glyphs(),
        "GH",
        false,
        false,
        b"C",
    )
    .unwrap();
    let mut edited = source.to_vec();
    edited.splice(start..end, bytes);
    let after = scan(&edited, &mut metrics);
    for text in [b"D", b"E"] {
        same_position(
            before
                .iter()
                .find(|t| t.decoded.as_slice() == text)
                .unwrap(),
            after.iter().find(|t| t.decoded.as_slice() == text).unwrap(),
        );
    }
    let painted = after
        .iter()
        .filter(|t| t.font_name == "GH")
        .collect::<Vec<_>>();
    let first = painted[0].source_position.as_ref().unwrap().matrix();
    let second = painted[1].source_position.as_ref().unwrap().matrix();
    assert!((second[4] - first[4] - (7.3 - 4.9 - 1.2)).abs() < 1e-10);
}

#[test]
fn singular_matrix_and_incomplete_codes_have_explicit_behavior() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let source = b"BT /V 10 Tf 1 0 0 0 20 20 Tm <0041> Tj <0042> Tj ET";
    let before = scan(source, &mut metrics);
    let target = &before[0];
    let (start, end, bytes) = rewrite_positioned(
        target,
        target,
        metrics.font("V").unwrap(),
        &[],
        &[0, 65],
        "AB",
        &glyphs(),
        "GV",
        true,
        true,
        &[],
    )
    .unwrap();
    let mut edited = source.to_vec();
    edited.splice(start..end, bytes);
    let after = scan(&edited, &mut metrics);
    same_position(
        &before[1],
        after.iter().find(|t| t.decoded == [0, 66]).unwrap(),
    );
    assert!(source_displacement(target, metrics.font("V").unwrap(), &[0]).is_err());
}

#[test]
fn writing_mode_conversion_preserves_following_source_text() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    for (source, source_font, generated_font, generated_vertical, target_codes, following) in [
        (
            b"BT /H 10 Tf (A) Tj (B) Tj ET".as_slice(),
            "H",
            "GV",
            true,
            b"A".as_slice(),
            b"B".as_slice(),
        ),
        (
            b"BT /V 10 Tf <0041> Tj <0042> Tj ET".as_slice(),
            "V",
            "GH",
            false,
            &[0, 65],
            &[0, 66],
        ),
    ] {
        let before = scan(source, &mut metrics);
        let target = before.iter().find(|t| t.decoded == target_codes).unwrap();
        let resolver = metrics.font(source_font).unwrap();
        let (start, end, bytes) = rewrite_positioned(
            target,
            target,
            resolver,
            &[],
            target_codes,
            "AB",
            &glyphs(),
            generated_font,
            generated_vertical,
            resolver.is_vertical(),
            &[],
        )
        .unwrap();
        let mut edited = source.to_vec();
        edited.splice(start..end, bytes);
        let after = scan(&edited, &mut metrics);
        same_position(
            before.iter().find(|t| t.decoded == following).unwrap(),
            after.iter().find(|t| t.decoded == following).unwrap(),
        );
    }
}

#[test]
fn generated_vertical_inline_edit_save_reopen_and_edit_again_preserves_content_order() {
    use crate::authoring::{PageSize, PdfBuilder, StandardFont, TextStyle};
    let font = crate::render::get_fallback_font("Symbol").unwrap();
    let mut builder = PdfBuilder::new();
    builder
        .add_page(PageSize::custom(200.0, 200.0))
        .draw_text(
            "ABC",
            10.0,
            150.0,
            &TextStyle::standard(StandardFont::Helvetica, 12.0),
        )
        .unwrap();
    let original = builder.to_bytes().unwrap();
    let options = AdvancedTextEditOptions {
        region: [20.0, 20.0, 180.0, 180.0],
        font_size: 12.0,
        ..Default::default()
    };
    let (mut bytes, _) = edit_advanced_text_pdf(
        &original,
        1,
        "ABC",
        "A\u{00A7}V",
        AdvancedTextMode::ParagraphReflowVertical,
        &options,
        Some(font),
    )
    .unwrap();
    let engine = ContentEngine::open_bytes(bytes.clone()).unwrap();
    let original_contents = engine.document().get_page(1).unwrap().contents;
    for (old, replacement) in [("A\u{00A7}V", "XY\u{00A7}"), ("XY\u{00A7}", "ZA\u{00A7}")] {
        let model = analyze_multi_run_text_range(&bytes, 1).unwrap();
        let start_byte = model.logical_text.find(old).unwrap();
        let logical_start = model.logical_text[..start_byte].chars().count();
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start,
            logical_end: logical_start + old.chars().count(),
            replacement_text: replacement.into(),
            mode: AdvancedTextMode::ParagraphReflowVertical,
            style_policy: MultiRunStylePolicy::InheritLeading,
            options: options.clone(),
            final_lines: None,
        };
        let (output, report) = edit_multi_run_text_range(&bytes, &request, Some(font)).unwrap();
        assert!(report.replacement_extracts && report.reachable_source_tokens_removed);
        let reopened = ContentEngine::open_bytes(output.clone()).unwrap();
        assert_eq!(
            reopened.document().get_page(1).unwrap().contents,
            original_contents
        );
        let text = reopened.get_page_text(1).unwrap();
        assert!(text.contains(replacement));
        assert!(!text.contains(old));
        bytes = output;
    }
}

#[test]
fn editing_a_generated_rotated_glyph_again_does_not_rotate_or_offset_it_twice() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    let source = b"BT /V 10 Tf 75 Tz 3 Ts 1 .25 .4 1 50 150 Tm <0041> Tj ET";
    let original = scan(source, &mut metrics);
    let first_glyph = vec![glyphs()[0].clone()];
    let token = &original[0];
    let (start, end, replacement) = rewrite_positioned(
        token,
        token,
        metrics.font("V").unwrap(),
        &[],
        &token.decoded,
        "A",
        &first_glyph,
        "GV",
        true,
        true,
        &[],
    )
    .unwrap();
    let mut once = source.to_vec();
    once.splice(start..end, replacement);
    let once_tokens = scan(&once, &mut metrics);
    let glyph = once_tokens.iter().find(|t| t.decoded == [0, 200]).unwrap();
    assert!(matches!(glyph.generated_basis, SourceBasis::Verified(_)));
    let expected = glyph.source_position.as_ref().unwrap().matrix();
    let (start, end, replacement) = rewrite_positioned(
        glyph,
        glyph,
        metrics.font("GV").unwrap(),
        &[],
        &glyph.decoded,
        "A",
        &first_glyph,
        "GV",
        true,
        true,
        &[],
    )
    .unwrap();
    let mut twice = once.clone();
    twice.splice(start..end, replacement);
    let twice_tokens = scan(&twice, &mut metrics);
    let glyph = twice_tokens.iter().find(|t| t.decoded == [0, 200]).unwrap();
    close(&glyph.source_position.as_ref().unwrap().matrix(), &expected);
}

#[test]
fn stale_or_malformed_layout_origin_is_not_silently_used_as_geometry() {
    let (engine, resources) = fixture();
    let mut metrics = Metrics::new(&resources, engine.document().reader());
    for hint in ["1 0 0 1 0 0 1 0 0 1 500 500", "1 0 0"] {
        let source = format!("BT /V 10 Tf /Span << /ActualText null /WFTextBasisV1 [{hint}] >> BDC 1 0 0 1 10 20 Tm <0041> Tj EMC ET");
        let tokens = scan(source.as_bytes(), &mut metrics);
        let token = &tokens[0];
        assert!(matches!(token.generated_basis, SourceBasis::Invalid));
        assert!(rewrite_positioned(
            token,
            token,
            metrics.font("V").unwrap(),
            &[],
            &token.decoded,
            "AB",
            &glyphs(),
            "GV",
            true,
            true,
            &[]
        )
        .is_err());
    }
}
