# Raster To Vector

Prompt 21 exposes bounded raster-to-vector analysis through `wellfriendpdf_engine::prompt21::raster_vectorization_report`, CLI commands `raster-vector-report` and `raster-vectorize`, and SDK/binding report wrappers.

Supported evidence:

| Content | Status |
| --- | --- |
| Monochrome line art and thresholded diagrams | `implemented_with_limits` |
| Horizontal/vertical lines, filled regions, rectangles, ellipse candidates | `implemented_with_limits` |
| Dense photos, textured art, arbitrary noisy scans | `unsupported_reported_exact` or low-confidence report-only |

The output is a reconstructed vector model from raster evidence, not the original authoring path. By default Wellfriend exports/report vectors and leaves source rasters unchanged. Replacement requires an explicit clone-one-resource policy because shared image XObjects can appear on multiple pages or in multiple placements.

Artifacts: `raster-vectorization-primitive-results-prompt21.json`, `raster-vectorization-topology-prompt21.json`, and `raster-vectorization-curve-error-prompt21.json`.
