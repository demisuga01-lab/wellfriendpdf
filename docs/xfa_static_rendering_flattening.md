# Static XFA rendering and flattening

Oxide lays out supported fields, captions, values, borders, basic check marks, draw text/lines/rectangles, and simple subforms into page items. Preview and flatten write ordinary overlay page content with the existing PDF editor/writer, then reopen through the normal renderer and record page hashes. This is not a separate raster-only renderer.

Modes are `extract_only`, `render_preview`, `flatten_supported_static`, `flatten_and_remove_xfa`, `preserve_unsupported_xfa_report_only`, and `fail_on_unsupported`. Static modes fail closed on a dynamic classification. Remove mode also fails on exact unsupported constructs. Overlay writing preserves unrelated page content, skips layout items for unavailable generated pages with a diagnostic, and never claims a generated page was inserted.

All mutations use a full rewrite. Reopen, page count, XFA presence, render hashes, visible-item count, deterministic bytes, and signature impact are in the report. Full rewrite invalidates existing signature byte ranges and may violate DocMDP/FieldMDP.
