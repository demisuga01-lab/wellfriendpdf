use std::path::PathBuf;
use std::sync::Arc;

use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBytes, PyDict, PyList, PyModule, PyType};
use serde::Serialize;
use serde_json::json;
use wellfriendpdf_engine::{
    sdk, CancelToken, ContentEngine, DocType, DocumentInfo, EvidenceBundle, ExtractOptions,
    ExtractionProfile, ImageLocateOptions, ImageOutputFormat, IntermediateStore, NetworkBudget,
    OcrPolicy, PageRegion, ParseOptions, RetrievalPolicy, SerializeOptions,
    SignatureRevocationMode, TrustStore, VerifyOptions,
};

mod ocr_backend;
use ocr_backend::PyOcrEngine;

create_exception!(wellfriendpdf, WellfriendError, PyException);

#[pyclass(name = "Document", module = "wellfriendpdf", unsendable)]
struct PyDocument {
    engine: Arc<ContentEngine>,
}

#[pyclass(name = "Page", module = "wellfriendpdf", unsendable)]
struct PyPage {
    engine: Arc<ContentEngine>,
    number: usize,
}

#[pyclass(name = "RegionPage", module = "wellfriendpdf", unsendable)]
struct PyRegionPage {
    engine: Arc<ContentEngine>,
    number: usize,
    region: PageRegion,
}

#[pyclass(name = "_PageIterator", module = "wellfriendpdf", unsendable)]
struct PyPageIterator {
    engine: Arc<ContentEngine>,
    next: usize,
    total: usize,
}

#[pyclass(name = "SignatureTrustStore", module = "wellfriendpdf", unsendable)]
struct PySignatureTrustStore {
    store: TrustStore,
    distrusted_certificate_sha256: Vec<String>,
}

#[pyclass(
    name = "SignatureIntermediateStore",
    module = "wellfriendpdf",
    unsendable
)]
struct PySignatureIntermediateStore {
    store: IntermediateStore,
}

#[pyclass(name = "SignatureEvidenceStore", module = "wellfriendpdf", unsendable)]
struct PySignatureEvidenceStore {
    ocsp_responses_der: Vec<Vec<u8>>,
    crls_der: Vec<Vec<u8>>,
    bundle: Option<EvidenceBundle>,
}

#[pyclass(
    name = "SignatureRetrievalPolicy",
    module = "wellfriendpdf",
    unsendable
)]
struct PySignatureRetrievalPolicy {
    policy: RetrievalPolicy,
}

#[pyclass(
    name = "SignatureValidationCancellation",
    module = "wellfriendpdf",
    unsendable
)]
struct PySignatureValidationCancellation {
    token: CancelToken,
}

#[pymethods]
impl PySignatureTrustStore {
    #[new]
    fn new() -> Self {
        Self {
            store: TrustStore::new(),
            distrusted_certificate_sha256: Vec::new(),
        }
    }

    #[pyo3(signature = (der, origin="python", purpose=None))]
    fn add_anchor_der(
        &mut self,
        der: Vec<u8>,
        origin: &str,
        purpose: Option<String>,
    ) -> PyResult<()> {
        self.store
            .add_der(&der, origin.to_string(), purpose)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    #[pyo3(signature = (pem, origin="python", purpose=None))]
    fn add_anchor_pem(&mut self, pem: &str, origin: &str, purpose: Option<String>) -> PyResult<()> {
        self.store
            .add_pem(pem.as_bytes(), origin.to_string(), purpose)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    #[pyo3(signature = (path, origin=None, purpose=None))]
    fn add_anchor_file(
        &mut self,
        path: PathBuf,
        origin: Option<String>,
        purpose: Option<String>,
    ) -> PyResult<()> {
        let bytes = read_signature_component_file(&path)?;
        let origin = origin.unwrap_or_else(|| "python:file".to_string());
        if looks_like_pem(&bytes) {
            self.store
                .add_pem(&bytes, origin, purpose)
                .map_err(|error| PyValueError::new_err(error.to_string()))
        } else {
            self.store
                .add_der(&bytes, origin, purpose)
                .map_err(|error| PyValueError::new_err(error.to_string()))
        }
    }

    fn add_distrusted_certificate_sha256(&mut self, fingerprint: &str) -> PyResult<()> {
        let normalized = VerifyOptions::default()
            .with_distrusted_certificate_sha256(fingerprint)
            .map_err(|error| PyValueError::new_err(error.to_string()))?
            .distrusted_certificate_sha256
            .into_iter()
            .next()
            .ok_or_else(|| PyValueError::new_err("empty certificate fingerprint"))?;
        if !self
            .distrusted_certificate_sha256
            .iter()
            .any(|existing| existing == &normalized)
        {
            self.distrusted_certificate_sha256.push(normalized);
            self.distrusted_certificate_sha256.sort();
        }
        Ok(())
    }

    fn len(&self) -> usize {
        self.store.anchors().len()
    }

    fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

#[pymethods]
impl PySignatureIntermediateStore {
    #[new]
    fn new() -> Self {
        Self {
            store: IntermediateStore::new(),
        }
    }

    fn add_der(&mut self, der: Vec<u8>) -> PyResult<()> {
        self.store
            .add_der(&der)
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn add_pem(&mut self, pem: &str) -> PyResult<()> {
        self.store
            .add_pem(pem.as_bytes())
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }

    fn add_file(&mut self, path: PathBuf) -> PyResult<()> {
        let bytes = read_signature_component_file(&path)?;
        if looks_like_pem(&bytes) {
            self.store
                .add_pem(&bytes)
                .map_err(|error| PyValueError::new_err(error.to_string()))
        } else {
            self.store
                .add_der(&bytes)
                .map_err(|error| PyValueError::new_err(error.to_string()))
        }
    }

    fn len(&self) -> usize {
        self.store.certificates_der().len()
    }

    fn is_empty(&self) -> bool {
        self.store.certificates_der().is_empty()
    }
}

#[pymethods]
impl PySignatureEvidenceStore {
    #[new]
    fn new() -> Self {
        Self {
            ocsp_responses_der: Vec::new(),
            crls_der: Vec::new(),
            bundle: None,
        }
    }

    fn add_ocsp_response_der(&mut self, der: Vec<u8>) {
        self.ocsp_responses_der.push(der);
    }

    fn add_ocsp_response_file(&mut self, path: PathBuf) -> PyResult<()> {
        let bytes = read_signature_component_file(&path)?;
        self.ocsp_responses_der.push(bytes);
        Ok(())
    }

    fn add_crl_der(&mut self, der: Vec<u8>) {
        self.crls_der.push(der);
    }

    fn add_crl_file(&mut self, path: PathBuf) -> PyResult<()> {
        let bytes = read_signature_component_file(&path)?;
        self.crls_der.push(bytes);
        Ok(())
    }

    fn import_bundle_json(&mut self, bundle_json: &str) -> PyResult<()> {
        let bundle: EvidenceBundle = serde_json::from_str(bundle_json)
            .map_err(|error| PyValueError::new_err(format!("evidence bundle JSON: {error}")))?;
        let budget = NetworkBudget::default();
        bundle
            .validate(budget.max_cache_entries, budget.max_cache_bytes)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        self.bundle = Some(bundle);
        Ok(())
    }

    fn set_bundle_json(&mut self, bundle_json: &str) -> PyResult<()> {
        self.import_bundle_json(bundle_json)
    }

    fn bundle_json(&self) -> PyResult<Option<String>> {
        self.bundle
            .as_ref()
            .map(|bundle| {
                serde_json::to_string(bundle).map_err(|error| {
                    WellfriendError::new_err(format!("JSON serialization error: {error}"))
                })
            })
            .transpose()
    }

    fn ocsp_count(&self) -> usize {
        self.ocsp_responses_der.len()
    }

    fn crl_count(&self) -> usize {
        self.crls_der.len()
    }
}

#[pymethods]
impl PySignatureRetrievalPolicy {
    #[new]
    fn new() -> Self {
        Self {
            policy: RetrievalPolicy::offline(),
        }
    }

    #[staticmethod]
    fn offline() -> Self {
        Self {
            policy: RetrievalPolicy::offline(),
        }
    }

    #[staticmethod]
    fn online() -> Self {
        Self {
            policy: RetrievalPolicy::online(),
        }
    }

    fn set_json(&mut self, policy_json: &str) -> PyResult<()> {
        let policy: RetrievalPolicy = serde_json::from_str(policy_json)
            .map_err(|error| PyValueError::new_err(format!("retrieval policy JSON: {error}")))?;
        policy
            .validate()
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        self.policy = policy;
        Ok(())
    }

    fn to_json(&self) -> PyResult<String> {
        serde_json::to_string(&self.policy)
            .map_err(|error| WellfriendError::new_err(format!("JSON serialization error: {error}")))
    }
}

#[pymethods]
impl PySignatureValidationCancellation {
    #[new]
    fn new() -> Self {
        Self {
            token: CancelToken::new(),
        }
    }

    fn cancel(&self) {
        self.token.cancel();
    }

    fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

/// Owned Prompt 24 signature-validation configuration.
///
/// The object owns every byte supplied by Python.  Certificates remain either
/// explicit trust anchors or untrusted intermediates according to the method
/// used to add them; evidence is never promoted to trust merely by being
/// loaded here.  Network retrieval remains disabled unless an explicit,
/// validated retrieval policy enables it.
#[pyclass(
    name = "SignatureValidationOptions",
    module = "wellfriendpdf",
    unsendable
)]
struct PySignatureValidationOptions {
    options: VerifyOptions,
}

#[pymethods]
impl PySignatureValidationOptions {
    #[new]
    fn new() -> Self {
        Self {
            options: VerifyOptions::default(),
        }
    }

    fn add_trust_anchor_der(&mut self, der: Vec<u8>) {
        self.options.trust_anchors_der.push(der);
    }

    fn add_intermediate_der(&mut self, der: Vec<u8>) {
        self.options.intermediates_der.push(der);
    }

    /// Add a certificate SHA-256 deny-list entry. It is enforced during path
    /// selection and does not merely annotate the resulting report.
    fn add_distrusted_certificate_sha256(&mut self, fingerprint: &str) -> PyResult<()> {
        self.options = self
            .options
            .clone()
            .with_distrusted_certificate_sha256(fingerprint)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(())
    }

    fn add_ocsp_response_der(&mut self, der: Vec<u8>) {
        self.options.ocsp_responses_der.push(der);
    }

    fn add_crl_der(&mut self, der: Vec<u8>) {
        self.options.crls_der.push(der);
    }

    fn set_validation_time_unix(&mut self, unix: u64) {
        self.options.validation_time_unix = Some(unix);
    }

    fn use_system_validation_time(&mut self) {
        self.options.validation_time_unix = None;
    }

    fn set_revocation_mode(&mut self, mode: &str) -> PyResult<()> {
        self.options.revocation_mode = match mode {
            "not_checked" | "not-checked" | "disabled" => SignatureRevocationMode::NotChecked,
            "offline_strict"
            | "offline-strict"
            | "offline_supplied_only"
            | "offline-supplied-only"
            | "require_any_fresh_evidence"
            | "require-any-fresh-evidence" => SignatureRevocationMode::OfflineStrict,
            "offline_best_effort" | "offline-best-effort" => {
                SignatureRevocationMode::OfflineBestEffort
            }
            "online_strict" | "online-strict" | "online_hard_fail" | "online-hard-fail"
            | "require_fresh_good" | "require-fresh-good" => SignatureRevocationMode::OnlineStrict,
            "online_best_effort"
            | "online-best-effort"
            | "online_best_evidence"
            | "online-best-evidence"
            | "soft_fail_network"
            | "soft-fail-network" => SignatureRevocationMode::OnlineBestEffort,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "unknown signature revocation mode '{mode}'"
                )))
            }
        };
        Ok(())
    }

    fn set_path_limits(
        &mut self,
        max_chain_depth: usize,
        max_path_candidates: usize,
    ) -> PyResult<()> {
        if max_chain_depth == 0 || max_path_candidates == 0 {
            return Err(PyValueError::new_err(
                "max_chain_depth and max_path_candidates must both be positive",
            ));
        }
        self.options.max_chain_depth = max_chain_depth;
        self.options.max_path_candidates = max_path_candidates;
        Ok(())
    }

    /// Set the shared CMS/PKIX algorithm policy from its JSON representation.
    /// Recognized legacy algorithms remain unavailable unless this policy
    /// explicitly permits them.
    fn set_algorithm_policy_json(&mut self, policy_json: &str) -> PyResult<()> {
        let policy: wellfriendpdf_engine::SignatureAlgorithmPolicy =
            serde_json::from_str(policy_json).map_err(|error| {
                PyValueError::new_err(format!("algorithm policy JSON: {error}"))
            })?;
        self.options = self
            .options
            .clone()
            .with_algorithm_policy(policy)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(())
    }

    /// Set the bounded AIA/OCSP/CRL retrieval policy.  Passing an offline
    /// policy keeps all transport disabled; the binding never enables it by
    /// default.
    fn set_retrieval_policy_json(&mut self, policy_json: &str) -> PyResult<()> {
        let policy: RetrievalPolicy = serde_json::from_str(policy_json)
            .map_err(|error| PyValueError::new_err(format!("retrieval policy JSON: {error}")))?;
        self.options = self
            .options
            .clone()
            .with_retrieval_policy(policy)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(())
    }

    /// Import portable evidence for offline replay.  Evidence is hash-checked
    /// here and cryptographically revalidated by the verification pipeline.
    fn set_evidence_bundle_json(&mut self, bundle_json: &str) -> PyResult<()> {
        let bundle: EvidenceBundle = serde_json::from_str(bundle_json)
            .map_err(|error| PyValueError::new_err(format!("evidence bundle JSON: {error}")))?;
        self.options = self
            .options
            .clone()
            .with_evidence_bundle(bundle)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(())
    }

    fn apply_trust_store(&mut self, store: PyRef<'_, PySignatureTrustStore>) -> PyResult<()> {
        let mut options = self.options.clone().with_trust_store(&store.store);
        for fingerprint in &store.distrusted_certificate_sha256 {
            options = options
                .with_distrusted_certificate_sha256(fingerprint)
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
        }
        self.options = options;
        Ok(())
    }

    fn apply_intermediate_store(&mut self, store: PyRef<'_, PySignatureIntermediateStore>) {
        self.options = self.options.clone().with_intermediate_store(&store.store);
    }

    fn apply_evidence_store(&mut self, store: PyRef<'_, PySignatureEvidenceStore>) -> PyResult<()> {
        let mut options = self.options.clone();
        options
            .ocsp_responses_der
            .extend(store.ocsp_responses_der.iter().cloned());
        options.crls_der.extend(store.crls_der.iter().cloned());
        if let Some(bundle) = &store.bundle {
            options = options
                .with_evidence_bundle(bundle.clone())
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
        }
        self.options = options;
        Ok(())
    }

    fn apply_retrieval_policy(
        &mut self,
        policy: PyRef<'_, PySignatureRetrievalPolicy>,
    ) -> PyResult<()> {
        self.options = self
            .options
            .clone()
            .with_retrieval_policy(policy.policy.clone())
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        Ok(())
    }

    fn set_cancellation(&mut self, cancellation: PyRef<'_, PySignatureValidationCancellation>) {
        self.options = self
            .options
            .clone()
            .with_cancellation_token(cancellation.token.clone());
    }
}

#[pymethods]
impl PyDocument {
    #[new]
    #[pyo3(signature = (source, password=None))]
    fn new(source: &Bound<'_, PyAny>, password: Option<&str>) -> PyResult<Self> {
        open_impl(source, password)
    }

    #[classmethod]
    #[pyo3(signature = (path, password=None))]
    fn from_path(
        _cls: &Bound<'_, PyType>,
        path: PathBuf,
        password: Option<&str>,
    ) -> PyResult<Self> {
        let engine = if let Some(password) = password {
            run_wellfriendpdf(|| ContentEngine::open_path_with_password(path, password.as_bytes()))?
        } else {
            run_wellfriendpdf(|| ContentEngine::open_path(path))?
        };
        Ok(Self {
            engine: Arc::new(engine),
        })
    }

    #[classmethod]
    #[pyo3(signature = (data, password=None))]
    fn from_bytes(
        _cls: &Bound<'_, PyType>,
        data: Vec<u8>,
        password: Option<&str>,
    ) -> PyResult<Self> {
        let engine = if let Some(password) = password {
            run_wellfriendpdf(|| {
                ContentEngine::open_bytes_with_password(data, password.as_bytes())
            })?
        } else {
            run_wellfriendpdf(|| ContentEngine::open_bytes(data))?
        };
        Ok(Self {
            engine: Arc::new(engine),
        })
    }

    #[getter]
    fn page_count(&self) -> PyResult<usize> {
        run_wellfriendpdf(|| self.engine.page_count())
    }

    #[getter]
    fn metadata<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let info = run_wellfriendpdf(|| DocumentInfo::gather(self.engine.document()))?;
        json_to_py(py, &info)
    }

    fn __len__(&self) -> PyResult<usize> {
        self.page_count()
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyResult<PyPageIterator> {
        Ok(PyPageIterator {
            engine: Arc::clone(&slf.engine),
            next: 1,
            total: run_wellfriendpdf(|| slf.engine.page_count())?,
        })
    }

    fn __getitem__(&self, index: isize) -> PyResult<PyPage> {
        let total = self.page_count()? as isize;
        let idx = if index < 0 { total + index } else { index };
        if idx < 0 || idx >= total {
            return Err(PyIndexError::new_err("page index out of range"));
        }
        self.page((idx + 1) as usize)
    }

    fn page(&self, number: usize) -> PyResult<PyPage> {
        validate_page(&self.engine, number)?;
        Ok(PyPage {
            engine: Arc::clone(&self.engine),
            number,
        })
    }

    fn pages(&self) -> PyResult<Vec<PyPage>> {
        let total = self.page_count()?;
        Ok((1..=total)
            .map(|number| PyPage {
                engine: Arc::clone(&self.engine),
                number,
            })
            .collect())
    }

    #[pyo3(signature = (page=None, profile="fast-text"))]
    fn extract_text(&self, page: Option<usize>, profile: &str) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        match page {
            Some(page) => {
                run_wellfriendpdf(|| self.engine.get_page_text_with_profile(page, profile))
            }
            None => all_text_with_profile(&self.engine, profile),
        }
    }

    #[pyo3(signature = (page=None))]
    fn extract_tables<'py>(&self, py: Python<'py>, page: Option<usize>) -> PyResult<Py<PyAny>> {
        if let Some(page) = page {
            let tables = run_wellfriendpdf(|| self.engine.extract_tables(page))?;
            return json_to_py(py, &tables);
        }

        let mut out = Vec::new();
        for number in 1..=run_wellfriendpdf(|| self.engine.page_count())? {
            let tables = run_wellfriendpdf(|| self.engine.extract_tables(number))?;
            for table in tables {
                out.push(json!({"page": number, "table": table}));
            }
        }
        json_to_py(py, &out)
    }

    #[pyo3(signature = (doc_type=None, min_confidence=0.0))]
    fn extract_fields<'py>(
        &self,
        py: Python<'py>,
        doc_type: Option<&str>,
        min_confidence: f32,
    ) -> PyResult<Py<PyAny>> {
        let options = ExtractOptions {
            min_confidence,
            doc_type: match doc_type {
                Some("auto") | None => None,
                Some(value) => Some(DocType::parse(value).ok_or_else(|| {
                    PyValueError::new_err(
                        "doc_type must be auto, invoice, receipt, form, or generic",
                    )
                })?),
            },
            ..Default::default()
        };
        let fields = run_wellfriendpdf(|| self.engine.extract_fields(&options))?;
        json_to_py(py, &fields)
    }

    #[pyo3(signature = (profile="fast-text", ocr=None, ocr_lang="eng", ocr_dpi=300))]
    fn document_model<'py>(
        &self,
        py: Python<'py>,
        profile: &str,
        ocr: Option<&Bound<'py, PyAny>>,
        ocr_lang: &str,
        ocr_dpi: u32,
    ) -> PyResult<Py<PyAny>> {
        let profile = parse_profile_py(profile)?;
        let options = parse_options_with_ocr(ocr, ocr_lang, ocr_dpi)?;
        let document =
            run_wellfriendpdf(|| self.engine.parse_document_with_profile(profile, &options))?;
        let value: serde_json::Value = serde_json::from_str(&document.to_json())
            .map_err(|err| WellfriendError::new_err(format!("document JSON error: {err}")))?;
        json_to_py(py, &value)
    }

    #[pyo3(signature = (detect_headings=true, profile="fast-text", ocr=None, ocr_lang="eng", ocr_dpi=300))]
    fn to_markdown<'py>(
        &self,
        detect_headings: bool,
        profile: &str,
        ocr: Option<&Bound<'py, PyAny>>,
        ocr_lang: &str,
        ocr_dpi: u32,
    ) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        if detect_headings {
            let options = parse_options_with_ocr(ocr, ocr_lang, ocr_dpi)?;
            let document =
                run_wellfriendpdf(|| self.engine.parse_document_with_profile(profile, &options))?;
            Ok(document.to_markdown(&SerializeOptions::default()))
        } else {
            all_text_with_profile(&self.engine, profile)
        }
    }

    #[pyo3(signature = (detect_headings=true, profile="fast-text", ocr=None, ocr_lang="eng", ocr_dpi=300))]
    fn markdown<'py>(
        &self,
        detect_headings: bool,
        profile: &str,
        ocr: Option<&Bound<'py, PyAny>>,
        ocr_lang: &str,
        ocr_dpi: u32,
    ) -> PyResult<String> {
        self.to_markdown(detect_headings, profile, ocr, ocr_lang, ocr_dpi)
    }

    #[pyo3(signature = (profile="fast-text", ocr=None, ocr_lang="eng", ocr_dpi=300))]
    fn to_html<'py>(
        &self,
        profile: &str,
        ocr: Option<&Bound<'py, PyAny>>,
        ocr_lang: &str,
        ocr_dpi: u32,
    ) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        let options = parse_options_with_ocr(ocr, ocr_lang, ocr_dpi)?;
        let document =
            run_wellfriendpdf(|| self.engine.parse_document_with_profile(profile, &options))?;
        Ok(document.to_html(&SerializeOptions::default()))
    }

    #[pyo3(signature = (page, dpi=150))]
    fn render(&self, page: usize, dpi: u32) -> PyResult<Vec<u8>> {
        run_wellfriendpdf(|| self.engine.render_page_png_fast(page, dpi))
    }

    // ── Report surfaces (shared wellfriendpdf_engine::sdk facade) ────────────────────
    //
    // Each returns a Python dict parsed from the SDK's versioned-JSON envelope
    // `{"schema_version", "kind", "report"}`. The same facade backs the C ABI,
    // so a report requested from Python and from C is byte-identical JSON.

    /// Security report: encryption, signatures, risky active content, findings.
    fn security_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::security_report_json(bytes, None))
    }

    /// Risky active-content inventory (JavaScript, launch/URI actions, etc.).
    fn risky_content_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::risky_content_report_json(bytes, None))
    }

    /// Parser diagnostics: repair/xref/revisions/linearization/encryption.
    /// `mode` is one of "strict" | "repair" | "audit".
    #[pyo3(signature = (mode="repair"))]
    fn parser_report<'py>(&self, py: Python<'py>, mode: &str) -> PyResult<Py<PyAny>> {
        let mode = mode.to_string();
        self.report_json(py, |bytes| {
            sdk::parser_report_json(bytes, Some(&mode), None)
        })
    }

    /// Color / prepress report. `profile` is "generic" | "pdfa" | "pdfx".
    #[pyo3(signature = (profile="generic"))]
    fn color_report<'py>(&self, py: Python<'py>, profile: &str) -> PyResult<Py<PyAny>> {
        let profile = profile.to_string();
        self.report_json(py, |bytes| sdk::color_report_json(bytes, Some(&profile)))
    }

    /// PDF/A validation report. `profile` in pdfa1b/pdfa2b/pdfa2a/pdfa3b/pdfa3a.
    #[pyo3(signature = (profile="pdfa2b"))]
    fn validate_pdfa<'py>(&self, py: Python<'py>, profile: &str) -> PyResult<Py<PyAny>> {
        let profile = profile.to_string();
        self.report_json(py, |bytes| {
            sdk::pdfa_validation_json(bytes, Some(&profile), None)
        })
    }

    /// PDF/UA (accessibility) validation report.
    fn validate_pdfua<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::pdfua_validation_json(bytes, None))
    }

    /// Prompt 26 clause-mapped PDF/A validation. `target` e.g. "PDF/A-2B".
    #[pyo3(signature = (target=None))]
    fn validate_pdfa_standards<'py>(
        &self,
        py: Python<'py>,
        target: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let target = target.map(str::to_string);
        self.report_json(py, |bytes| {
            sdk::pdfa_standards_json(bytes, target.as_deref(), None)
        })
    }

    /// Prompt 26 clause-mapped PDF/UA validation. `target` e.g. "PDF/UA-1".
    #[pyo3(signature = (target=None))]
    fn validate_pdfua_standards<'py>(
        &self,
        py: Python<'py>,
        target: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let target = target.map(str::to_string);
        self.report_json(py, |bytes| {
            sdk::pdfua_standards_json(bytes, target.as_deref(), None)
        })
    }

    /// Prompt 26 clause-mapped PDF/X validation. `target` e.g. "PDF/X-4".
    #[pyo3(signature = (target=None))]
    fn validate_pdfx_standards<'py>(
        &self,
        py: Python<'py>,
        target: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let target = target.map(str::to_string);
        self.report_json(py, |bytes| {
            sdk::pdfx_standards_json(bytes, target.as_deref(), None)
        })
    }

    /// Prompt 26 combined PDF/A + PDF/UA + PDF/X validation with cross-profile
    /// conflicts. A single profile passing never hides another failing.
    #[pyo3(signature = (target=None))]
    fn validate_standards_all<'py>(
        &self,
        py: Python<'py>,
        target: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let target = target.map(str::to_string);
        self.report_json(py, |bytes| {
            sdk::standards_all_json(bytes, target.as_deref(), None)
        })
    }

    /// Standards-profile report. `profile` in pdfa/pdfua/pdfx/security/all.
    #[pyo3(signature = (profile="all"))]
    fn validate<'py>(&self, py: Python<'py>, profile: &str) -> PyResult<Py<PyAny>> {
        let profile = profile.to_string();
        self.report_json(py, |bytes| {
            sdk::standards_profile_json(bytes, Some(&profile), None)
        })
    }

    /// Combined interactive report (forms + annotations + page operations).
    fn interactive_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::interactive_report_json(bytes, None))
    }

    /// AcroForm field inventory (trees, inheritance, widgets, XFA status).
    fn forms_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::forms_report_json(bytes, None))
    }

    /// Prompt 16 bounded XFA packet inventory and XML-safety report.
    fn xfa_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::xfa_report_json(bytes, None))
    }

    /// Prompt 16 static XFA fields/datasets/layout/provenance extraction.
    fn xfa_extract<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::xfa_extract_json(bytes, None))
    }

    /// Prompt 16 script/event inventory and fail-closed default policy.
    fn xfa_script_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::xfa_script_report_json(bytes, None))
    }

    /// Prompt 16 XFA-specific security/signature/redaction posture.
    fn xfa_security_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::xfa_security_report_json(bytes, None))
    }

    /// Bounded minimal dynamic XFA runtime report.
    #[pyo3(signature = (script_policy="disabled", execute_events=false))]
    fn xfa_runtime_report<'py>(
        &self,
        py: Python<'py>,
        script_policy: &str,
        execute_events: bool,
    ) -> PyResult<Py<PyAny>> {
        let script_policy = script_policy.to_string();
        self.report_json(py, |bytes| {
            sdk::xfa_runtime_report_json(bytes, Some(&script_policy), execute_events, None)
        })
    }

    /// Annotation inventory (kinds, quads, appearance status, unsafe actions).
    fn annotations_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::annotation_report_json(bytes, None))
    }

    /// Prompt 17 rich-media inventory. No player, network, filesystem, or media
    /// codec is invoked.
    fn rich_media_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::rich_media_report_json(bytes, None))
    }

    /// Prompt 17 annotation appearance generation report.
    #[pyo3(signature = (options_json=None))]
    fn annotation_appearance_report<'py>(
        &self,
        py: Python<'py>,
        options_json: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let options = options_json.map(str::to_string);
        self.report_json(py, |bytes| {
            sdk::annotation_appearance_report_json(bytes, options.as_deref(), None)
        })
    }

    /// Prompt 17 request-specific non-axis redaction plan.
    fn nonaxis_redaction_plan<'py>(
        &self,
        py: Python<'py>,
        options_json: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = options_json.to_string();
        self.report_json(py, |bytes| {
            sdk::nonaxis_redaction_plan_json(bytes, &options, None)
        })
    }

    /// Combined Prompt 17 report.
    fn prompt17_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt17_report_json(bytes, None))
    }

    /// Combined Prompt 18 secure-mutation report.
    fn prompt18_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt18_report_json(bytes, None))
    }

    fn prompt18b_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt18b_report_json(bytes, None))
    }

    fn form_js_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::form_js_report_json(bytes, None))
    }

    fn form_action_graph<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::form_action_graph_json(bytes, None))
    }

    fn interactive_data_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| {
            sdk::interactive_data_closeout_report_json(bytes, None)
        })
    }

    #[pyo3(signature = (layout="page-faithful"))]
    fn word_pagination_audit<'py>(&self, py: Python<'py>, layout: &str) -> PyResult<Py<PyAny>> {
        let layout = layout.to_string();
        self.report_json(py, |bytes| {
            sdk::word_pagination_audit_json(bytes, &layout, None)
        })
    }

    fn prompt19_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt19_report_json(bytes, None))
    }

    fn prompt20_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt20_report_json(bytes, None))
    }

    fn prompt20b_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt20b_report_json(bytes, None))
    }

    fn prompt31_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt31_report_json(bytes, None))
    }

    #[pyo3(signature = (page, source_text, replacement_text))]
    fn prompt31_provenance<'py>(
        &self,
        py: Python<'py>,
        page: usize,
        source_text: &str,
        replacement_text: &str,
    ) -> PyResult<Py<PyAny>> {
        let source_text = source_text.to_string();
        let replacement_text = replacement_text.to_string();
        self.report_json(py, |bytes| {
            sdk::prompt31_provenance_json(bytes, page, &source_text, &replacement_text, None)
        })
    }

    #[pyo3(signature = (request_json))]
    fn prompt31_edit_eligibility<'py>(
        &self,
        py: Python<'py>,
        request_json: &str,
    ) -> PyResult<Py<PyAny>> {
        let request_json = request_json.to_string();
        self.report_json(py, |bytes| {
            sdk::prompt31_edit_eligibility_json(bytes, &request_json, None)
        })
    }

    #[pyo3(signature = (request_json, output=None))]
    fn prompt31_operator_text_edit<'py>(
        &self,
        py: Python<'py>,
        request_json: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::prompt31_operator_text_edit_json(&bytes, request_json, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    fn prompt21_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt21_report_json(bytes, None))
    }

    #[pyo3(signature = (page=1, options_json=None))]
    fn prompt21_raster_vector_report<'py>(
        &self,
        py: Python<'py>,
        page: usize,
        options_json: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let options = options_json.map(str::to_string);
        self.report_json(py, |bytes| {
            sdk::prompt21_raster_vector_report_json(bytes, page, options.as_deref(), None)
        })
    }

    fn prompt21_font_reconstruction_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| {
            sdk::prompt21_font_reconstruction_report_json(bytes, None)
        })
    }

    fn prompt21_object_stream_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| {
            sdk::prompt21_object_stream_report_json(bytes, None)
        })
    }

    #[pyo3(signature = (output=None))]
    fn prompt21_pack_object_streams<'py>(
        &self,
        py: Python<'py>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::prompt21_pack_object_streams_json(&bytes, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    fn prompt22_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt22_report_json(bytes, None))
    }

    fn prompt23_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::prompt23_report_json(bytes, None))
    }

    fn writer_determinism_audit<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::writer_determinism_audit_json(bytes, None))
    }

    fn writer_external_diff<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::writer_external_diff_json(bytes, None))
    }

    fn writer_closeout_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::writer_closeout_report_json(bytes, None))
    }

    fn pubsec_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::pubsec_report_json(bytes, None))
    }

    fn aes_gcm_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::aes_gcm_report_json(bytes, None))
    }

    fn pdf_mac_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::pdf_mac_report_json(bytes, None))
    }

    fn pdf_mac_verify<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::pdf_mac_verify_json(bytes, None))
    }

    #[pyo3(signature = (output=None))]
    fn pdf_mac_create<'py>(
        &self,
        py: Python<'py>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| sdk::pdf_mac_create_json(&bytes, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (options_json=None, output=None))]
    fn prompt22_optimize<'py>(
        &self,
        py: Python<'py>,
        options_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let options = options_json.map(str::to_string);
        let (out, report) = run_wellfriendpdf(|| {
            sdk::prompt22_optimize_pdf_json(&bytes, options.as_deref(), None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (page=1))]
    fn prompt20b_text_range_analyze<'py>(
        &self,
        py: Python<'py>,
        page: usize,
    ) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| {
            sdk::prompt20b_text_range_analyze_json(bytes, page, None)
        })
    }

    #[pyo3(signature = (request_json, output=None))]
    fn edit_text_range<'py>(
        &self,
        py: Python<'py>,
        request_json: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::prompt20b_text_range_edit_json(&bytes, request_json, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (page=1))]
    fn prompt20_vector_list<'py>(&self, py: Python<'py>, page: usize) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| {
            sdk::prompt20_vector_list_json(bytes, page, None)
        })
    }

    #[pyo3(signature = (page))]
    fn prompt31_path_provenance<'py>(&self, py: Python<'py>, page: usize) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| {
            sdk::prompt31_path_provenance_json(bytes, page, None)
        })
    }

    #[pyo3(signature = (page, stable_id, operation_json, options_json=None, output=None))]
    fn prompt31_path_edit<'py>(
        &self,
        py: Python<'py>,
        page: usize,
        stable_id: &str,
        operation_json: &str,
        options_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::prompt31_path_edit_json(
                &bytes,
                page,
                stable_id,
                operation_json,
                options_json,
                None,
            )
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (page, occurrence=None))]
    fn prompt31_image_eligibility<'py>(
        &self,
        py: Python<'py>,
        page: usize,
        occurrence: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let occurrence = occurrence.map(str::to_string);
        self.report_json(py, |bytes| {
            let _ = occurrence.as_deref();
            sdk::prompt31_image_eligibility_json(bytes, page, None)
        })
    }

    fn associated_files_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::associated_files_report_json(bytes, None))
    }

    fn mask_redaction_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::mask_redaction_report_json(bytes, None))
    }

    fn edit_policy_report<'py>(&self, py: Python<'py>, operation: &str) -> PyResult<Py<PyAny>> {
        let operation = operation.to_string();
        self.report_json(py, |bytes| {
            sdk::edit_policy_report_json(bytes, &operation, None)
        })
    }

    /// Page-operations report (boxes, labels, destinations, preservation risk).
    fn pages_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::page_operations_report_json(bytes, None))
    }

    /// Signature report (validity, trust, coverage, LTV, certificate).
    fn signature_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::signature_report_json(bytes, None))
    }

    /// Prompt 24 signature report with explicit trust/evidence options JSON.
    fn signature_report_with_options<'py>(
        &self,
        py: Python<'py>,
        options_json: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = options_json.to_string();
        self.report_json(py, |bytes| {
            sdk::signature_report_with_options_json(bytes, &options, None)
        })
    }

    /// Prompt 24 validation outcome with an explicit replayable evidence bundle.
    /// Online retrieval is still disabled unless the options JSON enables the
    /// shared bounded retrieval policy.
    fn signature_validation_with_evidence<'py>(
        &self,
        py: Python<'py>,
        options_json: &str,
    ) -> PyResult<Py<PyAny>> {
        let options = options_json.to_string();
        self.report_json(py, |bytes| {
            sdk::signature_validation_with_evidence_json(bytes, &options, None)
        })
    }

    /// Validate signatures through the owned Prompt 24 options object.  This
    /// bypasses the JSON facade and calls the same typed engine API exposed by
    /// the Rust SDK.
    fn signature_validation<'py>(
        &self,
        py: Python<'py>,
        options: PyRef<'_, PySignatureValidationOptions>,
    ) -> PyResult<Py<PyAny>> {
        let options = options.options.clone();
        let reports = run_wellfriendpdf(|| self.engine.verify_signatures_with_options(&options))?;
        json_to_py(py, &reports)
    }

    /// Validate signatures and return both reports and an exportable evidence
    /// bundle using the owned Prompt 24 options object.
    fn signature_validation_with_evidence_options<'py>(
        &self,
        py: Python<'py>,
        options: PyRef<'_, PySignatureValidationOptions>,
    ) -> PyResult<Py<PyAny>> {
        let options = options.options.clone();
        let outcome = run_wellfriendpdf(|| {
            self.engine
                .verify_signatures_with_options_and_evidence(&options)
        })?;
        json_to_py(py, &outcome)
    }

    /// Prompt 25 signature-preserving form-fill plan.
    #[pyo3(signature = (field_name, value, options_json="{}"))]
    fn signature_preserving_form_plan<'py>(
        &self,
        py: Python<'py>,
        field_name: &str,
        value: &str,
        options_json: &str,
    ) -> PyResult<Py<PyAny>> {
        let field_name = field_name.to_string();
        let value = value.to_string();
        let options = options_json.to_string();
        self.report_json(py, |bytes| {
            sdk::signature_preserving_form_plan_json(bytes, &field_name, &value, &options, None)
        })
    }

    /// Font inventory (name, type, embedding status, subsetting, encoding).
    fn font_report<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::font_report_json(bytes, None))
    }

    /// Text semantic model: pages → blocks → paragraphs → lines → words/spans
    /// with geometry, confidence, provenance, and reading order. `pages` empty
    /// or None means all pages.
    #[pyo3(signature = (pages=None))]
    fn text_semantic<'py>(
        &self,
        py: Python<'py>,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        self.report_json(py, |bytes| sdk::text_semantic_json(bytes, &pages, None))
    }

    /// RAG-ready semantic chunk set (canonical model → chunks).
    fn chunks<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        self.report_json(py, |bytes| sdk::chunk_report_json(bytes, None))
    }

    /// Prompt 15 provenance-aware RAG chunks with stable hashes, table/cell,
    /// CJK dictionary, structure/MCID, ParentTree, and security metadata.
    #[pyo3(signature = (pages=None))]
    fn advanced_chunks<'py>(
        &self,
        py: Python<'py>,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        self.report_json(py, |bytes| {
            sdk::advanced_chunk_report_json(bytes, &pages, None)
        })
    }

    /// Full Prompt 15 semantic binding bundle as a versioned Python dictionary.
    #[pyo3(signature = (pages=None))]
    fn semantic_bundle<'py>(
        &self,
        py: Python<'py>,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        self.report_json(py, |bytes| {
            sdk::semantic_binding_report_json(bytes, &pages, None)
        })
    }

    /// Semantic text and CJK dictionary-token search with source provenance.
    #[pyo3(signature = (query, pages=None))]
    fn semantic_search<'py>(
        &self,
        py: Python<'py>,
        query: &str,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        let query = query.to_string();
        self.report_json(py, |bytes| {
            sdk::semantic_search_report_json(bytes, &pages, &query, None)
        })
    }

    /// TableFormer/Table Transformer hook and backend availability status.
    fn table_proposal_status<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let json = run_wellfriendpdf(sdk::table_proposal_status_json)?;
        parse_json_str(py, &json)
    }

    /// Tagged-structure semantic document (structure tree / MCID model).
    #[pyo3(signature = (pages=None))]
    fn semantic_document<'py>(
        &self,
        py: Python<'py>,
        pages: Option<Vec<usize>>,
    ) -> PyResult<Py<PyAny>> {
        let pages = pages.unwrap_or_default();
        self.report_json(py, |bytes| sdk::semantic_document_json(bytes, &pages, None))
    }

    // ── Output-producing operations (return (bytes, report) tuples) ──────────

    /// Prompt 25 signature-preserving form-fill execution with post-edit validation.
    #[pyo3(signature = (field_name, value, options_json="{}", explicit_invalidation_override=false, output=None))]
    fn signature_preserving_form_edit<'py>(
        &self,
        py: Python<'py>,
        field_name: &str,
        value: &str,
        options_json: &str,
        explicit_invalidation_override: bool,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::signature_preserving_form_edit_json(
                &bytes,
                field_name,
                value,
                options_json,
                explicit_invalidation_override,
                None,
            )
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (page, old_text, new_text, mode="rtl-reflow", options_json=None, output=None))]
    #[allow(clippy::too_many_arguments)]
    fn prompt20_text_edit<'py>(
        &self,
        py: Python<'py>,
        page: usize,
        old_text: &str,
        new_text: &str,
        mode: &str,
        options_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::prompt20_text_edit_json(&bytes, page, old_text, new_text, mode, options_json, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (page, stable_id, operation_json, options_json=None, output=None))]
    fn prompt20_vector_edit<'py>(
        &self,
        py: Python<'py>,
        page: usize,
        stable_id: &str,
        operation_json: &str,
        options_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::prompt20_vector_edit_json(
                &bytes,
                page,
                stable_id,
                operation_json,
                options_json,
                None,
            )
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (page, annotation_index=0, options_json=None, signature_policy_override=false, output=None))]
    fn prompt20_ink_fit<'py>(
        &self,
        py: Python<'py>,
        page: usize,
        annotation_index: usize,
        options_json: Option<&str>,
        signature_policy_override: bool,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::prompt20_ink_fit_json(
                &bytes,
                page,
                annotation_index,
                options_json,
                signature_policy_override,
                None,
            )
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Render a supported XFA preview as PDF overlay bytes plus a versioned
    /// report. The result can be written to `output`.
    #[pyo3(signature = (script_policy="disabled", execute_events=false, dpi=72, output=None))]
    fn xfa_render<'py>(
        &self,
        py: Python<'py>,
        script_policy: &str,
        execute_events: bool,
        dpi: u32,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::xfa_render_preview_json(&bytes, Some(script_policy), execute_events, dpi, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Flatten the supported static XFA subset under an explicit Prompt 16 mode.
    #[pyo3(signature = (mode="flatten_supported_static", output=None))]
    fn xfa_flatten<'py>(
        &self,
        py: Python<'py>,
        mode: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| sdk::xfa_flatten_json(&bytes, Some(mode), None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Remove or neutralize XFA active content under an explicit policy.
    #[pyo3(signature = (mode="remove_scripts_events_connections", output=None))]
    fn xfa_sanitize<'py>(
        &self,
        py: Python<'py>,
        mode: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| sdk::xfa_sanitize_json(&bytes, Some(mode), None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Export annotation XFDF. Returns `(xfdf_bytes, report)`.
    #[pyo3(signature = (output=None))]
    fn annotation_xfdf_export<'py>(
        &self,
        py: Python<'py>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| sdk::annotation_xfdf_export_json(&bytes, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Import annotation XFDF. Returns `(pdf_bytes, report)`.
    #[pyo3(signature = (xfdf, options_json=None, output=None))]
    fn annotation_xfdf_import<'py>(
        &self,
        py: Python<'py>,
        xfdf: &[u8],
        options_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::annotation_xfdf_import_json(&bytes, xfdf, options_json, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Generate annotation appearance streams. Returns `(pdf_bytes, report)`.
    #[pyo3(signature = (options_json=None, output=None))]
    fn annotation_appearance_generate<'py>(
        &self,
        py: Python<'py>,
        options_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::annotation_appearance_generate_json(&bytes, options_json, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Apply an explicit rich-media policy. Returns `(pdf_bytes, report)`.
    #[pyo3(signature = (mode="remove_active_content", custom_json=None, output=None))]
    fn rich_media_sanitize<'py>(
        &self,
        py: Python<'py>,
        mode: &str,
        custom_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::rich_media_sanitize_json(&bytes, Some(mode), custom_json, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Flatten safe static media posters and remove active media/payloads.
    #[pyo3(signature = (output=None))]
    fn rich_media_flatten_poster<'py>(
        &self,
        py: Python<'py>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::rich_media_flatten_poster_json(&bytes, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Apply polygonal non-axis image redaction. Returns `(pdf_bytes, report)`.
    #[pyo3(signature = (options_json, output=None))]
    fn redact_image_nonaxis<'py>(
        &self,
        py: Python<'py>,
        options_json: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::nonaxis_redaction_apply_json(&bytes, options_json, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (options_json, output=None))]
    fn redact_image_mask<'py>(
        &self,
        py: Python<'py>,
        options_json: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::redact_image_mask_json(&bytes, options_json, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (options_json, output=None))]
    fn redact_inline_image<'py>(
        &self,
        py: Python<'py>,
        options_json: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::redact_inline_image_json(&bytes, options_json, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    fn associated_file_extract<'py>(
        &self,
        py: Python<'py>,
        stable_id: &str,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (payload, report) =
            run_wellfriendpdf(|| sdk::associated_files_extract_json(&bytes, stable_id, None))?;
        Ok((
            PyBytes::new(py, &payload).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (payload, options_json, output=None))]
    fn associated_file_add<'py>(
        &self,
        py: Python<'py>,
        payload: &[u8],
        options_json: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::associated_files_add_json(&bytes, payload, options_json, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (payload, options_json, output=None))]
    fn associated_file_update_owner<'py>(
        &self,
        py: Python<'py>,
        payload: &[u8],
        options_json: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::associated_files_update_owner_json(&bytes, payload, options_json, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (options_json, output=None))]
    fn associated_file_remove_owner<'py>(
        &self,
        py: Python<'py>,
        options_json: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::associated_files_remove_owner_json(&bytes, options_json, None)
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (field_name, value, signature_policy_override=false, output=None))]
    fn incremental_form_edit<'py>(
        &self,
        py: Python<'py>,
        field_name: &str,
        value: &str,
        signature_policy_override: bool,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::incremental_form_edit_json(
                &bytes,
                field_name,
                value,
                signature_policy_override,
                None,
            )
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (options_json, signature_policy_override=false, output=None))]
    fn incremental_annotation_edit<'py>(
        &self,
        py: Python<'py>,
        options_json: &str,
        signature_policy_override: bool,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::incremental_annotation_edit_json(
                &bytes,
                options_json,
                signature_policy_override,
                None,
            )
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (options_json, signature_policy_override=false, output=None))]
    fn incremental_page_property_edit<'py>(
        &self,
        py: Python<'py>,
        options_json: &str,
        signature_policy_override: bool,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| {
            sdk::incremental_page_property_edit_json(
                &bytes,
                options_json,
                signature_policy_override,
                None,
            )
        })?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (options_json=None, output=None))]
    fn associated_files_sanitize<'py>(
        &self,
        py: Python<'py>,
        options_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::associated_files_sanitize_json(&bytes, options_json, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (options_json=None, output=None))]
    fn form_js_sanitize<'py>(
        &self,
        py: Python<'py>,
        options_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::form_js_sanitize_json(&bytes, options_json, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (options_json=None, output=None))]
    fn form_js_flatten_values<'py>(
        &self,
        py: Python<'py>,
        options_json: Option<&str>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::form_js_flatten_values_json(&bytes, options_json, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    #[pyo3(signature = (stable_ids, output=None))]
    fn associated_files_remove<'py>(
        &self,
        py: Python<'py>,
        stable_ids: Vec<String>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::associated_files_remove_json(&bytes, &stable_ids, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Sanitize: remove active/risky content and re-scan. `policy` is one of
    /// "strict" | "balanced" | "preserve-visual". Returns `(bytes, report)` and
    /// optionally writes the sanitized PDF to `output`.
    #[pyo3(signature = (policy="balanced", output=None))]
    fn sanitize<'py>(
        &self,
        py: Python<'py>,
        policy: &str,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| sdk::sanitize_json(&bytes, Some(policy), None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Canonicalize: deterministic full-rewrite copy + audit report. Returns
    /// `(bytes, report)`; `date_epoch` fixes the source date epoch.
    #[pyo3(signature = (date_epoch=None, output=None))]
    fn canonicalize<'py>(
        &self,
        py: Python<'py>,
        date_epoch: Option<i64>,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) = run_wellfriendpdf(|| sdk::canonicalize_json(&bytes, date_epoch, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }

    /// Redact every occurrence of `terms` (case-insensitive), full-rewrite, and
    /// verify absence. Returns `(bytes, report)`. `strict=True` raises if a term
    /// survives. Writes to `output` when given.
    #[pyo3(signature = (terms, strict=false, output=None))]
    fn redact<'py>(
        &self,
        py: Python<'py>,
        terms: Vec<String>,
        strict: bool,
        output: Option<PathBuf>,
    ) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
        let bytes = self.file_bytes();
        let (out, report) =
            run_wellfriendpdf(|| sdk::redact_terms_json(&bytes, &terms, strict, None))?;
        write_optional(&output, &out)?;
        Ok((
            PyBytes::new(py, &out).unbind(),
            parse_json_str(py, &report)?,
        ))
    }
}

impl PyDocument {
    /// The original file bytes backing this document (copied out of the reader).
    fn file_bytes(&self) -> Vec<u8> {
        self.engine.document().reader().file_bytes().to_vec()
    }

    /// Run a facade report over this document's bytes and parse it to a dict.
    fn report_json<'py, F>(&self, py: Python<'py>, f: F) -> PyResult<Py<PyAny>>
    where
        F: FnOnce(&[u8]) -> wellfriendpdf_engine::Result<String>,
    {
        let bytes = self.file_bytes();
        let json = run_wellfriendpdf(|| f(&bytes))?;
        parse_json_str(py, &json)
    }
}

#[pymethods]
impl PyPage {
    #[getter]
    fn number(&self) -> usize {
        self.number
    }

    #[getter]
    fn text(&self) -> PyResult<String> {
        run_wellfriendpdf(|| self.engine.get_page_text(self.number))
    }

    #[getter]
    fn words<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        page_words(py, &self.engine, self.number)
    }

    #[getter]
    fn tables<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let tables = run_wellfriendpdf(|| self.engine.extract_tables(self.number))?;
        json_to_py(py, &tables)
    }

    #[getter]
    fn images<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        page_images(py, &self.engine, self.number)
    }

    #[pyo3(signature = (profile="fast-text"))]
    fn text_with_profile(&self, profile: &str) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        run_wellfriendpdf(|| self.engine.get_page_text_with_profile(self.number, profile))
    }

    #[pyo3(signature = (x0, y0, x1, y1))]
    fn region(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> PyResult<PyRegionPage> {
        let region = run_wellfriendpdf(|| PageRegion::new(x0, y0, x1, y1))?;
        let region = run_wellfriendpdf(|| self.engine.clamp_region_to_page(self.number, region))?;
        Ok(PyRegionPage {
            engine: Arc::clone(&self.engine),
            number: self.number,
            region,
        })
    }

    #[pyo3(signature = (x0, y0, x1, y1))]
    fn within(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> PyResult<PyRegionPage> {
        self.region(x0, y0, x1, y1)
    }

    #[pyo3(signature = (detect_headings=true, profile="fast-text"))]
    fn markdown(&self, detect_headings: bool, profile: &str) -> PyResult<String> {
        let profile = parse_profile_py(profile)?;
        if !detect_headings {
            return run_wellfriendpdf(|| {
                self.engine.get_page_text_with_profile(self.number, profile)
            });
        }
        let options = ParseOptions {
            pages: vec![self.number],
            ..Default::default()
        };
        let document =
            run_wellfriendpdf(|| self.engine.parse_document_with_profile(profile, &options))?;
        Ok(document.to_markdown(&SerializeOptions::default()))
    }

    #[pyo3(signature = (dpi=150))]
    fn render(&self, dpi: u32) -> PyResult<Vec<u8>> {
        run_wellfriendpdf(|| self.engine.render_page_png_fast(self.number, dpi))
    }
}

#[pymethods]
impl PyRegionPage {
    #[getter]
    fn number(&self) -> usize {
        self.number
    }

    #[getter]
    fn bbox(&self) -> Vec<f64> {
        self.region.as_array().to_vec()
    }

    #[getter]
    fn text(&self) -> PyResult<String> {
        run_wellfriendpdf(|| self.engine.extract_text_in_region(self.number, self.region))
    }

    #[getter]
    fn words<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let words = run_wellfriendpdf(|| {
            self.engine
                .extract_words_in_region(self.number, self.region)
        })?;
        json_to_py(py, &words)
    }

    #[getter]
    fn tables<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        let tables = run_wellfriendpdf(|| {
            self.engine
                .extract_tables_in_region(self.number, self.region)
        })?;
        json_to_py(py, &tables)
    }

    #[getter]
    fn images<'py>(&self, py: Python<'py>) -> PyResult<Py<PyAny>> {
        page_region_images(py, &self.engine, self.number, self.region)
    }
}

#[pymethods]
impl PyPageIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<Option<PyPage>> {
        if self.next > self.total {
            return Ok(None);
        }
        let page = PyPage {
            engine: Arc::clone(&self.engine),
            number: self.next,
        };
        self.next += 1;
        Ok(Some(page))
    }
}

#[pyfunction]
#[pyo3(signature = (source, password=None))]
fn open(source: &Bound<'_, PyAny>, password: Option<&str>) -> PyResult<PyDocument> {
    open_impl(source, password)
}

#[pyfunction]
#[pyo3(signature = (token, signature_value, options_json="{}"))]
fn timestamp_token_validation<'py>(
    py: Python<'py>,
    token: &[u8],
    signature_value: &[u8],
    options_json: &str,
) -> PyResult<Py<PyAny>> {
    let json = run_wellfriendpdf(|| {
        sdk::timestamp_token_validation_json(token, signature_value, options_json)
    })?;
    parse_json_str(py, &json)
}

#[pyfunction]
#[pyo3(signature = (inputs, output=None, passwords=None))]
fn merge_pdfs(
    inputs: Vec<PathBuf>,
    output: Option<PathBuf>,
    passwords: Option<Vec<String>>,
) -> PyResult<Vec<u8>> {
    if inputs.is_empty() {
        return Err(PyValueError::new_err("inputs must not be empty"));
    }
    let mut engines = Vec::with_capacity(inputs.len());
    for (idx, path) in inputs.iter().enumerate() {
        let password = passwords
            .as_ref()
            .and_then(|values| values.get(idx))
            .map(String::as_str);
        engines.push(open_engine_path(path, password)?);
    }
    let mut specs = Vec::with_capacity(engines.len());
    for engine in &engines {
        let total = run_wellfriendpdf(|| engine.page_count())?;
        specs.push((engine.document(), (1..=total).collect::<Vec<_>>()));
    }
    let bytes = run_wellfriendpdf(|| wellfriendpdf_engine::build_merged(&specs))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, pages, output=None, password=None))]
fn extract_pages(
    pdf: PathBuf,
    pages: &str,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let total = run_wellfriendpdf(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let bytes = run_wellfriendpdf(|| engine.extract_pages(&selected))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, angle, pages="all", relative=false, output=None, password=None))]
fn rotate_pdf(
    pdf: PathBuf,
    angle: i32,
    pages: &str,
    relative: bool,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let total = run_wellfriendpdf(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let rotation = if relative {
        wellfriendpdf_engine::Rotation::Relative(angle)
    } else {
        wellfriendpdf_engine::Rotation::Absolute(angle)
    };
    let bytes =
        run_wellfriendpdf(|| wellfriendpdf_engine::rotate_pages(&engine, &selected, rotation))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, password=None))]
fn decrypt_pdf(pdf: PathBuf, output: Option<PathBuf>, password: Option<&str>) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let bytes = run_wellfriendpdf(|| wellfriendpdf_engine::decrypt_pdf(&engine))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, user_password="", owner_password=None, output=None, algo="aes256", permissions=-1, password=None))]
fn encrypt_pdf(
    pdf: PathBuf,
    user_password: &str,
    owner_password: Option<&str>,
    output: Option<PathBuf>,
    algo: &str,
    permissions: i32,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    use wellfriendpdf_engine::crypto::{secret_bytes, EncryptAlgorithm, EncryptParams};
    let engine = open_engine_path(&pdf, password)?;
    let algorithm = EncryptAlgorithm::parse(algo)
        .ok_or_else(|| PyValueError::new_err("algo must be aes256, aesgcm, aes128, or rc4"))?;
    let owner = owner_password.unwrap_or(user_password);
    let params = EncryptParams {
        user_password: secret_bytes(user_password.as_bytes().to_vec()),
        owner_password: secret_bytes(owner.as_bytes().to_vec()),
        permissions,
        algorithm,
        encrypt_metadata: true,
    };
    let bytes = run_wellfriendpdf(|| wellfriendpdf_engine::encrypt(&engine, &params))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, certificate, private_key, output=None))]
fn pubsec_decrypt_pdf(
    pdf: PathBuf,
    certificate: PathBuf,
    private_key: PathBuf,
    output: Option<PathBuf>,
) -> PyResult<Vec<u8>> {
    let cert =
        std::fs::read(&certificate).map_err(|err| WellfriendError::new_err(err.to_string()))?;
    let key =
        std::fs::read(&private_key).map_err(|err| WellfriendError::new_err(err.to_string()))?;
    let identity =
        run_wellfriendpdf(|| wellfriendpdf_engine::PubSecIdentity::from_bytes(&cert, &key))?;
    let provider = wellfriendpdf_engine::PubSecKeyProvider::single(identity);
    let bytes = std::fs::read(&pdf).map_err(|err| WellfriendError::new_err(err.to_string()))?;
    let engine = run_wellfriendpdf(|| {
        wellfriendpdf_engine::ContentEngine::open_bytes_with_pubsec_provider(bytes, &provider)
    })?;
    let out = run_wellfriendpdf(|| wellfriendpdf_engine::decrypt_pdf(&engine))?;
    write_optional(&output, &out)?;
    Ok(out)
}

#[pyfunction]
#[pyo3(signature = (pdf, pfx, password=None, output=None))]
fn pubsec_decrypt_pdf_pfx(
    pdf: PathBuf,
    pfx: PathBuf,
    password: Option<Vec<u8>>,
    output: Option<PathBuf>,
) -> PyResult<Vec<u8>> {
    let pfx_bytes = std::fs::read(&pfx).map_err(|err| WellfriendError::new_err(err.to_string()))?;
    let password = password.unwrap_or_default();
    let identity = run_wellfriendpdf(|| {
        wellfriendpdf_engine::PubSecIdentity::from_pkcs12_der(&pfx_bytes, &password)
    })?;
    let provider = wellfriendpdf_engine::PubSecKeyProvider::single(identity);
    let bytes = std::fs::read(&pdf).map_err(|err| WellfriendError::new_err(err.to_string()))?;
    let engine = run_wellfriendpdf(|| {
        wellfriendpdf_engine::ContentEngine::open_bytes_with_pubsec_provider(bytes, &provider)
    })?;
    let out = run_wellfriendpdf(|| wellfriendpdf_engine::decrypt_pdf(&engine))?;
    write_optional(&output, &out)?;
    Ok(out)
}

#[pyfunction]
#[pyo3(signature = (pdf, recipient_certificates, output=None, password=None))]
fn pubsec_encrypt_pdf(
    pdf: PathBuf,
    recipient_certificates: Vec<PathBuf>,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let options = pubsec_options_from_paths(&recipient_certificates)?;
    let (out, _) = run_wellfriendpdf(|| {
        wellfriendpdf_engine::encrypt_pdf_pubsec(engine.document().reader(), &options)
    })?;
    write_optional(&output, &out)?;
    Ok(out)
}

#[pyfunction]
#[pyo3(signature = (pdf, recipient_certificates, certificate=None, private_key=None, output=None, password=None))]
fn pubsec_reencrypt_pdf(
    pdf: PathBuf,
    recipient_certificates: Vec<PathBuf>,
    certificate: Option<PathBuf>,
    private_key: Option<PathBuf>,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let options = pubsec_options_from_paths(&recipient_certificates)?;
    let engine = match (certificate, private_key) {
        (Some(certificate), Some(private_key)) => {
            let cert = std::fs::read(&certificate)
                .map_err(|err| WellfriendError::new_err(err.to_string()))?;
            let key = std::fs::read(&private_key)
                .map_err(|err| WellfriendError::new_err(err.to_string()))?;
            let identity = run_wellfriendpdf(|| {
                wellfriendpdf_engine::PubSecIdentity::from_bytes(&cert, &key)
            })?;
            let provider = wellfriendpdf_engine::PubSecKeyProvider::single(identity);
            let bytes =
                std::fs::read(&pdf).map_err(|err| WellfriendError::new_err(err.to_string()))?;
            run_wellfriendpdf(|| {
                wellfriendpdf_engine::ContentEngine::open_bytes_with_pubsec_provider(
                    bytes, &provider,
                )
            })?
        }
        (None, None) => open_engine_path(&pdf, password)?,
        _ => {
            return Err(PyValueError::new_err(
                "pubsec_reencrypt_pdf requires both certificate and private_key, or neither",
            ));
        }
    };
    let (out, _) = run_wellfriendpdf(|| {
        wellfriendpdf_engine::reencrypt_pdf_pubsec(engine.document().reader(), &options)
    })?;
    write_optional(&output, &out)?;
    Ok(out)
}

#[pyfunction]
#[pyo3(signature = (pdf, recipient_certificates, pfx, password=None, output=None))]
fn pubsec_reencrypt_pdf_pfx(
    pdf: PathBuf,
    recipient_certificates: Vec<PathBuf>,
    pfx: PathBuf,
    password: Option<Vec<u8>>,
    output: Option<PathBuf>,
) -> PyResult<Vec<u8>> {
    let options = pubsec_options_from_paths(&recipient_certificates)?;
    let pfx_bytes = std::fs::read(&pfx).map_err(|err| WellfriendError::new_err(err.to_string()))?;
    let password = password.unwrap_or_default();
    let identity = run_wellfriendpdf(|| {
        wellfriendpdf_engine::PubSecIdentity::from_pkcs12_der(&pfx_bytes, &password)
    })?;
    let provider = wellfriendpdf_engine::PubSecKeyProvider::single(identity);
    let bytes = std::fs::read(&pdf).map_err(|err| WellfriendError::new_err(err.to_string()))?;
    let engine = run_wellfriendpdf(|| {
        wellfriendpdf_engine::ContentEngine::open_bytes_with_pubsec_provider(bytes, &provider)
    })?;
    let (out, _) = run_wellfriendpdf(|| {
        wellfriendpdf_engine::reencrypt_pdf_pubsec(engine.document().reader(), &options)
    })?;
    write_optional(&output, &out)?;
    Ok(out)
}

fn pubsec_options_from_paths(
    recipient_certificates: &[PathBuf],
) -> PyResult<wellfriendpdf_engine::PubSecEncryptOptions> {
    if recipient_certificates.is_empty() {
        return Err(PyValueError::new_err(
            "recipient_certificates must contain at least one certificate path",
        ));
    }
    let mut recipients = Vec::with_capacity(recipient_certificates.len());
    for path in recipient_certificates {
        let bytes = std::fs::read(path).map_err(|err| WellfriendError::new_err(err.to_string()))?;
        recipients.push(run_wellfriendpdf(|| {
            wellfriendpdf_engine::PubSecRecipientCertificate::from_bytes(&bytes)
        })?);
    }
    Ok(wellfriendpdf_engine::PubSecEncryptOptions {
        recipients,
        permissions: 0xFFFF_FFFCu32,
        encrypt_metadata: true,
        method: wellfriendpdf_engine::CryptMethod::AesV2,
        recipient_id_mode: wellfriendpdf_engine::PubSecRecipientIdMode::IssuerAndSerial,
    })
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, password=None))]
fn optimize_pdf(
    pdf: PathBuf,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let (bytes, _) = run_wellfriendpdf(|| wellfriendpdf_engine::optimize(&engine))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, password=None))]
fn repair_pdf(pdf: PathBuf, output: Option<PathBuf>, password: Option<&str>) -> PyResult<Vec<u8>> {
    let bytes = std::fs::read(&pdf).map_err(|err| WellfriendError::new_err(err.to_string()))?;
    let password = password.unwrap_or("").as_bytes().to_vec();
    let repaired = run_wellfriendpdf(|| wellfriendpdf_engine::repair(bytes, &password))?;
    write_optional(&output, &repaired)?;
    Ok(repaired)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, password=None))]
fn linearize_pdf(
    pdf: PathBuf,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let bytes = run_wellfriendpdf(|| wellfriendpdf_engine::linearize(&engine))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, out_dir, pages="all", dpi=150, quality=85, format="jpg", password=None))]
#[allow(clippy::too_many_arguments)]
fn pdf_to_images<'py>(
    py: Python<'py>,
    pdf: PathBuf,
    out_dir: PathBuf,
    pages: &str,
    dpi: u32,
    quality: u8,
    format: &str,
    password: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let engine = open_engine_path(&pdf, password)?;
    let total = run_wellfriendpdf(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let format = wellfriendpdf_engine::RasterImageFormat::parse(format)
        .ok_or_else(|| PyValueError::new_err("format must be jpg or png"))?;
    let results = run_wellfriendpdf(|| {
        wellfriendpdf_engine::export_pdf_pages_to_images(
            &engine, &out_dir, &selected, dpi, format, quality, "page",
        )
    })?;
    json_to_py(py, &results)
}

#[pyfunction]
#[pyo3(signature = (images, output=None, page_size="a4", margin=0.0))]
fn images_to_pdf(
    images: Vec<PathBuf>,
    output: Option<PathBuf>,
    page_size: &str,
    margin: f64,
) -> PyResult<Vec<u8>> {
    let page_size = wellfriendpdf_engine::ImagePdfPageSize::parse(page_size)
        .ok_or_else(|| PyValueError::new_err("page_size must be a4, letter, or size-to-image"))?;
    let bytes = run_wellfriendpdf(|| {
        wellfriendpdf_engine::images_to_pdf_from_paths(
            &images,
            wellfriendpdf_engine::ImageToPdfOptions {
                page_size,
                margin_points: margin,
            },
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, layout="pages", password=None))]
fn pdf_to_xlsx(
    pdf: PathBuf,
    output: Option<PathBuf>,
    layout: &str,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let layout = wellfriendpdf_engine::XlsxLayout::parse(layout)
        .ok_or_else(|| PyValueError::new_err("layout must be pages or tables"))?;
    let bytes = run_wellfriendpdf(|| {
        wellfriendpdf_engine::pdf_to_xlsx(&engine, &wellfriendpdf_engine::XlsxOptions { layout })
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, include_images=true, password=None))]
fn pdf_to_pptx(
    pdf: PathBuf,
    output: Option<PathBuf>,
    include_images: bool,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let bytes = run_wellfriendpdf(|| {
        wellfriendpdf_engine::pdf_to_pptx(
            &engine,
            &wellfriendpdf_engine::PptxOptions { include_images },
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, include_images=true, password=None, layout="flowing"))]
fn pdf_to_docx(
    pdf: PathBuf,
    output: Option<PathBuf>,
    include_images: bool,
    password: Option<&str>,
    layout: &str,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let layout = wellfriendpdf_engine::DocxLayout::parse(layout).ok_or_else(|| {
        WellfriendError::new_err("unknown DOCX layout; use flowing, page-faithful, or hybrid")
    })?;
    let bytes = run_wellfriendpdf(|| {
        wellfriendpdf_engine::pdf_to_docx(
            &engine,
            &wellfriendpdf_engine::DocxOptions {
                include_images,
                layout,
            },
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (docx, output=None))]
fn docx_to_pdf(docx: PathBuf, output: Option<PathBuf>) -> PyResult<Vec<u8>> {
    let input = std::fs::read(&docx)?;
    let bytes = run_wellfriendpdf(|| {
        wellfriendpdf_engine::docx_to_pdf(
            &input,
            &wellfriendpdf_engine::OfficeToPdfOptions::default(),
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (xlsx, output=None))]
fn xlsx_to_pdf(xlsx: PathBuf, output: Option<PathBuf>) -> PyResult<Vec<u8>> {
    let input = std::fs::read(&xlsx)?;
    let bytes = run_wellfriendpdf(|| {
        wellfriendpdf_engine::xlsx_to_pdf(
            &input,
            &wellfriendpdf_engine::OfficeToPdfOptions::default(),
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pptx, output=None))]
fn pptx_to_pdf(pptx: PathBuf, output: Option<PathBuf>) -> PyResult<Vec<u8>> {
    let input = std::fs::read(&pptx)?;
    let bytes = run_wellfriendpdf(|| {
        wellfriendpdf_engine::pptx_to_pdf(
            &input,
            &wellfriendpdf_engine::OfficeToPdfOptions::default(),
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (input, format))]
fn prompt22_office_inspect<'py>(
    py: Python<'py>,
    input: PathBuf,
    format: &str,
) -> PyResult<Py<PyAny>> {
    let bytes = std::fs::read(&input)?;
    let json = run_wellfriendpdf(|| sdk::prompt22_office_inspect_json(&bytes, format))?;
    parse_json_str(py, &json)
}

#[pyfunction]
#[pyo3(signature = (input, format, output=None))]
fn prompt22_office_to_pdf<'py>(
    py: Python<'py>,
    input: PathBuf,
    format: &str,
    output: Option<PathBuf>,
) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
    let bytes = std::fs::read(&input)?;
    let (out, report) = run_wellfriendpdf(|| sdk::prompt22_office_to_pdf_json(&bytes, format))?;
    write_optional(&output, &out)?;
    Ok((
        PyBytes::new(py, &out).unbind(),
        parse_json_str(py, &report)?,
    ))
}

#[pyfunction]
#[pyo3(signature = (pdf, text=None, image=None, output=None, pages="all", position="center", opacity=0.28, rotation=45.0, font_size=64.0, color="#8c8c8c", scale=0.5, password=None))]
#[allow(clippy::too_many_arguments)]
fn watermark_pdf(
    pdf: PathBuf,
    text: Option<String>,
    image: Option<PathBuf>,
    output: Option<PathBuf>,
    pages: &str,
    position: &str,
    opacity: f64,
    rotation: f64,
    font_size: f64,
    color: &str,
    scale: f64,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    if text.is_some() == image.is_some() {
        return Err(PyValueError::new_err("pass exactly one of text or image"));
    }
    let input = read_edit_input_py(&pdf, password)?;
    let engine = run_wellfriendpdf(|| ContentEngine::open_bytes(input.clone()))?;
    let total = run_wellfriendpdf(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let position = parse_stamp_position_py(position)?;
    let bytes = if let Some(text) = text {
        let color = parse_rgb_color_py(color)?;
        run_wellfriendpdf(|| {
            wellfriendpdf_engine::watermark_text_pdf(
                input,
                &text,
                wellfriendpdf_engine::TextWatermarkOptions {
                    pages: selected,
                    position,
                    opacity,
                    rotation_degrees: rotation,
                    font_size,
                    color,
                },
            )
        })?
    } else {
        let image_path = image.expect("checked above");
        let image =
            std::fs::read(&image_path).map_err(|err| WellfriendError::new_err(err.to_string()))?;
        run_wellfriendpdf(|| {
            wellfriendpdf_engine::watermark_image_pdf(
                input,
                &image,
                image_path.extension().and_then(|s| s.to_str()),
                wellfriendpdf_engine::ImageWatermarkOptions {
                    pages: selected,
                    position,
                    opacity,
                    scale,
                },
            )
        })?
    };
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, output=None, pages="all", position="bottom-center", format="Page {n} of {total}", start=1, font_size=10.0, color="#333333", password=None))]
#[allow(clippy::too_many_arguments)]
fn add_page_numbers(
    pdf: PathBuf,
    output: Option<PathBuf>,
    pages: &str,
    position: &str,
    format: &str,
    start: isize,
    font_size: f64,
    color: &str,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let input = read_edit_input_py(&pdf, password)?;
    let engine = run_wellfriendpdf(|| ContentEngine::open_bytes(input.clone()))?;
    let total = run_wellfriendpdf(|| engine.page_count())?;
    let selected = parse_pages_spec_py(pages, total)?;
    let position = parse_stamp_position_py(position)?;
    let color = parse_rgb_color_py(color)?;
    let bytes = run_wellfriendpdf(|| {
        wellfriendpdf_engine::add_page_numbers_pdf(
            input,
            wellfriendpdf_engine::PageNumberOptions {
                pages: selected,
                position,
                format: format.to_string(),
                start,
                font_size,
                color,
            },
        )
    })?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, order="all", output=None, password=None))]
fn organize_pdf(
    pdf: PathBuf,
    order: &str,
    output: Option<PathBuf>,
    password: Option<&str>,
) -> PyResult<Vec<u8>> {
    let engine = open_engine_path(&pdf, password)?;
    let total = run_wellfriendpdf(|| engine.page_count())?;
    let selected = parse_pages_spec_py(order, total)?;
    let bytes = run_wellfriendpdf(|| wellfriendpdf_engine::organize_pdf(&engine, &selected))?;
    write_optional(&output, &bytes)?;
    Ok(bytes)
}

#[pyfunction]
#[pyo3(signature = (pdf, password=None))]
fn fonts<'py>(py: Python<'py>, pdf: PathBuf, password: Option<&str>) -> PyResult<Py<PyAny>> {
    let engine = open_engine_path(&pdf, password)?;
    let fonts = run_wellfriendpdf(|| engine.list_fonts())?;
    json_to_py(py, &fonts)
}

#[pyfunction]
#[pyo3(signature = (pdf, password=None))]
fn verify_signatures<'py>(
    py: Python<'py>,
    pdf: PathBuf,
    password: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let engine = open_engine_path(&pdf, password)?;
    let sigs = run_wellfriendpdf(|| engine.verify_signatures())?;
    json_to_py(py, &sigs)
}

#[pyfunction]
#[pyo3(signature = (pdf, options_json, password=None))]
fn verify_signatures_with_options<'py>(
    py: Python<'py>,
    pdf: PathBuf,
    options_json: &str,
    password: Option<&str>,
) -> PyResult<Py<PyAny>> {
    let bytes = std::fs::read(&pdf)?;
    let password_bytes = password.map(str::as_bytes);
    let json = run_wellfriendpdf(|| {
        sdk::signature_report_with_options_json(&bytes, options_json, password_bytes)
    })?;
    parse_json_str(py, &json)
}

/// Feature / capability report: SDK version, envelope version, and which
/// optional engine capabilities are compiled into this build. No document input.
#[pyfunction]
fn feature_report(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let json = run_wellfriendpdf(sdk::feature_report_json)?;
    parse_json_str(py, &json)
}

/// Prompt 26 append-only incremental signing. `key_pem`/`cert_pem` are the
/// signer material and are never logged. `certify` (1|2|3) creates a DocMDP
/// certification signature; otherwise an approval signature is produced. The
/// signed PDF is reopened and validated before it is returned; a signature that
/// fails post-sign validation raises instead of returning a "signed" file.
#[pyfunction]
#[pyo3(signature = (pdf, key_pem, cert_pem, output=None, chain_pem=None, placeholder_size=16384, certify=None, field_name=None, reason=None, password=None))]
#[allow(clippy::too_many_arguments)]
fn sign_pdf<'py>(
    py: Python<'py>,
    pdf: PathBuf,
    key_pem: &str,
    cert_pem: &str,
    output: Option<PathBuf>,
    chain_pem: Option<Vec<String>>,
    placeholder_size: usize,
    certify: Option<u8>,
    field_name: Option<String>,
    reason: Option<String>,
    password: Option<&str>,
) -> PyResult<(Py<PyBytes>, Py<PyAny>)> {
    let engine = open_engine_path(&pdf, password)?;
    let chain = chain_pem.unwrap_or_default();
    let chain_refs: Vec<&str> = chain.iter().map(String::as_str).collect();
    let signer = run_wellfriendpdf(|| {
        wellfriendpdf_engine::PdfSigner::from_pem(key_pem, cert_pem, &chain_refs)
    })?;
    let mut signature = wellfriendpdf_engine::SignatureOptions {
        contents_reserved_bytes: placeholder_size,
        ..Default::default()
    };
    if let Some(field) = field_name {
        signature.field_name = field;
    }
    signature.reason = reason;
    let intent = match certify {
        Some(p) => wellfriendpdf_engine::SigningIntent::Certification {
            docmdp_permissions: p,
        },
        None => wellfriendpdf_engine::SigningIntent::Approval,
    };
    let options = wellfriendpdf_engine::IncrementalSigningOptions {
        signature,
        intent,
        retry_larger_placeholder: true,
        max_placeholder_bytes: 256 * 1024,
    };
    let result = run_wellfriendpdf(|| {
        wellfriendpdf_engine::sign_incremental(
            engine.document(),
            wellfriendpdf_engine::IncrementalSigner::Local(&signer),
            &options,
        )
    })?;
    if !result.post_sign.signature_valid {
        return Err(WellfriendError::new_err(
            "post-sign validation failed; signed output not returned",
        ));
    }
    write_optional(&output, &result.signed_pdf)?;
    let signed = PyBytes::new(py, &result.signed_pdf).unbind();
    let report_json = serde_json::to_string(&result)
        .map_err(|err| WellfriendError::new_err(format!("JSON serialization error: {err}")))?;
    Ok((signed, parse_json_str(py, &report_json)?))
}

/// Prompt 21 persistent history store report. No document input.
#[pyfunction]
fn prompt21_history_report(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let json = run_wellfriendpdf(sdk::prompt21_history_report_json)?;
    parse_json_str(py, &json)
}

/// Prompt 23 AES-GCM/PubSec tamper policy report. No secret material is accepted.
#[pyfunction]
fn crypto_tamper_test(py: Python<'_>) -> PyResult<Py<PyAny>> {
    let json = run_wellfriendpdf(sdk::crypto_tamper_test_json)?;
    parse_json_str(py, &json)
}

/// Decode budget report for a hypothetical image stream: shows the decode-limit
/// / decompression-bomb policy that decoding a `filter`/`width`/`height`/
/// `components` image would trigger. No document input.
#[pyfunction]
#[pyo3(signature = (filter, width, height, components=3))]
fn decode_budget_report(
    py: Python<'_>,
    filter: &str,
    width: u32,
    height: u32,
    components: u8,
) -> PyResult<Py<PyAny>> {
    let filter = filter.to_string();
    let json =
        run_wellfriendpdf(|| sdk::decode_budget_report_json(&filter, width, height, components))?;
    parse_json_str(py, &json)
}

/// Codec isolation diagnostic report over caller-supplied encoded stream bytes.
/// `policy` is "in_process", "isolated_preferred", "isolated_required",
/// "report_only", or "disabled".
#[pyfunction]
#[pyo3(signature = (filter, data, policy="in_process"))]
fn codec_isolation_report(
    py: Python<'_>,
    filter: &str,
    data: Vec<u8>,
    policy: &str,
) -> PyResult<Py<PyAny>> {
    let filter = filter.to_string();
    let policy = policy.to_string();
    let json =
        run_wellfriendpdf(|| sdk::codec_isolation_report_json(&filter, &data, Some(&policy)))?;
    parse_json_str(py, &json)
}

/// Resource-dedup report over caller-supplied resource byte buffers. Groups
/// byte-identical resources by content digest (the deterministic-writer dedup
/// evidence). Pass a list of `bytes`.
#[pyfunction]
fn resource_dedup_report(py: Python<'_>, resources: Vec<Vec<u8>>) -> PyResult<Py<PyAny>> {
    let json = run_wellfriendpdf(|| sdk::resource_dedup_report_json(&resources))?;
    parse_json_str(py, &json)
}

#[pymodule]
fn wellfriendpdf(py: Python<'_>, module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("WellfriendError", py.get_type::<WellfriendError>())?;
    module.add_class::<PyDocument>()?;
    module.add_class::<PySignatureTrustStore>()?;
    module.add_class::<PySignatureIntermediateStore>()?;
    module.add_class::<PySignatureEvidenceStore>()?;
    module.add_class::<PySignatureRetrievalPolicy>()?;
    module.add_class::<PySignatureValidationCancellation>()?;
    module.add_class::<PySignatureValidationOptions>()?;
    module.add_class::<PyPage>()?;
    module.add_class::<PyRegionPage>()?;
    module.add_function(wrap_pyfunction!(open, module)?)?;
    module.add_function(wrap_pyfunction!(timestamp_token_validation, module)?)?;
    module.add_function(wrap_pyfunction!(merge_pdfs, module)?)?;
    module.add_function(wrap_pyfunction!(extract_pages, module)?)?;
    module.add_function(wrap_pyfunction!(rotate_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(decrypt_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(encrypt_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pubsec_decrypt_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pubsec_decrypt_pdf_pfx, module)?)?;
    module.add_function(wrap_pyfunction!(pubsec_encrypt_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pubsec_reencrypt_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pubsec_reencrypt_pdf_pfx, module)?)?;
    module.add_function(wrap_pyfunction!(optimize_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(repair_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(linearize_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pdf_to_images, module)?)?;
    module.add_function(wrap_pyfunction!(images_to_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pdf_to_xlsx, module)?)?;
    module.add_function(wrap_pyfunction!(pdf_to_pptx, module)?)?;
    module.add_function(wrap_pyfunction!(pdf_to_docx, module)?)?;
    module.add_function(wrap_pyfunction!(docx_to_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(xlsx_to_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(pptx_to_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(prompt22_office_inspect, module)?)?;
    module.add_function(wrap_pyfunction!(prompt22_office_to_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(watermark_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(add_page_numbers, module)?)?;
    module.add_function(wrap_pyfunction!(organize_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(fonts, module)?)?;
    module.add_function(wrap_pyfunction!(verify_signatures, module)?)?;
    module.add_function(wrap_pyfunction!(verify_signatures_with_options, module)?)?;
    module.add_function(wrap_pyfunction!(feature_report, module)?)?;
    module.add_function(wrap_pyfunction!(sign_pdf, module)?)?;
    module.add_function(wrap_pyfunction!(prompt21_history_report, module)?)?;
    module.add_function(wrap_pyfunction!(crypto_tamper_test, module)?)?;
    module.add_function(wrap_pyfunction!(decode_budget_report, module)?)?;
    module.add_function(wrap_pyfunction!(codec_isolation_report, module)?)?;
    module.add_function(wrap_pyfunction!(resource_dedup_report, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add(
        "__report_envelope_version__",
        wellfriendpdf_engine::REPORT_ENVELOPE_VERSION,
    )?;
    Ok(())
}

fn open_impl(source: &Bound<'_, PyAny>, password: Option<&str>) -> PyResult<PyDocument> {
    if let Ok(data) = source.extract::<Vec<u8>>() {
        let engine = if let Some(password) = password {
            run_wellfriendpdf(|| {
                ContentEngine::open_bytes_with_password(data, password.as_bytes())
            })?
        } else {
            run_wellfriendpdf(|| ContentEngine::open_bytes(data))?
        };
        return Ok(PyDocument {
            engine: Arc::new(engine),
        });
    }

    let py = source.py();
    let os = py.import("os")?;
    let path_obj = os
        .call_method1("fspath", (source,))
        .map_err(|_| PyTypeError::new_err("source must be a filesystem path or bytes"))?;
    let path: PathBuf = path_obj.extract()?;
    let engine = if let Some(password) = password {
        run_wellfriendpdf(|| ContentEngine::open_path_with_password(path, password.as_bytes()))?
    } else {
        run_wellfriendpdf(|| ContentEngine::open_path(path))?
    };
    Ok(PyDocument {
        engine: Arc::new(engine),
    })
}

fn open_engine_path(path: &PathBuf, password: Option<&str>) -> PyResult<ContentEngine> {
    if let Some(password) = password {
        run_wellfriendpdf(|| ContentEngine::open_path_with_password(path, password.as_bytes()))
    } else {
        run_wellfriendpdf(|| ContentEngine::open_path(path))
    }
}

fn write_optional(path: &Option<PathBuf>, bytes: &[u8]) -> PyResult<()> {
    if let Some(path) = path {
        std::fs::write(path, bytes).map_err(|err| WellfriendError::new_err(err.to_string()))?;
    }
    Ok(())
}

fn read_edit_input_py(path: &PathBuf, password: Option<&str>) -> PyResult<Vec<u8>> {
    if password.is_some() {
        let engine = open_engine_path(path, password)?;
        run_wellfriendpdf(|| wellfriendpdf_engine::decrypt_pdf(&engine))
    } else {
        std::fs::read(path).map_err(|err| WellfriendError::new_err(err.to_string()))
    }
}

fn parse_pages_spec_py(spec: &str, total: usize) -> PyResult<Vec<usize>> {
    if total == 0 {
        return Err(PyValueError::new_err("document has no pages"));
    }
    let spec = spec.trim();
    if spec.is_empty() || spec.eq_ignore_ascii_case("all") {
        return Ok((1..=total).collect());
    }
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            let start: usize = start
                .trim()
                .parse()
                .map_err(|_| PyValueError::new_err(format!("invalid page '{part}'")))?;
            let end: usize = end
                .trim()
                .parse()
                .map_err(|_| PyValueError::new_err(format!("invalid page '{part}'")))?;
            if start == 0 || end == 0 || start > end {
                return Err(PyValueError::new_err(format!(
                    "invalid page range '{part}'"
                )));
            }
            for page in start..=end.min(total) {
                out.push(page);
            }
        } else {
            let page: usize = part
                .parse()
                .map_err(|_| PyValueError::new_err(format!("invalid page '{part}'")))?;
            if page == 0 || page > total {
                return Err(PyValueError::new_err(format!(
                    "page {page} out of range 1..={total}"
                )));
            }
            out.push(page);
        }
    }
    if out.is_empty() {
        return Err(PyValueError::new_err("page selection matched no pages"));
    }
    Ok(out)
}

fn parse_stamp_position_py(value: &str) -> PyResult<wellfriendpdf_engine::StampPosition> {
    wellfriendpdf_engine::StampPosition::parse(value)
        .ok_or_else(|| PyValueError::new_err(format!("unknown position '{value}'")))
}

fn parse_rgb_color_py(value: &str) -> PyResult<wellfriendpdf_engine::RgbColor> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return Err(PyValueError::new_err(format!(
            "color '{value}' must be #RRGGBB"
        )));
    }
    let r = u8::from_str_radix(&hex[0..2], 16)
        .map_err(|_| PyValueError::new_err(format!("color '{value}' must be #RRGGBB")))?;
    let g = u8::from_str_radix(&hex[2..4], 16)
        .map_err(|_| PyValueError::new_err(format!("color '{value}' must be #RRGGBB")))?;
    let b = u8::from_str_radix(&hex[4..6], 16)
        .map_err(|_| PyValueError::new_err(format!("color '{value}' must be #RRGGBB")))?;
    Ok(wellfriendpdf_engine::RgbColor {
        r: f64::from(r) / 255.0,
        g: f64::from(g) / 255.0,
        b: f64::from(b) / 255.0,
    })
}

fn run_wellfriendpdf<T, F>(operation: F) -> PyResult<T>
where
    F: FnOnce() -> wellfriendpdf_engine::Result<T>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(err)) => Err(WellfriendError::new_err(err.to_string())),
        Err(_) => Err(WellfriendError::new_err("Rust panic while processing PDF")),
    }
}

fn read_signature_component_file(path: &PathBuf) -> PyResult<Vec<u8>> {
    std::fs::read(path).map_err(|error| {
        PyValueError::new_err(format!(
            "signature validation file '{}': {error}",
            path.display()
        ))
    })
}

fn looks_like_pem(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        == Some(b'-')
}

fn validate_page(engine: &ContentEngine, page: usize) -> PyResult<()> {
    let total = run_wellfriendpdf(|| engine.page_count())?;
    if page == 0 || page > total {
        Err(PyIndexError::new_err(format!(
            "page {page} out of range 1..={total}"
        )))
    } else {
        Ok(())
    }
}

fn parse_profile_py(name: &str) -> PyResult<ExtractionProfile> {
    ExtractionProfile::parse(name).ok_or_else(|| {
        PyValueError::new_err(
            "profile must be fast-text, layout-faithful, tables-focused, or rag-chunks",
        )
    })
}

/// Build [`ParseOptions`] with an optional Python OCR backend wired in. When
/// `ocr` is `None`, returns the default options (OCR off — scanned pages degrade
/// to the placeholder, matching the pre-OCR behavior). When `ocr` is a Python
/// object, wraps it as a [`PyOcrEngine`] and enables the `Auto` policy so scanned
/// pages are recognized through it. `ocr_lang` is split on `+`/`,` (falling back
/// to `eng`); `ocr_dpi` sets the rasterization DPI (~300 is the sweet spot).
fn parse_options_with_ocr(
    ocr: Option<&Bound<'_, PyAny>>,
    ocr_lang: &str,
    ocr_dpi: u32,
) -> PyResult<ParseOptions> {
    let mut options = ParseOptions::default();
    let Some(obj) = ocr else {
        return Ok(options);
    };
    let backend = PyOcrEngine::new(obj)?;
    let langs: Vec<String> = ocr_lang
        .split(['+', ','])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    options.ocr = Some(Arc::new(backend));
    options.ocr_policy = OcrPolicy::Auto;
    options.ocr_options = wellfriendpdf_engine::OcrOptions {
        languages: if langs.is_empty() {
            vec!["eng".to_string()]
        } else {
            langs
        },
        dpi: ocr_dpi.max(1),
        psm: None,
    };
    options.ocr_dpi = ocr_dpi.max(1);
    Ok(options)
}

fn all_text_with_profile(engine: &ContentEngine, profile: ExtractionProfile) -> PyResult<String> {
    let total = run_wellfriendpdf(|| engine.page_count())?;
    let mut pages = Vec::new();
    for page in 1..=total {
        pages.push(run_wellfriendpdf(|| {
            engine.get_page_text_with_profile(page, profile)
        })?);
    }
    Ok(pages.join("\n"))
}

fn json_to_py<'py, T: Serialize>(py: Python<'py>, value: &T) -> PyResult<Py<PyAny>> {
    let raw = serde_json::to_string(value)
        .map_err(|err| WellfriendError::new_err(format!("JSON serialization error: {err}")))?;
    parse_json_str(py, &raw)
}

/// Parse a JSON string (an SDK-facade envelope) into a native Python object.
fn parse_json_str<'py>(py: Python<'py>, raw: &str) -> PyResult<Py<PyAny>> {
    let json = py.import("json")?;
    Ok(json.call_method1("loads", (raw,))?.unbind())
}

fn page_words<'py>(py: Python<'py>, engine: &ContentEngine, page: usize) -> PyResult<Py<PyAny>> {
    let words = run_wellfriendpdf(|| engine.extract_page_words(page))?;
    json_to_py(py, &words)
}

fn page_images<'py>(py: Python<'py>, engine: &ContentEngine, page: usize) -> PyResult<Py<PyAny>> {
    let options = ImageLocateOptions {
        pages: Some(vec![page]),
        ..Default::default()
    };
    let images = run_wellfriendpdf(|| engine.find_all_images(&options))?;
    let list = PyList::empty(py);
    for image in images {
        let dict = PyDict::new(py);
        dict.set_item("page", image.page_number)?;
        dict.set_item("name", image.xobject_name.clone())?;
        dict.set_item("width", image.width)?;
        dict.set_item("height", image.height)?;
        dict.set_item("bits_per_component", image.bits_per_component)?;
        dict.set_item("color_space", image.color_space.clone())?;
        dict.set_item("filters", image.filter.clone())?;
        dict.set_item("inline", image.is_inline)?;
        dict.set_item("mask", image.is_mask)?;
        dict.set_item("soft_mask", image.is_smask)?;
        match run_wellfriendpdf(|| engine.extract_image_bytes(&image, ImageOutputFormat::Png, None))
        {
            Ok(bytes) => {
                dict.set_item("format", "png")?;
                dict.set_item("data", PyBytes::new(py, &bytes))?;
            }
            Err(err) => {
                dict.set_item("format", py.None())?;
                dict.set_item("data", py.None())?;
                dict.set_item("error", err.to_string())?;
            }
        }
        list.append(dict)?;
    }
    Ok(list.into_any().unbind())
}

fn page_region_images<'py>(
    py: Python<'py>,
    engine: &ContentEngine,
    page: usize,
    region: PageRegion,
) -> PyResult<Py<PyAny>> {
    let images = run_wellfriendpdf(|| engine.find_page_images_in_region(page, region))?;
    let list = PyList::empty(py);
    for placed in images {
        let image = placed.image;
        let dict = PyDict::new(py);
        dict.set_item("page", image.page_number)?;
        dict.set_item("name", image.xobject_name.clone())?;
        dict.set_item("bbox", placed.bbox.to_vec())?;
        dict.set_item("width", image.width)?;
        dict.set_item("height", image.height)?;
        dict.set_item("bits_per_component", image.bits_per_component)?;
        dict.set_item("color_space", image.color_space.clone())?;
        dict.set_item("filters", image.filter.clone())?;
        dict.set_item("inline", image.is_inline)?;
        dict.set_item("mask", image.is_mask)?;
        dict.set_item("soft_mask", image.is_smask)?;
        match run_wellfriendpdf(|| engine.extract_image_bytes(&image, ImageOutputFormat::Png, None))
        {
            Ok(bytes) => {
                dict.set_item("format", "png")?;
                dict.set_item("data", PyBytes::new(py, &bytes))?;
            }
            Err(err) => {
                dict.set_item("format", py.None())?;
                dict.set_item("data", py.None())?;
                dict.set_item("error", err.to_string())?;
            }
        }
        list.append(dict)?;
    }
    Ok(list.into_any().unbind())
}
