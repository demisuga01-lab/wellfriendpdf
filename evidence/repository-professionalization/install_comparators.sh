#!/usr/bin/env bash
set -euo pipefail
VENV=/home/demisuga01/wellpdf/tmp/repository-professionalization-20260729T192340Z/comparator-venv
python3 -m venv "$VENV"
"$VENV/bin/python" -m pip install --no-cache-dir pypdfium2 pymupdf pikepdf pdfplumber pyhanko
