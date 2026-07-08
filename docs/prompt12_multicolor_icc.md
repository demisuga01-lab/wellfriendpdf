# Prompt 12 Multicolor ICC

Prompt 12 inventories ICC profiles whose color-space signatures expose more
than four channels, including `2CLR` through `FCLR`. The report preserves:

- profile hash and byte size
- profile class
- profile color-space signature
- PCS signature
- channel count
- channel labels when known
- declared `/N`
- channel mismatch status
- native and fallback transform posture

Native behavior is conservative. Gray, RGB, and CMYK transforms continue to use
the Prompt 11B CMM paths when legal. Higher-channel multicolor ICC profiles are
not coerced into RGB or CMYK. They are inventory-only or fail closed until the
renderer has a safe n-channel pixel format and destination contract.

DeviceN relationship:

- DeviceN component names and tint values are preserved in the sparse plate
  framebuffer.
- ICC channel labels are metadata unless DeviceN names and channel counts align
  safely.
- process components such as Cyan, Magenta, Yellow, and Black remain distinct
  from named spot or DeviceN plates.

Fallback behavior is explicit: default and WASM builds report multicolor ICC as
unsupported for proofing and may use alternate preview only when the PDF
provides a safe alternate color space.
