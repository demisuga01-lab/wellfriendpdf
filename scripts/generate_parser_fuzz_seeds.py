#!/usr/bin/env python3
"""Generate compact parser fuzz seeds and shallow structure mutations."""

from __future__ import annotations

import argparse
from pathlib import Path


def tiny_pdf() -> bytes:
    pdf = bytearray(b"%PDF-1.7\n")
    obj1 = len(pdf)
    pdf += b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n"
    obj2 = len(pdf)
    pdf += b"2 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n"
    obj3 = len(pdf)
    pdf += b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] >>\nendobj\n"
    xref = len(pdf)
    pdf += (
        f"xref\n0 4\n0000000000 65535 f\n{obj1:010} 00000 n\n"
        f"{obj2:010} 00000 n\n{obj3:010} 00000 n\n"
        f"trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    ).encode()
    return bytes(pdf)


def stream_mismatch_pdf() -> bytes:
    pdf = bytearray(b"%PDF-1.7\n")
    obj1 = len(pdf)
    pdf += b"1 0 obj\n<< /Length 100 >>\nstream\nabc\nendstream\nendobj\n"
    xref = len(pdf)
    pdf += (
        f"xref\n0 2\n0000000000 65535 f\n{obj1:010} 00000 n\n"
        f"trailer\n<< /Size 2 >>\nstartxref\n{xref}\n%%EOF\n"
    ).encode()
    return bytes(pdf)


def incremental_pdf() -> bytes:
    pdf = bytearray(tiny_pdf())
    prev = int(pdf.split(b"startxref\n")[-1].splitlines()[0])
    obj2 = len(pdf)
    pdf += b"2 0 obj\n<< /Type /Pages /Count 0 /Kids [] >>\nendobj\n"
    xref = len(pdf)
    pdf += (
        f"xref\n2 1\n{obj2:010} 00000 n\n"
        f"trailer\n<< /Size 4 /Root 1 0 R /Prev {prev} >>\n"
        f"startxref\n{xref}\n%%EOF\n"
    ).encode()
    return bytes(pdf)


def compact_pdf() -> bytes:
    return b"%PDF-1.7\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Count 0/Kids[]>>endobj\n%%EOF\n"


def mutate_offsets(seed: bytes) -> bytes:
    return seed.replace(b"startxref\n", b"startxref\n999999\n% original ")


def truncate_object(seed: bytes) -> bytes:
    marker = seed.find(b"endobj")
    return seed[:marker] if marker != -1 else seed[:-10]


SEEDS = {
    "minimal.pdf": tiny_pdf,
    "incremental.pdf": incremental_pdf,
    "stream_length_mismatch.pdf": stream_mismatch_pdf,
    "compact_syntax.pdf": compact_pdf,
}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, default=Path("fuzz") / "seeds" / "parser")
    parser.add_argument("--mutations", action="store_true")
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    generated = []
    for name, build in SEEDS.items():
        data = build()
        path = args.out_dir / name
        path.write_bytes(data)
        generated.append(path)
        if args.mutations:
            (args.out_dir / f"bad_offset_{name}").write_bytes(mutate_offsets(data))
            (args.out_dir / f"truncated_{name}").write_bytes(truncate_object(data))
    print(f"generated {len(list(args.out_dir.glob('*.pdf')))} parser seed(s) in {args.out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
