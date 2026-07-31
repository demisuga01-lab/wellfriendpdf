# Renderer before/after evidence

The final VPS campaign ran against `/mnt/wellpdf-block/corpus/real-5000-current` on `ubuntu@51.77.178.150`.

| Run | Files | Rendered pages | Failures | Median ms | P95 ms | P99 ms | Peak RSS KiB | Evidence |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Previous Wellfriend all-pages resolver-cache run | 5,044 | 116,493 | 30 | 1028.8 | 4980.8 | 19200.6 | 3687016 | `artifacts/wellfriend-render-all-pages-72-resolvercache-summary.json` |
| Final Wellfriend all-pages current run | 5,044 | 116,975 | 0 | 947.4 | 4170.3 | 11414.4 | 3624676 | `wellfriend-render-all-pages-final-current-2-summary.json` |

The final pass includes bounded compat-mode fallbacks for pathological Form XObject tiling-pattern replay and Type3 fallback text. High-quality mode retains the exact rendering path where supported.
