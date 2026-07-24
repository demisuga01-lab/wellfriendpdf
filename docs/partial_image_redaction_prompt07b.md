# Prompt 07B Partial Image Redaction

Prompt 07 removed intersecting image invocations conservatively. Prompt 07B adds
pixel-level partial image redaction for the safe common case.

## Supported

- Image XObject detection through content stream `Do` operators.
- Page-space redaction rectangles mapped into image pixels for axis-aligned
  image placement.
- 8-bit decoded DeviceGray and DeviceRGB image pixels are rewritten.
- A new image XObject is created for the affected invocation.
- The original image object remains untouched unless full removal is requested.
- Unsupported formats follow `ImageRedactionPolicy`: `partial`, `remove`, or
  `fail`.

CLI:

```powershell
wellfriendpdf redact input.pdf --rect 1:200,500,50,100 --image-policy partial --out redacted.pdf --strict
```

## Limits

- Non-axis-aligned image transforms are not partially rewritten.
- CMYK, masks, CCITT/JBIG2/JPX edge cases may fall back to removal or fail
  depending on policy.
- Inline image partial rewriting remains conservative.

