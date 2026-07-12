# Combined Prompt 20 Advanced Editing Audit

## Starting state

Prompt 20 began from the exact required checkpoint.

- HEAD: `61551f934238beddc21008944c75583dc144628f`
- subject: `Complete combined prompt 19 form js interactive docx layout`
- worktree: clean
- classification: `exact_expected_start`

The machine-readable record is generated at
`target/prompt20-advanced-editing/prompt20-starting-state.json`.

## Canonical paths audited before implementation

Prompt 20 extends shared engine models and writers. It does not add a CLI-only
editing implementation or language-specific mutation logic.

| Domain | Canonical path(s) reused |
|---|---|
| editable blocks, provenance, transactions, undo/redo | `crates/engine/src/editable.rs` |
| paragraph editing, redaction rewrite, deterministic content serialization | `crates/engine/src/editing.rs` |
| content operators/tokenization/graphics state | `crates/engine/src/content/operation.rs`, `content/parser.rs`, `content/state.rs` |
| generated-text shaping and bidi ordering | `crates/engine/src/fonts/shaper.rs`, `text/reading_order.rs` |
| Type0/CID/CMap/vertical metrics | `crates/engine/src/fonts/resolver.rs`, `fonts/cmap.rs`, `fonts/cid.rs`, `text/collector.rs` |
| Unicode Type0 embedding model | `crates/engine/src/authoring.rs`, `fonts/sfnt_subset.rs` |
| page/content stream ownership | `crates/engine/src/document.rs`, `reader.rs`, `writer.rs` |
| vector/path reconstruction and rendering | `crates/engine/src/analysis/graphics.rs`, `render/path.rs`, `render/page_renderer.rs` |
| annotation/XFDF ink data and static appearances | `crates/engine/src/prompt17.rs`, `form_exchange.rs` |
| signature-aware mutations | `crates/engine/src/prompt18.rs`, `signature.rs` |
| semantic/search/RAG provenance | `crates/engine/src/semantic*.rs`, `advanced_rag.rs`, `chunk.rs` |
| shared reports and public surfaces | `crates/engine/src/sdk.rs`, CLI and binding crates/directories |
| reference and closure harness conventions | `scripts/prompt18b_advanced_secure_mutation_audit.py`, `scripts/prompt19_interactive_docx_audit.py` |

## Boundaries

- Existing PDF glyph codes are decoded and preserved when safely patchable; they
  are not blindly reshaped.
- Newly generated complex-script text uses bounded shaping and explicit font
  mapping. Missing glyphs fail closed.
- Direct patches require proven byte, glyph-count, encoding, writing-mode, and
  advance compatibility.
- Vector objects represent actual bounded operator ranges, not inferred
  semantic shapes.
- Ink fitting preserves configured raw points and reports measured error; it
  does not claim recovery of pressure, tilt, velocity, or original pen timing.
- Structural incremental preservation and cryptographic signature validity are
  separate report fields.

## Implemented shared engine paths

- `analyze_advanced_text_reflow` records UAX #9 logical/visual runs, embedding
  levels, shaped GIDs, clusters, advances, offsets, vertical orientation, and
  exact missing-glyph diagnostics.
- `edit_advanced_text_pdf` removes one provenance-resolved source string,
  embeds a deterministic Type0/CIDFontType2 font with CID-to-GID and ToUnicode
  maps, writes Identity-H/Identity-V text, updates page resources and contents,
  saves incrementally, reopens, and proves replacement extraction plus old-text
  absence.
- `analyze_same_width_patch` and `apply_same_width_patch` retain lexical byte
  ranges and string representation for Tj/TJ/quote operators, enforce existing
  font/CMap/glyph/advance compatibility, replace one stream object, and prove
  original-prefix preservation.
- `list_vector_objects` reconstructs actual path/paint ranges with stable IDs,
  graphics state, clipping, marked-content, OCG, and resource provenance.
  `edit_vector_object` performs bounded geometry/style/delete/duplicate edits
  while verifying unrelated decoded prefix and suffix. Reachable Forms are
  inventoried to depth eight; explicit edit-all and top-level clone-edit-one
  policies prevent accidental shared-resource mutation and report clone graphs.
  Page-owned safe-context z-order operations and contiguous marked-content
  group/ungroup preserve the incremental prefix and reopen successfully.
  Indirect annotation appearance paths participate in the same stable inventory
  and operation-range editor; shared appearance streams fail closed.
- `fit_ink_stroke` implements cleanup, corner-preserving Douglas-Peucker,
  chord-length cubic fitting, bounded Newton refinement, recursive error split,
  caps, metrics, and deterministic hashes. `fit_annotation_ink_pdf` preserves
  raw points by policy, stores fitted curves, and writes a cubic appearance.

Focused engine proof currently covers fourteen Prompt 20 tests: Arabic RTL shaping
and save/reopen extraction, Identity-V vertical save/reopen extraction,
same-width token replacement, vector range editing, deterministic/error-bounded
ink fitting, closed strokes, malformed/non-finite denial, missing CJK glyph
reporting, fitted annotation appearance readback, and explicit shared-Form
edit-all/clone-one preservation, bounded z-order, group/ungroup, and incremental
patch-session undo/redo with branch redo clearing.

## Public surfaces

Rust exposes the canonical operations. CLI modes/commands are
`edit-text --mode rtl-reflow`, `vertical-reflow`, `same-width-patch`,
`vector-list`, `vector-edit`, `vector-delete`, `vector-duplicate`, `ink-fit`,
and `prompt20-report`. The shared feature report includes
`prompt20_vertical_rtl_patch_vector_ink_editing`. Python, C ABI, WASM, .NET,
and Java expose the versioned Prompt 20 report, vector inventory, and owned
text/vector/Ink mutations through established handle and owned-buffer
lifetimes; Java Maven and Gradle share that artifact.

## Exact bounded boundaries

The following are not misreported as complete: multi-token paragraph
selection, arbitrary bundled CJK coverage, nested Form clone-edit-one-instance,
pattern/shading program editing, cross-stream/Form grouping, and
clipping/marked-content/OCG z-order.
These appear as exact limits in
the feature matrix and release verdict. External reference tools are counted
only when actually executed; simple availability is not a pass.
