#!/usr/bin/env python3
"""Multi-fidelity challenger screening & sequential production evaluator.

Builds deterministic, seat-swapped, stratified schedules for a set of
single-shell (pure or camouflaged) challengers against a disclosed defender
pool, runs them via native `nash_refresh` (or exports shards for Colab), and
computes point estimates + always-valid / stratified upper bounds.

Usage examples:
  # Build a pools + schedule bundle for all distinct shells in belief-pool-v3
  python3 tools/nash-screen.py prepare \
      --defender data/nash-camouflage-v1/conclusion-candidate.json \
      --catalog data/belief-pool-v3/belief-pool.json \
      --pairs 4 --seed-tag tierA-s1 --out-dir target/tierA-s1

  # Run locally at a calibrated surrogate budget (or omit budget flags for prod 243k/729k)
  python3 tools/nash-screen.py run \
      --dir target/tierA-s1 --workers 10 --preview-iters 27000 --battle-iters 9000 --battle-mode fixed

  # Summarize & rank challengers with upper bounds (target <= 55%)
  python3 tools/nash-screen.py summarize --dir target/tierA-s1 --top 25
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from collections import defaultdict
from importlib.machinery import SourceFileLoader
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
STATS = SourceFileLoader("nash_stats", str(ROOT / "tools/nash-stats.py")).load_module()

BASE_TEAM_FILES = [
    "kodoku-beq-v1/novel_evolved_teams",
    "team-pool-v2/team-pool",
    "kodoku-r3/conclusion-equilibrium-deme",
]


def u32_hash(tag: str) -> int:
    return int.from_bytes(hashlib.sha256(tag.encode("utf-8")).digest()[:4], "big")


def shell_signature(sets: list[dict[str, Any]]) -> tuple[tuple[Any, ...], ...]:
    return tuple(
        (m["species"], int(m.get("level", 50)), str(m.get("gender", "")), bool(m.get("item")))
        for m in sets
    )


def load_base_team_ids() -> set[str]:
    ids: set[str] = set()
    for name in BASE_TEAM_FILES:
        d = json.loads((ROOT / "data" / f"{name}.json").read_text())
        for t in d["teams"]:
            ids.add(t["id"])
    return ids


def allocate_stratum_counts(weights: list[float], total_pairs: int) -> list[int]:
    """Deterministic proportional allocation (every positive weight gets >= 1 when total_pairs >= len)."""
    k = len(weights)
    if total_pairs >= k:
        counts = [1] * k
        rem = total_pairs - k
    else:
        counts = [0] * k
        rem = total_pairs
    raw = [rem * w for w in weights]
    floors = [int(r) for r in raw]
    for i in range(k):
        counts[i] += floors[i]
    leftover = rem - sum(floors)
    order = sorted(range(k), key=lambda i: (raw[i] - floors[i], weights[i]), reverse=True)
    for i in order[:leftover]:
        counts[i] += 1
    return counts


def cmd_prepare(args: argparse.Namespace) -> None:
    base_ids = load_base_team_ids()
    defender_doc = json.loads(args.defender.read_text())
    def_teams = defender_doc["teams"]
    def_total_w = sum(float(t["weight"]) for t in def_teams)
    def_members = [{"id": t["id"], "weight": float(t["weight"]) / def_total_w} for t in def_teams]

    extra_teams: dict[str, list[dict[str, Any]]] = {}
    for t in def_teams:
        if t["id"] not in base_ids:
            extra_teams[t["id"]] = t["sets"]

    # Load challengers from catalog, deduplicated by exact canonical set hash
    # (never dropping distinct variants that share a public shell).
    cat_doc = json.loads(args.catalog.read_text())
    cat_teams = cat_doc["teams"]
    challengers: list[tuple[str, list[dict[str, Any]]]] = []
    seen_sets: set[str] = set()
    id_filter = set(args.ids.split(",")) if args.ids else None

    for t in cat_teams:
        tid = t["id"]
        if id_filter is not None and tid not in id_filter:
            continue
        set_hash = hashlib.sha256(
            json.dumps(t["sets"], sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).hexdigest()
        if not args.all_variants and set_hash in seen_sets:
            continue
        seen_sets.add(set_hash)
        if tid not in base_ids:
            extra_teams[tid] = t["sets"]
        challengers.append((tid, [{"id": tid, "weight": 1.0}]))
        if args.limit and len(challengers) >= args.limit:
            break

    mixtures: dict[str, list[dict[str, Any]]] = {"defender": def_members}
    for cname, members in challengers:
        mixtures[f"ch:{cname}"] = members

    # Build deterministic stratified seat-swapped schedule with strictly
    # independent seeds per (stratum, rep) so stratified_bounds and sequential
    # confidence sequences remain statistically valid.
    def_weights = [m["weight"] for m in def_members]
    stratum_counts = allocate_stratum_counts(def_weights, args.pairs)

    schedule: list[dict[str, Any]] = []
    for cname, c_members in challengers:
        ch_mix = f"ch:{cname}"
        cid = c_members[0]["id"]
        block_idx = 0
        for s_idx, (dm, n_pairs) in enumerate(zip(def_members, stratum_counts)):
            did = dm["id"]
            for rep in range(n_pairs):
                tag = f"nc2000/screen/{args.seed_tag}/s{s_idx}/r{rep}"
                seed_ch = u32_hash(f"{tag}/ch/{cname if args.independent_seeds else 'shared'}")
                seed_def = u32_hash(f"{tag}/def/{did}")
                battle_seed = u32_hash(f"{tag}/battle")
                for row_seat in (0, 1):
                    if row_seat == 0:
                        ids = [cid, did]
                        seeds = [seed_ch, seed_def]
                        mix = [ch_mix, "defender"]
                    else:
                        ids = [did, cid]
                        seeds = [seed_def, seed_ch]
                        mix = ["defender", ch_mix]
                    schedule.append({
                        "block": block_idx,
                        "cell": {"challenger": cname, "def_id": did, "stratum": s_idx, "rep": rep},
                        "rowSeat": row_seat,
                        "ids": ids,
                        "seeds": seeds,
                        "battleSeed": battle_seed,
                        "mixtures": mix,
                    })
                block_idx += 1

    args.out_dir.mkdir(parents=True, exist_ok=True)
    pools_obj = {
        "teams": [{"id": tid, "sets": sets} for tid, sets in extra_teams.items()],
        "mixtures": mixtures,
    }
    (args.out_dir / "pools.json").write_text(json.dumps(pools_obj))
    with (args.out_dir / "schedule.jsonl").open("w") as fh:
        for row in schedule:
            fh.write(json.dumps(row) + "\n")
    meta = {
        "defender": str(args.defender),
        "catalog": str(args.catalog),
        "seed_tag": args.seed_tag,
        "pairs_per_challenger": args.pairs,
        "defender_weights": def_members,
        "stratum_counts": stratum_counts,
        "challenger_count": len(challengers),
        "total_games": len(schedule),
    }
    (args.out_dir / "meta.json").write_text(json.dumps(meta, indent=2))
    print(json.dumps({
        "out_dir": str(args.out_dir),
        "challengers": len(challengers),
        "pairs_per_challenger": args.pairs,
        "total_games": len(schedule),
        "stratum_counts": stratum_counts,
    }))


def cmd_run(args: argparse.Namespace) -> None:
    out_dir = args.dir / args.run_name
    env = os.environ.copy()
    env["NASH_POOLS"] = str(args.dir / "pools.json")
    if args.preview_iters is not None:
        env["NASH_PREVIEW_ITERS"] = str(args.preview_iters)
    if args.battle_iters is not None:
        env["NASH_BATTLE_ITERS"] = str(args.battle_iters)
    if args.battle_mode is not None:
        env["NASH_BATTLE_MODE"] = str(args.battle_mode)
    env["NASH_PREVIEW_CACHE"] = "1"
    res = subprocess.run(
        [str(args.binary), str(args.dir / "schedule.jsonl"), str(out_dir), str(args.workers)],
        env=env,
        capture_output=True,
        text=True,
        check=True,
    )
    print(res.stdout.strip())


def row_score(outcome: str, row_seat: int) -> float:
    if outcome in ("tie", "cap"):
        return 0.5
    win_seat = 0 if outcome == "p1" else 1
    return 1.0 if win_seat == row_seat else 0.0


def cmd_summarize(args: argparse.Namespace) -> None:
    meta = json.loads((args.dir / "meta.json").read_text())
    run_dir = args.dir / args.run_name
    def_weights = {m["id"]: float(m["weight"]) for m in meta["defender_weights"]}

    # Group seat pairs: (challenger, def_id, block) -> [scores]
    pairs: dict[tuple[str, str, int], list[float]] = defaultdict(list)
    total_games = 0
    for p in sorted(run_dir.glob("*.json")):
        if p.name == "manifest.json":
            continue
        r = json.loads(p.read_text())
        total_games += 1
        cell = r["cell"]
        key = (cell["challenger"], cell["def_id"], int(r["block"]))
        pairs[key].append(row_score(r["outcome"], int(r["rowSeat"])))

    by_ch_stratum: dict[str, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    for (cname, did, _blk), scs in pairs.items():
        if len(scs) == 2:
            by_ch_stratum[cname][did].append(0.5 * (scs[0] + scs[1]))

    ranked = []
    for cname, smap in by_ch_stratum.items():
        active_strata = [(w, smap[did]) for did, w in def_weights.items() if did in smap and smap[did]]
        w_sum = sum(w for w, _ in active_strata)
        norm_strata = [(w / w_sum, xs) for w, xs in active_strata]
        b = STATS.stratified_bounds(norm_strata, alpha=0.05, one_sided=True)
        # Also compute unweighted betting CS over all pairs when single-stratum
        all_xs = [x for _, xs in norm_strata for x in xs]
        bcs = STATS.betting_cs(all_xs, alpha=0.05, one_sided="upper") if len(norm_strata) == 1 else None
        ranked.append({
            "challenger": cname,
            "pairs": sum(len(xs) for _, xs in norm_strata),
            "mean": round(b["mean"], 4),
            "se": round(b["se"], 4),
            "upper_normal_95": round(b["upper_normal"], 4),
            "upper_eb_95": round(b["upper_eb"], 4),
            "upper_hoeffding_95": round(b["upper_hoeffding"], 4),
            **({"upper_betting_95": round(bcs["upper"], 4)} if bcs else {}),
        })

    ranked.sort(key=lambda r: (-r["mean"], -r["upper_normal_95"], r["challenger"]))
    summary = {
        "run_dir": str(run_dir),
        "total_games": total_games,
        "challengers_scored": len(ranked),
        "top": ranked[: args.top],
        "above_50_count": sum(1 for r in ranked if r["mean"] > 0.5),
    }
    out_file = args.out or (args.dir / f"summary-{args.run_name}.json")
    out_file.write_text(json.dumps({"summary": summary, "all": ranked}, indent=2))
    print(json.dumps(summary))


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_prep = sub.add_parser("prepare")
    p_prep.add_argument("--defender", type=Path, required=True)
    p_prep.add_argument("--catalog", type=Path, required=True)
    p_prep.add_argument("--pairs", type=int, default=4)
    p_prep.add_argument("--seed-tag", type=str, required=True)
    p_prep.add_argument("--out-dir", type=Path, required=True)
    p_prep.add_argument("--ids", type=str, default=None, help="Comma-separated challenger IDs")
    p_prep.add_argument("--all-variants", action="store_true")
    p_prep.add_argument("--independent-seeds", action="store_true")
    p_prep.add_argument("--limit", type=int, default=0)
    p_prep.set_defaults(func=cmd_prepare)

    p_run = sub.add_parser("run")
    p_run.add_argument("--dir", type=Path, required=True)
    p_run.add_argument("--run-name", type=str, default="out")
    p_run.add_argument("--binary", type=Path, default=ROOT / "target/native-pgo/release/examples/nash_refresh")
    p_run.add_argument("--workers", type=int, default=10)
    p_run.add_argument("--preview-iters", type=int, default=None)
    p_run.add_argument("--battle-iters", type=int, default=None)
    p_run.add_argument("--battle-mode", choices=["adaptive", "fixed"], default=None)
    p_run.set_defaults(func=cmd_run)

    p_sum = sub.add_parser("summarize")
    p_sum.add_argument("--dir", type=Path, required=True)
    p_sum.add_argument("--run-name", type=str, default="out")
    p_sum.add_argument("--top", type=int, default=20)
    p_sum.add_argument("--out", type=Path, default=None)
    p_sum.set_defaults(func=cmd_summarize)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
