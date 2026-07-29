# Transparency Rendering Annotations

Transparency Rendering adds a typed annotation report and preserves the existing common
annotation editing path.

## Reported Fields

- subtype
- page and annotation index
- rect
- contents
- flags
- color
- QuadPoints grouped into eight-number quads
- appearance stream presence
- action kind, target summary, and safety classification
- source object when indirect
- diagnostics

Unsafe actions such as JavaScript, Launch, and SubmitForm are reported but never
executed.

## Supported Behavior

- Read/report all annotation subtypes as data.
- Existing editor creates common Highlight, Text, Stamp, and Link annotations.
- Existing editor can flatten common added annotation visuals into page content.
- Transparency Closeout flattens common existing annotations: Highlight, Underline,
  StrikeOut, Squiggly, FreeText, Ink, Line, Square, Circle, PolyLine, Polygon,
  Stamp fallback, Text, and FileAttachment icon fallback.
- Redaction removes overlapping annotations through the editor's redaction path.

## Limits

- Full appearance generation for every annotation subtype is not claimed.
- Complex popup relationships and rich FreeText styling are reported where
  present but not fully regenerated.
- Action execution is intentionally out of scope.
