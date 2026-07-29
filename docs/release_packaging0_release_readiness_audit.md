# Release Readiness Benchmark release-readiness audit

Release Readiness Benchmark is the final release-hardening unit for Wellfriend PDF SDK. It records
the starting commit, VPS-only execution evidence, performance/stress results,
security package, package gates, public API inventory, and final release posture.

Heavy commands run on the isolated VPS at `35.185.176.47` under the 32 GiB
Wellfriend budget. Local Windows work is limited to source edits and Git hygiene.
Raw PDFs, benchmark output, sanitizer output, and private-like material remain in
the VPS result directory; only concise machine-readable summaries are retained in
`target/release_readiness_benchmark-release-readiness/`.
