# Parser Foundation Audit

This audit maps the parser state used for Prompt 01. It is based on the current
repository code, not roadmap assumptions. The working tree already contained
uncommitted OCR, Office, bindings, utility, and renderer work before this parser
slice; those files are treated as pre-existing and are not part of this audit.

## Current Architecture Map

| Subsystem | Status | Primary files/APIs | Notes |
| --- | --- | --- | --- |
| Lexical tokenization | Present | `crates/engine/src/parser.rs`, `PdfParser::{skip_ws_and_comments, parse_name, parse_literal_string, parse_hex_string, parse_number}` | Handles PDF whitespace including NUL, delimiters, comments, escaped literal strings, hex odd nibble, and name `#XX` escapes. Parser recursion is capped by `MAX_PARSE_DEPTH = 256`. |
| COS primitive parsing | Present | `crates/engine/src/parser.rs`, `crates/engine/src/object.rs` | Parses null, booleans, integers, reals, names, strings, arrays, dictionaries, streams, indirect references, and indirect objects. Exponent-form numbers are not accepted by the current parser. |
| Stream boundary recognition | Present, repair-oriented | `PdfParser::parse_indirect_object`, stream length fallback scan | Uses declared `/Length` when possible and scans for `endstream` as a fallback. Missing/mismatched streams produce parse errors or repaired raw stream extraction depending on context. |
| Classic xref tables | Present | `reader.rs::read_classic_xref` | Supports subsections, free/in-use entries, generation handling, trailer parse. Later revisions override earlier through `or_insert` while walking newest-to-oldest. |
| Xref streams | Present, hardened in this prompt | `reader.rs::{read_xref_stream, parse_xref_stream_entries}` | Supports `/W`, `/Index`, entry types 0/1/2, zero-width type fields. This prompt adds duplicate `/Index` object-number rejection. |
| Object streams | Present, hardened in this prompt | `reader.rs::{parse_object_stream, parse_object_stream_data}` | Lazy-decoded through bounded `BoundedObjectStreamCache`. This prompt rejects duplicate object IDs inside object streams and keeps `/N` preallocation bounded. |
| Trailer parsing | Present | `read_classic_xref`, `read_xref_stream`, `find_last_trailer_dictionary` | Repair mode can recover the last trailer dictionary or synthesize `/Root` and `/Size` from object scan candidates. |
| `startxref` lookup | Present | `reader.rs::find_startxref`, `parser_report::find_startxref_offset` | Normal mode uses last marker. Repair mode falls back to object scan when missing or unusable. Near-offset classic xref repair exists within a bounded window. |
| Incremental `/Prev` traversal | Present, reportable | `read_xref_chain`, `read_xref_chain_from_source`, `parser_report::inspect_revision_history` | Walks newest section first, follows `/Prev`, detects loops and applies `MAX_XREF_CHAIN_DEPTH = 256`. Parser-report exposes section offsets, trailer keys, duplicate objects, and newest winning revision per object number. |
| Encryption detection/hooks | Present | `reader.rs::setup_encryption`, `EncryptionContext`, `encrypt_dictionary` | Detects `/Encrypt`, verifies Standard Security Handler passwords, decrypts strings/streams on object access. Encrypted content without keys returns structured `OxideError`. |
| Linearization detection | Parser-level validation present | `info.rs` and `parser_report::detect_linearization` | Detects the early linearization dictionary and validates `/Linearized`, `/L`, `/H`, `/O`, `/E`, `/N`, and `/T` shape/ranges. Full hint-table entry validation and HTTP range planning remain out of scope. |
| Page tree/object resolver | Present | `PdfReader::{get_object, resolve, get_and_resolve}`, `document.rs`, `engine.rs` | Object lookup is on-demand. Reference resolution is depth-capped at 64 and detects cycles. |
| Lazy source/range reader | Present | `PdfSource`, `SeekableFileSource`, `PdfRangeReader`, `content_stream_range` | File-backed open reads prefix/tail/xref windows, not the whole file. Small-file fallback to full read remains bounded by `STREAMING_FULL_READ_FALLBACK_LIMIT`. |
| Stream filter decode | Present | `filters.rs`, `decode_stream_from_dict`, `decode_stream` | Decode caps are enforced in filter/image paths. Parser report does not force stream decoding. |
| Parser errors | Present, now supplemented | `error.rs::OxideError`, `parser_report.rs::ParserDiagnostic` | Existing errors are stable enough for callers; this prompt adds structured diagnostics with severity/category/code/offset/recovery fields. |
| Repair/recovery | Present, hardened in this prompt | `rebuild_xref_from_object_scan`, `repair_uncompressed_xref_offsets`, `scan_indirect_object_headers` | Tiers implemented today: trusted xref, near xref repair, object scan/trailer synthesis. This prompt records repair actions and ignores object-looking bytes inside normal stream spans. |
| Validation | Real generated Arlington tables, shallow predicate subset | `arlington.rs`, `parser_report.rs`, `scripts/generate_arlington_tables.py`, `crates/engine/src/generated/arlington_tables.rs` | Generated from upstream Arlington commit `5a8639424495c27a30df30bb9491a346f9316014`: 613 TSV files, 3983 key rules. Required keys, basic types, name enums, direct/indirect metadata, deprecated-key metadata, and unsupported predicate reporting are implemented. Full predicate evaluation is not complete. |
| Fuzzing | Present, extended | `fuzz/`, `crates/engine/src/fuzz.rs` | Existing out-of-workspace cargo-fuzz targets cover document parser, filters, writer, rendering-adjacent paths. This prompt adds COS object, parser report, xref stream, and object stream targets. |
| Corpus/differential | Present scripts, extended | `scripts/poppler_compare.py`, `scripts/differential_fuzz.py`, `scripts/parser_corpus_runner.py`, `scripts/parser_differential.py` | SafeDocs-compatible bounded JSONL corpus runner and parser-specific differential harness over `oxide parser-report`. External tools remain opt-in unless required by flag. |

## Fragile Assumptions Found

- Repair object scanning previously treated object-looking byte sequences inside
  stream data as possible indirect-object headers. This prompt now skips normal
  `stream`...`endstream` spans in the repair scanner.
- Xref stream `/Index` ranges could overlap and silently produce duplicate object
  numbers in one section. This prompt rejects duplicate object numbers.
- Object streams could declare duplicate object IDs, with last-writer wins in the
  temporary map. This prompt rejects duplicates.
- `/Prev` traversal had loop detection but no explicit depth ceiling. This prompt
  adds a 256-section cap.
- The public parser surface had no structured diagnostic report. This prompt adds
  `ParserReport`, `ParserDiagnostic`, and `oxide parser-report`.

## Current Deferred Items

- Full Arlington predicate expression evaluation beyond the safe represented subset.
- Public SDK object-history APIs beyond parser-report's revision summary.
- PDF rewrite/save of forensic page-tree recovery; current recovery is audit-only.
- HTTP range source client. The parser remains range-ready but does not fetch URLs.
- Perfect hashing/name interning/arena allocation optimization campaign.
- Full linearization hint-table entry validation and first-page byte-range plan.
