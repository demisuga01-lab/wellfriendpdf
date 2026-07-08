# Prompt 11 Reference Disagreement Policy

Renderer close-out compares Oxide against Poppler, PDFium, and MuPDF. A single
reference renderer is not treated as absolute truth when the references
disagree.

## Classification

- `all_reference_oxide_pass`: Oxide is inside the reference cluster.
- `reference_disagreement_oxide_inside_cluster`: references differ and Oxide is
  within a named accepted threshold of the cluster.
- `reference_disagreement_oxide_outside_cluster`: references differ and Oxide is
  outside the cluster.
- `unsupported_reported_expected`: Oxide reports a precise unsupported feature
  and owner.
- `malformed_reference_failure`: the fixture is malformed or a reference cannot
  render it consistently.
- `oxide_outlier`: references agree and Oxide is outside threshold.
- `unclassified_failure`: no owner or classification was assigned.

Prompt 11 completion requires zero Oxide outliers and zero unclassified
failures, or the final status must be partial.
