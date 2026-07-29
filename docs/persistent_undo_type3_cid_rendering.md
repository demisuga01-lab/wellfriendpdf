# Persistent Undo And Snapshot Model In Type3 CID Rendering

Type3 CID Rendering strengthens `EditTransactionLog` without adding a complex HAMT/RRB dependency.

Chosen design:

```text
stable object IDs -> edit transactions -> edit patches -> bounded digest checkpoints
```

Added structures:

- `EditPatch`: operation, stable target IDs, before/after text, diagnostics.
- `EditCheckpoint`: sequence, document text digest, transaction count, block count, text bytes.

Behavior:

- multi-edit undo/redo works for paragraph text operations.
- redo history is discarded after a branch edit.
- checkpoints are capped by `max_checkpoints` to avoid unbounded memory growth.
- transaction JSON is deterministic and suitable for audit/reporting.

Limits:

- checkpoints store compact digests and counts, not full structural snapshots.
- transaction import/replay is future SDK polish.
- a full persistent vector/map store can be added later if multi-user editing requires it.
