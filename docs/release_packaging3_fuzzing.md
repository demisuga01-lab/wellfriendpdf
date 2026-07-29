# text reflow Fuzzing

## Scope

text reflow extends the source editing provenance/operator-editing and editing transactions scene/transaction/font stack. It owns GeometricBlock and SemanticDocument routing and does not create a second parser, scene graph, font engine, semantic model, writer or binding-specific reflow implementation.

## Actual targets

`text_reflow_reflow` fuzzes bounded Unicode line breaking, hyphenation-policy
selection, preview planning, final-layout report generation, justification
policy, constraint input validation, and feature-matrix serialization.

`text_reflow_semantic_graph` fuzzes SemanticDocument analysis and reading-order
queries using a legal, repository-owned PDF while the input controls bounded
mode, direction, text, language, and review policy.

`text_reflow_transaction` fuzzes a provenance-resolved geometric rewrite,
output validation, and replay-verified undo on the same legal fixture. It
does not spawn processes or accept a fuzzed PDF parser input.

`text_reflow_reports` fuzzes request/report JSON parsing and the canonical
confidence-report API. These targets complement—not replace—the existing
parser and writer fuzzers.
## Limits

The targets are bounded to 2–4 KiB of fuzzer input and use repository-owned
legal PDFs for source-linked operations. They never run network or external
processes. Fuzz inventory/build/smoke evidence is recorded separately from
the release verdict; a successful smoke run does not establish full semantic
layout accuracy or package parity.
## Executed bounded smoke evidence

The final transferred snapshot has separate VPS build and smoke-stage logs
for `text_reflow_reflow`, `text_reflow_semantic_graph`, `text_reflow_transaction`, and
`text_reflow_reports`. The transaction target performs a real source rewrite,
output validation, and replay undo for each selected input, so its smoke
timeout is explicitly bounded at 20 seconds per input. The other text reflow
smoke targets use a five-second per-input limit. A timeout remains a fuzz
failure and must be reproduced before its bound is adjusted.

The transaction smoke first timed out at five seconds on its empty seed; a
direct reproduction completed below the explicit 20-second transaction bound,
after which the 64-run bounded smoke completed. The result directory records
the real exit code, duration, peak RSS, and log hash for every stage.

## Known limits

document subsystems owns full table/formula/OCR edit engines. document security owns final tagged-PDF/accessibility repair and forensic redaction closure. text reflow reports low-confidence semantic reconstruction and broad page-flow limitations instead of treating inference as exact fact.
