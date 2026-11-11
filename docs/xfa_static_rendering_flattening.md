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
anchor table) and anchor types; display picture clauses (see *Locales*)
and PDFium's default date/number display; barcodes (see *Barcodes*).

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
applicable): GIF/TIFF images, barcode types PDFium does not render either
(Code 93, Code 11, Interleaved 2 of 5, postal codes, Aztec, MaxiCode, RSS),
interactive events, validation messages, highlight and focus.

## Form scripts

Before layout the renderer runs the form's scripts the way PDFium's
`CXFA_FFDocView` does: `initialize` (children before parents),
`calculate` (document order, then only the calculations whose inputs
changed, until values settle), `ready` on `$form` (form:ready) and
`docReady`; after layout it runs `ready` on `$layout` (layout:ready) and
lays the form out again when that changed anything. Master-page fields with
layout:ready scripts are evaluated once per page, so `xfa.layout.page(this)`
page numbers come out right on every page. Scripts change field values,
presence, captions, choice-list items, fill/border/font colours and
repeating-subform instances (`instanceManager` `setInstances`,
`addInstance`, `insertInstance`, `removeInstance`, `moveInstance`, `count`,
`_name` shorthand); new instances run their initialize scripts. Interactive
events (click, enter, exit, change...) and `runAt="server"` scripts never
run.

**Validation** runs without a user interface. After the initialize and
calculate scripts (PDFium's `InitValidate`), every field, exclusion group
and subform with a `<validate>` runs its validate script (`scriptTest`:
a false result fails), its `nullTest` (`warning`/`error` on an empty
value) and its `formatTest` (the raw value must parse or format with
`<validate><picture>`, PDFium `CXFA_LocaleValue::ValidateValue`); values
that scripts change later (docReady, layout:ready, `execCalculate`...) are
validated again. Failures are recorded, never shown:
`ContentEngine::xfa_validation_results` lists the container's SOM
expression, the test, its severity and the `<message><text
name="nullTest|formatTest|scriptTest">` text or PDFium's default message
("Name cannot be blank.", "The value you entered for Name is invalid.").
`execValidate()` and a field's `execEvent("validate")` return whether the
null and format tests pass (true before the initial pass, as PDFium skips
them until then) and run the validate scripts after the calling script;
`xfa.host.validationsEnabled = false` turns validation off. Validation
never changes what is drawn. Unlike PDFium (which runs them anyway),
`scriptTest="disabled"` scripts do not run; `inactive` containers are not
validated.

The scripting object model (`crates/engine/src/xfa/som.rs`) resolves SOM
expressions: `a.b[2].c`, `[*]`, relative `[+1]`/`[-1]`, `..name`, `#class`,
`*`, `.[FormCalc predicate]`, `$`, `$form`, `$host`, `$layout`, `$event`,
`xfa.*`, and unqualified names by XFA scoping (children, then ancestors'
children; unnamed containers are transparent; unindexed names infer the
instance from the referencing node). Properties: `rawValue`,
`formattedValue`, `isNull`, `presence`, `access`, `caption.value.text`,
`value.#text`, `fillColor`, `borderColor`, `fontColor`, `name`,
`className`, `index`, `parent`, `all`, `nodes`, `length`,
`somExpression`, `selectedIndex`, `x`/`y`/`w`/`h`; methods
`resolveNode(s)`, `execCalculate`, `execInitialize`, `execEvent`,
`addItem`, `clearItems`, `deleteItem`, `setItems`, `getDisplayItem`,
`getSaveItem`, `boundItem`, `getItemState`/`setItemState`;
`xfa.host` (`messageBox`, `name`, `numPages`...), `xfa.layout` (`page`,
`pageCount`, `absPage`, `absPageCount`, `sheet`, `h`/`w`/`x`/`y`) and
`<variables>` script objects.

**Data DOM**: the datasets packet is loaded like PDFium's
`CXFA_DocumentBuilder::ParseDataGroup` (elements with element children are
`dataGroup`s, others `dataValue`s, `xfa:dataNode` and `xfa:contentType`
honoured, attributes become `metaData` data values) and reached through
`$data` (`xfa.datasets.data`), `$record` (its first data group), `!name`
(`xfa.datasets.name`, FormCalc) and `xfa.datasets`; JavaScript also
resolves the `$data`, `$record`, `$form`, `$host`, `$layout`, `$event`
and `$template` identifiers. Data nodes offer `value`, `isNull`,
`contains`, `name`, `className`, `parent`, `index`, `classIndex`,
`somExpression`, `all`, `nodes` (live: `item`, `namedItem`, `append`,
`insert`, `remove`, `length`), `model`, `getAttribute`, `setAttribute`,
`isRecordGroup`, `saveXML`, `resolveNode(s)` and `xfa.datasets.createNode`
(`dataGroup`, `dataValue`); children win over properties of the same
name, as in SOM resolution. Fields, exclusion groups and subforms expose
the data node they were merged with as `dataNode`; named containers
without data get new data nodes (PDFium's data merge creates them).
Values stay in sync both ways: writing a bound field updates its data
value and every other field bound to it, and writing a data value
(`$record.name.value = "x"`, FormCalc `$data.a.b = 1`) updates the fields
bound to it. The data DOM is capped at 50,000 nodes.

**FormCalc** (`formcalc.rs`) is a complete interpreter: literals, null
semantics, arithmetic/logical/relational operators, `var`, `if`/`elseif`,
`while`, `for ... upto/downto ... step`, `foreach`, `do ... end`, `func`,
`break`, `continue`, `return`, accessors and method calls, and the built-in
library: arithmetic (`Abs`, `Avg`, `Ceil`, `Count`, `Floor`, `Max`, `Min`,
`Mod`, `Round`, `Sum`), dates and times (`Date`, `Date2Num`, `DateFmt`,
`IsoDate2Num`, `IsoTime2Num`, `LocalDateFmt`, `LocalTimeFmt`, `Num2Date`,
`Num2GMTime`, `Num2Time`, `Time`, `Time2Num`, `TimeFmt`), financial
(`Apr`, `CTerm`, `FV`, `IPmt`, `NPV`, `Pmt`, `PPmt`, `PV`, `Rate`, `Term`),
logical (`Choose`, `Exists`, `HasValue`, `Oneof`, `Within`), strings
(`At`, `Concat`, `Decode`, `Encode`, `Format`, `Left`, `Len`, `Lower`,
`Ltrim`, `Parse`, `Replace`, `Right`, `Rtrim`, `Space`, `Str`, `Stuff`,
`Substr`, `Uuid`, `Upper`, `WordNum`) and `Eval`, `Null`, `Ref`,
`UnitType`, `UnitValue`. `Get`, `Post` and `Put` are refused.

**JavaScript** runs on [boa](https://boajs.dev) (`boa_engine`, MIT OR
Unlicense; cargo feature `xfa-js`, on by default; not on wasm32, where
JavaScript scripts are skipped and reported as
`xfa.script.javascript_unavailable`). Form objects are `Proxy` objects over
the same scripting object model; unqualified names resolve through a
`with` scope proxy, `this` is the script's node, and `app.alert`/`console`
write to `ContentEngine::xfa_script_log`. The context has no host
bindings: no network, file system, timers or modules.

**Bounds**: one budget per document: 3 s of script time, 20 million
FormCalc steps, 400 million JavaScript VM cycles (checked every 4,096),
call depth 64 (FormCalc) / 256 (JavaScript), 1 million loop iterations per
JavaScript frame, 1 MB strings, 1,024 instances per subform, 100,000 script
DOM nodes. JavaScript runs through boa's budgeted evaluation and is
abandoned when the budget runs out (after which no more JavaScript runs for
that document); array callbacks and `eval` are re-implemented in script so
they stay inside the budget. Code reached only through native callbacks
(`Function.prototype.call`, accessors, `String.prototype.replace` callbacks)
is bounded by the per-frame loop and recursion limits rather than by the
cycle budget. Script errors stop only that script; they are listed by
`ContentEngine::xfa_script_errors` and flagged as `xfa.script.error`.

## Barcodes

`<ui><barcode>` fields draw their value as PDFium's `CFWL_Barcode` does:
Code 39 (`code3Of9`, optional mod-43 check, wide/narrow ratio), Code 128
(`code128` choosing set B or C, `code128B`, `code128C`), Codabar (start and
end characters), EAN-8, EAN-13 and UPC-A (check digits computed; drawn as
PDFium's `ShowChars` lays them out: 7-module quiet zones (left for
EAN-13, both sides for UPC-A), bars over the full height, the digit
groups in white boxes at the bottom between the guard bars, which keep
their full length, and the EAN-13 leading digit and the UPC-A
number-system and check digits beside the bars), QR Code
(error-correction levels L/M/Q/H from `errorCorrectionLevel` 0-3), PDF417
(text/byte/numeric compaction, error-correction level 0-8, columns and rows
from the widget size) and Data Matrix (ECC 200). Linear symbols fill the
widget width with human-readable text above or below (`textLocation`);
matrix symbols use PDFium's module sizing (`moduleWidth`/`moduleHeight` in
whole points, integer scaling, centred; QR and Data Matrix symbols that do
not fit are reported as `xfa.render.barcode.too_small`). QR and Data Matrix
come from the `qrcode` and `datamatrix` crates (MIT OR Apache-2.0); the
other encoders are in `barcode.rs`.

## Layout: leaders, trailers, overflow and keep

Flowed (`tb`) subforms honour `<overflow leader trailer target>` (and the
legacy `<break overflowLeader overflowTrailer overflowTarget>`): when the
subform splits, the trailer ends the old content area (its height is
reserved), the leader starts the new one, and `target` picks the content
area or page area to continue in. When a nested subform splits, every
splitting ancestor places its overflow leader and trailer too (as
PDFium's content layout processors do, level by level): the trailers end
the old area innermost first, the leaders start the new one outermost
first, all their heights are reserved, and the innermost `target` wins; a
nested subform without its own `<overflow>` carries its ancestors'.
`<bookend leader trailer>` places a leader before and a trailer after the
subform's content. `breakBefore` and
`breakAfter` `leader`/`trailer` (legacy `beforeLeader`...) are placed
around the break; `targetType="contentArea"` with a `target` jumps to that
content area. `<keep intact>` keeps a subform in one content area;
`keep next` / `keep previous` chains move to the next content area together
when they fit there (chains of up to 512 siblings; measuring stops once a
chain is taller than an empty content area, where it cannot move as a
unit). Leader and trailer subforms are resolved by `#id` or by
name and are instantiated with their template defaults.

## Locales

Picture clauses and default formats use the field's locale (the nearest
`locale` attribute; `ambient` and missing fall back to the config packet's
`acrobat|present > common > locale`, else en_US). Locales come from the
document's `<localeSet>` packet, then PDFium's built-in locale data (en_US,
en_GB, de_DE, fr_FR, es_ES, es_MX (Latin America), it_IT, pt_BR, nl_NL,
ru_RU, ja_JP, ko_KR, zh_CN, zh_TW, zh_HK; other names map by language as in
PDFium's `CXFA_LocaleMgr`). Month, day, meridiem and era names, decimal,
grouping, percent, minus and currency symbols, and the
`full`/`long`/`med`/`short` date and time and `numeric`/`currency`/`percent`
number patterns come from the locale; pictures may name a locale
(`date(fr_FR){...}`) or use the locale shorthands (`date.long{}`,
`time.short{}`, `datetime.full{}`, `num.integer{}`, `num.decimal{}`,
`num.currency{}`, `num.percent{}`). Number pictures support
`9 8 z Z s S $ % E CR DB ( ) , . v` and literals, date pictures
`D DD J JJJ M MM MMM MMMM E EEE EEEE e G YY YYYY w WW`, time pictures
`h hh H HH k kk K KK M MM S SS FFF A Z z`; `null{}`/`zero{}` alternatives are
honoured. FormCalc's `Format`, `Parse`, `Date2Num`, `Num2Date`, `Time2Num`
and the other date functions use the same pictures and locales.
