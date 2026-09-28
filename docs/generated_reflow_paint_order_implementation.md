# Generated reflow paint-order governance

This source increment closes the silent stacking-order failure in generated
multi-run text replacement. It is source implementation only. No build, test,
renderer, PDF workload, or corpus qualification was executed.

## Implemented contract

`AdvancedTextEditOptions.paint_order_policy` is now a serialized, revision-bound
choice:

- `require_single_source_text_object` is the default. Every selected source
  string must belong to the same logical `BT`/`ET` slot. A text object may span
  ordered page `/Contents` members.
- `anchor_after_first_source_text_object` explicitly places one consolidated
  generated block after the first selected source text object.
- `anchor_after_last_source_text_object` explicitly places it after the last
  selected source text object.

The source scanner uses the canonical content tokenizer, so inline-image bytes,
escaped names, arrays, dictionaries, booleans, and null values cannot be
mistaken for executable text-object boundaries. Reused page-content streams,
opaque streams, malformed nesting, missing source strings, and incomplete text
objects fail closed.

All generated page-logical multi-run routes use this decision, including shaped
per-segment replacement, source-font per-segment replacement, and explicit
supplied-font replacement. Existing same-text-object inline writers continue to
preserve their exact source paint location without requiring an approval.

The mutation report records the applied policy, distinct source-text-object
count, and exact anchor stream object, generation, and decoded byte offset.
Inline and deletion-only routes omit that generated-paint receipt because they
do not relocate a generated block.

The page-logical analysis model also assigns every reported source span a
zero-based `source_text_object` ordinal across the exact ordered content
sequence. A planner can therefore detect a multi-slot selection and request the
decision before mutation rather than discovering it only during apply.

## Exact per-slot partitioning

`AdvancedTextEditOptions.paint_partitions` can preserve paint that lies between
selected source text objects instead of choosing one first/last block anchor.
Each partition binds:

- one selected `source_text_object` ordinal;
- one contiguous Unicode-scalar range of the complete replacement;
- one finite, nonempty layout region; and
- optional final lines covering exactly that partition text.

The plan must cover every selected source text object and the full replacement
exactly once, in increasing source-paint order. Empty ranges explicitly delete
one source slot without generating replacement paint there. Nonempty partitions
share the approved Type0 shaping font but own independent generated content and
logical-text carriers.

When several anchors occupy one physical stream, the transaction injects later
source slots first. This prevents generated `ET` operators from shifting the
ordinal used to locate an earlier original `ET`. All source deletion, generated
segments, font resources and page/Form resource changes remain one atomic
publication.

The report contains a receipt per partition with the source slot, replacement
range, region, generated/empty state, and exact anchor object, generation and
decoded offset.

## Revision-bound proposal and approval

`propose_generated_paint_partitions` removes the need for a host to invent
replacement ranges manually. It groups exact selected source spans by their
`BT`/`ET` paint-slot ordinal and apportions complete replacement graphemes by
selected source-scalar coverage using Hamilton's largest-remainder method. The
result records contributing spans, boundary classes and any source-authored
typed region that is identical across the complete slot.

The proposal is hashed with the exact input bytes and complete edit request. It
does not guess page geometry or apply an edit. The host approves one region and
optional final layout per proposed slot through
`apply_generated_paint_partition_proposal`. Apply recomputes the proposal from
the current revision, rejects stale or altered approvals, builds the exact
partitions, then uses the ordinary shaping-equivalence and atomic source writer.
This is deterministic assistance plus explicit authority, not silent semantic
inference.

The same transaction is now source-wired through the Rust SDK, C ABI/header,
.NET P/Invoke, Java FFM, Python/PyO3 and WASM/TypeScript surfaces. The HTTP
server exposes multipart propose/preview/apply/apply-reviewed endpoints under
`/api/v2/universal-editing/paint-partition/`. All surfaces accept the same
request/proposal/approval JSON contract; none reimplement apportionment or
proposal validation. Every apply surface also accepts the same optional
caller-approved font bytes used by native shaping, measurement and Type0
embedding, so a missing source subset does not become a binding-only refusal.
When font bytes are supplied, the approval must carry their lowercase SHA-256
as `font_sha256`; omitting the font requires that field to be absent/null. Apply
checks this binding before any mutation, preventing font substitution after
layout review.
Password-opened document handles forward their retained, zeroizing input
credential only for exact-source reparse; it is never serialized into the
proposal/report or reused as an output credential. The HTTP route likewise
accepts a separate input `password` field and optional binary `font` field.
Binding smoke/regression source covers the proposal kind, owned output and
apply report, but no binding or server build/execution has occurred in this
source-only phase.

Candidate preview is also canonical Rust rather than a browser-local apply and
render sequence. It applies into private bytes, renders bounded source/candidate
pages, reports pixel-change bounds and native diagnostics, and returns PNG byte
arrays but never candidate PDF bytes. Its compact publication receipt binds the
input, request, proposal, approval, font, candidate output and the displayed
page/PNG/difference evidence. Reviewed apply recomputes the candidate and
withholds output unless that receipt matches exactly. The local receipt is
content-bound rather than an authorization signature. Remote deployments can
use the dedicated `preview-authenticated` and `apply-authenticated` endpoints,
which add and verify a short-lived HMAC-SHA-256 wrapper bound to a public key
id, audience, issuance, expiry and the complete local receipt. The ordinary
apply route is retained for non-interactive integrations that intentionally do
not require a preview gate.

The retained browser worker/client now adds a stricter host workflow. It keeps
the exact request/proposal receipt, accepts reviewed regions and an optional
digest-bound font, calls the canonical private preview, and retains its compact
publication receipt inside the worker. Publication requires the identical
request, proposal, approval, font bytes and session revision; the reviewed apply
must accept that receipt before one bounded-undo revision is published. The
`wellfriend-paint-partition-editor` element exposes this sequence with explicit
human approval. These are same-engine preview safeguards, not independent
renderer evidence, and the JavaScript regression source remains unexecuted.

## Deliberate boundary

First/last anchoring is an explicit visual-order decision, not a claim that one
block can preserve every source relationship. If images, paths, shadings, or
unrelated text occur between selected source text objects, choosing the first or
last anchor puts the complete generated block on one side of that paint.

Automatic unreviewed inference of physical regions is not safe. The implemented
proposal deterministically apportions grapheme ranges, but still requires an
explicit revision-bound region/final-layout approval. Boundaries must be
grapheme-safe. For
horizontal and RTL text, every partition retains the complete paragraph's bidi
levels and non-emitting joining context. The writer shapes the unsplit hard line
and every partition, then accepts the split only when glyph IDs, clusters,
advances and offsets are identical. This permits safe contextual joins while
refusing boundaries that divide a ligature, kerning pair or other inseparable
OpenType result. Both automatic and caller-supplied vertical columns use global
paragraph context for measurement and emission, then apply the corresponding
vertical equivalence rule. `inherit_leading`, `inherit_trailing` and
`preserve_per_segment` reuse the existing grapheme-owned source-style map in
every partition, retaining font size, character/word spacing, horizontal scale,
rise, render mode and exact source paint commands. One approved embedded Type0
font still supplies all replacement outlines; preserving several unrelated
source font-family programs in one contextual run is not claimed.

## Regression source

An unexecuted regression covers two selected text objects separated by a filled
path. The default must reject with `ambiguous_generated_paint_order`; explicit
first and last policies must place generated paint on the approved side. A
partitioned replacement must put its leading segment before the path and its
trailing segment after it, remove both source strings, expose both receipts, and
accept a safe non-whitespace boundary while rejecting an OpenType-unsafe
ligature boundary. A preserve-per-segment case exercises the same partition
transaction with inherited source paint/text state. Proposal coverage includes
deterministic apportionment, approved apply and stale-proposal rejection.
