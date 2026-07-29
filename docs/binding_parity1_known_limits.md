# writer history Known Limits

Exact limits:

- Raster vectorization reconstructs bounded shape evidence, not original authoring paths.
- Photographs, textured art, and noisy continuous-tone scans are unsupported or low-confidence report-only cases.
- Text is not vectorized by default when semantic text evidence is present.
- Font repair does not claim original font identity or legal redistribution rights.
- External glyph generation is disabled by default and requires explicit backend provenance.
- Persistent history is editor state, not cryptographic PDF revision history.
- Object-stream packing is an opt-in full rewrite and invalidates prior cryptographic signatures.
- Linearization is not preserved unless a real linearizer is run after packing.

See `writer_history-limit-denial-results.json` for machine-readable policy rows.
