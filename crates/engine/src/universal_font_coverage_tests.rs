//! Unexecuted source regressions for exact-asset approval coverage.
use super::*;
use crate::editing_transactions::ApprovedFontAsset;

fn source_pdf() -> Vec<u8> {
    use crate::authoring::{PageSize, PdfBuilder, StandardFont, TextStyle};
    let mut builder = PdfBuilder::new();
    builder
        .add_page(PageSize::LETTER)
        .draw_text(
            "ABC",
            10.0,
            150.0,
            &TextStyle::standard(StandardFont::Helvetica, 12.0),
        )
        .unwrap();
    builder.to_bytes().unwrap()
}

fn request(asset: ApprovedFontAsset, replacement: &str) -> UniversalEditRequestV2 {
    UniversalEditRequestV2 {
        operation: UniversalEditOperationV2::Text {
            request: crate::editing_transactions::SceneTextEditRequest {
                requested_mode: crate::source_editing::TrueEditingMode::GeometricBlock,
                source_text: "ABC".into(),
                replacement_text: replacement.into(),
                font_policy: "allow_substitute".into(),
                approved_font_asset: Some(asset),
                region: Some([10.0, 10.0, 190.0, 190.0]),
                ..Default::default()
            },
        },
        policy: UniversalEditPolicyV2::default(),
    }
}

fn decision(plan: &UniversalEditPlanV2, font: &str) -> UniversalApprovalDecisionV2 {
    UniversalApprovalDecisionV2 {
        selected_candidate_ids: vec![plan
            .candidates
            .iter()
            .find(|c| c.exact)
            .unwrap()
            .candidate_id
            .clone()],
        approved_font: Some(font.into()),
        mutation_mode: plan.policy.mutation_mode,
        accept_visual_change: true,
        accept_signature_invalidation: false,
    }
}

#[test]
fn an_empty_subset_outline_cannot_be_approved_using_a_bundled_name_collision() {
    let font = crate::render::get_fallback_font("Symbol").unwrap();
    let face = ttf_parser::Face::parse(font, 0).unwrap();
    let subset = crate::fonts::sfnt_subset::subset_glyf_preserving_gids(
        font,
        &BTreeSet::from([face.glyph_index('A').unwrap().0]),
    )
    .unwrap();
    let asset = ApprovedFontAsset {
        lookup_name: "Helvetica".into(),
        bytes: subset.bytes,
    };
    let report = universal_substitution_report_v2(
        "Helvetica",
        "Z",
        Some("allow_substitute"),
        None,
        Some(&asset),
    );
    assert_eq!(
        report["ranked_candidates"][0]["eligible_for_approval"],
        false
    );
    assert_eq!(
        report["ranked_candidates"][0]["missing_glyph_clusters"],
        json!([0])
    );
    assert_eq!(report["approved_candidates"], json!([]));
    assert!(report["chosen_substitute"].is_null());
    assert_eq!(report["approval_scope"], "exact_supplied_font_asset");
}

#[test]
fn a_normalized_cluster_can_be_approved_without_its_precomposed_cmap_entry() {
    let bytes = crate::fonts::sfnt_subset::with_test_cmap(
        crate::render::get_fallback_font("Symbol").unwrap(),
        &[' ', 'A', '\u{030a}'],
    )
    .unwrap();
    assert!(ttf_parser::Face::parse(&bytes, 0)
        .unwrap()
        .glyph_index('Å')
        .is_none());
    let asset = ApprovedFontAsset {
        lookup_name: "Caller".into(),
        bytes,
    };
    let report = universal_substitution_report_v2(
        "Helvetica",
        "Å",
        Some("allow_substitute"),
        None,
        Some(&asset),
    );
    assert_eq!(
        report["ranked_candidates"][0]["eligible_for_approval"],
        true
    );
    assert_eq!(report["ranked_candidates"][0]["missing_scalars"], json!([]));
    assert_eq!(report["approved_candidates"], json!(["Caller"]));
    assert_eq!(report["chosen_substitute"], "Caller");
}

#[test]
fn malformed_bound_assets_cannot_inherit_other_candidates_approval() {
    let asset = ApprovedFontAsset {
        lookup_name: "Helvetica".into(),
        bytes: vec![1, 2, 3],
    };
    let report = universal_substitution_report_v2(
        "Helvetica",
        "A",
        Some("allow_substitute"),
        None,
        Some(&asset),
    );
    assert_eq!(report["approved_candidates"], json!([]));
    assert_eq!(report["status"], "caller_font_asset_ineligible");
}

#[test]
fn bundled_substitution_reports_shaped_cluster_metrics_not_cmap_only_approval() {
    let report = crate::editing_transactions::substitution_report(
        "Helvetica",
        "A\u{200d}B",
        Some("allow_substitute"),
    );
    let candidates = report["ranked_candidates"].as_array().unwrap();
    assert!(candidates
        .iter()
        .any(|candidate| candidate["eligible_for_approval"] == true));
    for candidate in candidates
        .iter()
        .filter(|candidate| candidate["eligible_for_approval"] == true)
    {
        assert_eq!(candidate["metrics"]["shaped_coverage_complete"], true);
        assert_eq!(candidate["metrics"]["missing_glyph_clusters"], json!([]));
        assert!(candidate["metrics"]["coverage_error"].is_null());
    }
}

#[test]
fn invalid_exact_asset_is_denied_by_the_complete_plan_and_approval_boundary() {
    let input = source_pdf();
    let font = crate::render::get_fallback_font("Symbol").unwrap();
    let face = ttf_parser::Face::parse(font, 0).unwrap();
    let subset = crate::fonts::sfnt_subset::subset_glyf_preserving_gids(
        font,
        &BTreeSet::from([face.glyph_index('A').unwrap().0]),
    )
    .unwrap();
    let plan = plan_universal_edit_v2(
        &input,
        &request(
            ApprovedFontAsset {
                lookup_name: "Helvetica".into(),
                bytes: subset.bytes,
            },
            "Z",
        ),
    )
    .unwrap();
    assert!(plan.candidates.iter().any(|candidate| candidate.exact));
    assert_eq!(plan.state, UniversalPlanStateV2::PolicyDenied);
    assert!(create_universal_approval_token_v2(&plan, decision(&plan, "Helvetica")).is_err());
}

#[test]
fn normalized_exact_asset_survives_plan_approval_apply_and_reopen() {
    let input = source_pdf();
    let bytes = crate::fonts::sfnt_subset::with_test_cmap(
        crate::render::get_fallback_font("Symbol").unwrap(),
        &[' ', 'A', '\u{030a}'],
    )
    .unwrap();
    let plan = plan_universal_edit_v2(
        &input,
        &request(
            ApprovedFontAsset {
                lookup_name: "Caller".into(),
                bytes,
            },
            "Å",
        ),
    )
    .unwrap();
    assert_eq!(plan.state, UniversalPlanStateV2::ApprovalRequired);
    assert!(create_universal_approval_token_v2(&plan, decision(&plan, "Helvetica")).is_err());
    let approval = create_universal_approval_token_v2(&plan, decision(&plan, "Caller")).unwrap();
    let (output, report) = apply_universal_edit_v2(&input, &plan, Some(&approval)).unwrap();
    assert!(report.changed);
    let reopened = crate::ContentEngine::open_bytes(output).unwrap();
    assert_eq!(
        reopened
            .collect_page_text_chunks(1)
            .unwrap()
            .into_iter()
            .map(|chunk| chunk.text)
            .collect::<String>(),
        "Å"
    );
}

#[test]
fn non_executable_source_candidate_does_not_erase_exact_logical_provenance() {
    let candidate = UniversalCandidateV2 {
        candidate_id: "instruction-refused-by-font".into(),
        page: 1,
        kind: "text_source_instruction".into(),
        source_identity: json!({
            "stream_object": 17,
            "stream_generation": 0,
            "decoded_byte_range": [40, 55],
        }),
        confidence: 0.7,
        exact: false,
        shared_resource: false,
        approval_reason: Some("replacement is not encodable by the source font".into()),
    };

    assert!(!has_executable_instruction_candidate(
        std::slice::from_ref(&candidate),
        17,
        0,
        [40, 55]
    ));
    assert!(has_executable_instruction_candidate(
        &[UniversalCandidateV2 {
            exact: true,
            ..candidate
        }],
        17,
        0,
        [40, 55]
    ));
}
