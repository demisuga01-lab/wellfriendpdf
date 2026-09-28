//! CVT staging and source-face composition fixtures, not executed in this change.
use super::*;
use std::collections::BTreeMap;
fn words(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|n| n.to_be_bytes()).collect()
}
fn coords(n: i16) -> [ttf_parser::NormalizedCoordinate; 1] {
    [ttf_parser::NormalizedCoordinate::from(n)]
}
fn cvar(deltas: &[i16]) -> Vec<u8> {
    let mut data = words(&[1, 0, deltas.len() as i16, (8 + deltas.len() * 6) as i16]);
    for _ in deltas {
        data.extend(words(&[3, 0x8000u16 as i16, 16384]));
    }
    for delta in deltas {
        data.push(0x40);
        data.extend(delta.to_be_bytes());
    }
    data
}
#[test]
fn cvt_rounds_accumulated_deltas_once_and_reports_changed_indices() {
    let source = words(&[100]);
    let stage = freeze(&source, &cvar(&[1, 1]), &coords(8192)).unwrap();
    assert_eq!(stage.bytes, words(&[101]));
    assert_eq!(
        stage.changes,
        [CvtChange {
            index: 0,
            before: 100,
            after: 101
        }]
    );
    assert_eq!(stage.tuple_count, 2);
    assert_eq!(stage.active_tuple_count, 2);
    assert!(stage.work > 0);
    assert_eq!(source, words(&[100]));
}
#[test]
fn negative_halves_and_overlapping_regions_use_checked_final_rounding() {
    assert_eq!(
        freeze(&words(&[100]), &cvar(&[-1]), &coords(8192))
            .unwrap()
            .bytes,
        words(&[100])
    );
    assert_eq!(
        freeze(&words(&[100]), &cvar(&[-1, -1]), &coords(8192))
            .unwrap()
            .bytes,
        words(&[99])
    );
    assert!(freeze(&words(&[32767]), &cvar(&[1]), &coords(16384)).is_err());
    assert!(freeze(&words(&[-32768]), &cvar(&[-1]), &coords(16384)).is_err());
}
#[test]
fn unchanged_instance_keeps_cvt_bytes_and_has_no_change_receipts() {
    let stage = freeze(&words(&[100]), &cvar(&[40]), &coords(0)).unwrap();
    assert_eq!(stage.bytes, words(&[100]));
    assert!(stage.changes.is_empty());
    assert_eq!(stage.active_tuple_count, 0);
}
#[test]
fn sparse_duplicate_cvt_indices_leave_other_values_and_indices_unchanged() {
    let mut data = words(&[1, 0, 1, 14, 9, 0xa000u16 as i16, 16384]);
    // Three references to CVT index one, followed by three signed byte deltas.
    data.extend([3, 2, 1, 0, 0, 2, 10, 0xfe, 7]);
    let stage = freeze(&words(&[100, 200, 300]), &data, &coords(16384)).unwrap();
    assert_eq!(stage.bytes, words(&[100, 215, 300]));
    assert_eq!(
        stage.changes,
        [CvtChange {
            index: 1,
            before: 200,
            after: 215
        }]
    );
}
#[test]
fn cvt_requires_complete_words_valid_versions_and_embedded_peak_records() {
    assert!(freeze(&[0], &cvar(&[1]), &coords(16384)).is_err());
    let mut bad = cvar(&[1]);
    bad[1] = 2;
    assert!(freeze(&words(&[100]), &bad, &coords(16384)).is_err());
    let mut bad = cvar(&[1]);
    bad[10] = 0;
    assert!(freeze(&words(&[100]), &bad, &coords(16384)).is_err());
}
fn tables(font: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
    crate::fonts::font_container::Container::parse(font)
        .unwrap()
        .faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, font[range.clone()].to_vec()))
        .collect()
}
fn source_font(variable: bool, include_cvt: bool) -> Vec<u8> {
    let mut tables = tables(&crate::fonts::pdf_embedding_fixtures::font(false, 0));
    tables.remove(b"CFF ");
    let mut glyph = words(&[1, 0, 0, 100, 100, 2, 0]);
    glyph.extend([0x31, 0x33, 0x27, 100, 100, 100]); // Three explicit triangle points.
    assert_eq!(glyph.len(), 20);
    let mut glyf = glyph.clone();
    glyf.extend(glyph);
    tables.insert(*b"glyf", glyf);
    tables.insert(*b"loca", words(&[0, 0, 10, 20, 20]));
    tables.get_mut(b"head").unwrap()[50..52].copy_from_slice(&[0, 0]);
    let mut maxp = vec![0; 32];
    maxp[1] = 1;
    maxp[5] = 4;
    maxp[7] = 3;
    maxp[9] = 1;
    maxp[15] = 2;
    maxp[25] = 1;
    tables.insert(*b"maxp", maxp);
    if include_cvt {
        tables.insert(*b"cvt ", words(&[100]));
    }
    tables.insert(*b"cvar", cvar(&[40]));
    if variable {
        let mut fvar = words(&[1, 0, 16, 2, 1, 20, 0, 8]);
        fvar.extend(b"TEST");
        for n in [-65536i32, 0, 65536] {
            fvar.extend(n.to_be_bytes());
        }
        fvar.extend(words(&[0, 256]));
        tables.insert(*b"fvar", fvar);
    }
    crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], tables).unwrap()
}
#[test]
fn selected_font_face_stages_cvt_with_the_same_normalized_coordinates_as_metrics() {
    let source = source_font(true, true);
    let request =
        crate::fonts::VariationRequest::none().with_axis(ttf_parser::Tag::from_bytes(b"TEST"), 0.5);
    let stage = crate::fonts::font_metric_instance::prepare(&source, 0, &request).unwrap();
    assert_eq!(stage.coordinates[0].get(), 8192);
    assert_eq!(stage.cvt.as_ref().unwrap().bytes, words(&[120]));
    let mut saved = tables(&source);
    saved.insert(*b"cvt ", stage.cvt.unwrap().bytes);
    saved.remove(b"cvar");
    saved.remove(b"fvar");
    saved.extend(stage.metrics.tables);
    let saved = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], saved).unwrap();
    let face = ttf_parser::Face::parse(&saved, 0).unwrap();
    assert_eq!(
        face.raw_face()
            .table(ttf_parser::Tag::from_bytes(b"cvt "))
            .unwrap(),
        words(&[120])
    );
    assert_eq!(face.glyph_index('A'), Some(ttf_parser::GlyphId(1)));
    assert_eq!(face.glyph_hor_advance(ttf_parser::GlyphId(1)), Some(600));
}
#[test]
fn nonvariable_sources_ignore_cvar_but_variable_sources_require_its_cvt_owner() {
    let source = source_font(false, false);
    let prepared = crate::fonts::font_metric_instance::prepare(
        &source,
        0,
        &crate::fonts::VariationRequest::none(),
    )
    .unwrap();
    assert!(prepared.cvt.is_none());
    assert!(prepared.ignored_variation_tables.contains(b"cvar"));
    assert!(crate::fonts::font_metric_instance::prepare(
        &source_font(true, false),
        0,
        &crate::fonts::VariationRequest::none()
    )
    .is_err());
}
#[test]
fn cvar_cannot_be_applied_to_cff_outlines() {
    let source = source_font(true, true);
    let mut modified = tables(&source);
    modified.remove(b"glyf");
    modified.remove(b"loca");
    modified.insert(*b"CFF ", crate::fonts::pdf_embedding_fixtures::cff(false));
    modified.insert(*b"maxp", vec![0, 0, 0x50, 0, 0, 4]);
    let font = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", modified).unwrap();
    assert!(crate::fonts::font_metric_instance::prepare(
        &font,
        0,
        &crate::fonts::VariationRequest::none()
    )
    .is_err());
}
#[test]
fn cancelled_cvt_staging_returns_no_output() {
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(freeze(&words(&[100]), &cvar(&[1]), &coords(16384)).is_err()));
}
