//! Unexecuted calculator conformance regression source, not runtime evidence.
use super::*;
use crate::object::{PdfDictionary, PdfObject};
use crate::render::function::{eval_function_n, validate_type4_program, PreparedFunction};
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects};

fn int(n: i32) -> Value {
    Value::Number(Number::Integer(n))
}
fn real(n: f64) -> Value {
    Value::Number(Number::Real(n))
}
fn run(program: &str) -> Result<Vec<Value>, Error> {
    let (code, _) = compile(program.as_bytes()).expect("valid calculator syntax");
    let mut stack = Vec::new();
    let mut remaining = super::super::MAX_FUNCTION_WORK;
    execute(&code, &mut stack, 0, &mut remaining)?;
    Ok(stack)
}
fn one(program: &str) -> Value {
    let stack = run(program).unwrap();
    assert_eq!(stack.len(), 1, "{program}");
    stack[0]
}
fn stream(program: &[u8], range: &[f64]) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(4));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("Range", numbers(range));
    PdfObject::Stream {
        dict,
        raw: program.to_vec(),
    }
}

#[test]
fn decimal_syntax_preserves_integer_real_types_and_promotes_large_integers() {
    assert_eq!(
        run("{ +17 -98 0001 -0 4. -.002 +.5 2147483648 }").unwrap(),
        vec![
            int(17),
            int(-98),
            int(1),
            int(0),
            real(4.0),
            real(-0.002),
            real(0.5),
            real(2147483648.0)
        ]
    );
    for token in [
        ".",
        "+",
        "--1",
        "1.2.3",
        "1e3",
        "1E-3",
        "16#FF",
        "nan",
        "inf",
        "/dup",
        "[1]",
        "(1)",
        "truefalse",
    ] {
        assert!(
            compile(format!("{{ {token} }}").as_bytes()).is_none(),
            "{token}"
        );
    }
    assert!(compile(format!("{{ {} }}", "9".repeat(400)).as_bytes()).is_none());
}

#[test]
fn scanner_uses_pdf_whitespace_and_ignores_binary_comment_bytes() {
    let (code, _) = compile(b"\0{%\xff\xfe ignored\r\n1\t2\x0cadd}\0").unwrap();
    let mut stack = Vec::new();
    execute(&code, &mut stack, 0, &mut 100).unwrap();
    assert_eq!(stack, vec![int(3)]);
    for program in [b"{1\x0b2 add}".as_slice(), b"{1\xc2\xa02 add}", b"{\xff}"] {
        assert!(compile(program).is_none());
    }
}

#[test]
fn integer_arithmetic_retains_types_and_promotes_only_on_overflow() {
    for (program, expected) in [
        ("{ 2 3 add }", int(5)),
        ("{ 2 3 sub }", int(-1)),
        ("{ 2 3 mul }", int(6)),
        ("{ 2147483647 1 add }", real(2147483648.0)),
        ("{ -2147483648 1 sub }", real(-2147483649.0)),
        ("{ 1073741824 2 mul }", real(2147483648.0)),
        ("{ 2 3.0 add }", real(5.0)),
        ("{ 2.0 3 sub }", real(-1.0)),
        ("{ 2.0 3 mul }", real(6.0)),
        ("{ 4 2 div }", real(2.0)),
    ] {
        assert_eq!(one(program), expected, "{program}");
    }
}

#[test]
fn integer_division_and_remainder_have_signed_truncation_semantics() {
    for (a, b, quotient, remainder) in [
        (5, 2, 2, 1),
        (-5, 2, -2, -1),
        (5, -2, -2, 1),
        (-5, -2, 2, -1),
    ] {
        assert_eq!(one(&format!("{{ {a} {b} idiv }}")), int(quotient));
        assert_eq!(one(&format!("{{ {a} {b} mod }}")), int(remainder));
    }
    assert_eq!(run("{ -2147483648 -1 idiv }"), Err(Error::UndefinedResult));
    assert_eq!(one("{ -2147483648 -1 mod }"), int(0));
    for program in ["{ 4.0 2 idiv }", "{ 4 2.0 mod }", "{ true 2 idiv }"] {
        assert_eq!(run(program), Err(Error::TypeCheck), "{program}");
    }
}

#[test]
fn unary_integer_overflow_promotes_without_changing_ordinary_types() {
    for (program, expected) in [
        ("{ -2147483648 neg }", real(2147483648.0)),
        ("{ -2147483648 abs }", real(2147483648.0)),
        ("{ -7 abs }", int(7)),
        ("{ 7 neg }", int(-7)),
        ("{ -7.0 abs }", real(7.0)),
        ("{ 7.0 neg }", real(-7.0)),
    ] {
        assert_eq!(one(program), expected, "{program}");
    }
}

#[test]
fn rounding_uses_positive_ties_and_preserves_numeric_type() {
    for (input, expected) in [
        (6.5, 7.0),
        (-6.5, -6.0),
        (-4.8, -5.0),
        (-0.5, 0.0),
        (0.5, 1.0),
        (3.2, 3.0),
    ] {
        assert_eq!(one(&format!("{{ {input} round }}")), real(expected));
    }
    for operator in ["round", "truncate", "floor", "ceiling"] {
        assert_eq!(one(&format!("{{ 99 {operator} }}")), int(99));
        assert_eq!(one(&format!("{{ 99.0 {operator} }}")), real(99.0));
    }
    assert_eq!(
        one("{ 4503599627370497.0 round }"),
        real(4503599627370497.0)
    );
    assert_eq!(one("{ -4.8 truncate }"), real(-4.0));
    assert_eq!(one("{ -4.8 ceiling }"), real(-4.0));
    assert_eq!(one("{ -4.8 floor }"), real(-5.0));
}

#[test]
fn explicit_conversions_check_type_and_integer_range() {
    assert_eq!(one("{ -47.8 cvi }"), int(-47));
    assert_eq!(one("{ 2147483647.99 cvi }"), int(i32::MAX));
    assert_eq!(one("{ -2147483648.99 cvi }"), int(i32::MIN));
    assert_eq!(one("{ 12 cvr }"), real(12.0));
    assert_eq!(one("{ 12.5 cvr }"), real(12.5));
    for program in ["{ 2147483648.0 cvi }", "{ -2147483649.0 cvi }"] {
        assert_eq!(run(program), Err(Error::RangeCheck));
    }
    assert_eq!(run("{ true cvr }"), Err(Error::TypeCheck));
    assert_eq!(run("{ cvr }"), Err(Error::StackUnderflow));
}

#[test]
fn every_boolean_overload_is_distinct_from_numeric_coercion() {
    for a in [false, true] {
        for b in [false, true] {
            for (operator, result) in [("and", a & b), ("or", a | b), ("xor", a ^ b)] {
                assert_eq!(
                    one(&format!("{{ {a} {b} {operator} }}")),
                    Value::Bool(result)
                );
            }
        }
    }
    assert_eq!(one("{ false not }"), Value::Bool(true));
    assert_eq!(one("{ true not }"), Value::Bool(false));
    for program in [
        "{ true 1 and }",
        "{ 1 false or }",
        "{ 1.0 2 xor }",
        "{ 1.0 not }",
        "{ true 1 add }",
    ] {
        assert_eq!(run(program), Err(Error::TypeCheck), "{program}");
    }
}

#[test]
fn bitwise_operators_keep_portable_32_bit_twos_complement() {
    assert_eq!(one("{ 15 10 and }"), int(10));
    assert_eq!(one("{ 5 10 or }"), int(15));
    assert_eq!(one("{ 15 10 xor }"), int(5));
    assert_eq!(one("{ 0 not }"), int(-1));
    assert_eq!(one("{ -1 not }"), int(0));
    for (program, expected) in [
        ("{ -1 -1 bitshift }", i32::MAX),
        ("{ 1 31 bitshift }", i32::MIN),
        ("{ -2147483648 -31 bitshift }", 1),
        ("{ -1 32 bitshift }", 0),
        ("{ -1 -32 bitshift }", 0),
        ("{ 1 -2147483648 bitshift }", 0),
        ("{ 1 2147483647 bitshift }", 0),
    ] {
        assert_eq!(one(program), int(expected), "{program}");
    }
    assert_eq!(run("{ 1 1.0 bitshift }"), Err(Error::TypeCheck));
}

#[test]
fn equality_supports_booleans_mixed_numbers_and_different_types() {
    for (a, b, equal) in [
        ("true", "true", true),
        ("false", "true", false),
        ("1", "1.0", true),
        ("1", "true", false),
        ("0", "false", false),
    ] {
        assert_eq!(one(&format!("{{ {a} {b} eq }}")), Value::Bool(equal));
        assert_eq!(one(&format!("{{ {a} {b} ne }}")), Value::Bool(!equal));
    }
    for (operator, expected) in [("gt", false), ("ge", true), ("lt", false), ("le", true)] {
        assert_eq!(
            one(&format!("{{ 2 2.0 {operator} }}")),
            Value::Bool(expected)
        );
        assert_eq!(
            run(&format!("{{ true false {operator} }}")),
            Err(Error::TypeCheck)
        );
    }
}

#[test]
fn undefined_math_does_not_fabricate_zero() {
    for program in [
        "{ 1 0 div }",
        "{ 1 0 idiv }",
        "{ 1 0 mod }",
        "{ -1 sqrt }",
        "{ 0 ln }",
        "{ -1 log }",
        "{ 0 0 atan }",
        "{ -1 .5 exp }",
        "{ 0 -1 exp }",
        "{ 10 400 exp }",
    ] {
        assert_eq!(run(program), Err(Error::UndefinedResult), "{program}");
    }
    assert_eq!(one("{ 9 sqrt }"), real(3.0));
    assert_eq!(one("{ 9 .5 exp }"), real(3.0));
    assert_eq!(one("{ 1 ln }"), real(0.0));
    assert_eq!(one("{ 100 log }"), real(2.0));
}

#[test]
fn trigonometry_preserves_quadrants_and_reduces_large_finite_angles() {
    for (angle, sine, cosine) in [
        (0, 0.0, 1.0),
        (90, 1.0, 0.0),
        (180, 0.0, -1.0),
        (270, -1.0, 0.0),
        (-90, -1.0, 0.0),
        (450, 1.0, 0.0),
    ] {
        assert_eq!(one(&format!("{{ {angle} sin }}")), real(sine));
        assert_eq!(one(&format!("{{ {angle} cos }}")), real(cosine));
    }
    for (y, x, angle) in [(0, 1, 0.0), (1, 0, 90.0), (0, -1, 180.0), (-1, 0, 270.0)] {
        assert_eq!(one(&format!("{{ {y} {x} atan }}")), real(angle));
    }
    let (code, _) = compile(b"{ sin }").unwrap();
    let mut stack = initial_stack(&[f64::MAX]).unwrap();
    execute(&code, &mut stack, 0, &mut 100).unwrap();
    let output = numeric_outputs(&stack, 1).unwrap()[0];
    assert!(output.is_finite() && (-1.0..=1.0).contains(&output));
}

#[test]
fn stack_operations_retain_value_types_and_use_integer_indices() {
    assert_eq!(
        run("{ true 2.0 2 copy }").unwrap(),
        vec![Value::Bool(true), real(2.0), Value::Bool(true), real(2.0)]
    );
    assert_eq!(
        run("{ 1 2 3 3 1 roll }").unwrap(),
        vec![int(3), int(1), int(2)]
    );
    assert_eq!(
        run("{ 1 2 3 3 -1 roll }").unwrap(),
        vec![int(2), int(3), int(1)]
    );
    assert_eq!(
        run("{ true 2 1 index exch pop }").unwrap(),
        vec![Value::Bool(true), Value::Bool(true)]
    );
    assert_eq!(run("{ 7 dup exch pop }").unwrap(), vec![int(7)]);
    for program in ["{ 1 1.0 copy }", "{ 1 0.0 index }", "{ 1 1 1.0 roll }"] {
        assert_eq!(run(program), Err(Error::TypeCheck));
    }
    for program in ["{ 1 -1 copy }", "{ 1 -1 index }", "{ 1 -1 0 roll }"] {
        assert_eq!(run(program), Err(Error::RangeCheck));
    }
    for program in [
        "{ pop }",
        "{ exch }",
        "{ 1 2 copy }",
        "{ 1 1 index }",
        "{ 1 2 0 roll }",
    ] {
        assert_eq!(run(program), Err(Error::StackUnderflow));
    }
}

#[test]
fn conditional_branches_are_syntax_and_only_selected_math_executes() {
    assert_eq!(one("{true{1}{1 0 div}ifelse}"), int(1));
    assert_eq!(one("{ false { 1 0 div } { 2 } ifelse }"), int(2));
    assert_eq!(one("{ 7 false { pop 8 } if }"), int(7));
    assert_eq!(
        one("{ true { false { 1 } { 2 } ifelse } { 3 } ifelse }"),
        int(2)
    );
    assert_eq!(run("{ 1 { 2 } if }"), Err(Error::TypeCheck));
    for program in [
        "{ { 1 } }",
        "{ true { 1 } dup if }",
        "{ true { 1 } pop }",
        "{ true if }",
        "{ true { 1 } ifelse }",
        "{ true { 1 } { 2 } if }",
        "{ false { unknown } if }",
    ] {
        assert!(compile(program.as_bytes()).is_none(), "{program}");
    }
}

#[test]
fn exact_numeric_output_arity_is_checked_after_selected_branch() {
    let reader = reader_with_objects(&[]);
    for (program, expected) in [
        (b"{ pop .25 }".as_slice(), vec![0.25]),
        (b"{ .25 }", vec![]),
        (b"{ pop }", vec![]),
        (b"{ pop false }", vec![]),
    ] {
        let object = stream(program, &[0.0, 1.0]);
        let prepared = PreparedFunction::prepare(&object, 1, &reader).unwrap();
        assert_eq!(prepared.evaluate(&[0.5]), expected);
        assert_eq!(eval_function_n(&object, &[0.5], &reader), expected);
    }
    let branch = stream(b"{ .5 lt { 1 } { 1 2 } ifelse }", &[0.0, 1.0]);
    assert_eq!(eval_function_n(&branch, &[0.25], &reader), vec![1.0]);
    assert!(eval_function_n(&branch, &[0.75], &reader).is_empty());
}

#[test]
fn real_function_inputs_need_explicit_conversion_for_integer_operations() {
    let reader = reader_with_objects(&[]);
    assert!(eval_function_n(&stream(b"{ 1 and }", &[0.0, 1.0]), &[1.0], &reader).is_empty());
    assert_eq!(
        eval_function_n(&stream(b"{ cvi 1 and }", &[0.0, 1.0]), &[1.0], &reader),
        vec![1.0]
    );
}

#[test]
fn stack_work_and_conditional_depth_limits_are_enforced() {
    assert_eq!(
        run(&format!("{{ {} }}", "1 ".repeat(MAX_TYPE4_STACK + 1))),
        Err(Error::StackOverflow)
    );
    let (code, _) = compile(b"{ true { true { 1 } if } if }").unwrap();
    assert_eq!(
        execute(&code, &mut Vec::new(), 0, &mut 5),
        Err(Error::WorkLimit)
    );
    let permitted = format!(
        "{{ {}1{} }}",
        "true { ".repeat(MAX_DEPTH),
        " } if".repeat(MAX_DEPTH)
    );
    assert_eq!(one(&permitted), int(1));
    let excessive = format!(
        "{{ {}1{} }}",
        "true { ".repeat(MAX_DEPTH + 1),
        " } if".repeat(MAX_DEPTH + 1)
    );
    assert!(compile(excessive.as_bytes()).is_none());
}

#[test]
fn program_token_caps_and_cancellation_apply_before_runtime() {
    assert!(compile(&vec![b' '; MAX_TYPE4_PROGRAM_BYTES + 1]).is_none());
    assert!(compile(format!("{{ {} }}", "1 pop ".repeat(MAX_TYPE4_TOKENS)).as_bytes()).is_none());
    let (code, _) = compile(b"{ 1 }").unwrap();
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| {
        assert!(compile(b"{ 1 }").is_none());
        assert_eq!(
            execute(&code, &mut Vec::new(), 0, &mut 100),
            Err(Error::Cancelled)
        );
    });
}

#[test]
fn editing_program_validation_uses_same_syntax_not_all_input_proof() {
    assert!(validate_type4_program(
        b"{ dup .5 gt { 1 } { 0 } ifelse exch pop }"
    ));
    assert!(!validate_type4_program(b"{ true { 1 } dup if }"));
    assert!(!validate_type4_program(b"{ 1e2 }"));
    // Syntactically valid, but runtime rejects the arithmetic and output shape.
    assert!(validate_type4_program(b"{ 1 0 div }"));
    assert!(eval_function_n(
        &stream(b"{ 1 0 div }", &[0.0, 1.0]),
        &[0.5],
        &reader_with_objects(&[])
    )
    .is_empty());
}

#[test]
fn separation_tint_uses_typed_logic_and_exact_three_component_output() {
    use crate::render::colorspace::{resolve_named_color, NamedColor};
    let reader = reader_with_objects(&[]);
    let function = stream(
        b"{ pop -6.5 round -6 eq -1 -1 bitshift 2147483647 eq and { .75 0 .25 } { 0 0 0 } ifelse }",
        &[0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
    );
    let space = PdfObject::Array(vec![
        PdfObject::Name("Separation".into()),
        PdfObject::Name("TypedInk".into()),
        PdfObject::Name("DeviceRGB".into()),
        function,
    ]);
    let NamedColor::Color(color) = resolve_named_color(&space, &[0.5], 1.0, &reader) else {
        panic!("typed tint colour");
    };
    assert!((color.r - 0.75).abs() < 1e-6);
    assert_eq!(color.g, 0.0);
    assert!((color.b - 0.25).abs() < 1e-6);
}
