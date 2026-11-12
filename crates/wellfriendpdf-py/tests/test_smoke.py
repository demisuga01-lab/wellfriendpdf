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


def test_extract_text_reading_order():
    doc = wellfriendpdf.open(FIXTURE)
    pages = [doc.extract_text_reading_order(page=n) for n in range(1, doc.page_count + 1)]
    assert "Hello" in pages[0]
    # The whole document is each page followed by a newline, as `extract-text` prints it.
    assert doc.extract_text_reading_order() == "".join(text + "\n" for text in pages)
    with pytest.raises(wellfriendpdf.WellfriendError):
        doc.extract_text_reading_order(page=doc.page_count + 1)


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

    request = '{"requested_mode":"operator_preserving","page":1,"source_text":"Hello","replacement_text":"Hallo"}'
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


IMAGE_FIXTURE = ROOT / "crates" / "engine" / "tests" / "fixtures" / "image_only.pdf"


def test_render_contract_raster_policy_forced_color_and_form_highlight_builders():
    doc = wellfriendpdf.open(IMAGE_FIXTURE)
    contract = doc.default_render_contract(1, 72)
    built = (
        contract.with_thin_line_mode("shape")
        .with_vector_antialias_gamma("gamma15")
        .with_rect_snapping("pdfium")
        .with_forced_colors((0, 0, 0), path_stroke=(10, 20, 30, 255), convert_fill_to_stroke=True)
        .with_form_highlight((0, 0, 255, 80), check_box=(255, 0, 0, 40), focus_field="form.name")
    )
    value = json.loads(built.to_json())
    assert value["thin_line_mode"] == "Shape"
    assert value["vector_antialias_gamma"] == "Gamma15"
    assert value["rect_snapping"] == "Pdfium"
    unadjusted = json.loads(contract.with_stroke_adjustment("off").to_json())
    assert unadjusted["stroke_adjustment"] == "Off"
    profiled = json.loads(contract.with_profile("pdfium-test").to_json())
    assert profiled["profile"] == "PdfiumTest"
    assert profiled["rect_snapping"] == "Pdfium"
    assert profiled["stroke_adjustment"] == "Off"
    assert profiled["form_highlight"]["text"] == {"r": 221, "g": 228, "b": 255, "a": 100}
    pdfium = json.loads(contract.with_profile("pdfium").to_json())
    assert pdfium["profile"] == "Pdfium"
    assert pdfium["text_gamma"] == "Pdfium"
    assert "form_highlight" not in pdfium
    default = json.loads(contract.with_profile("pdfium").with_profile("default").to_json())
    assert "profile" not in default and default.get("stroke_adjustment", "Default") == "Default"
    with pytest.raises(ValueError, match="render profile"):
        contract.with_profile("bogus")
    with pytest.raises(ValueError):
        contract.with_stroke_adjustment("bogus")
    assert doc.render_contract_png(contract.with_profile("pdfium-test")).startswith(b"\x89PNG")
    assert doc.render_contract_png(contract.with_stroke_adjustment("off")).startswith(b"\x89PNG")
    assert value["forced_colors"]["path_fill"] == {"r": 0, "g": 0, "b": 0, "a": 255}
    assert value["forced_colors"]["path_stroke"]["g"] == 20
    assert value["forced_colors"]["text_fill"] == value["forced_colors"]["path_fill"]
    assert value["forced_colors"]["convert_fill_to_stroke"] is True
    assert value["form_highlight"]["text"]["a"] == 80
    assert value["form_highlight"]["check_box"]["r"] == 255
    assert value["form_highlight"]["focus_field"] == "form.name"
    assert doc.render_contract_png(built).startswith(b"\x89PNG")

    cleared = json.loads(built.without_forced_colors().without_form_highlight().to_json())
    assert "forced_colors" not in cleared and "form_highlight" not in cleared
    from_json = json.loads(contract.with_form_highlight_json('{"signature":{"r":1,"g":2,"b":3,"a":4}}').to_json())
    assert from_json["form_highlight"]["signature"]["a"] == 4
    with pytest.raises(ValueError):
        contract.with_thin_line_mode("blurry")
    with pytest.raises(ValueError):
        contract.with_forced_colors((1, 2))


def test_page_media_thumbnail_image_objects_and_original_images():
    doc = wellfriendpdf.open(IMAGE_FIXTURE)
    assert doc.page_thumbnail_rgba(1) is None
    assert doc.decode_page_thumbnail(1) is None
    assert doc.page_thumbnail_png(1) is None

    images = doc.page_images_info(1)
    assert images and images[0]["index"] == 0
    original = doc.extract_image_original(1, images[0]["index"])
    assert isinstance(original["data"], bytes) and original["data"]
    assert original["extension"]
    assert isinstance(original["sidecar_data"], dict)
    with pytest.raises(wellfriendpdf.WellfriendError):
        doc.extract_image_original(1, 10_000)

    width, height, rgba = doc.render_image_object_rgba(1, index=0)
    assert len(rgba) == width * height * 4
    assert doc.render_image_object(1).startswith(b"\x89PNG")
    with pytest.raises(wellfriendpdf.WellfriendError):
        doc.render_image_object(1, name="NoSuchImage")


def test_search_text_options():
    doc = wellfriendpdf.open(FIXTURE)
    report = doc.search_text("Hello")
    assert report["schema_version"] == "binding_media.text_search.v1"
    assert report["match_count"] >= 1
    assert report["matches"][0]["quads"]
    insensitive = doc.search_text("hello", pages=[1], case_sensitive=False, whole_word=True)
    assert insensitive["options"]["case_sensitive"] is False
    assert insensitive["options"]["whole_word"] is True
    assert insensitive["match_count"] >= 1
    assert doc.search_text("hello")["match_count"] == 0
    with pytest.raises(wellfriendpdf.WellfriendError):
        doc.search_text("Hello", pages=[999])
