# Prompt 11 Reference Disagreement Policy

Renderer close-out compares Wellfriend against Poppler, PDFium, and MuPDF. A single
reference renderer is not treated as absolute truth when the references
disagree.

## Classification

- `all_reference_wellfriendpdf_pass`: Wellfriend is inside the reference cluster.
- `reference_disagreement_wellfriendpdf_inside_cluster`: references differ and Wellfriend is
  within a named accepted threshold of the cluster.
- `reference_disagreement_wellfriendpdf_outside_cluster`: references differ and Wellfriend is
  outside the cluster.
- `unsupported_reported_expected`: Wellfriend reports a precise unsupported feature
  and owner.
- `malformed_reference_failure`: the fixture is malformed or a reference cannot
  render it consistently.
- `wellfriendpdf_outlier`: references agree and Wellfriend is outside threshold.
- `unclassified_failure`: no owner or classification was assigned.

Prompt 11 completion requires zero Wellfriend outliers and zero unclassified
failures, or the final status must be partial.
