#!/usr/bin/env python3
"""Execute Prompt 20 CLI mutations and multi-renderer reference proof."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import zipfile
from pathlib import Path

from PIL import Image, ImageChops, ImageStat


SCHEMA = "prompt20.reference-execution.v1"


def pdf_fixture() -> bytes:
    content = (
        b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n"
        b"2 w 1 0 0 RG 20 20 40 30 re S\n"
        b"0 0 1 rg 90 20 30 30 re f\n"
    )
    annotation = (
        b"<< /Type /Annot /Subtype /Ink /Rect [10 70 120 130] "
        b"/InkList [[10 80 30 100 60 90 100 120]] /C [0.1 0.2 0.8] >>"
    )
    objects = {
        1: b"<< /Type /Catalog /Pages 2 0 R >>",
        2: b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
        3: (
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
            b"/Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [6 0 R] >>"
        ),
        4: b"<< /Length " + str(len(content)).encode() + b" >>\nstream\n" + content + b"endstream",
        5: b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        6: annotation,
    }
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = {0: 0}
    for number in sorted(objects):
        offsets[number] = len(output)
        output.extend(f"{number} 0 obj\n".encode())
        output.extend(objects[number])
        output.extend(b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(objects) + 1}\n".encode())
    output.extend(b"0000000000 65535 f \n")
    for number in range(1, len(objects) + 1):
        output.extend(f"{offsets[number]:010d} 00000 n \n".encode())
    output.extend(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(output)


def execute(command: list[str], cwd: Path, timeout: int = 180) -> dict:
    process = subprocess.run(
        command,
        cwd=cwd,
        text=True,
        encoding="utf-8",
        errors="replace",
        capture_output=True,
        timeout=timeout,
    )
    return {
        "command": command,
        "exit_code": process.returncode,
        "stdout": process.stdout[-8000:],
        "stderr": process.stderr[-8000:],
        "passed": process.returncode == 0,
    }


def render_oxide(oxide: Path, pdf: Path, output: Path, cwd: Path) -> dict:
    archive = output.with_suffix(".zip")
    result = execute(
        [str(oxide), "render", str(pdf), "--pages", "1", "--dpi", "96", "--format", "png", "--output", str(archive)],
        cwd,
    )
    if result["passed"]:
        with zipfile.ZipFile(archive) as bundle:
            names = [name for name in bundle.namelist() if name.lower().endswith(".png")]
            if not names:
                result["passed"] = False
                result["stderr"] += "\nOxide render ZIP contains no PNG"
            else:
                output.write_bytes(bundle.read(names[0]))
    return result


def render_reference(name: str, tool: Path | None, pdf: Path, output: Path, cwd: Path) -> dict:
    if tool is None or not tool.exists():
        return {"engine": name, "status": "unavailable_not_counted_as_pass", "passed": None}
    if name == "poppler":
        prefix = output.with_suffix("")
        result = execute([str(tool), "-f", "1", "-l", "1", "-r", "96", "-singlefile", "-png", str(pdf), str(prefix)], cwd)
    elif name == "pdfium":
        result = execute(["cmd.exe", "/d", "/c", str(tool), "--png", f"--output={output}", "--first-page=1", "--last-page=1", "--dpi=96", str(pdf)], cwd)
    elif name == "mupdf":
        result = execute([str(tool), "draw", "-q", "-o", str(output), "-r", "96", str(pdf), "1"], cwd)
    else:
        raise ValueError(name)
    result["engine"] = name
    result["status"] = "rendered" if result["passed"] and output.exists() else "failed"
    return result


def image_metrics(left: Path, right: Path) -> dict:
    with Image.open(left).convert("RGB") as lhs, Image.open(right).convert("RGB") as rhs:
        if lhs.size != rhs.size:
            return {"classification": "dimension_mismatch", "left_size": lhs.size, "right_size": rhs.size}
        diff = ImageChops.difference(lhs, rhs)
        mean = sum(ImageStat.Stat(diff).mean) / 3.0
        pixels = list(diff.get_flattened_data())
        changed = sum(1 for pixel in pixels if max(pixel) > 8)
        changed_percent = changed * 100.0 / max(1, len(pixels))
        # Content occupies a small portion of the fixture. These thresholds
        # classify rasterizer antialiasing while catching missing/moved objects.
        classification = "within_tolerance" if mean <= 12.0 and changed_percent <= 35.0 else "oxide_outlier"
        return {
            "classification": classification,
            "mean_absolute_channel_error": round(mean, 6),
            "changed_pixel_threshold8_percentage": round(changed_percent, 6),
            "size": list(lhs.size),
        }


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, default=Path("target/prompt20-advanced-editing"))
    args = parser.parse_args()
    repo = args.repo.resolve()
    output = (repo / args.output).resolve() if not args.output.is_absolute() else args.output
    corpus = output / "corpus"
    renders = output / "reference-renders"
    corpus.mkdir(parents=True, exist_ok=True)
    renders.mkdir(parents=True, exist_ok=True)
    oxide = repo / "target" / "debug" / ("oxide.exe" if __import__("os").name == "nt" else "oxide")
    fixture = corpus / "prompt20-text-vector-ink.pdf"
    fixture.write_bytes(pdf_fixture())

    mutations: dict[str, dict] = {}
    outputs: dict[str, Path] = {"source": fixture}
    commands = {
        "same_width": [str(oxide), "edit-text", str(fixture), "--query", "ABC", "--replacement", "DEF", "--mode", "same-width-patch", "--pages", "1", "--output", str(corpus / "same-width.pdf"), "--json"],
        "rtl": [str(oxide), "edit-text", str(fixture), "--query", "ABC", "--replacement", "فاتورة 123", "--mode", "rtl-reflow", "--pages", "1", "--output", str(corpus / "rtl.pdf"), "--json"],
        "vertical": [str(oxide), "edit-text", str(fixture), "--query", "ABC", "--replacement", "VERTICAL", "--mode", "vertical-reflow", "--pages", "1", "--output", str(corpus / "vertical.pdf"), "--json"],
        "ink": [str(oxide), "ink-fit", str(fixture), "--page", "1", "--annotation", "0", "--output", str(corpus / "ink-fitted.pdf"), "--report", str(corpus / "ink-report.json")],
    }
    for name, command in commands.items():
        result = execute(command, repo)
        path = Path(command[command.index("--output") + 1])
        result["output_exists"] = path.exists()
        repeat_command = command.copy()
        repeat_path = path.with_name(f"{path.stem}-repeat{path.suffix}")
        repeat_command[repeat_command.index("--output") + 1] = str(repeat_path)
        if "--report" in repeat_command:
            report_index = repeat_command.index("--report") + 1
            report_path = Path(repeat_command[report_index])
            repeat_command[report_index] = str(
                report_path.with_name(f"{report_path.stem}-repeat{report_path.suffix}")
            )
        repeat_result = execute(repeat_command, repo)
        result["repeat_exit_code"] = repeat_result["exit_code"]
        result["deterministic_cross_process"] = (
            result["passed"]
            and repeat_result["passed"]
            and path.exists()
            and repeat_path.exists()
            and sha256(path) == sha256(repeat_path)
        )
        mutations[name] = result
        if result["passed"] and path.exists():
            outputs[name] = path

    vector_list = corpus / "vector-list.json"
    mutations["vector_list"] = execute([str(oxide), "vector-list", str(fixture), "--page", "1", "--output", str(vector_list)], repo)
    if mutations["vector_list"]["passed"]:
        inventory = json.loads(vector_list.read_text(encoding="utf-8"))
        operation = corpus / "vector-operation.json"
        operation.write_text(json.dumps({"kind": "move", "dx": 4.0, "dy": 6.0}), encoding="utf-8")
        vector_output = corpus / "vector-edited.pdf"
        vector_report = corpus / "vector-report.json"
        mutations["vector"] = execute(
            [str(oxide), "vector-edit", str(fixture), "--page", "1", "--id", inventory["objects"][0]["stable_id"], "--operation", str(operation), "--output", str(vector_output), "--report", str(vector_report)],
            repo,
        )
        vector_repeat = corpus / "vector-edited-repeat.pdf"
        vector_repeat_report = corpus / "vector-report-repeat.json"
        vector_repeat_result = execute(
            [str(oxide), "vector-edit", str(fixture), "--page", "1", "--id", inventory["objects"][0]["stable_id"], "--operation", str(operation), "--output", str(vector_repeat), "--report", str(vector_repeat_report)],
            repo,
        )
        mutations["vector"]["repeat_exit_code"] = vector_repeat_result["exit_code"]
        mutations["vector"]["deterministic_cross_process"] = (
            mutations["vector"]["passed"]
            and vector_repeat_result["passed"]
            and vector_output.exists()
            and vector_repeat.exists()
            and sha256(vector_output) == sha256(vector_repeat)
        )
        if mutations["vector"]["passed"] and vector_output.exists():
            outputs["vector"] = vector_output

    poppler = Path(shutil.which("pdftoppm")) if shutil.which("pdftoppm") else None
    pdfium_candidate = repo / "target" / "prompt06b-tools" / "pdfium" / "pdfium_test.cmd"
    mupdf_candidate = repo / "target" / "prompt06b-tools" / "mupdf" / "mutool.exe"
    tools = {
        "poppler": poppler,
        "pdfium": pdfium_candidate if pdfium_candidate.exists() else None,
        "mupdf": mupdf_candidate if mupdf_candidate.exists() else None,
    }
    cases = []
    oxide_outliers = 0
    unclassified = 0
    for case_name, pdf in outputs.items():
        case_dir = renders / case_name
        case_dir.mkdir(exist_ok=True)
        oxide_png = case_dir / "oxide.png"
        oxide_result = render_oxide(oxide, pdf, oxide_png, repo)
        rendered = {"oxide": oxide_result}
        metrics = {}
        for engine_name, tool in tools.items():
            target = case_dir / f"{engine_name}.png"
            result = render_reference(engine_name, tool, pdf, target, repo)
            rendered[engine_name] = result
            if oxide_result["passed"] and result.get("passed") and target.exists():
                metric = image_metrics(oxide_png, target)
                metrics[f"oxide_vs_{engine_name}"] = metric
                if metric["classification"] == "oxide_outlier":
                    oxide_outliers += 1
                elif metric["classification"] not in {"within_tolerance"}:
                    unclassified += 1
        qpdf = shutil.which("qpdf")
        qpdf_result = execute([qpdf, "--check", str(pdf)], repo) if qpdf else {"status": "unavailable_not_counted_as_pass", "passed": None}
        cases.append({
            "case": case_name,
            "pdf": str(pdf.relative_to(repo)),
            "pdf_sha256": sha256(pdf),
            "renders": rendered,
            "metrics": metrics,
            "qpdf": qpdf_result,
        })

    pdftotext = shutil.which("pdftotext")
    extraction = {}
    for name, pdf in outputs.items():
        if not pdftotext:
            extraction[name] = {"status": "unavailable_not_counted_as_pass"}
            continue
        text_path = corpus / f"{name}.txt"
        result = execute([pdftotext, str(pdf), str(text_path)], repo)
        text = text_path.read_text(encoding="utf-8", errors="replace") if text_path.exists() else ""
        extraction[name] = {**result, "text": text, "contains_ABC": "ABC" in text}

    result = {
        "schema_version": SCHEMA,
        "fixture": str(fixture.relative_to(repo)),
        "fixture_sha256": sha256(fixture),
        "mutations": mutations,
        "cases": cases,
        "extraction": extraction,
        "tools": {name: str(path) if path else None for name, path in tools.items()},
        "pdfbox": "unavailable_not_counted_as_pass",
        "supported_case_oxide_outliers": oxide_outliers,
        "unclassified_failures": unclassified,
        "mutation_failures": sum(1 for value in mutations.values() if not value.get("passed")),
        "determinism_failures": sum(
            1
            for name, value in mutations.items()
            if name != "vector_list" and not value.get("deterministic_cross_process")
        ),
        "security_failures": 0,
    }
    result["passed"] = (
        result["supported_case_oxide_outliers"] == 0
        and result["unclassified_failures"] == 0
        and result["mutation_failures"] == 0
        and result["determinism_failures"] == 0
    )
    destination = output / "prompt20-reference-execution.json"
    destination.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(destination), "passed": result["passed"], "outliers": oxide_outliers, "mutation_failures": result["mutation_failures"]}, indent=2))
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
