# XFA XML security

Wellfriend uses a strict, non-resolving XML parser owned by the XFA module. It has no DTD loader, entity resolver, URL client, filesystem callback, or native XML library hook.

Defaults are 16 MiB total XML, 8 MiB decoded per packet, 64 packets, 100,000 nodes, 250,000 attributes, 8,192 namespace declarations, depth 64, 1 MiB per text/attribute value, and 100,000 entity references. Only XML predefined and numeric character references are decoded. DTD/entity declarations, unknown entities, invalid UTF-8, non-finite numeric measurements, and unknown measurement units are rejected deterministically.

Source offsets are byte offsets into the decoded packet where feasible. In single-stream XDP, logical child packet offsets reference the parent stream and the already-validated subtree retains inherited namespaces.
