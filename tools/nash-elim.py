#!/usr/bin/env python3
"""Elimination-first (Dominance Pruning) pipeline for Axis-A NC2000 Nash pool search.

Architecture:
  1. `audit-existing`: Integrates all on-disk evaluations (`phase1-cut` 50,460 games at
     9k/81k, `swiss-v1` 6,060 games at 3k/9k, `nash-ledger.jsonl` production 243k/729k)
     using a zero-bias fresh dominator frontier (`D_fresh` from `step2b`) to partition
     the 510 catalog shells into certified tiers (S3=294 safe, S5=149 core) with
     per-shell elimination certificates.
  2. `prepare-basis` / `run`: Evaluates the 9x9 Bare Orthogonal Basis `B9` at 9k/81k
     (bare singleton vs bare singleton, zero camouflage handicap, 12 seat-pairs/cell =
     864 games) to establish a zero-bias, low-SE dominator basis on the exact same scale.
  3. `prepare-round` / `run`: Adaptive sequential racing of surviving shells against `B9`
     across `mini1` (10w) + `mini2` (12w) via `tools/mini2_nash.py`.
  4. `analyze`: Computes exact bare-vs-B9 payoff profiles, stratified SE / Betting CS,
     zero-bias studentized convex-hull dominance against `B9` (preserving RPS specialists
     automatically), and writes certified survivor/eliminated archives.

Emits a single compact JSON summary line per command (M2M output principle).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import re
import time
from collections import Counter, defaultdict
from importlib.machinery import SourceFileLoader
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
STATS = SourceFileLoader("nash_stats", str(ROOT / "tools/nash-stats.py")).load_module()
LEDGER = SourceFileLoader("nash_ledger", str(ROOT / "tools/nash-ledger.py")).load_module()
MINI2 = SourceFileLoader("mini2_nash", str(ROOT / "tools/mini2_nash.py")).load_module()

DEFAULT_BIN = ROOT / "target/native-pgo/release/examples/nash_refresh"
DEFAULT_WORK = ROOT / "target/nash-work/elim-v1"

PHASE1_ANCHORS = [
    "anchor:APEX4",
    "anchor:Sample07",
    "anchor:D5",
    "anchor:s0155",
    "anchor:sample06",
]

# Known production-strong / historical-Nash shells for 0%-FN calibration audits (GT19).
GT19_IDS = [
    "apex-c04-bug-3atkstar-boomgen",
    "sample-07",
    "deme-r1-00-s0030",
    "red-v10-s1105-r2-s0155",
    "sample-06",
    "sample-17-orig",
    "mjj-2014-thdrcomb__56b67d",
    "mjj-2010-firecomb__ddddbb",
    "rental-cban-8",
    "deme-r3-b2-s0013",
    "nb-sample-13-0",
    "sample-08",
    "rental-cban-3",
    "sample-10",
    "pb-122",
    "mjj-2010-garap-a",
    "supp-deme-r1-00-s0042",
    "rental-cban-6__4681ec",
    "pb-74",
]

# Zero-bias fresh step2b dominator IDs for Stage-0 offline audit.
STAGE0_D_FRESH_IDS = [
    "apex-c04-bug-3atkstar-boomgen",
    "apex-c08-surgical-142",
    "sample-07",
    "deme-r1-00-s0030",
    "red-v10-s1105-r1-s0146",
    "sample-17-orig",
    "mjj-2014-thdrcomb__56b67d",
    "mjj-2010-firecomb__ddddbb",
    "rental-cban-8",
    "deme-r3-r0-s0018",
]

# 9-anchor Bare Orthogonal Basis (23 unique species, mean pairwise Jaccard 0.197).
B9_IDS = [
    "sample-07",                     # Zapdos55 / Cloyster / Snorlax / Electrode / Tauros / Marowak
    "apex-c04-bug-3atkstar-boomgen", # Snorlax / Cloyster / Gengar / Heracross / Marowak / Starmie
    "deme-r1-00-s0030",              # Tyranitar / Skarmory / Marowak / Snorlax / Raikou / Suicune
    "red-v10-s1105-r2-s0155",        # Blissey / Skarmory / Marowak / Snorlax / Miltank / Suicune
    "mjj-2010-firecomb__ddddbb",     # Moltres / Cloyster / Snorlax / Gengar / Machamp / Raikou
    "mjj-2014-thdrcomb__56b67d",     # Zapdos / Tyranitar / Venusaur / Steelix / Cloyster / Snorlax
    "rental-cban-8",                 # Tyranitar / Exeggutor / Venusaur / Golem / Snorlax / Miltank
    "sample-17-orig",                # Steelix / Exeggutor / Porygon2 / Snorlax / Zapdos / Suicune
    "pb-122",                        # Marowak / Raikou / Electrode / Jolteon / Exeggutor / Cloyster
]


def u32_hash(tag: str) -> int:
    return int.from_bytes(hashlib.sha256(tag.encode("utf-8")).digest()[:4], "big")


def seat_score(outcome: str, seat: int) -> float:
    if outcome in ("tie", "cap", "draw"):
        return 0.5
    return 1.0 if ((0 if outcome == "p1" else 1) == seat) else 0.0


def load_full_catalog() -> tuple[list[dict[str, Any]], dict[str, list[dict[str, Any]]]]:
    """Load the 691-team catalog metadata and complete team sets map."""
    cat_doc = json.loads((ROOT / "target/nash-work/swiss-v1/catalog.json").read_text())
    pools_doc = json.loads((ROOT / "target/nash-work/swiss-v1/pools.json").read_text())
    base_teams, _ = LEDGER.load_catalog_and_mixtures(None)
    all_sets: dict[str, list[dict[str, Any]]] = dict(base_teams)
    for t in pools_doc["teams"]:
        all_sets[t["id"]] = t["sets"]
    return cat_doc["teams"], all_sets


def solve_convex_hull_dominance(
    cand_m: list[float],
    cand_se: list[float],
    dom_m: list[list[float]],
    dom_se: list[list[float]],
    dom_names: list[str],
    *,
    z: float = 1.645,
    iters: int = 300,
) -> dict[str, Any]:
    """Compute max_{w in Delta} min_k ((w^T dom_m)_k - cand_m[k]) via RM+, plus
    exact studentized z-gap (diff - z * sqrt(cand_se_k^2 + sum_j w_j^2 dom_se_{j,k}^2))
    across all basis axes k."""
    k_dim = len(cand_m)
    m_dom = len(dom_m)
    assert m_dom > 0 and k_dim > 0

    r_reg = [0.0] * k_dim
    c_reg = [0.0] * m_dom
    c_sum = [0.0] * m_dom
    p = [1.0 / k_dim] * k_dim
    q = [1.0 / m_dom] * m_dom

    for t in range(1, iters + 1):
        col_pay = [sum(p[k] * (dom_m[j][k] - cand_m[k]) for k in range(k_dim)) for j in range(m_dom)]
        vc = sum(q[j] * col_pay[j] for j in range(m_dom))
        for j in range(m_dom):
            c_reg[j] = max(0.0, c_reg[j] + col_pay[j] - vc)
        sc = sum(c_reg)
        q = [x / sc for x in c_reg] if sc > 0.0 else [1.0 / m_dom] * m_dom

        row_pay = [cand_m[k] - sum(q[j] * dom_m[j][k] for j in range(m_dom)) for k in range(k_dim)]
        vr = sum(p[k] * row_pay[k] for k in range(k_dim))
        for k in range(k_dim):
            r_reg[k] = max(0.0, r_reg[k] + row_pay[k] - vr)
        sr = sum(r_reg)
        p = [x / sr for x in r_reg] if sr > 0.0 else [1.0 / k_dim] * k_dim

        for j in range(m_dom):
            c_sum[j] += t * q[j]

    cs = sum(c_sum)
    w = [x / cs for x in c_sum] if cs > 0.0 else [1.0 / m_dom] * m_dom

    pt_gaps: list[float] = []
    z_gaps: list[float] = []
    for k in range(k_dim):
        mu_d = sum(w[j] * dom_m[j][k] for j in range(m_dom))
        var_d = sum((w[j] ** 2) * (dom_se[j][k] ** 2) for j in range(m_dom))
        diff = mu_d - cand_m[k]
        se_diff = math.sqrt(cand_se[k] ** 2 + var_d)
        pt_gaps.append(diff)
        z_gaps.append(diff - z * se_diff)

    witness = {
        dom_names[j]: round(w[j], 4)
        for j in range(m_dom)
        if w[j] >= 0.01
    }
    return {
        "pt_gap": round(min(pt_gaps), 4),
        "z_gap": round(min(z_gaps), 4),
        "per_axis_pt_gap": [round(x, 4) for x in pt_gaps],
        "per_axis_z_gap": [round(x, 4) for x in z_gaps],
        "witness_mixture": witness,
    }


def pair_cell_stats(xs: list[float]) -> tuple[float, float]:
    """Mean and conservative Laplace-floored SE of seat-pair scores in {0, 0.5, 1}."""
    n = len(xs)
    if n == 0:
        return 0.5, 0.5
    m = sum(xs) / n
    v = sum((x - m) ** 2 for x in xs) / max(1, n - 1) if n > 1 else 0.25
    p_reg = (sum(xs) + 0.5) / (n + 1.0)
    v_floor = 0.5 * p_reg * (1.0 - p_reg)
    return m, math.sqrt(max(v, v_floor) / n)


def stratified_mean_u95(axis_lists: list[list[float]]) -> tuple[float, float, float, float]:
    """Equal-weight stratified mean, SE, one-sided 95% t-upper, and Betting CS upper."""
    k = len(axis_lists)
    w = 1.0 / k
    mean = 0.0
    var = 0.0
    flat: list[float] = []
    for xs in axis_lists:
        m, se = pair_cell_stats(xs)
        mean += w * m
        var += (w ** 2) * (se ** 2)
        flat.extend(xs)
    se_all = math.sqrt(var)
    n = len(flat)
    t_crit = 1.746 if n <= 18 else (1.699 if n <= 35 else 1.671)
    u95_t = min(1.0, mean + t_crit * se_all)
    u95_bet = float(STATS.betting_cs(flat, alpha=0.05, one_sided="upper")["upper"]) if flat else 1.0
    return mean, se_all, u95_t, u95_bet


# ---------------------------------------------------------------------------
# Subcommand: audit-existing (Stage 0, 0 new games)
# ---------------------------------------------------------------------------

def cmd_audit_existing(args: argparse.Namespace) -> None:
    t0 = time.monotonic()
    cat_entries, _ = load_full_catalog()
    cat_by_id = {e["id"]: e for e in cat_entries}
    shell_to_ids: dict[str, list[str]] = defaultdict(list)
    for e in cat_entries:
        shell_to_ids[e["shell_hash"]].append(e["id"])

    # 1. Load all phase1-cut 9k/81k games
    raw_pairs: dict[tuple[str, str, str, int], list[float]] = defaultdict(list)
    for phase, rel in [
        ("s1a", "target/nash-work/phase1-cut/stage1a/results.jsonl"),
        ("s1b", "target/nash-work/phase1-cut/stage1b/results.jsonl"),
        ("s2a", "target/nash-work/phase1-cut/step2a/results.jsonl"),
        ("s2b", "target/nash-work/phase1-cut/step2b/results.jsonl"),
    ]:
        fp = ROOT / rel
        if not fp.exists():
            continue
        for line in fp.open():
            if not line.strip():
                continue
            r = json.loads(line)
            c = r["cell"]
            raw_pairs[(phase, c["candidate"], c["anchor"], int(c["rep"]))].append(
                seat_score(r["outcome"], int(r["rowSeat"]))
            )

    all_pairs: dict[str, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    fresh_pairs: dict[str, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    for (phase, cid, anc, _rep), scs in raw_pairs.items():
        if len(scs) == 2:
            ps = 0.5 * (scs[0] + scs[1])
            all_pairs[cid][anc].append(ps)
            if phase == "s2b":
                fresh_pairs[cid][anc].append(ps)

    # 2. Load swiss-v1 (3k/9k) Bradley-Terry Elo
    swiss_games: dict[str, list[tuple[str, float]]] = defaultdict(list)
    for rd in range(1, 19):
        rf = ROOT / f"target/nash-work/swiss-v1/round-{rd}/results.jsonl"
        if not rf.exists():
            continue
        for line in rf.open():
            if not line.strip():
                continue
            r = json.loads(line)
            id0, id1 = r["ids"]
            s0 = seat_score(r["outcome"], 0)
            swiss_games[id0].append((id1, s0))
            swiss_games[id1].append((id0, 1.0 - s0))
    all_sw = sorted(swiss_games.keys())
    bt_s = {t: 1.0 for t in all_sw}
    for _ in range(80):
        ns = {}
        for t in all_sw:
            gs = swiss_games[t]
            w = sum(sc for _, sc in gs) + 0.5
            den = 1.0 / (bt_s[t] + 1.0) + sum(1.0 / (bt_s[t] + bt_s[o]) for o, _ in gs)
            ns[t] = w / den
        ml = sum(math.log(v) for v in ns.values()) / max(1, len(ns))
        bt_s = {k: v / math.exp(ml) for k, v in ns.items()}
    bt_elo = {k: 400.0 * math.log10(v) for k, v in bt_s.items()}

    # 3. Load production ledger (243k/729k)
    ledger_shells: dict[str, list[float]] = defaultdict(list)
    ledger_fp = ROOT / "target/nash-ledger.jsonl"
    if ledger_fp.exists():
        for line in ledger_fp.open():
            if not line.strip():
                continue
            r = json.loads(line)
            id0, id1 = r["ids"]
            s0 = seat_score(r["outcome"], 0)
            if id0 in cat_by_id:
                ledger_shells[cat_by_id[id0]["shell_hash"]].append(s0)
            if id1 in cat_by_id:
                ledger_shells[cat_by_id[id1]["shell_hash"]].append(1.0 - s0)

    # 4. Zero-bias fresh dominator frontier D_fresh
    d_ids = [t for t in STAGE0_D_FRESH_IDS if t in fresh_pairs]
    d_means = [[pair_cell_stats(fresh_pairs[t][a])[0] for a in PHASE1_ANCHORS] for t in d_ids]
    d_ses = [[pair_cell_stats(fresh_pairs[t][a])[1] for a in PHASE1_ANCHORS] for t in d_ids]

    gt19_shells = {cat_by_id[t]["shell_hash"] for t in GT19_IDS if t in cat_by_id}
    b9_by_shell = {cat_by_id[t]["shell_hash"]: t for t in B9_IDS if t in cat_by_id}

    records: list[dict[str, Any]] = []
    for sh, tids in sorted(shell_to_ids.items()):
        if all(t.startswith("supp-cov-") for t in tids):
            records.append({
                "shell_hash": sh,
                "rep_id": tids[0],
                "variants": tids,
                "species": cat_by_id[tids[0]]["species"],
                "tier": "CUT_S1_DUMMY",
                "in_s3": False,
                "in_s5": False,
                "is_gt19": False,
                "reason": "synthetic_supp_cov_dummy",
            })
            continue

        ok = [t for t in tids if t in all_pairs]
        t_stats = {}
        for t in ok:
            axis_lists = [all_pairs[t][a] for a in PHASE1_ANCHORS]
            m, se, u95_t, u95_b = stratified_mean_u95(axis_lists)
            anc_m = [pair_cell_stats(all_pairs[t][a])[0] for a in PHASE1_ANCHORS]
            anc_se = [pair_cell_stats(all_pairs[t][a])[1] for a in PHASE1_ANCHORS]
            t_stats[t] = {
                "mean": m,
                "se": se,
                "u95_t": u95_t,
                "u95_b": u95_b,
                "anc_m": anc_m,
                "anc_se": anc_se,
                "n_pairs": sum(len(x) for x in axis_lists),
            }

        # Pick shell representative: canonical B9 member if this shell is in B9, else highest phase1 mean
        if sh in b9_by_shell:
            rep = b9_by_shell[sh]
        else:
            rep = max(ok, key=lambda t: (t_stats[t]["mean"], t_stats[t]["n_pairs"], t))

        # Optimistic shell envelope across all variants for conservative elimination
        env_m = [max(t_stats[t]["anc_m"][k] for t in ok) for k in range(len(PHASE1_ANCHORS))]
        env_se = [
            t_stats[max(ok, key=lambda t: t_stats[t]["anc_m"][k])]["anc_se"][k]
            for k in range(len(PHASE1_ANCHORS))
        ]
        dom = solve_convex_hull_dominance(env_m, env_se, d_means, d_ses, d_ids, z=1.645)

        best_mean = max(t_stats[t]["mean"] for t in ok)
        best_u95 = max(t_stats[t]["u95_t"] for t in ok)
        best_u95_bet = max(t_stats[t]["u95_b"] for t in ok)
        max_a = max(env_m)
        sw_elo = max(bt_elo.get(t, -999.0) for t in ok)
        prod_scs = ledger_shells.get(sh, [])

        cut_s2 = (best_u95 < 0.45 and max_a <= 0.50) or (dom["z_gap"] > 0.0)
        cut_s3 = cut_s2 or (
            dom["pt_gap"] > 0.15 and best_u95 < 0.52 and max_a <= 0.58 and sw_elo < 200.0
        )
        in_s5 = (not cut_s3) and (dom["pt_gap"] <= 0.12) and (best_u95 >= 0.53 or max_a >= 0.667)

        if cut_s2:
            tier = "CUT_S2_DEAD"
            reason = "stat_dominated_z_gt_0" if dom["z_gap"] > 0.0 else "u95_lt_0.45_and_maxA_le_0.50"
        elif cut_s3:
            tier = "CUT_S3_SAFE"
            reason = "hull_gap_gt_0.15_and_u95_lt_0.52_and_sw_lt_200"
        elif in_s5:
            tier = "KEEP_S5_CORE"
            reason = "non_dominated_core"
        else:
            tier = "ARCHIVE_S3_BORDERLINE"
            reason = "borderline_deferred_pending_core"

        records.append({
            "shell_hash": sh,
            "rep_id": rep,
            "variants": tids,
            "n_variants": len(tids),
            "species": cat_by_id[rep]["species"],
            "tier": tier,
            "in_s3": not cut_s3,
            "in_s5": in_s5,
            "is_gt19": sh in gt19_shells,
            "reason": reason,
            "phase1_best_mean": round(best_mean, 4),
            "phase1_best_u95_t": round(best_u95, 4),
            "phase1_best_u95_bet": round(best_u95_bet, 4),
            "phase1_max_anchor": round(max_a, 4),
            "phase1_env_means": [round(x, 4) for x in env_m],
            "phase1_max_pairs": max(t_stats[t]["n_pairs"] for t in ok),
            "swiss_best_elo": round(sw_elo, 1),
            "prod_games": len(prod_scs),
            "prod_wr": round(sum(prod_scs) / len(prod_scs), 4) if prod_scs else None,
            "dominance": dom,
        })

    out_dir: Path = args.work_dir
    out_dir.mkdir(parents=True, exist_ok=True)
    audit_path = out_dir / "stage0-audit.json"
    s5_shells = [r for r in records if r["in_s5"]]
    s3_shells = [r for r in records if r["in_s3"]]

    tier_counts: dict[str, int] = defaultdict(int)
    for r in records:
        tier_counts[r["tier"]] += 1

    summary = {
        "total_shells": len(records),
        "total_teams": sum(len(r["variants"]) for r in records),
        "tier_counts": dict(sorted(tier_counts.items())),
        "s3_shells": len(s3_shells),
        "s3_teams": sum(len(r["variants"]) for r in s3_shells),
        "s5_shells": len(s5_shells),
        "s5_teams": sum(len(r["variants"]) for r in s5_shells),
        "gt19_in_s3": sum(1 for r in s3_shells if r["is_gt19"]),
        "gt19_in_s5": sum(1 for r in s5_shells if r["is_gt19"]),
        "d_fresh_ids": d_ids,
        "b9_ids": B9_IDS,
        "elapsed_s": round(time.monotonic() - t0, 2),
    }

    audit_path.write_text(json.dumps({"summary": summary, "shells": records}, indent=2) + "\n")
    (out_dir / "s5-shells.json").write_text(json.dumps({"shells": s5_shells}, indent=2) + "\n")
    (out_dir / "s3-shells.json").write_text(json.dumps({"shells": s3_shells}, indent=2) + "\n")
    print(json.dumps({"cmd": "audit-existing", "out": str(audit_path), **summary}))


# ---------------------------------------------------------------------------
# Helpers for Bare-Singleton Pools & Schedules
# ---------------------------------------------------------------------------

def write_bare_pools(path: Path, team_ids: list[str], all_sets: dict[str, list[dict[str, Any]]]) -> None:
    uniq = sorted(set(team_ids))
    base_teams, _ = LEDGER.load_catalog_and_mixtures(None)
    extra_teams = [{"id": tid, "sets": all_sets[tid]} for tid in uniq if tid not in base_teams]
    mixtures = {f"s:{tid}": [{"id": tid, "weight": 1.0}] for tid in uniq}
    path.write_text(json.dumps({"teams": extra_teams, "mixtures": mixtures}, separators=(",", ":")) + "\n")


# ---------------------------------------------------------------------------
# Subcommand: prepare-basis (9x9 Bare Orthogonal Basis B9)
# ---------------------------------------------------------------------------

def cmd_prepare_basis(args: argparse.Namespace) -> None:
    cat_entries, all_sets = load_full_catalog()
    cat_by_id = {e["id"]: e for e in cat_entries}
    for tid in B9_IDS:
        assert tid in all_sets, f"B9 id missing from catalog: {tid}"
    # Verify pairwise-distinct shells in B9
    b9_shells = {cat_by_id[tid]["shell_hash"] for tid in B9_IDS}
    assert len(b9_shells) == len(B9_IDS), "B9 must have pairwise-distinct shells"

    out_dir: Path = args.work_dir / "b9-matrix"
    out_dir.mkdir(parents=True, exist_ok=True)
    pools_path = out_dir / "pools.json"
    write_bare_pools(pools_path, B9_IDS, all_sets)

    rows: list[dict[str, Any]] = []
    blk = 0
    for i in range(len(B9_IDS)):
        for j in range(i + 1, len(B9_IDS)):
            a, b = B9_IDS[i], B9_IDS[j]
            for rep in range(args.pairs_per_cell):
                tag = f"nc2000/elim-v1/b9/{a}/vs/{b}/r{rep}"
                sa = u32_hash(f"{tag}/a")
                sb = u32_hash(f"{tag}/b")
                bs = u32_hash(f"{tag}/battle")
                for row_seat in (0, 1):
                    if row_seat == 0:
                        ids, seeds, mix = [a, b], [sa, sb], [f"s:{a}", f"s:{b}"]
                    else:
                        ids, seeds, mix = [b, a], [sb, sa], [f"s:{b}", f"s:{a}"]
                    rows.append({
                        "block": blk,
                        "cell": {"kind": "b9", "a": a, "b": b, "rep": rep},
                        "rowSeat": row_seat,
                        "ids": ids,
                        "seeds": seeds,
                        "battleSeed": bs,
                        "mixtures": mix,
                    })
                blk += 1

    sched_path = out_dir / "schedule.jsonl"
    sched_path.write_text("\n".join(json.dumps(r, separators=(",", ":")) for r in rows) + "\n")
    meta = {
        "kind": "b9-matrix",
        "pools": str(pools_path),
        "basis_ids": B9_IDS,
        "pairs_per_cell": args.pairs_per_cell,
        "total_pairs": blk,
        "total_games": len(rows),
        "preview_iters": args.preview_iters,
        "battle_iters": args.battle_iters,
        "battle_mode": args.battle_mode,
    }
    (out_dir / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    print(json.dumps({"cmd": "prepare-basis", "dir": str(out_dir), "pairs": blk, "games": len(rows)}))


# ---------------------------------------------------------------------------
# Subcommand: prepare-round (Racing Round r of surviving shells vs B9)
# ---------------------------------------------------------------------------

def cmd_prepare_round(args: argparse.Namespace) -> None:
    cat_entries, all_sets = load_full_catalog()
    cat_by_id = {e["id"]: e for e in cat_entries}
    b9_set = set(B9_IDS)
    b9_shells = {cat_by_id[t]["shell_hash"] for t in B9_IDS}

    src_doc = json.loads(args.shells_file.read_text())
    shell_list = src_doc["shells"] if "shells" in src_doc else src_doc["survivors"]
    candidates = [
        s["rep_id"]
        for s in shell_list
        if s["rep_id"] not in b9_set and s["shell_hash"] not in b9_shells
    ]

    out_dir: Path = args.work_dir / f"round-{args.round}"
    out_dir.mkdir(parents=True, exist_ok=True)
    pools_path = out_dir / "pools.json"
    write_bare_pools(pools_path, B9_IDS + candidates, all_sets)

    rows: list[dict[str, Any]] = []
    blk = 0
    for cid in candidates:
        for bid in B9_IDS:
            for rep in range(args.rep_start, args.rep_end):
                tag = f"nc2000/elim-v1/race/{cid}/vs/{bid}/r{rep}"
                sc = u32_hash(f"{tag}/c")
                sb = u32_hash(f"{tag}/b")
                bs = u32_hash(f"{tag}/battle")
                for row_seat in (0, 1):
                    if row_seat == 0:
                        ids, seeds, mix = [cid, bid], [sc, sb], [f"s:{cid}", f"s:{bid}"]
                    else:
                        ids, seeds, mix = [bid, cid], [sb, sc], [f"s:{bid}", f"s:{cid}"]
                    rows.append({
                        "block": blk,
                        "cell": {"kind": "race", "round": args.round, "candidate": cid, "basis": bid, "rep": rep},
                        "rowSeat": row_seat,
                        "ids": ids,
                        "seeds": seeds,
                        "battleSeed": bs,
                        "mixtures": mix,
                    })
                blk += 1

    sched_path = out_dir / "schedule.jsonl"
    sched_path.write_text("\n".join(json.dumps(r, separators=(",", ":")) for r in rows) + "\n")
    meta = {
        "kind": f"round-{args.round}",
        "round": args.round,
        "pools": str(pools_path),
        "shells_file": str(args.shells_file),
        "basis_ids": B9_IDS,
        "candidates": candidates,
        "rep_start": args.rep_start,
        "rep_end": args.rep_end,
        "total_pairs": blk,
        "total_games": len(rows),
        "preview_iters": args.preview_iters,
        "battle_iters": args.battle_iters,
        "battle_mode": args.battle_mode,
    }
    (out_dir / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    print(json.dumps({
        "cmd": "prepare-round",
        "round": args.round,
        "dir": str(out_dir),
        "candidates": len(candidates),
        "pairs": blk,
        "games": len(rows),
    }))


# ---------------------------------------------------------------------------
# Subcommand: run (Dual-node mini1 + mini2 resumable chunk execution)
# ---------------------------------------------------------------------------

def cmd_run(args: argparse.Namespace) -> None:
    t0 = time.monotonic()
    run_dir: Path = args.dir
    meta = json.loads((run_dir / "meta.json").read_text())
    pools_path = Path(meta["pools"])
    sched_lines = [l.strip() for l in (run_dir / "schedule.jsonl").open() if l.strip()]
    chunk_size = max(2, args.chunk_games + (args.chunk_games % 2))
    chunks_dir = run_dir / "chunks"
    chunks_dir.mkdir(parents=True, exist_ok=True)

    chunks: list[dict[str, Any]] = []
    for ci, start in enumerate(range(0, len(sched_lines), chunk_size)):
        c_rows = sched_lines[start : start + chunk_size]
        c_sched = chunks_dir / f"chunk-{ci:03d}.jsonl"
        c_out = chunks_dir / f"chunk-{ci:03d}-out"
        c_done = chunks_dir / f"chunk-{ci:03d}.done.json"
        if not c_sched.exists():
            c_sched.write_text("\n".join(c_rows) + "\n")
        chunks.append({
            "ci": ci,
            "sched": c_sched,
            "out": c_out,
            "done": c_done,
            "n_rows": len(c_rows),
            "start": start,
        })

    dispatch = MINI2.run_chunks_dual_node(
        chunks,
        binary=args.binary,
        pools_path=pools_path,
        local_workers=args.local_workers,
        mini2_workers=args.mini2_workers,
        use_mini2=args.mini2,
        preview_iters=int(meta["preview_iters"]),
        battle_iters=int(meta["battle_iters"]),
        battle_mode=str(meta["battle_mode"]),
        preview_cache=True,
    )

    # Assemble ordered results.jsonl
    results_path = run_dir / "results.jsonl"
    out_lines: list[str] = []
    for ch in chunks:
        for idx in range(ch["n_rows"]):
            jf = ch["out"] / f"{idx}.json"
            row = json.loads(jf.read_text())
            row["global_index"] = ch["start"] + idx
            out_lines.append(json.dumps(row, separators=(",", ":")))
    results_path.write_text("\n".join(out_lines) + "\n")

    wall_s = round(time.monotonic() - t0, 2)
    g_per_hr = round(len(out_lines) / (wall_s / 3600.0), 1) if wall_s > 0 else 0.0
    print(json.dumps({
        "cmd": "run",
        "dir": str(run_dir),
        "games": len(out_lines),
        "wall_s": wall_s,
        "games_per_hour": g_per_hr,
        **dispatch,
    }))


# ---------------------------------------------------------------------------
# Subcommand: analyze (Zero-Bias Bare-vs-B9 Dominance & Elimination)
# ---------------------------------------------------------------------------

def cmd_analyze(args: argparse.Namespace) -> None:
    t0 = time.monotonic()
    work_dir: Path = args.work_dir
    audit = json.loads((work_dir / "stage0-audit.json").read_text())
    shell_by_rep = {s["rep_id"]: s for s in audit["shells"]}
    cat_entries, _ = load_full_catalog()
    cat_by_id = {e["id"]: e for e in cat_entries}
    gt19_shells = {cat_by_id[t]["shell_hash"] for t in GT19_IDS if t in cat_by_id}

    # 1. Load B9 x B9 bare matrix
    b9_raw: dict[tuple[str, str, int], list[float]] = defaultdict(list)
    b9_res = work_dir / "b9-matrix/results.jsonl"
    for line in b9_res.open():
        if not line.strip():
            continue
        r = json.loads(line)
        c = r["cell"]
        b9_raw[(c["a"], c["b"], int(c["rep"]))].append(seat_score(r["outcome"], int(r["rowSeat"])))

    # Accumulate pair scores: pairs[row_id][basis_id] -> list of pair scores
    pairs: dict[str, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    b9_games = 0
    for (a, b, _rep), scs in b9_raw.items():
        if len(scs) == 2:
            ps = 0.5 * (scs[0] + scs[1])
            pairs[a][b].append(ps)
            pairs[b][a].append(1.0 - ps)
            b9_games += 2

    # 2. Load completed racing rounds (also track last round separately as fresh holdout!)
    race_games = 0
    rounds_loaded: list[int] = []
    last_round = max(args.rounds) if args.rounds else None
    fresh_pairs: dict[str, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    for rd in args.rounds:
        rf = work_dir / f"round-{rd}/results.jsonl"
        if not rf.exists():
            continue
        rounds_loaded.append(rd)
        rd_raw: dict[tuple[str, str, int], list[float]] = defaultdict(list)
        for line in rf.open():
            if not line.strip():
                continue
            r = json.loads(line)
            c = r["cell"]
            rd_raw[(c["candidate"], c["basis"], int(c["rep"]))].append(
                seat_score(r["outcome"], int(r["rowSeat"]))
            )
        for (cid, bid, _rep), scs in rd_raw.items():
            if len(scs) == 2:
                ps = 0.5 * (scs[0] + scs[1])
                pairs[cid][bid].append(ps)
                if rd == last_round:
                    fresh_pairs[cid][bid].append(ps)
                race_games += 2

    # 3. Compute exact B9 x B9 matrix & RM+ Nash equilibrium on B9
    n_b9 = len(B9_IDS)
    b9_mat = [[0.5] * n_b9 for _ in range(n_b9)]
    b9_se = [[0.0] * n_b9 for _ in range(n_b9)]
    for i, bi in enumerate(B9_IDS):
        for j, bj in enumerate(B9_IDS):
            if i == j:
                b9_mat[i][j] = 0.5
                b9_se[i][j] = 0.0
            else:
                m, se = pair_cell_stats(pairs[bi][bj])
                b9_mat[i][j] = m
                b9_se[i][j] = se

    b9_nash_weights = STATS.solve_symmetric_nash(b9_mat, iters=50_000)

    # 4. Evaluate every tested shell against B9 using ZERO-BIAS dominator basis D = B9!
    tested_reps = sorted(pairs.keys())
    rows: list[dict[str, Any]] = []
    for rep in tested_reps:
        is_b9 = rep in B9_IDS
        axis_lists: list[list[float]] = []
        cand_m: list[float] = []
        cand_se: list[float] = []
        for bj in B9_IDS:
            if rep == bj:
                cand_m.append(0.5)
                cand_se.append(0.0)
                axis_lists.append([0.5] * 12)
            else:
                xs = pairs[rep][bj]
                m, se = pair_cell_stats(xs)
                cand_m.append(m)
                cand_se.append(se)
                axis_lists.append(xs)

        mean, se_all, u95_t, u95_b = stratified_mean_u95(axis_lists)
        # Payoff against B9's bare Nash mixture
        vs_b9_nash = sum(b9_nash_weights[j] * cand_m[j] for j in range(n_b9))
        vs_b9_nash_se = math.sqrt(sum((b9_nash_weights[j] ** 2) * (cand_se[j] ** 2) for j in range(n_b9)))
        vs_b9_nash_u95 = min(1.0, vs_b9_nash + 1.645 * vs_b9_nash_se)

        # Exclude self from D when rep is in B9 so basis members are evaluated fairly too
        dom_idx = [i for i, bi in enumerate(B9_IDS) if bi != rep]
        dom = solve_convex_hull_dominance(
            cand_m,
            cand_se,
            [b9_mat[i] for i in dom_idx],
            [b9_se[i] for i in dom_idx],
            [B9_IDS[i] for i in dom_idx],
            z=1.645,
        )

        # Empirical-Bayes shrunken convex-hull dominance (prevents 2-pair single-axis luck
        # from hiding weak candidates under min_k over 9 axes; vanishes as n_per_axis grows).
        n_per_axis = 12.0 if is_b9 else (sum(len(pairs[rep][bj]) for bj in B9_IDS if bj != rep) / float(n_b9))
        m0 = args.eb_pseudo_pairs
        eb_m = [(n_per_axis * cand_m[k] + m0 * mean) / (n_per_axis + m0) for k in range(n_b9)]
        eb_dom = solve_convex_hull_dominance(
            eb_m,
            cand_se,
            [b9_mat[i] for i in dom_idx],
            [b9_se[i] for i in dom_idx],
            [B9_IDS[i] for i in dom_idx],
            z=1.645,
        )

        # Count how many B9 pillars this shell beats (> 0.50) and statistically credible RPS spikes
        n_gt50 = sum(1 for k, m in enumerate(cand_m) if (B9_IDS[k] != rep) and m > 0.50)
        max_b = max(m for k, m in enumerate(cand_m) if B9_IDS[k] != rep)
        max_b_l95 = max(
            max(0.0, cand_m[k] - 1.96 * cand_se[k])
            for k in range(n_b9)
            if B9_IDS[k] != rep
        )
        max_b_u95 = max(
            min(1.0, cand_m[k] + 1.96 * cand_se[k])
            for k in range(n_b9)
            if B9_IDS[k] != rep
        )
        n_pairs_tot = sum(len(pairs[rep][bj]) for bj in B9_IDS if bj != rep)
        s_meta = shell_by_rep.get(rep, {})
        sh = s_meta.get("shell_hash") or cat_by_id[rep]["shell_hash"]

        # Fresh holdout metrics from the latest round (zero Winner's Curse vs earlier selection rounds)
        if is_b9:
            fresh_mean = mean
            fresh_vs_nash = vs_b9_nash
            fresh_n_pairs = n_pairs_tot
        else:
            f_lists = [fresh_pairs[rep][bj] for bj in B9_IDS]
            fresh_n_pairs = sum(len(x) for x in f_lists)
            if fresh_n_pairs > 0:
                f_means = [sum(x) / len(x) if x else 0.5 for x in f_lists]
                fresh_mean = sum(f_means) / float(n_b9)
                fresh_vs_nash = sum(b9_nash_weights[k] * f_means[k] for k in range(n_b9))
            else:
                fresh_mean = 0.0
                fresh_vs_nash = 0.0

        # RPS spike protection: only triggers on statistically credible spikes (l95 >= cut_rps_l95),
        # so random 2-pair 3-1 wins across 9 anchors do not prevent cutting clearly weak shells.
        rps_protected = max_b_l95 >= args.cut_rps_l95
        if args.keep_mean is not None or args.keep_vs_nash is not None or args.keep_eb_gap is not None:
            # Explicit keep-union filter (optionally requiring fresh holdout confirmation)
            keep_cond = is_b9 or (rps_protected and args.fresh_mean is None and args.fresh_vs_nash is None)
            if args.keep_mean is not None and mean + 1e-9 >= args.keep_mean:
                if args.fresh_mean is None or fresh_mean + 1e-9 >= args.fresh_mean:
                    keep_cond = True
            if args.keep_vs_nash is not None and vs_b9_nash + 1e-9 >= args.keep_vs_nash:
                if args.fresh_vs_nash is None or fresh_vs_nash + 1e-9 >= args.fresh_vs_nash:
                    keep_cond = True
            if args.keep_eb_gap is not None and eb_dom["pt_gap"] <= args.keep_eb_gap + 1e-9:
                if args.fresh_mean is None or fresh_mean + 1e-9 >= args.fresh_mean:
                    keep_cond = True
            eliminated = not keep_cond
        else:
            eliminated = (not is_b9) and (not rps_protected) and (
                dom["z_gap"] > 0.0
                or (
                    eb_dom["pt_gap"] > args.cut_eb_gap
                    and u95_t < args.cut_u95
                    and vs_b9_nash_u95 < args.cut_nash_u95
                )
            )

        rows.append({
            "shell_hash": sh,
            "rep_id": rep,
            "variants": s_meta.get("variants", [rep]),
            "n_variants": len(s_meta.get("variants", [rep])),
            "species": cat_by_id[rep]["species"],
            "is_b9": is_b9,
            "is_gt19": sh in gt19_shells,
            "n_pairs": n_pairs_tot,
            "fresh_n_pairs": fresh_n_pairs,
            "mean": round(mean, 4),
            "fresh_mean": round(fresh_mean, 4),
            "se": round(se_all, 4),
            "u95_t": round(u95_t, 4),
            "u95_bet": round(u95_b, 4),
            "vs_b9_nash": round(vs_b9_nash, 4),
            "fresh_vs_b9_nash": round(fresh_vs_nash, 4),
            "vs_b9_nash_u95": round(vs_b9_nash_u95, 4),
            "max_basis": round(max_b, 4),
            "max_basis_l95": round(max_b_l95, 4),
            "max_basis_u95": round(max_b_u95, 4),
            "n_pillars_gt50": n_gt50,
            "per_basis": {B9_IDS[k]: round(cand_m[k], 4) for k in range(n_b9)},
            "dominance": dom,
            "eb_dominance": eb_dom,
            "rps_protected": rps_protected,
            "status": "eliminated" if eliminated else "survive",
        })

    rows.sort(key=lambda r: (r["status"] != "survive", r["eb_dominance"]["pt_gap"], -r["mean"]))
    survivors = [r for r in rows if r["status"] == "survive"]
    eliminated = [r for r in rows if r["status"] == "eliminated"]

    out_path: Path = args.out or (work_dir / f"elim-report-{args.mode}.json")
    report = {
        "mode": args.mode,
        "rounds_loaded": rounds_loaded,
        "b9_games": b9_games,
        "race_games": race_games,
        "b9_ids": B9_IDS,
        "b9_matrix": [[round(x, 4) for x in row] for row in b9_mat],
        "b9_nash": {B9_IDS[i]: round(b9_nash_weights[i], 4) for i in range(n_b9)},
        "thresholds": {
            "cut_eb_gap": args.cut_eb_gap,
            "cut_u95": args.cut_u95,
            "cut_nash_u95": args.cut_nash_u95,
            "cut_rps_l95": args.cut_rps_l95,
            "eb_pseudo_pairs": args.eb_pseudo_pairs,
            "keep_mean": args.keep_mean,
            "keep_vs_nash": args.keep_vs_nash,
            "keep_eb_gap": args.keep_eb_gap,
        },
        "counts": {
            "tested": len(rows),
            "survivors": len(survivors),
            "eliminated": len(eliminated),
            "gt19_tested": sum(1 for r in rows if r["is_gt19"]),
            "gt19_survived": sum(1 for r in survivors if r["is_gt19"]),
            "gt19_lost": [r["rep_id"] for r in eliminated if r["is_gt19"]],
        },
        "survivors": survivors,
        "eliminated": eliminated,
        "elapsed_s": round(time.monotonic() - t0, 2),
    }
    out_path.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({
        "cmd": "analyze",
        "out": str(out_path),
        "mode": args.mode,
        "rounds": rounds_loaded,
        "b9_games": b9_games,
        "race_games": race_games,
        **report["counts"],
        "b9_nash_nonzero": {k: v for k, v in report["b9_nash"].items() if v >= 0.01},
    }))


# ---------------------------------------------------------------------------
# Subcommand: sanity-audit (Hitchhiker / Theme / Dup Pruning + Borderline Rescue)
# ---------------------------------------------------------------------------

STRICT_NFE_MEME_SPECIES = {
    "Ivysaur", "Voltorb", "Koffing", "Abra", "Gastly", "Clefairy",
    "Nidorino", "Nidorina", "Eevee", "Venonat", "Ledyba", "Spinarak",
}


def _parse_pick_indices(inp: str) -> list[int]:
    return [int(x) - 1 for x in re.findall(r"\d+", inp or "")]


def _genderless_signature(sets: list[dict[str, Any]]) -> tuple[tuple[str, int, str, tuple[str, ...]], ...]:
    return tuple(sorted(
        (
            str(s["species"]),
            int(s["level"]),
            str(s.get("item") or ""),
            tuple(sorted(str(m) for m in s.get("moves", []))),
        )
        for s in sets
    ))


def _collect_pick_rates(work_dir: Path) -> tuple[dict[str, list[int]], Counter[str]]:
    picks: dict[str, list[int]] = defaultdict(lambda: [0] * 6)
    games: Counter[str] = Counter()
    sources = [
        (work_dir / "b9-matrix/results.jsonl", False),
        (work_dir / "round-1/results.jsonl", False),
        (work_dir / "round-2/results.jsonl", False),
        (work_dir / "round-3/results.jsonl", False),
        (ROOT / "target/nash-work/phase1-cut/stage1a/results.jsonl", True),
        (ROOT / "target/nash-work/phase1-cut/stage1b/results.jsonl", True),
        (ROOT / "target/nash-work/phase1-cut/step2a/results.jsonl", True),
        (ROOT / "target/nash-work/phase1-cut/step2b/results.jsonl", True),
    ]
    for fp, is_p1 in sources:
        if not fp.exists():
            continue
        for line in fp.open():
            if not line.strip():
                continue
            r = json.loads(line)
            sides = [int(r["rowSeat"])] if is_p1 else [0, 1]
            prev = r.get("preview", [])
            for side in sides:
                tid = r["ids"][side]
                games[tid] += 1
                if side < len(prev) and prev[side]:
                    for idx in _parse_pick_indices(prev[side].get("input", "")):
                        if 0 <= idx < 6:
                            picks[tid][idx] += 1
    return picks, games


def cmd_sanity_audit(args: argparse.Namespace) -> None:
    t0 = time.monotonic()
    work_dir: Path = args.work_dir
    core_path: Path = args.core_report or (work_dir / "prod-core.json")
    core_doc = json.loads(core_path.read_text())
    _, all_sets = load_full_catalog()
    picks, games = _collect_pick_rates(work_dir)

    raw_surv_ids = {r["rep_id"] for r in core_doc["survivors"]}
    all_r3 = [
        r for r in (core_doc["survivors"] + core_doc["eliminated"])
        if r["is_b9"] or r["n_pairs"] >= 72
    ]

    # 1. Exact gender-only duplicate collapse (same species/level/item/moves across all 6 mons)
    by_gsig: dict[Any, list[dict[str, Any]]] = defaultdict(list)
    for r in all_r3:
        by_gsig[_genderless_signature(all_sets[r["rep_id"]])].append(r)
    gender_dups: dict[str, str] = {}
    for grp in by_gsig.values():
        if len(grp) > 1:
            ordered = sorted(grp, key=lambda x: (not x["is_b9"], "__" in x["rep_id"], -x["mean"]))
            canon = ordered[0]["rep_id"]
            for dup in ordered[1:]:
                gender_dups[dup["rep_id"]] = canon

    clean_survivors: list[dict[str, Any]] = []
    dropped_sanity: list[dict[str, Any]] = []
    rescued: list[dict[str, Any]] = []
    watch_low_pick: list[dict[str, Any]] = []

    for r in all_r3:
        tid = r["rep_id"]
        sets = all_sets[tid]
        tot = games[tid]
        rates = [round(picks[tid][i] / tot, 4) if tot > 0 else 0.0 for i in range(6)]
        min_r = min(rates)
        n_zero = sum(1 for x in rates if x < 0.01)

        flags: list[str] = []
        if tid in gender_dups:
            flags.append(f"gender_only_duplicate_of:{gender_dups[tid]}")
        if tid.startswith("mjj-type-"):
            flags.append("theme_monotype_qa")
        if tid.startswith("offprior-") and n_zero > 0:
            flags.append("synth_offprior_dead_hitchhiker")
        nfe_hits = [s["species"] for s in sets if s["species"] in STRICT_NFE_MEME_SPECIES]
        if nfe_hits:
            flags.append(f"nfe_hitchhiker:{','.join(nfe_hits)}")
        if (not r["is_b9"]) and n_zero > 0 and r["mean"] < args.dead_slot_wr_floor:
            flags.append("dead_slot_low_overall_wr")

        slot_summary = [
            {
                "species": sets[i]["species"],
                "level": int(sets[i]["level"]),
                "pick_rate": rates[i],
            }
            for i in range(6)
        ]
        enriched = dict(r)
        enriched["pick_games"] = tot
        enriched["min_pick_rate"] = min_r
        enriched["zero_pick_slots"] = n_zero
        enriched["slot_picks"] = slot_summary
        enriched["sanity_flags"] = flags

        in_raw = tid in raw_surv_ids
        is_clean = len(flags) == 0

        qualifies_rescue = (
            is_clean
            and (min_r + 1e-9 >= args.rescue_min_pick)
            and (
                (r["mean"] + 1e-9 >= args.rescue_mean and r["fresh_mean"] + 1e-9 >= args.rescue_fresh_mean)
                or (
                    r["vs_b9_nash"] + 1e-9 >= args.rescue_vs_nash
                    and r["fresh_vs_b9_nash"] + 1e-9 >= args.rescue_fresh_vs_nash
                )
                or (
                    r["fresh_mean"] + 1e-9 >= args.rescue_alt_fresh_mean
                    and r["fresh_vs_b9_nash"] + 1e-9 >= args.rescue_alt_fresh_vs_nash
                )
            )
        )

        if in_raw and not is_clean:
            enriched["sanity_action"] = "dropped_from_core"
            dropped_sanity.append(enriched)
        elif in_raw and is_clean:
            enriched["sanity_action"] = "kept_core"
            clean_survivors.append(enriched)
            if min_r < 0.05:
                watch_low_pick.append({
                    "rep_id": tid,
                    "is_b9": r["is_b9"],
                    "low_slots": [s for s in slot_summary if s["pick_rate"] < 0.05],
                })
        elif (not in_raw) and qualifies_rescue:
            enriched["sanity_action"] = "rescued_borderline"
            clean_survivors.append(enriched)
            rescued.append(enriched)

    clean_survivors.sort(key=lambda x: (not x["is_b9"], -max(x["mean"], x["vs_b9_nash"]), -x["fresh_mean"]))
    out_path: Path = args.out or (work_dir / "prod-core-clean.json")
    out_doc = {
        "source_core": str(core_path),
        "b9_ids": core_doc["b9_ids"],
        "b9_nash": core_doc["b9_nash"],
        "rules": {
            "dead_slot_wr_floor": args.dead_slot_wr_floor,
            "rescue_min_pick": args.rescue_min_pick,
            "rescue_mean": args.rescue_mean,
            "rescue_fresh_mean": args.rescue_fresh_mean,
            "rescue_vs_nash": args.rescue_vs_nash,
            "rescue_fresh_vs_nash": args.rescue_fresh_vs_nash,
            "rescue_alt_fresh_mean": args.rescue_alt_fresh_mean,
            "rescue_alt_fresh_vs_nash": args.rescue_alt_fresh_vs_nash,
        },
        "counts": {
            "raw_core": len(raw_surv_ids),
            "dropped_sanity": len(dropped_sanity),
            "rescued_borderline": len(rescued),
            "final_clean_core": len(clean_survivors),
            "pairs": len(clean_survivors) * (len(clean_survivors) - 1) // 2,
            "watch_low_pick_utility": len(watch_low_pick),
        },
        "survivors": clean_survivors,
        "dropped_sanity": dropped_sanity,
        "rescued": rescued,
        "watch_low_pick": watch_low_pick,
        "elapsed_s": round(time.monotonic() - t0, 2),
    }
    out_path.write_text(json.dumps(out_doc, indent=2) + "\n")
    print(json.dumps({
        "cmd": "sanity-audit",
        "out": str(out_path),
        **out_doc["counts"],
        "dropped_ids": [x["rep_id"] for x in dropped_sanity],
        "rescued_ids": [x["rep_id"] for x in rescued],
    }))


# ---------------------------------------------------------------------------
# Subcommand: self-test
# ---------------------------------------------------------------------------

def cmd_self_test(_args: argparse.Namespace) -> None:
    # 1. Convex-hull dominance sanity: a dominated point has positive gap; a vertex has <= 0
    dom_m = [[0.8, 0.2], [0.2, 0.8]]
    dom_se = [[0.02, 0.02], [0.02, 0.02]]
    res_mid = solve_convex_hull_dominance([0.3, 0.3], [0.02, 0.02], dom_m, dom_se, ["A", "B"])
    assert res_mid["pt_gap"] > 0.19, f"expected ~0.20 gap, got {res_mid}"
    assert res_mid["z_gap"] > 0.14, f"expected positive z_gap, got {res_mid}"

    res_rps = solve_convex_hull_dominance([0.9, 0.1], [0.02, 0.02], dom_m, dom_se, ["A", "B"])
    assert res_rps["pt_gap"] < 0.0, f"RPS specialist must not be dominated, got {res_rps}"

    # 2. Verify B9 all exist and have pairwise distinct shells
    cat_entries, all_sets = load_full_catalog()
    cat_by_id = {e["id"]: e for e in cat_entries}
    shells = {cat_by_id[t]["shell_hash"] for t in B9_IDS}
    assert len(shells) == len(B9_IDS) == 9
    for t in B9_IDS:
        assert t in all_sets

    print(json.dumps({"cmd": "self-test", "ok": True, "b9_shells": len(shells)}))


def main() -> None:
    ap = argparse.ArgumentParser(description="Elimination-first Axis-A Nash search driver")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_st = sub.add_parser("self-test")
    p_st.set_defaults(func=cmd_self_test)

    p_au = sub.add_parser("audit-existing")
    p_au.add_argument("--work-dir", type=Path, default=DEFAULT_WORK)
    p_au.set_defaults(func=cmd_audit_existing)

    p_pb = sub.add_parser("prepare-basis")
    p_pb.add_argument("--work-dir", type=Path, default=DEFAULT_WORK)
    p_pb.add_argument("--pairs-per-cell", type=int, default=12)
    p_pb.add_argument("--preview-iters", type=int, default=81000)
    p_pb.add_argument("--battle-iters", type=int, default=9000)
    p_pb.add_argument("--battle-mode", choices=["fixed", "adaptive"], default="fixed")
    p_pb.set_defaults(func=cmd_prepare_basis)

    p_pr = sub.add_parser("prepare-round")
    p_pr.add_argument("--work-dir", type=Path, default=DEFAULT_WORK)
    p_pr.add_argument("--round", type=int, required=True)
    p_pr.add_argument("--shells-file", type=Path, required=True)
    p_pr.add_argument("--rep-start", type=int, required=True)
    p_pr.add_argument("--rep-end", type=int, required=True)
    p_pr.add_argument("--preview-iters", type=int, default=81000)
    p_pr.add_argument("--battle-iters", type=int, default=9000)
    p_pr.add_argument("--battle-mode", choices=["fixed", "adaptive"], default="fixed")
    p_pr.set_defaults(func=cmd_prepare_round)

    p_rn = sub.add_parser("run")
    p_rn.add_argument("--dir", type=Path, required=True)
    p_rn.add_argument("--binary", type=Path, default=DEFAULT_BIN)
    p_rn.add_argument("--chunk-games", type=int, default=120)
    p_rn.add_argument("--local-workers", type=int, default=10)
    p_rn.add_argument("--mini2-workers", type=int, default=12)
    p_rn.add_argument("--mini2", action="store_true", default=False)
    p_rn.set_defaults(func=cmd_run)

    p_an = sub.add_parser("analyze")
    p_an.add_argument("--work-dir", type=Path, default=DEFAULT_WORK)
    p_an.add_argument("--rounds", type=int, nargs="*", default=[1])
    p_an.add_argument("--mode", choices=["round1", "round2", "round3"], default="round1")
    p_an.add_argument("--cut-eb-gap", type=float, default=0.04)
    p_an.add_argument("--cut-u95", type=float, default=0.55)
    p_an.add_argument("--cut-nash-u95", type=float, default=0.65)
    p_an.add_argument("--cut-rps-l95", type=float, default=0.52)
    p_an.add_argument("--eb-pseudo-pairs", type=float, default=4.0)
    p_an.add_argument("--keep-mean", type=float, default=None)
    p_an.add_argument("--keep-vs-nash", type=float, default=None)
    p_an.add_argument("--keep-eb-gap", type=float, default=None)
    p_an.add_argument("--fresh-mean", type=float, default=None)
    p_an.add_argument("--fresh-vs-nash", type=float, default=None)
    p_an.add_argument("--out", type=Path, default=None)
    p_an.set_defaults(func=cmd_analyze)

    p_sa = sub.add_parser("sanity-audit")
    p_sa.add_argument("--work-dir", type=Path, default=DEFAULT_WORK)
    p_sa.add_argument("--core-report", type=Path, default=None)
    p_sa.add_argument("--dead-slot-wr-floor", type=float, default=0.45)
    p_sa.add_argument("--rescue-min-pick", type=float, default=0.03)
    p_sa.add_argument("--rescue-mean", type=float, default=0.535)
    p_sa.add_argument("--rescue-fresh-mean", type=float, default=0.51)
    p_sa.add_argument("--rescue-vs-nash", type=float, default=0.55)
    p_sa.add_argument("--rescue-fresh-vs-nash", type=float, default=0.52)
    p_sa.add_argument("--rescue-alt-fresh-mean", type=float, default=0.56)
    p_sa.add_argument("--rescue-alt-fresh-vs-nash", type=float, default=0.55)
    p_sa.add_argument("--out", type=Path, default=None)
    p_sa.set_defaults(func=cmd_sanity_audit)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
