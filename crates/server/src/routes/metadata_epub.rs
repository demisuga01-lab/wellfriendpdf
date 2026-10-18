//! Metadata editing and PDF to EPUB.
//!
//! - `POST /api/v1/edit-metadata`: multipart `file` (PDF) and `options` (JSON
//!   object, the CLI `--options` shape). Returns the PDF with an appended
//!   incremental revision; `x-signature-policy` carries the signature policy
//!   decision.
//! - `POST /api/v1/pdf-to-epub`: multipart `file` and optional `options`
//!   (`split`, `pages_per_chapter`, `include_images`, `include_furniture`,
//!   `language`, `title`, `password`). Returns `application/epub+zip`.

use axum::{body::Bytes, extract::Multipart, response::Response};
use wellfriendpdf_engine as engine;

use crate::{
    error::{ServerError, ServerResult},
    processing::ProcessedOutput,
};

async fn read_fields(mut multipart: Multipart) -> ServerResult<(Bytes, Option<String>)> {
    let mut file = None;
    let mut options = None;
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
                );
            }
            Some("options") => {
                options = Some(
                    field
                        .text()
                        .await
                        .map_err(|err| ServerError::InvalidParameter(err.to_string()))?,
                );
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }
    let file = file.ok_or(ServerError::MissingFile)?;
    let max_size = crate::config::get_config().max_file_size;
    if file.len() > max_size {
        return Err(ServerError::InvalidParameter(format!(
            "file too large: {} bytes (max {} bytes)",
            file.len(),
            max_size
        )));
    }
    Ok((file, options))
}

fn check_output(len: usize) -> ServerResult<()> {
    let max_output = crate::config::get_config().max_output_bytes;
    if max_output > 0 && len as u64 > max_output {
        return Err(ServerError::ResourceLimit(format!(
            "output is {len} bytes (max {max_output})"
        )));
    }
    Ok(())
}

/// `POST /api/v1/edit-metadata`
pub async fn edit_metadata(multipart: Multipart) -> ServerResult<Response> {
    let (file, options) = read_fields(multipart).await?;
    let options = options.ok_or_else(|| {
        ServerError::InvalidParameter("options (JSON metadata edit) is required".to_string())
    })?;
    let config = crate::config::get_config();
    let (pdf, report) =
        crate::processing::run_with_deadline_secs(config.request_timeout_secs, move |cancel| {
            cancel.scope(|| engine::metadata_edit::edit_metadata_json(&file, &options))
        })
        .await?
        .map_err(ServerError::from)?;
    check_output(pdf.len())?;
    let decision = serde_json::to_value(report.signature_policy_decision)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    Ok(crate::routes::pdf2img::output_to_response(
        ProcessedOutput {
            bytes: pdf,
            content_type: "application/pdf",
            filename: "output.pdf",
            extra_headers: vec![("x-signature-policy", decision)],
        },
    ))
}

/// `POST /api/v1/pdf-to-epub`
pub async fn pdf_to_epub(multipart: Multipart) -> ServerResult<Response> {
    let (file, options) = read_fields(multipart).await?;
    let config = crate::config::get_config();
    let (epub, report) =
        crate::processing::run_with_deadline_secs(config.request_timeout_secs, move |cancel| {
            cancel.scope(|| engine::epub::pdf_to_epub_json(&file, options.as_deref()))
        })
        .await?
        .map_err(ServerError::from)?;
    check_output(epub.len())?;
    Ok(crate::routes::pdf2img::output_to_response(
        ProcessedOutput {
            bytes: epub,
            content_type: "application/epub+zip",
            filename: "output.epub",
            extra_headers: vec![
                ("x-page-count", report.pages.to_string()),
                ("x-chapter-count", report.chapters.len().to_string()),
            ],
        },
    ))
}
