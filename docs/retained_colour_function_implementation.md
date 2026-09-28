# Retained colour functions: source implementation, qualification pending

This increment addresses repeated function preparation and unsafe/expensive
tint-cache identity. It does not complete the editing/rendering roadmap or
establish universal compatibility, measured speed or production readiness.

## Implemented source paths

- Every `PdfReader` constructor creates its own bounded prepared-function cache.
  Indirect IDs are interpreted only within that reader's object/revision and
  decryption namespace. Closing the reader releases cache ownership. Reopening
  an edited revision creates a new cache, even if its bytes or object IDs match.
- Keys contain an exact typed encoding of the supplied object plus input arity
  and single-function/component-array policy. Names/strings, numeric types,
  floating-point bits, dictionary keys, lengths, stream bytes, object numbers
  and generations remain distinct. References are not recursively resolved for
  key construction. Deep/large/wide keys bypass caching rather than hashing a
  truncated graph and treating it as identical.
- Both native evaluator APIs, all seven shading families, shading preflight,
  Separation/DeviceN conversion and soft-mask transfer now use retained graphs.
  Decoding, graph construction and evaluation occur outside the cache mutex.
  Concurrent misses can prepare independently, then reuse the canonical graph
  already admitted by another caller. A poisoned optional cache is bypassed.
- Tint-output cache keys use weak prepared-graph identity and exact input bits.
  They no longer hash/materialize the whole PDF or resolve every child stream on
  each tint sample. Weak references keep allocation identity from being reused
  without retaining the graph's decoded sample/program payload. Alternate-space
  conversion, opacity and colour policy remain outside the cached function
  output, so this does not conflate different rendering contexts.
- Soft-mask `/TR` preparation requires one input and exactly one declared output;
  it no longer discards extra channels. Both direct and indirect `/Identity` are
  recognized. The 256-entry LUT retains one graph and shares one 8,388,608-unit
  work allowance across all samples. Any failed sample rejects the whole LUT.
  Backdrop evaluation uses the same scalar preparation/evaluation path.
- Prepared storage accounting now includes the graph wrapper, root vector
  capacity and its Arc control-block estimate as well as existing node/payload
  charges. Existing per-evaluation work, cancellation and numeric checks remain.

## Explicit resource and correctness boundaries

- Reader cache: at most 64 entries and 32 MiB of charged graph/key/entry storage.
  Keys are capped at 256 KiB, 4,096 object visits and 64 nesting levels. A function
  exceeding a cache admission limit can still evaluate under the existing graph
  limits; caching is not a new semantic refusal.
- Eviction drops cache ownership, not active paint handles. Concurrent in-flight
  preparations, caller-retained/evicted graphs, key temporaries, builder maps,
  source objects, decoder temporaries, cache-container capacity and allocator
  overhead are not a process-wide RSS bound. The subsequent
  `function_memory_accounting_implementation.md` adds live-consumer graph and
  decoder-output reservations for paint paths. The subsequent
  `render_owned_function_cache_implementation.md` adds worker-owned retention,
  aggregate handoff eviction and image function-use policy. Standalone cache
  limits remain per reader, not per application/process; broader allocation
  accounting remains open.
- Direct-object lookups still encode their bounded source bytes; large direct
  keys bypass reuse. Reference lookups do not read the referenced graph on a hit.
  This is source-level complexity improvement, not a measured latency claim.
- The reader assumes its backing document remains stable, as its existing xref
  and object-stream caches do. Mutated PDFs require a new reader; this change
  does not provide filesystem snapshot isolation against external file writes.
- Invalid preparations and failed tint evaluations are not cached as successful
  results. Cancellation is checked before warm function lookup and evaluation;
  it remains cooperative, not an immediate native-code interruption guarantee.
- Tint-output report counters/budgets still describe the small thread-local
  output cache, not the new reader-owned graph cache. They must not be presented
  as total renderer memory accounting. Public graph-cache metrics and standalone
  configuration are added by the subsequent render-owned increment. Negative
  caching, full retained colour-space graphs, broader diagnostics and aggregate
  non-shading work accounting remain follow-up work.

## Regression source and evidence

Twenty-five new regression functions are written and **unexecuted**:

- 13 graph-cache cases: direct/indirect reuse, cross-reader object-ID isolation,
  exact key distinctions, deep/wide/large key bypass, valid uncached execution,
  consumer-policy isolation, LRU and byte eviction, duplicate/parallel admission,
  lifetime, cancellation and poisoned-cache fallback.
- Five scalar-transfer cases: all 256 expected values, retained identity,
  direct/indirect Identity, cancellation, output arity, malformed functions,
  sample-dependent failure and cumulative LUT budget.
- Four tint-cache cases: weak lifetime, signed-zero/evicted identity, cross-reader
  reference values and cancellation before a warm output hit. The existing
  document-boundary test now checks graph identity instead of file hashes.
- One file-backed reader case checks that tint evaluation leaves the whole-file
  raw-byte cache unmaterialized.
- Two complete page-render entry cases reject multichannel soft-mask transfer
  and compare every pixel for indirect Identity versus omitted transfer.

Rustfmt parsing/formatting and `git diff --check` were the only executed checks.
No compiler/build/typecheck, tests, PDF workloads, renderer, benchmark, browser,
binding run, commit, push or deployment was run. The pixel assertions are test
source, not rendered evidence. Exact-revision executable qualification and all
remaining items in `universal_editor_roadmap_tracking.md` remain required.
