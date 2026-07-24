#!/usr/bin/env python3
"""Prompt 05 hostile codec corpus generator and runner.

The committed artifact is this deterministic generator, not opaque hostile PDF
blobs. Generated fixtures live under target/prompt05-codec-closeout and are safe
to delete/recreate.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
import zlib
from dataclasses import dataclass
from pathlib import Path
from typing import Any


OUT_DIR = Path("target/prompt05-codec-closeout")
CORPUS_DIR = OUT_DIR / "hostile-corpus"
RAW_DIR = CORPUS_DIR / "raw"
PDF_DIR = CORPUS_DIR / "pdf"
MANIFEST = OUT_DIR / "hostile-corpus-manifest.json"
RUN_REPORT = OUT_DIR / "hostile-corpus-run.json"


@dataclass(frozen=True)
class Fixture:
    id: str
    category: str
    trigger_type: str
    expected_result: str
    validity: str
    payload: bytes
    stream_dict: str
    max_expected_memory_bytes: int = 64 * 1024 * 1024
    max_expected_time_ms: int = 15_000
    worker_isolation_expected: bool = False
    regression_seed: bool = False
    length_override: str | None = None


def z(data: bytes, level: int = 9) -> bytes:
    return zlib.compress(data, level)


def fixtures() -> list[Fixture]:
    bomb = z(b"A" * (2 * 1024 * 1024))
    tiny = z(b"\x00" * 64)
    return [
        Fixture("flate_bomb", "flate_bombs", "decompression_bomb", "resource_limit", "bomb_like", bomb, "/Filter /FlateDecode", 2 * 1024 * 1024),
        Fixture("predictor_bomb", "predictor_bombs", "oversized_predictor_row", "structured_decode_error", "malformed", z(b"\x00" * 128), "/Filter /FlateDecode /DecodeParms << /Predictor 15 /Columns 1048577 /Colors 4 /BitsPerComponent 8 >>"),
        Fixture("truncated_flate", "truncated_flate_streams", "truncated_zlib_payload", "structured_decode_error", "malformed", z(b"truncated flate stream")[:-4], "/Filter /FlateDecode"),
        Fixture("invalid_png_predictor", "invalid_png_predictors", "bad_png_predictor_rows", "structured_decode_error", "malformed", z(b"\x04bad"), "/Filter /FlateDecode /DecodeParms << /Predictor 15 /Columns 64 /Colors 3 /BitsPerComponent 8 >>"),
        Fixture("huge_dimensions", "huge_dimensions", "declared_image_dimension_bomb", "resource_limit_or_metadata_only", "bomb_like", tiny, "/Subtype /Image /Width 1000000 /Height 1000000 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /FlateDecode"),
        Fixture("dct_malformed_segments", "dct_malformed_segments", "bad_jpeg_marker_segment", "stopped_at_image_filter", "malformed", b"\xff\xd8\xff\xdb\x00\x00bad", "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /DCTDecode"),
        Fixture("dct_truncated_scans", "dct_truncated_scans", "missing_jpeg_eoi", "stopped_at_image_filter", "malformed", b"\xff\xd8\xff\xda\x00\x0c\x03\x01\x00\x02\x11\x03\x11\x00", "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /DCTDecode"),
        Fixture("jpx_malformed_boxes", "jpx_malformed_boxes", "bad_jp2_box_layout", "stopped_at_image_filter", "malformed", b"\x00\x00\x00\x0cjP  \r\n\x87\nBAD", "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /JPXDecode"),
        Fixture("jpx_excessive_components", "jpx_excessive_component_counts", "declared_component_stress", "stopped_at_image_filter", "malformed", b"jpx-components-65535", "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Filter /JPXDecode"),
        Fixture("jbig2_symbol_dictionary_stress", "jbig2_symbol_dictionary_stress", "symbol_dictionary_stress", "stopped_at_image_filter", "malformed", b"\x97JB2\r\n\x1a\nsymbol-dictionary-stress", "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /JBIG2Decode"),
        Fixture("jbig2_segment_order_corruption", "jbig2_segment_order_corruption", "segment_order_corruption", "stopped_at_image_filter", "malformed", b"\x97JB2\r\n\x1a\nbad-segment-order", "/Subtype /Image /Width 1 /Height 1 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /JBIG2Decode"),
        Fixture("ccitt_malformed_runs", "ccitt_malformed_runs", "bad_ccitt_run_lengths", "stopped_at_image_filter", "malformed", b"\xff" * 32, "/Subtype /Image /Width 1728 /Height 1 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /CCITTFaxDecode /DecodeParms << /Columns 1728 /Rows 1 /K 0 >>"),
        Fixture("ccitt_impossible_dimensions", "ccitt_impossible_dimensions", "impossible_ccitt_rows_columns", "stopped_at_image_filter", "malformed", b"\x00" * 8, "/Subtype /Image /Width 999999999 /Height 999999999 /BitsPerComponent 1 /ColorSpace /DeviceGray /Filter /CCITTFaxDecode /DecodeParms << /Columns 999999999 /Rows 999999999 /K 0 >>"),
        Fixture("filter_chain_loops", "filter_chain_loops", "excessive_filter_chain_depth", "resource_limit", "malformed", z(b"loop"), "/Filter [ /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode /FlateDecode ]"),
        Fixture("unknown_filters", "unknown_filters", "unknown_filter_name", "unsupported_filter", "malformed", b"unknown-filter", "/Filter /ExplodeDecode"),
        Fixture("wrong_decodeparms", "wrong_decodeparms", "invalid_decode_params", "structured_decode_error", "malformed", z(b"wrong params"), "/Filter /LZWDecode /DecodeParms << /EarlyChange 9 >>"),
        Fixture("negative_huge_length", "negative_or_huge_length", "huge_length_claim", "scheduler_admission_denial", "malformed", z(b"huge length"), "/Filter /FlateDecode", length_override="999999999"),
        Fixture("stream_endstream_mismatch", "stream_endstream_mismatch", "embedded_endstream_marker", "structured_decode_error", "malformed", b"abc\nendstream\ntrailer", "/Filter /ASCIIHexDecode"),
        Fixture("object_stream_edge", "object_stream_compression_edge_cases", "bad_objstm_first_n", "structured_decode_error", "malformed", z(b"1 0 2 10 broken"), "/Type /ObjStm /N 3 /First 1000 /Filter /FlateDecode"),
        Fixture("inline_image_eod_ambiguity", "inline_image_eod_ambiguity", "ambiguous_ei_marker", "safe_success_or_warning", "malformed", b"q BI /W 1 /H 1 /CS /RGB /BPC 8 ID abc EI EI Q", ""),
        Fixture("malformed_image_masks", "malformed_image_masks", "invalid_image_mask_shape", "resource_limit_or_metadata_only", "malformed", tiny, "/Subtype /Image /ImageMask true /Width 0 /Height 1000000 /BitsPerComponent 1 /Filter /FlateDecode"),
        Fixture("malformed_icc_profiles", "malformed_icc_profiles", "bad_icc_profile_stream", "structured_decode_error", "malformed", z(b"not an icc profile"), "/N 3 /Alternate /DeviceRGB /Filter /FlateDecode"),
        Fixture("embedded_file_bomb", "embedded_file_decompression_bombs", "embedded_file_flate_bomb", "resource_limit", "bomb_like", bomb, "/Type /EmbeddedFile /Filter /FlateDecode", 2 * 1024 * 1024),
        Fixture("metadata_stream_bomb", "metadata_stream_bombs", "metadata_flate_bomb", "resource_limit", "bomb_like", bomb, "/Type /Metadata /Subtype /XML /Filter /FlateDecode", 2 * 1024 * 1024),
        Fixture("incremental_revision_trap", "incremental_revision_stream_traps", "revision_filtered_stream_trap", "structured_decode_error", "malformed", z(b"not ascii after flate"), "/Filter [ /FlateDecode /ASCIIHexDecode ]"),
    ]


def stream_object(payload: bytes, stream_dict: str, length_override: str | None = None) -> bytes:
    length_value = length_override if length_override is not None else str(len(payload))
    prefix = f"<< /Length {length_value}"
    if stream_dict:
        prefix += f" {stream_dict}"
    prefix += " >>\nstream\n"
    return prefix.encode("ascii") + payload + b"\nendstream"


def pdf_bytes(hostile_stream: bytes, fixture_id: str) -> bytes:
    empty_content = stream_object(b"", "")
    objects = [
        (1, b"<< /Type /Catalog /Pages 2 0 R >>"),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Contents 4 0 R /Resources << >> >>"),
        (4, empty_content),
        (5, hostile_stream),
        (6, f"<< /Producer (Wellfriend Prompt05) /Fixture ({fixture_id}) >>".encode("ascii")),
    ]
    out = bytearray(b"%PDF-1.7\n% Wellfriend Prompt05 generated fixture\n")
    max_obj = max(number for number, _ in objects)
    offsets = [0] * (max_obj + 1)
    for number, body in objects:
        offsets[number] = len(out)
        out.extend(f"{number} 0 obj\n".encode("ascii"))
        out.extend(body)
        out.extend(b"\nendobj\n")
    xref = len(out)
    out.extend(f"xref\n0 {max_obj + 1}\n".encode("ascii"))
    out.extend(b"0000000000 65535 f \n")
    for number in range(1, max_obj + 1):
        out.extend(f"{offsets[number]:010d} 00000 n \n".encode("ascii"))
    out.extend(
        f"trailer\n<< /Size {max_obj + 1} /Root 1 0 R /Info 6 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode(
            "ascii"
        )
    )
    return bytes(out)


def ensure_dirs() -> None:
    RAW_DIR.mkdir(parents=True, exist_ok=True)
    PDF_DIR.mkdir(parents=True, exist_ok=True)


def generate() -> dict[str, Any]:
    ensure_dirs()
    entries = []
    for fixture in fixtures():
        raw_path = RAW_DIR / f"{fixture.id}.bin"
        pdf_path = PDF_DIR / f"{fixture.id}.pdf"
        raw_path.write_bytes(fixture.payload)
        hostile = stream_object(fixture.payload, fixture.stream_dict, fixture.length_override)
        pdf_path.write_bytes(pdf_bytes(hostile, fixture.id))
        entries.append(
            {
                "id": fixture.id,
                "category": fixture.category,
                "trigger_type": fixture.trigger_type,
                "expected_result": fixture.expected_result,
                "validity": fixture.validity,
                "max_expected_memory_bytes": fixture.max_expected_memory_bytes,
                "max_expected_time_ms": fixture.max_expected_time_ms,
                "worker_isolation_expected": fixture.worker_isolation_expected,
                "regression_seed": fixture.regression_seed,
                "generator_command": "python scripts/prompt05_hostile_codec_corpus.py generate",
                "raw_path": str(raw_path.as_posix()),
                "pdf_path": str(pdf_path.as_posix()),
                "stream_length_bytes": len(fixture.payload),
            }
        )
    manifest = {
        "schema_version": 1,
        "prompt": "combined_prompt05",
        "fixture_count": len(entries),
        "corpus_dir": str(CORPUS_DIR.as_posix()),
        "entries": entries,
    }
    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    MANIFEST.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return manifest


def parser_report_command(pdf: Path, wellfriendpdf_bin: str | None) -> list[str]:
    args = [
        "parser-report",
        str(pdf),
        "--json",
        "--include-decode",
        "--decode-profile",
        "low-memory",
        "--decode-max-stream-mb",
        "1",
        "--decode-scheduler-mb",
        "1",
        "--max-diagnostics",
        "200",
    ]
    if wellfriendpdf_bin:
        return [wellfriendpdf_bin, *args]
    return ["cargo", "run", "-p", "wellfriendpdf-cli", "--quiet", "--", *args]


def run_one(entry: dict[str, Any], wellfriendpdf_bin: str | None, timeout_sec: int) -> dict[str, Any]:
    pdf = Path(entry["pdf_path"])
    cmd = parser_report_command(pdf, wellfriendpdf_bin)
    started = time.perf_counter()
    try:
        proc = subprocess.run(
            cmd,
            text=True,
            capture_output=True,
            timeout=timeout_sec,
            check=False,
        )
        elapsed_ms = int((time.perf_counter() - started) * 1000)
    except subprocess.TimeoutExpired as exc:
        return {
            "id": entry["id"],
            "category": entry["category"],
            "status": "fail",
            "classification": "timeout",
            "elapsed_ms": timeout_sec * 1000,
            "command": cmd,
            "stdout_tail": (exc.stdout or "")[-1000:],
            "stderr_tail": (exc.stderr or "")[-1000:],
        }

    parsed: dict[str, Any] | None = None
    parse_error = None
    if proc.stdout.strip():
        try:
            parsed = json.loads(proc.stdout)
        except json.JSONDecodeError as exc:
            parse_error = str(exc)
    decode = parsed.get("decode") if isinstance(parsed, dict) else None
    metrics = decode.get("metrics", {}) if isinstance(decode, dict) else {}
    diagnostics = decode.get("diagnostics", []) if isinstance(decode, dict) else []
    structured = proc.returncode == 0 and isinstance(decode, dict)
    fail_closed = bool(
        structured
        and (
            decode.get("ok") is False
            or diagnostics
            or metrics.get("scheduler_budget_denials", 0)
            or metrics.get("cap_hits_by_limit", {})
            or entry["expected_result"].startswith("stopped_at_image_filter")
            or entry["expected_result"].startswith("safe_success")
        )
    )
    return {
        "id": entry["id"],
        "category": entry["category"],
        "status": "pass" if structured else "fail",
        "classification": "structured_decode_report" if structured else "runner_error",
        "expected_result": entry["expected_result"],
        "engine_exit_code": proc.returncode,
        "elapsed_ms": elapsed_ms,
        "fail_closed": fail_closed,
        "decode_ok": decode.get("ok") if isinstance(decode, dict) else None,
        "diagnostic_count": len(diagnostics),
        "metrics": metrics,
        "command": cmd,
        "json_parse_error": parse_error,
        "stderr_tail": proc.stderr[-1000:],
    }


def run(manifest_path: Path, wellfriendpdf_bin: str | None, timeout_sec: int) -> dict[str, Any]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    results = [run_one(entry, wellfriendpdf_bin, timeout_sec) for entry in manifest["entries"]]
    passed = sum(1 for result in results if result["status"] == "pass")
    report = {
        "schema_version": 1,
        "prompt": "combined_prompt05",
        "fixture_count": len(results),
        "passed": passed,
        "failed": len(results) - passed,
        "pass_rate": passed / len(results) if results else 0.0,
        "timeout_sec": timeout_sec,
        "wellfriendpdf_bin": wellfriendpdf_bin,
        "results": results,
    }
    RUN_REPORT.parent.mkdir(parents=True, exist_ok=True)
    RUN_REPORT.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("generate")
    run_parser = sub.add_parser("run")
    run_parser.add_argument("--manifest", type=Path, default=MANIFEST)
    run_parser.add_argument("--wellfriendpdf-bin")
    run_parser.add_argument("--timeout-sec", type=int, default=15)
    all_parser = sub.add_parser("all")
    all_parser.add_argument("--wellfriendpdf-bin")
    all_parser.add_argument("--timeout-sec", type=int, default=15)
    args = parser.parse_args()

    if args.command == "generate":
        manifest = generate()
        print(json.dumps({"manifest": str(MANIFEST), "fixtures": manifest["fixture_count"]}))
        return 0
    if args.command == "run":
        report = run(args.manifest, args.wellfriendpdf_bin, args.timeout_sec)
        print(json.dumps({"run_report": str(RUN_REPORT), "passed": report["passed"], "failed": report["failed"]}))
        return 0 if report["failed"] == 0 else 1
    if args.command == "all":
        generate()
        report = run(MANIFEST, args.wellfriendpdf_bin, args.timeout_sec)
        print(json.dumps({"manifest": str(MANIFEST), "run_report": str(RUN_REPORT), "passed": report["passed"], "failed": report["failed"]}))
        return 0 if report["failed"] == 0 else 1
    return 2


if __name__ == "__main__":
    sys.exit(main())
