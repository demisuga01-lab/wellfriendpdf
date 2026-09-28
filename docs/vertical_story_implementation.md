# Vertical linked stories: source implementation, not qualification

This increment is uncompiled and unexecuted. No Cargo command, build, test, PDF
workload, rendering, benchmark or deployment was run. The candidate remains
uncommitted; the full editor/rendering roadmap is not complete.

## Public request and saved state

`LinkedStoryRequest.writing_mode` accepts `horizontal_tb` (the backward-compatible
default), `vertical_rl` and `vertical_lr`. Vertical modes advance down each column;
columns progress right-to-left or left-to-right respectively. The mode is part of
the preview context hash, structural-merge authority and saved story metadata.
Changing it invalidates cached layout and requires a new checkpoint receipt.
Old requests without the field retain horizontal behavior.

The universal editing route discloses writing mode in candidate identity and
the implementation report, includes vertical decisions in approval/read/write
sets, and binds them through the existing request/plan hashes. Its page-pruning
impact report now also stops claiming original-prefix preservation when pages
are removed without adding any continuation pages.

The browser editor exposes a writing-mode selector; the TypeScript request
declaration exposes the same optional field. JSON-based native session surfaces
use the canonical Rust request. No browser, WASM or native binding execution is
claimed. Preview boxes remain geometry, not a rendered replacement-text proof.

## Shared layout and shaping

`story_writing_mode.rs` adapts physical page rectangles into logical inline/block
coordinates, reusing the existing prepared-paragraph paginator. It maps output
back to physical PDF coordinates before tagging, annotation anchoring, figure
placement, pruning, persistence and public preview publication. Continuation
frames use the approved final-frame geometry. Incremental checkpoints remain in
flow coordinates; retained output geometry is converted before prefix reuse.

The existing keep/orphan/widow rules, different-width frames, exclusions,
continuation allocation, contraction and explicit page-pruning policy remain in
use. This is not a second independently implemented paginator. Geometry mapping
does not upgrade the existing bounded greedy constraints to global optimization.

`fonts/vertical_fonts.rs` provides shared multi-font coverage, shaping and actual
outline measurement for line breaking and PDF emission:

- Font assignment uses the existing bounded contextual fallback cost model, with
  vertical glyph coverage. Font spans cannot divide extended graphemes.
- Upright runs use the canonical vertical shaper and its vertical GSUB/GPOS
  handling. Sideways runs retain paragraph-derived bidi levels across font changes
  before rotation, rather than reversing each font fragment independently.
- Measurement retains per-glyph offsets, contextual advances and cross advances,
  plus outline overhangs. The physical left/right extents are mapped to the
  appropriate block-axis ascent/descent without mirroring outlines.
- `advanced_story_vertical.rs` emits the same shaped runs with explicit matrices.
  It validates finite physical bounds against the approved frame before returning
  output. Per-glyph layout-origin markers retain repeat-edit positioning context.
- Frame font resources have separate horizontal/vertical CID namespaces and
  Type0 encodings. Existing approved story-wide font persistence and conservative
  retirement of unreferenced owner resources remain in use.

Ordinary story font size/color remain paragraph properties. This does not add
arbitrary inline style runs or recover absent font semantics.

## Tables, figures, annotations and tags

Supported table rows/cells, decorations and text use the same logical-axis
layout. Rows progress in the block direction and cells along the inline direction;
physical preview rectangles and grids are converted together. Existing table
ownership, fragmentation and repeated-artifact-header rules are retained. Table
column weights, row minima, padding and spacing are interpreted in the logical
layout axes; they are not additional physical x/y constraints.

Explicit images remain upright with their requested physical width/height. Their
reserved layout footprint swaps axes, so supported caption keep rules and pinned
exclusions apply before physical placement. Figure alignment and gap act along
the inline and block axes. Annotation offsets remain **physical PDF x/y offsets**
from the first painted line origin; they do not rotate annotation appearances.

The supported changed paragraph/table owners receive current Layout/WritingMode
attributes (`LrTb`, `RlTb`, `TbRl` or `TbLr`). Obsolete class/direct WritingMode
attributes are materialized and replaced per owner without modifying shared
ClassMap data. Existing geometry attributes are invalidated by the common repair
path. Untouched siblings do not receive the changed paragraph's direction.

Tagged `vertical_lr` requires an input declaring PDF 2.0 in the header or catalog
Version: `TbLr` is a PDF 2.0 value. Older tagged documents require a separate,
explicit version migration. This operation does not silently upgrade a PDF or
claim to preserve a conformance profile through that upgrade.

## Specification basis

The logical-axis model follows the distinction between inline and block flow in
[CSS Writing Modes](https://www.w3.org/TR/css-writing-modes-3/); it is an internal
coordinate model, not a claim that this SDK implements CSS layout.
PDF Layout attributes and table progression are defined in
[ISO 32000-1](https://opensource.adobe.com/dc-acrobat-sdk-docs/standards/pdfstandards/pdf/PDF32000_2008.pdf).
The additional `TbLr` value is defined in
[ISO 32000-2](https://developer.adobe.com/document-services/docs/assets/5b15559b96303194340b99820d3a70fa/PDF_ISO_32000-2.pdf).
The Unicode/OpenType shaping references and orientation data are documented in
`vertical_typography_implementation.md`. Specifications are design evidence, not
execution evidence for the SDK.

## Regression source and remaining work

Nineteen unexecuted vertical regression functions were added: five in
`fonts/vertical_fonts_tests.rs`, ten in `story_writing_mode_tests.rs`, two in
`advanced_story_vertical_tests.rs` and two in `tagged_story.rs`. They cover
multi-font/bidi/orientation agreement, grapheme boundaries, physical ink bounds,
RL/LR geometry, exclusions, keep constraints, empty breaks, incremental layout,
continuation growth, contraction/refill, save/reopen fonts/mode, tables, figures,
tag direction updates and the PDF-version gate. Font/geometry checks share native
shaping code and do not substitute for an independent renderer.
One additional unexecuted regression in `story_page_pruning_tests.rs` covers the
universal report's byte-prefix claim with and without page removals.

The later `paragraph_line_break_implementation.md` adds bounded Japanese-strict
and custom line-edge tailoring plus protected emergency-wrap settings to this
shared paginator, saved metadata, table paths and browser controls. It is not a
full Japanese typesetting engine; its added regressions remain unexecuted.

Still outside this increment: mixed orthogonal writing modes inside one story,
ruby, tate-chu-yoko, full Japanese typography, all baseline conventions, arbitrary
variable/exotic font programs, unavailable glyph recovery, general nested source
Forms, and the other items in `universal_editor_roadmap_tracking.md`. Typeface
licensing, source ownership, signed-document policies and resource budgets remain
enforced. Painted-pixel bounds (stroke/filter effects), extraction order and
assistive-technology behavior require independent executable qualification.

Only source inspection, rustfmt parsing/formatting and whitespace checks have
been performed on the Rust changes. JavaScript syntax checking is not browser
execution. This is neither a universal-editing nor a better-than-Acrobat claim.
