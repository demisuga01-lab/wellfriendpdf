# RAG Chunking Model

Prompt 06 does not replace the existing RAG chunker. It documents how the chunker relates to the new semantic text model.

## Existing Path

The `chunk` command and `crate::chunk` module operate over the canonical parsed document:

```text
ContentEngine::parse_document
-> Document body blocks
-> section-aware chunks
-> JSON chunk records with page/source metadata
```

This remains the primary RAG-facing path because it already understands headings, tables, figures, page references, block kinds, and serialization policy.

## Prompt 06 Additions

The new semantic text model adds lower-level geometry:

- words
- spans
- characters
- quads
- provenance
- confidence

Future RAG chunking can use these fields for span-level citations and highlight previews without changing chunk boundaries.

Prompt 06B adds MCID, StructTree role, and provenance summaries to the same
model. RAG citations should prefer spans/search matches whose provenance is
native PDF text, ToUnicode, or tagged ActualText, and should surface
low-confidence, hidden/OCR, or unknown provenance to callers.

## Recommended Use

- Use `oxide chunk` for production RAG ingestion.
- Use `extract-text --structured --format model-json` when an application needs character quads or match geometry.
- Use `ContentEngine::search_text` for query-time source highlighting.

## Limits

- The chunker is not switched to the Prompt 06 text model by default.
- Table chunking remains tied to the existing table/document model.
- OCR chunking still depends on the existing OCR seam and policy.

## Prompt 15 Advanced Path

`crate::advanced_rag` is now the additive provenance-aware path. Use
`oxide chunk --advanced` or `oxide semantic-export --view chunks` when chunks
need source spans, bboxes/quads, MCIDs, ParentTree status, CJK dictionary
metadata, stable hashes, table/cell IDs, and explicit security posture. The
legacy chunk schema remains available and unchanged.
