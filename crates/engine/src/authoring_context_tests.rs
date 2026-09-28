//! Unexecuted writer-context regression source; no independent raster evidence.
use super::*;
use crate::fonts::shaper::{ParagraphBidi, ShapeOptions, TextDirection};

#[test]
fn identical_text_with_distinct_joining_context_has_distinct_cids_and_only_visible_unicode() {
    let font = crate::render::get_fallback_font("Symbol").unwrap();
    let options = ShapeOptions {
        direction: Some(TextDirection::RightToLeft),
    };
    let middle = ParagraphBidi::new("ببب", options)
        .unwrap()
        .line(2..4)
        .unwrap();
    let alone = ParagraphBidi::new("ب", options)
        .unwrap()
        .line(0..2)
        .unwrap();
    let mut builder = PdfBuilder::new();
    let face = builder
        .register_font_bytes("Arabic", font.to_vec())
        .unwrap();
    let style = TextStyle::new(face, 12.0);
    let page = builder.add_page(PageSize::custom(200.0, 200.0));
    page.draw_text_resolved("ب", 20.0, 100.0, &style, &middle)
        .unwrap();
    page.draw_text_resolved("ب", 20.0, 80.0, &style, &alone)
        .unwrap();
    let plan = FontBuildPlan::from_builder(&builder).unwrap();
    let a = plan.shaped_run_resolved(face, "ب", Some(&middle)).unwrap();
    let b = plan.shaped_run_resolved(face, "ب", Some(&alone)).unwrap();
    assert_eq!(a.actual_text, "ب");
    assert_eq!(b.actual_text, "ب");
    assert_eq!(a.glyphs.len(), 1);
    assert_eq!(b.glyphs.len(), 1);
    assert_ne!(a.glyphs[0].cid, b.glyphs[0].cid);
    let embedded = plan.embedded_plan(face).unwrap();
    for glyph in [&a.glyphs[0], &b.glyphs[0]] {
        assert_eq!(
            embedded
                .entries
                .iter()
                .find(|e| e.cid == glyph.cid)
                .unwrap()
                .unicode,
            "ب"
        );
    }
    let output = builder.to_bytes().unwrap();
    let reopened = crate::ContentEngine::open_bytes(output).unwrap();
    assert_eq!(
        reopened
            .get_page_text(1)
            .unwrap()
            .chars()
            .filter(|c| *c == 'ب')
            .count(),
        2
    );
}

#[test]
fn invalid_joining_context_does_not_append_a_partial_authoring_command() {
    let mut builder = PdfBuilder::new();
    let page = builder.add_page(PageSize::custom(200.0, 200.0));
    let mut bidi = ParagraphBidi::new("X", ShapeOptions::default())
        .unwrap()
        .line(0..1)
        .unwrap();
    bidi.context.after = "abcdef".into();
    let before = page.commands.len();
    assert!(page
        .draw_text_resolved(
            "X",
            10.0,
            20.0,
            &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
            &bidi
        )
        .is_err());
    assert_eq!(page.commands.len(), before);
}

#[test]
fn hard_separators_are_rejected_before_appending_a_resolved_authoring_command() {
    for separator in [
        "\r", "\n", "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
    ] {
        let mut builder = PdfBuilder::new();
        let page = builder.add_page(PageSize::custom(200.0, 200.0));
        let text = format!("A{separator}B");
        let bidi = crate::fonts::shaper::LineBidi {
            levels: vec![0; text.len()],
            rtl: false,
            context: Default::default(),
        };
        assert!(page
            .draw_text_resolved(
                text,
                10.0,
                20.0,
                &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
                &bidi
            )
            .is_err());
        assert!(page.commands.is_empty());
    }
}

#[test]
fn distant_neighbour_synopsis_changes_cids_without_leaking_context_into_pdf_text() {
    let font = crate::render::get_fallback_font("Symbol").unwrap();
    let context = |neighbour| {
        let text = format!("{neighbour}{}ب", "\u{064e}".repeat(128));
        ParagraphBidi::new(
            &text,
            ShapeOptions {
                direction: Some(TextDirection::RightToLeft),
            },
        )
        .unwrap()
        .line(text.len() - 2..text.len())
        .unwrap()
    };
    let joined = context('ب');
    let separate = context('\u{200c}');
    assert_eq!(joined.context.before, separate.context.before);
    assert_ne!(
        joined.context.joining_before,
        separate.context.joining_before
    );
    let mut builder = PdfBuilder::new();
    let face = builder
        .register_font_bytes("Arabic", font.to_vec())
        .unwrap();
    let style = TextStyle::new(face, 12.0);
    let page = builder.add_page(PageSize::custom(200.0, 200.0));
    page.draw_text_resolved("ب", 20.0, 100.0, &style, &joined)
        .unwrap();
    page.draw_text_resolved("ب", 20.0, 80.0, &style, &separate)
        .unwrap();
    let plan = FontBuildPlan::from_builder(&builder).unwrap();
    let a = plan.shaped_run_resolved(face, "ب", Some(&joined)).unwrap();
    let b = plan
        .shaped_run_resolved(face, "ب", Some(&separate))
        .unwrap();
    assert_ne!(a.glyphs[0].cid, b.glyphs[0].cid);
    assert_eq!(a.actual_text, "ب");
    assert_eq!(b.actual_text, "ب");
    let output = crate::ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
    let text = output.get_page_text(1).unwrap();
    assert_eq!(text.chars().filter(|c| *c == 'ب').count(), 2);
    assert!(!text.contains('\u{064e}'));
    assert!(!text.contains('\u{200c}'));
}
