# Runtime mode

Wellfriend has exactly one public execution mode: `standard`.

`standard` keeps the full supported engine surface active while adapting concurrency, caches, rendering tiles/bands, temporary spill, copy-through writing, and incremental recomputation to the host limits. The minimum deployment target is 2 vCPU and 6 GB RAM; 4 vCPU and 8 GB RAM is the recommended baseline for higher request concurrency. No GPU, model, or cloud provider is required.

The former `research` mode has been merged into Standard and is deprecated. See [research-mode.md](research-mode.md) for the compatibility rules.

## Optional capabilities

Standard can use additional components when an operator turns them on explicitly. Every one of them is off by default, and each falls back to the deterministic CPU path when its prerequisites are missing.

| Capability (report name) | Enabled by | Additional prerequisite |
| --- | --- | --- |
| `gpu_hybrid_rendering` | `advanced.gpu_rendering` | `resources.allow_gpu = true` and a GPU on the host |
| GPU render backend | `rendering.experimental_gpu_backend` | `resources.allow_gpu = true` and a GPU on the host |
| `model_ensembles` | `advanced.model_fusion` | — (the OCR routing policy `model_fusion` additionally needs configured providers) |
| `distributed_workers` | `advanced.distributed_workers` | `resources.allow_external_processes = true` |
| `learned_cost_selection` | `advanced.learned_cost_selection` | — |
| `hardware_autotuning` | `advanced.hardware_autotuning` | — |
| `translation_validated_display_optimization` | `advanced.display_optimization_validation` | — |
| `experimental_solvers` | `advanced.experimental_solvers` | — |
| `hosted_api_ocr` | an enabled hosted OCR provider (registry permission `providers.allow_hosted_api`) | `resources.allow_external_network = true` and a host policy that allows external network providers |
| `cloud_document_intelligence` | an enabled cloud provider (registry permission `providers.allow_cloud_document_intelligence`) | `resources.allow_external_network = true` and a host policy that allows external network providers |

`advanced.experimental_renderer` and `advanced.instrumentation` are opt-in switches too. They do not appear as separate report entries.

Network access, hosted APIs, cloud providers, GPU use, and external processes remain explicit operator opt-ins. Merging research into Standard did not turn any of them on.

The capability report (`/api/v1/capabilities`, `wellfriendpdf capabilities`, and the binding equivalents) lists each optional capability with `mode: "standard"` and one of these states:

- `experimental`: the capability is enabled and its prerequisites are met.
- `inactive_policy`: the capability is off (the default), or a permission it needs (such as `allow_gpu`, `allow_external_processes`, or the network policy) is not granted.
- `inactive_missing_hardware`: the capability is enabled, but the host has no GPU.
- `inactive_missing_provider` / `configured`: the provider state for OCR families.

The report also lists `public_modes: ["standard"]` and `deprecated_mode_aliases: ["research"]`.

You can configure Standard through Rust, the CLI, configuration files, environment variables, server startup configuration, Python, the C ABI, WASM, .NET, and Java.

Optional capabilities that are unsupported, undersized, or blocked by policy report a structured inactive state. They never silently change document meaning, weaken redaction, bypass provenance, or disable canonical validation.
