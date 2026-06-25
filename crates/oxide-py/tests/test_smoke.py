from pathlib import Path

import pytest

import oxide


ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "crates" / "engine" / "tests" / "fixtures" / "multi_stream.pdf"
BROKEN = ROOT / "renderer-benchmark" / "corpus" / "hostile" / "hostile_000_random.pdf"


def test_open_path_text_and_render():
    doc = oxide.open(FIXTURE)
    assert doc.page_count >= 1
    assert "Hello" in doc.page(1).text
    assert doc[0].number == 1
    assert isinstance(doc.page(1).render(), bytes)


def test_open_bytes_and_python_native_outputs():
    doc = oxide.open(FIXTURE.read_bytes())
    assert isinstance(doc.metadata, dict)
    assert isinstance(doc.page(1).words, list)
    assert isinstance(doc.page(1).tables, list)
    assert isinstance(doc.extract_fields(), dict)
    assert isinstance(doc.document_model(), dict)
    assert isinstance(doc.to_markdown(), str)
    assert isinstance(doc.to_html(), str)


def test_iteration_yields_pages():
    doc = oxide.open(FIXTURE)
    assert [page.number for page in doc] == list(range(1, doc.page_count + 1))


def test_malformed_pdf_raises_oxide_error():
    if not BROKEN.exists():
        pytest.skip("robustness fixture not present")
    with pytest.raises(oxide.OxideError):
        oxide.open(BROKEN)
