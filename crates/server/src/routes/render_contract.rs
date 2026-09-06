//! Render-contract builder endpoint.
//!
//! `POST /api/v1/render-contract` exposes the server-side equivalent of the
//! typed render-contract builders in the language bindings. It builds the
//! canonical schema-v1 contract for a PDF page, applies explicit field
//! overrides, validates the final contract, and returns the normalized JSON.
//!
//! `POST /api/v1/render-contract/png`, `/raw`, and the matching
//! font-substitution-report variants consume a posted `contract_json` and
//! render through the same core contract paths used by the Rust, C, Python,
//! WASM, .NET, and Java surfaces. Report variants return a `multipart/mixed`
//! response with JSON metadata/report first and rendered bytes second.
//!
//! `POST /api/v1/render-contract/backend-plan-arena-report` exposes the same
//! retained hot/cold backend-plan arena report used by the SDK and bindings.

use axum::extract::Multipart;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use serde::Serialize;
use wellfriendpdf_engine::render::{
    AlphaMode, AnnotationRenderPolicy, BackendSelection, ColorManagementPolicy, ColorScheme,
    CompositingPolicy, ContractColor, DeterminismPolicy, DeviceClip, DeviceMatrix, ExactnessPolicy,
    ExecutionMode, FormRenderPolicy, HalftonePolicy, OptionalContentStateId, OverprintPolicy,
    PageBox, PixelFormat, PrintProfile, RenderContract, RenderingIntent, SmoothingPolicy,
};
use wellfriendpdf_engine::{
    ContentEngine, FontSubstitutionLog, RenderContractTelemetryReport, RenderMode,
};

use crate::error::{ServerError, ServerResult};

#[derive(Default)]
struct ContractFields {
    file: Option<Bytes>,
    password: Option<String>,
    contract_json: Option<String>,
    registered_fonts: Vec<(String, Bytes)>,
    page: Option<String>,
    dpi: Option<String>,
    render_mode: Option<String>,
    page_box: Option<String>,
    pixel_format: Option<String>,
    alpha_mode: Option<String>,
    width: Option<String>,
    height: Option<String>,
    stride: Option<String>,
    grayscale: Option<String>,
    reverse_byte_order: Option<String>,
    without_clip: Option<String>,
    clip_x: Option<String>,
    clip_y: Option<String>,
    clip_width: Option<String>,
    clip_height: Option<String>,
    transform_a: Option<String>,
    transform_b: Option<String>,
    transform_c: Option<String>,
    transform_d: Option<String>,
    transform_e: Option<String>,
    transform_f: Option<String>,
    background_r: Option<String>,
    background_g: Option<String>,
    background_b: Option<String>,
    background_a: Option<String>,
    execution_mode: Option<String>,
    backend: Option<String>,
    compositing: Option<String>,
    annotations: Option<String>,
    forms: Option<String>,
    optional_content: Option<String>,
    smoothing: Option<String>,
    text_smoothing: Option<String>,
    image_smoothing: Option<String>,
    path_smoothing: Option<String>,
    subpixel_text: Option<String>,
    color_scheme: Option<String>,
    print_profile: Option<String>,
    halftone: Option<String>,
    overprint: Option<String>,
    rendering_intent: Option<String>,
    color_management: Option<String>,
    exactness: Option<String>,
    determinism: Option<String>,
    max_pixels: Option<String>,
    max_decoded_bytes: Option<String>,
    max_temporary_bytes: Option<String>,
    max_cache_bytes: Option<String>,
}

#[derive(Serialize)]
struct RenderContractResponse {
    schema_version: u32,
    contract: RenderContract,
    contract_json: String,
    surface_byte_length: usize,
    cache_fingerprint: String,
    builder: &'static str,
}

#[derive(Serialize)]
struct RenderContractReportMetadata<'a> {
    contract_schema_version: u32,
    page_number: usize,
    cache_fingerprint: &'a str,
    rendered_content_type: &'static str,
    body_part_name: &'static str,
    body_filename: &'static str,
    rendered_byte_length: usize,
    contract_surface_byte_length: usize,
    width: u32,
    height: u32,
    stride: usize,
    pixel_format: String,
    alpha_mode: String,
    font_substitution_report: &'a FontSubstitutionLog,
    render_telemetry_report: &'a RenderContractTelemetryReport,
}

pub async fn handler(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;

    let page = parse_optional_usize(fields.page.as_deref(), "page", 1)?;
    if page == 0 {
        return Err(ServerError::InvalidParameter(
            "page must be 1-based, got 0".to_string(),
        ));
    }

    let dpi = parse_optional_u32(fields.dpi.as_deref(), "dpi", 150)?;
    let config = crate::config::get_config();
    if dpi < 24 || dpi > config.max_dpi {
        return Err(ServerError::InvalidParameter(format!(
            "dpi must be between 24 and {}, got {}",
            config.max_dpi, dpi
        )));
    }

    let render_mode = parse_render_mode(fields.render_mode.as_deref())?;
    let page_box = parse_page_box(fields.page_box.as_deref())?;
    let password = fields.password.clone().unwrap_or_default();
    let fields_for_work = fields;

    let value = crate::processing::run_with_timeout(config, move |_cancel| {
        let engine = ContentEngine::open_bytes_with_password(file.to_vec(), password.as_bytes())
            .map_err(ServerError::from)?;
        let page_count = engine.page_count().map_err(ServerError::from)?;
        if page > page_count {
            return Err(ServerError::InvalidParameter(format!(
                "page {} exceeds document length ({})",
                page, page_count
            )));
        }

        let mut contract = engine
            .default_render_contract_for_page_box(page, dpi, render_mode, page_box)
            .map_err(ServerError::from)?;
        apply_overrides(&mut contract, &fields_for_work)?;
        contract.validate().map_err(ServerError::from)?;
        crate::processing::check_render_pixels(
            crate::config::get_config(),
            page,
            contract.width,
            contract.height,
        )?;
        let surface_byte_length = surface_byte_length(&contract)?;
        let cache_fingerprint = contract.cache_fingerprint();
        let contract_json = serde_json::to_string(&contract)
            .map_err(|err| ServerError::Internal(format!("serialize render contract: {}", err)))?;

        Ok::<RenderContractResponse, ServerError>(RenderContractResponse {
            schema_version: contract.schema_version,
            contract,
            contract_json,
            surface_byte_length,
            cache_fingerprint,
            builder: "server_render_contract_builder",
        })
    })
    .await??;

    Ok((StatusCode::OK, Json(value)).into_response())
}

pub async fn backend_plan_arena_report(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;

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

    let render_mode = parse_render_mode(fields.render_mode.as_deref())?;
    let password = fields.password.clone().unwrap_or_default();
    let contract_json = fields.contract_json.clone();

    let value = crate::processing::run_with_timeout(config, move |_cancel| {
        let json = if let Some(contract_json) = contract_json.as_deref() {
            wellfriendpdf_engine::sdk::backend_plan_arena_report_for_contract_json(
                file.as_ref(),
                contract_json,
                Some(password.as_bytes()),
            )
        } else {
            wellfriendpdf_engine::sdk::backend_plan_arena_report_json(
                file.as_ref(),
                page,
                dpi,
                Some(render_mode.as_str()),
                Some(password.as_bytes()),
            )
        }
        .map_err(ServerError::from)?;
        let value: serde_json::Value = serde_json::from_str(&json).map_err(|err| {
            ServerError::Internal(format!(
                "serialize backend plan arena report response: {}",
                err
            ))
        })?;
        Ok::<serde_json::Value, ServerError>(value)
    })
    .await??;

    Ok((StatusCode::OK, Json(value)).into_response())
}

pub async fn render_png(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let contract = take_contract_json(&fields)?;
    let password = fields.password.clone().unwrap_or_default();
    let registered_fonts = fields.registered_fonts.clone();
    let config = crate::config::get_config();

    validate_server_contract_bounds(config, &contract)?;

    let png = crate::processing::run_with_timeout(config, move |cancel| {
        let mut engine =
            ContentEngine::open_bytes_with_password(file.to_vec(), password.as_bytes())
                .map_err(ServerError::from)?;
        register_uploaded_fonts(&mut engine, &registered_fonts)?;
        engine
            .render_page_png_with_contract(&contract, &cancel)
            .map_err(ServerError::from)
    })
    .await??;

    crate::processing::check_output_size(config, png.len())?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    Ok((StatusCode::OK, headers, png).into_response())
}

pub async fn render_png_with_font_substitution_report(
    multipart: Multipart,
) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let contract = take_contract_json(&fields)?;
    let password = fields.password.clone().unwrap_or_default();
    let registered_fonts = fields.registered_fonts.clone();
    let config = crate::config::get_config();

    validate_server_contract_bounds(config, &contract)?;
    let cache_fingerprint = contract.cache_fingerprint();
    let contract_surface_len = surface_byte_length(&contract)?;
    let contract_schema_version = contract.schema_version;
    let page_number = contract.page_number;
    let width = contract.width;
    let height = contract.height;
    let stride = contract.stride;
    let pixel_format = format!("{:?}", contract.pixel_format);
    let alpha_mode = format!("{:?}", contract.alpha_mode);

    let (png, report, telemetry_report) =
        crate::processing::run_with_timeout(config, move |cancel| {
            let mut engine =
                ContentEngine::open_bytes_with_password(file.to_vec(), password.as_bytes())
                    .map_err(ServerError::from)?;
            register_uploaded_fonts(&mut engine, &registered_fonts)?;
            engine
                .render_page_png_with_contract_and_telemetry_report(&contract, &cancel)
                .map_err(ServerError::from)
        })
        .await??;

    crate::processing::check_output_size(config, png.len())?;
    let metadata = RenderContractReportMetadata {
        contract_schema_version,
        page_number,
        cache_fingerprint: &cache_fingerprint,
        rendered_content_type: "image/png",
        body_part_name: "image",
        body_filename: "page.png",
        rendered_byte_length: png.len(),
        contract_surface_byte_length: contract_surface_len,
        width,
        height,
        stride,
        pixel_format,
        alpha_mode,
        font_substitution_report: &report,
        render_telemetry_report: &telemetry_report,
    };
    multipart_render_report_response(
        config,
        HeaderMap::new(),
        &cache_fingerprint,
        &metadata,
        "image",
        "page.png",
        "image/png",
        png,
    )
}

pub async fn render_raw(multipart: Multipart) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let contract = take_contract_json(&fields)?;
    let password = fields.password.clone().unwrap_or_default();
    let registered_fonts = fields.registered_fonts.clone();
    let config = crate::config::get_config();

    validate_server_contract_bounds(config, &contract)?;
    let surface_len = surface_byte_length(&contract)?;
    crate::processing::check_output_size(config, surface_len)?;
    let headers = raw_surface_headers(&contract, surface_len)?;

    let bytes = crate::processing::run_with_timeout(config, move |cancel| {
        let mut engine =
            ContentEngine::open_bytes_with_password(file.to_vec(), password.as_bytes())
                .map_err(ServerError::from)?;
        register_uploaded_fonts(&mut engine, &registered_fonts)?;
        let mut output = vec![0_u8; surface_len];
        engine
            .render_page_into_buffer(&contract, &cancel, &mut output)
            .map_err(ServerError::from)?;
        Ok::<Vec<u8>, ServerError>(output)
    })
    .await??;

    Ok((StatusCode::OK, headers, bytes).into_response())
}

pub async fn render_raw_with_font_substitution_report(
    multipart: Multipart,
) -> ServerResult<Response> {
    let mut fields = read_fields(multipart).await?;
    let file = take_file(&mut fields)?;
    let contract = take_contract_json(&fields)?;
    let password = fields.password.clone().unwrap_or_default();
    let registered_fonts = fields.registered_fonts.clone();
    let config = crate::config::get_config();

    validate_server_contract_bounds(config, &contract)?;
    let surface_len = surface_byte_length(&contract)?;
    crate::processing::check_output_size(config, surface_len)?;
    let headers = raw_surface_headers(&contract, surface_len)?;
    let cache_fingerprint = contract.cache_fingerprint();
    let contract_schema_version = contract.schema_version;
    let page_number = contract.page_number;
    let width = contract.width;
    let height = contract.height;
    let stride = contract.stride;
    let pixel_format = format!("{:?}", contract.pixel_format);
    let alpha_mode = format!("{:?}", contract.alpha_mode);

    let (bytes, report, telemetry_report) =
        crate::processing::run_with_timeout(config, move |cancel| {
            let mut engine =
                ContentEngine::open_bytes_with_password(file.to_vec(), password.as_bytes())
                    .map_err(ServerError::from)?;
            register_uploaded_fonts(&mut engine, &registered_fonts)?;
            let mut output = vec![0_u8; surface_len];
            let report = engine
                .render_page_into_buffer_with_telemetry_report(&contract, &cancel, &mut output)
                .map_err(ServerError::from)?;
            Ok::<(Vec<u8>, FontSubstitutionLog, RenderContractTelemetryReport), ServerError>((
                output, report.0, report.1,
            ))
        })
        .await??;

    let metadata = RenderContractReportMetadata {
        contract_schema_version,
        page_number,
        cache_fingerprint: &cache_fingerprint,
        rendered_content_type: "application/octet-stream",
        body_part_name: "surface",
        body_filename: "page.raw",
        rendered_byte_length: bytes.len(),
        contract_surface_byte_length: surface_len,
        width,
        height,
        stride,
        pixel_format,
        alpha_mode,
        font_substitution_report: &report,
        render_telemetry_report: &telemetry_report,
    };
    multipart_render_report_response(
        config,
        headers,
        &cache_fingerprint,
        &metadata,
        "surface",
        "page.raw",
        "application/octet-stream",
        bytes,
    )
}

async fn read_fields(mut multipart: Multipart) -> ServerResult<ContractFields> {
    let mut f = ContractFields::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ServerError::InvalidParameter(format!("multipart error: {}", e)))?
    {
        let name = field.name().map(str::to_owned);
        macro_rules! text {
            ($label:literal) => {
                Some(field.text().await.map_err(|e| {
                    ServerError::InvalidParameter(format!("failed to read {} field: {}", $label, e))
                })?)
            };
        }
        match name.as_deref() {
            Some("file") => {
                f.file = Some(field.bytes().await.map_err(|e| {
                    ServerError::InvalidParameter(format!("failed to read file field: {}", e))
                })?);
            }
            Some(field_name) if field_name.starts_with("registered_font:") => {
                let font_name = field_name
                    .trim_start_matches("registered_font:")
                    .trim()
                    .to_string();
                if font_name.is_empty() {
                    return Err(ServerError::InvalidParameter(
                        "registered_font field name must include a non-empty font name".to_string(),
                    ));
                }
                let bytes = field.bytes().await.map_err(|e| {
                    ServerError::InvalidParameter(format!(
                        "failed to read registered font field: {}",
                        e
                    ))
                })?;
                f.registered_fonts.push((font_name, bytes));
            }
            Some("password") => f.password = text!("password"),
            Some("contract_json") | Some("contract") => f.contract_json = text!("contract_json"),
            Some("page") => f.page = text!("page"),
            Some("dpi") => f.dpi = text!("dpi"),
            Some("render_mode") | Some("mode") => f.render_mode = text!("render_mode"),
            Some("page_box") => f.page_box = text!("page_box"),
            Some("pixel_format") => f.pixel_format = text!("pixel_format"),
            Some("alpha_mode") => f.alpha_mode = text!("alpha_mode"),
            Some("width") => f.width = text!("width"),
            Some("height") => f.height = text!("height"),
            Some("stride") => f.stride = text!("stride"),
            Some("grayscale") => f.grayscale = text!("grayscale"),
            Some("reverse_byte_order") => f.reverse_byte_order = text!("reverse_byte_order"),
            Some("without_clip") => f.without_clip = text!("without_clip"),
            Some("clip_x") => f.clip_x = text!("clip_x"),
            Some("clip_y") => f.clip_y = text!("clip_y"),
            Some("clip_width") => f.clip_width = text!("clip_width"),
            Some("clip_height") => f.clip_height = text!("clip_height"),
            Some("transform_a") => f.transform_a = text!("transform_a"),
            Some("transform_b") => f.transform_b = text!("transform_b"),
            Some("transform_c") => f.transform_c = text!("transform_c"),
            Some("transform_d") => f.transform_d = text!("transform_d"),
            Some("transform_e") => f.transform_e = text!("transform_e"),
            Some("transform_f") => f.transform_f = text!("transform_f"),
            Some("background_r") => f.background_r = text!("background_r"),
            Some("background_g") => f.background_g = text!("background_g"),
            Some("background_b") => f.background_b = text!("background_b"),
            Some("background_a") => f.background_a = text!("background_a"),
            Some("execution_mode") => f.execution_mode = text!("execution_mode"),
            Some("backend") => f.backend = text!("backend"),
            Some("compositing") => f.compositing = text!("compositing"),
            Some("annotations") => f.annotations = text!("annotations"),
            Some("forms") => f.forms = text!("forms"),
            Some("optional_content") => f.optional_content = text!("optional_content"),
            Some("smoothing") => f.smoothing = text!("smoothing"),
            Some("text_smoothing") => f.text_smoothing = text!("text_smoothing"),
            Some("image_smoothing") => f.image_smoothing = text!("image_smoothing"),
            Some("path_smoothing") => f.path_smoothing = text!("path_smoothing"),
            Some("subpixel_text") => f.subpixel_text = text!("subpixel_text"),
            Some("color_scheme") => f.color_scheme = text!("color_scheme"),
            Some("print_profile") => f.print_profile = text!("print_profile"),
            Some("halftone") => f.halftone = text!("halftone"),
            Some("overprint") => f.overprint = text!("overprint"),
            Some("rendering_intent") => f.rendering_intent = text!("rendering_intent"),
            Some("color_management") => f.color_management = text!("color_management"),
            Some("exactness") => f.exactness = text!("exactness"),
            Some("determinism") => f.determinism = text!("determinism"),
            Some("max_pixels") => f.max_pixels = text!("max_pixels"),
            Some("max_decoded_bytes") => f.max_decoded_bytes = text!("max_decoded_bytes"),
            Some("max_temporary_bytes") => f.max_temporary_bytes = text!("max_temporary_bytes"),
            Some("max_cache_bytes") => f.max_cache_bytes = text!("max_cache_bytes"),
            Some(unknown) => {
                let _ = field.bytes().await;
                tracing::debug!(
                    "render-contract endpoint: ignoring unknown field '{}'",
                    unknown
                );
            }
            None => {
                let _ = field.bytes().await;
            }
        }
    }
    Ok(f)
}

fn take_file(f: &mut ContractFields) -> ServerResult<Bytes> {
    let file = f.file.take().ok_or(ServerError::MissingFile)?;
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

fn take_contract_json(fields: &ContractFields) -> ServerResult<RenderContract> {
    let json = fields
        .contract_json
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ServerError::InvalidParameter(
                "Request must include a 'contract_json' field".to_string(),
            )
        })?;
    let contract: RenderContract = serde_json::from_str(json).map_err(|err| {
        ServerError::InvalidParameter(format!("contract_json is not a render contract: {}", err))
    })?;
    contract.validate().map_err(ServerError::from)?;
    Ok(contract)
}

fn register_uploaded_fonts(
    engine: &mut ContentEngine,
    registered_fonts: &[(String, Bytes)],
) -> ServerResult<()> {
    let max_size = crate::config::get_config().max_file_size;
    for (name, bytes) in registered_fonts {
        if bytes.len() > max_size {
            return Err(ServerError::InvalidParameter(format!(
                "registered font '{}' is too large: {} bytes (max {})",
                name,
                bytes.len(),
                max_size
            )));
        }
        engine
            .register_font_bytes(name.clone(), bytes.to_vec())
            .map_err(ServerError::from)?;
    }
    Ok(())
}

fn validate_server_contract_bounds(
    config: &crate::config::ServerConfig,
    contract: &RenderContract,
) -> ServerResult<()> {
    contract.validate().map_err(ServerError::from)?;
    crate::processing::check_render_pixels(
        config,
        contract.page_number,
        contract.width,
        contract.height,
    )?;
    Ok(())
}

fn raw_surface_headers(contract: &RenderContract, surface_len: usize) -> ServerResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    insert_header(&mut headers, "x-render-contract-width", contract.width)?;
    insert_header(&mut headers, "x-render-contract-height", contract.height)?;
    insert_header(&mut headers, "x-render-contract-stride", contract.stride)?;
    insert_header(&mut headers, "x-render-contract-surface-bytes", surface_len)?;
    headers.insert(
        axum::http::HeaderName::from_static("x-render-contract-pixel-format"),
        HeaderValue::from_str(&format!("{:?}", contract.pixel_format)).map_err(|err| {
            ServerError::Internal(format!("invalid pixel-format header value: {}", err))
        })?,
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-render-contract-alpha-mode"),
        HeaderValue::from_str(&format!("{:?}", contract.alpha_mode)).map_err(|err| {
            ServerError::Internal(format!("invalid alpha-mode header value: {}", err))
        })?,
    );
    Ok(headers)
}

#[allow(clippy::too_many_arguments)]
fn multipart_render_report_response(
    config: &crate::config::ServerConfig,
    mut headers: HeaderMap,
    cache_fingerprint: &str,
    metadata: &RenderContractReportMetadata<'_>,
    body_part_name: &'static str,
    body_filename: &'static str,
    body_content_type: &'static str,
    body: Vec<u8>,
) -> ServerResult<Response> {
    let metadata_json = serde_json::to_vec(metadata).map_err(|err| {
        ServerError::Internal(format!("serialize render report metadata: {}", err))
    })?;
    let boundary = choose_multipart_boundary(cache_fingerprint, &[&metadata_json, &body])?;
    let content_type = format!("multipart/mixed; boundary={}", boundary);

    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type)
            .map_err(|err| ServerError::Internal(format!("invalid content type: {}", err)))?,
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-render-contract-cache-fingerprint"),
        HeaderValue::from_str(cache_fingerprint).map_err(|err| {
            ServerError::Internal(format!("invalid cache-fingerprint header value: {}", err))
        })?,
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-render-contract-report-part"),
        HeaderValue::from_static("metadata"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-render-contract-body-part"),
        HeaderValue::from_static(body_part_name),
    );

    let mut payload =
        Vec::with_capacity(metadata_json.len() + body.len() + boundary.len() * 3 + 384);
    payload.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    payload.extend_from_slice(b"Content-Type: application/json\r\n");
    payload.extend_from_slice(b"Content-Disposition: form-data; name=\"metadata\"\r\n\r\n");
    payload.extend_from_slice(&metadata_json);
    payload.extend_from_slice(b"\r\n");
    payload.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
    payload.extend_from_slice(format!("Content-Type: {}\r\n", body_content_type).as_bytes());
    payload.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\r\n",
            body_part_name, body_filename
        )
        .as_bytes(),
    );
    payload.extend_from_slice(&body);
    payload.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
    crate::processing::check_output_size(config, payload.len())?;

    Ok((StatusCode::OK, headers, payload).into_response())
}

fn choose_multipart_boundary(cache_fingerprint: &str, parts: &[&[u8]]) -> ServerResult<String> {
    let seed = cache_fingerprint.get(..16).unwrap_or(cache_fingerprint);
    for suffix in 0..32 {
        let boundary = format!("wellfriendpdf-render-report-{}-{}", seed, suffix);
        if parts
            .iter()
            .all(|part| !contains_subslice(part, boundary.as_bytes()))
        {
            return Ok(boundary);
        }
    }
    Err(ServerError::Internal(
        "could not choose a multipart boundary absent from render report body".to_string(),
    ))
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn insert_header(
    headers: &mut HeaderMap,
    name: &'static str,
    value: impl ToString,
) -> ServerResult<()> {
    headers.insert(
        axum::http::HeaderName::from_static(name),
        HeaderValue::from_str(&value.to_string())
            .map_err(|err| ServerError::Internal(format!("invalid header value: {}", err)))?,
    );
    Ok(())
}

fn apply_overrides(contract: &mut RenderContract, fields: &ContractFields) -> ServerResult<()> {
    apply_surface_overrides(contract, fields)?;
    apply_clip_override(contract, fields)?;
    apply_transform_override(contract, fields)?;
    apply_background_override(contract, fields)?;
    apply_policy_overrides(contract, fields)?;
    apply_budget_overrides(contract, fields)?;
    Ok(())
}

fn apply_surface_overrides(
    contract: &mut RenderContract,
    fields: &ContractFields,
) -> ServerResult<()> {
    let has_surface_override = fields.pixel_format.is_some()
        || fields.alpha_mode.is_some()
        || fields.width.is_some()
        || fields.height.is_some()
        || fields.stride.is_some()
        || fields.grayscale.is_some()
        || fields.reverse_byte_order.is_some();
    if !has_surface_override {
        return Ok(());
    }

    let width = parse_optional_u32_with_default(fields.width.as_deref(), "width", contract.width)?;
    let height =
        parse_optional_u32_with_default(fields.height.as_deref(), "height", contract.height)?;
    let pixel_format = match fields.pixel_format.as_deref().map(str::trim) {
        None | Some("") => contract.pixel_format,
        Some(value) => parse_pixel_format(value)?,
    };
    let alpha_mode = match fields.alpha_mode.as_deref().map(str::trim) {
        None | Some("") => contract.alpha_mode,
        Some(value) => parse_alpha_mode(value)?,
    };
    let default_stride = width as usize * pixel_format.bytes_per_pixel();
    let stride =
        parse_optional_usize_with_default(fields.stride.as_deref(), "stride", default_stride)?;
    let grayscale =
        crate::params::parse_bool_param(fields.grayscale.as_deref(), contract.grayscale)?;
    let reverse_byte_order = crate::params::parse_bool_param(
        fields.reverse_byte_order.as_deref(),
        contract.reverse_byte_order,
    )?;

    contract.width = width;
    contract.height = height;
    contract.pixel_format = pixel_format;
    contract.alpha_mode = alpha_mode;
    contract.stride = stride;
    contract.grayscale = grayscale;
    contract.reverse_byte_order = reverse_byte_order;
    Ok(())
}

fn apply_clip_override(contract: &mut RenderContract, fields: &ContractFields) -> ServerResult<()> {
    if crate::params::parse_bool_param(fields.without_clip.as_deref(), false)? {
        contract.clip = None;
        return Ok(());
    }

    let has_clip = fields.clip_x.is_some()
        || fields.clip_y.is_some()
        || fields.clip_width.is_some()
        || fields.clip_height.is_some();
    if !has_clip {
        return Ok(());
    }

    contract.clip = Some(DeviceClip {
        x: require_i32(fields.clip_x.as_deref(), "clip_x")?,
        y: require_i32(fields.clip_y.as_deref(), "clip_y")?,
        width: require_u32(fields.clip_width.as_deref(), "clip_width")?,
        height: require_u32(fields.clip_height.as_deref(), "clip_height")?,
    });
    Ok(())
}

fn apply_transform_override(
    contract: &mut RenderContract,
    fields: &ContractFields,
) -> ServerResult<()> {
    let has_transform = fields.transform_a.is_some()
        || fields.transform_b.is_some()
        || fields.transform_c.is_some()
        || fields.transform_d.is_some()
        || fields.transform_e.is_some()
        || fields.transform_f.is_some();
    if !has_transform {
        return Ok(());
    }

    contract.transform = DeviceMatrix::from_f64([
        require_f64(fields.transform_a.as_deref(), "transform_a")?,
        require_f64(fields.transform_b.as_deref(), "transform_b")?,
        require_f64(fields.transform_c.as_deref(), "transform_c")?,
        require_f64(fields.transform_d.as_deref(), "transform_d")?,
        require_f64(fields.transform_e.as_deref(), "transform_e")?,
        require_f64(fields.transform_f.as_deref(), "transform_f")?,
    ]);
    Ok(())
}

fn apply_background_override(
    contract: &mut RenderContract,
    fields: &ContractFields,
) -> ServerResult<()> {
    let has_background = fields.background_r.is_some()
        || fields.background_g.is_some()
        || fields.background_b.is_some()
        || fields.background_a.is_some();
    if !has_background {
        return Ok(());
    }

    contract.background = ContractColor {
        r: require_u8(fields.background_r.as_deref(), "background_r")?,
        g: require_u8(fields.background_g.as_deref(), "background_g")?,
        b: require_u8(fields.background_b.as_deref(), "background_b")?,
        a: parse_optional_u8_with_default(fields.background_a.as_deref(), "background_a", 255)?,
    };
    Ok(())
}

fn apply_policy_overrides(
    contract: &mut RenderContract,
    fields: &ContractFields,
) -> ServerResult<()> {
    if let Some(value) = optional_text(fields.execution_mode.as_deref()) {
        contract.execution_mode = parse_execution_mode(value)?;
    }
    if let Some(value) = optional_text(fields.backend.as_deref()) {
        contract.backend = parse_backend_selection(value)?;
    }
    if let Some(value) = optional_text(fields.compositing.as_deref()) {
        contract.compositing = parse_compositing_policy(value)?;
    }
    if let Some(value) = optional_text(fields.annotations.as_deref()) {
        contract.annotations = parse_annotation_policy(value)?;
    }
    if let Some(value) = optional_text(fields.forms.as_deref()) {
        contract.forms = parse_form_policy(value)?;
    }
    if let Some(value) = optional_text(fields.optional_content.as_deref()) {
        contract.optional_content = OptionalContentStateId(value.to_string());
    }
    if let Some(value) = optional_text(fields.smoothing.as_deref()) {
        let smoothing = parse_smoothing_policy(value)?;
        contract.text_smoothing = smoothing;
        contract.image_smoothing = smoothing;
        contract.path_smoothing = smoothing;
    }
    if let Some(value) = optional_text(fields.text_smoothing.as_deref()) {
        contract.text_smoothing = parse_smoothing_policy(value)?;
    }
    if let Some(value) = optional_text(fields.image_smoothing.as_deref()) {
        contract.image_smoothing = parse_smoothing_policy(value)?;
    }
    if let Some(value) = optional_text(fields.path_smoothing.as_deref()) {
        contract.path_smoothing = parse_smoothing_policy(value)?;
    }
    if let Some(value) = optional_text(fields.subpixel_text.as_deref()) {
        contract.subpixel_text = parse_smoothing_policy(value)?;
    }
    if let Some(value) = optional_text(fields.color_scheme.as_deref()) {
        contract.color_scheme = parse_color_scheme(value)?;
    }
    if let Some(value) = optional_text(fields.print_profile.as_deref()) {
        contract.print_profile = parse_print_profile(value)?;
    }
    if let Some(value) = optional_text(fields.halftone.as_deref()) {
        contract.halftone = parse_halftone_policy(value)?;
    }
    if let Some(value) = optional_text(fields.overprint.as_deref()) {
        contract.overprint = parse_overprint_policy(value)?;
    }
    if let Some(value) = optional_text(fields.rendering_intent.as_deref()) {
        contract.rendering_intent = parse_rendering_intent(value)?;
    }
    if let Some(value) = optional_text(fields.color_management.as_deref()) {
        contract.color_management = parse_color_management(value)?;
    }
    if let Some(value) = optional_text(fields.exactness.as_deref()) {
        if let Some(exactness) = parse_exactness_policy(value)? {
            contract.exactness = exactness;
        }
    }
    if let Some(value) = optional_text(fields.determinism.as_deref()) {
        contract.determinism = parse_determinism_policy(value)?;
    }
    Ok(())
}

fn apply_budget_overrides(
    contract: &mut RenderContract,
    fields: &ContractFields,
) -> ServerResult<()> {
    if let Some(value) = parse_optional_u64(fields.max_pixels.as_deref(), "max_pixels")? {
        contract.resource_budget.max_pixels = value;
    }
    if let Some(value) =
        parse_optional_u64(fields.max_decoded_bytes.as_deref(), "max_decoded_bytes")?
    {
        contract.resource_budget.max_decoded_bytes = value;
    }
    if let Some(value) =
        parse_optional_u64(fields.max_temporary_bytes.as_deref(), "max_temporary_bytes")?
    {
        contract.resource_budget.max_temporary_bytes = value;
    }
    if let Some(value) = parse_optional_u64(fields.max_cache_bytes.as_deref(), "max_cache_bytes")? {
        contract.resource_budget.max_cache_bytes = value;
    }
    Ok(())
}

fn parse_page_box(value: Option<&str>) -> ServerResult<PageBox> {
    match optional_text(value) {
        None => Ok(PageBox::Crop),
        Some(value) => match normalized_policy_token(value).as_str() {
            "media" => Ok(PageBox::Media),
            "crop" => Ok(PageBox::Crop),
            "bleed" => Ok(PageBox::Bleed),
            "trim" => Ok(PageBox::Trim),
            "art" => Ok(PageBox::Art),
            _ => Err(ServerError::InvalidParameter(format!(
                "page_box must be one of Media, Crop, Bleed, Trim, Art; got '{}'",
                value
            ))),
        },
    }
}

fn parse_render_mode(value: Option<&str>) -> ServerResult<RenderMode> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(RenderMode::Compat),
        Some(value) => RenderMode::from_name(value).ok_or_else(|| {
            ServerError::InvalidParameter(format!(
                "render_mode must be 'compat' or 'high-quality', got '{}'",
                value
            ))
        }),
    }
}

fn parse_execution_mode(value: &str) -> ServerResult<ExecutionMode> {
    match normalized_policy_token(value).as_str() {
        "standard" => Ok(ExecutionMode::Standard),
        "research" => Ok(ExecutionMode::Research),
        _ => Err(ServerError::InvalidParameter(format!(
            "execution_mode must be Standard or Research; got '{}'",
            value
        ))),
    }
}

fn parse_backend_selection(value: &str) -> ServerResult<BackendSelection> {
    match normalized_policy_token(value).as_str() {
        "scalar-reference" | "scalarreference" => Ok(BackendSelection::ScalarReference),
        "standard-cpu" | "standardcpu" => Ok(BackendSelection::StandardCpu),
        "research-hybrid" | "researchhybrid" => Ok(BackendSelection::ResearchHybrid),
        _ => Err(ServerError::InvalidParameter(format!(
            "backend must be ScalarReference, StandardCpu, or ResearchHybrid; got '{}'",
            value
        ))),
    }
}

fn parse_compositing_policy(value: &str) -> ServerResult<CompositingPolicy> {
    match normalized_policy_token(value).as_str() {
        "compatibility" => Ok(CompositingPolicy::Compatibility),
        "high-quality" | "highquality" => Ok(CompositingPolicy::HighQuality),
        _ => Err(ServerError::InvalidParameter(format!(
            "compositing must be Compatibility or HighQuality; got '{}'",
            value
        ))),
    }
}

fn parse_annotation_policy(value: &str) -> ServerResult<AnnotationRenderPolicy> {
    match normalized_policy_token(value).as_str() {
        "include" => Ok(AnnotationRenderPolicy::Include),
        "exclude" => Ok(AnnotationRenderPolicy::Exclude),
        _ => Err(ServerError::InvalidParameter(format!(
            "annotations must be Include or Exclude; got '{}'",
            value
        ))),
    }
}

fn parse_form_policy(value: &str) -> ServerResult<FormRenderPolicy> {
    match normalized_policy_token(value).as_str() {
        "include" => Ok(FormRenderPolicy::Include),
        "exclude" => Ok(FormRenderPolicy::Exclude),
        _ => Err(ServerError::InvalidParameter(format!(
            "forms must be Include or Exclude; got '{}'",
            value
        ))),
    }
}

fn parse_smoothing_policy(value: &str) -> ServerResult<SmoothingPolicy> {
    match normalized_policy_token(value).as_str() {
        "disabled" => Ok(SmoothingPolicy::Disabled),
        "antialiased" | "anti-aliased" => Ok(SmoothingPolicy::Antialiased),
        "subpixel" => Ok(SmoothingPolicy::Subpixel),
        _ => Err(ServerError::InvalidParameter(format!(
            "smoothing policy must be Disabled, Antialiased, or Subpixel; got '{}'",
            value
        ))),
    }
}

fn parse_color_scheme(value: &str) -> ServerResult<ColorScheme> {
    match normalized_policy_token(value).as_str() {
        "light" => Ok(ColorScheme::Light),
        "dark" => Ok(ColorScheme::Dark),
        "forced-monochrome" | "forcedmonochrome" => Ok(ColorScheme::ForcedMonochrome),
        _ => Err(ServerError::InvalidParameter(format!(
            "color_scheme must be Light, Dark, or ForcedMonochrome; got '{}'",
            value
        ))),
    }
}

fn parse_print_profile(value: &str) -> ServerResult<PrintProfile> {
    match normalized_policy_token(value).as_str() {
        "display" => Ok(PrintProfile::Display),
        "print" => Ok(PrintProfile::Print),
        "proof" => Ok(PrintProfile::Proof),
        _ => Err(ServerError::InvalidParameter(format!(
            "print_profile must be Display, Print, or Proof; got '{}'",
            value
        ))),
    }
}

fn parse_halftone_policy(value: &str) -> ServerResult<HalftonePolicy> {
    match normalized_policy_token(value).as_str() {
        "disabled" => Ok(HalftonePolicy::Disabled),
        "screen" => Ok(HalftonePolicy::Screen),
        _ => Err(ServerError::InvalidParameter(format!(
            "halftone must be Disabled or Screen; got '{}'",
            value
        ))),
    }
}

fn parse_overprint_policy(value: &str) -> ServerResult<OverprintPolicy> {
    match normalized_policy_token(value).as_str() {
        "disabled" => Ok(OverprintPolicy::Disabled),
        "preview" => Ok(OverprintPolicy::Preview),
        "preserve-separations" | "preserveseparations" => Ok(OverprintPolicy::PreserveSeparations),
        _ => Err(ServerError::InvalidParameter(format!(
            "overprint must be Disabled, Preview, or PreserveSeparations; got '{}'",
            value
        ))),
    }
}

fn parse_rendering_intent(value: &str) -> ServerResult<RenderingIntent> {
    match normalized_policy_token(value).as_str() {
        "relative-colorimetric" | "relativecolorimetric" => {
            Ok(RenderingIntent::RelativeColorimetric)
        }
        "absolute-colorimetric" | "absolutecolorimetric" => {
            Ok(RenderingIntent::AbsoluteColorimetric)
        }
        "perceptual" => Ok(RenderingIntent::Perceptual),
        "saturation" => Ok(RenderingIntent::Saturation),
        _ => Err(ServerError::InvalidParameter(format!(
            "rendering_intent must be RelativeColorimetric, AbsoluteColorimetric, Perceptual, or Saturation; got '{}'",
            value
        ))),
    }
}

fn parse_color_management(value: &str) -> ServerResult<ColorManagementPolicy> {
    match normalized_policy_token(value).as_str() {
        "portable-qcms" | "portableqcms" => Ok(ColorManagementPolicy::PortableQcms),
        "native-littlecms" | "nativelittlecms" => Ok(ColorManagementPolicy::NativeLittleCms),
        "deterministic-fallback" | "deterministicfallback" => {
            Ok(ColorManagementPolicy::DeterministicFallback)
        }
        _ => Err(ServerError::InvalidParameter(format!(
            "color_management must be PortableQcms, NativeLittleCms, or DeterministicFallback; got '{}'",
            value
        ))),
    }
}

fn parse_exactness_policy(value: &str) -> ServerResult<Option<ExactnessPolicy>> {
    match normalized_policy_token(value).as_str() {
        "auto" => Ok(None),
        "compatibility" => Ok(Some(ExactnessPolicy::Compatibility)),
        "high-quality-exact" | "highqualityexact" => Ok(Some(ExactnessPolicy::HighQualityExact)),
        _ => Err(ServerError::InvalidParameter(format!(
            "exactness must be Auto, Compatibility, or HighQualityExact; got '{}'",
            value
        ))),
    }
}

fn parse_determinism_policy(value: &str) -> ServerResult<DeterminismPolicy> {
    match normalized_policy_token(value).as_str() {
        "required" => Ok(DeterminismPolicy::Required),
        "best-effort-research" | "besteffortresearch" => Ok(DeterminismPolicy::BestEffortResearch),
        _ => Err(ServerError::InvalidParameter(format!(
            "determinism must be Required or BestEffortResearch; got '{}'",
            value
        ))),
    }
}

fn parse_pixel_format(value: &str) -> ServerResult<PixelFormat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "rgba" | "rgba8" => Ok(PixelFormat::Rgba8),
        "bgra" | "bgra8" => Ok(PixelFormat::Bgra8),
        "rgb" | "rgb8" => Ok(PixelFormat::Rgb8),
        "bgr" | "bgr8" => Ok(PixelFormat::Bgr8),
        "gray" | "grey" | "gray8" | "grey8" | "grayscale" => Ok(PixelFormat::Gray8),
        _ => Err(ServerError::InvalidParameter(format!(
            "pixel_format must be one of Rgba8, Bgra8, Rgb8, Bgr8, Gray8; got '{}'",
            value
        ))),
    }
}

fn parse_alpha_mode(value: &str) -> ServerResult<AlphaMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "premultiplied" | "premul" | "pre" => Ok(AlphaMode::Premultiplied),
        "straight" | "unpremultiplied" => Ok(AlphaMode::Straight),
        "opaque" => Ok(AlphaMode::Opaque),
        _ => Err(ServerError::InvalidParameter(format!(
            "alpha_mode must be one of Premultiplied, Straight, Opaque; got '{}'",
            value
        ))),
    }
}

fn optional_text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn normalized_policy_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['_', ' '], "-")
}

fn surface_byte_length(contract: &RenderContract) -> ServerResult<usize> {
    contract
        .stride
        .checked_mul(contract.height as usize)
        .ok_or_else(|| {
            ServerError::ResourceLimit("render contract surface byte length overflow".to_string())
        })
}

fn parse_optional_usize(value: Option<&str>, field: &str, default: usize) -> ServerResult<usize> {
    parse_optional_usize_with_default(value, field, default)
}

fn parse_optional_usize_with_default(
    value: Option<&str>,
    field: &str,
    default: usize,
) -> ServerResult<usize> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(default),
        Some(value) => value.parse::<usize>().map_err(|_| {
            ServerError::InvalidParameter(format!(
                "{} must be a non-negative integer, got '{}'",
                field, value
            ))
        }),
    }
}

fn parse_optional_u32(value: Option<&str>, field: &str, default: u32) -> ServerResult<u32> {
    parse_optional_u32_with_default(value, field, default)
}

fn parse_optional_u32_with_default(
    value: Option<&str>,
    field: &str,
    default: u32,
) -> ServerResult<u32> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(default),
        Some(value) => value.parse::<u32>().map_err(|_| {
            ServerError::InvalidParameter(format!(
                "{} must be a non-negative integer, got '{}'",
                field, value
            ))
        }),
    }
}

fn parse_optional_u64(value: Option<&str>, field: &str) -> ServerResult<Option<u64>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) => value.parse::<u64>().map(Some).map_err(|_| {
            ServerError::InvalidParameter(format!(
                "{} must be a non-negative integer, got '{}'",
                field, value
            ))
        }),
    }
}

fn parse_optional_u8_with_default(
    value: Option<&str>,
    field: &str,
    default: u8,
) -> ServerResult<u8> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(default),
        Some(value) => value.parse::<u8>().map_err(|_| {
            ServerError::InvalidParameter(format!(
                "{} must be an integer between 0 and 255, got '{}'",
                field, value
            ))
        }),
    }
}

fn require_u8(value: Option<&str>, field: &str) -> ServerResult<u8> {
    let value = required(value, field)?;
    value.parse::<u8>().map_err(|_| {
        ServerError::InvalidParameter(format!(
            "{} must be an integer between 0 and 255, got '{}'",
            field, value
        ))
    })
}

fn require_u32(value: Option<&str>, field: &str) -> ServerResult<u32> {
    let value = required(value, field)?;
    value.parse::<u32>().map_err(|_| {
        ServerError::InvalidParameter(format!(
            "{} must be a non-negative integer, got '{}'",
            field, value
        ))
    })
}

fn require_i32(value: Option<&str>, field: &str) -> ServerResult<i32> {
    let value = required(value, field)?;
    value.parse::<i32>().map_err(|_| {
        ServerError::InvalidParameter(format!("{} must be an integer, got '{}'", field, value))
    })
}

fn require_f64(value: Option<&str>, field: &str) -> ServerResult<f64> {
    let value = required(value, field)?;
    value.parse::<f64>().map_err(|_| {
        ServerError::InvalidParameter(format!("{} must be a number, got '{}'", field, value))
    })
}

fn required<'a>(value: Option<&'a str>, field: &str) -> ServerResult<&'a str> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ServerError::InvalidParameter(format!("{} is required", field)))
}
