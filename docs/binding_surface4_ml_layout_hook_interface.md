# Semantic Intelligence ML Layout Hook Interface

Semantic Intelligence defines a backend-neutral layout proposal interface. The hook is
optional and disabled by default. Core extraction, search, RAG chunking,
redaction evidence, and semantic text do not require ML packages, model files,
network access, cloud credentials, or GPUs.

The stable proposal schema is `LayoutProposalSet`.

It records:

- schema version
- backend ID and backend type
- model name, version, and hash
- input page IDs and payload kind
- region proposals with label, confidence, geometry, polygon, reading-order hint, and provenance
- diagnostics
- runtime and memory metadata
- privacy flags
- deterministic merge outcome

Supported region labels include title, body, table, figure, caption, list,
header, footer, and unknown.

Merge policy:

- deterministic model is primary
- high-confidence proposals can add labels or hints
- low-confidence proposals remain suggestions
- proposals cannot silently delete deterministic text
- conflicts are surfaced as diagnostics
- table, figure, and caption hints preserve source span provenance

Exact limits:

- no real ML runtime is bundled
- semantic intelligence provides mock/template backends and schema validation
- cloud calls are never made by default
