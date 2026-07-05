# Oxide WASM Browser Demo

Build the raw WASM and browser glue:

```sh
cargo build -p oxide-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web \
  --out-dir crates/oxide-wasm/examples/browser/pkg \
  target/wasm32-unknown-unknown/release/oxide_wasm.wasm
```

Alternatively, `wasm-pack` can produce the same web package:

```sh
wasm-pack build crates/oxide-wasm --target web --out-dir examples/browser/pkg
```

Then serve `crates/oxide-wasm/examples/browser` with any static server and open
`index.html`. The demo reads a selected PDF as a `Uint8Array`, opens it with
`OxidePdf`, parses it to Markdown, splits it into RAG chunks, extracts
key-value fields, extracts page text, and renders page 1 to PNG bytes. Nothing
is uploaded.

## Scope

The Prompt 02 WASM wrapper exposes open-from-bytes, optional password open,
close and use-after-close checks, page count, parser methods (`parseMarkdown`,
`parseJson`), RAG `chunk`, key-value `extractFieldsJson`, plain/structured text
extraction, info JSON, render-page-to-PNG, facade-backed report JSON methods
for inspection/security/parser/color/validation/forms/annotations/pages/fonts/
signatures/semantics, output methods for sanitize, canonicalize, and
redact-terms workflows, and `OxidePdf.codecIsolationReportJson(filter, bytes,
policy)` for Prompt 03 codec policy diagnostics.

Use `policy="in_process"` for browser-safe local decode reports. Subprocess
policies return a structured unavailable/fail-closed report because
wasm/browser targets cannot spawn OS codec workers.

OCR is intentionally excluded in the browser: the Tesseract backend is an
external process, so the WASM surface is digital-born only. The WASM wrapper
also does not expose server endpoints, filesystem batch tools, async jobs,
C/Python bindings, native binary loading, or multi-threaded rayon execution.

Verified for Prompt 02: `cargo build -p oxide-wasm --target
wasm32-unknown-unknown` compiles the report and output bindings. Regenerate the
checked-in example `pkg/` glue with the commands above before using newly added
methods from `oxide_wasm.js`.
