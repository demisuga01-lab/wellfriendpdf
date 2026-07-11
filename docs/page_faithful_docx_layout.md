# Page-Faithful DOCX Layout

Prompt 19 strengthens the existing native OOXML writer rather than adding a
second converter.

Supported page-faithful behavior includes exact per-page width/height,
portrait/landscape sections, deterministic page breaks, zero-margin page-
relative anchors, styled paragraph/run content inside text boxes, stable z
order, anchored images, native confident tables, spans/vertical merges,
repeated header rows, no-split rows, external hyperlinks, stable relationship
IDs, deduplicated media, settings, and fixed metadata timestamps.

Flowing mode emits native paragraphs, headings, lists, tables, inline images,
and pagination controls. Hybrid mode deterministically keeps confident semantic
title/heading/list/table blocks flowing and positions lower-confidence content.

This is measured page-faithful reconstruction, not perfect Word reproduction.
Font substitution, line breaking, clipping, arbitrary vector/blend effects,
vertical/rotated text, dedicated header/footer parts, footnote/endnote
promotion, Word comments/content controls, and editor layout disagreements are
reported exactly when not represented.
