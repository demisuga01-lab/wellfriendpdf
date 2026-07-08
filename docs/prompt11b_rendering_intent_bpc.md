# Prompt 11B Rendering Intent And BPC

The native LittleCMS backend maps all four ICC rendering intents:

- Perceptual
- RelativeColorimetric
- Saturation
- AbsoluteColorimetric

Black-point compensation is implemented for native LittleCMS transforms by
passing the LittleCMS BPC flag when the option is requested. The default qcms
fallback continues to report BPC as unsupported.

Unsupported combinations fail closed when LittleCMS cannot create the requested
transform. Prompt 11B does not claim deep BPC equivalence for device-link or
multicolor ICC workflows; those are Prompt 12 owners.
