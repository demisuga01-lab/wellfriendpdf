# Wellfriend PDF SDK threat model

Primary assets are untrusted PDF bytes, extracted plaintext, rendering and redaction
outputs, signing keys/callback data, trust decisions, and memory ownership across
native bindings. The main attacker input is a hostile PDF or adjacent public API
value supplied to parsing, rendering, editing, cryptographic, or CLI paths.

Trust boundaries include the parser and codec limits, content renderer limits,
writer/edit policy, C ABI pointer/ownership boundary, external validator output,
optional network evidence retrieval, and build/package supply chain. Invariants are
bounded CPU/memory/output, structured failure for malformed content, no executable
PDF JavaScript, no implicit network fetch during normal parsing/rendering, no key or
secret logging, and fail-closed signature/evidence decisions.

Deployment-specific boundaries remain the operator's responsibility: TLS termination,
OS trust store configuration, HSM policy, and network egress. See `SECURITY.md` for
reporting and known limitations.
