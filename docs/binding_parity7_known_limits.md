# Crypto Standards Fuzz known limits

Crypto Standards Fuzz can close only with exact evidence. The following limits must remain explicit:

- Wellfriend PDF SDK is not an accredited standards certification authority.
- PDF/A-4, PDF/A-4e, and PDF/A-4f are exact unsupported unless implemented and proven
  against the selected veraPDF corpus.
- PDF/A-1a, PDF/A-2u, and PDF/A-3u are exact unsupported unless implemented.
- PDF/UA reading-order and semantic human judgement are not mechanically certified.
- Deep PDF/X DeviceN, Separation, overprint, and older-profile transparency behavior is
  bounded unless future corpus evidence expands it.
- Live TSA/OCSP/CRL retrieval remains policy-controlled and not a default network action.
- PAdES-B-LTA document/archive timestamp refresh is not claimed unless separately proven.
- WASM cannot use host filesystem, unrestricted network, OS trust store, or HSM/external
  signer host integration unless a safe host bridge is explicitly provided.
- qpdf is structural evidence only.
- Unavailable optional tools are recorded as unavailable, not passed.
