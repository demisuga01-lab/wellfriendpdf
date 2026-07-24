# Parser Foundation Commit Blocker

Prompt 01B parser work is validated locally but cannot be committed as a clean
parser-only commit from the current worktree without mixing unrelated phase
work.

## Blocking Conditions

- `crates/cli/src/main.rs` contains parser-report changes in the same dirty file
  as pre-existing Phase 3 utility commands, Office conversion commands, and OCR
  command wiring. Staging the whole file would commit unrelated features.
- `crates/engine/src/lib.rs` contains parser exports in the same dirty file as
  pre-existing OCR, Office, and utility module exports. A parser-only staged
  version from HEAD would not compile unless those unrelated untracked modules
  were also staged.
- `crates/wellfriendpdf-capi/*`, `crates/wellfriendpdf-py/*`, `crates/wellfriendpdf-wasm/src/lib.rs`,
  `crates/server/*`, and `bindings/*` are dirty/untracked from earlier binding,
  OCR, Office, and server work. They were built/tested as part of validation but
  are not parser-scope commit content.
- The untracked `bindings/` tree includes generated `bin/` and `obj/` outputs
  from .NET test runs. These should not be swept into a parser commit.
- The working tree includes unrelated untracked docs and scripts:
  `docs/ocr_backends.md`, `docs/office_conversions.md`,
  `docs/rendering_fidelity_baseline.md`, `public-benchmark/`, and
  `renderer-benchmark/scripts/rendering_fidelity_gallery.py`.

## Parser-Scope Files Added or Touched by Prompt 01A/01B

- `crates/engine/src/arlington.rs`
- `crates/engine/src/generated/arlington_tables.rs`
- `crates/engine/src/parser_report.rs`
- parser-report hunks in `crates/cli/src/main.rs`
- parser exports in `crates/engine/src/lib.rs`
- parser/fuzz hunks in `crates/engine/src/fuzz.rs` and `fuzz/Cargo.toml`
- `scripts/generate_arlington_tables.py`
- `scripts/parser_corpus_runner.py`
- `scripts/parser_differential.py`
- `scripts/generate_parser_fuzz_seeds.py`
- `docs/parser_foundation.md`
- `docs/parser_foundation_audit.md`
- `docs/arlington_validation.md`
- `docs/parser_repair.md`
- `docs/parser_differential_testing.md`
- `docs/parser_memory_architecture.md`
- `docs/parser_corpus_results.md`
- `docs/parser_foundation_commit_blocker.md`

## Safe Path to Commit Later

1. Commit or stash unrelated OCR, Office, Phase 3 utility, renderer, public
   benchmark, and binding work first.
2. Remove generated `bindings/**/bin` and `bindings/**/obj` outputs from the
   working tree or add appropriate ignore rules in a non-parser commit.
3. Re-run:

   ```text
   cargo fmt --check
   git diff --check
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   cargo build -p wellfriendpdf-capi
   cargo build -p wellfriendpdf-wasm --target wasm32-unknown-unknown
   python -m maturin build --manifest-path crates/wellfriendpdf-py/Cargo.toml
   ```

4. Stage parser files and parser hunks only, then commit them as one parser
   foundation commit.

## Hygiene Re-check - 2026-07-02

Commands run from `E:\wellpdfsdk`:

- `git status --short`
- `git diff --stat`
- `git diff --name-status`
- `git ls-files --others --exclude-standard`

Current branch/head at inspection: `main` at `2e92c27`.

### Commit Decision

No parser commit was created in this hygiene pass. A parser-only checkpoint is
still unsafe from the current worktree because required parser hunks live in
shared files that also contain unrelated OCR, Office, Phase 3 utility, and
binding changes. Staging those files whole would mix roadmap phases; hunk-only
staging would require constructing and validating a synthetic index state that
does not match the working tree. That is too risky for a checkpoint whose only
purpose is repository hygiene.

### Changed File Classification

#### 1. Parser Prompt 01A/01B scope

These are parser-scope and should be included in a future parser-only commit,
subject to separating the shared-file hunks noted below:

- `crates/engine/src/arlington.rs` - Arlington rule consumer and parser
  validation diagnostics.
- `crates/engine/src/generated/arlington_tables.rs` - generated Arlington table
  data from upstream commit `5a8639424495c27a30df30bb9491a346f9316014`; this
  generated file should be committed with parser work.
- `crates/engine/src/parser_report.rs` - strict/repair/audit parser report,
  revision history, linearization validation, repair summary, and diagnostics.
- `crates/engine/src/reader.rs` - parser diagnostics, strict open, xref/repair
  hardening, source metrics, and reader-side repair reporting.
- `crates/engine/src/fuzz.rs` - parser fuzz entry points only.
- `fuzz/Cargo.toml` - parser fuzz target bin declarations only.
- `fuzz/fuzz_targets/cos_object.rs` - parser fuzz target.
- `fuzz/fuzz_targets/parser_report.rs` - parser-report fuzz target.
- `fuzz/fuzz_targets/xref_stream.rs` - xref-stream fuzz target.
- `fuzz/fuzz_targets/object_stream.rs` - object-stream fuzz target.
- `scripts/fetch_arlington_model.py` - pinned Arlington fetch helper.
- `scripts/generate_arlington_tables.py` - generated Arlington table builder.
- `scripts/generate_parser_fuzz_seeds.py` - parser fuzz seed generator.
- `scripts/parser_corpus_runner.py` - bounded parser corpus/SafeDocs-compatible
  runner.
- `scripts/parser_differential.py` - parser differential harness.
- `docs/arlington_validation.md` - parser/Arlington docs.
- `docs/parser_foundation.md` - parser foundation docs.
- `docs/parser_foundation_audit.md` - parser architecture audit.
- `docs/parser_repair.md` - repair-mode docs.
- `docs/parser_differential_testing.md` - parser differential docs.
- `docs/parser_memory_architecture.md` - parser memory/lazy-loading docs.
- `docs/parser_corpus_results.md` - bounded parser corpus result docs.
- `docs/parser_foundation_commit_blocker.md` - this hygiene/blocker record.
- Parser hunks inside `crates/engine/src/lib.rs` - only `arlington` and
  `parser_report` module/export lines are parser-scope. The same file also has
  unrelated Office and Phase 3 utility exports.
- Parser-report hunks inside `crates/cli/src/main.rs` - only the
  `parser-report` command, args, runner, formatting, and help text are
  parser-scope. The same file also contains unrelated Phase 3 utility, Office,
  and OCR command changes.
- `crates/engine/tests/fixtures/arlington_mock.tsv` - old parser scaffold test
  fixture. It should be committed only if still referenced by parser tests, or
  removed in a parser cleanup commit; it should not be mixed into unrelated
  work.

#### 2. Existing unrelated OCR work

- `Cargo.lock` - adds `wellfriendpdf-ocr-tesseract` to dependency resolution.
- `crates/engine/src/extract/mod.rs` - adds OCR policy pass-through.
- `crates/engine/src/ocr/mod.rs` - OCR policy and backend concurrency changes.
- `crates/engine/src/ocr/dispatch.rs` - OCR dispatch/containment module.
- `crates/engine/src/parse.rs` - OCR policy/timeout routing in document parse.
- `crates/engine/examples/ocr_http_backends.rs` - OCR HTTP reference backend.
- `crates/engine/tests/ocr_containment.rs` - OCR containment tests.
- `crates/engine/tests/ocr_http_references.rs` - OCR HTTP reference tests.
- `crates/wellfriendpdf-ocr-tesseract/src/lib.rs` - Tesseract backend work.
- `crates/wellfriendpdf-ocr-tesseract/tests/smoke.rs` - Tesseract smoke tests.
- `crates/wellfriendpdf-capi/src/ocr_backend.rs` - C ABI OCR backend seam.
- `crates/wellfriendpdf-py/src/ocr_backend.rs` - Python OCR backend seam.
- `crates/wellfriendpdf-py/examples/local_ai_ocr_backend.py` - local-AI OCR example.
- `crates/server/Cargo.toml` - optional Tesseract OCR server feature.
- `crates/server/src/lib.rs` - server OCR module export.
- `crates/server/src/main.rs` - server OCR env initialization.
- `crates/server/src/ocr.rs` - server OCR hook.
- `crates/server/src/routes/parse_ops.rs` - server OCR application to parse
  routes.
- `docs/ocr_backends.md` - OCR backend guide.
- `docs/self_hosting.md` - OCR seam documentation hunk.
- OCR hunks inside `crates/cli/src/main.rs` - optional-value `--ocr` policy
  changes.
- OCR hunks inside `crates/wellfriendpdf-capi/*` and `crates/wellfriendpdf-py/*` - binding
  exposure for OCR.

#### 3. Existing unrelated Office conversion work

- `crates/engine/Cargo.toml` - moves/adds `zip` for runtime Office/OOXML work.
- `crates/engine/src/office.rs` - Office conversion implementation.
- `docs/document_hierarchy.md` - Phase 4 hierarchy decision.
- `docs/office_conversions.md` - Office conversion docs.
- `docs/office_to_pdf_architecture.md` - Office-to-PDF architecture docs.
- Office hunks inside `crates/cli/src/main.rs` - `pdf-to-xlsx`,
  `pdf-to-pptx`, `pdf-to-docx`, and Office-to-PDF commands.
- Office tests inside `crates/cli/tests/tool_surface.rs`.
- Office hunks inside `crates/engine/src/lib.rs`, `crates/wellfriendpdf-capi/*`,
  `crates/wellfriendpdf-py/*`, and `crates/wellfriendpdf-wasm/src/lib.rs`.

#### 4. Existing unrelated bindings work

- `bindings/dotnet/WellfriendPdf.Tests/WellfriendPdf.Tests.csproj`
- `bindings/dotnet/WellfriendPdf.Tests/WellfriendPdfSmokeTests.cs`
- `bindings/dotnet/WellfriendPdf/NativeMethods.cs`
- `bindings/dotnet/WellfriendPdf/OfficeConverters.cs`
- `bindings/dotnet/WellfriendPdf/WellfriendPdf.csproj`
- `bindings/dotnet/WellfriendPdf/WellfriendDocument.cs`
- `bindings/dotnet/WellfriendPdf/WellfriendPdfException.cs`
- `bindings/dotnet/WellfriendPdf/README.md`
- `bindings/java/README.md`
- `bindings/java/src/main/java/io/wellfriendpdf/WellfriendPdf.java`
- `bindings/java/src/test/java/io/wellfriendpdf/WellfriendPdfSmokeTest.java`
- `docs/dotnet_binding.md`
- `docs/java_binding.md`
- `docs/bindings.md` - binding-surface updates.
- `docs/python_binding.md` - Python binding expansion.
- `crates/wellfriendpdf-capi/cbindgen.toml`
- `crates/wellfriendpdf-capi/include/wellfriendpdf.h`
- `crates/wellfriendpdf-capi/src/lib.rs`
- `crates/wellfriendpdf-py/README.md`
- `crates/wellfriendpdf-py/src/lib.rs`
- `crates/wellfriendpdf-wasm/src/lib.rs`

The C ABI/Python/WASM files are mixed with OCR, Office, and Phase 3 utility
surface work. They are not parser-scope commit content.

#### 5. Existing unrelated renderer fidelity work

- `crates/engine/src/render/page_renderer.rs` - SMask/glyph hinting renderer
  fidelity changes and renderer tests.
- `docs/rendering_fidelity_baseline.md` - renderer fidelity baseline docs.
- `renderer-benchmark/scripts/rendering_fidelity_gallery.py` - renderer
  benchmark/gallery utility.

#### 6. Existing unrelated Phase 3 utility work

- `crates/engine/src/utilities.rs` - PDF-to-image, image-to-PDF, watermark,
  page-numbering, organize, and related utility surface.
- `docs/phase3_api_audit.md` - Phase 3 audit docs.
- `docs/phase3_summary.md` - Phase 3 summary docs.
- `docs/api_overview.md` - utility/Office/bindings overview changes.
- `docs/cli.md` - utility/Office CLI docs.
- Phase 3 utility hunks inside `crates/cli/src/main.rs`.
- Phase 3 utility tests inside `crates/cli/tests/tool_surface.rs`.
- Phase 3 utility hunks inside `crates/engine/src/lib.rs`,
  `crates/wellfriendpdf-capi/*`, `crates/wellfriendpdf-py/*`, and `crates/wellfriendpdf-wasm/src/lib.rs`.

#### 7. Public benchmark/report work

- `docs/benchmark_public.md`
- `extraction-benchmark/scripts/capability_probe.py`
- `public-benchmark/README.md`
- `public-benchmark/capability_matrix.json`
- `public-benchmark/manifests/public_corpus_manifest.json`
- `public-benchmark/requirements.txt`
- `public-benchmark/scripts/build_public_corpus.py`
- `public-benchmark/scripts/run_text_benchmark.py`

#### 8. Generated files that should or should not be committed

- `crates/engine/src/generated/arlington_tables.rs` - generated but
  parser-scope; should be committed with Arlington integration.
- `bindings/**/bin/**` - 130 generated .NET build/test output files; should
  not be committed in a parser checkpoint.
- `bindings/**/obj/**` - generated .NET build intermediates; should not be
  committed in a parser checkpoint.

#### 9. Accidental or unknown changes

No accidental/unknown paths remain after classification. The open issue is not
unknown ownership; it is that several required parser hunks are embedded in
files that also contain known unrelated work.

### Exact Remaining Blockers

- `crates/cli/src/main.rs` has parser-report changes in the same file as
  unrelated Phase 3 utility commands, Office conversion commands, and OCR flag
  policy rewiring. Whole-file staging is unsafe.
- `crates/engine/src/lib.rs` has required parser module/export changes mixed
  with unrelated Office and Phase 3 utility exports. Whole-file staging is
  unsafe.
- `crates/engine/src/parse.rs` and `crates/engine/src/extract/mod.rs` are OCR
  changes, not parser foundation changes. They must not be pulled into a parser
  checkpoint just because they compile with the current dirty tree.
- `Cargo.lock` and `crates/engine/Cargo.toml` are unrelated OCR/Office
  dependency changes, not parser dependencies.
- `crates/wellfriendpdf-capi/*`, `crates/wellfriendpdf-py/*`, `crates/wellfriendpdf-wasm/src/lib.rs`,
  `crates/server/*`, and `bindings/*` remain non-parser work and generated
  output. They must stay out of Prompt 01.

### Safe Next Step

To close Prompt 01 with a real parser commit, first isolate or commit the
unrelated OCR, Office, Phase 3 utility, bindings, renderer, and benchmark work.
After that, re-stage only the parser files listed in bucket 1 plus the
parser-only hunks in `crates/cli/src/main.rs` and `crates/engine/src/lib.rs`,
run the full validation gate, and commit as:

```text
Complete Prompt 01 parser foundation
```
