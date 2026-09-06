//! Image-decode capability and lifecycle endpoints.
//!
//! `POST /api/v1/image-decode/capability-report` exposes the per-document
//! metadata/region/reduction/progressive decoder capability report without
//! decoding image pixels.
//!
//! `POST /api/v1/progressive-image-decode/lifecycle-report` exposes the
//! binding-safe progressive image-decode lifecycle report for one discovered
//! image in a posted PDF.

use axum::extract::Multipart;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use serde_json::Value;
use wellfriendpdf_engine::sdk;

use crate::error::{ServerError, ServerResult};

#[derive(Default)]
struct ImageDecodeFields {
    file: Option<Bytes>,
    password: Option<String>,
    request_json: Option<String>,
}

pub async fn capability_report(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();

    let report_json = crate::processing::run_with_timeout(config, move |_cancel| {
        sdk::image_decode_capability_report_json(&file, Some(password.as_bytes()))
            .map_err(ServerError::from)
    })
    .await??;

    crate::processing::check_output_size(config, report_json.len())?;
    let value: Value = serde_json::from_str(&report_json).map_err(|err| {
        ServerError::Internal(format!(
            "image decode capability report JSON was invalid: {}",
            err
        ))
    })?;
    Ok((StatusCode::OK, Json(value)).into_response())
}

pub async fn progressive_lifecycle_report(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let request_json = fields.request_json.unwrap_or_else(|| "{}".to_string());
    let password = fields.password.unwrap_or_default();
    let config = crate::config::get_config();

    let report_json = crate::processing::run_with_timeout(config, move |_cancel| {
        sdk::progressive_image_decode_lifecycle_report_json(
            &file,
            &request_json,
            Some(password.as_bytes()),
        )
        .map_err(ServerError::from)
    })
    .await??;

    crate::processing::check_output_size(config, report_json.len())?;
    let value: Value = serde_json::from_str(&report_json).map_err(|err| {
        ServerError::Internal(format!(
            "progressive image decode lifecycle report JSON was invalid: {}",
            err
        ))
    })?;
    Ok((StatusCode::OK, Json(value)).into_response())
}

async fn read_fields(mut multipart: Multipart) -> ServerResult<ImageDecodeFields> {
    let mut fields = ImageDecodeFields::default();
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
            Some(unknown) => {
                let _ = field.bytes().await;
                tracing::debug!(
                    "image-decode endpoint: ignoring unknown field '{}'",
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

fn take_file(fields: &mut ImageDecodeFields) -> ServerResult<Bytes> {
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
