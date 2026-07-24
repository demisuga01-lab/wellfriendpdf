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
