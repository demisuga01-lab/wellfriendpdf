# Prompt 23B Normative Crypto Closure Audit

Status: `blocked_normative_dependency`

Prompt 23B started from clean HEAD
`29c7d14f7e975b5f591f078c1fa502eb2531ca6f`:

- `git status --short`: no entries
- `git rev-parse HEAD`: `29c7d14f7e975b5f591f078c1fa502eb2531ca6f`
- top commit: `29c7d14 Complete combined prompt 23 deterministic writer pubsec aesgcm`

## Gate Result

Prompt 23B requires exact normative text and vectors before any implementation
of public-key security-handler CMS recipient processing or PDF AES-GCM crypt
filters. The repository does not contain legally usable local copies of the
required PDF-family normative documents:

- ISO 32000-2:2020, including applicable errata/resolutions.
- ISO/TS 32003:2023.
- ISO/TS 32004:2024.
- ISO/TS 32002:2022, where CMS/signature terminology intersects the feature.

The available repository material includes Prompt 23 report-only docs, existing
Standard security-handler code, and CMS signature dependencies, but not the
required clauses for PubSec recipient payloads, AES-GCM nonce/tag/AAD layout,
PDF crypt-filter representation, file-key recovery, or interoperable vectors.

No code was changed for cryptography, CMS recipient recovery, AES-GCM object
handling, key providers, or bindings because doing so would require guessing
standardized byte layouts.

## Local Search Evidence

The following checks were performed before stopping:

- `rg --files | rg -i "(iso.?32000|32003|32004|32002|aes.?gcm|pubsec|public.?key|cms|pkcs|x509|rsa|errata|spec|standard|normative)"`
- `rg -n -i "(ISO/TS 32003|ISO 32000-2|ISO/TS 32004|ISO/TS 32002|Adobe\\.PubSec|PubSec|AES.?GCM|CMS|EnvelopedData|KeyTransRecipientInfo|RSAES|OAEP|authenticated encryption|normative)" docs crates scripts bindings tests fuzz .github`
- `rg --files -u | rg -i "(iso.?32000|iso.?ts.?32003|iso.?ts.?32004|iso.?ts.?32002|32000-2|32003|32004|32002|pdf.?2.?0|aes.?gcm|pubsec|public.?key.*security|cms|pkcs.?7|rfc5652|rfc5280|rfc8017|rfc5084|rfc5116|nist.*38d|sp800.*38d|x509|x\\.509|errata).*\\.(pdf|txt|md|html|json|xml|der|pem)$"`
- `Get-ChildItem -LiteralPath references -Recurse -Force`

The `references` directory contains only general engineering checklists. No
local normative PDF-family source files or interoperable AES-GCM/PubSec vector
sets were found.

## Decision

Prompt 23B is blocked before implementation. The next valid step is to acquire
the required standards through a project-approved, legally usable route and
store either ignored local copies or recorded local paths/hashes so the
normative dependency gate can pass.
