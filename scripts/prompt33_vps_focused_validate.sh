#!/usr/bin/env bash
# Run a bounded, serial focused Prompt 33 validation in a transferred source tree.
# Detailed output stays in the supplied result directory; stdout is one compact
# stage line per command for the invoking workstation.
set -euo pipefail

source_root=${1:?source root is required}
result_root=${2:?result root is required}
temp_root=${3:?temporary root is required}

mkdir -p "$result_root/logs"
export PATH="$HOME/.cargo/bin:$PATH"
# `/home` may hold retained Prompt 33 evidence and therefore be full. Callers
# can place only the disposable Cargo cache on another verified filesystem
# without changing the transferred source or result/evidence roots.
export CARGO_TARGET_DIR="${WELLPDF_PROMPT33_CARGO_TARGET_DIR:-$temp_root/target}"
mkdir -p "$CARGO_TARGET_DIR"
export CARGO_BUILD_JOBS=1
export RUST_TEST_THREADS=1
export RAYON_NUM_THREADS=1

run_stage() {
    local stage=$1
    shift
    set +e
    /usr/bin/time -f 'EXIT=%x DURATION_SEC=%e PEAK_RSS_KIB=%M' \
        -o "$result_root/$stage.time" \
        "$@" >"$result_root/logs/$stage.log" 2>&1
    local status=$?
    set -e
    local timing
    timing=$(tr '\n' ' ' <"$result_root/$stage.time")
    printf 'STAGE=%s STATUS=%s %s ARTIFACT=%s\n' \
        "$stage" \
        "$([ "$status" -eq 0 ] && printf pass || printf fail)" \
        "$timing" \
        "$result_root/logs/$stage.log"
    return "$status"
}

cd "$source_root"
run_stage engine-prompt33 cargo test -p wellfriendpdf-engine --lib prompt33 -- --nocapture
run_stage capi-prompt33 cargo test -p wellfriendpdf-capi capi_prompt33_reflow_surfaces_return_owned_outputs -- --nocapture
run_stage workspace-check cargo check --workspace --all-targets --jobs 1
run_stage workspace-clippy cargo clippy --workspace --all-targets --jobs 1 -- -D warnings
