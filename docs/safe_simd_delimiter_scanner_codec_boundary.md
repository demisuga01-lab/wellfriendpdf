# Codec Boundary Safe Delimiter Scanner

## Scope

The Codec Boundary scanner accelerates raw PDF marker candidate discovery. It does not replace PDF parsing and does not decide whether a marker is valid in lexical context.

Covered markers include:

- `obj`
- `endobj`
- `stream`
- `endstream`
- `xref`
- `trailer`
- `startxref`

## Implementation

The scalar reference remains `scan_markers_scalar()` in `crates/engine/src/decode_scanner.rs`.

The accelerated path is `scan_markers_accelerated()`. It uses safe Rust only:

1. collect the first byte of each marker;
2. scan for candidate offsets whose byte can start a marker;
3. validate the full marker at that offset;
4. sort candidates exactly like the scalar reference.

No unsafe SIMD is used because `wellfriendpdf-engine` forbids unsafe code. The implementation is a safe portable acceleration path rather than a CPU-specific SIMD intrinsic path.

## Parser Adoption

Codec Boundary routes parser/reader marker searches through `find_marker_accelerated()` / `rfind_marker_accelerated()` for stream, endstream, xref, trailer, startxref, and object-header recovery scans.

Higher-level code still handles comments, strings, hex strings, name objects, stream length validation, and inline image boundaries.

## Correctness Evidence

Tests compare scalar and accelerated output across:

- comments containing markers;
- literal strings;
- hex/name contexts;
- inline-image-like binary data;
- xref/trailer/startxref data;
- deterministic pseudo-random data;
- metamorphic prefix-padding offsets.

Fuzz target:

- `fuzz/fuzz_targets/decode_scanner.rs`

Artifacts:

- `target/codec_boundary-codec-boundary-scheduler/scanner-parity-report.json`
- `target/codec_boundary-codec-boundary-scheduler/scanner-benchmark-report.json`
