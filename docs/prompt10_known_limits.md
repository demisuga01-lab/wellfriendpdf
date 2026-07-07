# Prompt 10 Known Limits

Prompt 10B narrows the remaining limits to explicit security or exotic fidelity
cases.

## Remaining Limits

- COLR/CPAL v1 complex paint graphs remain unsupported unless they fit the
  supported solid-layer path.
- SVG-in-OpenType is blocked by security policy. Scripts, event handlers,
  external references, network access, foreignObject, animation, and remote
  resources are not executed or fetched.
- Advanced CID-keyed CFF clipping remains unsupported only when a real
  charstring-derived outline is not exposed to the renderer. Bounding-box
  clipping is not used as a substitute.
- Optional native hinting remains a future enhancement, not a default runtime
  dependency.
- Unsupported or malformed color bitmap payloads fail closed with diagnostics.

## Not Limits

- COLR/CPAL v0 solid layered glyph rendering is implemented.
- sbix PNG color glyph rendering is implemented.
- The shared bounded embedded-bitmap glyph path supports safe bitmap payloads
  exposed by the font parser.
- Korean and Hebrew rendered-page fixture gaps are closed.
- Prompt 10B multi-reference audit has zero Oxide outlier failures and zero
  unclassified failures.
