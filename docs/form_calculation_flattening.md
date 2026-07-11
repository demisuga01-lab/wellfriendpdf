# Form Calculation Flattening

`flatten_calculated_values_then_remove` is an opt-in deterministic calculation
pass. It is not Acrobat JavaScript.

Supported inputs are pure scalar arithmetic, string concatenation, comparisons,
Boolean operators, ternary conditionals, static `getField("name").value` reads,
bounded field writes, and static `AFSimple_Calculate` `SUM`, `AVG`, `PRD`,
`MIN`, or `MAX` lists. Numeric field strings are coerced to finite numbers.

Rejected inputs include loops, functions, `eval`, dynamic property traversal,
network/filesystem/process/UI/clipboard/timer APIs, division by zero, non-finite
values, dependency cycles, missing/ambiguous fields, excessive instructions,
and excessive value/mutation sizes.

The evaluator respects AcroForm `/CO` where present, records original and new
values, writes fields through `PdfEditor` (including the existing appearance
regeneration path), removes actions, and rescans the saved PDF. Unsupported
scripts remain explicit result rows; no arbitrary fallback execution occurs.
