//! Unexecuted source regressions for logical-axis pagination and PDF emission.
use super::*;
use crate::linked_stories::tests::{request, two_page_input};
fn close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-7, "{a} != {b}");
}
fn vertical_request(text: String, mode: WritingMode) -> LinkedStoryRequest {
    let mut r = request(text);
    r.writing_mode = mode;
    r.frames[0].rect = [10.0, 10.0, 72.0, 110.0];
    r.frames[1].rect = [90.0, 10.0, 190.0, 160.0];
    r
}
fn bounds(line: &StoryPaintLine, fonts: &[ApprovedFontAsset]) -> [f64; 4] {
    let text = line
        .text
        .trim_end_matches(crate::fonts::hard_break::is_hard_break);
    let spans = if line.font_spans.is_empty() {
        vec![crate::fonts::fallback::FontSpan {
            range: [0, text.len()],
            font_index: line.font_index,
        }]
    } else {
        crate::fonts::fallback::slice_spans(&line.font_spans, 0..text.len())
    };
    let metrics = fonts
        .iter()
        .map(|font| crate::fonts::line_layout::PreparedFontMetrics::new(&font.bytes).map(Some))
        .collect::<Result<Vec<_>>>()
        .unwrap();
    let m = if line.style_spans.is_empty() {
        let shaped = crate::fonts::vertical_fonts::shape_line_prepared(
            text,
            line.bidi.as_ref().unwrap(),
            &spans,
            fonts,
            &line.shaping,
            &metrics,
        )
        .unwrap();
        crate::fonts::vertical_fonts::measure_line_prepared(
            &shaped,
            fonts,
            &metrics,
            line.font_size,
            line.writing_mode,
        )
        .unwrap()
    } else {
        let style_ranges = line
            .style_spans
            .iter()
            .map(|style| style.range)
            .collect::<Vec<_>>();
        let styled =
            crate::fonts::fallback::intersect_styled_spans(&spans, &style_ranges, text.len())
                .unwrap();
        let settings = line
            .style_spans
            .iter()
            .map(|style| style.shaping.clone())
            .collect::<Vec<_>>();
        let sizes = line
            .style_spans
            .iter()
            .map(|style| style.font_size)
            .collect::<Vec<_>>();
        let shaped = crate::fonts::vertical_fonts::shape_styled_line_prepared(
            text,
            line.bidi.as_ref().unwrap(),
            &styled,
            fonts,
            &settings,
            &metrics,
        )
        .unwrap();
        crate::fonts::vertical_fonts::measure_styled_line_prepared(
            &shaped,
            fonts,
            &metrics,
            &sizes,
            line.writing_mode,
        )
        .unwrap()
    };
    let (left, right) = if line.writing_mode == WritingMode::VerticalRl {
        (m.descent, m.ascent)
    } else {
        (m.ascent, m.descent)
    };
    [
        line.x - left,
        line.baseline - m.advance - m.right_pad,
        line.x + right,
        line.baseline + m.left_pad,
    ]
}
fn inside(a: [f64; 4], b: [f64; 4]) {
    assert!(
        a[0] >= b[0] - 1e-7 && a[1] >= b[1] - 1e-7 && a[2] <= b[2] + 1e-7 && a[3] <= b[3] + 1e-7,
        "{a:?} outside {b:?}"
    );
}

#[test]
fn logical_axes_roundtrip_and_block_progression_do_not_mirror_glyphs() {
    for mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        let axes = Axes {
            rect: [20.0, 30.0, 160.0, 210.0],
            mode,
        };
        assert_eq!(axes.flow_rect(), [0.0, 0.0, 180.0, 140.0]);
        for point in [[0.0, 0.0], [18.0, 92.0], [180.0, 140.0]] {
            assert_eq!(axes.flow(axes.physical(point)), point);
        }
        let first = axes.physical([0.0, 130.0]);
        let next = axes.physical([0.0, 116.0]);
        assert_eq!(first[1], 210.0);
        assert_eq!(next[0] > first[0], mode == WritingMode::VerticalLr);
        let rect = [25.0, 40.0, 75.0, 120.0];
        assert_eq!(axes.rectangle(axes.rectangle(rect, false), true), rect);
    }
}

#[test]
fn vertical_text_grows_across_different_frames_and_repeated_continuation_pages() {
    for mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        let r = vertical_request("AV office e\u{301} next sentence. ".repeat(24), mode);
        let preview = super::super::layout(&r, &r.fonts, &[0], Vec::new()).unwrap();
        assert!(preview.generated_pages > 0);
        assert_eq!(
            preview
                .frames
                .iter()
                .flat_map(|f| &f.lines)
                .map(|l| l.text.as_str())
                .collect::<String>(),
            r.paragraphs[0].text
        );
        for frame in &preview.frames {
            for line in &frame.lines {
                inside(bounds(line, &r.fonts), frame.frame.rect);
                assert_eq!(line.writing_mode, mode);
            }
            for pair in frame.lines.windows(2) {
                assert_eq!(pair[1].x > pair[0].x, mode == WritingMode::VerticalLr);
            }
        }
        let mut shortened = r.clone();
        shortened.paragraphs[0].text = "Short".into();
        let after = super::super::layout(&shortened, &shortened.fonts, &[0], Vec::new()).unwrap();
        assert_eq!(after.generated_pages, 0);
        assert!(after.frames[1].lines.is_empty());
    }
}

#[test]
fn vertical_inline_styles_share_measurement_emission_and_reopen_ownership() {
    let input = two_page_input();
    for mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        let mut request = vertical_request("ABCD vertical".into(), mode);
        request.input_sha256 = hash(&input);
        request.paragraphs[0].line_height = 18.0;
        request.paragraphs[0].inline_styles = vec![StoryInlineStyleSpan {
            logical_range: [1, 4],
            preferred_font: Some("Times-Roman".into()),
            font_size: Some(15.0),
            rgb: Some([0.2, 0.5, 0.7]),
            shaping: None,
        }];
        request.fonts.push(ApprovedFontAsset {
            lookup_name: "Times-Roman".into(),
            bytes: crate::render::get_fallback_font("Times-Roman")
                .unwrap()
                .to_vec(),
        });
        let preview = preview_linked_story(&input, &request).unwrap();
        let line = preview
            .frames
            .iter()
            .flat_map(|frame| &frame.lines)
            .find(|line| !line.style_spans.is_empty())
            .unwrap();
        assert!(line
            .style_spans
            .iter()
            .any(|span| { span.font_size == 15.0 && span.rgb == [0.2, 0.5, 0.7] }));
        inside(
            bounds(line, &resolve_fonts(&input, &request).unwrap().0),
            preview.frames[0].frame.rect,
        );
        let (output, _) = apply_linked_story(&input, &request).unwrap();
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        assert_eq!(
            saved.paragraphs[0].inline_styles,
            request.paragraphs[0].inline_styles
        );
    }
}

#[test]
fn pinned_exclusions_stay_in_physical_space_in_both_column_directions() {
    for mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        let mut r = vertical_request("One two three four five six seven eight ".repeat(4), mode);
        r.frames[0].exclusions = vec![[35.0, 10.0, 50.0, 110.0]];
        let preview = super::super::layout(&r, &r.fonts, &[0], Vec::new()).unwrap();
        assert_eq!(preview.frames[0].frame.exclusions, r.frames[0].exclusions);
        for frame in &preview.frames {
            for line in &frame.lines {
                let ink = bounds(line, &r.fonts);
                inside(ink, frame.frame.rect);
                assert!(!frame.frame.exclusions.iter().any(|e| overlaps(e, &ink)));
            }
        }
    }
}

#[test]
fn keep_chain_tries_larger_vertical_frame_and_empty_breaks_allocate_safely() {
    let mut r = vertical_request("Heading".into(), WritingMode::VerticalRl);
    r.frames[0].rect = [10.0, 10.0, 24.0, 180.0];
    r.paragraphs[0].keep_with_next = true;
    let mut body = r.paragraphs[0].clone();
    body.id = "body".into();
    body.text = "one\ntwo\nthree".into();
    body.keep_with_next = false;
    body.keep_together = true;
    r.paragraphs.push(body);
    let preview = super::super::layout(&r, &r.fonts, &[0, 0], Vec::new()).unwrap();
    assert!(preview.frames[0].lines.is_empty());
    assert_eq!(preview.frames[1].lines.len(), 4);
    for i in 0..3 {
        let mut p = r.paragraphs[1].clone();
        p.id = format!("empty{i}");
        p.text.clear();
        p.break_before = true;
        r.paragraphs.push(p);
    }
    let preview = super::super::layout(&r, &r.fonts, &[0; 5], Vec::new()).unwrap();
    assert_eq!(preview.generated_pages, 3);
}

#[test]
fn vertical_incremental_layout_preserves_physical_prefix_and_reuses_context() {
    let mut r = vertical_request("ALPHA".into(), WritingMode::VerticalLr);
    for (id, text) in [("p2", "BETA"), ("p3", "GAMMA"), ("p4", "DELTA")] {
        let mut p = r.paragraphs[0].clone();
        p.id = id.into();
        p.text = text.into();
        r.paragraphs.push(p);
    }
    let indices = [0; 4];
    let old = super::super::layout(&r, &r.fonts, &indices, Vec::new()).unwrap();
    let hashes = r
        .paragraphs
        .iter()
        .map(value_hash)
        .collect::<Result<Vec<_>>>()
        .unwrap();
    r.paragraphs[2].text = "ZETA".into();
    let incremental = layout_seeded(
        &r,
        &r.fonts,
        &indices,
        Vec::new(),
        Some(LayoutSeed {
            previous: &old,
            paragraph_hashes: &hashes,
            font_indices: &indices,
        }),
    )
    .unwrap();
    let full = super::super::layout(&r, &r.fonts, &indices, Vec::new()).unwrap();
    assert!(incremental.reused_paragraphs > 0);
    assert_eq!(incremental.frames.len(), full.frames.len());
    for (a, b) in incremental.frames.iter().zip(&full.frames) {
        assert_eq!(a.frame.rect, b.frame.rect);
        assert_eq!(a.lines.len(), b.lines.len());
        for (a, b) in a.lines.iter().zip(&b.lines) {
            assert_eq!(a.text, b.text);
            close(a.x, b.x);
            close(a.baseline, b.baseline);
        }
    }
}

#[test]
fn vertical_save_reopen_growth_contraction_and_refill_keep_mode_and_fonts() {
    let input = two_page_input();
    let mut r = vertical_request(
        "Vertical save and reopen. ".repeat(24),
        WritingMode::VerticalRl,
    );
    r.input_sha256 = hash(&input);
    let (grown, preview) = apply_linked_story(&input, &r).unwrap();
    assert!(preview.generated_pages > 0);
    let mut saved = load_linked_stories(&grown).unwrap().remove(0).request;
    assert_eq!(saved.writing_mode, WritingMode::VerticalRl);
    saved.paragraphs[0].text = "Short".into();
    let (shortened, _) = apply_linked_story(&grown, &saved).unwrap();
    let mut saved = load_linked_stories(&shortened).unwrap().remove(0).request;
    saved.paragraphs[0].text = "Refill the saved vertical story. ".repeat(12);
    let (refilled, report) = apply_linked_story(&shortened, &saved).unwrap();
    assert!(report.frames.iter().skip(1).any(|f| !f.lines.is_empty()));
    let engine = ContentEngine::open_bytes(refilled).unwrap();
    for frame in report.frames {
        let text = engine.get_page_text(frame.frame.page).unwrap();
        for line in frame.lines {
            assert!(text.contains(line.text.trim()));
        }
    }
}

#[test]
fn vertical_tables_transform_cells_and_grid_without_losing_semantic_row_order() {
    let input = tables::tests::fixture(false);
    for mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        let mut r = tables::tests::request(&input, true);
        r.writing_mode = mode;
        let fonts = resolve_fonts(&input, &r).unwrap().0;
        let preview = preview_linked_story(&input, &r).unwrap();
        assert!(preview.generated_pages > 0);
        for frame in &preview.frames {
            for cell in &frame.table_cells {
                inside(cell.rect, frame.frame.rect);
            }
            for line in &frame.lines {
                inside(bounds(line, &fonts), frame.frame.rect);
            }
            assert!(frame.lines.iter().all(|l| l.writing_mode == mode));
        }
        let (output, _) = apply_linked_story(&input, &r).unwrap();
        let saved = load_linked_stories(&output).unwrap().remove(0).request;
        assert_eq!(saved.writing_mode, mode);
        assert_eq!(
            saved.table_layout.unwrap().rows.len(),
            r.table_layout.unwrap().rows.len()
        );
    }
}

#[test]
fn vertical_figure_footprints_stay_upright_and_captions_keep_with_them() {
    let input = crate::linked_stories::figure_tests::fixture();
    let mut r = crate::linked_stories::figure_tests::with_figures(&input);
    r.writing_mode = WritingMode::VerticalRl;
    let preview = preview_linked_story(&input, &r).unwrap();
    for frame in &preview.frames {
        for placement in &frame.figures {
            let figure = r
                .figures
                .iter()
                .find(|f| f.id == placement.figure_id)
                .unwrap();
            close(placement.rect[2] - placement.rect[0], figure.width);
            close(placement.rect[3] - placement.rect[1], figure.height);
            assert!(frame.paragraph_ids.contains(&figure.caption_paragraph));
            inside(placement.rect, frame.frame.rect);
        }
    }
}

#[test]
fn changing_writing_mode_invalidates_retained_preview_context() {
    let mut r = vertical_request("text".into(), WritingMode::VerticalRl);
    let first = layout_context_key(&r).unwrap();
    r.writing_mode = WritingMode::VerticalLr;
    assert_ne!(layout_context_key(&r).unwrap(), first);
    let mut value = serde_json::to_value(&r).unwrap();
    value.as_object_mut().unwrap().remove("writing_mode");
    assert_eq!(
        serde_json::from_value::<LinkedStoryRequest>(value)
            .unwrap()
            .writing_mode,
        WritingMode::HorizontalTb
    );
}

#[test]
fn universal_story_plan_discloses_and_binds_the_writing_mode() {
    use crate::universal_editing::{
        plan_universal_edit_v2, UniversalEditOperationV2, UniversalEditRequestV2,
    };
    let input = two_page_input();
    let mut story = vertical_request("Vertical plan".into(), WritingMode::VerticalRl);
    story.input_sha256 = hash(&input);
    let mut request = UniversalEditRequestV2 {
        operation: UniversalEditOperationV2::LinkedStory { request: story },
        policy: Default::default(),
    };
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    assert_eq!(plan.implementation_report["writing_mode"], "vertical_rl");
    assert_eq!(
        plan.candidates[0].source_identity["writing_mode"],
        "vertical_rl"
    );
    assert!(plan
        .approval_reasons
        .iter()
        .any(|reason| reason.contains("column progression")));
    assert!(plan
        .write_set
        .iter()
        .any(|path| path.contains("saved_flow_mode")));
    let UniversalEditOperationV2::LinkedStory { request: story } = &mut request.operation else {
        unreachable!()
    };
    story.writing_mode = WritingMode::VerticalLr;
    let other = plan_universal_edit_v2(&input, &request).unwrap();
    assert_ne!(other.plan_id, plan.plan_id);
    assert_ne!(
        other.candidates[0].candidate_id,
        plan.candidates[0].candidate_id
    );
    assert_eq!(other.implementation_report["writing_mode"], "vertical_lr");
}
