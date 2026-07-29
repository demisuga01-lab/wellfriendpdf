# crypto writer Bindings

Schema: `crypto_writer.deterministic-writer-pubsec-aesgcm.v1`

This document distinguishes structural support from cryptographic trust or validation claims. PubSec and AES-GCM remain disabled until exact normative dependencies are present.

## crypto writer Verdict

Writer determinism and writer close-out reporting are implemented with limits.
Public-key security-handler decryption and PDF AES-GCM authenticated encryption
remain exact unsupported states because the repository does not contain the
required normative specification text and test vectors. No nonce layout, tag
placement, CMS recipient processing, or AAD rule was inferred.
