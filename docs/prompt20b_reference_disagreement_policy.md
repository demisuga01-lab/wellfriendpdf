# Prompt 20B reference disagreement policy

Prompt 20B classifies visual and structural reference results as:

- `within_tolerance`: Wellfriend and the reference render are dimension-compatible
  and within configured pixel thresholds.
- `reference_unavailable_not_counted`: the reference binary or PDFBox runner is
  unavailable and is not treated as a pass.
- `classified_reference_disagreement`: references disagree with each other or a
  known viewer behavior differs, while the affected row remains explained.
- `wellfriendpdf_outlier`: supported Wellfriend output is outside the reference cluster.
- `unclassified_failure`: any unsupported, structural, or visual failure that
  has not been assigned a precise cause.

Prompt 20B acceptance requires zero `wellfriendpdf_outlier`, zero
`unclassified_failure`, and zero security failures for supported rows.
