//! Unexecuted source tests of the canonical generated-text writer's line plan.
use super::*;

#[test]
fn generated_horizontal_rtl_and_vertical_plans_never_encode_hard_breaks_as_glyphs() {
    let font = get_fallback_font("Symbol").unwrap();
    let options = AdvancedTextEditOptions {
        region: [10.0, 10.0, 200.0, 200.0],
        font_size: 12.0,
        max_lines_or_columns: 10,
        ..Default::default()
    };
    for mode in [
        AdvancedTextMode::ParagraphReflowHorizontal,
        AdvancedTextMode::ParagraphReflowRtl,
        AdvancedTextMode::ParagraphReflowVertical,
    ] {
        for separator in [
            "\r", "\n", "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
        ] {
            let layout =
                layout_generated_logical_text(&format!("A{separator}B"), mode, font, &options)
                    .unwrap();
            assert_eq!(layout.len(), 2, "{mode:?}, {separator:?}");
            assert_eq!(layout[0].len(), 1);
            assert_eq!(layout[1].len(), 1);
            assert_eq!(layout[0][0].visual_unicode, "A");
            assert_eq!(layout[1][0].visual_unicode, "B");
        }
    }
}
