# Prepress CMM Multicolor ICC

Prepress CMM inventories ICC profiles whose color-space signatures expose more
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
the Native CMM Backend CMM paths when legal. Higher-channel multicolor ICC profiles are
not coerced into RGB or CMYK. Nchannel Plate Prepress adds a bounded n-channel intermediate
pixel representation for 1 through 15 channels, so supported native contexts can
carry high-channel output without dropping labels or process/named component
identity.

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

Remaining native limit:

- if a real multicolor profile requires a pixel format that the safe LittleCMS
  wrapper does not expose, Wellfriend reports the profile, channel count, profile
  class, PCS, hash, and reason, and does not transform it.
