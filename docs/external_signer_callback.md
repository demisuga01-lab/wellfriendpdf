# External signer callback

`IncrementalSigner::External` receives a `CmsSigningRequest` for the exact planned signed
bytes. It returns a complete CMS through `CmsSigningResult`; the engine never writes a private
key, password, callback payload, or document bytes to diagnostics.

The engine rejects callback errors, malformed CMS content, certificate fingerprints that do not
match a configured pin, and negotiated algorithms outside the requested policy. It also rejects
a CMS that cannot fit the planned boundary after the allowed retry. The external path shares the
same append-only writer and mandatory post-sign reopen/validation as local signing.

WASM exposes only safe in-memory/report behaviour. Host filesystem, unrestricted network, OS
trust store, and host callback capabilities are not silently simulated.
