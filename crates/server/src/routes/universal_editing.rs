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
    password: Option<SensitiveBytes>,
    options_json: Option<String>,
    request_json: Option<String>,
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

async fn read_fields(mut multipart: Multipart) -> ServerResult<UniversalFields> {
    let mut fields = UniversalFields::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ServerError::InvalidParameter(format!("multipart error: {error}")))?
    {
        let name = field.name().map(str::to_owned);
        if name.as_deref() == Some("file") {
            fields.file = Some(field.bytes().await.map_err(|error| {
                ServerError::InvalidParameter(format!("failed to read file field: {error}"))
            })?);
            continue;
        }
        if matches!(
            name.as_deref(),
            Some("password") | Some("output_user_password") | Some("output_owner_password")
        ) {
            let value = field.bytes().await.map_err(|error| {
                ServerError::InvalidParameter(format!(
                    "failed to read credential field: {error}"
                ))
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
            file.len(), max_size
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

fn json_response(json: String) -> ServerResult<Response> {
    crate::processing::check_output_size(crate::config::get_config(), json.len())?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
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
        && haystack.windows(needle.len()).any(|window| window == needle)
}
