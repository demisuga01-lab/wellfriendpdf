#!/usr/bin/env python3
"""Prompt 06B target-local PDFium renderer wrapper.

This script gives the audit harness a deterministic PDFium-backed renderer even
on hosts that do not provide the standalone pdfium_test binary. It supports the
small pdfium_test-compatible argument subset used by the Prompt 06 reference
adapter and a direct JSON/version mode for the Prompt 06B bootstrap manifest.
"""

from __future__ import annotations

import argparse
import importlib.metadata
import json
import sys
from pathlib import Path
from typing import Any


def pdfium_versions() -> dict[str, Any]:
    import pypdfium2 as pdfium  # type: ignore
    import pypdfium2.version as version  # type: ignore

    try:
        package_version = importlib.metadata.version("pypdfium2")
    except importlib.metadata.PackageNotFoundError:
        package_version = getattr(version, "V_PYPDFIUM2", "unknown")
    return {
        "package": "pypdfium2",
        "package_version": package_version,
        "pdfium_version": getattr(version, "V_LIBPDFIUM_FULL", "unknown"),
        "pdfium_build": getattr(version, "V_BUILDNAME", "unknown"),
        "module": str(Path(pdfium.__file__).resolve()),
        "python": sys.executable,
    }


def render_png(input_pdf: Path, output_png: Path, page: int, dpi: int) -> None:
    import pypdfium2 as pdfium  # type: ignore

    output_png.parent.mkdir(parents=True, exist_ok=True)
    document = pdfium.PdfDocument(str(input_pdf))
    try:
        pdf_page = document[page - 1]
        try:
            bitmap = pdf_page.render(
                scale=dpi / 72.0,
                fill_color=(255, 255, 255, 255),
                rev_byteorder=True,
                draw_annots=True,
                may_draw_forms=True,
            )
        except TypeError:
            bitmap = pdf_page.render(
                scale=dpi / 72.0,
                fill_color=(255, 255, 255, 255),
                rev_byteorder=True,
            )
        bitmap.to_pil().convert("RGB").save(output_png)
        try:
            bitmap.close()
        except AttributeError:
            pass
        try:
            pdf_page.close()
        except AttributeError:
            pass
    finally:
        document.close()


def parse_pdfium_test_args(argv: list[str]) -> argparse.Namespace:
    output: str | None = None
    first_page = 1
    last_page = 1
    dpi = 72
    positional: list[str] = []
    idx = 0
    while idx < len(argv):
        arg = argv[idx]
        if arg == "--png":
            idx += 1
            continue
        if arg.startswith("--output="):
            output = arg.split("=", 1)[1]
            idx += 1
            continue
        if arg == "--output" and idx + 1 < len(argv):
            output = argv[idx + 1]
            idx += 2
            continue
        if arg.startswith("--first-page="):
            first_page = int(arg.split("=", 1)[1])
            idx += 1
            continue
        if arg.startswith("--last-page="):
            last_page = int(arg.split("=", 1)[1])
            idx += 1
            continue
        if arg.startswith("--dpi="):
            dpi = int(arg.split("=", 1)[1])
            idx += 1
            continue
        if arg.startswith("-"):
            idx += 1
            continue
        positional.append(arg)
        idx += 1
    if not output:
        raise SystemExit("pdfium wrapper requires --output=<png>")
    if not positional:
        raise SystemExit("pdfium wrapper requires an input PDF path")
    if last_page != first_page:
        raise SystemExit("pdfium wrapper supports one page per invocation")
    return argparse.Namespace(
        input=Path(positional[-1]),
        output=Path(output),
        page=first_page,
        dpi=dpi,
    )


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if "--version-json" in argv:
        print(json.dumps(pdfium_versions(), indent=2, sort_keys=True))
        return 0
    if "--version" in argv:
        versions = pdfium_versions()
        print(
            "pdfium_test wrapper "
            f"pypdfium2 {versions['package_version']} "
            f"pdfium {versions['pdfium_version']} "
            f"build {versions['pdfium_build']}"
        )
        return 0

    if "--input" in argv:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("--input", type=Path, required=True)
        parser.add_argument("--output", type=Path, required=True)
        parser.add_argument("--page", type=int, default=1)
        parser.add_argument("--dpi", type=int, default=72)
        args = parser.parse_args(argv)
    else:
        args = parse_pdfium_test_args(argv)
    render_png(args.input, args.output, args.page, args.dpi)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
