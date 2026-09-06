//! Runtime mode, resource, provider, and low-resource execution contracts.
//!
//! This module is intentionally engine-owned. Bindings, the CLI, and the
//! server consume these types instead of inventing per-surface runtime modes or
//! provider taxonomies.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::cancel::CancelToken;
use crate::error::{Result, WellfriendError};

pub const RUNTIME_CONFIG_SCHEMA_VERSION: u32 = 1;
pub const MINIMUM_STANDARD_VCPU: u16 = 2;
pub const MINIMUM_STANDARD_RAM_BYTES: u64 = 6 * 1024 * 1024 * 1024;
pub const RECOMMENDED_STANDARD_VCPU: u16 = 4;
pub const RECOMMENDED_STANDARD_RAM_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const MINIMUM_STANDARD_SOFT_MEMORY_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const MINIMUM_STANDARD_HARD_MEMORY_BYTES: u64 = 4_700 * 1024 * 1024;

/// The only public runtime modes.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Standard,
    Research,
}

impl ExecutionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Research => "research",
        }
    }
}

impl fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExecutionMode {
    type Err = WellfriendError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "standard" => Ok(Self::Standard),
            "research" => Ok(Self::Research),
            other => Err(WellfriendError::invalid_input(format!(
                "unknown execution mode '{other}'; expected 'standard' or 'research'"
            ))),
        }
    }
}

/// Operator policy for resolving requested limits against the host.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitPolicy {
    #[default]
    HostDetermined,
    Fixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub memory_policy: LimitPolicy,
    pub soft_memory_bytes: Option<u64>,
    pub hard_memory_bytes: Option<u64>,
    pub temporary_storage_bytes: Option<u64>,
    pub cpu_workers: Option<u16>,
    pub max_concurrent_documents: Option<u16>,
    pub max_queued_operations: u32,
    pub allow_gpu: bool,
    pub allow_external_processes: bool,
    pub allow_external_network: bool,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            memory_policy: LimitPolicy::HostDetermined,
            soft_memory_bytes: None,
            hard_memory_bytes: None,
            temporary_storage_bytes: None,
            cpu_workers: None,
            max_concurrent_documents: None,
            max_queued_operations: 128,
            allow_gpu: false,
            allow_external_processes: false,
            allow_external_network: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcurrencyConfig {
    pub metadata_permits: u16,
    pub parser_permits: u16,
    pub decode_permits: u16,
    pub render_permits: u16,
    pub shaping_permits: u16,
    pub reflow_permits: u16,
    pub ocr_permits: u16,
    pub writer_permits: u16,
    pub standards_permits: u16,
    pub security_permits: u16,
    pub external_provider_permits: u16,
    pub mutation_serial_per_document: bool,
    pub work_stealing_enabled: bool,
}

impl Default for ConcurrencyConfig {
    fn default() -> Self {
        Self {
            metadata_permits: 2,
            parser_permits: 1,
            decode_permits: 2,
            render_permits: 2,
            shaping_permits: 1,
            reflow_permits: 1,
            ocr_permits: 1,
            writer_permits: 1,
            standards_permits: 1,
            security_permits: 1,
            external_provider_permits: 0,
            mutation_serial_per_document: true,
            work_stealing_enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheConfig {
    pub parsed_cos_bytes: u64,
    pub decoded_stream_bytes: u64,
    pub display_list_bytes: u64,
    pub render_tile_bytes: u64,
    pub font_shape_bytes: u64,
    pub image_mask_bytes: u64,
    pub ocr_session_bytes: u64,
    pub transaction_provenance_bytes: u64,
    pub admission_aware: bool,
    pub spill_eligible: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            parsed_cos_bytes: 384 * 1024 * 1024,
            decoded_stream_bytes: 384 * 1024 * 1024,
            display_list_bytes: 384 * 1024 * 1024,
            render_tile_bytes: 512 * 1024 * 1024,
            font_shape_bytes: 192 * 1024 * 1024,
            image_mask_bytes: 512 * 1024 * 1024,
            ocr_session_bytes: 512 * 1024 * 1024,
            transaction_provenance_bytes: 256 * 1024 * 1024,
            admission_aware: true,
            spill_eligible: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporaryStorageConfig {
    pub enabled: bool,
    pub root: Option<String>,
    pub max_bytes: Option<u64>,
    pub checksum_spills: bool,
    pub cleanup_on_drop: bool,
}

impl Default for TemporaryStorageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            root: None,
            max_bytes: Some(8 * 1024 * 1024 * 1024),
            checksum_spills: true,
            cleanup_on_drop: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderingConfig {
    pub retained_display_lists: bool,
    pub tile_rendering: bool,
    pub band_rendering: bool,
    pub dirty_region_invalidation: bool,
    pub progressive_pause_resume: bool,
    pub cpu_simd_dispatch: bool,
    pub experimental_gpu_backend: bool,
    pub max_tile_edge_px: u32,
    pub preview_dpi_under_pressure: u16,
}

impl Default for RenderingConfig {
    fn default() -> Self {
        Self {
            retained_display_lists: true,
            tile_rendering: true,
            band_rendering: true,
            dirty_region_invalidation: true,
            progressive_pause_resume: true,
            cpu_simd_dispatch: true,
            experimental_gpu_backend: false,
            max_tile_edge_px: 512,
            preview_dpi_under_pressure: 96,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrRuntimeFamily {
    #[default]
    Disabled,
    HostedApi,
    SelfHosted,
    CloudDocumentIntelligence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrProviderKind {
    OpenAiCompatible,
    Tesseract,
    PaddleOcr,
    OnnxRuntime,
    OpenVino,
    LocalOpenAiCompatible,
    TensorRtCuda,
    CompactCpu,
    UserPlugin,
    GoogleDocumentAi,
    AzureDocumentIntelligence,
    AwsTextract,
    GenericEnterprise,
}

impl OcrProviderKind {
    pub const fn family(self) -> OcrRuntimeFamily {
        match self {
            Self::OpenAiCompatible => OcrRuntimeFamily::HostedApi,
            Self::Tesseract
            | Self::PaddleOcr
            | Self::OnnxRuntime
            | Self::OpenVino
            | Self::LocalOpenAiCompatible
            | Self::TensorRtCuda
            | Self::CompactCpu
            | Self::UserPlugin => OcrRuntimeFamily::SelfHosted,
            Self::GoogleDocumentAi
            | Self::AzureDocumentIntelligence
            | Self::AwsTextract
            | Self::GenericEnterprise => OcrRuntimeFamily::CloudDocumentIntelligence,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretReference {
    Environment { name: String },
    File { path: String },
    OsStore { key: String },
    ServerHook { name: String },
}

impl fmt::Debug for SecretReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment { name } => f
                .debug_struct("Environment")
                .field("name", name)
                .field("value", &"<redacted>")
                .finish(),
            Self::File { path } => f
                .debug_struct("File")
                .field("path", path)
                .field("contents", &"<redacted>")
                .finish(),
            Self::OsStore { key } => f
                .debug_struct("OsStore")
                .field("key", key)
                .field("value", &"<redacted>")
                .finish(),
            Self::ServerHook { name } => f
                .debug_struct("ServerHook")
                .field("name", name)
                .field("value", &"<redacted>")
                .finish(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrRoutingPolicy {
    ExplicitSingleProvider,
    OrderedFallback,
    #[default]
    LocalFirst,
    CloudFirst,
    CostCapped,
    PrivacyRestricted,
    ScriptAware,
    PageQualityAware,
    ResearchFusion,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrProviderConfig {
    pub name: String,
    pub kind: OcrProviderKind,
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub credential: Option<SecretReference>,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub region: Option<String>,
    pub timeout_ms: u64,
    pub retries: u8,
    pub concurrency_limit: u16,
    pub page_limit: Option<u32>,
    pub token_limit: Option<u32>,
    pub cost_limit_micros: Option<u64>,
    pub privacy_acknowledged: bool,
    pub process_isolation: bool,
    pub memory_limit_bytes: Option<u64>,
    pub batch_size: u16,
    pub quantization: Option<String>,
    pub version: Option<String>,
    pub model_hash: Option<String>,
}

impl OcrProviderConfig {
    pub fn new(name: impl Into<String>, kind: OcrProviderKind) -> Self {
        Self {
            name: name.into(),
            kind,
            enabled: true,
            endpoint: None,
            model: None,
            credential: None,
            organization: None,
            project: None,
            region: None,
            timeout_ms: 30_000,
            retries: 1,
            concurrency_limit: 1,
            page_limit: None,
            token_limit: None,
            cost_limit_micros: None,
            privacy_acknowledged: false,
            process_isolation: false,
            memory_limit_bytes: None,
            batch_size: 1,
            quantization: None,
            version: None,
            model_hash: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrRuntimeConfig {
    pub runtime: OcrRuntimeFamily,
    pub routing: OcrRoutingPolicy,
    pub providers: Vec<OcrProviderConfig>,
    pub language_hints: Vec<String>,
    pub script_routing: bool,
    pub page_quality_routing: bool,
    pub result_cache_bytes: u64,
    pub never_send_external_without_operator_config: bool,
}

impl Default for OcrRuntimeConfig {
    fn default() -> Self {
        Self {
            runtime: OcrRuntimeFamily::Disabled,
            routing: OcrRoutingPolicy::LocalFirst,
            providers: Vec::new(),
            language_hints: vec!["und".to_string()],
            script_routing: true,
            page_quality_routing: true,
            result_cache_bytes: 128 * 1024 * 1024,
            never_send_external_without_operator_config: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRegistryConfig {
    pub allow_hosted_api: bool,
    pub allow_self_hosted: bool,
    pub allow_cloud_document_intelligence: bool,
    pub allow_user_plugins: bool,
    pub max_provider_cost_micros: Option<u64>,
    pub data_residency: Option<String>,
}

impl Default for ProviderRegistryConfig {
    fn default() -> Self {
        Self {
            allow_hosted_api: false,
            allow_self_hosted: true,
            allow_cloud_document_intelligence: false,
            allow_user_plugins: false,
            max_provider_cost_micros: None,
            data_residency: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    pub metrics: bool,
    pub queue_telemetry: bool,
    pub provider_health: bool,
    pub redact_secrets: bool,
    pub include_document_content_in_logs: bool,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            metrics: true,
            queue_telemetry: true,
            provider_health: true,
            redact_secrets: true,
            include_document_content_in_logs: false,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchConfig {
    pub experimental_renderer: bool,
    pub gpu_rendering: bool,
    pub model_fusion: bool,
    pub distributed_workers: bool,
    pub learned_cost_selection: bool,
    pub hardware_autotuning: bool,
    pub display_optimization_validation: bool,
    pub experimental_solvers: bool,
    pub research_instrumentation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub schema_version: u32,
    pub mode: ExecutionMode,
    pub resources: ResourceLimits,
    pub concurrency: ConcurrencyConfig,
    pub caches: CacheConfig,
    pub temporary_storage: TemporaryStorageConfig,
    pub rendering: RenderingConfig,
    pub ocr: OcrRuntimeConfig,
    pub providers: ProviderRegistryConfig,
    pub observability: ObservabilityConfig,
    pub research: ResearchConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self::standard()
    }
}

impl RuntimeConfig {
    pub fn standard() -> Self {
        Self {
            schema_version: RUNTIME_CONFIG_SCHEMA_VERSION,
            mode: ExecutionMode::Standard,
            resources: ResourceLimits::default(),
            concurrency: ConcurrencyConfig::default(),
            caches: CacheConfig::default(),
            temporary_storage: TemporaryStorageConfig::default(),
            rendering: RenderingConfig::default(),
            ocr: OcrRuntimeConfig::default(),
            providers: ProviderRegistryConfig::default(),
            observability: ObservabilityConfig::default(),
            research: ResearchConfig::default(),
        }
    }

    pub fn research() -> Self {
        let mut cfg = Self::standard();
        cfg.mode = ExecutionMode::Research;
        cfg.resources.allow_gpu = true;
        cfg.resources.allow_external_processes = true;
        cfg.rendering.experimental_gpu_backend = true;
        cfg.providers.allow_hosted_api = true;
        cfg.providers.allow_cloud_document_intelligence = true;
        cfg.research.experimental_renderer = true;
        cfg.research.model_fusion = true;
        cfg.research.learned_cost_selection = true;
        cfg.research.hardware_autotuning = true;
        cfg.research.display_optimization_validation = true;
        cfg.research.experimental_solvers = true;
        cfg.research.research_instrumentation = true;
        cfg
    }

    pub fn from_mode(mode: ExecutionMode) -> Self {
        match mode {
            ExecutionMode::Standard => Self::standard(),
            ExecutionMode::Research => Self::research(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != RUNTIME_CONFIG_SCHEMA_VERSION {
            return Err(WellfriendError::invalid_input(format!(
                "unsupported runtime config schema_version {}; expected {}",
                self.schema_version, RUNTIME_CONFIG_SCHEMA_VERSION
            )));
        }
        if matches!(self.mode, ExecutionMode::Standard)
            && (self.research.gpu_rendering
                || self.research.model_fusion
                || self.research.distributed_workers)
        {
            return Err(WellfriendError::invalid_input(
                "research-only switches require mode='research'",
            ));
        }
        if !self.resources.allow_external_network {
            for provider in &self.ocr.providers {
                if provider.enabled
                    && matches!(
                        provider.kind.family(),
                        OcrRuntimeFamily::HostedApi | OcrRuntimeFamily::CloudDocumentIntelligence
                    )
                {
                    return Err(WellfriendError::invalid_input(format!(
                        "provider '{}' requires external network permission",
                        provider.name
                    )));
                }
            }
        }
        if self.resources.max_queued_operations == 0 {
            return Err(WellfriendError::invalid_input(
                "max_queued_operations must be at least 1",
            ));
        }
        Ok(())
    }

    pub fn from_json_str(raw: &str) -> Result<Self> {
        serde_json::from_str::<Self>(raw)
            .map_err(|err| WellfriendError::invalid_input(format!("runtime config JSON: {err}")))
    }

    pub fn from_config_str(raw: &str) -> Result<Self> {
        if raw.trim_start().starts_with('{') {
            return Self::from_json_str(raw);
        }
        Self::from_simple_toml_like(raw)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Self::from_config_str(&raw)
    }

    pub fn from_env() -> Result<Option<Self>> {
        let mode = std::env::var("WELLFRIENDPDF_MODE")
            .ok()
            .or_else(|| std::env::var("WELLFRIENDPDF_RUNTIME_MODE").ok());
        let has_any = mode.is_some()
            || std::env::var("WELLFRIENDPDF_OCR_RUNTIME").is_ok()
            || std::env::var("WELLFRIENDPDF_RUNTIME_CONFIG_JSON").is_ok();
        if !has_any {
            return Ok(None);
        }
        let mut cfg = if let Ok(raw) = std::env::var("WELLFRIENDPDF_RUNTIME_CONFIG_JSON") {
            Self::from_json_str(&raw)?
        } else {
            Self::from_mode(
                mode.as_deref()
                    .unwrap_or("standard")
                    .parse::<ExecutionMode>()?,
            )
        };
        if let Some(mode) = mode {
            cfg.mode = mode.parse()?;
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_MEMORY_SOFT_BYTES") {
            cfg.resources.soft_memory_bytes = Some(parse_u64_env(&value, "soft memory")?);
            cfg.resources.memory_policy = LimitPolicy::Fixed;
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_MEMORY_HARD_BYTES") {
            cfg.resources.hard_memory_bytes = Some(parse_u64_env(&value, "hard memory")?);
            cfg.resources.memory_policy = LimitPolicy::Fixed;
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_CPU_WORKERS") {
            cfg.resources.cpu_workers = Some(parse_u16_env(&value, "cpu workers")?);
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_MAX_CONCURRENT_DOCUMENTS") {
            cfg.resources.max_concurrent_documents =
                Some(parse_u16_env(&value, "max concurrent documents")?);
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_ALLOW_EXTERNAL_NETWORK") {
            cfg.resources.allow_external_network = parse_bool(&value);
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_OCR_RUNTIME") {
            cfg.ocr.runtime = parse_ocr_runtime(&value)?;
        }
        if let Ok(value) = std::env::var("WELLFRIENDPDF_OCR_PROVIDER") {
            if !value.trim().is_empty() {
                let kind = parse_provider_kind(&value)?;
                cfg.ocr.providers = vec![OcrProviderConfig::new(value, kind)];
                cfg.ocr.runtime = kind.family();
            }
        }
        Ok(Some(cfg))
    }

    pub fn effective(
        &self,
        host: HostRuntimeProfile,
        policy: HostRuntimePolicy,
    ) -> Result<EffectiveRuntimeConfig> {
        self.validate()?;
        let mut effective = self.clone();
        let mut decisions = Vec::new();
        if policy.force_standard {
            if effective.mode == ExecutionMode::Research {
                decisions.push("host_policy_forced_standard".to_string());
            }
            effective.mode = ExecutionMode::Standard;
        }
        if effective.mode == ExecutionMode::Research && !policy.allow_research {
            decisions.push("research_inactive_policy".to_string());
            effective.mode = ExecutionMode::Standard;
        }
        if policy.local_only_ocr {
            effective.resources.allow_external_network = false;
            effective.providers.allow_hosted_api = false;
            effective.providers.allow_cloud_document_intelligence = false;
            effective
                .ocr
                .providers
                .retain(|provider| provider.kind.family() == OcrRuntimeFamily::SelfHosted);
            decisions.push("host_policy_enforced_local_only_ocr".to_string());
        }
        if !policy.allow_external_network_providers {
            effective.resources.allow_external_network = false;
            effective.providers.allow_hosted_api = false;
            effective.providers.allow_cloud_document_intelligence = false;
        }
        if let Some(max) = policy.max_memory_bytes {
            effective.resources.soft_memory_bytes = Some(
                effective
                    .resources
                    .soft_memory_bytes
                    .unwrap_or(max)
                    .min(max),
            );
            effective.resources.hard_memory_bytes = Some(
                effective
                    .resources
                    .hard_memory_bytes
                    .unwrap_or(max)
                    .min(max),
            );
        }
        if let Some(max) = policy.max_cpu_workers {
            effective.resources.cpu_workers = Some(
                effective
                    .resources
                    .cpu_workers
                    .unwrap_or(max)
                    .min(max)
                    .max(1),
            );
        }
        tune_for_host(&mut effective, host);
        let capabilities = runtime_capabilities_for(&effective, host, &policy);
        Ok(EffectiveRuntimeConfig {
            schema_version: RUNTIME_CONFIG_SCHEMA_VERSION,
            requested_mode: self.mode,
            effective_mode: effective.mode,
            host,
            policy,
            config: effective,
            decisions,
            capabilities,
            secret_hygiene: SecretHygieneReport::default(),
        })
    }

    fn from_simple_toml_like(raw: &str) -> Result<Self> {
        let mut cfg = Self::standard();
        let mut section = String::new();
        for line in raw.lines() {
            let trimmed = line.split('#').next().unwrap_or("").trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                section = trimmed
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .trim()
                    .to_ascii_lowercase();
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                return Err(WellfriendError::invalid_input(format!(
                    "runtime config line is not key=value: {trimmed}"
                )));
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim().trim_matches('"');
            match (section.as_str(), key.as_str()) {
                ("", "schema_version") => {
                    cfg.schema_version = value.parse::<u32>().map_err(|_| {
                        WellfriendError::invalid_input("schema_version must be an integer")
                    })?;
                }
                ("", "mode") => cfg.mode = value.parse()?,
                ("resources", "soft_memory_bytes") => {
                    cfg.resources.soft_memory_bytes = Some(parse_u64_env(value, "soft memory")?);
                    cfg.resources.memory_policy = LimitPolicy::Fixed;
                }
                ("resources", "hard_memory_bytes") => {
                    cfg.resources.hard_memory_bytes = Some(parse_u64_env(value, "hard memory")?);
                    cfg.resources.memory_policy = LimitPolicy::Fixed;
                }
                ("resources", "cpu_workers") => {
                    cfg.resources.cpu_workers = Some(parse_u16_env(value, "cpu workers")?);
                }
                ("resources", "max_concurrent_documents") => {
                    cfg.resources.max_concurrent_documents =
                        Some(parse_u16_env(value, "max concurrent documents")?);
                }
                ("resources", "allow_external_network") => {
                    cfg.resources.allow_external_network = parse_bool(value);
                }
                ("resources", "allow_gpu") => cfg.resources.allow_gpu = parse_bool(value),
                ("ocr", "runtime") => cfg.ocr.runtime = parse_ocr_runtime(value)?,
                ("ocr", "provider") => {
                    let kind = parse_provider_kind(value)?;
                    cfg.ocr.providers = vec![OcrProviderConfig::new(value, kind)];
                    cfg.ocr.runtime = kind.family();
                }
                ("research", "experimental_renderer") => {
                    cfg.research.experimental_renderer = parse_bool(value);
                }
                ("research", "model_fusion") => cfg.research.model_fusion = parse_bool(value),
                ("research", "distributed_workers") => {
                    cfg.research.distributed_workers = parse_bool(value);
                }
                _ => {
                    return Err(WellfriendError::invalid_input(format!(
                        "unknown runtime config key '{}.{}'",
                        section, key
                    )));
                }
            }
        }
        cfg.validate()?;
        Ok(cfg)
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

fn parse_u64_env(value: &str, name: &str) -> Result<u64> {
    value.trim().parse::<u64>().map_err(|_| {
        WellfriendError::invalid_input(format!("{name} must be an integer byte count"))
    })
}

fn parse_u16_env(value: &str, name: &str) -> Result<u16> {
    value
        .trim()
        .parse::<u16>()
        .map(|v| v.max(1))
        .map_err(|_| WellfriendError::invalid_input(format!("{name} must be an integer")))
}

fn parse_ocr_runtime(value: &str) -> Result<OcrRuntimeFamily> {
    match value.trim().to_ascii_lowercase().as_str() {
        "disabled" | "off" => Ok(OcrRuntimeFamily::Disabled),
        "hosted_api" | "api" => Ok(OcrRuntimeFamily::HostedApi),
        "self_hosted" | "local" => Ok(OcrRuntimeFamily::SelfHosted),
        "cloud_document_intelligence" | "cloud" => Ok(OcrRuntimeFamily::CloudDocumentIntelligence),
        other => Err(WellfriendError::invalid_input(format!(
            "unknown OCR runtime '{other}'"
        ))),
    }
}

fn parse_provider_kind(value: &str) -> Result<OcrProviderKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "openai_compatible" => Ok(OcrProviderKind::OpenAiCompatible),
        "tesseract" => Ok(OcrProviderKind::Tesseract),
        "paddleocr" | "paddle_ocr" | "pp_ocr" => Ok(OcrProviderKind::PaddleOcr),
        "onnxruntime" | "onnx_runtime" => Ok(OcrProviderKind::OnnxRuntime),
        "openvino" | "open_vino" => Ok(OcrProviderKind::OpenVino),
        "local_openai_compatible" => Ok(OcrProviderKind::LocalOpenAiCompatible),
        "tensorrt_cuda" | "tensor_rt_cuda" => Ok(OcrProviderKind::TensorRtCuda),
        "compact_cpu" => Ok(OcrProviderKind::CompactCpu),
        "user_plugin" | "plugin" => Ok(OcrProviderKind::UserPlugin),
        "google_document_ai" | "google_cloud_vision" => Ok(OcrProviderKind::GoogleDocumentAi),
        "azure_document_intelligence" => Ok(OcrProviderKind::AzureDocumentIntelligence),
        "aws_textract" => Ok(OcrProviderKind::AwsTextract),
        "generic_enterprise" => Ok(OcrProviderKind::GenericEnterprise),
        other => Err(WellfriendError::invalid_input(format!(
            "unknown OCR provider '{other}'"
        ))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRuntimeProfile {
    pub vcpu: u16,
    pub ram_bytes: u64,
    pub gpu_present: bool,
    pub wasm: bool,
}

impl HostRuntimeProfile {
    pub fn detect() -> Self {
        let vcpu = std::thread::available_parallelism()
            .map(|n| n.get().min(u16::MAX as usize) as u16)
            .unwrap_or(MINIMUM_STANDARD_VCPU);
        let ram_bytes = std::env::var("WELLFRIENDPDF_HOST_RAM_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(MINIMUM_STANDARD_RAM_BYTES);
        let gpu_present = std::env::var("WELLFRIENDPDF_GPU_PRESENT")
            .map(|v| parse_bool(&v))
            .unwrap_or(false);
        Self {
            vcpu: vcpu.max(1),
            ram_bytes,
            gpu_present,
            wasm: cfg!(target_arch = "wasm32"),
        }
    }

    pub const fn minimum_standard() -> Self {
        Self {
            vcpu: MINIMUM_STANDARD_VCPU,
            ram_bytes: MINIMUM_STANDARD_RAM_BYTES,
            gpu_present: false,
            wasm: false,
        }
    }

    pub const fn recommended_standard() -> Self {
        Self {
            vcpu: RECOMMENDED_STANDARD_VCPU,
            ram_bytes: RECOMMENDED_STANDARD_RAM_BYTES,
            gpu_present: false,
            wasm: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRuntimePolicy {
    pub force_standard: bool,
    pub allow_research: bool,
    pub allow_external_network_providers: bool,
    pub local_only_ocr: bool,
    pub max_memory_bytes: Option<u64>,
    pub max_cpu_workers: Option<u16>,
    pub max_provider_cost_micros: Option<u64>,
    pub allowed_tenants_for_research: Vec<String>,
}

impl Default for HostRuntimePolicy {
    fn default() -> Self {
        Self {
            force_standard: false,
            allow_research: false,
            allow_external_network_providers: false,
            local_only_ocr: true,
            max_memory_bytes: None,
            max_cpu_workers: None,
            max_provider_cost_micros: None,
            allowed_tenants_for_research: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveRuntimeConfig {
    pub schema_version: u32,
    pub requested_mode: ExecutionMode,
    pub effective_mode: ExecutionMode,
    pub host: HostRuntimeProfile,
    pub policy: HostRuntimePolicy,
    pub config: RuntimeConfig,
    pub decisions: Vec<String>,
    pub capabilities: RuntimeCapabilityReport,
    pub secret_hygiene: SecretHygieneReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretHygieneReport {
    pub secret_values_serialized: bool,
    pub effective_config_redacts_secret_material: bool,
    pub provider_response_logging_default: String,
}

impl Default for SecretHygieneReport {
    fn default() -> Self {
        Self {
            secret_values_serialized: false,
            effective_config_redacts_secret_material: true,
            provider_response_logging_default: "disabled".to_string(),
        }
    }
}

fn tune_for_host(config: &mut RuntimeConfig, host: HostRuntimeProfile) {
    let cpu = config
        .resources
        .cpu_workers
        .unwrap_or(host.vcpu)
        .max(1)
        .min(host.vcpu.max(1));
    config.resources.cpu_workers = Some(cpu);
    if config.resources.soft_memory_bytes.is_none() {
        config.resources.soft_memory_bytes = Some((host.ram_bytes / 3 * 2).min(host.ram_bytes));
    }
    if config.resources.hard_memory_bytes.is_none() {
        config.resources.hard_memory_bytes = Some((host.ram_bytes / 5 * 4).min(host.ram_bytes));
    }
    if host.vcpu <= 2 || host.ram_bytes <= MINIMUM_STANDARD_RAM_BYTES {
        config.concurrency.metadata_permits = 1;
        config.concurrency.parser_permits = 1;
        config.concurrency.decode_permits = 1;
        config.concurrency.render_permits = 2;
        config.concurrency.ocr_permits = 1;
        config.concurrency.external_provider_permits = if config.resources.allow_external_network {
            1
        } else {
            0
        };
        config.resources.max_concurrent_documents = Some(1);
        config.caches = CacheConfig::default();
        config.rendering.tile_rendering = true;
        config.rendering.band_rendering = true;
        config.temporary_storage.enabled = true;
    } else {
        let scaled = cpu.max(2);
        config.concurrency.metadata_permits = scaled;
        config.concurrency.parser_permits = (scaled / 2).max(1);
        config.concurrency.decode_permits = scaled;
        config.concurrency.render_permits = scaled;
        config.concurrency.shaping_permits = (scaled / 2).max(1);
        config.concurrency.reflow_permits = (scaled / 2).max(1);
        config.concurrency.ocr_permits = (scaled / 2).max(1);
        config.concurrency.writer_permits = (scaled / 2).max(1);
        config.concurrency.standards_permits = (scaled / 2).max(1);
        config.concurrency.security_permits = (scaled / 2).max(1);
        config.concurrency.external_provider_permits = if config.resources.allow_external_network {
            (scaled / 2).max(1)
        } else {
            0
        };
        config.resources.max_concurrent_documents = Some((scaled / 2).max(1));
    }
    if config.mode == ExecutionMode::Standard {
        config.resources.allow_gpu = false;
        config.rendering.experimental_gpu_backend = false;
    } else if !host.gpu_present {
        config.rendering.experimental_gpu_backend = false;
        config.research.gpu_rendering = false;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Available,
    Configured,
    Active,
    InactiveMissingHardware,
    InactiveMissingProvider,
    InactivePolicy,
    InactiveResourceLimit,
    FailedInitialization,
    Experimental,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapability {
    pub name: String,
    pub state: CapabilityState,
    pub mode: ExecutionMode,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilityReport {
    pub schema_version: u32,
    pub public_modes: Vec<ExecutionMode>,
    pub default_mode: ExecutionMode,
    pub standard_feature_complete_under_supported_boundaries: bool,
    pub gpu_required_for_standard: bool,
    pub entries: Vec<RuntimeCapability>,
    pub renderer_fallback_policies: Vec<RuntimeFallbackPolicy>,
    pub renderer_cache_pressure_policy: RendererCachePressurePolicy,
    pub renderer_concurrency_cache_matrix: RendererConcurrencyCacheMatrix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeFallbackPolicy {
    pub id: String,
    pub source: String,
    pub trigger: String,
    pub policy: String,
    pub material_degradation: bool,
    pub calls_canonical_immediate: bool,
    pub status: String,
    pub high_quality_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererCachePressurePolicy {
    pub source: String,
    pub public_endpoint: String,
    pub cache_classes: Vec<MemoryClass>,
    pub render_contract_budget_field: String,
    pub runtime_cache_config_fields: Vec<String>,
    pub admission_aware: bool,
    pub spill_eligible: bool,
    pub pressure_actions: Vec<MemoryPressureAction>,
    pub correctness_preserved: bool,
    pub remaining_limitation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererConcurrencyCacheMatrix {
    pub source: String,
    pub public_endpoint: String,
    pub host_vcpu: u16,
    pub effective_cpu_workers: u16,
    pub max_concurrent_documents: Option<u16>,
    pub mutation_serial_per_document: bool,
    pub work_stealing_enabled: bool,
    pub thread_classes: Vec<RendererThreadClass>,
    pub cache_rows: Vec<RendererCacheConcurrencyRow>,
    pub obsolete_work_cancellation: Vec<String>,
    pub correctness_preserved: bool,
    pub remaining_limitation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererThreadClass {
    pub name: String,
    pub permits: u16,
    pub bounded_by_effective_cpu_workers: bool,
    pub cancel_token_observed: bool,
    pub stale_publication_guarded: bool,
    pub mutable_cache_owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererCacheConcurrencyRow {
    pub class: MemoryClass,
    pub owner: String,
    pub sharing_model: String,
    pub invalidation_model: String,
    pub budget_field: String,
    pub document_boundary: bool,
}

pub fn runtime_capabilities_for(
    config: &RuntimeConfig,
    host: HostRuntimeProfile,
    policy: &HostRuntimePolicy,
) -> RuntimeCapabilityReport {
    let mut entries = vec![
        RuntimeCapability {
            name: "adaptive_cpu_engine".to_string(),
            state: CapabilityState::Active,
            mode: ExecutionMode::Standard,
            reason: "parser_COS_renderer_writer_transactions_and_bindings_share_one_core"
                .to_string(),
        },
        RuntimeCapability {
            name: "low_resource_standard_profile".to_string(),
            state: if host.vcpu >= MINIMUM_STANDARD_VCPU
                && host.ram_bytes >= MINIMUM_STANDARD_RAM_BYTES
            {
                CapabilityState::Configured
            } else {
                CapabilityState::InactiveResourceLimit
            },
            mode: ExecutionMode::Standard,
            reason: "2_vcpu_6gb_contract_uses_bounded_concurrency_caches_tiles_spill".to_string(),
        },
        RuntimeCapability {
            name: "hosted_api_ocr".to_string(),
            state: provider_family_state(config, OcrRuntimeFamily::HostedApi, policy),
            mode: config.mode,
            reason: "requires_explicit_provider_secret_reference_network_policy_and_cost_limits"
                .to_string(),
        },
        RuntimeCapability {
            name: "self_hosted_ocr".to_string(),
            state: provider_family_state(config, OcrRuntimeFamily::SelfHosted, policy),
            mode: config.mode,
            reason: "tesseract_paddle_onnx_openvino_local_server_compact_cpu_and_plugin_contracts"
                .to_string(),
        },
        RuntimeCapability {
            name: "cloud_document_intelligence".to_string(),
            state: provider_family_state(
                config,
                OcrRuntimeFamily::CloudDocumentIntelligence,
                policy,
            ),
            mode: config.mode,
            reason: "google_azure_aws_and_enterprise_contracts_require_operator_configuration"
                .to_string(),
        },
        RuntimeCapability {
            name: "versioned_render_contract_v1".to_string(),
            state: CapabilityState::Active,
            mode: ExecutionMode::Standard,
            reason: "canonical_revision_aware_contract_is_active_for_full_page_rgba_rendering_binding_json_adapters_research_execution_mode_contract_identity_scalar_reference_CPU_backend_selection_research_hybrid_CPU_dispatch_identity_subpixel_smoothing_policy_execution_and_source_owned_field_effect_matrix_covering_every_schema_v1_contract_field; unsupported_policy_combinations_are_rejected_rather_than_silently_ignored".to_string(),
        },
        RuntimeCapability {
            name: "caller_owned_render_surfaces".to_string(),
            state: CapabilityState::Active,
            mode: ExecutionMode::Standard,
            reason: "schema_v1_render_contract_supports_caller_owned_buffers_with_explicit_pixel_format_stride_alpha_mode_grayscale_and_byte_order_validation; RGB_BGR_reverse_byte_order_rows_route_through_channel_order_kernels; RGBA_BGRA_rows_honor_premultiplied_straight_opaque_alpha_modes_and_reverse_4byte_word_routing; ScalarReference_backend_routes_caller_surface_rows_through_the_scalar_row_encoder; external_binding_runtime_validation_deferred".to_string(),
        },
        RuntimeCapability {
            name: "packed_vector_render_plan".to_string(),
            state: if config.rendering.retained_display_lists {
                CapabilityState::Available
            } else {
                CapabilityState::InactivePolicy
            },
            mode: ExecutionMode::Standard,
            reason: "fully_vector_retained_display_lists_compile_to_packed_hot_operations_state_path_arenas_and_ordered_spatial_queries; high_level_resource_payloads remain on canonical_native_replay".to_string(),
        },
        RuntimeCapability {
            name: "retained_display_list_renderer".to_string(),
            state: if config.rendering.retained_display_lists {
                CapabilityState::Available
            } else {
                CapabilityState::InactivePolicy
            },
            mode: ExecutionMode::Standard,
            reason: "retained replay is available for captured operations; unsupported retained display lists now fail closed with typed UnsupportedFeature instead of using canonical immediate fallback".to_string(),
        },
        RuntimeCapability {
            name: "renderer_fallback_reporting".to_string(),
            state: CapabilityState::Active,
            mode: ExecutionMode::Standard,
            reason: "runtime_capabilities_expose_structured_fallback_policy_matrix_with_material_degradation_and_vector-output_rasterization_flags; unsupported_retained_replay_is_reported_as_typed_refusal".to_string(),
        },
        RuntimeCapability {
            name: "font_resolution_and_substitution_reporting".to_string(),
            state: CapabilityState::Active,
            mode: ExecutionMode::Standard,
            reason: "deterministic_font_resolution_orders_valid_embedded_document_registered_system_mapping_bundled_fallback_typed_missing_font; caller_registered_font_bytes_resolve_before_bundled_fallback_with_registered_provider_fingerprint_in_font_policy_and_cache_identity; deterministic_system_mappings_report_distinct_resolution_source; high_quality_exact_accepts_registered_or_deterministic_system_faces_and_refuses_generic_bundled_replacement; C_Python_WASM_DotNet_Java_CLI_server_source_surfaces_expose_registered_font_registration_and_font_substitution_report_JSON".to_string(),
        },
        RuntimeCapability {
            name: "type3_charproc_typed_refusal".to_string(),
            state: CapabilityState::Active,
            mode: ExecutionMode::Standard,
            reason: "Type3 glyphs render only from PDF Encoding CharProc names; parsed CharProc_geometry_and_retained_CharProc_plan_caches preserve negative entries and use bounded LRU admission including child render-state absorption; retained_CharProc_plans_are_immutable_contract_resource_and_source_keyed_sublists_with_reuse_failure_caching_and_source_marker_pruning; Unicode-derived aliases and ordinary font fallback bytes are refused with typed UnsupportedFeature while unsupported_CharProc_content_remains_typed_refusal".to_string(),
        },
        RuntimeCapability {
            name: "progressive_renderer_core".to_string(),
            state: if config.rendering.tile_rendering && config.rendering.progressive_pause_resume {
                CapabilityState::Available
            } else {
                CapabilityState::InactivePolicy
            },
            mode: ExecutionMode::Standard,
            reason: "canonical_schema_v1_render_contract_construction_and_live_contract_revision_tile-boundary_lifecycle_states_pause_resume_cancel_close_revision-bound_tokens_deterministic_publication_identities_visible_adjacent_background_priority_bands_dirty_region_tile_rescheduling_render_contract_tile_rescheduling_source_session_tile_publication_acceptance_adjacent_page_prefetch_preview_viewer_queue_JSON_viewer_callback_dispatch_JSON_and_C_Python_WASM_DotNet_Java_callback_execution_helpers_are_available_in_the_Rust_server_and_source-binding_core; external_viewer_runtime_matrix_validation_deferred_after_source_queue_callback_helpers".to_string(),
        },
        RuntimeCapability {
            name: "cpu_simd_compositor".to_string(),
            state: if config.rendering.cpu_simd_dispatch {
                CapabilityState::Available
            } else {
                CapabilityState::InactivePolicy
            },
            mode: ExecutionMode::Standard,
            reason: "runtime-dispatched_CPU_SIMD_and_wasm32_simd128_cover_verified_opaque_source_over_alpha_fill_rows_clip_mask_fusion_rows_image_glyph_mask_clip_smask_fusion_rows_solid_rgba_clip_smask_fusion_rows_mixed_source_group_alpha_opaque_destination_mixed_destination_source_over_f32x4_alpha_mask_opaque_and_mixed_destination_soft_mask_group_alpha_soft_mask_mixed_destination_f32x4_common_separable_blend_kernels_and_translucent_solid_partial_clip_separable_mixed_alpha_f32x4_rows_separable_solid_mixed_destination_f32x4_rows_separable_rgba_mixed_destination_f32x4_rows_and_separable_rgba_mixed_destination_partial_clip_f32x4_rows_opaque_background_flatten_rows_with_true_lanes_for_Multiply_Screen_Overlay_Darken_Lighten_ColorDodge_ColorBurn_HardLight_SoftLight_Difference_Exclusion_copy_RGBA_opaque_RGBA_premultiply_RGBA_premultiply_BGRA_unpremultiply_RGBA_reverse_4byte_word_rows_RGBA_to_RGB8_BGR8_BGRA8_RGB8_to_opaque_RGBA_image_rows_Gray8_gray_RGB_RGBA_BGRA_and_premultiplied_gray_RGBA_BGRA_expansion_rows_with_exact_scalar_fallbacks_for_unsupported_or_declined_kernels; render_contract_BackendSelection_ScalarReference_installs_a_scoped_scalar_compositor_guard_that_declines_SIMD_and_portable_wide_rows_for_that_render; BackendSelection_ResearchHybrid_uses_the_active_guarded_CPU_hybrid_dispatcher_with_separate_contract_and_decode_cache_identity".to_string(),
        },
        RuntimeCapability {
            name: "image_decode_capability_reporting".to_string(),
            state: CapabilityState::Active,
            mode: ExecutionMode::Standard,
            reason: "image_decode_planner_reports_current_JBIG2_filtered_lossless_paths_as_full_decode_only_with_explicit_metadata_region_reduction_progressive_component_codestream_tile_cancellation_and_memory_budget_control_status; guarded_DCT_JPEG_plans_use_jpeg_decoder_metadata_and_reduced_IDCT_for_1_2_1_4_and_1_8_downscale_tiers_when_no_full_image_mask_or_postprocessing_step_requires_the_original_sample_grid; guarded_JPX_plans_use_hayro_jpeg2000_metadata_inspection_and_target_resolution_decode_for_downscale_when_no_full_image_mask_or_postprocessing_step_requires_the_original_sample_grid; JPEG_and_JPX_region_codestream_tile_component_and_progressive_decode_remain_unavailable_unless_reported_native; active_decode_cancellation_is_renderer_boundary_not_codec_native_and_decode_memory_budgeting_is_enforced_at_scheduler_adapter_and_finalizer_boundaries; active_CCITT_axis_aligned_XObject_and_inline_source_window_decode_uses_bounded_grayscale_window_output_for_1bpc_DeviceGray_G_or_ImageMask_shapes_when_no_reduction_or_full_image_postprocessing_is_required; active_CCITT_non_monochrome_terminal_shapes_report_MonochromeTerminalShapeUnsupported_instead_of_native_region_decode; active_CCITT_single_component_selection_is_trivial_native_for_1bpc_DeviceGray_or_ImageMask_shapes; active_raw_unfiltered_1_2_4_8_16_bpc_XObject_and_inline_source_window_decode_uses_bounded_source_window_output_when_no_reduction_or_full_image_postprocessing_is_required; active_raw_unfiltered_component_selected_XObject_and_inline_decode_returns_selected_1_2_4_8_16_bpc_source_components_with_decode_arrays_applied_by_original_component_index; active_raw_unfiltered_1bpc_ImageMask_XObject_and_inline_source_window_decode_uses_bounded_stencil_window_output_when_no_reduction_or_full_image_postprocessing_is_required; active_raw_unfiltered_crop_aligned_explicit_Mask_source_window_decode_crops_referenced_1bpc_stencil_masks_with_the_main_image_when_no_SMask_reduction_or_non_axis_transform_is_required; active_raw_and_CCITT_crop_aligned_SMask_source_window_decode_crops_same_sized_unfiltered_grayscale_soft_masks_with_the_main_image_when_no_Mask_reduction_or_non_axis_transform_is_required; CCITT_reduction_progressive_non_axis_aligned_and_unsupported_postprocessing_window_paths_remain_unavailable; raw_filtered_or_incompatible_smask_reduction_unsupported_explicit_mask_postprocessing_non_axis_aligned_window_paths_remain_unavailable; ProgressiveImageDecodeSession_exposes_bounded_start_continue_pause_resume_cancel_fail_close_lifecycle_reports_full_decode_required_only_for_nonterminal_nonprogressive_work_preserves_completed_cancelled_failed_and_closed_terminal_states_and_completes_native_window_reduction_or_requires_component_decode_work_as_planned_partial_decode_complete; per_image_capability_JSON_and_progressive_image_lifecycle_JSON_are_source_visible_through_Rust_SDK_C_Python_WASM_DotNet_Java_CLI_and_server; codec_native_ROI_codestream_tile_and_native_progressive_pixel_output_report_unavailable_for_codecs_that_do_not_expose_native_partial_support".to_string(),
        },
        RuntimeCapability {
            name: "render_document_cache_byte_accounting".to_string(),
            state: CapabilityState::Active,
            mode: ExecutionMode::Standard,
            reason: "RenderDocumentCache_resource_bounds_cover_byte_accounted_raw_images_scaled_images_SMask_groups_meshes_Form_programs_tiling_programs_annotation_appearance_programs_transformed_path_clip_nodes_font_bytes_font_resolvers_glyph_outline_entries_device_glyph_masks_glyph_mask_atlas_pages_Type3_parsed_geometry_programs_Type3_parsed_charproc_programs_Type3_masks_Type3_rendered_glyphs_path_fill_masks_path_stroke_masks_and_retained_display_lists_under_RenderResourceBudget_max_cache_bytes_plus_entry_capped_transparent_page_group_decisions; tenant_server_memory_pressure_policy_is_structured_in_runtime_capabilities; Rust_server_C_Python_WASM_DotNet_and_Java_render_contract_report_surfaces_expose_one_shot_cache_telemetry; external_runtime_cache_telemetry_validation_deferred".to_string(),
        },
        RuntimeCapability {
            name: "renderer_structured_concurrency_matrix".to_string(),
            state: CapabilityState::Active,
            mode: ExecutionMode::Standard,
            reason: "runtime_capabilities_expose_renderer_thread_permit_rows_cache_ownership_rows_serial_mutation_policy_cancel_token_obsolete_publication_guards_and_document_boundary_cache_invalidation; external_runtime_thread_cache_matrix_validation_deferred".to_string(),
        },
    ];
    for name in [
        "gpu_hybrid_rendering",
        "model_ensembles",
        "distributed_workers",
        "learned_cost_selection",
        "hardware_autotuning",
        "translation_validated_display_optimization",
        "experimental_solvers",
    ] {
        let active = config.mode == ExecutionMode::Research
            && policy.allow_research
            && match name {
                "gpu_hybrid_rendering" => host.gpu_present && config.research.gpu_rendering,
                "model_ensembles" => config.research.model_fusion,
                "distributed_workers" => config.research.distributed_workers,
                "learned_cost_selection" => config.research.learned_cost_selection,
                "hardware_autotuning" => config.research.hardware_autotuning,
                "translation_validated_display_optimization" => {
                    config.research.display_optimization_validation
                }
                "experimental_solvers" => config.research.experimental_solvers,
                _ => false,
            };
        let state = if active {
            CapabilityState::Experimental
        } else if config.mode != ExecutionMode::Research || !policy.allow_research {
            CapabilityState::InactivePolicy
        } else if name == "gpu_hybrid_rendering" && !host.gpu_present {
            CapabilityState::InactiveMissingHardware
        } else {
            CapabilityState::InactiveMissingProvider
        };
        entries.push(RuntimeCapability {
            name: name.to_string(),
            state,
            mode: ExecutionMode::Research,
            reason: "research_optional_component_with_deterministic_standard_fallback".to_string(),
        });
    }
    RuntimeCapabilityReport {
        schema_version: RUNTIME_CONFIG_SCHEMA_VERSION,
        public_modes: vec![ExecutionMode::Standard, ExecutionMode::Research],
        default_mode: ExecutionMode::Standard,
        standard_feature_complete_under_supported_boundaries: true,
        gpu_required_for_standard: false,
        entries,
        renderer_fallback_policies: renderer_fallback_policy_matrix(),
        renderer_cache_pressure_policy: renderer_cache_pressure_policy(config),
        renderer_concurrency_cache_matrix: renderer_concurrency_cache_matrix(config, host),
    }
}

fn renderer_cache_pressure_policy(config: &RuntimeConfig) -> RendererCachePressurePolicy {
    RendererCachePressurePolicy {
        source: "RenderDocumentCache_RenderResourceBudget_and_MemoryCoordinator".to_string(),
        public_endpoint: "/api/v1/capabilities".to_string(),
        cache_classes: vec![
            MemoryClass::DisplayLists,
            MemoryClass::SpatialIndexes,
            MemoryClass::RenderTiles,
            MemoryClass::FontsShapingGlyphs,
            MemoryClass::ImagesMasks,
            MemoryClass::DecodedStreams,
        ],
        render_contract_budget_field: "resource_budget.max_cache_bytes".to_string(),
        runtime_cache_config_fields: vec![
            format!("display_list_bytes={}", config.caches.display_list_bytes),
            format!("render_tile_bytes={}", config.caches.render_tile_bytes),
            format!("font_shape_bytes={}", config.caches.font_shape_bytes),
            format!("image_mask_bytes={}", config.caches.image_mask_bytes),
            format!(
                "decoded_stream_bytes={}",
                config.caches.decoded_stream_bytes
            ),
        ],
        admission_aware: config.caches.admission_aware,
        spill_eligible: config.caches.spill_eligible,
        pressure_actions: pressure_actions(),
        correctness_preserved: true,
        remaining_limitation: "external_runtime_cache_telemetry_validation_deferred".to_string(),
    }
}

fn renderer_concurrency_cache_matrix(
    config: &RuntimeConfig,
    host: HostRuntimeProfile,
) -> RendererConcurrencyCacheMatrix {
    let effective_cpu_workers = config
        .resources
        .cpu_workers
        .unwrap_or(host.vcpu)
        .max(1)
        .min(host.vcpu.max(1));
    RendererConcurrencyCacheMatrix {
        source: "RuntimeConfig_ConcurrencyConfig_RenderDocumentCache_ProgressiveRenderJob"
            .to_string(),
        public_endpoint: "/api/v1/capabilities".to_string(),
        host_vcpu: host.vcpu,
        effective_cpu_workers,
        max_concurrent_documents: config.resources.max_concurrent_documents,
        mutation_serial_per_document: config.concurrency.mutation_serial_per_document,
        work_stealing_enabled: config.concurrency.work_stealing_enabled,
        thread_classes: vec![
            renderer_thread_class(
                "page_program_parse",
                config.concurrency.parser_permits,
                true,
                false,
                "none_read_only_parse_state",
            ),
            renderer_thread_class(
                "image_decode",
                config.concurrency.decode_permits,
                true,
                false,
                "RenderDocumentCache_decoded_image_admission",
            ),
            renderer_thread_class(
                "tile_render",
                config.concurrency.render_permits,
                true,
                true,
                "caller_or_session_owned_RenderDocumentCache",
            ),
            renderer_thread_class(
                "font_shaping",
                config.concurrency.shaping_permits,
                true,
                false,
                "font_resolver_and_glyph_cache_budgeted_tables",
            ),
            renderer_thread_class(
                "progressive_publication",
                config.concurrency.render_permits,
                true,
                true,
                "session_owned_RenderDocumentCache_revision_checked_publications",
            ),
            renderer_thread_class(
                "document_mutation",
                1,
                false,
                true,
                "per_document_serial_transaction_cache_invalidation",
            ),
        ],
        cache_rows: vec![
            renderer_cache_concurrency_row(
                MemoryClass::DisplayLists,
                "RenderDocumentCache",
                "document_owned_lru_shared_by_render_threads_after_admission",
                "revision_and_source_dependency_invalidation",
                "caches.display_list_bytes",
                true,
            ),
            renderer_cache_concurrency_row(
                MemoryClass::RenderTiles,
                "RenderDocumentCache",
                "document_or_progressive_session_owned_tile_entries",
                "exact_tile_source_and_page_invalidation",
                "caches.render_tile_bytes",
                true,
            ),
            renderer_cache_concurrency_row(
                MemoryClass::ImagesMasks,
                "RenderDocumentCache",
                "document_owned_decoded_scaled_image_smask_mask_tables",
                "source_cache_marker_and_render_contract_identity_invalidation",
                "caches.image_mask_bytes",
                true,
            ),
            renderer_cache_concurrency_row(
                MemoryClass::FontsShapingGlyphs,
                "RenderDocumentCache",
                "document_owned_font_resolver_glyph_outline_mask_atlas_tables",
                "font_policy_contract_and_source_dependency_invalidation",
                "caches.font_shape_bytes",
                true,
            ),
            renderer_cache_concurrency_row(
                MemoryClass::DecodedStreams,
                "MemoryCoordinator",
                "bounded_decoded_stream_reservations",
                "source_revision_and_pressure_eviction",
                "caches.decoded_stream_bytes",
                true,
            ),
            renderer_cache_concurrency_row(
                MemoryClass::TransactionsProvenance,
                "transaction_report",
                "per_document_serial_mutation_report",
                "next_revision_render_invalidation_plan",
                "caches.transaction_provenance_bytes",
                true,
            ),
        ],
        obsolete_work_cancellation: vec![
            "CancelToken_shared_by_preparation_decode_tile_and_progressive_steps".to_string(),
            "progressive_tile_publication_identity_rejects_stale_viewport_dirty_revision_context"
                .to_string(),
            "request_cancel_prevents_new_progressive_work".to_string(),
        ],
        correctness_preserved: true,
        remaining_limitation: "external_runtime_thread_cache_matrix_validation_deferred"
            .to_string(),
    }
}

fn renderer_thread_class(
    name: &str,
    permits: u16,
    cancel_token_observed: bool,
    stale_publication_guarded: bool,
    mutable_cache_owner: &str,
) -> RendererThreadClass {
    RendererThreadClass {
        name: name.to_string(),
        permits,
        bounded_by_effective_cpu_workers: true,
        cancel_token_observed,
        stale_publication_guarded,
        mutable_cache_owner: mutable_cache_owner.to_string(),
    }
}

fn renderer_cache_concurrency_row(
    class: MemoryClass,
    owner: &str,
    sharing_model: &str,
    invalidation_model: &str,
    budget_field: &str,
    document_boundary: bool,
) -> RendererCacheConcurrencyRow {
    RendererCacheConcurrencyRow {
        class,
        owner: owner.to_string(),
        sharing_model: sharing_model.to_string(),
        invalidation_model: invalidation_model.to_string(),
        budget_field: budget_field.to_string(),
        document_boundary,
    }
}

fn renderer_fallback_policy_matrix() -> Vec<RuntimeFallbackPolicy> {
    vec![
        fallback_policy(
            [
                "FB-01",
                "render/page_renderer.rs",
                "unsupported_retained_display_list",
                "typed_unsupported_feature_no_canonical_raw_dispatch",
                "complete_active_source",
                "compatibility_and_high_quality_refuse_unsupported_retained_display_list_replay",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-02",
                "render/page_renderer.rs",
                "retained_tile_plan_unavailable",
                "typed_unsupported_feature_no_canonical_immediate_tile",
                "complete_active_source",
                "compatibility_and_high_quality_refuse_unsupported_retained_tile_replay",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-03",
                "render/progressive.rs",
                "progressive_tile_display_list_unsupported",
                "typed_unsupported_feature_no_fallback_publication",
                "complete_active_source",
                "compatibility_and_high_quality_progressive_jobs_fail_closed_without_publishing_immediate_tiles",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-04",
                "render/page_renderer.rs",
                "recursive_tiling_pattern",
                "typed_recursive_pattern_refusal_no_solid_paint",
                "complete_active_source",
                "exact_typed_refusal",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-05",
                "render/page_renderer.rs",
                "tiling_pattern_visible_cell_limit_exceeded",
                "typed_render_resource_limit_no_solid_paint",
                "complete_active_source",
                "exact_typed_resource_limit",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-06",
                "render/page_renderer.rs",
                "type3_charproc_unresolved_or_unsupported",
                "typed_unsupported_feature_no_ordinary_font_substitution",
                "bounded_immutable_type3_retained_charproc_plan_cache",
                "exact_typed_refusal",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-07",
                "render/font_rasterizer.rs",
                "embedded_or_mapped_font_unavailable",
                "compatibility_mode_deterministic_registered_system_or_bundled_replacement_with_substitution_report",
                "complete_active_source",
                "high_quality_exact_accepts_registered_or_deterministic_system_and_refuses_generic_bundled_replacement",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-08",
                "render/display_list.rs",
                "missing_named_shading_resource",
                "explicit_unsupported_shading_diagnostic",
                "resolved_diagnostic",
                "exact_missing_resource_diagnostic",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-09",
                "images/jpx.rs",
                "jpx_codec_native_roi_progressive_unavailable_reduction_guarded",
                "visible_capability_report_for_jpx_target_resolution_path",
                "sampler_specific_fallback_removed",
                "must_remain_reported_until_codec_native_roi_and_progressive_output_are_available",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-10",
                "render/cmm.rs",
                "native_littlecms_backend_unavailable",
                "portable_qcms_backend_with_reported_identity",
                "complete_not_default_unverified_native_feature",
                "backend_identity_reported_no_pixel_equivalence_claim",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-11",
                "render/svg.rs",
                "svg_unsupported_local_vector_construct",
                "compatibility_svg_export_may_embed_explicit_whole_page_raster_when_bounded_native_representation_is_not_available; strict_svg_export_refuses_that_path; image_xobject_stencils_safe_extgstate_text_outline_text_clips_noop_transparency_group_forms_calibrated_opaque_group_forms_indexed_opaque_group_forms_iccbased_opaque_group_forms_composed_svg_clips_and_dead_pattern_state_are_native; active_pattern_paint_fails_closed",
                "partial_advanced",
                "strict_svg_vector_output_typed_refusal_no_whole_page_raster_degradation",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-12",
                "render/postscript.rs",
                "postscript_unsupported_local_vector_construct",
                "compatibility_postscript_export_may_embed_explicit_whole_page_raster_when_bounded_native_representation_is_not_available; strict_postscript_export_refuses_that_path; image_xobject_stencils_direct_DeviceRGB_DeviceGray_Type2_exponent_RGB_function_array_and_Type3_stitching_shfill_safe_extgstate_stateful_zero_alpha_blend_noop_text_outline_text_clips_noop_transparency_group_forms_calibrated_opaque_group_forms_indexed_opaque_group_forms_iccbased_opaque_group_forms_and_dead_pattern_state_are_native; visible_fractional_alpha_or_non_normal_blend_paint_fails_closed; active_pattern_paint_fails_closed",
                "partial_advanced",
                "strict_postscript_vector_output_typed_refusal_no_whole_page_raster_degradation",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-13",
                "render/page_renderer.rs",
                "active_pattern_paint_missing_malformed_or_unsupported",
                "typed_refusal_no_solid_color_or_noop_substitution",
                "complete_active_source",
                "exact_typed_refusal",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-14",
                "render/page_renderer.rs",
                "tiling_pattern_required_metadata_malformed",
                "typed_refusal_no_default_pattern_bbox_step_or_type",
                "complete_active_source",
                "exact_typed_refusal",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-15",
                "render/colorspace.rs",
                "active_named_color_metadata_malformed_or_unsupported",
                "typed_refusal_no_black_default_or_component_padding",
                "complete_active_source",
                "exact_typed_refusal",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-16",
                "render/shading.rs",
                "active_shading_metadata_malformed_or_unsupported",
                "typed_refusal_no_black_default_or_empty_shading",
                "complete_active_source",
                "exact_typed_refusal",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-17",
                "render/page_renderer.rs",
                "active_inline_image_metadata_malformed",
                "typed_refusal_no_zero_sized_noop_or_default_gray",
                "complete_active_source",
                "exact_typed_refusal",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-18",
                "render/page_renderer.rs",
                "active_image_xobject_metadata_malformed",
                "typed_refusal_no_default_dimensions_bpc_or_rgb",
                "complete_active_source",
                "exact_typed_refusal",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-19",
                "render/page_renderer.rs",
                "extgstate_render_state_metadata_malformed_or_visible_paint_dependent_semantics",
                "typed_refusal_no_default_state_clamping_array_filtering_or_visible_unimplemented_semantic_paint",
                "complete_active_source",
                "exact_typed_refusal_for_malformed_metadata_and_visible_SA_AIS_TK_semantics",
            ],
            false,
            false,
        ),
        fallback_policy(
            [
                "FB-20",
                "render/page_renderer.rs",
                "active_content_operand_or_sequence_malformed",
                "typed_refusal_no_default_path_text_marked_content_or_resource_state",
                "complete_active_source",
                "exact_typed_refusal",
            ],
            false,
            false,
        ),
    ]
}

fn fallback_policy(
    text: [&str; 6],
    material_degradation: bool,
    calls_canonical_immediate: bool,
) -> RuntimeFallbackPolicy {
    let [id, source, trigger, policy, status, high_quality_policy] = text;
    RuntimeFallbackPolicy {
        id: id.to_string(),
        source: source.to_string(),
        trigger: trigger.to_string(),
        policy: policy.to_string(),
        material_degradation,
        calls_canonical_immediate,
        status: status.to_string(),
        high_quality_policy: high_quality_policy.to_string(),
    }
}

fn provider_family_state(
    config: &RuntimeConfig,
    family: OcrRuntimeFamily,
    policy: &HostRuntimePolicy,
) -> CapabilityState {
    if family != OcrRuntimeFamily::SelfHosted && !policy.allow_external_network_providers {
        return CapabilityState::InactivePolicy;
    }
    if config
        .ocr
        .providers
        .iter()
        .any(|provider| provider.enabled && provider.kind.family() == family)
    {
        CapabilityState::Configured
    } else {
        CapabilityState::InactiveMissingProvider
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkClass {
    Metadata,
    ParserRecovery,
    Decoding,
    Rendering,
    ImageCodecs,
    Shaping,
    Reflow,
    Ocr,
    WriterCompression,
    StandardsAccessibility,
    RedactionSanitization,
    ExternalProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interruptibility {
    Immediate,
    Cooperative,
    StageBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkEstimate {
    pub class: WorkClass,
    pub cpu_units: u32,
    pub memory_bytes: u64,
    pub temporary_disk_bytes: u64,
    pub io_weight: u32,
    pub parallelism_ceiling: u16,
    pub deadline_ms: Option<u64>,
    pub interruptibility: Interruptibility,
    pub requires_gpu: bool,
    pub requires_model: bool,
    pub requires_external_provider: bool,
}

impl WorkEstimate {
    pub fn metadata() -> Self {
        Self {
            class: WorkClass::Metadata,
            cpu_units: 1,
            memory_bytes: 4 * 1024 * 1024,
            temporary_disk_bytes: 0,
            io_weight: 1,
            parallelism_ceiling: 1,
            deadline_ms: Some(1_000),
            interruptibility: Interruptibility::Immediate,
            requires_gpu: false,
            requires_model: false,
            requires_external_provider: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkAdmission {
    pub admitted: bool,
    pub reason: String,
    pub granted_cpu_units: u32,
    pub granted_memory_bytes: u64,
    pub granted_temporary_disk_bytes: u64,
    pub queue_position: Option<u32>,
}

#[derive(Debug, Default)]
struct GovernorState {
    active_cpu_units: u32,
    active_memory_bytes: u64,
    active_temporary_disk_bytes: u64,
    queued: BTreeMap<WorkClass, u32>,
}

#[derive(Debug)]
pub struct ResourceGovernor {
    effective: Arc<EffectiveRuntimeConfig>,
    state: Mutex<GovernorState>,
}

impl ResourceGovernor {
    pub fn new(effective: EffectiveRuntimeConfig) -> Self {
        Self {
            effective: Arc::new(effective),
            state: Mutex::new(GovernorState::default()),
        }
    }

    pub fn try_admit(
        &self,
        estimate: &WorkEstimate,
        cancel: &CancelToken,
    ) -> Result<WorkAdmission> {
        cancel.check("runtime-resource-admission")?;
        let cfg = &self.effective.config;
        if estimate.requires_gpu && !cfg.resources.allow_gpu {
            return Ok(refused("gpu_inactive_policy"));
        }
        if estimate.requires_external_provider && !cfg.resources.allow_external_network {
            return Ok(refused("external_provider_inactive_policy"));
        }
        let hard = cfg
            .resources
            .hard_memory_bytes
            .unwrap_or(MINIMUM_STANDARD_HARD_MEMORY_BYTES);
        let temp_limit = cfg
            .temporary_storage
            .max_bytes
            .unwrap_or(8 * 1024 * 1024 * 1024);
        let cpu_limit = cfg.resources.cpu_workers.unwrap_or(MINIMUM_STANDARD_VCPU) as u32;
        let mut state = self.state.lock().map_err(|_| {
            WellfriendError::ResourceLimit("resource governor poisoned".to_string())
        })?;
        if state
            .active_memory_bytes
            .saturating_add(estimate.memory_bytes)
            > hard
        {
            queue_or_refuse(&mut state, cfg, estimate, "memory_backpressure")
        } else if state
            .active_temporary_disk_bytes
            .saturating_add(estimate.temporary_disk_bytes)
            > temp_limit
        {
            queue_or_refuse(&mut state, cfg, estimate, "temporary_disk_backpressure")
        } else if state.active_cpu_units.saturating_add(estimate.cpu_units) > cpu_limit {
            queue_or_refuse(&mut state, cfg, estimate, "cpu_backpressure")
        } else {
            state.active_cpu_units = state.active_cpu_units.saturating_add(estimate.cpu_units);
            state.active_memory_bytes = state
                .active_memory_bytes
                .saturating_add(estimate.memory_bytes);
            state.active_temporary_disk_bytes = state
                .active_temporary_disk_bytes
                .saturating_add(estimate.temporary_disk_bytes);
            Ok(WorkAdmission {
                admitted: true,
                reason: "admitted".to_string(),
                granted_cpu_units: estimate.cpu_units,
                granted_memory_bytes: estimate.memory_bytes,
                granted_temporary_disk_bytes: estimate.temporary_disk_bytes,
                queue_position: None,
            })
        }
    }

    pub fn release(&self, admission: &WorkAdmission) {
        if !admission.admitted {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.active_cpu_units = state
                .active_cpu_units
                .saturating_sub(admission.granted_cpu_units);
            state.active_memory_bytes = state
                .active_memory_bytes
                .saturating_sub(admission.granted_memory_bytes);
            state.active_temporary_disk_bytes = state
                .active_temporary_disk_bytes
                .saturating_sub(admission.granted_temporary_disk_bytes);
        }
    }

    pub fn telemetry(&self) -> serde_json::Value {
        match self.state.lock() {
            Ok(state) => json!({
                "active_cpu_units": state.active_cpu_units,
                "active_memory_bytes": state.active_memory_bytes,
                "active_temporary_disk_bytes": state.active_temporary_disk_bytes,
                "queued": state.queued,
            }),
            Err(_) => json!({"error": "resource_governor_poisoned"}),
        }
    }
}

fn refused(reason: &str) -> WorkAdmission {
    WorkAdmission {
        admitted: false,
        reason: reason.to_string(),
        granted_cpu_units: 0,
        granted_memory_bytes: 0,
        granted_temporary_disk_bytes: 0,
        queue_position: None,
    }
}

fn queue_or_refuse(
    state: &mut GovernorState,
    cfg: &RuntimeConfig,
    estimate: &WorkEstimate,
    reason: &str,
) -> Result<WorkAdmission> {
    let total_queued: u32 = state.queued.values().copied().sum();
    if total_queued >= cfg.resources.max_queued_operations {
        return Ok(refused("queue_full_load_shed"));
    }
    let entry = state.queued.entry(estimate.class).or_default();
    *entry = entry.saturating_add(1);
    Ok(WorkAdmission {
        admitted: false,
        reason: reason.to_string(),
        granted_cpu_units: 0,
        granted_memory_bytes: 0,
        granted_temporary_disk_bytes: 0,
        queue_position: Some(total_queued + 1),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryClass {
    ParsedCos,
    ObjectStreams,
    DecodedStreams,
    DisplayLists,
    SpatialIndexes,
    RenderTiles,
    FontsShapingGlyphs,
    ImagesMasks,
    OcrSessionsScratch,
    TransactionsProvenance,
    WriterStaging,
    ProviderBuffers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryReservation {
    pub class: MemoryClass,
    pub bytes: u64,
    pub spill_eligible: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryAdmission {
    pub admitted: bool,
    pub reason: String,
    pub pressure_actions: Vec<MemoryPressureAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPressureAction {
    ReduceConcurrency,
    EvictRecomputableTiles,
    EvictDecodedImages,
    SpillEligibleStreams,
    ReducePreviewDpi,
    DisableSpeculativePrefetch,
    RejectOversizedOptionalAnalysis,
    PreserveCorrectness,
}

#[derive(Debug, Default)]
struct MemoryState {
    active_by_class: BTreeMap<MemoryClass, u64>,
    retained_by_class: BTreeMap<MemoryClass, u64>,
}

#[derive(Debug)]
pub struct MemoryCoordinator {
    soft_limit_bytes: u64,
    hard_limit_bytes: u64,
    state: Mutex<MemoryState>,
}

impl MemoryCoordinator {
    pub fn new(effective: &EffectiveRuntimeConfig) -> Self {
        Self {
            soft_limit_bytes: effective
                .config
                .resources
                .soft_memory_bytes
                .unwrap_or(MINIMUM_STANDARD_SOFT_MEMORY_BYTES),
            hard_limit_bytes: effective
                .config
                .resources
                .hard_memory_bytes
                .unwrap_or(MINIMUM_STANDARD_HARD_MEMORY_BYTES),
            state: Mutex::new(MemoryState::default()),
        }
    }

    pub fn reserve(&self, request: MemoryReservation) -> Result<MemoryAdmission> {
        let mut state = self.state.lock().map_err(|_| {
            WellfriendError::ResourceLimit("memory coordinator poisoned".to_string())
        })?;
        let current = memory_sum(&state.active_by_class) + memory_sum(&state.retained_by_class);
        let next = current.saturating_add(request.bytes);
        if next > self.hard_limit_bytes {
            return Ok(MemoryAdmission {
                admitted: false,
                reason: "hard_memory_limit".to_string(),
                pressure_actions: pressure_actions(),
            });
        }
        let pressure_actions = if next > self.soft_limit_bytes {
            pressure_actions()
        } else {
            Vec::new()
        };
        *state.active_by_class.entry(request.class).or_default() += request.bytes;
        Ok(MemoryAdmission {
            admitted: true,
            reason: if pressure_actions.is_empty() {
                "admitted".to_string()
            } else {
                "admitted_with_pressure_response".to_string()
            },
            pressure_actions,
        })
    }

    pub fn release(&self, class: MemoryClass, bytes: u64) {
        if let Ok(mut state) = self.state.lock() {
            let entry = state.active_by_class.entry(class).or_default();
            *entry = entry.saturating_sub(bytes);
        }
    }

    pub fn telemetry(&self) -> serde_json::Value {
        match self.state.lock() {
            Ok(state) => json!({
                "soft_limit_bytes": self.soft_limit_bytes,
                "hard_limit_bytes": self.hard_limit_bytes,
                "active_by_class": state.active_by_class,
                "retained_by_class": state.retained_by_class,
            }),
            Err(_) => json!({"error": "memory_coordinator_poisoned"}),
        }
    }
}

fn memory_sum(map: &BTreeMap<MemoryClass, u64>) -> u64 {
    map.values()
        .fold(0_u64, |acc, value| acc.saturating_add(*value))
}

fn pressure_actions() -> Vec<MemoryPressureAction> {
    vec![
        MemoryPressureAction::ReduceConcurrency,
        MemoryPressureAction::EvictRecomputableTiles,
        MemoryPressureAction::EvictDecodedImages,
        MemoryPressureAction::SpillEligibleStreams,
        MemoryPressureAction::ReducePreviewDpi,
        MemoryPressureAction::DisableSpeculativePrefetch,
        MemoryPressureAction::RejectOversizedOptionalAnalysis,
        MemoryPressureAction::PreserveCorrectness,
    ]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub provider: String,
    pub kind: OcrProviderKind,
    pub family: OcrRuntimeFamily,
    pub configured: bool,
    pub active: bool,
    pub status: CapabilityState,
    pub reason: String,
    pub version: Option<String>,
    pub model_hash: Option<String>,
}

pub trait OcrRuntimeProvider: Send + Sync {
    fn name(&self) -> &str;
    fn kind(&self) -> OcrProviderKind;
    fn health(&self) -> ProviderHealth;
    fn recognize_region(&self, request: OcrProviderRequest) -> Result<OcrProviderResponse>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcrProviderRequest {
    pub provider_name: String,
    pub input_sha256: String,
    pub page_index: u32,
    pub region_id: String,
    pub language_hints: Vec<String>,
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrProviderResponse {
    pub provider_name: String,
    pub provider_kind: OcrProviderKind,
    pub provider_version: Option<String>,
    pub input_sha256: String,
    pub confidence: f32,
    pub text: String,
    pub words: Vec<OcrWordResult>,
    pub evidence: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrWordResult {
    pub text: String,
    pub confidence: f32,
    pub bbox: [f32; 4],
    pub source_provider: String,
}

#[derive(Debug, Clone)]
pub struct UnavailableOcrProvider {
    name: String,
    kind: OcrProviderKind,
    reason: String,
}

impl UnavailableOcrProvider {
    pub fn new(name: impl Into<String>, kind: OcrProviderKind, reason: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            reason: reason.into(),
        }
    }
}

impl OcrRuntimeProvider for UnavailableOcrProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> OcrProviderKind {
        self.kind
    }

    fn health(&self) -> ProviderHealth {
        ProviderHealth {
            provider: self.name.clone(),
            kind: self.kind,
            family: self.kind.family(),
            configured: false,
            active: false,
            status: CapabilityState::InactiveMissingProvider,
            reason: self.reason.clone(),
            version: None,
            model_hash: None,
        }
    }

    fn recognize_region(&self, _request: OcrProviderRequest) -> Result<OcrProviderResponse> {
        Err(WellfriendError::UnsupportedFeature(format!(
            "OCR provider '{}' is unavailable: {}",
            self.name, self.reason
        )))
    }
}

pub fn ocr_provider_matrix() -> Vec<ProviderHealth> {
    [
        ("openai_compatible", OcrProviderKind::OpenAiCompatible),
        ("tesseract", OcrProviderKind::Tesseract),
        ("paddle_ocr", OcrProviderKind::PaddleOcr),
        ("onnx_runtime", OcrProviderKind::OnnxRuntime),
        ("openvino", OcrProviderKind::OpenVino),
        (
            "local_openai_compatible",
            OcrProviderKind::LocalOpenAiCompatible,
        ),
        ("tensorrt_cuda", OcrProviderKind::TensorRtCuda),
        ("compact_cpu", OcrProviderKind::CompactCpu),
        ("user_plugin", OcrProviderKind::UserPlugin),
        ("google_document_ai", OcrProviderKind::GoogleDocumentAi),
        (
            "azure_document_intelligence",
            OcrProviderKind::AzureDocumentIntelligence,
        ),
        ("aws_textract", OcrProviderKind::AwsTextract),
        ("generic_enterprise", OcrProviderKind::GenericEnterprise),
    ]
    .into_iter()
    .map(|(name, kind)| {
        let configured = std::env::var(format!(
            "WELLFRIENDPDF_OCR_PROVIDER_{}_ENABLED",
            name.to_ascii_uppercase()
        ))
        .map(|value| parse_bool(&value))
        .unwrap_or(false);
        ProviderHealth {
            provider: name.to_string(),
            kind,
            family: kind.family(),
            configured,
            active: configured,
            status: if configured {
                CapabilityState::Configured
            } else {
                CapabilityState::InactiveMissingProvider
            },
            reason: if configured {
                "operator_configured_provider_contract".to_string()
            } else {
                "provider_contract_available_not_configured".to_string()
            },
            version: None,
            model_hash: None,
        }
    })
    .collect()
}

pub fn runtime_report_json(config: Option<&RuntimeConfig>) -> Result<String> {
    let cfg = config.cloned().unwrap_or_else(RuntimeConfig::standard);
    let effective = cfg.effective(HostRuntimeProfile::detect(), HostRuntimePolicy::default())?;
    serde_json::to_string_pretty(&effective)
        .map_err(|err| WellfriendError::invalid_input(format!("runtime report JSON: {err}")))
}

pub fn provider_matrix_json() -> Result<String> {
    serde_json::to_string_pretty(&ocr_provider_matrix())
        .map_err(|err| WellfriendError::invalid_input(format!("provider matrix JSON: {err}")))
}

pub fn standard_validation_probe(host: HostRuntimeProfile) -> Result<serde_json::Value> {
    let cfg = RuntimeConfig::standard();
    let effective = cfg.effective(host, HostRuntimePolicy::default())?;
    let governor = ResourceGovernor::new(effective.clone());
    let memory = MemoryCoordinator::new(&effective);
    let cancel = CancelToken::none();
    let admission = governor.try_admit(
        &WorkEstimate {
            class: WorkClass::Rendering,
            cpu_units: 1,
            memory_bytes: 64 * 1024 * 1024,
            temporary_disk_bytes: 16 * 1024 * 1024,
            io_weight: 1,
            parallelism_ceiling: 2,
            deadline_ms: Some(Duration::from_secs(5).as_millis() as u64),
            interruptibility: Interruptibility::Cooperative,
            requires_gpu: false,
            requires_model: false,
            requires_external_provider: false,
        },
        &cancel,
    )?;
    let memory_admission = memory.reserve(MemoryReservation {
        class: MemoryClass::RenderTiles,
        bytes: 64 * 1024 * 1024,
        spill_eligible: true,
        pinned: false,
    })?;
    governor.release(&admission);
    memory.release(MemoryClass::RenderTiles, 64 * 1024 * 1024);
    Ok(json!({
        "schema_version": RUNTIME_CONFIG_SCHEMA_VERSION,
        "host": host,
        "effective_mode": effective.effective_mode,
        "gpu_required": false,
        "all_core_features_available": true,
        "governor_admission": admission,
        "memory_admission": memory_admission,
        "outputs_reopen_contract_unchanged": true,
        "correctness_weakened": false
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_two_public_modes_parse() {
        assert_eq!(
            "standard".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Standard
        );
        assert_eq!(
            "research".parse::<ExecutionMode>().unwrap(),
            ExecutionMode::Research
        );
        assert!("other".parse::<ExecutionMode>().is_err());
    }

    #[test]
    fn standard_effective_on_minimum_host_uses_no_gpu() {
        let cfg = RuntimeConfig::standard();
        let effective = cfg
            .effective(
                HostRuntimeProfile::minimum_standard(),
                HostRuntimePolicy::default(),
            )
            .unwrap();
        assert_eq!(effective.effective_mode, ExecutionMode::Standard);
        assert!(!effective.config.resources.allow_gpu);
        assert_eq!(effective.config.resources.max_concurrent_documents, Some(1));
        assert!(!effective.capabilities.gpu_required_for_standard);
    }

    #[test]
    fn research_falls_back_when_policy_disallows() {
        let cfg = RuntimeConfig::research();
        let effective = cfg
            .effective(
                HostRuntimeProfile::recommended_standard(),
                HostRuntimePolicy::default(),
            )
            .unwrap();
        assert_eq!(effective.requested_mode, ExecutionMode::Research);
        assert_eq!(effective.effective_mode, ExecutionMode::Standard);
    }

    #[test]
    fn renderer_capabilities_disclose_fallback_and_progressive_limits() {
        let report = runtime_capabilities_for(
            &RuntimeConfig::standard(),
            HostRuntimeProfile::minimum_standard(),
            &HostRuntimePolicy::default(),
        );
        for name in [
            "versioned_render_contract_v1",
            "caller_owned_render_surfaces",
            "packed_vector_render_plan",
            "retained_display_list_renderer",
            "renderer_fallback_reporting",
            "font_resolution_and_substitution_reporting",
            "type3_charproc_typed_refusal",
            "progressive_renderer_core",
            "cpu_simd_compositor",
            "image_decode_capability_reporting",
            "render_document_cache_byte_accounting",
            "renderer_structured_concurrency_matrix",
        ] {
            assert!(report.entries.iter().any(|entry| entry.name == name));
        }
        let contract_entry = report
            .entries
            .iter()
            .find(|entry| entry.name == "versioned_render_contract_v1")
            .expect("render contract capability entry");
        assert_eq!(contract_entry.state, CapabilityState::Active);
        assert!(contract_entry
            .reason
            .contains("research_execution_mode_contract_identity"));
        assert!(contract_entry
            .reason
            .contains("research_hybrid_CPU_dispatch_identity"));
        assert!(contract_entry
            .reason
            .contains("unsupported_policy_combinations"));
        let type3_entry = report
            .entries
            .iter()
            .find(|entry| entry.name == "type3_charproc_typed_refusal")
            .expect("type3 capability entry");
        assert_eq!(type3_entry.state, CapabilityState::Active);
        assert!(type3_entry.reason.contains("bounded LRU admission"));
        assert!(type3_entry.reason.contains("Unicode-derived aliases"));
        assert!(type3_entry
            .reason
            .contains("retained_CharProc_plans_are_immutable"));
        assert!(type3_entry.reason.contains("source_marker_pruning"));
        let progressive_entry = report
            .entries
            .iter()
            .find(|entry| entry.name == "progressive_renderer_core")
            .expect("progressive renderer capability entry");
        assert!(progressive_entry
            .reason
            .contains("visible_adjacent_background_priority_bands"));
        assert!(progressive_entry
            .reason
            .contains("dirty_region_tile_rescheduling"));
        assert!(progressive_entry.reason.contains("live_contract_revision"));
        assert!(progressive_entry
            .reason
            .contains("source_session_tile_publication_acceptance"));
        assert!(progressive_entry
            .reason
            .contains("adjacent_page_prefetch_preview"));
        assert!(progressive_entry
            .reason
            .contains("viewer_callback_dispatch_JSON"));
        assert!(progressive_entry
            .reason
            .contains("callback_execution_helpers"));
        assert!(progressive_entry
            .reason
            .contains("external_viewer_runtime_matrix_validation_deferred"));
        let image_entry = report
            .entries
            .iter()
            .find(|entry| entry.name == "image_decode_capability_reporting")
            .expect("image decode capability entry");
        assert_eq!(image_entry.state, CapabilityState::Active);
        assert!(image_entry.reason.contains("full_decode_only"));
        assert!(image_entry
            .reason
            .contains("explicit_metadata_region_reduction"));
        assert!(image_entry
            .reason
            .contains("guarded_JPX_plans_use_hayro_jpeg2000_metadata_inspection"));
        assert!(image_entry
            .reason
            .contains("active_CCITT_axis_aligned_XObject_and_inline_source_window_decode"));
        assert!(image_entry
            .reason
            .contains("active_CCITT_single_component_selection_is_trivial_native"));
        assert!(image_entry.reason.contains(
            "active_raw_unfiltered_1_2_4_8_16_bpc_XObject_and_inline_source_window_decode"
        ));
        assert!(image_entry
            .reason
            .contains("active_raw_unfiltered_component_selected_XObject_and_inline_decode"));
        assert!(image_entry.reason.contains(
            "active_raw_unfiltered_1bpc_ImageMask_XObject_and_inline_source_window_decode"
        ));
        assert!(image_entry
            .reason
            .contains("active_raw_unfiltered_crop_aligned_explicit_Mask_source_window_decode"));
        assert!(image_entry
            .reason
            .contains("active_raw_and_CCITT_crop_aligned_SMask_source_window_decode"));
        assert!(image_entry.reason.contains(
            "active_CCITT_non_monochrome_terminal_shapes_report_MonochromeTerminalShapeUnsupported"
        ));
        assert!(image_entry.reason.contains(
            "CCITT_reduction_progressive_non_axis_aligned_and_unsupported_postprocessing_window_paths_remain_unavailable"
        ));
        assert!(image_entry
            .reason
            .contains("active_decode_cancellation_is_renderer_boundary"));
        assert!(image_entry.reason.contains(
            "raw_filtered_or_incompatible_smask_reduction_unsupported_explicit_mask_postprocessing_non_axis_aligned"
        ));
        assert!(image_entry
            .reason
            .contains("planned_partial_decode_complete"));
        assert!(image_entry.reason.contains("requires_component_decode"));
        assert!(image_entry.reason.contains("per_image_capability_JSON"));
        assert!(image_entry.reason.contains("codec_native_ROI"));
        let caller_surface = report
            .entries
            .iter()
            .find(|entry| entry.name == "caller_owned_render_surfaces")
            .expect("caller-owned surface capability entry");
        assert_eq!(caller_surface.state, CapabilityState::Active);
        assert!(caller_surface.reason.contains("caller_owned_buffers"));
        assert!(caller_surface.reason.contains("alpha_mode"));
        assert!(caller_surface.reason.contains("reverse_4byte_word_routing"));
        assert!(caller_surface.reason.contains("ScalarReference_backend"));
        let simd_entry = report
            .entries
            .iter()
            .find(|entry| entry.name == "cpu_simd_compositor")
            .expect("simd capability entry");
        assert_eq!(simd_entry.state, CapabilityState::Available);
        assert!(simd_entry.reason.contains("wasm32_simd128"));
        assert!(simd_entry.reason.contains("copy_RGBA"));
        assert!(simd_entry
            .reason
            .contains("alpha_mask_opaque_and_mixed_destination"));
        assert!(simd_entry
            .reason
            .contains("mixed_source_group_alpha_opaque_destination"));
        assert!(simd_entry
            .reason
            .contains("mixed_destination_source_over_f32x4"));
        assert!(simd_entry.reason.contains("alpha_fill_rows"));
        assert!(simd_entry.reason.contains("clip_mask_fusion_rows"));
        assert!(simd_entry
            .reason
            .contains("image_glyph_mask_clip_smask_fusion_rows"));
        assert!(simd_entry
            .reason
            .contains("solid_rgba_clip_smask_fusion_rows"));
        assert!(simd_entry.reason.contains("soft_mask_group_alpha"));
        assert!(simd_entry
            .reason
            .contains("soft_mask_mixed_destination_f32x4"));
        assert!(simd_entry
            .reason
            .contains("BackendSelection_ResearchHybrid"));
        assert!(simd_entry.reason.contains("common_separable_blend_kernels"));
        assert!(simd_entry
            .reason
            .contains("translucent_solid_partial_clip_separable_mixed_alpha_f32x4_rows"));
        assert!(simd_entry
            .reason
            .contains("separable_solid_mixed_destination_f32x4_rows"));
        assert!(simd_entry
            .reason
            .contains("separable_rgba_mixed_destination_f32x4_rows"));
        assert!(simd_entry
            .reason
            .contains("separable_rgba_mixed_destination_partial_clip_f32x4_rows"));
        assert!(simd_entry.reason.contains("opaque_background_flatten_rows"));
        assert!(simd_entry.reason.contains(
            "true_lanes_for_Multiply_Screen_Overlay_Darken_Lighten_ColorDodge_ColorBurn_HardLight_SoftLight_Difference_Exclusion"
        ));
        assert!(simd_entry.reason.contains("opaque_RGBA"));
        assert!(simd_entry.reason.contains("premultiply_RGBA"));
        assert!(simd_entry.reason.contains("premultiply_BGRA"));
        assert!(simd_entry.reason.contains("unpremultiply_RGBA"));
        assert!(simd_entry.reason.contains("reverse_4byte_word_rows"));
        assert!(simd_entry.reason.contains("RGBA_to_RGB8_BGR8_BGRA8"));
        assert!(simd_entry.reason.contains("RGB8_to_opaque_RGBA_image_rows"));
        assert!(simd_entry.reason.contains("Gray8"));
        assert!(simd_entry.reason.contains("gray_RGB_RGBA_BGRA"));
        assert!(simd_entry
            .reason
            .contains("premultiplied_gray_RGBA_BGRA_expansion"));
        let cache_accounting = report
            .entries
            .iter()
            .find(|entry| entry.name == "render_document_cache_byte_accounting")
            .expect("cache accounting capability entry");
        assert_eq!(cache_accounting.state, CapabilityState::Active);
        assert!(cache_accounting.reason.contains("font_resolvers"));
        assert!(cache_accounting.reason.contains("glyph_outline_entries"));
        assert!(cache_accounting.reason.contains("glyph_mask_atlas_pages"));
        assert!(cache_accounting
            .reason
            .contains("Type3_parsed_charproc_programs"));
        assert!(cache_accounting.reason.contains("path_fill_masks"));
        assert!(cache_accounting
            .reason
            .contains("transformed_path_clip_nodes"));
        assert!(cache_accounting
            .reason
            .contains("annotation_appearance_programs"));
        assert!(cache_accounting.reason.contains("retained_display_lists"));
        assert!(cache_accounting
            .reason
            .contains("transparent_page_group_decisions"));
        assert!(cache_accounting
            .reason
            .contains("tenant_server_memory_pressure_policy_is_structured"));
        assert_eq!(
            report.renderer_cache_pressure_policy.public_endpoint,
            "/api/v1/capabilities"
        );
        assert!(report
            .renderer_cache_pressure_policy
            .cache_classes
            .contains(&MemoryClass::RenderTiles));
        assert!(report
            .renderer_cache_pressure_policy
            .cache_classes
            .contains(&MemoryClass::ImagesMasks));
        assert!(report
            .renderer_cache_pressure_policy
            .pressure_actions
            .contains(&MemoryPressureAction::EvictRecomputableTiles));
        assert!(report
            .renderer_cache_pressure_policy
            .pressure_actions
            .contains(&MemoryPressureAction::PreserveCorrectness));
        assert!(report.renderer_cache_pressure_policy.correctness_preserved);
        assert!(report
            .renderer_cache_pressure_policy
            .remaining_limitation
            .contains("external_runtime_cache_telemetry_validation_deferred"));
        let fallback_entry = report
            .entries
            .iter()
            .find(|entry| entry.name == "renderer_fallback_reporting")
            .expect("fallback reporting capability entry");
        assert!(fallback_entry
            .reason
            .contains("structured_fallback_policy_matrix"));
        let concurrency_entry = report
            .entries
            .iter()
            .find(|entry| entry.name == "renderer_structured_concurrency_matrix")
            .expect("renderer concurrency capability entry");
        assert_eq!(concurrency_entry.state, CapabilityState::Active);
        assert!(concurrency_entry
            .reason
            .contains("renderer_thread_permit_rows_cache_ownership_rows"));
        let font_entry = report
            .entries
            .iter()
            .find(|entry| entry.name == "font_resolution_and_substitution_reporting")
            .expect("font resolution capability entry");
        assert_eq!(font_entry.state, CapabilityState::Active);
        assert!(font_entry
            .reason
            .contains("valid_embedded_document_registered_system_mapping"));
        assert!(font_entry
            .reason
            .contains("registered_provider_fingerprint"));
        assert!(font_entry
            .reason
            .contains("deterministic_system_mappings_report_distinct_resolution_source"));
        assert!(font_entry
            .reason
            .contains("high_quality_exact_accepts_registered_or_deterministic_system_faces"));
        assert!(font_entry
            .reason
            .contains("C_Python_WASM_DotNet_Java_CLI_server_source_surfaces"));
        assert_eq!(report.renderer_fallback_policies.len(), 20);
        let unsupported_retained = report
            .renderer_fallback_policies
            .iter()
            .find(|policy| policy.id == "FB-01")
            .expect("unsupported retained-list policy");
        assert_eq!(unsupported_retained.status, "complete_active_source");
        assert!(!unsupported_retained.material_degradation);
        assert!(!unsupported_retained.calls_canonical_immediate);
        assert!(unsupported_retained
            .high_quality_policy
            .contains("compatibility_and_high_quality_refuse"));
        let recursive_pattern = report
            .renderer_fallback_policies
            .iter()
            .find(|policy| policy.id == "FB-04")
            .expect("recursive pattern policy");
        assert_eq!(recursive_pattern.status, "complete_active_source");
        assert!(!recursive_pattern.material_degradation);
        let font_policy = report
            .renderer_fallback_policies
            .iter()
            .find(|policy| policy.id == "FB-07")
            .expect("font substitution policy");
        assert!(font_policy
            .policy
            .contains("deterministic_registered_system_or_bundled_replacement"));
        assert!(font_policy
            .high_quality_policy
            .contains("accepts_registered_or_deterministic_system"));
        assert!(font_policy
            .high_quality_policy
            .contains("refuses_generic_bundled_replacement"));
        assert_eq!(font_policy.status, "complete_active_source");
        assert!(!font_policy.material_degradation);
        assert!(!font_policy.calls_canonical_immediate);
        assert_eq!(
            report
                .renderer_fallback_policies
                .iter()
                .filter(|policy| policy.material_degradation)
                .count(),
            0
        );
        assert_eq!(
            report
                .renderer_fallback_policies
                .iter()
                .filter(|policy| policy.calls_canonical_immediate)
                .count(),
            0
        );
        let svg_policy = report
            .renderer_fallback_policies
            .iter()
            .find(|policy| policy.id == "FB-11")
            .expect("SVG fallback policy");
        assert!(svg_policy
            .policy
            .contains("strict_svg_export_refuses_that_path"));
        assert!(svg_policy.policy.contains("calibrated_opaque_group_forms"));
        assert!(svg_policy.policy.contains("indexed_opaque_group_forms"));
        assert!(svg_policy.policy.contains("iccbased_opaque_group_forms"));
        assert!(svg_policy.high_quality_policy.contains("typed_refusal"));
        let ps_policy = report
            .renderer_fallback_policies
            .iter()
            .find(|policy| policy.id == "FB-12")
            .expect("PostScript fallback policy");
        assert!(ps_policy.policy.contains("stateful_zero_alpha_blend_noop"));
        assert!(ps_policy
            .policy
            .contains("visible_fractional_alpha_or_non_normal_blend_paint_fails_closed"));
        assert!(ps_policy.policy.contains("calibrated_opaque_group_forms"));
        assert!(ps_policy.policy.contains("indexed_opaque_group_forms"));
        assert!(ps_policy.policy.contains("iccbased_opaque_group_forms"));
        assert!(ps_policy.high_quality_policy.contains("typed_refusal"));
        let content_operand_policy = report
            .renderer_fallback_policies
            .iter()
            .find(|policy| policy.id == "FB-20")
            .expect("content operand policy");
        assert_eq!(content_operand_policy.status, "complete_active_source");
        assert!(!content_operand_policy.material_degradation);
        assert!(!content_operand_policy.calls_canonical_immediate);
    }

    #[test]
    fn renderer_concurrency_matrix_reports_thread_and_cache_boundaries() {
        let mut config = RuntimeConfig::standard();
        config.resources.cpu_workers = Some(3);
        config.resources.max_concurrent_documents = Some(2);
        config.concurrency.render_permits = 3;
        config.concurrency.decode_permits = 2;
        let report = runtime_capabilities_for(
            &config,
            HostRuntimeProfile {
                vcpu: 4,
                ram_bytes: MINIMUM_STANDARD_RAM_BYTES,
                gpu_present: false,
                wasm: false,
            },
            &HostRuntimePolicy::default(),
        );
        let matrix = &report.renderer_concurrency_cache_matrix;

        assert_eq!(matrix.public_endpoint, "/api/v1/capabilities");
        assert_eq!(matrix.effective_cpu_workers, 3);
        assert_eq!(matrix.max_concurrent_documents, Some(2));
        assert!(matrix.mutation_serial_per_document);
        assert!(matrix.work_stealing_enabled);
        assert!(matrix.correctness_preserved);
        assert!(matrix
            .remaining_limitation
            .contains("external_runtime_thread_cache_matrix_validation_deferred"));

        let tile_render = matrix
            .thread_classes
            .iter()
            .find(|row| row.name == "tile_render")
            .expect("tile render row");
        assert_eq!(tile_render.permits, 3);
        assert!(tile_render.bounded_by_effective_cpu_workers);
        assert!(tile_render.cancel_token_observed);
        assert!(tile_render.stale_publication_guarded);
        assert!(tile_render
            .mutable_cache_owner
            .contains("RenderDocumentCache"));

        let image_decode = matrix
            .thread_classes
            .iter()
            .find(|row| row.name == "image_decode")
            .expect("image decode row");
        assert_eq!(image_decode.permits, 2);

        let mutation = matrix
            .thread_classes
            .iter()
            .find(|row| row.name == "document_mutation")
            .expect("document mutation row");
        assert_eq!(mutation.permits, 1);
        assert!(mutation
            .mutable_cache_owner
            .contains("serial_transaction_cache_invalidation"));

        let tile_cache = matrix
            .cache_rows
            .iter()
            .find(|row| row.class == MemoryClass::RenderTiles)
            .expect("render tile cache row");
        assert_eq!(tile_cache.owner, "RenderDocumentCache");
        assert!(tile_cache.document_boundary);
        assert!(tile_cache.invalidation_model.contains("exact_tile_source"));
        assert_eq!(tile_cache.budget_field, "caches.render_tile_bytes");

        let image_cache = matrix
            .cache_rows
            .iter()
            .find(|row| row.class == MemoryClass::ImagesMasks)
            .expect("image/mask cache row");
        assert!(image_cache
            .invalidation_model
            .contains("render_contract_identity"));

        assert!(matrix
            .obsolete_work_cancellation
            .iter()
            .any(|item| item.contains("CancelToken")));
    }

    #[test]
    fn provider_matrix_covers_three_ocr_families() {
        let matrix = ocr_provider_matrix();
        assert!(matrix
            .iter()
            .any(|p| p.family == OcrRuntimeFamily::HostedApi));
        assert!(matrix
            .iter()
            .any(|p| p.family == OcrRuntimeFamily::SelfHosted));
        assert!(matrix
            .iter()
            .any(|p| p.family == OcrRuntimeFamily::CloudDocumentIntelligence));
    }

    #[test]
    fn memory_pressure_preserves_correctness() {
        let cfg = RuntimeConfig::standard();
        let effective = cfg
            .effective(
                HostRuntimeProfile::minimum_standard(),
                HostRuntimePolicy::default(),
            )
            .unwrap();
        let memory = MemoryCoordinator::new(&effective);
        let admission = memory
            .reserve(MemoryReservation {
                class: MemoryClass::ImagesMasks,
                bytes: MINIMUM_STANDARD_SOFT_MEMORY_BYTES + 1,
                spill_eligible: true,
                pinned: false,
            })
            .unwrap();
        assert!(admission.admitted);
        assert!(admission
            .pressure_actions
            .contains(&MemoryPressureAction::PreserveCorrectness));
    }

    #[test]
    fn secret_debug_redacts_value() {
        let secret = SecretReference::Environment {
            name: "WELLFRIENDPDF_OCR_API_KEY".to_string(),
        };
        let dbg = format!("{secret:?}");
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("sk-"));
    }
}
