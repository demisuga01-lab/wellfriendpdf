# Prompt 07B Annotation Appearances

Prompt 07B expands common annotation flattening. The implementation generates
page-content visuals from existing annotation dictionaries and then removes the
flattened annotations from `/Annots`.

## Flattened Classes

- Highlight from `/QuadPoints` or `/Rect`.
- Underline, StrikeOut, and Squiggly from `/QuadPoints`.
- FreeText as a basic rectangle plus plain text.
- Ink from `/InkList`.
- Line from `/L`.
- Square and Circle from `/Rect`.
- PolyLine and Polygon from `/Vertices`.
- Stamp fallback as a labeled box when no usable AP is available.
- Text and FileAttachment as small icon boxes.

Widgets are not removed by annotation flattening; use form flattening for
widgets.

## Limits

- Rich FreeText, complex callouts, line endings, blend modes, and rotated
  annotation appearance details are approximated.
- Existing appearance streams are not rasterized wholesale.
- Link and Popup annotations are removed during flattening but not painted.

