//! Unexecuted specifications: synthetic advances test break logic, not glyph pixels.
use super::*;
use crate::fonts::line_layout::PreparedParagraph;
use crate::fonts::ShapeOptions;
use unicode_segmentation::UnicodeSegmentation;

fn lines(text: &str, width: f64, policy: &LineBreakSettings) -> Result<Vec<String>> {
    let prepared = PreparedParagraph::with_break_settings(text, ShapeOptions::default(), policy)?;
    let measured = prepared.break_lines_measured(0, width, 100_000, |range| {
        Ok(text[range]
            .graphemes(true)
            .filter(|g| {
                !g.chars().all(|c| {
                    matches!(
                        c,
                        '\r' | '\n'
                            | '\u{000b}'
                            | '\u{000c}'
                            | '\u{0085}'
                            | '\u{2028}'
                            | '\u{2029}'
                            | '\u{2060}'
                            | '\u{feff}'
                    )
                })
            })
            .count() as f64)
    })?;
    Ok(measured
        .into_iter()
        .map(|line| text[line.bytes].to_owned())
        .collect())
}

#[test]
fn default_wrap_prefers_words_then_uses_grapheme_safe_emergency_letters() {
    assert_eq!(
        lines("AA BB CC", 3.0, &Default::default()).unwrap(),
        ["AA ", "BB ", "CC"]
    );
    assert_eq!(
        lines("abcdef", 2.0, &Default::default()).unwrap(),
        ["ab", "cd", "ef"]
    );
    assert_eq!(
        lines("123456", 2.0, &Default::default()).unwrap(),
        ["12", "34", "56"]
    );
}

#[test]
fn preserve_words_reports_overflow_instead_of_splitting() {
    let policy = LineBreakSettings {
        emergency: EmergencyWrap::PreserveWords,
        ..Default::default()
    };
    assert!(lines("abcdef", 2.0, &policy).is_err());
    assert_eq!(lines("ab cd", 3.0, &policy).unwrap(), ["ab ", "cd"]);
}

#[test]
fn emergency_wrapping_does_not_erase_explicit_nonbreaking_controls() {
    for text in [
        "A\u{2060}B",
        "A\u{feff}B",
        "A\u{a0}B",
        "A\u{202f}B",
        "A\u{2011}B",
        "A\u{200d}B",
    ] {
        assert!(lines(text, 1.0, &Default::default()).is_err(), "{text:?}");
        assert_eq!(
            lines(text, 4.0, &Default::default()).unwrap().concat(),
            text
        );
    }
}

#[test]
fn combining_sequences_and_emoji_are_not_split_by_emergency_wrapping() {
    for text in ["e\u{301}e\u{301}", "👩‍👩‍👧‍👦👩‍👩‍👧‍👦", "🇮🇳🇮🇳"]
    {
        let output = lines(text, 1.0, &Default::default()).unwrap();
        assert_eq!(output.concat(), text);
        assert!(output.iter().all(|line| line.graphemes(true).count() == 1));
        assert!(lines(text, 0.5, &Default::default()).is_err());
    }
}

#[test]
fn japanese_strict_keeps_punctuation_groups_and_wave_dash_off_line_start() {
    let policy = LineBreakSettings {
        profile: LineBreakProfile::JapaneseStrict,
        ..Default::default()
    };
    assert_eq!(
        lines("漢字〜漢", 2.0, &policy).unwrap(),
        ["漢", "字〜", "漢"]
    );
    assert_eq!(
        lines("漢字（漢）漢", 3.0, &policy).unwrap(),
        ["漢字", "（漢）", "漢"]
    );
    let output = lines("漢（  字）漢", 5.0, &policy).unwrap();
    assert_eq!(output.concat(), "漢（  字）漢");
    assert!(output
        .iter()
        .all(|line| !line.trim_end().ends_with('（') && !line.starts_with('）')));
}

#[test]
fn custom_start_end_rules_are_applied_to_normal_and_emergency_opportunities() {
    let no_start = LineBreakSettings {
        prohibit_start: "b".into(),
        ..Default::default()
    };
    let no_end = LineBreakSettings {
        prohibit_end: "a".into(),
        ..Default::default()
    };
    assert!(lines("ab", 1.0, &no_start).is_err());
    assert!(lines("ab", 1.0, &no_end).is_err());
    assert!(lines("a   b", 3.0, &no_end).is_err());
    assert!(lines("a   b", 3.0, &no_start).is_err());
    assert_eq!(lines("ab", 2.0, &no_start).unwrap(), ["ab"]);
    assert_eq!(lines("a", 1.0, &no_end).unwrap(), ["a"]); // mandatory end
    let no_mark_end = LineBreakSettings {
        prohibit_end: "\u{301}".into(),
        ..Default::default()
    };
    let no_mark_start = LineBreakSettings {
        prohibit_start: "\u{301}".into(),
        ..Default::default()
    };
    assert!(lines("a\u{301}b", 1.0, &no_mark_end).is_err());
    assert!(lines("ab\u{301}", 1.0, &no_mark_start).is_err());
}

#[test]
fn explicit_line_breaks_take_precedence_over_tailoring_without_losing_text() {
    let policy = LineBreakSettings {
        prohibit_start: "A".into(),
        prohibit_end: "A".into(),
        ..Default::default()
    };
    for separator in [
        "\n", "\r\n", "\r", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
    ] {
        let text = format!("A{separator}A");
        let output = lines(&text, 1.0, &policy).unwrap();
        assert_eq!(output, [format!("A{separator}"), "A".into()]);
        assert_eq!(output.concat(), text);
    }
}

#[test]
fn retained_indexes_support_continuation_and_bounded_line_lookahead() {
    let text = "abcde abcde abcde";
    let prepared =
        PreparedParagraph::with_break_settings(text, ShapeOptions::default(), &Default::default())
            .unwrap();
    let measure = |range: std::ops::Range<usize>| Ok(range.len() as f64);
    let first = prepared.break_lines_measured(0, 3.0, 1, measure).unwrap();
    assert_eq!(&text[first[0].bytes.clone()], "abc");
    let next = prepared
        .break_lines_measured(first[0].bytes.end, 6.0, 100, measure)
        .unwrap();
    let joined = first
        .into_iter()
        .chain(next)
        .map(|line| &text[line.bytes])
        .collect::<String>();
    assert_eq!(joined, text);
}

#[test]
fn preserved_space_runs_have_bounded_emergency_progress() {
    let text = " ".repeat(10_000);
    let output = lines(&text, 100.0, &Default::default()).unwrap();
    assert_eq!(output.len(), 100);
    assert_eq!(output.concat(), text);
}

#[test]
fn invalid_policy_and_nonfinite_measurements_never_form_successful_lines() {
    for value in [" ".into(), "\n".into(), "a".repeat(4097)] {
        let policy = LineBreakSettings {
            prohibit_start: value,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
        assert!(
            PreparedParagraph::with_break_settings("abc", ShapeOptions::default(), &policy)
                .is_err()
        );
    }
    let prepared = PreparedParagraph::new("abc", ShapeOptions::default()).unwrap();
    for value in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(prepared
            .break_lines_measured(0, 10.0, 100, |_| Ok(value))
            .is_err());
    }
    assert!(serde_json::from_str::<LineBreakSettings>(r#"{"profile":"made_up"}"#).is_err());
    assert!(serde_json::from_str::<LineBreakSettings>(r#"{"prohibit_strat":"x"}"#).is_err());
}

#[test]
fn geometric_block_retains_a_prefix_for_a_wider_frame_without_changing_text() {
    use crate::fonts::line_layout::LineBlockReason;
    let text = "aa bbbbb cc";
    let policy = LineBreakSettings {
        emergency: EmergencyWrap::PreserveWords,
        ..Default::default()
    };
    let prepared =
        PreparedParagraph::with_break_settings(text, ShapeOptions::default(), &policy).unwrap();
    let measure = |range: std::ops::Range<usize>| Ok(range.len() as f64);
    let first = prepared
        .break_lines_measured_prefix(0, 3.0, 100, measure)
        .unwrap();
    assert_eq!(&text[first.lines[0].bytes.clone()], "aa ");
    let blocked = first.blocked.unwrap();
    assert_eq!(blocked.byte_start, 3);
    assert_eq!(blocked.reason, LineBlockReason::ProtectedSequenceTooWide);
    let rest = prepared
        .break_lines_measured_prefix(blocked.byte_start, 6.0, 100, measure)
        .unwrap();
    assert!(rest.blocked.is_none());
    assert_eq!(
        first
            .lines
            .into_iter()
            .chain(rest.lines)
            .map(|line| &text[line.bytes])
            .collect::<String>(),
        text
    );
    assert!(prepared.break_lines_measured(0, 3.0, 100, measure).is_err());
}

#[test]
fn bounded_prefix_distinguishes_lookahead_from_first_grapheme_overflow() {
    use crate::fonts::line_layout::LineBlockReason;
    let prepared = PreparedParagraph::new("AA BB CC", ShapeOptions::default()).unwrap();
    let measure = |range: std::ops::Range<usize>| Ok(range.len() as f64);
    let bounded = prepared
        .break_lines_measured_prefix(0, 3.0, 1, measure)
        .unwrap();
    assert_eq!(bounded.lines.len(), 1);
    assert!(bounded.blocked.is_none());
    assert!(bounded.lines[0].bytes.end < prepared.text.len());
    let blocked = prepared
        .break_lines_measured_prefix(0, 0.5, 10, measure)
        .unwrap();
    assert!(blocked.lines.is_empty());
    assert_eq!(
        blocked.blocked.unwrap().reason,
        LineBlockReason::GraphemeTooWide
    );
}

#[test]
fn prefix_probes_do_not_swallow_shaping_errors_or_cancellation_after_progress() {
    let prepared = PreparedParagraph::new("AA BB CC", ShapeOptions::default()).unwrap();
    let error = prepared
        .break_lines_measured_prefix(0, 3.0, 100, |range| {
            if range.start >= 3 {
                return Err(WellfriendError::UnsupportedFeature("font failure".into()));
            }
            Ok(range.len() as f64)
        })
        .unwrap_err();
    assert!(matches!(error, WellfriendError::UnsupportedFeature(s) if s == "font failure"));
    let cancel = crate::cancel::CancelToken::new();
    assert!(cancel
        .scope(
            || prepared.break_lines_measured_prefix(0, 3.0, 100, |range| {
                if range.start == 0 {
                    cancel.cancel();
                }
                Ok(range.len() as f64)
            })
        )
        .is_err());
}

#[test]
fn cancelled_preparation_and_prepared_continuation_are_rejected() {
    let prepared = PreparedParagraph::new("abcdef", ShapeOptions::default()).unwrap();
    let cancel = crate::cancel::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| PreparedParagraph::new("abcdef", ShapeOptions::default()))
        .is_err());
    assert!(cancel
        .scope(|| prepared.break_lines_measured(0, 2.0, 1, |_| Ok(1.0)))
        .is_err());
}
