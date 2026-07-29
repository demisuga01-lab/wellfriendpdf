# Raster Vectorization Security Limits

writer history raster vectorization is fail-closed and bounded before decode and during component extraction.

Limits are reported in `raster-vectorization-performance-memory-writer_history.json` and `writer_history-limit-denial-results.json`:

| Limit | Default |
| --- | --- |
| Pixel cap | 8,000,000 decoded pixels |
| Component cap | 20,000 connected components |
| Point cap | 400,000 component points |
| Curve segment cap | 16,384 |
| Color region cap | 256 |
| Time posture | 30,000 ms policy row |

Malformed images, oversized rasters, invalid buffers, and component explosions return structured diagnostics with operation, object, reason, and policy status. Text evidence is preserved separately unless the caller explicitly requests text outlines.
