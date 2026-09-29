//! Stage-separated in-process RAPTOR editing benchmark.
//!
//! The benchmark selects one unique, independently addressable ASCII word on
//! page one, plans an operator-preserving replacement, creates any required
//! approval, applies in the same retained process, then reopens and validates
//! the output. Plan, approval, apply and verification are timed separately so
//! subprocess startup and independent external validators are not disguised as
//! core edit latency.
//!
//! Usage:
//!   raptor_edit_benchmark <corpus-dir> [max-files=0] [output-dir]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use sha2::{Digest, Sha256};
use wellfriendpdf_engine::{
    apply_universal_edit_v2, create_universal_approval_token_v2, plan_universal_edit_v2,
    ContentEngine, SceneTextEditRequest, TrueEditingMode, UniversalApprovalDecisionV2,
    UniversalEditOperationV2, UniversalEditOutcomeV2, UniversalEditPolicyV2,
    UniversalEditRequestV2, UniversalMutationModeV2, UniversalPlanStateV2,
};

#[derive(Serialize)]
struct Observation {
    index: usize,
    path: String,
    relative_path: String,
    input_bytes: usize,
    input_sha256: String,
    source_text: String,
    replacement_text: String,
    target_discovery_ms: f64,
    plan_ms: f64,
    approval_ms: f64,
    apply_ms: f64,
    reopen_verify_ms: f64,
    verified_end_to_end_ms: f64,
    plan_state: UniversalPlanStateV2,
    candidates: usize,
    prepared_plan_cache_hit: bool,
    changed: bool,
    outcome: UniversalEditOutcomeV2,
    output_bytes: usize,
    output_sha256: String,
    replacement_observed_after_reopen: bool,
    source_count_before: usize,
    source_count_after: usize,
    replacement_count_after: usize,
}

#[derive(Serialize)]
struct Failure {
    path: String,
    stage: &'static str,
    elapsed_ms: f64,
    detail: String,
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn pdfs(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
            {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn emit_failure(path: &Path, stage: &'static str, started: Instant, detail: impl ToString) {
    println!(
        "{}",
        serde_json::to_string(&Failure {
            path: path.display().to_string(),
            stage,
            elapsed_ms: elapsed_ms(started),
            detail: detail.to_string(),
        })
        .expect("failure JSON serializes")
    );
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|character: char| !character.is_ascii_alphanumeric())
                .to_string()
        })
        .filter(|word| {
            (5..=24).contains(&word.len()) && word.bytes().all(|byte| byte.is_ascii_alphabetic())
        })
        .collect()
}

fn target(text: &str) -> Option<String> {
    let words = normalized_words(text);
    let mut counts = HashMap::<String, usize>::new();
    for word in &words {
        counts
            .entry(word.clone())
            .or_insert_with(|| text.matches(word).count());
    }
    words
        .into_iter()
        .filter(|word| counts.get(word) == Some(&1))
        .max_by_key(String::len)
}

fn count_occurrences(text: &str, expected: &str) -> usize {
    text.matches(expected).count()
}

fn replacement(source: &str) -> String {
    const ALPHABET: &[u8] = b"Verification";
    let mut result = String::with_capacity(source.len());
    for index in 0..source.len() {
        result.push(ALPHABET[index % ALPHABET.len()] as char);
    }
    if result == source {
        result.replace_range(0..1, if &result[0..1] == "Z" { "Y" } else { "Z" });
    }
    result
}

fn exact_candidate_id(plan: &wellfriendpdf_engine::UniversalEditPlanV2) -> Option<String> {
    let mut candidates = plan
        .candidates
        .iter()
        .filter(|candidate| candidate.exact)
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then_with(|| left.shared_resource.cmp(&right.shared_resource))
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    candidates
        .first()
        .map(|candidate| candidate.candidate_id.clone())
}

fn approved_font_for(
    plan: &wellfriendpdf_engine::UniversalEditPlanV2,
    candidate_id: &str,
) -> Option<String> {
    let candidate_font = plan
        .implementation_report
        .get("operation")
        .and_then(|value| value.get("font_substitution"))
        .and_then(|value| value.get("by_candidate"))
        .and_then(|value| value.get(candidate_id))
        .and_then(|value| value.get("approved_candidates"))
        .and_then(|value| value.get(0))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    candidate_font.or_else(|| {
        plan.preview
            .pointer("/font/required_approved_font")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let Some(root) = args.get(1).map(PathBuf::from) else {
        return Err("usage: raptor_edit_benchmark <corpus-dir> [max-files=0] [output-dir]".into());
    };
    let max_files = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let output_root = args.get(3).map(PathBuf::from);
    if let Some(root) = &output_root {
        std::fs::create_dir_all(root)?;
    }
    let mut paths = pdfs(&root)?;
    if max_files > 0 {
        paths.truncate(max_files);
    }

    for (index, path) in paths.into_iter().enumerate() {
        let one_based_index = index + 1;
        let total_start = Instant::now();
        let input = match std::fs::read(&path) {
            Ok(input) => input,
            Err(error) => {
                emit_failure(&path, "read", total_start, error);
                continue;
            }
        };
        let input_sha256 = digest(&input);

        let discovery_start = Instant::now();
        let engine = match ContentEngine::open_bytes(input.clone()) {
            Ok(engine) => engine,
            Err(error) => {
                emit_failure(&path, "target_open", total_start, error);
                continue;
            }
        };
        let before_text = match engine.get_page_text(1) {
            Ok(text) => text,
            Err(error) => {
                emit_failure(&path, "target_extract", total_start, error);
                continue;
            }
        };
        let Some(source_text) = target(&before_text) else {
            emit_failure(
                &path,
                "no_unique_ascii_page_one_word",
                total_start,
                "no unambiguous benchmark target",
            );
            continue;
        };
        let replacement_text = replacement(&source_text);
        let target_discovery_ms = elapsed_ms(discovery_start);

        let request = UniversalEditRequestV2 {
            operation: UniversalEditOperationV2::Text {
                request: SceneTextEditRequest {
                    requested_mode: TrueEditingMode::OperatorPreserving,
                    page: 1,
                    source_text: source_text.clone(),
                    replacement_text: replacement_text.clone(),
                    font_policy: "allow_substitute".to_string(),
                    ..SceneTextEditRequest::default()
                },
            },
            policy: UniversalEditPolicyV2 {
                mutation_mode: UniversalMutationModeV2::AuthorizedRewrite,
                ..UniversalEditPolicyV2::default()
            },
        };

        let plan_start = Instant::now();
        let plan = match plan_universal_edit_v2(&input, &request) {
            Ok(plan) => plan,
            Err(error) => {
                emit_failure(&path, "plan", total_start, error);
                continue;
            }
        };
        let plan_ms = elapsed_ms(plan_start);
        if !matches!(
            plan.state,
            UniversalPlanStateV2::Ready | UniversalPlanStateV2::ApprovalRequired
        ) {
            emit_failure(
                &path,
                "plan_not_applicable",
                total_start,
                format!("{:?}", plan.state),
            );
            continue;
        }

        let approval_start = Instant::now();
        let approval = if plan.state == UniversalPlanStateV2::ApprovalRequired {
            let Some(candidate_id) = exact_candidate_id(&plan) else {
                emit_failure(
                    &path,
                    "approval_required_without_exact_candidate",
                    total_start,
                    "approval-required plan has no exact source candidate",
                );
                continue;
            };
            let approved_font = approved_font_for(&plan, &candidate_id);
            match create_universal_approval_token_v2(
                &plan,
                UniversalApprovalDecisionV2 {
                    selected_candidate_ids: vec![candidate_id],
                    approved_font,
                    mutation_mode: plan.policy.mutation_mode,
                    accept_visual_change: true,
                    accept_signature_invalidation: true,
                },
            ) {
                Ok(approval) => Some(approval),
                Err(error) => {
                    emit_failure(&path, "approval", total_start, error);
                    continue;
                }
            }
        } else {
            None
        };
        let approval_ms = elapsed_ms(approval_start);

        let apply_start = Instant::now();
        let (output, report) = match apply_universal_edit_v2(&input, &plan, approval.as_ref()) {
            Ok(result) => result,
            Err(error) => {
                emit_failure(&path, "apply", total_start, error);
                continue;
            }
        };
        let apply_ms = elapsed_ms(apply_start);

        let verify_start = Instant::now();
        let reopened = match ContentEngine::open_bytes(output.clone()) {
            Ok(engine) => engine,
            Err(error) => {
                emit_failure(&path, "reopen", total_start, error);
                continue;
            }
        };
        let after_text = match reopened.get_page_text(1) {
            Ok(text) => text,
            Err(error) => {
                emit_failure(&path, "reopen_extract", total_start, error);
                continue;
            }
        };
        let reopen_verify_ms = elapsed_ms(verify_start);
        let source_count_before = count_occurrences(&before_text, &source_text);
        let source_count_after = count_occurrences(&after_text, &source_text);
        let replacement_count_after = count_occurrences(&after_text, &replacement_text);
        let replacement_observed_after_reopen =
            source_count_before == 1 && source_count_after == 0 && replacement_count_after >= 1;
        if let Some(root) = &output_root {
            let output_dir = root
                .join("files")
                .join(format!("{one_based_index:03}-{}", &input_sha256[..16]));
            std::fs::create_dir_all(&output_dir)?;
            std::fs::write(output_dir.join("edited.pdf"), &output)?;
        }
        let prepared_plan_cache_hit = report
            .operation_report
            .pointer("/raptor_prepared_plan/cache_hit")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        println!(
            "{}",
            serde_json::to_string(&Observation {
                index: one_based_index,
                path: path.display().to_string(),
                relative_path: path
                    .strip_prefix(&root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                input_bytes: input.len(),
                input_sha256,
                source_text,
                replacement_text,
                target_discovery_ms,
                plan_ms,
                approval_ms,
                apply_ms,
                reopen_verify_ms,
                verified_end_to_end_ms: elapsed_ms(total_start),
                plan_state: plan.state,
                candidates: plan.candidates.len(),
                prepared_plan_cache_hit,
                changed: report.changed,
                outcome: report.outcome,
                output_bytes: output.len(),
                output_sha256: digest(&output),
                replacement_observed_after_reopen,
                source_count_before,
                source_count_after,
                replacement_count_after,
            })?
        );
    }
    Ok(())
}
