# Deterministic Writer Closure In Prompt 08B

Prompt 08B adds an explicit deterministic-save report:

```rust
PdfEditor::save_to_bytes_with_options(mode, &DeterministicSaveOptions::default())
```

Reported policy:

- full rewrite uses deterministic object traversal and the existing xref-stream/object-stream writer mode.
- incremental save appends deterministic plain objects for the same input/edit/options.
- resource names use stable prefixes and deterministic next-name scanning.
- the first file ID is preserved when available.
- fixed PDF date policy can be supplied and is reported; the edit writer does not inject wall-clock metadata in the tested path.
- compression uses existing deterministic Flate settings.
- signature invalidation warning is reported when signatures exist.

Tests:

- repeated incremental save with the same input/edit/options has the same SHA-256 digest.
- output preserves the original byte prefix in incremental mode.

Limits:

- cryptographic signature preservation/validation is Prompt 09.
- high-effort compression and advanced object-stream packing are later optimization work.
