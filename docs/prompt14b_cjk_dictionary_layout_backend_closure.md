# Prompt 14B CJK Dictionary And Layout Backend Closure

Prompt 14B closes the strict Prompt 14 gap around production dictionary handling.
The default engine remains deterministic and offline. Raw extracted text is not
rewritten; dictionary segmentation is an optional token layer with provenance.

Implemented:

- a CJK dictionary provider/index layer;
- a manifest-plus-TSV user dictionary pack format;
- SHA-256 entry-file verification;
- language/script, source, license, version, hash, redistribution, and memory
  metadata in reports;
- deterministic longest-match segmentation with stable priority/order
  tie-breaking;
- token-aware search and RAG helper proof artifacts;
- precise local ML no-runtime policy and disabled cloud posture.

No large third-party dictionary is bundled. Applications provide production
dictionary packs, or a future feature-gated asset can be added only after
redistribution and license evidence is explicit.

Prompt 15 consumes this provider directly for semantic binding token pages,
semantic search, and CJK-aware RAG boundaries. It does not change the Prompt
14B raw-text, hash, license, entry-count, or memory-cap guarantees.
