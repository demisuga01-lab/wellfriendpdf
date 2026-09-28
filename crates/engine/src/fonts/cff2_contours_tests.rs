//! Regression source only; no geometry/font/PDF workload ran while adding it.
use super::super::super::{tests as fixtures, Program};
use super::super::{freeze_with_contours, tests as instance_tests};
use super::*;
use crate::fonts::font_instance;

fn options() -> ContourNormalization {
    ContourNormalization {
        tolerance_font_units: 0.001,
        allow_hint_loss: false,
    }
}
fn square(x: f64, y: f64, size: f64) -> BezPath {
    kurbo::Rect::new(x, y, x + size, y + size).to_path(0.)
}
fn join(paths: &[BezPath]) -> BezPath {
    let mut out = BezPath::new();
    for path in paths {
        out.extend(path.iter());
    }
    out
}
fn normalized(path: &BezPath) -> (Vec<u8>, ContourReport) {
    let opt = options();
    let mut report = ContourReport::new(&opt);
    let code = encode(path, -32168.).unwrap();
    let result = normalize(&code, -32168., 1, &opt, &mut report).unwrap();
    (result, report)
}
fn filled(path: &BezPath, p: (f64, f64)) -> bool {
    path.winding(p.into()) != 0
}
pub(crate) fn source_font(hinted: bool) -> Vec<u8> {
    let path = join(&[square(0., 0., 100.), square(50., 50., 100.)]);
    font_for_path(&path, hinted)
}
pub(crate) fn compatible_source_font() -> Vec<u8> {
    font_for_path(&square(0., 0., 100.), true)
}
fn font_for_path(path: &BezPath, hinted: bool) -> Vec<u8> {
    fixtures::sfnt(
        fixtures::fixture(
            &[vec![], source_program(path, hinted), vec![], vec![]],
            &[vec![]],
            &[None],
            &[0; 4],
            0,
            None,
            &[],
        ),
        false,
    )
}
fn source_program(path: &BezPath, hinted: bool) -> Vec<u8> {
    let code = encode(path, -32168.).unwrap();
    let mut at = 1;
    number(&code, &mut at, code[0], false).unwrap();
    let mut source = if hinted {
        vec![139, 159, 18, 19, 128]
    } else {
        vec![]
    };
    source.extend(&code[at..code.len() - 1]);
    source
}
fn request(source: &[u8], allow_hint_loss: bool) -> font_instance::FontInstanceRequest {
    let mut r = font_instance::tests::request(source);
    r.coordinates.clear();
    r.cff2_contours = Some(ContourNormalization {
        allow_hint_loss,
        ..options()
    });
    r
}

#[test]
fn overlapping_rectangles_become_one_nonzero_filled_boundary_with_same_width() {
    let source = join(&[square(0., 0., 100.), square(50., 50., 100.)]);
    let (bytes, report) = normalized(&source);
    let path = decode(&bytes).unwrap();
    assert!((path.area().abs() - 17500.).abs() < 0.01);
    assert_eq!(path.winding((75., 75.).into()).abs(), 1);
    assert!(filled(&path, (25., 25.)));
    assert!(filled(&path, (125., 125.)));
    assert!(!filled(&path, (25., 125.)));
    let mut at = 1;
    assert_eq!(
        number(&bytes, &mut at, bytes[0], false).unwrap(),
        Some(-32168.)
    );
    assert_eq!(report.normalized_glyphs, [1]);
    assert!(!report.independently_verified);
}
#[test]
fn crossing_cubic_outlines_remain_curves_not_polygons() {
    let a = kurbo::Circle::new((0., 0.), 100.).to_path(1e-6);
    let b = kurbo::Circle::new((80., 0.), 100.).to_path(1e-6);
    let (bytes, _) = normalized(&join(&[a, b]));
    let path = decode(&bytes).unwrap();
    assert!(path.iter().any(|e| matches!(e, PathEl::CurveTo(..))));
    for p in [(-50., 0.), (40., 0.), (130., 0.)] {
        assert!(filled(&path, p));
    }
    assert!(!filled(&path, (40., 150.)));
    assert!(Topology::from_path(&path, 0.000125)
        .unwrap()
        .has_normal_contours(()));
}
#[test]
fn holes_and_nested_islands_keep_nonzero_semantics() {
    let path = join(&[
        square(0., 0., 200.),
        square(20., 20., 160.).reverse_subpaths(),
        square(60., 60., 80.),
    ]);
    let (code, _) = normalized(&path);
    let saved = decode(&code).unwrap();
    assert!(filled(&saved, (10., 10.)));
    assert!(!filled(&saved, (40., 40.)));
    assert!(filled(&saved, (100., 100.)));
    assert!((saved.area().abs() - 20800.).abs() < 0.01);
}
#[test]
fn coincident_and_opposite_contours_do_not_duplicate_or_invent_ink() {
    let a = square(0., 0., 100.);
    let (same, _) = normalized(&join(&[a.clone(), a.clone()]));
    assert!((decode(&same).unwrap().area().abs() - 10000.).abs() < 0.01);
    let (cancelled, _) = normalized(&join(&[a.clone(), a.reverse_subpaths()]));
    assert_eq!(decode(&cancelled).unwrap().segments().count(), 0);
}
#[test]
fn self_crossing_contour_splits_without_evenodd_substitution() {
    let path = BezPath::from_vec(vec![
        PathEl::MoveTo((0., 0.).into()),
        PathEl::LineTo((100., 100.).into()),
        PathEl::LineTo((0., 100.).into()),
        PathEl::LineTo((100., 0.).into()),
        PathEl::ClosePath,
    ]);
    let (code, _) = normalized(&path);
    let saved = decode(&code).unwrap();
    assert!(filled(&saved, (50., 10.)));
    assert!(filled(&saved, (50., 90.)));
    assert!(!filled(&saved, (10., 50.)));
}
#[test]
fn encoding_closure_keeps_the_type2_pen_at_the_last_point() {
    let path = BezPath::from_vec(vec![
        PathEl::MoveTo((10., 10.).into()),
        PathEl::LineTo((20., 10.).into()),
        PathEl::LineTo((20., 20.).into()),
        PathEl::ClosePath,
        PathEl::MoveTo((100., 100.).into()),
        PathEl::LineTo((110., 100.).into()),
        PathEl::LineTo((110., 110.).into()),
        PathEl::ClosePath,
    ]);
    let code = encode(&path, -32168.).unwrap();
    assert_eq!(decode(&code).unwrap(), path);
}
#[test]
fn flex_decoding_keeps_curve_endpoints_and_records_hint_loss_at_publication() {
    for (op, values, expected) in [
        (34, vec![1.; 7], (6., 0.)),
        (35, vec![1.; 13], (6., 6.)),
        (36, vec![1.; 9], (6., 0.)),
        (37, vec![1.; 11], (0., 6.)),
    ] {
        let mut code = Vec::new();
        charstring::encode_number(&mut code, -32168.).unwrap();
        code.extend([139, 139, 21]);
        for v in values {
            charstring::encode_number(&mut code, v).unwrap();
        }
        code.extend([12, op, 14]);
        let path = decode(&code).unwrap();
        let ends = path
            .iter()
            .filter_map(|p| {
                if let PathEl::CurveTo(_, _, p) = p {
                    Some(p)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(ends.len(), 2);
        assert_eq!(ends[1], expected.into());
    }
}
#[test]
fn tolerance_input_and_aggregate_budgets_and_cancellation_fail_before_publication() {
    for tolerance in [f64::NAN, 0., -1., 1., QUANTUM / 2.] {
        assert!(ContourNormalization {
            tolerance_font_units: tolerance,
            allow_hint_loss: true
        }
        .validate()
        .is_err());
    }
    let code = encode(&square(0., 0., 100.), -32168.).unwrap();
    let opt = options();
    let mut report = ContourReport::new(&opt);
    report.input_segments = MAX_TOTAL;
    assert!(normalize(&code, -32168., 1, &opt, &mut report).is_err());
    let token = crate::CancelToken::new();
    token.cancel();
    token.scope(|| {
        assert!(normalize(&code, -32168., 1, &opt, &mut ContourReport::new(&opt)).is_err());
    });
    assert!(decode(&[139, 12, 0, 14]).is_err());
    assert!(decode(&[139, 139, 139, 21]).is_err());
}
#[test]
fn full_static_font_publishes_normalized_curves_and_rebuilt_metrics() {
    let source = source_font(false);
    let output = font_instance::prepare_font_instance(&source, &request(&source, false)).unwrap();
    let report = output.report.cff2.unwrap();
    assert!(report.contour_overlaps_checked && report.contour_overlaps_removed);
    assert_eq!(report.contour_normalization.unwrap().normalized_glyphs, [1]);
    let face = ttf_parser::Face::parse(&output.bytes, 0).unwrap();
    assert_eq!(face.glyph_hor_advance(ttf_parser::GlyphId(1)), Some(600));
    let rect = face.glyph_bounding_box(ttf_parser::GlyphId(1)).unwrap();
    assert_eq!((rect.x_max, rect.y_max), (150, 150));
    assert_eq!(
        face.tables().cff.unwrap().glyph_cid(ttf_parser::GlyphId(1)),
        Some(1)
    );
}

#[test]
fn cancelled_outline_components_rebase_head_hmtx_and_hhea_from_emitted_geometry() {
    let left = square(0., 0., 100.);
    let source = font_for_path(
        &join(&[
            left.clone(),
            left.reverse_subpaths(),
            square(200., 0., 100.),
        ]),
        false,
    );
    let output = font_instance::prepare_font_instance(&source, &request(&source, false)).unwrap();
    let face = ttf_parser::Face::parse(&output.bytes, 0).unwrap();
    assert_eq!(
        face.glyph_bounding_box(ttf_parser::GlyphId(1))
            .unwrap()
            .x_min,
        200
    );
    assert_eq!(
        face.glyph_hor_side_bearing(ttf_parser::GlyphId(1)),
        Some(200)
    );
    let tables = font_instance::tests::tables(&output.bytes);
    assert_eq!(
        i16::from_be_bytes(tables[b"head"][36..38].try_into().unwrap()),
        200
    );
    // A geometry change did not change the advance chosen before normalization.
    assert_eq!(face.glyph_hor_advance(ttf_parser::GlyphId(1)), Some(600));
}

#[test]
fn normalized_metric_rebase_does_not_apply_mvar_twice() {
    let source = source_font(false);
    let mut tables = font_instance::tests::tables(&source);
    tables.insert(
        *b"fvar",
        font_instance::tests::tables(&font_instance::tests::source())[b"fvar"].clone(),
    );
    let mut mvar = vec![0, 1, 0, 0, 0, 0, 0, 8, 0, 1, 0, 20];
    mvar.extend(b"undo");
    mvar.extend([0; 4]);
    for n in [1u16, 0, 12, 1, 0, 22, 1, 1, 0, 16384, 16384, 1, 1, 1, 0, 20] {
        mvar.extend(n.to_be_bytes());
    }
    tables.insert(*b"MVAR", mvar);
    let before = i16::from_be_bytes(tables[b"post"][8..10].try_into().unwrap());
    let source = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap();
    let mut r = request(&source, false);
    r.coordinates.insert("TEST".into(), 0.5);
    let output = font_instance::prepare_font_instance(&source, &r).unwrap();
    let saved = font_instance::tests::tables(&output.bytes);
    assert_eq!(
        i16::from_be_bytes(saved[b"post"][8..10].try_into().unwrap()),
        before + 10
    );
}
#[test]
fn hinted_overlap_requires_explicit_normalization_but_compatible_default_keeps_hints() {
    let source = source_font(true);
    let before = source.clone();
    assert!(font_instance::prepare_font_instance(&source, &request(&source, false)).is_err());
    let output = font_instance::prepare_font_instance(&source, &request(&source, true)).unwrap();
    let report = output.report.cff2.unwrap();
    assert_eq!((report.stem_hints, report.masks), (0, 0));
    assert_eq!(report.contour_normalization.unwrap().dehinted_glyphs, [1]);
    let mut r = request(&source, false);
    r.cff2_contours = None;
    let error = font_instance::prepare_font_instance(&source, &r)
        .err()
        .unwrap();
    assert!(error
        .to_string()
        .contains("enable cff2_contours normalization"));
    let compatible = compatible_source_font();
    r = request(&compatible, false);
    r.cff2_contours = None;
    let kept = font_instance::prepare_font_instance(&compatible, &r)
        .unwrap()
        .report
        .cff2
        .unwrap();
    assert_eq!((kept.stem_hints, kept.masks), (1, 1));
    assert!(kept.contour_overlaps_checked);
    assert!(!kept.contour_overlaps_removed);
    let check = kept.preserved_contour_check.unwrap();
    assert_eq!(check.exact_linear_preserved_glyphs, [1]);
    assert!(check.normalized_glyphs.is_empty());
    assert_eq!(source, before);
}
#[test]
fn hint_only_empty_glyph_and_private_owner_are_preserved_without_loss_consent() {
    let mut bytes = fixtures::fixture(
        &[vec![], vec![139, 159, 18], vec![], vec![]],
        &[vec![]],
        &[None],
        &[0; 4],
        0,
        None,
        &[],
    );
    let program = Program::parse(&bytes).unwrap();
    let fd = program.font_dicts[0].clone();
    let at = bytes.len();
    bytes[fd.start + 1..fd.start + 5].copy_from_slice(&2u32.to_be_bytes());
    bytes[fd.start + 6..fd.start + 10].copy_from_slice(&(at as u32).to_be_bytes());
    bytes.extend([159, 10]);
    let program = Program::parse(&bytes).unwrap();
    let opt = options();
    let stage = freeze_with_contours(
        &program,
        &[],
        &[600; 4],
        [0; 4],
        "RetainedHints",
        1 << 20,
        Some(&opt),
    )
    .unwrap();
    let report = stage.report.contour_normalization.unwrap();
    assert_eq!(report.private_hint_dictionaries_removed, 0);
    assert_eq!(report.private_hint_dictionaries_retained, 1);
    assert_eq!(report.private_hint_dictionaries_bypassed, 0);
    assert_eq!(report.preserved_glyphs, [0, 1, 2, 3]);
    assert_eq!(report.preserved_hint_glyphs, [0, 1, 2, 3]);
    assert!(report.dehinted_glyphs.is_empty());
    let (glyphs, private) = instance_tests::parts(&stage.bytes);
    assert_eq!(instance_tests::unpack(glyphs[1])[0].0, 18);
    assert_eq!(instance_tests::fields(private[0])[&10], [20.]);
    let preserved = freeze_with_contours(
        &program,
        &[],
        &[600; 4],
        [0; 4],
        "RetainedHints",
        1 << 20,
        None,
    )
    .unwrap();
    assert_eq!(stage.bytes, preserved.bytes);
    assert!(preserved.report.contour_overlaps_checked);
    let check = preserved.report.preserved_contour_check.unwrap();
    assert_eq!(check.preserved_glyphs, [0, 1, 2, 3]);
    assert!(check.normalized_glyphs.is_empty());
}

#[test]
fn default_preservation_rejects_overlaps_before_publication_without_hint_consent_question() {
    for hinted in [false, true] {
        let source = source_font(hinted);
        let mut req = request(&source, false);
        req.cff2_contours = None;
        let before = source.clone();
        let error = font_instance::prepare_font_instance(&source, &req)
            .err()
            .unwrap();
        let message = error.to_string();
        assert!(message.contains("glyph 1"));
        assert!(message.contains("enable cff2_contours normalization"));
        assert!(!message.contains("explicit hint-loss approval"));
        assert_eq!(source, before);
    }
}

#[test]
fn default_preservation_checks_curved_and_flat_flex_programs_without_rewriting() {
    let curve = kurbo::Circle::new((0., 0.), 100.).to_path(0.01);
    let source = font_for_path(&curve, true);
    let mut req = request(&source, false);
    req.cff2_contours = None;
    let saved = font_instance::prepare_font_instance(&source, &req).unwrap();
    let cff = saved.report.cff2.unwrap();
    assert!(cff.contour_overlaps_checked);
    let check = cff.preserved_contour_check.unwrap();
    assert_eq!(check.preserved_hint_glyphs, [1]);
    assert!(check.exact_linear_preserved_glyphs.is_empty());
    assert!(check.broadphase_pairs > 0);

    let mut code = Vec::new();
    charstring::encode_number(&mut code, -32168.).unwrap();
    code.extend([139, 139, 21]);
    for value in [10., 0., 10., 0., 10., 0., -10., 0., -10., 0., -10., 0., 50.] {
        charstring::encode_number(&mut code, value).unwrap();
    }
    code.extend([12, 35, 14]);
    let mut check = ContourReport::preserving_check();
    verify_preserved(&code, 8, &mut check, true).unwrap();
    assert_eq!(check.exact_linear_preserved_glyphs, [8]);
    assert_eq!(check.preserved_hint_glyphs, [8]);
    assert_eq!(check.broadphase_pairs, 0);
}

#[test]
fn older_cff_report_without_preservation_receipt_still_deserializes_as_unproven() {
    let source = compatible_source_font();
    let mut req = request(&source, false);
    req.cff2_contours = None;
    let report = font_instance::prepare_font_instance(&source, &req)
        .unwrap()
        .report
        .cff2
        .unwrap();
    let mut value = serde_json::to_value(report).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("preserved_contour_check");
    let restored: super::super::CffReport = serde_json::from_value(value).unwrap();
    assert!(restored.preserved_contour_check.is_none());
}

#[test]
fn compatible_boundaries_preserve_exact_glyph_programs_in_either_orientation() {
    let a = square(0., 0., 100.);
    for path in [
        a.clone(),
        a.reverse_subpaths(),
        join(&[a, square(200., 0., 100.).reverse_subpaths()]),
    ] {
        let source = font_for_path(&path, true);
        let mut req = request(&source, false);
        let normalized = font_instance::prepare_font_instance(&source, &req).unwrap();
        let report = normalized.report.cff2.as_ref().unwrap();
        assert_eq!((report.stem_hints, report.masks), (1, 1));
        let contour = report.contour_normalization.as_ref().unwrap();
        assert_eq!(contour.preserved_glyphs, [0, 1, 2, 3]);
        assert_eq!(contour.preserved_hint_glyphs, [1]);
        assert_eq!(contour.exact_linear_preserved_glyphs, [1]);
        assert!(contour.normalized_glyphs.is_empty());
        assert!(contour.dehinted_glyphs.is_empty());
        req.cff2_contours = None;
        let original = font_instance::prepare_font_instance(&source, &req).unwrap();
        let before = font_instance::tests::tables(&original.bytes);
        let after = font_instance::tests::tables(&normalized.bytes);
        let (before_glyphs, before_private) = instance_tests::parts(&before[b"CFF "]);
        let (after_glyphs, after_private) = instance_tests::parts(&after[b"CFF "]);
        assert_eq!(after_glyphs, before_glyphs);
        assert_eq!(after_private, before_private);
    }
}

#[test]
fn whole_font_keeps_quantum_separated_hinted_outlines_at_large_tolerance() {
    let source = font_for_path(
        &join(&[square(0., 0., 100.), square(100. + QUANTUM, 0., 100.)]),
        true,
    );
    let mut req = request(&source, false);
    req.cff2_contours.as_mut().unwrap().tolerance_font_units = 0.125;
    let after = font_instance::prepare_font_instance(&source, &req).unwrap();
    let cff = after.report.cff2.as_ref().unwrap();
    assert_eq!((cff.stem_hints, cff.masks), (1, 1));
    let report = cff.contour_normalization.as_ref().unwrap();
    assert_eq!(report.exact_linear_preserved_glyphs, [1]);
    assert_eq!(report.broadphase_pairs, 0);
    req.cff2_contours = None;
    let before = font_instance::prepare_font_instance(&source, &req).unwrap();
    let before = font_instance::tests::tables(&before.bytes);
    let after = font_instance::tests::tables(&after.bytes);
    let (before_glyphs, before_private) = instance_tests::parts(&before[b"CFF "]);
    let (after_glyphs, after_private) = instance_tests::parts(&after[b"CFF "]);
    assert_eq!(after_glyphs, before_glyphs);
    assert_eq!(after_private, before_private);
}

#[test]
fn shared_private_hints_are_retained_only_on_unchanged_glyphs() {
    let a = square(0., 0., 100.);
    let overlap = join(&[a.clone(), square(50., 50., 100.)]);
    let bytes = instance_tests::with_privates(
        fixtures::fixture(
            &[
                vec![],
                source_program(&a, true),
                source_program(&overlap, true),
                vec![],
            ],
            &[vec![]],
            &[None],
            &[0; 4],
            0,
            None,
            &[],
        ),
        &[vec![159, 10]],
    );
    let program = Program::parse(&bytes).unwrap();
    let opt = options();
    let rejected = freeze_with_contours(
        &program,
        &[],
        &[600; 4],
        [0; 4],
        "MixedHints",
        1 << 20,
        Some(&opt),
    );
    assert!(rejected.err().unwrap().to_string().contains("glyph 2"));
    let opt = ContourNormalization {
        allow_hint_loss: true,
        ..opt
    };
    let saved = freeze_with_contours(
        &program,
        &[],
        &[600; 4],
        [0; 4],
        "MixedHints",
        1 << 20,
        Some(&opt),
    )
    .unwrap();
    // Compare retained glyphs with a compatible baseline carrying the same
    // private dictionary. The strict preservation path must reject glyph 2's
    // known overlap rather than publishing it as checked unchanged content.
    let baseline_bytes = instance_tests::with_privates(
        fixtures::fixture(
            &[vec![], source_program(&a, true), vec![], vec![]],
            &[vec![]],
            &[None],
            &[0; 4],
            0,
            None,
            &[],
        ),
        &[vec![159, 10]],
    );
    let baseline_program = Program::parse(&baseline_bytes).unwrap();
    let before = freeze_with_contours(
        &baseline_program,
        &[],
        &[600; 4],
        [0; 4],
        "MixedHints",
        1 << 20,
        None,
    )
    .unwrap();
    let report = saved.report.contour_normalization.as_ref().unwrap();
    assert_eq!(report.preserved_glyphs, [0, 1, 3]);
    assert_eq!(report.preserved_hint_glyphs, [0, 1, 3]);
    assert_eq!(report.normalized_glyphs, [2]);
    assert_eq!(report.dehinted_glyphs, [2]);
    assert_eq!(report.private_hint_dictionaries_retained, 1);
    assert_eq!(report.private_hint_dictionaries_bypassed, 1);
    assert_eq!(report.private_hint_dictionaries_removed, 0);
    assert_eq!(saved.report.output_font_dicts, 2);
    assert_eq!((saved.report.stem_hints, saved.report.masks), (1, 1));
    let (glyphs, private) = instance_tests::parts(&saved.bytes);
    let (old_glyphs, old_private) = instance_tests::parts(&before.bytes);
    for gid in [0, 1, 3] {
        assert_eq!(glyphs[gid], old_glyphs[gid]);
    }
    assert_eq!(private[0], old_private[0]);
    assert_eq!(instance_tests::fields(private[1]).len(), 2);
    assert!(instance_tests::unpack(glyphs[2])
        .iter()
        .all(|(op, _, _)| !matches!(op, 1 | 3 | 18 | 19 | 20 | 23)));
    let (_, at) = instance_tests::read_index(&saved.bytes, 4);
    let (top, _) = instance_tests::read_index(&saved.bytes, at);
    let select = instance_tests::fields(top[0])[&0x0c25][0] as usize;
    assert_eq!(&saved.bytes[select..select + 5], &[0, 0, 0, 1, 0]);
}

#[test]
fn merged_cancelled_and_touching_edges_cannot_bypass_hint_loss_consent() {
    let a = square(0., 0., 100.);
    for path in [
        join(&[a.clone(), a.clone()]),
        join(&[a.clone(), a.reverse_subpaths()]),
        join(&[a, square(100., 100., 100.)]),
        BezPath::from_vec(vec![
            PathEl::MoveTo((0., 0.).into()),
            PathEl::LineTo((100., 100.).into()),
            PathEl::LineTo((0., 100.).into()),
            PathEl::LineTo((100., 0.).into()),
            PathEl::ClosePath,
        ]),
    ] {
        let code = encode(&path, -32168.).unwrap();
        let opt = options();
        let error =
            normalize_if_needed(&code, -32168., 9, &opt, &mut ContourReport::new(&opt), true)
                .err()
                .unwrap();
        assert!(error.to_string().contains("glyph 9"));
    }
}

#[test]
fn compatible_flex_keeps_its_original_operator_and_depth() {
    let mut code = Vec::new();
    charstring::encode_number(&mut code, -32168.).unwrap();
    code.extend([139, 139, 21]);
    // Two shallow curves above their closing baseline, no crossing or overlap.
    for value in [10., 1., 10., 0., 10., 0., 10., 0., 10., 0., 10., -1., 50.] {
        charstring::encode_number(&mut code, value).unwrap();
    }
    code.extend([12, 35, 14]);
    let opt = options();
    let mut report = ContourReport::new(&opt);
    let result = normalize_if_needed(&code, -32168., 1, &opt, &mut report, true).unwrap();
    assert!(!result.rewritten);
    assert_eq!(result.bytes, code);
    assert_eq!(report.preserved_hint_glyphs, [1]);
}

#[test]
fn compatible_glyphs_still_obey_aggregate_output_budget() {
    let code = encode(&square(0., 0., 100.), -32168.).unwrap();
    let opt = options();
    let mut report = ContourReport::new(&opt);
    report.output_segments = MAX_TOTAL;
    assert!(normalize_if_needed(&code, -32168., 1, &opt, &mut report, true).is_err());
}

#[test]
fn final_dictionary_capacity_depends_on_glyph_decisions_not_source_fd_count() {
    let count = 257;
    let simple = source_program(&square(0., 0., 100.), false);
    let overlap = source_program(
        &join(&[square(0., 0., 100.), square(50., 50., 100.)]),
        false,
    );
    let private = (0..count)
        .map(|i| {
            let mut bytes = Vec::new();
            super::super::entry(&mut bytes, 10, &[(i + 1) as f64]).unwrap();
            bytes
        })
        .collect::<Vec<_>>();
    let opt = ContourNormalization {
        allow_hint_loss: true,
        ..options()
    };
    for should_rewrite in [false, true] {
        let mut glyphs = vec![
            if should_rewrite {
                overlap.clone()
            } else {
                simple.clone()
            };
            count
        ];
        glyphs[0].clear(); // retain FD 0; every other owner has independent semantics
        let bytes = instance_tests::with_privates(
            fixtures::fixture(
                &glyphs,
                &vec![vec![]; count],
                &vec![None; count],
                &(0..count).collect::<Vec<_>>(),
                4,
                None,
                &[],
            ),
            &private,
        );
        let result = freeze_with_contours(
            &Program::parse(&bytes).unwrap(),
            &[],
            &vec![600; count],
            [0; 4],
            "LargeFDs",
            1 << 20,
            Some(&opt),
        );
        if should_rewrite {
            let stage = result.unwrap();
            assert_eq!(stage.report.output_font_dicts, 2);
            let report = stage.report.contour_normalization.unwrap();
            assert_eq!(report.private_hint_dictionaries_removed, 256);
            assert_eq!(report.private_hint_dictionaries_retained, 1);
            assert_eq!(report.private_hint_dictionaries_bypassed, 256);
            assert_eq!(report.preserved_glyphs, [0]);
            assert_eq!(report.normalized_glyphs.len(), 256);
        } else {
            assert!(result
                .err()
                .unwrap()
                .to_string()
                .contains("after contour decisions"));
        }
    }
}

#[test]
fn older_contour_receipts_deserialize_without_fabricating_preservation_evidence() {
    let mut value = serde_json::to_value(ContourReport::new(&options())).unwrap();
    let fields = value.as_object_mut().unwrap();
    for key in [
        "preserved_glyphs",
        "preserved_hint_glyphs",
        "private_hint_dictionaries_bypassed",
        "private_hint_dictionaries_retained",
        "exact_linear_preserved_glyphs",
        "exact_linear_work",
    ] {
        fields.remove(key);
    }
    let report: ContourReport = serde_json::from_value(value).unwrap();
    assert!(report.preserved_glyphs.is_empty());
    assert!(report.preserved_hint_glyphs.is_empty());
    assert_eq!(report.private_hint_dictionaries_bypassed, 0);
    assert_eq!(report.private_hint_dictionaries_retained, 0);
    assert!(report.exact_linear_preserved_glyphs.is_empty());
    assert_eq!(report.exact_linear_work, 0);
    assert!(!report.independently_verified);
}
#[test]
fn normalization_choices_affect_asset_identity_and_do_not_apply_to_truetype() {
    // Asset-identity comparison needs two valid publication paths. A known
    // overlapping outline is deliberately rejected when normalization is off.
    let source = compatible_source_font();
    let r = request(&source, false);
    let normalized = font_instance::prepare_font_instance(&source, &r).unwrap();
    let mut r = r;
    r.cff2_contours = None;
    let preserved = font_instance::prepare_font_instance(&source, &r).unwrap();
    assert_ne!(
        font_instance::tests::tables(&normalized.bytes)[b"name"],
        font_instance::tests::tables(&preserved.bytes)[b"name"]
    );
    let tt = font_instance::tests::source();
    let mut r = font_instance::tests::request(&tt);
    r.cff2_contours = Some(options());
    assert!(font_instance::prepare_font_instance(&tt, &r).is_err());
}
