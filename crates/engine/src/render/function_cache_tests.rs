//! Source regressions; do not execute before the authorized qualification phase.
use super::*;
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects, reference};

fn exponential(end: f64) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(2));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("C0", numbers(&[0.0]));
    dict.insert("C1", numbers(&[end]));
    dict.insert("N", PdfObject::Integer(1));
    PdfObject::Dictionary(dict)
}

#[test]
fn direct_and_indirect_repeated_lookups_reuse_the_same_graph() {
    let object = exponential(1.0);
    let reader = reader_with_objects(&[object.clone()]);
    for source in [object, reference(4)] {
        let first = PreparedFunction::cached_single(&source, 1, &reader).unwrap();
        let second = PreparedFunction::cached_single(&source.clone(), 1, &reader).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(second.evaluate(&[0.25]), vec![0.25]);
    }
    assert_eq!(reader.function_cache.lock().unwrap().entries.len(), 2);
}

#[test]
fn reader_boundaries_prevent_equal_object_ids_from_aliasing() {
    let first = reader_with_objects(&[exponential(1.0)]);
    let second = reader_with_objects(&[exponential(0.5)]);
    let a = PreparedFunction::cached_single(&reference(4), 1, &first).unwrap();
    let b = PreparedFunction::cached_single(&reference(4), 1, &second).unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    assert_eq!(a.evaluate(&[1.0]), vec![1.0]);
    assert_eq!(b.evaluate(&[1.0]), vec![0.5]);
    let equal_bytes = reader_with_objects(&[exponential(1.0)]);
    let c = PreparedFunction::cached_single(&reference(4), 1, &equal_bytes).unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
}

#[test]
fn exact_keys_preserve_types_bits_generations_and_consumer_shape() {
    for (a, b) in [
        (PdfObject::Integer(1), PdfObject::Real(1.0)),
        (PdfObject::Real(0.0), PdfObject::Real(-0.0)),
        (
            PdfObject::String(b"x".to_vec()),
            PdfObject::Name("x".into()),
        ),
        (
            PdfObject::Array(vec![]),
            PdfObject::Dictionary(PdfDictionary::empty()),
        ),
        (
            reference(4),
            PdfObject::Reference {
                number: 4,
                generation: 1,
            },
        ),
        (
            PdfObject::Array(vec![
                PdfObject::Name("ab".into()),
                PdfObject::Name("c".into()),
            ]),
            PdfObject::Array(vec![
                PdfObject::Name("a".into()),
                PdfObject::Name("bc".into()),
            ]),
        ),
    ] {
        assert_ne!(key(&a, 1, false).unwrap(), key(&b, 1, false).unwrap());
    }
    let object = exponential(1.0);
    assert_ne!(key(&object, 1, false), key(&object, 2, false));
    assert_ne!(key(&object, 1, false), key(&object, 1, true));
    let mut dictionary = PdfDictionary::empty();
    dictionary.insert("N", PdfObject::Integer(1));
    let stream = PdfObject::Stream {
        dict: dictionary.clone(),
        raw: b"x".to_vec(),
    };
    assert_ne!(
        key(&stream, 1, false),
        key(&PdfObject::Dictionary(dictionary), 1, false)
    );
}

#[test]
fn deep_source_keys_are_exact_or_bypassed_never_truncated() {
    let mut first = PdfObject::Integer(1);
    let mut second = PdfObject::Integer(2);
    for _ in 0..24 {
        first = PdfObject::Array(vec![first]);
        second = PdfObject::Array(vec![second]);
    }
    assert_ne!(
        key(&first, 1, false).unwrap(),
        key(&second, 1, false).unwrap()
    );
    for _ in 0..MAX_KEY_DEPTH {
        first = PdfObject::Array(vec![first]);
    }
    assert!(key(&first, 1, false).is_none());
    assert!(key(
        &PdfObject::Array(vec![PdfObject::Null; MAX_KEY_VISITS]),
        1,
        false
    )
    .is_none());
    assert!(key(&PdfObject::String(vec![0; MAX_KEY_BYTES]), 1, false).is_none());
}

#[test]
fn oversized_direct_key_bypasses_cache_without_rejecting_valid_function() {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(4));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("Range", numbers(&[0.0, 1.0]));
    let mut raw = b"{ %".to_vec();
    raw.extend(vec![b'x'; MAX_KEY_BYTES]);
    raw.extend_from_slice(b"\ndup mul }");
    let object = PdfObject::Stream { dict, raw };
    let reader = reader_with_objects(&[object.clone()]);
    let direct = PreparedFunction::cached_single(&object, 1, &reader).unwrap();
    assert_eq!(direct.evaluate(&[0.5]), vec![0.25]);
    assert!(reader.function_cache.lock().unwrap().entries.is_empty());
    let indirect = PreparedFunction::cached_single(&reference(4), 1, &reader).unwrap();
    assert_eq!(indirect.evaluate(&[0.5]), vec![0.25]);
    assert_eq!(reader.function_cache.lock().unwrap().entries.len(), 1);
}

#[test]
fn invalid_and_mismatched_functions_are_not_admitted_or_reused() {
    let reader = reader_with_objects(&[]);
    let object = exponential(1.0);
    assert!(PreparedFunction::cached_single(&object, 2, &reader).is_none());
    assert!(PreparedFunction::cached_single(&PdfObject::Null, 1, &reader).is_none());
    assert!(reader.function_cache.lock().unwrap().entries.is_empty());
    let array = PdfObject::Array(vec![object]);
    assert_eq!(
        PreparedFunction::cached(&array, 1, &reader)
            .unwrap()
            .evaluate(&[0.5]),
        vec![0.5]
    );
    assert!(PreparedFunction::cached_single(&array, 1, &reader).is_none());
    assert!(PreparedFunction::cached(&array, 0, &reader).is_none());
    assert_eq!(reader.function_cache.lock().unwrap().entries.len(), 1);
}

#[test]
fn lru_eviction_preserves_live_paint_handles() {
    let reader = reader_with_objects(&[]);
    *reader.function_cache.lock().unwrap() = FunctionCache::new(2, MAX_BYTES);
    let a = exponential(1.0);
    let b = exponential(2.0);
    let c = exponential(3.0);
    let first = PreparedFunction::cached_single(&a, 1, &reader).unwrap();
    let second = PreparedFunction::cached_single(&b, 1, &reader).unwrap();
    assert!(Arc::ptr_eq(
        &first,
        &PreparedFunction::cached_single(&a, 1, &reader).unwrap()
    ));
    PreparedFunction::cached_single(&c, 1, &reader).unwrap();
    let reloaded = PreparedFunction::cached_single(&b, 1, &reader).unwrap();
    assert!(!Arc::ptr_eq(&second, &reloaded));
    assert_eq!(second.evaluate(&[0.25]), vec![0.5]);
    assert_eq!(reader.function_cache.lock().unwrap().entries.len(), 2);
}

#[test]
fn byte_budget_charges_graph_and_key_and_bypasses_oversized_entries() {
    let reader = reader_with_objects(&[]);
    let object = exponential(1.0);
    let prepared = Arc::new(PreparedFunction::prepare_single(&object, 1, &reader).unwrap());
    let cache_key = key(&object, 1, false).unwrap();
    let bytes = prepared.retained_bytes() + cache_key.capacity() + std::mem::size_of::<Entry>();
    let mut cache = FunctionCache::new(2, bytes - 1);
    cache.insert(key(&object, 1, false).unwrap(), Arc::clone(&prepared));
    assert!(cache.entries.is_empty());
    let mut cache = FunctionCache::new(2, bytes);
    cache.insert(cache_key, Arc::clone(&prepared));
    assert_eq!(cache.bytes, bytes);
    assert_eq!(cache.entries.len(), 1);
    // Another same-sized key must evict despite the two-entry count budget.
    cache.insert(key(&exponential(2.0), 1, false).unwrap(), prepared);
    assert_eq!(cache.bytes, bytes);
    assert_eq!(cache.entries.len(), 1);
}

#[test]
fn duplicate_concurrent_admissions_converge_to_one_identity() {
    let reader = reader_with_objects(&[]);
    let object = exponential(1.0);
    let a = Arc::new(PreparedFunction::prepare_single(&object, 1, &reader).unwrap());
    let b = Arc::new(PreparedFunction::prepare_single(&object, 1, &reader).unwrap());
    let mut cache = FunctionCache::default();
    let first = cache.insert(key(&object, 1, false).unwrap(), a);
    let second = cache.insert(key(&object, 1, false).unwrap(), b);
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(cache.entries.len(), 1);
}

#[test]
fn parallel_readers_share_graphs_without_holding_locks_during_evaluation() {
    let reader = reader_with_objects(&[exponential(1.0)]);
    let handles = std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let graph = PreparedFunction::cached_single(&reference(4), 1, &reader).unwrap();
                    assert_eq!(graph.evaluate(&[0.5]), vec![0.5]);
                    graph
                })
            })
            .collect();
        tasks
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(handles
        .windows(2)
        .all(|pair| Arc::ptr_eq(&pair[0], &pair[1])));
}

#[test]
fn dropping_reader_releases_cache_but_not_external_graphs() {
    let (weak, retained) = {
        let reader = reader_with_objects(&[]);
        let a = PreparedFunction::cached_single(&exponential(1.0), 1, &reader).unwrap();
        let b = PreparedFunction::cached_single(&exponential(2.0), 1, &reader).unwrap();
        (Arc::downgrade(&a), b)
    };
    assert!(weak.upgrade().is_none());
    assert_eq!(retained.evaluate(&[0.25]), vec![0.5]);
}

#[test]
fn cancelled_warm_lookup_does_not_serve_or_poison_cache() {
    let reader = reader_with_objects(&[]);
    let object = exponential(1.0);
    let first = PreparedFunction::cached_single(&object, 1, &reader).unwrap();
    let cancel = crate::cancel::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| PreparedFunction::cached_single(&object, 1, &reader))
        .is_none());
    let again = PreparedFunction::cached_single(&object, 1, &reader).unwrap();
    assert!(Arc::ptr_eq(&first, &again));
}

#[test]
fn poisoned_optional_cache_falls_back_to_validated_graphs() {
    let reader = reader_with_objects(&[]);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = reader.function_cache.lock().unwrap();
        panic!("poison optional cache");
    }));
    let graph = PreparedFunction::cached_single(&exponential(1.0), 1, &reader).unwrap();
    assert_eq!(graph.evaluate(&[0.25]), vec![0.25]);
}

#[test]
fn explicit_cache_does_not_populate_or_change_standalone_policy() {
    let reader = reader_with_objects(&[]);
    reader.set_function_cache_byte_limit(0).unwrap();
    let cache = Mutex::new(FunctionCache::default());
    let resources = FunctionResources {
        cache: Some(&cache),
        ..Default::default()
    };
    let object = exponential(1.0);
    let first = prepare_with_resources(&object, 1, false, &reader, resources).unwrap();
    let second = prepare_with_resources(&object, 1, false, &reader, resources).unwrap();
    assert!(Arc::ptr_eq(first.graph(), second.graph()));
    assert_eq!(first.evaluate(&[0.25]), vec![0.25]);
    let metrics = cache.lock().unwrap().metrics();
    assert!(metrics.available);
    assert_eq!(
        (
            metrics.hits,
            metrics.misses,
            metrics.admissions,
            metrics.entries
        ),
        (1, 1, 1, 1)
    );
    assert!(metrics.bytes > first.retained_bytes());
    let standalone = reader.function_cache_metrics();
    assert_eq!(
        (
            standalone.max_bytes,
            standalone.entries,
            standalone.hits,
            standalone.misses
        ),
        (0, 0, 0, 0)
    );
}

#[test]
fn external_cache_rebinds_object_ids_on_both_lookup_and_admission() {
    let first = reader_with_objects(&[exponential(1.0)]);
    let second = reader_with_objects(&[exponential(0.5)]);
    let cache = Mutex::new(FunctionCache::default());
    let resources = FunctionResources {
        cache: Some(&cache),
        ..Default::default()
    };
    let a = prepare_with_resources(&reference(4), 1, false, &first, resources).unwrap();
    let b = prepare_with_resources(&reference(4), 1, false, &second, resources).unwrap();
    let c = prepare_with_resources(&reference(4), 1, false, &first, resources).unwrap();
    assert_eq!(a.evaluate(&[1.0]), vec![1.0]);
    assert_eq!(b.evaluate(&[1.0]), vec![0.5]);
    assert_eq!(c.evaluate(&[1.0]), vec![1.0]);
    assert!(!Arc::ptr_eq(a.graph(), c.graph()));
    let metrics = cache.lock().unwrap().metrics();
    assert_eq!(
        (metrics.entries, metrics.reader_rebinds, metrics.evictions),
        (1, 2, 2)
    );

    // Model the unlocked-prepare interleaving: another reader has populated
    // the cache after our miss but before our admission lock is reacquired.
    let candidate = Arc::new(PreparedFunction::prepare_single(&reference(4), 1, &second).unwrap());
    let mut locked = cache.lock().unwrap();
    locked.bind_reader(&second);
    let admitted = locked.insert(key(&reference(4), 1, false).unwrap(), candidate);
    assert_eq!(admitted.evaluate(&[1.0]), vec![0.5]);
    assert_eq!(locked.metrics().reader_rebinds, 3);
}

#[test]
fn reader_namespace_is_weak_and_cannot_keep_the_reader_cache_alive() {
    let cache = Mutex::new(FunctionCache::default());
    let identity = {
        let reader = reader_with_objects(&[exponential(1.0)]);
        prepare_with_resources(
            &reference(4),
            1,
            false,
            &reader,
            FunctionResources {
                cache: Some(&cache),
                ..Default::default()
            },
        )
        .unwrap();
        Arc::downgrade(&reader.function_cache)
    };
    assert!(identity.upgrade().is_none());
    let reader = reader_with_objects(&[exponential(0.5)]);
    let fresh = prepare_with_resources(
        &reference(4),
        1,
        false,
        &reader,
        FunctionResources {
            cache: Some(&cache),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(fresh.evaluate(&[1.0]), vec![0.5]);
    assert_eq!(cache.lock().unwrap().metrics().reader_rebinds, 1);
}

#[test]
fn changing_cache_limit_evicts_handles_without_disabling_evaluation() {
    let reader = reader_with_objects(&[]);
    let object = exponential(1.0);
    let held = PreparedFunction::cached_single(&object, 1, &reader).unwrap();
    reader.set_function_cache_byte_limit(0).unwrap();
    assert_eq!(reader.function_cache_metrics().entries, 0);
    assert_eq!(reader.function_cache_metrics().evictions, 1);
    let uncached = PreparedFunction::cached_single(&object, 1, &reader).unwrap();
    assert!(!Arc::ptr_eq(&held, &uncached));
    assert_eq!(uncached.evaluate(&[0.5]), held.evaluate(&[0.5]));
    assert_eq!(reader.function_cache_metrics().entries, 0);
    reader.set_function_cache_byte_limit(usize::MAX).unwrap();
    assert_eq!(reader.function_cache_metrics().max_bytes, MAX_BYTES);
    let fresh = PreparedFunction::cached_single(&object, 1, &reader).unwrap();
    let hit = PreparedFunction::cached_single(&object, 1, &reader).unwrap();
    assert!(Arc::ptr_eq(&fresh, &hit));
}

#[test]
fn independent_worker_limits_do_not_cross_talk_on_one_reader() {
    let reader = reader_with_objects(&[exponential(1.0)]);
    let cold = Mutex::new(FunctionCache::new(MAX_ENTRIES, 0));
    let warm = Mutex::new(FunctionCache::default());
    let before = reader.function_cache_metrics();
    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            for _ in 0..16 {
                let graph = prepare_with_resources(
                    &reference(4),
                    1,
                    false,
                    &reader,
                    FunctionResources {
                        cache: Some(&cold),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(graph.evaluate(&[0.5]), vec![0.5]);
            }
        });
        let b = scope.spawn(|| {
            for _ in 0..16 {
                let graph = prepare_with_resources(
                    &reference(4),
                    1,
                    false,
                    &reader,
                    FunctionResources {
                        cache: Some(&warm),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(graph.evaluate(&[0.5]), vec![0.5]);
            }
        });
        a.join().unwrap();
        b.join().unwrap();
    });
    let cold = cold.lock().unwrap().metrics();
    let warm = warm.lock().unwrap().metrics();
    assert_eq!(
        (cold.entries, cold.admissions, cold.skipped_oversized),
        (0, 0, 16)
    );
    assert_eq!((warm.entries, warm.admissions, warm.hits), (1, 1, 15));
    assert_eq!(reader.function_cache_metrics(), before);
}

#[test]
fn one_external_cache_can_be_shared_by_competing_reader_namespaces() {
    let first = reader_with_objects(&[exponential(1.0)]);
    let second = reader_with_objects(&[exponential(0.5)]);
    let cache = Mutex::new(FunctionCache::default());
    std::thread::scope(|scope| {
        let tasks: Vec<_> = [&first, &second]
            .into_iter()
            .enumerate()
            .map(|(index, reader)| {
                let cache = &cache;
                scope.spawn(move || {
                    for _ in 0..16 {
                        let graph = prepare_with_resources(
                            &reference(4),
                            1,
                            false,
                            reader,
                            FunctionResources {
                                cache: Some(cache),
                                ..Default::default()
                            },
                        )
                        .unwrap();
                        assert_eq!(
                            graph.evaluate(&[1.0]),
                            vec![if index == 0 { 1.0 } else { 0.5 }]
                        );
                    }
                })
            })
            .collect();
        for task in tasks {
            task.join().unwrap();
        }
    });
    let metrics = cache.lock().unwrap().metrics();
    assert_eq!(metrics.entries, 1);
    assert!(metrics.reader_rebinds >= 1);
    assert_eq!(first.function_cache_metrics().entries, 0);
    assert_eq!(second.function_cache_metrics().entries, 0);
}

#[test]
fn poisoned_override_does_not_fall_back_to_unbudgeted_standalone_retention() {
    let reader = reader_with_objects(&[]);
    let cache = Mutex::new(FunctionCache::default());
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = cache.lock().unwrap();
        panic!("poison render-owned cache");
    }));
    let graph = prepare_with_resources(
        &exponential(1.0),
        1,
        false,
        &reader,
        FunctionResources {
            cache: Some(&cache),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(graph.evaluate(&[0.25]), vec![0.25]);
    assert_eq!(reader.function_cache_metrics().entries, 0);
}
