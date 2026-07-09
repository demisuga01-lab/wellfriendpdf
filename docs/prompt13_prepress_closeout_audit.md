# Prompt 13 Prepress Close-Out Audit

The machine-readable audit is
`target/prompt13-prepress-closeout/prompt13-closeout-audit.json`.

Audit statuses use only:

- `implemented`
- `implemented_with_limits`
- `unsupported_reported_exact`
- `not_in_prompt13_scope`
- `blocked`

Completion rule: no Prompt 13-scope row may remain `blocked`.

Prompt 13-scope rows cover OP/op/OPM, DeviceCMYK, Separation, DeviceN,
alpha/transparency/soft-mask interactions, Form XObjects, safe Type3 charprocs,
tiling patterns, shadings, knockout replacement, plate and RGB preview
consistency, native/fallback behavior, benchmark/reference audit, public report
parity, and validation gates.
