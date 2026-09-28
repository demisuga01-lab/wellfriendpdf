//! Unexecuted regression source; no PDF workload was run for this change.
use super::*;
use crate::writer::{OutputObject, PdfWriter};

#[path = "universal_scoped_form_tests.rs"]
mod governed;

fn r(number: u32) -> PdfObject {
    reference((number, 0))
}
fn dict(entries: &[(&str, PdfObject)]) -> PdfDictionary {
    let mut result = PdfDictionary::empty();
    for (key, value) in entries {
        result.insert(*key, value.clone());
    }
    result
}
fn stream(mut dictionary: PdfDictionary, bytes: &[u8]) -> PdfObject {
    dictionary.insert("Length", PdfObject::Integer(bytes.len() as i64));
    PdfObject::Stream {
        dict: dictionary,
        raw: bytes.to_vec(),
    }
}
fn fixture(nested: bool, repeated: bool, second_font_differs: bool) -> Vec<u8> {
    let name = |value: &str| PdfObject::Name(value.into());
    let bbox = PdfObject::Array(
        vec![0, 0, 100, 80]
            .into_iter()
            .map(PdfObject::Integer)
            .collect(),
    );
    let font = dict(&[
        ("Type", name("Font")),
        ("Subtype", name("Type1")),
        ("BaseFont", name("Courier")),
        ("Encoding", name("WinAnsiEncoding")),
    ]);
    let alternate = dict(&[
        ("Type", name("Font")),
        ("Subtype", name("Type1")),
        ("BaseFont", name("Helvetica")),
        (
            "Encoding",
            PdfObject::Dictionary(dict(&[
                ("Type", name("Encoding")),
                ("BaseEncoding", name("WinAnsiEncoding")),
                (
                    "Differences",
                    PdfObject::Array(vec![PdfObject::Integer(65), name("Z")]),
                ),
            ])),
        ),
    ]);
    let resources = |font| {
        PdfObject::Dictionary(dict(&[
            ("Font", PdfObject::Dictionary(dict(&[("F1", r(font))]))),
            (
                "XObject",
                PdfObject::Dictionary(dict(&[
                    ("A", r(if nested { 8 } else { 5 })),
                    ("Leaf", r(5)),
                ])),
            ),
        ]))
    };
    let page = |font, repeat| {
        PdfObject::Dictionary(dict(&[
            ("Type", name("Page")),
            ("Parent", r(2)),
            ("MediaBox", bbox.clone()),
            ("Resources", resources(font)),
            (
                "Contents",
                if repeat {
                    PdfObject::Array(vec![r(4), r(4)])
                } else {
                    r(4)
                },
            ),
        ]))
    };
    let form = dict(&[
        ("Type", name("XObject")),
        ("Subtype", name("Form")),
        ("BBox", bbox.clone()),
        (
            "Matrix",
            PdfObject::Array(
                vec![1, 0, 0, 1, 2, 3]
                    .into_iter()
                    .map(PdfObject::Integer)
                    .collect(),
            ),
        ),
        (
            "Resources",
            PdfObject::Dictionary(dict(&[(
                "Font",
                PdfObject::Dictionary(dict(&[("F1", r(7))])),
            )])),
        ),
    ]);
    let mut objects = vec![
        OutputObject {
            number: 1,
            object: PdfObject::Dictionary(dict(&[("Type", name("Catalog")), ("Pages", r(2))])),
        },
        OutputObject {
            number: 2,
            object: PdfObject::Dictionary(dict(&[
                ("Type", name("Pages")),
                ("Count", PdfObject::Integer(2)),
                ("Kids", PdfObject::Array(vec![r(3), r(9)])),
            ])),
        },
        OutputObject {
            number: 3,
            object: page(6, repeated),
        },
        OutputObject {
            number: 4,
            object: stream(
                PdfDictionary::empty(),
                b"BT /F1 12 Tf ET\nq /A Do Q\nq 1 0 0 1 30 0 cm /A Do Q\n",
            ),
        },
        OutputObject {
            number: 5,
            object: stream(form, b"BT 1 0 0 1 10 20 Tm (ABC) Tj ET\n"),
        },
        OutputObject {
            number: 6,
            object: PdfObject::Dictionary(font),
        },
        OutputObject {
            number: 7,
            object: PdfObject::Dictionary(alternate),
        },
        OutputObject {
            number: 9,
            object: page(if second_font_differs { 7 } else { 6 }, false),
        },
    ];
    if nested {
        objects.push(OutputObject {
            number: 8,
            object: stream(
                dict(&[
                    ("Type", name("XObject")),
                    ("Subtype", name("Form")),
                    ("BBox", bbox),
                ]),
                b"q /Leaf Do Q\n",
            ),
        });
    }
    PdfWriter::new(objects, 1).write().unwrap()
}

fn selected(input: &[u8], slot: usize, nested: bool) -> FormTextTarget {
    analyze_form_text(input, 1)
        .unwrap()
        .occurrences
        .into_iter()
        .find(|item| {
            item.target.content_stream_index == slot
                && item.target.invocation_path.len() == if nested { 2 } else { 1 }
                && item.text.logical_text == "ABC"
        })
        .unwrap()
        .target
}
fn request(target: FormTextTarget, replacement: &str) -> FormTextEditRequest {
    FormTextEditRequest {
        edit: MultiRunTextRangeRequest {
            page: target.page,
            logical_start: 0,
            logical_end: 3,
            replacement_text: replacement.into(),
            mode: AdvancedTextMode::ParagraphReflowHorizontal,
            style_policy: MultiRunStylePolicy::InheritLeading,
            options: AdvancedTextEditOptions {
                region: [5.0, 5.0, 95.0, 75.0],
                font_size: 12.0,
                ..Default::default()
            },
            final_lines: None,
        },
        target,
        shared_form_policy: SharedFormEditPolicy::CloneEditOneInstance,
    }
}
fn object(input: &[u8], number: u32) -> PdfObject {
    ContentEngine::open_bytes(input.to_vec())
        .unwrap()
        .document()
        .reader()
        .get_object(number, 0)
        .unwrap()
}

#[test]
fn inherited_font_selection_is_object_bound_despite_child_name_shadowing() {
    let input = fixture(false, false, false);
    let inventory = analyze_form_text(&input, 1).unwrap();
    assert_eq!(inventory.occurrences.len(), 2);
    assert!(inventory
        .occurrences
        .iter()
        .all(|item| item.text.logical_text == "ABC"));
    let font = &inventory.occurrences[0].text.source_spans[0].font_resource;
    assert!(font.starts_with("WFInheritedFont"));
    assert_eq!(inventory.occurrences[0].form_bbox, [0.0, 0.0, 100.0, 80.0]);
}

#[test]
fn nested_form_text_clone_isolates_pages_and_repeated_contents_slots() {
    for nested in [false, true] {
        let input = fixture(nested, true, false);
        let target = selected(&input, 1, nested);
        let (output, report) = edit_form_text(&input, &request(target, "XYZ"), None).unwrap();
        assert!(output.starts_with(&input));
        assert!(report.source_form_retained);
        assert!(report.whole_direct_text_verified);
        assert_eq!(report.direct_text_before, "ABC");
        assert_eq!(report.direct_text_after, "XYZ");
        for number in [4, 5, 6, 7, 9] {
            assert_eq!(object(&input, number), object(&output, number));
        }
        if nested {
            assert_eq!(object(&input, 8), object(&output, 8));
        }
        let saved = ContentEngine::open_bytes(output.clone()).unwrap();
        assert_eq!(saved.document().get_page(1).unwrap().contents[0], (4, 0));
        assert_ne!(saved.document().get_page(1).unwrap().contents[1], (4, 0));
        assert_eq!(saved.document().get_page(2).unwrap().contents, vec![(4, 0)]);
        let inventory = analyze_form_text(&output, 1).unwrap();
        assert_eq!(
            inventory
                .occurrences
                .iter()
                .filter(|item| item.text.logical_text == "XYZ")
                .count(),
            1
        );
        assert!(analyze_form_text(&output, 2)
            .unwrap()
            .occurrences
            .iter()
            .filter(|item| !item.text.logical_text.is_empty())
            .all(|item| item.text.logical_text == "ABC"));
        let (again, second) =
            edit_form_text(&output, &request(report.target_after, "DEF"), None).unwrap();
        assert_eq!(second.direct_text_after, "DEF");
        assert!(again.starts_with(&output));
    }
}

#[test]
fn form_text_partial_deletion_retains_the_original_form() {
    let input = fixture(false, false, false);
    let mut edit = request(selected(&input, 0, false), "");
    edit.edit.logical_end = 1;
    let (output, report) = edit_form_text(&input, &edit, None).unwrap();
    assert_eq!(report.direct_text_after, "BC");
    assert!(report.whole_direct_text_verified);
    assert_eq!(object(&input, 5), object(&output, 5));
    assert!(report.native_edit.reachable_source_tokens_removed);
}

#[test]
fn separator_only_form_replacement_is_occurrence_bound_and_editable_again() {
    for nested in [false, true] {
        let input = fixture(nested, true, false);
        let edit = request(selected(&input, 1, nested), "\n\r\n");
        let (output, report) = edit_form_text(&input, &edit, None).unwrap();
        assert_eq!(report.direct_text_after, "\n\r\n");
        assert!(report.whole_direct_text_verified);
        assert_eq!(object(&output, 5), object(&input, 5));
        let inventory = analyze_form_text(&output, 1).unwrap();
        assert_eq!(
            inventory
                .occurrences
                .iter()
                .filter(|o| o.text.logical_text == "\n\r\n")
                .count(),
            1
        );
        let mut again = request(report.target_after, "DEF");
        again.edit.style_policy = MultiRunStylePolicy::ExplicitSupplied;
        again.edit.logical_end = 3;
        let (_, second) = edit_form_text(&output, &again, None).unwrap();
        assert_eq!(second.direct_text_after, "DEF");
        assert!(analyze_form_text(&output, 2)
            .unwrap()
            .occurrences
            .iter()
            .filter(|o| !o.text.logical_text.is_empty())
            .all(|o| o.text.logical_text == "ABC"));
    }
}

#[test]
fn generated_form_font_resources_stay_on_the_cloned_leaf() {
    let input = fixture(false, false, false);
    let mut edit = request(selected(&input, 0, false), "Unicode text");
    edit.edit.style_policy = MultiRunStylePolicy::ExplicitSupplied;
    let (output, report) = edit_form_text(&input, &edit, None).unwrap();
    assert_eq!(report.direct_text_after, "Unicode text");
    let leaf = stream_ref(report.target_after.invocation_path.last().unwrap());
    let saved = ContentEngine::open_bytes(output.clone()).unwrap();
    let PdfObject::Stream { dict, .. } = saved
        .document()
        .reader()
        .get_object(leaf.0, leaf.1)
        .unwrap()
    else {
        panic!()
    };
    let resources = dictionary(saved.document().reader(), dict.get("Resources"))
        .unwrap()
        .unwrap();
    let fonts = dictionary(saved.document().reader(), resources.get("Font"))
        .unwrap()
        .unwrap();
    assert!(fonts.iter().any(|(name, _)| name.starts_with("OxP20F")));
    let page_fonts = dictionary(
        saved.document().reader(),
        saved.document().get_page(1).unwrap().resources.get("Font"),
    )
    .unwrap()
    .unwrap();
    assert!(!page_fonts
        .iter()
        .any(|(name, _)| name.starts_with("OxP20F")));
    assert_eq!(object(&input, 5), object(&output, 5));
}

#[test]
fn stale_and_forged_form_targets_do_not_mutate() {
    let input = fixture(false, false, false);
    let target = selected(&input, 0, false);
    let (output, _) = edit_form_text(&input, &request(target.clone(), "XYZ"), None).unwrap();
    assert!(edit_form_text(&output, &request(target.clone(), "DEF"), None).is_err());
    let mut forged = target.clone();
    forged.invocation_path[0].form_object = 7;
    assert!(edit_form_text(&input, &request(forged, "DEF"), None).is_err());
    let mut refused = request(target, "DEF");
    refused.shared_form_policy = SharedFormEditPolicy::Reject;
    assert!(edit_form_text(&input, &refused, None).is_err());
}

#[test]
fn edit_all_requires_compatible_contexts_on_every_page() {
    let input = fixture(false, false, false);
    let mut edit = request(selected(&input, 0, false), "XYZ");
    edit.shared_form_policy = SharedFormEditPolicy::EditAllUses;
    let (output, report) = edit_form_text(&input, &edit, None).unwrap();
    assert!(!report.source_form_retained);
    for page in [1, 2] {
        assert!(analyze_form_text(&output, page)
            .unwrap()
            .occurrences
            .iter()
            .all(|item| item.text.logical_text == "XYZ"));
    }
    let input = fixture(false, false, true);
    let mut edit = request(selected(&input, 0, false), "XYZ");
    edit.shared_form_policy = SharedFormEditPolicy::EditAllUses;
    assert!(edit_form_text(&input, &edit, None)
        .unwrap_err()
        .to_string()
        .contains("incompatible inherited"));
}

#[test]
fn caller_actual_text_is_disclosed_and_requires_an_owner_transaction() {
    let input = fixture(false, false, false);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let input = write_incremental_update(
        engine.document().reader(),
        vec![IncrementalObject {
            number: 4,
            generation: 0,
            object: stream(
                PdfDictionary::empty(),
                b"BT /F1 12 Tf ET /Span << /ActualText (ABC) >> BDC /A Do EMC\n",
            ),
        }],
    )
    .unwrap();
    let inventory = analyze_form_text(&input, 1).unwrap();
    assert!(inventory.occurrences[0].external_actual_text_owner);
    assert!(edit_form_text(
        &input,
        &request(inventory.occurrences[0].target.clone(), "XYZ"),
        None
    )
    .unwrap_err()
    .to_string()
    .contains("caller ActualText"));
}

#[test]
fn form_scanner_preserves_native_offsets_and_parent_text_state() {
    let input = fixture(false, false, false);
    let engine = ContentEngine::open_bytes(input.clone()).unwrap();
    let scopes = discover_scopes(&engine, 1, "fixture").unwrap();
    assert_eq!(scopes.len(), 2);
    assert_eq!(scopes[0].initial.font_size, 12.0);
    assert_eq!(scopes[1].initial.font_size, 12.0);
    let content = match object(&input, 4) {
        PdfObject::Stream { raw, .. } => raw,
        _ => panic!(),
    };
    for scope in scopes {
        let step = &scope.target.path()[0];
        assert_eq!(
            &content[step.owner_operation_byte_start..step.owner_operation_byte_end],
            b"/A Do"
        );
    }
}

#[test]
fn resource_binding_reuses_aliases_and_rewrites_inherited_pattern_names() {
    let input = fixture(false, false, false);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let parent = dict(&[
        (
            "ColorSpace",
            PdfObject::Dictionary(dict(&[("C", PdfObject::Name("DeviceRGB".into()))])),
        ),
        ("Pattern", PdfObject::Dictionary(dict(&[("P", r(5))]))),
    ]);
    let mut child = dict(&[
        (
            "ColorSpace",
            PdfObject::Dictionary(dict(&[("C", PdfObject::Name("DeviceGray".into()))])),
        ),
        ("Pattern", PdfObject::Dictionary(dict(&[("P", r(4))]))),
    ]);
    let remapped = remap_paint(reader, &parent, &mut child, "/C cs /P scn").unwrap();
    assert!(remapped.contains("/WFInheritedColorSpace0 cs"));
    assert!(remapped.contains("/WFInheritedPattern0 scn"));
    assert_eq!(
        remap_paint(reader, &parent, &mut child, "/C cs /P scn").unwrap(),
        remapped
    );
}

#[test]
fn edit_all_detects_shared_annotation_appearance_programs() {
    let input = fixture(false, false, false);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let mut page = reader.get_object(3, 0).unwrap().as_dict().unwrap().clone();
    page.insert("Annots", PdfObject::Array(vec![r(10)]));
    let annotation = dict(&[
        ("Type", PdfObject::Name("Annot".into())),
        ("Subtype", PdfObject::Name("Stamp".into())),
        (
            "Rect",
            PdfObject::Array(
                vec![0, 0, 100, 80]
                    .into_iter()
                    .map(PdfObject::Integer)
                    .collect(),
            ),
        ),
        ("AP", PdfObject::Dictionary(dict(&[("N", r(5))]))),
    ]);
    let input = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: 3,
                generation: 0,
                object: PdfObject::Dictionary(page),
            },
            IncrementalObject {
                number: 10,
                generation: 0,
                object: PdfObject::Dictionary(annotation),
            },
        ],
    )
    .unwrap();
    let mut edit = request(selected(&input, 0, false), "XYZ");
    edit.shared_form_policy = SharedFormEditPolicy::EditAllUses;
    assert!(edit_form_text(&input, &edit, None)
        .unwrap_err()
        .to_string()
        .contains("non-page program"));
    edit.shared_form_policy = SharedFormEditPolicy::CloneEditOneInstance;
    let (output, _) = edit_form_text(&input, &edit, None).unwrap();
    assert_eq!(object(&input, 10), object(&output, 10));
    assert_eq!(object(&input, 5), object(&output, 5));
}

#[test]
fn sdk_form_text_json_round_trip_retains_native_target_binding() {
    let input = fixture(false, false, false);
    let inventory = crate::sdk::advanced_editing_form_text_analyze_json(&input, 1, None).unwrap();
    assert!(inventory.contains("advanced_editing_form_text_inventory"));
    let edit = request(selected(&input, 0, false), "XYZ");
    let (output, report) = crate::sdk::advanced_editing_form_text_edit_json(
        &input,
        &serde_json::to_string(&edit).unwrap(),
        None,
        None,
    )
    .unwrap();
    assert!(output.starts_with(&input));
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["kind"], "advanced_editing_form_text_edit_report");
    assert_eq!(report["report"]["direct_text_after"], "XYZ");
    assert_eq!(report["report"]["whole_direct_text_verified"], true);
    assert_eq!(report["report"]["source_form_retained"], true);
    assert_eq!(
        report["report"]["target_after"]["input_sha256"],
        format!("{:x}", Sha256::digest(&output))
    );
}

#[test]
fn omitted_nested_resources_use_the_page_not_the_parent_form_dictionary() {
    let input = fixture(true, false, false);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let PdfObject::Stream {
        dict: mut parent,
        raw,
    } = reader.get_object(8, 0).unwrap()
    else {
        panic!()
    };
    parent.insert(
        "Resources",
        PdfObject::Dictionary(dict(&[
            ("Font", PdfObject::Dictionary(dict(&[("F1", r(7))]))),
            ("XObject", PdfObject::Dictionary(dict(&[("Leaf", r(5))]))),
        ])),
    );
    let PdfObject::Stream { dict: mut leaf, .. } = reader.get_object(5, 0).unwrap() else {
        panic!()
    };
    leaf.remove("Resources");
    let input = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: 8,
                generation: 0,
                object: PdfObject::Stream { dict: parent, raw },
            },
            IncrementalObject {
                number: 5,
                generation: 0,
                object: stream(leaf, b"BT /F1 12 Tf 10 20 Td (ABC) Tj ET\n"),
            },
        ],
    )
    .unwrap();
    let target = selected(&input, 0, true);
    let (output, report) = edit_form_text(&input, &request(target, "XYZ"), None).unwrap();
    assert_eq!(report.direct_text_before, "ABC");
    assert_eq!(report.direct_text_after, "XYZ");
    assert_eq!(object(&input, 5), object(&output, 5));
    assert_eq!(object(&input, 8), object(&output, 8));
}

#[test]
fn inherited_device_colour_does_not_silently_adopt_a_different_default_space() {
    let input = fixture(false, false, false);
    let engine = ContentEngine::open_bytes(input).unwrap();
    let reader = engine.document().reader();
    let PdfObject::Stream {
        dict: mut leaf,
        raw,
    } = reader.get_object(5, 0).unwrap()
    else {
        panic!()
    };
    let mut resources = dictionary(reader, leaf.get("Resources")).unwrap().unwrap();
    resources.insert(
        "ColorSpace",
        PdfObject::Dictionary(dict(&[(
            "DefaultRGB",
            PdfObject::Array(vec![
                PdfObject::Name("CalRGB".into()),
                PdfObject::Dictionary(dict(&[(
                    "WhitePoint",
                    PdfObject::Array(vec![PdfObject::Integer(1); 3]),
                )])),
            ]),
        )])),
    );
    leaf.insert("Resources", PdfObject::Dictionary(resources));
    let input = write_incremental_update(
        reader,
        vec![
            IncrementalObject {
                number: 4,
                generation: 0,
                object: stream(PdfDictionary::empty(), b"BT /F1 12 Tf ET 1 0 0 rg /A Do\n"),
            },
            IncrementalObject {
                number: 5,
                generation: 0,
                object: PdfObject::Stream { dict: leaf, raw },
            },
        ],
    )
    .unwrap();
    let current = ContentEngine::open_bytes(input.clone()).unwrap();
    let scopes = discover_scopes(&current, 1, "fixture").unwrap();
    assert!(scopes[0].initial.unsupported_fill_paint_state);
    let edit = request(selected(&input, 0, false), "XYZ");
    assert!(edit_form_text(&input, &edit, None)
        .unwrap_err()
        .to_string()
        .contains("source paint command"));
}
