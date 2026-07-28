//! wasm-bindgen wrapper for `wellfriendpdf-engine`.
//!
//! The browser/Node/WebWorker surface accepts caller-provided bytes only. It
//! does not fetch URLs, read host files implicitly, or execute PDF active
//! content. Reports are routed through `wellfriendpdf_engine::sdk` so the JSON envelope
//! matches Rust, Python, and the C ABI.

#[cfg(target_arch = "wasm32")]
mod wasm_api {
    use wasm_bindgen::prelude::*;

    use wellfriendpdf_engine::{
        sdk, CancelToken, ChunkOptions, ContentEngine, DocType, EvidenceBundle, ExtractOptions,
        IncrementalSigner, IncrementalSigningOptions, IntermediateStore, NetworkBudget,
        ParseOptions, PdfSigner, RetrievalPolicy, SignatureOptions, SignatureRevocationMode,
        SigningIntent, TrustStore, VerifyOptions,
    };

    #[wasm_bindgen]
    pub struct SignatureTrustStore {
        store: TrustStore,
        distrusted_certificate_sha256: Vec<String>,
    }

    #[wasm_bindgen]
    impl SignatureTrustStore {
        #[wasm_bindgen(constructor)]
        pub fn new() -> SignatureTrustStore {
            SignatureTrustStore {
                store: TrustStore::new(),
                distrusted_certificate_sha256: Vec::new(),
            }
        }

        #[wasm_bindgen(js_name = addAnchorDer)]
        pub fn add_anchor_der(
            &mut self,
            der: &[u8],
            origin: Option<String>,
            purpose: Option<String>,
        ) -> Result<(), JsValue> {
            self.store
                .add_der(der, origin.unwrap_or_else(|| "wasm".to_string()), purpose)
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = addDistrustedCertificateSha256)]
        pub fn add_distrusted_certificate_sha256(
            &mut self,
            fingerprint: &str,
        ) -> Result<(), JsValue> {
            let normalized = VerifyOptions::default()
                .with_distrusted_certificate_sha256(fingerprint)
                .map_err(js_err)?
                .distrusted_certificate_sha256
                .into_iter()
                .next()
                .ok_or_else(|| JsValue::from_str("empty certificate fingerprint"))?;
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

        #[wasm_bindgen(js_name = anchorCount)]
        pub fn anchor_count(&self) -> usize {
            self.store.anchors().len()
        }
    }

    #[wasm_bindgen]
    pub struct SignatureIntermediateStore {
        store: IntermediateStore,
    }

    #[wasm_bindgen]
    impl SignatureIntermediateStore {
        #[wasm_bindgen(constructor)]
        pub fn new() -> SignatureIntermediateStore {
            SignatureIntermediateStore {
                store: IntermediateStore::new(),
            }
        }

        #[wasm_bindgen(js_name = addDer)]
        pub fn add_der(&mut self, der: &[u8]) -> Result<(), JsValue> {
            self.store.add_der(der).map_err(js_err)
        }

        #[wasm_bindgen(js_name = certificateCount)]
        pub fn certificate_count(&self) -> usize {
            self.store.certificates_der().len()
        }
    }

    #[wasm_bindgen]
    pub struct SignatureEvidenceStore {
        ocsp_responses_der: Vec<Vec<u8>>,
        crls_der: Vec<Vec<u8>>,
        bundle: Option<EvidenceBundle>,
    }

    #[wasm_bindgen]
    impl SignatureEvidenceStore {
        #[wasm_bindgen(constructor)]
        pub fn new() -> SignatureEvidenceStore {
            SignatureEvidenceStore {
                ocsp_responses_der: Vec::new(),
                crls_der: Vec::new(),
                bundle: None,
            }
        }

        #[wasm_bindgen(js_name = addOcspResponseDer)]
        pub fn add_ocsp_response_der(&mut self, der: &[u8]) {
            self.ocsp_responses_der.push(der.to_vec());
        }

        #[wasm_bindgen(js_name = addCrlDer)]
        pub fn add_crl_der(&mut self, der: &[u8]) {
            self.crls_der.push(der.to_vec());
        }

        #[wasm_bindgen(js_name = importBundleJson)]
        pub fn import_bundle_json(&mut self, bundle_json: &str) -> Result<(), JsValue> {
            let bundle: EvidenceBundle = serde_json::from_str(bundle_json)
                .map_err(|error| JsValue::from_str(&format!("evidence bundle JSON: {error}")))?;
            let budget = NetworkBudget::default();
            bundle
                .validate(budget.max_cache_entries, budget.max_cache_bytes)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            self.bundle = Some(bundle);
            Ok(())
        }

        #[wasm_bindgen(js_name = bundleJson)]
        pub fn bundle_json(&self) -> Result<Option<String>, JsValue> {
            self.bundle
                .as_ref()
                .map(|bundle| {
                    serde_json::to_string(bundle).map_err(|error| {
                        JsValue::from_str(&format!("evidence bundle JSON: {error}"))
                    })
                })
                .transpose()
        }

        #[wasm_bindgen(js_name = ocspCount)]
        pub fn ocsp_count(&self) -> usize {
            self.ocsp_responses_der.len()
        }

        #[wasm_bindgen(js_name = crlCount)]
        pub fn crl_count(&self) -> usize {
            self.crls_der.len()
        }
    }

    #[wasm_bindgen]
    pub struct SignatureRetrievalPolicy {
        policy: RetrievalPolicy,
    }

    #[wasm_bindgen]
    impl SignatureRetrievalPolicy {
        #[wasm_bindgen(constructor)]
        pub fn new() -> SignatureRetrievalPolicy {
            SignatureRetrievalPolicy {
                policy: RetrievalPolicy::offline(),
            }
        }

        #[wasm_bindgen(js_name = setJson)]
        pub fn set_json(&mut self, policy_json: &str) -> Result<(), JsValue> {
            let policy: RetrievalPolicy = serde_json::from_str(policy_json)
                .map_err(|error| JsValue::from_str(&format!("retrieval policy JSON: {error}")))?;
            policy
                .validate()
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            self.policy = policy;
            Ok(())
        }

        #[wasm_bindgen(js_name = toJson)]
        pub fn to_json(&self) -> Result<String, JsValue> {
            serde_json::to_string(&self.policy)
                .map_err(|error| JsValue::from_str(&format!("retrieval policy JSON: {error}")))
        }
    }

    #[wasm_bindgen]
    pub struct SignatureValidationCancellation {
        token: CancelToken,
    }

    #[wasm_bindgen]
    impl SignatureValidationCancellation {
        #[wasm_bindgen(constructor)]
        pub fn new() -> SignatureValidationCancellation {
            SignatureValidationCancellation {
                token: CancelToken::new(),
            }
        }

        pub fn cancel(&self) {
            self.token.cancel();
        }

        #[wasm_bindgen(js_name = isCancelled)]
        pub fn is_cancelled(&self) -> bool {
            self.token.is_cancelled()
        }
    }

    /// Owned offline Prompt 24 validation options for the WASM surface.
    ///
    /// WASM accepts caller-supplied trust anchors, intermediates, and
    /// revocation evidence, but has no native network transport.  Enabling a
    /// retrieval policy returns an exact unsupported error instead of allowing
    /// implicit browser networking or relying on ambient platform trust.
    #[wasm_bindgen]
    pub struct SignatureValidationOptions {
        options: VerifyOptions,
    }

    #[wasm_bindgen]
    impl SignatureValidationOptions {
        #[wasm_bindgen(constructor)]
        pub fn new() -> SignatureValidationOptions {
            SignatureValidationOptions {
                options: VerifyOptions::default(),
            }
        }

        #[wasm_bindgen(js_name = addTrustAnchorDer)]
        pub fn add_trust_anchor_der(&mut self, der: &[u8]) {
            self.options.trust_anchors_der.push(der.to_vec());
        }

        #[wasm_bindgen(js_name = addIntermediateDer)]
        pub fn add_intermediate_der(&mut self, der: &[u8]) {
            self.options.intermediates_der.push(der.to_vec());
        }

        #[wasm_bindgen(js_name = addDistrustedCertificateSha256)]
        pub fn add_distrusted_certificate_sha256(
            &mut self,
            fingerprint: &str,
        ) -> Result<(), JsValue> {
            self.options = self
                .options
                .clone()
                .with_distrusted_certificate_sha256(fingerprint)
                .map_err(js_err)?;
            Ok(())
        }

        #[wasm_bindgen(js_name = addOcspResponseDer)]
        pub fn add_ocsp_response_der(&mut self, der: &[u8]) {
            self.options.ocsp_responses_der.push(der.to_vec());
        }

        #[wasm_bindgen(js_name = addCrlDer)]
        pub fn add_crl_der(&mut self, der: &[u8]) {
            self.options.crls_der.push(der.to_vec());
        }

        #[wasm_bindgen(js_name = setValidationTimeUnix)]
        pub fn set_validation_time_unix(&mut self, unix: f64) -> Result<(), JsValue> {
            if !unix.is_finite() || unix < 0.0 || unix.fract() != 0.0 || unix > u64::MAX as f64 {
                return Err(JsValue::from_str(
                    "validation time must be a non-negative integral Unix second",
                ));
            }
            self.options.validation_time_unix = Some(unix as u64);
            Ok(())
        }

        #[wasm_bindgen(js_name = useSystemValidationTime)]
        pub fn use_system_validation_time(&mut self) {
            self.options.validation_time_unix = None;
        }

        #[wasm_bindgen(js_name = setRevocationMode)]
        pub fn set_revocation_mode(&mut self, mode: &str) -> Result<(), JsValue> {
            self.options.revocation_mode = match mode {
                "not_checked" | "not-checked" | "disabled" => SignatureRevocationMode::NotChecked,
                "offline_strict"
                | "offline-strict"
                | "offline_supplied_only"
                | "offline-supplied-only"
                | "require_any_fresh_evidence"
                | "require-any-fresh-evidence" => SignatureRevocationMode::OfflineStrict,
            "offline_best_effort"
                | "offline-best-effort" => SignatureRevocationMode::OfflineBestEffort,
                "online_strict"
                | "online-strict"
                | "online_hard_fail"
                | "online-hard-fail"
                | "online_best_effort"
                | "online-best-effort"
                | "online_best_evidence"
                | "online-best-evidence"
                | "soft_fail_network"
                | "soft-fail-network" => {
                    return Err(JsValue::from_str(
                        "online revocation modes are unsupported in WASM without an explicit host transport",
                    ))
                }
                _ => {
                    return Err(JsValue::from_str(&format!(
                        "unknown signature revocation mode '{mode}'"
                    )))
                }
            };
            Ok(())
        }

        #[wasm_bindgen(js_name = setPathLimits)]
        pub fn set_path_limits(
            &mut self,
            max_chain_depth: usize,
            max_path_candidates: usize,
        ) -> Result<(), JsValue> {
            if max_chain_depth == 0 || max_path_candidates == 0 {
                return Err(JsValue::from_str(
                    "max_chain_depth and max_path_candidates must both be positive",
                ));
            }
            self.options.max_chain_depth = max_chain_depth;
            self.options.max_path_candidates = max_path_candidates;
            Ok(())
        }

        #[wasm_bindgen(js_name = setAlgorithmPolicyJson)]
        pub fn set_algorithm_policy_json(&mut self, policy_json: &str) -> Result<(), JsValue> {
            let policy: wellfriendpdf_engine::SignatureAlgorithmPolicy =
                serde_json::from_str(policy_json).map_err(|error| {
                    JsValue::from_str(&format!("algorithm policy JSON: {error}"))
                })?;
            self.options = self
                .options
                .clone()
                .with_algorithm_policy(policy)
                .map_err(js_err)?;
            Ok(())
        }

        #[wasm_bindgen(js_name = setRetrievalPolicyJson)]
        pub fn set_retrieval_policy_json(&mut self, policy_json: &str) -> Result<(), JsValue> {
            let policy: RetrievalPolicy = serde_json::from_str(policy_json)
                .map_err(|error| JsValue::from_str(&format!("retrieval policy JSON: {error}")))?;
            if policy.enabled {
                return Err(JsValue::from_str(
                    "online retrieval is unsupported in WASM without an explicit host transport",
                ));
            }
            self.options = self
                .options
                .clone()
                .with_retrieval_policy(policy)
                .map_err(js_err)?;
            Ok(())
        }

        #[wasm_bindgen(js_name = setEvidenceBundleJson)]
        pub fn set_evidence_bundle_json(&mut self, bundle_json: &str) -> Result<(), JsValue> {
            let bundle: EvidenceBundle = serde_json::from_str(bundle_json)
                .map_err(|error| JsValue::from_str(&format!("evidence bundle JSON: {error}")))?;
            self.options = self
                .options
                .clone()
                .with_evidence_bundle(bundle)
                .map_err(js_err)?;
            Ok(())
        }

        #[wasm_bindgen(js_name = applyTrustStore)]
        pub fn apply_trust_store(&mut self, store: &SignatureTrustStore) -> Result<(), JsValue> {
            let mut options = self.options.clone().with_trust_store(&store.store);
            for fingerprint in &store.distrusted_certificate_sha256 {
                options = options
                    .with_distrusted_certificate_sha256(fingerprint)
                    .map_err(js_err)?;
            }
            self.options = options;
            Ok(())
        }

        #[wasm_bindgen(js_name = applyIntermediateStore)]
        pub fn apply_intermediate_store(&mut self, store: &SignatureIntermediateStore) {
            self.options = self.options.clone().with_intermediate_store(&store.store);
        }

        #[wasm_bindgen(js_name = applyEvidenceStore)]
        pub fn apply_evidence_store(
            &mut self,
            store: &SignatureEvidenceStore,
        ) -> Result<(), JsValue> {
            let mut options = self.options.clone();
            options
                .ocsp_responses_der
                .extend(store.ocsp_responses_der.iter().cloned());
            options.crls_der.extend(store.crls_der.iter().cloned());
            if let Some(bundle) = &store.bundle {
                options = options
                    .with_evidence_bundle(bundle.clone())
                    .map_err(js_err)?;
            }
            self.options = options;
            Ok(())
        }

        #[wasm_bindgen(js_name = applyRetrievalPolicy)]
        pub fn apply_retrieval_policy(
            &mut self,
            policy: &SignatureRetrievalPolicy,
        ) -> Result<(), JsValue> {
            if policy.policy.enabled {
                return Err(JsValue::from_str(
                    "online retrieval is unsupported in WASM without an explicit host transport",
                ));
            }
            self.options = self
                .options
                .clone()
                .with_retrieval_policy(policy.policy.clone())
                .map_err(js_err)?;
            Ok(())
        }

        #[wasm_bindgen(js_name = setCancellation)]
        pub fn set_cancellation(&mut self, cancellation: &SignatureValidationCancellation) {
            self.options = self
                .options
                .clone()
                .with_cancellation_token(cancellation.token.clone());
        }

        #[wasm_bindgen(js_name = onlineRetrievalCapability)]
        pub fn online_retrieval_capability() -> String {
            "unsupported_without_explicit_host_transport".to_string()
        }
    }

    #[wasm_bindgen]
    pub struct WellfriendPdf {
        engine: ContentEngine,
        bytes: Vec<u8>,
        closed: bool,
    }

    #[wasm_bindgen]
    pub struct WellfriendOutput {
        bytes: Vec<u8>,
        report_json: String,
    }

    #[wasm_bindgen]
    impl WellfriendOutput {
        #[wasm_bindgen(js_name = bytes)]
        pub fn bytes(&self) -> Vec<u8> {
            self.bytes.clone()
        }

        #[wasm_bindgen(js_name = byteLength)]
        pub fn byte_length(&self) -> usize {
            self.bytes.len()
        }

        #[wasm_bindgen(js_name = reportJson)]
        pub fn report_json(&self) -> String {
            self.report_json.clone()
        }
    }

    #[wasm_bindgen]
    impl WellfriendPdf {
        #[wasm_bindgen(constructor)]
        pub fn new(bytes: &[u8]) -> Result<WellfriendPdf, JsValue> {
            install_panic_hook();
            let engine = ContentEngine::open_bytes(bytes.to_vec()).map_err(js_err)?;
            Ok(Self {
                engine,
                bytes: bytes.to_vec(),
                closed: false,
            })
        }

        #[wasm_bindgen(js_name = openWithPassword)]
        pub fn open_with_password(bytes: &[u8], password: &[u8]) -> Result<WellfriendPdf, JsValue> {
            install_panic_hook();
            let engine = ContentEngine::open_bytes_with_password(bytes.to_vec(), password)
                .map_err(js_err)?;
            Ok(Self {
                engine,
                bytes: bytes.to_vec(),
                closed: false,
            })
        }

        #[wasm_bindgen(js_name = sdkVersion)]
        pub fn sdk_version() -> String {
            wellfriendpdf_engine::ENGINE_VERSION.to_string()
        }

        #[wasm_bindgen(js_name = abiVersion)]
        pub fn abi_version() -> u32 {
            sdk::REPORT_ENVELOPE_VERSION
        }

        #[wasm_bindgen(js_name = featureReportJson)]
        pub fn feature_report_json() -> Result<String, JsValue> {
            install_panic_hook();
            sdk::feature_report_json().map_err(js_err)
        }

        #[wasm_bindgen(js_name = prompt21HistoryReportJson)]
        pub fn prompt21_history_report_json() -> Result<String, JsValue> {
            install_panic_hook();
            sdk::prompt21_history_report_json().map_err(js_err)
        }

        #[wasm_bindgen(js_name = cryptoTamperTestJson)]
        pub fn crypto_tamper_test_json() -> Result<String, JsValue> {
            install_panic_hook();
            sdk::crypto_tamper_test_json().map_err(js_err)
        }

        #[wasm_bindgen(js_name = timestampTokenValidationJson)]
        pub fn timestamp_token_validation_json(
            token: &[u8],
            signature_value: &[u8],
            options_json: Option<String>,
        ) -> Result<String, JsValue> {
            install_panic_hook();
            sdk::timestamp_token_validation_json(
                token,
                signature_value,
                options_json.as_deref().unwrap_or("{}"),
            )
            .map_err(js_err)
        }

        #[wasm_bindgen(js_name = prompt22OfficeInspectJson)]
        pub fn prompt22_office_inspect_json(bytes: &[u8], format: &str) -> Result<String, JsValue> {
            install_panic_hook();
            sdk::prompt22_office_inspect_json(bytes, format).map_err(js_err)
        }

        #[wasm_bindgen(js_name = prompt22OfficeToPdf)]
        pub fn prompt22_office_to_pdf(
            bytes: &[u8],
            format: &str,
        ) -> Result<WellfriendOutput, JsValue> {
            install_panic_hook();
            let (out, report) = sdk::prompt22_office_to_pdf_json(bytes, format).map_err(js_err)?;
            Ok(WellfriendOutput {
                bytes: out,
                report_json: report,
            })
        }

        #[wasm_bindgen(js_name = decodeBudgetReportJson)]
        pub fn decode_budget_report_json(
            filter: &str,
            width: u32,
            height: u32,
            components: u8,
        ) -> Result<String, JsValue> {
            install_panic_hook();
            sdk::decode_budget_report_json(filter, width, height, components).map_err(js_err)
        }

        #[wasm_bindgen(js_name = codecIsolationReportJson)]
        pub fn codec_isolation_report_json(
            filter: &str,
            data: &[u8],
            policy: Option<String>,
        ) -> Result<String, JsValue> {
            install_panic_hook();
            sdk::codec_isolation_report_json(filter, data, policy.as_deref()).map_err(js_err)
        }

        #[wasm_bindgen(js_name = close)]
        pub fn close(&mut self) {
            self.closed = true;
        }

        #[wasm_bindgen(js_name = isClosed)]
        pub fn is_closed(&self) -> bool {
            self.closed
        }

        #[wasm_bindgen(js_name = pageCount)]
        pub fn page_count(&self) -> Result<usize, JsValue> {
            self.ensure_open()?;
            self.engine.page_count().map_err(js_err)
        }

        #[wasm_bindgen(js_name = extractText)]
        pub fn extract_text(&self, page: usize) -> Result<String, JsValue> {
            self.ensure_open()?;
            self.engine.get_page_text(page).map_err(js_err)
        }

        #[wasm_bindgen(js_name = extractStructuredText)]
        pub fn extract_structured_text(&self, page: usize) -> Result<String, JsValue> {
            self.ensure_open()?;
            self.engine.get_page_text_structured(page).map_err(js_err)
        }

        #[wasm_bindgen(js_name = extractSemanticJson)]
        pub fn extract_semantic_json(&self) -> Result<String, JsValue> {
            self.ensure_open()?;
            let semantic = self.engine.extract_semantic_document(&[]).map_err(js_err)?;
            serde_json::to_string(&semantic).map_err(|err| JsValue::from_str(&err.to_string()))
        }

        #[wasm_bindgen(js_name = parseMarkdown)]
        pub fn parse_markdown(&self) -> Result<String, JsValue> {
            self.ensure_open()?;
            let doc = self
                .engine
                .parse_document(&ParseOptions::default())
                .map_err(js_err)?;
            Ok(doc.to_markdown_default())
        }

        #[wasm_bindgen(js_name = parseJson)]
        pub fn parse_json(&self) -> Result<String, JsValue> {
            self.ensure_open()?;
            let doc = self
                .engine
                .parse_document(&ParseOptions::default())
                .map_err(js_err)?;
            Ok(doc.to_json())
        }

        #[wasm_bindgen(js_name = chunk)]
        pub fn chunk(&self, target_tokens: usize, overlap: usize) -> Result<String, JsValue> {
            self.ensure_open()?;
            let doc = self
                .engine
                .parse_document(&ParseOptions::default())
                .map_err(js_err)?;
            let mut opts = ChunkOptions::default();
            if target_tokens > 0 {
                opts.target_tokens = target_tokens;
            }
            if overlap > 0 {
                opts.overlap_tokens = overlap;
            }
            Ok(doc.chunk(&opts).to_json())
        }

        #[wasm_bindgen(js_name = extractFieldsJson)]
        pub fn extract_fields_json(&self, doc_type: &str) -> Result<String, JsValue> {
            self.ensure_open()?;
            let opts = ExtractOptions {
                doc_type: DocType::parse(doc_type),
                ..Default::default()
            };
            let fields = self.engine.extract_fields(&opts).map_err(js_err)?;
            Ok(fields.to_json())
        }

        #[wasm_bindgen(js_name = infoJson)]
        pub fn info_json(&self) -> Result<String, JsValue> {
            self.ensure_open()?;
            let info = self.engine.document_info().map_err(js_err)?;
            serde_json::to_string(&info).map_err(|err| JsValue::from_str(&err.to_string()))
        }

        #[wasm_bindgen(js_name = renderPagePng)]
        pub fn render_page_png(&self, page: usize, dpi: u32) -> Result<Vec<u8>, JsValue> {
            self.ensure_open()?;
            self.engine.render_page_png_fast(page, dpi).map_err(js_err)
        }

        #[wasm_bindgen(js_name = documentInfoJson)]
        pub fn document_info_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::document_info_json(b, None))
        }

        #[wasm_bindgen(js_name = securityReportJson)]
        pub fn security_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::security_report_json(b, None))
        }

        #[wasm_bindgen(js_name = riskyContentReportJson)]
        pub fn risky_content_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::risky_content_report_json(b, None))
        }

        #[wasm_bindgen(js_name = parserReportJson)]
        pub fn parser_report_json(&self, mode: Option<String>) -> Result<String, JsValue> {
            self.report(|b| sdk::parser_report_json(b, mode.as_deref(), None))
        }

        #[wasm_bindgen(js_name = colorReportJson)]
        pub fn color_report_json(&self, profile: Option<String>) -> Result<String, JsValue> {
            self.report(|b| sdk::color_report_json(b, profile.as_deref()))
        }

        #[wasm_bindgen(js_name = validateJson)]
        pub fn validate_json(&self, profile: Option<String>) -> Result<String, JsValue> {
            self.report(|b| sdk::standards_profile_json(b, profile.as_deref(), None))
        }

        #[wasm_bindgen(js_name = validatePdfaJson)]
        pub fn validate_pdfa_json(&self, profile: Option<String>) -> Result<String, JsValue> {
            self.report(|b| sdk::pdfa_validation_json(b, profile.as_deref(), None))
        }

        #[wasm_bindgen(js_name = validatePdfuaJson)]
        pub fn validate_pdfua_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::pdfua_validation_json(b, None))
        }

        /// Prompt 26 clause-mapped PDF/A validation. `target` e.g. "PDF/A-2B".
        #[wasm_bindgen(js_name = validatePdfaStandardsJson)]
        pub fn validate_pdfa_standards_json(
            &self,
            target: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::pdfa_standards_json(b, target.as_deref(), None))
        }

        /// Prompt 26 clause-mapped PDF/UA validation. `target` e.g. "PDF/UA-1".
        #[wasm_bindgen(js_name = validatePdfuaStandardsJson)]
        pub fn validate_pdfua_standards_json(
            &self,
            target: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::pdfua_standards_json(b, target.as_deref(), None))
        }

        /// Prompt 26 clause-mapped PDF/X validation. `target` e.g. "PDF/X-4".
        #[wasm_bindgen(js_name = validatePdfxStandardsJson)]
        pub fn validate_pdfx_standards_json(
            &self,
            target: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::pdfx_standards_json(b, target.as_deref(), None))
        }

        /// Prompt 26 combined PDF/A + PDF/UA + PDF/X validation with
        /// cross-profile conflicts.
        #[wasm_bindgen(js_name = validateStandardsAllJson)]
        pub fn validate_standards_all_json(
            &self,
            target: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::standards_all_json(b, target.as_deref(), None))
        }

        /// Exact WASM signing capability posture. In-memory local signing with
        /// caller-provided PEM key material is supported (pure compute); host
        /// filesystem key loading, network TSA acquisition, and JS external
        /// signer callbacks are reported unsupported rather than faked.
        #[wasm_bindgen(js_name = signingCapabilities)]
        pub fn signing_capabilities() -> String {
            r#"{"in_memory_local_signing":"supported","external_signer_callback":"unsupported_reported_exact","host_filesystem_key_load":"unsupported_reported_exact","network_tsa_acquisition":"unsupported_reported_exact","note":"WASM signs only with caller-supplied in-memory PEM key material; no host filesystem, no network TSA, no JS external-signer callback."}"#.to_string()
        }

        /// Prompt 26 append-only incremental signing plan (in-memory). `certify`
        /// in 1..=3 plans a certification (DocMDP) signature; else approval.
        #[wasm_bindgen(js_name = signPlanJson)]
        pub fn sign_plan_json(
            &self,
            key_pem: &str,
            cert_pem: &str,
            placeholder_size: usize,
            certify: i32,
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let signer = PdfSigner::from_pem(key_pem, cert_pem, &[]).map_err(js_err)?;
            let options = incremental_options(placeholder_size, certify);
            let plan = wellfriendpdf_engine::plan_signature_placeholder(
                self.engine.document(),
                &signer,
                &options,
            )
            .map_err(js_err)?;
            serde_json::to_string(&plan).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        /// Prompt 26 append-only incremental signing (in-memory). Produces a
        /// signed PDF whose original bytes are preserved as a prefix, reopened
        /// and validated before it is returned. `key_pem`/`cert_pem` are the
        /// caller-supplied in-memory signer material (never logged/persisted).
        #[wasm_bindgen(js_name = signPdf)]
        pub fn sign_pdf(
            &self,
            key_pem: &str,
            cert_pem: &str,
            placeholder_size: usize,
            certify: i32,
            field_name: Option<String>,
            reason: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let signer = PdfSigner::from_pem(key_pem, cert_pem, &[]).map_err(js_err)?;
            let mut options = incremental_options(placeholder_size, certify);
            if let Some(field) = field_name {
                options.signature.field_name = field;
            }
            options.signature.reason = reason;
            let result = wellfriendpdf_engine::sign_incremental(
                self.engine.document(),
                IncrementalSigner::Local(&signer),
                &options,
            )
            .map_err(js_err)?;
            if !result.post_sign.signature_valid {
                return Err(JsValue::from_str(
                    "post-sign validation failed; signed output not returned",
                ));
            }
            let report_json = serde_json::to_string(&result)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            Ok(WellfriendOutput {
                bytes: result.signed_pdf,
                report_json,
            })
        }

        #[wasm_bindgen(js_name = formsReportJson)]
        pub fn forms_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::forms_report_json(b, None))
        }

        #[wasm_bindgen(js_name = xfaReportJson)]
        pub fn xfa_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::xfa_report_json(b, None))
        }

        #[wasm_bindgen(js_name = xfaExtractJson)]
        pub fn xfa_extract_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::xfa_extract_json(b, None))
        }

        #[wasm_bindgen(js_name = xfaScriptReportJson)]
        pub fn xfa_script_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::xfa_script_report_json(b, None))
        }

        #[wasm_bindgen(js_name = xfaSecurityReportJson)]
        pub fn xfa_security_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::xfa_security_report_json(b, None))
        }

        #[wasm_bindgen(js_name = xfaRuntimeReportJson)]
        pub fn xfa_runtime_report_json(
            &self,
            script_policy: Option<String>,
            execute_events: bool,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::xfa_runtime_report_json(b, script_policy.as_deref(), execute_events, None)
            })
        }

        #[wasm_bindgen(js_name = annotationsReportJson)]
        pub fn annotations_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::annotation_report_json(b, None))
        }

        #[wasm_bindgen(js_name = richMediaReportJson)]
        pub fn rich_media_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::rich_media_report_json(b, None))
        }

        #[wasm_bindgen(js_name = annotationAppearanceReportJson)]
        pub fn annotation_appearance_report_json(
            &self,
            options_json: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::annotation_appearance_report_json(b, options_json.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = nonaxisRedactionPlanJson)]
        pub fn nonaxis_redaction_plan_json(&self, options_json: &str) -> Result<String, JsValue> {
            self.report(|b| sdk::nonaxis_redaction_plan_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = prompt17ReportJson)]
        pub fn prompt17_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt17_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt18ReportJson)]
        pub fn prompt18_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt18_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt18bReportJson)]
        pub fn prompt18b_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt18b_report_json(b, None))
        }

        #[wasm_bindgen(js_name = formJsReportJson)]
        pub fn form_js_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::form_js_report_json(b, None))
        }

        #[wasm_bindgen(js_name = formActionGraphJson)]
        pub fn form_action_graph_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::form_action_graph_json(b, None))
        }

        #[wasm_bindgen(js_name = interactiveDataReportJson)]
        pub fn interactive_data_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::interactive_data_closeout_report_json(b, None))
        }

        #[wasm_bindgen(js_name = wordPaginationAuditJson)]
        pub fn word_pagination_audit_json(&self, layout: &str) -> Result<String, JsValue> {
            self.report(|b| sdk::word_pagination_audit_json(b, layout, None))
        }

        #[wasm_bindgen(js_name = prompt19ReportJson)]
        pub fn prompt19_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt19_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt20ReportJson)]
        pub fn prompt20_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt20_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt20bReportJson)]
        pub fn prompt20b_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt20b_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt31ReportJson)]
        pub fn prompt31_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt31_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt32ReportJson)]
        pub fn prompt32_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt32_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt21ReportJson)]
        pub fn prompt21_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt21_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt22ReportJson)]
        pub fn prompt22_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt22_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt23ReportJson)]
        pub fn prompt23_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt23_report_json(b, None))
        }

        #[wasm_bindgen(js_name = writerDeterminismAuditJson)]
        pub fn writer_determinism_audit_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::writer_determinism_audit_json(b, None))
        }

        #[wasm_bindgen(js_name = writerExternalDiffJson)]
        pub fn writer_external_diff_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::writer_external_diff_json(b, None))
        }

        #[wasm_bindgen(js_name = writerCloseoutReportJson)]
        pub fn writer_closeout_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::writer_closeout_report_json(b, None))
        }

        #[wasm_bindgen(js_name = pubsecReportJson)]
        pub fn pubsec_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::pubsec_report_json(b, None))
        }

        #[wasm_bindgen(js_name = aesGcmReportJson)]
        pub fn aes_gcm_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::aes_gcm_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt21RasterVectorReportJson)]
        pub fn prompt21_raster_vector_report_json(
            &self,
            page: usize,
            options_json: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::prompt21_raster_vector_report_json(b, page, options_json.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = prompt21FontReconstructionReportJson)]
        pub fn prompt21_font_reconstruction_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt21_font_reconstruction_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt21ObjectStreamReportJson)]
        pub fn prompt21_object_stream_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt21_object_stream_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt20bTextRangeAnalyzeJson)]
        pub fn prompt20b_text_range_analyze_json(&self, page: usize) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt20b_text_range_analyze_json(b, page, None))
        }

        #[wasm_bindgen(js_name = prompt31ProvenanceJson)]
        pub fn prompt31_provenance_json(
            &self,
            page: usize,
            source_text: String,
            replacement_text: String,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::prompt31_provenance_json(b, page, &source_text, &replacement_text, None)
            })
        }

        #[wasm_bindgen(js_name = prompt31EditEligibilityJson)]
        pub fn prompt31_edit_eligibility_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt31_edit_eligibility_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt31PathProvenanceJson)]
        pub fn prompt31_path_provenance_json(&self, page: usize) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt31_path_provenance_json(b, page, None))
        }

        #[wasm_bindgen(js_name = prompt31ImageEligibilityJson)]
        pub fn prompt31_image_eligibility_json(
            &self,
            page: usize,
            occurrence: Option<String>,
        ) -> Result<String, JsValue> {
            let _ = occurrence.as_deref();
            self.report(|b| sdk::prompt31_image_eligibility_json(b, page, None))
        }

        #[wasm_bindgen(js_name = prompt32SceneReportJson)]
        pub fn prompt32_scene_report_json(
            &self,
            pages_json: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt32_scene_report_json(b, pages_json.as_deref(), None))
        }

        #[wasm_bindgen(js_name = prompt32SceneSelectJson)]
        pub fn prompt32_scene_select_json(&self, request_json: String) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt32_scene_select_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt32TransactionPlanJson)]
        pub fn prompt32_transaction_plan_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt32_transaction_plan_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt32TextMapJson)]
        pub fn prompt32_text_map_json(
            &self,
            text: String,
            direction: Option<String>,
        ) -> Result<String, JsValue> {
            sdk::prompt32_text_map_json(&text, direction.as_deref()).map_err(js_err)
        }

        #[wasm_bindgen(js_name = prompt32ShapeTextJson)]
        pub fn prompt32_shape_text_json(
            &self,
            text: String,
            direction: Option<String>,
        ) -> Result<String, JsValue> {
            sdk::prompt32_shape_text_json(&text, direction.as_deref()).map_err(js_err)
        }

        #[wasm_bindgen(js_name = prompt32FontSubsetPlanJson)]
        pub fn prompt32_font_subset_plan_json(
            &self,
            text: String,
            direction: Option<String>,
            policy: Option<String>,
        ) -> Result<String, JsValue> {
            sdk::prompt32_font_subset_plan_json(&text, direction.as_deref(), policy.as_deref())
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = prompt32FontSubstitutionReportJson)]
        pub fn prompt32_font_substitution_report_json(
            &self,
            requested_family: String,
            text: String,
            policy: Option<String>,
        ) -> Result<String, JsValue> {
            sdk::prompt32_font_substitution_report_json(&requested_family, &text, policy.as_deref())
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = prompt33ReportJson)]
        pub fn prompt33_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt33LayoutAnalyzeJson)]
        pub fn prompt33_layout_analyze_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_layout_analyze_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt33SemanticLayoutJson)]
        pub fn prompt33_semantic_layout_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_semantic_layout_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt33ReadingOrderReportJson)]
        pub fn prompt33_reading_order_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_reading_order_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt33FlowGraphReportJson)]
        pub fn prompt33_flow_graph_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_flow_graph_report_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt33ReflowPreviewJson)]
        pub fn prompt33_reflow_preview_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_reflow_preview_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt33OverflowReportJson)]
        pub fn prompt33_overflow_report_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_overflow_report_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt33ConstraintsReportJson)]
        pub fn prompt33_constraints_report_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_constraints_report_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt33ConfidenceReportJson)]
        pub fn prompt33_confidence_report_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_confidence_report_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt33ValidateReflowOutputJson)]
        pub fn prompt33_validate_reflow_output_json(
            &self,
            output_pdf: Vec<u8>,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::prompt33_validate_reflow_output_json(b, &output_pdf, &request_json, None)
            })
        }

        #[wasm_bindgen(js_name = prompt33ReflowOperationReportJson)]
        pub fn prompt33_reflow_operation_report_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt33_reflow_operation_report_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt20VectorListJson)]
        pub fn prompt20_vector_list_json(&self, page: usize) -> Result<String, JsValue> {
            self.report(|b| sdk::prompt20_vector_list_json(b, page, None))
        }

        #[wasm_bindgen(js_name = associatedFilesReportJson)]
        pub fn associated_files_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::associated_files_report_json(b, None))
        }

        #[wasm_bindgen(js_name = editPolicyReportJson)]
        pub fn edit_policy_report_json(&self, operation: &str) -> Result<String, JsValue> {
            self.report(|b| sdk::edit_policy_report_json(b, operation, None))
        }

        #[wasm_bindgen(js_name = pagesReportJson)]
        pub fn pages_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::page_operations_report_json(b, None))
        }

        #[wasm_bindgen(js_name = interactiveReportJson)]
        pub fn interactive_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::interactive_report_json(b, None))
        }

        #[wasm_bindgen(js_name = signatureReportJson)]
        pub fn signature_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::signature_report_json(b, None))
        }

        #[wasm_bindgen(js_name = signatureReportWithOptionsJson)]
        pub fn signature_report_with_options_json(
            &self,
            options_json: &str,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::signature_report_with_options_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = signatureValidationWithEvidenceJson)]
        pub fn signature_validation_with_evidence_json(
            &self,
            options_json: &str,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::signature_validation_with_evidence_json(b, options_json, None))
        }

        /// Offline Prompt 24 validation with owned caller-supplied trust and
        /// evidence.  WASM never performs implicit AIA, OCSP, or CRL retrieval.
        #[wasm_bindgen(js_name = signatureValidation)]
        pub fn signature_validation(
            &self,
            options: &SignatureValidationOptions,
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let reports = self
                .engine
                .verify_signatures_with_options(&options.options)
                .map_err(js_err)?;
            serde_json::to_string(&reports).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        /// Offline Prompt 24 validation plus a portable, hash-checked evidence
        /// bundle that can be replayed by a later WASM or native invocation.
        #[wasm_bindgen(js_name = signatureValidationWithEvidence)]
        pub fn signature_validation_with_evidence(
            &self,
            options: &SignatureValidationOptions,
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let outcome = self
                .engine
                .verify_signatures_with_options_and_evidence(&options.options)
                .map_err(js_err)?;
            serde_json::to_string(&outcome).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = signaturePreservingFormPlanJson)]
        pub fn signature_preserving_form_plan_json(
            &self,
            field_name: &str,
            value: &str,
            options_json: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::signature_preserving_form_plan_json(
                    b,
                    field_name,
                    value,
                    options_json.as_deref().unwrap_or("{}"),
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = signaturePreservingFormEdit)]
        pub fn signature_preserving_form_edit(
            &self,
            field_name: &str,
            value: &str,
            options_json: Option<String>,
            explicit_invalidation_override: bool,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::signature_preserving_form_edit_json(
                    b,
                    field_name,
                    value,
                    options_json.as_deref().unwrap_or("{}"),
                    explicit_invalidation_override,
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = fontReportJson)]
        pub fn font_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::font_report_json(b, None))
        }

        #[wasm_bindgen(js_name = textSemanticJson)]
        pub fn text_semantic_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::text_semantic_json(b, &[], None))
        }

        #[wasm_bindgen(js_name = semanticDocumentReportJson)]
        pub fn semantic_document_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::semantic_document_json(b, &[], None))
        }

        #[wasm_bindgen(js_name = chunksJson)]
        pub fn chunks_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::chunk_report_json(b, None))
        }

        #[wasm_bindgen(js_name = advancedChunksJson)]
        pub fn advanced_chunks_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::advanced_chunk_report_json(b, &[], None))
        }

        #[wasm_bindgen(js_name = semanticBundleJson)]
        pub fn semantic_bundle_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::semantic_binding_report_json(b, &[], None))
        }

        #[wasm_bindgen(js_name = semanticSearchJson)]
        pub fn semantic_search_json(&self, query: &str) -> Result<String, JsValue> {
            self.report(|b| sdk::semantic_search_report_json(b, &[], query, None))
        }

        #[wasm_bindgen(js_name = tableProposalStatusJson)]
        pub fn table_proposal_status_json() -> Result<String, JsValue> {
            install_panic_hook();
            sdk::table_proposal_status_json().map_err(js_err)
        }

        #[wasm_bindgen(js_name = prompt20TextEdit)]
        pub fn prompt20_text_edit(
            &self,
            page: usize,
            old_text: &str,
            new_text: &str,
            mode: &str,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::prompt20_text_edit_json(
                    b,
                    page,
                    old_text,
                    new_text,
                    mode,
                    options_json.as_deref(),
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = prompt31OperatorTextEdit)]
        pub fn prompt31_operator_text_edit(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::prompt31_operator_text_edit_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt32TransactionApply)]
        pub fn prompt32_transaction_apply(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::prompt32_transaction_apply_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt32SceneEditText)]
        pub fn prompt32_scene_edit_text(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::prompt32_scene_edit_text_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt33ReflowRegion)]
        pub fn prompt33_reflow_region(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::prompt33_reflow_region_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt33ReflowDocument)]
        pub fn prompt33_reflow_document(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::prompt33_reflow_document_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt33UndoReflow)]
        pub fn prompt33_undo_reflow(
            &self,
            output_pdf: Vec<u8>,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::prompt33_undo_reflow_json(b, &output_pdf, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt31PathEdit)]
        pub fn prompt31_path_edit(
            &self,
            page: usize,
            stable_id: &str,
            operation_json: &str,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::prompt31_path_edit_json(
                    b,
                    page,
                    stable_id,
                    operation_json,
                    options_json.as_deref(),
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = prompt20VectorEdit)]
        pub fn prompt20_vector_edit(
            &self,
            page: usize,
            stable_id: &str,
            operation_json: &str,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::prompt20_vector_edit_json(
                    b,
                    page,
                    stable_id,
                    operation_json,
                    options_json.as_deref(),
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = prompt20InkFit)]
        pub fn prompt20_ink_fit(
            &self,
            page: usize,
            annotation_index: usize,
            options_json: Option<String>,
            signature_policy_override: bool,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::prompt20_ink_fit_json(
                    b,
                    page,
                    annotation_index,
                    options_json.as_deref(),
                    signature_policy_override,
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = xfaRender)]
        pub fn xfa_render(
            &self,
            script_policy: Option<String>,
            execute_events: bool,
            dpi: u32,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::xfa_render_preview_json(b, script_policy.as_deref(), execute_events, dpi, None)
            })
        }

        #[wasm_bindgen(js_name = xfaFlatten)]
        pub fn xfa_flatten(&self, mode: Option<String>) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::xfa_flatten_json(b, mode.as_deref(), None))
        }

        #[wasm_bindgen(js_name = xfaSanitize)]
        pub fn xfa_sanitize(&self, mode: Option<String>) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::xfa_sanitize_json(b, mode.as_deref(), None))
        }

        #[wasm_bindgen(js_name = annotationXfdfExport)]
        pub fn annotation_xfdf_export(&self) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::annotation_xfdf_export_json(b, None))
        }

        #[wasm_bindgen(js_name = annotationXfdfImport)]
        pub fn annotation_xfdf_import(
            &self,
            xfdf: &[u8],
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::annotation_xfdf_import_json(b, xfdf, options_json.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = annotationAppearanceGenerate)]
        pub fn annotation_appearance_generate(
            &self,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::annotation_appearance_generate_json(b, options_json.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = editTextRange)]
        pub fn edit_text_range(&self, request_json: String) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::prompt20b_text_range_edit_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt21PackObjectStreams)]
        pub fn prompt21_pack_object_streams(&self) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::prompt21_pack_object_streams_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt22Optimize)]
        pub fn prompt22_optimize(
            &self,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::prompt22_optimize_pdf_json(b, options_json.as_deref(), None))
        }

        #[wasm_bindgen(js_name = richMediaSanitize)]
        pub fn rich_media_sanitize(
            &self,
            mode: Option<String>,
            custom_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::rich_media_sanitize_json(b, mode.as_deref(), custom_json.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = richMediaFlattenPoster)]
        pub fn rich_media_flatten_poster(&self) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::rich_media_flatten_poster_json(b, None))
        }

        #[wasm_bindgen(js_name = redactImageNonaxis)]
        pub fn redact_image_nonaxis(
            &self,
            options_json: &str,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::nonaxis_redaction_apply_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = redactImageMask)]
        pub fn redact_image_mask(&self, options_json: &str) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::redact_image_mask_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = redactInlineImage)]
        pub fn redact_inline_image(&self, options_json: &str) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::redact_inline_image_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = associatedFileAdd)]
        pub fn associated_file_add(
            &self,
            payload: &[u8],
            options_json: &str,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::associated_files_add_json(b, payload, options_json, None))
        }

        #[wasm_bindgen(js_name = associatedFileUpdateOwner)]
        pub fn associated_file_update_owner(
            &self,
            payload: &[u8],
            options_json: &str,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::associated_files_update_owner_json(b, payload, options_json, None))
        }

        #[wasm_bindgen(js_name = associatedFileRemoveOwner)]
        pub fn associated_file_remove_owner(
            &self,
            options_json: &str,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::associated_files_remove_owner_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = incrementalFormEdit)]
        pub fn incremental_form_edit(
            &self,
            field_name: &str,
            value: &str,
            signature_policy_override: bool,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::incremental_form_edit_json(
                    b,
                    field_name,
                    value,
                    signature_policy_override,
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = incrementalAnnotationEdit)]
        pub fn incremental_annotation_edit(
            &self,
            options_json: &str,
            signature_policy_override: bool,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::incremental_annotation_edit_json(
                    b,
                    options_json,
                    signature_policy_override,
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = incrementalPagePropertyEdit)]
        pub fn incremental_page_property_edit(
            &self,
            options_json: &str,
            signature_policy_override: bool,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::incremental_page_property_edit_json(
                    b,
                    options_json,
                    signature_policy_override,
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = associatedFilesSanitize)]
        pub fn associated_files_sanitize(
            &self,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::associated_files_sanitize_json(b, options_json.as_deref(), None))
        }

        #[wasm_bindgen(js_name = formJsSanitize)]
        pub fn form_js_sanitize(
            &self,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::form_js_sanitize_json(b, options_json.as_deref(), None))
        }

        #[wasm_bindgen(js_name = formJsFlattenValues)]
        pub fn form_js_flatten_values(
            &self,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::form_js_flatten_values_json(b, options_json.as_deref(), None))
        }

        #[wasm_bindgen(js_name = associatedFilesRemove)]
        pub fn associated_files_remove(
            &self,
            stable_ids_json: &str,
        ) -> Result<WellfriendOutput, JsValue> {
            let stable_ids: Vec<String> = serde_json::from_str(stable_ids_json)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            self.output(|b| sdk::associated_files_remove_json(b, &stable_ids, None))
        }

        #[wasm_bindgen(js_name = sanitize)]
        pub fn sanitize(&self, policy: Option<String>) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::sanitize_json(b, policy.as_deref(), None))
        }

        #[wasm_bindgen(js_name = canonicalize)]
        pub fn canonicalize(&self, date_epoch: Option<i64>) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::canonicalize_json(b, date_epoch, None))
        }

        #[wasm_bindgen(js_name = redactTermsJson)]
        pub fn redact_terms_json(
            &self,
            terms_json: &str,
            strict: bool,
        ) -> Result<WellfriendOutput, JsValue> {
            let terms: Vec<String> = serde_json::from_str(terms_json)
                .map_err(|err| JsValue::from_str(&err.to_string()))?;
            self.output(|b| sdk::redact_terms_json(b, &terms, strict, None))
        }

        fn ensure_open(&self) -> Result<(), JsValue> {
            if self.closed {
                Err(JsValue::from_str(
                    "WellfriendPdf document is closed; create a new instance before calling this method",
                ))
            } else {
                Ok(())
            }
        }

        fn report<F>(&self, f: F) -> Result<String, JsValue>
        where
            F: FnOnce(&[u8]) -> wellfriendpdf_engine::Result<String>,
        {
            self.ensure_open()?;
            f(&self.bytes).map_err(js_err)
        }

        fn output<F>(&self, f: F) -> Result<WellfriendOutput, JsValue>
        where
            F: FnOnce(&[u8]) -> wellfriendpdf_engine::Result<(Vec<u8>, String)>,
        {
            self.ensure_open()?;
            let (bytes, report_json) = f(&self.bytes).map_err(js_err)?;
            Ok(WellfriendOutput { bytes, report_json })
        }
    }

    fn js_err(err: wellfriendpdf_engine::WellfriendError) -> JsValue {
        JsValue::from_str(&err.to_string())
    }

    fn incremental_options(placeholder_size: usize, certify: i32) -> IncrementalSigningOptions {
        let intent = if (1..=3).contains(&certify) {
            SigningIntent::Certification {
                docmdp_permissions: certify as u8,
            }
        } else {
            SigningIntent::Approval
        };
        IncrementalSigningOptions {
            signature: SignatureOptions {
                contents_reserved_bytes: placeholder_size.max(1),
                ..Default::default()
            },
            intent,
            retry_larger_placeholder: true,
            max_placeholder_bytes: 256 * 1024,
        }
    }

    fn install_panic_hook() {
        #[cfg(feature = "panic-hook")]
        console_error_panic_hook::set_once();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct WellfriendWasmBuildsOnlyForWasm32;
