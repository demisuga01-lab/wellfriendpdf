package io.wellfriendpdf.packagesmoke;

import java.nio.file.Files;
import java.nio.file.Path;

import io.wellfriendpdf.Wellfriend;

public final class PackageSmoke {
    private PackageSmoke() {
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            throw new IllegalArgumentException("usage: PackageSmoke <fixture.pdf>");
        }
        Path fixture = Path.of(args[0]);
        if (!Files.exists(fixture)) {
            throw new IllegalArgumentException("fixture does not exist: " + fixture);
        }
        WellfriendPdf.Document.class.getMethod("open", Path.class, String.class);
        WellfriendPdf.Document.class.getMethod("open", byte[].class, String.class);
        WellfriendPdf.Document.class.getMethod("openPubSec", byte[].class, byte[].class, byte[].class);
        WellfriendPdf.Document.class.getMethod("openPubSecPfx", byte[].class, byte[].class, byte[].class);
        WellfriendPdf.Document.class.getMethod("pubsecEncryptPdf", byte[].class);
        WellfriendPdf.Document.class.getMethod("pdfMacCreate");
        WellfriendPdf.Document.class.getMethod("validatePdfaStandardsJson", String.class);
        WellfriendPdf.Document.class.getMethod("validatePdfuaStandardsJson", String.class);
        WellfriendPdf.Document.class.getMethod("validatePdfxStandardsJson", String.class);
        WellfriendPdf.Document.class.getMethod("validateAllStandardsJson", String.class);
        WellfriendPdf.Document.class.getMethod("incrementalSigningPlanJson", String.class, String.class, long.class, int.class);
        WellfriendPdf.Document.class.getMethod("signIncremental", String.class, String.class, long.class, int.class, String.class, String.class);
        if (WellfriendPdf.engineVersion().isBlank() || WellfriendPdf.abiVersion() < 1) {
            throw new AssertionError("version queries failed");
        }

        try (WellfriendPdf.Document doc = WellfriendPdf.Document.open(fixture, "")) {
            if (doc.pageCount() < 1) {
                throw new AssertionError("expected at least one page");
            }
            if (doc.page(1).text().isBlank()) {
                throw new AssertionError("text extraction returned blank");
            }
            String security = doc.securityReportJson();
            if (!doc.validatePdfaStandardsJson("PDF/A-2B").contains("pdfa_standards_validation")
                    || !doc.validatePdfuaStandardsJson("PDF/UA-1").contains("pdfua_standards_validation")
                    || !doc.validatePdfxStandardsJson("PDF/X-4").contains("pdfx_standards_validation")
                    || !doc.validateAllStandardsJson().contains("standards_all_validation")) {
                throw new AssertionError("IncrementalSigningStandards standards runtime surface missing");
            }
            if (!security.contains("\"schema_version\"")) {
                throw new AssertionError("security report missing schema_version");
            }
            String parser = doc.parserReportJson("repair");
            if (!parser.contains("\"schema_version\"")) {
                throw new AssertionError("parser report missing schema_version");
            }
            if (!doc.advancedChunksJson().contains("advanced_rag_chunk_set")
                    || !doc.semanticBundleJson().contains("semantic_binding_report")
                    || !doc.semanticSearchJson("the").contains("semantic_search_report")) {
                throw new AssertionError("Semantic Closeout semantic report surface missing");
            }
            if (!doc.advanced_editing_closeoutReportJson().contains("advanced_editing_closeout.multirun-form-appearance-closure.v1")
                    || !doc.advanced_editing_closeoutTextRangeAnalyzeJson(1).contains("advanced_editing_closeout_multi_run_range_model")) {
                throw new AssertionError("advanced editing closeout report/range surfaces missing");
            }
            if (!doc.writer_historyReportJson().contains("writer_history.raster-vector-font-persistent-object-stream.v1")
                    || !doc.writer_historyRasterVectorReportJson(1, "").contains("writer_history_raster_vector_report")
                    || !doc.writer_historyFontReconstructionReportJson().contains("writer_history_font_reconstruction_report")
                    || !doc.writer_historyObjectStreamReportJson().contains("writer_history_object_stream_report")) {
                throw new AssertionError("writer history report surfaces missing");
            }
            WellfriendPdf.BinaryResult writer_historyPacked = doc.writer_historyPackObjectStreams();
            if (writer_historyPacked.bytes().length == 0
                    || !writer_historyPacked.reportJson().contains("writer_history_pack_object_streams_report")) {
                throw new AssertionError("writer history packed output/report missing");
            }
            WellfriendPdf.BinaryResult sanitized = doc.sanitize("balanced");
            if (sanitized.bytes().length == 0 || !sanitized.reportJson().contains("sanitize_report")) {
                throw new AssertionError("sanitize output/report missing");
            }
            WellfriendPdf.BinaryResult pdfMac = doc.pdfMacCreate();
            if (pdfMac.bytes().length == 0 || !pdfMac.reportJson().contains("pdf_mac_create")) {
                throw new AssertionError("PDF-MAC output/report missing");
            }
            try (WellfriendPdf.Document pdfMacDoc = WellfriendPdf.Document.open(pdfMac.bytes())) {
                if (!pdfMacDoc.pdfMacVerifyJson().contains("\"state\":\"valid\"")) {
                    throw new AssertionError("PDF-MAC verification did not return valid");
                }
            }
        }

        String feature = WellfriendPdf.featureReportJson();
        if (!feature.contains("engine_tile_progressive_resume_supported")
                || !feature.contains("engine_render_cancellation_supported_binding_tokens_later")) {
            throw new AssertionError("feature report missing Annotation Ocg Rendering progress/cancellation posture");
        }
        if (!feature.contains("\"transparency_closeout_transparency_closure\"")
                || !feature.contains("\"wellfriendpdf_outlier_failures\":0")) {
            throw new AssertionError("feature report missing Transparency Closeout transparency closure posture");
        }
        if (!feature.contains("\"advanced_rendering_text_clipping_shading_patterns\"")
                || !feature.contains("\"rendering_modes\":[4,5,6,7]")) {
            throw new AssertionError("feature report missing Advanced Rendering text/shading/pattern posture");
        }
        if (!feature.contains("\"type3_cid_rendering_type3_cid_tensor_closure\"")
                || !feature.contains("complete_native_common_paths_with_reference_cluster_limits")
                || !feature.contains("native_tensor_product_interior")) {
            throw new AssertionError("feature report missing Type3 CID Rendering Type3/CID/tensor closure posture");
        }
        if (!feature.contains("\"annotation_ocg_rendering_annotation_ocg_progressive_cache\"")
                || !feature.contains("implemented_with_bounded_unsupported_reports")) {
            throw new AssertionError("feature report missing Annotation Ocg Rendering renderer posture");
        }
        if (!feature.contains("\"renderer_validation_annotation_progressive_cache_validation\"")
                || !feature.contains("implemented_and_proven")
                || !feature.contains("\"schema_change\":\"additive_section_only\"")) {
            throw new AssertionError("feature report missing Renderer Validation validation posture");
        }
        if (!feature.contains("\"multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness\"")
                || !feature.contains("unsupported_color_tables_are_detected_and_reported")
                || !feature.contains("\"additive_feature_report_multilingual_color_glyphs\"")) {
            throw new AssertionError("feature report missing Multilingual Color Glyphs CJK/RTL/color glyph reference posture");
        }
        if (!feature.contains("\"cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure\"")
                || !feature.contains("\"implemented_with_precise_security_and_exotic_limits\"")
                || !feature.contains("\"additive_feature_report_cjk_rtl_color_glyph_closeout\"")) {
            throw new AssertionError("feature report missing CJK RTL Color Glyph Closeout color glyph closure posture");
        }
        if (!feature.contains("\"color_glyph_hinting_color_glyph_hinting_cff_closure\"")
                || !feature.contains("\"implemented_with_operator_level_limits\"")
                || !feature.contains("\"additive_feature_report_color_glyph_hinting\"")) {
            throw new AssertionError("feature report missing Color Glyph Hinting color glyph hinting CFF closure posture");
        }
        if (!feature.contains("\"colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure\"")
                || !feature.contains("\"safe_static_subset_rendered_active_constructs_blocked\"")
                || !feature.contains("\"additive_feature_report_colrv_svg_bitmap\"")) {
            throw new AssertionError("feature report missing Colrv Svg Bitmap full color glyph closure posture");
        }
        if (!feature.contains("\"colrv_gradient_composite_colrv1_gradient_clip_composite_closure\"")
                || !feature.contains("\"implemented_with_exact_mode_limits\"")
                || !feature.contains("\"additive_feature_report_colrv_gradient_composite\"")) {
            throw new AssertionError("feature report missing Colrv Gradient Composite COLRv1 gradient clip composite posture");
        }
        if (!feature.contains("\"porterduff_radial_color_glyph_colrv1_porterduff_radial_closure\"")
                || !feature.contains("\"DestinationAtop\"")
                || !feature.contains("\"additive_feature_report_porterduff_radial_color_glyph\"")) {
            throw new AssertionError("feature report missing Porterduff Radial Color Glyph Porter-Duff radial closure posture");
        }
        if (!feature.contains("\"renderer_fuzz_cmm_renderer_fuzz_cmm_closeout\"")
                || !feature.contains("\"hard_blocked_precise_no_default_native_dependency\"")
                || !feature.contains("\"additive_feature_report_renderer_fuzz_cmm\"")) {
            throw new AssertionError("feature report missing Renderer Fuzz CMM renderer fuzz CMM closeout posture");
        }
        if (!feature.contains("\"native_cmm_backend_native_littlecms_cmm_backend_closure\"")
                || !feature.contains("\"native-cmm-lcms2\"")
                || !feature.contains("\"additive_feature_report_native_cmm_backend\"")) {
            throw new AssertionError("feature report missing Native CMM Backend native LittleCMS CMM posture");
        }
        if (!feature.contains("\"prepress_cmm_prepress_cmm_device_link_separation_plates\"")
                || !feature.contains("\"additive_feature_report_prepress_cmm\"")
                || !feature.contains("\"cache_key_includes_plate_state\":true")) {
            throw new AssertionError("feature report missing Prepress CMM prepress CMM plate posture");
        }
        if (!feature.contains("\"nchannel_plate_prepress_nchannel_plate_reference_closure\"")
                || !feature.contains("\"additive_feature_report_nchannel_plate_prepress\"")
                || !feature.contains("\"required_and_run_by_nchannel_plate_prepress_audit\"")) {
            throw new AssertionError("feature report missing Nchannel Plate Prepress n-channel plate closure posture");
        }
        if (!feature.contains("\"prepress_proofing_full_overprint_prepress_closeout\"")
                || !feature.contains("\"additive_feature_report_prepress_proofing\"")
                || !feature.contains("\"wellfriendpdf_outlier_failures\":0")) {
            throw new AssertionError("feature report missing Prepress Proofing overprint prepress closeout posture");
        }
        if (!feature.contains("\"semantic_intelligence_semantic_intelligence_parenttree_cjk_ml_layout\"")
                || !feature.contains("\"additive_feature_report_semantic_intelligence\"")
                || !feature.contains("\"cloud_upload_default\":false")) {
            throw new AssertionError("feature report missing Semantic Intelligence semantic intelligence posture");
        }
        if (!feature.contains("\"cjk_dictionary_layout_cjk_dictionary_layout_backend_closure\"")
                || !feature.contains("\"additive_feature_report_cjk_dictionary_layout\"")
                || !feature.contains("\"external_pack_support\":\"implemented\"")
                || !feature.contains("\"local_backend_status\":\"unsupported_reported_no_runtime\"")) {
            throw new AssertionError("feature report missing CJK Dictionary Layout dictionary provider closure posture");
        }
        if (!feature.contains("\"semantic_closeout_semantic_binding_rag_benchmark_closeout\"")
                || !feature.contains("\"additive_feature_report_semantic_closeout\"")
                || !feature.contains("\"model_can_rewrite_deterministic_text\":false")
                || !feature.contains("\"blocked\":0")) {
            throw new AssertionError("feature report missing Semantic Closeout semantic closeout posture");
        }
        if (!feature.contains("\"advanced_editing_closeout_multirun_form_appearance_closure\"")
                || !feature.contains("\"annotation_appearance_clone_one\"")
                || !feature.contains("\"binding_parity\"")) {
            throw new AssertionError("feature report missing advanced editing closeout closure posture");
        }
        if (!feature.contains("\"writer_history_raster_vector_font_persistent_object_stream\"")
                || !feature.contains("\"object_stream_packing\"")
                || !feature.contains("\"writer_mode\":\"XrefStreamWithObjStm\"")
                || !WellfriendPdf.writer_historyHistoryReportJson().contains("writer_history_history_report")) {
            throw new AssertionError("feature report missing writer history closure posture");
        }
        if (!feature.contains("\"crypto_writer_deterministic_writer_pubsec_aesgcm\"")
                || !feature.contains("\"public_key_handler_status\":\"implemented_with_limits\"")
                || !feature.contains("\"aes_gcm_decrypt_status\":\"implemented_with_limits\"")
                || !WellfriendPdf.cryptoTamperTestJson().contains("crypto_tamper_test")) {
            throw new AssertionError("feature report missing crypto writer writer/crypto posture");
        }
    }
}
