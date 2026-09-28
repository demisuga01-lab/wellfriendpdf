# Function memory accounting: source implementation, not qualification

This increment connects prepared PDF functions to active renderer memory limits.
It does not establish total-process memory bounds, measured performance, complete
render-contract coverage or completion of the universal editing roadmap.

## Implemented

- `FunctionResources` carries a consumer's graph-byte limit, decoded-function
  stream limit and optional shared `DecodeMemoryBudget`. `FunctionLease` owns
  the graph and a lifetime-bound reservation. A reader cache hit must satisfy the
  current limits and acquire a current-use reservation; it cannot inherit the
  looser limits under which the graph was originally prepared.
- Graph preparation grows its retained-storage reservation as nodes, numeric
  vectors, samples, compiled instructions and wrapper/root storage are charged.
  Shared nodes are charged once within a graph. Warm and cold paths use the same
  retained-storage model. The graph also records the largest decoded source
  stream, so a cache hit cannot bypass a stricter stream limit.
- Before decoding a function stream, the builder atomically reserves an output
  window between the minimum required bytes and the permitted maximum still
  available. The decoder receives that window's limit. After decoding, the
  window is resized to the buffer capacity. Sample storage transfers into the
  graph reservation without a release/reacquire race or double charge.
  Calculator source storage has a temporary window while compiled code receives
  its separate retained charge; the source window ends after compilation.
- New nonblocking reservation operations support zero-sized initial leases,
  checked growth/shrink and same-budget transfer. Failed growth leaves counters
  unchanged. Existing reservations release on ordinary return, errors and unwind.
  These paths never wait while a nested paint may hold sibling budget tokens.
- All seven shading families use leased function graphs. Preflight and paint
  both receive the function stream limit. During paint, graph reservations share
  the scheduler with scratch surfaces and mesh storage. Standalone shading calls
  create a local shared budget when the caller does not provide one.
- Named paint colours and shading colours use the same limits for tint functions,
  including nested Separation/DeviceN alternates, Indexed bases and ICC alternate
  conversion. A tint-output cache hit still obtains a live graph reservation.
- Soft-mask scalar transfer lookup tables and alpha-backdrop evaluation receive
  the active render state's temporary and decoded-byte limits. The graph's
  reservation remains live throughout LUT construction.
- Native evaluator/helper APIs retain their existing signatures and standalone
  bounded behavior. The new policy/lease types are crate-private; no external
  binding ABI or saved PDF schema is changed.

## What the accounting does and does not mean

- The reservation counts a defined retained-storage model, not allocator RSS.
  Some small metadata vectors and compiler structures are constructed before
  their retained charge is known. Parser/resolver objects, object-stream caches,
  exact cache-key buffers, compiler intermediates, native codec intermediates,
  evaluator stack/output temporaries and allocator overhead are not all covered
  by these tokens. Decoder output limits are not a proof about every codec's
  internal allocations. Hard process isolation and broader accounting remain.
- The 64 MiB per-graph and existing per-format limits remain upper caps. Consumer
  limits can lower them, not raise them. Nonblocking budget contention can reject
  an operation; this is not a queued scheduler or retry policy.
- Separate simultaneous consumers each reserve their own use, even if they share
  the same graph allocation. This is conservative accounting, not deduplicated
  physical-memory attribution. Eviction or reader closure does not release a
  live consumer's lease. Reader cache ownership remains separately bounded.
- The subsequent `render_owned_function_cache_implementation.md` increment adds
  caller-owned render retention, aggregate handoff eviction, reader configuration
  and graph-cache telemetry. Standalone retention remains separately bounded;
  aggregate admission-time and complete process-memory accounting remain open.
- That increment also propagates explicit function policy through image tint,
  Indexed and ICC-alternate conversion. It does not account for every image
  output buffer, CMM/codec allocation or standalone colour-report/native helper.
- Existing public failures can still collapse resource and malformed-function
  rejection into the same higher-level message. Richer typed diagnostics remain.
  Cancellation is cooperative, not forced interruption of native code.

## Regression source and verification boundary

Twenty new regression functions are written and **not executed**:

- Five reservation cases cover checked growth/shrink, decoder windows with live
  siblings, same-budget transfer, mismatched owners, cancellation and unwind.
- Ten function-resource cases cover cold/warm exact retained charges, stricter
  graph/stream policies, shared and independent budgets, eviction/reader lifetime,
  failed partial preparation, calculator temporary storage, cancellation, nested
  Indexed/ICC tint conversion and complete scalar-transfer LUT lifetime.
- Three shading integration cases check surface/function overlap, unchanged
  destination pixels after rejection, exact-budget pixel output, live sibling
  reservations and a warmed tint cache while the shading graph remains active.
- Two renderer-state cases check soft-mask LUT/backdrop limits with cold/warm
  graphs and overlap with existing offscreen storage.

Only rustfmt parsing/formatting and `git diff --check` ran. No compiler, build,
typecheck, tests, PDF workloads, rendered output, benchmark, binding/browser QA,
commit, push or deployment ran. These assertions are source, not passing results.
The wider roadmap and all exact-revision executable qualification remain open.
