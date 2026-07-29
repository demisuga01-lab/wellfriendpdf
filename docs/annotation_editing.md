# Annotation Editing

Prompt 34 uses `PdfEditor` for supported text, highlight, stamp, and URI-link
creation; source-content edits and rectangle-scoped deletion; and canonical
flattening. Prompt 17 XFDF import supplies the broader source-linked path for
supported geometry, reply, popup, and appearance records. Each changed output
is reopened and then passes through canonical appearance generation.

Unsupported annotation types, malformed geometry, unsafe actions, and invalid
reply relationships fail before output. The API never relies on a viewer to
invent a missing appearance for a supported edited annotation.

Prompt 34 reuses canonical annotation identity, geometry, XFDF, and appearance
generation. Supported appearance regeneration writes real appearance streams;
unsupported annotation geometry returns an exact refusal.

`annotation_move_resize` resolves the stable canonical XFDF identity, scales
rectangle-linked QuadPoints, vertices, line endpoints, callouts, and ink lists,
then imports only that preserved source record. The operation regenerates a
supported appearance while leaving unrelated annotations untouched; complex
new geometry remains explicit XFDF input rather than guessed from a rectangle.

`annotation_create_reply` verifies the stable parent ID in the current source
snapshot and creates a real `/IRT` reply relationship through the same canonical
XFDF importer. Missing or stale parents return `reply_relationship_invalid`
without modifying the PDF.
Table-linked annotation movement is available when a caller supplies a stable
table ID, cell coordinate, and canonical annotation ID. The action uses the
resolved cell bounds, regenerates the supported annotation appearance, and
does not move unrelated annotations or table source content.

OCR-link creation binds a reviewed generated searchable-text rectangle to a
canonical URI link annotation. The operation preserves the original scan and
uses the same page-space geometry for the invisible text and link rectangle.
