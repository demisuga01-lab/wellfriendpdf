# Packed mask and Indexed redaction

Wellfriend rewrites packed rows in sample space for one-bit stencils and 1/2/4/8-bit Indexed images. Row padding is decoded and rebuilt exactly. The polygon is inverse-mapped through the active image CTM; non-finite or singular mappings fail closed.

Stencil replacement uses the non-painting bit selected by `/Decode`. Indexed replacement selects a deterministic closest palette entry and replaces the affected indices themselves. The original lookup, `/Decode`, interpolation, color-key mask, dimensions, and unaffected samples remain unchanged. Shared invocations receive a cloned XObject.

Lossless filter completion and an exact decoded row length are mandatory. Unsupported codecs, malformed lookups, non-device Indexed bases, or ambiguous sample counts remove the invocation or error under strict policy. An overlay is never a secure result.
