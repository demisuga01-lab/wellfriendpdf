# Public static TrueType instancing

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This increment
connects the preceding internal font stages to one public asset-preparation
transaction and the existing editing, authoring and retained-session paths.
It is source implementation, not a compiled or runtime-qualified release.
The complete editor/rendering roadmap remains active.

## Public transaction

`fonts::font_instance::prepare_font_instance(source, request)` accepts an exact
source SHA, explicit face index, design coordinates and explicit instance names.
Omitted axes use their declared defaults; unknown axes, non-finite coordinates
and values outside the declared range are errors. Coordinates are not silently
clamped. The same selected face and normalized coordinates feed every stage.

For supported variable TrueType faces, publication combines:

- Native point-preserving `glyf`/`loca`, updated `head`/`maxp` and exact GID order.
- Per-glyph horizontal/vertical metrics, registered MVAR fields and varied CVT.
- Frozen GSUB/GPOS features and positioning, GDEF, BASE and JSTF ownership.
- The selected-coordinate hint fallback and cross-table contour-point checks.
- Explicit identity names, selected STAT records and registered-axis style fields.

Unresolved hint-review conditions, private MVAR owners, unchecked point references,
remaining layout variation stores and unknown table owners prevent publication.
Redundant declared/outline-derived metric differences require an explicit request
flag; the returned report retains each difference and its chosen precedence.
This is not a policy to discard all unknown tables or automatically strip hints.

Editable embedding bits are checked before preparation and after serialization.
No-subsetting bits remain intact. This checks machine-readable permissions, not
the full licence agreement. Removing a declared standalone or collection font
signature requires an explicit decision. Signature verification is not performed.

The canonical sfnt writer serializes once. Before returning bytes, the transaction
reopens the result, checks static status/GID count/UPEM and all staged metric rows,
and compares the exact table-owner set, lengths and SHA-256 receipts for every
generated and preserved table. Only `head.checkSumAdjustment`, owned by the final
writer, is excluded from the table hash. Original input bytes are immutable.
No partial font is registered if preparation or these postconditions fail.

Native input/output limits remain 256 MiB. Session transport bounds font input
and output to 4 MiB. Stages retain their separate aggregate-work budgets; output
size is not a promise about peak RSS. Name capture is capped at 4096 records,
256 language tags and 8 MiB of materialized string bytes, so small aliased input
cannot expand without a bound. Copying, hashing and stage loops poll cancellation;
the canonical sfnt serializer itself still has pre/post checks rather than
instruction-level cancellation. Memory, latency and cancellation timing are not
benchmarked.

## Names, style and static kerning

The caller supplies typographic family/subfamily, legacy family, PostScript name
and one regular/bold/italic/bold-italic style link. Identity translations are
replaced by explicit Unicode and Windows English records; this is not automatic
multilingual name generation. Copyright, licence, vendor and other non-identity
strings retain their source bytes. Format-1 language tags and sorted record
identities survive serialization. The unique identity includes the source face,
coordinates and chosen labels. The naming formats follow the
[OpenType name specification](https://learn.microsoft.com/en-us/typography/opentype/spec/name).

STAT formats 1 through 4 are filtered against selected design coordinates;
range records and older-sibling records retain their specified roles. Axis order
and opaque record extensions remain intact. When STAT references an identity
name about to change, all of that name's language/platform variants are copied
to a free private ID and the reference is relocated first. Renaming ID 2 therefore
does not silently change an old STAT fallback or value label. No STAT label is
invented for an unnamed coordinate. See the
[STAT specification](https://learn.microsoft.com/en-us/typography/opentype/spec/stat).

Registered `wght`, `wdth` and `slnt` coordinates update the corresponding weight,
width and italic-angle fields. Explicit style links update legacy selection and
head flags; applicable oblique bits are handled separately. Rebuilt average
advance and the prior MVAR fields are composed without overwriting embedding
rights. These fields do not infer the typographic meaning of arbitrary custom
axes. See [OS/2](https://learn.microsoft.com/en-us/typography/opentype/spec/os2)
and [post](https://learn.microsoft.com/en-us/typography/opentype/spec/post).

Non-variable version-0 `kern` tables are preserved, with format-0 pair identities
and format-2 class/row domains checked against the unchanged GIDs. Class entries
are offsets, not raw class numbers; default row/column values and array ownership
are checked. Extended/variation kerning tables require another owner and are not
treated as opaque static data. References:
[OpenType kern](https://learn.microsoft.com/en-us/typography/opentype/spec/kern),
[TrueType class-kerning offsets](https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6kern.html).

## Editing, authoring and session integration

The returned bytes are the exact standalone static asset, not an axis annotation
attached to the old variable font. Existing text coverage, shaping, substitution
approval and saved-story font persistence apply to these bytes.

- `ApprovedFontAsset::from_font_instance` returns the editable asset and report.
- `RegisteredFontProvider::register_font_instance_bytes` registers only after
  preparation succeeds.
- `PdfBuilder::register_font_instance_bytes` returns the registered `FontFace`
  and the same instance report through canonical authoring registration.
- Retained-session JSON command `prepare_font_instance` returns `{asset, report}`
  without changing PDF bytes, preview approval or undo history. Existing generic
  C, WASM, Python, Java and .NET session transports carry this command; no separate
  native symbol was added or executed in this increment.

The subsequent CFF2 increment advances session status to
`font_instance_protocol_version: 4` after checked default CFF2 publication (this TrueType
command shape is unchanged). Example command
shape (the source bytes and SHA come from the caller's inspected font):

```json
{
  "op": "prepare_font_instance",
  "lookup_name": "Body Selected",
  "bytes": [0],
  "request": {
    "selection": {
      "source_sha256": "<inspect_font source_sha256>",
      "face_index": 0,
      "allow_signature_removal": false
    },
    "coordinates": {"wght": 550},
    "naming": {
      "family": "Body",
      "subfamily": "Selected",
      "legacy_family": "Body Selected",
      "postscript_name": "Body-Selected",
      "style_link": "regular"
    },
    "accept_redundant_metric_differences": false
  }
}
```

`[0]` is a placeholder, not a usable font. Coordinates must use axes actually
advertised by the selected face. Add `asset` to an editable draft, choose its
lookup name, then preview and approve the normal PDF checkpoint.

The worker client provides `prepareFontInstance(name, bytes, request)` with
queue-entry copies of bytes and nested choices. Only this new command requires
the new capability gate; ordinary extraction remains available on its established
protocol. The local font picker exposes explicit axis fields, names, style links
and decisions. It does not install or fetch fonts, silently replace draft assets,
or approve a PDF edit. It remains disabled for collaborative structural drafts.

## Regression source and evidence

Thirty-one added regression functions are unexecuted: 15 public-font cases,
11 metadata cases, two retained-session cases, two client cases and one capability
case. Coverage includes exact outlines/metrics, default and non-default axes,
selected collection faces, names/STAT, alias-expansion bounds, kerning domains,
checksums, corrupted-table receipts, permissions/signatures, deterministic output,
budgets/cancellation, failed-registration atomicity, authoring extraction and
story prepare/checkpoint/reopen/re-edit. Browser source checks request snapshots
and native-version rejection; it does not establish browser behavior.

Only source review, rustfmt formatting/parser checks and whitespace checks were
performed. No Cargo, compiler, build, typecheck, tests, font/PDF workload, renderer,
benchmark, browser QA, commit, push or deployment was run.

## Remaining work

This closes the missing public publication path for the declared TrueType subset,
not for every font. The subsequent `cff2_static_instance_implementation.md`
adds bounded CFF2 publication; `cff2_contour_normalization_implementation.md`
adds explicit numerical overlap normalization without exact hint preservation.
Newer variation formats,
color/AAT/private table owners, general early/dynamic hint semantics, phantom-point
attachments and large layout-offset packing remain. Identity localization and
custom-axis semantics are explicit caller responsibilities. The component checks
are not a full font sanitizer or a hint-interpreter proof.

Real-font coverage, final pixels, independent rasterizers, bindings, repeated
editing and all broader roadmap qualifications remain pending. Neither universal
PDF editing nor superiority to Acrobat is established by this source increment.
