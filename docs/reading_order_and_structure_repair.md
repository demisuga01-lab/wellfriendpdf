# Reading Order and Structure Repair

document security consumes text reflow reading-order and semantic-flow evidence when
available. It treats painting order as secondary evidence and does not invent
tagged reading order when source evidence is unavailable.

Supported repairs preserve the document language, role and metadata evidence,
marked-content relationships, and recovered ParentTree mappings. Low-confidence
or contradictory reading-order evidence is reported as review-required or
unsupported according to the central document security typed-result contract.
