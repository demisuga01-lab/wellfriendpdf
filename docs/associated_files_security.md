# Associated files security

Extraction is byte/count capped and passes through the shared decode scheduler. Names are reduced to one path component; controls, separators, traversal components, and Windows reserved device names are rejected or sanitized. External and URL specifications are inventoried but never opened, fetched, or executed.

Mutation supports deterministic add, hash-based stream deduplication, selected/all removal, and sanitizer policies. Secure removal is a full rewrite because an incremental revision would retain the old payload. The output is reopened and inventoried again.

Executable or active extensions/MIME types are removed by the executable-or-unknown policy. Custom preservation requires both MIME and AFRelationship allowlists.
