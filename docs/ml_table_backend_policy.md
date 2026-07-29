# ML Table Backend Policy

Semantic Closeout ships a proposal contract, validator, deterministic merge, and mock
fixture. It does not ship a production model backend.

## Local Adapter Requirements

A future local adapter must be feature-gated and must receive a user-supplied
model path. Before inference it must verify and report model name, version,
SHA-256, source, license, and runtime. It must use the existing renderer,
deterministic preprocessing, bounded image dimensions, page count, timeout, and
memory. Output must be converted into `TableProposalSet` and validated before
merge.

Default contract limits are:

- timeout: 5,000 ms;
- memory: 256 MiB;
- pages per call: 4;
- image side: 2,048 pixels.

No ONNX, Torch, TableFormer, or Table Transformer dependency or weight is
bundled. The current status is `unsupported_reported_no_runtime`, which is a
supported, report-visible outcome rather than a hidden fallback.

## Cloud Adapter Requirements

There is no production cloud table adapter. A future application integration
must require all of the following before any request:

- explicit enablement and endpoint;
- API-key environment-variable name, never the secret value in a report;
- explicit image/text/metadata payload policy;
- user privacy acknowledgement;
- bounded timeout and retry count;
- response-size limits and Semantic Closeout schema validation;
- fail-closed malformed response behavior;
- no telemetry unless independently disclosed and enabled by the application.

The default is no upload, no endpoint, no retry, no telemetry, and no secret
logging. Mock backends are contract fixtures and are not model intelligence.
