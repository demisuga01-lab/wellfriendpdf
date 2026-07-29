# Undo, redo and history

## Scope

editing transactions extends the canonical source editing provenance and operator-editing path. It does not introduce a second parser, renderer, font engine or binding-specific editor. All mutation surfaces route through the shared Wellfriend PDF SDK engine and canonical writer.

Topic: Undo, redo and history.

## Implemented contract

Exact evidence means a report carries stable snapshot, object, stream, instruction, scene-node, grapheme, shaping-cluster or font-subset identifiers with source provenance. Inferred evidence is labeled as heuristic or unavailable rather than promoted to an exact source fact.

## Validation posture

Raw command logs are retained under the editing transactions VPS result folder. Published artifacts contain sanitized status, hashes and reproducibility commands.

## Known limits

text reflow owns broad geometric and semantic reflow. editing transactions refuses or escalates layout overflow, unsupported shaping/subset reconstruction, proprietary font restrictions, ambiguous provenance, and unsafe text clipping instead of painting overlays or silently altering neighboring content.
