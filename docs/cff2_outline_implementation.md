# CFF2 outlining prerequisite — source implementation, not qualification

This increment continues the font/rendering part of the full editing roadmap.
It does **not** finish variable-font instantiation, universal editing, or the
roadmap. No compiler, build, test, PDF, rendering or benchmark workload was run.
The work remains uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`.

## Source changes

`fonts/cff2_program.rs` retains one immutable source program and parses checked
INDEX ranges, per-glyph FontDICTSelect formats 0/3/4, Private DICT local
subroutines, inherited `vsindex` and the variation region store. Global subroutine
calls retain the calling glyph's local dictionary. Static CFF2 does not require
a VariationStore. An explicit variation index still requires a valid store.
Shared private dictionaries are retained once, including in cache accounting.

The existing ttf-parser 0.21 CFF2 implementation was inspected in source: its
first-local-dictionary selection, omitted Private DICT variation index and
64-region scalar array motivated this gateway. These are source observations,
not runtime reproductions. Updating to its inspected 0.25 implementation alone
would not close those same outlining cases.

`fonts/cff2_charstring.rs` evaluates blends at the face's normalized coordinates,
expands subroutines with a shared glyph operand stack, consumes hint masks as
data, normalizes large path operand groups and retains flex curves. Unknown
operators consume the operand stack rather than executing CFF1 return/endchar
semantics. BCD real parsing handles zero spellings and checks spelling/padding.
The evaluator supports more than 64 referenced regions subject to operand and
work limits; result-major deltas and stack prefixes are retained.

`fonts/sfnt_outline.rs` is the shared gateway for CFF2 outline and bounds work.
The source changes connect it to raster/vector glyph extraction, color-glyph
base outlines, strict glyph metrics, shaped coverage, horizontal/vertical
measurement and generated edit geometry. FontMatrix/unitsPerEm and maxp/glyph
counts must agree; competing outline tables are refused. Non-CFF2 routes
continue to use the existing decoder.
Strict outline helpers distinguish a decoder error from a legitimate blank
glyph. Legacy compatibility helpers retain their older optional-outline API;
they do not expose a detailed error receipt.

## Bounds and numerical contract

- Source table: 32 MiB; aggregate INDEX entries: one million; individual DICT:
  1 MiB; aggregate parsed DICT bytes: 16 MiB; axes: 64; variation references:
  one million. The extended VariationStore length sentinel is supported, with
  checked internal offsets rather than an unchecked read-through.
- Glyph evaluation: 513 operands, ten subroutine nesting levels, 96 hints,
  one million evaluation/emission steps and a 4 MiB normalized glyph budget.
  Cancellation is polled during parsing, hashing, expansion and interpolation,
  and after final path decoding before successful publication.
- A successful glyph is represented transiently as a two-glyph CFF1 program
  solely for the existing path decoder. It is **never embedded or saved**.
  Blended operands are rounded to Type 2 16.16 precision where non-integral.
  Out-of-range operands and bounding boxes are errors, not silent truncation.
- Hint masks are consumed, not raster hints applied. Flexes remain cubic curves;
  device-pixel flex flattening is not introduced. This is an unhinted outline
  path, not a claim of exact small-size raster equivalence.
- A SHA-256 keyed immutable cache has eight entries and an estimated 64 MiB
  retention bound. Parsing happens outside its lock; failed/cancelled parses
  are not published. Live caller references may survive eviction. This cache
  bound is not an RSS or whole-renderer memory guarantee. Retained measurement
  gateways reuse the parsed program; free per-glyph helpers still hash their
  supplied CFF2 table to establish identity.

## Regression source and checks

`fonts/cff2_program_tests.rs` adds 25 **unexecuted** regression functions covering:

- static and multi-dictionary outlines, every supported FDSelect representation,
  global-to-local calls and shared Private DICT ownership;
- inherited/overridden variation indices, normalized fvar coordinates,
  more than 64 regions, extended stores and multi-result blend prefixes;
- actual control/end points for alternating/optional curve operands and all
  flex variants, long path stacks, and binary hint masks;
- malformed offsets/ranges, missing stores, invalid glyphs/axes, recursion,
  operand and expansion limits, metadata mismatches and cancelled publication;
- strict rendering/coverage integration, legitimate blank glyphs, unknown
  operators and BCD edge cases. Cache tests use an isolated cache so concurrent
  regression execution cannot evict the entries under assertion.

Rustfmt formatting/parser checks and Git whitespace checks passed. These checks
do not establish type correctness, test results, visual quality, binding
execution, performance or interoperability.

## Still open

Follow-up: `font_variation_core_implementation.md` replaces the private CFF2
region parser with shared source-bound variation evaluation and adds a null
item-data regression. It also adds an internal layout-feature freezer, not a
complete font instancer.

Subsequent source work in `cff2_static_instance_implementation.md` adds a separate
public CFF2-to-CFF1 transaction, composing supported metrics, layout, naming and
hint owners with explicit permission decisions. Ordinary face extraction still
refuses CFF2. The outline-only projection described here remains a rendering
prerequisite, not the persisted asset. No external instancer was added. Contour
overlap handling and broader font semantics remain implementation gaps.

Independent real-font/raster comparisons, cancellation/memory measurements,
Rust and binding builds, all regressions and the remaining requirements in
`universal_editor_roadmap_tracking.md` remain pending. This increment supplies
one prerequisite, not evidence for an Adobe-superiority or universal-editing
claim.

## Primary sources

The [OpenType CFF2 specification](https://learn.microsoft.com/en-us/typography/opentype/spec/cff2)
defines the dictionaries, glyph ownership, variation operators, stack and
forward-compatible unknown-operator handling. The large-store compatibility
case follows the [FontTools CFF2 VariationStore reader](https://github.com/fonttools/fonttools/blob/main/Lib/fontTools/cffLib/__init__.py).
The original increment used ttf-parser 0.21.1 for the final CFF1 path decoder.
The subsequent coordinate/metric increment pins 0.25.1, also used by shaping;
see `font_instance_metrics_implementation.md`. The CFF2 source gateway remains
necessary after this dependency alignment. No external conversion executable
is invoked.
