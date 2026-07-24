# Font Reconstruction

Prompt 21 adds a safe font reconstruction report layer. It uses the existing font inventory and records reconstruction levels:

| Level | Behavior |
| --- | --- |
| `metadata_repair` | Builds deterministic metadata posture from font dictionaries and descriptors. |
| `unicode_mapping_repair` | Uses ToUnicode, encodings, predefined CMap evidence, and reports unresolved glyphs. |
| `encoding_cmap_repair` | Reports bounded CMap repair eligibility. |
| `outline_repackage` | Reports eligibility only when embedded outlines exist. |
| `subset_rebuild` | Requires existing outlines plus mapping evidence. |
| `external_glyph_generation_hook` | Disabled by default; no model weights are bundled. |

This is a repaired-font/eligibility report, not a claim that Wellfriend recovered the original font family. Deterministic names use an Wellfriend prefix and object number, and unresolved glyphs remain explicit.

Artifacts: `font-reconstruction-levels-prompt21.json`, `font-outline-repackage-prompt21.json`, and `font-subset-rebuild-prompt21.json`.
