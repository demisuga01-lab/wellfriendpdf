//! Unexecuted source regressions for explicit image function policy propagation.
use super::*;
use crate::decode_scheduler::DecodeMemoryBudget;
use crate::render::function::{FunctionCache, FunctionResources, PreparedFunction};
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects, reference};
use std::sync::{Arc, Mutex};

fn name(value: &str) -> PdfObject {
    PdfObject::Name(value.into())
}
fn sampled() -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(0));
    dict.insert("Domain", numbers(&[0.0, 1.0]));
    dict.insert("Range", numbers(&[0.0, 1.0]));
    dict.insert("Size", PdfObject::Array(vec![PdfObject::Integer(2)]));
    dict.insert("BitsPerSample", PdfObject::Integer(8));
    PdfObject::Stream {
        dict,
        raw: vec![0, 255],
    }
}
fn separation() -> PdfObject {
    PdfObject::Array(vec![
        name("Separation"),
        name("Ink"),
        name("DeviceGray"),
        reference(4),
    ])
}
fn indexed(base: PdfObject) -> PdfObject {
    PdfObject::Array(vec![
        name("Indexed"),
        base,
        PdfObject::Integer(0),
        PdfObject::String(vec![128]),
    ])
}
fn profile() -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("N", PdfObject::Integer(1));
    dict.insert("Alternate", separation());
    PdfObject::Stream { dict, raw: vec![] }
}
fn spaces() -> Vec<(&'static str, PdfObject, u8)> {
    let icc = PdfObject::Array(vec![name("ICCBased"), reference(5)]);
    vec![
        ("Separation", separation(), 128),
        (
            "DeviceN",
            PdfObject::Array(vec![
                name("DeviceN"),
                PdfObject::Array(vec![name("Ink")]),
                name("DeviceGray"),
                reference(4),
            ]),
            128,
        ),
        ("Indexed", indexed(separation()), 0),
        ("ICCBased", icc.clone(), 128),
        ("Indexed", indexed(icc), 0),
    ]
}
fn options<'a>(
    cache: &'a Mutex<FunctionCache>,
    budget: &'a Arc<DecodeMemoryBudget>,
) -> ImageColorOptions<'a> {
    ImageColorOptions {
        options: ColorTransformOptions {
            backend: cmm::ColorTransformBackend::DeterministicFallback,
            ..Default::default()
        },
        functions: FunctionResources {
            cache: Some(cache),
            memory: Some(budget),
            ..Default::default()
        },
    }
}
fn dictionary(space: PdfObject) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert("ColorSpace", space);
    dict
}
fn inline(
    reader: &PdfReader,
    family: &str,
    dict: &PdfDictionary,
    sample: u8,
    options: ImageColorOptions<'_>,
) -> Result<RawImage> {
    ImageDecoder::decode_inline_with_resolved_image_dictionary_and_param_array(
        &[sample],
        1,
        1,
        8,
        family,
        None,
        &[],
        &[],
        dict,
        &DecodeLimits::default(),
        Some(reader),
        options,
    )
}

#[test]
fn inline_tints_indexed_bases_and_icc_alternates_use_explicit_cache() {
    let reader = reader_with_objects(&[sampled(), profile()]);
    for (family, space, sample) in spaces() {
        let cache = Mutex::new(FunctionCache::default());
        let budget = Arc::new(DecodeMemoryBudget::new(4096));
        let policy = options(&cache, &budget);
        let dict = dictionary(space);
        for _ in 0..2 {
            let image = inline(&reader, family, &dict, sample, policy).unwrap();
            assert_eq!(image.pixels, vec![128, 128, 128, 255], "{family}");
            assert_eq!(image.channels, 4);
        }
        let metrics = cache.lock().unwrap().metrics();
        assert_eq!(metrics.entries, 1, "{family}");
        assert!(metrics.hits >= 1);
        assert_eq!(reader.function_cache_metrics().entries, 0);
        drop(budget.try_acquire(4096).unwrap());
    }
}

#[test]
fn warm_image_paths_do_not_bypass_stricter_graph_or_stream_policy() {
    let reader = reader_with_objects(&[sampled(), profile()]);
    for (family, space, sample) in spaces() {
        for warm in [false, true] {
            for decoded in [false, true] {
                let cache = Mutex::new(FunctionCache::default());
                let budget = Arc::new(DecodeMemoryBudget::new(4096));
                let mut policy = options(&cache, &budget);
                let dict = dictionary(space.clone());
                if warm {
                    inline(&reader, family, &dict, sample, policy).unwrap();
                }
                if decoded {
                    policy.functions.max_stream_bytes = 1;
                } else {
                    policy.functions.max_graph_bytes = 0;
                }
                assert!(
                    inline(&reader, family, &dict, sample, policy).is_err(),
                    "{family} warm={warm} decoded={decoded}"
                );
                drop(budget.try_acquire(4096).unwrap());
                assert_eq!(reader.function_cache_metrics().entries, 0);
            }
        }
    }
}

#[test]
fn byte_converter_and_packed_samples_keep_the_same_policy() {
    let reader = reader_with_objects(&[sampled(), profile()]);
    let cache = Mutex::new(FunctionCache::default());
    let budget = Arc::new(DecodeMemoryBudget::new(4096));
    let mut policy = options(&cache, &budget);
    for (family, space, sample) in spaces() {
        let dict = dictionary(space);
        let converted = ColorSpaceConverter::convert_with_options(
            vec![sample],
            1,
            1,
            family,
            &dict,
            &reader,
            policy,
        )
        .unwrap();
        assert_eq!(converted, (vec![128, 128, 128, 255], 4));
        policy.functions.max_graph_bytes = 0;
        assert!(ColorSpaceConverter::convert_with_options(
            vec![sample],
            1,
            1,
            family,
            &dict,
            &reader,
            policy
        )
        .is_err());
        policy.functions.max_graph_bytes = FunctionResources::default().max_graph_bytes;
    }
    assert_eq!(reader.function_cache_metrics().entries, 0);
}

#[test]
fn image_conversion_cannot_borrow_live_sibling_function_memory() {
    let reader = reader_with_objects(&[sampled(), profile()]);
    let cost = PreparedFunction::prepare_single(&reference(4), 1, &reader)
        .unwrap()
        .retained_bytes();
    for (family, space, sample) in spaces() {
        let cache = Mutex::new(FunctionCache::default());
        let budget = Arc::new(DecodeMemoryBudget::new(4096));
        let policy = options(&cache, &budget);
        let dict = dictionary(space);
        inline(&reader, family, &dict, sample, policy).unwrap();
        let sibling = budget.try_acquire(4096 - cost as u64 + 1).unwrap();
        assert!(
            inline(&reader, family, &dict, sample, policy).is_err(),
            "{family}"
        );
        drop(sibling);
        inline(&reader, family, &dict, sample, policy).unwrap();
        drop(budget.try_acquire(4096).unwrap());
    }
}

#[test]
fn zero_retention_still_converts_images_and_standalone_options_still_work() {
    let reader = reader_with_objects(&[sampled(), profile()]);
    let cache = Mutex::new(FunctionCache::default());
    cache.lock().unwrap().set_byte_limit(0);
    let budget = Arc::new(DecodeMemoryBudget::new(4096));
    let policy = options(&cache, &budget);
    let dict = dictionary(separation());
    inline(&reader, "Separation", &dict, 128, policy).unwrap();
    assert_eq!(cache.lock().unwrap().metrics().entries, 0);
    assert_eq!(reader.function_cache_metrics().entries, 0);
    let (pixels, channels) = ColorSpaceConverter::convert_with_options(
        vec![128],
        1,
        1,
        "Separation",
        &dict,
        &reader,
        policy.options,
    )
    .unwrap();
    assert_eq!((pixels, channels), (vec![128, 128, 128, 255], 4));
    assert_eq!(reader.function_cache_metrics().entries, 1);
}
