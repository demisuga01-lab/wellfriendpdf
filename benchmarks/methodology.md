# Benchmark methodology

Benchmarks are task-specific. They do not combine unrelated operations into one score, and they do not compare tools on operations that a tool does not implement.

## Measurement rules

- Same host and same corpus for direct comparisons.
- Release builds for Wellfriend runtime measurements.
- One worker unless the result explicitly states otherwise.
- Warmups before measured iterations.
- Median and p95 wall-clock time.
- Peak resident set size where the host exposes it.
- Correctness recorded separately from timing.
- Output reopen or verification required for mutating tasks.

## Corpus rules

The committed corpus manifest uses repository-owned/generated fixtures and compact checked-in fixtures. Large public corpora, private PDFs, raw rendered pages, and downloaded comparator binaries are not committed.

## Comparator rules

Wrappers are identified as wrappers. Unavailable tools are recorded as unavailable, not as losses. Commercial SDK behavior is documentation-only unless a legitimate benchmark license is available.
