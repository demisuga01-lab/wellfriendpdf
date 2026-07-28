# Hyphenation Policy

The runtime uses `hyphenation` 0.8.4 Knuth-Liang dictionaries for `en-US` and
Spanish only. Locale fallback is explicit (`en-*` to `en-US`, `es-*` to `es`),
with recorded provider/data licensing and cache key. Unsupported languages
return `hyphenation_unavailable`; they never receive English patterns.

Candidates respect grapheme boundaries, minimum word/prefix/suffix lengths,
URL/email/dotted-token exclusion, and the documented maximum two consecutive
hyphenated lines policy. That limit is enforced by both the greedy preview and
the bounded final dynamic-programming state graph; no accepted final layout can
contain a third consecutive generated dictionary hyphen. A selected dictionary break emits one visible trailing
hyphen with an empty ToUnicode mapping so logical extraction stays unchanged.
Source soft-hyphen handling and RTL-specific hyphenation policy remain refused.
