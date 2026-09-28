//! Regression source only; no font/PDF workload has been executed.
use super::*;

fn subset() -> Vec<u8> {
    let font = get_fallback_font("Symbol").unwrap();
    let face = ttf_parser::Face::parse(font, 0).unwrap();
    subset_glyf_preserving_gids(
        font,
        &BTreeSet::from([
            face.glyph_index('A').unwrap().0,
            face.glyph_index(' ').unwrap().0,
        ]),
    )
    .unwrap()
    .bytes
}
fn decomposed_font() -> Vec<u8> {
    crate::fonts::sfnt_subset::with_test_cmap(
        get_fallback_font("Symbol").unwrap(),
        &[' ', 'A', '\u{030a}'],
    )
    .unwrap()
}

#[test]
fn advanced_analysis_uses_outline_coverage_in_all_writing_modes() {
    for mode in [
        AdvancedTextMode::ParagraphReflowHorizontal,
        AdvancedTextMode::ParagraphReflowRtl,
        AdvancedTextMode::ParagraphReflowVertical,
    ] {
        let analysis =
            analyze_advanced_text_reflow("A Z", mode, Some(&subset()), TextReflowLimits::default())
                .unwrap();
        assert_eq!(analysis.missing_glyph_clusters, [2]);
        assert!(analysis
            .glyphs
            .iter()
            .filter(|g| g.source_cluster_utf8 == 2)
            .all(|g| g.missing));
        assert_eq!(
            analysis.status,
            AdvancedEditingSupportStatus::UnsupportedReportedExact
        );
    }
}

#[test]
fn absent_nominal_cmap_does_not_veto_valid_normalized_glyph_output() {
    let font = decomposed_font();
    for mode in [
        AdvancedTextMode::ParagraphReflowHorizontal,
        AdvancedTextMode::ParagraphReflowRtl,
        AdvancedTextMode::ParagraphReflowVertical,
    ] {
        let analysis =
            analyze_advanced_text_reflow("Å", mode, Some(&font), TextReflowLimits::default())
                .unwrap();
        assert!(!analysis.glyphs.is_empty());
        assert!(analysis.missing_glyph_clusters.is_empty());
        assert!(generated_glyph_plan("Å", mode, &font).is_ok());
    }
}

#[test]
fn final_layout_checks_coverage_instead_of_trusting_an_earlier_preview() {
    for mode in [
        AdvancedTextMode::ParagraphReflowHorizontal,
        AdvancedTextMode::ParagraphReflowVertical,
    ] {
        assert!(generated_glyph_plan("Z", mode, &subset()).is_err());
        let line = ExplicitLayoutLine {
            logical_text: "Z".into(),
            visual_text: "Z".into(),
            inserted_visual_hyphen: false,
            bidi: None,
        };
        assert!(layout_generated_explicit_lines(
            &[line],
            mode,
            &subset(),
            &AdvancedTextEditOptions::default(),
            None
        )
        .is_err());
    }
}

#[test]
fn analysis_preserves_context_and_provenance_across_mandatory_lines() {
    for mode in [
        AdvancedTextMode::ParagraphReflowHorizontal,
        AdvancedTextMode::ParagraphReflowRtl,
        AdvancedTextMode::ParagraphReflowVertical,
    ] {
        for separator in [
            "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
        ] {
            let text = format!("AB{separator}ب 123{separator}CD");
            let analysis =
                analyze_advanced_text_reflow(&text, mode, None, TextReflowLimits::default())
                    .unwrap();
            assert!(analysis.missing_glyph_clusters.is_empty());
            assert_eq!(analysis.visual_text.matches(separator).count(), 2);
            for glyph in &analysis.glyphs {
                let source = &analysis.bidi_runs[glyph.source_run_index];
                assert!((source.logical_byte_start..source.logical_byte_end)
                    .contains(&(glyph.source_cluster_utf8 as usize)));
                assert!(text.is_char_boundary(glyph.source_cluster_utf8 as usize));
            }
        }
    }
}

#[test]
fn decomposed_replacement_saves_logical_text_and_subset_holes_are_rejected() {
    let input = tests::advanced_editing_fixture_with_content(
        false,
        b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n",
    );
    let options = AdvancedTextEditOptions {
        region: [10.0, 10.0, 190.0, 190.0],
        ..Default::default()
    };
    assert!(edit_advanced_text_pdf(
        &input,
        1,
        "ABC",
        "Z",
        AdvancedTextMode::ParagraphReflowHorizontal,
        &options,
        Some(&subset())
    )
    .is_err());
    let (output, _) = edit_advanced_text_pdf(
        &input,
        1,
        "ABC",
        "Å",
        AdvancedTextMode::ParagraphReflowHorizontal,
        &options,
        Some(&decomposed_font()),
    )
    .unwrap();
    let engine = ContentEngine::open_bytes(output).unwrap();
    assert_eq!(
        engine
            .collect_page_text_chunks(1)
            .unwrap()
            .into_iter()
            .map(|chunk| chunk.text)
            .collect::<String>(),
        "Å"
    );
}
