# Final architectural-closure pre-audit

**Audit date:** 2026-08-06
**Repository:** `E:\wellpdfsdk`
**Starting checkpoint:** `7ce0b718f60042db646cb93ce63924b70a501977` on `main`
**Scope:** source inspection before the final implementation pass. This audit does not claim benchmark, corpus, latency, throughput, or competitor evidence.

## Method

The audit inspected the clean local checkpoint, the synchronized authorized VPS checkout, renderer entry points, retained replay, immediate/tile/progressive paths, cache keys, runtime capability reporting, bindings, renderer tests, feature definitions, vector outputs, and source markers including TODO/fallback/compatibility/dead-code terminology. It records source truth at the start of this task and is immediately followed by implementation; it is not a readiness report.

## Active call paths

- Standard immediate: `ContentEngine::render_page_cancellable_with_mode` → `PageRenderer::render_page_cancellable_with_mode` → `RenderState::dispatch_all`.
- Cached retained: `ContentEngine::render_page_cancellable_with_mode_and_cache` → `PageRenderer::get_or_build_display_list_with_cache` → native replay when captured, explicit immediate path only for unsupported lists.
- Retained tile: `render_page_display_list_tile_cancellable_with_mode_and_cache` → tile-local `RenderState::replay_display_list`.
- Progressive: `ProgressiveRenderJob::render_next` → retained tile replay, then explicit immediate tile handling only for unsupported display lists.

## Stable subsystem inventory

| ID | Subsystem | Existing source / entry point | Initial status | Required closure work |
|---|---|---|---|---|
| RV-01 | Canonical document and lazy views | `engine.rs`, `document.rs`, all consumers share `ContentEngine` | MISSING | Add canonical identity coordinator and lazy Render/Edit/Semantic/Validation views. |
| RV-02 | Versioned render contract | `render/buffer.rs`, `display_list.rs`, render APIs | MISSING | Add serializable versioned contract and use its identity in caches. |
| RV-03 | Backend boundary | `RenderDevice`, `CpuRenderDevice` | PARTIAL | Add backend-neutral plan/capability boundary and caller-surface policy. |
| RV-04 | Parsed page program | `ContentEngine::get_page_content` | PARTIAL | Link decoded program to compact source/resource IDs. |
| RV-05 | Retained display list | `DisplayList { ops: Vec<DisplayOp> }` | PARTIAL | Compile packed hot ops and immutable arenas. |
| RV-06 | Hot/cold separation | `DisplayOp` holds paths, state and raw ops | PARTIAL | Move raw high-level payloads/cold diagnostics out of hot commands. |
| RV-07 | Backend render plan | no plan symbol | MISSING | Compile contract + packed list + spatial batches. |
| RV-08 | Native retained replay | `RenderState::replay_display_list` | PARTIAL | Keep unsupported delegation explicit; compile native resource payloads incrementally. |
| RV-09 | Pre-resolved retained state | `RenderState` resolves resources during replay | PARTIAL | Intern states/paths and introduce native payload compile boundary. |
| RV-10 | Safe optimizations | bounds and rectangle specializations | PARTIAL | Make plan-level culling/batching explicit and testable. |
| RV-11 | Dependency/invalidation graph | page/DPI cache strings | MISSING | Add revision/source/page/tile graph and stale-cache prevention. |
| RV-12 | Spatial index | `RenderBounds` linear scan | PARTIAL | Add ordered index preserving paint order and unknown-bound handling. |
| RV-13 | SIMD | `render-simd`, `buffer.rs` | PARTIAL | Preserve scalar oracle; add remaining architecture/operation coverage. |
| RV-14 | Scan conversion | `path.rs` edge buckets and AA | PARTIAL | Close persistent scratch/AET/hostile-geometry policies. |
| RV-15 | Clip/mask graph | `ClipMask`, `AlphaMask` | PARTIAL | Add persistent interned representation and revision-aware keys. |
| RV-16 | Transparency groups | `page_renderer.rs`, `buffer.rs` | PARTIAL | Complete exact surface-pool/print contract. |
| RV-17 | Soft masks | SMask cache and compositor | PARTIAL | Complete contract/revision identity and fusion policy. |
| RV-18 | Shadings/functions | `shading.rs`, `function.rs` | PARTIAL | Compile retained payloads and strengthen exact resource limits. |
| RV-19 | Patterns | `paint_tiling_pattern_*` | PARTIAL | Replace solid approximation with typed exact bounded disposition. |
| RV-20 | Type 3 | Type 3 caches and compatibility path | PARTIAL | Compile immutable glyph sublists and remove silent substitution. |
| RV-21 | Fonts/glyphs | font/glyph caches | PARTIAL | Bound all caches and report deterministic resolution. |
| RV-22 | Images/JPX | image decoder/painter | PARTIAL | Metadata-first region/progressive strategy and full identity. |
| RV-23 | Colour/CMM | qcms/lcms2 paths | PARTIAL | Explicit backend policy, print/proof contract, native feature validation. |
| RV-24 | Form XObjects | Form program cache | PARTIAL | Contract/revision/resource-aware retained sublists. |
| RV-25 | Annotations/widgets | appearance/synthesis paths | PARTIAL | Appearance cache and independent contract controls. |
| RV-26 | Cache hierarchy | `RenderDocumentCache` | PARTIAL | Bound all maps, define revision/tenant/invalidation policy. |
| RV-27 | Allocation discipline | display ops/replay/cache cloning | PARTIAL | Packed arenas/scratch/no-copy cache views and invariants. |
| RV-28 | Tile scheduling | tile/band APIs | PARTIAL | Deterministic adaptive size/priority policy. |
| RV-29 | Progressive lifecycle | resume token only | PARTIAL | Explicit lifecycle, pause/resume/cancel/close reports. |
| RV-30 | Rust/CLI controls | page/DPI/mode/pipeline flags | PARTIAL | Expose contract and profile controls accurately. |
| RV-31 | C ABI parity | PNG/JPEG only | PARTIAL | Versioned contract/surface/progressive/cancel APIs. |
| RV-32 | Other binding parity | Python/WASM simple PNG; .NET/Java no raster API | PARTIAL | Add shared contract adapters and conformance coverage. |
| RV-33 | Print profile | prepress reports | PARTIAL | Add public display/print/proof contract. |
| RV-34 | Determinism/concurrency | tile/cache machinery | PARTIAL | Render-specific scheduler and deterministic test matrix. |
| RV-35 | Security/resource limits | distributed renderer limits | PARTIAL | Central render budget and typed outcome coverage. |
| RV-36 | Fallback closure | inventory contains 12 decisions | PARTIAL | Remove/retire each avoidable fallback; type/count/test residuals. |
| RV-37 | Visual-difference harness | reference Python scripts | PARTIAL | Complete normalized compact-fixture metrics and smoke manifest. |
| RV-38 | Direct PDFium C/C++ harness | absent | MISSING | Add standalone C harness, CMake build and tiny-fixture smoke. |
| RV-39 | Verification closure | Rust tests/CI/binding scripts | PARTIAL | VPS-only valid feature/binding/harness matrix. |
| RV-40 | Documentation/capabilities | prior audit/report/runtime | PARTIAL | Update only after source and VPS evidence support claims. |

## Exact active fallback baseline

Twelve active decisions were identified: retained-list, retained-tile, and progressive unsupported-list delegation; recursive-pattern and excessive-cell solid fallback; Type 3 compatibility substitution; bundled-font substitution; missing named-shading no-op; JPX compatibility path; portable qcms mode; SVG raster embedding; and PS raster embedding. Eight are materially degraded. The new implementation must either remove each one or make a typed, capability-reported exact policy; it must not silently relabel an approximation as completion.

## Initial conclusion

The checkpoint is stable and validated previously, but it is not the requested closed architecture. The immediate implementation sequence is: canonical document/views, render contract, packed-plan/spatial/invalidation foundations, progressive state machine, fallback and vector correctness fixes, binding/harness completion, then VPS-only verification.
