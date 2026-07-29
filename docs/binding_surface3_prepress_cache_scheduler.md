# Prepress Proofing Prepress Cache and Scheduler

Prepress Proofing cache identity includes:

- output intent
- rendering intent
- black point compensation
- plate fingerprint
- plate visibility
- fill overprint `op`
- stroke overprint `OP`
- overprint mode `OPM`
- alpha and soft-mask context where present

The separation framebuffer remains sparse and scheduler-accounted. It enforces
the existing caps: 32 prepress plates, 15 n-channel output samples, and the
64 MiB per-page sampled framebuffer budget. Excessive colorants or resource
bombs fail closed or degrade to report-only diagnostics instead of being treated
as successful proof output.
