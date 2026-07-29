#!/usr/bin/env python3
"""Build the deterministic wild-PDF robustness corpus manifest.

The manifest is small and tracked. PDF payloads under robustness-benchmark/corpus/
are generated or downloaded local data and stay gitignored.
"""

from __future__ import annotations

import hashlib
import json
import re
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO = Path(__file__).resolve().parents[2]
OUT = REPO / "robustness-benchmark" / "manifest.json"
GENERATED_DIR = REPO / "robustness-benchmark" / "corpus" / "generated"
TARGET_FAST_LOOP = 200


def rel(path: Path) -> str:
    return path.resolve().relative_to(REPO).as_posix()


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def load_json(path: Path) -> dict[str, Any] | None:
    if not path.exists():
        return None
    return json.loads(path.read_text(encoding="utf-8"))


def stable_id(prefix: str, raw: str) -> str:
    text = re.sub(r"[^A-Za-z0-9_.-]+", "_", raw).strip("_").lower()
    return f"{prefix}_{text}"[:120]


def add_entry(
    entries: list[dict[str, Any]],
    seen: set[str],
    *,
    path: Path,
    source_tier: str,
    origin: str,
    stress_tag: str,
    source_id: str,
    tags: list[str],
    source_url: str | None = None,
    license_name: str | None = None,
    notes: str | None = None,
    generated_by: str | None = None,
) -> bool:
    if not path.exists() or path.suffix.lower() != ".pdf":
        return False
    rpath = rel(path)
    if rpath in seen:
        return False
    seen.add(rpath)
    entries.append(
        {
            "id": stable_id(source_id, Path(rpath).stem),
            "path": rpath,
            "source_tier": source_tier,
            "origin": origin,
            "source_url": source_url,
            "license": license_name,
            "stress_tag": stress_tag,
            "tags": sorted(set(tags + [stress_tag])),
            "size_bytes": path.stat().st_size,
            "sha256": sha256(path),
            "notes": notes,
            "generated_by": generated_by,
            "fast_loop": True,
        }
    )
    return True


def stress_from_category(category: str, notes: str = "") -> str:
    text = f"{category} {notes}".lower()
    if "encrypted" in text or "protection" in text:
        return "encryption-edge"
    if "scan" in text or "image" in text:
        return "scanned-or-image"
    if "jbig2" in text or "jpeg2000" in text or "jpx" in text or "ccitt" in text:
        return "unsupported-or-rare-filter"
    if "form" in text or "acro" in text or "widget" in text:
        return "forms"
    if "cjk" in text or "rtl" in text or "unicode" in text or "font" in text:
        return "font-encoding"
    if "hostile" in text or "pathological" in text or "security" in text:
        return "pathological"
    if "pdfa" in text:
        return "spec-edge"
    if "multi-column" in text or "table" in text:
        return "layout-heavy"
    if "large" in text or "multipage" in text:
        return "large-or-multipage"
    return "real-clean"


def pdf_objects(objects: list[str]) -> bytes:
    out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for idx, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out.extend(f"{idx} 0 obj\n{body}\nendobj\n".encode("latin-1"))
    xref = len(out)
    out.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode("ascii"))
    for off in offsets[1:]:
        out.extend(f"{off:010d} 00000 n \n".encode("ascii"))
    out.extend(
        (
            f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n"
            f"startxref\n{xref}\n%%EOF\n"
        ).encode("ascii")
    )
    return bytes(out)


def write_generated(name: str, data: bytes) -> Path:
    GENERATED_DIR.mkdir(parents=True, exist_ok=True)
    path = GENERATED_DIR / name
    path.write_bytes(data)
    return path


def generate_malformed() -> list[tuple[Path, str, str]]:
    base = REPO / "tests" / "corpus" / "pdfs" / "existing" / "minimal.pdf"
    base_bytes = base.read_bytes()
    generated: list[tuple[Path, str, str]] = []

    corrupt = re.sub(rb"\n(\d{10}) 00000 n", b"\n9999999999 00000 n", base_bytes, count=2)
    generated.append(
        (
            write_generated("corrupt_xref_offsets.pdf", corrupt),
            "corrupt-xref",
            "Copied minimal.pdf and replaced the first normal xref offsets with 9999999999.",
        )
    )

    marker = base_bytes.rfind(b"startxref")
    generated.append(
        (
            write_generated("missing_startxref_trailer.pdf", base_bytes[:marker] + b"%%EOF\n"),
            "missing-startxref",
            "Copied minimal.pdf and removed the startxref offset and trailer tail.",
        )
    )

    cut = max(32, int(len(base_bytes) * 0.62))
    generated.append(
        (
            write_generated("truncated_mid_file.pdf", base_bytes[:cut]),
            "truncated",
            "Copied the first 62 percent of minimal.pdf to simulate a failed download.",
        )
    )

    generated.append(
        (
            write_generated("garbage_after_eof.pdf", base_bytes + b"\n%% local deterministic garbage after EOF\n" + (b"X" * 4096)),
            "garbage-after-eof",
            "Copied minimal.pdf and appended deterministic garbage bytes after EOF.",
        )
    )

    nested = "[" * 180 + "0" + "]" * 180
    deep_pdf = pdf_objects(
        [
            f"<< /Type /Catalog /Pages 2 0 R /OpenAction {nested} >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            "<< /Length 35 >>\nstream\nBT /F1 12 Tf 20 120 Td (deep) Tj ET\nendstream",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        ]
    )
    generated.append(
        (
            write_generated("deeply_nested_open_action.pdf", deep_pdf),
            "deep-nesting",
            "Generated a valid one-page PDF whose catalog OpenAction contains 180 nested arrays.",
        )
    )

    huge_pdf = pdf_objects(
        [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            "<< /Length 999999999 >>\nstream\nBT /F1 12 Tf 20 120 Td (huge length) Tj ET\nendstream",
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        ]
    )
    generated.append(
        (
            write_generated("huge_declared_stream_length.pdf", huge_pdf),
            "huge-declared-length",
            "Generated a one-page PDF whose content stream declares Length 999999999 with only small inline data.",
        )
    )
    return generated


def add_tests_corpus(entries: list[dict[str, Any]], seen: set[str]) -> int:
    manifest = load_json(REPO / "tests" / "corpus" / "manifest.json")
    if not manifest:
        return 0
    count = 0
    for raw in sorted(manifest.get("entries", []), key=lambda e: e.get("path", "")):
        path = REPO / raw["path"]
        category = raw.get("category") or "real-clean"
        if add_entry(
            entries,
            seen,
            path=path,
            source_tier="tier1-in-repo",
            origin="tests/corpus",
            stress_tag=stress_from_category(category, raw.get("notes") or ""),
            source_id=f"parity_{raw.get('id') or path.stem}",
            tags=[category],
            source_url=raw.get("source_url"),
            license_name=raw.get("license"),
            notes=raw.get("notes"),
        ):
            count += 1
    return count


def add_renderer(entries: list[dict[str, Any]], seen: set[str]) -> dict[str, int]:
    manifest = load_json(REPO / "renderer-benchmark" / "corpus" / "manifest.json")
    counts = {"hostile": 0, "real_world": 0}
    if not manifest:
        return counts
    raw_entries = sorted(manifest.get("entries", []), key=lambda e: (e.get("category", ""), e.get("path", "")))
    for raw in raw_entries:
        category = raw.get("category") or ""
        if not str(category).startswith("hostile-"):
            continue
        if add_entry(
            entries,
            seen,
            path=REPO / raw["path"],
            source_tier="tier1-in-repo-renderer",
            origin="renderer-benchmark hostile corpus",
            stress_tag=stress_from_category(category, raw.get("notes") or ""),
            source_id=f"renderer_{raw.get('id') or Path(raw['path']).stem}",
            tags=[category],
            license_name=raw.get("license"),
            notes=raw.get("notes"),
        ):
            counts["hostile"] += 1

    for raw in raw_entries:
        category = raw.get("category") or ""
        if not str(category).startswith("real-"):
            continue
        if counts["real_world"] >= 20:
            break
        if add_entry(
            entries,
            seen,
            path=REPO / raw["path"],
            source_tier="tier1-in-repo-renderer",
            origin="renderer-benchmark real-world corpus",
            stress_tag=stress_from_category(category, raw.get("notes") or ""),
            source_id=f"renderer_{raw.get('id') or Path(raw['path']).stem}",
            tags=[category],
            source_url=raw.get("source_url"),
            license_name=raw.get("license"),
            notes=raw.get("notes"),
        ):
            counts["real_world"] += 1
    return counts


def add_public(entries: list[dict[str, Any]], seen: set[str], target: int) -> int:
    manifest = load_json(REPO / "public-benchmark" / "manifests" / "public_corpus_manifest.json")
    if not manifest or len(entries) >= target:
        return 0
    raw_entries = [e for e in manifest.get("entries", []) if e.get("ok") and e.get("path")]
    raw_entries = sorted(raw_entries, key=lambda e: (",".join(e.get("tags") or []), e.get("path", "")))
    quotas = [
        ("safedocs-targeted", 10),
        ("pathological", 12),
        ("security", 8),
        ("pdfa", 10),
        ("pdfjs-fixtures", 12),
        ("forms", 8),
        ("encrypted", 6),
        ("scanned", 6),
        ("tables", 6),
        ("multilang", 4),
        ("arxiv-cs", 4),
    ]
    added = 0
    used_ids: set[str] = set()
    for tag, quota in quotas:
        taken = 0
        for raw in raw_entries:
            if len(entries) >= target or taken >= quota:
                break
            if raw.get("id") in used_ids:
                continue
            tags = list(raw.get("tags") or [])
            if tag != raw.get("category") and tag not in tags:
                continue
            path = REPO / raw["path"]
            if add_entry(
                entries,
                seen,
                path=path,
                source_tier="tier2-public-wild",
                origin=f"public-benchmark:{raw.get('source') or raw.get('category')}",
                stress_tag=stress_from_category(raw.get("category") or "", raw.get("notes") or " ".join(tags)),
                source_id=f"public_{raw.get('id') or path.stem}",
                tags=tags + [raw.get("category") or ""],
                source_url=raw.get("url"),
                license_name=raw.get("license"),
                notes=raw.get("notes"),
            ):
                used_ids.add(raw.get("id") or raw["path"])
                added += 1
                taken += 1
    for raw in raw_entries:
        if len(entries) >= target:
            break
        if raw.get("id") in used_ids:
            continue
        path = REPO / raw["path"]
        if add_entry(
            entries,
            seen,
            path=path,
            source_tier="tier2-public-wild",
            origin=f"public-benchmark:{raw.get('source') or raw.get('category')}",
            stress_tag=stress_from_category(raw.get("category") or "", raw.get("notes") or ""),
            source_id=f"public_{raw.get('id') or path.stem}",
            tags=list(raw.get("tags") or []) + [raw.get("category") or ""],
            source_url=raw.get("url"),
            license_name=raw.get("license"),
            notes=raw.get("notes"),
        ):
            added += 1
    return added


def main() -> int:
    entries: list[dict[str, Any]] = []
    seen: set[str] = set()
    source_counts: dict[str, Any] = {}

    source_counts["tests_corpus"] = add_tests_corpus(entries, seen)
    source_counts["renderer"] = add_renderer(entries, seen)

    for path, stress_tag, note in generate_malformed():
        add_entry(
            entries,
            seen,
            path=path,
            source_tier="tier3-generated-broken",
            origin="local malformed generator",
            stress_tag=stress_tag,
            source_id=f"generated_{path.stem}",
            tags=["malformed", stress_tag],
            license_name="project-generated",
            notes=note,
            generated_by="robustness-benchmark/scripts/build_corpus_manifest.py",
        )
    source_counts["generated_malformed"] = 6

    source_counts["public_added"] = add_public(entries, seen, TARGET_FAST_LOOP)
    entries = entries[:TARGET_FAST_LOOP]
    for idx, entry in enumerate(entries, start=1):
        entry["selection_index"] = idx

    manifest = {
        "version": 1,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "description": "Small indicative real-world robustness corpus for text-extraction survival measurement.",
        "target": {
            "fast_loop_files": TARGET_FAST_LOOP,
            "selection": "deterministic: all selected sources are sorted by path/category and truncated to the first 200 entries",
            "scale_later": "Increase TARGET_FAST_LOOP or adjust source quotas in build_corpus_manifest.py; bulk PDFs remain gitignored.",
            "label": "indicative (approx 200-file subset)",
        },
        "network_status": {
            "mozilla_pdfjs_raw": "reachable during Binding Surface HEAD probe",
            "veraPDF_corpus": "reachable during Binding Surface HEAD probe",
            "govdocs1_zip": "reachable during Binding Surface HEAD probe but not downloaded because first zip is about 486 MB",
            "local_public_benchmark": "used when present; corpus PDFs are gitignored",
        },
        "source_counts": source_counts,
        "entries": entries,
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"Wrote {OUT} with {len(entries)} entries")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
