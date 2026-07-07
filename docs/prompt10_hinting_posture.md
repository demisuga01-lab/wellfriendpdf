# Prompt 10 Hinting Posture

Prompt 10B chooses the pure-Rust acceptance path for hinting.

No native hinting backend is added, no native dependency is enabled silently,
and no optional native boundary is required for the current Prompt 10B release
threshold. Oxide continues to use the existing pure-Rust outline and raster
path with light grid fitting for small TrueType outlines where applicable.

Evidence:

- `hinting-posture-prompt10b.json`
- `prompt10b-multi-reference-diff-metrics.json`
- `prompt10b-reference-disagreement-summary.json`

The Prompt 10B CJK/RTL/color-glyph corpus records zero Oxide outlier failures
and zero unclassified failures under this posture. Optional native hinting may
be considered later as a feature-gated enhancement, but it is not a Prompt 10B
blocker.
