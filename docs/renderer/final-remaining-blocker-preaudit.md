# Final remaining renderer blocker preaudit

**Repository:** `E:\wellpdfsdk`
**Starting commit:** `bfc28b7a894f77623e75daf39e73f5a721cf01d5`
**Scope:** remaining source-backed renderer closure work after the universal revalidation. This is a preimplementation inventory, not a readiness claim and not benchmark evidence.

## Method

Inspected the committed implementation/fallback inventories, current renderer source, active entry points, binding code, and current fallback markers. The goal of this document is to bind each remaining blocker to an implementation, test, and public-surface requirement before source work begins.

| ID | Subsystem | Exact source locations / active path | Current behavior / output degradation | Missing implementation | Bindings affected | Tests missing | Completion evidence required |
|---|---|---|---|---|---|---|---|
| RB-01 | Universal packed high-level plans | `render/plan.rs:ColdPayload`, `PackedDisplayList::compile`, `RenderPlan::execute_vector_tile`; `page_renderer.rs:RenderState::replay_display_op` | Text/image/shading/pattern/Form remain raw `ContentOperation` cold payloads and replay through canonical interpreter | Typed pre-resolved payload arenas and native planned execution for every supported high-level op | all | high-level packed plan equivalence/no dictionary lookup/no immediate delegation | retained text/image/Form/shading/pattern/group fixtures execute plan path |
| RB-02 | Narrow edit invalidation | `render/invalidation.rs`; `RenderDocumentCache::invalidate_sources`; `editing_transactions.rs:dirty_region_report` | Graph exists but edits do not supply changed IDs; revision bind clears conservatively | transaction write-set → source ID mapping → dependency invalidation | Rust/C/Python/WASM/.NET/Java/server | shared resource, undo/redo, stale-tile tests | local edit preserves unrelated page tiles and invalidates transitive dependents |
| RB-03 | Persistent clip DAG | `render/buffer.rs:ClipMask`; `page_renderer.rs:clip_stack`; `plan.rs:OP_CLIP` | masks/clips clone through save/restore; no persistent representation | interned Full/Empty/Rectangle/Sparse/RLE/Dense/Composite clip graph | all | repeat/nested/transformed/tile/memory tests | no full-mask clone for reusable clips; tile-local materialization |
| RB-04 | Transparency, masks, print | `page_renderer.rs:apply_smask`, group handlers; `buffer.rs:composite_from`; `contract.rs:PrintProfile` | common semantics exist; print/halftone/proof fields are not renderer-active | packed group descriptors, complete mask cache identity, print/proof policy routing | all | group/knockout/print cache separation tests | exact supported groups; typed unsupported exact print paths |
| RB-05 | Adaptive tile scheduling | `render/progressive.rs:ProgressiveRenderJob` | fixed raster-order uniform tiles | deterministic visibility/budget-aware scheduler and obsolete-work cancellation | Rust/server/bindings | visible-first/budget/order determinism tests | priority schedule active in progressive execution |
| RB-06 | Region/progressive image decode | `page_renderer.rs:scheduled_decode_image_with_color_space`; `images/jpx.rs`; `image_painter.rs` | full image decode before paint; tile culling skips paint only | metadata/ROI/reduction/progressive decoder interface and cache key fields | all | region/reduction/cancel/cache tests | no invisible decode; codec-supported reduced decode active |
| RB-07 | Actual WASM SIMD | `crates/render-simd/src/lib.rs` wasm branches | wasm32 takes scalar oracle paths | guarded `simd128` kernels and equality tests | WASM | random/unaligned/boundary SIMD equivalence | actual wasm SIMD build selects kernels where supported |
| RB-08 | Progressive sessions | `render/progressive.rs`; binding crates lack handles/classes | lifecycle exists only in Rust | C opaque job, Python/WASM/.NET/Java/server adapters | C/Python/WASM/.NET/Java/server | lifecycle/cancel/close parity manifest | start/continue/pause/resume/cancel/close exposed everywhere |
| RB-09 | Caller-owned surfaces | `engine.rs:render_page_with_contract`; `PixelBuffer` | internal allocation only | validated output surface descriptor and `render_into` API | Rust/C/Python/WASM/.NET/Java | stride/format/short-buffer/ownership tests | caller buffer exercised in each applicable binding |
| RB-10 | Contract builders | `render/contract.rs`; binding default JSON adapters | default or JSON-only; core rejects deviations | active schema-v1 field handling and language builders | Rust/CLI/C/Python/WASM/.NET/Java/server | serialization/default/field effect parity tests | all public fields round-trip and alter active semantics or typed refusal |
| RB-11 | Font substitution diagnostics | `font_rasterizer.rs:get_fallback_font`; `page_renderer.rs:get_font_bytes` | deterministic bundled substitution can be invisible to render caller | structured render-time substitution events/report | all | embedded/missing/CID metric and report tests | report names requested/chosen font, reason, risks, glyph coverage |
| RB-12 | Type 3 residual Compat | `page_renderer.rs:render_text_string`, `get_type3_compat_font_bytes` | native-first but unresolved Compat can substitute font | packed Type 3 sublists/state machine and typed unresolved outcome | all | recursion/unsupported/color-resource tests | supported Type 3 never substitutes ordinary font |
| RB-13 | JPX semantics | `images/jpx.rs`; `image_painter.rs` | full decode and compatibility sampling path | capability-specific reduction/region policy and explicit unavailable result | all | JPX reduction/alpha/colour/cancel tests | exact decoder capability report and cache identity |
| RB-14 | SVG/PS regional fallback | `render/svg.rs:needs_raster_fallback`; `render/postscript.rs:needs_raster_fallback` | local image/shading/ExtGState can rasterize whole page | native image/gradient paths plus bounded regional fallback | CLI/Rust | mixed vector/raster bounds/order/seam tests | unsupported region only embeds raster; surrounding vectors remain native |
| RB-15 | Visual reference normalization | `tools/renderer-visual-diff/visual_diff.py` | metrics work but normalization/classification incomplete | canonical comparison surface, masks/classification/critical regions | tooling | alpha/premultiply/rotation/mask fixtures | cross-renderer compact manifest normalization passes |
| RB-16 | Active fallback categories | `docs/renderer/final-fallback-closure-report.md`; `runtime.rs` | 9 active categories, 5 material degradations | eliminate or convert to typed exact policy | all | one test per category, high-quality guard | zero silent material high-quality fallback categories |
| RB-17 | Cache resource accounting | `page_renderer.rs:enforce_bounded_maps` | mixed byte/entry caps and conservative eviction | aggregate budget, LRU admission, per-resource byte charge | Rust/server | aggregate/eviction/tenant tests | contract budget constrains aggregate caches deterministically |
| RB-18 | Deterministic rendering matrix | `progressive.rs`, `page_renderer.rs` | existing deterministic fixtures but no full worker/cache/tile matrix | deterministic execution policy and tests | Rust/server | cold/warm, one/multi worker, tile ordering tests | same pixels across supported execution schedules |

## Nine active fallback-policy categories

1. Unsupported retained-list canonical immediate replay.
2. Unsupported retained-tile canonical immediate replay.
3. Unsupported progressive-tile canonical immediate replay.
4. Unresolved Type 3 Compat substitution.
5. Deterministic bundled font substitution.
6. JPX compatibility path.
7. Portable qcms backend selection.
8. SVG raster embedding.
9. PS/EPS raster embedding.

The previous recursive/excessive pattern solid fallbacks and retained missing-named-shading no-op are already converted to typed failure/diagnostic behavior and are not counted as active output-degrading fallbacks.

## Immediate implementation order

1. Complete the active RenderContract/surface/cancellation foundation so bindings can share real controls.
2. Add C ABI progressive/caller-surface primitives and adapt Python/WASM/.NET/Java.
3. Convert bounded caches and transaction invalidation to active APIs with tests.
4. Add persistent clip plan representation and adaptive scheduler.
5. Implement actual WASM SIMD kernels with scalar equivalence.
6. Close remaining high-level packed plans and rendering-semantic fallbacks, then vector regionalization and visual normalization.

No performance, corpus, latency, throughput, or competitor benchmark is part of this preaudit or its implementation plan.
