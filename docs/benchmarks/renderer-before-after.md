# Renderer before/after evidence

The final VPS campaign ran against `/mnt/wellpdf-block/corpus/real-5000-current` on `ubuntu@51.77.178.150`.

| Run | Files | Rendered pages | Failures | Median ms | P95 ms | P99 ms | Peak RSS KiB | Evidence |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Previous Wellfriend all-pages resolver-cache run | 5,044 | 116,493 | 30 | 1028.8 | 4980.8 | 19200.6 | 3687016 | `artifacts/wellfriend-render-all-pages-72-resolvercache-summary.json` |
| Prior immediate-path all-pages run | 5,044 | 116,975 | 0 | 947.4 | 4170.3 | 11414.4 | 3624676 | `wellfriend-render-all-pages-final-current-2-summary.json` |
| Document-scoped render cache, 8 workers | 5,044 | 116,975 | 0 | 572.8 | 3224.5 | 10503.9 | 4156972 | `wellfriend-render-all-pages-document-cache-w8-summary.json` |

The final pass includes bounded compat-mode fallbacks for pathological Form XObject tiling-pattern replay and Type3 fallback text. High-quality mode retains the exact rendering path where supported.

The document-scoped render cache is keyed by font resource dictionaries and embedded font bytes, not by resource names alone. A same-binary 100-file on/off VPS probe produced identical raw render hashes and measured a 1.68x total-time speedup, so the cache is retained for repeated all-page rendering.
