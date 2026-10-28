# Research mode (deprecated)

Research mode has been merged into Standard. WellfriendPDF now has a single execution mode, `standard`. Every former research component is an ordinary optional Standard capability. Each one is off by default and is enabled only by its own explicit switch. See [runtime-modes.md](runtime-modes.md).

## What changed

- The `research` value of `ExecutionMode` was removed. Standard is the only variant.
- `ResearchConfig` / the `research` config section were renamed to `AdvancedConfig` / `advanced`. Switches such as `model_fusion`, `gpu_rendering`, and `distributed_workers` no longer require a special mode. The old error "research-only switches require mode='research'" is gone.
- `RuntimeConfig::research()` is `#[deprecated]` and returns `RuntimeConfig::standard()`. It no longer enables GPU, external processes, hosted APIs, or cloud providers. Enable the capabilities you need explicitly.
- Render contracts: `BackendSelection::ResearchHybrid` is now `Hybrid`, and `DeterminismPolicy::BestEffortResearch` is now `BestEffort`. The contract `ExecutionMode` has only `Standard`.
- OCR routing policy `research_fusion` is now `model_fusion`.
- The Rust module `research_edit_synthesis` (ECBES) is now `edit_synthesis`. The old path remains as a deprecated re-export.
- The server switches `WELLFRIENDPDF_ALLOW_RESEARCH` / `allow_research` and `WELLFRIENDPDF_FORCE_STANDARD` / `force_standard` are deprecated no-ops. They are still parsed, startup still succeeds, and a warning is logged.

## Compatibility aliases

Old inputs keep working and are normalized to Standard:

| Old input | Accepted where | Result |
| --- | --- | --- |
| `research` mode (`--mode research`, `WELLFRIENDPDF_MODE=research`, `mode = "research"`, JSON `"mode": "research"`) | CLI, env, config files, JSON, bindings, server | `standard`, plus the decision `research_mode_deprecated_merged_into_standard` |
| `[research]` section / `"research": {...}` key, `research_instrumentation` | config files, JSON | `advanced`, plus the decision `research_config_section_deprecated_renamed_to_advanced` |
| `force_standard` / `allow_research` / `allowed_tenants_for_research` host policy | server config, serialized policies | ignored, plus the decision `host_policy_research_switches_deprecated_ignored` |
| Contract `execution_mode: "Research"` | contract JSON, C ABI, WASM, Python, server, .NET, Java | `Standard` (same cache identity as Standard) |
| Contract `backend: "ResearchHybrid"` / `research-hybrid` | same | `Hybrid` |
| Contract `determinism: "BestEffortResearch"` / `best-effort-research` | same, plus CLI `--determinism` | `BestEffort` |

Deprecation decisions appear in `EffectiveRuntimeConfig.decisions`, which you can read through `wellfriendpdf runtime-config` and `/api/v1/runtime-config`. The CLI prints a one-line warning on stderr, and the server logs a warning. New output always uses the new names.

The deprecated aliases will be removed in a future schema version.
