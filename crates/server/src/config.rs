struct ReceiptHmacKeyInner(Vec<u8>);

impl Drop for ReceiptHmacKeyInner {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

#[derive(Clone)]
pub struct ReceiptHmacKey(std::sync::Arc<ReceiptHmacKeyInner>);

impl From<Vec<u8>> for ReceiptHmacKey {
    fn from(value: Vec<u8>) -> Self {
        Self(std::sync::Arc::new(ReceiptHmacKeyInner(value)))
    }
}

impl ReceiptHmacKey {
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_ref().0.as_slice()
    }
}

impl std::fmt::Debug for ReceiptHmacKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReceiptHmacKey")
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone)]
#[doc(hidden)]
pub struct ReceiptHmacVerificationKey {
    key_id: String,
    key: ReceiptHmacKey,
}

impl std::fmt::Debug for ReceiptHmacVerificationKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReceiptHmacVerificationKey")
            .field("key_id", &self.key_id)
            .field("key", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone)]
pub struct ServerConfig {
    pub port: u16,
    pub log_level: String,
    pub max_file_size: usize,
    pub max_dpi: u32,
    pub max_pages: usize,
    /// Comma-separated list of valid API keys. Empty means no keys configured;
    /// the server then REFUSES TO START unless `allow_unauthenticated` is set.
    pub api_keys: Vec<String>,
    /// Explicit dev-only opt-in (WELLFRIENDPDF_ALLOW_UNAUTHENTICATED=true) to run with
    /// no API keys. Fail-closed by default: without keys and without this flag,
    /// startup aborts rather than silently exposing every endpoint.
    pub allow_unauthenticated: bool,
    /// Allowlist of origins permitted for cross-origin (CORS) requests. Empty by
    /// default (most restrictive: no cross-origin access). Set
    /// WELLFRIENDPDF_CORS_ALLOWED_ORIGINS to a comma-separated list of full origins
    /// (e.g. `https://app.example.com`).
    pub cors_allowed_origins: Vec<String>,
    /// Dev-only opt-in (WELLFRIENDPDF_CORS_ALLOW_ANY=true) to allow ANY origin. Mirrors
    /// the auth dev opt-in; logs a warning on startup. Never enable in prod.
    pub cors_allow_any: bool,
    /// Maximum requests per minute per key. Zero disables rate limiting.
    pub rate_limit_per_min: u32,
    /// Wall-clock budget for the heavy processing phase of a single request,
    /// in seconds. When exceeded, a cooperative cancellation flag trips and the
    /// engine bails out of its hot loops, returning a timeout error rather than
    /// occupying a worker indefinitely. Zero disables the timeout.
    pub request_timeout_secs: u64,
    /// Cap on rendered pixels per page (width_px * height_px). A page whose
    /// MediaBox * DPI would exceed this is rejected BEFORE the pixel buffer is
    /// allocated, preventing a giant-MediaBox "pixel explosion" OOM.
    pub max_render_pixels: u64,
    /// Cap on total response/ZIP output bytes accumulated for a request. Once
    /// output crosses this while being built, the request errors instead of
    /// buffering an absurd payload (zip-bomb-like input/output asymmetry).
    pub max_output_bytes: u64,
    /// Cap on the number of images extracted in a single extract-images request.
    pub max_image_count: usize,
    /// Number of background worker tasks processing the async job queue.
    pub job_workers: usize,
    /// Bounded capacity of the async job queue. Submissions beyond this (with
    /// all workers busy) are rejected with 503 rather than accepted unbounded.
    pub job_queue_capacity: usize,
    /// Wall-clock budget for a single async JOB, in seconds. Larger than
    /// `request_timeout_secs` on purpose: a job is not holding a connection, so
    /// it can run longer. Zero disables the per-job timeout.
    pub job_timeout_secs: u64,
    /// How long a completed/failed job and its result are retained for the
    /// client to poll/download before the cleanup task removes them.
    pub job_retention_secs: u64,
    /// Backstop cap on the number of jobs retained in the store at once. Bounds
    /// memory/disk even if submissions outpace retention cleanup.
    pub max_jobs: usize,
    /// Backstop cap on active progressive render sessions retained in memory.
    pub max_progressive_sessions: usize,
    /// Idle timeout for progressive render sessions, in seconds.
    pub progressive_session_idle_secs: u64,
    /// Directory for job result files. `None` => a per-process subdir of the
    /// system temp dir (or the `WELLFRIENDPDF_JOB_RESULT_DIR` env override). Tests set
    /// this to a unique dir so on-disk cleanup can be verified in isolation.
    pub job_result_dir: Option<String>,
    /// Canonical engine runtime configuration. Publicly exposes only Standard
    /// and Research; host policy may still force Standard.
    pub runtime_config: wellfriendpdf_engine::RuntimeConfig,
    /// Administrative policy: disallow Research even if a request asks for it.
    pub force_standard: bool,
    /// Administrative policy: allow Research when explicitly configured.
    pub allow_research: bool,
    /// Administrative policy: permit externally hosted OCR/document providers.
    pub allow_external_network_providers: bool,
    /// Administrative policy: require OCR to stay local/self-hosted.
    pub local_only_ocr: bool,
    /// Optional HMAC-SHA-256 key used only by the authenticated paint-partition
    /// preview/publication endpoints. It is distinct from API keys and is
    /// never serialized or reused for document encryption.
    pub receipt_hmac_key: Option<ReceiptHmacKey>,
    /// Public rotation identifier embedded in authenticated receipts.
    pub receipt_hmac_key_id: String,
    /// Bounded grace-period verification ring. Issuance always uses the active
    /// key above; these entries can only verify already-issued receipts.
    #[doc(hidden)]
    pub receipt_hmac_previous_keys: Vec<ReceiptHmacVerificationKey>,
    /// Audience binding embedded in authenticated receipts.
    pub receipt_hmac_audience: String,
    /// Maximum age of a server-issued review receipt.
    pub receipt_hmac_ttl_secs: u64,
    /// Deferred parse error so `from_env` stays infallible and startup
    /// validation can fail closed with a precise message.
    #[doc(hidden)]
    pub receipt_hmac_config_error: Option<String>,
}

/// Deliberately omit every credential value. `ServerConfig` is commonly held
/// in request state and may eventually be included in a panic or tracing
/// context; deriving `Debug` would disclose all configured API keys.
impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerConfig")
            .field("port", &self.port)
            .field("log_level", &self.log_level)
            .field("max_file_size", &self.max_file_size)
            .field("max_dpi", &self.max_dpi)
            .field("max_pages", &self.max_pages)
            .field("api_key_count", &self.api_keys.len())
            .field("allow_unauthenticated", &self.allow_unauthenticated)
            .field("cors_allowed_origins", &self.cors_allowed_origins)
            .field("cors_allow_any", &self.cors_allow_any)
            .field("rate_limit_per_min", &self.rate_limit_per_min)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("max_render_pixels", &self.max_render_pixels)
            .field("max_output_bytes", &self.max_output_bytes)
            .field("max_image_count", &self.max_image_count)
            .field("job_workers", &self.job_workers)
            .field("job_queue_capacity", &self.job_queue_capacity)
            .field("job_timeout_secs", &self.job_timeout_secs)
            .field("job_retention_secs", &self.job_retention_secs)
            .field("max_jobs", &self.max_jobs)
            .field("max_progressive_sessions", &self.max_progressive_sessions)
            .field(
                "progressive_session_idle_secs",
                &self.progressive_session_idle_secs,
            )
            .field("job_result_dir", &self.job_result_dir)
            .field("runtime_config", &self.runtime_config)
            .field("force_standard", &self.force_standard)
            .field("allow_research", &self.allow_research)
            .field(
                "allow_external_network_providers",
                &self.allow_external_network_providers,
            )
            .field("local_only_ocr", &self.local_only_ocr)
            .field("receipt_hmac_configured", &self.receipt_hmac_key.is_some())
            .field("receipt_hmac_key_id", &self.receipt_hmac_key_id)
            .field(
                "receipt_hmac_previous_key_count",
                &self.receipt_hmac_previous_keys.len(),
            )
            .field("receipt_hmac_audience", &self.receipt_hmac_audience)
            .field("receipt_hmac_ttl_secs", &self.receipt_hmac_ttl_secs)
            .field(
                "receipt_hmac_config_valid",
                &self.receipt_hmac_config_error.is_none(),
            )
            .finish()
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            log_level: "info".to_string(),
            max_file_size: 52_428_800,
            max_dpi: 600,
            max_pages: 200,
            api_keys: Vec::new(),
            allow_unauthenticated: false,
            cors_allowed_origins: Vec::new(),
            cors_allow_any: false,
            rate_limit_per_min: 60,
            // 30s comfortably covers a large multi-page high-DPI render while
            // stopping a single pathological page from pegging a worker forever.
            request_timeout_secs: 30,
            // 100 megapixels: an A4 page at 600 DPI is ~35 MP, a US-Arch-E
            // sheet (36x48in) at 300 DPI is ~155 MP — so this admits normal
            // high-DPI work and a generous margin while rejecting the
            // 200-inch-square MediaBox class of attack (billions of pixels).
            max_render_pixels: 100_000_000,
            // 2 GiB: a 200-page render ZIP or a large image extraction stays
            // well under this; it only trips on genuinely runaway output.
            max_output_bytes: 2 * 1024 * 1024 * 1024,
            // 10k images is far more than any real document carries on the
            // pages a single request would target.
            max_image_count: 10_000,
            // A small fixed worker pool: the heavy work is CPU-bound and itself
            // internally parallel (rayon over pages), so a few concurrent jobs
            // saturate the machine without oversubscribing.
            job_workers: 2,
            // Absorb short bursts beyond the worker count; reject (503) past it.
            job_queue_capacity: 128,
            // 300s: five minutes lets a genuinely large render/extract finish,
            // far beyond the 30s sync cap — the whole point of the async path.
            job_timeout_secs: 300,
            // Keep finished results for an hour so clients have a comfortable
            // window to poll and download before cleanup reclaims them.
            job_retention_secs: 3_600,
            // Bound total retained jobs regardless of retention timing.
            max_jobs: 1_000,
            // Progressive sessions retain document state plus completed tile
            // buffers, so keep the in-memory surface smaller than the async job
            // store.
            max_progressive_sessions: 64,
            progressive_session_idle_secs: 300,
            job_result_dir: None,
            runtime_config: wellfriendpdf_engine::RuntimeConfig::standard(),
            force_standard: false,
            allow_research: false,
            allow_external_network_providers: false,
            local_only_ocr: true,
            receipt_hmac_key: None,
            receipt_hmac_key_id: "primary".to_string(),
            receipt_hmac_previous_keys: Vec::new(),
            receipt_hmac_audience: "wellfriendpdf-server".to_string(),
            receipt_hmac_ttl_secs: 900,
            receipt_hmac_config_error: None,
        }
    }
}

impl AsRef<ServerConfig> for ServerConfig {
    fn as_ref(&self) -> &ServerConfig {
        self
    }
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();

        if let Ok(value) = std::env::var("WELLFRIENDPDF_PORT") {
            if let Ok(port) = value.parse::<u16>() {
                cfg.port = port;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_LOG_LEVEL") {
            cfg.log_level = value;
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_MAX_FILE_SIZE") {
            if let Ok(max_file_size) = value.parse::<usize>() {
                cfg.max_file_size = max_file_size;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_MAX_DPI") {
            if let Ok(max_dpi) = value.parse::<u32>() {
                cfg.max_dpi = max_dpi.min(600);
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_MAX_PAGES") {
            if let Ok(max_pages) = value.parse::<usize>() {
                cfg.max_pages = max_pages;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_API_KEYS") {
            cfg.api_keys = value
                .split(',')
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty())
                .collect();
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_ALLOW_UNAUTHENTICATED") {
            cfg.allow_unauthenticated = parse_bool_env(&value);
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_CORS_ALLOWED_ORIGINS") {
            cfg.cors_allowed_origins = value
                .split(',')
                .map(|origin| origin.trim().to_string())
                .filter(|origin| !origin.is_empty())
                .collect();
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_CORS_ALLOW_ANY") {
            cfg.cors_allow_any = parse_bool_env(&value);
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_RATE_LIMIT_PER_MIN") {
            if let Ok(rate_limit_per_min) = value.parse::<u32>() {
                cfg.rate_limit_per_min = rate_limit_per_min;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_REQUEST_TIMEOUT_SECS") {
            if let Ok(request_timeout_secs) = value.parse::<u64>() {
                cfg.request_timeout_secs = request_timeout_secs;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_MAX_RENDER_PIXELS") {
            if let Ok(max_render_pixels) = value.parse::<u64>() {
                cfg.max_render_pixels = max_render_pixels;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_MAX_OUTPUT_BYTES") {
            if let Ok(max_output_bytes) = value.parse::<u64>() {
                cfg.max_output_bytes = max_output_bytes;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_MAX_IMAGE_COUNT") {
            if let Ok(max_image_count) = value.parse::<usize>() {
                cfg.max_image_count = max_image_count;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_JOB_WORKERS") {
            if let Ok(job_workers) = value.parse::<usize>() {
                // At least one worker, else queued jobs would never drain.
                cfg.job_workers = job_workers.max(1);
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_JOB_QUEUE_CAPACITY") {
            if let Ok(job_queue_capacity) = value.parse::<usize>() {
                cfg.job_queue_capacity = job_queue_capacity.max(1);
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_JOB_TIMEOUT_SECS") {
            if let Ok(job_timeout_secs) = value.parse::<u64>() {
                cfg.job_timeout_secs = job_timeout_secs;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_JOB_RETENTION_SECS") {
            if let Ok(job_retention_secs) = value.parse::<u64>() {
                cfg.job_retention_secs = job_retention_secs;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_MAX_JOBS") {
            if let Ok(max_jobs) = value.parse::<usize>() {
                cfg.max_jobs = max_jobs.max(1);
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_MAX_PROGRESSIVE_SESSIONS") {
            if let Ok(max_progressive_sessions) = value.parse::<usize>() {
                cfg.max_progressive_sessions = max_progressive_sessions.max(1);
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_PROGRESSIVE_SESSION_IDLE_SECS") {
            if let Ok(progressive_session_idle_secs) = value.parse::<u64>() {
                cfg.progressive_session_idle_secs = progressive_session_idle_secs;
            }
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_FORCE_STANDARD") {
            cfg.force_standard = parse_bool_env(&value);
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_ALLOW_RESEARCH") {
            cfg.allow_research = parse_bool_env(&value);
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_ALLOW_EXTERNAL_PROVIDERS") {
            cfg.allow_external_network_providers = parse_bool_env(&value);
        }

        if let Ok(value) = std::env::var("WELLFRIENDPDF_LOCAL_ONLY_OCR") {
            cfg.local_only_ocr = parse_bool_env(&value);
        }

        if let Ok(mut value) = std::env::var("WELLFRIENDPDF_RECEIPT_HMAC_KEY_HEX") {
            match decode_secret_hex(value.trim()) {
                Ok(secret) => {
                    cfg.receipt_hmac_key = Some(ReceiptHmacKey::from(secret));
                }
                Err(error) => {
                    cfg.receipt_hmac_config_error.get_or_insert_with(|| {
                        format!("WELLFRIENDPDF_RECEIPT_HMAC_KEY_HEX is invalid: {error}")
                    });
                }
            }
            // Best-effort removal of the extra owned environment copy.
            unsafe { value.as_mut_vec() }.fill(0);
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_RECEIPT_HMAC_KEY_ID") {
            cfg.receipt_hmac_key_id = value;
        }
        if let Ok(mut value) = std::env::var("WELLFRIENDPDF_RECEIPT_HMAC_PREVIOUS_KEYS") {
            match parse_previous_receipt_keys(value.trim()) {
                Ok(keys) => cfg.receipt_hmac_previous_keys = keys,
                Err(error) => {
                    cfg.receipt_hmac_config_error.get_or_insert_with(|| {
                        format!("WELLFRIENDPDF_RECEIPT_HMAC_PREVIOUS_KEYS is invalid: {error}")
                    });
                }
            }
            // This owned environment copy contains secret key material.
            unsafe { value.as_mut_vec() }.fill(0);
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_RECEIPT_HMAC_AUDIENCE") {
            cfg.receipt_hmac_audience = value;
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_RECEIPT_HMAC_TTL_SECS") {
            match value.parse::<u64>() {
                Ok(ttl) => cfg.receipt_hmac_ttl_secs = ttl,
                Err(_) => {
                    cfg.receipt_hmac_config_error.get_or_insert_with(|| {
                        "WELLFRIENDPDF_RECEIPT_HMAC_TTL_SECS must be an unsigned integer"
                            .to_string()
                    });
                }
            }
        }

        if let Ok(raw) = std::env::var("WELLFRIENDPDF_RUNTIME_CONFIG_JSON") {
            if let Ok(runtime) = wellfriendpdf_engine::RuntimeConfig::from_config_str(&raw) {
                cfg.runtime_config = runtime;
            }
        } else if let Ok(path) = std::env::var("WELLFRIENDPDF_RUNTIME_CONFIG_FILE") {
            if let Ok(runtime) = wellfriendpdf_engine::RuntimeConfig::from_path(path) {
                cfg.runtime_config = runtime;
            }
        } else if let Ok(mode) = std::env::var("WELLFRIENDPDF_MODE")
            .or_else(|_| std::env::var("WELLFRIENDPDF_RUNTIME_MODE"))
        {
            if let Ok(mode) = mode.parse::<wellfriendpdf_engine::ExecutionMode>() {
                cfg.runtime_config = wellfriendpdf_engine::RuntimeConfig::from_mode(mode);
            }
        }

        cfg
    }

    /// Fail-closed startup validation. Returns an error describing the
    /// misconfiguration if the server would otherwise come up in an unsafe
    /// state. The governing rule: an empty API-key list must NOT silently
    /// leave every endpoint open — it requires the explicit dev opt-in.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(error) = &self.receipt_hmac_config_error {
            return Err(error.clone());
        }
        if let Some(key) = &self.receipt_hmac_key {
            if !(32..=256).contains(&key.as_slice().len()) {
                return Err(
                    "WELLFRIENDPDF_RECEIPT_HMAC_KEY_HEX must decode to 32..=256 bytes".to_string(),
                );
            }
            if !valid_receipt_key_id(&self.receipt_hmac_key_id) {
                return Err(
                    "WELLFRIENDPDF_RECEIPT_HMAC_KEY_ID must be 1..=128 safe ASCII characters"
                        .to_string(),
                );
            }
            if self.receipt_hmac_audience.is_empty()
                || self.receipt_hmac_audience.len() > 256
                || self
                    .receipt_hmac_audience
                    .bytes()
                    .any(|byte| byte.is_ascii_control())
            {
                return Err(
                    "WELLFRIENDPDF_RECEIPT_HMAC_AUDIENCE must be 1..=256 non-control bytes"
                        .to_string(),
                );
            }
            if !(1..=604_800).contains(&self.receipt_hmac_ttl_secs) {
                return Err("WELLFRIENDPDF_RECEIPT_HMAC_TTL_SECS must be in 1..=604800".to_string());
            }
        }
        if !self.receipt_hmac_previous_keys.is_empty() && self.receipt_hmac_key.is_none() {
            return Err(
                "WELLFRIENDPDF_RECEIPT_HMAC_PREVIOUS_KEYS requires an active WELLFRIENDPDF_RECEIPT_HMAC_KEY_HEX"
                    .to_string(),
            );
        }
        if self
            .receipt_hmac_previous_keys
            .iter()
            .any(|entry| entry.key_id == self.receipt_hmac_key_id)
        {
            return Err(
                "WELLFRIENDPDF_RECEIPT_HMAC_PREVIOUS_KEYS must not repeat the active key id"
                    .to_string(),
            );
        }
        if self.api_keys.is_empty() && !self.allow_unauthenticated {
            return Err(
                "WELLFRIENDPDF_API_KEYS is empty and WELLFRIENDPDF_ALLOW_UNAUTHENTICATED is not set; \
                 refusing to start an unauthenticated server. Set WELLFRIENDPDF_API_KEYS to a \
                 comma-separated list of keys, or set WELLFRIENDPDF_ALLOW_UNAUTHENTICATED=true \
                 to explicitly run without authentication (dev only)."
                    .to_string(),
            );
        }
        self.runtime_config
            .validate()
            .map_err(|err| format!("runtime configuration error: {err}"))?;
        let policy = self.runtime_policy();
        self.runtime_config
            .effective(wellfriendpdf_engine::HostRuntimeProfile::detect(), policy)
            .map_err(|err| format!("runtime effective configuration error: {err}"))?;
        Ok(())
    }

    /// True when API-key authentication is actively enforced (keys are
    /// configured). When false, the server is running in the explicit
    /// dev-opt-in unauthenticated mode.
    pub fn auth_enforced(&self) -> bool {
        !self.api_keys.is_empty()
    }

    pub fn receipt_authentication_enabled(&self) -> bool {
        self.receipt_hmac_key.is_some()
    }

    /// Resolve only an active or explicitly retained grace-period key. The
    /// clone shares zeroizing storage and its Debug representation is redacted.
    pub fn receipt_verification_key(&self, key_id: &str) -> Option<ReceiptHmacKey> {
        if self.receipt_hmac_key_id == key_id {
            return self.receipt_hmac_key.clone();
        }
        self.receipt_hmac_previous_keys
            .iter()
            .find(|entry| entry.key_id == key_id)
            .map(|entry| entry.key.clone())
    }

    pub fn runtime_policy(&self) -> wellfriendpdf_engine::HostRuntimePolicy {
        wellfriendpdf_engine::HostRuntimePolicy {
            force_standard: self.force_standard,
            allow_research: self.allow_research,
            allow_external_network_providers: self.allow_external_network_providers,
            local_only_ocr: self.local_only_ocr,
            max_memory_bytes: None,
            max_cpu_workers: None,
            max_provider_cost_micros: self.runtime_config.providers.max_provider_cost_micros,
            allowed_tenants_for_research: Vec::new(),
        }
    }

    pub fn effective_runtime(
        &self,
    ) -> Result<wellfriendpdf_engine::EffectiveRuntimeConfig, wellfriendpdf_engine::WellfriendError>
    {
        self.runtime_config.effective(
            wellfriendpdf_engine::HostRuntimeProfile::detect(),
            self.runtime_policy(),
        )
    }
}

/// Parse a boolean-ish env value. Accepts true/1/yes/on (case-insensitive) as
/// true; everything else is false. Keeps the dev opt-ins explicit.
fn parse_bool_env(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

fn decode_secret_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) || value.is_empty() {
        return Err("expected a non-empty even number of hexadecimal characters".to_string());
    }
    if value.len() > 512 {
        return Err("decoded key would exceed 256 bytes".to_string());
    }
    let mut decoded = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let nibble = |byte: u8| -> Option<u8> {
            match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            }
        };
        decoded.push(
            nibble(pair[0])
                .and_then(|high| nibble(pair[1]).map(|low| (high << 4) | low))
                .ok_or_else(|| "expected hexadecimal characters only".to_string())?,
        );
    }
    Ok(decoded)
}

fn valid_receipt_key_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn parse_previous_receipt_keys(value: &str) -> Result<Vec<ReceiptHmacVerificationKey>, String> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut parsed = Vec::new();
    for entry in value.split(',') {
        if parsed.len() >= 8 {
            return Err("at most 8 previous keys are allowed".to_string());
        }
        let (key_id, hex) = entry
            .split_once('=')
            .ok_or_else(|| "expected comma-separated key-id=hex entries".to_string())?;
        let key_id = key_id.trim();
        let hex = hex.trim();
        if !valid_receipt_key_id(key_id) {
            return Err("every previous key id must be 1..=128 safe ASCII characters".to_string());
        }
        if parsed
            .iter()
            .any(|candidate: &ReceiptHmacVerificationKey| candidate.key_id == key_id)
        {
            return Err(format!("duplicate previous key id '{key_id}'"));
        }
        let key = ReceiptHmacKey::from(decode_secret_hex(hex)?);
        if !(32..=256).contains(&key.as_slice().len()) {
            return Err(format!(
                "previous key '{key_id}' must decode to 32..=256 bytes"
            ));
        }
        parsed.push(ReceiptHmacVerificationKey {
            key_id: key_id.to_string(),
            key,
        });
    }
    Ok(parsed)
}

#[cfg(test)]
mod receipt_auth_tests {
    use super::*;

    #[test]
    fn receipt_key_debug_is_redacted_and_secret_is_decoded_exactly() {
        let bytes = decode_secret_hex(&"5a".repeat(32)).unwrap();
        let key = ReceiptHmacKey::from(bytes);
        assert_eq!(key.as_slice(), &[0x5a; 32]);
        let debug = format!("{key:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("5a5a"));
    }

    #[test]
    fn receipt_configuration_fails_closed_on_invalid_bounds() {
        let mut config = ServerConfig {
            allow_unauthenticated: true,
            receipt_hmac_key: Some(ReceiptHmacKey::from(vec![0; 31])),
            ..ServerConfig::default()
        };
        assert!(config.validate().is_err());
        config.receipt_hmac_key = Some(ReceiptHmacKey::from(vec![0; 32]));
        config.receipt_hmac_ttl_secs = 0;
        assert!(config.validate().is_err());
        config.receipt_hmac_ttl_secs = 900;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn receipt_verification_ring_is_bounded_and_verification_only() {
        let previous = parse_previous_receipt_keys(&format!(
            "old-a={},old-b={}",
            "11".repeat(32),
            "22".repeat(32)
        ))
        .unwrap();
        let config = ServerConfig {
            allow_unauthenticated: true,
            receipt_hmac_key: Some(ReceiptHmacKey::from(vec![0x33; 32])),
            receipt_hmac_key_id: "active".to_string(),
            receipt_hmac_previous_keys: previous,
            ..ServerConfig::default()
        };
        assert!(config.validate().is_ok());
        assert_eq!(
            config.receipt_verification_key("old-b").unwrap().as_slice(),
            &[0x22; 32]
        );
        assert!(config.receipt_verification_key("missing").is_none());
        assert!(parse_previous_receipt_keys(&format!(
            "same={},same={}",
            "11".repeat(32),
            "22".repeat(32)
        ))
        .is_err());
    }
}

pub static CONFIG: std::sync::OnceLock<ServerConfig> = std::sync::OnceLock::new();

tokio::task_local! {
    static REQUEST_CONFIG: std::sync::Arc<ServerConfig>;
}

pub fn get_config() -> std::sync::Arc<ServerConfig> {
    REQUEST_CONFIG
        .try_with(std::sync::Arc::clone)
        .unwrap_or_else(|_| std::sync::Arc::new(CONFIG.get_or_init(ServerConfig::default).clone()))
}

pub async fn request_config_middleware(
    axum::extract::State(config): axum::extract::State<std::sync::Arc<ServerConfig>>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    REQUEST_CONFIG.scope(config, next.run(request)).await
}
