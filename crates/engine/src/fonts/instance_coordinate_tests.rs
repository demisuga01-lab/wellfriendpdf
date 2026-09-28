//! Coordinate/parser regressions, not whole-font variable-format qualification.
//! Synthetic metadata is attached to a static outline fixture. Unexecuted.
use super::*;
use std::collections::BTreeMap;

fn axis(tag: &[u8; 4]) -> (Tag, f32, f32, f32) {
    (Tag::from_bytes(tag), -1., 0., 1.)
}
fn fvar(axes: &[(Tag, f32, f32, f32)]) -> Vec<u8> {
    let mut out = Vec::new();
    for n in [
        1u16,
        0,
        16,
        2,
        axes.len() as u16,
        20,
        0,
        (axes.len() * 4 + 4) as u16,
    ] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    for (i, (tag, min, default, max)) in axes.iter().enumerate() {
        out.extend_from_slice(&tag.0.to_be_bytes());
        for value in [min, default, max] {
            out.extend_from_slice(&((*value * 65536.) as i32).to_be_bytes());
        }
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&(256u16 + i as u16).to_be_bytes());
    }
    out
}
fn avar(maps: &[Vec<(i16, i16)>]) -> Vec<u8> {
    let mut out = vec![0, 1, 0, 0, 0, 0];
    out.extend_from_slice(&(maps.len() as u16).to_be_bytes());
    for map in maps {
        out.extend_from_slice(&(map.len() as u16).to_be_bytes());
        for (from, to) in map {
            out.extend_from_slice(&from.to_be_bytes());
            out.extend_from_slice(&to.to_be_bytes());
        }
    }
    out
}
fn nonlinear() -> Vec<(i16, i16)> {
    vec![(-16384, -16384), (0, 0), (8192, 12288), (16384, 16384)]
}
fn font(fvar: Vec<u8>, avar: Option<Vec<u8>>) -> Vec<u8> {
    let input = crate::fonts::pdf_embedding_fixtures::font(false, 0);
    let container = crate::fonts::font_container::Container::parse(&input).unwrap();
    let mut tables = container.faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, input[range.clone()].to_vec()))
        .collect::<BTreeMap<_, _>>();
    tables.insert(*b"fvar", fvar);
    if let Some(avar) = avar {
        tables.insert(*b"avar", avar);
    }
    crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", tables).unwrap()
}
fn coords(face: &Face<'_>) -> Vec<i16> {
    face.variation_coordinates()
        .iter()
        .map(|c| c.get())
        .collect()
}

#[test]
fn nonlinear_axis_is_mapped_once_in_either_request_order() {
    let bytes = font(
        fvar(&[axis(b"TEST"), axis(b"NEXT")]),
        Some(avar(&[nonlinear(), vec![]])),
    );
    let forward = VariationRequest::none()
        .with_axis(Tag::from_bytes(b"TEST"), 0.5)
        .with_axis(Tag::from_bytes(b"NEXT"), 0.5);
    let reverse = VariationRequest::none()
        .with_axis(Tag::from_bytes(b"NEXT"), 0.5)
        .with_axis(Tag::from_bytes(b"TEST"), 0.5);
    for request in [forward, reverse] {
        let mut face = Face::parse(&bytes, 0).unwrap();
        assert!(apply_request_checked(&mut face, &request).unwrap());
        assert_eq!(coords(&face), vec![12288, 8192]);
    }
}

#[test]
fn later_partial_request_does_not_remap_an_existing_axis() {
    let bytes = font(
        fvar(&[axis(b"TEST"), axis(b"NEXT")]),
        Some(avar(&[nonlinear(), vec![]])),
    );
    let mut face = Face::parse(&bytes, 0).unwrap();
    for (tag, value) in [(b"TEST", 0.5), (b"NEXT", -0.5)] {
        apply_request_checked(
            &mut face,
            &VariationRequest::none().with_axis(Tag::from_bytes(tag), value),
        )
        .unwrap();
    }
    assert_eq!(coords(&face), vec![12288, -8192]);
    let reset = VariationRequest::none().with_axis(Tag::from_bytes(b"TEST"), 0.);
    apply_request_checked(&mut face, &reset).unwrap();
    assert_eq!(coords(&face), vec![0, -8192]);
}

#[test]
fn explicit_normal_descriptor_values_override_non_normal_font_defaults() {
    let bytes = font(
        fvar(&[(AXIS_WGHT, 100., 700., 900.), (AXIS_WDTH, 50., 75., 125.)]),
        None,
    );
    let mut face = Face::parse(&bytes, 0).unwrap();
    assert!(apply_request_checked(
        &mut face,
        &VariationRequest::from_descriptor(Some(400.), Some("Normal"))
    )
    .unwrap());
    assert_eq!(coords(&face), vec![-8192, 8192]);
}

#[test]
fn clamping_and_unknown_descriptor_axes_preserve_supported_coordinates() {
    let bytes = font(fvar(&[axis(b"TEST")]), None);
    let mut face = Face::parse(&bytes, 0).unwrap();
    let unknown = VariationRequest::none().with_axis(AXIS_WGHT, 700.);
    assert!(!apply_request_checked(&mut face, &unknown).unwrap());
    for (value, expected) in [(10., 16384), (-10., -16384)] {
        let request = unknown.clone().with_axis(Tag::from_bytes(b"TEST"), value);
        assert!(apply_request_checked(&mut face, &request).unwrap());
        assert_eq!(coords(&face), vec![expected]);
    }
}

#[test]
fn nonfinite_request_is_atomic_even_when_a_valid_axis_precedes_it() {
    let bytes = font(fvar(&[axis(b"TEST"), axis(b"NEXT")]), None);
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut face = Face::parse(&bytes, 0).unwrap();
        let request = VariationRequest::none()
            .with_axis(Tag::from_bytes(b"TEST"), 0.5)
            .with_axis(Tag::from_bytes(b"NEXT"), value);
        assert!(apply_request_checked(&mut face, &request).is_err());
        assert_eq!(coords(&face), vec![0, 0]);
    }
}

#[test]
fn malformed_avar_payload_does_not_masquerade_as_default_coordinates() {
    let mut truncated = avar(&[nonlinear()]);
    truncated.pop();
    let mut wrong_version = avar(&[nonlinear()]);
    wrong_version[1] = 2;
    for mapping in [
        truncated,
        wrong_version,
        avar(&[]),
        avar(&[vec![(0, 0)]]),
        avar(&[vec![(-16384, -16384), (0, 0), (8192, -1), (16384, 16384)]]),
        avar(&[vec![(-16384, -16384), (0, 0), (0, 1), (16384, 16384)]]),
    ] {
        let bytes = font(fvar(&[axis(b"TEST")]), Some(mapping));
        let mut face = Face::parse(&bytes, 0).unwrap();
        let request = VariationRequest::none().with_axis(Tag::from_bytes(b"TEST"), 0.5);
        assert!(apply_request_checked(&mut face, &request).is_err());
        assert_eq!(coords(&face), vec![0]);
    }
}

#[test]
fn raw_fvar_limits_and_record_stride_are_validated_before_parser_clamping() {
    let duplicate = fvar(&[axis(b"TEST"), axis(b"TEST")]);
    let invalid_range = fvar(&[(Tag::from_bytes(b"TEST"), 1., 0., 2.)]);
    let mut wrong_stride = fvar(&[axis(b"TEST")]);
    wrong_stride[11] = 24;
    for raw in [duplicate, invalid_range, wrong_stride] {
        let bytes = font(raw, None);
        let mut face = Face::parse(&bytes, 0).unwrap();
        let before = coords(&face);
        let request = VariationRequest::none().with_axis(Tag::from_bytes(b"TEST"), 0.5);
        assert!(apply_request_checked(&mut face, &request).is_err());
        assert_eq!(coords(&face), before);
    }
}

#[test]
fn zero_axis_fvar_is_static_while_truncated_fvar_is_an_error() {
    let request = VariationRequest::none().with_axis(AXIS_WGHT, 700.);
    let bytes = font(fvar(&[]), None);
    assert!(!apply_request_checked(&mut Face::parse(&bytes, 0).unwrap(), &request).unwrap());
    let bytes = font(vec![0, 1, 0, 0], None);
    assert!(apply_request_checked(&mut Face::parse(&bytes, 0).unwrap(), &request).is_err());
}

#[test]
fn parser_axis_capacity_failure_is_explicit_and_atomic() {
    for count in [63u8, 64] {
        let axes = (0..count)
            .map(|i| axis(&[b'X', b'X', b'X', i]))
            .collect::<Vec<_>>();
        let bytes = font(fvar(&axes), None);
        let mut face = Face::parse(&bytes, 0).unwrap();
        let before = coords(&face);
        let request = VariationRequest::none().with_axis(axes[0].0, 0.5);
        let result = apply_request_checked(&mut face, &request);
        if count == 63 {
            assert!(result.unwrap());
            assert_eq!(coords(&face)[0], 8192);
        } else {
            assert!(result.is_err());
            assert_eq!(coords(&face), before);
        }
    }
}

#[test]
fn cancellation_and_request_budget_prevent_coordinate_publication() {
    let bytes = font(fvar(&[axis(b"TEST")]), None);
    let mut face = Face::parse(&bytes, 0).unwrap();
    let request = VariationRequest::none().with_axis(Tag::from_bytes(b"TEST"), 0.5);
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(apply_request_checked(&mut face, &request).is_err()));
    let request = (0..65u8).fold(request, |r, i| {
        r.with_axis(Tag::from_bytes(&[b'X', b'X', b'X', i]), 0.5)
    });
    assert!(apply_request_checked(&mut face, &request).is_err());
    assert_eq!(coords(&face), vec![0]);
}
