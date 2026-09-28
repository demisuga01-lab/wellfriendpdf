# Pinned Adobe CMap source data

This directory contains 202 Encoding CMaps from Adobe's `cmap-resources` and
five CID-to-Unicode maps from `mapping-resources-pdf`. `manifest.json` records
the exact repository commits, original paths, Git blob IDs, SHA-256 hashes,
decoded lengths, character collections and resource metadata.

Each `.gz.hex` file is the original upstream bytes, losslessly gzip-compressed
and represented as whitespace-separated lowercase hexadecimal. Copyright
headers inside the resources are retained. The uncompressed texts of both
BSD-3-Clause licences are adjacent and must accompany redistribution notices.
These are mapping data, not font programs or recovered/licensed glyph outlines.

The engine includes these assets from `src/**/*`, so native and WASM use the
same pinned offline inventory. It does not fetch maps during a PDF operation.
The runtime checks the decoded length and SHA-256 before parsing a resource.
The initial compressed bytes total 3,018,251 bytes; text hex representation is
larger. This is an asset-size count, not a binary-size or memory benchmark.

Maintenance helpers, neither of which executes the engine:

- `scripts/fetch-adobe-cmap-assets.ps1` fetches explicit paths at a required
  commit, returning JSON with compressed hex and original hashes. It writes no
  local files and executes no downloaded code. Compare blob IDs with the pinned
  Git tree before installing updates with `apply_patch`.
- `scripts/generate-cmap-resource-index.mjs` emits the Rust index from the
  manifest; `--apply-patch` emits a patch for the existing index. Apply the
  reviewed patch and format that source with rustfmt. Inherited code lengths
  in the manifest must reflect the base map, not merely an absent local range.

The full-resource parser/hash regression has been added but **not run**. Source
vendoring and hash provenance do not establish successful engine parsing,
extraction, glyph rendering, editing, or interoperability.
