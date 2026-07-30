# Resource management

The adaptive resource governor receives a `WorkEstimate` for each expensive task. Estimates include CPU units, memory bytes, temporary disk bytes, I/O weight, parallelism ceiling, deadline, interruptibility, and provider requirements.

Work classes are separated for metadata, parser/recovery, decoding, rendering, image codecs, shaping, reflow, OCR, writer/compression, standards/accessibility, redaction/sanitization, and external providers. This prevents a single heavy class from exhausting every permit.

The memory coordinator owns the shared process budget for parsed data, object streams, decoded streams, display lists, spatial indexes, render tiles, fonts/shaping/glyphs, images/masks, OCR sessions, transactions/provenance, writer staging, and provider buffers.

Under pressure the engine response order is:

1. reduce concurrency;
2. evict recomputable tiles;
3. evict decoded images;
4. spill eligible streams;
5. reduce preview DPI;
6. disable speculative prefetch;
7. reject oversized optional analysis;
8. preserve output correctness.
