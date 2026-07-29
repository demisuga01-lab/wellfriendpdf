# Porterduff Radial Color Glyph COLRv1 Porter-Duff And Radial Closure

Porterduff Radial Color Glyph closes the final Multilingual Color Glyphs color-glyph correctness gaps left after
Colrv Gradient Composite:

- COLRv1 Porter-Duff composites: `Clear`, `Source`, `Destination`,
  `DestinationOver`, `SourceIn`, `DestinationIn`, `SourceOut`,
  `DestinationOut`, `SourceAtop`, `DestinationAtop`, and `Xor`.
- COLRv1 additive `Plus`.
- Moving-center COLRv1 radial gradients.

Porter-Duff and `Plus` source paints render into a scheduler-reserved
transparent glyph-local source surface. The source pixels are then composited
against the current glyph-local backdrop with straight-alpha Porter-Duff
equations using premultiplied intermediate math. SourceOver and PDF blend modes
continue to use the Transparency Rendering/07B blend machinery.

Moving-center radial gradients now use an analytic two-circle solve per covered
pixel:

```text
|P - C0 - t * (C1 - C0)|^2 = (r0 + t * (r1 - r0))^2
```

The renderer selects the largest finite root whose interpolated radius is
non-negative, matching the existing radial shading solver posture, then applies
the COLRv1 pad/repeat/reflect stop behavior.

Artifacts live under `target/multilingual_color_glyphs-cjk-rtl-color-glyph-reference/`, including
`porterduff_radial_color_glyph-closure-audit.json`, Porter-Duff/radial matrices, multi-reference
render results, diff metrics, disagreement summary, and the HTML report.
