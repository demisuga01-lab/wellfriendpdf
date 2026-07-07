# Prompt 09 Tile, Band, And Cache Performance

Prompt 09B proves deterministic tile/band/cache behavior for the Prompt 09 renderer posture.

## Tile/Full Equivalence

Tile rendering remains compatibility-safe: render full page deterministically, then crop/stitch tiles. Prompt 09B uses exact-pixel Rust tests to prove stitched tile output matches full-page output.

Artifact:

- `tile-full-equivalence-prompt09b.json`

## Band/Full Equivalence

Band rendering uses deterministic vertical band stitching and exact-pixel comparison against full-page output.

Artifact:

- `band-full-equivalence-prompt09b.json`

## Cache/No-Cache Equivalence

The render tile cache is byte-budgeted and deterministic. Prompt 09B guards:

- cold cache insert path
- warm cache hit path
- disabled/oversized skip path
- deterministic LRU eviction
- OCG-aware key separation

Artifact:

- `cache-equivalence-prompt09b.json`

## Metrics

Prompt 09B records fixture count, fixture categories, tile sizes, band heights, cache hit/miss/insert/eviction posture, peak retained bytes, elapsed render time per fixture, scheduler denial behavior, and cancellation posture.

Artifacts:

- `tile-band-cache-performance-prompt09b.json`
- `tile-band-cache-memory-prompt09b.json`

## Remaining Limits

Prompt 09 does not introduce global image, Form, pattern, shading, clip-mask, or transparency-group surface caches. Parallel tile rendering remains disabled by default to preserve deterministic output.
