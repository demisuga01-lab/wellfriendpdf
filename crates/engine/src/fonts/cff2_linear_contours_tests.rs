//! Regression source only. No font/geometry/PDF execution during this change.
use super::super::{encode, normalize_if_needed, ContourNormalization};
use super::*;
use kurbo::Shape;

fn options(tolerance: f64) -> ContourNormalization {
    ContourNormalization {
        tolerance_font_units: tolerance,
        allow_hint_loss: false,
    }
}
fn square(x: f64, y: f64, size: f64) -> BezPath {
    kurbo::Rect::new(x, y, x + size, y + size).to_path(0.)
}
fn join(paths: &[BezPath]) -> BezPath {
    let mut result = BezPath::new();
    for path in paths {
        result.extend(path.iter());
    }
    result
}
fn classify(path: &BezPath) -> Option<bool> {
    compatible(path, &mut ContourReport::new(&options(0.125))).unwrap()
}
fn polygon(points: &[(f64, f64)]) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for &point in &points[1..] {
        path.line_to(point);
    }
    path.close_path();
    path
}

#[test]
fn determinant_sign_is_exact_when_large_products_differ_by_one() {
    let n = 1i64 << 36;
    let p = FixedPoint { x: 0, y: 0 };
    let q = FixedPoint { x: n, y: n - 1 };
    let r = FixedPoint { x: n - 1, y: n - 2 };
    assert_eq!(cross(p, q, r), -1);
    assert_eq!(cross(p, r, q), 1);
    let p = FixedPoint { x: -n, y: -n };
    let q = FixedPoint { x: n, y: -n };
    let r = FixedPoint { x: -n, y: n };
    assert_eq!(cross(p, q, r), 1i128 << 74);
}

#[test]
fn disjoint_rings_in_either_orientation_are_compatible() {
    let a = square(0., 0., 100.);
    let b = square(200., 0., 100.);
    for left in [a.clone(), a.reverse_subpaths()] {
        for right in [b.clone(), b.reverse_subpaths()] {
            assert_eq!(classify(&join(&[left.clone(), right])), Some(true));
        }
    }
}

#[test]
fn nested_rings_require_zero_unit_winding_transitions() {
    for a in [false, true] {
        for b in [false, true] {
            for c in [false, true] {
                let mut paths = vec![
                    square(0., 0., 100.),
                    square(10., 10., 80.),
                    square(20., 20., 60.),
                ];
                for (path, reversed) in paths.iter_mut().zip([a, b, c]) {
                    if reversed {
                        *path = path.reverse_subpaths();
                    }
                }
                // The middle ring must cancel the outer ring. Once the
                // winding is zero inside that hole, either orientation of the
                // innermost island is a valid zero-to-unit transition.
                assert_eq!(classify(&join(&paths)), Some(a != b));
            }
        }
    }
}

#[test]
fn same_winding_nested_ink_and_coincident_cancelled_edges_require_rewrite() {
    let a = square(0., 0., 100.);
    for path in [
        join(&[a.clone(), square(10., 10., 80.)]),
        join(&[a.clone(), a.clone()]),
        join(&[a.clone(), a.reverse_subpaths()]),
    ] {
        assert_eq!(classify(&path), Some(false));
    }
}

#[test]
fn crossings_vertex_touches_t_junctions_and_backtracking_do_not_pass() {
    for path in [
        join(&[square(0., 0., 100.), square(100., 100., 100.)]),
        join(&[
            square(0., 0., 100.),
            polygon(&[(50., 100.), (40., 150.), (60., 150.)]),
        ]),
        polygon(&[(0., 0.), (100., 100.), (0., 100.), (100., 0.)]),
        polygon(&[(0., 0.), (100., 0.), (50., 0.), (50., 100.), (0., 100.)]),
        join(&[
            square(0., 0., 100.),
            polygon(&[(0., 50.), (50., 10.), (50., 90.)]),
        ]),
    ] {
        assert_eq!(classify(&path), Some(false));
    }
}

#[test]
fn collinear_forward_vertices_and_duplicate_points_preserve_the_source() {
    let path = polygon(&[
        (0., 0.),
        (0., 0.),
        (50., 0.),
        (100., 0.),
        (100., 100.),
        (0., 100.),
        (0., 0.),
    ]);
    assert_eq!(classify(&path), Some(true));
    let code = encode(&path, -32168.).unwrap();
    let opt = options(0.125);
    let mut report = ContourReport::new(&opt);
    let output = normalize_if_needed(&code, -32168., 3, &opt, &mut report, true).unwrap();
    assert_eq!(output.bytes, code);
    assert!(!output.rewritten);
    assert_eq!(report.exact_linear_preserved_glyphs, [3]);
    assert_eq!(report.broadphase_pairs, 0);
}

#[test]
fn one_quantum_gaps_are_preserved_independently_of_requested_tolerance() {
    let path = join(&[square(0., 0., 100.), square(100. + QUANTUM, 0., 100.)]);
    let code = encode(&path, -32168.).unwrap();
    for tolerance in [QUANTUM, 0.001, 0.125] {
        let opt = options(tolerance);
        let mut report = ContourReport::new(&opt);
        let output = normalize_if_needed(&code, -32168., 3, &opt, &mut report, true).unwrap();
        assert_eq!(output.bytes, code);
        assert!(!output.rewritten);
        assert_eq!(report.exact_linear_preserved_glyphs, [3]);
        assert_eq!(report.preserved_hint_glyphs, [3]);
        assert!(report.exact_linear_work > 0);
        assert_eq!(report.broadphase_pairs, 0);
        assert_eq!(report.potential_intersections, 0);
        assert!(!report.independently_verified);
    }
}

#[test]
fn one_quantum_overlap_cannot_be_overridden_by_tolerant_topology() {
    let path = join(&[square(0., 0., 100.), square(100. - QUANTUM, 0., 100.)]);
    let code = encode(&path, -32168.).unwrap();
    let opt = options(0.125);
    let mut report = ContourReport::new(&opt);
    let error = normalize_if_needed(&code, -32168., 7, &opt, &mut report, true)
        .err()
        .unwrap();
    assert!(error.to_string().contains("glyph 7"));
    assert_eq!(report.broadphase_pairs, 0);
    assert!(report.preserved_glyphs.is_empty());
    assert!(report.normalized_glyphs.is_empty());
}

#[test]
fn exhaustive_rectangle_contact_grid_matches_interval_oracle() {
    for x in -3..=3 {
        for y in -3..=3 {
            for a in [false, true] {
                for b in [false, true] {
                    let mut left = square(0., 0., 2.);
                    let mut right = square(f64::from(x), f64::from(y), 2.);
                    if a {
                        left = left.reverse_subpaths();
                    }
                    if b {
                        right = right.reverse_subpaths();
                    }
                    let separated = x > 2 || x + 2 < 0 || y > 2 || y + 2 < 0;
                    assert_eq!(
                        classify(&join(&[left, right])),
                        Some(separated),
                        "{x},{y},{a},{b}"
                    );
                }
            }
        }
    }
}

#[test]
fn reflection_translation_and_extreme_coordinate_domains_preserve_decisions() {
    for scale in [-1., 1.] {
        for shift in [-1_048_000., 0., 1_048_000.] {
            let path = polygon(&[
                (shift, shift),
                (shift + 100. * scale, shift),
                (shift + 100. * scale, shift + 100.),
                (shift, shift + 100.),
            ]);
            assert_eq!(classify(&path), Some(true));
        }
    }
    assert_eq!(
        classify(&polygon(&[
            (-1_048_576., -1_048_576.),
            (1_048_576., -1_048_576.),
            (1_048_576., 1_048_576.),
            (-1_048_576., 1_048_576.)
        ])),
        Some(true)
    );
}

#[test]
fn unsupported_curves_route_to_general_solver_without_claiming_exactness() {
    let curve = kurbo::Circle::new((0., 0.), 100.).to_path(0.01);
    assert_eq!(classify(&curve), None);
    let mixed = join(&[square(200., 0., 100.), curve]);
    assert_eq!(classify(&mixed), None);
}

#[test]
fn invalid_fixed_domain_and_open_paths_fail_instead_of_rounding() {
    for path in [
        square(QUANTUM / 2., 0., 100.),
        square(1_048_577., 0., 100.),
        polygon(&[(0., 0.), (f64::INFINITY, 0.), (0., 100.)]),
        BezPath::from_vec(vec![
            PathEl::MoveTo((0., 0.).into()),
            PathEl::LineTo((1., 1.).into()),
        ]),
    ] {
        assert!(compatible(&path, &mut ContourReport::new(&options(0.001))).is_err());
    }
    // A closed collinear contour has zero ink and can be preserved exactly.
    assert_eq!(
        classify(&polygon(&[(0., 0.), (1., 0.), (2., 0.)])),
        Some(true)
    );
}

#[test]
fn exact_work_and_path_capacity_are_bounded_and_cancelled() {
    let path = square(0., 0., 100.);
    let mut report = ContourReport::new(&options(0.001));
    report.exact_linear_work = MAX_WORK;
    assert!(compatible(&path, &mut report).is_err());
    report.exact_linear_work = usize::MAX;
    assert!(compatible(&path, &mut report).is_err());
    let mut huge = BezPath::new();
    for _ in 0..MAX_OUTPUT + 1 {
        huge.move_to((0., 0.));
        huge.close_path();
    }
    assert!(compatible(&huge, &mut ContourReport::new(&options(0.001))).is_err());
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    cancel.scope(|| {
        assert!(compatible(&path, &mut ContourReport::new(&options(0.001))).is_err());
    });
}
