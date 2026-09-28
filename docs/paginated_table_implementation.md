# PDF-native table pagination — source implementation

Date: 2026-09-16. Dirty candidate over main
`27e62db3a1b84804339e65b6025273fd003b3736`.

No compiler, build, type check, test, PDF workload, renderer, benchmark or
deployment was run. Rust formatting/parser checks, JavaScript syntax checks and
tracked diff whitespace checks do not establish executable correctness.

## Implemented connection

`LinkedStoryRequest.table_layout` selects a table layout mode in the existing
story engine. It uses the same approved source ranges/owned frames, signature
policy, font pool, OpenType shaping, bidi context, source writer, continuation-page
batch, save/reopen metadata, cancellation, preview receipt and undo transaction.
It does not create a parallel PDF store or one story per cell.

The topology contains stable row/cell IDs, positive column weights, explicit
row/column spans, padding, fills, optional borders, header rows and row rules.
Every grid position must have one owner, including explicit empty cells. A cell
uses its ID as its paragraph ID by default. Optional ordered `paragraph_ids`
bind independently styled paragraphs without flattening their fonts, colours,
shaping, bidi or spacing. Every paragraph must belong to exactly one cell.
Overlaps, holes, duplicate IDs, spans crossing the header boundary
and contradictory row rules are rejected before mutation.

The older `table_apply_typed_values` route remains available for fixed boxes.
The paginated route supports up to 4,096 cells in one story instead of allocating
a separate persistent story for every cell. There are at most 16,384 paragraphs,
4,096 blocks in one cell and four million source text bytes in one story.

## Multiple styled paragraphs in a cell

Each block retains its real paragraph-local UTF-8 ranges and prepared bidi model.
The paginator carries a separate cell cursor, including one virtual separator
after each explicit block, so consecutive empty blocks and an empty final block
still make progress. These separators never become PDF glyphs or ActualText.
Empty blocks reserve their line height and spacing, but emit no fabricated text.
Preview `paragraph_fragments` records actual paragraph IDs, byte ranges and line
counts; the older cell `logical_byte_range` is the virtual cursor for explicit
blocks and remains ordinary paragraph byte offsets for legacy cells.

Block before/after spacing is applied at its real beginning/end, not replayed on
every page. Paragraph keep-together/keep-with-next rules constrain candidate cell
breaks. Widow/orphan checks apply separately to each block, including intermediate
fragments and actual next-frame widths. Forced frame breaks still use row rules.
Blocks have bounded shared flow caches in addition to prepared paragraph caches.
All non-artifact source ranges are checked for contiguous complete consumption.

Typed numeric/formula/text values can own one paragraph, even when its ID differs
from the cell ID. Multi-block cells do not accept a scalar typed value: draft
recalculation preserves their content instead of overwriting multiple paragraphs.
Text evaluation joins block strings with newlines only in its returned value;
that joined representation does not replace the editable blocks or source PDF.
Duplicate/shared/missing block ownership and size limits are checked before
materializing those values. Failed draft synchronization restores original text.

Tagged cells use explicit per-block ownership, described below; an unbound list
of paragraphs is not silently merged into one semantic owner. Nested tables,
figures and other mixed-object blocks are not implemented by this paragraph path.

## Layout and pagination

- Column weights scale to each approved frame's actual inner width. Cells use
  the shared contextual font assignment, prepared paragraph indexes, paragraph
  bidi levels and final-line measurement used by PDF emission.
- Row heights satisfy both individual-row minima and spanning-cell content
  requirements. Forward constraints on row-boundary positions are solved as
  longest paths in an acyclic graph, yielding a deterministic minimum total
  height under those lower-bound constraints.
- Rowspans form connected sizing groups, but no longer force an indivisible
  page unit. The paginator cuts a row band at permitted row boundaries or
  within a row that explicitly allows splitting. Each intersecting cell keeps
  its own logical byte cursor; its remaining text is reshaped at the next width.
  Cells originating in earlier rows continue beside later-row cells without
  changing their logical row/column span. Explicit keep chains remain atomic.
- A non-splittable row may move as a whole even when a spanning cell continues
  through it. Split glyphs are never simulated by clipping a tall rendering.
  Minimum row heights persist through text and geometry-only continuations.
  Completed cell text is not replayed, and its old empty-line height is not
  repeatedly reserved. A zero-text fragment counts as progress only when rows
  or a real minimum-height requirement advance.
- Widow lookahead uses the next frame's actual column geometry. Final fragment
  counts are checked again; a remaining incompatible constraint is an explicit
  failure, not a successful-overflow report. This is not global optimal pagination.
- Header rows repeat at the start of populated continuation frames. Repeated
  header text and decorative grid paint are marked as artifacts. Explicit
  logical ownership is available through the tagged-table path below; neither
  feature establishes PDF/UA conformance.
- Cell fills and borders are actual PDF paint inside the frame owner. Collinear
  border intervals are merged so shared edges, including long rowspan edges
  adjoining shorter rows, are not painted multiple times.
- Preview records include each cell fragment's rectangle, logical byte range,
  line count and continuation/header flags. Non-repeated cell coverage is checked
  for exact contiguous consumption. Frames unused after contraction are cleared;
  provenance-owned empty-page removal remains a separate roadmap item.

Prepared logical indexes and bounded per-cell/width shaping caches are reused
through pagination. Continuation windows share the cached line buffers, and
prefix height sums avoid re-summing the whole remaining cell on each page.
Different widths or nonmatching line boundaries are reshaped. Output
lines/fragments, topology, formulas and decoded source-grid
bytes have explicit limits; no measured latency or memory claim is made.

## Existing source grid

Moving cell text without removing its original grid would leave stale borders.
For each unowned input table frame, callers must explicitly keep or remove every
intersecting vector occurrence from the existing vector inventory. Zero-width
and zero-height ruled paths are included in this intersection check.

Removal is bound to the original page occurrence, decoded range and byte digest.
The story writer first isolates shared page streams. The table writer then
rewrites all approved path ranges in one private batch, preserving unrelated
bytes and graphics-state operators. Only complete page-owned path/paint sequences
without clipping, marked ownership, optional content, Forms or interleaved
operators use this route. Other paint constructs need an explicit object-graph
migration; the editor does not silently delete them by rectangle.

After save, generated text and grid are part of the same hashed frame owner, so
future edits replace them together. Initial source decisions are not replayed
against new object offsets. Incremental historical bytes remain: this is editing,
not permanent redaction.

## Values and API entry points

Cell `value` is optional. Without it, the styled paragraphs supply plain text.
With it, the existing exact decimal/formula evaluator determines the text. It
supports explicit constants, cell references, add/subtract/multiply and sums,
rejects cycles and missing/non-numeric references, and never silently rounds.

Call `linked_stories::tables::synchronize_values(&mut request)` to materialize
approved values into the draft before requesting a preview. A stale mismatch
between typed values and paragraph text is rejected. Preview/save metadata retains
the formulas and topology, and loading checks their consistency again.
Failed draft recalculation restores the original text; it does not clone the
potentially large font pool merely to stage formula changes.

## Tagged table transaction

`table_layout.tagging` is separate from paragraph `source_tags`. It binds an
existing Table with direct TR children or THead/TBody/TFoot row groups. A TH/TD
may directly own text or contain explicitly approved paragraph paths, such as
TH -> P -> Span, including shared intermediate owners. Paths are retained, not
flattened into TH text.
Every desired row and cell explicitly identifies an existing owner to reuse or
null to create a new owner. TH/TD roles, header Scope and header-cell references
are approved input, not deductions from visual placement or font weight.

Preflight verifies the complete selected subtree and actual text occurrences,
rejects foreign owners, cycles and dangling
references, and requires renewed review of retained Alt/E descriptions. A
reused cell keeps its role and ID. Removing a referenced owner or changing its
role requires further explicit relationship migration; this path fails closed.

`content_paths[cell_id]` lists descendants from the cell's immediate child to its
text leaf; each entry contains `source` and optional `semantic_text` approval.
Omitting an existing path is rejected rather than deleting its intermediate tags.
Leaf MCR ownership is separate from the TH/TD ID used by Headers references.
For independently styled paragraphs, `blocks[paragraph_id]` contains a `path`
of the same source/review entries, starting below the cell root and ending at
one source text leaf. Every paragraph in that cell needs an explicit entry.
An empty path explicitly creates a new direct child paragraph (default P, or an
approved supported `new_role`); its optional `semantic_text` describes that new
leaf. Reused leaves keep their roles and take reviews on their path entries.
`blocks` and the legacy `content_paths` cannot both govern one cell.

The source tree is traversed once with node/depth/path budgets. A prefix-tree
construction checks that the desired paragraph order has a consistent ordered
tree: two paths can share an ancestor, but an order that interleaves another
branch between that ancestor's children requires explicit restructuring. Duplicate
leaf reuse, multiple parents, cycles, foreign-cell reuse, conflicting shared-owner
reviews and mixed structural/paint ownership are rejected before mutation.
Removed descendants still pass the existing Ref/Headers preservation guard.

The transaction stages real object references, not byte offsets, and snapshots
the affected ancestor child lists. These survive canonical page insertion and
renumbering and are checked before finishing. Paragraph leaves receive their own
line MCRs and ActualText; containers receive ordered children and have stale
ActualText/page geometry removed. Each shared ancestor's attributes are rewritten
once. The TH/TD role, persistent ID and header links remain on the cell root.
An originally text-bearing TH/TD can therefore become a block container without
changing the ID referenced by other cells or external structure references.
Empty paragraphs remain explicit empty text owners, not fake marked text.
Saved requests rebind every paragraph path by stable keys for repeated edits.

`groups` partitions desired row IDs in logical order, with explicit THead/TBody/TFoot
roles, optional reused `source` owners and independent description review.
Every imported group must be reused or explicitly listed in `removed_groups`;
an omitted group does not silently authorize flattening. Group boundaries and
the THead prefix are checked against the approved cell spans and repeated headers.

An existing first/last Caption subtree is preserved as opaque, unselected content.
Its effective page association is pinned before changing table/page ownership,
including when it previously inherited Pg from Table. This preserves a caption;
it does not implement automatic caption movement or infer its editing scope.

The shared private story transaction detaches selected source wrappers, edits
the real frame content, inserts continuation pages in a batch, then assigns
page-local MCIDs to non-artifact output lines. Cell MCRs are ordered by approved
frame/line order. Rows and cells are reattached in approved logical grid order;
RowSpan, ColSpan, Scope and Headers attributes are regenerated. Obsolete
Container ActualText and geometric Layout BBox/Width/Height entries are removed,
while unrelated semantic metadata remains. Attribute dictionaries are copied
per owner rather than mutated through shared references. ParentTree and IDTree
are rebuilt from reachable output owners before publication.

Class-based C attributes are expanded from StructTreeRoot.ClassMap before direct
A attributes, preserving the specified precedence and per-attachment revision
numbers. The selected owner receives local copies; shared classes and unselected
users remain unchanged. Indirect attribute streams preserve opaque payload bytes
and are cloned when their dictionaries change. New Table attributes carry the
new structural revision; unknown attributes retain their older revisions, so
their application-specific meaning is not falsely re-qualified. Empty obsolete
Table attribute objects are retired rather than accumulating on every rewrite.
Reference checks use winning owner/namespace properties, including class-based
Headers, so an explicitly overridden old relationship does not remain active.
This resolver also serves paragraph-story attribute rewriting and relationship
checks; it is not a general standards/accessibility certification engine.

Repeated header lines receive no logical-cell MCIDs. The next edit recognizes
their artifact status only inside an approved owned table frame. Table/row/cell
stable keys survive canonical renumbering and are stored in story metadata.
Meaning-dependent approval text is not silently carried into the next edit.
The universal plan includes tag-tree read/write declarations and approval reasons.

Browser request types expose the binding. Imported table stories display cell
roles/header relationships and offer lazy Alt/E review controls for cells,
descendants, rows, groups and Table. Stable-key aliases in the tag inventory keep
those controls bound to current structural owners after canonical renumbering.
The paragraph sibling binding control cannot overwrite table bindings. Owner
mapping and topology editing still use explicit request JSON.

Available routes:

- Existing universal `LinkedStory` operation, with table-aware source identity,
  approval reasons, read/write sets and cell-fragment preview.
- Generic document-subsystem action `table_apply_flowing_story`, containing a
  `story` request. Planning includes the layout preview; approved application
  uses the canonical story transaction.
- WASM `synchronizeTableValuesJson()` and browser client
  `synchronizeTableValues(request)`, followed by ordinary preview/checkpoint.
- Imported/saved table stories work in the browser editor. Text cells can be
  edited there; typed numeric/formula fields and topology currently use request
  JSON plus the recalculation control. Unstructured text cells allow adding,
  reordering and deleting paragraphs inside that cell; deleting its final block
  is disabled. The displayed order follows the cell's approved block order,
  not paragraph storage order. Tagged cells enable the same controls after an
  explicit paragraph-ownership conversion/binding; typed scalar cells keep them
  disabled. Shared semantic owners have one review control applied to all of
  that owner's path occurrences. Paragraph/tag records use own-property access
  and data-property writes, so IDs such as `constructor` or `__proto__` do not
  become inherited bindings. A complete interactive grid/topology UI remains work.

## Regression source, not results

New unexecuted cases cover multi-page growth, repeated headers, native grid
writing, contiguous cell text, bounds, save/reopen contraction, original-grid
decision/removal, merged cells, invalid topology, minimum-height fragments and
exact formula persistence. Additional cases cover multi-rowspan continuation,
non-splittable row boundaries, minimum-height conservation, atomic draft failure,
tagged growth/contraction/repeated edits, artifact-header exclusion from logical
MCIDs, retained header IDs, regenerated span attributes, unrelated metadata,
header cycles and referenced-owner rejection. Further unexecuted regressions
cover nested-path/group growth and repeat editing, renewed leaf description
approval, explicit group dissolution, ClassMap/A precedence and shared-class
preservation, class-based reference overrides, indirect attribute-stream cloning,
attribute revision/size stability, caption page inheritance across insertion and
invalid span arithmetic. A browser-controller case checks draft snapshotting
without publishing PDF bytes. These are assertions to run later, not pass claims.
Additional unexecuted cases cover styled multi-block growth/save/reopen, empty
interior/final blocks without fake text, block-order independence, non-flattening
formula recalculation, aliased scalar-value paragraphs, duplicate ownership and
the tagged single-leaf compatibility boundary. A further layout case requires a
kept heading/body pair to skip a small frame and fit a larger existing frame.
Tagged-block cases add header-ID/external-reference preservation through growth,
reorder/delete/reopen, empty owners, shared nested ancestors, renewed descriptions,
conflicting-review and referenced-removal rejection, and ordered-tree invariants.
These additional regression sources have not been compiled or executed.

## Remaining table work

General nested tables and mixed-object cells; arbitrary external semantic
relationship migration; automatic caption
movement, anchored images/widgets/notes and numbering;
richer border styles and complex source grid
graphs; interactive topology authoring; safe empty continuation-page removal;
global break optimization and full independent rendering/extraction qualification
remain unfinished. The entire roadmap is still active.

## Primary references

- [W3C CSS Tables Level 3](https://www.w3.org/TR/css-tables-3/): spanning cells, row sizing and header repetition informed the model. This implementation does not claim CSS conformance.
- [W3C CSS Fragmentation Level 3](https://www.w3.org/TR/css-break-3/): explicit break constraints and widow/orphan concepts informed the break policy, not a claim of complete fragmentation coverage.
- [PDF Association document-interchange errata](https://pdf-issues.pdfa.org/32000-2-2020/clause14.html): logical structure and table attributes inform ownership handling; actual conformance remains unqualified.
- [Adobe PDF Reference 1.6, sections 10.6.4 and 10.7.3](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf): attribute revisions and table row-group/caption structure inform preservation rules, not runtime proof.
