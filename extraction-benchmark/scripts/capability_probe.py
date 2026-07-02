"""Capability probe: do the 200 slice PDFs actually contain AcroForm form fields?

If they do not, then AcroForm-reading competitors (pypdf get_fields, pymupdf
widgets, pdf_oxide form-fill) recover nothing by corpus construction -- the
ground-truth 'fields' are rendered key-value TEXT, not interactive widgets.
That is a capability/source mismatch, not a quality loss, and this probe makes
the claim evidence-based rather than asserted. Reads only the 200 slice files.
"""
from __future__ import annotations
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import competitive_benchmark as cb  # noqa: E402

entries = cb.load_entries(cb.REPO / "test_corpus", 200, "has-fields")
print(f"slice_files={len(entries)} (must be <=200)")

pypdf_with_acroform = 0
pypdf_total_fields = 0
pymupdf_with_acroform = 0
pymupdf_total_widgets = 0
truth_field_pairs = 0

from pypdf import PdfReader  # noqa: E402
import fitz  # PyMuPDF  # noqa: E402

for e in entries:
    truth_field_pairs += len(e["label"].get("fields") or {})
    # pypdf AcroForm
    try:
        r = PdfReader(str(e["pdf"]))
        f = r.get_fields() or {}
        if f:
            pypdf_with_acroform += 1
            pypdf_total_fields += len(f)
    except Exception:
        pass
    # pymupdf AcroForm widgets
    try:
        d = fitz.open(str(e["pdf"]))
        n = 0
        for pg in d:
            n += len(list(pg.widgets() or []))
        d.close()
        if n:
            pymupdf_with_acroform += 1
            pymupdf_total_widgets += n
    except Exception:
        pass

print(f"truth_field_pairs_in_slice={truth_field_pairs}")
print(f"pypdf:   files_with_AcroForm={pypdf_with_acroform}/200  total_acroform_fields={pypdf_total_fields}")
print(f"pymupdf: files_with_AcroForm={pymupdf_with_acroform}/200  total_widgets={pymupdf_total_widgets}")
