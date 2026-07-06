# Prompt 08 Renderer Audit

Prompt 08 audit script:

- `scripts/prompt08_text_shading_patterns_audit.py`
- `scripts/prompt08b_type3_cid_tensor_audit.py`

The script generates 26 deterministic PDF fixtures under
`target/prompt08-text-shading-patterns/corpus/`, copies the target-local
Prompt 06B/07B reference tool manifest, and renders each page through Oxide,
Poppler, PDFium, and MuPDF.

Current audit summary:

- Fixtures: 26.
- Pairwise comparisons: 156.
- `all_references_agree_oxide_passes`: 19.
- `references_disagree_oxide_within_cluster`: 3.
- `unsupported_reported_expected`: 3.
- `malformed_reference_failure`: 1.
- Oxide outlier failures: 0.
- Prompt 08 cluster-tolerance acceptances: 2.

Primary artifacts:

- `target/prompt08-text-shading-patterns/corpus-manifest.json`
- `target/prompt08-text-shading-patterns/reference-tool-manifest.json`
- `target/prompt08-text-shading-patterns/multi-reference-render-results.json`
- `target/prompt08-text-shading-patterns/visual-diff-metrics.json`
- `target/prompt08-text-shading-patterns/reference-disagreement-summary.json`
- `target/prompt08-text-shading-patterns/html-report/index.html`

The two cluster-tolerance acceptances are recorded per page in
`multi-reference-render-results.json`; the raw Prompt 06B classification is
preserved beside the Prompt 08 classification.

Prompt 08B closure summary:

- Fixtures: 21.
- Pairwise comparisons: 126.
- `all_references_agree_oxide_passes`: 11.
- `unsupported_reported_expected`: 10.
- Oxide outlier failures: 0.
- Unclassified failures: 0.

Prompt 08B artifacts:

- `target/prompt08b-type3-cid-tensor/prompt08b-corpus-manifest.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-reference-tool-manifest.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-render-results.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-diff-metrics.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-reference-disagreement-summary.json`
- `target/prompt08b-type3-cid-tensor/prompt08b-html-report/index.html`

The Type3 rows are expected reference-cluster limitations: Oxide clips from the
Type3 charproc path, while Poppler, PDFium, and MuPDF render the generated Type3
`Tr` fixtures without applying the Type3 clip. Those rows are not bbox fallback.
