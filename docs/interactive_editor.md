# Interactive editor (operation lists)

`crates/wellfriendpdf-editor` is the editing model behind Wellfriend's
in-browser PDF editor. An edit is an operation in an `OperationList`
(`ops.rs`); the browser applies each operation with the WebAssembly build
(`crates/wellfriendpdf-editor-wasm`) to preview it, and the server replays the
saved list with the native build (`wellfriendpdf pdf-editor-apply`). Both run
`OperationList::validate` first and apply operations with the same code, so the
preview is the saved result; applying a list twice yields identical bytes.

## Operations

| Operation | Implementation |
| --- | --- |
| `replaceText` | `text_reflow::apply_reflow_region` (the style-preserving reflow transaction `TextEditSession` uses). The source font program is kept when it can draw the text; otherwise the edit is retried with the operation's `fallbackFont` as `approved_substitute`. An attempt is accepted only when the page reads back as the requested text and the edited lines gain no overlapping glyphs. |
| `transformImage`, `deleteImage`, `replaceImage`, `editVector` | `wellfriendpdf-edit` object edits (`transform_image`, `delete_image`, `replace_image`, `edit_vector_path`), run on bytes through `scratch.rs` (a private temp directory natively, `wellfriendpdf_edit::vfs` in-memory files on wasm32). |
| `addText`, `addImage`, `addShape`, `addLine`, `addWhiteout` | Authored with `PdfBuilder` on a page the size of the target MediaBox, then merged into the page content with renamed resources (`overlay::merge`), so added text is ordinary page text. |
| `addLink` | A `/Link` annotation (`overlay::add_link`). |
| `redact` | `PdfEditor::redact` (full rewrite). The save is refused if any glyph remains under the area. |

Fonts come only from the caller's library (`FontSource`: `MemoryFonts` in the
browser, `DirectoryFonts` on the server); host-installed fonts are disabled
before every apply (`apply::host_independent`).

## Page model for the UI

- `PdfEditorSession` (wasm): `pageSizes`, `pageInfo` (user space to 72-dpi
  pixels), `renderPage` (RGBA), `textLayout` (glyph quads, lines, paragraphs,
  page fonts with their library matches), `pageObjects` (image placements,
  vector paths), `apply`, `setBytes`, `bytes`.
- `fonts::page_fonts` names each font resource's family, subset/embedding and
  the metric-compatible or same-family library face.

## Builds

```sh
cargo test --release -p wellfriendpdf-editor   # WELLFRIENDPDF_REAL_PDF_DIR=<dir> adds real-PDF round trips
cargo build --release -p wellfriendpdf-cli      # pdf-editor-apply
cargo build --release -p wellfriendpdf-editor-wasm --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir pkg target/wasm32-unknown-unknown/release/wellfriendpdf_editor_wasm.wasm
wasm-opt -Oz pkg/wellfriendpdf_editor_wasm_bg.wasm -o pkg/wellfriendpdf_editor_wasm_bg.wasm
```

`pdf-editor-apply` reads `{input_path, output_path, font_dir,
operations_path | operations, allow_signature_invalidation}` on stdin and prints
the `pdf-edit` envelope; failure codes are `PDF_EDITOR_*`.

## Known limits

- Text in a few authored layouts (per-glyph `TJ` runs inside `/ActualText`
  spans) cannot reuse the source program; such edits use the library font and
  say so.
- Vector paths are selectable only inside an isolated `q … Q` group.
- Image replacement takes JPEG; placements must be isolated `q cm Do Q`.
- Object inspection (`pageObjects`) inspects the whole document each time.
