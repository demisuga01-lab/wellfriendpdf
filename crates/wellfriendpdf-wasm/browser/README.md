# Retained PDF-native story editor

Source implementation, not an executed browser release. Build the WASM package
on the testing VPS before serving this module. No generated WASM bundle, browser
test, PDF render, binding build or type check was produced in this change.

```js
import { StoryWorkerClient } from '@wellfriendpdf/wellfriendpdf-wasm/story-client';
import '@wellfriendpdf/wellfriendpdf-wasm/story-editor';

const client = new StoryWorkerClient({
  wasmModuleUrl: new URL('/sdk/pkg/wellfriendpdf_wasm.js', location.href),
  wasmBinaryUrl: new URL('/sdk/pkg/wellfriendpdf_wasm_bg.wasm', location.href)
});
const editor = document.createElement('wellfriend-story-editor');
editor.client = client;
document.querySelector('#pdf-workspace').append(editor);
// Call client.close() when this application session is discarded.
```

Serve the package's `browser` directory, worker and WASM assets through the
application's trusted origin. Configure module-worker/WASM MIME and CSP for the
host environment. Never take executable module URLs from a PDF or untrusted
document metadata. The editor receives explicit user-provided file bytes; it
does not fetch PDFs or upload them. Host font licensing, auth, persistence and
transport encryption remain application responsibilities.

## Editing workflow

The local font picker accepts TTF/OTF/TTC/OTC files up to 4 MiB. Pick an explicit
face, a unique draft lookup name and any required font-signature removal decision.
Then select that name on a paragraph and preview again. It does not install fonts,
replace an existing same-named draft asset, or approve the PDF edit. Supported
variable TrueType and CFF2 faces offer explicit static preparation; CFF2 emits
CFF1 and requires that option, including for a face with no variation axes.
Font import is disabled for collaboration
drafts; detach first when changing structural font assets.
Hosts can use `StoryWorkerClient.inspectFont(bytes)` and
`prepareFont(name, bytes, {source_sha256, face_index, allow_signature_removal})`;
the result's `asset` fits `StoryRequest.fonts`. Native status advertises
`font_asset_protocol_version: 1`. Discovery/preparation leaves PDF bytes and undo
history unchanged. Standalone `<wellfriend-font-picker>` consumers must handle its
cancelable `fontprepared` event and add `detail.asset` to their own draft.
See `docs/font_collection_preparation_implementation.md` for limits and deferred QA.

For static TrueType/CFF1 output, call `prepareFontInstance(name, bytes, request)`.
The request contains `selection` (the same hash/face/signature decision),
`coordinates` (axis tags to design values), and `naming` with `family`, `subfamily`,
`legacy_family`, `postscript_name` and `style_link` (`regular`, `bold`, `italic`,
or `bold_italic`). Optional `accept_redundant_metric_differences` defaults to false.
Omitted axes use defaults; unknown/out-of-range values do not silently clamp.
The worker requires native `font_instance_protocol_version: 4` only for this new
command. It returns the same draft asset shape plus a static-instance report.
The picker exposes these fields when the selected face is variable TrueType or
CFF2. Non-variable CFF2 uses an empty coordinate map.
Review changed names, metric differences and component normalization before using
the asset. Localized identity names are replaced; licence/vendor records remain.
Unhandled tables or hint semantics can prevent preparation. See
`docs/static_truetype_instance_implementation.md` and
`docs/cff2_static_instance_implementation.md`. Optional `cff2_contours` has
`tolerance_font_units` (finite, 1/65536..=0.125) and `allow_hint_loss` (default false).
It requests curve-preserving nonzero union; hinted sources require explicit
hint-loss consent. Omitting it preserves bytes only after a compatibility check;
outlines requiring reconstruction are rejected. The picker exposes both
choices without preselecting them. Reports disclose normalized/dehinted/emptied
glyphs and numerical guards; these are not independent pixel-fidelity proof.
See `docs/cff2_contour_normalization_implementation.md`. No browser/runtime
qualification has been run.

Each paragraph has **Line wrapping and Japanese punctuation** controls. Choose
Unicode or Japanese-strict tailoring, allow emergency letter/number wrapping or
keep overlong words intact, and supply additional no-start/no-end characters.
The native story and table layout use the same settings in horizontal and
vertical modes. Explicit newlines win; protected sequences may require a wider
frame. This is bounded tailoring, not complete CSS/JLREQ typesetting. Changes
invalidate preview approval and require another preview before saving.

Deploy `story-capabilities.js`, the worker and generated WASM together. The worker
requires native status `line_break_policy_version: 2` and
`line_shaping_context_version: 3`; old SDKs must not silently
discard the new request field. Stories with nondefault settings use saved-story
metadata schema 2, schema 3 for balanced composition, schema 4 for physical page
breaks, or schema 5 for validated inline-style spans; older native readers
must be upgraded before editing them.
The text-history event/checkpoint schemas are separate. Native hosts using the
JSON protocol should check the same capability before submitting a policy.
See `docs/paragraph_line_break_implementation.md` for source evidence and limits.
The composition selector offers fast local wrapping or a bounded whole-segment
break optimizer. Balanced mode minimizes emergency breaks, line count and squared
unused widths at the current frame width. It can reach an explicit work limit;
there is no silent fast-mode fallback. It is not global variable-frame pagination.
See `docs/balanced_line_composition_implementation.md` for its limits.
Paragraph-derived neighbour context follows the same lines through native
measurement, font fallback, vertical shaping and PDF emission. Version 2 also
retains the actual nearest nontransparent scalar beyond long mark sequences for
the pinned joining state machine; it is not a complete typography guarantee.
See `docs/joining_context_implementation.md` and
`docs/joining_synopsis_implementation.md`; these paths remain unexecuted.
Version 3 adds shared hard-line handling, including VT/FF, across shaping,
measurement and emission. It preserves paragraph bidi context across line
separators; it does not make FF an automatic page-creation command. See
`docs/hard_line_policy_implementation.md` for evidence and remaining boundaries.

**Review structural branches from this draft** opens typed conflict review.
Import complete base-bound branch snapshots, inspect alternatives, retain or
explicitly resolve unplaced paragraphs, and acknowledge every conflict. Native
resolution protects unrelated automatic changes and source/font/permission
authority. Inconsistent table text is reported, not silently recalculated; the
panel offers an explicit recalculation proposal. Review the complete logical
order before adoption, then use the normal native preview/checkpoint workflow.
Detach an attached causal draft explicitly before structural adoption. A saved
causal text epoch may detach after a structural save; no automatic migration is
implied. Merge packages contain source text/font assets and can retain pending
decisions, but importing always recomputes the review and resets approvals.
The standalone `story-structure-editor` package export and client methods
`reviewStructure`/`resolveStructure` use the same native session. See
`docs/story_structure_resolution_implementation.md`; browser execution and
qualification remain pending.

**Begin text history from this draft** opens the offline logical history panel.
It records exact text selections/insertions, merges exported histories, reports
missing dependencies and offers the combined wording as an explicitly reviewed
unsaved draft. Native preview/checkpoint then binds the canonical history and
saves both the story and its causal checkpoint in one session publication.
Imports never silently overwrite a changed host draft. You can also import a
history package with its base story into an empty history panel while the
original PDF revision is open. Export retains deleted wording and supplied font
assets for a new-epoch package; it is not redaction. Replica IDs are not
authenticated. After saving/reopening, **Resume saved text history** verifies the
stored checkpoint and source owners, then merges any retained same-epoch local
operations. Original scalar identities survive page/layout changes through the
native writer. Ordinary edits can detach saved history; they do not silently
rebase it. **Detach collaboration draft** is an explicit choice before making
non-causal draft changes. Starting a replacement epoch requires its separate
checkbox; old PDF revisions still remain. Arbitrary external rebasing and non-paragraph block
collaboration remain unsupported. See
`docs/story_history_checkpoint_implementation.md` and
`docs/story_text_history_implementation.md` for limits and unexecuted regressions.

Hosts can import `@wellfriendpdf/wellfriendpdf-wasm/story-history-editor` or call
`beginTextHistory`, `mergeTextHistories`, `editTextHistory`, `styleTextHistory`,
`structureTextHistory`, `inlineStyleTextHistory`,
`resolveInlineStyleTextHistory`, `textHistoryDelta`,
and the single/grouped activity methods on the shared client. This adds no server
or automatic network transport.
For durable history use `prepareHistory`, `resumeHistory`, `joinHistory`,
`editHistory`, `styleHistory`, `structureHistory`, `inlineStyleHistory`,
`resolveInlineStyleHistory`, `historyDelta`,
`previewHistory`, and `checkpointHistory`.
Grouped methods are `setTextHistoryOperationsActive` and
`setHistoryOperationsActive`; every target must be an original edit from the
same replica and share the expected activity. Use the normalized
`prepared.source` and exact receipt returned by the preview;
changing either requires another review. History packages exported after resume
contain the story ID and log without stale font/source assets. Import them against
the matching saved checkpoint. Worker cancellation or undo invalidates approval;
preview again before checkpointing.

The history panel also offers **Selective undo / redo** for original text edits
of the chosen replica. Choose a recent edit or enter its exact older sequence.
This adds a new schema-2 activity event; it does not restore old PDF bytes or
remove concurrent edits. Review the resulting wording and adopt it before native
preview/save. APIs are `setTextHistoryOperationActive` and
`setHistoryOperationActive`; explicit atomic groups use the plural methods above.
The panel's **Automatic typing groups** field debounces consecutive keystrokes
for 750 ms and submits one grapheme-safe exact-source operation. It maps DOM
UTF-16 and normalized newlines back to source UTF-8, preserves retained source
line endings, waits for IME composition to finish, enforces the native 4 MiB
edit budget and requires a pending burst to be flushed or discarded before
another history mutation. Integrators using a different editor must provide
their own equivalent grouping policy; the native log deliberately has no wall
clock heuristic.
The **Causal paragraph formatting** panel records typed paragraph-level style and
pagination patches in the same immutable history. Disjoint concurrent fields merge;
different concurrent values for one field are displayed with their operation IDs
and suppress native publication until the exact conflict report is resolved. The
component derives non-conflicting preimages from the current projection and requires
every conflicting field in the selected paragraph to be replaced together. This
does not replace character-range rich-text marks. Client APIs are
`styleTextHistory` and `styleHistory`; see
`docs/story_paragraph_style_history_implementation.md`.
The **Causal inline rich text** panel converts an exact grapheme selection into
stable scalar-atom targets, applies preferred-font/size/colour/shaping fields and
shows per-atom concurrent candidates. Clearing a field restores paragraph
inheritance. A resolution must replace exactly one field over every conflicting
atom for the selected paragraph. Supported horizontal/vertical stories and table
cells use one native style-run model for measurement, shaping, painting and
checkpoint/reopen owner verification. See
`docs/story_inline_style_history_implementation.md`.
The **Causal paragraph list** panel adds stable-ID insertion, single-paragraph
movement, logical deletion and exact resolution of concurrent presence/position
conflicts. Delete-versus-concurrent-content is never silently accepted. These
operations use schema 4 and the same native preview/checkpoint route; see
`docs/story_paragraph_structure_history_implementation.md`.
The panel's **Start a compact replacement epoch** section is available only for
a resumed, non-empty saved history. It plans against the exact current PDF,
checkpoint, canonical history and complete displayed frontier. Applying requires
separate acknowledgement of operation/selective-undo loss, retirement of every
prior-epoch replica and review of the exact plan hash. The new saved epoch has an
empty operation log and refuses prior-epoch history. This is logical compaction:
incremental PDF bytes can still retain old metadata/text, replica acknowledgements
are not authenticated, and the report promises exact byte undo only when the
bounded browser session actually retained the preimage.
Whole-PDF undo is still exact snapshot restoration;
never use it to reuse an issued actor sequence. Rejoin retained events first, or
use a new host-provisioned replica/epoch. IDs are not authentication. See
`docs/story_selective_undo_implementation.md` for semantics and qualification gaps.

The embedded **Native Form / annotation text editing** panel now selects exact
source-local occurrences, plans the native write, and displays before/candidate
PNG pages before explicit approval. It shares the session, cancellation and undo
history. Clear a loaded story draft explicitly before applying a native edit;
this does not delete saved stories. Annotation comments versus synchronized
FreeText are explicit choices. Ordinary plain text fields have a separate choice
that synchronizes the value and every normal widget appearance, preserves reset
values/source defaults, and reviews all widget pages (up to eight in the panel).
Formatted, nested, specialized, rich or scripted fields require advanced mappings
or further implementation. Tagged migration and signature-changing rewrites are
not implicitly approved by this panel. These are source changes,
not an executed UI release or independent-render qualification.

Hosts can also import `@wellfriendpdf/wellfriendpdf-wasm/scoped-text-editor`,
attach the same client to `wellfriend-scoped-text-editor`, or use the client's
`scopedSources`, `planScopedText`, `previewScopedText` and `applyScopedText` methods.
Previews return transferable PNG byte arrays and exact plan/candidate receipts;
they do not publish PDF bytes. `applyScopedText` requires a completed matching
preview in the current worker epoch plus an explicit native approval decision.
The host remains responsible for authorization and human review. Before/candidate
images precede final output gates; apply can still refuse. For the full wire
contract, limits and unexecuted regression inventory see
`docs/scoped_candidate_preview_implementation.md` in the repository.

For a page-logical replacement that crosses several original `BT`/`ET` paint
slots, import `@wellfriendpdf/wellfriendpdf-wasm/paint-partition-editor`, attach
the same `StoryWorkerClient`, and set its `request` to the multi-run paragraph
request. The element exposes every proposed source slot and requires a finite
region for each. An optional 1..=4 MiB font is SHA-256-bound into the approval.
The worker calls the canonical Rust private-preview path, which applies the
approval to disposable bytes, renders bounded before/candidate page PNGs and
returns no candidate PDF. It retains the compact publication receipt internally.
`applyPaintPartitions` accepts only that exact request, proposal, approval, font
and current revision; reviewed native apply must validate the receipt before the
worker publishes one undoable revision. `proposePaintPartitions` and
`previewPaintPartitions` are also available for custom hosts. This is a
same-engine visual review, not independent-render qualification, and none of the
browser source has been executed in the current source-only phase.

Remote browser applications can import
`@wellfriendpdf/wellfriendpdf-wasm/authenticated-paint-partition-client` and use
`AuthenticatedPaintPartitionHttpClient`. It posts bounded multipart requests to
the server's propose, authenticated-preview and authenticated-apply endpoints,
requires the signed receipt in the preview response, and parses the final
`multipart/mixed` response without converting PDF bytes to text. The client
accepts an API key but has no HMAC-key parameter: the receipt-signing key remains
server-only. The host must still show every returned before/candidate image and
obtain the user's authorization before calling `applyAuthenticated`.

Open a PDF and select a saved story, import a revision-bound story JSON request,
or use **New story → Load page source**. Select the exact logical source range,
draw its approved rectangle, then link it. Repeat in page order to approve more
frames. No automatic reading-order relationship is inferred. Paragraph controls
edit text, order and size. Request JSON exposes the additional style, exclusion
and annotation-anchor options supported by Rust.

The **Writing mode** selector sets the story's `writing_mode` to `horizontal_tb`,
`vertical_rl` or `vertical_lr`. Omitted mode keeps older requests horizontal.
Vertical text advances down columns; column progression is right-to-left or
left-to-right. Changing this decision invalidates the preview/approval receipt.
Supported tables, image footprints and caption constraints use the same logical
flow axes; images themselves stay upright. Annotation offsets remain physical
PDF x/y offsets. Tagged vertical-LR needs a PDF 2.0 input; the editor does not
silently change the document version. See `docs/vertical_story_implementation.md`
for the unexecuted source paths and typography limits.

For a tagged PDF, use **Load structure owners** and select a contiguous sibling
interval. Bind it to the draft and review each paragraph's reuse/create choice.
For images, include their sibling Figure leaves in that interval and assign
each image's Figure owner separately from its caption. New ownership for an
untagged source image needs an explicit alternate-description review. Existing
Figure descriptions may remain unchanged while the same image moves.
The inventory's leaf flag is a candidate, not a guarantee that all its content
is editable: native preflight checks complete source ownership. Existing Alt/E
wording requires explicit alternate/expanded-text review (blank removes it).
Nonempty descriptions require renewed review after each checkpoint. Request JSON
exposes `source_tags` for the same operation; `client.tags()` provides the inventory.

Table stories can be imported with `table_layout`: approved row/column spans,
column weights, headers, row split rules and source-grid keep/remove decisions.
Preview contains native cell-fragment geometry and writes real grid/text content.
Use **Recalculate typed table values**, or `client.synchronizeTableValues(request)`,
after changing explicit decimal/formula values in request JSON. This is a draft
operation; it neither saves bytes nor grants approval. Text cells remain editable,
but numeric/formula fields and topology currently require request JSON. Table
paragraph controls operate within a cell without detaching its ownership. Untyped
cells have cell-local add/move/delete paragraph controls; the last block
cannot be removed. `cells[].paragraph_ids` gives the approved order independently
of paragraph storage order. Each block retains its own story style and text.
Tagged cells enable these controls after explicit per-paragraph binding; scalar
typed cells keep them disabled. Preview `paragraph_fragments` reports real per-block byte
ranges; explicit cell cursors include virtual separators that are not PDF text.

Rowspans can fragment across frames without clipping glyphs; explicit row split
and keep rules still apply. Optional `table_layout.tagging` binds an existing
Table, every desired row/cell's reuse/create decision, and explicit TH/TD roles,
Scope and header-cell IDs. Load owners with `client.tags()` and import the full
binding in request JSON. The paragraph-sibling binding button is disabled for
tables. `content_paths` binds a cell's retained single-text descendant chain.
`blocks[paragraph_id]` binds separate paragraph paths below a stable TH/TD root;
an empty path explicitly creates a new P (or supported `new_role`). The **Separate
paragraph tags** button converts an existing single-paragraph binding, preserving
its cell ID and any approved descendant path. Preview/checkpoint still require
explicit approval. Additional/reordered/deleted paragraphs keep complete block
bindings; referenced removals or interleaved shared ancestors can reject a draft.
Each shared source ancestor has one semantic-review control that updates all its
path occurrences, so contradictory per-path reviews cannot be hidden in the UI.
`groups` partitions rows under explicit THead/TBody/TFoot owners. Imported group
removal requires `removed_groups`. Lazy description-review controls cover cells,
descendants, rows, groups and Table. Tag inventory `stable_keys` includes shared
parent aliases used by these controls. Ownership/topology changes still use JSON.
Repeated painted headers are artifacts, not duplicate logical headers. Shared
ClassMap attributes are materialized per changed owner without modifying shared
classes. Nested tables and mixed-object content still need their own layout and
ownership model and are not silently flattened. Runtime and accessibility
qualification remain pending.

The page background uses native rendering of the current saved PDF. Dashed boxes
show native layout geometry only. They are **not** replacement glyph pixels,
and the browser textarea font does not establish final typography. Review the
affected pages, continuation count and actual font substitutions, acknowledge
the exact preview, then apply. The writer rewrites PDF source content; download
returns the resulting bytes, not HTML, an annotation screenshot or a DOM overlay.

Saved frames/anchors are rebound after checkpoint. Exported story requests are
bound to the current SHA; source indexes from a prior revision cannot be replayed.
The session receipt also binds request and preview hashes; changing a draft or
preview invalidates approval. Approval is an explicit client decision, not an
authentication/signature mechanism.

## Cancellation, transactions and history

Native WASM work runs in a dedicated worker. `cancel()` terminates that worker,
rejects outstanding/queued requests and recreates it from the last published PDF.
This can discard an in-flight checkpoint whose output has not been published.
The main controller retains exact byte preimages, with a combined eight-snapshot
/128 MiB undo/redo budget. History has no persistent storage; closing the client
releases it. Input/output size is bounded to 256 MiB and JSON to 32 MiB.

`mergeText` and `mergeStructure` return conflicts or a candidate. They never save
or grant authority. Feed a successful candidate through preview and a new explicit
checkpoint receipt. Source ownership and signature/font permissions cannot be
changed by the structural merger. It supports paragraph text/style, insertion,
deletion, conservative ordering and frame rectangles/exclusions; it is not a
distributed CRDT, arbitrary PDF object merge or redaction merge.

## Boundaries

No app-specific Wellfriend frontend was changed. This is an embeddable SDK client,
not proof of production app integration. Tagged repagination supports approved
page-owned paragraph leaves and explicitly approved bounded contentless Figure
subtrees, not arbitrary nested semantic migration or PDF/UA certification.
Explicit image/caption blocks, including selected page-owned
Figure leaves, now use native story
pagination, batch source-image staging and saved ownership. Load page images,
attach one to a caption paragraph, review dimensions/alignment/stacking, then
preview and approve. Figure boxes show reserved geometry only; saved pixels
come from the native PDF renderer. Saved figures have explicit queued deletion
with cancel/preview/approval and byte-exact checkpoint undo. Cancelling an unsaved
attachment leaves the original image untouched. Image deletion retains caption
text by default and is not sanitizing redaction. Untagged OCR groups, OCR
exclusively owned by the same selected Figure leaf, and one explicitly selected
content-only sibling P/Span OCR owner now move and delete with their native
image. Initial capture requires exact invisible span selection, or an explicit
decision that page OCR is unrelated. Use **OCR semantic owner** to approve
`merge_into_figure`; the source owner is consumed, and owners with extra
semantics or relationships refuse. The same explicit owner policy supports
source Forms; an approved reused-Form split preserves residual Figure and OCR
owners for the other invocations. Exact multi-owner OCR partitions and bounded
contentless Figure subtrees are supported; their explicit page bindings follow
the Figure destination. One-shot complete-tree deletion requires its own browser
approval and refuses surviving relationships. A preserved reused-Form subtree
requires its separate clone checkbox; the residual receives a copy with rewritten
parentage and internal Figure/subtree relationships, without copied IDs or page
bindings. External links use the existing outbound/incoming selectors across the
complete tree; multiple trees split in one request share a coordinated clone map.
Content-bearing/shared reused subtrees,
mixed-object table cells, advanced vertical
typography and general source inference
retain their engine limits. The annotation panel now loads `annotations()`, shows
complete popup/reply group receipts and requires explicit group approval before
attaching to a paragraph. Each member appears in `anchor_moves`; offsets can be
edited and associations unlinked without deleting PDF objects. Unnamed members
are stamped only by an approved checkpoint, preserving identity across page
insertion. Native geometry, ownership, topology and paint-order checks still
govern save; these controls have not undergone browser/runtime qualification.
Encrypted
input/output and signature approvals should use the governed universal API;
this convenience session is for caller-owned plaintext inputs.

Annotation discovery separates the opaque `annotation_id` from page-local `name`.
Repeated names on different pages and unnamed objects remain independently
selectable. The optional anchor policy `rename_conflicting_names` defaults to
false; enabling it permits only necessary destination-name repairs reported in
`anchor_moves[].name_change`. Original name references in scripts or external
FDF are not inferred or rewritten. Review the explicit old/new names in the
preview before checkpointing. Persisted editing IDs remain stable across such
repairs, canonical page insertion and save/reopen.

Standalone image/OCR movement is available through `imageOcrSources(page)`,
`previewImageMove(request)` and `moveImage(request, preview.plan_sha256)`.
Set `request.ocr` to exact selected invisible `span_ids` and their concatenated
font-decoded `expected_text`; do not select by matching words alone. For a
nested Form image, load `formTextSources(page)` and also set `form_target` to
the exact occurrence target that has the same invocation path as the selected
image. The story image panel performs that match and shows only the target's
invisible spans. Page-owned carriers omit `form_target`. Review the
native preview before approval. Existing owned groups move intact without a new
OCR selection. The worker reopens accepted output and the client retains bounded
undo/redo preimages. For linked-story flow, the image panel now selects exact OCR
operands into `figure.ocr`, or confirms `figure.ocr_unrelated`. The transaction
rebases disjoint paragraph sources after the atomic image/OCR detachment. Saved
groups consume these one-shot fields and carry their OCR on repeat checkpoints
and deletion. Tagged initial capture requires native validation of complete
same-Figure ownership or exact, caller-selected sibling owners partitioned by
span ID; a
non-relocatable candidate is not itself authorization. No automatic association
inference is implied.
Nested same-Figure or explicitly partitioned sibling-owner OCR uses bounded
private visual/search Form chains. Outer caller-owned and partial/shared
`ActualText` still refuse.
The Figure-owner panel exposes `preserve_semantic_subtree` and the one-shot
`delete_semantic_subtree` removal approval. Preservation retains a
source-bound descendant tree only when every descendant is semantic-only and
contentless; valid page bindings follow the Figure destination, while native
preflight refuses independent MCID/MCR/OBJR ownership, malformed page bindings,
sharing and cycles rather than flattening them. Reused-Form splitting additionally
requires `clone_semantic_subtree_for_reused_form`; internal Figure/subtree `/Ref`
links follow the clones, while external links require the outbound/incoming
relationship decisions. Deletion
requires the separate complete-subtree checkbox and
remains non-redactive.
Source only; no browser or WASM execution was performed.

Linked-story requests also support opt-in `prune_empty_pages`. The editor's
checkbox enables native planning; the report lists exact input pages proposed
for removal, retained candidates/reasons and the output page count. Only newly
marked, still-owned empty continuation pages qualify. Original/unmarked pages
and pages with surviving content or dependencies remain. Output destinations
use the post-pruning page numbers; geometry overlays use stable frame IDs to
stay on their saved input page until checkpoint. See
`docs/story_page_pruning_implementation.md` for labels, references and limits.
Tile hosts must evict pages beyond the new output count as well as invalidating
reported rectangles. Pruning is not historical-byte sanitization.

Primary browser reference: [worker termination](https://developer.mozilla.org/en-US/docs/Web/API/Worker/terminate).
The no-execution constraint applies to implementation work, not to later user
invocations of preview, rendering or checkpoint methods.
