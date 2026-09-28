# Linked image/caption blocks — source implementation

This is a continuation of the complete editor roadmap, not its completion.
No builds, tests, PDF workloads, rendering or browser execution were run.

## Connected implementation

`LinkedStoryRequest.figures` binds each image source to exactly one caption
paragraph, with explicit width, height, gap, alignment and foreground/background
order. Sources use the existing native image-occurrence or saved-capsule binding.
No association is inferred from visual proximity.

The canonical paginator reserves the image before its caption. The caption
wraps at the image width; both stay in one frame. A later approved frame is tried
when the first is too small. Pinned exclusions move the image block down; caption
lines also avoid exclusions. Keep-with-next lookahead includes image geometry.
An empty caption reserves a real image block without inserting fake text.
Continuation-page budgets and preserve-layout restrictions still apply.

Preview checkpoints retain figure placements alongside text lines. Figure inputs
participate in layout-cache and approval hashes; source pages receive conservative
whole-page invalidation. Changing a caption/figure invalidates old approval.
Paragraph/figure lookups are indexed for the layout pass.

The writer performs a private two-phase image transaction:

1. Bind all image sources on the original revision. One shared occurrence/owner
   inventory avoids a whole-document inventory for every image. Retain one
   mutation buffer for each selected page-stream occurrence, reject overlap,
   and remove all selected paints in one batch. Shared definitions are untouched.
2. Keep native capsule references in private staging metadata while the existing
   story writer rewrites text and inserts continuation pages. References, rather
   than embedded object numbers or source byte offsets, survive canonical remapping.
3. Place all capsules on final pages, validate geometry/page state, and remove the
   staging entry from the current catalog. Verify reopened native owners, finish
   any approved Figure/caption tag migration, then persist rebound bindings.

The entire intermediate output remains private on cancellation or error. This is
an ordinary incremental edit: historical intermediate revisions and unused
resources can remain in the file; it is not redaction or secure erasure.

Complete, byte-verified native edge wrappers can be retired in batches, avoiding
one new live wrapper pair per ordinary repeated checkpoint. Interleaved unknown
content is retained. Capsules are reused on repeat edits, not repeatedly wrapped
inside new Forms. Each capture pass now opens and fingerprints one immutable
document, analyzes signature policy once, and shares decoded pages through an
aggregate-bounded transaction-local Arc cache. Contexts bind their bytes, revision
and policy together and never survive a mutation. Initial story occurrences on
one page now share one complete operator/state walk: each requested source range
captures its exact prefix at the matching operation, all capsules share Arc-backed
bytes, and the page is still parsed through its end for balance validation. Target
page validation and owned-fragment discovery are also cached once per page. This
removes per-figure PDF reopening, hashing, decoding, prefix walking, balance scans
and owned-marker scans. No memory/latency performance result is claimed.

Saved story ownership prevents reassigning a figure to another story, dropping
its binding silently, or moving it with the standalone image API while leaving
stale story metadata. Page indexes are rebound from stable native owners on load.

`figure_detachments: [{ figure_id, binding }]` is a separate one-shot command.
For an untagged saved story it verifies the exact current native owner, leaves
that painted occurrence in place, removes only the story's saved ownership, and
is consumed after save. The detached binding can then be supplied to another
story on the next exact document revision. This standalone two-checkpoint route
is deliberately not described as atomic and remains untagged-only; tagged
ownership uses the dedicated coordinated transfer below.

For two already-saved stories, the Rust engine also exposes
`preview_story_figure_transfer` and `apply_story_figure_transfer`. The preview
privately checkpoints the exact source detachment, reloads the target story on
that intermediate revision, preserves the source Figure geometry/style, and
plans its attachment to one existing target caption paragraph. Apply requires
the exact plan hash and publishes bytes only after the source metadata no longer
owns the key, the target metadata owns it, and exactly one native paint owner is
reachable. The private intermediate remains in incremental history, so this is
atomic publication to the caller rather than byte-history erasure.
When both saved stories are tagged, the transfer additionally requires one
existing page-owned Figure leaf or explicitly preserved bounded contentless
Figure subtree, plus one reusable target-caption leaf. It checks
both current paint-to-structure selections, moves the exact Figure node between
the approved parent sibling intervals, preserves its semantic strings and other
leaf attributes, rewrites its `/P` and story identity, recomputes both insertion
indexes, and rebuilds the ParentTree. For a subtree, descendants retain their
order and metadata; valid explicit `/Pg` bindings follow the final image page in
the normal target finish transaction. That transaction replaces only the root's
direct source-content position with the relocated image MCID. Publication
revalidates both saved story selections and the ParentTree. Mixed tagged/untagged
transfers, content-bearing/shared Figure subtrees and tagged table-cell objects
fail closed.
The same request is accepted as universal-editing operation kind
`story_figure_transfer`, so every existing JSON universal-plan/apply transport
(C ABI, Python, Java, .NET and WASM) can carry the approval-bound route without
a new native symbol. Dedicated `story_figure_transfer_preview/apply` SDK
functions and matching C ABI, Python, Java, .NET and WASM methods now expose the
same request and exact plan hash without making callers construct the universal
operation envelope. The versioned request remains JSON at foreign-function
boundaries; richer generated language models and product UI remain separate.
The conservative structural merge treats figure source/geometry decisions as
authority that requires explicit replanning, not automatic conflict resolution.

### Explicit saved-figure deletion

`figure_removals: [{ figure_id, binding }]` is a one-shot request field. The
figure must belong to this saved story, be absent from the retained `figures`
array, and carry the exact current native key, page, rectangle and content hash.
Duplicate, retained, foreign, unknown and stale removal owners are refused.
Simply dropping a saved figure from the request is still not deletion authority.

Removal and movement share the same immutable-source batch. A removed figure
gets no new capsule/placement; the reopened output must contain none of its
native paint markers. Deleting every figure also uses this path with zero
placement entries. The preview lists removals and invalidates their source
pages. Signature policy, cancellation, exact approval, and atomic private
serialization still apply. Persisted story metadata clears consumed removal
commands; loading metadata with a pending command is rejected. Captions are
independent text and remain unless explicitly edited/deleted. Historical bytes
and shared resources can remain, so this is not secure deletion or redaction.

## Browser/source surface

The retained WASM session exposes `imageSourcesJson(page)`, routed through
`StoryWorkerClient.images(page)`. The embeddable editor can load page images,
attach one to a caption, edit dimensions/gap/alignment/stacking, and preview its
reserved geometry. These are labelled boxes, not DOM or raster substitutes for
the PDF image. Publication still uses the existing exact-preview receipt,
checkpoint, native saved-page rendering and undo/redo flow.
The editor also queues/cancels saved-image deletion, keeps its caption by default,
and distinguishes cancelling a new attachment (which leaves the original image
untouched) from deleting a saved painted occurrence. Removal appears in the
approval report and is covered by native byte-exact undo/redo.

This is source wiring, not generated-WASM, ABI or browser qualification.

## Explicit remaining work

- Content-bearing or externally shared reused-Form Figure
  subtrees and mixed semantic groups. The in-story route described in
  `tagged_figure_implementation.md` preserves an explicitly approved bounded
  contentless subtree, clone it for a residual reused-Form owner and apply
  explicit external relationship policies across the tree; cross-story transfer
  now carries the same validated contentless tree without flattening it. Captions
  remain separate paragraph owners.
- Untagged source image/OCR groups now use explicit span ownership, batch
  removal, paragraph-range rebinding and owned-group movement/deletion; see
  `story_ocr_implementation.md`. The tagged route now also carries OCR exclusively
  owned by the same selected Figure leaf; see `tagged_ocr_implementation.md`.
  Separate text owners and automatic association inference remain incomplete.
  Unrelated page OCR requires an explicit decision.
- Standalone and linked-story occurrence-specific nested-Form image relocation
  use private capsule/source-clone chains; story batches merge shared path
  prefixes and rewrite each affected page stream once. The tagged route also
  migrates a content-only Figure MCR. A reused tagged Form refuses unless the
  caller explicitly approves a semantic split; that path retains a residual
  Figure leaf for unselected invocations and moves the original leaf with the
  selected image. Outbound `/Ref` can explicitly move, remain or copy between
  those owners. Incoming `/Ref` owners can follow, retarget or reference both
  through bounded nested direct/indirect arrays; affected containers preserve
  topology through copy-on-write. Same-Figure invisible OCR inside the selected nested Form now uses an
  exact Form target plus parallel private visual/search chains; see
  `nested_form_ocr_implementation.md`. Cyclic/excessive/non-reference relationship
  containers, separately owned text/OCR, text-derived clipping, page/Form transparency groups,
  differing rotation/UserUnit, arbitrary float/wrap layout and mixed
  image/table cells remain.
- Untagged saved figures support explicit in-place detachment and subsequent
  revision-bound attachment. The plan-hash-gated two-story atomic-publication
  helper now supports either two untagged stories or two tagged stories with one
  exact page-owned Figure leaf or validated contentless subtree and a reusable
  target-caption leaf. Content-bearing/shared subtrees, mixed tagged/untagged conversion, generated native request
  classes and product UI remain. Operation-specific binding methods are present.
  Deleting a caption or dropping a saved binding still does not silently delete
  the image.
- Opt-in empty owned continuation-page pruning is added in
  `story_page_pruning_implementation.md`; uncertain/dependent pages are retained.
  Global pagination, notes/numbering and production application integration remain.
- Independent pixels, masks/colour/blending, text semantics, repeated checkpoint,
  interoperability, memory and cancellation qualification remain unexecuted.

Admission limits include 1,024 figure blocks per ordinary story, 128 MiB of new
captured programs, 256 MiB of aggregate cached decoded pages per capture pass,
and 256 MiB of retained selected mutation buffers, in addition to the native
fragment and story budgets. These are guards, not
performance guarantees.

Nine additional unexecuted regression cases cover later-frame placement,
multi-image binding through canonical page insertion and backward flow,
save/reopen/repeat ownership, standalone-owner protection, pinned artwork,
empty captions, dropped-owner refusal and stale incremental preview receipts.
The four deletion cases cover mixed movement/removal of shared image definitions,
zero-placement deletion batches, stale/duplicate/retained/foreign ownership,
command consumption on reopen, approval invalidation and exact undo/redo bytes.
Two native-fragment regression cases check Arc page-buffer sharing, aggregate
cache accounting, independent contexts, revision/policy mismatch rejection and
multi-occurrence batch capsules against the prior single-capture result. They
have not been executed either.
