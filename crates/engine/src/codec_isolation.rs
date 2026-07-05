//! OS subprocess isolation for bounded codec decode work.
//!
//! Prompt 03 starts with lossless stream filters because they are already
//! centralized in the engine and can be moved across a process boundary without
//! changing PDF object ownership. This is containment, not a complete sandbox:
//! the parent process owns policy, limits, timeout handling, response
//! validation, and fallback decisions.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{OxideError, Result};
use crate::filters::{apply_filter_bytes_in_process_with_limits, DecodeLimits};

pub const CODEC_WORKER_PROTOCOL_VERSION: u32 = 1;
pub const CODEC_WORKER_VERSION: &str = env!("CARGO_PKG_VERSION");

const DEFAULT_TIMEOUT_MS: u64 = 2_000;
const DEFAULT_MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
const RESPONSE_OVERHEAD_BYTES: u64 = 64 * 1024;
const MAX_RESPONSE_JSON_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodecIsolationPolicy {
    #[default]
    InProcess,
    IsolatedPreferred,
    IsolatedRequired,
    ReportOnly,
    Disabled,
}

impl CodecIsolationPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "in_process" | "in-process" | "inprocess" => Some(Self::InProcess),
            "isolated_preferred" | "isolated-preferred" | "preferred" => {
                Some(Self::IsolatedPreferred)
            }
            "isolated_required" | "isolated-required" | "required" => Some(Self::IsolatedRequired),
            "report_only" | "report-only" => Some(Self::ReportOnly),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::InProcess => "in_process",
            Self::IsolatedPreferred => "isolated_preferred",
            Self::IsolatedRequired => "isolated_required",
            Self::ReportOnly => "report_only",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CodecIsolationLimits {
    pub max_input_bytes: u64,
    pub max_decoded_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub timeout_milliseconds: u64,
    pub deterministic: bool,
}

impl Default for CodecIsolationLimits {
    fn default() -> Self {
        let decode = DecodeLimits::default();
        Self {
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_decoded_bytes: decode.max_decoded_bytes_per_stream,
            max_width: decode.max_image_width,
            max_height: decode.max_image_height,
            max_pixels: decode.max_image_pixels,
            timeout_milliseconds: DEFAULT_TIMEOUT_MS,
            deterministic: true,
        }
    }
}

impl CodecIsolationLimits {
    fn as_decode_limits(&self) -> DecodeLimits {
        DecodeLimits {
            max_decoded_bytes_per_stream: self.max_decoded_bytes,
            max_image_width: self.max_width,
            max_image_height: self.max_height,
            max_image_pixels: self.max_pixels,
            max_image_decoded_bytes: self.max_decoded_bytes,
            ..DecodeLimits::default()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CodecIsolationConfig {
    pub policy: CodecIsolationPolicy,
    pub worker_path: Option<PathBuf>,
    pub limits: CodecIsolationLimits,
    pub trace_id: Option<String>,
    #[doc(hidden)]
    pub worker_test_mode: Option<String>,
}

impl CodecIsolationConfig {
    pub fn with_policy(policy: CodecIsolationPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    pub fn from_policy_str(policy: Option<&str>) -> Result<Self> {
        let parsed = match policy {
            Some(value) => CodecIsolationPolicy::parse(value).ok_or_else(|| {
                OxideError::invalid_input(format!(
                    "unknown codec isolation policy '{value}'; use in_process, isolated_preferred, isolated_required, report_only, or disabled"
                ))
            })?,
            None => CodecIsolationPolicy::InProcess,
        };
        Ok(Self::with_policy(parsed))
    }

    pub fn with_worker_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.worker_path = Some(path.into());
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.limits.timeout_milliseconds = timeout_ms.max(1);
        self
    }

    pub fn with_max_decoded_bytes(mut self, max_decoded_bytes: u64) -> Self {
        self.limits.max_decoded_bytes = max_decoded_bytes.max(1);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CodecDimensions {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub pixels: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CodecIsolationReport {
    pub protocol_version: u32,
    pub request_id: String,
    pub codec_kind: String,
    pub requested_policy: String,
    pub isolation_mode: String,
    pub status: String,
    pub ok: bool,
    pub platform_supported: bool,
    pub worker_available: bool,
    pub worker_used: bool,
    pub fallback_used: bool,
    pub fallback_reason: Option<String>,
    pub decoded_byte_length: Option<usize>,
    pub dimensions: Option<CodecDimensions>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub elapsed_milliseconds: u64,
    pub worker_version: Option<String>,
    pub limit_failed: Option<String>,
    pub trace_id: Option<String>,
    pub supported_worker_codecs: Vec<String>,
    pub guarantee: String,
}

impl CodecIsolationReport {
    fn new(
        filter_name: &str,
        policy: &CodecIsolationPolicy,
        request_id: String,
        trace_id: Option<String>,
    ) -> Self {
        Self {
            protocol_version: CODEC_WORKER_PROTOCOL_VERSION,
            request_id,
            codec_kind: canonical_filter_name(filter_name).to_string(),
            requested_policy: policy.as_str().to_string(),
            isolation_mode: policy.as_str().to_string(),
            status: "pending".to_string(),
            ok: false,
            platform_supported: platform_supports_process_isolation(),
            worker_available: false,
            worker_used: false,
            fallback_used: false,
            fallback_reason: None,
            decoded_byte_length: None,
            dimensions: None,
            warnings: Vec::new(),
            errors: Vec::new(),
            elapsed_milliseconds: 0,
            worker_version: None,
            limit_failed: None,
            trace_id,
            supported_worker_codecs: supported_worker_codecs()
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            guarantee: "subprocess isolation contains worker crashes, timeouts, and bounded output; it is not an OS-enforced sandbox"
                .to_string(),
        }
    }

    fn fail(mut self, status: &str, message: impl Into<String>) -> Self {
        self.status = status.to_string();
        self.ok = false;
        self.errors.push(message.into());
        self
    }

    fn success(mut self, mode: &str, decoded_len: usize, elapsed_ms: u64) -> Self {
        self.status = "success".to_string();
        self.ok = true;
        self.isolation_mode = mode.to_string();
        self.decoded_byte_length = Some(decoded_len);
        self.elapsed_milliseconds = elapsed_ms;
        self
    }
}

#[derive(Clone, Debug)]
pub struct CodecIsolationDecode {
    pub decoded: Option<Vec<u8>>,
    pub report: CodecIsolationReport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodecWorkerRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub codec_kind: String,
    pub input_length: usize,
    pub input_bytes: Vec<u8>,
    pub decode_options: serde_json::Value,
    pub maximum_decoded_bytes: u64,
    pub maximum_width: u32,
    pub maximum_height: u32,
    pub maximum_pixels: u64,
    pub timeout_milliseconds: u64,
    pub deterministic: bool,
    pub trace_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodecWorkerResponse {
    pub protocol_version: u32,
    pub request_id: String,
    pub status: String,
    pub decoded_byte_length: usize,
    pub decoded_bytes: Vec<u8>,
    pub dimensions: Option<CodecDimensions>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub elapsed_milliseconds: u64,
    pub worker_version: String,
    pub isolation_mode: String,
    pub limit_failed: Option<String>,
}

pub fn supported_worker_codecs() -> &'static [&'static str] {
    &[
        "FlateDecode",
        "ASCIIHexDecode",
        "ASCII85Decode",
        "RunLengthDecode",
        "LZWDecode",
    ]
}

pub fn platform_supports_process_isolation() -> bool {
    cfg!(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "macos"
    ))
}

pub fn default_worker_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OXIDE_CODEC_WORKER").map(PathBuf::from) {
        if path.exists() {
            return Some(path);
        }
    }

    let exe = std::env::current_exe().ok()?;
    let file_name = if cfg!(windows) {
        "oxide-codec-worker.exe"
    } else {
        "oxide-codec-worker"
    };
    let sibling = exe.parent()?.join(file_name);
    if sibling.exists() {
        Some(sibling)
    } else {
        None
    }
}

pub fn codec_isolation_availability_report() -> serde_json::Value {
    serde_json::json!({
        "status": if platform_supports_process_isolation() { "available_when_worker_present" } else { "unavailable_on_target" },
        "default_policy": CodecIsolationPolicy::InProcess.as_str(),
        "modes": [
            "in_process",
            "isolated_preferred",
            "isolated_required",
            "report_only",
            "disabled"
        ],
        "worker_protocol_version": CODEC_WORKER_PROTOCOL_VERSION,
        "worker_binary": default_worker_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "not_found".to_string()),
        "supported_worker_codecs": supported_worker_codecs(),
        "wasm": {
            "subprocess_isolation": "unavailable",
            "reason": "browser and wasm32-unknown-unknown targets cannot spawn OS codec workers"
        },
        "guarantee": "crash, timeout, and output-size containment only; not a formal sandbox"
    })
}

pub fn decode_filter_with_isolation(
    filter_name: &str,
    input: &[u8],
    config: &CodecIsolationConfig,
) -> CodecIsolationDecode {
    let request_id = new_request_id();
    let mut report = CodecIsolationReport::new(
        filter_name,
        &config.policy,
        request_id.clone(),
        config.trace_id.clone(),
    );

    if input.len() as u64 > config.limits.max_input_bytes {
        report.status = "input_cap_exceeded".to_string();
        report.limit_failed = Some("max_input_bytes".to_string());
        report.errors.push(format!(
            "input is {} bytes, exceeding max_input_bytes {}",
            input.len(),
            config.limits.max_input_bytes
        ));
        return CodecIsolationDecode {
            decoded: None,
            report,
        };
    }

    match config.policy {
        CodecIsolationPolicy::Disabled => CodecIsolationDecode {
            decoded: None,
            report: report.fail(
                "disabled",
                "codec subprocess isolation is disabled by policy",
            ),
        },
        CodecIsolationPolicy::ReportOnly => CodecIsolationDecode {
            decoded: None,
            report: report.fail(
                "report_only",
                format!("{filter_name} was not decoded because report_only policy was requested"),
            ),
        },
        CodecIsolationPolicy::InProcess => in_process_decode(filter_name, input, report, config),
        CodecIsolationPolicy::IsolatedRequired => {
            if !platform_supports_process_isolation() {
                return CodecIsolationDecode {
                    decoded: None,
                    report: report.fail(
                        "failed_closed",
                        "subprocess codec isolation is not supported on this platform",
                    ),
                };
            }
            match worker_decode(filter_name, input, config, report) {
                WorkerAttempt::Success(decoded) => decoded,
                WorkerAttempt::Failure(report) => CodecIsolationDecode {
                    decoded: None,
                    report: report.fail(
                        "failed_closed",
                        "isolated_required policy refused in-process fallback",
                    ),
                },
            }
        }
        CodecIsolationPolicy::IsolatedPreferred => {
            if !platform_supports_process_isolation() {
                report.warnings.push(
                    "subprocess codec isolation is not supported on this platform; falling back by policy"
                        .to_string(),
                );
                return fallback_in_process(
                    filter_name,
                    input,
                    report,
                    "platform_unavailable",
                    config,
                );
            }
            match worker_decode(filter_name, input, config, report) {
                WorkerAttempt::Success(decoded) => decoded,
                WorkerAttempt::Failure(report) => fallback_in_process(
                    filter_name,
                    input,
                    report,
                    "worker_unavailable_or_failed",
                    config,
                ),
            }
        }
    }
}

pub fn codec_dimension_report(
    filter_name: &str,
    width: u32,
    height: u32,
    components: u8,
    config: &CodecIsolationConfig,
) -> CodecIsolationReport {
    let request_id = new_request_id();
    let mut report = CodecIsolationReport::new(
        filter_name,
        &config.policy,
        request_id,
        config.trace_id.clone(),
    );
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    report.dimensions = Some(CodecDimensions {
        width,
        height,
        components,
        pixels,
    });
    if width > config.limits.max_width {
        report.status = "dimension_cap_exceeded".to_string();
        report.limit_failed = Some("max_width".to_string());
        report.errors.push(format!(
            "width {width} exceeds max_width {}",
            config.limits.max_width
        ));
        return report;
    }
    if height > config.limits.max_height {
        report.status = "dimension_cap_exceeded".to_string();
        report.limit_failed = Some("max_height".to_string());
        report.errors.push(format!(
            "height {height} exceeds max_height {}",
            config.limits.max_height
        ));
        return report;
    }
    if pixels > config.limits.max_pixels {
        report.status = "dimension_cap_exceeded".to_string();
        report.limit_failed = Some("max_pixels".to_string());
        report.errors.push(format!(
            "{pixels} pixels exceeds max_pixels {}",
            config.limits.max_pixels
        ));
        return report;
    }
    let bytes = pixels.saturating_mul(u64::from(components.max(1)));
    if bytes > config.limits.max_decoded_bytes {
        report.status = "decoded_output_cap_exceeded".to_string();
        report.limit_failed = Some("max_decoded_bytes".to_string());
        report.errors.push(format!(
            "{bytes} decoded bytes exceeds max_decoded_bytes {}",
            config.limits.max_decoded_bytes
        ));
        return report;
    }
    report.success("parent_dimension_check", bytes as usize, 0)
}

pub fn worker_handle_request(
    request: CodecWorkerRequest,
    self_test_mode: Option<&str>,
) -> CodecWorkerResponse {
    if let Some(mode) = self_test_mode {
        match mode {
            "wrong-id" => {
                return worker_error_response(
                    &request,
                    "wrong-request-id",
                    "success",
                    Vec::new(),
                    None,
                );
            }
            "oversized" => {
                let len = request
                    .maximum_decoded_bytes
                    .saturating_add(1)
                    .min(1_048_576) as usize;
                return worker_success_response(&request, vec![0; len], 0);
            }
            "unsupported" => {
                return worker_error_response(
                    &request,
                    &request.request_id,
                    "unsupported_codec",
                    vec![format!(
                        "{} is unsupported by the worker",
                        request.codec_kind
                    )],
                    None,
                );
            }
            _ => {}
        }
    }

    let started = Instant::now();
    if request.protocol_version != CODEC_WORKER_PROTOCOL_VERSION {
        return worker_error_response(
            &request,
            &request.request_id,
            "invalid_request",
            vec![format!(
                "protocol_version {} does not match {}",
                request.protocol_version, CODEC_WORKER_PROTOCOL_VERSION
            )],
            None,
        );
    }
    if request.input_length != request.input_bytes.len() {
        return worker_error_response(
            &request,
            &request.request_id,
            "invalid_request",
            vec![format!(
                "input_length {} does not match {} bytes",
                request.input_length,
                request.input_bytes.len()
            )],
            None,
        );
    }
    if request.input_bytes.len() as u64 > DEFAULT_MAX_INPUT_BYTES {
        return worker_error_response(
            &request,
            &request.request_id,
            "input_cap_exceeded",
            vec![format!(
                "input exceeds worker hard cap of {DEFAULT_MAX_INPUT_BYTES} bytes"
            )],
            Some("max_input_bytes".to_string()),
        );
    }
    if !supported_worker_codecs()
        .iter()
        .any(|name| *name == canonical_filter_name(&request.codec_kind))
    {
        return worker_error_response(
            &request,
            &request.request_id,
            "unsupported_codec",
            vec![format!(
                "{} is not enabled in the Prompt 03 worker",
                request.codec_kind
            )],
            None,
        );
    }

    let limits = DecodeLimits {
        max_decoded_bytes_per_stream: request.maximum_decoded_bytes,
        max_image_width: request.maximum_width,
        max_image_height: request.maximum_height,
        max_image_pixels: request.maximum_pixels,
        max_image_decoded_bytes: request.maximum_decoded_bytes,
        ..DecodeLimits::default()
    };
    match apply_filter_bytes_in_process_with_limits(
        canonical_filter_name(&request.codec_kind),
        &request.input_bytes,
        None,
        &limits,
    ) {
        Ok(decoded) => {
            if decoded.len() as u64 > request.maximum_decoded_bytes {
                worker_error_response(
                    &request,
                    &request.request_id,
                    "decoded_output_cap_exceeded",
                    vec![format!(
                        "decoded output {} exceeds maximum_decoded_bytes {}",
                        decoded.len(),
                        request.maximum_decoded_bytes
                    )],
                    Some("maximum_decoded_bytes".to_string()),
                )
            } else {
                worker_success_response(&request, decoded, elapsed_ms(started))
            }
        }
        Err(err) => worker_error_response(
            &request,
            &request.request_id,
            "decode_failed",
            vec![err.to_string()],
            None,
        ),
    }
}

enum WorkerAttempt {
    Success(CodecIsolationDecode),
    Failure(CodecIsolationReport),
}

fn in_process_decode(
    filter_name: &str,
    input: &[u8],
    report: CodecIsolationReport,
    config: &CodecIsolationConfig,
) -> CodecIsolationDecode {
    let started = Instant::now();
    let limits = config.limits.as_decode_limits();
    match apply_filter_bytes_in_process_with_limits(filter_name, input, None, &limits) {
        Ok(decoded) => {
            let len = decoded.len();
            CodecIsolationDecode {
                decoded: Some(decoded),
                report: report.success("in_process", len, elapsed_ms(started)),
            }
        }
        Err(err) => CodecIsolationDecode {
            decoded: None,
            report: report.fail("decode_failed", err.to_string()),
        },
    }
}

fn fallback_in_process(
    filter_name: &str,
    input: &[u8],
    mut report: CodecIsolationReport,
    reason: &str,
    config: &CodecIsolationConfig,
) -> CodecIsolationDecode {
    let started = Instant::now();
    let limits = config.limits.as_decode_limits();
    match apply_filter_bytes_in_process_with_limits(filter_name, input, None, &limits) {
        Ok(decoded) => {
            let len = decoded.len();
            report.status = "fallback_success".to_string();
            report.ok = true;
            report.isolation_mode = "in_process".to_string();
            report.fallback_used = true;
            report.fallback_reason = Some(reason.to_string());
            report.decoded_byte_length = Some(len);
            report.elapsed_milliseconds = elapsed_ms(started);
            CodecIsolationDecode {
                decoded: Some(decoded),
                report,
            }
        }
        Err(err) => {
            report.status = "fallback_failed".to_string();
            report.fallback_used = true;
            report.fallback_reason = Some(reason.to_string());
            report.errors.push(err.to_string());
            CodecIsolationDecode {
                decoded: None,
                report,
            }
        }
    }
}

fn worker_decode(
    filter_name: &str,
    input: &[u8],
    config: &CodecIsolationConfig,
    mut report: CodecIsolationReport,
) -> WorkerAttempt {
    let Some(worker_path) = config.worker_path.clone().or_else(default_worker_path) else {
        report.status = "worker_unavailable".to_string();
        report.errors.push(
            "codec worker binary not found; set OXIDE_CODEC_WORKER or pass worker_path".to_string(),
        );
        return WorkerAttempt::Failure(report);
    };
    if !worker_path.exists() {
        report.status = "worker_unavailable".to_string();
        report.errors.push(format!(
            "codec worker does not exist: {}",
            worker_path.display()
        ));
        return WorkerAttempt::Failure(report);
    }
    report.worker_available = true;
    report.worker_used = true;
    report.isolation_mode = "subprocess".to_string();

    let request = CodecWorkerRequest {
        protocol_version: CODEC_WORKER_PROTOCOL_VERSION,
        request_id: report.request_id.clone(),
        codec_kind: canonical_filter_name(filter_name).to_string(),
        input_length: input.len(),
        input_bytes: input.to_vec(),
        decode_options: serde_json::json!({}),
        maximum_decoded_bytes: config.limits.max_decoded_bytes,
        maximum_width: config.limits.max_width,
        maximum_height: config.limits.max_height,
        maximum_pixels: config.limits.max_pixels,
        timeout_milliseconds: config.limits.timeout_milliseconds,
        deterministic: config.limits.deterministic,
        trace_id: config.trace_id.clone(),
    };

    let run_dir = std::env::temp_dir();
    let stem = format!("oxide-codec-{}", request.request_id);
    let request_path = run_dir.join(format!("{stem}.request.json"));
    let response_path = run_dir.join(format!("{stem}.response.json"));
    let result = run_worker_process(
        &worker_path,
        &request_path,
        &response_path,
        &request,
        config,
    );
    let _ = fs::remove_file(&request_path);
    let _ = fs::remove_file(&response_path);

    match result {
        Ok(response) => match validate_worker_response(&request, response, &config.limits) {
            Ok((decoded, response, elapsed_ms)) => {
                report.status = "success".to_string();
                report.ok = true;
                report.decoded_byte_length = Some(decoded.len());
                report.elapsed_milliseconds = elapsed_ms;
                report.worker_version = Some(response.worker_version);
                report.limit_failed = response.limit_failed;
                report.warnings.extend(response.warnings);
                report.errors.extend(response.errors);
                report.dimensions = response.dimensions;
                WorkerAttempt::Success(CodecIsolationDecode {
                    decoded: Some(decoded),
                    report,
                })
            }
            Err(failure) => {
                let mut failure = *failure;
                failure.requested_policy = report.requested_policy;
                failure.trace_id = report.trace_id;
                failure.supported_worker_codecs = report.supported_worker_codecs;
                WorkerAttempt::Failure(failure)
            }
        },
        Err(message) => {
            report.status = classify_worker_failure(&message).to_string();
            report.errors.push(message);
            WorkerAttempt::Failure(report)
        }
    }
}

fn run_worker_process(
    worker_path: &Path,
    request_path: &Path,
    response_path: &Path,
    request: &CodecWorkerRequest,
    config: &CodecIsolationConfig,
) -> std::result::Result<CodecWorkerResponse, String> {
    let request_json =
        serde_json::to_vec(request).map_err(|err| format!("worker request JSON error: {err}"))?;
    fs::write(request_path, request_json)
        .map_err(|err| format!("failed to write worker request: {err}"))?;

    let started = Instant::now();
    let mut command = Command::new(worker_path);
    command
        .arg("--request")
        .arg(request_path)
        .arg("--response")
        .arg(response_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env_clear();
    if let Some(mode) = &config.worker_test_mode {
        command.env("OXIDE_CODEC_WORKER_SELF_TEST", mode);
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to spawn codec worker: {err}"))?;
    let timeout = Duration::from_millis(config.limits.timeout_milliseconds.max(1));
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!("codec worker exited with status {status}"));
                }
                break;
            }
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "codec worker timed out after {} ms",
                        config.limits.timeout_milliseconds
                    ));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(err) => return Err(format!("failed while waiting for codec worker: {err}")),
        }
    }

    let metadata = fs::metadata(response_path)
        .map_err(|err| format!("codec worker did not produce a response: {err}"))?;
    let max_response = config
        .limits
        .max_decoded_bytes
        .saturating_mul(8)
        .saturating_add(RESPONSE_OVERHEAD_BYTES)
        .clamp(RESPONSE_OVERHEAD_BYTES, MAX_RESPONSE_JSON_BYTES);
    if metadata.len() > max_response {
        return Err(format!(
            "codec worker response is {} bytes, exceeding parent cap {max_response}",
            metadata.len()
        ));
    }
    let bytes =
        fs::read(response_path).map_err(|err| format!("failed to read worker response: {err}"))?;
    serde_json::from_slice(&bytes).map_err(|err| format!("invalid worker response JSON: {err}"))
}

fn validate_worker_response(
    request: &CodecWorkerRequest,
    response: CodecWorkerResponse,
    limits: &CodecIsolationLimits,
) -> std::result::Result<(Vec<u8>, CodecWorkerResponse, u64), Box<CodecIsolationReport>> {
    let mut report = CodecIsolationReport::new(
        &request.codec_kind,
        &CodecIsolationPolicy::IsolatedRequired,
        request.request_id.clone(),
        request.trace_id.clone(),
    );
    report.worker_available = true;
    report.worker_used = true;
    report.isolation_mode = "subprocess".to_string();
    report.worker_version = Some(response.worker_version.clone());
    report.limit_failed = response.limit_failed.clone();
    report.warnings = response.warnings.clone();
    report.errors = response.errors.clone();
    report.dimensions = response.dimensions.clone();
    report.elapsed_milliseconds = response.elapsed_milliseconds;

    if response.protocol_version != CODEC_WORKER_PROTOCOL_VERSION {
        return Err(Box::new(report.fail(
            "invalid_worker_response",
            format!(
                "worker protocol_version {} does not match {}",
                response.protocol_version, CODEC_WORKER_PROTOCOL_VERSION
            ),
        )));
    }
    if response.request_id != request.request_id {
        return Err(Box::new(report.fail(
            "invalid_worker_response",
            format!(
                "worker response request_id '{}' does not match '{}'",
                response.request_id, request.request_id
            ),
        )));
    }
    if response.decoded_byte_length != response.decoded_bytes.len() {
        return Err(Box::new(report.fail(
            "invalid_worker_response",
            format!(
                "decoded_byte_length {} does not match {} bytes",
                response.decoded_byte_length,
                response.decoded_bytes.len()
            ),
        )));
    }
    if response.decoded_bytes.len() as u64 > limits.max_decoded_bytes {
        report.limit_failed = Some("max_decoded_bytes".to_string());
        return Err(Box::new(report.fail(
            "decoded_output_cap_exceeded",
            format!(
                "worker returned {} decoded bytes, exceeding max_decoded_bytes {}",
                response.decoded_bytes.len(),
                limits.max_decoded_bytes
            ),
        )));
    }
    if response.status != "success" {
        return Err(Box::new(
            report.fail(&response.status, "worker reported decode failure"),
        ));
    }
    let elapsed = response.elapsed_milliseconds;
    Ok((response.decoded_bytes.clone(), response, elapsed))
}

fn worker_success_response(
    request: &CodecWorkerRequest,
    decoded: Vec<u8>,
    elapsed_milliseconds: u64,
) -> CodecWorkerResponse {
    CodecWorkerResponse {
        protocol_version: CODEC_WORKER_PROTOCOL_VERSION,
        request_id: request.request_id.clone(),
        status: "success".to_string(),
        decoded_byte_length: decoded.len(),
        decoded_bytes: decoded,
        dimensions: None,
        warnings: Vec::new(),
        errors: Vec::new(),
        elapsed_milliseconds,
        worker_version: CODEC_WORKER_VERSION.to_string(),
        isolation_mode: "subprocess".to_string(),
        limit_failed: None,
    }
}

fn worker_error_response(
    _request: &CodecWorkerRequest,
    request_id: &str,
    status: &str,
    errors: Vec<String>,
    limit_failed: Option<String>,
) -> CodecWorkerResponse {
    CodecWorkerResponse {
        protocol_version: CODEC_WORKER_PROTOCOL_VERSION,
        request_id: request_id.to_string(),
        status: status.to_string(),
        decoded_byte_length: 0,
        decoded_bytes: Vec::new(),
        dimensions: None,
        warnings: Vec::new(),
        errors,
        elapsed_milliseconds: 0,
        worker_version: CODEC_WORKER_VERSION.to_string(),
        isolation_mode: "subprocess".to_string(),
        limit_failed,
    }
}

fn canonical_filter_name(name: &str) -> &str {
    match name {
        "Fl" => "FlateDecode",
        "LZW" => "LZWDecode",
        "AHx" => "ASCIIHexDecode",
        "A85" => "ASCII85Decode",
        "RL" => "RunLengthDecode",
        "DCT" => "DCTDecode",
        "CCF" => "CCITTFaxDecode",
        other => other,
    }
}

fn classify_worker_failure(message: &str) -> &'static str {
    if message.contains("timed out") {
        "worker_timeout"
    } else if message.contains("response") || message.contains("JSON") {
        "invalid_worker_response"
    } else if message.contains("exited") {
        "worker_exit"
    } else {
        "worker_failed"
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn new_request_id() -> String {
    let pid = std::process::id();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{pid}-{now}")
}

pub fn write_worker_response(path: &Path, response: &CodecWorkerResponse) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    let json = serde_json::to_vec(response).map_err(std::io::Error::other)?;
    file.write_all(&json)?;
    file.flush()
}
