# Glyph Generation Hook

The writer history glyph-generation hook is an external backend contract. It is disabled by default and does not upload glyph evidence or call cloud services silently.

Required backend fields include:

| Field | Purpose |
| --- | --- |
| backend id/version | Audit the provider implementation. |
| input glyph evidence | Show what visual/font evidence was used. |
| Unicode target and neighboring policy | Bind output to explicit requested glyphs. |
| output outline or bitmap | Carry generated geometry. |
| confidence | Keep generated results distinct from original evidence. |
| license/provenance | Prevent accidental redistribution claims. |
| deterministic seed/settings | Report reproducibility posture. |
| privacy and local/cloud status | Make data movement explicit. |

Generated glyphs are marked generated and are never treated as licensed original font glyphs.
