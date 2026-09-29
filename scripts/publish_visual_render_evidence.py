#!/usr/bin/env python3
"""Publish human-viewable, reproducible cross-renderer PDF evidence.

Unlike render_reference_compare.py, this harness deliberately retains compact
comparison sheets. Full-resolution rasters remain transient, but their hashes,
dimensions, render durations, and pixel metrics are recorded for every page.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import shutil
import subprocess
import sys
import tempfile
import time
import zipfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from PIL import Image, ImageChops, ImageDraw, ImageFont


ENGINES = ("wellfriendpdf", "pdfium", "mupdf", "poppler")
REFERENCES = ("pdfium", "mupdf", "poppler")
PAIRWISE_ENGINES = (
    ("wellfriendpdf", "pdfium"),
    ("wellfriendpdf", "mupdf"),
    ("wellfriendpdf", "poppler"),
    ("pdfium", "mupdf"),
    ("pdfium", "poppler"),
    ("mupdf", "poppler"),
)
ENGINE_LABELS = {
    "wellfriendpdf": "Wellfriend PDF",
    "pdfium": "PDFium",
    "mupdf": "MuPDF",
    "poppler": "Poppler",
}
# Individual sheets are evidence, not thumbnails.  A 600-pixel-wide page panel
# keeps ordinary document text recognizable when the WebP is opened or zoomed;
# the separate contact sheets remain compact corpus overviews.
PANEL_WIDTH = 600
PANEL_HEIGHT = 840
PANEL_GAP = 16
TITLE_HEIGHT = 50


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_name(value: str) -> str:
    filtered = "".join(char.lower() if char.isalnum() else "-" for char in value)
    return "-".join(part for part in filtered.split("-") if part)[:72] or "pdf"


def run_timed(argv: list[str], timeout: int) -> dict[str, Any]:
    started = time.perf_counter()
    completed = subprocess.run(
        argv,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )
    return {
        "argv": argv,
        "exit": completed.returncode,
        "duration_ms": round((time.perf_counter() - started) * 1000.0, 3),
        "stderr_tail": completed.stderr.decode("utf-8", "replace")[-2000:],
    }


def load_rgb(path: Path) -> Image.Image:
    with Image.open(path) as image:
        return image.convert("RGB")


def normalized_png(image: Image.Image, output: Path) -> dict[str, Any]:
    image.save(output, format="PNG", optimize=False, compress_level=6)
    return {
        "sha256": sha256_file(output),
        "width": image.width,
        "height": image.height,
        "mode": image.mode,
        "bytes": output.stat().st_size,
    }


def render_wellfriendpdf(binary: Path, pdf: Path, dpi: int, work: Path, timeout: int) -> tuple[Image.Image, dict[str, Any]]:
    output_zip = work / "wellfriendpdf.zip"
    result = run_timed(
        [
            str(binary),
            "render",
            str(pdf),
            "--output",
            str(output_zip),
            "--pages",
            "1",
            "--dpi",
            str(dpi),
            "--format",
            "png",
            "--render-quality",
            "compat",
        ],
        timeout,
    )
    if result["exit"] != 0:
        raise RuntimeError(f"Wellfriend PDF exited {result['exit']}: {result['stderr_tail']}")
    with zipfile.ZipFile(output_zip) as archive:
        names = sorted(name for name in archive.namelist() if name.lower().endswith(".png"))
        if not names:
            raise RuntimeError("Wellfriend PDF produced no PNG")
        extracted = work / "wellfriendpdf-source.png"
        with archive.open(names[0]) as source, extracted.open("wb") as target:
            shutil.copyfileobj(source, target)
    return load_rgb(extracted), result


def render_helper(
    helper: Path,
    engine: str,
    pdf: Path,
    dpi: int,
    work: Path,
    timeout: int,
) -> tuple[Image.Image, dict[str, Any]]:
    output = work / f"{engine}-source.png"
    result = run_timed(
        [
            sys.executable,
            str(helper),
            "--reference-helper",
            engine,
            "--pdf",
            str(pdf),
            "--page-number",
            "1",
            "--dpi",
            str(dpi),
            "--png-output",
            str(output),
        ],
        timeout,
    )
    if result["exit"] != 0 or not output.exists():
        raise RuntimeError(f"{engine} exited {result['exit']}: {result['stderr_tail']}")
    return load_rgb(output), result


def render_poppler(pdf: Path, dpi: int, work: Path, timeout: int) -> tuple[Image.Image, dict[str, Any]]:
    prefix = work / "poppler-source"
    result = run_timed(
        [
            "pdftoppm",
            "-f",
            "1",
            "-l",
            "1",
            "-r",
            str(dpi),
            "-png",
            "-singlefile",
            str(pdf),
            str(prefix),
        ],
        timeout,
    )
    output = prefix.with_suffix(".png")
    if result["exit"] != 0 or not output.exists():
        raise RuntimeError(f"Poppler exited {result['exit']}: {result['stderr_tail']}")
    return load_rgb(output), result


def diff_metric(base: Image.Image, reference: Image.Image) -> dict[str, Any]:
    if base.size != reference.size:
        return {
            "same_size": False,
            "wellfriendpdf_size": list(base.size),
            "reference_size": list(reference.size),
        }
    with ImageChops.difference(base, reference) as difference:
        total = base.width * base.height
        channels = difference.split()
        try:
            channel_histograms = [channel.histogram() for channel in channels]
            absolute_sum = sum(
                delta * count
                for histogram in channel_histograms
                for delta, count in enumerate(histogram)
            )
            squared_sum = sum(
                delta * delta * count
                for histogram in channel_histograms
                for delta, count in enumerate(histogram)
            )
            with ImageChops.lighter(channels[0], channels[1]) as red_green_max:
                with ImageChops.lighter(red_green_max, channels[2]) as per_pixel_max:
                    maximum_histogram = per_pixel_max.histogram()
            changed = sum(maximum_histogram[9:])
            max_delta = next(
                (delta for delta in range(255, -1, -1) if maximum_histogram[delta]),
                0,
            )
        finally:
            for channel in channels:
                channel.close()
    mean_squared_delta = squared_sum / (total * 3.0)
    rmse = math.sqrt(mean_squared_delta)
    return {
        "same_size": True,
        "changed_pixel_threshold8_percentage": round(changed * 100.0 / total, 6),
        "mean_absolute_channel_delta": round(absolute_sum / (total * 3.0), 6),
        "mean_squared_channel_delta": round(mean_squared_delta, 6),
        "root_mean_squared_channel_delta": round(rmse, 6),
        "psnr_db": None if mean_squared_delta == 0.0 else round(10.0 * math.log10((255.0**2) / mean_squared_delta), 6),
        "max_channel_delta": max_delta,
    }


def fit_panel(image: Image.Image, label: str) -> Image.Image:
    canvas = Image.new("RGB", (PANEL_WIDTH, PANEL_HEIGHT), "white")
    copy = image.copy()
    copy.thumbnail((PANEL_WIDTH, PANEL_HEIGHT - TITLE_HEIGHT), Image.Resampling.LANCZOS)
    left = (PANEL_WIDTH - copy.width) // 2
    top = TITLE_HEIGHT + (PANEL_HEIGHT - TITLE_HEIGHT - copy.height) // 2
    canvas.paste(copy, (left, top))
    ImageDraw.Draw(canvas).text((12, 14), label, fill="black", font=ImageFont.load_default())
    copy.close()
    return canvas


def heatmap(base: Image.Image, reference: Image.Image) -> Image.Image:
    if base.size != reference.size:
        return Image.new("RGB", base.size, (160, 0, 0))
    with ImageChops.difference(base, reference) as difference:
        return difference.point(lambda value: min(255, value * 4))


def comparison_sheet(
    images: dict[str, Image.Image],
    metrics: dict[str, dict[str, Any]],
    durations: dict[str, float],
    title: str,
    output: Path,
) -> None:
    panels: list[Image.Image] = []
    for engine in ENGINES:
        panels.append(
            fit_panel(images[engine], f"{ENGINE_LABELS[engine]} - {durations[engine]:.3f} ms")
        )
    for reference in REFERENCES:
        diff = heatmap(images["wellfriendpdf"], images[reference])
        metric = metrics[reference]
        value = metric.get("changed_pixel_threshold8_percentage", "size mismatch")
        panels.append(fit_panel(diff, f"diff x4: Wellfriend PDF vs {reference} - changed > 8: {value}%"))
        diff.close()
    panels.append(fit_panel(Image.new("RGB", (10, 10), "white"), "Black/bright regions are amplified pixel deltas"))
    width = PANEL_WIDTH * 4 + PANEL_GAP * 5
    height = PANEL_HEIGHT * 2 + PANEL_GAP * 3 + 34
    sheet = Image.new("RGB", (width, height), (235, 235, 235))
    draw = ImageDraw.Draw(sheet)
    draw.text((PANEL_GAP, 8), title, fill="black", font=ImageFont.load_default())
    y_offset = 34
    for index, panel in enumerate(panels):
        row, column = divmod(index, 4)
        x = PANEL_GAP + column * (PANEL_WIDTH + PANEL_GAP)
        y = y_offset + PANEL_GAP + row * (PANEL_HEIGHT + PANEL_GAP)
        sheet.paste(panel, (x, y))
        panel.close()
    output.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(output, format="WEBP", quality=88, method=6)
    sheet.close()


def compare_one(task: dict[str, Any]) -> dict[str, Any]:
    pdf = Path(task["pdf"])
    index = int(task["index"])
    started_at = utc_now()
    started = time.perf_counter()
    record: dict[str, Any] = {
        "index": index,
        "corpus": task["corpus"],
        "relative_path": task["relative_path"],
        "input_sha256": sha256_file(pdf),
        "started_at_utc": started_at,
    }
    with tempfile.TemporaryDirectory(prefix="wellfriendpdf-visual-") as temporary:
        work = Path(temporary)
        images: dict[str, Image.Image] = {}
        commands: dict[str, dict[str, Any]] = {}
        try:
            rotation = (index - 1) % len(ENGINES)
            execution_order = ENGINES[rotation:] + ENGINES[:rotation]
            for engine in execution_order:
                if engine == "wellfriendpdf":
                    images[engine], commands[engine] = render_wellfriendpdf(
                        Path(task["wellfriend_bin"]),
                        pdf,
                        int(task["dpi"]),
                        work,
                        int(task["timeout"]),
                    )
                elif engine == "poppler":
                    images[engine], commands[engine] = render_poppler(
                        pdf, int(task["dpi"]), work, int(task["timeout"])
                    )
                else:
                    images[engine], commands[engine] = render_helper(
                        Path(task["helper"]),
                        engine,
                        pdf,
                        int(task["dpi"]),
                        work,
                        int(task["timeout"]),
                    )
            raster_identity = {}
            for engine, image in images.items():
                raster_identity[engine] = normalized_png(image, work / f"{engine}-normalized.png")
            pairwise_metrics = {
                f"{left}_vs_{right}": diff_metric(images[left], images[right])
                for left, right in PAIRWISE_ENGINES
            }
            metrics = {
                reference: pairwise_metrics[f"wellfriendpdf_vs_{reference}"]
                for reference in REFERENCES
            }
            durations = {engine: float(commands[engine]["duration_ms"]) for engine in ENGINES}
            artifact_name = f"{index:03d}-{safe_name(str(task['relative_path']))}.webp"
            artifact = Path(task["artifact_dir"]) / artifact_name
            comparison_sheet(
                images,
                metrics,
                durations,
                f"{task['corpus']} #{index}: {task['relative_path']} - page 1 at {task['dpi']} DPI",
                artifact,
            )
            record.update(
                status="pass",
                render_execution_order=list(execution_order),
                commands=commands,
                rasters=raster_identity,
                comparisons=metrics,
                pairwise_comparisons=pairwise_metrics,
                artifact=artifact_name,
                artifact_sha256=sha256_file(artifact),
                artifact_bytes=artifact.stat().st_size,
            )
        except Exception as exc:  # noqa: BLE001 - exact failure class and message are evidence.
            record.update(status="fail", error_class=type(exc).__name__, error=str(exc))
        finally:
            for image in images.values():
                image.close()
    record["finished_at_utc"] = utc_now()
    record["elapsed_ms"] = round((time.perf_counter() - started) * 1000.0, 3)
    return record


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = round((len(ordered) - 1) * fraction)
    return round(ordered[index], 3)


def duration_summary(records: list[dict[str, Any]], engine: str) -> dict[str, Any]:
    values = [float(record["commands"][engine]["duration_ms"]) for record in records if record["status"] == "pass"]
    return {
        "n": len(values),
        "p50_ms": percentile(values, 0.50),
        "p90_ms": percentile(values, 0.90),
        "p95_ms": percentile(values, 0.95),
        "p99_ms": percentile(values, 0.99),
        "max_ms": round(max(values), 3) if values else None,
    }


def make_contact_sheets(records: list[dict[str, Any]], artifact_dir: Path, output_dir: Path) -> list[dict[str, Any]]:
    passed = [record for record in records if record["status"] == "pass"]
    outputs = []
    for group_index in range(0, len(passed), 25):
        group = passed[group_index : group_index + 25]
        tile_width, tile_height = 300, 230
        columns = 5
        rows = math.ceil(len(group) / columns)
        sheet = Image.new("RGB", (columns * tile_width, rows * tile_height), "white")
        for slot, record in enumerate(group):
            with Image.open(artifact_dir / record["artifact"]) as source:
                tile = source.convert("RGB")
                tile.thumbnail((tile_width - 8, tile_height - 24), Image.Resampling.LANCZOS)
            x = (slot % columns) * tile_width + 4
            y = (slot // columns) * tile_height + 20
            sheet.paste(tile, (x, y))
            ImageDraw.Draw(sheet).text((x, y - 16), f"#{record['index']} {record['relative_path'][:34]}", fill="black")
            tile.close()
        name = f"contact-{group_index + 1:03d}-{group_index + len(group):03d}.webp"
        path = output_dir / name
        output_dir.mkdir(parents=True, exist_ok=True)
        sheet.save(path, format="WEBP", quality=84, method=6)
        sheet.close()
        outputs.append({"file": name, "sha256": sha256_file(path), "bytes": path.stat().st_size})
    return outputs


def collect_inputs(corpus: Path, limit: int) -> list[tuple[Path, str]]:
    files = sorted(path for path in corpus.rglob("*.pdf") if path.is_file())[:limit]
    return [(path, path.relative_to(corpus).as_posix()) for path in files]


def collect_edited_inputs(
    editing_results: Path, editing_root: Path, limit: int
) -> list[tuple[Path, str]]:
    rows = [
        json.loads(line)
        for line in editing_results.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    rows = [row for row in rows if "index" in row and "input_sha256" in row]
    rows.sort(key=lambda row: int(row["index"]))
    inputs = []
    for row in rows[:limit]:
        index = int(row["index"])
        digest = str(row["input_sha256"])
        edited = editing_root / "files" / f"{index:03d}-{digest[:16]}" / "edited.pdf"
        if not edited.is_file():
            raise FileNotFoundError(f"edited output is missing: {edited}")
        inputs.append((edited, str(row["relative_path"])))
    return inputs


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--editing-results", type=Path)
    parser.add_argument("--editing-root", type=Path)
    parser.add_argument("--corpus-label", required=True)
    parser.add_argument("--wellfriend-bin", required=True, type=Path)
    parser.add_argument("--reference-helper", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--dpi", type=int, default=144)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--timeout-sec", type=int, default=180)
    args = parser.parse_args()

    if args.editing_results is not None:
        if args.editing_root is None or args.corpus is not None:
            parser.error("--editing-results requires --editing-root and cannot be combined with --corpus")
    elif args.corpus is None:
        parser.error("provide --corpus or --editing-results with --editing-root")

    started_at = utc_now()
    started = time.perf_counter()
    artifacts = args.output_dir / "pages"
    contacts = args.output_dir / "contacts"
    artifacts.mkdir(parents=True, exist_ok=False)
    if args.editing_results is not None:
        inputs = collect_edited_inputs(args.editing_results, args.editing_root, args.limit)
    else:
        inputs = collect_inputs(args.corpus, args.limit)
    tasks = [
        {
            "index": index,
            "pdf": str(pdf),
            "relative_path": relative_path,
            "corpus": args.corpus_label,
            "wellfriend_bin": str(args.wellfriend_bin),
            "helper": str(args.reference_helper),
            "artifact_dir": str(artifacts),
            "dpi": args.dpi,
            "timeout": args.timeout_sec,
        }
        for index, (pdf, relative_path) in enumerate(inputs, start=1)
    ]
    records: list[dict[str, Any] | None] = [None] * len(tasks)
    with ThreadPoolExecutor(max_workers=max(1, args.workers)) as executor:
        futures = {executor.submit(compare_one, task): task for task in tasks}
        for completed, future in enumerate(as_completed(futures), start=1):
            record = future.result()
            records[int(record["index"]) - 1] = record
            print(f"[{completed}/{len(tasks)}] {record['status']} {record['relative_path']}", flush=True)
    final_records = [record for record in records if record is not None]
    with (args.output_dir / "results.jsonl").open("w", encoding="utf-8") as stream:
        for record in final_records:
            stream.write(json.dumps(record, sort_keys=True) + "\n")
    contact_sheets = make_contact_sheets(final_records, artifacts, contacts)
    summary = {
        "schema_version": "wellfriend.visual_render_evidence.v1",
        "corpus": args.corpus_label,
        "started_at_utc": started_at,
        "finished_at_utc": utc_now(),
        "wall_seconds": round(time.perf_counter() - started, 3),
        "dpi": args.dpi,
        "workers": args.workers,
        "timing_protocol": {
            "renderer_order": "deterministic Latin rotation by one-based corpus index",
            "recommended_workers_for_comparable_timings": 1,
            "process_scope": "one fresh renderer process per PDF page and tool",
        },
        "files_attempted": len(final_records),
        "passed": sum(record["status"] == "pass" for record in final_records),
        "failed": sum(record["status"] != "pass" for record in final_records),
        "render_duration": {
            engine: duration_summary(final_records, engine) for engine in ENGINES
        },
        "contact_sheets": contact_sheets,
        "retention_policy": {
            "retained": "one labeled WebP comparison with three amplified diff panels per page, contact sheets, raster hashes, dimensions, durations, and metrics",
            "transient": "full-resolution PNG rasters",
        },
    }
    (args.output_dir / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    return 0 if summary["failed"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
