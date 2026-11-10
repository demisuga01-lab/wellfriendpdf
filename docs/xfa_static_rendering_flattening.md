# Static XFA rendering and flattening

Wellfriend lays out supported fields, captions, values, borders, basic check marks, draw text/lines/rectangles, and simple subforms into page items. Preview and flatten write ordinary overlay page content with the existing PDF editor/writer, then reopen through the normal renderer and record page hashes. This is not a separate raster-only renderer.

Modes are `extract_only`, `render_preview`, `flatten_supported_static`, `flatten_and_remove_xfa`, `preserve_unsupported_xfa_report_only`, and `fail_on_unsupported`. Static modes fail closed on a dynamic classification. Remove mode also fails on exact unsupported constructs. Overlay writing preserves unrelated page content, skips layout items for unavailable generated pages with a diagnostic, and never claims a generated page was inserted.

All mutations use a full rewrite. Reopen, page count, XFA presence, render hashes, visible-item count, deterministic bytes, and signature impact are in the report. Full rewrite invalidates existing signature byte ranges and may violate DocMDP/FieldMDP.

## Page rendering (XFA foreground and full XFA pages)

The page renderer draws XFA forms the way PDFium does (`xfa/fxfa`), from a
model built once per document (`crates/engine/src/xfa/render.rs`): the
template is merged with the datasets packet (`form.rs`: `match="once"`,
`global`, `dataRef`, `none`, repeating subforms from `<occur>` and data
groups, `<bindItems>` choice lists), laid out (`layout.rs`) and painted into
PDF content (`paint.rs`, `text.rs`, `picture.rs`, `style.rs`).

**Form type** (`ContentEngine::xfa_form_type`): `Full` when the config packet
says `<dynamicRender>required</dynamicRender>` (PDFium `kXFAFull`), otherwise
`Foreground` (`kXFAForeground`); `None` without a usable template.

**Render contract switch** `RenderContract::xfa` (`XfaRenderPolicy`):

| policy | effect |
| --- | --- |
| `auto` (default) | On a *static* (`Foreground`) form, a page gets the XFA overlay only when it has no widget annotations or at least one widget without a normal appearance (`/AP /N`). Pages whose widgets all have appearances render exactly as before. Dynamic forms' own (placeholder) PDF pages are left alone. |
| `foreground` | Every XFA document gets the overlay on its PDF pages. |
| `off` | Never draw XFA. |

PDFs without `/AcroForm /XFA` are never affected by any policy, and `auto`
is omitted from the serialized contract, so existing cache identities are
unchanged. `FormRenderPolicy::Exclude` also suppresses the overlay. When
the overlay is drawn, the page's widget annotations are skipped: the XFA
fields replace them (as in PDFium, where XFA widgets are the page's form
layer). On a static page that has its own content stream only fields and
exclusion groups are painted (the page already carries the static draws);
a page without content gets everything.

**Drawn**: field values (datasets merge, template defaults), captions with
placement, reserve, margins and para alignment; fonts (typeface, size,
weight, posture, underline, line-through, colour; embedded `/AcroForm /DR`
fonts of the same family when the PDF has non-subset ones, else standard-14
stand-ins with Unicode fallback); `<para>` alignment, margins, indent,
spacing and line height; borders with per-edge thickness, colour, presence,
dash styles, `hand`, rounded/chamfered corners and lowered/raised/etched/
embossed 3-D strokes; solid, linear, radial, pattern and stipple fills;
check buttons and radio buttons (check/circle/cross/diamond/square/star
marks, on/off/neutral from items and exclusion groups); combo boxes (drop
button) and list boxes (selection); text, numeric, date-time and password
edits, multi-line wrapping and comb fields with cell separators; push button
captions; draws (text, rich text XHTML subset, lines, rectangles, arcs,
images); images from inline base64 data or `/Names /XFAImages` (JPEG, PNG,
BMP; external `href`s are never fetched); rotation (90/180/270 with PDFium's
anchor table) and anchor types; display picture clauses `num{}`, `date{}`,
`time{}`, `text{}` (en_US) and PDFium's default date/number display.

**Full XFA pages**: `ContentEngine::xfa_page_count` and
`ContentEngine::render_xfa_page` render dynamic forms from the XFA layout —
page sets, page areas (`medium`, `occur max`, ordered occurrence and simple
`pagePosition`), content areas, positioned/`tb`/`lr-tb`/`rl-tb`/table/row
flows, growable sizes (`minW`/`maxW`/`minH`/`maxH`), splitting of
top-to-bottom subforms across content areas, `breakBefore`/`breakAfter` and
legacy `<break>` targets, and overflow onto new pages — so the page count
can exceed the PDF's own. `ContentEngine::xfa_layout_pdf` returns those pages
as a standalone PDF. Static forms report their PDF page count and render the
PDF page with the overlay forced on.

**Not drawn** (reported by `ContentEngine::xfa_render_unsupported` where
applicable): scripts (JavaScript/FormCalc events do not run, so script-set
values, presence and captions are not applied), barcodes, GIF/TIFF images,
leaders/trailers and overflow targets, `keep` chains, interactive highlight
and focus, and locales other than en_US for picture clauses.
