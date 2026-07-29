# Semantic Intelligence Cloud Layout Backend Template

`MockCloudLayoutBackend` is the Semantic Intelligence cloud backend template. It is safe by
default and performs no network request in tests.

Cloud backend requirements:

- backend must be explicitly enabled
- endpoint must be explicitly configured
- API keys are referenced by environment/config name only
- API key values are never logged
- user privacy acknowledgement is required
- payload policy must be explicit
- page selection and payload type must be explicit
- unsafe defaults return diagnostics and send no payload

Payload policies:

- metadata only
- text only
- image only
- text and image
- redacted text only

Audit behavior:

- diagnostics record status and policy decisions
- audit logs do not include document content
- audit logs do not include secret values
- malformed responses fail closed
- rate limit, timeout, disabled, and invalid-schema states are separately reportable

Exact limits:

- real cloud integration is not implemented by default
- no default endpoint exists
- no document can leave the process without explicit application configuration
