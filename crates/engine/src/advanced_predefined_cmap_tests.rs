//! Unexecuted save/reopen/source-mutation regressions for named CMaps.
use super::*;
use crate::fonts::predefined_cmap::tests::font;
use crate::fonts::variable_cmap_tests::pdf_with_font;

#[test]
fn repeated_named_cmap_deletion_preserves_remaining_variable_length_codes() {
    let mut input = pdf_with_font(
        font("90ms-RKSJ-H", "Japan1"),
        b"BT /F 10 Tf 1 0 0 1 30 700 Tm <93FA414243> Tj ET",
    );
    assert_eq!(
        analyze_multi_run_text_range(&input, 1)
            .unwrap()
            .logical_text,
        "日ABC"
    );
    for (start, end, expected) in [(0, 1, "ABC"), (1, 2, "AC")] {
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
        assert!(report.output_reopened && report.reachable_source_tokens_removed);
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

#[test]
fn named_cmap_reverse_encoding_is_used_by_same_width_patch() {
    let input = pdf_with_font(
        font("90ms-RKSJ-H", "Japan1"),
        b"BT /F 10 Tf 1 0 0 1 30 700 Tm <4142> Tj ET",
    );
    let (output, report) =
        apply_same_width_patch(&input, 1, "AB", "BA", &SameWidthPatchOptions::default()).unwrap();
    assert!(report.output_reopened && report.replacement_extracts && report.old_text_absent);
    assert_eq!(
        analyze_multi_run_text_range(&output, 1)
            .unwrap()
            .logical_text,
        "BA"
    );
}
