# Wellfriend benchmark suite

The benchmark suite measures actual runtime work on compact legal fixtures. It is intended to support README numbers and regressions, not to replace exhaustive product validation.

## Layout

```text
benchmarks/
  corpus/manifest.json
  harness/
  results/latest/
    environment.json
    tool-versions.json
    raw-results.json
    results.csv
    summary.json
    summary.md
    correctness.json
```

## Running

From the repository root, run the benchmark example in release mode:

```bash
cargo run --release -p wellfriendpdf-engine --example repository_benchmarks -- benchmarks/results/latest
```

The run uses one process and one worker, performs warmups before measured iterations, verifies save/reopen where the task writes a document, and records median and p95 timing separately from correctness.
