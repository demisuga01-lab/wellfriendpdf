//! Unexecuted vector-classifier/loader regressions.
use super::*;
use crate::render::parameter_dictionary::tests::{numbers, reader_with_objects, reference};

fn function() -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    for (key, number) in [
        ("FunctionType", 4),
        ("Domain", 6),
        ("C0", 7),
        ("C1", 8),
        ("N", 9),
    ] {
        dict.insert(key, reference(number));
    }
    dict
}
fn objects() -> Vec<PdfObject> {
    vec![
        PdfObject::Integer(2),
        numbers(&[0.0, 0.0, 10.0, 0.0]),
        numbers(&[0.0, 1.0]),
        numbers(&[0.0]),
        numbers(&[1.0]),
        PdfObject::Integer(1),
        numbers(&[1.0, 0.0, 0.0, 1.0, 2.0, 3.0]),
    ]
}
fn shading() -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("ShadingType", reference(4));
    dict.insert("Coords", reference(5));
    dict.insert("Domain", reference(6));
    dict.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
    dict.insert("Function", PdfObject::Dictionary(function()));
    PdfObject::Dictionary(dict)
}

#[test]
fn svg_and_postscript_shading_loaders_accept_the_same_indirect_parameters() {
    let reader = reader_with_objects(&objects());
    let mut resources = PageResources::default();
    resources.shadings.insert("S".into(), shading());
    for target in [VectorOutputTarget::Svg, VectorOutputTarget::PostScript] {
        let Some(VectorShading::Axial(shading)) =
            load_vector_shading_for_target(&resources, Some(&reader), "S", target)
        else {
            panic!("indirect axial parameters must load")
        };
        assert_eq!(shading.coords, [0.0, 0.0, 10.0, 0.0]);
        assert_eq!(shading.domain, [0.0, 1.0]);
        assert!(shading.stops.len() >= 2);
    }
}

#[test]
fn pattern_matrix_parameters_and_function_arrays_keep_reference_identity() {
    let mut data = objects();
    data.push(PdfObject::Array(vec![PdfObject::Dictionary(function())]));
    data.push(reference(11));
    let reader = reader_with_objects(&data);
    let parsed = parse_vector_shading_functions(&reference(12), Some(&reader), true).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].sample(0.5).unwrap(), vec![0.5]);
    let mut pattern = PdfDictionary::empty();
    pattern.insert("PatternType", reference(4));
    pattern.insert("Matrix", reference(10));
    pattern.insert("Shading", shading());
    let mut resources = PageResources::default();
    resources
        .patterns
        .insert("P".into(), PdfObject::Dictionary(pattern));
    for target in [VectorOutputTarget::Svg, VectorOutputTarget::PostScript] {
        let pattern =
            load_vector_shading_pattern_for_target(&resources, Some(&reader), "P", target).unwrap();
        assert_eq!(pattern.matrix, [1.0, 0.0, 0.0, 1.0, 2.0, 3.0]);
    }
}

#[test]
fn vector_and_native_exponential_samples_agree_for_nonunit_domains() {
    let mut dict = PdfDictionary::empty();
    dict.insert("FunctionType", PdfObject::Integer(2));
    dict.insert("Domain", numbers(&[2.0, 4.0]));
    dict.insert("C0", numbers(&[-10.0]));
    dict.insert("C1", numbers(&[-9.0]));
    dict.insert("N", PdfObject::Integer(1));
    let reader = reader_with_objects(&[]);
    let function = parse_vector_shading_function(&dict, Some(&reader), true).unwrap();
    for input in [1.0, 2.0, 3.0, 4.0, 5.0] {
        let native = crate::render::function::eval_function_n(
            &PdfObject::Dictionary(dict.clone()),
            &[input],
            &reader,
        );
        assert_eq!(function.sample(input).unwrap(), native);
    }
}
