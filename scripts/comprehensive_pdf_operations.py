#!/usr/bin/env python3
"""Reproducible 150-document PDF operation and conversion benchmark.

The controller records raw observations only.  It never treats an unsupported
tool/operation pair as a timed success, and it validates every produced PDF
before deleting the temporary payload.  Timing excludes the controller's
postcondition checks.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import html.parser
import io
import json
import os
import random
import re
import shutil
import subprocess
import tempfile
import time
import zipfile
from pathlib import Path
from typing import Any, Iterable
from xml.etree import ElementTree


SCHEMA = "wellfriendpdf.comprehensive-operations.v1"
PASSWORD = "wf-corpus-user"
OWNER_PASSWORD = "wf-corpus-owner"


def utc_now() -> str:
    import datetime

    return datetime.datetime.now(datetime.UTC).isoformat()


def sanitized_command(command: list[str], corpus: Path) -> list[str]:
    root = str(corpus)
    return [part.replace(root, "$CORPUS") for part in command]


def tail(value: str, limit: int = 4000) -> str:
    return value[-limit:]


def execute(
    command: list[str],
    timeout_sec: int,
    *,
    stdout_path: Path | None = None,
) -> dict[str, Any]:
    started = time.perf_counter_ns()
    stdout_handle = stdout_path.open("wb") if stdout_path else subprocess.PIPE
    try:
        try:
            completed = subprocess.run(
                command,
                stdin=subprocess.DEVNULL,
                stdout=stdout_handle,
                stderr=subprocess.PIPE,
                timeout=timeout_sec,
                check=False,
            )
            elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000.0
            stdout = "" if stdout_path else completed.stdout.decode("utf-8", "replace")
            stderr = completed.stderr.decode("utf-8", "replace")
            return {
                "exit_code": completed.returncode,
                "elapsed_ms": round(elapsed_ms, 6),
                "timed_out": False,
                "stdout_tail": tail(stdout),
                "stderr_tail": tail(stderr),
            }
        except subprocess.TimeoutExpired as error:
            elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000.0
            return {
                "exit_code": None,
                "elapsed_ms": round(elapsed_ms, 6),
                "timed_out": True,
                "stdout_tail": tail((error.stdout or b"").decode("utf-8", "replace")),
                "stderr_tail": tail((error.stderr or b"").decode("utf-8", "replace")),
            }
    finally:
        if stdout_path:
            stdout_handle.close()


def execute_sequence(commands: list[list[str]], timeout_sec: int) -> dict[str, Any]:
    """Time a deterministic multi-process toolkit operation as one workflow."""
    started = time.perf_counter_ns()
    stdout_parts: list[str] = []
    stderr_parts: list[str] = []
    exit_code: int | None = 0
    timed_out = False
    for command in commands:
        remaining = max(1, timeout_sec - int((time.perf_counter_ns() - started) / 1_000_000_000))
        result = execute(command, remaining)
        stdout_parts.append(result.get("stdout_tail", ""))
        stderr_parts.append(result.get("stderr_tail", ""))
        if result["timed_out"]:
            timed_out = True
            exit_code = None
            break
        if result["exit_code"] != 0:
            exit_code = result["exit_code"]
            break
    return {
        "exit_code": exit_code,
        "elapsed_ms": round((time.perf_counter_ns() - started) / 1_000_000.0, 6),
        "timed_out": timed_out,
        "stdout_tail": tail("\n".join(stdout_parts)),
        "stderr_tail": tail("\n".join(stderr_parts)),
        "steps": commands,
    }


def qpdf_pages(path: Path, password: str | None = None) -> int | None:
    command = ["qpdf"]
    if password:
        command.append(f"--password={password}")
    command += ["--show-npages", str(path)]
    completed = subprocess.run(command, capture_output=True, text=True, check=False)
    try:
        return int(completed.stdout.strip()) if completed.returncode in (0, 3) else None
    except ValueError:
        return None


def validate_pdf(
    path: Path,
    *,
    expected_pages: int | None = None,
    password: str | None = None,
    expect_encrypted: bool | None = None,
    expect_linearized: bool = False,
) -> dict[str, Any]:
    if not path.is_file():
        return {"exists": False, "valid": False}
    check = ["qpdf"]
    if password:
        check.append(f"--password={password}")
    check += ["--check", str(path)]
    checked = subprocess.run(check, capture_output=True, text=True, check=False)
    pages = qpdf_pages(path, password)
    encrypted_probe = subprocess.run(
        ["qpdf", "--show-encryption", str(path)], capture_output=True, text=True, check=False
    )
    encrypted = "File is not encrypted" not in encrypted_probe.stdout
    linearized = None
    linearization_probe: dict[str, Any] | None = None
    if expect_linearized:
        probe = subprocess.run(
            ["qpdf", "--check-linearization", str(path)],
            capture_output=True,
            text=True,
            check=False,
        )
        probe_text = (probe.stdout + probe.stderr).lower()
        # qpdf uses exit 3 for a recognized linearized file with non-fatal
        # compatibility warnings. Preserve those warnings in the probe record,
        # but do not conflate them with an exit-2/not-linearized result.
        linearized = probe.returncode in (0, 3) and "not linearized" not in probe_text
        linearization_probe = {
            "exit_code": probe.returncode,
            "output_tail": tail(probe.stderr + probe.stdout),
        }
    valid = checked.returncode in (0, 3) and pages is not None
    if expected_pages is not None:
        valid = valid and pages == expected_pages
    if expect_encrypted is not None:
        valid = valid and encrypted == expect_encrypted
    if expect_linearized:
        valid = valid and linearized is True
    return {
        "exists": True,
        "bytes": path.stat().st_size,
        "qpdf_check_exit": checked.returncode,
        "qpdf_check_tail": tail(checked.stderr + checked.stdout),
        "page_count": pages,
        "expected_page_count": expected_pages,
        "encrypted": encrypted,
        "expected_encrypted": expect_encrypted,
        "linearized": linearized,
        "linearization_probe": linearization_probe,
        "valid": valid,
    }


def raster_fingerprint(path: Path, page: int, password: str | None = None) -> dict[str, Any]:
    command = ["mutool", "draw", "-q", "-r", "36", "-F", "ppm", "-o", "-"]
    if password:
        command += ["-p", password]
    command += [str(path), str(page)]
    completed = subprocess.run(command, capture_output=True, check=False, timeout=300)
    if completed.returncode != 0:
        return {"ok": False, "exit_code": completed.returncode, "stderr_tail": tail(completed.stderr.decode("utf-8", "replace"))}
    match = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", completed.stdout)
    if not match:
        return {"ok": False, "error": "invalid_ppm"}
    return {
        "ok": True,
        "width": int(match.group(1)),
        "height": int(match.group(2)),
        "sha256": hashlib.sha256(completed.stdout).hexdigest(),
    }


def raster_equivalence(
    left_path: Path,
    right_path: Path,
    page: int = 1,
    password: str | None = None,
) -> dict[str, Any]:
    """Compare two externally rendered pages with an anti-aliasing tolerance.

    Replaying an annotation appearance as page content can change sub-byte edge
    coverage even when geometry, colour, blend mode, and visible output are
    preserved. Exact PPM hashes remain recorded by ``raster_fingerprint``; this
    companion check rejects substantive changes while tolerating <= 8-level
    rasterizer rounding at a tiny number of edge pixels.
    """
    try:
        from PIL import Image, ImageChops
    except ImportError:
        return {"equivalent": False, "reason": "pillow_unavailable"}

    images = []
    for path in (left_path, right_path):
        command = ["mutool", "draw", "-q", "-r", "144", "-F", "ppm", "-o", "-"]
        if password:
            command += ["-p", password]
        command += [str(path), str(page)]
        completed = subprocess.run(command, capture_output=True, check=False, timeout=300)
        if completed.returncode != 0:
            return {
                "equivalent": False,
                "reason": "render_failed",
                "exit_code": completed.returncode,
                "stderr_tail": tail(completed.stderr.decode("utf-8", "replace")),
            }
        images.append(Image.open(io.BytesIO(completed.stdout)).convert("RGB"))

    left, right = images
    if left.size != right.size:
        return {
            "equivalent": False,
            "same_size": False,
            "left_size": left.size,
            "right_size": right.size,
        }
    diff = ImageChops.difference(left, right)
    histogram = diff.histogram()
    total_samples = left.size[0] * left.size[1] * 3
    absolute_sum = sum((index % 256) * count for index, count in enumerate(histogram))
    extrema = diff.getextrema()
    max_channel_delta = max(high for _low, high in extrema)
    changed_pixels_threshold8 = sum(1 for pixel in diff.getdata() if max(pixel) > 8)
    mean_absolute_error = absolute_sum / max(1, total_samples)
    equivalent = changed_pixels_threshold8 == 0 and mean_absolute_error <= 0.02
    return {
        "equivalent": equivalent,
        "same_size": True,
        "width": left.size[0],
        "height": left.size[1],
        "changed_pixels_threshold8": changed_pixels_threshold8,
        "max_channel_delta": max_channel_delta,
        "mean_absolute_error": round(mean_absolute_error, 9),
    }


def structural_visual_checks(
    operation: str,
    source: Path,
    outputs: Path | list[Path],
    pages: int,
    password: str | None,
) -> dict[str, Any]:
    source_first = raster_fingerprint(source, 1)
    result: dict[str, Any] = {"source_first": source_first}
    if isinstance(outputs, list):
        comparisons = []
        for page_number, output in enumerate(outputs, start=1):
            expected = raster_fingerprint(source, page_number)
            observed = raster_fingerprint(output, 1)
            comparisons.append({"source": expected, "output": observed, "exact_raster_match": expected.get("sha256") == observed.get("sha256")})
        result["split_page_comparisons"] = comparisons
        result["visual_postcondition"] = all(item["exact_raster_match"] for item in comparisons)
        return result
    output_first = raster_fingerprint(outputs, 1, password)
    result["output_first"] = output_first
    if operation == "merge":
        repeated = raster_fingerprint(outputs, pages + 1, password)
        result["repeated_source_first"] = repeated
        result["visual_postcondition"] = source_first.get("sha256") == output_first.get("sha256") == repeated.get("sha256")
    elif operation == "organize":
        expected_last = raster_fingerprint(source, pages)
        output_second = raster_fingerprint(outputs, 2, password)
        output_third = raster_fingerprint(outputs, 3, password)
        result.update({"source_last": expected_last, "output_second": output_second, "output_third": output_third})
        result["visual_postcondition"] = expected_last.get("sha256") == output_first.get("sha256") and source_first.get("sha256") == output_second.get("sha256") == output_third.get("sha256")
    elif operation == "extract-pages":
        result["visual_postcondition"] = source_first.get("sha256") == output_first.get("sha256")
        if pages > 1:
            source_second = raster_fingerprint(source, 2)
            output_second = raster_fingerprint(outputs, 2, password)
            result.update({"source_second": source_second, "output_second": output_second})
            result["visual_postcondition"] = result["visual_postcondition"] and source_second.get("sha256") == output_second.get("sha256")
    elif operation in {"watermark", "page-numbers", "crop", "nup", "resize", "rotate"}:
        result["visual_postcondition"] = source_first.get("sha256") != output_first.get("sha256")
        if operation == "rotate":
            result["dimensions_swapped_or_square"] = (
                source_first.get("width") == output_first.get("height")
                and source_first.get("height") == output_first.get("width")
            ) or source_first.get("width") == source_first.get("height")
            result["visual_postcondition"] = result["visual_postcondition"] and result["dimensions_swapped_or_square"]
    elif operation == "flatten":
        equivalence = raster_equivalence(source, outputs, 1, password)
        result["appearance_equivalence"] = equivalence
        result["visual_postcondition"] = bool(equivalence.get("equivalent"))
    else:
        result["visual_postcondition"] = source_first.get("sha256") == output_first.get("sha256")
    return result


def damaged_copy(source: Path, destination: Path) -> dict[str, Any]:
    data = bytearray(source.read_bytes())
    matches = list(re.finditer(rb"startxref\s+(\d+)", data))
    if not matches:
        destination.write_bytes(data[:-8])
        return {"mutation": "truncate-final-8-bytes"}
    match = matches[-1]
    start, end = match.span(1)
    data[start:end] = b"0" * (end - start)
    destination.write_bytes(data)
    return {"mutation": "zero-final-startxref", "offset": start}


def classify(run: dict[str, Any], quality: dict[str, Any]) -> str:
    if run["timed_out"]:
        return "timeout"
    if run["exit_code"] in (0, 3) and quality.get("valid", False):
        return "pass"
    combined = (run.get("stdout_tail", "") + run.get("stderr_tail", "")).lower()
    if "unsupported" in combined or "not supported" in combined or "blocked" in combined:
        return "typed_refusal"
    return "fail"


class TextCollector(html.parser.HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.parts: list[str] = []

    def handle_data(self, data: str) -> None:
        self.parts.append(data)


def normalized_tokens(text: str) -> list[str]:
    return re.findall(r"[\w]+", text.casefold(), flags=re.UNICODE)


def token_agreement(reference: list[str], observed: list[str]) -> dict[str, Any]:
    if not reference:
        return {"applicable": False, "reason": "reference_text_empty"}
    left, right = collections.Counter(reference), collections.Counter(observed)
    overlap = sum((left & right).values())
    precision = overlap / max(1, sum(right.values()))
    recall = overlap / max(1, sum(left.values()))
    f1 = 0.0 if precision + recall == 0 else 2 * precision * recall / (precision + recall)
    return {
        "applicable": True,
        "reference_tokens": sum(left.values()),
        "observed_tokens": sum(right.values()),
        "token_precision_vs_poppler": round(precision, 6),
        "token_recall_vs_poppler": round(recall, 6),
        "token_f1_vs_poppler": round(f1, 6),
        "note": "agreement with Poppler extraction, not semantic ground truth",
    }


def text_from_artifact(path: Path, kind: str) -> str:
    if kind in {"txt", "md"}:
        return path.read_text("utf-8", errors="replace")
    if kind == "html":
        parser = TextCollector()
        parser.feed(path.read_text("utf-8", errors="replace"))
        return " ".join(parser.parts)
    if kind == "json":
        value = json.loads(path.read_text("utf-8", errors="replace"))
        parts: list[str] = []

        def walk(item: Any) -> None:
            if isinstance(item, str):
                parts.append(item)
            elif isinstance(item, list):
                for child in item:
                    walk(child)
            elif isinstance(item, dict):
                for child in item.values():
                    walk(child)

        walk(value)
        return " ".join(parts)
    if kind in {"docx", "pptx", "xlsx"}:
        prefixes = {
            "docx": ("word/",),
            "pptx": ("ppt/slides/",),
            "xlsx": ("xl/worksheets/", "xl/sharedStrings.xml"),
        }[kind]
        parts = []
        with zipfile.ZipFile(path) as archive:
            for name in archive.namelist():
                if not any(name.startswith(prefix) for prefix in prefixes):
                    continue
                if not name.endswith(".xml"):
                    continue
                try:
                    root = ElementTree.fromstring(archive.read(name))
                except ElementTree.ParseError:
                    continue
                parts.extend(node.text or "" for node in root.iter() if node.text)
        return " ".join(parts)
    return ""


def validate_artifact(path: Path, kind: str, reference: list[str]) -> dict[str, Any]:
    if not path.exists():
        return {"exists": False, "valid": False}
    result: dict[str, Any] = {"exists": True, "bytes": path.stat().st_size}
    try:
        if kind in {"docx", "pptx", "xlsx", "zip"}:
            with zipfile.ZipFile(path) as archive:
                bad = archive.testzip()
                names = set(archive.namelist())
            required = {
                "docx": "word/document.xml",
                "pptx": "ppt/presentation.xml",
                "xlsx": "xl/workbook.xml",
            }.get(kind)
            result.update(
                {
                    "zip_valid": bad is None,
                    "required_part": required,
                    "required_part_present": required is None or required in names,
                    "entry_count": len(names),
                }
            )
            valid = bad is None and bool(names) and (required is None or required in names)
        elif kind in {"json", "table-json"}:
            parsed_json = json.loads(path.read_text("utf-8", errors="strict"))
            valid = path.stat().st_size > 1
            if kind == "table-json":
                tables = parsed_json.get("tables", []) if isinstance(parsed_json, dict) else parsed_json
                result["table_count"] = len(tables) if isinstance(tables, list) else None
        elif kind in {"txt", "md", "html", "svg", "ps", "eps"}:
            valid = path.stat().st_size > 0
            if kind == "html":
                valid = valid and "<" in path.read_text("utf-8", errors="replace")[:4096]
        elif kind in {"png", "jpg"}:
            from PIL import Image

            with Image.open(path) as image:
                image.verify()
            with Image.open(path) as image:
                result.update(
                    {
                        "image_format": image.format,
                        "width": image.width,
                        "height": image.height,
                        "mode": image.mode,
                    }
                )
                expected_format = "PNG" if kind == "png" else "JPEG"
                valid = image.format == expected_format and image.width > 0 and image.height > 0
        else:
            valid = path.stat().st_size > 0
        result["valid"] = valid
        if valid and kind in {"txt", "md", "html", "json", "docx", "pptx", "xlsx"}:
            observed = normalized_tokens(text_from_artifact(path, kind))
            result["text_agreement"] = token_agreement(reference, observed)
    except Exception as error:  # noqa: BLE001 - evidence must retain parser class.
        result.update({"valid": False, "validation_error": error.__class__.__name__})
    return result


def write_row(stream: Any, row: dict[str, Any]) -> None:
    stream.write(json.dumps(row, sort_keys=True) + "\n")
    stream.flush()


def cleanup_temp_children(directory: Path, keep: Iterable[Path] = ()) -> None:
    retained = {path.resolve() for path in keep if path.exists()}
    for child in directory.iterdir():
        if child.resolve() in retained:
            continue
        if child.is_dir() and not child.is_symlink():
            shutil.rmtree(child)
        else:
            child.unlink(missing_ok=True)


def common_row(
    phase: str,
    operation: str,
    tool: str,
    entry: dict[str, Any],
    command: list[str],
    corpus: Path,
) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA,
        "timestamp_utc": utc_now(),
        "phase": phase,
        "operation": operation,
        "tool": tool,
        "relative_path": entry["relative_path"],
        "input_sha256": entry["sha256"],
        "input_bytes": entry["bytes"],
        "corpus_partition": entry.get("corpus_partition"),
        "command": sanitized_command(command, corpus),
    }


def timeout_for(entry: dict[str, Any], multiplier: float = 1.0) -> int:
    mib = int(entry["bytes"]) / (1024 * 1024)
    return max(180, min(7200, int((180 + mib * 12) * multiplier)))


def structural_tasks(
    operation: str,
    tool: str,
    source: Path,
    pages: int,
    tmp: Path,
    wf: Path,
    sdk: Path,
    cert: Path,
    key: Path,
) -> tuple[list[str], Path | list[Path], dict[str, Any]]:
    output = tmp / f"{operation}-{tool}.pdf"
    expected = pages
    options: dict[str, Any] = {"expected_pages": expected}
    if operation == "merge":
        expected = pages * 2
        options["expected_pages"] = expected
        commands = {
            "wellfriend": [str(wf), "merge", str(source), str(source), "-o", str(output), "--json"],
            "qpdf": ["qpdf", "--empty", "--pages", str(source), str(source), "--", str(output)],
            "mupdf": ["mutool", "merge", "-o", str(output), str(source), "1-N", str(source), "1-N"],
            "poppler": ["pdfunite", str(source), str(source), str(output)],
        }
        return commands[tool], output, options
    if operation == "split":
        last = min(2, pages)
        options["expected_split_files"] = last
        if tool == "wellfriend":
            pattern = tmp / "split-wellfriend-%d.pdf"
            command = [str(wf), "split", str(source), "-f", "1", "-l", str(last), "-o", str(pattern), "--json"]
        elif tool == "qpdf":
            pattern = tmp / "split-qpdf-%d.pdf"
            command = ["qpdf", str(source), "--pages", ".", f"1-{last}", "--", "--split-pages=1", str(pattern)]
        elif tool == "mupdf":
            pattern = tmp / "split-mupdf-%d.pdf"
            command = ["mutool", "convert", "-F", "pdf", "-o", str(pattern), str(source), f"1-{last}"]
        else:
            pattern = tmp / "split-poppler-%d.pdf"
            command = ["pdfseparate", "-f", "1", "-l", str(last), str(source), str(pattern)]
        return command, [tmp / f"split-{tool}-{index}.pdf" for index in range(1, last + 1)], options
    if operation == "lock":
        options.update({"expected_encrypted": True, "password": PASSWORD})
        commands = {
            "wellfriend": [str(wf), "encrypt", str(source), "-o", str(output), "--user-pw", PASSWORD, "--owner-pw", OWNER_PASSWORD, "--algo", "aes256", "--json"],
            "qpdf": ["qpdf", "--encrypt", PASSWORD, OWNER_PASSWORD, "256", "--", str(source), str(output)],
            "mupdf": ["mutool", "clean", "-E", "aes-256", "-U", PASSWORD, "-O", OWNER_PASSWORD, str(source), str(output)],
        }
        return commands[tool], output, options
    if operation == "rotate":
        commands = {
            "wellfriend": [str(wf), "rotate", str(source), "--angle", "90", "--relative", "--pages", "all", "-o", str(output), "--json"],
            "qpdf": ["qpdf", str(source), "--rotate=+90:1-z", str(output)],
        }
        return commands[tool], output, options
    if operation == "organize":
        order = f"{pages},1,1"
        options["expected_pages"] = 3
        commands = {
            "wellfriend": [str(wf), "organize", str(source), "--order", order, "-o", str(output), "--json"],
            "qpdf": ["qpdf", "--empty", "--pages", str(source), order, "--", str(output)],
            "mupdf": ["mutool", "merge", "-o", str(output), str(source), order],
        }
        return commands[tool], output, options
    if operation == "linearize":
        options["expect_linearized"] = True
        commands = {
            "wellfriend": [str(wf), "linearize", str(source), "-o", str(output), "--json"],
            "qpdf": ["qpdf", "--linearize", str(source), str(output)],
        }
        return commands[tool], output, options
    if operation == "flatten":
        commands = {
            "wellfriend": [str(wf), "annotations-flatten", str(source), "-o", str(output), "--json"],
            "qpdf": ["qpdf", str(source), "--flatten-annotations=all", str(output)],
        }
        return commands[tool], output, options
    if operation == "watermark":
        return [str(wf), "watermark", str(source), "--text", "WF BENCHMARK", "--pages", "1", "--position", "center", "--opacity", "0.28", "-o", str(output), "--json"], output, options
    if operation == "metadata":
        return [str(sdk), "metadata", str(source), str(output), "Subject", "Wellfriend corpus benchmark"], output, options
    if operation == "resize":
        options["expected_pages"] = 1
        return [str(wf), "pages-scale", str(source), "--scale", "0.9", "--pages", "1", "--dpi", "72", "-o", str(output), "--json"], output, options
    if operation == "extract-pages":
        last = min(2, pages)
        options["expected_pages"] = last
        commands = {
            "wellfriend": [str(wf), "extract-pages", str(source), f"1-{last}", "-o", str(output), "--json"],
            "qpdf": ["qpdf", "--empty", "--pages", str(source), f"1-{last}", "--", str(output)],
            "mupdf": ["mutool", "merge", "-o", str(output), str(source), f"1-{last}"],
            "poppler": ["pdfunite", str(source), str(output)],
        }
        return commands[tool], output, options
    if operation == "optimize":
        commands = {
            "wellfriend": [str(wf), "optimize", str(source), "-o", str(output), "--json"],
            "qpdf": ["qpdf", "--object-streams=generate", "--recompress-flate", "--compression-level=9", str(source), str(output)],
            "mupdf": ["mutool", "clean", "-gggg", "-z", str(source), str(output)],
        }
        return commands[tool], output, options
    if operation == "canonicalize":
        commands = {
            "wellfriend": [str(wf), "canonicalize", str(source), "-o", str(output), "--json", "--source-date-epoch", "0"],
            "qpdf": ["qpdf", "--qdf", "--object-streams=disable", str(source), str(output)],
            "mupdf": ["mutool", "clean", "-gg", str(source), str(output)],
        }
        return commands[tool], output, options
    if operation == "sanitize":
        return [str(wf), "sanitize", str(source), "-o", str(output), "--policy", "balanced", "--json"], output, options
    if operation == "page-numbers":
        return [str(wf), "add-page-numbers", str(source), "--pages", "1", "--position", "bottom-center", "--format", "WF {n}/{total}", "-o", str(output), "--json"], output, options
    if operation == "crop":
        return [str(wf), "pages-crop", str(source), "--rect", "10,10,300,300", "--pages", "1", "-o", str(output), "--json"], output, options
    if operation == "nup":
        options["expected_pages"] = 1
        last = min(2, pages)
        return [str(wf), "pages-nup", str(source), "--columns", "2", "--rows", "1", "--pages", f"1-{last}", "--dpi", "72", "-o", str(output), "--json"], output, options
    if operation == "sign":
        return [str(wf), "signature-sign", str(source), "--key", str(key), "--cert", str(cert), "-o", str(output), "--field-name", "WFBenchmarkSignature", "--reason", "Corpus interoperability benchmark", "--json", "--force"], output, options
    raise ValueError(f"unknown task {operation}/{tool}")


STRUCTURAL_SUPPORT = {
    "merge": ("wellfriend", "qpdf", "mupdf", "poppler"),
    "split": ("wellfriend", "qpdf", "mupdf", "poppler"),
    "lock": ("wellfriend", "qpdf", "mupdf"),
    "unlock": ("wellfriend", "qpdf", "mupdf"),
    "rotate": ("wellfriend", "qpdf"),
    "repair": ("wellfriend", "qpdf", "mupdf"),
    "organize": ("wellfriend", "qpdf", "mupdf", "poppler"),
    "linearize": ("wellfriend", "qpdf"),
    "flatten": ("wellfriend", "qpdf"),
    "watermark": ("wellfriend",),
    "metadata": ("wellfriend",),
    "resize": ("wellfriend",),
    "extract-pages": ("wellfriend", "qpdf", "mupdf", "poppler"),
    "optimize": ("wellfriend", "qpdf", "mupdf"),
    "canonicalize": ("wellfriend", "qpdf", "mupdf"),
    "sanitize": ("wellfriend",),
    "page-numbers": ("wellfriend",),
    "crop": ("wellfriend",),
    "nup": ("wellfriend",),
    "sign": ("wellfriend", "poppler"),
}


def run_structural(args: argparse.Namespace, entries: list[dict[str, Any]], stream: Any, completed: set[tuple[str, str, str]]) -> None:
    rng = random.Random(args.seed)
    cert_dir = args.output / "ephemeral-signing"
    cert_dir.mkdir(parents=True, exist_ok=True)
    key, cert = cert_dir / "key.pem", cert_dir / "cert.pem"
    if not key.exists() or not cert.exists():
        subprocess.run(
            ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(key), "-out", str(cert), "-subj", "/CN=Wellfriend Corpus Benchmark", "-days", "7"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    nss_dir = cert_dir / "nssdb"
    p12 = cert_dir / "benchmark-signing.p12"
    nss_ready = cert_dir / ".nss-ready"
    if not nss_ready.exists():
        nss_dir.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["openssl", "pkcs12", "-export", "-out", str(p12), "-inkey", str(key), "-in", str(cert), "-name", "Wellfriend Benchmark", "-passout", "pass:"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        subprocess.run(
            ["certutil", "-N", "-d", f"sql:{nss_dir}", "--empty-password"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        subprocess.run(
            ["pk12util", "-i", str(p12), "-d", f"sql:{nss_dir}", "-W", ""],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        subprocess.run(
            ["certutil", "-M", "-d", f"sql:{nss_dir}", "-n", "Wellfriend Benchmark", "-t", "CT,C,C"],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        nss_ready.write_text("ready\n", encoding="utf-8")
    for doc_index, entry in enumerate(entries, start=1):
        source = args.corpus / entry["relative_path"]
        pages = qpdf_pages(source)
        if pages is None:
            write_row(stream, {**common_row("structural", "precheck", "qpdf", entry, [], args.corpus), "status": "fail", "reason": "source_page_count_unavailable"})
            continue
        with tempfile.TemporaryDirectory(dir=args.work_dir) as directory:
            tmp = Path(directory)
            encrypted = tmp / "shared-encrypted.pdf"
            damaged = tmp / "damaged.pdf"
            damage_info = damaged_copy(source, damaged)
            encrypt_seed = execute(["qpdf", "--encrypt", PASSWORD, OWNER_PASSWORD, "256", "--", str(source), str(encrypted)], timeout_for(entry))
            operations = list(STRUCTURAL_SUPPORT)
            rng.shuffle(operations)
            for operation in operations:
                tools = list(STRUCTURAL_SUPPORT[operation])
                rng.shuffle(tools)
                for tool in tools:
                    key_tuple = (operation, tool, entry["relative_path"])
                    if key_tuple in completed:
                        continue
                    extra: dict[str, Any] = {}
                    sequence: list[list[str]] | None = None
                    if operation == "unlock":
                        output = tmp / f"unlock-{tool}.pdf"
                        commands = {
                            "wellfriend": [str(args.wellfriend), "decrypt", str(encrypted), "--password", PASSWORD, "-o", str(output), "--json"],
                            "qpdf": ["qpdf", f"--password={PASSWORD}", "--decrypt", str(encrypted), str(output)],
                            "mupdf": ["mutool", "clean", "-p", PASSWORD, "-D", str(encrypted), str(output)],
                        }
                        command, outputs = commands[tool], output
                        options = {"expected_pages": pages, "expected_encrypted": False}
                        extra["shared_encryption_seed"] = encrypt_seed
                    elif operation == "repair":
                        output = tmp / f"repair-{tool}.pdf"
                        commands = {
                            "wellfriend": [str(args.wellfriend), "repair", str(damaged), "-o", str(output), "--json"],
                            "qpdf": ["qpdf", str(damaged), str(output)],
                            "mupdf": ["mutool", "clean", str(damaged), str(output)],
                        }
                        command, outputs = commands[tool], output
                        options = {"expected_pages": pages}
                        extra["damage"] = damage_info
                    elif operation == "split" and tool == "mupdf":
                        last = min(2, pages)
                        outputs = [tmp / f"split-mupdf-{page}.pdf" for page in range(1, last + 1)]
                        sequence = [
                            ["mutool", "merge", "-o", str(output), str(source), str(page)]
                            for page, output in enumerate(outputs, start=1)
                        ]
                        command = ["mupdf-toolkit-sequence", *sum(sequence, [])]
                        options = {"expected_split_files": last}
                    elif operation == "organize" and tool == "poppler":
                        output = tmp / "organize-poppler.pdf"
                        last_pattern = tmp / "organize-poppler-last-%d.pdf"
                        first_pattern = tmp / "organize-poppler-first-%d.pdf"
                        last_page = tmp / f"organize-poppler-last-{pages}.pdf"
                        first_page = tmp / "organize-poppler-first-1.pdf"
                        sequence = [
                            ["pdfseparate", "-f", str(pages), "-l", str(pages), str(source), str(last_pattern)],
                            ["pdfseparate", "-f", "1", "-l", "1", str(source), str(first_pattern)],
                            ["pdfunite", str(last_page), str(first_page), str(first_page), str(output)],
                        ]
                        command, outputs = ["poppler-toolkit-sequence", *sum(sequence, [])], output
                        options = {"expected_pages": 3}
                    elif operation == "extract-pages" and tool == "poppler":
                        output = tmp / "extract-pages-poppler.pdf"
                        last = min(2, pages)
                        pattern = tmp / "extract-pages-poppler-%d.pdf"
                        pieces = [tmp / f"extract-pages-poppler-{page}.pdf" for page in range(1, last + 1)]
                        sequence = [
                            ["pdfseparate", "-f", "1", "-l", str(last), str(source), str(pattern)],
                            ["pdfunite", *[str(piece) for piece in pieces], str(output)],
                        ]
                        command, outputs = ["poppler-toolkit-sequence", *sum(sequence, [])], output
                        options = {"expected_pages": last}
                    elif operation == "sign" and tool == "poppler":
                        output = tmp / "sign-poppler.pdf"
                        command = [
                            "pdfsig",
                            "-add-signature",
                            "-new-signature-field-name",
                            "WFBenchmarkSignature",
                            "-nssdir",
                            f"sql:{nss_dir}",
                            "-nick",
                            "Wellfriend Benchmark",
                            str(source),
                            str(output),
                        ]
                        outputs = output
                        options = {"expected_pages": pages}
                    else:
                        command, outputs, options = structural_tasks(
                            operation, tool, source, pages, tmp, args.wellfriend, args.sdk_adapter, cert, key
                        )
                    task_timeout = timeout_for(entry, 2.0 if operation in {"resize", "flatten"} else 1.0)
                    run = execute_sequence(sequence, task_timeout) if sequence is not None else execute(command, task_timeout)
                    if isinstance(outputs, list):
                        validations = [validate_pdf(path, expected_pages=1) for path in outputs]
                        quality = {
                            "valid": len(validations) == options["expected_split_files"] and all(item.get("valid") for item in validations),
                            "outputs": validations,
                        }
                    else:
                        quality = validate_pdf(
                            outputs,
                            expected_pages=options.get("expected_pages"),
                            password=options.get("password"),
                            expect_encrypted=options.get("expected_encrypted"),
                            expect_linearized=options.get("expect_linearized", False),
                        )
                    if quality.get("valid"):
                        visual = structural_visual_checks(
                            operation,
                            source,
                            outputs,
                            pages,
                            options.get("password"),
                        )
                        quality["visual_postcondition"] = visual
                        quality["valid"] = quality["valid"] and bool(visual.get("visual_postcondition"))
                    if operation == "sign" and outputs.exists():
                        verify = execute(
                            [str(args.wellfriend), "signature-verify", str(outputs), "--field-name", "WFBenchmarkSignature", "--trust-anchor", str(cert), "--revocation", "not-checked", "--json"],
                            timeout_for(entry),
                        )
                        quality["signature_verification"] = verify
                        poppler_verify = execute(
                            ["pdfsig", "-nssdir", f"sql:{nss_dir}", str(outputs)],
                            timeout_for(entry),
                        )
                        quality["poppler_signature_verification"] = poppler_verify
                        quality["valid"] = bool(
                            quality.get("valid", False)
                            and poppler_verify["exit_code"] == 0
                            and (tool != "wellfriend" or verify["exit_code"] == 0)
                        )
                    row = common_row("structural", operation, tool, entry, command, args.corpus)
                    row.update({"source_pages": pages, "run": run, "quality": quality, "status": classify(run, quality), **extra})
                    write_row(stream, row)
                    cleanup_temp_children(tmp, (encrypted, damaged))
        print(f"structural {doc_index}/{len(entries)} {entry['relative_path']}", flush=True)


CONVERSION_SUPPORT = {
    "text": ("wellfriend", "mupdf", "poppler"),
    "html": ("wellfriend", "mupdf", "poppler"),
    "markdown": ("wellfriend",),
    "json": ("wellfriend",),
    "docx": ("wellfriend",),
    "pptx": ("wellfriend",),
    "xlsx": ("wellfriend",),
    "svg-first-page": ("wellfriend", "mupdf"),
    "postscript-first-page": ("wellfriend", "poppler"),
    "eps-first-page": ("wellfriend", "poppler"),
    "png-first-page": ("wellfriend", "mupdf", "poppler"),
    "jpeg-first-page": ("wellfriend", "poppler"),
    "tables-json": ("wellfriend",),
}


def conversion_task(operation: str, tool: str, source: Path, tmp: Path, wf: Path) -> tuple[list[str], Path, str, Path | None]:
    suffix = {
        "text": "txt", "html": "html", "markdown": "md", "json": "json", "docx": "docx",
        "pptx": "pptx", "xlsx": "xlsx", "svg-first-page": "zip" if tool == "wellfriend" else "svg",
        "postscript-first-page": "ps",
        "eps-first-page": "zip" if tool == "wellfriend" else "eps",
        "png-first-page": "png",
        "jpeg-first-page": "jpg",
        "tables-json": "json",
    }[operation]
    output = tmp / f"{operation}-{tool}.{suffix}"
    stdout_path = None
    if tool == "wellfriend":
        commands = {
            "text": [str(wf), "extract-text", str(source), "-o", str(output)],
            "html": [str(wf), "pdf-to-html", str(source), "-o", str(output), "--json"],
            "markdown": [str(wf), "pdf-to-markdown", str(source), "-o", str(output), "--json"],
            "json": [str(wf), "pdf-to-json", str(source), "-o", str(output), "--json"],
            "docx": [str(wf), "pdf-to-docx", str(source), "-o", str(output), "--layout", "flowing", "--json"],
            "pptx": [str(wf), "pdf-to-pptx", str(source), "-o", str(output), "--json"],
            "xlsx": [str(wf), "pdf-to-xlsx", str(source), "-o", str(output), "--layout", "pages", "--json"],
            "svg-first-page": [str(wf), "render", str(source), "-o", str(output), "--pages", "1", "--format", "svg", "--dpi", "72", "--json"],
            "postscript-first-page": [str(wf), "render", str(source), "-o", str(output), "--pages", "1", "--format", "ps", "--dpi", "72", "--json"],
            "eps-first-page": [str(wf), "render", str(source), "-o", str(output), "--pages", "1", "--format", "eps", "--dpi", "72", "--json"],
            "png-first-page": [str(wf), "pdf-to-jpg", str(source), "--out-dir", str(tmp / "png-first-page-wellfriend"), "--pages", "1", "--dpi", "72", "--format", "png", "--stem", "page", "--json"],
            "jpeg-first-page": [str(wf), "pdf-to-jpg", str(source), "--out-dir", str(tmp / "jpeg-first-page-wellfriend"), "--pages", "1", "--dpi", "72", "--format", "jpg", "--quality", "90", "--stem", "page", "--json"],
            "tables-json": [str(wf), "extract-tables", str(source), "-o", str(output), "--format", "json", "--structure"],
        }
        if operation in {"png-first-page", "jpeg-first-page"}:
            extension = "png" if operation == "png-first-page" else "jpg"
            output = tmp / f"{operation}-wellfriend" / f"page-001.{extension}"
        return commands[operation], output, "table-json" if operation == "tables-json" else suffix, stdout_path
    if tool == "mupdf":
        fmt = {"text": "text", "html": "html", "svg-first-page": "svg", "png-first-page": "png"}[operation]
        pages = "1" if operation in {"svg-first-page", "png-first-page"} else "1-N"
        if operation == "svg-first-page":
            pattern = tmp / "svg-first-page-mupdf-%d.svg"
            output = tmp / "svg-first-page-mupdf-1.svg"
            return ["mutool", "convert", "-F", fmt, "-o", str(pattern), str(source), pages], output, suffix, stdout_path
        if operation == "png-first-page":
            return ["mutool", "draw", "-q", "-r", "72", "-F", "png", "-o", str(output), str(source), "1"], output, suffix, stdout_path
        return ["mutool", "convert", "-F", fmt, "-o", str(output), str(source), pages], output, suffix, stdout_path
    if operation == "text":
        return ["pdftotext", "-enc", "UTF-8", str(source), str(output)], output, suffix, stdout_path
    if operation == "html":
        stdout_path = output
        return ["pdftohtml", "-q", "-s", "-i", "-stdout", str(source)], output, suffix, stdout_path
    if operation == "postscript-first-page":
        return ["pdftops", "-f", "1", "-l", "1", str(source), str(output)], output, suffix, stdout_path
    if operation == "eps-first-page":
        return ["pdftops", "-f", "1", "-l", "1", "-eps", str(source), str(output)], output, suffix, stdout_path
    if operation == "png-first-page":
        prefix = output.with_suffix("")
        return ["pdftoppm", "-q", "-f", "1", "-l", "1", "-singlefile", "-r", "72", "-png", str(source), str(prefix)], output, suffix, stdout_path
    if operation == "jpeg-first-page":
        prefix = output.with_suffix("")
        return ["pdftoppm", "-q", "-f", "1", "-l", "1", "-singlefile", "-r", "72", "-jpeg", "-jpegopt", "quality=90", str(source), str(prefix)], output, suffix, stdout_path
    raise ValueError(f"unknown conversion {operation}/{tool}")


def run_conversions(args: argparse.Namespace, entries: list[dict[str, Any]], stream: Any, completed: set[tuple[str, str, str]]) -> None:
    rng = random.Random(args.seed + 19)
    for doc_index, entry in enumerate(entries, start=1):
        source = args.corpus / entry["relative_path"]
        with tempfile.TemporaryDirectory(dir=args.work_dir) as directory:
            tmp = Path(directory)
            reference_path = tmp / "poppler-reference.txt"
            reference_run = execute(["pdftotext", "-enc", "UTF-8", str(source), str(reference_path)], timeout_for(entry))
            reference = normalized_tokens(reference_path.read_text("utf-8", errors="replace")) if reference_path.exists() else []
            operations = list(CONVERSION_SUPPORT)
            rng.shuffle(operations)
            for operation in operations:
                tools = list(CONVERSION_SUPPORT[operation])
                rng.shuffle(tools)
                for tool in tools:
                    key_tuple = (operation, tool, entry["relative_path"])
                    if key_tuple in completed:
                        continue
                    command, output, kind, stdout_path = conversion_task(operation, tool, source, tmp, args.wellfriend)
                    run = execute(command, timeout_for(entry, 3.0 if operation in {"docx", "pptx", "xlsx"} else 1.5), stdout_path=stdout_path)
                    if tool == "wellfriend" and operation in {"png-first-page", "jpeg-first-page"} and not output.exists():
                        generated = sorted(output.parent.glob(f"page-*.{kind}"))
                        if len(generated) == 1:
                            output = generated[0]
                    quality = validate_artifact(output, kind, reference)
                    row = common_row("conversion", operation, tool, entry, command, args.corpus)
                    row.update({"run": run, "quality": quality, "reference_extraction": reference_run, "status": classify(run, quality)})
                    write_row(stream, row)
                    cleanup_temp_children(tmp, (reference_path,))
        print(f"conversion {doc_index}/{len(entries)} {entry['relative_path']}", flush=True)


def edit_candidates(text: str) -> list[str]:
    numeric = re.findall(r"(?<!\d)\d{2,6}(?!\d)", text)
    alphabetic = [
        token
        for token in re.findall(r"[A-Za-z]{5,20}", text)
        if token.casefold() not in {"copyright", "https", "document", "preprint"}
        and not token.isupper()
    ]
    seen: set[str] = set()
    return [token for token in [*numeric, *alphabetic] if not (token in seen or seen.add(token))]


def same_length_replacement(token: str) -> str:
    if token.isdigit():
        final = "0" if token[-1] != "0" else "1"
        return token[:-1] + final
    final = "x" if token[-1].casefold() != "x" else "y"
    return token[:-1] + final


def raster_difference(source: Path, edited: Path, tmp: Path) -> dict[str, Any]:
    try:
        from PIL import Image, ImageChops
    except ImportError:
        return {"applicable": False, "reason": "pillow_unavailable"}
    paths = []
    for label, pdf in (("before", source), ("after", edited)):
        prefix = tmp / label
        run = subprocess.run(
            ["pdftoppm", "-q", "-f", "1", "-l", "1", "-r", "72", "-png", str(pdf), str(prefix)],
            capture_output=True,
            check=False,
        )
        found = sorted(tmp.glob(f"{label}-*.png"))
        if run.returncode != 0 or not found:
            return {"applicable": False, "reason": f"{label}_render_failed"}
        paths.append(found[0])
    with Image.open(paths[0]) as left_image, Image.open(paths[1]) as right_image:
        left, right = left_image.convert("RGB"), right_image.convert("RGB")
        if left.size != right.size:
            return {"applicable": True, "same_size": False, "before_size": left.size, "after_size": right.size}
        diff = ImageChops.difference(left, right)
        changed = sum(1 for pixel in diff.getdata() if max(pixel) > 8)
        total = left.size[0] * left.size[1]
        return {"applicable": True, "same_size": True, "changed_pixels_threshold8": changed, "changed_percentage": round(changed * 100 / max(1, total), 6)}


def run_editing(args: argparse.Namespace, entries: list[dict[str, Any]], stream: Any, completed: set[tuple[str, str, str]]) -> None:
    profiles = (
        "operator-preserving",
        "scene-source-edit",
        "paragraph-reflow",
        "paragraph-reflow-sdk",
        "geometric-reflow",
        "semantic-reflow",
    )
    for doc_index, entry in enumerate(entries, start=1):
        source = args.corpus / entry["relative_path"]
        with tempfile.TemporaryDirectory(dir=args.work_dir) as directory:
            tmp = Path(directory)
            extracted = tmp / "source-page-1.txt"
            extraction = execute([str(args.wellfriend), "extract-text", str(source), "--pages", "1", "-o", str(extracted)], timeout_for(entry))
            text = extracted.read_text("utf-8", errors="replace") if extracted.exists() else ""
            candidates = edit_candidates(text)
            for profile in profiles:
                key_tuple = (profile, "wellfriend", entry["relative_path"])
                if key_tuple in completed:
                    continue
                row = common_row("editing", profile, "wellfriend", entry, [], args.corpus)
                row["source_extraction"] = extraction
                if not candidates:
                    row.update({"status": "not_applicable", "reason": "no_page_1_editable_text_token"})
                    write_row(stream, row)
                    continue
                token = next((candidate for candidate in candidates if candidate.isalpha()), candidates[0])
                replacement = same_length_replacement(token)
                output = tmp / f"edited-{profile}.pdf"
                report = tmp / f"edited-{profile}.json"
                if profile == "operator-preserving":
                    eligibility_evidence = []
                    for candidate in candidates[:12]:
                        candidate_replacement = same_length_replacement(candidate)
                        eligibility_path = tmp / "operator-eligibility.json"
                        eligibility_run = execute(
                            [str(args.wellfriend), "edit-eligibility", str(source), "--page", "1", "--source-text", candidate, "--replacement-text", candidate_replacement, "-o", str(eligibility_path)],
                            timeout_for(entry),
                        )
                        eligible_mode = None
                        refusal_code = None
                        try:
                            eligibility_json = json.loads(eligibility_path.read_text("utf-8"))
                            eligibility_report = eligibility_json.get("report", {})
                            eligible_mode = eligibility_report.get("eligible_mode")
                            refusal = eligibility_report.get("refusal") or {}
                            refusal_code = refusal.get("code")
                        except Exception:
                            pass
                        eligibility_evidence.append({"source_text": candidate, "replacement_text": candidate_replacement, "run": eligibility_run, "eligible_mode": eligible_mode, "refusal_code": refusal_code})
                        if eligible_mode:
                            token, replacement = candidate, candidate_replacement
                            break
                    row["eligibility_probes"] = eligibility_evidence
                    command = [str(args.wellfriend), "edit-text-operator", str(source), "--page", "1", "--source-text", token, "--replacement-text", replacement, "-o", str(output), "--report", str(report)]
                elif profile == "scene-source-edit":
                    command = [str(args.wellfriend), "scene-edit-text", str(source), "--page", "1", "--source-text", token, "--replacement-text", replacement, "-o", str(output), "--report", str(report)]
                elif profile == "paragraph-reflow":
                    command = [str(args.wellfriend), "edit-text", str(source), "--pages", "1", "--query", token, "--replacement", replacement + "-edited", "--max-replacements", "1", "-o", str(output), "--json"]
                    replacement += "-edited"
                elif profile == "paragraph-reflow-sdk":
                    replacement += "-edited"
                    command = [str(args.sdk_adapter), "paragraph-reflow", str(source), str(output), token, replacement]
                elif profile == "geometric-reflow":
                    replacement += "-edited"
                    command = [str(args.wellfriend), "reflow-region", str(source), "--page", "1", "--source-text", token, "--replacement-text", replacement, "--mode", "geometric_block", "-o", str(output), "--report", str(report)]
                else:
                    replacement += "-edited"
                    command = [str(args.wellfriend), "reflow-document", str(source), "--page", "1", "--source-text", token, "--replacement-text", replacement, "--mode", "semantic_document", "--allow-page-creation", "-o", str(output), "--report", str(report)]
                row["command"] = sanitized_command(command, args.corpus)
                run = execute(command, timeout_for(entry, 3.0))
                quality = validate_pdf(output, expected_pages=qpdf_pages(source))
                if output.exists() and quality.get("valid"):
                    reopened = tmp / f"reopened-{profile}.txt"
                    reopen_run = execute([str(args.wellfriend), "extract-text", str(output), "--pages", "1", "-o", str(reopened)], timeout_for(entry))
                    reopened_text = reopened.read_text("utf-8", errors="replace") if reopened.exists() else ""
                    quality["reopen_extraction"] = reopen_run
                    quality["replacement_present_after_reopen"] = replacement.casefold() in reopened_text.casefold()
                    quality["source_occurrences_before"] = text.casefold().count(token.casefold())
                    quality["source_occurrences_after"] = reopened_text.casefold().count(token.casefold())
                    quality["visual_change"] = raster_difference(source, output, tmp)
                    quality["valid"] = quality["valid"] and quality["replacement_present_after_reopen"]
                row.update({"source_text": token, "replacement_text": replacement, "run": run, "quality": quality, "status": classify(run, quality)})
                write_row(stream, row)
                cleanup_temp_children(tmp, (extracted,))

            vector_key = ("vector-duplicate", "wellfriend", entry["relative_path"])
            if vector_key not in completed:
                row = common_row("editing", "vector-duplicate", "wellfriend", entry, [], args.corpus)
                inventory_path = tmp / "vector-inventory.json"
                inventory_run = execute(
                    [str(args.wellfriend), "vector-list", str(source), "--page", "1", "-o", str(inventory_path)],
                    timeout_for(entry, 2.0),
                )
                row["inventory_run"] = inventory_run
                objects: list[dict[str, Any]] = []
                try:
                    inventory_json = json.loads(inventory_path.read_text("utf-8", errors="strict"))
                    if isinstance(inventory_json, dict) and isinstance(inventory_json.get("objects"), list):
                        objects = [item for item in inventory_json["objects"] if isinstance(item, dict)]
                except Exception:
                    pass
                candidates = [
                    item for item in objects
                    if item.get("stable_id") and not item.get("clipping_path")
                ]
                if not candidates:
                    row.update({"status": "not_applicable", "reason": "no_page_1_non_clipping_vector_object"})
                    write_row(stream, row)
                else:
                    candidate = candidates[0]
                    output = tmp / "edited-vector-duplicate.pdf"
                    report = tmp / "edited-vector-duplicate.json"
                    command = [
                        str(args.wellfriend), "vector-duplicate", str(source),
                        "--page", "1", "--id", str(candidate["stable_id"]),
                        "--dx", "6", "--dy=-6", "-o", str(output), "--report", str(report),
                    ]
                    row["command"] = sanitized_command(command, args.corpus)
                    run = execute(command, timeout_for(entry, 3.0))
                    quality = validate_pdf(output, expected_pages=qpdf_pages(source))
                    if output.exists() and quality.get("valid"):
                        reopened_inventory = tmp / "vector-inventory-after.json"
                        reopen_run = execute(
                            [str(args.wellfriend), "vector-list", str(output), "--page", "1", "-o", str(reopened_inventory)],
                            timeout_for(entry, 2.0),
                        )
                        after_count = None
                        try:
                            after_json = json.loads(reopened_inventory.read_text("utf-8", errors="strict"))
                            after_count = len(after_json.get("objects", [])) if isinstance(after_json, dict) and isinstance(after_json.get("objects"), list) else None
                        except Exception:
                            pass
                        quality["reopen_inventory"] = reopen_run
                        quality["objects_before"] = len(objects)
                        quality["objects_after"] = after_count
                        quality["object_count_increased"] = after_count is not None and after_count > len(objects)
                        quality["visual_change"] = raster_difference(source, output, tmp)
                        quality["valid"] = bool(
                            quality["valid"]
                            and reopen_run["exit_code"] == 0
                            and quality["object_count_increased"]
                            and (quality["visual_change"].get("changed_pixels_threshold8") or 0) > 0
                        )
                    row.update({"stable_id": candidate["stable_id"], "run": run, "quality": quality, "status": classify(run, quality)})
                    write_row(stream, row)
                cleanup_temp_children(tmp, (extracted,))
        print(f"editing {doc_index}/{len(entries)} {entry['relative_path']}", flush=True)


def run_pdfa_validation(args: argparse.Namespace, entries: list[dict[str, Any]], stream: Any, completed: set[tuple[str, str, str]]) -> None:
    for index, entry in enumerate(entries, start=1):
        source = args.corpus / entry["relative_path"]
        with tempfile.TemporaryDirectory(dir=args.work_dir) as directory:
            tmp = Path(directory)
            for tool, command in (
                ("wellfriend", [str(args.wellfriend), "pdfa-validate", str(source), "--target", "PDF/A-2B", "--json", "--fail-on", "never"]),
                ("verapdf", [str(args.verapdf), "--format", "json", "--flavour", "2b", "--maxfailures", "100", str(source)]),
            ):
                key_tuple = ("pdfa-2b-validation", tool, entry["relative_path"])
                if key_tuple in completed:
                    continue
                report_path = tmp / f"{tool}.json"
                run = execute(command, timeout_for(entry, 2.0), stdout_path=report_path)
                parsed = None
                try:
                    parsed = json.loads(report_path.read_text("utf-8", errors="strict"))
                except Exception:
                    pass
                interesting: dict[str, list[Any]] = {}
                if parsed is not None:
                    def walk(value: Any) -> None:
                        if isinstance(value, dict):
                            for key, child in value.items():
                                if key.casefold() in {"compliant", "iscompliant", "passedchecks", "failedchecks", "status"} and isinstance(child, (str, int, float, bool, type(None))):
                                    interesting.setdefault(key, []).append(child)
                                walk(child)
                        elif isinstance(value, list):
                            for child in value:
                                walk(child)
                    walk(parsed)
                quality = {
                    "valid": run["exit_code"] in (0, 1) and parsed is not None,
                    "json_parsed": parsed is not None,
                    "report_bytes": report_path.stat().st_size if report_path.exists() else 0,
                    "report_sha256": hashlib.sha256(report_path.read_bytes()).hexdigest() if report_path.exists() else None,
                    "outcome_fields": interesting,
                }
                compliance_values = [
                    value
                    for key, values in interesting.items()
                    if key.casefold() in {"compliant", "iscompliant"}
                    for value in values
                    if isinstance(value, bool)
                ]
                quality["reported_compliant"] = (
                    all(compliance_values) if compliance_values else None
                )
                row = common_row("pdfa-validation", "pdfa-2b-validation", tool, entry, command, args.corpus)
                row.update({"run": run, "quality": quality, "status": "pass" if quality["valid"] else classify(run, quality)})
                write_row(stream, row)
        print(f"pdfa-validation {index}/{len(entries)} {entry['relative_path']}", flush=True)


def run_pdfa_conversion(args: argparse.Namespace, entries: list[dict[str, Any]], stream: Any, completed: set[tuple[str, str, str]]) -> None:
    for index, entry in enumerate(entries, start=1):
        source = args.corpus / entry["relative_path"]
        with tempfile.TemporaryDirectory(dir=args.work_dir) as directory:
            tmp = Path(directory)
            for tool in ("wellfriend", "ghostscript"):
                key_tuple = ("pdfa-2b-conversion", tool, entry["relative_path"])
                if key_tuple in completed:
                    continue
                output = tmp / f"pdfa-{tool}.pdf"
                if tool == "wellfriend":
                    command = [str(args.sdk_adapter), "pdfa-2b", str(source), str(output)]
                else:
                    command = ["gs", "-dBATCH", "-dNOPAUSE", "-dSAFER", "-sDEVICE=pdfwrite", "-dPDFA=2", "-dPDFACompatibilityPolicy=1", "-sColorConversionStrategy=RGB", f"-sOutputFile={output}", str(source)]
                run = execute(command, timeout_for(entry, 4.0))
                quality = validate_pdf(output, expected_pages=qpdf_pages(source))
                if output.exists() and quality.get("valid"):
                    before = raster_fingerprint(source, 1)
                    after = raster_fingerprint(output, 1)
                    quality["first_page_visual"] = {
                        "source": before,
                        "converted": after,
                        "exact_raster_match": before.get("sha256") == after.get("sha256"),
                    }
                    vera_report = tmp / f"pdfa-{tool}-verapdf.json"
                    vera = execute(
                        [str(args.verapdf), "--format", "json", "--flavour", "2b", "--maxfailures", "100", str(output)],
                        timeout_for(entry, 2.0),
                        stdout_path=vera_report,
                    )
                    quality["verapdf"] = vera
                    vera_parsed = None
                    try:
                        vera_parsed = json.loads(vera_report.read_text("utf-8", errors="strict"))
                    except Exception:
                        pass
                    quality["verapdf_completed"] = vera["exit_code"] in (0, 1) and vera_parsed is not None
                    quality["verapdf_report_bytes"] = vera_report.stat().st_size if vera_report.exists() else 0
                    quality["verapdf_report_sha256"] = hashlib.sha256(vera_report.read_bytes()).hexdigest() if vera_report.exists() else None
                    outcome_fields: dict[str, list[Any]] = {}
                    def collect(value: Any) -> None:
                        if isinstance(value, dict):
                            for key, child in value.items():
                                if key.casefold() in {"compliant", "iscompliant", "passedchecks", "failedchecks", "status"} and isinstance(child, (str, int, float, bool, type(None))):
                                    outcome_fields.setdefault(key, []).append(child)
                                collect(child)
                        elif isinstance(value, list):
                            for child in value:
                                collect(child)
                    if vera_parsed is not None:
                        collect(vera_parsed)
                    quality["verapdf_outcome_fields"] = outcome_fields
                    compliance_values = [
                        value
                        for key, values in outcome_fields.items()
                        if key.casefold() in {"compliant", "iscompliant"}
                        for value in values
                        if isinstance(value, bool)
                    ]
                    quality["verapdf_compliant"] = (
                        all(compliance_values) if compliance_values else None
                    )
                    quality["valid"] = bool(
                        quality.get("valid")
                        and quality["verapdf_completed"]
                        and quality["verapdf_compliant"] is True
                    )
                row = common_row("pdfa-conversion", "pdfa-2b-conversion", tool, entry, command, args.corpus)
                row.update({"run": run, "quality": quality, "status": classify(run, quality)})
                write_row(stream, row)
                cleanup_temp_children(tmp)
        print(f"pdfa-conversion {index}/{len(entries)} {entry['relative_path']}", flush=True)


def read_completed(path: Path, phase: str) -> set[tuple[str, str, str]]:
    completed: set[tuple[str, str, str]] = set()
    if not path.exists():
        return completed
    for line in path.read_text("utf-8", errors="replace").splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if row.get("phase") == phase:
            completed.add((str(row.get("operation")), str(row.get("tool")), str(row.get("relative_path"))))
    return completed


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", required=True, choices=("structural", "conversion", "editing", "pdfa-validation", "pdfa-conversion"))
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--wellfriend", type=Path, required=True)
    parser.add_argument("--sdk-adapter", type=Path, required=True)
    parser.add_argument("--verapdf", type=Path, default=Path("/root/verapdf/verapdf"))
    parser.add_argument("--seed", type=int, default=20261003)
    parser.add_argument("--limit", type=int, default=150)
    args = parser.parse_args()

    args.output.mkdir(parents=True, exist_ok=True)
    args.work_dir.mkdir(parents=True, exist_ok=True)
    entries = json.loads(args.manifest.read_text("utf-8"))["files"][: args.limit]
    if len(entries) != args.limit:
        raise SystemExit(f"expected {args.limit} corpus entries, found {len(entries)}")
    raw_path = args.output / f"{args.phase}.jsonl"
    completed = read_completed(raw_path, args.phase)
    with raw_path.open("a", encoding="utf-8", newline="\n") as stream:
        if args.phase == "structural":
            run_structural(args, entries, stream, completed)
        elif args.phase == "conversion":
            run_conversions(args, entries, stream, completed)
        elif args.phase == "editing":
            run_editing(args, entries, stream, completed)
        elif args.phase == "pdfa-validation":
            run_pdfa_validation(args, entries, stream, completed)
        else:
            run_pdfa_conversion(args, entries, stream, completed)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
