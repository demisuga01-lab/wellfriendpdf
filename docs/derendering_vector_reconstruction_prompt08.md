# De-rendering And Vector Reconstruction In Prompt 08

Prompt 08 introduces the shared editable model and documents the boundary between
implemented editable reconstruction and research-grade de-rendering.

Implemented now:

- text blocks are reconstructed from semantic/layout extraction.
- tables use the Prompt 07 table grid model.
- images are represented as editable placeholders with page placement metadata.
- simple version/dedup sketches support unchanged resource detection.
- existing annotations and ink paths remain handled by Prompt 07 surfaces.

Vector reconstruction posture:

- existing vector paths can be preserved by the renderer/display-list and page
  operation paths.
- Prompt 08 does not claim arbitrary path grouping into semantic editable
  shapes.
- simple line/rectangle preservation remains a future page-shape editing polish.

Curve fitting:

- no large model is bundled.
- ink annotations keep original points through the Prompt 07 annotation model.
- CPU-safe polyline-to-Bezier fitting is documented as a bounded follow-up
  rather than a Prompt 08 blocker.

Raster-to-vector:

- deep vectorization of scanned pages is research work.
- optional threshold/contour vectorization may be added later if bounded and
  clearly separate from OCR.

Font reconstruction:

- generating replacement fonts for rasterized or subset glyph-only text is not
  implemented.
- text editing uses safe removal plus replacement drawing through the authoring
  font path.
