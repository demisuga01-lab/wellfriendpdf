# Prompt 12 Spot And DeviceN Plate Rendering

Separation color spaces now produce plate records that preserve:

- spot name
- tint value
- alternate preview
- alpha
- page/object provenance where available
- operation type
- Prompt 13 overprint posture

DeviceN color spaces produce one contribution per component. Process colorants
are marked as process plates; non-process named components are marked as
DeviceN plates. The report keeps these concepts separate instead of flattening
everything into RGB.

Tint transforms are used for preview when the existing bounded function
evaluator can execute them safely. Malformed or excessive functions are
reported. The tint transform result is only alternate preview data; it does not
replace named plate preservation.

Supported Prompt 12 write paths:

- simple fill
- stroke
- fill-then-stroke
- child group plate absorption for report continuity

Prompt 12B extends supported write paths:

- text fill and stroke modes that paint, including supported Type0/CID and
  Type3 path geometry
- stencil images using the current Separation/DeviceN color
- named Separation/DeviceN image color-space samples where resolvable
- axial/radial/mesh shading plate samples where the shading color space is
  resolvable
- colored tiling patterns and uncolored caller-color pattern samples
- shading pattern samples

Exact remaining limits are narrow: resource-heavy Type3 charprocs with nested
XObjects/shadings/images and unsafe high-channel packed image layouts are
report-visible or fail closed. Full overprint compositing remains Prompt 13.
