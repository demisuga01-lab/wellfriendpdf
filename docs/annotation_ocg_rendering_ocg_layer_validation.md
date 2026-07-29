# Annotation Ocg Rendering OCG Layer Validation

Renderer Validation validates the default-view optional-content implementation added in Annotation Ocg Rendering. The authoritative matrix is `target/annotation_ocg_rendering-annotation-ocg-progressive-cache/ocg-layer-matrix-renderer_validation.json`.

## Proven Paths

- Document `/OCProperties` discovery
- OCG inventory and metadata
- OCMD inventory and `AnyOn`, `AllOn`, `AnyOff`, `AllOff` policy evaluation
- Default configuration, `/BaseState`, `/ON`, and `/OFF`
- `/Intent /View` matching and `/Usage /View` state posture
- Marked-content visibility stack with balanced `BDC`/`EMC`
- OCG-gated Form XObjects
- OCG-gated annotations
- Pattern and shading operations hidden by OCG marked content
- OCG visibility fingerprint in render tile cache keys

## Cache Fingerprint

`RenderCacheKey` includes page number, DPI, render mode, tile rectangle, and the OCG visibility fingerprint. Renderer Validation adds `render_cache_key_includes_visibility_fingerprint`, which proves two otherwise identical tile keys do not alias when layer state differs.

Artifact:

- `target/annotation_ocg_rendering-annotation-ocg-progressive-cache/ocg-cache-key-fingerprint-renderer_validation.json`

## Reference Evidence

Renderer Validation reference artifacts:

- `ocg-reference-results-renderer_validation.json`
- `multi-reference-render-results-renderer_validation.json`
- `reference-disagreement-summary-renderer_validation.json`

The OCMD `AllOn` fixture is a classified reference disagreement with Wellfriend inside the acceptable Renderer Validation policy cluster. There are 0 stale-cache visibility bugs, 0 OCG outliers, and 0 unclassified OCG failures.

## Remaining Limits

Alternate configuration selection and active Print/Export mode selection are parsed/reportable but are not public render options yet. Malformed or cyclic optional-content references fail open with diagnostics.
