# Prepress Proofing Prepress Close-Out Audit

The machine-readable audit is
`target/prepress_proofing-prepress-closeout/prepress_proofing-closeout-audit.json`.

Audit statuses use only:

- `implemented`
- `implemented_with_limits`
- `unsupported_reported_exact`
- `not_in_prepress_proofing_scope`
- `blocked`

Completion rule: no Prepress Proofing-scope row may remain `blocked`.

Prepress Proofing-scope rows cover OP/op/OPM, DeviceCMYK, Separation, DeviceN,
alpha/transparency/soft-mask interactions, Form XObjects, safe Type3 charprocs,
tiling patterns, shadings, knockout replacement, plate and RGB preview
consistency, native/fallback behavior, benchmark/reference audit, public report
parity, and validation gates.
