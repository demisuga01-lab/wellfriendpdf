#!/usr/bin/env python3
"""All-page, no-raster-retention renderer timing and fidelity benchmark.

Timing runs one renderer at a time and drains concatenated raw PPM frames to
/dev/null. Fidelity runs the four renderers together only to keep corresponding
pages in memory; those concurrent observations are never used as performance
measurements. Every page receives dimension and downsampled perceptual metrics.
First/middle/last pages of each document additionally receive full-resolution
pixel metrics.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import random
import subprocess
import time
from pathlib import Path
from typing import Any, BinaryIO

import numpy as np
from PIL import Image

try:
    from skimage.metrics import structural_similarity
except ImportError:  # pragma: no cover - recorded in environment evidence.
    structural_similarity = None


SCHEMA = "wellfriendpdf.render-stream.v1"
ENGINES = ("wellfriend", "pdfium", "mupdf", "poppler")


def utc_now() -> str:
    import datetime

    return datetime.datetime.now(datetime.UTC).isoformat()


def read_ppm_frame(stream: BinaryIO) -> tuple[int, int, bytes] | None:
    magic = stream.readline()
    if magic == b"":
        return None
    if magic != b"P6\n":
        raise ValueError(f"unexpected PPM magic: {magic[:40]!r}")
    dimensions = stream.readline().strip().split()
    maximum = stream.readline().strip()
    if len(dimensions) != 2 or maximum != b"255":
        raise ValueError("invalid streamed PPM header")
    width, height = int(dimensions[0]), int(dimensions[1])
    expected = width * height * 3
    chunks = bytearray()
    while len(chunks) < expected:
        chunk = stream.read(expected - len(chunks))
        if not chunk:
            raise EOFError(f"short PPM frame: {len(chunks)}/{expected}")
        chunks.extend(chunk)
    return width, height, bytes(chunks)


def thumbnail_array(width: int, height: int, pixels: bytes, edge: int) -> np.ndarray:
    image = Image.frombytes("RGB", (width, height), pixels)
    image.thumbnail((edge, edge), Image.Resampling.LANCZOS)
    canvas = Image.new("RGB", (edge, edge), "white")
    left = (edge - image.width) // 2
    top = (edge - image.height) // 2
    canvas.paste(image, (left, top))
    return np.asarray(canvas, dtype=np.uint8)


def pixel_metrics(left: np.ndarray, right: np.ndarray) -> dict[str, Any]:
    if left.shape != right.shape:
        return {"same_shape": False, "left_shape": list(left.shape), "right_shape": list(right.shape)}
    delta = left.astype(np.int16) - right.astype(np.int16)
    absolute = np.abs(delta)
    mse = float(np.mean(delta.astype(np.float64) ** 2))
    mae = float(np.mean(absolute))
    changed = float(np.mean(np.max(absolute, axis=2) > 8) * 100.0)
    psnr = None if mse == 0 else 20.0 * math.log10(255.0 / math.sqrt(mse))
    result: dict[str, Any] = {
        "same_shape": True,
        "mae": round(mae, 6),
        "rmse": round(math.sqrt(mse), 6),
        "psnr_db": None if psnr is None else round(psnr, 6),
        "changed_pixel_threshold8_percentage": round(changed, 6),
        "max_channel_delta": int(absolute.max(initial=0)),
    }
    if structural_similarity is not None:
        result["ssim"] = round(
            float(structural_similarity(left, right, channel_axis=2, data_range=255)), 8
        )
    return result


def frame_metrics(
    frames: dict[str, tuple[int, int, bytes]],
    detailed: bool,
    thumbnail_edge: int,
) -> dict[str, Any]:
    thumbnails = {
        engine: thumbnail_array(width, height, pixels, thumbnail_edge)
        for engine, (width, height, pixels) in frames.items()
    }
    pairs: dict[str, Any] = {}
    for left_index, left_name in enumerate(ENGINES):
        for right_name in ENGINES[left_index + 1 :]:
            key = f"{left_name}_vs_{right_name}"
            pair = {
                "dimensions_equal": frames[left_name][:2] == frames[right_name][:2],
                "thumbnail": pixel_metrics(thumbnails[left_name], thumbnails[right_name]),
            }
            if detailed and frames[left_name][:2] == frames[right_name][:2]:
                width, height, left_pixels = frames[left_name]
                right_pixels = frames[right_name][2]
                left = np.frombuffer(left_pixels, dtype=np.uint8).reshape(height, width, 3)
                right = np.frombuffer(right_pixels, dtype=np.uint8).reshape(height, width, 3)
                pair["full_resolution"] = pixel_metrics(left, right)
            pairs[key] = pair
    return {
        "dimensions": {engine: list(frames[engine][:2]) for engine in ENGINES},
        "rgb_sha256": {engine: hashlib.sha256(frames[engine][2]).hexdigest() for engine in ENGINES},
        "thumbnail_edge": thumbnail_edge,
        "detailed_full_resolution": detailed,
        "pairs": pairs,
    }


def command_for(engine: str, adapter_dir: Path, pdf: Path, dpi: int) -> list[str]:
    return [str(adapter_dir / f"stream-{engine}"), "--stream-pages", str(pdf), str(dpi)] if engine != "wellfriend" else [str(adapter_dir / "stream-wellfriend"), str(pdf), str(dpi)]


def timeout_for(entry: dict[str, Any]) -> int:
    mib = int(entry["bytes"]) / (1024 * 1024)
    return max(600, min(21600, int(600 + mib * 45)))


def completed_keys(path: Path, phase: str) -> set[tuple[str, str]]:
    result: set[tuple[str, str]] = set()
    if not path.exists():
        return result
    for line in path.read_text("utf-8", errors="replace").splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if row.get("phase") != phase:
            continue
        result.add((str(row.get("relative_path")), str(row.get("engine", "all"))))
    return result


def run_timing(args: argparse.Namespace, entries: list[dict[str, Any]], raw: Path) -> None:
    completed = completed_keys(raw, "timing")
    rng = random.Random(args.seed)
    with raw.open("a", encoding="utf-8", newline="\n") as stream:
        for index, entry in enumerate(entries, start=1):
            pdf = args.corpus / entry["relative_path"]
            engines = list(ENGINES)
            rng.shuffle(engines)
            for engine in engines:
                if (entry["relative_path"], engine) in completed:
                    continue
                command = command_for(engine, args.adapter_dir, pdf, args.dpi)
                started = time.perf_counter_ns()
                try:
                    completed_process = subprocess.run(
                        command,
                        stdin=subprocess.DEVNULL,
                        stdout=subprocess.DEVNULL,
                        stderr=subprocess.PIPE,
                        timeout=timeout_for(entry),
                        check=False,
                    )
                    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000.0
                    row = {
                        "schema_version": SCHEMA,
                        "timestamp_utc": utc_now(),
                        "phase": "timing",
                        "engine": engine,
                        "relative_path": entry["relative_path"],
                        "input_sha256": entry["sha256"],
                        "input_bytes": entry["bytes"],
                        "dpi": args.dpi,
                        "page_count": entry.get("page_count"),
                        "elapsed_ms": round(elapsed_ms, 6),
                        "exit_code": completed_process.returncode,
                        "status": "pass" if completed_process.returncode == 0 else "fail",
                        "stderr_tail": completed_process.stderr.decode("utf-8", "replace")[-4000:],
                        "contract": "open once, render every page to RGB8 PPM frames, drain to /dev/null",
                    }
                except subprocess.TimeoutExpired as error:
                    row = {
                        "schema_version": SCHEMA,
                        "timestamp_utc": utc_now(),
                        "phase": "timing",
                        "engine": engine,
                        "relative_path": entry["relative_path"],
                        "input_sha256": entry["sha256"],
                        "input_bytes": entry["bytes"],
                        "dpi": args.dpi,
                        "page_count": entry.get("page_count"),
                        "elapsed_ms": round((time.perf_counter_ns() - started) / 1_000_000.0, 6),
                        "exit_code": None,
                        "status": "timeout",
                        "stderr_tail": (error.stderr or b"").decode("utf-8", "replace")[-4000:],
                    }
                stream.write(json.dumps(row, sort_keys=True) + "\n")
                stream.flush()
            print(f"render-timing {index}/{len(entries)} {entry['relative_path']}", flush=True)


def run_quality(args: argparse.Namespace, entries: list[dict[str, Any]], raw: Path) -> None:
    completed = completed_keys(raw, "quality")
    with raw.open("a", encoding="utf-8", newline="\n") as stream:
        for index, entry in enumerate(entries, start=1):
            if (entry["relative_path"], "all") in completed:
                continue
            pdf = args.corpus / entry["relative_path"]
            expected_pages = int(entry["page_count"])
            detailed_pages = {1, max(1, (expected_pages + 1) // 2), expected_pages}
            document_errors: dict[str, str] = {}
            failures_by_reference: dict[str, int] = {}
            attempts_by_reference: dict[str, int] = {}

            # Render Wellfriend once. Retain only 96x96 thumbnails, dimensions,
            # hashes, and three full-resolution frames for detailed checks.
            wellfriend = subprocess.Popen(
                command_for("wellfriend", args.adapter_dir, pdf, args.dpi),
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                bufsize=1024 * 1024,
            )
            wf_pages: list[dict[str, Any]] = []
            try:
                assert wellfriend.stdout is not None
                for page_number in range(1, expected_pages + 1):
                    frame = read_ppm_frame(wellfriend.stdout)
                    if frame is None:
                        raise EOFError("Wellfriend ended before expected page count")
                    width, height, pixels = frame
                    wf_pages.append(
                        {
                            "width": width,
                            "height": height,
                            "sha256": hashlib.sha256(pixels).hexdigest(),
                            "thumbnail": thumbnail_array(width, height, pixels, args.thumbnail_edge),
                            "pixels": pixels if page_number in detailed_pages else None,
                        }
                    )
                assert wellfriend.stderr is not None
                stderr = wellfriend.stderr.read().decode("utf-8", "replace")
                return_code = wellfriend.wait(timeout=30)
                if return_code != 0:
                    document_errors["wellfriend"] = stderr[-4000:]
            except Exception as error:  # noqa: BLE001
                document_errors["wellfriend"] = f"{error.__class__.__name__}:{error}"
            finally:
                if wellfriend.poll() is None:
                    wellfriend.kill()
                wellfriend.wait()

            if len(wf_pages) == expected_pages and "wellfriend" not in document_errors:
                for reference in ("pdfium", "mupdf", "poppler"):
                    process = subprocess.Popen(
                        command_for(reference, args.adapter_dir, pdf, args.dpi),
                        stdin=subprocess.DEVNULL,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        bufsize=1024 * 1024,
                    )
                    attempted = 0
                    failed = 0
                    reference_error = None
                    try:
                        assert process.stdout is not None
                        for page_number in range(1, expected_pages + 1):
                            try:
                                frame = read_ppm_frame(process.stdout)
                                if frame is None:
                                    raise EOFError("renderer ended before expected page count")
                                width, height, pixels = frame
                                wf_page = wf_pages[page_number - 1]
                                thumb = thumbnail_array(width, height, pixels, args.thumbnail_edge)
                                detailed = page_number in detailed_pages
                                metrics: dict[str, Any] = {
                                    "dimensions_equal": (wf_page["width"], wf_page["height"]) == (width, height),
                                    "wellfriend_dimensions": [wf_page["width"], wf_page["height"]],
                                    "reference_dimensions": [width, height],
                                    "wellfriend_rgb_sha256": wf_page["sha256"],
                                    "reference_rgb_sha256": hashlib.sha256(pixels).hexdigest(),
                                    "thumbnail_edge": args.thumbnail_edge,
                                    "thumbnail": pixel_metrics(wf_page["thumbnail"], thumb),
                                    "detailed_full_resolution": detailed,
                                }
                                if detailed and metrics["dimensions_equal"]:
                                    left = np.frombuffer(wf_page["pixels"], dtype=np.uint8).reshape(height, width, 3)
                                    right = np.frombuffer(pixels, dtype=np.uint8).reshape(height, width, 3)
                                    metrics["full_resolution"] = pixel_metrics(left, right)
                                status = "pass"
                                error_text = None
                            except Exception as error:  # noqa: BLE001
                                status = "fail"
                                error_text = f"{error.__class__.__name__}:{error}"
                                metrics = {}
                                failed += 1
                                reference_error = error_text
                            attempted += 1
                            row = {
                                "schema_version": SCHEMA,
                                "timestamp_utc": utc_now(),
                                "phase": "quality-page",
                                "reference": reference,
                                "relative_path": entry["relative_path"],
                                "input_sha256": entry["sha256"],
                                "page_number": page_number,
                                "page_count": expected_pages,
                                "dpi": args.dpi,
                                "status": status,
                                "error": error_text,
                                "metrics": metrics,
                            }
                            stream.write(json.dumps(row, sort_keys=True) + "\n")
                            if page_number % 25 == 0:
                                stream.flush()
                            if reference_error is not None:
                                break
                        if reference_error is not None and process.poll() is None:
                            process.kill()
                        assert process.stderr is not None
                        stderr = process.stderr.read().decode("utf-8", "replace")
                        return_code = process.wait(timeout=30)
                        if return_code != 0:
                            document_errors[reference] = stderr[-4000:] or reference_error or f"exit {return_code}"
                    finally:
                        if process.poll() is None:
                            process.kill()
                        process.wait()
                    attempts_by_reference[reference] = attempted
                    failures_by_reference[reference] = failed
            else:
                for reference in ("pdfium", "mupdf", "poppler"):
                    attempts_by_reference[reference] = 0
                    failures_by_reference[reference] = expected_pages
            summary = {
                "schema_version": SCHEMA,
                "timestamp_utc": utc_now(),
                "phase": "quality",
                "engine": "all",
                "relative_path": entry["relative_path"],
                "input_sha256": entry["sha256"],
                "dpi": args.dpi,
                "expected_pages": expected_pages,
                "wellfriend_pages": len(wf_pages),
                "attempted_pages_by_reference": attempts_by_reference,
                "failed_pages_by_reference": failures_by_reference,
                "renderer_errors": document_errors,
                "status": "pass" if all(value == 0 for value in failures_by_reference.values()) and not document_errors else "fail",
                "contract": f"Wellfriend rendered once; each reference rendered sequentially; all pages compared using {args.thumbnail_edge}x{args.thumbnail_edge} thumbnails; first/middle/last also full resolution",
            }
            stream.write(json.dumps(summary, sort_keys=True) + "\n")
            stream.flush()
            print(f"render-quality {index}/{len(entries)} {entry['relative_path']} wf_pages={len(wf_pages)} failures={sum(failures_by_reference.values())}", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", required=True, choices=("timing", "quality"))
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--adapter-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--thumbnail-edge", type=int, default=96)
    parser.add_argument("--seed", type=int, default=20261003)
    parser.add_argument("--limit", type=int, default=150)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    manifest = json.loads(args.manifest.read_text("utf-8"))
    entries = manifest["files"][: args.limit]
    inventory_path = args.output / "page-inventory.json"
    inventory = json.loads(inventory_path.read_text("utf-8")) if inventory_path.exists() else {}
    if isinstance(inventory, list):
        page_counts = {
            str(row.get("relative_path")): int(row["page_count"])
            for row in inventory
            if row.get("status") == "ok" and isinstance(row.get("page_count"), int)
        }
    else:
        page_counts = inventory.get("page_counts", inventory)
    for entry in entries:
        value = page_counts.get(entry["relative_path"])
        if isinstance(value, dict):
            value = value.get("page_count")
        if not isinstance(value, int):
            raise SystemExit(f"missing page count for {entry['relative_path']} in {inventory_path}")
        entry["page_count"] = value
    raw = args.output / f"render-{args.phase}.jsonl"
    if args.phase == "timing":
        run_timing(args, entries, raw)
    else:
        run_quality(args, entries, raw)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
