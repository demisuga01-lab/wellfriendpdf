# PDF/X validation

The PDF/X validator reports GTS_PDFX/XMP identification, output intent, page-box posture,
embedded-font posture, risky active content, and supported color/prepress evidence. Missing
output intent, missing profile identification, invalid page boxes, and non-embedded fonts are
reported as failures.

Deep DeviceN/Separation/overprint analysis and older PDF/X transparency corpus parity are
explicit Prompt 27 deferrals. They are not hidden warnings and cannot create a conformant PDF/X
result. The validator does not claim RIP or print-certification equivalence.
