# Offline predefined CMap implementation

Status: source implementation, uncompiled and unqualified. This continues the
full editor/rendering roadmap; it is not a universal-editor completion claim.
The dirty `main` worktree remains based on
`27e62db3a1b84804339e65b6025273fd003b3736`. No commit or push was made.

## Implemented in source

- Bundled 202 Encoding resources from Adobe's `cmap-resources` at
  `f5cf3bca7fdfeaceb77aa82847e974f2306c20b4`, including the repository's legacy
  and deprecated resources, and five `pdf2unicode` resources from
  `mapping-resources-pdf` at `2dd5e53fb74a01718b9dfd448a0d1cce6fff2aa5`.
  Original bytes are retained losslessly as gzip/hex with copyright headers,
  both complete licences, a per-resource manifest and root NOTICE entries.
- Added lazy offline loading with declared-size/SHA-256 checks, canonical
  declarative parsing, named inheritance, a cycle/depth guard and cooperative
  cancellation. No PDF operation downloads a CMap. Generated metadata preserves
  mixed code lengths rather than declaring all UTF-16 resources two-byte.
- Named Encoding resolution now supplies actual code-to-CID mappings before
  width/W2 lookup and CID-to-GID mapping. Unknown names fail explicitly instead
  of becoming an implicit identity encoding. Names are case-sensitive.
- Structured CIDSystemInfo parsing supports literal dictionaries and the
  conventional `dict dup begin ... end` form. Nested metadata cannot replace
  immediate Registry/Ordering/Supplement fields. Stream-dictionary metadata
  composes with inheritance and is checked against explicit body declarations.
  Named and embedded maps are
  checked against the CIDFont's declared character collection; Identity maps
  remain collection-neutral. Supplement differences do not redefine old CIDs.
- Without explicit ToUnicode, the resolver composes source-code mappings with
  the declared collection's CID-to-Unicode data. Known UTF-8/UTF-16/UTF-32/UCS-2
  sources preserve their encoded Unicode scalars, including supplementary
  characters. Identity-encoded Japanese CIDs are no longer interpreted merely
  as numerically equal Unicode scalars when their collection is declared.
  Explicit ToUnicode remains authoritative.
- Synthesized Unicode maps feed the same reverse encoder and byte-provenance
  APIs used by source edits. Missing collection mappings do not silently invent
  Unicode. Ambiguous reverse encodings remain disclosed, not guessed away.
- Parsed Encoding programs and synthesized Unicode maps are immutable/shared.
  Each cache has a 16-entry/32-MiB estimated-retained-storage eviction policy;
  live resolver references can outlive cache eviction. This is not a process RSS
  ceiling or a measured performance result. Cancellation errors are not cached.
- Removed substring-based CMap name/writing-mode discovery. Metadata reporting
  now distinguishes a bundled resource from demonstrated output compatibility.
  Also corrected a bounds check that could index past a truncated W2 triple.

Files: `fonts/predefined_cmap.rs`, `predefined_resources.rs`,
`predefined_unicode.rs`, generated `predefined_resource_data.rs`,
`fonts/cmaps/`, the shared CMap modules, resolver and font diagnostics.
The existing `src/**/*` Cargo package rule includes the data and licences.

## Source acquisition and maintenance

Source acquisition compared every imported resource's size and Git blob SHA-1
against the pinned Git tree and recorded its original SHA-256. There are 207
mapping assets; their compressed bytes total 3,018,251 bytes. These are source
inventory/provenance facts, not evidence that the Rust parser accepted them.

`scripts/fetch-adobe-cmap-assets.ps1` performs explicit pinned source-data reads
and returns compressed data/metadata without local writes or code execution.
`scripts/generate-cmap-resource-index.mjs` emits the Rust index or an
`apply_patch` update from the manifest. Neither executes the PDF engine. The
generator was added but not run in this change; the equivalent index was
generated during source vendoring and formatted with rustfmt.

See `crates/engine/src/fonts/cmaps/README.md` and `manifest.json` for provenance
and redistribution files. The resource licences do not supply font programs or
grant rights to unavailable glyph outlines.

## Verification boundary

Twenty-one new regression functions were added, **not executed**:

- 19 in `fonts/predefined_cmap_tests.rs`: complete-resource hash/parser coverage,
  inventory/case rules, Shift-JIS CIDs, supplementary Unicode encodings,
  vertical inheritance, Identity collection extraction, explicit ToUnicode,
  collection/stream-dictionary mismatch, unknown names, embedded/named inheritance, shared cache
  cancellation, nested metadata, unavailable Unicode, W2 bounds and a reopened
  PDF's CID-to-GID/width/extraction route.
- 2 in `advanced_predefined_cmap_tests.rs`: repeated variable-code deletion and
  source-font reverse encoding through same-width save/reopen.

The glyph-decoder fixture intentionally maps a Japanese CID to an existing test
font GID. It verifies the planned identity path, not Japanese outline fidelity.
Existing resolver writing-mode and UTF-16-length assertions were updated.

Rustfmt formatting/parser checks and Git whitespace checks are the only local
code checks authorized. No compiler, build, typecheck, tests, PDF workload,
pixel rendering, benchmark, bindings, browser QA or deployment ran. Added source
assertions and bundled data do not establish passing runtime behavior.

## Remaining work

- Execute the complete-resource parser regression, not only a few sampled names,
  then qualify glyph selection, text extraction and repeated edits on real CJK
  PDFs with native and fallback fonts and independent renderers.
- The five bundled Unicode collections cover CNS1, GB1, Japan1, Korea1 and KR.
  This does not create missing Unicode semantics for arbitrary/private, Manga1
  identity-coded or deprecated Japan2 collections; supply authoritative
  ToUnicode where no mapping exists. Unicode-encoded resources can still retain
  their own known scalar semantics.
- Arbitrary executable/rearranged-font PostScript CMaps and best-match recovery
  for malformed byte sequences remain separate work. Resource names outside
  the pinned inventory are not automatically trusted or fetched.
- CFF2/collections/non-default transforms, broader typography/layout/tagged
  ownership, other roadmap implementation and all qualification gates remain
  open. No Adobe-superiority claim is made.

## Primary sources

Adobe's [CMap resources](https://github.com/adobe-type-tools/cmap-resources/tree/f5cf3bca7fdfeaceb77aa82847e974f2306c20b4)
provide character-code-to-CID data. Its separate
[PDF mapping resources](https://github.com/adobe-type-tools/mapping-resources-pdf/tree/2dd5e53fb74a01718b9dfd448a0d1cce6fff2aa5)
provide CID-to-Unicode data. Their distinct purposes guide the composed resolver;
neither repository demonstrates correctness of this SDK's implementation.
