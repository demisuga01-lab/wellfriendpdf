# Prompt 12 N-Channel Pixel Format

Prompt 12B adds a bounded internal n-channel/intermediate pixel representation
for prepress plate output. It is not an RGB preview buffer and it is not a full
press simulator. Its purpose is to preserve the color structure needed by later
overprint simulation and plate export work.

The representation records:

- dynamic channel labels for 1 through 15 channels
- process component slots for Gray/RGB/CMYK-style process plates
- named spot and DeviceN component slots
- tint values and alternate preview RGB
- alpha/coverage
- source object and color-space provenance
- profile hash and transform key context
- rendering intent and black-point-compensation posture
- output-intent/proofing context
- fallback/native backend status

Memory is bounded by the same prepress caps used by the separation framebuffer:
32 plates by default, a 64 MiB page accounting budget, and fail-closed behavior
for excessive channel counts, excessive plates, huge surfaces, non-finite
values, or malformed profile contexts.

The cache fingerprint includes backend, profile hash, input/output channel
counts, channel labels, rendering intent, BPC state, output intent, and plate
fingerprint. Changing any of those fields must not reuse stale converted pixels
or stale plate samples.

Fallback and WASM builds expose the same report fields, but they do not claim
native n-channel ICC proofing. They are inventory/preview-only for device-link
and multicolor ICC transforms.
