#!/usr/bin/env python3
"""Build the public PDF benchmark corpus manifest.

The downloaded PDFs are intentionally local-only. Commit the manifest and this
script, not the corpus. The script is resumable and records failures instead of
aborting the whole corpus build.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import random
import re
import shutil
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from dataclasses import dataclass, asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_CORPUS_DIR = REPO_ROOT / "public-benchmark" / "corpus" / "pdfs"
DEFAULT_MANIFEST = REPO_ROOT / "public-benchmark" / "manifests" / "public_corpus_manifest.json"
USER_AGENT = "WellfriendPublicBenchmark/1.0 (+https://github.com/demisuga01-lab/wellfriendpdf-sdk)"


@dataclass(frozen=True)
class Candidate:
    id: str
    url: str
    source: str
    category: str
    license: str
    license_url: str | None
    tags: list[str]
    notes: str = ""
    local_source_path: str | None = None


def http_get(url: str, *, timeout: int, retries: int = 3) -> bytes:
    last: Exception | None = None
    for attempt in range(retries):
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as response:
                return response.read()
        except (urllib.error.URLError, TimeoutError, OSError) as err:
            last = err
            time.sleep(min(10.0, 1.5 * (attempt + 1)))
    raise RuntimeError(str(last))


def http_json(url: str, *, timeout: int) -> Any:
    return json.loads(http_get(url, timeout=timeout).decode("utf-8", "replace"))


def sanitize_id(raw: str) -> str:
    value = re.sub(r"[^A-Za-z0-9_.-]+", "_", raw).strip("._")
    return value[:140] or "pdf"


def guess_tags(name: str, source: str, category: str) -> list[str]:
    text = f"{name} {source} {category}".lower()
    tags = {category}
    rules = {
        "forms": ["form", "acro", "xfa", "f1040", "widget", "checkbox", "irs"],
        "tables": ["table", "invoice", "statement", "report"],
        "scanned": ["scan", "image_only", "ocr"],
        "multilang": ["cjk", "arabic", "hebrew", "rtl", "unicode", "japan", "chinese"],
        "pdfa": ["pdfa", "pdf-a", "vera"],
        "security": ["encrypt", "password", "signature", "unicode-password"],
        "pathological": ["safedocs", "compacted", "targeted", "invalid", "edge"],
        "digital-born": ["arxiv", "paper", "pdfjs", "verapdf"],
    }
    for tag, needles in rules.items():
        if any(needle in text for needle in needles):
            tags.add(tag)
    return sorted(tags)


def github_tree(owner: str, repo: str, branch: str, timeout: int) -> list[dict[str, Any]]:
    url = f"https://api.github.com/repos/{owner}/{repo}/git/trees/{branch}?recursive=1"
    data = http_json(url, timeout=timeout)
    return data.get("tree", [])


def github_raw(owner: str, repo: str, branch: str, path: str) -> str:
    quoted = "/".join(urllib.parse.quote(part) for part in path.split("/"))
    return f"https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{quoted}"


def collect_github_pdfs(
    *,
    owner: str,
    repo: str,
    branch: str,
    prefix: str,
    source: str,
    category: str,
    license_name: str,
    license_url: str | None,
    timeout: int,
    limit: int | None,
) -> list[Candidate]:
    out: list[Candidate] = []
    for item in github_tree(owner, repo, branch, timeout):
        path = item.get("path", "")
        if item.get("type") != "blob":
            continue
        if not path.lower().endswith(".pdf"):
            continue
        if prefix and not path.startswith(prefix):
            continue
        name = Path(path).name
        cid = sanitize_id(f"{source}_{Path(path).with_suffix('').as_posix()}")
        out.append(
            Candidate(
                id=cid,
                url=github_raw(owner, repo, branch, path),
                source=source,
                category=category,
                license=license_name,
                license_url=license_url,
                tags=guess_tags(name, source, category),
                notes=f"GitHub path: {path}",
            )
        )
    random.Random(17).shuffle(out)
    return out[:limit] if limit else out


def collect_arxiv(max_items: int, timeout: int, sleep_sec: float) -> list[Candidate]:
    categories = [
        ("cs.AI", "arxiv-cs"),
        ("cs.CL", "arxiv-cs"),
        ("cs.CV", "arxiv-cs"),
        ("cs.DL", "arxiv-cs"),
        ("cs.IR", "arxiv-cs"),
        ("cs.LG", "arxiv-cs"),
        ("cs.SE", "arxiv-cs"),
        ("cs.CR", "arxiv-cs"),
        ("stat.ML", "arxiv-stat"),
        ("stat.AP", "arxiv-stat"),
        ("math.NA", "arxiv-math"),
        ("math.ST", "arxiv-math"),
        ("physics.comp-ph", "arxiv-physics"),
        ("physics.soc-ph", "arxiv-physics"),
        ("astro-ph.IM", "arxiv-astro"),
        ("eess.IV", "arxiv-eess"),
        ("q-bio.QM", "arxiv-qbio"),
        ("econ.EM", "arxiv-econ"),
    ]
    per_category = max(1, (max_items + len(categories) - 1) // len(categories))
    out: list[Candidate] = []
    ns = {"atom": "http://www.w3.org/2005/Atom"}
    for cat, bench_category in categories:
        fetched = 0
        start = 0
        while fetched < per_category and len(out) < max_items:
            batch = min(100, per_category - fetched)
            query = urllib.parse.urlencode(
                {
                    "search_query": f"cat:{cat}",
                    "start": start,
                    "max_results": batch,
                    "sortBy": "submittedDate",
                    "sortOrder": "descending",
                }
            )
            url = f"https://export.arxiv.org/api/query?{query}"
            try:
                data = http_get(url, timeout=timeout).decode("utf-8", "replace")
                root = ET.fromstring(data)
            except Exception as err:  # noqa: BLE001 - source outages are data, not fatal
                message = str(err)
                if "429" in message or "Too Many Requests" in message:
                    print(f"warn: arXiv rate limited for {cat} start={start}; backing off", flush=True)
                    time.sleep(max(60.0, sleep_sec * 6))
                    try:
                        data = http_get(url, timeout=timeout).decode("utf-8", "replace")
                        root = ET.fromstring(data)
                    except Exception as retry_err:  # noqa: BLE001
                        print(f"warn: arXiv query failed for {cat} start={start}: {retry_err}", flush=True)
                        break
                else:
                    print(f"warn: arXiv query failed for {cat} start={start}: {err}", flush=True)
                    break
            entries = root.findall("atom:entry", ns)
            if not entries:
                break
            for entry in entries:
                arxiv_id = (entry.findtext("atom:id", default="", namespaces=ns) or "").rsplit("/", 1)[-1]
                title = " ".join((entry.findtext("atom:title", default="", namespaces=ns) or "").split())
                pdf_url = None
                for link in entry.findall("atom:link", ns):
                    if link.attrib.get("title") == "pdf" or link.attrib.get("type") == "application/pdf":
                        pdf_url = link.attrib.get("href")
                        break
                if not pdf_url and arxiv_id:
                    pdf_url = f"https://arxiv.org/pdf/{arxiv_id}.pdf"
                if not pdf_url:
                    continue
                cid = sanitize_id(f"arxiv_{cat}_{arxiv_id}")
                tags = guess_tags(f"{title} {cat}", "arxiv", bench_category)
                if cat.startswith("cs.") or cat.startswith("stat."):
                    tags.append("tables")
                out.append(
                    Candidate(
                        id=cid,
                        url=pdf_url,
                        source="arxiv",
                        category=bench_category,
                        license="arXiv license metadata varies by paper; local benchmark use only",
                        license_url="https://arxiv.org/help/license",
                        tags=sorted(set(tags)),
                        notes=f"{cat}: {title[:180]}",
                    )
                )
                fetched += 1
                if fetched >= per_category or len(out) >= max_items:
                    break
            start += len(entries)
            time.sleep(sleep_sec)
    return out[:max_items]


def collect_repo_corpus(limit: int | None) -> list[Candidate]:
    manifest_path = REPO_ROOT / "tests" / "corpus" / "manifest.json"
    if not manifest_path.exists():
        return []
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    out: list[Candidate] = []
    for entry in manifest.get("entries", []):
        path = REPO_ROOT / entry["path"]
        if not path.exists():
            continue
        category = str(entry.get("category") or "repo-corpus")
        cid = sanitize_id(f"repo_{entry.get('id') or path.stem}")
        out.append(
            Candidate(
                id=cid,
                url=str(entry.get("source_url") or path),
                source=str(entry.get("source") or "repo-corpus"),
                category=category,
                license=str(entry.get("license") or "see source manifest"),
                license_url=entry.get("license_url"),
                tags=guess_tags(path.name, str(entry.get("source") or ""), category),
                notes=f"Seeded from tests/corpus manifest: {entry.get('notes') or ''}",
                local_source_path=str(path),
            )
        )
    return out[:limit] if limit else out


def is_probably_pdf(data: bytes) -> bool:
    return b"%PDF" in data[:1024]


def write_atomic(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_bytes(data)
    tmp.replace(path)


def download_one(candidate: Candidate, corpus_dir: Path, timeout: int, max_bytes: int) -> dict[str, Any]:
    try:
        if candidate.local_source_path:
            data = Path(candidate.local_source_path).read_bytes()
        else:
            data = http_get(candidate.url, timeout=timeout)
        if len(data) > max_bytes:
            return {"id": candidate.id, "url": candidate.url, "ok": False, "error": f"too_large:{len(data)}"}
        if not is_probably_pdf(data):
            return {"id": candidate.id, "url": candidate.url, "ok": False, "error": "not_pdf"}
        sha = hashlib.sha256(data).hexdigest()
        rel = Path(candidate.source) / f"{candidate.id}-{sha[:12]}.pdf"
        dest = corpus_dir / rel
        if not dest.exists():
            write_atomic(dest, data)
        rec = asdict(candidate)
        rec.update(
            {
                "ok": True,
                "sha256": sha,
                "size_bytes": len(data),
                "path": str(dest.relative_to(REPO_ROOT)).replace("\\", "/"),
            }
        )
        return rec
    except Exception as err:  # noqa: BLE001
        return {"id": candidate.id, "url": candidate.url, "source": candidate.source, "ok": False, "error": str(err)[:500]}


def load_previous_entries(path: Path) -> tuple[list[dict[str, Any]], set[str]]:
    if not path.exists():
        return [], set()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return [], set()
    entries = [entry for entry in data.get("entries", []) if entry.get("ok")]
    hashes = {entry["sha256"] for entry in entries if entry.get("sha256")}
    return entries, hashes


def category_breakdown(entries: list[dict[str, Any]]) -> dict[str, int]:
    out: dict[str, int] = {}
    for entry in entries:
        for tag in entry.get("tags", [entry.get("category", "unknown")]):
            out[tag] = out.get(tag, 0) + 1
    return dict(sorted(out.items()))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-count", type=int, default=4500)
    parser.add_argument("--corpus-dir", default=str(DEFAULT_CORPUS_DIR))
    parser.add_argument("--output-manifest", default=str(DEFAULT_MANIFEST))
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--timeout", type=int, default=45)
    parser.add_argument("--max-file-mb", type=int, default=50)
    parser.add_argument("--arxiv-sleep-sec", type=float, default=10.0)
    parser.add_argument("--pdfjs-limit", type=int, default=1200)
    parser.add_argument("--verapdf-limit", type=int, default=1200)
    parser.add_argument("--safedocs-limit", type=int, default=400)
    parser.add_argument("--repo-corpus-limit", type=int, default=100)
    parser.add_argument("--no-repo-corpus", action="store_true")
    args = parser.parse_args()

    corpus_dir = Path(args.corpus_dir)
    if not corpus_dir.is_absolute():
        corpus_dir = REPO_ROOT / corpus_dir
    manifest_path = Path(args.output_manifest)
    if not manifest_path.is_absolute():
        manifest_path = REPO_ROOT / manifest_path
    corpus_dir.mkdir(parents=True, exist_ok=True)
    manifest_path.parent.mkdir(parents=True, exist_ok=True)

    candidates: list[Candidate] = []
    if not args.no_repo_corpus:
        candidates.extend(collect_repo_corpus(args.repo_corpus_limit))
    candidates.extend(
        collect_github_pdfs(
            owner="mozilla",
            repo="pdf.js",
            branch="master",
            prefix="test/pdfs/",
            source="mozilla-pdfjs",
            category="pdfjs-fixtures",
            license_name="Apache-2.0",
            license_url="https://github.com/mozilla/pdf.js/blob/master/LICENSE",
            timeout=args.timeout,
            limit=args.pdfjs_limit,
        )
    )
    candidates.extend(
        collect_github_pdfs(
            owner="veraPDF",
            repo="veraPDF-corpus",
            branch="master",
            prefix="",
            source="verapdf-corpus",
            category="pdfa-pdfua",
            license_name="CC-BY-4.0",
            license_url="https://github.com/veraPDF/veraPDF-corpus",
            timeout=args.timeout,
            limit=args.verapdf_limit,
        )
    )
    candidates.extend(
        collect_github_pdfs(
            owner="pdf-association",
            repo="safedocs",
            branch="main",
            prefix="",
            source="darpa-safedocs",
            category="safedocs-targeted",
            license_name="Apache-2.0",
            license_url="https://github.com/pdf-association/safedocs/blob/main/LICENSE",
            timeout=args.timeout,
            limit=args.safedocs_limit,
        )
    )
    remaining = max(0, args.target_count - len(candidates))
    if remaining:
        candidates.extend(collect_arxiv(remaining, args.timeout, args.arxiv_sleep_sec))

    seen_url: set[str] = set()
    unique_candidates = []
    for candidate in candidates:
        if candidate.url in seen_url:
            continue
        seen_url.add(candidate.url)
        unique_candidates.append(candidate)

    previous_entries, previous_hashes = load_previous_entries(manifest_path)
    entries = list(previous_entries)
    failures: list[dict[str, Any]] = []
    max_bytes = args.max_file_mb * 1024 * 1024
    print(f"Collected {len(unique_candidates)} candidate URLs; target={args.target_count}")
    print(f"Existing manifest entries: {len(previous_entries)}")

    def should_skip(candidate: Candidate) -> bool:
        return any(entry.get("url") == candidate.url for entry in previous_entries)

    pending = [candidate for candidate in unique_candidates if not should_skip(candidate)]
    random.Random(29).shuffle(pending)
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = [pool.submit(download_one, candidate, corpus_dir, args.timeout, max_bytes) for candidate in pending]
        for index, future in enumerate(concurrent.futures.as_completed(futures), start=1):
            record = future.result()
            if record.get("ok"):
                if record["sha256"] not in previous_hashes:
                    previous_hashes.add(record["sha256"])
                    entries.append(record)
            else:
                failures.append(record)
            if index % 25 == 0 or len(entries) >= args.target_count:
                print(f"downloaded={len(entries)} failures={len(failures)} processed={index}/{len(futures)}", flush=True)
                write_manifest(manifest_path, entries, failures, args, complete=False)
            if len(entries) >= args.target_count:
                break

    write_manifest(manifest_path, entries[: args.target_count], failures, args, complete=len(entries) >= args.target_count)
    print(f"Wrote {manifest_path}")
    print(f"Corpus entries: {min(len(entries), args.target_count)}")
    print(f"Categories/tags: {category_breakdown(entries[: args.target_count])}")
    return 0


def write_manifest(path: Path, entries: list[dict[str, Any]], failures: list[dict[str, Any]], args: argparse.Namespace, complete: bool) -> None:
    payload = {
        "version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "complete": complete,
        "target_count": args.target_count,
        "entry_count": len(entries),
        "failure_count": len(failures),
        "corpus_storage": "public-benchmark/corpus/ is gitignored; do not commit downloaded PDFs.",
        "sources": {
            "pdf_wellfriendpdf_comparable": ["veraPDF corpus", "Mozilla pdf.js test/pdfs", "DARPA SafeDocs"],
            "scale_fill": ["arXiv public API"],
            "repo_seed": "tests/corpus manifest entries are optionally copied into the ignored local corpus",
        },
        "category_breakdown": category_breakdown(entries),
        "entries": entries,
        "failures": failures[-1000:],
    }
    path.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
