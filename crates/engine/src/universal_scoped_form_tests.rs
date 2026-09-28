//! Unexecuted full-route regression source using real source Form fixtures.
use super::*;
use crate::universal_editing::scoped_text::{ScopedTextEditRequest, ScopedTextSource};
use crate::universal_editing::*;

fn governed(request: FormTextEditRequest) -> UniversalEditRequestV2 {
    UniversalEditRequestV2 {
        operation: UniversalEditOperationV2::ScopedText {
            request: ScopedTextEditRequest {
                source: ScopedTextSource::Form { request },
                approved_font_asset: None,
                planned_output_sha256: None,
            },
        },
        policy: UniversalEditPolicyV2::default(),
    }
}
fn approve(plan: &UniversalEditPlanV2) -> UniversalApprovalTokenV2 {
    create_universal_approval_token_v2(
        plan,
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
        },
    )
    .unwrap()
}

#[test]
fn governed_nested_form_clone_preserves_other_page_and_source_programs() {
    let input = fixture(true, true, false);
    let request = governed(request(selected(&input, 1, true), "XYZ"));
    let plan = plan_universal_edit_v2(&input, &request).unwrap();
    let (output, report) = apply_universal_edit_v2(&input, &plan, Some(&approve(&plan))).unwrap();
    assert_eq!(report.affected_pages, vec![1]);
    assert_eq!(
        report.operation_report["native"]["direct_text_after"],
        "XYZ"
    );
    for id in [4, 5, 8, 9] {
        assert_eq!(object(&input, id), object(&output, id));
    }
    assert!(analyze_form_text(&output, 2)
        .unwrap()
        .occurrences
        .iter()
        .filter(|item| !item.text.logical_text.is_empty())
        .all(|item| item.text.logical_text == "ABC"));
    assert_eq!(
        plan.preview["output_sha256"],
        format!("{:x}", Sha256::digest(output))
    );
}

#[test]
fn governed_edit_all_uses_reports_document_invalidation_and_changes_each_occurrence() {
    let input = fixture(false, false, false);
    let mut edit = request(selected(&input, 0, false), "XYZ");
    edit.shared_form_policy = SharedFormEditPolicy::EditAllUses;
    let plan = plan_universal_edit_v2(&input, &governed(edit)).unwrap();
    let (output, report) = apply_universal_edit_v2(&input, &plan, Some(&approve(&plan))).unwrap();
    assert_eq!(report.affected_pages, vec![1, 2]);
    assert!(report.affected_objects.contains(&"object-5-0".into()));
    assert_eq!(
        report.operation_report["invalidation"],
        "conservative_document_wide_shared_or_tagged_ownership"
    );
    for page in [1, 2] {
        assert!(analyze_form_text(&output, page)
            .unwrap()
            .occurrences
            .iter()
            .all(|item| item.text.logical_text == "XYZ"));
    }
}

#[test]
fn incompatible_edit_all_contexts_do_not_become_approvable_candidates() {
    let input = fixture(false, false, true);
    let mut edit = request(selected(&input, 0, false), "XYZ");
    edit.shared_form_policy = SharedFormEditPolicy::EditAllUses;
    let plan = plan_universal_edit_v2(&input, &governed(edit)).unwrap();
    assert_eq!(plan.state, UniversalPlanStateV2::PolicyDenied);
    assert!(plan.candidates.is_empty());
    let error = apply_universal_edit_v2(&input, &plan, None).unwrap_err();
    assert!(matches!(
        error,
        WellfriendError::UnsupportedFeature(ref message) if message.contains("no_change")
    ));
}

#[test]
fn source_scope_inventory_is_explicit_and_uses_the_requested_page_window() {
    let input = fixture(false, false, false);
    let options = UniversalAnalyzeOptionsV2 {
        pages: vec![2],
        include_scoped_text_sources: true,
        ..Default::default()
    };
    let model = analyze_universal_document_v2(&input, &options).unwrap();
    assert_eq!(model.scoped_text_sources["status"], "bound");
    let pages = model.scoped_text_sources["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["page"], 2);
    assert_eq!(
        pages[0]["forms"]["occurrences"].as_array().unwrap().len(),
        2
    );
    assert!(pages[0]["appearances"]["occurrences"]
        .as_array()
        .unwrap()
        .is_empty());
    let model =
        analyze_universal_document_v2(&input, &UniversalAnalyzeOptionsV2::default()).unwrap();
    assert_eq!(model.scoped_text_sources["status"], "not_requested");
}

#[test]
fn nested_signature_override_is_normalized_to_the_governing_policy() {
    let input = fixture(false, false, false);
    let mut edit = request(selected(&input, 0, false), "XYZ");
    edit.edit.options.signature_policy_override = true;
    edit.edit.options.deterministic = false;
    let plan = plan_universal_edit_v2(&input, &governed(edit)).unwrap();
    let UniversalEditOperationV2::ScopedText { request } = plan.execution_operation else {
        panic!()
    };
    let ScopedTextSource::Form { request } = request.source else {
        panic!()
    };
    assert!(!request.edit.options.signature_policy_override);
    assert!(request.edit.options.deterministic);
}
