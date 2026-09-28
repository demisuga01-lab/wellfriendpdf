//! Unexecuted source regressions for the packed-sample evaluator.
use super::super::{
    eval_function_n, eval_function_or_array_n, resolve_stream_bytes_limited,
    validate_function_shape, BitReader,
};
use super::*;
use crate::object::{PdfDictionary, PdfObject};
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects, reference};

fn packed(values: &[u32], bits: usize) -> Vec<u8> {
    let mut bytes = vec![0_u8; (values.len() * bits).div_ceil(8)];
    for (sample, &value) in values.iter().enumerate() {
        for bit in 0..bits {
            let index = sample * bits + bit;
            bytes[index / 8] |= (((value >> (bits - bit - 1)) & 1) as u8) << (7 - index % 8);
        }
    }
    bytes
}

fn function(
    sizes: &[usize],
    bits: usize,
    outputs: usize,
    values: &[u32],
    order: Option<i64>,
) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(0));
    dict.insert(
        "Size",
        PdfObject::Array(
            sizes
                .iter()
                .map(|&size| PdfObject::Integer(size as i64))
                .collect(),
        ),
    );
    dict.insert("BitsPerSample", PdfObject::Integer(bits as i64));
    dict.insert(
        "Domain",
        numbers(
            &sizes
                .iter()
                .flat_map(|size| [0.0, size.saturating_sub(1).max(1) as f64])
                .collect::<Vec<_>>(),
        ),
    );
    dict.insert("Range", numbers(&[0.0, 1.0].repeat(outputs)));
    if let Some(order) = order {
        dict.insert("Order", PdfObject::Integer(order));
    }
    PdfObject::Stream {
        dict,
        raw: packed(values, bits),
    }
}

fn set(object: &mut PdfObject, key: &str, value: PdfObject) {
    let PdfObject::Stream { dict, .. } = object else {
        panic!("sample stream")
    };
    dict.insert(key, value);
}

fn evaluate(object: &PdfObject, inputs: &[f64]) -> Vec<f64> {
    eval_function_n(object, inputs, &reader_with_objects(&[]))
}

fn close(value: f64, expected: f64) {
    assert!((value - expected).abs() <= 2e-12, "{value} != {expected}");
}

#[test]
fn cubic_differs_from_linear_at_an_interior_sample_interval() {
    let cubic = function(&[4], 8, 1, &[0, 0, 255, 0], Some(3));
    let linear = function(&[4], 8, 1, &[0, 0, 255, 0], Some(1));
    close(evaluate(&cubic, &[1.5])[0], 0.5625);
    close(evaluate(&linear, &[1.5])[0], 0.5);
    let default = function(&[4], 8, 1, &[0, 0, 255, 0], None);
    assert_eq!(evaluate(&linear, &[1.5]), evaluate(&default, &[1.5]));
}

#[test]
fn cubic_preserves_sample_points_and_clips_the_input_domain() {
    let object = function(&[4], 8, 1, &[0, 85, 170, 255], Some(3));
    for (input, expected) in [
        (-1.0, 0.0),
        (0.0, 0.0),
        (1.0, 1.0 / 3.0),
        (2.0, 2.0 / 3.0),
        (3.0, 1.0),
        (9.0, 1.0),
    ] {
        close(evaluate(&object, &[input])[0], expected);
    }
}

#[test]
fn endpoint_extension_and_cubic_overshoot_are_explicit() {
    let mut object = function(&[4], 8, 1, &[0, 0, 255, 255], Some(3));
    set(&mut object, "Decode", numbers(&[0.0, 1.0]));
    set(&mut object, "Range", numbers(&[-1.0, 2.0]));
    close(evaluate(&object, &[0.5])[0], -0.0625);
    close(evaluate(&object, &[2.5])[0], 1.0625);
    set(&mut object, "Range", numbers(&[0.0, 1.0]));
    assert_eq!(evaluate(&object, &[0.5]), vec![0.0]);
    assert_eq!(evaluate(&object, &[2.5]), vec![1.0]);
}

#[test]
fn decode_is_applied_after_cubic_interpolation_before_range_clipping() {
    let mut object = function(&[4], 8, 1, &[0, 0, 255, 0], Some(3));
    set(&mut object, "Decode", numbers(&[2.0, -2.0]));
    set(&mut object, "Range", numbers(&[-10.0, 10.0]));
    close(evaluate(&object, &[1.5])[0], -0.25);
    close(evaluate(&object, &[0.5])[0], 2.25);
    set(&mut object, "Range", numbers(&[-0.1, 0.1]));
    assert_eq!(evaluate(&object, &[1.5]), vec![-0.1]);
}

#[test]
fn small_axes_ignore_order_three_without_changing_other_dimensions() {
    for (size, values, coordinate, expected) in [
        (1, vec![255], 99.0, 1.0),
        (2, vec![0, 255], 0.5, 0.5),
        (3, vec![0, 255, 0], 0.5, 0.5),
    ] {
        close(
            evaluate(&function(&[size], 8, 1, &values, Some(3)), &[coordinate])[0],
            expected,
        );
    }
    // Dimension 0 varies fastest; only (1,1,0,1) is nonzero.
    let mut values = vec![0; (4 * 3) * 4];
    values[1 + 4 + 4 * 3] = 255;
    let object = function(&[4, 3, 1, 4], 8, 1, &values, Some(3));
    close(
        evaluate(&object, &[1.5, 0.5, 99.0, 1.5])[0],
        0.5625 * 0.5 * 0.5625,
    );
}

#[test]
fn tensor_cubic_interpolation_uses_dimension_zero_fastest_and_keeps_channels() {
    let mut values = vec![0; 4 * 4 * 2];
    values[(1 + 4) * 2] = 255;
    values[(2 + 2 * 4) * 2 + 1] = 255;
    let object = function(&[4, 4], 8, 2, &values, Some(3));
    let output = evaluate(&object, &[1.5, 1.5]);
    close(output[0], 0.5625_f64.powi(2));
    close(output[1], 0.5625_f64.powi(2));
    assert_eq!(evaluate(&object, &[1.0, 1.0]), vec![1.0, 0.0]);
    assert_eq!(evaluate(&object, &[2.0, 2.0]), vec![0.0, 1.0]);
}

#[test]
fn eight_dimensional_stencils_do_not_allocate_a_tensor_buffer() {
    let mut values = vec![0; 4_usize.pow(8)];
    values[(4_usize.pow(8) - 1) / 3] = 255;
    let object = function(&[4; 8], 8, 1, &values, Some(3));
    close(evaluate(&object, &[1.5; 8])[0], 0.5625_f64.powi(8));
}

#[test]
fn every_allowed_sample_width_has_the_same_cubic_result() {
    for bits in [1, 2, 4, 8, 12, 16, 24, 32] {
        let max = max_value(bits) as u32;
        let object = function(&[4], bits, 1, &[0, 0, max, 0], Some(3));
        close(evaluate(&object, &[1.5])[0], 0.5625);
    }
}

#[test]
fn random_access_packed_reads_match_the_independent_sequential_bit_reader() {
    for bits in [1, 2, 4, 8, 12, 16, 24, 32] {
        let max = max_value(bits) as u32;
        let values: Vec<u32> = (0_u32..31)
            .map(|v| v.wrapping_mul(0x9e3779b9) & max)
            .collect();
        let bytes = packed(&values, bits);
        let mut oracle = BitReader::new(&bytes);
        for (index, expected) in values.into_iter().enumerate() {
            assert_eq!(oracle.read(bits), Some(expected));
            assert_eq!(read_sample(&bytes, index, bits), Some(f64::from(expected)));
        }
    }
    assert_eq!(read_sample(&[0xff], usize::MAX, 32), None);
    assert_eq!(read_sample(&[0xff], 0, 3), None);
    assert_eq!(read_sample(&[0xff], 0, 12), None);
}

#[test]
fn reversed_encode_and_tiny_domains_map_into_the_same_cubic_grid() {
    let mut object = function(&[4], 8, 1, &[0, 0, 255, 0], Some(3));
    set(&mut object, "Domain", numbers(&[0.0, 2e-20]));
    set(&mut object, "Encode", numbers(&[3.0, 0.0]));
    close(evaluate(&object, &[1e-20])[0], 0.5625);
    close(evaluate(&object, &[2e-20 / 3.0])[0], 1.0);
}

#[test]
fn invalid_order_values_are_rejected_by_shape_and_evaluation() {
    let reader = reader_with_objects(&[]);
    let mut object = function(&[4], 8, 1, &[0, 0, 255, 0], Some(3));
    for value in [
        PdfObject::Integer(0),
        PdfObject::Integer(2),
        PdfObject::Integer(-1),
        PdfObject::Real(3.0),
        PdfObject::Boolean(true),
        numbers(&[3.0]),
    ] {
        set(&mut object, "Order", value);
        assert!(!validate_function_shape(&object, 1, &reader));
        assert!(eval_function_n(&object, &[1.5], &reader).is_empty());
    }
    set(&mut object, "Order", PdfObject::Null);
    close(eval_function_n(&object, &[1.5], &reader)[0], 0.5);
}

#[test]
fn indirect_order_parameters_and_function_arrays_reach_the_cubic_evaluator() {
    let reader = reader_with_objects(&[PdfObject::Integer(3), reference(4)]);
    let mut object = function(&[4], 8, 1, &[0, 0, 255, 0], None);
    set(&mut object, "Order", reference(5));
    assert!(validate_function_shape(&object, 1, &reader));
    close(eval_function_n(&object, &[1.5], &reader)[0], 0.5625);
    let array = PdfObject::Array(vec![object.clone(), object]);
    assert_eq!(
        eval_function_or_array_n(&array, &[1.5], &reader),
        vec![0.5625; 2]
    );
}

#[test]
fn interpolation_is_bounded_and_rejects_invalid_coordinates_or_short_samples() {
    assert!(interpolate(&[], &[4], &[1.5], 1, 8, Order::Cubic).is_none());
    assert!(interpolate(&[0; 4], &[4], &[f64::NAN], 1, 8, Order::Cubic).is_none());
    assert!(interpolate(&[0; 4], &[4], &[1.5], 0, 8, Order::Cubic).is_none());
    assert!(interpolate(
        &[],
        &[MAX_TYPE0_SAMPLE_VALUES + 1],
        &[0.5],
        1,
        8,
        Order::Cubic
    )
    .is_none());
    assert!(interpolate(&[], &[4], &[1.5], 1, 3, Order::Cubic).is_none());
    let reader = reader_with_objects(&[]);
    let excessive = function(&[MAX_TYPE0_SAMPLE_VALUES + 1], 8, 1, &[], Some(3));
    assert!(!validate_function_shape(&excessive, 1, &reader));
    let object = function(&[4], 8, 1, &[0, 0, 255], Some(3));
    assert!(eval_function_n(&object, &[1.5], &reader).is_empty());
    let object = function(&[4], 8, 1, &[0, 0, 255, 0], Some(3));
    assert!(resolve_stream_bytes_limited(&object, &reader, Some(3)).is_none());
}

#[test]
fn cancelled_sampling_returns_no_partial_colour() {
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| {
        assert!(interpolate(&[0, 0, 255, 0], &[4], &[1.5], 1, 8, Order::Cubic).is_none());
        assert!(evaluate(&function(&[4], 8, 1, &[0, 0, 255, 0], Some(3)), &[1.5]).is_empty());
    });
}

#[test]
fn extreme_decode_values_interpolate_before_final_range_clipping() {
    let mut object = function(&[4], 8, 1, &[0, 0, 255, 255], Some(3));
    set(&mut object, "Decode", numbers(&[-1e308, 1e308]));
    set(&mut object, "Range", numbers(&[-1.0, 1.0]));
    close(evaluate(&object, &[1.5])[0], 0.0);
    assert_eq!(evaluate(&object, &[0.5]), vec![-1.0]);
    assert_eq!(evaluate(&object, &[2.5]), vec![1.0]);
}

#[test]
fn separation_tint_transform_uses_order_three_in_the_shared_colour_path() {
    use crate::render::colorspace::{resolve_named_color, NamedColor};
    let mut transform = function(&[4], 8, 1, &[0, 0, 255, 0], Some(3));
    set(&mut transform, "Domain", numbers(&[0.0, 1.0]));
    let space = PdfObject::Array(vec![
        PdfObject::Name("Separation".into()),
        PdfObject::Name("Ink".into()),
        PdfObject::Name("DeviceGray".into()),
        transform,
    ]);
    match resolve_named_color(&space, &[0.5], 1.0, &reader_with_objects(&[])) {
        NamedColor::Color(colour) => {
            assert!((colour.r - 0.5625).abs() < 1e-6);
            assert_eq!(colour.r, colour.g);
            assert_eq!(colour.g, colour.b);
        }
        other => panic!("cubic tint failed: {other:?}"),
    }
}
