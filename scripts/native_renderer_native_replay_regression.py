#!/usr/bin/env python3
"""Assert Native Renderer native replay counters on focused fixtures."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/native_renderer-renderer-native-replay")
REPORT = OUT_DIR / "native-replay-regression.json"


def write_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run_command(cmd: list[str], timeout: int = 120) -> dict[str, Any]:
    started = time.time()
    try:
        proc = subprocess.run(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=timeout,
            check=False,
        )
        return {
            "command": cmd,
            "exit_status": proc.returncode,
            "stdout": proc.stdout[-4000:],
            "stderr": proc.stderr[-4000:],
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": False,
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "command": cmd,
            "exit_status": None,
            "stdout": (exc.stdout or "")[-4000:] if isinstance(exc.stdout, str) else "",
            "stderr": (exc.stderr or "")[-4000:] if isinstance(exc.stderr, str) else "",
            "elapsed_ms": int((time.time() - started) * 1000),
            "timed_out": True,
        }


def wellfriendpdf_base_command(args: argparse.Namespace) -> list[str]:
    if args.wellfriendpdf_bin:
        return [str(Path(args.wellfriendpdf_bin))]
    suffix = ".exe" if os.name == "nt" else ""
    for candidate in [Path("target/debug") / f"wellfriendpdf{suffix}", Path("target/release") / f"wellfriendpdf{suffix}"]:
        if candidate.exists():
            return [str(candidate)]
    return ["cargo", "run", "-p", "wellfriendpdf-cli", "--quiet", "--"]


def write_inline_image_fixture(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    stream = b"q 40 0 0 40 20 20 cm BI /W 1 /H 1 /CS /RGB /BPC 8 ID \x00\xff\x00 EI Q\n"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>",
        b"<< /Length " + str(len(stream)).encode("ascii") + b" >>\nstream\n" + stream + b"endstream",
    ]
    data = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for idx, obj in enumerate(objects, start=1):
        offsets.append(len(data))
        data.extend(f"{idx} 0 obj\n".encode("ascii"))
        data.extend(obj)
        data.extend(b"\nendobj\n")
    xref = len(data)
    data.extend(f"xref\n0 {len(objects) + 1}\n".encode("ascii"))
    data.extend(b"0000000000 65535 f \n")
    for off in offsets[1:]:
        data.extend(f"{off:010d} 00000 n \n".encode("ascii"))
    data.extend(
        (
            f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n"
            f"startxref\n{xref}\n%%EOF\n"
        ).encode("ascii")
    )
    path.write_bytes(data)


def fixture_cases() -> list[dict[str, Any]]:
    inline = OUT_DIR / "generated-fixtures" / "native-regression-inline-image.pdf"
    write_inline_image_fixture(inline)
    return [
        {
            "id": "native_text",
            "path": "tests/corpus/pdfs/generated/generated_basic_text.pdf",
            "required_counter": "native_text_ops",
            "min_count": 1,
            "max_compatibility_runs": 0,
        },
        {
            "id": "native_image_xobject",
            "path": "tests/corpus/pdfs/generated/generated_image_only.pdf",
            "required_counter": "native_image_xobjects",
            "min_count": 1,
            "max_compatibility_runs": 0,
        },
        {
            "id": "native_inline_image",
            "path": str(inline),
            "required_counter": "native_inline_images",
            "min_count": 1,
            "max_compatibility_runs": 0,
        },
        {
            "id": "native_form_xobject",
            "path": "renderer-benchmark/corpus/synthetic/synthetic_form_000.pdf",
            "required_counter": "native_form_xobjects",
            "min_count": 1,
            "max_compatibility_runs": 0,
        },
    ]


def run_case(base: list[str], case: dict[str, Any]) -> dict[str, Any]:
    output = OUT_DIR / "regression" / f"{case['id']}.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        *base,
        "render-compare",
        case["path"],
        "--pages",
        "1",
        "--dpi",
        "72",
        "--output",
        str(output),
        "--pretty",
    ]
    result = run_command(cmd)
    report: dict[str, Any] | None = None
    if output.exists():
        try:
            report = json.loads(output.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            report = None
    totals = report.get("totals", {}) if report else {}
    counter = int(totals.get(case["required_counter"], 0) or 0)
    compatibility = int(totals.get("compatibility_runs", 0) or 0)
    passed = (
        result["exit_status"] == 0
        and report is not None
        and counter >= int(case["min_count"])
        and compatibility <= int(case["max_compatibility_runs"])
    )
    return {
        "id": case["id"],
        "path": case["path"],
        "required_counter": case["required_counter"],
        "observed_counter": counter,
        "observed_compatibility_runs": compatibility,
        "passed": passed,
        "report_artifact": str(output).replace("\\", "/"),
        "command": result,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wellfriendpdf-bin", help="Path to wellfriendpdf executable; defaults to target binary or cargo run")
    args = parser.parse_args()
    base = wellfriendpdf_base_command(args)
    cases = fixture_cases()
    results = [run_case(base, case) for case in cases]
    report = {
        "schema_version": 1,
        "kind": "native_renderer_native_replay_regression",
        "status": "passed" if all(item["passed"] for item in results) else "failed",
        "cases": results,
    }
    write_json(REPORT, report)
    print(json.dumps({"status": report["status"], "artifact": str(REPORT)}, indent=2))
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
