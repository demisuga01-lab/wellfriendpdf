# XFA packet inventory

`xfa-report` accepts both AcroForm `/XFA` forms: an ordered array of packet-name/stream pairs and one XDP stream. For an XDP stream, validated child subtrees retain inherited namespace context.

Each packet reports order, name, PDF object reference, decoded length, SHA-256, root name and namespace, parse/duplicate/malformed state, encryption/decode posture, byte offsets, and diagnostics. Duplicate packets are preserved in source order. Recognized names include `template`, `datasets`, `form`, `config`, `localeSet`, `connectionSet`, `sourceSet`, `xmpmeta`, and ancillary/unknown names. Inventory does not imply execution.

Decode and XML caps come from `XfaLimits`. A malformed packet is rejected and reported without preventing unrelated PDF page access. Packet bytes and raw unsafe handles are never exposed through public bindings.
