//! Regression source only; these cases have not been executed.
use super::*;
use crate::fonts::{ShapeOptions, ShapedGlyph, TextDirection, TextShaper};

fn font() -> &'static [u8] {
    crate::render::get_fallback_font("Symbol").unwrap()
}
fn shaped(text: &str) -> ShapedRun {
    TextShaper::shape(font(), text, ShapeOptions::default()).unwrap()
}
fn glyph(gid: u16, cluster: u32) -> ShapedGlyph {
    ShapedGlyph {
        glyph_id: gid,
        cluster,
        advance: 500.0,
        offset_x: 0.0,
        offset_y: 0.0,
    }
}
fn run(glyphs: Vec<ShapedGlyph>) -> ShapedRun {
    ShapedRun {
        glyphs,
        direction: TextDirection::LeftToRight,
        used_complex_shaping: true,
    }
}

#[test]
fn retained_subset_cmap_reports_missing_nonzero_glyph_clusters() {
    let face = ttf_parser::Face::parse(font(), 0).unwrap();
    let subset = super::super::sfnt_subset::subset_glyf_preserving_gids(
        font(),
        &BTreeSet::from([
            face.glyph_index('A').unwrap().0,
            face.glyph_index(' ').unwrap().0,
        ]),
    )
    .unwrap();
    let run = TextShaper::shape(&subset.bytes, "A Z", ShapeOptions::default()).unwrap();
    assert!(run.glyphs.iter().all(|glyph| glyph.glyph_id > 0));
    assert_eq!(
        missing_glyph_clusters(&subset.bytes, "A Z", &run).unwrap(),
        [2]
    );
    let summary = analyze_text(&subset.bytes, "A Z").unwrap();
    assert_eq!(summary.missing_scalars, ['Z']);
    assert_eq!((summary.covered_scalars, summary.required_scalars), (1, 2));
}

#[test]
fn decomposition_without_a_nominal_precomposed_cmap_is_supported() {
    let font = super::super::sfnt_subset::with_test_cmap(font(), &[' ', 'A', '\u{030a}']).unwrap();
    let face = ttf_parser::Face::parse(&font, 0).unwrap();
    assert!(face.glyph_index('Å').is_none());
    let run = TextShaper::shape(&font, "Å", ShapeOptions::default()).unwrap();
    assert!(!run.glyphs.is_empty());
    assert!(missing_glyph_clusters(&font, "Å", &run).unwrap().is_empty());
    let summary = analyze_text(&font, "Å").unwrap();
    assert_eq!((summary.covered_scalars, summary.required_scalars), (1, 1));
    assert!(summary.missing_scalars.is_empty());
}

#[test]
fn shaped_joiners_and_a_mark_on_a_space_do_not_require_a_painted_space() {
    for text in [
        "A\u{200d}B",
        "ب\u{200c}ب",
        "ب\u{200d}ب",
        "A\u{fe0f}",
        " \u{0301}",
        "\u{00a0}\u{0301}",
    ] {
        assert!(
            missing_glyph_clusters(font(), text, &shaped(text))
                .unwrap()
                .is_empty(),
            "{text:?}"
        );
    }
}

#[test]
fn allowing_a_space_outline_does_not_hide_a_removed_combining_mark() {
    let face = ttf_parser::Face::parse(font(), 0).unwrap();
    let subset = super::super::sfnt_subset::subset_glyf_preserving_gids(
        font(),
        &BTreeSet::from([face.glyph_index(' ').unwrap().0]),
    )
    .unwrap();
    let run = TextShaper::shape(&subset.bytes, " \u{0301}", ShapeOptions::default()).unwrap();
    assert!(!missing_glyph_clusters(&subset.bytes, " \u{0301}", &run)
        .unwrap()
        .is_empty());
}

#[test]
fn malformed_clusters_are_rejected_even_when_the_glyph_has_an_outline() {
    let face = ttf_parser::Face::parse(font(), 0).unwrap();
    let gid = face.glyph_index('A').unwrap().0;
    for cluster in [1, 2, u32::MAX] {
        assert!(missing_glyph_clusters(font(), "é", &run(vec![glyph(gid, cluster)])).is_err());
    }
    let mut bad = glyph(gid, 0);
    bad.offset_x = f64::NAN;
    assert!(missing_glyph_clusters(font(), "A", &run(vec![bad])).is_err());
}

#[test]
fn notdef_is_missing_even_for_whitespace_and_duplicate_clusters_are_deduplicated() {
    assert_eq!(
        missing_glyph_clusters(font(), " ", &run(vec![glyph(0, 0)])).unwrap(),
        [0]
    );
    assert_eq!(
        missing_glyph_clusters(
            font(),
            "AB",
            &run(vec![glyph(0, 1), glyph(0, 0), glyph(0, 1)])
        )
        .unwrap(),
        [0, 1]
    );
}

#[test]
fn empty_output_and_omitted_visible_prefix_cannot_pass_coverage_vacuously() {
    assert_eq!(
        missing_glyph_clusters(font(), "AB", &run(vec![])).unwrap(),
        [0, 1]
    );
    assert!(
        missing_glyph_clusters(font(), "\u{200d}\u{2060}\n", &run(vec![]))
            .unwrap()
            .is_empty()
    );
    let gid = ttf_parser::Face::parse(font(), 0)
        .unwrap()
        .glyph_index('B')
        .unwrap()
        .0;
    assert_eq!(
        missing_glyph_clusters(font(), "AB", &run(vec![glyph(gid, 1)])).unwrap(),
        [0]
    );
}

#[test]
fn a_hard_separator_does_not_authorize_an_empty_visible_letter_glyph() {
    let space = ttf_parser::Face::parse(font(), 0)
        .unwrap()
        .glyph_index(' ')
        .unwrap()
        .0;
    assert_eq!(
        missing_glyph_clusters(font(), "A\n", &run(vec![glyph(space, 0)])).unwrap(),
        [0]
    );
}

#[test]
fn cancelled_coverage_returns_a_cancellation_error_not_a_covered_result() {
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    assert!(matches!(
        token.scope(|| missing_glyph_clusters(font(), "A", &run(vec![]))),
        Err(WellfriendError::Cancelled(_))
    ));
}
