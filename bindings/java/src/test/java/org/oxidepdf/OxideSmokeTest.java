package org.oxidepdf;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.Map;

public final class OxideSmokeTest {
    public static void main(String[] args) throws Exception {
        Path fixture = fixturePath();
        Path prompt15Fixture = locateFixture("multi_stream.pdf");
        try (Oxide.Document doc = Oxide.Document.open(fixture)) {
            assertTrue(doc.pageCount() >= 1, "page count");
            assertTrue(!doc.page(1).text().isBlank(), "text extraction");
            assertTrue(doc.parseJson().contains("\"schema_version\""), "parse json");
            Map<String, String> reports = new LinkedHashMap<>();
            reports.put("feature", Oxide.featureReportJson());
            reports.put("security", doc.securityReportJson());
            reports.put("parser", doc.parserReportJson("repair"));
            reports.put("color", doc.colorReportJson("generic"));
            reports.put("validate_security", doc.validateJson("security"));
            reports.put("pdfa_standards", doc.validatePdfaStandardsJson("PDF/A-2B"));
            reports.put("pdfua_standards", doc.validatePdfuaStandardsJson("PDF/UA-1"));
            reports.put("pdfx_standards", doc.validatePdfxStandardsJson("PDF/X-4"));
            reports.put("standards_all", doc.validateAllStandardsJson());
            reports.put("docmdp_permissions", doc.docMdpPermissionReportJson());
            reports.put("fieldmdp_permissions", doc.fieldMdpPermissionReportJson());
            assertTrue(reports.get("pdfa_standards").contains("\"kind\":\"pdfa_standards_validation\""),
                "Prompt26 PDF/A standards envelope");
            assertTrue(reports.get("pdfua_standards").contains("\"kind\":\"pdfua_standards_validation\""),
                "Prompt26 PDF/UA standards envelope");
            assertTrue(reports.get("pdfx_standards").contains("\"kind\":\"pdfx_standards_validation\""),
                "Prompt26 PDF/X standards envelope");
            assertTrue(reports.get("standards_all").contains("\"kind\":\"standards_all_validation\""),
                "Prompt26 combined standards envelope");
            reports.put("forms", doc.formsReportJson());
            reports.put("xfa", doc.xfaReportJson());
            reports.put("xfa_extract", doc.xfaExtractJson());
            reports.put("xfa_script", doc.xfaScriptReportJson());
            reports.put("xfa_security", doc.xfaSecurityReportJson());
            reports.put("xfa_runtime", doc.xfaRuntimeReportJson("disabled", false));
            reports.put("annotations", doc.annotationsReportJson());
            reports.put("rich_media", doc.richMediaReportJson());
            reports.put("annotation_appearance", doc.annotationAppearanceReportJson(null));
            reports.put("prompt17", doc.prompt17ReportJson());
            reports.put("prompt18", doc.prompt18ReportJson());
            reports.put("prompt18b", doc.prompt18bReportJson());
            reports.put("form_js", doc.formJavaScriptReportJson());
            reports.put("form_action_graph", doc.formActionGraphJson());
            reports.put("interactive_data", doc.interactiveDataReportJson());
            reports.put("word_pagination", doc.wordPaginationAuditJson("page-faithful"));
            reports.put("prompt19", doc.prompt19ReportJson());
            reports.put("prompt20", doc.prompt20ReportJson());
            reports.put("prompt20b", doc.prompt20bReportJson());
            reports.put("associated_files", doc.associatedFilesReportJson());
            reports.put("edit_policy", doc.editPolicyReportJson("incremental_save"));
            reports.put("pages", doc.pagesReportJson());
            reports.put("interactive", doc.interactiveReportJson());
            reports.put("chunks", doc.chunksJson());
            try (Oxide.Document prompt15 = Oxide.Document.open(prompt15Fixture)) {
                reports.put("advanced_chunks", prompt15.advancedChunksJson());
                reports.put("semantic_bundle", prompt15.semanticBundleJson());
                reports.put("semantic_search", prompt15.semanticSearchJson("Hello"));
                String rangeModel = prompt15.prompt20bTextRangeAnalyzeJson(1);
                reports.put("prompt20b_range_model", rangeModel);
                assertTrue(rangeModel.contains("prompt20b_multi_run_range_model"), "Prompt20B range model");
                Oxide.BinaryResult rangeEdited = prompt15.editTextRange("""
                        {
                          "page": 1,
                          "logical_start": 0,
                          "logical_end": 5,
                          "replacement_text": "Java20B",
                          "mode": "paragraph_reflow_horizontal",
                          "style_policy": "inherit_leading",
                          "options": {
                            "region": [20.0, 80.0, 180.0, 140.0],
                            "font_size": 12.0,
                            "line_spacing": 1.2,
                            "max_lines_or_columns": 4096,
                            "overflow_policy": "error",
                            "signature_policy_override": false,
                            "deterministic": true
                          }
                        }
                        """);
                assertPrefix(rangeEdited.bytes(), "%PDF-", "Prompt20B text range edit");
                assertTrue(
                    rangeEdited.reportJson().contains("prompt20b_multi_run_text_edit_report"),
                    "Prompt20B text range edit report");
                reports.put("prompt20b_range_edit", rangeEdited.reportJson());
            }
            assertTrue(reports.get("feature").contains("feature_report"), "feature report");
            assertTrue(
                "[]".equals(doc.signatureReportWithOptionsJson("{\"policy_profile\":\"offline_strict\"}")),
                "signature report with offline options");
            try (Oxide.SignatureValidationOptions signatureOptions = new Oxide.SignatureValidationOptions()) {
                signatureOptions.setRevocationMode(1);
                signatureOptions.setRevocationMode(3);
                signatureOptions.setRevocationMode(4);
                signatureOptions.setAlgorithmPolicyJson("{\"allow_rsa_pkcs1v15\":false}");
                signatureOptions.setPathLimits(8, 128);
                signatureOptions.addDistrustedCertificateSha256("0".repeat(64));
                assertTrue("[]".equals(doc.signatureValidationReport(signatureOptions)),
                    "signature report with native options handle");
                assertTrue(doc.signatureValidationWithEvidence(signatureOptions).contains("evidence_bundle"),
                    "signature evidence outcome with native options handle");
            }
            try (
                Oxide.SignatureTrustStore trust = new Oxide.SignatureTrustStore();
                Oxide.SignatureIntermediateStore intermediates = new Oxide.SignatureIntermediateStore();
                Oxide.SignatureEvidenceStore evidence = new Oxide.SignatureEvidenceStore();
                Oxide.SignatureRetrievalPolicy retrieval = new Oxide.SignatureRetrievalPolicy();
                Oxide.SignatureValidationCancellation cancellation =
                    new Oxide.SignatureValidationCancellation();
                Oxide.SignatureValidationOptions signatureOptions = new Oxide.SignatureValidationOptions()
            ) {
                evidence.addOcspDer("untrusted-ocsp".getBytes(StandardCharsets.US_ASCII));
                evidence.addCrlDer("untrusted-crl".getBytes(StandardCharsets.US_ASCII));
                retrieval.setJson("{\"enabled\":false}");
                signatureOptions.applyTrustStore(trust);
                signatureOptions.applyIntermediateStore(intermediates);
                signatureOptions.applyEvidenceStore(evidence);
                signatureOptions.applyRetrievalPolicy(retrieval);
                signatureOptions.setCancellation(cancellation);
                cancellation.cancel();
                boolean cancelled = false;
                try {
                    doc.signatureValidationReport(signatureOptions);
                } catch (Oxide.OxideException expected) {
                    cancelled = expected.getMessage().contains("operation cancelled");
                }
                assertTrue(cancelled, "signature component handles observe cancellation");
            }
            String prompt26KeyPath = System.getenv("OXIDE_PROMPT26_SIGNING_KEY_PEM");
            String prompt26CertPath = System.getenv("OXIDE_PROMPT26_SIGNING_CERT_PEM");
            if (prompt26KeyPath != null && !prompt26KeyPath.isBlank()
                    && prompt26CertPath != null && !prompt26CertPath.isBlank()) {
                String keyPem = Files.readString(Path.of(prompt26KeyPath), StandardCharsets.UTF_8);
                String certPem = Files.readString(Path.of(prompt26CertPath), StandardCharsets.UTF_8);
                String approvalPlan = doc.incrementalSigningPlanJson(keyPem, certPem, 4096, 0);
                String certificationPlan = doc.incrementalSigningPlanJson(keyPem, certPem, 4096, 1);
                assertTrue(approvalPlan.contains("reserved_bytes"), "Prompt26 approval signing plan");
                assertTrue(certificationPlan.contains("reserved_bytes"), "Prompt26 certification signing plan");
                Oxide.BinaryResult signed = doc.signIncremental(
                    keyPem, certPem, 4096, 0, "Prompt26Java", "Java runtime smoke");
                assertTrue(signed.bytes().length > Files.size(fixture), "Prompt26 incremental output grew");
                assertTrue(new String(signed.bytes(), 0, 5, StandardCharsets.US_ASCII).equals("%PDF-"),
                    "Prompt26 signed PDF prefix");
                assertTrue(signed.reportJson().contains("post_sign"), "Prompt26 post-sign report");
                assertTrue(signed.reportJson().contains("prefix_preserved"), "Prompt26 preserved prefix report");
                try (Oxide.Document signedDocument = Oxide.Document.open(signed.bytes())) {
                    assertTrue(!"[]".equals(signedDocument.signatureReportJson()),
                        "Prompt26 post-sign native validation");
                }
                boolean invalidSignerInput = false;
                try {
                    doc.incrementalSigningPlanJson("", certPem, 4096, 0);
                } catch (IllegalArgumentException expected) {
                    invalidSignerInput = true;
                }
                assertTrue(invalidSignerInput, "Prompt26 invalid signing input rejected before native call");
            } else {
                // Package-only runs deliberately do not store key material. The
                // final binding harness supplies target-only PEM files and
                // exercises the branch above for the real signing smoke.
                assertTrue(Oxide.Document.class.getMethod(
                    "incrementalSigningPlanJson", String.class, String.class, long.class, int.class) != null,
                    "Prompt26 signing-plan API");
                assertTrue(Oxide.Document.class.getMethod(
                    "signIncremental", String.class, String.class, long.class, int.class, String.class, String.class) != null,
                    "Prompt26 signing API");
            }

            String timestamp = Oxide.timestampTokenValidationJson(
                "not-a-rfc3161-token".getBytes(StandardCharsets.US_ASCII),
                "cms-signature-value".getBytes(StandardCharsets.US_ASCII));
            assertTrue(timestamp.contains("\"kind\":\"timestamp_token_validation\""),
                "Prompt25 timestamp report kind");
            assertTrue(timestamp.contains("\"token_type\":\"signature_timestamp\""),
                "Prompt25 timestamp token type");
            assertTrue(timestamp.contains("\"status\":\"malformed\""),
                "Prompt25 malformed timestamp status");
            try (Oxide.Document form = Oxide.Document.open(locateFixture("form_160f.pdf"))) {
                String plan = form.signaturePreservingFormPlanJson("name", "Prompt25", "{}");
                assertTrue(plan.contains("\"kind\":\"signature_preserving_edit_plan\""),
                    "Prompt25 signature-preserving plan kind");
                assertTrue(plan.contains("prompt25.tsa-dss-ltv-mdp-signature-edits.v1"),
                    "Prompt25 signature-preserving plan schema");
                assertTrue(plan.contains("\"prefix_preservation_required\":true"),
                    "Prompt25 prefix preservation requirement");
            }
            assertTrue(!Oxide.engineVersion().isBlank(), "engine version");
            assertTrue(Oxide.abiVersion() >= 1, "abi version");
            for (Map.Entry<String, String> entry : reports.entrySet()) {
                assertReport(entry.getValue(), entry.getKey() + " report");
            }

            byte[] docx = doc.toDocx(true);
            byte[] faithfulDocx = doc.toDocx("page-faithful", true);
            byte[] xlsx = doc.toXlsx("pages");
            byte[] pptx = doc.toPptx(true);
            Oxide.BinaryResult sanitized = doc.sanitize("balanced");
            Oxide.BinaryResult canonicalized = doc.canonicalize(0L);
            Oxide.BinaryResult pdfMac = doc.pdfMacCreate();
            Oxide.BinaryResult xfdf = doc.annotationXfdfExport();
            Oxide.BinaryResult appearances = doc.annotationAppearanceGenerate(null);
            Oxide.BinaryResult mediaSanitized = doc.richMediaSanitize("remove_all_media", null);
            reports.put("sanitize", sanitized.reportJson());
            reports.put("canonicalize", canonicalized.reportJson());
            reports.put("pdf_mac_create", pdfMac.reportJson());
            reports.put("xfdf", xfdf.reportJson());
            reports.put("appearances", appearances.reportJson());
            reports.put("media_sanitized", mediaSanitized.reportJson());
            assertPrefix(docx, "PK", "docx");
            assertPrefix(faithfulDocx, "PK", "page-faithful docx");
            assertPrefix(xlsx, "PK", "xlsx");
            assertPrefix(pptx, "PK", "pptx");
            assertPrefix(sanitized.bytes(), "%PDF-", "sanitized pdf");
            assertPrefix(canonicalized.bytes(), "%PDF-", "canonicalized pdf");
            assertPrefix(pdfMac.bytes(), "%PDF-", "PDF-MAC pdf");
            assertTrue(pdfMac.reportJson().contains("pdf_mac_create"), "PDF-MAC create report");
            try (Oxide.Document pdfMacDoc = Oxide.Document.open(pdfMac.bytes())) {
                assertTrue(
                    pdfMacDoc.pdfMacVerifyJson().contains("\"state\":\"valid\""),
                    "PDF-MAC verify valid");
            }
            assertPrefix(xfdf.bytes(), "<?xml", "annotation xfdf");
            assertPrefix(appearances.bytes(), "%PDF-", "annotation appearances pdf");
            assertPrefix(mediaSanitized.bytes(), "%PDF-", "media sanitized pdf");
            assertReport(sanitized.reportJson(), "sanitize report");
            assertReport(canonicalized.reportJson(), "canonicalize report");
            assertPrefix(Oxide.Office.docxToPdf(docx), "%PDF-", "docx pdf");
            assertPrefix(Oxide.Office.xlsxToPdf(xlsx), "%PDF-", "xlsx pdf");
            assertPrefix(Oxide.Office.pptxToPdf(pptx), "%PDF-", "pptx pdf");
            writePrompt02Artifact(fixture, prompt15Fixture, reports, sanitized, canonicalized);
        }

        try (Oxide.Document emptyPassword = Oxide.Document.open(fixture, "")) {
            assertTrue(emptyPassword.pageCount() >= 1, "explicit empty password open");
        }
        try (Oxide.Document ignoredPassword = Oxide.Document.open(
                Files.readAllBytes(fixture),
                "ignored-for-unencrypted")) {
            assertTrue(ignoredPassword.pageCount() >= 1, "password open from bytes");
        }
        try (Oxide.Document disposed = Oxide.Document.open(fixture)) {
            disposed.close();
            disposed.close();
            boolean closedRejected = false;
            try {
                disposed.validateAllStandardsJson();
            } catch (IllegalStateException expected) {
                closedRejected = true;
            }
            assertTrue(closedRejected, "Prompt26 AutoCloseable close is deterministic and idempotent");
        }

        String feature = Oxide.featureReportJson();
        assertTrue(feature.contains("\"progress\""), "progress feature posture");
        assertTrue(
            feature.contains("engine_tile_progressive_resume_supported"),
            "progressive resume feature status");
        assertTrue(feature.contains("\"cancellation\""), "cancellation feature posture");
        assertTrue(
            feature.contains("engine_render_cancellation_supported_binding_tokens_later"),
            "cancellation binding token status");
        assertTrue(feature.contains("\"codec_isolation\""), "codec isolation feature posture");
        assertTrue(feature.contains("\"prompt07_transparency_compositing\""), "prompt07 feature posture");
        assertTrue(
            feature.contains("native_foundation_with_prompt07b_closure"),
            "prompt07 native foundation status");
        assertTrue(feature.contains("\"prompt07b_transparency_closure\""), "prompt07b closure posture");
        assertTrue(feature.contains("\"oxide_outlier_failures\":0"), "prompt07b outlier count");
        assertTrue(feature.contains("\"memory_cap_mb\":4096"), "prompt07 memory cap");
        assertTrue(feature.contains("\"Luminosity\""), "prompt07 blend mode report");
        assertTrue(
            feature.contains("\"prompt08_text_clipping_shading_patterns\""),
            "prompt08 feature posture");
        assertTrue(
            feature.contains("native_common_paths_with_bounded_unsupported_reports"),
            "prompt08 native status");
        assertTrue(feature.contains("\"rendering_modes\":[4,5,6,7]"), "prompt08 text clip modes");
        assertTrue(
            feature.contains("\"prompt09_annotation_ocg_progressive_cache\""),
            "prompt09 feature posture");
        assertTrue(
            feature.contains("implemented_with_bounded_unsupported_reports"),
            "prompt09 implementation status");
        assertTrue(
            feature.contains("\"prompt09b_annotation_progressive_cache_validation\""),
            "prompt09b feature posture");
        assertTrue(feature.contains("implemented_and_proven"), "prompt09b closure status");
        assertTrue(
            feature.contains("\"schema_change\":\"additive_section_only\""),
            "prompt09b additive schema status");
        assertTrue(
            feature.contains("\"prompt10_cjk_rtl_color_glyph_reference_harness\""),
            "prompt10 feature posture");
        assertTrue(
            feature.contains("unsupported_color_tables_are_detected_and_reported"),
            "prompt10 color glyph reporting posture");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt10\""),
            "prompt10 additive schema status");
        assertTrue(
            feature.contains("\"prompt10b_color_glyph_cjk_rtl_fidelity_closure\""),
            "prompt10b feature posture");
        assertTrue(
            feature.contains("\"implemented_with_precise_security_and_exotic_limits\""),
            "prompt10b color glyph closure posture");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt10b\""),
            "prompt10b additive schema status");
        assertTrue(
            feature.contains("\"prompt10c_color_glyph_hinting_cff_closure\""),
            "prompt10c feature posture");
        assertTrue(
            feature.contains("\"implemented_with_operator_level_limits\""),
            "prompt10c colrv1 closure posture");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt10c\""),
            "prompt10c additive schema status");
        assertTrue(
            feature.contains("\"prompt10d_full_colrv1_svg_color_glyph_closure\""),
            "prompt10d feature posture");
        assertTrue(
            feature.contains("\"safe_static_subset_rendered_active_constructs_blocked\""),
            "prompt10d SVG static renderer posture");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt10d\""),
            "prompt10d additive schema status");
        assertTrue(
            feature.contains("\"prompt10e_colrv1_gradient_clip_composite_closure\""),
            "prompt10e feature posture");
        assertTrue(
            feature.contains("\"implemented_with_exact_mode_limits\""),
            "prompt10e composite closure posture");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt10e\""),
            "prompt10e additive schema status");
        assertTrue(
            feature.contains("\"prompt10f_colrv1_porterduff_radial_closure\""),
            "prompt10f feature posture");
        assertTrue(
            feature.contains("\"DestinationAtop\""),
            "prompt10f Porter-Duff posture");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt10f\""),
            "prompt10f additive schema status");
        assertTrue(
            feature.contains("\"prompt11_renderer_fuzz_cmm_closeout\""),
            "prompt11 feature posture");
        assertTrue(
            feature.contains("\"hard_blocked_precise_no_default_native_dependency\""),
            "prompt11 native CMM posture");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt11\""),
            "prompt11 additive schema status");
        assertTrue(
            feature.contains("\"prompt11b_native_littlecms_cmm_backend_closure\""),
            "prompt11b native CMM posture");
        assertTrue(
            feature.contains("\"native-cmm-lcms2\""),
            "prompt11b native CMM feature flag");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt11b\""),
            "prompt11b additive schema status");
        assertTrue(
            feature.contains("\"prompt12_prepress_cmm_device_link_separation_plates\""),
            "prompt12 prepress CMM plate posture");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt12\""),
            "prompt12 additive schema status");
        assertTrue(
            feature.contains("\"cache_key_includes_plate_state\":true"),
            "prompt12 plate cache key status");
        assertTrue(
            feature.contains("\"prompt12b_nchannel_plate_reference_closure\""),
            "prompt12B n-channel plate reference closure");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt12b\""),
            "prompt12B additive schema status");
        assertTrue(
            feature.contains("\"required_and_run_by_prompt12b_audit\""),
            "prompt12B reference audit status");
        assertTrue(
            feature.contains("\"prompt13_full_overprint_prepress_closeout\""),
            "prompt13 full overprint prepress closeout");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt13\""),
            "prompt13 additive schema status");
        assertTrue(
            feature.contains("\"oxide_outlier_failures\":0"),
            "prompt13 zero Oxide outliers");
        assertTrue(
            feature.contains("\"prompt14_semantic_intelligence_parenttree_cjk_ml_layout\""),
            "prompt14 semantic intelligence foundation");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt14\""),
            "prompt14 additive schema status");
        assertTrue(
            feature.contains("\"cloud_upload_default\":false"),
            "prompt14 cloud disabled by default");
        assertTrue(
            feature.contains("\"prompt14b_cjk_dictionary_layout_backend_closure\""),
            "prompt14B dictionary provider closure");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt14b\""),
            "prompt14B additive schema status");
        assertTrue(
            feature.contains("\"external_pack_support\":\"implemented\""),
            "prompt14B external dictionary pack support");
        assertTrue(
            feature.contains("\"local_backend_status\":\"unsupported_reported_no_runtime\""),
            "prompt14B local runtime policy");
        assertTrue(
            feature.contains("\"prompt15_semantic_binding_rag_benchmark_closeout\""),
            "prompt15 semantic binding and RAG closeout");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt15\""),
            "prompt15 additive schema status");
        assertTrue(
            feature.contains("\"model_can_rewrite_deterministic_text\":false"),
            "prompt15 deterministic text preservation");
        assertTrue(feature.contains("\"blocked\":0"), "prompt15 blocked count");
        assertTrue(
            feature.contains("\"prompt16_xfa_runtime_sandbox_closure\""),
            "prompt16 XFA runtime sandbox closure");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt16\""),
            "prompt16 additive schema status");
        assertTrue(
            feature.contains("\"scripts_disabled_events_not_executed\""),
            "prompt16 default script policy");
        assertTrue(
            feature.contains("\"prompt17_annotation_xfdf_media_nonaxis_redaction\""),
            "prompt17 feature closure");
        assertTrue(
            feature.contains("\"additive_feature_report_prompt17\""),
            "prompt17 additive schema status");
        assertTrue(
            feature.contains("\"overlay_only_redaction_success_claims\":0"),
            "prompt17 secure redaction posture");
        assertTrue(
            feature.contains("\"prompt23_deterministic_writer_pubsec_aesgcm\""),
            "prompt23 feature posture");
        assertTrue(
            feature.contains("\"public_key_handler_status\":\"implemented_with_limits\""),
            "prompt23 public-key posture");
        assertTrue(
            feature.contains("\"aes_gcm_decrypt_status\":\"implemented_with_limits\""),
            "prompt23 AES-GCM posture");
        assertTrue(
            Oxide.Document.class.getMethod("openPubSec", byte[].class, byte[].class, byte[].class) != null,
            "PubSec open runtime surface");
        assertTrue(
            Oxide.Document.class.getMethod("openPubSecPfx", byte[].class, byte[].class, byte[].class) != null,
            "PubSec PFX open runtime surface");
        assertTrue(
            Oxide.Document.class.getMethod("pubsecEncryptPdf", byte[].class) != null,
            "PubSec encrypt runtime surface");
        assertTrue(
            Oxide.Document.class.getMethod("pdfMacCreate") != null,
            "PDF-MAC create runtime surface");
        assertTrue(
            Oxide.cryptoTamperTestJson().contains("crypto_tamper_test"),
            "prompt23 tamper report");
        String isolation = Oxide.codecIsolationReportJson(
            "FlateDecode",
            "not-decoded-in-report-only".getBytes(StandardCharsets.UTF_8),
            "report_only");
        assertTrue(isolation.contains("codec_isolation_report"), "codec isolation report");
        assertTrue(isolation.contains("report_only"), "codec isolation report_only status");

        for (int i = 0; i < 25; i++) {
            try (Oxide.Document doc = Oxide.Document.open(fixture)) {
                assertTrue(doc.pageCount() >= 1, "stress page count");
                assertReport(doc.securityReportJson(), "stress security report");
            }
        }

        boolean threw = false;
        try {
            Oxide.Document.open(new byte[] {1, 2, 3, 4});
        } catch (Oxide.OxideException expected) {
            threw = expected.status() != 0;
        }
        assertTrue(threw, "malformed input exception");

        String secret = "do-not-echo-java-password";
        try {
            Oxide.Document.open(new byte[] {1, 2, 3, 4}, secret);
            throw new AssertionError("malformed password open should fail");
        } catch (Oxide.OxideException expected) {
            assertTrue(!expected.getMessage().contains(secret), "password not echoed");
        }
    }

    private static void assertReport(String json, String label) {
        assertTrue(json.contains("\"schema_version\""), label);
    }

    private static void assertPrefix(byte[] bytes, String expected, String label) {
        String actual = new String(bytes, 0, Math.min(bytes.length, expected.length()), StandardCharsets.US_ASCII);
        assertTrue(expected.equals(actual), label + " prefix");
    }

    private static void writePrompt02Artifact(
            Path fixture,
            Path prompt15Fixture,
            Map<String, String> reports,
            Oxide.BinaryResult sanitized,
            Oxide.BinaryResult canonicalized) throws Exception {
        String dir = System.getenv("OXIDE_PROMPT02_ARTIFACT_DIR");
        if (dir == null || dir.isBlank()) {
            return;
        }

        Path artifactDir = Path.of(dir);
        Files.createDirectories(artifactDir);
        StringBuilder json = new StringBuilder();
        json.append("{\n");
        json.append("  \"surface\": \"java\",\n");
        json.append("  \"fixture\": \"").append(escape(fixture.toString())).append("\",\n");
        json.append("  \"prompt15_fixture\": \"").append(escape(prompt15Fixture.toString())).append("\",\n");
        json.append("  \"engine_version\": \"").append(escape(Oxide.engineVersion())).append("\",\n");
        json.append("  \"abi_version\": ").append(Oxide.abiVersion()).append(",\n");
        json.append("  \"reports\": {\n");
        int index = 0;
        for (Map.Entry<String, String> entry : reports.entrySet()) {
            if (index++ > 0) {
                json.append(",\n");
            }
            byte[] bytes = entry.getValue().getBytes(StandardCharsets.UTF_8);
            json.append("    \"").append(escape(entry.getKey())).append("\": {")
                .append("\"sha256\": \"").append(sha256(bytes)).append("\", ")
                .append("\"bytes\": ").append(bytes.length).append("}");
        }
        json.append("\n  },\n");
        json.append("  \"outputs\": {\n");
        json.append("    \"sanitized\": {\"bytes\": ").append(sanitized.bytes().length)
            .append(", \"sha256\": \"").append(sha256(sanitized.bytes())).append("\"},\n");
        json.append("    \"canonicalized\": {\"bytes\": ").append(canonicalized.bytes().length)
            .append(", \"sha256\": \"").append(sha256(canonicalized.bytes())).append("\"}\n");
        json.append("  }\n");
        json.append("}\n");
        Files.writeString(artifactDir.resolve("java-smoke.json"), json.toString(), StandardCharsets.UTF_8);
    }

    private static String sha256(byte[] bytes) throws Exception {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
    }

    private static String escape(String value) {
        return value.replace("\\", "\\\\").replace("\"", "\\\"");
    }

    private static void assertTrue(boolean value, String label) {
        if (!value) {
            throw new AssertionError(label);
        }
    }

    private static Path fixturePath() throws Exception {
        String env = System.getenv("OXIDE_FIXTURE_PDF");
        if (env != null && !env.isBlank() && Files.exists(Path.of(env))) {
            return Path.of(env);
        }
        return locateFixture("tracemonkey.pdf");
    }

    private static Path locateFixture(String name) {
        Path dir = Path.of("").toAbsolutePath();
        while (dir != null) {
            Path candidate = dir.resolve("crates/engine/tests/fixtures").resolve(name);
            if (Files.exists(candidate)) {
                return candidate;
            }
            dir = dir.getParent();
        }
        throw new IllegalStateException("Could not locate fixture " + name);
    }
}
