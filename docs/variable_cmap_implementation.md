# Length-aware PDF character-code implementation

Status: implemented in source; not compiled or runtime-qualified. This extends
the CFF/native-CID increment, not completion of the universal-editor roadmap.
Changes remain local and uncommitted on the existing dirty `main` worktree.

## Changes

- `fonts/character_code.rs` retains both the numeric value and encoded length
  of one-through-four-byte codes. A bounded DFA decodes declared byte-wise
  code-space ranges without enumerating an entire 32-bit domain. Prefix-ambiguous
  spaces, truncated codes and invalid offsets return errors without advancing
  the caller's offset or padding source bytes.
- `fonts/cmap_program.rs` and `cmap_stream.rs` provide the shared declarative
  grammar and stream/inheritance loader for Encoding and ToUnicode. Canonical
  tokens distinguish commands from comments, strings and literal metadata.
  CID ranges, notdef mappings, UTF-16BE strings, range arrays, ordered overrides,
  stream inheritance and explicit WMode/name consistency use the same parser.
  ToUnicode ranges increment only the last destination byte; overflow is an
  error. Destinations retain multi-scalar strings and supplementary characters.
- `fonts/cmap.rs` no longer uses a second substring scanner or truncates source
  keys to 16 bits. Malformed input exposes an error and no partial map.
  Compatibility repair is limited to omitted record counts and code spaces;
  legacy scalar projection omits multi-scalar values and conflicting
  length-specific numeric keys.
- `fonts/resolver_codes.rs` exposes a fused, cancellation-aware iterator with
  exact byte provenance. Source mutation checks Encoding/ToUnicode consistency.
  Bounded dynamic programming reverse-encodes through actual source mappings,
  including multi-scalar ligatures, and reports alternative complete encodings.
  It does not enumerate a four-byte domain or invent missing ToUnicode entries.
- The resolver, renderer's glyph decoder, text collector, inline displacement,
  multi-run selection, same-width patch analysis and redaction byte slicing use
  these original code boundaries. Character codes, native CIDs, GIDs and Unicode
  strings remain distinct. Font widths/W2 follow the mapped CID; word spacing
  follows the single-byte encoded space, not the extracted Unicode character.
- Preserved-style run measurement now includes every emitted character-spacing
  advance, including the final code in each concatenated run. Same-width patches
  cannot report equal movement merely because glyph widths match when the
  number of encoded word-spacing occurrences changes.

The parser bounds decoded CMap bytes, tokens, inheritance depth, expanded
assignments, Unicode text storage and DFA states/work. Reverse lookup has text
and work limits. Cooperative cancellation covers parsing, expansion, mapping
validation, reverse indexing/matching and output-path reconstruction.

## API note

`FontResolver::code_size()` and `ToUnicodeCMap::code_size()` return zero for
mixed-length code spaces. Callers must use `codes()`/`next_code()` and the
length-aware `CharacterCode` methods rather than chunking byte strings.
Legacy `u16`-code helpers remain for fixed-length callers; they cannot identify
which of two differently encoded codes with the same numeric value was intended.
The engine consumers above were migrated together.

## Unexecuted regressions

Twenty-five new regression functions were added:

- `fonts/variable_cmap_tests.rs`: 17 cases for byte provenance, native CIDs,
  horizontal/vertical spacing, reverse ligature mapping, decoder/extractor
  agreement, four-byte spaces, invalid sequences, cancellation, structured
  grammar, UTF-16 ranges, ordered overrides, inheritance and numeric collisions.
- `advanced_variable_cmap_tests.rs`: 6 cases for indivisible source mappings,
  displacement, preserved-style spacing, same-width spacing eligibility,
  incremental save/reopen and repeated multi-run deletion.
- `editing_variable_cmap_tests.rs`: 2 cases retaining untouched variable-length
  bytes and removing a whole multi-scalar glyph during geometric redaction.

Existing fixed-width and truncated-code assertions were updated for the common
decoder. These are source assertions, not observed successes or redaction proof.

Rustfmt formatting/parser checks completed. Git whitespace checks are recorded
in the implementation handoff. No compiler, build, typecheck, tests, generated
PDFs, rendering, benchmarks, bindings, browser QA, deployment, commit or push
were run. Syntax-format acceptance does not establish Rust type correctness.

## Still open

- This increment initially left predefined Adobe/East Asian resources open.
  The subsequent [offline predefined CMap implementation](predefined_cmap_implementation.md)
  bundles pinned resources and replaces that metadata/legacy fallback. Its
  complete-resource parser and interoperability regressions remain unexecuted.
- Arbitrary executable/rearranged-font PostScript CMaps are not interpreted.
  Invalid byte sequences fail explicitly; best-partial-match recovery for
  malformed rendered text is not implemented by this strict decoder.
- Missing logical Unicode semantics cannot be recovered from CID numbers with
  certainty. Existing heuristic extraction fallback is not authoritative
  source reconstruction. A selection still cannot split one source mapping
  without a reconstruction policy.
- CFF2, font collections, non-default CFF transforms, broader font programs,
  complex layout/tagged ownership and the other roadmap items remain open.
- Exact-revision compilation, binding parity, save/reopen/edit behavior,
  independent glyph/pixel checks, large-CMap resource use and cancellation
  latency still require the later authorized VPS qualification phase.

## Primary references

Adobe's [PDF 1.6 reference, sections 5.6.4 and 5.9.2](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf)
describes length-sensitive code spaces, Encoding versus ToUnicode, destination
strings and last-byte range increments. Adobe's
[CMap/CIDFont specification](https://www.adobe.com/content/dam/acom/en/devnet/font/pdfs/5014.CIDFont_Spec.pdf)
defines declarative mappings, inheritance and later CID-range overrides.
These references guide the source implementation; they do not validate its output.
