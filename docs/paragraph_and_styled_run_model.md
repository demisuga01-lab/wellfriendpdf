# Paragraph and Styled-Run Model

`ParagraphStyleModel` exposes exact advanced editing token spans, font-resource
identity, marked-content depth, Unicode scalar range, and grapheme range for a
source-linked paragraph. It can report multiple source spans and never
fabricates a style mapping.

`font_policy=preserve_original_per_run` activates an executable, source-linked
multi-run serializer for a narrow supported boundary. It removes the selected
source operands and replays their existing font resource, font size,
character/word spacing, horizontal scaling, text rise, text rendering mode,
and DeviceGray/RGB/CMYK paint state at the final line positions. The existing
font CMap encodes the replacement, so it does not silently substitute a font or
flatten the paragraph to one generated Type0 style.

This boundary requires one contiguous page content stream, an unambiguous
whole-token selection, a unique encoding in every original CMap, and
left-to-right horizontal writing. A changed-length replacement assigns every
complete replacement grapheme to a deterministic proportional source-style
owner. That preserves the source style order without flattening fonts or
splitting a grapheme; it does not pretend to infer an author-selected style
for inserted text. Runtime fixtures cover equal- and changed-length text with
two fonts, two sizes, and two colors. RTL/mixed-bidi output, nested or partial
MCID/property-list preservation, links, arbitrary color spaces, source
clipping text, inserted dictionary hyphens, and vertical writing remain typed
refusals.

Invisible text rendering mode is replayed. Text clipping modes are refused,
because relocating clipping text into an independent generated stream would
alter clipping for later source graphics.

`rebuild_subset_or_generated_type0` remains the default for normal source
rewrites; it preserves logical text, shaping, alignment, and bounded spacing
but intentionally does not claim preservation of arbitrary source styles.
