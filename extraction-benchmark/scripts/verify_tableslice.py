"""Independent verification of the table-slice validation run.

Measurement only. Reuses competitive_benchmark.py's EXACT table_score so no
scorer divergence is possible, then:
  1. Recomputes macro metrics from records.jsonl for every tool and checks them
     against summary.json (catches reporting drift).
  2. Aggregates structural detail from the per-file records: macro shape-F1,
     pooled/micro cell precision & recall, table over/under-detection counts.
  3. Re-runs oxide extract-tables on the SAME first-200 has-tables slice and
     re-scores with table_score() to reproduce the oxide aggregate a second way.

Never touches more than the 200 selected files.
"""
from __future__ import annotations
import json
import statistics
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import competitive_benchmark as cb  # noqa: E402

REPO = cb.REPO
CORPUS = REPO / "test_corpus"
OXIDE = REPO / "target" / "release" / cb.exe("oxide")
RUN_DIR = REPO / "target" / "competitive-benchmark" / "tableslice-validation"
LIMIT = 200
TOOLS = ("oxide", "pymupdf", "pdfplumber")


def load_records():
    by_id = {}
    for line in (RUN_DIR / "records.jsonl").read_text(encoding="utf-8").splitlines():
        if line.strip():
            r = json.loads(line)
            by_id[r["id"]] = r
    return by_id


def recompute_and_struct():
    by_id = load_records()
    summary = json.loads((RUN_DIR / "summary.json").read_text(encoding="utf-8"))
    out = {}
    for tool in TOOLS:
        rows = [r["tables"][tool] for r in by_id.values()
                if r.get("tables", {}).get(tool, {}).get("ok")]

        def mean(key):
            vs = [x[key] for x in rows if isinstance(x.get(key), (int, float))]
            return round(statistics.fmean(vs), 5) if vs else None

        sum_tt = sum(x.get("truth_tables", 0) for x in rows)
        sum_pt = sum(x.get("predicted_tables", 0) for x in rows)
        sum_tc = sum(x.get("truth_cells", 0) for x in rows)
        sum_pc = sum(x.get("predicted_cells", 0) for x in rows)
        over = sum(1 for x in rows if x.get("predicted_tables", 0) > x.get("truth_tables", 0))
        under = sum(1 for x in rows if x.get("predicted_tables", 0) < x.get("truth_tables", 0))
        exact = sum(1 for x in rows if x.get("predicted_tables", 0) == x.get("truth_tables", 0))
        prec_below_1 = sum(1 for x in rows if isinstance(x.get("table_cell_precision"), (int, float)) and x["table_cell_precision"] < 0.999)
        out[tool] = {
            "scored": len(rows),
            "macro_cell_f1": mean("table_cell_f1"),
            "macro_cell_recall": mean("table_cell_recall"),
            "macro_cell_precision": mean("table_cell_precision"),
            "macro_shape_f1": mean("table_shape_f1"),
            "macro_teds_approx": mean("table_teds_approx"),
            "sum_truth_tables": sum_tt,
            "sum_predicted_tables": sum_pt,
            "sum_truth_cells": sum_tc,
            "sum_predicted_cells": sum_pc,
            "files_over_detect_tables": over,
            "files_under_detect_tables": under,
            "files_exact_table_count": exact,
            "files_cell_precision_below_1": prec_below_1,
        }
    return out, summary.get("table_accuracy", {})


def run_oxide_tables(pdf: Path):
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as tf:
        outp = Path(tf.name)
    try:
        p = subprocess.run(
            [str(OXIDE), "extract-tables", str(pdf), "--format", "json",
             "--output", str(outp)],
            cwd=REPO, capture_output=True, text=True, timeout=60,
        )
        if p.returncode != 0 or not outp.exists():
            return None
        payload = json.loads(outp.read_text(encoding="utf-8", errors="replace"))
        pred = []
        for pg in payload.get("pages", []):
            for tb in pg.get("tables", []):
                pred.append({"rows": tb.get("rows") or []})
        return pred
    except Exception:
        return None
    finally:
        try:
            outp.unlink()
        except OSError:
            pass


def independent_oxide_rescore():
    entries = cb.load_entries(CORPUS, LIMIT, "has-tables")
    assert len(entries) <= 200, f"slice exceeded 200: {len(entries)}"
    f1, rec, prec, shape, teds = [], [], [], [], []
    for e in entries:
        pred = run_oxide_tables(e["pdf"]) or []
        sc = cb.table_score(e["label"].get("tables") or [], pred)
        f1.append(sc["table_cell_f1"]); rec.append(sc["table_cell_recall"])
        prec.append(sc["table_cell_precision"]); shape.append(sc["table_shape_f1"])
        teds.append(sc["table_teds_approx"])
    return {
        "files": len(entries),
        "macro_cell_f1": round(statistics.fmean(f1), 5),
        "macro_cell_recall": round(statistics.fmean(rec), 5),
        "macro_cell_precision": round(statistics.fmean(prec), 5),
        "macro_shape_f1": round(statistics.fmean(shape), 5),
        "macro_teds_approx": round(statistics.fmean(teds), 5),
    }


def main():
    print("== Recompute macro metrics + structural detail from records.jsonl ==")
    recomputed, summary = recompute_and_struct()
    for tool in TOOLS:
        print(f"[{tool}] recomputed: {json.dumps(recomputed[tool])}")
        print(f"[{tool}] summary   : {json.dumps(summary.get(tool))}")
    print()
    print("== Independent oxide extract-tables re-run + re-score (<=200 files) ==")
    print(json.dumps(independent_oxide_rescore(), indent=2))


if __name__ == "__main__":
    main()
