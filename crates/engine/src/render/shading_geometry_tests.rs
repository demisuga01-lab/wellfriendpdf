//! Unexecuted analytic regression source; not compiler or rendering evidence.
use super::*;

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 2e-13 * expected.abs().max(1.0),
        "{actual} != {expected}"
    );
}

#[test]
fn axial_parameter_is_invariant_under_tiny_and_large_units() {
    for scale in [1e-200, 1e-8, 1.0, 1e200] {
        let geometry = AxialGeometry::new([0.0, 0.0, 10.0 * scale, 0.0]).unwrap();
        close(
            geometry
                .parameter((2.5 * scale, 987.0 * scale), [false; 2])
                .unwrap()
                .unwrap(),
            0.25,
        );
        assert_eq!(geometry.parameter((-scale, 0.0), [false; 2]).unwrap(), None);
        assert_eq!(
            geometry.parameter((-scale, 0.0), [true, false]).unwrap(),
            Some(0.0)
        );
        assert_eq!(
            geometry
                .parameter((11.0 * scale, 0.0), [false, true])
                .unwrap(),
            Some(1.0)
        );
    }
}

#[test]
fn axial_projection_handles_overflowing_differences_and_distant_perpendicular_samples() {
    let geometry = AxialGeometry::new([-f64::MAX, 0.0, f64::MAX, 0.0]).unwrap();
    close(
        geometry.parameter((0.0, 3.0), [false; 2]).unwrap().unwrap(),
        0.5,
    );
    let geometry = AxialGeometry::new([0.0, 0.0, 1e-100, 0.0]).unwrap();
    assert_eq!(
        geometry.parameter((0.0, 1e300), [false; 2]).unwrap(),
        Some(0.0)
    );
    assert_eq!(geometry.parameter((1e300, 0.0), [false; 2]).unwrap(), None);
    assert_eq!(
        geometry.parameter((1e300, 0.0), [false, true]).unwrap(),
        Some(1.0)
    );
}

#[test]
fn axial_translation_and_rotation_preserve_projection() {
    let geometry = AxialGeometry::new([1234.0, -901.0, 1237.0, -897.0]).unwrap();
    // Halfway along the (3,4) axis, displaced by a perpendicular (-4,3).
    close(
        geometry
            .parameter((1231.5, -896.0), [false; 2])
            .unwrap()
            .unwrap(),
        0.5,
    );
    let empty = AxialGeometry::new([7.0, 8.0, 7.0, 8.0]).unwrap();
    assert_eq!(empty.parameter((7.0, 8.0), [true; 2]).unwrap(), None);
}

#[test]
fn radial_expansion_contraction_and_extensions_are_unit_invariant() {
    for scale in [1e-200, 1e-8, 1.0, 1e200] {
        for (first, last) in [(1.0, 3.0), (3.0, 1.0)] {
            let geometry =
                RadialGeometry::new([0.0, 0.0, first * scale, 0.0, 0.0, last * scale]).unwrap();
            close(
                geometry
                    .parameter((2.0 * scale, 0.0), [false; 2])
                    .unwrap()
                    .unwrap(),
                0.5,
            );
            assert_eq!(
                geometry.parameter((4.0 * scale, 0.0), [false; 2]).unwrap(),
                None
            );
            assert_eq!(geometry.parameter((0.0, 0.0), [false; 2]).unwrap(), None);
            let inner = if first < last { 0.0 } else { 1.0 };
            let outer = 1.0 - inner;
            close(
                geometry.parameter((0.0, 0.0), [true; 2]).unwrap().unwrap(),
                inner,
            );
            close(
                geometry
                    .parameter((4.0 * scale, 0.0), [true; 2])
                    .unwrap()
                    .unwrap(),
                outer,
            );
        }
    }
}

#[test]
fn radial_later_valid_circle_wins_and_negative_radius_is_rejected() {
    // Equal radii moving horizontally: x=1 lies on circles at s=.25 and .75.
    let geometry = RadialGeometry::new([0.0, 0.0, 0.5, 2.0, 0.0, 0.5]).unwrap();
    close(
        geometry.parameter((1.0, 0.0), [false; 2]).unwrap().unwrap(),
        0.75,
    );
    for scale in [1e-200, 1e-8, 1e200] {
        let scaled =
            RadialGeometry::new([0.0, 0.0, 0.5 * scale, 2.0 * scale, 0.0, 0.5 * scale]).unwrap();
        close(
            scaled.parameter((scale, 0.0), [false; 2]).unwrap().unwrap(),
            0.75,
        );
    }
    // Shrinking concentric circles produce s=.25 and s=1.75 after squaring;
    // the latter is forbidden because its radius is negative, even if extended.
    let geometry = RadialGeometry::new([0.0, 0.0, 2.0, 0.0, 0.0, 0.0]).unwrap();
    close(
        geometry.parameter((1.5, 0.0), [true; 2]).unwrap().unwrap(),
        0.25,
    );
}

#[test]
fn radial_linear_tangent_and_common_generator_cases_are_explicit() {
    let geometry = RadialGeometry::new([0.0, 0.0, 0.0, 1.0, 0.0, 1.0]).unwrap();
    close(
        geometry.parameter((1.0, 0.0), [false; 2]).unwrap().unwrap(),
        0.5,
    );
    assert_eq!(
        geometry.parameter((0.0, 0.0), [false; 2]).unwrap(),
        Some(1.0)
    );
    let tangent = RadialGeometry::new([0.0, 0.0, 1.0, 2.0, 0.0, 1.0]).unwrap();
    close(
        tangent.parameter((1.0, 1.0), [false; 2]).unwrap().unwrap(),
        0.5,
    );
    assert_eq!(tangent.parameter((1.0, 1.001), [true; 2]).unwrap(), None);
    let empty = RadialGeometry::new([0.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap();
    assert_eq!(empty.parameter((1.0, 0.0), [true; 2]).unwrap(), None);
}

#[test]
fn stable_quadratic_retains_the_small_root() {
    let mut found: Vec<f64> = roots(1.0, 1.0, 1e-20).into_iter().flatten().collect();
    found.sort_by(f64::total_cmp);
    assert!((found[0] / 5e-21 - 1.0).abs() < 1e-14);
    close(found[1], 2.0);
    assert_eq!(roots(1.0, 0.0, 0.0), [Some(0.0), None]);
    assert_eq!(roots(1.0, 0.0, 1.0), [None, None]);
    close(roots(0.0, 0.5, 0.25)[0].unwrap(), 0.25);
    assert_eq!(roots(0.0, 0.0, 1.0), [None, None]);
}

#[test]
fn domain_mapping_preserves_endpoints_without_overflow() {
    let max = f64::MAX;
    assert_eq!(domain_value(-max, max, 0.0), -max);
    assert_eq!(domain_value(-max, max, 1.0), max);
    assert_eq!(domain_value(-max, max, 0.5), 0.0);
    close(domain_value(-max, max, 0.75) / max, 0.5);
    close(domain_value(max, max * 0.5, 0.5) / max, 0.75);
    assert_eq!(
        domain_value(f64::from_bits(1), f64::from_bits(1), 0.5),
        f64::from_bits(1)
    );
}

#[test]
fn analytic_inverse_preserves_scale_reflection_and_translation() {
    for scale in [1e-200, 1e-8, 1.0, 1e200] {
        let transform = Transform2D::from([
            2.0 * scale,
            scale,
            scale,
            -3.0 * scale,
            4.0 * scale,
            -5.0 * scale,
        ]);
        let result = inverse(&transform).unwrap().transform_point(
            transform.transform_point(3.0, 2.0).0,
            transform.transform_point(3.0, 2.0).1,
        );
        close(result.0, 3.0);
        close(result.1, 2.0);
    }
    let transform = Transform2D::scale(1e-200, 1e200);
    let result = inverse(&transform).unwrap().transform_point(1e-200, 1e200);
    close(result.0, 1.0);
    close(result.1, 1.0);
}

#[test]
fn invalid_inputs_and_nonrepresentable_inverses_do_not_become_colours() {
    assert!(AxialGeometry::new([0.0, 0.0, f64::INFINITY, 1.0]).is_err());
    assert!(RadialGeometry::new([0.0, 0.0, -1.0, 0.0, 0.0, 1.0]).is_err());
    let geometry = RadialGeometry::new([0.0, 0.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
    assert!(geometry.parameter((f64::NAN, 0.0), [true; 2]).is_err());
    assert!(inverse(&Transform2D::scale(0.0, 1.0)).is_none());
    assert!(inverse(&Transform2D::from([1.0, 2.0, 2.0, 4.0, 0.0, 0.0])).is_none());
    assert!(inverse(&Transform2D::scale(f64::from_bits(1), 1.0)).is_none());
    let disproportionate = RadialGeometry::new([0.0, 0.0, 1.0, 1e-200, 0.0, 1.0]).unwrap();
    assert!(disproportionate.parameter((0.0, 1.0), [true; 2]).is_err());
}
