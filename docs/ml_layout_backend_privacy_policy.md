# ML Layout Backend Privacy Policy

Cloud layout remains disabled by default. No document payload may leave the
process unless an application explicitly configures:

- endpoint;
- secret source through environment/config;
- payload policy;
- page or region selection;
- timeout/retry limits;
- privacy acknowledgement.

Secrets are never logged. Mock cloud tests do not perform real network calls.
Malformed responses fail closed and deterministic extraction remains primary.
