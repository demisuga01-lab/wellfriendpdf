#!/usr/bin/env bash
# Run exactly one bounded Prompt 33 validation stage in a transferred source
# tree. The caller supplies the command as argv, avoiding shell-eval quoting
# and keeping every stage's real exit/timing evidence separate.
set -euo pipefail

source_root=${1:?source root is required}
result_root=${2:?result root is required}
temp_root=${3:?temporary root is required}
stage=${4:?stage name is required}
shift 4

if [[ $# -eq 0 ]]; then
    printf 'prompt33_vps_run_stage: validation command is required\n' >&2
    exit 64
fi

mkdir -p "$result_root/logs"
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="${WELLPDF_PROMPT33_CARGO_TARGET_DIR:-$temp_root/target}"
export CARGO_BUILD_JOBS=1
export RUST_TEST_THREADS=1
export RAYON_NUM_THREADS=1
mkdir -p "$CARGO_TARGET_DIR"

log="$result_root/logs/$stage.log"
time_file="$result_root/$stage.time"
set +e
(
    cd "$source_root"
    /usr/bin/time -f 'EXIT=%x DURATION_SEC=%e PEAK_RSS_KIB=%M' -o "$time_file" "$@"
) >"$log" 2>&1
status=$?
set -e

if [[ -f "$time_file" ]]; then
    timing=$(tr '\n' ' ' <"$time_file")
else
    timing='EXIT=unavailable DURATION_SEC=unavailable PEAK_RSS_KIB=unavailable'
fi
if [[ "$status" -eq 0 ]]; then
    state=pass
else
    state=fail
fi
printf 'STAGE=%s STATUS=%s %s FAILURES=%s ARTIFACT=%s\n' \
    "$stage" "$state" "$timing" "$([[ "$status" -eq 0 ]] && printf 0 || printf 1)" "$log"
exit "$status"
