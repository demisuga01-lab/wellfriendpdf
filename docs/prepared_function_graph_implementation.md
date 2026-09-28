# Prepared PDF functions: source implementation, not qualification

This increment replaces repeated native function decoding/parsing with a shared
immutable evaluator. It does not complete the editor/rendering roadmap or
establish compatibility, speed, memory use or superiority over another product.

## Implemented paths

- `PreparedFunction` retains typed Domain/Range metadata, packed sampled data,
  exponential coefficients, stitching edges and compiled calculator branches.
  It retains no PDF reader or source object graph. Inputs and every child/parent
  output still receive their own declared clipping; stitching keeps exact
  half-open boundary ownership and the supported empty terminal interval.
- Both existing native evaluator entry points now use this graph. The single
  function API does not silently accept component arrays. The array API requires
  equally dimensioned scalar children. Old sampled/calculator dispatch bodies
  were removed rather than left as alternative active interpreters.
- Type 1, axial, radial, triangle/lattice and Coons/tensor patch shading paths
  prepare once before their paint loops. Existing exact-parameter colour caches
  remain separate. Repeated indirect children and reference aliases share one
  immutable node/decoded table per preparation, keyed by object and generation.
  There is no global object-number cache that could cross document revisions.
- Alias resolution during graph construction is iterative, with cycle checks.
  Cached subgraph height is checked at each use so memoization cannot bypass the
  nesting limit. The graph has no cycles; its data is released after all paint
  and subsequent reader-cache ownership has ended.
- Calculator programs compile once and share immutable code. The subsequent
  `calculator_semantics_implementation.md` increment replaces operand procedures
  with syntactic branches, preserves integer/real/boolean types, checks exact
  output arity and implements numeric/operator edge handling. The editing-side
  validator uses the same compiler. Integer division overflow fails closed;
  minimum-integer remainder by -1 returns zero without native overflow.

## Explicit limits and work accounting

- At most 4,096 preparation visits, 16 nested stitching edges, 64 reference
  aliases per chain, and 16 component functions. References/aliases consume visits
  too; these are not promises of accepting every graph with 4,096 unique nodes.
- Prepared node/payload storage has a 64 MiB budget. Numeric/sample vector
  capacities are charged; calculator instruction/Arc storage is conservatively
  charged. Shared nodes are charged once. Builder maps, source/reader objects,
  temporary decoder/parser allocations and allocator overhead are **not** a
  total-RSS bound. `function_memory_accounting_implementation.md` subsequently
  adds active-consumer graph and decoder-output reservations, not all allocations.
- Existing Type 0 limits remain: eight dimensions, 4,194,304 sample values,
  16 MiB decoded data. Decode receives the smaller remaining graph/stream limit;
  short streams are rejected during preparation, before painting samples.
- Type 4 uses a 1 MiB decoded-program cap, 16,384 tokens, 1,024 stack entries and
  64 nested procedures. Programs exceeding these limits fail closed, including
  when the program is supplied through an editing resource request.
- One evaluation shares 8,388,608 work units across component roots, stitching
  children, sampled tap/channel operations, calculator operations, stack checks,
  copy and roll. Recursive calls cannot reset the allowance. Cancellation is
  polled during preparation and evaluation; this is cooperative, not an
  interruption guarantee inside a decoder.
- Shading consumers supply the smaller remaining render allowance and debit
  actual function work to their existing cumulative render budget. That budget
  now counts function work as well as pixel/mesh work. Cache hits do not repeat
  function work. Concurrent callers can perform bounded in-flight work before
  the atomic debit detects exhaustion; this is not a wall-clock deadline.

## Regression source and checks

Eighteen new regression functions are present and **unexecuted**:

- Sixteen graph/interpreter cases cover cold-versus-retained API parity, fixed
  numeric expectations, reader-independent lifetime, alias sharing/one-time
  accounting, clipping and terminal boundaries, extreme Decode, two-input
  component arrays, compiled procedures, nested/shared budgets, memory/visit/
  cycle/depth guards, malformed/oversized programs, invalid arrays/short samples,
  cancellation and signed integer division/remainder overflow.
- One raw/compiled render-entry case covers calculator component arrays in
  function, axial and radial shadings with expected output pixels.
- One mesh case covers prepared calculator conversion and cumulative work debit.

The parity assertions exercise two entry paths to the same implementation; they
are not an independent numerical oracle. Prior sampled-function fixed values,
packed-reader tests and render assertions remain relevant and also unexecuted.
Only rustfmt parsing/formatting and `git diff --check` were performed. No compiler,
build, typecheck, test, PDF workload, rendering, benchmark, browser/binding run,
commit, push or deployment was performed.

## Remaining implementation and qualification

- The subsequent `retained_colour_function_implementation.md` increment adds
  reader-owned graph reuse for tint/transfer/native APIs and cross-paint calls,
  weak tint-output identities and cumulative scalar-transfer LUT work. Full
  colour-space retention, broader allocation accounting, public cache metrics,
  broader failure diagnostics and aggregate accounting outside these paths remain.
- Typed integer/real/boolean handling and calculator branch/operator semantics
  are now implemented in the subsequent calculator increment. Numerical limits,
  independent interpreter/corpus parity and public diagnostics still require
  qualification; the implementation is not all-input mathematical proof.
- The existing decoder, ICC, colour conversion, export, transparency and wider
  font/layout/tag/history/scan/UI boundaries remain. This increment changes
  native evaluation, not every vector-export representation.
- Exact-revision compilation, current regressions, independent extraction/render
  comparisons, real PDFs, binding execution and memory/cancellation/performance
  qualification remain required after execution is authorized.
