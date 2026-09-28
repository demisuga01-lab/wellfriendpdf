//! Unexecuted regression source and a small shared PDF fixture builder.
use super::*;

pub(crate) fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}
pub(crate) fn numbers(values: &[f64]) -> PdfObject {
    PdfObject::Array(values.iter().copied().map(PdfObject::Real).collect())
}
pub(crate) fn bytes_with_objects(extra: &[PdfObject]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>".to_vec(),
    ];
    for object in extra {
        let mut bytes = Vec::new();
        crate::writer::serialize_object(object, &mut bytes);
        objects.push(bytes);
    }
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in &offsets[1..] {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    pdf
}
pub(crate) fn reader_with_objects(extra: &[PdfObject]) -> PdfReader {
    PdfReader::from_bytes(bytes_with_objects(extra)).unwrap()
}

#[test]
fn direct_parameters_borrow_and_preserve_opaque_graphs() {
    let mut dict = PdfDictionary::empty();
    dict.insert("ShadingType", PdfObject::Integer(2));
    dict.insert("Coords", numbers(&[0.0, 0.0, 10.0, 0.0]));
    dict.insert("PrivateGraph", reference(999));
    let resolved = shading(&dict, None).unwrap();
    assert!(matches!(&resolved, Cow::Borrowed(_)));
    assert_eq!(resolved.get("PrivateGraph"), dict.get("PrivateGraph"));
}

#[test]
fn indirect_scalars_arrays_elements_and_alias_chains_resolve_without_mutation() {
    let reader = reader_with_objects(&[
        reference(5),
        PdfObject::Integer(2),
        PdfObject::Integer(0),
        PdfObject::Integer(10),
        PdfObject::Array(vec![reference(6), reference(6), reference(7), reference(6)]),
        PdfObject::Boolean(true),
        PdfObject::Array(vec![reference(9), PdfObject::Boolean(false)]),
    ]);
    let mut dict = PdfDictionary::empty();
    dict.insert("ShadingType", reference(4));
    dict.insert("Coords", reference(8));
    dict.insert("Extend", reference(10));
    let resolved = shading(&dict, Some(&reader)).unwrap();
    assert_eq!(resolved.get_integer("ShadingType"), Some(2));
    assert_eq!(
        resolved.get("Coords").unwrap().as_array().unwrap()[2].as_number(),
        Some(10.0)
    );
    assert_eq!(
        resolved.get("Extend").unwrap().as_array().unwrap()[0].as_bool(),
        Some(true)
    );
    assert_eq!(dict.get("ShadingType"), Some(&reference(4)));
    assert!(matches!(
        shading(&resolved, Some(&reader)).unwrap(),
        Cow::Borrowed(_)
    ));
}

#[test]
fn dictionary_nulls_are_absent_but_array_nulls_remain_invalid_values() {
    let reader = reader_with_objects(&[PdfObject::Null]);
    let mut dict = PdfDictionary::empty();
    for key in [
        "Domain",
        "Extend",
        "Function",
        "BBox",
        "Background",
        "AntiAlias",
    ] {
        dict.insert(key, reference(4));
    }
    dict.insert("Coords", PdfObject::Array(vec![PdfObject::Null; 4]));
    let resolved = shading(&dict, Some(&reader)).unwrap();
    for key in [
        "Domain",
        "Extend",
        "Function",
        "BBox",
        "Background",
        "AntiAlias",
    ] {
        assert!(!resolved.contains_key(key));
    }
    assert!(resolved.contains_key("Coords"));
}

#[test]
fn cycles_unavailable_readers_and_oversized_arrays_are_errors() {
    let reader = reader_with_objects(&[reference(5), reference(4)]);
    let mut dict = PdfDictionary::empty();
    dict.insert("Coords", reference(4));
    assert!(shading(&dict, Some(&reader)).is_err());
    assert!(shading(&dict, None).is_err());
    dict.insert("Coords", PdfObject::Array(vec![reference(99); 7]));
    assert!(shading(&dict, Some(&reader))
        .unwrap_err()
        .contains("6-element"));
    dict.insert(
        "Functions",
        PdfObject::Array(vec![PdfObject::Null; MAX_STITCHING_FUNCTIONS + 1]),
    );
    assert!(function(&dict, Some(&reader))
        .unwrap_err()
        .contains("child limit"));
}

#[test]
fn function_children_keep_source_references_and_optional_nulls_use_defaults() {
    let reader = reader_with_objects(&[
        PdfObject::Array(vec![reference(5)]),
        PdfObject::Dictionary(PdfDictionary::empty()),
        PdfObject::Null,
    ]);
    let mut dict = PdfDictionary::empty();
    dict.insert("Functions", reference(4));
    dict.insert("Range", reference(6));
    let normalized = function(&dict, Some(&reader)).unwrap();
    assert_eq!(
        normalized.get("Functions").unwrap().as_array().unwrap(),
        &vec![reference(5)]
    );
    assert!(!normalized.contains_key("Range"));
}

#[test]
fn pattern_parameters_share_the_same_reference_and_null_rules() {
    let reader = reader_with_objects(&[
        PdfObject::Integer(2),
        numbers(&[1.0, 0.0, 0.0, 1.0, 2.0, 3.0]),
        PdfObject::Null,
    ]);
    let mut dict = PdfDictionary::empty();
    dict.insert("PatternType", reference(4));
    dict.insert("Matrix", reference(5));
    dict.insert("Shading", reference(6));
    let result = pattern(&dict, Some(&reader)).unwrap();
    assert_eq!(result.get_integer("PatternType"), Some(2));
    assert_eq!(result.get("Matrix").unwrap().as_array().unwrap().len(), 6);
    assert!(!result.contains_key("Shading"));
}
