# Final local implementation closure

**Repository:** `E:\wellpdfsdk`
**Branch:** `main`
**Start commit:** `2c893fbfe5ca3799f7ba9e437fe080f63735e0ca`
**Final commit:** this report is intended to be committed with the final local source delta.
**Local work root:** `.work/final-universal-renderer-implementation`
**Scope boundary:** local source implementation only. No VPS, SSH, PDF corpus, real-PDF verification campaign, performance benchmark, competitor comparison, deployment, release tag, or package publication was used.

## Resource controls

Commands in this continuation used single-job Rust execution and local disk-backed temp/target storage:

```text
CARGO_BUILD_JOBS=1
RUST_TEST_THREADS=1
RAYON_NUM_THREADS=1
DOTNET_PROCESSOR_COUNT=3
UV_THREADPOOL_SIZE=3
NODE_OPTIONS=--max-old-space-size=768
CARGO_TARGET_DIR=E:\wellpdfsdk\.work\final-universal-renderer-implementation\cargo-target
TEMP=E:\wellpdfsdk\.work\final-universal-renderer-implementation\temp
TMP=E:\wellpdfsdk\.work\final-universal-renderer-implementation\temp
```

One earlier interrupted cargo run briefly left multiple compiler processes above the intended memory ceiling; those processes exited before continuation. During the Form XObject retained-plan slice, two focused Cargo tests were accidentally launched together, but Cargo serialized them on its package/artifact locks. During the latest server render-contract builder pass, one focused Cargo test and one Cargo check were accidentally launched together; the check timed out while waiting/compiling, no Cargo/rustc/rustdoc processes remained afterward, and the focused test, check, fmt, and clippy gates were rerun serially under the single-job policy. During the latest named-color tint-transform slice, two focused Cargo tests were also launched together and serialized on Cargo package/artifact locks. During the latest DCT Separation inline-image slice, one Cargo check and one clippy run were accidentally launched together; clippy waited on Cargo's build lock, then completed under the single-job settings. During the latest JPX dimension-consistency slice, two focused Cargo test filters were accidentally launched together and serialized on Cargo package/artifact locks; later fmt/check/clippy gates were run serially. During the latest ResearchHybrid backend slice, two focused Cargo tests were accidentally launched together and serialized on Cargo package/artifact locks. During the clip-DAG dense alpha fusion slice, an overly broad `fuses` test filter matched unrelated stale tests; the stale exact image-decode fixture was corrected, and the affected tests were rerun with exact filters. During the latest transparency reusable-surface slice, the interrupted/truncated Cargo output was discarded, no Cargo/rustc process remained, and all focused gates were rerun serially.

## Latest progressive retained-tile refusal event gates

This continuation makes progressive unsupported retained-tile refusals auditable through the existing structured fallback-event report fields. `render/progressive.rs::render_next` now records `unsupported_display_list_retained_tile_refusal` before moving a job to `Failed` when retained display-list tile replay returns the typed unsupported retained-replay error. The event records call site, trigger, output, degradation, Standard/high-quality availability, replacement, final policy, and tile identity; it does not publish fallback pixels or re-enable immediate rendering.

The first focused rerun proved the compatibility path and exposed that the high-quality error string used a different prefix; the refusal classifier was widened to the shared `unsupported retained display-list replay` text and rerun successfully.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_refuses_unsupported_display_list_tile --lib --jobs 1 -- --test-threads=1 --nocapture` before high-quality classifier widening | 1 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_refuses_unsupported_display_list_tile --lib --jobs 1 -- --test-threads=1 --nocapture` after classifier widening | 0 |
| `cargo test -p wellfriendpdf-engine fallback_report_keeps_codes_and_structured_policy_details --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest PostScript exact shading-domain regionalization gates

This continuation narrows the SVG/PS regional-vector fallback boundary without approximating nonlinear SVG output. `render/vector_fallback.rs` now carries the PDF shading `/Domain` on axial/radial vector shading records and allows exact PostScript Type 2/Type 3 function sidecars when the shading domain is finite and non-zero, instead of requiring `[0 1]`. `render/postscript.rs` emits `/Domain [...]` only when an exact PostScript function sidecar is present; sampled fallback gradients keep normalized stop offsets. SVG nonlinear shadings still refuse whole-page/vector-exact output because SVG cannot represent the nonlinear PDF function exactly.

Two focused Cargo tests in this slice were accidentally launched together and serialized on Cargo package/artifact locks. Later check and clippy gates were run serially under the single-job policy.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_type2_function_with_non_unit_shading_domain_is_exact_postscript_regional --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_shading_domain_clause_is_exact_function_only --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest managed render-cache handle smoke gates

This continuation verifies the managed non-progressive caller-owned render-cache source surface without running a broad binding/package matrix. The first .NET runtime smoke without a native DLL load path failed at process startup with `DllNotFoundException`; rerunning the same focused test with `PATH` pointing at the configured `.work` Cargo target DLL passed and exercised `RenderCache`, cached contract PNG/report rendering, render-invalidation-plan application, and cache clearing. Java main/test sources and the package-smoke source compiled directly with Java 25 preview enabled and cover the matching `WellfriendPdf.RenderCache` source surface.

| Command | Exit |
|---|---:|
| `dotnet test bindings\dotnet\WellfriendPdf.Tests\WellfriendPdf.Tests.csproj --filter FullyQualifiedName~RasterRenderingUsesTheCanonicalNativeSurface -m:1 --logger "console;verbosity=minimal"` without native DLL path | 1 |
| `dotnet test bindings\dotnet\WellfriendPdf.Tests\WellfriendPdf.Tests.csproj --filter FullyQualifiedName~RasterRenderingUsesTheCanonicalNativeSurface -m:1 --logger "console;verbosity=minimal"` with `.work\final-universal-renderer-implementation\cargo-target\debug` on `PATH` | 0 |
| `javac --enable-preview --release 25 -d .work\final-universal-renderer-implementation\javac-cache-smoke bindings\java\src\main\java\io\wellfriendpdf\WellfriendPdf.java bindings\java\src\test\java\io\wellfriendpdf\WellfriendPdfSmokeTest.java` | 0 |
| `javac --enable-preview --release 25 -d .work\final-universal-renderer-implementation\javac-package-smoke bindings\java\src\main\java\io\wellfriendpdf\WellfriendPdf.java bindings\java\package-smoke\PackageSmoke.java` | 0 |

## Latest SDK feature-report deferred-validation wording gates

This continuation aligns the public SDK feature report with current source
truth. Renderer categories that previously used slash-later buckets are now
split into source-active typed-limit categories and explicit deferred external
runtime-validation categories. Progressive viewer queue/callback and
cancellation wording also reports source-exposed C/Python/WASM/.NET/Java/server
surfaces while leaving external runtime matrix validation deferred. The focused
test serializes the report and guards against reintroducing the old stale
phrases.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine font_signature_decode_dedup_feature_envelopes --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest image capability unavailable-wording gates

This continuation updates runtime image-decode capability wording so codec-native ROI, codestream-tile, and progressive pixel output are reported unavailable when the selected codec adapter lacks native partial support. This is a reporting accuracy slice; it does not claim new native decoder support.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest runtime deferred-validation wording gates

This continuation updates runtime capability text and server assertions so source-active binding/cache/concurrency surfaces are not reported as implementation-incomplete merely because external host/runtime matrix proof is deferred. Real source gaps remain marked separately in the renderer reports.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server capabilities_endpoint_exposes_renderer_cache_pressure_and_concurrency_policy --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-server --tests --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-server --tests --jobs 1 -- -D warnings` | 0 |

## Latest Type 3 runtime capability reporting gates

This continuation aligns `runtime.rs::runtime_capabilities_for` and the runtime fallback matrix with the retained Type 3 CharProc subplan implementation already present in `render/page_renderer.rs`. Runtime reporting now names the bounded immutable retained CharProc plan cache, contract/resource/source-keyed sublists, cached retained-plan refusals, and source-marker pruning instead of advertising the retained sublist state machine as incomplete.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type3_retained_charproc --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type3_program_cache --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest runtime image-decode capability reporting gates

This continuation aligns `runtime.rs::runtime_capabilities_for` with the bounded progressive image decode lifecycle implemented in `render/image_decode_planning.rs`. The image-decode capability entry now reports that native source-window, native reduction, or native `requires_component_decode` work completes as `planned_partial_decode_complete`, while codec families without native partial/progressive pixel output stay explicit unavailable capabilities.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest alpha-mask fused clip-window allocation gates

This continuation removes a per-scanline allocation from `render/buffer.rs::AlphaMask::fused_clip_window`. The helper now allocates one reusable clip-opacity row for the bounded fused window, fills it for each destination row, and preserves the existing `wellfriendpdf_render_simd::multiply_alpha_rows` path before falling back to scalar multiplication. This keeps clip + soft-mask/image-mask fusion on bounded row storage instead of allocating a fresh `Vec<u8>` for every scanline.

This is a hot-path allocation closure for the existing bounded mask fusion primitive. It does not claim universal concrete-clip elimination or broader transparency semantic closure.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_mask_fused_clip_window --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest transparency reusable backdrop-surface gates

This continuation reduces a remaining bounded transparency-group allocation and clone boundary in `render/page_renderer.rs` and `render/buffer.rs`. `PixelBuffer::copy_rect_into_buffer` now copies a bounded source window into an already-owned destination surface without replacing that destination's clip, soft-mask, or knockout state, so pooled offscreen buffers can be reused for backdrop snapshots. `PixelBuffer::take_knockout_backdrop` lets a stored knockout backdrop be extracted and returned to the renderer's offscreen pool instead of being dropped through `clear_knockout_backdrop`. `RenderState::handle_do_form_group` now uses the existing offscreen pool for non-isolated group backdrop windows, for the copied non-isolated group buffer, and for the knockout backdrop seed; those surfaces are recycled after the group composite.

This is a source-level surface-lifetime closure for bounded Form transparency groups. It does not claim full PDF group colour-space, backdrop, print, or complete knockout semantic closure.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine --lib copy_rect_into_buffer_reuses_existing_surface --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib knockout_backdrop_can_be_extracted_for_reuse --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib transparency_group --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib knockout --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |

## Latest display-list active clip-DAG replay gates

This continuation closes a retained display-list replay gap where `CpuRenderDevice` rebuilt saved clip state by reclassifying the current materialized `PixelBuffer` mask on every graphics-state save. `render/display_list.rs` now carries an active `Arc<ClipNode>` beside the raster buffer, pushes that node directly on `q`, composes new `W` path clips through `ClipDag::intersect`, and reinstalls the saved node on `Q`. Rectangle clips route through the DAG rectangle constructor, general path clips intern their rasterized mask once as a node, and the buffer is synchronized from the installed node through a single helper. The focused regression asserts that save/restore reuses the same active clip-DAG node rather than a reclassified mask.

This is a retained-replay source slice. `PixelBuffer` still owns concrete masks at paint boundaries, and broader active page-renderer clip adoption plus full clip/mask fusion remain tracked separately.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib cpu_render_device_save_restore_reuses_active_clip_dag_node --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine --lib display_list --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |

## Latest active RenderState clip-DAG replay gates

This continuation carries the same active-node model into `render/page_renderer.rs::RenderState`. The page/content, retained display-list, packed-plan, Form cleanup, annotation appearance, tiling-pattern, shading-pattern, Type 3 CharProc, SMask group, and transparency/Form group paths now save, compose, clear, and restore the active `Arc<ClipNode>` through `RenderState::current_clip` instead of treating `PixelBuffer::clip_mask()` as the logical clip source. Explicit render-contract identity refresh re-interns the current buffer clip into the new scoped DAG so revision/contract/tile clip identity remains aligned after contract changes. The only direct buffer synchronization point in active render state is now `install_clip_node`.

This removes the remaining active-renderer save/restore mask reclassification sites found in this pass. It still does not remove the concrete `PixelBuffer` mask required by current paint kernels.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine --lib q_restore_restores_previous_clip_mask --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib content_path_clip_replay_reuses_transformed_clip_node_cache --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib clip_dag --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib display_list --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |

## Latest clip-DAG dimension-keyed materialization gates

This continuation closes a stale-mask risk in persistent clip-DAG nodes. `render/clip_dag.rs::ClipNode` now stores lazily materialized `ClipMask`s in a dimension-keyed cache rather than a single `OnceLock<ClipMask>`. Dimensionless structural states such as `Full`, `Empty`, and `Rectangle` can therefore be materialized for multiple destination sizes without returning a mask from a previous size. Same-dimension calls still reuse the cached `Arc<ClipMask>`, `approximate_bytes` accounts for all realized dimension entries, and the display-list restore path clones from the returned `Arc` without changing the saved-state sharing model.

This is a source-correctness slice for persistent clip reuse. It does not perform corpus rendering or claim broader transparency/clip corpus verification.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine clip_dag --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest image XObject color-space/role cache identity gates

This continuation closes an image cache identity hole in `render/page_renderer.rs`. `image_xobject_cache_key` now includes the resolved image color-space string plus the image-mask and soft-mask roles in addition to object identity, dimensions, bits-per-component, and filter chain. Same-object decode entries can no longer collide when the renderer sees the image through a different color-space family or mask role in the same render context; color-space override objects remain hashed through the existing override path. The helper's inline-image branch now records bits-per-component, filter chain, color-space, mask roles, payload length, and a payload hash so any shared path that receives an inline `ImageReference` is not keyed by size and length alone.

This is a cache-key source-correctness slice. It does not add fake native region/progressive decode capabilities.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_cache_key --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_cache_key --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest transaction write-set binding alias gates

This continuation narrows transaction-driven renderer invalidation for binding/server JSON producers. `render/transaction_invalidation.rs` now matches render write-set field names and structured object-reference keys using case/separator-insensitive aliases. Existing snake_case keys such as `changed_font_refs`, `changed_output_intent_refs`, `render_write_set_refs`, `object_number`, and `generation_number` still work, while camelCase equivalents such as `changedFontRefs`, `changedOutputIntentRefs`, `renderWriteSetRefs`, `fontObject`, `fontGeneration`, `objectRef`, and `sourceRef` now map into canonical source IDs instead of being missed and forcing broader cache reset behavior. The typed `RenderInvalidationCachePlan` boundary now also accepts binding-style direct plans and envelopes, including `schemaVersion`, `nextRevision`, `mappedSourceIds`, `sourceCacheMarkers`, `sourceId`, `objectNumber`, `generationNumber`, `affectedPages`, `affectedTiles`, `pageNumber`, `conservativeResetRequired`, and `report.renderInvalidation`.

The public `TransactionWriteSet` serde boundary now accepts `affectedObjectRefs`, `affectedObjects`, `affectedPages`, `affectedTiles`, and `nextRevision`; `affectedTiles` accepts both the existing Rust tuple shape and binding-style `{pageNumber, tile}` objects. Dirty-region to render-tile conversion now accepts binding-style page and bounds aliases such as `pageNumber` and `dirtyRegion`, so non-Rust transaction reports can still produce exact tile invalidation when the caller supplies the matching viewport. This is a transaction write-set extraction/application slice. It does not claim arbitrary unknown references can be narrowed; unmapped refs still correctly trigger conservative reset.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nested_write_set_collector_accepts_camel_case_binding_refs --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nested_write_set_collector --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_invalidation_plan_json_maps_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_invalidation_plan_json_accepts_camel_case --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_invalidation_plan_json_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_write_set_json --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine dirty_regions --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest image decode render-contract identity gates

This continuation closes a decoded-image cache identity hole in `render/image_decode_planning.rs` and `render/page_renderer.rs`. `ImageDecodeCacheKey` now carries the active render-contract fingerprint through `ImageContractState` and serializes it as a stable `:contract:` cache-key fragment. Inline-image and Image XObject planning now pass the active `RenderState` render-contract fingerprint into the planner, so decoded image cache entries cannot collide across document revision, output policy, CMM/overprint, tile/viewport, backend, or resource-budget identities that are already represented by the render contract. The planner's target format, backend, and contract identity are grouped in `ImageDecodePlanIdentity` to keep the call boundary lint-clean.

This is a cache-correctness and source-identity slice. It does not add fake native JPEG/JPX/JBIG2 region or progressive pixel output; those codecs continue to report their current decoder API limits explicitly.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine cache_key_includes_render_contract_identity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest packed Form XObject metadata gates

This continuation tightens resource-aware packed Form replay. `ResolvedXObjectHandle` already carried the Form XObject BBox and Matrix parsed from `PageResources`; `render/page_renderer.rs` now threads those pre-resolved values through the packed XObject replay metadata and into the cached Form XObject program builder. Packed Form replay therefore uses the compiled handle metadata when available and only falls back to stream-dictionary extraction when a legacy/non-resource-aware caller lacks the handle fields. The descriptor test now asserts both BBox and Matrix retention.

This is a bounded packed-plan pre-resolution slice. Form stream decode/content parsing still correctly uses the Form source object and cache identity; broader universal packed payload closure remains incomplete.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine resource_aware_plan_pre_resolves_high_level_handles --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_replay_refuses_unresolved_resource_descriptors --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest knockout group row-replacement gates

This continuation closes a bounded transparency-group flattening boundary in `render/buffer.rs`. `PixelBuffer::knockout_from` no longer walks the destination pixel-by-pixel for same-origin knockout group replacement. It now routes through a row helper that slices destination/source rows, applies the same source-alpha, group-alpha, soft-mask, and clip-opacity calculation as the old loop, preserves replacement RGB semantics for transparent soft-mask output, skips clip-zero pixels, and reports `knockout_row_pixels` through `PixelCompositorStats` and CLI compositor telemetry.

This is a source hot-path closure for existing knockout replacement semantics. It does not claim full PDF knockout/backdrop/group-colour-space closure, non-device group-space compositing, or corpus validation.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine knockout_from_fuses_partial_clip_and_soft_mask_into_row_path --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1 -- -D warnings` | 0 |

## Latest LCD glyph clip/SMask row-fusion gates

This continuation closes a supported glyph-mask paint boundary in `render/buffer.rs`. Normal Compat LCD/subpixel glyph masks no longer dispatch every painted pixel through the general `blend_lcd_pixel` path when clip and soft-mask state are active. `PixelBuffer::blend_lcd_alpha_mask_strided` now slices each bounded LCD row once, computes the same red/green/blue subpixel coverages, fuses installed clip opacity and SMask bytes in row-local code, writes the destination row directly with the same source-over math as the old pixel oracle, and reports `lcd_row_pixels` in `PixelCompositorStats`. The old per-pixel LCD helper is retained only as a test oracle.

This is a source hot-path closure for LCD glyph masks. It does not claim universal concrete-mask elimination, non-normal LCD blending, high-quality LCD policy changes, or corpus/runtime font validation.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine lcd_alpha_mask_fuses_partial_clip_and_smask_into_row_path --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest transaction transitive source-dependency gates

This continuation tightens the renderer dependency graph and source-scoped artifact pruning. `RenderDependencyGraph` now records source-to-source dependency edges and expands a changed source through every dependent source before computing invalidated pages/tiles. `RenderDocumentCache` uses the same expanded source set when pruning source-scoped raw/scaled image, SMask, shading, Form, tiling-pattern, annotation-appearance, and Type 3 artifact caches, so a nested shared-resource edit also evicts parent retained resource programs whose cache keys are tied to the parent object.

The retained display-list tile resource walker now records those parent-to-child source edges while it walks bounded resource dependencies for Forms, images, shadings, patterns, fonts, color spaces, and ExtGState state. The focused Form XObject test now proves a nested image edit invalidates the intersecting tile and prunes the cached parent Form program instead of leaving it behind with a stale parent marker.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine shared_resource --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine bounded_form_xobject_records_transitive_resource_tile_dependency --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest C/Python/WASM render-contract builder parity gates

This continuation closes a binding-source parity gap in the schema-v1 render-contract builders. C previously exposed only surface, clip, device-transform, background, and resource-budget builder calls, while Python and WASM mirrored the same limited set. The C ABI/header now expose `wellfriendpdf_render_contract_with_schema_policies`, which can replace every remaining non-identity schema policy field with nullable string inputs: page box, execution mode, backend, compositing, annotations, forms, optional-content identity, text/image/path/subpixel smoothing, color scheme, print profile, halftone, overprint, rendering intent, color-management policy, exactness, and determinism. Python and WASM now expose method-level builders for the same fields, and the TypeScript declarations name the accepted schema enum families.

All three bindings continue to validate through the canonical Rust `RenderContract::validate` path after each builder change. The new enum parser accepts canonical schema variant names plus common lowercase kebab/snake forms, so callers do not need to hand-edit raw JSON to build a full-field contract.

| Command | Exit |
|---|---:|
| `cargo check -p wellfriendpdf-capi --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_render_contract_schema_policy_builder_round_trips --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-py --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --lib --jobs 1` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo clippy -p wellfriendpdf-capi --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-py --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-wasm --lib --jobs 1 -- -D warnings` | 0 |

## Latest scaled image decode reservation and renderer marker gates

This continuation closes a reduced-decode resource-accounting gap in `render/page_renderer.rs`. Native scaled JPEG and JPX XObject decode scheduling now reserves memory from the requested reduced target dimensions instead of taking the maximum of the reduced target and the full uncompressed source image. This keeps exact native reduced decode from being rejected by the scheduler merely because the original source image is large, while the actual decoder still enforces stream and decoded-image limits. Inline scaled JPEG/JPX decode already used the target-size reservation path.

The packed descriptor arena also no longer retains raw operand vectors for unsupported state operators. `GraphicsStateDescriptor::Unsupported` now carries only the operator needed to emit `PackedCompileRefusal::UnsupportedStateOperator`, so fail-closed unsupported-state handling does not keep a raw operation payload in the active descriptor arena.

Stale `DisplayListStats` compatibility-run counters were removed because retained `BX`/`EX` handling is represented as typed state descriptors or malformed-sequence refusals, not as replay fallback runs. The tile-stitch display-list test now asserts the current tile-scoped transparent-page-group cache identity: four stitched tile renders create four tile-specific group-decision entries while sharing one display-list entry and preserving pixel equality.

The renderer source marker audit also no longer reports `TODO`, `FIXME`, `stub`, `placeholder`, `not implemented`, `unimplemented!`, `todo!`, `immediate renderer`, `legacy renderer`, or `allow(dead_code)` hits in the searched renderer/image/font/SIMD/runtime scopes. ExtGState `/SA`, `/AIS`, and `/TK` visible-paint boundaries remain explicit typed exact-policy refusals, with runtime messages now describing policy boundaries instead of implementation stubs.

| Command | Exit |
|---|---:|
| `rg -n "TODO|FIXME|HACK|TEMP|placeholder|stub|not implemented|unimplemented!|todo!|solid fallback|immediate renderer|legacy renderer|allow\\(dead_code\\)" crates\\engine\\src\\render crates\\engine\\src\\images crates\\engine\\src\\fonts crates\\render-simd\\src crates\\engine\\src\\runtime.rs` | 1 (no matches) |
| `cargo test -p wellfriendpdf-engine scaled_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine extgstate_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine unsupported_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine compile_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine graphics_state_descriptor_unknown_operator_is_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine display_list_tile_stitch_matches_full_page --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest runtime font capability and clip-DAG materialization gates

This continuation updates source capability disclosure and a bounded persistent-clip edge case. `runtime.rs` now reports an explicit `font_resolution_and_substitution_reporting` capability for the active font resolution order: embedded/document, caller-registered provider, deterministic system mapping, bundled fallback, and typed missing-font refusal. The entry names the registered-provider fingerprint in font-policy/cache identity, deterministic-system resolution source, high-quality/exact acceptance of registered or deterministic-system faces, generic bundled refusal, and Rust/server/C/Python/WASM/.NET/Java/CLI source surfaces.

`render/clip_dag.rs` also no longer materializes dense partial-coverage clip nodes as an empty mask when the requested output size differs from the stored dense-mask dimensions. The materializer now crops the overlapping source alpha rows and pads outside the source extent as clipped pixels, preserving visible partial coverage instead of silently losing the clip.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine materialize_dense_alpha_size_mismatch_preserves_overlap --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest packed text font-resolution gates

This source slice tightens resource-aware packed text replay. `GraphicsStateDescriptor::ApplyExtGState` now carries an optional pre-resolved font handle for well-formed ExtGState `/Font` entries, and resource-aware packed compilation emits typed refusals for missing ExtGState-selected fonts. `PackedDisplayList::compile_with_resources` tracks active resolved text-font state across packed `q/Q` and refuses visible text-showing operators before a pre-resolved font is active, instead of compiling them into paintable text descriptors that would later depend on fallback resource lookup. Packed replay now saves/restores active pre-resolved font/color/pattern resources alongside graphics, clip, and soft-mask stacks, so `Q` restores the hot replay resource state rather than clearing it. Stale or manually constructed packed ExtGState descriptors that carry a well-formed `/Font` entry without a pre-resolved font handle now fail typed at replay before mutating text state.

This closes a packed-plan pre-resolution hole for text and ExtGState font selection without claiming universal packed coverage for every remaining high-level operation.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib packed_retained_text --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib packed_text_font_tracking_restores_unresolved_state_after_q --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib packed_ext_g_state --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib packed_plan_restore_restores_pre_resolved_active_font_resource --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib packed_plan_replay_refuses_unresolved_state_resource_descriptors --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- crates/engine/src/render/plan.rs crates/engine/src/render/page_renderer.rs` | 0 |

## Latest explicit contract backend-plan arena gates

This source slice extends the render-view backend-plan arena report from default display-page reporting to explicit schema-v1 render-contract reporting. `BackendPlanArenaReport` now includes contract arena kind, execution mode, backend selection, print profile, and output-surface fields; `RenderDocumentView::backend_plan_arena_report_for_contract` validates the supplied contract and compiles the same page through the render view without constructing edit, semantic, or validation views. Print and proof contracts now report distinct CPU packed-plan arena identities through the Rust SDK, CLI `--contract-json`, server `contract_json`, C ABI/header, Python, WASM/TypeScript, .NET, and Java source surfaces. This closes the source-level print/proof contract-plan observability gap; it does not claim GPU/debug backend-native arenas, separation-preserving print raster output, or external runtime binding matrices.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib render_view_exposes_print_contract_backend_plan_arena_report --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib backend_plan_arena_report_accepts_explicit_print_contract --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib backend_plan_arena_report --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-server --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-capi --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-py --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-cli backend_plan_arena_report_command_emits_json --test tool_surface --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server render_contract_backend_plan_arena_report_route_returns_json --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `dotnet build bindings\dotnet\WellfriendPdf.Tests\WellfriendPdf.Tests.csproj --nologo --verbosity minimal` | 0 |
| `javac --release 25 --enable-preview -d .work\final-universal-renderer-implementation\java-classes bindings\java\src\main\java\io\wellfriendpdf\WellfriendPdf.java` | 0 |
| `javac --release 25 --enable-preview -cp .work\final-universal-renderer-implementation\java-classes -d .work\final-universal-renderer-implementation\java-classes bindings\java\src\test\java\io\wellfriendpdf\WellfriendPdfSmokeTest.java` | 0 |
| `javac --release 25 --enable-preview -cp .work\final-universal-renderer-implementation\java-classes -d .work\final-universal-renderer-implementation\java-classes bindings\java\package-smoke\PackageSmoke.java` | 0 |

## Latest adaptive progressive scheduler policy gates

This source slice makes adaptive progressive tile sizing depend on deterministic render-plan and contract policy inputs instead of page area plus a default budget alone. `ProgressiveTileSchedulerReport` is now carried in progressive tokens and step reports, recording fixed/adaptive selection mode, requested and selected tile dimensions, supported sizes, page pixels, temporary-memory budget, render contract fingerprint, execution mode, backend, print profile, output surface, packed hot/descriptor/native-batch counts, compile-refusal count, image/clip/transparency/path complexity signals, pressure reasons, and the final selection reason. Adaptive requests still choose from `128`, `192`, `256`, `384`, and `512`, but now step down deterministically for high-quality mode, print/proof profiles, scalar backend, transparency, image density, clip pressure, and plan-complexity tiers. Fixed tile requests remain caller-selected and are reported as fixed. The same JSON flows through existing Rust, server, C/Python/WASM/.NET/Java progressive surfaces without adding independent binding schedulers. This closes a source-level adaptive scheduling observability gap; external viewer runtime matrices remain future validation work.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib adaptive_tile_size --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib adaptive_scheduler --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib zero_tile_dimension_selects_adaptive_size --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib step_report_includes_adjacent_page_prefetch_and_viewer_queue_preview --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server progressive_start_accepts_adaptive_tile_size --test progressive_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-capi --lib --jobs 1` | 0 |

## Latest clip-DAG soft/image alpha window fusion gates

This source slice adds `ClipAlphaFusionWindow` and `ClipDag::fuse_alpha_mask_window`, a bounded-window DAG entry point for fusing existing clip nodes with alpha-bearing soft masks or image/stencil masks. The helper materializes at most the requested source window, combines alpha using the same rounded div-255 opacity math as paint-time soft-mask/clip fusion, interns the destination-local result with the active revision/render-contract/tile identity scope, simplifies all-empty/all-full/binary outputs back to cheaper structural states, and short-circuits empty clips to the existing empty flyweight. This closes another source-level clip + soft/image mask fusion boundary without claiming codec-native mask decode or universal paint-boundary fusion.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_mask_window --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine binary_image_mask_window_simplifies_to_structural_clip_node --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest clip-DAG dense alpha fusion gates

This source slice extends `ClipState::intersect_structural` so dense alpha clips intersect structurally with rectangles, binary row masks, RLE masks, and other dense alpha masks instead of falling back to composite materialization. Dense/dense intersections preserve PDF alpha semantics with bytewise minimum coverage; dense/rectangle and dense/binary intersections preserve existing alpha inside visible coverage and zero outside it. The alpha-byte classifier simplifies all-empty, all-full, and binary results back to the cheaper clip representations before retaining a `DenseMask`, so save/restore and repeated clip intersections keep using DAG nodes rather than cloned full masks where the result is still structural.

The same pass repaired the stale `exact_contract_refuses_visible_image_requiring_unavailable_region_decode` fixture so it uses a real DCT/JPEG filtered image. The earlier raw-image fixture no longer represented an unavailable source-region decode path because guarded raw source-window decode is now implemented.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine dense_rectangle_intersection_fuses_dense_without_materialization --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine dense_dense_intersection_fuses_min_alpha_without_materialization --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine dense_binary_intersection_fuses_dense_without_materialization --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine exact_contract_refuses_visible_image_requiring_unavailable_region_decode --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest ResearchHybrid image decode target-format gate

This source slice aligns the active image decode target-format helper with the renderer behavior documented by the prior cache-identity slice. `ImageDecodeTargetFormat::for_backend` now maps `StandardCpu`, `ScalarReference`, and active CPU `ResearchHybrid` contracts to the current `raw-image-8-interleaved` decoded output, while `ImageDecodeBackendIdentity` still keeps separate `standard-cpu`, `scalar-reference`, and `research-hybrid` cache-key fragments. The stale unimplemented `rgba8-premultiplied` target enum value was removed instead of kept as dead code, so active cache keys name only decoded target shapes the renderer actually produces.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine cache_key_includes_target_format_and_backend_identity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest render-contract research execution-mode gates

This source slice makes active CPU render contracts accept `ExecutionMode::Research` as an identity-bearing CPU contract field. `engine.rs` now preserves `Research` execution mode in accepted contract comparison and the render-contract cache fingerprint while still routing through the local deterministic CPU renderer. The focused negative test now uses a mismatched `page_identity` to keep the active CPU field-list diagnostic covered without treating research execution mode as unsupported. This does not enable research-only GPU, distributed, solver, or autotuning runtime capabilities; those remain governed by the runtime capability policy matrix.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine contract_research_execution_mode_renders_with_distinct_identity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_reports_unsupported_semantic_fields_by_name --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest render-contract research-hybrid backend gates

This source slice makes active CPU render contracts accept `BackendSelection::ResearchHybrid` as a guarded CPU hybrid-dispatch backend. `engine.rs` now keeps `ResearchHybrid` in the accepted contract identity, while `ScalarReference` remains the only backend that installs the scoped scalar-compositor guard and declines SIMD/portable-wide row helpers. `ResearchHybrid` uses the active guarded CPU dispatcher, keeps a distinct render-contract cache fingerprint, and continues to flow through retained plan contracts plus image decode backend/cache identity. The later research execution-mode slice moves the negative unsupported-contract diagnostic to a mismatched `page_identity`, so field-list diagnostics still fail typed without treating `ResearchHybrid` or `ExecutionMode::Research` as unsupported. This does not implement GPU, printer, debug-device, or fully backend-native packed payload backends.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine contract_research_hybrid_backend_renders_with_distinct_identity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_reports_unsupported_semantic_fields_by_name --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server render_contract_builder_accepts_research_hybrid_backend --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest prepress plate report exposure gates

This source slice wires the existing active render-interpreter Prepress CMM Separation/DeviceN plate framebuffer report through public source surfaces. `sdk::prepress_plate_report_json` now wraps `ContentEngine::prepress_plate_report` in the shared versioned SDK envelope; CLI `prepress-plate-report`, server `POST /api/v1/prepress/plate-report`, C ABI/header, Python, WASM/TypeScript, .NET, and Java source shims expose the same report. The report records deterministic plane order, plate summaries, tint/alpha/provenance contribution counts, bounded memory accounting, and plate cache fingerprint data for supported `/Separation` and `/DeviceN` fill/stroke paths. This does not claim full CMYK/DeviceN separation-preserving print output, native CMM runtime validation, or certification-grade proofing; those remain explicit print/CMM limits.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine prepress_plate_report_exposes_separation_framebuffer_shape --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-cli prepress_plate_report_command_emits_json --test tool_surface --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server prepress_plate_report_route_returns_json --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-capi --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-py --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-cli --test tool_surface --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-server --test server_integration --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-capi --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-py --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1 -- -D warnings` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --nologo /m:1 -v:minimal -p:UseSharedCompilation=false` | 0 |
| `javac -J-Xmx1024m --enable-preview --release 25 -d .work/final-universal-renderer-implementation/java-classes bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java` | 0 |
| `javac -J-Xmx1024m --enable-preview --release 25 -cp .work/final-universal-renderer-implementation/java-classes -d .work/final-universal-renderer-implementation/java-classes bindings/java/package-smoke/PackageSmoke.java` | 0 |

## Latest structured shared-resource invalidation gates

This source slice extends render-invalidation plan application for shared renderer resources carried in full SDK/server/report envelopes. `render/transaction_invalidation.rs` now normalizes nested write-set refs encoded as strings, `[object, generation]` arrays, `{object_number, generation}` objects, and `{ref: ...}`/`{object_ref: ...}`/`{source_ref: ...}` records before applying the plan to a caller-owned `RenderDocumentCache`. Known structured font/image/Form/shading/pattern/resource refs merge into mapped source IDs and source-cache marker entries; unknown structured refs still force the conservative reset path. This narrows binding/server cache-host invalidation for structured shared-resource reports without claiming the broader arbitrary shared-resource matrix is complete.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest strict vector-output fallback gates

This source slice separates legacy compatibility vector export from high-quality/exact vector policy. `render_page_svg` and `render_page_ps` keep explicit `is_rasterized` compatibility output when a caller wants a usable SVG/PS artifact for unsupported local vector constructs. New strict entry points `render_page_svg_strict`, `render_page_ps_strict`, `ContentEngine::render_page_svg_strict`, `ContentEngine::render_page_ps_strict`, `render_document_ps_strict`, and `render_page_eps_strict` refuse whole-page raster fallback with typed `UnsupportedFeature`. The CLI `render` command now exposes this policy through `--strict-vector` for `--format svg`, `--format ps`, and `--format eps`, reports `strict_vector` in JSON output, and rejects `--strict-vector` on raster output instead of silently ignoring it. Runtime `renderer_fallback_policies` now reports zero material-degrading high-quality rows and zero high-quality canonical-immediate routes; compatibility font replacement remains explicitly reported but high-quality/exact paths refuse generic bundled replacement.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine strict --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-cli render_strict_vector --test tool_surface --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-cli --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-cli --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest transparent-blend no-paint gates

This source slice narrows a remaining transparency/regionalization boundary without changing strict refusal semantics. `render/vector_fallback.rs` first removed the fully transparent non-normal blend fallback when both stroking and nonstroking alpha constants were explicitly zero, and `render/page_renderer.rs` uses the same complete no-paint test before deciding whether a top-level transparent page backdrop is needed. The later stateful classifier extension keeps supported PostScript alpha/blend ExtGState metadata until a paint operation proves whether that state is visible: exact no-paint fills/strokes with the relevant alpha at zero remain native no-ops even if the unused alpha channel is fractional or the blend mode is non-normal, while visible fractional-alpha or non-normal-blend paint still fails closed to the typed whole-page raster/refusal boundary.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine transparent_page_group_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ext_gstate --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest invalid exact-tile guard gates

This source slice closes a stale-cache edge in transaction-driven invalidation. `render/transaction_invalidation.rs` now ignores page-0 or zero-sized affected tiles before selecting page-artifacts-plus-exact-tiles invalidation. A malformed binding-safe render-invalidation plan can no longer satisfy "this affected page has exact tiles" with a tile that cannot invalidate any raster pixels; affected pages fall back to page-wide recorded tile invalidation when exact tile coverage is invalid or absent.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine zero_sized_explicit_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_invalidation_plan_json_zero_sized_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine exact_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest render-contract scalar-reference backend gates

This source slice implements active CPU render-contract `BackendSelection::ScalarReference` instead of exposing it as a builder value that was refused before rendering. `render/buffer.rs` now has a scoped scalar-compositor guard; while an explicit contract selects `ScalarReference`, public compositor backend reporting returns `scalar`, architecture SIMD calls and portable-wide row helpers decline, and existing scalar row compositors perform the paint. At this historical point `engine.rs` still refused `ResearchHybrid`; the later research-hybrid backend gate above makes it active through the guarded CPU dispatcher with separate contract/cache identity. `render/page_renderer.rs` carries the backend policy into retained plan contracts, active `RenderState`, annotation replay, child SMask/transparency-group states, SMask cache identity, and inline/Image XObject decode planning/cache identity. `image_decode_planning.rs` exposes the existing backend target formatter to the renderer, while the default helper remains test-only. Separation-preserving overprint remains explicit typed unsupported contract semantics where no source implementation exists. No PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine contract_scalar_reference_backend_uses_scalar_compositor --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_reports_unsupported_semantic_fields_by_name --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest render-contract path-smoothing gates

This source slice implements active CPU render-contract `path_smoothing` for `Disabled`, `Antialiased`, and `Subpixel`. The explicit contract path now accepts those values, carries the policy into the retained plan contract fingerprint, active `RenderState`, annotation replay policy, and child soft-mask/transparency-group states, and salts reusable path fill/stroke mask cache keys with binary-alpha state. `Disabled` thresholds cached path fill/stroke masks, keeps the direct RGB path fallback on binary scanline coverage, and routes CMYK overprint-preview path coverage through a binary scanline compositor; `Antialiased` preserves the existing coverage path; and `Subpixel` uses the existing device-space subpixel antialias coverage path. No PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine contract_path_smoothing_disabled_thresholds_fill_edges --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_reports_unsupported_semantic_fields_by_name --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_text_smoothing_disabled_thresholds_glyph_edges --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_image_smoothing_disabled_overrides_interpolate_true --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- crates/engine/src/engine.rs crates/engine/src/render/page_renderer.rs crates/engine/src/render/path.rs crates/engine/src/render/display_list.rs` | 0 |

## Latest render-contract text-smoothing gates

This source slice implements active CPU render-contract `text_smoothing` for `Disabled`, `Antialiased`, and `Subpixel`, plus the separate `subpixel_text` policy. The explicit contract path now accepts those values, carries the policy into the retained plan contract fingerprint, active `RenderState`, annotation replay policy, and child soft-mask/transparency-group states, and salts device glyph-mask cache keys with binary-alpha state. `Disabled` thresholds cached glyph alpha masks and routes direct ordinary/color glyph fill fallback paths through binary scanline coverage, `Antialiased` preserves the existing coverage path, and subpixel text paints cached or direct glyph masks through an RGB LCD-style per-channel compositor. No PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine contract_text_smoothing_disabled_thresholds_glyph_edges --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_image_smoothing_disabled_overrides_interpolate_true --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_reports_unsupported_semantic_fields_by_name --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- crates/engine/src/engine.rs crates/engine/src/render/page_renderer.rs crates/engine/src/render/path.rs` | 0 |

## Latest render-contract image-smoothing gates

This source slice implements active CPU render-contract `image_smoothing` for `Disabled`, `Antialiased`, and `Subpixel`. The explicit contract path now carries the smoothing policy into the retained plan contract fingerprint, active `RenderState`, annotation replay policy, and child soft-mask/transparency-group states. Inline images and Image XObjects still validate malformed `/Interpolate` metadata first, then `SmoothingPolicy::Disabled` forces nearest-neighbour sampling while `Antialiased` and `Subpixel` preserve the PDF `/Interpolate` behavior. No PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine contract_image_smoothing_disabled_overrides_interpolate_true --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_reports_unsupported_semantic_fields_by_name --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- crates/engine/src/engine.rs crates/engine/src/render/page_renderer.rs` | 0 |

## Latest packed culled text advancement gates

This source slice removes the retained replay no-op used when packed text bounds were outside the active viewport. Non-clipping culled `Tj`, `TJ`, `'`, and `"` descriptors now call a no-paint `RenderState` text-advance path that decodes through the active font resolver, applies PDF/font/Type 3 advance metrics, honors character and word spacing, and preserves `TJ` adjustments before returning without paint. Text clipping modes continue through normal replay because they can mutate clip state. This closes a source gap where one offscreen text run could leave later visible text at the wrong matrix position. No PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine packed_culled_text --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_culled_spacing_next_line_show_advances_text_state_without_painting --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- crates/engine/src/render/page_renderer.rs docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md docs/renderer/final-local-implementation-closure.md docs/renderer/final-fallback-closure-report.md` | 0 |

## Latest scan-converter convex telemetry gates

This source slice exposes the existing guarded convex sampled-span fill counter through `PathRasterStats::convex_fast_pixels` and includes `convex_fast_pixels_delta` in both CLI render telemetry summary shapes. The convex fill fast path was already byte-checked against the general scanline compositor; this makes its source invariant externally visible to renderer telemetry and later verification harnesses instead of private to the path module. The same CLI check also exposed and repaired one retained-list API fallout: `render_page_display_list_with_mode` now returns a `PixelBuffer` on success, so the display-list report path wraps the successful render in `Some(...)` while preserving typed unsupported errors.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine convex_polygon_fill_fast_path_matches_general_scanline --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine concave_polygon_fill_rejects_convex_fast_path --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- crates/engine/src/render/path.rs crates/cli/src/main.rs docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md docs/renderer/final-local-implementation-closure.md` | 0 |

## Latest scan-converter invariant gates

This source slice extends the scanline invariant suite with direct private-boundary coverage for active-edge lifecycle and scratch-pool reuse. `scanline_active_edges_drop_ended_edges_before_later_rows` proves ended edges are removed before later rows publish crossings, and `scanline_scratch_pools_return_empty_bounded_vectors` proves crossing scratch returns empty while `u16` scanline accumulators return zero-filled to the requested length. These tests complement the existing crossing filtering/sorting, sequential-row, bucket parity, subsample parity, convex/concave fast-path, and stroke fast-path equivalence tests.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine scanline_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest PostScript Type 2 Range gates

This source slice keeps finite Type 2 function `/Domain` metadata and finite unit `/Range` metadata on exact PostScript Type 2 shading-function sidecars for direct DeviceRGB, DeviceGray, and DeviceCMYK shadings, component-function arrays, and Type 3 stitching segments whose nested Type 2 functions carry finite domains or ranges. Same-exponent component arrays collapse only when the function domains match and the range set can be represented exactly as a multi-output `/Range`; mixed-domain, mixed-exponent, or partially ranged arrays serialize as exact per-component PostScript function arrays. The PostScript writer serializes the preserved `/Domain` and `/Range` into native `shfill` functions, so those local nonlinear shadings remain bounded PostScript regional fallbacks instead of forcing whole-page raster output. SVG remains conservative for the nonlinear/ranged cases, and non-unit shading dictionaries, out-of-unit ranges, malformed domains/ranges, mesh shadings, transparency interactions, corpus validation, and external viewer proof remain incomplete.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_type2_function_with_range --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_type2_function_with_non_unit_domain --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_rgb_shading_function_preserves_exact_type2_exponent --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_cmyk_shading_function_preserves_exact_type3_stitching --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_device_rgb_type2_function_array_with_ranges --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_device_cmyk_type2_function_array_with_ranges --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type2_function_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_rgb_shading_function_preserves_exact_type2_component_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_cmyk_shading_function_preserves_exact_type2_component_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest retained-list unsupported replay refusal gates

This source slice removes compatibility fallbacks that published unsupported retained display lists through canonical raw/immediate rendering. Full-page direct, cached, contract, display-list helper, direct display-list replay, and cached display-list replay paths now return typed `UnsupportedFeature` for unsupported retained replay in compatibility mode; high-quality/exact mode keeps the existing `HighQualityExact` refusal. `render_page_display_list_tile_cancellable_with_mode_and_cache` now returns the same typed refusal for unsupported retained tile replay, and public tile helpers plus band rendering propagate it instead of immediate-rendering the tile. `ProgressiveRenderJob::render_next` receives that typed refusal, marks the job failed through the existing error path, and records no fallback tile publication. The structured fallback-event type remains available for diagnostics/tests, but unsupported retained full-page, retained tile, and progressive retained tile replay are no longer counted as active immediate-routing fallbacks.

Follow-up retained replay cleanup: full-page display-list helpers and `render_page_display_list_tile_cancellable_with_mode_and_cache` now return `Result<PixelBuffer>` rather than `Result<Option<PixelBuffer>>`, so callers cannot route unsupported retained replay through a `None` branch. The public tile helper and band renderer call the retained tile path directly, and the private immediate tile helper was removed. Progressive fallback-event test metadata now describes typed refusal rather than canonical immediate tile publication.

CLI corpus follow-up: `wellfriendpdf render-corpus --pipeline display-list` now uses the same fail-closed retained replay policy. Both cache-enabled and cache-disabled `render_corpus_display_list_page_*` branches return typed `UnsupportedFeature` when `DisplayList::is_fully_supported()` is false, include the retained-list refusal reason, and require callers to choose `--pipeline immediate` explicitly if they want immediate rendering. The per-file JSON records a failed file/page and `display_list_fallbacks` remains zero.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_display_list_page_helpers_refuse_unsupported_list --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_display_list_tile_refuses_unsupported_list --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine compat_display_list_bands_refuse_unsupported_list_without_immediate_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_refuses_unsupported_display_list_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine fallback_report_keeps_codes_and_structured_policy_details --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine unsupported_retained --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-cli --test tool_surface render_corpus_display_list_refuses_unsupported_retained_replay --jobs 1 -- --exact --nocapture` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-cli --bin wellfriendpdf --test tool_surface --jobs 1 -- -D warnings` | 0 |

## Latest AcroForm widget invalidation gates

This source slice adds structured AcroForm widget/AP render-invalidation reports to `DocumentSubsystemsOperationReport` for FormData actions and to secure-mutation incremental form value updates. Reports include field, widget, and appearance refs where available, affected pages, dirty widget rectangles for live appearance changes, and value/default deltas. No PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine canonical_form_action_reopens_and_exposes_inverse_proof --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine document_subsystems --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test secure_mutation_closeout_advanced_secure_mutation --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --all-features --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --all-features --jobs 1 -- -D warnings` | 0 |

## Latest progressive session render-invalidation cache-host gates

This source slice adds direct render-invalidation plan application to server-owned and binding-owned progressive render sessions. `ProgressiveRenderJob::apply_render_invalidation_plan_json` applies a plan body or SDK-envelope-derived plan to the session's retained `RenderDocumentCache`, clears only affected retained current-page tile buffers, advances the scheduler/publication identity, and reports obsolete tile-publication identities so callers can reject stale surfaces before presentation. The server exposes the same path through `/api/v1/progressive/:id/apply-render-invalidation` on the owner-scoped session store; C, Python, WASM/TypeScript, .NET, and Java expose it through their existing progressive session handles. This closes the progressive binding/server shared-cache mutation gap for render-invalidation plans; non-progressive binding cache handles and broader transitive/shared-resource write-set closure remain incomplete.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-server --test progressive_integration progressive_apply_render_invalidation_obsoletes_retained_tile_publication --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_apply_render_invalidation_plan_obsoletes_retained_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --nologo /m:1 -v:minimal` | 0 |
| `javac --enable-preview --release 25 -d .work/final-universal-renderer-implementation/java-classes bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --all-features --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --all-features --jobs 1 -- -D warnings` | 0 |

## Latest active clip-DAG identity gates

This source slice wires active page, SMask `/G`, and transparency/Form group `ClipDag` construction to `ClipIdentityScope` derived from document revision, active render-contract fingerprint, and tile-local viewport identity. Explicit contract policy application refreshes the active DAG scope after replacing the legacy fingerprint, so future clip nodes carry non-default revision/contract/tile salts. The tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine clip_identity_scope --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_state_clip_dag_uses_active_contract_scope --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest clip-DAG transitive pruning gates

This source slice changes `ClipDag::evict_unused` from a single retain pass into a same-call transitive prune. When an unused composite node is dropped from the interner, child nodes newly orphaned by that drop are pruned before the call returns. Full/Empty flyweights and externally held save/restore/composite handles remain live. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine clip_dag --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest render-view backend-plan arena gates

This source slice adds `BackendPlanArenaReport` and `RenderDocumentView::backend_plan_arena_report`. The render view now compiles a page display list through `RenderPlan::compile_with_resources` and exposes backend-plan arena counts for hot ops, path/clip/state/bounds arenas, typed descriptors, cold diagnostics, contiguous vector/native batches, native-replay requirements, descriptor-kind counts, and compile-refusal reasons without constructing edit, semantic, or validation views. The same report is source-visible through the shared SDK envelope plus CLI `backend-plan-arena-report`, server `POST /api/v1/render-contract/backend-plan-arena-report`, C ABI/header, Python, WASM/TypeScript, .NET, and Java wrappers. Later continuations added `BackendDocumentPlanArena`, `BackendDocumentPagePlan`, and `BackendDocumentPlanArenaReport` so an explicit render-view request owns the compiled CPU `RenderPlan` for every page and aggregates document-level packed-plan counts through the Rust SDK JSON surface, then added explicit contract-driven per-page reports for display, print, and proof CPU packed-plan arena identities without rendering pixels or constructing edit/semantic/validation views. GPU/debug backend-specific arenas, print-native backend payloads, and external runtime binding validation remain future backend/platform work. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine document_view --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine backend_plan_arena --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-cli backend_plan_arena_report_command_emits_json --test tool_surface --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-server --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-server render_contract_backend_plan_arena_report_route_returns_json --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-capi --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-py --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --nologo /m:1 -v:minimal` | 0 |
| `javac --enable-preview --release 25 -d .work/final-universal-renderer-implementation/java-classes bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java` | 0 |

## Latest document-view CLI/server report gates

This source slice exposes the canonical lazy document-view report through CLI `document-views-report` and server `POST /api/v1/document-views/report`. Both surfaces return the shared SDK `document_views_report` envelope, keeping render/edit/semantic/validation view boundaries observable without rendering pixels or forcing semantic/validation materialization. This stayed local and fixture-scoped; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-cli document_views_report_command_emits_json --test tool_surface --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-server --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-server document_views_report_route_returns_json --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest image-decode capability CLI/server report gates

This source slice exposes the per-document image-decode capability report through CLI `image-decode-capability-report` and server `POST /api/v1/image-decode/capability-report`. Both surfaces return the shared SDK `image_decode_capability_report` envelope, making codec metadata-inspection, region, reduction, progressive, tile, component, cancellation, and memory-budget capability states visible without decoding pixels. This stayed local and synthetic/fixture-scoped; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-cli image_decode_capability_report_command_emits_json --test tool_surface --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-cli report_command_emits_json --test tool_surface --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-server --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-server image_decode_capability_report_route_returns_json --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server report_route_returns_json --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest soft-mask group cache identity gates

This source slice moves SMask group cache lookup after `/G` Form resource merging, transparency group `/CS` policy validation, and derived `/BC`/default-alpha backdrop identity calculation. Cached SMask groups can no longer bypass malformed, missing-resource, non-device group color-space, or malformed backdrop/transfer-function refusals, and `smask_group_cache_key` is now salted with the resolved group color policy, deterministic `/G` resource fingerprint, and initial-backdrop/outside-alpha identity. The tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine smask_group_cache_key --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest SVG/PS inert semantic transparency-group Form gates

This source slice lets the SVG/PS Form XObject classifier replay isolated or knockout transparency-group Forms natively only when a conservative Form-stream scan, starting from the inherited caller graphics state, proves the semantic group flags are inert: no group color space or unknown group keys, no inherited or local alpha-changing ExtGState, no soft mask, no non-normal blend, no pattern paint, no image/inline-image/nested resource paint, and only opaque path/text/shading paint. Alpha-bearing semantic groups, including those made alpha-bearing by caller state inherited into the Form, remain on the whole-page raster/refusal boundary. The tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback transparency_group --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest SVG/PS device group-colour Form gates

This source slice lets the SVG/PS Form XObject classifier replay transparency
groups with explicit DeviceGray, DeviceRGB/RGB, or DeviceCMYK/CMYK group colour
spaces only when the same conservative Form-stream scan proves the group is
opaque, normal-blend, and vector-safe. Alpha-bearing DeviceRGB group-colour
Forms still remain on the whole-page raster/refusal boundary, and non-device or
malformed group colour spaces remain refused. The tests stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine device_group_color_space --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_form_xobject --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest SVG/PS calibrated inert group-colour Form gates

This source slice extends the SVG/PS Form XObject classifier for the same
opaque, normal-blend, vector-safe transparency-group subset. Direct calibrated
CalGray, CalRGB, and Lab `/Group /CS` arrays, plus resource-resolved calibrated
group color spaces, now stay native when the calibrated parameter dictionary is
well formed and the Form-stream proof shows the group color space is inert.
Alpha-bearing calibrated groups still fail typed through the active raster
group-color boundary instead of being replayed natively or approximated as
DeviceRGB. Richer non-device group color spaces and malformed calibrated group
metadata remain on the vector fallback/refusal boundary. The tests stayed local
and synthetic; no PDF corpus, benchmark, competitor comparison, VPS,
deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine calibrated_group_color_space --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine device_group_color_space --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_form_xobject --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| scoped `git diff --check` over calibrated group-colour Form source/report files | 0, CRLF normalization warnings only |

## Completed source work

| Area | Previous state | Implemented architecture | Source files | Entry points | Fallback policy | Cache / invalidation | Binding exposure | Remaining limitation |
|---|---|---|---|---|---|---|---|---|
| Canonical document views | Older reports still marked RV-01 as missing or undocumented at the SDK/binding boundary. | `CanonicalDocument` coordinates immutable source identity, revision, fingerprint, object/page IDs, and lazy view materialization counters; render, edit, semantic, and validation borrowed views expose scoped entry points without constructing each other; `RenderDocumentView` now owns default-contract, page-box contract, canonical contract render, caller-buffer render, font-report, telemetry-report, per-page backend-plan arena report wrappers, and explicit whole-document CPU backend plan arenas; `DocumentViewsReport`, `BackendPlanArenaReport`, and `BackendDocumentPlanArenaReport` make those boundaries observable. | `crates/engine/src/render/document_view.rs`, `crates/engine/src/engine.rs`, `crates/engine/src/sdk.rs`, `crates/cli/src/main.rs`, `crates/server/src/routes/document_views.rs`, `crates/server/src/routes/render_contract.rs`, C/Python/WASM/.NET/Java source shims | `ContentEngine::document_views_report`, `RenderDocumentView::{default_render_contract,backend_plan_arena_report,backend_document_plan_arena,render_with_contract,render_into_buffer_with_contract}`, `sdk::{document_views_report_json,backend_plan_arena_report_json,backend_document_plan_arena_report_json}`, CLI `document-views-report`/`backend-plan-arena-report`, server `POST /api/v1/document-views/report` and `POST /api/v1/render-contract/backend-plan-arena-report`, `wellfriendpdf_document_views_report_json`, `wellfriendpdf_document_backend_plan_arena_report_json`, Python `document_views_report`/`backend_plan_arena_report`, WASM `documentViewsReportJson`/`backendPlanArenaReportJson`, .NET `DocumentViewsReportJson`/`BackendPlanArenaReportJson`, Java `documentViewsReportJson`/`backendPlanArenaReportJson` | No render fallback; this is a source-boundary/reporting surface plus render-view API routing. | Report construction reads canonical identity and materialization counters without forcing semantic or validation materialization; per-page and document-level backend-plan wrappers increment only render materialization counters and own CPU `RenderPlan` values when explicitly requested. | Rust SDK, CLI, server, C ABI/header, Python, WASM/TypeScript, .NET, and Java source; whole-document arena JSON is Rust SDK source-active. | Ordinary view handles remain borrowed/lazy by design; explicit CPU backend packed document arenas are active, while GPU/printer/debug backend arenas and external runtime binding validation remain future backend/platform work. |
| Caller-owned surfaces | `reverse_byte_order` was rejected and `alpha_mode` was accepted as contract layout metadata without active RGBA/BGRA row semantics. | Row encoder reverses output bytes for supported formats after format conversion, and RGBA/BGRA caller-owned rows honor `AlphaMode::Premultiplied`, `Straight`, and `Opaque`. | `crates/engine/src/engine.rs` | `render_page_into_buffer`, contract JSON callers | No fallback for byte-order-only or alpha-mode-only layout. | Contract identity includes caller-surface layout; premultiplied RGBA rows may use the SIMD crate before scalar fallback. | Rust, C ABI, Python, WASM, .NET, Java where contract buffer APIs exist. | Binding build matrix remains local-unverified. |
| Print halftone and plate reporting | Screen halftone was not renderer-active and the active Separation/DeviceN plate framebuffer report was not publicly exposed as a first-class renderer source surface. | Ordered 4x4 screen over RGB bytes; alpha preserved. The active render-interpreter plate report now exposes deterministic plane order, plate summaries, contribution counts, bounded memory accounting, and cache fingerprint data for supported `/Separation` and `/DeviceN` fill/stroke paths. | `engine.rs`, `render/print_profile.rs`, `render/buffer.rs`, `render/contract.rs`, `render/page_renderer.rs`, `prepress.rs`, `sdk.rs`, `crates/cli/src/main.rs`, `crates/server/src/routes/prepress.rs`, C/Python/WASM/.NET/Java shims | `render_page_into_buffer`, `render_page_with_contract`, `ContentEngine::prepress_plate_report`, `sdk::prepress_plate_report_json`, CLI `prepress-plate-report`, server `POST /api/v1/prepress/plate-report`, and binding report methods. | `Screen + PreserveSeparations` returns typed unsupported; the plate report is a source-visible side-channel, not full separation-preserving output. | Contract halftone policy participates in accepted-policy comparison; the plate report includes bounded memory and cache-fingerprint identity. | Rust SDK, CLI, server, C ABI/header, Python, WASM/TypeScript, .NET, and Java source surfaces. | Full CMYK/DeviceN/proof/separation-preserving print execution and native CMM runtime validation remain incomplete. |
| Progressive server API | Rust lifecycle existed without a bounded HTTP session surface. | Owner-scoped session store, max session cap, idle reaper, start/step/pause/resume/cancel/close/status/finish routes. The session store keeps the cancellation source outside the mutable job lock so cancel requests signal the token before terminal cleanup waits on an in-flight step. | `crates/server/src/progressive_sessions.rs`, `crates/server/src/routes/progressive.rs`, `crates/server/src/app.rs`, `config.rs` | `/api/v1/progressive/*` | Unsupported retained tile replay now returns typed `UnsupportedFeature` instead of publishing immediate fallback tiles; structured fallback diagnostics remain reportable for other counted fallback events. | Session finish/cancel/close release retained tile buffers; idle reap cancels stale jobs. | Server HTTP API. | Full cross-binding runtime parity remains unverified. |
| Adaptive tile scheduling | Fixed tile sizes only, with viewport hint ordering. | Zero tile dimension selects deterministic size from 128/192/256/384/512 by page size and render temporary budget; hinted work is ordered into visible, adjacent-viewport, then background priority bands; dirty-region revision selectively invalidates and requeues intersecting tiles before clean/background work; completed tile publication JSON exposes `priority_class`; source sessions now evaluate tile-publication JSON and reject stale publication identities, obsolete tile identities, tile/order mismatches, revision mismatches, and visibility mismatches; source queue reports expose adjacent-page prefetch preview items before background prefetch work when neighboring pages exist; source queue execution renders owned current-page queue items; bounded adjacent-page prefetch execution validates a prefetch identity and, for live source sessions, creates a child job for the neighboring page; source callback dispatch reports derive publication/prefetch/queue events without rendering. | `render/progressive.rs`, server route, C/Python/WASM/.NET/Java wrappers | Rust progressive constructors, `ProgressiveRenderJob::viewer_queue_report`, `ProgressiveRenderJob::execute_viewer_queue`, `ProgressiveRenderJob::execute_adjacent_page_prefetch`, `ProgressiveRenderJob::viewer_callback_dispatch_report`, and `ProgressiveRenderJob::dispatch_viewer_callbacks`; server `tile_size=adaptive`, `/api/v1/progressive/:id/evaluate-publication`, `/api/v1/progressive/:id/queue`, `/api/v1/progressive/:id/queue/execute`, `/api/v1/progressive/:id/adjacent-prefetch/execute`, and `/api/v1/progressive/:id/callbacks`; C ABI `wellfriendpdf_progressive_render_evaluate_tile_publication_json`, `wellfriendpdf_progressive_render_viewer_queue_json`, `wellfriendpdf_progressive_render_execute_viewer_queue_json`, `wellfriendpdf_progressive_render_execute_adjacent_page_prefetch_json`, `wellfriendpdf_progressive_render_viewer_callback_dispatch_json`, and `wellfriendpdf_progressive_render_dispatch_viewer_callbacks`; Python/WASM/.NET/Java publication evaluation, viewer queue, queue-execution, adjacent-prefetch execution, callback-dispatch, and synchronous callback helper wrappers | None; fixed sizes still accepted. | Token records selected tile dimensions, scheduler generation, dirty region, and explicit publication identity; step reports expose per-tile priority class, adjacent-page prefetches, bounded viewer queue preview items, queue execution reports with `queue_before`/render step/`queue_after` identity state, and adjacent-prefetch execution reports with source/prefetch page state. | Rust, server, C/Python/WASM via zero dimensions, publication-evaluation JSON, viewer queue JSON, viewer queue execution JSON, adjacent-prefetch execution, viewer callback dispatch JSON, and host callback execution; .NET/Java helper methods. | External viewer runtime matrices and broader queue policy remain incomplete; source adjacent-prefetch execution, callback dispatch JSON, and synchronous callback helpers are active. |
| Progressive publication identity and obsolete work | Step reports exposed completed tile rectangles but no stable publication discriminator. | Tokens and step reports now expose deterministic job publication identity; completed tile publications carry per-tile identities salted by document fingerprint/revision, page, DPI, render mode, tile grid, page pixels, render-contract fingerprint, visibility fingerprint, viewport hint, dirty region, scheduler generation, tile index, and tile order; viewport revisions clear prior retained tiles; dirty-region revisions preserve clean tiles, invalidate only intersecting tile buffers, mint a fresh publication identity, report bounded tile-level `obsolete_publications`; render-context revisions replace caller-visible contract/visibility identity, clear retained surfaces, and mark the prior publication identity obsolete; source-session acceptance reports reject stale or malformed tile publications; viewer queue, adjacent-prefetch execution, and callback-dispatch reports reuse the same report shape without rendering unrelated work. | `render/progressive.rs`, `render/mod.rs`, `lib.rs`, `runtime.rs`, `sdk.rs`, server/C/Python/WASM/.NET/Java wrappers | `ProgressiveRenderJob::token`, `ProgressiveRenderJob::render_next`, `ProgressiveRenderJob::revise_viewport_hint`, `ProgressiveRenderJob::revise_dirty_region`, `ProgressiveRenderJob::revise_render_context`, `ProgressiveRenderJob::evaluate_tile_publication`, `ProgressiveRenderJob::viewer_queue_report`, `ProgressiveRenderJob::execute_adjacent_page_prefetch`, `ProgressiveRenderJob::viewer_callback_dispatch_report`, `ProgressiveRenderJob::dispatch_viewer_callbacks`, JSON report APIs and binding callback helpers | None; this is metadata plus bounded retained-tile invalidation and source-session stale-publication rejection. | Callers can reject stale tile publications through source-session acceptance reports when a newer viewport/revision/render-context/job identity is active; the source session API marks prior viewport publications, dirty tiles, and render-context publications obsolete; queue preview reports publish deterministic current/adjacent/background ranks; adjacent-prefetch execution reports source and child-job identity state; callback dispatch reports expose ordered ready/obsolete/scheduled events and suppress post-terminal callbacks; C/Python/WASM/.NET/Java helpers synchronously invoke caller callbacks for those events. | Rust/server/C/Python/WASM/.NET/Java source. | External viewer runtime matrices and broader queue policy remain incomplete. |
| Progressive request cancellation | Progressive step wrappers passed `CancelToken::none()`, so cancel paths were terminal between steps but not source-wired for in-flight progressive work. | `ProgressiveRenderJob` owns a session cancellation source, links it with caller tokens during `render_next`, resets the retained token on resume/viewport revision, exposes request-cancel, adjacent-prefetch execution, viewer callback dispatch reports, and synchronous callback helper surfaces in server/C/Python/WASM/.NET/Java source, and adds binding-native cancellation ergonomics for Python, WASM, .NET, and Java step/finish calls. | `cancel.rs`, `render/progressive.rs`, `crates/server/src/progressive_sessions.rs`, `crates/wellfriendpdf-capi/src/lib.rs`, `crates/wellfriendpdf-py/src/lib.rs`, `crates/wellfriendpdf-wasm/src/lib.rs`, `.NET/Java` wrappers | `ProgressiveRenderJob::request_cancel`, `ProgressiveRenderJob::execute_adjacent_page_prefetch`, `ProgressiveRenderJob::viewer_callback_dispatch_report`, `ProgressiveRenderJob::dispatch_viewer_callbacks`, `wellfriendpdf_progressive_render_request_cancel`, `wellfriendpdf_progressive_render_execute_adjacent_page_prefetch_json`, `wellfriendpdf_progressive_render_viewer_callback_dispatch_json`, `wellfriendpdf_progressive_render_dispatch_viewer_callbacks`, `request_cancel`, `requestCancel`, `RequestCancel`, Python `dispatch_viewer_callbacks`/`step_with_cancellation`/`finish_png_with_cancellation`, WASM `dispatchViewerCallbacks`/`stepWithCancellation`/`finishPngWithCancellation`, `.NET DispatchViewerCallbacks`/`StepJson`/`FinishPng(CancellationToken)`, Java `dispatchViewerCallbacks`/`stepJson`/`finishPng(BooleanSupplier)` | Request cancellation returns a resumable step report; terminal cancel still releases retained tiles and suppresses callback dispatch; pre-cancelled binding calls request native cancellation before raising the language cancellation exception. | Session token is atomic and clone-owned; terminal cancel/close still clear retained tiles and cache. | Rust/server/C/Python/WASM/.NET/Java source. | External binding runtime gates and full viewer queue policy remain incomplete. |
| Contract render cancellation | Normal and report-returning contract PNG and caller-owned buffer render wrappers used uncancellable `CancelToken::none()` paths or lacked binding-owned cancellation handles outside progressive rendering. | Core contract render helpers accept caller cancellation sources; C ABI/header expose `WellfriendRenderCancellation` plus cancel/status ownership APIs and cancellable JSON/handle contract PNG and caller-owned buffer entrypoints, including font-substitution and render-telemetry report variants; Python/WASM expose `RenderCancellation` and cancellable contract PNG/caller-buffer/report methods; .NET and Java expose binding-owned render cancellation handles plus status queries and contract PNG/caller-buffer/report overloads. | `cancel.rs`, `crates/wellfriendpdf-capi/src/lib.rs`, `crates/wellfriendpdf-capi/include/wellfriendpdf.h`, `crates/wellfriendpdf-py/src/lib.rs`, `crates/wellfriendpdf-wasm/src/lib.rs`, `crates/wellfriendpdf-wasm/wellfriendpdf.d.ts`, `.NET/Java` wrappers | `wellfriendpdf_render_cancellation_new`, `wellfriendpdf_render_cancellation_cancel`, `wellfriendpdf_render_cancellation_is_cancelled`, `wellfriendpdf_document_render_page_png_with_contract_json_and_cancellation`, `wellfriendpdf_document_render_into_buffer_with_contract_json_and_cancellation`, report `*_json_and_cancellation` variants, Python/WASM `RenderCancellation`, Python `render_contract_into*_with_cancellation`, .NET `RenderCancellation`/`CancellationToken` overloads, Java `RenderCancellation` overloads | Cancellation is cooperative and source-wired through render descriptor replay; compatibility render methods still use explicit non-cancellable tokens. | Cancellation handles are opaque/binding-owned and freed through language resource wrappers. | C/Python/WASM/.NET/Java source plus .NET build and Java source compile. | Python bytearray/progressive-job GIL behavior, single-thread JS hosts, external native binding runtime smokes, and broader runtime matrices remain incomplete. |
| Progressive queue execution cancellation | Public queue execution and adjacent-page prefetch execution wrappers existed, but binding surfaces only exposed the non-cancellable compatibility paths. | Core execution functions already accepted caller cancellation; C/Python/WASM/.NET/Java now expose source-visible cancellation for owned current-page queue execution and adjacent-page prefetch execution. | `crates/wellfriendpdf-capi/src/lib.rs`, `crates/wellfriendpdf-capi/include/wellfriendpdf.h`, `crates/wellfriendpdf-py/src/lib.rs`, `crates/wellfriendpdf-wasm/src/lib.rs`, `crates/wellfriendpdf-wasm/wellfriendpdf.d.ts`, `.NET/Java` wrappers | `wellfriendpdf_progressive_render_execute_viewer_queue_json_and_cancellation`, `wellfriendpdf_progressive_render_execute_adjacent_page_prefetch_json_and_cancellation`, Python `execute_viewer_queue_json_with_cancellation`/`execute_adjacent_page_prefetch_with_cancellation`, WASM `executeViewerQueueJsonWithCancellation`/`executeAdjacentPagePrefetchWithCancellation`, .NET `RenderCancellation`/`CancellationToken` overloads, Java `RenderCancellation` overloads | Cancellation remains cooperative and bounded to execution calls; legacy compatibility wrappers stay non-cancellable. | Uses the same opaque/binding-owned cancellation handle family as contract rendering. | C/Python/WASM/.NET/Java source plus .NET build and Java source compile. | External viewer runtime matrices and broader queue policy validation remain incomplete. |
| Server render-contract builder and render routes | Server render contract exposure was limited to default/raw JSON paths and later to focused geometry/surface/resource fields. | `/api/v1/render-contract` builds the canonical schema-v1 contract from a PDF page and requested page box, applies typed surface, clip, device-transform, background, execution/backend/compositing, annotation/form, optional-content, smoothing, color/prepress, exactness, determinism, and resource-budget overrides, validates the result, computes surface byte length, and returns normalized contract JSON plus cache fingerprint. `/api/v1/render-contract/png` consumes canonical PNG-capable contracts, `/api/v1/render-contract/raw` consumes caller-owned surface contracts without normalizing away pixel format, alpha mode, stride, grayscale, or byte-order fields, and `/api/v1/render-contract/png-with-font-substitution-report` plus `/api/v1/render-contract/raw-with-font-substitution-report` return `multipart/mixed` with JSON metadata/report first and rendered bytes second. Public `max_decoded_bytes` now applies to page content stream decode before render planning and to downstream render decode paths; contract page-content decode observes the caller `CancelToken`; public `max_temporary_bytes` rejects both canonical RGBA working surfaces and clipped full-page intermediate-plus-crop allocations before render allocation; render-contract telemetry now exposes the source-owned field-effect matrix for every serialized schema-v1 field. | `crates/server/src/routes/render_contract.rs`, `crates/server/src/app.rs`, `crates/server/src/routes/mod.rs`, `crates/engine/src/render/contract.rs`, `crates/engine/src/engine.rs` | `POST /api/v1/render-contract`; `POST /api/v1/render-contract/png`; `POST /api/v1/render-contract/raw`; `POST /api/v1/render-contract/png-with-font-substitution-report`; `POST /api/v1/render-contract/raw-with-font-substitution-report` | Invalid fields, unsupported render modes, invalid contract policies, cancellation, over-budget dimensions, over-budget content/decode work, over-budget temporary working surfaces, non-invertible transforms, PNG-incompatible caller-surface layouts, and oversized rendered/multipart output return existing typed HTTP errors. | Uses core `RenderContract::validate`, core page-box-aware contract construction, core contract page-content decode limits and cancellation prechecks, core clipped-contract temporary budget checks, server render-pixel/output caps, core contract cache fingerprint, core `RenderContractFieldEffect` registry/telemetry, `render_page_png_with_contract`, `render_page_into_buffer`, `render_page_png_with_contract_and_font_substitution_report`, and `render_page_into_buffer_with_font_substitution_report`. | Server HTTP API plus all bindings that pass contract JSON/handles to core render methods. | Source field-effect parity is mechanically guarded; external client/runtime gates remain future platform validation. |
| Image metadata planning | Non-zero tile origins failed open and decoded, and inline images decoded before viewport rejection. | Planner uses tile-local transformed bounds and skips image XObjects and inline images outside the active tile/viewport before decode; offscreen inline images return before scheduler reservation; active inline image, Image XObject, and referenced explicit-mask `/Filter` metadata must be a name or all-name array before bpc/color-space planning; planned image decode keys now drive raw XObject decoded-image cache lookup/admission; typed Rust/source capability reports mark JBIG2/lossless paths as full-decode-only, mark safe DCT/JPEG downscale plans as native reduced-IDCT, mark safe JPX downscale plans as native target-resolution reduction, and keep JPEG region/progressive unavailable plus JPX ROI/region/tile/component/progressive unavailable at the current decoder API boundary while guarded axis-aligned CCITT source-clipped XObjects use bounded grayscale source-window decode/cache output and inline images use bounded grayscale source-window decode output when no reduction or full-image postprocessing is required; active decode-required paths consume/log those reports, visible decode-required plans record whether the viewport needs source-region or reduction decode support, `HighQualityExact` refuses unsupported paths when the required support is unavailable, and a per-image capability report enumerates discovered images without decoding pixels. `ProgressiveImageDecodeSession` exposes a bounded start/continue/pause/resume/cancel/close lifecycle for image decode work, reports `full_decode_required` only for nonterminal non-progressive work, preserves terminal states through continue-after-cancel/close, and retains no decoded pixels; lifecycle JSON is exposed through Rust SDK, C ABI/header, Python, WASM, .NET, Java, CLI, and server. | `render/image_decode_planning.rs`, `render/page_renderer.rs`, `runtime.rs`, `engine.rs`, `sdk.rs`, CLI/server/C/Python/WASM/.NET/Java binding sources | Image XObject render planning before decode/cache lookup; inline-image render planning before decode scheduler reservation; referenced explicit-mask metadata planning; guarded CCITT source-window XObject render decode/cache output and inline-image render decode output; guarded DCT/JPEG reduced-IDCT downscale decode for safe XObject/inline-image paths; runtime capability JSON exposes full-decode codec limitations, guarded DCT/JPX/CCITT exceptions, and lifecycle boundary; `image_decode_capability_report` / `imageDecodeCapabilityReportJson` exposes per-image codec region/reduction/progressive statuses; `progressive_image_decode_lifecycle_report_json` / binding equivalents expose lifecycle reports for selected discovered images. | Degenerate/nonfinite transforms still fail open to avoid incorrect culling; JPEG region/progressive paths, JBIG2/lossless ROI/reduction/progressive paths, and CCITT reduction/progressive/non-axis/unsupported-postprocessing-window paths remain unavailable; JPX ROI/region/tile/component/progressive paths are explicit unavailable capabilities at the current decoder API boundary; the lifecycle does not fake native progressive pixel output. | Cache key includes source identity, revision, render-contract identity, tile viewport, device transform, target size, quality, explicit stable codec/source-region/reduction/decode-array/image-mask/soft-mask/interpolation fragments, print/profile policy, optional-content state, and resource budget; progressive-image lifecycle state retains no decoded pixels while native progressive output is unavailable and releases retained state on cancel/close. | Internal renderer path; Rust SDK, C ABI/header, Python, WASM, .NET, Java, CLI, and server source surfaces expose per-image capability/lifecycle JSON. | JPEG region/progressive decode, JBIG2/lossless ROI/reduction/progressive decode, and CCITT reduction/progressive/non-axis/unsupported-postprocessing-window closure remain incomplete; JPX ROI/region/tile/component/progressive output is reported unavailable at the current decoder API boundary. |
| Image SMask discovery | Soft-mask classification used a second object-reader pass. | `/SMask` object references are collected during primary XObject traversal. | `images/locator.rs` | `ImageLocator::find_page_images`, `find_all_images` | No renderer fallback; classification only. | Eliminates the secondary lookup pass. | Rust image locator and extraction routes. | Inline soft-mask relationships still follow PDF source expressiveness. |
| Persistent clip DAG | Generic path/intersection clip states did not expose the requested persistent representation variants, structural non-rectangle intersections, tile-local materialization, or repeated transformed-path clip reuse. | `ClipState` now represents `Full`, `Empty`, `Rectangle`, `SparseSpans`, `RleMask`, `DenseMask`, and `Composite` intersection nodes; `ClipNode` records stable ID, operation, parent edge, visible bounds, revision/render-contract/tile identity salts, and structural memory charge; rectangle/span/RLE intersections stay structural without dense materialization; window materialization produces destination-local masks for rectangle/span/RLE/dense/composite states; composite window intersections short-circuit empty/all-visible child windows; partial-coverage clips round-trip through `ClipMask::from_alpha_bytes`; page/content/Form/packed-plan save-restore paths restore buffer clips from saved DAG state without populating node materialization caches; repeated transformed path clip installs now reuse a bounded `PathClipNodeCache` and intersect through `ClipDag` before restoring the concrete `PixelBuffer` mask; active ClipDag intern tables transitively prune unreferenced nodes, including composite children orphaned during the same pruning call, and report final intern/pruning stats through render-contract telemetry; normal Compat LCD/subpixel glyph masks now fuse clip opacity and SMask bytes in row-local code instead of dispatching each painted pixel through the general pixel compositor. | `render/clip_dag.rs`, `render/buffer.rs`, `render/page_renderer.rs`, `engine.rs` | `ClipDag::intern_mask`, `intern_option`, `intersect`; renderer save/restore stacks hold `Arc<ClipNode>`; content-stream `W/W*`, display-list clips, packed-plan clips, Form BBox clips, transparency-group clip carry, tiling-pattern clips, and shading-pattern clips install through DAG nodes where their inputs are already clip masks or transformed paths; `RenderContractTelemetryReport` serializes the final per-render `ClipDagStats`; `PixelBuffer::blend_lcd_alpha_mask_strided` reports LCD row compositor coverage through `PixelCompositorStats::lcd_row_pixels`. | No visual fallback; materialization remains exact when a concrete `ClipMask` is required. | Scoped identity fields participate in stable node IDs; transformed-path clip cache keys include the render-contract fingerprint and the cache is carried through `RenderDocumentCache` plus child render-state absorption while staying bounded by entry count/estimated bytes; active ClipDag intern tables use a default node cap with transitive stale-node pruning and live-node preservation. | Internal renderer path plus Rust/server/C/Python/WASM/.NET/Java render-report JSON source plumbing. | `PixelBuffer` still consumes concrete masks at general paint boundaries, and universal concrete-mask elimination plus complete active-buffer contract adoption remain incomplete. |
| Compact clip/soft-mask fusion | Compact transparency-group compositing fell back to per-pixel scalar when the parent clip had partial coverage or the soft mask did not cover the destination window. | `AlphaMask::fused_clip_window` materializes a bounded destination-local mask that multiplies soft-mask bytes with `ClipMask` opacity; `composite_from_at` routes partial clips, empty/out-of-window clips, and offset/smaller soft masks through row soft-mask kernels. | `render/buffer.rs` | `PixelBuffer::composite_from_at` compact group composition | No visual fallback; destination-window opacity is preserved as 8-bit row coverage before source-over. | Focused tests cover fused byte math and verify the partial-clip/SMask case reaches the soft-mask row compositor. | Internal transparency-group composite path. | Full direct-paint dense clip/mask fusion, image-mask fusion, high-quality/blend-mode group-space closure, and universal DAG hot replay remain incomplete. |
| Direct alpha-mask clip/SMask fusion | Glyph, Type3, stroked-path, and image-mask-style alpha paints fell back to `blend_pixel` whenever a soft mask or partial clip was active. | `blend_alpha_mask` now fuses the paint mask, current SMask bytes, and partial `ClipMask` opacity into bounded row coverage and calls the normal Compat alpha-mask row compositor. | `render/buffer.rs` | `PixelBuffer::blend_alpha_mask` | Knockout, high-quality rendering, and non-normal blend modes keep the scalar semantic fallback. | Focused test verifies partial clip plus SMask reaches row compositing. | Internal alpha-mask paint path. | Image decode ROI/reduction, non-normal blend fusion, and universal DAG hot replay remain incomplete. |
| RGBA fragment clip/SMask fusion | Cached RGBA glyph/image fragments fell back to `blend_pixel` whenever a soft mask or partial clip was active. | `blend_rgba_pixels_at` now fuses source alpha with current SMask bytes and partial `ClipMask` opacity into bounded row-local RGBA and dispatches to the normal Compat row compositor. | `render/buffer.rs` | `PixelBuffer::blend_rgba_pixels_at` | Knockout, high-quality rendering, and non-normal blend modes keep the scalar semantic fallback. | Focused test verifies partial clip plus SMask reaches row compositing. | Internal cached RGBA glyph/image fragment path. | Image decode ROI/reduction, non-normal blend fusion, high-quality/group-space closure, and universal DAG hot replay remain incomplete. |
| Solid fill clip/SMask fusion | Solid rectangle paints fell back to `blend_pixel` whenever a soft mask or partial clip was active. | `fill_rect` now fuses current SMask bytes and partial `ClipMask` opacity into bounded row coverage and dispatches to the normal Compat alpha-mask row compositor with the solid paint color. | `render/buffer.rs` | `PixelBuffer::fill_rect` | Knockout, high-quality rendering, and non-normal blend modes keep the scalar semantic fallback. | Focused test verifies partial clip plus SMask reaches row compositing. | Internal solid rectangle paint path. | Non-normal blend fusion, high-quality/group-space closure, and universal DAG hot replay remain incomplete. |
| Separable blend source-alpha fusion | Translucent separable solid fills over opaque destinations and separable solid fills with partial clips fell back to `blend_pixel`, then partial-clip opaque fills later used a scalar row loop and non-opaque destination rows still missed the row compositor. | `fill_rect` now routes guarded separable solid fills through f32x4 row compositors for opaque and mixed-destination rows, with or without partial clips, mixing the separable blended color by source alpha, clip opacity, and destination alpha for all PDF separable modes, with scalar tails. | `render/buffer.rs` | `PixelBuffer::fill_rect` with separable blend modes | SMask, knockout, high-quality rendering, and non-solid/group blends keep the scalar semantic fallback. | Focused tests compare opaque, translucent, partial-clip, and mixed-destination separable modes against `blend_pixel` and verify the wide separable row counter. | Internal solid rectangle paint path for separable blend modes. | General non-normal blend fusion, high-quality/group-space closure, and universal DAG hot replay remain incomplete. |
| Separable RGBA partial-clip fusion | Cached RGBA glyph/image fragments over opaque destinations fell back to `blend_pixel` for separable blend modes when the active clip had partial coverage, then later used a scalar row loop and only accepted fully opaque source/destination rows. | `blend_rgba_pixels_at` now detects partial-clip RGBA rows under separable blend modes and applies a f32x4 row compositor that mixes the separable blended fragment color by source alpha, clip opacity, and destination alpha, with scalar tails. | `render/buffer.rs` | `PixelBuffer::blend_rgba_pixels_at` with separable blend modes | SMask, knockout, non-partial separable RGBA row fusion, high-quality rendering, non-separable modes, and group blends keep the scalar semantic fallback. | Focused tests compare opaque, mixed-alpha, and mixed-destination separable partial clips against `blend_pixel` and verify the wide separable row counter. | Internal cached RGBA glyph/image fragment path for separable blend modes. | General non-normal blend fusion, high-quality/group-space closure, image decode ROI/reduction, and universal DAG hot replay remain incomplete. |
| C ABI declarations | C exports existed without matching public header declarations for the new renderer APIs. | Header now declares opaque progressive handle, contract JSON rendering, caller buffer rendering, progressive lifecycle functions, request-cancel, viewport revision, dirty-region revision, tile-publication evaluation, viewer queue JSON, viewer queue execution JSON, adjacent-page prefetch execution JSON with optional child job handle, viewer callback dispatch JSON, and callback-pointer dispatch. | `crates/wellfriendpdf-capi/include/wellfriendpdf.h` | C consumers and generated wrappers. | FFI returns status/error strings; callback-pointer dispatch stops with an error when a callback returns non-zero. | Caller-owned buffers remain caller-owned; per-event callback JSON is valid only during the callback; adjacent-prefetch child jobs are caller-owned handles. | C ABI. | External C consumer build not run in this local pass. |
| WASM TypeScript declarations | Rust WASM methods existed without matching `.d.ts` declarations. | Declaration file now exposes `ProgressiveRenderJob`, contract PNG, caller buffer, progressive job creation, tile-publication evaluation, `viewerQueueJson`, `executeViewerQueueJson`, `AdjacentPagePrefetchExecution`, `executeAdjacentPagePrefetch`, `viewerCallbackDispatchJson`, and `dispatchViewerCallbacks`. | `crates/wellfriendpdf-wasm/wellfriendpdf.d.ts` | TypeScript consumers. | WASM methods report JS errors from engine errors. | WASM caller buffer writes are explicit; per-event callback JSON is synchronous host-call data; adjacent-prefetch child jobs transfer through `takeJob`. | WASM/TypeScript. | `wasm-pack` build not run. |
| WASM SIMD Gray8 channel conversion | The SIMD crate had row compositor kernels but no contract-surface grayscale/channel-conversion SIMD lane. | `render-simd` now exposes `rgba_to_gray8`; wasm32+`simd128` uses vector luma conversion for four RGBA pixels per iteration, and `render_page_into_buffer` routes `PixelFormat::Gray8` contract rows through it before scalar fallback. | `crates/render-simd/src/lib.rs`, `crates/engine/src/engine.rs` | `wellfriendpdf_render_simd::rgba_to_gray8`; `ContentEngine::render_page_into_buffer` / `render_page_into_buffer_with_font_substitution_report` for Gray8 contract surfaces | Non-wasm and wasm-without-simd decline or use the scalar oracle; unsupported conversions and SIMD categories remain scalar. | No cache state; row encoder zero-fills stride padding before the SIMD early return. | Rust core caller-owned surface path and bindings that use the core contract renderer. | Full WASM SIMD operation coverage remains incomplete for unsupported blend modes and broader non-normal/high-quality row coverage. |
| WASM SIMD grayscale expansion rows | Grayscale caller-owned RGB/RGBA/BGRA rows used scalar luma expansion. | `render-simd` now exposes `rgba_to_gray_rgb8`, `rgba_to_gray_rgba8`, `rgba_to_gray_bgra8`, `rgba_to_premultiplied_gray_rgba8`, and `rgba_to_premultiplied_gray_bgra8`; wasm32+`simd128` load/store groups expand four-pixel RGBA source rows into gray RGB/BGR and straight/opaque/premultiplied RGBA/BGRA contract rows with scalar-equivalence guards, and the core row encoder calls them for non-byte-reversed grayscale RGB/BGR and straight/opaque/premultiplied RGBA/BGRA rows before scalar fallback. | `crates/render-simd/src/lib.rs`, `crates/engine/src/engine.rs` | `wellfriendpdf_render_simd::{rgba_to_gray_rgb8,rgba_to_gray_rgba8,rgba_to_gray_bgra8,rgba_to_premultiplied_gray_rgba8,rgba_to_premultiplied_gray_bgra8}`; `ContentEngine::render_page_into_buffer` / `render_page_into_buffer_with_font_substitution_report` for simple grayscale contract surfaces | Byte-reversed rows, non-wasm rows, and unsupported rows stay on the scalar exact path. | No cache state; row bounds and scalar guards define exact fallback behavior. | Rust core caller-owned surface path and WASM target source. | Runtime wasm test-runner execution and full SIMD category coverage remain incomplete. |
| Native SSSE3 grayscale contract rows | Native x86/x86_64 builds still declined Gray8, grayscale RGB/RGBA/BGRA expansion, and premultiplied grayscale alpha-bearing rows that already had wasm `simd128` helpers. | `render-simd` now dispatches `rgba_to_gray8`, `rgba_to_gray_rgb8`, `rgba_to_gray_rgba8`, `rgba_to_gray_bgra8`, and `rgba_to_premultiplied_gray_rgba8` through guarded SSSE3 luma kernels when available. `rgba_to_premultiplied_gray_bgra8` shares the same gray/alpha output path. The kernels preserve the exact `77/150/29` luma contract, force alpha only when requested, retain scalar debug oracles, and keep scalar tails for uneven rows. | `crates/render-simd/src/lib.rs`, `crates/engine/src/engine.rs` | `wellfriendpdf_render_simd::{rgba_to_gray8,rgba_to_gray_rgb8,rgba_to_gray_rgba8,rgba_to_gray_bgra8,rgba_to_premultiplied_gray_rgba8,rgba_to_premultiplied_gray_bgra8}`; `ContentEngine::render_page_into_buffer` / `render_page_into_buffer_with_font_substitution_report` for grayscale contract surfaces | Non-SSSE3 native hosts, non-wasm non-x86 hosts, byte-reversed rows, and unsupported row shapes decline to scalar fallback. | No cache state; row bounds and scalar guards define exact fallback behavior. | Rust core caller-owned surface path. | Full SIMD category coverage remains incomplete for remaining non-normal/high-quality paths and external runtime matrices. |
| CPU/WASM SIMD alpha/glyph-mask rows | Glyph/image alpha-mask paints still used the engine-local portable row path, and native mixed-destination alpha-mask rows had no `render-simd` public path. | `render-simd` now exposes `blend_alpha_mask_opaque_destination` and `blend_alpha_mask_normal`; native x86/x86_64 SSE2 and wasm32+`simd128` compute grouped alpha-mask rows with scalar-equivalence guards. The opaque-destination helper uses integer row math, while the mixed-destination normal Compat helper uses grouped loads/stores with the exact scalar floating-point source-over byte contract. `PixelBuffer::blend_alpha_mask` calls them before the existing portable/scalar fallback. | `crates/render-simd/src/lib.rs`, `crates/engine/src/render/buffer.rs` | `wellfriendpdf_render_simd::{blend_alpha_mask_opaque_destination,blend_alpha_mask_normal}`; `PixelBuffer::blend_alpha_mask` for glyph, Type 3, stroked-path, and image-mask alpha rows under normal Compat conditions | Non-SSE2 native hosts, non-wasm non-x86 hosts, and unsupported rows decline to the existing portable/scalar fallback; non-normal and unsupported blend SIMD categories remain incomplete. | No cache state; row bounds and scalar guards define exact fallback behavior. | Internal renderer compositing path plus native/WASM target source. | Runtime wasm test-runner execution and full SIMD category coverage remain incomplete. |
| LCD subpixel glyph-mask row fusion | Normal Compat LCD/subpixel glyph masks used the per-pixel LCD compositor even when the paint was a bounded row with active clip/SMask state. | `PixelBuffer::blend_lcd_alpha_mask_strided` now computes LCD red/green/blue coverage row-local, fuses installed clip opacity and SMask bytes once per row, writes the destination row directly with the existing LCD source-over math, and keeps the former per-pixel LCD helper as a test-only oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::blend_lcd_alpha_mask_strided`; `PixelCompositorStats::lcd_row_pixels` | High-quality, non-normal blend, knockout, and unsupported LCD policies continue to route through the existing alpha-mask fallback/oracle path. | No cache state; row bounds, subpixel coverage bytes, clip opacity bytes, SMask bytes, and scalar-oracle equality define deterministic behavior. | Internal renderer subpixel glyph paint path. | Non-normal/high-quality LCD policy and external font/runtime matrices remain incomplete. |
| Knockout group row replacement | `knockout_from` flattened group replacement pixel-by-pixel even when source/destination rows, soft masks, and clips were already bounded. | `knockout_from` now uses row slices to replace destination RGB and alpha with source RGB plus source/group/soft-mask/clip-scaled alpha, preserves clip-zero skips and zero-soft-mask transparent replacement RGB semantics, and reports `PixelCompositorStats::knockout_row_pixels`. | `crates/engine/src/render/buffer.rs`, `crates/cli/src/main.rs` | `PixelBuffer::knockout_from`; CLI compositor telemetry `knockout_row_pixels_delta` | This preserves existing replacement semantics; it is not full PDF knockout/backdrop/group-colour-space closure. | No cache state; source row, destination row, clip opacity bytes, SMask bytes, and scalar-oracle equality define deterministic behavior. | Internal transparency-group flattening path plus CLI telemetry. | Full non-device group-space, backdrop interaction, and corpus/runtime transparency validation remain incomplete. |
| WASM SIMD soft-mask group-alpha rows | Soft-mask compositing with non-opaque group alpha was rejected before the SIMD/wide row dispatch, and mixed-destination soft-mask rows fell through the scalar general compositor. | `render-simd` now computes the engine-compatible `src_alpha * mask * group_alpha / 255^2` effective alpha, accepts non-opaque `group_alpha_255` in the public soft-mask dispatcher, and the engine opaque-destination soft-mask row path calls the SIMD/portable-wide route before scalar fallback; mixed-destination normal Compat soft-mask rows now use a portable `f32x4` general compositor before scalar tails. | `crates/render-simd/src/lib.rs`, `crates/engine/src/render/buffer.rs`, `crates/engine/src/runtime.rs`, `crates/cli/src/main.rs` | `wellfriendpdf_render_simd::composite_soft_mask_opaque_destination`; `PixelBuffer::composite_from` and `composite_from_at` normal Compat soft-mask rows; `runtime_capabilities_for`; CLI compositor telemetry | Non-wasm rows without an accepted native SIMD helper still use the portable-wide row or scalar fallback; non-normal/high-quality group-space rows remain separate incomplete categories. | No cache state; row bounds, engine-compatible effective-alpha helper, scalar alpha-byte semantics, and scalar guards define exact fallback behavior. | Internal renderer soft-mask compositing path, runtime capability JSON, CLI telemetry, and WASM target source. | Runtime wasm test-runner execution and full SIMD category coverage remain incomplete. |
| Source-over mixed-alpha group-opacity rows | Normal Compat source-over rows with partial group alpha used the wide uniform-alpha row path only when every source pixel was fully opaque. Mixed source-alpha rows dropped to the general scalar compositor. | The engine opaque-destination source-over row path now scales each source pixel alpha by byte-effective group alpha and processes mixed-source rows through the portable-wide uniform-alpha route before scalar tail fallback. Runtime capabilities disclose the mixed-source group-alpha opaque-destination lane. | `crates/engine/src/render/buffer.rs`, `crates/engine/src/runtime.rs` | `PixelBuffer::composite_from`; `PixelBuffer::composite_from_at`; `runtime_capabilities_for` | Non-normal blend modes, high-quality group-space blending, and full transparency-group closure remain incomplete. | No cache state; byte-effective source/group alpha and row bounds define deterministic fallback behavior. | Internal renderer source-over compositing path and runtime capability JSON. | Full SIMD category coverage remains incomplete. |
| Source-over mixed-destination rows | Normal Compat source-over rows whose destination row contained non-opaque alpha fell through the general scalar compositor after the opaque-destination fast paths declined. | `composite_normal_compat_row` now dispatches those mixed-destination source-over rows through a portable `f32x4` compositor that preserves scalar source-alpha, group-alpha, destination-alpha, RGB rounding, and alpha truncation semantics, with scalar tails for short rows. Runtime capabilities disclose the mixed-destination source-over lane and CLI telemetry exposes `wide_general_pixels_delta`. | `crates/engine/src/render/buffer.rs`, `crates/engine/src/runtime.rs`, `crates/cli/src/main.rs` | `PixelBuffer::composite_from`; `PixelBuffer::composite_from_at`; `runtime_capabilities_for`; CLI compositor telemetry | Non-normal blend modes, high-quality group-space blending, and full transparency-group closure remain incomplete. | No cache state; row bounds and scalar-equivalence guards define deterministic fallback behavior. | Internal renderer source-over compositing path, runtime capability JSON, and CLI telemetry. | Full SIMD category coverage remains incomplete. |
| High-quality normal group scalar rows | High-quality Normal group flattening used the per-pixel `blend_pixel` dispatcher for row-sized full and compact group windows even when no soft mask, installed SMask, partial clip, or knockout state was active. | `composite_normal_high_quality_row` and `composite_normal_high_quality_row_partial_clip` now handle eligible full-page and compact group windows, including no clip, all-visible clip, binary-clip runs, and partial clips, while preserving the existing `blend_pixel` linear-light RGB conversion, source/group alpha multiplication, clip coverage, destination-alpha handling, and alpha truncation. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::composite_from`; `PixelBuffer::composite_from_at` | Dedicated knockout row entries cover active knockout backdrop rows; non-device group colour-space/backdrop semantics and full group-space closure remain on the scalar semantic fallback. | No cache state; row bounds, clip opacity bytes, and scalar-oracle equality define deterministic fallback behavior. | Internal high-quality group-flattening path. | Broader transparency-group closure remains incomplete. |
| High-quality blend group scalar rows | High-quality non-normal group flattening used the per-pixel `blend_pixel` dispatcher for row-sized full and compact group windows even when no soft mask, installed SMask, partial clip, or knockout state was active. | `composite_high_quality_blend_row` and `composite_high_quality_blend_row_partial_clip` now handle eligible full-page and compact group windows, including no clip, all-visible clip, binary-clip runs, and partial clips, while preserving the existing `blend_pixel` linear-light blend/source-over oracle for source/group alpha, clip coverage, destination alpha, RGB conversion, and alpha truncation. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::composite_from`; `PixelBuffer::composite_from_at` with non-normal blend modes | Dedicated knockout row entries cover active knockout backdrop rows; non-device group colour-space/backdrop semantics and full group-space closure remain on the scalar semantic fallback. | No cache state; row bounds, clip opacity bytes, blend mode, and scalar-oracle equality define deterministic fallback behavior. | Internal high-quality group-flattening path for separable and non-separable blend modes. | Broader transparency-group closure remains incomplete. |
| High-quality cached RGBA scalar rows | High-quality Normal cached RGBA glyph/image fragments used the per-pixel `blend_pixel` dispatcher for row-sized windows even when no SMask, partial clip, or knockout state was active. | `blend_rgba_pixels_at` now routes eligible cached RGBA rows, including no clip, all-visible clip, binary-clip runs, and partial clips, through high-quality normal row oracles while preserving the same linear-light source-over scalar oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::blend_rgba_pixels_at` | Dedicated knockout row entries cover active knockout backdrop rows; full group-space closure remains on the scalar semantic fallback. | No cache state; row bounds, clip opacity bytes, and scalar-oracle equality define deterministic fallback behavior. | Internal cached Type 3 glyph/image fragment path. | Broader transparency-group closure remains incomplete. |
| High-quality blend cached RGBA scalar rows | High-quality non-normal cached RGBA glyph/image fragments used the per-pixel `blend_pixel` dispatcher for row-sized windows even when no SMask or knockout state was active. | `blend_rgba_pixels_at` now routes eligible high-quality non-normal cached RGBA rows, including no clip, all-visible clip, binary-clip runs, and partial clips, through `composite_high_quality_blend_row` or `composite_high_quality_blend_row_partial_clip` while preserving the same linear-light blend/source-over scalar oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::blend_rgba_pixels_at` with non-normal blend modes | Dedicated knockout row entries cover active knockout backdrop rows; full group-space closure remains on the scalar semantic fallback. | No cache state; row bounds, clip opacity bytes, blend mode, and scalar-oracle equality define deterministic fallback behavior. | Internal cached Type 3 glyph/image fragment path for separable and non-separable blend modes. | Broader transparency-group closure remains incomplete. |
| High-quality alpha-mask scalar rows | High-quality Normal glyph/image/stroke alpha-mask paints used the per-pixel `blend_pixel` dispatcher for row-sized windows and partial-clip coverage even when no SMask or knockout state was active. | `blend_alpha_mask` now routes eligible alpha-mask rows, including no clip, all-visible clip, binary-clip runs, and partial clips, through `blend_alpha_mask_run_high_quality_normal` or `blend_alpha_mask_run_high_quality_normal_partial_clip` while preserving the same linear-light source-over scalar oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::blend_alpha_mask`; `PixelBuffer::blend_alpha_mask_strided` | Dedicated knockout row entries cover active knockout backdrop rows; full group-space closure remains on the scalar semantic fallback. | No cache state; row bounds, clip opacity bytes, and scalar-oracle equality define deterministic fallback behavior. | Internal alpha-mask paint path. | Broader transparency-group closure remains incomplete. |
| High-quality blend alpha-mask scalar rows | High-quality non-normal glyph/image/stroke alpha-mask paints used the per-pixel `blend_pixel` dispatcher for row-sized windows even when no SMask or knockout state was active. | `blend_alpha_mask` and `blend_alpha_mask_strided` now route eligible high-quality non-normal alpha-mask rows, including no clip, all-visible clip, binary-clip runs, and partial clips, through `blend_alpha_mask_run_high_quality_blend` or `blend_alpha_mask_run_high_quality_blend_partial_clip` while preserving the same linear-light blend/source-over scalar oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::blend_alpha_mask`; `PixelBuffer::blend_alpha_mask_strided` with non-normal blend modes | Dedicated knockout row entries cover active knockout backdrop rows; full group-space closure remains on the scalar semantic fallback. | No cache state; row bounds, clip opacity bytes, mask bytes, blend mode, and scalar-oracle equality define deterministic fallback behavior. | Internal alpha-mask paint path for separable and non-separable blend modes. | Broader transparency-group closure remains incomplete. |
| High-quality solid-fill scalar rows | High-quality Normal rectangle fills used the per-pixel `blend_pixel` dispatcher for translucent row-sized windows and for partial-clip coverage even when no SMask or knockout state was active. | `fill_rect` now routes eligible solid rows, including no clip, all-visible clip, binary-clip runs, and partial clips, through `blend_solid_run_high_quality_normal` or `blend_solid_run_high_quality_normal_partial_clip` while preserving the same linear-light source-over scalar oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::fill_rect` | Dedicated knockout row entries cover active knockout backdrop rows; full group-space closure remains on the scalar semantic fallback. | No cache state; row bounds, clip opacity bytes, and scalar-oracle equality define deterministic fallback behavior. | Internal solid rectangle paint path. | Broader transparency-group closure remains incomplete. |
| High-quality blend solid-fill scalar rows | High-quality non-normal solid rectangle fills used the per-pixel `blend_pixel` dispatcher for row-sized windows even when no SMask or knockout state was active. | `fill_rect` now routes eligible high-quality non-normal solid rows, including no clip, all-visible clip, binary-clip runs, and partial clips, through `blend_solid_run_high_quality_blend` or `blend_solid_run_high_quality_blend_partial_clip`, preserving the same linear-light blend/source-over scalar oracle without the Normal-mode opaque-source shortcut. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::fill_rect` with non-normal blend modes | Dedicated knockout row entries cover active knockout backdrop rows; full group-space closure remains on the scalar semantic fallback. | No cache state; row bounds, clip opacity bytes, blend mode, and scalar-oracle equality define deterministic fallback behavior. | Internal high-quality solid rectangle paint path for separable and non-separable blend modes. | Broader transparency-group closure remains incomplete. |
| High-quality SMask solid-fill scalar rows | High-quality solid rectangle fills with an installed SMask used the per-pixel `blend_pixel` dispatcher even when knockout state was absent. | `fill_rect` now routes eligible high-quality solid rows with installed SMask through `blend_solid_run_high_quality_masked`, fusing source alpha, SMask bytes, optional clip opacity, and the active blend mode while preserving the same linear-light blend/source-over scalar oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::fill_rect` with installed `PixelBuffer::smask` | Dedicated knockout row entries cover active knockout backdrop rows; full group-space closure remains on the scalar semantic fallback. | No cache state; row bounds, SMask bytes, clip opacity bytes, blend mode, and scalar-oracle equality define deterministic fallback behavior. | Internal high-quality solid rectangle paint path for Normal and non-normal blend modes under installed SMask. | Broader transparency-group closure remains incomplete. |
| High-quality SMask RGBA, alpha-mask, and group soft-mask rows | High-quality cached RGBA paints, glyph/stroke alpha-mask paints, and full/compact group flattening with active masks still used the per-pixel `blend_pixel` dispatcher even when knockout state was absent. | `HighQualityMaskSources`, `composite_high_quality_masked_row`, and `blend_alpha_mask_run_high_quality_masked` now route direct cached RGBA rows, direct alpha-mask rows, full-page group soft-mask rows, and compact group soft-mask rows through scalar high-quality mask row oracles. The helpers fuse source alpha, group alpha where applicable, installed SMask bytes for direct paints, group soft-mask bytes for group flattening, optional clip opacity, and active blend mode while preserving the same linear-light blend/source-over scalar oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::blend_rgba_pixels_at`; `PixelBuffer::blend_alpha_mask`; `PixelBuffer::blend_alpha_mask_strided`; `PixelBuffer::composite_from`; `PixelBuffer::composite_from_at` | Dedicated knockout row entries cover active knockout backdrop rows; non-device group colour-space/backdrop semantics and full transparency-group closure remain on the scalar semantic fallback. | No cache state; row bounds, mask bytes, clip opacity bytes, blend mode, group alpha, and scalar-oracle equality define deterministic fallback behavior. | Internal high-quality direct paint and group-flattening mask paths. | Broader transparency-group closure remains incomplete. |
| High-quality knockout solid-fill rows | High-quality solid rectangle fills inside an active knockout backdrop used the per-pixel `blend_pixel` dispatcher even for row-sized direct paint windows. | `fill_rect` now routes eligible high-quality knockout solid rows through `blend_solid_run_high_quality_knockout`, reading destination RGB/alpha from the stored knockout backdrop and fusing source alpha, installed SMask bytes, optional clip opacity, and active blend mode while preserving the same linear-light blend/source-over scalar oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::fill_rect` with `PixelBuffer::knockout_backdrop` | Non-device group colour-space/backdrop compositing and full group-space closure remain on the scalar semantic fallback. | No cache state; row bounds, knockout backdrop pixels, SMask bytes, clip opacity bytes, blend mode, and scalar-oracle equality define deterministic fallback behavior. | Internal high-quality knockout group direct solid paint path. | Broader knockout/transparency-group closure remains incomplete. |
| High-quality knockout RGBA, alpha-mask, and group soft-mask rows | High-quality cached RGBA paints, alpha-mask paints, and full/compact group soft-mask composites inside an active knockout backdrop used the per-pixel `blend_pixel` dispatcher even for row-sized direct and group windows. | `blend_rgba_pixels_at`, `blend_alpha_mask_strided`, `composite_from`, and `composite_from_at` now route eligible high-quality knockout rows through `composite_high_quality_masked_row_with_backdrop` and `blend_alpha_mask_run_high_quality_knockout`, reading destination RGB/alpha from the stored knockout backdrop and fusing source alpha, installed SMask bytes, group soft-mask bytes, optional clip opacity, group alpha, and active blend mode while preserving the same linear-light blend/source-over scalar oracle. | `crates/engine/src/render/buffer.rs` | `PixelBuffer::blend_rgba_pixels_at`; `PixelBuffer::blend_alpha_mask`; `PixelBuffer::blend_alpha_mask_strided`; `PixelBuffer::composite_from`; `PixelBuffer::composite_from_at` with `PixelBuffer::knockout_backdrop` | Non-device group colour-space/backdrop compositing and full group-space closure remain on the scalar semantic fallback. | No cache state; row bounds, knockout backdrop pixels, SMask bytes, soft-mask bytes, clip opacity bytes, blend mode, group alpha, and scalar-oracle equality define deterministic fallback behavior. | Internal high-quality knockout group direct paint and group-flattening paths. | Broader knockout/transparency-group closure remains incomplete. |
| Transparency group color-space policy | Active Form transparency groups and SMask groups ignored `/Group /CS` unless luminosity `/BC` needed a direct name. Unsupported or resource-named group spaces could therefore continue under an implicit RGB/backdrop assumption. | `transparency_group_color_space_policy` now validates direct, indirect, and resource-resolved group `/CS` before Form group or SMask group rendering. DeviceGray, DeviceRGB/sRGB, and DeviceCMYK are accepted; missing `/CS` keeps the existing default/inherited device policy; opaque normal Form groups and `/S /Alpha` SMask `/G` groups with no `/BC` can render well-formed CalGray, CalRGB, Lab, structurally exact Indexed, or structurally valid ICCBased `/CS` values through an explicit non-device-inert policy; `/S /Luminosity` SMask `/BC` resolves well-formed CalGray, CalRGB, Lab, Indexed, ICCBased, Separation, and DeviceN group spaces through the named-color/CMM resolver; malformed arrays, missing resources, unresolved references, malformed Indexed palettes, malformed tint transforms, malformed ICC profile references or `/N` metadata, alpha SMask non-device groups with `/BC`, alpha-bearing non-device Forms, and richer non-device group spaces fail typed before offscreen allocation, mask rendering, or backdrop color conversion. | `crates/engine/src/render/page_renderer.rs` | `RenderState::handle_do_form_group`; `RenderState::apply_smask`; `transparency_group_color_space_policy`; `smask_backdrop_color` | Pattern, alpha SMask non-device groups with `/BC`, and alpha/backdrop-observable non-device group color spaces still require true group color-space compositing/backdrop implementation rather than silent approximation. | Group policy is derived from merged Form/SMask resources plus the document reader before rendering; non-device luminosity `/BC` cache keys include the resolved color-space fingerprint. | Internal Form XObject transparency groups and ExtGState SMask `/G` groups. | Non-device group compositing outside the CalGray/CalRGB/Lab/Indexed/ICCBased inert Form/alpha-SMask and luminosity-backdrop subsets and full transparency-group closure remain incomplete. |
| Separable solid mixed-destination rows | Separable `fill_rect` rows over non-opaque destination alpha were excluded by the opaque-destination probe and fell back to per-pixel `blend_pixel`. | `fill_rect` now routes guarded separable solid rows through `blend_separable_src_over_run` and `blend_separable_src_over_partial_clip_run`; opaque rows keep the existing optimized separable helpers, while mixed-destination rows use portable `f32x4` source-over math that preserves the scalar blend oracle for source alpha, clip alpha, destination alpha, RGB rounding, and alpha truncation, with scalar tails. Runtime capabilities disclose `separable_solid_mixed_destination_f32x4_rows`. | `crates/engine/src/render/buffer.rs`, `crates/engine/src/runtime.rs` | `PixelBuffer::fill_rect`; `runtime_capabilities_for` | SMask, knockout, high-quality/group-space blending, and non-separable modes keep their existing semantic fallback paths. | No cache state; row bounds, clip opacity bytes, and scalar-equivalence guards define deterministic fallback behavior. | Internal solid-fill separable blend path and runtime capability JSON. | Full non-normal/high-quality group-space closure remains incomplete. |
| WASM SIMD premultiply RGBA/BGRA contract rows | Premultiplied caller-owned RGBA/BGRA rows used only scalar row encoding. | `render-simd` now exposes `premultiply_rgba` and `premultiply_bgra8`; wasm32+`simd128` premultiplies four straight RGBA pixels per vector with scalar-equivalence guards, and the core row encoder calls them for simple premultiplied RGBA/BGRA contract rows before scalar fallback. | `crates/render-simd/src/lib.rs`, `crates/engine/src/engine.rs` | `wellfriendpdf_render_simd::{premultiply_rgba,premultiply_bgra8}`; `ContentEngine::render_page_into_buffer` / `render_page_into_buffer_with_font_substitution_report` for non-grayscale, non-byte-reversed premultiplied RGBA/BGRA rows | Non-wasm and unsupported rows decline to scalar fallback; grayscale and byte-reversed premultiplied rows remain scalar. | No cache state; row bounds and scalar guards define exact fallback behavior. | Rust core caller-owned surface path and WASM target source. | Runtime wasm test-runner execution and full SIMD category coverage remain incomplete. |
| CPU/WASM SIMD unpremultiply RGBA library rows | Unpremultiplication was listed as a remaining SIMD gap and initially had no native public row path. | `render-simd` now exposes `unpremultiply_rgba`; native x86/x86_64 builds dispatch through an SSE2 batched four-pixel load/store row path when available, and wasm32+`simd128` uses the parallel load/store group path. Both preserve scalar-equivalent reciprocal conversion, exact alpha-zero handling, scalar tails, and guarded scalar checks. It is not wired into `render_page_into_buffer` because `PixelBuffer` stores straight RGBA and the contract encoder must not unpremultiply those rows. | `crates/render-simd/src/lib.rs` | `wellfriendpdf_render_simd::unpremultiply_rgba` | Non-SSE2 native hosts, non-wasm non-x86 hosts, and unsupported rows decline to scalar fallback; unsupported conversions remain scalar. | No cache state; row bounds and scalar guards define exact fallback behavior. | Native/WASM target source and library callers that already hold associated RGBA rows. | Runtime wasm test-runner execution and full SIMD category coverage remain incomplete. |
| WASM SIMD RGB/BGR/BGRA channel rows | Simple caller-owned RGB/BGR/BGRA rows used scalar channel-copy/reorder loops. | `render-simd` now exposes `rgba_to_rgb8`, `rgba_to_bgr8`, and `rgba_to_bgra8`; wasm32+`simd128` uses byte shuffles for four-pixel groups, and the core row encoder calls them for non-grayscale, non-byte-reversed RGB/BGR rows plus straight/opaque BGRA rows before scalar fallback. | `crates/render-simd/src/lib.rs`, `crates/engine/src/engine.rs` | `wellfriendpdf_render_simd::{rgba_to_rgb8,rgba_to_bgr8,rgba_to_bgra8}`; `ContentEngine::render_page_into_buffer` / `render_page_into_buffer_with_font_substitution_report` for simple RGB/BGR/BGRA contract surfaces | Premultiplied BGRA, grayscale, and byte-reversed rows stay on the scalar exact path; unsupported/non-wasm rows decline to scalar fallback. | No cache state; row bounds and scalar guards define exact fallback behavior. | Rust core caller-owned surface path and WASM target source. | Runtime wasm test-runner execution and full SIMD category coverage remain incomplete. |
| WASM SIMD copy/opaque RGBA rows | Straight and opaque RGBA caller-owned rows used scalar copy/alpha-force loops. | `render-simd` now exposes `copy_rgba` and `rgba_to_opaque_rgba`; wasm32+`simd128` uses direct vector load/store or byte shuffles for four-pixel groups, and the core row encoder calls them for non-grayscale, non-byte-reversed straight/opaque RGBA rows before scalar fallback. | `crates/render-simd/src/lib.rs`, `crates/engine/src/engine.rs` | `wellfriendpdf_render_simd::{copy_rgba,rgba_to_opaque_rgba}`; `ContentEngine::render_page_into_buffer` / `render_page_into_buffer_with_font_substitution_report` for simple RGBA contract surfaces | Grayscale and byte-reversed rows stay on the scalar exact path; unsupported/non-wasm rows decline to scalar fallback. | No cache state; row bounds and scalar guards define exact fallback behavior. | Rust core caller-owned surface path and WASM target source. | Runtime wasm test-runner execution and full SIMD category coverage remain incomplete. |
| WASM SIMD RGB image-span rows | Cached/scaled RGB image spans expanded into opaque RGBA destination rows through scalar loops. | `render-simd` now exposes `rgb8_to_opaque_rgba`; wasm32+`simd128` load/store groups expand RGB source triples into opaque RGBA rows with scalar-equivalence guards. The unclipped and one-to-one binary-clipped RGB image row writers call it before scalar fallback. | `crates/render-simd/src/lib.rs`, `crates/engine/src/render/buffer.rs` | `wellfriendpdf_render_simd::rgb8_to_opaque_rgba`; `PixelBuffer::write_opaque_rgb_run_unclipped`; one-to-one binary-clipped RGB image row writes | Non-wasm and unsupported rows decline to scalar fallback; unsupported blend kernels and broader non-normal/high-quality SIMD rows remain separate incomplete categories. | No cache state; row bounds and scalar guards define exact fallback behavior. | Internal RGB image-span paint path and WASM target source. | Runtime wasm test-runner execution and full SIMD category coverage remain incomplete. |
| Native SSSE3 RGB/BGR channel and RGB image-span rows | Native x86/x86_64 builds still declined simple RGB/BGR contract rows and RGB image-span expansion that already had wasm `simd128` row helpers. | `render-simd` now dispatches `rgba_to_rgb8`, `rgba_to_bgr8`, and `rgb8_to_opaque_rgba` through SSSE3 byte-shuffle kernels when available. The kernels pack four RGBA pixels into RGB/BGR triplets or expand four RGB triplets into opaque RGBA, retain scalar debug oracles, and keep scalar tails for uneven rows. | `crates/render-simd/src/lib.rs`, `crates/engine/src/engine.rs`, `crates/engine/src/render/buffer.rs` | `wellfriendpdf_render_simd::{rgba_to_rgb8,rgba_to_bgr8,rgb8_to_opaque_rgba}`; `ContentEngine::render_page_into_buffer` / `render_page_into_buffer_with_font_substitution_report`; `PixelBuffer::write_opaque_rgb_run_unclipped`; one-to-one binary-clipped RGB image row writes | Non-SSSE3 native hosts, non-wasm non-x86 hosts, and unsupported row shapes decline to scalar fallback; unsupported blend kernels and broader non-normal/high-quality SIMD rows remain separate incomplete categories. | No cache state; row bounds and scalar guards define exact fallback behavior. | Rust core caller-owned surface path and internal RGB image-span paint path. | Full SIMD category coverage remains incomplete for remaining grayscale/native non-normal/high-quality paths and external runtime matrices. |
| CPU/WASM SIMD common separable blend rows | Common separable opaque-source-over-opaque-destination blend rows used the engine portable-wide path with no native `render-simd` helper and no wasm `simd128` helper. | `render-simd` now exposes `blend_separable_opaque_destination` for all PDF separable blend modes: Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn, HardLight, SoftLight, Difference, and Exclusion. Native x86/x86_64 SSE2 covers the full set with integer lanes for the integer byte formulas and float lanes for ColorDodge, ColorBurn, and SoftLight; wasm32+`simd128` uses true lane math for the full separable set; the engine's common separable opaque-destination row path calls it before the existing portable-wide fallback. | `crates/render-simd/src/lib.rs`, `crates/engine/src/render/buffer.rs` | `wellfriendpdf_render_simd::blend_separable_opaque_destination`; `PixelBuffer::fill_rect` and cached RGBA fragment paths when they reach opaque-destination separable row compositing | Non-SSE2 native hosts, non-wasm non-x86 hosts, and rows declined by the helper keep the existing portable/scalar fallback; engine solid fills and cached RGBA rows over non-opaque destinations now use portable `f32x4`, while non-separable modes, high-quality blending, and group-space closure remain incomplete. | No cache state; row bounds and scalar guards define exact fallback behavior. | Internal separable blend row path plus native/WASM target source. | The separable helper has a Node-executed `.work` wasm cdylib smoke; general wasm-bindgen-test harness execution and full SIMD category coverage remain incomplete. |
| Direct PDFium harness | Harness source existed but lacked several requested control/manifest fields and accepted `--forms 1` without actually drawing widget form appearances. | Direct C harness supports one/all pages, page box, matrix, clip, DPI, explicit dimensions, annotation flags, active PDFium form-fill rendering for the supported media-box viewport path, raw BGRA/BGRx output, output hash, worker-count metadata, JSONL typed failures, and a version manifest. | `tools/pdfium-harness/render_page.c`, `CMakeLists.txt`, `smoke.sh` | `wellfriend-pdfium-harness` executable when built against an official PDFium SDK. | Typed JSONL page/document/manifest/form-fill/form-policy/bitmap-size/output-path errors. | Manifest records selection, output, PDF file version, public-API version availability, requested matrix/clip, background, serial worker execution, and form-fill API posture; per-page JSONL records expose actual output dimensions, raw byte count, effective matrix/clip, output kind, `forms_rendered`, and `form_rendering`. | Standalone C/CMake tool. | Local PDFium SDK build/runtime smoke was not run because no SDK was provisioned and no download is allowed; a compile-only MSVC `/W4 /WX` source guard was run against stub public PDFium headers, and form rendering with custom matrix, clip, or crop-box selection is explicitly refused instead of rendered through an incompatible PDFium viewport call. |
| Visual normalization harness | Compact metrics existed, but render-context normalization/classification and explicit RGB-only color-difference metrics were incomplete. | The compact visual-diff harness now supports canonical RGBA policy reporting, raw stride and packed byte-order normalization, explicit straight/premultiplied/opaque alpha semantics, active render-context background compositing, grayscale luma conversion, exact post-rotation dimension validation, render-context JSON sidecars, deterministic structured font-environment canonicalization/fingerprints, media/crop/matrix/rotation/dimensions/channel/alpha/background/annotation/form/OCG/color/smoothing/font/malformed-policy metadata, context-difference taxonomy, RGB-only `color_mae`/`color_rmse`/`max_color_delta`, and per-region likely-cause classifications. | `tools/renderer-visual-diff/visual_diff.py`, `normalization-manifest.json`, `test_visual_normalization.py` | `python -m pytest tools/renderer-visual-diff/test_visual_normalization.py -q`; `visual_diff.py --left ... --right ... --left-alpha-mode ... --right-alpha-mode ... --left-byte-order ... --right-byte-order ... --left-context-json ... --right-context-json ...` | No fallback; the tool classifies expected differences and refuses invalid dimensions/stride/masks/alpha/background/grayscale/font-environment modes. | Canonical comparison report records normalization policy, expected dimensions, applied channel and byte order, applied alpha semantics, applied background/grayscale normalization, canonical font-environment fingerprints, context differences, and RGB-only color-difference metrics for later corpus adjudication. | Python tool only. | Large corpus/reference execution, competitor comparisons, and human adjudication remain deferred. |
| SVG/PS regional inline/Form/shading/ExtGState/text-outline regionalization | Form XObjects, shadings, ExtGState, text clipping, and all inline images forced whole-page SVG/PS raster embedding. | The shared vector fallback classifier now decodes/parses vector-safe Form XObjects, including no-op, opaque-inert isolated/knockout transparency-group Forms, opaque normal DeviceGray/DeviceRGB/DeviceCMYK group-colour transparency Forms, and opaque normal well-formed CalGray/CalRGB/Lab/Indexed/ICCBased/Separation/DeviceN group-colour transparency Forms, applies Form matrices, merges scoped resources, enforces depth/cycle guards, clips to BBox, and replays vector-safe Form subprograms natively in both SVG and PostScript/EPS sinks. It also classifies finite affine Image XObject placements including stencil masks, complete finite-affine DeviceGray/DeviceRGB/DeviceCMYK inline image sequences except CCITT/JBIG2 terminal RGB/CMYK declarations, resource-named device color spaces, direct/resource-resolved CalGray/CalRGB/Lab inline image color spaces, direct/resource-resolved Indexed inline image color spaces, direct/resource-resolved ICCBased inline image color spaces, preflighted opaque direct/resource-resolved Separation/DeviceN inline image color spaces, DCT-filtered opaque resource-resolved Separation and one-colorant DeviceN inline images, DCT-filtered calibrated/Indexed/ICCBased inline images, simple inline image masks, simple linear DeviceGray/DeviceRGB/DeviceCMYK plus direct/resource-resolved CalGray/CalRGB/Lab/Separation axial shadings with preserved supported `/Extend` clips, endpoint-sampled contained non-unit shading `/Domain` support, function-domain-clipped stop-list support, and transformed shading `/BBox` clipping, finite/invertible affine-transformed DeviceGray/DeviceRGB/DeviceCMYK plus direct/resource-resolved CalGray/CalRGB/Lab/Separation radial shadings including finite concentric non-default `/Extend` clips through the same simple Type 2 endpoint-sampling, continuous Type 3 stitching stop-list, function-domain clipping, and BBox path, simple finite-matrix `/PatternType 2` shading-pattern fill/stroke path paints and glyph-outline text paints with shading BBox clip intersection, simple colored and color-inheriting uncolored tiling-pattern path paints, glyph-outline text paints, vector-safe resource-bearing image/Form/shading/inline-image tile cells, plus tiling-pattern painted inline/Image XObject stencil masks, SVG normal-blend scalar `CA`/`ca` ExtGState opacity for path, text, regional image, and direct shading output, stateful exact PostScript zero-alpha `CA`/`ca` no-op paint, fully transparent regional RGBA no-op regions, binary 0/255 regional RGBA alpha clips, safe line-style/dash/font/flatness/smoothness/default-state/identity-transfer/first-supported-normal-blend-array ExtGState dictionaries, styled stroked/fill-stroke text outlines, font-resolved text-clipping glyph clips, and overwritten dead pattern color state, then lets the SVG/PS sinks emit bounded transformed raster regions with inert SVG data attributes or PostScript comments naming the regional kind and device-space bounds for image XObject and inline-image segments, PostScript `imagemask` stencil regions, native SVG `<linearGradient>`/`<radialGradient>`/PostScript `shfill`, clipped SVG gradient geometry, bounded native tiling-pattern tile replay, applied vector line/dash/opacity/no-op-alpha state, both fill and stroke text-outline paints, shading-pattern glyph-outline text paints, or native glyph clips while preserving surrounding vector operators. SVG stacked path/Form BBox/text clips now compose by registering child `clipPath` definitions that reference the previous active clip. | `render/vector_fallback.rs`, `render/svg.rs`, `render/postscript.rs`, `tests/regional_vector_fallback.rs` | `ContentEngine::render_page_svg`, `render_page_ps`, `render_document_ps`, `render_page_eps` | Alpha/backdrop-observable semantic transparency groups, alpha/backdrop-observable or richer non-device group-colour transparency Forms, visible PostScript fractional-alpha/non-normal-blend paint, fractional regional RGBA, SVG soft masks or semantic transparency groups outside native blend/opacity, visible PostScript non-normal-blend paint, soft masks, non-identity transfer functions, overprint-enabled ExtGState, invalid overprint mode, stroke-adjustment-enabled state, alpha-source-enabled state, text-knockout-disabled state, unresolved/preflight-unavailable text-clipping render modes, non-linear/mesh/malformed radial shadings, unsupported radial Extend geometry, malformed or degenerate shading BBoxes, unsupported named color spaces, advanced pattern paint and resource-bearing tiling cells outside the vector-safe image/Form/shading/inline-image subset, unsupported advanced pattern-space transforms, active patterned text outside the supported shading/tiling glyph-outline subset, unsafe nested Forms, degenerate/unresolvable image transforms, active pattern-painted stencil masks outside the supported shading/tiling subset, `/None` or unresolved Separation/DeviceN/tint-space inline-image resources, multi-component DeviceN terminal-codec tint-space inline images, non-DCT terminal-codec tint-space inline images, CCITT/JBIG2 terminal inline images declared as non-gray device color spaces, non-DCT terminal-codec calibrated/Indexed/ICCBased inline images, and unsupported inline-image color/filter shapes still fail closed to whole-page raster embedding. | Form streams are decoded with the normal lossless stream decoder and scoped resource merge; recursion is bounded at 8; transparency-group Forms with explicit DeviceGray/DeviceRGB/DeviceCMYK `/CS`, well-formed CalGray/CalRGB/Lab/Indexed/ICCBased/Separation/DeviceN `/CS`, `/I true`, or `/K true` are accepted only when a conservative Form scan starting from inherited caller graphics state proves opaque normal non-pattern vector-safe paint; malformed, `/None`, unresolved, alpha/backdrop-observable, and richer non-device group colour spaces remain conservative; inline image payloads use the existing inline decoder and can resolve resource names to simple device color spaces, CalGray/CalRGB/Lab arrays, Indexed arrays, ICCBased arrays, preflighted opaque Separation/DeviceN arrays, DCT-filtered opaque resource-resolved Separation and one-colorant DeviceN inline images, and DCT-filtered calibrated/Indexed/ICCBased inline images; simple masks and Image XObject stencils paint through the active fill color and Decode array; finite image placements carry an SVG/PostScript affine matrix plus bounded device box and inert region metadata for focused order/bounds checks; simple axial/radial shading classification resolves the shading/function dictionaries, samples contained linear Type 2 shading-domain endpoints, inserts native stops at in-range Type 2 function-domain boundaries and continuous Type 3 stitching bounds, carries non-degenerate BBoxes into native clips, and rejects ambiguous, discontinuous, or non-linear functions, malformed BBoxes, or malformed/degenerate/unsupported radial geometry instead of approximating; simple finite-matrix shading patterns reuse the same vector-safe shading subset for path fill/stroke paints and glyph-outline text paints and reject unsupported advanced pattern-space transforms; simple tiling patterns require complete decoded streams, finite geometry, bounded visible cells, and nested resource programs that either are absent or classify as vector-safe image/Form/shading/inline-image replay; nested `/Pattern` color spaces remain unsupported; ExtGState classification accepts finite non-negative font arrays after indirect resource normalization, first-supported normal-compatible `/BM` arrays, and four-`/Identity` `TR`/`TR2` arrays, and rejects unknown keys, visible PostScript fractional-alpha paint, visible non-normal-blend paint, soft masks, non-identity transfer functions, invalid dash arrays, negative/non-finite dash values, non-finite/negative dash phases, all-zero dash patterns, negative/non-finite flatness or smoothness values, overprint-enabled dictionaries, invalid overprint mode, `SA true`, `AIS true`, and `TK false`; text-clipping render modes require reader-backed font preflight and fail closed when glyph outlines cannot be resolved; unsupported active pattern paint outside the simple shading/tiling subsets remains fail-closed but setting pattern state and overwriting it before paint does not force a whole-page fallback. | Rust engine SVG/PS/EPS APIs and CLI output routes. | Full SVG/PS regional fallback remains incomplete for non-linear/mesh/malformed radial gradients, unsupported radial Extend geometry, malformed or degenerate shading BBoxes, unsupported named color spaces, advanced pattern paint and resource-bearing tiling cells outside the vector-safe image/Form/shading/inline-image subset, unsupported advanced pattern-space transforms, active patterned text outside the supported shading/tiling glyph-outline subset, alpha/backdrop-observable semantic transparency groups, alpha/backdrop-observable or richer non-device group-colour transparency Forms, visible PostScript fractional-alpha/non-normal-blend paint, fractional regional RGBA, unresolved/preflight-unavailable text clipping, `/None` or unresolved Separation/DeviceN/tint-space inline image resources, multi-component DeviceN terminal-codec tint-space inline images, non-DCT terminal-codec tint-space inline images, CCITT/JBIG2 terminal inline images declared as non-gray device color spaces, non-DCT terminal-codec calibrated/Indexed/ICCBased inline images, and unsupported local regions. |
| CLI render-contract controls | `render` used the legacy raster route and lacked contract/caller-surface controls. | Raster `render` can route through schema-v1 `RenderContract`, emit raw caller-owned surfaces, include contract and font-substitution sidecars, read an exact contract JSON file, and build schema fields for page box, device transform, background, clip, pixel format, alpha mode, grayscale, byte-order, halftone, print-profile, overprint, rendering intent, color management, exactness, determinism, annotation, form, and resource budgets. | `crates/cli/src/main.rs`, `crates/cli/tests/tool_surface.rs`, `crates/engine/src/render/contract.rs`, `crates/engine/src/engine.rs`, `render/font_substitution_report.rs`, `render/page_renderer.rs`, `images/decoder.rs`, `images/smask.rs` | `wellfriendpdf render --render-contract`, `--format raw`, `--write-contract-json`, `--contract-json`, `--font-substitution-report`, `--page-box`, `--device-transform`, `--background`, `--pixel-format`, `--alpha-mode`, `--grayscale`, `--print-profile`, `--overprint`, `--rendering-intent`, `--color-management`, `--exactness`, `--determinism`, `--max-render-pixels`, `--max-decoded-bytes`, `--max-temporary-bytes`, `--max-cache-bytes` | Raw-surface layout flags require `--format raw`; vector SVG/PS/EPS paths reject contract-only flags rather than ignoring them; JSON input cannot be mixed with builder flags. | Sidecar/input JSON records the exact contract including alpha mode; font-substitution sidecar records bounded events and overflow; engine validation enforces max-pixel and max-temporary-byte budgets; clipped contract renders also budget the full-page intermediate plus crop; contract background, all five page boxes, device transform, rendering intent for active ICC/named-color paint and image CMM paths, `OverprintPolicy::Preview` for PDF graphics-state DeviceCMYK overprint preview, exactness refusal/fallback policy, deterministic CPU output satisfying `BestEffortResearch`, decoded-byte scheduler/filter caps for page content streams and downstream decode paths, temporary-surface reservation caps, and byte-cache admission caps are honored. | CLI and Rust engine. | Non-default execution remains incomplete for full native CMM and proof/separation output; separation-preserving overprint remains unsupported, exactness override is honored for current exact-refusal/fallback boundaries, and looser determinism policy is accepted by deterministic CPU output, but universal exact rendering remains incomplete. |
| .NET render-contract builder | .NET callers could request default contract JSON and pass raw JSON to contract render APIs, but had no managed schema-v1 builder or typed overloads. | `RenderContract` now round-trips native contract JSON as typed managed fields, exposes enum-backed schema values plus matrix, background, clip, surface, and resource-budget helpers, validates schema/stride/dimensions/transform/budget shape before serialization, and feeds typed PNG/caller-owned/font-report render overloads. | `bindings/dotnet/WellfriendPdf/RenderContract.cs`, `WellfriendPdfDocument.cs`, `WellfriendPdfSmokeTests.cs`, `.NET README` | `DefaultRenderContract`, `RenderPagePng(RenderContract)`, `RenderPageIntoBuffer(RenderContract, byte[])`, `RenderPagePngWithFontSubstitutionReport(RenderContract)`, `RenderPageIntoBufferWithFontSubstitutionReport(RenderContract, byte[])` | The active CPU renderer still typed-refuses unsupported semantic deviations; native-library-backed .NET raster smoke was not run in this pass. | Managed validation mirrors schema-v1 shape checks for schema version, positive page/dimensions, six finite invertible matrix values, stride minimum, clip non-emptiness, optional-content identity, and max-pixel budget. | .NET source surface; managed build and focused builder unit test pass. | Other language builders and full non-default field execution parity remain incomplete. |
| Java render-contract builder | Java callers could request default contract JSON and pass raw JSON to contract render APIs, but had no dependency-free schema-v1 builder or typed overloads. | `WellfriendPdf.RenderContract` now parses and serializes schema-v1 contract JSON, preserves unsigned matrix bit patterns, exposes typed pixel-format/alpha-mode enums plus matrix, background, clip, surface, and resource-budget helpers, validates schema/stride/dimensions/transform/budget shape before serialization, and feeds typed PNG/caller-owned/font-report render overloads. | `bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java`, `WellfriendPdfSmokeTest.java`, `bindings/java/README.md` | `defaultRenderContract`, `renderPagePng(RenderContract)`, `renderPageIntoBuffer(RenderContract, ByteBuffer)`, `renderPagePngWithFontSubstitutionReport(RenderContract)`, `renderPageIntoBufferWithFontSubstitutionReport(RenderContract, ByteBuffer)`, `WellfriendPdfSmokeTest --contract-builder-only` | The active CPU renderer still typed-refuses unsupported semantic deviations; native-library-backed Java raster smoke was not run in this pass. | Dependency-free Java JSON parser/writer validates the schema-v1 builder path without adding package dependencies. | Java source surface; direct `javac` compile of main+smoke and builder-only smoke pass. | Full non-default field execution parity and external runtime gates remain incomplete. |
| Python render-contract builder | Python callers could request default contract JSON and pass raw JSON to contract render APIs, but had no typed schema-v1 builder or typed overloads. | `RenderContract` now wraps the canonical Rust `RenderContract`, parses/serializes schema-v1 JSON, exposes surface, background, clip, transform, and resource-budget helpers that run engine validation, and the existing contract PNG/caller-buffer/font-report methods accept either typed contracts or legacy JSON strings. | `crates/wellfriendpdf-py/src/lib.rs` | `Document.default_render_contract`, `RenderContract.from_json`, `RenderContract.to_json`, `with_surface`, `with_clip`, `without_clip`, `with_device_transform`, `with_background`, `with_resource_budget`, `render_contract_png(RenderContract)`, `render_contract_into(RenderContract, bytearray)` | The active CPU renderer still typed-refuses unsupported semantic deviations; Python package/native runtime smoke was not run in this pass. | Builder validation uses the canonical Rust contract type and `RenderContract::validate`. | Python source surface; `cargo check -p wellfriendpdf-py --jobs 1` passes. | Full non-default field execution parity and external runtime gates remain incomplete. |
| WASM render-contract builder | WASM callers could request default contract JSON and pass raw JSON to contract render APIs, but had no object wrapper or object-based render methods in the checked-in TypeScript surface. | `WellfriendRenderContract` now wraps the canonical Rust `RenderContract`, parses/serializes schema-v1 JSON, exposes surface, background, clip, transform, and resource-budget helpers that run engine validation, and `WellfriendPdf` adds object-based PNG/caller-buffer/font-report render methods beside the legacy JSON methods. | `crates/wellfriendpdf-wasm/src/lib.rs`, `crates/wellfriendpdf-wasm/wellfriendpdf.d.ts` | `WellfriendRenderContract.fromJson`, `toJson`, `withSurface`, `withClip`, `withoutClip`, `withDeviceTransform`, `withBackground`, `withResourceBudget`, `defaultRenderContract`, `renderContractObjectPng`, `renderContractObjectInto`, `renderContractObjectPngWithFontSubstitutionReport`, `renderContractObjectIntoWithFontSubstitutionReport` | The active CPU renderer still typed-refuses unsupported semantic deviations; wasm-pack/browser/Node package runtime smoke was not run in this pass. | Builder validation uses the canonical Rust contract type and `RenderContract::validate`. | WASM source surface; `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1` passes. | Full non-default field execution parity and external runtime gates remain incomplete. |
| C ABI render-contract handle | C callers could request default contract JSON and pass raw JSON to contract render APIs, but had no owned typed handle or handle-based render methods. | `WellfriendRenderContract` now wraps the canonical Rust `RenderContract`, parses/serializes schema-v1 JSON, exposes surface, background, clip, transform, and resource-budget helpers that run engine validation, reports surface byte length, and adds handle-based PNG/caller-buffer/font-report render methods beside the legacy JSON methods. | `crates/wellfriendpdf-capi/src/lib.rs`, `crates/wellfriendpdf-capi/include/wellfriendpdf.h` | `wellfriendpdf_render_contract_from_json`, `wellfriendpdf_render_contract_to_json`, `wellfriendpdf_render_contract_surface_byte_length`, `wellfriendpdf_render_contract_with_surface`, `wellfriendpdf_render_contract_with_clip`, `wellfriendpdf_render_contract_without_clip`, `wellfriendpdf_render_contract_with_device_transform`, `wellfriendpdf_render_contract_with_background`, `wellfriendpdf_render_contract_with_resource_budget`, `wellfriendpdf_document_render_page_png_with_contract_handle`, `wellfriendpdf_document_render_into_buffer_with_contract_handle`, and font-report handle variants | The active CPU renderer still typed-refuses unsupported semantic deviations; external C consumer build/runtime smoke was not run in this pass. | Builder validation uses the canonical Rust contract type and `RenderContract::validate`; caller-owned buffer length uses `stride * height`. | C ABI source/header; `cargo check -p wellfriendpdf-capi --lib --jobs 1` and focused handle test pass. | Full non-default field execution parity and external runtime gates remain incomplete. |
| Render-time font substitution report binding parity | Bounded report collection existed in Rust render APIs and CLI sidecars, but C/Python/WASM/.NET/Java render entry points lacked report-returning surfaces and events lacked caller-visible coverage/risk metadata. | Fast PNG, contract PNG, and caller-owned contract rendering now have additive report-returning APIs across C ABI, Python, WASM, .NET, and Java source surfaces; each event carries requested PDF font, embedded state, encoding summary, resolution source, selected replacement, bounded required-glyph coverage, missing-glyph count/samples, visual-risk category, extraction/editing impact, font-policy identity, and deterministic risk flags for the first observed fallback text run; persistent font byte/resolver cache keys and derived glyph hashes are salted with render-contract identity and resource budget; reused device glyph masks can paint from bounded atlas pages with public telemetry. | `crates/engine/src/engine.rs`, `crates/engine/src/render/font_substitution_report.rs`, `crates/engine/src/render/page_renderer.rs`, `crates/engine/src/render/buffer.rs`, `crates/wellfriendpdf-capi/src/lib.rs`, `crates/wellfriendpdf-capi/include/wellfriendpdf.h`, `crates/wellfriendpdf-py/src/lib.rs`, `crates/wellfriendpdf-wasm/src/lib.rs`, `crates/wellfriendpdf-wasm/wellfriendpdf.d.ts`, `.NET/Java` wrappers | `render_page_png_fast_with_font_substitution_report`, `wellfriendpdf_document_render_page_png_with_font_substitution_report_json`, `render_with_font_substitution_report`, `renderPagePngWithFontSubstitutionReport`, `RenderPagePngWithFontSubstitutionReportJson`, Java `renderPagePngWithFontSubstitutionReportJson` plus contract/caller-buffer variants | Report only; bundled substitution fallback remains active. | Existing `FontSubstitutionLog` event cap and bounded missing-glyph samples keep report size bounded; contract-scoped font keys prevent stale font/resolver/glyph reuse across render contracts; glyph atlas pages are byte-accounted and emptied when their last entry is evicted. | Rust/C/Python/WASM/.NET/Java source. | Single-flight closure and external binding runtime gates remain incomplete. |
| Transaction/cache invalidation | Cached render dependency recording only bound the page object, narrow invalidation ignored page-only write sets, source-editing transaction IDs did not map to render dependency IDs, and the graph could not express tile-only source invalidation. | Cached page render setup now records page object, content streams, bounded indirect resource references, and selected page dictionary references; `PageResources` preserves indirect font, color-space, ExtGState, and Properties references through page/Form merges; supported source-editing text transactions refresh their post-apply write set from the concrete source mutation report; scene `geometric_block` and `semantic_document` text transactions now route through TextReflow preview/apply paths rather than a blanket stub, forward compact request region/flow/layout/review options, then convert source refs, affected pages, dirty regions, typed refusals, and inverse metadata into the same EditingTransactions invalidation report; advanced-editing Link annotation moves and Ink appearance regeneration now add structured `CacheInvalidationReport` render write-set refs, changed/created object refs, affected pages, and page-space dirty regions; DocumentSubsystems AcroForm value/default/import/reset/create/delete/move reports and secure-mutation incremental form fills now expose structured field/widget/AP render write-set refs, affected pages, and page-space dirty regions where live widget appearances change; transaction invalidation maps PDF refs, structured `[object, generation]`/`{object_number, generation}`/`{ref}` records, and `object-*`/`stream-*` source IDs, deduplicates them, unions mapped object dependencies with explicit affected pages, accepts explicit pixel-space dirty tiles, records source-to-tile graph edges, converts dirty-region report rectangles to render tiles when callers provide the active viewport/grid, scans bounded retained resource ops at raster-cache insertion to record concrete tile edges for resolved font-backed text, color-space-backed paint/text, ExtGState-backed paint/text/inline-image, XObject, shading, pattern-path, pattern-painted text, and active `/OC` marked-content Properties resources, prunes page-scoped decoded-image, scaled-image, soft-mask, mesh-shading, Form-program, tiling-program, and annotation-appearance artifacts on full page invalidation, prunes source-scoped decoded-image, scaled-image, soft-mask, mesh-shading, Form-program, tiling-program, and annotation-appearance artifacts for mapped source IDs, and can evict exact raster tiles without page artifact pruning for tile-scoped invalidation. When explicit dirty tiles are supplied with affected pages or mapped source-page dependencies, page-scoped retained artifacts are still pruned but raster tile invalidation is limited to the explicit/source-tile set instead of expanding to every recorded page tile; Form, mesh-shading, tiling-program, and annotation-appearance keys are salted with active page/contract identity where applicable. The SDK now emits a render-invalidation plan beside transaction apply output, including mapped source IDs, `source_cache_markers`, unmapped reset indicators, next revision, cache entry-point guidance, and optional dirty render tiles from caller-supplied viewport/grid data; C/Python/WASM/.NET/Java and a server multipart route expose that plan. `RenderInvalidationCachePlan::from_json` and `apply_render_invalidation_plan_json_to_cache` parse either the plan body or SDK envelope, normalize string and structured nested write-set refs, and apply exact tile/page/source invalidation or conservative resets to `RenderDocumentCache`. | `render/invalidation.rs`, `render/transaction_invalidation.rs`, `render/page_renderer.rs`, `render/display_list.rs`, `editing_transactions.rs`, `text_reflow.rs`, `advanced_editing.rs`, `document_subsystems.rs`, `secure_mutation.rs`, `sdk.rs`, `crates/server/src/routes/editing_transactions.rs`, C/Python/WASM/.NET/Java binding sources | Cached display-list/page/tile render paths, `ContentEngine::{invalidate_for_transaction,invalidate_for_transaction_with_tiles}`, `dirty_regions_to_render_tiles`, `apply_transaction_with_invalidation`, `RenderInvalidationCachePlan::from_json`, `RenderInvalidationCachePlan::apply_to_cache`, `apply_render_invalidation_plan_json_to_cache`, `apply_scene_text_transaction`, `plan_scene_text_transaction`, `text_reflow::apply_reflow_region`, `text_reflow::apply_reflow_document`, `advanced_editing::move_link_annotation_rect_pdf`, `advanced_editing::fit_annotation_ink_pdf`, `apply_document_subsystems`, `secure_mutation::incremental_form_value_update_pdf`, `secure_mutation::apply_signature_preserving_form_fill`, `sdk::editing_transactions_transaction_apply_with_render_invalidation_json`, C ABI `wellfriendpdf_document_editing_transactions_transaction_apply_with_render_invalidation_json`, Python `editing_transactions_transaction_apply_with_render_invalidation`, WASM `editing_transactionsTransactionApplyWithRenderInvalidation`, .NET `EditingTransactionsTransactionApplyWithRenderInvalidation`, Java `editing_transactionsTransactionApplyWithRenderInvalidation`, and server `/api/v1/editing-transactions/apply-with-render-invalidation`. | Unknown string or structured object refs still force conservative full reset; conservative page-level resource dependencies remain active for structural and transitive uncertainty; structured invalidation is now emitted by Link annotation move, Ink appearance regeneration, DocumentSubsystems AcroForm actions, and secure-mutation incremental form fills. | Ref traversal is depth/count bounded; string and structured object/stream source IDs map through the canonical object table; explicit affected tiles, source-tile dependencies, converted dirty regions, compact TextReflow option forwarding, advanced annotation object/page/dirty-region write-set reports, AcroForm field/widget/AP invalidation reports, bounded retained resource-op tile edges including active marked-content Properties edges, source-scoped artifact markers, binding-safe invalidation-plan `source_cache_markers`, and shared plan-application helpers feed exact raster/artifact eviction by cache-owning callers; cache keys carry revision/render-contract/resource-budget identity where active. | Rust engine cache path plus Rust SDK, C ABI/header, Python, WASM/TypeScript declarations, .NET, Java, and server source surfaces. | Shared-resource transitive invalidation matrices, safe narrowing of conservative page-level resource dependencies, external cache-handle runtime matrix validation and broader shared-resource narrowing remain incomplete. |

## Required subsystem status

| Item | Status |
|---|---|
| Packed backend-plan status | PARTIAL_ADVANCED: packed vector plans exist, and image XObjects, including first-level named image color-space payloads, Form XObjects, shadings, ExtGState, `Tf` font dictionaries, named color-space objects, `SCN`/`scn` pattern objects, BDC/DP Properties objects, inline `/OC` dictionaries, supported Form XObject stream plans including transparency-group interiors, supported tiling-pattern stream plans, supported annotation appearance stream plans, and guarded Type 3 CharProc plans with clips, typed resources, `d0`/`d1` metrics, and vector paths with per-paint inherited/explicit color handling now pre-resolve/cache/evaluate resource handles/dictionaries when compiled with page/form resources; marked-content visibility boundaries force packed adapter replay when vector clip/fill/stroke visibility state is needed; text shaping/font-program closure, unsupported/advanced pattern subplans, alternate optional-content configuration/public-selection plans, complete transparency/group semantics, complete colour transforms, and complete backend-native payloads remain incomplete. |
| Retained immediate-delegation status | COMPLETE_ACTIVE_SOURCE: default, cached, contract full-page, retained tile, direct tile, and progressive supported-list rendering route through packed plans with caller cancellation propagated into typed descriptor replay; unsupported retained state descriptors and packed compile refusals record fatal renderer errors instead of logging/skipping, and packed-plan dispatch stops after those typed refusals before later operations can paint partial output; compatibility and high-quality/exact page/display-list/tile/band/progressive unsupported-list paths now refuse unsupported retained display-list replay with typed `UnsupportedFeature` instead of using canonical raw/immediate dispatch. |
| Hot/cold display-list status | PARTIAL_ADVANCED: vector hot/cold arenas exist; packed cold tables are diagnostics-only with no raw `ContentOperation` payload variants; retained state ops use typed `GraphicsStateDescriptor` payloads; retained stateful pattern paths use typed `PatternPathDescriptor` payloads; retained text/text-state ops and inline images use typed payloads; retained Image XObject, Form XObject, and shading ops store typed resource names, covered XObject handles, and first-level named Image XObject color-space payloads where available; covered XObject/shading/ExtGState/font/color-space/pattern/Properties retained ops carry resource-aware descriptors; `RenderPlan` batches now split contiguous descriptor-backed and pure-vector hot-op runs; identity `cm` no-op transforms fold independently, adjacent duplicate absolute state setters fold only when their typed descriptors are identical, and adjacent same-slot absolute setters replace the earlier setter when no paint/scope/save/restore/non-idempotent operation intervenes, while non-identity `cm`, relative text movement, scope markers, save/restore, compile refusals, ExtGState overwrites, and paint/native ops remain unfused; `BackendPlanArenaReport` and `BackendDocumentPlanArenaReport` expose source-operation, folded-no-op-state, folded-duplicate-state, and folded-overwritten-state counters; `RenderDocumentView::backend_plan_arena_report` exposes per-page hot/cold arena and descriptor/refusal counts through the render-view, SDK, CLI, server, and binding source boundaries, and `RenderDocumentView::backend_document_plan_arena` owns CPU `RenderPlan` values for every page with aggregate Rust SDK JSON reporting. Parser-level page programs remain compatibility source inputs; broader transform folding and backend-specialized GPU/printer/debug payload arenas remain future backend/platform work. |
| Transaction invalidation status | PARTIAL_ADVANCED: affected object refs, source-editing `object-*`/`stream-*` IDs, TextReflow-routed `geometric_block`/`semantic_document` scene text transactions with compact request region/flow/layout/review option forwarding, advanced-editing Link/Ink annotation object/page/dirty-region write-set reports, DocumentSubsystems AcroForm action invalidation reports, secure-mutation incremental form-fill field/widget/AP reports, affected pages, explicit dirty tiles, source-to-tile graph edges, source-to-source dependency expansion, caller-supplied viewport/grid dirty-region conversion, preserved indirect font/color-space/ExtGState/Properties refs and Image XObject stream dictionaries, automatic bounded retained resource-op tile edge extraction for resolved font/XObject/shading/pattern/color-space/named Image XObject color-space/referenced image Mask and SMask color-space/retained inline-image color-space/native shading color-space/pattern color-space/ExtGState SMask group color-space/nested direct-array color-space/active marked-content Properties/ExtGState resources, source-scoped artifact pruning for mapped decoded/scaled image, SMask, mesh, Form, tiling, annotation-appearance, and dependent parent source IDs, cross-binding/server render-invalidation plan propagation with `source_cache_markers`, nested render write-set refs mapped through cache-remembered object identities, broad write-set metadata strings ignored unless they are parseable refs or structured object/generation pairs, and Rust cache-owner plan parsing/application now drive renderer invalidation for supported paths. Explicit dirty-tile plans with affected pages prune page-scoped retained artifacts while keeping raster invalidation to the exact dirty/source-tile set; coarse source-derived pages expand recorded page raster tiles when exact/source-tile coverage is absent or partial; nested shared-resource edits now expand through dependent sources before invalidating tiles and pruning parent resource programs; unknown explicit nested refs conservatively reset instead of publishing stale cache entries. Safe narrowing of conservative page-level resource dependencies, external cache-handle runtime matrix validation and broader shared-resource narrowing are not finished. |
| Cache dependency graph status | PARTIAL_ADVANCED: graph foundations record bounded page/content/resource dependencies, source-to-tile edges, source-to-source dependent edges, bounded retained resource-op tile edges for resource-backed paint/text/image/form/shading/pattern operations including named Image XObject color-space, referenced image Mask/SMask color-space, retained inline-image color-space, native shading color-space, pattern color-space, ExtGState SMask group color-space, active marked-content Properties/OC resources, and nested direct-array color-space dependencies, exact raster-tile invalidation, page-artifact-plus-exact-tile invalidation, source-derived partial exact-tile coverage expansion, source-scoped artifact pruning across changed and dependent parent sources, and page-scoped artifact pruning for full page invalidation; universal operation-to-source extraction and external host-cache mutation are not complete. |
| Persistent clip-DAG status | PARTIAL_ADVANCED: clip DAG nodes now expose Full/Empty/Rectangle/SparseSpans/RleMask/DenseMask/Composite representations with stable ID, operation, parent, bounds, identity-scope, and memory-charge metadata; active page, SMask `/G`, transparency/Form group RenderState DAGs, retained display-list/packed-plan replay, and display-list `CpuRenderDevice` replay now carry active clip nodes rather than save-time materialized-mask reclassification; rectangle/span/RLE intersections remain structural; DAG-native window materialization is used for transparency group clip carry and short-circuits empty/all-visible composite child windows; page/content/Form/annotation/pattern/shading/Type 3 temporary scopes save and restore the active `Arc<ClipNode>`, explicit render-contract identity refresh re-interns the current clip into the new scoped DAG, and page/content/Form/packed-plan save-restore rebuilds buffer clips from DAG state without populating node materialization caches; repeated transformed path clip installs hit a bounded render-contract-scoped DAG-node cache carried through `RenderDocumentCache`; active intern tables self-prune transitive unreferenced subgraphs behind a cap, including composite children orphaned during the same pruning call; path clip-node cache entries/bytes/counters and active ClipDag intern/pruning counters are exposed through `RenderContractTelemetryReport` and CLI `render-corpus`; compact group, direct alpha-mask, LCD subpixel glyph-mask, RGBA fragment, and solid fill clip/soft-mask fusion are active; `PixelBuffer` still consumes concrete masks at paint boundaries and universal concrete-mask elimination is incomplete. |
| Transparency status | PARTIAL_ADVANCED: common group and blend behavior is active; RGB/default group spaces keep the active RGB compositor path, opaque-normal DeviceGray/DeviceCMYK active Form groups render through a conservative inert subset, alpha/backdrop-observable DeviceGray/DeviceCMYK groups now fail typed instead of silently using RGB group-space compositing, and existing knockout group replacement now uses bounded row slices with clip/SMask/group-alpha fusion. Full non-RGB group-space conversion, backdrop interaction, and complete PDF knockout semantics remain incomplete. |
| Soft-mask status | PARTIAL_ADVANCED: common SMask paths and image SMask discovery are active; SMask group cache identity now includes source seed, document revision, schema-v1/legacy render-contract identity, tile-local viewport, device transform, active clip, render mode/quality, print profile, annotation/form policy, optional-content state, resource budget, `/TR` transfer-function identity, and resolved non-device group color-space fingerprints for calibrated/profiled/indexed/tint-transform luminosity backdrops. SMask `/G` group `/CS` now accepts direct/resource-resolved device spaces; `/S /Alpha` SMask `/G` groups with no `/BC` accept well-formed calibrated CalGray/CalRGB/Lab, structurally exact Indexed, and structurally valid ICCBased group spaces through a non-device-inert policy; `/S /Luminosity` SMask `/BC` resolves well-formed CalGray, CalRGB, Lab, Indexed, ICCBased, Separation, and DeviceN group spaces through the named-color/CMM resolver; malformed, missing-resource, malformed Indexed, malformed tint transforms, malformed ICCBased profile metadata, alpha SMask non-device groups with `/BC`, or richer non-device spaces fail typed before rendering. Compact `composite_from_at` windows, direct alpha-mask paints, LCD subpixel glyph masks, RGBA fragments, and solid fills now fuse partial clip opacity with soft-mask coverage into row kernels. Normal Compat soft-mask rows cover both opaque-destination SIMD/wide group-alpha paths and mixed-destination portable f32x4 general rows before scalar tails. Separable solid fills, including translucent source-alpha fills, now cover opaque and mixed-destination rows plus partial clips through f32x4 rows before scalar tails; cached RGBA fragments, including mixed source-alpha and mixed-destination rows, also fuse partial clips through f32x4 rows before scalar tails. Full non-normal/high-quality/general clip/mask fusion and alpha/backdrop-observable non-device group-space semantics beyond the non-device-inert/luminosity-backdrop subsets are still incomplete. |
| Print-profile status | PARTIAL_ADVANCED: screen halftone is active, CLI exposes display/print/proof contract selection, `Proof` contracts now fail closed unless `NativeLittleCms` is available, native-enabled proof renders have a bounded final OutputIntent proof-transform hook over the rendered RGB surface, and the active render-interpreter Separation/DeviceN plate framebuffer report is exposed through Rust SDK, CLI, server, C ABI/header, Python, WASM/TypeScript, .NET, and Java source surfaces. Normal RGB/gray render-contract output still refuses `PreserveSeparations`; the prepress `SeparationFramebuffer` surface now validates `PreserveSeparations` as the explicit N-channel source output path. Certification-grade proofing, native-CMM runtime validation, and direct raster-contract delivery of CMYK/DeviceN rows remain incomplete. |
| Adaptive scheduler status | PARTIAL_ADVANCED: deterministic adaptive tile sizing, visible/adjacent/background viewport priority bands, dirty-region selective tile requeueing, structured progressive fallback policy reporting, caller-visible job/tile publication identities with serialized `priority_class`, bounded source-level viewport and dirty-tile obsolete-publication reporting, source-session tile-publication acceptance/rejection, adjacent-page prefetch preview planning, bounded viewer queue preview JSON, source viewer queue execution JSON for owned current-page tiles, bounded adjacent-page child-session prefetch execution, source viewer callback dispatch JSON, synchronous C/Python/WASM/.NET/Java callback helpers, and progressive request cancellation are active; external viewer runtime matrices and broader queue policy remain incomplete. |
| Region image-decode implementation status | PARTIAL_ADVANCED: metadata culling is tile-origin aware for XObjects and inline images, offscreen inline images skip decode scheduler reservation, guarded CCITT/raw source windows cover axis-aligned XObjects/inline images plus same-sized unfiltered grayscale SMask partners for raw XObjects, and `HighQualityExact` refuses visible full-decode-only image paths when the active viewport requires source-region decode support; JPEG/JBIG2/filtered-lossless decoder APIs and the current JPX public API expose no native ROI/source-region pixel output, so those cases are reported unavailable rather than faked while remaining source-owned raw filtered, non-axis, and postprocessed windows stay partial. |
| Scaled image-decode status | PARTIAL_ADVANCED: target-size cache identity exists, guarded DCT/JPEG Image XObject and inline-image plans now use `jpeg-decoder` reduced-IDCT output for 1/2, 1/4, and 1/8 downscale tiers, guarded JPX Image XObject and inline-image plans now use `hayro-jpeg2000` target-resolution decode, scaled XObject scheduler reservations are based on requested target dimensions instead of full uncompressed source dimensions, and `HighQualityExact` refuses visible image paths when required reduction remains unavailable; JBIG2/lossless reduction and CCITT reduction remain incomplete. |
| Progressive image-decode status | PARTIAL_ADVANCED: bounded start/continue/pause/resume/cancel/fail/close/document_close lifecycle reporting is implemented and exposed through Rust SDK, C ABI/header, Python, WASM, .NET, Java, CLI, and server while reporting `full_decode_required` for current non-progressive codec adapters without retaining decoded pixels; terminal reports now carry a release reason for cancellation, render failure, session close, or document close; `continue_decode` preserves `Completed`, `Cancelled`, `Failed`, and `Closed` terminal states before consulting future native-progressive capability, terminal reports suppress `full_decode_required`, and the SDK JSON envelope covers continue-after-cancel/fail/close/document_close so cancellation, failure, session close, or document close cannot be resurrected into completion through binding-facing source; codec-native progressive pixel continuation is reported unavailable for current decoder APIs rather than faked. Evidence: `progressive_image_decode_continue_preserves_terminal_states`, `progressive_image_decode_document_close_releases_state`, `progressive_image_decode_pause_resume_cancel_close_release_state`, `progressive_image_decode_lifecycle_report_envelope`, and `capi_progressive_image_decode_lifecycle_report_json`. |
| WASM SIMD implementation status | PARTIAL_ADVANCED: actual wasm32+`simd128` kernels exist for opaque fills, normal opaque-destination blends, normal source-over rows, alpha/glyph-mask opaque- and mixed-destination rows, soft-mask rows including non-opaque group-alpha effective-alpha rows, separable opaque-destination rows with true lane math for Multiply, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn, HardLight, SoftLight, Difference, and Exclusion, copy/opaque RGBA contract rows, premultiply RGBA contract rows, premultiply BGRA rows using the same vector premultiply lanes plus a BGRA channel shuffle instead of per-pixel scalar extraction, unpremultiply RGBA library rows, RGB/BGR/BGRA contract channel rows, RGB8-to-opaque-RGBA image rows using bounded zero-extending loads plus SIMD expansion shuffles, and shared lane-luma Gray8 plus straight/opaque/premultiplied grayscale RGB/RGBA/BGRA expansion rows using SIMD shuffles before scalar tails; CPU portable `f32x4` also covers normal Compat mixed-destination source-over, soft-mask rows, separable solid-fill mixed-destination rows, and separable cached-RGBA mixed-destination rows with no clip, binary clips, all-visible clips, and partial clips before scalar tails; runtime capabilities name this bounded lane set; full requested SIMD operation coverage remains incomplete. |
| Rust progressive API status | ACTIVE: lifecycle, token, pause/resume/cancel/close, request_cancel, adaptive sizing, structured fallback policy details, publication identity, viewport obsolete-publication reports, dirty-region obsolete-publication reports, tile-publication acceptance reports, viewer queue reports, viewer queue execution reports, `execute_adjacent_page_prefetch`, viewer callback dispatch reports, and `dispatch_viewer_callbacks`. |
| C progressive API status | SOURCE_ACTIVE: exported in Rust and declared in C header, including request-cancel, viewport revision JSON, dirty-region revision JSON, tile-publication evaluation JSON, viewer queue JSON, viewer queue execution JSON, adjacent-page prefetch execution JSON, cancellable queue/prefetch execution entrypoints, viewer callback dispatch JSON, and callback-pointer dispatch. |
| Python progressive API source status | SOURCE_ACTIVE: PyO3 progressive job wrapper exists, including request_cancel, `dispatch_viewer_callbacks`, `step_with_cancellation`, `finish_png_with_cancellation`, `revise_viewport_hint_json`, `revise_dirty_region_json`, `revise_render_context_json`, `evaluate_tile_publication_json`, `viewer_queue_json`, `execute_viewer_queue_json`, `execute_viewer_queue_json_with_cancellation`, `execute_adjacent_page_prefetch`, `execute_adjacent_page_prefetch_with_cancellation`, and `viewer_callback_dispatch_json`. |
| WASM progressive API source status | SOURCE_ACTIVE: Rust methods and TypeScript declarations exist, including `requestCancel`, `dispatchViewerCallbacks`, `stepWithCancellation`, `finishPngWithCancellation`, `reviseViewportHintJson`, `reviseDirtyRegionJson`, `reviseRenderContextJson`, `evaluateTilePublicationJson`, `viewerQueueJson`, `executeViewerQueueJson`, `executeViewerQueueJsonWithCancellation`, `executeAdjacentPagePrefetch`, `executeAdjacentPagePrefetchWithCancellation`, `AdjacentPagePrefetchExecution`, and `viewerCallbackDispatchJson`. |
| .NET progressive API source status | SOURCE_ACTIVE: wrapper and adaptive helper exist, including `RequestCancel`, `DispatchViewerCallbacks`, `ReviseViewportHintJson`, `ClearViewportHintJson`, `ReviseDirtyRegionJson`, `ReviseRenderContextJson`, `EvaluateTilePublicationJson`, `ViewerQueueJson`, `ExecuteViewerQueueJson`, `ExecuteAdjacentPagePrefetch`, `ViewerCallbackDispatchJson`, `CancellationToken` step/finish/queue/prefetch overloads, and `RenderCancellation` queue/prefetch overloads. |
| Java progressive API source status | SOURCE_ACTIVE: wrapper and adaptive helper exist, including `requestCancel`, `dispatchViewerCallbacks`, `reviseViewportHintJson`, `clearViewportHintJson`, `reviseDirtyRegionJson`, `reviseRenderContextJson`, `evaluateTilePublicationJson`, `viewerQueueJson`, `executeViewerQueueJson`, `executeAdjacentPagePrefetch`, `viewerCallbackDispatchJson`, `BooleanSupplier` step/finish cancellation overloads, and `RenderCancellation` queue/prefetch overloads. |
| Server progressive API status | ACTIVE: bounded HTTP session API with cancellation-first session store path, viewport revision route, dirty-region route, render-context revision route, tile-publication evaluation route, viewer queue route, viewer queue execution route, adjacent-page prefetch execution route, viewer callbacks route, obsolete-publication reports, adjacent-page prefetch preview/execution reports, and tests. |
| Caller-owned surface status by binding | Rust/C/Python/WASM/.NET/Java source paths exist where contract JSON is exposed; reverse byte order and RGBA/BGRA alpha-mode row encoding are implemented in core; runtime capabilities expose caller-owned buffer support and the active alpha-mode boundary. External binding runtime gates remain incomplete. |
| Cancellation parity status | PARTIAL_ADVANCED: linked engine `CancelToken` values and progressive request-cancel plus callback-dispatch source surfaces exist for Rust/server/C/Python/WASM/.NET/Java; C/Python/WASM/.NET/Java expose synchronous callback helpers that dispatch zero events after terminal states; Python exposes callable/boolean step/finish cancellation predicates plus `RenderCancellation` queue/prefetch execution methods, WASM exposes boolean/AbortSignal-style step/finish methods plus `RenderCancellation` queue/prefetch execution methods, .NET exposes `CancellationToken` step/finish/queue/prefetch overloads and `RenderCancellation` queue/prefetch overloads, and Java exposes `BooleanSupplier` step/finish cancellation overloads plus `RenderCancellation` queue/prefetch overloads; external runtime binding gates and complete viewer cancellation policy remain incomplete. |
| Contract-builder parity status | PARTIAL_ADVANCED: CLI and server can build schema-v1 geometry, surface, page-box, annotation/form, optional-content, smoothing, prepress/color, exactness, determinism, and resource-budget fields and can replay exact full-field contract JSON where applicable; server exposes `/api/v1/render-contract`, `/api/v1/render-contract/png`, `/api/v1/render-contract/raw`, `/api/v1/render-contract/png-with-font-substitution-report`, and `/api/v1/render-contract/raw-with-font-substitution-report` routes; C ABI, .NET, Java, Python, and WASM now have typed schema-v1 contract objects/handles with typed/object render overloads; .NET and Java typed builders expose full-field setter coverage for page box, execution/backend/compositing policy, annotations/forms, optional content, smoothing, prepress/color, exactness, determinism, and resource budgets, with Java validating parsed enum names instead of accepting arbitrary non-empty strings; C ABI/header exposes a nullable-string full schema policy builder, while Python and WASM/TypeScript expose method-level builders for the same page-box, policy, smoothing, color/print/prepress, exactness, and determinism fields through canonical Rust validation. Core validation enforces public max-pixel, page-content/downstream max-decoded-byte, and max-temporary-byte budgets, including clipped full-page intermediates, and page-content decode observes caller cancellation. Full non-default execution parity and external runtime gates remain incomplete. |
| Font-substitution reporting status | COMPLETE_ACTIVE_SOURCE: deterministic fallback events are recorded, serializable, returned from Rust render APIs, exposed through CLI sidecars, available through C/Python/WASM/.NET/Java source render APIs, and returned by server contract PNG/raw report routes as `multipart/mixed` metadata. Events include requested PDF font, embedded state, encoding, high-level substitution reason, provider selection reason, metric posture, Standard14/document-provided/caller-registered/deterministic-system/bundled resolution source, selected replacement, bounded required glyph coverage, missing glyphs, visual-risk category, extraction/editing impact, font-policy identity, and risk flags; source single-flight coalesces equivalent renderer-originated report events while preserving first observed page/coverage; `ContentEngine::register_font_bytes` and `with_registered_font_bytes` provide a caller-registered provider tier before bundled fallback, normalize subset names, report `user_registered`, and salt font-policy/cache identity with byte-sensitive provider fingerprints; C ABI/header, Python, WASM/TypeScript, .NET, Java, CLI render, server render-contract routes, and server progressive start expose registered-font input paths; FontDescriptor Symbolic/Nonsymbolic, Italic, ForceBold, and FontWeight hints feed deterministic provider selection, so unknown non-Standard14 symbolic descriptor fonts route to the symbolic coverage face and report `CoverageOnly` with selection reason `symbolic_flag` in compatibility mode; high-quality render mode and explicit `HighQualityExact` contract rendering accept valid caller-registered and deterministic-system replacements but refuse generic bundled replacement when no valid embedded/document/system/registered replacement exists; font byte/resolver cache keys and glyph hashes include render-contract identity, resource budget, and registered-provider identity; device glyph masks have a bounded atlas-backed reuse path and telemetry. External registered-font runtime matrices are future validation. |
| Type 3 status | PARTIAL_ADVANCED: Type 3 glyphs render only from PDF Encoding CharProc names; CharProcs with clips, typed text/image/Form/shading/inline/pattern resources, Type 3 `d0`/`d1` metrics descriptors, and vector paths with per-paint inherited/explicit fill-stroke color handling can replay through guarded retained plans; focused synthetic coverage now proves resource-backed image, Form XObject, and named shading CharProcs through the retained path; parsed CharProc/geometry caches now track explicit `Unvisited`/`Compiling`/`Compiled`/`Failed` lifecycle states, preserve failure reasons, refresh recency on hits, and absorb child render-state results through bounded LRU admission; Type 3 mask/rendered-glyph cache pressure now evicts least-recently-used entries without clearing hot entries; rendered-glyph cache bounds require exact four-number finite font-level `/FontBBox` arrays before bounded cached-surface admission; Unicode-derived CharProc aliases, ordinary font fallback bytes, recursive CharProc nesting overflow, and present malformed Type 3 `/FontMatrix` values are refused with typed `UnsupportedFeature`; broader unsupported CharProc state/resource matrices remain incomplete. |
| JPX status | COMPLETE_ACTIVE_SOURCE: JPX decode exists, `images/jpx.rs::inspect_metadata` exposes dimensions, bit depth, color-space family, channel counts, and alpha state without decoding pixels, guarded downscale plans use `hayro-jpeg2000` target-resolution decode when no image mask or full-image postprocessing step requires the original sample grid, active decode-required paths consume explicit capability reports, `HighQualityExact` refuses visible JPX paths when required source-region support or unsafe reduction support is unavailable, runtime capabilities expose native metadata inspection and guarded reduction plus unavailable ROI/region/tile/component/progressive limitations, per-image capability JSON is source-visible through Rust SDK/C/Python/WASM/.NET/Java, and the progressive-image lifecycle report is source-visible through Rust SDK, C ABI/header, Python, WASM, .NET, Java, CLI, and server while reporting full-decode-required rather than native continuation for JPX. The installed `hayro-jpeg2000` public API exposes target-resolution decoding but not ROI/source-region, component-subset, tile-pixel, progressive-continuation, or cancellation hooks, so those modes remain explicit unavailable capabilities instead of missing Wellfriend source code. |
| SIMD compositor status | PARTIAL_ADVANCED: CPU SIMD subsets and wasm32+`simd128` row kernels have scalar-equivalence guards, soft-mask opaque-destination rows now stay on wide/SIMD paths for non-opaque group alpha, soft-mask mixed-destination rows now stay on a portable `f32x4` general row path before scalar tails, mixed-source source-over rows now stay on the wide uniform-alpha path for partial group alpha over opaque destinations, normal Compat mixed-destination source-over rows now stay on a portable `f32x4` row path before scalar tails, normal Compat LCD glyph masks now use a row-local clip/SMask fusion path before the per-pixel oracle, separable solid fills and partial-clip cached RGBA rows over mixed destination alpha now stay on portable `f32x4` rows before scalar tails, native x86/x86_64 unpremultiply rows now use guarded SSE2 load/store groups with exact scalar reciprocal conversion, native opaque-background page flattening now has a guarded SSE2 row helper, and runtime capabilities disclose the bounded implemented lane set; full operation coverage remains incomplete. |
| Scan converter status | PARTIAL_ADVANCED: derivative-root monotonic cubic decomposition, finite transformed-point filtering before scanline/stroke emission, active edge buckets, precomputed edge slope/direction descriptors, row-local scratch reuse, a guarded sequential active-edge table for fill, binary-clip, alpha-mask, and subsampled compositor rows, stroke/path logic, a deterministic `prepare_scanline_crossings` active-edge invariant boundary, guarded direct span rendering for exact integer device-space horizontal/vertical solid butt/projecting-square single-segment and monotonic-collinear polyline strokes, sampled-coverage direct spans for simple solid butt single-segment and monotonic-collinear polyline hairlines, guarded opaque rectangular miter stroke spans, and opposite-winding closed-stroke contours exist; the scan-converter invariant suite now directly covers crossing filtering/sorting, active-edge sequential-row/bucket/subsample parity, ended-edge lifecycle, scratch-pool reuse contracts, convex/concave fast-path guards, and stroke fast-path equivalence. Broader geometry/AET closure outside these guarded scanline paths, reusable Form/Type 3 geometry fast paths, and broader stroke fast-path closure remain incomplete. |
| Image-cache status | PARTIAL_ADVANCED: raw decoded-image cache lookup/admission now uses planner keys salted with revision, render-contract identity, active rendering intent, tile viewport, device transform, target size, quality, stable codec/source-region/reduction/decode-array/image-mask/soft-mask/interpolation fragments, policy, optional-content state, and resource budget; inline images share metadata-first culling before decode but are not cached as decoded document resources; planner capability reports explicitly mark metadata inspection, mark JBIG2/filtered-lossless paths full-decode-only, mark guarded DCT/JPEG downscale tiers as native reduced-IDCT when safe, mark guarded JPX downscale tiers as native target-resolution reduction when safe, and keep JPEG region/tile/component/progressive unavailable plus JPX ROI/region/tile/component/progressive unavailable at the current decoder API boundary, while guarded axis-aligned CCITT source-clipped XObjects can decode/cache bounded source windows, CCITT inline images can decode bounded source windows, guarded raw unfiltered 1/2/4/8/16-bit Image XObjects can decode/cache bounded source windows including same-sized unfiltered grayscale SMask partners, and guarded raw unfiltered 1/2/4/8/16-bit inline images can decode bounded source windows; active decode paths consume those reports, runtime capabilities disclose JPX metadata inspection, guarded DCT/JPX reduction plus CCITT and raw source-window exceptions, visible plans record source-region/reduction requirements for exact-policy refusal, per-image capability JSON is exposed through Rust SDK/C/Python/WASM/.NET/Java source APIs, and progressive-image lifecycle JSON uses the same stable source/cache identity while retaining no pixels for unavailable native progressive decoders through Rust SDK, C ABI/header, Python, WASM, .NET, Java, CLI, and server; broader JPEG/JBIG2/filtered-lossless decoder-native ROI/region/tile/component/progressive decode and remaining reduction decode remain incomplete, while JPX unsupported modes are explicit unavailable capabilities rather than silent cache fallbacks. |
| Render-document-cache budget status | PARTIAL_ADVANCED: byte-charged raw image, scaled image, SMask group, mesh-shading, Form XObject program, tiling-pattern program, annotation appearance program, transformed path clip-node, font-byte, font-resolver, glyph outline, device glyph-mask, glyph-mask atlas page, Type 3 parsed geometry program, Type 3 parsed CharProc program, Type 3 mask, Type 3 rendered-glyph, path fill-mask, path stroke-mask, and retained display-list caches enforce deterministic caps where applicable and aggregate `RenderResourceBudget.max_cache_bytes` cleanup on return from render state where applicable; active ClipDag intern tables self-prune transitive unreferenced subgraphs and report final per-render stats; transparent-page-group decisions enforce deterministic contract- and revision-scoped entry-capped LRU admission; runtime capability JSON and server `/api/v1/capabilities` expose a structured renderer cache-pressure policy with cache classes, budget fields, pressure actions, correctness-preservation state, and a cache-accounting reason that names transformed path clip-node, glyph-outline, glyph-atlas, Type 3 parsed-program, Type 3 mask/rendered-glyph, and path-mask coverage; Rust `RenderContractTelemetryReport`, server multipart contract-report routes, and C/Python/WASM/.NET/Java source render-report APIs expose one-shot per-render cache counters including path clip-node, glyph-atlas, and active ClipDag stats; CLI `render-corpus` JSON exposes retained display-list, annotation appearance program, path clip-node, glyph-atlas, and active ClipDag cache stats; external runtime telemetry parity remains incomplete. |
| Glyph-cache status | PARTIAL_ADVANCED: bounded glyph outline caches exist, device glyph-mask and related path-mask cache pressure evicts least-recently-used masks one entry at a time instead of clearing hot masks, glyph hashes inherit the contract/resource-budget-scoped font cache key, reused device glyph masks can be packed into bounded atlas pages that paint via strided alpha rows and release empty pages on eviction, renderer-originated substitution report events now coalesce equivalent selections while preserving first observed page/coverage, and failed bounded color-glyph sub-rendering declines color fill so the ordinary outline fallback can paint instead of silently omitting the glyph; external runtime gates remain incomplete. |
| Colour-cache status | PARTIAL_ADVANCED: CMM/profile cache paths exist, portable qcms remains the default supported backend, active PDF/contract rendering intent is now threaded into ICCBased paint, image XObject decode, Separation/DeviceN alternate ICC resolution, and SVG/PS named-color resolution, explicit `ColorManagementPolicy` now selects the active ICC backend policy for contract paint/image paths and raw image cache identity, `DeterministicFallback` avoids ICC backend use, explicit `NativeLittleCms` render contracts now fail typed when the active build/target has no native CMM backend, and `Proof` routes through the native OutputIntent proof transform when available. Complete native CMM runtime validation plus print/proof separation remain incomplete. |
| Synthesized/Type3 DeviceCMYK status | COMPLETE_ACTIVE_SOURCE: synthesized annotation `/C`, widget `/MK /BG`/`/MK /BC`, widget `/DA k`, and Type 3 device-color pixels route through `render/cmm.rs::device_cmyk_to_srgb` after finite unit normalization instead of retaining a local algebraic CMYK preview conversion. Evidence: `synthesized_annotation_cmyk_uses_shared_device_cmyk_fallback`. |
| Form XObject retained status | PARTIAL_ADVANCED: reusable programs exist, are keyed by active page/render-contract identity plus active parent-resource fingerprint, and supported Form streams, including transparency-group interiors, carry retained plans that replay through the packed adapter. Retained display-list compilation now rejects missing XObject resources, XObject streams without `/Subtype`, and unsupported XObject subtypes before native Form descriptor synthesis; packed-plan compilation also turns stale/manual Image/Form descriptors with resolved subtype mismatches into explicit compile refusals. Missing/malformed Form fetch/decode/parse inputs, missing or malformed Form/SMask `/BBox` arrays, malformed present Form/annotation/SMask `/Matrix` arrays, direct/indirect recursion or depth-limit hits, malformed transparency-group `/I`/`/K` flags, and transparency-group offscreen allocation denials fail typed. Unsupported Form display lists, full alpha/backdrop-observable group-space/backdrop/knockout closure, and full operation-level invalidation remain incomplete. |
| Annotation/widget appearance status | PARTIAL_ADVANCED: render paths exist, appearance Form streams decode/parse into a bounded page/render-contract-scoped program cache, supported appearance streams cache retained plans and replay through the packed adapter, cached display-list annotation replay uses the caller-owned document cache, retained annotation replay honors active print/annotation/form contract policies, page-source invalidation prunes appearance programs with other page-scoped artifacts, mapped annotation object/source invalidation prunes source-scoped appearance program cache entries, and Rust/server/C/Python/WASM/.NET/Java render-report routes plus CLI `render-corpus` expose annotation appearance program cache stats in source. Visible annotations with malformed `/F` visibility flags, missing or malformed `/Rect`, malformed selected `/AS` appearance state metadata, malformed synthesized text-markup `/QuadPoints`, line `/L`, ink `/InkList` geometry, synthesized annotation `/C`/`/CA` paint metadata, synthesized FreeText `/Q` alignment metadata, synthesized widget `/AS` state metadata, synthesized widget `/Ff`/`/Q` integer metadata, synthesized widget `/MK /BG`/`/MK /BC` color metadata, or synthesized widget `/DA` font/color operands, selected malformed appearance streams with missing or malformed `/BBox`, selected missing appearance states, selected state objects that do not resolve to streams, and selected appearance decode failures now fail typed and annotation-stage fatal errors propagate through public/cached/tile replay paths. The lower-level synthesized markup/line/ink/FreeText helpers also reject malformed paint, geometry, and alignment metadata locally if reached without the normal preflight, and the lower-level synthesized Widget path rejects malformed `/DA` and `/MK` metadata locally before defaulted or clamped appearance construction. Advanced-editing Link annotation moves and Ink appearance regeneration now emit structured `CacheInvalidationReport` render write-set refs, changed/created object refs, affected pages, and page-space dirty regions for cache-owning callers. DocumentSubsystems AcroForm actions and secure-mutation incremental form fills now expose structured field/widget/AP write-set refs, affected pages, and page-space dirty regions where live widget appearances change; cache-plan envelope application normalizes AP-state, normal/rollover/down appearance stream, and before/after appearance refs into source-scoped invalidation with unknown-ref conservative reset. Dirty-region conversion accepts annotation/widget/appearance and before/after rect/bounds aliases for exact-tile conversion. Source cache-handle adoption is active across C/Python/WASM/.NET/Java; external runtime matrix validation remains deferred. |
| SVG regional-fallback status | PARTIAL_ADVANCED: finite affine image XObject regions including stencil masks, complete finite-affine DeviceGray/DeviceRGB/DeviceCMYK inline images except CCITT/JBIG2 terminal RGB/CMYK declarations, resource-named device color spaces, direct/resource-resolved CalGray/CalRGB/Lab inline images, direct/resource-resolved Indexed inline images, direct/resource-resolved ICCBased inline images, preflighted opaque direct/resource-resolved Separation/DeviceN inline images, DCT-filtered opaque resource-resolved Separation and one-colorant DeviceN inline images, DCT-filtered calibrated/Indexed/ICCBased inline images, simple inline image masks, vector-safe Form XObjects including no-op, opaque-inert isolated/knockout transparency-group Forms, opaque normal DeviceGray/DeviceRGB/DeviceCMYK group-colour transparency Forms, and opaque normal well-formed CalGray/CalRGB/Lab/Indexed/ICCBased/Separation/DeviceN group-colour transparency Forms, composed stacked SVG path/Form/text clips, dense ordinary text, simple linear DeviceGray/DeviceRGB/DeviceCMYK plus direct/resource-resolved CalGray/CalRGB/Lab/Separation axial shadings with endpoint-sampled contained non-unit shading `/Domain` support, function-domain-clipped stop-list support, and transformed shading `/BBox` clipping, finite/invertible affine-transformed DeviceGray/DeviceRGB/DeviceCMYK plus direct/resource-resolved CalGray/CalRGB/Lab/Separation radial shadings through the same simple Type 2 endpoint-sampling, continuous Type 3 stitching stop-list, function-domain clipping, and BBox path, simple finite-matrix shading-pattern fill/stroke path paints and glyph-outline text paints with shading BBox clip intersection, simple colored and color-inheriting uncolored tiling-pattern path paints, vector-safe resource-bearing image/Form/shading/inline-image tile cells, normal-blend scalar `CA`/`ca` ExtGState opacity, safe line cap/join/miter/dash/font/flatness/smoothness/default-state/identity-transfer/first-supported-normal-blend-array ExtGState, styled stroked/fill-stroke text outlines, font-resolved text-clipping glyph clips, and overwritten dead pattern color state are preserved as native/vector-bounded output; whole-page raster embedding remains for unsupported constructs, including malformed or degenerate shading BBoxes, advanced pattern paint and resource-bearing tiling cells outside the vector-safe image/Form/shading/inline-image subset, unsupported advanced pattern-space transforms, active patterned text outside the supported shading/tiling glyph-outline subset, alpha/backdrop-observable semantic transparency groups, alpha/backdrop-observable or richer non-device group-colour transparency Forms, soft masks, and unresolved/preflight-unavailable text clipping. |
| PS regional-fallback status | PARTIAL_ADVANCED: finite affine image XObject regions including stencil masks, complete finite-affine DeviceGray/DeviceRGB/DeviceCMYK inline images except CCITT/JBIG2 terminal RGB/CMYK declarations, resource-named device color spaces, direct/resource-resolved CalGray/CalRGB/Lab inline images, direct/resource-resolved Indexed inline images, direct/resource-resolved ICCBased inline images, preflighted opaque direct/resource-resolved Separation/DeviceN inline images, DCT-filtered opaque resource-resolved Separation and one-colorant DeviceN inline images, DCT-filtered calibrated/Indexed/ICCBased inline images, simple inline image masks, PostScript `imagemask` stencil emission, vector-safe Form XObjects including no-op, opaque-inert isolated/knockout transparency-group Forms, opaque normal DeviceGray/DeviceRGB/DeviceCMYK group-colour transparency Forms, and opaque normal well-formed CalGray/CalRGB/Lab/Indexed/ICCBased/Separation/DeviceN group-colour transparency Forms, dense ordinary text, simple linear DeviceGray/DeviceRGB/DeviceCMYK plus direct/resource-resolved CalGray/CalRGB/Lab/Separation axial shadings with endpoint-sampled contained non-unit shading `/Domain` support, function-domain-clipped stop-list support, and transformed shading `/BBox` clipping, finite/invertible affine-transformed DeviceGray/DeviceRGB/DeviceCMYK plus direct/resource-resolved CalGray/CalRGB/Lab/Separation radial shadings through the same simple Type 2 endpoint-sampling, continuous Type 3 stitching stop-list, function-domain clipping, and BBox path, simple finite-matrix shading-pattern fill/stroke path paints and glyph-outline text paints with shading BBox clip intersection, simple colored and color-inheriting uncolored tiling-pattern path paints, vector-safe resource-bearing image/Form/shading/inline-image tile cells, safe opaque line cap/join/miter/dash/font/flatness/smoothness/default-state/identity-transfer/first-supported-normal-blend-array ExtGState, exact zero-alpha ExtGState no-op paint, fully transparent regional RGBA no-op regions, binary 0/255 regional RGBA alpha clips, styled stroked/fill-stroke text outlines, font-resolved text-clipping glyph clips, and overwritten dead pattern color state are preserved as native/vector-bounded output; whole-page raster embedding remains for unsupported constructs, including malformed or degenerate shading BBoxes, advanced pattern paint and resource-bearing tiling cells outside the vector-safe image/Form/shading/inline-image subset, unsupported advanced pattern-space transforms, active patterned text outside the supported shading/tiling glyph-outline subset, alpha/backdrop-observable semantic transparency groups, alpha/backdrop-observable or richer non-device group-colour transparency Forms, visible fractional-alpha paint, fractional regional RGBA, visible non-normal-blend paint, and unresolved/preflight-unavailable text clipping. |
| Visual-normalization harness status | COMPLETE_ACTIVE_SOURCE: compact tooling now covers canonical comparison surface, raw stride/channel/byte-order/straight-premultiplied-opaque alpha/rotation normalization, render-context sidecars including deterministic structured font-environment identity/fingerprints, expected-difference masks, metrics including RGB-only `color_mae`/`color_rmse`/`max_color_delta`, connected regions, severity, and cause taxonomy. Future corpus/reference execution is deferred. |
| Direct PDFium harness status | COMPLETE_ACTIVE_SOURCE: source and smoke script expose the required one-PDF, selected/all-pages, matrix, clip, page-box, DPI/output-dimension, annotation/form, raw bitmap, pixel-format, output-hash, worker-count, JSONL, typed-failure, and version-manifest controls. Supported media-box form widgets render through PDFium form-fill when the harness is built, unsupported form/matrix/clip/crop combinations fail typed, serial worker execution is disclosed, and local SDK build/runtime execution remains deferred to the future verification machine. |

## Fallback status

**Latest native SSSE3 grayscale completion:** native x86/x86_64 builds now route Gray8, grayscale RGB/RGBA/BGRA expansion, and premultiplied grayscale alpha-bearing contract rows through guarded SSSE3 luma kernels when available, with scalar debug oracles and scalar tails.

**Latest native SIMD backend reporting completion:** render-simd and engine compositor backend reporting now distinguish `ssse3` from `sse2`, so native SSSE3 channel/grayscale row capability is visible through detected-backend and runtime capability surfaces instead of being collapsed into the older SSE2 bucket.

**Latest native unpremultiply SIMD completion:** native x86/x86_64 builds now route `unpremultiply_rgba` through a guarded SSE2-dispatched four-pixel load/store row path when available. The exact reciprocal conversion remains scalar per channel to preserve the existing byte contract, with scalar debug oracles and scalar tails.

**Fallback categories found at start:** retained unsupported-list fallback, retained tile fallback, progressive tile fallback, unresolved Type 3 Compat fallback, bundled font substitution, JPX compatibility/full-decode paths, qcms portable color backend, SVG whole-page raster fallback, PS/EPS whole-page raster fallback, non-active halftone policy, caller-surface byte-order refusal, non-zero tile-origin decode fail-open.

**Newly completed gaps:** caller-surface byte-order refusal, caller-owned RGBA/BGRA alpha-mode row encoding, RGB screen-halftone execution, server progressive sessions, adaptive progressive tile sizing, progressive publication identity reporting, bounded viewport obsolete-publication source policy, source-session tile-publication acceptance/rejection, adjacent-page prefetch preview planning, bounded source viewer queue preview JSON, source viewer queue execution JSON for owned current-page tiles, bounded adjacent-page child-session prefetch execution through Rust/server/C/Python/WASM/.NET/Java source surfaces, source viewer callback dispatch JSON and terminal callback suppression policy, C/Python/WASM/.NET/Java synchronous callback execution helpers, progressive request-cancel source parity, Python progressive cancellation predicate step/finish methods, WASM boolean/AbortSignal-style progressive step/finish cancellation methods, .NET `CancellationToken` progressive step/finish overloads, Java `BooleanSupplier` progressive step/finish overloads, tile-origin metadata culling, inline-image metadata-first decode skip, per-image decode capability JSON source parity, guarded DCT/JPEG native reduced-IDCT downscale planning and decode for safe Image XObject/inline-image paths, bounded progressive-image decode lifecycle with typed full-decode-required reporting, cross-binding/server/CLI progressive-image lifecycle JSON source parity, HighQualityExact unsupported retained display-list refusal, HighQualityExact visible image source-region/reduction full-decode-only refusal, primary-walk SMask classification, SMask render-contract cache identity, SMask group color-policy/resource-scope cache identity, compact group clip/soft-mask row fusion, direct alpha-mask clip/SMask row fusion, RGBA fragment clip/SMask row fusion, solid fill clip/SMask row fusion, separable opaque solid-fill partial-clip fusion, separable opaque cached-RGBA partial-clip fusion, separable solid-fill mixed-destination f32x4 rows, separable cached-RGBA mixed-destination partial-clip f32x4 rows, normal Compat mixed-destination source-over f32x4 rows, normal Compat soft-mask mixed-destination f32x4 rows, active non-RGB transparency group-space typed refusal for alpha/backdrop-observable DeviceGray/DeviceCMYK Form groups outside the opaque-normal inert subset, native x86/x86_64 SSE2 alpha/glyph-mask opaque-destination row compositing, native x86/x86_64 SSE2 opaque-background page flattening, native x86/x86_64 SSE2 RGBA copy/opaque caller-surface rows, native x86/x86_64 SSE2 BGRA caller-surface channel rows, native x86/x86_64 SSSE3 RGB/BGR contract channel and RGB image-span rows, native x86/x86_64 SSE2 premultiplied RGBA/BGRA caller-surface rows, ScalarReference caller-surface scalar row preservation, wasm SIMD Gray8 contract channel conversion, wasm SIMD grayscale RGB/RGBA/BGRA expansion rows including premultiplied alpha-bearing rows, wasm SIMD alpha/glyph-mask opaque- and mixed-destination row compositing, wasm SIMD common separable blend rows, wasm SIMD copy/opaque RGBA contract rows, wasm SIMD premultiply RGBA/BGRA contract rows, wasm SIMD unpremultiply RGBA library rows, wasm SIMD RGB/BGR/BGRA contract channel rows, wasm SIMD RGB image-span rows, EditingTransactions geometric/semantic TextReflow mode routing, EditingTransactions compact TextReflow option forwarding, advanced-editing Link/Ink annotation structured render write-set reporting, transaction affected-page unioning, source-editing object/stream invalidation mapping, dirty-region report rectangle to render-tile conversion, explicit dirty-tile transaction write sets, source-to-tile dependency graph edges, page-artifact-plus-exact-tile invalidation for dirty-tile plans with affected pages, cross-binding/server transaction render-invalidation plan propagation, preserved indirect resource reference maps, bounded retained resource-op tile edge recording including font-backed text, color-space-backed paint/text, and ExtGState-backed paint/text/inline-image resources, exact raster-tile invalidation without page artifact pruning, bounded page resource dependency recording, page-scoped artifact pruning, Form/shading/pattern/annotation-appearance program page-contract cache identity, Form program parent-resource cache identity, Form/tiling/annotation-appearance program byte-charged cache accounting, transformed path clip-node aggregate byte accounting and telemetry, Type 3 parsed CharProc/geometry explicit lifecycle states plus LRU admission, transparent-page-group LRU admission and invalidation pruning, font/display-list byte-charged cache accounting, public max-decoded-byte render-contract enforcement for page content stream decode, render-contract page-content decode cancellation precheck, public max-temporary-byte render-contract enforcement for canonical and clipped temporary surfaces, runtime cache-accounting capability disclosure, runtime structured renderer cache-pressure policy disclosure, runtime structured fallback-policy matrix disclosure, CLI retained display-list and path clip-node cache-stat telemetry, Rust/server/C/Python/WASM/.NET/Java one-shot render telemetry source exposure, glyph/Type 3/path mask LRU pressure eviction, bounded glyph-mask atlas pages with strided alpha-row painting and telemetry, persistent clip representation variants, bounded transformed path clip-node caching, scanline active-edge preparation invariant, guarded axis-aligned solid butt/projecting-square stroke span fast path, structured progressive fallback policy reporting, C header/WASM declaration parity, direct PDFium harness source controls plus PDFium form-fill rendering/refusal, visual-normalization harness architecture including explicit alpha semantics, SVG/PS vector-safe Form regionalization, SVG/PS simple inline-image and inline-mask regionalization, SVG/PS affine local image regionalization, SVG/PS simple axial and finite/invertible affine-transformed radial shading native gradient/shfill regionalization, SVG/PS safe opaque ExtGState line-style/dash/font/flatness/smoothness/default-state/identity-transfer replay, CLI raw contract-surface controls, .NET typed render-contract builder source parity, Java typed render-contract builder source parity, Python typed render-contract builder source parity, render-time font-substitution report source parity, retained `StateOp` typed `GraphicsStateDescriptor` payloads, retained `NativePatternPathOp` typed `PatternPathDescriptor` payloads, packed named color-space pre-resolution for retained state replay, packed optional-content vector visibility replay including inline `/OC` dictionary evaluation, supported Form XObject retained plan replay including transparency-group interiors, supported annotation appearance retained plan replay, default/contract/tile/progressive render routing through retained packed plans for supported lists, resource-scoped retained tiling-pattern stream subplans for supported pattern streams, guarded retained Type 3 CharProc subplans for clips, typed resources, resource-backed image/Form/shading paints, vector paths with per-paint inherited/explicit colors, `d0`/`d1` metrics descriptors, Type 3 Unicode-derived CharProc alias removal, named-color tint-transform shape refusal, and multi-input DeviceN Function Type 2/3 tint-shape refusal.

**Latest source-scoped invalidation increment:** `RenderDocumentCache` now remembers canonical object-number cache markers for mapped source IDs, SDK/cache-plan JSON now emits those markers as `source_cache_markers`, `RenderInvalidationCachePlan` derives the standard marker list from object number/generation when marker strings are omitted, and mapped source invalidation prunes decoded/scaled image, SMask, mesh-shading, Form program, tiling-pattern program, and annotation appearance program cache entries even when the invalidation remains tile-scoped rather than page-scoped.

**Latest nested write-set invalidation increment:** `RenderDocumentCache` now remembers object number/generation to source identity mappings, and cache-plan JSON application scans full SDK/server/report envelopes for nested render write-set fields before mutating cache state. PDF refs, structured `[object, generation]` arrays, `{object_number, generation}` objects, `{ref}`/`{object_ref}`/`{source_ref}` records, and `object-*`/`stream-*` refs are merged into mapped source IDs and source-cache marker entries when known; unmapped nested refs force a conservative reset so source-bound artifacts and raster tiles are not left stale.

**Latest annotation/widget AP dirty-route increment:** `render/transaction_invalidation.rs` now accepts AP-state aliases from nested SDK/server/cache-plan envelopes, including `changed*APRefs`, annotation/widget/form/signature AP refs, AP stream refs, normal/rollover/down appearance stream refs, appearance-state refs, and before/after appearance refs with structured object/generation keys. Dirty-region conversion also accepts annotation/widget/appearance and before/after rect/bounds aliases for exact-tile conversion when cache owners supply a viewport/grid. Known refs merge into source cache marker invalidation; unknown AP refs keep the existing conservative reset behavior. Evidence: `nested_write_set_collector_accepts_annotation_widget_ap_state_aliases`, `render_invalidation_plan_json_maps_annotation_widget_ap_alias_refs`, `render_invalidation_plan_json_unknown_ap_alias_ref_resets_cache`, `dirty_regions_accept_annotation_widget_and_appearance_bounds_aliases`, and `cargo fmt --all --check` (0).

**Latest exact-tile invalidation increment:** `RenderDependencyGraph`, `RenderDocumentCache`, `TransactionWriteSet`, and `RenderInvalidationCachePlan` now have a page-artifact-plus-exact-tile path. Dirty-tile plans with affected pages or mapped source-page dependencies prune retained/page-scoped artifacts but no longer expand raster tile invalidation to every recorded tile on the page. Evidence: `cargo test -p wellfriendpdf-engine exact_tile --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine explicit_tiles_with_reported_page_do_not_expand_all_page_tiles --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo fmt --all --check` (0), `cargo check --workspace --all-targets --jobs 1` (0), and `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` (0).

**Latest separable source-alpha row increment:** `render/buffer.rs` now routes guarded separable solid fills and cached RGBA rows through CPU `wide::f32x4` channel and coverage lanes for all PDF separable blend modes. Solid fills use source alpha for no-clip/binary-clip rows, source-alpha times clip-alpha for partial clips, and destination-alpha-aware source-over math for mixed-destination rows; cached RGBA rows use per-pixel source alpha for no-clip, all-visible, binary-clip, and partial-clip rows, and use destination-alpha-aware source-over math for both opaque and mixed-destination rows, with scalar tails and scalar fallback for declined conditions. `runtime.rs` discloses the CPU f32x4 translucent-solid/mixed-alpha partial-clip separable row capability plus `separable_solid_mixed_destination_f32x4_rows`, `separable_rgba_mixed_destination_f32x4_rows`, and `separable_rgba_mixed_destination_partial_clip_f32x4_rows`. Evidence: `cargo test -p wellfriendpdf-engine separable_rgba_pixels_at --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine separable_ --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo fmt --all --check` (0), `cargo check -p wellfriendpdf-engine --lib --jobs 1` (0), and `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` (0).

**Latest compact group compositor oracle increment:** `render/buffer.rs::composite_from_at` now routes non-normal and high-quality compact group fallback compositing through the same `blend_pixel` scalar oracle used by full-page group flattening, while temporarily detaching the active page SMask so caller-provided soft masks are not multiplied twice. This removes the previous compact-only preblend path that bypassed high-quality gamma handling and could apply blend-mode math twice. Evidence: `cargo test -p wellfriendpdf-engine compact_ --lib --jobs 1 -- --test-threads=1 --nocapture` (0) and `cargo test -p wellfriendpdf-engine composite_from_at --lib --jobs 1 -- --test-threads=1 --nocapture` (0).

**Latest source-over mixed-destination row increment:** `render/buffer.rs` now routes normal Compat source-over rows with non-opaque destination alpha through a portable `wide::f32x4` general-row compositor before scalar fallback. The row helper preserves the existing scalar oracle's source-alpha/group-alpha multiplication, destination-alpha composition, RGB rounding, and alpha truncation semantics, uses scalar tails for short rows, exposes `wide_general_pixels` telemetry through CLI compositor stats, and `runtime.rs` names the `mixed_destination_source_over_f32x4` capability. Evidence: `cargo test -p wellfriendpdf-engine composite_from_mixed_destination_general_row_uses_wide_path --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine "composite_from_" --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo fmt --all --check` (0), `cargo check -p wellfriendpdf-engine --lib --jobs 1` (0), `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` (0), `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` (0), and `cargo clippy -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1 -- -D warnings` (0).

**Latest soft-mask mixed-destination row increment:** `render/buffer.rs` now routes normal Compat soft-mask rows with non-opaque destination alpha through a portable `wide::f32x4` general-row compositor before scalar fallback. The helper uses the existing `soft_mask_effective_alpha` byte contract, preserves scalar RGB rounding and alpha truncation, keeps scalar tails for short rows, exposes `wide_soft_mask_general_pixels` telemetry through CLI compositor stats, and `runtime.rs` names the `soft_mask_mixed_destination_f32x4` capability. Evidence: `cargo test -p wellfriendpdf-engine composite_from_soft_mask_mixed_destination_general_row_uses_wide_path --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine "composite_from_" --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo fmt --all --check` (0), `cargo check -p wellfriendpdf-engine --lib --jobs 1` (0), `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` (0), `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` (0), and `cargo clippy -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1 -- -D warnings` (0).

**Latest SVG/PS vector text metric-source increment:** `render/glyph_outline.rs` now exposes strict vector-only glyph advance extraction for sfnt and bare CFF fonts, `render/text_decode.rs` shares that rule for decoded glyphs, and `render/vector_fallback.rs`, `render/svg.rs`, and `render/postscript.rs` now require PDF widths, strict font advances, or valid vertical advances before native text replay. SVG/PostScript no longer synthesize `500` horizontal or `-1000` vertical advances for vector text; metric-unavailable text stays on the typed whole-page raster/refusal boundary instead of emitting incomplete native output. Evidence: `cargo test -p wellfriendpdf-engine ordinary_text_without_metric_preflight_stays_whole_page_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine text_clipping_render_modes_without_font_preflight_stay_whole_page_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine svg_output_safe_extgstate_font_stays_vector_and_applies_font_size --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine ps_output_safe_extgstate_font_stays_vector_and_applies_font_size --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` (0), and `cargo test -p wellfriendpdf-engine text_clipping_render_mode_stays_native --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` (0).

Additional SVG/PS vector glyph-outline exactness gap completed in this continuation: vector text outline emission now uses mapped-outline helpers that refuse code-point-derived compatibility glyph IDs. `vector_fallback.rs` requires a real mapped outline for visible or clipping text even when PDF widths provide movement, and `svg.rs`/`postscript.rs` use the same strict helper before emitting glyph paths. Evidence: `cargo test -p wellfriendpdf-engine vector_mapped_outline_refuses_synthetic_simple_gid --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine ordinary_text_without_metric_preflight_stays_whole_page_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo test -p wellfriendpdf-engine text_clipping_render_modes_without_font_preflight_stay_whole_page_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` (0), `cargo check -p wellfriendpdf-engine --lib --jobs 1` (0), and `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` (0).

**Latest exact raster text metric-source increment:** `render/page_renderer.rs` now routes high-quality/exact raster text movement through explicit metric-source resolution. Exact raster text may advance from PDF widths, native Type 3 metrics, or strict variation-aware sfnt/bare-CFF font advances; otherwise it records a typed exactness refusal instead of using the raster-compatible `500` horizontal fallback. Vertical text now requires a valid vertical CMap advance instead of synthesizing `-1000`. Compatibility mode keeps the legacy deterministic best-effort fallback for metricless ordinary fonts. Evidence: `cargo test -p wellfriendpdf-engine text_advance --lib --jobs 1 -- --test-threads=1 --nocapture` (0).

**Latest display-list CPU path-mask cache increment:** `render/display_list.rs` now backs direct CPU fill and stroke path-mask caches with byte-accounted LRU order. Cache hits refresh recency, key replacement subtracts the old byte charge before admission, and entry/byte pressure evicts one cold mask at a time instead of clearing all hot masks. Evidence: `cargo test -p wellfriendpdf-engine cpu_path_ --lib --jobs 1 -- --test-threads=1 --nocapture` (0).

**Latest packed-plan missing-resource descriptor increment:** `render/plan.rs` now emits explicit `PackedCompileRefusal` descriptors when Image XObject, Form XObject, or named shading operations are compiled with an available resource table but the named resource is absent. Packed execution dispatches those refusals instead of producing a descriptor that falls back to runtime name lookup. Evidence: `cargo test -p wellfriendpdf-engine packed_plan_refuses_missing_resources_when_resource_table_is_available --lib --jobs 1 -- --test-threads=1 --nocapture` (0) and `cargo test -p wellfriendpdf-engine packed_plan_refuses_resolved_xobject_subtype_mismatch --lib --jobs 1 -- --test-threads=1 --nocapture` (0).

**Latest completed source increment:** deterministic font substitution now carries descriptor Symbolic/Nonsymbolic, Italic, ForceBold, and FontWeight hints into provider selection. Unknown non-Standard14 symbolic descriptor fonts route to the symbolic coverage face and report `CoverageOnly` plus provider selection reason `symbolic_flag` in compatibility mode; high-quality/exact rendering still refuses generic bundled replacement.

Additional CMM validation gap completed in this continuation: explicit `NativeLittleCms` render-contract policy is now refused during validation when the active build/target has no native CMM backend, including separation-preserving prepress combinations that previously only required the enum value.

Additional render-contract exactness gap completed in this continuation: `ExactnessPolicy` is now accepted independently from `CompositingPolicy` and is honored by the active renderer's exact-refusal boundary. Compat compositing can request `HighQualityExact` fail-closed behavior, and HighQuality compositing with compatibility exactness still receives the non-HighQuality typed retained-replay refusal for unsupported retained lists instead of a canonical raw fallback.

Additional render-contract determinism gap completed in this continuation: `DeterminismPolicy::BestEffortResearch` is now accepted as a looser policy satisfied by the deterministic CPU renderer, while retaining distinct render-contract cache identity.

Additional render-contract rendering-intent gap completed in this continuation: `RenderingIntent` is now accepted as an active CPU-renderer policy, seeds the initial graphics state for contract renders and annotation appearances, maps PDF `ri`/ExtGState `RI` names into CMM options, participates in ICC transform and raw image cache identity, and is passed through active ICCBased/named-color paint and image decode paths. Full native CMM, proof/separation output, overprint, and corpus-level color validation remain incomplete.

Additional direct PDFium harness gap completed in this continuation: `--forms 1` now initializes PDFium's public form-fill environment, wraps each loaded page with `FORM_OnAfterLoadPage`/`FORM_OnBeforeClosePage`, renders widget appearances with `FPDF_FFLDraw`, and records `forms_rendered`/`form_rendering` plus manifest `formfill_api`. Form rendering combined with custom matrix, clip, or crop-box selection now emits a typed `forms_policy` JSONL error instead of silently producing a content-only bitmap under a misleading form flag.

Additional direct PDFium harness metadata gap completed in this continuation: per-page JSONL now records actual output dimensions, raw byte count, effective matrix, effective clip, output kind, and explicit serial worker execution; the manifest records requested matrix/clip, background, serial worker execution, and output paths. Raw-output path allocation and bitmap byte-size overflow now fail with typed JSONL errors. A compile-only MSVC `/W4 /WX` guard against stub public PDFium headers passed without provisioning or linking PDFium.

Additional direct PDFium harness typed-classification gap completed in this continuation: every JSONL error now carries `failure_class` alongside the precise `code` and `detail`. The classes separate invalid requests, unsupported option combinations, PDFium runtime failures, bitmap surface failures, raw output failures, manifest output failures, and generic harness failures, while preserving the existing specific codes for consumers that already key on them. The compile-only MSVC guard was rerun through `VsDevCmd.bat` and passed against the stub public PDFium headers without provisioning or linking PDFium.

Additional separable row gap completed in this continuation: the engine opaque-destination separable row compositor now accepts Overlay, ColorDodge, ColorBurn, HardLight, and SoftLight in both the public `render-simd` separable helper and the guarded portable-wide fallback, completing the helper's coverage of the PDF separable blend-mode set. The native x86/x86_64 helper now covers the full separable set before portable/scalar fallback, using integer lanes for Multiply, Screen, Overlay, Darken, Lighten, HardLight, Difference, and Exclusion, and float lanes for ColorDodge, ColorBurn, and SoftLight. High-quality/group-space blending and broader non-normal coverage remain incomplete.

Additional WASM separable lane gap completed in this continuation: the wasm32+`simd128` separable opaque-destination helper no longer uses a scalar per-channel loop for any PDF separable blend mode. ColorDodge and ColorBurn now use wasm float lanes for the division-heavy byte contract, SoftLight uses wasm float lanes for the polynomial/sqrt branch, and all separable modes force alpha lanes to 255 after each group store. A temporary `.work` cdylib smoke executed the helper under Node and returned `run_wasm_simd_smoke=0`. Broader non-normal/high-quality SIMD rows and general wasm-bindgen-test harness execution remain incomplete.

Additional visual-normalization metric gap completed in this continuation: `tools/renderer-visual-diff/visual_diff.py` now reports RGB-only `color_mae`, `color_rmse`, and `max_color_delta` separately from alpha error, and `normalization-manifest.json` advertises those fields in the comparison output. The compact pytest suite covers the new metrics without any PDF corpus or renderer benchmark.

Additional render-contract color-management gap completed in this continuation: `ColorManagementPolicy` is now accepted as an active CPU-renderer policy for non-proof contract renders, is inherited by annotation appearances, soft masks, and transparency groups, selects the ICC backend policy used by active ICCBased/named-color paint and image decode paths, and salts raw image cache identity. `PortableQcms` forces portable qcms even on native-capable builds, `NativeLittleCms` remains validation-gated by native backend availability, and `DeterministicFallback` avoids ICC backend transforms instead of silently using qcms/native CMM.

Additional calibrated image ColorSpace gap completed in this continuation: direct CalGray/CalRGB/Lab image conversion and Indexed calibrated base-palette conversion now use checked parameter readers, so missing parameter arrays, missing required `/WhitePoint`, malformed optional `/Gamma` or `/Matrix`, invalid Lab `/Range`, and unresolved parameter references fail typed before conversion can synthesize default calibrated-space parameters.

Additional shading metadata gap completed in this continuation: active named-shading and shading-pattern validation now requires `/ShadingType`, and lower-level mesh decode/paint helpers require explicit mesh `ShadingType` values plus valid Type 5 `/VerticesPerRow` metadata. Missing type fields no longer default to unsupported sentinel or Type 4/6 mesh semantics before validation/direct helper paint.

Additional direct shading helper metadata gap completed in this continuation: `render/page_renderer.rs::validate_shading_dictionary_for_paint` is now crate-visible, and `render/shading.rs::ShadingRenderer::paint_with_options` invokes it before dispatching direct Type 1-7 shading paint. Standalone direct shading helper calls with missing required metadata, such as missing `/ColorSpace`, now decline paint instead of using the lower-level `/DeviceRGB` or geometry/function defaults.

Additional active pattern metadata gap completed in this continuation: pattern fill/stroke dispatch now requires `/PatternType` before selecting tiling or shading pattern replay. Missing pattern type fails typed as missing required metadata instead of flowing through an unsupported PatternType 0 sentinel.

Additional print-proof gap completed in this continuation: `PrintProfile::Proof` now requires `ColorManagementPolicy::NativeLittleCms` instead of letting portable/deterministic CMM silently pass. When native CMM is available and the document has a catalog OutputIntent `DestOutputProfile`, contract rendering applies the existing native output-intent proof transform to the final RGB surface before halftone screening; missing profiles or unavailable transforms return typed unsupported errors.

Additional proof OutputIntent selection gap completed in this continuation: proof profile discovery now scans catalog `/OutputIntents` candidates until it finds the first dictionary with `DestOutputProfile`, so an unprofiled first intent no longer hides a later usable profile. Malformed or image-filtered profile streams still fail closed instead of being skipped. Evidence: `cargo test -p wellfriendpdf-engine proof_output_intent_profile_scan_skips_unprofiled_entries --lib --jobs 1 -- --test-threads=1 --nocapture`.

Additional render-contract overprint gap completed in this continuation: `OverprintPolicy::Preview` is now accepted as an active CPU-renderer policy and gates PDF graphics-state DeviceCMYK overprint preview for fill/stroke painting and plate-contribution modeling; `OverprintPolicy::Disabled` forces knockout behavior for contract renders. `OverprintPolicy::PreserveSeparations` remains a typed unsupported semantic until true separation-preserving output is implemented.

Additional scan-converter gaps completed in this continuation: guarded opaque rectangular miter stroke span fast path with opposite-winding closed-stroke contour correction, derivative-root monotonic cubic decomposition before recursive flatness subdivision, finite transformed-point filtering before scanline/stroke emission, sampled-coverage direct spans for simple solid butt hairlines, guarded sequential active-edge reuse for scanline fill/binary-clip/alpha-mask/subsampled-compositor rows with bucket and subsample parity tests, exact direct spans for monotonic collinear axis-aligned solid butt/projecting-square polylines, and sampled-coverage direct spans for monotonic collinear simple solid butt hairline polylines.

Additional color-glyph fallback gap completed in this continuation: `page_renderer.rs` now treats COLR/CPAL, SVG-in-OpenType, raster color glyph decode errors, COLRv1 temporary-surface denials, and Porter-Duff source-surface denials as a declined color fill rather than a successful paint. The normal glyph outline path can then paint the glyph in compatibility mode instead of silently omitting it after a failed bounded color-glyph sub-render.

Additional SVG-in-OpenType color-glyph geometry gap completed in this continuation: `color_glyph.rs` now distinguishes omitted optional SVG geometry attributes from present malformed numeric attributes. Omitted `x`, `y`, `cx`, `cy`, `x1`, `y1`, `x2`, and `y2` keep SVG defaults, while malformed present optional or required numeric geometry attributes fail typed instead of being treated as absent and painted at default coordinates. Negative `stroke-width` also fails typed instead of being clamped to zero.

Additional SVG-in-OpenType color-glyph paint-metadata gap completed in this continuation: `color_glyph.rs` now rejects out-of-range static-subset `rgb()` components, `rgb()` percentages, and opacity attributes instead of clamping them into range. Missing optional paint opacity still keeps the SVG default, but present invalid color or opacity metadata fails typed before a color glyph can paint with silently altered color or alpha.

Additional Type 3 FontMatrix fail-closed gap completed in this continuation: `page_renderer.rs` now keeps the existing absent `/FontMatrix` scale default, but present non-array, wrong-length, nonnumeric, or non-finite Type 3 `/FontMatrix` values record typed renderer errors before glyph CTM construction. Malformed Type 3 matrices no longer collapse to the 0.001 fallback scale before cached geometry, rendered glyph caches, or full CharProc replay can paint.

Additional Type 3 FontBBox cache-bound gap completed in this continuation: `page_renderer.rs` now requires exact four-number finite font-level `/FontBBox` arrays before using them for bounded rendered-glyph cache surfaces. Overlong or malformed FontBBox arrays no longer seed truncated cache bounds; they decline cached bounded-surface admission and fall back to full CharProc rendering.

Additional image Filter metadata gap completed in this continuation: `page_renderer.rs` now requires inline image, Image XObject, and referenced explicit-mask `/Filter` metadata to be a single name or an all-name array before bpc/color-space planning. Malformed filter arrays no longer drop non-name entries before terminal-codec metadata defaults, cache-key construction, or mask decode planning.

Additional image boolean metadata gap completed in this continuation: `page_renderer.rs` now requires active inline image and Image XObject `/ImageMask` and `/Interpolate` entries, plus referenced explicit-mask `/ImageMask` entries, to be booleans before bpc/color-space planning, decode-cache identity construction, mask decode planning, or scaled-image fast-path selection. Missing booleans still use PDF defaults, but malformed present booleans record typed render errors instead of defaulting to `false`.

Additional transparency group flag gap completed in this continuation: `page_renderer.rs` now requires active Form XObject transparency-group `/I` and `/K` entries to be booleans before backdrop selection, knockout setup, offscreen allocation, or child rendering. Missing flags still use the PDF default `false`, but malformed present flags record typed render errors instead of defaulting to `false`.

Additional malformed Form Matrix gap completed in this continuation: `page_renderer.rs` now defaults Form `/Matrix` to identity only when the entry is absent, while present non-array, short, extra-long, non-numeric, or non-finite matrices record typed renderer errors for Form XObjects, annotation appearances, and soft-mask groups. `vector_fallback.rs` rejects malformed present Form matrices before native SVG/PS vector replay instead of treating them as identity transforms.

Additional malformed Form BBox gap completed in this continuation: `page_renderer.rs` now requires Form XObjects, annotation appearance Form streams, and SMask `/G` Form streams to carry an exact four-number finite `/BBox` before stream decode, offscreen allocation, retained-plan cache admission, clipping, or child render dispatch. Missing, non-array, nonnumeric, non-finite, short, or overlong Form BBox values fail typed instead of rendering unclipped/unbounded Form content. `vector_fallback.rs` also requires exact Form BBox metadata before SVG/PS vector-safe Form regionalization, so malformed Form bounds route away from native vector replay instead of dropping the local Form clip.

Additional annotation Rect gap completed in this continuation: `page_renderer.rs` now requires visible annotations that pass print/form/optional-content filters to carry an exact four-number finite `/Rect` before appearance selection, synthetic appearance construction, cache lookup, or appearance rendering. Missing, non-array, nonnumeric, non-finite, short, or overlong annotation rectangles record typed render errors instead of silently skipping visible annotations or truncating/filtering malformed rectangle arrays.

Additional annotation visibility-flag gap completed in this continuation: `page_renderer.rs` now preflights present annotation `/F` values as integers before display/print visibility decisions. Missing `/F` still uses the PDF default flags, but present malformed flags record typed render errors instead of defaulting to zero and treating the annotation as visible.

Additional annotation Subtype gap completed in this continuation: visible annotation rendering now requires `/Subtype` to be present and name-valued before FormRenderPolicy handling, optional-content checks, appearance selection, or synthesis. Missing or malformed `/Subtype` records typed render errors instead of silently skipping visible annotation paint or rendering with ambiguous annotation semantics.

Additional optional-content visibility gap completed in this continuation: `optional_content.rs` now exposes strict render visibility evaluation, and `page_renderer.rs` routes active marked-content, XObject, annotation, shading, and pattern `/OC` decisions through it. Malformed `/OCProperties`, malformed default-configuration fields, malformed or missing `/OCGs`, malformed OCG dictionaries, missing property resources, cyclic references, non-OCG/OCMD objects, empty or malformed OCMD member lists, and unsupported OCMD policies record typed render errors instead of treating uncertain visibility as visible.

Additional annotation appearance-state gap completed in this continuation: `page_renderer.rs` now treats author-provided `/AP /N` state dictionaries as strict selected appearance metadata. Present `/AS` must be a name, the named state must exist, and the selected state object must resolve to a stream; malformed selected state metadata records typed render errors instead of selecting `/Off`, the first non-Off state, or synthesized appearance fallback.

Additional synthesized annotation geometry gap completed in this continuation: when no author-provided appearance is selected, `page_renderer.rs` now preflights text-markup `/QuadPoints`, line annotation `/L`, and ink annotation `/InkList` before synthetic appearance construction. Missing, non-array, nonnumeric, non-finite, short, overlong, uneven, or structurally malformed geometry records typed render errors instead of synthesizing full-Rect markup, skipping line/ink paint, filtering bad coordinates, or truncating extra points.

Additional synthesized annotation paint-metadata gap completed in this continuation: the same no-author-appearance path now preflights present `/C` color arrays and `/CA` opacity values for synthesized markup, square, circle, line, ink, and FreeText annotations. Missing optional color/opacity still uses the PDF/default synthesis behavior, but malformed, nonnumeric, non-finite, wrong-cardinality, or out-of-range present values record typed render errors instead of defaulting colors or clamping opacity.

Additional synthesized annotation helper-local gap completed in this continuation: the lower-level synthesized markup, line, ink, and FreeText helpers now enforce their own present `/C`, `/CA`, `/QuadPoints`, `/L`, `/InkList`, and `/Q` validity checks. A direct helper call can no longer clamp/default malformed paint metadata, broaden malformed text-markup geometry to a full-Rect mark, truncate line coordinates, filter bad coordinates, emit a partial ink path prefix, or default malformed FreeText alignment.

Additional vector `TJ` helper-local gap completed in this continuation: the lower-level SVG/PS vector text string collector now accepts only string chunks plus integer/real kerning adjustments in `TJ` arrays. If reached directly, malformed non-string and nonnumber array members now decline native vector text replay instead of being filtered away while later valid strings still paint.

Additional synthesized widget helper-local gap completed in this continuation: `synthesize_annotation_appearance` now invokes the widget metadata refusal boundary locally before no-author-appearance Widget construction, and the lower-level `/DA` parser plus `/MK` color helper reject malformed present values instead of returning defaulted style or clamped chrome colors. A direct helper call can no longer synthesize Widget pixels from malformed default appearance or MK paint metadata below the public annotation preflight.

Additional synthesized FreeText alignment gap completed in this continuation: no-author-appearance FreeText synthesis now preflights present `/Q` values before text layout. Missing `/Q` keeps established left-alignment behavior, but present malformed alignment records typed render errors instead of silently defaulting to left alignment.

Additional synthesized FreeText contents gap completed in this continuation: no-author-appearance FreeText synthesis now preflights present `/Contents` display metadata before text layout. Missing `/Contents` keeps the established no-synthesis behavior, but malformed present contents record typed render errors instead of filtering display arrays or rendering partial content.

Additional widget MK paint-metadata gap completed in this continuation: no-author-appearance Widget synthesis now resolves `/MK` through the reader and preflights present `/BG` and `/BC` color arrays before drawing synthesized field/button chrome. Missing optional MK colors still uses established widget defaults, but malformed MK dictionaries, nonnumeric, non-finite, wrong-cardinality, or out-of-range present color values record typed render errors instead of defaulting or clamping background/border colors.

Additional widget default-appearance gap completed in this continuation: no-author-appearance Widget synthesis now preflights inherited field or AcroForm `/DA` strings before parsing synthesized text style. Missing `/DA` still uses the established widget defaults, and valid font size `0` still auto-sizes, but present non-string or unparsable `/DA`, malformed `Tf`, and malformed, nonnumeric, non-finite, wrong-cardinality, or out-of-range `g`/`rg`/`k` color operands record typed render errors instead of silently using default text style or clamping color.

Additional widget appearance-state gap completed in this continuation: no-author-appearance Widget synthesis now preflights present `/AS` values before checkbox/radio state construction. Missing `/AS` keeps existing value-driven synthesis behavior, but present non-name `/AS` records typed render errors instead of being ignored while `/V` drives checked-state fallback.

Additional widget field-type gap completed in this continuation: no-author-appearance Widget synthesis now preflights inherited `/FT` before field-kind dispatch. Missing `/FT` still leaves no field kind to synthesize, but present malformed field types record typed render errors instead of silently skipping widget synthesis.

Additional widget field-integer gap completed in this continuation: no-author-appearance Widget synthesis now preflights inherited `/Ff` and `/Q` plus AcroForm `/Q` before field kind and alignment synthesis. Missing entries keep established PDF defaults, but present malformed integers record typed render errors instead of defaulting to checkbox, single-line text, or left alignment.

Additional widget AcroForm boolean gap completed in this continuation: no-author-appearance Widget synthesis now preflights AcroForm `/NeedAppearances` before checked-state synthesis. Missing `/NeedAppearances` keeps the PDF default `false`, but present malformed booleans record typed render errors instead of defaulting to `false`.

Additional widget display-value gap completed in this continuation: no-author-appearance Widget synthesis now preflights present text-field `/V`, button `/V`, choice `/V`, choice `/Opt`, and pushbutton `/MK /CA` display metadata before appearance construction. Missing optional values keep the established no-synthesis/default behavior, but malformed present values record typed render errors instead of filtering display arrays, skipping invalid choice options, treating malformed button values as unchecked, or dropping malformed captions.

Additional spatial-culling/allocation gaps completed in this continuation: `RenderSpatialIndex` now builds a deterministic bounded BVH hierarchy for larger known-bounds plans and queries it with the same unknown-op execution, exact bounds filtering, deduplication, and paint-order restoration as the linear/grid paths; `query_into` lets warm replay callers reuse the result vector at the API boundary, linear/grid/BVH queries write directly into that caller-owned buffer without a separate candidate vector, `RenderPlan::execute_vector_tile_with_scratch` lets vector-tile replay reuse caller-owned operation-selection scratch, and `RenderPlan` batch metadata now splits contiguous descriptor-backed and pure-vector hot-op runs.

Additional packed state/path arena proof completed in this continuation: repeated identical vector paints now have a focused guard proving they share one packed path arena entry, one packed DrawState arena entry, and matching hot-op payload/state IDs. This closes the stale "state intern tables absent" source-report claim for vector paint payloads while keeping resource-specific backend arenas partial.

Additional packed-plan fail-closed gap completed in this continuation: `RenderStatePlanAdapter` now records fatal renderer errors for unsupported retained state descriptors and packed compile refusals instead of logging/skipping them, and `PackedDisplayList::execute_plan` honors a dispatcher stop hook after those typed refusals so retained descriptor replay cannot silently continue producing partial pixels.

Additional CCITT decode gap reduced in this continuation: `images/ccitt.rs` exposes `decode_window` with a bounded grayscale sink that decodes sequential CCITT rows while retaining only the requested source rectangle, including clean refusal for out-of-range windows. `render/image_decode_planning.rs` now emits guarded CCITT `SubRect` plans for axis-aligned source-clipped XObjects and inline images that do not require reduction or full-image postprocessing; `page_renderer.rs` decodes/caches XObject windows, decodes inline windows through the inline scheduler, and paints both through a source-region CTM. `runtime.rs` discloses the guarded active path. RB-06 remains partial because CCITT reduction/progressive/non-axis/unsupported-postprocessing-window paths, JPEG region/progressive output, and JBIG2/lossless ROI/reduction/progressive output remain incomplete; JPX ROI/region/tile/component/progressive output is reported unavailable at the current decoder API boundary.

Additional raw SMask source-window gap reduced in this continuation: `render/page_renderer.rs` now lets matching unfiltered grayscale image `/SMask` streams share the guarded raw Image XObject source-window boundary. The main raw image and soft mask are cropped through the same source rectangle, and `images/smask.rs` refuses source-window combine unless dimensions, filter state, color family, and supported bit depth match the strict window path. Filtered, mismatched, non-grayscale, reduced, or otherwise postprocessed soft masks still require full-image decode or typed refusal. This reduces the previous RB-06 SMask/postprocessing full-image boundary for a safe raw subset.

Additional DCT/JPEG scaled-decode gap reduced in this continuation: `render/image_decode_planning.rs` now recognizes `jpeg-decoder` reduced-IDCT support and marks safe DCT downscale plans with native 1/2, 1/4, or 1/8 reduction tiers. `images/decoder.rs` records original JPEG header dimensions before scaling, validates PDF dictionary dimensions against those original headers, and exposes scaled XObject/inline DCT decode paths. `page_renderer.rs` routes native-reduction DCT plans through those scaled decoders while keeping image masks, SMask/Mask postprocessing, source-region-only requirements, and non-DCT codecs on their existing guarded paths. JPEG region/progressive decode, masked reduced-output alignment, JBIG2/lossless ROI/reduction/progressive output, and CCITT reduction remain incomplete; JPX ROI/region/tile/component/progressive output is an explicit unavailable capability at the current decoder API boundary.

Additional JPX target-resolution decode gap reduced in this continuation: `images/jpx.rs` now exposes `hayro-jpeg2000` target-resolution decode for guarded downscale paths. `images/decoder.rs` parses the original JPX codestream dimensions before reduced decode, validates those original dimensions against the PDF image dictionary, validates the reduced output invariants separately, and exposes reduced Image XObject and inline JPX decode entry points. `render/image_decode_planning.rs` marks safe JPX downscale plans with native reduction tiers when no image mask or full-image postprocessing step requires the original sample grid, `page_renderer.rs` routes those plans through the JPX target-resolution decoder, and `runtime.rs` discloses guarded JPX reduction while ROI/region/tile/component/progressive output remains explicitly unavailable at the current decoder API boundary.

Additional image painter fail-closed gap completed in this continuation: `render/image_painter.rs` now refuses invalid `RawImage` buffers before sampling and no longer uses per-channel `unwrap_or(0/255)` defaults inside `get_pixel_channels`. A malformed short RGB buffer leaves the destination unchanged instead of synthesizing opaque black pixels through the common image painter.

Additional SVG/PS ExtGState regionalization gap reduced in this continuation: `vector_fallback.rs` now treats explicit `TR /Identity`, `TR2 /Identity`, and valid `OPM 0/1` without enabled overprint flags as vector-safe no-op state for resolved opaque ExtGState dictionaries. Non-identity/default transfer functions, invalid OPM values, invalid flatness/smoothness values, and actual overprint flags still force whole-page raster embedding.

Additional SVG/PS alpha ExtGState gap reduced in this continuation: SVG classification is now sink-specific, so finite scalar `CA`/`ca` values in otherwise vector-safe normal-blend ExtGState dictionaries remain native SVG opacity instead of forcing whole-page raster embedding. PostScript/EPS classification accepts opaque alpha and exact zero-alpha paint no-ops; fractional alpha still rejects because the current PostScript sink has no native transparency operator and must not approximate alpha against a white backdrop.

Additional graphics-state flatness/smoothness gap reduced in this continuation: the PDF `i` operator and valid ExtGState `/FL` entries now update active graphics state, and path fill/stroke rasterization, transformed path clip-node cache identity, path mask cache identity, SVG path replay, and PostScript path replay use the normalized flatness tolerance. Valid `/SM` entries are recorded in graphics state and now drive bounded Coons/tensor patch-mesh subdivision tolerance; Type 1-3 shadings remain per-pixel sampled and do not require subdivision tuning.

Additional SVG/PS DeviceCMYK/calibrated/Separation shading regionalization gap reduced in this continuation: simple supported axial and finite/invertible affine-transformed radial DeviceCMYK shadings now convert their Type 2 function endpoints through the deterministic DeviceCMYK preview transform; direct or page-resource-resolved CalGray/CalRGB/Lab shading color spaces convert through the existing calibrated CMM helpers; and supported Separation spaces convert through the existing tint-transform resolver. SVG and PostScript/EPS then emit native DeviceRGB gradients/shfill instead of whole-page raster embedding. Non-linear functions, mesh shadings, malformed or degenerate radial geometry, unsupported named color spaces, advanced pattern paint or resource-bearing tiling cells outside the vector-safe subset, non-identity shading-pattern paint, transparency, and unsupported local constructs remain whole-page SVG/PS raster fallbacks.

Additional SVG/PS stencil-mask and active-pattern-paint gap reduced in this continuation: finite affine Image XObject stencil masks now keep surrounding SVG/PS vector output regional, and PostScript inline/XObject stencils emit bounded `imagemask` regions rather than RGBA `colorimage` regions. The classifier now ignores overwritten dead pattern color state, but still fails closed when fill/stroke/text paint or stencil-mask fill would require unsupported active pattern semantics.

Additional SVG/PS text-outline and clipping gap reduced in this continuation: stroked text modes now carry vector-safe line cap/join/miter/dash state, fill-stroke text modes emit both fill and stroke paints, and font-resolved text-clipping render modes 4-7 accumulate glyph outlines into native SVG `clipPath` and PostScript `clip` state instead of forcing whole-page raster embedding. Text clipping remains fail-closed when font preflight is unavailable or glyph outlines cannot be resolved.

Additional SVG/PS dense-text fallback gap reduced in this continuation: ordinary pages with more than the old 64 text-showing operation scope limit now remain vector-classified instead of forcing whole-page raster embedding. Pattern-painted text outside the supported shading/tiling glyph-outline subset and unresolved/preflight-unavailable text clipping remain explicit fail-closed whole-page fallback cases.

Additional SVG/PS shading-pattern regionalization gap reduced in this continuation: simple `/PatternType 2` shading patterns with absent/finite `/Matrix` and active CTM, and vector-safe axial/radial shading dictionaries now remain native for fill/stroke path paints and glyph-outline text fill/stroke paints. SVG emits a clipped gradient rectangle; PostScript/EPS emits clipped LanguageLevel 3 `shfill`. Unsupported pattern matrices or pattern-space transforms, active patterned text outside the supported shading/tiling glyph-outline subset, alpha/backdrop-observable semantic transparency, and other unsupported pattern constructs remain whole-page fallback.

Additional PostScript exact shading-function gap reduced in this continuation: direct DeviceRGB/DeviceGray axial/radial shadings whose single Type 2 function has a finite domain and exponent now remain regional for PostScript/EPS, direct DeviceCMYK shadings whose single Type 2 function has a finite domain and exponent now remain regional with native `/DeviceCMYK` `shfill`, direct DeviceRGB arrays of one-component Type 2 functions collapse to one exact RGB Type 2 sidecar when all components share the same finite domain and exponent and now serialize as exact per-component PostScript function arrays when domains or exponents differ, direct DeviceCMYK arrays of one-component Type 2 functions now collapse to one exact CMYK Type 2 sidecar for shared domains/exponents or serialize as exact `/ColorSpace /DeviceCMYK` per-component PostScript function arrays for mixed domains/exponents, and direct DeviceRGB/DeviceGray Type 3 stitching functions whose Type 2 segments can be serialized exactly as RGB now remain regional for PostScript/EPS even when the stitch is discontinuous. The vector classifier carries exact Type 2 `/Domain`, `/N`, finite unit `/Range`, per-channel RGB/CMYK function-array metadata, and Type 3 segment metadata for those DeviceRGB/DeviceGray/DeviceCMYK cases, expands DeviceGray functions to exact RGB sidecars, preserves direct DeviceCMYK components for PostScript, and the PostScript sink emits native LanguageLevel 3 `/FunctionType 2`, function-array, or `/FunctionType 3` `shfill`. SVG, calibrated/named nonlinear shadings, non-unit shading dictionaries, out-of-unit or malformed ranges, malformed domains, mesh shadings, malformed shading dictionaries, and unsupported local constructs remain whole-page fallback boundaries instead of being approximated.

Additional PostScript stitching-function gap reduced in this continuation: direct `/DeviceRGB`, `/DeviceGray`, and `/DeviceCMYK` Type 3 stitching functions whose Type 2 segments are finite and unit-domain now remain regional for PostScript/EPS with native `/FunctionType 3` `shfill`, preserving non-linear segment `/N` exponents exactly. CMYK stitching uses `/ColorSpace /DeviceCMYK`; SVG and inexact/non-unit-domain stitching segments remain conservative.

Additional SVG clip-composition gap reduced in this continuation: stacked path clips and Form BBox clips now produce composed SVG `clipPath` definitions that reference the previous active clip instead of replacing it, so later native SVG elements keep PDF's intersecting clip semantics for nested clip paths.

Additional SVG/PS inline-image resource color-space gap reduced in this continuation: complete finite-affine inline images can now preserve native SVG/PS output when their color space is a direct/resource-resolved CalGray/CalRGB/Lab array, a direct/resource-resolved Indexed array, a direct/resource-resolved ICCBased array, a preflighted opaque direct/resource-resolved Separation/DeviceN array, a DCT-filtered opaque resource-resolved Separation or one-colorant DeviceN inline image, or a DCT-filtered calibrated/Indexed/ICCBased inline image, reusing the existing inline decoder and color conversion helpers. `/None` or unresolved Separation/DeviceN/tint spaces, multi-component DeviceN terminal-codec tint-space inline images, non-DCT terminal-codec tint-space inline images, CCITT/JBIG2 terminal inline images declared as non-gray device color spaces, non-DCT terminal-codec calibrated/Indexed/ICCBased inline images, and unsupported inline-image shapes remain whole-page SVG/PS raster fallbacks.

Additional SVG/PS regional emit and active inline resource-color fallback gap reduced in this continuation: SVG/PS regional Image XObject replay now builds decode references from the source image dictionary's declared dimensions, bits, color space, and filter chain instead of assuming DeviceRGB/no filters, and approved regional image/inline-image decode or encode failures now return typed output errors instead of silently dropping the bounded image while reporting `has_regional_images`. Active whole-page fallback rendering now resolves resource-named inline image color spaces through page resources and salts the inline decode cache key with the resolved color-space object, so unsupported regional cases such as `/Separation /None` can still rasterize through the canonical renderer without hitting `/CS0` as an unknown device color space.

Additional pattern missing-resource/no-name gap completed in this continuation: `display_list.rs` now records missing active pattern resources as unsupported retained-plan diagnostics instead of emitting a fully supported `NativePatternPathOp`, and `page_renderer.rs` records a typed `UnsupportedFeature` when `/Pattern cs` or `/Pattern CS` reaches a paint operator without an active pattern name from `scn`/`SCN`. The focused `missing_pattern_resource_is_explicitly_unsupported`, `pattern_color_space_without_pattern_name_returns_typed_refusal`, and `pattern_` tests cover the retained compiler and immediate paint paths.

Additional uncolored tiling-pattern color fail-closed gap completed in this continuation: `page_renderer.rs` now rejects PaintType 2 tiling-pattern paints whose active `scn`/`SCN` color components cannot be reconstructed as DeviceGray, DeviceRGB, or DeviceCMYK. The previous helper inferred `/DeviceRGB` for unexpected component counts such as two components, which could replay a malformed uncolored pattern with fabricated color semantics instead of failing typed.

Additional tiling-pattern required metadata fail-closed gap completed in this continuation: `page_renderer.rs` now requires active PatternType 1 tiling-pattern streams to carry an exact four-number `/BBox`, finite numeric `/XStep` and `/YStep`, `/PaintType` 1 or 2, and `/TilingType` 1, 2, or 3 before exact tile replay. Missing, malformed, overlong, or unsupported values fail typed instead of filtering malformed `/BBox` arrays, defaulting step values to zero, defaulting missing `/PaintType` to 1, or ignoring `/TilingType`.

Additional ExtGState missing-resource gap completed in this continuation: `display_list.rs` now records missing or malformed `gs` operands as unsupported retained-plan diagnostics instead of dropping the operator during compilation, and `page_renderer.rs` records typed `UnsupportedFeature` failures in both immediate and packed descriptor replay when `/ExtGState` lacks the named resource. The synthetic `missing_extgstate_resource_returns_typed_refusal` test covers the retained route that previously painted with default graphics state.

Additional SMask malformed-resource gap completed in this continuation: `page_renderer.rs` now records typed `UnsupportedFeature` failures for malformed soft-mask dictionaries, references, backdrop colors, and transfer functions that previously logged and continued without applying the mask, silently used a default backdrop, or silently used identity/defaulted transfer behavior, including missing `/G`, `/G` resolving to a non-stream object, non-Form `/G` streams, `/G` decode/parse failures, unsupported `/SMask` object types, unresolved SMask references, malformed or unsupported `/S` subtypes, malformed present `/BC` arrays, unsupported or malformed `/TR` functions, malformed supported `/TR` function shapes, and offscreen allocation refusal. `/SMask /None` remains the explicit mask-clear case, and absent `/BC` still uses the spec/default backdrop. The focused `smask_missing_group_returns_typed_refusal`, `unsupported_smask_subtype_returns_typed_refusal`, `malformed_smask_subtype_returns_typed_refusal`, `alpha_smask_malformed_backdrop_color_returns_typed_refusal`, `unsupported_smask_transfer_function_returns_typed_refusal`, `malformed_smask_transfer_function_returns_typed_refusal`, and `smask` tests cover the new refusal boundary and valid soft-mask paths.

Additional active image decode/mask failure gap completed in this continuation: `page_renderer.rs`, `images/decoder.rs`, and `images/smask.rs` now record typed `UnsupportedFeature`/`MalformedPdf` failures when active inline images have malformed `DecodeParms` or fail decode, when image XObjects fail decode, carry malformed `/Decode` arrays, declare unsupported/malformed color spaces, carry unresolved Separation/DeviceN tint-space transforms, or decode to a byte length that does not match declared dimensions/channels, when inline, XObject, or explicit stencil image masks carry malformed `/Decode` arrays, when image masks omit color space and therefore must decode as one-channel stencils rather than default RGB images, when stencil conversion receives a short image-mask buffer, when image XObject soft-mask loading resolves a malformed value/reference, has malformed `/Matte`, fails decode, decodes to dimensions that do not match the source image, decodes to a non-alpha channel count or wrong byte length, or would combine with a pre-alpha main image by dropping existing alpha, or when an explicit `/Mask` reference is not a stream, has an unsupported value, supplies a malformed color-key array, applies a color-key mask to a short source buffer, decodes to dimensions that do not match the source image, or reaches composition with short main/mask buffers. These paths previously logged and continued by dropping the image, treating unsupported color spaces as raw RGB/gray or malformed Indexed palettes as gray, rendering unresolved tint-space transforms as grayscale fallback pixels, ignoring image/stencil-mask decode mapping, padding/truncating decoded pixels, padding stencil/color-key/explicit-mask samples, sampling non-alpha SMask data as grayscale alpha, dropping pre-existing image alpha, ignoring malformed matte data, or rendering the source image unmasked. The focused `inline_image_decode_params_without_filter_returns_typed_refusal`, `image_xobject_decode_failure_returns_typed_refusal`, `image_xobject_malformed_decode_array_returns_typed_refusal`, `image_xobject_unknown_color_space_returns_typed_refusal`, `image_xobject_malformed_indexed_color_space_returns_typed_refusal`, `malformed_resource_separation_image_color_space_returns_typed_refusal`, `resource_separation_image_color_space_uses_tint_transform`, `image_mask_to_stencil_rejects_short_buffer`, `explicit_color_key_mask_rejects_short_main_buffer`, `explicit_image_mask_rejects_short_main_buffer`, `explicit_image_mask_rejects_short_mask_buffer`, `image_xobject_mask_malformed_decode_array_returns_typed_refusal`, `image_xobject_explicit_mask_malformed_decode_array_returns_typed_refusal`, `inline_image_mask_malformed_decode_array_returns_typed_refusal`, `image_xobject_smask_dimension_mismatch_returns_typed_refusal`, `image_xobject_smask_empty_matte_returns_typed_refusal`, `image_xobject_malformed_color_key_mask_returns_typed_refusal`, `image_xobject_mask_dimension_mismatch_returns_typed_refusal`, `build_raw_image_rejects_malformed_decode_array`, `build_raw_image_rejects_mismatched_buffers`, `build_raw_image_uses_one_channel_for_image_mask`, `combine_rgba_rejects_short_mask_buffer`, `combine_rgba_rejects_multichannel_mask_buffer`, `combine_rgba_rejects_prealpha_main_image`, `smask_matte_rgb_rejects_empty_matte_array`, `image_xobject`, `xobject_resource_`, and `inline_image` tests cover the changed image/XObject neighborhood.

Additional terminal codec decoded-length gap completed in this continuation: `images/jpx.rs`, `images/ccitt.rs`, and `images/jbig2.rs` now fail with typed `MalformedPdf` when decoded JPX byte counts or CCITT/JBIG2 grayscale sink sample counts do not exactly match declared dimensions and channels. These terminal decoders previously warning-logged and synthesized plausible pixels by padding or truncating their output. The focused `jpx_exact_length_refuses_short_output`, `jpx_exact_length_refuses_long_output`, `ccitt_full_sink_refuses_short_output`, `ccitt_window_sink_refuses_short_output`, and `jbig2_sink_refuses_short_output` tests cover the new fail-closed boundary.

Additional JPX color-semantics gap completed in this continuation: `images/jpx.rs` now returns typed `UnsupportedFeature` for ICC/unknown JPX color spaces whose channel count is not gray, RGB, or CMYK-compatible. Those decoded samples previously warning-logged and passed through unconverted to renderer samplers, which have no defined color semantics for those channel counts and would paint fallback black pixels.

Additional DCT dimension-consistency gap completed in this continuation: `images/decoder.rs` now returns typed `MalformedPdf` when an inline image or image XObject declares dimensions that differ from the decoded JPEG header. The affected DCT paths previously warning-logged and continued using the JPEG header dimensions, which could hide malformed PDF image metadata behind plausible pixels.

Additional packed sub-byte image gap completed in this continuation: `images/decoder.rs` now returns typed `MalformedPdf` when 1/2/4-bit image sample data does not contain the required row-padded bytes for the declared dimensions and channel count. The old unpacker zero-filled missing row bytes, synthesizing plausible black samples from truncated source data.

Additional DCT color-component gap completed in this continuation: `images/decoder.rs` now returns typed `UnsupportedFeature` when decoded JPEG component count disagrees with the declared PDF image color space, except for the valid declared-CMYK conversion path. The old finish path could hand back non-matching gray/RGB channel counts to the renderer and produce plausible but wrong pixels.

Additional 16-bit image exact-byte gap completed in this continuation: `images/decoder.rs` now returns typed `MalformedPdf` when 16-bit image sample data is shorter or longer than the exact byte count required by declared dimensions and channel count. The old normalizer accepted a trailing half sample because `chunks(2)` treated a single high byte as a complete output sample.

Additional Indexed palette exact-length gap completed in this continuation: `images/decoder.rs` now validates converted Indexed palettes before expansion and indexes only exact palette entries. The old defensive branches could synthesize black palette bytes if a malformed or buggy palette conversion produced fewer bytes than required.

Additional JPX stored-buffer exact-length gap completed in this continuation: `images/jpx.rs` now validates the raw stored JPX decode byte count before alpha-channel splitting and color conversion. The old alpha split used `chunks_exact`, so trailing bytes in an alpha-bearing JPX output could be dropped before the final post-conversion length check.

Additional JPX alpha-composition fail-closed gap completed in this continuation: `images/jpx.rs` now validates gray/RGB color bytes and alpha bytes against the declared JPX dimensions before composing RGBA output. The private JPX alpha helpers no longer use fallback opaque alpha values or allow `chunks_exact` to drop malformed color bytes if they are reused from another decoder path.

Additional JPX SMaskInData metadata gap completed in this continuation: `images/decoder.rs` now validates JPX `/SMaskInData` as integer `0`, `1`, or `2`, and requires decoded alpha samples when `/SMaskInData 1` or `2` declares internal soft-mask data. Malformed metadata or non-zero declarations without decoded alpha now fail typed instead of being ignored by the finalizer.

Additional JPX finalizer invariant gap completed in this continuation: `images/decoder.rs` now validates normalized JPX `RawImage` output at the checked finalizer boundary. Non-8-bit samples, zero-channel output, decoded byte-length mismatches, and decoded dimensions over the image decode budget fail typed before `/SMaskInData` handling or downstream paint/cache behavior.

Additional DCT inline ColorSpace gap completed in this continuation: `images/decoder.rs` now routes the legacy inline `DCTDecode` path through the checked DCT finalizer. Inline JPEG component counts must match the declared PDF image `ColorSpace` before `RawImage` output is returned, so mismatched declarations such as RGB JPEG data under `/DeviceCMYK` fail typed instead of painting plausible bytes with the wrong color semantics.

Additional image normalized-length gap completed in this continuation: `images/decoder.rs` now validates exact normalized sample-buffer length immediately after bit-depth normalization in `build_raw_image`, before decode arrays or color conversion can hide malformed byte counts. The DCT CMYK finalizer shortcut also applies decode-budget and exact decoded-length checks before conversion.

Additional CMYK converter length gap completed in this continuation: `images/decoder.rs` now validates exact DeviceCMYK and fallback ICCBased-CMYK input length before `ColorSpaceConverter::convert_with_options` calls the raw chunked CMYK-to-RGB helper. Short or trailing malformed CMYK buffers now fail typed at the image conversion boundary.

Additional color-glyph JPEG invariant gap completed in this continuation: `render/color_glyph.rs` now validates decoded JPEG color-glyph dimensions, channel count, and byte length before returning `RawImage` output or converting CMYK bytes to RGB. Malformed sbix/CBDT JPEG payloads now fail typed before chunked color conversion can drop trailing bytes.

Additional SVG/PS regional image/mask fail-closed gap completed in this continuation: `render/vector_fallback.rs`, `render/svg.rs`, and `render/postscript.rs` now validate regional decoded `RawImage` buffers before SVG PNG embedding, PostScript `colorimage`, or PostScript `imagemask` emission. Regional color images must be non-empty 8-bit gray/RGB/RGBA with exact decoded byte counts, regional stencil masks must be non-empty exact one-channel 8-bit buffers, and PostScript regional color output clips binary 0/255 RGBA alpha and refuses fractional RGBA instead of silently dropping alpha. The previous regional paths could sample missing mask or grayscale bytes as zero, synthesize transparent/black output, or lose alpha in a bounded vector-output region.

Additional SVG/PS calibrated-shading fail-closed gap completed in this continuation: `render/vector_fallback.rs` now validates CalGray, CalRGB, and Lab shading color-space dictionaries before classifying an axial/radial shading as native SVG/PS vector output. Missing required `WhitePoint`, malformed `Gamma`, malformed `Matrix`, malformed `BlackPoint`, or malformed/inverted Lab `Range` now prevents native vector shading classification instead of converting the shading endpoints with default calibrated-space parameters and silently changing gradient colors.

Additional active named-color fail-closed gap completed in this continuation: `render/colorspace.rs` now resolves valid bare `/DeviceGray`, `/DeviceRGB`, and `/DeviceCMYK` color-space resource aliases only with exact finite components, returns a typed `NamedColor::Invalid` outcome for malformed CalGray, CalRGB, and Lab color-space dictionaries or missing/overlong/non-finite calibrated components, requires exact finite Separation/DeviceN tint components, uses the supplied `/All` tint instead of forcing full ink, and no longer sends unsupported or malformed Separation/DeviceN alternate spaces through the default black or `/DeviceRGB` `ColorSpaceHandler` branches. `render/page_renderer.rs` records missing, unsupported, or invalid active named fill/stroke color spaces as fatal `UnsupportedFeature` conditions; `images/decoder.rs` turns invalid tint-space image conversion into typed refusal; and SVG/PS/shading/vector paths treat invalid named colors as no native color sample rather than silently substituting black.

Additional named-color component exactness gap completed in this continuation: `render/colorspace.rs` now requires exact finite component counts for bare device named-color resources, CalGray/CalRGB/Lab named colors, Separation tint vectors, DeviceN tint vectors, and device alternate-space output before conversion. `render/cmm.rs` also requires ICCBased scalar component vectors to match the profile `/N` exactly. Overlong named-color, tint, calibrated, ICCBased, or alternate-device vectors now fail typed instead of being truncated through finite-prefix checks or later device conversion.

Additional SVG/PS named-paint fail-closed gap completed in this continuation: `render/vector_fallback.rs` now preflights ordinary path fill, stroke, fill-stroke, text fill/stroke modes, Image XObject stencil masks, and inline stencil masks when the active paint uses a named color-space resource. Valid resource-named `/DeviceGray`, `/DeviceRGB`, and `/DeviceCMYK` paint remains vector-safe only with exact finite component vectors; unresolved resources, unsupported names, malformed resolved spaces, invalid tint/calibrated spaces, or missing components force the SVG/PS whole-page typed raster/refusal boundary before `render/svg.rs` or `render/postscript.rs` can reach the generic named-color black fallback.

Additional named-color tint-transform shape gap completed in this continuation: `render/colorspace.rs` now validates Separation and DeviceN tint transforms with `render/function.rs::validate_function_shape` before evaluation, and returns typed invalid named-color output for malformed transform dictionaries or runtime-empty transform output. The named-color path also rejects Function Type 2 and Function Type 3 tint transforms when DeviceN supplies more than one tint input, so multi-input DeviceN cannot silently ignore extra tint components through a single-input function shape; generic function validation remains compatible with existing Type 1 shading behavior.

Additional SMask matte/backdrop color fail-closed gap completed in this continuation: `render/color.rs` now exposes a device-only `ColorSpaceHandler::try_from_components` helper for callers that only have a color-space family name and no calibrated parameter dictionary. `images/smask.rs` uses it for image SMask `/Matte`, and `render/page_renderer.rs` uses it for family-name-only luminosity SMask `/BC` group color conversion, so unsupported or calibrated family names in those contexts fail typed instead of using default calibrated parameters or the generic black fallback. A later SMask slice resolves well-formed non-device luminosity `/BC` values when the actual group color-space object is available.

Additional device-family component exactness gap completed in this continuation: `render/color.rs::ColorSpaceHandler::try_from_components` now rejects missing, overlong, or non-finite `/DeviceGray`, `/DeviceRGB`, and `/DeviceCMYK` component vectors instead of delegating to the defaulting `from_components` converter. This preserves fail-closed behavior for image SMask `/Matte`, luminosity SMask `/BC`, and SVG/PS named-paint preflight when only a family name is available.

Additional active shading ColorSpace fail-closed gap completed in this continuation: `render/page_renderer.rs` now preflights active shading `/ColorSpace` entries before painting and validates sampled function output arity against the resolved color space. Missing, unsupported, malformed calibrated, malformed ICCBased, unsupported Separation/DeviceN, and too-short, overlong, or non-finite device color output cases now produce typed shading refusals instead of reaching the generic component-to-color black/default branch.

Additional SVG/PS vector-shading component exactness gap completed in this continuation: `render/vector_fallback.rs` now requires exact finite sampled function output arity before native SVG/PostScript simple-shading replay for `/DeviceGray`, `/DeviceRGB`, `/DeviceCMYK`, `/CalGray`, `/CalRGB`, and `/Lab` color spaces. Overlong or non-finite vector-shading outputs now stay on the explicit whole-page SVG/PS raster fallback boundary instead of being truncated into native gradients or `shfill`.

Additional malformed shading dictionary/function fail-closed gap completed in this continuation: `render/page_renderer.rs` now validates present shading `/Coords`, `/Domain`, `/Extend`, Type 1 `/Matrix`, Type 4-7 mesh `/BitsPerCoordinate`, `/BitsPerComponent`, required `/BitsPerFlag`, exact finite `/Decode` cardinality, and Type 5 `/VerticesPerRow` fields before active shading or shading-pattern paint, preserving PDF defaults only when those fields are truly absent. `render/function.rs` also exposes strict Function Type 0/2/3/4 shape validation for active render callers and rejects short Type 0 sampled-function streams before interpolation. `render/shading.rs` refuses to instantiate mesh decode state for invalid bit fields or non-exact/non-finite `/Decode` arrays, so direct mesh callers cannot default or truncate malformed decode ranges. `render/vector_fallback.rs` rejects malformed Type 2 function arrays and malformed simple shading `/Coords`, optional `/BBox`, non-default or malformed `/Domain`, and malformed `/Extend` before SVG/PS simple shading native replay. Malformed present shading fields or function `/Domain`, `/Range`, `/C0`, `/C1`, `/Bounds`, `/Encode`, `/Size`, or `/BitsPerSample` fields no longer collapse through numeric filtering/defaults, malformed mesh decode fields no longer default to implied ranges, malformed vector-output simple shading geometry no longer emits native gradients, and missing Type 0 sample bytes no longer read as zero.

Additional direct shading transform/decode fail-closed gap completed in this continuation: `render/shading.rs` now refuses singular or non-finite direct Type 1-3 paint transforms and singular Type 1 shading matrices instead of using identity inverse transforms below active shading validation. Mesh vertex and patch readers now consume exact validated `/Decode` values by direct index after `MeshDecode::from_dict` has proven cardinality, so lower stream readers no longer retain per-component decode defaults.

Additional ExtGState helper-local fail-closed gap completed in this continuation: `content/state.rs` now exposes `GraphicsState::try_apply_ext_g_state`, which validates present ExtGState render metadata before mutating graphics state while retaining well-formed `/SA`, `/AIS`, and `/TK` values as active state. Active page rendering, packed descriptor replay, display-list capture, SVG replay, PostScript replay, vector-output classification, conservative Form subset checks, and text collector state tracking now route resolved ExtGState dictionaries through that strict helper instead of calling the legacy clamping/filtering mutator directly. Malformed lower-level ExtGState dictionaries therefore fail typed, decline vector replay, or skip text-collector state mutation before alpha, blend, line, dash, overprint, transfer, flatness, smoothness, rendering-intent, or font state can be partially applied or silently ignored; visible active/retained/packed paint that observes unsupported stroke-adjustment, alpha-source, or disabled-text-knockout semantics now fails typed at the paint boundary.

Additional Type 0/2/3 function evaluator-local gap completed in this continuation: `render/function.rs::eval_type0` now enforces strict `/Size`, `/Domain`, `/Range`, present `/Encode`, and present `/Decode` metadata at the evaluator boundary itself. `render/shading.rs::eval_type2` and `eval_type3` now enforce required `/Domain`, `/N`, `/Functions`, `/Bounds`, and `/Encode` locally and reject malformed present `/C0`/`/C1` arrays. Direct evaluator use can no longer filter malformed arrays into smaller sample lattices, stitched segments, or component vectors, or substitute default mappings for malformed present shape fields below the public validator.

Additional malformed RawImage sampler fail-closed gap completed in this continuation: `render/image_painter.rs` now samples decoded pixels through `RawImage::pixel` and returns transparent samples for short buffers or unsupported channel counts. Normal active image painting already rejects invalid decoded image shapes before paint; this removes the public sampling helpers' remaining opaque-black defensive return if malformed `RawImage` data is sampled directly.

Additional inline image metadata fail-closed gap completed in this continuation: `render/page_renderer.rs` now refuses inline images whose `/Width` or `/Height` is missing, non-numeric, non-positive, non-integer, or above the renderer dimension limit; refuses non-mask inline images whose `/BitsPerComponent` is missing or not one of 1, 2, 4, 8, or 16 except for terminal JPX filters that carry sample depth in the codestream; and refuses non-mask inline images whose `/ColorSpace` entry is missing or present as a non-name object. Valid inline image masks still use the stencil-mask `/DeviceGray` and 1-bpc path when `/ColorSpace` or `/BitsPerComponent` is omitted. The previous path turned missing/invalid dimensions into a zero-sized no-op, defaulted missing/non-numeric bpc to 8, clamped unsupported bpc values, and defaulted malformed or absent non-mask color-space metadata to `/DeviceGray`, producing plausible pixels from ambiguous source bytes.

Additional image XObject metadata fail-closed gap completed in this continuation: `render/page_renderer.rs` now refuses Image XObjects whose `/Width` or `/Height` is missing, non-integer, non-positive, or above the renderer dimension limit; refuses non-mask Image XObjects whose `/BitsPerComponent` is missing or not one of 1, 2, 4, 8, or 16 except for terminal JPX filters that carry sample depth; and refuses non-mask Image XObjects whose `/ColorSpace` is missing, malformed, or not a name/array except for terminal JPX filters that carry color metadata. The previous active path defaulted missing/invalid dimensions to 1, defaulted missing bpc to 8, clamped unsupported bpc values, and defaulted missing or malformed color-space metadata to `/DeviceRGB` before cache-key construction and decode planning.

Additional explicit image Mask and image SMask metadata fail-closed gap completed in this continuation: `render/page_renderer.rs` now refuses referenced explicit `/Mask` streams whose `/Width` or `/Height` is missing/invalid instead of inheriting the main image dimensions, refuses non-stencil explicit masks whose `/BitsPerComponent` or `/ColorSpace` metadata is missing or malformed instead of defaulting to 1 bpc or `/DeviceRGB`, and validates stencil-mask `/BitsPerComponent` as exactly 1 when present. `images/smask.rs` now refuses image SMask streams whose `/Width`, `/Height`, `/BitsPerComponent`, or `/ColorSpace` metadata is missing or malformed instead of inheriting source-image dimensions, defaulting bpc to 8, or defaulting color space to `/DeviceGray`. Valid stencil masks may still omit bpc and color-space metadata where the PDF stencil path supplies the 1-bpc `/DeviceGray` alpha interpretation.

Additional color-key image Mask range fail-closed gap completed in this continuation: `render/page_renderer.rs::apply_color_key_image_mask` now requires active color-key `/Mask` arrays to contain exactly `2 * component_count` finite integer entries for the resolved DeviceGray/RGB image data. Nonnumeric, non-finite, fractional, out-of-range, overlong, short, or reversed ranges fail typed instead of filtering malformed entries, ignoring extras, clamping or rounding sample bounds, swapping reversed ranges, padding short source pixels, or rendering the source image unmasked.

Additional image cache identity gap completed in this continuation: `render/page_renderer.rs::inline_image_decode_cache_base_key` now hashes the complete inline-image payload instead of only the first 128 bytes and salts the inline-image decode identity with document revision, render-contract fingerprint, tile-local viewport, device transform, render mode, CMM policy/intent, print/annotation/form policy, resource budgets, and optional-content visibility. `render/image_decode_planning.rs` also adds an explicit `ImageComponentSelection` fragment to the image decode contract key so JPX and future component-selective decodes cannot collide with all-component plans. This is a source-level cache-correctness closure; JPX ROI/region/tile/component/progressive output is an explicit unavailable capability at the current decoder API boundary, while JBIG2/lossless region/reduction/progressive and broader component-selective pixel output remain incomplete.

Additional scan-converter convex-geometry fast-path gap completed in this continuation: `render/path.rs` now detects finite, closed, single-subpath convex fills and paints them through direct sampled spans before constructing the general edge-bucket scanline table. The guard rejects non-finite, multi-subpath, degenerate, and concave paths and keeps those cases on the existing bounded generic scanline route. Focused tests compare the convex path byte-for-byte against the generic scanline compositor and prove the concave rejection boundary.

**Fallback categories remaining:** compatibility-mode generic bundled font substitution, JBIG2/lossless codec limitations where native ROI/reduction/progressive output is still missing, JPEG region/progressive limitations where the decoder API still lacks native support, JPX ROI/region/tile/component/progressive capability refusals at the current decoder API boundary, portable qcms default backend where native CMM is not selected, and explicit compatibility SVG/PS/EPS whole-page raster export for unsupported local constructs. Strict SVG/PS/EPS/PS-document export now refuses whole-page raster fallback with typed `UnsupportedFeature`, and unsupported retained display-list replay now fails typed instead of publishing compatibility retained-to-immediate output. Type 3 remains partial, but unresolved glyph programs, symbolic-descriptor generic font compatibility selection, exact/high-quality unsupported retained display-list replay, high-quality generic bundled font replacement, explicit unavailable `NativeLittleCms` backend selection, exact visible unsupported source-region/reduction-required image paths, safe DCT/JPEG downscale plans, guarded JPX target-resolution downscale plans, malformed named-color tint transforms, active shading ColorSpace/dictionary-field/mesh-decode/function-shape/function-output failures, short Type 0 sampled-function streams, malformed public RawImage sampling, missing or invalid inline/image-XObject dimensions or bpc metadata, missing or malformed non-mask inline/image-XObject ColorSpace metadata, and the former JPX-only sampler branch now produce typed refusal, coverage-only compatibility reporting, transparent no-sample output, or use the canonical route rather than degraded output.

**Material-degrading high-quality fallbacks remaining:** none in the runtime fallback-policy matrix. High-quality and exact contracts now fail closed for unsupported retained display-list replay, generic bundled font replacement, strict SVG/PS/EPS whole-page raster fallback, and visible image paths when required source-region/reduction support is unavailable; safe DCT/JPEG downscale plans use native reduced-IDCT output, guarded JPX downscale plans use target-resolution decode, and JPX-specific sampler compatibility has been removed. JPEG region/progressive and JBIG2/lossless ROI/reduction/progressive output remain incomplete; JPX ROI/region/tile/component/progressive output remains an explicit unavailable capability at the current decoder API boundary rather than a silent fallback.

Additional SVG blend-mode regional-vector gap completed in this continuation: `render/vector_fallback.rs` now treats ExtGState `/BM` safety as output-target-specific. Conservative/no-reader paths still require first-supported Normal/Compatible semantics, SVG accepts every syntactically valid PDF blend mode that `GraphicsState` can apply when the rest of the ExtGState is safe, and PostScript now accepts supported alpha/blend metadata statefully while refusing only visible paint that would require unsupported fractional alpha or non-normal blending. `render/svg.rs` maps supported modes to native `mix-blend-mode` styles and threads the style through path, glyph, regional image, inline-image, native shading, shading-pattern, and tiling-pattern paint output. PostScript visible fractional-alpha/non-normal-blend paint, soft masks, semantic transparency groups, and broader unsupported local SVG/PS regionalization remain incomplete.

## Focused checks executed so far

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback svg_output_supported_blend_extgstate_uses_native_mix_blend_mode --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback svg_output_alpha_blend_extgstate_uses_native_opacity_and_mix_blend_mode --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback extgstate --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_blend_or_smask_ext_gstate_stays_whole_page --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- crates/engine/src/render/vector_fallback.rs crates/engine/src/render/svg.rs crates/engine/tests/regional_vector_fallback.rs` | 0 |
| `cargo test -p wellfriendpdf-engine rendering_intent --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine color_management_policy --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine deterministic_fallback_policy_disables_icc_backend --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_cache_key_includes_render_contract_identity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine overprint_preview --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine clip_dag --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine transformed_path_clip --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine content_path_clip_replay_reuses_transformed_clip_node_cache --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_contract_telemetry_report_exposes_cache_counters --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine clip --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine materialize_window_composite_short_circuits_empty_child_window --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine restore_clip_from_dag_node_preserves_unmaterialized_saved_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine dense_intersection_records_composite_metadata --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine q_restore_restores_previous_clip_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine adaptive_tile --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine zero_tile_dimension_selects_adaptive_size --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine tile_origin_participates_in_metadata_culling --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine mark_soft_masks_uses_collected_primary_walk_refs --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine fused_clip --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine composite_from_at_fuses --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine blend_alpha_mask_fuses --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine blend_rgba_pixels_at_fuses --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine fill_rect_fuses --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine separable_opaque_fill_rect_partial_clip --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine separable_rgba_pixels_at_partial_clip --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine separable_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine separable_difference_exclusion_fill_rect_uses_wide_row_path --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine exclusion --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd separable_blend --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd rgba_to_gray8_scalar_matches_contract_luma_weights --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `$env:RUSTFLAGS='-C target-feature=+simd128'; cargo check -p wellfriendpdf-render-simd --target wasm32-unknown-unknown --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-render-simd --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check -p wellfriendpdf-engine --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/buffer.rs crates/engine/src/runtime.rs docs/renderer/final-local-implementation-closure.md docs/renderer/final-universal-renderer-implementation-report.md docs/renderer/final-universal-implementation-report.md docs/renderer/complete-algorithm-and-method-inventory.md` | 0 |
| `cargo test -p wellfriendpdf-engine blend_alpha_mask_fuses_partial_clip_and_smask_into_row_path --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine contract_row_encoder --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine contract_row_encoder --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_document_cache_enforces_aggregate_resource_budget --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine render_document_cache_byte_charges_path_clip_nodes --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cache_evicts_least_recently_used_entry_by_byte_budget --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine render_document_cache_byte_charges_font_maps --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine render_document_cache_byte_charges_display_list_maps --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine display_list_page_invalidation_updates_byte_accounting --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine transparent_page_group_cache_evicts_lru_and_prunes_order_on_invalidation --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-cli render_artifact_cache_stats_json_exposes_retained_byte_accounting_shape --bin wellfriendpdf --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_inline_image_outside_viewport_skips_decode_scheduler --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine tile_origin_participates_in_metadata_culling --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine halftone --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine contract_row_encoder_honors_reverse_byte_order --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine contract_accepts_valid_custom_max_pixel_budget --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine contract_honors_non_white_background --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine contract_media_page_box_uses_media_dimensions --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine contract_prepress_page_boxes_use_retained_dimensions --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine contract_honors_device_translation_transform --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine viewport_device_transform_applies_after_page_transform --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine pixel_window_subtracts_tile_origin_after_device_transform --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine "resource_budget" --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine contract_max_decoded_bytes_limits_page_content_stream_decode --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_page_content_stream_decode_observes_cancel_token --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine visible_clipped_image_marks_source_region_requirement --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine downscaled_image_marks_reduction_requirement --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine exact_contract_refuses_visible_image_requiring_unavailable_region_decode --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ccitt --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine exactness_policy_is_honored_independent_of_compositing_mode --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine best_effort_determinism_policy_accepts_deterministic_cpu_output --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine native_littlecms --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine preserve_separations --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine proof_refuses_deterministic_fallback_cmm --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_validate_accepts_valid_print_combinations --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine valid_combinations_pass --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --features native-cmm-lcms2 --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine "transaction" --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine changed_source_can_invalidate_tile_without_full_page_artifacts --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine explicit_affected_tile_invalidates_without_full_page_artifacts --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine render_cache_invalidates_exact_tiles --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine dirty_regions_convert_to_intersecting_render_tiles --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine dirty_regions_skip_invalid_or_other_page_entries --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine explicit_tile_transaction_invalidates_tile_without_page_artifacts --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine tile_scoped_source_invalidation_prunes_only_matching_raster_tiles --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine program_cache_keys_include_page_and_contract_identity --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine cached_render_dependencies_include_resource_xobjects --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine narrow_page_invalidation_prunes_page_scoped_artifact_caches --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine render_document_cache_retains_form_xobject_programs --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine fallback_report_keeps_codes_and_structured_policy_details --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback svg_output_rotated_image_xobject_uses_affine_regional_embed --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback ps_output_rotated_image_xobject_uses_affine_regional_colorimage --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine axial_shading --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine ext_gstate --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback ext_gstate --jobs 1 -- --test-threads=1`; `cargo test -p wellfriendpdf-engine --test regional_vector_fallback extgstate_font --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine ext_gstate_dash_pattern_updates_graphics_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback svg_output_safe_ext_gstate_stays_vector_and_applies_line_width --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback ps_output_safe_ext_gstate_stays_vector_and_applies_line_width --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading_is_regional --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading_with_nonuniform_ctm_is_regional_vector_output --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine cmyk_ --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine publication_identity --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server capabilities_endpoint_exposes_renderer_cache_pressure_policy --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_contract_telemetry_report_exposes_cache_counters --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server render_contract_png_report_route_returns_png_and_report_part --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server render_contract_raw_report_route_returns_surface_and_report_part --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_image_decode --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine glyph_mask_cache_evicts_lru_without_clearing_hot_masks --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine glyph_mask_cache --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine glyph_mask_atlas --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine blend_alpha_mask_strided --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_contract_telemetry_report_exposes_cache_counters --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type3_and_path_mask_caches_evict_lru_without_full_clear --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine type3_program_cache --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine text_is_replayable_as_native_operation --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine text_showing_native_op_records_tile_culling_bounds --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine text_showing_bounds_use_pre_advance_text_matrix --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine text_descriptor_compiles_tj_without_raw_content_operation --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine execute_plan_dispatches_text_through_typed_descriptor --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image_descriptor --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine execute_plan_dispatches_inline_image_through_typed_descriptor --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_native_op_records_tile_culling_bounds --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine form_xobject_native_op_stores_typed_resource_name --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine named_shading_is_replayable_as_native_operation --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine scanline_crossings_prepare_sorted_monotonic_active_edge_row --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine linked_pair_observes_either_parent --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine reset_refreshes_existing_clones --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine session_request_cancel_yields_resumable_step_and_resume_refreshes_token --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine revise_viewport_hint_obsoletes_prior_publications_and_reorders_work --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_request_cancel_reports_resumable_step --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_revise_viewport_hint_reports_obsolete_publication --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-server progressive_cancel_prevents_further_steps --test progressive_integration --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-server progressive_revise_viewport_reports_obsolete_publication --test progressive_integration --jobs 1 -- --test-threads=1` | 0 |
| `cargo check -p wellfriendpdf-server --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-server render_contract --test server_integration --jobs 1 -- --test-threads=1` | 0 |
| `python -m pytest tools/renderer-visual-diff/test_visual_normalization.py -q` | 0 |
| `cargo test -p wellfriendpdf-engine render_page_returns_font_substitution_report --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine exact_contract_refuses_generic_bundled_font_substitution --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine resource_aware_plan_pre_resolves_high_level_handles --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine pattern --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine color_space --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine optional_content --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine annotation_appearance --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine form_xobject --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_form_xobject_offscreen_budget --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_renders_semi_transparent_red --test integration --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine display_list --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine contract --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine progressive --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-engine typed_state_descriptor --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine risk_flags_report_missing_glyph_and_metric_risks --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine exact_contract_refuses_unsupported_display_list_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-capi capi_render_font_substitution_report_outputs_owned_json --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-server --test progressive_integration --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-cli cli_render_contract_parsers_accept_public_values --bin wellfriendpdf --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-cli render_contract_raw_surface_and_sidecar_runs --test tool_surface --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-cli render_contract_media_page_box_uses_media_dimensions --test tool_surface --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-cli render_contract_trim_page_box_uses_trim_dimensions --test tool_surface --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-cli render_contract_device_transform_shifts_raw_surface --test tool_surface --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine type3 --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine type3_ --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_image_decode --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_image_decode_lifecycle_report_json --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-server progressive_image_decode_lifecycle_route_returns_report --test server_integration --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine packed_vector_plan_replays_without_raw_content_cold_table --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine typed_state_descriptor_has_no_content_operation_in_active_plan --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine unsupported_state_operator_produces_compile_refusal --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine pattern_descriptor --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine execute_plan_dispatches_pattern_through_typed_descriptor --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine optional_content_marked_content_replays_as_state_ops_without_page_fallback --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine ext_gstate_source_dependency_records_bounded_inline_image_tile --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine color_space_source_dependency_records_text_tile --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine document_view --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-capi capi_read_only_report_envelopes --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --nologo /m:1 /p:BaseIntermediateOutputPath=... /p:BaseOutputPath=...` | 1 |
| `dotnet restore bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --nologo` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --nologo /m:1` | 0 |
| `javac --release 25 -d .work/final-universal-renderer-implementation/java-classes bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java` | 0 |
| `cargo check -p wellfriendpdf-py --all-targets --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --all-targets --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-py --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_render_report_outputs_owned_json --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-capi --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-py --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-wasm --all-targets --target wasm32-unknown-unknown --jobs 1 -- -D warnings` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --nologo /m:1 -v:minimal` | 0 |
| `javac --enable-preview --release 25 -d .work/final-universal-renderer-implementation/java-classes bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java` | 0 |
| `cargo check -p wellfriendpdf-engine --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-engine editing_transaction_apply_with_render_invalidation_exposes_dirty_tiles --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_editing_transactions_scene_transaction_font_surfaces_return_owned_outputs --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-server editing_transaction_apply_route_returns_render_invalidation_plan --test server_integration --jobs 1 -- --test-threads=1` | 0 |
| `cargo check -p wellfriendpdf-engine -p wellfriendpdf-capi -p wellfriendpdf-server -p wellfriendpdf-py -p wellfriendpdf-wasm --all-targets --jobs 1` | 0 |
| `javac --release 25 -d .work/final-universal-renderer-implementation/java-classes bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --nologo /m:1` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore -v:minimal` | 0 |
| `javac --enable-preview --release 25 -d bindings/java/target/classes <main Java sources> bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java` | 0 |
| `java --enable-preview --enable-native-access=ALL-UNNAMED -cp bindings/java/target/classes io.wellfriendpdf.WellfriendPdfSmokeTest --contract-builder-only` | 0 |

## Final lightweight gates

The required non-all-features gates in this table were rerun after the current Type 3 parsed-program cache, transparent-page-group cache, packed cold-table cleanup, retained typed-payload updates, bounded color-space/ExtGState source-to-tile dependency updates, public max-temporary-byte contract enforcement, Python/WASM/.NET/Java progressive cancellation binding updates, cross-binding progressive-image lifecycle JSON updates, source viewer queue execution JSON updates, source adjacent-page prefetch execution updates, source viewer callback dispatch JSON updates, C/Python/WASM/.NET/Java synchronous callback helper updates, and sequential active-edge scanline/compositor update: `cargo fmt --all --check`, `cargo check --workspace --all-targets --jobs 1`, and `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` all exited 0. The all-features workspace check and clippy gates were also rerun under the same single-job environment and exited 0. Focused SDK/C/server lifecycle, temporary resource-budget, viewer queue execution, adjacent-page prefetch execution, callback-dispatch, and scanline/path tests plus WASM target, .NET, and Java source builds passed; the broad Java compile that included `WellfriendPdfJUnitTest.java` still requires the external JUnit classpath and was not counted as a lifecycle failure.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine "temporary_budget" --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo check -p wellfriendpdf-capi -p wellfriendpdf-py -p wellfriendpdf-wasm --all-targets --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine viewer_queue_execution --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_execute_viewer_queue_json_advances_work --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-server progressive_queue_execute_advances_owned_work_and_defers_adjacent_prefetch --test progressive_integration --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine adjacent_page_prefetch_execution --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_execute_adjacent_page_prefetch_returns_child_handle --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-server progressive_adjacent_prefetch_execute_creates_child_session --test progressive_integration --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine viewer_callback_dispatch --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine font_signature_decode_dedup_feature_envelopes --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_viewer_callback_dispatch_json_exposes_events --lib --jobs 1 -- --test-threads=1` (covers JSON and `wellfriendpdf_progressive_render_dispatch_viewer_callbacks`) | 0 |
| `cargo test -p wellfriendpdf-server progressive_queue_reports_adjacent_page_prefetch_preview --test progressive_integration --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_image_decode --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_image_decode_lifecycle_report_json --lib --jobs 1 -- --test-threads=1` | 0 |
| `cargo test -p wellfriendpdf-server progressive_image_decode_lifecycle_route_returns_report --test server_integration --jobs 1 -- --test-threads=1` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --nologo /m:1 -v:minimal` | 0 |
| `javac --enable-preview --release 25 -d .work/final-universal-renderer-implementation/java-classes bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java` | 0 |
| `java --enable-preview --enable-native-access=ALL-UNNAMED -cp .work/final-universal-renderer-implementation/java-classes io.wellfriendpdf.WellfriendPdfSmokeTest --contract-builder-only` | 0 |
| `rg` source guard for PDFium harness required switches/manifest/form-fill fields | 0 |
| `cl /std:c11 /W4 /WX /I .work\final-universal-renderer-implementation\pdfium-stub\public /c tools\pdfium-harness\render_page.c` | 0 |
| `cargo test -p wellfriendpdf-engine axis_aligned_solid_stroke_rect --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine axis_aligned_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine rect_stroke --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine hairline --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine axis_aligned_polyline_hairline_fast_path_matches_outline_fill --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine integer_axis_aligned_butt_stroke_fast_path_matches_outline_fill --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine integer_axis_aligned_polyline_stroke_fast_path_matches_outline_fill --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine spatial_index_bvh_query_matches_linear_scan_for_large_plan --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine spatial_index_query_into_reuses_output_buffer --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine spatial_index_query_into_reuses_output_buffer_for_bvh --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine spatial_index_query_into_reuses_output_buffer_for_grid --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine spatial_index --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_plan_tile_replay_reuses_selection_scratch --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_plan_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine scanline --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine stroke_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cubic_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine path::tests --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

Two initial PowerShell-redirection attempts at the workspace check/clippy gates exited `-1` with only cargo progress lines and no Rust diagnostics. The same commands passed when rerun through `cmd /c` redirection under the same single-job environment.

An earlier 2026-08-16 continuation had one all-features workspace clippy attempt time out before returning diagnostics and it was killed. The same all-features check and clippy commands were rerun later in this local single-job environment and completed with exit code 0.

The PDFium harness was not built or executed because this local machine did not have a configured official PDFium SDK root and the task forbids downloading/provisioning comparator binaries.

## Latest local stencil/pattern continuation gates

These gates were run after the SVG/PS Image XObject stencil-mask and active-pattern-paint classifier update, using `CARGO_BUILD_JOBS=1`, `--jobs 1`, and the `.work/final-universal-renderer-implementation` cargo target/temp directories.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback pattern --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback image_xobject_mask --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback inline_image_mask --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback --jobs 1 -- --test-threads=1` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <touched renderer/report files>` | 0 |
| stale guard for old pattern-colour-space, mask-colorimage, and solid-pattern-fallback wording | 1 (no matches) |

## Latest local SVG/PS dense-text and regional-image gates

These gates were run after removing the dense ordinary text whole-page fallback threshold, preserving source image metadata for SVG/PS regional Image XObject replay, propagating approved regional image/inline-image replay failures instead of silently dropping bounded images, resolving resource-named inline image color spaces in the active renderer, and correcting the malformed `Indexed` DCT inline-image fixture palette bound. They used `CARGO_BUILD_JOBS=1`, `--jobs 1`, and the `.work/final-universal-renderer-implementation` cargo target/temp directories.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine active_renderer_inline_image_resource_indexed_color_space_resolves_named_resource --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine raster_fallback_triggers_on_unsupported_constructs --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest local SVG/PS shading-pattern gates

These gates were run after adding guarded SVG/PS regional replay for simple `/PatternType 2` shading-pattern fill/stroke path paints with absent/finite `/Matrix` and active CTM, and vector-safe axial or finite/invertible affine-transformed radial shading dictionaries. Tiling patterns, unsupported advanced pattern-space transforms, patterned text outside that path-paint subset, stencils, transparency, and other unsupported pattern constructs remained whole-page fallback at that checkpoint. They used `CARGO_BUILD_JOBS=1`, `--jobs 1`, and the `.work/final-universal-renderer-implementation` cargo target/temp directories.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine simple_shading_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `git diff --check -- <touched renderer/report files>` | 0 |

## Latest local SVG alpha ExtGState gate

This gate was run after splitting SVG and PostScript vector-output classification for scalar alpha ExtGState dictionaries and then extending SVG scalar opacity from path/text output to regional image and direct shading output. SVG now accepts finite normal-blend `CA`/`ca` opacity as native vector opacity on path, text, regional image, and direct shading elements; PostScript/EPS falls back for fractional alpha and treats exact zero-alpha paint as a no-op. It used `CARGO_BUILD_JOBS=1`, `--jobs 1`, and the `.work/final-universal-renderer-implementation` cargo target/temp directories.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine normal_alpha_extgstate --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ext_gstate --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback svg_alpha --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `git diff --check -- <touched renderer/report files>` | 0 |

## Latest local PostScript zero-alpha ExtGState gate

This gate was run after changing the PostScript vector classifier and sink to accept exact `CA`/`ca` zero-alpha paint as a no-op while retaining whole-page fallback for fractional alpha. The sink now skips fully transparent path, text, shading, image, stencil-mask paint, and fully transparent regional RGBA buffers instead of approximating alpha against a white backdrop. It used `CARGO_BUILD_JOBS=1`, `--jobs 1`, and the `.work/final-universal-renderer-implementation` cargo target/temp directories.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback alpha_extgstate --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine regional_rgba_alpha_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine regional_colorimage_hex --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <touched renderer/report files>` | 0 |

## Latest local SVG/PS no-op ExtGState array gate

This gate was run after aligning SVG/PS vector-output ExtGState classification with the active renderer's `/BM` array rule for the first renderer-supported blend mode. `/BM` arrays whose first supported mode is normal-compatible now remain vector-safe, four-`/Identity` `TR`/`TR2` arrays are accepted as exact no-ops, and malformed arrays or arrays whose first supported blend mode is non-normal still fail closed. It used `CARGO_BUILD_JOBS=1`, `--jobs 1`, and the `.work/final-universal-renderer-implementation` cargo target/temp directories.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine noop_blend_and_transfer_arrays_remain_vector_safe --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_blend_or_smask_ext_gstate_stays_whole_page --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <touched renderer/report files>` | 0 |

## Latest local SVG/PS shading-pattern text/transform/matrix gates

These gates were run after extending the guarded SVG/PS regional replay subset to simple `/PatternType 2` shading-pattern glyph-outline text fills/strokes, active-CTM-transformed path paints, and finite pattern-matrix axial path paints, using vector-safe axial or finite/invertible affine-transformed radial shading restrictions. Tiling patterns, unsupported advanced pattern-space transforms, stencils, transparency, and other unsupported pattern constructs remain whole-page fallback. They used `CARGO_BUILD_JOBS=1`, `--jobs 1`, and the `.work/final-universal-renderer-implementation` cargo target/temp directories.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine matrix_shading_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading_pattern_with_matrix --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine translated_shading_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading_pattern_text --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine fill_stroke_text --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest local malformed-resource refusal gates

These gates were run after converting missing active pattern resources, `/Pattern` paint without an active pattern name, unsupported pattern dictionaries, non-stream PatternType 1 resources, malformed tiling-pattern geometry, tiling-pattern decode/parse failures, missing/malformed named shading resources, active shading dictionaries with missing required fields, malformed mesh-decode fields, unsupported or malformed functions, or unsupported types, unavailable active mesh-shading streams, malformed shading patterns, malformed SMask dictionaries/references, present `/BC` backdrop arrays, and `/TR` transfer functions, active image decode/mask failures including malformed inline `DecodeParms`, malformed image `/Decode` arrays, malformed image color spaces, decoded-length mismatches, malformed image-mask `/Decode` arrays, malformed image SMask streams and `/Matte` arrays, SMask alpha channel/length conflicts, pre-alpha main-image conflicts, malformed color-key `/Mask` arrays, and dimension-mismatched explicit mask streams, missing active XObject resources, non-stream XObjects, missing/unsupported XObject subtypes, Form XObject fetch/decode/parse failures, Form XObject recursion/depth-limit failures, transparency-group Form offscreen allocation denials, selected malformed annotation appearance streams, recursive Type 3 CharProc failures, and missing active ExtGState resources from log-and-skip, pad/truncate, alpha-dropping, or default-state behavior into typed renderer refusals. They used `CARGO_BUILD_JOBS=1`, `--jobs 1`, and single-threaded test execution where applicable.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine pattern_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine missing_pattern_resource_is_explicitly_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine pattern_color_space_without_pattern_name_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine missing_extgstate_resource_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_missing_group_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine unsupported_smask_subtype_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_smask_subtype_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_smask_malformed_backdrop_color_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine unsupported_smask_transfer_function_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image_decode_params_without_filter_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine build_raw_image_rejects_malformed_decode_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_malformed_decode_array_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_unknown_color_space_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_malformed_indexed_color_space_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_mask_malformed_decode_array_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_explicit_mask_malformed_decode_array_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image_mask_malformed_decode_array_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_decode_failure_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_smask_dimension_mismatch_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_smask_empty_matte_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_malformed_color_key_mask_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_mask_dimension_mismatch_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_matte_rgb_rejects_empty_matte_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_combine_rgba_rejects_dimension_mismatch --test integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine xobject_resource_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-engine --test patterns --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_active_shading_dictionary --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_shading_pattern_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <touched renderer/report files>` | 0 |

## Latest exact image decode/SMask length gates

These gates were run after changing decoded image buffers from warning-and-pad/truncate behavior to exact-length `MalformedPdf`, decoding `/ImageMask true` images as one-channel stencils before length validation, refusing unresolved Separation/DeviceN image tint-space transforms instead of rendering grayscale fallback pixels, changing stencil, color-key, and explicit-mask composition to reject short buffers instead of padding samples, and changing image SMask composition to reject short/multichannel alpha buffers and pre-alpha main images instead of padding, sampling the wrong channel data, or dropping alpha. They used `CARGO_BUILD_JOBS=1`, `--jobs 1`, and the `.work/final-universal-renderer-implementation` cargo target/temp directories.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine build_raw_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_resource_separation_image_color_space_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine resource_separation_image_color_space_uses_tint_transform --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine explicit_color_key_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine explicit_image_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_mask_to_stencil --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine combine_rgba --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `git diff --check -- <touched renderer/report files>` | 0 |
| stale guard for old image pad/truncate, tint-space grayscale fallback, explicit-mask padding, and SMask alpha-dropping warning strings | 1 (no matches) |

## Latest image decode cache identity gates

These gates were run after inline-image decode keys stopped hashing only the first 128 payload bytes and started salting the complete inline payload plus active render context, and after the image decode planner added an explicit component-selection fragment for JPX/future component-selective cache identity. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine cache_key_includes_component_selection_identity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image_decode_cache_key_hashes_full_payload_and_render_context --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <image decode cache identity source/report files>` | 0 |

Additional image decode target/backend cache identity gap completed in this continuation: `render/image_decode_planning.rs` now carries explicit `ImageDecodeTargetFormat` and `ImageDecodeBackendIdentity` fields in `ImageContractState` and serializes them into `ImageDecodeCacheKey::to_cache_string`. The active planner maps the accepted schema-v1 `StandardCpu`, `ScalarReference`, and later `ResearchHybrid` backends to the current raw 8-bit interleaved decode output while preserving separate `standard-cpu`, `scalar-reference`, and `research-hybrid` cache identities. This closes the remaining prompt #24 source gap where decoded target format and renderer backend were not explicitly represented in the structured decode contract key. Broader backend runtime parity beyond the currently accepted CPU paths remains incomplete.

## Latest image decode target/backend cache identity gates

These gates were run after image decode planner keys gained explicit decoded target-format and backend fragments. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine cache_key_includes_target_format_and_backend_identity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- <image decode target/backend source/report files>` | 0, CRLF normalization warnings only |

Additional image decode execution-control capability gap completed in this continuation: `ImageDecodeCapabilityReport` now exposes `cancellation` and `memory_budget` control status for every image decode plan. Current codec adapters report renderer-boundary cancellation rather than codec-native cancellation, because `CancelToken` is observed before scheduling/adapter work but the selected JPEG/JPX/JBIG2/lossless codec APIs do not expose an interruptible mid-decode hook. Memory-budget status is also reported as renderer-boundary because the decode scheduler, adapter limits, and checked finalizers enforce budgets around decoder output rather than through codec-native allocation callbacks. `ImageDecodeCapabilityDocumentReport` now publishes native cancellation, renderer-boundary cancellation, and renderer-boundary memory-budget counts, and runtime capabilities state this distinction explicitly.

Additional JPX metadata-inspection capability gap completed in this continuation:
`images/jpx.rs::inspect_metadata` now parses JPX codestream/container headers
through `hayro-jpeg2000` without decoding pixels and reports dimensions,
original bit depth, color-space family, color/stored channel counts, and alpha
presence behind the existing decode-budget guard. `ImageDecodeCapabilityReport`
now serializes `metadata_inspection`, and
`ImageDecodeCapabilityDocumentReport` publishes
`native_metadata_inspection_count`; current JPX plans report native metadata
inspection while keeping ROI/region, codestream tile, component-subset,
progressive pixel output, codec-native cancellation, corpus validation, and
benchmark evidence unavailable or deferred as appropriate.

## Latest image decode execution-control capability gates

These gates were run after per-image decode capability reports gained explicit cancellation and memory-budget control status. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_capability_report_lists_images_without_decoding --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- <image decode execution-control source/report files>` | 0, CRLF normalization warnings only |

## Latest JPX metadata-inspection capability gates

These gates were run after JPX codestream/container metadata inspection became
a public source adapter boundary and after per-image capability reports gained
the `metadata_inspection` field. They stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine jpx --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_capability_report_lists_images_without_decoding --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <JPX metadata-inspection source/report files>` | 0, CRLF normalization warnings only |

Additional .NET/Java render-contract builder parity gap completed in this continuation: the typed builders now expose setters for all non-identity schema-v1 contract policy fields that were previously only mutable through raw JSON or object initializers, including page box, execution/backend/compositing policy, annotations/forms, optional-content identity, text/image/path/subpixel smoothing, color scheme, print profile, halftone, overprint, rendering intent, color-management policy, exactness, determinism, and resource budgets. The Java contract parser now validates schema enum names against typed enums instead of accepting arbitrary non-empty strings for policy fields. Full runtime binding matrices remain future platform validation.

## Latest .NET/Java render-contract builder parity gates

These gates were run after adding full-field .NET/Java typed render-contract setters and Java enum validation. They stayed local and source-level; no native PDF rendering, corpus, VPS, package publication, or external client matrix was used. `gradle` and `mvn` were not available on `PATH`, so Java validation used direct JDK 25 compilation and the existing contract-only smoke entrypoint.

| Command | Exit code |
|---|---:|
| `dotnet build bindings/dotnet/WellfriendPdf/WellfriendPdf.csproj -v:minimal -p:UseSharedCompilation=false` | 0 |
| `dotnet test bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --filter RenderContractBuilderRoundTripsSchemaJson -v:minimal -p:UseSharedCompilation=false` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj -v:minimal -p:UseSharedCompilation=false` | 0 |
| `javac --enable-preview --release 25 -d .work/final-universal-renderer-implementation/java-classes bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java` | 0 |
| `java --enable-preview --enable-native-access=ALL-UNNAMED -cp .work/final-universal-renderer-implementation/java-classes io.wellfriendpdf.WellfriendPdfSmokeTest --contract-builder-only` | 0 |

Additional server render-contract builder gap completed in this continuation: `/api/v1/render-contract` now accepts `page_box` during default contract construction and typed multipart overrides for execution/backend/compositing policy, annotations/forms, optional content, common and per-kind smoothing, color scheme, print profile, halftone, overprint, rendering intent, color-management policy, exactness, determinism, and resource budgets. The route serializes the normalized full-field schema-v1 contract and keeps invalid or unsupported policy combinations behind existing typed server errors. The core contract telemetry now publishes a field-effect matrix that is tested against every serialized schema-v1 field, so source field parity no longer depends on a prose checklist; external client matrix validation remains incomplete.

## Latest server render-contract builder parity gates

These gates were run after adding the full-field server render-contract builder overrides. They stayed local and source-level; no corpus, VPS, native CMM provisioning, package publication, or external client matrix was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-server render_contract_builder_returns_valid_contract_json --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-server --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-server --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <touched binding/server/report files>` | 0 |

## Latest terminal codec decoded-length gates

These gates were run after changing JPX, CCITT, and JBIG2 terminal decoded-output length mismatches from warning-and-pad/truncate behavior to typed `MalformedPdf` failures, after changing unsupported JPX ICC/unknown channel counts from unconverted pass-through to typed `UnsupportedFeature`, after changing DCT dictionary/header dimension mismatches from warning-and-header-use to typed `MalformedPdf`, after changing short packed 1/2/4-bit rows from zero-fill to typed `MalformedPdf`, after changing DCT component-count/color-space mismatches to typed `UnsupportedFeature`, after changing malformed 16-bit sample byte counts to typed `MalformedPdf`, after validating Indexed converted palette lengths instead of synthesizing black palette bytes, and after validating raw stored JPX decoded byte counts before alpha splitting. They stayed local and source-level; no corpus, benchmark, competitor comparison, VPS, or deployment path was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine images::jpx --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 (9 tests) |
| `cargo test -p wellfriendpdf-engine images::ccitt --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine images::jbig2 --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine images::decoder --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 (41 tests) |
| `cargo test -p wellfriendpdf-engine images::decoder::tests::dct_inline_dimension_mismatch_returns_malformed_pdf --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine decode_jpx --test integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine jpx --test integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <terminal codec/report files>` | 0 |
| stale guard for old terminal image decoder pad/truncate, JPX unconverted-pass-through, DCT header-use, packed-row zero-fill, 16-bit half-sample, and Indexed palette black-fill warning/test strings | 1 (no matches) |

## Latest JPX alpha-composition fail-closed gates

These gates were run after changing JPX gray/RGB alpha composition to validate declared color and alpha lengths instead of substituting fallback opaque alpha or letting `chunks_exact` drop malformed color tails. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, or deployment path was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine images::jpx --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 (12 tests) |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- <JPX alpha-composition source/report files>` | 0 |
| stale guard for old JPX opaque-alpha synthesis | 1 (no matches in `images/jpx.rs`) |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest DCT/JPEG reduced-IDCT gates

These gates were run after wiring guarded DCT/JPEG downscale plans through `jpeg-decoder` native reduced-IDCT output for safe Image XObject and inline-image paths. The change stayed local and source-level; it used synthetic JPEG unit coverage only and did not use a PDF corpus, benchmark, competitor comparison, VPS, or deployment path.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine jpeg_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 (9 tests) |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 (27 tests) |
| `cargo test -p wellfriendpdf-engine images::decoder --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 (43 tests) |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <DCT/JPEG reduction source/report files>` | 0 |

## Latest image painter fail-closed gates

These gates were run after changing `ImagePainter` to reject invalid decoded image buffers before sampling instead of substituting fallback black/opaque channel bytes. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, or deployment path was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine paint_image_with_short_buffer_does_not_synthesize_black_pixels --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_painter --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 (23 tests) |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- <DCT/JPEG reduction and image-painter source/report files>` | 0 |
| stale guard for old image-painter fallback sampling and stale DCT/JPEG report strings | 1 (no matches) |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS regional image/mask fail-closed gates

These gates were run after changing SVG/PS regional image and stencil-mask emitters to validate decoded buffers before sampling or embedding, and after changing PostScript regional RGBA output to refuse non-opaque alpha instead of dropping it. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine inline_stencil_mask_to_rgba_refuses_short_mask_buffer --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine regional_colorimage_hex_refuses --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine regional_stencil_mask_validation_refuses_short_mask_buffer --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render::postscript --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render::vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- <SVG/PS regional image/mask source/report files>` | 0 |
| stale guard for old regional zero-fill/drop-alpha sampling strings | 1 (no matches) |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS calibrated-shading fail-closed gates

These gates were run after changing SVG/PS vector shading classification to require well-formed CalGray, CalRGB, and Lab parameter dictionaries before emitting native gradients or `shfill`. Valid calibrated shadings still remain native; malformed calibrated shading dictionaries fall back to the explicit whole-page vector-output raster policy instead of default-parameter gradient conversion. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine "shading_with_malformed" --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine calgray_shading_without_whitepoint_stays_whole_page_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render::vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cal --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine lab --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- crates/engine/src/render/vector_fallback.rs` | 0 |
| stale guard for old calibrated vector-shading default-parameter conversion | 1 (no matches) |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest active named-color fail-closed gates

These gates were run after changing active named color-space resolution to accept valid bare `/Device*` resource aliases, reject malformed CalGray, CalRGB, and Lab definitions/components, propagate invalid Separation/DeviceN calibrated alternates, require exact finite Separation/DeviceN tint components, use the supplied `/All` tint instead of forcing full ink, and refuse unsupported or malformed alternate spaces instead of defaulting to black or `/DeviceRGB`. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine calrgb --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine calgray --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine lab --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine render::colorspace --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine colorspace --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tint --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `git diff --check -- <active named-color source/report files>` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-engine smask_matte_rgb --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_backdrop_color_rejects_unsupported_group_color_space --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest named-color component exactness gates

These gates were run after named-color resolution began rejecting overlong component vectors for bare device aliases, calibrated spaces, Separation/DeviceN tint input, device alternate output, and ICCBased scalar conversion. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine colorspace --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cmm --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tint --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/colorspace.rs crates/engine/src/render/cmm.rs docs/renderer/final-local-implementation-closure.md docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-fallback-closure-report.md docs/renderer/final-fallback-inventory.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md` | 0 |

## Latest named-color tint-transform shape gates

These gates were run after Separation and DeviceN tint transforms began using the active-render function-shape validator before evaluation, and after DeviceN tint transforms started rejecting Function Type 2/3 shapes when multiple tint inputs are supplied. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine tint_transform --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tint --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test shadings function_based_shading_type1_varies_with_x --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest active shading ColorSpace fail-closed gates

These gates were run after active shading dictionary validation began preflighting `/ColorSpace` and function output component arity before paint. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_active_shading_dictionary_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest active shading device component exactness gates

These gates were run after active shading device color validation began requiring exact finite `/DeviceGray`, `/DeviceRGB`, and `/DeviceCMYK` sampled function outputs before paint. Overlong outputs now fail typed instead of being truncated by later device-color conversion. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_active_shading_dictionary_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/page_renderer.rs docs/renderer/final-local-implementation-closure.md docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-fallback-closure-report.md docs/renderer/final-fallback-inventory.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md` | 0 |

## Latest convex fill fast-path gates

These gates were run after finite, closed, single-subpath convex fills began using direct sampled spans before the general edge-bucket scanline compositor. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine convex --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine aa_color_compositor_matches_generic_pixel_compositor --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine evenodd_and_nonzero_fill_both_paint_pixels --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest malformed shading dictionary/function/mesh gates

These gates were run after active shading validation began preflighting present shading `/Coords`, `/Domain`, `/Extend`, Type 1 `/Matrix`, Type 4-7 mesh bit-depth fields, exact finite mesh `/Decode` cardinality, Type 5 `/VerticesPerRow`, supported PDF Function Type 0/2/3/4 dictionary shape, required Type 0 `/BitsPerSample`, and Type 0 sampled-stream byte length before paint. Malformed present shading, mesh, or function arrays/dictionaries now fail typed instead of being filtered/defaulted, direct mesh decode state rejects invalid bit fields and non-exact/non-finite `/Decode` arrays, Type 0 sampled functions no longer default missing `/BitsPerSample` to 8 or read unavailable samples as zero, and SVG/PS simple shading classification rejects malformed Type 2 function arrays before native vector replay. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine mesh_decode --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_shading_geometry_stays_whole_page_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine function_shape --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type0_rejects_short_sample_stream_instead_of_zero_padding --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type0_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_active_shading_dictionary_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_shading_pattern_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_function_array_shading_stays_whole_page_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine function --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <shading function source/report files>` | 0 |

## Latest direct shading helper metadata gates

These gates were run after `ShadingRenderer::paint_with_options` began reusing the active shading dictionary validator before direct helper dispatch. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine direct_shading_paint_rejects_missing_color_space --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_active_shading_dictionary_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_shading_pattern_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test shadings function_based_shading_type1_varies_with_x --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest SMask transfer-function shape gates

These gates were run after SMask `/TR` transfer functions began using strict function-shape validation before LUT construction. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_smask_transfer_function_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine unsupported_smask_transfer_function_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest malformed RawImage sampler gates

These gates were run after changing public image sampling helpers to return transparent samples for short decoded buffers or unsupported channel counts instead of opaque black. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine sample_short_buffer_is_transparent_not_black --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine sample_unsupported_channels_is_transparent_not_black --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_painter --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest image color-space channel metadata fail-closed gates

These gates were run after `images/decoder.rs` stopped defaulting malformed DeviceN image color-space component counts to one channel and stopped defaulting ICCBased image channel metadata to RGB when no supported ICC channel metadata is available. DeviceN image normalization and tint conversion now require a resolved non-empty component-name array within the supported component limit, and ICCBased images require reader-backed ICC channel metadata before byte-count normalization or conversion. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine build_raw_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine device_n_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine icc_based_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tint --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <image ColorSpace channel source/report files>` | 0 |

## Latest image ICC/Indexed channel metadata fail-closed gates

These gates were run after `render/cmm.rs` stopped defaulting missing ICC profile `/N` metadata to RGB and stopped clamping invalid ICC component counts into the supported range, and after `images/decoder.rs` stopped clamping Indexed DeviceN base component arrays and Indexed ICCBased base `/N` values before palette sizing/conversion. ICCBased image and named-color conversion now require explicit supported profile channel metadata, Indexed DeviceN bases require a strict non-empty component-name array, and Indexed ICCBased bases require a valid `/N` value. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine icc_based_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine channel_clamp --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cmm --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest inline image metadata fail-closed gates

These gates were run after inline images stopped turning missing or invalid `/Width` and `/Height` into silent no-ops, after non-mask inline images stopped defaulting/clamping `/BitsPerComponent`, and after non-mask inline images stopped defaulting missing or non-name `/ColorSpace` metadata to `/DeviceGray`. Valid inline image masks without `/ColorSpace` or `/BitsPerComponent` remain accepted through the stencil-mask path. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_inline_image_missing_width --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_inline_image_zero_height --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_inline_image_missing_bits_per_component --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_inline_image_unsupported_bits_per_component --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_inline_image_missing_color_space --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_inline_image_mask_without_color_space --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest image locator metadata fail-closed gates

These gates were run after image inventory and extraction references stopped fabricating decodable image metadata. `images/locator.rs` now requires image XObject and inline-image references to carry positive dimensions, strict non-mask BPC and ColorSpace metadata except terminal JPX sample-depth/color-model cases, strict image-mask BPC where present, boolean ImageMask metadata, and name/name-array filters. Malformed locator metadata now fails typed before image inventory, extraction, or decode-capability reports can expose guessed dimensions, bpc, color space, or filter chains. Valid image masks still use the PDF stencil defaults. These tests stayed local and synthetic or used tiny repository fixtures; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine locator --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test integration image_locator --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test integration extract_image_bytes --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test integration locator_captures --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_capability_report --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <image locator source/report files>` | 0 |

## Latest SVG/PS regional image metadata fail-closed gates

These gates were run after SVG/PS regional Image XObject classification and sink reference construction started requiring a resolved stream dictionary with positive dimensions, strict non-mask BPC and ColorSpace metadata, strict stencil-mask BPC, and well-formed stencil `/Decode` before native regional embedding. Resource-named Image XObject color spaces that pass classification are now decoded through the resolved color-space object rather than a default sink reference. SVG/PS regional inline-image classification mirrors the active renderer's positive dimension, non-mask BPC, non-mask ColorSpace, filter, DecodeParms, and stencil `/Decode` preflight before bounded native image output. Malformed regional image metadata now routes through the existing whole-page typed raster/refusal boundary instead of defaulting dimensions, bpc, color space, or mask polarity. The full regional-vector test target also exposed and corrected one stale synthetic dense-text fixture to wrap `Tj` operations in a valid `BT`/`ET` text object. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback classifier_malformed_inline_image_metadata_stays_whole_page --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback inline_image --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_image_xobject_metadata_triggers_whole_page --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback image_xobject --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback image_xobject_resource_color_space --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <SVG/PS regional image metadata source/report files>` | 0 |

## Latest SVG/PS named-paint fail-closed gates

These gates were run after SVG/PS vector-output classification started preflighting named path/text/stencil-mask paint before native vector replay. Resource-named device paint remains vector-safe only when the resolved device family and supplied components are exact and finite; unresolved, unsupported, invalid, or incomplete named paint now routes through the whole-page typed raster/refusal boundary before SVG/PS sinks can use the generic named-color black fallback. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine named_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <SVG/PS named-paint source/report files>` | 0 |

## Latest device-family component exactness gates

These gates were run after `ColorSpaceHandler::try_from_components` stopped padding, truncating, or defaulting malformed device-family component vectors. Image SMask `/Matte`, luminosity SMask `/BC`, and SVG/PS named-paint preflight now share the same exact finite `/DeviceGray`, `/DeviceRGB`, and `/DeviceCMYK` component boundary. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine try_from_components_requires_exact_finite_device_components --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_matte_rgb --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_backdrop_color --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <device-family component exactness source/report files>` | 0 |

## Latest inline image sequence fail-closed gates

These gates were run after retained display-list capture, immediate page dispatch, and SVG/PS vector classification stopped accepting malformed `BI`/`ID`/data/`EI` ordering. Malformed sequences now reject data without `ID`, `ID` without `BI`, dangling `BI`, pending `ID`, missing payload bytes, and payloads without closing `EI`, instead of fabricating empty-parameter inline images, silently dropping the pending inline image, or painting unterminated payloads. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_inline_image_sequence_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_inline_image_sequence_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image_triggers_whole_page --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <inline-image-sequence source/report files>` | 0 |

## Latest image XObject metadata fail-closed gates

These gates were run after Image XObjects stopped defaulting missing/invalid `/Width`, `/Height`, `/BitsPerComponent`, and `/ColorSpace` metadata before cache-key construction and decode planning. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_image_xobject_missing_width --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_image_xobject_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest explicit image Mask and image SMask metadata fail-closed gates

These gates were run after referenced explicit `/Mask` streams stopped inheriting source-image dimensions or defaulting non-stencil bpc/color-space metadata, and after image SMask streams stopped inheriting dimensions or defaulting `/BitsPerComponent` and `/ColorSpace`. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine image_xobject_explicit_mask_missing --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_smask_missing --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest image Filter metadata fail-closed gates

These gates were run after active inline image, Image XObject, and referenced explicit-mask `/Filter` metadata stopped filtering malformed array entries before bpc/color-space planning. Filter metadata now requires a single name or an all-name array before terminal-codec metadata defaults, cache-key construction, or mask decode planning. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine malformed_filter --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest image boolean metadata fail-closed gates

These gates were run after active inline images, Image XObjects, and referenced explicit Mask streams stopped defaulting malformed present `/ImageMask` or `/Interpolate` metadata to `false`. Missing booleans still use PDF defaults; present malformed booleans now fail typed before bpc/color-space planning, decode-cache identity, mask decode planning, or scaled-image fast-path selection. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine malformed_booleans --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_explicit_mask_malformed_boolean_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest transparency group color-space policy gates

These gates were run after active Form XObject transparency groups and SMask `/G` groups began validating `/Group /CS` before rendering. Direct and resource-resolved DeviceGray, DeviceRGB/sRGB, and DeviceCMYK group spaces are accepted; malformed arrays, missing resources, unresolved references, and non-device group spaces outside explicitly proven inert subsets now fail typed before Form group setup or soft-mask group rendering. Luminosity SMask `/BC` conversion uses the resolved group device policy for exact component counts. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine transparency_group --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest active calibrated/Indexed/ICCBased inert Form-group color-space gates

These gates were run after active Form XObject transparency groups stopped refusing every non-device `/Group /CS` before considering whether the group color space is semantically inert. Opaque, normal-blend Form streams with well-formed CalGray, CalRGB, Lab, structurally exact Indexed, or structurally valid ICCBased group color spaces now render through an explicit non-device-inert policy that has no SMask `/BC` component interpretation. Alpha-bearing Forms, malformed calibrated dictionaries, malformed Indexed palettes, malformed ICCBased profile references or `/N` metadata, missing resources, SMask `/G` groups that are not the alpha/no-`/BC` subset, and backdrop-observable non-device group semantics still fail typed instead of synthesizing default calibrated/ICC conversion or silently treating the group as DeviceRGB. Runtime capability reporting now names calibrated, Indexed, and ICCBased opaque group Forms. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_calibrated --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_indexed --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_iccbased --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_color_space --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine indexed_group_color_space --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine iccbased_group_color_space --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest active alpha SMask calibrated inert group color-space gates

These gates were run after `/S /Alpha` SMask `/G` groups stopped refusing well-formed calibrated, structurally exact Indexed, or structurally valid ICCBased group `/CS` values when no `/BC` backdrop is present. In that subset the rendered mask consumes only the child group's alpha channel, so CalGray/CalRGB/Lab/Indexed/ICCBased color conversion is not observable and the group uses the same non-device-inert policy as opaque active Form groups. The calibrated positive test also builds a retained display list, asserts it remains fully supported, and compares retained replay pixels to active rendering. Later local slices added `/S /Luminosity` `/BC` conversion for well-formed calibrated/Indexed/ICCBased and Separation/DeviceN group spaces with resolved color-space cache identity. Alpha SMask groups with `/BC`, malformed calibrated dictionaries, malformed Indexed palettes, malformed tint transforms, malformed ICCBased profile references or `/N` metadata, missing resources, and richer non-device group spaces still fail typed. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_smask_calibrated --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_smask_indexed --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_smask_iccbased --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_group --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_backdrop_color --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- <scoped transparency group files>` | 0 |

## Latest transparency group flag fail-closed gates

These gates were run after active Form XObject transparency groups stopped defaulting malformed present `/I` or `/K` flags to `false`. Missing flags still use PDF defaults; present malformed flags now fail typed before backdrop selection, knockout setup, offscreen allocation, or child rendering. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine transparency_group_malformed_flags_return_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest color-key image Mask range fail-closed gates

These gates were run after color-key `/Mask` arrays stopped filtering malformed entries, ignoring extra ranges, clamping or rounding malformed sample bounds, swapping reversed ranges, or applying masks to padded source pixels. Active color-key masks now require exact finite integer source-sample ranges for the resolved DeviceGray/RGB component count. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine color_key --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <color-key source/report files>` | 0 |

## Latest uncolored tiling-pattern color fail-closed gates

These gates were run after PaintType 2 tiling-pattern paints stopped inferring `/DeviceRGB` for unexpected active pattern-color component counts. They stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine uncolored_tiling_pattern_invalid_component_count --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tiling_pattern --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest Type 3 path-collector fail-closed gates

These gates were run after `Type3PathCollector` stopped synthesizing path geometry from malformed Type 3 CharProc path and graphics-state operands. The path-only collector now refuses missing/non-finite/wrong-count operands for path construction, colors, `cm`, `w`, `J`, `j`, `M`, `d`, `d0`, and `d1`; refuses `l/c/v/y` without an active current point; and rejects malformed dash arrays instead of using default line state, identity transforms, solid dash state, ignored path segments, or extra color operands. Malformed Type 3 clipping CharProcs now surface through the existing typed renderer refusal path. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine type3_path_collector_rejects --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type3_clipping_charproc_malformed_line_width_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type3 --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <Type 3 source/report files>` | 0 |

## Latest active graphics-state operand and restore-sequence fail-closed gates

These gates were run after non-text active graphics-state operators stopped substituting identity/default operands for malformed input, and after stream-local `Q` restore underflow was wired into SVG/PS vector-output classification. `graphics_state_operand_refusal` now gates retained `GraphicsStateDescriptor` compilation, display-list capture, and immediate page rendering for `q`, `Q`, `cm`, `w`, `J`, `j`, `M`, `d`, `ri`, `i`, direct device colors, `CS`, `cs`, `SC/SCN/sc/scn`, and `gs`; display-list capture, immediate page rendering, and vector-output classification reject `Q` when it would restore past the saved graphics-state depth active for the current stream/Form. Malformed operands and restore underflow now record typed unsupported diagnostics, fatal render refusals, or whole-page vector fallback before `GraphicsState::process` can apply default line state, identity transforms, solid dash state, default colors, default color spaces, missing ExtGState names, or a tolerant empty-stack restore. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine graphics_state_descriptor_rejects_malformed_state_operands --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_graphics_state_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_graphics_state_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_operands_stay_whole_page_vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine graphics_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine graphics_state_descriptor --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <graphics-state source/report files>` | 0 |

## Latest active text operand and object-sequence fail-closed gates

These gates were run after active text-state and text-showing operators stopped substituting empty strings, zero advances, default font sizes, identity text matrices, ignored `TJ` items, or malformed `BT`/`ET` boundaries. `text_operand_refusal` now gates retained text/state descriptor compilation, display-list text capture, and immediate page text rendering for `BT`, `ET`, `Tf`, `Td`, `TD`, `Tm`, `T*`, `Tc`, `Tw`, `Tz`, `TL`, `Tr`, `Ts`, `Tj`, `TJ`, `'`, and `"`; display-list capture, immediate/packed page rendering, and SVG/PS vector-output classification also reject nested `BT`, unmatched `ET`, showing/positioning operators outside a text object, and unterminated `BT`. Malformed text operands and object sequences now record typed unsupported diagnostics, fatal render refusals, or whole-page vector fallback before `RetainedTextOp::from_content_operation`, text painting, or `GraphicsState::process` can fabricate defaults. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine text_operand_validator_rejects_malformed_text_operands --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_text_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_text_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine text_is_replayable_as_native_operation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_operands_stay_whole_page_vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine text --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <text-operand source/report files>` | 0 |

## Latest vector TJ helper-local gates

This focused gate was run after `render/vector_fallback.rs` stopped filtering
malformed direct `TJ` array members in the lower-level string collection helper.
Valid string chunks are still collected, integer and real kerning adjustments
remain skipped, and non-string/nonnumber entries now return no native vector
text collection. The test stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication was
used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine tj_string_operand_collection_rejects_invalid_items --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest visual text byte-sequence fail-closed gates

These gates were run after visual text decoding stopped padding an odd trailing byte in two-byte Type0/CID text strings into a synthetic character code. `render/text_decode.rs` now exposes checked decode helpers for active raster/SVG/PS consumers; active page rendering records a typed fatal render refusal, and SVG/PS vector output records the same typed unsupported error, before a malformed string can synthesize a CID/GID, glyph advance, or native vector outline. The older compatibility wrapper remains for callers that still request best-effort decoded glyph vectors, but current renderer output paths use the checked boundary. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine type0_visual_text --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_type0_odd_text_code_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest active marked-content operand and sequence fail-closed gates

These gates were run after active marked-content and compatibility-section operators stopped substituting empty tags, empty property lists, ignored compatibility operands, default visible state, leaked end-of-stream visibility, or unbalanced compatibility-section no-ops for malformed input. `marked_content_operand_refusal` now gates retained state descriptor compilation, display-list state capture, and immediate page optional-content visibility mutation for `BMC`, `BDC`, `EMC`, `MP`, `DP`, `BX`, and `EX`; display-list finish, relative immediate dispatch boundaries, packed replay, and SVG/PS vector classification now reject unmatched `EMC`, cross-stream `EMC`, unmatched `EX`, and unterminated `BMC`/`BDC`/`BX` boundaries before `GraphicsStateDescriptor::compile_with_resources`, `DisplayListBuilder::push_state_op`, `RenderState::push_optional_content_visibility`, or native vector replay can fabricate defaults. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine marked_content_operand_validator_rejects_malformed_operands --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_marked_content_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine unterminated_marked_content_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine unbalanced_compatibility_section_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_marked_content_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine unterminated_marked_content_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine unbalanced_compatibility_section_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_operands_stay_whole_page_vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine marked_content --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <marked-content source/report files>` | 0 |

## Latest active path operand fail-closed gates

These gates were run after active page path construction, paint, and clip operators stopped dropping malformed construction input, painting or clipping with operand-bearing path operators, treating empty clipping paths as no-ops, leaving dangling `W/W*` clip requests supported at stream end, or synthesizing missing current points through lower-level `Path` helper behavior. `path_operand_refusal` now gates display-list path capture and immediate page path mutation for `m`, `l`, `c`, `v`, `y`, `h`, `re`, `S`, `s`, `f/F`, `f*`, `B`, `B*`, `b`, `b*`, `n`, `W`, and `W*`; valid empty or move-only clipping paths install an explicit empty clip, while repeated or unterminated `W/W*` sequences record typed unsupported diagnostics, fatal render refusals, or whole-page vector fallback before `DisplayListBuilder`, `RenderState::dispatch_all`, retained clip replay, or SVG/PS classification can fabricate default path/clip state. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine path_operand_validator_rejects_malformed_operands --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_path_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_path_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine empty_path_clip_installs_empty_clip --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_operands_stay_whole_page_vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine path --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <path-operand source/report files>` | 0 |

## Latest active resource invocation operand fail-closed gates

These gates were run after active XObject and shading resource invocation operators stopped silently accepting missing, wrong-type, or extra operands before retained, immediate, or SVG/PS regional-vector resource dispatch. `resource_invocation_operand_refusal` now gates display-list resource capture, immediate page dispatch, and vector-output classification for `Do` and `sh`; malformed operands now record typed unsupported diagnostics, fatal render refusals, or whole-page raster classification before `DisplayListBuilder::push_native_xobject`, `DisplayListBuilder::push_native_shading`, `RenderState::handle_do`, `RenderState::handle_sh`, or regional vector replay can no-op, use empty names, or drop extra operands. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine resource_invocation_operand_validator_rejects_malformed_operands --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_resource_invocation_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_resource_invocation_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_resource_invocation_stays_whole_page_vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine xobject_resource --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine named_shading --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <resource-invocation source/report files>` | 0 |

## Latest SVG/PS vector operand preflight gates

These gates were run after SVG/PS vector-output classification started reusing the active operand refusal family before native sink replay. `classify_ops_for_vector_output` now refuses malformed non-text graphics-state, text, marked-content, page-path, and resource-invocation operands before `GraphicsState::process`, native path/text replay, visibility state, or resource dispatch can default malformed input. The vector path routes those malformed operands to the existing typed raster/refusal boundary instead of emitting malformed native SVG/PostScript output. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_operands_stay_whole_page_vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <vector-operand source/report files>` | 0 |

## Latest Type 3 glyph metric operand fail-closed gates

These gates were run after generic `d0`/`d1` metric handling stopped accepting malformed glyph displacement or bounding-box operands before retained descriptor compilation, display-list capture, immediate page dispatch, or SVG/PS vector-output classification, and after present malformed Type 3 `/FontMatrix` values stopped defaulting to the 0.001 scale. Malformed metrics now fail typed, mark the retained list unsupported, or route through the existing typed raster/refusal boundary instead of producing zero-width/zero-bbox descriptors or malformed native vector output; malformed present FontMatrix values record typed renderer errors before glyph CTM construction. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine type3_glyph_metric_operand_validator_rejects_malformed_operands --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_type3_glyph_metric_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_type3_glyph_metric_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_operands_stay_whole_page_vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_type3_font_matrix_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <Type 3 metric source/report files>` | 0 |

## Latest Type 3 FontBBox cache-bound exactness gates

These gates were run after Type 3 font-level `/FontBBox` stopped accepting overlong arrays for bounded rendered-glyph cache surfaces. Exact four-number finite FontBBox arrays may still seed bounded cache allocation when a CharProc-level `d1` bbox is unavailable; malformed or overlong font-level bounds decline cached bounded-surface admission and fall back to full CharProc rendering. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine type3_font_bbox_requires_exact_numeric_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type3 --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest color-glyph fallback gates

These gates were run after failed COLR/CPAL, SVG-in-OpenType, raster color-glyph decode, COLRv1 temporary-surface, and Porter-Duff source-surface paths stopped reporting color paint success, after SVG-in-OpenType geometry parsing stopped treating malformed present numeric geometry attributes as absent defaults or clamping negative `stroke-width` to zero, and after SVG-in-OpenType paint metadata stopped clamping out-of-range `rgb()` components, percentages, or opacity values. The color-glyph helper now declines color fill on bounded failures so the ordinary glyph outline fallback can paint in compatibility mode, and the SVG static-subset parser preserves omitted optional defaults while refusing malformed present geometry, invalid stroke width, and invalid paint metadata. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine color_glyph_surface_denial_declines_color_fill_for_outline_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_offscreen_surface_fails_closed_over_budget --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine svg_static_subset --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <color-glyph fallback source/report files>` | 0 |
| `cargo test -p wellfriendpdf-engine svg_static_subset --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine color_glyph --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/color_glyph.rs docs/renderer/final-local-implementation-closure.md docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-fallback-closure-report.md docs/renderer/final-fallback-inventory.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md` | 0 |

## Latest malformed Form Matrix gates

These gates were run after present malformed Form `/Matrix` entries stopped defaulting to identity. Absent `/Matrix` still uses the PDF identity default; malformed present arrays now fail typed for active Form XObjects, annotation appearances, and soft-mask groups, and SVG/PS vector Form preflight rejects them before native vector replay. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_form_xobject_matrix_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine extract_form_matrix --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_form_matrix_extractor_rejects_malformed_present_matrix --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine form_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest malformed Form BBox gates

These gates were run after Form XObjects, annotation appearance Forms, and SMask `/G` Forms stopped treating missing or malformed `/BBox` metadata as an optional unclipped Form replay. Active Form BBox parsing now requires exactly four finite numbers before stream decode, retained-plan cache admission, offscreen allocation, clipping, or child rendering. SVG/PS vector Form preflight uses the same exact BBox requirement before native regional Form replay. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine bbox --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine form_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_form --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest annotation Rect fail-closed gates

These gates were run after visible annotations stopped silently skipping missing or malformed `/Rect` values. Active annotation rendering now requires an exact four-number finite `/Rect` after print/form/optional-content exclusion and before appearance selection, synthesized appearance construction, cache lookup, or appearance rendering. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine annotation_missing_or_malformed_rect_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest annotation visibility-flag fail-closed gates

These gates were run after annotation rendering stopped defaulting malformed present `/F` flags to zero before display/print visibility decisions. Missing `/F` still uses the PDF default flags; present non-integer `/F` now fails typed before visibility filtering. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine annotation_malformed_flags_return_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest annotation Subtype fail-closed gates

These gates were run after visible annotation rendering stopped silently skipping missing or malformed `/Subtype` values. Active annotation rendering now requires `/Subtype` to be present and name-valued before FormRenderPolicy, optional-content checks, appearance selection, cache lookup, or synthetic appearance construction. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine annotation_missing_or_malformed_subtype_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest annotation appearance-state fail-closed gates

These gates were run after selected annotation appearance state dictionaries stopped treating malformed present `/AS` metadata as `/Off` or first-state fallback. Present `/AS` must be a name, selected named states must exist, and selected state objects must resolve to streams before appearance cache admission or rendering. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine annotation_malformed_appearance_state_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest synthesized annotation geometry fail-closed gates

These gates were run after synthetic appearances for text markup, line, and ink annotations stopped falling back from malformed geometry to full-Rect marks, skipped strokes, or filtered coordinate arrays. When no author-provided appearance is selected, `/QuadPoints`, `/L`, and `/InkList` are now preflighted before synthetic appearance construction. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine annotation_malformed --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest synthesized annotation paint metadata fail-closed gates

These gates were run after synthesized annotations stopped defaulting or clamping present malformed `/C` and `/CA` paint metadata. Missing optional color/opacity still uses the explicit synthesis defaults; present malformed, wrong-cardinality, nonnumeric, non-finite, or out-of-range values now fail typed before synthetic appearance construction. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine synthesized_annotation_malformed_color_or_opacity_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest synthesized annotation helper-local gates

These gates were run after the lower-level no-author-appearance synthesis
helpers stopped relying solely on caller preflight for annotation paint,
geometry, and alignment metadata. Direct helper use now rejects malformed
present `/C`, `/CA`, `/QuadPoints`, `/L`, `/InkList`, and FreeText `/Q`
metadata locally instead of clamping/defaulting paint values, widening malformed
text-markup geometry to the annotation Rect, truncating line coordinates,
filtering bad coordinates, emitting a partial ink path prefix, or defaulting
FreeText alignment. These tests
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine synthesized_annotation_helpers_reject_malformed_metadata_locally --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine synthesized_annotation_malformed_color_or_opacity_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine markup_annotation_malformed_quadpoints_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine line_annotation_malformed_l_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ink_annotation_malformed_inklist_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine freetext_annotation_malformed_alignment_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest synthesized FreeText alignment fail-closed gates

These gates were run after synthesized FreeText appearances stopped defaulting malformed present `/Q` alignment metadata to left alignment. Missing `/Q` still uses the established left-alignment default; present malformed `/Q` now fails typed before text layout. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine freetext_annotation_malformed_alignment_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest synthesized FreeText contents fail-closed gates

These gates were run after synthesized FreeText appearances stopped filtering malformed present `/Contents` display metadata before text layout. Missing `/Contents` still uses the established no-synthesis behavior; malformed present contents now fail typed before appearance construction. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine freetext_annotation_malformed --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest widget MK color metadata fail-closed gates

These gates were run after synthesized Widget appearances stopped defaulting or clamping present malformed `/MK /BG` and `/MK /BC` color metadata. Missing optional MK colors still use the established widget synthesis defaults; malformed MK dictionaries, wrong-cardinality, nonnumeric, non-finite, or out-of-range present values now fail typed before synthetic widget appearance construction. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine widget_malformed_mk_colors_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine widget --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest widget default-appearance metadata fail-closed gates

These gates were run after synthesized Widget appearances stopped silently defaulting or clamping malformed present `/DA` text-style operators. Missing `/DA` still uses established widget synthesis defaults and `Tf` size `0` remains the existing auto-size path; present non-string or unparsable `/DA`, malformed `Tf`, and malformed, wrong-cardinality, nonnumeric, non-finite, or out-of-range `g`/`rg`/`k` operands now fail typed before synthetic widget appearance construction. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine widget_malformed_default_appearance_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine widget --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest synthesized widget helper-local gates

These gates were run after the lower-level synthesized Widget helper stopped
relying solely on caller preflight for `/DA` and `/MK` metadata. Direct helper
use now declines malformed present default-appearance color/style operands and
malformed or out-of-range MK color arrays instead of defaulting style or
clamping widget chrome colors. These tests stayed local and synthetic; no PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine synthesized_widget_helpers_reject_malformed_metadata_locally --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine widget_malformed --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest widget appearance-state metadata fail-closed gates

These gates were run after synthesized Widget appearances stopped ignoring malformed present `/AS` metadata before checkbox/radio state synthesis. Missing `/AS` still uses established value-driven synthesis behavior; present non-name `/AS` now fails typed before `/V` can drive checked-state fallback. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine widget_malformed_synthesized_appearance_state_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine widget --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest widget field-type metadata fail-closed gates

These gates were run after synthesized Widget appearances stopped silently skipping widgets with malformed present inherited `/FT` field-type metadata. Missing `/FT` still leaves no field kind to synthesize; present malformed `/FT` now fails typed before field-kind dispatch. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine widget_malformed_field_type_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine widget --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest widget field-integer metadata fail-closed gates

These gates were run after synthesized Widget appearances stopped defaulting malformed present `/Ff`, local `/Q`, or AcroForm `/Q` values before field kind and alignment synthesis. Missing entries still use established PDF defaults; present malformed integer metadata now fails typed before synthesis. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine widget_malformed_field_integer_metadata_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine widget --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest widget AcroForm boolean metadata fail-closed gates

These gates were run after synthesized Widget appearances stopped defaulting malformed present AcroForm `/NeedAppearances` values before checked-state synthesis. Missing `/NeedAppearances` still uses the PDF default `false`; present malformed boolean metadata now fails typed before synthesis. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine widget_malformed_need_appearances_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine widget --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest widget display-value metadata fail-closed gates

These gates were run after synthesized Widget appearances stopped filtering malformed present `/V`, `/Opt`, and `/MK /CA` display metadata before text, choice, button, checkbox, or radio synthesis. Missing optional values still use the established no-synthesis/default behavior; malformed present values now fail typed before appearance construction. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine widget_malformed_display_text_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine widget --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest malformed pattern metadata/Matrix gates

These gates were run after active tiling patterns stopped filtering malformed `/BBox` arrays, defaulting missing or malformed `/XStep`/`/YStep` to zero, defaulting missing `/PaintType` to 1, and ignoring missing, malformed, or unsupported `/TilingType`. Tiling-pattern streams now require an exact four-number `/BBox`, finite numeric `/XStep` and `/YStep`, `/PaintType` 1 or 2, and `/TilingType` 1, 2, or 3 before exact tile replay. These gates also cover present malformed tiling and shading pattern `/Matrix` entries no longer defaulting to identity. Absent pattern `/Matrix` values still use the PDF identity default; present non-array, wrong-length, nonnumeric, or non-finite matrices now fail typed for active tiling and shading pattern rendering, and SVG/PS vector-output preflight rejects malformed present pattern matrices before native replay. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_tiling_pattern_geometry_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tiling_pattern --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_shading_pattern_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_pattern_matrix_extractor_rejects_malformed_present_matrix --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine pattern --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <pattern Matrix source/report files>` | 0 |

## Latest optional-content visibility fail-closed gates

These gates were run after active optional-content visibility evaluation stopped defaulting malformed OCG/OCMD metadata to visible. Immediate and retained marked-content visibility plus object-level XObject, annotation, shading, and pattern `/OC` checks now use strict visibility evaluation. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine optional_content_malformed_membership_policy_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine optional_content --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest ExtGState render-state metadata fail-closed gates

These gates were run after display-list capture plus immediate and packed ExtGState replay stopped accepting malformed present render-state values before graphics-state mutation. Missing optional ExtGState entries still use established graphics-state defaults; valid `/BM` arrays can still select the first supported named mode, but present malformed alpha, line style, miter, blend, rendering intent, overprint, flatness, smoothness, dash, or font metadata now fails typed instead of keeping default state, clamping out-of-range values, or filtering malformed arrays. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine malformed_extgstate_blend_mode_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_extgstate_state_metadata_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine extgstate --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest ExtGState strict-helper gates

These gates were run after renderer-owned ExtGState replay paths were routed through `GraphicsState::try_apply_ext_g_state` before graphics-state mutation. The strict helper preserves valid ExtGState metadata but refuses malformed present render-state metadata without mutating the caller's state; vector classification keeps no-op/safe ExtGState dictionaries vector-safe while malformed, alpha/blend, overprint, SMask, or transfer cases stay on the explicit whole-page fallback boundary. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine try_apply_ext_gstate --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ext_gstate --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine noop_blend_and_transfer_arrays_remain_vector_safe --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest image Decode exact-array fail-closed gates

These gates were run after raw image decoding stopped filtering malformed `/Decode` arrays. Present non-mask image `/Decode` arrays now require exactly two finite numeric values per active image channel; short, overlong, nonnumeric, or non-finite entries fail typed before pixel normalization instead of being ignored or truncated. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine build_raw_image_rejects_malformed_decode_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_malformed_decode_array_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine decode_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS regional image boolean metadata fail-closed gates

These gates were run after SVG/PS vector-output classification and regional Image XObject replay stopped defaulting malformed present `/ImageMask`, `/IM`, `/Interpolate`, or `/I` metadata to `false`. The classifier now routes malformed regional image boolean metadata to the typed whole-page raster/refusal boundary, and the SVG/PS regional emitters revalidate those booleans before XObject regional decode. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine classifier_malformed --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest decoder boolean parameter fail-closed gates

These gates were run after lower-level image normalization and CCITT DecodeParms parsing stopped accepting name-valued boolean spellings. Raw image normalization now rejects present non-boolean `/ImageMask` or `/IM` before choosing stencil-mask channels, and CCITT `/BlackIs1`, `/EncodedByteAlign`, `/EndOfLine`, and `/EndOfBlock` parameters now require actual PDF booleans instead of accepting `/true` or `/false` names. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine build_raw_image_rejects_malformed_image_mask_boolean --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ccitt_params_reject_name_valued_booleans --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine build_raw_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ccitt_params --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest high-quality retained display-list refusal gates

These gates were run after unsupported retained display-list replay stopped returning `None` or using raw/immediate dispatch in page, display-list, tile, band, and progressive paths. Compatibility and high-quality paths now return typed `UnsupportedFeature` refusals for unsupported retained replay, and progressive rendering does not publish a fallback tile event. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_display_list_page_helpers_refuse_unsupported_list --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_display_list_tile_refuses_unsupported_list --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_progressive_refuses_unsupported_display_list_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality font-substitution refusal gates

These gates were run after legacy high-quality `RenderState` construction started deriving `ExactnessPolicy::HighQualityExact` from `RenderMode::HighQuality`. Compatibility rendering still records deterministic bundled substitution events, but high-quality rendering now accepts valid caller-registered or deterministic-system replacements and refuses generic bundled replacement when no valid embedded/document/registered/system font program exists. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_render_refuses_generic_bundled_font_substitution --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest font-substitution descriptor-routing gates

These gates were run after FontDescriptor Symbolic/Nonsymbolic, Italic, ForceBold, and FontWeight hints started feeding deterministic replacement provider selection. Compatibility rendering now routes unknown non-Standard14 symbolic descriptor fonts through the symbolic coverage lookup and reports `CoverageOnly`; high-quality/exact rendering still refuses generic bundled replacement. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used. The scoped diff check emitted only Git LF-to-CRLF normalization warnings for touched files.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine symbolic_descriptor --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/fonts/provider.rs crates/engine/src/render/page_renderer.rs docs/renderer/final-local-implementation-closure.md docs/renderer/final-fallback-inventory.md docs/renderer/final-fallback-closure-report.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md docs/renderer/complete-algorithm-and-method-inventory.md` | 0 |

## Latest font-substitution selection-reason gates

These gates were run after public font-substitution events gained a provider selection reason in addition to the existing high-level substitution reason. Compatibility reports now expose why the selected deterministic face was chosen, including `symbolic_flag` for unknown non-Standard14 symbolic descriptor fonts. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used. The scoped diff check emitted only Git LF-to-CRLF normalization warnings for touched files.

The same gate group was refreshed after core, C API, and server wrapper shape tests began asserting the prompt-required high-level substitution `reason` and metric-compatibility `metric_posture` fields as public JSON strings.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine font_substitution_report --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine symbolic_descriptor --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-capi capi_render_font_substitution_report_outputs_owned_json --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server render_contract --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/font_substitution_report.rs crates/engine/src/fonts/provider.rs crates/engine/src/render/page_renderer.rs crates/engine/src/engine.rs crates/wellfriendpdf-capi/src/lib.rs crates/server/tests/server_integration.rs docs/renderer/final-local-implementation-closure.md docs/renderer/final-fallback-inventory.md docs/renderer/final-fallback-closure-report.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md docs/renderer/complete-algorithm-and-method-inventory.md` | 0 |

## Latest DeviceN/Separation tint-transform cache gates

These gates were run after Separation and DeviceN tint-transform outputs started using a bounded thread-local LRU cache keyed by source document hash/length, resolved tint-function fingerprint, exact input count, and tint input bits. Invalid or oversized transform outputs are not admitted, DeviceN no longer allocates a copied tint vector before evaluation, and `ColorReport` now exposes tint-transform cache metrics and configured entry/byte caps. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine reports_tint_transform_cache_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tint_transform_cache --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tint_transform --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/colorspace.rs crates/engine/src/color_report.rs crates/engine/src/lib.rs docs/renderer/final-local-implementation-closure.md docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md` | 0 |

## Latest clip-DAG intern-table pruning gates

These gates were run after the persistent clip DAG started pruning unreferenced interned nodes automatically when insertion exceeds its default max-node cap. Full/Empty flyweights and live externally referenced save/restore/composite nodes remain valid, and `ClipDagStats` now reports max nodes, pruning passes, pruned nodes, and over-capacity-live-node cases. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine clip_dag --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine content_path_clip_replay_reuses_transformed_clip_node_cache --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/clip_dag.rs docs/renderer/final-local-implementation-closure.md docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md` | 0 |

## Latest clip-DAG render-telemetry gates

These gates were run after render-state return started preserving the active `ClipDagStats` snapshot in `RenderDocumentCache` and `RenderContractTelemetryReport` began serializing those counters. The same stats are now exposed in CLI `render-corpus` per-file `cache_after_file.clip_dag` JSON. The exposed telemetry covers interned nodes, node cap, lookups, hits, nodes created, pruning passes, pruned nodes, live-node over-capacity cases, and approximate bytes for the just-completed render. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine render_contract_telemetry_report_exposes_cache_counters --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-cli clip_dag_stats_json_exposes_render_telemetry_shape --bin wellfriendpdf --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1 -- -D warnings` | 0 |

## Latest CLI annotation-appearance cache-telemetry gates

These gates were run after CLI `render-corpus` started carrying the retained annotation appearance program entry count and byte-accounted cache stats in per-file `cache_after_file.annotation_appearance_program_entries` and `cache_after_file.annotation_appearance_program_cache` JSON. The cache stats use the same retained byte/accounting shape already used for display-list, path clip-node, glyph-atlas, Form, and tiling program cache telemetry. These tests stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-cli render_artifact_cache_stats_json_exposes_retained_byte_accounting_shape --bin wellfriendpdf --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1 -- -D warnings` | 0 |

## Latest source-scoped artifact invalidation gates

These gates were run after `RenderDocumentCache` started retaining canonical object-number cache markers for mapped source IDs, binding-safe invalidation plans started emitting `source_cache_markers`, the plan applier learned to derive omitted marker strings from object number/generation, and source-bound artifact caches were pruned during source invalidation. The pruned artifact classes are decoded image, scaled image, SMask group, mesh-shading, Form program, tiling-pattern program, and annotation appearance program caches. The path remains local and cache-focused; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine render_invalidation_plan_json_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_invalidation_plan_json_registers_source_cache_markers --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine editing_transaction_apply_with_render_invalidation_exposes_dirty_tiles --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tile_scoped_source_invalidation_prunes_source_scoped_artifact_caches --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation_registers_source_cache_markers --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest nested write-set invalidation gates

These gates were run after `RenderDocumentCache` started retaining object number/generation to source identity mappings and the cache-plan applier started scanning direct plans plus SDK/server/report envelopes for nested render write-set fields. Known PDF refs and `object-*`/`stream-*` refs are merged into mapped source IDs and source-cache marker entries before exact raster/source-artifact invalidation; unknown nested refs conservatively reset the document cache. The path stayed local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest progressive checked-finish binding gates

These gates were run after public progressive finish surfaces stopped collapsing
incomplete tile assembly into legacy `None`/`undefined`/generic incomplete
results. C ABI, Python, WASM, and server finish paths now call
`ProgressiveRenderJob::finish_checked()`; .NET and Java inherit the checked
native error through their existing C ABI finish wrappers. The path stayed local
and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_finish_before_complete_uses_checked_error --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server progressive_finish_before_complete_returns_error --test progressive_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-capi -p wellfriendpdf-py -p wellfriendpdf-wasm -p wellfriendpdf-server --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-capi -p wellfriendpdf-py -p wellfriendpdf-wasm -p wellfriendpdf-server --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest raw Image XObject source-window gates

These gates were run after unfiltered 1/2/4/8/16-bit raw Image XObjects gained a guarded
axis-aligned source-window path. The planner now classifies empty-filter image
XObjects as raw plans, refuses unsupported raw window shapes explicitly, the
decoder crops only the requested bounded source rows before raw image
construction, and page rendering admits the windowed result under a cache key
that includes the planned source region. Filtered lossless, masked,
reduced-resolution, postprocessed, inline raw, and non-axis-aligned partial
decode remain outside this increment. The path stayed local and synthetic; no
PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine raw_unfiltered_xobject_plan_uses_windowed_source_region_when_clipped --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine raw_window_plan_refuses_filtered_or_unsafe_shapes --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine raw_xobject_window_decode_uses_requested_source_region --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine scheduled_raw_window_decode_uses_planned_source_region_cache_key --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest inline raw image source-window gates

These gates were run after guarded axis-aligned unfiltered 1/2/4/8/16-bit inline images
started using the same source-window planning boundary as raw Image XObjects.
The inline decoder now crops only the requested bounded rows from the inline
byte payload before raw image construction, and page rendering routes the
planned inline raw window through the decode scheduler before painting with the
existing source-region CTM mapping. Filtered, masked,
reduced-resolution, postprocessed, and non-axis-aligned raw windows remain
outside this increment. The path stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine raw_unfiltered_inline_plan_uses_windowed_source_region_when_clipped --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_raw_window_decode_uses_requested_source_region --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine scheduled_inline_raw_window_decode_uses_planned_source_region --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine raw_window_plan_refuses_filtered_or_unsafe_shapes --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest image-decode runtime capability gates

These gates were run after `runtime.rs` stopped reporting the older CCITT-only
XObject exception and started disclosing the guarded CCITT XObject/inline and
raw unfiltered 1/2/4/8/16-bit XObject/inline source-window paths. The same
reason string continues to mark JBIG2/filtered-lossless paths full-decode-only,
guarded JPX downscale as target-resolution native reduction, JPEG/JPX
region/progressive unavailable, and raw filtered or incompatible SMask,
unsupported-explicit-mask, reduction, unsupported-postprocessing, or non-axis
windows incomplete while preserving the guarded raw/CCITT crop-aligned SMask
source-window exception. The path stayed local and synthetic;
no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest progressive deterministic schedule gates

These gates were run after adding synthetic deterministic schedule checks for
progressive rendering and display-list band/cache replay. The progressive tests
render the same page through full-page rendering, single-tile progressive
quanta, batched progressive quanta, a viewport-hinted progressive order, and a
scoped two-worker progressive job matrix with different quantum schedules; they
assert byte-identical final RGBA output, stable unhinted publication
identity/order, and a changed hinted identity/order. The band/cache tests
stitch several cold band heights, warm-cache band crops from a cached full-page
raster, alternating band ownership across two separate worker-local
`RenderDocumentCache` instances, and a scoped two-worker concurrent band render
back to the full display-list render. This is focused source evidence only;
corpus determinism, external viewer runtime determinism, broader server/external
runtime matrices, hardware parity, and final visual parity remain deferred.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_quantum_schedules_finish_to_identical_pixels --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_concurrent_worker_jobs_finish_to_identical_pixels --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine display_list_band_schedules_match_full_page_across_cache_states --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine display_list_band_schedules_match_full_page_across_worker_local_caches --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine display_list_band_schedules_match_full_page_across_concurrent_worker_local_caches --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest progressive render-context revision gates

These gates were run after progressive publication identities gained the active
render-contract fingerprint and a caller-visible render-context revision API.
`ProgressiveRenderJob::revise_render_context` accepts replacement
render-contract and optional-content visibility fingerprints, cancels
session-owned in-flight work, clears retained surfaces/cache, bumps scheduler
generation, reports a bounded obsolete publication, and leaves stale tile
rejection on the existing source-session acceptance path. Server, C, Python,
WASM/TypeScript, .NET, and Java source wrappers expose the same JSON report
surface. This is source and compile evidence only; external viewer runtime
matrices remain incomplete. No PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine revise_render_context_obsoletes_prior_publications --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-server progressive_revise_render_context_obsoletes_publications --test progressive_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-server --all-targets --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-capi --all-targets --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --all-targets --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-py --all-targets --jobs 1` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf/WellfriendPdf.csproj --nologo` | 0 |
| `javac --release 22 -d .work/final-universal-renderer-implementation/java-check bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1` | 0 |
| `cargo fmt --all --check` | 0 |
| scoped `git diff --check` over render-context source/binding/docs files | 0, CRLF normalization warnings only |

## Latest contract and progressive execution cancellation binding gates

These gates were run after normal and report-returning contract PNG plus
caller-owned buffer render paths stopped being limited to `CancelToken::none()`
source wiring, and after progressive viewer-queue execution plus adjacent-page
prefetch execution gained binding-level cancellation entrypoints instead of only
their non-cancellable compatibility wrappers. The C ABI and public header now expose
`WellfriendRenderCancellation` plus cancel/status ownership APIs and
cancellable contract JSON/handle PNG and caller-owned buffer entrypoints,
including font-substitution and render-telemetry report variants, plus
cancellable progressive queue and prefetch execution entrypoints. Python and
WASM expose `RenderCancellation` classes and cancellable contract render
methods, including Python bytearray caller-owned surface methods, and
cancellable progressive queue/prefetch execution methods; WASM TypeScript
declarations now include typed-object report and caller-owned buffer report
cancellation variants. .NET exposes
`RenderCancellation` with `IsCancelled` for contract PNG/caller-buffer/report
methods plus `CancellationToken` convenience overloads for plain and
report-returning PNG/caller-owned buffer methods, viewer-queue execution, and
adjacent-page prefetch execution. Java exposes a
`RenderCancellation` AutoCloseable with `isCancelled()` and matching contract
PNG/direct-buffer/report overloads plus progressive queue/prefetch execution
overloads. The Python, WASM, .NET, and Java READMEs document the cancellation
surface and its remaining runtime-matrix boundary. This is source and compile
evidence only; Python cancellable PNG/report methods now release the GIL, but
Python bytearray/progressive-job GIL behavior, single-thread JS hosts, .NET/Java
native runtime smokes, browser/Node WASM smokes, and broader external binding
runtime matrices remain incomplete.
The path stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-capi capi_render_cancellation_reports_status --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-capi -p wellfriendpdf-py -p wellfriendpdf-wasm --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-capi -p wellfriendpdf-py -p wellfriendpdf-wasm --all-targets --jobs 1 -- -D warnings` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf/WellfriendPdf.csproj -v:minimal -maxcpucount:1 -p:BuildInParallel=false -p:UseSharedCompilation=false` | 0 |
| `javac --enable-preview --release 25 -d .work/final-universal-renderer-implementation/java-classes bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java` | 0 |

Additional WASM typed-object report and caller-owned buffer report cancellation
variants were added after the base gate above. The addendum gates stayed local
and source-only:

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-wasm --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-wasm --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-wasm --target wasm32-unknown-unknown --lib --jobs 1 -- -D warnings` | 0 |

Additional C ABI progressive queue/prefetch cancellation tests were added after
the same base gate. They verify pre-cancelled
`WellfriendRenderCancellation` handles are observed by the exported
viewer-queue and adjacent-page prefetch execution functions:

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_execute_viewer_queue_json_observes_cancellation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_execute_adjacent_page_prefetch_observes_cancellation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-capi --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-capi --all-targets --jobs 1 -- -D warnings` | 0 |

Additional Python GIL-detach plumbing for cancellable contract PNG/report
methods was added after the same base gate. Bytearray caller-owned surface
methods and mutable progressive job methods remain GIL-bound:

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-py --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-py --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest runtime fallback policy matrix gates

These gates were run after `RuntimeCapabilityReport::renderer_fallback_policies`
was expanded from 12 to 20 machine-readable rows, matching the documented
runtime fallback taxonomy. The new rows cover already fail-closed active pattern
paint metadata, tiling-pattern required metadata, named-color metadata, active
shading metadata, inline image metadata, Image XObject metadata, ExtGState
render-state metadata, and active content operand/sequence refusals. They are
typed-refusal rows and do not increase the material-degradation or
canonical-immediate fallback counts. A later source slice updated the retained
full-page, retained tile, and progressive retained tile rows to typed-refusal
policy entries, so the runtime matrix no longer counts them as
canonical-immediate publications. The path stayed local and synthetic; no PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS regional order and bounds gates

These gates were run after SVG and PostScript regional raster emission gained
inert diagnostics that identify regional Image XObject or inline-image segments
and record their device-space bounds. The focused Image XObject and inline-image
tests assert the synthetic `40.000 30.000 20.000 10.000` bounds and verify each
regional raster marker remains between the surrounding blue and green vector
paths. This is
order/bounds source evidence only; it does not close all SVG/PS regional
fallback, seam, transparency, pattern, gradient, or corpus visual-parity work.
The path stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback svg_output_inline_image_uses_regional_embed --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback ps_output_inline_image_uses_regional_colorimage --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback svg_output_image_xobject_resource_color_space_uses_regional_embed --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback ps_output_image_xobject_resource_color_space_uses_regional_colorimage --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS linear shading Domain gates

These gates were run after SVG/PostScript simple-shading regionalization was
extended from unit shading `/Domain` only to finite, non-degenerate shading
domains whose full interval is contained in an `N=1` Type 2 function domain.
The vector fallback now samples the Type 2 function at the shading-domain
endpoints before emitting the existing native SVG gradient or PostScript
`shfill` form. Non-linear functions, malformed function-domain cases,
mesh shadings, transparency, advanced patterns, and corpus visual parity remain
incomplete.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine non_unit --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine shading_domain_outside_function_domain_is_regional_vector_output --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS shading BBox gates

These gates were run after accepted simple vector shadings started carrying
valid `/BBox` metadata through the vector fallback model. Direct SVG shading
output now registers a native clipPath for the transformed shading BBox, and
PostScript wraps native `shfill` with a BBox clip path; shading-pattern replay
also intersects its path clip with the shading BBox. Degenerate or malformed
BBoxes still route away from native vector replay.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine bounded_axial_shading --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS Type 2 function-array shading gates

These gates were run after simple vector-shading classification began accepting
`/Function` arrays whose elements are all linear Type 2 component functions.
The component outputs are sampled at the accepted shading-domain endpoints and
concatenated into the existing SVG gradient or PostScript `shfill` color path.
Malformed arrays, empty arrays, non-linear elements, oversized outputs, clipped
mesh shadings, transparency, advanced patterns,
and corpus visual parity remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine function_array_axial_shading --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS shading-pattern stencil-mask gates

These gates were run after SVG and PostScript regional fallback began accepting
Image XObject and inline-image stencil masks painted with a vector-safe simple
shading pattern. SVG emits a decoded stencil `<mask>` and paints the existing
native shading-pattern gradient through that mask while preserving the active
clip and any accepted shading `/BBox` clip. PostScript vectorizes bounded
painted stencil cells into a native clip path and replays the same shading
pattern through `shfill`; larger masks remain conservative under the
`MAX_PATTERN_STENCIL_CLIP_RECTS` cap.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine shading_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS colored/uncolored tiling-pattern gates

These gates were run after the SVG/PostScript regional fallback subset began
accepting simple colored and uncolored `/PatternType 1` tiling-pattern path paints. The
accepted subset requires a complete lossless pattern stream, exact
non-degenerate `/BBox`, positive finite `/XStep` and `/YStep`, `/PaintType` 1 or 2,
`/TilingType` 1 through 3, a finite `/Matrix`, and a tile program that
classifies as pure vector without nested resource or pattern paint. `/PaintType`
2 additionally requires finite caller DeviceGray, DeviceRGB, or DeviceCMYK
components and rejects tile streams that set paint color themselves. SVG and
PostScript now clip the painted path, compute the visible tile range in pattern
space, apply per-tile BBox clips, and replay each tile stream as native vector
paths under `MAX_VECTOR_TILING_PATTERN_CELLS`. Nested pattern-paint,
color-setting uncolored, negative-step, over-cap, malformed, transparency-bearing, and
advanced tiling-pattern cases remain conservative.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine colored_tiling_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine uncolored_tiling_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS tiling-pattern stencil-mask gates

These gates were run after the SVG/PostScript regional fallback subset began
accepting Image XObject and inline-image stencil masks painted with supported
colored and uncolored `/PatternType 1` tiling patterns. SVG emits a decoded
stencil `<mask>` and replays bounded visible tiles as native SVG paths through
that mask while preserving the active clip. PostScript vectorizes bounded
painted stencil cells into a native clip path and reuses the existing tiling
tile replay, so these covered masks avoid `imagemask`, `colorimage`, and
`shfill`. The same finite geometry, complete stream, caller-color, and
visible-cell caps from the path-paint tiling subset apply.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine tiling_pattern_inline_image_mask --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tiling_pattern_image_xobject_mask --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS tiling-pattern glyph-outline text gates

These gates were run after the SVG/PostScript regional fallback subset began
accepting glyph-outline text fill/stroke painted with supported colored and
uncolored `/PatternType 1` tiling patterns. SVG and PostScript now try the
bounded tiling-pattern clip replay for text outlines before falling back to the
existing shading-pattern text replay. The covered text path remains
glyph-outline replay with the same complete stream, finite geometry,
caller-color, and visible-cell caps used by path paints and stencil masks.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine tiling_pattern_text --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS resource-bearing tiling-pattern gates

These gates were run after the SVG/PostScript regional fallback subset began
accepting resource-bearing `/PatternType 1` tile cells whose nested resource
programs are independently vector-safe. The classifier now threads the active
Form stack and viewport scale into tiling-pattern checks, permits scoped
regional Image XObjects, Form XObjects, named shadings, and complete inline
images inside colored tiling cells, and still rejects nested `/Pattern` color
spaces, color-setting uncolored tiles, over-cap tile counts, transparency, and
unsupported resource programs. SVG and PostScript replay each tile with the
pattern's merged resources and install the nested regional allow-lists returned
by the same scoped classifier used for Form replay.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine resource_tile --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS affine radial-shading gates

These gates were run after the SVG/PostScript regional fallback subset began
accepting finite/invertible affine-transformed radial shadings instead of only
uniform-circle CTMs. The classifier now treats simple Type 3 radial shadings
as vector-safe when the active shading transform is finite and invertible. SVG
keeps uniform-circle cases on the existing device-coordinate radial-gradient
path and emits `gradientTransform` for elliptical/sheared radial output.
PostScript keeps uniform-circle cases on the existing device-coordinate
`shfill` path and emits affine `concat` plus source-space Type 3 `shfill` for
elliptical/sheared transforms. Non-linear, mesh, malformed, degenerate, and
otherwise unsupported shading dictionaries remain conservative fallback cases.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine nonuniform_radial --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading_with_nonuniform_ctm_is_regional_vector_output --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS nonzero-start-radius radial-shading gates

These gates were run after the SVG/PostScript regional fallback subset first
widened simple Type 3 radial shadings beyond the prior point-source radial
subset by accepting positive start radii in increasing-radius dictionaries. SVG
emits that PDF start radius as the `fr` focal-radius attribute on native
`<radialGradient>` output, while PostScript carries both radii through native
Type 3 `shfill`. Degenerate, negative-radius, non-linear, mesh, malformed, and
otherwise unsupported shading dictionaries remain conservative fallback cases.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nonzero_start_radius --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading_with_nonzero_start_radius_is_regional_vector_output --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS reversed-radii radial-shading gates

These gates were run after the SVG/PostScript regional fallback subset began
accepting simple Type 3 radial shadings where both radii are non-negative and
unequal, including PDF dictionaries whose start radius is larger than the end
radius. SVG normalizes those reversed-radii dictionaries by swapping the focal
and end circles plus color stops so native `<radialGradient>` output remains
representable; PostScript keeps the original PDF Type 3 coordinates and radii
in native `shfill`. Equal-radius, negative-radius, non-linear, mesh, malformed,
and otherwise unsupported shading dictionaries remain conservative fallback
cases.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine reversed_radii --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading_with_reversed_radii_is_regional_vector_output --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS DeviceN shading-color gates

These gates were run after the SVG/PostScript vector-shading color conversion
path began routing simple `/DeviceN` color spaces through the same exact finite
named-color resolver already used for vector-safe `/Separation` shadings. The
covered fixture is a resource-named, one-component DeviceN axial shading with a
valid Type 2 tint transform to `/DeviceRGB`; malformed or unsupported DeviceN
spaces remain fail-closed through the named-color resolver. SVG emits the
resolved RGB stops in native `<linearGradient>` output, and PostScript emits
them in native Type 2 `shfill`.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine devicen_axial --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS clipped Type 2 function-domain gates

These gates were run after the SVG/PostScript regional fallback subset began
accepting simple Type 2 shading domains that extend outside the Type 2
function domain when the output can be represented as bounded native RGB
stops. `VectorShading` now carries explicit stops at shading endpoints and at
in-range function-domain boundaries. SVG emits those stops directly in native
gradients; PostScript keeps Type 2 functions for ordinary two-stop output and
uses a LanguageLevel 3 Type 3 stitching function for multi-stop clipped-domain
output. Non-linear functions, malformed domains, malformed arrays, oversized
component output, mesh shadings, and unsupported color spaces remain
conservative fallback cases.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine clipped_function_domain --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading_domain_outside_function_domain_is_regional_vector_output --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS Type 3 stitching-function shading gates

These gates were run after the SVG/PostScript regional fallback subset began
accepting continuous Type 3 stitching functions made from linear Type 2
subfunctions. The classifier now flattens supported stitching functions into
bounded RGB stop lists, inserts stops at stitching bounds, validates
per-segment encode/domain metadata, requires consistent component counts, and
rejects discontinuous stitching functions instead of emitting inexact native
gradients. SVG emits the flattened stops directly; PostScript emits native
`shfill` with a LanguageLevel 3 Type 3 function built from the flattened stops.
Malformed bounds, malformed encode arrays, non-linear subfunctions,
discontinuous segments, mesh shadings, and unsupported color spaces remain
conservative fallback cases.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine stitching_function --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine stitching_function_axial_shading --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest PostScript exact Type 3 stitching-function gates

These gates were run after the PostScript regional fallback subset began
carrying exact Type 3 stitching-function sidecars instead of relying on
flattened RGB stops when a stitch is discontinuous. The new path is
target-specific: SVG and conservative classification still reject
discontinuous stitching; PostScript admits direct DeviceRGB/DeviceGray,
unit-domain Type 3 functions whose Type 2 segments can be serialized exactly
as RGB, and direct DeviceRGB Type 2 function arrays whose component functions
share the same unit-domain exponent. If a
function requires exact PostScript output but the exact sidecar cannot be
built, classification now fails closed instead of approximating sampled stops.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine discontinuous_stitching_function_axial_shading --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine discontinuous_device_gray_stitching_function_axial_shading --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_device_rgb_type2_function_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_rgb_shading_function_preserves_exact_type3_stitching --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_rgb_shading_function_preserves_exact --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest SVG/PS ICCBased Gray/RGB and native-CMYK shading-color gates

These gates were run after the SVG/PostScript vector-shading color conversion
path began accepting resource-named Gray and RGB `/ICCBased` axial and radial
shadings whose ICC profile declares `/N 1` or `/N 3` and whose sampled Type 2
function components exactly match that profile channel count. The path reuses
the existing strict CMM scalar conversion helper. Default portable qcms remains
limited to Gray/RGB profile shapes, while four-channel `/N 4` ICCBased vector
shadings are admitted only when the `native-cmm-lcms2` backend is active. SVG
emits the converted RGB stops in native `<linearGradient>` or
`<radialGradient>` output, and PostScript emits them in native Type 2/Type 3
`shfill`.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine vector_iccbased_shading_options_keep_qcms_for_gray_rgb_and_gate_cmyk --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine resource_iccbased_ --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine native_lcms2_iccbased_cmyk_radial_shading --test regional_vector_fallback --features native-cmm-lcms2 --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-engine font_signature_decode_dedup_feature_envelopes --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest portable qcms CMYK admission gates

These gates were run after the portable qcms ICC transform path began rejecting
four-component CMYK ICC profiles before qcms transform construction. CMYK ICC
profile-to-sRGB remains available through the native LittleCMS feature path;
the default portable backend now fails closed for CMYK profiles that the qcms
debug build cannot safely transform, instead of panicking during transform
construction.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine portable_qcms_rejects_unsupported_cmyk_profile_without_panic --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cmm --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine iccbased --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS vector-shading component exactness gates

These gates were run after the SVG/PostScript vector-shading color conversion
path stopped accepting overlong sampled function outputs for `/DeviceGray`,
`/DeviceRGB`, `/DeviceCMYK`, `/CalGray`, `/CalRGB`, and `/Lab`. Supported
simple shadings now require exact finite component arity before native
`<linearGradient>`, `<radialGradient>`, or `shfill` replay. Overlong or
non-finite vector-shading outputs stay on the explicit whole-page SVG/PS raster
fallback boundary instead of being truncated into native output.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine overlong_vector_shading_components_stay_whole_page_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS Indexed constant shading-color gates

These gates were run after active shading validation, named-color resolution,
and SVG/PostScript vector-shading conversion gained strict `/Indexed` handling
for DeviceGray, DeviceRGB, DeviceCMYK, CalGray, CalRGB, Lab, Separation,
DeviceN, default-qcms Gray/RGB ICCBased, and native-LittleCMS CMYK ICCBased
base palettes. Valid constant resource-named Indexed axial/radial shadings now
stay native in SVG/PostScript, while malformed lookup tables, unsupported
remaining Indexed bases, non-integer sampled indexes, overlong component
vectors, and non-constant Indexed color transitions remain on explicit
typed-refusal or whole-page fallback paths instead of falling through to
generic black/default color conversion or inaccurate smooth native gradients.
Focused integration coverage includes native SVG/PostScript output for
constant Indexed DeviceRGB, DeviceGray, DeviceCMYK, CalRGB, Lab, Separation,
default-qcms ICCBased RGB, and native-LittleCMS ICCBased CMYK shadings;
resolver unit coverage includes CalGray and DeviceN Indexed bases.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine indexed --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine resource_indexed_ --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine resource_indexed_iccbased_cmyk --test regional_vector_fallback --features native-cmm-lcms2 --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS JPX internal alpha classifier gates

These gates were run after the SVG/PostScript regional Image XObject and
inline-image classifiers became target-aware for terminal JPX images with
declared internal soft-mask data. PostScript now defers `/SMaskInData 1` and
`/SMaskInData 2` JPX Image XObjects and inline images to decoded regional alpha
classification instead of rejecting them purely from metadata: opaque, fully
transparent, and binary 0/255 decoded alpha can stay regional, while fractional
decoded alpha falls back through whole-page raster output. SVG keeps the same
declared internal-alpha JPX Image XObjects and inline images eligible for
regional PNG embedding. This stayed local and synthetic; it does not claim
broader JPX ROI/region/progressive decode closure or generic image
soft-mask regionalization.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine internal_alpha_defers --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest SVG/PS tiling-pattern Matrix gates

These gates were run after focused coverage was added for finite invertible
affine `/Matrix` metadata on the already-supported simple colored tiling-pattern
SVG/PostScript subset. The existing replay path applies the pattern matrix
before the active page CTM, uses that matrix-bearing CTM for visible-cell
enumeration, and replays translated tiles with per-tile BBox clips. Synthetic
SVG/PostScript fixtures for `/Matrix [2 0 0 1 15 0]` now stay native, preserve
red/blue tile paints, and assert scaled/translated tile geometry in device
space instead of identity-matrix coordinates. This does not claim closure for
nested pattern paint, transparency-bearing cells, over-cap tiling,
malformed/non-invertible matrices, or resource programs outside the vector-safe
image/Form/shading/inline-image subset.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine matrix_tiling_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine tiling_pattern --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest SVG/PS named-color tint-transform shading gates

These gates were run after direct `/Separation` and one-colorant `/DeviceN`
vector-shading conversion gained a fail-closed tint-transform preflight. Native
SVG gradients are still admitted only when the named-color tint transform is a
linear Type 2 function with `N 1` and the alternate is a direct device family
handled by the RGB stop path. PostScript `shfill` now has a separate exact
named-color path for one-component Separation or DeviceN shadings: when both the
source shading tint function and the color-space tint transform can be
serialized as finite PostScript Type 2/Type 3 functions, the sink emits a native
`[/Separation ...]` or single-colorant `[/DeviceN ...]` color space instead of
flattening endpoint RGB stops. Multi-input DeviceN calculator transforms,
unsupported alternates, malformed functions, unresolved transform metadata, and
SVG nonlinear tint transforms stay on the whole-page fallback boundary.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_tint --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine multi_input --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine resource_separation_axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine resource_devicen_axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest SVG/PS DCT Separation inline-image gates

These gates were run after regional inline-image classification admitted DCT
terminal inline images whose resolved resource color space is an
opaque-paintable `/Separation` tint space. The decoded JPEG still has to match
the declared single tint channel before raw-image construction. Multi-component
`/DeviceN` terminal-codec inline images and non-DCT tint-space terminal-codec
inline images remain whole-page fallback cases.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine separation_dct --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image_resource_separation --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image_resource_devicen --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS DCT one-colorant DeviceN inline-image gates

These gates were run after regional inline-image classification admitted DCT
terminal inline images whose resolved resource color space is an
opaque-paintable one-colorant `/DeviceN` tint space. The decoded JPEG still has
to match the declared single tint channel before raw-image construction.
Multi-component `/DeviceN` terminal-codec inline images and non-DCT tint-space
terminal-codec inline images remain whole-page fallback cases.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine devicen_single_dct --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image_resource_devicen --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image_resource_separation --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest SVG/PS monochrome terminal inline-image classifier gates

These gates were run after regional inline-image classification was tightened
for monochrome terminal codecs. CCITT/JBIG2 terminal inline images whose
declared or resolved device color space is `DeviceRGB` or `DeviceCMYK` now stay
on the whole-page fallback boundary because those decoders hand back
monochrome/grayscale samples and do not apply RGB/CMYK color semantics after
decode. `DeviceGray` CCITT/JBIG2 terminal inline images remain regionally
eligible. DCT/JPX terminal image classification and non-terminal RGB/CMYK
inline-image classification are unchanged.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine monochrome_terminal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest monochrome terminal image decode guard gates

These gates were run after `images/decoder.rs` gained a fail-closed guard for
CCITT/JBIG2 terminal image decoders. Full inline decode, Image XObject decode,
inline CCITT source-window decode, and Image XObject CCITT source-window decode
now reject non-`DeviceGray`/`G` color spaces before returning monochrome samples
to the renderer. `page_renderer.rs` threads the effective inline or Image
XObject color-space override into the CCITT window wrappers. This does not add
palette, calibrated, ICC, tint-space, ROI/reduction, or progressive color
conversion for those monochrome terminal codecs; non-gray declarations now fail
typed instead of producing color-semantically wrong pixels.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine monochrome_terminal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ccitt --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine jbig2 --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest JPX dimension-consistency gates

These gates were run after every JPX terminal decode path was routed through a
checked finalizer in `images/decoder.rs`. The finalizer now rejects decoded JPX
dimensions that disagree with the PDF inline image or Image XObject dictionary
`/Width` and `/Height` before `/SMaskInData` handling or downstream
paint/cache behavior. This mirrors the existing DCT dimension-consistency guard
and does not add JPX region, reduction, tile/component, progressive, or corpus
verification coverage.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine jpx_finish --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine jpx --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest JPX SMaskInData metadata gates

These gates were run after the JPX finalizer started validating
`/SMaskInData` metadata. Non-integer values, values outside `0`, `1`, and `2`,
and `/SMaskInData 1` or `2` declarations without decoded alpha samples now
fail typed before downstream paint/cache behavior. This does not add JPX region,
reduction, tile/component, progressive, cancellation-inside-codec, or corpus
verification coverage.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine jpx_smask_in_data --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine jpx --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest JPX finalizer invariant gates

These gates were run after the JPX finalizer started validating normalized
`RawImage` output. Non-8-bit samples, zero-channel output, decoded byte-length
mismatches, and decoded dimensions over the image decode budget now fail typed
before `/SMaskInData` handling or downstream paint/cache behavior. This does
not add JPX ROI/region, tile/component, progressive, cancellation-inside-codec,
or corpus verification coverage.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine jpx_finish --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine jpx_smask_in_data --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine jpx --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest JPX target-resolution reduction gates

These gates were run after guarded JPX downscale plans started using
`hayro-jpeg2000` target-resolution decode. The reduced decode path validates
original JPX codestream dimensions against the PDF image dictionary before
accepting smaller decoded output, and it remains disabled when image masks or
full-image postprocessing need the original sample grid. This does not add JPX
ROI/region, tile/component, progressive, cancellation-inside-codec, or corpus
verification coverage.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine jpx_downscale --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine reduced_jpx_finish --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render::image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_image_decode_session_reports_full_decode_required --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest JPEG/JPX asymmetric reduction gates

These gates were run after the image decode planner stopped selecting a native
DCT/JPEG or JPX reduced-resolution tier unless the reduced output covers both
requested target axes. Asymmetric downscale requests that would undersample one
axis now keep full-resolution decode identity and exact-mode unsupported
reporting instead of caching unsafe reduced pixels. This does not add JPEG/JPX
ROI, region, progressive, tile/component, cancellation-inside-codec, corpus, or
benchmark coverage.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest DCT inline ColorSpace gates

These gates were run after the legacy inline `DCTDecode` path was routed
through the checked DCT finalizer. Inline JPEG component counts now have to
match the declared PDF image `ColorSpace` before output reaches downstream
paint/cache behavior. This does not add new JPEG ROI, progressive, or corpus
coverage.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine dct_inline --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine dct_finish --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest image normalized-length gates

These gates were run after `build_raw_image` started validating normalized
sample-buffer length before decode arrays and color conversion. The DCT CMYK
finalizer shortcut also now applies decode-budget and exact decoded-length
checks before conversion. The attempted `decoded-length` filter matched 0 tests
and is not counted as validation evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine dct_finish --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine build_raw_image_rejects_mismatched_buffers --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine dct_inline --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest CMYK converter length gates

These gates were run after DeviceCMYK and fallback ICCBased-CMYK image
conversion entry points started validating exact input byte length before
calling the raw chunked CMYK-to-RGB helper. This does not add new
color-management, ICC, or corpus validation.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine color_space_converter_rejects_short_device_cmyk_input --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cmyk --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest color-glyph JPEG invariant gates

These gates were run after decoded JPEG color-glyph payloads started validating
pixel cap, channel count, and exact byte length before `RawImage` return or
CMYK conversion. This does not add new color-glyph payload formats or corpus
font validation.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine color_glyph_jpeg_invariants --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine color_glyph --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped renderer/report files>` | 0 |

## Latest utility image sample-length gates

These gates were run after utility image ingestion started validating exact
decoded sample length before JPEG CMYK conversion, PNG
grayscale/grayscale-alpha/RGB/RGBA expansion, and rendered-page
RGBA/gray-to-RGB conversion. This does not add new image codec support, corpus
validation, or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine utility_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine utilities::tests --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped utility/report files>` | 0 |

## Latest color-glyph PNG invariant gates

These gates were run after decoded PNG color-glyph payloads started validating
pixel cap, channel count, and exact byte length before `RawImage` return.
Grayscale-alpha samples are checked as two-channel decoded data before RGBA
expansion. This does not add new color-glyph payload formats or corpus font
validation.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine color_glyph_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine color_glyph --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped color-glyph/report files>` | 0 |

## Latest color-glyph bitmap exactness gates

These gates were run after supported BGRA, gray8, gray subbyte, and mono
bitmap color-glyph decoders started requiring exact payload byte lengths before
expansion. Overlong payloads now fail typed instead of being sliced to the
computed image span. This does not add new color-glyph payload formats or
corpus font validation.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine color_glyph --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped color-glyph/report files>` | 0 |

## Latest sub-byte row exactness gates

These gates were run after decoded 1/2/4 bpc image data started requiring an
exact match to the computed packed row-byte length before sample unpacking.
Overlong decoded buffers now fail typed before decode arrays, color conversion,
or `RawImage` return. This does not add new codec support, corpus validation,
or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine subbyte_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine build_raw_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped image-decoder/report files>` | 0 |

## Latest raw window source-length gates

These gates were run after unfiltered 1/2/4/8/16-bit raw source-window decode started
requiring the full source image stream length to match dimensions and channels
exactly before cropping the requested window. This does not add filtered
source-window support, corpus validation, or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine raw_window --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped image-decoder/report files>` | 0 |

## Latest raw window bit-depth gates

These gates were run after guarded axis-aligned unfiltered raw Image XObjects
and inline images gained bounded source-window decoding for 1/2/4/8/16 bpc
samples. The decoder crops byte-aligned 8/16 bpc source rows directly and
re-packs 1/2/4 bpc source samples into destination-local packed rows before the
existing raw image normalizer expands to 8-bit output. Filtered,
unsupported-explicit-mask/SMask, reduced-resolution, postprocessed, and non-axis-aligned raw windows remain
outside this increment. The path stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine raw_window --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine raw_unfiltered_subbyte_xobject_plan_uses_windowed_source_region_when_clipped --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest raw ImageMask source-window gates

These gates were run after guarded axis-aligned unfiltered 1 bpc `/ImageMask
true` Image XObjects and inline images gained bounded raw source-window
planning. XObject masks pass through the checked raw-window crop and dictionary
driven one-channel normalization before stencil expansion; inline masks reuse
the 1 bpc DeviceGray raw-window path before `/Decode` polarity and fill color
are applied. Filtered masks, unsupported `/Mask`, `/SMask`, reduced-resolution,
postprocessed, and non-axis-aligned raw windows remain outside this increment.
The path stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine raw_window --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest raw explicit Mask source-window gates

These gates were run after guarded axis-aligned unfiltered raw Image XObjects
with color-key `/Mask` arrays or same-sized referenced 1 bpc stencil `/Mask`
streams gained crop-aligned source-window planning. Referenced stencil masks are
preflighted before the exemption is selected, decoded under their own
source-region cache key, and combined with the cropped main image through the
existing explicit-mask combiner. Filtered masks, non-stencil or mismatched
referenced masks, `/SMask`, reduced-resolution, postprocessed, and
non-axis-aligned raw windows remain outside this increment. The path stayed
local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS,
deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine raw_unfiltered_explicit_mask_plan_uses_windowed_source_region_when_crop_aligned --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_explicit_stencil_mask_uses_shared_raw_source_window_when_clipped --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_color_key_mask_keeps_raw_source_window_when_clipped --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_explicit_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest raw SMask source-window gates

These gates were run after guarded axis-aligned unfiltered raw Image XObjects
with same-sized unfiltered grayscale `/SMask` streams gained crop-aligned
source-window planning. The main image keeps the raw `SubRect` cache identity,
the soft mask is decoded through the same source window before alpha combine,
and incompatible filtered, mismatched, non-grayscale, reduced, or postprocessed
SMask cases remain outside this increment. The path stayed local and synthetic;
no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine raw_unfiltered_smask_plan_uses_windowed_source_region_when_crop_aligned --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_smask_keeps_raw_source_window_when_clipped --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_source_window --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_filter_names --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest CCITT SMask source-window gates

These gates were run after guarded monochrome CCITT Image XObjects with
same-sized unfiltered grayscale `/SMask` streams gained crop-aligned
source-window planning. The main image keeps the CCITT `SubRect` cache
identity, the soft mask is decoded through the same raw source window before
alpha combine, and the guard refuses unknown filter wrappers, inline images,
image masks, SMask images, non-gray declarations, reduced-output, non-axis, or
otherwise postprocessed SMask cases. Runtime capability reporting now discloses
the raw/CCITT SMask source-window exception instead of the previous broad
postprocessing-window-unavailable boundary. The path stayed local and synthetic; no
PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine ccitt_smask_plan_uses_windowed_source_region_when_crop_aligned --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_smask_shared_source_window_allows_guarded_ccitt_main_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask_loader_crops_source_window_for_ccitt_main_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest Indexed lookup exactness gates

These gates were run after image decoding and active/named color resolution
started requiring `/Indexed` lookup tables to match `(hival + 1) *
base_channels` exactly. Overlong lookup data now fails typed instead of being
prefix-truncated before palette conversion. This does not add new color-space
support, corpus validation, or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine indexed_overlong --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine overlong_lookup --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine indexed --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped Indexed/report files>` | 0 |

## Latest Indexed arity exactness gates

These gates were run after image decoding and active/named color resolution
started requiring `/Indexed` color-space arrays to contain exactly four
entries. Overlong arrays now fail typed instead of carrying ignored trailing
metadata after the lookup table. This does not add new color-space support,
corpus validation, or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine overlong_color_space_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine indexed --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped Indexed/report files>` | 0 |

## Latest calibrated arity exactness gates

These gates were run after image decoding, active/named color resolution, and
SVG/PS vector-shading validation started requiring CalGray, CalRGB, and Lab
color-space arrays to contain exactly two entries before parameter parsing.
Overlong arrays now fail typed or stay on the conservative vector fallback
boundary instead of carrying ignored trailing metadata. This does not add new
CMM support, corpus validation, or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine overlong_color_space_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine calrgb --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine calibrated --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine inline_image --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped calibrated/report files>` | 0 |

## Latest sbix parser fail-closed gates

These gates were run after color-glyph sbix parsing started validating strike
headers, strike-offset tables, glyph-offset tables, record bounds, origin
fields, duplicate records, and pixels-per-em values before payload selection.
Malformed sbix metadata now fails typed instead of being coerced to defaults,
converted to an unsupported dummy payload, or treated as absent glyph data.
This does not add new sbix payload formats, corpus font validation, or
benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine sbix_parser --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine color_glyph --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped sbix/report files>` | 0 |

## Latest PDF function input-arity gates

These gates were run after PDF function evaluation started validating Type 0,
Type 2, Type 3, and Type 4 required shapes before evaluation and rejecting
missing or non-finite caller inputs. Sampled Type 0 functions no longer fill
omitted dimensions with zero, Type 2/3 functions no longer evaluate with
absent input or missing required domain/encode metadata, and Type 4 calculator
functions execute only the strict `/Domain` input count. This does not add new
function types, corpus validation, or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine function --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped function/report files>` | 0 |

## Latest Type 0/2/3 function evaluator-local gates

These gates were run after the Type 0, Type 2, and Type 3 function evaluators
stopped relying solely on the public function-shape validator for required local
shape metadata. Direct evaluator use now returns no function output for
malformed local shape fields instead of filtering smaller sample grids,
stitched segments, or component vectors, or borrowing default mappings for
malformed present fields. These tests stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine type0_evaluator_rejects_malformed_local_shape_fields --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type0_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine evaluator_rejects_malformed_local_shape_fields --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type2_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type3_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest SMask transfer backdrop gates

These gates were run after alpha soft-mask `/BC` default-alpha probing started
validating `/TR` function shape before testing `TR(0)`. Unsupported,
malformed, or empty-output transfer functions now fail typed instead of being
interpreted as transparent-at-zero. This does not add new transfer-function
types, transparency-group semantics, corpus validation, or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine smask_transfer --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_smask_backdrop_transfer --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine smask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- <scoped SMask transfer/report files>` | 0 |

## Latest SVG shading BBox clip fail-closed gates

These gates were run after SVG native shading and shading-pattern replay stopped
turning an unflattenable accepted shading `/BBox` into an empty clip path.
Accepted shading BBox clip materialization now fails typed if the device-space
clip cannot be flattened, so SVG replay does not silently drop direct shading,
shading-pattern, or pattern-painted stencil output. This does not add native
mesh, nonlinear shading, transparency, pattern, corpus validation, or benchmark
evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine shading_bbox_clip_refuses_unflattenable_geometry --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine svg_output_bounded_axial_shading_clips_native_gradient --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine svg_output_simple_shading_pattern_fill_uses_clipped_gradient --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest overprint CMYK state exactness gates

These gates were run after active page and retained display-list
overprint-preview CMYK dispatch stopped padding malformed internal `DeviceCMYK`
color state with zero components. The helpers now require exactly four finite
components before the overprint-preview row path is used; valid CMYK preview
rendering remains unchanged. This does not add
separation-preserving print output, native CMM proofing, corpus validation, or
benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine device_cmyk_components_requires_exact_finite_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine display_list_cmyk_components_require_exact_finite_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine overprint_preview_policy_controls_device_cmyk_paint --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ext_gstate_overprint_metadata_is_captured --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest annotation display-text exactness gates

These gates were run after synthesized annotation/widget display text stopped
serializing non-finite internal real values as `0` and stopped partially
filtering malformed array members in the non-strict helper. Explicit `/Off`
remains the accepted empty state. This does not add corpus validation or
external runtime parity.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine annotation_display_text_refuses_nonfinite_or_filtered_array_values --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine widget_malformed_display_text_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest Type 4 calculator output exactness gates

These gates were run after the Type 4 calculator evaluator stopped coercing
booleans to numeric operands or final output components, stopped using numeric
truthiness for `if`/`ifelse`, and stopped filtering final procedures while
preserving earlier numeric stack values. This closes a pure function-evaluator
boundary for shading, tint-transform, and transfer-function consumers without
running corpus validation.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine type4_rejects_boolean_or_procedure_outputs_instead_of_numeric_coercion --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type4_rejects_boolean_numeric_operands_and_numeric_conditions --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type4_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest image filter-chain capability exactness gates

These gates were run after image decode capability planning stopped ignoring
unknown filters when a later terminal filter was recognized. Unknown wrappers
now force an `UnknownCodec` report instead of inheriting CCITT/raw/JPEG
source-window or reduction capabilities. Valid fully known CCITT chains still
report native source-window support.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine unknown_filter_in_chain_prevents_terminal_codec_capability_claim --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ccitt_plan_uses_windowed_source_region_when_clipped --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest active shading Domain sample exactness gates

These gates were run after active shading function sample inputs stopped using a
loose numeric-array helper for present `/Domain` metadata. Present malformed
Domain entries now fail typed through strict finite numeric parsing instead of
being dropped from the sample array or treated as absent defaults. This does not
add mesh/nonlinear shading support, corpus validation, or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_active_shading_dictionary_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest visual font metric exactness gates

These gates were run after visual text decoding started validating render
metric metadata before glyph placement. Present simple `/Widths`, CID `/DW` and
`/W`, and CID vertical `/DW2` and `/W2` metadata now fail typed when malformed
instead of filtering malformed array members, truncating simple widths, or
defaulting malformed vertical triples. Standard 14 and absent spec defaults
remain accepted.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine visual_font_metrics_reject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine fonts::resolver --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine visual_text --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS regional Image XObject color-space preflight gates

These gates were run after regional Image XObject classification started
resolving non-mask `/ColorSpace` metadata through page resources and checking
the resolved space with the same vector-output support predicate used for
regional inline images. Unsupported direct names, unsupported resource aliases,
and malformed color-space arrays now remain on the whole-page typed
raster/refusal boundary before bounded SVG/PostScript image emission.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine classifier_unsupported_image_xobject_color_space_stays_whole_page --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine svg_output_image_xobject_resource_color_space_uses_regional_embed --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_output_image_xobject_resource_color_space_uses_regional_colorimage --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest inline image parameter-pair preflight gates

These gates were run after inline-image `ID` parameter handling stopped
normalizing malformed metadata into smaller valid dictionaries. Non-name keys,
dangling keys, duplicate canonical keys, and short/long alias duplicates now
fail typed in the parser or stay on the SVG/PostScript whole-page typed
raster/refusal boundary before active decode planning or bounded regional
emission.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_inline_image_parameters --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_inline_image_malformed_parameter_pairs_return_typed_refusal --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine content::parser --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --all-targets --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest PostScript binary regional alpha gates

These gates were run after PostScript regional RGBA replay stopped treating all
mixed alpha as unsupported. Decoded fully transparent regional RGBA now remains
an exact no-op, binary 0/255 RGBA builds a bounded device-space clip before
`colorimage`, and fractional RGBA still falls back through whole-page raster
output. JPX `/SMaskInData 1` and `2` metadata is now deferred to this decoded
alpha decision instead of rejected purely from metadata. This stayed local and
synthetic; it does not add fractional transparency, broader soft-mask
regionalization, corpus validation, benchmark evidence, or external viewer
proof.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine internal_alpha_defers --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine regional_rgba_alpha_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine regional_colorimage_hex --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine regional_binary_alpha_clip_path --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest retained XObject subtype gates

These gates were run after retained display-list XObject capture stopped
representing missing XObject resources, XObject streams with no `/Subtype`, or
unsupported XObject subtypes as native Form descriptors. Packed-plan compilation
also refuses stale or manually constructed Image/Form descriptors whose resolved
XObject subtype does not match the retained descriptor kind. Those cases now
mark the retained display list unsupported or produce an explicit packed compile
refusal before native Image/Form replay can be selected.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine malformed_xobject_subtype_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_refuses_resolved_xobject_subtype_mismatch --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine xobject_subtype --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest packed state-resource descriptor gates

These gates were run after packed graphics-state descriptor compilation stopped
allowing resource-aware state operations to retain missing runtime lookups.
When a page/form resource table is available, missing `/ExtGState`, `/Font`,
named non-device color-space, pattern color, and marked-content `/Properties`
resources now compile to typed packed refusals. Retained text-state `/Tf`
descriptors use the same missing-font refusal path. Resource-free debug
compilation still permits unresolved names, while resource-aware active plans
fail closed before vector replay can skip a refusal descriptor.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine packed_plan_refuses --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_retained_text_refuses --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest Type 3 retained exactness gates

These gates were run after Type 3 CharProc full replay stopped using immediate
CharProc dispatch when a retained subplan is unavailable in compatibility or
high-quality/exact rendering. The retained Type 3 compiler now returns the
exact retained replay refusal reason, including packed descriptor refusals such
as unsupported state operators. Compatibility and high-quality/exact rendering
record a typed unsupported retained CharProc replay error instead of running
the immediate CharProc path. The full CharProc replay helper now returns `false`
when retained compilation or execution records that fatal boundary, instead of
returning success after recording the typed error. Parsed CharProc and
path-geometry cache misses now also retain the exact source-collection failure
reason for unresolved `/CharProcs`, decode failures, byte/op caps, parse
failures, or unsupported/empty geometry instead of collapsing those failures
into a generic unavailable entry.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine type3 --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest retained subprogram exactness gates

These gates were run after cached Form XObject, transparency Form group,
annotation appearance, tiling-pattern, and soft-mask `/G` Form subprograms
stopped dropping the retained-plan refusal reason. Subprogram replay now retains
or computes the reason beside the optional plan. High-quality/exact and
compatibility rendering record typed retained-replay refusals instead of
dispatching raw subprogram operations through the immediate renderer.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine unsupported_retained --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SMask cache identity gates

These gates were run after soft-mask `/G` cache identity stopped relying on
clip visible bounds alone and after `/BC`/default-alpha backdrop semantics moved
ahead of cache lookup. The SMask group cache key now includes the structural
clip-state fingerprint, active color-management policy, active overprint policy,
resolved group color policy, deterministic `/G` resource fingerprint, and
derived initial-backdrop/outside-alpha identity in addition to revision, source
seed, render contract, viewport/tile, device transform, print/profile policy,
optional-content state, and resource budget.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine smask_group_cache_key_includes_contract_output_identity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest SMask non-device backdrop gates

These gates were run after luminosity soft-mask `/BC` conversion stopped treating
well-formed non-device group color spaces as an automatic typed refusal.
`render/page_renderer.rs` now carries a resolved CalGray/CalRGB/Lab/Indexed/
ICCBased/Separation/DeviceN color-space object in the SMask group policy,
resolves `/S /Luminosity` `/BC` through the existing named-color/CMM path, and
salts the soft-mask cache key with the resolved non-device color-space
fingerprint. Alpha SMask groups with non-device `/BC`, malformed Indexed
palettes, malformed tint transforms, malformed or empty ICCBased profiles,
richer non-device group spaces, and broader active group-space compositing still
fail typed. These tests stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication was
used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine smask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest transparency group Separation/DeviceN inert gates

These gates were run after opaque, normal Form transparency groups started
accepting well-formed one-component Separation and DeviceN group `/CS` values
through the existing non-device-inert active policy. Alpha/backdrop-observable
Form groups and alpha SMask groups with non-device `/BC` still fail typed; this
does not claim full non-device group-space compositing.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest SVG/PS tint-transform group-colour Form gates

These gates were run after SVG/PostScript vector Form classification started
accepting opaque, normal, vector-safe transparency-group Forms whose group `/CS`
is a well-formed Separation or one-/multi-component DeviceN tint-transform color
space. The vector path validates non-`/None` colorants, exact DeviceN component
names/counts, and zero/full tint samples through the named-color resolver before
native replay. Alpha/backdrop-observable Forms, malformed tint transforms,
unsupported alternates, malformed Separation colorants, `/Separation /None`, and
all-`/None` DeviceN spaces still fail closed to the typed whole-page
raster/refusal boundary. The focused regional-vector filter now covers 14
SVG/PostScript group-colour cases, including two-colorant DeviceN.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine vector_group_color_space_rejects_malformed_separation_colorant --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine group_color_space_opaque_transparency_group_form_xobject_replays_natively --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-engine --test regional_vector_fallback --jobs 1 -- -D warnings` | 0 |

## Latest transparent-page-group contract identity gates

These gates were run after the transparent-page-group decision cache stopped
using only page number plus document revision. The key now also includes the
active render-contract fingerprint while preserving the `page:{n}:` prefix used
by page-artifact invalidation, so display/print/proof, optional-content,
resource-budget, and other contract variants cannot share a stale page
transparency decision.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine transparent_page_group --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest transaction dependency-retention gates

These gates were run after cache-level narrow invalidation stopped clearing the
dependency graph on successful non-reset revision advancement.
`RenderDependencyGraph::advance_revision` is now the explicit non-destructive
revision path, while `reset_revision` remains the dependency-clearing reset
path. Sequential source-only edits keep the recorded source-to-page/tile edges
across revision changes, so a second source edit invalidates its own recorded
consumers instead of falling back to conservative cache reset. This stayed local
and source-only; no PDF corpus, benchmark, competitor comparison, VPS,
deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine sequential_source_only_edits_retain_dependency_edges --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest transaction dirty-tile coverage gates

These gates were run after exact dirty-tile cache invalidation stopped assuming
that any non-empty tile set covers every reported affected page or every
source-derived invalidated page. Direct `TransactionWriteSet` application and
binding-safe JSON plan application now use exact tile invalidation only when
each reported affected page has at least one supplied dirty tile. If a
multi-page plan supplies exact tiles for only a subset, recorded page tiles for
uncovered reported pages are invalidated through the page/tile path instead of
being left stale. The dependency graph also expands recorded page tiles for any
source-derived invalidated page whose exact/source-tile set is absent or only
partially covers that page's recorded rendered tiles. SDK invalidation reports
now expose `exact_tile_coverage_complete` and `uncovered_affected_pages`. This
stayed local and source-only; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine exact_tile_mode_expands_source_pages_without_exact_tile_coverage --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine exact_tile_mode_expands_source_pages_with_partial_source_tile_coverage --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine exact_tiles_expand_coarse_source_pages_with_partial_source_tile_coverage --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine editing_transaction_apply_with_render_invalidation_exposes_dirty_tiles --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render::invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest direct shading helper-local gates

These gates were run after direct Type 1 function-based, Type 2 axial, and Type
3 radial shading paint helpers stopped filtering malformed arrays, truncating
overlong arrays, defaulting malformed booleans, or falling back to
`/DeviceRGB` for missing/malformed color-space metadata below the public active
shading validator, or fall through to compatibility black/transparent colors
for unsupported named color-space resolution. Present malformed `/Coords`,
`/Domain`, `/Extend`, Type 1 `/Matrix`, malformed shading `/ColorSpace`
metadata, or unsupported color spaces now return without paint when the private
helpers are called directly. Missing optional `/Domain`, `/Extend`, and
Type 1 `/Matrix` still use the PDF defaults. This stayed local and synthetic;
no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine direct_shading_helpers_reject_malformed_local_shape_fields --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine direct_shading_helpers_reject_unsupported_color_space_locally --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest SVG/PS shading Extend gates

These gates were run after vector shading plans started carrying parsed
`/Extend` flags, absent `/Extend` became an executable default-to-non-extended
contract, the PostScript writer stopped hard-coding `[true true]` in
native `shfill` dictionaries, and the SVG writer started composing native
clipPaths for otherwise vector-safe axial shadings whose start and/or end is
not extended. A later local slice adds exact SVG clipping for the finite
concentric radial subset; nonconcentric, reversed-radius, non-finite, or
otherwise unsupported radial non-default `/Extend` cases remain fail-closed.
This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nonextended_axial_shading_is_svg_and_postscript_native --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine absent_extend_axial_shading_defaults_to_nonextended_vector_flags --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_output_nonextended_axial_shading_preserves_extend_flags --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine vector_fallback --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nonextended_axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine shading_bbox_clip_refuses_unflattenable_geometry --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest SVG/PS named paint color gates

These gates were run after SVG and PostScript vector sinks started enforcing
named paint color-space resolution locally below classifier preflight. Valid
`/Separation /None` still produces no paint, but malformed, missing, invalid,
or unsupported named paint color spaces now record fatal vector-output errors
instead of transparent paint or compatibility black fallback. This stayed local
and synthetic; no PDF corpus, benchmark, competitor comparison, VPS,
deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine svg_named_color_resolution_rejects_invalid_space_locally --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine postscript_named_color_resolution_rejects_invalid_space_locally --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest generic device color-component arity gates

These gates were run after retained display-list capture and active page
rendering started validating generic `SC`/`SCN`/`sc`/`scn` operands against
the current device color space before graphics-state mutation. Generic
`/DeviceGray`, `/DeviceRGB`, and `/DeviceCMYK` paint now requires exact finite
component counts and rejects stray pattern resource names, so malformed device
paint cannot reach retained path color capture or active `ColorSpaceHandler`
conversion through padded or truncated components. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_device_sc_arity_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_device_sc_arity_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_device_sc_arity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest marked-content and restore side-stack gates

These gates were run after active page rendering made the lower-level
optional-content visibility pop fail typed on empty marked-content state, and
retained display-list capture made `Q` restore reject desynchronized clip,
color-explicitness, and soft-mask side stacks instead of defaulting those
states. This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine graphics_state_restore_side_stack_desync_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine direct_marked_content_visibility_pop_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_graphics_state_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_marked_content_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest retained text helper-local gates

These gates were run after retained text conversion started applying the shared
text operand validator locally and retained text replay started treating
malformed or unsupported retained text descriptors as fatal render refusals
instead of zero/default operand conversion or skipped retained replay. The valid
native retained text path was checked as well. This stayed local and synthetic;
no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine retained_text_conversion_rejects_malformed_operands_locally --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine direct_retained_text_replay_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_text_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_text_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine text_is_replayable_as_native_operation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest retained restore replay gates

These gates were run after retained display-list replay and packed-plan replay
started refusing `Restore` underflow before mutating graphics state. Retained
restore now requires matching graphics-state, clip, and SMask side-stack entries
instead of warning and continuing with default or stale retained clip/mask state.
This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine direct_retained_restore_underflow_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_restore_underflow_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_graphics_state_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine graphics_state_restore_side_stack_desync_marks_display_list_unsupported --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest page-content and Form restore side-stack gates

These gates were run after active page-content `Q` replay and Form XObject
cleanup started refusing missing clip/SMask side-stack sentinel state before
graphics-state mutation or cleanup restore. Page-content restore and Form cleanup
now return typed fatal refusals instead of warning and continuing with stale or
default clip/mask state, while existing balanced `q/Q` clip and SMask restores
remain covered. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication was
used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine side_stack_desync_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine q_restore --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_graphics_state_operator_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest render-facing device color-state gates

These gates were run after `ColorSpaceHandler::strict_to_render_color` became
the render-facing device graphics-state conversion boundary for active page
paint, retained display-list simple-color capture, Type 3 inherited vector
color replay, and SVG/PostScript vector sinks. Internal `/DeviceGray`,
`/DeviceRGB`, and `/DeviceCMYK` state now requires exact finite component
vectors below PDF operand validation; malformed state records typed
render/vector-output refusal or declines retained metadata instead of padding
missing components into plausible pixels. This stayed local and synthetic; no
PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine strict_to_render_color_requires_exact_finite_device_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine display_list_simple_color_requires_exact_finite_device_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine device_color_resolution_rejects_malformed_state_locally --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_device_sc_arity_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest SVG radial Extend gates

These gates were run after SVG vector-shading classification started accepting
finite concentric radial shadings with explicit non-default `/Extend` pairs and
the SVG sink started composing clipped native radial gradients for that exact
subset. `[true false]` clips to the outer radius, `[false false]` clips to an
even-odd annulus when the inner radius is positive, and `[false true]` clips to
the page minus the positive inner radius. Nonconcentric, reversed-radius,
non-finite, or otherwise unsupported radial non-default `/Extend` shadings
remain conservative for SVG; PostScript continues to preserve native `/Extend`
semantics through `shfill`. This stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nonextended_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine svg_output_nonextended_concentric_radial_shading_uses_clipped_native_gradient --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine radial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest PostScript DeviceCMYK exact shading-function gates

These gates were run after PostScript vector-shading classification started
carrying exact direct `/DeviceCMYK` Type 2 function sidecars and the PostScript
sink started emitting those shadings as native `/ColorSpace /DeviceCMYK`
LanguageLevel 3 `shfill` dictionaries. This preserves finite unit-domain
DeviceCMYK `/C0`, `/C1`, and `/N` values instead of forcing whole-page raster
fallback for nonlinear Type 2 functions or converting direct linear CMYK
shadings to sampled RGB anchors. SVG, calibrated/named nonlinear shadings,
non-unit shading dictionaries, out-of-unit or malformed ranges, malformed domains, mesh shadings, malformed dictionaries, and
unsupported local constructs remain conservative. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_device_cmyk_axial_shading_is_exact_postscript_regional --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_output_nonlinear_cmyk_axial_shading_uses_exact_device_cmyk_shfill --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cmyk_axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cmyk_radial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_rgb_shading_function --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest PostScript mixed-exponent RGB function-array gates

These gates were run after PostScript vector-shading classification started
carrying exact direct `/DeviceRGB` Type 2 component-function arrays whose
channels have different finite unit-domain exponents. Same-exponent arrays
still collapse to one RGB `/FunctionType 2` sidecar; mixed-exponent arrays now
serialize as exact per-component PostScript `/Function` arrays instead of
forcing whole-page raster fallback or approximating from sampled RGB stops. SVG,
calibrated/named nonlinear shadings, non-unit shading dictionaries, out-of-unit or malformed ranges, malformed domains, mesh
shadings, malformed dictionaries, and unsupported local constructs remain
conservative. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication
was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_device_rgb_type2_function_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_rgb_shading_function_preserves_exact_type2_component_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_output_mixed_exponent_function_array_axial_shading_uses_exact_native_functions --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine function_array --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest PostScript DeviceCMYK function-array gates

These gates were run after PostScript vector-shading classification started
carrying exact direct `/DeviceCMYK` Type 2 component-function arrays. Arrays
with the same finite unit-domain exponent collapse to one exact CMYK
`/FunctionType 2` sidecar; mixed-exponent arrays serialize as exact
per-component PostScript `/Function` arrays under `/ColorSpace /DeviceCMYK`.
SVG, calibrated/named nonlinear shadings, non-unit shading dictionaries, out-of-unit or malformed ranges, malformed domains,
mesh shadings, malformed dictionaries, and unsupported local constructs remain
conservative. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication
was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_device_cmyk_type2_function_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_cmyk_shading_function_preserves_exact_type2_component_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_output_mixed_exponent_cmyk_function_array_axial_shading_uses_exact_native_functions --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine cmyk --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest PostScript DeviceCMYK stitching-function gates

These gates were run after PostScript vector-shading classification started
carrying exact direct Type 3 stitching functions whose Type 2 segments are
finite and unit-domain, including non-linear segment exponents. The PostScript
sink serializes RGB/Gray stitching as native `/FunctionType 3` and CMYK
stitching as native `/ColorSpace /DeviceCMYK` `/FunctionType 3` `shfill`
instead of whole-page raster fallback. SVG, non-unit shading dictionaries, out-of-unit or malformed ranges, malformed domains,
mesh shadings, malformed dictionaries, and unsupported local constructs remain
conservative. This stayed local and synthetic; no PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_stitching_function_axial_shading_is_postscript_regional --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine discontinuous_device_cmyk_stitching_function_axial_shading_is_postscript_regional --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_cmyk_shading_function_preserves_exact_type3_stitching --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_output_cmyk_stitching_function_axial_shading_uses_exact_native_functions --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine stitching_function --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest advanced annotation invalidation gates

These gates were run after `advanced_editing.rs` added structured render
write-set fields to `CacheInvalidationReport` and populated them for explicit
Link annotation rectangle moves plus Ink annotation appearance regeneration.
The Link path reports the changed annotation object, affected page, and both
old/new annotation rectangles. The Ink path reports the changed annotation
object, created appearance Form object, affected page, and regenerated
appearance rectangle. This stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine explicit_link_annotation_rect_move_preserves_action_and_quadpoints --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine annotation_ink_fit_saves_cubic_appearance_and_raw_points --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine advanced_editing --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest EditingTransactions TextReflow routing gates

These gates were run after `editing_transactions.rs` stopped blanket-refusing
scene text `geometric_block` and `semantic_document` modes before TextReflow
could handle them, then after the compact scene request began forwarding
explicit TextReflow region, allowed-expansion, next-flow target, downstream
move, layout, language, direction, alignment, line-height, hyphenation,
page-creation, font-reduction, signature, and low-confidence review-approval
fields. Geometric scene text transactions now route through
`text_reflow::apply_reflow_region`; semantic scene text transactions route
through `text_reflow::apply_reflow_document` and preserve TextReflow typed
review/refusal behavior. The adapter converts source refs, affected pages,
dirty regions, typed refusal metadata, and inverse metadata back into the
EditingTransactions render-invalidation report shape. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine geometric_block_routes_to_text_reflow_transaction_adapter --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine geometric_block_scene_request_forwards_explicit_reflow_region --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine semantic_document_route_preserves_text_reflow_review_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine semantic_document_scene_request_forwards_review_approval --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine geometric_block_text_reflow_transaction_drives_render_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine editing_transactions --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest non-progressive render cache-handle gates

These gates were run after ordinary contract rendering gained caller-owned
`RenderDocumentCache` handles outside progressive sessions. The Rust engine now
has cache-taking contract render and report paths, the C ABI/header expose an
opaque `WellfriendRenderCache` with clear and render-invalidation-plan
application, and Python, WASM/TypeScript, .NET, and Java expose matching
non-progressive cache-owner APIs for PNG contract rendering and render-report
output. The report scope for these paths is
`caller_owned_render_cache_report`, so hosts can distinguish one-shot telemetry
from caller-owned cache telemetry. This stayed local and synthetic; no PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used. Shared-resource transitive write-set
classification, safe narrowing of conservative page-level resource
dependencies, and external binding runtime parity remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-capi capi_render_cache_handle_renders_and_applies_invalidation_plan --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-py --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1` | 0 |
| `dotnet build bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --nologo /m:1 -v:minimal` | 0 |
| `javac --enable-preview --release 25 -d .work\final-universal-renderer-implementation\java-classes bindings\java\src\main\java\io\wellfriendpdf\WellfriendPdf.java bindings\java\src\test\java\io\wellfriendpdf\WellfriendPdfSmokeTest.java` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --all-features --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --all-features --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest transitive retained resource-op tile dependency gates

These gates were run after retained tile dependency recording began walking one
bounded source-reference set for intersecting XObject/Form, named shading, and
active state-resource objects. A Form XObject tile now records the Form source
object and nested resource objects declared by that Form resource dictionary.
Text/font, fill/stroke color-space, ExtGState, and pattern paints also record
nested descriptor/function/SMask/resource refs when the operation intersects the
tile, so edits to those nested sources can invalidate the exact recorded tile
without expanding raster invalidation to the whole page. Source-only mapped
transactions and render-invalidation plans with no reported affected pages now
enter page-artifacts-plus-exact-tiles invalidation, preserving exact source
tiles when recorded coverage is complete and expanding source-derived pages when
coverage is incomplete. This stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used. Broader transaction write-set classification across
arbitrary shared resources, remaining conservative page-level narrowing, and
external runtime matrices remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine bounded_form_xobject_records_transitive_resource_tile_dependency --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transitive_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine source_dependency --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine bounded_form_xobject --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine exact_tile_mode --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-targets --all-features --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --all-features --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0 |

## Latest WASM separable lane gates

These gates were run after `blend_separable_opaque_destination` stopped using a
scalar per-channel loop for the wasm separable modes.
wasm32+`simd128` now uses lane math for Multiply, Screen, Overlay, Darken,
Lighten, ColorDodge, ColorBurn, HardLight, SoftLight, Difference, and
Exclusion and forces alpha lanes to 255. Runtime
capabilities now disclose the same lane/fallback boundary. This stayed local
and synthetic; no PDF corpus, benchmark, competitor comparison, VPS,
deployment, release, tag, or package publication was used. Broader
non-normal/high-quality SIMD rows and general wasm-bindgen-test harness
execution remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-render-simd separable --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --all-targets --jobs 1` | 0 |
| `$env:RUSTFLAGS='-C target-feature=+simd128'; cargo check -p wellfriendpdf-render-simd --target wasm32-unknown-unknown --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --all-targets --jobs 1 -- -D warnings` | 0 |
| `$env:RUSTFLAGS='-C target-feature=+simd128'; $env:CARGO_TARGET_DIR='E:\wellpdfsdk\.work\final-universal-renderer-implementation\cargo-target'; cargo build --manifest-path .work\final-universal-renderer-implementation\wasm-simd-smoke\Cargo.toml --target wasm32-unknown-unknown --release --jobs 1` | 0 |
| `node -e "WebAssembly.instantiate(...wellfriendpdf_wasm_simd_smoke.wasm...).then(({instance})=>instance.exports.run_wasm_simd_smoke())"` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/render-simd/src/lib.rs crates/engine/src/runtime.rs docs/renderer/final-remaining-blocker-preaudit.md docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-local-implementation-closure.md docs/renderer/final-universal-implementation-report.md docs/renderer/final-universal-renderer-implementation-report.md` | 0 |

Runtime wasm test execution is still not counted as complete evidence. The
local `wasm-bindgen-test-runner` attempt rejected libtest-style
`--test-threads`/`--nocapture` arguments, and the retry without those arguments
started the wasm test artifact but reported `no tests to run` because the
current wasm-only functions are ordinary `#[test]` functions rather than a
wasm-bindgen-test-exported harness. A separate temporary `.work` cdylib smoke
was then built with wasm32+`simd128`, run through Node's WebAssembly runtime,
and returned `run_wasm_simd_smoke=0` for the separable helper. That is a narrow
runtime proof for this helper, not a complete replacement for a general wasm
test harness.

## Latest native separable blend SIMD gates

These gates were run after native x86/x86_64 `blend_separable_opaque_destination`
stopped returning scalar-only for common opaque-destination separable blend rows.
The helper now dispatches to an SSE2 row path when available. Multiply, Screen,
Overlay, Darken, Lighten, HardLight, Difference, and Exclusion use exact integer
lane math; ColorDodge, ColorBurn, and SoftLight use SSE float lanes with grouped
gather/scatter. The helper uses scalar debug oracles and scalar
tails for uneven row lengths. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd blend_separable_opaque_dst_public_uses_sse2_when_available --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd separable --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd sse --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest opaque-background flatten SIMD gates

These gates were run after Compat page-background flattening gained native
x86/x86_64 SSE2 and wasm32+`simd128` `render-simd::flatten_opaque_background`
row helpers before the existing portable-wide fallback. The helper uses the same
byte source-over contract as `flatten_compat_onto_opaque_background`, scalar
debug oracles, and scalar tails for uneven row lengths. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd flatten_opaque_background --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine flatten_compat_opaque_background --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |
| `$env:RUSTFLAGS='-C target-feature=+simd128'; cargo check -p wellfriendpdf-render-simd --target wasm32-unknown-unknown --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest soft-mask SIMD group-alpha gates

These gates were run after `composite_soft_mask_opaque_destination` stopped
rejecting non-opaque group alpha before SIMD dispatch. The SIMD crate and the
engine row compositor now share an engine-compatible effective-alpha helper for
`src_alpha * mask * group_alpha / 255^2`, wasm32+`simd128` and native guarded
kernels can accept `group_alpha_255 < 255`, and the engine opaque-destination
soft-mask row path stays on the wide/SIMD route for partial group opacity
before scalar fallback. This stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used. Broader non-normal, high-quality, and group-space SIMD
coverage remains incomplete.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-render-simd soft_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine composite_from_soft_mask_fast_path_matches_opaque_destination_math --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `$env:RUSTFLAGS='-C target-feature=+simd128'; cargo check -p wellfriendpdf-render-simd --target wasm32-unknown-unknown --jobs 1` | 0 |

## Latest source-over group-alpha row gates

These gates were run after normal Compat source-over rows stopped dropping
mixed source-alpha plus partial group alpha over opaque destinations directly
to the general scalar compositor. The generalized opaque-destination
uniform-alpha row path scales each source pixel alpha by the byte-effective
group alpha, uses the portable-wide row compositor for two-pixel groups, and
keeps a scalar tail for short rows. Runtime capability JSON now names the
mixed-source group-alpha opaque-destination lane. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used. Non-normal blend modes,
high-quality group-space blending, and full
transparency-group closure remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine uniform_alpha --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine composite_from_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest source-over mixed-destination row gates

These gates were run after normal Compat source-over rows with non-opaque
destination alpha stopped dropping directly to the general scalar compositor.
`composite_normal_compat_row` now routes those rows through a portable
`wide::f32x4` helper before scalar tails, preserving the scalar source-over
oracle for source alpha, group alpha, destination alpha, RGB rounding, and
alpha truncation. Runtime capability JSON now names
`mixed_destination_source_over_f32x4`, and CLI compositor telemetry exposes
`wide_general_pixels_delta`. This stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used. Non-normal blend modes, high-quality group-space
blending, and full transparency-group closure remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine composite_from_mixed_destination_general_row_uses_wide_path --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine "composite_from_" --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1 -- -D warnings` | 0 |

## Latest soft-mask mixed-destination row gates

These gates were run after normal Compat soft-mask rows with non-opaque
destination alpha stopped dropping directly to the scalar soft-mask general
compositor. The new portable `wide::f32x4` row helper uses the existing
`soft_mask_effective_alpha` byte contract, preserves scalar RGB rounding and
alpha truncation, and keeps scalar tails for short rows. Runtime capability JSON
now names `soft_mask_mixed_destination_f32x4`, and CLI compositor telemetry
exposes `wide_soft_mask_general_pixels_delta`. This stayed local and synthetic;
no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used. Non-normal blend modes, high-quality
group-space blending, and full transparency-group closure remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine composite_from_soft_mask_mixed_destination_general_row_uses_wide_path --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine "composite_from_" --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-cli --bin wellfriendpdf --jobs 1 -- -D warnings` | 0 |

## Latest separable mixed-destination solid-fill row gates

These gates were run after separable `fill_rect` rows stopped requiring opaque
destination alpha before using row compositors. Opaque destination rows still use
the existing optimized separable helpers; mixed-destination rows now use a
portable `wide::f32x4` source-over helper that preserves the scalar separable
blend oracle for source alpha, active partial-clip opacity, destination alpha,
RGB rounding, and alpha truncation, with scalar tails for short rows. Runtime
capability JSON now names `separable_solid_mixed_destination_f32x4_rows`. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used. High-quality
group-space blending and full transparency-group closure remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine separable_fill_rect_mixed_destination --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine separable_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest separable RGBA mixed-destination partial-clip row gates

These gates were run after cached RGBA glyph/image fragment rows with separable
blend modes and partial clips stopped requiring opaque destination alpha before
using row compositors. Opaque destination rows still use the existing optimized
partial-clip helper; mixed-destination rows now use a portable `wide::f32x4`
source-over helper that preserves the scalar separable blend oracle for
per-pixel source alpha, active partial-clip opacity, destination alpha, RGB
rounding, and alpha truncation, with scalar tails for short rows. Runtime
capability JSON now names
`separable_rgba_mixed_destination_partial_clip_f32x4_rows`. This stayed local
and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used. High-quality group-space
blending and full transparency-group closure remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine separable_rgba_pixels_at --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine separable_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/buffer.rs crates/engine/src/runtime.rs docs/renderer/final-local-implementation-closure.md docs/renderer/final-remaining-blocker-preaudit.md docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-universal-renderer-implementation-report.md` | 0 |

## Latest separable RGBA mixed-destination row gates

These gates were run after cached RGBA glyph/image fragment rows with separable
blend modes and no clip, all-visible clips, or binary clips stopped falling
back to per-pixel `blend_pixel` for non-opaque destination rows. Opaque
destination rows use the separable RGBA row helper, and mixed-destination rows
use a portable `wide::f32x4` source-over helper that preserves the scalar
separable blend oracle for per-pixel source alpha, destination alpha, RGB
rounding, and alpha truncation, with scalar tails for short rows. Runtime
capability JSON now names `separable_rgba_mixed_destination_f32x4_rows`. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used. High-quality
group-space blending and full transparency-group closure remain incomplete.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine separable_rgba_pixels_at --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine separable_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/buffer.rs crates/engine/src/runtime.rs docs/renderer/final-local-implementation-closure.md docs/renderer/final-remaining-blocker-preaudit.md docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-universal-renderer-implementation-report.md` | 0 |

## Latest compact group compositor oracle gates

These gates were run after compact `composite_from_at` non-normal and
high-quality fallback compositing stopped using its own preblend loop and
delegated to the same `blend_pixel` scalar oracle used by full-page group
flattening. The fallback still stays scalar; this closes the compact-only
semantic divergence for non-normal blend modes, non-separable blend modes, and
high-quality gamma handling without claiming SIMD/group-space completion. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine compact_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine composite_from_at --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine "composite_from_" --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `git diff --check -- crates/engine/src/render/buffer.rs crates/engine/src/runtime.rs docs/renderer/final-local-implementation-closure.md docs/renderer/final-remaining-blocker-preaudit.md docs/renderer/complete-algorithm-and-method-inventory.md docs/renderer/final-universal-renderer-implementation-report.md` | 0 |

## Latest high-quality normal group row gates

These gates were run after high-quality Normal group compositing stopped using
per-pixel dispatcher fallback for unmasked full-row, compact binary-clip, and
partial-clip windows. `composite_from` and `composite_from_at` now route
eligible windows through `composite_normal_high_quality_row` or
`composite_normal_high_quality_row_partial_clip`, preserving the `blend_pixel`
linear-light source-over oracle for source alpha, group alpha, clip coverage,
destination alpha, RGB conversion, and alpha truncation. Dedicated knockout row
gates cover active knockout backdrops; non-device group colour-space/backdrop semantics and
full group-space closure remain on the scalar semantic fallback. This stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_group_composite_row_matches_blend_pixel_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_group_composite_partial_clip_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine compact_high_quality_binary_clip_group_composite_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine compact_high_quality_partial_clip_group_composite_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality blend group row gates

These gates were run after high-quality non-normal group compositing stopped
using per-pixel dispatcher fallback for unmasked full-row, compact, and
partial-clip windows. `composite_from` and `composite_from_at` now route eligible
windows through `composite_high_quality_blend_row` or
`composite_high_quality_blend_row_partial_clip`, preserving the `blend_pixel`
linear-light blend/source-over oracle for source alpha, group alpha, clip
coverage, destination alpha, RGB conversion, and alpha truncation. Dedicated
knockout row gates cover active knockout backdrops; non-device group colour-space/backdrop
semantics and full group-space closure remain on the scalar semantic fallback. This stayed local
and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_blend_group --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine compact_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine "composite_from_" --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality cached RGBA row gates

These gates were run after high-quality Normal cached RGBA glyph/image fragment
painting stopped using per-pixel dispatcher fallback for unmasked no-clip,
all-visible, binary-clip, and partial-clip row windows. `blend_rgba_pixels_at`
now routes eligible rows through `composite_normal_high_quality_row` or
`composite_normal_high_quality_row_partial_clip`, preserving the `blend_pixel`
linear-light source-over oracle for per-pixel source alpha, clip coverage,
destination alpha, RGB conversion, and alpha truncation. Dedicated knockout row
gates cover active knockout backdrops; full group-space closure remains on the scalar semantic fallback. This stayed
local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_rgba_pixels_at_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_rgba_pixels_at_binary_clip_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_rgba_pixels_at_partial_clip_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality blend cached RGBA row gates

These gates were run after high-quality non-normal cached RGBA glyph/image
fragment painting stopped using per-pixel dispatcher fallback for unmasked
no-clip, all-visible, binary-clip, and partial-clip row windows.
`blend_rgba_pixels_at` now routes eligible rows through
`composite_high_quality_blend_row` or
`composite_high_quality_blend_row_partial_clip`, preserving the `blend_pixel`
linear-light blend/source-over oracle for per-pixel source alpha, clip coverage,
destination alpha, RGB conversion, and alpha truncation. Dedicated knockout row
gates cover active knockout backdrops; full group-space closure remains on the scalar semantic fallback. This stayed
local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_blend_rgba_pixels_at --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine rgba_pixels_at --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality alpha-mask row gates

These gates were run after high-quality Normal alpha-mask glyph/image/stroke
painting stopped using per-pixel dispatcher fallback for unmasked no-clip,
all-visible, binary-clip, and partial-clip row windows. `blend_alpha_mask` and
`blend_alpha_mask_strided` now route eligible rows through
`blend_alpha_mask_run_high_quality_normal` or
`blend_alpha_mask_run_high_quality_normal_partial_clip`, preserving the
`blend_pixel` linear-light source-over oracle for constant paint colour, mask
alpha, clip coverage, destination alpha, RGB conversion, and alpha truncation.
Dedicated knockout row gates cover active knockout backdrops; full group-space
closure remains on the scalar semantic fallback. This stayed local and synthetic; no PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_alpha_mask_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_alpha_mask_binary_clip_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_alpha_mask_partial_clip_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality blend alpha-mask row gates

These gates were run after high-quality non-normal alpha-mask glyph/image/stroke
painting stopped using per-pixel dispatcher fallback for unmasked no-clip,
all-visible, binary-clip, and partial-clip row windows. `blend_alpha_mask` and
`blend_alpha_mask_strided` now route eligible rows through
`blend_alpha_mask_run_high_quality_blend` or
`blend_alpha_mask_run_high_quality_blend_partial_clip`, preserving the
`blend_pixel` linear-light blend/source-over oracle for constant paint colour,
mask alpha, clip coverage, destination alpha, RGB conversion, and alpha
truncation. Dedicated knockout row gates cover active knockout backdrops; full
group-space closure remains on the scalar semantic fallback. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_blend_alpha_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality solid-fill row gates

These gates were run after high-quality Normal rectangle fills stopped using
per-pixel dispatcher fallback for unmasked no-clip, all-visible, binary-clip,
and partial-clip row windows. `fill_rect` now routes eligible rows through
`blend_solid_run_high_quality_normal` or
`blend_solid_run_high_quality_normal_partial_clip`, preserving the `blend_pixel`
linear-light source-over oracle for constant paint colour, source alpha, clip
coverage, destination alpha, RGB conversion, and alpha truncation. Dedicated
knockout row gates cover active knockout backdrops; full group-space closure remains on the scalar semantic fallback. This
stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_fill_rect_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_fill_rect_binary_clip_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_fill_rect_partial_clip_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality blend solid-fill row gates

These gates were run after high-quality non-normal solid rectangle fills stopped
using per-pixel dispatcher fallback for unmasked no-clip, all-visible,
binary-clip, and partial-clip row windows. `fill_rect` now routes eligible rows
through `blend_solid_run_high_quality_blend` or
`blend_solid_run_high_quality_blend_partial_clip`, preserving the `blend_pixel`
linear-light blend/source-over oracle for source alpha, clip coverage,
destination alpha, RGB conversion, and alpha truncation. Dedicated knockout row
gates cover active knockout backdrops; full group-space closure remains on the scalar semantic fallback. This stayed local
and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_blend_fill_rect --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine fill_rect --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality SMask solid-fill row gates

These gates were run after high-quality solid rectangle fills with an installed
SMask stopped using per-pixel dispatcher fallback for unmasked no-clip,
all-visible, binary-clip, and partial-clip row windows. `fill_rect` now routes
eligible rows through `blend_solid_run_high_quality_masked`, preserving the
`blend_pixel` linear-light blend/source-over oracle for source alpha, installed
SMask bytes, clip coverage, destination alpha, RGB conversion, and alpha
truncation. Dedicated knockout row gates cover active knockout backdrops; full
group-space closure remains on the scalar semantic fallback. This stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine smask_fill_rect --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine fill_rect --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality SMask RGBA/alpha/group row gates

These gates were run after high-quality installed-SMask cached RGBA paints,
installed-SMask alpha-mask paints, and full/compact group soft-mask composites
stopped using per-pixel dispatcher fallback for eligible non-knockout rows.
`blend_rgba_pixels_at`, `blend_alpha_mask_strided`, `composite_from`, and
`composite_from_at` now share high-quality masked row oracles that preserve the
`blend_pixel` linear-light blend/source-over oracle for source alpha, group
alpha, installed SMask bytes, group soft-mask bytes, clip coverage, destination
alpha, RGB conversion, and alpha truncation. Dedicated knockout row gates cover
active knockout backdrops; non-device group colour-space/backdrop semantics and full
transparency-group closure remain on the scalar semantic fallback. This stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_smask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_blend_smask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine group_soft_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality knockout solid-fill row gates

These gates were run after high-quality solid rectangle fills with an active
knockout backdrop stopped using per-pixel dispatcher fallback for eligible
direct paint rows. `fill_rect` now routes those rows through
`blend_solid_run_high_quality_knockout`, preserving the `blend_pixel`
linear-light blend/source-over oracle while reading the destination RGB/alpha
from the stored knockout backdrop rather than the already-mutated destination
row. The helper also fuses installed SMask bytes and optional clip coverage.
Non-device group colour-space/backdrop compositing and full transparency-group closure remain
on the scalar semantic fallback. This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_knockout_smask_fill_rect_partial_clip_uses_row_oracle --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine fill_rect --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest high-quality knockout RGBA/alpha/group row gates

These gates were run after high-quality cached RGBA paints, alpha-mask paints,
and full/compact group soft-mask composites with an active knockout backdrop
stopped using per-pixel dispatcher fallback for eligible rows.
`blend_rgba_pixels_at` and `composite_from`/`composite_from_at` now route those
rows through `composite_high_quality_masked_row_with_backdrop`; alpha-mask rows
route through `blend_alpha_mask_run_high_quality_knockout`. All of these row
oracles read destination RGB/alpha from the stored knockout backdrop, preserve
the existing `blend_pixel` linear-light source-over/blend math, and fuse source
alpha, installed SMask bytes, group soft-mask bytes, clip coverage, group alpha,
and active blend mode. Non-device group colour-space/backdrop compositing and full
transparency-group closure remain incomplete. This stayed local and synthetic;
no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine high_quality_knockout --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine rgba_pixels_at --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine group_soft_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine "composite_from_" --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest Type 3 program lifecycle gates

These gates were run after parsed Type 3 CharProc and geometry caches stopped
encoding lifecycle solely as missing entries or `Option<Arc<_>>`. Cache misses
now commit an explicit `Compiling` state before source collection, then replace
that with either `Compiled` or `Failed` plus a retained failure reason. Child
render-state cache absorption preserves those states through bounded LRU
admission. The CharProc and path-geometry source-collection paths store the
exact unresolved, decode, byte-cap, parse, op-cap, unsupported-path, or
empty-geometry reason in their negative cache entries. This stayed local and
synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication
was used. Broader unsupported CharProc state/resource matrices remain
incomplete.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine type3_program_cache --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine type3 --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest JPX alias decode-planning gates

These gates were run after `/JPX` abbreviation handling was made consistent
with `/JPXDecode` across image decode planning, stream lossless stopping,
decoder terminal dispatch, inline JPX target-resolution dispatch, DecodeParms
filter matching, codec-isolation backend registry/canonicalization, and
SVG/PostScript vector-fallback image filter
classification. Abbreviation-bearing JPX images no longer report
`UnknownCodec`, lose guarded target-resolution reduction/backend-reporting
eligibility, or whole-page rasterize from vector fallback solely because the
short filter name was used. This does not add JPX ROI/region, tile/component,
progressive, cancellation-inside-codec, corpus validation, or benchmark evidence. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine jpx_abbreviation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine jpx --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `git diff --check -- <JPX alias source/report files>` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest component-selective image decode capability gates

These gates were run after image decode planning stopped treating a requested
component subset as merely a cache-key distinction. `ImageDecodePlan` now
records `requires_component_decode`, `ImageDecodeCapabilityReport` exposes
`component_decode`, exact-mode limitation summaries include
`component-selection`, and the document-level image decode capability report
publishes `native_component_decode_count`. The guarded unfiltered raw decoder
now has an actual component-selection path for XObject and inline source
windows: selected 1/2/4/8/16-bit source samples are extracted in component
order, normalized to 8-bit interleaved output, and `/Decode` arrays are applied
against the original source component indexes. Full-image raw component
requests use an explicit full raw window rather than silently falling back to
the all-component path. The planner also normalizes explicit all-component
requests for known DeviceGray/RGB/CMYK-family source shapes so
`Components([0,1,2])` does not require native subset support for a three-channel
image, while invalid known indexes fail exact component-selection planning before
the raw decoder can be advertised as native. The image-reference capability
report now also uses image shape metadata to mark unfiltered raw and monochrome
CCITT source-window support as native in per-image JSON, instead of leaving those
entries at the filter-only conservative boundary. JPEG/JPX/JBIG2 and
filtered-lossless adapters still report unavailable native component-subset
decode; CCITT reports the trivial single-component selection as native for 1 bpc
DeviceGray/ImageMask shapes, while invalid or nontrivial subset requests fail
closed instead of planning all-component output under a subset identity. JPX
ROI/region/tile/progressive pixel output remains incomplete. This stayed local
and synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine component_selection --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine raw_component --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine component_subset_plan_fails_exact_when_native_component_decode_is_unavailable --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_capability_report_lists_images_without_decoding --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest CCITT component-selection gates

These gates were run after the image decode planner stopped treating the
trivial single-component CCITT selection as unavailable. A 1 bpc DeviceGray or
ImageMask CCITT image now reports `component_decode: Native` for explicit
`Components([0])` because it is equivalent to all-component decode, while
invalid component indexes still fail exact planning with
`ComponentSelectionUnavailable`. This does not add nontrivial CCITT component
subsets, region APIs for other codecs, corpus validation, or benchmark evidence.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine ccitt_single_component_selection_is_trivial_native --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ccitt_component_selection_rejects_invalid_known_component_index --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_reference_capability_reports_shape_aware_raw_and_ccitt_windows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest codestream tile image decode capability gates

These gates were run after codec-native codestream tile decode stopped being an
implicit JPX gap. `ImageDecodeCapabilityReport` now exposes `tile_decode`, and
the document-level capability report publishes `native_tile_decode_count`.
Current JPEG/JPX adapters do not expose native codec tile decode, so JPX plans
report `CodestreamTileDecodeUnavailable` rather than implying support through
renderer tile scheduling or source-region cache identity. Guarded JPX
target-resolution reduction remains available where already implemented;
JPX ROI/region/progressive pixel output and native codestream tile pixel output
remain incomplete. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication
was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine jpx_plan_reports_full_decode_only_capability --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine jpx_downscale_plan_reports_native_reduction_capability --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_capability_report_lists_images_without_decoding --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| scoped `git diff --check` over codestream tile capability/report files | 0, CRLF normalization warnings only |

## Latest progressive image-decode failed-state gates

These gates were run after the progressive image-decode lifecycle gained an
explicit `Failed` terminal state and a binding-facing `fail` JSON action.
`ProgressiveImageDecodeSession::fail` releases retained state, reports
`full_decode_required: false`, and `continue_decode` preserves failed sessions
as terminal before consulting future native-progressive capability. The SDK
envelope now accepts `"fail"` in lifecycle action lists, so render/decode
failure cleanup can be represented through the same Rust SDK, C ABI/header,
Python, WASM, .NET, Java, CLI, and server JSON surfaces without pretending
native progressive pixel continuation exists. This stayed local and synthetic;
no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo test -p wellfriendpdf-engine progressive_image_decode_pause_resume_cancel_close_release_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_image_decode_continue_preserves_terminal_states --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_image_decode_lifecycle_report_envelope --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| scoped `git diff --check` over progressive image-decode failure lifecycle/report files | 0, CRLF normalization warnings only |

## Latest progressive image-decode document-close gates

These gates were run after document close became a first-class progressive
image-decode lifecycle action rather than an implied ordinary close. The shared
`ProgressiveImageDecodeReport` now carries `release_reason`, and
`ProgressiveImageDecodeSession::close_for_document_close` releases retained
decoder state, reports phase `document_close`, and preserves that reason across
continue-after-close terminal reports. The SDK JSON lifecycle envelope accepts
`"document_close"` (and normalized hyphenated spelling through the existing
action normalizer), so Rust SDK, C ABI/header, Python, WASM, .NET, Java, CLI,
and server JSON callers all use the same source-level document-owned cleanup
boundary. This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_image_decode --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_image_decode_lifecycle_report_json --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-capi --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-capi --lib --jobs 1 -- -D warnings` | 0 |

## Latest retained exactness regression-gate cleanup

The explicit renderer marker scan found a stale contract exactness test that
still expected HighQuality compositing with `Compatibility` exactness to publish
a canonical raw fallback for an unsupported retained display list. Current
source already refuses that path as a typed unsupported retained-replay error.
The test now asserts the active fail-closed behavior and verifies the message is
not a `HighQualityExact` refusal, preserving the independent exactness policy
distinction without reintroducing retained-to-immediate fallback.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine exactness_policy_is_honored_independent_of_compositing_mode --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest visual-normalization pixel-policy gates

These gates were run after the fixture-scale visual-reference normalization
harness moved background, grayscale, dimension, packed raw byte-order, and
structured font-environment metadata from passive policy capture into active
canonical-pixel/context normalization.
Explicit render-context RGBA backgrounds are now composited into the
straight-alpha canonical surface, `grayscale=true` converts RGB channels to
canonical luma while preserving alpha, declared dimensions are enforced after
EXIF/render-context rotation as exact comparison bounds rather than treated as
resize hints, and little-endian packed raw pixels are byte-swapped per pixel
before channel interpretation. Structured `font_environment` sidecars now
canonicalize to a stable string and SHA-256 fingerprint so equivalent manifests
with different JSON key order, case, or Windows path separators do not create
false `font_fallback` classifications, while real fallback/provider changes
remain classified. The normalization report and manifest now expose
`expected_size`, `background_rgba_applied`, `grayscale_applied`,
`byte_order_applied`, and `font_environment_fingerprint` through the context
policy. This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `python -m pytest tools/renderer-visual-diff/test_visual_normalization.py -q` | 0 |
| `python -m py_compile tools/renderer-visual-diff/visual_diff.py` | 0 |
| `python -m json.tool tools/renderer-visual-diff/normalization-manifest.json` | 0 |
| scoped `git diff --check` over visual-normalization harness/report files | 0, CRLF normalization warnings only |

## Latest reverse-byte-order caller-surface SIMD gates

These gates were run after caller-owned reverse-byte-order rows stopped being
limited to the scalar encoder when an existing channel-conversion kernel could
produce the pre-reversed row. `render-simd` now exposes a guarded
`reverse_4byte_words_in_place` helper with scalar oracle coverage and a
x86 AVX2/SSE2, ARM NEON, and wasm32+`simd128` kernels for RGBA/BGRA-sized
words. The core contract row encoder routes byte-reversed RGB/BGR rows through
the opposite channel-order kernel, routes byte-reversed RGBA/BGRA rows through
the existing straight/opaque/premultiplied/grayscale conversion kernels followed
by the reverse-word helper, and on native hosts can scalar-convert the row once
before using the native reverse-word helper when the conversion helper declines. If
any helper declines, the exact scalar row encoder still overwrites the
destination. Runtime capabilities now name the reverse-byte-order caller-surface
routing. This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-render-simd reverse_4byte --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_row_encoder_honors_reverse_byte_order --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_row_encoder --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --all-targets --jobs 1` | 0 |
| `$env:RUSTFLAGS='-C target-feature=+simd128'; cargo check -p wellfriendpdf-render-simd --target wasm32-unknown-unknown --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| scoped `git diff --check` over reverse-byte-order caller-surface source/report files | 0, CRLF normalization warnings only |

## Latest native alpha-mask SIMD gates

These gates were run after `render-simd` stopped returning scalar-only on
native x86/x86_64 builds for opaque-destination alpha/glyph-mask rows.
`blend_alpha_mask_opaque_destination` now dispatches to an SSE2 row kernel when
available, computes the same rounded `color.a * mask / 255` effective alpha as
the scalar glyph/image-mask compositor, forces destination alpha to 255, uses
the scalar oracle in debug builds, and keeps scalar tails for uneven row
lengths. The wasm32+`simd128` path and scalar fallback remain unchanged. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd blend_alpha_mask_opaque_dst_public_uses_sse2_when_available --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest native mixed-destination alpha-mask SIMD gates

These gates were run after `render-simd` stopped returning scalar-only on
native x86/x86_64 builds for normal Compat alpha/glyph-mask rows over mixed
destination alpha. `blend_alpha_mask_normal` now dispatches to an SSE2
four-pixel load/store row path when available, keeps the exact scalar
floating-point source-over byte contract for the per-pixel reciprocal math,
uses scalar debug oracles, and keeps scalar tails for uneven row lengths. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd blend_alpha_mask_normal_public_uses_sse2_when_available --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd sse --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest native caller-surface copy SIMD gates

These gates were run after native x86/x86_64 builds stopped declining simple
caller-surface RGBA copy and opaque-alpha rows. `render-simd` now dispatches
`copy_rgba` and `rgba_to_opaque_rgba` to SSE2 row kernels when available,
preserving the scalar byte contract, forcing only the alpha byte for opaque
RGBA output, using scalar debug oracles, and keeping scalar tails for uneven
row lengths. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication
was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd rgba_copy_and_opaque_public_use_sse2_when_available --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest native premultiply SIMD gates

These gates were run after native x86/x86_64 builds stopped declining
premultiplied caller-surface RGBA/BGRA rows. `render-simd` now dispatches
`premultiply_rgba` and `premultiply_bgra8` to SSE2 row kernels when available,
using the same byte-exact div255 rounding as the scalar contract, reordering
RGBA words to BGRA after premultiplication for BGRA output, retaining scalar
debug oracles, and keeping scalar tails for uneven rows. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd premultiply_public_uses_sse2_when_available --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest native BGRA conversion SIMD gates

These gates were run after native x86/x86_64 builds stopped declining BGRA
caller-surface channel conversion rows. `render-simd` now dispatches
`rgba_to_bgra8` to an SSE2 row kernel when available, preserving both straight
alpha and forced-opaque alpha semantics, reusing the scalar oracle in debug
builds, and keeping scalar tails for uneven rows. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd rgba_to_bgra_public_uses_sse2_when_available --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd sse2 --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest native SSSE3 RGB/BGR conversion SIMD gates

These gates were run after native x86/x86_64 builds stopped declining simple
RGB/BGR caller-surface channel rows and RGB image-span expansion rows when
SSSE3 is available. `render-simd` now dispatches `rgba_to_rgb8`,
`rgba_to_bgr8`, and `rgb8_to_opaque_rgba` to SSSE3 byte-shuffle kernels with
scalar debug oracles and scalar tails for uneven rows. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd rgb_bgr_channel_public_uses_ssse3_when_available --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd sse --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest native SSSE3 grayscale SIMD gates

These gates were run after native x86/x86_64 builds stopped declining Gray8,
grayscale RGB/RGBA/BGRA expansion, and premultiplied grayscale alpha-bearing
contract rows when SSSE3 is available. `render-simd` now dispatches the
grayscale helpers to guarded SSSE3 luma kernels, preserving the exact
`77/150/29` luma contract, forced-alpha semantics, scalar debug oracles, and
scalar tails. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication
was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd grayscale_public_uses_ssse3_when_available --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd sse --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest native SSSE3 backend reporting gates

These gates were run after `SimdBackend` and `PixelCompositorBackend` gained
an explicit `ssse3` backend value. `active_backend`,
`pixel_compositor_backend`, and detected hardware backend reporting can now
surface SSSE3-capable hosts separately from SSE2-only hosts while preserving
the existing scalar-reference guard. This stayed local and synthetic; no PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd sse --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd reverse_4byte --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest native unpremultiply SIMD gates

These gates were run after native x86/x86_64 `unpremultiply_rgba` stopped
returning scalar-only and gained an SSE2-dispatched four-pixel load/store row
path with scalar-equivalent reciprocal conversion, alpha-zero handling, scalar
tails, and debug scalar-oracle checks. This stayed local and synthetic; no PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd unpremultiply_public_uses_sse2_when_available --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd sse --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest ScalarReference caller-surface row gates

These gates were run after native caller-surface row helpers became active for
more x86/x86_64 layouts. `engine.rs` now routes caller-owned contract row
encoding through `encode_contract_row_for_backend`; `BackendSelection::ScalarReference`
zero-fills the destination row and calls the scalar row loop directly, so
straight/opaque/premultiplied and byte-reversed caller-owned output does not
re-enter the native SIMD helpers after the render-time scalar guard drops. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine scalar_reference_contract_row_encoder_uses_scalar_loop --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_scalar_reference_backend_uses_scalar_compositor --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |

## Latest packed replay pre-resolved resource gates

These gates were run after `RenderStatePlanAdapter` stopped performing
replay-time resource-name fallback for packed Image XObject, Form XObject,
named shading, ExtGState, font, non-device named color-space, named pattern,
and `/OC` marked-content property descriptors. Active public page rendering
already compiles retained plans with page resources; this slice makes the
replay layer enforce that invariant too, so descriptors compiled or constructed
without pre-resolved handles/objects/dictionaries now record typed fatal
retained-replay refusals instead of calling `handle_do`, `handle_sh`, the page
ExtGState/font/color-space/pattern/Properties maps, or later font-resource
resolution during replay. Built-in DeviceGray/DeviceRGB/DeviceCMYK/Pattern
color-space names remain valid without resource objects.
`GraphicsStateDescriptor::to_content_operation` remains only a descriptor
round-trip/diagnostic helper and is not part of the active packed replay path.
This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_replay_refuses_unresolved_resource_descriptors --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_replay_refuses_unresolved_state_resource_descriptors --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine resource_invocation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine typed_state_descriptor_round_trips_to_content_operation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest active shading function-array gates

These gates were run after active raster shading stopped rejecting well-formed
PDF `/Function` arrays. `render/function.rs` now exposes
`eval_function_or_array_n` and `validate_function_or_array_shape`, accepting an
array of one-output component functions while rejecting empty, oversized,
malformed, or multi-output component arrays. `render/shading.rs` uses the same
array-aware evaluator for Type 1, axial/radial, and mesh shading sampling, and
`render/page_renderer.rs::validate_shading_dictionary_for_paint` validates the
array shape and sampled output before paint. Malformed arrays still fail typed;
well-formed DeviceRGB component arrays now render through the active CPU
shading path instead of being refused as unsupported. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine function_array --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --test shadings axial_shading_function_array_paints_component_gradient --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine malformed_active_shading_dictionary_returns_typed_refusal --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| scoped `git diff --check` over active shading function-array source files | 0, CRLF normalization warnings only |

## Latest render-contract color-scheme and diagnostic gates

These gates were run after the active CPU render-contract path stopped treating
color-scheme policy as an opaque semantic-policy refusal. `engine.rs` now
post-processes the canonical RGBA surface for `ColorScheme::Dark` by inverting
RGB while preserving alpha, and for `ColorScheme::ForcedMonochrome` by using
the same deterministic contract luma weights as caller-owned grayscale row
encoding. Both policies feed the same bytes into canonical and caller-owned
outputs. Other unsupported semantic policies remain typed refusals, but the
refusal now names the exact divergent fields such as `backend` instead of
hiding them behind a generic renderer limitation. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication
was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine contract_forced_monochrome_color_scheme_posts_processes_surface --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_dark_color_scheme_inverts_surface_rgb --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_reports_unsupported_semantic_fields_by_name --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest render-contract subpixel smoothing gates

These gates were run after the active CPU render-contract path stopped treating
`SmoothingPolicy::Subpixel` and `subpixel_text` as unsupported semantic fields.
`engine.rs` now accepts `text_smoothing`, `image_smoothing`, `path_smoothing`,
and `subpixel_text` subpixel policy values at the contract boundary, passes
`subpixel_text` into retained plan identity, active render state, annotation
replay, SMask groups, and transparency/Form child render states. The later
research-hybrid backend slice also removes `ResearchHybrid` from the unsupported
backend diagnostic. Cached and
direct ordinary glyph-mask fills now use `PixelBuffer::blend_lcd_alpha_mask_strided`
for RGB per-channel LCD-style coverage when text subpixel policy is active;
high-quality/blend/knockout text mask cases reuse the existing grayscale alpha
compositor rather than silently changing blend semantics. Image `Subpixel`
continues through the PDF interpolation path, and path `Subpixel` continues
through the existing device-space subpixel antialias coverage path. This stayed
local and synthetic; no PDF corpus, benchmark, competitor comparison, VPS,
deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine contract_subpixel_text_uses_lcd_glyph_mask_policy --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine contract_reports_unsupported_semantic_fields_by_name --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest WASM premultiply and grayscale SIMD gates

These gates were run after `render-simd` stopped extracting wasm32+`simd128`
premultiply-BGRA, RGB8-to-opaque-RGBA, and grayscale expansion groups into
temporary byte arrays for per-pixel scalar channel work. `crates/render-simd/src/lib.rs`
now shares the exact vector premultiply-RGBA lane helper between RGBA and BGRA
output, emits BGRA with a SIMD channel shuffle, expands RGB8 image rows to
opaque RGBA with bounded zero-extending loads plus a SIMD shuffle and alpha vector, computes grayscale through a
shared four-pixel SIMD luma helper, and uses SIMD shuffles for GrayRGB,
GrayRGBA, GrayBGRA, and premultiplied GrayRGBA rows before scalar tails. GrayRGB,
RGB, and BGR wasm 12-byte row groups and native SSSE3 RGB/BGR row groups store
through bounded 64-bit plus 32-bit lane stores instead of temporary staging
arrays, and wasm Gray8 stores through a bounded 32-bit lane store instead of a
temporary vector staging array. Native SSSE3 RGB8-to-opaque-RGBA reads its 12-byte source group through
bounded 64-bit plus 32-bit lane loads instead of a temporary source staging array.
Native SSSE3 grayscale rows keep four-pixel luma in a SIMD register, expand
GrayRGB and GrayRGBA with byte shuffles, and route premultiplied GrayRGBA through
the shared SSE2 premultiply group helper instead of scalar expansion arrays.
Debug
scalar-oracle checks remain on the public guards. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo check -p wellfriendpdf-render-simd --target wasm32-unknown-unknown --jobs 1` with `RUSTFLAGS='-C target-feature=+simd128'` | 0 |
| `cargo test -p wellfriendpdf-render-simd premultiply --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd gray --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd rgb --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-engine contract_row_encoder --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo fmt --all` | 0 |

## Latest active non-RGB transparency group gates

These gates were run after active Form transparency groups stopped accepting
alpha/backdrop-observable DeviceGray or DeviceCMYK group color spaces as if the
RGB compositor implemented full non-RGB group-space blending. DeviceRGB/default
group spaces keep the existing active path, while DeviceGray/DeviceCMYK active
Form groups now render only when the existing conservative Form-stream scanner
proves an opaque normal inert subset. Alpha-bearing DeviceCMYK groups fail typed
before offscreen rendering instead of silently blending in RGB group space. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_device_cmyk --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transparency_group_color_space --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest Type 3 retained resource gates

These gates were run after adding focused synthetic coverage for resource-backed
Type 3 CharProc retained replay. `render/page_renderer.rs` now has direct tests
that a Type 3 CharProc can paint an Image XObject, a Form XObject, and a named
shading through the font resource dictionary without falling back to ordinary
font substitution or unsupported retained replay. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine type3_charproc_renders_resource_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest print separation policy contract gates

These gates were run after `OverprintPolicy::PreserveSeparations` stopped being
conditionally accepted by `validate_print_profile_prepress` when native CMM is
compiled. The active render-contract target is bounded RGB/gray raster output;
Separation and DeviceN plate data remains exposed through the prepress plate
report rather than being represented as separation-preserving output channels.
The contract validator now fails closed for `PreserveSeparations` even when
`ColorManagementPolicy::NativeLittleCms` is requested, while `OverprintPolicy::Preview`
and RGB ordered halftone remain active supported raster policies. Page-renderer
annotation tests now keep real raster coverage to Display/Print and assert Proof
visibility at the print-profile flag boundary so proof visibility is not tested
through an invalid non-proofed raster shortcut. This stayed local and synthetic;
no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine preserve_separations --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine print_profile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine overprint_preview_policy_is_accepted_by_contract_renderer --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest deterministic font-provider mapping gates

These gates were run after the default bundled provider began implementing the
deterministic system-mapping tier before generic bundled fallback. Stable
Arial/Calibri/Segoe UI/Tahoma/Verdana/Helvetica Neue aliases map to
Helvetica-compatible bundled faces, Times New Roman/Georgia/Cambria/Constantia/
Garamond aliases map to Times-compatible faces, and Courier New/Consolas/
Inconsolata/Lucida Console aliases map to Courier-compatible faces. The selected
matches report `FontProviderSource::DeterministicSystemMapping`,
`reason: DeterministicSystemMapping`, and
`resolution_source: "deterministic_system_mapping"`; caller-registered matches
still report `resolution_source: "user_registered"`. Symbol-family names and
Symbolic descriptor hints stay on the coverage fallback path and are not labeled
as deterministic sans mappings. Generic bundled fallback remains refused by
high-quality/exact contracts, and external registered-font runtime validation
remain incomplete rather than claimed. The actual render-policy path now accepts
non-embedded deterministic aliases such as `ArialMT` under HighQuality while a
truly generic non-embedded font such as `SomeCorporateSans` still refuses as
generic bundled fallback, and symbolic descriptor fallback remains
coverage-only. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication was
used. The compile/test/clippy gates below used
`CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_INCREMENTAL=0` after the default debug
profile rustc process exited without a source diagnostic under the local 4 GiB
resource cap.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine deterministic_system_aliases_report_mapping_source --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine symbolic_system_aliases_do_not_report_system_mapping --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine font_replacement_selection_labels_external_provider_sources --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_render_accepts_deterministic_system_font_mapping --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine high_quality_render_refuses_generic_bundled_font_substitution --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine symbolic_descriptor_routes_generic_compat_fallback_to_coverage_face --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest transaction shared-resource alias gates

These gates were run after the render-invalidation bridge began recognizing a
broader set of nested shared-resource and document-structure write-set aliases
from binding/server transaction envelopes. The collector now accepts XObject,
ColorSpace, ExtGState/graphics-state, resource dictionary, tiling-pattern,
appearance-stream, widget/form/signature appearance, Mask/SMask/image SMask,
transparency group, OCG/OCMD/OC config, Properties, page/page tree, catalog,
metadata/XMP, Names/name tree/page labels, StructTree, ParentTree, RoleMap,
shared-resource, resource-write-set, and transitive write-set fields, plus
structured per-resource object/generation keys. Known refs merge into mapped
source IDs and source-cache markers; unknown refs still force the existing
conservative reset. This narrows more shared-resource edits without weakening
fail-closed behavior for unclassified dependencies. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib nested_write_set_collector_accepts_document_structure_and_group_aliases --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib transaction_invalidation --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest print separation surface validation gates

These gates were run after print/prepress validation became surface-aware and
the production prepress plate-report path began validating against the
`SeparationFramebuffer` output surface before dispatching page content. Normal
RGB/gray render contracts still fail closed for `OverprintPolicy::PreserveSeparations`,
but the prepress N-channel surface now validates that policy as the explicit
separation-preserving source path. RGB ordered halftone plus preserved
separations remains refused, and proof output still keeps the native-CMM
requirement. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication was
used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine print_profile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine prepress_plate_report --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest owned backend document arena gates

These gates were run after the render-view source boundary gained an explicit
owned CPU backend document arena. `BackendDocumentPlanArena` now retains the
compiled `RenderPlan` for every page in the current source revision, exposes
per-page source/resource identities, and returns `BackendDocumentPlanArenaReport`
aggregate counts through `sdk::backend_document_plan_arena_report_json`. Ordinary
`document_views_report` remains lazy and does not construct the arena unless the
render view asks for it. This stayed local and synthetic; no PDF corpus,
benchmark, competitor comparison, VPS, deployment, release, tag, or package
publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine backend_document_plan_arena --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine "document_views_report" --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest broad workspace gate refresh

These gates were run after the SVG/PS tint-transform group-colour Form slice,
the later source-continuation slices, and the owned backend document arena
tests were present in the current dirty tree. The first broad Clippy pass found
one test-only `len_zero` lint in `crates/engine/src/sdk.rs`; the assertion now
uses `is_empty()` without changing the test condition. The rerun completed
cleanly under the same local target/temp directory and one-job Cargo limit. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 1, fixed `clippy::len_zero` in `crates/engine/src/sdk.rs` |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest .NET, Java, and WASM binding gates

These gates were run after the broad workspace source gates. The first .NET
runtime test attempt compiled the managed projects but failed because
`wellfriendpdf_capi.dll` was not yet built or on the DLL search path. Building
`wellfriendpdf-capi` under the task-local Cargo target produced the native DLL,
and rerunning the .NET smoke suite with that directory prepended to `PATH`
passed. Java was validated without Gradle or Maven because neither command is
installed on `PATH`; the Java 25 compiler is present, so the main binding,
non-JUnit smoke, and package smoke were compiled directly with preview enabled
into the task-local `.work` directory and then run against the same native DLL.
The WASM binding source and render-SIMD `simd128` source paths were also checked
against `wasm32-unknown-unknown`; the SIMD check was rerun by itself after an
earlier parallel attempt was cancelled while waiting on Cargo's build lock to
preserve the sequential heavy-toolchain policy.
This stayed local and small-fixture only; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `dotnet test bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --configuration Debug` | 1, native DLL missing from loader path |
| `cargo build -p wellfriendpdf-capi --jobs 1` | 0 |
| `dotnet test bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj --no-restore --configuration Debug` with `.work/.../cargo-target/debug` on `PATH` | 0, 15 passed |
| `javac --release 25 --enable-preview -encoding UTF-8` for `WellfriendPdf.java`, `WellfriendPdfSmokeTest.java`, and `PackageSmoke.java` | 0 |
| `java --enable-preview --enable-native-access=ALL-UNNAMED -cp .work/.../java-classes io.wellfriendpdf.WellfriendPdfSmokeTest --contract-builder-only` | 0 |
| `java --enable-preview --enable-native-access=ALL-UNNAMED -cp .work/.../java-classes io.wellfriendpdf.WellfriendPdfSmokeTest` with `WELLFRIENDPDF_NATIVE_LIBRARY=.work/.../wellfriendpdf_capi.dll` | 0 |
| `java --enable-preview --enable-native-access=ALL-UNNAMED -cp .work/.../java-classes io.wellfriendpdf.packagesmoke.PackageSmoke crates/engine/tests/fixtures/tracemonkey.pdf` with `WELLFRIENDPDF_NATIVE_LIBRARY=.work/.../wellfriendpdf_capi.dll` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-render-simd --target wasm32-unknown-unknown --jobs 1` with `RUSTFLAGS='-C target-feature=+simd128'` | 0 |

## Latest nested retained active-resource stack gates

These gates were run after nested retained replay cleanup was tightened for
Form XObjects, tiling-pattern tiles, annotation appearances, and Type 3
CharProcs. Nested retained replay now restores the active pre-resolved resource
side stack along with the active font/color-space/pattern handles when an inner
packed plan records a fatal error after `q` and stops before a balancing `Q`.
Annotation appearances also clear active pre-resolved resources when they reset
to a fresh appearance graphics state, then restore the outer handles on exit.
Type 3 retained CharProc replay now returns `false` when retained replay records
a fatal render error instead of reporting the CharProc as successfully rendered.
This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib active_resource_stack_after_inner_refusal --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib packed_plan_restore_restores_pre_resolved_active_font_resource --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest progressive image full-decode reporting gates

These gates were run after the progressive image-decode lifecycle report stopped
using capability availability as a proxy for the actual decode shape. A plan now
reports `full_decode_required` whenever it requests the full source region at
full resolution and no native progressive decoder is available, even if the same
codec family can window other clipped shapes. This fixes raw/CCITT-style
under-reporting without claiming new native progressive, ROI, or codestream-tile
decode support. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication was
used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib progressive_full_raw_plan_reports_full_decode_required_even_with_window_capability --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib progressive_image_decode_session_reports_full_decode_required --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest transaction write-set alias gates

These gates were run after the render-invalidation nested write-set collector
was extended for prompt-level shared-resource vocabulary that was still missing
from the source alias table. Binding/server/report envelopes can now surface
form-field, form-value, AcroForm, optional-content state, Type 3 glyph/CharProc,
font-descriptor, CMap, image-mask, soft-mask, content stream, page
content/program, page resource dictionary, render-relevant structure, output
intent, ICC profile, transfer function, halftone, print profile, and
render-contract object refs through explicit `changed_*_refs` aliases or
matching structured object/generation keys. Known refs still map to source cache
markers; unknown refs still force conservative reset. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nested_write_set_collector --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest packed inline-image color-space pre-resolution gates

These gates were run after packed inline-image descriptors gained a pre-resolved
named color-space payload. Resource-aware packed compilation now resolves
well-formed inline image `/ColorSpace /Name` or `/CS /Name` operands through
`PageResources::color_spaces`; missing named color-space resources become typed
packed compile refusals with usage `inline image`. Built-in device spaces still
need no resource object, including inline abbreviations `/G`, `/RGB`, and
`/CMYK`; image masks still do not require `/ColorSpace`, and
malformed inline image parameters remain handled by the existing renderer
fail-closed parser. Packed replay now passes the resolved color-space object to
the inline image paint path instead of doing a resource-name lookup there. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib inline_image_descriptor --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest Type 3 retained CharProc subplan cache gates

These gates were run after Type 3 full CharProc replay stopped compiling the
retained packed subplan on every glyph render. `render/page_renderer.rs` now
keeps a bounded `Type3ProgramCache<Type3RetainedCharProcPlan>` beside the
parsed CharProc and geometry caches. Retained subplan keys include the parsed
glyph identity, merged resource fingerprint, active retained render-contract
fingerprint, and any indirect CharProc source marker, so display/print,
resource, tile-local viewport, and source-revision differences do not collide.
Negative entries preserve exact retained-plan refusal reasons, child render
states absorb retained-plan cache entries back into the parent, memory-pressure
accounting includes the packed subplan, and source/page invalidation prunes
matching retained Type 3 entries. The follow-up source-identity slice also salts
active parsed CharProc, Type 3 geometry, Type 3 mask, and rendered-colour-glyph
cache keys with canonical document revision plus an indirect CharProc
`type3-program` source marker where available, and prunes those entries from
source-scoped transaction invalidation. This stayed local and synthetic; no PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib type3_retained_charproc --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib type3_program_cache --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib type3_charproc_render_cache_key --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib type3_source_pruning --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib render_document_cache_budget_evicts_type3_parsed_program_caches --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest CCITT image decode capability exactness gates

These gates were run after the image decode planner stopped advertising native
CCITT source-window decode for terminal monochrome streams whose declared image
shape cannot be satisfied by that decoder. `render/image_decode_planning.rs`
now reports `MonochromeTerminalShapeUnsupported` unless the image is 1 bpc and
declared as an image mask, `DeviceGray`, or `G`. This aligns plan-time exact
refusal and document capability reports with the decoder's existing
fail-closed terminal-color-space guard, so exact rendering does not fake
region-decode support for CCITT images declared as RGB/CMYK. Runtime capability
reporting names the same terminal-shape boundary. This stayed local and
synthetic; no PDF corpus, benchmark, competitor comparison, VPS, deployment,
release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine ccitt_plan --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |

## Latest ExtGState paint-dependent semantic gates

These gates were run after well-formed `/SA`, `/AIS`, and `/TK` ExtGState
values stopped being rejected as metadata-only unsupported flags. The active
graphics state now records those values across `q/Q`, retained `DrawState` and
packed plan fingerprints include them, and packed compilation routes visible
paint that observes `/AIS true` or `/SA true` out of pure vector replay. Active,
retained, and packed replay now fail typed at the paint boundary for visible
stroke-adjustment, alpha-source, or disabled-text-knockout semantics, while
non-observing paths such as `/SA true` fill-only paint still render. The same
focused gate now covers retained visible text refusals for `/TK false`,
`/AIS true`, and stroked-text `/SA true`, not just direct text rendering. This
stayed local and synthetic; no PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine extgstate_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine try_apply_ext_gstate --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_vector_plan --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |

## Latest progressive center-visible scheduling gates

These gates were run after progressive tile ordering stopped treating the
center-visible tile as only a viewer-queue label. `render/progressive.rs` now
uses an explicit center-visible schedule rank before other visible tiles during
initial scheduling and rescheduling, while preserving visible, near-visible,
adjacent-page, and background queue reporting. This stayed local and synthetic;
no PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine center_visible_tile_orders_before_other_visible_tiles --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine step_report_includes_adjacent_page_prefetch_and_viewer_queue_preview --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |

## Latest PostScript named-color exact shading gates

These gates were run after PostScript vector shading replay gained a separate
exact named-color sidecar for one-component Separation or DeviceN shadings. The
PostScript sink now emits native `[/Separation ...]` or single-colorant
`[/DeviceN ...]` color spaces when both the source shading tint function and the
color-space tint transform serialize as finite Type 2/Type 3 functions. SVG
nonlinear tint transforms and multi-input/calculator DeviceN cases remain
fail-closed. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication was
used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine nonlinear_tint --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine multi_input --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine resource_separation_axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine resource_devicen_axial_shading --test regional_vector_fallback --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_rgb_shading_function --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ps_cmyk_shading_function --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-engine --test regional_vector_fallback --jobs 1 -- -D warnings` | 0 |

## Latest display-list raw-operation storage and image capability gates

These gates were run after retained display-list compilation stopped keeping
cloned raw `ContentOperation` values for path assembly and inline-image pending
state. Path assembly now tracks a typed count, inline-image sequence state keeps
only the pending parameter operands, and retained byte accounting measures typed
payloads instead of building temporary raw operation vectors. A source guard
confirms the removed raw-operation storage patterns are absent from
`render/display_list.rs`.

The same pass split static image capability "full-decode only" classification
from progressive full-source work reporting. Document-level image capability
reports now count raw/CCITT images with native region or component capability as
not full-decode-only, while progressive image sessions still report
`full_decode_required` for nonterminal full-source, non-native-progressive work.
This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib image_decode_capability_report --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib progressive_full_raw_plan_reports_full_decode_required_even_with_window_capability --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib progressive_image_decode_session_reports_full_decode_required --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib native --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `rg -n "path_ops:\s*Vec<ContentOperation>|pending_inline:\s*Option<ContentOperation>|pending_inline_params:\s*Option<ContentOperation>|estimate_ops_bytes|op\.clone\(\)" crates/engine/src/render/display_list.rs` | 1, expected no matches |

## Latest image DecodeParms cache-identity gates

These gates were run after image decode cache identity gained an explicit
`DecodeParms` fingerprint. `render/image_decode_planning.rs` now carries the
field through `ImageMetadata`, `ImageContractState`, and
`ImageDecodeCacheKey::to_cache_string`; active inline-image planning fingerprints
the normalized inline `/DecodeParms` operands, and Image XObject planning
fingerprints `/DecodeParms` or `/DP` from the stream dictionary. This prevents
otherwise-identical image payload/filter/cache inputs with different predictor,
CCITT, JBIG2, or other filter-parameter semantics from sharing decoded pixels.
This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib cache_key_includes_decode_params --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib cache_key_differs_for_decode_params --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib image_decode_planning --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest portable image decode cache-identity gates

These gates were run after decoded image cache identity was split from pure
page/tile placement. `render/page_renderer.rs::image_decode_cache_base_key` now
uses document revision, decoded source identity, and a decode-policy fingerprint
for render mode, backend, print/exactness policy, CMM intent/backend, overprint,
image smoothing, and resource-budget identity. It no longer salts decoded image
base keys with page number, viewport, tile origin, device transform, annotation
policy, form policy, or optional-content visibility. Inline-image base keys use
the same decode-policy fingerprint while still hashing the full inline payload.
The final `ImageDecodeCacheKey` produced by `render/image_decode_planning.rs`
continues to include the planned source region, target size, reduction level,
quality, component selection, target format, backend, decode array,
DecodeParms, image mask, soft mask, interpolation, and decode-policy identity.
This allows decoded resources to be reused across pages, Form invocations,
pattern cells, and repeated viewer rendering when the requested decoded pixels
are compatible, while incompatible regions/scales/policies still produce
separate final keys. This stayed local and synthetic; no PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication was
used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib image_decode_cache_key_uses_portable_decode_contract_identity --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib inline_image_decode_cache_key_hashes_full_payload_and_decode_policy --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib image_decode_cache_key --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib image_decode_planning --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest standalone display-list replay fail-closed gates

These gates were run after the public display-list CPU replay helpers stopped
silently skipping high-level native retained operations. The standalone
`render/display_list.rs::render_display_list` helper returns
`Result<PixelBuffer>`, refuses unsupported display lists, and returns a typed
`UnsupportedFeature` when a list contains native text, Image XObject, shading,
pattern path, inline-image, or Form XObject operations that require page-context
`RenderState` replay. The exported `replay_display_list` entry point now also
returns `Result<()>`, refuses unsupported lists, and preflights native high-level
operations against an explicit `RenderDevice::supports_native_high_level_ops`
capability before dispatching to device hooks. The state/native hook methods no
longer have default no-op or warning-only bodies, so a replay device must make
state handling explicit and any device that opts into native high-level replay
must provide explicit handlers. Vector-only display lists still render through
`CpuRenderDevice`, while high-level retained lists must use `PageRenderer`
display-list replay, which has the resource dictionaries,
font/image/shading/pattern handlers, optional-content state, and soft-mask
context required for correct native execution. This closes silent material
degradation paths in public helpers without adding immediate-render fallback.
This stayed local and synthetic; no PDF corpus, benchmark, competitor
comparison, VPS, deployment, release, tag, or package publication was used.

| Command | Exit code |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine --lib standalone_display_list_replay_refuses_native_high_level_ops --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib direct_display_list_replay_refuses_native_high_level_ops_on_default_device --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine --lib display_list --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |

## Latest SIMD alpha-fill lane gates

This source slice adds an explicit alpha-fill SIMD API for translucent solid
fills over opaque destinations. `crates/render-simd/src/lib.rs` now exposes
`fill_alpha_run`, which routes through the same guarded native/wasm
source-over kernels as `blend_normal_opaque_destination` and declines no-op or
opaque cases that are handled by existing fill paths. `render/buffer.rs` now
calls this named alpha-fill lane from the normal Compat opaque-destination
solid-fill fast path, and `runtime.rs` reports `alpha_fill_rows` in the
`cpu_simd_compositor` capability entry. This is a source/API closure for the
named alpha-fill SIMD category; it does not claim performance or full SIMD
category coverage.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |

## Latest SIMD clip/soft-mask fusion row gates

This source slice adds an explicit alpha-row product SIMD API for bounded
clip/soft-mask fusion. `crates/render-simd/src/lib.rs` now exposes
`multiply_alpha_rows`, with guarded native x86/x86_64 AVX2/SSE2 and wasm32
`simd128` lanes plus scalar-oracle debug checks. `render/buffer.rs`
`AlphaMask::fused_clip_window` now materializes the exact local clip row and
uses that helper before falling back to the scalar alpha product. `runtime.rs`
reports `clip_mask_fusion_rows` in the `cpu_simd_compositor` capability entry.
This closes a named SIMD row category for the existing fused alpha-window path;
it does not claim full SIMD category coverage or broader transparency closure.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_mask_fused_clip_window_wide_row_matches_scalar_product --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-render-simd public_kernels_match_scalar_or_cleanly_decline_unaligned_rows --lib --jobs 1 -- --nocapture` after adding AVX2 dispatch | 0 |
| `cargo check -p wellfriendpdf-render-simd --lib --jobs 1` after adding AVX2 dispatch | 0 |
| `cargo clippy -p wellfriendpdf-render-simd --lib --jobs 1 -- -D warnings` after adding AVX2 dispatch | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --tests --jobs 1 -- -D warnings` | 0 |

## Latest direct paint fusion row gates

This source slice routes direct image/glyph alpha-mask, solid-fill, and RGBA
fragment paint fusion through the same bounded alpha-row product helper used by
compact clip/soft-mask windows. `render/buffer.rs` now reuses row scratch,
copies or materializes the active paint alpha row, materializes
destination-local SMask and partial-clip opacity rows only when needed, and
calls `wellfriendpdf_render_simd::multiply_alpha_rows` before falling back to
the exact scalar `round(alpha * mask / 255)` product. This keeps
`blend_alpha_mask`, `fill_rect`, and `blend_rgba_pixels_at` on their existing
fused row compositors while adding named SIMD-backed routes for mask/solid/RGBA
+ SMask + clip products. `runtime.rs` reports
`image_glyph_mask_clip_smask_fusion_rows` and
`solid_rgba_clip_smask_fusion_rows` in the `cpu_simd_compositor` capability
entry. This does not claim universal blend-mode or transparency closure.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine paint_fusion_wide_row_matches_scalar_product --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine fuses_partial_clip_and_smask_into_row_path --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest transaction display/proof/prepress alias gates

This source slice expands transaction render-invalidation write-set extraction
for display, print, proof, and color-management profile changes carried by
binding/server/report envelopes. `render/transaction_invalidation.rs` now
accepts explicit refs and structured object/generation keys for
display/print/proof OutputIntent entries, display/proof/proofing profiles,
color-management policy/profile records, CMM profiles, Separation profiles, and
DeviceN profile aliases. The same collector now accepts rendering-intent,
transfer-function, halftone-screen, overprint, prepress policy, prepress plate,
ink/spot color, Separation/DeviceN plate, black-point compensation, black
generation, undercolor-removal, trapping, trap-network, image filter,
Decode/DecodeParms, interpolation, color-key-mask, image/SMask matte, and
JPEG/DCT/JPX/CCITT/JBIG2 parameter aliases. Known refs still map into canonical
source IDs and source-cache markers; unknown refs still take the conservative
reset path. This narrows another transaction
classification gap without claiming arbitrary shared-resource invalidation is
complete.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine nested_write_set_collector_accepts_display_proof_profile_aliases --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nested_write_set_collector_accepts_prepress_overprint_aliases --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine nested_write_set_collector_accepts_image_decode_parameter_aliases --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 45 tests |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest packed-plan optimizer gates

This source slice adds an explicit, reportable packed-plan optimizer boundary.
`PackedDisplayList` now records a `PackedPlanOptimizationReport` with source
operation count, emitted hot operation count, and folded duplicate state
operation count. The compiler folds identity `cm` no-op transforms and adjacent
duplicate idempotent absolute state setters whose typed
`GraphicsStateDescriptor` values are identical. Adjacent same-slot absolute
setters also replace the earlier setter when no paint, scope, save/restore, or
non-idempotent operation intervenes. A later local slice also composes adjacent
typed `cm` matrix concatenations with the active `concat_matrix(current,
previous)` order, and drops adjacent inverse matrix pairs whose composition is
identity. It does not fold non-adjacent or geometry-dependent `cm`
concatenation, relative text movement, text or marked-content scopes,
compatibility sections, save/restore, compile refusals, ExtGState overwrites, or
any paint/native operation. `BackendPlanArenaReport` and
`BackendDocumentPlanArenaReport` expose the source-operation,
folded-no-op-state, folded-duplicate-state, and folded-overwritten-state
counters so callers can audit whether the safe optimizer ran.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine packed_plan_folds_adjacent_duplicate_idempotent_state_descriptors --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_folds_identity_matrix_concatenation_as_noop_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_overwrites_adjacent_same_slot_state_setter --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_interns_repeated_vector_state_and_path_entries --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_folds_adjacent_matrix_concatenation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 12 tests |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest packed matrix-composition optimizer gates

This source slice narrows RV-10's transform folding gap. `PackedDisplayList`
detects adjacent typed `cm` state descriptors before paint and composes them
into one matrix descriptor using the same `concat_matrix(current, previous)`
order as active replay. It now also tracks a CTM slot inside no-paint state runs
so separated `cm` operators compose across independent state setters such as
line width, while separated inverse pairs collapse to no referenced CTM hot op.
The optimizer does not cross paint, geometry, clip, scope, save/restore, compile
refusal, ExtGState side-effect, or resource boundaries.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_folds_adjacent_matrix_concatenation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine non_adjacent_matrix --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo test -p wellfriendpdf-engine non_adjacent_inverse_matrix --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 22 tests |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest backend-plan resource-arena report gates

This source slice makes typed packed payload coverage auditable from the
existing backend-plan report surface. `BackendPlanArenaReport` now carries
`resource_arena_entries`, and `BackendDocumentPlanArenaReport` carries
`total_resource_arena_entries`, with counters derived from typed
`NativeDescriptor` payloads for fonts, image/Form XObjects, Type 3 metrics,
patterns, shadings, transparency groups, a descriptor-level appearance slot,
color spaces, ExtGStates, Properties resources, and inline images. Annotation
and widget appearance program cache telemetry remains separate in
`RenderDocumentCache`.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine backend_plan_arena_report_exposes_resource_specific_payload_counts --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine backend_plan_arena --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 5 tests |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest contract-field and spatial scratch gates

This source slice adds a source-owned `RenderContractFieldEffect` registry beside
`RenderContract`, exposes it through `RenderContractTelemetryReport`, and guards
it against every serialized schema-v1 contract field. It also closes the linear
spatial-index scratch proof so `RenderSpatialIndex::query_into` now has focused
linear, grid, and BVH reuse coverage. The packed vector state/path arena guard
proves repeated identical vector paints share path/state arena entries and hot-op
IDs. These changes do not claim resource-specific backend arenas or external
runtime matrix validation complete.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine field_effect_registry_covers_every_serialized_contract_field --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_contract_telemetry_report_exposes_cache_counters --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine spatial_index_query_into_reuses_output_buffer --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 3 tests |
| `cargo test -p wellfriendpdf-engine packed_plan_interns_repeated_vector_state_and_path_entries --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 1, fixed fuzz-feature `TextShaper::shape` call to borrow `Cow<[u8]>` as `&[u8]` |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest non-adjacent packed-state optimizer gates

This source slice extends the packed-plan optimizer across no-paint state runs
for independent scalar/text setter slots. `PackedDisplayList` now tracks the
last emitted safe state slot until a paint, native operation, clip, save/restore,
scope marker, compile refusal, ExtGState side effect, dependent color-space/color
setter, or other non-idempotent state boundary appears. A later same-slot setter
inside that safe run replaces the earlier descriptor, while an identical later
setter is folded as a duplicate. Paint boundaries are guarded so visible output
state is not moved across rendering operations.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine packed_plan_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 15 tests |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest PixelBuffer active-clip borrow gates

This source slice removes concrete active-clip clones from the main
`PixelBuffer` row paint paths. `blend_alpha_mask_strided`,
`blend_rgba_pixels_at`, `fill_rect`, `composite_from`, and
`composite_from_at` now borrow the active `ClipMask` while splitting mutable
pixel-row writes into local data bindings where clip-run callbacks are used.
This narrows the RV-15 paint-boundary allocation gap for alpha-mask,
cached-RGBA, solid-fill, full-buffer composite, and offset-composite rows. It
does not remove concrete clip masks from raster-buffer installation or complete
full clip/mask fusion.

| Command | Exit |
|---|---:|
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_mask --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 19 tests |
| `cargo test -p wellfriendpdf-engine fill_rect --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 35 tests |
| `cargo test -p wellfriendpdf-engine composite_from --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 15 tests |
| `cargo test -p wellfriendpdf-engine blend_rgba_pixels_at --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 4 tests |
| `cargo fmt --all --check` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest dirty-region cache-plan merge gates

This source slice lets cache-owning callers merge page-space dirty regions into
a `RenderInvalidationCachePlan` before applying it. The plan method reuses the
existing caller-supplied viewport/grid conversion, deduplicates derived tiles,
and then leaves exact tile selection to the existing cache-application path.
This narrows page-level dirty-region invalidation when the caller has viewport
and tile-grid context; the parser still does not infer page geometry, DPI, or
tile size from an edit report.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine render_invalidation_plan_merges_dirty_region_tiles_before_apply --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 46 tests |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest transaction category-mutation alias gates

This source slice closes a category-specific transaction write-set gap in the
render-invalidation collector. Binding/server/report envelopes can now use
created, removed, deleted, added, updated, modified, or replaced prefixes for
the same renderer resource categories already covered by the canonical
`changed_*_refs` table. Examples include created image refs, removed Form
XObject refs, deleted optional-content config refs, added annotation appearance
refs, updated proof-profile refs, modified JPX parameter refs, and replaced
resource-dictionary refs, including structured objects whose object/generation
keys carry the same mutation prefix such as `createdImageObject` or
`removedFormXObjectGeneration`. The collector remains bounded to known renderer
write-set suffixes, so arbitrary similarly named objects outside a recognized
write-set field do not enter the invalidation graph. Known refs still map into
canonical source IDs/source-cache markers; unknown refs still trigger the
existing conservative reset path.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine nested_write_set_collector_accepts_category_specific_mutation_aliases --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 47 tests |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest transaction broad-write-set metadata gates

This source slice narrows render-invalidation plan parsing for rich transaction
write-set envelopes. Broad fields such as `writeSet`, `affectedObjects`, and
`dirtyObjects` now collect parseable PDF/source refs and structured
object/generation pairs, including `targetObject`/`targetGeneration`, while
ignoring ordinary metadata strings such as operation names, reasons, and
semantic labels. Explicit renderer ref fields such as `changedFontRefs` remain
strict: unknown non-empty strings in those fields still enter the unknown-ref
path and trigger the existing conservative reset. Parseable refs found through
broad containers are canonicalized to `N G R` before cache mapping.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine broad_write_set_collector_ignores_non_ref_metadata_strings --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine render_invalidation_plan_json_ignores_broad_write_set_metadata_strings --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 49 tests |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest renderer graph write-set alias gates

This source slice extends render-invalidation write-set normalization for
prompt-level renderer graph nodes that were not previously named directly.
Cache-plan and transaction envelopes can now report changed retained operations,
display operations, display lists, render resources, render sublists, Form/Type
3/pattern/appearance sublists, spatial entries/indexes, backend plans,
backend-plan cache entries, render cache entries, cache entries, and render
tiles through snake_case or binding-style camelCase fields. Mutation-prefixed
fields such as `createdBackendPlanRefs` reuse the existing known-category
prefix matcher. Known refs map into canonical source IDs and source-cache
markers; unknown refs still conservatively reset rather than publishing stale
tiles.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine nested_write_set_collector_accepts_render_structure_aliases --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine transaction_invalidation --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 53 tests |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest scoped renderer source-marker audit

After the renderer graph alias slice, the scoped source-marker audit over
renderer, image, font, SIMD, binding, and server source paths returned no
matches for the prompt's unresolved-marker set. The only prior hit was an
image-extraction server ZIP throughput note; it was reworded as an explicit
sequential CPU/memory policy and does not affect renderer replay behavior.

| Command | Exit |
|---|---:|
| `rg -n "TODO|FIXME|HACK|unimplemented!|todo!|not implemented|stub|solid fallback|immediate renderer|legacy renderer|allow\\(dead_code\\)" crates\engine\src\render crates\engine\src\images crates\engine\src\fonts crates\render-simd\src crates\wellfriendpdf-capi\src crates\wellfriendpdf-py\src crates\wellfriendpdf-wasm\src bindings\dotnet bindings\java crates\server\src -g "*.rs" -g "*.cs" -g "*.java"` | 1, no matches |

## Latest renderer concurrency/cache matrix gates

This source slice closes the RV-34 "no structured concurrency proof" report
gap at the public source/API level. `RuntimeCapabilityReport` now includes a
`renderer_concurrency_cache_matrix` surfaced through SDK/runtime JSON and the
server `/api/v1/capabilities` endpoint. The matrix records host/effective CPU
workers, max concurrent documents, per-document serial mutation policy,
work-stealing policy, renderer thread permit rows for page-program parsing,
image decode, tile render, font shaping, progressive publication, and document
mutation, plus cache ownership rows for display lists, render tiles,
image/mask caches, font/glyph caches, decoded streams, and transaction
provenance. It also records cancellation and stale-publication guard surfaces.
This does not claim external runtime thread/cache matrix validation; that
remains a platform/runtime verification item.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_concurrency_matrix_reports_thread_and_cache_boundaries --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo test -p wellfriendpdf-server capabilities_endpoint_exposes_renderer_cache_pressure_and_concurrency_policy --test server_integration --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo clippy -p wellfriendpdf-server --test server_integration --jobs 1 -- -D warnings` | 0 |

## Latest packed viewport/accounting hot-plan gates

This source slice moves page/tile viewport data needed by packed vector replay
and source-size accounting into `PackedDisplayList` itself. Spatial-index grid
construction and `RenderPlan::execute_vector_tile_with_scratch` now derive tile
windows from the packed viewport, and `estimate_render_plan_bytes` uses the
cached compile-time source byte count, rather than dereferencing the retained
source display list for those fields. The remaining packed transparent-page-
group source-list dependency is closed by the metadata gate below.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_uses_packed_viewport_for_vector_tile_execution --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_vector_plan_replays_without_raw_content_cold_table --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest retained transparent-page-group stat gates

This source slice moves top-level transparent-page-group planning for retained
display lists into `DisplayListStats`. The retained classifier now derives
`requires_transparent_page_group` from ExtGState alpha/blend/SMask resource
dictionaries and Form XObject transparency-group metadata in `PageResources`,
while preserving the complete no-paint alpha exception. Packed/contract page
rendering consumes that retained stat instead of re-fetching raw page content
and rescanning `ContentOperation` values after display-list compilation.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine transparent_form_xobject_sets_page_group_stat_without_raw_rescan --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine alpha_ext_gstate_does_not_force_page_compatibility_run --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine complete_no_paint_alpha_extgstate_does_not_require_page_group --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest packed transparent-page-group metadata gates

This source slice removes the last retained source-list payload from packed
plans. `PackedDisplayList::compile_with_resources` copies
`DisplayListStats::requires_transparent_page_group` into packed metadata,
`PackedDisplayList` no longer stores an `Arc<DisplayList>` or exposes
`source()`, and packed replay uses
`plan.packed.requires_transparent_page_group()` for both uncached and cached
transparent-page-group decisions.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_carries_transparent_page_group_stat_without_source_display_list --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest progressive image bounded-plan lifecycle gates

This source slice closes a progressive image-decode lifecycle gap for image
plans that already have native bounded decode work. Native sub-rectangle and
native reduced-resolution plans plus plan-derived raw component-subset requests
now complete with
`planned_partial_decode_complete` instead of remaining in a nonterminal
`full_decode_required` state. Full-output JPEG/JPX/JBIG2/lossless paths whose
selected decoder APIs still lack native progressive or partial pixel
continuation continue to report typed `full_decode_required` outcomes.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_native_window_plan_completes_without_full_decode_requirement --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_native_reduction_plan_completes_without_full_decode_requirement --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_native_component_subset_plan_completes_without_full_decode_requirement --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_image_decode_session_reports_full_decode_required --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_planning --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 58 tests |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest packed color-space optimizer gates

This source slice narrows RV-10's remaining dependent color-space/color setter
optimizer gap. Packed state-run optimization now overwrites non-adjacent
`CS`/`cs` setters across independent no-paint state such as line width, while
preserving both color-space descriptors when an intervening generic
`SC/SCN`/`sc/scn` or device color setter would make in-place rewrite
semantically unsafe.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_overwrites_non_adjacent_color_space_across_independent_state --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_does_not_overwrite_color_space_across --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 2 tests |
| `cargo test -p wellfriendpdf-engine packed_plan_ --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 19 tests |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest full workspace gates

These gates were run after the PixelBuffer active-clip borrowing slice and the
dirty-region cache-plan tile merge slice. They are no longer final-current
after the later renderer graph alias, packed matrix, SDK wording, managed
cache-handle, packed image XObject color-space payload, image color-space
resource-arena accounting, and Image XObject color-space exact-tile dependency
slices; final workspace gates still need to be rerun before completion.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |

## Latest packed Image XObject color-space payload gates

This source slice narrows the remaining packed high-level payload coverage gap
for Image XObjects. Resource-aware packed compilation now captures a
pre-resolved first-level named image color-space resource from Image XObject
stream dictionaries in `ResolvedXObjectHandle.image_color_space`. Packed replay
threads that payload through `ResolvedXObjectReplayMetadata`; `handle_do_image`
validates that the payload name matches the image dictionary before decoding,
then uses the resolved object instead of consulting the page resource map by
name for the first-level image color-space lookup.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine resource_aware_plan_pre_resolves_high_level_handles --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine packed_plan_replay_refuses_unresolved_resource_descriptors --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest Image XObject color-space dependency/accounting gates

This source slice extends the packed image color-space work into the metadata
surfaces that drive reporting and exact-tile invalidation. Backend-plan
resource arena counts now include pre-resolved Image XObject color-space
payloads, and the retained tile-resource dependency walker records named Image
XObject color-space resources plus transitive references behind those resources.
When exact tile coverage exists, changing a tint-transform/function used by an
image stream's `/ColorSpace /Name` can invalidate the recorded image tile
through source-to-source expansion instead of widening directly to the whole
page.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine backend_plan_arena_counts_image_xobject_color_space_payloads --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine image_xobject_color_space_dependency_records_transitive_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine backend_plan_arena --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 6 tests |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest image color-space dependency gates

This source slice extends exact-tile color-space dependency extraction to
referenced image `/Mask` and `/SMask` streams, retained inline images, native
shading dictionaries, pattern resources, ExtGState soft-mask group color spaces,
and direct color-space arrays that contain nested named color-space resources.
When a mask stream, inline image, active paint state, shading dictionary,
pattern resource, or SMask group uses a named `/ColorSpace` resource inside a
direct `/Indexed`, `/Separation`, or `/DeviceN`-style color-space object, the
retained tile-resource dependency walker now maps that nested resource and its
transitive references into the same source-to-tile/source-to-source graph as the
Image XObject, paint operation, shading operation, pattern paint, or ExtGState
paint. A changed tint-transform/function behind the nested resource therefore
invalidates the recorded tile when exact tile coverage exists, instead of
depending on broader page/resource invalidation.

| Command | Exit |
|---|---:|
| `cargo test -p wellfriendpdf-engine image_xobject_smask_color_space_dependency_records_transitive_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine color_space_dependency_records_transitive_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 4 tests |
| `cargo test -p wellfriendpdf-engine pattern_color_space_dependency_records_nested_resource_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine pattern_source_dependency --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine pattern_path_source_dependency_uses_typed_state_save_restore --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine extgstate_smask_group_color_space_dependency_records_transitive_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0 |
| `cargo test -p wellfriendpdf-engine ext_gstate_source_dependency_records --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 2 tests |
| `cargo test -p wellfriendpdf-engine nested_named_resource_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 3 tests |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest marked-content Properties dependency gates

This source slice preserves indirect `/Properties` resource refs through
`PageResources` and Form resource overlays, then records active `/OC`
marked-content property dependencies on each tile-affecting retained paint.
The marked-content stack is separate from graphics-state save/restore state, so
`EMC` restores the previous optional-content dependency scope before later paint
operations. A changed OCG/OCMD property object can now invalidate only the
recorded tile when exact tile coverage exists, instead of relying on broader
page/resource invalidation or missing the tile edge.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine properties --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 4 tests |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest SMask transfer-function cache-identity gates

This source slice closes a soft-mask cache identity hole around ExtGState SMask
`/TR` transfer functions. `smask_transfer_function_cache_label` now derives a
deterministic identity for absent/`/Identity`, direct function objects, and
indirect transfer-function refs before any SMask group cache lookup can hit.
Indirect refs use a `smask-transfer:ref:{object}:{generation}:` marker that is
also emitted by `source_cache_markers_for_object`, so cache owners can prune
transfer-function-dependent SMask entries when a referenced `/TR` object changes.
`smask_group_cache_key` includes that label, so a cached mask whose alpha was
post-processed by one `/TR` curve cannot be reused for a different transfer
function, and malformed transfer functions cannot bypass typed validation by
hitting an older cache entry. No PDF corpus, benchmark, competitor comparison,
VPS, deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine smask_group_cache_key --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 2 tests |
| `cargo test -p wellfriendpdf-engine smask_transfer --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 3 tests |
| `cargo test -p wellfriendpdf-engine transaction_invalidation_registers_source_cache_markers --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0, LF-to-CRLF warnings only |

## Latest ICC transform render-contract cache-scope/profile-digest/proof-cache/byte-accounting gates

This source slice separates bounded ICC transform cache entries by active
render-contract/output-profile identity. `ColorTransformOptions.cache_scope`
feeds `IccTransformKey`, and the key also records transform kind, full SHA-256
profile digest, and profile length so ordinary ICCBased-to-sRGB, built-in sRGB
proof probes, native output-intent proof transforms, and same-length profile
bytes cannot alias. Page rendering derives scope from the schema-v1
render-contract fingerprint, and output-intent proofing uses the same scope
helper. Native LittleCMS output-intent proof transforms now use the same bounded
ICC transform cache admission, eviction, and metrics path instead of building an
uncached proof transform for each call. ICC transform cache entries now carry
byte costs, maximum byte caps, byte-pressure eviction, over-budget rejection,
hit-order LRU refresh, and public `ColorReport` admission/rejection/byte
telemetry. Standalone CMM helper calls keep deterministic scope `0`. No PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine transform_cache --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 12 tests |
| `cargo test -p wellfriendpdf-engine output_intent_proof_transform_uses_bounded_cache --lib --features native-cmm-lcms2 --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --features native-cmm-lcms2 --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0, LF-to-CRLF warnings only |

## Latest font source-marker cache pruning gates

This source slice salts document font byte/resolver cache keys with
`font:ref:{object}:{generation}:` markers from preserved page/Form font
resource refs and nested indirect refs inside font dictionaries. Source-scoped
cache pruning now removes matching font byte and font resolver entries when a
mapped font, FontDescriptor, FontFile, CMap, or descendant source object changes,
while unrelated font cache entries remain admitted and exact raster-tile
invalidation stays narrow. No PDF corpus, benchmark, competitor comparison, VPS,
deployment, release, tag, or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine font_cache --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 4 tests |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest image decode tile/component write-set invalidation gates

This source slice extends transaction/cache-plan nested write-set normalization
for render-relevant image decode objects. Source-region, image-region,
region-decode, reduction, image-component, component-selection, codec-tile,
image-tile, JPX-tile, JPX-component, progressive-image, and image-progression
refs now accept snake_case and binding-style camelCase structured
object/generation aliases. Known refs map through cache-owned source identities,
register `image-decode:ref:{object}:{generation}:` source-cache markers, and
invalidate exact dependent tiles; unknown refs keep the conservative reset path.
This is source invalidation/ref-normalization work only and does not claim
codec-native ROI/progressive JPX pixel output. No PDF corpus, benchmark,
competitor comparison, VPS, deployment, release, tag, or package publication was
used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine image_decode_parameter_aliases --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo test -p wellfriendpdf-engine image_decode_tile_component_refs --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest optional-content Properties write-set invalidation gates

This follow-up wires the newly recorded active `/OC` marked-content property
tile edges into transaction cache-plan application. Nested write-set collection
now accepts `changed_oc_properties_refs` / `changedOCPropertiesRefs` and
`changed_properties_refs` / `changedPropertiesRefs` structured
object/generation aliases; known refs map through `RenderDocumentCache` source
identities, register `properties:ref:{object}:{generation}:` source-cache
markers, and invalidate only exact dependent tiles. Unknown refs keep the
existing conservative reset behavior. This stayed local and synthetic; no PDF
corpus, benchmark, competitor comparison, VPS, deployment, release, tag, or
package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo test -p wellfriendpdf-engine optional_content_properties --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 2 tests |
| `cargo fmt --all --check` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |

## Latest progressive render-contract parity gates

This source slice routes progressive rendering through the canonical schema-v1
render contract instead of the old page/DPI/render-mode-only contract proxy.
`RenderContract::with_render_tile` derives tile-local contracts while preserving
exactness, compositing, color, print/proof, optional-content, budget, backend,
and determinism identity. `ContentEngine` exposes contract-backed progressive
job constructors, `ProgressiveRenderJob` stores a normalized base contract,
derives per-tile cache identity from it, tiles over the caller's device-space
clip when the contract is clipped, assembles the final progressive surface at
clip-relative offsets, and records retained-replay refusal policy from
`ExactnessPolicy` instead of inferring exactness from `RenderMode`. The page
renderer now has a contract-aware progressive tile path that validates
canonical RGBA progressive output, applies contract color, proof, halftone,
resource-budget, optional-content, and cache identity policy, and keeps
unsupported retained replay as typed refusal instead of immediate fallback. The
server accepts `render_contract_json` / `contract_json` at
progressive start, and Rust, C ABI/header, Python, WASM/TypeScript, .NET, and
Java source surfaces expose matching contract-backed progressive constructors.
The source surface is complete for full-page progressive sessions; arbitrary
foreign caller-owned progressive surfaces and external runtime package matrices
remain deferred or covered by non-progressive caller-owned rendering surfaces.
No PDF corpus, benchmark, competitor comparison, VPS, deployment, release, tag,
or package publication was used.

| Command | Exit |
|---|---:|
| `cargo fmt --all` | 0 |
| `cargo check -p wellfriendpdf-engine --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-engine progressive_contract --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 3 tests |
| `cargo test -p wellfriendpdf-engine fallback_report_keeps_codes_and_structured_policy_details --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo test -p wellfriendpdf-engine progressive_refuses_unsupported_display_list_tile --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 2 tests |
| `cargo test -p wellfriendpdf-server --test progressive_integration progressive_start_accepts_render_contract_json --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo check -p wellfriendpdf-server --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-capi --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-py --lib --jobs 1` | 0 |
| `cargo check -p wellfriendpdf-wasm --lib --jobs 1` | 0 |
| `cargo test -p wellfriendpdf-capi capi_progressive_render_new_with_contract_json_preserves_contract_identity --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `dotnet build bindings\dotnet\WellfriendPdf\WellfriendPdf.csproj -maxcpucount:1 -p:UseSharedCompilation=false` | 0 |
| `javac --enable-preview --release 25 -d .work\final-universal-renderer-implementation\java-classes bindings\java\src\main\java\io\wellfriendpdf\WellfriendPdf.java` | 0 |
| `cargo fmt --all --check` | 0 |
| `cargo clippy -p wellfriendpdf-engine --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-server --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-capi --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-py --lib --jobs 1 -- -D warnings` | 0 |
| `cargo clippy -p wellfriendpdf-wasm --lib --jobs 1 -- -D warnings` | 0 |
| `git diff --check` | 0, LF-to-CRLF warnings only |

## Latest full workspace check refresh

This broad source/build gate was rerun after the packed Image XObject
color-space payload, resource-arena accounting, and exact-tile dependency
slices.

| Command | Exit |
|---|---:|
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |

## Final progressive contract-revision and README gates

The final source slice makes full live schema-v1 contract revision explicit in
the progressive capability report and every binding guide. The Rust job already
reconstructed its normalized output region, tile grid, publication identity,
and tile-local contract identity; this pass adds a direct C ABI behavior test,
keeps compatibility refusal wording aligned with native retained-plan support,
and guards the SDK/runtime feature reports against regressing to the older
fingerprint-only description. The root README now leads with a reserved,
clearly pending benchmark-results section and documents actual build and usage
flows for the Rust engine, CLI, server, C ABI, Python, WASM, .NET, Java, OCR,
SIMD, color management, visual-normalization harness, and PDFium harness.

The final marker scan found no actionable `TODO`, `FIXME`, `HACK`, `todo!`,
`unimplemented!`, stub, immediate-renderer delegation, or legacy-renderer
delegation in the renderer, image, font, SIMD, binding, and server source
scope. Remaining compatibility/fallback references are explicit policy,
capability, test, or vector-export boundaries already catalogued below.

| Command | Exit |
|---|---:|
| `cargo fmt --all --check` | 0 |
| `cargo test -p wellfriendpdf-engine revise_render_contract --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo test -p wellfriendpdf-capi capi_progressive_revise_render_contract_updates_live_job --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo test -p wellfriendpdf-server --test progressive_integration progressive_revise_render_context_accepts_render_contract_json --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo test -p wellfriendpdf-engine renderer_capabilities_disclose_fallback_and_progressive_limits --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo test -p wellfriendpdf-engine font_signature_decode_dedup_feature_envelopes --lib --jobs 1 -- --test-threads=1 --nocapture` | 0, 1 test |
| `cargo check --workspace --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check --workspace --all-features --all-targets --jobs 1` | 0 |
| `cargo clippy --workspace --all-features --all-targets --jobs 1 -- -D warnings` | 0 |
| `cargo check -p wellfriendpdf-wasm --target wasm32-unknown-unknown --features panic-hook --jobs 1` | 0 |
| `dotnet build bindings\dotnet\WellfriendPdf\WellfriendPdf.csproj --nologo -v:minimal -maxcpucount:1 -p:UseSharedCompilation=false` | 0, 0 warnings |
| `.NET RenderContractBuilderRoundTripsSchemaJson focused test` | 0, 1 test |
| Java 25 preview main/test compile and `WellfriendPdfSmokeTest --contract-builder-only` | 0 |
| `python -m pytest tools\renderer-visual-diff\test_visual_normalization.py -q` | 0, 46 tests |
| `python -m py_compile tools\renderer-visual-diff\visual_diff.py` | 0 |
| `python -m json.tool tools\renderer-visual-diff\normalization-manifest.json` | 0 |
| README relative-link check | 0, 13 links, 0 missing |
| scoped renderer source-marker audit | 1 from `rg`, meaning no matches |
| `git diff --check` | 0, line-ending normalization warnings only |

The first final Clippy invocation outlived the command-output window. A second
invocation briefly waited on Cargo's artifact lock; the duplicate waiting
process was stopped, the original completed, and Clippy was rerun from the
settled cache with exit 0. Only one compiler was active, and the final gates
continued serially.

## Final reconciled 61-item report

The large tables above are a chronological implementation journal. Their older
`PARTIAL` and `PARTIAL_ADVANCED` labels describe the state at the time of each
slice and are not the final verdict. The reconciled source state is below.

| # | Required result | Final source state |
|---:|---|---|
| 1 | Starting commit | `2c893fbfe5ca3799f7ba9e437fe080f63735e0ca` |
| 2 | Final commit | The commit containing this report; the post-commit hash is recorded in the final task response. |
| 3 | Branch | `main` |
| 4 | Local worktree state | Required clean after the final commit; verified in the post-commit gate. |
| 5 | GitHub push status | Required `HEAD == origin/main`; verified in the post-push gate and recorded in the final task response. |
| 6 | Packed backend plans | Complete for supported high-level visual operations through dense hot ops and typed pre-resolved descriptors/subplans; unsupported semantics fail typed. |
| 7 | Retained immediate delegation | Removed from supported retained replay; unsupported retained replay produces typed refusal and no fallback pixels. |
| 8 | Hot/cold display lists | Packed hot arenas are separated from diagnostics/provenance cold tables; no raw `ContentOperation` payload is stored in hot replay. |
| 9 | Transaction invalidation | Render write sets, page/source/resource aliases, dirty regions/tiles, and revisions drive narrow invalidation; uncertainty broadens safely. |
| 10 | Cache dependency graph | Bounded source-to-source, source-to-page, source-to-resource, source-to-tile, and retained-artifact edges are active. |
| 11 | Persistent clip DAG | Full/empty/rectangle/sparse-span/RLE/dense/composite nodes, stable scoped identity, interning, pruning, and lazy window realization are active. |
| 12 | Transparency | Bounded isolated/non-isolated/knockout/nested group architecture, blend execution, group identity, clip/mask fusion, and typed unsupported boundaries are active. |
| 13 | Soft masks | Alpha/luminosity, backdrop, transfer, color-policy, revision, contract, tile, clip, backend, quality, and profile identity are represented and guarded. |
| 14 | Print profile | Display and print/proof contracts are separate and key DPI, CMYK/named color, overprint, intent, halftone, proofing, annotations/forms, CMM, and budgets. |
| 15 | Adaptive scheduler | Deterministic 128/192/256/384/512/adaptive selection, viewer priorities, dirty rescheduling, obsolete-work cancellation, and publication guards are active. |
| 16 | Region image decode | Metadata-first source-window planning and exact region paths are active where codec adapters support them; unavailable codec-native ROI is reported, not simulated. |
| 17 | Scaled image decode | JPEG reduced-IDCT and guarded JPX target-resolution decode are active with scale/reduction-aware cache identity. |
| 18 | Progressive image decode | Bounded start/continue/pause/resume/cancel/fail/close/document-close state and capability reporting are implemented; adapters without incremental pixels report unavailable. |
| 19 | WASM SIMD | Actual `wasm32` `simd128` kernels and guarded scalar fallback are present; the wasm32 target check passes. |
| 20 | Rust progressive API | Complete lifecycle, contract construction/revision, queues, prefetch, callbacks, cancellation, and checked finish. |
| 21 | C progressive API | Opaque job/cancellation ownership, full contract revision, lifecycle, queue/prefetch/callback, publication, and finish APIs are present and behavior-tested. |
| 22 | Python progressive API | Source-complete lifecycle, full contract revision, cancellation, queue/prefetch/callback, and checked finish surface. |
| 23 | WASM progressive API | Source-complete lifecycle, full contract revision, cancellation/AbortSignal checks, queue/prefetch/callback, and checked finish surface. |
| 24 | .NET progressive API | Managed owned session, typed/full-JSON contract revision, cancellation, queue/prefetch/callback, and checked finish source compiles. |
| 25 | Java progressive API | FFM-owned session, typed/full-JSON contract revision, cancellation, queue/prefetch/callback, and checked finish source compiles. |
| 26 | Server progressive API | Owner-scoped bounded sessions expose start/step/pause/resume/revise/cancel/finish/close plus publication and queue operations. |
| 27 | Caller-owned surfaces | Rust, C, Python, WASM, .NET, and Java applicable APIs validate dimensions, stride, format, alpha, byte order, length, and cancellation. |
| 28 | Cancellation parity | A shared renderer cancellation state reaches preparation, planning, decoding, transparency, tile execution, progressive work, and all binding layers. |
| 29 | Contract-builder parity | Canonical schema-v1 construction/round-trip and validation exist in Rust, CLI/server transport, C, Python, WASM, .NET, and Java. |
| 30 | Font substitution | Deterministic ordered resolution and bounded reports expose request, embedding, encoding, coverage, replacement, reason, metrics/risk, impacts, and policy identity. |
| 31 | Type 3 | Supported CharProcs use native retained subplans with resources, paths, images, forms, patterns, shadings, transparency, matrices, and bounded caches; unsupported recursion/content refuses typed. |
| 32 | JPX | Metadata and dimension checks plus guarded target-resolution reduction are active; unavailable ROI/tile/component/progressive adapter capabilities are explicit and fully keyed. |
| 33 | SIMD compositor | Scalar-oracle-checked native, portable-wide, and wasm kernels cover fills, source-over, masks, glyphs, soft masks, conversions, premultiplication, grayscale, and common blends. |
| 34 | Scan converter | Adaptive curves, monotonic decomposition, tile-local buckets, reusable active edges/spans, winding rules, AA, stroke geometry, hairlines, degenerates, and fast paths are active. |
| 35 | Image cache | Bounded revision/contract/codec/decode/region/reduction/scale/color/mask/interpolation/format/backend keys with admission, eviction, and invalidation. |
| 36 | Glyph cache | Bounded font/policy/contract/outline/mask/atlas identity, byte accounting, eviction, source pruning, and document isolation. |
| 37 | Color cache | Bounded ICC/named-color/proof/display/print/intent/CMM/contract identities with transform and proof-cache accounting. |
| 38 | Form XObjects | Reusable retained Form sublists, pre-resolved resources, transparency metadata, recursion guards, cache identity, and shared-resource invalidation are active. |
| 39 | Annotation/widgets | Appearance selection/synthesis is fail-closed; appearance caches, form values, object/source dependencies, dirty regions, and invalidation are active. |
| 40 | SVG regional fallback | Native vector output is preserved for supported operations; unsupported bounded regions use reported regional rasterization, while strict mode refuses whole-page compatibility fallback. |
| 41 | PS regional fallback | Native LanguageLevel 3 vector/shading/image output and bounded regional fallback are active; unsupported transparency or global semantics are reported/refused by policy. |
| 42 | Visual normalization | Canonical box/matrix/dimension/channel/alpha/background/profile/policy normalization and future metric/classification tooling are implemented without corpus execution. |
| 43 | Fallback categories found at start | 12: three retained/tile/progressive delegation paths, unresolved Type 3, bundled font substitution, JPX compatibility, qcms selection, SVG and PS/EPS whole-page compatibility, inactive halftone, byte-order refusal, and non-zero tile-origin decode fail-open. |
| 44 | Fallback categories remaining | 6 explicit compatibility/capability categories: bundled-font Compat policy, codec-native partial-decode unavailability, JPEG region/progressive unavailability, JPX ROI/tile/component/progressive unavailability, portable qcms selection, and opt-in SVG/PS/EPS whole-page compatibility export. |
| 45 | Material-degrading high-quality fallback | Zero; high-quality/exact paths fail typed rather than silently substituting, skipping, delegating, or broad-rasterizing. |
| 46 | Newly discovered gaps | Full progressive contract revision was missing from public capability/usage reporting and lacked a direct C behavior test; root/component documentation was stale. |
| 47 | Newly discovered gaps completed | Runtime/SDK reporting, C behavior test, retained-refusal wording, binding guides, and the repository README were completed. |
| 48 | Lightweight checks executed | Formatting, focused state/API tests, default/all-feature workspace checks and Clippy, wasm32 check, .NET build/test, Java compile/smoke, marker/link/diff audits. |
| 49 | Commands and exits | Every final command listed in the preceding gate table exited 0, except the no-match `rg` audit whose expected exit was 1. |
| 50 | CPU ceiling | Final gates used one Cargo job/test thread and one .NET/JVM worker, below the 3-logical-CPU ceiling. |
| 51 | RAM ceiling | Disk-backed target/temp paths, serial toolchains, JVM `-Xmx1024m`, and no concurrent heavy toolchains were used in the final gate. |
| 52 | Local storage path | `E:\wellpdfsdk\.work\final-universal-renderer-implementation` |
| 53 | Resource ceiling exceeded | An earlier interrupted continuation briefly exceeded the intended task RAM ceiling; the final continuation did not. A duplicate final Clippy waiter did not run a second compiler. |
| 54 | Corrective action | Stray/duplicate compiler work was allowed to exit or stopped, process state was rechecked, jobs were reduced to one, and every affected gate was rerun serially. |
| 55 | Real PDF corpus | Not used. |
| 56 | Performance benchmark | Not run. |
| 57 | Competitor benchmark | Not run. |
| 58 | VPS | Not used. |
| 59 | Deployment | None. |
| 60 | Release/tag/publication | None. |
| 61 | Final verdict | `IMPLEMENTATION_COMPLETE_AWAITING_FINAL_VERIFICATION` |

## Code-size accounting

The final staged change contains 156,613 insertions and 25,849 deletions across
120 files, including eight new source files. A
path/extension classification counts 134,638 added production-source physical
lines and 10,541 added test-source physical lines. The resulting checkout has
458,009 production-source physical lines across 392 files and 38,523 test-source
physical lines across 119 files. These are physical-line counts, not a claim
that comments or generated-looking declarations are executable statements;
functionality is supported by the compile, lint, and focused behavior gates
recorded above.

## Boundary confirmations

No real PDF corpus was used. No performance benchmark was run. No competitor benchmark was run. No VPS was used. No deployment occurred. No release, tag, or package publication occurred.

## Final implementation verdict

`IMPLEMENTATION_COMPLETE_AWAITING_FINAL_VERIFICATION`
