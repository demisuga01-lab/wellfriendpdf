//! Rust-native text, Markdown, CSV, JSON and SVG to PDF.
//!
//! Each route takes multipart `file` (required) and `options` (optional JSON
//! object, the same shape as the CLI `--options`) and returns the PDF. The
//! response carries `x-page-count`. Converter-specific failures keep their
//! stable code as the message prefix (for example `csv_column_limit_exceeded:`).
//! The server never reads local files: Markdown `base_dir` is rejected.

use axum::{body::Bytes, extract::Multipart, response::Response};
use wellfriendpdf_engine as engine;

use crate::{
    error::{ServerError, ServerResult},
    processing::ProcessedOutput,
};

#[derive(Clone, Copy)]
enum Converter {
    Text,
    Markdown,
    Csv,
    Json,
    Svg,
}

pub async fn text_to_pdf(multipart: Multipart) -> ServerResult<Response> {
    handle(multipart, Converter::Text).await
}

pub async fn markdown_to_pdf(multipart: Multipart) -> ServerResult<Response> {
    handle(multipart, Converter::Markdown).await
}

pub async fn csv_to_pdf(multipart: Multipart) -> ServerResult<Response> {
    handle(multipart, Converter::Csv).await
}

pub async fn json_to_pdf(multipart: Multipart) -> ServerResult<Response> {
    handle(multipart, Converter::Json).await
}

pub async fn svg_to_pdf(multipart: Multipart) -> ServerResult<Response> {
    handle(multipart, Converter::Svg).await
}

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

fn convert(
    converter: Converter,
    input: &[u8],
    options: Option<&str>,
) -> engine::Result<engine::NativeConversionOutput> {
    match converter {
        Converter::Text => engine::text_to_pdf_with_report(
            input,
            &engine::parse_native_options::<engine::TextToPdfOptions>(options)?,
        ),
        Converter::Markdown => {
            let parsed = engine::parse_native_options::<engine::MarkdownToPdfOptions>(options)?;
            if parsed.base_dir.is_some() {
                return Err(engine::WellfriendError::invalid_input(
                    "invalid_options: base_dir is not accepted over HTTP; embed images as data URIs",
                ));
            }
            engine::markdown_to_pdf_with_report(input, &parsed)
        }
        Converter::Csv => engine::csv_to_pdf_with_report(
            input,
            &engine::parse_native_options::<engine::CsvToPdfOptions>(options)?,
        ),
        Converter::Json => engine::json_to_pdf_with_report(
            input,
            &engine::parse_native_options::<engine::JsonToPdfOptions>(options)?,
        ),
        Converter::Svg => engine::svg_to_pdf_with_report(
            input,
            &engine::parse_native_options::<engine::SvgToPdfOptions>(options)?,
        ),
    }
}

async fn handle(multipart: Multipart, converter: Converter) -> ServerResult<Response> {
    let (file, options) = read_fields(multipart).await?;
    let config = crate::config::get_config();
    let max_output = config.max_output_bytes;
    let output =
        crate::processing::run_with_deadline_secs(config.request_timeout_secs, move |cancel| {
            cancel.scope(|| convert(converter, &file, options.as_deref()))
        })
        .await?
        .map_err(ServerError::from)?;
    if max_output > 0 && output.pdf.len() as u64 > max_output {
        return Err(ServerError::ResourceLimit(format!(
            "output is {} bytes (max {max_output})",
            output.pdf.len()
        )));
    }
    let processed = ProcessedOutput {
        bytes: output.pdf,
        content_type: "application/pdf",
        filename: "output.pdf",
        extra_headers: vec![("x-page-count", output.report.pages.to_string())],
    };
    Ok(crate::routes::pdf2img::output_to_response(processed))
}
