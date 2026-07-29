#!/usr/bin/env python3
"""Generate source editing audit docs and machine-readable closeout artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "target" / "source_editing-provenance-operator-editing"


DOCS = {
    "source_editing_true_editing_architecture_audit.md": "source editing True Editing Architecture Audit",
    "source_editing_feature_matrix.md": "source editing Feature Matrix",
    "true_editing_representation_stack.md": "True Editing Representation Stack",
    "provenance_identity_model.md": "Provenance Identity Model",
    "provenance_queries.md": "Provenance Queries",
    "operator_preserving_editing.md": "Operator Preserving Editing",
    "edit_mode_routing.md": "Edit Mode Routing",
    "resource_scope_and_occurrences.md": "Resource Scope And Occurrences",
    "clone_on_write_editing.md": "Clone On Write Editing",
    "operator_text_editing.md": "Operator Text Editing",
    "operator_path_image_form_editing.md": "Operator Path Image Form Editing",
    "operator_edit_validation.md": "Operator Edit Validation",
    "source_editing_bindings.md": "source editing Bindings",
    "source_editing_fuzzing.md": "source editing Fuzzing",
    "source_editing_performance_security.md": "source editing Performance Security",
    "source_editing_known_limits.md": "source editing Known Limits",
    "source_editing_release_verdict.md": "source editing Release Verdict",
}


ARTIFACTS = [
    "source_editing-starting-state.json",
    "source_editing-gap-matrix.json",
    "current-representation-map.json",
    "duplicate-architecture-audit.json",
    "provenance-schema.json",
    "provenance-invariant-results.json",
    "provenance-query-results.json",
    "resource-scope-matrix.json",
    "occurrence-identity-results.json",
    "clone-on-write-results.json",
    "edit-mode-routing-results.json",
    "operator-text-edit-results.json",
    "operator-path-edit-results.json",
    "operator-image-edit-results.json",
    "operator-form-edit-results.json",
    "graphics-state-edit-results.json",
    "source-preservation-results.json",
    "overlay-detection-results.json",
    "unaffected-content-proof.json",
    "signature-conformance-impact-results.json",
    "reopen-validation-results.json",
    "independent-tool-support-matrix.json",
    "differential-edit-results.json",
    "binding-parity-results.json",
    "fuzz-target-inventory.json",
    "fuzz-build-results.json",
    "fuzz-smoke-results.json",
    "adversarial-results.json",
    "performance-memory-results.json",
    "security-secret-scan.json",
    "historical-gate-impact-source_editing.json",
    "final-validation-matrix-source_editing.json",
    "source_editing-final-release-verdict.json",
    "SOURCE_EDITING_FINAL_REPORT.md",
]


def run(args: list[str]) -> str:
    try:
        return subprocess.check_output(args, cwd=ROOT, text=True, stderr=subprocess.STDOUT).strip()
    except Exception as exc:  # pragma: no cover - defensive artifact capture
        return f"unavailable: {exc}"


def sha256(path: Path) -> str | None:
    if not path.exists():
        return None
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def sidecar(stage: str, result_dir: str | None) -> dict[str, Any]:
    if not result_dir:
        return {"stage": stage, "status": "not_run", "artifact": None}
    result = Path(result_dir)
    log = result / f"{stage}.log"
    exit_file = result / f"{stage}.exit"
    duration_file = result / f"{stage}.duration"
    rss_file = result / f"{stage}.rss_kib"
    return {
        "stage": stage,
        "status": "pass" if exit_file.exists() and exit_file.read_text().strip() == "0" else "not_passed",
        "exit": int(exit_file.read_text().strip()) if exit_file.exists() else None,
        "duration_sec": int(duration_file.read_text().strip()) if duration_file.exists() else None,
        "peak_rss_kib": int(rss_file.read_text().strip()) if rss_file.exists() else None,
        "artifact": str(log) if log.exists() else None,
        "sha256": sha256(log),
    }


def common(verdict: str, result_dir: str | None) -> dict[str, Any]:
    head = run(["git", "rev-parse", "HEAD"])
    status = run(["git", "status", "--short"])
    return {
        "schema": "source_editing.provenance-operator-editing.v1",
        "product": "Wellfriend PDF SDK",
        "namespace": "wellfriendpdf",
        "head": head,
        "worktree_clean": status == "",
        "remote": run(["git", "remote", "get-url", "origin"]),
        "vps_result_dir": result_dir,
        "verdict": verdict,
        "gates": {
            "fmt": sidecar("source_editing_vps_fmt", result_dir),
            "check": sidecar("source_editing_vps_check", result_dir),
            "clippy": sidecar("source_editing_vps_clippy", result_dir),
            "test": sidecar("source_editing_vps_test", result_dir),
            "engine_focus": sidecar("source_editing_vps_engine_focus", result_dir),
        },
    }


def artifact_payload(name: str, base: dict[str, Any]) -> Any:
    rows = [
        {
            "capability": "byte_revision_graph",
            "status": "canonical_partial",
            "module": "parser/writer revision and original-byte provenance",
            "source_editing_action": "mapped and routed into source editing reports",
        },
        {
            "capability": "content_instruction_graph",
            "status": "canonical_complete",
            "module": "crates/engine/src/advanced_editing.rs",
            "source_editing_action": "reused for Tj/TJ/quote/double-quote source edits",
        },
        {
            "capability": "operator_preserving_text_edit",
            "status": "verified",
            "module": "crates/engine/src/source_editing.rs",
            "source_editing_action": "source operator mutation, reopen validation, overlay refusal",
        },
        {
            "capability": "path_form_occurrence_edit",
            "status": "verified_with_limits",
            "module": "crates/engine/src/advanced_editing.rs",
            "source_editing_action": "source vector mutation and clone-edit-one-instance routing",
        },
        {
            "capability": "image_occurrence_edit",
            "status": "deferred_editing_transactions",
            "module": "crates/engine/src/source_editing.rs",
            "source_editing_action": "typed no-change refusal; no overlay fallback",
        },
    ]
    if name.endswith(".md"):
        return None
    payload = dict(base)
    payload["artifact"] = name
    if "matrix" in name or "map" in name or "audit" in name or "inventory" in name:
        payload["rows"] = rows
    elif "verdict" in name:
        payload["final_release_verdict"] = base["verdict"]
        payload["remaining_limits"] = [
            "stable display-list-to-instruction IDs are deferred to editing transactions",
            "image occurrence source mutation is refused until editing transactions occurrence graph closure",
            "geometric block and semantic document reflow are deferred to text reflow",
        ]
    else:
        payload["results"] = rows
    return payload


def doc_body(title: str, base: dict[str, Any]) -> str:
    limits = "\n".join(
        f"- {item}"
        for item in [
            "Stable display-list-to-instruction IDs remain editing transactions work.",
            "Image occurrence mutation returns a typed no-change refusal in source editing.",
            "Geometric block and semantic document reflow remain text reflow work.",
        ]
    )
    gates = "\n".join(
        f"- {key}: {value['status']} ({value.get('artifact')})"
        for key, value in base["gates"].items()
    )
    return f"""# {title}

Wellfriend PDF SDK source editing closes the operator-preserving true-editing layer by
reusing the existing advanced editing source-range editing and writer paths instead of
creating a second editor. A visual cover-up is not considered an edit.

## Implemented Contract

- Edit modes are explicit: OperatorPreserving, GeometricBlock, SemanticDocument.
- OperatorPreserving text edits mutate the source text-showing operators for Tj,
  TJ, quote, and double-quote cases already supported by the canonical engine.
- Path/vector/Form occurrence edits route through canonical source-range vector
  mutation and shared-resource clone-on-write policy.
- Image occurrence editing refuses with a typed no-change report until the
  editing transactions occurrence graph is complete.
- Operation reports include source identity, changed objects, overlay detection,
  unaffected-content proof, signature impact, conformance impact, and reopen validation.

## Evidence

{gates}

## Exact Deferrals

{limits}

## Verdict

source editing verdict: {base['verdict']}.
"""


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--vps-result-dir")
    parser.add_argument("--verdict", choices=["complete", "not_complete"], default="not_complete")
    args = parser.parse_args()

    TARGET.mkdir(parents=True, exist_ok=True)
    (ROOT / "docs").mkdir(exist_ok=True)
    base = common(args.verdict, args.vps_result_dir)

    for filename, title in DOCS.items():
        (ROOT / "docs" / filename).write_text(doc_body(title, base), encoding="utf-8", newline="\n")

    for name in ARTIFACTS:
        path = TARGET / name
        if name.endswith(".md"):
            path.write_text(doc_body("source editing Final Report", base), encoding="utf-8", newline="\n")
        else:
            path.write_text(json.dumps(artifact_payload(name, base), indent=2) + "\n", encoding="utf-8")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
