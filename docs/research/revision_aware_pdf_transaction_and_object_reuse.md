# RAPTOR: revision-aware PDF execution

## Abstract

Revision-Aware PDF Transaction and Object Reuse (RAPTOR) is an execution model
for low-latency PDF parsing, editing, and rendering. Its premise is simple: a PDF
operation should pay only for facts that have not already been established for
the same immutable revision. The system therefore gives every parsed object,
page program, semantic projection, edit plan, decoded asset, and raster product
an explicit revision identity, dependency set, resource cost, and output
contract. Reuse is permitted only when all four still match; otherwise the
artifact is invalidated or recomputed.

RAPTOR is not a reduced-quality renderer and does not weaken edit validation.
It separates unavoidable work from accidental repetition, makes tail latency a
first-class design constraint, and reports metadata access, full traversal,
semantic extraction, rasterization, encoding, and proof as distinct workloads.
The current implementation adds indexed page access, progressive source reads,
bounded immutable caches, prepared edit transactions, exact discrete-color
memoization, retained render products, cooperative cancellation, and a
stage-separated qualification harness.

## 1. Motivation

PDF is a random-access object graph wrapped in a byte-oriented container. Yet a
naive engine can turn a small request—read the page count, edit one operand, or
render one page—into whole-document work. Three pathologies follow.

1. **Scope amplification.** A root metadata query materializes every page or
   hashes and clones data not required by the query.
2. **Revision amnesia.** Planning, applying, reopening, and rendering repeat the
   same immutable analysis because no authenticated artifact survives between
   stages.
3. **Tail amplification.** A mathematically pure transform is evaluated once per
   pixel or glyph even when the input domain contains only a small number of
   distinct values.

Median latency can look healthy while a small feature class dominates P95,
P99, and maximum time. RAPTOR therefore optimizes the dependency graph rather
than a benchmark corpus: the unit of work is the smallest source-derived fact
that can be reused without changing semantics.

## 2. Model

Let an immutable input revision be identified by digest `R`. Every derived
artifact `a` has a canonical key `K(a)`, dependency set `D(a)`, byte cost `B(a)`,
and contract `C(a)` describing modes that affect its meaning: color policy,
optional-content state, viewport, font set, security policy, and resource limits.

An artifact is reusable exactly when

```text
Rrequest = R(a)
and Crequest = C(a)
and every identity in D(a) is unchanged.
```

For an edit with physical and semantic write set `W`, invalidation is the least
fixed point

```text
I0 = W
Ii+1 = Ii ∪ {a | D(a) ∩ Ii ≠ ∅}
I* = fixed_point(I).
```

Unknown ownership widens the affected set to the containing page or document.
It never narrows it speculatively. Under memory budget `M`, the execution
objective is

```text
minimize  open + resolve + decode + interpret + layout + raster + encode + proof
subject to
  observable_output = reference_execution_output
  admitted_cache_bytes ≤ M
  cancellation remains observable
  every declared postcondition is checked.
```

This is a constrained execution problem, not a promise of fixed latency for
unbounded inputs.

## 3. Architecture

### 3.1 Progressive source opening

Path-backed input first reads a small suffix to locate the final cross-reference
anchor. The window doubles only when the marker or complete cross-reference
section is not present, up to a declared bound. Each retry parses into a private
delta map and commits it only after successful completion, so a truncated attempt
cannot contaminate the live object index.

The root page-tree `Count` is the page-count index. Reading it does not instantiate
every page. A requested page walks only the necessary branch and skips sibling
subtrees by validated `Count` values. A malformed index falls back to complete
traversal; the optimization therefore changes ordinary cost, not recovery
semantics.

For a file of `n` bytes, suffix discovery costs `O(min(n, w))` bytes of I/O for
the first successful window `w`, rather than an unconditional large tail read.
A page lookup costs `O(h + v)` object resolutions for tree height `h` and visited
siblings `v`; complete page materialization remains `O(P)` for `P` pages.

### 3.2 Immutable artifact graph

The engine retains bounded artifacts at four levels:

- source: cross-reference entries, trailer chain, encryption context, repair facts;
- object: parsed indirect objects and decoded object streams;
- page: inherited resources, decoded operations, text provenance, display lists;
- output: shaped runs, decoded images, raster tiles, edit candidates, proof facts.

Misses use two-phase locking: inspect under a read guard, derive without a cache
lock, then admit under a short write guard. Large entries are not admitted, and
all caches have explicit entry and byte bounds. A cache hit returns the same
semantic value as fresh derivation.

### 3.3 Prepared edit transactions

Planning produces a revision-bound artifact containing the canonical operation,
candidate identities, policy result, approval surface, read/write sets, and
immutable transaction analysis. Apply authenticates the supplied plan and
approval, then reuses that artifact rather than planning again. A cache miss,
expiry, revision mismatch, or authority mismatch takes the full planning path or
fails closed.

Content-derived identities are evaluated lazily. A request for a revision ID
does not also compute an unused document ID and raw digest; each value retains
its established byte-level definition and is materialized at most once inside
the immutable-input scope. When a prepared plan is admitted, its already parsed
engine may be retained under the same revision and authority key. The cache
charges both the serialized plan and a conservative multiple of the input size
against its byte budget. Apply restores that engine only after independently
checking the supplied bytes against the planned revision.

Derived region, preview, and transaction identifiers compose the authenticated
revision identity with their local parameters. They do not rehash the complete
source bytes under a new prefix for every derived node. This preserves content
binding while changing full-input hashing from one pass per reported identifier
to one pass per identity contract.

Prepared projections are also scope-specific. A local text edit obtains exact
text-scene identities from the retained page text model; it does not enumerate
images, vectors, annotations, and unrelated resource definitions merely to name
the selected text nodes. Provenance construction stops at the operation's
declared cardinality bound instead of hashing every span and discarding the
surplus. A complete editable scene graph remains available to operations whose
dependency set actually includes those object classes.

Immutable policy analysis is cached on the retained engine by operation and
target. The cache is revision-local because it belongs to that engine instance;
it cannot survive a changed byte revision. This removes repeated signature and
permission walks between planning and application without allowing a decision
from one document or field to authorize another.

Mutation still rescans the selected live source range, serializes a new revision,
reopens it, checks source removal and replacement extraction, and enforces any
standards or security obligations. Prepared execution removes duplicate analysis;
it does not convert verification into trust.

### 3.4 Exact discrete-transform memoization

After image samples have been normalized to eight-bit components, a color
transform receives values from a finite domain. For a one-component tint image,
that domain has at most 256 inputs even if the image contains tens of millions
of pixels. RAPTOR evaluates each observed tuple once:

```text
T : {0,…,255}^k → RGBA8
pixel(i) = T(sample_tuple(i)).
```

For `k = 1`, a 256-entry direct table is used; for `k = 2`, a 65,536-entry table;
for wider spaces, a bounded sparse map stores observed tuples and falls back to
direct evaluation after the admission limit. This is exact memoization—there is
no interpolation, quantization, or color approximation. Its cost is
`O(U·F + N)` instead of `O(N·F)`, where `N` is the pixel count, `U` the number of
distinct tuples, and `F` the transform-evaluation cost.

### 3.5 Retained rendering

Display lists, fonts, decoded images, masks, and raster products are keyed by the
complete render contract. Repeated rendering can replay retained immutable work;
an edit invalidates only products in the dependency closure. Cold raster,
retained raster, image encoding, filesystem output, and process startup remain
separate measurements.

### 3.6 Tail-aware scheduling

RAPTOR treats P95/P99 regressions as feature-class failures, not statistical
noise. Each observation records input size, page count, pixel count, color-space
class, operation count, cache state, fallback path, and stage times. Cooperative
cancellation is polled at bounded work intervals. Expensive codecs without an
interrupt callback remain bounded by their nearest governed call boundary.

## 4. Correctness invariants

The implementation maintains these invariants:

- a cache key includes every mode that can change the result;
- a failed progressive parse commits no partial index state;
- indexed page data is checked against materialized structure when full traversal
  is requested;
- prepared plans are revision- and authority-bound;
- mutation and proof operate on the exact output bytes being published;
- exact memoization returns the same transform result for every sample tuple;
- output quality is never traded for speed inside a fidelity-preserving route;
- a timeout cannot silently convert unfinished work into a successful result.

For append-only local edits, unchanged-page proof may use a physical influence
cone instead of re-extracting the whole document. That proof is admissible only
when the original byte prefix is retained, page count is unchanged, rewritten
existing objects are contained in the declared affected page/content set, and no
changed stream is shared by an untouched page. Otherwise validation widens.

## 5. Evaluation protocol

The benchmark suite reports one row per workload and tools as columns. Raw JSONL
retains every document, stage, command, output hash, failure, and timestamp.

Parsing is split into:

- path-backed source/xref open;
- indexed page count;
- requested-page materialization;
- complete page-tree materialization;
- page-program parsing, cold and retained;
- full semantic extraction, cold and retained.

Editing is split into target discovery, canonical planning, approval validation,
mutation/serialization, internal reopen/postconditions, and independent reopen
verification. Rendering is split into open, page-program/display-list work, cold
raster, retained raster, encoding, and complete process time. Visual quality is
reported separately from speed using dimensions, hashes, MAE, RMSE, PSNR,
changed-pixel thresholds, reference disagreement, and readable comparison sheets.

Percentiles use a declared estimator. Reports include standard deviation, median
absolute deviation, P99/P50 amplification, failures, timeouts, and the exact
population. Cross-tool rows are labeled by workload; unlike operations are not
presented as equivalent parsing or rendering.

## 6. Research position

RAPTOR combines established ideas—random-access object indexing, retained-mode
graphics, memoization, incremental computation, and transactional validation—into
a single revision-aware PDF execution graph. Relevant foundations include the
[PDF specification archive](https://pdfa.org/resource/pdf-specification-archive/),
[self-adjusting computation](https://www.cs.cmu.edu/~blelloch/papers/ABBHT09.pdf),
and [parallel self-adjusting computation](https://arxiv.org/abs/2105.06712).

The contribution claimed here is the project-specific composition and its
executable invariants, not established academic novelty. Novelty, generality,
and comparative superiority require peer review and reproducible evidence.

## 7. Limits and falsifiability

No bounded implementation can guarantee a latency percentile for every valid or
hostile PDF. Page dimensions, decompression ratios, font programs, function
graphs, transparency, and proof contracts are unbounded in the format unless the
host imposes limits. RAPTOR therefore makes resource refusal part of the contract.

The model is falsified by any case where a reused artifact changes observable
output, an undeclared dependency survives invalidation, an exact memo produces a
different pixel, or a published benchmark hides a failure or combines unlike
workloads. These are testable conditions. The current empirical status belongs
in timestamped qualification reports, not in the algorithm definition.
