# Rendering pipeline

The canonical rendering pipeline is:

1. open bytes and revision provenance;
2. resolve the COS graph and page tree;
3. decode page contents through bounded stream filters;
4. parse source content instructions;
5. compile or inspect the immutable display list;
6. choose a backend render plan from the page capability vector;
7. execute scalar/SIMD CPU rendering, or an explicitly configured Research
   backend when available;
8. render annotations, forms, optional-content state, OCR/searchable layers,
   and tagged/provenance side effects;
9. encode requested output or return raw pixel evidence;
10. record telemetry, failures, deterministic hashes, and resource use.

Standard mode is deterministic and CPU-first. Research mode may add optional
accelerators, but absence of those accelerators falls back to the Standard
pipeline with an inactive capability report.

The `render-corpus` CLI command added for the renderer-capability campaign runs
this pipeline in-process across a real corpus and writes JSONL plus aggregate
summary evidence without committing generated page images.
