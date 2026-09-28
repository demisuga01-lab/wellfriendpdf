# Story workflow continuation — source implementation, not completion certification

Date: 2026-09-16. Branch `main`, base
`27e62db3a1b84804339e65b6025273fd003b3736`, uncommitted working tree.
Existing dirty changes were retained. No commit, push, branch deletion or deployment.

## Exact outcome

This pass connects additional editing workflows; **the entire requested universal
PDF/editor/rendering roadmap is still not complete**. The limitations below are
engineering work, not merely a pending VPS test campaign.

### Retained native editing in the browser

`StoryEditSession` exposes Rust session open, source analysis, page/annotation
inventory, preview, receipt-bound checkpoint, bytes, save/reopen stories,
undo/redo and text/structural merging through WASM. `StoryPreviewReceipt` binds
the exact revision, request and preview; a changed draft, preview, checkpoint or
undo cannot reuse it. The receipt is not authentication or a digital signature.

`browser/story-worker.js` and `StoryWorkerClient` keep synchronous WASM work off
the UI thread, serialize commands, reject stale revisions and suppress cancelled
results. Cancellation terminates the worker and restores the last published PDF;
it can discard an in-flight checkpoint before its result is published. Whole-byte
undo/redo preimages remain outside the worker, under an eight-snapshot/128 MiB
combined budget. PDF input/output is limited to 256 MiB, JSON to 32 MiB and the
convenience render surface to 16 million pixels. These are guards, not measured
memory/latency guarantees.

`wellfriend-story-editor` is an embeddable SDK component with file opening,
source-order range selection, drawn or numeric approved frame rectangles, frame
linking, paragraph text/order/size controls, font/page-creation options, layout
review, explicit acknowledgement, native checkpoint, download and undo/redo.
Source selections map HTML UTF-16 and normalized CR/CRLF offsets to the exact
PDF scalar indexes. Source text is never inserted as HTML or executable code.
Frame coordinates use the native renderer's CropBox/rotation/UserUnit transform.

The canvas background is native output from the saved PDF. Draft frame boxes
are a **geometry preview**, not final replacement glyph pixels; the textarea
font is not asserted to match the PDF. The component does not provide arbitrary
per-glyph hit testing or an integrated anchor-authoring panel. Applications can
provide richer requests and explicit anchors through the exported controller.
No app-specific Wellfriend frontend outside this repository was changed.

See [integration instructions](../crates/wellfriendpdf-wasm/browser/README.md) and
the [source example](../crates/wellfriendpdf-wasm/examples/browser/story-editor.html).
The package exports and TypeScript declarations include the client/session.
Existing prebuilt WASM bundles have **not** been regenerated.

### Structural offline merging

`story_structure_merge` accepts complete branch snapshots on one exact common
PDF/story revision. It merges independently changed paragraph style fields,
disjoint grapheme-boundary text replacements, noncompeting paragraph insertions
and deletions, conservative paragraph order, and frame rectangles/exclusions.
Same-field competitors, overlapping text edits, competing insertion gaps,
delete/edit or delete/reorder cases return conflicts and **no candidate**.
Branch order is canonicalized for deterministic results.

Source frames/ownership, font assets, signature authority and other mutation
permissions cannot be changed through merging. A merged candidate still goes
through ordinary preview, approval, layout and output checks; the helper does
not save automatically. Snapshot text diffs use one minimal contiguous grapheme
hunk, so some independent multi-hunk changes are conservatively conflicts. The
existing explicit text-patch API permits finer edits. This is not an operation
CRDT, network collaboration service, object/redaction merge or arbitrary frame
topology merger. Font bytes are streamed into fingerprints and are not cloned
once per merged paragraph.

### Paragraph-bound annotation movement

`annotation_anchor_sources` exposes annotation identity/geometry hashes and
complete popup/reply component receipts. Unnamed members have revision-bound IDs
which approved checkpoints persist before page insertion.
`LinkedStoryRequest.annotation_anchors` binds a source annotation to the first
painted line of an explicit paragraph plus a page-space offset. Preview reports
the source and destination geometry. Apply translates supported geometry in the
original dictionaries, updates `/P` and both pages' `/Annots` in one incremental
batch, and verifies resulting rectangles/page ownership after reopening.

Appearance streams, action dictionaries and field ownership are retained, not
regenerated via XFDF. Source geometry hashes survive canonical object renumbering;
they do not fingerprint the whole appearance graph. Saved metadata rebinds the
new geometry, and a different story cannot claim the same saved anchor.

This path requires unique identities, normalized finite rectangles, supported
annotation geometry, crop-box fit and matching page rotation/UserUnit. Popup/reply
groups now move together with explicit complete-group approval, source annotation
order and save/reopen checks; see `story_annotation_group_implementation.md`.
Shared tagged appearances, extended `Path`, measurement and `ExData` geometry
require separate semantic handling. No implicit
image, caption, table or arbitrary artwork migration was added. Later increments
add bounded annotation OBJR/MCR page migration and approved paragraph-leaf tagged
stories; see `tagged_ownership_implementation.md` and `tagged_story_implementation.md`.
Unknown unrelated dictionaries remain preserved.

Continuation insertion now reports full-page invalidations for all shifted output
page indexes, not just story rectangles. Annotation movement also contributes old
and new dirty regions, avoiding a false assumption that page-number caches remain
valid after insertion.

## Verification actually performed

- Rustfmt source parsing/formatting of new Rust modules and parsing of touched
  existing modules.
- `node --check` parsing of client, worker, component and regression-source JS.
  This did not execute those programs or test functions.
- `git diff --check` whitespace inspection.
- Manual source tracing of preview/approval/save/reopen, cancellation/history,
  browser source-offset mapping, native coordinate transforms, annotation
  geometry and structural merge conflict handling.

No Cargo invocation, compilation/type check, test, benchmark, PDF workload,
rendering, browser execution or deployment. Parsing does **not** establish Rust
type correctness, WASM linkage, layout fidelity or browser usability.

Added/extended **unexecuted** regressions cover stale receipts leaving bytes
unchanged, merged story checkpoint/reopen, branch-order invariance, competing
insertions, delete/edit and source-authority conflicts, cross-page annotation
Rect/QuadPoints movement with action/AP preservation, browser queue undo/redo,
cancellation recovery, scalar/CRLF offsets and rotated coordinate mapping.

## Still unfinished code-level scope

- Tag-tree, MCID and ParentTree ownership migration during story repagination.
- General editable table topology, merged cells, row fragmentation and repeating
  headers; existing typed decimal/formula editing is fixed-grid only.
- Automatic image/caption/footnote anchoring, global discrete constraint search,
  document numbering and provenance-safe removal of empty generated pages.
- Complete vertical/variable/exotic-font typography and arbitrary paint-order,
  clipping/state and shared Form occurrence editing.
- Full structural CRDT/history collaboration and a production application's
  integrated direct-manipulation editor; additional retained-session ABIs.
- Arbitrary scan pixel reconstruction, missing original semantic information and
  complete renderer codec/colour/transparency coverage.

No new evidence supports universal compatibility, production readiness or being
better than Acrobat. On the eventual VPS, first compile the exact engine/WASM/
binding revision, then run the added tests and real multilingual, tagged, signed,
encrypted and mixed-layout PDFs with independent extraction/rendering. Include
cancellation, worker restarts, history eviction and repeated-save resource usage.
Do not count old baselines as current results.

## References

- [MDN worker termination](https://developer.mozilla.org/en-US/docs/Web/API/Worker/terminate)
  informs actual worker-stop cancellation rather than a queued cancellation message.
- [PDF Association link geometry](https://pdfa.org/introducing-non-rectangular-links-to-pdf/)
  distinguishes rectangular and quadrilateral link hit regions; both are moved.
- [Adobe PDF reference](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf)
  describes annotation rectangles, local appearances and page-related geometry.
  This implementation is not a conformance certification.
