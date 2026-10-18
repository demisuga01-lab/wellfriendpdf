//! Native PDF/A conversion.
//!
//! `POST /api/v1/pdfa-convert` takes multipart `file` (required), `options`
//! (optional JSON object, the CLI `--options` shape, e.g. `{"level":"2b"}`)
//! and `password` (optional). Installed system fonts are never used over HTTP
//! so the server's output does not depend on the host's font directory.
//!
//! On success the response is the PDF/A file with `x-pdfa-level` and
//! `x-pdfa-validated: true`. When the converted file does not pass the PDF/A
//! validator the response is `422` with the JSON report
//! `{"ok":false,"code":"PDFA_CONVERSION_FAILED","failed_rules":[...]}`; no
//! file that claims PDF/A without passing is ever returned.

use axum::{
    extract::Multipart,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use wellfriendpdf_engine as engine;

use crate::{
    error::{ServerError, ServerResult},
    processing::ProcessedOutput,
};

pub async fn handler(mut multipart: Multipart) -> ServerResult<Response> {
    let mut file = None;
    let mut options = None;
    let mut password = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| ServerError::InvalidParameter(format!("multipart error: {err}")))?
    {
        match field.name() {
            Some("file") => {
                file = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|err| ServerError::InvalidParameter(err.to_string()))?,
                )
            }
            Some("options") => {
                options = Some(
                    field
                        .text()
                        .await
                        .map_err(|err| ServerError::InvalidParameter(err.to_string()))?,
                )
            }
            Some("password") => {
                password = Some(
                    field
                        .text()
                        .await
                        .map_err(|err| ServerError::InvalidParameter(err.to_string()))?,
                )
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }
    let file = file.ok_or(ServerError::MissingFile)?;
    let config = crate::config::get_config();
    if file.len() > config.max_file_size {
        return Err(ServerError::InvalidParameter(format!(
            "file too large: {} bytes (max {} bytes)",
            file.len(),
            config.max_file_size
        )));
    }
    let mut parsed: engine::PdfAConvertOptions = match options.as_deref() {
        Some(raw) if !raw.trim().is_empty() => serde_json::from_str(raw)
            .map_err(|err| ServerError::InvalidParameter(format!("invalid options: {err}")))?,
        _ => engine::PdfAConvertOptions::default(),
    };
    parsed.system_fonts = false;
    parsed.profile().map_err(ServerError::from)?;
    let max_output = config.max_output_bytes;
    let outcome =
        crate::processing::run_with_deadline_secs(config.request_timeout_secs, move |cancel| {
            cancel.scope(|| {
                let document = match password {
                    Some(pw) => {
                        engine::PdfDocument::open_bytes_with_password(file.to_vec(), pw.as_bytes())?
                    }
                    None => engine::PdfDocument::open_bytes(file.to_vec())?,
                };
                engine::convert_pdf_to_pdfa(&document, &parsed)
            })
        })
        .await?
        .map_err(ServerError::from)?;
    let Some(bytes) = outcome.bytes else {
        return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(outcome.report)).into_response());
    };
    if max_output > 0 && bytes.len() as u64 > max_output {
        return Err(ServerError::ResourceLimit(format!(
            "output is {} bytes (max {max_output})",
            bytes.len()
        )));
    }
    let processed = ProcessedOutput {
        bytes,
        content_type: "application/pdf",
        filename: "output.pdf",
        extra_headers: vec![
            ("x-pdfa-level", outcome.report.level.clone()),
            ("x-pdfa-validated", "true".to_string()),
            (
                "x-pdfa-fonts-embedded",
                outcome.report.fixes.fonts_embedded.to_string(),
            ),
        ],
    };
    Ok(crate::routes::pdf2img::output_to_response(processed))
}
