//! Unexecuted vertical writer regressions; no PDF workloads are authorized yet.
use super::*;
fn font() -> &'static [u8] {
    crate::render::get_fallback_font("Symbol").unwrap()
}
fn options() -> AdvancedTextEditOptions {
    AdvancedTextEditOptions {
        region: [20.0, 20.0, 180.0, 180.0],
        font_size: 12.0,
        ..Default::default()
    }
}
fn matrices(content: &str) -> Vec<[f64; 6]> {
    content
        .lines()
        .filter(|l| l.contains(" Tm "))
        .map(|line| {
            let mut values = line.split_whitespace();
            std::array::from_fn(|_| values.next().unwrap().parse().unwrap())
        })
        .collect()
}
fn within(glyphs: &[GeneratedGlyph], content: &str, size: f64, region: [f64; 4]) {
    let ms = matrices(content);
    assert_eq!(ms.len(), glyphs.len());
    for (glyph, m) in glyphs.iter().zip(ms) {
        if let Some(b) = glyph.bounds {
            for x in [b[0], b[2]] {
                for y in [b[1], b[3]] {
                    let px = m[0] * x * size / 1000.0 + m[2] * y * size / 1000.0 + m[4];
                    let py = m[1] * x * size / 1000.0 + m[3] * y * size / 1000.0 + m[5];
                    assert!(px >= region[0] - 1e-5 && px <= region[2] + 1e-5, "x {px}");
                    assert!(py >= region[1] - 1e-5 && py <= region[3] + 1e-5, "y {py}");
                }
            }
        }
    }
}
#[test]
fn actual_shaped_origins_are_applied_once_and_ink_stays_inside_column() {
    let glyphs = glyph_plan("A\u{00A7}Ve\u{301}", font(), None).unwrap();
    let options = options();
    let (content, report) = serialize_generated_text(
        &[glyphs.clone()],
        "FV",
        &options,
        true,
        None,
        Some("A\u{00A7}Ve\u{301}"),
    )
    .unwrap();
    within(&glyphs, &content, options.font_size, options.region);
    assert!(content.contains("0 -1 1 0"));
    assert!(content.contains("1 0 0 1"));
    assert!(content.contains("/ActualText"));
    assert_eq!(
        report[0].natural_width,
        extent(&glyphs, options.font_size).unwrap()
    );
}
#[test]
fn cid_vertical_metrics_are_independent_of_horizontal_width_and_have_zero_origins() {
    let glyphs = glyph_plan("A\u{00A7}", font(), None).unwrap();
    let objects =
        build_type0_font_objects(font(), &glyphs, true, 100, 101, 102, 103, 104, 105).unwrap();
    let descendant = objects
        .iter()
        .find(|o| o.number == 104)
        .unwrap()
        .object
        .as_dict()
        .unwrap();
    let widths = descendant.get_array("W").unwrap();
    let vertical = descendant.get_array("W2").unwrap();
    for (index, glyph) in glyphs.iter().enumerate() {
        assert_eq!(widths[index * 2].as_integer(), Some(i64::from(glyph.cid)));
        assert!(
            (widths[index * 2 + 1].as_array().unwrap()[0]
                .as_number()
                .unwrap()
                - glyph.font_width)
                .abs()
                < 1e-6
        );
        let metrics = vertical[index * 2 + 1].as_array().unwrap();
        assert!((metrics[0].as_number().unwrap() + glyph.advance).abs() < 1e-6);
        assert_eq!(metrics[1].as_integer(), Some(0));
        assert_eq!(metrics[2].as_integer(), Some(0));
    }
}
#[test]
fn vertical_lines_reuse_logical_breaking_and_never_slice_combining_glyph_arrays() {
    let mut options = options();
    options.region = [0.0, 0.0, 200.0, 32.0];
    let text = "office e\u{301} AV next\nlast";
    let columns = layout(text, font(), &options).unwrap();
    assert!(columns.len() > 1);
    for column in &columns {
        assert!(extent(column, options.font_size).unwrap() <= 32.0 + 1e-7);
        assert!(!column.iter().any(|g| g.visual_unicode == "\u{301}"));
    }
    let (content, _) =
        serialize_generated_text(&columns, "FV", &options, true, None, Some(text)).unwrap();
    assert!(content.contains("/ActualText"));
}
#[test]
fn end_alignment_and_justification_use_final_extent_without_splitting_mark_clusters() {
    let glyphs = glyph_plan("A V e\u{301}", font(), None).unwrap();
    let mut options = options();
    options.alignment = GeneratedTextAlignment::End;
    let (content, _) =
        serialize_generated_text(&[glyphs.clone()], "FV", &options, true, None, None).unwrap();
    within(&glyphs, &content, options.font_size, options.region);
    options.alignment = GeneratedTextAlignment::Justify;
    options.justify_last_line = true;
    options.region[1] = options.region[3] - extent(&glyphs, options.font_size).unwrap() - 6.0;
    let (content, adjustments) =
        serialize_generated_text(&[glyphs.clone()], "FV", &options, true, None, None).unwrap();
    assert!(adjustments[0].residual.abs() < 1e-7);
    assert!(adjustments[0].word_spacing > 0.0);
    within(&glyphs, &content, options.font_size, options.region);
}
#[test]
fn columns_cannot_silently_overflow_left_edge_or_clip_a_tall_glyph() {
    let glyphs = glyph_plan("AV", font(), None).unwrap();
    let mut options = options();
    options.region = [0.0, 0.0, 13.0, 100.0];
    assert!(serialize_generated_text(
        &[glyphs.clone(), glyphs.clone()],
        "FV",
        &options,
        true,
        None,
        None
    )
    .is_err());
    options.region = [0.0, 0.0, 100.0, 1.0];
    assert!(serialize_generated_text(&[glyphs], "FV", &options, true, None, None).is_err());
}
#[test]
fn positioned_vertical_columns_use_their_own_rectangles() {
    let glyphs = glyph_plan("A\u{00A7}", font(), None).unwrap();
    let options = options();
    let regions = [[30.0, 20.0, 60.0, 150.0], [110.0, 40.0, 140.0, 180.0]];
    let (content, _) = serialize_generated_text(
        &[glyphs.clone(), glyphs.clone()],
        "FV",
        &options,
        true,
        Some(&regions),
        None,
    )
    .unwrap();
    let ms = matrices(&content);
    assert!(ms[..glyphs.len()].iter().all(|m| m[4] < 70.0));
    assert!(ms[glyphs.len()..].iter().all(|m| m[4] > 100.0));
}
#[test]
fn styled_vertical_layout_accounts_for_source_spacing_and_scale() {
    let glyphs = glyph_plan("AV", font(), None).unwrap();
    let options = options();
    let span = PreservedStyleSpan {
        byte_start: 0,
        byte_end: 2,
        style: PreservedTextStyle {
            font_resource: "FV".into(),
            font_size: 12.0,
            character_spacing: 0.5,
            word_spacing: 0.0,
            horizontal_scaling: 125.0,
            text_rise: 1.0,
            text_render_mode: 0,
            fill_color_command: "0 g".into(),
            stroke_color_command: "0 G".into(),
            vertical: false,
        },
    };
    let expected = metrics(
        &glyphs,
        options.font_size,
        Some(std::slice::from_ref(&span)),
    )
    .unwrap()
    .height();
    let (_, report) =
        serialize_generated_preserved_styles(&[glyphs], &[span], "FV", &options, true, Some("AV"))
            .unwrap();
    assert!((report[0].natural_width - expected).abs() < 1e-7);
}
#[test]
fn vertical_source_edit_reopens_and_its_codes_can_be_deleted_through_logical_selection() {
    use crate::authoring::{PageSize, PdfBuilder, StandardFont, TextStyle};
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
    let input = builder.to_bytes().unwrap();
    let options = options();
    let text = "A\u{00A7}Ve\u{301}";
    let (output, report) = edit_advanced_text_pdf(
        &input,
        1,
        "ABC",
        text,
        AdvancedTextMode::ParagraphReflowVertical,
        &options,
        Some(font()),
    )
    .unwrap();
    assert!(report.replacement_extracts && report.removed_old_reachable_content);
    let reopened = ContentEngine::open_bytes(output.clone()).unwrap();
    assert!(reopened.get_page_text(1).unwrap().contains(text));
    let model = analyze_multi_run_text_range(&output, 1).unwrap();
    assert!(model.logical_text.contains(text));
    let start = model.logical_text.find(text).unwrap();
    let logical_start = model.logical_text[..start].chars().count();
    // Full source removal must delete the old visible codes. Separate inline
    // regressions exercise rotated replacement and repeated source editing.
    let (deleted, report) = edit_multi_run_text_range(
        &output,
        &MultiRunTextRangeRequest {
            page: 1,
            logical_start,
            logical_end: logical_start + text.chars().count(),
            replacement_text: String::new(),
            mode: AdvancedTextMode::ParagraphReflowVertical,
            style_policy: MultiRunStylePolicy::ExplicitSupplied,
            options,
            final_lines: None,
        },
        Some(font()),
    )
    .unwrap();
    assert!(report.reachable_source_tokens_removed);
    assert!(!ContentEngine::open_bytes(deleted)
        .unwrap()
        .get_page_text(1)
        .unwrap()
        .contains(text));
}
