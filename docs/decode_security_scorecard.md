# Decode Security Scorecard

Java Packaging closes the Binding Parity leftover list by adding public diagnostics,
configurable limits, a cache utility, memory-token scheduling, scanner
abstraction, fuzz/corpus harnesses, and a documented sandbox decision. This
scorecard is intentionally blunt: DONE means implemented and tested; DEFERRED
WITH REASON means intentionally bounded and documented.

## Filters and Codecs

| Item | Status | Implementation | Caps | Diagnostics | Fuzz/corpus | Isolation | Remaining risk |
| --- | --- | --- | --- | --- | --- | --- | --- |
| FlateDecode | DONE | `filters.rs` zlib/raw deflate | decoded bytes, chain depth, ratio profile field | `DecodeDiagnostic` via report | `filters`, seeds, property tests | in-process Rust | no known Java Packaging leftover |
| LZWDecode | DONE | `filters.rs` | decoded bytes, EarlyChange validation | `DecodeDiagnostic` via report | `filters`, unit caps | in-process Rust | bounded materialization for some paths |
| RunLengthDecode | DONE | `filters.rs` | decoded bytes | `DecodeDiagnostic` via report | `filters`, seeds, unit caps | in-process Rust | no known Java Packaging leftover |
| ASCIIHexDecode | DONE | `filters.rs` | decoded bytes | malformed/cap diagnostics | `filters`, seeds, unit caps | in-process Rust | no known Java Packaging leftover |
| ASCII85Decode | DONE | `filters.rs` | decoded bytes | malformed/cap diagnostics | `filters`, seeds, unit caps | in-process Rust | no known Java Packaging leftover |
| PNG/TIFF predictors | DONE | `filters.rs` | row bytes, columns, colors | predictor diagnostics | `predictor`, unit/property tests | in-process Rust | no known Java Packaging leftover |
| DCT/JPEG | DONE WITH BOUNDED LIMIT | `jpeg-decoder` adapter | width, height, pixels, decoded bytes | image budget diagnostics | `image_decoders`, corpus runner | in-process Rust | no OS process sandbox |
| JPX/JPEG2000 | DONE WITH BOUNDED LIMIT | `hayro-jpeg2000` adapter | width, height, components, pixels | image budget diagnostics | `image_decoders`, corpus runner | in-process Rust | no OS process sandbox |
| CCITTFaxDecode | DONE WITH BOUNDED LIMIT | `hayro-ccitt` adapter | rows, columns, pixels | image budget diagnostics | `image_decoders`, corpus runner | in-process Rust | no OS process sandbox |
| JBIG2Decode | DONE WITH BOUNDED LIMIT | `hayro-jbig2` adapter | page/region pixels, decoded bytes | image budget diagnostics | `image_decoders`, corpus runner | in-process Rust | no OS process sandbox; no JBIG2 writing |
| Unknown filters | DONE | central filter resolver | no decoded allocation | `decode_unsupported_filter` | unit/fuzz | n/a | no known Java Packaging leftover |

## Cross-Cutting Infrastructure

| Item | Status | Evidence | Remaining risk |
| --- | --- | --- | --- |
| Structured decode diagnostics | DONE | `DecodeReport`, `DecodeDiagnostic`, parser-report `--include-decode`, unit tests | Python/C ABI direct exposure deferred to binding API pass |
| Configurable limits | DONE | `DecodeLimits`, profiles, CLI profile/overrides, unit tests | CLI exposes high-value overrides, not every internal field |
| Codec sandboxing | DEFERRED WITH REASON | `docs/codec_sandboxing.md` Outcome C decision, Rust dependency audit | OS isolation revisited if native codec dependency or fuzz evidence appears |
| Decoded-stream cache | DONE | `DecodeCache` LRU with byte budget, max entry, metrics, tests | not wired as a global image-pixel cache by design |
| Work-stealing scheduler | DONE | `DecodeMemoryBudget`, `ScheduledDecodeJob`, Rayon ordered execution, tests | broad render/extraction adoption deferred to subsystem scheduler integration |
| SIMD scanner | DEFERRED WITH REASON | scalar scanner abstraction, accelerated fallback status, equality tests, bench script | no unsafe SIMD in `wellfriendpdf-engine` because unsafe code is forbidden |
| Fuzz campaign harness | DONE | `scripts/run_decode_fuzz_campaign.py`, seed folders, README | long overnight results are local/operator-run, not committed |
| Hostile codec corpus runner | DONE | `scripts/codec_corpus_runner.py` | raw codec samples are cataloged; full raw decode CLI is not exposed |
| Parser-report integration | DONE | opt-in `decode` section and top-level mapped diagnostics | full-document stream decode is opt-in to avoid surprising cost |
| 2 GB discipline | DONE WITH BOUNDED LIMIT | `DecodeLimits`, scheduler memory tokens, cache budget | aggregate memory depends on callers using scheduler for broad parallel jobs |

## Java Packaging Closure

Nothing remains as a hidden Binding Parity leftover. The exact bounded future items are:

- direct Python/C ABI decode diagnostic exposure during a binding-surface pass;
- OS-level codec isolation if Wellfriend adopts native codec libraries or fuzz/corpus
  evidence justifies process containment;
- safe SIMD implementation only if it can respect the engine's no-unsafe policy
  or live behind an explicitly reviewed feature boundary;
- wider scheduler adoption by render/extraction/OCR page pipelines in their own
  subsystem prompts.
