# Low-resource deployment

The minimum Standard deployment target is 2 vCPU and 6 GB RAM.

At this size Wellfriend keeps the supported feature set active but reduces concurrency, speculative work, and cache budgets. Large pages use tile or band rendering. Eligible decoded streams, rasters, OCR intermediates, and writer staging can spill to bounded temporary storage. Optional oversized analysis is refused rather than silently weakening output correctness.

Recommended Standard deployment is 4 vCPU and 8 GB RAM. That profile enables higher CPU concurrency, larger bounded caches, more concurrent documents, and stronger server scheduling without exposing a separate public mode.
