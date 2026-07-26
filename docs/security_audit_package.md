# Security audit package

The Prompt 30 package complements prior fuzz, sanitizer, corpus, crypto, and
standards work. It contains a repository threat model, Cargo metadata dependency
inventory, license audit, SBOM fallback, secret scan, and unsafe/native boundary
review. Results are emitted under `target/prompt30-release-readiness/`.

The SBOM fallback is intentionally marked as limited when a dedicated CycloneDX or
similar generator is unavailable: Cargo package metadata is useful evidence but not
a substitute for a distributor-produced transitive SBOM. Likewise, missing audit
tools are recorded as unavailable rather than passed.

Release evidence does not replace an independent paid security review or an
operator's TLS, trust-store, credential, and deployment policy.
