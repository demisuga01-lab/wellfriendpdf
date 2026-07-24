//! Bounded, replayable evidence handling for signature validation.
//!
//! This module deliberately separates transport from trust. A fetched
//! certificate, OCSP response, or CRL is only untrusted evidence until the
//! signature-validation pipeline authenticates and applies it to a specific
//! certificate and policy.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use thiserror::Error;

use crate::cancel::CancelToken;

/// Current portable evidence-bundle schema.
pub const EVIDENCE_BUNDLE_SCHEMA_VERSION: u32 = 1;

/// Class of bytes held by an evidence record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Certificate,
    Ocsp,
    Crl,
}

impl EvidenceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Certificate => "certificate",
            Self::Ocsp => "ocsp",
            Self::Crl => "crl",
        }
    }
}

/// Purpose of a controlled retrieval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalKind {
    AiaIssuer,
    Ocsp,
    Crl,
}

impl RetrievalKind {
    fn evidence_kind(self) -> EvidenceKind {
        match self {
            Self::AiaIssuer => EvidenceKind::Certificate,
            Self::Ocsp => EvidenceKind::Ocsp,
            Self::Crl => EvidenceKind::Crl,
        }
    }
}

/// HTTP method deliberately permitted by the retrieval layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMethod {
    Get,
    Post,
}

/// OCSP nonce behavior for online retrieval. Disabled is the interoperable
/// default; required mode creates a fresh nonce for every request and rejects
/// a response that does not echo it exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OcspNoncePolicy {
    #[default]
    Disabled,
    Required,
}

/// Resource caps for one validation session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkBudget {
    pub max_aia_requests: usize,
    pub max_ocsp_requests: usize,
    pub max_crl_requests: usize,
    pub max_total_requests: usize,
    pub max_redirects: usize,
    pub max_response_bytes: usize,
    pub max_total_response_bytes: usize,
    pub max_cache_entries: usize,
    pub max_cache_bytes: usize,
    pub connect_timeout_ms: u64,
    pub total_timeout_ms: u64,
    /// Deadline for the complete AIA/OCSP/CRL retrieval session, not one HTTP
    /// request. A validation cannot multiply per-request timeouts indefinitely.
    pub max_total_time_ms: u64,
}

impl Default for NetworkBudget {
    fn default() -> Self {
        Self {
            max_aia_requests: 4,
            max_ocsp_requests: 8,
            max_crl_requests: 8,
            max_total_requests: 16,
            max_redirects: 3,
            max_response_bytes: 8 * 1024 * 1024,
            max_total_response_bytes: 32 * 1024 * 1024,
            max_cache_entries: 64,
            max_cache_bytes: 64 * 1024 * 1024,
            connect_timeout_ms: 5_000,
            total_timeout_ms: 20_000,
            max_total_time_ms: 30_000,
        }
    }
}

impl NetworkBudget {
    /// Reject nonsensical or potentially unbounded resource configurations.
    pub fn validate(&self) -> Result<(), EvidenceError> {
        if self.max_total_requests == 0
            || self.max_response_bytes == 0
            || self.max_total_response_bytes == 0
            || self.max_response_bytes > self.max_total_response_bytes
            || self.connect_timeout_ms == 0
            || self.total_timeout_ms == 0
            || self.max_total_time_ms == 0
            || self.connect_timeout_ms > self.total_timeout_ms
        {
            return Err(EvidenceError::InvalidBudget);
        }
        Ok(())
    }
}

/// Explicit transport policy. The default is strictly offline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetrievalPolicy {
    pub enabled: bool,
    pub allow_http: bool,
    pub allow_https: bool,
    pub allowed_hosts: Vec<String>,
    pub denied_hosts: Vec<String>,
    pub allow_private_network: bool,
    pub allow_non_default_ports: bool,
    pub allowed_ports: Vec<u16>,
    pub allow_cross_origin_redirects: bool,
    /// Optional caller-selected directory for an atomic, content-checked cache
    /// of evidence that has already passed cryptographic validation. The cache
    /// is never enabled implicitly and does not turn retrieved certificates
    /// into trust anchors.
    pub cache_directory: Option<String>,
    /// Whether online OCSP requests must carry a fresh nonce and responses
    /// must echo it. This deliberately does not apply to caller-supplied or
    /// replayed evidence, which has no request transaction to bind to.
    pub ocsp_nonce_policy: OcspNoncePolicy,
    pub budget: NetworkBudget,
}

impl Default for RetrievalPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_http: true,
            allow_https: true,
            allowed_hosts: Vec::new(),
            denied_hosts: Vec::new(),
            allow_private_network: false,
            allow_non_default_ports: false,
            allowed_ports: Vec::new(),
            allow_cross_origin_redirects: false,
            cache_directory: None,
            ocsp_nonce_policy: OcspNoncePolicy::Disabled,
            budget: NetworkBudget::default(),
        }
    }
}

impl RetrievalPolicy {
    /// A policy that permits no connection attempts.
    pub fn offline() -> Self {
        Self::default()
    }

    /// A bounded default suitable for callers that consciously opt into HTTP
    /// and HTTPS evidence retrieval.
    pub fn online() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    /// Validate policy values before starting a session.
    pub fn validate(&self) -> Result<(), EvidenceError> {
        self.budget.validate()?;
        if self.enabled && !self.allow_http && !self.allow_https {
            return Err(EvidenceError::NoAllowedSchemes);
        }
        if self
            .cache_directory
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(EvidenceError::InvalidCachePath);
        }
        Ok(())
    }
}

/// Content-addressed evidence record suitable for export and offline replay.
///
/// `bytes_hex` intentionally keeps the portable format dependency-free. It is
/// bounded by the importing store and never makes evidence trusted by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub kind: EvidenceKind,
    pub sha256: String,
    pub bytes_hex: String,
    pub source_uri: Option<String>,
    /// SHA-256 of the full normalized source URI. This is a cache identity
    /// only: ordinary reports retain the redacted `source_uri` and never emit
    /// credentials, query values, or fragments. Keeping the identity separate
    /// avoids aliasing two responder resources whose public display URI is the
    /// same after query redaction.
    #[serde(default)]
    pub source_uri_sha256: Option<String>,
    pub retrieved_at_unix: Option<u64>,
    pub content_type: Option<String>,
    /// Hash of the OCSP request body that obtained this response. It prevents
    /// one responder URI from serving cached status for a different CertID.
    /// AIA and CRL records have no request body and keep this `None`.
    #[serde(default)]
    pub request_body_sha256: Option<String>,
    pub validated_at_acquisition: bool,
}

impl EvidenceRecord {
    /// Construct a record while computing its immutable content identifier.
    pub fn from_bytes(
        kind: EvidenceKind,
        bytes: &[u8],
        source_uri: Option<String>,
        retrieved_at_unix: Option<u64>,
        content_type: Option<String>,
        request_body_sha256: Option<String>,
        validated_at_acquisition: bool,
    ) -> Self {
        Self {
            kind,
            sha256: sha256_hex(bytes),
            bytes_hex: hex_encode(bytes),
            source_uri_sha256: source_uri.as_deref().map(|uri| sha256_hex(uri.as_bytes())),
            source_uri: source_uri.map(|uri| redact_uri_text(&uri)),
            retrieved_at_unix,
            content_type,
            request_body_sha256,
            validated_at_acquisition,
        }
    }

    /// Decode and integrity-check the stored raw bytes.
    pub fn bytes(&self) -> Result<Vec<u8>, EvidenceError> {
        let bytes = hex_decode(&self.bytes_hex).map_err(|_| EvidenceError::InvalidEvidenceHex)?;
        if sha256_hex(&bytes) != self.sha256 {
            return Err(EvidenceError::EvidenceHashMismatch);
        }
        Ok(bytes)
    }

    fn id(&self) -> String {
        match self.request_body_sha256.as_deref() {
            Some(request_hash) => {
                format!("{}:{}:{}", self.kind.as_str(), self.sha256, request_hash)
            }
            None => format!("{}:{}", self.kind.as_str(), self.sha256),
        }
    }
}

/// Portable bundle for deterministic offline evidence replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EvidenceBundle {
    pub schema_version: u32,
    pub source_document_sha256: Option<String>,
    pub signature_identifier: Option<String>,
    pub validation_time_unix: Option<u64>,
    pub policy_sha256: Option<String>,
    pub records: Vec<EvidenceRecord>,
}

impl Default for EvidenceBundle {
    fn default() -> Self {
        Self {
            schema_version: EVIDENCE_BUNDLE_SCHEMA_VERSION,
            source_document_sha256: None,
            signature_identifier: None,
            validation_time_unix: None,
            policy_sha256: None,
            records: Vec::new(),
        }
    }
}

impl EvidenceBundle {
    /// Check schema, duplicate identity, raw hashes, and bounded total size.
    pub fn validate(&self, max_entries: usize, max_bytes: usize) -> Result<(), EvidenceError> {
        if self.schema_version != EVIDENCE_BUNDLE_SCHEMA_VERSION {
            return Err(EvidenceError::UnsupportedBundleSchema(self.schema_version));
        }
        if self.records.len() > max_entries {
            return Err(EvidenceError::EvidenceLimitExceeded);
        }
        let mut total = 0usize;
        let mut seen = BTreeMap::new();
        for record in &self.records {
            let bytes = record.bytes()?;
            total = total
                .checked_add(bytes.len())
                .ok_or(EvidenceError::EvidenceLimitExceeded)?;
            if total > max_bytes {
                return Err(EvidenceError::EvidenceLimitExceeded);
            }
            if seen.insert(record.id(), ()).is_some() {
                return Err(EvidenceError::DuplicateEvidence);
            }
        }
        Ok(())
    }
}

/// Bounded in-memory evidence cache. It only serves records marked validated
/// at acquisition; validators must still evaluate freshness and applicability
/// at use time.
#[derive(Debug, Clone)]
pub struct EvidenceStore {
    max_entries: usize,
    max_bytes: usize,
    records: BTreeMap<String, EvidenceRecord>,
}

impl Default for EvidenceStore {
    fn default() -> Self {
        let budget = NetworkBudget::default();
        Self::new(budget.max_cache_entries, budget.max_cache_bytes)
    }
}

impl EvidenceStore {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            max_entries,
            max_bytes,
            records: BTreeMap::new(),
        }
    }

    pub fn import_bundle(
        bundle: &EvidenceBundle,
        max_entries: usize,
        max_bytes: usize,
    ) -> Result<Self, EvidenceError> {
        bundle.validate(max_entries, max_bytes)?;
        let mut store = Self::new(max_entries, max_bytes);
        for record in &bundle.records {
            store.insert(record.clone())?;
        }
        Ok(store)
    }

    pub fn export_bundle(
        &self,
        source_document_sha256: Option<String>,
        signature_identifier: Option<String>,
        validation_time_unix: Option<u64>,
        policy_sha256: Option<String>,
    ) -> EvidenceBundle {
        EvidenceBundle {
            schema_version: EVIDENCE_BUNDLE_SCHEMA_VERSION,
            source_document_sha256,
            signature_identifier,
            validation_time_unix,
            policy_sha256,
            // Do not export arbitrary imported or rejected bytes merely
            // because they happened to pass through the store. A replay
            // bundle contains only evidence that this pipeline accepted at
            // acquisition; imported items are still revalidated when used.
            records: self
                .records
                .values()
                .filter(|record| record.validated_at_acquisition)
                .cloned()
                .collect(),
        }
    }

    pub fn insert(&mut self, mut record: EvidenceRecord) -> Result<(), EvidenceError> {
        // Imported bundles are untrusted input. Keep provenance useful while
        // ensuring reports and cache traces cannot echo URL credentials,
        // query secrets, or fragments supplied by a bundle producer.
        record.source_uri = record.source_uri.map(|uri| redact_uri_text(&uri));
        let bytes = record.bytes()?;
        let id = record.id();
        if self.records.contains_key(&id) {
            return Ok(());
        }
        let next_count = self.records.len().saturating_add(1);
        let next_bytes = self
            .total_bytes()
            .checked_add(bytes.len())
            .ok_or(EvidenceError::EvidenceLimitExceeded)?;
        if next_count > self.max_entries || next_bytes > self.max_bytes {
            return Err(EvidenceError::EvidenceLimitExceeded);
        }
        self.records.insert(id, record);
        Ok(())
    }

    pub fn records(&self) -> impl Iterator<Item = &EvidenceRecord> {
        self.records.values()
    }

    pub fn total_bytes(&self) -> usize {
        self.records
            .values()
            .filter_map(|record| record.bytes().ok())
            .map(|bytes| bytes.len())
            .sum()
    }

    fn find_validated(
        &self,
        kind: EvidenceKind,
        source_uri: &str,
        request_body_sha256: Option<&str>,
    ) -> Option<EvidenceRecord> {
        let source_uri_sha256 = sha256_hex(source_uri.as_bytes());
        self.records
            .values()
            .find(|record| {
                record.kind == kind
                    && record.validated_at_acquisition
                    && record.source_uri_sha256.as_deref() == Some(source_uri_sha256.as_str())
                    && record.request_body_sha256.as_deref() == request_body_sha256
            })
            .cloned()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_bundle_atomically(
        &self,
        path: &std::path::Path,
        source_document_sha256: Option<String>,
        signature_identifier: Option<String>,
        validation_time_unix: Option<u64>,
        policy_sha256: Option<String>,
    ) -> Result<(), EvidenceError> {
        let parent = path.parent().ok_or(EvidenceError::InvalidCachePath)?;
        std::fs::create_dir_all(parent).map_err(EvidenceError::CacheIo)?;
        if path.exists() {
            return Err(EvidenceError::CacheAlreadyExists);
        }
        let bundle = self.export_bundle(
            source_document_sha256,
            signature_identifier,
            validation_time_unix,
            policy_sha256,
        );
        let encoded = serde_json::to_vec_pretty(&bundle).map_err(EvidenceError::BundleJson)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(EvidenceError::InvalidCachePath)?;
        let temporary = parent.join(format!(".{file_name}.wellfriendpdf-partial"));
        if temporary.exists() {
            return Err(EvidenceError::CacheAlreadyExists);
        }
        std::fs::write(&temporary, encoded).map_err(EvidenceError::CacheIo)?;
        std::fs::rename(&temporary, path).map_err(EvidenceError::CacheIo)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_bundle(
        path: &std::path::Path,
        max_entries: usize,
        max_bytes: usize,
    ) -> Result<Self, EvidenceError> {
        let bytes = std::fs::read(path).map_err(EvidenceError::CacheIo)?;
        if bytes.len() > max_bytes.saturating_mul(3) {
            return Err(EvidenceError::EvidenceLimitExceeded);
        }
        let bundle: EvidenceBundle =
            serde_json::from_slice(&bytes).map_err(EvidenceError::BundleJson)?;
        Self::import_bundle(&bundle, max_entries, max_bytes)
    }

    /// Persist only cryptographically accepted evidence in a fixed, atomic
    /// cache file inside a caller-selected directory. Unlike explicit evidence
    /// export, a cache may be replaced as fresher validated responses arrive.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn write_cache_atomically(&self, directory: &std::path::Path) -> Result<(), EvidenceError> {
        if directory.as_os_str().is_empty() {
            return Err(EvidenceError::InvalidCachePath);
        }
        std::fs::create_dir_all(directory).map_err(EvidenceError::CacheIo)?;
        let destination = directory.join("validated-evidence-cache.json");
        let temporary = directory.join(".validated-evidence-cache.json.wellfriendpdf-partial");
        if temporary.exists() {
            return Err(EvidenceError::CacheAlreadyExists);
        }
        let bundle = self.export_bundle(None, None, None, None);
        let encoded = serde_json::to_vec_pretty(&bundle).map_err(EvidenceError::BundleJson)?;
        std::fs::write(&temporary, encoded).map_err(EvidenceError::CacheIo)?;
        std::fs::rename(&temporary, &destination).map_err(EvidenceError::CacheIo)
    }

    /// Load an explicit persistent cache. Corrupted, oversized, unvalidated,
    /// or schema-incompatible records are rejected rather than silently
    /// reused. Absence means an empty cache, not a transport failure.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_cache(
        directory: &std::path::Path,
        max_entries: usize,
        max_bytes: usize,
    ) -> Result<Self, EvidenceError> {
        if directory.as_os_str().is_empty() {
            return Err(EvidenceError::InvalidCachePath);
        }
        let source = directory.join("validated-evidence-cache.json");
        if !source.exists() {
            return Ok(Self::new(max_entries, max_bytes));
        }
        let store = Self::read_bundle(&source, max_entries, max_bytes)?;
        if store
            .records()
            .any(|record| !record.validated_at_acquisition)
        {
            return Err(EvidenceError::UnvalidatedCacheEntry);
        }
        Ok(store)
    }
}

/// Bounded, secret-safe transport provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalTrace {
    pub kind: RetrievalKind,
    pub requested_uri: String,
    pub final_uri: Option<String>,
    pub method: RetrievalMethod,
    pub request_body_sha256: Option<String>,
    pub resolved_ips: Vec<String>,
    pub selected_ip: Option<String>,
    pub redirect_chain: Vec<String>,
    pub response_status: Option<u16>,
    pub content_type: Option<String>,
    pub response_sha256: Option<String>,
    pub response_bytes: usize,
    pub cache_hit: bool,
    pub tls_verified: Option<bool>,
    pub error: Option<String>,
}

/// Fetched bytes and their final trace.
#[derive(Debug, Clone)]
pub struct RetrievalResponse {
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
    /// SHA-256 cache identity of the complete final URI. The raw URI stays
    /// private to the transport layer; `final_uri` is redacted for reports.
    pub source_uri_sha256: Option<String>,
    pub final_uri: String,
    pub trace: RetrievalTrace,
}

/// A reusable, stateful bounded retrieval session.
#[derive(Debug, Clone)]
pub struct RetrievalSession {
    policy: RetrievalPolicy,
    cache: EvidenceStore,
    cancellation: CancelToken,
    #[cfg(not(target_arch = "wasm32"))]
    cache_directory: Option<std::path::PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    requests: usize,
    #[cfg(not(target_arch = "wasm32"))]
    aia_requests: usize,
    #[cfg(not(target_arch = "wasm32"))]
    ocsp_requests: usize,
    #[cfg(not(target_arch = "wasm32"))]
    crl_requests: usize,
    #[cfg(not(target_arch = "wasm32"))]
    response_bytes: usize,
    traces: Vec<RetrievalTrace>,
    #[cfg(not(target_arch = "wasm32"))]
    started: std::time::Instant,
}

impl RetrievalSession {
    pub fn new(policy: RetrievalPolicy) -> Result<Self, EvidenceError> {
        policy.validate()?;
        #[cfg(not(target_arch = "wasm32"))]
        let cache_directory = policy
            .cache_directory
            .as_ref()
            .map(std::path::PathBuf::from);
        #[cfg(not(target_arch = "wasm32"))]
        let cache = match &cache_directory {
            Some(directory) => EvidenceStore::read_cache(
                directory,
                policy.budget.max_cache_entries,
                policy.budget.max_cache_bytes,
            )?,
            None => EvidenceStore::new(
                policy.budget.max_cache_entries,
                policy.budget.max_cache_bytes,
            ),
        };
        #[cfg(target_arch = "wasm32")]
        let cache = EvidenceStore::new(
            policy.budget.max_cache_entries,
            policy.budget.max_cache_bytes,
        );
        Ok(Self {
            cache,
            policy,
            cancellation: CancelToken::none(),
            #[cfg(not(target_arch = "wasm32"))]
            cache_directory,
            #[cfg(not(target_arch = "wasm32"))]
            requests: 0,
            #[cfg(not(target_arch = "wasm32"))]
            aia_requests: 0,
            #[cfg(not(target_arch = "wasm32"))]
            ocsp_requests: 0,
            #[cfg(not(target_arch = "wasm32"))]
            crl_requests: 0,
            #[cfg(not(target_arch = "wasm32"))]
            response_bytes: 0,
            traces: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            started: std::time::Instant::now(),
        })
    }

    pub fn with_cache(mut self, cache: EvidenceStore) -> Self {
        self.cache = cache;
        self
    }

    /// Attach the caller's cooperative cancellation token. Requests are
    /// checked before cache use, DNS resolution, each redirect, and after an
    /// HTTP response returns. A blocking socket operation still remains bound
    /// by the policy deadline, so cancellation cannot create an unbounded
    /// wait.
    pub fn with_cancellation_token(mut self, cancellation: CancelToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn policy(&self) -> &RetrievalPolicy {
        &self.policy
    }

    pub fn traces(&self) -> &[RetrievalTrace] {
        &self.traces
    }

    pub fn into_traces(self) -> Vec<RetrievalTrace> {
        self.traces
    }

    pub fn cache(&self) -> &EvidenceStore {
        &self.cache
    }

    pub fn cache_validated(&mut self, record: EvidenceRecord) -> Result<(), EvidenceError> {
        if !record.validated_at_acquisition {
            return Err(EvidenceError::UnvalidatedCacheEntry);
        }
        self.cache.insert(record)?;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(directory) = &self.cache_directory {
            self.cache.write_cache_atomically(directory)?;
        }
        Ok(())
    }

    /// Fetch an AIA, OCSP, or CRL resource under the session policy. The
    /// caller remains responsible for cryptographic validation of the bytes.
    pub fn fetch(
        &mut self,
        kind: RetrievalKind,
        uri: &str,
        method: RetrievalMethod,
        body: Option<&[u8]>,
    ) -> Result<RetrievalResponse, EvidenceError> {
        let mut cache_trace = self.base_trace(kind, uri, method, body);
        if self.cancellation.is_cancelled() {
            return self.fail(cache_trace, EvidenceError::Cancelled);
        }
        if let Some(record) = self.cache.find_validated(
            kind.evidence_kind(),
            uri,
            cache_trace.request_body_sha256.as_deref(),
        ) {
            let bytes = match record.bytes() {
                Ok(bytes) => bytes,
                Err(error) => return self.fail(cache_trace, error),
            };
            if bytes.len() > self.policy.budget.max_response_bytes {
                return self.fail(cache_trace, EvidenceError::ResponseLimitExceeded);
            }
            cache_trace.final_uri = record.source_uri.clone();
            cache_trace.content_type = record.content_type.clone();
            cache_trace.response_sha256 = Some(record.sha256);
            cache_trace.response_bytes = bytes.len();
            cache_trace.cache_hit = true;
            self.traces.push(cache_trace.clone());
            return Ok(RetrievalResponse {
                bytes,
                content_type: cache_trace.content_type.clone(),
                source_uri_sha256: record.source_uri_sha256.clone(),
                final_uri: cache_trace
                    .final_uri
                    .clone()
                    .unwrap_or(cache_trace.requested_uri.clone()),
                trace: cache_trace,
            });
        }
        #[cfg(target_arch = "wasm32")]
        {
            return self.fail(cache_trace, EvidenceError::UnsupportedOnWasm);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.fetch_native(kind, uri, method, body)
        }
    }

    fn base_trace(
        &self,
        kind: RetrievalKind,
        uri: &str,
        method: RetrievalMethod,
        body: Option<&[u8]>,
    ) -> RetrievalTrace {
        RetrievalTrace {
            kind,
            requested_uri: redact_uri_text(uri),
            final_uri: None,
            method,
            request_body_sha256: body.map(sha256_hex),
            resolved_ips: Vec::new(),
            selected_ip: None,
            redirect_chain: Vec::new(),
            response_status: None,
            content_type: None,
            response_sha256: None,
            response_bytes: 0,
            cache_hit: false,
            tls_verified: None,
            error: None,
        }
    }

    fn fail<T>(
        &mut self,
        mut trace: RetrievalTrace,
        error: EvidenceError,
    ) -> Result<T, EvidenceError> {
        trace.error = Some(error.to_string());
        self.traces.push(trace);
        Err(error)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn reserve_request(&mut self, kind: RetrievalKind) -> Result<(), EvidenceError> {
        if self.started.elapsed()
            >= std::time::Duration::from_millis(self.policy.budget.max_total_time_ms)
        {
            return Err(EvidenceError::DeadlineExceeded);
        }
        if self.requests >= self.policy.budget.max_total_requests {
            return Err(EvidenceError::RequestLimitExceeded);
        }
        let used = match kind {
            RetrievalKind::AiaIssuer => &mut self.aia_requests,
            RetrievalKind::Ocsp => &mut self.ocsp_requests,
            RetrievalKind::Crl => &mut self.crl_requests,
        };
        let limit = match kind {
            RetrievalKind::AiaIssuer => self.policy.budget.max_aia_requests,
            RetrievalKind::Ocsp => self.policy.budget.max_ocsp_requests,
            RetrievalKind::Crl => self.policy.budget.max_crl_requests,
        };
        if *used >= limit {
            return Err(EvidenceError::RequestLimitExceeded);
        }
        *used += 1;
        self.requests += 1;
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn fetch_native(
        &mut self,
        kind: RetrievalKind,
        uri: &str,
        method: RetrievalMethod,
        body: Option<&[u8]>,
    ) -> Result<RetrievalResponse, EvidenceError> {
        use reqwest::blocking::Client;
        use reqwest::redirect::Policy;
        use std::io::Read;
        use std::net::IpAddr;
        use std::time::Duration;
        use url::Url;

        let mut trace = self.base_trace(kind, uri, method, body);
        if !self.policy.enabled {
            return self.fail(trace, EvidenceError::NetworkDisabled);
        }
        let mut current = match Url::parse(uri) {
            Ok(url) => url,
            Err(_) => return self.fail(trace, EvidenceError::InvalidUrl),
        };
        if current.username() != "" || current.password().is_some() {
            return self.fail(trace, EvidenceError::CredentialsInUrl);
        }
        trace.requested_uri = redact_url(&current);
        let request_body = body.unwrap_or_default().to_vec();
        let mut redirects = 0usize;

        loop {
            if self.cancellation.is_cancelled() {
                return self.fail(trace, EvidenceError::Cancelled);
            }
            let (host, _port, resolved) = match self.validate_endpoint(&current) {
                Ok(value) => value,
                Err(error) => return self.fail(trace, error),
            };
            trace.resolved_ips = resolved.iter().map(|addr| addr.ip().to_string()).collect();
            trace.selected_ip = resolved.first().map(|addr| addr.ip().to_string());
            if let Err(error) = self.reserve_request(kind) {
                return self.fail(trace, error);
            }

            let mut builder = Client::builder()
                .no_proxy()
                .redirect(Policy::none())
                .connect_timeout(Duration::from_millis(self.policy.budget.connect_timeout_ms))
                .timeout(self.remaining_request_timeout()?)
                .user_agent("wellfriendpdf-signature-validation/1");
            if host.parse::<IpAddr>().is_err() {
                builder = builder.resolve_to_addrs(&host, &resolved);
            }
            let client = match builder.build() {
                Ok(client) => client,
                Err(error) => return self.fail(trace, EvidenceError::Http(error.to_string())),
            };
            let request = match method {
                RetrievalMethod::Get => client.get(current.clone()),
                RetrievalMethod::Post => client
                    .post(current.clone())
                    .header("content-type", "application/ocsp-request")
                    .body(request_body.clone()),
            }
            .header("accept", accepted_mime_types(kind));
            let mut response = match request.send() {
                Ok(response) => response,
                Err(error) => return self.fail(trace, EvidenceError::Http(error.to_string())),
            };
            if self.cancellation.is_cancelled() {
                return self.fail(trace, EvidenceError::Cancelled);
            }
            trace.response_status = Some(response.status().as_u16());
            trace.tls_verified = (current.scheme() == "https").then_some(true);

            if response.status().is_redirection() {
                if redirects >= self.policy.budget.max_redirects {
                    return self.fail(trace, EvidenceError::RedirectLimitExceeded);
                }
                let Some(location) = response.headers().get(reqwest::header::LOCATION) else {
                    return self.fail(trace, EvidenceError::RedirectWithoutLocation);
                };
                let Ok(location) = location.to_str() else {
                    return self.fail(trace, EvidenceError::RedirectWithoutLocation);
                };
                let next = match current.join(location) {
                    Ok(next) => next,
                    Err(_) => return self.fail(trace, EvidenceError::InvalidUrl),
                };
                if next.username() != "" || next.password().is_some() {
                    return self.fail(trace, EvidenceError::CredentialsInUrl);
                }
                if !self.policy.allow_cross_origin_redirects && !same_origin(&current, &next) {
                    return self.fail(trace, EvidenceError::CrossOriginRedirect);
                }
                if kind == RetrievalKind::Ocsp
                    && method == RetrievalMethod::Post
                    && !same_origin(&current, &next)
                {
                    return self.fail(trace, EvidenceError::CrossOriginRedirect);
                }
                trace.redirect_chain.push(redact_url(&next));
                current = next;
                redirects += 1;
                continue;
            }

            if !response.status().is_success() {
                return self.fail(trace, EvidenceError::HttpStatus(response.status().as_u16()));
            }
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            if !content_type_is_permitted(kind, content_type.as_deref()) {
                return self.fail(trace, EvidenceError::UnexpectedContentType);
            }
            let content_encoding = response
                .headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .and_then(|value| value.to_str().ok())
                .map(str::trim);
            if content_encoding.is_some_and(|encoding| !encoding.eq_ignore_ascii_case("identity")) {
                return self.fail(trace, EvidenceError::CompressedResponseForbidden);
            }
            trace.content_type = content_type.clone();
            let mut bytes = Vec::new();
            let max = self.policy.budget.max_response_bytes;
            let mut limited = response.by_ref().take((max as u64).saturating_add(1));
            if let Err(error) = limited.read_to_end(&mut bytes) {
                return self.fail(trace, EvidenceError::Http(error.to_string()));
            }
            if bytes.len() > max {
                return self.fail(trace, EvidenceError::ResponseLimitExceeded);
            }
            self.response_bytes = match self.response_bytes.checked_add(bytes.len()) {
                Some(value) => value,
                None => return self.fail(trace, EvidenceError::ResponseLimitExceeded),
            };
            if self.response_bytes > self.policy.budget.max_total_response_bytes {
                return self.fail(trace, EvidenceError::ResponseLimitExceeded);
            }
            trace.response_bytes = bytes.len();
            trace.response_sha256 = Some(sha256_hex(&bytes));
            trace.final_uri = Some(redact_url(&current));
            self.traces.push(trace.clone());
            return Ok(RetrievalResponse {
                bytes,
                content_type,
                source_uri_sha256: Some(sha256_hex(current.as_str().as_bytes())),
                final_uri: redact_url(&current),
                trace,
            });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn remaining_request_timeout(&self) -> Result<std::time::Duration, EvidenceError> {
        let limit = std::time::Duration::from_millis(self.policy.budget.max_total_time_ms);
        let elapsed = self.started.elapsed();
        let remaining = limit
            .checked_sub(elapsed)
            .ok_or(EvidenceError::DeadlineExceeded)?;
        Ok(std::time::Duration::from_millis(self.policy.budget.total_timeout_ms).min(remaining))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn validate_endpoint(
        &self,
        url: &url::Url,
    ) -> Result<(String, u16, Vec<std::net::SocketAddr>), EvidenceError> {
        use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

        let (host, port) = validate_url_components(&self.policy, url)?;
        let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
            vec![SocketAddr::new(ip, port)]
        } else {
            (host.as_str(), port)
                .to_socket_addrs()
                .map_err(|error| EvidenceError::Dns(error.to_string()))?
                .collect::<Vec<_>>()
        };
        if addresses.is_empty() {
            return Err(EvidenceError::Dns("no addresses returned".to_string()));
        }
        if !self.policy.allow_private_network
            && addresses.iter().any(|address| forbidden_ip(address.ip()))
        {
            return Err(EvidenceError::ForbiddenAddress);
        }
        Ok((host, port, addresses))
    }
}

/// Apply the deterministic, non-network portion of retrieval URI policy.
///
/// Callers can use this to reject malformed schemes, URL credentials, local
/// aliases, host allow/deny rules, and forbidden ports before any resolver or
/// HTTP implementation is involved. DNS/IP validation remains in the native
/// transport immediately before a connection is made.
#[cfg_attr(not(any(test, feature = "fuzzing")), allow(dead_code))]
pub(crate) fn validate_retrieval_uri_syntax(
    policy: &RetrievalPolicy,
    uri: &str,
) -> Result<(), EvidenceError> {
    policy.validate()?;
    let url = url::Url::parse(uri).map_err(|_| EvidenceError::InvalidUrl)?;
    validate_url_components(policy, &url).map(|_| ())
}

fn validate_url_components(
    policy: &RetrievalPolicy,
    url: &url::Url,
) -> Result<(String, u16), EvidenceError> {
    let scheme_allowed = match url.scheme() {
        "http" => policy.allow_http,
        "https" => policy.allow_https,
        _ => return Err(EvidenceError::UnsupportedScheme),
    };
    if !scheme_allowed {
        return Err(EvidenceError::SchemeForbidden);
    }
    if url.username() != "" || url.password().is_some() {
        return Err(EvidenceError::CredentialsInUrl);
    }
    let host = url
        .host_str()
        .ok_or(EvidenceError::InvalidUrl)?
        .to_ascii_lowercase();
    // `localhost` and every delegated `.localhost` name are reserved local
    // names. Reject them before resolver behavior can vary by host platform.
    if !policy.allow_private_network && (host == "localhost" || host.ends_with(".localhost")) {
        return Err(EvidenceError::ForbiddenAddress);
    }
    if host_is_denied(&host, &policy.denied_hosts) {
        return Err(EvidenceError::HostDenied);
    }
    if !policy.allowed_hosts.is_empty() && !host_is_allowed(&host, &policy.allowed_hosts) {
        return Err(EvidenceError::HostNotAllowlisted);
    }
    let port = url
        .port_or_known_default()
        .ok_or(EvidenceError::InvalidUrl)?;
    let is_default_port =
        (url.scheme() == "http" && port == 80) || (url.scheme() == "https" && port == 443);
    if !is_default_port && !policy.allow_non_default_ports && !policy.allowed_ports.contains(&port)
    {
        return Err(EvidenceError::PortForbidden);
    }
    Ok((host, port))
}

/// Retrieval/evidence errors are intentionally distinct so policy callers never
/// collapse unavailable or rejected evidence into a good revocation result.
#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("signature validation retrieval was cancelled")]
    Cancelled,
    #[error("network retrieval is disabled by policy")]
    NetworkDisabled,
    #[error("network retrieval is unsupported on this target")]
    UnsupportedOnWasm,
    #[error("invalid retrieval URL")]
    InvalidUrl,
    #[error("retrieval URL contains credentials")]
    CredentialsInUrl,
    #[error("retrieval scheme is unsupported")]
    UnsupportedScheme,
    #[error("retrieval scheme is forbidden by policy")]
    SchemeForbidden,
    #[error("retrieval host is denied by policy")]
    HostDenied,
    #[error("retrieval host is not allowlisted")]
    HostNotAllowlisted,
    #[error("retrieval port is forbidden by policy")]
    PortForbidden,
    #[error("retrieval destination resolves to a forbidden address")]
    ForbiddenAddress,
    #[error("DNS resolution failed: {0}")]
    Dns(String),
    #[error("request limit exceeded")]
    RequestLimitExceeded,
    #[error("network retrieval session deadline exceeded")]
    DeadlineExceeded,
    #[error("redirect limit exceeded")]
    RedirectLimitExceeded,
    #[error("redirect did not provide a valid Location")]
    RedirectWithoutLocation,
    #[error("cross-origin redirect is forbidden")]
    CrossOriginRedirect,
    #[error("HTTP transport failed: {0}")]
    Http(String),
    #[error("HTTP response status {0}")]
    HttpStatus(u16),
    #[error("response content type is not permitted for this evidence kind")]
    UnexpectedContentType,
    #[error("compressed network evidence is forbidden")]
    CompressedResponseForbidden,
    #[error("response exceeds the configured byte budget")]
    ResponseLimitExceeded,
    #[error("network budget is invalid")]
    InvalidBudget,
    #[error("no network schemes are allowed")]
    NoAllowedSchemes,
    #[error("evidence bytes are not valid hexadecimal")]
    InvalidEvidenceHex,
    #[error("evidence content hash does not match the declared SHA-256")]
    EvidenceHashMismatch,
    #[error("evidence bundle schema {0} is unsupported")]
    UnsupportedBundleSchema(u32),
    #[error("evidence bundle contains duplicate evidence identities")]
    DuplicateEvidence,
    #[error("evidence count or byte budget exceeded")]
    EvidenceLimitExceeded,
    #[error("unvalidated evidence cannot be inserted into the retrieval cache")]
    UnvalidatedCacheEntry,
    #[error("cache path is invalid")]
    InvalidCachePath,
    #[error("cache destination already exists")]
    CacheAlreadyExists,
    #[error("cache I/O failed: {0}")]
    CacheIo(std::io::Error),
    #[error("evidence bundle JSON failed: {0}")]
    BundleJson(serde_json::Error),
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn hex_decode(text: &str) -> Result<Vec<u8>, ()> {
    if !text.len().is_multiple_of(2) {
        return Err(());
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    let raw = text.as_bytes();
    for pair in raw.chunks_exact(2) {
        let high = hex_value(pair[0]).ok_or(())?;
        let low = hex_value(pair[1]).ok_or(())?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn redact_uri_text(uri: &str) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    {
        url::Url::parse(uri)
            .map(|parsed| redact_url(&parsed))
            .unwrap_or_else(|_| "<invalid-url>".to_string())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = uri;
        "<redacted-url>".to_string()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn redact_url(url: &url::Url) -> String {
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.set_query(None);
    redacted.set_fragment(None);
    redacted.to_string()
}

#[cfg(not(target_arch = "wasm32"))]
fn same_origin(left: &url::Url, right: &url::Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str().map(str::to_ascii_lowercase)
            == right.host_str().map(str::to_ascii_lowercase)
        && left.port_or_known_default() == right.port_or_known_default()
}

fn host_is_allowed(host: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|entry| {
        let entry = entry.trim().trim_start_matches('.').to_ascii_lowercase();
        host == entry || host.strip_suffix(&format!(".{entry}")).is_some()
    })
}

fn host_is_denied(host: &str, denied: &[String]) -> bool {
    denied.iter().any(|entry| {
        let entry = entry.trim().trim_start_matches('.').to_ascii_lowercase();
        host == entry || host.strip_suffix(&format!(".{entry}")).is_some()
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn forbidden_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(ip) => forbidden_ipv4(ip),
        IpAddr::V6(ip) => {
            if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
                return true;
            }
            let first = ip.segments()[0];
            if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 {
                return true;
            }
            ipv4_mapped(ip).map(forbidden_ipv4).unwrap_or(false)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn forbidden_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 240
}

#[cfg(not(target_arch = "wasm32"))]
fn ipv4_mapped(ip: std::net::Ipv6Addr) -> Option<std::net::Ipv4Addr> {
    let octets = ip.octets();
    if octets[..10] == [0; 10]
        && (octets[10] == 0 && octets[11] == 0 || octets[10] == 0xff && octets[11] == 0xff)
    {
        Some(std::net::Ipv4Addr::new(
            octets[12], octets[13], octets[14], octets[15],
        ))
    } else {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn accepted_mime_types(kind: RetrievalKind) -> &'static str {
    match kind {
        RetrievalKind::AiaIssuer => {
            "application/pkix-cert, application/pkcs7-mime, application/octet-stream"
        }
        RetrievalKind::Ocsp => "application/ocsp-response, application/octet-stream",
        RetrievalKind::Crl => {
            "application/pkix-crl, application/pkcs7-mime, application/octet-stream"
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn content_type_is_permitted(kind: RetrievalKind, content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return true;
    };
    let normalized = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if normalized == "text/html" || normalized == "application/xhtml+xml" {
        return false;
    }
    match kind {
        RetrievalKind::AiaIssuer => matches!(
            normalized.as_str(),
            "application/pkix-cert"
                | "application/x-x509-ca-cert"
                | "application/pkcs7-mime"
                | "application/octet-stream"
        ),
        RetrievalKind::Ocsp => matches!(
            normalized.as_str(),
            "application/ocsp-response" | "application/octet-stream"
        ),
        RetrievalKind::Crl => matches!(
            normalized.as_str(),
            "application/pkix-crl"
                | "application/x-pkcs7-crl"
                | "application/pkcs7-mime"
                | "application/octet-stream"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_rejects_corrupt_hash_and_duplicate_identity() {
        let mut bad =
            EvidenceRecord::from_bytes(EvidenceKind::Ocsp, b"ocsp", None, None, None, None, false);
        bad.sha256 = "00".repeat(32);
        let bundle = EvidenceBundle {
            records: vec![bad],
            ..EvidenceBundle::default()
        };
        assert!(matches!(
            bundle.validate(4, 1024),
            Err(EvidenceError::EvidenceHashMismatch)
        ));

        let record =
            EvidenceRecord::from_bytes(EvidenceKind::Crl, b"crl", None, None, None, None, false);
        let bundle = EvidenceBundle {
            records: vec![record.clone(), record],
            ..EvidenceBundle::default()
        };
        assert!(matches!(
            bundle.validate(4, 1024),
            Err(EvidenceError::DuplicateEvidence)
        ));
    }

    #[test]
    fn validated_cache_replays_without_network() {
        let uri = "https://evidence.example.test/issuer";
        let record = EvidenceRecord::from_bytes(
            EvidenceKind::Certificate,
            b"certificate",
            Some(uri.to_string()),
            Some(1),
            Some("application/pkix-cert".to_string()),
            None,
            true,
        );
        let mut store = EvidenceStore::new(4, 1024);
        store.insert(record).unwrap();
        let policy = RetrievalPolicy::offline();
        let mut session = RetrievalSession::new(policy).unwrap().with_cache(store);
        let result = session
            .fetch(RetrievalKind::AiaIssuer, uri, RetrievalMethod::Get, None)
            .unwrap();
        assert_eq!(result.bytes, b"certificate");
        assert!(result.trace.cache_hit);
        assert_eq!(session.traces().len(), 1);
    }

    #[test]
    fn pre_cancelled_retrieval_does_not_use_cache_or_network() {
        let cancellation = CancelToken::new();
        cancellation.cancel();
        let mut session = RetrievalSession::new(RetrievalPolicy::offline())
            .unwrap()
            .with_cancellation_token(cancellation);
        assert!(matches!(
            session.fetch(
                RetrievalKind::AiaIssuer,
                "https://evidence.example.test/issuer",
                RetrievalMethod::Get,
                None,
            ),
            Err(EvidenceError::Cancelled)
        ));
        assert_eq!(session.traces().len(), 1);
        assert_eq!(
            session.traces()[0].error.as_deref(),
            Some("signature validation retrieval was cancelled")
        );
    }

    #[test]
    fn ocsp_cache_is_bound_to_the_request_body() {
        let uri = "https://ocsp.example.test/status";
        let request_one = b"certid-one";
        let record = EvidenceRecord::from_bytes(
            EvidenceKind::Ocsp,
            b"validated-ocsp-response",
            Some(uri.to_string()),
            Some(1),
            Some("application/ocsp-response".to_string()),
            Some(sha256_hex(request_one)),
            true,
        );
        let mut store = EvidenceStore::new(4, 1024);
        store.insert(record).unwrap();
        let mut session = RetrievalSession::new(RetrievalPolicy::offline())
            .unwrap()
            .with_cache(store);

        let hit = session
            .fetch(
                RetrievalKind::Ocsp,
                uri,
                RetrievalMethod::Post,
                Some(request_one),
            )
            .unwrap();
        assert!(hit.trace.cache_hit);
        assert!(matches!(
            session.fetch(
                RetrievalKind::Ocsp,
                uri,
                RetrievalMethod::Post,
                Some(b"certid-two"),
            ),
            Err(EvidenceError::NetworkDisabled)
        ));
    }

    #[test]
    fn cache_does_not_alias_query_redacted_uris() {
        let first = "https://evidence.example.test/status?partition=one";
        let second = "https://evidence.example.test/status?partition=two";
        let record = EvidenceRecord::from_bytes(
            EvidenceKind::Crl,
            b"validated-crl",
            Some(first.to_string()),
            Some(1),
            Some("application/pkix-crl".to_string()),
            None,
            true,
        );
        assert_eq!(
            record.source_uri.as_deref(),
            Some("https://evidence.example.test/status")
        );
        assert!(record.source_uri_sha256.is_some());

        let mut store = EvidenceStore::new(4, 1024);
        store.insert(record).unwrap();
        let mut session = RetrievalSession::new(RetrievalPolicy::offline())
            .unwrap()
            .with_cache(store);
        let first_result = session
            .fetch(RetrievalKind::Crl, first, RetrievalMethod::Get, None)
            .unwrap();
        assert!(first_result.trace.cache_hit);
        assert!(matches!(
            session.fetch(RetrievalKind::Crl, second, RetrievalMethod::Get, None),
            Err(EvidenceError::NetworkDisabled)
        ));
    }

    #[test]
    fn unvalidated_cache_entry_is_not_accepted() {
        let mut session = RetrievalSession::new(RetrievalPolicy::offline()).unwrap();
        let record =
            EvidenceRecord::from_bytes(EvidenceKind::Crl, b"crl", None, None, None, None, false);
        assert!(matches!(
            session.cache_validated(record),
            Err(EvidenceError::UnvalidatedCacheEntry)
        ));
    }

    #[test]
    fn cached_response_still_obeys_the_active_response_limit() {
        let uri = "https://evidence.example.test/crl";
        let record = EvidenceRecord::from_bytes(
            EvidenceKind::Crl,
            b"oversized",
            Some(uri.to_string()),
            Some(1),
            Some("application/pkix-crl".to_string()),
            None,
            true,
        );
        let mut store = EvidenceStore::new(4, 1024);
        store.insert(record).unwrap();
        let mut policy = RetrievalPolicy::offline();
        policy.budget.max_response_bytes = 4;
        let mut session = RetrievalSession::new(policy).unwrap().with_cache(store);
        assert!(matches!(
            session.fetch(RetrievalKind::Crl, uri, RetrievalMethod::Get, None),
            Err(EvidenceError::ResponseLimitExceeded)
        ));
    }

    #[test]
    fn imported_provenance_is_redacted_before_it_can_reach_reports() {
        let mut record = EvidenceRecord::from_bytes(
            EvidenceKind::Certificate,
            b"certificate",
            None,
            Some(1),
            None,
            None,
            true,
        );
        record.source_uri =
            Some("https://user:secret@example.test/issuer?token=hidden#fragment".to_string());
        let mut store = EvidenceStore::new(4, 1024);
        store.insert(record).unwrap();
        let provenance = store
            .records()
            .next()
            .and_then(|item| item.source_uri.as_deref())
            .expect("source provenance retained");
        assert_eq!(provenance, "https://example.test/issuer");
        assert!(!provenance.contains("secret"));
        assert!(!provenance.contains("token"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn private_and_documentation_addresses_are_rejected() {
        use std::net::{IpAddr, Ipv4Addr};
        assert!(forbidden_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(forbidden_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))));
        assert!(forbidden_ip(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))));
        assert!(!forbidden_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn url_policy_rejects_credentials_and_private_destinations_before_connecting() {
        let mut policy = RetrievalPolicy::online();
        policy.allowed_hosts.push("example.test".to_string());
        let mut session = RetrievalSession::new(policy).unwrap();
        assert!(matches!(
            session.fetch(
                RetrievalKind::Crl,
                "http://user:secret@example.test/list",
                RetrievalMethod::Get,
                None
            ),
            Err(EvidenceError::CredentialsInUrl)
        ));
        let mut session = RetrievalSession::new(RetrievalPolicy::online()).unwrap();
        assert!(matches!(
            session.fetch(
                RetrievalKind::Crl,
                "http://127.0.0.1/list",
                RetrievalMethod::Get,
                None
            ),
            Err(EvidenceError::ForbiddenAddress)
        ));
    }

    #[test]
    fn uri_syntax_policy_rejects_local_aliases_and_unsupported_schemes_without_dns() {
        let policy = RetrievalPolicy::online();
        assert!(matches!(
            validate_retrieval_uri_syntax(&policy, "http://localhost/issuer"),
            Err(EvidenceError::ForbiddenAddress)
        ));
        assert!(matches!(
            validate_retrieval_uri_syntax(&policy, "https://ocsp.localhost/status"),
            Err(EvidenceError::ForbiddenAddress)
        ));
        assert!(matches!(
            validate_retrieval_uri_syntax(&policy, "file:///C:/private.crl"),
            Err(EvidenceError::UnsupportedScheme)
        ));
        assert!(matches!(
            validate_retrieval_uri_syntax(&policy, "https://user:secret@example.test/ocsp"),
            Err(EvidenceError::CredentialsInUrl)
        ));

        let mut private_opt_in = policy;
        private_opt_in.allow_private_network = true;
        assert!(validate_retrieval_uri_syntax(&private_opt_in, "http://localhost/issuer").is_ok());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn bounded_local_transport_fetches_only_with_explicit_private_network_opt_in() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let read = stream.read(&mut request).unwrap();
            assert!(std::str::from_utf8(&request[..read])
                .unwrap()
                .starts_with("GET /issuer HTTP/1.1"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/pkix-cert\r\nContent-Length: 4\r\nConnection: close\r\n\r\nCERT",
                )
                .unwrap();
        });

        let mut policy = RetrievalPolicy::online();
        policy.allow_private_network = true;
        policy.allow_non_default_ports = true;
        policy.allowed_ports.push(port);
        let mut session = RetrievalSession::new(policy).unwrap();
        let result = session
            .fetch(
                RetrievalKind::AiaIssuer,
                &format!("http://127.0.0.1:{port}/issuer"),
                RetrievalMethod::Get,
                None,
            )
            .unwrap();
        assert_eq!(result.bytes, b"CERT");
        assert_eq!(result.trace.response_status, Some(200));
        assert_eq!(result.trace.selected_ip.as_deref(), Some("127.0.0.1"));
        server.join().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn cross_origin_redirect_is_rejected_before_following() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let header = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://localhost:{port}/final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(header.as_bytes()).unwrap();
        });

        let mut policy = RetrievalPolicy::online();
        policy.allow_private_network = true;
        policy.allow_non_default_ports = true;
        policy.allowed_ports.push(port);
        let mut session = RetrievalSession::new(policy).unwrap();
        assert!(matches!(
            session.fetch(
                RetrievalKind::AiaIssuer,
                &format!("http://127.0.0.1:{port}/issuer"),
                RetrievalMethod::Get,
                None,
            ),
            Err(EvidenceError::CrossOriginRedirect)
        ));
        server.join().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn network_response_size_limit_is_enforced() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/pkix-crl\r\nContent-Length: 8\r\nConnection: close\r\n\r\nTOOLARGE",
                )
                .unwrap();
        });

        let mut policy = RetrievalPolicy::online();
        policy.allow_private_network = true;
        policy.allow_non_default_ports = true;
        policy.allowed_ports.push(port);
        policy.budget.max_response_bytes = 4;
        let mut session = RetrievalSession::new(policy).unwrap();
        assert!(matches!(
            session.fetch(
                RetrievalKind::Crl,
                &format!("http://127.0.0.1:{port}/leaf.crl"),
                RetrievalMethod::Get,
                None,
            ),
            Err(EvidenceError::ResponseLimitExceeded)
        ));
        server.join().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn html_or_login_page_content_type_is_rejected() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 13\r\nConnection: close\r\n\r\n<html></html>",
                )
                .unwrap();
        });

        let mut policy = RetrievalPolicy::online();
        policy.allow_private_network = true;
        policy.allow_non_default_ports = true;
        policy.allowed_ports.push(port);
        let mut session = RetrievalSession::new(policy).unwrap();
        assert!(matches!(
            session.fetch(
                RetrievalKind::AiaIssuer,
                &format!("http://127.0.0.1:{port}/issuer"),
                RetrievalMethod::Get,
                None,
            ),
            Err(EvidenceError::UnexpectedContentType)
        ));
        server.join().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn compressed_network_evidence_is_rejected_without_decompression() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/ocsp-response\r\nContent-Encoding: gzip\r\nContent-Length: 4\r\nConnection: close\r\n\r\nBOMB",
                )
                .unwrap();
        });

        let mut policy = RetrievalPolicy::online();
        policy.allow_private_network = true;
        policy.allow_non_default_ports = true;
        policy.allowed_ports.push(port);
        let mut session = RetrievalSession::new(policy).unwrap();
        assert!(matches!(
            session.fetch(
                RetrievalKind::Ocsp,
                &format!("http://127.0.0.1:{port}/ocsp"),
                RetrievalMethod::Post,
                Some(b"request"),
            ),
            Err(EvidenceError::CompressedResponseForbidden)
        ));
        server.join().unwrap();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn persistent_cache_reloads_validated_records_after_replacement() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock after Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir()
            .join("prompt24-evidence-cache")
            .join(format!("{}-{suffix}", std::process::id()));
        let mut policy = RetrievalPolicy::offline();
        policy.cache_directory = Some(directory.to_string_lossy().into_owned());

        let mut session = RetrievalSession::new(policy.clone()).unwrap();
        session
            .cache_validated(EvidenceRecord::from_bytes(
                EvidenceKind::Ocsp,
                b"validated-ocsp-one",
                Some("https://ocsp.example.test/status".to_string()),
                Some(1),
                Some("application/ocsp-response".to_string()),
                Some(sha256_hex(b"ocsp-request-one")),
                true,
            ))
            .unwrap();
        // A second write exercises replacement of an existing cache file,
        // which is the path used when fresher evidence arrives.
        session
            .cache_validated(EvidenceRecord::from_bytes(
                EvidenceKind::Crl,
                b"validated-crl-two",
                Some("https://crl.example.test/list".to_string()),
                Some(2),
                Some("application/pkix-crl".to_string()),
                None,
                true,
            ))
            .unwrap();

        let reloaded = RetrievalSession::new(policy).unwrap();
        assert_eq!(reloaded.cache().records().count(), 2);
        assert!(reloaded
            .cache()
            .records()
            .all(|record| record.validated_at_acquisition));
        assert!(directory.join("validated-evidence-cache.json").is_file());
    }
}
