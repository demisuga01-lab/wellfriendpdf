#!/usr/bin/env python3
"""Compare Wellfriend raster output with local reference renderers.

The script intentionally stores aggregate metrics and path hashes only. It does
not retain rendered page images or source PDF content in the report.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import sys
import tempfile
import time
import warnings
import zipfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from PIL import Image, ImageChops

warnings.filterwarnings("ignore", category=DeprecationWarning)

REFERENCE_RENDER_TIMEOUT_SEC = 120
PAGE_NAME_RE = re.compile(r"(?:^|[/\\])page-(\d+)\.", re.IGNORECASE)


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8", "replace")).hexdigest()


def zip_page_name_key(name: str) -> tuple[int, str]:
    """Sort render ZIP entries by numeric page first, then stable name."""

    match = PAGE_NAME_RE.search(name)
    if match:
        return (int(match.group(1)), name)
    return (sys.maxsize, name)


def collect_pdfs(root: Path, limit: int | None) -> list[Path]:
    files = sorted(path for path in root.rglob("*.pdf") if path.is_file())
    return files[:limit] if limit else files


def load_wellfriend_png(
    wellfriend_bin: Path,
    pdf: Path,
    page_number: int,
    dpi: int,
    tmp: Path,
    render_quality: str,
    timeout_sec: int,
) -> Image.Image:
    out_zip = tmp / "wellfriend.zip"
    subprocess.run(
        [
            str(wellfriend_bin),
            "render",
            str(pdf),
            "--output",
            str(out_zip),
            "--pages",
            str(page_number),
            "--dpi",
            str(dpi),
            "--format",
            "png",
            "--render-quality",
            render_quality,
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=timeout_sec,
    )
    with zipfile.ZipFile(out_zip) as zf:
        names = [name for name in zf.namelist() if name.lower().endswith(".png")]
        if not names:
            raise RuntimeError("wellfriend render zip contained no PNG")
        with zf.open(names[0]) as handle:
            with Image.open(handle) as image:
                return image.convert("RGBA")


def render_wellfriend_pngs_to_dir(
    wellfriend_bin: Path,
    pdf: Path,
    dpi: int,
    tmp: Path,
    output_dir: Path,
    render_quality: str,
    timeout_sec: int,
    page_count: int,
    timeout_multiplier: int,
) -> list[Path]:
    out_zip = tmp / "wellfriend-all.zip"
    subprocess.run(
        [
            str(wellfriend_bin),
            "render",
            str(pdf),
            "--output",
            str(out_zip),
            "--pages",
            "all",
            "--dpi",
            str(dpi),
            "--format",
            "png",
            "--render-quality",
            render_quality,
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=timeout_sec * max(1, page_count) * max(1, timeout_multiplier),
    )
    output_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(out_zip) as zf:
        names = sorted(
            (name for name in zf.namelist() if name.lower().endswith(".png")),
            key=zip_page_name_key,
        )
        paths = []
        for index, name in enumerate(names, start=1):
            path = output_dir / f"page-{index:06d}.png"
            with zf.open(name) as src, path.open("wb") as dst:
                shutil.copyfileobj(src, dst)
            paths.append(path)
    if not paths:
        raise RuntimeError("wellfriend render zip contained no PNG")
    return paths


def pdfium_page_count(pdf: Path) -> int:
    import pypdfium2 as pdfium

    doc = pdfium.PdfDocument(str(pdf))
    try:
        return len(doc)
    finally:
        close = getattr(doc, "close", None)
        if close is not None:
            close()


def render_pdfium_png_to_path(pdf: Path, page_number: int, dpi: int, output: Path) -> None:
    import pypdfium2 as pdfium

    doc = pdfium.PdfDocument(str(pdf))
    try:
        page = doc[page_number - 1]
        try:
            bitmap = page.render(scale=dpi / 72.0)
            try:
                image = bitmap.to_pil().convert("RGBA")
                try:
                    image.save(output)
                finally:
                    image.close()
            finally:
                close = getattr(bitmap, "close", None)
                if close is not None:
                    close()
        finally:
            close = getattr(page, "close", None)
            if close is not None:
                close()
    finally:
        close = getattr(doc, "close", None)
        if close is not None:
                close()


def render_pdfium_pngs_to_dir(pdf: Path, dpi: int, output_dir: Path) -> list[Path]:
    import pypdfium2 as pdfium

    output_dir.mkdir(parents=True, exist_ok=True)
    paths = []
    doc = pdfium.PdfDocument(str(pdf))
    try:
        for page_index in range(len(doc)):
            page = doc[page_index]
            try:
                bitmap = page.render(scale=dpi / 72.0)
                try:
                    image = bitmap.to_pil().convert("RGBA")
                    try:
                        path = output_dir / f"page-{page_index + 1:06d}.png"
                        image.save(path)
                        paths.append(path)
                    finally:
                        image.close()
                finally:
                    close = getattr(bitmap, "close", None)
                    if close is not None:
                        close()
            finally:
                close = getattr(page, "close", None)
                if close is not None:
                    close()
    finally:
        close = getattr(doc, "close", None)
        if close is not None:
            close()
    return paths


def render_mupdf_png_to_path(pdf: Path, page_number: int, dpi: int, output: Path) -> None:
    import fitz

    with fitz.open(str(pdf)) as doc:
        page = doc[page_number - 1]
        pix = page.get_pixmap(matrix=fitz.Matrix(dpi / 72.0, dpi / 72.0), alpha=False)
        image = Image.frombytes("RGB", [pix.width, pix.height], pix.samples).convert("RGBA")
        try:
            image.save(output)
        finally:
            image.close()


def render_mupdf_pngs_to_dir(pdf: Path, dpi: int, output_dir: Path) -> list[Path]:
    import fitz

    output_dir.mkdir(parents=True, exist_ok=True)
    paths = []
    with fitz.open(str(pdf)) as doc:
        for page_index in range(len(doc)):
            page = doc[page_index]
            pix = page.get_pixmap(matrix=fitz.Matrix(dpi / 72.0, dpi / 72.0), alpha=False)
            image = Image.frombytes("RGB", [pix.width, pix.height], pix.samples).convert("RGBA")
            try:
                path = output_dir / f"page-{page_index + 1:06d}.png"
                image.save(path)
                paths.append(path)
            finally:
                image.close()
    return paths


def load_reference_png(
    kind: str, pdf: Path, page_number: int, dpi: int, tmp: Path, timeout_sec: int
) -> Image.Image:
    out_png = tmp / f"{kind}.png"
    subprocess.run(
        [
            sys.executable,
            str(Path(__file__).resolve()),
            "--reference-helper",
            kind,
            "--pdf",
            str(pdf),
            "--page-number",
            str(page_number),
            "--dpi",
            str(dpi),
            "--png-output",
            str(out_png),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=timeout_sec,
    )
    with Image.open(out_png) as image:
        return image.convert("RGBA")


def load_reference_pngs(
    kind: str,
    pdf: Path,
    dpi: int,
    tmp: Path,
    timeout_sec: int,
    page_count: int,
    timeout_multiplier: int,
) -> list[Path]:
    out_dir = tmp / kind
    out_dir.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            sys.executable,
            str(Path(__file__).resolve()),
            "--reference-helper-batch",
            kind,
            "--pdf",
            str(pdf),
            "--dpi",
            str(dpi),
            "--png-output-dir",
            str(out_dir),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=timeout_sec * max(1, page_count) * max(1, timeout_multiplier),
    )
    return sorted(out_dir.glob("page-*.png"))


def load_poppler_png(pdf: Path, page_number: int, dpi: int, tmp: Path) -> Image.Image:
    prefix = tmp / "poppler"
    subprocess.run(
        [
            "pdftoppm",
            "-f",
            str(page_number),
            "-l",
            str(page_number),
            "-r",
            str(dpi),
            "-png",
            str(pdf),
            str(prefix),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=120,
    )
    images = sorted(tmp.glob("poppler-*.png"))
    if not images:
        raise RuntimeError("pdftoppm produced no PNG")
    with Image.open(images[0]) as image:
        return image.convert("RGBA")


def render_poppler_pngs_to_dir(
    pdf: Path,
    dpi: int,
    tmp: Path,
    output_dir: Path,
    page_count: int,
    timeout_multiplier: int,
) -> list[Path]:
    prefix = tmp / "poppler-all"
    subprocess.run(
        [
            "pdftoppm",
            "-r",
            str(dpi),
            "-png",
            str(pdf),
            str(prefix),
        ],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        timeout=REFERENCE_RENDER_TIMEOUT_SEC * max(1, page_count) * max(1, timeout_multiplier),
    )
    output_dir.mkdir(parents=True, exist_ok=True)
    paths = []
    for index, src in enumerate(sorted(tmp.glob("poppler-all-*.png")), start=1):
        dst = output_dir / f"page-{index:06d}.png"
        shutil.move(str(src), str(dst))
        paths.append(dst)
    if not paths:
        raise RuntimeError("pdftoppm produced no PNG")
    return paths


def diff_metric(a: Image.Image, b: Image.Image) -> dict[str, object]:
    if a.size != b.size:
        return {"same_size": False, "size_a": list(a.size), "size_b": list(b.size)}
    with ImageChops.difference(a, b) as diff:
        pixels = diff.getdata()
        total = a.size[0] * a.size[1]
        changed = 0
        max_delta = 0
        for pixel in pixels:
            local = max(pixel)
            if local > 8:
                changed += 1
            if local > max_delta:
                max_delta = local
    return {
        "same_size": True,
        "changed_pixel_threshold8_percentage": round((changed / total) * 100.0, 6)
        if total
        else 0.0,
        "max_channel_delta": max_delta,
    }


def compare_page(task: dict[str, object]) -> dict[str, object]:
    index = int(task["index"])
    pdf = Path(task["pdf"])
    page_number = int(task["page_number"])
    tools = dict(task["tools"])
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        wellfriend: Image.Image | None = None
        refs: dict[str, Image.Image] = {}
        reference_failures = 0
        try:
            wellfriend = load_wellfriend_png(
                Path(task["wellfriend_bin"]),
                pdf,
                page_number,
                int(task["dpi"]),
                tmp,
                str(task["wellfriend_render_quality"]),
                int(task["wellfriend_timeout_sec"]),
            )
            reference_errors = {}
            for ref_name in ("pdfium", "mupdf"):
                if str(tools.get(ref_name)) != "available":
                    continue
                try:
                    refs[ref_name] = load_reference_png(
                        ref_name,
                        pdf,
                        page_number,
                        int(task["dpi"]),
                        tmp,
                        int(task["reference_timeout_sec"]),
                    )
                except Exception as exc:  # noqa: BLE001 - classify without retaining path/content.
                    reference_errors[ref_name] = exc.__class__.__name__
                    reference_failures += 1
            if tools.get("poppler") == "available":
                try:
                    refs["poppler"] = load_poppler_png(pdf, page_number, int(task["dpi"]), tmp)
                except Exception as exc:  # noqa: BLE001 - classify without retaining path/content.
                    reference_errors["poppler"] = exc.__class__.__name__
                    reference_failures += 1
            comparisons = {name: diff_metric(wellfriend, image) for name, image in refs.items()}
            ok = True
        except Exception as exc:  # noqa: BLE001 - report exact class without content.
            comparisons = {}
            reference_errors = {}
            ok = False
            error = exc.__class__.__name__
        else:
            error = None
        finally:
            for image in refs.values():
                image.close()
            if wellfriend is not None:
                wellfriend.close()
    return {
        "index": index,
        "page": {
            "path_sha256": sha256_text(str(pdf)),
            "page_number": page_number,
            "ok": ok,
            "error_class": error,
            "reference_errors": reference_errors,
            "comparisons": comparisons,
        },
        "failures": 0 if ok else 1,
        "reference_failures": reference_failures,
    }


def compare_pdf_batch(task: dict[str, object]) -> dict[str, object]:
    first_index = int(task["first_index"])
    pdf = Path(task["pdf"])
    page_count = int(task["page_count"])
    timeout_multiplier = int(task.get("batch_timeout_multiplier", 1))
    tools = dict(task["tools"])
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        reference_failures = 0
        pages = []
        try:
            wellfriend_paths = render_wellfriend_pngs_to_dir(
                Path(task["wellfriend_bin"]),
                pdf,
                int(task["dpi"]),
                tmp,
                tmp / "wellfriend",
                str(task["wellfriend_render_quality"]),
                int(task["wellfriend_timeout_sec"]),
                page_count,
                timeout_multiplier,
            )
            reference_paths: dict[str, list[Path]] = {}
            batch_reference_errors_by_name = {}
            for ref_name in ("pdfium", "mupdf"):
                if str(tools.get(ref_name)) != "available":
                    continue
                try:
                    reference_paths[ref_name] = load_reference_pngs(
                        ref_name,
                        pdf,
                        int(task["dpi"]),
                        tmp,
                        int(task["reference_timeout_sec"]),
                        page_count,
                        timeout_multiplier,
                    )
                except Exception as exc:  # noqa: BLE001 - classify without retaining path/content.
                    batch_reference_errors_by_name[ref_name] = exc.__class__.__name__
            if tools.get("poppler") == "available":
                try:
                    reference_paths["poppler"] = render_poppler_pngs_to_dir(
                        pdf,
                        int(task["dpi"]),
                        tmp,
                        tmp / "poppler",
                        page_count,
                        timeout_multiplier,
                    )
                except Exception as exc:  # noqa: BLE001 - classify without retaining path/content.
                    batch_reference_errors_by_name["poppler"] = exc.__class__.__name__
            for page_number in range(1, page_count + 1):
                comparisons = {}
                reference_errors = {}
                if page_number <= len(wellfriend_paths):
                    with Image.open(wellfriend_paths[page_number - 1]) as wf_image:
                        wellfriend = wf_image.convert("RGBA")
                    try:
                        for ref_name in ("pdfium", "mupdf", "poppler"):
                            if str(tools.get(ref_name)) != "available":
                                continue
                            paths = reference_paths.get(ref_name)
                            if paths is None:
                                if ref_name not in batch_reference_errors_by_name:
                                    continue
                                try:
                                    if ref_name == "poppler":
                                        ref = load_poppler_png(
                                            pdf, page_number, int(task["dpi"]), tmp
                                        )
                                    else:
                                        ref = load_reference_png(
                                            ref_name,
                                            pdf,
                                            page_number,
                                            int(task["dpi"]),
                                            tmp,
                                            int(task["reference_timeout_sec"])
                                            * max(1, timeout_multiplier),
                                        )
                                except Exception as exc:  # noqa: BLE001 - classify without retaining content.
                                    reference_errors[ref_name] = (
                                        f"{batch_reference_errors_by_name[ref_name]}->{exc.__class__.__name__}"
                                    )
                                    reference_failures += 1
                                    continue
                                try:
                                    comparisons[ref_name] = diff_metric(wellfriend, ref)
                                finally:
                                    ref.close()
                                continue
                            if page_number > len(paths):
                                reference_errors[ref_name] = "MissingRenderedPage"
                                reference_failures += 1
                                continue
                            with Image.open(paths[page_number - 1]) as ref_image:
                                ref = ref_image.convert("RGBA")
                            try:
                                comparisons[ref_name] = diff_metric(wellfriend, ref)
                            finally:
                                ref.close()
                        ok = True
                        error = None
                    finally:
                        wellfriend.close()
                else:
                    ok = False
                    error = "MissingRenderedPage"
                pages.append(
                    {
                        "path_sha256": sha256_text(str(pdf)),
                        "page_number": page_number,
                        "ok": ok,
                        "error_class": error,
                        "reference_errors": reference_errors,
                        "comparisons": comparisons,
                    }
                )
        except Exception as exc:  # noqa: BLE001 - report exact class without content.
            pages = [
                {
                    "path_sha256": sha256_text(str(pdf)),
                    "page_number": page_number,
                    "ok": False,
                    "error_class": exc.__class__.__name__,
                    "reference_errors": {},
                    "comparisons": {},
                }
                for page_number in range(1, page_count + 1)
            ]
    failures = sum(1 for page in pages if not page["ok"])
    return {
        "first_index": first_index,
        "pages": pages,
        "failures": failures,
        "reference_failures": reference_failures,
    }


def summarize_batch_result(result: dict[str, object]) -> dict[str, object]:
    pages = list(result["pages"])
    error_classes: dict[str, int] = {}
    reference_error_classes: dict[str, int] = {}
    reference_error_by_name_class: dict[str, int] = {}
    for page in pages:
        page_dict = dict(page)
        error_class = page_dict.get("error_class")
        if error_class:
            key = str(error_class)
            error_classes[key] = error_classes.get(key, 0) + 1
        for reference_name, reference_error in dict(page_dict.get("reference_errors") or {}).items():
            key = str(reference_error)
            reference_error_classes[key] = reference_error_classes.get(key, 0) + 1
            by_name_key = f"{reference_name}:{key}"
            reference_error_by_name_class[by_name_key] = (
                reference_error_by_name_class.get(by_name_key, 0) + 1
            )
    return {
        "first_index": int(result["first_index"]),
        "pages": len(pages),
        "failures": int(result["failures"]),
        "reference_failures": int(result["reference_failures"]),
        "error_classes": error_classes,
        "reference_error_classes": reference_error_classes,
        "reference_error_by_name_class": reference_error_by_name_class,
    }


def parse_reference_tools(value: str) -> list[str]:
    selected: list[str] = []
    valid = {"pdfium", "mupdf", "poppler"}
    for raw_part in value.split(","):
        name = raw_part.strip().lower()
        if not name:
            continue
        if name not in valid:
            raise argparse.ArgumentTypeError(
                "reference tools must be a comma-separated subset of pdfium,mupdf,poppler"
            )
        if name not in selected:
            selected.append(name)
    if not selected:
        raise argparse.ArgumentTypeError("at least one reference tool is required")
    return selected


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--reference-helper", choices=["pdfium", "mupdf"])
    parser.add_argument("--reference-helper-batch", choices=["pdfium", "mupdf"])
    parser.add_argument("--pdf", type=Path)
    parser.add_argument("--page-number", type=int, default=1)
    parser.add_argument("--png-output", type=Path)
    parser.add_argument("--png-output-dir", type=Path)
    parser.add_argument("--corpus", type=Path)
    parser.add_argument("--wellfriend-bin", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--limit", type=int, default=25)
    parser.add_argument("--pages", choices=["first", "all"], default="first")
    parser.add_argument("--dpi", type=int, default=72)
    parser.add_argument("--wellfriend-render-quality", default="compat")
    parser.add_argument("--wellfriend-timeout-sec", type=int, default=120)
    parser.add_argument("--reference-timeout-sec", type=int, default=REFERENCE_RENDER_TIMEOUT_SEC)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--batch-by-file", action="store_true")
    parser.add_argument(
        "--reference-tools",
        type=parse_reference_tools,
        default=parse_reference_tools("pdfium,mupdf,poppler"),
        help="comma-separated subset of pdfium,mupdf,poppler for corpus comparisons",
    )
    args = parser.parse_args()

    if args.reference_helper:
        if args.pdf is None or args.png_output is None:
            return 2
        if args.reference_helper == "pdfium":
            render_pdfium_png_to_path(args.pdf, args.page_number, args.dpi, args.png_output)
        else:
            render_mupdf_png_to_path(args.pdf, args.page_number, args.dpi, args.png_output)
        return 0

    if args.reference_helper_batch:
        if args.pdf is None or args.png_output_dir is None:
            return 2
        if args.reference_helper_batch == "pdfium":
            render_pdfium_pngs_to_dir(args.pdf, args.dpi, args.png_output_dir)
        else:
            render_mupdf_pngs_to_dir(args.pdf, args.dpi, args.png_output_dir)
        return 0

    if args.corpus is None or args.wellfriend_bin is None or args.output is None:
        return 2

    tools = {}
    for reference_tool in args.reference_tools:
        if reference_tool == "poppler":
            tools[reference_tool] = "available" if shutil.which("pdftoppm") else "unavailable"
        else:
            tools[reference_tool] = "available"
    pages = []
    failures = 0
    reference_failures = 0
    start = time.time()
    pdfs = collect_pdfs(args.corpus, args.limit)
    page_plan: list[tuple[Path, int]] = []
    pdf_page_counts: list[tuple[Path, int]] = []
    if args.pages == "all":
        for pdf in pdfs:
            try:
                page_count = max(1, pdfium_page_count(pdf))
            except Exception:  # noqa: BLE001 - exact failure is recorded per attempted page below.
                page_count = 1
            pdf_page_counts.append((pdf, page_count))
            page_plan.extend((pdf, page_number) for page_number in range(1, page_count + 1))
    else:
        page_plan = [(pdf, 1) for pdf in pdfs]
    progress_path = args.output.with_suffix(args.output.suffix + ".progress.json")
    batch_results_path = args.output.with_suffix(args.output.suffix + ".batches.jsonl")
    workers = max(1, args.workers)
    tasks = [
        {
            "index": index,
            "pdf": str(pdf),
            "page_number": page_number,
            "tools": tools,
            "wellfriend_bin": str(args.wellfriend_bin),
            "dpi": args.dpi,
            "wellfriend_render_quality": args.wellfriend_render_quality,
            "wellfriend_timeout_sec": args.wellfriend_timeout_sec,
            "reference_timeout_sec": args.reference_timeout_sec,
        }
        for index, (pdf, page_number) in enumerate(page_plan, start=1)
    ]
    completed = 0
    next_progress = 100
    if args.batch_by_file and args.pages == "all":
        batch_tasks = []
        first_index = 1
        for pdf, page_count in pdf_page_counts:
            batch_tasks.append(
                {
                    "first_index": first_index,
                    "pdf": str(pdf),
                    "page_count": page_count,
                    "tools": tools,
                    "wellfriend_bin": str(args.wellfriend_bin),
                    "dpi": args.dpi,
                    "wellfriend_render_quality": args.wellfriend_render_quality,
                    "wellfriend_timeout_sec": args.wellfriend_timeout_sec,
                    "reference_timeout_sec": args.reference_timeout_sec,
                    "batch_timeout_multiplier": workers,
                }
            )
            first_index += page_count
        results: list[dict[str, object] | None] = [None] * len(page_plan)
        with ThreadPoolExecutor(max_workers=workers) as executor:
            future_map = {executor.submit(compare_pdf_batch, task): task for task in batch_tasks}
            for future in as_completed(future_map):
                result = future.result()
                batch_pages = list(result["pages"])
                completed += len(batch_pages)
                failures += int(result["failures"])
                reference_failures += int(result["reference_failures"])
                with batch_results_path.open("a", encoding="utf-8") as batch_stream:
                    batch_stream.write(json.dumps(summarize_batch_result(result)) + "\n")
                first = int(result["first_index"]) - 1
                for offset, page in enumerate(batch_pages):
                    if first + offset < len(results):
                        results[first + offset] = page
                if completed >= len(page_plan) or completed >= next_progress:
                    progress_path.write_text(
                        json.dumps(
                            {
                                "schema_version": "wellfriend.reference_render_compare.progress.v1",
                                "attempted": completed,
                                "total": len(page_plan),
                                "failures": failures,
                                "reference_failures": reference_failures,
                                "elapsed_sec": round(time.time() - start, 3),
                            },
                            indent=2,
                        ),
                        encoding="utf-8",
                    )
                    while next_progress <= completed:
                        next_progress += 100
        pages = [page for page in results if page is not None]
    else:
        results = [None] * len(tasks)
        with ThreadPoolExecutor(max_workers=workers) as executor:
            future_map = {executor.submit(compare_page, task): task for task in tasks}
            for future in as_completed(future_map):
                result = future.result()
                completed += 1
                failures += int(result["failures"])
                reference_failures += int(result["reference_failures"])
                results[int(result["index"]) - 1] = result["page"]
                if completed == len(page_plan) or completed >= next_progress:
                    progress_path.write_text(
                        json.dumps(
                            {
                                "schema_version": "wellfriend.reference_render_compare.progress.v1",
                                "attempted": completed,
                                "total": len(page_plan),
                                "failures": failures,
                                "reference_failures": reference_failures,
                                "elapsed_sec": round(time.time() - start, 3),
                            },
                            indent=2,
                        ),
                        encoding="utf-8",
                    )
                    while next_progress <= completed:
                        next_progress += 100
        pages = [page for page in results if page is not None]

    report = {
        "schema_version": "wellfriend.reference_render_compare.v1",
        "dpi": args.dpi,
        "limit": args.limit,
        "wellfriend_render_quality": args.wellfriend_render_quality,
        "files_attempted": len(pdfs),
        "pages_mode": args.pages,
        "workers": workers,
        "batch_by_file": bool(args.batch_by_file and args.pages == "all"),
        "pages_attempted": len(pages),
        "failures": failures,
        "reference_failures": reference_failures,
        "duration_sec": round(time.time() - start, 3),
        "requested_reference_tools": args.reference_tools,
        "tools": tools,
        "pages": pages,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2), encoding="utf-8")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
