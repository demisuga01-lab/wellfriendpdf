//! Unexecuted field-wide source regressions; the existing AP fixture supplies
//! shared programs and extraction, not a substitute mocked writer.
use super::*;
use crate::advanced_editing::form_text::appearance::widgets::*;
use crate::universal_editing::scoped_text::{ScopedTextEditRequest, ScopedTextSource};
use crate::universal_editing::*;

fn field_fixture(two_pages: bool) -> Vec<u8> {
    let input = fixture(false);
    let mut catalog = object(&input, 1).as_dict().unwrap().clone();
    catalog.insert(
        "AcroForm",
        PdfObject::Dictionary(d(&[
            ("Fields", PdfObject::Array(vec![r(12)])),
            ("NeedAppearances", PdfObject::Boolean(false)),
            (
                "DR",
                PdfObject::Dictionary(d(&[("Font", PdfObject::Dictionary(d(&[("F1", r(6))])))])),
            ),
            ("DA", PdfObject::String(b"/F1 12 Tf 0 g".to_vec())),
        ])),
    );
    let field = PdfObject::Dictionary(d(&[
        ("FT", n("Tx")),
        ("T", crate::annotation_identity::text_string("name")),
        ("V", crate::annotation_identity::text_string("ABC")),
        ("DV", crate::annotation_identity::text_string("RESET")),
        ("Kids", PdfObject::Array(vec![r(7), r(9)])),
    ]));
    let mut updates = vec![(1, PdfObject::Dictionary(catalog)), (12, field)];
    for id in [7, 9] {
        let mut widget = object(&input, id).as_dict().unwrap().clone();
        widget.insert("Subtype", n("Widget"));
        widget.insert("Parent", r(12));
        widget.remove("AS");
        widget.insert("AP", PdfObject::Dictionary(d(&[("N", r(5))])));
        widget.insert("P", r(if two_pages && id == 9 { 13 } else { 3 }));
        updates.push((id, PdfObject::Dictionary(widget)));
    }
    if two_pages {
        let mut pages = object(&input, 2).as_dict().unwrap().clone();
        pages.insert("Kids", PdfObject::Array(vec![r(3), r(13)]));
        pages.insert("Count", PdfObject::Integer(2));
        let mut first = object(&input, 3).as_dict().unwrap().clone();
        first.insert("Annots", PdfObject::Array(vec![r(7)]));
        let mut second = first.clone();
        second.insert("Annots", PdfObject::Array(vec![r(9)]));
        updates.extend([
            (2, PdfObject::Dictionary(pages)),
            (3, PdfObject::Dictionary(first)),
            (13, PdfObject::Dictionary(second)),
        ]);
    }
    let engine = ContentEngine::open_bytes(input).unwrap();
    write_incremental_update(
        engine.document().reader(),
        updates
            .into_iter()
            .map(|(number, object)| IncrementalObject {
                number,
                generation: 0,
                object,
            })
            .collect(),
    )
    .unwrap()
}
fn field_request(input: &[u8]) -> WidgetTextEditRequest {
    let field = discover_widget_fields(input, &[1]).unwrap().remove(0);
    let engine = ContentEngine::open_bytes(input.to_vec()).unwrap();
    let widgets = (1..=engine.page_count().unwrap())
        .flat_map(|page| analyze_appearance_text(input, page).unwrap().occurrences)
        .filter(|item| [7, 9].contains(&item.target.annotation.0))
        .map(|item| WidgetAppearanceEdit {
            appearance: request(item.target, "XYZ"),
            expected_display: "ABC".into(),
            replacement_display: "XYZ".into(),
        })
        .collect();
    WidgetTextEditRequest {
        target: field.target,
        expected_value: "ABC".into(),
        replacement_value: "XYZ".into(),
        widgets,
        default_appearance: WidgetDefaultAppearance::PreserveSourceDefaults,
        update_default_value: false,
        discard_rich_text: false,
        allow_read_only: false,
        preserve_actions_without_execution: false,
    }
}
fn value(input: &[u8], id: u32, key: &str) -> String {
    let object = object(input, id);
    let value = object.as_dict().unwrap().get(key).unwrap();
    let PdfObject::String(bytes) = value else {
        panic!("text string expected")
    };
    crate::info::decode_pdf_text_string(bytes)
}

#[test]
fn two_page_shared_widgets_and_value_update_together_without_changing_reset_value() {
    let input = field_fixture(true);
    let request = field_request(&input);
    let (output, report) = edit_widget_text(&input, &request, None).unwrap();
    assert!(output.starts_with(&input));
    assert_eq!(value(&output, 12, "V"), "XYZ");
    assert_eq!(value(&output, 12, "DV"), "RESET");
    assert_eq!(report.affected_pages, vec![1, 2]);
    assert!(report.all_widget_displays_verified);
    assert!(report.single_published_revision);
    let original = ContentEngine::open_bytes(input.clone()).unwrap();
    let saved = ContentEngine::open_bytes(output.clone()).unwrap();
    assert_eq!(
        saved.document().reader().trailer().get_integer("Prev"),
        Some(original.document().reader().startxref_offset() as i64)
    );
    for page in [1, 2] {
        assert!(analyze_appearance_text(&output, page)
            .unwrap()
            .occurrences
            .iter()
            .all(|item| item.text.logical_text == "XYZ"));
    }
    for id in [1, 2, 3, 4, 5, 6, 8, 13] {
        assert_eq!(object(&input, id), object(&output, id));
    }
    assert!(report
        .widgets
        .iter()
        .all(|r| r.target_after.input_sha256 == format!("{:x}", Sha256::digest(&output))));
    let mut again = field_request(&input);
    again.target = report.target_after.clone();
    again.expected_value = "XYZ".into();
    again.replacement_value = "DEF".into();
    for (widget, report) in again.widgets.iter_mut().zip(report.widgets) {
        widget.appearance.target = report.target_after;
        widget.appearance.edit.replacement_text = "DEF".into();
        widget.expected_display = "XYZ".into();
        widget.replacement_display = "DEF".into();
    }
    let (second, _) = edit_widget_text(&output, &again, None).unwrap();
    assert_eq!(value(&second, 12, "V"), "DEF");
}

#[test]
fn missing_duplicate_stale_and_wrong_display_widgets_refuse_before_publication() {
    let input = field_fixture(false);
    let request = field_request(&input);
    let mut broken = request.clone();
    broken.widgets.pop();
    assert!(edit_widget_text(&input, &broken, None).is_err());
    broken = request.clone();
    broken.widgets[1] = broken.widgets[0].clone();
    assert!(edit_widget_text(&input, &broken, None).is_err());
    broken = request.clone();
    broken.widgets[1].expected_display = "other".into();
    assert!(edit_widget_text(&input, &broken, None).is_err());
    broken = request.clone();
    broken.widgets[1].replacement_display = "incorrect".into();
    assert!(edit_widget_text(&input, &broken, None).is_err());
    broken = request.clone();
    broken.expected_value = "old".into();
    assert!(edit_widget_text(&input, &broken, None).is_err());
    broken = request;
    broken.target.input_sha256 = "stale".into();
    assert!(edit_widget_text(&input, &broken, None).is_err());
    assert_eq!(value(&input, 12, "V"), "ABC");
}

#[test]
fn explicit_default_font_binding_updates_dr_and_removes_widget_da_overrides() {
    let input = field_fixture(false);
    let mut widget = object(&input, 7).as_dict().unwrap().clone();
    widget.insert("DA", PdfObject::String(b"/Old 9 Tf".to_vec()));
    let input = modify(&input, 7, PdfObject::Dictionary(widget));
    let mut request = field_request(&input);
    request.default_appearance = WidgetDefaultAppearance::FromEditedAppearance {
        widget: (7, 0),
        font_resource: "F1".into(),
        font_size: 13.0,
        rgb: [0.2, 0.3, 0.4],
    };
    request.update_default_value = true;
    let (output, report) = edit_widget_text(&input, &request, None).unwrap();
    assert_eq!(value(&output, 12, "DV"), "XYZ");
    assert!(value(&output, 12, "DA").starts_with("/WFField12_0_0 13 Tf"));
    assert!(object(&output, 7).as_dict().unwrap().get("DA").is_none());
    let engine = ContentEngine::open_bytes(output).unwrap();
    let reader = engine.document().reader();
    let form = reader
        .resolve(
            engine
                .document()
                .get_catalog()
                .unwrap()
                .get("AcroForm")
                .unwrap()
                .clone(),
        )
        .unwrap();
    let dr = reader
        .resolve(form.as_dict().unwrap().get("DR").unwrap().clone())
        .unwrap();
    let fonts = reader
        .resolve(dr.as_dict().unwrap().get("Font").unwrap().clone())
        .unwrap();
    assert_eq!(
        fonts.as_dict().unwrap().get_reference("WFField12_0_0"),
        Some((6, 0))
    );
    assert_eq!(
        report.default_appearance["policy"],
        "from_edited_appearance"
    );
}

#[test]
fn inherited_value_is_shadowed_only_on_the_selected_terminal_field() {
    let input = field_fixture(false);
    let mut field = object(&input, 12).as_dict().unwrap().clone();
    field.remove("FT");
    field.remove("V");
    field.insert("Parent", r(13));
    let mut catalog = object(&input, 1).as_dict().unwrap().clone();
    let mut form = catalog.get("AcroForm").unwrap().as_dict().unwrap().clone();
    form.insert("Fields", PdfObject::Array(vec![r(13)]));
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let parent = PdfObject::Dictionary(d(&[
        ("FT", n("Tx")),
        ("V", crate::annotation_identity::text_string("ABC")),
        ("Kids", PdfObject::Array(vec![r(12)])),
    ]));
    let engine = ContentEngine::open_bytes(input).unwrap();
    let input = write_incremental_update(
        engine.document().reader(),
        vec![
            (1, PdfObject::Dictionary(catalog)),
            (12, PdfObject::Dictionary(field)),
            (13, parent),
        ]
        .into_iter()
        .map(|(number, object)| IncrementalObject {
            number,
            generation: 0,
            object,
        })
        .collect(),
    )
    .unwrap();
    let (output, _) = edit_widget_text(&input, &field_request(&input), None).unwrap();
    assert_eq!(value(&output, 12, "V"), "XYZ");
    assert_eq!(value(&output, 13, "V"), "ABC");
    assert_eq!(object(&input, 13), object(&output, 13));
}

#[test]
fn rich_text_read_only_and_actions_need_separate_explicit_decisions() {
    let input = field_fixture(false);
    let mut field = object(&input, 12).as_dict().unwrap().clone();
    field.insert("Ff", PdfObject::Integer(1 | (1 << 25)));
    field.insert(
        "RV",
        crate::annotation_identity::text_string("<body>ABC</body>"),
    );
    field.insert(
        "AA",
        PdfObject::Dictionary(d(&[(
            "V",
            PdfObject::Dictionary(d(&[
                ("S", n("JavaScript")),
                ("JS", PdfObject::String(b"unchanged".to_vec())),
            ])),
        )])),
    );
    let input = modify(&input, 12, PdfObject::Dictionary(field));
    let mut request = field_request(&input);
    assert!(edit_widget_text(&input, &request, None).is_err());
    request.allow_read_only = true;
    assert!(edit_widget_text(&input, &request, None).is_err());
    request.discard_rich_text = true;
    assert!(edit_widget_text(&input, &request, None).is_err());
    request.preserve_actions_without_execution = true;
    let (output, report) = edit_widget_text(&input, &request, None).unwrap();
    assert!(report.actions_preserved_without_execution);
    let field = object(&output, 12);
    assert!(field.as_dict().unwrap().get("RV").is_none());
    assert_eq!(field.as_dict().unwrap().get_integer("Ff"), Some(1));
    assert_eq!(
        field.as_dict().unwrap().get("AA"),
        object(&input, 12).as_dict().unwrap().get("AA")
    );
}

#[test]
fn coordinated_widget_edit_enters_governed_preview_approval_and_apply() {
    let input = field_fixture(true);
    let request = UniversalEditRequestV2 {
        operation: UniversalEditOperationV2::ScopedText {
            request: ScopedTextEditRequest {
                source: ScopedTextSource::WidgetField {
                    request: field_request(&input),
                },
                approved_font_asset: None,
                planned_output_sha256: None,
            },
        },
        policy: UniversalEditPolicyV2::default(),
    };
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    assert_eq!(plan.state, UniversalPlanStateV2::ApprovalRequired);
    assert_eq!(plan.candidates[0].kind, "scoped_text_field");
    let preview = crate::universal_editing::scoped_preview::preview_scoped_candidate(
        &input,
        &plan,
        &crate::universal_editing::scoped_preview::ScopedPreviewOptions {
            pages: vec![1, 2],
            dpi: 72,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(preview.pages.len(), 2);
    assert!(preview
        .pages
        .iter()
        .all(|page| page.difference.changed_pixels > 0));
    assert!(apply_universal_edit_v2(&input, &plan, None).is_err());
    let approval = create_universal_approval_token_v2(
        &plan,
        UniversalApprovalDecisionV2 {
            selected_candidate_ids: plan.selected_candidate_ids.clone(),
            approved_font: plan.preview["font"]["required_approved_font"]
                .as_str()
                .map(str::to_owned),
            mutation_mode: plan.policy.mutation_mode,
            accept_visual_change: true,
            accept_signature_invalidation: false,
        },
    )
    .unwrap();
    let (output, report) = apply_universal_edit_v2(&input, &plan, Some(&approval)).unwrap();
    assert!(report.changed);
    assert_eq!(report.affected_pages, vec![1, 2]);
    assert_eq!(value(&output, 12, "V"), "XYZ");
    assert_eq!(
        preview.candidate_output_sha256,
        format!("{:x}", Sha256::digest(&output))
    );
    assert_eq!(
        plan.preview["output_sha256"],
        format!("{:x}", Sha256::digest(&output))
    );
}

#[test]
fn merged_widget_field_keeps_the_new_value_when_rebinding_its_appearance() {
    let input = field_fixture(false);
    let mut widget = object(&input, 7).as_dict().unwrap().clone();
    widget.remove("Parent");
    let field = object(&input, 12);
    for key in ["FT", "T", "V", "DV"] {
        widget.insert(key, field.as_dict().unwrap().get(key).unwrap().clone());
    }
    let mut catalog = object(&input, 1).as_dict().unwrap().clone();
    let mut form = catalog.get("AcroForm").unwrap().as_dict().unwrap().clone();
    form.insert("Fields", PdfObject::Array(vec![r(7)]));
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let mut page = object(&input, 3).as_dict().unwrap().clone();
    page.insert("Annots", PdfObject::Array(vec![r(7)]));
    let engine = ContentEngine::open_bytes(input).unwrap();
    let input = write_incremental_update(
        engine.document().reader(),
        vec![(1, catalog), (3, page), (7, widget)]
            .into_iter()
            .map(|(number, dict)| IncrementalObject {
                number,
                generation: 0,
                object: PdfObject::Dictionary(dict),
            })
            .collect(),
    )
    .unwrap();
    let request = field_request(&input);
    assert_eq!(request.target.field, (7, 0));
    assert_eq!(request.widgets.len(), 1);
    let (output, report) = edit_widget_text(&input, &request, None).unwrap();
    assert_eq!(value(&output, 7, "V"), "XYZ");
    assert_eq!(value(&output, 7, "DV"), "RESET");
    assert!(report.field_ownership_verified);
    assert_eq!(object(&input, 12), object(&output, 12));
}

#[test]
fn maximum_length_and_global_regeneration_flags_are_not_silently_ignored() {
    let original = field_fixture(false);
    for maximum in [0, 2] {
        let mut field = object(&original, 12).as_dict().unwrap().clone();
        field.insert("MaxLen", PdfObject::Integer(maximum));
        let input = modify(&original, 12, PdfObject::Dictionary(field));
        assert!(edit_widget_text(&input, &field_request(&input), None).is_err());
    }
    let mut catalog = object(&original, 1).as_dict().unwrap().clone();
    let mut form = catalog.get("AcroForm").unwrap().as_dict().unwrap().clone();
    form.insert("NeedAppearances", PdfObject::Boolean(true));
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let input = modify(&original, 1, PdfObject::Dictionary(catalog));
    assert!(edit_widget_text(&input, &field_request(&input), None).is_err());
}

#[test]
fn invalid_sibling_display_keeps_the_entire_transaction_private() {
    let input = field_fixture(true);
    let mut request = field_request(&input);
    request.widgets[1].appearance.edit.logical_end = 99;
    assert!(edit_widget_text(&input, &request, None).is_err());
    assert_eq!(value(&input, 12, "V"), "ABC");
    for page in [1, 2] {
        assert!(analyze_appearance_text(&input, page)
            .unwrap()
            .occurrences
            .iter()
            .all(|item| item.text.logical_text == "ABC"));
    }
}

#[test]
fn shared_structural_actual_text_rebases_between_widgets_and_survives_default_catalog_update() {
    use crate::tagged_structure::stream_clones::{StructureActualTextUpdate, TaggedClonePolicy};
    let input = field_fixture(false);
    let mut catalog = object(&input, 1).as_dict().unwrap().clone();
    catalog.insert("StructTreeRoot", r(13));
    let root = PdfObject::Dictionary(d(&[("Type", n("StructTreeRoot")), ("K", r(14))]));
    let mut pages = object(&input, 2).as_dict().unwrap().clone();
    pages.insert("Kids", PdfObject::Array(vec![r(3), r(15)]));
    pages.insert("Count", PdfObject::Integer(2));
    let mut other_page = object(&input, 3).as_dict().unwrap().clone();
    other_page.remove("Annots");
    let owner = PdfObject::Dictionary(d(&[
        ("Type", n("StructElem")),
        ("S", n("Form")),
        ("P", r(13)),
        (
            "ActualText",
            crate::annotation_identity::text_string("ABC ABC"),
        ),
        (
            "K",
            PdfObject::Array(
                [7, 9]
                    .into_iter()
                    .map(|id| {
                        PdfObject::Dictionary(d(&[
                            ("Type", n("OBJR")),
                            ("Obj", r(id)),
                            ("Pg", r(3)),
                        ]))
                    })
                    .collect(),
            ),
        ),
    ]));
    let engine = ContentEngine::open_bytes(input).unwrap();
    let input = write_incremental_update(
        engine.document().reader(),
        vec![
            (1, PdfObject::Dictionary(catalog)),
            (2, PdfObject::Dictionary(pages)),
            (13, root),
            (14, owner),
            (15, PdfObject::Dictionary(other_page)),
        ]
        .into_iter()
        .map(|(number, object)| IncrementalObject {
            number,
            generation: 0,
            object,
        })
        .collect(),
    )
    .unwrap();
    let input = crate::tagged_structure::rebuild_parent_tree(&input, "en")
        .unwrap()
        .0;
    let mut request = field_request(&input);
    for widget in &mut request.widgets {
        widget.appearance.tagged_clone.policy = TaggedClonePolicy::MoveExclusiveNamespaces;
        widget
            .appearance
            .tagged_clone
            .actual_text_updates
            .push(StructureActualTextUpdate {
                element: (14, 0),
                expected_text: "ABC ABC".into(),
                replacement_text: "XYZ XYZ".into(),
            });
    }
    request.default_appearance = WidgetDefaultAppearance::FromEditedAppearance {
        widget: (7, 0),
        font_resource: "F1".into(),
        font_size: 0.0,
        rgb: [0.0, 0.0, 0.0],
    };
    let (output, report) = edit_widget_text(&input, &request, None).unwrap();
    assert_eq!(value(&output, 14, "ActualText"), "XYZ XYZ");
    assert_eq!(value(&output, 12, "V"), "XYZ");
    assert_eq!(report.widget_pages, vec![1]);
    assert_eq!(report.affected_pages, vec![1, 2]);
    assert_eq!(
        report.affected_pages_scope,
        "conservative_document_wide_for_structural_owners"
    );
    assert!(report.default_appearance["automatic_viewer_font_size"]
        .as_bool()
        .unwrap());
    assert!(value(&output, 12, "DA").contains(" 0 Tf"));
    crate::tagged_structure::validate_parent_tree(&output).unwrap();
    request.widgets[1]
        .appearance
        .tagged_clone
        .actual_text_updates[0]
        .replacement_text = "conflicting".into();
    assert!(edit_widget_text(&input, &request, None).is_err());
}

#[test]
fn shared_field_index_keeps_independent_owners_separate_and_detects_stray_claims() {
    let original = field_fixture(false);
    let mut first = object(&original, 12).as_dict().unwrap().clone();
    first.insert("Kids", PdfObject::Array(vec![r(7)]));
    let mut second = first.clone();
    second.insert("T", crate::annotation_identity::text_string("second"));
    second.insert("Kids", PdfObject::Array(vec![r(9)]));
    let mut widget = object(&original, 9).as_dict().unwrap().clone();
    widget.insert("Parent", r(13));
    let mut catalog = object(&original, 1).as_dict().unwrap().clone();
    let mut form = catalog.get("AcroForm").unwrap().as_dict().unwrap().clone();
    form.insert("Fields", PdfObject::Array(vec![r(12), r(13)]));
    catalog.insert("AcroForm", PdfObject::Dictionary(form));
    let engine = ContentEngine::open_bytes(original.clone()).unwrap();
    let input = write_incremental_update(
        engine.document().reader(),
        [(1, catalog), (9, widget), (12, first.clone()), (13, second)]
            .into_iter()
            .map(|(number, dictionary)| IncrementalObject {
                number,
                generation: 0,
                object: PdfObject::Dictionary(dictionary),
            })
            .collect(),
    )
    .unwrap();
    let fields = discover_widget_fields(&input, &[1]).unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].target.field, (12, 0));
    assert_eq!(fields[1].target.field, (13, 0));
    assert!(fields.iter().all(|field| field.widgets.len() == 1));
    let mut request = field_request(&input);
    request
        .widgets
        .retain(|widget| widget.appearance.target.annotation == (7, 0));
    let (output, _) = edit_widget_text(&input, &request, None).unwrap();
    assert_eq!(value(&output, 12, "V"), "XYZ");
    assert_eq!(value(&output, 13, "V"), "ABC");
    assert_eq!(object(&input, 9), object(&output, 9));

    // On the original field, widget 9 still claims Parent 12. Removing it
    // from Kids must not turn the indexed lookup into a partial transaction.
    let broken = modify(&original, 12, PdfObject::Dictionary(first));
    assert!(discover_widget_fields(&broken, &[1]).is_err());
}
