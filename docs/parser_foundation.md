# Parser Foundation

Wellfriend's parser foundation covers the early PDF pipeline: bytes, lexical/COS
object parsing, xref and object resolution, lazy source access, incremental
revision inspection, linearization metadata validation, repair scanning, and
structured parser diagnostics. This is not a claim of complete PDF-spec
perfection. It is a bounded, diagnostic, repair-aware foundation for the rest
of the SDK.

## Binding SurfaceA and 01B Scope

Binding SurfaceA added structured parser reporting, strict/repair/audit modes,
parser diagnostics, source metrics, xref/object-stream hardening, `/Prev` depth
caps, a SafeDocs-compatible corpus runner, a parser-report CLI command, parser
fuzz targets, and Arlington-shaped validation scaffolding.

Binding SurfaceB replaces the Arlington scaffold with generated tables from real
upstream Arlington TSVs, expands parser-report with revision history,
linearization validation, damaged-object repair summaries, differential testing
scripts, bounded corpus execution, and structure-aware fuzz seed generation.

## Modes

- **Strict**: requires a PDF header, readable `startxref`, usable xref section
  or xref stream, and trailer dictionary. No object-scan repair is run.
- **Repair**: uses bounded fallback paths when strict xref/trailer parsing
  fails: near-offset xref repair, indirect-object scan, trailer synthesis, and
  xref offset repair.
- **Audit**: runs strict first, then repair, and reports both outcomes.
- **Lazy/range**: file-backed open reads prefix/tail/xref windows and loads
  objects on demand. Object streams are cached behind a bounded cache.
- **Encrypted**: structural metadata can be inspected where possible; encrypted
  content requires a valid password before encrypted strings/streams are
  decoded.
- **Capped**: parser recursion, reference resolution, object scan size, object
  stream cache size, `/Prev` chain length, decoded streams, and large-file
  fallback paths are bounded.

## Diagnostic Schema

Rust exposes `ParserReport` and `ParserDiagnostic`; CLI exposes:

```text
wellfriendpdf parser-report input.pdf --mode audit --json
```

Each diagnostic includes severity, category, stable code, message, optional
source, dictionary path, key, expected type, actual type, byte offset/range,
object id, page number, recovery action, incomplete-output flag, and hostile
input flag.

Normal open behavior is unchanged. Parser reports are an explicit audit
surface and may parse/inspect more than normal lazy open.

## Arlington Validation

The generated Arlington table currently comes from:

- upstream repository: `https://github.com/pdf-association/arlington-pdf-model`
- pinned commit: `5a8639424495c27a30df30bb9491a346f9316014`
- local generation source: `target/arlington-pdf-model-5a863942/tsv/latest`
- generated Rust table: `crates/engine/src/generated/arlington_tables.rs`

Coverage from the current generation:

| Metric | Count |
| --- | ---: |
| TSV files | 613 |
| object/dictionary models | 613 |
| key rules | 3983 |
| required-key rules | 924 |
| type rules | 3983 |
| version metadata rules | 3983 |
| indirect-reference policy rules | 441 |
| link metadata rules | 1698 |
| unsupported predicates reported | 3429 |
| generator parse warnings | 0 |

The runtime validator consumes generated Rust tables, not TSV files. It checks
required keys, basic object types, allowed name values, shallow direct/indirect
policies where the rule is representable, deprecated-key metadata, and reports
unsupported Arlington predicates explicitly. It does not yet evaluate the full
Arlington predicate expression language.

Regenerate with:

```text
python scripts/fetch_arlington_model.py --out target/arlington-pdf-model-5a863942
python scripts/generate_arlington_tables.py --arlington-root target/arlington-pdf-model-5a863942 --commit 5a8639424495c27a30df30bb9491a346f9316014 --out crates/engine/src/generated/arlington_tables.rs --stats-json target/arlington/arlington_stats.json --complete
```

Use `git diff -- crates/engine/src/generated/arlington_tables.rs` after
regeneration to detect drift.

## Revision History

`parser-report` follows the latest `startxref` and `/Prev` chain with loop and
depth-cap protection. It reports each xref/trailer section's offset, section
type, trailer keys, `/Prev`, `/Size`, `/Root`, `/Info`, `/Encrypt`, `/ID`,
`/XRefStm`, object numbers visible in that section, duplicate objects, and the
newest revision that wins for each object number.

This is parser-level provenance, not signature validation. It intentionally
preserves the facts a later digital-signature phase needs: where revisions
start, which objects changed, and which section wins.

## Linearization Validation

Linearization validation is parser-level. It detects a linearization dictionary
near the beginning of the file and validates `/Linearized`, `/L`, `/H`, `/O`,
`/E`, `/N`, and `/T` shape/ranges. The report distinguishes:

- `linearized_detected`
- `linearization_valid`
- `first_page_fast_open_candidate`
- declared vs actual file length
- declared page count where present
- declared main xref offset and whether it points to an xref candidate

This is not a complete HTTP range planner and does not validate every hint-table
entry.

## Repair and Forensic Reporting

Repair mode remains conservative. The damaged-object summary reports:

- objects expected from xref
- objects recovered from xref
- objects carved by the scanner
- missing objects
- duplicate objects
- truncated objects
- stream length mismatches
- missing `endstream` cases
- trailer reconstruction observed through diagnostics
- recovered page dictionaries for audit-only page-tree reconstruction
- parse failure notes and skipped byte ranges
- a coarse confidence label

Forensic page-tree reconstruction is report-only. If `/Root` or `/Pages` is not
usable but page dictionaries are carved, parser-report marks an audit-only
recovered page list. It does not rewrite a PDF or silently replace the document
model.

## Differential Testing

Use:

```text
python scripts/parser_differential.py tests --wellfriendpdf target/debug/wellfriendpdf.exe --limit 25 --json-out target/parser-diff/results.jsonl --markdown-out target/parser-diff/results.md
```

The harness compares shallow, stable parser facts against available external
tools such as qpdf, Poppler `pdfinfo`, and MuPDF `mutool`. Missing tools are
reported as skipped unless `--require-tools` is used.

## SafeDocs-Compatible Corpus Gate

External corpora are not vendored. Point the runner at a SafeDocs checkout or
any local PDF directory:

```text
python scripts/parser_corpus_runner.py --input PATH_TO_CORPUS --wellfriendpdf-bin target/debug/wellfriendpdf.exe --output target/parser-corpus/audit.jsonl --summary docs/parser_corpus_results.md --mode audit --limit 200 --max-total-bytes 1073741824 --timeout 30
```

The runner preserves directory-derived categories, supports bounded file/byte
limits, records timeouts fail-soft, and can resume from existing JSONL output.

## Fuzzing

Current parser fuzz targets include COS object parsing, parser-report open,
xref stream parsing, and object stream parsing. Generate compact seeds with:

```text
python scripts/generate_parser_fuzz_seeds.py --out-dir fuzz/seeds/parser --mutations
```

Run short smoke checks with cargo-fuzz when the nightly/cargo-fuzz toolchain is
available. Normal CI should compile fuzz targets, not run long fuzz campaigns.

## Memory Model

Normal file-backed open remains lazy/range-based. Parser-report is an audit
operation and may inspect xref chains, root/trailer dictionaries, and bounded
object scans, but it still avoids unbounded multi-GB full-document materializing.
The current generated Arlington table is compile-time Rust data and does not
load TSV files at runtime.

## Known Limits

- Arlington predicate expressions are counted and reported but only a safe
  subset is enforced.
- Arlington validation is shallow for many object types; it is not a full
  semantic validator like veraPDF.
- Linearization validation checks dictionary shape and offsets but not every
  hint-table entry.
- Forensic page recovery is audit/report-only and does not create a repaired PDF.
- Differential comparison is shallow and compares stable facts, not full COS
  semantic equivalence.
- Accepting a damaged PDF in repair mode does not imply that the file is valid
  or safe. Strict correctness, repair usefulness, and unsafe permissiveness are
  separate concepts.
