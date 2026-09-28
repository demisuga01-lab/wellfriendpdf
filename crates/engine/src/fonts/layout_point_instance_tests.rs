//! Cross-table contour identity regression source; deliberately unexecuted.
use super::*;

fn checked(tables: &BTreeMap<[u8; 4], Arc<[u8]>>, counts: &[u16]) -> Result<LayoutStage> {
    freeze_with_points(tables, &[], Some(Arc::from(counts)))
}
fn cursive_points(glyphs: &[u16], point: u16, alias: bool) -> Vec<u8> {
    let mut out = words(&[1, 0, glyphs.len() as u16]);
    out.resize(6 + glyphs.len() * 4, 0);
    let mut first = None;
    for i in 0..glyphs.len() {
        if let Some(at) = first.filter(|_| alias) {
            set(&mut out, 6 + i * 4, at);
        } else {
            first = Some(append(&mut out, 6 + i * 4, 0, &words(&[2, 10, 20, point])));
        }
    }
    append(&mut out, 2, 0, &coverage(glyphs));
    out
}
fn positioned(kind: u16, program: Vec<u8>) -> BTreeMap<[u8; 4], Arc<[u8]>> {
    BTreeMap::from([(*b"GPOS", Arc::from(layout(&[(kind, program)])))])
}

#[test]
fn shared_cursive_anchor_is_checked_for_every_covered_glyph_before_cache_reuse() {
    let tables = positioned(3, cursive_points(&[1, 2], 3, true));
    let original = tables.clone();
    let stage = checked(&tables, &[0, 4, 4]).unwrap();
    assert_eq!(stage.contour_point_references, 1); // One relocated anchor.
    assert_eq!(stage.checked_point_references, 2); // Two actual owners.
    assert_eq!(stage.unchecked_point_references, 0);
    assert!(checked(&tables, &[0, 4, 3]).is_err());
    assert_eq!(tables, original);
}
#[test]
fn point_domain_rejects_phantoms_empty_glyphs_and_out_of_range_glyph_ids() {
    let tables = positioned(3, cursive_points(&[1], 3, false));
    assert!(checked(&tables, &[0, 3]).is_err()); // First phantom, not contour point.
    assert!(checked(&tables, &[0]).is_err());
    let empty = positioned(3, cursive_points(&[1], 0, false));
    assert!(checked(&empty, &[0, 0]).is_err());
    assert!(checked(&empty, &[0, 1]).is_ok());
}
#[test]
fn coverage_range_indices_bind_distinct_glyph_domains() {
    let mut program = cursive_points(&[1, 2], 3, true);
    append(&mut program, 2, 0, &words(&[2, 1, 1, 2, 0]));
    let tables = positioned(3, program);
    assert_eq!(
        checked(&tables, &[0, 4, 7])
            .unwrap()
            .checked_point_references,
        2
    );
    assert!(checked(&tables, &[0, 4, 3]).is_err());
}
#[test]
fn isolated_layout_stage_reports_unchecked_points_instead_of_claiming_validation() {
    let tables = positioned(3, cursive_points(&[1, 2], 3, true));
    let stage = freeze(&tables, &[]).unwrap();
    assert_eq!(stage.checked_point_references, 0);
    assert_eq!(stage.unchecked_point_references, 2);
}

fn mark_points(kind: u16) -> Vec<u8> {
    let mut out = words(&[1, 0, 0, 1, 0, 0]);
    let marks = append(&mut out, 8, 0, &words(&[1, 0, 0]));
    append(&mut out, marks + 4, marks, &words(&[2, 0, 0, 2]));
    let bases = append(&mut out, 10, 0, &words(&[2, 0, 0]));
    if kind == 5 {
        for (i, point) in [5, 7].into_iter().enumerate() {
            let components = append(&mut out, bases + 2 + i * 2, bases, &words(&[2, 0, 0]));
            let anchor = append(
                &mut out,
                components + 2,
                components,
                &words(&[2, 0, 0, point]),
            );
            set(&mut out, components + 4, anchor - components);
        }
    } else {
        for (i, point) in [5, 7].into_iter().enumerate() {
            append(
                &mut out,
                bases + 2 + i * 2,
                bases,
                &words(&[2, 0, 0, point]),
            );
        }
    }
    append(&mut out, 2, 0, &coverage(&[3]));
    append(&mut out, 4, 0, &coverage(&[1, 2]));
    out
}
#[test]
fn mark_base_and_mark_mark_anchors_use_the_row_glyph_not_mark_coverage() {
    for kind in [4, 6] {
        let tables = positioned(kind, mark_points(kind));
        assert_eq!(
            checked(&tables, &[0, 6, 8, 3])
                .unwrap()
                .checked_point_references,
            3
        );
        for invalid in [[0, 5, 8, 3], [0, 6, 7, 3], [0, 6, 8, 2]] {
            assert!(checked(&tables, &invalid).is_err());
        }
    }
}
#[test]
fn ligature_component_rows_reference_the_whole_ligature_glyph_point_domain() {
    let tables = positioned(5, mark_points(5));
    let stage = checked(&tables, &[0, 6, 8, 3]).unwrap();
    assert_eq!(stage.checked_point_references, 5); // Mark + two components per ligature.
    assert!(checked(&tables, &[0, 6, 7, 3]).is_err());
}

fn definition_points(caret: bool) -> Vec<u8> {
    let mut out = words(&[1, 0, 0, 0, 0, 0]);
    let list = append(
        &mut out,
        if caret { 8 } else { 6 },
        0,
        &words(&[0, 2, 0, 0]),
    );
    let record = if caret {
        let glyph = append(&mut out, list + 4, list, &words(&[1, 0]));
        append(&mut out, glyph + 2, glyph, &words(&[2, 3]));
        glyph
    } else {
        append(&mut out, list + 4, list, &words(&[2, 1, 3]))
    };
    set(&mut out, list + 6, record - list);
    append(&mut out, list, list, &coverage(&[1, 2]));
    out
}
#[test]
fn gdef_attachments_and_carets_are_bound_to_each_coverage_owner() {
    for caret in [false, true] {
        let tables = BTreeMap::from([(*b"GDEF", Arc::from(definition_points(caret)))]);
        let stage = checked(&tables, &[0, 4, 4]).unwrap();
        assert_eq!(stage.checked_point_references, if caret { 2 } else { 4 });
        assert!(checked(&tables, &[0, 4, 3]).is_err());
        assert!(checked(&tables, &[0, 4]).is_err());
    }
}
#[test]
fn generated_outline_authority_must_match_maxp_glyph_count() {
    let mut tables = positioned(3, cursive_points(&[1], 0, false));
    tables.insert(*b"maxp", Arc::from(words(&[1, 0, 3])));
    assert!(checked(&tables, &[0, 1]).is_err());
    assert!(checked(&tables, &[0, 1, 0]).is_ok());
    assert!(checked(&tables, &vec![1; 65536]).is_err());
}
#[test]
fn point_reference_validation_is_cancellable_and_does_not_mutate_source() {
    let tables = positioned(3, cursive_points(&[1], 0, false));
    let original = tables.clone();
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(checked(&tables, &[0, 1]).is_err()));
    assert_eq!(tables, original);
}
