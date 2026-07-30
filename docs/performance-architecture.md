# Performance architecture

Wellfriend optimizes the existing canonical engine rather than replacing it with a separate fast path. The runtime architecture coordinates:

- lazy structural loading and compact COS data;
- streaming filters with limits and cancellation;
- copy-through writing for unchanged objects and streams;
- retained display lists;
- tile/band/scissor rendering;
- dirty-region invalidation;
- shape-plan and glyph caches;
- incremental reflow and dependency invalidation;
- persistent transaction snapshots;
- bounded table/math/OCR/form/security workloads;
- shared memory and queue admission.

Research-only backends provide contracts for GPU/hybrid rendering, learned cost selection, autotuning, model fusion, distributed workers, and validation-backed display-list rewrites. They are not default production paths.
