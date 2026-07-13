# Persistent Version Graph

Prompt 21 version graph support covers branch-local undo/redo, named checkpoint posture, deterministic version hashes, changed object IDs, and conflict reporting.

Rules:

| Operation | Policy |
| --- | --- |
| Undo | Moves to parent version without mutating parent state. |
| Redo | Branch-local child restoration. |
| Branch creation | Keeps sibling branches addressable by version ID. |
| Diff | Reports changed object/resource/path IDs. |
| Merge | Detects conflicts; does not auto-merge conflicting page-content edits. |

Artifacts: `persistent-version-graph-prompt21.json`, `persistent-undo-redo-prompt21.json`, and `persistent-checkpoint-restore-prompt21.json`.
