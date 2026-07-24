#!/usr/bin/env python3
"""Write Prompt 02 smoke/parity artifacts from binding test outputs."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "prompt02-binding-parity"


def read_json(path: Path) -> dict | None:
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, payload: dict) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def wasm_smoke() -> dict:
    debug_wasm = ROOT / "target" / "wasm32-unknown-unknown" / "debug" / "wellfriendpdf_wasm.wasm"
    release_wasm = ROOT / "target" / "wasm32-unknown-unknown" / "release" / "wellfriendpdf_wasm.wasm"
    built = release_wasm if release_wasm.exists() else debug_wasm if debug_wasm.exists() else None
    node_glue = OUT / "wasm-node-pkg" / "wellfriendpdf_wasm.js"
    runtime = run_wasm_node_smoke(node_glue) if node_glue.exists() else None
    return {
        "surface": "wasm",
        "status": "pass" if built and runtime and runtime["status"] == "pass" else "partial" if built else "missing",
        "command": "cargo build -p wellfriendpdf-wasm --target wasm32-unknown-unknown",
        "wasm_artifact": str(built.relative_to(ROOT)) if built else None,
        "typescript_declarations": (ROOT / "crates" / "wellfriendpdf-wasm" / "wellfriendpdf.d.ts").exists(),
        "package_json": (ROOT / "crates" / "wellfriendpdf-wasm" / "package.json").exists(),
        "browser_example": (ROOT / "crates" / "wellfriendpdf-wasm" / "examples" / "browser" / "index.html").exists(),
        "runtime_smoke": runtime or {
            "status": "not_run",
            "reason": "Node wasm-bindgen glue not generated under target/prompt02-binding-parity/wasm-node-pkg",
        },
    }


def run_wasm_node_smoke(node_glue: Path) -> dict:
    js = """
const fs = require('fs');
const wasm = require('./target/prompt02-binding-parity/wasm-node-pkg/wellfriendpdf_wasm.js');
const bytes = fs.readFileSync('crates/engine/tests/fixtures/tracemonkey.pdf');
const pdf = new wasm.WellfriendPdf(bytes);
const sec = pdf.securityReportJson();
const san = pdf.sanitize('balanced');
if (!sec.includes('schema_version')) throw new Error('security report missing schema_version');
if (san.byteLength() < 5) throw new Error('sanitize output too small');
console.log(JSON.stringify({
  pages: pdf.pageCount(),
  securityBytes: Buffer.byteLength(sec),
  sanitizeBytes: san.byteLength(),
  sanitizeReportBytes: Buffer.byteLength(san.reportJson())
}));
"""
    try:
        proc = subprocess.run(
            ["node", "-e", js],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
            timeout=60,
        )
        result = json.loads(proc.stdout.strip())
        result["status"] = "pass"
        result["node_glue"] = str(node_glue.relative_to(ROOT))
        return result
    except Exception as exc:  # pragma: no cover - diagnostic artifact path
        return {"status": "fail", "reason": str(exc)}


def compare_reports(dotnet: dict | None, java: dict | None) -> dict:
    if not dotnet or not java:
        return {
            "status": "partial",
            "reason": "dotnet-smoke.json and java-smoke.json are required for hash comparison",
            "common_reports": {},
            "mismatches": [],
        }

    dotnet_reports = dotnet.get("reports", {})
    java_reports = java.get("reports", {})
    exclusions = {
        "advanced_chunks": "Prompt 15 binding smokes intentionally use different semantic fixtures",
        "semantic_bundle": "Prompt 15 binding smokes intentionally use different semantic fixtures",
        "semantic_search": "Prompt 15 binding smokes intentionally use different semantic fixtures",
        "xfa_runtime": "runtime report contains volatile elapsed-time measurements",
    }
    common = sorted((set(dotnet_reports) & set(java_reports)) - exclusions.keys())
    compared = {}
    mismatches = []
    for name in common:
        dotnet_hash = dotnet_reports[name]["sha256"]
        java_hash = java_reports[name]["sha256"]
        same = dotnet_hash == java_hash
        compared[name] = {
            "dotnet_sha256": dotnet_hash,
            "java_sha256": java_hash,
            "match": same,
        }
        if not same:
            mismatches.append(name)

    return {
        "status": "pass" if not mismatches and common else "fail",
        "basis": "byte-identical UTF-8 JSON hashes for stable common reports from .NET and Java C ABI wrappers",
        "excluded_reports": exclusions,
        "common_reports": compared,
        "mismatches": mismatches,
        "dotnet_engine_version": dotnet.get("engine_version"),
        "java_engine_version": java.get("engine_version"),
        "abi_versions": {
            "dotnet": dotnet.get("abi_version"),
            "java": java.get("abi_version"),
        },
        "c_abi_basis": "wellfriendpdf-capi facade tests exercise the same exported functions directly",
        "wasm_basis": "wellfriendpdf-wasm methods call wellfriendpdf_engine::sdk directly; runtime hash comparison awaits regenerated wasm-bindgen glue",
    }


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    dotnet = read_json(OUT / "dotnet-smoke.json")
    java = read_json(OUT / "java-smoke.json")

    write_json(OUT / "wasm-smoke.json", wasm_smoke())
    if not dotnet:
        write_json(
            OUT / "dotnet-smoke.json",
            {
                "surface": "dotnet",
                "status": "missing",
                "reason": "Run dotnet tests with WELLFRIENDPDF_PROMPT02_ARTIFACT_DIR=target/prompt02-binding-parity",
            },
        )
    if not java:
        write_json(
            OUT / "java-smoke.json",
            {
                "surface": "java",
                "status": "missing",
                "reason": "Run Java smoke with WELLFRIENDPDF_PROMPT02_ARTIFACT_DIR=target/prompt02-binding-parity",
            },
        )
    write_json(OUT / "cross-binding-parity.json", compare_reports(dotnet, java))
    print(f"wrote Prompt 02 smoke artifacts under {OUT}")


if __name__ == "__main__":
    main()
