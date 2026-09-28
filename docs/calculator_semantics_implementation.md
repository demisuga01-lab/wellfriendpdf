# Typed calculator functions: source implementation, not qualification

The native Type 4 evaluator now uses a typed compiler and bounded execution
engine in `crates/engine/src/render/calculator_function.rs`. The prior
floating-point-only interpreter was removed. This is a renderer/editing-resource
increment, not completion of the full editor roadmap.

## Contract and research basis

PDF calculator operands use PDF decimal syntax, not PostScript radix/exponent
syntax. Conditional braces delimit expressions rather than stack-manipulable
procedure objects. [Adobe PDF reference, sections 3.2.2 and 3.9.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf)

Final stack size must equal the Range output count and every result must be
numeric. Wider intermediate real representations are permitted.
[Adobe PDF reference 1.6, section 3.9.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf)

Operator behavior follows the referenced PostScript definitions: integer
arithmetic promotes overflowing results where specified; integer-only operators
do not silently truncate real operands; boolean and bitwise overloads remain
distinct; right shifts fill with zeros; round breaks ties toward the larger
integer; undefined arithmetic is an error. The implementation uses signed
32-bit integers and finite binary64 real values, an explicit numeric profile
rather than a claim of matching every interpreter's precision.
[Adobe PostScript Language Reference, operator details and implementation limits](https://www.adobe.com/jp/print/postscript/pdfs/PLRM.pdf)

## Implemented and connected

- The scanner recognizes decimal integer/real literals, PDF whitespace including
  NUL, boolean literals and only the calculator operator vocabulary. Comments
  can contain arbitrary bytes. Malformed numeric tokens, general PostScript
  numbers, names, arrays and unsupported tokens fail preparation.
- The compiler produces typed literals, opcode enums and immutable conditional
  branches. It rejects unbound if/ifelse, free blocks and attempts to manipulate
  blocks with operand-stack operators. Only the selected branch executes.
- Numeric operations retain integer/real identity. Explicit conversion checks
  range; integer division rejects overflow; remainder uses a wider intermediate
  to handle the minimum integer divided by -1 without a Rust panic. Round avoids
  both negative-tie errors and large-real `floor(x + .5)` errors. Trigonometric
  input reduction avoids overflow from converting huge finite degrees directly
  to radians. Nonfinite arithmetic results fail closed.
- Equality handles boolean values, mixed numeric types and unequal types.
  Boolean/bitwise operators and stack indices enforce their operand types.
- The exact final numeric stack is validated after the chosen branch. Old code
  silently selected the final numeric suffix, allowing leftover inputs to pass.
- The existing single/component-array APIs and retained shading graphs use this
  evaluator. The editing resource validator uses the same compiler. Function
  samples enter as real values; integer-only work requires explicit conversion.
  Syntax validation alone is not a proof that every possible input succeeds.
- Existing graph limits, 1 MiB program bytes, 16,384 tokens, 1,024 stack entries,
  64 nested branches, shared execution accounting and cumulative shading debits
  remain active. Lexical scanning and execution poll cancellation. Calculator
  failures are internally typed; public native function APIs still return an
  empty result for failure and higher-level callers supply diagnostics.

## Regression source and checks

Twenty-one new regression functions remain **unexecuted**:

- Twenty calculator/tint cases cover decimal syntax, binary comments, numeric
  types, promotion, signed division/remainder, rounding, conversions, every
  boolean overload, bitwise shifts, equality/relations, undefined arithmetic,
  trigonometry, stack behavior, branch grammar/execution, exact output counts,
  real input conventions, limits, cancellation, editing-side validation and
  Separation tint integration.
- One raw/compiled shading case asserts pixel output from a program combining
  negative rounding, logical right shift and boolean conjunction.

Earlier tests were adapted to compiled branches and the 32-bit numeric profile.
A constant-output fixture now explicitly consumes its input. Two limit tests
now supply the required input instead of failing before reaching their stated
token/stack conditions. Assertions were not executed or weakened to claim a pass.

Only rustfmt parsing/formatting, source inspection and whitespace checks were
performed. No compiler, build, typecheck, test runner, PDF workload, rendering,
benchmark, browser/binding run, commit, push or deployment was performed.

## Still required

- Numerical/corpus comparisons against independent implementations, including
  extreme real values, underflow, rounding boundaries and branch-sensitive
  output shape. This is not an interval-certified or bit-identical interpreter.
- Richer public failure diagnostics, full retained colour-space graphs and
  aggregate memory reservations. Reader-owned tint/transfer and cross-paint
  function caches are added in `retained_colour_function_implementation.md`.
- The remaining font/layout/tag/history/scan/UI, colour/codec/transparency/export
  implementation and all exact-revision executable qualification gates. No
  universal-editing, production-readiness or better-than-Acrobat claim is made.
