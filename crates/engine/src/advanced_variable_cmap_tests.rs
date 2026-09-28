//! Unexecuted source regressions for variable-code source editing.
use super::*;
use crate::fonts::variable_cmap_tests::{font_dictionary, pdf, ENCODED, LOGICAL};

#[test]
fn scalar_selection_preserves_exact_variable_code_boundaries() {
    let resolver = FontResolver::new_from_dict_only(&font_dictionary(false));
    for (scalar, byte) in [(0, 0), (1, 1), (2, 3), (4, 6), (5, 10), (6, 11)] {
        assert_eq!(
            source_byte_offset_for_scalar(&resolver, ENCODED, scalar).unwrap(),
            byte
        );
    }
    assert!(
        source_byte_offset_for_scalar(&resolver, ENCODED, 3).is_err(),
        "a source fi ligature is indivisible"
    );
    assert_eq!(
        encode_with_existing_font(&resolver, LOGICAL).unwrap(),
        (ENCODED.to_vec(), false)
    );
}

#[test]
fn source_displacement_uses_encoded_codes_not_unicode_scalar_count() {
    let body = b"BT /F 10 Tf 1 Tc 2 Tw 1 0 0 1 30 700 Tm <4100418100019000004120> Tj ET";
    let token = scan_text_string_tokens(body).unwrap().remove(0);
    for vertical in [false, true] {
        let resolver = FontResolver::new_from_dict_only(&font_dictionary(vertical));
        let advance = inline_text::source_displacement(&token, &resolver, ENCODED).unwrap();
        assert_eq!(advance, if vertical { [0.0, -39.0] } else { [34.0, 0.0] });
    }
}

#[test]
fn preserved_style_measurement_matches_all_emitted_spacing() {
    let resolver = FontResolver::new_from_dict_only(&font_dictionary(false));
    let body = b"BT /F 10 Tf 1 Tc 2 Tw 1 0 0 1 30 700 Tm <4100418100019000004120> Tj ET";
    let token = scan_text_string_tokens(body).unwrap().remove(0);
    let style = preserved_style_from_token(&token).unwrap();
    assert_eq!(
        preserved_run_advance(&resolver, ENCODED, LOGICAL, &style).unwrap(),
        34.0
    );
    assert_eq!(
        preserved_run_advance(&resolver, &[0x81, 0, 1], "fi", &style).unwrap(),
        7.0
    );
}

#[test]
fn same_width_patch_includes_encoded_word_spacing_in_eligibility() {
    let mut font = crate::PdfDictionary::empty();
    font.insert("Subtype", crate::PdfObject::Name("Type1".into()));
    font.insert("BaseFont", crate::PdfObject::Name("Courier".into()));
    let resolver = FontResolver::new_from_dict_only(&font);
    for (spacing, expected) in [(0.0, true), (2.0, false), (-2.0, false)] {
        let body = format!("BT /F 10 Tf {spacing} Tw <20> Tj ET");
        let token = scan_text_string_tokens(body.as_bytes()).unwrap().remove(0);
        let result = evaluate_patch_candidate(
            1,
            10,
            0,
            &crate::PdfDictionary::empty(),
            &token,
            &font,
            &resolver,
            "A",
            &SameWidthPatchOptions::default(),
            false,
            false,
        );
        assert_eq!(result.eligible, expected, "{}", result.exact_reason);
        if !expected {
            assert!(result.exact_reason.contains("word-spacing"));
        }
    }
}

#[test]
fn same_width_patch_saves_and_reopens_with_variable_code_reordering() {
    let input = pdf(
        false,
        b"BT /F 10 Tf 1 0 0 1 30 700 Tm <41004181000190000041> Tj ET",
    );
    let options = SameWidthPatchOptions::default();
    let analysis =
        analyze_same_width_patch(&input, 1, "ABfi\u{1f600}", "BAfi\u{1f600}", &options).unwrap();
    assert_eq!(analysis.candidates.len(), 1);
    assert!(analysis.candidates[0].eligible);
    let (output, report) =
        apply_same_width_patch(&input, 1, "ABfi\u{1f600}", "BAfi\u{1f600}", &options).unwrap();
    assert!(report.output_reopened && report.replacement_extracts);
    assert!(output.starts_with(&input));
    assert_eq!(
        analyze_multi_run_text_range(&output, 1)
            .unwrap()
            .logical_text,
        "BAfi\u{1f600}"
    );
}

#[test]
fn repeated_multi_run_deletion_keeps_unsplit_remaining_source_codes() {
    let mut input = pdf(
        false,
        b"BT /F 10 Tf 1 0 0 1 30 700 Tm <41004181000190000041> Tj ET",
    );
    for (start, end, expected) in [(1, 2, "Afi\u{1f600}"), (1, 3, "A\u{1f600}")] {
        let request = MultiRunTextRangeRequest {
            page: 1,
            logical_start: start,
            logical_end: end,
            replacement_text: String::new(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::ExplicitSupplied,
            options: AdvancedTextEditOptions {
                region: [20.0, 20.0, 300.0, 750.0],
                ..Default::default()
            },
            final_lines: None,
        };
        let (output, report) = edit_multi_run_text_range(&input, &request, None).unwrap();
        assert!(report.reachable_source_tokens_removed && report.output_reopened);
        assert!(output.starts_with(&input));
        assert_eq!(
            analyze_multi_run_text_range(&output, 1)
                .unwrap()
                .logical_text,
            expected
        );
        input = output;
    }
}
