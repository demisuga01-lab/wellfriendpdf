use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = wellfriendpdf_server::config::ServerConfig::from_env();
    let port = config.port;
    let log_level = config.log_level.clone();

    let env_filter = EnvFilter::try_new(&log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    // Fail-closed startup: refuse to come up unauthenticated unless the
    // operator has explicitly opted into the dev mode. This turns a silent
    // misconfiguration (forgot to set keys) into a loud, immediate failure
    // rather than an open server.
    if let Err(msg) = config.validate() {
        tracing::error!("{}", msg);
        eprintln!("FATAL: {}", msg);
        std::process::exit(1);
    }

    if !config.auth_enforced() {
        tracing::warn!(
            "WELLFRIENDPDF_ALLOW_UNAUTHENTICATED is set: starting WITHOUT API-key \
             authentication. Every data endpoint is open. This is intended for \
             local development ONLY — set WELLFRIENDPDF_API_KEYS before deploying."
        );
    }
    if config.cors_allow_any {
        tracing::warn!(
            "WELLFRIENDPDF_CORS_ALLOW_ANY is set: CORS will accept ANY origin. Intended \
             for local development ONLY — set WELLFRIENDPDF_CORS_ALLOWED_ORIGINS in prod."
        );
    }

    let _ = wellfriendpdf_server::config::CONFIG.set(config);
    let config = wellfriendpdf_server::config::get_config();

    // Register the server-wide OCR backend from the environment (WELLFRIENDPDF_OCR).
    // No-op unless built with `--features ocr` and WELLFRIENDPDF_OCR is auto/force;
    // logs the outcome so the operator sees whether scanned pages get OCR'd.
    wellfriendpdf_server::ocr::init_from_env();

    // Build the app and start the rate-limiter cleanup task on the same limiter
    // the app uses, so per-key buckets don't accumulate unbounded.
    let limiter = Arc::new(wellfriendpdf_server::rate_limit::RateLimiter::new(
        config.rate_limit_per_min,
    ));
    let _cleanup = limiter.spawn_cleanup(Duration::from_secs(60));
    let app = wellfriendpdf_server::app::create_app_with_limiter(config.clone(), limiter);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(&addr).await?;

    tracing::info!("Wellfriend listening on {}", addr);
    // axum::serve handles SIGTERM gracefully under the tokio runtime.
    axum::serve(listener, app).await?;

    Ok(())
}
