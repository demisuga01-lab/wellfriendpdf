# PDF Authoring

Selected TTC/OTC faces can be registered through
`PdfBuilder::register_font_face_bytes(name, bytes, &FontFaceSelection)`. Inspect
with `fonts::font_asset::inspect_font_asset` first and supply the exact source
hash and chosen face index. The method returns `(FontFace, FontPreparationReport)`;
collection signature removal requires explicit approval. It preserves font tables
and uses the existing TrueType/CFF1 embedding path, not a CFF2/static-variable
conversion. See [collection preparation](font_collection_preparation_implementation.md).

Wellfriend can create new PDFs from scratch with `PdfBuilder`. The authoring layer
builds a normal PDF object graph and serializes it through the existing writer,
so authored output uses the same xref-stream/object-stream machinery as the
structural writer.

```rust
use wellfriendpdf_engine::authoring::{PageSize, PdfBuilder};
use wellfriendpdf_engine::{Color, GraphicsStyle, StandardFont, TextStyle};

let mut doc = PdfBuilder::new();
doc.set_title("Report").set_author("Wellfriend");

let page = doc.add_page(PageSize::LETTER);
page.draw_text(
    "Quarterly report",
    72.0,
    720.0,
    &TextStyle::standard(StandardFont::HelveticaBold, 18.0),
)?;
page.draw_rect(
    72.0,
    680.0,
    180.0,
    24.0,
    &GraphicsStyle::fill_stroke(
        Color::device_rgb(0.92, 0.95, 0.98),
        Color::device_rgb(0.1, 0.2, 0.3),
        1.0,
    ),
);

doc.save("report.pdf")?;
# Ok::<(), wellfriendpdf_engine::WellfriendError>(())
```

## Coordinates

Authoring uses native PDF user space: the origin is the bottom-left corner of
the page, x grows right, and y grows upward. `draw_text("x", 72, 720, ...)`
places the text baseline one inch from the left edge and ten inches above the
bottom of a US Letter page.

For UI-style top-left positioning, use `PdfPageBuilder::pdf_y_from_top()` or
`draw_text_from_top()`.

## Pages

`PageSize` provides common sizes (`LETTER`, `LEGAL`, `A3`, `A4`, `A5`), custom
point sizes, and inch/mm helpers:

```rust
let portrait = PageSize::A4;
let landscape = PageSize::A4.landscape();
let badge = PageSize::inches(3.5, 2.0);
let custom = PageSize::custom(300.0, 200.0);
```

Margins can be attached to a page for layout helpers with
`add_page_with_margins`, but primitive drawing APIs always accept explicit PDF
coordinates.

## Text And Fonts

The authoring API supports:

- All PDF Standard-14 faces via `StandardFont`: Helvetica, Times, Courier,
  Symbol, and ZapfDingbats.
- A bundled Unicode baseline via `FontFace::BuiltinUnicode`, embedded as a
  Type0 TrueType font with ToUnicode and CIDToGIDMap.
- Editable standalone TrueType and OpenType/CFF1 programs via
  `PdfBuilder::register_font_bytes`. TrueType uses Type0/CIDFontType2 with
  Identity-H and CIDToGIDMap. CFF1 uses CIDFontType0 with FontFile3/OpenType and
  an Encoding CMap targeting native CIDs. Both retain widths and ToUnicode.
- `draw_text`, `draw_text_line`, `draw_text_from_top`.
- `wrap_text` and `draw_paragraph` with left, center, and right alignment plus
  exact left/right/center/decimal tab stops through `ParagraphStyle::tab_stops`.
- Gray, RGB, and CMYK fill colors through the shared `Color` type.

Standard fonts use WinAnsi encoding. If text contains characters outside
WinAnsi, use `TextStyle::unicode(...)` or `FontFace::BuiltinUnicode`.

```rust
let serif = doc.register_font_bytes(
    "LiberationSerif",
    include_bytes!("../crates/engine/fonts/LiberationSerif-Regular.ttf").as_slice(),
)?;
doc.add_page(PageSize::LETTER).draw_text(
    "Unicode custom font: cafe \u{03c0}",
    72.0,
    720.0,
    &TextStyle::new(serif, 12.0),
)?;
# Ok::<(), wellfriendpdf_engine::WellfriendError>(())
```

Permitted TrueType fonts use the existing GID-preserving subsetter, with reported
full-program fallback. Fonts prohibiting subsetting retain the full program;
fonts prohibiting editable outline embedding are refused. CFF1 embeds whole.
The writer raises the declared PDF version to at least 1.6 for OpenType output.
See `cff_encoding_implementation.md` for the current source-only verification
boundary and remaining font/encoding limits.

## Graphics

The graphics API supports line, rectangle, rounded rectangle, circle, ellipse, polygon,
and arbitrary paths. Each draw is wrapped in `q`/`Q` and can set stroke color,
fill color, line width, line cap/join, and dash pattern through `GraphicsStyle`.

## Images

Register images on the document and place them on pages with `draw_image`.

- JPEG bytes are embedded directly with `DCTDecode`; they are decoded only to
  read dimensions and channel count.
- PNG bytes and raw RGB/RGBA samples are embedded as Flate-compressed image
  XObjects.
- Gray/RGB alpha channels are emitted as grayscale `/SMask` image XObjects.

```rust
let jpeg = doc.add_jpeg_image(std::fs::read("photo.jpg")?)?;
let rgba = doc.add_rgba_image(2, 2, vec![
    255, 0, 0, 255, 0, 255, 0, 180,
    0, 0, 255, 120, 255, 255, 0, 64,
])?;

let page = doc.add_page(PageSize::LETTER);
page.draw_image(jpeg, 72.0, 560.0, 144.0, 96.0);
page.draw_image(rgba, 240.0, 560.0, 96.0, 96.0);
# Ok::<(), wellfriendpdf_engine::WellfriendError>(())
```

## Flow page breaks

Flow paragraphs
interpret U+000C as a physical next-page command, and
`add_page_break_to(FlowPageBreak::{NextPage,NextOddPage,NextEvenPage})` exposes
explicit parity. Page-local paragraphs and table cells reject U+000C because
they cannot safely own the successor page/grid transaction.

## Sections, running content and page labels

`FlowDocument::from_section` gives every flow page an explicit section owner.
Sections define page geometry, margins, optional first-page masters, odd/even
masters and PDF page-label numbering. `start_section_on` can begin the next
section on the next, next odd, or next even physical page. Any parity page
inserted before the destination remains owned by the preceding section and does
not paint a running master.

```rust
use wellfriendpdf_engine::{
    FlowDocument, FlowPageBreak, FlowSection, Margins, PageNumberField,
    PageNumberStyle, PageSize, RunningText, RunningTextPart,
    SectionPageMaster, TextAlign, TextStyle,
};

let footer = RunningText::new(
    vec![
        RunningTextPart::Text("Page ".into()),
        RunningTextPart::Field(PageNumberField::SectionPage),
        RunningTextPart::Text(" of ".into()),
        RunningTextPart::Field(PageNumberField::SectionLastPage),
    ],
    TextStyle::unicode(9.0),
)
.align(TextAlign::Center)
.baseline_from_edge(24.0);

let body = FlowSection::new(PageSize::LETTER, Margins::all(72.0))
    .mirrored_margins(true)
    .odd_master(SectionPageMaster::new().footer(footer.clone()))
    .even_master(SectionPageMaster::new().footer(footer));
let mut flow = FlowDocument::from_section(body)?;

let appendix = FlowSection::new(PageSize::A4.landscape(), Margins::all(54.0))
    .first_master(SectionPageMaster::new().header(
        RunningText::literal("Appendix", TextStyle::unicode(9.0))
            .baseline_from_edge(24.0),
    ))
    .page_numbering(1, PageNumberStyle::LowerRoman);
flow.start_section_on(appendix, FlowPageBreak::NextOddPage)?;
# Ok::<(), wellfriendpdf_engine::WellfriendError>(())
```

Tabs remain U+0009 in logical text and are positioned fields rather than font
glyphs or guessed spaces. `draw_text` and `text_width` therefore reject tabs;
use a paragraph API with explicit/default stops:

```rust
use wellfriendpdf_engine::{ParagraphStyle, TabAlignment, TabLeader, TabStop, TabStops};

let paragraph = ParagraphStyle::new().tab_stops(TabStops {
    stops: vec![
        TabStop { position: 144.0, alignment: TabAlignment::Left, decimal: '.', decimal_token: None, leader: TabLeader::Dots, bar: false },
        TabStop { position: 288.0, alignment: TabAlignment::Decimal, decimal: '.', decimal_token: Some(".-".into()), leader: TabLeader::None, bar: true },
    ],
    default_interval: 36.0,
});
page.draw_paragraph(
    "Description\tAmount\nService\t125.-",
    72.0,
    700.0,
    360.0,
    &TextStyle::unicode(11.0),
    &paragraph,
)?;
# Ok::<(), wellfriendpdf_engine::WellfriendError>(())
```

Stops are measured from the paragraph's inline-axis origin. Dot, dash and solid
leaders span only the resolved unoccupied gap, while `bar: true` paints a
perpendicular rule at the stop. Both are artifacts rather than logical text;
see `tab_stop_layout_implementation.md` for exact limits and qualification
status.

Running text is resolved on a private serialization clone after the final page
and section counts are known. It is marked as a pagination artifact, must fit on
one line entirely inside the reserved top or bottom margin, and never mutates
the caller's builder. `DocumentPage`/`DocumentPages` use physical PDF pages;
`SectionPage` uses the configured label start; `SectionPages` is the section's
physical page count; `SectionLastPage` is its final label number. Roman labels
are intentionally bounded to 1..=3999. Non-default section numbering is also
written to the catalog's `/PageLabels` number tree.

Changing a section's page size or margins after its pages have been laid out is
rejected. Marker-level note references and automatic section
inference from imported PDFs are separate features, not implied by this API.
When mirrored margins are enabled, even physical pages swap left/right margins;
page width and top/bottom reservations remain unchanged.

## Footnotes and endnotes

Footnotes bind an explicit, nonempty UTF-8 marker range in the body paragraph.
The marker remains in the paragraph and is reused as the note label, preventing
numbering text from silently drifting away from its source reference.

```rust
use wellfriendpdf_engine::{FlowEndnote, FlowFootnote};

let text = "A source-backed claim¹";
let marker = text.find('¹').unwrap();
let footnote = FlowFootnote::new(
    marker..marker + '¹'.len_utf8(),
    "Primary source citation.",
    TextStyle::unicode(8.0),
);
let receipt = flow.add_paragraph_with_footnotes(
    text,
    &TextStyle::unicode(11.0),
    &ParagraphStyle::new(),
    &[footnote],
)?;

let endnotes = [FlowEndnote::new(
    "1.",
    "Document-level note.",
    TextStyle::unicode(9.0),
)];
flow.add_endnotes(&endnotes, FlowPageBreak::NextOddPage)?;
```

When the marker is not already present, configure the section's
`FootnoteNumbering` and use `add_numbered_footnoted_paragraph`. Each
`NumberedFootnote` supplies an exact UTF-8 insertion boundary. The returned
report contains the original offset, assigned number/label, enriched marker
range and the downstream layout receipt. Section scope restarts at the section's
configured start; document scope continues across section changes. Failed
layout rolls the counter back.

Footnote space is reserved at the physical page bottom during pagination, so
later paragraphs, images and authored tables cannot paint through it. Long notes
continue with contiguous source-range receipts; decorative separator rules are
artifacts. Note text is painted only on the private final serialization clone.
Endnote collections validate all labels before allocation, can begin on explicit
page parity, and flow through ordinary pages transactionally.

Fresh footnote/endnote bodies emit stable, unique-ID `/Note` structure elements
and a StructTreeRoot `/IDTree`; their body paragraph uses `/P`. Exact source
marker ranges become ordered `/Reference` children, ordinary text becomes
`/Span`, and reciprocal `/Ref` relationships connect each marker and `/Note`.
The writer retains the complete line's shaping and refuses a marker boundary
that divides an OpenType cluster. The API does not yet infer references,
renumber arbitrary pre-existing markers or migrate imported-PDF notes.

## Named anchors and body fields

Flow authoring can bind the current page/cursor to a stable name and reference
that anchor before or after it is declared. Named anchors are emitted as PDF XYZ
destinations. Body fields can resolve the current/final document page, current
section page/count/last label, or an anchor's document/section page.

```rust
use wellfriendpdf_engine::{BodyField, BodyFieldFormat, BodyFieldPart};

flow.add_field_paragraph(
    &[
        BodyFieldPart::text("See Appendix on page "),
        BodyFieldPart::field(
            BodyField::AnchorDocumentPage("appendix".into()),
            BodyFieldFormat::new(4).link_to_anchor(true),
        ),
        BodyFieldPart::text("."),
    ],
    &TextStyle::unicode(10.0),
    &ParagraphStyle::new(),
)?;
flow.add_page_break();
flow.add_anchor("appendix")?;
```

`max_characters` is an explicit pagination contract. Layout uses a conservative
placeholder, retains its exact line boundaries, then resolves final values on a
private serialization clone with whole-paragraph bidi and fallback context. An
unknown anchor, a value exceeding capacity, or final glyph geometry exceeding
the reservation rejects serialization. Extracted logical text contains the real
value rather than placeholder padding. Fields do not silently repaginate.

Anchor-page fields may opt into a native `/Link` annotation with
`link_to_anchor(true)`. Final materialization measures the resolved value's
actual shaped advance cells (including bidi and approved fallback runs), checks
the hitbox against the owning page, emits a nonvisual annotation border and
points `/Dest` at the same named XYZ destination. Overlapping or duplicate
generated hitboxes fail closed. Field paragraphs use `/P`; clickable fields
receive checked `/StructParent`, ParentTree and `OBJR` relationships. Positioned
tab fields now participate in reserved-field geometry, while inline style changes
inside one deferred field and broader accessibility relationships remain
separate work.

## Document outline

A fresh document can publish a nested native PDF outline whose items point to
the same stable authored anchors. Targets may be declared after the outline is
configured; final serialization resolves the complete tree atomically.

```rust
use wellfriendpdf_engine::PdfOutlineEntry;

flow.set_outline(vec![
    PdfOutlineEntry::new("Introduction", "intro"),
    PdfOutlineEntry::new("Appendices", "appendices").children(vec![
        PdfOutlineEntry::new("Appendix A", "appendix-a"),
        PdfOutlineEntry::new("Appendix B", "appendix-b").open(false),
    ]),
])?;

flow.add_anchor("intro")?;
// ... authored content ...
flow.add_anchor("appendices")?;
flow.add_anchor("appendix-a")?;
flow.add_anchor("appendix-b")?;
```

When the visible heading and outline should share one authority, use the atomic
capture API instead:

```rust
flow.add_outlined_heading("Introduction", 1, "intro")?;
flow.add_outlined_heading("Background", 2, "background")?;
flow.add_outlined_heading("Methods", 2, "methods")?;
flow.add_outlined_heading("Results", 1, "results")?;
```

Levels cannot be skipped. A shaping, font-coverage or pagination failure rolls
back the heading text, cursor, anchor, outline item and hierarchy stack together.

The writer emits bounded indirect outline objects with exact parent, sibling,
child and open/closed count relationships, then opens the outline panel through
the catalog. This is viewer navigation; it does not paint a table of contents
onto document pages or create an accessibility heading tree.

The same explicit outline can be painted at the current flow position:

```rust
use wellfriendpdf_engine::TableOfContentsStyle;

let toc = flow.add_table_of_contents(&TableOfContentsStyle::new())?;
assert!(!toc.rows.is_empty());
```

Titles wrap in a hierarchy-indented column. Page values occupy an independent
fixed-width right-aligned column, remain deferred until final pagination and
become native clickable links to their anchors. Optional dot leaders are
measured rather than clipped. The call is one rollback-capable transaction; it
does not insert front-matter pages ahead of content already authored.
`page_side`, `title_align` and `page_align` support a physical left page column
and right-aligned mixed-direction titles without changing logical text.
`level_styles` supplies validated exact-level typography, indentation, spacing,
alignment and leader overrides. Bidirectional `keep_with_next` and
`keep_with_previous` relationships keep bounded hierarchy chains on one page
whenever the chain fits an empty page.

At serialization the explicit TOC receives `/TOC` and `/TOCI` structure
elements. Page fragments use MCIDs, and generated page-value links are associated
with their owning rows through ParentTree/`OBJR` relationships. This is source
support, not a PDF/UA conformance claim.

When the body already exists, the TOC can be staged into a distinct front
section and atomically prepended:

```rust
use wellfriendpdf_engine::{
    AuthorPageSize, FlowSection, Margins, PageNumberStyle, TableOfContentsStyle,
};

let front = FlowSection::new(AuthorPageSize::A4, Margins::all(48.0))
    .page_numbering(1, PageNumberStyle::LowerRoman);
let report = flow.prepend_table_of_contents(front, &TableOfContentsStyle::new())?;
assert_eq!(report.inserted_pages % 2, 0);
```

An automatic suppressed parity page keeps every existing body page on its
original odd/even side. Anchors, section ownership, deferred-field identities
and retained footnote references shift in the same splice transaction.

For complete title/copyright/dedication front matter, stage ordinary authoring
operations in the general transaction:

```rust
let report = flow.prepend_front_matter(front, |front| {
    front.add_paragraph(
        "Document title",
        &TextStyle::unicode(24.0),
        &ParagraphStyle::new().align(TextAlign::Center),
    )?;
    front.add_page_break();
    front.add_table_of_contents(&TableOfContentsStyle::new())?;
    Ok(())
})?;
```

New front sections, section-scoped notes, images, fonts and anchors merge only
after the callback succeeds. Document metadata and the established outline are
not mutable through this staging transaction.

A back-of-document index can bind terms to anchors that already exist:

```rust
use wellfriendpdf_engine::{DocumentIndexStyle, PdfIndexEntry};

let entries = vec![
    PdfIndexEntry::new("Compression").anchors(["compression-a", "compression-b"]),
    PdfIndexEntry::new("Rendering")
        .anchor("rendering")
        .see_also("Colour management"),
];
let index = flow.add_document_index(&entries, &DocumentIndexStyle::new())?;
assert_eq!(index.rows.len(), 2);
```

Occurrences sort by physical page, collapse repeated occurrences on one page
and become clickable deferred page values. Configurable consecutive-page ranges
retain clickable endpoints and never cross a section-number reset. Sorting may
preserve authored order, use deterministic Unicode-scalar ordering or consume
explicit application-provided locale collation keys.

The explicit index receives an `/Index` container and `/P` row elements. Wrapped
rows retain one semantic identity across pages, including their generated link
annotation ownership.

## Tables

`TableBuilder` renders fixed-width columns, optional header rows, borders,
fills, padding, wrapped cell text, and per-column alignment. `draw_on_page`
draws a table at an explicit top-left anchor; `FlowDocument::add_table` handles
page breaks and repeats the header row on continuation pages.

```rust
use wellfriendpdf_engine::{TableBuilder, TableCell, TableColumn, TextAlign};

let mut table = TableBuilder::new(vec![
    TableColumn::new(96.0),
    TableColumn::new(260.0).align(TextAlign::Left),
])
.caption("Performance summary")
.summary("Metrics with explicit row and column header relationships");
table.set_header(["Metric", "Notes"]);
table.add_row([
    TableCell::text("Throughput").row_header(),
    TableCell::text("Wrapped text is measured from glyph widths."),
]);
table.push_row(
    wellfriendpdf_engine::TableRow::new(vec![
        "Appendix".into(),
        "This row begins on the next odd physical page.".into(),
    ])
    .page_break_before(FlowPageBreak::NextOddPage),
);
```

Row-owned page commands are honored only by `FlowDocument::add_table`. They
repeat the table header at the destination, account for parity blanks in
`TableFlowReport::page_breaks`, and participate in the surrounding append
rollback. `draw_on_page` refuses them because a page-local operation cannot own
successor pages.

Flowed tables emit `/Table`, `/Caption`, `/THead`, `/TBody`, `/TR`, `/TH` and
`/TD` structure elements. Column and caller-declared row headers carry `/Scope`, receive stable
IDs in the structure IDTree, and each data cell emits explicit `/Headers`
associations. Optional summaries remain on `/Table`; split cells keep one
identity across fragments. `TableCell::column_span(n)` occupies `n` consecutive
fixed-grid columns, paints one merged cell and emits `/ColSpan`; its data-cell
header associations cover every occupied column. Cross-row spans remain a
separate pagination feature. `TableCell::row_span(n)` uses a table-wide occupancy
plan and row-height constraint graph; fitting connected span blocks paint and
paginate atomically and publish `/RowSpan`. Connected blocks taller than one
fresh page continue only at measured row/shaped-line boundaries with exact
source-range receipts; impossible line-minimum combinations fail atomically.
Captions are kept with a feasible first table
fragment or the append refuses before mutation, and their page/geometry is
reported in `TableFlowReport::caption`.
Only the first header is semantic, while repeated headers and cell backgrounds
are artifacts. The page-local `draw_on_page` API has no document structure
registry and therefore does not make this accessibility guarantee.

Fresh tables may reuse the SDK's exact typed-value engine. Give the table a
stable `.identity(...)` and create value cells with `TableCell::typed(id,
TableValue)`. Decimal coefficients are strings with an explicit scale; formulas
are dependency-sorted, cycles and missing/non-numeric references fail, arithmetic
is checked, and display-scale reduction refuses implicit rounding. Evaluation
occurs before layout on a private clone, so the caller's cell model is unchanged.
`TableFlowReport::evaluated_values` records the exact strings actually shaped and
painted. Flow-authored typed tables also persist their IDs, topology, exact value
graph and evaluated strings in a bounded catalog-owned registry.
`load_authored_typed_tables` reopens that registry, independently reevaluates the
graph and rejects inconsistent stored results.
`inspect_authored_typed_table_sources` additionally verifies the exact private
table/cell marked-content owners and their finite inner-cell content regions,
then returns page-logical source ranges without word matching.
Regions must remain inside their page's effective crop box; private metadata in
an untrusted or modified PDF is never accepted as arbitrary drawing authority.
`mutate_authored_typed_table` binds the request to the input hash, updates exact
values/formulas, recalculates dependents, dynamically rebinds every exact owner,
source-edits changed single-fragment cells inside that retained region and
replaces the registry atomically before reopening and rechecking it. Old output
without bounded region ownership is refused instead of receiving a default-page
overlay. Clearing a value retains one zero-width operand inside the same exact
owner, so a later refill remains source-addressable. A continued value may
collapse into its first existing owned rectangle when the complete replacement
fits there; the transaction then clears the remaining exact fragments while
retaining their zero-width owners. Redistribution, growth, fragment allocation
requires a persisted layout model. One fully typed, headerless single-row table
on a simple page template can allocate canonical continuation pages for every
sibling cell and bind each new MCID back to its original TH/TD owner. With
`prune_empty_continuations: true`, later contraction removes only empty pages
carrying the exact table/cell-count provenance and unchanged non-cell content
digest; annotations, additional page state, live incoming references and
original/unmarked pages are retained with machine-readable reasons. Repeating
headers, multiple body rows, mixed/partially typed rows and complex page masters
still fail closed. The same two operations have JSON SDK and C ABI entry points.
Java, .NET, Python and WASM expose the same inspect/mutate JSON envelopes;
Python/WASM and the additive C/Java/.NET overload accept approved font bytes.
Page-local `draw_on_page` has no document registry.

## Flow Layout

`FlowDocument` is a single-column layout helper over `PdfBuilder`. It tracks a
cursor, wraps paragraphs, inserts headings, lists, images, tables, spacers, and
creates new pages automatically when content reaches the bottom margin.

Flow paragraphs, field paragraphs and headings emit standard `/P` and
`/H1`-`/H6` structure elements. Lists emit `/L`, `/LI`, `/Lbl` and `/LBody`.
Use `add_figure(image, width, height, alt)` for meaningful images; it requires
alternate text and emits `/Figure`. `add_image` is the compatibility path for a
decorative image and is enclosed in `/Artifact` marked content.

Set the document language explicitly when authoring tagged content:

```rust
flow.builder_mut().set_language("en-US")?;
```

The bounded RFC 5646/BCP 47 grammar is validated before the value is written to
catalog `/Lang`; the engine does not guess a language from text.

Once any flow structure element exists, final serialization requires every
painting command to belong to a semantic element or an explicit artifact.
Combining tagged flow content with unowned page-local drawing fails closed.

```rust
use wellfriendpdf_engine::{FlowDocument, Margins};

let mut flow = FlowDocument::new(PageSize::LETTER, Margins::all(72.0));
flow.add_heading("Report", 1)?;
flow.add_paragraph(
    "Flowed text wraps within the page margins and continues on new pages.",
    &TextStyle::standard(StandardFont::Helvetica, 11.0),
    &ParagraphStyle::new(),
)?;
flow.add_table(&table)?;
flow.save("flow-report.pdf")?;
# Ok::<(), wellfriendpdf_engine::WellfriendError>(())
```
