# Revision-Aware PDF Transaction and Object Reuse (RAPTOR)

## Research and engineering specification

| Field | Value |
| --- | --- |
| Working name | Revision-Aware PDF Transaction and Object Reuse |
| Acronym | RAPTOR |
| Initial specification | 29 September 2026 |
| Repository | WellfriendPDF SDK |
| Specification status | Working research and performance architecture |
| Implementation status | Object/page caches, lazy diagnostics, prepared edit/source/page artifacts, transaction-scoped immutable engine reuse, append-only influence-cone proof, retained-render dependency completion, multi-page/CLI/C/Python/WASM integration, and stage-separated benchmarks implemented; the first 100-PDF VPS campaign completed on 29 September 2026 |
| Current claim | A project-originated composition of established incremental-computation and retained-rendering ideas for immutable PDF revisions |
| Claims explicitly not made | Academic novelty, patentability, fixed latency for every PDF, lossless equivalence to every renderer, or superiority before comparable measurements |

RAPTOR is a latency architecture for PDF parsing, editing, and rendering. It does
not weaken correctness checks, change painting semantics, or special-case a test
corpus. Instead, it assigns every reusable artifact to one immutable source
revision, records its dependencies, retains it under explicit memory bounds, and
invalidates only artifacts reachable from an edit's write set.

The central rule is:

> Parse or derive an immutable fact once per revision; reuse it only while its
> complete dependency identity remains valid; keep verification independent from
> mutation; and disclose cold, warm, core, encoding, and proof time separately.

This is necessary because a single number called “PDF time” otherwise mixes
unrelated work: process startup, cross-reference discovery, semantic analysis,
rasterization, PNG compression, filesystem output, and independent verification.

RAPTOR complements ECBES rather than replacing it. ECBES decides which
revision-bound edit candidate is admissible under evidence and fidelity
constraints. RAPTOR retains the authenticated candidate, immutable parser
artifacts, and renderer dependencies so the selected transaction does not redo
work whose authority and source revision are unchanged. ECBES remains the
decision/proof layer; RAPTOR is the execution/reuse layer.

---

## 1. Research basis

RAPTOR combines established ideas but applies them to WellfriendPDF's native edit
and render transaction graph:

- PDF cross-reference structures exist to support random access; an implementation
  need not scan or parse every object merely to open a document. See the
  [PDF Association's ISO 32000-2 issue text for cross-reference data](https://pdf-issues.pdfa.org/32000-2-2020/clause07.html)
  and its [PDF specification archive](https://pdfa.org/resource/pdf-specification-archive/).
- A retained display list avoids repeatedly interpreting the same page program.
  MuPDF describes this explicitly in its
  [DisplayList documentation](https://mupdf.readthedocs.io/en/1.27.1/reference/javascript/types/DisplayList.html)
  and documents parallel display-list rendering in its
  [C API overview](https://mupdf.readthedocs.io/en/latest/reference/c/overview.html).
- Progressive rendering allows expensive work to be divided and resumed rather
  than blocking one uninterruptible call. PDFium exposes this model through its
  [progressive rendering API](https://pdfium.googlesource.com/pdfium.git/%2B/chromium/2443/public/fpdf_progressive.h).
- Runtime-composed, SIMD-oriented raster stages are a proven implementation model;
  Skia's [raster pipeline](https://github.com/google/skia/blob/main/src/core/SkRasterPipeline.h)
  is an important reference. Vello also documents CPU sparse-strip and GPU paths
  designed for parallel vector rendering in its
  [official repository](https://github.com/linebender/vello).
- Incremental computation can retain a dependency graph and propagate only the
  consequences of changed inputs. The core model is described in
  [Self-Adjusting Computation](https://www.cs.cmu.edu/~blelloch/papers/ABBHT09.pdf)
  and extended to parallel execution in
  [Parallel Self-Adjusting Computation](https://arxiv.org/abs/2105.06712).

The project-originated part is the revision-bound composition: the same dependency
identity controls parser artifacts, editable semantic provenance, render display
lists, and post-edit validation. A formal prior-art and patent search has not been
performed, so RAPTOR must not yet be described as a novel academic algorithm.

---

## 2. Problem definition

Let a PDF revision be identified by a cryptographic digest `R`. Let each derived
artifact `a` have:

- a type and canonical key `k(a)`;
- a bounded value size `s(a)`;
- a set of direct source or artifact dependencies `D(a)`;
- the source revision `R(a)`; and
- a semantic mode, resource budget, and output contract `C(a)`.

An artifact is reusable if and only if:

```text
R(request) = R(a)
and C(request) = C(a)
and every dependency identity in D(a) is unchanged.
```

For an edit transaction with write set `W`, invalidation is the transitive closure:

```text
I0 = W
Ii+1 = Ii union { a | D(a) intersects Ii }
invalidate = fixed_point(I)
```

Unchanged artifacts outside that closure remain valid. Unknown dependency
ownership fails closed by widening invalidation to the owning page or document.

The optimization objective is not merely minimum latency. Under correctness and
memory constraints it is:

```text
minimize  open + decode + interpret + layout + raster + encode + proof
subject to
  output_semantics = unoptimized_output_semantics
  cache_bytes <= configured_budget
  cancellation remains observable
  all claimed postconditions are verified
```

No latency target can be guaranteed for unbounded page dimensions, adversarial
compression, arbitrarily large documents, remote OCR, or deliberately expensive
proof contracts. Those cases must be budgeted or rejected explicitly.

---

## 3. Artifact graph

RAPTOR uses five layers.

### 3.1 Source layer

- header and version;
- cross-reference entries and trailer chain;
- encryption context;
- raw indirect-object source ranges; and
- deterministic repair diagnostics.

Normal open discovers the minimum source structure needed for random access.
Whole-file diagnostics are lazy and execute only when a caller requests a parser
report. Repair diagnostics discovered during mandatory open remain attached to the
revision.

### 3.2 Object layer

Ordinary indirect objects and decoded object streams are retained in separate,
bounded caches. Large individual objects are not admitted. Cache lookup never
changes source semantics: callers receive the same parsed and decrypted object
they would receive from a fresh source parse.

The first implementation uses FIFO admission/eviction so concurrent cache hits
take only a shared lock. A later measured implementation may adopt segmented LRU
or TinyLFU admission if corpus-independent evidence shows a better hit-rate versus
contention trade-off.

### 3.3 Page and semantic layer

- resolved page tree;
- page resources;
- decoded content operations;
- source-scoped text chunks; and
- higher-level layout or document-model products.

These artifacts are immutable for a `ContentEngine` revision and are bounded by
entry count and estimated heap bytes. Page count and indexed lookup use a shared
page-tree projection rather than cloning the entire page list.

A cache miss follows a strict two-phase locking rule: copy an existing artifact
under a read guard, drop that guard, derive the missing artifact without holding
the cache lock, then acquire the write guard only for bounded admission. No miss
path may attempt to upgrade a live read guard. Concurrent misses may duplicate
pure derivation work, but they cannot deadlock or expose a partially built value.

#### 3.3.1 Transaction-scoped immutable engine reuse

Opening the same byte slice through several editing subsystems used to create a
new `PdfDocument`, canonical document projection, ordinary-object cache,
object-stream cache, and page-artifact cache each time. RAPTOR now installs one
synchronous revision scope at every public plan/apply boundary. Nested opens of
the exact same bytes clone the retained `ContentEngine`, whose immutable document
and caches are reference counted. Registered caller fonts remain value-cloned, so
mutating one returned engine does not contaminate the retained default engine.

The scope is deliberately not a global document cache. It is removed by an RAII
guard on success or unwind. Exact clone recognition retains comparable source
bytes only for inputs at most 64 MiB; larger inputs use pointer identity and fall
back to independent opens rather than adding unbounded memory. A different output
revision never aliases the input engine because byte equality is required.

### 3.4 Edit transaction layer

An edit session should retain:

- revision-bound source and semantic artifacts;
- the canonical candidate plan;
- approval authority;
- exact read and write sets;
- staged writer products; and
- independent verification obligations.

Mutation creates a new revision. The new session may inherit only artifacts proven
outside the write-set influence closure. Authentication of a caller-supplied plan
must not be removed for speed; an opaque in-process planned-edit handle can avoid
replanning because the engine itself owns the canonical plan and staged bytes.

The initial implementation uses a bounded, ten-minute process-local prepared
plan cache keyed by `(revision_id, plan_id)`. Apply still verifies the supplied
plan identifier, compares the complete caller plan with the retained canonical
plan, and validates approval. A miss or expired entry takes the established full
replanning path. The cache therefore changes repeated work, not edit authority.

Text plans additionally retain bounded source analysis keyed by the exact
revision, page, source string, and replacement string. The artifact contains the
same-width eligibility result and page-logical multi-run provenance. It has a
32-entry, 64 MiB total, 16 MiB per-entry, ten-minute policy. Apply still rescans
the live mutation tokens and decoded buffers; only immutable analysis is reused.

For append-only local text edits, the postcondition engine may replace an
O(document-pages) untouched-page re-extraction pass with an exact physical
influence-cone proof:

```text
physical_write_set = xref_definitions(output) - xref_definitions(input)
existing_write_set = physical_write_set intersect existing_object_ids(input)

pass only if
  output has the exact input byte prefix
  page count is unchanged
  existing_write_set is a subset of the affected page object and its content streams
  no changed content stream is referenced by an untouched page
  the affected page Annots entry is byte-model equal when no link move was requested
```

Affected-page extraction, source removal, generated-text extraction, and decoded
stream checks still execute. Non-incremental output, declared annotation moves,
shared changed streams, or an unexpected existing-object rewrite cannot use this
fast proof. This is dependency-cone verification, not skipped verification.

### 3.5 Render layer

The existing bounded `RenderDocumentCache` retains font programs/resolvers, glyph
geometry and masks, decoded/scaled images, Form and pattern programs, display
lists, raster tiles, transparency decisions, and dependency identities.

RAPTOR integrates that cache into long-lived Python, C, and WebAssembly document
handles. Adding a caller-owned font invalidates the retained render state. Parallel
rendering may use one cache per worker to avoid serializing unrelated pages.

The renderer also records when the conservative dependency closure for a page
has completed. Repeated display-list or raster hits on the same revision skip the
whole resource-graph walk. Revision reset or page invalidation removes that marker,
so an edited page must rebuild its dependency closure before new artifacts are
admitted.

---

## 4. Quality-preserving acceleration

RAPTOR permits only transformations that preserve the configured render contract:

1. Reuse parsed immutable data instead of reparsing bytes.
2. Reuse decoded assets instead of decoding compressed data again.
3. Replay the same retained display list instead of reinterpreting operators.
4. Reuse raster tiles only under the same revision, viewport, mode, color,
   optional-content, and resource-budget identity.
5. Vectorize mathematically equivalent pixel stages.
6. Separate PNG encoding from raster timing; changing compression effort is not a
   raster-quality improvement and must be reported independently.

Approximate rendering, reduced resolution, dropped effects, lossy image
substitution, or altered font selection are forbidden in a no-quality-degradation
benchmark.

---

## 5. Benchmark contract

Every campaign records exact source SHA, build profile, compiler, machine,
renderer versions, corpus manifest hash, page selection, DPI, output format, warmup
policy, repetitions, and timeout/resource limits.

### 5.1 Parsing

Report separate distributions for:

- cold process startup;
- source open/xref discovery;
- first semantic parse;
- warm semantic parse on the same revision;
- serialization; and
- optional diagnostics.

QPDF `--check`, MuPDF `info`, and a full semantic document model are different
operations and must be labeled rather than presented as equivalent parse work.

### 5.2 Editing

Report:

- target discovery and planning;
- approval creation/validation;
- mutation and serialization;
- strict reopen;
- internal postconditions;
- independent parsing/extraction checks; and
- independent visual verification.

“Verified end to end” is their sum, while “edit core” excludes external process
startup. Both numbers are published.

### 5.3 Rendering

For the same PDF, page, page box, DPI, color mode, and output format, report:

- cold open;
- display-list compilation;
- cold raster;
- warm retained raster;
- image encoding; and
- end-to-end tool process time.

Quality records dimensions, alpha policy, pixel hashes, MAE, MSE, RMSE, PSNR,
maximum channel error, changed-pixel ratios, unsupported operations, and visual outliers.
Reference renderers are allowed to disagree; their pairwise differences must be
published alongside WellfriendPDF comparisons.

The human-readable summary table uses tools as columns and benchmark/statistic
rows, while raw per-document/per-page observations remain available as JSONL.

---

## 6. Acceptance rules

The requested latency SLOs are qualification targets, not constants embedded in
source. A campaign passes only if the measured supported population satisfies all
declared percentiles and maximums, with timeouts and refused files counted and
explained. Results may be stratified by file size, page count, image pixels, and
feature class, but the aggregate must not silently exclude difficult documents.

“10% faster than the fastest renderer” requires the same workload and output
contract, enough repetitions for confidence intervals, and a lower WellfriendPDF
upper confidence bound than `0.90 *` the competitor's median or other preregistered
statistic. One favorable run or different PNG settings do not establish it.

No benchmark result may claim:

- an unexecuted path passed;
- a warm-session time is cold-process time;
- a same-engine check is independent evidence;
- a quality score proves semantic correctness; or
- a bounded corpus proves performance for every possible PDF.

---

## 7. Implementation phases

1. **Phase 1 — shared immutable reuse:** lazy diagnostics, bounded ordinary-object
   cache, shared page tree, bounded page operations/resources/text artifacts,
   retained render caches in language bindings.
2. **Phase 2 — prepared semantic parsing:** cache prepared bidi, grapheme, line
   break, layout, and document-model dependency products; remove remaining
   quadratic reading-order work where measured.
3. **Phase 3 — planned-edit sessions:** opaque canonical plans and staged writes,
   revision-transition invalidation, incremental postconditions, and proof reuse.
4. **Phase 4 — renderer execution:** measured tile binning, sparse coverage,
   SIMD raster stages, parallel display-list replay, and viewport-priority
   progressive scheduling.
5. **Phase 5 — qualification:** exact clean VPS build, regression suites, bindings,
   100-file campaign, independent renderers/parsers, artifact publication, and
   explicit SLO pass/fail report.

Phase completion is determined by executable evidence, not by the presence of this
document.

---

## 8. First measured qualification

The 29 September 2026 VPS campaign used 100 real-world PDFs (236 MiB) with
manifest SHA-256
`c1356fbc00549cb30d20c794207a73b90aa5671b96c590843af8dd89f7743a0f`.
The complete evidence and readable four-renderer sheets are published in the
[RAPTOR qualification report](../reports/raptor-vps-20260929/README.md).

The campaign confirmed exact cache reuse for all 100 core observations: cold
and warm page programs, semantic results, and raster hashes matched. It also
confirmed 98/98 applicable source edits after save/reopen; two of the 100 inputs
were explicitly non-applicable to the unique-word benchmark.

The latency objectives were not achieved:

- structural parse was P50 20.020 ms, P90 93.424 ms, P95 158.914 ms, P99
  254.819 ms, and maximum 315.642 ms;
- edit apply was P50 2,115.375 ms and P99/maximum 8,207.122 ms;
- verified edit end to end was P50 3,586.492 ms and P99/maximum 12,481.508 ms;
- edited-output process rendering was P50 432.789 ms and P90 716.201 ms, but
  P95 was 1,005.492 ms and the `i1040gi.pdf` maximum was 43,689.996 ms.

Wellfriend PDF was at least 10% faster than the fastest reference renderer at
P50 and P90 only. P95, P99, and maximum failed, so the aggregate superiority
target failed. Warm retained raster performance (P50 3.664 ms, P99 41.526 ms)
shows that reuse is effective, while the cold and process distributions show
that image decoding/raster long tails and edit planning/serialization remain
the dominant work.

These measurements refine the next research priorities:

1. introduce bounded region/tile decoding for the `i1040gi.pdf`-class image
   path without changing the render contract;
2. convert prepared edit plans into a durable session API so planning is not
   repeated at save time;
3. batch incremental object serialization and postcondition reads over one
   revision transition;
4. stratify results by image pixels, compressed bytes, object count, page
   program size, and font complexity before selecting the next algorithmic
   optimization; and
5. use repeated runs and confidence intervals before any renderer-speed
   superiority claim.
