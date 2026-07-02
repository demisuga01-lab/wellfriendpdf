#!/usr/bin/env python3
"""Fetch the pinned Arlington PDF Model checkout used by Oxide generation."""

from __future__ import annotations

import argparse
import subprocess
from pathlib import Path


DEFAULT_REPO = "https://github.com/pdf-association/arlington-pdf-model.git"
DEFAULT_COMMIT = "5a8639424495c27a30df30bb9491a346f9316014"


def run(cmd: list[str], cwd: Path | None = None) -> None:
    subprocess.run(cmd, cwd=cwd, check=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=DEFAULT_REPO)
    parser.add_argument("--commit", default=DEFAULT_COMMIT)
    parser.add_argument(
        "--out",
        type=Path,
        default=Path("target") / "arlington-pdf-model-5a863942",
    )
    args = parser.parse_args()

    if args.out.exists():
        run(["git", "fetch", "--depth", "1", "origin", args.commit], cwd=args.out)
    else:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        run(["git", "clone", "--depth", "1", args.repo, str(args.out)])
    run(["git", "checkout", "--detach", args.commit], cwd=args.out)
    resolved = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=args.out, text=True
    ).strip()
    if resolved != args.commit:
        raise SystemExit(f"expected {args.commit}, got {resolved}")
    print(f"Arlington checkout ready at {args.out} ({resolved})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
