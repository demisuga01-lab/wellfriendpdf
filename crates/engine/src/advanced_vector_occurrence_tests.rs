//! Unexecuted source regressions: occurrence ownership, not raster fidelity.
use super::super::tests::{bare_vector_fixture, nested_shared_form_fixture, shared_form_fixture};
use super::*;
use crate::PdfDictionary;

fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}

fn shared_pages(input: &[u8], repeated: bool) -> Vec<u8> {
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let mut page_dict = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    let second_page = next_advanced_object_number(reader, "fixture").unwrap();
    let original_page = page_dict.clone();
    if repeated {
        page_dict.insert(
            "Contents",
            PdfObject::Array(vec![reference(4), reference(4)]),
        );
    }
    let mut pages = reader.get_object(2, 0).unwrap().as_dict().unwrap().clone();
    pages.insert(
        "Kids",
        PdfObject::Array(vec![reference(3), reference(second_page)]),
    );
    pages.insert("Count", PdfObject::Integer(2));
    write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: 2,
                generation: 0,
                object: PdfObject::Dictionary(pages),
            },
            IncrementalObject {
                number: 3,
                generation: 0,
                object: PdfObject::Dictionary(page_dict),
            },
            IncrementalObject {
                number: second_page,
                generation: 0,
                object: PdfObject::Dictionary(original_page),
            },
        ],
    )
    .unwrap()
}

fn unchanged_object(input: &[u8], output: &[u8], number: u32) {
    let before = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let after = ContentEngine::open_bytes(output.to_vec()).unwrap();
    assert_eq!(
        before.document().reader().get_object(number, 0).unwrap(),
        after.document().reader().get_object(number, 0).unwrap()
    );
}

fn page_vectors(input: &[u8], page: usize) -> serde_json::Value {
    serde_json::to_value(list_vector_objects(input, page).unwrap().objects).unwrap()
}

fn assert_report_bound(output: &[u8], report: &VectorEditReport) {
    let after = report.after.as_ref().unwrap();
    let inventory = list_vector_objects(output, after.provenance.page).unwrap();
    let saved = inventory
        .objects
        .iter()
        .find(|object| object.stable_id == after.stable_id)
        .unwrap();
    assert_eq!(
        serde_json::to_value(saved).unwrap(),
        serde_json::to_value(after).unwrap()
    );
}

#[test]
fn repeated_contents_have_distinct_vector_identities() {
    let input = shared_pages(&shared_form_fixture(), true);
    let inventory = list_vector_objects(&input, 1).unwrap();
    assert_eq!(inventory.objects.len(), 4);
    let ids = inventory
        .objects
        .iter()
        .map(|object| &object.stable_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), inventory.objects.len());
    let direct = shared_pages(&bare_vector_fixture(b"0 0 10 10 re S\n"), true);
    let inventory = list_vector_objects(&direct, 1).unwrap();
    assert_eq!(inventory.objects.len(), 2);
    assert_ne!(
        inventory.objects[0].stable_id,
        inventory.objects[1].stable_id
    );
}

#[test]
fn top_and_nested_form_clones_isolate_the_selected_page_slot() {
    for nested in [false, true] {
        let source = if nested {
            nested_shared_form_fixture()
        } else {
            shared_form_fixture()
        };
        for repeated in [false, true] {
            let input = shared_pages(&source, repeated);
            let inventory = list_vector_objects(&input, 1).unwrap();
            let selected = inventory
                .objects
                .iter()
                .find(|object| object.provenance.content_stream_index == usize::from(repeated))
                .unwrap();
            let options = VectorEditOptions {
                shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
                ..Default::default()
            };
            let (output, report) = edit_vector_object(
                &input,
                1,
                &selected.stable_id,
                VectorEditOperation::Move { dx: 3.0, dy: 4.0 },
                &options,
            )
            .unwrap();
            assert!(output.starts_with(&input));
            assert_eq!(page_vectors(&input, 2), page_vectors(&output, 2));
            unchanged_object(&input, &output, 4);
            unchanged_object(&input, &output, 5);
            if nested {
                unchanged_object(&input, &output, 6);
            }
            let saved = ContentEngine::open_bytes(output.clone()).unwrap();
            let page = saved.document().get_page(1).unwrap();
            assert_ne!(page.contents[usize::from(repeated)], (4, 0));
            if repeated {
                assert_eq!(page.contents[0], (4, 0));
            }
            assert_eq!(saved.document().get_page(2).unwrap().contents, vec![(4, 0)]);
            assert_report_bound(&output, &report);
            let after = report.after.as_ref().unwrap();
            assert_eq!(
                after.bbox,
                [
                    selected.bbox[0] + 3.0,
                    selected.bbox[1] + 4.0,
                    selected.bbox[2] + 3.0,
                    selected.bbox[3] + 4.0
                ]
            );
            let (second_output, second_report) = edit_vector_object(
                &output,
                1,
                &after.stable_id,
                VectorEditOperation::Move { dx: 1.0, dy: 0.0 },
                &options,
            )
            .unwrap();
            assert_report_bound(&second_output, &second_report);
            assert_eq!(page_vectors(&input, 2), page_vectors(&second_output, 2));
        }
    }
}

#[test]
fn direct_vector_edits_and_deletion_are_occurrence_local() {
    let input = shared_pages(
        &bare_vector_fixture(b"0 0 10 10 re S\n30 30 5 5 re S\n"),
        true,
    );
    let before = list_vector_objects(&input, 1).unwrap();
    let selected = before
        .objects
        .iter()
        .find(|object| object.provenance.content_stream_index == 1)
        .unwrap();
    let (output, report) = edit_vector_object(
        &input,
        1,
        &selected.stable_id,
        VectorEditOperation::Move { dx: 2.0, dy: 3.0 },
        &VectorEditOptions::default(),
    )
    .unwrap();
    unchanged_object(&input, &output, 4);
    assert_eq!(page_vectors(&input, 2), page_vectors(&output, 2));
    assert_report_bound(&output, &report);
    let (deleted, report) = edit_vector_object(
        &output,
        1,
        &report.after.unwrap().stable_id,
        VectorEditOperation::Delete,
        &VectorEditOptions::default(),
    )
    .unwrap();
    assert!(report.after.is_none());
    assert_eq!(list_vector_objects(&deleted, 1).unwrap().objects.len(), 3);
    assert_eq!(page_vectors(&input, 2), page_vectors(&deleted, 2));
}

#[test]
fn group_and_ungroup_keep_repeated_streams_separate() {
    let input = shared_pages(
        &bare_vector_fixture(b"0 0 10 10 re S\n30 30 5 5 re S\n"),
        true,
    );
    let before = list_vector_objects(&input, 1).unwrap();
    let selected = before
        .objects
        .iter()
        .filter(|object| object.provenance.content_stream_index == 1)
        .collect::<Vec<_>>();
    let (output, report) = edit_vector_object(
        &input,
        1,
        &selected[1].stable_id,
        VectorEditOperation::GroupWith {
            stable_ids: vec![selected[0].stable_id.clone()],
        },
        &VectorEditOptions::default(),
    )
    .unwrap();
    assert_report_bound(&output, &report);
    unchanged_object(&input, &output, 4);
    let grouped = list_vector_objects(&output, 1).unwrap();
    assert_eq!(
        grouped
            .objects
            .iter()
            .filter(|object| !object.provenance.wellfriendpdf_groups.is_empty())
            .count(),
        2
    );
    assert_eq!(page_vectors(&input, 2), page_vectors(&output, 2));
    let (ungrouped, report) = edit_vector_object(
        &output,
        1,
        &report.after.unwrap().stable_id,
        VectorEditOperation::Ungroup,
        &VectorEditOptions::default(),
    )
    .unwrap();
    assert_report_bound(&ungrouped, &report);
    assert!(list_vector_objects(&ungrouped, 1)
        .unwrap()
        .objects
        .iter()
        .all(|object| object.provenance.wellfriendpdf_groups.is_empty()));
    assert_eq!(page_vectors(&input, 2), page_vectors(&ungrouped, 2));
}

#[test]
fn duplicate_returns_the_new_copy_not_the_retained_original() {
    for form_owned in [false, true] {
        let source = if form_owned {
            shared_form_fixture()
        } else {
            bare_vector_fixture(b"0 0 10 10 re S\n")
        };
        let input = shared_pages(&source, true);
        let inventory = list_vector_objects(&input, 1).unwrap();
        let selected = inventory
            .objects
            .iter()
            .find(|object| object.provenance.content_stream_index == 1)
            .unwrap();
        let options = VectorEditOptions {
            shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
            ..Default::default()
        };
        let (output, report) = edit_vector_object(
            &input,
            1,
            &selected.stable_id,
            VectorEditOperation::Duplicate { dx: 11.0, dy: 12.0 },
            &options,
        )
        .unwrap();
        assert_report_bound(&output, &report);
        let after = report.after.unwrap();
        assert_eq!(
            after.bbox,
            [
                selected.bbox[0] + 11.0,
                selected.bbox[1] + 12.0,
                selected.bbox[2] + 11.0,
                selected.bbox[3] + 12.0
            ]
        );
        assert_eq!(
            list_vector_objects(&output, 1).unwrap().objects.len(),
            inventory.objects.len() + 1
        );
        assert_eq!(page_vectors(&input, 2), page_vectors(&output, 2));
    }
}

#[test]
fn invocation_chain_must_match_the_selected_root_slot_and_resources() {
    let input = shared_pages(&nested_shared_form_fixture(), true);
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let page = engine.document().get_page(1).unwrap();
    let reader = engine.document().reader();
    let before = list_vector_objects(&input, 1).unwrap().objects.remove(0);
    assert_eq!(
        invocation_resources(reader, &page, &before).unwrap().len(),
        2
    );
    let mut mismatched = before.clone();
    mismatched.provenance.form_invocation_path[0].owner_stream_object = 5;
    assert!(invocation_resources(reader, &page, &mismatched).is_err());
    let mut mismatched = before.clone();
    mismatched.provenance.form_invocation_path[1].resource_name = "Unknown".into();
    assert!(invocation_resources(reader, &page, &mismatched).is_err());
    let mut mismatched = before;
    mismatched.provenance.content_stream_index = 2;
    assert!(invocation_resources(reader, &page, &mismatched).is_err());
}

#[test]
fn grouping_cannot_mix_identical_stream_occurrences() {
    let input = shared_pages(
        &bare_vector_fixture(b"0 0 10 10 re S\n30 30 5 5 re S\n"),
        true,
    );
    let before = list_vector_objects(&input, 1).unwrap();
    let first = before
        .objects
        .iter()
        .find(|object| object.provenance.content_stream_index == 0)
        .unwrap();
    let second = before
        .objects
        .iter()
        .find(|object| object.provenance.content_stream_index == 1)
        .unwrap();
    assert!(edit_vector_object(
        &input,
        1,
        &second.stable_id,
        VectorEditOperation::GroupWith {
            stable_ids: vec![first.stable_id.clone()]
        },
        &VectorEditOptions::default()
    )
    .is_err());
}

#[test]
fn z_order_is_occurrence_local_and_returns_saved_identity() {
    let input = shared_pages(
        &bare_vector_fixture(b"0 0 10 10 re S\n30 30 5 5 re S\n"),
        true,
    );
    let before = list_vector_objects(&input, 1).unwrap();
    let selected = before
        .objects
        .iter()
        .find(|object| object.provenance.content_stream_index == 1)
        .unwrap();
    let (output, report) = edit_vector_object(
        &input,
        1,
        &selected.stable_id,
        VectorEditOperation::BringForward,
        &VectorEditOptions::default(),
    )
    .unwrap();
    unchanged_object(&input, &output, 4);
    assert_eq!(page_vectors(&input, 2), page_vectors(&output, 2));
    assert_report_bound(&output, &report);
}

#[test]
fn explicit_edit_all_keeps_shared_form_semantics() {
    let input = shared_pages(&shared_form_fixture(), true);
    let before = list_vector_objects(&input, 1).unwrap();
    let selected = before
        .objects
        .iter()
        .find(|object| object.provenance.content_stream_index == 1)
        .unwrap();
    let (output, report) = edit_vector_object(
        &input,
        1,
        &selected.stable_id,
        VectorEditOperation::Move { dx: 3.0, dy: 4.0 },
        &VectorEditOptions {
            shared_form_policy: SharedFormEditPolicy::EditAllUses,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(report.cloned_form.is_none());
    assert_ne!(page_vectors(&input, 2), page_vectors(&output, 2));
    unchanged_object(&input, &output, 4);
    assert_report_bound(&output, &report);
}

#[test]
fn nested_clone_materializes_inherited_form_resources() {
    let input = nested_shared_form_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let mut page_dict = reader.get_object(3, 0).unwrap().as_dict().unwrap().clone();
    let mut resources = page.resources.clone();
    let mut xobjects = resources.get_dict("XObject").unwrap().clone();
    xobjects.insert("Leaf", reference(6));
    resources.insert("XObject", PdfObject::Dictionary(xobjects));
    page_dict.insert("Resources", PdfObject::Dictionary(resources));
    let PdfObject::Stream { mut dict, raw } = reader.get_object(5, 0).unwrap() else {
        panic!()
    };
    dict.remove("Resources");
    let input = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: 3,
                generation: 0,
                object: PdfObject::Dictionary(page_dict),
            },
            IncrementalObject {
                number: 5,
                generation: 0,
                object: PdfObject::Stream { dict, raw },
            },
        ],
    )
    .unwrap();
    let before = list_vector_objects(&input, 1).unwrap();
    let (output, report) = edit_vector_object(
        &input,
        1,
        &before.objects[0].stable_id,
        VectorEditOperation::Move { dx: 3.0, dy: 4.0 },
        &VectorEditOptions {
            shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
            ..Default::default()
        },
    )
    .unwrap();
    unchanged_object(&input, &output, 4);
    unchanged_object(&input, &output, 5);
    unchanged_object(&input, &output, 6);
    assert_report_bound(&output, &report);
}

#[test]
fn stage_preserves_pending_resources_and_reserves_above_pending_objects() {
    let input = shared_pages(&shared_form_fixture(), true);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let mut page_dict = reader.get_object(3, 0).unwrap().as_dict().unwrap().clone();
    page_dict.insert("WFMarker", PdfObject::Integer(19));
    let mut updates = vec![
        IncrementalObject {
            number: 4,
            generation: 0,
            object: reader.get_object(4, 0).unwrap(),
        },
        IncrementalObject {
            number: 3,
            generation: 0,
            object: PdfObject::Dictionary(page_dict),
        },
        IncrementalObject {
            number: 100,
            generation: 0,
            object: PdfObject::Null,
        },
    ];
    let cloned = stage(reader, &page, 1, &mut updates).unwrap();
    assert_eq!(cloned, (101, 0));
    assert!(updates.iter().all(|update| update.number != 4));
    let dictionary = updates
        .iter()
        .find(|update| update.number == 3)
        .unwrap()
        .object
        .as_dict()
        .unwrap();
    assert_eq!(dictionary.get_integer("WFMarker"), Some(19));
    assert_eq!(
        dictionary.get_array("Contents").unwrap(),
        &[reference(4), reference(101)]
    );
}

#[test]
fn stage_rejects_stale_topology_duplicate_updates_and_object_exhaustion_atomically() {
    let input = shared_pages(&shared_form_fixture(), true);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let source = IncrementalObject {
        number: 4,
        generation: 0,
        object: reader.get_object(4, 0).unwrap(),
    };
    let mut dictionary = reader.get_object(3, 0).unwrap().as_dict().unwrap().clone();
    dictionary.insert("Contents", reference(4));
    for mut updates in [
        vec![source.clone(), source.clone()],
        vec![
            source.clone(),
            IncrementalObject {
                number: 3,
                generation: 0,
                object: PdfObject::Dictionary(dictionary),
            },
        ],
        vec![
            source.clone(),
            IncrementalObject {
                number: u32::MAX,
                generation: 0,
                object: PdfObject::Null,
            },
        ],
    ] {
        let before = format!("{updates:?}");
        assert!(stage(reader, &page, 1, &mut updates).is_err());
        assert_eq!(before, format!("{updates:?}"));
    }
    assert!(stage(reader, &page, 2, &mut vec![source]).is_err());
}

#[test]
fn cloning_does_not_duplicate_stream_structure_keys() {
    let input = shared_form_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let PdfObject::Stream { mut dict, raw } = reader.get_object(5, 0).unwrap() else {
        panic!()
    };
    dict.insert("StructParents", PdfObject::Integer(9));
    let input = write_incremental_update(
        reader,
        vec![IncrementalObject {
            number: 5,
            generation: 0,
            object: PdfObject::Stream { dict, raw },
        }],
    )
    .unwrap();
    let before = list_vector_objects(&input, 1).unwrap();
    let error = edit_vector_object(
        &input,
        1,
        &before.objects[0].stable_id,
        VectorEditOperation::Move { dx: 3.0, dy: 4.0 },
        &VectorEditOptions {
            shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("stream-owned structure"));
}

#[test]
fn clone_guard_follows_structure_k_without_following_parent_cycles() {
    let input = shared_form_fixture();
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    for stream_owned in [false, true] {
        let mut catalog = engine.document().get_catalog().unwrap();
        catalog.insert("StructTreeRoot", reference(7));
        let mut root = PdfDictionary::empty();
        root.insert("Type", PdfObject::Name("StructTreeRoot".into()));
        root.insert("K", reference(8));
        let mut element = PdfDictionary::empty();
        element.insert("Type", PdfObject::Name("StructElem".into()));
        element.insert("S", PdfObject::Name("Figure".into()));
        element.insert("P", reference(7));
        element.insert("Pg", reference(3));
        let mut mcr = PdfDictionary::empty();
        mcr.insert("Type", PdfObject::Name("MCR".into()));
        mcr.insert("MCID", PdfObject::Integer(0));
        if stream_owned {
            mcr.insert("Stm", reference(5));
        }
        element.insert("K", PdfObject::Dictionary(mcr));
        let input = write_incremental_update(
            reader,
            vec![
                IncrementalObject {
                    number: 1,
                    generation: 0,
                    object: PdfObject::Dictionary(catalog),
                },
                IncrementalObject {
                    number: 7,
                    generation: 0,
                    object: PdfObject::Dictionary(root),
                },
                IncrementalObject {
                    number: 8,
                    generation: 0,
                    object: PdfObject::Dictionary(element),
                },
            ],
        )
        .unwrap();
        let saved = ContentEngine::open_bytes(input).unwrap();
        let result = check_clone_ownership(saved.document().reader(), &BTreeSet::from([(5, 0)]));
        assert_eq!(result.is_err(), stream_owned);
    }
}
