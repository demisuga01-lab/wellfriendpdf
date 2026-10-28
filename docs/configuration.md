# Runtime configuration

The canonical engine configuration is `RuntimeConfig` in `wellfriendpdf_engine::runtime`.

Configuration precedence is:

1. Standard defaults.
2. Optional configuration file.
3. Environment variables.
4. Explicit API/server policy overrides.

The public mode field accepts only `standard`. `research` is still accepted as a deprecated alias that maps to `standard` and records the decision `research_mode_deprecated_merged_into_standard` (see [research-mode.md](research-mode.md)).

Example Standard configuration:

```toml
schema_version = 1
mode = "standard"

[resources]
cpu_workers = 2
soft_memory_bytes = 4294967296
hard_memory_bytes = 4928307200
allow_external_network = false

[ocr]
runtime = "self_hosted"
provider = "tesseract"
```

Example Standard configuration with optional capabilities enabled explicitly:

```toml
schema_version = 1
mode = "standard"

[resources]
cpu_workers = 8
allow_external_network = true
allow_gpu = true

[providers]
allow_hosted_api = true

[advanced]
experimental_renderer = true
model_fusion = true
gpu_rendering = true
distributed_workers = false

[ocr]
runtime = "hosted_api"
provider = "openai_compatible"
```

Secrets are represented by references such as environment variables, mounted secret files, OS secret stores, or server hooks. Effective configuration reports never serialize secret values.
