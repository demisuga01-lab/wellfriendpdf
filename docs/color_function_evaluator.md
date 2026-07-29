# PDF Function Evaluator

PDF functions are used by shadings, Separation/DeviceN tint transforms, transfer
functions, and future prepress validation. Wellfriend centralizes them in
`crates/engine/src/render/function.rs`.

## Supported Types

| type | status | use |
| --- | --- | --- |
| Type 0 sampled | DONE WITH BOUNDED LIMIT | sampled shadings and tint transforms |
| Type 2 exponential | DONE | linear/exponential interpolation |
| Type 3 stitching | DONE | piecewise function composition |
| Type 4 PostScript calculator | DONE WITH BOUNDED LIMIT | arithmetic tint transforms and function shadings |

## Type 0 Limits

- `MAX_TYPE0_SAMPLE_VALUES = 4_194_304`
- supported `BitsPerSample`: 1, 2, 4, 8, 12, 16, 24, 32
- decoded sample stream goes through the central stream decode path
- malformed functions return an empty result rather than panicking

Decode Scheduler adds a checked sample-count cap so hostile `/Size` arrays cannot
multiply into large allocations or unbounded bit reads.

## Type 4 Calculator Subset

Supported operators include arithmetic, numeric conversion, comparisons,
boolean operations, stack manipulation, and `if`/`ifelse` procedures. The
interpreter intentionally has no file, network, system, dictionary, loop, or
VM-level PostScript behavior.

Decode Scheduler limits:

- `MAX_TYPE4_TOKENS = 16_384`
- `MAX_TYPE4_STACK = 1_024`
- `PS_MAX_DEPTH = 64`
- non-finite numeric values are rejected

Unsupported or malformed calculator programs return an empty result and can be
reported through the higher-level color diagnostics when they appear in
Separation/DeviceN or color-space reports.

## Tests

Focused tests cover:

- Type 0 exact sample points, interpolation, 16-bit samples, and sample cap;
- Type 4 token cap and stack cap;
- arithmetic, conditionals, stack operations, and tint-transform-style programs;
- DeviceN color-space resolution through a Type 4 tint transform.
