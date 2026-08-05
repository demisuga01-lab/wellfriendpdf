# Final renderer fallback inventory

**Snapshot:** `E:\wellpdfsdk` after the interrupted renderer continuation was repaired for parsing, unsafe-SIMD boundary compilation, scalar-equivalence fallback behavior, scanline coverage, and warnings-as-errors cleanliness.
**Method:** source inspection only. This is not corpus or performance evidence. A “fallback” here means an active branch that changes execution strategy, declines native retained replay, substitutes resources, or loses vector/exact semantic information. It is not a raw search count for the word `fallback`.

## Count by type

| Type | Active fallback decisions | Approximate/degraded decisions | Source basis |
|---|---:|---:|---|
| Immediate retained-list fallback | 3 | 0 when canonical immediate rendering succeeds | `page_renderer.rs:558+`, `1002+`, `progressive.rs:164+` |
| Patterns | 2 | 2 | `page_renderer.rs:5465-5526` |
| Type 3 | 1 | 1 in compatibility mode | `page_renderer.rs:5951+` |
| Fonts | 1 | 1 when an embedded face cannot be used | `font_rasterizer.rs:get_fallback_font` |
| Shadings | 1 | 1 (missing named resource canonical no-op) | `display_list.rs` native shading handling/tests |
| Images | 1 | 1 compatibility-specific JPX path | `image_painter.rs` / `page_renderer.rs:4658` |
| Clipping | 0 | 0 | General scalar/scanline paths are exact strategy changes, not degraded fallbacks |
| Annotations/forms | 0 | 0 | Missing appearances use explicit synthesis or are skipped by visibility policy |
| Colour | 1 | 0 claimed; different CMM implementation | `cmm.rs` qcms fallback |
| SVG/PS vector output | 2 | 2 for vector-semantic preservation, raster pixels remain engine-rendered | `svg.rs`, `postscript.rs` |
| **Total active decisions** | **12** | **8** | Source inspection of active renderer/output call paths |

| ID | Source location | Trigger | Exact behavior | Approximate? | Calls immediate renderer? | Mode | Supported replacement | Current status | Tests / source evidence | Final disposition |
|---|---|---|---|---|---|---|---|---|---|---|
| FB-01 | `render/page_renderer.rs:558-610` | `DisplayList::is_fully_supported()` is false | Cached page route executes canonical `RenderState::dispatch_all` immediate path | No; intended canonical path | Yes | Standard | Packed native retained plan | PARTIAL | Display-list tests cover supported native ops, not closure for every PDF operator | Must remain explicitly counted until unsupported retained operations are compiled natively |
| FB-02 | `render/page_renderer.rs:1002-1022` | Retained tile render returns `None` | Tile uses immediate tile renderer | No; canonical tile rendering | Yes | Standard | Native retained tile plan | PARTIAL | Tile/band tests; source call is explicit | Must remain counted and exposed by progressive/tile reporting |
| FB-03 | `render/progressive.rs:176-185` | Progressive tile list is unsupported | Progressive job calls immediate tile renderer | No; canonical tile rendering | Yes | Standard | Native retained progressive plan | PARTIAL | `progressive_render_resume_matches_full_page` | Must remain counted; no claim that progressive is retained-only |
| FB-04 | `render/page_renderer.rs:5465-5466` | Direct/indirect tiling-pattern recursion detected | Logs warning then `paint_tiling_pattern_solid_fallback` | Yes | No direct page call identified | Standard | Exact typed recursion refusal or bounded native cycle semantics | PARTIAL | `recursive_tiling_pattern_uses_bounded_fallback` | High-quality exact behavior must not silently use this paint fallback |
| FB-05 | `render/page_renderer.rs:5514-5526` | Pattern cell count exceeds `COMPAT_TILE_CAP` | Logs warning then uses solid paint fallback | Yes | No direct page call identified | Standard | Visible-cell native range with typed resource limit | PARTIAL | Pattern tests; cap source is explicit | Must become typed/counted resource-limit result in exact mode |
| FB-06 | `render/page_renderer.rs:5951-6000` | Type 3 rendering cannot use the normal resolved font path in compatibility mode | Enables compatibility font bytes/fallback; high-quality path rejects it | Yes in compatibility mode | No direct page call identified | Standard | Immutable compiled Type 3 glyph sublist/cache state machine | PARTIAL | Type 3 unit tests and `type3_charproc_renders_resource_xobject_image` | Keep explicit and counted until exact native Type 3 closure |
| FB-07 | `render/font_rasterizer.rs:75+` | Embedded/mapped font unavailable or unsupported | Uses bundled Liberation/DejaVu font bytes by family/name heuristic | Yes | No | Standard | Embedded/valid resolved font program | PARTIAL | font fallback tests | Keep explicit substitution diagnostic; do not present as original font equivalence |
| FB-08 | `render/display_list.rs` native shading handling | Named shading resource is absent/unresolvable | Native op reaches canonical no-op path rather than page-wide fallback | Yes—content cannot paint | No | Standard | Valid resolved resource / typed malformed-resource result | PARTIAL | `missing_named_shading_stays_native_and_replays_canonical_noop` | Must be reported as missing-resource fallback, never hidden in exact mode |
| FB-09 | `render/image_painter.rs:231`, `page_renderer.rs:4658` | JPX compatibility path is selected | Uses older compatibility image behavior | Potentially | No | Standard | Fully unified JPX sampling path | PARTIAL | JPX fixture tests | Keep visibility in capability/fallback report until output contract selects one exact route |
| FB-10 | `render/cmm.rs:3,117,151+` | Optional LittleCMS backend is not compiled/available | Uses qcms transform backend | Not asserted pixel-identical across CMMs | No | Standard; native CMM optional | `native-cmm-lcms2` when configured | COMPLETE_NOT_DEFAULT only after feature validation; currently UNVERIFIED | CMM tests cover portable path | Report backend identity in render contract/capabilities |
| FB-11 | `render/svg.rs:57-103` | SVG vector writer sees image/shading/unsupported vector feature | Emits embedded raster image for page | Vector semantics degraded; raster pixels come from engine | Yes | Standard output conversion | Native SVG implementation for feature | PARTIAL | SVG raster-fallback test | Retain explicit `rasterized` reporting |
| FB-12 | `render/postscript.rs:76-123` | PS/EPS vector writer sees image/shading/unsupported vector feature | Emits rasterized page embed | Vector semantics degraded; raster pixels come from engine | Yes | Standard output conversion | Native PostScript implementation for feature | PARTIAL | PS raster-fallback test | Retain explicit `rasterized` reporting |

## Closure policy

- No fallback is treated as proof that a feature is universally implemented.
- Exact/canonical immediate rendering is not labelled approximate merely because it is a different execution strategy; it remains a blocker for the native-retained requirement.
- Pattern solid paints, Type 3 compatibility fonts, missing-resource no-ops, and vector-output rasterization are materially visible/degraded and must remain exposed.
- The current renderer has no source-proven zero-fallback exact mode across all requested PDF operations.
- No corpus, latency, throughput, pixel-difference, or competitor benchmark was run to produce this inventory.
