//! Unexecuted hard-line specifications. No independent glyph/pixel evidence.
use super::*;
use crate::fonts::{
    line_break_policy::{LineBreakSettings, LineComposition},
    line_layout::PreparedParagraph,
    shaper::{LineBidi, OpenTypeSettings, ParagraphBidi},
    ShapeOptions, TextDirection, TextShaper,
};

fn font() -> &'static [u8] {
    crate::render::get_fallback_font("Symbol").unwrap()
}
fn separators() -> [&'static str; 8] {
    [
        "\r", "\n", "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
    ]
}

#[test]
fn forced_break_predicate_does_not_treat_spaces_or_formatting_controls_as_breaks() {
    for s in separators() {
        assert!(s.chars().all(is_hard_break));
    }
    for c in [
        ' ', '\t', '\u{a0}', '\u{202f}', '\u{2060}', '\u{200d}', '\u{2067}', 'A',
    ] {
        assert!(!is_hard_break(c), "{c:?}");
    }
}

#[test]
fn logical_ranges_retain_exact_utf8_and_each_visible_range_omits_only_its_separator() {
    for separator in separators() {
        let source = format!("é{separator}ب");
        let lines = logical_lines(&source).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].logical, 0..2 + separator.len());
        assert_eq!(lines[0].visible, 0..2);
        assert_eq!(
            lines
                .iter()
                .map(|l| &source[l.logical.clone()])
                .collect::<String>(),
            source
        );
        assert_eq!(
            lines
                .iter()
                .map(|l| &source[l.visible.clone()])
                .collect::<Vec<_>>(),
            vec!["é", "ب"]
        );
    }
}

#[test]
fn crlf_and_consecutive_breaks_do_not_invent_or_erase_empty_lines() {
    let source = "A\r\n\r\n\u{000b}B\u{000c}";
    let lines = logical_lines(source).collect::<Result<Vec<_>>>().unwrap();
    assert_eq!(
        lines
            .iter()
            .map(|l| &source[l.visible.clone()])
            .collect::<Vec<_>>(),
        vec!["A", "", "", "B"]
    );
    assert_eq!(
        lines
            .iter()
            .map(|l| &source[l.logical.clone()])
            .collect::<String>(),
        source
    );
    assert!(logical_lines("").next().is_none());
    let prepared = ParagraphBidi::new("A\r\nB", ShapeOptions::default()).unwrap();
    let mut visible = Vec::new();
    prepared
        .all_hard_lines(|r, _| {
            visible.push(r);
            Ok(true)
        })
        .unwrap();
    assert_eq!(visible, vec![0..1, 3..4]);
}

#[test]
fn trimming_preserves_significant_spaces_and_controls_instead_of_generic_whitespace() {
    for separator in separators() {
        let text = format!("A \t\u{a0}{separator}");
        assert_eq!(text.trim_end_matches(is_hard_break), "A \t\u{a0}");
    }
    assert_eq!("A\nB".trim_end_matches(is_hard_break), "A\nB");
}

#[test]
fn paragraph_shaping_omits_separator_glyphs_and_keeps_original_cluster_offsets() {
    for separator in separators() {
        let text = format!("A{separator}B");
        for settings in [
            OpenTypeSettings::default(),
            OpenTypeSettings {
                language: Some("en".into()),
                features: vec![],
            },
        ] {
            let run =
                TextShaper::shape_with_settings(font(), &text, ShapeOptions::default(), &settings)
                    .unwrap();
            assert_eq!(run.glyphs.len(), 2, "{separator:?}");
            assert_eq!(
                run.glyphs
                    .iter()
                    .map(|g| g.cluster as usize)
                    .collect::<Vec<_>>(),
                vec![0, 1 + separator.len()]
            );
            assert!(!crate::fonts::shaper::has_missing_glyphs(font(), &text, &run).unwrap());
            assert!(run.glyphs.iter().all(|g| g.glyph_id != 0));
        }
    }
}

#[test]
fn line_separator_preserves_parent_bidi_isolates_instead_of_restarting_analysis() {
    let text = "English \u{2067}אבג\u{2028}123\u{2069} end";
    let prepared = ParagraphBidi::new(text, ShapeOptions::default()).unwrap();
    let mut lines = Vec::new();
    prepared
        .all_hard_lines(|range, bidi| {
            lines.push((range, bidi));
            Ok(true)
        })
        .unwrap();
    assert_eq!(lines.len(), 2);
    let (range, bidi) = &lines[1];
    assert!(text[range.clone()].starts_with("123"));
    assert!(bidi.levels[..3].iter().all(|n| *n == 2));
    let isolated = ParagraphBidi::new(&text[range.clone()], ShapeOptions::default())
        .unwrap()
        .line(0..range.len())
        .unwrap();
    assert_ne!(&bidi.levels[..3], &isolated.levels[..3]);
}

#[test]
fn hard_line_boundaries_stop_joining_even_inside_the_same_bidi_paragraph() {
    for separator in ["\u{000b}", "\u{000c}", "\u{2028}"] {
        let text = format!("ب{separator}ب");
        let prepared = ParagraphBidi::new(
            &text,
            ShapeOptions {
                direction: Some(TextDirection::RightToLeft),
            },
        )
        .unwrap();
        let mut count = 0;
        prepared
            .all_hard_lines(|range, bidi| {
                assert!(bidi.context.is_empty());
                let shaped =
                    TextShaper::shape_resolved(font(), &text[range], &bidi, &Default::default())
                        .unwrap();
                let isolated = TextShaper::shape(font(), "ب", ShapeOptions::default()).unwrap();
                assert_eq!(shaped.glyphs[0].glyph_id, isolated.glyphs[0].glyph_id);
                count += 1;
                Ok(true)
            })
            .unwrap();
        assert_eq!(count, 2);
    }
}

#[test]
fn already_broken_horizontal_and_vertical_apis_reject_embedded_hard_separators() {
    for separator in separators() {
        let text = format!("A{separator}B");
        let bidi = LineBidi {
            levels: vec![0; text.len()],
            rtl: false,
            context: Default::default(),
        };
        assert!(TextShaper::shape_resolved(font(), &text, &bidi, &Default::default()).is_err());
        assert!(
            crate::fonts::vertical::shape_resolved(font(), &text, &bidi, &Default::default())
                .is_err()
        );
    }
}

#[test]
fn vertical_coverage_splits_hard_lines_without_requiring_control_glyphs() {
    for separator in separators() {
        assert!(crate::fonts::vertical_fonts::covers(
            font(),
            &format!("A{separator}B"),
            ShapeOptions::default(),
            &Default::default()
        )
        .unwrap());
    }
    assert!(crate::fonts::vertical_fonts::covers(
        &[],
        "",
        ShapeOptions::default(),
        &Default::default()
    )
    .is_err());
    assert!(crate::fonts::vertical_fonts::covers(
        font(),
        "",
        ShapeOptions::default(),
        &Default::default()
    )
    .unwrap());
}

#[test]
fn greedy_and_balanced_measurement_use_the_same_hard_break_policy() {
    for composition in [LineComposition::Greedy, LineComposition::Balanced] {
        for separator in separators() {
            let text = format!("A{separator}B");
            let policy = LineBreakSettings {
                composition,
                ..Default::default()
            };
            let prepared =
                PreparedParagraph::with_break_settings(&text, ShapeOptions::default(), &policy)
                    .unwrap();
            let lines = prepared
                .break_lines(font(), 0, 12.0, 100.0, &Default::default(), 10)
                .unwrap();
            assert_eq!(lines.len(), 2);
            assert_eq!(
                lines
                    .iter()
                    .map(|l| &text[l.bytes.clone()])
                    .collect::<String>(),
                text
            );
            assert!(lines.iter().all(|l| l.width > 0.0 && l.width < 20.0));
        }
    }
}

#[test]
fn scanning_and_coverage_visitors_respect_cancellation_and_early_exit() {
    let cancel = crate::CancelToken::new();
    let text = "a".repeat(4096);
    let mut iter = logical_lines(&text);
    cancel.cancel();
    assert!(cancel.scope(|| iter.next().unwrap()).is_err());
    assert!(iter.next().is_none());
    let prepared = ParagraphBidi::new("A\nB", ShapeOptions::default()).unwrap();
    let mut visits = 0;
    assert!(!prepared
        .all_hard_lines(|_, _| {
            visits += 1;
            Ok(false)
        })
        .unwrap());
    assert_eq!(visits, 1);
    assert!(cancel
        .scope(|| prepared.all_hard_lines(|_, _| Ok(true)))
        .is_err());
}
