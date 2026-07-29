# Release Packaging Codec Threat Model

This threat model is specific to Wellfriend's current decode architecture. It covers hostile PDF stream data that reaches `crates/engine/src/filters.rs`, image decode modules, encrypted streams, inline images, and binding-facing report surfaces.

## Trust Boundaries

- Untrusted input: PDF bytes, stream dictionaries, `/Filter` arrays, `/DecodeParms`, image dimensions, inline image payloads, encrypted stream bytes after decryption, and package-consumer byte arrays passed to codec reports.
- Trusted parent: API policy selection, decode limits, timeout limits, worker path, request IDs, response validation, and deterministic JSON envelopes.
- Less-trusted child: `wellfriendpdf-codec-worker`, which decodes a single bounded request and returns JSON to the parent.
- Binding boundary: Rust, CLI, Python, C ABI, WASM, .NET, and Java must receive stable reports and must not observe parent crashes from worker failures.

## Codec Families

| Family | Current Release Packaging worker status | Primary attacker goals | Controls |
| --- | --- | --- | --- |
| Flate/predictors | Implemented for stream bytes | Decompression bombs, predictor amplification, malformed zlib streams | input cap, decoded cap, worker timeout, response cap |
| RunLength | Implemented | Output amplification, malformed run packets | decoded cap, structured decode failure |
| ASCIIHex/ASCII85 | Implemented | Malformed terminator, oversized decoded output | decoded cap, structured failure |
| LZW | Implemented | Dictionary growth, malformed codes | decoded cap, timeout, structured failure |
| DCT/JPEG | Reported unsupported by worker | Complex decoder bugs, large image allocation | in-process budget checks, fail-closed isolated policy |
| JPX/JPEG 2000 | Reported unsupported by worker | Tile/memory amplification, complex codestream parsing | dimension/budget checks, fail-closed isolated policy |
| JBIG2 | Reported unsupported by worker | Symbol dictionary abuse, exploited decoder class | dimension/budget checks, no silent fallback in required mode |
| CCITT | Reported unsupported by worker | Bitstream parser hazards, row/dimension mismatch | dimension/budget checks, no silent fallback in required mode |
| Crypt/encrypted streams | Parent decrypts before filter decode | Confuse parser/decrypt/filter order | policy reports distinguish codec phase from crypto phase |
| Inline images | Parent parser supplies bytes and dimensions | Parser interaction, tiny object with huge declared output | parent dimension report and decode caps |

## Policy Decisions

- Default policy is `in_process` to preserve existing behavior.
- `isolated_required` fails closed when the worker is missing, unsupported, crashes, times out, returns invalid JSON, returns a mismatched request ID, or exceeds response/output caps.
- `isolated_preferred` may fall back to in-process only by explicit policy and reports `fallback_used` with a reason.
- `report_only` performs no decode and returns a deterministic diagnostic envelope.
- WASM/browser targets cannot spawn workers and therefore report subprocess isolation as unavailable.

## Residual Risk

Subprocess isolation contains worker crash, timeout, and oversized response failures. It is not a formally verified sandbox, not a syscall filter, and not a substitute for container/OS sandboxing in hostile multi-tenant deployments.

## Codec Boundary Native Boundary Update

Codec Boundary adds a central codec backend registry and an explicit native/C codec policy:

- pure Rust remains the default implementation posture;
- native/C codec dependencies are denied by default;
- the native dependency allowlist is empty;
- future native entries must require the `native-codecs` feature, worker/sandbox execution, and report fields;
- native in-process decode is forbidden by default;
- unknown native dependencies must fail closed rather than silently falling back.

RLBox/WASM sandboxing is not claimed. The Codec Boundary feasibility artifact hard-blocks it for this repository state because the required C/C++ WASM toolchain and reproducible sandbox integration were not available in the local evidence pass.

Renderer decode now uses scheduler memory tokens for image, inline image, soft mask, Form, annotation, tiling-pattern, and mesh-shading decode paths while preserving deterministic content order.
