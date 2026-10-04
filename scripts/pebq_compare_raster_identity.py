#!/usr/bin/env python3
"""Compare native raster identities across two PEBQ JSONL artifacts."""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path
from typing import Any


def load(path: Path, engine: str) -> dict[str, set[tuple[int, int, str]]]:
    grouped: dict[str, set[tuple[int, int, str]]] = defaultdict(set)
    with path.open(encoding="utf-8") as stream:
        for line in stream:
            if not line.strip():
                continue
            row: dict[str, Any] = json.loads(line)
            if row.get("engine") != engine or row.get("status") != "ok":
                continue
            grouped[str(row["relative_path"])].add(
                (
                    int(row.get("width", 0)),
                    int(row.get("height", 0)),
                    str(row.get("raster_fnv1a64", "")),
                )
            )
    return grouped


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--engine", default="wellfriendpdf")
    args = parser.parse_args()

    reference = load(args.reference, args.engine)
    candidate = load(args.candidate, args.engine)
    shared = sorted(set(reference) & set(candidate))
    mismatches = [
        {
            "relative_path": path,
            "reference": sorted(reference[path]),
            "candidate": sorted(candidate[path]),
        }
        for path in shared
        if reference[path] != candidate[path]
    ]
    result = {
        "engine": args.engine,
        "reference_documents": len(reference),
        "candidate_documents": len(candidate),
        "shared_documents": len(shared),
        "matching_documents": len(shared) - len(mismatches),
        "mismatches": mismatches,
    }
    print(json.dumps(result, sort_keys=True))
    return 1 if mismatches or len(shared) != len(reference) or len(shared) != len(candidate) else 0


if __name__ == "__main__":
    raise SystemExit(main())
