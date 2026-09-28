# Editing follow-up: source changes and exact remaining scope

Date: 2026-09-16. Working branch: `main`. Base HEAD:
`27e62db3a1b84804339e65b6025273fd003b3736`. Changes are uncommitted and include
the prior dirty candidate; no commit, push, deployment or branch deletion occurred.

Later source continuation: [retained browser sessions, structural merging and
explicit annotation anchors](story_workflow_continuation.md). That report updates
the browser/merge/annotation boundaries below; it does not close the whole roadmap.

## Verification boundary

No Cargo command, compiler/type check, build, test, benchmark, PDF workload,
rendering or deployment was executed. Rustfmt accepted the source syntax and
`git diff --check` checks whitespace only. Added regression functions have **not
run**. This report is not evidence of a compiling, production-ready, universal
or Acrobat-superior editor. The complete requested roadmap is **not finished**.

## Ten reported cases: changes in source

| Reported case | Implemented change | Remaining qualification/boundary |
|---|---|---|
| Empty paragraphs plus repeated breaks index beyond frames | Shared `ensure_story_frame` allocates every skipped frame within the approved page budget; empty/trailing breaks materialize explicitly | Added consecutive-break and page-budget regression; unrun |
| Keep-with-next rejects a small first frame | Try later approved frames before declaring an empty-frame fit impossible | Existing keep-chain regression plus source review; no executed pass |
| Frame subsets lose story-wide coverage on reopen | Catalog `WellfriendStoryFonts` retains licensed, hash-checked full selected programs independently of painted subsets; all mixed-run fonts are included | Fonts cannot acquire missing original glyphs or additional licensing rights |
| Repeated edits accumulate active font resources | New Type0 resources carry exact `WFStoryOwner`; bounded reachability scans content, Forms, patterns, Type3 programs and appearances; only unused matching-owner entries are retired | Unknown/unmarked legacy fonts are retained; incremental history is not erased; resource and byte budgets still apply |
| Older wrapped reflow loses bidi context | `LayoutLine` and `ExplicitLayoutLine` carry resolved paragraph-derived levels; subset/positioned routes preserve them; authoring continuation lines use `draw_text_resolved`; authoring caches include the levels | Vertical layout and independent visual fidelity remain unqualified; legacy candidate measurement is bounded, not a new high-volume layout engine |
| Authoring drops shaping offsets/contextual widths | Reusable CID widths use nominal metrics; each occurrence retains its advance and X/Y offsets and emits an explicit matrix; logical clusters map once | Added occurrence-position regression; no rendered result |
| Joiner carriers are classified as absent outlines | Rustybuzz removes default-ignorable output carriers after shaping, retaining their shaping influence; visible coverage remains strict | Added joiner/variation-control cases and existing missing-subset checks; unrun |
| Widows counted at wrong frame width | Rebreak proposed continuation at the actual next width, move the preceding boundary backward, recheck orphan minimum and final destination | Bounded greedy pagination may still refuse a layout needing global backtracking; it must not report an invalid final one-line continuation as fitting |
| Repeated whole-paragraph preparation per story frame | `PreparedParagraph` retains bidi, grapheme and UAX #14 indexes once per paragraph per layout request | Width-specific shaping remains necessary; no timing/memory result |
| Each continuation repeatedly rewrites the growing document | Canonical `insert_authored_pages_preserving_catalog` copies source once, inserts an ordered batch, adjusts ancestor counts and page labels once, then serializes once; linked and older page-flow routes use it | Authored page buffers still consume memory; no measured scaling claim |

## New features connected to actual code paths

### Contextual multi-font paragraphs

`fonts/fallback.rs` assigns approved fonts to script/whitespace-delimited
contextual units using a bounded dynamic-programming cost (font preference,
metric distance and switches). It never uses scalar-by-scalar fallback. Joining
words, Indic sequences and extended graphemes remain intact. One covering font
is preferred when available; unresolved units return a precise failure.

The paginator and frame writer call the same resolved-level mixed-font shaping
and measurement functions. The writer emits visual font runs inside one logical
`ActualText` line scope, embeds used fonts and persists the story-wide programs.
The mixed-subset regression exercises selection, layout and PDF emission in code;
it was not executed.

### Per-edit preservation contracts

`policy.edit_contract` is part of the universal v2 plan/approval identity. Input
preconditions are checked during planning; actual reopened output is checked
before publication. Supported conditions are exact substring occurrence counts,
unchanged extracted page text, exact serialized PDF object values, opaque decoded
stream slices, and page counts. An optional bounded object-number inventory
records actual pre/post hashes. Font substitutions remain in the operation report.

An optional `render_oracle` requires explicit output pages and supplied reference
rasters for every page. Failed/refused comparisons withhold output. No raster was
generated or comparison executed in this change. References are supplied by the
caller; the SDK does not certify their independence.

These checks cover declared conditions on normalized plaintext output, **not**
arbitrary semantic equivalence, referred-to objects unless separately listed,
historical-byte sanitization, or the final encrypted transport. Object IDs can
change in canonical rewrites; callers must declare output references explicitly.

### Offline logical text merging

`story_merge::{story_fingerprint, merge_story_branches, plan_merged_story}` and
`LinkedStorySession::merge_checkpoint` accept immutable patches against a common
PDF hash and story hash. Edits use paragraph IDs and UTF-8 grapheme boundaries,
verify preimages, deduplicate repeated operation IDs, and deterministically merge
disjoint edits. Overlap and ambiguous insertion boundaries produce conflicts,
never an arbitrary winner. Conflicted checkpoints leave the session unchanged.

The later continuation adds WASM text/structural merge methods, conservative
paragraph/style/order/frame-geometry merging and a worker-backed session. It is
still **not** a full CRDT, networking service, C/Java/.NET merge ABI or arbitrary
object/redaction merger.

### Explicit typed table values

Generic document-subsystem JSON action `table_apply_typed_values` accepts an
approved `TypedEditableTable`. Values are text, decimal coefficients/scales, or
explicit formula ASTs (constant, cell reference, add, subtract, multiply, sum).
Checked i128 decimal arithmetic avoids binary floating-point conversion and
refuses overflow or implicit rounding. Dependency ordering detects cycles and
non-numeric references; no formula is guessed from appearance.

Each cell uses one approved preserve-layout story. Bindings/geometry/preimages
are validated before mutation; descending source edits operate on private bytes.
Failure publishes no intermediate PDF. Values/formulas and cell ownership are
persisted, reopened via `load_typed_tables`, and exposed in universal analysis as
`saved_typed_tables`; externally changed cells require formula reconciliation.

This is **fixed-grid value editing** under a 64-owned-cell-story document limit,
not automatic table recognition, merged-cell topology editing, row fragmentation,
multi-page table movement or repeated headers. The source preview and mutation
may be expensive; no performance target is established.

### Uncertainty-aware scan review

`ocr_reconstruct_visible_words.review` optionally binds alternative readings,
provider/version calibration, exact input hash and per-word review receipts.
Review receipts cover source/replacement text, geometry, font size and OCR-layer
selection. Changed words invalidate receipts. Ambiguous, uncalibrated, handwritten
or graphics-intersecting cases require an explicit reviewer; background
approximation requires separate acknowledgement.

The finite-sample threshold uses the split-conformal `(n+1)` rank and can abstain
on every reading when calibration is insufficient. Held-out score validity and
exchangeability are caller assumptions, not facts established by the SDK.
Recognition confidence does not certify pixel reconstruction or typography.
Omitting this optional richer gate preserves the older explicit-approval API;
the legacy route must not be marketed as calibrated uncertainty handling.

## Not implemented by this follow-up

- Automatic anchored image/caption movement and document-wide discrete constraint
  optimization, footnotes, lists or section/page numbering. The later continuation
  implements explicitly bound supported annotation/widget/link translation.
- General tagged repagination. The later `tagged_story_implementation.md`
  increment migrates approved page-owned paragraph leaves, including MCIDs,
  MCRs, ParentTree and saved identities; nested semantics remain incomplete.
- Full mixed-content/tagged table topology and splitting through multi-row spans.
  The later `paginated_table_implementation.md` increment adds approved topology,
  atomic merged-row groups, single-row splitting and repeated headers.
- Full CRDT operation history and collaboration UI. Conservative logical
  structural three-way merge is added by the later continuation.
- A complete bidirectional PDF/logical lens engine. Opaque-slice contracts and
  owned source spans are useful pieces, not lens-law or arbitrary-object proof.
- Arbitrary scan pixel recovery, full vertical/variable-font typography,
  per-glyph paint interleaving in every route, or expanded renderer codec/colour
  coverage. No claim that these are merely testing tasks.
- Production-application browser integration, additional C/Java/.NET session ABIs
  and performance qualification. The later continuation adds an embeddable WASM
  editor and worker controller. Generated-page pruning remains unimplemented.

## VPS handoff

First pin and compile the exact eventual revision across engine/server/bindings.
Then run regressions covering the ten cases plus mixed-font save/reopen, contract
failure immutability, merge arrival-order/conflicts, decimal/cycle failures,
typed-table edit/reopen/reconcile, calibration/receipt changes and cancellation.
Use independent extraction and rendering, a multilingual/hostile PDF corpus,
tagged/signed/encrypted documents, dense resources and repeated-edit size/RSS
measurements. Do not count old baselines as passes for these changes.

## Primary references consulted

- [HarfBuzz buffer semantics](https://harfbuzz.github.io/harfbuzz-hb-buffer.html)
  for clusters, positioning and post-shaping default-ignorable handling.
- [Unicode UAX #9](https://www.unicode.org/reports/tr9/) and
  [UAX #14](https://www.unicode.org/reports/tr14/) for paragraph/line bidi and
  logical line boundaries. Algorithm use is not conformance certification.
- [Local-first software](https://www.inkandswitch.com/essay/local-first/)
  informs offline operation design; it does not establish visual PDF convergence.
- [Angelopoulos and Bates, conformal prediction](https://arxiv.org/abs/2107.07511)
  informs the optional candidate-set threshold, with its assumptions disclosed.
