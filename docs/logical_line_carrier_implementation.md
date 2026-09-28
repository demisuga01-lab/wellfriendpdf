# Logical blank-line carriers — source implementation, not qualification

This follows `hard_line_policy_implementation.md`. The full editor/rendering
roadmap remains active. No build, type check, test, PDF generation/extraction,
rendering, benchmark, deployment, commit or push was performed for this change.

## Source change

The story writer previously emitted an `/ActualText` scope containing no
text-showing operation for a hard-separator-only line. Both collectors attach
ActualText to emitted text; saved metadata was therefore not an extraction
postcondition. The plain-text formatter also discarded all-whitespace lines and
added an inferred newline after source text already ending in a separator.

`advanced_story_carriers.rs` now stages a private Type0 font when a frame needs
logical whitespace with no visible glyphs. It uses the existing bundled DejaVu
Sans space, requires a non-notdef empty TrueType outline, and reuses the canonical
font-subsetting/embedding writer. Seven deterministic CIDs map to CR, LF, VT, FF,
NEL, LS and PS through ToUnicode. CRLF remains two exact source scalars in one
text-showing operand. All horizontal/vertical advances and origins are zero.

The carrier emits ordinary `Tr 0` text using an empty outline, not an OCR layer
or a retained copy of deleted text. Its existing line-level ActualText,
paragraph-owner and artifact scopes are retained. Identity-H/Identity-V follows
the story's writing mode; absolute placement and reset text state prevent
spacing or matrix advance from a prior line affecting the carrier. This does
not add visible characters or require a glyph in the user's approved font.

Fonts have explicit frame ownership and a private carrier marker. The existing
reachability cleanup retires unused owned carrier resources; story font
discovery excludes them from substitution candidates and saved user assets.
No public request or metadata schema change is required. Incremental history
still retains prior revisions: this is ordinary editing, not sanitizing
redaction.

After writing, the story transaction now:

1. Locates the exact frame-owned output range and parses that content.
2. Checks the private fonts' seven ToUnicode mappings and zero advances.
3. Collects text within that owner and compares the ordered carrier strings,
   ActualText presence, writing mode and zero widths with the planned lines.
4. Rejects missing/duplicate carriers. A matching string elsewhere on the page
   cannot satisfy this check; ActualText cannot conceal a damaged ToUnicode map.

These are implemented runtime postconditions, **not postconditions executed in
this source-only change**. They use the SDK's own parser/collector and are not
independent interoperability proof.

The shared formatter now retains source hard separators even on blank-only
lines and avoids appending a duplicate inferred boundary. Configured line
endings still apply to geometry-inferred boundaries; explicit source CRLF,
LS/PS and other supported separators retain their original scalar sequence.
Whitespace-only lines without an explicit hard separator retain the prior
skip behavior. Geometry reconstruction and optional heading/paragraph spacing
remain formatting policies, not an exact original-document text serialization.

## Standards basis and boundaries

PDF defines ToUnicode mappings, text rendering modes and ActualText separately.
The implementation supplies actual encoded text as well as marked-content
replacement text; it does not depend on an empty marked-content sequence being
interpreted as text. See ISO 32000-1 sections 9.3.6, 9.10.3 and 14.9.4 in the
[Adobe-hosted specification](https://developer.adobe.com/document-services/docs/assets/35e4369068f86065372c18787171a17e/PDF_ISO_32000-1.pdf).
The [PDF Association tagging guide](https://pdfa.org/download-area/publications/Tagged-PDF-Best-Practice-Guide.pdf)
also distinguishes replacement text and whitespace from generic scanned-page
text. Neither reference establishes that these unexecuted outputs satisfy
PDF/UA, PDF/A or all viewer behavior.

This closes the identified **story/table hard-only-line source path**. The later
`generated_logical_carrier_implementation.md` extends the shared carrier to
bounded and multi-run generated writers, including scoped Form/appearance
editing. It does not establish equivalent treatment for every authoring route or
arbitrary glyphless/default-ignorable input. Empty paragraphs still contain no
logical characters and do not acquire fabricated whitespace. Form feed remains
a mandatory line boundary here, not automatic page creation. General section
pagination, arbitrary vertical reading order, source/logical ownership outside
the supported story scope and the wider roadmap remain open.

## Unexecuted regression source

Thirteen new test functions cover:

- Exact/bounded separator encoding, cancellation, object-space overflow and
  absence of unused carrier allocation.
- All supported separators, CRLF, three story writing modes, direct source
  selection mappings, zero advances and extraction without ActualText.
- A damaged ToUnicode map hidden by correct ActualText, plus a missing carrier
  with a duplicate outside its owner.
- Repeated rewrite/delete/refill and obsolete carrier font retirement.
- Paragraph/artifact wrappers, story checkpoint/reopen with leading/interior/
  trailing blank lines, exclusion from the substitution font pool, table cells
  and a genuinely tagged table retaining MCID ownership and ParentTree validity.
- Blank-line formatting and explicit-versus-inferred line endings.

The tests are in `advanced_story_carrier_tests.rs`, `story_hard_break_tests.rs`,
`tagged_table_tests.rs` and `text/formatter.rs`. Their presence is not passing
evidence. Only Rust formatting/parsing and source/whitespace inspection were
performed. Exact-build execution, real save/reopen flows, independent extraction,
pixel comparison, accessibility/standards validation and the VPS corpus remain
pending. This is not a universal-editor or better-than-Acrobat completion claim.
