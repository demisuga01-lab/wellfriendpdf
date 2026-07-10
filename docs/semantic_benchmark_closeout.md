# Semantic Benchmark Close-out

Run:

```powershell
python scripts/prompt15_semantic_intelligence_benchmark.py
```

The generator writes to `target/prompt15-semantic-closeout` and fails if an
executable Prompt 15 gate or CLI sample fails.

## Corpus

The generated corpus has 20 categories: valid and broken tags, orphan and
conflicting MCIDs, untagged text, Chinese/Japanese/Korean/mixed CJK, simple,
complex, and merged-cell tables, figure/caption, heading/section,
multi-column/reading-order stress, RAG, model proposal/conflict, and
redaction. Fixtures are synthetic and redistributable; they are not presented
as a representative production corpus.

The CJK PDFs use deterministic ToUnicode CMaps so extraction and dictionary
segmentation run through the PDF pipeline. Table truth records the generated
grid dimensions. The redaction fixture is rewritten by the real CLI and then
reopened to prove the removed term is absent.

## Metrics

Rows record text coverage, reading order, block/paragraph counts, table
precision/recall, cell matching, heading paths, CJK segmentation, search, RAG
boundaries, provenance, ParentTree diagnostics, proposal acceptance/rejection,
malformed fail-closed behavior, runtime, memory availability, and report size.

Runtime is observational and is not part of deterministic score identity.
Peak process-tree memory is recorded when `psutil` is available; otherwise the
artifact says that memory collection was unavailable.

The generated simple and complex tables currently score `1.0` cell matching.
The synthetic merged-header table scores `6/7` (`0.857142...`) because the
deterministic extractor finds six body cells but omits the merged heading. The
typed table-row/cell fixture separately proves that supplied merged-cell spans
are preserved during chunking. No perfect merged-table extraction claim is
made.

## References

The harness probes Docling, LayoutParser, pdfplumber, and Camelot. It never
downloads model weights. Docling/LayoutParser are executed only when a licensed
offline model integration is explicitly configured in a later adapter.
pdfplumber and Camelot use local fixtures when installed. Only records with
`executed=true` are comparison evidence.

The current reference artifact records an executed pdfplumber comparison with
an exact normalized text match. This is one fixture, not broad external parity.

## Claims

The scorecard closes the semantic framework and fixture contracts. It does not
claim production ML vision, full document understanding, external-model
quality, or parity with an unexecuted reference.
