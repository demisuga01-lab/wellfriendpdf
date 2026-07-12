#!/usr/bin/env python3
"""Run Oxide large-file probe operations under a hard per-process memory cap.

The Windows path uses a Job Object with process/job memory limits and samples
the child process working set/private bytes at fixed intervals. The child emits
newline-delimited JSON progress events, which are preserved in the result file.
"""

from __future__ import annotations

import argparse
import csv
import ctypes
import json
import os
import platform
import queue
import subprocess
import sys
import threading
import time
from ctypes import wintypes
from pathlib import Path
from typing import Any


PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
PROCESS_VM_READ = 0x0010
JOB_OBJECT_LIMIT_PROCESS_MEMORY = 0x00000100
JOB_OBJECT_LIMIT_JOB_MEMORY = 0x00000200
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000
JobObjectExtendedLimitInformation = 9


class IO_COUNTERS(ctypes.Structure):
    _fields_ = [
        ("ReadOperationCount", ctypes.c_ulonglong),
        ("WriteOperationCount", ctypes.c_ulonglong),
        ("OtherOperationCount", ctypes.c_ulonglong),
        ("ReadTransferCount", ctypes.c_ulonglong),
        ("WriteTransferCount", ctypes.c_ulonglong),
        ("OtherTransferCount", ctypes.c_ulonglong),
    ]


class JOBOBJECT_BASIC_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", ctypes.c_longlong),
        ("PerJobUserTimeLimit", ctypes.c_longlong),
        ("LimitFlags", wintypes.DWORD),
        ("MinimumWorkingSetSize", ctypes.c_size_t),
        ("MaximumWorkingSetSize", ctypes.c_size_t),
        ("ActiveProcessLimit", wintypes.DWORD),
        ("Affinity", ctypes.c_size_t),
        ("PriorityClass", wintypes.DWORD),
        ("SchedulingClass", wintypes.DWORD),
    ]


class JOBOBJECT_EXTENDED_LIMIT_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("BasicLimitInformation", JOBOBJECT_BASIC_LIMIT_INFORMATION),
        ("IoInfo", IO_COUNTERS),
        ("ProcessMemoryLimit", ctypes.c_size_t),
        ("JobMemoryLimit", ctypes.c_size_t),
        ("PeakProcessMemoryUsed", ctypes.c_size_t),
        ("PeakJobMemoryUsed", ctypes.c_size_t),
    ]


class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD),
        ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


def _kernel32() -> ctypes.WinDLL:
    return ctypes.WinDLL("kernel32", use_last_error=True)


def _psapi() -> ctypes.WinDLL:
    return ctypes.WinDLL("psapi", use_last_error=True)


def _check_windows_bool(ok: int, call: str) -> None:
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error(), call)


def create_limited_job(limit_bytes: int) -> int | None:
    if platform.system() != "Windows":
        return None
    kernel32 = _kernel32()
    kernel32.CreateJobObjectW.argtypes = [wintypes.LPVOID, wintypes.LPCWSTR]
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    job = kernel32.CreateJobObjectW(None, None)
    if not job:
        raise ctypes.WinError(ctypes.get_last_error(), "CreateJobObjectW")

    info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION()
    info.BasicLimitInformation.LimitFlags = (
        JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_JOB_MEMORY
        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    )
    info.ProcessMemoryLimit = limit_bytes
    info.JobMemoryLimit = limit_bytes
    kernel32.SetInformationJobObject.argtypes = [
        wintypes.HANDLE,
        ctypes.c_int,
        wintypes.LPVOID,
        wintypes.DWORD,
    ]
    _check_windows_bool(
        kernel32.SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            ctypes.byref(info),
            ctypes.sizeof(info),
        ),
        "SetInformationJobObject",
    )
    return int(job)


def assign_process_to_job(job: int | None, process_handle: int) -> None:
    if job is None:
        return
    kernel32 = _kernel32()
    kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    _check_windows_bool(
        kernel32.AssignProcessToJobObject(wintypes.HANDLE(job), wintypes.HANDLE(process_handle)),
        "AssignProcessToJobObject",
    )


def close_handle(handle: int | None) -> None:
    if handle is None:
        return
    kernel32 = _kernel32()
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle(wintypes.HANDLE(handle))


def terminate_job(job: int | None, code: int = 1) -> None:
    if job is None:
        return
    kernel32 = _kernel32()
    kernel32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel32.TerminateJobObject(wintypes.HANDLE(job), code)


def sample_memory(pid: int) -> dict[str, int]:
    if platform.system() != "Windows":
        return sample_memory_fallback(pid)
    kernel32 = _kernel32()
    psapi = _psapi()
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, False, pid)
    if not handle:
        return {}
    try:
        counters = PROCESS_MEMORY_COUNTERS_EX()
        counters.cb = ctypes.sizeof(PROCESS_MEMORY_COUNTERS_EX)
        psapi.GetProcessMemoryInfo.argtypes = [
            wintypes.HANDLE,
            ctypes.POINTER(PROCESS_MEMORY_COUNTERS_EX),
            wintypes.DWORD,
        ]
        ok = psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb)
        if not ok:
            return {}
        return {
            "working_set_bytes": int(counters.WorkingSetSize),
            "peak_working_set_bytes": int(counters.PeakWorkingSetSize),
            "private_bytes": int(counters.PrivateUsage),
            "peak_private_bytes": int(counters.PeakPagefileUsage),
        }
    finally:
        close_handle(int(handle))


def sample_memory_fallback(pid: int) -> dict[str, int]:
    statm = Path(f"/proc/{pid}/statm")
    if not statm.exists():
        return {}
    fields = statm.read_text().split()
    if len(fields) < 2:
        return {}
    page_size = os.sysconf("SC_PAGE_SIZE")
    return {"working_set_bytes": int(fields[1]) * page_size}


def reader_thread(stream: Any, out: "queue.Queue[tuple[str, str]]", name: str) -> None:
    try:
        for line in iter(stream.readline, ""):
            out.put((name, line.rstrip("\n")))
    finally:
        stream.close()


def run_capped_command(args: argparse.Namespace, command: list[str]) -> dict[str, Any]:
    result_dir = Path(args.results_dir)
    result_dir.mkdir(parents=True, exist_ok=True)
    run_id = args.run_id or build_run_id(args)
    sample_path = result_dir / f"{run_id}.samples.csv"
    result_path = result_dir / f"{run_id}.json"

    limit_bytes = int(args.memory_limit_mb * 1024 * 1024)
    job = create_limited_job(limit_bytes)
    proc = subprocess.Popen(
        command,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
        universal_newlines=True,
    )
    try:
        if platform.system() == "Windows":
            assign_process_to_job(job, int(proc._handle))  # noqa: SLF001 - Windows handle from Popen.
        output_queue: "queue.Queue[tuple[str, str]]" = queue.Queue()
        threads = [
            threading.Thread(target=reader_thread, args=(proc.stdout, output_queue, "stdout")),
            threading.Thread(target=reader_thread, args=(proc.stderr, output_queue, "stderr")),
        ]
        for thread in threads:
            thread.daemon = True
            thread.start()

        start = time.perf_counter()
        deadline = start + args.timeout_seconds
        events: list[dict[str, Any]] = []
        stderr_lines: list[str] = []
        samples: list[dict[str, int]] = []
        peak_working_set = 0
        peak_private = 0
        first_page_elapsed_ms: int | None = None
        pages_completed = 0
        timed_out = False

        with sample_path.open("w", newline="", encoding="utf-8") as sample_file:
            writer = csv.DictWriter(
                sample_file,
                fieldnames=[
                    "elapsed_ms",
                    "working_set_bytes",
                    "peak_working_set_bytes",
                    "private_bytes",
                    "peak_private_bytes",
                ],
            )
            writer.writeheader()
            while True:
                now = time.perf_counter()
                while True:
                    try:
                        stream_name, line = output_queue.get_nowait()
                    except queue.Empty:
                        break
                    if stream_name == "stderr":
                        stderr_lines.append(line)
                        continue
                    if not line:
                        continue
                    try:
                        event = json.loads(line)
                    except json.JSONDecodeError:
                        event = {"event": "raw_stdout", "line": line}
                    if not isinstance(event, dict):
                        event = {"event": "raw_stdout", "line": line}
                    events.append(event)
                    if event.get("event") == "page_done" and first_page_elapsed_ms is None:
                        first_page_elapsed_ms = int(event.get("elapsed_ms", 0))
                    if "pages_completed" in event:
                        pages_completed = max(pages_completed, int(event["pages_completed"]))
                    elif event.get("event") == "page_done" and "page" in event:
                        pages_completed = max(pages_completed, int(event["page"]))

                mem = sample_memory(proc.pid)
                if mem:
                    mem["elapsed_ms"] = int((now - start) * 1000)
                    peak_working_set = max(peak_working_set, mem.get("peak_working_set_bytes", 0))
                    peak_private = max(peak_private, mem.get("private_bytes", 0))
                    row = {
                        "elapsed_ms": mem.get("elapsed_ms", 0),
                        "working_set_bytes": mem.get("working_set_bytes", 0),
                        "peak_working_set_bytes": mem.get("peak_working_set_bytes", 0),
                        "private_bytes": mem.get("private_bytes", 0),
                        "peak_private_bytes": mem.get("peak_private_bytes", 0),
                    }
                    samples.append(row)
                    writer.writerow(row)
                    sample_file.flush()

                if proc.poll() is not None:
                    break
                if now > deadline:
                    timed_out = True
                    terminate_job(job)
                    proc.kill()
                    break
                time.sleep(args.sample_interval_ms / 1000.0)

        for thread in threads:
            thread.join(timeout=1.0)

        elapsed_ms = int((time.perf_counter() - start) * 1000)
        exit_code = proc.poll()
        stderr_text = "\n".join(stderr_lines)
        hit_memory_cap = (
            peak_working_set >= int(limit_bytes * 0.98)
            or peak_private >= int(limit_bytes * 0.98)
            or "memory allocation" in stderr_text.lower()
            or "out of memory" in stderr_text.lower()
            or "not enough memory" in stderr_text.lower()
        )
        result = {
            "run_id": run_id,
            "command": command,
            "platform": platform.platform(),
            "memory_limit_bytes": limit_bytes,
            "sample_interval_ms": args.sample_interval_ms,
            "timeout_seconds": args.timeout_seconds,
            "elapsed_ms": elapsed_ms,
            "exit_code": exit_code,
            "timed_out": timed_out,
            "hit_memory_cap": hit_memory_cap,
            "peak_working_set_bytes": peak_working_set,
            "peak_private_bytes": peak_private,
            "time_to_first_page_ms": first_page_elapsed_ms,
            "pages_completed": pages_completed,
            "events": events,
            "stderr_tail": stderr_lines[-80:],
            "samples_csv": str(sample_path),
        }
        result_path.write_text(json.dumps(result, indent=2), encoding="utf-8")
        return result
    finally:
        close_handle(job)


def build_run_id(args: argparse.Namespace) -> str:
    stem = "command"
    if getattr(args, "input", None):
        stem = Path(args.input).stem
    operation = getattr(args, "operation", "run")
    mode = getattr(args, "mode", "default")
    timestamp = time.strftime("%Y%m%d-%H%M%S")
    safe = "".join(ch if ch.isalnum() or ch in "._-" else "_" for ch in f"{stem}-{operation}-{mode}")
    return f"{timestamp}-{safe}"


def cmd_run(args: argparse.Namespace) -> int:
    command = [
        str(Path(args.worker)),
        "--input",
        str(Path(args.input)),
        "--operation",
        args.operation,
        "--pages",
        args.pages,
        "--mode",
        args.mode,
        "--dpi",
        str(args.dpi),
    ]
    if args.output:
        command += ["--output", str(Path(args.output))]
    if args.password:
        command += ["--password", args.password]
    result = run_capped_command(args, command)
    if args.quiet:
        status = "timeout" if result["timed_out"] else "exit"
        print(
            json.dumps(
                {
                    "run_id": result["run_id"],
                    "status": status,
                    "exit_code": result["exit_code"],
                    "hit_memory_cap": result["hit_memory_cap"],
                    "elapsed_ms": result["elapsed_ms"],
                    "peak_working_set_bytes": result["peak_working_set_bytes"],
                    "time_to_first_page_ms": result["time_to_first_page_ms"],
                    "pages_completed": result["pages_completed"],
                }
            )
        )
    else:
        print(json.dumps(result, indent=2))
    return 0 if result["exit_code"] == 0 and not result["timed_out"] else 1


def cmd_verify_cap(args: argparse.Namespace) -> int:
    code = (
        "import time\n"
        "blocks=[]\n"
        "for i in range(512):\n"
        "    blocks.append(bytearray(4*1024*1024))\n"
        "    time.sleep(0.01)\n"
    )
    command = [sys.executable, "-c", code]
    result = run_capped_command(args, command)
    print(json.dumps(result, indent=2))
    if result["exit_code"] == 0:
        print("expected capped process to exit non-zero", file=sys.stderr)
        return 1
    return 0


def cmd_exec(args: argparse.Namespace) -> int:
    """Run an arbitrary validation command inside the same capped Job Object."""
    command = list(args.command_args)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        print("exec requires a command after --", file=sys.stderr)
        return 2
    result = run_capped_command(args, command)
    print(json.dumps(result, indent=2))
    return 0 if result["exit_code"] == 0 and not result["timed_out"] else 1


def add_common_run_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--memory-limit-mb", type=int, default=2048)
    parser.add_argument("--sample-interval-ms", type=int, default=250)
    parser.add_argument("--timeout-seconds", type=int, default=7200)
    parser.add_argument("--results-dir", default="large-file-profile/results")
    parser.add_argument("--run-id")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    run = sub.add_parser("run", help="run one probe operation under the memory cap")
    add_common_run_args(run)
    run.add_argument("--worker", required=True)
    run.add_argument("--input", required=True)
    run.add_argument("--operation", required=True)
    run.add_argument("--pages", default="all")
    run.add_argument("--mode", default="page", choices=["page", "aggregate"])
    run.add_argument("--dpi", type=int, default=72)
    run.add_argument("--output")
    run.add_argument("--password")
    run.add_argument("--quiet", action="store_true")
    run.set_defaults(func=cmd_run)

    verify = sub.add_parser("verify-cap", help="verify the OS memory cap on a throwaway process")
    add_common_run_args(verify)
    verify.set_defaults(func=cmd_verify_cap)

    execute = sub.add_parser(
        "exec", help="run an arbitrary command under the process-tree memory cap"
    )
    add_common_run_args(execute)
    execute.add_argument("command_args", nargs=argparse.REMAINDER)
    execute.set_defaults(func=cmd_exec)

    args = parser.parse_args()
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
