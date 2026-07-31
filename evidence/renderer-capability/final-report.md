# Renderer final evidence

Result folder: `/mnt/wellpdf-block/results/renderer-capability-20260730T172700Z`.

Corpus: 5,044 real PDFs, 17,059,245,901 bytes, 116,975 external-renderer pages, duplicate SHA-256 values: 0.

Final Wellfriend all-pages run: 5,044 files / 116,975 rendered pages / 0 failures. The run used 72 DPI, compat render quality, raw hash evidence, immediate pipeline, 8 workers, and a 300000 ms per-page timeout. It completed in 991 seconds with peak RSS 3624676 KiB. Median per-file wall time was 947.4 ms; P95 4170.3 ms; P99 11414.4 ms.

Follow-up display-list all-pages run: 5,044 files / 116,975 rendered pages / 0 failures. The run used the same 72 DPI, compat render quality, raw hash evidence, 8 workers, and 300000 ms per-page timeout, but selected the display-list pipeline. It completed in 1065 seconds with peak RSS 3241976 KiB. Median per-file wall time was 970.4 ms; P95 4269.2 ms; P99 11114.6 ms. This is evidence that the display-list path is functional at corpus scale, not evidence that it beats the immediate path on this corpus.

One candidate renderer micro-optimization for direct initial page-buffer filling was tested on the VPS using the existing 80-file render probe. It compiled and passed focused pixel-buffer tests, but the measured probe did not improve the display-list path against the immediate path, so the code change was reverted and classified as a rejected candidate rather than retained production code. The retained source tree therefore stays tied to measured improvements only.

Before the final slow-path fixes, the prior full all-pages run had 30 failed files. The remaining failure subset rerun completed with 0 failures.

Raw per-file rows and command logs remain on the VPS/block-storage result folder; this repository keeps aggregate evidence and artifact hashes only.

Final VPS workspace gates passed after the all-pages run:

- `final-vps-fmt-current`: exit 0
- `final-vps-diff-check-current`: exit 0
- `final-vps-cargo-check-current`: exit 0
- `final-vps-clippy-current`: exit 0
- `final-vps-cargo-test-current`: exit 0
