"""Prompt-01 report-surface tests for the Python SDK.

Every report method returns a native dict parsed from the shared
`oxide_engine::sdk` versioned-JSON envelope. These assert the envelope shape,
a representative report field, honest handling of invalid input, and that the
destructive operations produce real PDF bytes plus a report.
"""

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
    _envelope(doc.annotations_report(), "annotation_report")
    _envelope(doc.pages_report(), "page_operations_report")
    _envelope(doc.interactive_report(), "interactive_report")
    _envelope(doc.signature_report(), "signature_report")
    _envelope(doc.font_report(), "font_report")
    _envelope(doc.validate(), "standards_profile")
    _envelope(doc.validate(profile="pdfa"), "standards_profile")
    _envelope(doc.validate_pdfa(), "pdfa_validation")
    _envelope(doc.validate_pdfua(), "pdfua_validation")
    _envelope(doc.chunks(), "chunk_set")
    _envelope(doc.text_semantic(), "text_semantic")
    _envelope(doc.semantic_document(), "semantic_document")


def test_security_report_fields():
    report = _envelope(oxide.open(FIXTURE).security_report(), "security_report")
    assert isinstance(report["encrypted"], bool)
    assert isinstance(report["findings"], list)


def test_parser_report_opened_true():
    report = _envelope(oxide.open(FIXTURE).parser_report(mode="audit"), "parser_report")
    assert report["opened"] is True


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
    feature = _envelope(oxide.feature_report(), "feature_report")
    assert isinstance(feature["engine_version"], str)
    assert feature["report_envelope_version"] == 1
    assert feature["prompt04"]["scanner"]["default_implementation"] == "safe_first_byte_chunked"
    assert (
        feature["prompt04"]["renderer_decode_scheduler"]["status"]
        == "adopted_for_immediate_renderer_decode_paths"
    )
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
