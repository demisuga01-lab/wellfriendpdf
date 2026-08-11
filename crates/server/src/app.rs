use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderName, HeaderValue, Method};
use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

use crate::config::{get_config, ServerConfig};
use crate::jobs::JobsState;
use crate::progressive_sessions::ProgressiveSessionStore;
use crate::rate_limit::RateLimiter;
use crate::routes;

// -- Wellfriend HTTP API v1 -----------------------------------------------------
//
// POST /api/v1/extract-text
//   Extract plain text from a PDF.
//   Fields: file, pages, page_markers, preserve_layout, output_format.
//
// POST /api/v1/extract-images
//   Extract embedded images from a PDF as a ZIP archive.
//   Fields: file, pages, format, quality, min_width, min_height,
//           include_masks, include_inline, output_format.
//
// POST /api/v1/analyze
//   Analyze whether a PDF has a real text layer.
//   Fields: file.
//
// POST /api/v1/pdf2img
//   Render PDF pages to PNG or JPEG images as a ZIP archive.
//   Fields: file, pages, dpi (24-600), format (png/jpg), quality.
//
// POST /api/v1/parse
//   Parse a PDF into the canonical document model, serialized as Markdown /
//   JSON / HTML (the same schema the CLI `parse` and the bindings emit).
//   Fields: file, pages, format (markdown/json/html), password.
//
// POST /api/v1/chunk
//   Split a PDF into RAG-ready semantic chunks (JSON).
//   Fields: file, pages, target_tokens, overlap, keep_furniture, password.
//
// POST /api/v1/extract-fields
//   Extract structured key-value fields (JSON).
//   Fields: file, pages, doc_type (auto/invoice/receipt/form/generic), password.
//
// POST /api/v1/info
//   Document metadata, pdfinfo-style (JSON). Fields: file, password.
//
//   These parser endpoints are digital-born only: OCR is not performed
//   server-side (no Tesseract dependency). Use /api/v1/analyze to detect
//   scanned input.
//
// --- Async job API (for large/slow inputs; additive to the sync endpoints) ---
//
// POST /api/v1/jobs/pdf2img
// POST /api/v1/jobs/extract-images
//   Submit a background job. Same multipart fields as the sync endpoint.
//   Returns 202 Accepted with { job_id, status, status_url, result_url }.
//
// GET /api/v1/jobs/{id}
//   Poll job status: queued / running / completed / failed (+ progress).
//
// GET /api/v1/jobs/{id}/result
//   Download a completed job's output. 409 if not ready; 404 if unknown,
//   expired, or owned by another caller.
//
// GET /api/v1/version
//   Returns server and engine version info.
//
// GET /api/v1/capabilities
// GET /api/v1/runtime-config
// GET /api/v1/providers
//   Runtime mode, effective resource policy, and provider capability reports.
//
// GET /health
//   Docker/k8s health check.
//
// GET /readiness
//   Kubernetes readiness probe.
// ---------------------------------------------------------------------------
pub fn create_app() -> Router {
    create_app_with_config(get_config().clone())
}

pub fn create_app_with_config(config: ServerConfig) -> Router {
    // Rate limiter shares its state via Arc so the periodic cleanup task (when
    // spawned) and the middleware operate on the same map.
    let limiter = Arc::new(RateLimiter::new(config.rate_limit_per_min));
    create_app_with_limiter(config, limiter)
}

/// Build the app around a caller-provided rate limiter. `main` uses this to
/// share the limiter it spawned the cleanup task on; tests use
/// [`create_app_with_config`], which constructs an internal limiter.
pub fn create_app_with_limiter(config: ServerConfig, limiter: Arc<RateLimiter>) -> Router {
    let config = Arc::new(config);

    // Start the async job subsystem (worker pool + bounded queue + retention
    // cleanup task). The background tasks must outlive this function: in
    // production they run for the process lifetime; under a `#[tokio::test]`
    // runtime they are cancelled when that runtime is torn down at test end.
    // We therefore detach the guards rather than dropping them (dropping would
    // abort the workers immediately). Tests that need to inspect/await job
    // completion drive everything through HTTP against the returned router.
    let (jobs_state, guards) = JobsState::start(Arc::clone(&config));
    std::mem::forget(guards);

    // Job routes carry `JobsState`; the rest are stateless. Build the job
    // sub-router with its state, then merge — `with_state` erases the state
    // type so the merged router is uniformly `Router<()>`.
    let job_routes = Router::new()
        .route("/api/v1/jobs/pdf2img", post(routes::jobs::submit_pdf2img))
        .route(
            "/api/v1/jobs/extract-images",
            post(routes::jobs::submit_extract_images),
        )
        .route("/api/v1/jobs/:id", get(routes::jobs::status))
        .route("/api/v1/jobs/:id/result", get(routes::jobs::result))
        .with_state(jobs_state);

    // Progressive render session routes carry their own store state.
    let progressive_store = ProgressiveSessionStore::new(
        config.max_progressive_sessions,
        Duration::from_secs(config.progressive_session_idle_secs),
    );
    let progressive_state = routes::progressive::ProgressiveState {
        store: progressive_store,
    };
    let progressive_cleanup =
        crate::progressive_sessions::spawn_cleanup_task(progressive_state.store.clone());
    std::mem::forget(progressive_cleanup);
    let progressive_routes = Router::new()
        .route(
            "/api/v1/progressive/start",
            post(routes::progressive::start),
        )
        .route(
            "/api/v1/progressive/:id/step",
            post(routes::progressive::step),
        )
        .route(
            "/api/v1/progressive/:id/pause",
            post(routes::progressive::pause),
        )
        .route(
            "/api/v1/progressive/:id/resume",
            post(routes::progressive::resume),
        )
        .route(
            "/api/v1/progressive/:id/cancel",
            post(routes::progressive::cancel),
        )
        .route(
            "/api/v1/progressive/:id/close",
            post(routes::progressive::close),
        )
        .route(
            "/api/v1/progressive/:id/status",
            get(routes::progressive::status),
        )
        .route(
            "/api/v1/progressive/:id/finish",
            get(routes::progressive::finish_png),
        )
        .with_state(progressive_state);

    Router::new()
        .route("/health", get(routes::health::health))
        .route("/readiness", get(routes::health::readiness))
        .route("/api/v1/health", get(routes::health::health))
        .route("/api/v1/version", get(routes::health::version))
        .route("/api/v1/readiness", get(routes::health::readiness))
        .route(
            "/api/v1/capabilities",
            get(routes::health::runtime_capabilities),
        )
        .route(
            "/api/v1/runtime-config",
            get(routes::health::runtime_config),
        )
        .route("/api/v1/providers", get(routes::health::providers))
        .route("/api/v1/extract-text", post(routes::extract_text::handler))
        .route(
            "/api/v1/extract-images",
            post(routes::extract_images::handler),
        )
        .route("/api/v1/analyze", post(routes::analyze::handler))
        .route("/api/v1/pdf2img", post(routes::pdf2img::handler))
        .route("/api/v1/parse", post(routes::parse_ops::parse))
        .route("/api/v1/chunk", post(routes::parse_ops::chunk))
        .route(
            "/api/v1/extract-fields",
            post(routes::parse_ops::extract_fields),
        )
        .route("/api/v1/info", post(routes::parse_ops::info))
        .merge(job_routes)
        .merge(progressive_routes)
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(config.max_file_size))
        .layer(build_cors_layer(&config))
        .layer(middleware::from_fn_with_state(
            limiter,
            crate::rate_limit::rate_limit_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            config,
            crate::auth::auth_middleware,
        ))
}

/// Build a restrictive-by-default CORS layer from configuration.
///
/// Default (no `cors_allowed_origins`, no `cors_allow_any`): no cross-origin
/// access is granted — the safest posture for an auth-gated API that may handle
/// sensitive documents. Deployers opt in by listing their frontend origin(s) in
/// `WELLFRIENDPDF_CORS_ALLOWED_ORIGINS`. The `WELLFRIENDPDF_CORS_ALLOW_ANY` dev opt-in mirrors
/// the auth dev opt-in for local development only.
fn build_cors_layer(config: &ServerConfig) -> CorsLayer {
    // Only the methods the API actually serves, and only the headers a real
    // client needs (auth + multipart content-type), rather than "any".
    let methods = [Method::GET, Method::POST, Method::OPTIONS];
    let headers = [
        HeaderName::from_static("content-type"),
        HeaderName::from_static("authorization"),
        HeaderName::from_static("x-api-key"),
    ];

    let base = CorsLayer::new()
        .allow_methods(methods)
        .allow_headers(headers);

    if config.cors_allow_any {
        // Dev-only: warn loudly. (Startup also warns; this guards the layer.)
        tracing::warn!(
            "CORS is in ALLOW-ANY mode (WELLFRIENDPDF_CORS_ALLOW_ANY) — any origin may \
             call this API. Do NOT use this in production."
        );
        return base.allow_origin(AllowOrigin::any());
    }

    // Parse the configured origins into HeaderValues. Anything unparseable is
    // dropped with a warning rather than silently widening access.
    let origins: Vec<HeaderValue> = config
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| match origin.parse::<HeaderValue>() {
            Ok(value) => Some(value),
            Err(_) => {
                tracing::warn!(origin = %origin, "ignoring unparseable CORS origin");
                None
            }
        })
        .collect();

    // Empty list => AllowOrigin::list(empty) matches no origin: most
    // restrictive (effectively same-origin only), which is the secure default.
    base.allow_origin(AllowOrigin::list(origins))
}
