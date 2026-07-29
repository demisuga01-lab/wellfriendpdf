#!/usr/bin/env bash
set +e
if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
export PATH="$HOME/.cargo/bin:$PATH"
RES="$1"
SRC="$2"
NAME="$3"
shift 3
cd "$SRC" || exit 99
START=$(date +%s)
/usr/bin/time -v -o "$RES/$NAME.time" "$@" >"$RES/$NAME.log" 2>&1
CODE=$?
END=$(date +%s)
HASH=$(sha256sum "$RES/$NAME.log" | sed 's/ .*//')
RSS=$(python3 - "$RES/$NAME.time" <<'PY'
import re, sys
try:
    text=open(sys.argv[1], encoding='utf-8', errors='replace').read()
except OSError:
    print(0)
    raise SystemExit
m=re.search(r'Maximum resident set size \(kbytes\):\s*(\d+)', text)
print(m.group(1) if m else 0)
PY
)
echo "exit=$CODE duration=$((END-START)) rss=$RSS sha256=$HASH"
exit "$CODE"
