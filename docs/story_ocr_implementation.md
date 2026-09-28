# Linked-story image/OCR coordination

Source-only continuation over `27e62db3a1b84804339e65b6025273fd003b3736`.
The worktree is uncommitted. No compiler, builds, tests, browser execution,
PDF workloads, rendering or deployment were run. This closes another source
integration gap, not the full editor/rendering roadmap or qualification gates.

## Implemented source path

`StoryFigure` accepts one of two one-shot decisions for an initial page-owned
image on a page with invisible text:

- `ocr: { span_ids, expected_text }`: exact invisible source operands approved
  to move with this image; source order determines the expected decoded text.
- `ocr_unrelated: true`: explicit confirmation that the page's invisible text
  does not belong to this image and must remain where it is.

The choices are mutually exclusive. No decision is inferred from matching words
or geometric proximity. Existing `Owned` groups retain their complete contents;
they do not accept fresh initial-source decisions. Saved metadata clears both
one-shot fields after rebinding to the native group. Deleting an owned figure
removes its current visual and search invocation together, retaining its caption
unless the paragraph itself is edited. It is not sanitizing redaction.

Initial captures reuse `ocr_carriers`: original glyph codes, fonts, matrices and
complete direct ActualText scopes are retained in an image/search Form group.
Every image, selected text operand and logical-text cleanup is bound against the
same immutable input. The story detachment batch now accepts replacement patches
as well as paint removals, combines them once per Contents occurrence and verifies
the rewritten streams after reopening. Native search-program metadata is checked
after canonical continuation-page insertion, before final placement.
Reused search programs must contain only invisible glyphs and nonpainting state;
visual path/image/shading paint is refused even if private metadata was changed.

Selected OCR glyphs and paragraph source ranges must be disjoint. The new
`story_ocr_ranges` step collects the selected operands' font-decoded Unicode-scalar
ranges, rejects duplicate/overlapping ownership, and rebases initial paragraph
ranges after the complete detachment batch. Before any paragraph write, it checks
the frame's expected text against the actual rebased document. It never reuses a
stale byte offset or searches for another occurrence of the same wording. This
handles OCR preceding a caption, several captures on one page, varying CMap
Unicode lengths and source streams whose ActualText replacement changes length.
Saved native frame owners continue to use their existing byte-bound ownership.

The capture/rebase/insert/place/metadata sequence stays private until the story
transaction succeeds. Existing signature policy, preview receipts, cancellation,
whole-image/caption layout, changed-page invalidation and undo/redo remain active.
There is a 65,536-operand aggregate story OCR limit in addition to the existing
per-page, decoded-source and capture budgets. Captured program plus source-text
metadata contributes to the aggregate capsule budget.
Reused native Forms are charged for their decoded source program bytes too,
including groups selected for deletion; prior ownership is not a budget bypass.

## Browser and API

The existing universal linked-story route and WASM session accept the fields
through their typed request deserialization. The browser component loads exact
OCR spans with page images, distinguishes invisible source operands, exposes a
multi-select and the explicit unrelated-text confirmation, and shows the selected
text/count on its figure card. Changing the chosen image clears the OCR decision;
changing the revision clears inventories. A new preview and receipt are required
after changing association. Duplicate initial associations are rejected both in
the browser and native transaction.

This is an embeddable source implementation, not evidence of a usable production
frontend or browser/device qualification.

## Remaining boundaries

- A later continuation supports OCR and image paint wholly owned by the same
  Figure leaf; see `tagged_ocr_implementation.md`. Exact content-only sibling
  OCR owners may be assigned by span, and an explicitly approved bounded
  contentless Figure subtree is retained without flattening. Owners or
  descendants with independent semantics/content still require a separate
  migration policy.
- Partial/shared ActualText owners, optional-content OCR, text clipping, font
  changes through ExtGState and nested source-Form selections retain the native
  capture limits. No arbitrary scan-text reconstruction or background repair was
  added here.
- Initial undecided image/OCR association remains a request for an explicit
  choice. A source glyph cannot be both moved with an image and independently
  replaced as paragraph text in the same request.
- Automatic reading-order/association inference, content-bearing/shared reused
  semantic subtrees, nested relationship containers, mixed-object tables,
  cross-story subtree transfer and
  the broader roadmap remain incomplete.
- Incremental historical bytes and unused resources remain. Group consistency
  checks do not establish independent visual/search fidelity or PDF/UA compliance.

## Regression source and checks

Six integration regressions cover batch removal with caption-range rebinding;
continuation insertion and repeated contraction; whole-group deletion; duplicate,
overlapping and stale associations; explicit unrelated text; exact receipt/undo/
redo behavior; and multi-scalar CMap mappings including a supplementary character.
One focused range test covers boundary and empty-range decisions, and one native
program test rejects visual paint inside a reused search Form. These eight
tests are source only and have not run.

Permitted source checks completed: `rustfmt` parsing/formatting of the changed
OCR/story modules and regression files, parse-only formatting of the capability
registry, `node --check` of the browser editor and `git diff --check` (with
`core.safecrlf=false`). All completed without reported errors. No compiler or
runtime result is implied. The caption regression helper now collects the
complete visible source-span range instead of assuming OLD occupies one Tj.

The qualification campaign still needs exact-revision compilation, Rust/WASM and
other binding execution, these regressions, realistic scan/caption documents,
independent extraction/rendering, cancellation/memory tests and browser/device QA.
