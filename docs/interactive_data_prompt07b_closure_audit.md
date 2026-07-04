# Prompt 07B Interactive/Data Closure Audit

Prompt 07B closes the practical SDK gaps left after Prompt 07 without
reopening tables, fonts, color, parser, or semantic extraction.

| Feature | Prompt 07 status | Prompt 07B target | Implementation result | Tests | Remaining limit |
| --- | --- | --- | --- | --- | --- |
| FDF export | Not implemented | Fields-only exchange | Implemented via `export_form_data(..., Fdf)` | `fdf_xfdf_form_exchange_roundtrips_field_values`, CLI smoke | Annotation FDF is not imported/exported |
| FDF import | Not implemented | Fill matching AcroForm fields | Implemented via bounded parser and `apply_form_data_pdf` | Same | No action/JavaScript execution |
| XFDF export | Not implemented | Fields-only XML | Implemented with XML escaping | Same | Annotation XFDF intentionally unsupported |
| XFDF import | Not implemented | Safe fields-only parser | Implemented with DTD/entity rejection | module test `xfdf_rejects_external_entities` | No external entity/network support |
| JSON form import/export | Report/fill only | Exchange format | Implemented through `FormDataSet` | CLI smoke | Binding exposure remains SDK polish |
| Text field appearance regeneration | Basic fill/flatten | Preserve common path | Existing regenerated widget AP retained | form flatten tests | Rich text unsupported |
| Checkbox/radio appearance regeneration | Basic | Preserve common path | Existing `/AS`/AP update retained | form tests | Producer-specific button glyphs bounded |
| Choice field/list/combo appearance regeneration | Basic | Preserve common path | Existing choice fill/appearance retained | form tests | Rich list styling bounded |
| FreeText appearance | Reported | Basic flatten coverage | Basic rect/text flatten for existing annotations | annotation flatten test | Rich text unsupported |
| Text markup appearance | Added annotations only | Existing annotation flatten | Highlight, underline, strikeout, squiggly flatten from QuadPoints | annotation flatten test | Complex rotation is approximated |
| Ink appearance | Reported | Flatten common ink paths | InkList polyline flatten | compile/tests through flatten path | Smoothing not implemented |
| Line/square/circle/polyline/polygon appearance | Reported | Flatten common shapes | Basic stroke geometry flatten | compile/tests through flatten path | Advanced line endings bounded |
| Stamp appearance | Added stamp only | Missing stamp fallback | Label-box fallback for missing AP | annotation tests | Named artwork is preserved only if present |
| File attachment annotation handling | Reported | Remove under policy | Region/all policy removes FileAttachment annotations | attachment policy test | Visual icon is basic |
| Popup relation handling | Reported | Avoid false paint | Popup skipped during flatten | annotation flatten test | Popup editing bounded |
| Annotation flattening | Added annotations only | Existing common classes | `PdfEditor::flatten_annotations` flattens common visuals and removes annotations | `annotation_flattening_removes_common_annotations_but_keeps_widgets`, CLI smoke | Widgets remain unless form flattening requested |
| Crop pages | Bounded | Metadata-preserving crop | `crop_pages` writes `/CropBox` in preserved graph | `crop_pages_persists_crop_box_and_preserves_text`, CLI smoke | Does not transform content |
| Scale pages | Smoke-level | Deterministic visual scale | Raster visual copy with explicit interactivity warning | `visual_scale_and_nup_outputs_reopen`, CLI smoke | Navigation/forms are not relinked into raster pages |
| N-up/impose | Smoke-level | Deterministic visual n-up | Raster visual imposition | same | Interactive structures intentionally omitted |
| Page labels relinking | Reported | Preserve/diagnose | Structural crop/rotate preserve graph; raster scale/n-up diagnose loss | docs/CLI JSON | Arbitrary relinking after visual imposition is Prompt 08/09 policy work |
| Outlines/bookmarks relinking | Reported | Preserve/diagnose | Preserved for graph-preserving ops; diagnosed for visual ops | page report tests | Deleted-page destination rewrite remains bounded |
| Named destinations relinking | Reported | Preserve/diagnose | Preserved for graph-preserving ops; diagnosed for visual ops | page report tests | Arbitrary destination retargeting bounded |
| Annotation/form page-reference relinking | Reported | Preserve/diagnose | Preserved for crop/rotate; visual ops omit with diagnostics | CLI JSON | Full relink for n-up is intentionally not claimed |
| Partial image redaction | Full image removal | Pixel-level rewrite | Implemented for decoded 8-bit gray/RGB image XObjects with axis-aligned placement | `partial_image_redaction_rewrites_pixels_and_preserves_uncovered_area` | Unsupported formats fall back/remove/fail by policy |
| Attachment removal | Metadata scrub only | Clear policy | `AttachmentRedactionPolicy` with keep/remove-all/remove-overlapping | `attachment_removal_policy_drops_embedded_file_name_tree` | Associated-files edge cases are report-only |
| Metadata scrub | Implemented | Preserve | Retained default scrub behavior | redaction tests | Full privacy sanitizer remains Prompt 09 |
| Unsafe action removal | Reported/overlap removal | Preserve under redaction | Overlapping annotations/links are removed; actions never executed | Prompt 07 tests | Global action sanitizer is Prompt 09 |
| XFA detection/static export | Detected | Honest boundary | Detection/report retained | forms report tests | Dynamic XFA runtime unsupported |
| Signature invalidation warning | Reported | Honest boundary | Reports invalidation risk; no crypto claim | page report tests | Full signature validation/preservation is Prompt 09 |

Prompt 07B status: implemented for the interactive/data closure subset. Full
signature validation, dynamic XFA, and exhaustive page-destination retargeting
remain outside Prompt 07.
