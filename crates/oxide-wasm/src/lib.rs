//! wasm-bindgen wrapper for `oxide-engine`.
//!
//! The browser/Node/WebWorker surface accepts caller-provided bytes only. It
//! does not fetch URLs, read host files implicitly, or execute PDF active
//! content. Reports are routed through `oxide_engine::sdk` so the JSON envelope
//! matches Rust, Python, and the C ABI.

#[cfg(target_arch = "wasm32")]
mod wasm_api {
    use wasm_bindgen::prelude::*;

    use oxide_engine::{sdk, ChunkOptions, ContentEngine, DocType, ExtractOptions, ParseOptions};

    #[wasm_bindgen]
    pub struct OxidePdf {
        engine: ContentEngine,
        bytes: Vec<u8>,
        closed: bool,
    }

    #[wasm_bindgen]
    pub struct OxideOutput {
        bytes: Vec<u8>,
        report_json: String,
    }

    #[wasm_bindgen]
    impl OxideOutput {
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
    impl OxidePdf {
        #[wasm_bindgen(constructor)]
        pub fn new(bytes: &[u8]) -> Result<OxidePdf, JsValue> {
            install_panic_hook();
            let engine = ContentEngine::open_bytes(bytes.to_vec()).map_err(js_err)?;
            Ok(Self {
                engine,
                bytes: bytes.to_vec(),
                closed: false,
            })
        }

        #[wasm_bindgen(js_name = openWithPassword)]
        pub fn open_with_password(bytes: &[u8], password: &[u8]) -> Result<OxidePdf, JsValue> {
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
            oxide_engine::ENGINE_VERSION.to_string()
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

        #[wasm_bindgen(js_name = prompt22OfficeInspectJson)]
        pub fn prompt22_office_inspect_json(bytes: &[u8], format: &str) -> Result<String, JsValue> {
            install_panic_hook();
            sdk::prompt22_office_inspect_json(bytes, format).map_err(js_err)
        }

        #[wasm_bindgen(js_name = prompt22OfficeToPdf)]
        pub fn prompt22_office_to_pdf(bytes: &[u8], format: &str) -> Result<OxideOutput, JsValue> {
            install_panic_hook();
            let (out, report) = sdk::prompt22_office_to_pdf_json(bytes, format).map_err(js_err)?;
            Ok(OxideOutput {
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
        ) -> Result<OxideOutput, JsValue> {
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

        #[wasm_bindgen(js_name = prompt20VectorEdit)]
        pub fn prompt20_vector_edit(
            &self,
            page: usize,
            stable_id: &str,
            operation_json: &str,
            options_json: Option<String>,
        ) -> Result<OxideOutput, JsValue> {
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
        ) -> Result<OxideOutput, JsValue> {
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
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| {
                sdk::xfa_render_preview_json(b, script_policy.as_deref(), execute_events, dpi, None)
            })
        }

        #[wasm_bindgen(js_name = xfaFlatten)]
        pub fn xfa_flatten(&self, mode: Option<String>) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::xfa_flatten_json(b, mode.as_deref(), None))
        }

        #[wasm_bindgen(js_name = xfaSanitize)]
        pub fn xfa_sanitize(&self, mode: Option<String>) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::xfa_sanitize_json(b, mode.as_deref(), None))
        }

        #[wasm_bindgen(js_name = annotationXfdfExport)]
        pub fn annotation_xfdf_export(&self) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::annotation_xfdf_export_json(b, None))
        }

        #[wasm_bindgen(js_name = annotationXfdfImport)]
        pub fn annotation_xfdf_import(
            &self,
            xfdf: &[u8],
            options_json: Option<String>,
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| {
                sdk::annotation_xfdf_import_json(b, xfdf, options_json.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = annotationAppearanceGenerate)]
        pub fn annotation_appearance_generate(
            &self,
            options_json: Option<String>,
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| {
                sdk::annotation_appearance_generate_json(b, options_json.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = editTextRange)]
        pub fn edit_text_range(&self, request_json: String) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::prompt20b_text_range_edit_json(b, &request_json, None))
        }

        #[wasm_bindgen(js_name = prompt21PackObjectStreams)]
        pub fn prompt21_pack_object_streams(&self) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::prompt21_pack_object_streams_json(b, None))
        }

        #[wasm_bindgen(js_name = prompt22Optimize)]
        pub fn prompt22_optimize(
            &self,
            options_json: Option<String>,
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::prompt22_optimize_pdf_json(b, options_json.as_deref(), None))
        }

        #[wasm_bindgen(js_name = richMediaSanitize)]
        pub fn rich_media_sanitize(
            &self,
            mode: Option<String>,
            custom_json: Option<String>,
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| {
                sdk::rich_media_sanitize_json(b, mode.as_deref(), custom_json.as_deref(), None)
            })
        }

        #[wasm_bindgen(js_name = richMediaFlattenPoster)]
        pub fn rich_media_flatten_poster(&self) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::rich_media_flatten_poster_json(b, None))
        }

        #[wasm_bindgen(js_name = redactImageNonaxis)]
        pub fn redact_image_nonaxis(&self, options_json: &str) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::nonaxis_redaction_apply_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = redactImageMask)]
        pub fn redact_image_mask(&self, options_json: &str) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::redact_image_mask_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = redactInlineImage)]
        pub fn redact_inline_image(&self, options_json: &str) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::redact_inline_image_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = associatedFileAdd)]
        pub fn associated_file_add(
            &self,
            payload: &[u8],
            options_json: &str,
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::associated_files_add_json(b, payload, options_json, None))
        }

        #[wasm_bindgen(js_name = associatedFileUpdateOwner)]
        pub fn associated_file_update_owner(
            &self,
            payload: &[u8],
            options_json: &str,
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::associated_files_update_owner_json(b, payload, options_json, None))
        }

        #[wasm_bindgen(js_name = associatedFileRemoveOwner)]
        pub fn associated_file_remove_owner(
            &self,
            options_json: &str,
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::associated_files_remove_owner_json(b, options_json, None))
        }

        #[wasm_bindgen(js_name = incrementalFormEdit)]
        pub fn incremental_form_edit(
            &self,
            field_name: &str,
            value: &str,
            signature_policy_override: bool,
        ) -> Result<OxideOutput, JsValue> {
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
        ) -> Result<OxideOutput, JsValue> {
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
        ) -> Result<OxideOutput, JsValue> {
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
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::associated_files_sanitize_json(b, options_json.as_deref(), None))
        }

        #[wasm_bindgen(js_name = formJsSanitize)]
        pub fn form_js_sanitize(
            &self,
            options_json: Option<String>,
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::form_js_sanitize_json(b, options_json.as_deref(), None))
        }

        #[wasm_bindgen(js_name = formJsFlattenValues)]
        pub fn form_js_flatten_values(
            &self,
            options_json: Option<String>,
        ) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::form_js_flatten_values_json(b, options_json.as_deref(), None))
        }

        #[wasm_bindgen(js_name = associatedFilesRemove)]
        pub fn associated_files_remove(
            &self,
            stable_ids_json: &str,
        ) -> Result<OxideOutput, JsValue> {
            let stable_ids: Vec<String> = serde_json::from_str(stable_ids_json)
                .map_err(|error| JsValue::from_str(&error.to_string()))?;
            self.output(|b| sdk::associated_files_remove_json(b, &stable_ids, None))
        }

        #[wasm_bindgen(js_name = sanitize)]
        pub fn sanitize(&self, policy: Option<String>) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::sanitize_json(b, policy.as_deref(), None))
        }

        #[wasm_bindgen(js_name = canonicalize)]
        pub fn canonicalize(&self, date_epoch: Option<i64>) -> Result<OxideOutput, JsValue> {
            self.output(|b| sdk::canonicalize_json(b, date_epoch, None))
        }

        #[wasm_bindgen(js_name = redactTermsJson)]
        pub fn redact_terms_json(
            &self,
            terms_json: &str,
            strict: bool,
        ) -> Result<OxideOutput, JsValue> {
            let terms: Vec<String> = serde_json::from_str(terms_json)
                .map_err(|err| JsValue::from_str(&err.to_string()))?;
            self.output(|b| sdk::redact_terms_json(b, &terms, strict, None))
        }

        fn ensure_open(&self) -> Result<(), JsValue> {
            if self.closed {
                Err(JsValue::from_str(
                    "OxidePdf document is closed; create a new instance before calling this method",
                ))
            } else {
                Ok(())
            }
        }

        fn report<F>(&self, f: F) -> Result<String, JsValue>
        where
            F: FnOnce(&[u8]) -> oxide_engine::Result<String>,
        {
            self.ensure_open()?;
            f(&self.bytes).map_err(js_err)
        }

        fn output<F>(&self, f: F) -> Result<OxideOutput, JsValue>
        where
            F: FnOnce(&[u8]) -> oxide_engine::Result<(Vec<u8>, String)>,
        {
            self.ensure_open()?;
            let (bytes, report_json) = f(&self.bytes).map_err(js_err)?;
            Ok(OxideOutput { bytes, report_json })
        }
    }

    fn js_err(err: oxide_engine::OxideError) -> JsValue {
        JsValue::from_str(&err.to_string())
    }

    fn install_panic_hook() {
        #[cfg(feature = "panic-hook")]
        console_error_panic_hook::set_once();
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct OxideWasmBuildsOnlyForWasm32;
