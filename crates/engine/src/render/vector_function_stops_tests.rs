//! Source regressions only. These have not been executed in this change.
use super::super::*;
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects};

fn linear(domain: [f64; 2], c0: &[f64], c1: &[f64]) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(2));
    dict.insert("Domain", numbers(&domain));
    dict.insert("C0", numbers(c0));
    dict.insert("C1", numbers(c1));
    dict.insert("N", PdfObject::Integer(1));
    dict
}

fn stitch(children: Vec<PdfDictionary>, bounds: &[f64], encode: &[f64]) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(3));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert(
        "Functions",
        PdfObject::Array(children.into_iter().map(PdfObject::Dictionary).collect()),
    );
    dict.insert("Bounds", numbers(bounds));
    dict.insert("Encode", numbers(encode));
    dict
}

fn stops(function: PdfDictionary, domain: [f64; 2]) -> VectorShadingStops {
    let mut dict = PdfDictionary::empty();
    dict.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
    dict.insert("Function", PdfObject::Dictionary(function));
    simple_vector_shading_stops(
        &dict,
        None,
        &PageResources::default(),
        domain,
        VectorOutputTarget::Svg,
    )
    .unwrap()
}

fn assert_gray(stops: &[VectorShadingStop], expected: &[(f64, f32)]) {
    assert_eq!(stops.len(), expected.len(), "{stops:?}");
    for (stop, &(offset, gray)) in stops.iter().zip(expected) {
        assert!((stop.offset - offset).abs() < 1e-14, "{stop:?}");
        assert!(
            stop.rgb.iter().all(|v| (*v - gray).abs() < 1e-6),
            "{stop:?}"
        );
    }
}

#[test]
fn type2_output_range_creates_plateaus_instead_of_endpoint_smearing() {
    let mut dict = linear([0.0, 1.0], &[0.0], &[1.0]);
    dict.insert("Range", numbers(&[0.25, 0.75]));
    assert_gray(
        &stops(dict, [0.0, 1.0]).stops,
        &[(0.0, 0.25), (0.25, 0.25), (0.75, 0.75), (1.0, 0.75)],
    );
}

#[test]
fn device_clipping_is_an_explicit_breakpoint_without_function_range() {
    let dict = linear([0.0, 1.0], &[-1.0], &[3.0]);
    assert_gray(
        &stops(dict, [0.0, 1.0]).stops,
        &[(0.0, 0.0), (0.25, 0.0), (0.5, 1.0), (1.0, 1.0)],
    );
}

#[test]
fn stitching_tracks_child_domain_clamps_and_parent_range() {
    let child = linear([0.25, 0.75], &[0.0], &[1.0]);
    let mut dict = stitch(vec![child], &[], &[0.0, 1.0]);
    dict.insert("Range", numbers(&[0.375, 0.625]));
    assert_gray(
        &stops(dict, [0.0, 1.0]).stops,
        &[
            (0.0, 0.375),
            (0.25, 0.375),
            (0.375, 0.375),
            (0.625, 0.625),
            (0.75, 0.625),
            (1.0, 0.625),
        ],
    );
}

#[test]
fn reversed_encode_and_shading_domain_preserve_plateau_geometry() {
    let child = linear([0.25, 0.75], &[0.0], &[1.0]);
    let dict = stitch(vec![child], &[], &[1.0, 0.0]);
    assert_gray(
        &stops(dict.clone(), [0.0, 1.0]).stops,
        &[(0.0, 0.75), (0.25, 0.75), (0.75, 0.25), (1.0, 0.25)],
    );
    assert_gray(
        &stops(dict, [1.0, 0.0]).stops,
        &[(0.0, 0.25), (0.25, 0.25), (0.75, 0.75), (1.0, 0.75)],
    );
}

#[test]
fn native_and_vector_stitching_agree_at_and_around_exact_boundaries() {
    let mut dict = stitch(
        vec![
            linear([0.0, 1.0], &[0.0], &[0.0]),
            linear([0.0, 1.0], &[1.0], &[1.0]),
        ],
        &[0.5],
        &[0.0, 1.0, 0.0, 1.0],
    );
    dict.insert("Range", numbers(&[0.25, 0.75]));
    let parsed = parse_vector_shading_function(&dict, None, true).unwrap();
    let reader = reader_with_objects(&[]);
    for (input, expected) in [(0.5 - 1e-12, 0.25), (0.5, 0.75), (0.5 + 1e-12, 0.75)] {
        assert_eq!(parsed.sample(input).unwrap(), vec![expected]);
        assert_eq!(
            parsed.sample(input).unwrap(),
            crate::render::function::eval_function_n(
                &PdfObject::Dictionary(dict.clone()),
                &[input],
                &reader
            )
        );
    }
    assert!(
        parse_vector_shading_function(&dict, None, false).is_none(),
        "SVG hard jumps remain routed to raster until boundary ownership is implemented"
    );
}

#[test]
fn parent_clipping_can_make_a_discontinuous_child_pair_continuous() {
    let mut dict = stitch(
        vec![
            linear([0.0, 1.0], &[0.0], &[1.0]),
            linear([0.0, 1.0], &[0.75], &[1.0]),
        ],
        &[0.5],
        &[0.0, 1.0, 0.0, 1.0],
    );
    dict.insert("Range", numbers(&[0.0, 0.5]));
    let parsed = parse_vector_shading_function(&dict, None, false).unwrap();
    assert_eq!(parsed.sample(0.5).unwrap(), vec![0.5]);
    assert_gray(
        &stops(dict, [0.0, 1.0]).stops,
        &[(0.0, 0.0), (0.25, 0.5), (0.5, 0.5), (1.0, 0.5)],
    );
}

#[test]
fn empty_terminal_segment_uses_encode_start_and_survives_exact_ps_mapping() {
    let dict = stitch(
        vec![
            linear([0.0, 1.0], &[0.0], &[0.0]),
            linear([0.0, 1.0], &[0.0], &[1.0]),
        ],
        &[1.0],
        &[0.0, 1.0, 0.25, 0.75],
    );
    let parsed = parse_vector_shading_function(&dict, None, true).unwrap();
    assert_eq!(parsed.sample(1.0 - 1e-12).unwrap(), vec![0.0]);
    assert_eq!(parsed.sample(1.0).unwrap(), vec![0.25]);
    let VectorShadingFunction::Stitching(function) = parsed else {
        panic!("stitching")
    };
    assert!(exact_postscript_stitching_rgb_function(
        &function,
        PostScriptExactRgbMapping::DeviceGray
    )
    .is_some());
}

#[test]
fn distinct_tiny_intervals_are_not_deduplicated_or_rejected() {
    let function =
        parse_vector_shading_function(&linear([1e-12, 2e-12], &[0.0], &[1.0]), None, false)
            .unwrap();
    let mut offsets = vector_shading_stop_offsets([0.0, 1.0], &[function]).unwrap();
    offsets.sort_by(f64::total_cmp);
    offsets.dedup();
    assert_eq!(offsets, vec![0.0, 1e-12, 2e-12, 1.0]);
    assert_eq!(vector_shading_domain(&[1e-12, 2e-12]), Some([1e-12, 2e-12]));
    let mut offsets = vec![0.0, 1.0];
    push_domain_boundary_offset([-1e308, 1e308], 0.0, &mut offsets).unwrap();
    assert_eq!(offsets.last(), Some(&0.5));
}

#[test]
fn parent_ranges_are_retained_by_rgb_gray_cmyk_and_tint_adapters() {
    for channels in [1, 3, 4] {
        let mut dict = stitch(
            vec![linear(
                [0.0, 1.0],
                &vec![0.0; channels],
                &vec![1.0; channels],
            )],
            &[],
            &[0.0, 1.0],
        );
        dict.insert("Range", numbers(&[0.25, 0.75].repeat(channels)));
        let VectorShadingFunction::Stitching(function) =
            parse_vector_shading_function(&dict, None, true).unwrap()
        else {
            panic!("stitching")
        };
        match channels {
            1 => {
                assert_eq!(
                    exact_postscript_stitching_tint_function(&function)
                        .unwrap()
                        .range,
                    Some([0.25, 0.75])
                );
                assert_eq!(
                    exact_postscript_stitching_rgb_function(
                        &function,
                        PostScriptExactRgbMapping::DeviceGray
                    )
                    .unwrap()
                    .range,
                    Some([[0.25, 0.75]; 3])
                );
            }
            3 => assert_eq!(
                exact_postscript_stitching_rgb_function(
                    &function,
                    PostScriptExactRgbMapping::DeviceRgb
                )
                .unwrap()
                .range,
                Some([[0.25, 0.75]; 3])
            ),
            4 => assert_eq!(
                exact_postscript_stitching_cmyk_function(&function)
                    .unwrap()
                    .range,
                Some([[0.25, 0.75]; 4])
            ),
            _ => unreachable!(),
        }
    }
}

#[test]
fn malformed_ranges_missing_bounds_and_fractional_negative_domains_fail_closed() {
    let mut dict = stitch(vec![linear([0.0, 1.0], &[0.0], &[1.0])], &[], &[0.0, 1.0]);
    dict.insert("Range", numbers(&[0.75, 0.25]));
    assert!(parse_vector_shading_function(&dict, None, true).is_none());
    dict.insert("Range", numbers(&[0.0, 1.0, 0.0, 1.0]));
    assert!(parse_vector_shading_function(&dict, None, true).is_none());
    dict.remove("Range");
    dict.remove("Bounds");
    assert!(parse_vector_shading_function(&dict, None, true).is_none());
    let mut dict = linear([-1.0, 1.0], &[0.0], &[1.0]);
    dict.insert("N", PdfObject::Real(0.5));
    assert!(parse_vector_shading_function(&dict, None, true).is_none());
    dict.insert("N", PdfObject::Real(1.0 + 1e-12));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    assert!(
        parse_vector_shading_function(&dict, None, false).is_none(),
        "nearly linear is not linear"
    );
}

#[test]
fn stop_accumulation_is_bounded_before_push() {
    let mut offsets = vec![0.0; super::MAX_STOPS];
    assert!(push_domain_boundary_offset([0.0, 1.0], 0.5, &mut offsets).is_none());
    assert_eq!(offsets.len(), super::MAX_STOPS);
}

#[test]
fn nonlinear_colour_conversion_is_adaptively_sampled_or_preserved_exactly() {
    let reader = reader_with_objects(&[]);
    let resources = PageResources::default();
    let mut shading = PdfDictionary::empty();
    shading.insert("ColorSpace", PdfObject::Name("DeviceCMYK".into()));
    shading.insert(
        "Function",
        PdfObject::Dictionary(linear([0.0, 1.0], &[0.0; 4], &[1.0; 4])),
    );
    let svg = simple_vector_shading_stops(
        &shading,
        Some(&reader),
        &resources,
        [0.0, 1.0],
        VectorOutputTarget::Svg,
    )
    .unwrap();
    assert!(
        svg.stops.len() > 2,
        "nonlinear CMYK-to-sRGB conversion needs interior stops"
    );
    let ps = simple_vector_shading_stops(
        &shading,
        Some(&reader),
        &resources,
        [0.0, 1.0],
        VectorOutputTarget::PostScript,
    )
    .unwrap();
    assert!(matches!(
        ps.ps_function,
        Some(VectorPostScriptShadingFunction::Type2Cmyk(_))
    ));

    let mut params = PdfDictionary::empty();
    params.insert("WhitePoint", numbers(&[0.95047, 1.0, 1.08883]));
    params.insert("Gamma", PdfObject::Real(2.0));
    shading.insert(
        "ColorSpace",
        PdfObject::Array(vec![
            PdfObject::Name("CalGray".into()),
            PdfObject::Dictionary(params),
        ]),
    );
    shading.insert(
        "Function",
        PdfObject::Dictionary(linear([0.0, 1.0], &[0.0], &[1.0])),
    );
    let svg = simple_vector_shading_stops(
        &shading,
        Some(&reader),
        &resources,
        [0.0, 1.0],
        VectorOutputTarget::Svg,
    )
    .unwrap();
    assert!(
        svg.stops.len() > 2,
        "nonlinear calibrated conversion needs interior stops"
    );
    assert!(simple_vector_shading_stops(
        &shading,
        Some(&reader),
        &resources,
        [0.0, 1.0],
        VectorOutputTarget::PostScript,
    )
    .is_none());
    shading.insert(
        "Function",
        PdfObject::Dictionary(linear([0.0, 1.0], &[0.5], &[0.5])),
    );
    assert!(
        simple_vector_shading_stops(
            &shading,
            Some(&reader),
            &resources,
            [0.0, 1.0],
            VectorOutputTarget::Svg
        )
        .is_some(),
        "a constant source may still use a constant vector colour"
    );
}

#[test]
fn nearly_equal_component_domains_and_exponents_are_not_merged() {
    let mut red = linear([0.0, 1.0], &[0.0], &[1.0]);
    let green = linear([1e-12, 1.0], &[0.0], &[1.0]);
    let blue = linear([0.0, 1.0], &[0.0], &[1.0]);
    for exponent in [1.0, 1.0 + 1e-12] {
        red.insert("N", PdfObject::Real(exponent));
        let parsed = parse_vector_shading_functions(
            &PdfObject::Array(vec![
                PdfObject::Dictionary(red.clone()),
                PdfObject::Dictionary(green.clone()),
                PdfObject::Dictionary(blue.clone()),
            ]),
            None,
            true,
        )
        .unwrap();
        assert!(matches!(
            exact_postscript_type2_function_array_rgb_function(&parsed),
            Some(VectorPostScriptShadingFunction::Type2RgbArray(_))
        ));
    }
}
