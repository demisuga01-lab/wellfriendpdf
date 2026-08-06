# Final renderer fallback closure report

**Repository:** `E:\wellpdfsdk`
**Closure branch:** `main`
**Scope:** source-level fallback disposition and compact functional verification before the separate benchmark task. This report contains no performance, corpus, latency, throughput, or competitor result.

## Summary

The previous inventory contained 12 fallback decisions, eight of which could materially change visual output. This pass removed the two tiling-pattern solid-colour degradations, changed missing named shading from an implicit no-op to an explicit unsupported diagnostic, made supported Type 3 charprocs native-first, and made PostScript ExtGState output safe by raster fallback rather than silently omitting transparency.

**Current active fallback-policy categories:** 9.
**Current materially degraded output categories:** 5.
**Resolved into exact typed refusal/diagnostic rather than degraded pixels:** 3.

`HighQualityExact` policy does not silently take the removed pattern solid fallback. Invalid recursive or over-limit pattern resources return a typed `UnsupportedFeature` through root page, tile, retained, and progressive paths.

| ID | Previous decision | Current source disposition | Exact source | Classification | Verification |
|---|---|---|---|---|---|
| FB-01 | Unsupported retained list delegates to immediate renderer | Remains explicit only for `DisplayList::is_fully_supported() == false`; retained diagnostics identify the reason | `render/page_renderer.rs`, `render_page_cancellable_with_mode_and_cache` | Explicit canonical fallback; not an approximation claim | retained/fallback capability tests |
| FB-02 | Unsupported retained tile delegates to immediate tile renderer | Remains explicit and is reported by progressive fallback events | `render/page_renderer.rs`, `render_page_display_list_tile_cancellable_with_mode_and_cache`; `progressive.rs` | Explicit canonical fallback | progressive lifecycle and tile tests |
| FB-03 | Unsupported progressive tile delegates to immediate tile renderer | Remains explicit `unsupported_display_list_immediate_tile` event | `render/progressive.rs` | Explicit canonical fallback | progressive unit and existing equivalence tests |
| FB-04 | Recursive pattern painted a solid colour | **Removed.** Root renderer records typed recursive-pattern failure; no solid paint is emitted | `render/page_renderer.rs`, `paint_tiling_pattern_with_device_clip` | Exact typed refusal | `recursive_tiling_pattern_returns_typed_refusal` |
| FB-05 | Excessive pattern cell count painted a solid colour | **Removed.** Root renderer records a typed exact render-limit failure | `render/page_renderer.rs`, pattern cell-limit branch | Typed resource-limit refusal | source guard; shared failure propagation |
| FB-06 | Type 3 compatibility font could precede native charproc rendering | **Mitigated.** Supported Type 3 glyphs call `render_type3_glyph` first; compatibility font remains only after native resolution fails in Compat mode | `render/page_renderer.rs`, `render_text_string` | Residual degraded compatibility path for unresolved Type 3 only | `type3_charproc_renders_resource_xobject_image` |
| FB-07 | Bundled font substitution | Remains deterministic fallback for missing/non-embedded fonts; core does not yet expose full per-glyph public substitution report | `render/font_rasterizer.rs`; `page_renderer.rs` | Materially degraded when metrics/glyph coverage differ | existing font resolver tests |
| FB-08 | Missing named shading silently painted nothing | **Removed from retained replay.** Display-list build records `UnsupportedRenderOp { operator: "sh", reason: ... }` | `render/display_list.rs`, `push_native_shading` | Explicit unsupported diagnostic | `missing_named_shading_is_explicitly_unsupported` |
| FB-09 | JPX compatibility image path | Retained; decoder support is active, but multi-resolution/region decode is not closed | `render/image_painter.rs`; `images/jpx.rs` | Potential quality/resource degradation | JPX decoder tests |
| FB-10 | Portable qcms colour path | Retained as an explicit supported portable backend; native LittleCMS is now provisioned and all-features compiled | `render/cmm.rs`; `native-cmm-lcms2` | Supported backend selection, not an unexplained fallback | VPS all-features check/test |
| FB-11 | SVG whole-page raster embed | Retained where the SVG sink cannot faithfully express the construct; ExtGState/dense-text handling is explicit | `render/svg.rs` | Materially degraded vector preservation | SVG output tests |
| FB-12 | PS/EPS raster embed / missing ExtGState semantics | Retained only as explicit raster embedding; `gs` now safely triggers it rather than silently losing transparency | `render/postscript.rs`, `needs_raster_fallback` | Materially degraded vector preservation but safe output | `raster_fallback_triggers_on_images_and_shadings` |

## Fallback reporting contract

- `DisplayList::unsupported` is the retained source-level classification boundary.
- `ProgressiveRenderStepReport::fallback_events` records progressive retained-to-immediate decisions.
- Runtime capabilities expose retained replay, fallback reporting, progressive lifecycle, SIMD fallback semantics, versioned contract, and packed-vector-plan status.
- No fallback removed by this pass is relabeled as native support when its semantics are unavailable. Pattern cycles and exactness limits fail closed instead.

## Remaining closure blockers

The residual policies above, incomplete native compiled payloads for high-level retained operations, incomplete public progressive sessions outside Rust, incomplete full binding contract builders, vector regionalization, font substitution reporting, and JPX region/progressive decode remain benchmark-readiness blockers.
