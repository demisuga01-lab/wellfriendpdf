# Concurrency

Standard mode derives concurrency from the effective host profile and resource policy. On the 2 vCPU / 6 GB minimum profile, CPU-heavy work is intentionally constrained and mutation remains serial per document. On the 4 vCPU / 8 GB recommended profile and larger hosts, independent work such as pages, tiles, streams, OCR regions, validation rules, and output preparation can use more permits while remaining bounded.

The server uses bounded queues and backpressure. Untrusted requests cannot raise memory, GPU, provider, network, worker, or tenant policy limits.

Deterministic document meaning is independent of scheduling. Parallel preparation is allowed only where final emission, transaction ordering, and validation remain deterministic.
