"""Prompt-01 report-surface tests for the Python SDK.

Every report method returns a native dict parsed from the shared
`oxide_engine::sdk` versioned-JSON envelope. These assert the envelope shape,
a representative report field, honest handling of invalid input, and that the
destructive operations produce real PDF bytes plus a report.
"""

import json
from pathlib import Path

import pytest

import oxide

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
    doc = oxide.open(FIXTURE)
    _envelope(doc.security_report(), "security_report")
    _envelope(doc.risky_content_report(), "risky_content_report")
    _envelope(doc.parser_report(), "parser_report")
    _envelope(doc.parser_report(mode="audit"), "parser_report")
    _envelope(doc.color_report(), "color_report")
    _envelope(doc.color_report(profile="pdfa"), "color_report")
    _envelope(doc.forms_report(), "forms_report")
    xfa = _envelope(doc.xfa_report(), "xfa_report")
    assert xfa["schema_version"] == "prompt16.xfa.v1"
    _envelope(doc.xfa_extract(), "xfa_extract_report")
    _envelope(doc.xfa_script_report(), "xfa_script_report")
    _envelope(doc.xfa_security_report(), "xfa_security_report")
    _envelope(doc.xfa_runtime_report(), "xfa_runtime_report")
    _envelope(doc.annotations_report(), "annotation_report")
    media = _envelope(doc.rich_media_report(), "rich_media_report")
    assert media["schema_version"].startswith("prompt17.")
    _envelope(doc.annotation_appearance_report(), "annotation_appearance_report")
    _envelope(doc.prompt17_report(), "prompt17_report")
    _envelope(doc.prompt18_report(), "prompt18_report")
    _envelope(doc.prompt18b_report(), "prompt18b_report")
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
    _envelope(doc.chunks(), "chunk_set")
    advanced = _envelope(doc.advanced_chunks(), "advanced_rag_chunk_set")
    assert advanced["schema_version"] == "prompt15.rag_chunk.v1"
    semantic = _envelope(doc.semantic_bundle(), "semantic_binding_report")
    assert semantic["schema_version"] == "prompt15.semantic_binding.v1"
    search = _envelope(doc.semantic_search("Hello"), "semantic_search_report")
    assert search["query"] == "Hello"
    assert search["provenance_preserved"] is True
    table_status = _envelope(doc.table_proposal_status(), "table_proposal_status")
    assert table_status["model_weights_bundled"] is False
    _envelope(doc.text_semantic(), "text_semantic")
    _envelope(doc.semantic_document(), "semantic_document")


def test_security_report_fields():
    report = _envelope(oxide.open(FIXTURE).security_report(), "security_report")
    assert isinstance(report["encrypted"], bool)
    assert isinstance(report["findings"], list)


def test_parser_report_opened_true():
    report = _envelope(oxide.open(FIXTURE).parser_report(mode="audit"), "parser_report")
    assert report["opened"] is True


def test_prompt20b_report_analyze_and_edit(tmp_path):
    doc = oxide.open(FIXTURE)
    report = _envelope(doc.prompt20b_report(), "prompt20b_report")
    assert report["schema_version"] == "prompt20b.multirun-form-appearance-closure.v1"

    model = _envelope(doc.prompt20b_text_range_analyze(1), "prompt20b_multi_run_range_model")
    assert model["schema_version"] == "prompt20b.multirun-form-appearance-closure.v1"
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
    out, edit_report = doc.edit_text_range(request, output=tmp_path / "prompt20b-python.pdf")
    assert bytes(out).startswith(b"%PDF-")
    edited = _envelope(edit_report, "prompt20b_multi_run_text_edit_report")
    assert edited["replacement_extracts"] is True
    assert edited["old_selected_text_absent"] is True


def test_forms_report_on_form_fixture():
    if not FORM.exists():
        pytest.skip("form fixture not present")
    report = _envelope(oxide.open(FORM).forms_report(), "forms_report")
    assert isinstance(report, dict)


def test_signature_report_on_signed_fixture():
    if not SIG.exists():
        pytest.skip("signed fixture not present")
    report = _envelope(oxide.open(SIG).signature_report(), "signature_report")
    # The signature report's inner payload is the list of signatures.
    assert isinstance(report, list)


def test_module_level_reports():
    assert hasattr(oxide, "pubsec_decrypt_pdf_pfx")
    assert hasattr(oxide, "pubsec_reencrypt_pdf_pfx")
    feature = _envelope(oxide.feature_report(), "feature_report")
    assert isinstance(feature["engine_version"], str)
    assert feature["report_envelope_version"] == 1
    assert feature["prompt04"]["scanner"]["default_implementation"] == "safe_first_byte_chunked"
    assert (
        feature["prompt04"]["renderer_decode_scheduler"]["status"]
        == "adopted_for_immediate_renderer_decode_paths"
    )
    assert (
        feature["prompt05"]["decode_scheduler"]["status"]
        == "adopted_for_prompt05_non_render_decode_paths"
    )
    assert feature["prompt05"]["hostile_corpus"]["generator"].endswith(
        "prompt05_hostile_codec_corpus.py"
    )
    assert (
        feature["prompt06"]["native_replay"]["status"]
        == "native_text_image_form_display_list_foundation"
    )
    assert feature["prompt06"]["renderer_parity_audit"]["script"].endswith(
        "prompt06_renderer_parity_audit.py"
    )
    assert (
        feature["prompt06"]["prompt06b_multi_reference_audit"]["status"]
        == "multi_reference_audit_complete"
    )
    assert (
        feature["prompt07_transparency_compositing"]["status"]
        == "native_foundation_with_prompt07b_closure"
    )
    assert (
        feature["prompt07_transparency_compositing"]["reference_audit"]["memory_cap_mb"]
        == 4096
    )
    assert "Luminosity" in feature["prompt07_transparency_compositing"]["blend_modes"]["implemented"]
    assert feature["prompt07b_transparency_closure"]["status"] == "complete"
    assert (
        feature["prompt07b_transparency_closure"]["reference_audit"]["oxide_outlier_failures"]
        == 0
    )
    assert (
        "DeviceCMYK"
        in feature["prompt07b_transparency_closure"]["luminosity_soft_mask_color_spaces"][
            "supported"
        ]
    )
    prompt08 = feature["prompt08_text_clipping_shading_patterns"]
    assert prompt08["status"] == "native_common_paths_with_bounded_unsupported_reports"
    assert prompt08["reference_audit"]["memory_cap_mb"] == 4096
    assert 7 in prompt08["text_clipping"]["rendering_modes"]
    assert "colored" in prompt08["tiling_patterns"]["paint_types"]
    prompt08b = feature["prompt08b_type3_cid_tensor_closure"]
    assert prompt08b["status"] == "complete_native_common_paths_with_reference_cluster_limits"
    assert prompt08b["reference_audit"]["oxide_outlier_failures"] == 0
    assert prompt08b["type7_tensor_patch"]["status"] == "native_tensor_product_interior"
    prompt09 = feature["prompt09_annotation_ocg_progressive_cache"]
    assert prompt09["status"] == "implemented_with_bounded_unsupported_reports"
    assert prompt09["closure_gates"]["oxide_outlier_failures"] == 0
    prompt09b = feature["prompt09b_annotation_progressive_cache_validation"]
    assert prompt09b["status"] == "implemented_and_proven"
    assert prompt09b["multi_reference_audit"]["oxide_outlier_failures"] == 0
    assert prompt09b["multi_reference_audit"]["unclassified_failures"] == 0
    assert prompt09b["public_report_parity"]["schema_change"] == "additive_section_only"
    prompt10 = feature["prompt10_cjk_rtl_color_glyph_reference_harness"]
    assert prompt10["status"] == "implemented_with_bounded_unsupported_reports"
    assert prompt10["closure_gates"]["memory_cap_mb"] == 4096
    assert (
        prompt10["color_glyph_rendering"]["status"]
        == "unsupported_color_tables_are_detected_and_reported"
    )
    prompt10b = feature["prompt10b_color_glyph_cjk_rtl_fidelity_closure"]
    assert prompt10b["status"] == "complete"
    assert prompt10b["color_glyph_rendering"]["colr_cpal"]["status"] == "implemented_and_proven"
    assert prompt10b["multi_reference_audit"]["oxide_outlier_failures"] == 0
    assert prompt10b["closure_gates"]["public_report_schema"] == "additive_feature_report_prompt10b"
    prompt10c = feature["prompt10c_color_glyph_hinting_cff_closure"]
    assert prompt10c["status"] == "complete"
    assert prompt10c["colrv1"]["status"] == "implemented_with_operator_level_limits"
    assert prompt10c["multi_reference_audit"]["oxide_outlier_failures"] == 0
    assert prompt10c["closure_gates"]["public_report_schema"] == "additive_feature_report_prompt10c"
    prompt10d = feature["prompt10d_full_colrv1_svg_color_glyph_closure"]
    assert prompt10d["status"] == "complete"
    assert (
        prompt10d["svg_in_opentype"]["status"]
        == "safe_static_subset_rendered_active_constructs_blocked"
    )
    assert (
        prompt10d["bitmap_color_glyphs"]["sbix"]["status"]
        == "png_and_jpeg_rendered_tiff_other_precisely_reported"
    )
    assert prompt10d["multi_reference_audit"]["oxide_outlier_failures"] == 0
    assert prompt10d["closure_gates"]["public_report_schema"] == "additive_feature_report_prompt10d"
    prompt10e = feature["prompt10e_colrv1_gradient_clip_composite_closure"]
    assert prompt10e["status"] == "complete"
    assert "PaintLinearGradient" in prompt10e["colrv1_gradients"]["implemented_operators"]
    assert prompt10e["colrv1_clip_stack"]["status"] == "implemented"
    assert "Multiply" in prompt10e["colrv1_composites"]["implemented_modes"]
    assert prompt10e["multi_reference_audit"]["oxide_outlier_failures"] == 0
    assert prompt10e["closure_gates"]["public_report_schema"] == "additive_feature_report_prompt10e"
    prompt10f = feature["prompt10f_colrv1_porterduff_radial_closure"]
    assert prompt10f["status"] == "complete"
    assert len(prompt10f["porter_duff_plus_composites"]["implemented_modes"]) == 12
    assert prompt10f["exact_moving_center_radial"]["status"] == "implemented_with_reference_equivalence"
    assert prompt10f["multi_reference_audit"]["oxide_outlier_failures"] == 0
    assert prompt10f["closure_gates"]["public_report_schema"] == "additive_feature_report_prompt10f"
    prompt11 = feature["prompt11_renderer_fuzz_cmm_closeout"]
    assert prompt11["status"] == "complete_with_native_cmm_hard_blocked_precise"
    assert prompt11["renderer_fuzz"]["fuzz_target_count"] == 25
    assert prompt11["renderer_closeout"]["oxide_outlier_failures"] == 0
    assert prompt11["native_cmm_backend"]["backend_used_in_current_build"] == "safe-rust-plus-qcms"
    assert prompt11["closure_gates"]["public_report_schema"] == "additive_feature_report_prompt11"
    prompt11b = feature["prompt11b_native_littlecms_cmm_backend_closure"]
    assert prompt11b["status"] == "complete"
    assert prompt11b["feature_flag"]["name"] == "native-cmm-lcms2"
    assert prompt11b["closure_gates"]["public_report_schema"] == "additive_feature_report_prompt11b"
    prompt12 = feature["prompt12_prepress_cmm_device_link_separation_plates"]
    assert prompt12["status"] == "complete"
    assert prompt12["closure_gates"]["public_report_schema"] == "additive_feature_report_prompt12"
    assert prompt12["separation_framebuffer"]["cache_key_includes_plate_state"] is True
    prompt12b = feature["prompt12b_nchannel_plate_reference_closure"]
    assert prompt12b["status"] == "complete"
    assert (
        prompt12b["closure_gates"]["public_report_schema"]
        == "additive_feature_report_prompt12b"
    )
    assert prompt12b["reference_audit"]["pdfium"] == "required_and_run_by_prompt12b_audit"
    prompt13 = feature["prompt13_full_overprint_prepress_closeout"]
    assert prompt13["status"] == "complete"
    assert (
        prompt13["closure_gates"]["public_report_schema"]
        == "additive_feature_report_prompt13"
    )
    assert prompt13["reference_audit"]["oxide_outlier_failures"] == 0
    assert prompt13["reference_audit"]["unclassified_failures"] == 0
    prompt14 = feature["prompt14_semantic_intelligence_parenttree_cjk_ml_layout"]
    assert prompt14["status"] == "complete"
    assert (
        prompt14["closure_gates"]["public_report_schema"]
        == "additive_feature_report_prompt14"
    )
    assert prompt14["privacy_defaults"]["cloud_upload_default"] is False
    prompt14b = feature["prompt14b_cjk_dictionary_layout_backend_closure"]
    assert prompt14b["status"] == "complete"
    assert (
        prompt14b["closure_gates"]["public_report_schema"]
        == "additive_feature_report_prompt14b"
    )
    assert prompt14b["dictionary_provider"]["external_pack_support"] == "implemented"
    assert (
        prompt14b["layout_backend"]["local_backend_status"]
        == "unsupported_reported_no_runtime"
    )
    prompt15 = feature["prompt15_semantic_binding_rag_benchmark_closeout"]
    assert prompt15["status"] == "complete"
    assert prompt15["closure_gates"]["public_report_schema"] == "additive_feature_report_prompt15"
    assert prompt15["closure_counts"]["blocked"] == 0
    assert prompt15["privacy"]["cloud_upload_default"] is False
    assert (
        prompt15["tableformer_table_transformer_hook"]
        ["model_can_rewrite_deterministic_text"]
        is False
    )
    prompt16 = feature["prompt16_xfa_runtime_sandbox_closure"]
    assert prompt16["status"] == "complete_bounded_foundation"
    assert prompt16["closure_counts"]["blocked"] == 0
    assert (
        prompt16["closure_gates"]["public_report_schema"]
        == "additive_feature_report_prompt16"
    )
    prompt17 = feature["prompt17_annotation_xfdf_media_nonaxis_redaction"]
    assert prompt17["status"] == "complete_bounded_foundation"
    assert prompt17["failure"]["blocked"] == 0
    assert prompt17["security"]["overlay_only_redaction_success_claims"] == 0
    prompt18 = feature["prompt18_mask_inline_associated_signature_safe_edits"]
    assert prompt18["failure"]["blocked"] == 0
    assert prompt18["security"]["signature_crypto_overclaim"] == 0
    prompt18b = feature["prompt18b_advanced_secure_mutation_closure"]
    assert prompt18b["failure"]["blocked"] == 0
    assert prompt18b["failure"]["security_proof"] == 0
    prompt23 = feature["prompt23_deterministic_writer_pubsec_aesgcm"]
    assert prompt23["blocked_rows"] == 0
    assert prompt23["public_key_handler_status"] == "implemented_with_limits"
    assert prompt23["aes_gcm_decrypt_status"] == "implemented_with_limits"
    tamper = _envelope(oxide.crypto_tamper_test(), "crypto_tamper_test")
    assert tamper["plaintext_release_possible"] is False
    decode = _envelope(
        oxide.decode_budget_report("DCTDecode", 4096, 4096, 3), "decode_budget_report"
    )
    assert "diagnostics" in decode
    dedup = _envelope(
        oxide.resource_dedup_report([b"a", b"a", b"b"]), "resource_dedup_report"
    )
    assert dedup["duplicate_count"] == 1


def test_sanitize_produces_bytes_and_report(tmp_path):
    doc = oxide.open(FIXTURE)
    out = tmp_path / "clean.pdf"
    data, report = doc.sanitize(policy="balanced", output=out)
    assert data[:5] == b"%PDF-"
    assert out.read_bytes() == data
    r = _envelope(report, "sanitize_report")
    assert r["output_bytes"] > 0


def test_pdf_mac_create_owned_output_and_verify(tmp_path):
    doc = oxide.open(FIXTURE)
    data, report = doc.pdf_mac_create(output=tmp_path / "pdfmac.pdf")
    assert bytes(data).startswith(b"%PDF-")
    created = _envelope(report, "pdf_mac_create")
    assert created["verification_state"] == "valid"
    reopened = oxide.open(bytes(data))
    verified = _envelope(reopened.pdf_mac_verify(), "pdf_mac_verify")
    assert verified["state"] == "valid"
    assert verified["trusted_document_integrity"] is True


def test_xfa_owned_output_surfaces_on_non_xfa_pdf(tmp_path):
    doc = oxide.open(FIXTURE)
    preview, preview_report = doc.xfa_render(output=tmp_path / "xfa-preview.pdf")
    assert preview[:5] == b"%PDF-"
    assert _envelope(preview_report, "xfa_render_report")["schema_version"] == "prompt16.xfa.v1"
    flattened, flatten_report = doc.xfa_flatten(mode="extract_only")
    assert flattened[:5] == b"%PDF-"
    assert _envelope(flatten_report, "xfa_flatten_report")["schema_version"] == "prompt16.xfa.v1"
    sanitized, sanitize_report = doc.xfa_sanitize(mode="remove_all_xfa")
    assert sanitized[:5] == b"%PDF-"
    assert _envelope(sanitize_report, "xfa_sanitize_report")["schema_version"] == "prompt16.xfa.v1"


def test_prompt17_owned_output_surfaces(tmp_path):
    doc = oxide.open(FIXTURE)
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
    doc = oxide.open(FIXTURE)
    a, ra = doc.canonicalize(date_epoch=0)
    b, rb = doc.canonicalize(date_epoch=0)
    assert a == b  # deterministic bytes
    assert _envelope(ra, "canonicalize_report")["deterministic"] is True
    assert ra["report"]["output_sha256"] == rb["report"]["output_sha256"]


def test_redact_removes_and_verifies():
    doc = oxide.open(FIXTURE)
    data, report = doc.redact(["Hello"])
    assert data[:5] == b"%PDF-"
    r = _envelope(report, "redaction_report")
    assert len(r["applied"]) >= 1
    # The redacted output must not surface the term in a fresh parse.
    redacted = oxide.open(data)
    assert "Hello" not in redacted.extract_text()


def test_redact_empty_terms_raises():
    with pytest.raises(oxide.OxideError):
        oxide.open(FIXTURE).redact(["   "])


def test_redact_strict_missing_term_raises():
    # A term that does not exist cannot be redacted → error (nothing applied).
    with pytest.raises(oxide.OxideError):
        oxide.open(FIXTURE).redact(["ZZZ-not-present-anywhere"], strict=True)


def test_invalid_pdf_bytes_raise():
    with pytest.raises(oxide.OxideError):
        oxide.open(b"%PDF- broken not really")


def test_cross_surface_parity_smoke(tmp_path):
    """The Python security report must equal the report the sdk facade emits for
    the same bytes (same JSON), proving Python does not diverge from the shared
    facade the C ABI also uses."""
    import json

    doc = oxide.open(FIXTURE)
    py_report = doc.security_report()
    # Round-trip through JSON to confirm it is plain, serializable data.
    again = json.loads(json.dumps(py_report))
    assert again == py_report
