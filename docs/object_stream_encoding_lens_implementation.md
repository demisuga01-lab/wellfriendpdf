# Atomic object-stream encoding lens - source implementation

This increment extends the revision-bound `object_graph` transaction. It does
not make arbitrary codecs, encryption filters or external-file streams safe, and
it has not been compiled or executed in the current source-only phase.

## Contract

`UniversalObjectMutationV2.stream_encoding` may be supplied only for an existing
object, with `lens_action: "replace"` and a null `value` placeholder. The normal
fingerprint-bound path, including explicit indirect dereference segments, must
select one stream. Planning and apply resolve the same owner and refuse stale
roots, stale dereference targets, cycles, competing write targets and undeclared
global impact.

Three modes are represented by `UniversalStreamEncodingUpdateV2`:

- `raw`: exact already-encoded bytes plus an optional direct Filter name/name
  array and matching null/dictionary DecodeParms value/array;
- `unfiltered`: decoded bytes, with Filter and DecodeParms removed;
- `flate`: decoded bytes compressed by the SDK at level 6 and labelled with one
  direct `FlateDecode` filter.

All modes set direct Length from the final raw bytes, clear stale DL and preserve
every other stream-dictionary entry. External-file streams (`F`, `FFilter` or
`FDecodeParms`) and Crypt filters are refused because they require separate file
or document-security authority. Ordinary object-lens paths still cannot mutate
Length, Filter or DecodeParms independently.

For example, an existing stream can be rewritten without reconstructing its
unrelated dictionary metadata:

```json
{
  "target": {
    "kind": "existing",
    "number": 42,
    "generation": 0,
    "expected_fingerprint": "64_HEX_CHARACTERS"
  },
  "path": [],
  "lens_action": "replace",
  "stream_encoding": {
    "kind": "flate",
    "data": [72, 101, 108, 108, 111]
  },
  "value": { "kind": "null" }
}
```

The request still uses the existing plan/approval/apply flow and declared page
or global-resource impact. It is not a lower-authority shortcut.

## Postconditions and limits

Before serialization, the transaction compares non-encoding dictionary siblings
and applies the inverse to a clone of the parsed owner. After incremental writing,
the exact mutated-object fingerprint and every preserved traversal-owner
fingerprint are checked again. Unfiltered and SDK-Flate modes additionally decode
the reopened selected stream and require the requested length and SHA-256. Raw
mode can prove only the supplied encoded bytes and control graph; the SDK cannot
infer what an arbitrary proprietary filter should decode to.

One additional regression function covers direct Flate sibling preservation,
invalid DecodeParms arity, Crypt/external-file refusal, complete plan/apply/reopen
for unfiltered bytes, inverse-law accounting and decoded-byte postconditions. It
is present but unexecuted. Rust formatting and whitespace checks are not compiler,
codec, interoperability or PDF-corpus evidence.

