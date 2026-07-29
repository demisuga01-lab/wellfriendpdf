#!/usr/bin/env bash
set -uo pipefail

if [ "$#" -lt 5 ]; then
  echo "usage: prompt35_vps_stage.sh <stage> <result-dir> <source-dir> <tmp-dir> <command> [args...]" >&2
  exit 64
fi

stage="$1"
result_dir="$2"
source_dir="$3"
tmp_dir="$4"
shift 4

log_dir="$result_dir/logs"
log="$log_dir/$stage.log"
mem="$result_dir/$stage.mem"
time_file="$result_dir/$stage.time"

mkdir -p "$log_dir" "$tmp_dir"

start="$(date +%s)"
set +e
(
  cd "$source_dir" &&
  export TMPDIR="$tmp_dir" TMP="$tmp_dir" TEMP="$tmp_dir" &&
  /usr/bin/time -f "PEAK_RSS_KIB=%M" -o "$mem" "$@"
) >"$log" 2>&1
code="$?"
set -e
end="$(date +%s)"

peak="$(sed -n 's/^PEAK_RSS_KIB=//p' "$mem" 2>/dev/null | tail -1)"
if [ -z "$peak" ]; then
  peak=0
fi
duration="$((end - start))"
printf 'EXIT=%s DURATION_SEC=%s PEAK_RSS_KIB=%s\n' "$code" "$duration" "$peak" >"$time_file"
sha="$(sha256sum "$log" | awk '{print $1}')"
printf 'EXIT=%s DURATION_SEC=%s PEAK_RSS_KIB=%s SHA256=%s\n' "$code" "$duration" "$peak" "$sha"
exit "$code"
