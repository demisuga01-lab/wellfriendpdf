# Advanced Rendering Renderer Audit

Advanced Rendering audit script:

- `scripts/advanced_rendering_text_shading_patterns_audit.py`
- `scripts/type3_cid_rendering_type3_cid_tensor_audit.py`

The script generates 26 deterministic PDF fixtures under
`target/advanced_rendering-text-shading-patterns/corpus/`, copies the target-local
Reference Renderer/07B reference tool manifest, and renders each page through Wellfriend,
Poppler, PDFium, and MuPDF.

Current audit summary:

- Fixtures: 26.
- Pairwise comparisons: 156.
- `all_references_agree_wellfriendpdf_passes`: 19.
- `references_disagree_wellfriendpdf_within_cluster`: 3.
- `unsupported_reported_expected`: 3.
- `malformed_reference_failure`: 1.
- Wellfriend outlier failures: 0.
- Advanced Rendering cluster-tolerance acceptances: 2.

Primary artifacts:

- `target/advanced_rendering-text-shading-patterns/corpus-manifest.json`
- `target/advanced_rendering-text-shading-patterns/reference-tool-manifest.json`
- `target/advanced_rendering-text-shading-patterns/multi-reference-render-results.json`
- `target/advanced_rendering-text-shading-patterns/visual-diff-metrics.json`
- `target/advanced_rendering-text-shading-patterns/reference-disagreement-summary.json`
- `target/advanced_rendering-text-shading-patterns/html-report/index.html`

The two cluster-tolerance acceptances are recorded per page in
`multi-reference-render-results.json`; the raw Reference Renderer classification is
preserved beside the Advanced Rendering classification.

Type3 CID Rendering closure summary:

- Fixtures: 21.
- Pairwise comparisons: 126.
- `all_references_agree_wellfriendpdf_passes`: 11.
- `unsupported_reported_expected`: 10.
- Wellfriend outlier failures: 0.
- Unclassified failures: 0.

Type3 CID Rendering artifacts:

- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-corpus-manifest.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-reference-tool-manifest.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-render-results.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-diff-metrics.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-reference-disagreement-summary.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-html-report/index.html`

The Type3 rows are expected reference-cluster limitations: Wellfriend clips from the
Type3 charproc path, while Poppler, PDFium, and MuPDF render the generated Type3
`Tr` fixtures without applying the Type3 clip. Those rows are not bbox fallback.
