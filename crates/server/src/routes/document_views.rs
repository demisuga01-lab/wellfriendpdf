//! Canonical document-view boundary report endpoint.
//!
//! `POST /api/v1/document-views/report` exposes the same lazy view-boundary
//! report as the Rust SDK and language bindings without rendering pixels or
//! materializing semantic/validation views.

use axum::extract::Multipart;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use serde_json::Value;
use wellfriendpdf_engine::sdk;

use crate::error::{ServerError, ServerResult};

#[derive(Default)]
struct DocumentViewFields {
    file: Option<Bytes>,
    password: Option<String>,
}

pub async fn report(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();

    let report_json = crate::processing::run_with_timeout(config, move |_cancel| {
        sdk::document_views_report_json(&file, Some(password.as_bytes())).map_err(ServerError::from)
    })
    .await??;

    crate::processing::check_output_size(config, report_json.len())?;
    let value: Value = serde_json::from_str(&report_json).map_err(|err| {
        ServerError::Internal(format!("document views report JSON was invalid: {}", err))
    })?;
    Ok((StatusCode::OK, Json(value)).into_response())
}

async fn read_fields(mut multipart: Multipart) -> ServerResult<DocumentViewFields> {
    let mut fields = DocumentViewFields::default();
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
            Some(unknown) => {
                let _ = field.bytes().await;
                tracing::debug!(
                    "document-views endpoint: ignoring unknown field '{}'",
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

fn take_file(fields: &mut DocumentViewFields) -> ServerResult<Bytes> {
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
