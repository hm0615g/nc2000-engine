#!/usr/bin/env python3
"""All-pairs Round-Robin scheduler and symmetric Nash matrix analyzer.

Supports both singleton teams (`s:<id>` from a Swiss pools.json) and
arbitrary disclosed multi-variant shell mixtures (`mix:<name>`).
Each (i, j, rep) seat pair uses strictly independent SHA-256 seeds.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import defaultdict
from importlib.machinery import SourceFileLoader
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
STATS = SourceFileLoader("nash_stats", str(ROOT / "tools/nash-stats.py")).load_module()


def u32_hash(tag: str) -> int:
    return int.from_bytes(hashlib.sha256(tag.encode("utf-8")).digest()[:4], "big")


def seat_score(outcome: str, seat: int) -> float:
    if outcome in ("tie", "cap"):
        return 0.5
    win_seat = 0 if outcome == "p1" else 1
    return 1.0 if win_seat == seat else 0.0


def cmd_prepare(args: argparse.Namespace) -> None:
    pools_doc = json.loads(args.pools.read_text())
    mixtures = pools_doc["mixtures"]
    names = [s.strip() for s in args.mixtures.split(",") if s.strip()]
    for name in names:
        assert name in mixtures, f"mixture {name!r} not found in {args.pools}"
        members = mixtures[name]
        assert len(members) == 1, f"prepare currently expects singleton mixtures or pass --allow-multi; got {name} ({len(members)})"

    args.out_dir.mkdir(parents=True, exist_ok=True)
    sched: list[dict[str, Any]] = []
    blk = 0
    for i in range(len(names)):
        for j in range(i + 1, len(names)):
            ma, mb = names[i], names[j]
            ida = mixtures[ma][0]["id"]
            idb = mixtures[mb][0]["id"]
            for rep in range(args.pairs):
                tag = f"nc2000/rr/{args.seed_tag}/{ma}/{mb}/r{rep}"
                sa = u32_hash(f"{tag}/a")
                sb = u32_hash(f"{tag}/b")
                bs = u32_hash(f"{tag}/battle")
                for row_seat in (0, 1):
                    if row_seat == 0:
                        ids, seeds, mix = [ida, idb], [sa, sb], [ma, mb]
                    else:
                        ids, seeds, mix = [idb, ida], [sb, sa], [mb, ma]
                    sched.append({
                        "block": blk,
                        "cell": {"a": ma, "b": mb, "rep": rep},
                        "rowSeat": row_seat,
                        "ids": ids,
                        "seeds": seeds,
                        "battleSeed": bs,
                        "mixtures": mix,
                    })
                blk += 1

    with (args.out_dir / "schedule.jsonl").open("w") as fh:
        for row in sched:
            fh.write(json.dumps(row) + "\n")
    meta = {
        "pools": str(args.pools),
        "seed_tag": args.seed_tag,
        "pairs_per_matchup": args.pairs,
        "mixtures": names,
        "matchups": len(names) * (len(names) - 1) // 2,
        "total_games": len(sched),
    }
    (args.out_dir / "meta.json").write_text(json.dumps(meta, indent=2))
    print(json.dumps({
        "out_dir": str(args.out_dir),
        "n_teams": len(names),
        "matchups": meta["matchups"],
        "pairs_per_matchup": args.pairs,
        "total_games": len(sched),
    }))


def cmd_analyze(args: argparse.Namespace) -> None:
    meta = json.loads((args.dir / "meta.json").read_text())
    names: list[str] = meta["mixtures"]
    idx = {m: i for i, m in enumerate(names)}
    n = len(names)

    pair_scores: dict[tuple[str, str, int], list[float]] = defaultdict(list)
    total_games = 0
    for rdir in args.result_dirs:
        for p in sorted(rdir.glob("*.json")):
            if p.name == "manifest.json":
                continue
            r = json.loads(p.read_text())
            total_games += 1
            cell = r["cell"]
            key = (cell["a"], cell["b"], int(r["block"]))
            pair_scores[key].append(seat_score(r["outcome"], int(r["rowSeat"])))

    cell_pairs: dict[tuple[int, int], list[float]] = defaultdict(list)
    for (ma, mb, _blk), scs in pair_scores.items():
        if len(scs) == 2 and ma in idx and mb in idx:
            ps = 0.5 * (scs[0] + scs[1])
            ia, ib = idx[ma], idx[mb]
            cell_pairs[(ia, ib)].append(ps)
            cell_pairs[(ib, ia)].append(1.0 - ps)

    M = [[0.5] * n for _ in range(n)]
    SE = [[0.0] * n for _ in range(n)]
    NP = [[0] * n for _ in range(n)]
    for i in range(n):
        for j in range(n):
            if i == j:
                continue
            xs = cell_pairs[(i, j)]
            NP[i][j] = len(xs)
            if xs:
                m = sum(xs) / len(xs)
                v = sum((x - m) ** 2 for x in xs) / (len(xs) - 1) if len(xs) > 1 else 0.25
                M[i][j] = m
                SE[i][j] = math.sqrt(max(v, 0.01) / len(xs))
            else:
                SE[i][j] = 0.25

    audit = STATS.nash_uncertainty_audit(names, M, SE, boot=args.boot, seed=20261120)
    # Also compute mean win rate vs field and worst matchup for each team
    rows = []
    for i, name in enumerate(names):
        sh = audit["shells"][name]
        opp_scores = [M[i][j] for j in range(n) if j != i]
        worst_j = min((j for j in range(n) if j != i), key=lambda j: M[i][j])
        best_j = max((j for j in range(n) if j != i), key=lambda j: M[i][j])
        rows.append({
            "name": name,
            "nash_weight": sh["weight"],
            "boot_p10": sh["p10"],
            "boot_med": sh["median"],
            "boot_p90": sh["p90"],
            "ev_vs_nash": sh["ev_vs_eq"],
            "mean_vs_field": round(sum(opp_scores) / len(opp_scores), 4),
            "worst_opp": names[worst_j],
            "worst_score": round(M[i][worst_j], 4),
            "best_opp": names[best_j],
            "best_score": round(M[i][best_j], 4),
        })
    rows.sort(key=lambda r: (-r["nash_weight"], -r["ev_vs_nash"], -r["mean_vs_field"]))

    report = {
        "total_games": total_games,
        "expected_games": meta["total_games"],
        "names": names,
        "matrix": [[round(v, 4) for v in row] for row in M],
        "se_matrix": [[round(v, 4) for v in row] for row in SE],
        "pairs_matrix": NP,
        "ranked": rows,
        "next_cells": audit["next_cells"][:20],
    }
    out_path = args.out or (args.dir / "rr-report.json")
    out_path.write_text(json.dumps(report, indent=2))
    print(json.dumps({
        "report": str(out_path),
        "games": total_games,
        "support_ge_1pct": [
            {"name": r["name"], "w": r["nash_weight"], "p10": r["boot_p10"], "p90": r["boot_p90"], "field": r["mean_vs_field"]}
            for r in rows if r["nash_weight"] >= 0.01
        ],
        "top_ev": [{"name": r["name"], "ev": r["ev_vs_nash"], "field": r["mean_vs_field"]} for r in rows[:10]],
    }))


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_prep = sub.add_parser("prepare")
    p_prep.add_argument("--pools", type=Path, required=True)
    p_prep.add_argument("--mixtures", type=str, required=True)
    p_prep.add_argument("--pairs", type=int, default=8)
    p_prep.add_argument("--seed-tag", type=str, required=True)
    p_prep.add_argument("--out-dir", type=Path, required=True)
    p_prep.set_defaults(func=cmd_prepare)

    p_an = sub.add_parser("analyze")
    p_an.add_argument("--dir", type=Path, required=True)
    p_an.add_argument("--boot", type=int, default=400)
    p_an.add_argument("--out", type=Path, default=None)
    p_an.add_argument("result_dirs", nargs="+", type=Path)
    p_an.set_defaults(func=cmd_analyze)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
