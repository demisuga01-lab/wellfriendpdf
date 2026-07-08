# Prompt 12 Spot And DeviceN Plate Rendering

Separation color spaces now produce plate records that preserve:

- spot name
- tint value
- alternate preview
- alpha
- page/object provenance where available
- operation type
- overprint-pending posture

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

Images, shadings, patterns, and full text outline plate emission remain
report-visible limits when the renderer cannot safely write true plate
contributions yet.
