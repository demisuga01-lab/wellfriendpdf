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
manifest="$out_dir/manifest.json"
"$harness" --input "$fixture" --page 1 --dpi 72 --annotations 1 --forms 1 \
  --page-box media --pixel-format bgra --workers 1 \
  --output "$raw" --jsonl "$jsonl" --manifest "$manifest"

test -s "$raw"
test -s "$jsonl"
test -s "$manifest"
grep -q '"engine":"pdfium-c"' "$jsonl"
grep -q '"status":"ok"' "$jsonl"
grep -q '"width":' "$jsonl"
grep -q '"pixel_format":"bgra"' "$jsonl"
grep -q '"hash_fnv1a64":' "$jsonl"
grep -q '"harness":"wellfriend-pdfium-harness"' "$manifest"
grep -q '"pdfium_runtime_version":"not_exposed_by_public_c_api"' "$manifest"
printf 'PDFIUM_HARNESS_SMOKE=PASS output=%s jsonl=%s manifest=%s\n' "$raw" "$jsonl" "$manifest"
