#!/usr/bin/env python3
"""Derive reproducible percentile/SLO summaries from RAPTOR JSONL evidence.

Percentiles use the nearest-rank definition over successful observations. A
failure row is never silently converted to latency zero or removed from the
reported population: failure stages and counts are emitted alongside every
summary. The script intentionally keeps core, process, encoding, and proof
timings separate.
"""

from __future__ import annotations

import argparse
import json
import math
import statistics
from collections import Counter
from pathlib import Path
from typing import Any, Callable


PERCENTILES = (50, 90, 95, 99)
LATENCY_SLO_MS = {"p50": 15.0, "p90": 30.0, "p95": 50.0, "p99": 75.0, "max": 200.0}
EDIT_LATENCY_SLO_MS = {
    "p50": 1000.0,
    "p90": 1500.0,
    "p95": 1750.0,
    "p99": 2000.0,
    "max": 2000.0,
}


def rows(path: Path | None) -> list[dict[str, Any]]:
    if path is None:
        return []
    output: list[dict[str, Any]] = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if not line.strip():
            continue
        value = json.loads(line)
        if not isinstance(value, dict):
            raise ValueError(f"{path}:{line_number}: expected JSON object")
        output.append(value)
    return output


def nearest_rank(values: list[float], percentile: int) -> float:
    ordered = sorted(values)
    rank = max(1, math.ceil(percentile / 100.0 * len(ordered)))
    return ordered[rank - 1]


def distribution(values: list[float], suffix: str = "ms") -> dict[str, Any]:
    if not values:
        return {"count": 0}
    result: dict[str, Any] = {
        "count": len(values),
        f"mean_{suffix}": round(statistics.fmean(values), 6),
        f"stddev_{suffix}": round(statistics.pstdev(values), 6),
    }
    median = statistics.median(values)
    result[f"median_absolute_deviation_{suffix}"] = round(
        statistics.median(abs(value - median) for value in values), 6
    )
    for percentile in PERCENTILES:
        result[f"p{percentile}_{suffix}"] = round(nearest_rank(values, percentile), 6)
    result[f"max_{suffix}"] = round(max(values), 6)
    result["p99_to_p50_ratio"] = round(
        result[f"p99_{suffix}"] / result[f"p50_{suffix}"], 6
    ) if result[f"p50_{suffix}"] else None
    return result


def summarize_fields(
    successful: list[dict[str, Any]],
    extractors: dict[str, Callable[[dict[str, Any]], float]],
) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for name, extractor in extractors.items():
        values: list[float] = []
        for row in successful:
            try:
                value = float(extractor(row))
            except (KeyError, TypeError, ValueError):
                continue
            if math.isfinite(value) and value >= 0.0:
                values.append(value)
        result[name] = distribution(values)
    return result


def failure_summary(all_rows: list[dict[str, Any]]) -> dict[str, Any]:
    failures = [row for row in all_rows if "stage" in row]
    return {
        "count": len(failures),
        "by_stage": dict(sorted(Counter(str(row.get("stage", "unknown")) for row in failures).items())),
    }


def collapse_numeric_repetitions(
    observations: list[dict[str, Any]], key: str
) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = {}
    for row in observations:
        grouped.setdefault(str(row[key]), []).append(row)
    collapsed = []
    for identity, group in sorted(grouped.items()):
        value = dict(group[-1])
        for field in set().union(*(row.keys() for row in group)):
            samples = [row.get(field) for row in group]
            if all(isinstance(sample, bool) for sample in samples):
                value[field] = all(samples)
            elif all(
                isinstance(sample, (int, float)) and not isinstance(sample, bool)
                for sample in samples
            ):
                value[field] = statistics.median(float(sample) for sample in samples)
        value[key] = identity
        value["benchmark_repetitions"] = len(group)
        collapsed.append(value)
    return collapsed


def external_parser_summary(all_rows: list[dict[str, Any]]) -> dict[str, Any]:
    observations = [row for row in all_rows if "commands" in row]
    metadata = next((row["metadata"] for row in all_rows if "metadata" in row), None)
    tools = sorted(
        {
            tool
            for row in observations
            for tool in row.get("commands", {}).keys()
        }
    )
    result: dict[str, Any] = {}
    for tool in tools:
        commands = [row.get("commands", {}).get(tool, {}) for row in observations]
        accepted = [
            command
            for command in commands
            if command.get("status") in {"accepted", "accepted_with_warnings"}
        ]
        values = [float(command["duration_ms"]) for command in accepted]
        result[tool] = {
            "command_workload": next(
                (
                    command.get("command", [])[:-1]
                    for command in commands
                    if command.get("command")
                ),
                [],
            ),
            "observations": len(commands),
            "accepted": len(accepted),
            "status_counts": dict(
                sorted(Counter(str(command.get("status", "missing")) for command in commands).items())
            ),
            "fresh_process_timing": distribution(values),
        }
    return {
        "metadata": metadata,
        "workload_warning": (
            "These tools execute different inspection contracts; timings are labeled fresh-process "
            "measurements, not semantic-equivalence claims."
        ),
        "tools": result,
    }


def slo_result(
    stats: dict[str, Any], targets: dict[str, float] = LATENCY_SLO_MS
) -> dict[str, Any]:
    if stats.get("count", 0) == 0:
        return {"status": "not_measured"}
    measured = {
        "p50": stats["p50_ms"],
        "p90": stats["p90_ms"],
        "p95": stats["p95_ms"],
        "p99": stats["p99_ms"],
        "max": stats["max_ms"],
    }
    checks = {name: measured[name] <= limit for name, limit in targets.items()}
    return {
        "status": "pass" if all(checks.values()) else "fail",
        "target_ms": targets,
        "measured_ms": measured,
        "checks": checks,
    }


def core_summary(all_rows: list[dict[str, Any]]) -> dict[str, Any]:
    raw_successful = [row for row in all_rows if "stage" not in row]
    successful = collapse_numeric_repetitions(raw_successful, "path")
    timings = summarize_fields(
        successful,
        {
            "file_read": lambda row: row["read_ms"],
            "input_sha256": lambda row: row["input_sha256_ms"],
            "file_source_open_xref": lambda row: row["file_source_open_ms"],
            "indexed_page_count": lambda row: row["indexed_page_count_ms"],
            "metadata_page_count_e2e": lambda row: row["file_source_open_ms"]
            + row["indexed_page_count_ms"],
            "requested_page_materialize": lambda row: row["requested_page_materialize_ms"],
            "requested_page_e2e": lambda row: row["file_source_open_ms"]
            + row["indexed_page_count_ms"]
            + row["requested_page_materialize_ms"],
            "full_page_tree_materialize": lambda row: row["full_page_tree_materialize_ms"],
            "full_page_tree_e2e": lambda row: row["file_source_open_ms"]
            + row["indexed_page_count_ms"]
            + row["requested_page_materialize_ms"]
            + row["full_page_tree_materialize_ms"],
            "byte_source_engine_open": lambda row: row["open_ms"],
            "page_program_cold": lambda row: row["page_program_parse_cold_ms"],
            "page_program_warm": lambda row: row["page_program_parse_warm_ms"],
            "semantic_document_cold": lambda row: row["semantic_parse_cold_ms"],
            "semantic_document_warm": lambda row: row["semantic_parse_warm_ms"],
            "semantic_document_cold_with_open": lambda row: row["semantic_session_open_ms"]
            + row["semantic_parse_cold_ms"],
            "raster_cold": lambda row: row["raster_cold_ms"],
            "raster_warm": lambda row: row["raster_warm_ms"],
            "png_encode": lambda row: row["png_encode_ms"],
            "render_cold_with_open_and_encode": lambda row: row["render_session_open_ms"]
            + row["raster_cold_ms"]
            + row["png_encode_ms"],
        },
    )
    exactness = {
        "page_program_exact": sum(row.get("page_program_output_exact_match") is True for row in successful),
        "semantic_output_exact": sum(row.get("semantic_output_exact_match") is True for row in successful),
        "raster_exact": sum(row.get("raster_exact_match") is True for row in successful),
        "documents": len(successful),
        "raw_observations": len(raw_successful),
    }
    return {
        "observations": len(successful),
        "failures": failure_summary(all_rows),
        "timings": timings,
        "exactness": exactness,
        "requested_metadata_page_count_slo": slo_result(timings["metadata_page_count_e2e"]),
        "requested_first_page_slo": slo_result(timings["requested_page_e2e"]),
    }


def edit_summary(all_rows: list[dict[str, Any]]) -> dict[str, Any]:
    successful = [row for row in all_rows if "stage" not in row]
    timings = summarize_fields(
        successful,
        {
            "target_discovery": lambda row: row["target_discovery_ms"],
            "plan": lambda row: row["plan_ms"],
            "approval": lambda row: row["approval_ms"],
            "apply": lambda row: row["apply_ms"],
            "reopen_verify": lambda row: row["reopen_verify_ms"],
            "verified_end_to_end": lambda row: row["verified_end_to_end_ms"],
        },
    )
    verification = {
        "replacement_observed_after_reopen": sum(
            row.get("replacement_observed_after_reopen") is True for row in successful
        ),
        "prepared_plan_cache_hits": sum(row.get("prepared_plan_cache_hit") is True for row in successful),
        "prepared_engine_reused": sum(
            row.get("prepared_engine_reused") is True for row in successful
        ),
        "prepared_text_transaction_reused": sum(
            row.get("prepared_text_transaction_reused") is True for row in successful
        ),
        "observations": len(successful),
    }
    return {
        "observations": len(successful),
        "failures": failure_summary(all_rows),
        "timings": timings,
        "verification": verification,
        "requested_apply_slo": slo_result(timings["apply"], EDIT_LATENCY_SLO_MS),
        "requested_verified_e2e_slo": slo_result(
            timings["verified_end_to_end"], EDIT_LATENCY_SLO_MS
        ),
    }


def visual_summary(all_rows: list[dict[str, Any]]) -> dict[str, Any]:
    successful = [row for row in all_rows if row.get("status") == "pass"]
    tools = ("wellfriendpdf", "pdfium", "mupdf", "poppler")
    timing_extractors = {
        tool: (lambda row, name=tool: row["commands"][name]["duration_ms"]) for tool in tools
    }
    process_timings = summarize_fields(successful, timing_extractors)
    quality: dict[str, Any] = {}
    metric_fields = {
        "changed_gt_8_percent": ("changed_pixel_threshold8_percentage", "percent"),
        "mean_absolute_channel_delta": ("mean_absolute_channel_delta", "channel_delta"),
        "root_mean_squared_channel_delta": (
            "root_mean_squared_channel_delta",
            "channel_delta",
        ),
        "psnr_db": ("psnr_db", "db"),
    }
    pairs = sorted(
        {
            pair
            for row in successful
            for pair in row.get("pairwise_comparisons", {}).keys()
        }
    )
    for pair in pairs:
        for label, (field, suffix) in metric_fields.items():
            values = []
            for row in successful:
                metric = row.get("pairwise_comparisons", {}).get(pair, {})
                value = metric.get(field)
                if value is not None:
                    values.append(float(value))
            quality[f"{pair}_{label}"] = distribution(values, suffix)
    speed_checks: dict[str, Any] = {}
    for statistic in ("p50_ms", "p90_ms", "p95_ms", "p99_ms", "max_ms"):
        competitors = {
            tool: float(process_timings[tool][statistic])
            for tool in ("pdfium", "mupdf", "poppler")
            if statistic in process_timings.get(tool, {})
        }
        if not competitors or statistic not in process_timings.get("wellfriendpdf", {}):
            speed_checks[statistic] = {"status": "not_measured"}
            continue
        fastest_tool = min(competitors, key=competitors.get)
        fastest_ms = competitors[fastest_tool]
        wellfriend_ms = float(process_timings["wellfriendpdf"][statistic])
        required_ms = fastest_ms * 0.90
        speed_checks[statistic] = {
            "status": "pass" if wellfriend_ms <= required_ms else "fail",
            "wellfriendpdf_ms": wellfriend_ms,
            "fastest_competitor": fastest_tool,
            "fastest_competitor_ms": fastest_ms,
            "required_for_ten_percent_lead_ms": round(required_ms, 6),
            "observed_ratio": round(wellfriend_ms / fastest_ms, 6) if fastest_ms else None,
        }
    return {
        "observations": len(successful),
        "failures": {
            "count": len(all_rows) - len(successful),
            "statuses": dict(sorted(Counter(str(row.get("status", "missing")) for row in all_rows if row.get("status") != "pass").items())),
        },
        "process_render_timings": process_timings,
        "ten_percent_faster_than_fastest_competitor": {
            "status": (
                "pass"
                if speed_checks
                and all(check.get("status") == "pass" for check in speed_checks.values())
                else "fail"
            ),
            "checks": speed_checks,
        },
        "quality": quality,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", type=Path, required=True)
    parser.add_argument("--edit", type=Path)
    parser.add_argument("--visual", type=Path)
    parser.add_argument("--external-parse", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    core_rows = rows(args.core)
    edit_rows = rows(args.edit)
    visual_rows = rows(args.visual)
    external_rows = rows(args.external_parse)
    result = {
        "schema_version": 2,
        "kind": "raptor_stage_separated_benchmark_summary",
        "percentile_definition": "nearest_rank",
        "latency_units": "milliseconds",
        "core": core_summary(core_rows),
        "editing": edit_summary(edit_rows) if args.edit else None,
        "visual_rendering": visual_summary(visual_rows) if args.visual else None,
        "external_parser_tools": external_parser_summary(external_rows) if args.external_parse else None,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
