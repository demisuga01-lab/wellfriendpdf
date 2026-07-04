"""Cross-language SDK facade demo (Python side).

Runs the oxide report surfaces over a PDF and prints a compact summary. This is
the Python counterpart of the Rust `sdk_reports` example and the C
`sdk_reports.c` example — all three call the SAME shared facade and receive the
SAME versioned-JSON envelopes.

    python sdk_reports.py input.pdf [out.json]

With a second argument, writes every report envelope to that path as one JSON
object (used to generate the Prompt-01 Python smoke artifact).
"""

import json
import sys

import oxide


def main() -> int:
    path = sys.argv[1] if len(sys.argv) > 1 else None
    if path is None:
        print("usage: sdk_reports.py input.pdf [out.json]", file=sys.stderr)
        return 2
    out = sys.argv[2] if len(sys.argv) > 2 else None

    doc = oxide.open(path)

    reports = {
        "feature": oxide.feature_report(),
        "document_info": {"page_count": doc.page_count, "metadata": doc.metadata},
        "security": doc.security_report(),
        "parser": doc.parser_report(mode="audit"),
        "color": doc.color_report(),
        "fonts": doc.font_report(),
        "signatures": doc.signature_report(),
        "forms": doc.forms_report(),
        "annotations": doc.annotations_report(),
        "pages": doc.pages_report(),
        "interactive": doc.interactive_report(),
        "standards": doc.validate(),
        "pdfa": doc.validate_pdfa(),
        "pdfua": doc.validate_pdfua(),
        "chunk": doc.chunks(),
        "text_semantic": doc.text_semantic(),
        "decode_budget": oxide.decode_budget_report("DCTDecode", 4096, 4096, 3),
    }

    print(f"oxide SDK facade — {path}")
    for name, report in reports.items():
        kind = report.get("kind", "-")
        schema = report.get("schema_version", "-")
        print(f"  {name:<14} kind={kind} schema={schema}")

    if out is not None:
        payload = {
            "envelope_version": oxide.__report_envelope_version__,
            "source": path,
            **reports,
        }
        with open(out, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, indent=2)
        print(f"wrote {out}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
