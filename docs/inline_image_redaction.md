# Inline image redaction

Inline images are recognized by the content tokenizer state machine. BI keys and filter abbreviations are normalized by the shared parser; binary payload bytes are captured without a naive `EI` substring search.

Bounded 8-bit DeviceGray, DeviceRGB, and DeviceCMYK samples can be decoded from raw, Flate, ASCII85, ASCIIHex, RunLength, DCT, JPX, CCITT, or JBIG2 chains when the shared decoder supports the exact input. Affected samples are replaced and emitted as deterministic Flate data with rebuilt BI/ID/EI boundaries. Surrounding operators and other inline images are preserved.

Predictor dictionaries, sub-byte ImageMask data, unsupported color spaces, malformed dictionaries, excessive bytes/pixels, and decoder failures remove the whole inline invocation or fail closed. Promotion to an XObject is reported as an exact unused strategy in the bounded implementation because direct rewrite is deterministic for supported rows.
