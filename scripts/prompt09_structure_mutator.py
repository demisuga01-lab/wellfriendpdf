#!/usr/bin/env python3
"""Prompt 09 structure-aware PDF mutator.

This is a deterministic, offline helper for smoke-scale hostile-PDF testing. It
keeps the `%PDF` envelope and mutates common COS structures so parser,
sanitizer, signature, validation, and writer paths see malformed-but-recognizable
documents. It does not execute document actions and performs no network access.
"""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path


MUTATIONS = {
    "length_zero": lambda b: b.replace(b"/Length ", b"/Length 0 %", 1),
    "byte_range_overlap": lambda b: b.replace(
        b"/ByteRange [0 ", b"/ByteRange [0 10 5 ", 1
    ),
    "inject_javascript": lambda b: b.replace(
        b"/Type /Catalog",
        b"/Type /Catalog /OpenAction << /S /JavaScript /JS (app.alert('mutated')) >>",
        1,
    ),
    "inject_launch": lambda b: b.replace(
        b"/Type /Catalog",
        b"/Type /Catalog /OpenAction << /S /Launch /F (calc.exe) >>",
        1,
    ),
    "corrupt_output_intents": lambda b: b.replace(b"/OutputIntents", b"/OutputIntents 42 %", 1),
    "corrupt_structtree": lambda b: b.replace(b"/StructTreeRoot", b"/StructTreeRoot 7", 1),
    "duplicate_object_header": lambda b: b.replace(b"1 0 obj", b"1 0 obj\n1 0 obj", 1),
}


def mutate(data: bytes, name: str) -> bytes:
    fn = MUTATIONS[name]
    mutated = fn(data)
    if mutated == data:
        mutated = data + f"\n% wellfriendpdf-mutator:{name}\n".encode("ascii")
    return mutated


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate Prompt 09 structure-aware PDF mutations.")
    parser.add_argument("input", type=Path)
    parser.add_argument("--out-dir", type=Path, default=Path("target/prompt09-structure-mutations"))
    parser.add_argument("--list", action="store_true", help="list mutation names and exit")
    args = parser.parse_args()

    if args.list:
        for name in sorted(MUTATIONS):
            print(name)
        return 0

    data = args.input.read_bytes()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    manifest = []
    for name in sorted(MUTATIONS):
        mutated = mutate(data, name)
        digest = hashlib.sha256(mutated).hexdigest()
        out = args.out_dir / f"{args.input.stem}.{name}.pdf"
        out.write_bytes(mutated)
        manifest.append(f"{out.name},{name},{len(mutated)},{digest}")
    (args.out_dir / "manifest.csv").write_text(
        "file,mutation,bytes,sha256\n" + "\n".join(manifest) + "\n",
        encoding="utf-8",
    )
    print(f"wrote {len(manifest)} mutation(s) to {args.out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
