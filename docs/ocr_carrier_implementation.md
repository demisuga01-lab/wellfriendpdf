# Native image/OCR groups and text-displacement corrections

Status: source implementation only, not a completed universal editor. No builds,
typechecks, tests, PDF workloads or rendering were run for this change. Current
working-tree changes remain uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`.

## Coordinated native operation

`ImageFragmentMove.ocr` optionally selects page-owned invisible glyph operands by
exact `span_ids` and concatenated font-decoded `expected_text`. These IDs come
from `analyze_multi_run_text_range` / WASM `imageOcrSourcesJson`. Visible duplicates
are not substitutes for the selected invisible occurrence. Request, preview and
approval are bound to the complete input revision.

The capture writer replays source text matrices, transforms, spacing, glyph codes
and resources in a searchable Form, suppressing unselected text and visual paint.
It does not recognize text or synthesize another font. Complete selected direct
ActualText scopes stay in that Form; the source scope value is cleared. Splitting
a scope is rejected. Every selected glyph is removed from its current source
stream with a numeric positioning replacement; all image/text/metadata patches
use one immutable offset set, applied once per affected Contents occurrence.

The image and search Forms share one native parent and one destination transform.
Later `Owned` moves reuse the same group without recapturing or duplicating OCR.
Reopen checks verify the rewritten source streams, group structure, search-program
hash, native owner, destination and metadata. This is source consistency evidence
performed by the implementation, not an independent extraction/pixel oracle.
Private `WFOcrText` is UTF-8 metadata about font-decoded source text, not extracted
ActualText or a text-search postcondition. Original incremental history and
unused resources remain; this operation is not redaction.

Rust and the governed universal route accept the option. WASM exposes the source
model, preview and apply; the worker/client adds `imageOcrSources`,
`previewImageMove` and `moveImage`, using existing revision checks, cancellation,
output publication and bounded undo/redo. A later linked-story continuation adds
an OCR span-selection panel; see `story_ocr_implementation.md`. Production-app
integration remains unclaimed.

## Boundaries still open

- This primitive is coordinated movement, not OCR-image reconstruction, edited
  scanned wording or automatic image/text association.
- The linked-story integration now batches captures, rebinds disjoint paragraph
  ranges and moves/deletes owned groups. Standalone moves still refuse saved-story
  source pages; use the story transaction. That transaction now supports one
  existing Figure leaf owning the image and all selected OCR; see
  `tagged_ocr_implementation.md`. Separate text-owner migration is still incomplete.
- Nested source Forms, selected named/shared logical properties, partial
  ActualText scopes, optional-content OCR, text-derived clipping and font changes
  through ExtGState remain explicit boundaries. Original image restrictions on
  page groups, rotation/UserUnit and source-state replay also remain.
- Selection is bounded to 4,096 source operands and 4 MB of decoded Unicode;
  stream/program and aggregate source/cache budgets and cancellation checks apply.
- No authenticity, font-permission, PDF/UA, universal compatibility or independent
  viewer certification is implied by private ownership/hash metadata.

## Additional source defects corrected during integration

The destructive text helper and raster/SVG/PostScript writers used a
direction-dependent sign for vertical character spacing. Numeric TJ adjustment
in those rendering routes also always advanced the horizontal axis. All now use
shared signed displacement helpers, alongside extraction. Word-spacing eligibility
now uses encoded single-byte 0x20, not a Unicode-space match, including the shared
decoder used by these renderers. These rules follow
[Adobe PDF Reference 1.6, sections 5.2.1-2 and 5.3.3](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf).

This does not close general vertical-font/shaping support or variable-width CMap
decoding. Rendering and editing must still be qualified together on real PDFs.

## Regression source and deferred qualification

Six native integration cases cover visible duplicates and following-text advance;
cross-stream shared ActualText; rotated/sheared repeat movement; invalid selections
and receipts; clipping/optional-content rejection; and Identity-V spacing through
removal/save/reopen. Three resolver cases cover signed displacement, TJ axis and
encoded-space eligibility. A raster-state case covers a rotated vertical axis.
All are unexecuted source, not passing tests.

Before qualification: compile the exact revision and all bindings, run these
cases and the existing suites, add independent extraction/render comparisons,
exercise both vector exporters and retained/culling paths, validate real fonts,
and measure memory/cancellation/repeat-save growth on the VPS corpus.

Source-only checks completed: rustfmt formatting/parse checks for the touched
Rust files; parse-only formatting output for the large existing editing/universal
modules; `node --check` for the worker and controller; and repository-configured
`git diff --check`. All final checks exited zero. An earlier diff check with
automatic CRLF normalization disabled reported the working tree's CRLF endings
as trailing whitespace; the repository-configured rerun passed without changing
those files. None of these checks typechecks Rust or executes a PDF operation.
