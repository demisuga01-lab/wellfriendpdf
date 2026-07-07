# Prompt 09 OCG Layer Validation

Prompt 09B validates the default-view optional-content implementation added in Prompt 09. The authoritative matrix is `target/prompt09-annotation-ocg-progressive-cache/ocg-layer-matrix-prompt09b.json`.

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

`RenderCacheKey` includes page number, DPI, render mode, tile rectangle, and the OCG visibility fingerprint. Prompt 09B adds `render_cache_key_includes_visibility_fingerprint`, which proves two otherwise identical tile keys do not alias when layer state differs.

Artifact:

- `target/prompt09-annotation-ocg-progressive-cache/ocg-cache-key-fingerprint-prompt09b.json`

## Reference Evidence

Prompt 09B reference artifacts:

- `ocg-reference-results-prompt09b.json`
- `multi-reference-render-results-prompt09b.json`
- `reference-disagreement-summary-prompt09b.json`

The OCMD `AllOn` fixture is a classified reference disagreement with Oxide inside the acceptable Prompt 09B policy cluster. There are 0 stale-cache visibility bugs, 0 OCG outliers, and 0 unclassified OCG failures.

## Remaining Limits

Alternate configuration selection and active Print/Export mode selection are parsed/reportable but are not public render options yet. Malformed or cyclic optional-content references fail open with diagnostics.
