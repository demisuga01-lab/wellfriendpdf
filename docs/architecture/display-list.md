# Display list

The display list is Wellfriend's immutable page-level drawing program. It is
source-linked and records both native replay operations and typed compatibility
fallbacks to the immediate content renderer.

Current design points:

- vector operations carry captured draw state;
- text, image, inline-image, and Form XObject operations can be replayed through
  native page-renderer dispatch where supported;
- fallback is explicit and counted, not silent;
- feature counters classify paths, text, images, shadings, patterns,
  transparency, optional content, annotations, and unsupported operators;
- memory estimates are exposed for bounded scheduling and cache decisions.

This task strengthens the display-list execution path by making tiles and bands
execute the canonical page program into a tile-local viewport rather than
rendering a full page and cropping it. That reduces low-resource peak pixel
allocation and makes progressive rendering a real bounded execution strategy.
