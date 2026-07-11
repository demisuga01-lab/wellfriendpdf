import json
import sys

import oxide


if len(sys.argv) != 2:
    raise SystemExit("usage: prompt19_interactive_docx.py input.pdf")

source = sys.argv[1]
document = oxide.open(source)
print(json.dumps(document.form_js_report(), indent=2))
print(json.dumps(document.interactive_data_report(), indent=2))
print(json.dumps(document.prompt19_report(), indent=2))

oxide.pdf_to_docx(
    source,
    output="prompt19-page-faithful.docx",
    include_images=True,
    layout="page-faithful",
)
