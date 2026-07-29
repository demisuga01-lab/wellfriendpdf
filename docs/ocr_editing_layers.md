# OCR Editing Layers

OCR analysis keeps three linked layers explicit: original scan, searchable text,
and approved editable reconstruction. Page classification, existing word boxes,
and OCR-layer state come from the canonical classifier, collector, and
`ocr::OcrEngine`/preprocess interfaces. Existing searchable text can be
corrected only with explicit approval and source-linked Prompt 33 rewriting;
the source scan is retained.

Creating recognition output for an image-only scan requires an injected
canonical OCR provider. In its absence the engine returns `provider_unavailable`
without creating duplicate text or rasterizing the page. Low-confidence or
unapproved reconstruction returns `reconstruction_review_required` unchanged.

For a provider result that has already been reviewed, `ocr_add_searchable_text`
creates an actual invisible PDF text instruction (`Tr 3`) through the canonical
editor on an image-only page. The visible scan remains untouched and the action
records provider identity/version, confidence, source document/page geometry,
and the reversible preimage. This bounded route accepts exact ASCII only until
a canonical `/ToUnicode`-capable OCR font resource is selected; non-ASCII text
returns `unsupported_script` instead of being lossy encoded.

`ocr_add_searchable_words` accepts an atomic reviewed provider word batch for a
single image-only page. Every word records its geometry, confidence, line link,
and text hash in the transaction report; all words are written together as
invisible text or none are written. Existing searchable layers still reject
the operation to prevent duplicate extraction text.

OCR keeps the original scan, searchable text, and editable reconstruction as
separate provenance-linked layers. Low-confidence correction and reconstruction
require approval; unsupported providers leave the original scan unchanged.
`ocr_add_searchable_text_with_link` creates reviewed invisible OCR text and a
canonical URI link annotation at the same source-image rectangle in one
transaction. The original scan remains visible, the OCR text remains
searchable, and undo restores both layers atomically.

## Searchable geometry correction

`ocr_correct_geometry` rewrites a provenance-resolved existing searchable text
instruction through the Prompt 33 source-reflow path while retaining the scan
as the visual source. It requires explicit review approval, validates the
target PDF-space rectangle, and supports exact transaction undo.
