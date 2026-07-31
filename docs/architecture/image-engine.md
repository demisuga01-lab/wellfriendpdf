# Image engine

The image renderer decodes through bounded stream filters and image codecs, then
paints image XObjects, inline images, masks, soft masks, and colour-converted
pixels through the canonical page renderer.

The execution policy is:

- do not decode more than required for the requested page/tile where the current
  codec path permits it;
- retain original image streams when rewriting unchanged content;
- enforce decompressed-size, pixel-count, recursion, and temporary-storage
  limits;
- report unsupported or malformed image combinations as typed evidence;
- keep OCR/searchable/reconstruction layers separate from the source scan.

Tile-local rendering reduces page-surface allocation for large scanned pages.
Codec-specific partial decode remains an optimization boundary to be measured
and adopted only when correctness is preserved.
