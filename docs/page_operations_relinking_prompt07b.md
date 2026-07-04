# Prompt 07B Page Operations and Relinking

Prompt 07B separates graph-preserving page operations from visual page
reassembly.

## Graph-Preserving

- `rotate_pages` and `crop_pages` mutate leaf page dictionaries.
- Forms, annotations, outlines, labels, named destinations, attachments, and
  catalog structures remain reachable.
- Signature invalidation risk is reported; cryptographic preservation is not
  claimed.

## Visual Reassembly

- `scale_pdf_pages` rasterizes selected pages and places them on fresh pages.
- `n_up_pdf` rasterizes selected pages into a grid on fresh sheets.
- These operations are deterministic and safe for visual output, but they do
  not claim interactive relinking.

CLI JSON reports the preservation mode for each operation.

## Limits

- Arbitrary destination retargeting after delete/reorder/impose remains bounded
  unless the source graph is preserved.
- N-up interactive widget/link relinking is intentionally not claimed.

