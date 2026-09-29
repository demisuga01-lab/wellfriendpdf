//! Governed native Form/AP text edits. Planning stages a private, deterministic
//! candidate; apply reuses that private revision-bound artifact when available,
//! or recomputes it on a cache miss, and publishes those same bytes only after
//! canonical plan and approval checks have succeeded.
use super::*;
use crate::advanced_editing::form_text::{self, appearance};
use crate::editing_transactions::ApprovedFontAsset;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ScopedTextSource {
    Form {
        request: form_text::FormTextEditRequest,
    },
    Appearance {
        request: appearance::AppearanceTextEditRequest,
    },
    WidgetField {
        request: appearance::widgets::WidgetTextEditRequest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopedTextEditRequest {
    pub source: ScopedTextSource,
    #[serde(default)]
    pub approved_font_asset: Option<ApprovedFontAsset>,
    /// Planner-owned execution receipt. New user requests must omit this field.
    /// It enters the canonical plan/approval digest together with pinned bytes.
    #[serde(default)]
    pub planned_output_sha256: Option<String>,
}

#[derive(Clone)]
pub(super) struct StagedScopedText {
    pub bytes: Vec<u8>,
    pub report: Value,
    pub pages: Vec<usize>,
    pub objects: Vec<String>,
    pub cloned_resources: Vec<String>,
}

pub(super) struct ScopedTextPlan {
    pub execution: ScopedTextEditRequest,
    pub state: UniversalPlanStateV2,
    pub candidate: Option<UniversalCandidateV2>,
    pub preview: Value,
    pub implementation: Value,
    pub read_set: Vec<String>,
    pub write_set: Vec<String>,
    pub reasons: Vec<String>,
    pub staged: Option<StagedScopedText>,
}

fn invalid(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("universal scoped text: {message}"))
}

pub(super) fn discover(input: &[u8], pages: &[usize]) -> Result<Value> {
    let mut result = Vec::new();
    let mut occurrences = 0usize;
    let mut serialized_bytes = 0usize;
    for &page in pages {
        crate::cancel::check_current_cancel("universal scoped text source discovery")?;
        let forms = form_text::analyze_form_text(input, page)?;
        let appearances = appearance::analyze_appearance_text(input, page)?;
        occurrences = occurrences
            .saturating_add(forms.occurrences.len())
            .saturating_add(appearances.occurrences.len());
        if occurrences > MAX_TEXT_CANDIDATES {
            return Err(WellfriendError::ResourceLimit(
                "scoped text discovery occurrence budget exceeded".into(),
            ));
        }
        let value = json!({"page":page, "forms":forms, "appearances":appearances});
        serialized_bytes =
            serialized_bytes.saturating_add(serde_json::to_vec(&value).map_err(json_error)?.len());
        if serialized_bytes > 64 * 1024 * 1024 {
            return Err(WellfriendError::ResourceLimit(
                "scoped text discovery report budget exceeded; request a smaller page window"
                    .into(),
            ));
        }
        result.push(value);
    }
    let widget_fields = appearance::widgets::discover_widget_fields(input, pages)?;
    serialized_bytes = serialized_bytes.saturating_add(
        serde_json::to_vec(&widget_fields)
            .map_err(json_error)?
            .len(),
    );
    if serialized_bytes > 64 * 1024 * 1024 {
        return Err(WellfriendError::ResourceLimit(
            "scoped field discovery report budget exceeded".into(),
        ));
    }
    Ok(
        json!({"status":"bound", "input_sha256":digest_hex(input), "pages":result,
        "widget_fields":widget_fields,
        "coordinate_space":"source_local_before_form_matrix_or_appearance_placement",
        "not_page_logical_or_visual_hit_testing":true}),
    )
}

impl ScopedTextSource {
    fn normalize_options(&mut self, signature_override: bool) {
        let configure = |edit: &mut crate::advanced_editing::MultiRunTextRangeRequest| {
            edit.options.signature_policy_override = signature_override;
            edit.options.deterministic = true;
        };
        match self {
            Self::Form { request } => configure(&mut request.edit),
            Self::Appearance { request } => configure(&mut request.edit),
            Self::WidgetField { request } => {
                for widget in &mut request.widgets {
                    configure(&mut widget.appearance.edit);
                }
            }
        }
    }
    fn page(&self) -> usize {
        match self {
            Self::Form { request } => request.target.page,
            Self::Appearance { request } => request.target.page,
            Self::WidgetField { request } => request.target.page,
        }
    }
    fn target(&self) -> Result<Value> {
        match self {
            Self::Form { request } => serde_json::to_value(&request.target).map_err(json_error),
            Self::Appearance { request } => {
                serde_json::to_value(&request.target).map_err(json_error)
            }
            Self::WidgetField { request } => {
                serde_json::to_value(&request.target).map_err(json_error)
            }
        }
    }
    fn global_invalidation(&self) -> bool {
        match self {
            Self::Form { request } => {
                request.shared_form_policy == SharedFormEditPolicy::EditAllUses
            }
            Self::Appearance { request } => {
                request.tagged_clone.policy
                    != crate::tagged_structure::stream_clones::TaggedClonePolicy::Reject
            }
            Self::WidgetField { .. } => true,
        }
    }
}

fn font_asset(request: &ScopedTextEditRequest) -> Result<ApprovedFontAsset> {
    let asset = if let Some(asset) = &request.approved_font_asset {
        asset.clone()
    } else {
        let bytes = crate::render::get_fallback_font("Symbol")
            .ok_or_else(|| invalid("bundled source-writer fallback unavailable"))?;
        ApprovedFontAsset {
            lookup_name: "bundled-source-writer-fallback".into(),
            bytes: bytes.to_vec(),
        }
    };
    if asset.lookup_name.trim().is_empty()
        || asset.lookup_name.len() > 255
        || asset.bytes.is_empty()
        || asset.bytes.len() > 256 * 1024 * 1024
    {
        return Err(invalid("invalid or oversized font asset"));
    }
    ttf_parser::Face::parse(&asset.bytes, 0)
        .map_err(|_| invalid("font asset is not a supported sfnt/OpenType face"))?;
    Ok(asset)
}

pub(super) fn plan(
    input: &[u8],
    request: &ScopedTextEditRequest,
    policy: &UniversalEditPolicyV2,
) -> Result<ScopedTextPlan> {
    crate::cancel::check_current_cancel("universal scoped text candidate planning")?;
    if request.planned_output_sha256.is_some() {
        return Err(invalid(
            "new requests cannot set the planner-owned output receipt",
        ));
    }
    let asset = font_asset(request)?;
    let font_hash = digest_hex(&asset.bytes);
    let mut execution = request.clone();
    execution.approved_font_asset = Some(asset.clone());
    execution
        .source
        .normalize_options(policy.mutation_mode == UniversalMutationModeV2::AuthorizedRewrite);
    let before = ContentEngine::open_bytes(input.to_vec())?;
    let outcome = match &execution.source {
        ScopedTextSource::Form { request } => {
            form_text::edit_form_text(input, request, Some(&asset.bytes)).and_then(
                |(bytes, report)| Ok((bytes, serde_json::to_value(report).map_err(json_error)?)),
            )
        }
        ScopedTextSource::Appearance { request } => {
            appearance::edit_appearance_text(input, request, Some(&asset.bytes)).and_then(
                |(bytes, report)| Ok((bytes, serde_json::to_value(report).map_err(json_error)?)),
            )
        }
        ScopedTextSource::WidgetField { request } => {
            appearance::widgets::edit_widget_text(input, request, Some(&asset.bytes)).and_then(
                |(bytes, report)| Ok((bytes, serde_json::to_value(report).map_err(json_error)?)),
            )
        }
    };
    let (bytes, native_report) = match outcome {
        Ok(value) => value,
        Err(WellfriendError::UnsupportedFeature(reason)) => {
            return Ok(ScopedTextPlan {
                execution,
                state: UniversalPlanStateV2::PolicyDenied,
                candidate: None,
                preview: json!({"kind":"native_scoped_text", "status":"unsupported_no_output", "reason":reason}),
                implementation: json!({"route":"native_scoped_text", "candidate_created":false, "pixel_comparison_performed":false}),
                read_set: vec![format!("input-sha256:{}", digest_hex(input))],
                write_set: Vec::new(),
                reasons: vec![reason],
                staged: None,
            })
        }
        Err(error) => return Err(error),
    };
    let after = ContentEngine::open_bytes(bytes.clone())?;
    let definitions = after
        .document()
        .reader()
        .incremental_definition_ids_since(before.document().reader())?;
    let old_ids = before
        .document()
        .reader()
        .object_ids()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut font_definitions = Vec::new();
    let mut new_definitions = Vec::new();
    let mut cloned_resources = Vec::new();
    for &(number, generation) in &definitions {
        crate::cancel::check_current_cancel("scoped text output resource inventory")?;
        if !old_ids.contains(&(number, generation)) {
            new_definitions.push(format!("object-{number}-{generation}"));
        }
        let object = after.document().reader().get_object(number, generation)?;
        if !old_ids.contains(&(number, generation))
            && object
                .as_stream()
                .is_some_and(|(dict, _)| dict.get_name("Subtype") == Some("Form"))
        {
            cloned_resources.push(format!("object-{number}-{generation}"));
        }
        // Revisions of an existing source font are not substitutions, while
        // generated Type0 dictionaries may be direct children of a rewritten
        // resource dictionary rather than standalone new objects.  Bind the
        // approval gate to the writer's explicit generated-font marker instead
        // of guessing from indirect-object allocation.
        if contains_generated_font_marker(&object, 0) {
            font_definitions.push((number, generation));
        }
    }
    // A new font definition is a conservative typography-change gate: even
    // re-encoding/subsetting the same family needs disclosure. The source
    // writer itself checks shaping coverage and embedding restrictions.
    let generated_font = native_report
        .pointer("/native_edit/generated_font_used")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let state = if generated_font && !policy.allow_font_substitution {
        UniversalPlanStateV2::PolicyDenied
    } else {
        UniversalPlanStateV2::ApprovalRequired
    };
    let output_hash = digest_hex(&bytes);
    execution.planned_output_sha256 = Some(output_hash.clone());
    let source_json = serde_json::to_vec(&execution.source).map_err(json_error)?;
    let candidate_id = stable_id("scoped-text-v2", &[input, &source_json]);
    let candidate = UniversalCandidateV2 {
        candidate_id: candidate_id.clone(), page: execution.source.page(), kind: if matches!(&execution.source,ScopedTextSource::WidgetField{..}) {"scoped_text_field"} else {"scoped_text_occurrence"}.into(),
        source_identity: execution.source.target()?, confidence: 1.0, exact: true,
        // Reusable definition ownership is governed even when no second use
        // is currently known; this is not an occurrence-count measurement.
        shared_resource: true,
        approval_reason: Some("approve the exact source scope, all declared widget/value/default changes, shared-resource policy, text/style changes and annotation/tag decisions".into()),
    };
    let global = execution.source.global_invalidation();
    let pages = if global {
        (1..=after.document().page_count()?).collect::<Vec<_>>()
    } else {
        vec![execution.source.page()]
    };
    let objects = definitions
        .iter()
        .map(|(n, g)| format!("object-{n}-{g}"))
        .collect::<Vec<_>>();
    let required_font = generated_font.then_some(asset.lookup_name.as_str());
    let preview = json!({
        "kind":"native_scoped_text", "status":"private_candidate_staged", "output_sha256":output_hash,
        "source":execution.source, "native_report":native_report, "written_definitions":objects,
        "new_definitions":new_definitions, "affected_pages":pages,
        "invalidation":if global { "conservative_document_wide_shared_or_tagged_ownership" } else { "selected_page" },
        "font": {"lookup_name":asset.lookup_name, "program_sha256":font_hash, "program_bytes":asset.bytes.len(),
            "new_font_definitions":font_definitions, "requires_font_approval":generated_font, "required_approved_font":required_font,
            "source":if request.approved_font_asset.is_some() { "caller_asset" } else { "pinned_bundled_source_writer_fallback" },
            "unused_asset_does_not_imply_font_change":true},
        "pixel_comparison_performed":false, "visual_fidelity_certified":false,
        "native_target_reuse":"valid for the candidate bytes only; rediscover after canonical rewrite or output encryption"
    });
    let mut reasons = vec!["approve the native scoped text candidate, including source-local geometry, shared-resource and annotation/tag decisions".into()];
    if generated_font {
        reasons.push(if policy.allow_font_substitution { "approve the exact pinned shaping font used by the generated candidate".into() }
            else { "font substitution is disabled but the candidate writes new font definitions; replan with an explicit permitted font policy".into() });
    }
    let report = json!({"route":"native_scoped_text", "candidate_output_sha256":output_hash, "native":native_report,
        "font":preview["font"], "written_definitions":objects, "new_definitions":new_definitions,
        "invalidation":preview["invalidation"], "native_target_reuse":preview["native_target_reuse"],
        "qualification":"source_implementation_only; vps_corpus_gate_pending"});
    Ok(ScopedTextPlan {
        execution,
        state,
        candidate: Some(candidate),
        preview,
        implementation: json!({"route":"native_scoped_text", "private_candidate_recomputed_at_apply":true,
            "publish_same_staged_bytes_after_approval":true, "new_font_definition_gate_is_conservative":true,
            "pixel_comparison_performed":false, "qualification":"vps_corpus_gate_pending"}),
        read_set: vec![format!("input-sha256:{}", digest_hex(input))],
        write_set: objects.clone(),
        reasons,
        staged: Some(StagedScopedText {
            bytes,
            report,
            pages,
            objects,
            cloned_resources,
        }),
    })
}

fn contains_generated_font_marker(object: &crate::PdfObject, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    let dictionary_contains = |dictionary: &crate::PdfDictionary| {
        dictionary.get_name("WFAdvancedEditingGeneratedFont") == Some("V1")
            || dictionary
                .iter()
                .any(|(_, value)| contains_generated_font_marker(value, depth + 1))
    };
    match object {
        crate::PdfObject::Dictionary(dictionary) => dictionary_contains(dictionary),
        crate::PdfObject::Stream { dict, .. } => dictionary_contains(dict),
        crate::PdfObject::Array(values) => values
            .iter()
            .any(|value| contains_generated_font_marker(value, depth + 1)),
        _ => false,
    }
}

pub(super) fn approve(
    plan: &UniversalEditPlanV2,
    decision: &UniversalApprovalDecisionV2,
) -> Result<()> {
    if decision.selected_candidate_ids.len() != 1
        || decision.selected_candidate_ids != plan.selected_candidate_ids
    {
        return Err(invalid(
            "approval must select the exact source scope, including all widgets for a field transaction",
        ));
    }
    let required = plan
        .preview
        .pointer("/font/required_approved_font")
        .and_then(Value::as_str);
    if decision.approved_font.as_deref() != required {
        return Err(invalid("approval font must exactly match the candidate's required pinned font, or be omitted for unchanged source fonts"));
    }
    Ok(())
}
