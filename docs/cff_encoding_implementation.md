# OpenType/CFF1 embedding and CID encoding - source implementation

This increment extends the existing authoring, generated editing and renderer
paths. It does not finish the universal-editor roadmap or establish runtime
correctness. Work remains uncommitted over
`27e62db3a1b84804339e65b6025273fd003b3736` on `main`.

## Implemented changes

- `fonts/pdf_embedding.rs` classifies editable standalone glyf and OpenType/CFF1
  programs, checks editable outline permission and no-subsetting restrictions,
  and resolves CFF charset identity through ttf-parser. A bounded metadata reader
  retrieves the CFF ROS and PostScript name; it does not execute CharStrings.
- CFF character codes, native CIDs and glyph indexes are separate. Generated
  two-byte character codes retain their own ToUnicode values, including different
  logical strings that use the same physical glyph. An embedded Encoding CMap
  selects native CIDs. Widths and vertical metrics use those CIDs, not assumed
  glyph indexes. CFF registry/ordering/supplement and the embedded PostScript
  font name are retained.
- `authoring.rs` and `authoring_fallback.rs` accept supported CFF1 programs through
  the existing registration, shaping, fallback-stack and serialization APIs.
  CFF uses CIDFontType0 and FontFile3/OpenType, without CIDToGIDMap. TrueType keeps
  CIDFontType2 and the existing GID-preserving subset path; no-subsetting fonts
  retain their exact full program. CFF1 currently embeds whole, not subsetted.
- `advanced_editing.rs` uses the same CFF identity and encoding plan. All
  generated fonts allocate independent character codes; CFF no longer forces
  code = GID. Existing positioned glyph emission, source transactions and
  ToUnicode/ActualText routes remain authoritative.
- `fonts/cid_encoding.rs` parses declarative embedded Encoding CMaps using the
  canonical tokenizer. It supports uniform one-/two-byte codes, cidchar,
  cidrange, notdef entries, writing mode and bounded stream/Identity inheritance.
  Code-space bounds are checked per byte, including inherited bounds. Metadata
  strings and nested literal/dictionary scopes are not treated as mapping
  instructions. Conflicting maps, unsupported executable instructions, invalid
  bounds and excessive token/expansion/nesting work fail explicitly. Long loops
  poll the existing cancellation authority.
- `FontResolver` caches Encoding and sfnt-CFF native-CID-to-GID maps. Unicode and
  word-spacing eligibility use the original character code; W/W2 use the mapped
  native CID. Shared visual decoding selects the actual sfnt glyph index. Bare
  CFF keeps its existing rasterizer mapping and is not mapped twice. Destructive
  source rewriting and inline position discovery reject invalid cached encoding
  state instead of using an assumed identity map. Best-effort extraction remains
  a separate API, not a guarantee of safe editing.
- `writer_feature_version.rs` enforces the OpenType minimum declared version.
  Whole-document output raises the header to at least PDF 1.6; incremental output
  composes a catalog Version update with any already-pending catalog changes,
  without changing the original byte prefix. Newer versions are not downgraded.
  The same final header calculation prevents setter ordering from dropping the
  writer's xref/encryption minimum. Raw signing/incremental bodies remain a
  deliberately lower-level caller-owned API.

## Source regressions and checks

Thirty-two new regression functions were added, **not executed**:

- `fonts/pdf_embedding_tests.rs`: 7 metadata, permissions, code identity,
  deterministic mapping, malformed charset and standard-string cases.
- `fonts/cid.rs`: 2 sfnt-versus-bare-CFF mapping and read-versus-edit permission
  cases, in addition to the existing CID regressions.
- `fonts/cid_encoding_tests.rs`: 10 mapping, inheritance, byte-wise code-space,
  nested metadata, writing-mode, malformed-input and width/Unicode cases.
- `authoring_cff_tests.rs`: 4 fresh-document save/reopen/visual-decode,
  mixed-CFF/TrueType fallback, denied-registration and no-subsetting cases.
- `advanced_cff_tests.rs`: 3 generated horizontal/vertical incremental-save,
  native CID, distinct logical-code and pre-serialization refusal cases.
- `writer_feature_version_tests.rs`: 6 header minimum, catalog composition,
  newer-version preservation, no-op, invalid-catalog and output-derived edit
  contract inventory cases.

Fixtures construct small name-keyed and CID-keyed CFF1 programs in Rust, with
CID-keyed GID order deliberately different from CIDs (`0, 42, 7, 1000`). Their
construction and the assertions are source only, not generated-PDF evidence.
The CFF standard SID name table is attributed to ttf-parser and includes its MIT
notice in `fonts/TTF_PARSER_CFF_LICENSE.txt`.

Rustfmt formatting/parser checks and Git whitespace checks completed. No Cargo,
compiler, typecheck, build, test, PDF workload, rendering, benchmark, bindings,
browser QA, deployment, commit or push was performed. A successful syntax-format
pass is not evidence that these modules type-check or that the regressions pass.

## Remaining boundaries

- CFF2 instantiation, collections, CFF subsetting/subroutine reconstruction,
  arbitrary color/variable-font semantics and unavailable/licensed glyphs are
  not solved by this change.
- The subsequent [length-aware CMap increment](variable_cmap_implementation.md)
  replaces this increment's initial one-/two-byte decoder with shared
  one-through-four-byte and mixed-length decoding. The later
  [predefined resource implementation](predefined_cmap_implementation.md) adds
  pinned East Asian maps. Arbitrary executable PostScript CMaps and executable
  qualification remain open.
- Arbitrary malformed CFF recovery, non-default CFF matrices and broad exotic
  font interoperability still require implementation review and corpus evidence.
- Individual fallback assignment choices and every historical font/writer path
  are not qualified by these source changes. Independent output inspection,
  raster/vector comparisons, save/reopen/edit cycles and binding execution remain
  pending. The regression visual-decode assertions do not compare rendered pixels.
- The declared PDF version floor is not PDF/A, PDF/UA, PDF/X or signature-policy
  conformance validation. The existing output-derived edit-contract inventory
  includes writer-added catalog changes; optional caller assertions that demand
  exact catalog preservation must still pass against the actual output.
- Full editor/rendering roadmap completion and an Acrobat comparison remain open.

## Primary references consulted

- Adobe's [PDF 1.6 reference](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf),
  glyph selection in CIDFonts and embedded OpenType programs.
- Adobe's [CMap/CIDFont specification](https://www.adobe.com/content/dam/acom/en/devnet/font/pdfs/5014.CIDFont_Spec.pdf),
  declarative code spaces, CID mapping and inheritance.
- Microsoft's [OpenType CFF specification](https://learn.microsoft.com/en-us/typography/opentype/otspec183/cff),
  CFF versus sfnt glyph order and table requirements.

These references guide implementation; they do not constitute validation of
this repository's output.
