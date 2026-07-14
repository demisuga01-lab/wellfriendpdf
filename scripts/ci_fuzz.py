#!/usr/bin/env python3
"""Private-CI cargo-fuzz runner.

This script keeps the GitHub Actions workflow small and gives developers the
same commands locally. It deliberately runs the out-of-workspace `fuzz/` crate
and does not affect normal stable builds.
"""

from __future__ import annotations

import argparse
from collections import deque
import json
import os
import subprocess
import sys
import time
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
FUZZ = REPO / "fuzz"
DEFAULT_TARGETS = [
    "parse_pdf",
    "filters",
    "predictor",
    "content_tokenizer",
    "image_decoders",
    "fonts",
    "cmap",
    "crypto",
    "functions",
    "writer",
    "document_rewrite",
    "linearize",
    "pdfa",
    "editing",
    "signature_validation",
    "structured_pdf",
]

DEFAULT_KILL_GRACE_SECONDS = 5
DEFAULT_BUILD_TIMEOUT_SECONDS = 600


def github_escape(value: str) -> str:
    return value.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


def github_error(title: str, message: str) -> None:
    if os.environ.get("GITHUB_ACTIONS") == "true":
        print(
            f"::error title={github_escape(title)}::{github_escape(message)}",
            flush=True,
        )


def count_files(path: Path) -> int:
    if not path.exists():
        return 0
    return sum(1 for item in path.rglob("*") if item.is_file())


def process_tree_snapshot(pid: int) -> list[dict[str, str | int]]:
    if os.name != "nt":
        return [{"pid": pid, "note": "process tree snapshot is Windows-only in this harness"}]
    ps = [
        "powershell",
        "-NoProfile",
        "-Command",
        (
            "$pid = "
            + str(pid)
            + "; "
            + "$all = Get-CimInstance Win32_Process; "
            + "$seen = @{}; $queue = New-Object System.Collections.Queue; "
            + "$queue.Enqueue($pid); "
            + "while ($queue.Count -gt 0) { "
            + "$current = [int]$queue.Dequeue(); "
            + "if ($seen.ContainsKey($current)) { continue }; "
            + "$seen[$current] = $true; "
            + "$children = $all | Where-Object { $_.ParentProcessId -eq $current }; "
            + "foreach ($child in $children) { $queue.Enqueue([int]$child.ProcessId) } "
            + "}; "
            + "$all | Where-Object { $seen.ContainsKey([int]$_.ProcessId) } | "
            + "Select-Object ProcessId,Name,CommandLine | ConvertTo-Json -Compress"
        ),
    ]
    try:
        proc = subprocess.run(ps, capture_output=True, text=True, timeout=10)
    except Exception as exc:  # pragma: no cover - diagnostics only
        return [{"pid": pid, "error": str(exc)}]
    if proc.returncode != 0 or not proc.stdout.strip():
        return [{"pid": pid, "error": (proc.stderr or "empty process snapshot").strip()}]
    try:
        parsed = json.loads(proc.stdout)
    except json.JSONDecodeError:
        return [{"pid": pid, "raw": proc.stdout.strip()[:1000]}]
    if isinstance(parsed, dict):
        parsed = [parsed]
    return [
        {
            "pid": int(item.get("ProcessId", 0)),
            "name": str(item.get("Name", "")),
            "command_line": str(item.get("CommandLine", "")),
        }
        for item in parsed
    ]


def terminate_process_tree(process: subprocess.Popen[str], grace_seconds: int) -> dict[str, object]:
    if process.poll() is not None:
        return {"attempted": False, "reason": "process already exited"}
    before = process_tree_snapshot(process.pid)
    try:
        process.terminate()
        process.wait(timeout=grace_seconds)
    except subprocess.TimeoutExpired:
        if os.name == "nt":
            subprocess.run(
                ["taskkill", "/PID", str(process.pid), "/T", "/F"],
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                timeout=max(grace_seconds, 1),
            )
        else:
            process.kill()
        try:
            process.wait(timeout=grace_seconds)
        except subprocess.TimeoutExpired:
            pass
    return {
        "attempted": True,
        "pid": process.pid,
        "process_tree_before_termination": before,
        "returncode_after_termination": process.poll(),
    }


def run(
    cmd: list[str],
    *,
    cwd: Path = FUZZ,
    timeout_seconds: int,
    kill_grace_seconds: int,
    target: str,
    phase: str,
    target_index: int,
    target_total: int,
    configured_duration_seconds: int | None,
    max_len: int | None,
) -> dict[str, object]:
    start = time.monotonic()
    started_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    print(
        (
            f"== fuzz progress target={target} index={target_index}/{target_total} "
            f"phase={phase} start={started_at} duration={configured_duration_seconds} "
            f"max_len={max_len} timeout={timeout_seconds}s =="
        ),
        flush=True,
    )
    print("+", " ".join(cmd), flush=True)
    process = subprocess.Popen(
        cmd,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    print(f"== fuzz process target={target} phase={phase} pid={process.pid} ==", flush=True)
    timed_out = False
    output = ""
    termination: dict[str, object] = {"attempted": False}
    try:
        output, _ = process.communicate(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        termination = terminate_process_tree(process, kill_grace_seconds)
        more, _ = process.communicate(timeout=max(kill_grace_seconds, 1))
        output = (output or "") + (more or "")
    elapsed = time.monotonic() - start
    tail = deque(output.splitlines(), maxlen=120)
    if output:
        print(output, end="" if output.endswith("\n") else "\n", flush=True)
    returncode = process.returncode
    status = "timeout" if timed_out else ("passed" if returncode == 0 else "failed")
    print(
        (
            f"== fuzz complete target={target} index={target_index}/{target_total} "
            f"phase={phase} status={status} exit={returncode} elapsed={elapsed:.3f}s =="
        ),
        flush=True,
    )
    if returncode != 0 or timed_out:
        tail_text = "\n".join(tail)[-3500:]
        github_error(
            "cargo-fuzz command failed",
            f"command: {' '.join(cmd)}\nexit: {returncode}\ntimeout: {timed_out}\n{tail_text}",
        )
    return {
        "target": target,
        "phase": phase,
        "target_index": target_index,
        "target_total": target_total,
        "command": cmd,
        "cwd": str(cwd),
        "pid": process.pid,
        "started_at": started_at,
        "elapsed_seconds": round(elapsed, 3),
        "configured_duration_seconds": configured_duration_seconds,
        "max_len": max_len,
        "timeout_seconds": timeout_seconds,
        "kill_grace_seconds": kill_grace_seconds,
        "exit_code": returncode,
        "timed_out": timed_out,
        "status": status,
        "tail": list(tail),
        "termination": termination,
    }


def has_seed(corpus_dir: Path) -> bool:
    return corpus_dir.exists() and any(path.is_file() for path in corpus_dir.rglob("*"))


def artifact_prefix(target: str) -> str:
    path = REPO / "target" / "ci-fuzz-artifacts" / target
    path.mkdir(parents=True, exist_ok=True)
    return f"{path.as_posix()}/"


def parse_targets(raw: str) -> list[str]:
    if raw == "all":
        return DEFAULT_TARGETS
    targets = [item.strip() for item in raw.split(",") if item.strip()]
    unknown = sorted(set(targets) - set(DEFAULT_TARGETS))
    if unknown:
        raise SystemExit(f"unknown fuzz target(s): {', '.join(unknown)}")
    return targets


def fuzz_sanitizer_args(sanitizer: str | None) -> list[str]:
    if not sanitizer:
        return []
    return ["--sanitizer", sanitizer]


def build_target(
    target: str,
    sanitizer: str | None,
    *,
    timeout_seconds: int,
    kill_grace_seconds: int,
    target_index: int,
    target_total: int,
) -> dict[str, object]:
    return run(
        ["cargo", "+nightly", "fuzz", "build", *fuzz_sanitizer_args(sanitizer), target],
        timeout_seconds=timeout_seconds,
        kill_grace_seconds=kill_grace_seconds,
        target=target,
        phase="build",
        target_index=target_index,
        target_total=target_total,
        configured_duration_seconds=None,
        max_len=None,
    )


def replay_regressions(
    target: str,
    sanitizer: str | None,
    *,
    timeout_seconds: int,
    kill_grace_seconds: int,
    target_index: int,
    target_total: int,
) -> dict[str, object]:
    corpus = FUZZ / "corpus" / target
    if not has_seed(corpus):
        print(f"skip {target}: no committed regression/seed corpus", flush=True)
        return {
            "target": target,
            "phase": "regression",
            "target_index": target_index,
            "target_total": target_total,
            "status": "skipped_no_corpus",
            "corpus_count": 0,
            "artifact_count": count_files(REPO / "target" / "ci-fuzz-artifacts" / target),
        }
    return run(
        [
            "cargo",
            "+nightly",
            "fuzz",
            "run",
            *fuzz_sanitizer_args(sanitizer),
            target,
            f"corpus/{target}",
            "--",
            "-runs=0",
            f"-artifact_prefix={artifact_prefix(target)}",
        ],
        timeout_seconds=timeout_seconds,
        kill_grace_seconds=kill_grace_seconds,
        target=target,
        phase="regression",
        target_index=target_index,
        target_total=target_total,
        configured_duration_seconds=None,
        max_len=None,
    )


def timed_fuzz(
    target: str,
    seconds: int,
    max_len: int,
    sanitizer: str | None,
    *,
    timeout_seconds: int,
    kill_grace_seconds: int,
    target_index: int,
    target_total: int,
) -> dict[str, object]:
    (FUZZ / "corpus" / target).mkdir(parents=True, exist_ok=True)
    return run(
        [
            "cargo",
            "+nightly",
            "fuzz",
            "run",
            *fuzz_sanitizer_args(sanitizer),
            target,
            "--",
            f"-max_total_time={seconds}",
            f"-max_len={max_len}",
            f"-artifact_prefix={artifact_prefix(target)}",
        ],
        timeout_seconds=timeout_seconds,
        kill_grace_seconds=kill_grace_seconds,
        target=target,
        phase="smoke",
        target_index=target_index,
        target_total=target_total,
        configured_duration_seconds=seconds,
        max_len=max_len,
    )


def write_report(path: Path | None, report: dict[str, object]) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True), encoding="utf-8")


def remaining_timeout(deadline: float, requested: int) -> int:
    remaining = int(deadline - time.monotonic())
    if remaining <= 0:
        return 1
    return max(1, min(requested, remaining))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--targets", default="all", help="'all' or comma-separated target names")
    parser.add_argument(
        "--mode",
        choices=["build", "regression", "smoke", "deep"],
        required=True,
    )
    parser.add_argument("--seconds", type=int, default=45)
    parser.add_argument("--max-len", type=int, default=65536)
    parser.add_argument("--sanitizer", help="Optional cargo-fuzz sanitizer, such as address")
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--print-targets", action="store_true")
    parser.add_argument("--build-timeout", type=int, default=DEFAULT_BUILD_TIMEOUT_SECONDS)
    parser.add_argument("--per-target-timeout", type=int)
    parser.add_argument("--global-timeout", type=int)
    parser.add_argument("--kill-grace", type=int, default=DEFAULT_KILL_GRACE_SECONDS)
    parser.add_argument("--json-report", type=Path)
    args = parser.parse_args()

    targets = parse_targets(args.targets)
    if args.print_targets:
        print("\n".join(targets))
        return

    per_target_timeout = args.per_target_timeout or max(args.seconds + 30, 60)
    global_timeout = args.global_timeout or (
        len(targets) * ((0 if args.no_build else args.build_timeout) + per_target_timeout + 15)
    )
    campaign_started = time.monotonic()
    campaign_deadline = campaign_started + global_timeout
    report: dict[str, object] = {
        "schema": "ci_fuzz_report_v2",
        "started_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "targets": targets,
        "mode": args.mode,
        "seconds": args.seconds,
        "max_len": args.max_len,
        "sanitizer": args.sanitizer,
        "build_timeout_seconds": args.build_timeout,
        "per_target_timeout_seconds": per_target_timeout,
        "global_timeout_seconds": global_timeout,
        "kill_grace_seconds": args.kill_grace,
        "results": [],
    }
    exit_code = 0
    total = len(targets)
    for index, target in enumerate(targets, start=1):
        elapsed_campaign = time.monotonic() - campaign_started
        if elapsed_campaign > global_timeout:
            exit_code = 124
            report["results"].append(
                {
                    "target": target,
                    "target_index": index,
                    "target_total": total,
                    "status": "global_timeout_before_target",
                    "elapsed_campaign_seconds": round(elapsed_campaign, 3),
                }
            )
            break
        print(f"== {target} ({args.mode}) [{index}/{total}] ==", flush=True)
        target_results: list[dict[str, object]] = []
        artifact_count_before = count_files(REPO / "target" / "ci-fuzz-artifacts" / target)
        corpus_count_before = count_files(FUZZ / "corpus" / target)
        if not args.no_build:
            result = build_target(
                target,
                args.sanitizer,
                timeout_seconds=remaining_timeout(campaign_deadline, args.build_timeout),
                kill_grace_seconds=args.kill_grace,
                target_index=index,
                target_total=total,
            )
            target_results.append(result)
            if result["status"] != "passed":
                exit_code = int(result.get("exit_code") or 124)
        if exit_code == 0:
            if args.mode == "build":
                pass
            elif args.mode == "regression":
                target_results.append(
                    replay_regressions(
                        target,
                        args.sanitizer,
                        timeout_seconds=remaining_timeout(campaign_deadline, per_target_timeout),
                        kill_grace_seconds=args.kill_grace,
                        target_index=index,
                        target_total=total,
                    )
                )
            else:
                target_results.append(
                    timed_fuzz(
                        target,
                        args.seconds,
                        args.max_len,
                        args.sanitizer,
                        timeout_seconds=remaining_timeout(campaign_deadline, per_target_timeout),
                        kill_grace_seconds=args.kill_grace,
                        target_index=index,
                        target_total=total,
                    )
                )
        target_status = "passed"
        for result in target_results:
            if result.get("status") not in {"passed", "skipped_no_corpus"}:
                target_status = str(result.get("status"))
                exit_code = int(result.get("exit_code") or 124)
                break
        report["results"].append(
            {
                "target": target,
                "target_index": index,
                "target_total": total,
                "status": target_status,
                "corpus_count_before": corpus_count_before,
                "corpus_count_after": count_files(FUZZ / "corpus" / target),
                "artifact_count_before": artifact_count_before,
                "artifact_count_after": count_files(REPO / "target" / "ci-fuzz-artifacts" / target),
                "phases": target_results,
            }
        )
        if target_status not in {"passed", "skipped_no_corpus"}:
            break
    report["completed_at"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    report["elapsed_seconds"] = round(time.monotonic() - campaign_started, 3)
    report["status"] = "passed" if exit_code == 0 else "failed"
    write_report(args.json_report, report)
    if exit_code != 0:
        raise SystemExit(exit_code)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
