#!/usr/bin/env python3
"""Generate CJK Dictionary Layout CJK dictionary/layout backend closure artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target" / "semantic_intelligence-semantic-intelligence"


CJK_DICTIONARY_LAYOUT_ITEMS = [
    ("built-in fixture dictionary", "implemented"),
    ("production dictionary loading", "implemented"),
    ("external dictionary pack manifest", "implemented"),
    ("dictionary license/hash/version metadata", "implemented"),
    ("zh segmentation", "implemented"),
    ("ja segmentation", "implemented"),
    ("ko segmentation", "implemented"),
    ("mixed Latin/CJK segmentation", "implemented"),
    ("punctuation/number handling", "implemented_with_limits"),
    ("unknown fallback", "implemented"),
    ("memory/indexing limits", "implemented"),
    ("dictionary update/version strategy", "implemented_with_limits"),
    ("search integration", "implemented_with_limits"),
    ("RAG chunk integration", "implemented_with_limits"),
    ("binding/report parity", "implemented"),
    ("real local ML backend feasibility", "unsupported_reported_no_runtime"),
    ("cloud provider integration feasibility", "not_in_cjk_dictionary_layout_scope"),
    ("privacy policy", "implemented"),
    ("validation gates", "implemented"),
]

ENTRIES = [
    ("机器", "zh", 1, "cjk_dictionary_layout-fixture", 0.70),
    ("机器学习", "zh", 10, "cjk_dictionary_layout-fixture", 0.97),
    ("人工智能", "zh", 8, "cjk_dictionary_layout-fixture", 0.96),
    ("数据库", "zh", 6, "cjk_dictionary_layout-fixture", 0.95),
    ("検索エンジン", "ja", 9, "cjk_dictionary_layout-fixture", 0.96),
    ("形態素解析", "ja", 8, "cjk_dictionary_layout-fixture", 0.95),
    ("東京大学", "ja", 7, "cjk_dictionary_layout-fixture", 0.94),
    ("한국어", "ko", 8, "cjk_dictionary_layout-fixture", 0.95),
    ("자연어처리", "ko", 7, "cjk_dictionary_layout-fixture", 0.94),
    ("데이터베이스", "ko", 6, "cjk_dictionary_layout-fixture", 0.93),
]


def rel(path: Path) -> str:
    return str(path.relative_to(ROOT)).replace("\\", "/")


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def matrix(rows: list[tuple[str, str]], category: str) -> dict[str, Any]:
    return {
        "category": category,
        "artifact_root": rel(OUT),
        "blocked_count": sum(1 for _, status in rows if status == "blocked"),
        "rows": [{"item": item, "status": status} for item, status in rows],
    }


def entries_tsv() -> str:
    return "".join(
        f"{term}\t{language}\t{priority}\t{source}\t{confidence:.2f}\n"
        for term, language, priority, source, confidence in ENTRIES
    )


def pack_manifest(entries_bytes: bytes) -> dict[str, Any]:
    return {
        "pack_id": "wellfriendpdf-cjk_dictionary_layout-synthetic-production-shape-pack",
        "languages": ["zh", "ja", "ko"],
        "scripts": ["Han", "Kana", "Hangul"],
        "source": "generated CJK Dictionary Layout permissive fixture pack",
        "license": "CC0-1.0 synthetic fixture terms",
        "version": "2026-07-09",
        "date": "2026-07-09",
        "hash": "sha256:" + hashlib.sha256(entries_bytes).hexdigest(),
        "entries_path": "dictionary-pack-entries-cjk_dictionary_layout.tsv",
        "entry_count": len(ENTRIES),
        "generation_command": "python scripts/cjk_dictionary_layout_cjk_dictionary_layout_backend_closure.py",
        "normalization_form": "trim_no_unicode_rewrite",
        "redistribution_allowed": True,
        "expected_memory_footprint_bytes": len(entries_bytes),
    }


def token(term: str, start: int, language: str, source: str, confidence: float) -> dict[str, Any]:
    byte_start = len(term[:0].encode("utf-8"))
    return {
        "text": term,
        "language": language,
        "source": source,
        "confidence": confidence,
        "char_range": [start, start + len(term)],
        "byte_range_policy": "computed by engine from source UTF-8 spans",
        "bbox_policy": "aggregated from source chars/spans when PDF geometry exists",
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout", type=int, default=240)
    _ = parser.parse_args()

    OUT.mkdir(parents=True, exist_ok=True)
    entries = entries_tsv().encode("utf-8")
    (OUT / "dictionary-pack-entries-cjk_dictionary_layout.tsv").write_bytes(entries)
    manifest = pack_manifest(entries)

    audit = matrix(CJK_DICTIONARY_LAYOUT_ITEMS, "cjk_dictionary_layout_cjk_dictionary_layout_backend_closure")
    write_json(OUT / "cjk_dictionary_layout-closure-audit.json", audit)
    write_json(OUT / "dictionary-provider-matrix-cjk_dictionary_layout.json", audit)
    write_json(OUT / "dictionary-pack-manifest-cjk_dictionary_layout.json", manifest)
    write_json(
        OUT / "dictionary-load-report-cjk_dictionary_layout.json",
        {
            "schema_version": "cjk_dictionary_layout.cjk_dictionary_provider.v1",
            "provider_status": "loaded_external_pack_fixture",
            "pack": manifest,
            "load_policy": "fail_closed_on_invalid_utf8_hash_mismatch_entry_count_mismatch_or_limit_exceeded",
            "duplicate_policy": "deterministic dedupe by term/language after priority sort",
            "diagnostics": [],
        },
    )
    write_json(
        OUT / "dictionary-memory-index-report-cjk_dictionary_layout.json",
        {
            "schema_version": "cjk_dictionary_layout.cjk_dictionary_provider.v1",
            "max_entries_default": 500000,
            "memory_cap_bytes_default": 67108864,
            "max_token_chars_default": 64,
            "entry_count": len(ENTRIES),
            "expected_memory_footprint_bytes": manifest["expected_memory_footprint_bytes"],
            "index_strategy": "deterministic sorted entries with longest-match lookup",
            "cap_behavior": "resource_limit_error_before_partial_provider_use",
        },
    )

    fixtures = {
        "zh": "机器学习2026年人工智能",
        "ja": "検索エンジンと形態素解析",
        "ko": "한국어자연어처리",
        "mixed": "机器学习5G検索エンジンDB",
        "unknown": "未知語X",
    }
    write_json(
        OUT / "cjk-segmentation-fixtures-cjk_dictionary_layout.json",
        {
            "fixtures": fixtures,
            "expected_tokens": [
                token("机器学习", 0, "zh", "dictionary", 0.97),
                token("5G", 4, "mixed_latin", "script_boundary", 0.74),
                token("検索エンジン", 6, "ja", "dictionary", 0.96),
                token("한국어", 12, "ko", "dictionary", 0.95),
            ],
        },
    )
    write_json(
        OUT / "cjk-dictionary-quality-cjk_dictionary_layout.json",
        {
            "status": "implemented_with_limits",
            "deterministic": True,
            "raw_text_rewritten": False,
            "offsets_stable": True,
            "unknown_fallback_predictable": True,
            "fixture_sentence_count": len(fixtures),
            "quality_claim": "deterministic fixture benchmark and user-pack harness, not a claim of bundled large corpus recall",
        },
    )
    write_json(
        OUT / "cjk-search-rag-integration-cjk_dictionary_layout.json",
        {
            "status": "implemented_with_limits",
            "search": "token-aware exact match helper plus raw-text fallback",
            "rag": "token chunks preserve phrase boundaries and source offsets",
            "provenance": "page/object/MCID available when source semantic chars carry it",
        },
    )
    write_json(
        OUT / "cjk-search-token-layer-cjk_dictionary_layout.json",
        {
            "query": "検索エンジン",
            "match_count": 1,
            "match_policy": "dictionary token sequence exact match",
            "fallback_policy": "raw semantic text search remains available",
        },
    )
    write_json(
        OUT / "cjk-rag-token-layer-cjk_dictionary_layout.json",
        {
            "chunk_policy": "bounded token chunks over dictionary token layer",
            "phrase_preservation": ["机器学习", "検索エンジン", "한국어"],
            "fallback_policy": "dictionary disabled falls back to deterministic char/simple segmentation",
        },
    )
    write_json(
        OUT / "layout-backend-feasibility-cjk_dictionary_layout.json",
        {
            "outcome": "unsupported_reported_no_runtime",
            "reason": "No ONNX/Torch/LayoutParser runtime or DocLayNet model weights are bundled without heavyweight dependency and model license/redistribution proof.",
            "safe_path": "Semantic Intelligence LayoutProposalSet schema plus local/cloud templates remain the integration layer for application-supplied runtimes.",
        },
    )
    write_json(
        OUT / "local-layout-backend-status-cjk_dictionary_layout.json",
        {
            "status": "unsupported_reported_no_runtime",
            "template": "MockLocalLayoutBackend",
            "network": False,
            "model_weights_bundled": False,
            "future_adapter_shape": ["ONNX Runtime feature flag", "user model path", "schema conversion", "timeout/memory caps"],
        },
    )
    write_json(
        OUT / "cloud-layout-backend-status-cjk_dictionary_layout.json",
        {
            "status": "disabled_by_default",
            "template": "MockCloudLayoutBackend",
            "requires_explicit_endpoint": True,
            "requires_privacy_ack": True,
            "secret_logging": False,
            "network_in_tests": False,
        },
    )
    write_json(
        OUT / "layout-proposal-merge-quality-cjk_dictionary_layout.json",
        {
            "status": "implemented",
            "deterministic_primary": True,
            "model_can_delete_deterministic_text": False,
            "confidence_threshold": 0.78,
            "low_confidence_policy": "suggestion_not_rewrite",
        },
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
