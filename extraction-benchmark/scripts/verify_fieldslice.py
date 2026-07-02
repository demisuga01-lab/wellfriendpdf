"""Independent verification of the field-slice validation run.

Measurement only. Reuses competitive_benchmark.py's EXACT scoring functions
(field_score/pred_fields/nkey/nval/load_entries) so no scorer divergence is
possible, then:
  1. Recomputes the macro-averaged field metrics from records.jsonl and checks
     them against summary.json (catches any reporting drift).
  2. Re-runs oxide extract-fields on the SAME first-200 has-fields slice and
     produces a key-level miss breakdown to identify the dominant failure mode.

It never touches more than the 200 selected files.
"""
from __future__ import annotations
import json
import statistics
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import competitive_benchmark as cb  # noqa: E402

REPO = cb.REPO
CORPUS = REPO / "test_corpus"
OXIDE = REPO / "target" / "release" / cb.exe("oxide")
RUN_DIR = REPO / "target" / "competitive-benchmark" / "fieldslice-validation"
LIMIT = 200


def recompute_from_records():
    recs_path = RUN_DIR / "records.jsonl"
    summ_path = RUN_DIR / "summary.json"
    by_id = {}
    for line in recs_path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        by_id[r["id"]] = r
    summary = json.loads(summ_path.read_text(encoding="utf-8"))
    out = {}
    for tool in ("oxide", "pypdf"):
        rows = [r["fields"][tool] for r in by_id.values()
                if r.get("fields", {}).get(tool, {}).get("ok")]
        def mean(key):
            vs = [x[key] for x in rows if isinstance(x.get(key), (int, float))]
            return round(statistics.fmean(vs), 5) if vs else None
        out[tool] = {
            "scored": len(rows),
            "field_f1": mean("field_f1"),
            "field_recall": mean("field_recall"),
            "field_precision": mean("field_precision"),
            "field_value_f1": mean("field_value_f1"),
            "sum_truth_fields": sum(x.get("truth_fields", 0) for x in rows),
            "sum_predicted_fields": sum(x.get("predicted_fields", 0) for x in rows),
        }
    return out, summary.get("field_accuracy", {})


def run_oxide(pdf: Path) -> dict | None:
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as tf:
        outp = Path(tf.name)
    try:
        p = subprocess.run(
            [str(OXIDE), "extract-fields", str(pdf), "--format", "json",
             "--output", str(outp)],
            cwd=REPO, capture_output=True, text=True, timeout=60,
        )
        if p.returncode != 0 or not outp.exists():
            return None
        return json.loads(outp.read_text(encoding="utf-8", errors="replace"))
    except Exception:
        return None
    finally:
        try:
            outp.unlink()
        except OSError:
            pass


def failure_modes():
    entries = cb.load_entries(CORPUS, LIMIT, "has-fields")
    assert len(entries) <= 200, f"slice exceeded 200: {len(entries)}"
    per_file_f1 = []
    per_file_val_f1 = []
    per_file_prec = []
    per_file_rec = []
    strict_tp = strict_pred = strict_truth = 0
    missed_keys = Counter()          # truth key normalized -> strict-miss count
    missed_but_value_found = Counter()  # value present under another key (alias)
    missed_value_absent = Counter()  # value not present at all
    spurious_keys = Counter()        # predicted (k,v) not in truth
    zero_field_files = []
    for e in entries:
        truth = e["label"].get("fields") or {}
        payload = run_oxide(e["pdf"]) or {"fields": []}
        sc = cb.field_score(truth, payload)
        per_file_f1.append(sc["field_f1"])
        per_file_val_f1.append(sc["field_value_f1"])
        per_file_prec.append(sc["field_precision"])
        per_file_rec.append(sc["field_recall"])
        # key-level analysis using identical normalization
        t_pairs = [(cb.nkey(k), cb.nval(v)) for k, v in truth.items() if cb.nval(v)]
        p_pairs = cb.pred_fields(payload)
        if not p_pairs:
            zero_field_files.append(e["id"])
        strict_truth += len(t_pairs)
        strict_pred += len(p_pairs)
        p_set = Counter(f"{k}\0{v}" for k, v in p_pairs)
        p_vals = Counter(v for _k, v in p_pairs)
        matched = Counter()
        for k, v in t_pairs:
            key = f"{k}\0{v}"
            if p_set.get(key, 0) - matched.get(key, 0) > 0:
                matched[key] += 1
                strict_tp += 1
            else:
                missed_keys[k] += 1
                if p_vals.get(v, 0) > 0:
                    missed_but_value_found[k] += 1
                else:
                    missed_value_absent[k] += 1
        t_set = Counter(f"{k}\0{v}" for k, v in t_pairs)
        for k, v in p_pairs:
            key = f"{k}\0{v}"
            if t_set.get(key, 0) == 0:
                spurious_keys[k] += 1
    return {
        "files": len(entries),
        "macro_field_f1": round(statistics.fmean(per_file_f1), 5),
        "macro_value_f1": round(statistics.fmean(per_file_val_f1), 5),
        "macro_precision": round(statistics.fmean(per_file_prec), 5),
        "macro_recall": round(statistics.fmean(per_file_rec), 5),
        "micro_strict_tp": strict_tp,
        "micro_strict_pred": strict_pred,
        "micro_strict_truth": strict_truth,
        "micro_precision": round(strict_tp / strict_pred, 5) if strict_pred else None,
        "micro_recall": round(strict_tp / strict_truth, 5) if strict_truth else None,
        "files_zero_fields": len(zero_field_files),
        "top_missed_keys": missed_keys.most_common(12),
        "missed_value_found_under_other_key": missed_but_value_found.most_common(12),
        "missed_value_absent": missed_value_absent.most_common(12),
        "top_spurious_keys": spurious_keys.most_common(12),
    }


def main():
    print("== Recompute macro metrics from records.jsonl vs summary.json ==")
    recomputed, summary = recompute_from_records()
    for tool in ("oxide", "pypdf"):
        print(f"[{tool}] recomputed: {recomputed[tool]}")
        print(f"[{tool}] summary   : {summary.get(tool)}")
    print()
    print("== Independent oxide re-run + failure-mode breakdown (<=200 files) ==")
    fm = failure_modes()
    print(json.dumps(fm, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
