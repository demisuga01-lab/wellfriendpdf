pub async fn health() -> &'static str {
    "ok"
}

pub async fn readiness() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ready",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

pub async fn version() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "product": "Wellfriend",
        "version": env!("CARGO_PKG_VERSION"),
        "engine": wellfriendpdf_engine::ENGINE_VERSION,
    }))
}

pub async fn runtime_config() -> axum::Json<serde_json::Value> {
    let cfg = crate::config::get_config();
    let value = match cfg.effective_runtime() {
        Ok(effective) => serde_json::to_value(effective).unwrap_or_else(|error| {
            serde_json::json!({
                "status": "serialization_error",
                "error": error.to_string()
            })
        }),
        Err(error) => serde_json::json!({
            "status": "runtime_config_error",
            "error": error.to_string()
        }),
    };
    axum::Json(value)
}

pub async fn runtime_capabilities() -> axum::Json<serde_json::Value> {
    let cfg = crate::config::get_config();
    let value = match cfg.effective_runtime() {
        Ok(effective) => serde_json::to_value(effective.capabilities).unwrap_or_else(|error| {
            serde_json::json!({
                "status": "serialization_error",
                "error": error.to_string()
            })
        }),
        Err(error) => serde_json::json!({
            "status": "runtime_config_error",
            "error": error.to_string()
        }),
    };
    axum::Json(value)
}

pub async fn providers() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "schema_version": wellfriendpdf_engine::RUNTIME_CONFIG_SCHEMA_VERSION,
        "providers": wellfriendpdf_engine::ocr_provider_matrix(),
        "secret_values_serialized": false
    }))
}
