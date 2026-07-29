# advanced editing known limits

- advanced editing closeout supports contiguous whole decoded string-token selections in one
  page content stream across `Tj`, `TJ`, quote, and double-quote operators.
  Partial-token, cross-stream, cross-page, malformed-CMap, and arbitrary Type3
  selections fail closed. Per-segment source-style output is not yet a
  generated-run serializer. Visual selections must be resolved to one
  unambiguous logical range by the caller before mutation.
- Bundled DejaVu supports Arabic and Hebrew but not arbitrary CJK. Vertical
  Japanese needs a caller-supplied font with the requested glyphs.
- Same-width patching rejects Type3, shaping, bidi/vertical reorder, changed
  glyph structure, clipping render modes, ambiguous mappings, encryption, and
  unsupported filter chains.
- Page-owned and reachable Form vector ranges are editable. Shared Forms require
  an explicit reject/edit-all/clone-one policy; clone-one recursively copies a
  selected losslessly-decodable invocation path to the page. Missing resource
  dictionaries and malformed/cyclic graphs fail closed.
  Arbitrary pattern program editing, shading-mesh editing, semantic shape
  inference remain exact unsupported. Group/ungroup is bounded to contiguous
  page-owned ranges; z-order is bounded to page-owned objects outside clipping,
  marked-content, and OCG contexts.
- Indirect annotation appearance vectors are inventoried and editable. A
  shared appearance stream is cloned for the selected annotation's `/N`, `/R`,
  `/D`, or selected state when clone-one is explicitly requested; sibling AP
  entries and `/AS` are retained. Vectors inside nested appearance Forms are
  cloned through the selected annotation's AP owner path.
- Ink fitting approximates geometry only and never reconstructs pen dynamics.
- Incremental prefix and signature dictionary preservation do not establish
  cryptographic validity, certification acceptance, trust, or warning-free UI.
- External reference binaries that are unavailable are reported unavailable,
  not passed.
