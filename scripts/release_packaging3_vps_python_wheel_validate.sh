#!/usr/bin/env bash
# Build a fresh isolated Python environment for one already-built text reflow
# wheel, then execute the binding smoke suite without mutating the host Python.
set -euo pipefail

source_root=${1:?source root is required}
result_root=${2:?result root is required}
wheel_path=${3:?wheel path is required}
venv_root=${4:?fresh venv root is required}

if [[ -e "$venv_root" ]]; then
    echo "fresh Python wheel validation requires a non-existent venv path: $venv_root" >&2
    exit 2
fi

mkdir -p "$result_root"
python3 -m venv "$venv_root"
"$venv_root/bin/pip" install --disable-pip-version-check -q "$wheel_path" pytest \
    >"$result_root/wheel-install.log" 2>&1

set +e
/usr/bin/time -f 'EXIT=%x DURATION_SEC=%e PEAK_RSS_KIB=%M' \
    -o "$result_root/wheel-test.time" \
    "$venv_root/bin/python" -m pytest -q "$source_root/crates/wellfriendpdf-py/tests/test_smoke.py" \
    >"$result_root/wheel-test.log" 2>&1
status=$?
set -e
timing=$(tr '\n' ' ' <"$result_root/wheel-test.time")
printf 'STAGE=python-fresh-wheel STATUS=%s %s ARTIFACT=%s\n' \
    "$([ "$status" -eq 0 ] && printf pass || printf fail)" \
    "$timing" \
    "$result_root/wheel-test.log"
exit "$status"
