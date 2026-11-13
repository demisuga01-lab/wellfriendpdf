#!/usr/bin/env python3
"""Turn text-page traces (trace.so output) into an lld --symbol-ordering-file.

usage: gen_order.py BINARY OUT TRACE...

Each trace line is a file offset in BINARY's executable segment where a page
was first executed. The function containing each offset is listed once, most
frequently hit (across traces) first, ties by first appearance.
"""
import bisect
import collections
import subprocess
import sys

binary, out = sys.argv[1], sys.argv[2]
traces = sys.argv[3:]

# executable segment: vaddr - offset
delta = None
for line in subprocess.run(["readelf", "-lW", binary], capture_output=True, text=True).stdout.splitlines():
    f = line.split()
    if len(f) >= 7 and f[0] == "LOAD" and "E" in f[6:]:
        delta = int(f[2], 16) - int(f[1], 16)
assert delta is not None

syms = []
for line in subprocess.run(["nm", "--defined-only", "-S", binary], capture_output=True, text=True).stdout.splitlines():
    p = line.split()
    if len(p) != 4 or p[2] not in "tTwW":
        continue
    syms.append((int(p[0], 16), int(p[1], 16), p[3]))
syms.sort()
addrs = [s[0] for s in syms]

count = collections.Counter()
first = {}
seq = 0
for path in traces:
    seen = set()
    for line in open(path):
        line = line.strip()
        if not line:
            continue
        va = int(line, 16) + delta
        i = bisect.bisect_right(addrs, va) - 1
        if i < 0:
            continue
        a, size, name = syms[i]
        if not (a <= va < a + max(size, 1)):
            continue
        if name not in first:
            first[name] = seq
            seq += 1
        if name not in seen:
            seen.add(name)
            count[name] += 1

order = sorted(first, key=lambda n: (-count[n], first[n]))
with open(out, "w") as fh:
    for name in order:
        fh.write(name + "\n")
size = {s[2]: s[1] for s in syms}
print("functions", len(order), "bytes", sum(size.get(n, 0) for n in order))
