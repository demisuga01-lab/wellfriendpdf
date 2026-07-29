from pathlib import Path
import json

import pytest

import wellfriendpdf


ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "crates" / "engine" / "tests" / "fixtures" / "multi_stream.pdf"
BROKEN = ROOT / "renderer-benchmark" / "corpus" / "hostile" / "hostile_000_random.pdf"


@pytest.fixture
def sample_pdf() -> Path:
    return FIXTURE


def test_open_path_text_and_render():
    doc = wellfriendpdf.open(FIXTURE)
    assert doc.page_count >= 1
    assert "Hello" in doc.page(1).text
    assert doc[0].number == 1
    assert isinstance(doc.page(1).render(), bytes)


def test_open_bytes_and_python_native_outputs():
    doc = wellfriendpdf.open(FIXTURE.read_bytes())
    assert isinstance(doc.metadata, dict)
    assert isinstance(doc.page(1).words, list)
    assert isinstance(doc.page(1).tables, list)
    assert isinstance(doc.extract_fields(), dict)
    assert isinstance(doc.document_model(), dict)
    assert isinstance(doc.to_markdown(), str)
    assert isinstance(doc.to_html(), str)


def test_region_profile_and_markdown_controls():
    doc = wellfriendpdf.open(FIXTURE)
    page = doc.page(1)
    region = page.region(0, 0, 10000, 10000)

    assert "Hello" in region.text
    assert isinstance(region.words, list)
    assert isinstance(region.tables, list)
    assert isinstance(region.images, list)
    bbox = page.within(0, 0, 10000, 10000).bbox
    assert bbox[0] == 0.0
    assert bbox[1] == 0.0
    assert bbox[2] <= 10000.0
    assert bbox[3] <= 10000.0
    assert "Hello" in doc.extract_text(profile="layout-faithful")
    assert isinstance(doc.document_model(profile="rag-chunks"), dict)
    assert isinstance(doc.to_markdown(detect_headings=False, profile="fast-text"), str)


def test_iteration_yields_pages():
    doc = wellfriendpdf.open(FIXTURE)
    assert [page.number for page in doc] == list(range(1, doc.page_count + 1))


def test_malformed_pdf_raises_wellfriendpdf_error():
    if not BROKEN.exists():
        pytest.skip("robustness fixture not present")
    with pytest.raises(wellfriendpdf.WellfriendError):
        wellfriendpdf.open(BROKEN)


def test_editing_transactions_surfaces(sample_pdf):
    data = open(sample_pdf, "rb").read()
    factories = [
        lambda: wellfriendpdf.PyDocument(data),
        lambda: wellfriendpdf.PyDocument(str(sample_pdf)),
        lambda: wellfriendpdf.Document(data),
        lambda: wellfriendpdf.Document(str(sample_pdf)),
        lambda: wellfriendpdf.open_document(str(sample_pdf)),
        lambda: wellfriendpdf.open(str(sample_pdf)),
        lambda: wellfriendpdf.load(str(sample_pdf)),
    ]
    doc = None
    required_doc_attrs = {"editing_transactions_report", "editing_transactions_report_json"}
    for factory in factories:
        try:
            candidate = factory()
            if any(hasattr(candidate, name) for name in required_doc_attrs):
                doc = candidate
                break
        except Exception:
            continue
    assert doc is not None

    def call_json(names, *args):
        for name in names:
            method = getattr(doc, name, None)
            if method is not None:
                return method(*args)
        raise AttributeError(names[0])

    def parse_json(value):
        if isinstance(value, dict):
            return value
        return json.loads(value)

    request = '{"requested_mode":"operator_preserving","page":1,"source_text":"Hello","replacement_text":"World"}'
    assert isinstance(parse_json(call_json(["editing_transactions_report", "editing_transactions_report_json"])), dict)
    assert isinstance(parse_json(call_json(["editing_transactions_scene_report", "editing_transactions_scene_report_json"])), dict)
    assert isinstance(parse_json(call_json(["editing_transactions_scene_select", "editing_transactions_scene_select_json"], '{"page":1,"point":[20,100]}')), dict)
    assert isinstance(parse_json(call_json(["editing_transactions_transaction_plan", "editing_transactions_transaction_plan_json"], request)), dict)
    transaction_apply = call_json(["editing_transactions_transaction_apply", "editing_transactions_transaction_apply_json"], request)
    if isinstance(transaction_apply, tuple):
        # Output-producing bindings return `(pdf_bytes, report)` rather than a
        # report-only JSON value. The smoke contract validates the shared
        # report while retaining ownership coverage for the produced bytes.
        assert isinstance(transaction_apply[0], bytes)
        transaction_apply = transaction_apply[1]
    assert isinstance(parse_json(transaction_apply), dict)
    assert isinstance(parse_json(call_json(["editing_transactions_text_map", "editing_transactions_text_map_json"], "A\u0301B", "ltr")), dict)
    assert isinstance(parse_json(call_json(["editing_transactions_shape_text", "editing_transactions_shape_text_json"], "A\u0301B", "ltr")), dict)
    assert isinstance(parse_json(call_json(["editing_transactions_font_subset_plan", "editing_transactions_font_subset_plan_json"], "A\u0301B", "ltr", "preserve_existing_font")), dict)
    assert isinstance(parse_json(call_json(["editing_transactions_font_substitution_report", "editing_transactions_font_substitution_report_json"], "Helvetica", "A\u0301B", "no_silent_substitution")), dict)


def test_text_reflow_query_surfaces_share_preview_request(sample_pdf):
    doc = wellfriendpdf.open(sample_pdf)
    request = json.dumps({
        "requested_mode": "geometric_block",
        "page": 1,
        "source_text": "Hello",
        "replacement_text": "World",
        "region": [10.0, 10.0, 260.0, 90.0],
        "language": "en",
        "hyphenation": True,
        "layout_constraints": [{
            "constraint_id": "python_soft_height",
            "variable": "region_height",
            "relation": "ge",
            "value": 500.0,
            "priority": "weak",
        }],
    })

    def call_json(name):
        method = getattr(doc, name)
        value = method(request)
        return value if isinstance(value, dict) else json.loads(value)

    assert call_json("text_reflow_overflow_report")["kind"] == "text_reflow_overflow_report"
    constraints = call_json("text_reflow_constraints_report")
    assert constraints["kind"] == "text_reflow_constraints_report"
    assert "python_soft_height" in json.dumps(constraints)
    assert call_json("text_reflow_confidence_report")["kind"] == "text_reflow_confidence_report"
    output, apply = doc.text_reflow_reflow_region(request)
    assert isinstance(output, bytes)
    assert apply["kind"] == "text_reflow_reflow_region"
    validation = doc.text_reflow_validate_reflow_output(output, request)
    assert validation["kind"] == "text_reflow_validate_reflow_output"
    assert validation["report"]["valid"] is True
    restored, undo = doc.text_reflow_undo_reflow(output, request)
    assert restored == sample_pdf.read_bytes()
    assert undo["kind"] == "text_reflow_undo_reflow"
    assert undo["report"]["byte_exact_restoration"] is True
