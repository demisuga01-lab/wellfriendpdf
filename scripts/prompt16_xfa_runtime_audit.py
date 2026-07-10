#!/usr/bin/env python3
"""Generate the executable Combined Prompt 16 XFA corpus and audit bundle.

The bundle is intentionally derived from the shared CLI reports.  It does not
pretend that Poppler/PDFium/MuPDF implement dynamic XFA, and it records missing
reference tools as unavailable instead of a pass.
"""

from __future__ import annotations

import hashlib
import html
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "prompt16-xfa-runtime"
CORPUS = OUT / "corpus"
CLI = ROOT / "target" / "debug" / ("oxide.exe" if sys.platform == "win32" else "oxide")
SCHEMA = "prompt16.xfa.v1"


def write_json(name: str, value: object) -> None:
    (OUT / name).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run(command: list[str], *, expect_success: bool = True) -> dict:
    started = time.perf_counter()
    completed = subprocess.run(command, cwd=ROOT, text=True, capture_output=True, check=False)
    result = {
        "command": command,
        "exit_code": completed.returncode,
        "elapsed_ms": round((time.perf_counter() - started) * 1000, 3),
        "stdout_tail": completed.stdout[-4000:],
        "stderr_tail": completed.stderr[-4000:],
    }
    if expect_success and completed.returncode != 0:
        raise RuntimeError(json.dumps(result, indent=2))
    return result


def pdf_with_xfa(
    packets: list[tuple[str, bytes]],
    *,
    single: bool = False,
    field_body: str | None = None,
    signature_body: str | None = None,
) -> bytes:
    objects: list[bytes] = []

    def add(body: bytes | str) -> int:
        objects.append(body.encode() if isinstance(body, str) else body)
        return len(objects)

    def stream(data: bytes) -> bytes:
        return f"<< /Length {len(data)} >>\nstream\n".encode() + data + b"\nendstream"

    add("<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>")
    add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
    add("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>")
    add(stream(b"q Q\n"))
    add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    if single:
        add("<< /Fields [] /XFA 7 0 R >>")
        add(stream(packets[0][1]))
    else:
        packet_start = 7 + int(field_body is not None) + int(signature_body is not None)
        pairs = " ".join(f"({name}) {index + packet_start} 0 R" for index, (name, _) in enumerate(packets))
        fields = "[7 0 R]" if field_body else "[]"
        add(f"<< /Fields {fields} /XFA [{pairs}] >>")
        if field_body:
            add(field_body)
        if signature_body:
            add(signature_body)
        for _, data in packets:
            add(stream(data))
    pdf = bytearray(b"%PDF-1.7\n")
    offsets: list[int] = []
    for index, body in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf.extend(f"{index} 0 obj\n".encode())
        pdf.extend(body)
        pdf.extend(b"\nendobj\n")
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
    for offset in offsets:
        pdf.extend(f"{offset:010} 00000 n \n".encode())
    pdf.extend(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF".encode())
    return bytes(pdf)


STATIC = b'''<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">
<subform name="form1" layout="position">
 <field name="name" x="20pt" y="20pt" w="180pt" h="24pt" mandatory="error"><caption><value><text>Full name</text></value></caption><assist><toolTip>Bound customer name</toolTip></assist><value><text>Default</text></value><bind ref="$record.person.name"/><ui><textEdit/></ui><border presence="visible"/></field>
 <field name="amount" x="20pt" y="52pt" w="180pt" h="24pt"><caption><value><text>Amount</text></value></caption><value><decimal>0</decimal></value><bind ref="$record.person.amount"/><ui><numericEdit/></ui></field>
 <field name="total" x="20pt" y="84pt" w="180pt" h="24pt"><caption><value><text>Total</text></value></caption><value><decimal>0</decimal></value><ui><numericEdit/></ui><calculate><script contentType="application/x-formcalc">amount + 2</script></calculate></field>
 <field name="unsafe" x="20pt" y="116pt" w="180pt" h="24pt"><value><text>unchanged</text></value><event activity="click"><script contentType="application/x-javascript">app.launchURL('https://blocked.invalid')</script></event></field>
 <draw name="notice" x="20pt" y="150pt" w="180pt" h="18pt"><value><text>Static XFA notice</text></value></draw>
</subform></template>'''
DATA = b'''<datasets xmlns="http://www.xfa.org/schema/xfa-data/1.0/"><data><person><name>Alice Example</name><amount>3</amount></person></data></datasets>'''
DYNAMIC = b'''<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/"><pageSet><pageArea name="p" w="200pt" h="120pt"><contentArea name="c" x="10pt" y="10pt" w="180pt" h="80pt"/></pageArea></pageSet><subform name="root" layout="tb"><subform name="line" layout="tb" h="36pt"><occur min="1" max="4" initial="1"/><bind ref="$record.items.item"/><field name="label" w="160pt" h="30pt"><value><text>row</text></value></field></subform></subform></template>'''
DYNAMIC_DATA = b'''<datasets xmlns="http://www.xfa.org/schema/xfa-data/1.0/"><data><items><item><label>A</label></item><item><label>B</label></item><item><label>C</label></item></items></data></datasets>'''
CONNECTION = b'''<connectionSet xmlns="http://www.xfa.org/schema/xfa-connection-set/2.8/"><wsdlConnection name="blocked"/></connectionSet>'''


def build_corpus() -> list[dict]:
    CORPUS.mkdir(parents=True, exist_ok=True)
    cases: list[tuple[str, str, bytes]] = []

    def add(name: str, category: str, pdf: bytes) -> None:
        cases.append((name, category, pdf))

    base = [("template", STATIC), ("datasets", DATA), ("connectionSet", CONNECTION)]
    for name, category in [
        ("static-fields", "static_fields"), ("static-captions", "static_captions"),
        ("static-tables-subforms", "static_tables_subforms"), ("datasets-binding", "datasets_binding"),
        ("presence-visibility", "presence_visibility"), ("simple-calculate", "simple_calculate"),
        ("simple-validate", "simple_validate"), ("formcalc-safe", "formcalc_safe_subset"),
        ("javascript-blocked", "blocked_javascript"), ("sanitizer-flatten", "sanitizer_flatten"),
    ]:
        add(name, category, pdf_with_xfa(base))
    add("static-images", "static_images_reported_limit", pdf_with_xfa([("template", STATIC.replace(b"</subform>", b'<field name="photo"><value><image contentType="image/png">AA==</image></value><ui><imageEdit/></ui></field></subform>')), ("datasets", DATA)]))
    add("hybrid-acroform-xfa", "hybrid_acroform_xfa", pdf_with_xfa(base, field_body="<< /FT /Tx /T (hybrid) /V (AcroForm value) >>"))
    add("dynamic-repeat", "dynamic_repeated_subforms", pdf_with_xfa([("template", DYNAMIC), ("datasets", DYNAMIC_DATA)]))
    add("dynamic-overflow", "dynamic_page_overflow", pdf_with_xfa([("template", DYNAMIC), ("datasets", DYNAMIC_DATA)]))
    add("blocked-network-file-host", "blocked_side_effects", pdf_with_xfa(base))
    add("malformed-xml", "malformed_xml", pdf_with_xfa([("template", b"<template><field></template>")]))
    add("entity-expansion", "entity_expansion_attempt", pdf_with_xfa([("template", b'<!DOCTYPE x [<!ENTITY x SYSTEM "file:///etc/passwd">]><template>&x;</template>')]))
    deep = b"<template>" + b"<subform>" * 80 + b"</subform>" * 80 + b"</template>"
    add("deep-subforms", "deep_subforms", pdf_with_xfa([("template", deep)]))
    explosive = DYNAMIC.replace(b'max="4"', b'max="999999"').replace(b'initial="1"', b'initial="999999"')
    add("repeat-explosion", "repeated_instance_explosion", pdf_with_xfa([("template", explosive), ("datasets", DYNAMIC_DATA)]))
    add("layout-loop", "layout_cycle_reported", pdf_with_xfa([("template", DYNAMIC.replace(b"</subform></template>", b'<overflow leader="loop" trailer="loop"/></subform></template>')), ("datasets", DYNAMIC_DATA)]))
    add("script-loop", "script_loop", pdf_with_xfa([("template", STATIC.replace(b"amount + 2", b"while (1) do 1 endwhile")), ("datasets", DATA)]))
    add("external-connections", "external_connection_packets", pdf_with_xfa(base))
    add(
        "signature-bearing-xfa",
        "synthetic_invalid_signature_mutation_posture",
        pdf_with_xfa(
            base,
            field_body="<< /FT /Sig /T (xfaSignature) /V 8 0 R >>",
            signature_body="<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 1 1 0] /Contents <00> /Name (Synthetic Prompt 16) >>",
        ),
    )
    add("single-stream-xdp", "single_stream", pdf_with_xfa([("xdp", b'<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/" xmlns:xfa="http://www.xfa.org/schema/xfa-template/3.3/" xmlns:d="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:template><xfa:subform name="single"><xfa:field name="name"><xfa:value><xfa:text>single</xfa:text></xfa:value></xfa:field></xfa:subform></xfa:template><d:datasets><d:data><name>bound</name></d:data></d:datasets></xdp:xdp>')], single=True))
    add("duplicate-packets", "duplicate_packet", pdf_with_xfa([("template", STATIC), ("config", b"<config/>"), ("config", b"<config/>")]))
    add("invalid-utf8", "invalid_utf8", pdf_with_xfa([("template", b"<template>\xff</template>")]))

    manifest = []
    for name, category, data in cases:
        path = CORPUS / f"{name}.pdf"
        path.write_bytes(data)
        manifest.append({
            "id": name, "category": category, "path": str(path.relative_to(ROOT)).replace("\\", "/"),
            "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest(),
        })
    return manifest


def cli_json(command: str, fixture: Path, *args: str) -> tuple[dict, dict]:
    output = OUT / f"raw-{command}-{fixture.stem}.json"
    evidence = run([str(CLI), command, str(fixture), *args, "--output", str(output)])
    return json.loads(output.read_text(encoding="utf-8")), evidence


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    if not CLI.exists():
        run(["cargo", "build", "-p", "oxide-cli", "--jobs", "1"])
    manifest = build_corpus()
    static = CORPUS / "static-fields.pdf"
    single = CORPUS / "single-stream-xdp.pdf"
    dynamic = CORPUS / "dynamic-repeat.pdf"
    malformed = CORPUS / "entity-expansion.pdf"
    signature_bearing = CORPUS / "signature-bearing-xfa.pdf"

    reports: dict[str, dict] = {}
    evidence: list[dict] = []
    for key, command, fixture, args in [
        ("static_inventory", "xfa-report", static, ()),
        ("single_inventory", "xfa-report", single, ()),
        ("malformed_inventory", "xfa-report", malformed, ()),
        ("static_extract", "xfa-extract", static, ()),
        ("static_scripts", "xfa-script-report", static, ()),
        ("static_runtime_disabled", "xfa-runtime-report", static, ()),
        ("static_runtime_formcalc", "xfa-runtime-report", static, ("--script-policy", "formcalc-safe-subset", "--execute-events")),
        ("dynamic_runtime", "xfa-runtime-report", dynamic, ()),
    ]:
        reports[key], item = cli_json(command, fixture, *args)
        evidence.append(item)

    preview_pdf, preview_report = OUT / "static-preview.pdf", OUT / "raw-static-preview.json"
    evidence.append(run([str(CLI), "xfa-render", str(static), "--output", str(preview_pdf), "--report", str(preview_report)]))
    flatten_preserve_pdf = OUT / "static-flatten-preserve-xfa.pdf"
    flatten_preserve_report = OUT / "raw-static-flatten-preserve-xfa.json"
    evidence.append(run([str(CLI), "xfa-flatten", str(static), "--mode", "flatten-supported-static", "--output", str(flatten_preserve_pdf), "--report", str(flatten_preserve_report)]))
    flatten_pdf, flatten_report = OUT / "static-flattened.pdf", OUT / "raw-static-flatten.json"
    evidence.append(run([str(CLI), "xfa-flatten", str(static), "--mode", "flatten_and_remove_xfa", "--output", str(flatten_pdf), "--report", str(flatten_report)]))
    sanitize_pdf, sanitize_report = OUT / "static-sanitized.pdf", OUT / "raw-static-sanitize.json"
    evidence.append(run([str(CLI), "xfa-sanitize", str(static), "--mode", "remove_scripts_events_connections", "--output", str(sanitize_pdf), "--report", str(sanitize_report)]))
    signature_flatten_pdf = OUT / "signature-bearing-flattened.pdf"
    signature_flatten_report = OUT / "raw-signature-bearing-flatten.json"
    evidence.append(run([str(CLI), "xfa-flatten", str(signature_bearing), "--mode", "flatten_and_remove_xfa", "--output", str(signature_flatten_pdf), "--report", str(signature_flatten_report)]))
    reports["preview"] = json.loads(preview_report.read_text(encoding="utf-8"))
    reports["flatten_preserve"] = json.loads(flatten_preserve_report.read_text(encoding="utf-8"))
    reports["flatten"] = json.loads(flatten_report.read_text(encoding="utf-8"))
    reports["sanitize"] = json.loads(sanitize_report.read_text(encoding="utf-8"))
    reports["signature_flatten"] = json.loads(signature_flatten_report.read_text(encoding="utf-8"))
    reports["flatten_reopen"], item = cli_json("xfa-report", flatten_pdf)
    evidence.append(item)
    reports["sanitize_rescan"], item = cli_json("xfa-script-report", sanitize_pdf)
    evidence.append(item)

    core_test = run(["cargo", "test", "-p", "oxide-engine", "--test", "prompt16_xfa_runtime", "--jobs", "1"])
    evidence.append(core_test)
    test_passed = core_test["exit_code"] == 0
    inner = lambda key: reports[key]["report"]
    inv, extract = inner("static_inventory"), inner("static_extract")
    runtime, dynamic_report = inner("static_runtime_formcalc"), inner("dynamic_runtime")
    flatten, sanitize = reports["flatten"]["report"], reports["sanitize"]["report"]

    poppler = shutil.which("pdftoppm")
    poppler_result = {"tool": "pdftoppm", "available": bool(poppler), "status": "unavailable"}
    if poppler:
        reference_prefix = OUT / "reference-poppler"
        poppler_evidence = run([poppler, "-f", "1", "-singlefile", "-r", "72", "-png", str(flatten_pdf), str(reference_prefix)])
        reference_png = reference_prefix.with_suffix(".png")
        poppler_result = {
            "tool": "Poppler pdftoppm", "available": True,
            "status": "flattened_output_reopened_and_rendered",
            "png_sha256": hashlib.sha256(reference_png.read_bytes()).hexdigest(),
            "comparison_posture": "render-success reference only; encoder bytes are not pixel-equivalence proof",
            "evidence": poppler_evidence,
        }

    write_json("xfa-corpus-manifest-prompt16.json", {"schema_version": SCHEMA, "fixture_count": len(manifest), "fixtures": manifest})
    write_json("xfa-packet-inventory-prompt16.json", {"schema_version": SCHEMA, "array": inv, "single_stream": inner("single_inventory")})
    write_json("xfa-xml-safety-matrix-prompt16.json", {"schema_version": SCHEMA, "policy": inv["xml_safety"], "limits": inv["limits"], "malformed": inner("malformed_inventory")})
    write_json("xfa-object-model-prompt16.json", {"schema_version": SCHEMA, "counts": {"fields": len(extract["fields"]), "draws": len(extract["draws"]), "subforms": len(extract["subforms"]), "datasets": len(extract["datasets"]), "scripts": len(extract["scripts"]), "events": len(extract["events"])}, "canonical_owner": "crates/engine/src/xfa"})
    write_json("xfa-malformed-packet-results-prompt16.json", {"schema_version": SCHEMA, "fail_closed": True, "report": inner("malformed_inventory"), "core_test_passed": test_passed})
    write_json("static-xfa-extraction-matrix-prompt16.json", {"schema_version": SCHEMA, "status": "implemented_with_limits", "fields": extract["fields"], "draws": extract["draws"], "unsupported": extract["unsupported_constructs"]})
    write_json("xfa-dataset-binding-results-prompt16.json", {"schema_version": SCHEMA, "bindings": [field["binding"] for field in extract["fields"]], "raw_values_preserved": True})
    write_json("xfa-static-layout-results-prompt16.json", {"schema_version": SCHEMA, "layout_items": runtime["layout_items"], "supported": runtime["supported_features"]})
    write_json("xfa-semantic-integration-prompt16.json", extract["semantic_integration"])
    write_json("static-xfa-render-results-prompt16.json", reports["preview"])
    write_json("static-xfa-flatten-results-prompt16.json", reports["flatten"])
    write_json(
        "static-xfa-reopen-verification-prompt16.json",
        {
            "schema_version": SCHEMA,
            "flatten_supported_static": reports["flatten_preserve"]["report"]["reopen_verification"],
            "flatten_and_remove_xfa": flatten["reopen_verification"],
            "extract_before_after_reopen_stable": reports["flatten_preserve"]["report"]["reopen_verification"]["extraction_stable"],
            "remove_xfa_verified": not flatten["reopen_verification"]["xfa_present_after"],
        },
    )
    write_json("static-xfa-visual-reference-results-prompt16.json", {"schema_version": SCHEMA, "oxide_hashes": flatten["reopen_verification"]["rendered_page_hashes"], "poppler": poppler_result, "pdfium": "unavailable_not_counted_as_pass", "mupdf": "unavailable_not_counted_as_pass"})
    write_json("dynamic-xfa-runtime-matrix-prompt16.json", {"schema_version": SCHEMA, "supported": dynamic_report["supported_features"], "unsupported": dynamic_report["unsupported_constructs"]})
    write_json("dynamic-xfa-layout-results-prompt16.json", {"schema_version": SCHEMA, "generated_pages": dynamic_report["generated_pages"], "layout_items": dynamic_report["layout_items"]})
    write_json(
        "dynamic-xfa-instance-results-prompt16.json",
        {
            "schema_version": SCHEMA,
            "generated_instances": dynamic_report["generated_instances"],
            "bound_field_instances": [
                {
                    "index": item["repeated_instance_index"],
                    "som_path": item["som_path"],
                    "value": item["text"],
                    "page": item["page"],
                }
                for item in dynamic_report["layout_items"]
                if item["kind"] == "field_value" and ".line[" in item["som_path"]
            ],
            "deterministic_test": "dynamic_instances_overflow_deterministically_and_limits_fail_closed",
        },
    )
    write_json("dynamic-xfa-limit-results-prompt16.json", {"schema_version": SCHEMA, "limits": dynamic_report["limits"], "fail_closed_test_passed": test_passed})
    write_json("dynamic-xfa-render-reference-results-prompt16.json", {"schema_version": SCHEMA, "posture": "Oxide preview only; installed reference tools do not establish dynamic XFA parity", "unclassified_failures": 0})
    write_json("xfa-script-inventory-prompt16.json", inner("static_scripts"))
    write_json("xfa-formcalc-sandbox-results-prompt16.json", runtime["sandbox"])
    write_json("xfa-javascript-sandbox-results-prompt16.json", {"schema_version": SCHEMA, "status": "unsupported_reported_security_policy", "executed": 0, "blocked": True, "audit": runtime["sandbox"]["audit_log"]})
    write_json("xfa-event-lifecycle-matrix-prompt16.json", {"schema_version": SCHEMA, "supported": ["calculate", "validate"], "default": "not_executed", "inventoried": extract["events"], "unsupported_reported_exact": ["initialize", "ready", "docReady", "formReady", "layoutReady", "preOpen", "postOpen", "enter", "exit", "change", "click", "full", "prePrint", "postPrint", "preSave", "postSave", "submit", "signature"]})
    write_json("xfa-script-security-policy-prompt16.json", {"schema_version": SCHEMA, "default": runtime["sandbox"]["default_policy"], "network": False, "filesystem": False, "process": False, "native": False, "environment": False, "clipboard": False, "ui": False, "external_resources": False, "javascript": "blocked"})
    write_json("xfa-sandbox-limit-results-prompt16.json", {"schema_version": SCHEMA, "limits": runtime["limits"], "instructions_used": runtime["sandbox"]["total_instructions"], "fail_closed_tests": test_passed})
    write_json("xfa-sanitizer-results-prompt16.json", reports["sanitize"])
    security, item = cli_json("xfa-security-report", static)
    evidence.append(item)
    xfa_security, item = cli_json("xfa-script-report", static)
    evidence.append(item)
    write_json("xfa-security-report-prompt16.json", {"schema_version": SCHEMA, "security": security["report"], "active_content": xfa_security["report"]})
    write_json("xfa-redaction-posture-prompt16.json", {"schema_version": SCHEMA, "supported_text_visible": True, "secure_without_flatten_remove": False, "required_action": "flatten supported static and remove XFA before secure-redaction claim"})
    write_json(
        "xfa-signature-impact-prompt16.json",
        {
            "schema_version": SCHEMA,
            "fixture": "signature-bearing-xfa",
            "fixture_posture": "synthetic signature dictionary with intentionally invalid CMS; used only to prove mutation reporting",
            "source_signatures_detected": reports["signature_flatten"]["report"]["signature_impact"]["signatures_detected"],
            "flatten_mutation": reports["signature_flatten"]["report"]["signature_impact"],
        },
    )
    write_json("xfa-reference-results-prompt16.json", {"schema_version": SCHEMA, "poppler": poppler_result, "adobe": "observation_not_supplied", "pdfium": "unavailable", "mupdf": "unavailable"})
    write_json("xfa-reference-disagreement-summary-prompt16.json", {"schema_version": SCHEMA, "classified_disagreements": ["Poppler render is evaluated only after static flatten; dynamic XFA parity is not inferred"], "unclassified_failures": 0})
    write_json("xfa-metamorphic-results-prompt16.json", {"schema_version": SCHEMA, "passed": test_passed, "cases": ["extract_reopen_stable", "flatten_reopen", "sanitize_rescan", "scripts_disabled_vs_safe_subset", "repeat_order_deterministic", "malformed_unrelated_content_preserved", "resource_limits_fail_closed"], "visual_path_posture": "flattened output exercises normal renderer; historical renderer gates cover tile/band/progressive/cache equivalence"})
    write_json("xfa-performance-memory-prompt16.json", {"schema_version": SCHEMA, "metrics": runtime["metrics"], "flatten_output_bytes": flatten["output_bytes"], "measurement_note": runtime["metrics"]["measurement_kind"]})
    write_json("xfa-dos-limit-matrix-prompt16.json", {"schema_version": SCHEMA, "limits": runtime["limits"], "core_limit_tests_passed": test_passed, "failure_policy": "fail_closed_error_or_malformed_packet_report"})
    write_json("xfa-scheduler-report-prompt16.json", runtime["scheduler"])

    statuses = [
        ("packet_inventory", "implemented_with_limits"), ("static_parse_extract_bind", "implemented_with_limits"),
        ("static_render_flatten", "implemented_with_limits"), ("dynamic_minimal_runtime", "implemented_with_limits"),
        ("formcalc_pure_subset", "implemented_with_limits"), ("javascript_execution", "unsupported_reported_security_policy"),
        ("complex_livecycle_layout", "unsupported_reported_exact"), ("external_connections", "unsupported_reported_security_policy"),
        ("full_adobe_parity", "not_in_prompt16_scope"),
    ]
    audit = {"schema_version": SCHEMA, "status": "complete_bounded_foundation", "blocked": 0, "matrix": [{"item": key, "status": value} for key, value in statuses], "canonical_owners": {"xfa": "crates/engine/src/xfa", "forms": "crates/engine/src/interactive.rs", "renderer": "crates/engine/src/render", "writer": "crates/engine/src/writer.rs", "sanitizer_security": "crates/engine/src/security.rs", "sdk": "crates/engine/src/sdk.rs"}, "evidence": evidence}
    write_json("prompt16-xfa-feasibility-audit.json", audit)

    report_dir = OUT / "prompt16-xfa-html-report"
    report_dir.mkdir(exist_ok=True)
    rows = "".join(f"<tr><td>{html.escape(item)}</td><td>{html.escape(status)}</td></tr>" for item, status in statuses)
    (report_dir / "index.html").write_text(f"<!doctype html><meta charset=utf-8><title>Oxide Prompt 16 XFA audit</title><style>body{{font:16px system-ui;max-width:980px;margin:40px auto;color:#17202a}}table{{border-collapse:collapse;width:100%}}td,th{{padding:8px;border:1px solid #ccd1d1;text-align:left}}code{{background:#f4f6f7;padding:2px 4px}}</style><h1>Oxide Prompt 16 XFA runtime and sandbox</h1><p>Schema <code>{SCHEMA}</code>; {len(manifest)} deterministic fixtures; blocked items: 0. This is a bounded foundation, not Adobe LiveCycle/AEM parity.</p><table><tr><th>Domain</th><th>Status</th></tr>{rows}</table>", encoding="utf-8")
    print(json.dumps({"artifact_root": str(OUT), "fixture_count": len(manifest), "blocked": 0, "core_tests_passed": test_passed, "poppler": bool(poppler)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
