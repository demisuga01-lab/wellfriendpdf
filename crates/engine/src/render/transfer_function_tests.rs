//! Unexecuted scalar transfer and full-LUT regression source.
use super::*;
use crate::object::PdfDictionary;
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects, reference};

fn calculator(program: &[u8], outputs: usize) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(4));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("Range", numbers(&[0.0, 1.0].repeat(outputs)));
    PdfObject::Stream {
        dict,
        raw: program.to_vec(),
    }
}

#[test]
fn transfer_lut_reuses_one_graph_and_matches_every_scalar_sample() {
    let object = calculator(b"{ dup mul }", 1);
    let reader = reader_with_objects(&[object.clone()]);
    let transfer = PreparedTransfer::prepare(&reference(4), &reader).unwrap();
    let second = PreparedTransfer::prepare(&reference(4), &reader).unwrap();
    assert!(Arc::ptr_eq(
        transfer.function.as_ref().unwrap(),
        second.function.as_ref().unwrap()
    ));
    let table = transfer.lookup_table().unwrap();
    for (index, actual) in table.into_iter().enumerate() {
        let input = index as f64 / 255.0;
        assert_eq!(actual, (input * input * 255.0).round() as u8);
    }
    assert_eq!(transfer.evaluate(0.5), Some(0.25));
}

#[test]
fn identity_and_indirect_identity_are_exact_and_cancellable() {
    let identity = PdfObject::Name("Identity".into());
    let reader = reader_with_objects(&[identity.clone()]);
    for source in [identity, reference(4)] {
        let transfer = PreparedTransfer::prepare(&source, &reader).unwrap();
        assert!(transfer.is_identity());
        assert_eq!(
            transfer.lookup_table().unwrap(),
            std::array::from_fn(|index| index as u8)
        );
        assert_eq!(transfer.evaluate(-1.0), Some(0.0));
        assert_eq!(transfer.evaluate(2.0), Some(1.0));
        assert!(transfer.evaluate(f64::NAN).is_none());
        let cancel = crate::cancel::CancelToken::new();
        cancel.cancel();
        assert!(cancel.scope(|| transfer.lookup_table()).is_none());
        assert!(cancel
            .scope(|| PreparedTransfer::prepare(&source, &reader))
            .is_none());
    }
}

#[test]
fn transfer_rejects_extra_channels_arrays_and_malformed_functions() {
    let reader = reader_with_objects(&[]);
    for object in [
        calculator(b"{ dup }", 2),
        PdfObject::Array(vec![calculator(b"{ }", 1)]),
        PdfObject::Name("Default".into()),
        PdfObject::Null,
        calculator(b"{ unknown }", 1),
    ] {
        assert!(PreparedTransfer::prepare(&object, &reader).is_none());
    }
}

#[test]
fn transfer_reports_failure_at_any_lut_sample_not_just_midpoint() {
    // Midpoint succeeds, but sample zero must fail rather than publish a
    // partially initialized or identity-substituted LUT.
    let reader = reader_with_objects(&[]);
    let transfer = PreparedTransfer::prepare(&calculator(b"{ 1 exch div }", 1), &reader).unwrap();
    assert_eq!(transfer.evaluate(0.5), Some(1.0));
    assert!(transfer.lookup_table().is_none());
}

#[test]
fn transfer_lut_budget_is_shared_across_all_samples() {
    let reader = reader_with_objects(&[]);
    let transfer = PreparedTransfer::prepare(&calculator(b"{ dup mul }", 1), &reader).unwrap();
    let (_, cost) = transfer
        .function
        .as_ref()
        .unwrap()
        .evaluate_metered(&[0.5], MAX_FUNCTION_WORK);
    assert!(cost > 0);
    assert!(transfer.lookup_table_with_budget(cost * 255).is_none());
    assert!(transfer.lookup_table_with_budget(cost * 256).is_some());
    let identity = PreparedTransfer::prepare(&PdfObject::Name("Identity".into()), &reader).unwrap();
    assert!(identity.lookup_table_with_budget(255).is_none());
    assert!(identity.lookup_table_with_budget(256).is_some());
}
