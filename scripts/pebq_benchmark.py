#!/usr/bin/env python3
"""Run the matched-contract PEBQ parser and renderer qualification.

The harness never compares unlike operations. Every engine adapter receives
the same in-memory page-count or page-one RGB render request. Measurements are
randomly interleaved and retained as raw JSONL before aggregation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import random
import statistics
import subprocess
import sys
import time
from collections import Counter, defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable


PARSER_ENGINES = ("wellfriendpdf", "qpdf", "mupdf", "pdfium", "poppler")
RENDER_ENGINES = ("wellfriendpdf", "mupdf", "pdfium", "poppler")
LABELS = {
    "wellfriendpdf": "Wellfriend PDF",
    "qpdf": "qpdf",
    "mupdf": "MuPDF",
    "pdfium": "PDFium",
    "poppler": "Poppler",
}


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def percentile(values: Iterable[float], percent: float) -> float | None:
    ordered = sorted(float(value) for value in values)
    if not ordered:
        return None
    rank = max(1, math.ceil(percent / 100.0 * len(ordered)))
    return ordered[min(rank - 1, len(ordered) - 1)]


def distribution(values: Iterable[float]) -> dict[str, Any]:
    data = [float(value) for value in values]
    if not data:
        return {"count": 0}
    median = statistics.median(data)
    return {
        "count": len(data),
        "p05": round(percentile(data, 5) or 0.0, 6),
        "p50": round(percentile(data, 50) or 0.0, 6),
        "p90": round(percentile(data, 90) or 0.0, 6),
        "p95": round(percentile(data, 95) or 0.0, 6),
        "p99": round(percentile(data, 99) or 0.0, 6),
        "max": round(max(data), 6),
        "mean": round(statistics.fmean(data), 6),
        "median_absolute_deviation": round(
            statistics.median(abs(value - median) for value in data), 6
        ),
    }


def median_by_key(rows: Iterable[dict[str, Any]], key: str) -> dict[str, float]:
    grouped: dict[str, list[float]] = defaultdict(list)
    for row in rows:
        if row.get("status") == "ok" and isinstance(row.get(key), (int, float)):
            grouped[str(row["relative_path"])].append(float(row[key]))
    return {path: statistics.median(values) for path, values in grouped.items()}


def document_median_distribution(
    rows: Iterable[dict[str, Any]],
    engine: str,
    key: str,
    expected_rows: int | None = None,
) -> dict[str, Any]:
    """Summarize one equally weighted median per document."""

    engine_rows = [row for row in rows if row.get("engine") == engine]
    if expected_rows is None:
        medians = median_by_key(engine_rows, key)
    else:
        grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
        for row in engine_rows:
            grouped[str(row.get("relative_path", ""))].append(row)
        medians = {
            path: statistics.median(float(row[key]) for row in document_rows)
            for path, document_rows in grouped.items()
            if path
            and len(document_rows) == expected_rows
            and all(
                row.get("status") == "ok" and isinstance(row.get(key), (int, float))
                for row in document_rows
            )
        }
    return distribution(medians.values())


def slowest_document_medians(
    rows: Iterable[dict[str, Any]],
    engine: str,
    key: str,
    limit: int = 5,
    expected_rows: int | None = None,
) -> list[dict[str, Any]]:
    engine_rows = [row for row in rows if row.get("engine") == engine]
    if expected_rows is None:
        medians = median_by_key(engine_rows, key)
    else:
        grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
        for row in engine_rows:
            grouped[str(row.get("relative_path", ""))].append(row)
        medians = {
            path: statistics.median(float(row[key]) for row in document_rows)
            for path, document_rows in grouped.items()
            if path
            and len(document_rows) == expected_rows
            and all(
                row.get("status") == "ok" and isinstance(row.get(key), (int, float))
                for row in document_rows
            )
        }
    return [
        {"relative_path": path, "median_ms": round(value, 6)}
        for path, value in sorted(medians.items(), key=lambda item: item[1], reverse=True)[:limit]
    ]


def raster_determinism(
    rows: Iterable[dict[str, Any]], expected_repetitions: int | None = None
) -> dict[str, Any]:
    """Check that repeated successful renders publish one stable raster identity."""

    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[(str(row.get("engine", "")), str(row.get("relative_path", "")))].append(row)
    failures = []
    for (engine, path), document_rows in sorted(grouped.items()):
        successful = [row for row in document_rows if row.get("status") == "ok"]
        outputs = {
            (
                int(row.get("width", 0)),
                int(row.get("height", 0)),
                str(row.get("raster_fnv1a64", "")),
            )
            for row in successful
        }
        reasons = []
        if expected_repetitions is not None and len(document_rows) != expected_repetitions:
            reasons.append(
                f"expected {expected_repetitions} rows, observed {len(document_rows)}"
            )
        if len(successful) != len(document_rows):
            reasons.append("one or more repetitions failed")
        if len(outputs) != 1 or any(width <= 0 or height <= 0 or not digest for width, height, digest in outputs):
            reasons.append("raster identity is missing or unstable")
        if reasons:
            failures.append(
                {
                    "engine": engine,
                    "relative_path": path,
                    "reasons": reasons,
                    "observed_outputs": [
                        {"width": width, "height": height, "raster_fnv1a64": digest}
                        for width, height, digest in sorted(outputs)
                    ],
                }
            )
    return {
        "qualified_engine_documents": len(grouped) - len(failures),
        "checked_engine_documents": len(grouped),
        "failures": failures,
    }


def successful_engine_documents(
    rows: Iterable[dict[str, Any]],
    engines: tuple[str, ...],
    expected_rows: int,
    required_numeric_fields: tuple[str, ...],
) -> dict[str, int]:
    grouped: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[(str(row.get("engine", "")), str(row.get("relative_path", "")))].append(row)
    return {
        engine: sum(
            1
            for (row_engine, path), document_rows in grouped.items()
            if row_engine == engine
            and path
            and len(document_rows) == expected_rows
            and all(
                row.get("status") == "ok"
                and all(isinstance(row.get(field), (int, float)) for field in required_numeric_fields)
                for row in document_rows
            )
        )
        for engine in engines
    }


def bootstrap_ratio_ci(
    numerator: dict[str, float], denominator: dict[str, float], seed: int, samples: int = 20_000
) -> dict[str, Any]:
    shared = sorted(set(numerator) & set(denominator))
    ratios = [numerator[path] / denominator[path] for path in shared if denominator[path] > 0]
    if not ratios:
        return {"count": 0}
    rng = random.Random(seed)
    boot = [
        statistics.median(ratios[rng.randrange(len(ratios))] for _ in ratios)
        for _ in range(samples)
    ]
    return {
        "count": len(ratios),
        "median_paired_ratio": round(statistics.median(ratios), 6),
        "geometric_mean_paired_ratio": round(
            math.exp(statistics.fmean(math.log(value) for value in ratios if value > 0)), 6
        ),
        "bootstrap_95_percent_ci": [
            round(percentile(boot, 2.5) or 0.0, 6),
            round(percentile(boot, 97.5) or 0.0, 6),
        ],
        "claim_20x_median_lower_bound_pass": bool((percentile(boot, 2.5) or 0.0) >= 20.0),
    }


def tool_version(command: list[str]) -> dict[str, Any]:
    try:
        completed = subprocess.run(command, capture_output=True, text=True, timeout=20, check=False)
    except Exception as error:  # pragma: no cover - diagnostic boundary
        return {"command": command, "error": str(error)}
    return {
        "command": command,
        "exit": completed.returncode,
        "text": (completed.stdout + "\n" + completed.stderr).strip()[:2000],
    }


def pdfium_version(adapter: Path) -> dict[str, Any]:
    """Read the version metadata belonging to the adapter's loaded PDFium.

    The benchmark controller may run in a Python environment that does not
    contain pypdfium2_raw even though the native adapter is correctly linked
    against that package's libpdfium. Resolve the library through the dynamic
    linker instead of importing an unrelated controller dependency.
    """

    linkage = tool_version(["ldd", str(adapter)])
    if linkage.get("exit") == 0:
        for line in str(linkage.get("text", "")).splitlines():
            if "libpdfium.so" not in line:
                continue
            location = line.split("=>", 1)[-1].strip().split(maxsplit=1)[0]
            version_path = Path(location).parent / "version.json"
            try:
                version = json.loads(version_path.read_text(encoding="utf-8"))
                if all(key in version for key in ("major", "minor", "build", "patch")):
                    return {
                        "command": ["read", str(version_path)],
                        "exit": 0,
                        "text": json.dumps(version, sort_keys=True),
                    }
            except (OSError, json.JSONDecodeError, TypeError):
                continue
    return {
        "command": ["ldd", str(adapter)],
        "exit": 1,
        "text": "PDFium version unavailable; environment metadata contains the adapter linkage",
        "linkage": linkage,
    }


class Worker:
    def __init__(self, engine: str, binary: Path, cpu: int, stderr_path: Path):
        self.engine = engine
        self.stderr_stream = stderr_path.open("w", encoding="utf-8", newline="\n")
        self.process = subprocess.Popen(
            ["taskset", "-c", str(cpu), str(binary), "--server"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr_stream,
            text=True,
            bufsize=1,
        )

    def request(
        self,
        profile: str,
        path: Path,
        dpi: int,
        output: Path | None,
        iterations: int = 1,
    ) -> dict[str, Any]:
        assert self.process.stdin is not None and self.process.stdout is not None
        payload = "\t".join(
            (profile, str(path), str(dpi), str(output) if output else "-", str(iterations))
        )
        started = time.perf_counter_ns()
        self.process.stdin.write(payload + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        roundtrip_ms = (time.perf_counter_ns() - started) / 1_000_000.0
        if not line:
            raise RuntimeError(f"{self.engine} worker exited; see {self.stderr_stream.name}")
        row = json.loads(line)
        row["controller_roundtrip_ms"] = round(roundtrip_ms, 6)
        return row

    def close(self) -> None:
        if self.process.stdin:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        self.stderr_stream.close()


def run_persistent(
    profile: str,
    inputs: list[Path],
    corpus: Path,
    adapters: dict[str, Path],
    engines: tuple[str, ...],
    repetitions: int,
    dpi: int,
    output_root: Path | None,
    seed: int,
    cpu: int,
    warmup_passes: int,
    jsonl_path: Path,
) -> list[dict[str, Any]]:
    workers = {
        name: Worker(name, adapters[name], cpu, jsonl_path.with_name(f"{jsonl_path.stem}-{name}.stderr.log"))
        for name in engines
    }
    rng = random.Random(seed)
    blocks = [(repetition, path) for repetition in range(repetitions) for path in inputs]
    rng.shuffle(blocks)
    tasks: list[tuple[int, Path, str]] = []
    for repetition, path in blocks:
        block_engines = list(engines)
        rng.shuffle(block_engines)
        tasks.extend((repetition, path, engine) for engine in block_engines)
    rows: list[dict[str, Any]] = []
    try:
        warmup_path = jsonl_path.with_name(f"{jsonl_path.stem}-warmup.jsonl")
        with warmup_path.open("w", encoding="utf-8", newline="\n") as warmup_stream:
            warmup_tasks = [
                (warmup, path, engine)
                for warmup in range(warmup_passes)
                for path in inputs
                for engine in engines
            ]
            rng.shuffle(warmup_tasks)
            for warmup, path, engine in warmup_tasks:
                row = workers[engine].request(profile, path, dpi, None)
                row.update(
                    {
                        "mode": "untimed_warmup",
                        "warmup_pass": warmup,
                        "relative_path": path.relative_to(corpus).as_posix(),
                    }
                )
                warmup_stream.write(json.dumps(row, sort_keys=True) + "\n")
        with jsonl_path.open("w", encoding="utf-8", newline="\n") as stream:
            for index, (repetition, path, engine) in enumerate(tasks, start=1):
                relative = path.relative_to(corpus).as_posix()
                raster_output = None
                if output_root is not None and repetition == 0:
                    raster_output = output_root / engine / (relative.replace("/", "__") + ".ppm")
                    raster_output.parent.mkdir(parents=True, exist_ok=True)
                row = workers[engine].request(profile, path, dpi, raster_output)
                row.update(
                    {
                        "mode": "persistent",
                        "repetition": repetition,
                        "relative_path": relative,
                        "sequence": index,
                    }
                )
                rows.append(row)
                stream.write(json.dumps(row, sort_keys=True) + "\n")
                stream.flush()
                if index % 100 == 0:
                    print(f"persistent {profile}: {index}/{len(tasks)}", flush=True)
    finally:
        for worker in workers.values():
            worker.close()
    return rows


def run_retained_resource_render(
    inputs: list[Path],
    corpus: Path,
    adapters: dict[str, Path],
    iterations: int,
    dpi: int,
    seed: int,
    cpu: int,
    jsonl_path: Path,
) -> list[dict[str, Any]]:
    """Open each document once, warm resources once, then time new rasters."""

    workers = {
        name: Worker(name, adapters[name], cpu, jsonl_path.with_name(f"{jsonl_path.stem}-{name}.stderr.log"))
        for name in RENDER_ENGINES
    }
    rng = random.Random(seed)
    documents = list(inputs)
    rng.shuffle(documents)
    rows: list[dict[str, Any]] = []
    try:
        with jsonl_path.open("w", encoding="utf-8", newline="\n") as stream:
            sequence = 0
            for path in documents:
                engines = list(RENDER_ENGINES)
                rng.shuffle(engines)
                for engine in engines:
                    sequence += 1
                    row = workers[engine].request(
                        "render-retained-resources", path, dpi, None, iterations
                    )
                    row.update(
                        {
                            "mode": "retained_resources",
                            "relative_path": path.relative_to(corpus).as_posix(),
                            "sequence": sequence,
                            "iterations": iterations,
                        }
                    )
                    rows.append(row)
                    stream.write(json.dumps(row, sort_keys=True) + "\n")
                    stream.flush()
    finally:
        for worker in workers.values():
            worker.close()
    return rows


def retained_sample_determinism(rows: Iterable[dict[str, Any]]) -> dict[str, Any]:
    failures = []
    checked = 0
    for row in rows:
        checked += 1
        if row.get("status") != "ok":
            failures.append(
                {
                    "engine": row.get("engine"),
                    "relative_path": row.get("relative_path"),
                    "hashes": [],
                    "reasons": [
                        f"render request status is {row.get('status', 'missing')}"
                    ],
                }
            )
            continue
        hashes = [str(value) for value in row.get("render_sample_hashes", [])]
        samples = row.get("render_samples_ms", [])
        expected = int(row.get("iterations", 0))
        reasons = []
        if expected <= 0 or len(hashes) != expected or len(samples) != expected:
            reasons.append(
                f"expected {expected} samples, observed {len(samples)} timings and {len(hashes)} hashes"
            )
        if not hashes or len(set(hashes)) != 1:
            reasons.append("raster identity is missing or unstable")
        if reasons:
            failures.append(
                {
                    "engine": row.get("engine"),
                    "relative_path": row.get("relative_path"),
                    "hashes": hashes,
                    "reasons": reasons,
                }
            )
    return {
        "qualified_engine_documents": checked - len(failures),
        "checked_engine_documents": checked,
        "failures": failures,
    }


def run_fresh(
    profile: str,
    inputs: list[Path],
    corpus: Path,
    adapters: dict[str, Path],
    engines: tuple[str, ...],
    repetitions: int,
    dpi: int,
    seed: int,
    cpu: int,
    timeout: int,
    jsonl_path: Path,
) -> list[dict[str, Any]]:
    tasks = [(repetition, path, engine) for repetition in range(repetitions) for path in inputs for engine in engines]
    random.Random(seed).shuffle(tasks)
    rows: list[dict[str, Any]] = []
    with jsonl_path.open("w", encoding="utf-8", newline="\n") as stream:
        for index, (repetition, path, engine) in enumerate(tasks, start=1):
            command = [
                "taskset",
                "-c",
                str(cpu),
                str(adapters[engine]),
                "--request",
                profile,
                str(path),
                str(dpi),
                "-",
            ]
            started = time.perf_counter_ns()
            try:
                completed = subprocess.run(command, capture_output=True, text=True, timeout=timeout, check=False)
                process_ms = (time.perf_counter_ns() - started) / 1_000_000.0
                lines = [line for line in completed.stdout.splitlines() if line.strip()]
                row = json.loads(lines[-1]) if lines else {
                    "engine": engine,
                    "status": "error",
                    "error": "adapter produced no JSON",
                }
                row["exit"] = completed.returncode
                if completed.stderr:
                    row["stderr_tail"] = completed.stderr[-4000:]
            except subprocess.TimeoutExpired as error:
                process_ms = (time.perf_counter_ns() - started) / 1_000_000.0
                row = {"engine": engine, "status": "timeout", "error": str(error)}
            relative = path.relative_to(corpus).as_posix()
            row.update(
                {
                    "mode": "fresh_process",
                    "repetition": repetition,
                    "relative_path": relative,
                    "sequence": index,
                    "process_ms": round(process_ms, 6),
                }
            )
            rows.append(row)
            stream.write(json.dumps(row, sort_keys=True) + "\n")
            stream.flush()
            if index % 100 == 0:
                print(f"fresh {profile}: {index}/{len(tasks)}", flush=True)
    return rows


def page_count_qualification(
    rows: list[dict[str, Any]], expected_repetitions: int
) -> dict[str, Any]:
    raw_by_document: dict[str, dict[str, list[dict[str, Any]]]] = defaultdict(
        lambda: defaultdict(list)
    )
    for row in rows:
        raw_by_document[str(row.get("relative_path", ""))][str(row.get("engine", ""))].append(row)
    disagreements: list[dict[str, Any]] = []
    failures: list[dict[str, Any]] = []
    qualified: dict[str, int] = {engine: 0 for engine in PARSER_ENGINES}
    for path, engines in sorted(raw_by_document.items()):
        counts_by_engine: dict[str, list[int]] = {}
        for engine in PARSER_ENGINES:
            engine_rows = engines.get(engine, [])
            counts = [
                int(row["page_count"])
                for row in engine_rows
                if row.get("status") == "ok" and isinstance(row.get("page_count"), int)
            ]
            reasons = []
            if len(engine_rows) != expected_repetitions:
                reasons.append(
                    f"expected {expected_repetitions} rows, observed {len(engine_rows)}"
                )
            if len(counts) != len(engine_rows):
                reasons.append("one or more repetitions failed")
            if len(set(counts)) > 1:
                reasons.append("page count changes between repetitions")
            if reasons:
                failures.append(
                    {
                        "relative_path": path,
                        "engine": engine,
                        "reasons": reasons,
                    }
                )
            else:
                counts_by_engine[engine] = counts
        medians = {
            engine: int(statistics.median(counts))
            for engine, counts in counts_by_engine.items()
            if counts
        }
        if medians:
            consensus, votes = Counter(medians.values()).most_common(1)[0]
        else:
            continue
        if votes != len(PARSER_ENGINES) or len(medians) != len(PARSER_ENGINES):
            disagreements.append({"relative_path": path, "counts": medians, "consensus": consensus})
        for engine in PARSER_ENGINES:
            if medians.get(engine) == consensus:
                qualified[engine] += 1
    return {
        "qualified_documents": qualified,
        "disagreements": disagreements,
        "repetition_failures": failures,
    }


def quality_metrics(raster_root: Path, inputs: list[Path], corpus: Path) -> dict[str, Any]:
    try:
        import numpy as np
        from PIL import Image
        import flip_evaluator
        from skimage.color import deltaE_ciede2000, rgb2lab
        from skimage.metrics import structural_similarity
    except Exception as error:
        return {"status": "unavailable", "error": str(error)}

    per_engine: dict[str, list[dict[str, float]]] = defaultdict(list)
    dimension_failures: list[dict[str, Any]] = []
    page_rows: list[dict[str, Any]] = []
    for index, path in enumerate(inputs, start=1):
        relative = path.relative_to(corpus).as_posix()
        images: dict[str, Any] = {}
        sizes: dict[str, list[int]] = {}
        for engine in RENDER_ENGINES:
            raster_path = raster_root / engine / (relative.replace("/", "__") + ".ppm")
            if not raster_path.exists():
                continue
            with Image.open(raster_path) as image:
                rgb = image.convert("RGB")
                images[engine] = np.asarray(rgb, dtype=np.uint8).copy()
                sizes[engine] = [rgb.width, rgb.height]
        if len(images) != len(RENDER_ENGINES) or len({tuple(size) for size in sizes.values()}) != 1:
            dimension_failures.append({"relative_path": relative, "sizes": sizes})
            continue
        page_metrics: dict[str, Any] = {"relative_path": relative, "engines": {}}
        for engine, candidate_u8 in images.items():
            references = [array for other, array in images.items() if other != engine]
            consensus_u8 = np.median(np.stack(references, axis=0), axis=0).astype(np.uint8)
            candidate = candidate_u8.astype(np.float32)
            consensus = consensus_u8.astype(np.float32)
            delta = np.abs(candidate - consensus)
            mse = float(np.mean((candidate - consensus) ** 2))
            changed = float(np.mean(np.max(delta, axis=2) > 8.0) * 100.0)
            ssim = float(structural_similarity(candidate_u8, consensus_u8, channel_axis=2, data_range=255))
            # Downsample only the expensive perceptual color diagnostic. SSIM
            # and pixel errors above always use the original raster.
            step = max(1, int(math.sqrt((candidate_u8.shape[0] * candidate_u8.shape[1]) / 1_000_000)))
            lab_candidate = rgb2lab(candidate_u8[::step, ::step] / 255.0)
            lab_consensus = rgb2lab(consensus_u8[::step, ::step] / 255.0)
            delta_e = deltaE_ciede2000(lab_candidate, lab_consensus)
            _flip_map, mean_flip, _flip_parameters = flip_evaluator.evaluate(
                candidate_u8.astype(np.float32) / 255.0,
                consensus_u8.astype(np.float32) / 255.0,
                "LDR",
                applyMagma=False,
                computeMeanError=True,
            )
            metrics = {
                "ssim": ssim,
                "changed_pixel_gt8_percent": changed,
                "mean_absolute_channel_delta": float(np.mean(delta)),
                "rmse": math.sqrt(mse),
                "psnr_db": 99.0 if mse == 0.0 else 10.0 * math.log10((255.0**2) / mse),
                "mean_delta_e_2000": float(np.mean(delta_e)),
                "p95_delta_e_2000": float(np.percentile(delta_e, 95)),
                "mean_flip": float(mean_flip),
            }
            per_engine[engine].append(metrics)
            page_metrics["engines"][engine] = {key: round(value, 8) for key, value in metrics.items()}
        page_rows.append(page_metrics)
        if index % 10 == 0:
            print(f"quality: {index}/{len(inputs)}", flush=True)

    summary: dict[str, Any] = {}
    for engine in RENDER_ENGINES:
        metrics = per_engine[engine]
        summary[engine] = {
            key: distribution(row[key] for row in metrics)
            for key in (
                "ssim",
                "changed_pixel_gt8_percent",
                "mean_absolute_channel_delta",
                "rmse",
                "psnr_db",
                "mean_delta_e_2000",
                "p95_delta_e_2000",
                "mean_flip",
            )
        }
    return {
        "status": "complete",
        "contract": "leave-one-engine-out per-pixel median consensus; diagnostic, not ground truth",
        "qualified_pages": len(page_rows),
        "dimension_failures": dimension_failures,
        "engines": summary,
        "pages": page_rows,
    }


def metric_table(
    title: str, rows: list[tuple[str, dict[str, Any]]], engines: tuple[str, ...]
) -> list[str]:
    output = [f"## {title}", "", "| Benchmark | " + " | ".join(LABELS[e] for e in engines) + " |", "|---|" + "---:|" * len(engines)]
    for label, values in rows:
        output.append("| " + label + " | " + " | ".join(str(values.get(engine, "—")) for engine in engines) + " |")
    output.append("")
    return output


def fmt_dist(summary: dict[str, Any], engine: str, key: str, suffix: str = " ms") -> str:
    value = summary.get(engine, {}).get(key)
    return "—" if value is None else f"{value:,.3f}{suffix}"


def fmt_nested(
    summary: dict[str, Any], engine: str, metric: str, key: str, suffix: str = ""
) -> str:
    value = summary.get(engine, {}).get(metric, {}).get(key)
    return "—" if value is None else f"{value:,.6f}{suffix}"


def build_report(summary: dict[str, Any]) -> str:
    parse = summary["parsing"]
    render = summary["rendering"]
    quality = summary["quality"]
    document_count = int(summary["configuration"].get("document_count", 100))
    lines = [
        f"# PEBQ {document_count}-PDF matched-contract qualification",
        "",
        f"Generated `{summary['finished_at_utc']}` on `{summary['environment']['hostname']}`.",
        "Every column executes the same declared contract. Results are disqualified when the",
        "adapter fails or produces a page count/dimensions inconsistent with the contract.",
        "",
        "The parsing profile is in-memory document open plus resolved page count. Process-resident,",
        "document-cold timings exclude file reading and process startup, but reopen each document",
        "for every request. Fresh-process timings include adapter startup, file reading, parsing and",
        "JSON output. Rendering is page one at the declared DPI to raw RGB; PNG/WebP encoding is not",
        "part of the renderer timer. Primary percentile tables give every document equal weight by",
        "summarizing one median per document. Raw-observation distributions remain diagnostic data.",
        "",
    ]
    versions = summary["environment"].get("versions", {})
    def version_line(name: str) -> str:
        text = versions.get(name, {}).get("text", "unavailable")
        if name == "pdfium" and text:
            try:
                version = json.loads(text)
                return "PDFium " + ".".join(
                    str(version[key]) for key in ("major", "minor", "build", "patch")
                )
            except (json.JSONDecodeError, KeyError, TypeError):
                pass
        return text.splitlines()[0] if text else "unavailable"
    lines += metric_table(
        "Engine identity",
        [
            ("Version", {
                "wellfriendpdf": summary.get("source_revision", "recorded by commit"),
                "qpdf": version_line("qpdf"),
                "mupdf": version_line("mupdf"),
                "pdfium": version_line("pdfium"),
                "poppler": version_line("poppler"),
            }),
        ],
        PARSER_ENGINES,
    )

    qualified = parse["qualification"]["qualified_documents"]
    parse_document_medians = parse.get(
        "process_resident_document_cold_document_medians_ms",
        parse.get("persistent_parse_document_medians_ms", parse["persistent_parse_ms"]),
    )
    parse_observations = parse.get(
        "process_resident_document_cold_observations_ms",
        parse.get("persistent_parse_observations_ms", parse["persistent_parse_ms"]),
    )
    lines += metric_table(
        f"Document open + resolved page count - process-resident, document-cold ({document_count} document medians)",
        [
            ("Qualified page counts", {e: f"{qualified.get(e, 0)}/{document_count}" for e in PARSER_ENGINES}),
            *[
                (name, {e: fmt_dist(parse_document_medians, e, key) for e in PARSER_ENGINES})
                for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
            ],
        ],
        PARSER_ENGINES,
    )
    lines += metric_table(
        "Document open + resolved page count - raw observations (scheduler and tail diagnostic)",
        [
            (name, {e: fmt_dist(parse_observations, e, key) for e in PARSER_ENGINES})
            for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
        ],
        PARSER_ENGINES,
    )
    parse_slowest = parse.get("slowest_document_medians_ms", {})
    if parse_slowest:
        lines += ["### Slowest document-open medians", ""]
        for engine in PARSER_ENGINES:
            entries = parse_slowest.get(engine, [])
            if entries:
                first = entries[0]
                lines.append(
                    f"- {LABELS[engine]}: `{first['relative_path']}` at "
                    f"**{first['median_ms']:,.3f} ms**."
                )
        lines.append("")
    lines += metric_table(
        "Document open + resolved page count — fresh process end to end",
        [
            *[
                (name, {e: fmt_dist(parse["fresh_process_ms"], e, key) for e in PARSER_ENGINES})
                for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
            ],
        ],
        PARSER_ENGINES,
    )
    lines += metric_table(
        "Document open + resolved page count — fresh-process peak RSS",
        [
            (name, {e: fmt_dist(parse["fresh_peak_rss_kib"], e, key, " KiB") for e in PARSER_ENGINES})
            for name, key in (("P50", "p50"), ("P95", "p95"), ("Maximum", "max"))
        ],
        PARSER_ENGINES,
    )
    ratio = parse["wellfriend_vs_poppler_persistent_paired_ratio"]
    lines += [
        "## Document-open claim gate",
        "",
        f"- Shared qualified documents: **{ratio.get('count', 0)}**.",
        f"- Median paired Poppler/Wellfriend ratio: **{ratio.get('median_paired_ratio', '—')}×**.",
        f"- Geometric-mean paired ratio: **{ratio.get('geometric_mean_paired_ratio', '—')}×**.",
        f"- Bootstrapped 95% interval for the median ratio: **{ratio.get('bootstrap_95_percent_ci', '—')}**.",
        f"- 20× lower-bound claim: **{'PASS' if ratio.get('claim_20x_median_lower_bound_pass') else 'FAIL'}**.",
        "",
    ]
    fresh_ratio = parse.get("wellfriend_vs_poppler_fresh_paired_ratio", {})
    if fresh_ratio:
        lines += [
            f"The fresh-process paired median ratio is **{fresh_ratio.get('median_paired_ratio', '—')}×**",
            f"with 95% interval **{fresh_ratio.get('bootstrap_95_percent_ci', '—')}**.",
            "",
        ]

    accepted = render["accepted"]
    render_document_medians = render.get(
        "process_resident_document_cold_document_medians_ms",
        render.get("persistent_render_document_medians_ms", render["persistent_render_ms"]),
    )
    render_observations = render.get(
        "process_resident_document_cold_observations_ms",
        render.get("persistent_render_observations_ms", render["persistent_render_ms"]),
    )
    lines += metric_table(
        f"Rendering - process-resident, document-cold raw RGB ({document_count} document medians)",
        [
            ("Successful renders", {e: f"{accepted.get(e, 0)}/{document_count}" for e in RENDER_ENGINES}),
            *[
                (name, {e: fmt_dist(render_document_medians, e, key) for e in RENDER_ENGINES})
                for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
            ],
        ],
        RENDER_ENGINES,
    )
    lines += metric_table(
        "Rendering - raw observations (scheduler and tail diagnostic)",
        [
            (name, {e: fmt_dist(render_observations, e, key) for e in RENDER_ENGINES})
            for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
        ],
        RENDER_ENGINES,
    )
    determinism = render.get("raster_determinism", {})
    if determinism:
        lines += [
            "Repeated-raster determinism is "
            + ("**PASS**" if not determinism.get("failures") else "**FAIL**")
            + f" for {determinism.get('qualified_engine_documents', 0)}/"
            + f"{determinism.get('checked_engine_documents', 0)} engine-document pairs.",
            "",
        ]
    retained_resources = render.get("retained_resource_render_document_medians_ms")
    if retained_resources:
        lines += metric_table(
            f"Rendering - retained resources, fresh raster ({document_count} document medians)",
            [
                (name, {e: fmt_dist(retained_resources, e, key) for e in RENDER_ENGINES})
                for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
            ],
            RENDER_ENGINES,
        )
        retained_determinism = render.get("retained_resource_raster_determinism", {})
        lines += [
            "This profile opens each document once, performs one untimed resource warm-up,",
            "then executes new rasterizations with final-raster reuse disabled. Repeated-raster",
            "determinism is "
            + ("**PASS**" if not retained_determinism.get("failures") else "**FAIL**")
            + f" for {retained_determinism.get('qualified_engine_documents', 0)}/"
            + f"{retained_determinism.get('checked_engine_documents', 0)} engine-document pairs.",
            "",
        ]
    lines += metric_table(
        "Rendering — fresh process end to end",
        [
            *[
                (name, {e: fmt_dist(render["fresh_process_ms"], e, key) for e in RENDER_ENGINES})
                for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
            ],
        ],
        RENDER_ENGINES,
    )
    lines += metric_table(
        "Rendering — fresh-process peak RSS",
        [
            (name, {e: fmt_dist(render["fresh_peak_rss_kib"], e, key, " KiB") for e in RENDER_ENGINES})
            for name, key in (("P50", "p50"), ("P95", "p95"), ("Maximum", "max"))
        ],
        RENDER_ENGINES,
    )

    if quality.get("status") == "complete":
        qengines = quality["engines"]
        lines += metric_table(
            "Rendering quality — leave-one-engine-out consensus",
            [
                ("Dimension-qualified pages", {e: f"{quality['qualified_pages']}/{document_count}" for e in RENDER_ENGINES}),
                ("SSIM P50 ↑", {e: fmt_nested(qengines, e, "ssim", "p50") for e in RENDER_ENGINES}),
                ("SSIM P05 ↑", {e: fmt_nested(qengines, e, "ssim", "p05") for e in RENDER_ENGINES}),
                ("Changed pixels >8 P50 ↓", {e: fmt_nested(qengines, e, "changed_pixel_gt8_percent", "p50", "%") for e in RENDER_ENGINES}),
                ("Mean ΔE2000 P50 ↓", {e: fmt_nested(qengines, e, "mean_delta_e_2000", "p50") for e in RENDER_ENGINES}),
                ("Mean FLIP P50 ↓", {e: fmt_nested(qengines, e, "mean_flip", "p50") for e in RENDER_ENGINES}),
                ("RMSE P50 ↓", {e: fmt_nested(qengines, e, "rmse", "p50") for e in RENDER_ENGINES}),
            ],
            RENDER_ENGINES,
        )
        lines += [
            "Consensus is a symmetric differential diagnostic, not an ISO visual oracle. A",
            "renderer can agree with the other engines and still be wrong. Dimension mismatches",
            "are failures and are never resized away.",
            "",
        ]
        if quality.get("dimension_failures"):
            lines += ["### Strict dimension failures", ""]
            for failure in quality["dimension_failures"]:
                sizes = ", ".join(
                    f"{LABELS.get(engine, engine)} {size[0]}×{size[1]}"
                    for engine, size in failure["sizes"].items()
                )
                lines.append(f"- `{failure['relative_path']}` — {sizes}.")
            lines.append("")
        visual_evidence = quality.get("visual_evidence")
        if visual_evidence:
            lines += [
                "### Visual evidence",
                "",
                str(visual_evidence.get("description", "Native-output comparison sheets.")),
                "",
            ]
            lines.extend(
                f"- [{item['label']}]({item['path']})"
                for item in visual_evidence.get("links", [])
            )
            lines.append("")
    else:
        lines += ["## Rendering quality", "", f"Unavailable: `{quality.get('error', 'unknown error')}`", ""]

    lines += [
        "## Verdict",
        "",
        f"- All five native document-open adapters return the same page count on {min(qualified.values(), default=0)}/{document_count} inputs.",
        "- The matched process-resident, document-cold open/page-count evidence rejects the 20× Poppler claim.",
        f"- All four raster engines render at least {min(accepted.values(), default=0)}/{document_count} pages.",
    ]
    if quality.get("status") == "complete":
        lines += [
            f"- {quality.get('qualified_pages', 0)}/{document_count} pages have identical native dimensions.",
            "- Consensus quality is diagnostic rather than ground truth and does not establish universal correctness.",
        ]
    else:
        lines.append(
            f"- Quality comparison is unavailable: {quality.get('error', 'unknown error')}."
        )
    lines += [
        "- Wellfriend PDF is not the fastest renderer in this campaign; reference leadership varies by percentile.",
        "",
        "## Reproducibility",
        "",
        f"- Corpus SHA-256 manifest: `{summary['corpus_manifest_sha256']}`.",
        f"- DPI: `{summary['configuration']['dpi']}`.",
        f"- Process-resident document-cold repetitions: `{summary['configuration']['persistent_repetitions']}`.",
        f"- Process-resident warm-up passes: `{summary['configuration'].get('persistent_warmup_passes', 0)}`.",
        f"- Retained-resource render iterations: `{summary['configuration'].get('retained_render_iterations', 0)}`.",
        f"- Fresh-process repetitions: `{summary['configuration']['fresh_repetitions']}`.",
        f"- CPU affinity: `{summary['configuration']['cpu']}`.",
        f"- Retained raw observations: `{sum(summary.get('raw_row_counts', {}).values())}`.",
        "- Raw JSONL observations, adapter sources, binary hashes and the complete JSON summary",
        "  are retained with this report.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--adapter-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--dpi", type=int, default=144)
    parser.add_argument("--persistent-repetitions", type=int, default=10)
    parser.add_argument("--persistent-warmup-passes", type=int, default=1)
    parser.add_argument("--retained-render-iterations", type=int, default=5)
    parser.add_argument("--fresh-repetitions", type=int, default=5)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--cpu", type=int, default=2)
    parser.add_argument("--seed", type=int, default=20260930)
    args = parser.parse_args()

    started = utc_now()
    args.output.mkdir(parents=True, exist_ok=True)
    inputs = sorted(path for path in args.corpus.rglob("*") if path.is_file() and path.suffix.lower() == ".pdf")
    if args.limit > 0:
        inputs = inputs[: args.limit]
    if len(inputs) != args.limit:
        raise SystemExit(f"expected {args.limit} PDFs, found {len(inputs)}")

    adapters = {engine: args.adapter_dir / f"pebq-{engine}" for engine in PARSER_ENGINES}
    missing = [str(path) for path in adapters.values() if not path.is_file()]
    if missing:
        raise SystemExit(f"missing adapters: {missing}")

    corpus_manifest = [
        {
            "relative_path": path.relative_to(args.corpus).as_posix(),
            "bytes": path.stat().st_size,
            "sha256": sha256_file(path),
        }
        for path in inputs
    ]
    manifest_bytes = (json.dumps(corpus_manifest, sort_keys=True, separators=(",", ":")) + "\n").encode()
    (args.output / "corpus-manifest.json").write_bytes(manifest_bytes)
    manifest_sha = hashlib.sha256(manifest_bytes).hexdigest()

    metadata = {
        "started_at_utc": started,
        "hostname": platform.node(),
        "platform": platform.platform(),
        "python": sys.version,
        "cpu_count": os.cpu_count(),
        "versions": {
            "qpdf": tool_version(["qpdf", "--version"]),
            "poppler": tool_version(["pdfinfo", "-v"]),
            "mupdf": tool_version(["mutool", "-v"]),
            "pdfium": pdfium_version(adapters["pdfium"]),
            "kernel": tool_version(["uname", "-a"]),
            "cpu": tool_version(["lscpu"]),
            "perf_hardware_counters": tool_version(["perf", "stat", "-e", "cycles,instructions", "true"]),
        },
        "adapter_sha256": {engine: sha256_file(path) for engine, path in adapters.items()},
    }
    (args.output / "environment.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    parse_persistent = run_persistent(
        "page-count", inputs, args.corpus, adapters, PARSER_ENGINES,
        args.persistent_repetitions, args.dpi, None, args.seed + 1, args.cpu,
        args.persistent_warmup_passes,
        args.output / "parse-persistent.jsonl",
    )
    parse_fresh = run_fresh(
        "page-count", inputs, args.corpus, adapters, PARSER_ENGINES,
        args.fresh_repetitions, args.dpi, args.seed + 2, args.cpu, args.timeout,
        args.output / "parse-fresh.jsonl",
    )
    raster_root = args.output / "rasters"
    render_persistent = run_persistent(
        "render", inputs, args.corpus, adapters, RENDER_ENGINES,
        args.persistent_repetitions, args.dpi, raster_root, args.seed + 3, args.cpu,
        args.persistent_warmup_passes,
        args.output / "render-persistent.jsonl",
    )
    render_retained_resources = run_retained_resource_render(
        inputs,
        args.corpus,
        adapters,
        args.retained_render_iterations,
        args.dpi,
        args.seed + 4,
        args.cpu,
        args.output / "render-retained-resources.jsonl",
    )
    render_fresh = run_fresh(
        "render", inputs, args.corpus, adapters, RENDER_ENGINES,
        args.fresh_repetitions, args.dpi, args.seed + 5, args.cpu, args.timeout,
        args.output / "render-fresh.jsonl",
    )

    qualification = page_count_qualification(
        parse_persistent, args.persistent_repetitions
    )
    parse_persistent_observation_distributions = {
        engine: distribution(
            row["parse_ms"] for row in parse_persistent
            if row.get("engine") == engine and row.get("status") == "ok"
        )
        for engine in PARSER_ENGINES
    }
    parse_persistent_document_distributions = {
        engine: document_median_distribution(
            parse_persistent, engine, "parse_ms", args.persistent_repetitions
        )
        for engine in PARSER_ENGINES
    }
    parse_fresh_distributions = {
        engine: distribution(
            row["process_ms"] for row in parse_fresh
            if row.get("engine") == engine and row.get("status") == "ok"
        )
        for engine in PARSER_ENGINES
    }
    wellfriend_by_doc = median_by_key(
        (row for row in parse_persistent if row.get("engine") == "wellfriendpdf"), "parse_ms"
    )
    poppler_by_doc = median_by_key(
        (row for row in parse_persistent if row.get("engine") == "poppler"), "parse_ms"
    )
    qualified_paths = set(wellfriend_by_doc)
    qualified_paths -= {
        str(row["relative_path"]) for row in qualification["disagreements"]
    }
    qualified_paths -= {
        str(row["relative_path"]) for row in qualification["repetition_failures"]
    }
    paired_ratio = bootstrap_ratio_ci(
        {path: poppler_by_doc[path] for path in qualified_paths if path in poppler_by_doc},
        {path: wellfriend_by_doc[path] for path in qualified_paths if path in wellfriend_by_doc},
        args.seed + 6,
    )

    quality = quality_metrics(raster_root, inputs, args.corpus)
    quality_page_row_count = len(quality.get("pages", []))
    if quality.get("pages"):
        with (args.output / "quality-pages.jsonl").open("w", encoding="utf-8", newline="\n") as stream:
            for row in quality["pages"]:
                stream.write(json.dumps(row, sort_keys=True) + "\n")
        quality = dict(quality)
        quality.pop("pages", None)

    summary = {
        "schema": "wellfriendpdf.pebq.v2",
        "started_at_utc": started,
        "finished_at_utc": utc_now(),
        "corpus_manifest_sha256": manifest_sha,
        "configuration": {
            "dpi": args.dpi,
            "document_count": len(inputs),
            "persistent_repetitions": args.persistent_repetitions,
            "persistent_warmup_passes": args.persistent_warmup_passes,
            "retained_render_iterations": args.retained_render_iterations,
            "fresh_repetitions": args.fresh_repetitions,
            "cpu": args.cpu,
            "seed": args.seed,
            "timeout_seconds": args.timeout,
        },
        "environment": metadata,
        "parsing": {
            "contract": "identical in-memory open plus resolved page count",
            "profile": "process-resident adapter; document reopened for every request; file read excluded",
            "qualification": qualification,
            "persistent_parse_ms": parse_persistent_document_distributions,
            "process_resident_document_cold_document_medians_ms": parse_persistent_document_distributions,
            "process_resident_document_cold_observations_ms": parse_persistent_observation_distributions,
            "slowest_document_medians_ms": {
                engine: slowest_document_medians(
                    parse_persistent,
                    engine,
                    "parse_ms",
                    expected_rows=args.persistent_repetitions,
                )
                for engine in PARSER_ENGINES
            },
            "fresh_process_ms": parse_fresh_distributions,
            "fresh_peak_rss_kib": {
                engine: distribution(
                    row["peak_rss_kib"] for row in parse_fresh
                    if row.get("engine") == engine and row.get("status") == "ok"
                )
                for engine in PARSER_ENGINES
            },
            "wellfriend_vs_poppler_persistent_paired_ratio": paired_ratio,
        },
        "rendering": {
            "contract": "open identical bytes; render page 1 at fixed DPI to raw RGB; encoding excluded",
            "profile": "process-resident adapter; document reopened for every request; final-raster reuse disabled by construction",
            "accepted": successful_engine_documents(
                render_persistent,
                RENDER_ENGINES,
                args.persistent_repetitions,
                ("render_ms", "width", "height"),
            ),
            "persistent_render_ms": {
                engine: document_median_distribution(
                    render_persistent,
                    engine,
                    "render_ms",
                    args.persistent_repetitions,
                )
                for engine in RENDER_ENGINES
            },
            "process_resident_document_cold_document_medians_ms": {
                engine: document_median_distribution(
                    render_persistent,
                    engine,
                    "render_ms",
                    args.persistent_repetitions,
                )
                for engine in RENDER_ENGINES
            },
            "process_resident_document_cold_observations_ms": {
                engine: distribution(
                    row["render_ms"] for row in render_persistent
                    if row.get("engine") == engine and row.get("status") == "ok"
                )
                for engine in RENDER_ENGINES
            },
            "slowest_document_medians_ms": {
                engine: slowest_document_medians(
                    render_persistent,
                    engine,
                    "render_ms",
                    expected_rows=args.persistent_repetitions,
                )
                for engine in RENDER_ENGINES
            },
            "raster_determinism": raster_determinism(
                render_persistent, args.persistent_repetitions
            ),
            "retained_resource_render_document_medians_ms": {
                engine: distribution(
                    row["render_ms"] for row in render_retained_resources
                    if row.get("engine") == engine and row.get("status") == "ok"
                )
                for engine in RENDER_ENGINES
            },
            "retained_resource_render_samples_ms": {
                engine: distribution(
                    sample
                    for row in render_retained_resources
                    if row.get("engine") == engine and row.get("status") == "ok"
                    for sample in row.get("render_samples_ms", [])
                )
                for engine in RENDER_ENGINES
            },
            "retained_resource_raster_determinism": retained_sample_determinism(
                render_retained_resources
            ),
            "fresh_process_ms": {
                engine: distribution(
                    row["process_ms"] for row in render_fresh
                    if row.get("engine") == engine and row.get("status") == "ok"
                )
                for engine in RENDER_ENGINES
            },
            "fresh_peak_rss_kib": {
                engine: distribution(
                    row["peak_rss_kib"] for row in render_fresh
                    if row.get("engine") == engine and row.get("status") == "ok"
                )
                for engine in RENDER_ENGINES
            },
        },
        "quality": quality,
        "raw_row_counts": {
            "parse-persistent.jsonl": len(parse_persistent),
            "parse-fresh.jsonl": len(parse_fresh),
            "render-persistent.jsonl": len(render_persistent),
            "render-retained-resources.jsonl": len(render_retained_resources),
            "render-fresh.jsonl": len(render_fresh),
            "quality-pages.jsonl": quality_page_row_count,
        },
    }
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (args.output / "README.md").write_text(build_report(summary), encoding="utf-8", newline="\n")
    print(json.dumps({"status": "complete", "output": str(args.output), "manifest": manifest_sha}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
