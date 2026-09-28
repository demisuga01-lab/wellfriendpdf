# Paragraph-wide line composition — source implementation, unqualified

This extends the paragraph policy and joining-context work. It does not complete
the editor/rendering roadmap. No compiler, tests, PDF workloads or rendering were
run for this change.

## New behavior

`LineBreakSettings.composition` accepts `greedy` (the existing default) or
`balanced`. It is part of the paragraph request, saved policy, review hash and
layout cache identity. Greedy is omitted from serialized settings, so older
omitted-default request hashes are not changed merely by adding this field.

Balanced composition considers a whole forced-break segment at the current
inline width. Each allowed byte boundary is a DAG node. Each fitting, freshly
shaped source range is an edge; widths are not assumed to increase with length.
The chosen path minimizes, in order:

1. Emergency word-breaking opportunities used.
2. Number of lines.
3. Sum of squared normalized unused widths, including the final line.

This is a defined engineering objective, not a universal aesthetic score. The
first pass uses natural opportunities only. If it cannot reach the segment end,
the second includes the existing policy-approved emergency edges. A complete
natural path has zero emergency cost and therefore dominates the second pass.
An already-fitting whole segment is the one-line optimum and avoids graph
construction. Ties use a stable predecessor order.

Measurements retain paragraph bidi, joining summaries, font choices, glyph
offsets and outline extents through the existing callback. The invocation-local
range cache shares measurements across both passes, without reusing metrics from
another font/style/provider invocation. No prefix-width subtraction or visual
glyph-array slicing is used. Callers supplying a measurement callback must keep
its font/style/metric semantics fixed throughout that invocation.

On success the source recurrence evaluates every reachable candidate edge under
this objective and width, unless the one-line optimum already applies. This is
not proof of arbitrary typographic correctness. Explicit budgets can prevent
completion; those return errors rather than a silently substituted greedy plan.

Forced breaks divide independent segments and are never crossed by an edge.
Existing Unicode/Japanese/custom rules, grapheme boundaries and nonbreaking
controls filter both passes. No text or whitespace is removed from returned
logical ranges. This does not add hyphenation, justification stretch/shrink,
dictionary segmentation or font-specific break tailoring.

## Flow integration and costs

`PreparedParagraph` dispatches both single-font and measured/multi-font calls.
Stories, captions, horizontal/vertical flow and table cells already share those
calls and now receive the selected composition. The paginator still handles
height, exclusions, keep chains, widows and the next frame separately.

If no full path fits, composition returns the best path to the furthest reachable
source boundary with `NoFittingContinuation`. A story may use a later, wider
approved frame. A line-count lookahead returns a prefix of the chosen plan and
does not pretend that a cut-off point is paragraph completion. Font errors,
nonfinite widths, cancellation and resource errors are not geometric overflow.

Per invocation, source limits are:

- At most 4,096 graph nodes per forced-break segment, including its start.
- At most 131,072 distinct candidate measurements.
- At most 32 MiB of cumulative source-range bytes submitted for measurement.
- The existing 4 MB effective paragraph input and 100,000 returned-line limits.

Cancellation is checked before and after measurements, on metric-cache hits and
while reconstructing paths. The metric cache cannot exceed the measurement
count; its allocation overhead is not included in the byte-work budget. These
are algorithmic limits, not wall-time, RSS, native-code interruption or throughput
guarantees. Worst-case graph work is quadratic in candidate count, and shaping
cost is additional. Large/complex balanced requests can fail explicitly even
when the user-selected fast mode could produce a layout.

Complete composition plans are not yet retained across separate frame-width or
widow-probe calls. Paragraph indexes and the shaping cache are reused, but graph
optimization can repeat. Variable-width/height global pagination, dependency
constraints and incremental downstream convergence remain separate work.

The older `text_reflow` preview and final optimizer also no longer stop merely
because one measured candidate is too wide. Candidate-loop cancellation is
added. Their existing candidate budget and different cost/hyphenation model
remain; considering previously skipped candidates can reach that budget sooner.
This does not unify that older API with the new story policy or close all its
layout limitations.

## Persistence, bindings and browser controls

Balanced stories require saved-story metadata schema **3**. Default stories use
schema 1; other nondefault wrapping policies use schema 2. Current readers accept
versions 1–3 only when the contained request meets that version's requirements.
Downgraded balanced metadata is rejected by story loading and durable-history
resume. Paragraph policy remains one atomic structural-merge field.

Native status and browser state expose `line_break_policy_version: 2` and
`line_shaping_context_version: 3` after the shared hard-line update. The guard
requires the matching versions.
Deploy the guard, worker and rebuilt WASM together. JSON-based native bindings
use the shared typed request; no alternate layout implementation is introduced.
Rust callers constructing `LineBreakSettings` literals must initialize the new
field or use `Default`.

The browser paragraph panel includes fast/balanced composition with its work-limit
notice. Changing it invalidates preview approval. A fresh receipt is required
before publication, and saved/reopened stories preserve the choice.

## Research basis and limits

Paragraph-wide shortest-path/dynamic-programming selection follows the design
principle described by Knuth and Plass. This implementation uses measured shaped
line edges and the explicit cost above; it is **not** a full TeX boxes/glue/
penalties implementation or a claim to reproduce its output.
[Knuth and Plass, Breaking Paragraphs into Lines](https://gwern.net/doc/design/typography/tex/1981-knuth.pdf)

CSS Text 4 likewise distinguishes line-breaking opportunities from composition
style. The SDK's `balanced` option is not a CSS conformance claim or an exact
implementation of a browser's balancing algorithm.
[CSS Text Level 4](https://www.w3.org/TR/css-text-4/#valdef-text-wrap-style-balance)

Greedy remains a local heuristic; balanced is optimal only for its declared
candidate graph, fixed inline width and additive objective when its budget
completes. Neither establishes full document repagination or Adobe superiority.
General font/script semantics, global page constraints, broader rendering and
all exact-build/corpus qualification remain open.

The subsequently identified hard-separator list divergence, including omitted
VT/FF, is addressed in source by `hard_line_policy_implementation.md`. That update
also splits paragraph shaping into visible hard lines without resetting bidi at
every separator. Blank-only logical carriers, page-break semantics and executable
qualification remain separate; graph-level tests are not glyph/pixel evidence.

## Verification boundary

Twenty-one new regression functions are **unexecuted**: fifteen compositor cases,
five story/table integration cases and one older-reflow case. They cover improved
path cost, non-monotonic widths, natural versus emergency preferences, grapheme/
nonbreaking/custom policies, hard breaks, wider-frame continuation, bounded
lookahead, invalid metrics, cancellation, budgets, per-call cache isolation,
legacy JSON, exhaustive tiny-graph comparison, writing modes, approval changes,
schema downgrade and save/reopen. Existing history, structural-review and
native/browser capability regressions are extended, also unexecuted.

Only source inspection, Rust parsing/formatting, JavaScript syntax and Git
whitespace checks were performed. No Cargo, compiler, type check, test, PDF
workload, rendering, benchmark, browser/device QA, deployment, commit or push.
Synthetic algorithm expectations and written regressions are not executable or
independent visual evidence. The full roadmap remains active.
