# Word Pagination Failure Audit

The form action policy taxonomy separates structural OOXML, semantic readback, visual
raster, pagination, and editor-dependent results. It covers page size/margins,
sections/page breaks, headers/footers, font metrics/line wraps, anchors/z-order,
images/crops, tables/merges/row splitting, paragraph pagination controls,
columns, notes, lists/fields, rotated/vertical/RTL/CJK text, links/bookmarks,
comments/forms, clipping, floating objects, and unsupported PDF constructs.

The baseline Type3 CID Rendering writer used one hard-coded A4 `sectPr`, so mixed page
sizes and landscape pages could not preserve pagination. form action policy emits one
exact-size section per parsed PDF page and deterministic next-page section
breaks. Page-faithful and hybrid layouts use page-relative anchors; flowing mode
retains native paragraphs/tables but still preserves explicit page sections.

Metrics include page/section count, page sizes in twips, paragraph/text-box/
table/merge/image/hyperlink counts, package part count, byte size, readback
status, and repeat hash. LibreOffice/Word export results are populated only when
those tools actually run.
