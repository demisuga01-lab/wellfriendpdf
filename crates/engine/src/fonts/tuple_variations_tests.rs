//! Deliberately unexecuted arithmetic/format regressions.
use super::*;
fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|n| n.to_be_bytes()).collect()
}
fn coords(values: &[i16]) -> Vec<ttf_parser::NormalizedCoordinate> {
    values
        .iter()
        .copied()
        .map(ttf_parser::NormalizedCoordinate::from)
        .collect()
}
fn packed_deltas(values: &[i16]) -> Vec<u8> {
    let mut data = Vec::new();
    for chunk in values.chunks(64) {
        data.push(0x40 | (chunk.len() as u8 - 1));
        for value in chunk {
            data.extend(value.to_be_bytes());
        }
    }
    data
}
fn packed_points(values: &[u16]) -> Vec<u8> {
    let mut data = if values.len() < 128 {
        vec![values.len() as u8]
    } else {
        words(&[0x8000 | values.len() as u16])
    };
    let mut last = 0;
    for chunk in values.chunks(128) {
        data.push(0x80 | (chunk.len() as u8 - 1));
        for point in chunk {
            data.extend((point - last).to_be_bytes());
            last = *point;
        }
    }
    data
}
struct Tuple {
    peak: Vec<i16>,
    bounds: Option<(Vec<i16>, Vec<i16>)>,
    shared_peak: Option<u16>,
    points: Option<Vec<u16>>,
    x: Vec<i16>,
    y: Vec<i16>,
}
fn tuple(peak: &[i16], points: Option<&[u16]>, x: &[i16], y: &[i16]) -> Tuple {
    Tuple {
        peak: peak.to_vec(),
        bounds: None,
        shared_peak: None,
        points: points.map(<[u16]>::to_vec),
        x: x.to_vec(),
        y: y.to_vec(),
    }
}
fn fixture(cvt: bool, tuples: &[Tuple], shared: Option<&[u16]>) -> Vec<u8> {
    let mut out = if cvt { words(&[1, 0]) } else { Vec::new() };
    out.extend(words(&[
        tuples.len() as u16 | if shared.is_some() { 0x8000 } else { 0 },
        0,
    ]));
    let offset_field = out.len() - 2;
    let mut payloads = Vec::new();
    for tuple in tuples {
        let mut data = tuple
            .points
            .as_ref()
            .map_or_else(Vec::new, |points| packed_points(points));
        data.extend(packed_deltas(&tuple.x));
        if !cvt {
            data.extend(packed_deltas(&tuple.y));
        }
        let flags = tuple.shared_peak.unwrap_or(0x8000)
            | if tuple.bounds.is_some() { 0x4000 } else { 0 }
            | if tuple.points.is_some() { 0x2000 } else { 0 };
        out.extend(words(&[data.len() as u16, flags]));
        if tuple.shared_peak.is_none() {
            for value in &tuple.peak {
                out.extend(value.to_be_bytes());
            }
        }
        if let Some((start, end)) = &tuple.bounds {
            for value in start.iter().chain(end) {
                out.extend(value.to_be_bytes());
            }
        }
        payloads.push(data);
    }
    let offset = out.len() as u16;
    out[offset_field..offset_field + 2].copy_from_slice(&offset.to_be_bytes());
    if let Some(points) = shared {
        out.extend(packed_points(points));
    }
    for payload in payloads {
        out.extend(payload);
    }
    out
}
fn run(data: &[u8], coordinates: &[i16], domain: Domain<'_>) -> Deltas {
    resolve(
        data,
        &coords(coordinates),
        &[],
        domain,
        &mut Budget::default(),
    )
    .unwrap()
}
#[test]
fn signed_byte_word_and_zero_runs_decode_without_losing_negatives() {
    let mut cursor = Cursor {
        data: &[1, 0x80, 0x7f, 0x81, 0x41, 0x80, 0, 0x7f, 0xff],
        at: 0,
    };
    assert_eq!(
        deltas(&mut cursor, 6, &mut Budget::default()).unwrap(),
        [-128, 127, 0, 0, -32768, 32767]
    );
    assert_eq!(cursor.at, cursor.data.len());
}
#[test]
fn packed_points_accumulate_across_runs_and_retain_duplicates() {
    let mut cursor = Cursor {
        data: &[4, 1, 3, 0, 0x81, 0, 255, 0, 1],
        at: 0,
    };
    let points = points(&mut cursor, 300, &mut Budget::default()).unwrap();
    assert_eq!(
        (0..4).map(|i| points.at(i)).collect::<Vec<_>>(),
        [3, 3, 258, 259]
    );
    let original = (0..200).collect::<Vec<u16>>();
    let bytes = packed_points(&original);
    let mut cursor = Cursor {
        data: &bytes,
        at: 0,
    };
    assert_eq!(points_len(&mut cursor, 200), 200);
}
fn points_len(cursor: &mut Cursor<'_>, total: usize) -> usize {
    points(cursor, total, &mut Budget::default())
        .unwrap()
        .len(total)
}
#[test]
fn shared_points_and_private_overrides_are_distinct() {
    let data = fixture(
        true,
        &[
            tuple(&[16384], None, &[10], &[]),
            tuple(&[16384], Some(&[1]), &[20], &[]),
        ],
        Some(&[0]),
    );
    let out = run(&data, &[16384], Domain::Cvt(3));
    assert_eq!(out.values, [[10., 0.], [20., 0.], [0., 0.]]);
}
#[test]
fn an_explicit_all_points_marker_overrides_a_shared_subset() {
    let data = fixture(
        true,
        &[tuple(&[16384], Some(&[]), &[1, 2, 3], &[])],
        Some(&[1]),
    );
    assert_eq!(
        run(&data, &[16384], Domain::Cvt(3)).values,
        [[1., 0.], [2., 0.], [3., 0.]]
    );
}
#[test]
fn repeated_cvt_indices_add_all_their_deltas() {
    let data = fixture(
        true,
        &[tuple(&[16384], Some(&[1, 1, 1]), &[10, -2, 7], &[])],
        None,
    );
    assert_eq!(
        run(&data, &[8192], Domain::Cvt(3)).values,
        [[0., 0.], [7.5, 0.], [0., 0.]]
    );
}
#[test]
fn explicit_zero_is_a_touched_point_for_contour_interpolation() {
    let data = fixture(
        false,
        &[tuple(&[16384], Some(&[0, 2]), &[0, 20], &[0, 40])],
        None,
    );
    let points = [[0, 0], [50, 50], [100, 100]];
    let out = run(
        &data,
        &[16384],
        Domain::Simple {
            points: &points,
            contour_ends: &[2],
        },
    );
    assert_eq!(&out.values[..3], [[0., 0.], [10., 20.], [20., 40.]]);
    assert_eq!(&out.values[3..], [[0., 0.]; 4]);
}
#[test]
fn duplicate_point_deltas_accumulate_before_inference_and_scaling() {
    let data = fixture(
        false,
        &[tuple(
            &[16384],
            Some(&[0, 0, 2]),
            &[10, 10, 40],
            &[20, -10, 30],
        )],
        None,
    );
    let points = [[0, 0], [50, 50], [100, 100]];
    let out = run(
        &data,
        &[8192],
        Domain::Simple {
            points: &points,
            contour_ends: &[2],
        },
    );
    assert_eq!(&out.values[..3], [[10., 5.], [15., 10.], [20., 15.]]);
}
#[test]
fn inference_uses_original_geometry_independently_for_each_tuple() {
    let data = fixture(
        false,
        &[
            tuple(&[16384], Some(&[0, 2]), &[100, 0], &[0, 0]),
            tuple(&[16384], Some(&[0, 2]), &[0, 100], &[0, 0]),
        ],
        None,
    );
    let points = [[0, 0], [25, 0], [100, 0]];
    let out = run(
        &data,
        &[16384],
        Domain::Simple {
            points: &points,
            contour_ends: &[2],
        },
    );
    assert_eq!(&out.values[..3], [[100., 0.]; 3]);
}
#[test]
fn inference_wraps_point_order_and_never_crosses_contours() {
    let data = fixture(
        false,
        &[tuple(
            &[16384],
            Some(&[1, 3, 5]),
            &[10, 30, 8],
            &[20, 60, 16],
        )],
        None,
    );
    let points = [
        [0, 0],
        [10, 10],
        [20, 20],
        [30, 30],
        [40, 40],
        [5, 5],
        [15, 15],
        [99, 99],
    ];
    let out = run(
        &data,
        &[16384],
        Domain::Simple {
            points: &points,
            contour_ends: &[4, 6, 7],
        },
    );
    assert_eq!(
        &out.values[..8],
        [
            [10., 20.],
            [10., 20.],
            [20., 40.],
            [30., 60.],
            [30., 60.],
            [8., 16.],
            [8., 16.],
            [0., 0.]
        ]
    );
}
#[test]
fn equal_endpoint_coordinates_use_equal_delta_or_zero_without_division() {
    let data = fixture(
        false,
        &[tuple(&[16384], Some(&[0, 2]), &[10, 20], &[7, 7])],
        None,
    );
    let points = [[10, 10], [100, -100], [10, 10]];
    let out = run(
        &data,
        &[16384],
        Domain::Simple {
            points: &points,
            contour_ends: &[2],
        },
    );
    assert_eq!(out.values[1], [0., 7.]);
}
#[test]
fn component_and_phantom_slots_do_not_receive_inferred_deltas() {
    let data = fixture(
        false,
        &[tuple(
            &[16384],
            Some(&[1, 3, 4, 5, 6]),
            &[20, 1, 2, 3, 4],
            &[30, 5, 6, 7, 8],
        )],
        None,
    );
    let out = run(&data, &[16384], Domain::Components(3));
    assert_eq!(
        out.values,
        [
            [0., 0.],
            [20., 30.],
            [0., 0.],
            [1., 5.],
            [2., 6.],
            [3., 7.],
            [4., 8.]
        ]
    );
}
#[test]
fn glyph_all_points_data_includes_four_phantom_points() {
    let data = fixture(
        false,
        &[tuple(
            &[16384],
            None,
            &[1, 2, 3, 4, 5],
            &[-1, -2, -3, -4, -5],
        )],
        None,
    );
    let out = run(
        &data,
        &[8192],
        Domain::Simple {
            points: &[[0, 0]],
            contour_ends: &[0],
        },
    );
    assert_eq!(
        out.values,
        [[0.5, -0.5], [1., -1.], [1.5, -1.5], [2., -2.], [2.5, -2.5]]
    );
}
#[test]
fn nonintermediate_regions_end_at_the_peak_and_do_not_cross_zero() {
    let data = fixture(true, &[tuple(&[8192], None, &[20], &[])], None);
    for (coord, expected) in [(0, 0.), (4096, 10.), (8192, 20.), (12288, 0.), (-8192, 0.)] {
        assert_eq!(run(&data, &[coord], Domain::Cvt(1)).values[0][0], expected);
    }
    let data = fixture(true, &[tuple(&[-16384], None, &[-20], &[])], None);
    assert_eq!(run(&data, &[-8192], Domain::Cvt(1)).values[0][0], -10.);
}
#[test]
fn intermediate_regions_multiply_axis_scalars_and_ignore_neutral_axes() {
    let mut t = tuple(&[8192, 16384, 0], None, &[80], &[]);
    t.bounds = Some((vec![0, 0, -16384], vec![16384, 16384, 16384]));
    let data = fixture(true, &[t], None);
    assert_eq!(
        run(&data, &[12288, 8192, -16384], Domain::Cvt(1)).values[0][0],
        20.
    );
    assert_eq!(
        run(&data, &[16384, 8192, 0], Domain::Cvt(1)).values[0][0],
        0.
    );
}
#[test]
fn malformed_intermediate_order_ignores_that_axis_as_the_spec_requires() {
    let mut t = tuple(&[8192], None, &[20], &[]);
    t.bounds = Some((vec![12288], vec![16384]));
    assert_eq!(
        run(&fixture(true, &[t], None), &[0], Domain::Cvt(1)).values[0][0],
        20.
    );
    let mut t = tuple(&[8192], None, &[30], &[]);
    t.bounds = Some((vec![-16384], vec![16384]));
    assert_eq!(
        run(&fixture(true, &[t], None), &[-8192], Domain::Cvt(1)).values[0][0],
        30.
    );
}
#[test]
fn shared_peaks_are_gvar_only_and_embedded_peak_ignores_low_index_bits() {
    let mut t = tuple(&[], None, &[20, 0, 0, 0], &[0, 0, 0, 0]);
    t.shared_peak = Some(0);
    let data = fixture(false, &[t], None);
    let out = resolve(
        &data,
        &coords(&[8192]),
        &[vec![16384]],
        Domain::Components(0),
        &mut Budget::default(),
    )
    .unwrap();
    assert_eq!(out.values[0][0], 10.);
    assert!(resolve(
        &data,
        &coords(&[8192]),
        &[],
        Domain::Components(0),
        &mut Budget::default()
    )
    .is_err());
    let mut t = tuple(&[], None, &[20], &[]);
    t.shared_peak = Some(0);
    assert!(resolve(
        &fixture(true, &[t], None),
        &coords(&[8192]),
        &[vec![16384]],
        Domain::Cvt(1),
        &mut Budget::default()
    )
    .is_err());
    let mut embedded = fixture(true, &[tuple(&[16384], None, &[20], &[])], None);
    embedded[10..12].copy_from_slice(&0x8fffu16.to_be_bytes());
    assert_eq!(run(&embedded, &[8192], Domain::Cvt(1)).values[0][0], 10.);
}
#[test]
fn bad_runs_truncation_and_out_of_range_points_are_rejected_even_when_inactive() {
    for data in [&[2, 0, 1][..], &[0x40, 0][..]] {
        assert!(deltas(&mut Cursor { data, at: 0 }, 1, &mut Budget::default()).is_err());
    }
    let data = fixture(true, &[tuple(&[16384], Some(&[2]), &[20], &[])], None);
    assert!(resolve(
        &data,
        &coords(&[0]),
        &[],
        Domain::Cvt(2),
        &mut Budget::default()
    )
    .is_err());
    let data = fixture(true, &[tuple(&[16384], None, &[20], &[])], None);
    for end in 0..data.len() {
        assert!(resolve(
            &data[..end],
            &coords(&[0]),
            &[],
            Domain::Cvt(1),
            &mut Budget::default()
        )
        .is_err());
    }
}
#[test]
fn tuple_length_boundaries_do_not_borrow_bytes_from_the_next_tuple() {
    let mut data = fixture(
        true,
        &[
            tuple(&[16384], None, &[10], &[]),
            tuple(&[16384], None, &[20], &[]),
        ],
        None,
    );
    data[8..10].copy_from_slice(&1u16.to_be_bytes());
    assert!(resolve(
        &data,
        &coords(&[16384]),
        &[],
        Domain::Cvt(1),
        &mut Budget::default()
    )
    .is_err());
}
#[test]
fn resource_and_cancel_limits_precede_large_output_allocation() {
    let data = fixture(true, &[], None);
    assert!(resolve(
        &data,
        &[],
        &[],
        Domain::Cvt(ITEM_LIMIT + 1),
        &mut Budget::default()
    )
    .is_err());
    let mut budget = Budget { work: 16_000_000 };
    assert!(resolve(&data, &[], &[], Domain::Cvt(0), &mut budget).is_err());
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| {
        assert!(resolve(&data, &[], &[], Domain::Cvt(0), &mut Budget::default()).is_err())
    });
}
#[test]
fn contours_headers_reserved_bits_and_coordinates_are_validated() {
    let source = fixture(false, &[], None);
    assert!(resolve(
        &source,
        &[],
        &[],
        Domain::Simple {
            points: &[[0, 0]],
            contour_ends: &[]
        },
        &mut Budget::default()
    )
    .is_err());
    let original = fixture(true, &[tuple(&[16384], None, &[1], &[])], None);
    for (at, value) in [(4, 0x4001), (6, 1), (10, 0x9000), (12, 20000)] {
        let mut data = original.clone();
        data[at..at + 2].copy_from_slice(&(value as u16).to_be_bytes());
        assert!(resolve(
            &data,
            &coords(&[16384]),
            &[],
            Domain::Cvt(1),
            &mut Budget::default()
        )
        .is_err());
    }
}

#[test]
fn cumulative_point_differences_can_address_phantoms_above_u16() {
    // Point count is only two; point numbers are cumulative, not individually u16.
    let mut cursor = Cursor {
        data: &[2, 0x81, 0xff, 0xff, 0, 2],
        at: 0,
    };
    let selected = points(&mut cursor, 65540, &mut Budget::default()).unwrap();
    assert_eq!(selected.at(0), 65535);
    assert_eq!(selected.at(1), 65537);
}

#[test]
fn empty_domains_and_two_byte_all_markers_are_bounded() {
    let empty = fixture(true, &[tuple(&[16384], None, &[], &[])], None);
    assert!(run(&empty, &[16384], Domain::Cvt(0)).values.is_empty());
    let mut cursor = Cursor {
        data: &[0x80, 0],
        at: 0,
    };
    assert_eq!(
        points(&mut cursor, 70000, &mut Budget::default())
            .unwrap()
            .len(70000),
        70000
    );
    assert_eq!(cursor.at, 2);
}
