# Non-axis partial image redaction and proof posture

Requests contain page-space polygons or rotated-CropBox-space polygons. The engine maps page rotation/CropBox offsets, tracks the content CTM, rejects singular/non-finite transforms, inverts arbitrary rotation/skew/reflection/nonuniform/negative-scale image CTMs, and maps the real polygon to image samples. A conservative cell/polygon intersection plus a one-sample safety margin prevents residual boundary pixels.

For decodable 8-bit Gray/RGB/CMYK Image XObjects, only the affected invocation receives a deterministic cloned resource and rewritten stream; other uses retain the original. Rewritten clones omit original Mask/SMask references. Decodable DCT/JPX/CCITT/JBIG2/Indexed/ICCBased inputs participate when the canonical image decoder yields supported samples.

Unsupported bit depths/decoders, singular transforms, inline images, and unproven nested Form rewrites use per-invocation secure removal or explicit fail-closed policy. Clipping is ignored for coverage, which may remove extra pixels but cannot preserve clipped sensitive pixels. Overlay marks provide visible feedback only and are never a security proof.

```json
{
  "requests": [{
    "page": 1,
    "polygon": [[55,50],[95,60],[90,95],[48,82]],
    "coordinate_space": "pdf_user_space",
    "fallback_policy": "secure_rewrite_or_remove",
    "fill": [0,0,0]
  }],
  "deterministic": true,
  "fail_on_unsupported": false
}
```

```text
oxide redact-image-nonaxis input.pdf plan.json --dry-run
oxide redact-image-nonaxis input.pdf plan.json --output redacted.pdf --json
```

Full rewrite removes prior revision bytes. An unaffected reuse may intentionally keep the original source image reachable; the redacted invocation no longer references those samples.
