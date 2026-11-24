#!/usr/bin/env python3
"""Phase 0 surrogate fidelity & MLMC variance-reduction calibrator.

Replays archived production (729k preview / adaptive 243k battle) disclosed-pool
games from `data/nash-camouflage-v1/` at lower native budgets via `nash_refresh`
and measures:
  - exact throughput & preview-cache hit rate
  - same-seed game & seat-pair correlation (MLMC control-variate gain)
  - matchup-level Pearson/Spearman correlation, MAE, and systematic bias
"""

from __future__ import annotations

import argparse
import json
import math
import os
import shutil
import subprocess
import tempfile
import time
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent

# (games_jsonl, pools_json) relative to data/nash-camouflage-v1/
DATASETS: list[tuple[str, str]] = [
    ("screen-games.jsonl", "screen-pools.json"),
    ("response-games.jsonl", "response-pools.json"),
    ("refine-games.jsonl", "response-pools.json"),
    ("fit-games.jsonl", "response-pools.json"),
    ("deviation-games.jsonl", "final-pools.json"),
    ("confirmation-games.jsonl", "confirmation-pools.json"),
    ("stratified-games.jsonl", "confirmation-pools.json"),
    ("s0155/screen-games.jsonl", "s0155/pools.json"),
    ("s0155/holdout-games.jsonl", "s0155/pools.json"),
    ("sample17/games.jsonl", "sample17/pools.json"),
    ("sample17/holdout-games.jsonl", "sample17/pools.json"),
    ("new-matchups/screen-games.jsonl", "new-matchups/pools.json"),
    ("new-matchups/holdout-games.jsonl", "new-matchups/pools.json"),
    ("new-matchups/secondary-games.jsonl", "new-matchups/pools.json"),
]


CORE_DATASETS: list[tuple[str, str]] = [
    ("fit-games.jsonl", "response-pools.json"),
    ("confirmation-games.jsonl", "confirmation-pools.json"),
    ("s0155/holdout-games.jsonl", "s0155/pools.json"),
    ("sample17/holdout-games.jsonl", "sample17/pools.json"),
    ("new-matchups/holdout-games.jsonl", "new-matchups/pools.json"),
]


def row_score(outcome: str, row_seat: int) -> float:
    if outcome in ("tie", "cap"):
        return 0.5
    win_seat = 0 if outcome == "p1" else 1
    return 1.0 if win_seat == row_seat else 0.0


def pearson(xs: list[float], ys: list[float]) -> float:
    n = len(xs)
    if n < 2:
        return float("nan")
    mx = sum(xs) / n
    my = sum(ys) / n
    vx = sum((x - mx) ** 2 for x in xs)
    vy = sum((y - my) ** 2 for y in ys)
    if vx <= 0 or vy <= 0:
        return float("nan")
    cov = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    return cov / math.sqrt(vx * vy)


def ranks(vals: list[float]) -> list[float]:
    indexed = sorted(enumerate(vals), key=lambda kv: kv[1])
    out = [0.0] * len(vals)
    i = 0
    while i < len(indexed):
        j = i
        while j + 1 < len(indexed) and indexed[j + 1][1] == indexed[i][1]:
            j += 1
        avg_rank = 0.5 * (i + j) + 1.0
        for k in range(i, j + 1):
            out[indexed[k][0]] = avg_rank
        i = j + 1
    return out


def spearman(xs: list[float], ys: list[float]) -> float:
    if len(xs) < 2:
        return float("nan")
    return pearson(ranks(xs), ranks(ys))


def sample_var(xs: list[float]) -> float:
    n = len(xs)
    if n < 2:
        return float("nan")
    m = sum(xs) / n
    return sum((x - m) ** 2 for x in xs) / (n - 1)


def parse_budget_spec(spec: str) -> dict[str, Any]:
    # Format: name:battle:preview:mode  e.g. 3k_9k:3000:9000:fixed
    parts = spec.split(":")
    if len(parts) != 4:
        raise ValueError(f"invalid budget spec {spec!r}, expected name:battle:preview:mode")
    return {
        "name": parts[0],
        "battle_iters": int(parts[1]),
        "preview_iters": int(parts[2]),
        "battle_mode": parts[3],
    }


def run_budget(
    binary: Path,
    budget: dict[str, Any],
    workers: int,
    datasets: list[tuple[str, str]],
) -> dict[str, Any]:
    base_dir = ROOT / "data" / "nash-camouflage-v1"
    tmp = Path(tempfile.mkdtemp(prefix=f"nc2000-cal-{budget['name']}-"))
    try:
        total_games = 0
        total_wall_ms = 0
        total_cpu_ms = 0
        total_wasm_ms = 0
        total_hits = 0
        total_misses = 0
        exact_outcome_matches = 0
        preview_pick_total = 0
        preview_pick_matches = 0

        # Game & pair accumulators
        game_prod: list[float] = []
        game_surr: list[float] = []
        pair_groups: dict[tuple[Any, ...], list[tuple[float, float]]] = defaultdict(list)
        matchup_groups: dict[tuple[str, str, str], list[tuple[float, float]]] = defaultdict(list)

        t_start = time.monotonic()
        for games_rel, pools_rel in datasets:
            games_path = base_dir / games_rel
            pools_path = base_dir / pools_rel
            ref_rows = [json.loads(l) for l in games_path.open() if l.strip()]
            out_dir = tmp / games_rel.replace("/", "_").replace(".jsonl", "")
            env = os.environ.copy()
            env["NASH_POOLS"] = str(pools_path)
            env["NASH_PREVIEW_ITERS"] = str(budget["preview_iters"])
            env["NASH_BATTLE_ITERS"] = str(budget["battle_iters"])
            env["NASH_BATTLE_MODE"] = str(budget["battle_mode"])
            env["NASH_PREVIEW_CACHE"] = "1"
            res = subprocess.run(
                [str(binary), str(games_path), str(out_dir), str(workers)],
                env=env,
                capture_output=True,
                text=True,
                check=True,
            )
            run_meta = json.loads(res.stdout.strip())
            total_wall_ms += int(run_meta["wall_ms"])
            total_hits += int(run_meta.get("preview_cache_hits", 0))
            total_misses += int(run_meta.get("preview_cache_misses", 0))

            for idx, ref in enumerate(ref_rows):
                got = json.loads((out_dir / f"{idx}.json").read_text())
                total_games += 1
                total_cpu_ms += int(got["ms"])
                total_wasm_ms += int(ref.get("ms", 0))

                rs = int(ref["rowSeat"])
                sp = row_score(ref["outcome"], rs)
                ss = row_score(got["outcome"], rs)
                game_prod.append(sp)
                game_surr.append(ss)
                if ref["outcome"] == got["outcome"]:
                    exact_outcome_matches += 1

                for side in (0, 1):
                    preview_pick_total += 1
                    if ref["preview"][side]["input"] == got["preview"][side]["input"]:
                        preview_pick_matches += 1

                mix = ref["mixtures"]
                row_mix = mix[rs]
                col_mix = mix[1 - rs]
                # Pair key: same file, block, cell, mixtures (opposite rowSeat)
                pair_key = (
                    games_rel,
                    json.dumps(ref.get("block"), sort_keys=True),
                    json.dumps(ref.get("cell"), sort_keys=True),
                    row_mix,
                    col_mix,
                    int(ref["battleSeed"]),
                )
                pair_groups[pair_key].append((sp, ss))
                matchup_groups[(games_rel, row_mix, col_mix)].append((sp, ss))

        wall_s = time.monotonic() - t_start

        # Seat-pair level metrics (complete pairs of 2 seats)
        pair_prod: list[float] = []
        pair_surr: list[float] = []
        for vals in pair_groups.values():
            if len(vals) == 2:
                pair_prod.append(0.5 * (vals[0][0] + vals[1][0]))
                pair_surr.append(0.5 * (vals[0][1] + vals[1][1]))

        rho_game = pearson(game_prod, game_surr)
        rho_pair = pearson(pair_prod, pair_surr)
        mlmc_var_ratio = (1.0 - rho_pair ** 2) if not math.isnan(rho_pair) else float("nan")
        mlmc_eff_gain = (1.0 / mlmc_var_ratio) if mlmc_var_ratio > 0 else float("nan")
        prod_pair_var = sample_var(pair_prod)

        # Matchup level (>= 8 games = >= 4 seat pairs)
        m_rows = []
        for (file_rel, row_mix, col_mix), vals in sorted(matchup_groups.items()):
            if len(vals) < 8:
                continue
            mp = sum(v[0] for v in vals) / len(vals)
            ms = sum(v[1] for v in vals) / len(vals)
            m_rows.append({
                "file": file_rel,
                "row_mix": row_mix,
                "col_mix": col_mix,
                "games": len(vals),
                "prod_score": round(mp, 4),
                "surr_score": round(ms, 4),
                "diff": round(ms - mp, 4),
            })

        m_prod = [r["prod_score"] for r in m_rows]
        m_surr = [r["surr_score"] for r in m_rows]
        m_diffs = [r["diff"] for r in m_rows]
        m_mae = sum(abs(d) for d in m_diffs) / len(m_diffs) if m_diffs else float("nan")
        m_bias = sum(m_diffs) / len(m_diffs) if m_diffs else float("nan")
        m_max_abs = max((abs(d) for d in m_diffs), default=0.0)
        worst_outliers = sorted(m_rows, key=lambda r: abs(r["diff"]), reverse=True)[:8]

        # Preview cache stats
        total_previews = total_hits + total_misses
        hit_rate = (total_hits / total_previews) if total_previews > 0 else 0.0

        return {
            "budget": budget,
            "games": total_games,
            "seat_pairs": len(pair_prod),
            "wall_s": round(wall_s, 2),
            "games_per_min": round(total_games / (wall_s / 60.0), 1) if wall_s > 0 else 0.0,
            "games_per_hour": round(total_games / (wall_s / 3600.0), 1) if wall_s > 0 else 0.0,
            "mean_cpu_ms_per_game": round(total_cpu_ms / total_games, 1),
            "speedup_vs_archived_wasm": round(total_wasm_ms / total_cpu_ms, 2) if total_cpu_ms > 0 else 0.0,
            "preview_cache_hits": total_hits,
            "preview_cache_misses": total_misses,
            "preview_cache_hit_rate": round(hit_rate, 4),
            "exact_outcome_match_rate": round(exact_outcome_matches / total_games, 4),
            "preview_pick_match_rate": round(preview_pick_matches / preview_pick_total, 4),
            "game_corr": round(rho_game, 4),
            "seat_pair_corr": round(rho_pair, 4),
            "prod_seat_pair_var": round(prod_pair_var, 4),
            "mlmc_var_reduction_factor": round(mlmc_eff_gain, 3),
            "matchup_count_ge8": len(m_rows),
            "matchup_pearson_r": round(pearson(m_prod, m_surr), 4),
            "matchup_spearman_rho": round(spearman(m_prod, m_surr), 4),
            "matchup_mae": round(m_mae, 4),
            "matchup_mean_bias": round(m_bias, 4),
            "matchup_max_abs_diff": round(m_max_abs, 4),
            "worst_outliers": worst_outliers,
            "matchups": m_rows,
        }
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--binary",
        type=Path,
        default=ROOT / "target/native-pgo/release/examples/nash_refresh",
    )
    ap.add_argument("--workers", type=int, default=10)
    ap.add_argument("--subset", choices=["all", "core"], default="all")
    ap.add_argument(
        "--budgets",
        nargs="+",
        default=[
            "3k_9k:3000:9000:fixed",
            "9k_27k:9000:27000:fixed",
            "27k_81k:27000:81000:fixed",
        ],
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=ROOT / "target/phase0-surrogate-calibration.json",
    )
    args = ap.parse_args()

    datasets = CORE_DATASETS if args.subset == "core" else DATASETS
    results = []
    for spec in args.budgets:
        budget = parse_budget_spec(spec)
        results.append(run_budget(args.binary, budget, args.workers, datasets))

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps({"results": results}, indent=2))

    # Single-line structured telemetry summary
    compact = [
        {
            "name": r["budget"]["name"],
            "games": r["games"],
            "wall_s": r["wall_s"],
            "g_per_hr": r["games_per_hour"],
            "cache_hit": r["preview_cache_hit_rate"],
            "outcome_agree": r["exact_outcome_match_rate"],
            "pick_agree": r["preview_pick_match_rate"],
            "pair_corr": r["seat_pair_corr"],
            "mlmc_gain": r["mlmc_var_reduction_factor"],
            "m_r": r["matchup_pearson_r"],
            "m_rho": r["matchup_spearman_rho"],
            "m_mae": r["matchup_mae"],
        }
        for r in results
    ]
    print(json.dumps({"out": str(args.out), "prod_pair_var": results[0]["prod_seat_pair_var"], "budgets": compact}))


if __name__ == "__main__":
    main()
