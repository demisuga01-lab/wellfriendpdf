//! Source regressions only; execution is deferred to the authorized VPS phase.
use super::*;
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects, reference};

fn exponential(c0: f64, c1: f64) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(2));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("C0", numbers(&[c0]));
    dict.insert("C1", numbers(&[c1]));
    dict.insert("N", PdfObject::Integer(1));
    PdfObject::Dictionary(dict)
}

fn sampled(order: i64) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(0));
    dict.insert("Domain", numbers(&[0.0, 3.0]));
    dict.insert("Range", numbers(&[0.0, 1.0]));
    dict.insert("Size", PdfObject::Array(vec![PdfObject::Integer(4)]));
    dict.insert("BitsPerSample", PdfObject::Integer(8));
    dict.insert("Order", PdfObject::Integer(order));
    PdfObject::Stream {
        dict,
        raw: vec![0, 0, 255, 0],
    }
}

fn calculator(program: &[u8]) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(4));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("Range", numbers(&[0.0, 1.0]));
    PdfObject::Stream {
        dict,
        raw: program.to_vec(),
    }
}

fn set(object: &mut PdfObject, key: &str, value: PdfObject) {
    match object {
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
            dict.insert(key, value);
        }
        _ => panic!("function expected"),
    }
}

fn stitching(children: Vec<PdfObject>, bounds: &[f64]) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(3));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("Bounds", numbers(bounds));
    dict.insert("Encode", numbers(&[0.0, 1.0].repeat(children.len())));
    dict.insert("Functions", PdfObject::Array(children));
    PdfObject::Dictionary(dict)
}

fn close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-12, "{actual:?} != {expected:?}");
    }
}

fn builder(reader: &PdfReader, bytes: usize) -> Builder<'_> {
    Builder::new(
        reader,
        FunctionResources {
            max_graph_bytes: bytes,
            ..FunctionResources::default()
        },
    )
}

#[test]
fn prepared_samples_match_direct_evaluation_repeatedly() {
    let reader = reader_with_objects(&[]);
    for order in [1, 3] {
        let object = sampled(order);
        let prepared = PreparedFunction::prepare(&object, 1, &reader).unwrap();
        for input in [-100.0, 0.0, 0.25, 1.0, 1.5, 2.0, 2.75, 3.0, 100.0] {
            close(
                &prepared.evaluate(&[input]),
                &eval_function_n(&object, &[input], &reader),
            );
        }
        close(
            &prepared.evaluate(&[1.5]),
            &[if order == 3 { 0.5625 } else { 0.5 }],
        );
    }
}

#[test]
fn prepared_graph_outlives_reader_and_source_bytes() {
    let prepared = {
        let reader = reader_with_objects(&[sampled(3)]);
        PreparedFunction::prepare(&reference(4), 1, &reader).unwrap()
    };
    close(&prepared.evaluate(&[1.5]), &[0.5625]);
}

#[test]
fn aliases_share_decoded_nodes_and_are_charged_once() {
    let reader = reader_with_objects(&[reference(5), sampled(3), reference(4)]);
    let mut builder = builder(&reader, MAX_RETAINED_BYTES);
    let first = builder.node(&reference(4), 1, 0, 0).unwrap();
    let remaining = builder.bytes;
    let second = builder.node(&reference(6), 1, 0, 0).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(builder.bytes, remaining);
    let array = PdfObject::Array(vec![reference(4), reference(5), reference(6)]);
    let prepared = PreparedFunction::prepare(&array, 1, &reader).unwrap();
    assert!(prepared
        .roots
        .windows(2)
        .all(|pair| Arc::ptr_eq(&pair[0], &pair[1])));
    close(&prepared.evaluate(&[1.5]), &[0.5625; 3]);
}

#[test]
fn parent_and_child_ranges_and_exact_terminal_interval_survive_preparation() {
    let reader = reader_with_objects(&[]);
    let mut first = exponential(-2.0, 2.0);
    set(&mut first, "Range", numbers(&[0.1, 0.9]));
    let mut object = stitching(vec![first, exponential(4.0, 5.0)], &[1.0]);
    set(&mut object, "Range", numbers(&[0.2, 0.8]));
    let prepared = PreparedFunction::prepare(&object, 1, &reader).unwrap();
    for value in [-1.0, 0.0, 0.25, 0.5, 0.99, 1.0, 2.0] {
        close(
            &prepared.evaluate(&[value]),
            &eval_function_n(&object, &[value], &reader),
        );
    }
    close(&prepared.evaluate(&[1.0]), &[0.8]);
}

#[test]
fn prepared_exponential_uses_nonunit_input_not_normalized_position() {
    let reader = reader_with_objects(&[]);
    let mut object = exponential(0.0, 1.0);
    set(&mut object, "Domain", numbers(&[2.0, 4.0]));
    set(&mut object, "N", PdfObject::Integer(2));
    let prepared = PreparedFunction::prepare(&object, 1, &reader).unwrap();
    close(&prepared.evaluate(&[3.0]), &[9.0]);
    close(&prepared.evaluate(&[-1.0]), &[4.0]);
}

#[test]
fn sampled_decode_extremes_clip_after_interpolation() {
    let reader = reader_with_objects(&[]);
    let mut object = sampled(3);
    set(&mut object, "Decode", numbers(&[-f64::MAX, f64::MAX]));
    set(&mut object, "Range", numbers(&[-1.0, 1.0]));
    let prepared = PreparedFunction::prepare(&object, 1, &reader).unwrap();
    for x in [0.25, 1.5, 2.0, 2.75] {
        close(
            &prepared.evaluate(&[x]),
            &eval_function_n(&object, &[x], &reader),
        );
    }
}

#[test]
fn two_input_calculator_array_uses_shared_input_and_distinct_outputs() {
    let reader = reader_with_objects(&[]);
    let mut x = calculator(b"{ pop }");
    let mut y = calculator(b"{ exch pop }");
    for object in [&mut x, &mut y] {
        set(object, "Domain", numbers(&[0.0, 1.0, 0.0, 1.0]));
    }
    let array = PdfObject::Array(vec![x, y]);
    let prepared = PreparedFunction::prepare(&array, 2, &reader).unwrap();
    close(&prepared.evaluate(&[0.25, 0.75]), &[0.25, 0.75]);
    close(&prepared.evaluate(&[-1.0, 2.0]), &[0.0, 1.0]);
    assert!(PreparedFunction::prepare(&array, 1, &reader).is_none());
    assert!(prepared.evaluate(&[0.5]).is_empty());
    assert!(prepared.evaluate(&[f64::NAN, 0.5]).is_empty());
}

#[test]
fn calculator_nested_procedures_are_precompiled_and_reusable() {
    let reader = reader_with_objects(&[]);
    let object = calculator(b"{ dup .5 lt { dup mul } { 1 exch sub } ifelse }");
    let prepared = PreparedFunction::prepare(&object, 1, &reader).unwrap();
    let Kind::Calculator(program) = &prepared.roots[0].kind else {
        panic!("calculator");
    };
    assert!(program
        .iter()
        .any(|token| matches!(token, calculator::Instruction::Branch { .. })));
    close(&prepared.evaluate(&[0.25]), &[0.0625]);
    close(&prepared.evaluate(&[0.75]), &[0.25]);
    close(&prepared.evaluate(&[0.25]), &[0.0625]);
}

#[test]
fn nested_calculator_calls_share_budget_without_resetting_it() {
    let (program, _) = compile_type4_program(b"{ true { true { 1 } if } if }").unwrap();
    let mut remaining = 5;
    assert!(exec_ps_with_budget(&program, &mut Vec::new(), 0, &mut remaining).is_err());
    let mut remaining = 100;
    let mut stack = Vec::new();
    exec_ps_with_budget(&program, &mut stack, 0, &mut remaining).unwrap();
    assert_eq!(calculator::numeric_outputs(&stack, 1), Some(vec![1.0]));
}

#[test]
fn component_arrays_share_one_work_budget() {
    let reader = reader_with_objects(&[]);
    let object = exponential(0.0, 1.0);
    let single = PreparedFunction::prepare(&object, 1, &reader).unwrap();
    let array =
        PreparedFunction::prepare(&PdfObject::Array(vec![object.clone(), object]), 1, &reader)
            .unwrap();
    assert!(single.evaluate_with_budget(&[0.5], &mut 4).is_some());
    assert!(array.evaluate_with_budget(&[0.5], &mut 4).is_none());
    close(&array.evaluate(&[0.5]), &[0.5, 0.5]);
}

#[test]
fn sampled_taps_consume_shared_work_before_output_allocation() {
    let reader = reader_with_objects(&[]);
    let prepared = PreparedFunction::prepare(&sampled(3), 1, &reader).unwrap();
    assert!(prepared.evaluate_with_budget(&[1.5], &mut 4).is_none());
    assert!(prepared.evaluate_with_budget(&[1.5], &mut 100).is_some());
}

#[test]
fn preparation_limits_retained_bytes_visits_cycles_and_memoized_depth() {
    let reader = reader_with_objects(&[exponential(0.0, 1.0)]);
    let mut limited = builder(&reader, 1);
    assert!(limited.node(&sampled(3), 1, 0, 0).is_none());
    let mut limited = builder(&reader, MAX_RETAINED_BYTES);
    limited.visits = 0;
    assert!(limited.node(&reference(4), 1, 0, 0).is_none());

    let cycle = reader_with_objects(&[stitching(vec![reference(4)], &[])]);
    assert!(PreparedFunction::prepare(&reference(4), 1, &cycle).is_none());
    let alias_cycle = reader_with_objects(&[reference(5), reference(4)]);
    assert!(PreparedFunction::prepare(&reference(4), 1, &alias_cycle).is_none());

    let reader = reader_with_objects(&[stitching(vec![exponential(0.0, 1.0)], &[])]);
    let mut cached = builder(&reader, MAX_RETAINED_BYTES);
    cached.node(&reference(4), 1, 0, 0).unwrap();
    assert!(cached.node(&reference(4), 1, 16, 0).is_none());
    assert!(cached.node(&reference(4), 1, 15, 0).is_some());
}

#[test]
fn malformed_and_oversized_programs_fail_before_evaluation() {
    let reader = reader_with_objects(&[]);
    for bytes in [
        b"dup".as_slice(),
        b"{ 1 } { 2 }",
        b"{ true { 1 }",
        b"{ nan }",
        b"{ unknown }",
        &[0xff],
    ] {
        assert!(PreparedFunction::prepare(&calculator(bytes), 1, &reader).is_none());
        assert!(!validate_type4_program(bytes));
    }
    assert!(!validate_type4_program(&vec![
        b' ';
        MAX_TYPE4_PROGRAM_BYTES + 1
    ]));
    let oversized = format!("{{ {} }}", "1 pop ".repeat(MAX_TYPE4_TOKENS));
    assert!(!validate_type4_program(oversized.as_bytes()));
    let too_deep = format!("{{ {}1{} }}", "true { ".repeat(65), " } if".repeat(65));
    assert!(!validate_type4_program(too_deep.as_bytes()));
}

#[test]
fn malformed_graphs_do_not_prepare_partial_component_arrays() {
    let reader = reader_with_objects(&[]);
    let mut multi = exponential(0.0, 1.0);
    set(&mut multi, "C0", numbers(&[0.0, 0.0]));
    set(&mut multi, "C1", numbers(&[1.0, 1.0]));
    assert!(
        PreparedFunction::prepare(&PdfObject::Array(vec![multi.clone()]), 1, &reader).is_none()
    );
    let mismatched = stitching(vec![exponential(0.0, 1.0), multi], &[0.5]);
    assert!(PreparedFunction::prepare(&mismatched, 1, &reader).is_none());
    let mut short = sampled(3);
    if let PdfObject::Stream { raw, .. } = &mut short {
        raw.clear();
    }
    assert!(PreparedFunction::prepare(&short, 1, &reader).is_none());
}

#[test]
fn preparation_and_cached_evaluation_both_observe_cancellation() {
    let reader = reader_with_objects(&[]);
    let object = sampled(3);
    let prepared = PreparedFunction::prepare(&object, 1, &reader).unwrap();
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| {
        assert!(PreparedFunction::prepare(&object, 1, &reader).is_none());
        assert!(prepared.evaluate(&[1.5]).is_empty());
    });
    close(&prepared.evaluate(&[1.5]), &[0.5625]);
}

#[test]
fn calculator_integer_overflow_fails_closed_without_division_panic() {
    let (division, _) = compile_type4_program(b"{ -2147483648 -1 idiv }").unwrap();
    assert!(exec_ps_with_budget(&division, &mut Vec::new(), 0, &mut 1000).is_err());
    let (remainder, _) = compile_type4_program(b"{ -2147483648 -1 mod }").unwrap();
    let mut stack = Vec::new();
    exec_ps_with_budget(&remainder, &mut stack, 0, &mut 1000).unwrap();
    assert_eq!(calculator::numeric_outputs(&stack, 1), Some(vec![0.0]));
}
