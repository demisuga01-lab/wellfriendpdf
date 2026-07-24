# Prompt 09 Renderer Failure Taxonomy

Prompt 09B uses the same audit vocabulary as Prompt 06B through Prompt 08B.

## Classifications

- `all_references_agree_wellfriendpdf_pass`: Poppler, PDFium, MuPDF, and Wellfriend render inside the configured visual threshold.
- `reference_disagreement_wellfriendpdf_inside_cluster`: reference engines disagree, and Wellfriend matches one reference or remains inside a documented policy cluster.
- `unsupported_reported_expected`: content is intentionally not rendered or generated, and the public report names the subtype/operator/dictionary context and owner.
- `malformed_reference_failure`: the fixture is malformed and at least one reference cannot render; the row must name the malformed construct.
- `wellfriendpdf_outlier_failure`: references agree and Wellfriend is outside threshold.
- `unclassified_failure`: no policy explains the mismatch.

## Prompt 09B Gate

Prompt 09B completion requires:

- 0 `wellfriendpdf_outlier_failure`
- 0 `unclassified_failure`
- no broad future bucket for annotation, OCG, progressive, tile, band, or cache behavior

The Prompt 09B summary artifact reports both counts:

- `target/prompt09-annotation-ocg-progressive-cache/reference-disagreement-summary-prompt09b.json`
