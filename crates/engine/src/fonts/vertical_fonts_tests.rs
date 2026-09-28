//! Regression source, intentionally unexecuted in the source-only phase.
use super::*;
fn fonts() -> Vec<ApprovedFontAsset> {
    ["Helvetica", "Times-Roman"]
        .into_iter()
        .map(|name| ApprovedFontAsset {
            lookup_name: name.into(),
            bytes: crate::render::get_fallback_font(name).unwrap().to_vec(),
        })
        .collect()
}
fn bidi(text: &str) -> LineBidi {
    super::super::shaper::resolve_line_bidi(text, 0..text.len(), ShapeOptions::default()).unwrap()
}
#[test]
fn single_font_vertical_fallback_matches_the_canonical_orientation_shaper() {
    let fonts = fonts();
    let text = "AV\u{00A7}e\u{301} next";
    let bidi = bidi(text);
    let expected =
        vertical::shape_resolved(&fonts[0].bytes, text, &bidi, &Default::default()).unwrap();
    let runs = shape_line(
        text,
        &bidi,
        &[fallback::FontSpan {
            range: [0, text.len()],
            font_index: 0,
        }],
        &fonts,
        &Default::default(),
    )
    .unwrap();
    let observed = runs
        .into_iter()
        .flat_map(|run| {
            run.glyphs.into_iter().map(move |mut g| {
                g.glyph.cluster += run.range.start as u32;
                g
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(observed.len(), expected.len());
    for (a, b) in observed.iter().zip(&expected) {
        assert_eq!(a.glyph, b.glyph);
        assert_eq!(a.rotate_clockwise, b.rotate_clockwise);
        assert_eq!(a.cross_advance, b.cross_advance);
    }
}
#[test]
fn multiscript_multifont_vertical_metrics_use_actual_outline_bounds() {
    let fonts = fonts();
    let text = "AV fi e\u{301}\u{00A7}";
    let spans = [
        fallback::FontSpan {
            range: [0, 3],
            font_index: 0,
        },
        fallback::FontSpan {
            range: [3, text.len()],
            font_index: 1,
        },
    ];
    let runs = shape_line(text, &bidi(text), &spans, &fonts, &Default::default()).unwrap();
    assert!(runs.iter().any(|r| r.font_index == 0));
    assert!(runs.iter().any(|r| r.font_index == 1));
    let rl = measure_line(&runs, &fonts, 12.0, WritingMode::VerticalRl).unwrap();
    let lr = measure_line(&runs, &fonts, 12.0, WritingMode::VerticalLr).unwrap();
    assert_eq!(rl.advance, lr.advance);
    assert_eq!(rl.ascent, lr.descent);
    assert_eq!(rl.descent, lr.ascent);
    assert!(rl.width() >= rl.advance && rl.ascent >= 6.0 && rl.descent >= 6.0);
}
#[test]
fn prepared_vertical_faces_preserve_shaping_and_metrics() {
    let fonts = fonts();
    let text = "AV fi e\u{301}\u{00A7}";
    let spans = [fallback::FontSpan {
        range: [0, text.len()],
        font_index: 0,
    }];
    let metrics = fonts
        .iter()
        .map(|font| Some(crate::fonts::line_layout::PreparedFontMetrics::new(&font.bytes).unwrap()))
        .collect::<Vec<_>>();
    let ordinary = shape_line(text, &bidi(text), &spans, &fonts, &Default::default()).unwrap();
    let prepared = shape_line_prepared(
        text,
        &bidi(text),
        &spans,
        &fonts,
        &Default::default(),
        &metrics,
    )
    .unwrap();
    assert_eq!(ordinary.len(), prepared.len());
    for (ordinary, prepared) in ordinary.iter().zip(&prepared) {
        assert_eq!(ordinary.range, prepared.range);
        assert_eq!(ordinary.font_index, prepared.font_index);
        assert_eq!(ordinary.glyphs.len(), prepared.glyphs.len());
        for (ordinary, prepared) in ordinary.glyphs.iter().zip(&prepared.glyphs) {
            assert_eq!(ordinary.glyph, prepared.glyph);
            assert_eq!(ordinary.rotate_clockwise, prepared.rotate_clockwise);
            assert_eq!(ordinary.cross_advance, prepared.cross_advance);
        }
    }
    let ordinary = measure_line(&ordinary, &fonts, 12.0, WritingMode::VerticalRl).unwrap();
    let prepared =
        measure_line_prepared(&prepared, &fonts, &metrics, 12.0, WritingMode::VerticalRl).unwrap();
    assert_eq!(ordinary.advance, prepared.advance);
    assert_eq!(ordinary.left_pad, prepared.left_pad);
    assert_eq!(ordinary.right_pad, prepared.right_pad);
    assert_eq!(ordinary.ascent, prepared.ascent);
    assert_eq!(ordinary.descent, prepared.descent);
}
#[test]
fn font_spans_cannot_split_a_combining_grapheme() {
    let fonts = fonts();
    let text = "e\u{301}";
    let split = [
        fallback::FontSpan {
            range: [0, 1],
            font_index: 0,
        },
        fallback::FontSpan {
            range: [1, text.len()],
            font_index: 1,
        },
    ];
    assert!(shape_line(text, &bidi(text), &split, &fonts, &Default::default()).is_err());
}
#[test]
fn vertical_coverage_handles_paragraph_breaks_without_painting_separators() {
    let fonts = fonts();
    assert!(covers(
        &fonts[0].bytes,
        "first\r\nsecond\u{2028}third",
        ShapeOptions::default(),
        &Default::default()
    )
    .unwrap());
    let selected = fallback::resolve_contextual_fonts_for_mode(
        "first\nsecond",
        &fonts,
        &[(0, 0.0), (1, 1.0)],
        ShapeOptions::default(),
        &Default::default(),
        WritingMode::VerticalRl,
    )
    .unwrap();
    assert_eq!(selected.first().unwrap().range[0], 0);
    assert_eq!(selected.last().unwrap().range[1], 12);
}
#[test]
fn sideways_bidi_across_font_changes_uses_whole_run_visual_order() {
    let fonts = fonts();
    let text = "ABC 123 DEF";
    let bidi = LineBidi {
        levels: vec![1; text.len()],
        rtl: true,
        context: Default::default(),
    };
    let spans = [
        fallback::FontSpan {
            range: [0, 4],
            font_index: 0,
        },
        fallback::FontSpan {
            range: [4, text.len()],
            font_index: 1,
        },
    ];
    let horizontal =
        fallback::shape_line(text, &bidi, &spans, &fonts, &Default::default()).unwrap();
    let vertical = shape_line(text, &bidi, &spans, &fonts, &Default::default()).unwrap();
    assert_eq!(horizontal.len(), vertical.len());
    for (h, v) in horizontal.iter().zip(&vertical) {
        assert_eq!(h.range, v.range);
        assert_eq!(h.font_index, v.font_index);
        assert_eq!(
            h.shaped
                .glyphs
                .iter()
                .map(|g| g.glyph_id)
                .collect::<Vec<_>>(),
            v.glyphs
                .iter()
                .map(|g| g.glyph.glyph_id)
                .collect::<Vec<_>>()
        );
        assert!(v.glyphs.iter().all(|g| g.rotate_clockwise));
    }
}
