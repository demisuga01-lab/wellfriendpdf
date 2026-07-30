# Provider security

Provider integrations use secret references instead of secret values. Effective configuration, debug output, capability reports, and provider matrices do not serialize API keys, authorization headers, mounted secret contents, private document text, or raw provider responses.

Supported secret-reference families:

- environment variable;
- mounted secret file;
- OS secret store;
- server secret-provider hook.

External providers require explicit network permission, provider configuration, privacy acknowledgement, timeout and rate limits, and cost accounting. Server operators can enforce local-only OCR and prohibit Research.
