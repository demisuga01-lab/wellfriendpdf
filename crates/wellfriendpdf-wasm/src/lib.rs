//! wasm-bindgen wrapper for `wellfriendpdf-engine`.
//!
//! The browser/Node/WebWorker surface accepts caller-provided bytes only. It
//! does not fetch URLs, read host files implicitly, or execute PDF active
//! content. Reports are routed through `wellfriendpdf_engine::sdk` so the JSON envelope
//! matches Rust, Python, and the C ABI.

#[cfg(target_arch = "wasm32")]
mod wasm_api {
    use js_sys::{Function, Reflect};
    use serde::de::DeserializeOwned;
    use wasm_bindgen::prelude::*;

    use wellfriendpdf_engine::render::{
        apply_render_invalidation_plan_json_to_cache, AlphaMode, ContractColor, DeviceClip,
        DeviceMatrix, PixelFormat, RenderContract,
    };
    use wellfriendpdf_engine::{
        sdk, CancelToken, ChunkOptions, ContentEngine, DocType, EvidenceBundle, ExtractOptions,
        IncrementalSigner, IncrementalSigningOptions, IntermediateStore, NetworkBudget,
        ParseOptions, PdfSigner, RenderDocumentCache, RetrievalPolicy, SignatureOptions,
        SignatureRevocationMode, SigningIntent, TrustStore, VerifyOptions,
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

    #[wasm_bindgen]
    pub struct RenderCancellation {
        token: CancelToken,
    }

    #[wasm_bindgen]
    impl RenderCancellation {
        #[wasm_bindgen(constructor)]
        pub fn new() -> RenderCancellation {
            RenderCancellation {
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

    /// Owned offline Signature Validation validation options for the WASM surface.
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
    pub struct ProgressiveRenderJob {
        job: wellfriendpdf_engine::ProgressiveRenderJob,
    }

    #[wasm_bindgen]
    pub struct AdjacentPagePrefetchExecution {
        report_json: String,
        job: Option<ProgressiveRenderJob>,
    }

    #[wasm_bindgen]
    pub struct WellfriendOutput {
        bytes: Vec<u8>,
        report_json: String,
    }

    #[wasm_bindgen]
    pub struct WellfriendRenderContract {
        contract: RenderContract,
    }

    #[wasm_bindgen]
    pub struct RenderCache {
        cache: RenderDocumentCache,
    }

    #[wasm_bindgen]
    impl RenderCache {
        #[wasm_bindgen(constructor)]
        pub fn new() -> RenderCache {
            RenderCache {
                cache: RenderDocumentCache::new(),
            }
        }

        pub fn clear(&mut self) {
            self.cache.clear();
        }

        #[wasm_bindgen(js_name = applyRenderInvalidationPlanJson)]
        pub fn apply_render_invalidation_plan_json(
            &mut self,
            plan_json: &str,
        ) -> Result<String, JsValue> {
            let report = apply_render_invalidation_plan_json_to_cache(&mut self.cache, plan_json)
                .map_err(js_err)?;
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }
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
    impl AdjacentPagePrefetchExecution {
        #[wasm_bindgen(js_name = reportJson)]
        pub fn report_json(&self) -> String {
            self.report_json.clone()
        }

        #[wasm_bindgen(js_name = hasJob)]
        pub fn has_job(&self) -> bool {
            self.job.is_some()
        }

        #[wasm_bindgen(js_name = takeJob)]
        pub fn take_job(&mut self) -> Option<ProgressiveRenderJob> {
            self.job.take()
        }
    }

    #[wasm_bindgen]
    impl WellfriendRenderContract {
        #[wasm_bindgen(js_name = fromJson)]
        pub fn from_json(json: &str) -> Result<WellfriendRenderContract, JsValue> {
            Ok(WellfriendRenderContract {
                contract: parse_render_contract_json(json)?,
            })
        }

        #[wasm_bindgen(js_name = toJson)]
        pub fn to_json(&self) -> Result<String, JsValue> {
            serde_json::to_string(&self.contract)
                .map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = surfaceByteLength)]
        pub fn surface_byte_length(&self) -> Result<usize, JsValue> {
            self.contract
                .stride
                .checked_mul(self.contract.height as usize)
                .ok_or_else(|| JsValue::from_str("render contract surface byte length overflows"))
        }

        #[wasm_bindgen(js_name = withSurface)]
        #[allow(clippy::too_many_arguments)]
        pub fn with_surface(
            &self,
            width: u32,
            height: u32,
            pixel_format: Option<String>,
            alpha_mode: Option<String>,
            stride: Option<usize>,
            grayscale: Option<bool>,
            reverse_byte_order: Option<bool>,
        ) -> Result<WellfriendRenderContract, JsValue> {
            if width == 0 || height == 0 {
                return Err(JsValue::from_str(
                    "render contract surface dimensions must be positive",
                ));
            }
            let pixel_format = parse_pixel_format(pixel_format.as_deref())?;
            let alpha_mode = parse_alpha_mode(alpha_mode.as_deref())?;
            let minimum_stride = width as usize * pixel_format.bytes_per_pixel();
            let stride = stride.unwrap_or(minimum_stride);
            if stride < minimum_stride {
                return Err(JsValue::from_str(&format!(
                    "render contract stride {stride} is below the required {minimum_stride} bytes"
                )));
            }

            let mut contract = self.contract.clone();
            contract.width = width;
            contract.height = height;
            contract.pixel_format = pixel_format;
            contract.alpha_mode = alpha_mode;
            contract.stride = stride;
            contract.grayscale = grayscale.unwrap_or(contract.grayscale);
            contract.reverse_byte_order = reverse_byte_order.unwrap_or(contract.reverse_byte_order);
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withClip)]
        pub fn with_clip(
            &self,
            x: i32,
            y: i32,
            width: u32,
            height: u32,
        ) -> Result<WellfriendRenderContract, JsValue> {
            if width == 0 || height == 0 {
                return Err(JsValue::from_str("render contract clip must be non-empty"));
            }
            let mut contract = self.contract.clone();
            contract.clip = Some(DeviceClip {
                x,
                y,
                width,
                height,
            });
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withoutClip)]
        pub fn without_clip(&self) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.clip = None;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withDeviceTransform)]
        pub fn with_device_transform(
            &self,
            a: f64,
            b: f64,
            c: f64,
            d: f64,
            e: f64,
            f: f64,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.transform = DeviceMatrix::from_f64([a, b, c, d, e, f]);
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withBackground)]
        pub fn with_background(
            &self,
            r: u8,
            g: u8,
            b: u8,
            a: Option<u8>,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.background = ContractColor {
                r,
                g,
                b,
                a: a.unwrap_or(255),
            };
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withPageBox)]
        pub fn with_page_box(&self, page_box: &str) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.page_box = parse_contract_enum("page_box", page_box)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withExecutionMode)]
        pub fn with_execution_mode(
            &self,
            execution_mode: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.execution_mode = parse_contract_enum("execution_mode", execution_mode)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withBackend)]
        pub fn with_backend(&self, backend: &str) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.backend = parse_contract_enum("backend", backend)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withCompositing)]
        pub fn with_compositing(
            &self,
            compositing: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.compositing = parse_contract_enum("compositing", compositing)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withAnnotations)]
        pub fn with_annotations(
            &self,
            annotations: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.annotations = parse_contract_enum("annotations", annotations)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withForms)]
        pub fn with_forms(&self, forms: &str) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.forms = parse_contract_enum("forms", forms)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withOptionalContent)]
        pub fn with_optional_content(
            &self,
            optional_content: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            if optional_content.trim().is_empty() {
                return Err(JsValue::from_str(
                    "render contract optional_content must be present",
                ));
            }
            let mut contract = self.contract.clone();
            contract.optional_content =
                wellfriendpdf_engine::render::OptionalContentStateId(optional_content.to_string());
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withSmoothing)]
        pub fn with_smoothing(&self, smoothing: &str) -> Result<WellfriendRenderContract, JsValue> {
            let smoothing = parse_contract_enum("smoothing", smoothing)?;
            let mut contract = self.contract.clone();
            contract.text_smoothing = smoothing;
            contract.image_smoothing = smoothing;
            contract.path_smoothing = smoothing;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withTextSmoothing)]
        pub fn with_text_smoothing(
            &self,
            text_smoothing: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.text_smoothing = parse_contract_enum("text_smoothing", text_smoothing)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withImageSmoothing)]
        pub fn with_image_smoothing(
            &self,
            image_smoothing: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.image_smoothing = parse_contract_enum("image_smoothing", image_smoothing)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withPathSmoothing)]
        pub fn with_path_smoothing(
            &self,
            path_smoothing: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.path_smoothing = parse_contract_enum("path_smoothing", path_smoothing)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withSubpixelText)]
        pub fn with_subpixel_text(
            &self,
            subpixel_text: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.subpixel_text = parse_contract_enum("subpixel_text", subpixel_text)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withColorScheme)]
        pub fn with_color_scheme(
            &self,
            color_scheme: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.color_scheme = parse_contract_enum("color_scheme", color_scheme)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withPrintProfile)]
        pub fn with_print_profile(
            &self,
            print_profile: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.print_profile = parse_contract_enum("print_profile", print_profile)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withHalftone)]
        pub fn with_halftone(&self, halftone: &str) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.halftone = parse_contract_enum("halftone", halftone)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withOverprint)]
        pub fn with_overprint(&self, overprint: &str) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.overprint = parse_contract_enum("overprint", overprint)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withRenderingIntent)]
        pub fn with_rendering_intent(
            &self,
            rendering_intent: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.rendering_intent = parse_contract_enum("rendering_intent", rendering_intent)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withColorManagement)]
        pub fn with_color_management(
            &self,
            color_management: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.color_management = parse_contract_enum("color_management", color_management)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withExactness)]
        pub fn with_exactness(&self, exactness: &str) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.exactness = parse_contract_enum("exactness", exactness)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withDeterminism)]
        pub fn with_determinism(
            &self,
            determinism: &str,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            contract.determinism = parse_contract_enum("determinism", determinism)?;
            validate_render_contract(contract)
        }

        #[wasm_bindgen(js_name = withResourceBudget)]
        pub fn with_resource_budget(
            &self,
            max_pixels: Option<u64>,
            max_decoded_bytes: Option<u64>,
            max_temporary_bytes: Option<u64>,
            max_cache_bytes: Option<u64>,
        ) -> Result<WellfriendRenderContract, JsValue> {
            let mut contract = self.contract.clone();
            if let Some(max_pixels) = max_pixels {
                contract.resource_budget.max_pixels = max_pixels;
            }
            if let Some(max_decoded_bytes) = max_decoded_bytes {
                contract.resource_budget.max_decoded_bytes = max_decoded_bytes;
            }
            if let Some(max_temporary_bytes) = max_temporary_bytes {
                contract.resource_budget.max_temporary_bytes = max_temporary_bytes;
            }
            if let Some(max_cache_bytes) = max_cache_bytes {
                contract.resource_budget.max_cache_bytes = max_cache_bytes;
            }
            validate_render_contract(contract)
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

        #[wasm_bindgen(js_name = registerFontBytes)]
        pub fn register_font_bytes(
            &mut self,
            name: &str,
            font_bytes: &[u8],
        ) -> Result<(), JsValue> {
            self.ensure_open()?;
            self.engine
                .register_font_bytes(name.to_string(), font_bytes.to_vec())
                .map_err(js_err)
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

        #[wasm_bindgen(js_name = runtimeCapabilitiesJson)]
        pub fn runtime_capabilities_json(config_json: Option<String>) -> Result<String, JsValue> {
            install_panic_hook();
            sdk::runtime_capabilities_json(config_json.as_deref()).map_err(js_err)
        }

        #[wasm_bindgen(js_name = runtimeConfigJson)]
        pub fn runtime_config_json(config_json: Option<String>) -> Result<String, JsValue> {
            install_panic_hook();
            sdk::runtime_effective_config_json(config_json.as_deref()).map_err(js_err)
        }

        #[wasm_bindgen(js_name = ocrProviderMatrixJson)]
        pub fn ocr_provider_matrix_json() -> Result<String, JsValue> {
            install_panic_hook();
            sdk::ocr_provider_matrix_json().map_err(js_err)
        }

        #[wasm_bindgen(js_name = writer_historyHistoryReportJson)]
        pub fn writer_history_history_report_json() -> Result<String, JsValue> {
            install_panic_hook();
            sdk::writer_history_history_report_json().map_err(js_err)
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

        #[wasm_bindgen(js_name = compression_officeOfficeInspectJson)]
        pub fn compression_office_office_inspect_json(
            bytes: &[u8],
            format: &str,
        ) -> Result<String, JsValue> {
            install_panic_hook();
            sdk::compression_office_office_inspect_json(bytes, format).map_err(js_err)
        }

        #[wasm_bindgen(js_name = compression_officeOfficeToPdf)]
        pub fn compression_office_office_to_pdf(
            bytes: &[u8],
            format: &str,
        ) -> Result<WellfriendOutput, JsValue> {
            install_panic_hook();
            let (out, report) =
                sdk::compression_office_office_to_pdf_json(bytes, format).map_err(js_err)?;
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

        #[wasm_bindgen(js_name = imageDecodeCapabilityReportJson)]
        pub fn image_decode_capability_report_json(&self) -> Result<String, JsValue> {
            self.ensure_open()?;
            sdk::image_decode_capability_report_json(&self.bytes, None).map_err(js_err)
        }

        #[wasm_bindgen(js_name = progressiveImageDecodeLifecycleReportJson)]
        pub fn progressive_image_decode_lifecycle_report_json(
            &self,
            request_json: &str,
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            sdk::progressive_image_decode_lifecycle_report_json(&self.bytes, request_json, None)
                .map_err(js_err)
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

        #[wasm_bindgen(js_name = renderPagePngWithFontSubstitutionReport)]
        pub fn render_page_png_with_font_substitution_report(
            &self,
            page: usize,
            dpi: u32,
            mode: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let mode = mode.unwrap_or_else(|| "compat".to_string());
            let render_mode = wellfriendpdf_engine::RenderMode::from_name(&mode)
                .ok_or_else(|| JsValue::from_str("mode must be compat or high"))?;
            let (bytes, log) = self
                .engine
                .render_page_png_fast_with_font_substitution_report(page, dpi, render_mode)
                .map_err(js_err)?;
            let report_json = serde_json::to_string(&log)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = defaultRenderContractJson)]
        pub fn default_render_contract_json(
            &self,
            page: usize,
            dpi: u32,
            mode: Option<String>,
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let mode = mode.unwrap_or_else(|| "compat".to_string());
            let render_mode = wellfriendpdf_engine::RenderMode::from_name(&mode)
                .ok_or_else(|| JsValue::from_str("mode must be compat or high"))?;
            let contract = self
                .engine
                .default_render_contract(page, dpi, render_mode)
                .map_err(js_err)?;
            serde_json::to_string(&contract).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = defaultRenderContract)]
        pub fn default_render_contract(
            &self,
            page: usize,
            dpi: u32,
            mode: Option<String>,
        ) -> Result<WellfriendRenderContract, JsValue> {
            self.ensure_open()?;
            let mode = mode.unwrap_or_else(|| "compat".to_string());
            let render_mode = wellfriendpdf_engine::RenderMode::from_name(&mode)
                .ok_or_else(|| JsValue::from_str("mode must be compat or high"))?;
            let contract = self
                .engine
                .default_render_contract(page, dpi, render_mode)
                .map_err(js_err)?;
            Ok(WellfriendRenderContract { contract })
        }

        #[wasm_bindgen(js_name = renderContractPng)]
        pub fn render_contract_png(&self, contract_json: &str) -> Result<Vec<u8>, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            self.engine
                .render_page_png_with_contract(&contract, &CancelToken::none())
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractPngWithRenderCache)]
        pub fn render_contract_png_with_render_cache(
            &self,
            contract_json: &str,
            cache: &mut RenderCache,
        ) -> Result<Vec<u8>, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            self.engine
                .render_page_png_with_contract_and_cache(
                    &contract,
                    &CancelToken::none(),
                    &mut cache.cache,
                )
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractPngWithCancellation)]
        pub fn render_contract_png_with_cancellation(
            &self,
            contract_json: &str,
            cancellation: &RenderCancellation,
        ) -> Result<Vec<u8>, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            self.engine
                .render_page_png_with_contract(&contract, &cancellation.token)
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractObjectPng)]
        pub fn render_contract_object_png(
            &self,
            contract: &WellfriendRenderContract,
        ) -> Result<Vec<u8>, JsValue> {
            self.ensure_open()?;
            self.engine
                .render_page_png_with_contract(&contract.contract, &CancelToken::none())
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractObjectPngWithRenderCache)]
        pub fn render_contract_object_png_with_render_cache(
            &self,
            contract: &WellfriendRenderContract,
            cache: &mut RenderCache,
        ) -> Result<Vec<u8>, JsValue> {
            self.ensure_open()?;
            self.engine
                .render_page_png_with_contract_and_cache(
                    &contract.contract,
                    &CancelToken::none(),
                    &mut cache.cache,
                )
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractObjectPngWithCancellation)]
        pub fn render_contract_object_png_with_cancellation(
            &self,
            contract: &WellfriendRenderContract,
            cancellation: &RenderCancellation,
        ) -> Result<Vec<u8>, JsValue> {
            self.ensure_open()?;
            self.engine
                .render_page_png_with_contract(&contract.contract, &cancellation.token)
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractPngWithFontSubstitutionReport)]
        pub fn render_contract_png_with_font_substitution_report(
            &self,
            contract_json: &str,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let (bytes, log) = self
                .engine
                .render_page_png_with_contract_and_font_substitution_report(
                    &contract,
                    &CancelToken::none(),
                )
                .map_err(js_err)?;
            let report_json = serde_json::to_string(&log)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractPngWithFontSubstitutionReportWithCancellation)]
        pub fn render_contract_png_with_font_substitution_report_with_cancellation(
            &self,
            contract_json: &str,
            cancellation: &RenderCancellation,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let (bytes, log) = self
                .engine
                .render_page_png_with_contract_and_font_substitution_report(
                    &contract,
                    &cancellation.token,
                )
                .map_err(js_err)?;
            let report_json = serde_json::to_string(&log)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractPngWithRenderReport)]
        pub fn render_contract_png_with_render_report(
            &self,
            contract_json: &str,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let (bytes, log, telemetry_report) = self
                .engine
                .render_page_png_with_contract_and_telemetry_report(&contract, &CancelToken::none())
                .map_err(js_err)?;
            let report_json = contract_render_report_json(&log, &telemetry_report)?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractPngWithRenderCacheReport)]
        pub fn render_contract_png_with_render_cache_report(
            &self,
            contract_json: &str,
            cache: &mut RenderCache,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let (bytes, log, telemetry_report) = self
                .engine
                .render_page_png_with_contract_and_telemetry_report_and_cache(
                    &contract,
                    &CancelToken::none(),
                    &mut cache.cache,
                )
                .map_err(js_err)?;
            let report_json = contract_render_report_json(&log, &telemetry_report)?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractPngWithRenderReportWithCancellation)]
        pub fn render_contract_png_with_render_report_with_cancellation(
            &self,
            contract_json: &str,
            cancellation: &RenderCancellation,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let (bytes, log, telemetry_report) = self
                .engine
                .render_page_png_with_contract_and_telemetry_report(&contract, &cancellation.token)
                .map_err(js_err)?;
            let report_json = contract_render_report_json(&log, &telemetry_report)?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractObjectPngWithFontSubstitutionReport)]
        pub fn render_contract_object_png_with_font_substitution_report(
            &self,
            contract: &WellfriendRenderContract,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let (bytes, log) = self
                .engine
                .render_page_png_with_contract_and_font_substitution_report(
                    &contract.contract,
                    &CancelToken::none(),
                )
                .map_err(js_err)?;
            let report_json = serde_json::to_string(&log)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractObjectPngWithFontSubstitutionReportWithCancellation)]
        pub fn render_contract_object_png_with_font_substitution_report_with_cancellation(
            &self,
            contract: &WellfriendRenderContract,
            cancellation: &RenderCancellation,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let (bytes, log) = self
                .engine
                .render_page_png_with_contract_and_font_substitution_report(
                    &contract.contract,
                    &cancellation.token,
                )
                .map_err(js_err)?;
            let report_json = serde_json::to_string(&log)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractObjectPngWithRenderReport)]
        pub fn render_contract_object_png_with_render_report(
            &self,
            contract: &WellfriendRenderContract,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let (bytes, log, telemetry_report) = self
                .engine
                .render_page_png_with_contract_and_telemetry_report(
                    &contract.contract,
                    &CancelToken::none(),
                )
                .map_err(js_err)?;
            let report_json = contract_render_report_json(&log, &telemetry_report)?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractObjectPngWithRenderCacheReport)]
        pub fn render_contract_object_png_with_render_cache_report(
            &self,
            contract: &WellfriendRenderContract,
            cache: &mut RenderCache,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let (bytes, log, telemetry_report) = self
                .engine
                .render_page_png_with_contract_and_telemetry_report_and_cache(
                    &contract.contract,
                    &CancelToken::none(),
                    &mut cache.cache,
                )
                .map_err(js_err)?;
            let report_json = contract_render_report_json(&log, &telemetry_report)?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractObjectPngWithRenderReportWithCancellation)]
        pub fn render_contract_object_png_with_render_report_with_cancellation(
            &self,
            contract: &WellfriendRenderContract,
            cancellation: &RenderCancellation,
        ) -> Result<WellfriendOutput, JsValue> {
            self.ensure_open()?;
            let (bytes, log, telemetry_report) = self
                .engine
                .render_page_png_with_contract_and_telemetry_report(
                    &contract.contract,
                    &cancellation.token,
                )
                .map_err(js_err)?;
            let report_json = contract_render_report_json(&log, &telemetry_report)?;
            Ok(WellfriendOutput { bytes, report_json })
        }

        #[wasm_bindgen(js_name = renderContractInto)]
        pub fn render_contract_into(
            &self,
            contract_json: &str,
            output: &mut [u8],
        ) -> Result<(), JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            self.engine
                .render_page_into_buffer(&contract, &CancelToken::none(), output)
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractIntoWithCancellation)]
        pub fn render_contract_into_with_cancellation(
            &self,
            contract_json: &str,
            output: &mut [u8],
            cancellation: &RenderCancellation,
        ) -> Result<(), JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            self.engine
                .render_page_into_buffer(&contract, &cancellation.token, output)
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractObjectInto)]
        pub fn render_contract_object_into(
            &self,
            contract: &WellfriendRenderContract,
            output: &mut [u8],
        ) -> Result<(), JsValue> {
            self.ensure_open()?;
            self.engine
                .render_page_into_buffer(&contract.contract, &CancelToken::none(), output)
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractObjectIntoWithCancellation)]
        pub fn render_contract_object_into_with_cancellation(
            &self,
            contract: &WellfriendRenderContract,
            output: &mut [u8],
            cancellation: &RenderCancellation,
        ) -> Result<(), JsValue> {
            self.ensure_open()?;
            self.engine
                .render_page_into_buffer(&contract.contract, &cancellation.token, output)
                .map_err(js_err)
        }

        #[wasm_bindgen(js_name = renderContractIntoWithFontSubstitutionReport)]
        pub fn render_contract_into_with_font_substitution_report(
            &self,
            contract_json: &str,
            output: &mut [u8],
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let log = self
                .engine
                .render_page_into_buffer_with_font_substitution_report(
                    &contract,
                    &CancelToken::none(),
                    output,
                )
                .map_err(js_err)?;
            serde_json::to_string(&log).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = renderContractIntoWithFontSubstitutionReportWithCancellation)]
        pub fn render_contract_into_with_font_substitution_report_with_cancellation(
            &self,
            contract_json: &str,
            output: &mut [u8],
            cancellation: &RenderCancellation,
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let log = self
                .engine
                .render_page_into_buffer_with_font_substitution_report(
                    &contract,
                    &cancellation.token,
                    output,
                )
                .map_err(js_err)?;
            serde_json::to_string(&log).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = renderContractIntoWithRenderReport)]
        pub fn render_contract_into_with_render_report(
            &self,
            contract_json: &str,
            output: &mut [u8],
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let (log, telemetry_report) = self
                .engine
                .render_page_into_buffer_with_telemetry_report(
                    &contract,
                    &CancelToken::none(),
                    output,
                )
                .map_err(js_err)?;
            contract_render_report_json(&log, &telemetry_report)
        }

        #[wasm_bindgen(js_name = renderContractIntoWithRenderReportWithCancellation)]
        pub fn render_contract_into_with_render_report_with_cancellation(
            &self,
            contract_json: &str,
            output: &mut [u8],
            cancellation: &RenderCancellation,
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let (log, telemetry_report) = self
                .engine
                .render_page_into_buffer_with_telemetry_report(
                    &contract,
                    &cancellation.token,
                    output,
                )
                .map_err(js_err)?;
            contract_render_report_json(&log, &telemetry_report)
        }

        #[wasm_bindgen(js_name = renderContractObjectIntoWithFontSubstitutionReport)]
        pub fn render_contract_object_into_with_font_substitution_report(
            &self,
            contract: &WellfriendRenderContract,
            output: &mut [u8],
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let log = self
                .engine
                .render_page_into_buffer_with_font_substitution_report(
                    &contract.contract,
                    &CancelToken::none(),
                    output,
                )
                .map_err(js_err)?;
            serde_json::to_string(&log).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = renderContractObjectIntoWithFontSubstitutionReportWithCancellation)]
        pub fn render_contract_object_into_with_font_substitution_report_with_cancellation(
            &self,
            contract: &WellfriendRenderContract,
            output: &mut [u8],
            cancellation: &RenderCancellation,
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let log = self
                .engine
                .render_page_into_buffer_with_font_substitution_report(
                    &contract.contract,
                    &cancellation.token,
                    output,
                )
                .map_err(js_err)?;
            serde_json::to_string(&log).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = renderContractObjectIntoWithRenderReport)]
        pub fn render_contract_object_into_with_render_report(
            &self,
            contract: &WellfriendRenderContract,
            output: &mut [u8],
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let (log, telemetry_report) = self
                .engine
                .render_page_into_buffer_with_telemetry_report(
                    &contract.contract,
                    &CancelToken::none(),
                    output,
                )
                .map_err(js_err)?;
            contract_render_report_json(&log, &telemetry_report)
        }

        #[wasm_bindgen(js_name = renderContractObjectIntoWithRenderReportWithCancellation)]
        pub fn render_contract_object_into_with_render_report_with_cancellation(
            &self,
            contract: &WellfriendRenderContract,
            output: &mut [u8],
            cancellation: &RenderCancellation,
        ) -> Result<String, JsValue> {
            self.ensure_open()?;
            let (log, telemetry_report) = self
                .engine
                .render_page_into_buffer_with_telemetry_report(
                    &contract.contract,
                    &cancellation.token,
                    output,
                )
                .map_err(js_err)?;
            contract_render_report_json(&log, &telemetry_report)
        }

        #[wasm_bindgen(js_name = progressiveRenderJob)]
        pub fn progressive_render_job(
            &self,
            page: usize,
            dpi: u32,
            tile_width: u32,
            tile_height: u32,
            mode: Option<String>,
        ) -> Result<ProgressiveRenderJob, JsValue> {
            self.ensure_open()?;
            let mode = mode.unwrap_or_else(|| "compat".to_string());
            let render_mode = wellfriendpdf_engine::RenderMode::from_name(&mode)
                .ok_or_else(|| JsValue::from_str("mode must be compat or high"))?;
            let job = self
                .engine
                .progressive_render_job_with_mode(page, dpi, tile_width, tile_height, render_mode)
                .map_err(js_err)?;
            Ok(ProgressiveRenderJob { job })
        }

        #[wasm_bindgen(js_name = progressiveRenderJobWithContractJson)]
        pub fn progressive_render_job_with_contract_json(
            &self,
            contract_json: &str,
            tile_width: u32,
            tile_height: u32,
        ) -> Result<ProgressiveRenderJob, JsValue> {
            self.ensure_open()?;
            let contract = parse_render_contract_json(contract_json)?;
            let job = self
                .engine
                .progressive_render_job_with_contract(contract, tile_width, tile_height)
                .map_err(js_err)?;
            Ok(ProgressiveRenderJob { job })
        }

        #[wasm_bindgen(js_name = documentInfoJson)]
        pub fn document_info_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::document_info_json(b, None))
        }

        #[wasm_bindgen(js_name = documentViewsReportJson)]
        pub fn document_views_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::document_views_report_json(b, None))
        }

        #[wasm_bindgen(js_name = backendPlanArenaReportJson)]
        pub fn backend_plan_arena_report_json(
            &self,
            page: usize,
            dpi: u32,
            mode: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::backend_plan_arena_report_json(b, page, dpi, mode.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = backendPlanArenaReportForContractJson)]
        pub fn backend_plan_arena_report_for_contract_json(
            &self,
            contract_json: &str,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::backend_plan_arena_report_for_contract_json(b, contract_json, None)
            })
        }

        #[wasm_bindgen(js_name = prepressPlateReportJson)]
        pub fn prepress_plate_report_json(&self, page: usize, dpi: u32) -> Result<String, JsValue> {
            self.report(|b| sdk::prepress_plate_report_json(b, page, dpi, None))
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

        /// Incremental Signing Standards clause-mapped PDF/A validation. `target` e.g. "PDF/A-2B".
        #[wasm_bindgen(js_name = validatePdfaStandardsJson)]
        pub fn validate_pdfa_standards_json(
            &self,
            target: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::pdfa_standards_json(b, target.as_deref(), None))
        }

        /// Incremental Signing Standards clause-mapped PDF/UA validation. `target` e.g. "PDF/UA-1".
        #[wasm_bindgen(js_name = validatePdfuaStandardsJson)]
        pub fn validate_pdfua_standards_json(
            &self,
            target: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::pdfua_standards_json(b, target.as_deref(), None))
        }

        /// Incremental Signing Standards clause-mapped PDF/X validation. `target` e.g. "PDF/X-4".
        #[wasm_bindgen(js_name = validatePdfxStandardsJson)]
        pub fn validate_pdfx_standards_json(
            &self,
            target: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::pdfx_standards_json(b, target.as_deref(), None))
        }

        /// Incremental Signing Standards combined PDF/A + PDF/UA + PDF/X validation with
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

        /// Incremental Signing Standards append-only incremental signing plan (in-memory). `certify`
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

        /// Incremental Signing Standards append-only incremental signing (in-memory). Produces a
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

        #[wasm_bindgen(js_name = annotation_media_redactionReportJson)]
        pub fn annotation_media_redaction_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::annotation_media_redaction_report_json(b, None))
        }

        #[wasm_bindgen(js_name = secure_mutationReportJson)]
        pub fn secure_mutation_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::secure_mutation_report_json(b, None))
        }

        #[wasm_bindgen(js_name = secure_mutation_closeoutReportJson)]
        pub fn secure_mutation_closeout_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::secure_mutation_closeout_report_json(b, None))
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

        #[wasm_bindgen(js_name = form_action_policyReportJson)]
        pub fn form_action_policy_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::form_action_policy_report_json(b, None))
        }

        #[wasm_bindgen(js_name = advanced_editingReportJson)]
        pub fn advanced_editing_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::advanced_editing_report_json(b, None))
        }

        #[wasm_bindgen(js_name = advanced_editing_closeoutReportJson)]
        pub fn advanced_editing_closeout_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::advanced_editing_closeout_report_json(b, None))
        }

        #[wasm_bindgen(js_name = source_editingReportJson)]
        pub fn source_editing_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::source_editing_report_json(b, None))
        }

        #[wasm_bindgen(js_name = editing_transactionsReportJson)]
        pub fn editing_transactions_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::editing_transactions_report_json(b, None))
        }

        #[wasm_bindgen(js_name = writer_historyReportJson)]
        pub fn writer_history_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::writer_history_report_json(b, None))
        }

        #[wasm_bindgen(js_name = compression_officeReportJson)]
        pub fn compression_office_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::compression_office_report_json(b, None))
        }

        #[wasm_bindgen(js_name = crypto_writerReportJson)]
        pub fn crypto_writer_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::crypto_writer_report_json(b, None))
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

        #[wasm_bindgen(js_name = writer_historyRasterVectorReportJson)]
        pub fn writer_history_raster_vector_report_json(
            &self,
            page: usize,
            options_json: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::writer_history_raster_vector_report_json(
                    b,
                    page,
                    options_json.as_deref(),
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = writer_historyFontReconstructionReportJson)]
        pub fn writer_history_font_reconstruction_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::writer_history_font_reconstruction_report_json(b, None))
        }

        #[wasm_bindgen(js_name = writer_historyObjectStreamReportJson)]
        pub fn writer_history_object_stream_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::writer_history_object_stream_report_json(b, None))
        }

        #[wasm_bindgen(js_name = advanced_editing_closeoutTextRangeAnalyzeJson)]
        pub fn advanced_editing_closeout_text_range_analyze_json(
            &self,
            page: usize,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::advanced_editing_closeout_text_range_analyze_json(b, page, None))
        }

        #[wasm_bindgen(js_name = source_editingProvenanceJson)]
        pub fn source_editing_provenance_json(
            &self,
            page: usize,
            source_text: String,
            replacement_text: String,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::source_editing_provenance_json(b, page, &source_text, &replacement_text, None)
            })
        }

        #[wasm_bindgen(js_name = source_editingEditEligibilityJson)]
        pub fn source_editing_edit_eligibility_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::source_editing_edit_eligibility_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = source_editingPathProvenanceJson)]
        pub fn source_editing_path_provenance_json(&self, page: usize) -> Result<String, JsValue> {
            self.report(|b| sdk::source_editing_path_provenance_json(b, page, None))
        }

        #[wasm_bindgen(js_name = source_editingImageEligibilityJson)]
        pub fn source_editing_image_eligibility_json(
            &self,
            page: usize,
            occurrence: Option<String>,
        ) -> Result<String, JsValue> {
            let _ = occurrence.as_deref();
            self.report(|b| sdk::source_editing_image_eligibility_json(b, page, None))
        }

        #[wasm_bindgen(js_name = editing_transactionsSceneReportJson)]
        pub fn editing_transactions_scene_report_json(
            &self,
            pages_json: Option<String>,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::editing_transactions_scene_report_json(b, pages_json.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = editing_transactionsSceneSelectJson)]
        pub fn editing_transactions_scene_select_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::editing_transactions_scene_select_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = editing_transactionsTransactionPlanJson)]
        pub fn editing_transactions_transaction_plan_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::editing_transactions_transaction_plan_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = editing_transactionsTextMapJson)]
        pub fn editing_transactions_text_map_json(
            &self,
            text: String,
            direction: Option<String>,
        ) -> Result<String, JsValue> {
            sdk::editing_transactions_text_map_json(&text, direction.as_deref()).map_err(js_err)
        }

        #[wasm_bindgen(js_name = editing_transactionsShapeTextJson)]
        pub fn editing_transactions_shape_text_json(
            &self,
            text: String,
            direction: Option<String>,
        ) -> Result<String, JsValue> {
            sdk::editing_transactions_shape_text_json(&text, direction.as_deref()).map_err(js_err)
        }

        #[wasm_bindgen(js_name = editing_transactionsFontSubsetPlanJson)]
        pub fn editing_transactions_font_subset_plan_json(
            &self,
            text: String,
            direction: Option<String>,
            policy: Option<String>,
        ) -> Result<String, JsValue> {
            sdk::editing_transactions_font_subset_plan_json(
                &text,
                direction.as_deref(),
                policy.as_deref(),
            )
            .map_err(js_err)
        }

        #[wasm_bindgen(js_name = editing_transactionsFontSubstitutionReportJson)]
        pub fn editing_transactions_font_substitution_report_json(
            &self,
            requested_family: String,
            text: String,
            policy: Option<String>,
        ) -> Result<String, JsValue> {
            sdk::editing_transactions_font_substitution_report_json(
                &requested_family,
                &text,
                policy.as_deref(),
            )
            .map_err(js_err)
        }

        #[wasm_bindgen(js_name = text_reflowReportJson)]
        pub fn text_reflow_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_report_json(b, None))
        }

        #[wasm_bindgen(js_name = text_reflowLayoutAnalyzeJson)]
        pub fn text_reflow_layout_analyze_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_layout_analyze_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = text_reflowSemanticLayoutJson)]
        pub fn text_reflow_semantic_layout_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_semantic_layout_json(b, None))
        }

        #[wasm_bindgen(js_name = text_reflowReadingOrderReportJson)]
        pub fn text_reflow_reading_order_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_reading_order_report_json(b, None))
        }

        #[wasm_bindgen(js_name = text_reflowFlowGraphReportJson)]
        pub fn text_reflow_flow_graph_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_flow_graph_report_json(b, None))
        }

        #[wasm_bindgen(js_name = text_reflowReflowPreviewJson)]
        pub fn text_reflow_reflow_preview_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_reflow_preview_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = text_reflowOverflowReportJson)]
        pub fn text_reflow_overflow_report_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_overflow_report_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = text_reflowConstraintsReportJson)]
        pub fn text_reflow_constraints_report_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_constraints_report_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = text_reflowConfidenceReportJson)]
        pub fn text_reflow_confidence_report_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_confidence_report_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = text_reflowValidateReflowOutputJson)]
        pub fn text_reflow_validate_reflow_output_json(
            &self,
            output_pdf: Vec<u8>,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| {
                sdk::text_reflow_validate_reflow_output_json(b, &output_pdf, &request_json, None)
            })
        }

        #[wasm_bindgen(js_name = text_reflowReflowOperationReportJson)]
        pub fn text_reflow_reflow_operation_report_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::text_reflow_reflow_operation_report_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = document_subsystemsReportJson)]
        pub fn document_subsystems_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::document_subsystems_report_json(b, None))
        }

        #[wasm_bindgen(js_name = document_subsystemsAnalyzeJson)]
        pub fn document_subsystems_analyze_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::document_subsystems_analyze_json(b, None))
        }

        #[wasm_bindgen(js_name = document_subsystemsPlanJson)]
        pub fn document_subsystems_plan_json(
            &self,
            request_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::document_subsystems_plan_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = document_securityReportJson)]
        pub fn document_security_report_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::document_security_report_json(b, None))
        }

        #[wasm_bindgen(js_name = document_securityAnalyzeJson)]
        pub fn document_security_analyze_json(&self) -> Result<String, JsValue> {
            self.report(|b| sdk::document_security_analyze_json(b, None))
        }

        #[wasm_bindgen(js_name = document_securityPlanJson)]
        pub fn document_security_plan_json(&self, request_json: String) -> Result<String, JsValue> {
            self.report(|b| sdk::document_security_plan_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = document_securityVerifyResidualJson)]
        pub fn document_security_verify_residual_json(
            &self,
            terms_json: String,
        ) -> Result<String, JsValue> {
            self.report(|b| sdk::document_security_verify_residual_json(b, &terms_json, None))
        }

        #[wasm_bindgen(js_name = advanced_editingVectorListJson)]
        pub fn advanced_editing_vector_list_json(&self, page: usize) -> Result<String, JsValue> {
            self.report(|b| sdk::advanced_editing_vector_list_json(b, page, None))
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

        /// Offline Signature Validation validation with owned caller-supplied trust and
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

        /// Offline Signature Validation validation plus a portable, hash-checked evidence
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

        #[wasm_bindgen(js_name = advanced_editingTextEdit)]
        pub fn advanced_editing_text_edit(
            &self,
            page: usize,
            old_text: &str,
            new_text: &str,
            mode: &str,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::advanced_editing_text_edit_json(
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

        #[wasm_bindgen(js_name = source_editingOperatorTextEdit)]
        pub fn source_editing_operator_text_edit(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::source_editing_operator_text_edit_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = editing_transactionsTransactionApply)]
        pub fn editing_transactions_transaction_apply(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::editing_transactions_transaction_apply_json(b, &request_json, None)
            })
        }

        #[wasm_bindgen(js_name = editing_transactionsTransactionApplyWithRenderInvalidation)]
        pub fn editing_transactions_transaction_apply_with_render_invalidation(
            &self,
            request_json: String,
            render_invalidation_options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::editing_transactions_transaction_apply_with_render_invalidation_json(
                    b,
                    &request_json,
                    render_invalidation_options_json.as_deref(),
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = editing_transactionsSceneEditText)]
        pub fn editing_transactions_scene_edit_text(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::editing_transactions_scene_edit_text_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = text_reflowReflowRegion)]
        pub fn text_reflow_reflow_region(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::text_reflow_reflow_region_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = text_reflowReflowDocument)]
        pub fn text_reflow_reflow_document(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::text_reflow_reflow_document_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = text_reflowUndoReflow)]
        pub fn text_reflow_undo_reflow(
            &self,
            output_pdf: Vec<u8>,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::text_reflow_undo_reflow_json(b, &output_pdf, &request_json, None))
        }

        #[wasm_bindgen(js_name = document_subsystemsApply)]
        pub fn document_subsystems_apply(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::document_subsystems_apply_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = document_subsystemsUndo)]
        pub fn document_subsystems_undo(
            &self,
            output_pdf: Vec<u8>,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::document_subsystems_undo_json(b, &output_pdf, &request_json, None))
        }

        #[wasm_bindgen(js_name = document_securityApply)]
        pub fn document_security_apply(
            &self,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::document_security_apply_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = document_securityUndo)]
        pub fn document_security_undo(
            &self,
            output_pdf: Vec<u8>,
            request_json: String,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::document_security_undo_json(b, &output_pdf, &request_json, None))
        }

        #[wasm_bindgen(js_name = source_editingPathEdit)]
        pub fn source_editing_path_edit(
            &self,
            page: usize,
            stable_id: &str,
            operation_json: &str,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::source_editing_path_edit_json(
                    b,
                    page,
                    stable_id,
                    operation_json,
                    options_json.as_deref(),
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = advanced_editingVectorEdit)]
        pub fn advanced_editing_vector_edit(
            &self,
            page: usize,
            stable_id: &str,
            operation_json: &str,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::advanced_editing_vector_edit_json(
                    b,
                    page,
                    stable_id,
                    operation_json,
                    options_json.as_deref(),
                    None,
                )
            })
        }

        #[wasm_bindgen(js_name = advanced_editingInkFit)]
        pub fn advanced_editing_ink_fit(
            &self,
            page: usize,
            annotation_index: usize,
            options_json: Option<String>,
            signature_policy_override: bool,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::advanced_editing_ink_fit_json(
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
            self.output(|b| {
                sdk::advanced_editing_closeout_text_range_edit_json(b, &request_json, None)
            })
        }

        #[wasm_bindgen(js_name = writer_historyPackObjectStreams)]
        pub fn writer_history_pack_object_streams(&self) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| sdk::writer_history_pack_object_streams_json(b, None))
        }

        #[wasm_bindgen(js_name = compression_officeOptimize)]
        pub fn compression_office_optimize(
            &self,
            options_json: Option<String>,
        ) -> Result<WellfriendOutput, JsValue> {
            self.output(|b| {
                sdk::compression_office_optimize_pdf_json(b, options_json.as_deref(), None)
            })
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

    #[wasm_bindgen]
    impl ProgressiveRenderJob {
        #[wasm_bindgen(js_name = stateJson)]
        pub fn state_json(&self) -> Result<String, JsValue> {
            serde_json::to_string(&self.job.state())
                .map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = step)]
        pub fn step(&mut self, max_tiles: usize) -> Result<String, JsValue> {
            let report = self
                .job
                .render_next(max_tiles, &CancelToken::none())
                .map_err(js_err)?;
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = stepWithCancellation)]
        pub fn step_with_cancellation(
            &mut self,
            max_tiles: usize,
            cancellation: JsValue,
        ) -> Result<String, JsValue> {
            if wasm_cancellation_requested(&cancellation) {
                self.job.request_cancel();
                return Err(JsValue::from_str(
                    "Wellfriend progressive render step was cancelled",
                ));
            }
            let report = self.step(max_tiles)?;
            if wasm_cancellation_requested(&cancellation) {
                self.job.request_cancel();
                return Err(JsValue::from_str(
                    "Wellfriend progressive render step was cancelled",
                ));
            }
            Ok(report)
        }

        #[wasm_bindgen(js_name = tokenJson)]
        pub fn token_json(&self) -> Result<String, JsValue> {
            serde_json::to_string(&self.job.token())
                .map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = pauseJson)]
        pub fn pause_json(&mut self) -> Result<String, JsValue> {
            let token = self.job.pause().map_err(js_err)?;
            serde_json::to_string(&token).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = resumeJson)]
        pub fn resume_json(&mut self, token_json: &str) -> Result<(), JsValue> {
            let token: wellfriendpdf_engine::ProgressiveRenderToken =
                serde_json::from_str(token_json).map_err(|error| {
                    JsValue::from_str(&format!("progressive token JSON: {error}"))
                })?;
            self.job.resume(&token).map_err(js_err)
        }

        #[wasm_bindgen(js_name = cancel)]
        pub fn cancel(&mut self) {
            self.job.cancel();
        }

        #[wasm_bindgen(js_name = requestCancel)]
        pub fn request_cancel(&self) {
            self.job.request_cancel();
        }

        #[wasm_bindgen(js_name = reviseViewportHintJson)]
        pub fn revise_viewport_hint_json(
            &mut self,
            hint_present: bool,
            x: u32,
            y: u32,
            width: u32,
            height: u32,
        ) -> Result<String, JsValue> {
            let viewport_hint = hint_present.then_some(wellfriendpdf_engine::RenderTile {
                x,
                y,
                width,
                height,
            });
            let report = self
                .job
                .revise_viewport_hint(viewport_hint)
                .map_err(js_err)?;
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = reviseDirtyRegionJson)]
        pub fn revise_dirty_region_json(
            &mut self,
            dirty_present: bool,
            x: u32,
            y: u32,
            width: u32,
            height: u32,
        ) -> Result<String, JsValue> {
            let dirty_region = dirty_present.then_some(wellfriendpdf_engine::RenderTile {
                x,
                y,
                width,
                height,
            });
            let report = self.job.revise_dirty_region(dirty_region).map_err(js_err)?;
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = reviseRenderContextJson)]
        pub fn revise_render_context_json(
            &mut self,
            render_contract_fingerprint: Option<String>,
            visibility_fingerprint: Option<String>,
        ) -> Result<String, JsValue> {
            let report = self
                .job
                .revise_render_context(render_contract_fingerprint, visibility_fingerprint)
                .map_err(js_err)?;
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = reviseRenderContractJson)]
        pub fn revise_render_contract_json(
            &mut self,
            contract_json: &str,
        ) -> Result<String, JsValue> {
            let contract: wellfriendpdf_engine::RenderContract =
                serde_json::from_str(contract_json).map_err(|error| {
                    JsValue::from_str(&format!("progressive render contract JSON: {error}"))
                })?;
            let report = self.job.revise_render_contract(contract).map_err(js_err)?;
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = applyRenderInvalidationPlanJson)]
        pub fn apply_render_invalidation_plan_json(
            &mut self,
            plan_json: &str,
        ) -> Result<String, JsValue> {
            let report = self
                .job
                .apply_render_invalidation_plan_json(plan_json)
                .map_err(js_err)?;
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = evaluateTilePublicationJson)]
        pub fn evaluate_tile_publication_json(
            &self,
            publication_json: &str,
        ) -> Result<String, JsValue> {
            let publication: wellfriendpdf_engine::ProgressiveTilePublication =
                serde_json::from_str(publication_json).map_err(|error| {
                    JsValue::from_str(&format!("progressive tile publication JSON: {error}"))
                })?;
            let report = self.job.evaluate_tile_publication(&publication);
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = viewerQueueJson)]
        pub fn viewer_queue_json(&self) -> Result<String, JsValue> {
            let report = self.job.viewer_queue_report();
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = executeViewerQueueJson)]
        pub fn execute_viewer_queue_json(&mut self, max_items: usize) -> Result<String, JsValue> {
            let report = self
                .job
                .execute_viewer_queue(max_items, &CancelToken::none())
                .map_err(js_err)?;
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = executeViewerQueueJsonWithCancellation)]
        pub fn execute_viewer_queue_json_with_cancellation(
            &mut self,
            max_items: usize,
            cancellation: &RenderCancellation,
        ) -> Result<String, JsValue> {
            let report = self
                .job
                .execute_viewer_queue(max_items, &cancellation.token)
                .map_err(js_err)?;
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = executeAdjacentPagePrefetch)]
        pub fn execute_adjacent_page_prefetch(
            &self,
            prefetch_identity: &str,
            max_tiles: usize,
        ) -> Result<AdjacentPagePrefetchExecution, JsValue> {
            let execution = self
                .job
                .execute_adjacent_page_prefetch(prefetch_identity, max_tiles, &CancelToken::none())
                .map_err(js_err)?;
            let report_json = serde_json::to_string(&execution.report)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            Ok(AdjacentPagePrefetchExecution {
                report_json,
                job: execution.job.map(|job| ProgressiveRenderJob { job }),
            })
        }

        #[wasm_bindgen(js_name = executeAdjacentPagePrefetchWithCancellation)]
        pub fn execute_adjacent_page_prefetch_with_cancellation(
            &self,
            prefetch_identity: &str,
            max_tiles: usize,
            cancellation: &RenderCancellation,
        ) -> Result<AdjacentPagePrefetchExecution, JsValue> {
            let execution = self
                .job
                .execute_adjacent_page_prefetch(prefetch_identity, max_tiles, &cancellation.token)
                .map_err(js_err)?;
            let report_json = serde_json::to_string(&execution.report)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            Ok(AdjacentPagePrefetchExecution {
                report_json,
                job: execution.job.map(|job| ProgressiveRenderJob { job }),
            })
        }

        #[wasm_bindgen(js_name = viewerCallbackDispatchJson)]
        pub fn viewer_callback_dispatch_json(&self) -> Result<String, JsValue> {
            let report = self.job.viewer_callback_dispatch_report();
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = dispatchViewerCallbacks)]
        pub fn dispatch_viewer_callbacks(&self, callback: &Function) -> Result<String, JsValue> {
            let report = self.job.viewer_callback_dispatch_report();
            for event in &report.events {
                let event_json = serde_json::to_string(event)
                    .map_err(|error| JsValue::from_str(&error.to_string()))?;
                callback.call1(&JsValue::NULL, &JsValue::from_str(&event_json))?;
            }
            serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
        }

        #[wasm_bindgen(js_name = finishPng)]
        pub fn finish_png(&self) -> Result<Vec<u8>, JsValue> {
            let buffer = self.job.finish_checked().map_err(js_err)?;
            wellfriendpdf_engine::images::encoder::ImageEncoder::encode_png_fast(
                &buffer.to_raw_image(),
            )
            .map_err(js_err)
        }

        #[wasm_bindgen(js_name = finishPngWithCancellation)]
        pub fn finish_png_with_cancellation(
            &self,
            cancellation: JsValue,
        ) -> Result<Vec<u8>, JsValue> {
            if wasm_cancellation_requested(&cancellation) {
                self.job.request_cancel();
                return Err(JsValue::from_str(
                    "Wellfriend progressive render finish was cancelled",
                ));
            }
            let png = self.finish_png()?;
            if wasm_cancellation_requested(&cancellation) {
                self.job.request_cancel();
                return Err(JsValue::from_str(
                    "Wellfriend progressive render finish was cancelled",
                ));
            }
            Ok(png)
        }

        #[wasm_bindgen(js_name = close)]
        pub fn close(&mut self) {
            self.job.close();
        }
    }

    fn wasm_cancellation_requested(cancellation: &JsValue) -> bool {
        cancellation.as_bool().unwrap_or_else(|| {
            Reflect::get(cancellation, &JsValue::from_str("aborted"))
                .ok()
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        })
    }

    fn js_err(err: wellfriendpdf_engine::WellfriendError) -> JsValue {
        JsValue::from_str(&err.to_string())
    }

    fn contract_render_report_json(
        log: &wellfriendpdf_engine::FontSubstitutionLog,
        telemetry_report: &wellfriendpdf_engine::RenderContractTelemetryReport,
    ) -> Result<String, JsValue> {
        let report = serde_json::json!({
            "font_substitution_report": log,
            "render_telemetry_report": telemetry_report,
        });
        serde_json::to_string(&report).map_err(|error| JsValue::from_str(&error.to_string()))
    }

    fn parse_render_contract_json(json: &str) -> Result<RenderContract, JsValue> {
        let contract: RenderContract = serde_json::from_str(json)
            .map_err(|error| JsValue::from_str(&format!("render contract JSON: {error}")))?;
        contract
            .validate()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(contract)
    }

    fn validate_render_contract(
        contract: RenderContract,
    ) -> Result<WellfriendRenderContract, JsValue> {
        contract
            .validate()
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(WellfriendRenderContract { contract })
    }

    fn parse_pixel_format(value: Option<&str>) -> Result<PixelFormat, JsValue> {
        match value.unwrap_or("Rgba8") {
            "Rgba8" | "rgba8" | "rgba" => Ok(PixelFormat::Rgba8),
            "Bgra8" | "bgra8" | "bgra" => Ok(PixelFormat::Bgra8),
            "Rgb8" | "rgb8" | "rgb" => Ok(PixelFormat::Rgb8),
            "Bgr8" | "bgr8" | "bgr" => Ok(PixelFormat::Bgr8),
            "Gray8" | "gray8" | "gray" | "grey8" | "grey" => Ok(PixelFormat::Gray8),
            other => Err(JsValue::from_str(&format!(
                "unsupported render contract pixel_format '{other}'"
            ))),
        }
    }

    fn parse_alpha_mode(value: Option<&str>) -> Result<AlphaMode, JsValue> {
        match value.unwrap_or("Premultiplied") {
            "Premultiplied" | "premultiplied" => Ok(AlphaMode::Premultiplied),
            "Straight" | "straight" => Ok(AlphaMode::Straight),
            "Opaque" | "opaque" => Ok(AlphaMode::Opaque),
            other => Err(JsValue::from_str(&format!(
                "unsupported render contract alpha_mode '{other}'"
            ))),
        }
    }

    fn parse_contract_enum<T>(field: &str, value: &str) -> Result<T, JsValue>
    where
        T: DeserializeOwned,
    {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(JsValue::from_str(&format!(
                "render contract {field} must be present"
            )));
        }
        let canonical = canonical_contract_enum_name(trimmed);
        for candidate in [trimmed, canonical.as_str()] {
            if let Ok(parsed) =
                serde_json::from_value(serde_json::Value::String(candidate.to_string()))
            {
                return Ok(parsed);
            }
        }
        Err(JsValue::from_str(&format!(
            "unsupported render contract {field} '{value}'"
        )))
    }

    fn canonical_contract_enum_name(value: &str) -> String {
        let mut out = String::new();
        let mut uppercase_next = true;
        for ch in value.chars() {
            if ch.is_ascii_alphanumeric() {
                if uppercase_next {
                    out.push(ch.to_ascii_uppercase());
                    uppercase_next = false;
                } else {
                    out.push(ch);
                }
            } else {
                uppercase_next = true;
            }
        }
        out
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
