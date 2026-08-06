using System.Text;
using System.Text.Json;
using System.Security.Cryptography;
using WellfriendPdf;
using Xunit;

namespace WellfriendPdf.Tests;

public sealed class WellfriendPdfSmokeTests
{
    [Fact]
    public void OpenExtractAndConvert()
    {
        using var doc = WellfriendDocument.Open(FixturePath());
        Assert.True(doc.PageCount >= 1);
        Assert.False(string.IsNullOrWhiteSpace(doc.GetPage(1).Text));
        Assert.Contains("\"schema_version\"", doc.ParseJson());
        var reports = new Dictionary<string, string>
        {
            ["feature"] = WellfriendDocument.FeatureReportJson(),
            ["security"] = doc.SecurityReportJson(),
            ["parser"] = doc.ParserReportJson(),
            ["color"] = doc.ColorReportJson(),
            ["validate_security"] = doc.ValidateJson("security"),
            ["forms"] = doc.FormsReportJson(),
            ["xfa"] = doc.XfaReportJson(),
            ["xfa_extract"] = doc.XfaExtractJson(),
            ["xfa_script"] = doc.XfaScriptReportJson(),
            ["xfa_security"] = doc.XfaSecurityReportJson(),
            ["xfa_runtime"] = doc.XfaRuntimeReportJson(),
            ["annotations"] = doc.AnnotationsReportJson(),
            ["rich_media"] = doc.RichMediaReportJson(),
            ["annotation_appearance"] = doc.AnnotationAppearanceReportJson(),
            ["annotation_media_redaction"] = doc.AnnotationMediaRedactionReportJson(),
            ["secure_mutation"] = doc.SecureMutationReportJson(),
            ["secure_mutation_closeout"] = doc.SecureMutationCloseoutReportJson(),
            ["form_js"] = doc.FormJavaScriptReportJson(),
            ["form_action_graph"] = doc.FormActionGraphJson(),
            ["interactive_data"] = doc.InteractiveDataReportJson(),
            ["word_pagination"] = doc.WordPaginationAuditJson(),
            ["form_action_policy"] = doc.FormActionPolicyReportJson(),
            ["advanced_editing"] = doc.AdvancedEditingReportJson(),
            ["advanced_editing_closeout"] = doc.AdvancedEditingCloseoutReportJson(),
            ["associated_files"] = doc.AssociatedFilesReportJson(),
            ["edit_policy"] = doc.EditPolicyReportJson("incremental_save"),
            ["pages"] = doc.PagesReportJson(),
            ["interactive"] = doc.InteractiveReportJson(),
            ["chunks"] = doc.ChunksJson(),
            ["advanced_chunks"] = doc.AdvancedChunksJson(),
            ["semantic_bundle"] = doc.SemanticBundleJson(),
            ["semantic_search"] = doc.SemanticSearchJson("the"),
        };
        Assert.Contains("feature_report", reports["feature"]);
        Assert.Equal("[]", doc.SignatureReportWithOptionsJson("""{"policy_profile":"offline_strict"}"""));
        using var signatureOptions = new SignatureValidationOptions();
        signatureOptions.SetRevocationMode(SignatureRevocationMode.OfflineStrict);
        signatureOptions.SetRevocationMode(SignatureRevocationMode.OnlineStrict);
        signatureOptions.SetRevocationMode(SignatureRevocationMode.OnlineBestEffort);
        signatureOptions.SetAlgorithmPolicyJson("{\"allow_rsa_pkcs1v15\":false}");
        signatureOptions.SetPathLimits(8, 128);
        signatureOptions.AddDistrustedCertificateSha256(new string('0', 64));
        Assert.Equal("[]", doc.SignatureValidationReport(signatureOptions));
        Assert.Contains("evidence_bundle", doc.SignatureValidationWithEvidence(signatureOptions));
        Assert.False(string.IsNullOrWhiteSpace(WellfriendDocument.EngineVersion()));
        Assert.True(WellfriendDocument.AbiVersion >= 1);
        foreach (var report in reports.Values)
        {
            AssertReport(report);
        }

        using (var advanced_editing_closeoutDoc = WellfriendDocument.Open(FixturePath("multi_stream.pdf")))
        {
            var rangeModel = advanced_editing_closeoutDoc.AdvancedEditingCloseoutTextRangeAnalyzeJson();
            AssertReport(rangeModel);
            Assert.Contains("advanced_editing_closeout_multi_run_range_model", rangeModel);
            var rangeDoc = JsonDocument.Parse(rangeModel);
            var firstRange = rangeDoc.RootElement.GetProperty("report").GetProperty("source_spans")[0].GetProperty("logical_range");
            var requestJson = $$"""
            {
              "page": 1,
              "logical_start": {{firstRange[0].GetInt32()}},
              "logical_end": {{firstRange[1].GetInt32()}},
              "replacement_text": "Net20B",
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
            """;
            var rangeEdited = advanced_editing_closeoutDoc.EditTextRange(requestJson);
            Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(rangeEdited.Bytes, 0, 5));
            Assert.Contains("advanced_editing_closeout_multi_run_text_edit_report", rangeEdited.ReportJson);
            reports["advanced_editing_closeout_range_model"] = rangeModel;
            reports["advanced_editing_closeout_range_edit"] = rangeEdited.ReportJson;
        }

        var docx = doc.ToDocx();
        var faithfulDocx = doc.ToDocx("page-faithful");
        var xlsx = doc.ToXlsx();
        var pptx = doc.ToPptx();
        var sanitized = doc.Sanitize();
        var canonicalized = doc.Canonicalize(0);
        var pdfMac = doc.PdfMacCreate();
        var xfdf = doc.AnnotationXfdfExport();
        var appearances = doc.AnnotationAppearanceGenerate();
        var mediaSanitized = doc.RichMediaSanitize("remove_all_media");
        reports["sanitize"] = sanitized.ReportJson;
        reports["canonicalize"] = canonicalized.ReportJson;
        reports["pdf_mac_create"] = pdfMac.ReportJson;
        reports["xfdf"] = xfdf.ReportJson;
        reports["appearances"] = appearances.ReportJson;
        reports["media_sanitized"] = mediaSanitized.ReportJson;

        Assert.StartsWith("<?xml", Encoding.UTF8.GetString(xfdf.Bytes));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(appearances.Bytes, 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(mediaSanitized.Bytes, 0, 5));

        Assert.StartsWith("PK", Encoding.ASCII.GetString(docx, 0, 2));
        Assert.StartsWith("PK", Encoding.ASCII.GetString(faithfulDocx, 0, 2));
        Assert.StartsWith("PK", Encoding.ASCII.GetString(xlsx, 0, 2));
        Assert.StartsWith("PK", Encoding.ASCII.GetString(pptx, 0, 2));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(sanitized.Bytes, 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(canonicalized.Bytes, 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(pdfMac.Bytes, 0, 5));
        Assert.Contains("pdf_mac_create", pdfMac.ReportJson);
        using (var pdfMacDoc = WellfriendDocument.Open(pdfMac.Bytes))
        {
            Assert.Contains("\"state\":\"valid\"", pdfMacDoc.PdfMacVerifyJson());
        }
        AssertReport(sanitized.ReportJson);
        AssertReport(canonicalized.ReportJson);
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(OfficeConverters.DocxToPdf(docx), 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(OfficeConverters.XlsxToPdf(xlsx), 0, 5));
        Assert.StartsWith("%PDF-", Encoding.ASCII.GetString(OfficeConverters.PptxToPdf(pptx), 0, 5));
        WriteBindingParityArtifact(FixturePath(), reports, sanitized, canonicalized);
    }

    [Fact]
    public void RasterRenderingUsesTheCanonicalNativeSurface()
    {
        using var doc = WellfriendDocument.Open(FixturePath());
        var png = doc.GetPage(1).RenderPng();
        var jpeg = doc.GetPage(1).RenderJpeg();
        var contract = doc.DefaultRenderContractJson(1);
        var contractPng = doc.RenderPagePngWithContractJson(contract);
        Assert.True(png.Length > 8);
        Assert.Equal(new byte[] { 0x89, 0x50, 0x4E, 0x47 }, png[..4]);
        Assert.Equal(new byte[] { 0x89, 0x50, 0x4E, 0x47 }, contractPng[..4]);
        Assert.True(jpeg.Length > 4);
        Assert.Equal(new byte[] { 0xFF, 0xD8 }, jpeg[..2]);
    }

    [Fact]
    public void SignatureComponentHandlesHaveExplicitOwnershipAndCancellation()
    {
        using var doc = WellfriendDocument.Open(FixturePath());
        using var trust = new SignatureTrustStore();
        using var intermediates = new SignatureIntermediateStore();
        using var evidence = new SignatureEvidenceStore();
        using var retrieval = new SignatureRetrievalPolicy();
        using var cancellation = new SignatureValidationCancellation();
        using var options = new SignatureValidationOptions();

        evidence.AddOcspDer(Encoding.ASCII.GetBytes("untrusted-ocsp"));
        evidence.AddCrlDer(Encoding.ASCII.GetBytes("untrusted-crl"));
        retrieval.SetJson("{\"enabled\":false}");
        options.ApplyTrustStore(trust);
        options.ApplyIntermediateStore(intermediates);
        options.ApplyEvidenceStore(evidence);
        options.ApplyRetrievalPolicy(retrieval);
        options.SetCancellation(cancellation);

        cancellation.Cancel();
        var error = Assert.Throws<WellfriendPdfException>(() => doc.SignatureValidationReport(options));
        Assert.Contains("operation cancelled", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void PadesLTVTimestampAndSignaturePreservingPlanUseNativeRuntime()
    {
        var timestampJson = WellfriendDocument.TimestampTokenValidationJson(
            Encoding.ASCII.GetBytes("not-a-rfc3161-token"),
            Encoding.ASCII.GetBytes("cms-signature-value"));
        using (var parsed = JsonDocument.Parse(timestampJson))
        {
            Assert.Equal("timestamp_token_validation", parsed.RootElement.GetProperty("kind").GetString());
            var report = parsed.RootElement.GetProperty("report");
            Assert.Equal("signature_timestamp", report.GetProperty("token_type").GetString());
            Assert.Equal("malformed", report.GetProperty("status").GetString());
        }

        using var doc = WellfriendDocument.Open(FixturePath("form_160f.pdf"));
        var planJson = doc.SignaturePreservingFormPlan("name", "PadesLTV");
        using (var parsed = JsonDocument.Parse(planJson))
        {
            Assert.Equal("signature_preserving_edit_plan", parsed.RootElement.GetProperty("kind").GetString());
            var report = parsed.RootElement.GetProperty("report");
            Assert.Equal(
                "pades_ltv.tsa-dss-ltv-mdp-signature-edits.v1",
                report.GetProperty("schema_version").GetString());
            Assert.True(report.GetProperty("prefix_preservation_required").GetBoolean());
        }
    }

    [Fact]
    public void IncrementalSigningStandardsStandardsSigningAndPermissionReportsUseOwnedNativeRuntime()
    {
        var sourceBytes = File.ReadAllBytes(FixturePath());
        using var doc = WellfriendDocument.Open(sourceBytes);

        static void AssertEnvelope(string json, string kind)
        {
            using var parsed = JsonDocument.Parse(json);
            Assert.Equal(1, parsed.RootElement.GetProperty("schema_version").GetInt32());
            Assert.Equal(kind, parsed.RootElement.GetProperty("kind").GetString());
            Assert.True(parsed.RootElement.TryGetProperty("report", out _));
        }

        AssertEnvelope(doc.ValidatePdfaStandardsJson("PDF/A-2B"), "pdfa_standards_validation");
        AssertEnvelope(doc.ValidatePdfuaStandardsJson("PDF/UA-1"), "pdfua_standards_validation");
        AssertEnvelope(doc.ValidatePdfxStandardsJson("PDF/X-4"), "pdfx_standards_validation");
        AssertEnvelope(doc.ValidateAllStandardsJson(), "standards_all_validation");
        AssertReport(doc.DocMdpPermissionReportJson());
        AssertReport(doc.FieldMdpPermissionReportJson());

        using var key = RSA.Create(2048);
        var request = new System.Security.Cryptography.X509Certificates.CertificateRequest(
            "CN=Wellfriend Incremental Signing Standards .NET",
            key,
            HashAlgorithmName.SHA256,
            RSASignaturePadding.Pkcs1);
        using var certificate = request.CreateSelfSigned(
            DateTimeOffset.UtcNow.AddDays(-1),
            DateTimeOffset.UtcNow.AddDays(30));
        var keyPem = key.ExportPkcs8PrivateKeyPem();
        var certPem = certificate.ExportCertificatePem();

        var approvalPlan = doc.IncrementalSigningPlanJson(keyPem, certPem, placeholderSize: 4096);
        using (var parsed = JsonDocument.Parse(approvalPlan))
        {
            Assert.True(parsed.RootElement.GetProperty("reserved_bytes").GetInt32() >= 4096);
            Assert.True(parsed.RootElement.TryGetProperty("byte_range", out _));
        }
        var certificationPlan = doc.IncrementalSigningPlanJson(
            keyPem, certPem, placeholderSize: 4096, certify: 1);
        Assert.Contains("reserved_bytes", certificationPlan);

        var signed = doc.SignIncremental(
            keyPem,
            certPem,
            placeholderSize: 4096,
            fieldName: "IncrementalSigningStandardsDotNet",
            reason: "managed runtime smoke");
        Assert.True(signed.Bytes.Length > sourceBytes.Length);
        Assert.True(sourceBytes.AsSpan().SequenceEqual(signed.Bytes.AsSpan(0, sourceBytes.Length)));
        Assert.Contains("post_sign", signed.ReportJson);
        Assert.Contains("prefix_preserved", signed.ReportJson);
        using (var signedDocument = WellfriendDocument.Open(signed.Bytes))
        {
            Assert.NotEqual("[]", signedDocument.SignatureReportJson());
        }

        Assert.Throws<ArgumentNullException>(() =>
            doc.IncrementalSigningPlanJson(null!, certPem));
        Assert.Throws<ArgumentOutOfRangeException>(() =>
            doc.SignIncremental(keyPem, certPem, placeholderSize: 0));

        var disposed = WellfriendDocument.Open(FixturePath());
        disposed.Dispose();
        Assert.Throws<ObjectDisposedException>(() => disposed.ValidateAllStandardsJson());
    }

    [Fact]
    public void MalformedInputRaisesManagedException()
    {
        var ex = Assert.Throws<WellfriendPdfException>(() => WellfriendDocument.Open(new byte[] { 1, 2, 3, 4 }));
        Assert.NotEqual(0, ex.Status);
    }

    [Fact]
    public void PasswordOpenRoutesThroughNativeAbiWithoutLeakingSecrets()
    {
        using (var empty = WellfriendDocument.Open(FixturePath(), password: ""))
        {
            Assert.True(empty.PageCount >= 1);
        }

        var bytes = File.ReadAllBytes(FixturePath());
        using (var ignored = WellfriendDocument.Open(bytes, password: "ignored-for-unencrypted"))
        {
            Assert.True(ignored.PageCount >= 1);
        }

        const string secret = "do-not-echo-dotnet-password";
        var ex = Assert.Throws<WellfriendPdfException>(() => WellfriendDocument.Open(new byte[] { 1, 2, 3, 4 }, secret));
        Assert.DoesNotContain(secret, ex.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void FeatureReportRecordsJavaPackagingProgressAndCancellationPosture()
    {
        var feature = WellfriendDocument.FeatureReportJson();
        Assert.Contains("\"progress\"", feature);
        Assert.Contains("engine_tile_progressive_resume_supported", feature);
        Assert.Contains("\"cancellation\"", feature);
        Assert.Contains("engine_render_cancellation_supported_binding_tokens_later", feature);
        Assert.Contains("\"codec_isolation\"", feature);
        Assert.Contains("\"transparency_rendering_transparency_compositing\"", feature);
        Assert.Contains("native_foundation_with_transparency_closeout_closure", feature);
        Assert.Contains("\"transparency_closeout_transparency_closure\"", feature);
        Assert.Contains("\"wellfriendpdf_outlier_failures\":0", feature);
        Assert.Contains("\"memory_cap_mb\":4096", feature);
        Assert.Contains("\"Luminosity\"", feature);
        Assert.Contains("\"advanced_rendering_text_clipping_shading_patterns\"", feature);
        Assert.Contains("native_common_paths_with_bounded_unsupported_reports", feature);
        Assert.Contains("\"rendering_modes\":[4,5,6,7]", feature);
        Assert.Contains("\"type3_cid_rendering_type3_cid_tensor_closure\"", feature);
        Assert.Contains("complete_native_common_paths_with_reference_cluster_limits", feature);
        Assert.Contains("native_tensor_product_interior", feature);
        Assert.Contains("\"annotation_ocg_rendering_annotation_ocg_progressive_cache\"", feature);
        Assert.Contains("implemented_with_bounded_unsupported_reports", feature);
        Assert.Contains("\"renderer_validation_annotation_progressive_cache_validation\"", feature);
        Assert.Contains("implemented_and_proven", feature);
        Assert.Contains("\"schema_change\":\"additive_section_only\"", feature);
        Assert.Contains("\"multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness\"", feature);
        Assert.Contains("unsupported_color_tables_are_detected_and_reported", feature);
        Assert.Contains("\"additive_feature_report_multilingual_color_glyphs\"", feature);
        Assert.Contains("\"cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure\"", feature);
        Assert.Contains("\"implemented_with_precise_security_and_exotic_limits\"", feature);
        Assert.Contains("\"additive_feature_report_cjk_rtl_color_glyph_closeout\"", feature);
        Assert.Contains("\"color_glyph_hinting_color_glyph_hinting_cff_closure\"", feature);
        Assert.Contains("\"implemented_with_operator_level_limits\"", feature);
        Assert.Contains("\"additive_feature_report_color_glyph_hinting\"", feature);
        Assert.Contains("\"colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure\"", feature);
        Assert.Contains("\"safe_static_subset_rendered_active_constructs_blocked\"", feature);
        Assert.Contains("\"additive_feature_report_colrv_svg_bitmap\"", feature);
        Assert.Contains("\"colrv_gradient_composite_colrv1_gradient_clip_composite_closure\"", feature);
        Assert.Contains("\"implemented_with_exact_mode_limits\"", feature);
        Assert.Contains("\"additive_feature_report_colrv_gradient_composite\"", feature);
        Assert.Contains("\"porterduff_radial_color_glyph_colrv1_porterduff_radial_closure\"", feature);
        Assert.Contains("\"DestinationAtop\"", feature);
        Assert.Contains("\"implemented_with_reference_equivalence\"", feature);
        Assert.Contains("\"additive_feature_report_porterduff_radial_color_glyph\"", feature);
        Assert.Contains("\"renderer_fuzz_cmm_renderer_fuzz_cmm_closeout\"", feature);
        Assert.Contains("\"hard_blocked_precise_no_default_native_dependency\"", feature);
        Assert.Contains("\"additive_feature_report_renderer_fuzz_cmm\"", feature);
        Assert.Contains("\"native_cmm_backend_native_littlecms_cmm_backend_closure\"", feature);
        Assert.Contains("\"native-cmm-lcms2\"", feature);
        Assert.Contains("\"additive_feature_report_native_cmm_backend\"", feature);
        Assert.Contains("\"prepress_cmm_prepress_cmm_device_link_separation_plates\"", feature);
        Assert.Contains("\"additive_feature_report_prepress_cmm\"", feature);
        Assert.Contains("\"cache_key_includes_plate_state\":true", feature);
        Assert.Contains("\"nchannel_plate_prepress_nchannel_plate_reference_closure\"", feature);
        Assert.Contains("\"additive_feature_report_nchannel_plate_prepress\"", feature);
        Assert.Contains("\"required_and_run_by_nchannel_plate_prepress_audit\"", feature);
        Assert.Contains("\"prepress_proofing_full_overprint_prepress_closeout\"", feature);
        Assert.Contains("\"additive_feature_report_prepress_proofing\"", feature);
        Assert.Contains("\"wellfriendpdf_outlier_failures\":0", feature);
        Assert.Contains("\"semantic_intelligence_semantic_intelligence_parenttree_cjk_ml_layout\"", feature);
        Assert.Contains("\"additive_feature_report_semantic_intelligence\"", feature);
        Assert.Contains("\"cloud_upload_default\":false", feature);
        Assert.Contains("\"cjk_dictionary_layout_cjk_dictionary_layout_backend_closure\"", feature);
        Assert.Contains("\"additive_feature_report_cjk_dictionary_layout\"", feature);
        Assert.Contains("\"external_pack_support\":\"implemented\"", feature);
        Assert.Contains("\"local_backend_status\":\"unsupported_reported_no_runtime\"", feature);
        Assert.Contains("\"semantic_closeout_semantic_binding_rag_benchmark_closeout\"", feature);
        Assert.Contains("\"additive_feature_report_semantic_closeout\"", feature);
        Assert.Contains("\"model_can_rewrite_deterministic_text\":false", feature);
        Assert.Contains("\"blocked\":0", feature);
        Assert.Contains("\"xfa_runtime_xfa_runtime_sandbox_closure\"", feature);
        Assert.Contains("\"additive_feature_report_xfa_runtime\"", feature);
        Assert.Contains("\"scripts_disabled_events_not_executed\"", feature);
        Assert.Contains("\"annotation_media_redaction_annotation_xfdf_media_nonaxis_redaction\"", feature);
        Assert.Contains("\"additive_feature_report_annotation_media_redaction\"", feature);
        Assert.Contains("\"overlay_only_redaction_success_claims\":0", feature);
        Assert.Contains("\"crypto_writer_deterministic_writer_pubsec_aesgcm\"", feature);
        Assert.Contains("\"public_key_handler_status\":\"implemented_with_limits\"", feature);
        Assert.Contains("\"aes_gcm_decrypt_status\":\"implemented_with_limits\"", feature);
        Assert.NotNull(typeof(WellfriendDocument).GetMethod(
            nameof(WellfriendDocument.OpenPubSec),
            new[] { typeof(byte[]), typeof(byte[]), typeof(byte[]) }));
        Assert.NotNull(typeof(WellfriendDocument).GetMethod(
            nameof(WellfriendDocument.OpenPubSecPfx),
            new[] { typeof(byte[]), typeof(byte[]), typeof(byte[]) }));
        Assert.NotNull(typeof(WellfriendDocument).GetMethod(
            nameof(WellfriendDocument.PubSecEncryptPdf),
            new[] { typeof(byte[]) }));
        Assert.NotNull(typeof(WellfriendDocument).GetMethod(nameof(WellfriendDocument.PdfMacCreate)));
        Assert.Contains("crypto_tamper_test", WellfriendDocument.CryptoTamperTestJson());
        var isolation = WellfriendDocument.CodecIsolationReportJson(
            "FlateDecode",
            Encoding.UTF8.GetBytes("not-decoded-in-report-only"),
            "report_only");
        Assert.Contains("codec_isolation_report", isolation);
        Assert.Contains("report_only", isolation);
    }

    [Fact]
    public void RepeatedOpenReportAndDisposeStress()
    {
        for (var i = 0; i < 25; i++)
        {
            using var doc = WellfriendDocument.Open(FixturePath());
            Assert.True(doc.PageCount >= 1);
            Assert.Contains("\"schema_version\"", doc.SecurityReportJson());
        }
    }

    [Fact]
    public void EditingTransactionsSceneTransactionFontSurfacesAreSharedAndOwned()
    {
        using var doc = WellfriendDocument.Open(FixturePath("multi_stream.pdf"));

        var closeout = doc.EditingTransactionsReportJson();
        Assert.Contains("editing_transactions.scene-transactions-fonts-shaping.v1", closeout);

        var scene = doc.EditingTransactionsSceneReportJson("[1]");
        Assert.Contains("editing_transactions_scene_report", scene);
        Assert.Contains("\"nodes\"", scene);
        Assert.Contains("\"snapshot_id\"", scene);
        Assert.Contains("\"revision_id\"", scene);

        var request = """
        {
          "requested_mode":"operator_preserving",
          "page":1,
          "source_text":"Hello",
          "replacement_text":"HELLO"
        }
        """;
        var plan = doc.EditingTransactionsTransactionPlanJson(request);
        Assert.Contains("editing_transactions_transaction_plan", plan);
        Assert.Contains("transaction_id", plan);
        Assert.Contains("operator_preserving", plan);

        var identity = doc.EditingTransactionsTextMapJson("A\u0301B", "ltr");
        Assert.Contains("editing_transactions_text_map", identity);
        Assert.Contains("grapheme_clusters", identity);

        var subset = doc.EditingTransactionsFontSubsetPlanJson("Hello", "ltr", "reuse_embedded_subset");
        Assert.Contains("editing_transactions_font_subset_plan", subset);
        Assert.Contains("deterministic_subset_tag", subset);

        var substitution = doc.EditingTransactionsFontSubstitutionReportJson("EditingTransactionsMissingFont", "Hello", "explicit_approval_required");
        Assert.Contains("editing_transactions_font_substitution_report", substitution);
        Assert.Contains("EditingTransactionsMissingFont", substitution);
    }

    [Fact]
    public void TextReflowGeometricSemanticReflowSurfacesAreSharedAndOwned()
    {
        using var doc = WellfriendDocument.Open(FixturePath("multi_stream.pdf"));
        const string request = "{\"requested_mode\":\"geometric_block\",\"page\":1,\"source_text\":\"Hello\",\"replacement_text\":\"World\",\"region\":[10.0,10.0,260.0,90.0],\"language\":\"en\",\"hyphenation\":true,\"layout_constraints\":[{\"constraint_id\":\"dotnet_soft_height\",\"variable\":\"region_height\",\"relation\":\"ge\",\"value\":500.0,\"priority\":\"weak\"}]}";

        Assert.Contains("text_reflow.geometric-semantic-reflow.v1", doc.TextReflowReportJson());
        Assert.Contains("text_reflow_layout_analyze", doc.TextReflowLayoutAnalyzeJson(request));
        Assert.Contains("text_reflow_semantic_layout", doc.TextReflowSemanticLayoutJson());
        Assert.Contains("text_reflow_reading_order_report", doc.TextReflowReadingOrderReportJson());
        Assert.Contains("text_reflow_flow_graph_report", doc.TextReflowFlowGraphReportJson());
        Assert.Contains("text_reflow_reflow_preview", doc.TextReflowReflowPreviewJson(request));
        Assert.Contains("text_reflow_overflow_report", doc.TextReflowOverflowReportJson(request));
        var constraints = doc.TextReflowConstraintsReportJson(request);
        Assert.Contains("text_reflow_constraints_report", constraints);
        Assert.Contains("dotnet_soft_height", constraints);
        Assert.Contains("text_reflow_confidence_report", doc.TextReflowConfidenceReportJson(request));
        Assert.Contains("text_reflow_reflow_operation_report", doc.TextReflowReflowOperationReportJson(request));
        var geometric = doc.TextReflowReflowRegion(request);
        Assert.Contains("text_reflow_reflow_region", geometric.ReportJson);
        Assert.Contains(
            "text_reflow_validate_reflow_output",
            doc.TextReflowValidateReflowOutputJson(geometric.Bytes, request));
        var undo = doc.TextReflowUndoReflow(geometric.Bytes, request);
        Assert.Contains("text_reflow_undo_reflow", undo.ReportJson);
        Assert.Contains("\"byte_exact_restoration\":true", undo.ReportJson);
        Assert.Equal(File.ReadAllBytes(FixturePath("multi_stream.pdf")), undo.Bytes);
        var correctionError = Assert.Throws<WellfriendPdfException>(() =>
            doc.TextReflowReflowApproveStructureJson("{\"node\":\"reviewed\"}"));
        Assert.Contains("structure_update_failed", correctionError.Message);
    }

    [Fact]
    public void DocumentSubsystemsRuntimeSurfacesUseTheCanonicalCore()
    {
        using var doc = WellfriendDocument.Open(FixturePath("multi_stream.pdf"));
        Assert.Contains("document_subsystems.tables-math-ocr-forms-annotations.v1", doc.DocumentSubsystemsReportJson());
        Assert.Contains("document_subsystems_analyze", doc.DocumentSubsystemsAnalyzeJson());
    }

    [Fact]
    public void DisposeIsIdempotent()
    {
        var doc = WellfriendDocument.Open(FixturePath());
        doc.Dispose();
        doc.Dispose();
        Assert.Throws<ObjectDisposedException>(() => doc.PageCount);
    }

    private static string FixturePath(string name = "tracemonkey.pdf")
    {
        var env = Environment.GetEnvironmentVariable("WELLFRIENDPDF_FIXTURE_PDF");
        if (name == "tracemonkey.pdf" && !string.IsNullOrWhiteSpace(env) && File.Exists(env))
        {
            return env;
        }

        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            var candidate = Path.Combine(dir.FullName, "crates", "engine", "tests", "fixtures", name);
            if (File.Exists(candidate))
            {
                return candidate;
            }
            dir = dir.Parent;
        }

        throw new FileNotFoundException($"Could not locate {name} fixture.");
    }

    private static void AssertReport(string json)
    {
        Assert.Contains("\"schema_version\"", json);
    }

    private static void WriteBindingParityArtifact(
        string fixture,
        IReadOnlyDictionary<string, string> reports,
        WellfriendBinaryResult sanitized,
        WellfriendBinaryResult canonicalized)
    {
        var artifactDir = Environment.GetEnvironmentVariable("WELLFRIENDPDF_BINDING_PARITY_ARTIFACT_DIR");
        if (string.IsNullOrWhiteSpace(artifactDir))
        {
            return;
        }

        Directory.CreateDirectory(artifactDir);
        var payload = new
        {
            surface = "dotnet",
            fixture,
            engine_version = WellfriendDocument.EngineVersion(),
            abi_version = WellfriendDocument.AbiVersion,
            reports = reports.ToDictionary(
                kvp => kvp.Key,
                kvp => new { sha256 = Sha256(Encoding.UTF8.GetBytes(kvp.Value)), bytes = Encoding.UTF8.GetByteCount(kvp.Value) }),
            outputs = new
            {
                sanitized = new { bytes = sanitized.Bytes.Length, sha256 = Sha256(sanitized.Bytes) },
                canonicalized = new { bytes = canonicalized.Bytes.Length, sha256 = Sha256(canonicalized.Bytes) },
            },
        };
        File.WriteAllText(
            Path.Combine(artifactDir, "dotnet-smoke.json"),
            JsonSerializer.Serialize(payload, new JsonSerializerOptions { WriteIndented = true }));
    }

    private static string Sha256(byte[] bytes)
    {
        return Convert.ToHexString(SHA256.HashData(bytes)).ToLowerInvariant();
    }
}
