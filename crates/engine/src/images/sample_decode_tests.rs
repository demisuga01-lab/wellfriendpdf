//! Regression source only: these assertions have not been executed.
use super::*;
use crate::render::{
    cmm,
    colorspace::{self, NamedColor},
};

fn reader() -> PdfReader {
    PdfReader::from_bytes(crate::render::shading::tests_minimal_pdf()).unwrap()
}
fn numbers(values: &[f64]) -> PdfObject {
    PdfObject::Array(values.iter().copied().map(PdfObject::Real).collect())
}
fn dict(space: PdfObject, decode: Option<&[f64]>) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert("ColorSpace", space);
    if let Some(decode) = decode {
        dict.insert("Decode", numbers(decode));
    }
    dict
}
fn icc(n: i64, range: &[f64], alternate: PdfObject) -> PdfObject {
    let mut profile = PdfDictionary::empty();
    profile.insert("N", PdfObject::Integer(n));
    profile.insert("Range", numbers(range));
    profile.insert("Alternate", alternate);
    PdfObject::Array(vec![
        PdfObject::Name("ICCBased".into()),
        PdfObject::Stream {
            dict: profile,
            raw: vec![],
        },
    ])
}
fn lab() -> PdfObject {
    let mut params = PdfDictionary::empty();
    params.insert("WhitePoint", numbers(&[0.9505, 1.0, 1.089]));
    params.insert("Range", numbers(&[-100.0, 100.0, -100.0, 100.0]));
    PdfObject::Array(vec![
        PdfObject::Name("Lab".into()),
        PdfObject::Dictionary(params),
    ])
}
fn options() -> cmm::ColorTransformOptions {
    cmm::ColorTransformOptions {
        backend: cmm::ColorTransformBackend::DeterministicFallback,
        ..Default::default()
    }
}
fn map(values: &[(f64, f64)]) -> DecodeMap {
    DecodeMap::from_dictionary(&PdfDictionary::empty(), values, None).unwrap()
}

#[test]
fn one_bit_rows_do_not_consume_padding_as_next_row_samples() {
    let data = [0b1011_1111, 0b0101_1111];
    let samples = PackedSamples::new(&data, 3, 2, 1, 1).unwrap();
    let map = map(&[(0.0, 1.0)]);
    for (i, expected) in [1.0, 0.0, 1.0, 0.0, 1.0, 0.0].into_iter().enumerate() {
        let mut out = [0.0];
        samples.decode_pixel(i, &map, &mut out).unwrap();
        assert_eq!(out, [expected]);
    }
}

#[test]
fn two_and_four_bit_interleaved_samples_restart_at_row_boundaries() {
    for (bits, data, top, expected) in [
        (
            2,
            vec![0b0110_1111, 0b1110_0111],
            3.0,
            [[1.0, 2.0, 3.0], [3.0, 2.0, 1.0]],
        ),
        (
            4,
            vec![0x12, 0x3f, 0x45, 0x6f],
            15.0,
            [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]],
        ),
    ] {
        let samples = PackedSamples::new(&data, 1, 2, 3, bits).unwrap();
        let map = map(&[(0.0, top); 3]);
        for (i, expected) in expected.into_iter().enumerate() {
            let mut out = [0.0; 3];
            samples.decode_pixel(i, &map, &mut out).unwrap();
            for (actual, expected) in out.into_iter().zip(expected) {
                assert!((actual - expected).abs() < 1e-12);
            }
        }
    }
}

#[test]
fn sixteen_bit_big_endian_samples_keep_low_order_bits() {
    let samples = PackedSamples::new(&[0x01, 0x00, 0x01, 0x01], 2, 1, 1, 16).unwrap();
    let map = map(&[(0.0, 65535.0)]);
    let mut out = [0.0];
    samples.decode_pixel(0, &map, &mut out).unwrap();
    assert!((out[0] - 256.0).abs() < 1e-10);
    samples.decode_pixel(1, &map, &mut out).unwrap();
    assert!((out[0] - 257.0).abs() < 1e-10);
}

#[test]
fn bad_packing_lengths_geometry_depth_and_component_access_fail_closed() {
    for (data, w, h, c, b) in [
        (vec![], 1, 1, 1, 8),
        (vec![0, 0], 1, 1, 1, 8),
        (vec![0], 1, 1, 0, 8),
        (vec![0], 1, 1, 17, 8),
        (vec![0], 1, 1, 1, 3),
        (vec![], u32::MAX, u32::MAX, 16, 16),
    ] {
        assert!(PackedSamples::new(&data, w, h, c, b).is_err());
    }
    let samples = PackedSamples::new(&[0], 1, 1, 1, 8).unwrap();
    assert!(samples
        .decode_pixel(1, &map(&[(0.0, 1.0)]), &mut [0.0])
        .is_err());
    assert!(samples
        .decode_pixel(0, &map(&[(0.0, 1.0)]), &mut [0.0; 2])
        .is_err());
    assert_eq!(
        PackedSamples::new(&[], 0, 2, 1, 8).unwrap().pixel_count(),
        0
    );
}

#[test]
fn decode_accepts_descending_constant_and_non_unit_endpoints() {
    let dict = dict(
        PdfObject::Name("DeviceRGB".into()),
        Some(&[100.0, -100.0, 42.0, 42.0, -20.0, 80.0]),
    );
    let decode = DecodeMap::from_dictionary(&dict, &[(0.0, 1.0); 3], None).unwrap();
    let samples = PackedSamples::new(&[255, 12, 0], 1, 1, 3, 8).unwrap();
    let mut out = [0.0; 3];
    samples.decode_pixel(0, &decode, &mut out).unwrap();
    assert_eq!(out, [-100.0, 42.0, -20.0]);
}

#[test]
fn decode_validates_null_alias_types_counts_and_finite_endpoints() {
    let mut dict = PdfDictionary::empty();
    dict.insert("D", numbers(&[1.0, 0.0]));
    assert_eq!(
        DecodeMap::from_dictionary(&dict, &[(0.0, 1.0)], None)
            .unwrap()
            .0,
        vec![(1.0, 0.0)]
    );
    dict.insert("Decode", PdfObject::Null);
    assert_eq!(
        DecodeMap::from_dictionary(&dict, &[(0.0, 1.0)], None)
            .unwrap()
            .0,
        vec![(0.0, 1.0)]
    );
    for invalid in [
        PdfObject::Name("Invalid".into()),
        numbers(&[0.0]),
        numbers(&[f64::NAN, 1.0]),
        numbers(&[0.0, f64::INFINITY]),
        PdfObject::Reference {
            number: 99,
            generation: 0,
        },
    ] {
        dict.insert("Decode", invalid);
        assert!(DecodeMap::from_dictionary(&dict, &[(0.0, 1.0)], None).is_err());
    }
}

#[test]
fn finite_extreme_decode_bounds_do_not_overflow_subtraction() {
    let samples = PackedSamples::new(&[0, 128, 255], 3, 1, 1, 8).unwrap();
    let decode = map(&[(-f64::MAX, f64::MAX)]);
    for index in 0..3 {
        let mut out = [0.0];
        samples.decode_pixel(index, &decode, &mut out).unwrap();
        assert!(out[0].is_finite());
    }
}

#[test]
fn lab_explicit_decode_is_applied_before_colour_conversion() {
    let reader = reader();
    let space = lab();
    let dict = dict(space.clone(), Some(&[50.0, 50.0, -20.0, -20.0, 30.0, 30.0]));
    let image = convert_component_image(
        &[0; 3],
        1,
        1,
        3,
        8,
        "Lab",
        &dict,
        Some(&reader),
        options(),
        Some(&space),
    )
    .unwrap();
    let NamedColor::Color(expected) = colorspace::resolve_named_color_with_options(
        &space,
        &[50.0, -20.0, 30.0],
        1.0,
        &reader,
        options(),
    ) else {
        panic!("valid Lab colour rejected")
    };
    assert_eq!(image.channels, 3);
    assert_eq!(image.pixels, expected.to_pixel_color()[..3]);
}

#[test]
fn lab_decode_values_are_clipped_to_component_ranges_not_unit_ranges() {
    let reader = reader();
    let space = lab();
    let dict = dict(
        space.clone(),
        Some(&[200.0, 200.0, -200.0, -200.0, 300.0, 300.0]),
    );
    let image = convert_component_image(
        &[0; 3],
        1,
        1,
        3,
        8,
        "Lab",
        &dict,
        Some(&reader),
        options(),
        Some(&space),
    )
    .unwrap();
    let NamedColor::Color(expected) = colorspace::resolve_named_color_with_options(
        &space,
        &[100.0, -100.0, 100.0],
        1.0,
        &reader,
        options(),
    ) else {
        panic!("valid Lab colour rejected")
    };
    assert_eq!(image.pixels, expected.to_pixel_color()[..3]);
}

#[test]
fn icc_lab_alternate_receives_signed_decode_values_without_byte_quantization() {
    let reader = reader();
    let space = icc(3, &[0.0, 100.0, -100.0, 100.0, -100.0, 100.0], lab());
    let dict = dict(space.clone(), Some(&[50.0, 50.0, -20.0, -20.0, 30.0, 30.0]));
    let image = convert_component_image(
        &[0; 6],
        1,
        1,
        3,
        16,
        "ICCBased",
        &dict,
        Some(&reader),
        options(),
        Some(&space),
    )
    .unwrap();
    let NamedColor::Color(expected) = crate::render::icc_conversion::resolve_color(
        &space,
        &[50.0, -20.0, 30.0],
        1.0,
        &reader,
        options(),
    )
    .unwrap() else {
        panic!("valid ICC alternate rejected")
    };
    assert_eq!(image.channels, 4);
    assert_eq!(image.pixels, expected.to_pixel_color());
}

#[test]
fn original_icc_range_and_replacement_icc_range_have_distinct_decode_defaults() {
    let reader = reader();
    let space = icc(1, &[0.25, 0.75], PdfObject::Name("DeviceGray".into()));
    let dict = dict(space.clone(), None);
    let direct = convert_component_image(
        &[64],
        1,
        1,
        1,
        8,
        "ICCBased",
        &dict,
        Some(&reader),
        options(),
        Some(&space),
    )
    .unwrap();
    let device = PdfObject::Name("DeviceGray".into());
    let remapped = convert_component_image(
        &[64],
        1,
        1,
        1,
        8,
        "ICCBased",
        &dict,
        Some(&reader),
        options(),
        Some(&device),
    )
    .unwrap();
    assert_eq!(direct.pixels, vec![96, 96, 96, 255]);
    assert_eq!(remapped.pixels, vec![64, 64, 64, 255]);
}

#[test]
fn full_inline_decoder_preserves_low_bits_before_high_gain_decode() {
    let reader = reader();
    let space = icc(1, &[0.0, 1.0], PdfObject::Name("DeviceGray".into()));
    let dict = dict(space.clone(), Some(&[0.0, 65535.0]));
    let image=super::super::decoder::ImageDecoder::decode_inline_with_resolved_image_dictionary_and_param_array(
        &[0,1],1,1,16,"ICCBased",Some(&space),&[],&[],&dict,
        &crate::filters::DecodeLimits::default(),Some(&reader),options(),
    ).unwrap();
    assert_eq!(image.pixels, vec![255; 4]);
}

#[test]
fn full_inline_decoder_retains_source_default_before_replacing_colour_space() {
    let reader = reader();
    let replacement = icc(1, &[0.25, 0.75], PdfObject::Name("DeviceGray".into()));
    for (source, expected) in [
        (replacement.clone(), vec![96, 96, 96, 255]),
        (PdfObject::Name("DeviceGray".into()), vec![64, 64, 64, 255]),
    ] {
        let dict = dict(source, None);
        let image = super::super::decoder::ImageDecoder::decode_inline_with_resolved_image_dictionary_and_param_array(
            &[64], 1, 1, 8, "ICCBased", Some(&replacement), &[], &[], &dict,
            &crate::filters::DecodeLimits::default(), Some(&reader), options(),
        ).unwrap();
        assert_eq!(image.pixels, expected);
    }
}

#[test]
fn component_conversion_observes_cancellation_without_returning_pixels() {
    let reader = reader();
    let space = lab();
    let dict = dict(space.clone(), None);
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| convert_component_image(
            &[0; 3],
            1,
            1,
            3,
            8,
            "Lab",
            &dict,
            Some(&reader),
            options(),
            Some(&space)
        ))
        .is_err());
}

#[test]
fn optimized_inline_raw_window_retains_original_icc_decode_domain() {
    use crate::images::decoder::{ImageDecoder, RawImageDecodeWindow};
    let reader = reader();
    let space = icc(1, &[0.25, 0.75], PdfObject::Name("DeviceGray".into()));
    let device = PdfObject::Name("DeviceGray".into());
    for (source, first) in [(&space, 96), (&device, 64)] {
        let image =
            ImageDecoder::decode_inline_raw_window_with_resolved_color_space_and_param_array(
                &[20, 64, 96, 128],
                2,
                2,
                8,
                "ICCBased",
                Some(&space),
                &[],
                &[],
                RawImageDecodeWindow {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 2,
                },
                &crate::filters::DecodeLimits::default(),
                Some(&reader),
                options(),
                Some(source),
            )
            .unwrap();
        assert_eq!(
            image.pixels,
            vec![first, first, first, 255, 128, 128, 128, 255]
        );
    }
}

#[test]
fn optimized_inline_scaled_jpeg_retains_original_icc_decode_domain() {
    use crate::images::{
        decoder::{ImageDecoder, RawImage},
        encoder::ImageEncoder,
    };
    let reader = reader();
    let space = icc(1, &[0.25, 0.75], PdfObject::Name("DeviceGray".into()));
    let device = PdfObject::Name("DeviceGray".into());
    let jpeg = ImageEncoder::encode_jpeg(
        &RawImage {
            width: 8,
            height: 8,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![64; 64],
        },
        100,
    )
    .unwrap();
    for (source, expected) in [(&space, 96i16), (&device, 64i16)] {
        let image =
            ImageDecoder::decode_inline_scaled_dct_with_resolved_color_space_and_param_array(
                &jpeg,
                8,
                8,
                "ICCBased",
                Some(&space),
                &["DCTDecode"],
                &[None],
                4,
                4,
                &crate::filters::DecodeLimits::default(),
                Some(&reader),
                options(),
                Some(source),
            )
            .unwrap();
        assert_eq!((image.width, image.height, image.channels), (4, 4, 4));
        assert!(image.is_valid());
        assert!(image
            .pixels
            .chunks_exact(4)
            .all(|pixel| (i16::from(pixel[0]) - expected).abs() <= 2 && pixel[3] == 255));
    }
}

#[test]
fn missing_source_alias_is_not_silently_assumed_to_have_unit_decode() {
    let reader = reader();
    let space = icc(1, &[0.25, 0.75], PdfObject::Name("DeviceGray".into()));
    let dict = dict(space, None);
    let unknown = PdfObject::Name("UnresolvedAlias".into());
    assert!(convert_component_image(
        &[64],
        1,
        1,
        1,
        8,
        "ICCBased",
        &dict,
        Some(&reader),
        options(),
        Some(&unknown)
    )
    .is_err());
}

#[test]
fn explicit_icc_decode_does_not_require_unused_original_default_domains() {
    let reader = reader();
    let space = icc(1, &[0.25, 0.75], PdfObject::Name("DeviceGray".into()));
    let dict = dict(space, Some(&[0.25, 0.75]));
    let unknown = PdfObject::Name("UnresolvedAlias".into());
    let image = convert_component_image(
        &[64],
        1,
        1,
        1,
        8,
        "ICCBased",
        &dict,
        Some(&reader),
        options(),
        Some(&unknown),
    )
    .unwrap();
    assert_eq!(image.pixels, vec![96, 96, 96, 255]);
}
