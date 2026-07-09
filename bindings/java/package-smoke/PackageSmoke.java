package org.oxidepdf.packagesmoke;

import java.nio.file.Files;
import java.nio.file.Path;

import org.oxidepdf.Oxide;

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
        Oxide.Document.class.getMethod("open", Path.class, String.class);
        Oxide.Document.class.getMethod("open", byte[].class, String.class);
        if (Oxide.engineVersion().isBlank() || Oxide.abiVersion() < 1) {
            throw new AssertionError("version queries failed");
        }

        try (Oxide.Document doc = Oxide.Document.open(fixture, "")) {
            if (doc.pageCount() < 1) {
                throw new AssertionError("expected at least one page");
            }
            if (doc.page(1).text().isBlank()) {
                throw new AssertionError("text extraction returned blank");
            }
            String security = doc.securityReportJson();
            if (!security.contains("\"schema_version\"")) {
                throw new AssertionError("security report missing schema_version");
            }
            String parser = doc.parserReportJson("repair");
            if (!parser.contains("\"schema_version\"")) {
                throw new AssertionError("parser report missing schema_version");
            }
            Oxide.BinaryResult sanitized = doc.sanitize("balanced");
            if (sanitized.bytes().length == 0 || !sanitized.reportJson().contains("sanitize_report")) {
                throw new AssertionError("sanitize output/report missing");
            }
        }

        String feature = Oxide.featureReportJson();
        if (!feature.contains("engine_tile_progressive_resume_supported")
                || !feature.contains("engine_render_cancellation_supported_binding_tokens_later")) {
            throw new AssertionError("feature report missing Prompt 09 progress/cancellation posture");
        }
        if (!feature.contains("\"prompt07b_transparency_closure\"")
                || !feature.contains("\"oxide_outlier_failures\":0")) {
            throw new AssertionError("feature report missing Prompt 07B transparency closure posture");
        }
        if (!feature.contains("\"prompt08_text_clipping_shading_patterns\"")
                || !feature.contains("\"rendering_modes\":[4,5,6,7]")) {
            throw new AssertionError("feature report missing Prompt 08 text/shading/pattern posture");
        }
        if (!feature.contains("\"prompt08b_type3_cid_tensor_closure\"")
                || !feature.contains("complete_native_common_paths_with_reference_cluster_limits")
                || !feature.contains("native_tensor_product_interior")) {
            throw new AssertionError("feature report missing Prompt 08B Type3/CID/tensor closure posture");
        }
        if (!feature.contains("\"prompt09_annotation_ocg_progressive_cache\"")
                || !feature.contains("implemented_with_bounded_unsupported_reports")) {
            throw new AssertionError("feature report missing Prompt 09 renderer posture");
        }
        if (!feature.contains("\"prompt09b_annotation_progressive_cache_validation\"")
                || !feature.contains("implemented_and_proven")
                || !feature.contains("\"schema_change\":\"additive_section_only\"")) {
            throw new AssertionError("feature report missing Prompt 09B validation posture");
        }
        if (!feature.contains("\"prompt10_cjk_rtl_color_glyph_reference_harness\"")
                || !feature.contains("unsupported_color_tables_are_detected_and_reported")
                || !feature.contains("\"additive_feature_report_prompt10\"")) {
            throw new AssertionError("feature report missing Prompt 10 CJK/RTL/color glyph reference posture");
        }
        if (!feature.contains("\"prompt10b_color_glyph_cjk_rtl_fidelity_closure\"")
                || !feature.contains("\"implemented_with_precise_security_and_exotic_limits\"")
                || !feature.contains("\"additive_feature_report_prompt10b\"")) {
            throw new AssertionError("feature report missing Prompt 10B color glyph closure posture");
        }
        if (!feature.contains("\"prompt10c_color_glyph_hinting_cff_closure\"")
                || !feature.contains("\"implemented_with_operator_level_limits\"")
                || !feature.contains("\"additive_feature_report_prompt10c\"")) {
            throw new AssertionError("feature report missing Prompt 10C color glyph hinting CFF closure posture");
        }
        if (!feature.contains("\"prompt10d_full_colrv1_svg_color_glyph_closure\"")
                || !feature.contains("\"safe_static_subset_rendered_active_constructs_blocked\"")
                || !feature.contains("\"additive_feature_report_prompt10d\"")) {
            throw new AssertionError("feature report missing Prompt 10D full color glyph closure posture");
        }
        if (!feature.contains("\"prompt10e_colrv1_gradient_clip_composite_closure\"")
                || !feature.contains("\"implemented_with_exact_mode_limits\"")
                || !feature.contains("\"additive_feature_report_prompt10e\"")) {
            throw new AssertionError("feature report missing Prompt 10E COLRv1 gradient clip composite posture");
        }
        if (!feature.contains("\"prompt10f_colrv1_porterduff_radial_closure\"")
                || !feature.contains("\"DestinationAtop\"")
                || !feature.contains("\"additive_feature_report_prompt10f\"")) {
            throw new AssertionError("feature report missing Prompt 10F Porter-Duff radial closure posture");
        }
        if (!feature.contains("\"prompt11_renderer_fuzz_cmm_closeout\"")
                || !feature.contains("\"hard_blocked_precise_no_default_native_dependency\"")
                || !feature.contains("\"additive_feature_report_prompt11\"")) {
            throw new AssertionError("feature report missing Prompt 11 renderer fuzz CMM closeout posture");
        }
        if (!feature.contains("\"prompt11b_native_littlecms_cmm_backend_closure\"")
                || !feature.contains("\"native-cmm-lcms2\"")
                || !feature.contains("\"additive_feature_report_prompt11b\"")) {
            throw new AssertionError("feature report missing Prompt 11B native LittleCMS CMM posture");
        }
        if (!feature.contains("\"prompt12_prepress_cmm_device_link_separation_plates\"")
                || !feature.contains("\"additive_feature_report_prompt12\"")
                || !feature.contains("\"cache_key_includes_plate_state\":true")) {
            throw new AssertionError("feature report missing Prompt 12 prepress CMM plate posture");
        }
        if (!feature.contains("\"prompt12b_nchannel_plate_reference_closure\"")
                || !feature.contains("\"additive_feature_report_prompt12b\"")
                || !feature.contains("\"required_and_run_by_prompt12b_audit\"")) {
            throw new AssertionError("feature report missing Prompt 12B n-channel plate closure posture");
        }
        if (!feature.contains("\"prompt13_full_overprint_prepress_closeout\"")
                || !feature.contains("\"additive_feature_report_prompt13\"")
                || !feature.contains("\"oxide_outlier_failures\":0")) {
            throw new AssertionError("feature report missing Prompt 13 overprint prepress closeout posture");
        }
        if (!feature.contains("\"prompt14_semantic_intelligence_parenttree_cjk_ml_layout\"")
                || !feature.contains("\"additive_feature_report_prompt14\"")
                || !feature.contains("\"cloud_upload_default\":false")) {
            throw new AssertionError("feature report missing Prompt 14 semantic intelligence posture");
        }
        if (!feature.contains("\"prompt14b_cjk_dictionary_layout_backend_closure\"")
                || !feature.contains("\"additive_feature_report_prompt14b\"")
                || !feature.contains("\"external_pack_support\":\"implemented\"")
                || !feature.contains("\"local_backend_status\":\"unsupported_reported_no_runtime\"")) {
            throw new AssertionError("feature report missing Prompt 14B dictionary provider closure posture");
        }
    }
}
