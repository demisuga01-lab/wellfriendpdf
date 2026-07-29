# Annotation Ocg Rendering Tile, Band, And Cache Performance

Renderer Validation proves deterministic tile/band/cache behavior for the Annotation Ocg Rendering renderer posture.

## Tile/Full Equivalence

Tile rendering remains compatibility-safe: render full page deterministically, then crop/stitch tiles. Renderer Validation uses exact-pixel Rust tests to prove stitched tile output matches full-page output.

Artifact:

- `tile-full-equivalence-renderer_validation.json`

## Band/Full Equivalence

Band rendering uses deterministic vertical band stitching and exact-pixel comparison against full-page output.

Artifact:

- `band-full-equivalence-renderer_validation.json`

## Cache/No-Cache Equivalence

The render tile cache is byte-budgeted and deterministic. Renderer Validation guards:

- cold cache insert path
- warm cache hit path
- disabled/oversized skip path
- deterministic LRU eviction
- OCG-aware key separation

Artifact:

- `cache-equivalence-renderer_validation.json`

## Metrics

Renderer Validation records fixture count, fixture categories, tile sizes, band heights, cache hit/miss/insert/eviction posture, peak retained bytes, elapsed render time per fixture, scheduler denial behavior, and cancellation posture.

Artifacts:

- `tile-band-cache-performance-renderer_validation.json`
- `tile-band-cache-memory-renderer_validation.json`

## Remaining Limits

Annotation Ocg Rendering does not introduce global image, Form, pattern, shading, clip-mask, or transparency-group surface caches. Parallel tile rendering remains disabled by default to preserve deterministic output.
