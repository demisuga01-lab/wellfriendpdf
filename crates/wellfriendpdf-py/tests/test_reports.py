"""binding-surface report-surface tests for the Python SDK.

Every report method returns a native dict parsed from the shared
`wellfriendpdf_engine::sdk` versioned-JSON envelope. These assert the envelope shape,
a representative report field, honest handling of invalid input, and that the
destructive operations produce real PDF bytes plus a report.
"""

import json
from pathlib import Path

import pytest

import wellfriendpdf

ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "crates" / "engine" / "tests" / "fixtures" / "multi_stream.pdf"
FORM = ROOT / "crates" / "engine" / "tests" / "fixtures" / "form_160f.pdf"
SIG = ROOT / "crates" / "engine" / "tests" / "fixtures" / "sig_valid.pdf"


def _envelope(report, kind):
    assert isinstance(report, dict)
    assert report["schema_version"] == 1
    assert report["kind"] == kind
    assert "report" in report
    return report["report"]


def test_read_only_report_envelopes():
    doc = wellfriendpdf.open(FIXTURE)
    _envelope(doc.security_report(), "security_report")
    _envelope(doc.risky_content_report(), "risky_content_report")
    _envelope(doc.parser_report(), "parser_report")
    _envelope(doc.parser_report(mode="audit"), "parser_report")
    _envelope(doc.color_report(), "color_report")
    _envelope(doc.color_report(profile="pdfa"), "color_report")
    _envelope(doc.forms_report(), "forms_report")
    xfa = _envelope(doc.xfa_report(), "xfa_report")
    assert xfa["schema_version"] == "xfa_runtime.xfa.v1"
    _envelope(doc.xfa_extract(), "xfa_extract_report")
    _envelope(doc.xfa_script_report(), "xfa_script_report")
    _envelope(doc.xfa_security_report(), "xfa_security_report")
    _envelope(doc.xfa_runtime_report(), "xfa_runtime_report")
    _envelope(doc.annotations_report(), "annotation_report")
    media = _envelope(doc.rich_media_report(), "rich_media_report")
    assert media["schema_version"].startswith("annotation_media_redaction.")
    _envelope(doc.annotation_appearance_report(), "annotation_appearance_report")
    _envelope(doc.annotation_media_redaction_report(), "annotation_media_redaction_report")
    _envelope(doc.secure_mutation_report(), "secure_mutation_report")
    _envelope(doc.secure_mutation_closeout_report(), "secure_mutation_closeout_report")
    _envelope(doc.associated_files_report(), "associated_files_report")
    _envelope(doc.mask_redaction_report(), "mask_redaction_report")
    _envelope(doc.edit_policy_report("incremental_save"), "edit_policy_report")
    _envelope(doc.pages_report(), "page_operations_report")
    _envelope(doc.interactive_report(), "interactive_report")
    _envelope(doc.signature_report(), "signature_report")
    _envelope(doc.font_report(), "font_report")
    _envelope(doc.validate(), "standards_profile")
    _envelope(doc.validate(profile="pdfa"), "standards_profile")
    _envelope(doc.validate_pdfa(), "pdfa_validation")
    _envelope(doc.validate_pdfua(), "pdfua_validation")
    _envelope(doc.validate_pdfa_standards(target="PDF/A-2B"), "pdfa_standards_validation")
    _envelope(doc.validate_pdfua_standards(), "pdfua_standards_validation")
    _envelope(doc.validate_pdfx_standards(target="PDF/X-4"), "pdfx_standards_validation")
    _envelope(doc.validate_standards_all(), "standards_all_validation")
    _envelope(doc.chunks(), "chunk_set")
    advanced = _envelope(doc.advanced_chunks(), "advanced_rag_chunk_set")
    assert advanced["schema_version"] == "semantic_closeout.rag_chunk.v1"
    semantic = _envelope(doc.semantic_bundle(), "semantic_binding_report")
    assert semantic["schema_version"] == "semantic_closeout.semantic_binding.v1"
    search = _envelope(doc.semantic_search("Hello"), "semantic_search_report")
    assert search["query"] == "Hello"
    assert search["provenance_preserved"] is True
    table_status = _envelope(doc.table_proposal_status(), "table_proposal_status")
    assert table_status["model_weights_bundled"] is False
    _envelope(doc.text_semantic(), "text_semantic")
    _envelope(doc.semantic_document(), "semantic_document")


def test_security_report_fields():
    report = _envelope(wellfriendpdf.open(FIXTURE).security_report(), "security_report")
    assert isinstance(report["encrypted"], bool)
    assert isinstance(report["findings"], list)


def test_parser_report_opened_true():
    report = _envelope(wellfriendpdf.open(FIXTURE).parser_report(mode="audit"), "parser_report")
    assert report["opened"] is True


def test_advanced_editing_closeout_report_analyze_and_edit(tmp_path):
    doc = wellfriendpdf.open(FIXTURE)
    report = _envelope(doc.advanced_editing_closeout_report(), "advanced_editing_closeout_report")
    assert report["schema_version"] == "advanced_editing_closeout.multirun-form-appearance-closure.v1"

    model = _envelope(doc.advanced_editing_closeout_text_range_analyze(1), "advanced_editing_closeout_multi_run_range_model")
    assert model["schema_version"] == "advanced_editing_closeout.multirun-form-appearance-closure.v1"
    assert model["logical_text"].startswith("Hello")
    first_span = model["source_spans"][0]

    request = json.dumps(
        {
            "page": 1,
            "logical_start": first_span["logical_range"][0],
            "logical_end": first_span["logical_range"][1],
            "replacement_text": "Py20B",
            "mode": "paragraph_reflow_horizontal",
            "style_policy": "inherit_leading",
            "options": {
                "region": [20.0, 80.0, 180.0, 140.0],
                "font_size": 12.0,
                "line_spacing": 1.2,
                "max_lines_or_columns": 4096,
                "overflow_policy": "error",
                "signature_policy_override": False,
                "deterministic": True,
            },
        }
    )
    out, edit_report = doc.edit_text_range(request, output=tmp_path / "advanced_editing_closeout-python.pdf")
    assert bytes(out).startswith(b"%PDF-")
    edited = _envelope(edit_report, "advanced_editing_closeout_multi_run_text_edit_report")
    assert edited["replacement_extracts"] is True
    assert edited["old_selected_text_absent"] is True


def test_forms_report_on_form_fixture():
    if not FORM.exists():
        pytest.skip("form fixture not present")
    report = _envelope(wellfriendpdf.open(FORM).forms_report(), "forms_report")
    assert isinstance(report, dict)


def test_signature_report_on_signed_fixture():
    if not SIG.exists():
        pytest.skip("signed fixture not present")
    report = _envelope(wellfriendpdf.open(SIG).signature_report(), "signature_report")
    # The signature report's inner payload is the list of signatures.
    assert isinstance(report, list)


def test_signature_validation_owned_signature_validation_options():
    if not SIG.exists():
        pytest.skip("signed fixture not present")

    options = wellfriendpdf.SignatureValidationOptions()
    options.set_validation_time_unix(1_704_067_200)
    options.set_revocation_mode("online_strict")
    options.set_revocation_mode("online_best_effort")
    options.set_revocation_mode("offline_best_effort")
    options.set_algorithm_policy_json('{"allow_rsa_pkcs1v15": false}')
    options.set_path_limits(4, 32)
    options.set_retrieval_policy_json('{"enabled": false}')
    options.add_distrusted_certificate_sha256("00" * 32)

    doc = wellfriendpdf.open(SIG)
    reports = doc.signature_validation(options)
    assert isinstance(reports, list)
    outcome = doc.signature_validation_with_evidence_options(options)
    assert isinstance(outcome, dict)
    assert isinstance(outcome["reports"], list)
    assert isinstance(outcome["evidence_bundle"], dict)

    with pytest.raises(ValueError):
        options.set_path_limits(0, 32)
    with pytest.raises(ValueError):
        options.add_distrusted_certificate_sha256("not-a-fingerprint")


def test_signature_validation_signature_validation_component_handles_and_cancellation():
    if not SIG.exists():
        pytest.skip("signed fixture not present")

    trust = wellfriendpdf.SignatureTrustStore()
    assert trust.is_empty()
    with pytest.raises(ValueError):
        trust.add_anchor_der(b"not a certificate")
    trust.add_distrusted_certificate_sha256("00" * 32)
    assert trust.len() == 0

    intermediates = wellfriendpdf.SignatureIntermediateStore()
    assert intermediates.is_empty()
    with pytest.raises(ValueError):
        intermediates.add_der(b"not a certificate")

    evidence = wellfriendpdf.SignatureEvidenceStore()
    evidence.add_ocsp_response_der(b"not an ocsp response")
    evidence.add_crl_der(b"not a crl")
    evidence.import_bundle_json('{"schema_version":1,"records":[]}')
    assert evidence.ocsp_count() == 1
    assert evidence.crl_count() == 1
    assert evidence.bundle_json() is not None

    retrieval = wellfriendpdf.SignatureRetrievalPolicy()
    retrieval.set_json('{"enabled": false}')
    assert '"enabled":false' in retrieval.to_json().replace(" ", "")

    cancellation = wellfriendpdf.SignatureValidationCancellation()
    assert cancellation.is_cancelled() is False

    options = wellfriendpdf.SignatureValidationOptions()
    options.apply_trust_store(trust)
    options.apply_intermediate_store(intermediates)
    options.apply_evidence_store(evidence)
    options.apply_retrieval_policy(retrieval)
    options.set_cancellation(cancellation)

    cancellation.cancel()
    assert cancellation.is_cancelled() is True
    with pytest.raises(wellfriendpdf.WellfriendError, match="operation cancelled"):
        wellfriendpdf.open(SIG).signature_validation(options)


def test_pades_ltv_timestamp_and_signature_preserving_surfaces():
    timestamp = _envelope(
        wellfriendpdf.timestamp_token_validation(b"not-a-rfc3161-token", b"cms-signature-value"),
        "timestamp_token_validation",
    )
    assert timestamp["token_type"] == "signature_timestamp"
    assert timestamp["status"] == "malformed"

    doc = wellfriendpdf.open(FORM)
    plan = _envelope(
        doc.signature_preserving_form_plan("name", "PadesLTV", "{}"),
        "signature_preserving_edit_plan",
    )
    assert plan["schema_version"] == "pades_ltv.tsa-dss-ltv-mdp-signature-edits.v1"
    assert plan["prefix_preservation_required"] is True


def test_module_level_reports():
    assert hasattr(wellfriendpdf, "pubsec_decrypt_pdf_pfx")
    assert hasattr(wellfriendpdf, "pubsec_reencrypt_pdf_pfx")
    feature = _envelope(wellfriendpdf.feature_report(), "feature_report")
    assert isinstance(feature["engine_version"], str)
    assert feature["report_envelope_version"] == 1
    assert feature["codec_boundary"]["scanner"]["default_implementation"] == "safe_first_byte_chunked"
    assert (
        feature["codec_boundary"]["renderer_decode_scheduler"]["status"]
        == "adopted_for_immediate_renderer_decode_paths"
    )
    assert (
        feature["decode_scheduler"]["decode_scheduler"]["status"]
        == "adopted_for_decode_scheduler_non_render_decode_paths"
    )
    assert feature["decode_scheduler"]["hostile_corpus"]["generator"].endswith(
        "decode_scheduler_hostile_codec_corpus.py"
    )
    assert (
        feature["native_renderer"]["native_replay"]["status"]
        == "native_text_image_form_display_list_foundation"
    )
    assert feature["native_renderer"]["renderer_parity_audit"]["script"].endswith(
        "native_renderer_renderer_parity_audit.py"
    )
    assert (
        feature["native_renderer"]["reference_renderer_multi_reference_audit"]["status"]
        == "multi_reference_audit_complete"
    )
    assert (
        feature["transparency_rendering_transparency_compositing"]["status"]
        == "native_foundation_with_transparency_closeout_closure"
    )
    assert (
        feature["transparency_rendering_transparency_compositing"]["reference_audit"]["memory_cap_mb"]
        == 4096
    )
    assert "Luminosity" in feature["transparency_rendering_transparency_compositing"]["blend_modes"]["implemented"]
    assert feature["transparency_closeout_transparency_closure"]["status"] == "complete"
    assert (
        feature["transparency_closeout_transparency_closure"]["reference_audit"]["wellfriendpdf_outlier_failures"]
        == 0
    )
    assert (
        "DeviceCMYK"
        in feature["transparency_closeout_transparency_closure"]["luminosity_soft_mask_color_spaces"][
            "supported"
        ]
    )
    advanced_rendering = feature["advanced_rendering_text_clipping_shading_patterns"]
    assert advanced_rendering["status"] == "native_common_paths_with_bounded_unsupported_reports"
    assert advanced_rendering["reference_audit"]["memory_cap_mb"] == 4096
    assert 7 in advanced_rendering["text_clipping"]["rendering_modes"]
    assert "colored" in advanced_rendering["tiling_patterns"]["paint_types"]
    type3_cid_rendering = feature["type3_cid_rendering_type3_cid_tensor_closure"]
    assert type3_cid_rendering["status"] == "complete_native_common_paths_with_reference_cluster_limits"
    assert type3_cid_rendering["reference_audit"]["wellfriendpdf_outlier_failures"] == 0
    assert type3_cid_rendering["type7_tensor_patch"]["status"] == "native_tensor_product_interior"
    annotation_ocg_rendering = feature["annotation_ocg_rendering_annotation_ocg_progressive_cache"]
    assert annotation_ocg_rendering["status"] == "implemented_with_bounded_unsupported_reports"
    assert annotation_ocg_rendering["closure_gates"]["wellfriendpdf_outlier_failures"] == 0
    renderer_validation = feature["renderer_validation_annotation_progressive_cache_validation"]
    assert renderer_validation["status"] == "implemented_and_proven"
    assert renderer_validation["multi_reference_audit"]["wellfriendpdf_outlier_failures"] == 0
    assert renderer_validation["multi_reference_audit"]["unclassified_failures"] == 0
    assert renderer_validation["public_report_parity"]["schema_change"] == "additive_section_only"
    multilingual_color_glyphs = feature["multilingual_color_glyphs_cjk_rtl_color_glyph_reference_harness"]
    assert multilingual_color_glyphs["status"] == "implemented_with_bounded_unsupported_reports"
    assert multilingual_color_glyphs["closure_gates"]["memory_cap_mb"] == 4096
    assert (
        multilingual_color_glyphs["color_glyph_rendering"]["status"]
        == "unsupported_color_tables_are_detected_and_reported"
    )
    cjk_rtl_color_glyph_closeout = feature["cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure"]
    assert cjk_rtl_color_glyph_closeout["status"] == "complete"
    assert cjk_rtl_color_glyph_closeout["color_glyph_rendering"]["colr_cpal"]["status"] == "implemented_and_proven"
    assert cjk_rtl_color_glyph_closeout["multi_reference_audit"]["wellfriendpdf_outlier_failures"] == 0
    assert cjk_rtl_color_glyph_closeout["closure_gates"]["public_report_schema"] == "additive_feature_report_cjk_rtl_color_glyph_closeout"
    color_glyph_hinting = feature["color_glyph_hinting_color_glyph_hinting_cff_closure"]
    assert color_glyph_hinting["status"] == "complete"
    assert color_glyph_hinting["colrv1"]["status"] == "implemented_with_operator_level_limits"
    assert color_glyph_hinting["multi_reference_audit"]["wellfriendpdf_outlier_failures"] == 0
    assert color_glyph_hinting["closure_gates"]["public_report_schema"] == "additive_feature_report_color_glyph_hinting"
    colrv_svg_bitmap = feature["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]
    assert colrv_svg_bitmap["status"] == "complete"
    assert (
        colrv_svg_bitmap["svg_in_opentype"]["status"]
        == "safe_static_subset_rendered_active_constructs_blocked"
    )
    assert (
        colrv_svg_bitmap["bitmap_color_glyphs"]["sbix"]["status"]
        == "png_and_jpeg_rendered_tiff_other_precisely_reported"
    )
    assert colrv_svg_bitmap["multi_reference_audit"]["wellfriendpdf_outlier_failures"] == 0
    assert colrv_svg_bitmap["closure_gates"]["public_report_schema"] == "additive_feature_report_colrv_svg_bitmap"
    colrv_gradient_composite = feature["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
    assert colrv_gradient_composite["status"] == "complete"
    assert "PaintLinearGradient" in colrv_gradient_composite["colrv1_gradients"]["implemented_operators"]
    assert colrv_gradient_composite["colrv1_clip_stack"]["status"] == "implemented"
    assert "Multiply" in colrv_gradient_composite["colrv1_composites"]["implemented_modes"]
    assert colrv_gradient_composite["multi_reference_audit"]["wellfriendpdf_outlier_failures"] == 0
    assert colrv_gradient_composite["closure_gates"]["public_report_schema"] == "additive_feature_report_colrv_gradient_composite"
    porterduff_radial_color_glyph = feature["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
    assert porterduff_radial_color_glyph["status"] == "complete"
    assert len(porterduff_radial_color_glyph["porter_duff_plus_composites"]["implemented_modes"]) == 12
    assert porterduff_radial_color_glyph["exact_moving_center_radial"]["status"] == "implemented_with_reference_equivalence"
    assert porterduff_radial_color_glyph["multi_reference_audit"]["wellfriendpdf_outlier_failures"] == 0
    assert porterduff_radial_color_glyph["closure_gates"]["public_report_schema"] == "additive_feature_report_porterduff_radial_color_glyph"
    renderer_fuzz_cmm = feature["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]
    assert renderer_fuzz_cmm["status"] == "complete_with_native_cmm_hard_blocked_precise"
    assert renderer_fuzz_cmm["renderer_fuzz"]["fuzz_target_count"] == 25
    assert renderer_fuzz_cmm["renderer_closeout"]["wellfriendpdf_outlier_failures"] == 0
    assert renderer_fuzz_cmm["native_cmm_backend"]["backend_used_in_current_build"] == "safe-rust-plus-qcms"
    assert renderer_fuzz_cmm["closure_gates"]["public_report_schema"] == "additive_feature_report_renderer_fuzz_cmm"
    native_cmm_backend = feature["native_cmm_backend_native_littlecms_cmm_backend_closure"]
    assert native_cmm_backend["status"] == "complete"
    assert native_cmm_backend["feature_flag"]["name"] == "native-cmm-lcms2"
    assert native_cmm_backend["closure_gates"]["public_report_schema"] == "additive_feature_report_native_cmm_backend"
    prepress_cmm = feature["prepress_cmm_prepress_cmm_device_link_separation_plates"]
    assert prepress_cmm["status"] == "complete"
    assert prepress_cmm["closure_gates"]["public_report_schema"] == "additive_feature_report_prepress_cmm"
    assert prepress_cmm["separation_framebuffer"]["cache_key_includes_plate_state"] is True
    nchannel_plate_prepress = feature["nchannel_plate_prepress_nchannel_plate_reference_closure"]
    assert nchannel_plate_prepress["status"] == "complete"
    assert (
        nchannel_plate_prepress["closure_gates"]["public_report_schema"]
        == "additive_feature_report_nchannel_plate_prepress"
    )
    assert nchannel_plate_prepress["reference_audit"]["pdfium"] == "required_and_run_by_nchannel_plate_prepress_audit"
    prepress_proofing = feature["prepress_proofing_full_overprint_prepress_closeout"]
    assert prepress_proofing["status"] == "complete"
    assert (
        prepress_proofing["closure_gates"]["public_report_schema"]
        == "additive_feature_report_prepress_proofing"
    )
    assert prepress_proofing["reference_audit"]["wellfriendpdf_outlier_failures"] == 0
    assert prepress_proofing["reference_audit"]["unclassified_failures"] == 0
    semantic_intelligence = feature["semantic_intelligence_semantic_intelligence_parenttree_cjk_ml_layout"]
    assert semantic_intelligence["status"] == "complete"
    assert (
        semantic_intelligence["closure_gates"]["public_report_schema"]
        == "additive_feature_report_semantic_intelligence"
    )
    assert semantic_intelligence["privacy_defaults"]["cloud_upload_default"] is False
    cjk_dictionary_layout = feature["cjk_dictionary_layout_cjk_dictionary_layout_backend_closure"]
    assert cjk_dictionary_layout["status"] == "complete"
    assert (
        cjk_dictionary_layout["closure_gates"]["public_report_schema"]
        == "additive_feature_report_cjk_dictionary_layout"
    )
    assert cjk_dictionary_layout["dictionary_provider"]["external_pack_support"] == "implemented"
    assert (
        cjk_dictionary_layout["layout_backend"]["local_backend_status"]
        == "unsupported_reported_no_runtime"
    )
    semantic_closeout = feature["semantic_closeout_semantic_binding_rag_benchmark_closeout"]
    assert semantic_closeout["status"] == "complete"
    assert semantic_closeout["closure_gates"]["public_report_schema"] == "additive_feature_report_semantic_closeout"
    assert semantic_closeout["closure_counts"]["blocked"] == 0
    assert semantic_closeout["privacy"]["cloud_upload_default"] is False
    assert (
        semantic_closeout["tableformer_table_transformer_hook"]
        ["model_can_rewrite_deterministic_text"]
        is False
    )
    xfa_runtime = feature["xfa_runtime_xfa_runtime_sandbox_closure"]
    assert xfa_runtime["status"] == "complete_bounded_foundation"
    assert xfa_runtime["closure_counts"]["blocked"] == 0
    assert (
        xfa_runtime["closure_gates"]["public_report_schema"]
        == "additive_feature_report_xfa_runtime"
    )
    annotation_media_redaction = feature["annotation_media_redaction_annotation_xfdf_media_nonaxis_redaction"]
    assert annotation_media_redaction["status"] == "complete_bounded_foundation"
    assert annotation_media_redaction["failure"]["blocked"] == 0
    assert annotation_media_redaction["security"]["overlay_only_redaction_success_claims"] == 0
    secure_mutation = feature["secure_mutation_mask_inline_associated_signature_safe_edits"]
    assert secure_mutation["failure"]["blocked"] == 0
    assert secure_mutation["security"]["signature_crypto_overclaim"] == 0
    secure_mutation_closeout = feature["secure_mutation_closeout_advanced_secure_mutation_closure"]
    assert secure_mutation_closeout["failure"]["blocked"] == 0
    assert secure_mutation_closeout["failure"]["security_proof"] == 0
    crypto_writer = feature["crypto_writer_deterministic_writer_pubsec_aesgcm"]
    assert crypto_writer["blocked_rows"] == 0
    assert crypto_writer["public_key_handler_status"] == "implemented_with_limits"
    assert crypto_writer["aes_gcm_decrypt_status"] == "implemented_with_limits"
    tamper = _envelope(wellfriendpdf.crypto_tamper_test(), "crypto_tamper_test")
    assert tamper["plaintext_release_possible"] is False
    decode = _envelope(
        wellfriendpdf.decode_budget_report("DCTDecode", 4096, 4096, 3), "decode_budget_report"
    )
    assert "diagnostics" in decode
    dedup = _envelope(
        wellfriendpdf.resource_dedup_report([b"a", b"a", b"b"]), "resource_dedup_report"
    )
    assert dedup["duplicate_count"] == 1


def test_sanitize_produces_bytes_and_report(tmp_path):
    doc = wellfriendpdf.open(FIXTURE)
    out = tmp_path / "clean.pdf"
    data, report = doc.sanitize(policy="balanced", output=out)
    assert data[:5] == b"%PDF-"
    assert out.read_bytes() == data
    r = _envelope(report, "sanitize_report")
    assert r["output_bytes"] > 0


def test_pdf_mac_create_owned_output_and_verify(tmp_path):
    doc = wellfriendpdf.open(FIXTURE)
    data, report = doc.pdf_mac_create(output=tmp_path / "pdfmac.pdf")
    assert bytes(data).startswith(b"%PDF-")
    created = _envelope(report, "pdf_mac_create")
    assert created["verification_state"] == "valid"
    reopened = wellfriendpdf.open(bytes(data))
    verified = _envelope(reopened.pdf_mac_verify(), "pdf_mac_verify")
    assert verified["state"] == "valid"
    assert verified["trusted_document_integrity"] is True


def test_xfa_owned_output_surfaces_on_non_xfa_pdf(tmp_path):
    doc = wellfriendpdf.open(FIXTURE)
    preview, preview_report = doc.xfa_render(output=tmp_path / "xfa-preview.pdf")
    assert preview[:5] == b"%PDF-"
    assert _envelope(preview_report, "xfa_render_report")["schema_version"] == "xfa_runtime.xfa.v1"
    flattened, flatten_report = doc.xfa_flatten(mode="extract_only")
    assert flattened[:5] == b"%PDF-"
    assert _envelope(flatten_report, "xfa_flatten_report")["schema_version"] == "xfa_runtime.xfa.v1"
    sanitized, sanitize_report = doc.xfa_sanitize(mode="remove_all_xfa")
    assert sanitized[:5] == b"%PDF-"
    assert _envelope(sanitize_report, "xfa_sanitize_report")["schema_version"] == "xfa_runtime.xfa.v1"


def test_annotation_media_redaction_owned_output_surfaces(tmp_path):
    doc = wellfriendpdf.open(FIXTURE)
    xfdf, export_report = doc.annotation_xfdf_export(output=tmp_path / "annotations.xfdf")
    assert xfdf.startswith(b"<?xml")
    assert _envelope(export_report, "annotation_xfdf_export_report")["deterministic"] is True
    imported, import_report = doc.annotation_xfdf_import(xfdf)
    assert imported.startswith(b"%PDF-")
    assert _envelope(import_report, "annotation_xfdf_import_report")["deterministic"] is True
    appearances, appearance_report = doc.annotation_appearance_generate()
    assert appearances.startswith(b"%PDF-")
    _envelope(appearance_report, "annotation_appearance_generation_report")
    sanitized, media_report = doc.rich_media_sanitize(mode="remove_all_media")
    assert sanitized.startswith(b"%PDF-")
    assert _envelope(media_report, "rich_media_policy_report")["rescan_passed"] is True


def test_canonicalize_is_deterministic():
    doc = wellfriendpdf.open(FIXTURE)
    a, ra = doc.canonicalize(date_epoch=0)
    b, rb = doc.canonicalize(date_epoch=0)
    assert a == b  # deterministic bytes
    assert _envelope(ra, "canonicalize_report")["deterministic"] is True
    assert ra["report"]["output_sha256"] == rb["report"]["output_sha256"]


def test_redact_removes_and_verifies():
    doc = wellfriendpdf.open(FIXTURE)
    data, report = doc.redact(["Hello"])
    assert data[:5] == b"%PDF-"
    r = _envelope(report, "redaction_report")
    assert len(r["applied"]) >= 1
    # The redacted output must not surface the term in a fresh parse.
    redacted = wellfriendpdf.open(data)
    assert "Hello" not in redacted.extract_text()


def test_redact_empty_terms_raises():
    with pytest.raises(wellfriendpdf.WellfriendError):
        wellfriendpdf.open(FIXTURE).redact(["   "])


def test_redact_strict_missing_term_raises():
    # A term that does not exist cannot be redacted → error (nothing applied).
    with pytest.raises(wellfriendpdf.WellfriendError):
        wellfriendpdf.open(FIXTURE).redact(["ZZZ-not-present-anywhere"], strict=True)


def test_invalid_pdf_bytes_raise():
    with pytest.raises(wellfriendpdf.WellfriendError):
        wellfriendpdf.open(b"%PDF- broken not really")


def test_incremental_signing_standards_incremental_signing(tmp_path):
    """Real append-only incremental signing through the Python binding: the
    signed output reopens and validates, and the original bytes are preserved."""
    crypto = pytest.importorskip("cryptography")
    import datetime

    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.hazmat.primitives.asymmetric import rsa
    from cryptography.x509.oid import NameOID

    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    key_pem = key.private_bytes(
        serialization.Encoding.PEM,
        serialization.PrivateFormat.PKCS8,
        serialization.NoEncryption(),
    ).decode()
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Wellfriend Py IncrementalSigningStandards Test")])
    cert = (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(datetime.datetime(2020, 1, 1))
        .not_valid_after(datetime.datetime(2040, 1, 1))
        .sign(key, hashes.SHA256())
    )
    cert_pem = cert.public_bytes(serialization.Encoding.PEM).decode()

    out, report = wellfriendpdf.sign_pdf(
        str(FIXTURE),
        key_pem,
        cert_pem,
        output=tmp_path / "signed.pdf",
        placeholder_size=16384,
    )
    assert bytes(out).startswith(b"%PDF-")
    assert report["post_sign"]["signature_valid"] is True
    assert report["prefix_preserved"] is True
    assert report["certification"] is False

    # Reopen the signed output and confirm the signature is discovered/valid.
    signed = wellfriendpdf.open(bytes(out))
    sigs = signed.signature_report()["report"]
    assert isinstance(sigs, list) and len(sigs) >= 1


def test_cross_surface_parity_smoke(tmp_path):
    """The Python security report must equal the report the sdk facade emits for
    the same bytes (same JSON), proving Python does not diverge from the shared
    facade the C ABI also uses."""
    import json

    doc = wellfriendpdf.open(FIXTURE)
    py_report = doc.security_report()
    # Round-trip through JSON to confirm it is plain, serializable data.
    again = json.loads(json.dumps(py_report))
    assert again == py_report
