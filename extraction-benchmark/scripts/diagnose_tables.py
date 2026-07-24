"""Part A diagnostic harness for table over-detection.

Measurement + diagnosis only. Runs `wellfriendpdf extract-tables` on the SAME
deterministic first-200 has-tables slice (via cb.load_entries), reproduces the
baseline predicted-vs-truth table counts and macro metrics with cb.table_score
(the UNCHANGED scorer), then classifies every over-detected file's extra tables
into a cause histogram and computes oracle "what-if" scores that bound how much
dropping-false / merging-split tables could move shape-F1 and precision.

Crash-safe: bounded concurrency (<=4), per-file subprocess isolation via
cb.monitored (60s timeout + 2048 MB RSS cap + tree-kill + pipe cleanup),
per-record JSONL checkpoint with flush+fsync, resume-on-restart. Never touches
more than the 200 selected files.
"""
from __future__ import annotations
import json
import os
import statistics
import sys
import threading
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import competitive_benchmark as cb  # noqa: E402

CORPUS = cb.REPO / "test_corpus"
LIMIT = 200
OUT_DIR = cb.REPO / "target" / "table-diagnosis"
WORK = OUT_DIR / "work"
CKPT = OUT_DIR / "records.jsonl"
MATCH_THRESHOLD = 0.5  # min fraction of a predicted table's cells explained by a GT table

_lock = threading.Lock()


def args_for(wellfriendpdf_bin: Path) -> SimpleNamespace:
    return SimpleNamespace(
        wellfriendpdf_bin=str(wellfriendpdf_bin), timeout=60, max_memory_mb=2048,
        poll_interval_ms=50,
    )


def cell_counter(grid) -> Counter:
    c = Counter()
    for row in grid or []:
        for cell in row or []:
            n = cb.norm_line(cell)
            if n:
                c[n] += 1
    return c


def gt_grid(t) -> list:
    return ([t.get("headers") or []] if t.get("headers") else []) + (t.get("rows") or [])


def shape_str(grid) -> str:
    g = grid or []
    return f"{len(g)}x{max((len(r or []) for r in g), default=0)}" if g else ""


def containment(pred_cells: Counter, gt_cells: Counter) -> float:
    tot = sum(pred_cells.values())
    if tot == 0:
        return 0.0
    ov = sum(min(v, gt_cells.get(k, 0)) for k, v in pred_cells.items())
    return ov / tot


def table_features(rows) -> dict:
    """Structural features for a predicted grid (no GT knowledge)."""
    rows = rows or []
    n_rows = len(rows)
    n_cols = max((len(r or []) for r in rows), default=0)
    if n_rows == 0 or n_cols == 0:
        return {"populated_cols": 0, "full_cols": 0, "fill_ratio": 0.0,
                "prose_ratio": 0.0, "code_ratio": 0.0, "kv_ratio": 0.0, "data_rows": 0}
    col_pop = [0] * n_cols
    non_empty = 0
    text_cells = prose_cells = code_cells = 0
    data_rows = 0
    kv_rows = 0
    for row in rows:
        rpop = 0
        for c in range(n_cols):
            s = str(row[c]).strip() if c < len(row) else ""
            if s:
                col_pop[c] += 1
                non_empty += 1
                rpop += 1
                text_cells += 1
                words = len(s.split())
                if words >= 4 or any(p in s for p in ".!?"):
                    prose_cells += 1
                if any(ch.isdigit() for ch in s) or "-" in s or "/" in s:
                    code_cells += 1
        if rpop >= 2:
            data_rows += 1
        # key-value: exactly 2 populated cells and first ends with ':'
        pops = [str(row[c]).strip() for c in range(min(n_cols, len(row))) if str(row[c]).strip()]
        if len(pops) == 2 and pops[0].endswith(":"):
            kv_rows += 1
    populated_cols = sum(1 for v in col_pop if v > 0)
    full_cols = sum(1 for v in col_pop if v >= 0.6 * n_rows)
    return {
        "populated_cols": populated_cols,
        "full_cols": full_cols,
        "fill_ratio": round(non_empty / (n_rows * n_cols), 3),
        "prose_ratio": round(prose_cells / text_cells, 3) if text_cells else 0.0,
        "code_ratio": round(code_cells / text_cells, 3) if text_cells else 0.0,
        "kv_ratio": round(kv_rows / n_rows, 3),
        "data_rows": data_rows,
    }


def prose_like(grid) -> bool:
    """Mirror of the engine's prose heuristic, for FALSE-TABLE subtyping."""
    text_cells = prose_cells = code_cells = 0
    for row in grid or []:
        for cell in row or []:
            s = str(cell).strip()
            if not s:
                continue
            text_cells += 1
            words = len(s.split())
            if words >= 4 or any(p in s for p in ".!?"):
                prose_cells += 1
            if any(ch.isdigit() for ch in s) or "-" in s or "/" in s:
                code_cells += 1
    if text_cells == 0:
        return False
    return (prose_cells / text_cells) >= 0.45 and (code_cells / text_cells) < 0.35


def run_one(entry, wellfriendpdf_bin: Path):
    pdf = entry["pdf"]
    out = WORK / f"{entry['id']}.tables.json"
    r = cb.monitored(cb.wellfriendpdf_tables(pdf, out, args_for(wellfriendpdf_bin)), args_for(wellfriendpdf_bin))
    pred_tables = []  # list of {page, source, conf, rows, shape, ncells}
    if r.ok and out.exists():
        payload = cb.read_json(out)
        if payload:
            for pg in payload.get("pages", []):
                pnum = pg.get("page")
                for tb in pg.get("tables", []):
                    rows = tb.get("rows") or []
                    pred_tables.append({
                        "page": pnum,
                        "source": tb.get("source"),
                        "confidence": tb.get("confidence"),
                        "rows": rows,
                        "shape": shape_str(rows),
                    })
    try:
        out.unlink()
    except OSError:
        pass

    gt_tables = entry["label"].get("tables") or []
    pred_for_score = [{"rows": p["rows"]} for p in pred_tables]
    score = cb.table_score(gt_tables, pred_for_score)

    # --- Match predicted -> GT by cell containment ---
    gt_cells = [cell_counter(gt_grid(t)) for t in gt_tables]
    assign = []  # per predicted table: gt index or -1
    for p in pred_tables:
        pc = cell_counter(p["rows"])
        best, best_ov = -1, 0.0
        for gi, gc in enumerate(gt_cells):
            ov = containment(pc, gc)
            if ov > best_ov:
                best, best_ov = gi, ov
        assign.append(best if best_ov >= MATCH_THRESHOLD else -1)

    # group predicted by assigned GT
    by_gt = {}
    unmatched = []
    for idx, gi in enumerate(assign):
        if gi < 0:
            unmatched.append(idx)
        else:
            by_gt.setdefault(gi, []).append(idx)

    # cause counts for EXTRA tables on this file
    causes = Counter()
    false_ruled = false_borderless = 0
    for idx in unmatched:
        p = pred_tables[idx]
        if prose_like(p["rows"]):
            causes["false-table-prose"] += 1
        else:
            causes["false-table-other"] += 1
        if p["source"] == "ruled":
            false_ruled += 1
        else:
            false_borderless += 1
    for gi, idxs in by_gt.items():
        if len(idxs) <= 1:
            continue
        pages = {pred_tables[i]["page"] for i in idxs}
        extra = len(idxs) - 1
        if len(pages) == len(idxs):
            causes["split-cross-page"] += extra
        elif len(pages) > 1:
            causes["split-mixed"] += extra
        else:
            causes["split-same-page"] += extra

    # --- Oracle what-ifs (bound the opportunity; NOT a fix) ---
    # drop-false: remove unmatched predicted tables
    keep = [pred_tables[i] for i in range(len(pred_tables)) if assign[i] >= 0]
    score_dropfalse = cb.table_score(gt_tables, [{"rows": p["rows"]} for p in keep])
    # merge-splits: for each GT with >1 assigned, stack their rows into one table
    merged = []
    used = set()
    for gi, idxs in by_gt.items():
        if len(idxs) > 1:
            stacked = []
            for i in sorted(idxs, key=lambda k: (pred_tables[k]["page"] or 0)):
                stacked.extend(pred_tables[i]["rows"])
                used.add(i)
            merged.append({"rows": stacked})
    for i in range(len(pred_tables)):
        if i not in used and assign[i] >= 0:
            merged.append({"rows": pred_tables[i]["rows"]})
    # both: drop false + merge splits (merged already excludes unmatched)
    score_both = cb.table_score(gt_tables, merged)

    return {
        "id": entry["id"],
        "ok": r.ok,
        "failure_kind": r.kind(),
        "page_count": entry["label"].get("page_count"),
        "truth_tables": len(gt_tables),
        "predicted_tables": len(pred_tables),
        "unmatched": len(unmatched),
        "false_ruled": false_ruled,
        "false_borderless": false_borderless,
        "causes": dict(causes),
        "sources": dict(Counter(p["source"] for p in pred_tables)),
        "score": score,
        "score_dropfalse": score_dropfalse,
        "score_both": score_both,
        "gt_shapes": dict(Counter(shape_str(gt_grid(t)) for t in gt_tables)),
        "pred_shapes": dict(Counter(p["shape"] for p in pred_tables)),
        "pred_detail": [
            {"page": p["page"], "source": p["source"], "shape": p["shape"],
             "confidence": p["confidence"], "assigned_gt": assign[i],
             "feat": table_features(p["rows"])}
            for i, p in enumerate(pred_tables)
        ],
    }


def load_done():
    done = {}
    if CKPT.exists():
        for line in CKPT.read_text(encoding="utf-8").splitlines():
            if line.strip():
                rec = json.loads(line)
                done[rec["id"]] = rec
    return done


def append_ckpt(rec):
    with _lock:
        with open(CKPT, "a", encoding="utf-8") as f:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")
            f.flush()
            os.fsync(f.fileno())


def main():
    fresh = "--resume" not in sys.argv
    wellfriendpdf_bin = cb.REPO / "target" / "release" / cb.exe("wellfriendpdf")
    if not wellfriendpdf_bin.exists():
        print(f"FATAL: missing {wellfriendpdf_bin}", file=sys.stderr)
        sys.exit(2)
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    WORK.mkdir(parents=True, exist_ok=True)
    if fresh and CKPT.exists():
        CKPT.unlink()

    entries = cb.load_entries(CORPUS, LIMIT, "has-tables")
    assert len(entries) <= 200, f"slice exceeded 200: {len(entries)}"
    (OUT_DIR / "slice_files.json").write_text(
        json.dumps([e["id"] for e in entries], indent=0), encoding="utf-8")

    done = load_done()
    todo = [e for e in entries if e["id"] not in done]
    print(f"slice={len(entries)} done={len(done)} todo={len(todo)} bin={wellfriendpdf_bin.name}")

    results = list(done.values())
    with ThreadPoolExecutor(max_workers=4) as ex:
        futs = {ex.submit(run_one, e, wellfriendpdf_bin): e["id"] for e in todo}
        for i, fut in enumerate(as_completed(futs), 1):
            rec = fut.result()
            append_ckpt(rec)
            results.append(rec)
            if i % 25 == 0 or i == len(todo):
                print(f"  ...{i}/{len(todo)}")

    results = [r for r in results if r["id"] in {e["id"] for e in entries}]
    scored = [r for r in results if r.get("ok")]
    print(f"\nscored {len(scored)}/{len(entries)} (failures: "
          f"{[(r['id'], r['failure_kind']) for r in results if not r.get('ok')]})")

    def macro(key, sub="score"):
        vs = [r[sub][key] for r in scored]
        return round(statistics.fmean(vs), 5) if vs else None

    sum_pred = sum(r["predicted_tables"] for r in scored)
    sum_truth = sum(r["truth_tables"] for r in scored)
    over = sum(1 for r in scored if r["predicted_tables"] > r["truth_tables"])
    under = sum(1 for r in scored if r["predicted_tables"] < r["truth_tables"])
    exact = sum(1 for r in scored if r["predicted_tables"] == r["truth_tables"])

    print("\n==== BASELINE (indicative, <=200-file table slice; scorer UNCHANGED) ====")
    print(f"cell-F1={macro('table_cell_f1')} recall={macro('table_cell_recall')} "
          f"precision={macro('table_cell_precision')} "
          f"TEDS={macro('table_teds_approx')} shape-F1={macro('table_shape_f1')}")
    print(f"predicted_tables={sum_pred} truth_tables={sum_truth} "
          f"(ratio {sum_pred / max(1, sum_truth):.3f})")
    print(f"files over={over} under={under} exact={exact}")

    print("\n==== OVER-DETECTION CAUSE HISTOGRAM (extra tables across over-detected files) ====")
    total_causes = Counter()
    for r in scored:
        total_causes.update(r["causes"])
    for cause, n in total_causes.most_common():
        print(f"  {cause:24s} {n}")
    print(f"  {'TOTAL extra classified':24s} {sum(total_causes.values())}")
    print(f"  (raw extra = predicted-truth = {sum_pred - sum_truth})")

    print("\n==== ORACLE WHAT-IF (bounds only; uses GT to match; NOT the fix) ====")
    print(f"drop-false : shape-F1={macro('table_shape_f1','score_dropfalse')} "
          f"precision={macro('table_cell_precision','score_dropfalse')} "
          f"recall={macro('table_cell_recall','score_dropfalse')} "
          f"cell-F1={macro('table_cell_f1','score_dropfalse')} "
          f"TEDS={macro('table_teds_approx','score_dropfalse')}")
    print(f"drop+merge : shape-F1={macro('table_shape_f1','score_both')} "
          f"precision={macro('table_cell_precision','score_both')} "
          f"recall={macro('table_cell_recall','score_both')} "
          f"cell-F1={macro('table_cell_f1','score_both')} "
          f"TEDS={macro('table_teds_approx','score_both')}")

    # Worst over-detected files
    worst = sorted(scored, key=lambda r: r["predicted_tables"] - r["truth_tables"], reverse=True)[:15]
    print("\n==== WORST OVER-DETECTED FILES (top 15) ====")
    for r in worst:
        print(f"  {r['id']} pages={r['page_count']:>3} truth={r['truth_tables']:>2} "
              f"pred={r['predicted_tables']:>3} unmatched={r['unmatched']:>2} "
              f"sources={r['sources']} causes={r['causes']}")

    (OUT_DIR / "summary.json").write_text(json.dumps({
        "slice": len(entries), "scored": len(scored),
        "macro": {k: macro(k) for k in ("table_cell_f1", "table_cell_recall",
                                        "table_cell_precision", "table_teds_approx",
                                        "table_shape_f1")},
        "sum_predicted_tables": sum_pred, "sum_truth_tables": sum_truth,
        "files_over": over, "files_under": under, "files_exact": exact,
        "causes": dict(total_causes),
        "oracle_dropfalse": {k: macro(k, "score_dropfalse") for k in
                             ("table_shape_f1", "table_cell_precision", "table_cell_recall",
                              "table_cell_f1", "table_teds_approx")},
        "oracle_both": {k: macro(k, "score_both") for k in
                        ("table_shape_f1", "table_cell_precision", "table_cell_recall",
                         "table_cell_f1", "table_teds_approx")},
    }, indent=2), encoding="utf-8")
    print(f"\nwrote {OUT_DIR / 'summary.json'}")


if __name__ == "__main__":
    main()
