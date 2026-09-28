# Linked-story editing and parser/writer hardening

Date: 2026-09-16. Base revision: `27e62db3a1b84804339e65b6025273fd003b3736`.

Later source work is recorded in [the workflow continuation](story_workflow_continuation.md):
WASM sessions and an embeddable browser client, conservative structural merges,
and explicitly bound annotation movement. Qualification and the broader gaps remain.
Further [table pagination and tagging work](paginated_table_implementation.md)
adds row/column spans, multi-rowspan fragmentation, explicit text-cell semantics,
exact values and owned grid painting. The [full roadmap](universal_editor_roadmap_tracking.md)
remains active; the older limits below describe this report's original scope.

## Verdict

This change implements additional source code, **not the complete universal-editor
roadmap**. No compiler, build, test suite, PDF workload, renderer, benchmark or
deployment was run. Rustfmt's source parser and `git diff --check` were used;
neither establishes type correctness or PDF correctness. New regression tests
are source fixtures, not passing-test evidence.

The SDK must not be described as universally compatible, production-qualified,
or better than Acrobat on the strength of this change.

## The eight audited findings

| Finding | Source change | Boundary still requiring qualification/work |
|---|---|---|
| Moved bidi range reused | Borrow a cloned range before accessing its start | No compiler/type-check result |
| Byte-indexed levels zipped with runs | Read the level at each visual run's starting byte | Full UAX #9 conformance, including controls across wrapped lines, remains unqualified |
| Inline image data lexed as operators | Editing uses canonical `ContentTokenizer::next_spanned` | Inherits canonical tokenizer's inline-image terminator behavior; not a new arbitrary-codec proof |
| Appended reflow changes painting order | Four reflow serialization routes insert at an exact source text object's closing `ET`, preserving order relative to subsequent page artwork; neutralize the source CTM locally without resetting clip/transparency. Generated multi-run replacements discover every selected `BT`/`ET` slot and fail closed by default when consolidation crosses slots. Callers may approve first/last anchoring or supply an exact per-slot text/range/region partition; a revision-bound proposal apportions complete graphemes by selected source coverage and requires explicit regions before apply. Partition anchors apply in descending source order so interleaved paint stays between their generated segments. Horizontal/RTL and automatic/explicit vertical partitions retain paragraph bidi/joining context and are compared glyph-for-glyph with the unsplit OpenType result. Leading/trailing/per-segment inheritance retains grapheme-owned source size, spacing, scale, rise, render mode and paint commands in every partition | Unreviewed geometry inference remains unsafe. Exact partitions refuse OpenType-unsafe splits and use one approved embedded Type0 program for replacement outlines; existing inline clipping/tagged routes remain separate |
| Single-token deletion shifts following text | Use the destructive writer's exact `TJ` displacement compensation instead of an empty string | Vertical and unusual source encodings require executable fixtures |
| Dictionary literals clear operands | Canonical null/boolean values stay operands; direct BDC property lookup skips nested values and diagnoses duplicate ownership | Indirect/shared property ownership still requires its existing governed route |
| Leading/trailing style policies indistinguishable | Choose first/last source typography, retain insertion order, and apply inherited inline paint/size/scale/rise with source endpoint compensation. Zero-width leading insertion selects the following run and trailing insertion the preceding run (edge fallback only), accepts grapheme-safe positions inside an operand, shapes through its embedded or approved coverage font, and stays inside the original text/marked-content scope. The positioned writer preserves resolved transformed matrices, clipping union and the exact source endpoint without inverse synthesis; direct isomorphic ActualText is spliced atomically instead of discarded | Unresolved matrix/font history, mismatched writing modes, multi-line insertion, non-isomorphic/shared ActualText and some mixed-style shaping cases remain bounded. This is not full style-policy closure |
| Escaped PDF resource names | Canonical name decoding; restored source font names are re-escaped through the writer | Non-UTF-8 name compatibility is inherited from the canonical parser |

The last two partial areas above must not be reported as universally closed.

## Typography and font resolution

- Generated Latin/default text now goes through Rustybuzz, including GSUB/GPOS,
  instead of individual cmap lookups and natural advances.
- Bidi runs are itemized into Unicode-script runs without splitting graphemes.
  The hardcoded list of eleven complex scripts is removed.
- A bounded thread-local shaped-run cache hashes actual font bytes, logical
  text and direction (128 entries, 4 MiB payload budget per thread).
- Horizontal generated reflow chooses logical UAX #14/grapheme boundaries,
  measures candidate final lines with the selected font, and reshapes final
  lines. It does not cut an already reordered visual paragraph into lines.
- One logical cluster is mapped once in generated ToUnicode data, even when
  shaping produces multiple glyphs for it.
- Linked stories try embedded source programs, caller-approved full programs,
  and permitted bundled alternatives. Candidates must have shaped glyph
  coverage and editable outline-embedding permission. Metric ranking considers
  representative advances, x-height, weight and italic style.
- Measurement, serialization and embedding use the same resolved bytes.
  Fonts with a no-subsetting restriction are embedded whole, when editable
  embedding is permitted; restriction bits are not bypassed.

The continuation pass also implements:

- Paragraph-context bidi levels, followed by line-specific UAX #9 L1/L2 handling.
  Width probes retain the preceding isolate/embedding context. They reuse the
  paragraph analysis and copy only the candidate line's levels for L1.
- Per-paragraph `shaping.language` and global `shaping.features` settings, used
  for font coverage, measurement and serialization. Feature ranges are rejected
  rather than silently reinterpreted at script-run boundaries.
- Contextual-line cache keys that include resolved levels, language and features.
- Outline/GPOS-based overhang, ascender and descender measurements. The story
  paginator accounts for those bounds in line origins, capacity and exclusions.
- Missing-outline detection for embedded subsets whose cmap still names an
  emptied glyph slot. A nonzero glyph ID alone is no longer sufficient coverage.
- Zero-width glyph metrics are preserved, rather than forced to a width of one.

The latest follow-up adds contextual multi-font assignment inside paragraphs,
with switches restricted to script/whitespace boundaries and shared measurement
and output shaping. A joining word or grapheme is never split between fonts.
Uncovered contextual units still fail closed. Complete vertical typography and
variable-font instances remain incomplete. Geometry checks are not rendered-pixel
or arbitrary-clip fidelity evidence. See the [follow-up report](editing_followup_implementation.md).

## Linked-story API

The existing universal JSON plan/approval/apply path accepts:

```json
{
  "operation": {
    "kind": "linked_story",
    "request": {
      "story_id": "contract-body",
      "input_sha256": "SHA256_OF_THE_EXACT_INPUT_BYTES",
      "mode": "flow_document",
      "frames": [
        {
          "id": "body-1", "page": 1,
          "logical_range": [0, 8], "expected_text": "Old text",
          "rect": [36, 36, 576, 756], "exclusions": []
        }
      ],
      "paragraphs": [
        {
          "id": "paragraph-1", "text": "Replacement paragraph.",
          "preferred_font": "Helvetica", "font_size": 12,
          "line_height": 14.4, "rgb": [0, 0, 0],
          "rtl": false, "orphans": 2, "widows": 2,
          "keep_together": false, "keep_with_next": false,
          "page_break_before": "none"
        }
      ],
      "fonts": [], "allow_font_substitution": true,
      "allow_page_creation": true, "max_new_pages": 64
    }
  },
  "policy": { "allow_font_substitution": true }
}
```

This is a schema example, not an executed request. Source ranges are Unicode
scalar ranges from `analyze_multi_run_text_range`, not UTF-8 byte offsets, DOM
selection indexes or visually guessed word positions. Frame rectangles must
fit the source crop box. Approval covers source ownership, reading order,
resolved font choices, affected pages and continuation pages.

Implemented source behavior:

- Explicit occupied frames, columns and pages can participate in one approved
  story. Frame ranges, expected source text, input hash and nonoverlap are checked.
- Growth distributes text forward; contraction redistributes it from the first
  frame and deletes the old content of unused frames. Original pages are retained.
- Pagination handles explicit breaks, paragraph spacing, keep-together,
  keep-with-next, widows/orphans and pinned exclusion rectangles.
  Keep-with-next reserves the following orphan minimum or keep-together chain
  with actual font metrics and exclusions; widow adjustment rechecks orphans.
- Continuation pages use canonical ordered page insertion, after the last
  approved source frame. The caller bounds creation.
- The source writer removes selected current-revision glyph codes, embeds each
  used font once per frame, places final lines at source-block anchors and reopens
  the result. Internal intermediate bytes are not published on failure/cancel.
- Direct logical-text-only `/Span /ActualText` scopes remain distinguishable from
  tagged/optional-content ownership, including after the first generated edit.
- `preserve_layout` accepts one frame and does not create continuation pages.
- Shared page-content streams are cloned per selected page occurrence before
  mutation. This does not extend page-logical editing into nested shared Forms.
- Every output frame, including an empty one, has a `WFStoryFrame` marked-content
  owner, a paint-model digest and a shaped-run digest. The latter binds the
  approved font bytes plus every emitted glyph ID, CID, mapping, advance,
  offset, orientation and outline metric, and is rechecked after all later
  transaction stages. Later story edits replace that exact source range in its
  existing paint slot; they do not locate it by matching words.
- Apply reports return `rebound_frames` and `output_sha256`. The catalog retains
  bounded story JSON without font binaries or saved signature authority. A
  separate catalog-owned stream registry retains the approved story-wide font
  programs so per-frame subsets do not destroy unchanged-story coverage.
- `load_linked_stories` and the existing universal analyze JSON response's
  `saved_linked_stories` field return verified, current-revision requests. Owner
  markers survive canonical object renumbering; page indexes are rebound by
  marker. Missing, changed or duplicate owners fail closed.
- Font lookup aliases survive embedding so saved stories can resolve their
  embedded programs again. Missing newly typed subset glyphs still need an
  approved full font or permitted fallback.
- Continuation-page creation on signed input requires explicit rewrite approval;
  the plan reports the canonical rewrite instead of claiming prefix preservation.

Generic JSON bindings carry the new operation through the existing API, without
duplicating the story engine. The later retained-session work adds a dedicated C
handle ABI plus .NET, Java, Python and WASM wrappers. Current source compiles and
managed packages build, while full native-loading/browser qualification remains
separate.

## Interactive session

`LinkedStorySession` provides retained document bytes/parsed-document access,
preview versus checkpoint operations, identical-request caching, cached source
validation/font pools, cancellation and dirty rectangles for tile invalidation.
Changed previews reuse a matching paragraph prefix (rechecking the predecessor's
keep-with-next constraint) and reuse the suffix only after boundary state and all
remaining logical/style/font inputs converge. `reused_paragraphs` reports reuse.
Per-frame line breaking has capacity-plus-widow lookahead instead of shaping the
entire remaining story on each continuation page.

Undo and redo retain exact byte snapshots under a combined 8-snapshot / 128 MiB
history budget. A new checkpoint clears redo. `saved_stories()` supplies current
revision bindings after a checkpoint; stale requests are still rejected.

The continuation adds WASM session APIs and an embeddable worker-backed browser
story editor, including exact preview receipts. Subsequent work adds handle-based
C/Java/.NET/Python session APIs, password-aware open and exact-byte credentials.
It does not integrate a separate production application's UI.
Stateless save/reopen story editing remains wired through generic JSON surfaces.
The bindings compile/build; browser and native-loading workflows remain unexecuted.

The subsequent tab-stop increment gives story and table-cell paragraphs exact
left/right/center/decimal U+0009 fields on the writing mode's inline axis. It
retains full paragraph bidi context, source-relative font/style partitions,
logical text, semantic owners, paint receipts and saved paragraph-style history;
nondefault settings or tabbed text require saved-story schema 6. Dot/dash/solid
leaders and perpendicular bar rules are exact artifact geometry in the paint
model and require schema 7; durable history uses seed schema 5 for decorated
settings. Exact multi-character decimal tokens require schema 8 / seed schema
6. User-defined leaders and locale inference are not implemented. See
`tab_stop_layout_implementation.md`. All regressions remain unexecuted.

## Work that remains code-level, not just testing

1. General per-object paint-order/state preservation and nested Form occurrence
   ownership. Source-adjacent zero-width leading/trailing inheritance now uses
   the same-text-object positioned writer for supported horizontal/vertical
   runs, including resolved transformed matrices, marked-content ownership and
   clipping union. Generated multi-run replacement binds every source string to
   its exact text-object slot; the default refuses silent consolidation across
   several slots, while explicit first/last anchoring makes the stacking change
   caller-owned. An exact per-slot partition plan now preserves arbitrary
   intervening paint for replacement segments. Horizontal/RTL partitions retain
   paragraph bidi/joining context and must reproduce the unsplit OpenType glyphs;
   automatic and explicit vertical columns use the corresponding vertical check.
   Inherited source text/paint state is grapheme-owned inside every partition,
   with one approved Type0 outline program. A revision-bound
   Hamilton-apportioned proposal removes manual scalar mapping but leaves
   physical regions/final layout under explicit approval. Unrelated source
   font-family preservation, unresolved source histories and unsupported
   shared-owner decisions remain open.
2. Complete vertical matrix/orientation support and fallback inside currently
   uncovered contextual units. Contextual multi-font runs are now implemented
   in source; qualification remains unrun. The subsequent
   `vertical_typography_implementation.md` increment implements generated vertical
   shaping/metrics/column emission in advanced editing. The later
   `vertical_story_implementation.md` adds uniform vertical-RL/LR stories with
   shared multi-font measurement/emission, physical geometry conversion and
   supported tag directions. Mixed orthogonal stories, ruby/tate-chu-yoko/kinsoku
   and executable qualification remain open. The roadmap tracker supersedes
   historical remaining-work entries as later increments land.
3. Automatic anchored image/caption movement, general mixed-content/tagged tables,
   lists/notes/section/page-number semantics. Approved table topology, single-row
   fragmentation and repeated headers now use the shared story transaction; see
   `paginated_table_implementation.md`. Explicit supported annotation,
   link and widget anchors are now implemented within documented bounds.
4. General nested tagged-structure migration. Approved page-owned paragraph
   leaves and content-only Form-MCR Figure leaves now migrate MCIDs/MCRs/
   ParentTree and preserve selected roles/order. An explicitly approved bounded
   Figure descendant tree is retained without flattening when every descendant
   is contentless and uniquely parented; explicit page bindings follow the Figure
   destination. A separate one-shot approval deletes the whole validated tree
   only when no surviving relationship targets it. An explicitly approved reused
   Form occurrence can copy-on-write clone a contentless tree onto the residual
   Figure, rebuilding `/P`, `/K` and internal Figure/subtree `/Ref` links while
   removing clone IDs, page bindings and story keys. External relationships on
   any member use the same explicit outbound/incoming policies as the root. It
   otherwise keeps a residual Figure leaf for
   unselected invocations;
   outbound `/Ref` ownership has explicit move/retain/copy policies and incoming
   owners have follow/retarget/reference-both policies through bounded nested
   direct/indirect array graphs with topology-preserving copy-on-write rewrites.
   Cyclic/excessive/non-reference relationship containers,
   content-bearing/shared reused Figure subtrees,
   arbitrary inline semantics, tables/lists and layout-attribute migration remain.
   See `tagged_story_implementation.md` for the bounded transaction and APIs.
5. Broader generated-page removal policy. Opt-in pruning now removes empty
   story-owned continuation pages only after owner, vacancy and live-reference
   checks; original/modified/referenced pages stay retained and reported.
6. Production client integration and binding runtime matrices. Retained native
   session APIs exist for C, .NET, Java, Python and WASM, including
   permissions/owner-credentialed Standard-handler open that preserves authority
   while retaining no password. User/open credentials fail closed because the
   session exposes an unencrypted working revision.
   Font ranking now parses each immutable program once per resolution request,
   and horizontal/vertical story pagination reuses request-local parsed sfnt
   faces plus prepared CFF2 outline programs for every width probe and final
   line metric. The cache borrows the bounded approved font pool and cannot
   outlive the request. WASM sessions and an embeddable browser story client are
   implemented in source; no latency, memory or browser result exists.
7. Arbitrary scan reconstruction, exotic font programs and complete renderer/
   image/transparency/colour-space coverage. The new scan review gate does not
   improve the underlying pixel reconstruction algorithm or renderer coverage.

These remain actual implementation gaps, not merely outstanding tests. The
request to finish the entire universal-editor roadmap is not fulfilled by this
patch; it must not be described that way.

## VPS qualification still required

Pin the final commit and run workspace/binding compilation first. Then execute
the added regressions plus independent extraction/rendering checks for:

- null/boolean/nested ActualText properties, inline binary image payloads and
  escaped resource names;
- mixed Arabic/Hebrew/English/numbers, Latin kerning/ligatures and combining marks;
- numeric source displacement, transformed pages, later artwork, overlapping
  text, clipping, alpha and repeated source objects;
- both inheritance policies, repeated edit/save/reopen and existing OCR layers;
- occupied multi-frame growth/contraction, multi-page continuation, impossible
  pagination constraints, stale approvals, cancellation and exact undo;
- peak allocation, font embedding duplication, output-size growth and timeout
  permit release under realistic and hostile documents.

The continuation adds unexecuted cases for empty-frame save/reopen/refill, stale
owner rejection, shared-stream isolation, incremental/full-layout equivalence,
cancelled-session immutability, undo/redo, paragraph-isolate continuation and
subset cmap/outline coverage. These assertions are source, not pass results.
Additional cases cover widow/orphan interaction and keep-with-next chains.

Retain all failures. No Adobe comparison is established until the same tasks and
documents are measured against a named Acrobat release.

## Primary references used for the implementation

- [unicode-bidi visual_runs contract](https://docs.rs/unicode-bidi/latest/unicode_bidi/struct.BidiInfo.html#method.visual_runs): levels and ranges use byte indexing.
- [HarfBuzz buffer and unsafe-to-break contract](https://harfbuzz.github.io/harfbuzz-hb-buffer.html): line boundaries may require reshaping both sides of a cluster boundary.
- Installed Rustybuzz 0.20.1, unicode-script 0.5.8 and ttf-parser 0.21.1 source APIs were inspected for shaping, script tags and embedding-permission behavior.
