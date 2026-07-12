# Multi-run text-range editing

`MultiRunTextRangeRequest` selects by logical Unicode scalar offsets over the page-local, provenance-bearing sequence of decoded PDF text-showing operands. `analyze_multi_run_text_range` exposes the stable source span IDs, stream/object references, operators, `TJ` element indices, byte ranges, font resources, and bidi logical-to-visual runs before mutation.

The supported true-edit subset spans contiguous whole string operands in one page content stream, including `Tj`, string elements in `TJ`, quote, and double-quote operators. Replacement removes every selected source operand from reachable content and writes deterministic Type0 shaped text through the existing paragraph reflow writer. A zero-width boundary is a bounded insertion; an empty replacement is deletion.

Partial-token, cross-stream/cross-page, malformed-CMap, missing-provenance, and arbitrary Type3 selections fail closed. `preserve_per_segment` is reported as a limit because the current generated Type0 output has a normalized style run; it never silently pretends to retain source style segmentation. Logical/visual RTL mapping comes from bidi/shaping provenance, not x-coordinate sorting. Incremental prefix preservation is structural evidence, not a cryptographic-signature-validity assertion.

Focused Prompt 20B fixtures cover multiple `Tj` operands, `Tj` followed by
`TJ`, quote and double-quote operators, font/size/color changes, mixed
RTL/LTR analysis, vertical replacement serialization, insertion, deletion,
unsupported partial-token ranges, undo/redo, and branch redo clearing. The
CLI artifact bundle records replacement, insertion, deletion, reopen/extract,
reachable-stream, determinism, and signature-impact JSON.
