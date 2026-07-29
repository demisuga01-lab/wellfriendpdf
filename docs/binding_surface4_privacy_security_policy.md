# Semantic Intelligence Privacy And Security Policy

Semantic Intelligence keeps ML and cloud behavior opt-in.

Default behavior:

- ML layout backends are disabled
- cloud layout backends are disabled
- no endpoint is configured
- no payload leaves the process
- no API key is read or logged
- deterministic extraction works offline

Cloud enablement requires:

- explicit backend enablement
- explicit endpoint
- explicit payload policy
- explicit user acknowledgement
- explicit page selection
- timeout and retry limits

Payload controls:

- metadata-only mode
- text-only mode
- image-only mode
- text-and-image mode
- redacted-text mode where a caller supplies redacted content
- maximum image side
- maximum pages per call

Audit rules:

- log status, policy, timing, and diagnostics
- do not log document content
- do not log secrets
- reject malformed backend responses
- report blocked cloud requests as privacy-policy diagnostics

Exact limits:

- redacted-payload quality depends on caller-supplied redaction
- real cloud provider terms are outside the SDK and must be supplied by the application
