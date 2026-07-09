# CJK Search And RAG Token Integration

Dictionary segmentation feeds an additional token layer. Raw-text search remains
available, and raw extracted text is not rewritten.

Search behavior:

- token-aware exact matching can match dictionary token sequences;
- mixed Latin/CJK queries are segmented with the same provider;
- result spans preserve source byte and character offsets;
- page/object/MCID provenance is retained when source semantic chars carry it.

RAG behavior:

- token chunks can preserve dictionary phrase boundaries;
- chunks carry source offset ranges, language tags, confidence, and provenance;
- table/paragraph boundaries remain owned by the deterministic semantic model;
- when dictionary mode is disabled or unavailable, extraction falls back to the
  deterministic char/simple tokenization modes.
