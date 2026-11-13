"""Installed-wheel parity check against the Wellfriend CLI.

This is intentionally a standalone script, not a pytest default test: it runs the
deterministic first-N corpus loop and shells out to the release CLI for each file.

Text is compared per extraction profile against `extract-text --raw --profile P`,
the CLI path that runs the same engine call as `Document.extract_text(profile=P)`.
The CLI's default output is Poppler-style reading order (since "Make layout-aware
reading order the default for extract-text"); its parity pair is
`Document.extract_text_reading_order()`, compared against plain `extract-text`.
`--raw` with the fast-text profile is true content-stream order (pdftotext -raw),
which is not what the bindings' `extract_text()` returns, so it is not checked.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

import wellfriendpdf


ROOT = Path(__file__).resolve().parents[3]


def norm_text(value: str) -> str:
    return value.replace("\r\n", "\n").replace("\r", "\n").strip()


def run_cli(wellfriendpdf_bin: Path, args: list[str]) -> str:
    completed = subprocess.run(
        [str(wellfriendpdf_bin), *args],
        cwd=ROOT,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=60,
        check=True,
    )
    return completed.stdout


def cli_tables(raw: str) -> list[dict]:
    data = json.loads(raw)
    out = []
    for page in data.get("pages", []):
        for table in page.get("tables", []):
            out.append({"page": page["page"], "table": table})
    return out


# Profiles whose `--raw` CLI output is the engine call behind Document.extract_text(profile=...).
TEXT_PROFILES = ("layout-faithful", "tables-focused", "rag-chunks")


def check_file(wellfriendpdf_bin: Path, pdf: Path) -> tuple[bool, str]:
    try:
        doc = wellfriendpdf.open(pdf)
    except wellfriendpdf.WellfriendError as error:
        # A file the engine cannot open is in parity when the CLI refuses it
        # too (corpus files include deliberately broken PDFs).
        try:
            run_cli(wellfriendpdf_bin, ["extract-text", str(pdf)])
        except subprocess.CalledProcessError:
            return True, f"{pdf.name} (refused by both: {error})"
        return False, f"{pdf.name}: binding cannot open a file the CLI reads ({error})"

    for profile in TEXT_PROFILES:
        py_text = norm_text(doc.extract_text(profile=profile))
        cli_text = norm_text(
            run_cli(wellfriendpdf_bin, ["extract-text", str(pdf), "--raw", "--profile", profile])
        )
        if py_text != cli_text:
            return False, f"text mismatch for {pdf.name} (profile {profile})"

    # Reading order: the CLI default prints each page followed by a newline,
    # exactly what extract_text_reading_order() returns for the whole document.
    py_reading = norm_text(doc.extract_text_reading_order())
    cli_reading = norm_text(run_cli(wellfriendpdf_bin, ["extract-text", str(pdf)]))
    if py_reading != cli_reading:
        return False, f"reading-order text mismatch for {pdf.name}"

    py_tables = doc.extract_tables()
    cli_table_payload = cli_tables(run_cli(wellfriendpdf_bin, ["extract-tables", str(pdf), "--format", "json"]))
    if py_tables != cli_table_payload:
        return False, f"table mismatch for {pdf.name}"

    py_fields = doc.extract_fields()
    cli_fields = json.loads(run_cli(wellfriendpdf_bin, ["extract-fields", str(pdf), "--format", "json"]))
    if py_fields != cli_fields:
        return False, f"field mismatch for {pdf.name}"

    return True, pdf.name


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path, default=ROOT / "test_corpus")
    parser.add_argument("--wellfriendpdf-bin", type=Path, default=Path(os.environ.get("WELLFRIENDPDF_BIN", ROOT / "target" / "release" / "wellfriendpdf.exe")))
    parser.add_argument("--limit", type=int, default=200)
    args = parser.parse_args()

    pdfs = sorted(args.corpus.glob("*.pdf"))[: args.limit]
    if not pdfs:
        print(f"no PDFs found under {args.corpus}", file=sys.stderr)
        return 2

    for index, pdf in enumerate(pdfs, start=1):
        ok, message = check_file(args.wellfriendpdf_bin, pdf)
        if not ok:
            print(message, file=sys.stderr)
            return 1
        if index % 25 == 0 or index == len(pdfs):
            print(f"{index}/{len(pdfs)} parity ok")

    print(f"Python binding parity ok: {len(pdfs)} files")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
