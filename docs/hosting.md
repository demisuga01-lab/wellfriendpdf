# Hosting workflow

Standard is the recommended hosting mode for production APIs and ordinary commercial deployments.

Typical workflow:

1. Build the CLI/server and choose `mode = "standard"`.
2. Set memory, worker, queue, and temporary-storage limits.
3. Choose an OCR runtime policy: disabled, self-hosted, hosted API, or cloud document intelligence.
4. Disable external providers unless the operator has approved data-residency, cost, privacy, and provider-governance controls.
5. Start the server with API keys and restrictive CORS.
6. Query `/api/v1/capabilities`, `/api/v1/runtime-config`, and `/api/v1/providers`.
7. Monitor queue depth, memory-pressure actions, provider health, and cancellation.

Standard is the only execution mode; the former research mode was merged into it (see [research-mode.md](research-mode.md)). Optional capabilities such as GPU rendering, hosted or cloud OCR providers, model fusion, distributed workers, and experimental solvers stay off unless explicitly enabled (see [runtime-modes.md](runtime-modes.md)). Enable them only for controlled R&D or enterprise evaluation: they can have worse latency, queue growth, memory pressure, provider cost, or fallback behavior on undersized hosts.
