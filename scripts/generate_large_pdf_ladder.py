#!/usr/bin/env python3
"""Generate deterministic synthetic large-PDF fixtures for Oxide profiling.

The generator writes PDF bytes incrementally so multi-GB fixtures can be created
without holding the PDF in memory. Fixtures are intentionally simple: one page
tree, one Helvetica font, and one content stream per page. Size-axis fixtures use
large uncompressed content streams; page-axis fixtures use many small pages.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import BinaryIO


BASE_STREAM_TEMPLATE = b"BT /F1 12 Tf 72 720 Td (Synthetic page %06d) Tj ET\n"
PADDING_CHUNK = b"% " + (b"x" * 8190) + b"\n"


def write_payload(out: BinaryIO, page: int, length: int) -> None:
    prefix = BASE_STREAM_TEMPLATE % page
    if length < len(prefix):
        length = len(prefix)
    out.write(prefix)
    remaining = length - len(prefix)
    while remaining > 0:
        chunk = PADDING_CHUNK[: min(remaining, len(PADDING_CHUNK))]
        out.write(chunk)
        remaining -= len(chunk)


def write_obj(out: BinaryIO, offsets: list[int], number: int, body: bytes) -> None:
    offsets[number] = out.tell()
    out.write(f"{number} 0 obj\n".encode("ascii"))
    out.write(body)
    out.write(b"\nendobj\n")


def generate_pdf(output: Path, pages: int, stream_bytes_per_page: int) -> dict[str, int | str]:
    if pages <= 0:
        raise ValueError("pages must be positive")
    if stream_bytes_per_page <= 0:
        raise ValueError("stream-bytes-per-page must be positive")

    output.parent.mkdir(parents=True, exist_ok=True)
    max_object = 3 + pages * 2
    offsets = [0] * (max_object + 1)
    with output.open("wb") as out:
        out.write(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        write_obj(out, offsets, 1, b"<< /Type /Catalog /Pages 2 0 R >>")
        kids = " ".join(f"{4 + (page - 1) * 2} 0 R" for page in range(1, pages + 1))
        write_obj(
            out,
            offsets,
            2,
            f"<< /Type /Pages /Count {pages} /Kids [{kids}] >>".encode("ascii"),
        )
        write_obj(out, offsets, 3, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")

        for page in range(1, pages + 1):
            page_obj = 4 + (page - 1) * 2
            content_obj = page_obj + 1
            page_body = (
                f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
                f"/Resources << /Font << /F1 3 0 R >> >> /Contents {content_obj} 0 R >>"
            ).encode("ascii")
            write_obj(out, offsets, page_obj, page_body)

            offsets[content_obj] = out.tell()
            out.write(f"{content_obj} 0 obj\n".encode("ascii"))
            out.write(f"<< /Length {stream_bytes_per_page} >>\nstream\n".encode("ascii"))
            write_payload(out, page, stream_bytes_per_page)
            out.write(b"\nendstream\nendobj\n")

        startxref = out.tell()
        out.write(f"xref\n0 {max_object + 1}\n".encode("ascii"))
        out.write(b"0000000000 65535 f \n")
        for number in range(1, max_object + 1):
            out.write(f"{offsets[number]:010d} 00000 n \n".encode("ascii"))
        out.write(
            (
                f"trailer\n<< /Size {max_object + 1} /Root 1 0 R >>\n"
                f"startxref\n{startxref}\n%%EOF\n"
            ).encode("ascii")
        )

    return {
        "path": str(output),
        "bytes": output.stat().st_size,
        "pages": pages,
        "stream_bytes_per_page": stream_bytes_per_page,
        "objects": max_object,
    }


def cmd_generate(args: argparse.Namespace) -> int:
    metadata = generate_pdf(Path(args.output), args.pages, args.stream_bytes_per_page)
    print(json.dumps(metadata, indent=2))
    return 0


def cmd_ladder(args: argparse.Namespace) -> int:
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    manifest: list[dict[str, int | str]] = []

    for target_mb in args.size_targets_mb:
        target_bytes = int(target_mb * 1024 * 1024)
        pages = args.size_axis_pages
        estimated_overhead = 4096 + pages * 512
        stream_bytes = max(64, (target_bytes - estimated_overhead) // pages)
        output = out_dir / f"synthetic-size-{target_mb}mb-{pages}p.pdf"
        metadata = generate_pdf(output, pages, stream_bytes)
        metadata["axis"] = "size"
        metadata["target_mb"] = target_mb
        manifest.append(metadata)

    for pages in args.page_targets:
        output = out_dir / f"synthetic-pages-{pages}p.pdf"
        metadata = generate_pdf(output, pages, args.page_axis_stream_bytes)
        metadata["axis"] = "pages"
        manifest.append(metadata)

    manifest_path = out_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(json.dumps({"manifest": str(manifest_path), "files": manifest}, indent=2))
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    gen = sub.add_parser("generate", help="generate one synthetic PDF")
    gen.add_argument("--output", required=True)
    gen.add_argument("--pages", type=int, required=True)
    gen.add_argument("--stream-bytes-per-page", type=int, required=True)
    gen.set_defaults(func=cmd_generate)

    ladder = sub.add_parser("ladder", help="generate size/page-count ladder PDFs")
    ladder.add_argument("--out-dir", default="large-file-profile/generated")
    ladder.add_argument("--size-targets-mb", nargs="+", type=int, default=[50, 200, 500, 1024])
    ladder.add_argument("--size-axis-pages", type=int, default=4)
    ladder.add_argument("--page-targets", nargs="+", type=int, default=[50, 1000, 5000])
    ladder.add_argument("--page-axis-stream-bytes", type=int, default=256)
    ladder.set_defaults(func=cmd_ladder)

    args = parser.parse_args()
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
