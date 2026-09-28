//! Source-to-metric-stage regressions. No build or execution in this change.
use super::*;
fn tables(source: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
    Container::parse(source).unwrap().faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, source[range.clone()].to_vec()))
        .collect()
}
fn font() -> Vec<u8> {
    crate::fonts::pdf_embedding_fixtures::font(false, 0)
}
fn variable_font() -> Vec<u8> {
    let mut tables = tables(&font());
    let mut fvar = vec![0, 1, 0, 0, 0, 16, 0, 2, 0, 1, 0, 20, 0, 0, 0, 8];
    fvar.extend_from_slice(b"TEST");
    for n in [-65536i32, 0, 65536] {
        fvar.extend_from_slice(&n.to_be_bytes());
    }
    fvar.extend_from_slice(&[0, 0, 1, 0]);
    tables.insert(*b"fvar", fvar);
    let mut hvar = vec![0; 20];
    hvar[1] = 1;
    hvar[7] = 20;
    for n in [
        1u16, 0, 12, 1, 0, 22, 1, 1, 0, 16384, 16384, 4, 1, 1, 0, 0, 40, 80, 0,
    ] {
        hvar.extend_from_slice(&n.to_be_bytes());
    }
    tables.insert(*b"HVAR", hvar);
    crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap()
}

#[test]
fn actual_font_outlines_and_gids_feed_the_metric_stage() {
    let source = font();
    let stage = prepare(&source, 0, &VariationRequest::none()).unwrap();
    assert_eq!(
        stage.source_sha256,
        format!("{:x}", Sha256::digest(&source))
    );
    assert_eq!(stage.face_index, 0);
    assert_eq!(stage.outline_kind, OutlineKind::Cff);
    assert!(stage.coordinates.is_empty());
    assert_eq!(stage.geometry.len(), 4);
    assert!(stage.geometry[0].instance.is_none());
    assert_eq!(stage.geometry[1].instance.unwrap().x_max, 100);
    assert_eq!(
        stage
            .metrics
            .horizontal
            .iter()
            .map(|m| m.advance)
            .collect::<Vec<_>>(),
        vec![600, 600, 600, 300]
    );
}

#[test]
fn selected_coordinates_resolve_metrics_from_the_same_source_face() {
    let source = variable_font();
    let request = VariationRequest::none().with_axis(ttf_parser::Tag::from_bytes(b"TEST"), 0.5);
    let stage = prepare(&source, 0, &request).unwrap();
    assert_eq!(stage.coordinates[0].get(), 8192);
    assert_eq!(stage.metrics.horizontal[1].advance, 620);
    assert_eq!(stage.metrics.horizontal[2].advance, 640);
    let default = prepare(&source, 0, &VariationRequest::none()).unwrap();
    assert_eq!(default.metrics.horizontal[1].advance, 600);
}

#[test]
fn collection_face_selection_is_not_silently_replaced_by_face_zero() {
    let first = font();
    let second = variable_font();
    let source = crate::fonts::font_asset::tests::collection(&[&first, &second], 0x10000, false);
    let request = VariationRequest::none().with_axis(ttf_parser::Tag::from_bytes(b"TEST"), 1.);
    let first = prepare(&source, 0, &VariationRequest::none()).unwrap();
    let second = prepare(&source, 1, &request).unwrap();
    assert_eq!(first.metrics.horizontal[1].advance, 600);
    assert_eq!(second.metrics.horizontal[1].advance, 640);
    assert_eq!(second.face_index, 1);
    assert_eq!(first.source_sha256, second.source_sha256);
    assert!(prepare(&source, 2, &request).is_err());
    assert!(prepare(&source, 0, &request).is_err()); // Do not hide an unknown explicit axis.
}

#[test]
fn nonvariable_face_ignores_inapplicable_metric_variation_tables() {
    let mut tables = tables(&font());
    tables.insert(*b"HVAR", vec![0xff]);
    tables.insert(*b"MVAR", vec![0xff]);
    let source = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap();
    let stage = prepare(&source, 0, &VariationRequest::none()).unwrap();
    assert_eq!(stage.ignored_variation_tables, vec![*b"HVAR", *b"MVAR"]);
    assert_eq!(stage.metrics.horizontal[1].advance, 600);
}

#[test]
fn malformed_outline_table_is_not_captured_as_all_blank_glyphs() {
    let mut tables = tables(&font());
    tables.insert(*b"CFF ", vec![0]);
    let source = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap();
    assert!(prepare(&source, 0, &VariationRequest::none()).is_err());
}

#[test]
fn captured_metric_tables_reopen_with_the_same_glyph_order_and_advances() {
    let source = font();
    let stage = prepare(&source, 0, &VariationRequest::none()).unwrap();
    let mut output = tables(&source);
    output.extend(stage.metrics.tables);
    let rebuilt = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", output).unwrap();
    let face = ttf_parser::Face::parse(&rebuilt, 0).unwrap();
    assert_eq!(face.number_of_glyphs(), 4);
    for (gid, advance) in [(0, 600), (1, 600), (2, 600), (3, 300)] {
        assert_eq!(
            face.glyph_hor_advance(ttf_parser::GlyphId(gid)),
            Some(advance)
        );
    }
    assert_eq!(face.glyph_index('A'), Some(ttf_parser::GlyphId(1)));
    assert_eq!(face.glyph_index('B'), Some(ttf_parser::GlyphId(2)));
}

#[test]
fn cancellation_prevents_source_bound_metric_preparation() {
    let source = font();
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(prepare(&source, 0, &VariationRequest::none()).is_err()));
}
