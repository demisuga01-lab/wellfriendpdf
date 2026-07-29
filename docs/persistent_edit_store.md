# Persistent Edit Store

writer history adds persistent structural-sharing data structures for editor history reporting:

| Structure | Implementation |
| --- | --- |
| HAMT-style map | 32-way `Arc` trie with path copying for ID-to-object state. |
| RRB-style vector | Persistent chunked `Arc<Vec<Arc<Vec<_>>>>` operation sequence. |
| Version graph | Immutable version IDs, branches, parent links, and deterministic hashes. |

The store is editor state, not PDF revision history. Saving a version does not mutate prior snapshots, and restore checks schema/hash before decoding bounded JSON.

Artifacts: `persistent-hamt-results-writer_history.json`, `persistent-rrb-results-writer_history.json`, and `persistent-memory-benchmark-writer_history.json`.
