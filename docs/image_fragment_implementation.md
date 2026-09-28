# Native image relocation — source implementation, not qualification

This continues the image/anchor workstream. Explicit ordinary-story image/caption
flow now uses this primitive; see `story_figure_implementation.md`. Arbitrary
figure migration and the full editor roadmap remain incomplete. No build, test,
PDF execution, rendering or deployment was performed for this change.

## Implemented paths

`crates/engine/src/image_fragments.rs` exposes:

- `preview_image_fragment_move`: binds input SHA-256, occurrence, page geometry
  and an explicit foreground/background choice to a deterministic receipt.
- `apply_image_fragment_move`: rechecks that receipt and publishes only after
  the private output has reopened and source/owner postconditions have passed.
- `image_fragment_bindings`: rediscovers native ownership after save/reopen;
  subsequent moves use these bindings rather than stale byte offsets.

`UniversalEditOperationV2::ImageFragment` uses the existing universal planning,
approval and apply routes, including their outer preservation/conformance gates.
The new candidate requires an explicit selection and visual-change approval.
The Rust API and standalone WASM wrappers call the same implementation; the
WASM functions do not silently replace an existing story session's revision.

## Source-preservation mechanism

A first move captures the page graphics-state prefix up to the selected image.
The canonical tokenizer frames operators, dictionaries, escaped names and
opaque inline-image payloads; the canonical parser resolves operands. The
capture keeps transforms, colours, ExtGState references and supported path
clipping. Preceding paints are suppressed, and preceding text/ActualText is
not copied into a second searchable layer. Closing path paints retain their
close-path/clipping effect without retaining the paint.

The captured program becomes a native Form XObject with the original resource
dictionary. Image samples, image masks, soft-mask references and colour-space
graphs are reused, not rasterized or re-encoded. The source crop is retained as
the Form bounding box, transformed with the image. A required destination
rectangle determines positive axis scaling and translation; scale is bounded
to `1e-9..=1e9` and coordinates to finite magnitudes at most `1e9`.

Only the selected content-stream **occurrence** is cloned and edited. This
distinguishes two uses of the same stream on one page, as well as shared streams
across pages. The selected image paint is removed from the current page stream;
the original definition and historical revisions remain available.

Standalone page selections now also support an image reached through a bounded
nested Form invocation path. Capture constructs an image-only leaf program and
one required-state program for every ancestor invocation. Publication builds a
private capsule chain with the original Form matrices and effective resource
scopes. Independently, it clones the leaf and each selected ancestor, removes
the image only from the leaf clone, gives every parent clone a collision-free
private child name, and redirects only the selected outer page invocation. The
shared image, original Forms and original page stream are therefore left
unchanged for every other occurrence. Structure ownership is deliberately not
copied onto these private visual Forms.

Untagged linked-story batches use the same capture contract through a single
prefix-tree source transaction. Paths sharing a page invocation or deeper Form
prefix share one private clone; selected leaf removals and child redirections
are combined as non-overlapping patches, each affected page stream is rewritten
once, and page resources receive collision-free private roots atomically. This
avoids sequential offset rebasing and last-clone-wins loss when several story
figures originate inside the same nested Form graph.

Foreground placement saves default page state before existing content and
restores it before invoking the capsule. Merely adding `q` immediately before
the image would inherit the preceding clip, transform and opacity. Background
placement executes before original content. Both choices deliberately change
painting order and potentially the blending backdrop; neither is described as
automatic visual equivalence. This design follows the graphics-state and Form
inheritance model in Adobe's [PDF Reference, sections 4.3 and 4.9](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf).

Native ownership markers hold a stable key, destination rectangle and
byte-bound content. Repeated movement reuses the same capsule. Complete
edge-positioned isolation wrappers are retired only when their ownership
metadata and full decoded bytes match; interleaved or externally changed
scaffolds are not guessed away. This bounds live wrapper count for the common
uninterleaved repeated-move path, not every possible mixed editing sequence.
Incremental historical storage and unused source resources are not collected.

## Still incomplete

- Ordinary linked stories now integrate explicitly bound image/caption blocks.
  Selected page-owned Figure leaves and uniquely invoked nested Form-MCR Figure
  leaves use the coordinated tag transaction. A bounded, explicitly approved
  semantic-only Figure descendant tree is retained when its descendants are
  contentless and uniquely parented; explicit page bindings follow the Figure.
  One-shot complete-tree deletion is available only after relationship checks.
  A separately approved reused-Form split copy-on-write clones a contentless tree
  for the residual Figure, rebuilding parent/child/internal relationships and
  removing copied identities/page bindings; external links use the explicit
  subtree-wide split policies. Arbitrary layout inference, floating figures,
  content-bearing/shared reused Figure
  subtrees and mixed-object table cells remain separate work.
- Explicit page-owned invisible OCR operands can now be grouped with the image
  through `ImageFragmentMove.ocr`; see [native OCR groups](ocr_carrier_implementation.md).
  Unselected text, captions, links and annotations stay in place. Story/tag OCR
  coordination and automatic association are still separate implementation work.
- Standalone movement refuses a native image owned by a saved linked story;
  moving it through that story preserves the corresponding ownership metadata.
- Initial capture supports page-owned image paints (including inline images),
  standalone occurrence-specific nested Form paths, untagged linked-story
  nested batches, and the Form-MCR tagged Figure path. Reused tagged Forms remain
  refusal-by-default, but an explicit story-tag decision now moves the existing
  Figure owner with one selected occurrence and preserves a residual Figure leaf
  for the unselected Form invocations. Outbound `/Ref` ownership requires an
  explicit move/retain/copy policy. Incoming `/Ref` owners can follow, retarget
  or reference both resulting Figures through bounded nested direct/indirect
  array graphs; rewritten indirect paths preserve topology and are cloned rather
  than mutated in place. Cyclic/excessive/non-reference relationship containers,
  nested OCR/text-owner groups and Form/page transparency-group
  migration still require coordinated policies. Already-created native capsules
  are reusable.
- Tagged Figure/MCID movement and images carrying logical marked-content
  ownership are refused by this route, not silently detached from their tags.
- Text-derived clipping needs glyph-outline capture. Page transparency groups
  need explicit compositing migration. Unsupported state operators, external
  content files and unbalanced streams are refused.
- Source/target page rotation and UserUnit must match. Unrelated artwork is not
  automatically collision-checked. A new background can change blend results.
- The standalone writer requires a decrypted editing revision and follows
  signature policy. It is ordinary editing, not sanitizing redaction.
- Binding-source existence is not ABI, compiler or browser execution evidence.

## Added regression source — not executed

Cases cover state across Contents members, clipping-path preservation without
preceding paint, logical-text suppression, inline binary isolation, explicit
selection of a repeated shared stream, retention of image/mask definitions,
cross-page moves, save/reopen/repeat moves, native capsule reuse, safe edge
wrapper retirement, negative/stale receipts and the universal approval route.
The nested-Form case selects one of two invocations of a shared two-level Form
chain, moves only that occurrence, and checks that the original image, both
Forms and shared page stream remain byte-identical while the sibling occurrence
survives. A linked-story case selects both shared occurrences, merges their
clone prefixes in one stage transaction, publishes both native owners, reopens
the story and checks that every original source object remains byte-identical.
The VPS campaign must compile these exact changes and check independent pixel,
extraction, colour/soft-mask and multi-reader output, not merely reported flags.
