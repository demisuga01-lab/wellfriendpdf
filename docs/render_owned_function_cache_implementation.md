# Render-owned function cache and image policy propagation

Status: implemented in source; not compiled, executed or runtime-qualified.
This closes a retained-function ownership gap, not the whole editor roadmap.

## Ownership and retention

- `RenderDocumentCache` now owns a prepared-function cache. A render state takes
  it at checkout and returns it on both existing handoff routes. Offscreen mask
  and transparency-group children share their parent's cache. Independent
  workers do not change one another's retention allowance.
- Each cache remains bounded by 64 entries and 32 MiB of charged graph, exact
  key and entry storage. A render contract can lower that byte cap. Zero skips
  retention without disabling valid function evaluation. A later render adopts
  its own policy, rather than inheriting a previous caller's smaller limit.
- Retained function bytes are included in aggregate resource-cache totals and
  the existing largest-oldest-candidate eviction policy. Aggregate eviction runs
  when the render cache is handed back; it is **not** an all-cache admission-time
  or process-RSS limit. Per-function-cache admissions still enforce their cap.
- Reference keys are scoped to a weak reader-allocation identity. Switching
  readers evicts the previous namespace before lookup and again before cold
  admission. Preparation runs outside the mutex; a competing reader cannot make
  a freshly prepared graph reuse another document's equal object numbers.
  The weak token neither retains the reader nor permits allocation-address ABA.
- Eviction leaves live graph handles and their active-use reservations valid.
  Poisoned optional caches bypass retention without falling back into the
  reader cache. Handoff can replace the poisoned cache. Unavailable telemetry
  does not silently count its storage as zero.

## Function-use policy, including images

- Named paint, shading preflight/paint, scalar soft-mask LUTs and alpha-backdrop
  checks use explicit render-owned function resources. Standalone evaluators
  retain their separate reader-local default. Retargeting shading colour
  provenance preserves its already-configured cache, work/memory policy and
  clipping context instead of silently clearing those fields.
- Image colour conversion now carries the same explicit context through full
  image decode, raw windows, reduced JPEG decode, complete inline dictionaries,
  byte conversion and packed sample-domain conversion. Separation/DeviceN,
  Indexed bases, ICC alternates and Indexed ICC bases no longer silently select
  the reader cache when invoked through these renderer routes.
- Existing CMM-option callers convert to the standalone context; no thread-local
  policy override or new public decoder argument is required. The richer context
  remains crate-private. Native JPEG/JPX codec behavior is not requalified here.
- Cold and warm graph use must still fit the consumer's graph/decoded-stream
  caps and acquire a live temporary-memory reservation. The image decode and
  temporary schedulers retain their separate budgets. Image output buffers,
  native codec working allocations, ICC profiles/transforms, cache keys, parser
  temporaries and allocator overhead are not all charged by the function lease.
- Tint image loops now poll cooperative cancellation. This is not preemption of
  a blocking native codec, nor a cumulative whole-image function-work budget.

## Diagnostics

`FunctionCacheMetrics` reports availability, lookup hits/misses, admissions,
evictions, rejected retention, namespace rebinds, entries, bytes and limits.
Hits count successful lookups, including graphs subsequently rejected by a
stricter active-use policy. Oversized-key and poisoned-cache bypasses are not
lookups. Zero-cap skipped admissions share the `skipped_oversized` counter.
Counters belong to the cache lifetime, not necessarily one render invocation.

`PdfReader::function_cache_metrics()` and
`PdfReader::set_function_cache_byte_limit()` address the standalone reader cache.
`RenderDocumentCache::function_cache_metrics()` and the additive
`RenderContractTelemetryReport.function_cache` field address the render cache.
Neither snapshot is total renderer memory. A poisoned render cache reports
`available: false`; its aggregate byte snapshot is conservatively `usize::MAX`
until recovery. Capability text now discloses handoff-only aggregate eviction
and separate standalone retention. Binding transport execution remains unrun.

## Regression source, not results

Twenty new regression functions were added and **not executed**:

- Seven cache cases cover explicit ownership, reader namespace changes and
  admission interleavings, weak lifetime, limit changes, independent concurrent
  policies, competing readers and poisoned overrides.
- Eight renderer cases cover both cache handoffs, mixed function/image aggregate
  eviction, independent worker limits, reader changes, paint/shading/SMask
  routing, clearing/poison recovery, scheduled XObject/inline-image routing and
  restoring default retention after a zero-cache contract.
- Five image cases cover tint/Indexed/ICC nesting, cold/warm graph and stream
  rejection, byte/packed conversion parity, active sibling reservations,
  zero retention and backward-compatible standalone options.

Existing telemetry serialization assertions and a warmed soft-mask regression
were updated. Assertions about output pixels are expected results in source,
not observed rendering evidence.

Only rustfmt parsing/formatting and whitespace checks ran. No build, compiler,
typecheck, tests, PDF workload, rendering, benchmark, browser/binding execution,
commit, push or deployment ran. Current-revision compilation, independent output
checks, memory/concurrency measurement and the wider roadmap remain open.
