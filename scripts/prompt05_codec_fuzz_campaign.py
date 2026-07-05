#!/usr/bin/env python3
"""Prompt 05 codec fuzz campaign harness.

Smoke mode is intentionally bounded and stable-friendly: it compiles every fuzz
target and records whether cargo-fuzz/libFuzzer is available for actual run
campaigns. Local-long and release-long modes emit exact commands and artifact
layout for release engineers.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/prompt05-codec-closeout")
INVENTORY = OUT_DIR / "fuzz-target-inventory.json"
SMOKE_REPORT = OUT_DIR / "fuzz-smoke-report.json"
ARTIFACT_DIR = OUT_DIR / "fuzz-artifacts"
CORPUS_DIR = OUT_DIR / "fuzz-corpus"


LOGICAL_TARGETS = [
    ("filter_chain", "filters", "raw filter-chain selector bytes", "target/prompt05-codec-closeout/hostile-corpus/raw"),
    ("image_inventory", "parser_report", "PDF wrapper bytes that drive stream and image inventory", "target/prompt05-codec-closeout/hostile-corpus/pdf"),
    ("dct", "image_decoders", "image decoder selector plus DCT bytes", "target/prompt05-codec-closeout/hostile-corpus/raw"),
    ("jpx", "image_decoders", "image decoder selector plus JPX bytes", "target/prompt05-codec-closeout/hostile-corpus/raw"),
    ("jbig2", "image_decoders", "image decoder selector plus JBIG2 bytes", "target/prompt05-codec-closeout/hostile-corpus/raw"),
    ("ccitt", "image_decoders", "image decoder selector plus CCITT bytes", "target/prompt05-codec-closeout/hostile-corpus/raw"),
    ("predictor", "predictor", "predictor parameter bytes plus payload", "target/prompt05-codec-closeout/hostile-corpus/raw"),
    ("pdf_wrapper", "parser_report", "whole-PDF wrapper corpus", "target/prompt05-codec-closeout/hostile-corpus/pdf"),
    ("inline_image", "parser_report", "content stream with inline image ambiguity", "target/prompt05-codec-closeout/hostile-corpus/pdf"),
    ("worker_protocol", "filters", "worker-compatible filter payloads and caps", "target/prompt05-codec-closeout/hostile-corpus/raw"),
    ("scheduler_admission", "filters", "small and oversized decode jobs under scheduler caps", "target/prompt05-codec-closeout/hostile-corpus/raw"),
]


def cargo_fuzz_available() -> tuple[bool, str]:
    cargo = shutil.which("cargo")
    if not cargo:
        return False, "cargo not found on PATH"
    proc = subprocess.run(
        [cargo, "fuzz", "--help"],
        text=True,
        capture_output=True,
        timeout=20,
        check=False,
    )
    if proc.returncode == 0:
        return True, "cargo fuzz is available"
    return False, (proc.stderr or proc.stdout or "cargo fuzz returned non-zero").strip()


def inventory() -> dict[str, Any]:
    entries = []
    for logical, backing, shape, seed in LOGICAL_TARGETS:
        entries.append(
            {
                "logical_target": logical,
                "cargo_fuzz_bin": backing,
                "accepted_input_shape": shape,
                "seed_source": seed,
                "timeout_policy": "smoke uses -runs=1 when cargo-fuzz is available; local-long uses 4h; release-long uses 72h per target",
                "memory_policy": "run under Prompt 05 release harness memory cap; decode targets keep DecodeLimits caps enabled",
                "artifact_path": str((ARTIFACT_DIR / logical).as_posix()),
                "corpus_path": str((CORPUS_DIR / logical).as_posix()),
                "reproduction_command": f"cargo +nightly fuzz run {backing} -- target/prompt05-codec-closeout/fuzz-artifacts/{logical}/crash",
                "minimize_command": f"cargo +nightly fuzz tmin {backing} target/prompt05-codec-closeout/fuzz-artifacts/{logical}/crash",
                "promotion_path": f"target/prompt05-codec-closeout/regressions/{logical}/",
            }
        )
    report = {
        "schema_version": 1,
        "prompt": "combined_prompt05",
        "target_count": len(entries),
        "targets": entries,
        "dictionary": {
            "path": "target/prompt05-codec-closeout/fuzz-dictionary.txt",
            "tokens": [
                "/Filter",
                "/DecodeParms",
                "FlateDecode",
                "DCTDecode",
                "JPXDecode",
                "JBIG2Decode",
                "CCITTFaxDecode",
                "stream",
                "endstream",
                "BI",
                "ID",
                "EI",
            ],
        },
    }
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    (OUT_DIR / "fuzz-dictionary.txt").write_text(
        "\n".join(f'"{token}"' for token in report["dictionary"]["tokens"]) + "\n",
        encoding="utf-8",
    )
    INVENTORY.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def prepare_seed_corpus(targets: list[dict[str, Any]]) -> None:
    for target in targets:
        corpus_path = Path(target["corpus_path"])
        artifact_path = Path(target["artifact_path"])
        corpus_path.mkdir(parents=True, exist_ok=True)
        artifact_path.mkdir(parents=True, exist_ok=True)
        seed_source = Path(target["seed_source"])
        if not seed_source.exists():
            continue
        for seed in seed_source.iterdir():
            if seed.is_file():
                destination = corpus_path / seed.name
                if not destination.exists():
                    shutil.copyfile(seed, destination)


def run_command(cmd: list[str], timeout_sec: int) -> dict[str, Any]:
    started = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            text=True,
            capture_output=True,
            timeout=timeout_sec,
            check=False,
        )
        return {
            "command": cmd,
            "status": "pass" if proc.returncode == 0 else "fail",
            "exit_code": proc.returncode,
            "elapsed_ms": int((time.perf_counter() - started) * 1000),
            "stdout_tail": proc.stdout[-2000:],
            "stderr_tail": proc.stderr[-2000:],
        }
    except subprocess.TimeoutExpired as exc:
        return {
            "command": cmd,
            "status": "fail",
            "exit_code": None,
            "elapsed_ms": timeout_sec * 1000,
            "classification": "timeout",
            "stdout_tail": (exc.stdout or "")[-2000:],
            "stderr_tail": (exc.stderr or "")[-2000:],
        }


def campaign_commands(mode: str, targets: list[dict[str, Any]]) -> list[dict[str, Any]]:
    if mode == "smoke":
        runs = "1"
        max_total_time = "30"
    elif mode == "local-long":
        runs = "0"
        max_total_time = str(4 * 60 * 60)
    else:
        runs = "0"
        max_total_time = str(72 * 60 * 60)

    commands = []
    seen = set()
    for target in targets:
        backing = target["cargo_fuzz_bin"]
        if backing in seen:
            continue
        seen.add(backing)
        commands.append(
            {
                "target": backing,
                "command": [
                    "cargo",
                    "+nightly",
                    "fuzz",
                    "run",
                    backing,
                    str(target["corpus_path"]),
                    "--",
                    f"-runs={runs}",
                    f"-max_total_time={max_total_time}",
                    "-timeout=10",
                    f"-artifact_prefix={target['artifact_path']}/",
                    "-dict=target/prompt05-codec-closeout/fuzz-dictionary.txt",
                ],
            }
        )
    return commands


def smoke(mode: str, dry_run: bool, timeout_sec: int) -> dict[str, Any]:
    inv = inventory()
    prepare_seed_corpus(inv["targets"])
    compile_result = run_command(
        ["cargo", "check", "--manifest-path", "fuzz/Cargo.toml", "--bins", "--jobs", "1"],
        timeout_sec,
    )
    available, reason = cargo_fuzz_available()
    commands = campaign_commands(mode, inv["targets"])
    if mode == "smoke" and available and not dry_run:
        fuzz_runs = [run_command(item["command"], timeout_sec) for item in commands[:3]]
    else:
        fuzz_runs = [
            {
                "command": item["command"],
                "status": "skipped" if not available or dry_run else "planned",
                "reason": "dry run requested" if dry_run else reason,
            }
            for item in commands
        ]
    report = {
        "schema_version": 1,
        "prompt": "combined_prompt05",
        "mode": mode,
        "dry_run": dry_run,
        "target_inventory": str(INVENTORY.as_posix()),
        "compile_check": compile_result,
        "cargo_fuzz_available": available,
        "cargo_fuzz_reason": reason,
        "campaign_commands": commands,
        "fuzz_runs": fuzz_runs,
        "crash_artifact_layout": str(ARTIFACT_DIR.as_posix()),
        "minimization_workflow": [
            "copy crash artifact into target/prompt05-codec-closeout/fuzz-artifacts/<logical_target>/",
            "run the recorded cargo +nightly fuzz tmin command",
            "rerun the recorded reproduction command",
            "promote minimized bytes into target/prompt05-codec-closeout/regressions/<logical_target>/ and add a manifest row",
        ],
    }
    SMOKE_REPORT.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=["smoke", "local-long", "release-long"], default="smoke")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--timeout-sec", type=int, default=180)
    args = parser.parse_args()
    report = smoke(args.mode, args.dry_run, args.timeout_sec)
    print(
        json.dumps(
            {
                "inventory": str(INVENTORY),
                "smoke_report": str(SMOKE_REPORT),
                "compile_status": report["compile_check"]["status"],
                "cargo_fuzz_available": report["cargo_fuzz_available"],
            }
        )
    )
    return 0 if report["compile_check"]["status"] == "pass" else 1


if __name__ == "__main__":
    sys.exit(main())
