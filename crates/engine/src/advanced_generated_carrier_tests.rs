//! Source-only regressions. No test/PDF workload was executed for this change.
use super::*;

fn input() -> Vec<u8> {
    tests::advanced_editing_fixture_with_content(false, b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n")
}

fn options() -> AdvancedTextEditOptions {
    AdvancedTextEditOptions {
        region: [10.0, 10.0, 190.0, 190.0],
        font_size: 12.0,
        ..Default::default()
    }
}

fn request(
    text: &str,
    mode: AdvancedTextMode,
    policy: MultiRunStylePolicy,
) -> MultiRunTextRangeRequest {
    MultiRunTextRangeRequest {
        page: 1,
        logical_start: 0,
        logical_end: 3,
        replacement_text: text.into(),
        mode,
        style_policy: policy,
        options: options(),
        final_lines: None,
    }
}

fn source_text(output: &[u8]) -> String {
    ContentEngine::open_bytes(output.to_vec())
        .unwrap()
        .collect_page_text_chunks(1)
        .unwrap()
        .into_iter()
        .map(|c| c.text)
        .collect()
}

#[test]
fn control_only_replacements_have_real_source_codes_and_can_be_edited_again() {
    let logical = "\u{00ad}\u{200d}\u{e0100}";
    for mode in [
        AdvancedTextMode::ParagraphReflowHorizontal,
        AdvancedTextMode::ParagraphReflowRtl,
        AdvancedTextMode::ParagraphReflowVertical,
    ] {
        for policy in [
            MultiRunStylePolicy::ExplicitSupplied,
            MultiRunStylePolicy::PreservePerSegment,
            MultiRunStylePolicy::InheritLeading,
            MultiRunStylePolicy::InheritTrailing,
        ] {
            let edit = request(logical, mode, policy);
            let (output, _) = edit_multi_run_text_range(&input(), &edit, None).unwrap();
            assert_eq!(source_text(&output), logical);
            let model = analyze_multi_run_text_range(&output, 1).unwrap();
            assert_eq!(model.logical_text, logical);
            let mut next = request(
                "NEW",
                AdvancedTextMode::ParagraphReflowHorizontal,
                MultiRunStylePolicy::ExplicitSupplied,
            );
            next.logical_end = logical.chars().count();
            let (output, _) = edit_multi_run_text_range(&output, &next, None).unwrap();
            assert_eq!(source_text(&output), "NEW");
        }
    }
}

#[test]
fn bounded_and_inserted_controls_keep_source_order_without_duplicate_extraction() {
    let logical = "\u{200d}\u{fe0f}";
    let (output, _) = edit_advanced_text_pdf(
        &input(),
        1,
        "ABC",
        logical,
        AdvancedTextMode::ParagraphReflowHorizontal,
        &options(),
        None,
    )
    .unwrap();
    assert_eq!(source_text(&output), logical);
    for position in [0, 1, 3] {
        let mut edit = request(
            logical,
            AdvancedTextMode::ParagraphReflowHorizontal,
            MultiRunStylePolicy::ExplicitSupplied,
        );
        edit.logical_start = position;
        edit.logical_end = position;
        let (output, _) = edit_multi_run_text_range(&input(), &edit, None).unwrap();
        assert_eq!(
            source_text(&output),
            format!("{}{logical}{}", &"ABC"[..position], &"ABC"[position..])
        );
    }
}

#[test]
fn inline_tagged_and_clipping_controls_keep_the_following_source_endpoint() {
    for tagged in [false, true] {
        let content = if tagged {
            b"BT /F1 12 Tf 3 Tc 10 150 Td /Span << /ActualText (ABC) >> BDC (ABC) Tj EMC (Z) Tj ET\n".as_slice()
        } else {
            b"BT /F1 12 Tf 3 Tc 7 Tr 10 150 Td (ABC) Tj (Z) Tj ET\n".as_slice()
        };
        let input = tests::advanced_editing_fixture_with_content(false, content);
        let before = ContentEngine::open_bytes(input.clone())
            .unwrap()
            .collect_page_text_chunks(1)
            .unwrap();
        let before_suffix = before.iter().find(|chunk| chunk.text == "Z").unwrap();
        let edit = request(
            "\u{200d}\u{200c}",
            AdvancedTextMode::ParagraphReflowHorizontal,
            MultiRunStylePolicy::PreservePerSegment,
        );
        let (output, _) = edit_multi_run_text_range(&input, &edit, None).unwrap();
        assert_eq!(source_text(&output), "\u{200d}\u{200c}Z");
        let engine = ContentEngine::open_bytes(output).unwrap();
        let after = engine.collect_page_text_chunks(1).unwrap();
        let after_suffix = after.iter().find(|chunk| chunk.text == "Z").unwrap();
        assert!((after_suffix.x - before_suffix.x).abs() < 1e-8);
        assert!((after_suffix.y - before_suffix.y).abs() < 1e-8);
        assert!(after
            .iter()
            .filter(|chunk| chunk.text.contains('\u{200d}'))
            .all(|chunk| chunk.width == 0.0));
    }
}

#[test]
fn bounded_reflow_preserves_separator_only_replacement_in_every_writing_mode() {
    for mode in [
        AdvancedTextMode::ParagraphReflowHorizontal,
        AdvancedTextMode::ParagraphReflowRtl,
        AdvancedTextMode::ParagraphReflowVertical,
    ] {
        for separator in [
            "\r", "\n", "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
        ] {
            let input = input();
            let (output, report) = edit_advanced_text_pdf(
                &input,
                1,
                "ABC",
                separator,
                mode,
                &options(),
                Some(get_fallback_font("Symbol").unwrap()),
            )
            .unwrap();
            assert!(report.removed_old_reachable_content);
            assert_eq!(source_text(&output), separator);
            assert_eq!(
                analyze_multi_run_text_range(&output, 1)
                    .unwrap()
                    .logical_text,
                separator
            );
        }
    }
}

#[test]
fn generated_and_preserved_style_routes_keep_glyphless_replacements() {
    for mode in [
        AdvancedTextMode::ParagraphReflowHorizontal,
        AdvancedTextMode::ParagraphReflowRtl,
        AdvancedTextMode::ParagraphReflowVertical,
    ] {
        for policy in [
            MultiRunStylePolicy::ExplicitSupplied,
            MultiRunStylePolicy::PreservePerSegment,
            MultiRunStylePolicy::InheritLeading,
            MultiRunStylePolicy::InheritTrailing,
        ] {
            let edit = request("\n\r\n\u{2028}", mode, policy);
            let (output, report) = edit_multi_run_text_range(&input(), &edit, None).unwrap();
            assert!(report.reachable_source_tokens_removed);
            assert_eq!(source_text(&output), edit.replacement_text);
            let engine = ContentEngine::open_bytes(output.clone()).unwrap();
            let carrier_chunks = engine.collect_page_text_chunks(1).unwrap();
            assert_eq!(carrier_chunks.len(), 1);
            assert_eq!(carrier_chunks[0].width, 0.0);
            assert_eq!(
                carrier_chunks[0].is_vertical,
                mode == AdvancedTextMode::ParagraphReflowVertical
            );
            assert_eq!(
                analyze_multi_run_text_range(&output, 1)
                    .unwrap()
                    .logical_text,
                edit.replacement_text
            );
        }
    }
}

#[test]
fn blank_replacements_can_be_reselected_and_replaced_again_by_source_codes() {
    let edit = request(
        "\r\n\n",
        AdvancedTextMode::ParagraphReflowHorizontal,
        MultiRunStylePolicy::ExplicitSupplied,
    );
    let (output, _) = edit_multi_run_text_range(&input(), &edit, None).unwrap();
    let model = analyze_multi_run_text_range(&output, 1).unwrap();
    assert_eq!(model.logical_text, edit.replacement_text);
    let mut next = request(
        "NEW",
        AdvancedTextMode::ParagraphReflowHorizontal,
        MultiRunStylePolicy::ExplicitSupplied,
    );
    next.logical_end = model.logical_text.chars().count();
    let (second, _) = edit_multi_run_text_range(&output, &next, None).unwrap();
    assert_eq!(source_text(&second), "NEW");
}

#[test]
fn zero_width_separator_insertion_keeps_source_order_without_double_extraction() {
    for insertion in [0, 1, 3] {
        let mut edit = request(
            "\n",
            AdvancedTextMode::ParagraphReflowHorizontal,
            MultiRunStylePolicy::ExplicitSupplied,
        );
        edit.logical_start = insertion;
        edit.logical_end = insertion;
        let (output, _) = edit_multi_run_text_range(&input(), &edit, None).unwrap();
        let expected = format!("{}\n{}", &"ABC"[..insertion], &"ABC"[insertion..]);
        assert_eq!(source_text(&output), expected);
        let engine = ContentEngine::open_bytes(output).unwrap();
        let mut collector = crate::text::TextCollector::new(
            engine.get_page_resources(1).unwrap(),
            engine.document().reader(),
        );
        let operations = engine.get_page_content(1).unwrap();
        assert_eq!(
            collector
                .collect(&operations)
                .into_iter()
                .map(|c| c.text)
                .collect::<String>(),
            expected
        );
    }
}

#[test]
fn hard_breaks_inherited_without_explicit_layout_keep_exact_fonts_and_logical_text() {
    let input = input();
    for separator in ["\r\n", "\u{000b}", "\u{000c}", "\u{2028}"] {
        let replacement = format!("A{separator}{separator}B");
        let edit = request(
            &replacement,
            AdvancedTextMode::ParagraphReflowHorizontal,
            MultiRunStylePolicy::PreservePerSegment,
        );
        let (output, report) = edit_multi_run_text_range(&input, &edit, None).unwrap();
        assert_eq!(report.operation, "replace_preserving_per_segment_styles");
        assert_eq!(source_text(&output), replacement);
        let engine = ContentEngine::open_bytes(output).unwrap();
        let resources = engine.get_page_resources(1).unwrap();
        assert!(!resources.fonts.values().any(story_carriers::is_font));
        let ops = engine.get_page_content(1).unwrap();
        let matrices = ops
            .iter()
            .filter(|op| op.operator == "Tm")
            .filter_map(|op| op.operand(5).and_then(crate::content::Operand::as_number))
            .collect::<Vec<_>>();
        assert!(matrices.windows(2).any(|pair| pair[0] > pair[1]));
    }
}

#[test]
fn justification_leaves_blank_lines_unexpanded_in_horizontal_and_vertical_routes() {
    for mode in [
        AdvancedTextMode::ParagraphReflowHorizontal,
        AdvancedTextMode::ParagraphReflowVertical,
    ] {
        for policy in [
            MultiRunStylePolicy::ExplicitSupplied,
            MultiRunStylePolicy::PreservePerSegment,
        ] {
            let mut edit = request("\n\n", mode, policy);
            edit.options.alignment = GeneratedTextAlignment::Justify;
            edit.options.justify_last_line = true;
            edit.options.max_word_spacing = 0.0;
            edit.options.max_character_spacing = 0.0;
            let (output, _) = edit_multi_run_text_range(&input(), &edit, None).unwrap();
            assert_eq!(source_text(&output), edit.replacement_text);
        }
    }
}

#[test]
fn explicit_and_inherited_hard_line_layouts_cannot_overflow_the_frame_height() {
    for policy in [
        MultiRunStylePolicy::ExplicitSupplied,
        MultiRunStylePolicy::PreservePerSegment,
    ] {
        for explicit in [false, true] {
            let mut edit = request(
                "\n\n\n",
                AdvancedTextMode::ParagraphReflowHorizontal,
                policy,
            );
            edit.options.region = [10.0, 10.0, 190.0, 25.0]; // one 12-point line
            if explicit {
                edit.final_lines = Some(
                    (0..3)
                        .map(|_| ExplicitLayoutLine {
                            logical_text: "\n".into(),
                            visual_text: String::new(),
                            inserted_visual_hyphen: false,
                            bidi: None,
                        })
                        .collect(),
                );
            }
            assert!(edit_multi_run_text_range(&input(), &edit, None).is_err());
        }
    }
}

#[test]
fn bounded_mixed_text_retains_exact_separator_scalars_in_actual_text() {
    for separator in [
        "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
    ] {
        let replacement = format!("A{separator}{separator}B");
        let (output, _) = edit_advanced_text_pdf(
            &input(),
            1,
            "ABC",
            &replacement,
            AdvancedTextMode::ParagraphReflowHorizontal,
            &options(),
            None,
        )
        .unwrap();
        assert_eq!(source_text(&output), replacement);
    }
}

#[test]
fn line_capacity_clamps_before_conversion_and_rejects_invalid_derived_geometry() {
    let mut options = options();
    options.region = [0.0, 0.0, 100.0, 1e300];
    options.max_lines_or_columns = 10_000;
    assert_eq!(horizontal_line_capacity(&options).unwrap(), 10_000);
    options.font_size = 1e-200;
    assert_eq!(horizontal_line_capacity(&options).unwrap(), 10_000);
    options.font_size = f64::MAX;
    options.line_spacing = 2.0;
    assert!(horizontal_line_capacity(&options).is_err());
    options.font_size = 12.0;
    options.region[1] = -f64::MAX;
    options.region[3] = f64::MAX;
    assert!(horizontal_line_capacity(&options).is_err());
    options.region = [0.0, 0.0, 100.0, 100.0];
    options.font_size = f64::MIN_POSITIVE;
    options.line_spacing = f64::MIN_POSITIVE;
    assert!(horizontal_line_capacity(&options).is_err());
    options.font_size = 12.0;
    options.line_spacing = 1.0;
    options.region[3] = 6.0;
    assert_eq!(horizontal_line_capacity(&options).unwrap(), 0);
    options.max_word_spacing = f64::NAN;
    assert!(horizontal_line_capacity(&options).is_err());
}

#[test]
fn hard_only_carriers_remain_inside_clipping_or_tagged_inline_scopes() {
    for source in [
        b"BT /F1 12 Tf 7 Tr (ABC) Tj ET".as_slice(),
        b"/P << /MCID 1 >> BDC BT /F1 12 Tf (ABC) Tj ET EMC".as_slice(),
    ] {
        let input = tests::advanced_editing_fixture_with_content(false, source);
        let edit = request(
            "\n",
            AdvancedTextMode::ParagraphReflowHorizontal,
            MultiRunStylePolicy::PreservePerSegment,
        );
        let (output, report) = edit_multi_run_text_range(&input, &edit, None).unwrap();
        assert!(report.reachable_source_tokens_removed);
        assert_eq!(source_text(&output), "\n");
        let model = analyze_multi_run_text_range(&output, 1).unwrap();
        assert_eq!(model.logical_text, "\n");
        assert_eq!(model.source_spans.len(), 1);
        if source[0] == b'/' {
            assert!(model.source_spans[0].marked_content_depth > 0);
        } else {
            assert_eq!(model.source_spans[0].text_render_mode, 7);
        }
    }
}
