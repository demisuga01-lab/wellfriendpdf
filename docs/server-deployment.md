# Server deployment

Server administrators can:

- disable external provider networks;
- require local/self-hosted OCR;
- cap memory, CPU workers, queues, and provider cost;
- inspect effective runtime configuration without exposing secrets.

Relevant environment variables include:

- `WELLFRIENDPDF_MODE`
- `WELLFRIENDPDF_RUNTIME_CONFIG_FILE`
- `WELLFRIENDPDF_RUNTIME_CONFIG_JSON`
- `WELLFRIENDPDF_ALLOW_EXTERNAL_PROVIDERS`
- `WELLFRIENDPDF_LOCAL_ONLY_OCR`
- `WELLFRIENDPDF_CPU_WORKERS`
- `WELLFRIENDPDF_MEMORY_SOFT_BYTES`
- `WELLFRIENDPDF_MEMORY_HARD_BYTES`

Runtime endpoints:

- `GET /api/v1/capabilities`
- `GET /api/v1/runtime-config`
- `GET /api/v1/providers`

These endpoints are reports. They do not upload documents to OCR/model providers.

Standard is the only execution mode. `WELLFRIENDPDF_FORCE_STANDARD` and `WELLFRIENDPDF_ALLOW_RESEARCH` are deprecated no-ops: they are still parsed, startup still succeeds, and the server logs a warning. `WELLFRIENDPDF_MODE=research` maps to `standard`. Optional capabilities are enabled through the runtime configuration (see [runtime-modes.md](runtime-modes.md)).
