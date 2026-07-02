# Phase 3 Summary

Phase 3 audited the existing document/structural operation surface and added the
small utility conversions and page-editing workflows that were still missing.

## Audit

The cross-surface baseline is recorded in `docs/phase3_api_audit.md`. The audit
found that Rust and CLI already covered most existing operations, while Python
and the C ABI primarily exposed parsing/extraction/rendering. Phase 3 added
thin binding wrappers instead of reimplementing engine logic per surface.

## Added Utilities

- `pdf-to-jpg` / `pdf-to-image`: renders pages through the existing renderer and
  writes one JPG or PNG per page.
- `image-to-pdf`: wraps JPG/PNG inputs into a PDF using the existing authoring
  writer.
- `watermark`: appends text or image overlay content streams. It does not
  rasterize and replace pages.
- `add-page-numbers`: uses the same overlay mechanism as watermarking with
  per-page format substitution.
- `organize`: copies pages in caller-supplied order; repeated indices duplicate
  pages and omitted indices delete pages. CLI also supports inserting pages
  from a second PDF.
- `decrypt`: writes a normalized unencrypted copy of a password-opened document.

## Surface Coverage

- Rust: new helpers are exported from `oxide_engine::utilities` and the crate
  root.
- CLI: commands added for all Phase 3 utilities, with JSON summaries for
  scriptable workflows.
- Python: module-level helpers mirror the new utilities and the most useful
  structural operations.
- C ABI: byte-buffer functions were added for page raster JPEG, page
  extraction/organization, rotate, optimize, linearize, decrypt/encrypt,
  HTML/fonts/signature JSON, watermark text, page numbers, image-to-PDF, and
  merge.
- WASM: unchanged by design; browser parsing/rendering remains digital-born and
  no filesystem utility surface is added.

## Memory Discipline

The new Rust utility layer renders or decodes one page/image at a time at the
operation boundary. Output PDFs necessarily retain encoded image objects in the
authoring builder until serialization, while input image decoding is bounded by
the engine decode-pixel cap.

## Extraction Accuracy

Phase 3 does not alter the text/table/field extraction pipeline. The required
slice targets remain:

- field-F1: `0.72503`
- table shape-F1: `0.96232`
- char-sim: `0.92743`
- word-F1: `1.0`

The validation run for this phase should re-run those slices after workspace
tests and clippy.
