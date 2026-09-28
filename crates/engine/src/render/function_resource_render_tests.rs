//! Unexecuted integration source for function/surface/tint memory overlap.
use super::*;
use crate::decode_scheduler::DecodeMemoryBudget;
use crate::render::function::PreparedFunction;
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects};
use std::sync::Arc;

fn function() -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(0));
    dict.insert("Domain", numbers(&[0.0, 1.0, 0.0, 1.0]));
    dict.insert("Range", numbers(&[0.0, 1.0]));
    dict.insert("Size", PdfObject::Array(vec![PdfObject::Integer(2); 2]));
    dict.insert("BitsPerSample", PdfObject::Integer(8));
    PdfObject::Stream {
        dict,
        raw: vec![128; 4],
    }
}

fn shading() -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert("ShadingType", PdfObject::Integer(1));
    dict.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
    dict.insert("Matrix", numbers(&[10.0, 0.0, 0.0, 10.0, 0.0, 0.0]));
    dict.insert("Function", function());
    dict
}

fn paint(
    reader: &PdfReader,
    dict: &PdfDictionary,
    budget: &Arc<DecodeMemoryBudget>,
    limit: usize,
) -> (Result<(), String>, PixelBuffer) {
    let viewport = Viewport::new([0.0, 0.0, 10.0, 10.0], 72);
    let mut target = PixelBuffer::new_filled_with_mode(
        10,
        10,
        [255; 4],
        crate::render::buffer::RenderMode::Compat,
    );
    let result = ShadingRenderer::paint_with_options_cancellable(
        dict,
        &Transform2D::identity(),
        &viewport,
        &mut target,
        reader,
        None,
        ShadingRenderOptions::default()
            .with_working_byte_limit(limit)
            .with_memory_budget(budget),
        &CancelToken::none(),
        &AtomicU64::new(MAX_SHADING_WORK_UNITS),
    );
    (result, target)
}

fn unchanged(target: &PixelBuffer) {
    for y in 0..10 {
        for x in 0..10 {
            assert_eq!(target.get_pixel(x, y), [255; 4]);
        }
    }
}

#[test]
fn function_storage_and_shading_scratch_share_one_allowance_cold_and_warm() {
    // Type 1 has a 10x10 RGBA scratch and fixed 4096-byte parser reserve.
    let base = 400 + 4096;
    for warm in [false, true] {
        let reader = reader_with_objects(&[]);
        let cost = PreparedFunction::prepare(&function(), 2, &reader)
            .unwrap()
            .retained_bytes();
        if warm {
            PreparedFunction::cached(&function(), 2, &reader).unwrap();
        }
        let limit = base + cost - 1;
        let budget = Arc::new(DecodeMemoryBudget::new(limit as u64));
        let (result, target) = paint(&reader, &shading(), &budget, limit);
        assert!(result.is_err(), "warm={warm}");
        unchanged(&target);
        drop(budget.try_acquire(limit as u64).unwrap());
        assert!(budget.metrics().peak_reserved_bytes <= limit as u64);
    }
}

#[test]
fn exact_function_surface_budget_succeeds_but_live_sibling_reduces_capacity() {
    let reader = reader_with_objects(&[]);
    let graph = PreparedFunction::cached(&function(), 2, &reader).unwrap();
    let limit = 400 + 4096 + graph.retained_bytes();
    let budget = Arc::new(DecodeMemoryBudget::new(limit as u64));
    let (result, target) = paint(&reader, &shading(), &budget, limit);
    result.unwrap();
    assert_eq!(target.get_pixel(5, 5), [128, 128, 128, 255]);
    let sibling = budget.try_acquire(1).unwrap();
    let (result, target) = paint(&reader, &shading(), &budget, limit);
    assert!(result.is_err());
    unchanged(&target);
    drop(sibling);
    drop(budget.try_acquire(limit as u64).unwrap());
}

#[test]
fn warm_tint_result_cannot_bypass_overlap_with_active_shading_function() {
    let reader = reader_with_objects(&[]);
    let mut tint = PdfDictionary::empty();
    tint.insert("FunctionType", PdfObject::Integer(2));
    tint.insert("Domain", numbers(&[0.0, 1.0]));
    tint.insert("C0", numbers(&[0.0]));
    tint.insert("C1", numbers(&[1.0]));
    tint.insert("N", PdfObject::Integer(1));
    let tint = PdfObject::Dictionary(tint);
    let main_cost = PreparedFunction::cached(&function(), 2, &reader)
        .unwrap()
        .retained_bytes();
    let tint_cost = PreparedFunction::cached_single(&tint, 1, &reader)
        .unwrap()
        .retained_bytes();
    let mut dict = shading();
    dict.insert(
        "ColorSpace",
        PdfObject::Array(vec![
            PdfObject::Name("Separation".into()),
            PdfObject::Name("Ink".into()),
            PdfObject::Name("DeviceGray".into()),
            tint,
        ]),
    );
    let limit = 400 + 4096 + main_cost + tint_cost - 1;
    let budget = Arc::new(DecodeMemoryBudget::new(limit as u64));
    let (result, target) = paint(&reader, &dict, &budget, limit);
    assert!(result.is_err());
    unchanged(&target);
    drop(budget.try_acquire(limit as u64).unwrap());
    let budget = Arc::new(DecodeMemoryBudget::new((limit + 1) as u64));
    let (result, target) = paint(&reader, &dict, &budget, limit + 1);
    result.unwrap();
    assert_eq!(target.get_pixel(5, 5), [128, 128, 128, 255]);
}
