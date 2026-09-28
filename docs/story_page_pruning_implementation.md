# Owned continuation-page pruning — source implementation

This continues the existing uncommitted `main` candidate based on
`27e62db3a1b84804339e65b6025273fd003b3736`. It closes another bounded source path,
not the entire roadmap. No compiler/build, tests, PDF workload, rasterization,
benchmark, browser/binding execution, commit, push or deployment was run.

## Behavior and authority

`LinkedStoryRequest.prune_empty_pages` defaults to false. It is included in the
revision/layout cache key, structural-merge authority, persisted story request
and preview receipt. Enabling it does not authorize removing arbitrary blank
pages. Newly created continuation pages receive a versioned page dictionary
marker containing the story and frame ownership keys. Old/unmarked pages are
not retroactively classified as generated based on their text, position or ID.
Private ownership metadata is a declaration bound to the input revision, not a
cryptographic attestation of a document's origin.

The planner considers existing continuation pages whose frames become empty in
the proposed layout. It verifies exact native frame bindings first, then checks
all page content outside those frames and approved native image groups. It uses
the canonical content-operation parser and an explicit non-painting operator
set, not extracted whitespace or a visually blank raster. Unknown operators,
unrelated text/artwork/owners, surviving annotations or extra page-level features
retain the page. Visible and invisible text both matter. A story always retains
at least one frame for later editing. Explicit paragraph frame breaks preserve
both boundary frames, including intentional empty/trailing pages; they are not
treated as unused overflow merely because they have no painted text.

A bounded walk of the live PDF object graph checks incoming page references.
Only validated page-tree Kids and exact tag/annotation owners which the current
transaction migrates are exempt at planning time. Indirect Kids arrays and
aliases reached under different permissions are distinguished. Scripts and
numeric page dependencies which cannot safely be remapped retain candidates.
Named destinations, links, articles and other surviving page references protect
the referenced page; they are not silently retargeted elsewhere.

After the existing frame/image/tag/annotation transaction, the writer checks
again: the approved pages must actually be vacant and have no live incoming
references except page-tree Kids. No migration exemption survives this check.
The incremental writer removes only these leaves, drops empty Pages branches,
updates ancestor Counts and leaves surviving page object identities intact.
The output is reopened, its count is checked, and tagged output is checked
against the canonical ParentTree validator before publication.

This changes the current page tree; obsolete indirect objects and historical
revisions may remain in the file. It is not sanitizing deletion or redaction.

## Labels, page mapping and sessions

When PageLabels exists, its bounded number tree is read and the label run for
each surviving page is preserved. A removed run boundary or gap creates a new
entry with the appropriate start number. Style, prefix and other label values
are retained. Without explicit labels, ordinary ordinal page numbers change.
No printed page-number text is inferred or rewritten. This follows the
[PDF label model](https://pdf-issues.pdfa.org/32000-2-2020/clause12.html#1242-page-labels-and-indices),
not a claim of runtime conformance.

`preview.page_pruning` reports:

- `removed_pages`: input page numbers to be removed;
- `retained_pages`: owned candidates kept, each with a reason;
- `output_page_count`: the count after creation and removal.

Preview frames, annotation **destinations**, changed pages and full-page
invalidations use final output numbering. Annotation source pages and deletion
bindings remain input identities. Raw transactional layout is kept separate
from this projection so writes never use premature shifted indexes. Reopened
story metadata excludes removed frames; other stories still rebind by native
identity. Prefix-layout reuse is disabled for pruning previews until it can
reuse the unprojected checkpoints correctly; prepared font reuse remains.
Tile consumers must also evict entries beyond the new output page count.

The shared Rust/WASM/C/Java/.NET/Python request transport carries the option.
The browser offers an explicit checkbox and reports the plan for approval.
Its geometry overlays match stable frame IDs to the saved input page rather
than drawing shifted output frame numbers over unrelated saved pixels.
The governed universal plan records the read/write sets and page-tree removal
impact. Signed-document page removal requires explicit policy override.

## Regression source and remaining boundaries

Eight unexecuted regression sources cover contraction/regrowth/reopen with exact
session history and cancelled/stale approvals; table grid contraction; tagged
owner-tree preservation; moved annotations; original/unmarked/modified pages;
destinations/scripts and numeric labels; indirect Kids-array aliases; and
deliberate empty frame breaks through repeated save/reopen.
These tests have not been run and do not establish visible correctness.

Pages created before this ownership marker are retained. Arbitrary external
dependencies, unknown page features, scripts, numeric navigation/print ranges,
separately owned semantic groups not proved to depart, and unknown paint remain
retention boundaries. General notes/numbering, global pagination optimization,
opaque extension semantics and the full roadmap remain open. Qualification
still needs the exact compiler/bindings, regression suite, independent rendering
and extraction, save/reopen interoperability, cancellation and large-PDF corpus.

## Checks performed

`rustfmt --check` succeeded for the changed story, ownership, anchor, merge and
new pruning/regression modules. Parse-only `rustfmt --emit stdout` succeeded for
the writer root, table layout and universal capability/dispatch registry.
`node --check` succeeded for the browser editor, and
`git -c core.safecrlf=false diff --check` reported no whitespace errors.
These are syntax/formatting checks, not Rust type/macro checking, executed tests,
PDF semantic/visual verification or evidence of production readiness.
