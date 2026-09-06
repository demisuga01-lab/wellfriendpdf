//! Editing transaction endpoints.
//!
//! `POST /api/v1/editing-transactions/apply-with-render-invalidation` applies
//! a source-backed text transaction and returns both edited PDF bytes and the
//! binding-safe render-invalidation plan emitted by the SDK.

use axum::extract::Multipart;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use wellfriendpdf_engine::sdk;

use crate::error::{ServerError, ServerResult};

#[derive(Default)]
struct EditingTransactionFields {
    file: Option<Bytes>,
    password: Option<String>,
    request_json: Option<String>,
    render_invalidation_options_json: Option<String>,
}

pub async fn apply_with_render_invalidation(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request_json = take_required_text(&fields.request_json, "request_json")?.to_string();
    let render_invalidation_options_json = fields.render_invalidation_options_json.clone();
    let password = fields.password.clone().unwrap_or_default();
    let config = crate::config::get_config();

    let (document, report_json) = crate::processing::run_with_timeout(config, move |_cancel| {
        sdk::editing_transactions_transaction_apply_with_render_invalidation_json(
            &file,
            &request_json,
            render_invalidation_options_json.as_deref(),
            Some(password.as_bytes()),
        )
        .map_err(ServerError::from)
    })
    .await??;

    crate::processing::check_output_size(config, document.len())?;
    crate::processing::check_output_size(config, report_json.len())?;
    multipart_response(&report_json, document)
}

async fn read_fields(mut multipart: Multipart) -> ServerResult<EditingTransactionFields> {
    let mut fields = EditingTransactionFields::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| ServerError::InvalidParameter(format!("multipart error: {}", err)))?
    {
        let name = field.name().map(str::to_owned);
        macro_rules! text {
            ($label:literal) => {
                Some(field.text().await.map_err(|err| {
                    ServerError::InvalidParameter(format!(
                        "failed to read {} field: {}",
                        $label, err
                    ))
                })?)
            };
        }
        match name.as_deref() {
            Some("file") => {
                fields.file = Some(field.bytes().await.map_err(|err| {
                    ServerError::InvalidParameter(format!("failed to read file field: {}", err))
                })?);
            }
            Some("password") => fields.password = text!("password"),
            Some("request_json") | Some("request") => fields.request_json = text!("request_json"),
            Some("render_invalidation_options_json") | Some("render_invalidation") => {
                fields.render_invalidation_options_json = text!("render_invalidation_options_json")
            }
            Some(unknown) => {
                let _ = field.bytes().await;
                tracing::debug!(
                    "editing-transactions endpoint: ignoring unknown field '{}'",
                    unknown
                );
            }
            None => {
                let _ = field.bytes().await;
            }
        }
    }
    Ok(fields)
}

fn take_file(fields: &mut EditingTransactionFields) -> ServerResult<Bytes> {
    let file = fields.file.take().ok_or(ServerError::MissingFile)?;
    let max_size = crate::config::get_config().max_file_size;
    if file.len() > max_size {
        return Err(ServerError::InvalidParameter(format!(
            "file too large: {} bytes (max {} bytes = {} MB)",
            file.len(),
            max_size,
            max_size / (1024 * 1024)
        )));
    }
    Ok(file)
}

fn take_required_text<'a>(value: &'a Option<String>, field: &str) -> ServerResult<&'a str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ServerError::InvalidParameter(format!("{} is required", field)))
}

fn multipart_response(report_json: &str, document: Vec<u8>) -> ServerResult<Response> {
    let boundary = choose_boundary(report_json.as_bytes(), &document)?;
    let content_type = format!("multipart/mixed; boundary={}", boundary);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .map_err(|err| ServerError::Internal(format!("invalid content type: {}", err)))?,
    );

    let mut payload =
        Vec::with_capacity(report_json.len() + document.len() + boundary.len() * 3 + 384);
    payload.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    payload.extend_from_slice(b"Content-Type: application/json\r\n");
    payload.extend_from_slice(b"Content-Disposition: form-data; name=\"metadata\"\r\n\r\n");
    payload.extend_from_slice(report_json.as_bytes());
    payload.extend_from_slice(b"\r\n");
    payload.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    payload.extend_from_slice(b"Content-Type: application/pdf\r\n");
    payload.extend_from_slice(
        b"Content-Disposition: form-data; name=\"document\"; filename=\"edited.pdf\"\r\n\r\n",
    );
    payload.extend_from_slice(&document);
    payload.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
    crate::processing::check_output_size(crate::config::get_config(), payload.len())?;

    Ok((StatusCode::OK, headers, payload).into_response())
}

fn choose_boundary(report_json: &[u8], document: &[u8]) -> ServerResult<String> {
    for suffix in 0..32 {
        let boundary = format!("wellfriendpdf-editing-transaction-{}", suffix);
        let bytes = boundary.as_bytes();
        if !contains_subslice(report_json, bytes) && !contains_subslice(document, bytes) {
            return Ok(boundary);
        }
    }
    Err(ServerError::Internal(
        "could not choose a multipart boundary absent from editing transaction body".to_string(),
    ))
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}
