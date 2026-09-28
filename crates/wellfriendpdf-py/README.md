# wellfriendpdf-py

Python bindings for the Wellfriend PDF SDK engine, built with PyO3 and maturin.

```python
import wellfriendpdf

doc = wellfriendpdf.open("input.pdf")
print(doc.page_count)
print(doc.extract_text())

page = doc.page(1)
print(page.text)
print(page.words[:5])

left_half = page.region(0, 0, 306, 792)
print(left_half.text)

for table in page.tables:
    print(table["rows"])

fields = doc.extract_fields(doc_type="auto")
markdown = doc.to_markdown(detect_headings=True, profile="rag-chunks")
png_bytes = page.render(dpi=150)

docx_bytes = wellfriendpdf.pdf_to_docx("input.pdf", output="input.docx")
pdf_bytes = wellfriendpdf.docx_to_pdf("input.docx", output="from_docx.pdf")
```

### ECBES universal transaction

`doc.ecbes_universal_edit(request_json, output=...)` materializes the bounded
canonical universal-edit candidates declared in the request from the same input
revision and returns `(selected_pdf_bytes, report)`. If no candidate qualifies,
the bytes are the exact original transport. Optional `output_user_password` and
`output_owner_password` select the apply-only secured-output entry point; secrets
are not serialized. See
[`docs/research/evidence_constrained_bidirectional_edit_synthesis.md`](../../docs/research/evidence_constrained_bidirectional_edit_synthesis.md).

### Retained editing sessions (source-only, not runtime-qualified)

`StoryEditSession` keeps a PDF revision and bounded checkpoint history in memory.
It uses the shared native story protocol, not a second Python PDF writer.

```python
import json

with wellfriendpdf.StoryEditSession(pdf_bytes) as editor:
    status = json.loads(editor.status_json())
    sources = json.loads(editor.command_json('{"op":"source_model","page":1}'))
    # request must bind this revision, exact source ranges and approved fonts.
    reviewed = json.loads(editor.preview_json(json.dumps(request)))
    # Obtain the user's approval of reviewed["preview"] before checkpointing.
    report = json.loads(editor.checkpoint_json(
        json.dumps(request), json.dumps(reviewed["receipt"])))
    output_pdf = editor.bytes()
    editor.undo()  # exact pre-checkpoint bytes; invalidates the old receipt
    editor.redo()
```

All discovery, table synchronization and conservative merge operations are
available through `command_json`; see [the protocol](../../docs/native_story_sessions.md).
Rendering is only performed by an explicit `render_page_png(page, dpi=96)` call,
bounded to 300 DPI / 16 million pixels. Input must be immutable Python `bytes`
(1..=256 MiB); JSON commands are at most 32 MiB. Output PDFs/PNGs are independent
`bytes`, JSON methods return strings, and `undo`/`redo` return booleans.

Commands, preview, checkpoint, undo/redo, construction and rendering accept the
optional existing `RenderCancellation` token. Native work detaches from the
Python interpreter so another thread can signal it. Session calls are protected
by a nonblocking mutex: concurrent calls, including close, fail as busy rather
than wait; retry only after the active call completes. Sequential cross-thread
use is allowed. `close()` is idempotent and a context manager closes on exit.
Unwinding panics poison the retained session; close and reopen known bytes.
No guarantee is made for aborting panics, OOM, native codec interruption or
free-threaded wheel compatibility. Cancellation is never checked after a
successfully published checkpoint in order to reclassify success as failure.

This convenience session does not supply encryption credentials or bypass
signature policies. It does not write paths or confer user authorization.
No Python imports, tests, native builds, rendering or PDF workloads were run
for this source addition. [Implementation report](../../docs/python_story_sessions.md).

### Document reports

Every report method returns a native dict (versioned-JSON envelope
`{"schema_version", "kind", "report"}`), backed by the shared
`wellfriendpdf_engine::sdk` facade the C ABI also uses:

```python
doc.security_report()          # encryption, signatures, risky content
doc.parser_report(mode="audit")# repair/xref/revisions/linearization/encryption
doc.color_report()             # ICC, output intents, spot/DeviceN, overprint
doc.forms_report()             # AcroForm fields, XFA status
doc.annotations_report()       # annotations, appearances, unsafe actions
doc.pages_report()             # boxes, labels, destinations
doc.interactive_report()       # forms + annotations + page ops
doc.signature_report()         # validity, trust, coverage, LTV
doc.font_report()              # fonts, embedding, subsetting
doc.validate_pdfa(); doc.validate_pdfua(); doc.validate(profile="all")
doc.text_semantic(); doc.chunks(); doc.semantic_document()
doc.semantic_bundle()          # full Semantic Closeout semantic report
doc.advanced_chunks()          # provenance/table/CJK/security-aware chunks
doc.semantic_search("invoice") # semantic + dictionary-token provenance
doc.image_decode_capability_report()
doc.progressive_image_decode_lifecycle_report('{"image_index":0}')
doc.table_proposal_status()    # hook/runtime/privacy status, no model load
sources = doc.authored_typed_table_sources()
data, rep = doc.mutate_authored_typed_table(request_json, font_bytes=None)
# request_json may set prune_empty_continuations=true; retained pages include reasons

# Output-producing (return (bytes, report)):
data, rep = doc.sanitize(policy="balanced", output="clean.pdf")
data, rep = doc.canonicalize(date_epoch=0)          # deterministic
data, rep = doc.redact(["SECRET"], strict=True)     # verifies absence
data, rep = doc.editing_transactions_transaction_apply_with_render_invalidation(
    request_json,
    render_invalidation_options_json='{"page_number":1,"dpi":72,"tile_width":256,"tile_height":256}',
)

cache = doc.render_cache()
png = doc.render_contract_png_with_render_cache(contract_json, cache)
png, cache_report = doc.render_contract_png_with_render_cache_report(contract_json, cache)
invalidation_report = cache.apply_render_invalidation_plan_json(render_invalidation_json)

# No-document queries:
wellfriendpdf.feature_report()                               # version + capabilities
wellfriendpdf.decode_budget_report("DCTDecode", 4096, 4096, 3)
wellfriendpdf.codec_isolation_report("FlateDecode", b"...", policy="in_process")
wellfriendpdf.resource_dedup_report([b"a", b"a", b"b"])
```

See `../../docs/python_sdk_binding_surface.md` and `examples/sdk_reports.py`.

Scanned-page OCR can be supplied by any Python object implementing
`recognize(image_bytes, info) -> list[dict]`:

```python
class MyOcr:
    def recognize(self, image_bytes, info):
        return [
            {"text": "Hello", "bbox": [72, 60, 140, 88],
             "confidence": 0.98, "line_id": 0},
        ]

markdown = doc.to_markdown(ocr=MyOcr(), ocr_lang="eng", ocr_dpi=300)
```

See `examples/local_ai_ocr_backend.py` for a runnable local-AI template with a
real `pytesseract` fallback when those Python packages are installed.

## Build And Install

```powershell
python -m pip install maturin
cd crates\wellfriendpdf-py
python -m maturin build
python -m pip install target\wheels\wellfriendpdf-0.1.0-*.whl
python -c "import wellfriendpdf; print(wellfriendpdf.__version__)"
```

`wellfriendpdf.open()` accepts a filesystem path or raw `bytes`. Password-protected PDFs
can be opened with `password="..."`.

## Exposed In This Binding

- `wellfriendpdf.open(path_or_bytes, password=None)`
- `Document.page_count`, `Document.metadata`, `Document.page(n)`,
  iteration over pages, and `doc[index]`
- `Document.extract_text`, `extract_tables`, `extract_fields`,
  `document_model`, `to_markdown`, `to_html`, and `render`
- `Page.text`, `Page.words`, `Page.tables`, `Page.images`,
- `Page.markdown`, `Page.render`, `Page.text_with_profile`, and
  `Page.region(...)` / `Page.within(...)`
- `RegionPage.text`, `RegionPage.words`, `RegionPage.tables`, and
  `RegionPage.images`
- Module helpers for structural ops and conversions including
  `pdf_to_xlsx`, `pdf_to_pptx`, `pdf_to_docx`, `docx_to_pdf`,
  `xlsx_to_pdf`, and `pptx_to_pdf`
- Editing transaction apply can return a render-invalidation plan through
  `editing_transactions_transaction_apply_with_render_invalidation(...)`,
  including mapped source IDs and optional dirty render tiles for caller caches.
- Image decode APIs expose per-image region/reduction/progressive capability
  status and bounded progressive image-decode lifecycle JSON through
  `image_decode_capability_report()` and
  `progressive_image_decode_lifecycle_report(request_json)`, including
  `document_close` release reports for document-owned decoder state.
- Contract rendering exposes cooperative cancellation through
  `RenderCancellation`, including `cancel()` and `is_cancelled()`, for PNG,
  bytearray caller-owned surfaces, font-substitution report, and
  render-telemetry report methods. Existing compatibility methods remain
  non-cancellable; cancellable PNG and PNG report methods release the Python
  GIL while rendering so another Python thread can call `cancel()`. Cancellable
  bytearray caller-owned surface methods and mutable progressive job methods
  still run with the GIL held because they involve Python-owned buffers or
  unsendable job state.
- Progressive render jobs expose
  `revise_render_contract_json(contract_json)` for full live schema-v1 contract
  revision, plus `request_cancel`,
  `step_with_cancellation(max_tiles, predicate_or_bool)`, and
  `finish_png_with_cancellation(predicate_or_bool)` for Python-side
  cancellation ergonomics; `finish_png()` and the cancellation variant raise
  checked tile-assembly diagnostics when called before completion, plus
  `viewer_queue_json()`,
  `execute_viewer_queue_json(max_items)`,
  `execute_viewer_queue_json_with_cancellation(max_items, cancellation)`,
  `execute_adjacent_page_prefetch(prefetch_identity, max_tiles)`,
  `execute_adjacent_page_prefetch_with_cancellation(prefetch_identity,
  max_tiles, cancellation)`, and
  `viewer_callback_dispatch_json()` for source-visible viewer scheduling,
  owned current-page queue execution, cancellable queue/prefetch execution,
  bounded adjacent-page child-session prefetch execution, and callback dispatch
  reports.

Region coordinates are PDF user-space points with origin at the page's
bottom-left. Scoped extraction includes an item when its center is in the region
or at least half of its box overlaps the region.

Named profiles are `fast-text`, `layout-faithful`, `tables-focused`, and
`rag-chunks`. Profiles are convenience bundles over Rust engine options; they do
not reimplement parsing in Python.

Errors from the Rust engine are converted to `wellfriendpdf.WellfriendError`. The binding
catches Rust panics at the Python boundary and converts them to that exception
instead of aborting the interpreter.

## Deferred

Cross-platform prebuilt wheels are future CI work; the local platform wheel is
built with maturin.

### Revision-bound text paint partitions (source-only, unqualified)

`Document.propose_text_range_paint_partitions(request_json)` returns a parsed,
non-mutating proposal bound to the exact input and request. Review every
candidate region/final-line layout, serialize the proposal envelope and an
approval carrying the same `proposal_id`, then call
`apply_text_range_paint_partitions(..., font_bytes=None)`. Optional approved
font bytes drive shaping, measurement and PDF embedding when the retained
source program lacks coverage. The engine recomputes the canonical proposal
before mutation and rejects stale, altered, missing or reordered approvals.
When `font_bytes` is supplied, the approval must carry its lowercase SHA-256 as
`font_sha256`.
Interactive hosts should use `preview_text_range_paint_partitions`, display its
PNG evidence, and pass `report["publication_receipt"]` to
`apply_reviewed_text_range_paint_partitions`; mismatched output is withheld.
Trusted native hosts may call
`wellfriendpdf.authenticate_text_range_paint_partition_receipt` and
`wellfriendpdf.verify_authenticated_text_range_paint_partition_receipt` with a
32..=256-byte HMAC-SHA-256 key. Do not expose a server-held key to untrusted
Python plugin/client code; remote clients should use the authenticated HTTP
preview/apply endpoints.
This binding path has not been compiled or executed in the current source-only
phase.

Every SDK-backed Python document route whose native facade accepts an input
password now forwards the retained zeroizing credential, including reflow,
standards, XFA, signatures, semantics, redaction, associated files and
sanitation. It is not serialized into reports or reused for output encryption.
