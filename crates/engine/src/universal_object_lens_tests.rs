use super::*;
use crate::writer::{OutputObject, PdfWriter};
use std::collections::BTreeMap;

fn object_reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}

fn cross_reference_lens_fixture() -> Vec<u8> {
    let mut catalog = PdfDictionary::empty();
    catalog.insert("Type", PdfObject::Name("Catalog".into()));
    catalog.insert("Pages", object_reference(2));
    let mut pages = PdfDictionary::empty();
    pages.insert("Type", PdfObject::Name("Pages".into()));
    pages.insert("Count", PdfObject::Integer(0));
    pages.insert("Kids", PdfObject::Array(Vec::new()));
    let mut root = PdfDictionary::empty();
    root.insert("Child", object_reference(4));
    root.insert("Unchanged", PdfObject::String(b"root".to_vec()));
    let mut child = PdfDictionary::empty();
    child.insert("Value", PdfObject::Integer(1));
    child.insert("Back", object_reference(3));
    child.insert("Unchanged", PdfObject::String(b"child".to_vec()));
    let mut second_root = PdfDictionary::empty();
    second_root.insert("Child", object_reference(4));
    second_root.insert("Unchanged", PdfObject::String(b"second-root".to_vec()));
    PdfWriter::new(
        vec![
            OutputObject {
                number: 1,
                object: PdfObject::Dictionary(catalog),
            },
            OutputObject {
                number: 2,
                object: PdfObject::Dictionary(pages),
            },
            OutputObject {
                number: 3,
                object: PdfObject::Dictionary(root),
            },
            OutputObject {
                number: 4,
                object: PdfObject::Dictionary(child),
            },
            OutputObject {
                number: 5,
                object: PdfObject::Dictionary(second_root),
            },
            OutputObject {
                number: 6,
                object: PdfObject::Stream {
                    dict: PdfDictionary::new(BTreeMap::from([
                        ("Length".into(), PdfObject::Integer(3)),
                        ("Filter".into(), PdfObject::Name("ASCIIHexDecode".into())),
                        ("DecodeParms".into(), PdfObject::Null),
                        ("Subtype".into(), PdfObject::Name("Opaque".into())),
                    ])),
                    raw: b"00>".to_vec(),
                },
            },
        ],
        1,
    )
    .write()
    .unwrap()
}

#[test]
fn direct_object_lens_replaces_one_leaf_and_preserves_opaque_siblings() {
    let mut config = PdfDictionary::empty();
    config.insert("Mode", PdfObject::Name("Old".into()));
    config.insert(
        "Thresholds",
        PdfObject::Array(vec![PdfObject::Integer(1), PdfObject::Integer(2)]),
    );
    let mut stream_dict = PdfDictionary::empty();
    stream_dict.insert("Length", PdfObject::Integer(5));
    stream_dict.insert("Subtype", PdfObject::Name("Opaque".into()));
    let opaque = PdfObject::Stream {
        dict: stream_dict,
        raw: vec![0, 1, 2, 3, 4],
    };
    let mut root = PdfDictionary::empty();
    root.insert("Config", PdfObject::Dictionary(config));
    root.insert("OpaqueSibling", opaque.clone());
    let mut object = PdfObject::Dictionary(root);
    let path = vec![
        UniversalObjectPathSegmentV2::Key {
            key: "Config".into(),
        },
        UniversalObjectPathSegmentV2::Key { key: "Mode".into() },
    ];
    validate_object_lens_path_v2(&object, &path).unwrap();
    let before = object.clone();
    let replacement = PdfObject::Name("New".into());
    replace_object_lens_value_v2(&mut object, &path, replacement.clone()).unwrap();
    verify_object_lens_laws_v2(
        &before,
        &object,
        &path,
        UniversalObjectLensActionV2::Replace,
        &replacement,
    )
    .unwrap();

    let PdfObject::Dictionary(root) = object else {
        panic!("root dictionary disappeared")
    };
    assert_eq!(root.get("OpaqueSibling"), Some(&opaque));
    assert_eq!(
        root.get_dict("Config").unwrap().get_name("Mode"),
        Some("New")
    );
    assert_eq!(
        root.get_dict("Config").unwrap().get_array("Thresholds"),
        Some(&[PdfObject::Integer(1), PdfObject::Integer(2)][..])
    );
}

#[test]
fn object_lens_refuses_indirect_crossing_and_stream_encoding_changes() {
    let indirect = PdfObject::Dictionary(PdfDictionary::new(BTreeMap::from([(
        "Child".into(),
        PdfObject::Reference {
            number: 7,
            generation: 0,
        },
    )])));
    assert!(validate_object_lens_path_v2(
        &indirect,
        &[
            UniversalObjectPathSegmentV2::Key {
                key: "Child".into()
            },
            UniversalObjectPathSegmentV2::Key {
                key: "Value".into()
            }
        ]
    )
    .is_err());

    let stream = PdfObject::Stream {
        dict: PdfDictionary::new(BTreeMap::from([
            ("Length".into(), PdfObject::Integer(3)),
            ("Filter".into(), PdfObject::Name("FlateDecode".into())),
        ])),
        raw: vec![1, 2, 3],
    };
    assert!(validate_object_lens_path_v2(
        &stream,
        &[UniversalObjectPathSegmentV2::Key {
            key: "Filter".into()
        }]
    )
    .is_err());
}

#[test]
fn explicit_stream_encoding_lens_reencodes_atomically_and_preserves_siblings() {
    let mut dict = PdfDictionary::empty();
    dict.insert("Length", PdfObject::Integer(3));
    dict.insert("Filter", PdfObject::Name("ASCIIHexDecode".into()));
    dict.insert("DecodeParms", PdfObject::Null);
    dict.insert("DL", PdfObject::Integer(1));
    dict.insert("Subtype", PdfObject::Name("Opaque".into()));
    dict.insert("VendorState", PdfObject::String(vec![0, 255, 1]));
    let before = PdfObject::Stream {
        dict,
        raw: b"00>".to_vec(),
    };
    let mut after = before.clone();
    apply_stream_encoding_update_v2(
        &mut after,
        &[],
        &UniversalStreamEncodingUpdateV2::Flate {
            data: b"decoded payload".to_vec(),
        },
        &BTreeMap::new(),
    )
    .unwrap();
    verify_stream_encoding_laws_v2(&before, &after, &[]).unwrap();
    let PdfObject::Stream { dict, raw } = &after else {
        panic!("stream encoding lens replaced the stream kind")
    };
    assert_eq!(dict.get_name("Filter"), Some("FlateDecode"));
    assert!(dict.get("DecodeParms").is_none());
    assert!(dict.get("DL").is_none());
    assert_eq!(dict.get_name("Subtype"), Some("Opaque"));
    assert_eq!(
        dict.get("VendorState"),
        Some(&PdfObject::String(vec![0, 255, 1]))
    );
    assert_eq!(dict.get_integer("Length"), Some(raw.len() as i64));

    let filters = UniversalPdfValueV2::Array {
        items: vec![
            UniversalPdfValueV2::Name {
                value: "ASCII85Decode".into(),
            },
            UniversalPdfValueV2::Name {
                value: "FlateDecode".into(),
            },
        ],
    };
    assert_eq!(stream_encoding_filter_count_v2(&filters).unwrap(), 2);
    assert!(validate_stream_decode_parms_v2(
        &UniversalPdfValueV2::Array {
            items: vec![UniversalPdfValueV2::Null]
        },
        2
    )
    .is_err());
    assert!(stream_encoding_filter_count_v2(&UniversalPdfValueV2::Name {
        value: "Crypt".into()
    })
    .is_err());
    assert!(validate_stream_encoding_target_v2(&PdfObject::Stream {
        dict: PdfDictionary::new(BTreeMap::from([(
            "F".into(),
            PdfObject::String(b"external.bin".to_vec())
        )])),
        raw: Vec::new()
    })
    .is_err());

    let input = cross_reference_lens_fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let original = engine.document().reader().get_object(6, 0).unwrap();
    let request = UniversalObjectGraphEditRequestV2 {
        mutations: vec![UniversalObjectMutationV2 {
            target: UniversalObjectTargetV2::Existing {
                number: 6,
                generation: 0,
                expected_fingerprint: universal_object_fingerprint_v2(&original),
            },
            path: Vec::new(),
            lens_action: UniversalObjectLensActionV2::Replace,
            stream_encoding: Some(UniversalStreamEncodingUpdateV2::Unfiltered {
                data: b"plain bytes".to_vec(),
            }),
            value: UniversalPdfValueV2::Null,
        }],
        affected_pages: Vec::new(),
        acknowledge_global_resource_impact: true,
    };
    let plan = plan_object_graph_edit_v2(&input, &request, &revision_id(&input)).unwrap();
    assert_eq!(plan.report["stream_encoding_lens_count"], 1);
    let (output, report, _, _, _) = apply_object_graph_edit_v2(
        &input,
        &request,
        UniversalMutationModeV2::PreserveSignatures,
    )
    .unwrap();
    assert_eq!(report["stream_encoding_inverse_law_checks"], 1);
    assert_eq!(report["decoded_stream_postconditions_verified"], 1);
    let reopened = ContentEngine::open_bytes(output).unwrap();
    let PdfObject::Stream { dict, raw } = reopened.document().reader().get_object(6, 0).unwrap()
    else {
        panic!("reopened encoding target is not a stream")
    };
    assert_eq!(raw, b"plain bytes");
    assert!(dict.get("Filter").is_none());
    assert!(dict.get("DecodeParms").is_none());
    assert_eq!(dict.get_name("Subtype"), Some("Opaque"));
}

#[test]
fn object_lens_insert_and_remove_preserve_container_order_and_require_exact_state() {
    let mut root = PdfObject::Dictionary(PdfDictionary::new(BTreeMap::from([
        ("A".into(), PdfObject::Integer(1)),
        (
            "Items".into(),
            PdfObject::Array(vec![PdfObject::Integer(10), PdfObject::Integer(30)]),
        ),
    ])));
    let insert_key = vec![UniversalObjectPathSegmentV2::Key { key: "B".into() }];
    validate_object_lens_mutation_v2(&root, &insert_key, UniversalObjectLensActionV2::Insert)
        .unwrap();
    let before_insert_key = root.clone();
    let inserted_key = PdfObject::Integer(2);
    insert_object_lens_value_v2(&mut root, &insert_key, inserted_key.clone()).unwrap();
    verify_object_lens_laws_v2(
        &before_insert_key,
        &root,
        &insert_key,
        UniversalObjectLensActionV2::Insert,
        &inserted_key,
    )
    .unwrap();
    assert!(validate_object_lens_mutation_v2(
        &root,
        &insert_key,
        UniversalObjectLensActionV2::Insert
    )
    .is_err());

    let insert_array = vec![
        UniversalObjectPathSegmentV2::Key {
            key: "Items".into(),
        },
        UniversalObjectPathSegmentV2::Index { index: 1 },
    ];
    validate_object_lens_mutation_v2(&root, &insert_array, UniversalObjectLensActionV2::Insert)
        .unwrap();
    let before_insert_array = root.clone();
    let inserted_array = PdfObject::Integer(20);
    insert_object_lens_value_v2(&mut root, &insert_array, inserted_array.clone()).unwrap();
    verify_object_lens_laws_v2(
        &before_insert_array,
        &root,
        &insert_array,
        UniversalObjectLensActionV2::Insert,
        &inserted_array,
    )
    .unwrap();
    let remove_first = vec![
        UniversalObjectPathSegmentV2::Key {
            key: "Items".into(),
        },
        UniversalObjectPathSegmentV2::Index { index: 0 },
    ];
    validate_object_lens_mutation_v2(&root, &remove_first, UniversalObjectLensActionV2::Remove)
        .unwrap();
    let before_remove = root.clone();
    remove_object_lens_value_v2(&mut root, &remove_first).unwrap();
    verify_object_lens_laws_v2(
        &before_remove,
        &root,
        &remove_first,
        UniversalObjectLensActionV2::Remove,
        &PdfObject::Null,
    )
    .unwrap();

    let PdfObject::Dictionary(root) = root else {
        panic!("root dictionary disappeared")
    };
    assert_eq!(root.get_integer("A"), Some(1));
    assert_eq!(root.get_integer("B"), Some(2));
    assert_eq!(
        root.get_array("Items"),
        Some(&[PdfObject::Integer(20), PdfObject::Integer(30)][..])
    );
}

#[test]
fn cross_reference_lens_binds_each_owner_and_writes_only_the_resolved_target() {
    let input = cross_reference_lens_fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let reader = engine.document().reader();
    let root = reader.get_object(3, 0).unwrap();
    let child = reader.get_object(4, 0).unwrap();
    let root_fingerprint = universal_object_fingerprint_v2(&root);
    let child_fingerprint = universal_object_fingerprint_v2(&child);
    let request = UniversalObjectGraphEditRequestV2 {
        mutations: vec![UniversalObjectMutationV2 {
            target: UniversalObjectTargetV2::Existing {
                number: 3,
                generation: 0,
                expected_fingerprint: root_fingerprint.clone(),
            },
            path: vec![
                UniversalObjectPathSegmentV2::Key {
                    key: "Child".into(),
                },
                UniversalObjectPathSegmentV2::Dereference {
                    expected_fingerprint: child_fingerprint.clone(),
                },
                UniversalObjectPathSegmentV2::Key {
                    key: "Value".into(),
                },
            ],
            lens_action: UniversalObjectLensActionV2::Replace,
            stream_encoding: None,
            value: UniversalPdfValueV2::Integer { value: 2 },
        }],
        affected_pages: Vec::new(),
        acknowledge_global_resource_impact: true,
    };
    let plan = plan_object_graph_edit_v2(&input, &request, &revision_id(&input)).unwrap();
    assert_eq!(plan.write_set, vec!["object-4-0"]);
    assert!(plan
        .read_set
        .contains(&format!("object-3-0:{root_fingerprint}")));
    assert!(plan
        .read_set
        .contains(&format!("object-4-0:{child_fingerprint}")));

    let (output, report, _, affected_objects, _) = apply_object_graph_edit_v2(
        &input,
        &request,
        UniversalMutationModeV2::PreserveSignatures,
    )
    .unwrap();
    assert_eq!(affected_objects, vec!["object-4-0"]);
    assert_eq!(
        report["preserved_traversal_objects_verified"],
        serde_json::json!(1)
    );
    assert_eq!(
        report["exact_parsed_model_lens_law_checks"],
        serde_json::json!(1)
    );
    let reopened = ContentEngine::open_bytes(output).unwrap();
    let output_root = reopened.document().reader().get_object(3, 0).unwrap();
    let output_child = reopened.document().reader().get_object(4, 0).unwrap();
    assert_eq!(
        universal_object_fingerprint_v2(&output_root),
        root_fingerprint
    );
    let PdfObject::Dictionary(output_child) = output_child else {
        panic!("cross-reference lens child dictionary disappeared")
    };
    assert_eq!(output_child.get_integer("Value"), Some(2));
    assert_eq!(
        output_child.get("Unchanged"),
        Some(&PdfObject::String(b"child".to_vec()))
    );
}

#[test]
fn cross_reference_lens_refuses_stale_fingerprints_and_cycles() {
    let input = cross_reference_lens_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let root = reader.get_object(3, 0).unwrap();
    let child = reader.get_object(4, 0).unwrap();
    let root_fingerprint = universal_object_fingerprint_v2(&root);
    let child_fingerprint = universal_object_fingerprint_v2(&child);

    assert!(resolve_object_lens_owner_v2(
        reader,
        3,
        0,
        root.clone(),
        &[
            UniversalObjectPathSegmentV2::Key {
                key: "Child".into(),
            },
            UniversalObjectPathSegmentV2::Dereference {
                expected_fingerprint: "0".repeat(64),
            },
            UniversalObjectPathSegmentV2::Key {
                key: "Value".into(),
            },
        ],
        UniversalObjectLensActionV2::Replace,
    )
    .is_err());

    assert!(resolve_object_lens_owner_v2(
        reader,
        3,
        0,
        root,
        &[
            UniversalObjectPathSegmentV2::Key {
                key: "Child".into(),
            },
            UniversalObjectPathSegmentV2::Dereference {
                expected_fingerprint: child_fingerprint,
            },
            UniversalObjectPathSegmentV2::Key { key: "Back".into() },
            UniversalObjectPathSegmentV2::Dereference {
                expected_fingerprint: root_fingerprint,
            },
        ],
        UniversalObjectLensActionV2::Replace,
    )
    .is_err());
}

#[test]
fn cross_reference_lens_refuses_competing_resolved_write_targets() {
    let input = cross_reference_lens_fixture();
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let reader = engine.document().reader();
    let child_fingerprint = universal_object_fingerprint_v2(&reader.get_object(4, 0).unwrap());
    let mutations = [3, 5]
        .into_iter()
        .map(|number| UniversalObjectMutationV2 {
            target: UniversalObjectTargetV2::Existing {
                number,
                generation: 0,
                expected_fingerprint: universal_object_fingerprint_v2(
                    &reader.get_object(number, 0).unwrap(),
                ),
            },
            path: vec![
                UniversalObjectPathSegmentV2::Key {
                    key: "Child".into(),
                },
                UniversalObjectPathSegmentV2::Dereference {
                    expected_fingerprint: child_fingerprint.clone(),
                },
                UniversalObjectPathSegmentV2::Key {
                    key: "Value".into(),
                },
            ],
            lens_action: UniversalObjectLensActionV2::Replace,
            stream_encoding: None,
            value: UniversalPdfValueV2::Integer {
                value: i64::from(number),
            },
        })
        .collect::<Vec<_>>();
    let request = UniversalObjectGraphEditRequestV2 {
        mutations,
        affected_pages: Vec::new(),
        acknowledge_global_resource_impact: true,
    };
    assert!(plan_object_graph_edit_v2(&input, &request, &revision_id(&input)).is_err());
}
