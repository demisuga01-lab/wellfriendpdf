# Canonical PDF Serialization

Schema: `prompt23.deterministic-writer-pubsec-aesgcm.v1`

Canonical serialization is based on deterministic object traversal, dictionary key ordering, finite number formatting, stream length normalization, xref policy, and reopened-output verification.

## Prompt 23 Verdict

Writer determinism and writer close-out reporting are implemented with limits.
Public-key security-handler decryption and PDF AES-GCM authenticated encryption
remain exact unsupported states because the repository does not contain the
required normative specification text and test vectors. No nonce layout, tag
placement, CMS recipient processing, or AAD rule was inferred.
