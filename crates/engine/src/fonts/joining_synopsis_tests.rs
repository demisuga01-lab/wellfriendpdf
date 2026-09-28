//! Unexecuted synopsis specifications, using the pinned shaping backend rather
//! than independent raster/typography evidence.
use super::*;
use crate::fonts::{
    shaper::{LineBidi, ParagraphBidi},
    ShapeOptions, TextDirection, TextShaper,
};

fn font() -> &'static [u8] {
    crate::render::get_fallback_font("Symbol").unwrap()
}
fn rtl() -> ShapeOptions {
    ShapeOptions {
        direction: Some(TextDirection::RightToLeft),
    }
}
fn line(text: &str, range: Range<usize>) -> LineBidi {
    ParagraphBidi::new(text, rtl())
        .unwrap()
        .line(range)
        .unwrap()
}
fn gids(text: &str, bidi: &LineBidi) -> Vec<u16> {
    TextShaper::shape_resolved(font(), text, bidi, &Default::default())
        .unwrap()
        .glyphs
        .into_iter()
        .map(|g| g.glyph_id)
        .collect()
}
fn marks() -> String {
    "\u{064e}".repeat(128)
}

#[test]
fn transparency_matches_explicit_joining_overrides_and_general_categories() {
    for c in [
        '\u{064e}', '\u{070f}', '\u{061c}', '\u{0301}', '\u{20dd}', '\u{fe0f}',
    ] {
        assert!(properties::transparent(c), "{c:?}");
    }
    for c in [
        'ب', 'A', ' ', '\n', '\u{0600}', '\u{200c}', '\u{200d}', '\u{2066}',
    ] {
        assert!(!properties::transparent(c), "{c:?}");
    }
}

#[test]
fn indexed_long_transparent_edges_retain_only_nearest_joining_scalars() {
    let text = format!("ب{}ب{}ب", marks(), marks());
    let start = 2 + marks().len();
    let context = JoiningIndex::new(&text)
        .unwrap()
        .context(start..start + 2)
        .unwrap();
    assert_eq!(context.before, "\u{064e}".repeat(5));
    assert_eq!(context.after, "\u{064e}".repeat(5));
    assert_eq!(context.joining_before, Some('ب'));
    assert_eq!(context.joining_after, Some('ب'));
    context.validate().unwrap();
    assert!(serde_json::to_vec(&context).unwrap().len() < 256);
}

#[test]
fn indexed_context_and_cancellable_scan_agree_at_all_scalar_boundaries() {
    let text = format!("Aب{}\u{200c}{}ب\n{}ب", marks(), marks(), marks());
    let index = JoiningIndex::new(&text).unwrap();
    for (start, c) in text.char_indices() {
        let range = start..start + c.len_utf8();
        assert_eq!(
            index.context(range.clone()).unwrap(),
            ShapingContext::default().slice(&text, range).unwrap()
        );
    }
}

#[test]
fn nested_font_run_slices_keep_a_distant_joining_scalar_without_rescanning_parent() {
    let text = format!("ب{}ب{}ب", marks(), marks());
    let range = 32..text.len() - 32;
    let parent = JoiningIndex::new(&text)
        .unwrap()
        .context(range.clone())
        .unwrap();
    let child_range = 2 + marks().len() - range.start..4 + marks().len() - range.start;
    let actual = parent
        .slice(&text[range.clone()], child_range.clone())
        .unwrap();
    let expected = JoiningIndex::new(&text)
        .unwrap()
        .context(range.start + child_range.start..range.start + child_range.end)
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn forced_breaks_terminate_synopsis_search_even_beyond_the_raw_window() {
    for separator in ["\n", "\r\n", "\u{000b}", "\u{0085}", "\u{2028}", "\u{2029}"] {
        let text = format!("ب{separator}{}ب{}{separator}ب", marks(), marks());
        let start = 2 + separator.len() + marks().len();
        let context = JoiningIndex::new(&text)
            .unwrap()
            .context(start..start + 2)
            .unwrap();
        assert!(context.joining_before.is_none());
        assert!(context.joining_after.is_none());
    }
}

#[test]
fn nonjoining_and_join_causing_format_characters_are_not_skipped_as_transparent() {
    for boundary in ['\u{200c}', '\u{200d}', '\u{0600}'] {
        let text = format!("ب{boundary}{}ب", marks());
        let start = text.len() - 2;
        let actual = line(&text, start..text.len());
        assert_eq!(actual.context.joining_before, Some(boundary));
        let reference = format!("{boundary}ب");
        let expected = line(&reference, boundary.len_utf8()..reference.len());
        assert_eq!(gids("ب", &actual), gids("ب", &expected));
    }
}

#[test]
fn long_mark_context_preserves_medial_forms_instead_of_isolated_forms() {
    let text = format!("ب{}ب{}ب", marks(), marks());
    let start = 2 + marks().len();
    let actual = line(&text, start..start + 2);
    let expected = line("ببب", 2..4);
    assert_eq!(gids("ب", &actual), gids("ب", &expected));
    assert_ne!(gids("ب", &actual), gids("ب", &line("ب", 0..2)));
    let shaped = TextShaper::shape_resolved(font(), "ب", &actual, &Default::default()).unwrap();
    assert_eq!(shaped.glyphs.len(), 1);
    assert_eq!(shaped.glyphs[0].cluster, 0);
}

#[test]
fn identical_raw_edges_with_different_synopses_do_not_alias_the_shape_cache() {
    let joined = format!("ب{}ب", marks());
    let separate = format!("\u{200c}{}ب", marks());
    let a = line(&joined, joined.len() - 2..joined.len());
    let b = line(&separate, separate.len() - 2..separate.len());
    assert_eq!(a.context.before, b.context.before);
    assert_ne!(a.context.joining_before, b.context.joining_before);
    let first = gids("ب", &a);
    let other = gids("ب", &b);
    let again = gids("ب", &a);
    assert_ne!(first, other);
    assert_eq!(first, again);
}

#[test]
fn invalid_synopsis_cannot_override_raw_context_or_bridge_a_hard_break() {
    for (before, joining_before) in [
        (String::new(), Some('ب')),
        ("a".repeat(5), Some('ب')),
        ("\u{064e}".repeat(4), Some('ب')),
        ("\u{064e}".repeat(5), Some('\u{064e}')),
        ("\u{064e}".repeat(5), Some('\n')),
    ] {
        let context = ShapingContext {
            before,
            joining_before,
            ..Default::default()
        };
        assert!(context.validate().is_err());
        assert!(context.slice("ب", 0..2).is_err());
    }
}

#[test]
fn synopsis_round_trips_while_legacy_raw_edges_remain_explicitly_without_it() {
    let text = format!("ب{}ب", marks());
    let context = JoiningIndex::new(&text)
        .unwrap()
        .context(text.len() - 2..text.len())
        .unwrap();
    let restored: ShapingContext =
        serde_json::from_slice(&serde_json::to_vec(&context).unwrap()).unwrap();
    assert_eq!(context, restored);
    let old: ShapingContext =
        serde_json::from_value(serde_json::json!({"before":"\u{064e}".repeat(5),"after":""}))
            .unwrap();
    assert!(old.joining_before.is_none());
    old.validate().unwrap();
}

#[test]
fn long_context_scans_and_index_preparation_respect_cancellation_and_budget() {
    let text = format!("ب{}ب", marks());
    let index = JoiningIndex::new(&text).unwrap();
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel.scope(|| JoiningIndex::new(&text)).is_err());
    assert!(cancel
        .scope(|| index.context(text.len() - 2..text.len()))
        .is_err());
    assert!(cancel
        .scope(|| ShapingContext::default().slice(&text, text.len() - 2..text.len()))
        .is_err());
    assert!(JoiningIndex::new(&"x".repeat(4_000_001)).is_err());
}

#[test]
fn reshapeable_latin_ligatures_are_not_a_blanket_refusal_to_wrap() {
    let text = "ffi";
    let single = TextShaper::shape(font(), "f", ShapeOptions::default()).unwrap();
    let width = crate::fonts::line_layout::measure_run(font(), &single, 12.0)
        .unwrap()
        .width()
        + 0.1;
    let lines =
        crate::fonts::line_layout::break_lines(font(), text, 12.0, width, ShapeOptions::default())
            .unwrap();
    assert!(lines.len() > 1);
    assert_eq!(
        lines
            .iter()
            .map(|l| &text[l.bytes.clone()])
            .collect::<String>(),
        text
    );
    assert!(lines.iter().all(|l| l.width <= width + 1e-7));
}

#[test]
fn font_fallback_and_vertical_itemization_retain_the_distant_joining_scalar() {
    use crate::editing_transactions::ApprovedFontAsset;
    use crate::fonts::{fallback, vertical_fonts};
    let fonts = vec![ApprovedFontAsset {
        lookup_name: "Arabic".into(),
        bytes: font().to_vec(),
    }];
    let spans = [fallback::FontSpan {
        range: [0, 2],
        font_index: 0,
    }];
    for neighbour in ['ب', '\u{200c}', '\u{200d}'] {
        let source = format!("{neighbour}{}ب", marks());
        let bidi = line(&source, source.len() - 2..source.len());
        let expected = gids("ب", &bidi);
        let horizontal =
            fallback::shape_line("ب", &bidi, &spans, &fonts, &Default::default()).unwrap();
        assert_eq!(horizontal.len(), 1);
        assert_eq!(
            horizontal[0]
                .shaped
                .glyphs
                .iter()
                .map(|g| g.glyph_id)
                .collect::<Vec<_>>(),
            expected
        );
        let vertical =
            vertical_fonts::shape_line("ب", &bidi, &spans, &fonts, &Default::default()).unwrap();
        assert_eq!(vertical.len(), 1);
        assert_eq!(
            vertical[0]
                .glyphs
                .iter()
                .map(|g| g.glyph.glyph_id)
                .collect::<Vec<_>>(),
            expected
        );
        assert!(vertical[0].glyphs.iter().all(|g| g.rotate_clockwise));
    }
}

#[test]
fn emergency_wrapping_does_not_split_the_marked_grapheme_or_drop_its_synopsis() {
    use crate::fonts::line_layout::PreparedParagraph;
    let text = format!("ب{}ب", marks());
    let prepared = PreparedParagraph::new(&text, rtl()).unwrap();
    let end = text.len() - 2;
    let prefix = TextShaper::shape_resolved(
        font(),
        &text[..end],
        &prepared.bidi.line(0..end).unwrap(),
        &Default::default(),
    )
    .unwrap();
    let suffix = TextShaper::shape_resolved(
        font(),
        &text[end..],
        &prepared.bidi.line(end..text.len()).unwrap(),
        &Default::default(),
    )
    .unwrap();
    let width = crate::fonts::line_layout::measure_run(font(), &prefix, 12.0)
        .unwrap()
        .width()
        .max(
            crate::fonts::line_layout::measure_run(font(), &suffix, 12.0)
                .unwrap()
                .width(),
        )
        + 0.1;
    let lines = prepared
        .break_lines(font(), 0, 12.0, width, &Default::default(), 100)
        .unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].bytes, 0..end);
    assert_eq!(lines[1].bytes, end..text.len());
    let last = prepared.bidi.line(lines[1].bytes.clone()).unwrap();
    assert_eq!(last.context.joining_before, Some('ب'));
    let run = TextShaper::shape_resolved(font(), "ب", &last, &Default::default()).unwrap();
    assert_eq!(run.glyphs[0].glyph_id, gids("ب", &line("بب", 2..4))[0]);
    let metric = crate::fonts::line_layout::measure_run(font(), &run, 12.0).unwrap();
    assert!((metric.width() - lines[1].width).abs() < 1e-7);
}
