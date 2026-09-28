# PDF Function Evaluator

PDF functions are used by shadings, Separation/DeviceN tint transforms, transfer
functions, and future prepress validation. Wellfriend centralizes them in
`crates/engine/src/render/function.rs`.

## Supported Types

| type | status | use |
| --- | --- | --- |
| Type 0 sampled | Bounded linear/cubic source implementation; current qualification pending | sampled shadings and tint transforms |
| Type 2 exponential | Source implementation; current qualification pending | linear/exponential interpolation |
| Type 3 stitching | Bounded source implementation; current qualification pending | piecewise function composition |
| Type 4 PostScript calculator | Typed decimal/branch/operator source implementation; current qualification pending | arithmetic tint transforms and function shadings |

## Type 0 Limits

- `MAX_TYPE0_SAMPLE_VALUES = 4_194_304`
- `MAX_TYPE0_INTERPOLATION_DIMENSIONS = 8`
- Order 1 and tensor cubic Order 3; axes of Size < 4 fall back to linear
- sample-value cap also bounds tap/channel reads per evaluation; cancellation is
  polled every 256 reads; decoded sampled streams are capped at 16 MiB
- supported `BitsPerSample`: 1, 2, 4, 8, 12, 16, 24, 32
- decoded sample stream goes through the central stream decode path
- malformed functions return an empty result rather than panicking

Decode Scheduler adds a checked sample-count cap so hostile `/Size` arrays cannot
multiply into large allocations or unbounded bit reads.

## Type 4 Calculator Subset

Supported operators include arithmetic, numeric conversion, comparisons,
boolean operations, stack manipulation, and `if`/`ifelse` expressions. The
interpreter intentionally has no file, network, system, dictionary, loop, or
VM-level PostScript behavior.

Decode Scheduler limits:

- `MAX_TYPE4_TOKENS = 16_384`
- `MAX_TYPE4_STACK = 1_024`
- calculator `MAX_DEPTH = 64`
- decoded program cap: 1 MiB; one complete outer procedure is required
- compiled immutable branches and shared graph-evaluation work budget
- non-finite numeric values are rejected

Signed i32 integers and finite binary64 reals retain distinct types; boolean
overloads, integer-only operations, error handling and exact final output count
are implemented in source. Function inputs are real-valued samples. Decimal
syntax and conditional expressions follow PDF, not general PostScript; floating
precision and interpreter/corpus parity remain unqualified. See
`calculator_semantics_implementation.md`.

Unsupported or malformed calculator programs return an empty result and can be
reported through the higher-level color diagnostics when they appear in
Separation/DeviceN or color-space reports.

## Tests

Focused tests cover:

- Type 0 exact sample points, interpolation, 16-bit samples, and sample cap;
- Type 4 token cap and stack cap;
- arithmetic, conditionals, stack operations, and tint-transform-style programs;
- DeviceN color-space resolution through a Type 4 tint transform.

Current sampled/prepared-function additions are unexecuted. All seven native
shading families retain prepared function graphs for their paint loops. The
subsequent `retained_colour_function_implementation.md` increment adds bounded
reader-owned reuse for native/tint/transfer/shading calls and cumulatively bounded
scalar transfer LUTs. Cache admission limits do not reject valid uncached graphs. See
`function_memory_accounting_implementation.md` for live consumer reservations,
warm/cold resource-policy checks and the explicit accounting/coverage boundaries;
`sampled_function_interpolation_implementation.md` for the numerical convention,
and `prepared_function_graph_implementation.md` for integration, explicit graph/
program/work limits, regression-source coverage and remaining qualification.
