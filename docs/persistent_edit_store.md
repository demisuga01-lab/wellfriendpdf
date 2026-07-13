# Persistent Edit Store

Prompt 21 adds persistent structural-sharing data structures for editor history reporting:

| Structure | Implementation |
| --- | --- |
| HAMT-style map | 32-way `Arc` trie with path copying for ID-to-object state. |
| RRB-style vector | Persistent chunked `Arc<Vec<Arc<Vec<_>>>>` operation sequence. |
| Version graph | Immutable version IDs, branches, parent links, and deterministic hashes. |

The store is editor state, not PDF revision history. Saving a version does not mutate prior snapshots, and restore checks schema/hash before decoding bounded JSON.

Artifacts: `persistent-hamt-results-prompt21.json`, `persistent-rrb-results-prompt21.json`, and `persistent-memory-benchmark-prompt21.json`.
