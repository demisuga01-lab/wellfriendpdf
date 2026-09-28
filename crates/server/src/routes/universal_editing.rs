//! Universal editing v2 endpoints.

use axum::extract::Multipart;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use wellfriendpdf_engine::sdk;

use crate::error::{ServerError, ServerResult};

const MAX_CREDENTIAL_BYTES: usize = 4096;

#[derive(Default)]
struct SensitiveBytes(Vec<u8>);

impl SensitiveBytes {
    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl std::ops::Deref for SensitiveBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl Drop for SensitiveBytes {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

#[derive(Default)]
struct UniversalFields {
    file: Option<Bytes>,
    font: Option<Bytes>,
    password: Option<SensitiveBytes>,
    options_json: Option<String>,
    request_json: Option<String>,
    proposal_json: Option<String>,
    publication_receipt_json: Option<String>,
    authenticated_publication_receipt_json: Option<String>,
    plan_json: Option<String>,
    decision_json: Option<String>,
    approval_json: Option<String>,
    object_number: Option<String>,
    generation: Option<String>,
    output_user_password: Option<SensitiveBytes>,
    output_owner_password: Option<SensitiveBytes>,
}

pub async fn capabilities() -> ServerResult<Response> {
    json_response(sdk::universal_editing_capabilities_v2_json()?)
}

pub async fn analyze(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let password = fields.password.unwrap_or_default();
    let options = fields.options_json;
    let config = crate::config::get_config();
    let report = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            sdk::universal_editing_analyze_v2_json(
                &file,
                options.as_deref(),
                Some(password.as_slice()),
            )
            .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, report.len())?;
    json_response(report)
}

pub async fn render_qualification(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let password = fields.password.unwrap_or_default();
    let options = fields.options_json;
    let config = crate::config::get_config();
    let report = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            sdk::universal_render_qualification_v2_json(
                &file,
                options.as_deref(),
                Some(password.as_slice()),
            )
            .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, report.len())?;
    json_response(report)
}

pub async fn inspect_object(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let number = required(&fields.object_number, "object_number")?
        .parse::<u32>()
        .map_err(|_| ServerError::InvalidParameter("object_number must be u32".to_string()))?;
    let generation = fields
        .generation
        .as_deref()
        .unwrap_or("0")
        .parse::<u16>()
        .map_err(|_| ServerError::InvalidParameter("generation must be u16".to_string()))?;
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();
    let report = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            sdk::universal_editing_inspect_object_v2_json(
                &file,
                number,
                generation,
                Some(password.as_slice()),
            )
            .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, report.len())?;
    json_response(report)
}

pub async fn plan(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request = required(&fields.request_json, "request_json")?.to_string();
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();
    let report = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            sdk::universal_editing_plan_v2_json(&file, &request, Some(password.as_slice()))
                .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, report.len())?;
    json_response(report)
}

pub async fn scoped_preview(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let plan = required(&fields.plan_json, "plan_json")?.to_string();
    let options = fields.options_json;
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();
    let report = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            sdk::universal_editing_scoped_preview_v2_json(
                &file,
                &plan,
                options.as_deref(),
                Some(password.as_slice()),
            )
            .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, report.len())?;
    json_response(report)
}

/// Propose exact source-paint partitions without mutating the uploaded PDF.
pub async fn paint_partition_propose(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request = required(&fields.request_json, "request_json")?.to_string();
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();
    let report = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            sdk::advanced_editing_closeout_paint_partition_propose_json(
                &file,
                &request,
                Some(password.as_slice()),
            )
            .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, report.len())?;
    json_response(report)
}

/// Render a private before/candidate comparison for an exact reviewed
/// partition. Candidate PDF bytes are never returned by this endpoint.
pub async fn paint_partition_preview(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request = required(&fields.request_json, "request_json")?.to_string();
    let proposal = required(&fields.proposal_json, "proposal_json")?.to_string();
    let approval = required(&fields.approval_json, "approval_json")?.to_string();
    let font = fields.font;
    let options = fields.options_json;
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();
    let report = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            sdk::advanced_editing_closeout_paint_partition_preview_json(
                &file,
                &request,
                &proposal,
                &approval,
                font.as_deref(),
                options.as_deref(),
                Some(password.as_slice()),
            )
            .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, report.len())?;
    json_response(report)
}

/// Render the canonical private preview and add a short-lived HMAC-SHA-256
/// receipt issued by this server. The signing key is server-only and distinct
/// from both API keys and PDF encryption credentials.
pub async fn paint_partition_preview_authenticated(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request = required(&fields.request_json, "request_json")?.to_string();
    let proposal = required(&fields.proposal_json, "proposal_json")?.to_string();
    let approval = required(&fields.approval_json, "approval_json")?.to_string();
    let font = fields.font;
    let options = fields.options_json;
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();
    let signing_key = config.receipt_hmac_key.clone().ok_or_else(|| {
        ServerError::Unsupported(
            "authenticated receipt issuance requires WELLFRIENDPDF_RECEIPT_HMAC_KEY_HEX"
                .to_string(),
        )
    })?;
    let key_id = config.receipt_hmac_key_id.clone();
    let audience = config.receipt_hmac_audience.clone();
    let ttl = config.receipt_hmac_ttl_secs;
    let report = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            let report = sdk::advanced_editing_closeout_paint_partition_preview_json(
                &file,
                &request,
                &proposal,
                &approval,
                font.as_deref(),
                options.as_deref(),
                Some(password.as_slice()),
            )?;
            authenticate_preview_report(&report, &key_id, &audience, ttl, signing_key.as_slice())
                .map_err(wellfriendpdf_engine::WellfriendError::invalid_input)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, report.len())?;
    json_response(report)
}

/// Apply an explicit reviewed partition approval to the exact proposal/input.
pub async fn paint_partition_apply(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request = required(&fields.request_json, "request_json")?.to_string();
    let proposal = required(&fields.proposal_json, "proposal_json")?.to_string();
    let approval = required(&fields.approval_json, "approval_json")?.to_string();
    let font = fields.font;
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();
    let (document, report) = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            sdk::advanced_editing_closeout_paint_partition_apply_with_font_json(
                &file,
                &request,
                &proposal,
                &approval,
                font.as_deref(),
                Some(password.as_slice()),
            )
            .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, document.len())?;
    crate::processing::check_output_size(&config, report.len())?;
    multipart_response(&report, document)
}

/// Publish only the exact candidate covered by the canonical preview receipt.
pub async fn paint_partition_apply_reviewed(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request = required(&fields.request_json, "request_json")?.to_string();
    let proposal = required(&fields.proposal_json, "proposal_json")?.to_string();
    let approval = required(&fields.approval_json, "approval_json")?.to_string();
    let receipt =
        required(&fields.publication_receipt_json, "publication_receipt_json")?.to_string();
    let font = fields.font;
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();
    let (document, report) = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            sdk::advanced_editing_closeout_paint_partition_apply_reviewed_with_font_json(
                &file,
                &request,
                &proposal,
                &approval,
                &receipt,
                font.as_deref(),
                Some(password.as_slice()),
            )
            .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, document.len())?;
    crate::processing::check_output_size(&config, report.len())?;
    multipart_response(&report, document)
}

/// Publish only after both the engine's exact content binding and this host's
/// short-lived authenticated review receipt verify.
pub async fn paint_partition_apply_authenticated(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request = required(&fields.request_json, "request_json")?.to_string();
    let proposal = required(&fields.proposal_json, "proposal_json")?.to_string();
    let approval = required(&fields.approval_json, "approval_json")?.to_string();
    let authenticated_receipt = required(
        &fields.authenticated_publication_receipt_json,
        "authenticated_publication_receipt_json",
    )?
    .to_string();
    let font = fields.font;
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();
    let authenticated = serde_json::from_str::<
        wellfriendpdf_engine::universal_editing::scoped_preview::AuthenticatedPaintPartitionPublicationReceipt,
    >(&authenticated_receipt)
    .map_err(|error| {
        ServerError::InvalidParameter(format!(
            "invalid authenticated_publication_receipt_json: {error}"
        ))
    })?;
    let key_id = authenticated.key_id.clone();
    let signing_key = config
        .receipt_verification_key(&key_id)
        .ok_or(ServerError::ReceiptAuthentication)?;
    let audience = config.receipt_hmac_audience.clone();
    let (document, report) = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            let now = unix_time_now().map_err(ServerError::Internal)?;
            let receipt = wellfriendpdf_engine::universal_editing::scoped_preview::verify_authenticated_paint_partition_publication_receipt(
                &authenticated,
                &key_id,
                &audience,
                now,
                30,
                signing_key.as_slice(),
            )
            .map_err(|_| ServerError::ReceiptAuthentication)?;
            let receipt_json = serde_json::to_string(&receipt).map_err(|error| {
                ServerError::Internal(format!(
                    "could not serialize verified publication receipt: {error}"
                ))
            })?;
            sdk::advanced_editing_closeout_paint_partition_apply_reviewed_with_font_json(
                &file,
                &request,
                &proposal,
                &approval,
                &receipt_json,
                font.as_deref(),
                Some(password.as_slice()),
            )
            .map_err(ServerError::from)
        })
    })
    .await??;
    crate::processing::check_output_size(&config, document.len())?;
    crate::processing::check_output_size(&config, report.len())?;
    multipart_response(&report, document)
}

pub async fn approve(multipart: Multipart) -> ServerResult<Response> {
    let fields = read_fields(multipart).await?;
    let plan = required(&fields.plan_json, "plan_json")?;
    let decision = required(&fields.decision_json, "decision_json")?;
    let report = sdk::universal_editing_approval_v2_json(plan, decision)?;
    crate::processing::check_output_size(crate::config::get_config(), report.len())?;
    json_response(report)
}

pub async fn apply(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let plan = required(&fields.plan_json, "plan_json")?.to_string();
    let approval = fields.approval_json;
    let password = fields.password.unwrap_or_default();
    let output_user_password = fields.output_user_password;
    let output_owner_password = fields.output_owner_password;
    let config = crate::config::get_config();
    let (document, report) = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            if output_user_password.is_some() || output_owner_password.is_some() {
                let user = output_user_password.as_deref().unwrap_or_default();
                let owner = output_owner_password.as_deref().unwrap_or(user);
                sdk::universal_editing_apply_v2_with_output_credentials_json(
                    &file,
                    &plan,
                    approval.as_deref(),
                    Some(password.as_slice()),
                    user,
                    owner,
                )
                .map_err(ServerError::from)
            } else {
                sdk::universal_editing_apply_v2_json(
                    &file,
                    &plan,
                    approval.as_deref(),
                    Some(password.as_slice()),
                )
                .map_err(ServerError::from)
            }
        })
    })
    .await??;
    crate::processing::check_output_size(&config, document.len())?;
    crate::processing::check_output_size(&config, report.len())?;
    multipart_response(&report, document)
}

/// Materialize the caller's canonical universal-edit candidates from one
/// immutable revision, apply the ECBES evidence gates, and return only the
/// selected PDF (or the exact uploaded transport when no candidate qualifies).
pub async fn ecbes(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request = required(&fields.request_json, "request_json")?.to_string();
    let password = fields.password.unwrap_or_default();
    let output_user_password = fields.output_user_password;
    let output_owner_password = fields.output_owner_password;
    let config = crate::config::get_config();
    let (document, report) = crate::processing::run_with_timeout(&config, move |cancel| {
        cancel.scope(|| {
            if output_user_password.is_some() || output_owner_password.is_some() {
                let user = output_user_password.as_deref().unwrap_or_default();
                let owner = output_owner_password.as_deref().unwrap_or(user);
                sdk::ecbes_universal_edit_with_output_credentials_json(
                    &file,
                    &request,
                    Some(password.as_slice()),
                    user,
                    owner,
                )
                .map_err(ServerError::from)
            } else {
                sdk::ecbes_universal_edit_json(&file, &request, Some(password.as_slice()))
                    .map_err(ServerError::from)
            }
        })
    })
    .await??;
    crate::processing::check_output_size(&config, document.len())?;
    crate::processing::check_output_size(&config, report.len())?;
    multipart_response(&report, document)
}

async fn read_fields(mut multipart: Multipart) -> ServerResult<UniversalFields> {
    let mut fields = UniversalFields::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ServerError::InvalidParameter(format!("multipart error: {error}")))?
    {
        let name = field.name().map(str::to_owned);
        if matches!(
            name.as_deref(),
            Some("file") | Some("font") | Some("font_bytes")
        ) {
            let value = field.bytes().await.map_err(|error| {
                ServerError::InvalidParameter(format!("failed to read binary field: {error}"))
            })?;
            if name.as_deref() == Some("file") {
                fields.file = Some(value);
            } else {
                fields.font = Some(value);
            }
            continue;
        }
        if matches!(
            name.as_deref(),
            Some("password") | Some("output_user_password") | Some("output_owner_password")
        ) {
            let value = field.bytes().await.map_err(|error| {
                ServerError::InvalidParameter(format!("failed to read credential field: {error}"))
            })?;
            if value.len() > MAX_CREDENTIAL_BYTES {
                return Err(ServerError::InvalidParameter(format!(
                    "credential field exceeds {MAX_CREDENTIAL_BYTES} bytes"
                )));
            }
            let value = SensitiveBytes(value.to_vec());
            match name.as_deref() {
                Some("password") => fields.password = Some(value),
                Some("output_user_password") => fields.output_user_password = Some(value),
                Some("output_owner_password") => fields.output_owner_password = Some(value),
                _ => unreachable!("credential field name was matched above"),
            }
            continue;
        }
        let text = field.text().await.map_err(|error| {
            ServerError::InvalidParameter(format!("failed to read text field: {error}"))
        })?;
        match name.as_deref() {
            Some("options") | Some("options_json") => fields.options_json = Some(text),
            Some("request") | Some("request_json") => fields.request_json = Some(text),
            Some("proposal") | Some("proposal_json") => fields.proposal_json = Some(text),
            Some("publication_receipt") | Some("publication_receipt_json") => {
                fields.publication_receipt_json = Some(text)
            }
            Some("authenticated_publication_receipt")
            | Some("authenticated_publication_receipt_json") => {
                fields.authenticated_publication_receipt_json = Some(text)
            }
            Some("plan") | Some("plan_json") => fields.plan_json = Some(text),
            Some("decision") | Some("decision_json") => fields.decision_json = Some(text),
            Some("approval") | Some("approval_json") => fields.approval_json = Some(text),
            Some("object_number") | Some("number") => fields.object_number = Some(text),
            Some("generation") => fields.generation = Some(text),
            Some(unknown) => tracing::debug!(
                "universal-editing endpoint: ignoring unknown field '{}'",
                unknown
            ),
            None => {}
        }
    }
    Ok(fields)
}

fn take_file(fields: &mut UniversalFields) -> ServerResult<Bytes> {
    let file = fields.file.take().ok_or(ServerError::MissingFile)?;
    let max_size = crate::config::get_config().max_file_size;
    if file.len() > max_size {
        return Err(ServerError::InvalidParameter(format!(
            "file too large: {} bytes (max {})",
            file.len(),
            max_size
        )));
    }
    Ok(file)
}

fn required<'a>(value: &'a Option<String>, name: &str) -> ServerResult<&'a str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ServerError::InvalidParameter(format!("{name} is required")))
}

fn unix_time_now() -> Result<u64, String> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))
}

fn authenticate_preview_report(
    report_json: &str,
    key_id: &str,
    audience: &str,
    ttl_secs: u64,
    key: &[u8],
) -> Result<String, String> {
    let mut envelope: serde_json::Value = serde_json::from_str(report_json)
        .map_err(|error| format!("canonical preview returned invalid JSON: {error}"))?;
    let report = envelope
        .get_mut("report")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "canonical preview envelope has no report object".to_string())?;
    let receipt = report
        .get("publication_receipt")
        .cloned()
        .ok_or_else(|| "canonical preview report has no publication receipt".to_string())?;
    let receipt = serde_json::from_value::<
        wellfriendpdf_engine::universal_editing::scoped_preview::PaintPartitionPublicationReceipt,
    >(receipt)
    .map_err(|error| format!("canonical preview receipt is malformed: {error}"))?;
    let issued_at_unix = unix_time_now()?;
    let expires_at_unix = issued_at_unix
        .checked_add(ttl_secs)
        .ok_or_else(|| "authenticated receipt expiry overflow".to_string())?;
    let authenticated = wellfriendpdf_engine::universal_editing::scoped_preview::authenticate_paint_partition_publication_receipt(
        receipt,
        key_id,
        audience,
        issued_at_unix,
        expires_at_unix,
        key,
    )
    .map_err(|error| error.to_string())?;
    report.insert(
        "authenticated_publication_receipt".to_string(),
        serde_json::to_value(authenticated)
            .map_err(|error| format!("could not serialize authenticated receipt: {error}"))?,
    );
    serde_json::to_string(&envelope)
        .map_err(|error| format!("could not serialize authenticated preview: {error}"))
}

fn json_response(json: String) -> ServerResult<Response> {
    crate::processing::check_output_size(crate::config::get_config(), json.len())?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok((StatusCode::OK, headers, json).into_response())
}

fn multipart_response(report_json: &str, document: Vec<u8>) -> ServerResult<Response> {
    let boundary = choose_boundary(report_json.as_bytes(), &document)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&format!("multipart/mixed; boundary={boundary}"))
            .map_err(|error| ServerError::Internal(format!("invalid content type: {error}")))?,
    );
    let mut payload = Vec::with_capacity(report_json.len() + document.len() + 384);
    payload.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    payload.extend_from_slice(b"Content-Type: application/json\r\n");
    payload.extend_from_slice(b"Content-Disposition: form-data; name=\"metadata\"\r\n\r\n");
    payload.extend_from_slice(report_json.as_bytes());
    payload.extend_from_slice(b"\r\n");
    payload.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    payload.extend_from_slice(b"Content-Type: application/pdf\r\n");
    payload.extend_from_slice(
        b"Content-Disposition: form-data; name=\"document\"; filename=\"edited.pdf\"\r\n\r\n",
    );
    payload.extend_from_slice(&document);
    payload.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    crate::processing::check_output_size(crate::config::get_config(), payload.len())?;
    Ok((StatusCode::OK, headers, payload).into_response())
}

fn choose_boundary(report: &[u8], document: &[u8]) -> ServerResult<String> {
    for suffix in 0..32 {
        let candidate = format!("wellfriendpdf-universal-v2-{suffix}");
        if !contains(report, candidate.as_bytes()) && !contains(document, candidate.as_bytes()) {
            return Ok(candidate);
        }
    }
    Err(ServerError::Internal(
        "could not choose a safe universal editing multipart boundary".to_string(),
    ))
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[cfg(test)]
mod authenticated_receipt_tests {
    use super::*;

    #[test]
    fn authenticated_preview_report_round_trips_through_engine_verifier() {
        let receipt = serde_json::json!({
            "schema_version":"advanced_editing.paint-partition-publication-receipt.v1",
            "proposal_id":"proposal",
            "input_sha256":"1".repeat(64),
            "request_sha256":"2".repeat(64),
            "approval_sha256":"3".repeat(64),
            "font_sha256":null,
            "candidate_output_sha256":"4".repeat(64),
            "preview_evidence_sha256":"5".repeat(64),
            "receipt_id":"57fd8a15df7b47894fd7cfdb620b0e72b9133bc7096b3c45cf892ce3c36da060"
        });
        let envelope = serde_json::json!({
            "schema_version":1,
            "kind":"advanced_editing_closeout_paint_partition_preview",
            "report":{"publication_receipt":receipt}
        });
        let key = [0x44; 32];
        let signed = authenticate_preview_report(
            &envelope.to_string(),
            "primary",
            "wellfriendpdf-server",
            900,
            &key,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&signed).unwrap();
        let authenticated = serde_json::from_value::<
            wellfriendpdf_engine::universal_editing::scoped_preview::AuthenticatedPaintPartitionPublicationReceipt,
        >(value["report"]["authenticated_publication_receipt"].clone())
        .unwrap();
        let verified = wellfriendpdf_engine::universal_editing::scoped_preview::verify_authenticated_paint_partition_publication_receipt(
            &authenticated,
            "primary",
            "wellfriendpdf-server",
            authenticated.issued_at_unix,
            0,
            &key,
        )
        .unwrap();
        assert_eq!(verified.proposal_id, "proposal");
    }
}
