# Benchmark methodology

Wellfriend benchmark evidence is split into two classes.

1. Compact repository fixtures exercise exact SDK operations and are suitable for
   CI, package gates, and regression tests.
2. Large real-world corpus campaigns run on VPS storage and commit only aggregate
   results, methodology, hashes, and legal summaries. Raw PDFs, raw logs, rendered
   pages, and large per-file artifacts stay off Git.

Renderer-capability benchmarks use Standard mode unless explicitly stated. A row
is benchmark-safe only when the tool performs the same task on the same input
set with the same page selection, DPI, output family, timeout policy, and worker
budget. If equivalence is not possible, the result is classified as a capability
observation rather than a win/loss.

The real 5,044-PDF corpus is public, externally downloaded, and domain-skewed
toward arXiv academic PDFs. It contains 17,059,245,901 bytes and 116,784
qpdf-counted pages with zero duplicate SHA-256 values in the committed aggregate.

Unsupported, timed-out, unavailable, and malformed-input rows remain in the
evidence. They are not removed from medians or silently converted to passes.

## Renderer 5,044-PDF corpus method

All renderer rows use the block-storage corpus at `/mnt/wellpdf-block/corpus/real-5000-current` on `ubuntu@51.77.178.150`. Runs are command-line, 72 DPI, same corpus, bounded timeout, aggregate logs retained under the result folder. Wellfriend final evidence mode is raw pixel hashing; comparator renderers use their available raster output path. Wrapper relationships are labeled.
