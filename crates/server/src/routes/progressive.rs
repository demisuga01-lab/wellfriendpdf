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
//! POST /api/v1/progressive/:id/cancel - Cancel the session
//! POST /api/v1/progressive/:id/close - Close and release a session
//! GET  /api/v1/progressive/:id/status - Get current session status/token
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
use serde_json::json;

use wellfriendpdf_engine::{
    CancelToken, ContentEngine, ImageEncoder, ProgressiveRenderToken, RenderMode, RenderTile,
    WellfriendError,
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
    pub viewport_hint_x: Option<u32>,
    pub viewport_hint_y: Option<u32>,
    pub viewport_hint_w: Option<u32>,
    pub viewport_hint_h: Option<u32>,
}

#[derive(Deserialize)]
pub struct StepParams {
    pub max_tiles: Option<usize>,
}

#[derive(Deserialize)]
pub struct ResumeParams {
    pub token: ProgressiveRenderToken,
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

    let render_mode = match params.render_mode.as_deref() {
        None | Some("") | Some("compat") => RenderMode::Compat,
        Some("high_quality") | Some("high-quality") => RenderMode::HighQuality,
        Some(other) => {
            return Err(ServerError::InvalidParameter(format!(
                "render_mode must be 'compat' or 'high_quality', got '{}'",
                other
            )));
        }
    };

    let viewport_hint = match (
        params.viewport_hint_x,
        params.viewport_hint_y,
        params.viewport_hint_w,
        params.viewport_hint_h,
    ) {
        (Some(x), Some(y), Some(w), Some(h)) => Some(RenderTile {
            x,
            y,
            width: w,
            height: h,
        }),
        _ => None,
    };

    let engine = ContentEngine::open_bytes(pdf_bytes.to_vec()).map_err(ServerError::from)?;
    let viewport = engine.page_viewport(page, dpi).map_err(ServerError::from)?;
    crate::processing::check_render_pixels(config, page, viewport.width_px, viewport.height_px)?;

    let job = engine
        .progressive_render_job_with_viewport_hint(
            page,
            dpi,
            tile_width,
            tile_height,
            render_mode,
            viewport_hint,
        )
        .map_err(ServerError::from)?;

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

/// POST /api/v1/progressive/:id/cancel
pub async fn cancel(
    State(state): State<ProgressiveState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> ServerResult<Response> {
    let owner = caller_identity(&headers);
    state.store.with_session_mut(&session_id, &owner, |job| {
        job.cancel();
    })?;

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

    let resp = StatusResponse {
        session_id,
        state: token.lifecycle_state.clone(),
        token,
    };
    Ok((StatusCode::OK, Json(resp)).into_response())
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
        .with_session(&session_id, &owner, |job| job.finish())?;

    let buffer = match buffer {
        Some(buf) => buf,
        None => {
            return Err(ServerError::InvalidParameter(
                "progressive render is not yet complete; keep stepping or check status".to_string(),
            ));
        }
    };

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
    let mut vh_x: Option<u32> = None;
    let mut vh_y: Option<u32> = None;
    let mut vh_w: Option<u32> = None;
    let mut vh_h: Option<u32> = None;

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
            viewport_hint_x: vh_x,
            viewport_hint_y: vh_y,
            viewport_hint_w: vh_w,
            viewport_hint_h: vh_h,
        },
    ))
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
