//! Unexecuted source regressions. Direct Rustybuzz expectations check plumbing,
//! not independent typography, raster output or cross-engine conformance.
use super::*;
use crate::editing_transactions::ApprovedFontAsset;
use crate::fonts::{
    fallback,
    shaper::{LineBidi, OpenTypeSettings, ParagraphBidi},
    ShapeOptions, ShapedGlyph, TextDirection, TextShaper,
};

fn font() -> &'static [u8] {
    crate::render::get_fallback_font("Symbol").unwrap()
}
fn rtl() -> ShapeOptions {
    ShapeOptions {
        direction: Some(TextDirection::RightToLeft),
    }
}
fn direct(text: &str, range: Range<usize>) -> Vec<ShapedGlyph> {
    let face = rustybuzz::Face::from_slice(font(), 0).unwrap();
    let scale = 1000.0 / f64::from(face.units_per_em());
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
    buffer.push_str(&text[range.clone()]);
    buffer.set_pre_context(&text[..range.start]);
    buffer.set_post_context(&text[range.end..]);
    buffer.set_direction(rustybuzz::Direction::RightToLeft);
    buffer.set_script("Arab".parse().unwrap());
    buffer.guess_segment_properties();
    let shaped = rustybuzz::shape(&face, &[], buffer);
    shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .map(|(g, p)| ShapedGlyph {
            glyph_id: g.glyph_id as u16,
            cluster: g.cluster,
            advance: f64::from(p.x_advance) * scale,
            offset_x: f64::from(p.x_offset) * scale,
            offset_y: f64::from(p.y_offset) * scale,
        })
        .collect()
}
fn same_glyphs(a: &[ShapedGlyph], b: &[ShapedGlyph]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert_eq!((a.glyph_id, a.cluster), (b.glyph_id, b.cluster));
        assert!((a.advance - b.advance).abs() < 1e-7);
        assert!((a.offset_x - b.offset_x).abs() < 1e-7);
        assert!((a.offset_y - b.offset_y).abs() < 1e-7);
    }
}

#[test]
fn bounded_edges_preserve_utf8_and_nested_slice_context() {
    let source = "0123456789";
    let parent = ShapingContext::default().slice(source, 2..8).unwrap();
    assert_eq!(parent.before, "01");
    assert_eq!(parent.after, "89");
    assert_eq!(
        parent.slice(&source[2..8], 2..4).unwrap(),
        ShapingContext::default().slice(source, 4..6).unwrap()
    );
    let source = "😀".repeat(32);
    let context = ShapingContext::default().slice(&source, 40..44).unwrap();
    assert_eq!(context.before, "😀".repeat(5));
    assert_eq!(context.after, "😀".repeat(5));
    assert_eq!(context.before.len(), 20);
    context.validate().unwrap();
}

#[test]
fn hard_breaks_stop_neighbour_context_in_both_directions() {
    for separator in [
        "\r", "\n", "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
    ] {
        let text = format!("ب{separator}ب");
        let before = ShapingContext::default().slice(&text, 0..2).unwrap();
        let after = ShapingContext::default()
            .slice(&text, text.len() - 2..text.len())
            .unwrap();
        assert!(before.after.is_empty(), "{separator:?}");
        assert!(after.before.is_empty(), "{separator:?}");
    }
}

#[test]
fn malformed_context_and_ranges_are_rejected_without_truncating_authority() {
    for before in ["abcdef".into(), "😀".repeat(6), "\n".into()] {
        let context = ShapingContext {
            before,
            after: String::new(),
            ..Default::default()
        };
        assert!(context.validate().is_err());
        assert!(context.slice("a", 0..1).is_err());
    }
    for range in [1..2, 3..1, 0..99] {
        assert!(ShapingContext::default().slice("😀", range).is_err());
    }
    assert!(serde_json::from_str::<ShapingContext>(r#"{"before":"a","befor":"b"}"#).is_err());
}

#[test]
fn omitted_context_keeps_legacy_bidi_json_and_slices_retain_edges() {
    let legacy = serde_json::json!({"levels":[0,0],"rtl":false});
    let decoded: LineBidi = serde_json::from_value(legacy.clone()).unwrap();
    assert!(decoded.context.is_empty());
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    let text = "0123456789";
    let parent = ParagraphBidi::new(text, ShapeOptions::default())
        .unwrap()
        .line(2..8)
        .unwrap();
    let child = parent.slice(&text[2..8], 2..4, false).unwrap();
    assert_eq!(
        child.context,
        ShapingContext::default().slice(text, 4..6).unwrap()
    );
    assert_eq!(child.levels, vec![0, 0]);
}

#[test]
fn wrapped_arabic_initial_medial_and_final_forms_match_explicit_buffer_context() {
    let text = "ببب";
    let paragraph = ParagraphBidi::new(text, rtl()).unwrap();
    for range in [0..2, 2..4, 4..6] {
        let bidi = paragraph.line(range.clone()).unwrap();
        let actual =
            TextShaper::shape_resolved(font(), &text[range.clone()], &bidi, &Default::default())
                .unwrap();
        same_glyphs(&actual.glyphs, &direct(text, range.clone()));
        assert!(actual
            .glyphs
            .iter()
            .all(|g| (g.cluster as usize) < range.len()));
    }
}

#[test]
fn shape_cache_distinguishes_same_text_with_different_joining_context() {
    let text = "ببب";
    let medial = ParagraphBidi::new(text, rtl()).unwrap().line(2..4).unwrap();
    let isolated = ParagraphBidi::new("ب", rtl()).unwrap().line(0..2).unwrap();
    let first = TextShaper::shape_resolved(font(), "ب", &medial, &Default::default()).unwrap();
    let other = TextShaper::shape_resolved(font(), "ب", &isolated, &Default::default()).unwrap();
    let repeated = TextShaper::shape_resolved(font(), "ب", &medial, &Default::default()).unwrap();
    assert_eq!(first, repeated);
    assert_ne!(first.glyphs[0].glyph_id, other.glyphs[0].glyph_id);
}

#[test]
fn font_itemization_retains_logical_neighbours_without_emitting_them() {
    let text = "ببب";
    let bidi = ParagraphBidi::new(text, rtl())
        .unwrap()
        .line(0..text.len())
        .unwrap();
    let fonts = vec![
        ApprovedFontAsset {
            lookup_name: "a".into(),
            bytes: font().to_vec(),
        },
        ApprovedFontAsset {
            lookup_name: "b".into(),
            bytes: font().to_vec(),
        },
    ];
    let spans = [
        fallback::FontSpan {
            range: [0, 2],
            font_index: 0,
        },
        fallback::FontSpan {
            range: [2, 4],
            font_index: 1,
        },
        fallback::FontSpan {
            range: [4, 6],
            font_index: 0,
        },
    ];
    let runs = fallback::shape_line(text, &bidi, &spans, &fonts, &Default::default()).unwrap();
    let actual = runs
        .iter()
        .flat_map(|r| {
            r.shaped
                .glyphs
                .iter()
                .map(move |g| (g.glyph_id, g.cluster as usize + r.range.start))
        })
        .collect::<Vec<_>>();
    let expected = direct(text, 0..text.len())
        .iter()
        .map(|g| (g.glyph_id, g.cluster as usize))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 3);
}

#[test]
fn sideways_vertical_runs_use_the_same_contextual_forms_as_horizontal_runs() {
    let whole = "ببب";
    let text = &whole[2..4];
    let bidi = ParagraphBidi::new(whole, rtl())
        .unwrap()
        .line(2..4)
        .unwrap();
    let fonts = vec![ApprovedFontAsset {
        lookup_name: "Arabic".into(),
        bytes: font().to_vec(),
    }];
    let spans = [fallback::FontSpan {
        range: [0, text.len()],
        font_index: 0,
    }];
    let vertical =
        crate::fonts::vertical_fonts::shape_line(text, &bidi, &spans, &fonts, &Default::default())
            .unwrap();
    let expected = TextShaper::shape_resolved(font(), text, &bidi, &Default::default()).unwrap();
    let actual = &vertical[0].glyphs;
    assert_eq!(actual.len(), expected.glyphs.len());
    for (a, b) in actual.iter().zip(&expected.glyphs) {
        assert!(a.rotate_clockwise);
        assert_eq!(a.glyph.glyph_id, b.glyph_id);
        assert_eq!(a.glyph.cluster, b.cluster);
        assert_eq!(a.glyph.advance, b.advance);
    }
}

#[test]
fn paragraph_and_settings_entrypoints_share_resolved_shaping_and_direction() {
    let text = "ببب\nببب";
    for settings in [
        OpenTypeSettings::default(),
        OpenTypeSettings {
            language: Some("ar".into()),
            features: vec![],
        },
    ] {
        let shaped =
            TextShaper::shape_with_settings(font(), text, ShapeOptions::default(), &settings)
                .unwrap();
        assert_eq!(shaped.direction, TextDirection::RightToLeft);
        let paragraph = ParagraphBidi::new(text, ShapeOptions::default()).unwrap();
        let mut expected = Vec::new();
        for range in [0..6, 7..13] {
            let line = paragraph.line(range.clone()).unwrap();
            assert!(line.context.is_empty());
            let run =
                TextShaper::shape_resolved(font(), &text[range.clone()], &line, &settings).unwrap();
            expected.extend(run.glyphs.into_iter().map(|mut g| {
                g.cluster += range.start as u32;
                g
            }));
        }
        same_glyphs(&shaped.glyphs, &expected);
    }
}

#[test]
fn emergency_line_measurement_uses_the_same_context_as_final_line_shapes() {
    let text = "ب".repeat(16);
    let prepared = crate::fonts::line_layout::PreparedParagraph::new(&text, rtl()).unwrap();
    let lines = prepared
        .break_lines(font(), 0, 12.0, 18.0, &Default::default(), 100)
        .unwrap();
    assert!(lines.len() > 1);
    assert_eq!(
        lines
            .iter()
            .map(|l| &text[l.bytes.clone()])
            .collect::<String>(),
        text
    );
    for line in lines {
        let bidi = prepared.bidi.line(line.bytes.clone()).unwrap();
        let run = TextShaper::shape_resolved(
            font(),
            &text[line.bytes.clone()],
            &bidi,
            &Default::default(),
        )
        .unwrap();
        same_glyphs(&run.glyphs, &direct(&text, line.bytes));
        let metric = crate::fonts::line_layout::measure_run(font(), &run, 12.0).unwrap();
        assert!((metric.width() - line.width).abs() < 1e-7);
    }
}

#[test]
fn malformed_context_is_checked_on_all_empty_and_cached_shaping_paths() {
    let mut bidi = ParagraphBidi::new("ب", rtl()).unwrap().line(0..2).unwrap();
    TextShaper::shape_resolved(font(), "ب", &bidi, &Default::default()).unwrap();
    bidi.context.before = "abcdef".into();
    assert!(TextShaper::shape_resolved(font(), "ب", &bidi, &Default::default()).is_err());
    assert!(
        crate::fonts::vertical::shape_resolved(font(), "ب", &bidi, &Default::default()).is_err()
    );
    bidi.levels.clear();
    assert!(fallback::shape_line("", &bidi, &[], &[], &Default::default()).is_err());
    assert!(
        crate::fonts::vertical_fonts::shape_line("", &bidi, &[], &[], &Default::default()).is_err()
    );
}

#[test]
fn retained_bidi_finds_later_hard_paragraphs_without_cross_boundary_context() {
    let text = "ببب\n".repeat(1024);
    let prepared = ParagraphBidi::new(&text, rtl()).unwrap();
    for index in 0..1024 {
        let start = index * 7;
        let line = prepared.line(start..start + 6).unwrap();
        assert!(line.rtl);
        assert!(line.context.is_empty());
        assert_eq!(line.levels.len(), 6);
    }
    assert!(prepared.line(4..9).is_err());
}

#[test]
fn styled_font_itemization_orders_bidi_runs_once_across_style_boundaries() {
    let text = "A\u{05d0}B";
    let bidi = ParagraphBidi::new(text, ShapeOptions::default())
        .unwrap()
        .line(0..text.len())
        .unwrap();
    let fonts = vec![ApprovedFontAsset {
        lookup_name: "Symbol".into(),
        bytes: font().to_vec(),
    }];
    let spans = vec![
        fallback::StyledFontSpan {
            range: [0, 1],
            font_index: 0,
            style_index: 0,
        },
        fallback::StyledFontSpan {
            range: [1, 3],
            font_index: 0,
            style_index: 1,
        },
        fallback::StyledFontSpan {
            range: [3, 4],
            font_index: 0,
            style_index: 0,
        },
    ];
    let runs = fallback::shape_styled_line(
        text,
        &bidi,
        &spans,
        &fonts,
        &[OpenTypeSettings::default(), OpenTypeSettings::default()],
    )
    .unwrap();
    let logical = [(0..1, 0usize), (1..3, 1usize), (3..4, 0usize)];
    let levels = logical
        .iter()
        .map(|(range, _)| unicode_bidi::Level::new(bidi.levels[range.start]).unwrap())
        .collect::<Vec<_>>();
    let order = unicode_bidi::BidiInfo::reorder_visual(&levels);
    assert_eq!(
        runs.iter()
            .map(|run| (run.range.clone(), run.style_index))
            .collect::<Vec<_>>(),
        order
            .into_iter()
            .map(|index| logical[index].clone())
            .collect::<Vec<_>>()
    );
    let mut invalid = spans.clone();
    invalid[1].range[0] = 2;
    assert!(fallback::shape_styled_line(
        text,
        &bidi,
        &invalid,
        &fonts,
        &[OpenTypeSettings::default(), OpenTypeSettings::default()],
    )
    .is_err());
}

#[test]
fn font_and_style_partitions_intersect_without_losing_either_boundary() {
    let fonts = vec![
        fallback::FontSpan {
            range: [0, 2],
            font_index: 3,
        },
        fallback::FontSpan {
            range: [2, 4],
            font_index: 7,
        },
    ];
    let styles = [[0, 1], [1, 3], [3, 4]];
    let combined = fallback::intersect_styled_spans(&fonts, &styles, 4).unwrap();
    assert_eq!(
        combined
            .iter()
            .map(|span| (span.range, span.font_index, span.style_index))
            .collect::<Vec<_>>(),
        vec![
            ([0, 1], 3, 0),
            ([1, 2], 3, 1),
            ([2, 3], 7, 1),
            ([3, 4], 7, 2),
        ]
    );
    assert!(fallback::intersect_styled_spans(&fonts, &[[0, 1], [2, 4]], 4).is_err());
}

#[test]
fn cancelled_contextual_cache_hits_do_not_bypass_cancellation() {
    let bidi = ParagraphBidi::new("ببب", rtl())
        .unwrap()
        .line(2..4)
        .unwrap();
    TextShaper::shape_resolved(font(), "ب", &bidi, &Default::default()).unwrap();
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| TextShaper::shape_resolved(font(), "ب", &bidi, &Default::default()))
        .is_err());
}
