# Annotation appearance generation

Prompt 17 normalizes generated appearances as Form XObjects with deterministic `/N`, `/R`, `/D`, `/BBox`, `/Matrix`, resources, ExtGState opacity/blend, and object order. Policies preserve valid appearances, regenerate missing/malformed appearances, or regenerate all supported appearances. Every mutation reports signature impact.

Supported bounded generation includes FreeText; Line/Square/Circle/Polygon/PolyLine; Highlight/Underline/Squiggly/StrikeOut with arbitrary quad polygons; Stamp; Caret; Ink strokes; Text/FileAttachment icons; common Widget chrome/text; and Redact previews. Border width/dash, opacity, safe blend modes, line endings, deterministic cloudy-vector approximations, rotation matrices, and repeated Redact overlay text are retained in the generated stream. Redact AP is visual preview only and is explicitly separate from applied secure redaction.

Valid static AP for PrinterMark, TrapNet, Watermark, 3D, RichMedia, Movie, Sound, Screen, and unknown subtypes is preserved and can be flattened. A deterministic `INERT` placeholder is available only under explicit policy.

```text
oxide annotation-appearance-generate input.pdf --output appearances.pdf --json
oxide annotation-appearance-report input.pdf
```

Exact limits: FreeText uses sanitized plain text with bounded Helvetica/WinAnsi layout; advanced CSS, full bidi shaping, CJK font fallback embedding, exact proprietary stamp art, and pixel-identical Acrobat-private cloudy-border geometry are reported limits. Oxide's cloud geometry is a bounded deterministic vector approximation. Unsupported generation never silently succeeds.
