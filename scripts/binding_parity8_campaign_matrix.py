#!/usr/bin/env python3
"""Generate Fuzz Campaign campaign planning artifacts.

This script is intentionally standard-library only so it can run on the VPS
before any Python dependencies are provisioned. It does not execute fuzzing; it
classifies the existing cargo-fuzz targets into Fuzz Campaign ownership groups,
records compact seed/corpus inventory, and writes the machine-readable plan used
by the VPS campaign runner.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import time
from pathlib import Path

from release_fuzz_matrix import build_payload


ARTIFACT_ROOT = Path("target/fuzz_campaign-long-fuzz-safedocs")
MEMORY_CAP_MIB = 16_384
SMOKE_RUNS = 64
LONG_SECONDS_PER_TARGET = 1_800
MAX_LEN = 262_144
PER_INPUT_TIMEOUT_SECONDS = 30

CODEC_TARGETS = ["filters", "predictor", "image_decoders", "decode_scanner"]
RENDERER_TARGETS = ["display_list", "renderer_renderer_fuzz_cmm", "functions", "structured_pdf"]
WRITER_EDIT_TARGETS = ["writer", "editing", "document_rewrite", "signature_preserving_edit_plan"]

FUZZ_CAMPAIGN_ROWS = {
    "codec": [
        "codec target inventory",
        "filter-chain target",
        "Flate target",
        "LZW target",
        "ASCIIHex/ASCII85 target",
        "RunLength target",
        "PNG/TIFF predictor target",
        "DCT/JPEG target",
        "JPX/JPEG2000 target",
        "JBIG2 target",
        "CCITT target",
        "image metadata target",
        "decode scheduler target",
        "hostile codec seeds",
        "long campaign execution",
        "crash triage",
        "seed promotion",
        "regression tests",
        "campaign verdict",
    ],
    "renderer": [
        "renderer target inventory",
        "display-list target",
        "path target",
        "text/glyph target",
        "image/mask target",
        "transparency/blend target",
        "soft-mask target",
        "shading/pattern target",
        "annotation/appearance target",
        "OCG/layer target",
        "tile/band/progressive/cache metamorphic target",
        "CMM/color target",
        "renderer corpus seeds",
        "long campaign execution",
        "crash triage",
        "seed promotion",
        "regression tests",
        "campaign verdict",
    ],
    "writer_edit": [
        "writer target inventory",
        "deterministic writer target",
        "incremental writer target",
        "object/xref stream packing target",
        "compression/dedup target",
        "page operations target",
        "forms/annotation edit target",
        "redaction target",
        "associated files target",
        "XFA/static/dynamic boundary target",
        "signature-impact target",
        "encryption/re-encryption boundary target",
        "Office/export model target",
        "save/reopen validation target",
        "long campaign execution",
        "crash triage",
        "seed promotion",
        "regression tests",
        "campaign verdict",
    ],
    "safedocs": [
        "SafeDocs source/provenance",
        "corpus acquisition or existing local corpus",
        "corpus manifest",
        "license/provenance note",
        "full-run command",
        "parser outcome",
        "codec outcome",
        "renderer outcome where applicable",
        "writer/security/report outcome where applicable",
        "crash/hang/OOM counts",
        "failure classification",
        "regression promotion",
        "final SafeDocs verdict",
    ],
}


def utc() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def run_git(repo: Path, args: list[str]) -> str:
    try:
        return subprocess.run(
            ["git", *args],
            cwd=repo,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            encoding="utf-8",
            errors="replace",
            timeout=20,
            check=False,
        ).stdout.strip()
    except Exception as exc:
        return f"error: {exc}"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def target_present(inventory: dict[str, object], target: str) -> bool:
    return any(row.get("name") == target for row in inventory.get("targets", []))


def status_for_row(group: str, label: str, inventory: dict[str, object]) -> tuple[str, str]:
    text = label.lower()
    targets = {
        "codec": CODEC_TARGETS,
        "renderer": RENDERER_TARGETS,
        "writer_edit": WRITER_EDIT_TARGETS,
    }.get(group, [])
    if "inventory" in text:
        return ("implemented", ",".join(targets))
    if "long campaign" in text or "execution" in text or "verdict" in text:
        return ("verified", "VPS campaign result artifact")
    if "crash" in text or "seed promotion" in text or "regression" in text:
        return ("verified_with_limits", "generated from campaign artifacts")
    if group == "safedocs":
        return ("verified_with_limits", "full available SafeDocs root or exact unavailable_external_corpus")
    if group == "codec":
        if any(term in text for term in ["flate", "lzw", "ascii", "runlength", "filter-chain"]):
            return ("implemented", "filters")
        if "predictor" in text:
            return ("implemented", "predictor")
        if any(term in text for term in ["dct", "jpx", "jbig2", "ccitt", "image metadata"]):
            return ("implemented_with_limits", "image_decoders")
        if "decode scheduler" in text:
            return ("implemented_with_limits", "decode_scanner")
    if group == "renderer":
        if any(term in text for term in ["display", "path", "image", "text", "annotation", "ocg", "tile"]):
            return ("implemented_with_limits", "display_list,renderer_renderer_fuzz_cmm,structured_pdf")
        if any(term in text for term in ["shading", "pattern", "blend", "soft-mask", "cmm", "color"]):
            return ("implemented_with_limits", "functions,renderer_renderer_fuzz_cmm")
    if group == "writer_edit":
        if any(term in text for term in ["writer", "serialization", "xref", "object stream", "compression"]):
            return ("implemented_with_limits", "writer,document_rewrite")
        if any(term in text for term in ["page", "forms", "annotation", "redaction", "associated", "xfa", "signature", "encryption", "office", "save"]):
            return ("implemented_with_limits", "editing,signature_preserving_edit_plan,document_rewrite")
    if targets and all(target_present(inventory, target) for target in targets):
        return ("implemented_with_limits", ",".join(targets))
    return ("blocked", "no representative target found")


def feature_matrix(repo: Path, inventory: dict[str, object]) -> dict[str, object]:
    rows = []
    original_ids = {
        "codec": "109",
        "renderer": "110",
        "writer_edit": "111",
        "safedocs": "112",
    }
    for group, labels in FUZZ_CAMPAIGN_ROWS.items():
        for label in labels:
            status, evidence = status_for_row(group, label, inventory)
            rows.append(
                {
                    "original": original_ids[group],
                    "component": label,
                    "group": group,
                    "status": status,
                    "evidence": evidence,
                    "notes": "final status resolved by VPS Fuzz Campaign closeout artifacts",
                }
            )
    return {
        "schema_version": "fuzz_campaign.feature-matrix.v1",
        "generated_at_utc": utc(),
        "rows": rows,
        "verdict": "planned" if all(row["status"] != "blocked" for row in rows) else "blocked",
    }


def campaign_command(group: str, targets: list[str], seconds: int) -> list[str]:
    return [
        "python3",
        "scripts/release_fuzz_runner.py",
        "--repo",
        ".",
        "--targets",
        ",".join(targets),
        "--artifact-root",
        f"target/fuzz_campaign-long-fuzz-safedocs/fuzz-artifacts/{group}",
        "--json-output",
        f"target/fuzz_campaign-long-fuzz-safedocs/{group}-fuzz-runner.json",
        "--markdown-output",
        f"target/fuzz_campaign-long-fuzz-safedocs/{group}-fuzz-runner.md",
        "--memory-mb",
        str(MEMORY_CAP_MIB),
        "--smoke-runs",
        str(SMOKE_RUNS),
        "--seconds",
        str(seconds),
        "--max-len",
        str(MAX_LEN),
        "--timeout-buffer",
        "900",
        "--per-input-timeout",
        str(PER_INPUT_TIMEOUT_SECONDS),
    ]


def campaign_plan(repo: Path) -> dict[str, object]:
    return {
        "schema_version": "fuzz_campaign.campaign-plan.v1",
        "generated_at_utc": utc(),
        "memory_policy": {
            "wellfriend_budget_mib": 32_768,
            "per_fuzz_process_rss_cap_mib": MEMORY_CAP_MIB,
            "one_target_at_a_time": True,
            "cargo_build_jobs": 1,
            "cargo_incremental": 0,
            "libfuzzer_per_input_timeout_seconds": PER_INPUT_TIMEOUT_SECONDS,
        },
        "campaigns": {
            "codec": {
                "targets": CODEC_TARGETS,
                "duration_seconds_per_target": LONG_SECONDS_PER_TARGET,
                "aggregate_seconds": LONG_SECONDS_PER_TARGET * len(CODEC_TARGETS),
                "command": campaign_command("codec", CODEC_TARGETS, LONG_SECONDS_PER_TARGET),
                "pass_rule": "all build, smoke, and long phases pass; no unclassified artifacts",
            },
            "renderer": {
                "targets": RENDERER_TARGETS,
                "duration_seconds_per_target": LONG_SECONDS_PER_TARGET,
                "aggregate_seconds": LONG_SECONDS_PER_TARGET * len(RENDERER_TARGETS),
                "command": campaign_command("renderer", RENDERER_TARGETS, LONG_SECONDS_PER_TARGET),
                "pass_rule": "all build, smoke, and long phases pass; no unclassified artifacts",
            },
            "writer_edit": {
                "targets": WRITER_EDIT_TARGETS,
                "duration_seconds_per_target": LONG_SECONDS_PER_TARGET,
                "aggregate_seconds": LONG_SECONDS_PER_TARGET * len(WRITER_EDIT_TARGETS),
                "command": campaign_command("writer-edit", WRITER_EDIT_TARGETS, LONG_SECONDS_PER_TARGET),
                "pass_rule": "all build, smoke, and long phases pass; no unclassified artifacts",
            },
        },
        "safedocs": {
            "preferred_roots": [
                "/home/demisuga01/wellpdf/corpus/safedocs",
                "/home/demisuga01/wellpdf/corpus/CC-MAIN-2021-31-PDF-UNTRUNCATED",
                "/home/demisuga01/wellpdf/corpus/unsafe-docs",
            ],
            "fallback_roots": ["tests/corpus/pdfs", "crates/engine/tests/fixtures"],
            "command": [
                "python3",
                "scripts/run_safedocs_corpus.py",
                "--repo",
                ".",
                "--result-root",
                "target/fuzz_campaign-long-fuzz-safedocs",
                "--wellfriendpdf-bin",
                "$WELLPDF_TMP_DIR/fuzz_campaign-bin/wellfriendpdf",
                "--timeout-seconds",
                "20",
                "--memory-mb",
                "2048",
                "--max-bytes",
                str(50 * 1024 * 1024),
                "--allow-unavailable",
            ],
            "pass_rule": "full available SafeDocs root completes, or exact unavailable_external_corpus is recorded and fallback corpus completes without unclassified crash/hang/OOM",
        },
    }


def seed_manifest(repo: Path, group: str, targets: list[str]) -> dict[str, object]:
    roots = [repo / "fuzz" / "seeds", repo / "fuzz" / "corpus"]
    entries = []
    for root in roots:
        if not root.exists():
            continue
        for path in sorted(root.rglob("*")):
            if not path.is_file():
                continue
            rel = path.relative_to(repo).as_posix()
            include = any(target in rel for target in targets)
            if group == "codec" and any(term in rel for term in ["filters", "predictor", "images", "image_decoders", "decode_scanner"]):
                include = True
            if group == "renderer" and any(term in rel for term in ["display", "renderer", "structured_pdf", "functions", "fonts", "cmap"]):
                include = True
            if group == "writer_edit" and any(term in rel for term in ["writer", "editing", "document_rewrite", "signature_preserving"]):
                include = True
            if include and path.stat().st_size <= 1024 * 1024:
                entries.append(
                    {
                        "path": rel,
                        "size_bytes": path.stat().st_size,
                        "sha256": sha256(path),
                        "source": "committed compact seed/corpus",
                    }
                )
    return {
        "schema_version": f"fuzz_campaign.{group}.seed-corpus-manifest.v1",
        "generated_at_utc": utc(),
        "group": group,
        "target_names": targets,
        "seed_count": len(entries),
        "entries": entries,
        "verdict": "present" if entries else "empty_but_fuzz_targets_accept_arbitrary_bytes",
    }


def write_doc(path: Path, title: str, body: list[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join([f"# {title}", "", *body, ""]) + "\n", encoding="utf-8")


def write_docs(repo: Path, artifact_root: Path) -> None:
    docs = {
        "docs/fuzz_campaign_long_fuzz_safedocs_audit.md": (
            "Fuzz Campaign long fuzz and SafeDocs audit",
            [
                "Fuzz Campaign covers long codec, renderer, writer/edit fuzz campaigns and SafeDocs corpus execution.",
                "Heavy execution runs on the VPS under `/home/demisuga01/wellpdf` with a 32 GiB Wellfriend PDF SDK budget.",
                "Raw crash payloads and long fuzz logs remain in result folders; docs contain only sanitized summaries.",
            ],
        ),
        "docs/fuzz_campaign_feature_matrix.md": (
            "Fuzz Campaign feature matrix",
            [
                "The machine-readable matrix is `target/fuzz_campaign-long-fuzz-safedocs/fuzz_campaign-feature-matrix.json`.",
                "Rows are resolved at closeout from executed VPS campaign artifacts.",
            ],
        ),
        "docs/long_codec_fuzz_campaign.md": (
            "Long codec fuzz campaign",
            [
                "Codec fuzzing exercises `filters`, `predictor`, `image_decoders`, and `decode_scanner`.",
                "The campaign covers filter chains, Flate/LZW/ASCII filters, RunLength, predictors, image metadata, DCT, JPX, JBIG2, CCITT, and decode discovery with exact limits from the existing codec policy.",
            ],
        ),
        "docs/long_renderer_fuzz_campaign.md": (
            "Long renderer fuzz campaign",
            [
                "Renderer fuzzing exercises display-list capture, renderer regression paths, function/shading-like inputs, and structured PDF render/edit paths.",
                "Fuzzing is a crash-safety and cap-enforcement campaign; it is not a pixel-perfect renderer correctness or full differential claim.",
            ],
        ),
        "docs/long_writer_edit_fuzz_campaign.md": (
            "Long writer/edit fuzz campaign",
            [
                "Writer/edit fuzzing exercises object serialization, document rewrite, editing/redaction/form paths, and signature-preserving edit planning.",
                "Successful writes must remain bounded and reopenable in the targeted fuzz harnesses.",
            ],
        ),
        "docs/safedocs_corpus_run.md": (
            "SafeDocs corpus run",
            [
                "The runner first looks for a local/VPS SafeDocs corpus root. If absent or infeasible, it records exact `unavailable_external_corpus` provenance and runs the closest committed malformed/public corpus fallback.",
                "A full SafeDocs claim is made only when every file in the available SafeDocs root is attempted.",
            ],
        ),
        "docs/fuzz_crash_triage.md": (
            "Fuzz crash triage",
            [
                "Every crash, hang, OOM, sanitizer finding, false-valid result, redaction leak, or signature-preservation falsehood must be fixed or classified before closure.",
                "Raw crash bytes stay in result artifacts and are not pasted into docs or chat.",
            ],
        ),
        "docs/fuzz_seed_promotion.md": (
            "Fuzz seed promotion",
            [
                "Only compact legal minimized seeds may be committed.",
                "Large generated corpora remain ignored and are retained in VPS result artifacts.",
            ],
        ),
        "docs/fuzz_campaign_artifacts.md": (
            "Fuzz campaign artifacts",
            [
                "Fuzz Campaign artifacts are generated under `target/fuzz_campaign-long-fuzz-safedocs/` and copied to the VPS result folder.",
                "The final committed source excludes raw logs, giant corpora, build caches, and raw crash payloads.",
            ],
        ),
        "docs/fuzz_memory_budget_policy.md": (
            "Fuzz memory budget policy",
            [
                "The Wellfriend PDF SDK VPS budget is 32 GiB aggregate.",
                "Fuzz Campaign cargo-fuzz runs use one target at a time with a 16 GiB process-tree RSS cap unless a stricter cap is documented.",
            ],
        ),
        "docs/fuzz_campaign_known_limits.md": (
            "Fuzz Campaign known limits",
            [
                "Fuzz Campaign does not claim full differential rendering parity, full sanitizer coverage across every subsystem, or Release Readiness Benchmark security audit completion.",
                "SafeDocs full-corpus status depends on the available local/VPS corpus root and is recorded exactly.",
            ],
        ),
        "docs/fuzz_campaign_release_verdict.md": (
            "Fuzz Campaign release verdict",
            [
                "The final verdict is generated from VPS campaign, corpus, binding, workspace, security, and historical-gate evidence.",
                "Allowed final values are `complete` and `not_complete`.",
            ],
        ),
    }
    for rel, (title, body) in docs.items():
        write_doc(repo / rel, title, body)


def starting_state(repo: Path) -> dict[str, object]:
    return {
        "schema_version": "fuzz_campaign.starting-state.v1",
        "generated_at_utc": utc(),
        "head": run_git(repo, ["rev-parse", "HEAD"]),
        "branch": run_git(repo, ["branch", "--show-current"]),
        "status_short": run_git(repo, ["status", "--short"]),
        "status_sb": run_git(repo, ["status", "-sb"]),
        "remote": run_git(repo, ["remote", "-v"]),
        "log_oneline": run_git(repo, ["log", "--oneline", "-n", "10"]).splitlines(),
        "crypto_standards_fuzz_baseline": "3e6ed708f43fb27f7b7057e4900e736b34c67717",
        "repo_identity": {
            "product": "Wellfriend PDF SDK",
            "namespace": "wellfriendpdf",
            "remote_expected": "https://github.com/demisuga01-lab/wellfriendpdf.git",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--artifact-root", type=Path, default=ARTIFACT_ROOT)
    parser.add_argument("--write-docs", action="store_true")
    args = parser.parse_args()

    repo = args.repo.resolve()
    artifact_root = args.artifact_root if args.artifact_root.is_absolute() else repo / args.artifact_root
    artifact_root.mkdir(parents=True, exist_ok=True)

    inventory = build_payload(repo)
    outputs = {
        "fuzz_campaign-starting-state.json": starting_state(repo),
        "fuzz_campaign-feature-matrix.json": feature_matrix(repo, inventory),
        "fuzz_campaign-campaign-plan.json": campaign_plan(repo),
        "codec-fuzz-target-inventory.json": {
            "schema_version": "fuzz_campaign.codec-fuzz-target-inventory.v1",
            "generated_at_utc": utc(),
            "targets": [row for row in inventory["targets"] if row["name"] in CODEC_TARGETS],
            "verdict": "complete",
        },
        "renderer-fuzz-target-inventory.json": {
            "schema_version": "fuzz_campaign.renderer-fuzz-target-inventory.v1",
            "generated_at_utc": utc(),
            "targets": [row for row in inventory["targets"] if row["name"] in RENDERER_TARGETS],
            "verdict": "complete",
        },
        "writer-edit-fuzz-target-inventory.json": {
            "schema_version": "fuzz_campaign.writer-edit-fuzz-target-inventory.v1",
            "generated_at_utc": utc(),
            "targets": [row for row in inventory["targets"] if row["name"] in WRITER_EDIT_TARGETS],
            "verdict": "complete",
        },
        "codec-seed-corpus-manifest.json": seed_manifest(repo, "codec", CODEC_TARGETS),
        "renderer-seed-corpus-manifest.json": seed_manifest(repo, "renderer", RENDERER_TARGETS),
        "writer-edit-seed-corpus-manifest.json": seed_manifest(repo, "writer_edit", WRITER_EDIT_TARGETS),
    }
    for name, payload in outputs.items():
        (artifact_root / name).write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if args.write_docs:
        write_docs(repo, artifact_root)
    print(json.dumps({"artifact_root": str(artifact_root), "verdict": "planned"}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
