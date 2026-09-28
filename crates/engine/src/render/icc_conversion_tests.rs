//! Unexecuted source regressions; dummy profile bytes below test Alternate
//! routing, not the correctness of a real ICC implementation.
use super::*;

fn reader() -> PdfReader {
    PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap()
}
fn name(value: &str) -> PdfObject {
    PdfObject::Name(value.into())
}
fn numbers(values: &[f64]) -> PdfObject {
    PdfObject::Array(values.iter().copied().map(PdfObject::Real).collect())
}
fn options() -> cmm::ColorTransformOptions {
    cmm::ColorTransformOptions {
        backend: cmm::ColorTransformBackend::DeterministicFallback,
        ..Default::default()
    }
}
fn space(n: i64, alternate: Option<PdfObject>, ranges: Option<&[f64]>) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("N", PdfObject::Integer(n));
    if let Some(alternate) = alternate {
        dict.insert("Alternate", alternate);
    }
    if let Some(ranges) = ranges {
        dict.insert("Range", numbers(ranges));
    }
    PdfObject::Array(vec![
        name("ICCBased"),
        PdfObject::Stream { dict, raw: vec![] },
    ])
}
fn image_dict(space: &PdfObject) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert("ColorSpace", space.clone());
    dict
}
fn pixel(color: NamedColor) -> [u8; 4] {
    match color {
        NamedColor::Color(color) => color.to_pixel_color(),
        NamedColor::NoPaint => [0; 4],
        other => panic!("unexpected {other:?}"),
    }
}
fn spot(colorant: &str) -> PdfObject {
    let mut function = PdfDictionary::empty();
    function.insert("FunctionType", PdfObject::Integer(2));
    function.insert("Domain", numbers(&[0.0, 1.0]));
    function.insert("C0", numbers(&[0.0, 0.0, 0.0]));
    function.insert("C1", numbers(&[0.0, 1.0, 0.0]));
    function.insert("N", PdfObject::Integer(1));
    PdfObject::Array(vec![
        name("Separation"),
        name(colorant),
        name("DeviceRGB"),
        PdfObject::Dictionary(function),
    ])
}

#[test]
fn omitted_alternates_select_device_spaces_with_matching_components() {
    let reader = reader();
    for (n, values) in [
        (1, vec![128]),
        (3, vec![30, 120, 200]),
        (4, vec![0, 0, 0, 255]),
    ] {
        let space = space(n, None, None);
        let components = values
            .iter()
            .map(|v| f64::from(*v) / 255.0)
            .collect::<Vec<_>>();
        let expected = pixel(resolve_color(&space, &components, 1.0, &reader, options()).unwrap());
        let (pixels, channels) =
            convert_image(&values, &image_dict(&space), &reader, options()).unwrap();
        assert_eq!(channels, 4);
        assert_eq!(pixels, expected.to_vec());
    }
}
#[test]
fn declared_spot_alternate_replaces_channel_count_guessing_for_paint_and_images() {
    let reader = reader();
    let space = space(1, Some(spot("Green")), None);
    assert_eq!(
        pixel(resolve_color(&space, &[1.0], 1.0, &reader, options()).unwrap()),
        [0, 255, 0, 255]
    );
    assert_eq!(
        convert_image(&[255, 0], &image_dict(&space), &reader, options()).unwrap(),
        (vec![0, 255, 0, 255, 0, 0, 0, 255], 4)
    );
}
#[test]
fn none_alternate_preserves_no_paint_instead_of_black_rgb() {
    let reader = reader();
    let space = space(1, Some(spot("None")), None);
    assert_eq!(
        resolve_color(&space, &[0.5], 1.0, &reader, options()).unwrap(),
        NamedColor::NoPaint
    );
    assert_eq!(
        convert_image(&[128], &image_dict(&space), &reader, options()).unwrap(),
        (vec![0; 4], 4)
    );
}
#[test]
fn source_range_is_clipped_before_alternate_without_normalising_values() {
    let reader = reader();
    let space = space(1, Some(name("DeviceGray")), Some(&[10.0, 20.0]));
    assert_eq!(
        pixel(resolve_color(&space, &[15.0], 1.0, &reader, options()).unwrap()),
        [255; 4]
    );
    assert_eq!(
        convert_image(&[128], &image_dict(&space), &reader, options()).unwrap(),
        (vec![255; 4], 4)
    );
    let ranged = self::space(1, Some(spot("Green")), Some(&[0.25, 0.75]));
    assert_eq!(
        pixel(resolve_color(&ranged, &[0.0], 1.0, &reader, options()).unwrap()),
        [0, 64, 0, 255]
    );
    assert_eq!(
        pixel(resolve_color(&ranged, &[1.0], 1.0, &reader, options()).unwrap()),
        [0, 191, 0, 255]
    );
}
#[test]
fn lab_alternate_keeps_signed_and_non_unit_components_and_alpha() {
    let reader = reader();
    let mut params = PdfDictionary::empty();
    params.insert("WhitePoint", numbers(&[0.9505, 1.0, 1.089]));
    let lab = PdfObject::Array(vec![name("Lab"), PdfObject::Dictionary(params)]);
    let space = space(
        3,
        Some(lab.clone()),
        Some(&[0.0, 100.0, -100.0, 100.0, -100.0, 100.0]),
    );
    let values = [50.0, -20.0, 30.0];
    let expected =
        colorspace::resolve_named_color_with_options(&lab, &values, 0.5, &reader, options());
    assert_eq!(
        resolve_color(&space, &values, 0.5, &reader, options()).unwrap(),
        expected
    );
}
#[test]
fn malformed_metadata_does_not_fall_back_by_channel_count() {
    let reader = reader();
    for space in [
        space(2, None, None),
        space(1, None, Some(&[1.0, 0.0])),
        space(1, None, Some(&[0.0])),
        space(1, Some(name("DeviceRGB")), None),
        space(
            1,
            Some(PdfObject::Array(vec![name("Pattern"), name("DeviceGray")])),
            None,
        ),
    ] {
        assert!(resolve_color(&space, &[0.5], 1.0, &reader, options()).is_err());
        assert!(convert_image(&[128], &image_dict(&space), &reader, options()).is_err());
    }
}
#[test]
fn unavailable_profile_uses_valid_declared_alternate_and_records_route() {
    let reader = reader();
    let space = space(1, Some(spot("Green")), None);
    let before = metrics();
    let actual = resolve_color(
        &space,
        &[1.0],
        1.0,
        &reader,
        cmm::ColorTransformOptions::default(),
    )
    .unwrap();
    assert_eq!(pixel(actual), [0, 255, 0, 255]);
    assert!(metrics().unavailable_profile_alternates > before.unavailable_profile_alternates);
}
#[test]
fn corrupt_stream_filter_is_an_error_not_an_unavailable_profile_fallback() {
    let reader = reader();
    let mut space = space(1, Some(spot("Green")), None);
    if let PdfObject::Array(items) = &mut space {
        if let PdfObject::Stream { dict, .. } = &mut items[1] {
            dict.insert("Filter", name("NoSuchFilter"));
        }
    }
    assert!(resolve_color(
        &space,
        &[1.0],
        1.0,
        &reader,
        cmm::ColorTransformOptions::default()
    )
    .is_err());
    assert!(convert_image(
        &[255],
        &image_dict(&space),
        &reader,
        cmm::ColorTransformOptions::default()
    )
    .is_err());
}
#[test]
fn nested_alternates_and_scope_bound_default_aliases_are_supported() {
    let reader = reader();
    let inner = space(1, Some(spot("Green")), None);
    let outer = space(1, Some(inner), None);
    assert_eq!(
        pixel(resolve_color(&outer, &[1.0], 1.0, &reader, options()).unwrap()),
        [0, 255, 0, 255]
    );
    let mut resources = PageResources::default();
    resources
        .color_spaces
        .insert("DefaultGray".into(), spot("Green"));
    let bound = default_colorspace::bind(&space(1, None, None), &resources, &reader).unwrap();
    assert_eq!(
        pixel(resolve_color(&bound, &[1.0], 1.0, &reader, options()).unwrap()),
        [0, 255, 0, 255]
    );
}
#[test]
fn recursion_and_cancellation_fail_without_poisoning_later_conversion() {
    let reader = reader();
    let mut deep = space(1, None, None);
    for _ in 0..40 {
        deep = space(1, Some(deep), None);
    }
    assert!(resolve_color(&deep, &[0.5], 1.0, &reader, options()).is_err());
    let ordinary = space(1, None, None);
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| convert_image(&[128], &image_dict(&ordinary), &reader, options()))
        .is_err());
    assert_eq!(
        pixel(resolve_color(&ordinary, &[0.0], 1.0, &reader, options()).unwrap()),
        [0, 0, 0, 255]
    );
    ACTIVE_DEPTH.with(|depth| assert_eq!(depth.get(), 0));
}
#[test]
fn components_and_sample_lengths_are_validated_before_fallback() {
    let reader = reader();
    let space = space(3, None, None);
    assert!(resolve_color(&space, &[f64::NAN, 0.0, 0.0], 1.0, &reader, options()).is_err());
    assert!(resolve_color(&space, &[0.0], 1.0, &reader, options()).is_err());
    assert!(resolve_color(&space, &[0.0; 3], f32::NAN, &reader, options()).is_err());
    assert!(convert_image(&[0, 0], &image_dict(&space), &reader, options()).is_err());
    assert_eq!(
        convert_image(&[], &image_dict(&space), &reader, options()).unwrap(),
        (vec![], 4)
    );
}
#[test]
fn inline_decoder_and_shared_colour_converter_use_the_same_alternate_route() {
    let reader = reader();
    let space = space(1, Some(spot("Green")), None);
    let dict = image_dict(&space);
    let image=crate::images::decoder::ImageDecoder::decode_inline_with_resolved_image_dictionary_and_param_array(
        &[255],1,1,8,"ICCBased",Some(&space),&[],&[],&dict,&DecodeLimits::default(),Some(&reader),options(),
    ).unwrap();
    assert_eq!(image.channels, 4);
    assert_eq!(image.pixels, vec![0, 255, 0, 255]);
    assert!(
        crate::images::decoder::ColorSpaceConverter::convert_with_options(
            vec![255],
            2,
            1,
            "ICCBased",
            &dict,
            &reader,
            options()
        )
        .is_err()
    );
    assert_eq!(
        crate::images::decoder::ColorSpaceConverter::convert_with_options(
            vec![255],
            1,
            1,
            "ICCBased",
            &dict,
            &reader,
            options()
        )
        .unwrap(),
        (image.pixels, 4)
    );
}

#[test]
fn colour_report_labels_usage_scope_and_accepts_reports_without_new_metrics() {
    use crate::color_report::{color_report, ColorReport, ColorValidationProfile};
    let reader = reader();
    let space = space(1, None, None);
    resolve_color(&space, &[0.5], 1.0, &reader, options()).unwrap();
    let report = color_report(&reader, ColorValidationProfile::Generic);
    assert!(report
        .icc_alternate_usage
        .scope
        .contains("not_per_document"));
    assert!(report.icc_alternate_usage.policy_alternates > 0);
    let mut legacy = serde_json::to_value(&report).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("icc_alternate_usage");
    let decoded: ColorReport = serde_json::from_value(legacy).unwrap();
    assert_eq!(decoded.icc_alternate_usage.policy_alternates, 0);
}

#[test]
fn indexed_icc_palette_retains_requested_backend_policy() {
    let reader = reader();
    let mut base = space(1, Some(spot("Green")), None);
    // Policy fallback intentionally does not decode the ICC program. If the
    // palette silently resets options to portable CMM this filter is rejected.
    if let PdfObject::Array(items) = &mut base {
        if let PdfObject::Stream { dict, .. } = &mut items[1] {
            dict.insert("Filter", name("NoSuchFilter"));
        }
    }
    let indexed = PdfObject::Array(vec![
        name("Indexed"),
        base,
        PdfObject::Integer(0),
        PdfObject::String(vec![255]),
    ]);
    let dict = image_dict(&indexed);
    assert_eq!(
        crate::images::decoder::ColorSpaceConverter::convert_with_options(
            vec![0],
            1,
            1,
            "Indexed",
            &dict,
            &reader,
            options(),
        )
        .unwrap(),
        (vec![0, 255, 0, 255], 4),
    );
    assert!(
        crate::images::decoder::ColorSpaceConverter::convert_with_options(
            vec![0],
            1,
            1,
            "Indexed",
            &dict,
            &reader,
            cmm::ColorTransformOptions::default(),
        )
        .is_err()
    );
}

#[cfg(all(feature = "native-cmm-lcms2", not(target_arch = "wasm32")))]
#[test]
fn supported_profile_uses_native_cmm_without_an_alternate_and_keeps_shape() {
    let reader = reader();
    let mut space = space(3, None, None);
    if let PdfObject::Array(items) = &mut space {
        if let PdfObject::Stream { raw, .. } = &mut items[1] {
            *raw = lcms2::Profile::new_srgb().icc().unwrap();
        }
    }
    let options = cmm::ColorTransformOptions {
        backend: cmm::ColorTransformBackend::NativeLittleCms,
        ..Default::default()
    };
    let before = metrics();
    let (pixels, channels) = convert_image(
        &[255, 0, 0, 0, 255, 0],
        &image_dict(&space),
        &reader,
        options,
    )
    .unwrap();
    assert_eq!(channels, 3);
    assert_eq!(pixels.len(), 6);
    assert!(pixels[0] > 250 && pixels[1] < 5 && pixels[2] < 5);
    let after = metrics();
    assert_eq!(after.native_conversions, before.native_conversions + 1);
    assert_eq!(
        after.unavailable_profile_alternates,
        before.unavailable_profile_alternates
    );
}
