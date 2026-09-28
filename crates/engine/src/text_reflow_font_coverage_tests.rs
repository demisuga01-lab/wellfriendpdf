//! Source regression only; not compiled or executed in this source-only phase.
use super::*;

#[test]
fn approved_reflow_uses_shaped_outlines_instead_of_nominal_cmap_in_every_mode() {
    let full = get_fallback_font("Symbol").unwrap();
    let decomposed =
        crate::fonts::sfnt_subset::with_test_cmap(full, &[' ', 'A', '\u{030a}']).unwrap();
    let face = ttf_parser::Face::parse(full, 0).unwrap();
    let subset = crate::fonts::sfnt_subset::subset_glyf_preserving_gids(
        full,
        &std::collections::BTreeSet::from([face.glyph_index('A').unwrap().0]),
    )
    .unwrap();
    for direction in ["left_to_right", "right_to_left", "vertical_rl"] {
        let mut request: GeometricReflowRequest = serde_json::from_value(json!({
            "source_text": "ABC", "replacement_text": "Å", "direction": direction,
            "font_policy": "approved_substitute:Caller",
            "approved_font_asset": { "lookup_name": "Caller", "bytes": decomposed },
        }))
        .unwrap();
        let (name, bytes) = approved_reflow_font(&request).unwrap().unwrap();
        assert_eq!(name, "Caller");
        assert_eq!(bytes, decomposed.as_slice());
        request.replacement_text = "Z".into();
        request.approved_font_asset.as_mut().unwrap().bytes = subset.bytes.clone();
        assert!(approved_reflow_font(&request).is_err());
        request.font_policy = "approved_substitute:Another".into();
        assert!(approved_reflow_font(&request).is_err());
    }
}
