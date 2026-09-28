# Explicit font collection preparation — source implementation

This increment is uncompiled and unexecuted. No builds, tests, PDF workloads,
rendering, browser QA, benchmarks or deployments were run. It does not close the
complete roadmap or establish universal editing / superiority over Acrobat.

## Implemented source path

`fonts::font_asset::inspect_font_asset` enumerates standalone sfnt files and
TTC/OTC version 1/2 collections. It reports the exact source SHA-256, explicit
face indices, names, outline format, glyph count, units, parsed variation axes,
OS/2 embedding bits, subsetting permission and signature presence.

`prepare_font_asset` requires that source hash and an explicit face index.
Standalone input at index zero remains byte-exact. A selected collection face
becomes a standalone sfnt through the existing shared sfnt writer:

- Collection table offsets are interpreted from the beginning of the file.
- Shared and interleaved table ranges are supported. Only selected-face tables
  are copied; glyph IDs, cmap, GSUB/GPOS/GDEF, metrics, names and OS/2 remain intact.
- Sorted standalone directories, four-byte alignment, per-table checksums and
  the `head.checkSumAdjustment` are regenerated. The shared writer now always
  zeros that field before calculating the directory's head checksum.
- Collection-level or selected-face DSIG removal needs explicit approval. The
  receipt reports removal and never claims signature verification. Preparation
  receipts describe preparation only, not later embedding/subsetting changes.
- TrueType/glyf and OpenType/CFF1 reuse the canonical embedding classifier. CFF1
  native charset/ROS identity is not rewritten as glyph-order identity.
- Malformed present OS/2 tables cannot become permissive "missing" tables.

Directory counts, ranges and duplicate tags are validated. Tables may not overlap
directory/header storage. The direct Rust budget is 256 MiB input/output, 256
faces, 256 tables per face, 4,096 name records and 64 parsed axes. Output expansion
is checked before allocating selected table copies. Hashing and copying poll
cancellation at 64-KiB boundaries; directory/name loops also poll. The existing
sfnt serializer/checksum routine is checked before/after, not preempted inside.
These are implementation budgets, not measured memory or timing guarantees.

## Wired consumers

| Consumer | Entry point / behavior |
|---|---|
| Native authoring | `PdfBuilder::register_font_face_bytes` returns registered face plus receipt |
| Rendering provider | `RegisteredFontProvider::register_font_face_bytes`; `ContentEngine` forwards it; existing unchecked rendering registration remains unchanged |
| Source editing and linked stories | `ApprovedFontAsset::from_font_face` returns the existing asset type plus receipt; normal planning, approval, persistence, fallback and embedding use normalized bytes |
| Shared session protocol | `inspect_font` and `prepare_font`, available through existing WASM/Python/C/Java/.NET generic command transports; status advertises `font_asset_protocol_version: 1` |
| Browser client | `inspectFont` and `prepareFont` snapshot bytes and selection before queued worker work; do not publish a PDF revision or create undo history |
| Browser editor | Local font picker requires explicit face choice, unique draft name and signature decision; adds the asset without replacing same-named assets; paragraph font selection invalidates prior preview approval |

The JSON session/browser path limits font input **and prepared output** to 4 MiB,
in addition to the existing 32-MiB total command limit. Output is budgeted before
materialization. Larger collections require the direct Rust preparation API;
this increment does not add a large binary-font transport to every binding.
Font preparation never supplies font files from the network or installs them.
Font and PDF signatures are different; this does not authorize PDF signature
invalidation. Hosts remain responsible for their font licences and asset policy.

## Usage

Rust: inspect bytes; let the user choose a catalog face; create
`FontFaceSelection { source_sha256, face_index, allow_signature_removal }`; pass
that selection and the same bytes to the authoring/provider registration method
or `ApprovedFontAsset::from_font_face` for an editing request.

Browser: inspect, display the catalog to the user, then pass their exact selection:

```js
const catalog = await client.inspectFont(fontBytes);
// chosenIndex and allowRemoval come from an explicit user decision.
const prepared = await client.prepareFont("Approved Face", fontBytes, {
  source_sha256: catalog.source_sha256,
  face_index: chosenIndex,
  allow_signature_removal: allowRemoval,
});
draft.fonts.push(prepared.asset);
```

Generic JSON hosts send `inspect_font` with `bytes`, then `prepare_font` with
`lookup_name`, the same `bytes` and `selection`. The result is `{asset, report}`.
Add the asset to the story's fonts and choose its lookup name. This changes a
draft, not the document; preview/checkpoint approval remains mandatory.
Preparing a font does not assert coverage for arbitrary replacement text: the
existing contextual coverage and shaping pipeline still evaluates it.

## Regression source and verification boundary

Added 20 Rust font regressions: TTC v1/v2 sharing, exact directory aliases,
interleaved table ranges, exact table preservation, both
signature locations, standalone signatures, source/index mismatch, malformed
directories and permissions, rights/no-subsetting, CFF1 native identity, CFF2
non-conversion, shaping preservation, head checksums, authoring/reopen, provider
atomicity, cancellation, preallocation budgets and retained variation metadata.
Two session regressions exercise asset preparation followed by story checkpoint,
reopen and re-edit, plus protocol failures without publication. Two browser-client
regressions cover queued snapshots and payload rejection. All 24 are unexecuted.
The synthetic variable-face case tests preserved metadata, not variation rendering.

Only rustfmt formatting/parser checks and Git whitespace checks were performed.
They do not establish Rust type correctness, binding execution, browser behavior,
font/PDF compatibility, pixel fidelity or production readiness.

## Still open

Follow-up: `static_truetype_instance_implementation.md` adds a separate explicit
public static TrueType transaction and browser controls. It handles selected
non-default coordinates for its declared table/hint subset; ordinary extraction
below still retains variable data, and CFF2 static publication remains open.

- Static instantiation of non-default variable-font coordinates and CFF2. Glyph
  outlines alone are insufficient: variation-dependent metrics and layout tables
  must describe the same instance. This extractor retains variable data/defaults;
  it never pretends that extraction is static instantiation.
- WOFF/WOFF2, broader font programs, arbitrary custom tables/semantics and malformed
  font repair. Discovery/preparation is not a complete font sanitizer.
- Large binary-font transports, richer per-run browser font controls, real browser
  integration qualification and actual licensed CJK/variable/collection corpora.
- The rest of `universal_editor_roadmap_tracking.md`, including independent output
  evidence. Existing source limitations are not removed by this report.

## Primary format references

Implementation follows Microsoft's [OpenType font file and collection format](https://learn.microsoft.com/en-us/typography/opentype/spec/otff)
for file-relative collection directories, table sharing, alignment and checksums.
The [head table specification](https://learn.microsoft.com/en-us/typography/opentype/spec/head)
explains why the old collection checksum adjustment cannot be reused unchanged.
The [DSIG specification](https://learn.microsoft.com/en-us/typography/opentype/spec/dsig)
describes font signatures; this implementation detects declarations but does not
verify them. Source dependencies used are the existing ttf-parser 0.21 and shared
sfnt writer; no new runtime dependency or external conversion executable was added.
