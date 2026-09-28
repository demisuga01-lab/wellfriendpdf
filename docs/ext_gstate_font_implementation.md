# ExtGState font selection: source implementation

Status: uncompiled and unexecuted. No Cargo command, build, test, PDF workload,
rendering, benchmark, deployment, commit or push was performed. This increment
does not complete the editor/rendering roadmap.

## Corrected paths

PDF can select the current font and size through `gs` and an ExtGState `/Font`
array, without a preceding `Tf`. Its font can be an indirect object absent from
the ordinary `/Resources/Font` dictionary. The previous loader normalized only
fonts already named in that dictionary; the editing scanner ignored the change,
and the position tracker deliberately invalidated such history.

The source now connects these paths:

- `ext_gstate_fonts.rs`, called from the canonical `PageResources` loader,
  resolves indirect Font arrays and size values, reuses an existing matching
  font-resource name deterministically, or creates a collision-checked internal
  alias for the original indirect font object. The source font and ExtGState
  objects are not modified during resource loading. Repeated names resolving to
  the same object choose the lexicographically first name, not HashMap order.
- The common editing scanner applies `gs` font/size changes. Multi-run discovery,
  same-width patch analysis, bounded source reflow and native OCR capture use the
  same resource-aware scanner. q/Q and ordered Contents streams retain the
  selection. A font-bearing gs can resolve the font again, but cannot invent a
  cursor after previously unmeasurable text; a new positioning origin is needed.
- `advanced_text_resources.rs` installs missing ExtGState font aliases in the
  output page's local Font dictionary in the same incremental transaction as
  text edits. This covers source-font restoration and style-preserving output
  that emits Tf. Existing generated resources are retained. Colliding aliases
  never overwrite a different output font. Shared inherited resources, original
  font objects and original ExtGState dictionaries remain untouched. The helper
  can materialize other normalized ExtGState aliases on that page too; it is not
  a minimum-object-change proof.
- Text extraction loads the font selected by gs instead of waiting for Tf.
  Rendering receives the same normalized font registry. The raw renderer now
  rebinds the selected font object even when a Form reuses the same resource name;
  a non-font gs leaves the existing object binding alone. The retained renderer's
  existing pre-resolved font binding remains in use.
- Native invisible OCR capture no longer blanket-refuses a Font-bearing gs.
  It resolves source metrics during selection/removal, then retains the original
  gs command and resource graph in the search program. No font substitution or
  invented Tf is needed in that carrier; source-position compensation still uses
  the canonical displacement writer.

Replaying the original gs after a generated edit would also replay unrelated
graphics parameters. The writer therefore restores the source font through its
installed alias and preserves the surrounding state instead.

## Design basis

Adobe's [PDF Reference, section 4.3.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.7old.pdf)
defines the font-reference/size array in the graphics state parameter dictionary.
The [PDF Association graphics-state overview](https://pdfa.org/cheat-sheets-for-pdf-for-free/)
also distinguishes text font/size selection from other graphics-state entries.
These references establish the input model, not evidence that this SDK renders
or edits the resulting files correctly.

## Regression source and remaining work

Ten unexecuted regression functions were added: seven in
`advanced_ext_gstate_tests.rs`, two in `advanced_inline_text_tests.rs` and one in
the renderer's existing test module. They cover indirect arrays/sizes, font-name
collisions, deterministic alias selection, extraction and source discovery,
same-width editing, positioned vertical inline edit/save/reopen with untouched
following text, idempotent materialization, selected invisible carrier capture,
cross-stream state, q/Q and same-name renderer object rebinding.

No regression was executed. Only source inspection, rustfmt parsing/formatting
and whitespace checks were performed. The renderer regression checks state
binding, not pixels; source-level extraction checks are not independent output
qualification.

Remaining boundaries include malformed/non-reference font objects, missing or
ambiguous CMaps, renderer font-size/profile limits, exotic font programs and
occurrence-specific nested Form editing. This does not repair every Font value,
solve arbitrary inherited Form semantics, reconstruct missing glyphs or override
embedding/signature policies. Independent rendering, current compilation,
binding execution and the difficult-PDF corpus remain required. See
`universal_editor_roadmap_tracking.md` for the full outstanding scope.
