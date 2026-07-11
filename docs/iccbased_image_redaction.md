# ICCBased image redaction

Common ICCBased Gray, RGB, and CMYK images are rewritten in original source-channel space. The ICC profile reference, `/Alternate`, `/N`, `/Decode`, dimensions, and provenance remain attached to the cloned image. Secure mutation does not round-trip through an sRGB preview CMM.

Explicit masks and soft masks are cloned and rewritten with transparent samples in the affected region, clearing hidden source color and alpha while preserving unaffected transparency. Channel mismatch, `/N` outside 1/3/4, unsupported codecs, and unsafe mask layouts fail closed.

The native or fallback CMM used for viewing is separate from source-sample mutation. Preserving a profile reference does not claim colorimetric equivalence for every viewer.
