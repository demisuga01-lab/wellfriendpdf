package io.wellfriendpdf;

import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.Map;

public final class WellfriendPdfSmokeTest {
    public static void main(String[] args) throws Exception {
        Path fixture = fixturePath();
        Path semantic_closeoutFixture = locateFixture("multi_stream.pdf");
        try (WellfriendPdf.Document doc = WellfriendPdf.Document.open(fixture)) {
            assertTrue(doc.pageCount() >= 1, "page count");
            assertTrue(!doc.page(1).text().isBlank(), "text extraction");
            byte[] png = doc.page(1).renderPng();
            byte[] jpeg = doc.page(1).renderJpeg(72, (byte) 85);
            String contract = doc.defaultRenderContractJson(1, 72);
            byte[] contractPng = doc.renderPagePngWithContractJson(contract);
            ByteBuffer callerSurface = ByteBuffer.allocateDirect(4_000_000);
            doc.renderPageIntoBufferWithContractJson(contract, callerSurface);
            assertTrue(callerSurface.get(0) != 0 || callerSurface.get(1) != 0 || callerSurface.get(2) != 0,
                "caller-owned render surface");
            assertTrue(png.length > 8 && png[0] == (byte) 0x89 && png[1] == (byte) 0x50,
                "PNG raster rendering");
            assertTrue(contractPng.length > 8 && contractPng[0] == (byte) 0x89 && contractPng[1] == (byte) 0x50,
                "contract PNG raster rendering");
            assertTrue(jpeg.length > 4 && jpeg[0] == (byte) 0xff && jpeg[1] == (byte) 0xd8,
                "JPEG raster rendering");
            assertTrue(doc.parseJson().contains("\"schema_version\""), "parse json");
            Map<String, String> reports = new LinkedHashMap<>();
            reports.put("feature", WellfriendPdf.featureReportJson());
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
                "IncrementalSigningStandards PDF/A standards envelope");
            assertTrue(reports.get("pdfua_standards").contains("\"kind\":\"pdfua_standards_validation\""),
                "IncrementalSigningStandards PDF/UA standards envelope");
            assertTrue(reports.get("pdfx_standards").contains("\"kind\":\"pdfx_standards_validation\""),
                "IncrementalSigningStandards PDF/X standards envelope");
            assertTrue(reports.get("standards_all").contains("\"kind\":\"standards_all_validation\""),
                "IncrementalSigningStandards combined standards envelope");
            reports.put("forms", doc.formsReportJson());
            reports.put("xfa", doc.xfaReportJson());
            reports.put("xfa_extract", doc.xfaExtractJson());
            reports.put("xfa_script", doc.xfaScriptReportJson());
            reports.put("xfa_security", doc.xfaSecurityReportJson());
            reports.put("xfa_runtime", doc.xfaRuntimeReportJson("disabled", false));
            reports.put("annotations", doc.annotationsReportJson());
            reports.put("rich_media", doc.richMediaReportJson());
            reports.put("annotation_appearance", doc.annotationAppearanceReportJson(null));
            reports.put("annotation_media_redaction", doc.annotation_media_redactionReportJson());
            reports.put("secure_mutation", doc.secure_mutationReportJson());
            reports.put("secure_mutation_closeout", doc.secure_mutation_closeoutReportJson());
            reports.put("form_js", doc.formJavaScriptReportJson());
            reports.put("form_action_graph", doc.formActionGraphJson());
            reports.put("interactive_data", doc.interactiveDataReportJson());
            reports.put("word_pagination", doc.wordPaginationAuditJson("page-faithful"));
            reports.put("form_action_policy", doc.form_action_policyReportJson());
            reports.put("advanced_editing", doc.advanced_editingReportJson());
            reports.put("advanced_editing_closeout", doc.advanced_editing_closeoutReportJson());
            reports.put("editing_transactions", doc.editing_transactionsReportJson());
            reports.put("document_subsystems", doc.document_subsystemsReportJson());
            assertTrue(
                reports.get("document_subsystems").contains("document_subsystems.tables-math-ocr-forms-annotations.v1"),
                "DocumentSubsystems feature schema");
            assertTrue(
                reports.get("editing_transactions").contains("editing_transactions.scene-transactions-fonts-shaping.v1"),
                "EditingTransactions closeout schema");
            reports.put("associated_files", doc.associatedFilesReportJson());
            reports.put("edit_policy", doc.editPolicyReportJson("incremental_save"));
            reports.put("pages", doc.pagesReportJson());
            reports.put("interactive", doc.interactiveReportJson());
            reports.put("chunks", doc.chunksJson());
            try (WellfriendPdf.Document semantic_closeout = WellfriendPdf.Document.open(semantic_closeoutFixture)) {
                reports.put("advanced_chunks", semantic_closeout.advancedChunksJson());
                reports.put("semantic_bundle", semantic_closeout.semanticBundleJson());
                reports.put("semantic_search", semantic_closeout.semanticSearchJson("Hello"));
                String scene = semantic_closeout.editing_transactionsSceneReportJson("[1]");
                assertTrue(scene.contains("editing_transactions_scene_report"), "EditingTransactions scene graph");
                assertTrue(scene.contains("\"nodes\""), "EditingTransactions scene nodes");
                String txRequest = """
                        {
                          "requested_mode": "operator_preserving",
                          "page": 1,
                          "source_text": "Hello",
                          "replacement_text": "HELLO"
                        }
                        """;
                String txPlan = semantic_closeout.editing_transactionsTransactionPlanJson(txRequest);
                assertTrue(txPlan.contains("editing_transactions_transaction_plan"), "EditingTransactions transaction plan");
                assertTrue(txPlan.contains("transaction_id"), "EditingTransactions transaction id");
                String textMap = semantic_closeout.editing_transactionsTextMapJson("A\u0301B", "ltr");
                assertTrue(textMap.contains("editing_transactions_text_map"), "EditingTransactions text map");
                String text_reflowRequest = """
                        {
                          "requested_mode": "geometric_block",
                          "page": 1,
                          "source_text": "Hello",
                          "replacement_text": "World",
                          "region": [10.0, 10.0, 260.0, 90.0],
                          "language": "en",
                          "hyphenation": true,
                          "layout_constraints": [{
                            "constraint_id": "java_soft_height",
                            "variable": "region_height",
                            "relation": "ge",
                            "value": 500.0,
                            "priority": "weak"
                          }]
                        }
                        """;
                reports.put("text_reflow", semantic_closeout.text_reflowReportJson());
                assertTrue(
                    semantic_closeout.document_subsystemsAnalyzeJson().contains("document_subsystems_analyze"),
                    "DocumentSubsystems source-linked analysis");
                assertTrue(
                    reports.get("text_reflow").contains("text_reflow.geometric-semantic-reflow.v1"),
                    "TextReflow schema");
                assertTrue(semantic_closeout.text_reflowLayoutAnalyzeJson(text_reflowRequest).contains("text_reflow_layout_analyze"),
                    "TextReflow layout analysis");
                assertTrue(semantic_closeout.text_reflowOverflowReportJson(text_reflowRequest).contains("text_reflow_overflow_report"),
                    "TextReflow overflow report");
                String text_reflowConstraints = semantic_closeout.text_reflowConstraintsReportJson(text_reflowRequest);
                assertTrue(text_reflowConstraints.contains("text_reflow_constraints_report"),
                    "TextReflow constraint report");
                assertTrue(text_reflowConstraints.contains("java_soft_height"),
                    "TextReflow request constraint reaches canonical engine");
                assertTrue(semantic_closeout.text_reflowConfidenceReportJson(text_reflowRequest).contains("text_reflow_confidence_report"),
                    "TextReflow confidence report");
                WellfriendPdf.BinaryResult text_reflowApplied = semantic_closeout.text_reflowReflowRegion(text_reflowRequest);
                assertPrefix(text_reflowApplied.bytes(), "%PDF-", "TextReflow geometric reflow");
                assertTrue(
                    semantic_closeout.text_reflowValidateReflowOutputJson(text_reflowApplied.bytes(), text_reflowRequest)
                        .contains("text_reflow_validate_reflow_output"),
                    "TextReflow output validation");
                WellfriendPdf.BinaryResult text_reflowUndo =
                    semantic_closeout.text_reflowUndoReflow(text_reflowApplied.bytes(), text_reflowRequest);
                assertTrue(text_reflowUndo.reportJson().contains("text_reflow_undo_reflow"),
                    "TextReflow executable undo report");
                assertTrue(text_reflowUndo.reportJson().contains("\"byte_exact_restoration\":true"),
                    "TextReflow byte-exact undo");
                assertTrue(java.util.Arrays.equals(Files.readAllBytes(semantic_closeoutFixture), text_reflowUndo.bytes()),
                    "TextReflow undo restores source bytes");
                String shape = semantic_closeout.editing_transactionsShapeTextJson("ffi", "ltr");
                assertTrue(shape.contains("\"glyphs\""), "EditingTransactions shaping");
                String subset = semantic_closeout.editing_transactionsFontSubsetPlanJson("Hello", "ltr", "reuse_embedded_subset");
                assertTrue(subset.contains("editing_transactions_font_subset_plan"), "EditingTransactions subset plan");
                assertTrue(subset.contains("deterministic_subset_tag"), "EditingTransactions deterministic subset tag");
                String rangeModel = semantic_closeout.advanced_editing_closeoutTextRangeAnalyzeJson(1);
                reports.put("advanced_editing_closeout_range_model", rangeModel);
                assertTrue(rangeModel.contains("advanced_editing_closeout_multi_run_range_model"), "AdvancedEditingB range model");
                WellfriendPdf.BinaryResult rangeEdited = semantic_closeout.editTextRange("""
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
                assertPrefix(rangeEdited.bytes(), "%PDF-", "AdvancedEditingB text range edit");
                assertTrue(
                    rangeEdited.reportJson().contains("advanced_editing_closeout_multi_run_text_edit_report"),
                    "AdvancedEditingB text range edit report");
                reports.put("advanced_editing_closeout_range_edit", rangeEdited.reportJson());
            }
            assertTrue(reports.get("feature").contains("feature_report"), "feature report");
            assertTrue(
                "[]".equals(doc.signatureReportWithOptionsJson("{\"policy_profile\":\"offline_strict\"}")),
                "signature report with offline options");
            try (WellfriendPdf.SignatureValidationOptions signatureOptions = new WellfriendPdf.SignatureValidationOptions()) {
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
                WellfriendPdf.SignatureTrustStore trust = new WellfriendPdf.SignatureTrustStore();
                WellfriendPdf.SignatureIntermediateStore intermediates = new WellfriendPdf.SignatureIntermediateStore();
                WellfriendPdf.SignatureEvidenceStore evidence = new WellfriendPdf.SignatureEvidenceStore();
                WellfriendPdf.SignatureRetrievalPolicy retrieval = new WellfriendPdf.SignatureRetrievalPolicy();
                WellfriendPdf.SignatureValidationCancellation cancellation =
                    new WellfriendPdf.SignatureValidationCancellation();
                WellfriendPdf.SignatureValidationOptions signatureOptions = new WellfriendPdf.SignatureValidationOptions()
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
                } catch (WellfriendPdf.WellfriendPdfException expected) {
                    cancelled = expected.getMessage().contains("operation cancelled");
                }
                assertTrue(cancelled, "signature component handles observe cancellation");
            }
            String incremental_signing_standardsKeyPath = System.getenv("WELLFRIENDPDF_INCREMENTAL_SIGNING_STANDARDS_SIGNING_KEY_PEM");
            String incremental_signing_standardsCertPath = System.getenv("WELLFRIENDPDF_INCREMENTAL_SIGNING_STANDARDS_SIGNING_CERT_PEM");
            if (incremental_signing_standardsKeyPath != null && !incremental_signing_standardsKeyPath.isBlank()
                    && incremental_signing_standardsCertPath != null && !incremental_signing_standardsCertPath.isBlank()) {
                String keyPem = Files.readString(Path.of(incremental_signing_standardsKeyPath), StandardCharsets.UTF_8);
                String certPem = Files.readString(Path.of(incremental_signing_standardsCertPath), StandardCharsets.UTF_8);
                String approvalPlan = doc.incrementalSigningPlanJson(keyPem, certPem, 4096, 0);
                String certificationPlan = doc.incrementalSigningPlanJson(keyPem, certPem, 4096, 1);
                assertTrue(approvalPlan.contains("reserved_bytes"), "IncrementalSigningStandards approval signing plan");
                assertTrue(certificationPlan.contains("reserved_bytes"), "IncrementalSigningStandards certification signing plan");
                WellfriendPdf.BinaryResult signed = doc.signIncremental(
                    keyPem, certPem, 4096, 0, "IncrementalSigningStandardsJava", "Java runtime smoke");
                assertTrue(signed.bytes().length > Files.size(fixture), "IncrementalSigningStandards incremental output grew");
                assertTrue(new String(signed.bytes(), 0, 5, StandardCharsets.US_ASCII).equals("%PDF-"),
                    "IncrementalSigningStandards signed PDF prefix");
                assertTrue(signed.reportJson().contains("post_sign"), "IncrementalSigningStandards post-sign report");
                assertTrue(signed.reportJson().contains("prefix_preserved"), "IncrementalSigningStandards preserved prefix report");
                try (WellfriendPdf.Document signedDocument = WellfriendPdf.Document.open(signed.bytes())) {
                    assertTrue(!"[]".equals(signedDocument.signatureReportJson()),
                        "IncrementalSigningStandards post-sign native validation");
                }
                boolean invalidSignerInput = false;
                try {
                    doc.incrementalSigningPlanJson("", certPem, 4096, 0);
                } catch (IllegalArgumentException expected) {
                    invalidSignerInput = true;
                }
                assertTrue(invalidSignerInput, "IncrementalSigningStandards invalid signing input rejected before native call");
            } else {
                // Package-only runs deliberately do not store key material. The
                // final binding harness supplies target-only PEM files and
                // exercises the branch above for the real signing smoke.
                assertTrue(WellfriendPdf.Document.class.getMethod(
                    "incrementalSigningPlanJson", String.class, String.class, long.class, int.class) != null,
                    "IncrementalSigningStandards signing-plan API");
                assertTrue(WellfriendPdf.Document.class.getMethod(
                    "signIncremental", String.class, String.class, long.class, int.class, String.class, String.class) != null,
                    "IncrementalSigningStandards signing API");
            }

            String timestamp = WellfriendPdf.timestampTokenValidationJson(
                "not-a-rfc3161-token".getBytes(StandardCharsets.US_ASCII),
                "cms-signature-value".getBytes(StandardCharsets.US_ASCII));
            assertTrue(timestamp.contains("\"kind\":\"timestamp_token_validation\""),
                "PadesLTV timestamp report kind");
            assertTrue(timestamp.contains("\"token_type\":\"signature_timestamp\""),
                "PadesLTV timestamp token type");
            assertTrue(timestamp.contains("\"status\":\"malformed\""),
                "PadesLTV malformed timestamp status");
            try (WellfriendPdf.Document form = WellfriendPdf.Document.open(locateFixture("form_160f.pdf"))) {
                String plan = form.signaturePreservingFormPlanJson("name", "PadesLTV", "{}");
                assertTrue(plan.contains("\"kind\":\"signature_preserving_edit_plan\""),
                    "PadesLTV signature-preserving plan kind");
                assertTrue(plan.contains("pades_ltv.tsa-dss-ltv-mdp-signature-edits.v1"),
                    "PadesLTV signature-preserving plan schema");
                assertTrue(plan.contains("\"prefix_preservation_required\":true"),
                    "PadesLTV prefix preservation requirement");
            }
            assertTrue(!WellfriendPdf.engineVersion().isBlank(), "engine version");
            assertTrue(WellfriendPdf.abiVersion() >= 1, "abi version");
            for (Map.Entry<String, String> entry : reports.entrySet()) {
                assertReport(entry.getValue(), entry.getKey() + " report");
            }

            byte[] docx = doc.toDocx(true);
            byte[] faithfulDocx = doc.toDocx("page-faithful", true);
            byte[] xlsx = doc.toXlsx("pages");
            byte[] pptx = doc.toPptx(true);
            WellfriendPdf.BinaryResult sanitized = doc.sanitize("balanced");
            WellfriendPdf.BinaryResult canonicalized = doc.canonicalize(0L);
            WellfriendPdf.BinaryResult pdfMac = doc.pdfMacCreate();
            WellfriendPdf.BinaryResult xfdf = doc.annotationXfdfExport();
            WellfriendPdf.BinaryResult appearances = doc.annotationAppearanceGenerate(null);
            WellfriendPdf.BinaryResult mediaSanitized = doc.richMediaSanitize("remove_all_media", null);
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
            try (WellfriendPdf.Document pdfMacDoc = WellfriendPdf.Document.open(pdfMac.bytes())) {
                assertTrue(
                    pdfMacDoc.pdfMacVerifyJson().contains("\"state\":\"valid\""),
                    "PDF-MAC verify valid");
            }
            assertPrefix(xfdf.bytes(), "<?xml", "annotation xfdf");
            assertPrefix(appearances.bytes(), "%PDF-", "annotation appearances pdf");
            assertPrefix(mediaSanitized.bytes(), "%PDF-", "media sanitized pdf");
            assertReport(sanitized.reportJson(), "sanitize report");
            assertReport(canonicalized.reportJson(), "canonicalize report");
            assertPrefix(WellfriendPdf.Office.docxToPdf(docx), "%PDF-", "docx pdf");
            assertPrefix(WellfriendPdf.Office.xlsxToPdf(xlsx), "%PDF-", "xlsx pdf");
            assertPrefix(WellfriendPdf.Office.pptxToPdf(pptx), "%PDF-", "pptx pdf");
            writeBindingParityArtifact(fixture, semantic_closeoutFixture, reports, sanitized, canonicalized);
        }

        try (WellfriendPdf.Document emptyPassword = WellfriendPdf.Document.open(fixture, "")) {
            assertTrue(emptyPassword.pageCount() >= 1, "explicit empty password open");
        }
        try (WellfriendPdf.Document ignoredPassword = WellfriendPdf.Document.open(
                Files.readAllBytes(fixture),
                "ignored-for-unencrypted")) {
            assertTrue(ignoredPassword.pageCount() >= 1, "password open from bytes");
        }
        try (WellfriendPdf.Document disposed = WellfriendPdf.Document.open(fixture)) {
            disposed.close();
            disposed.close();
            boolean closedRejected = false;
            try {
                disposed.validateAllStandardsJson();
            } catch (IllegalStateException expected) {
                closedRejected = true;
            }
            assertTrue(closedRejected, "IncrementalSigningStandards AutoCloseable close is deterministic and idempotent");
        }

        String feature = WellfriendPdf.featureReportJson();
        assertTrue(feature.contains("\"progress\""), "progress feature posture");
        assertTrue(
            feature.contains("engine_tile_progressive_resume_supported"),
            "progressive resume feature status");
        assertTrue(feature.contains("\"cancellation\""), "cancellation feature posture");
        assertTrue(
            feature.contains("engine_render_cancellation_supported_binding_tokens_later"),
            "cancellation binding token status");
        assertTrue(feature.contains("\"codec_isolation\""), "codec isolation feature posture");
        assertTrue(feature.contains("\"transparency_rendering_transparency_compositing\""), "transparency_rendering feature posture");
        assertTrue(
            feature.contains("native_foundation_with_transparency_closeout_closure"),
            "transparency_rendering native foundation status");
        assertTrue(feature.contains("\"transparency_closeout_transparency_closure\""), "transparency_closeout closure posture");
        assertTrue(feature.contains("\"wellfriendpdf_outlier_failures\":0"), "transparency_closeout outlier count");
        assertTrue(feature.contains("\"memory_cap_mb\":4096"), "transparency_rendering memory cap");
        assertTrue(feature.contains("\"Luminosity\""), "transparency_rendering blend mode report");
        assertTrue(
            feature.contains("\"advanced_rendering_text_clipping_shading_patterns\""),
            "advanced_rendering feature posture");
        assertTrue(
            feature.contains("native_common_paths_with_bounded_unsupported_reports"),
            "advanced_rendering native status");
        assertTrue(feature.contains("\"rendering_modes\":[4,5,6,7]"), "advanced_rendering text clip modes");
        assertTrue(
            feature.contains("\"annotation_ocg_rendering_annotation_ocg_progressive_cache\""),
            "annotation_ocg_rendering feature posture");
        assertTrue(
            feature.contains("implemented_with_bounded_unsupported_reports"),
            "annotation_ocg_rendering implementation status");
        assertTrue(
            feature.contains("\"renderer_validation_annotation_progressive_cache_validation\""),
            "renderer_validation feature posture");
        assertTrue(feature.contains("implemented_and_proven"), "renderer_validation closure status");
        assertTrue(
            feature.contains("\"schema_change\":\"additive_section_only\""),
            "renderer_validation additive schema status");
        assertTrue(
            feature.contains("\"multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness\""),
            "multilingual_color_glyphs feature posture");
        assertTrue(
            feature.contains("unsupported_color_tables_are_detected_and_reported"),
            "multilingual_color_glyphs color glyph reporting posture");
        assertTrue(
            feature.contains("\"additive_feature_report_multilingual_color_glyphs\""),
            "multilingual_color_glyphs additive schema status");
        assertTrue(
            feature.contains("\"cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure\""),
            "cjk_rtl_color_glyph_closeout feature posture");
        assertTrue(
            feature.contains("\"implemented_with_precise_security_and_exotic_limits\""),
            "cjk_rtl_color_glyph_closeout color glyph closure posture");
        assertTrue(
            feature.contains("\"additive_feature_report_cjk_rtl_color_glyph_closeout\""),
            "cjk_rtl_color_glyph_closeout additive schema status");
        assertTrue(
            feature.contains("\"color_glyph_hinting_color_glyph_hinting_cff_closure\""),
            "color_glyph_hinting feature posture");
        assertTrue(
            feature.contains("\"implemented_with_operator_level_limits\""),
            "color_glyph_hinting colrv1 closure posture");
        assertTrue(
            feature.contains("\"additive_feature_report_color_glyph_hinting\""),
            "color_glyph_hinting additive schema status");
        assertTrue(
            feature.contains("\"colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure\""),
            "colrv_svg_bitmap feature posture");
        assertTrue(
            feature.contains("\"safe_static_subset_rendered_active_constructs_blocked\""),
            "colrv_svg_bitmap SVG static renderer posture");
        assertTrue(
            feature.contains("\"additive_feature_report_colrv_svg_bitmap\""),
            "colrv_svg_bitmap additive schema status");
        assertTrue(
            feature.contains("\"colrv_gradient_composite_colrv1_gradient_clip_composite_closure\""),
            "colrv_gradient_composite feature posture");
        assertTrue(
            feature.contains("\"implemented_with_exact_mode_limits\""),
            "colrv_gradient_composite composite closure posture");
        assertTrue(
            feature.contains("\"additive_feature_report_colrv_gradient_composite\""),
            "colrv_gradient_composite additive schema status");
        assertTrue(
            feature.contains("\"porterduff_radial_color_glyph_colrv1_porterduff_radial_closure\""),
            "porterduff_radial_color_glyph feature posture");
        assertTrue(
            feature.contains("\"DestinationAtop\""),
            "porterduff_radial_color_glyph Porter-Duff posture");
        assertTrue(
            feature.contains("\"additive_feature_report_porterduff_radial_color_glyph\""),
            "porterduff_radial_color_glyph additive schema status");
        assertTrue(
            feature.contains("\"renderer_fuzz_cmm_renderer_fuzz_cmm_closeout\""),
            "renderer_fuzz_cmm feature posture");
        assertTrue(
            feature.contains("\"hard_blocked_precise_no_default_native_dependency\""),
            "renderer_fuzz_cmm native CMM posture");
        assertTrue(
            feature.contains("\"additive_feature_report_renderer_fuzz_cmm\""),
            "renderer_fuzz_cmm additive schema status");
        assertTrue(
            feature.contains("\"native_cmm_backend_native_littlecms_cmm_backend_closure\""),
            "native_cmm_backend native CMM posture");
        assertTrue(
            feature.contains("\"native-cmm-lcms2\""),
            "native_cmm_backend native CMM feature flag");
        assertTrue(
            feature.contains("\"additive_feature_report_native_cmm_backend\""),
            "native_cmm_backend additive schema status");
        assertTrue(
            feature.contains("\"prepress_cmm_prepress_cmm_device_link_separation_plates\""),
            "prepress_cmm prepress CMM plate posture");
        assertTrue(
            feature.contains("\"additive_feature_report_prepress_cmm\""),
            "prepress_cmm additive schema status");
        assertTrue(
            feature.contains("\"cache_key_includes_plate_state\":true"),
            "prepress_cmm plate cache key status");
        assertTrue(
            feature.contains("\"nchannel_plate_prepress_nchannel_plate_reference_closure\""),
            "nchannel_plate_prepress n-channel plate reference closure");
        assertTrue(
            feature.contains("\"additive_feature_report_nchannel_plate_prepress\""),
            "nchannel_plate_prepress additive schema status");
        assertTrue(
            feature.contains("\"required_and_run_by_nchannel_plate_prepress_audit\""),
            "nchannel_plate_prepress reference audit status");
        assertTrue(
            feature.contains("\"prepress_proofing_full_overprint_prepress_closeout\""),
            "prepress_proofing full overprint prepress closeout");
        assertTrue(
            feature.contains("\"additive_feature_report_prepress_proofing\""),
            "prepress_proofing additive schema status");
        assertTrue(
            feature.contains("\"wellfriendpdf_outlier_failures\":0"),
            "prepress_proofing zero Wellfriend outliers");
        assertTrue(
            feature.contains("\"semantic_intelligence_semantic_intelligence_parenttree_cjk_ml_layout\""),
            "semantic_intelligence semantic intelligence foundation");
        assertTrue(
            feature.contains("\"additive_feature_report_semantic_intelligence\""),
            "semantic_intelligence additive schema status");
        assertTrue(
            feature.contains("\"cloud_upload_default\":false"),
            "semantic_intelligence cloud disabled by default");
        assertTrue(
            feature.contains("\"cjk_dictionary_layout_cjk_dictionary_layout_backend_closure\""),
            "cjk_dictionary_layout dictionary provider closure");
        assertTrue(
            feature.contains("\"additive_feature_report_cjk_dictionary_layout\""),
            "cjk_dictionary_layout additive schema status");
        assertTrue(
            feature.contains("\"external_pack_support\":\"implemented\""),
            "cjk_dictionary_layout external dictionary pack support");
        assertTrue(
            feature.contains("\"local_backend_status\":\"unsupported_reported_no_runtime\""),
            "cjk_dictionary_layout local runtime policy");
        assertTrue(
            feature.contains("\"semantic_closeout_semantic_binding_rag_benchmark_closeout\""),
            "semantic_closeout semantic binding and RAG closeout");
        assertTrue(
            feature.contains("\"additive_feature_report_semantic_closeout\""),
            "semantic_closeout additive schema status");
        assertTrue(
            feature.contains("\"model_can_rewrite_deterministic_text\":false"),
            "semantic_closeout deterministic text preservation");
        assertTrue(feature.contains("\"blocked\":0"), "semantic_closeout blocked count");
        assertTrue(
            feature.contains("\"xfa_runtime_xfa_runtime_sandbox_closure\""),
            "xfa_runtime XFA runtime sandbox closure");
        assertTrue(
            feature.contains("\"additive_feature_report_xfa_runtime\""),
            "xfa_runtime additive schema status");
        assertTrue(
            feature.contains("\"scripts_disabled_events_not_executed\""),
            "xfa_runtime default script policy");
        assertTrue(
            feature.contains("\"annotation_media_redaction_annotation_xfdf_media_nonaxis_redaction\""),
            "annotation_media_redaction feature closure");
        assertTrue(
            feature.contains("\"additive_feature_report_annotation_media_redaction\""),
            "annotation_media_redaction additive schema status");
        assertTrue(
            feature.contains("\"overlay_only_redaction_success_claims\":0"),
            "annotation_media_redaction secure redaction posture");
        assertTrue(
            feature.contains("\"crypto_writer_deterministic_writer_pubsec_aesgcm\""),
            "crypto_writer feature posture");
        assertTrue(
            feature.contains("\"public_key_handler_status\":\"implemented_with_limits\""),
            "crypto_writer public-key posture");
        assertTrue(
            feature.contains("\"aes_gcm_decrypt_status\":\"implemented_with_limits\""),
            "crypto_writer AES-GCM posture");
        assertTrue(
            WellfriendPdf.Document.class.getMethod("openPubSec", byte[].class, byte[].class, byte[].class) != null,
            "PubSec open runtime surface");
        assertTrue(
            WellfriendPdf.Document.class.getMethod("openPubSecPfx", byte[].class, byte[].class, byte[].class) != null,
            "PubSec PFX open runtime surface");
        assertTrue(
            WellfriendPdf.Document.class.getMethod("pubsecEncryptPdf", byte[].class) != null,
            "PubSec encrypt runtime surface");
        assertTrue(
            WellfriendPdf.Document.class.getMethod("pdfMacCreate") != null,
            "PDF-MAC create runtime surface");
        assertTrue(
            WellfriendPdf.cryptoTamperTestJson().contains("crypto_tamper_test"),
            "crypto_writer tamper report");
        String isolation = WellfriendPdf.codecIsolationReportJson(
            "FlateDecode",
            "not-decoded-in-report-only".getBytes(StandardCharsets.UTF_8),
            "report_only");
        assertTrue(isolation.contains("codec_isolation_report"), "codec isolation report");
        assertTrue(isolation.contains("report_only"), "codec isolation report_only status");

        for (int i = 0; i < 25; i++) {
            try (WellfriendPdf.Document doc = WellfriendPdf.Document.open(fixture)) {
                assertTrue(doc.pageCount() >= 1, "stress page count");
                assertReport(doc.securityReportJson(), "stress security report");
            }
        }

        boolean threw = false;
        try {
            WellfriendPdf.Document.open(new byte[] {1, 2, 3, 4});
        } catch (WellfriendPdf.WellfriendPdfException expected) {
            threw = expected.status() != 0;
        }
        assertTrue(threw, "malformed input exception");

        String secret = "do-not-echo-java-password";
        try {
            WellfriendPdf.Document.open(new byte[] {1, 2, 3, 4}, secret);
            throw new AssertionError("malformed password open should fail");
        } catch (WellfriendPdf.WellfriendPdfException expected) {
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

    private static void writeBindingParityArtifact(
            Path fixture,
            Path semantic_closeoutFixture,
            Map<String, String> reports,
            WellfriendPdf.BinaryResult sanitized,
            WellfriendPdf.BinaryResult canonicalized) throws Exception {
        String dir = System.getenv("WELLFRIENDPDF_BINDING_PARITY_ARTIFACT_DIR");
        if (dir == null || dir.isBlank()) {
            return;
        }

        Path artifactDir = Path.of(dir);
        Files.createDirectories(artifactDir);
        StringBuilder json = new StringBuilder();
        json.append("{\n");
        json.append("  \"surface\": \"java\",\n");
        json.append("  \"fixture\": \"").append(escape(fixture.toString())).append("\",\n");
        json.append("  \"semantic_closeout_fixture\": \"").append(escape(semantic_closeoutFixture.toString())).append("\",\n");
        json.append("  \"engine_version\": \"").append(escape(WellfriendPdf.engineVersion())).append("\",\n");
        json.append("  \"abi_version\": ").append(WellfriendPdf.abiVersion()).append(",\n");
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
        String env = System.getenv("WELLFRIENDPDF_FIXTURE_PDF");
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
