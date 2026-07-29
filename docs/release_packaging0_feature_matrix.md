# Release Readiness Benchmark feature matrix

The generated `target/release_readiness_benchmark-release-readiness/release_readiness_benchmark-feature-matrix.json`
maps the four final roadmap units to evidence:

- 117: public corpus, generated fixtures, bounded batch/parallel benchmarks, and memory results.
- 118: threat model, dependency/license/SBOM posture, secret scan, and native-boundary audit.
- 119: Rust, CLI, Python, C ABI, WASM, .NET, and Java API inventories plus package gates.
- 120: available external-tool comparison and an evidence-backed release go/no-go.

Rows distinguish implemented evidence from unavailable external tools. An external
tool being absent is never converted into a pass.
