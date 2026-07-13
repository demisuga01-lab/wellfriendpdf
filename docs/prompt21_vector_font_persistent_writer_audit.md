# Prompt 21 Vector Font Persistent Writer Audit

Combined Prompt 21 started from commit `5573732eb187b9e0e882d9474a9d6a07315144a2` (`Close combined prompt 20B multirun form appearance closure`) with a clean worktree verified before edits. The machine-readable checkpoint is `target/prompt21-vector-font-persistent-writer/prompt21-starting-state.json`.

Canonical paths reused:

| Area | Path |
| --- | --- |
| Shared implementation | `crates/engine/src/prompt21.rs` |
| SDK facade | `crates/engine/src/sdk.rs` |
| CLI | `crates/cli/src/main.rs` |
| Raster inventory/decode | `crates/engine/src/images/locator.rs`, `crates/engine/src/images/decoder.rs` |
| Font inventory | `crates/engine/src/fonts_report.rs` |
| Writer object/xref streams | `crates/engine/src/writer.rs` |
| C ABI / Python / WASM | `crates/oxide-capi`, `crates/oxide-py`, `crates/oxide-wasm` |
| .NET / Java | `bindings/dotnet`, `bindings/java` |

The audit harness is `scripts/prompt21_vector_font_persistent_writer_audit.py`. It emits the feature matrix, raster/font/persistent/object-stream reports, reference-tool results, metamorphic results, performance/limit files, and HTML index under `target/prompt21-vector-font-persistent-writer/`.

No Prompt 21 feature-matrix row is `blocked`. Risky areas remain `implemented_with_limits` or exact unsupported policy rows rather than being reported as complete reconstruction.
