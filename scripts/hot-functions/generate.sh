#!/bin/bash
# Regenerate crates/cli/link/hot-functions.txt, the function order the
# `wellfriendpdf` executable is linked with on x86_64 Linux (see
# crates/cli/build.rs).
#
# usage: scripts/hot-functions/generate.sh [PDF_DIR...]
#
# Builds the release CLI, runs it on every PDF under PDF_DIR (default:
# tests/corpus/pdfs and crates/engine/tests/fixtures) with a tracer that
# records the first instruction run in each 4 KiB page of the executable's
# code (re-armed every 2 ms), maps those addresses to functions, and writes
# them most-used first. Needs gcc, readelf, nm and python3. The list names
# symbols as this toolchain and Cargo.lock mangle them; a stale list only
# loses the memory saving (the linker skips names it does not find).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
HERE="$ROOT/scripts/hot-functions"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
# Link without the old order so the trace sees the code where the compiler
# put it; the list is then the same whatever order was used before.
WELLFRIENDPDF_NO_HOT_ORDER=1 cargo build --release -p wellfriendpdf-cli --manifest-path "$ROOT/Cargo.toml"
BIN="$TARGET_DIR/release/wellfriendpdf"
gcc -O1 -shared -fPIC -o "$WORK/trace.so" "$HERE/trace.c"
DIRS=("$@")
if [ ${#DIRS[@]} -eq 0 ]; then
  DIRS=("$ROOT/tests/corpus/pdfs" "$ROOT/crates/engine/tests/fixtures")
fi
n=0
while IFS= read -r -d '' pdf; do
  n=$((n + 1))
  for cmd in "render:--format png --dpi 150 --pages all -o $WORK/out.zip" "extract-text:"; do
    sub=${cmd%%:*}
    args=${cmd#*:}
    # shellcheck disable=SC2086
    timeout 300 env TRACE_EXE=wellfriendpdf TRACE_OUT="$WORK/trace-$n-$sub.txt" \
      LD_PRELOAD="$WORK/trace.so" "$BIN" --threads 1 "$sub" "$pdf" $args >/dev/null 2>&1 || true
    rm -f "$WORK/out.zip"
  done
done < <(find "${DIRS[@]}" -name '*.pdf' -print0 | sort -z)
mkdir -p "$ROOT/crates/cli/link"
python3 "$HERE/gen_order.py" "$BIN" "$ROOT/crates/cli/link/hot-functions.txt" "$WORK"/trace-*.txt
