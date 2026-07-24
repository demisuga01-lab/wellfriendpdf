# XFA security threat model

Trust boundary: every PDF object, decoded stream, XML byte, dataset node, measurement, SOM expression, script, event, image reference, and connection packet is hostile.

Primary threats are XML entity/DTD abuse, decompression and node bombs, recursive/repeated layout explosion, script loops/allocations, host escape, external data exfiltration, hidden text regeneration after redaction, duplicate-packet ambiguity, and signature/certification invalidation. Controls are bounded decode/XML/model/layout/script resources, no external resolvers or host APIs, default-disabled active content, deterministic ordering/diagnostics, cancellation, scheduler admission, fail-closed mutation modes, post-sanitize rescan, and signature/redaction posture reports.

Residual risks are exact unsupported LiveCycle layout and proprietary behavior. Wellfriend does not claim that unflattened unsupported dynamic XFA is securely redacted.
