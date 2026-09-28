//! Unexecuted regression source for the full native scoped plan/approve/apply
//! route. Reuses the appearance fixtures, not a mock transaction dispatcher.
use super::*;
use crate::universal_editing::scoped_text::{ScopedTextEditRequest, ScopedTextSource};
use crate::universal_editing::*;

fn governed(edit: AppearanceTextEditRequest) -> UniversalEditRequestV2 {
    UniversalEditRequestV2 {
        operation: UniversalEditOperationV2::ScopedText {
            request: ScopedTextEditRequest {
                source: ScopedTextSource::Appearance { request: edit },
                approved_font_asset: None,
                planned_output_sha256: None,
            },
        },
        policy: UniversalEditPolicyV2::default(),
    }
}
fn approve(plan: &UniversalEditPlanV2) -> UniversalApprovalTokenV2 {
    create_universal_approval_token_v2(plan, decision(plan)).unwrap()
}
fn decision(plan: &UniversalEditPlanV2) -> UniversalApprovalDecisionV2 {
    UniversalApprovalDecisionV2 {
        selected_candidate_ids: plan.selected_candidate_ids.clone(),
        approved_font: plan
            .preview
            .pointer("/font/required_approved_font")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        mutation_mode: plan.policy.mutation_mode,
        accept_visual_change: true,
        accept_signature_invalidation: false,
    }
}

#[test]
fn rendered_scoped_preview_is_bound_to_the_native_candidate_and_keeps_other_appearance() {
    use crate::universal_editing::scoped_preview::{
        preview_scoped_candidate, ScopedPreviewOptions,
    };
    let input = fixture(false);
    let plan =
        plan_universal_edit_v2(&input, &governed(request(selected(&input, false), "XYZ"))).unwrap();
    let options = ScopedPreviewOptions {
        dpi: 72,
        ..Default::default()
    };
    let preview = preview_scoped_candidate(&input, &plan, &options).unwrap();
    assert_eq!(preview.plan_id, plan.plan_id);
    assert_eq!(
        preview.input_sha256,
        format!("{:x}", Sha256::digest(&input))
    );
    assert_eq!(preview.total_pixels, 300 * 200 * 2);
    let page = &preview.pages[0];
    assert_eq!((page.page, page.width, page.height), (1, 300, 200));
    assert!(page.before.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(page.candidate.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(page.difference.changed_pixels > 0);
    // The other annotation uses the same old AP at x=150. Clone-one must not
    // change its pixels; neither must the unrelated page text change.
    assert!(page.difference.bounds.unwrap()[2] <= 100);
    assert!(apply_universal_edit_v2(&input, &plan, None).is_err());
    let (output, _) = apply_universal_edit_v2(&input, &plan, Some(&approve(&plan))).unwrap();
    assert_eq!(
        preview.candidate_output_sha256,
        format!("{:x}", Sha256::digest(&output))
    );
}

#[test]
fn preview_refuses_mutated_plans_and_combined_pixel_overrun() {
    use crate::universal_editing::scoped_preview::{
        preview_scoped_candidate, ScopedPreviewOptions,
    };
    let input = fixture(false);
    let plan =
        plan_universal_edit_v2(&input, &governed(request(selected(&input, false), "XYZ"))).unwrap();
    let mut forged = plan.clone();
    forged.preview["output_sha256"] = serde_json::json!("forged");
    assert!(preview_scoped_candidate(&input, &forged, &Default::default()).is_err());
    // Each page alone would fit. The budget explicitly counts both rasters.
    let options = ScopedPreviewOptions {
        dpi: 72,
        max_total_pixels: 60_001,
        ..Default::default()
    };
    assert!(matches!(
        preview_scoped_candidate(&input, &plan, &options),
        Err(crate::WellfriendError::ResourceLimit(_))
    ));
    let mut annotation = object(&input, 7).as_dict().unwrap().clone();
    annotation.insert(
        "Contents",
        crate::annotation_identity::text_string("new revision"),
    );
    let changed = modify(&input, 7, PdfObject::Dictionary(annotation));
    assert!(preview_scoped_candidate(&changed, &plan, &Default::default()).is_err());
}

#[test]
fn scoped_preview_sdk_returns_rasters_not_published_pdf_bytes() {
    let input = fixture(false);
    let request = governed(request(selected(&input, false), "XYZ"));
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    let report = crate::sdk::universal_editing_scoped_preview_v2_json(
        &input,
        &serde_json::to_string(&plan).unwrap(),
        Some(r#"{"dpi":24}"#),
        None,
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(value["kind"], "universal_editing_scoped_preview_v2");
    assert_eq!(value["report"]["pages"][0]["before"]["png"][0], 137);
    assert!(value["report"].get("bytes").is_none());
    assert_eq!(
        value["report"]["candidate_output_sha256"],
        plan.preview["output_sha256"]
    );
}

#[test]
fn scoped_appearance_candidate_requires_approval_and_matches_the_published_bytes() {
    let input = fixture(false);
    let mut request = governed(request(selected(&input, false), "XYZ"));
    request.policy.allow_font_substitution = false;
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    assert_eq!(plan.state, UniversalPlanStateV2::ApprovalRequired);
    assert_eq!(plan.preview["font"]["requires_font_approval"], false);
    assert!(apply_universal_edit_v2(&input, &plan, None).is_err());
    let (output, result) = apply_universal_edit_v2(&input, &plan, Some(&approve(&plan))).unwrap();
    assert_eq!(result.outcome, UniversalEditOutcomeV2::Applied);
    assert_eq!(
        plan.preview["output_sha256"],
        format!("{:x}", Sha256::digest(&output))
    );
    assert_eq!(
        result.operation_report["native"]["direct_text_after"],
        "XYZ"
    );
    assert!(output.starts_with(&input));
    assert_eq!(object(&input, 5), object(&output, 5));
    assert_eq!(object(&input, 9), object(&output, 9));
    assert!(result.affected_objects.contains(&"object-7-0".into()));
    assert!(!result.affected_objects.contains(&"object-5-0".into()));
}

#[test]
fn generated_font_requires_exact_pinned_font_approval_not_an_arbitrary_name() {
    let input = fixture(false);
    let mut edit = request(selected(&input, false), "Long Unicode replacement");
    edit.edit.style_policy = MultiRunStylePolicy::ExplicitSupplied;
    let request = governed(edit);
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    assert_eq!(plan.preview["font"]["requires_font_approval"], true);
    let mut decision = decision(&plan);
    decision.approved_font = None;
    assert!(create_universal_approval_token_v2(&plan, decision.clone()).is_err());
    decision.approved_font = Some("unplanned-font".into());
    assert!(create_universal_approval_token_v2(&plan, decision).is_err());
    let (output, result) = apply_universal_edit_v2(&input, &plan, Some(&approve(&plan))).unwrap();
    assert!(result.changed);
    assert_eq!(
        result.operation_report["native"]["direct_text_after"],
        "Long Unicode replacement"
    );
    assert_eq!(
        plan.preview["output_sha256"],
        format!("{:x}", Sha256::digest(output))
    );
}

#[test]
fn generated_font_is_withheld_when_substitution_is_disabled() {
    let input = fixture(false);
    let mut edit = request(selected(&input, false), "XYZ");
    edit.edit.style_policy = MultiRunStylePolicy::ExplicitSupplied;
    let mut request = governed(edit);
    request.policy.allow_font_substitution = false;
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    assert_eq!(plan.state, UniversalPlanStateV2::PolicyDenied);
    let error = apply_universal_edit_v2(&input, &plan, None).unwrap_err();
    assert!(matches!(
        error,
        WellfriendError::UnsupportedFeature(ref message) if message.contains("no_change")
    ));
}

#[test]
fn scoped_approval_rejects_missing_duplicate_wrong_and_extraneous_font_decisions() {
    let input = fixture(false);
    let plan =
        plan_universal_edit_v2(&input, &governed(request(selected(&input, false), "XYZ"))).unwrap();
    for selected in [
        Vec::new(),
        vec!["not-this-scope".into()],
        vec![plan.selected_candidate_ids[0].clone(); 2],
    ] {
        let mut decision = decision(&plan);
        decision.selected_candidate_ids = selected;
        assert!(create_universal_approval_token_v2(&plan, decision).is_err());
    }
    let mut decision = decision(&plan);
    decision.approved_font = Some("unneeded-font".into());
    assert!(create_universal_approval_token_v2(&plan, decision).is_err());
}

#[test]
fn tampered_candidate_preview_and_pinned_program_are_rejected() {
    let input = fixture(false);
    let plan =
        plan_universal_edit_v2(&input, &governed(request(selected(&input, false), "XYZ"))).unwrap();
    let approval = approve(&plan);
    let mut forged = plan.clone();
    forged.preview["output_sha256"] = serde_json::Value::String("forged".into());
    assert!(apply_universal_edit_v2(&input, &forged, Some(&approval)).is_err());
    let mut forged = plan.clone();
    let UniversalEditOperationV2::ScopedText { request } = &mut forged.execution_operation else {
        panic!()
    };
    request.approved_font_asset.as_mut().unwrap().bytes[0] ^= 1;
    assert!(apply_universal_edit_v2(&input, &forged, Some(&approval)).is_err());
    let mut forged = plan.clone();
    let UniversalEditOperationV2::ScopedText { request } = &mut forged.execution_operation else {
        panic!()
    };
    request.planned_output_sha256 = Some("forged".into());
    assert!(apply_universal_edit_v2(&input, &forged, Some(&approval)).is_err());
}

#[test]
fn source_receipt_cannot_be_prepopulated_and_input_revision_cannot_be_swapped() {
    let input = fixture(false);
    let request = governed(request(selected(&input, false), "XYZ"));
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    let mut owner = object(&input, 7).as_dict().unwrap().clone();
    owner.insert(
        "Contents",
        crate::annotation_identity::text_string("changed"),
    );
    let changed = modify(&input, 7, PdfObject::Dictionary(owner));
    assert!(apply_universal_edit_v2(&changed, &plan, Some(&approve(&plan))).is_err());
    let mut request = request;
    let UniversalEditOperationV2::ScopedText { request: scoped } = &mut request.operation else {
        panic!()
    };
    scoped.planned_output_sha256 = Some("caller-supplied".into());
    assert!(plan_universal_edit_v2(&input, &request).is_err());
}

#[test]
fn native_widget_boundary_becomes_a_non_applicable_no_output_plan() {
    let input = fixture(false);
    let mut owner = object(&input, 7).as_dict().unwrap().clone();
    owner.insert("Subtype", n("Widget"));
    let input = modify(&input, 7, PdfObject::Dictionary(owner));
    let plan =
        plan_universal_edit_v2(&input, &governed(request(selected(&input, false), "XYZ"))).unwrap();
    assert_eq!(plan.state, UniversalPlanStateV2::PolicyDenied);
    assert!(plan.candidates.is_empty());
    assert_eq!(plan.preview["status"], "unsupported_no_output");
    let error = apply_universal_edit_v2(&input, &plan, None).unwrap_err();
    assert!(matches!(
        error,
        WellfriendError::UnsupportedFeature(ref message) if message.contains("no_change")
    ));
}

#[test]
fn universal_tagged_clone_preserves_parent_ownership_and_binds_actual_text_decisions() {
    use crate::tagged_structure::stream_clones::{StructureActualTextUpdate, TaggedClonePolicy};
    let input = super::tagged::tagged_fixture(false, true);
    let mut owner = object(&input, 13).as_dict().unwrap().clone();
    owner.insert("ActualText", crate::annotation_identity::text_string("ABC"));
    let input = modify(&input, 13, PdfObject::Dictionary(owner));
    let mut edit = request(selected(&input, false), "XYZ");
    edit.tagged_clone.policy = TaggedClonePolicy::SplitSharedNamespacesAfterSource;
    edit.tagged_clone
        .actual_text_updates
        .push(StructureActualTextUpdate {
            element: (13, 0),
            expected_text: "ABC".into(),
            replacement_text: "ABC XYZ".into(),
        });
    let request = governed(edit);
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    assert_eq!(
        plan.preview["invalidation"],
        "conservative_document_wide_shared_or_tagged_ownership"
    );
    let (output, report) = apply_universal_edit_v2(&input, &plan, Some(&approve(&plan))).unwrap();
    assert!(report.changed);
    assert!(
        crate::tagged_structure::validate_parent_tree(&output)
            .unwrap()
            .ownership_verified
    );
    assert_eq!(
        object(&output, 13).as_dict().unwrap().get("ActualText"),
        Some(&crate::annotation_identity::text_string("ABC XYZ"))
    );
    let mut forged = plan;
    let UniversalEditOperationV2::ScopedText { request } = &mut forged.execution_operation else {
        panic!()
    };
    let ScopedTextSource::Appearance { request } = &mut request.source else {
        panic!()
    };
    request.tagged_clone.actual_text_updates[0].replacement_text = "unapproved".into();
    assert!(apply_universal_edit_v2(&input, &forged, None).is_err());
}

#[test]
fn governed_free_text_metadata_decision_is_in_the_exact_candidate() {
    let input = fixture(false);
    let mut owner = object(&input, 7).as_dict().unwrap().clone();
    owner.insert("Subtype", n("FreeText"));
    owner.insert("Contents", crate::annotation_identity::text_string("ABC"));
    owner.insert("RC", PdfObject::String(b"rich".to_vec()));
    let input = modify(&input, 7, PdfObject::Dictionary(owner));
    let mut edit = request(selected(&input, false), "XYZ");
    edit.metadata_policy = AppearanceMetadataPolicy::SynchronizeFreeTextPlainText;
    let plan = plan_universal_edit_v2(&input, &governed(edit)).unwrap();
    assert_eq!(plan.preview["native_report"]["rich_text_discarded"], true);
    let (output, _) = apply_universal_edit_v2(&input, &plan, Some(&approve(&plan))).unwrap();
    assert_eq!(
        object(&output, 7).as_dict().unwrap().get("Contents"),
        Some(&crate::annotation_identity::text_string("XYZ"))
    );
    assert!(!object(&output, 7).as_dict().unwrap().contains_key("RC"));
}

#[test]
fn cancelled_scoped_planning_and_apply_do_not_publish_a_candidate() {
    let input = fixture(false);
    let request = governed(request(selected(&input, false), "XYZ"));
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    let approval = approve(&plan);
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    assert!(matches!(
        token.scope(|| plan_universal_edit_v2(&input, &request)),
        Err(WellfriendError::Cancelled(_))
    ));
    assert!(matches!(
        token.scope(|| apply_universal_edit_v2(&input, &plan, Some(&approval))),
        Err(WellfriendError::Cancelled(_))
    ));
}

#[test]
fn json_facades_carry_scoped_requests_without_new_abi_dispatch() {
    let input = fixture(false);
    let request = governed(request(selected(&input, false), "XYZ"));
    let plan_json = crate::sdk::universal_editing_plan_v2_json(
        &input,
        &serde_json::to_string(&request).unwrap(),
        None,
    )
    .unwrap();
    let envelope: serde_json::Value = serde_json::from_str(&plan_json).unwrap();
    let plan: UniversalEditPlanV2 = serde_json::from_value(envelope["report"].clone()).unwrap();
    let plan_json = serde_json::to_string(&plan).unwrap();
    let approval = crate::sdk::universal_editing_approval_v2_json(
        &plan_json,
        &serde_json::to_string(&decision(&plan)).unwrap(),
    )
    .unwrap();
    let approval: serde_json::Value = serde_json::from_str(&approval).unwrap();
    let (output, report) = crate::sdk::universal_editing_apply_v2_json(
        &input,
        &plan_json,
        Some(&approval["report"].to_string()),
        None,
    )
    .unwrap();
    assert_ne!(output, input);
    let report: serde_json::Value = serde_json::from_str(&report).unwrap();
    assert_eq!(report["report"]["outcome"], "applied");
}

#[test]
fn universal_analysis_supplies_exact_appearance_targets_for_the_governed_route() {
    let input = fixture(false);
    let options = UniversalAnalyzeOptionsV2 {
        pages: vec![1],
        include_scoped_text_sources: true,
        ..Default::default()
    };
    let model = analyze_universal_document_v2(&input, &options).unwrap();
    let sources = model.scoped_text_sources["pages"][0]["appearances"]["occurrences"]
        .as_array()
        .unwrap();
    let target: AppearanceTextTarget =
        serde_json::from_value(sources[0]["target"].clone()).unwrap();
    assert_eq!(target.input_sha256, format!("{:x}", Sha256::digest(&input)));
    let plan = plan_universal_edit_v2(&input, &governed(request(target, "XYZ"))).unwrap();
    let (output, report) = apply_universal_edit_v2(&input, &plan, Some(&approve(&plan))).unwrap();
    assert!(report.changed);
    assert_eq!(
        analyze_appearance_text(&output, 1).unwrap().occurrences[0]
            .text
            .logical_text,
        "XYZ"
    );
}

#[test]
fn caller_font_asset_bytes_and_name_are_bound_in_the_execution_plan() {
    let input = fixture(false);
    let mut edit = request(selected(&input, false), "XYZ");
    edit.edit.style_policy = MultiRunStylePolicy::ExplicitSupplied;
    let mut request = governed(edit);
    let bytes = crate::render::get_fallback_font("Symbol").unwrap().to_vec();
    let UniversalEditOperationV2::ScopedText { request: scoped } = &mut request.operation else {
        panic!()
    };
    scoped.approved_font_asset = Some(crate::editing_transactions::ApprovedFontAsset {
        lookup_name: "approved-caller-font".into(),
        bytes: bytes.clone(),
    });
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    assert_eq!(
        plan.preview["font"]["program_sha256"],
        format!("{:x}", Sha256::digest(&bytes))
    );
    assert_eq!(
        plan.preview["font"]["required_approved_font"],
        "approved-caller-font"
    );
    let UniversalEditOperationV2::ScopedText { request } = &plan.execution_operation else {
        panic!()
    };
    assert_eq!(request.approved_font_asset.as_ref().unwrap().bytes, bytes);
    let (_, result) = apply_universal_edit_v2(&input, &plan, Some(&approve(&plan))).unwrap();
    assert!(result.changed);
}
