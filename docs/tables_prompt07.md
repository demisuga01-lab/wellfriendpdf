# Prompt 07 Tables

Wellfriend's table engine remains classical and CPU-safe. Prompt 07 documents and
keeps the existing hybrid behavior rather than replacing it with a model:

1. Semantic tables from tagged/structured extraction when available.
2. Lattice mode from ruled lines, rectangles, and intersections.
3. Stream mode from Prompt 06B word geometry and row/column alignment.
4. Network/alignment heuristics inside the borderless detector for dense
   repeated x/y alignments.
5. A hybrid resolver that reports semantic/ruled candidates first and accepts
   borderless candidates only when their cell graph is regular enough to avoid
   prose/key-value false positives.

The public table model exposes rows, cells, row/column positions, spans,
headers, confidence, source mode, CSV, HTML, and JSON-friendly structures.
Spanning cells and header hierarchy are covered by the existing table structure
smoke tests.

Prompt 07 baseline:

- `target/competitive-benchmark/prompt07-tables-before`
- current scorer table cell-F1: 0.987
- TEDS approx: 0.981

Known bounded limits:

- There is no built-in ML table model. A future hook can accept external
  TableFormer/Table Transformer-style predictions.
- Network/alignment mode is part of borderless detection rather than a separate
  public algorithm enum.
- Table extraction from OCR word boxes remains bounded to OCR text extraction
  surfaces until a later table/OCR integration pass.

