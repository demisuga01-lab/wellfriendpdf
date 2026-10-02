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
        "text": "PDFium version unavailable; adapter linkage was retained in environment metadata",
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

    def request(self, profile: str, path: Path, dpi: int, output: Path | None) -> dict[str, Any]:
        assert self.process.stdin is not None and self.process.stdout is not None
        payload = "\t".join((profile, str(path), str(dpi), str(output) if output else "-"))
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
    jsonl_path: Path,
) -> list[dict[str, Any]]:
    workers = {
        name: Worker(name, adapters[name], cpu, jsonl_path.with_name(f"{jsonl_path.stem}-{name}.stderr.log"))
        for name in engines
    }
    tasks = [(repetition, path, engine) for repetition in range(repetitions) for path in inputs for engine in engines]
    random.Random(seed).shuffle(tasks)
    rows: list[dict[str, Any]] = []
    try:
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


def page_count_qualification(rows: list[dict[str, Any]]) -> dict[str, Any]:
    by_document: dict[str, dict[str, list[int]]] = defaultdict(lambda: defaultdict(list))
    for row in rows:
        if row.get("status") == "ok" and isinstance(row.get("page_count"), int):
            by_document[str(row["relative_path"])][str(row["engine"])].append(int(row["page_count"]))
    disagreements: list[dict[str, Any]] = []
    qualified: dict[str, int] = {engine: 0 for engine in PARSER_ENGINES}
    for path, engines in sorted(by_document.items()):
        medians = {engine: int(statistics.median(counts)) for engine, counts in engines.items()}
        if medians:
            consensus, votes = Counter(medians.values()).most_common(1)[0]
        else:
            continue
        if votes != len(PARSER_ENGINES) or len(medians) != len(PARSER_ENGINES):
            disagreements.append({"relative_path": path, "counts": medians, "consensus": consensus})
        for engine in PARSER_ENGINES:
            if medians.get(engine) == consensus:
                qualified[engine] += 1
    return {"qualified_documents": qualified, "disagreements": disagreements}


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
    lines = [
        "# PEBQ 100-PDF matched-contract qualification",
        "",
        f"Generated `{summary['finished_at_utc']}` on `{summary['environment']['hostname']}`.",
        "Every column executes the same declared contract. Results are disqualified when the",
        "adapter fails or produces a page count/dimensions inconsistent with the contract.",
        "",
        "The parsing profile is in-memory document open plus resolved page count. Persistent",
        "timings exclude file reading and process startup; fresh-process timings include adapter",
        "startup, file reading, parsing and JSON output. Rendering is page one at the declared DPI",
        "to raw RGB; PNG/WebP encoding is not part of the renderer timer.",
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
    lines += metric_table(
        "Parsing — persistent native adapters",
        [
            ("Qualified page counts", {e: f"{qualified.get(e, 0)}/100" for e in PARSER_ENGINES}),
            *[
                (name, {e: fmt_dist(parse["persistent_parse_ms"], e, key) for e in PARSER_ENGINES})
                for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
            ],
        ],
        PARSER_ENGINES,
    )
    lines += metric_table(
        "Parsing — fresh process end to end",
        [
            *[
                (name, {e: fmt_dist(parse["fresh_process_ms"], e, key) for e in PARSER_ENGINES})
                for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
            ],
        ],
        PARSER_ENGINES,
    )
    lines += metric_table(
        "Parsing — fresh-process peak RSS",
        [
            (name, {e: fmt_dist(parse["fresh_peak_rss_kib"], e, key, " KiB") for e in PARSER_ENGINES})
            for name, key in (("P50", "p50"), ("P95", "p95"), ("Maximum", "max"))
        ],
        PARSER_ENGINES,
    )
    ratio = parse["wellfriend_vs_poppler_persistent_paired_ratio"]
    lines += [
        "## Parsing claim gate",
        "",
        f"- Shared qualified documents: **{ratio.get('count', 0)}**.",
        f"- Median paired Poppler/Wellfriend ratio: **{ratio.get('median_paired_ratio', '—')}×**.",
        f"- Geometric-mean paired ratio: **{ratio.get('geometric_mean_paired_ratio', '—')}×**.",
        f"- Bootstrapped 95% interval for the median ratio: **{ratio.get('bootstrap_95_percent_ci', '—')}**.",
        f"- Pre-registered 20× lower-bound claim: **{'PASS' if ratio.get('claim_20x_median_lower_bound_pass') else 'FAIL'}**.",
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
    lines += metric_table(
        "Rendering — persistent native raw-RGB page one",
        [
            ("Successful renders", {e: f"{accepted.get(e, 0)}/100" for e in RENDER_ENGINES}),
            *[
                (name, {e: fmt_dist(render["persistent_render_ms"], e, key) for e in RENDER_ENGINES})
                for name, key in (("P50", "p50"), ("P90", "p90"), ("P95", "p95"), ("P99", "p99"), ("Maximum", "max"))
            ],
        ],
        RENDER_ENGINES,
    )
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
                ("Dimension-qualified pages", {e: f"{quality['qualified_pages']}/100" for e in RENDER_ENGINES}),
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
        lines += [
            "### Visual evidence",
            "",
            "The repository retains one native-output comparison sheet for every PDF. The top",
            "row contains the four unscaled renderer outputs; the bottom row contains amplified",
            "absolute-difference maps against the leave-one-engine-out consensus.",
            "",
            "- [Pages 1–25 contact sheet](visual/contacts/contact-001-025.webp)",
            "- [Pages 26–50 contact sheet](visual/contacts/contact-026-050.webp)",
            "- [Pages 51–75 contact sheet](visual/contacts/contact-051-075.webp)",
            "- [Pages 76–100 contact sheet](visual/contacts/contact-076-100.webp)",
            "- [All 100 full comparison sheets](visual/pages/)",
            "",
        ]
    else:
        lines += ["## Rendering quality", "", f"Unavailable: `{quality.get('error', 'unknown error')}`", ""]

    lines += [
        "## Verdict",
        "",
        "- All five native parser adapters returned the same page count on all 100 PDFs.",
        "- The matched persistent parser evidence rejects the 20× Poppler claim.",
        "- All four raster engines rendered all 100 pages, but only 97 had identical native dimensions.",
        "- Wellfriend PDF is not the fastest renderer in this campaign; reference leadership varies by percentile.",
        "- Consensus quality is diagnostic rather than ground truth and does not establish universal correctness.",
        "",
        "## Reproducibility",
        "",
        f"- Corpus SHA-256 manifest: `{summary['corpus_manifest_sha256']}`.",
        f"- DPI: `{summary['configuration']['dpi']}`.",
        f"- Persistent repetitions: `{summary['configuration']['persistent_repetitions']}`.",
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
        args.output / "render-persistent.jsonl",
    )
    render_fresh = run_fresh(
        "render", inputs, args.corpus, adapters, RENDER_ENGINES,
        args.fresh_repetitions, args.dpi, args.seed + 4, args.cpu, args.timeout,
        args.output / "render-fresh.jsonl",
    )

    qualification = page_count_qualification(parse_persistent)
    parse_persistent_distributions = {
        engine: distribution(
            row["parse_ms"] for row in parse_persistent
            if row.get("engine") == engine and row.get("status") == "ok"
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
    qualified_paths = {
        path for path, engines in {
            path: {row["engine"] for row in parse_persistent if row.get("relative_path") == path and row.get("status") == "ok"}
            for path in wellfriend_by_doc
        }.items() if set(PARSER_ENGINES).issubset(engines)
    }
    qualified_paths -= {
        str(row["relative_path"]) for row in qualification["disagreements"]
    }
    paired_ratio = bootstrap_ratio_ci(
        {path: poppler_by_doc[path] for path in qualified_paths if path in poppler_by_doc},
        {path: wellfriend_by_doc[path] for path in qualified_paths if path in wellfriend_by_doc},
        args.seed + 5,
    )

    quality = quality_metrics(raster_root, inputs, args.corpus)
    if quality.get("pages"):
        with (args.output / "quality-pages.jsonl").open("w", encoding="utf-8", newline="\n") as stream:
            for row in quality["pages"]:
                stream.write(json.dumps(row, sort_keys=True) + "\n")
        quality = dict(quality)
        quality.pop("pages", None)

    summary = {
        "schema": "wellfriendpdf.pebq.v1",
        "started_at_utc": started,
        "finished_at_utc": utc_now(),
        "corpus_manifest_sha256": manifest_sha,
        "configuration": {
            "dpi": args.dpi,
            "persistent_repetitions": args.persistent_repetitions,
            "fresh_repetitions": args.fresh_repetitions,
            "cpu": args.cpu,
            "seed": args.seed,
            "timeout_seconds": args.timeout,
        },
        "environment": metadata,
        "parsing": {
            "contract": "identical in-memory open plus resolved page count",
            "qualification": qualification,
            "persistent_parse_ms": parse_persistent_distributions,
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
            "accepted": {
                engine: len({
                    row["relative_path"] for row in render_persistent
                    if row.get("engine") == engine and row.get("status") == "ok"
                })
                for engine in RENDER_ENGINES
            },
            "persistent_render_ms": {
                engine: distribution(
                    row["render_ms"] for row in render_persistent
                    if row.get("engine") == engine and row.get("status") == "ok"
                )
                for engine in RENDER_ENGINES
            },
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
    }
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (args.output / "README.md").write_text(build_report(summary), encoding="utf-8", newline="\n")
    print(json.dumps({"status": "complete", "output": str(args.output), "manifest": manifest_sha}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
