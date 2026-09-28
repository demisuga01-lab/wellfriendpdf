//! Unexecuted algorithm specifications; synthetic widths are not renderer proof.
use super::*;
use crate::fonts::{
    line_break_policy::{EmergencyWrap, LineBreakSettings, LineComposition},
    ShapeOptions,
};

fn policy() -> LineBreakSettings {
    LineBreakSettings {
        composition: LineComposition::Balanced,
        ..Default::default()
    }
}
fn prepared(text: &str) -> PreparedParagraph<'_> {
    PreparedParagraph::with_break_settings(text, ShapeOptions::default(), &policy()).unwrap()
}
fn ranges(batch: &LineBreakBatch) -> Vec<Range<usize>> {
    batch.lines.iter().map(|l| l.bytes.clone()).collect()
}

#[test]
fn whole_segment_cost_improves_on_locally_longest_first_line() {
    let text = "aaa bb cc dd";
    let measure = |r: Range<usize>| Ok(r.len() as f64);
    let fast = PreparedParagraph::new(text, ShapeOptions::default())
        .unwrap()
        .break_lines_measured_prefix(0, 10.0, 100, measure)
        .unwrap();
    let balanced = prepared(text)
        .break_lines_measured_prefix(0, 10.0, 100, measure)
        .unwrap();
    assert_eq!(ranges(&fast), vec![0..10, 10..12]);
    assert_eq!(ranges(&balanced), vec![0..7, 7..12]);
    let slack = |batch: &LineBreakBatch| {
        batch
            .lines
            .iter()
            .map(|l| (10.0 - l.width).powi(2))
            .sum::<f64>()
    };
    assert!(slack(&balanced) < slack(&fast));
}

#[test]
fn later_narrower_candidate_is_considered_after_an_overwide_prefix() {
    let p = prepared("a b c d");
    let batch = p
        .break_lines_measured_prefix(0, 5.0, 100, |r| {
            Ok(match (r.start, r.end) {
                (0, 4) => 4.0,
                (4, 7) => 3.0,
                _ => 20.0,
            })
        })
        .unwrap();
    assert_eq!(ranges(&batch), vec![0..4, 4..7]);
    assert!(batch.blocked.is_none());
}

#[test]
fn a_wide_first_grapheme_does_not_preclude_a_narrower_shaped_pair() {
    let batch = prepared("abc")
        .break_lines_measured_prefix(0, 1.0, 100, |r| {
            Ok(match (r.start, r.end) {
                (0, 2) | (2, 3) => 1.0,
                _ => 10.0,
            })
        })
        .unwrap();
    assert_eq!(ranges(&batch), vec![0..2, 2..3]);
    assert!(batch.blocked.is_none());
}

#[test]
fn natural_breaks_win_over_fewer_lines_that_split_words() {
    let batch = prepared("aaa bbb ccc")
        .break_lines_measured_prefix(0, 6.0, 100, |r| Ok(r.len() as f64))
        .unwrap();
    assert_eq!(ranges(&batch), vec![0..4, 4..8, 8..11]);
    assert!(batch.blocked.is_none());
}

#[test]
fn emergency_pass_remains_grapheme_safe_and_respects_explicit_nonbreaks() {
    let text = "e\u{301}e\u{301}";
    let batch = prepared(text)
        .break_lines_measured_prefix(0, 3.0, 100, |r| Ok(r.len() as f64))
        .unwrap();
    assert_eq!(ranges(&batch), vec![0..3, 3..6]);
    let p = prepared("a\u{2060}b");
    let blocked = p
        .break_lines_measured_prefix(0, 1.0, 100, |r| Ok(r.len() as f64))
        .unwrap();
    assert!(blocked.lines.is_empty());
    assert_eq!(blocked.blocked.unwrap().byte_start, 0);
}

#[test]
fn mandatory_breaks_are_not_crossed_even_when_a_combined_measurement_would_fit() {
    let p = prepared("ab\ncd");
    let batch = p
        .break_lines_measured_prefix(0, 100.0, 100, |r| {
            assert!(r.end <= 3 || r.start >= 3);
            Ok(0.0)
        })
        .unwrap();
    assert_eq!(ranges(&batch), vec![0..3, 3..5]);
}

#[test]
fn geometric_prefix_can_resume_in_a_wider_frame_without_text_loss() {
    let policy = LineBreakSettings {
        emergency: EmergencyWrap::PreserveWords,
        ..policy()
    };
    let text = "aa bbbbb cc";
    let p = PreparedParagraph::with_break_settings(text, ShapeOptions::default(), &policy).unwrap();
    let first = p
        .break_lines_measured_prefix(0, 3.0, 100, |r| Ok(r.len() as f64))
        .unwrap();
    assert_eq!(ranges(&first), vec![0..3]);
    let blocked = first.blocked.unwrap();
    assert_eq!(blocked.byte_start, 3);
    let rest = p
        .break_lines_measured_prefix(blocked.byte_start, 6.0, 100, |r| Ok(r.len() as f64))
        .unwrap();
    assert!(rest.blocked.is_none());
    assert_eq!(
        first
            .lines
            .iter()
            .chain(&rest.lines)
            .map(|l| &text[l.bytes.clone()])
            .collect::<String>(),
        text
    );
}

#[test]
fn lookahead_is_a_prefix_of_the_whole_plan_not_a_fake_paragraph_end() {
    let p = prepared("aaa bb cc dd");
    let all = p
        .break_lines_measured_prefix(0, 10.0, 100, |r| Ok(r.len() as f64))
        .unwrap();
    let one = p
        .break_lines_measured_prefix(0, 10.0, 1, |r| Ok(r.len() as f64))
        .unwrap();
    assert_eq!(ranges(&one), vec![all.lines[0].bytes.clone()]);
    assert!(one.blocked.is_none());
    let limited = PreparedParagraph::with_break_settings(
        "aa bbbbb",
        ShapeOptions::default(),
        &LineBreakSettings {
            emergency: EmergencyWrap::PreserveWords,
            ..policy()
        },
    )
    .unwrap()
    .break_lines_measured_prefix(0, 3.0, 1, |r| Ok(r.len() as f64))
    .unwrap();
    assert!(limited.blocked.is_none());
    assert_eq!(ranges(&limited), vec![0..3]);
}

#[test]
fn malformed_measurements_and_provider_errors_are_not_geometric_overflow() {
    for value in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(prepared("ab cd")
            .break_lines_measured_prefix(0, 5.0, 10, |_| Ok(value))
            .is_err());
    }
    assert!(matches!(
        prepared("ab cd").break_lines_measured_prefix(0, 5.0, 10, |_| Err(
            WellfriendError::UnsupportedFeature("font error".into())
        )),
        Err(WellfriendError::UnsupportedFeature(_))
    ));
}

#[test]
fn candidate_and_measurement_budgets_error_instead_of_returning_greedy_output() {
    let text = "a ".repeat(MAX_CANDIDATES);
    assert!(matches!(
        prepared(&text).break_lines_measured_prefix(0, 5.0, 1, |_| Ok(10.0)),
        Err(WellfriendError::ResourceLimit(_))
    ));
    let mut calls = Budget {
        measurements: MAX_MEASUREMENTS,
        bytes: 0,
        ..Default::default()
    };
    assert!(matches!(
        calls.charge(1),
        Err(WellfriendError::ResourceLimit(_))
    ));
    let mut bytes = Budget {
        measurements: 0,
        bytes: MAX_MEASURED_BYTES,
        ..Default::default()
    };
    assert!(matches!(
        bytes.charge(1),
        Err(WellfriendError::ResourceLimit(_))
    ));
    let text = "a ".repeat(600);
    assert!(matches!(
        prepared(&text).break_lines_measured_prefix(0, 5.0, 1, |r| Ok(r.len() as f64)),
        Err(WellfriendError::ResourceLimit(_))
    ));
}

#[test]
fn cancellation_is_observed_before_and_during_candidate_measurement() {
    let p = prepared("ab cd ef");
    let token = crate::CancelToken::new();
    token.cancel();
    assert!(token
        .scope(|| p.break_lines_measured_prefix(0, 4.0, 10, |_| Ok(1.0)))
        .is_err());
    let token = crate::CancelToken::new();
    assert!(token
        .scope(|| p.break_lines_measured_prefix(0, 4.0, 10, |_| {
            token.cancel();
            Ok(1.0)
        }))
        .is_err());
}

#[test]
fn default_policy_serialization_and_unknown_composition_rejection_are_explicit() {
    let old: LineBreakSettings = serde_json::from_str(r#"{"profile":"unicode"}"#).unwrap();
    assert!(old.is_default());
    assert!(serde_json::to_value(old)
        .unwrap()
        .get("composition")
        .is_none());
    assert_eq!(
        serde_json::to_value(policy()).unwrap()["composition"],
        "balanced"
    );
    assert!(serde_json::from_str::<LineBreakSettings>(r#"{"composition":"automatic"}"#).is_err());
}

#[test]
fn candidate_metrics_are_reused_only_within_the_same_composition_call() {
    use std::{cell::RefCell, collections::BTreeSet};
    let p = prepared("abcdefgh");
    for limit in [2.0, 3.0] {
        let seen = RefCell::new(BTreeSet::new());
        let batch = p
            .break_lines_measured_prefix(0, limit, 100, |r| {
                assert!(
                    seen.borrow_mut().insert((r.start, r.end)),
                    "metric was queried twice"
                );
                Ok(r.len() as f64)
            })
            .unwrap();
        assert!(batch.blocked.is_none());
        assert!(batch.lines.iter().all(|line| line.width <= limit));
    }
}

#[test]
fn custom_prohibitions_apply_to_both_composition_passes() {
    let policy = LineBreakSettings {
        prohibit_start: "b".into(),
        ..policy()
    };
    let p = PreparedParagraph::with_break_settings("ab", ShapeOptions::default(), &policy).unwrap();
    let batch = p
        .break_lines_measured_prefix(0, 1.0, 100, |r| Ok(r.len() as f64))
        .unwrap();
    assert!(batch.lines.is_empty());
    assert!(batch.blocked.is_some());
    assert!(p
        .break_lines_measured_prefix(1, 1.0, 100, |_| Ok(1.0))
        .unwrap()
        .blocked
        .is_none());
    assert!(prepared("e\u{301}")
        .break_lines_measured_prefix(1, 5.0, 100, |_| Ok(1.0))
        .is_err());
}

#[test]
fn tiny_nonmonotonic_dags_match_exhaustively_enumerated_break_paths() {
    // The oracle enumerates partitions; it does not reuse the DP recurrence.
    for count in 2..=8 {
        let text = "a".repeat(count);
        for width in 1..=4usize {
            let advance = |start: usize, end: usize| (start * 7 + end * 11) % 7;
            let mut expected = None::<(usize, usize, usize)>;
            for mask in 0..(1usize << (count - 1)) {
                let mut ends = (1..count)
                    .filter(|n| mask & (1 << (n - 1)) != 0)
                    .collect::<Vec<_>>();
                ends.push(count);
                let mut start = 0;
                let mut cost = 0;
                let mut fits = true;
                for end in &ends {
                    let m = advance(start, *end);
                    if m > width {
                        fits = false;
                        break;
                    }
                    cost += (width - m).pow(2);
                    start = *end;
                }
                let candidate = (ends.len() - 1, ends.len(), cost);
                if fits && expected.is_none_or(|old| candidate < old) {
                    expected = Some(candidate);
                }
            }
            let batch = prepared(&text)
                .break_lines_measured_prefix(0, width as f64, 100, |r| {
                    Ok(advance(r.start, r.end) as f64)
                })
                .unwrap();
            if let Some(expected) = expected {
                assert!(batch.blocked.is_none());
                let observed = (
                    batch.lines.len() - 1,
                    batch.lines.len(),
                    batch
                        .lines
                        .iter()
                        .map(|l| (width - l.width as usize).pow(2))
                        .sum(),
                );
                assert_eq!(observed, expected);
            } else {
                assert!(batch.blocked.is_some());
            }
        }
    }
}
