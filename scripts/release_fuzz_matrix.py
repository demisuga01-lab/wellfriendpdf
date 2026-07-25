#!/usr/bin/env python3
"""Build the Prompt 27 release fuzz target inventory.

The script intentionally uses only the Python standard library so the same
command works on a fresh VPS runner and in GitHub Actions. It reads the
out-of-workspace `fuzz/Cargo.toml`, classifies each target into release tiers,
and writes deterministic JSON/Markdown evidence for Prompt 27.
"""

from __future__ import annotations

import argparse
import json
import re
import time
from pathlib import Path


SCHEMA_VERSION = "prompt27.release-fuzz-target-inventory.v1"
DEFAULT_OUTPUT = Path("target/prompt27-verapdf-crypto-fuzz/release-fuzz-target-inventory.json")
PROMPT27_FUZZ_MEMORY_CAP_MIB = 16_384

PARSER_TARGETS = {
    "parse_pdf": "end-to-end parser/COS/xref/object-stream/open-bytes coverage",
    "content_tokenizer": "page content tokenizer/lexer",
    "cos_object": "COS object parser, numbers, names, strings, arrays, dictionaries",
    "parser_report": "parser diagnostics, trailer/root/catalog reporting",
    "xref_stream": "xref stream parsing",
    "object_stream": "object stream parsing",
    "document_rewrite": "incremental revision/rewrite parser interaction",
    "linearize": "linearization hint parsing/reporting",
    "structured_pdf": "malformed object graph and structure traversal",
    "decode_scanner": "repair scanner and decode-discovery parser path",
}

SUBSYSTEMS = {
    "parse_pdf": "parser/cos/xref",
    "content_tokenizer": "parser/tokenizer",
    "cos_object": "parser/cos",
    "parser_report": "parser/diagnostics",
    "xref_stream": "parser/xref-stream",
    "object_stream": "parser/object-stream",
    "document_rewrite": "writer/parser-revision-chain",
    "linearize": "parser/linearization",
    "structured_pdf": "parser/structure",
    "decode_scanner": "parser/repair-scan",
    "filters": "stream-filters/codecs",
    "predictor": "stream-filters/predictors",
    "image_decoders": "stream-filters/image-codecs",
    "fonts": "renderer/fonts",
    "font_mapping": "renderer/font-mapping",
    "cmap": "renderer/cmap",
    "functions": "renderer/functions",
    "display_list": "renderer/display-list",
    "renderer_prompt11": "renderer/prompt11-regression",
    "writer": "writer/edit",
    "editing": "writer/edit",
    "color_report": "standards/color",
    "pdfa": "standards/pdfa",
    "pdfua_structure": "standards/pdfua",
    "pdfx_prepress": "standards/pdfx",
    "cross_profile_standards": "standards/cross-profile",
    "standards_xmp_identifier": "standards/xmp",
    "crypto": "crypto/encryption",
    "signature_validation": "signatures/validation",
    "signature_evidence": "signatures/evidence",
    "timestamp_token": "signatures/timestamp",
    "signature_preserving_edit_plan": "signatures/edit-policy",
    "incremental_signing_plan": "signatures/incremental-signing",
    "cms_insertion_boundary": "signatures/cms-boundary",
    "external_signer_response": "signatures/external-signer",
    "mdp_permission_parser": "signatures/mdp",
    "post_signature_modification": "signatures/post-signature-modification",
}

RELEASE_CRITICAL = {
    "parse_pdf",
    "content_tokenizer",
    "cos_object",
    "xref_stream",
    "object_stream",
    "document_rewrite",
    "linearize",
    "structured_pdf",
    "decode_scanner",
    "filters",
    "crypto",
    "signature_validation",
    "signature_evidence",
    "timestamp_token",
    "incremental_signing_plan",
    "cms_insertion_boundary",
    "pdfa",
    "pdfua_structure",
    "pdfx_prepress",
    "cross_profile_standards",
    "standards_xmp_identifier",
}

HIGH_PRIORITY_LONG = {
    # One end-to-end high-priority parser campaign target. The specialized
    # parser targets still build and smoke-run; the long campaign target is the
    # broadest parser entry point and exercises the actual open-bytes path.
    "parse_pdf",
}

TARGET_RE = re.compile(
    r"\[\[bin\]\]\s*name\s*=\s*\"(?P<name>[^\"]+)\"\s*path\s*=\s*\"(?P<path>[^\"]+)\"",
    re.MULTILINE,
)


def parse_fuzz_targets(fuzz_toml: Path) -> list[dict[str, str]]:
    text = fuzz_toml.read_text(encoding="utf-8")
    targets = []
    for match in TARGET_RE.finditer(text):
        targets.append({"name": match.group("name"), "path": match.group("path")})
    return targets


def target_row(target: dict[str, str], repo: Path) -> dict[str, object]:
    name = target["name"]
    subsystem = SUBSYSTEMS.get(name, "unclassified")
    is_parser = name in PARSER_TARGETS
    seed_path = Path("fuzz") / "corpus" / name
    return {
        "name": name,
        "subsystem": subsystem,
        "crate_or_bin": "fuzz/" + target["path"],
        "input_cap_bytes": 262_144 if is_parser or "signature" in subsystem or "standards" in subsystem else 65_536,
        "memory_cap_mib": PROMPT27_FUZZ_MEMORY_CAP_MIB,
        "expected_run_mode": (
            "build+smoke+long_high_priority" if name in HIGH_PRIORITY_LONG else
            "build+smoke_release_critical" if name in RELEASE_CRITICAL else
            "build+smoke"
        ),
        "seed_corpus_path": seed_path.as_posix(),
        "seed_count": count_files(repo / seed_path),
        "owner": "Wellfriend PDF SDK parser/release engineering",
        "ci_inclusion": ci_tier(name),
        "release_inclusion": "release-fuzz" if name in RELEASE_CRITICAL else "inventory-only-until-prioritized",
        "current_build_status": "not_run_by_inventory",
        "current_smoke_status": "not_run_by_inventory",
        "long_campaign_status": "planned_prompt27" if name in HIGH_PRIORITY_LONG else "smoke_only_prompt27",
        "prompt27_parser_scope": PARSER_TARGETS.get(name),
    }


def ci_tier(name: str) -> str:
    if name in {"parse_pdf", "filters", "content_tokenizer", "crypto", "pdfa", "signature_validation"}:
        return "pr_smoke"
    if name in RELEASE_CRITICAL:
        return "nightly_or_manual_release"
    return "manual_release_inventory"


def count_files(path: Path) -> int:
    if not path.exists():
        return 0
    return sum(1 for item in path.rglob("*") if item.is_file())


def parser_scope_coverage(rows: list[dict[str, object]]) -> list[dict[str, object]]:
    present = {str(row["name"]) for row in rows}
    requirements = [
        ("COS object parser", "cos_object"),
        ("tokenizer/lexer", "content_tokenizer"),
        ("numeric/name/string parsing", "cos_object"),
        ("stream dictionary parsing", "parse_pdf"),
        ("xref table parsing", "parse_pdf"),
        ("xref stream parsing", "xref_stream"),
        ("object stream parsing", "object_stream"),
        ("trailer/root/catalog parsing", "parser_report"),
        ("incremental revision chain parsing", "document_rewrite"),
        ("repair scanner", "decode_scanner"),
        ("linearization hint parsing", "linearize"),
        ("hybrid-reference parsing", "parse_pdf"),
        ("encrypted-object metadata parsing without keys", "crypto"),
        ("malformed object graph traversal", "structured_pdf"),
    ]
    return [
        {
            "requirement": requirement,
            "representative_target": target,
            "status": "covered" if target in present else "missing",
        }
        for requirement, target in requirements
    ]


def write_markdown(payload: dict[str, object], path: Path) -> None:
    rows = payload["targets"]
    lines = [
        "# Prompt 27 release fuzz target inventory",
        "",
        f"Generated: `{payload['generated_at_utc']}`",
        "",
        "| target | subsystem | CI | release | long campaign |",
        "| --- | --- | --- | --- | --- |",
    ]
    for row in rows:
        lines.append(
            "| {name} | {subsystem} | {ci} | {release} | {long} |".format(
                name=row["name"],
                subsystem=row["subsystem"],
                ci=row["ci_inclusion"],
                release=row["release_inclusion"],
                long=row["long_campaign_status"],
            )
        )
    lines.extend(["", "## Parser scope coverage", ""])
    lines.extend(["| requirement | target | status |", "| --- | --- | --- |"])
    for item in payload["parser_scope_coverage"]:
        lines.append(
            f"| {item['requirement']} | {item['representative_target']} | {item['status']} |"
        )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def build_payload(repo: Path) -> dict[str, object]:
    fuzz_toml = repo / "fuzz" / "Cargo.toml"
    targets = parse_fuzz_targets(fuzz_toml)
    rows = [target_row(target, repo) for target in targets]
    unclassified = [row["name"] for row in rows if row["subsystem"] == "unclassified"]
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "repo": str(repo),
        "fuzz_manifest": str(fuzz_toml),
        "target_count": len(rows),
        "parser_target_count": sum(1 for row in rows if row["name"] in PARSER_TARGETS),
        "release_critical_count": sum(1 for row in rows if row["name"] in RELEASE_CRITICAL),
        "high_priority_long_campaign_targets": sorted(HIGH_PRIORITY_LONG),
        "unclassified_targets": unclassified,
        "parser_scope_coverage": parser_scope_coverage(rows),
        "targets": rows,
        "verdict": "complete_inventory" if not unclassified else "inventory_has_unclassified_targets",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--markdown-output", type=Path, default=None)
    args = parser.parse_args()

    repo = args.repo.resolve()
    payload = build_payload(repo)
    output = args.output if args.output.is_absolute() else repo / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.markdown_output:
        md = args.markdown_output if args.markdown_output.is_absolute() else repo / args.markdown_output
        write_markdown(payload, md)
    print(json.dumps({"output": str(output), "verdict": payload["verdict"]}, sort_keys=True))
    return 0 if payload["verdict"] == "complete_inventory" else 2


if __name__ == "__main__":
    raise SystemExit(main())
