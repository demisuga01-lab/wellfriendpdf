//! Prepress report endpoints.
//!
//! `POST /api/v1/prepress/plate-report` exposes the active render
//! interpreter's sparse Separation/DeviceN plate framebuffer report without
//! exporting production press surfaces or running corpus verification.

use axum::extract::Multipart;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use serde_json::Value;
use wellfriendpdf_engine::sdk;

use crate::error::{ServerError, ServerResult};

#[derive(Default)]
struct PrepressFields {
    file: Option<Bytes>,
    password: Option<String>,
    page: Option<String>,
    dpi: Option<String>,
}

pub async fn plate_report(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let password = fields.password.unwrap_or_default();
    let page = parse_optional_usize(fields.page.as_deref(), "page", 1)?;
    if page == 0 {
        return Err(ServerError::InvalidParameter(
            "page must be 1-based, got 0".to_string(),
        ));
    }
    let dpi = parse_optional_u32(fields.dpi.as_deref(), "dpi", 72)?;
    if dpi == 0 {
        return Err(ServerError::InvalidParameter(
            "dpi must be positive, got 0".to_string(),
        ));
    }
    let config = crate::config::get_config();
    if dpi > config.max_dpi {
        return Err(ServerError::InvalidParameter(format!(
            "dpi must be at most {}, got {}",
            config.max_dpi, dpi
        )));
    }

    let report_json = crate::processing::run_with_timeout(config, move |_cancel| {
        sdk::prepress_plate_report_json(&file, page, dpi, Some(password.as_bytes()))
            .map_err(ServerError::from)
    })
    .await??;

    crate::processing::check_output_size(config, report_json.len())?;
    let value: Value = serde_json::from_str(&report_json).map_err(|err| {
        ServerError::Internal(format!("prepress plate report JSON was invalid: {}", err))
    })?;
    Ok((StatusCode::OK, Json(value)).into_response())
}

async fn read_fields(mut multipart: Multipart) -> ServerResult<PrepressFields> {
    let mut fields = PrepressFields::default();
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
            Some("page") => fields.page = text!("page"),
            Some("dpi") => fields.dpi = text!("dpi"),
            Some(unknown) => {
                let _ = field.bytes().await;
                tracing::debug!("prepress endpoint: ignoring unknown field '{}'", unknown);
            }
            None => {
                let _ = field.bytes().await;
            }
        }
    }
    Ok(fields)
}

fn take_file(fields: &mut PrepressFields) -> ServerResult<Bytes> {
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

fn parse_optional_usize(
    raw: Option<&str>,
    name: &'static str,
    default: usize,
) -> ServerResult<usize> {
    raw.filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .trim()
                .parse::<usize>()
                .map_err(|err| ServerError::InvalidParameter(format!("invalid {name}: {err}")))
        })
        .unwrap_or(Ok(default))
}

fn parse_optional_u32(raw: Option<&str>, name: &'static str, default: u32) -> ServerResult<u32> {
    raw.filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .trim()
                .parse::<u32>()
                .map_err(|err| ServerError::InvalidParameter(format!("invalid {name}: {err}")))
        })
        .unwrap_or(Ok(default))
}
