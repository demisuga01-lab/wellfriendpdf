#!/usr/bin/env python3
"""Release fuzz runner used by Prompt 27 and later hardening campaigns.

Runs cargo-fuzz one target at a time with explicit memory/time policy and writes
machine-readable evidence. The defaults preserve serial Prompt 25B-style fuzzing
posture: dev fuzz builds, high codegen units, no trace compares, bounded input,
and explicit process-tree RSS caps. Prompt 27+ VPS campaigns use a user-approved
16 GiB per-process cap while keeping the overall Wellfriend test allocation
under 32 GiB.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import time
from pathlib import Path

from release_fuzz_matrix import HIGH_PRIORITY_LONG, RELEASE_CRITICAL, build_payload


SCHEMA_VERSION = "wellfriendpdf.release-fuzz-runner.v2"
PARSER_GROUP = [
    "parse_pdf",
    "content_tokenizer",
    "cos_object",
    "parser_report",
    "xref_stream",
    "object_stream",
    "document_rewrite",
    "linearize",
    "structured_pdf",
    "decode_scanner",
    "crypto",
]

GROUPS = {
    "parser": PARSER_GROUP,
    "release-critical": sorted(RELEASE_CRITICAL),
    "standards": [
        "pdfa",
        "pdfua_structure",
        "pdfx_prepress",
        "cross_profile_standards",
        "standards_xmp_identifier",
    ],
    "signatures": [
        "signature_validation",
        "signature_evidence",
        "timestamp_token",
        "signature_preserving_edit_plan",
        "incremental_signing_plan",
        "cms_insertion_boundary",
        "external_signer_response",
        "mdp_permission_parser",
        "post_signature_modification",
    ],
}


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def resolve_targets(repo: Path, raw_targets: str | None, groups: list[str]) -> list[str]:
    inventory = build_payload(repo)
    known = {row["name"] for row in inventory["targets"]}
    selected: list[str] = []
    for group in groups:
        if group not in GROUPS:
            raise SystemExit(f"unknown group: {group}")
        selected.extend(GROUPS[group])
    if raw_targets:
        if raw_targets == "all":
            selected.extend(sorted(known))
        else:
            selected.extend(item.strip() for item in raw_targets.split(",") if item.strip())
    if not selected:
        selected.extend(PARSER_GROUP)
    unique = []
    for target in selected:
        if target not in known:
            raise SystemExit(f"unknown fuzz target: {target}")
        if target not in unique:
            unique.append(target)
    return unique


def read_rss_kib(pid: int) -> int:
    try:
        for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8", errors="replace").splitlines():
            if line.startswith("VmRSS:"):
                parts = line.split()
                return int(parts[1]) if len(parts) >= 2 else 0
    except (FileNotFoundError, ProcessLookupError, PermissionError, ValueError):
        return 0
    return 0


def proc_parent_map() -> dict[int, int]:
    parents: dict[int, int] = {}
    proc_root = Path("/proc")
    if os.name == "nt" or not proc_root.exists():
        return parents
    for item in proc_root.iterdir():
        if not item.name.isdigit():
            continue
        try:
            stat = (item / "stat").read_text(encoding="utf-8", errors="replace")
            after_comm = stat.rsplit(")", 1)[1].strip().split()
            if len(after_comm) >= 2:
                parents[int(item.name)] = int(after_comm[1])
        except (FileNotFoundError, ProcessLookupError, PermissionError, ValueError, IndexError):
            continue
    return parents


def process_tree_pids(root_pid: int) -> list[int]:
    parents = proc_parent_map()
    children: dict[int, list[int]] = {}
    for pid, ppid in parents.items():
        children.setdefault(ppid, []).append(pid)
    seen: set[int] = set()
    queue = [root_pid]
    while queue:
        pid = queue.pop(0)
        if pid in seen:
            continue
        seen.add(pid)
        queue.extend(children.get(pid, []))
    return sorted(seen)


def process_tree_rss_kib(root_pid: int) -> int:
    return sum(read_rss_kib(pid) for pid in process_tree_pids(root_pid))


def terminate_process_tree(proc: subprocess.Popen[object]) -> None:
    if proc.poll() is not None:
        return
    try:
        if os.name != "nt":
            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
        else:
            proc.terminate()
    except (ProcessLookupError, PermissionError):
        pass
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline and proc.poll() is None:
        time.sleep(0.1)
    if proc.poll() is None:
        try:
            if os.name != "nt":
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            else:
                proc.kill()
        except (ProcessLookupError, PermissionError):
            pass


def cargo_fuzz_options() -> list[str]:
    return ["-D", "--codegen-units", "256", "--no-trace-compares", "--disable-branch-folding", "false"]


def collect_artifacts(repo: Path, target: str, artifact_root: Path) -> list[dict[str, object]]:
    candidates = [repo / "fuzz" / "artifacts" / target, artifact_root / target]
    out: list[dict[str, object]] = []
    for root in candidates:
        if not root.exists():
            continue
        for path in sorted(root.rglob("*")):
            if path.is_file():
                out.append(
                    {
                        "path": str(path),
                        "size_bytes": path.stat().st_size,
                        "mtime_utc": time.strftime(
                            "%Y-%m-%dT%H:%M:%SZ", time.gmtime(path.stat().st_mtime)
                        ),
                    }
                )
    return out


def run_command(
    cmd: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    log_path: Path,
    timeout_seconds: int,
    memory_mb: int | None = None,
    duration_complete_seconds: int | None = None,
) -> dict[str, object]:
    start = time.monotonic()
    started = utc()
    log_path.parent.mkdir(parents=True, exist_ok=True)
    timed_out = False
    duration_completed = False
    memory_exceeded = False
    peak_rss_kib = 0
    exit_code: int | None
    with log_path.open("w", encoding="utf-8", errors="replace") as log:
        log.write(f"$ {' '.join(cmd)}\n")
        log.flush()
        popen_kwargs: dict[str, object] = {
            "cwd": cwd,
            "env": env,
            "stdout": log,
            "stderr": subprocess.STDOUT,
            "text": True,
        }
        if os.name != "nt":
            popen_kwargs["preexec_fn"] = os.setsid
        proc = subprocess.Popen(cmd, **popen_kwargs)
        memory_cap_kib = memory_mb * 1024 if memory_mb else None
        while proc.poll() is None:
            elapsed_now = time.monotonic() - start
            rss_kib = process_tree_rss_kib(proc.pid) if memory_cap_kib else 0
            peak_rss_kib = max(peak_rss_kib, rss_kib)
            if memory_cap_kib and rss_kib > memory_cap_kib:
                memory_exceeded = True
                log.write(
                    f"\nMEMORY_LIMIT_EXCEEDED process_tree_rss_kib={rss_kib} "
                    f"cap_kib={memory_cap_kib}\n"
                )
                log.flush()
                terminate_process_tree(proc)
                break
            if duration_complete_seconds and elapsed_now >= duration_complete_seconds:
                duration_completed = True
                log.write(
                    f"\nDURATION_COMPLETE requested_seconds={duration_complete_seconds} "
                    f"elapsed_seconds={elapsed_now:.3f}\n"
                )
                log.flush()
                terminate_process_tree(proc)
                break
            if elapsed_now > timeout_seconds:
                timed_out = True
                log.write(f"\nTIMEOUT after {timeout_seconds}s\n")
                log.flush()
                terminate_process_tree(proc)
                break
            time.sleep(0.5)
        exit_code = proc.poll()
        if exit_code is None:
            try:
                exit_code = proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                terminate_process_tree(proc)
                exit_code = proc.poll()
    elapsed = round(time.monotonic() - start, 3)
    output = log_path.read_text(encoding="utf-8", errors="replace")
    tail = output.splitlines()[-120:]
    if memory_exceeded:
        status = "memory_exceeded"
    elif duration_completed:
        status = "passed"
    elif timed_out:
        status = "timeout"
    else:
        status = "passed" if exit_code == 0 else "failed"
    return {
        "command": cmd,
        "cwd": str(cwd),
        "log_path": str(log_path),
        "started_at_utc": started,
        "elapsed_seconds": elapsed,
        "timeout_seconds": timeout_seconds,
        "timed_out": timed_out,
        "duration_complete_seconds": duration_complete_seconds,
        "duration_completed": duration_completed,
        "memory_cap_mib": memory_mb,
        "memory_exceeded": memory_exceeded,
        "peak_rss_kib": peak_rss_kib,
        "exit_code": exit_code,
        "status": status,
        "tail": tail,
    }


def run_target(
    repo: Path,
    target: str,
    *,
    artifact_root: Path,
    memory_mb: int,
    build: bool,
    smoke_runs: int,
    timed_seconds: int,
    max_len: int,
    timeout_buffer: int,
    per_input_timeout: int,
) -> dict[str, object]:
    fuzz_dir = repo / "fuzz"
    env = os.environ.copy()
    env.setdefault("CARGO_BUILD_JOBS", "1")
    env.setdefault("CARGO_INCREMENTAL", "0")
    env.setdefault("WELLFRIENDPDF_FUZZ_NO_NETWORK", "1")
    env.setdefault("WELLPDF_DISABLE_NETWORK", "1")
    target_artifacts = artifact_root / target
    target_artifacts.mkdir(parents=True, exist_ok=True)
    phases: list[dict[str, object]] = []

    if build:
        cmd = ["cargo", "+nightly", "fuzz", "build", target] + cargo_fuzz_options()
        phases.append(
            run_command(
                cmd,
                cwd=fuzz_dir,
                env=env,
                log_path=target_artifacts / "build.log",
                timeout_seconds=900,
                memory_mb=memory_mb,
            )
        )
        if phases[-1]["status"] != "passed":
            return finish_target(repo, target, target_artifacts, phases, memory_mb, max_len)

    if smoke_runs > 0:
        # `cargo fuzz run` can still rebuild the ASan binary after an explicit
        # `cargo fuzz build`, especially after source changes or target switches.
        # The smoke timeout therefore includes build overhead plus the bounded
        # libFuzzer run, while the process-tree RSS monitor remains the memory
        # guard.
        smoke_timeout = max(timeout_buffer + smoke_runs, 600)
        libfuzzer_args = [
            f"-runs={smoke_runs}",
            f"-max_len={max_len}",
            f"-rss_limit_mb={memory_mb}",
            f"-timeout={per_input_timeout}",
            f"-artifact_prefix={target_artifacts.as_posix()}/",
        ]
        cmd = ["cargo", "+nightly", "fuzz", "run", target] + cargo_fuzz_options() + ["--"] + libfuzzer_args
        phases.append(
            run_command(
                cmd,
                cwd=fuzz_dir,
                env=env,
                log_path=target_artifacts / "smoke.log",
                timeout_seconds=smoke_timeout,
                memory_mb=memory_mb,
            )
        )
        if phases[-1]["status"] != "passed":
            return finish_target(repo, target, target_artifacts, phases, memory_mb, max_len)

    if timed_seconds > 0:
        timed_timeout = timed_seconds + max(timeout_buffer, 600)
        libfuzzer_args = [
            f"-max_total_time={timed_seconds}",
            f"-max_len={max_len}",
            f"-rss_limit_mb={memory_mb}",
            f"-timeout={per_input_timeout}",
            f"-artifact_prefix={target_artifacts.as_posix()}/",
        ]
        cmd = ["cargo", "+nightly", "fuzz", "run", target] + cargo_fuzz_options() + ["--"] + libfuzzer_args
        phases.append(
            run_command(
                cmd,
                cwd=fuzz_dir,
                env=env,
                log_path=target_artifacts / "long.log",
                timeout_seconds=timed_timeout,
                memory_mb=memory_mb,
                duration_complete_seconds=timed_seconds,
            )
        )

    return finish_target(repo, target, target_artifacts, phases, memory_mb, max_len)


def finish_target(
    repo: Path,
    target: str,
    target_artifacts: Path,
    phases: list[dict[str, object]],
    memory_mb: int,
    max_len: int,
) -> dict[str, object]:
    return {
        "target": target,
        "memory_cap_mib": memory_mb,
        "input_cap_bytes": max_len,
        "artifact_dir": str(target_artifacts),
        "phases": phases,
        "artifacts": collect_artifacts(repo, target, target_artifacts),
        "status": "passed" if phases and all(p["status"] == "passed" for p in phases) else "failed",
    }


def write_markdown(payload: dict[str, object], path: Path) -> None:
    lines = [
        "# Prompt 27 release fuzz runner result",
        "",
        f"Generated: `{payload['generated_at_utc']}`",
        f"Verdict: `{payload['verdict']}`",
        "",
        "| target | status | phases | artifact dir |",
        "| --- | --- | --- | --- |",
    ]
    for target in payload["targets"]:
        phases = ", ".join(f"{p['status']}:{Path(p['log_path']).name}" for p in target["phases"])
        lines.append(f"| {target['target']} | {target['status']} | {phases} | {target['artifact_dir']} |")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--targets", default=None)
    parser.add_argument("--group", action="append", default=[])
    parser.add_argument("--artifact-root", type=Path, default=Path("target/prompt27-verapdf-crypto-fuzz/release-fuzz-artifacts"))
    parser.add_argument("--json-output", type=Path, default=Path("target/prompt27-verapdf-crypto-fuzz/release-fuzz-runner-smoke.json"))
    parser.add_argument("--markdown-output", type=Path, default=None)
    parser.add_argument("--memory-mb", type=int, default=16384)
    parser.add_argument("--smoke-runs", type=int, default=64)
    parser.add_argument("--seconds", type=int, default=0, help="timed fuzz seconds per selected target")
    parser.add_argument("--long-high-priority", action="store_true")
    parser.add_argument("--high-priority-seconds", type=int, default=1800)
    parser.add_argument("--max-len", type=int, default=262144)
    parser.add_argument("--timeout-buffer", type=int, default=120)
    parser.add_argument("--per-input-timeout", type=int, default=30)
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--no-smoke", action="store_true")
    args = parser.parse_args()

    repo = args.repo.resolve()
    targets = resolve_targets(repo, args.targets, args.group)
    artifact_root = args.artifact_root if args.artifact_root.is_absolute() else repo / args.artifact_root
    output = args.json_output if args.json_output.is_absolute() else repo / args.json_output
    high_priority = sorted(HIGH_PRIORITY_LONG & set(targets)) if args.long_high_priority else []
    started = utc()
    results = []
    for target in targets:
        timed_seconds = args.seconds
        if target in high_priority:
            timed_seconds = max(timed_seconds, args.high_priority_seconds)
        results.append(
            run_target(
                repo,
                target,
                artifact_root=artifact_root,
                memory_mb=args.memory_mb,
                build=not args.no_build,
                smoke_runs=0 if args.no_smoke else args.smoke_runs,
                timed_seconds=timed_seconds,
                max_len=args.max_len,
                timeout_buffer=args.timeout_buffer,
                per_input_timeout=args.per_input_timeout,
            )
        )
    passed = all(item["status"] == "passed" for item in results)
    payload = {
        "schema_version": SCHEMA_VERSION,
        "generated_at_utc": utc(),
        "started_at_utc": started,
        "repo": str(repo),
        "targets_requested": targets,
        "memory_cap_mib": args.memory_mb,
        "prompt25b_low_memory_posture": {
            "cargo_build_jobs": os.environ.get("CARGO_BUILD_JOBS", "1"),
            "cargo_incremental": os.environ.get("CARGO_INCREMENTAL", "0"),
            "dev_build": True,
            "codegen_units": 256,
            "no_trace_compares": True,
            "input_cap_bytes": args.max_len,
            "one_target_at_a_time": True,
            "user_approved_memory_override": "16 GiB per fuzz process on VPS; total Wellfriend allocation remains 32 GiB",
        },
        "long_campaign": {
            "high_priority_targets": high_priority,
            "seconds_per_high_priority_target": args.high_priority_seconds if high_priority else 0,
            "minimum_policy": "at least 30 minutes per high-priority parser target",
            "met_policy": False,
        },
        "targets": results,
        "verdict": "passed" if passed else "failed",
    }
    # Compute long policy without hiding phase failures.
    timed_ok = []
    for item in results:
        if item["target"] in high_priority:
            long_phases = [p for p in item["phases"] if Path(str(p["log_path"])).name == "long.log"]
            timed_ok.append(bool(long_phases) and all(p["status"] == "passed" for p in long_phases))
    payload["long_campaign"]["met_policy"] = bool(high_priority) and all(timed_ok)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.markdown_output:
        md = args.markdown_output if args.markdown_output.is_absolute() else repo / args.markdown_output
        write_markdown(payload, md)
    print(json.dumps({"output": str(output), "verdict": payload["verdict"]}, sort_keys=True))
    return 0 if passed else 2


if __name__ == "__main__":
    raise SystemExit(main())
