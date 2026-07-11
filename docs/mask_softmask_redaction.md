# Mask and soft-mask redaction

Oxide maps each page polygon through the current graphics transform into image sample space and rejects singular or non-finite transforms. For bounded 8-bit Gray/RGB/CMYK images, it clones the affected Image XObject, replaces conservatively covered samples, writes deterministic Flate bytes, and removes `Mask` and `SMask` from the affected clone. This removes visible color and hidden alpha/mask reachability for that invocation while preserving unaffected shared uses.

ImageMask, packed stencil data, unavailable JPX/CCITT/JBIG2 decoders, unsafe ICCBased/Indexed layouts, excessive pixels, and malformed transforms use complete instance removal or explicit failure. A visual overlay is never secure redaction.

Valid example: a 2x2 RGB image with a grayscale `SMask` is partially redacted; the output clone has rewritten RGB samples and no `SMask` key. Failure example: strict policy applied to a singular transform returns a fail-closed diagnostic.
