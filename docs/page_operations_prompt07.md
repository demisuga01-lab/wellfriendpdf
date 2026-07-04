# Prompt 07 Page Operations

Prompt 07 adds a page-operation audit report so callers can understand what
must be preserved before running organize/rotate/extract/merge-style operations.

## Reported

- page count
- per-page MediaBox, CropBox, rotation, object id, and annotation count
- outline presence and bounded outline-node count
- page-label presence
- named-destination presence
- embedded-file name-tree presence
- AcroForm presence
- rewrite signature-invalidation risk

## Existing Operations

The existing structural utilities cover merge, split, extract pages, organize,
insert pages, and rotate pages. Full-rewrite editing now retains only objects
reachable from the updated root and info dictionaries, which prevents stale
redacted streams from remaining in output bytes while preserving reachable
catalog structures such as outlines, name trees, attachments, and AcroForms.

## Limits

- Crop/scale/n-up/impose are not promoted beyond existing smoke-level support.
- Rewriting page labels, outlines, and named destinations after arbitrary page
  reindexing remains bounded page-operation polish.
- Existing signatures may be invalidated by page operations. Prompt 09 owns
  full signature validation and preservation policy.

