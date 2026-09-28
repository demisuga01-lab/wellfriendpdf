//! Regression source only: not executed as part of this implementation.
use super::*;

fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Net {
    std::array::from_fn(|i| {
        std::array::from_fn(|j| (x + w * i as f64 / 3.0, y + h * j as f64 / 3.0))
    })
}
fn points(net: Net) -> Vec<PatchPoint> {
    vec![
        net[0][0], net[0][1], net[0][2], net[0][3], net[1][3], net[2][3], net[3][3], net[3][2],
        net[3][1], net[3][0], net[2][0], net[1][0], net[1][1], net[1][2], net[2][2], net[2][1],
    ]
}
fn colors() -> [MeshSample; 4] {
    [MeshSample::new(&[0.0]).unwrap(); 4]
}
fn bits(point: PatchPoint) -> [u64; 2] {
    [point.0.to_bits(), point.1.to_bits()]
}
fn assert_near(a: PatchPoint, b: PatchPoint, tolerance: f64) {
    assert!(
        magnitude(difference(a, b)) <= tolerance,
        "{a:?} versus {b:?}"
    );
}
fn evaluate(net: Net, u: f64, v: f64) -> PatchPoint {
    curve(net.map(|row| curve(row, v)), u)
}

#[test]
fn coons_degree_elevation_preserves_the_surface_and_boundary_controls() {
    let mut p = points(rectangle(-20.0, 5.0, 80.0, 50.0));
    p[1].0 -= 15.0;
    p[2].0 += 7.0;
    p[4].1 += 24.0;
    p[8].0 += 10.0;
    p[11].1 -= 9.0;
    let net = control_net(&p[..12], 6).unwrap();
    assert_eq!(net[0], [p[0], p[1], p[2], p[3]]);
    assert_eq!(net[3], [p[9], p[8], p[7], p[6]]);
    for iu in 0..=16 {
        for iv in 0..=16 {
            let (u, v) = (iu as f64 / 16.0, iv as f64 / 16.0);
            assert_near(evaluate(net, u, v), coons_point(&p[..12], u, v), 1e-10);
        }
    }
}

#[test]
fn tensor_net_preserves_all_sixteen_controls_and_surface() {
    let mut net = rectangle(0.0, 0.0, 70.0, 40.0);
    net[1][1].1 += 13.0;
    net[2][2].0 -= 17.0;
    let p = points(net);
    assert_eq!(control_net(&p, 7).unwrap(), net);
    for iu in 0..=16 {
        for iv in 0..=16 {
            let (u, v) = (iu as f64 / 16.0, iv as f64 / 16.0);
            assert_near(evaluate(net, u, v), tensor_point(&p, u, v), 1e-10);
        }
    }
}

#[test]
fn projected_curvature_increases_refinement_and_translation_does_not() {
    let mut net = rectangle(0.0, 0.0, 30.0, 30.0);
    net[1][1].1 += 20.0;
    let p = points(net);
    let c = colors();
    let small = Patch::new(&p, &c, 7, &Transform2D::identity()).unwrap();
    let large = Patch::new(&p, &c, 7, &Transform2D::scale(64.0, 64.0)).unwrap();
    let translated = Patch::new(&p, &c, 7, &Transform2D::translation(1000.0, -2000.0)).unwrap();
    assert!(large.steps > small.steps);
    assert_eq!(small.steps, translated.steps);
    assert!(small.steps.is_power_of_two());
    assert!(large.steps.is_power_of_two());
    assert_eq!(required_steps(&rectangle(0.0, 0.0, 30.0, 30.0)).unwrap(), 1);
}

#[test]
fn mixed_derivative_refines_a_bilinear_twist_even_with_zero_pure_curvature() {
    let net = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            let (u, v) = (i as f64 / 3.0, j as f64 / 3.0);
            (100.0 * u, 100.0 * v + 100.0 * u * v)
        })
    });
    assert!(required_steps(&net).unwrap() > 1);
}

#[test]
fn negotiated_axes_share_curved_edges_without_refining_disconnected_patches() {
    let mut a = rectangle(0.0, 0.0, 30.0, 30.0);
    a[3][1].0 += 6.0;
    a[3][2].0 -= 6.0;
    let mut b = rectangle(30.0, 0.0, 30.0, 30.0);
    b[0] = a[3];
    b[1][1].0 += 200.0;
    let mut batch = PatchBatch::new(ShadingRenderOptions::default());
    for net in [a, b, rectangle(2000.0, 2000.0, 30.0, 30.0)] {
        batch
            .push(&points(net), &colors(), 7, &Transform2D::identity())
            .unwrap();
    }
    assert!(batch.patches[1].steps > batch.patches[0].steps);
    let (axes, _memory, _) = batch.axes().unwrap();
    assert_eq!(axes[1].steps, axes[3].steps);
    assert_eq!(axes[0].steps, batch.patches[0].steps);
    assert_eq!(axes[4].steps, 1);
    assert_eq!(axes[5].steps, 1);
    let steps = axes[1].steps;
    for i in 0..=steps {
        assert_eq!(
            bits(batch.patches[0].point(axes[0].steps, i, axes[0].steps, steps)),
            bits(batch.patches[1].point(0, i, axes[2].steps, steps))
        );
    }
}

#[test]
fn all_reuse_flags_match_boundary_bits_after_axis_rotation_and_reversal() {
    let mut original = points(rectangle(-2.0, 7.0, 41.0, 37.0));
    original[4].1 += 0.3;
    original[7].0 -= 0.17;
    original[11].1 -= 0.19;
    let transform = Transform2D::from([1.2, 0.7, -0.3, 2.1, 100.3, -47.1]);
    for kind in [6, 7] {
        for flag in 1..=3 {
            let count = if kind == 6 { 12 } else { 16 };
            let old = &original[..count];
            let incoming = points(rectangle(100.0, 50.0, 30.0, 40.0));
            let (assembled, c) = assemble_patch(
                flag,
                &incoming[4..count],
                &colors()[2..],
                old,
                &colors(),
                kind,
            )
            .unwrap();
            let first = Patch::new(old, &colors(), kind, &transform).unwrap();
            let second = Patch::new(&assembled, &c, kind, &transform).unwrap();
            let boundary = flag as usize;
            assert_eq!(first.edges[boundary].key, second.edges[0].key);
            for i in 0..=64 {
                let original_index = if flag == 1 { i } else { 64 - i };
                assert_eq!(
                    bits(first.edges[boundary].point(original_index, 64)),
                    bits(second.edges[0].point(i, 64)),
                    "kind {kind}, flag {flag}, sample {i}"
                );
            }
        }
    }
}

#[test]
fn canonical_edges_normalize_signed_zero_but_do_not_merge_different_curves() {
    let t = Transform2D::identity();
    let a = Boundary::new([(0.0, -0.0), (0.0, 1.0), (0.0, 2.0), (0.0, 3.0)], &t).unwrap();
    let b = Boundary::new([(-0.0, 3.0), (-0.0, 2.0), (-0.0, 1.0), (-0.0, 0.0)], &t).unwrap();
    assert_eq!(a.key, b.key);
    for i in 0..=32 {
        assert_eq!(bits(a.point(i, 32)), bits(b.point(32 - i, 32)));
    }
    let c = Boundary::new([(0.0, 0.0), (0.1, 1.0), (0.0, 2.0), (0.0, 3.0)], &t).unwrap();
    assert_ne!(a.key, c.key);
    let loop_edge = Boundary::new([(1.3, 9.7), (2.1, 3.4), (2.1, 3.4), (1.3, 9.7)], &t).unwrap();
    for i in 0..=64 {
        assert_eq!(
            bits(loop_edge.point(i, 64)),
            bits(loop_edge.point(64 - i, 64))
        );
    }
    let collapsed = Boundary::new([(1.3, 9.7); 4], &t).unwrap();
    for i in 0..=64 {
        assert_eq!(bits(collapsed.point(i, 64)), bits((1.3, 9.7)));
    }
}

#[test]
fn transitive_axis_constraints_propagate_in_both_source_orders() {
    let mut nets = [
        rectangle(0.0, 0.0, 30.0, 30.0),
        rectangle(30.0, 0.0, 30.0, 30.0),
        rectangle(60.0, 0.0, 30.0, 30.0),
    ];
    nets[2][1][1].1 += 200.0;
    for reverse in [false, true] {
        let mut batch = PatchBatch::new(ShadingRenderOptions::default());
        for i in 0..3 {
            let index = if reverse { 2 - i } else { i };
            batch
                .push(&points(nets[index]), &colors(), 7, &Transform2D::identity())
                .unwrap();
        }
        let (axes, _memory, _) = batch.axes().unwrap();
        assert_eq!(axes[1].steps, axes[3].steps);
        assert_eq!(axes[3].steps, axes[5].steps);
        assert_eq!(
            axes[1].steps,
            batch.patches.iter().map(|p| p.steps).max().unwrap()
        );
    }
}

#[test]
fn estimated_geometry_error_bounds_dense_interior_probes() {
    let mut net = rectangle(-5.0, 2.0, 40.0, 30.0);
    net[1][1].1 += 60.0;
    net[2][2].0 -= 30.0;
    net[0][1].0 -= 12.0;
    let n = required_steps(&net).unwrap();
    for iu in 0..n {
        for iv in 0..n {
            let u = iu as f64 / n as f64;
            let v = iv as f64 / n as f64;
            let h = 1.0 / n as f64;
            let a = evaluate(net, u, v);
            let b = evaluate(net, u + h, v);
            let c = evaluate(net, u, v + h);
            let d = evaluate(net, u + h, v + h);
            for (s, t) in [
                (0.25, 0.25),
                (0.75, 0.75),
                (0.2, 0.7),
                (0.8, 0.4),
                (0.5, 0.5),
            ] {
                let approximation = if s + t <= 1.0 {
                    (
                        a.0 * (1.0 - s - t) + b.0 * s + c.0 * t,
                        a.1 * (1.0 - s - t) + b.1 * s + c.1 * t,
                    )
                } else {
                    (
                        b.0 * (1.0 - t) + c.0 * (1.0 - s) + d.0 * (s + t - 1.0),
                        b.1 * (1.0 - t) + c.1 * (1.0 - s) + d.1 * (s + t - 1.0),
                    )
                };
                assert_near(
                    approximation,
                    evaluate(net, u + s * h, v + t * h),
                    GEOMETRY_TOLERANCE_PX + 1e-10,
                );
            }
        }
    }
}

#[test]
fn nonfinite_derivatives_and_excessive_refinement_do_not_under_refine() {
    let mut net = rectangle(0.0, 0.0, 1.0, 1.0);
    net[1][1] = (f64::MAX, -f64::MAX);
    net[2][1] = (-f64::MAX, f64::MAX);
    assert!(required_steps(&net).is_err());
    net = rectangle(0.0, 0.0, 1.0, 1.0);
    net[1][1].1 = 1e12;
    assert!(required_steps(&net).is_err());
    let mut p = points(net);
    p[0].0 = f64::NAN;
    assert!(control_net(&p, 7).is_err());
    assert!(control_net(&p[..11], 6).is_err());
}

#[test]
fn patch_collection_topology_and_rows_are_separately_bounded_and_release_tokens() {
    use crate::decode_scheduler::DecodeMemoryBudget;
    use std::sync::Arc;
    let p = points(rectangle(0.0, 0.0, 10.0, 10.0));
    let memory = Arc::new(DecodeMemoryBudget::new(1_000_000));
    let options = ShadingRenderOptions::default()
        .with_memory_budget(&memory)
        .with_working_byte_limit(2 * std::mem::size_of::<Patch>());
    let mut batch = PatchBatch::new(options);
    batch
        .push(&p, &colors(), 7, &Transform2D::identity())
        .unwrap();
    assert!(batch.axes().is_err());
    drop(batch);
    drop(memory.try_acquire(1_000_000).unwrap());
    let mut empty = PatchBatch::new(ShadingRenderOptions::default().with_working_byte_limit(0));
    assert!(empty
        .push(&p, &colors(), 7, &Transform2D::identity())
        .is_err());
}

#[test]
fn topology_work_and_cancellation_are_not_reported_as_success() {
    let work = AtomicU64::new(64);
    let options = ShadingRenderOptions {
        work_budget: Some(&work),
        ..Default::default()
    };
    let mut batch = PatchBatch::new(options);
    batch
        .push(
            &points(rectangle(0.0, 0.0, 10.0, 10.0)),
            &colors(),
            7,
            &Transform2D::identity(),
        )
        .unwrap();
    assert!(batch.axes().is_err());
    let mut batch = PatchBatch::new(ShadingRenderOptions::default());
    batch
        .push(
            &points(rectangle(0.0, 0.0, 10.0, 10.0)),
            &colors(),
            7,
            &Transform2D::identity(),
        )
        .unwrap();
    let token = CancelToken::new();
    token.cancel();
    assert!(token.scope(|| batch.axes()).is_err());
}

#[test]
fn offscreen_control_hulls_do_not_consume_visible_refinement_budget() {
    let mut net = rectangle(1e6, 0.0, 30.0, 30.0);
    net[1][1].1 = 1e12;
    assert!(Patch::new(&points(net), &colors(), 7, &Transform2D::identity()).is_err());
    let mut batch =
        PatchBatch::new(ShadingRenderOptions::default()).with_device_bounds(Some((0, 0, 10, 10)));
    batch
        .push(&points(net), &colors(), 7, &Transform2D::identity())
        .unwrap();
    assert!(batch.patches[0].outside);
    assert_eq!(batch.patches[0].steps, 1);
    // A control hull intersecting the viewport cannot use that shortcut.
    net[1][1].0 = 0.0;
    assert!(batch
        .push(&points(net), &colors(), 7, &Transform2D::identity())
        .is_err());
}

#[test]
fn streaming_rows_fit_the_accounted_budget_without_a_full_grid() {
    let p = points(rectangle(0.0, 0.0, 10.0, 10.0));
    let bytes = 2 * std::mem::size_of::<Patch>()
        + 4 * std::mem::size_of::<EdgeOwner>()
        + 2 * std::mem::size_of::<Axis>()
        + 4 * std::mem::size_of::<MeshVertex>();
    let reader = PdfReader::from_bytes(tests_minimal_pdf()).unwrap();
    for (limit, success) in [(bytes - 1, false), (bytes, true)] {
        let options = ShadingRenderOptions::default().with_working_byte_limit(limit);
        let mut batch = PatchBatch::new(options);
        batch
            .push(&p, &colors(), 7, &Transform2D::identity())
            .unwrap();
        let paint = MeshPaint {
            function: None,
            color_space: "DeviceGray",
            color_space_obj: None,
            reader: &reader,
            options,
            patch_corners: None,
        };
        let mut buf = PixelBuffer::new(10, 10);
        assert_eq!(batch.paint(&mut buf, &paint).is_ok(), success);
        assert_eq!(
            buf.get_pixel(5, 5),
            if success {
                [0, 0, 0, 255]
            } else {
                [0, 0, 0, 0]
            }
        );
    }
}

#[test]
fn collection_growth_accounts_for_simultaneous_old_and_new_allocations() {
    use crate::decode_scheduler::DecodeMemoryBudget;
    use std::sync::Arc;
    let bytes = std::mem::size_of::<Patch>();
    let memory = Arc::new(DecodeMemoryBudget::new((6 * bytes) as u64));
    let mut batch = PatchBatch::new(
        ShadingRenderOptions::default()
            .with_memory_budget(&memory)
            .with_working_byte_limit(6 * bytes),
    );
    let p = points(rectangle(0.0, 0.0, 30.0, 30.0));
    for _ in 0..3 {
        batch
            .push(&p, &colors(), 7, &Transform2D::identity())
            .unwrap();
    }
    assert_eq!(memory.metrics().peak_reserved_bytes, (6 * bytes) as u64);
    assert_eq!(batch.reserved_bytes, 4 * bytes);
    drop(batch);
    drop(memory.try_acquire((6 * bytes) as u64).unwrap());
    let mut tight =
        PatchBatch::new(ShadingRenderOptions::default().with_working_byte_limit(4 * bytes));
    for _ in 0..2 {
        tight
            .push(&p, &colors(), 7, &Transform2D::identity())
            .unwrap();
    }
    assert!(tight
        .push(&p, &colors(), 7, &Transform2D::identity())
        .is_err());
    assert_eq!(tight.patches.len(), 2);
}
