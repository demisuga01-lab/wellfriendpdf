#!/usr/bin/env bash
# Direct PDFium harness smoke only. This script intentionally renders one
# repository-owned fixture and performs no timing, corpus, or comparison work.
set -euo pipefail

harness=${1:?usage: smoke.sh HARNESS FIXTURE [OUT_DIR]}
fixture=${2:?usage: smoke.sh HARNESS FIXTURE [OUT_DIR]}
out_dir=${3:-"$(mktemp -d)"}
mkdir -p "$out_dir"

raw="$out_dir/page-1.bgra"
jsonl="$out_dir/result.jsonl"
"$harness" --input "$fixture" --page 1 --dpi 72 --annotations 1 --forms 1 \
  --output "$raw" --jsonl "$jsonl"

test -s "$raw"
test -s "$jsonl"
grep -q '"engine":"pdfium-c"' "$jsonl"
grep -q '"status":"ok"' "$jsonl"
grep -q '"width":' "$jsonl"
grep -q '"hash_fnv1a64":' "$jsonl"
printf 'PDFIUM_HARNESS_SMOKE=PASS output=%s manifest=%s\n' "$raw" "$jsonl"
