# Explicit authoring font stacks - source implementation, not qualification

This increment extends the Rust fresh-document authoring API. It is not a new
imported-document editor and does not qualify the universal-editor roadmap.

## Implemented source path

`PdfBuilder::register_font_stack` accepts an ordered list of registered custom,
builtin Unicode, Standard-14 or previously registered stack handles. It returns
`FontFace::Fallback(FontStackId)`, usable in the existing `TextStyle` without a
new required struct field. Page text, paragraph, table-cell and flow methods
resolve this handle before appending drawing commands.

Stacks flatten nested declarations, deduplicate physical members, reuse identical
declarations and share immutable registry/font assets across pages and document
clones. Late registration updates existing pages. Validation happens before
registry mutation; later failure restores the prior registries. Drawing commands
capture exact physical custom-font assets, so transferring a page cannot silently
rebind its existing glyphs to another document's same-numbered font handle.

Standard-14 stack entries are an **explicit opt-in** to the corresponding bundled
TrueType program. They are registered as physical embedded fonts, not left to the
reader's substitute font. Existing Standard-14 styles retain their old behavior;
there is no system-font search or automatic substitution outside the declared
stack. Stack registration checks the shared embedding permission policy. The
subsequent `cff_encoding_implementation.md` increment extends standalone TrueType
support to OpenType/CFF1 with its own Type0/FontFile3/native-CID route. Collections,
CFF2 and arbitrary programs are not silently passed to the Type2 writer.

The shared contextual resolver now accepts borrowed font programs as its
internal core. Existing transaction-asset entry points delegate to it. Authoring
does not copy font programs into temporary approved-asset objects per line.
Assignment uses ordered costs and the existing bounded font-switch penalty,
switching at contextual script/whitespace boundaries rather than assembling a
joining word from individually covered characters. No approved complete unit
means an error, not arbitrary glyph substitution.

A prepared paragraph retains its bidi/context analysis and complete font spans.
Line fitting and final runs use those spans and paragraph-derived line levels.
Commands retain the exact shaped glyph runs used by measurement. Font planning
checks their coverage and feeds the existing Type0/CID/subset writer, preserving
per-occurrence advances and offsets. Mixed-font lines have one outer ActualText
owner; individual visual runs do not introduce competing logical-text owners.
Hard-break/default-ignorable-only lines keep the existing private zero-width
logical carrier rather than claiming visible output from a stack member.

Authoring measures signed glyph advances with the same pen movement used by its
writer, including backward movement and final ink/advance bounds. This geometry
shares the existing line-metrics implementation through an explicit progression
policy. Older imported-editor emitters still use their absolute-advance policy;
this change does not silently change that separate contract. Measurement rejects
nonfinite size/positions and polls cancellation.

## Read-only substitution disclosure

`PdfPageBuilder::preview_font_stack(text, width, style)` uses the same paragraph
plan without appending commands. It reports line/source UTF-8 ranges, logical
text, measured bounds, each visual run's physical face and exact SHA-256, fallback
choice, direction, glyph count and advance. Control-only lines disclose the
private carrier separately and have no stack paint runs. These are data previews,
not rendered pixels or claims of visual equivalence to the unavailable font.

Example API sequence (not executed):

```rust,ignore
let mut pdf = PdfBuilder::new();
let primary = pdf.register_font_bytes("ApprovedPrimary", primary_bytes)?;
let secondary = pdf.register_font_bytes("ApprovedSecondary", secondary_bytes)?;
let stack = pdf.register_font_stack(&[primary, secondary])?;
let style = TextStyle::new(stack, 12.0);
let page = pdf.add_page(AuthorPageSize::LETTER);
let disclosure = page.preview_font_stack(text, 450.0, &style)?;
// Present disclosure / obtain any application-required appearance approval.
page.draw_paragraph(text, 72.0, 720.0, 450.0, &style,
                    &ParagraphStyle::default())?;
```

`FontStackId`, `FontStackLinePreview` and `FontStackRunPreview` are exported from
the authoring module, crate root and prelude. Font handles remain document-local.
Consumers with exhaustive `FontFace` matches must handle the new variant.

## Verification boundary

Twelve regression functions were added in `authoring_fallback_tests.rs`: mixed
complementary subsets and exact writer glyphs, wrapped bidi/source ranges,
explicit Standard-14 conversion/deduplication, registration failure/cancellation,
uncovered-unit and geometry rollback, indivisible contextual units, transferred
page asset identity, cloned registry snapshots, table/flow and list rollback,
logical-only carriers, borrowed/owned-core parity, and signed pen movement.
Save/reopen assertions are present in source, not executed evidence.

Integration inspection also corrected three existing regression fixtures in
`table_layout.rs`, `tagged_story.rs` and `tagged_table_tests.rs` that referenced
the nonexistent `FontFace::Helvetica` variant. They now select
`FontFace::Standard(StandardFont::Helvetica)`. This is a static source correction,
not a compiler-confirmed result.

Only rustfmt formatting/parser checks and Git whitespace inspection were run.
No compiler, Cargo, builds, tests, PDF workloads, rendering, benchmarks, binding
execution, browser QA or deployments were run. No commit or push was made.

## Still open

- Runtime correctness and interoperability of these paths, including repeated
  saves, independent extraction, font appearance and font permission behavior.
- Contextual assignment probes units separately; final line shaping validates
  coverage but does not search every alternative context-dependent assignment.
- Beyond the subsequent OpenType/CFF1 increment, broader authoring font-program
  kinds, variable/color fonts, vertical/ruby,
  explicit language/features/composition controls and tab-stop layout.
- Authored row fragmentation is extended by the subsequent
  `authored_table_pagination_implementation.md` increment; merged/nested cells,
  accessibility tagging, sections and footnotes remain open.
- This new authoring API is not wired to every managed binding or browser UI.
- Per-glyph selection geometry inside ActualText, retained-session performance,
  resource budgets under real documents, and the rest of the editor/rendering
  roadmap remain unqualified.

The implementation does not establish universal editing or superiority to
Acrobat. The full roadmap remains active and incomplete.
