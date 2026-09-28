//! Unexecuted function-semantic regressions, not runtime qualification.
use super::*;
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects, reference};

fn exponential(domain: [f64; 2], c0: &[f64], c1: &[f64], n: f64) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(2));
    dict.insert("Domain", numbers(&domain));
    dict.insert("C0", numbers(c0));
    dict.insert("C1", numbers(c1));
    dict.insert("N", PdfObject::Real(n));
    dict
}
fn calc(domain: &[f64], range: &[f64], program: &[u8]) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(4));
    dict.insert("Domain", numbers(domain));
    dict.insert("Range", numbers(range));
    PdfObject::Stream {
        dict,
        raw: program.to_vec(),
    }
}
fn stitching(
    domain: [f64; 2],
    children: Vec<PdfObject>,
    bounds: &[f64],
    encode: &[f64],
) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(3));
    dict.insert("Domain", numbers(&domain));
    dict.insert("Functions", PdfObject::Array(children));
    dict.insert("Bounds", numbers(bounds));
    dict.insert("Encode", numbers(encode));
    dict
}
fn evaluate(dict: PdfDictionary, input: f64) -> Vec<f64> {
    eval_function_n(
        &PdfObject::Dictionary(dict),
        &[input],
        &reader_with_objects(&[]),
    )
}
fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
}

#[test]
fn exponential_uses_actual_input_and_preserves_nonunit_outputs() {
    let dict = exponential([2.0, 4.0], &[-10.0, 100.0], &[-9.0, 102.0], 1.0);
    assert_eq!(evaluate(dict.clone(), 3.0), vec![-7.0, 106.0]);
    assert_eq!(evaluate(dict, 9.0), vec![-6.0, 108.0]);
    assert_eq!(
        evaluate(exponential([2.0, 4.0], &[0.0], &[1.0], 2.0), 3.0),
        vec![9.0]
    );
}

#[test]
fn exponential_range_is_optional_and_clips_each_declared_channel() {
    let mut dict = exponential([2.0, 4.0], &[-10.0, 100.0], &[-9.0, 102.0], 1.0);
    dict.insert("Range", numbers(&[-8.0, -6.0, 101.0, 105.0]));
    assert_eq!(evaluate(dict, 3.0), vec![-7.0, 105.0]);
}

#[test]
fn exponential_rejects_undefined_exponents_and_accepts_integer_negative_inputs() {
    assert!(evaluate(exponential([-1.0, 1.0], &[0.0], &[1.0], 0.5), 0.5).is_empty());
    assert!(evaluate(exponential([0.0, 1.0], &[0.0], &[1.0], -1.0), 0.5).is_empty());
    assert_eq!(
        evaluate(exponential([-3.0, -1.0], &[0.0], &[1.0], 2.0), -2.0),
        vec![4.0]
    );
    close(
        evaluate(exponential([1.0, 4.0], &[0.0], &[1.0], -1.0), 2.0)[0],
        0.5,
    );
}

#[test]
fn calculator_clips_every_input_before_executing_the_program() {
    let reader = reader_with_objects(&[]);
    let one = calc(&[0.0, 1.0], &[0.0, 100.0], b"{ dup mul }");
    assert_eq!(eval_function_n(&one, &[-10.0], &reader), vec![0.0]);
    assert_eq!(eval_function_n(&one, &[10.0], &reader), vec![1.0]);
    let two = calc(&[0.0, 1.0, 10.0, 20.0], &[0.0, 100.0], b"{ add }");
    assert_eq!(eval_function_n(&two, &[2.0, -5.0], &reader), vec![11.0]);
}

#[test]
fn sampled_functions_keep_tiny_domains_and_signed_decode_values() {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(0));
    dict.insert("Domain", numbers(&[0.0, 1e-20]));
    dict.insert("Range", numbers(&[-10.0, 20.0]));
    dict.insert("Decode", numbers(&[-20.0, 40.0]));
    dict.insert("Size", PdfObject::Array(vec![PdfObject::Integer(2)]));
    dict.insert("BitsPerSample", PdfObject::Integer(8));
    let object = PdfObject::Stream {
        dict,
        raw: vec![0, 255],
    };
    close(
        eval_function_n(&object, &[0.5e-20], &reader_with_objects(&[]))[0],
        10.0,
    );
}

#[test]
fn stitching_uses_exact_half_open_boundaries_and_parent_range() {
    let child = || PdfObject::Dictionary(exponential([0.0, 1.0], &[0.0], &[1.0], 1.0));
    let mut dict = stitching(
        [0.0, 2e-20],
        vec![child(), child()],
        &[1e-20],
        &[0.0, 1.0, 1.0, 0.0],
    );
    close(evaluate(dict.clone(), 0.5e-20)[0], 0.5);
    close(evaluate(dict.clone(), 1.5e-20)[0], 0.5);
    dict.insert("Range", numbers(&[0.0, 0.6]));
    close(evaluate(dict, 1e-20)[0], 0.6);
    let first = PdfObject::Dictionary(exponential([0.0, 1.0], &[2.0], &[2.0], 1.0));
    let last = PdfObject::Dictionary(exponential([0.0, 1.0], &[5.0], &[5.0], 1.0));
    let dict = stitching([0.0, 1.0], vec![first, last], &[0.5], &[0.0, 1.0, 0.0, 1.0]);
    assert_eq!(evaluate(dict, 0.5), vec![5.0]);
}

#[test]
fn stitching_empty_last_interval_uses_its_encode_start() {
    let child = || PdfObject::Dictionary(exponential([0.0, 1.0], &[0.0], &[1.0], 1.0));
    let dict = stitching(
        [0.0, 1.0],
        vec![child(), child()],
        &[1.0],
        &[0.0, 1.0, 0.75, 0.0],
    );
    assert_eq!(evaluate(dict, 1.0), vec![0.75]);
}

#[test]
fn invalid_ranges_bounds_dimensions_and_output_arities_are_rejected() {
    let reader = reader_with_objects(&[]);
    for values in [vec![1.0, 0.0], vec![0.0, 1.0, 0.0, 1.0]] {
        let mut dict = exponential([0.0, 1.0], &[0.0], &[1.0], 1.0);
        dict.insert("Range", numbers(&values));
        let obj = PdfObject::Dictionary(dict);
        assert!(!validate_function_shape(&obj, 1, &reader));
        assert!(eval_function_n(&obj, &[0.5], &reader).is_empty());
    }
    let a = PdfObject::Dictionary(exponential([0.0, 1.0], &[0.0], &[1.0], 1.0));
    let b = PdfObject::Dictionary(exponential([0.0, 1.0], &[0.0, 0.0], &[1.0, 1.0], 1.0));
    let dict = stitching(
        [0.0, 1.0],
        vec![a.clone(), b],
        &[0.5],
        &[0.0, 1.0, 0.0, 1.0],
    );
    assert!(evaluate(dict, 0.25).is_empty());
    let dict = stitching(
        [0.0, 1.0],
        vec![a.clone(), a.clone(), a.clone()],
        &[0.75, 0.5],
        &[0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
    );
    assert!(evaluate(dict, 0.25).is_empty());
    assert!(!validate_function_shape(&a, 2, &reader));
    assert!(eval_function_n(&a, &[0.5, 0.75], &reader).is_empty());
}

#[test]
fn indirect_function_fields_and_arrays_follow_reference_chains() {
    let reader = reader_with_objects(&[
        PdfObject::Integer(2),
        numbers(&[2.0, 4.0]),
        numbers(&[-10.0]),
        numbers(&[-9.0]),
        PdfObject::Integer(1),
        PdfObject::Array(vec![reference(11)]),
        reference(9),
        PdfObject::Dictionary({
            let mut d = PdfDictionary::empty();
            for (k, n) in [
                ("FunctionType", 4),
                ("Domain", 5),
                ("C0", 6),
                ("C1", 7),
                ("N", 8),
            ] {
                d.insert(k, reference(n));
            }
            d
        }),
    ]);
    assert!(validate_function_or_array_shape(&reference(10), 1, &reader));
    assert_eq!(
        eval_function_or_array_n(&reference(10), &[3.0], &reader),
        vec![-7.0]
    );
}

#[test]
fn recursive_function_shape_traversal_has_a_shared_visit_budget() {
    let leaf = PdfObject::Dictionary(exponential([0.0, 1.0], &[0.0], &[1.0], 1.0));
    let object = PdfObject::Dictionary(stitching(
        [0.0, 1.0],
        vec![leaf.clone(), leaf],
        &[0.5],
        &[0.0, 1.0, 0.0, 1.0],
    ));
    let reader = reader_with_objects(&[]);
    assert_eq!(
        validate_function_shape_inner(&object, 1, &reader, 0, &mut 2),
        None
    );
    assert_eq!(
        validate_function_shape_inner(&object, 1, &reader, 0, &mut 3),
        Some(1)
    );
    let cycle = PdfObject::Dictionary(stitching([0.0, 1.0], vec![reference(4)], &[], &[0.0, 1.0]));
    let reader = reader_with_objects(&[cycle]);
    assert!(!validate_function_shape(&reference(4), 1, &reader));
    assert!(eval_function_n(&reference(4), &[0.5], &reader).is_empty());
}

#[test]
fn interval_mapping_covers_tiny_huge_and_degenerate_domains() {
    close(domain_position(0.5e-300, 0.0, 1e-300).unwrap(), 0.5);
    close(domain_position(0.0, -f64::MAX, f64::MAX).unwrap(), 0.5);
    assert_eq!(domain_position(2.0, 2.0, 2.0), Some(0.0));
    assert_eq!(domain_position(0.5, 1.0, 0.0), None);
}
