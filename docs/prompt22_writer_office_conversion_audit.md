# Prompt 22 Writer and Office Conversion Audit

## Starting checkpoint

- Expected Prompt 21 final commit: `7ac69de3b0df433a08d5bbef858a4451bf6da590`
- Actual HEAD verified before Prompt 22 edits: `7ac69de3b0df433a08d5bbef858a4451bf6da590`
- Worktree verified clean before Prompt 22 edits: `True`
- Prompt 21 commit message: `Complete combined prompt 21 raster vector font persistent object streams`

The current generator also records the post-edit status in `prompt22-starting-state.json` so the audit preserves both the pre-edit checkpoint and the live repository state when artifacts were regenerated.

## Implementation summary

- Zopfli-class compression is implemented in `crates/engine/src/prompt22.rs` using the pure-Rust `zopfli` crate as a direct dependency. It is optional and does not change the default writer fast path.
- Compression modes are `fast`, `balanced`, `best`, `zopfli`, and `zopfli_bounded`. Zopfli is bounded by input bytes, iteration count, block cap, and stream-level cancellation checkpoints.
- Recompression decodes each eligible stream, encodes with the selected mode, decodes the candidate bytes, and commits only when decoded bytes match and the savings threshold is met.
- Global dedup is a deterministic full-rewrite planning pass over eligible streams. It buckets by SHA-256 but only deduplicates after canonical stream bytes compare equal.
- Encrypted PDF optimization is refused rather than writing decrypted output or changing encryption semantics. Full rewrite is reported as signature-impacting.
- Office package security is enforced in `crates/engine/src/office.rs` before DOCX/PPTX/XLSX conversion. ZIP path traversal, bombs, unsupported methods, macros, OLE, ActiveX, embedded executables, XML entities, and external relationships are blocked or reported.
- DOCX/PPTX/XLSX-to-PDF uses Oxide-native parsing and authoring paths. Microsoft Office, LibreOffice, Ghostscript, browser rendering, and cloud conversion are not production dependencies.

## Feature matrix

| Feature | Category | Status | Surface | Exact limit |
| --- | --- | --- | --- | --- |
| `p22-zopfli-backend` | compression | implemented_with_limits | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | stream-boundary cancellation only |
| `p22-deflate-modes` | compression | implemented | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | default writer fast path unchanged |
| `p22-decoded-equality` | compression | implemented_with_limits | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | filter chains and unsafe codecs are reported, not recompressed |
| `p22-global-dedup` | writer | implemented_with_limits | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | full rewrite only; encrypted inputs refused |
| `p22-office-security` | office-security | implemented_with_limits | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | conservative scanner; no external fetch |
| `p22-docx-to-pdf` | office-conversion | implemented_with_limits | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | page-faithful, not Word-identical |
| `p22-pptx-to-pdf` | office-conversion | implemented_with_limits | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | unsafe media/action content blocked or reported |
| `p22-xlsx-to-pdf` | office-conversion | implemented_with_limits | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | formulas are not executed |
| `p22-public-bindings` | bindings | implemented_with_limits | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | Java/.NET runtime tests require native library path |
| `p22-quality-benchmark` | benchmark | implemented_with_limits | rust, cli, python, c_abi, wasm, dotnet, java_maven, java_gradle | reference tools optional only |

## Security posture

- Office packages are hostile ZIP/XML inputs. Prompt 22 inspection never fetches relationships and never executes formula, macro, DDE, OLE, ActiveX, JavaScript, media, or remote content.
- XLSX formulas are not executed; conversion uses cached/stored cell values and benchmark reporting records unsupported or missing cached values.
- ZIP and XML limits are serialized in package-security artifacts and exposed through public reports.

## Benchmark posture

- The bundled benchmark artifacts classify reference tools as optional. Unavailable Office, LibreOffice, qpdf, Poppler, PDFium, and MuPDF binaries are reported as unavailable, not passed.
- Required production proofs are Oxide-native: generated PDFs reopen through Oxide, Prompt 22 tests prove decoded equality and package blocking, and binding surfaces route through the shared SDK facade.

## Release verdict

Prompt 22 is implemented with exact limits. No Prompt 22-scope feature row is `blocked`; unsupported cases use the requested exact or security-policy status classes.
