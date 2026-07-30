# Runtime modes

Wellfriend exposes exactly two public execution modes: `standard` and `research`.

`standard` is the default production mode. It keeps the full supported engine surface active while adapting concurrency, caches, rendering tiles/bands, temporary spill, copy-through writing, and incremental recomputation to the host limits. The minimum deployment target is 2 vCPU and 6 GB RAM; 4 vCPU and 8 GB RAM is the recommended baseline for higher request concurrency.

`research` includes Standard plus optional provider and accelerator contracts. GPU rendering, advanced local models, hosted OCR/VLM APIs, cloud document intelligence, model fusion, distributed workers, learned cost selection, autotuning, and experimental solvers are inactive unless explicitly configured and permitted by host policy. Missing accelerators are reported as inactive and the engine falls back to Standard behavior.

Public mode selection is available through Rust, CLI, configuration files, environment variables, server startup policy, permitted per-request server policy, Python, C ABI, WASM, .NET, and Java.

Unsupported, undersized, or policy-denied optional capabilities return structured inactive states. They do not silently change document meaning, weaken redaction, bypass provenance, or disable canonical validation.
