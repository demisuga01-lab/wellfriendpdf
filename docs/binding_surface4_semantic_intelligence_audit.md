# Semantic Intelligence Semantic Intelligence Audit

Artifact root: `target/semantic_intelligence-semantic-intelligence`.

Audit verdict: complete with bounded limits. No Semantic Intelligence-scope item remains
`blocked`.

Status matrix:

| Item | Status |
| --- | --- |
| existing StructTree access | implemented |
| existing MCID mapping | implemented |
| existing ParentTree parser | implemented_with_limits |
| broken ParentTree behavior | implemented_with_limits |
| ParentTree-only recovery path | implemented_with_limits |
| structure-node reconstruction | implemented_with_limits |
| orphan marked-content recovery | implemented |
| broken role map behavior | implemented_with_limits |
| reading-order interaction | implemented_with_limits |
| CJK baseline segmentation | implemented |
| dictionary-backed CJK segmentation | implemented_with_limits |
| user dictionary support | implemented_with_limits |
| dictionary license policy | implemented |
| segmentation confidence/provenance | implemented |
| search/RAG integration | implemented_with_limits |
| table/figure/caption interaction | implemented_with_limits |
| ML layout hook interface | implemented |
| local backend template | implemented |
| cloud backend template | implemented |
| privacy policy | implemented |
| backend result schema | implemented |
| deterministic merge policy | implemented |
| confidence threshold policy | implemented |
| binding/report exposure | implemented |
| validation gates | implemented_with_limits |

Required JSON artifacts:

- `semantic_intelligence-audit.json`
- `parenttree-recovery-matrix-semantic_intelligence.json`
- `parenttree-recovered-graph-semantic_intelligence.json`
- `parenttree-conflict-diagnostics-semantic_intelligence.json`
- `parenttree-recovery-provenance-semantic_intelligence.json`
- `cjk-dictionary-segmentation-matrix-semantic_intelligence.json`
- `cjk-dictionary-fixtures-semantic_intelligence.json`
- `cjk-token-provenance-semantic_intelligence.json`
- `cjk-search-rag-integration-semantic_intelligence.json`
- `cjk-dictionary-license-report-semantic_intelligence.json`
- `ml-layout-hook-schema-semantic_intelligence.json`
- `ml-layout-merge-policy-semantic_intelligence.json`
- `ml-layout-privacy-policy-semantic_intelligence.json`
- `ml-layout-fixture-results-semantic_intelligence.json`
- `local-layout-backend-template-semantic_intelligence.json`
- `cloud-layout-backend-template-semantic_intelligence.json`
- `layout-backend-availability-semantic_intelligence.json`
- `layout-backend-mock-results-semantic_intelligence.json`
- `layout-backend-privacy-audit-semantic_intelligence.json`
- `semantic-regression-results-semantic_intelligence.json`
- `parenttree-quality-results-semantic_intelligence.json`
- `cjk-segmentation-quality-results-semantic_intelligence.json`
- `ml-layout-merge-quality-results-semantic_intelligence.json`

Public report section:

`semantic_intelligence_semantic_intelligence_parenttree_cjk_ml_layout`

Public report exposure is additive across Rust, CLI, Python, C ABI, WASM, .NET,
Java Maven, and Java Gradle package smokes.
