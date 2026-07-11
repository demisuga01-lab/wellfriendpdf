# DOCX Readback Validation

Prompt 19 validates generated packages independently of the writer by reopening
the ZIP and inspecting `document.xml`, relationships, styles, numbering,
settings, media, section properties, anchors, text boxes, tables, and links.

Pass criteria are a readable package, required parts, one section per source
page in the supported corpus, exact page dimensions, stable package hash,
stable relationship/media names, and no missing hyperlink/image targets.

LibreOffice headless and Microsoft Word automation are optional external
harnesses. When present they export DOCX to PDF and record page count/sizes,
text coverage, and renderer evidence. When absent, artifacts say
`tool_unavailable`; OOXML readback is never relabeled as a Word render.
