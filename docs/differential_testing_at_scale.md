# Differential testing at scale

`scripts/run_differential_at_scale.py` compares Wellfriend PDF SDK against available external tools over the Malformed Coverage corpus manifest.

Comparison dimensions include:

- Wellfriend open/parse success and diagnostics;
- qpdf structural checks where available;
- Poppler text/page metadata tools where available;
- MuPDF checks where available;
- veraPDF PDF/A status where applicable;
- pyHanko signature status where applicable;
- page-count, extraction, render-smoke, and repair-mode differences when stable.

The goal is not to make Wellfriend match every permissive external parser. The goal is to detect high-severity regressions, classify disagreements, and keep malformed-input behavior deterministic and fail-closed.

Output artifacts:

- `differential-tool-support-matrix.json`
- `differential-corpus-manifest.json`
- `differential-run-results.json`
- `differential-disagreement-buckets.json`
- `differential-scale-scorecard.json`
- `differential-manual-review-queue.json`
