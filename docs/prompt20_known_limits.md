# Prompt 20 known limits

- Serialized RTL/vertical replacement currently selects one decoded PDF string
  token. Multi-token paragraph selection is rejected; overlay is not automatic.
- Bundled DejaVu supports Arabic and Hebrew but not arbitrary CJK. Vertical
  Japanese needs a caller-supplied font with the requested glyphs.
- Same-width patching rejects Type3, shaping, bidi/vertical reorder, changed
  glyph structure, clipping render modes, ambiguous mappings, encryption, and
  unsupported filter chains.
- Page-owned and reachable Form vector ranges are editable. Shared Forms require
  an explicit reject/edit-all/clone-one policy; clone-one is bounded to a
  top-level page invocation, while nested clone-one remains exact unsupported.
  Arbitrary pattern program editing, shading-mesh editing, semantic shape
  inference remain exact unsupported. Group/ungroup is bounded to contiguous
  page-owned ranges; z-order is bounded to page-owned objects outside clipping,
  marked-content, and OCG contexts.
- Indirect annotation appearance vectors are inventoried and editable. An
  appearance stream referenced by multiple annotations is diagnosed and
  rejected until explicitly cloned; Oxide never silently edits all uses.
- Ink fitting approximates geometry only and never reconstructs pen dynamics.
- Incremental prefix and signature dictionary preservation do not establish
  cryptographic validity, certification acceptance, trust, or warning-free UI.
- External reference binaries that are unavailable are reported unavailable,
  not passed.
