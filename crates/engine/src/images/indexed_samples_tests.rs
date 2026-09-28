//! Unexecuted regressions for source-domain Indexed conversion.
use super::*;
use crate::images::sample_decode::convert_component_image;

fn reader() -> PdfReader {
    PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap()
}
fn name(s: &str) -> PdfObject {
    PdfObject::Name(s.into())
}
fn nums(values: &[f64]) -> PdfObject {
    PdfObject::Array(values.iter().copied().map(PdfObject::Real).collect())
}
fn indexed_space(base: PdfObject, hival: i64, bytes: Vec<u8>) -> PdfObject {
    PdfObject::Array(vec![
        name("Indexed"),
        base,
        PdfObject::Integer(hival),
        PdfObject::String(bytes),
    ])
}
fn dict(space: PdfObject, decode: Option<&[f64]>) -> PdfDictionary {
    let mut result = PdfDictionary::empty();
    result.insert("ColorSpace", space);
    if let Some(decode) = decode {
        result.insert("Decode", nums(decode));
    }
    result
}
fn options() -> cmm::ColorTransformOptions {
    cmm::ColorTransformOptions {
        backend: cmm::ColorTransformBackend::DeterministicFallback,
        ..Default::default()
    }
}
fn grey_icc() -> PdfObject {
    let mut profile = PdfDictionary::empty();
    profile.insert("N", PdfObject::Integer(1));
    profile.insert("Range", nums(&[0.25, 0.75]));
    profile.insert("Alternate", name("DeviceGray"));
    PdfObject::Array(vec![
        name("ICCBased"),
        PdfObject::Stream {
            dict: profile,
            raw: vec![],
        },
    ])
}
fn rgb_palette() -> PdfObject {
    indexed_space(name("DeviceRGB"), 1, vec![255, 0, 0, 0, 0, 255])
}

fn paint(
    space: &PdfObject,
    source: Option<&PdfObject>,
    alpha: f32,
) -> crate::render::color::RenderColor {
    match colorspace::resolve_named_color_with_source(
        space,
        source,
        &[0.0],
        alpha,
        &reader(),
        options(),
    ) {
        NamedColor::Color(color) => color,
        other => panic!("palette paint rejected: {other:?}"),
    }
}

#[test]
fn indexed_paint_shares_source_domains_with_image_palette_without_byte_quantization() {
    let reader = reader();
    let target = indexed_space(grey_icc(), 0, vec![64]);
    let device = indexed_space(name("DeviceGray"), 0, vec![64]);
    for (source, expected) in [
        (&target, 0.25 + 0.5 * 64.0 / 255.0),
        (&device, 64.0 / 255.0),
    ] {
        let color = paint(&target, Some(source), 0.37);
        assert!((f64::from(color.r) - expected).abs() < 1e-6);
        assert_eq!(color.a, 0.37);
        assert!(
            (f64::from(color.r) * 255.0 - 96.0).abs() > 0.01,
            "paint must not round component values through an image byte buffer"
        );
        let image = convert_indices(
            1,
            &dict(target.clone(), None),
            &reader,
            options(),
            Some(source),
            |_| Ok(0.0),
        )
        .unwrap();
        assert_eq!(image.1, 4);
        assert_eq!(image.0, color.with_alpha(1.0).to_pixel_color());
    }
}

fn icc_alternate(alternate: PdfObject) -> PdfObject {
    let mut profile = PdfDictionary::empty();
    profile.insert("N", PdfObject::Integer(1));
    profile.insert("Alternate", alternate);
    PdfObject::Array(vec![
        name("ICCBased"),
        PdfObject::Stream {
            dict: profile,
            raw: vec![],
        },
    ])
}

fn tint_alternate(alternate: PdfObject, device_n: bool) -> PdfObject {
    let mut tint = PdfDictionary::empty();
    tint.insert("FunctionType", PdfObject::Integer(2));
    tint.insert("Domain", nums(&[0.0, 1.0]));
    tint.insert("C0", nums(&[0.0]));
    tint.insert("C1", nums(&[1.0]));
    tint.insert("N", PdfObject::Integer(1));
    PdfObject::Array(vec![
        name(if device_n { "DeviceN" } else { "Separation" }),
        if device_n {
            PdfObject::Array(vec![name("ReviewInk")])
        } else {
            name("ReviewInk")
        },
        alternate,
        PdfObject::Dictionary(tint),
    ])
}

#[test]
fn nested_icc_alternate_palette_keeps_original_domain_for_paint_and_image() {
    let reader = reader();
    let target = icc_alternate(indexed_space(grey_icc(), 0, vec![64]));
    let source = icc_alternate(indexed_space(name("DeviceGray"), 0, vec![64]));
    assert_eq!(
        paint(&target, Some(&source), 1.0).to_pixel_color(),
        [64, 64, 64, 255]
    );
    assert_eq!(
        paint(&target, None, 1.0).to_pixel_color(),
        [96, 96, 96, 255]
    );
    let image = convert_component_image(
        &[0],
        1,
        1,
        1,
        8,
        "ICCBased",
        &dict(target, None),
        Some(&reader),
        options(),
        Some(&source),
    )
    .unwrap();
    assert_eq!(image.pixels, vec![64, 64, 64, 255]);
}

#[test]
fn separation_and_devicen_carry_provenance_through_cie_alternates() {
    for device_n in [false, true] {
        let target = tint_alternate(
            icc_alternate(indexed_space(grey_icc(), 0, vec![64])),
            device_n,
        );
        let source = tint_alternate(
            icc_alternate(indexed_space(name("DeviceGray"), 0, vec![64])),
            device_n,
        );
        assert_eq!(
            paint(&target, Some(&source), 1.0).to_pixel_color(),
            [64, 64, 64, 255]
        );
        assert_eq!(
            paint(&target, None, 1.0).to_pixel_color(),
            [96, 96, 96, 255]
        );
    }
}

#[test]
fn default_replacement_node_does_not_misinterpret_device_source_as_icc_alternate() {
    let target = icc_alternate(indexed_space(grey_icc(), 0, vec![64]));
    assert_eq!(
        paint(&target, Some(&name("DeviceGray")), 1.0).to_pixel_color(),
        [96, 96, 96, 255]
    );
}

#[test]
fn indexed_paint_no_ink_is_not_opaque_black() {
    let mut no_ink = tint_alternate(name("DeviceGray"), false);
    if let PdfObject::Array(items) = &mut no_ink {
        items[1] = name("None");
    }
    let space = indexed_space(no_ink, 0, vec![255]);
    assert_eq!(
        resolve_color(&space, None, 0.0, 0.4, &reader(), options()).unwrap(),
        NamedColor::NoPaint
    );
}

#[test]
fn indexed_paint_rejects_provenance_mismatch_and_nonfinite_components() {
    let target = rgb_palette();
    let wrong_hival = indexed_space(name("DeviceRGB"), 0, vec![0, 0, 0]);
    let wrong_channels = indexed_space(name("DeviceGray"), 1, vec![0, 0]);
    for source in [&wrong_hival, &wrong_channels] {
        assert!(resolve_color(&target, Some(source), 0.0, 1.0, &reader(), options()).is_err());
    }
    assert!(resolve_color(&target, None, f64::NAN, 1.0, &reader(), options()).is_err());
    assert!(resolve_color(&target, None, 0.0, f32::INFINITY, &reader(), options()).is_err());
}

#[test]
fn one_bit_indexed_rows_skip_padding_without_rescaling_indices() {
    let reader = reader();
    let space = rgb_palette();
    let dict = dict(space.clone(), None);
    let image = convert_component_image(
        &[0b0101_1111, 0b1011_1111],
        3,
        2,
        1,
        1,
        "Indexed",
        &dict,
        Some(&reader),
        options(),
        Some(&space),
    )
    .unwrap();
    assert_eq!(image.channels, 3);
    assert_eq!(
        image.pixels,
        vec![255, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 255]
    );
}

#[test]
fn two_bit_default_decode_selects_all_original_integer_indices() {
    let reader = reader();
    let space = indexed_space(name("DeviceGray"), 3, vec![0, 64, 128, 255]);
    let dict = dict(space.clone(), None);
    let image = convert_component_image(
        &[0b0001_1011],
        4,
        1,
        1,
        2,
        "Indexed",
        &dict,
        Some(&reader),
        options(),
        Some(&space),
    )
    .unwrap();
    assert_eq!(image.channels, 1);
    assert_eq!(image.pixels, vec![0, 64, 128, 255]);
    assert_eq!(
        convert_normalized(&[0, 85, 170, 255], 2, &dict, &reader, 4, 1, options()).unwrap(),
        (image.pixels, 1)
    );
}

#[test]
fn explicit_reversed_decode_is_shared_by_packed_and_normalized_routes() {
    let reader = reader();
    let space = rgb_palette();
    let dict = dict(space.clone(), Some(&[1.0, 0.0]));
    let packed = convert_component_image(
        &[0, 255],
        2,
        1,
        1,
        8,
        "Indexed",
        &dict,
        Some(&reader),
        options(),
        Some(&space),
    )
    .unwrap();
    let normalized = convert_normalized(&[0, 255], 8, &dict, &reader, 2, 1, options()).unwrap();
    assert_eq!(packed.pixels, vec![0, 0, 255, 255, 0, 0]);
    assert_eq!((packed.pixels, packed.channels), normalized);
}

#[test]
fn fractional_decode_rounds_half_up_and_outside_values_clip_to_hival() {
    let reader = reader();
    let half = dict(rgb_palette(), Some(&[0.5, 0.5]));
    assert_eq!(
        convert_normalized(&[0], 8, &half, &reader, 1, 1, options())
            .unwrap()
            .0,
        vec![0, 0, 255]
    );
    let outside = dict(rgb_palette(), Some(&[-100.0, 100.0]));
    assert_eq!(
        convert_normalized(&[0, 255], 8, &outside, &reader, 2, 1, options())
            .unwrap()
            .0,
        vec![255, 0, 0, 0, 0, 255]
    );
}

#[test]
fn palette_icc_range_is_scaled_before_alternate_conversion() {
    let reader = reader();
    let space = indexed_space(grey_icc(), 0, vec![64]);
    let dict = dict(space, None);
    assert_eq!(
        convert_normalized(&[0], 8, &dict, &reader, 1, 1, options()).unwrap(),
        (vec![96, 96, 96, 255], 4)
    );
}

#[test]
fn remapped_base_preserves_original_palette_domain() {
    let reader = reader();
    let bound = indexed_space(grey_icc(), 0, vec![64]);
    let source = indexed_space(name("DeviceGray"), 0, vec![64]);
    let dict = dict(bound, None);
    assert_eq!(
        convert_indices(1, &dict, &reader, options(), Some(&source), |_| Ok(0.0)).unwrap(),
        (vec![64, 64, 64, 255], 4)
    );
}

#[test]
fn lab_palette_bytes_are_scaled_to_signed_component_ranges() {
    let reader = reader();
    let mut params = PdfDictionary::empty();
    params.insert("WhitePoint", nums(&[0.9505, 1.0, 1.089]));
    params.insert("Range", nums(&[-40.0, 80.0, -20.0, 60.0]));
    let lab = PdfObject::Array(vec![name("Lab"), PdfObject::Dictionary(params)]);
    let space = indexed_space(lab.clone(), 0, vec![128, 64, 192]);
    let dict = dict(space, None);
    let expected = colorspace::resolve_named_color_with_options(
        &lab,
        &[
            128.0 * 100.0 / 255.0,
            -40.0 + 64.0 * 120.0 / 255.0,
            -20.0 + 192.0 * 80.0 / 255.0,
        ],
        1.0,
        &reader,
        options(),
    );
    let NamedColor::Color(expected) = expected else {
        panic!("valid Lab palette rejected")
    };
    assert_eq!(
        convert_normalized(&[0], 8, &dict, &reader, 1, 1, options()).unwrap(),
        (expected.to_pixel_color()[..3].to_vec(), 3)
    );
}

#[test]
fn filtered_lookup_stream_is_decoded_once_as_palette_bytes() {
    let reader = reader();
    let mut space = rgb_palette();
    let mut filter = PdfDictionary::empty();
    filter.insert("Filter", name("ASCIIHexDecode"));
    if let PdfObject::Array(items) = &mut space {
        items[3] = PdfObject::Stream {
            dict: filter,
            raw: b"FF00000000FF>".to_vec(),
        };
    }
    let dict = dict(space, None);
    assert_eq!(
        convert_normalized(&[1, 0], 8, &dict, &reader, 2, 1, options())
            .unwrap()
            .0,
        vec![0, 0, 255, 255, 0, 0]
    );
}

#[test]
fn no_paint_palette_keeps_transparent_alpha() {
    let reader = reader();
    let mut tint = PdfDictionary::empty();
    tint.insert("FunctionType", PdfObject::Integer(2));
    tint.insert("Domain", nums(&[0.0, 1.0]));
    tint.insert("C0", nums(&[0.0]));
    tint.insert("C1", nums(&[1.0]));
    tint.insert("N", PdfObject::Integer(1));
    let none = PdfObject::Array(vec![
        name("Separation"),
        name("None"),
        name("DeviceGray"),
        PdfObject::Dictionary(tint),
    ]);
    let dict = dict(indexed_space(none, 0, vec![128]), None);
    assert_eq!(
        convert_normalized(&[0], 8, &dict, &reader, 1, 1, options()).unwrap(),
        (vec![0; 4], 4)
    );
}

#[test]
fn invalid_palettes_decode_maps_and_base_families_are_rejected() {
    let reader = reader();
    for space in [
        indexed_space(name("DeviceRGB"), 0, vec![0, 0]),
        indexed_space(name("DeviceRGB"), 0, vec![0; 4]),
        indexed_space(name("DeviceGray"), 256, vec![0; 257]),
        indexed_space(name("Pattern"), 0, vec![0]),
        indexed_space(rgb_palette(), 0, vec![0]),
    ] {
        assert!(convert_normalized(&[0], 8, &dict(space, None), &reader, 1, 1, options()).is_err());
    }
    for bad in [nums(&[0.0]), nums(&[f64::NAN, 1.0]), name("Invalid")] {
        let mut dict = dict(rgb_palette(), None);
        dict.insert("Decode", bad);
        assert!(convert_normalized(&[0], 8, &dict, &reader, 1, 1, options()).is_err());
    }
}

#[test]
fn replacement_component_and_palette_extent_mismatch_fail_closed() {
    let reader = reader();
    let dict = dict(rgb_palette(), None);
    for source in [
        indexed_space(name("DeviceGray"), 1, vec![0, 255]),
        indexed_space(name("DeviceRGB"), 0, vec![0; 3]),
    ] {
        assert!(convert_indices(1, &dict, &reader, options(), Some(&source), |_| Ok(0.0)).is_err());
    }
}

#[test]
fn indexed_lengths_bit_depth_budgets_and_nonfinite_values_are_checked() {
    let reader = reader();
    let dict = dict(rgb_palette(), None);
    assert!(convert_normalized(&[0], 8, &dict, &reader, 2, 1, options()).is_err());
    assert!(convert_normalized(&[0], 16, &dict, &reader, 1, 1, options()).is_err());
    assert!(
        convert_indices(usize::MAX, &dict, &reader, options(), None, |_| panic!(
            "oversized output must fail before samples"
        ))
        .is_err()
    );
    assert!(convert_indices(1, &dict, &reader, options(), None, |_| Ok(f64::NAN)).is_err());
}

#[test]
fn indexed_conversion_honours_cancellation_before_publishing_output() {
    let reader = reader();
    let dict = dict(rgb_palette(), None);
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| convert_normalized(&[0], 8, &dict, &reader, 1, 1, options()))
        .is_err());
}

#[test]
fn full_inline_indexed_decoder_uses_original_base_domain_with_same_render_graph() {
    let reader = reader();
    let bound = indexed_space(grey_icc(), 0, vec![64]);
    for (source, expected) in [
        (bound.clone(), vec![96, 96, 96, 255]),
        (
            indexed_space(name("DeviceGray"), 0, vec![64]),
            vec![64, 64, 64, 255],
        ),
    ] {
        let dict = dict(source, None);
        let image=crate::images::decoder::ImageDecoder::decode_inline_with_resolved_image_dictionary_and_param_array(
            &[0],1,1,8,"Indexed",Some(&bound),&[],&[],&dict,
            &crate::filters::DecodeLimits::default(),Some(&reader),options(),
        ).unwrap();
        assert_eq!(image.pixels, expected);
    }
}
