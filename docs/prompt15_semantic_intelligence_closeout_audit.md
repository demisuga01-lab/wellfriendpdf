# Prompt 15 Semantic Intelligence Close-out Audit

Artifact root: `target/prompt15-semantic-closeout`

Allowed status vocabulary follows Combined Prompt 15. No in-scope row is
blocked.

| Item | Status | Evidence |
|---|---|---|
| TableFormer proposal schema | implemented | `table-proposal-schema-prompt15.json` |
| Table Transformer proposal schema | implemented | `table-proposal-schema-prompt15.json` |
| Table proposal region geometry | implemented | `table-proposal-schema-prompt15.json` |
| Table structure proposal merge | implemented | `table-proposal-merge-results-prompt15.json` |
| Table cell proposal merge | implemented | `table-proposal-merge-results-prompt15.json` |
| Deterministic table preservation | implemented | `table-proposal-merge-results-prompt15.json` |
| ML confidence thresholds | implemented | `table-proposal-merge-results-prompt15.json` |
| Conflicting proposal diagnostics | implemented | `table-proposal-conflict-diagnostics-prompt15.json` |
| Local table model adapter feasibility | unsupported_reported_no_runtime | `table-ml-backend-status-prompt15.json` |
| Cloud table model adapter feasibility | unsupported_reported_no_runtime | `table-ml-backend-status-prompt15.json` |
| Semantic binding exposure for Rust | implemented | `semantic-binding-exposure-matrix-prompt15.json` |
| Semantic binding exposure for CLI | implemented | `semantic-binding-exposure-matrix-prompt15.json` |
| Semantic binding exposure for Python | implemented | `semantic-binding-exposure-matrix-prompt15.json` |
| Semantic binding exposure for C ABI | implemented | `semantic-binding-exposure-matrix-prompt15.json` |
| Semantic binding exposure for WASM | implemented | `semantic-binding-exposure-matrix-prompt15.json` |
| Semantic binding exposure for .NET | implemented | `semantic-binding-exposure-matrix-prompt15.json` |
| Semantic binding exposure for Java Maven | implemented | `semantic-binding-exposure-matrix-prompt15.json` |
| Semantic binding exposure for Java Gradle | implemented | `semantic-binding-exposure-matrix-prompt15.json` |
| Advanced RAG chunk model | implemented | `rag-chunk-schema-prompt15.json` |
| Chunk provenance | implemented | `rag-provenance-quality-prompt15.json` |
| CJK token-aware chunking | implemented_with_limits | `rag-cjk-token-chunking-prompt15.json` |
| Table-aware chunking | implemented | `rag-table-chunking-prompt15.json` |
| Figure/caption-aware chunking | implemented_with_limits | `rag-chunking-modes-prompt15.json` |
| Heading/section-aware chunking | implemented | `rag-chunking-modes-prompt15.json` |
| Structure-tree-aware chunking | implemented | `rag-provenance-quality-prompt15.json` |
| Citation/reference-aware chunking where available | implemented_with_limits | `rag-provenance-quality-prompt15.json` |
| Redaction/security-aware chunking | implemented_with_limits | `rag-security-redaction-posture-prompt15.json` |
| Benchmark corpus | implemented_with_limits | `semantic-benchmark-manifest.json` |
| External reference availability | implemented_with_limits | `semantic-reference-availability-prompt15.json` |
| Semantic scorecard | implemented | `semantic-scorecard-prompt15.json` |
| Public report parity | implemented | `semantic-binding-parity-prompt15.json` |
| Validation gates | implemented | `validation-gates-prompt15.json` |

Counts: 24 `implemented`, 6 `implemented_with_limits`, 2
`unsupported_reported_no_runtime`, and 0 `blocked`.

The machine-readable source of truth is `prompt15-closeout-audit.json`.
