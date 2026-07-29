"""Release Packaging codec isolation report example.

Build/install the wheel, then run:

    python crates/wellfriendpdf-py/examples/codec_isolation_report.py

The example uses the public `wellfriendpdf.codec_isolation_report` helper and prints the
same versioned envelope shape returned by Rust, CLI, C ABI, WASM, .NET, and
Java.
"""

import json
import sys
import zlib

import wellfriendpdf


def main() -> int:
    policy = sys.argv[1] if len(sys.argv) > 1 else "in_process"
    encoded = zlib.compress(b"hello wellfriendpdf")
    envelope = wellfriendpdf.codec_isolation_report("FlateDecode", encoded, policy=policy)

    print(json.dumps(envelope, indent=2))
    report = envelope["report"]
    if report["status"] != "success":
        print(f"codec isolation status: {report['status']}", file=sys.stderr)
        return 1
    print(f"decoded bytes: {report['decoded_byte_length']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
