# Performance and stress benchmark

Run `scripts/run_prompt30_performance_stress.py` only on the VPS. The harness
generates deterministic synthetic PDFs (many pages, object-dense, and text-heavy),
uses an allow-listed public corpus, and calls the real `wellfriendpdf` CLI in
separate capped processes.

It records parse/audit, first-page text extraction, first-page render smoke,
sequential batch throughput, and bounded parallel parser throughput. The runner
uses a per-process address-space cap and limits worker count so configured aggregate
memory remains below the 32 GiB Wellfriend allocation. A crash, timeout, or cap
bypass is a failure; a malformed input rejected cleanly is reported separately.

These measurements are reproducible release evidence, not universal hardware or
renderer-fidelity claims.
