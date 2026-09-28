# Generated vertical typography: source implementation

This increment changes the existing advanced source-editing path, not a browser
overlay or raster replacement. It is uncompiled and unexecuted. No tests, PDF
workloads, render comparisons or benchmarks were run. The full roadmap remains
incomplete.

## Replaced approximations

Previously, the generated vertical path shaped horizontally, assigned a fixed
one-em advance to every output glyph, classified orientations with a few ranges,
and embedded only a default DW2 entry. It could split a shaped glyph array into
columns and lacked bounded positioned-column emission and vertical justification.

The current source connects these stages:

- `fonts/vertical.rs` itemizes complete graphemes using Unicode orientation,
  shapes upright runs top-to-bottom with Rustybuzz, and retains vertical advances,
  origins/GPOS offsets and cross-axis advances. Sideways runs use horizontal
  shaping with paragraph-derived bidi levels, then rotate their glyph outlines.
- Tr/Tu decisions compare complete cluster glyph sequences with horizontal
  shaping. A missing transformed Tr glyph uses a rotated fallback; Tu uses an
  upright fallback. Cluster lookup is indexed, not a whole-run scan per glyph.
- `advanced_vertical_text.rs` reuses `PreparedParagraph` and its logical
  line-breaking indexes. It measures proposed columns with the actual final
  shaping, including ink overhangs, and reshapes at chosen logical boundaries.
  It does not cut a visual glyph array into columns.
- The canonical Type0 builder now writes horizontal W metrics separately from
  vertical W2 advances. Generated W2 origins are explicitly zero because the
  emitted per-glyph Tm already contains the shaping origin; PDF readers must not
  subtract that origin a second time. Explicit glyph placement remains the
  authority for contextual positioning and reused CIDs.
- One vertical serializer serves ordinary and style-preserving generation,
  including explicit column rectangles. It uses actual outline bounds, tracks
  column capacity on both axes, handles top/end/center placement, and applies
  bounded justification gaps only between clusters. Source tracking is applied
  once per cluster, with source writing-mode spacing semantics retained.
- Single-token vertical replacement now writes logical ActualText, as the
  multi-run path already does. ToUnicode remains in the font. Logical text is
  therefore not inferred from rotated glyph coordinates alone. Analysis reports
  whole-input UTF-8 cluster offsets and actual vertical shaping results.

The existing source-operand deletion, font-embedding policy, incremental writer,
source paint anchor and save/reopen postconditions remain in use. The subsequent
`inline_text_position_implementation.md` increment tracks source matrices and
restores both the original line origin and endpoint, including rotated/offset
inline glyphs. Generated glyphs now carry checked layout-origin metadata for
repeat editing; text rise remains in the source axes even for sideways glyphs.

## Research/specification basis

Unicode's [UAX #50](https://www.unicode.org/reports/tr50/tr50-33.html) defines the
mixed-orientation property and transformed-glyph fallback distinction.
`vertical_orientation_data.rs` is generated from the official
[Unicode 17.0.0 VerticalOrientation data](https://www.unicode.org/Public/17.0.0/ucd/VerticalOrientation.txt).
It merges adjacent equal non-default ranges and leaves the specified R default
implicit: 189 ranges, without a new runtime dependency. The source digest is
`dcef09c3fb24d356b042569c328ec341efc5b53447700d799f2fb4834c3cd3cd`.
The Unicode license accompanies the data in `fonts/UNICODE_LICENSE.txt`.

HarfBuzz documents separate horizontal/vertical advances and glyph origins in
its [shaping guide](https://harfbuzz.github.io/shaping-and-shape-plans.html) and
[font-functions guide](https://harfbuzz.github.io/fonts-and-faces-native-opentype.html).
The integration uses the checked local Rustybuzz 0.20.1 API. These sources justify
the implementation model; they do not qualify this SDK's output.

## Remaining implementation and verification

- The subsequent `vertical_story_implementation.md` connects uniform vertical-RL
  and vertical-LR linked stories, cross-page pagination and contextual multi-font
  fallback in source. Mixed orthogonal stories and general inline mixed-style
  column breaking remain incomplete; the new paths are still unexecuted.
- Inline rotated/offset replacement now has source-matrix tracking. The later
  `ext_gstate_font_implementation.md` resolves ExtGState-only indirect fonts and
  source restoration aliases. Unresolvable font history and occurrence-specific
  Form integration remain bounded; see `inline_text_position_implementation.md`.
- The later `paragraph_line_break_implementation.md` adds bounded strict/custom
  line-edge rules and emergency-wrap controls for story/table paragraphs. It does
  not expose custom wrap policy on every original source-editing operation.
- This is default mixed orientation, not complete ruby, tate-chu-yoko, advanced
  kinsoku tailoring, every baseline convention or arbitrary vertical script
  layout. Non-default variable-font instancing and exotic font embedding remain
  separate work. Required glyph coverage/embedding permissions still apply.
- Justification cannot exceed the requested spacing limits. Overflow beyond a
  bounded region fails before publication; this increment does not pretend that
  clipping or unbounded expansion establishes fitting visible text.
- Bounds are glyph-outline bounds. Source stroke/miter expansion, filters and
  other paint effects are not a complete painted-pixel bounds guarantee.
- The vertical shaper currently has no dedicated retained shaped-run cache.
  Cancellation is cooperative between shaping calls, not inside every font call.
- W2/ToUnicode interoperability, realistic CJK fonts with vertical GSUB/GPOS,
  VORG/vmtx, mixed-direction continuations, clipping, performance and independent
  rendering all require exact-revision executable qualification.

Twelve new regression functions were added, not run: four in `fonts/vertical.rs`
and eight in `advanced_vertical_text_tests.rs`. They cover orientation data,
horizontal shaping preservation for sideways runs, raw vertical metrics, bidi
context, ink-bound placement, W/W2 separation, logical breaks, justification,
overflow, positioned columns, mixed styles and source edit/reopen/deletion.
These include in-memory PDF workflows to execute later; their existence is not
evidence those workflows pass. The font-metric checks use the same shaping
library and do not substitute for an independent renderer.

Only rustfmt parsing/format checks and source whitespace checks were performed.
