//! Source-only feature-version regressions; execution deferred.
use super::*;
use crate::authoring::{PageSize, PdfBuilder};

fn opentype() -> PdfObject {
    let mut dict = PdfDictionary::empty();
    dict.insert("Subtype", PdfObject::Name("OpenType".into()));
    PdfObject::Stream {
        dict,
        raw: Vec::new(),
    }
}
fn source(version: &str) -> crate::ContentEngine {
    let mut doc = PdfBuilder::new()
        .with_writer_mode(WriterMode::ClassicXref)
        .with_version(version);
    doc.add_page(PageSize::LETTER);
    crate::ContentEngine::open_bytes(doc.to_bytes().unwrap()).unwrap()
}

#[test]
fn writer_features_cannot_be_downgraded_by_setter_order() {
    let writer = PdfWriter::new(Vec::new(), 1)
        .with_mode(WriterMode::XrefStream)
        .with_version("1.2");
    assert_eq!(header(&writer).unwrap(), "1.5");
    let writer = PdfWriter::new(
        vec![OutputObject {
            number: 1,
            object: opentype(),
        }],
        1,
    )
    .with_mode(WriterMode::ClassicXref)
    .with_version("1.4");
    assert_eq!(header(&writer).unwrap(), "1.6");
    assert_eq!(header(&writer.with_version("2.0")).unwrap(), "2.0");
}

#[test]
fn incremental_version_composes_with_existing_catalog_mutations() {
    let engine = source("1.4");
    let reader = engine.document().reader();
    let (number, generation) = reader.root_reference().unwrap();
    let mut object = reader.get_object(number, generation).unwrap();
    let PdfObject::Dictionary(dict) = &mut object else {
        panic!()
    };
    dict.insert(
        "WFKeep",
        PdfObject::String(b"other pending mutation".to_vec()),
    );
    let mut changes = vec![
        IncrementalObject {
            number,
            generation,
            object,
        },
        IncrementalObject {
            number: 100,
            generation: 0,
            object: opentype(),
        },
    ];
    incremental(reader, &mut changes).unwrap();
    incremental(reader, &mut changes).unwrap();
    assert_eq!(changes.len(), 2);
    let dict = changes[0].object.as_dict().unwrap();
    assert_eq!(dict.get_name("Version"), Some("1.6"));
    assert!(dict.contains_key("WFKeep"));
}

#[test]
fn newer_catalog_version_is_not_downgraded() {
    let engine = source("1.4");
    let reader = engine.document().reader();
    let (number, generation) = reader.root_reference().unwrap();
    let mut object = reader.get_object(number, generation).unwrap();
    let PdfObject::Dictionary(dict) = &mut object else {
        panic!()
    };
    dict.insert("Version", PdfObject::Name("2.0".into()));
    let mut changes = vec![
        IncrementalObject {
            number,
            generation,
            object,
        },
        IncrementalObject {
            number: 100,
            generation: 0,
            object: opentype(),
        },
    ];
    incremental(reader, &mut changes).unwrap();
    assert_eq!(
        changes[0].object.as_dict().unwrap().get_name("Version"),
        Some("2.0")
    );
}

#[test]
fn no_opentype_feature_does_not_add_a_catalog_revision() {
    let engine = source("1.4");
    let reader = engine.document().reader();
    let mut changes = vec![IncrementalObject {
        number: 100,
        generation: 0,
        object: PdfObject::Integer(1),
    }];
    incremental(reader, &mut changes).unwrap();
    assert_eq!(changes.len(), 1);
}

#[test]
fn output_derived_contract_inventory_reports_writer_added_catalog_version() {
    use sha2::{Digest, Sha256};
    let engine = source("1.4");
    let reader = engine.document().reader();
    let input = reader.file_bytes();
    let (root, generation) = reader.root_reference().unwrap();
    let output = write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: 100,
            generation: 0,
            object: opentype(),
        }],
    )
    .unwrap();
    let contract = crate::edit_contracts::EditContract {
        input_sha256: format!("{:x}", Sha256::digest(input)),
        inventory_objects: true,
        ..Default::default()
    };
    let report = crate::edit_contracts::verify_edit_contract(input, &output, &contract).unwrap();
    assert!(report
        .object_changes
        .unwrap()
        .iter()
        .any(|change| change.number == root && change.generation == generation));
}

#[test]
fn malformed_catalog_version_and_generation_are_rejected() {
    let engine = source("1.4");
    let reader = engine.document().reader();
    let (number, generation) = reader.root_reference().unwrap();
    let mut object = reader.get_object(number, generation).unwrap();
    let PdfObject::Dictionary(dict) = &mut object else {
        panic!()
    };
    dict.insert("Version", PdfObject::Integer(16));
    let mut changes = vec![
        IncrementalObject {
            number,
            generation,
            object,
        },
        IncrementalObject {
            number: 100,
            generation: 0,
            object: opentype(),
        },
    ];
    assert!(incremental(reader, &mut changes).is_err());
    changes[0].generation += 1;
    assert!(incremental(reader, &mut changes).is_err());
}
