# Prompt 14 Semantic Intelligence Audit

Artifact root: `target/prompt14-semantic-intelligence`.

Audit verdict: complete with bounded limits. No Prompt 14-scope item remains
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

- `prompt14-audit.json`
- `parenttree-recovery-matrix-prompt14.json`
- `parenttree-recovered-graph-prompt14.json`
- `parenttree-conflict-diagnostics-prompt14.json`
- `parenttree-recovery-provenance-prompt14.json`
- `cjk-dictionary-segmentation-matrix-prompt14.json`
- `cjk-dictionary-fixtures-prompt14.json`
- `cjk-token-provenance-prompt14.json`
- `cjk-search-rag-integration-prompt14.json`
- `cjk-dictionary-license-report-prompt14.json`
- `ml-layout-hook-schema-prompt14.json`
- `ml-layout-merge-policy-prompt14.json`
- `ml-layout-privacy-policy-prompt14.json`
- `ml-layout-fixture-results-prompt14.json`
- `local-layout-backend-template-prompt14.json`
- `cloud-layout-backend-template-prompt14.json`
- `layout-backend-availability-prompt14.json`
- `layout-backend-mock-results-prompt14.json`
- `layout-backend-privacy-audit-prompt14.json`
- `semantic-regression-results-prompt14.json`
- `parenttree-quality-results-prompt14.json`
- `cjk-segmentation-quality-results-prompt14.json`
- `ml-layout-merge-quality-results-prompt14.json`

Public report section:

`prompt14_semantic_intelligence_parenttree_cjk_ml_layout`

Public report exposure is additive across Rust, CLI, Python, C ABI, WASM, .NET,
Java Maven, and Java Gradle package smokes.
