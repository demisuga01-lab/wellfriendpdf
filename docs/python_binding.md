# Python Binding

Wellfriend now has a PyO3/maturin binding in `crates/wellfriendpdf-py` that installs a Python
module named `wellfriendpdf`.

```python
import wellfriendpdf

doc = wellfriendpdf.open("invoice.pdf")
print(doc.page_count)
print(doc.extract_text())
print(doc.page(1).tables)
print(doc.extract_fields(doc_type="auto"))
print(doc.page(1).region(0, 0, 306, 792).text)
print(doc.to_markdown(detect_headings=True, profile="rag-chunks"))
png = doc.page(1).render(dpi=150)

wellfriendpdf.pdf_to_images("input.pdf", "pages", pages="1-3", format="jpg", dpi=150)
combined = wellfriendpdf.images_to_pdf(["scan1.jpg", "scan2.png"], output="combined.pdf")
xlsx = wellfriendpdf.pdf_to_xlsx("tables.pdf", output="tables.xlsx", layout="pages")
pptx = wellfriendpdf.pdf_to_pptx("report.pdf", output="slides.pptx")
docx = wellfriendpdf.pdf_to_docx("report.pdf", output="report.docx")
pdf = wellfriendpdf.docx_to_pdf("report.docx", output="from_docx.pdf")
numbered = wellfriendpdf.add_page_numbers("input.pdf", output="numbered.pdf")
```

The API is intentionally Python-native: text is `str`, rendered pages and images
are `bytes`, and tables/fields/document models are returned as lists and dicts.
Malformed-input errors become `wellfriendpdf.WellfriendError`; Rust panics are caught at the
FFI boundary and converted to the same exception type.

## Build

```powershell
python -m pip install maturin
cd crates\wellfriendpdf-py
python -m maturin build
python -m pip install target\wheels\wellfriendpdf-0.1.0-*.whl
python -c "import wellfriendpdf; print(wellfriendpdf.__version__)"
```

Verified locally on Windows with Python 3.14.3. Prebuilt cross-platform wheels
are not claimed yet; CI wheel publishing is future packaging work.

## Current Coverage

- Open from path or bytes, optionally with a password.
- Document/page text extraction, with named profiles:
  `fast-text`, `layout-faithful`, `tables-focused`, and `rag-chunks`.
- Lazy page properties: `page.text`, `page.words`, `page.tables`, `page.images`.
- Document-level tables, fields, document model, markdown, HTML, and rendering.
- Region/scoped extraction through `page.region(x0, y0, x1, y1)` and
  `page.within(...)`. The returned scoped object exposes `.text`, `.words`,
  `.tables`, and `.images`.
- Explicit markdown heading control through
  `to_markdown(detect_headings=True|False)` and `page.markdown(...)`.
- OCR injection for scanned pages through `document_model(..., ocr=backend)`,
  `to_markdown(..., ocr=backend)`, and `to_html(..., ocr=backend)`, where
  `backend` is any Python object with
  `recognize(image_bytes, info) -> list[dict]`. See
  `crates/wellfriendpdf-py/examples/local_ai_ocr_backend.py` and
  `docs/ocr_backends.md`.
- Structural and utility helpers:
  `merge_pdfs`, `extract_pages`, `rotate_pdf`, `encrypt_pdf`, `decrypt_pdf`,
  `optimize_pdf`, `repair_pdf`, `linearize_pdf`, `pdf_to_images`,
  `images_to_pdf`, `pdf_to_xlsx`, `pdf_to_pptx`, `pdf_to_docx`,
  `docx_to_pdf`, `xlsx_to_pdf`, `pptx_to_pdf`, `watermark_pdf`,
  `add_page_numbers`, `organize_pdf`, `fonts`, and `verify_signatures`.

`pdf_to_xlsx` accepts `layout="pages"` or `layout="tables"`. `pdf_to_pptx`
maps each PDF page to one editable slide and can disable image export with
`include_images=False`. `pdf_to_docx` reconstructs a flowing editable document;
Office-to-PDF helpers use Wellfriend's native writer and do not require LibreOffice.

Region coordinates are PDF user-space points with origin at the page's
bottom-left. Scoped extraction includes an item when its center is inside the
region or at least half of its bounding box overlaps it. Markdown heading
detection is heuristic unless the source PDF supplies tagged structure.
