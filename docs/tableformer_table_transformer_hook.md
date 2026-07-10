# TableFormer And Table Transformer Hook

`table_intelligence` defines the optional table-model proposal contract. The
contract is compatible with TableFormer/Table Transformer style outputs, but it
does not require either runtime and does not bundle model weights.

## Proposal Schema

`TableProposalSet` contains:

- model backend, name, version, hash, source, license, and runtime metadata;
- input pages and payload type;
- renderer, color space, resizing, normalization, image limits, and affine
  model-to-PDF coordinate transform;
- privacy mode, explicit upload and acknowledgement flags;
- table regions with confidence and reading-order hints;
- row and column boundaries with geometry and confidence;
- cells with bbox/polygon, row/column, row/column span, role, confidence, and
  optional proposed text;
- source span and MCID provenance;
- per-element and set-level diagnostics, runtime, and memory observations.

The machine schema is
`target/prompt15-semantic-closeout/table-proposal-schema-prompt15.json`.

## Validation

Validation fails closed for unsupported schema versions, invalid confidence,
non-finite or inverted geometry, duplicate IDs or boundary indexes, zero spans,
overlapping proposed cells, row/column kind mismatches, page zero, absent or
duplicate input pages, empty model metadata, inconsistent or author-original
proposal provenance, malformed coordinate transforms, resource-cap breaches,
or unauthorized cloud payloads. One invalid proposal set is not partially
merged.

Default caps are 4,096 table proposals, 8,192 row or column boundaries per
table, 250,000 proposed cells per table, four input pages, 2,048 pixels on the
long image side, 1,200 DPI, five seconds reported runtime, and 256 MiB reported
memory.

## Deterministic Merge

`merge_table_proposals_deterministic` sorts proposals by page, descending
confidence, then ID. The default region threshold is `0.82`, element threshold
is `0.78`, deterministic association IoU is `0.20`, and competing proposal IoU
is `0.70`.

- A high-confidence proposal overlapping a deterministic table can enrich the
  table through a separate overlay.
- A high-confidence proposal without a deterministic match remains a candidate
  region.
- A low-confidence proposal remains a suggestion.
- A lower-priority overlapping proposal is rejected with a conflict diagnostic.
- Proposed text disagreement is reported and deterministic text is retained.
- A proposed cell outside the deterministic grid is reported and not accepted.

Even if a caller sets unsafe policy booleans, deterministic tables, cells,
text, spans, MCIDs, and provenance remain authoritative. The flags are ignored
and `table.merge.policy_hardened` is emitted. Model elements always carry
inferred proposal provenance and `author_original=false`.

## Backend Boundary

`table_model_backend_status_report` reports a complete hook with no bundled
runtime. A local adapter would need a user-supplied licensed model path,
existing renderer input, deterministic preprocessing, a 5-second default
timeout, 256 MiB memory limit, four-page batch cap, and 2,048-pixel image-side
cap. These are contract defaults, not a claim that an ONNX/Torch adapter is
present.

There is no production cloud table provider. Cloud remains disabled and no
network operation occurs in tests or the SDK default path.
