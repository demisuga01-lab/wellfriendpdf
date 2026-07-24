# Prompt 26 performance and security posture

The performance audit records bounded timings and memory for plan/sign/reopen, per-family
validation, validate-all, and cross-profile aggregation. It also covers large metadata,
structure-tree, font, page, and signature cases where fixtures are available. Results are
environment evidence, not universal throughput claims.

No private keys, callback secrets, passwords, tokens, SSH keys, or raw confidential document
content may appear in logs or reports. Signing tests generate temporary credentials only in the
task temp directory and remove them after evidence capture. The secret scan records matches by
file/category without echoing any candidate secret value.
