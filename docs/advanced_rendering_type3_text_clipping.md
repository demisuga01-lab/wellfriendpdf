# Advanced Rendering Type3 Text Clipping

Type3 CID Rendering adds Type3 text clipping without bounding-box approximation.

Implementation path:

- Text rendering modes `4`, `5`, `6`, and `7` route Type3 glyphs through
  charproc path collection when clipping is active.
- Supported charproc path operators are collected in glyph space:
  `m`, `l`, `c`, `v`, `y`, `h`, `re`, fill operators, stroke operators, `cm`,
  `q`, `Q`, and stroke-style operators.
- The collected path is transformed through FontMatrix, font size, horizontal
  scaling, text rise, text matrix, and CTM before it is unioned into the pending
  text clip mask.
- The accumulated text clip intersects the current clip at `ET`, using the same
  clip stack as ordinary path clipping.

Unsupported and fail-closed behavior:

- Image-only, shading-only, pattern-only, text-only, recursive, or resource-heavy
  charprocs are not converted to fake clip geometry.
- Unsupported Type3 clip extraction sets an empty fail-closed text clip and logs
  the unsupported reason.
- Safety caps: 1 MiB charproc bytes, 4096 charproc operators, 8192 path
  segments, and graphics-state depth 32.

Evidence:

- `cargo test -p wellfriendpdf-engine --test type3_cid_rendering_type3_cid_tensor --jobs 1`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-type3-clip-matrix.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-render-results.json`
- `target/type3_cid_rendering-type3-cid-tensor/type3_cid_rendering-html-report/index.html`

Reference posture:

Poppler, PDFium, and MuPDF render the generated Type3 `Tr` clipping fixtures
without applying the Type3 clip. Wellfriend records those rows as
`unsupported_reported_expected` reference-cluster limitations while keeping the
native path-collection output and local unit tests as the closure proof. No
bbox fake clipping is used.
