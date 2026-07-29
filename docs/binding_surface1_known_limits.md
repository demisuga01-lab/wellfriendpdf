# Renderer Fuzz CMM And 11B Known Limits

Renderer parity closure from Renderer Fuzz CMM is sufficient to begin advanced CMM and
prepress work.

Native CMM Backend closes the native LittleCMS backend gap for common ICCBased
Gray/RGB/CMYK preview transforms and basic output-intent proofing foundations.

Remaining bounded limits:

- Full device-link ICC execution.
- Multicolor ICC and n-color transforms.
- True separation framebuffer.
- Spot and DeviceN plate previews.
- Full overprint simulation.
- Certification-grade PDF/X proofing.
- Binding packages that bundle native LittleCMS artifacts by default.

These are exact later owners for Prepress CMM/13. Native CMM implementation itself
is no longer a broad future bucket.
