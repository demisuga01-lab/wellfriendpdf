//! Regression source only. Not executed in the source-only implementation.
use super::*;
use crate::PdfDictionary;

fn reader() -> PdfReader {
    PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap()
}
fn name(s: &str) -> PdfObject {
    PdfObject::Name(s.into())
}
fn calgray() -> PdfObject {
    let mut params = PdfDictionary::empty();
    params.insert(
        "WhitePoint",
        PdfObject::Array(vec![
            PdfObject::Real(0.9505),
            PdfObject::Integer(1),
            PdfObject::Real(1.089),
        ]),
    );
    params.insert("Gamma", PdfObject::Real(2.2));
    PdfObject::Array(vec![name("CalGray"), PdfObject::Dictionary(params)])
}

#[test]
fn identity_null_and_intrinsic_names_do_not_recurse_or_shadow_device_spaces() {
    let reader = reader();
    let mut resources = PageResources::default();
    resources
        .color_spaces
        .insert("DeviceGray".into(), name("DeviceRGB"));
    assert_eq!(
        bind(&name("DeviceGray"), &resources, &reader).unwrap(),
        name("DeviceGray")
    );
    resources
        .color_spaces
        .insert("DefaultGray".into(), name("DeviceGray"));
    assert_eq!(
        bind(&name("DeviceGray"), &resources, &reader).unwrap(),
        name("DeviceGray")
    );
    resources
        .color_spaces
        .insert("DefaultGray".into(), PdfObject::Null);
    assert_eq!(
        bind(&name("DeviceGray"), &resources, &reader).unwrap(),
        name("DeviceGray")
    );
    // Abbreviations are reserved only in inline-image syntax, not arbitrary cs.
    resources.color_spaces.insert("G".into(), calgray());
    assert_eq!(bind(&name("G"), &resources, &reader).unwrap(), calgray());
}

#[test]
fn underlying_pattern_indexed_and_tint_alternates_receive_defaults_without_changing_inputs() {
    let reader = reader();
    let mut resources = PageResources::default();
    resources
        .color_spaces
        .insert("DefaultGray".into(), calgray());
    resources
        .color_spaces
        .insert("Alias".into(), name("DeviceGray"));
    assert_eq!(
        bind(&name("Alias"), &resources, &reader).unwrap(),
        calgray()
    );
    for (source, index) in [
        (vec![name("Pattern"), name("Alias")], 1),
        (
            vec![
                name("Indexed"),
                name("Alias"),
                PdfObject::Integer(1),
                PdfObject::String(vec![0, 255]),
            ],
            1,
        ),
        (
            vec![
                name("Separation"),
                name("Ink"),
                name("Alias"),
                PdfObject::Null,
            ],
            2,
        ),
        (
            vec![
                name("DeviceN"),
                PdfObject::Array(vec![name("Ink")]),
                name("Alias"),
                PdfObject::Null,
            ],
            2,
        ),
    ] {
        let original = PdfObject::Array(source.clone());
        let bound = bind(&original, &resources, &reader).unwrap();
        let mut expected = source;
        expected[index] = calgray();
        assert_eq!(bound, PdfObject::Array(expected));
        assert_eq!(
            initial_components(&original, &resources, &reader)
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn defaults_are_not_reapplied_to_their_own_underlying_device_space() {
    let reader = reader();
    let mut resources = PageResources::default();
    let replacement = PdfObject::Array(vec![
        name("Separation"),
        name("Ink"),
        name("DeviceGray"),
        PdfObject::Null,
    ]);
    resources
        .color_spaces
        .insert("DefaultGray".into(), replacement.clone());
    assert_eq!(
        bind(&name("DeviceGray"), &resources, &reader).unwrap(),
        replacement
    );
    assert_eq!(
        initial_components(&name("DeviceGray"), &resources, &reader).unwrap(),
        vec![0.0]
    );
    assert_eq!(
        initial_components(&name("DeviceCMYK"), &resources, &reader).unwrap(),
        vec![0.0, 0.0, 0.0, 1.0]
    );
}

#[test]
fn invalid_default_families_component_counts_and_alias_cycles_are_rejected() {
    let reader = reader();
    for bad in [
        name("DeviceRGB"),
        PdfObject::Array(vec![
            name("Lab"),
            PdfObject::Dictionary(PdfDictionary::empty()),
        ]),
        PdfObject::Array(vec![name("Pattern"), name("DeviceGray")]),
        PdfObject::Array(vec![
            name("Indexed"),
            name("DeviceGray"),
            PdfObject::Integer(0),
            PdfObject::String(vec![0]),
        ]),
    ] {
        let mut resources = PageResources::default();
        resources.color_spaces.insert("DefaultGray".into(), bad);
        assert!(bind(&name("DeviceGray"), &resources, &reader).is_err());
    }
    let mut resources = PageResources::default();
    resources
        .color_spaces
        .insert("DefaultGray".into(), name("Loop"));
    resources
        .color_spaces
        .insert("Loop".into(), name("DefaultGray"));
    assert!(bind(&name("DeviceGray"), &resources, &reader).is_err());
}

#[test]
fn icc_defaults_require_a_compatible_profile_component_count_and_preserve_profile_bytes() {
    let reader = reader();
    let mut resources = PageResources::default();
    let mut dict = PdfDictionary::empty();
    dict.insert("N", PdfObject::Integer(3));
    let space = PdfObject::Array(vec![
        name("ICCBased"),
        PdfObject::Stream {
            dict,
            raw: vec![1, 2, 3],
        },
    ]);
    resources
        .color_spaces
        .insert("DefaultRGB".into(), space.clone());
    let bound = bind(&name("DeviceRGB"), &resources, &reader).unwrap();
    let PdfObject::Stream {
        dict: bound_dict,
        raw,
    } = &bound.as_array().unwrap()[1]
    else {
        panic!("profile stream expected")
    };
    assert_eq!(raw, &vec![1, 2, 3]);
    assert_eq!(bound_dict.get("Alternate"), Some(&name("DeviceRGB")));
    // Only the ephemeral bound graph materialises the implicit alternate.
    assert_eq!(resources.color_spaces.get("DefaultRGB"), Some(&space));
    resources.color_spaces.insert("DefaultGray".into(), space);
    assert!(bind(&name("DeviceGray"), &resources, &reader).is_err());
    // Binding validates arity, not the bytes of a complete ICC profile; the
    // existing CMM is still responsible for actual conversion/profile validity.
}

#[test]
fn bound_graph_is_a_snapshot_and_cancelled_binding_returns_no_result() {
    let reader = reader();
    let mut resources = PageResources::default();
    resources
        .color_spaces
        .insert("DefaultGray".into(), calgray());
    let bound = bind(&name("DeviceGray"), &resources, &reader).unwrap();
    resources.color_spaces.clear();
    assert_eq!(bound, calgray());
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| bind(&name("DeviceGray"), &resources, &reader))
        .is_err());
}

#[test]
fn initial_lab_and_icc_components_clip_to_declared_ranges_and_reject_invalid_bounds() {
    let reader = reader();
    let resources = PageResources::default();
    let numbers =
        |values: &[f64]| PdfObject::Array(values.iter().copied().map(PdfObject::Real).collect());
    let mut params = PdfDictionary::empty();
    params.insert("Range", numbers(&[10.0, 20.0, -30.0, -10.0]));
    let lab = PdfObject::Array(vec![name("Lab"), PdfObject::Dictionary(params)]);
    assert_eq!(
        initial_components(&lab, &resources, &reader).unwrap(),
        vec![0.0, 10.0, -10.0]
    );
    let mut dict = PdfDictionary::empty();
    dict.insert("N", PdfObject::Integer(3));
    dict.insert("Range", numbers(&[0.2, 0.8, 0.0, 1.0, 0.1, 0.9]));
    assert_eq!(
        icc_component_ranges(&dict, &reader).unwrap(),
        vec![(0.2, 0.8), (0.0, 1.0), (0.1, 0.9)]
    );
    let icc = PdfObject::Array(vec![
        name("ICCBased"),
        PdfObject::Stream {
            dict: dict.clone(),
            raw: vec![],
        },
    ]);
    assert_eq!(
        initial_components(&icc, &resources, &reader).unwrap(),
        vec![0.2, 0.0, 0.1]
    );
    for values in [
        &[0.0, 1.0][..],
        &[1.0, 0.0, 0.0, 1.0, 0.0, 1.0][..],
        &[0.0, f64::NAN, 0.0, 1.0, 0.0, 1.0][..],
    ] {
        dict.insert("Range", numbers(values));
        assert!(icc_component_ranges(&dict, &reader).is_err());
    }
}

#[test]
fn inline_abbreviations_are_canonicalised_only_in_colour_space_positions() {
    let indexed = PdfObject::Array(vec![
        name("I"),
        name("G"),
        PdfObject::Integer(1),
        PdfObject::String(vec![0, 255]),
    ]);
    let expected = PdfObject::Array(vec![
        name("Indexed"),
        name("DeviceGray"),
        PdfObject::Integer(1),
        PdfObject::String(vec![0, 255]),
    ]);
    assert_eq!(canonical_inline(&indexed).unwrap(), expected);
    let sep = PdfObject::Array(vec![
        name("Separation"),
        name("G"),
        name("RGB"),
        PdfObject::Null,
    ]);
    assert_eq!(
        canonical_inline(&sep).unwrap(),
        PdfObject::Array(vec![
            name("Separation"),
            name("G"),
            name("DeviceRGB"),
            PdfObject::Null
        ])
    );
    assert!(canonical_inline(&PdfObject::Array(vec![])).is_err());
    let mut deep = name("G");
    for _ in 0..MAX_DEPTH {
        deep = PdfObject::Array(vec![name("Pattern"), deep]);
    }
    assert!(canonical_inline(&deep).is_err());
}

#[test]
fn indexed_palette_streams_decode_filters_and_enforce_a_bounded_byte_table() {
    let reader = reader();
    let mut dict = PdfDictionary::empty();
    dict.insert("Filter", name("ASCIIHexDecode"));
    let stream = PdfObject::Stream {
        dict,
        raw: b"FF0000 00FF00>".to_vec(),
    };
    assert_eq!(
        crate::render::colorspace::indexed_lookup_bytes(&stream, &reader).unwrap(),
        vec![255, 0, 0, 0, 255, 0]
    );
    assert!(crate::render::colorspace::indexed_lookup_bytes(
        &PdfObject::String(vec![0; 4097]),
        &reader
    )
    .is_err());
    assert!(
        crate::render::colorspace::indexed_lookup_bytes(&PdfObject::Integer(1), &reader).is_err()
    );
    let mut invalid = PdfDictionary::empty();
    invalid.insert("Filter", name("DCTDecode"));
    assert!(crate::render::colorspace::indexed_lookup_bytes(
        &PdfObject::Stream {
            dict: invalid,
            raw: vec![0]
        },
        &reader
    )
    .is_err());
}
