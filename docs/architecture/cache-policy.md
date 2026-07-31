# Cache policy

Rendering caches are bounded, keyed by semantic inputs, and governed by the
shared runtime memory coordinator.

Cache classes include display lists, spatial indexes, render tiles, fonts,
glyphs, decoded images, ICC transforms, patterns, OCR intermediates, and writer
staging. Cache entries must record size and must be evictable unless pinned by
an active operation.

Under memory pressure Standard mode reduces concurrency, evicts recomputable
tiles, evicts decoded images, spills eligible streams, disables speculative
prefetch, and rejects optional oversized analysis. It must not weaken output
correctness to fit memory.
