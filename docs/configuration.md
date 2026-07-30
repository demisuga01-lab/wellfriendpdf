# Runtime configuration

The canonical engine configuration is `RuntimeConfig` in `wellfriendpdf_engine::runtime`.

Configuration precedence is:

1. Standard defaults.
2. Optional configuration file.
3. Environment variables.
4. Explicit API/server policy overrides.

The public mode field accepts only:

- `standard`
- `research`

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

Example Research configuration:

```toml
schema_version = 1
mode = "research"

[resources]
cpu_workers = 8
allow_external_network = true
allow_gpu = true

[research]
experimental_renderer = true
model_fusion = true
distributed_workers = false

[ocr]
runtime = "hosted_api"
provider = "openai_compatible"
```

Secrets are represented by references such as environment variables, mounted secret files, OS secret stores, or server hooks. Effective configuration reports never serialize secret values.
