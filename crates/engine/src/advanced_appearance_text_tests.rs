//! Unexecuted regression source. No builds, tests, PDFs or rendering were run.
use super::*;
use crate::writer::{OutputObject, PdfWriter};

#[path = "universal_scoped_appearance_tests.rs"]
mod governed;
#[path = "advanced_appearance_tag_tests.rs"]
mod tagged;
#[path = "advanced_widget_text_tests.rs"]
mod widgets;

fn r(number: u32) -> PdfObject {
    reference((number, 0))
}
fn n(value: &str) -> PdfObject {
    PdfObject::Name(value.into())
}
fn d(items: &[(&str, PdfObject)]) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    for (key, value) in items {
        dict.insert(*key, value.clone());
    }
    dict
}
fn array(values: &[i64]) -> PdfObject {
    PdfObject::Array(values.iter().copied().map(PdfObject::Integer).collect())
}
fn form(text: &[u8], nested_resources: bool) -> PdfObject {
    let mut resources = d(&[("Font", PdfObject::Dictionary(d(&[("F1", r(6))])))]);
    if nested_resources {
        resources.insert("XObject", PdfObject::Dictionary(d(&[("Leaf", r(5))])));
    }
    PdfObject::Stream {
        dict: d(&[
            ("Type", n("XObject")),
            ("Subtype", n("Form")),
            ("BBox", array(&[0, 0, 100, 80])),
            ("Matrix", array(&[0, 1, -1, 0, 80, 0])),
            (
                "Group",
                PdfObject::Dictionary(d(&[
                    ("S", n("Transparency")),
                    ("I", PdfObject::Boolean(true)),
                ])),
            ),
            ("Resources", PdfObject::Dictionary(resources)),
            ("Length", PdfObject::Integer(text.len() as i64)),
        ]),
        raw: text.to_vec(),
    }
}
fn fixture(nested: bool) -> Vec<u8> {
    let resources =
        PdfObject::Dictionary(d(&[("Font", PdfObject::Dictionary(d(&[("F1", r(6))])))]));
    let annotation = |x| {
        PdfObject::Dictionary(d(&[
            ("Type", n("Annot")),
            ("Subtype", n("Stamp")),
            ("Rect", array(&[x, 0, x + 100, 80])),
            ("AP", r(8)),
            ("AS", n("Yes")),
            (
                "Contents",
                crate::annotation_identity::text_string("comment"),
            ),
        ]))
    };
    let page_text = b"BT /F1 10 Tf (PAGE) Tj ET";
    let objects = vec![
        PdfObject::Dictionary(d(&[("Type", n("Catalog")), ("Pages", r(2))])),
        PdfObject::Dictionary(d(&[
            ("Type", n("Pages")),
            ("Count", PdfObject::Integer(1)),
            ("Kids", PdfObject::Array(vec![r(3)])),
        ])),
        PdfObject::Dictionary(d(&[
            ("Type", n("Page")),
            ("Parent", r(2)),
            ("MediaBox", array(&[0, 0, 300, 200])),
            ("Contents", r(4)),
            ("Resources", resources),
            ("Annots", PdfObject::Array(vec![r(7), r(9)])),
        ])),
        PdfObject::Stream {
            dict: d(&[("Length", PdfObject::Integer(page_text.len() as i64))]),
            raw: page_text.to_vec(),
        },
        form(
            b"/Span << /ActualText (ABC) >> BDC BT /F1 12 Tf 1 0 0 1 5 20 Tm (ABC) Tj ET EMC",
            false,
        ),
        PdfObject::Dictionary(d(&[
            ("Type", n("Font")),
            ("Subtype", n("Type1")),
            ("BaseFont", n("Courier")),
            ("Encoding", n("WinAnsiEncoding")),
        ])),
        annotation(0),
        PdfObject::Dictionary(d(&[
            (
                "N",
                PdfObject::Dictionary(d(&[
                    ("Yes", r(if nested { 10 } else { 5 })),
                    ("Off", r(11)),
                ])),
            ),
            ("R", r(11)),
            ("D", r(11)),
            ("WFKeep", n("unchanged")),
        ])),
        annotation(150),
        form(b"/Leaf Do q 1 0 0 1 20 0 cm /Leaf Do Q", true),
        form(b"BT /F1 12 Tf (OFF) Tj ET", false),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, object)| OutputObject {
        number: index as u32 + 1,
        object,
    })
    .collect();
    PdfWriter::new(objects, 1).write().unwrap()
}
fn object(bytes: &[u8], number: u32) -> PdfObject {
    ContentEngine::open_bytes(bytes.to_vec())
        .unwrap()
        .document()
        .reader()
        .get_object(number, 0)
        .unwrap()
}
fn modify(bytes: &[u8], number: u32, value: PdfObject) -> Vec<u8> {
    let engine = ContentEngine::open_bytes(bytes.to_vec()).unwrap();
    write_incremental_update(
        engine.document().reader(),
        vec![IncrementalObject {
            number,
            generation: 0,
            object: value,
        }],
    )
    .unwrap()
}
fn selected(bytes: &[u8], nested: bool) -> AppearanceTextTarget {
    analyze_appearance_text(bytes, 1)
        .unwrap()
        .occurrences
        .into_iter()
        .find(|source| {
            source.target.annotation == (7, 0)
                && source.target.invocation_path.len() == usize::from(nested)
                && source.text.logical_text == "ABC"
        })
        .unwrap()
        .target
}
fn request(target: AppearanceTextTarget, text: &str) -> AppearanceTextEditRequest {
    AppearanceTextEditRequest {
        edit: MultiRunTextRangeRequest {
            page: target.page,
            logical_start: 0,
            logical_end: 3,
            replacement_text: text.into(),
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
        metadata_policy: AppearanceMetadataPolicy::PreserveAnnotationMetadata,
        tagged_clone: Default::default(),
    }
}

#[test]
fn selected_appearance_clone_preserves_page_other_annotations_and_other_states() {
    for nested in [false, true] {
        let input = fixture(nested);
        let (output, report) =
            edit_appearance_text(&input, &request(selected(&input, nested), "XYZ"), None).unwrap();
        assert!(output.starts_with(&input));
        assert_eq!(report.direct_text_after, "XYZ");
        assert!(report.whole_direct_text_verified);
        assert!(report.source_programs_retained);
        for number in [1, 2, 3, 4, 5, 6, 8, 9, 10, 11] {
            assert_eq!(
                object(&input, number),
                object(&output, number),
                "object {number}"
            );
        }
        let new_root = object(&output, report.target_after.appearance_stream.0);
        let original_root = object(&input, if nested { 10 } else { 5 });
        for key in ["Matrix", "BBox", "Group"] {
            assert_eq!(
                new_root.as_stream().unwrap().0.get(key),
                original_root.as_stream().unwrap().0.get(key)
            );
        }
        let owner = object(&output, 7);
        let ap = owner
            .as_dict()
            .unwrap()
            .get("AP")
            .unwrap()
            .as_dict()
            .unwrap();
        assert_eq!(ap.get("R"), Some(&r(11)));
        assert_eq!(ap.get("D"), Some(&r(11)));
        assert_eq!(ap.get("WFKeep"), Some(&n("unchanged")));
        assert_eq!(
            ap.get("N").unwrap().as_dict().unwrap().get("Off"),
            Some(&r(11))
        );
        assert_eq!(
            owner.as_dict().unwrap().get("Contents"),
            Some(&crate::annotation_identity::text_string("comment"))
        );
    }
}

#[test]
fn nested_repeated_leaf_edits_only_one_do_occurrence() {
    let input = fixture(true);
    let (output, _) =
        edit_appearance_text(&input, &request(selected(&input, true), "XYZ"), None).unwrap();
    let text = analyze_appearance_text(&output, 1)
        .unwrap()
        .occurrences
        .into_iter()
        .filter(|item| !item.target.invocation_path.is_empty())
        .map(|item| item.text.logical_text)
        .collect::<Vec<_>>();
    assert_eq!(text, vec!["XYZ", "ABC", "ABC", "ABC"]);
}

#[test]
fn separator_only_appearance_edit_preserves_other_occurrences_and_reopens() {
    for nested in [false, true] {
        let input = fixture(nested);
        let (output, report) =
            edit_appearance_text(&input, &request(selected(&input, nested), "\r\n"), None).unwrap();
        assert_eq!(report.direct_text_after, "\r\n");
        assert!(report.whole_direct_text_verified);
        assert_eq!(object(&output, 9), object(&input, 9));
        assert_eq!(object(&output, 5), object(&input, 5));
        let mut again = request(report.target_after, "DEF");
        again.edit.logical_end = 2;
        again.edit.style_policy = MultiRunStylePolicy::ExplicitSupplied;
        let (_, second) = edit_appearance_text(&output, &again, None).unwrap();
        assert_eq!(second.direct_text_after, "DEF");
    }
}

#[test]
fn saved_appearance_target_can_be_used_for_a_second_edit() {
    let input = fixture(false);
    let (first, report) =
        edit_appearance_text(&input, &request(selected(&input, false), "XYZ"), None).unwrap();
    assert_eq!(
        report.target_after.input_sha256,
        format!("{:x}", Sha256::digest(&first))
    );
    let (second, report) =
        edit_appearance_text(&first, &request(report.target_after, "DEF"), None).unwrap();
    assert_eq!(report.direct_text_after, "DEF");
    assert!(report.whole_direct_text_verified);
    assert_eq!(
        analyze_appearance_text(&second, 1).unwrap().occurrences[1]
            .text
            .logical_text,
        "ABC"
    );
}

#[test]
fn stale_or_forged_appearance_targets_are_not_applied() {
    let input = fixture(false);
    for change in 0..5 {
        let mut target = selected(&input, false);
        match change {
            0 => target.input_sha256.clear(),
            1 => target.annotation_index = 1,
            2 => target.annotation = (9, 0),
            3 => target.appearance_stream = (11, 0),
            _ => target.normal_state = Some("Off".into()),
        }
        assert!(edit_appearance_text(&input, &request(target, "XYZ"), None).is_err());
    }
}

#[test]
fn partial_appearance_deletion_preserves_following_text_and_logical_mapping() {
    let input = fixture(false);
    let mut edit = request(selected(&input, false), "");
    edit.edit.logical_start = 1;
    edit.edit.logical_end = 2;
    let (output, report) = edit_appearance_text(&input, &edit, None).unwrap();
    assert_eq!(report.direct_text_after, "AC");
    let result = analyze_appearance_text(&output, 1).unwrap();
    assert_eq!(result.occurrences[0].text.logical_text, "AC");
    assert_eq!(result.occurrences[1].text.logical_text, "ABC");
}

#[test]
fn direct_normal_stream_slot_is_rebound_without_inventing_state_dictionary() {
    let mut ap = object(&fixture(false), 8).as_dict().unwrap().clone();
    ap.insert("N", r(5));
    let input = modify(&fixture(false), 8, PdfObject::Dictionary(ap));
    let target = selected(&input, false);
    assert_eq!(target.normal_state, None);
    let (output, report) = edit_appearance_text(&input, &request(target, "XYZ"), None).unwrap();
    let object = object(&output, 7);
    let n = object
        .as_dict()
        .unwrap()
        .get("AP")
        .unwrap()
        .as_dict()
        .unwrap()
        .get("N")
        .unwrap();
    assert_eq!(
        n.as_reference(),
        Some(report.target_after.appearance_stream)
    );
}

#[test]
fn free_text_plain_text_policy_synchronizes_contents_and_explicitly_discards_rich_text() {
    let input = fixture(false);
    let mut owner = object(&input, 7).as_dict().unwrap().clone();
    owner.insert("Subtype", n("FreeText"));
    owner.insert("Contents", crate::annotation_identity::text_string("ABC"));
    owner.insert(
        "RC",
        crate::annotation_identity::text_string("<body><p>ABC</p></body>"),
    );
    let input = modify(&input, 7, PdfObject::Dictionary(owner));
    let mut edit = request(selected(&input, false), "XYZ");
    edit.metadata_policy = AppearanceMetadataPolicy::SynchronizeFreeTextPlainText;
    let (output, report) = edit_appearance_text(&input, &edit, None).unwrap();
    assert!(report.annotation_contents_updated);
    assert!(report.rich_text_discarded);
    let owner = object(&output, 7);
    assert_eq!(
        owner.as_dict().unwrap().get("Contents"),
        Some(&crate::annotation_identity::text_string("XYZ"))
    );
    assert!(owner.as_dict().unwrap().get("RC").is_none());
}

#[test]
fn free_text_metadata_mismatch_is_not_silently_overwritten() {
    let input = fixture(false);
    let mut owner = object(&input, 7).as_dict().unwrap().clone();
    owner.insert("Subtype", n("FreeText"));
    let input = modify(&input, 7, PdfObject::Dictionary(owner));
    let mut edit = request(selected(&input, false), "XYZ");
    edit.metadata_policy = AppearanceMetadataPolicy::SynchronizeFreeTextPlainText;
    assert!(edit_appearance_text(&input, &edit, None)
        .unwrap_err()
        .to_string()
        .contains("exact direct-text mapping"));
}

#[test]
fn widget_values_and_stream_owned_tags_are_not_bypassed_by_appearance_cloning() {
    let input = fixture(false);
    let mut owner = object(&input, 7).as_dict().unwrap().clone();
    owner.insert("Subtype", n("Widget"));
    let widget = modify(&input, 7, PdfObject::Dictionary(owner));
    assert!(
        edit_appearance_text(&widget, &request(selected(&widget, false), "XYZ"), None)
            .unwrap_err()
            .to_string()
            .contains("field value")
    );
    let PdfObject::Stream { mut dict, raw } = object(&input, 5) else {
        panic!()
    };
    dict.insert("StructParents", PdfObject::Integer(0));
    let tagged = modify(&input, 5, PdfObject::Stream { dict, raw });
    assert!(
        edit_appearance_text(&tagged, &request(selected(&tagged, false), "XYZ"), None)
            .unwrap_err()
            .to_string()
            .contains("stream-owned structure")
    );
}

#[test]
fn duplicate_annotation_page_ownership_requires_normalization_not_global_mutation() {
    let input = fixture(false);
    let mut page = object(&input, 3).as_dict().unwrap().clone();
    page.insert("Annots", PdfObject::Array(vec![r(7), r(7)]));
    let input = modify(&input, 3, PdfObject::Dictionary(page));
    assert!(
        edit_appearance_text(&input, &request(selected(&input, false), "XYZ"), None)
            .unwrap_err()
            .to_string()
            .contains("duplicate page ownership")
    );
}

#[test]
fn appearance_json_facade_returns_saved_revision_and_local_source_report() {
    let input = fixture(false);
    let inventory =
        crate::sdk::advanced_editing_appearance_text_analyze_json(&input, 1, None).unwrap();
    let inventory: serde_json::Value = serde_json::from_str(&inventory).unwrap();
    assert_eq!(
        inventory["kind"],
        "advanced_editing_appearance_text_inventory"
    );
    let request = serde_json::to_string(&request(selected(&input, false), "XYZ")).unwrap();
    let (output, report) =
        crate::sdk::advanced_editing_appearance_text_edit_json(&input, &request, None, None)
            .unwrap();
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["report"]["direct_text_after"], "XYZ");
    assert_eq!(
        report["report"]["target_after"]["input_sha256"],
        format!("{:x}", Sha256::digest(&output))
    );
}

#[test]
fn generated_font_resources_belong_to_the_cloned_appearance_not_the_page() {
    let input = fixture(false);
    let mut edit = request(selected(&input, false), "Unicode text");
    edit.edit.style_policy = MultiRunStylePolicy::ExplicitSupplied;
    let (output, report) = edit_appearance_text(&input, &edit, None).unwrap();
    assert_eq!(report.direct_text_after, "Unicode text");
    assert_eq!(object(&input, 3), object(&output, 3));
    let leaf = object(&output, report.target_after.appearance_stream.0);
    let resources = leaf
        .as_stream()
        .unwrap()
        .0
        .get("Resources")
        .unwrap()
        .as_dict()
        .unwrap();
    assert!(
        resources
            .get("Font")
            .unwrap()
            .as_dict()
            .unwrap()
            .iter()
            .count()
            > 1
    );
}

#[test]
fn cancelled_appearance_edit_does_not_return_partial_bytes() {
    let input = fixture(false);
    let edit = request(selected(&input, false), "XYZ");
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    assert!(matches!(
        token.scope(|| edit_appearance_text(&input, &edit, None)),
        Err(WellfriendError::Cancelled(_))
    ));
}
