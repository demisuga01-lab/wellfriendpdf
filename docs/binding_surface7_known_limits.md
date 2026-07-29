# annotation/media redaction exact known limits

- XFDF rich text is bounded sanitized plain text, not arbitrary XHTML/CSS fidelity.
- XFDF actions, attachments, and media payloads are inventory/policy metadata; import never creates active actions or payloads.
- Standalone Widget creation is unsupported; existing Widgets update through canonical field semantics.
- FreeText CJK fallback embedding, full bidi shaping, proprietary stamp art, and pixel-identical Acrobat cloudy borders are reported limits; cloudy borders use a bounded deterministic vector approximation.
- Media playback, media codec decode, Flash/SWF execution, 3D execution/JavaScript, external URLs, and launch actions are never supported by this policy layer.
- A media poster can be flattened only from a valid static AP. Unsafe payload decode is never used to create one.
- Direct Image XObjects require canonical decoder output with 8-bit Gray/RGB/CMYK samples for partial rewrite. Unsupported inputs use secure invocation removal or strict failure.
- Sub-byte and stencil images are decoded defensively for rendering, but annotation/media redaction does not claim sample-space rewriting for them; intersecting invocations are removed or the strict policy fails closed.
- Inline images are removed as complete BI/ID/data/EI groups when intersecting; partial inline re-encoding is not claimed.
- Nested Forms are removed at the affected invocation when bounded recursive sample rewrite cannot be proven. This favors security over fidelity.
- Clipping is conservatively ignored for sample coverage; more pixels may be removed.
- Prior byte-range signatures are invalidated by every annotation/media redaction full rewrite.
- Validation is admitted under a 4 GiB ceiling with one Cargo build/test job and one Rayon thread. Windows does not provide a portable subprocess peak-RSS reading in this harness, so the evidence records the configured cap and serial posture rather than inventing a measured peak.
