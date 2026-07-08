# Prompt 11B ICC Transform Backend

The native transform backend supports common PDF ICCBased preview paths:

- Gray profile bytes plus one-channel pixels to sRGB.
- RGB profile bytes plus three-channel pixels to sRGB.
- CMYK profile bytes plus four-channel pixels to sRGB.

The native path validates profile parsing and channel count through LittleCMS
before transform creation. Oversized profiles fail before parse. Malformed or
channel-mismatched profiles fail closed.

The default qcms fallback remains available in builds without
`native-cmm-lcms2`.

The transform cache is thread-local and bounded. Keys include:

- backend
- profile hash
- profile length
- channel count
- source and destination pixel formats
- rendering intent
- black-point compensation option

This prevents stale reuse when backend, profile, intent, or BPC changes.
