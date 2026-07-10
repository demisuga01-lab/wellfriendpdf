# Prompt 15 Semantic Intelligence Close-out

Combined Prompt 15 closes the semantic framework phase. Oxide now has one
binding-neutral semantic export, a table-specialized proposal contract,
provenance-aware RAG chunks, and an availability-aware benchmark. Deterministic
PDF extraction remains authoritative throughout.

## Delivered Surfaces

- `ContentEngine::semantic_binding_report` returns the typed Rust report.
- `oxide semantic-export` exports the bundle or focused summary, semantic,
  table, token, chunk, search, and status views.
- Python exposes `semantic_bundle`, `advanced_chunks`, `semantic_search`, and
  `table_proposal_status` as dictionaries.
- C exposes owned versioned JSON through
  `oxide_document_semantic_bundle_json`,
  `oxide_document_advanced_chunks_json`, and
  `oxide_document_semantic_search_json`.
- WASM, .NET, and Java wrap those same canonical SDK envelopes.
- Java Maven and Gradle package smokes assert the Prompt 15 report section and
  invoke the new document methods.

The semantic bundle contains the canonical document, detailed text model,
structure tree and MCID model, ParentTree recovery report, deterministic
tables, CJK token pages and dictionary metadata, advanced chunks, search
results, layout/table backend status, privacy status, and optional table
proposal merge diagnostics.

## Authority Model

1. PDF text, geometry, tables, forms, tags, and security scans are extracted
   deterministically.
2. ParentTree repairs and other inferred structure remain marked as repaired or
   inferred rather than author-original.
3. Dictionary segmentation adds a token layer and never rewrites raw text.
4. Layout and table model output is a proposal layer. It may add candidates,
   confidence, labels, or grid hints, but cannot delete cells, rewrite text, or
   replace provenance.
5. Model output is never labeled author-original.

## Runtime And Privacy Posture

No TableFormer, Table Transformer, ONNX, Torch, Docling, or LayoutParser model
runtime or weight is bundled. The schema accepts application-generated model
proposals and records model name, version, hash, source, license, runtime, input
metadata, preprocessing, transforms, confidence, and diagnostics.

Cloud upload remains disabled. No endpoint is called by Prompt 15. A future
application adapter must provide an endpoint, API-key environment variable,
payload policy, explicit privacy acknowledgement, timeout, retry policy, and
schema validation. Secrets are not report fields.

## Evidence

The artifact root is `target/prompt15-semantic-closeout`. It contains the
32-row audit, 20-category benchmark manifest and scorecard, generated fixtures,
binding matrices, table/RAG schemas, conflict diagnostics, reference
availability, regression summary, validation records, and HTML report.

The benchmark claim is bounded to deterministic fixtures and executed
references. It does not claim general document understanding or production ML
vision quality.
