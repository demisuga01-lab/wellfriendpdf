//! Source-only regressions: execution is deferred to authorized qualification.
use super::*;
use crate::object::{PdfDictionary, PdfObject};
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects};

fn sampled() -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(0));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("Range", numbers(&[0.0, 1.0]));
    dict.insert("Size", PdfObject::Array(vec![PdfObject::Integer(4)]));
    dict.insert("BitsPerSample", PdfObject::Integer(8));
    PdfObject::Stream {
        dict,
        raw: vec![0, 0, 255, 255],
    }
}

fn resources(budget: &Arc<DecodeMemoryBudget>) -> FunctionResources<'_> {
    FunctionResources {
        memory: Some(budget),
        ..FunctionResources::default()
    }
}

fn fully_released(budget: &Arc<DecodeMemoryBudget>, limit: u64) {
    drop(
        budget
            .try_acquire(limit)
            .expect("all graph/window reservations must be released"),
    );
}

#[test]
fn cold_and_warm_consumers_hold_exact_retained_charge_until_drop() {
    let reader = reader_with_objects(&[]);
    let budget = Arc::new(DecodeMemoryBudget::new(4096));
    for _ in 0..2 {
        let lease = PreparedFunction::cached_with_resources(
            &sampled(),
            1,
            false,
            &reader,
            resources(&budget),
        )
        .unwrap();
        assert_eq!(
            lease.memory.as_ref().unwrap().bytes(),
            lease.retained_bytes() as u64
        );
        assert_eq!(lease.evaluate(&[0.5]), vec![0.5]);
        let spare = 4096 - lease.retained_bytes() as u64;
        let other = budget.try_acquire(spare).unwrap();
        assert!(budget.try_acquire(1).is_err());
        drop(other);
        drop(lease);
        fully_released(&budget, 4096);
    }
    assert!(budget.metrics().peak_reserved_bytes <= 4096);
}

#[test]
fn warm_graph_cannot_bypass_smaller_graph_or_stream_policy() {
    let reader = reader_with_objects(&[]);
    let graph = PreparedFunction::cached_single(&sampled(), 1, &reader).unwrap();
    for policy in [
        FunctionResources {
            max_graph_bytes: graph.retained_bytes() - 1,
            ..FunctionResources::default()
        },
        FunctionResources {
            max_stream_bytes: 3,
            ..FunctionResources::default()
        },
        FunctionResources {
            max_graph_bytes: 0,
            ..FunctionResources::default()
        },
    ] {
        assert!(
            PreparedFunction::cached_with_resources(&sampled(), 1, false, &reader, policy)
                .is_none()
        );
    }
    assert!(PreparedFunction::cached_with_resources(
        &sampled(),
        1,
        false,
        &reader,
        FunctionResources {
            max_graph_bytes: graph.retained_bytes(),
            max_stream_bytes: 4,
            memory: None,
            cache: None,
        }
    )
    .is_some());
}

#[test]
fn cached_graph_consumers_share_aggregate_budget_without_waiting() {
    let reader = reader_with_objects(&[]);
    let graph = PreparedFunction::cached_single(&sampled(), 1, &reader).unwrap();
    let cost = graph.retained_bytes() as u64;
    let budget = Arc::new(DecodeMemoryBudget::new(cost));
    let first =
        PreparedFunction::cached_with_resources(&sampled(), 1, false, &reader, resources(&budget))
            .unwrap();
    assert!(PreparedFunction::cached_with_resources(
        &sampled(),
        1,
        false,
        &reader,
        resources(&budget)
    )
    .is_none());
    drop(first);
    let second =
        PreparedFunction::cached_with_resources(&sampled(), 1, false, &reader, resources(&budget))
            .unwrap();
    assert!(Arc::ptr_eq(&graph, second.graph()));
    drop(second);
    fully_released(&budget, cost);
}

#[test]
fn cache_eviction_and_reader_drop_do_not_release_active_use_lease() {
    let reader = reader_with_objects(&[]);
    let budget = Arc::new(DecodeMemoryBudget::new(4096));
    let lease =
        PreparedFunction::cached_with_resources(&sampled(), 1, false, &reader, resources(&budget))
            .unwrap();
    *reader.function_cache.lock().unwrap() = crate::render::function::FunctionCache::default();
    drop(reader);
    assert!(budget.try_acquire(4096).is_err());
    assert_eq!(lease.evaluate(&[0.5]), vec![0.5]);
    drop(lease);
    fully_released(&budget, 4096);
}

#[test]
fn separate_render_budgets_account_for_same_shared_graph_independently() {
    let reader = reader_with_objects(&[]);
    let first = Arc::new(DecodeMemoryBudget::new(4096));
    let second = Arc::new(DecodeMemoryBudget::new(4096));
    let a =
        PreparedFunction::cached_with_resources(&sampled(), 1, false, &reader, resources(&first))
            .unwrap();
    let b =
        PreparedFunction::cached_with_resources(&sampled(), 1, false, &reader, resources(&second))
            .unwrap();
    assert!(Arc::ptr_eq(a.graph(), b.graph()));
    assert!(first.try_acquire(4096).is_err());
    assert!(second.try_acquire(4096).is_err());
    drop(a);
    fully_released(&first, 4096);
    assert!(second.try_acquire(4096).is_err());
    drop(b);
    fully_released(&second, 4096);
}

#[test]
fn failed_cold_preparations_release_partial_graph_and_decoder_windows() {
    for (function, policy_limit) in [(sampled(), 1), (sampled(), 512)] {
        let reader = reader_with_objects(&[]);
        let budget = Arc::new(DecodeMemoryBudget::new(4096));
        let mut object = function;
        if policy_limit == 512 {
            if let PdfObject::Stream { raw, .. } = &mut object {
                raw.clear();
            }
        }
        let policy = FunctionResources {
            max_graph_bytes: policy_limit,
            ..resources(&budget)
        };
        assert!(
            PreparedFunction::cached_with_resources(&object, 1, false, &reader, policy).is_none()
        );
        fully_released(&budget, 4096);
    }
}

#[test]
fn calculator_program_window_is_released_but_retained_code_stays_charged() {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(4));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("Range", numbers(&[0.0, 1.0]));
    let mut raw = b"{ %".to_vec();
    raw.extend(vec![b'x'; 4096]);
    raw.extend_from_slice(b"\ndup mul }");
    let raw_len = raw.len();
    let object = PdfObject::Stream { dict, raw };
    let reader = reader_with_objects(&[]);
    let budget = Arc::new(DecodeMemoryBudget::new(8192));
    let lease =
        PreparedFunction::cached_with_resources(&object, 1, false, &reader, resources(&budget))
            .unwrap();
    assert_eq!(
        lease.memory.as_ref().unwrap().bytes(),
        lease.retained_bytes() as u64
    );
    assert!(lease.retained_bytes() < raw_len);
    assert_eq!(lease.evaluate(&[0.5]), vec![0.25]);
    assert!(PreparedFunction::cached_with_resources(
        &object,
        1,
        false,
        &reader,
        FunctionResources {
            max_stream_bytes: raw_len - 1,
            ..resources(&budget)
        }
    )
    .is_none());
    drop(lease);
    fully_released(&budget, 8192);
}

#[test]
fn cancellation_does_not_leak_cold_or_warm_reservations() {
    let reader = reader_with_objects(&[]);
    let budget = Arc::new(DecodeMemoryBudget::new(4096));
    let cancel = crate::cancel::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| PreparedFunction::cached_with_resources(
            &sampled(),
            1,
            false,
            &reader,
            resources(&budget)
        ))
        .is_none());
    fully_released(&budget, 4096);
    PreparedFunction::cached_single(&sampled(), 1, &reader).unwrap();
    assert!(cancel
        .scope(|| PreparedFunction::cached_with_resources(
            &sampled(),
            1,
            false,
            &reader,
            resources(&budget)
        ))
        .is_none());
    fully_released(&budget, 4096);
}

#[test]
fn nested_indexed_and_icc_alternates_carry_function_limits() {
    use crate::render::{
        cmm,
        colorspace::{resolve_named_color_with_resources, NamedColor},
    };
    let separation = PdfObject::Array(vec![
        PdfObject::Name("Separation".into()),
        PdfObject::Name("Ink".into()),
        PdfObject::Name("DeviceGray".into()),
        sampled(),
    ]);
    let indexed = PdfObject::Array(vec![
        PdfObject::Name("Indexed".into()),
        separation.clone(),
        PdfObject::Integer(0),
        PdfObject::String(vec![128]),
    ]);
    let mut profile = PdfDictionary::empty();
    profile.insert("N", PdfObject::Integer(1));
    profile.insert("Alternate", separation.clone());
    let icc = PdfObject::Array(vec![
        PdfObject::Name("ICCBased".into()),
        PdfObject::Stream {
            dict: profile,
            raw: Vec::new(),
        },
    ]);
    let reader = reader_with_objects(&[]);
    let budget = Arc::new(DecodeMemoryBudget::new(4096));
    let options = cmm::ColorTransformOptions {
        backend: cmm::ColorTransformBackend::DeterministicFallback,
        ..Default::default()
    };
    for (space, value) in [(separation, 0.5), (indexed, 0.0), (icc, 0.5)] {
        assert!(matches!(
            resolve_named_color_with_resources(
                &space,
                None,
                &[value],
                1.0,
                &reader,
                options,
                resources(&budget)
            ),
            NamedColor::Color(_)
        ));
        // The successful lookup warmed both caches. Neither may bypass the new limit.
        assert!(matches!(
            resolve_named_color_with_resources(
                &space,
                None,
                &[value],
                1.0,
                &reader,
                options,
                FunctionResources {
                    max_graph_bytes: 0,
                    ..resources(&budget)
                }
            ),
            NamedColor::Invalid(_)
        ));
        fully_released(&budget, 4096);
    }
}

#[test]
fn scalar_transfer_holds_graph_reservation_through_complete_lut() {
    let reader = reader_with_objects(&[]);
    let budget = Arc::new(DecodeMemoryBudget::new(4096));
    let transfer = crate::render::function::PreparedTransfer::prepare_with_resources(
        &sampled(),
        &reader,
        resources(&budget),
    )
    .unwrap();
    assert!(budget.try_acquire(4096).is_err());
    let table = transfer.lookup_table().unwrap();
    assert_eq!(table[0], 0);
    assert_eq!(table[255], 255);
    assert!(budget.try_acquire(4096).is_err());
    drop(transfer);
    fully_released(&budget, 4096);
}
