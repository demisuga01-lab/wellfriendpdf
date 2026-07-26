# Release readiness report

The machine-readable go/no-go is
`target/prompt30-release-readiness/release-readiness-go-no-go.json`. The allowed
outcomes are `release_ready`, `release_ready_with_limits`, and `not_release_ready`.

`release_ready_with_limits` is appropriate when all Prompt 30-owned gates pass but
external validators, comprehensive third-party SBOM generation, paid audit work, or
deployment-local trust/TLS policy remain outside the repository's directly verified
scope. The final status must be backed by the final validation matrix, not this prose.
