//! HTTP routes for progressive render sessions.
//!
//! These endpoints expose the owned [`ProgressiveRenderJob`] lifecycle through a
//! server-managed session store. Each session holds a document engine and a
//! progressive render job whose tiles are rendered step-by-step.
//!
//! ## API
//!
//! POST /api/v1/progressive/start - Create a session, returns session_id + token
//! POST /api/v1/progressive/:id/step - Render the next batch of tiles
//! POST /api/v1/progressive/:id/pause - Pause the session
//! POST /api/v1/progressive/:id/resume - Resume the session with its token
//! POST /api/v1/progressive/:id/viewport - Revise visible-work priority
//! POST /api/v1/progressive/:id/dirty-region - Reschedule dirty tiles
//! POST /api/v1/progressive/:id/render-context - Revise render identity or render contract state
//! POST /api/v1/progressive/:id/apply-render-invalidation - Apply source-edit cache invalidation
//! POST /api/v1/progressive/:id/evaluate-publication - Accept/reject a tile publication
//! POST /api/v1/progressive/:id/queue/execute - Execute owned viewer queue work
//! POST /api/v1/progressive/:id/adjacent-prefetch/execute - Execute adjacent-page prefetch
//! POST /api/v1/progressive/:id/cancel - Cancel the session
//! POST /api/v1/progressive/:id/close - Close and release a session
//! GET  /api/v1/progressive/:id/status - Get current session status/token
//! GET  /api/v1/progressive/:id/queue - Get current viewer queue preview
//! GET  /api/v1/progressive/:id/callbacks - Get deterministic viewer callback dispatch plan
//! GET  /api/v1/progressive/:id/finish - Finish and download the composited PNG
//!
//! Sessions are subject to a configurable idle timeout; expired sessions are
//! automatically cancelled and removed.

use axum::{
    extract::{Multipart, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use wellfriendpdf_engine::{
    CancelToken, ContentEngine, ImageEncoder, ProgressiveRenderStepReport, ProgressiveRenderToken,
    ProgressiveTilePublication, RenderContract, RenderMode, RenderTile, WellfriendError,
};

use crate::auth::caller_identity;
use crate::error::{ServerError, ServerResult};
use crate::progressive_sessions::ProgressiveSessionStore;

/// Shared state for progressive-render routes (injected via Axum state).
#[derive(Clone)]
pub struct ProgressiveState {
    pub store: ProgressiveSessionStore,
}

// ---------- Request / Response types ----------

#[derive(Deserialize)]
pub struct StartParams {
    pub page: Option<usize>,
    pub dpi: Option<u32>,
    pub tile_size: Option<String>,
    pub tile_width: Option<u32>,
    pub tile_height: Option<u32>,
    pub render_mode: Option<String>,
    pub render_contract_json: Option<String>,
    pub viewport_hint_x: Option<u32>,
    pub viewport_hint_y: Option<u32>,
    pub viewport_hint_w: Option<u32>,
    pub viewport_hint_h: Option<u32>,
    #[serde(skip)]
    pub registered_fonts: Vec<(String, Vec<u8>)>,
}

#[derive(Deserialize)]
pub struct StepParams {
    pub max_tiles: Option<usize>,
}

#[derive(Deserialize)]
pub struct QueueExecuteParams {
    pub max_items: Option<usize>,
    pub max_tiles: Option<usize>,
}

#[derive(Deserialize)]
pub struct AdjacentPrefetchExecuteParams {
    pub prefetch_identity: String,
    pub max_tiles: Option<usize>,
}

#[derive(Deserialize)]
pub struct ResumeParams {
    pub token: ProgressiveRenderToken,
}

#[derive(Deserialize)]
pub struct ViewportParams {
    pub viewport_hint_x: Option<u32>,
    pub viewport_hint_y: Option<u32>,
    pub viewport_hint_w: Option<u32>,
    pub viewport_hint_h: Option<u32>,
}

#[derive(Deserialize)]
pub struct DirtyRegionParams {
    pub dirty_region_x: Option<u32>,
    pub dirty_region_y: Option<u32>,
    pub dirty_region_w: Option<u32>,
    pub dirty_region_h: Option<u32>,
}

#[derive(Deserialize)]
pub struct RenderContextParams {
    pub render_contract_json: Option<String>,
    pub contract_json: Option<String>,
    pub render_contract_fingerprint: Option<String>,
    pub visibility_fingerprint: Option<String>,
}

#[derive(Deserialize)]
pub struct EvaluatePublicationParams {
    pub publication: ProgressiveTilePublication,
}

#[derive(Serialize)]
pub struct StartResponse {
    pub session_id: String,
    pub token: ProgressiveRenderToken,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub session_id: String,
    pub state: String,
    pub token: ProgressiveRenderToken,
    pub viewer_queue_report: ProgressiveRenderStepReport,
}

// ---------- Handlers ----------

/// POST /api/v1/progressive/start
///
/// Multipart: file (PDF bytes) + JSON fields for page/dpi/tile size.
pub async fn start(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> ServerResult<Response> {
    let (pdf_bytes, params) = extract_start_fields(multipart).await?;

    let config = crate::config::get_config();
    if pdf_bytes.len() > config.max_file_size {
        return Err(ServerError::InvalidParameter(format!(
            "file too large: {} bytes (max {} bytes = {} MB)",
            pdf_bytes.len(),
            config.max_file_size,
            config.max_file_size / (1024 * 1024)
        )));
    }

    let page = params.page.unwrap_or(1);
    let dpi = params.dpi.unwrap_or(150);
    let (tile_width, tile_height) = resolve_tile_request(&params)?;
    let render_contract = params
        .render_contract_json
        .as_deref()
        .map(|json| {
            serde_json::from_str::<RenderContract>(json).map_err(|err| {
                ServerError::InvalidParameter(format!("render_contract_json is invalid: {err}"))
            })
        })
        .transpose()?;
    let page = render_contract
        .as_ref()
        .map(|contract| contract.page_number)
        .unwrap_or(page);
    let dpi = render_contract
        .as_ref()
        .map(|contract| contract.dpi)
        .unwrap_or(dpi);

    if dpi < 24 || dpi > config.max_dpi {
        return Err(ServerError::InvalidParameter(format!(
            "dpi must be between 24 and {}, got {}",
            config.max_dpi, dpi
        )));
    }
    if (tile_width == 0) != (tile_height == 0) {
        return Err(ServerError::InvalidParameter(
            "tile_width and tile_height must both be > 0, or both be 0 for adaptive".to_string(),
        ));
    }

    let render_mode = parse_start_render_mode(params.render_mode.as_deref())?;

    let viewport_hint = parse_viewport_hint(
        params.viewport_hint_x,
        params.viewport_hint_y,
        params.viewport_hint_w,
        params.viewport_hint_h,
    )?;

    let mut engine = ContentEngine::open_bytes(pdf_bytes.to_vec()).map_err(ServerError::from)?;
    register_uploaded_fonts(&mut engine, &params.registered_fonts)?;
    let viewport = match render_contract.as_ref() {
        Some(contract) => engine
            .page_viewport_for_box(contract.page_number, contract.dpi, contract.page_box)
            .map_err(ServerError::from)?,
        None => engine.page_viewport(page, dpi).map_err(ServerError::from)?,
    };
    crate::processing::check_render_pixels(config, page, viewport.width_px, viewport.height_px)?;

    let job = match render_contract {
        Some(contract) => engine
            .progressive_render_job_with_contract_and_viewport_hint(
                contract,
                tile_width,
                tile_height,
                viewport_hint,
            )
            .map_err(ServerError::from)?,
        None => engine
            .progressive_render_job_with_viewport_hint(
                page,
                dpi,
                tile_width,
                tile_height,
                render_mode,
                viewport_hint,
            )
            .map_err(ServerError::from)?,
    };

    let token = job.token();
    let session_id = state.store.insert(caller_identity(&headers), job)?;

    let resp = StartResponse { session_id, token };
    Ok((StatusCode::CREATED, Json(resp)).into_response())
}

/// POST /api/v1/progressive/:id/step
pub async fn step(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    body: Option<Json<StepParams>>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let max_tiles = body.as_ref().and_then(|b| b.max_tiles).unwrap_or(4);

    let report = state
        .store
        .with_session_mut(&session_id, &owner, |job| {
            job.render_next(max_tiles, &CancelToken::none())
        })?
        .map_err(ServerError::from)?;

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// POST /api/v1/progressive/:id/pause
pub async fn pause(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let token = state
        .store
        .with_session_mut(&session_id, &owner, |job| job.pause())?
        .map_err(ServerError::from)?;

    Ok((StatusCode::OK, Json(json!({ "token": token }))).into_response())
}

/// POST /api/v1/progressive/:id/resume
pub async fn resume(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(params): Json<ResumeParams>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let token = state
        .store
        .with_session_mut(&session_id, &owner, |job| {
            job.resume(&params.token)?;
            Ok::<ProgressiveRenderToken, WellfriendError>(job.token())
        })?
        .map_err(ServerError::from)?;

    Ok((StatusCode::OK, Json(json!({ "token": token }))).into_response())
}

/// POST /api/v1/progressive/:id/viewport
pub async fn revise_viewport(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(params): Json<ViewportParams>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let viewport_hint = parse_viewport_hint(
        params.viewport_hint_x,
        params.viewport_hint_y,
        params.viewport_hint_w,
        params.viewport_hint_h,
    )?;
    let report = state
        .store
        .revise_viewport_hint(&session_id, &owner, viewport_hint)?
        .map_err(ServerError::from)?;

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// POST /api/v1/progressive/:id/dirty-region
pub async fn revise_dirty_region(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(params): Json<DirtyRegionParams>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let dirty_region = parse_viewport_hint(
        params.dirty_region_x,
        params.dirty_region_y,
        params.dirty_region_w,
        params.dirty_region_h,
    )?;
    let report = state
        .store
        .revise_dirty_region(&session_id, &owner, dirty_region)?
        .map_err(ServerError::from)?;

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// POST /api/v1/progressive/:id/render-context
pub async fn revise_render_context(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(params): Json<RenderContextParams>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let RenderContextParams {
        render_contract_json,
        contract_json,
        render_contract_fingerprint,
        visibility_fingerprint,
    } = params;
    let contract_json = match (render_contract_json, contract_json) {
        (Some(render_contract_json), Some(contract_json))
            if render_contract_json != contract_json =>
        {
            return Err(ServerError::InvalidParameter(
                "render_contract_json and contract_json disagree".to_string(),
            ));
        }
        (Some(render_contract_json), _) => Some(render_contract_json),
        (None, Some(contract_json)) => Some(contract_json),
        (None, None) => None,
    };
    let report = if let Some(contract_json) = contract_json {
        if render_contract_fingerprint.is_some() || visibility_fingerprint.is_some() {
            return Err(ServerError::InvalidParameter(
                "render_contract_json cannot be combined with fingerprint-only render context fields"
                    .to_string(),
            ));
        }
        let contract = serde_json::from_str::<RenderContract>(&contract_json).map_err(|err| {
            ServerError::InvalidParameter(format!("render_contract_json is invalid: {err}"))
        })?;
        state
            .store
            .revise_render_contract(&session_id, &owner, contract)?
            .map_err(ServerError::from)?
    } else {
        state
            .store
            .revise_render_context(
                &session_id,
                &owner,
                render_contract_fingerprint,
                visibility_fingerprint,
            )?
            .map_err(ServerError::from)?
    };

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// POST /api/v1/progressive/:id/apply-render-invalidation
pub async fn apply_render_invalidation(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(body): Json<Value>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let plan_json = render_invalidation_plan_body_json(&body)?;
    let report = state
        .store
        .apply_render_invalidation_plan_json(&session_id, &owner, &plan_json)?
        .map_err(ServerError::from)?;

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// POST /api/v1/progressive/:id/evaluate-publication
pub async fn evaluate_publication(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(params): Json<EvaluatePublicationParams>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let report = state
        .store
        .evaluate_tile_publication(&session_id, &owner, &params.publication)?;

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// POST /api/v1/progressive/:id/cancel
pub async fn cancel(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    state.store.cancel(&session_id, &owner)?;

    Ok((StatusCode::OK, Json(json!({ "cancelled": true }))).into_response())
}

/// POST /api/v1/progressive/:id/close
pub async fn close(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    if !state.store.remove(&session_id, &owner) {
        return Err(ServerError::InvalidParameter(format!(
            "progressive session '{}' not found",
            session_id
        )));
    }

    Ok((StatusCode::OK, Json(json!({ "closed": true }))).into_response())
}

/// GET /api/v1/progressive/:id/status
pub async fn status(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let token = state
        .store
        .with_session(&session_id, &owner, |job| job.token())?;
    let viewer_queue_report = state.store.viewer_queue_report(&session_id, &owner)?;

    let resp = StatusResponse {
        session_id,
        state: token.lifecycle_state.clone(),
        token,
        viewer_queue_report,
    };
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// GET /api/v1/progressive/:id/queue
pub async fn queue(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let report = state.store.viewer_queue_report(&session_id, &owner)?;

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// POST /api/v1/progressive/:id/queue/execute
pub async fn execute_queue(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    body: Option<Json<QueueExecuteParams>>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let max_items = body
        .as_ref()
        .and_then(|b| b.max_items.or(b.max_tiles))
        .unwrap_or(4);
    let report = state
        .store
        .execute_viewer_queue(&session_id, &owner, max_items)?
        .map_err(ServerError::from)?;

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// POST /api/v1/progressive/:id/adjacent-prefetch/execute
pub async fn execute_adjacent_prefetch(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(params): Json<AdjacentPrefetchExecuteParams>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let report = state
        .store
        .execute_adjacent_page_prefetch(
            &session_id,
            &owner,
            &params.prefetch_identity,
            params.max_tiles.unwrap_or(1),
        )?
        .map_err(ServerError::from)?;

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// GET /api/v1/progressive/:id/callbacks
pub async fn callbacks(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let report = state
        .store
        .viewer_callback_dispatch_report(&session_id, &owner)?;

    Ok((StatusCode::OK, Json(report)).into_response())
}

/// GET /api/v1/progressive/:id/finish
///
/// Returns the composited PNG if the render is complete.
pub async fn finish_png(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    let buffer = state
        .store
        .with_session(&session_id, &owner, |job| job.finish_checked())?
        .map_err(|err| ServerError::InvalidParameter(err.to_string()))?;

    let raw = buffer.to_raw_image();
    let png_bytes = ImageEncoder::encode_png_fast(&raw)
        .map_err(|e| ServerError::Internal(format!("PNG encode failed: {}", e)))?;
    let _ = state.store.remove(&session_id, &owner);

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"progressive.png\""),
    );

    Ok((StatusCode::OK, headers, png_bytes).into_response())
}

// ---------- Field extraction ----------

async fn extract_start_fields(mut multipart: Multipart) -> ServerResult<(Vec<u8>, StartParams)> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut page: Option<usize> = None;
    let mut dpi: Option<u32> = None;
    let mut tile_size: Option<String> = None;
    let mut tile_width: Option<u32> = None;
    let mut tile_height: Option<u32> = None;
    let mut render_mode: Option<String> = None;
    let mut render_contract_json: Option<String> = None;
    let mut vh_x: Option<u32> = None;
    let mut vh_y: Option<u32> = None;
    let mut vh_w: Option<u32> = None;
    let mut vh_h: Option<u32> = None;
    let mut registered_fonts: Vec<(String, Vec<u8>)> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| ServerError::InvalidParameter(format!("multipart error: {}", err)))?
    {
        let name = field.name().map(str::to_owned);
        match name.as_deref() {
            Some("file") => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|err| ServerError::InvalidParameter(format!("{}", err)))?;
                file_bytes = Some(bytes.to_vec());
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
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|err| ServerError::InvalidParameter(format!("{}", err)))?;
                registered_fonts.push((font_name, bytes.to_vec()));
            }
            Some("page") => {
                let text = read_text_field(field).await?;
                page = Some(parse_usize_field(&text, "page")?);
            }
            Some("dpi") => {
                let text = read_text_field(field).await?;
                dpi = Some(parse_u32_field(&text, "dpi")?);
            }
            Some("tile_width") => {
                let text = read_text_field(field).await?;
                tile_width = Some(parse_u32_field(&text, "tile_width")?);
            }
            Some("tile_height") => {
                let text = read_text_field(field).await?;
                tile_height = Some(parse_u32_field(&text, "tile_height")?);
            }
            Some("tile_size") => {
                let text = read_text_field(field).await?;
                tile_size = Some(text.trim().to_string());
            }
            Some("render_mode") => {
                let text = read_text_field(field).await?;
                render_mode = Some(text.trim().to_string());
            }
            Some("render_contract_json") | Some("contract_json") => {
                let text = read_text_field(field).await?;
                render_contract_json = Some(text.trim().to_string());
            }
            Some("viewport_hint_x") => {
                let text = read_text_field(field).await?;
                vh_x = Some(parse_u32_field(&text, "viewport_hint_x")?);
            }
            Some("viewport_hint_y") => {
                let text = read_text_field(field).await?;
                vh_y = Some(parse_u32_field(&text, "viewport_hint_y")?);
            }
            Some("viewport_hint_w") => {
                let text = read_text_field(field).await?;
                vh_w = Some(parse_u32_field(&text, "viewport_hint_w")?);
            }
            Some("viewport_hint_h") => {
                let text = read_text_field(field).await?;
                vh_h = Some(parse_u32_field(&text, "viewport_hint_h")?);
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let pdf = file_bytes.ok_or(ServerError::MissingFile)?;

    Ok((
        pdf,
        StartParams {
            page,
            dpi,
            tile_size,
            tile_width,
            tile_height,
            render_mode,
            render_contract_json,
            viewport_hint_x: vh_x,
            viewport_hint_y: vh_y,
            viewport_hint_w: vh_w,
            viewport_hint_h: vh_h,
            registered_fonts,
        },
    ))
}

fn parse_start_render_mode(value: Option<&str>) -> ServerResult<RenderMode> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(RenderMode::Compat),
        Some(value) => RenderMode::from_name(&value.replace('_', "-")).ok_or_else(|| {
            ServerError::InvalidParameter(format!(
                "render_mode must be 'compat' or 'high_quality', got '{}'",
                value
            ))
        }),
    }
}

fn register_uploaded_fonts(
    engine: &mut ContentEngine,
    registered_fonts: &[(String, Vec<u8>)],
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
            .register_font_bytes(name.clone(), bytes.clone())
            .map_err(ServerError::from)?;
    }
    Ok(())
}

fn resolve_tile_request(params: &StartParams) -> ServerResult<(u32, u32)> {
    if let Some(tile_size) = params.tile_size.as_deref() {
        let normalized = tile_size.trim().to_ascii_lowercase();
        return match normalized.as_str() {
            "adaptive" => Ok((0, 0)),
            "128" | "192" | "256" | "384" | "512" => {
                let size = normalized.parse::<u32>().expect("validated tile size");
                Ok((size, size))
            }
            other => Err(ServerError::InvalidParameter(format!(
                "tile_size must be adaptive, 128, 192, 256, 384, or 512; got '{}'",
                other
            ))),
        };
    }

    Ok((
        params.tile_width.unwrap_or(256),
        params.tile_height.unwrap_or(256),
    ))
}

fn parse_viewport_hint(
    x: Option<u32>,
    y: Option<u32>,
    w: Option<u32>,
    h: Option<u32>,
) -> ServerResult<Option<RenderTile>> {
    match (x, y, w, h) {
        (Some(x), Some(y), Some(width), Some(height)) => Ok(Some(RenderTile {
            x,
            y,
            width,
            height,
        })),
        (None, None, None, None) => Ok(None),
        _ => Err(ServerError::InvalidParameter(
            "viewport_hint_x, viewport_hint_y, viewport_hint_w, and viewport_hint_h must be supplied together"
                .to_string(),
        )),
    }
}

fn render_invalidation_plan_body_json(value: &Value) -> ServerResult<String> {
    if let Some(plan_json) = value.as_str() {
        return Ok(plan_json.to_string());
    }
    if !value.is_object() {
        return Err(ServerError::InvalidParameter(
            "render invalidation body must be a JSON plan object, SDK envelope, or string field"
                .to_string(),
        ));
    }
    if let Some(plan_json) = value
        .get("render_invalidation_plan_json")
        .or_else(|| value.get("plan_json"))
        .and_then(Value::as_str)
    {
        return Ok(plan_json.to_string());
    }
    let plan_value = value
        .get("render_invalidation")
        .or_else(|| value.get("plan"))
        .unwrap_or(value);
    serde_json::to_string(plan_value).map_err(|err| {
        ServerError::InvalidParameter(format!(
            "render invalidation JSON serialization failed: {err}"
        ))
    })
}

async fn read_text_field(field: axum::extract::multipart::Field<'_>) -> ServerResult<String> {
    field
        .text()
        .await
        .map_err(|err| ServerError::InvalidParameter(format!("{}", err)))
}

fn parse_u32_field(value: &str, name: &str) -> ServerResult<u32> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|_| ServerError::InvalidParameter(format!("{} must be an unsigned integer", name)))
}

fn parse_usize_field(value: &str, name: &str) -> ServerResult<usize> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|_| ServerError::InvalidParameter(format!("{} must be an unsigned integer", name)))
}
