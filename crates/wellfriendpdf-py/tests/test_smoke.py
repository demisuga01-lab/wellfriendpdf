from pathlib import Path

import pytest

import wellfriendpdf


ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "crates" / "engine" / "tests" / "fixtures" / "multi_stream.pdf"
BROKEN = ROOT / "renderer-benchmark" / "corpus" / "hostile" / "hostile_000_random.pdf"


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


def test_prompt32_surfaces(sample_pdf):
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
    required_doc_attrs = {"prompt32_report", "prompt32_report_json"}
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

    request = '{"requested_mode":"operator_preserving","page_index":0,"selection":{"text":"Hello"},"replacement":"Hi"}'
    assert isinstance(parse_json(call_json(["prompt32_report", "prompt32_report_json"])), dict)
    assert isinstance(parse_json(call_json(["prompt32_scene_report", "prompt32_scene_report_json"])), dict)
    assert isinstance(parse_json(call_json(["prompt32_scene_select", "prompt32_scene_select_json"], '{"page_index":0,"point":[20,100]}')), dict)
    assert isinstance(parse_json(call_json(["prompt32_transaction_plan", "prompt32_transaction_plan_json"], request)), dict)
    assert isinstance(parse_json(call_json(["prompt32_transaction_apply", "prompt32_transaction_apply_json"], request)), dict)
    assert isinstance(parse_json(call_json(["prompt32_text_map", "prompt32_text_map_json"], "A\u0301B", "ltr")), dict)
    assert isinstance(parse_json(call_json(["prompt32_shape_text", "prompt32_shape_text_json"], "A\u0301B", "ltr")), dict)
    assert isinstance(parse_json(call_json(["prompt32_font_subset_plan", "prompt32_font_subset_plan_json"], "A\u0301B", "ltr", "preserve_existing_font")), dict)
    assert isinstance(parse_json(call_json(["prompt32_font_substitution_report", "prompt32_font_substitution_report_json"], "Helvetica", "A\u0301B", "no_silent_substitution")), dict)