#!/usr/bin/env python3
"""Generate the Combined Prompt 02 binding parity matrix.

The Prompt 01 matrix remains the source for Rust/Python/C ABI/CLI feature
coverage. This generator adds Prompt 02 surfaces (WASM, .NET, Java, docs, and
packaging) plus diagnostics/package rows that are specific to the new binding
work.
"""

from __future__ import annotations

import json
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROMPT01_JSON = ROOT / "target" / "prompt01-binding-core" / "binding-gap-matrix.json"
OUT_DIR = ROOT / "target" / "prompt02-binding-parity"
JSON_OUT = OUT_DIR / "binding-gap-matrix.json"
MD_OUT = ROOT / "docs" / "bindings_prompt02_gap_matrix.md"

PUB = "implemented_public"
PART = "partial_public"
INT = "implemented_internal"
CLI = "cli_only"
UNSUP = "unsupported_reported"
MISS = "missing"
DEF = "deferred"
BLK = "blocked"

VOCAB = [PUB, PART, INT, CLI, UNSUP, MISS, DEF, BLK]
SURFACES = ["rust", "python", "c_abi", "wasm", "dotnet", "java", "cli", "docs", "packaging"]

WASM_PUBLIC = {
    "open.options",
    "parser.trailer_id",
    "parser.revisions",
    "parser.page_tree",
    "parser.repair",
    "parser.linearization",
    "parser.encryption_status",
    "parser.malformed_recovery",
    "parser.arlington",
    "decode.flate_predictor",
    "decode.dct",
    "decode.jpx",
    "decode.jbig2",
    "decode.ccitt",
    "decode.bomb",
    "decode.unsupported_filter",
    "render.raster",
    "render.dpi_scale",
    "render.image_output_encoding",
    "fonts.inventory",
    "fonts.embedding_status",
    "text.spans",
    "text.char_provenance",
    "text.word_line_grouping",
    "text.quad_bbox",
    "color.icc_inventory",
    "color.output_intent",
    "color.device_cmyk",
    "color.devicen_sep",
    "color.spot",
    "color.overprint",
    "color.rendering_intent",
    "color.prepress_warning",
    "color.conversion_diag",
    "color.pdfx",
    "sem.structtree",
    "sem.reading_order",
    "sem.paragraphs",
    "sem.rag_chunk",
    "sem.json_model",
    "forms.acroform_inventory",
    "annot.inventory",
    "annot.appearance_status",
    "page.ops",
    "page.boxes",
    "forms.xfa",
    "annot.rich_media",
    "redact.plan",
    "redact.apply",
    "redact.text_proof",
    "sanitize.policy",
    "sanitize.js",
    "sanitize.launch",
    "sanitize.uri",
    "sanitize.metadata",
    "sanitize.safe_output_proof",
    "sanitize.rescan",
    "security.risk_class",
    "sec.report",
    "sec.permissions",
    "sig.byterange",
    "sig.preservation_warning",
    "std.pdfa",
    "std.pdfua",
    "std.pdfx",
    "std.canonicalize",
    "std.threat_model",
    "diag.schema",
    "diag.error_taxonomy",
    "diag.warning_severity",
    "diag.report_versioning",
    "diag.feature_availability",
    "doc.api",
    "doc.examples",
    "pkg.metadata",
    "pkg.platform_matrix",
    "pkg.ci_smoke",
    "pkg.release_manifest",
}

NATIVE_PUBLIC = {
    "open.byte_source",
    "open.options",
    "parser.trailer_id",
    "parser.revisions",
    "parser.page_tree",
    "parser.repair",
    "parser.linearization",
    "parser.encryption_status",
    "parser.malformed_recovery",
    "parser.arlington",
    "color.icc_inventory",
    "color.output_intent",
    "color.device_cmyk",
    "color.devicen_sep",
    "color.spot",
    "color.overprint",
    "color.rendering_intent",
    "color.prepress_warning",
    "color.conversion_diag",
    "color.pdfx",
    "sem.rag_chunk",
    "sem.json_model",
    "forms.acroform_inventory",
    "annot.inventory",
    "annot.appearance_status",
    "page.ops",
    "page.boxes",
    "forms.xfa",
    "annot.rich_media",
    "redact.plan",
    "redact.apply",
    "redact.text_proof",
    "sanitize.policy",
    "sanitize.js",
    "sanitize.launch",
    "sanitize.uri",
    "sanitize.metadata",
    "sanitize.safe_output_proof",
    "sanitize.rescan",
    "security.risk_class",
    "conv.docx_faithful",
    "conv.docx_flow",
    "conv.pptx",
    "conv.xlsx",
    "conv.office_to_pdf",
    "writer.full_rewrite",
    "writer.deterministic",
    "sec.report",
    "sec.permissions",
    "std.pdfa",
    "std.pdfua",
    "std.pdfx",
    "std.canonicalize",
    "std.threat_model",
    "diag.schema",
    "diag.error_taxonomy",
    "diag.warning_severity",
    "diag.report_versioning",
    "diag.feature_availability",
    "test.capi",
    "test.cross_lang_golden",
    "doc.api",
    "doc.examples",
    "pkg.metadata",
    "pkg.platform_matrix",
    "pkg.ci_smoke",
    "pkg.abi_compat",
    "pkg.release_manifest",
}

UNSUPPORTED_IDS = {
    "diag.progress",
    "diag.cancellation",
    "decode.cancellation",
    "render.cancellation",
}

EXTRA_ROWS = [
    {
        "id": "wasm.input.bytes",
        "category": "prompt02-wasm",
        "feature": "WASM open from bytes and lifecycle",
        "surfaces": {"wasm": PUB, "dotnet": DEF, "java": DEF, "docs": PUB, "packaging": PUB},
        "note": "WellfriendPdf constructor, openWithPassword, close/isClosed, and use-after-close guard.",
    },
    {
        "id": "wasm.input.path",
        "category": "prompt02-wasm",
        "feature": "WASM open from host file path",
        "surfaces": {"wasm": UNSUP, "dotnet": DEF, "java": DEF, "docs": PUB, "packaging": PUB},
        "note": "Browser/WebWorker cannot read host paths; callers pass bytes from File/API/Node fs.",
    },
    {
        "id": "wasm.typescript",
        "category": "prompt02-wasm",
        "feature": "TypeScript declarations",
        "surfaces": {"wasm": PUB, "dotnet": DEF, "java": DEF, "docs": PUB, "packaging": PUB},
        "note": "crates/wellfriendpdf-wasm/wellfriendpdf.d.ts declares reports and output ownership.",
    },
    {
        "id": "wasm.package",
        "category": "prompt02-wasm",
        "feature": "WASM package metadata",
        "surfaces": {"wasm": PART, "dotnet": DEF, "java": DEF, "docs": PUB, "packaging": PART},
        "note": "package.json and docs added; wasm-pack is required to regenerate publishable pkg glue.",
    },
    {
        "id": "dotnet.native.loading",
        "category": "prompt02-dotnet",
        "feature": ".NET native binary loading",
        "surfaces": {"wasm": DEF, "dotnet": PUB, "java": DEF, "docs": PUB, "packaging": PUB},
        "note": "Resolver checks WELLFRIENDPDF_NATIVE_LIBRARY and RID runtime/native locations.",
    },
    {
        "id": "dotnet.nuget",
        "category": "prompt02-dotnet",
        "feature": ".NET NuGet metadata and pack smoke",
        "surfaces": {"wasm": DEF, "dotnet": PUB, "java": DEF, "docs": PUB, "packaging": PUB},
        "note": "WellfriendPdf.csproj includes package metadata/readme/license/tags.",
    },
    {
        "id": "dotnet.binary.output",
        "category": "prompt02-dotnet",
        "feature": ".NET output buffer ownership",
        "surfaces": {"wasm": DEF, "dotnet": PUB, "java": DEF, "docs": PUB, "packaging": PUB},
        "note": "WellfriendBinaryResult copies bytes to managed memory and frees native buffers.",
    },
    {
        "id": "java.native.loading",
        "category": "prompt02-java",
        "feature": "Java native binary loading",
        "surfaces": {"wasm": DEF, "dotnet": DEF, "java": PUB, "docs": PUB, "packaging": PUB},
        "note": "FFM loader checks WELLFRIENDPDF_NATIVE_LIBRARY and RID runtime/native locations.",
    },
    {
        "id": "java.maven",
        "category": "prompt02-java",
        "feature": "Java Maven package metadata",
        "surfaces": {"wasm": DEF, "dotnet": DEF, "java": PUB, "docs": PUB, "packaging": PUB},
        "note": "pom.xml records Maven metadata and binds WellfriendPdfSmokeTest into mvn test; Prompt 02B package smoke runs mvn test/package.",
    },
    {
        "id": "java.binary.output",
        "category": "prompt02-java",
        "feature": "Java output buffer ownership",
        "surfaces": {"wasm": DEF, "dotnet": DEF, "java": PUB, "docs": PUB, "packaging": PUB},
        "note": "BinaryResult copies native buffers into byte[] before freeing them.",
    },
    {
        "id": "diag.cross_binding_envelope",
        "category": "prompt02-diagnostics",
        "feature": "cross-binding JSON envelope parity",
        "surfaces": {"wasm": PUB, "dotnet": PUB, "java": PUB, "docs": PUB, "packaging": PUB},
        "note": "All Prompt 02 wrappers call shared facade or C ABI report functions.",
    },
    {
        "id": "diag.progress_posture",
        "category": "prompt02-diagnostics",
        "feature": "progress callback posture",
        "surfaces": {"wasm": UNSUP, "dotnet": UNSUP, "java": UNSUP, "docs": PUB, "packaging": PUB},
        "note": "No progress callbacks are exposed until engine calls can observe them.",
    },
    {
        "id": "diag.cancel_posture",
        "category": "prompt02-diagnostics",
        "feature": "cancellation token posture",
        "surfaces": {"wasm": UNSUP, "dotnet": UNSUP, "java": UNSUP, "docs": PUB, "packaging": PUB},
        "note": "Engine render internals can observe CancelToken, but Prompt 02 WASM/.NET/Java report/output bindings expose no cancellable render or token-aware facade operation.",
    },
    {
        "id": "prompt02b.cabi.password_open",
        "category": "prompt02b-closure",
        "feature": "C ABI open with optional password",
        "surfaces": {"c_abi": PUB, "dotnet": PUB, "java": PUB, "docs": PUB, "packaging": PUB},
        "note": "wellfriendpdf_document_open_from_bytes_with_password uses UTF-8 pointer+length and preserves existing open ABI.",
    },
    {
        "id": "prompt02b.dotnet.password_open",
        "category": "prompt02b-closure",
        "feature": ".NET password-open parity",
        "surfaces": {"dotnet": PUB, "docs": PUB, "packaging": PUB},
        "note": "WellfriendDocument.Open(path/bytes, string? password) routes through the password-aware C ABI.",
    },
    {
        "id": "prompt02b.java.password_open",
        "category": "prompt02b-closure",
        "feature": "Java password-open parity",
        "surfaces": {"java": PUB, "docs": PUB, "packaging": PUB},
        "note": "WellfriendPdf.Document.open(Path/byte[], String password) routes UTF-8 bytes through the password-aware C ABI.",
    },
    {
        "id": "prompt02b.java.maven_package",
        "category": "prompt02b-closure",
        "feature": "Java Maven package smoke",
        "surfaces": {"java": PUB, "docs": PUB, "packaging": PUB},
        "note": "scripts/prompt02b_java_package_smoke.ps1 runs Maven version/test/package with a target-local Maven fallback.",
    },
    {
        "id": "prompt02b.java.gradle_policy",
        "category": "prompt02b-closure",
        "feature": "Java Gradle package support",
        "surfaces": {"java": PUB, "docs": PUB, "packaging": PUB},
        "note": "Prompt 02C adds build.gradle/settings.gradle plus a target-local Gradle 9.6.1 bootstrap that runs clean test, jar, build, JAR inspection, runtime smoke, and Maven/Gradle equivalence.",
    },
    {
        "id": "prompt02c.java.gradle_package",
        "category": "prompt02c-closure",
        "feature": "Java Gradle build/package/JAR smoke",
        "surfaces": {"java": PUB, "docs": PUB, "packaging": PUB},
        "note": "scripts/prompt02c_gradle_package_smoke.ps1 runs Gradle version/test/jar/build, smokes build/libs/wellfriendpdf-sdk-0.1.0.jar, and writes gradle-jar-smoke plus Maven/Gradle equivalence artifacts.",
    },
    {
        "id": "prompt02b.java.jar_verification",
        "category": "prompt02b-closure",
        "feature": "Java JAR package verification",
        "surfaces": {"java": PUB, "docs": PUB, "packaging": PUB},
        "note": "Package smoke inspects the JAR as a ZIP, rejects test/native/build-junk entries, and runs from the packaged artifact.",
    },
    {
        "id": "prompt02b.progress_posture",
        "category": "prompt02b-closure",
        "feature": "Prompt 02B progress closure",
        "surfaces": {"wasm": UNSUP, "dotnet": UNSUP, "java": UNSUP, "docs": PUB, "packaging": PUB},
        "note": "Shared feature report records progress_not_supported; no binding exposes no-op callbacks.",
    },
    {
        "id": "prompt02b.cancellation_posture",
        "category": "prompt02b-closure",
        "feature": "Prompt 02B cancellation closure",
        "surfaces": {"wasm": UNSUP, "dotnet": UNSUP, "java": UNSUP, "docs": PUB, "packaging": PUB},
        "note": "Shared feature report records binding cancellation unsupported while naming render internals that already observe CancelToken.",
    },
    {
        "id": "prompt02b.memory_evidence",
        "category": "prompt02b-closure",
        "feature": "Prompt 02B memory/leak evidence",
        "surfaces": {"wasm": DEF, "dotnet": PUB, "java": PUB, "c_abi": PUB, "docs": PUB, "packaging": PUB},
        "note": "C ABI/.NET/Java repeated open/report/dispose stress tests plus existing Linux sanitizer CI gate; local Valgrind/LLVM cov unavailable on Windows host.",
    },
    {
        "id": "diag.panic_boundary",
        "category": "prompt02-diagnostics",
        "feature": "panic and exception boundary",
        "surfaces": {"wasm": PUB, "dotnet": PUB, "java": PUB, "docs": PUB, "packaging": PUB},
        "note": "WASM maps errors to JsValue; .NET/Java preserve C ABI status and messages.",
    },
    {
        "id": "test.prompt02_smokes",
        "category": "prompt02-release",
        "feature": "Prompt 02 binding smoke tests",
        "surfaces": {"wasm": PART, "dotnet": PUB, "java": PUB, "docs": PUB, "packaging": PUB},
        "note": "cargo wasm build, .NET tests, and Java smoke cover report/output paths; browser glue regeneration requires wasm-pack/wasm-bindgen.",
    },
]


def classify_wasm(row: dict) -> str:
    fid = row["id"]
    cat = row["category"]
    if fid in WASM_PUBLIC:
        return PUB
    if fid in UNSUPPORTED_IDS:
        return UNSUP
    if fid in {"open.byte_source"}:
        return PART
    if cat in {"parser", "decode", "render", "fonts", "text", "color", "semantic", "forms", "security", "standards", "diagnostics"}:
        if row["surfaces"].get("rust") == INT:
            return DEF
        return PART
    if cat == "editing":
        if fid in {"conv.markdown", "conv.json", "writer.full_rewrite", "writer.deterministic"}:
            return PUB
        if fid in {"conv.docx_faithful", "conv.docx_flow", "conv.pptx", "conv.xlsx", "conv.office_to_pdf"}:
            return UNSUP
        return PART
    if cat == "release":
        if fid in {"test.rust", "test.cross_lang_golden", "test.snapshot_schema", "doc.api", "doc.examples", "pkg.metadata", "pkg.platform_matrix", "pkg.ci_smoke", "pkg.release_manifest"}:
            return PART if fid.startswith("pkg.") or fid.startswith("test.") else PUB
        return PART
    return PART


def classify_native(row: dict) -> str:
    fid = row["id"]
    cat = row["category"]
    if fid in NATIVE_PUBLIC:
        return PUB
    if fid in UNSUPPORTED_IDS:
        return UNSUP
    if fid == "open.options":
        return PART
    if row["surfaces"].get("rust") == INT and row["surfaces"].get("c_abi") in {MISS, INT}:
        return DEF
    if cat in {"parser", "decode", "render", "fonts", "text", "color", "semantic", "forms", "security", "standards", "diagnostics"}:
        return PART
    if cat == "editing":
        if fid in {"conv.html", "conv.markdown", "edit.paragraph_reflow", "edit.insert_delete_text", "writer.incremental"}:
            return MISS
        return PART
    if cat == "release":
        return PART
    return PART


def doc_status(row: dict) -> str:
    fid = row["id"]
    if fid in {"diag.progress", "diag.cancellation", "decode.cancellation", "render.cancellation"}:
        return PUB
    if row["category"] == "release":
        return PUB
    return PART


def packaging_status(row: dict) -> str:
    if row["category"] == "release" or row["id"].startswith("pkg."):
        return PART
    return PART


def action_for(row: dict, statuses: dict) -> str:
    gaps = [name for name in ("wasm", "dotnet", "java") if statuses[name] != PUB]
    if not gaps:
        return "No Prompt 02 action; public or report-backed on WASM/.NET/Java."
    if any(statuses[name] == UNSUP for name in gaps):
        return "Unsupported status is intentional and documented; do not expose a fake wrapper."
    if any(statuses[name] == MISS for name in gaps):
        return "Future prompt must add a real shared-facade/C ABI entry point before exposing this surface."
    return "Partial coverage is report-backed; add standalone typed wrapper only when the shared facade supports it."


def normalize_extra(row: dict) -> dict:
    surfaces = {surface: row["surfaces"].get(surface, DEF) for surface in SURFACES}
    for inherited in ("rust", "python", "c_abi", "cli"):
        surfaces[inherited] = row["surfaces"].get(inherited, DEF)
    return {
        "id": row["id"],
        "category": row["category"],
        "feature": row["feature"],
        "surfaces": surfaces,
        "note": row["note"],
        "action": row["note"],
        "tests": "Prompt 02 focused binding smokes and generated artifacts",
    }


def build_rows() -> list[dict]:
    prompt01 = json.loads(PROMPT01_JSON.read_text(encoding="utf-8"))
    rows = []
    for source in prompt01["features"]:
        surfaces = {surface: source["surfaces"].get(surface, DEF) for surface in ("rust", "python", "c_abi", "cli")}
        surfaces["wasm"] = classify_wasm(source)
        surfaces["dotnet"] = classify_native(source)
        surfaces["java"] = classify_native(source)
        surfaces["docs"] = doc_status(source)
        surfaces["packaging"] = packaging_status(source)
        rows.append(
            {
                "id": source["id"],
                "category": source["category"],
                "feature": source["feature"],
                "surfaces": surfaces,
                "note": source.get("note", ""),
                "action": action_for(source, surfaces),
                "tests": tests_for(source["id"], source["category"], surfaces),
            }
        )
    rows.extend(normalize_extra(row) for row in EXTRA_ROWS)
    return rows


def tests_for(fid: str, category: str, surfaces: dict) -> str:
    if category == "release":
        return "pack/build/test commands listed in docs and final validation"
    if fid.startswith(("diag.", "test.", "pkg.", "doc.")):
        return "Prompt 02 matrix/docs plus focused smoke artifacts"
    if any(surfaces[name] == PUB for name in ("wasm", "dotnet", "java")):
        return "WASM cargo build; .NET WellfriendPdfSmokeTests; Java WellfriendPdfSmokeTest; C ABI facade tests"
    return "Matrixed as partial/unsupported/deferred; no fake wrapper test"


def counts(rows: list[dict]) -> dict:
    by_surface = {}
    for surface in SURFACES:
        by_surface[surface] = dict(Counter(row["surfaces"][surface] for row in rows))
    return by_surface


def write_json(rows: list[dict]) -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    payload = {
        "schema_version": 1,
        "prompt": "combined-02-wasm-dotnet-java-diagnostics-parity",
        "envelope_version": 1,
        "status_vocabulary": VOCAB,
        "surfaces": SURFACES,
        "feature_count": len(rows),
        "surface_tallies": counts(rows),
        "non_public_rows": [
            {
                "id": row["id"],
                "feature": row["feature"],
                "surfaces": {k: v for k, v in row["surfaces"].items() if k in {"wasm", "dotnet", "java"} and v != PUB},
                "action": row["action"],
            }
            for row in rows
            if any(row["surfaces"][surface] != PUB for surface in ("wasm", "dotnet", "java"))
        ],
        "features": rows,
    }
    JSON_OUT.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_markdown(rows: list[dict]) -> None:
    tallies = counts(rows)
    by_category: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        by_category[row["category"]].append(row)

    lines = [
        "# Combined Prompt 02 - Binding Gap Matrix",
        "",
        "Human-readable view of `target/prompt02-binding-parity/binding-gap-matrix.json`.",
        "",
        f"**Rows:** {len(rows)}",
        "",
        "## Surface Counts",
        "",
        "| Surface | implemented_public | partial_public | unsupported_reported | missing | deferred | implemented_internal | cli_only | blocked |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for surface in SURFACES:
        c = tallies[surface]
        lines.append(
            f"| {surface} | {c.get(PUB, 0)} | {c.get(PART, 0)} | {c.get(UNSUP, 0)} | "
            f"{c.get(MISS, 0)} | {c.get(DEF, 0)} | {c.get(INT, 0)} | {c.get(CLI, 0)} | {c.get(BLK, 0)} |"
        )
    lines.extend(
        [
            "",
            "Statuses: `implemented_public`, `partial_public`, `implemented_internal`, `cli_only`, `unsupported_reported`, `missing`, `deferred`, `blocked`.",
            "",
        ]
    )

    for category in sorted(by_category):
        lines.extend(
            [
                f"## {category}",
                "",
                "| Feature | Rust | Python | C ABI | WASM | .NET | Java | CLI | Docs | Packaging | Action |",
                "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
            ]
        )
        for row in by_category[category]:
            s = row["surfaces"]
            feature = f"{row['feature']} (`{row['id']}`)"
            action = row["action"].replace("|", "/")
            lines.append(
                f"| {feature} | {s['rust']} | {s['python']} | {s['c_abi']} | {s['wasm']} | "
                f"{s['dotnet']} | {s['java']} | {s['cli']} | {s['docs']} | {s['packaging']} | {action} |"
            )
        lines.append("")

    while lines and lines[-1] == "":
        lines.pop()
    MD_OUT.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    rows = build_rows()
    write_json(rows)
    write_markdown(rows)
    print(f"wrote {JSON_OUT}")
    print(f"wrote {MD_OUT}")
    print(json.dumps(counts(rows), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
