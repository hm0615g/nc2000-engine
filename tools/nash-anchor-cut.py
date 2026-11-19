#!/usr/bin/env python3
"""Phase 1: Conservative Fixed 5-Anchor Cut for eliminating provably non-competitive teams.

Eliminates ONLY teams/shells that fail ALL THREE conservative safety conditions at 9k/81k:
  (a) Overall 5-anchor 30-pair (60-game) one-sided 95% upper bound < 0.35
      (conservative max of stratified t-bound, bootstrap 95%, and exact binomial Clopper-Pearson)
  (b) Every individual anchor's 6-pair (12-game) one-sided 95% upper bound < 0.55
      (protects RPS specialists that counter even a single macro-archetype)
  (c) Shell-level protection: if ANY variant sharing `shell_hash` survives, ALL variants
      in that shell survive (protects camouflage-only partner variants like Deme g3-c4).

Stage 1a: All 661 non-dummy teams x 5 anchors x 2 seat pairs (10 pairs = 20 games/team).
          Teams with overall >= 40% OR >= 3/4 wins on any anchor pass immediately.
Stage 1b: Remaining teams play +4 seat pairs per anchor (cumulative 30 pairs = 60 games/team),
          then the 3-way cut rule is applied with zero peeking bias.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import math
import os
import random
import shutil
import subprocess
import tarfile
import time
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent

ANCHORS: list[tuple[str, list[tuple[str, float]]]] = [
    (
        "anchor:APEX4",
        [
            ("apex-c04-bug-3atkstar-boomgen", 0.40),
            ("red-v10-s1105-r2-s0142", 0.30),
            ("apex-c08-surgical-142", 0.15),
            ("red-v10-s1105-r1-s0148", 0.15),
        ],
    ),
    (
        "anchor:Sample07",
        [
            ("sample-07", 0.50),
            ("sample-07-beq-g2-c4", 0.50),
        ],
    ),
    (
        "anchor:D5",
        [
            ("deme-r1-00-s0030", 0.20),
            ("deme-r1-00-s0030-beq-g2-c3-legal", 0.20),
            ("deme-r1-00-s0030-beq-g2-c3-beq-g3-c0-legal", 0.20),
            ("deme-r1-00-s0030-beq-g2-c3-beq-g3-c4-legal", 0.20),
            ("deme-r1-00-s0030-beq-g2-c3-beq-g3-c5-legal", 0.20),
        ],
    ),
    (
        "anchor:s0155",
        [
            ("red-v10-s1105-r2-s0155", 1.00),
        ],
    ),
    (
        "anchor:sample06",
        [
            ("sample-06", 0.25),
            ("red-v10-s1105-r1-s0082", 0.25),
            ("red-v10-s1105-r1-s0146", 0.25),
            ("red-v10-s1105-r1-s0199", 0.25),
        ],
    ),
]


def u32_hash(tag: str) -> int:
    return int.from_bytes(hashlib.sha256(tag.encode("utf-8")).digest()[:4], "big")


def draw_variant(members: list[tuple[str, float]], tag: str) -> str:
    """Deterministic independent i.i.d. draw from exact conditional weights q_b."""
    u = int.from_bytes(hashlib.sha256(tag.encode("utf-8")).digest()[:8], "big") / float(1 << 64)
    acc = 0.0
    for tid, w in members:
        acc += w
        if u < acc:
            return tid
    return members[-1][0]


def seat_score(outcome: str, seat: int) -> float:
    if outcome in ("tie", "cap"):
        return 0.5
    win_seat = 0 if outcome == "p1" else 1
    return 1.0 if win_seat == seat else 0.0


def clopper_pearson_upper(k: float, n: int, alpha: float = 0.05) -> float:
    """Exact one-sided (1-alpha) binomial upper bound for k wins in n trials (ceil k for half-wins)."""
    ki = int(math.ceil(k - 1e-12))
    if ki >= n:
        return 1.0
    # Solve P(Bin(n, p) <= ki) = alpha via bisection
    lo, hi = ki / n, 1.0
    for _ in range(48):
        mid = 0.5 * (lo + hi)
        # CDF P(X <= ki)
        cdf = 0.0
        for i in range(ki + 1):
            cdf += math.comb(n, i) * (mid ** i) * ((1.0 - mid) ** (n - i))
        if cdf > alpha:
            lo = mid
        else:
            hi = mid
    return hi


def conservative_upper_95(pair_scores: list[float], n_games: int, rng: random.Random) -> float:
    """Conservative one-sided 95% upper bound: max of Student-t, empirical bootstrap, and Clopper-Pearson."""
    n = len(pair_scores)
    if n == 0:
        return 1.0
    mean = sum(pair_scores) / n
    wins = sum(pair_scores) * 2.0  # exact game-point sum out of n_games = 2*n
    cp = clopper_pearson_upper(wins, n_games, alpha=0.05)
    if n == 1:
        return max(cp, 1.0)
    var = sum((x - mean) ** 2 for x in pair_scores) / (n - 1)
    se = math.sqrt(var / n)
    # Exact one-sided 95% t critical values (df = 5 -> 2.015, df = 29 -> 1.699)
    t_crit = 2.015 if n <= 6 else (1.833 if n <= 10 else 1.70)
    t_upper = min(1.0, mean + t_crit * se)
    # Bootstrap 95th percentile
    boots = sorted(
        sum(pair_scores[rng.randrange(n)] for _ in range(n)) / n
        for _ in range(2000)
    )
    b_upper = boots[int(0.95 * len(boots))]
    return min(1.0, max(cp, t_upper, b_upper))


def run_cmd(cmd: list[str], *, env: dict[str, str] | None = None, stdin_text: str | None = None) -> str:
    res = subprocess.run(cmd, env=env, input=stdin_text, capture_output=True, text=True, check=True)
    return res.stdout.strip()


def stop_colab_sessions(sessions: list[str]) -> None:
    for s in sessions:
        subprocess.run(["colab", "--auth", "adc", "stop", "-s", s], capture_output=True, text=True, check=False)


def build_schedule(
    candidates: list[str],
    rep_start: int,
    rep_end: int,
    seed_tag: str,
) -> list[dict[str, Any]]:
    """Generate seat-swapped i.i.d. schedule for `candidates` against all 5 anchors for reps [rep_start, rep_end)."""
    rows: list[dict[str, Any]] = []
    blk = 0
    for cid in candidates:
        for aname, members in ANCHORS:
            for rep in range(rep_start, rep_end):
                tag = f"nc2000/p1cut/{seed_tag}/{cid}/{aname}/r{rep}"
                aid = draw_variant(members, f"{tag}/draw")
                sc = u32_hash(f"{tag}/ch")
                sa = u32_hash(f"{tag}/an")
                bs = u32_hash(f"{tag}/battle")
                for row_seat in (0, 1):
                    if row_seat == 0:
                        ids, seeds, mix = [cid, aid], [sc, sa], [f"s:{cid}", aname]
                    else:
                        ids, seeds, mix = [aid, cid], [sa, sc], [aname, f"s:{cid}"]
                    rows.append({
                        "block": blk,
                        "cell": {"candidate": cid, "anchor": aname, "anchor_id": aid, "rep": rep},
                        "rowSeat": row_seat,
                        "ids": ids,
                        "seeds": seeds,
                        "battleSeed": bs,
                        "mixtures": mix,
                    })
                blk += 1
    return rows


def split_pairs(lines: list[str], fracs: list[float], out_files: list[Path]) -> list[int]:
    assert len(lines) % 2 == 0
    n_pairs = len(lines) // 2
    tot = sum(fracs)
    used = 0
    counts = []
    for i, f in enumerate(fracs):
        c = (n_pairs - used) if i == len(fracs) - 1 else max(0, min(int(round(n_pairs * f / tot)), n_pairs - used))
        counts.append(c)
        used += c
    cur = 0
    out_counts = []
    for c, path in zip(counts, out_files):
        chunk = lines[cur : cur + 2 * c]
        cur += 2 * c
        path.write_text("\n".join(chunk) + ("\n" if chunk else ""))
        out_counts.append(len(chunk))
    return out_counts


def run_local_shard(
    binary: Path, pools: Path, shard: Path, out_dir: Path, workers: int, preview_iters: int, battle_iters: int
) -> dict[str, Any]:
    if out_dir.exists():
        shutil.rmtree(out_dir)
    env = os.environ.copy()
    env["NASH_POOLS"] = str(pools)
    env["NASH_PREVIEW_ITERS"] = str(preview_iters)
    env["NASH_BATTLE_ITERS"] = str(battle_iters)
    env["NASH_BATTLE_MODE"] = "fixed"
    env["NASH_PREVIEW_CACHE"] = "1"
    out = run_cmd([str(binary), str(shard), str(out_dir), str(workers)], env=env)
    return json.loads(out.splitlines()[-1])


def run_colab_shard(
    session: str, shard: Path, out_dir: Path, tag: str, preview_iters: int, battle_iters: int, workers: int
) -> dict[str, Any]:
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    r_sched = f"/tmp/p1-{tag}-{session}.jsonl"
    r_out = f"/tmp/p1-out-{tag}-{session}"
    r_tar = f"/tmp/p1-out-{tag}-{session}.tar.gz"
    l_tar = out_dir.parent / f"colab-{tag}-{session}.tar.gz"

    run_cmd(["colab", "--auth", "adc", "upload", "-s", session, str(shard), r_sched])
    script = f"""
import json, subprocess, time
t0 = time.time()
cmd = r'''
set -euo pipefail
rm -rf {r_out} {r_tar}
NASH_POOLS=/tmp/p1-pools.json NASH_PREVIEW_ITERS={preview_iters} NASH_BATTLE_ITERS={battle_iters} NASH_BATTLE_MODE=fixed NASH_PREVIEW_CACHE=1 \\
  /content/nc2000/target/native-pgo/release/examples/nash_refresh {r_sched} {r_out} {workers} > /tmp/p1-run.json
tar -czf {r_tar} -C {r_out} .
'''
subprocess.run(["bash", "-c", cmd], check=True)
meta = json.loads(open("/tmp/p1-run.json").read().strip())
meta["session"] = {session!r}
meta["total_s"] = round(time.time() - t0, 1)
print(json.dumps(meta))
"""
    stdout = run_cmd(["colab", "--auth", "adc", "exec", "-s", session, "--timeout", "7200"], stdin_text=script)
    run_cmd(["colab", "--auth", "adc", "download", "-s", session, r_tar, str(l_tar)])
    with tarfile.open(l_tar, "r:gz") as tf:
        tf.extractall(out_dir, filter="data")
    l_tar.unlink(missing_ok=True)
    for line in reversed(stdout.splitlines()):
        line = line.strip()
        if line.startswith("{") and line.endswith("}"):
            return json.loads(line)
    return {"session": session}


def execute_stage(
    stage_dir: Path,
    tag: str,
    schedule_rows: list[dict[str, Any]],
    binary: Path,
    pools: Path,
    local_workers: int,
    colab_sessions: list[str],
    colab_workers: int,
    local_weight: float,
    colab_weight: float,
    preview_iters: int,
    battle_iters: int,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    stage_dir.mkdir(parents=True, exist_ok=True)
    sched_path = stage_dir / "schedule.jsonl"
    lines = [json.dumps(r) for r in schedule_rows]
    sched_path.write_text("\n".join(lines) + "\n")

    shard_dirs: list[Path] = []
    metas: list[dict[str, Any]] = []

    if not colab_sessions:
        od = stage_dir / "out-local"
        m = run_local_shard(binary, pools, sched_path, od, local_workers, preview_iters, battle_iters)
        shard_dirs.append(od)
        metas.append({"worker": "local", **m})
    else:
        fracs = [local_weight] + [colab_weight] * len(colab_sessions)
        shard_files = [stage_dir / "shard-local.jsonl"] + [stage_dir / f"shard-{s}.jsonl" for s in colab_sessions]
        counts = split_pairs(lines, fracs, shard_files)

        futs: dict[concurrent.futures.Future[Any], tuple[str, Path, Path]] = {}
        with concurrent.futures.ThreadPoolExecutor(max_workers=1 + len(colab_sessions)) as ex:
            if counts[0] > 0:
                od = stage_dir / "out-local"
                futs[ex.submit(run_local_shard, binary, pools, shard_files[0], od, local_workers, preview_iters, battle_iters)] = ("local", shard_files[0], od)
            for idx, sess in enumerate(colab_sessions, 1):
                if counts[idx] > 0:
                    od = stage_dir / f"out-{sess}"
                    futs[ex.submit(run_colab_shard, sess, shard_files[idx], od, tag, preview_iters, battle_iters, colab_workers)] = (sess, shard_files[idx], od)
            failed: list[tuple[str, Path, Path]] = []
            for fut in concurrent.futures.as_completed(futs):
                name, sf, od = futs[fut]
                try:
                    m = fut.result()
                    shard_dirs.append(od)
                    metas.append({"worker": name, **m})
                except Exception as e:
                    metas.append({"worker": name, "error": str(e)})
                    failed.append((name, sf, od))
                    if name in colab_sessions:
                        colab_sessions.remove(name)
        for name, sf, od in failed:
            od_fb = stage_dir / f"out-fb-{name}"
            m = run_local_shard(binary, pools, sf, od_fb, local_workers, preview_iters, battle_iters)
            shard_dirs.append(od_fb)
            metas.append({"worker": f"fallback-{name}", **m})

    # Collect & validate exact game count
    expected_keys = {(int(r["block"]), int(r["rowSeat"])) for r in schedule_rows}
    collected: dict[tuple[int, int], dict[str, Any]] = {}
    for d in shard_dirs:
        for p in sorted(d.glob("*.json")):
            if p.name == "manifest.json":
                continue
            r = json.loads(p.read_text())
            k = (int(r["block"]), int(r["rowSeat"]))
            if k in expected_keys:
                collected[k] = r
    missing = expected_keys - set(collected.keys())
    assert not missing, f"{tag} missing {len(missing)} games"
    ordered = [collected[(int(r["block"]), int(r["rowSeat"]))] for r in schedule_rows]
    with (stage_dir / "results.jsonl").open("w") as fh:
        for rec in ordered:
            fh.write(json.dumps(rec) + "\n")
    return ordered, metas


def aggregate_results(result_lists: list[list[dict[str, Any]]]) -> dict[str, dict[str, list[float]]]:
    """Return cand -> anchor -> list of seat-pair scores in [0, 1]."""
    by_pair: dict[tuple[str, str, int], list[float]] = defaultdict(list)
    for rows in result_lists:
        for r in rows:
            cell = r["cell"]
            key = (cell["candidate"], cell["anchor"], int(cell["rep"]))
            by_pair[key].append(seat_score(r["outcome"], int(r["rowSeat"])))
    out: dict[str, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    for (cid, aname, rep), scs in sorted(by_pair.items()):
        assert len(scs) == 2, f"incomplete seat pair {(cid, aname, rep)}"
        out[cid][aname].append(0.5 * (scs[0] + scs[1]))
    return out


def cmd_prepare(args: argparse.Namespace) -> None:
    swiss_dir = args.swiss_dir
    cat = json.loads((swiss_dir / "catalog.json").read_text())["teams"]
    swiss_pools = json.loads((swiss_dir / "pools.json").read_text())

    # Exclude ONLY verified synthetic species-coverage filler teams (`supp-cov-*`)
    excluded = [t["id"] for t in cat if t["id"].startswith("supp-cov-")]
    kept = [t for t in cat if not t["id"].startswith("supp-cov-")]

    # Build pools.json containing all teams + singleton mixtures + 5 multi-variant anchors
    mixtures = dict(swiss_pools["mixtures"])
    for aname, members in ANCHORS:
        mixtures[aname] = [{"id": tid, "weight": w} for tid, w in members]

    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "candidates.json").write_text(json.dumps({"teams": kept, "excluded_supp_cov": excluded}, indent=2))
    (args.out_dir / "pools.json").write_text(json.dumps({"teams": swiss_pools["teams"], "mixtures": mixtures}))

    shells = {t["shell_hash"] for t in kept}
    print(json.dumps({
        "out_dir": str(args.out_dir),
        "candidates": len(kept),
        "excluded_supp_cov": len(excluded),
        "distinct_shells": len(shells),
        "anchors": [a[0] for a in ANCHORS],
    }))


def cmd_run(args: argparse.Namespace) -> None:
    work_dir = args.dir
    cand_doc = json.loads((work_dir / "candidates.json").read_text())
    cand_rows = cand_doc["teams"]
    cand_ids = [t["id"] for t in cand_rows]
    shell_of = {t["id"]: t["shell_hash"] for t in cand_rows}
    meta_by_id = {t["id"]: t for t in cand_rows}
    pools = work_dir / "pools.json"
    active_colab = list(args.colab_sessions)
    t0 = time.monotonic()

    try:
        # Stage 1a: reps [0, 2) => 2 pairs/anchor = 10 pairs (20 games) per candidate
        s1a_dir = work_dir / "stage1a"
        if (s1a_dir / "results.jsonl").exists():
            s1a_res = [json.loads(l) for l in (s1a_dir / "results.jsonl").open() if l.strip()]
            s1a_meta = [{"reused": True}]
        else:
            s1a_sched = build_schedule(cand_ids, 0, args.s1a_reps, args.seed_tag)
            s1a_res, s1a_meta = execute_stage(
                s1a_dir, "s1a", s1a_sched, args.binary, pools,
                args.local_workers, active_colab, args.colab_workers,
                args.local_weight, args.colab_weight, args.preview_iters, args.battle_iters,
            )

        agg1a = aggregate_results([s1a_res])
        early_pass = []
        to_s1b = []
        for cid in cand_ids:
            amap = agg1a[cid]
            all_xs = [x for a, _ in ANCHORS for x in amap[a]]
            overall = sum(all_xs) / len(all_xs)
            max_anchor_wins = max(sum(amap[a]) * 2.0 for a, _ in ANCHORS)
            if overall >= args.early_pass_mean or max_anchor_wins >= args.early_pass_anchor_wins:
                early_pass.append(cid)
            else:
                to_s1b.append(cid)

        (work_dir / "stage1a-summary.json").write_text(json.dumps({
            "wall_s": round(time.monotonic() - t0, 1),
            "workers": s1a_meta,
            "total_candidates": len(cand_ids),
            "early_pass": len(early_pass),
            "to_stage1b": len(to_s1b),
        }, indent=2))

        # Stage 1b: reps [s1a_reps, s1a_reps + s1b_reps) => +4 pairs/anchor => 30 pairs (60 games) total
        s1b_res: list[dict[str, Any]] = []
        s1b_meta: list[dict[str, Any]] = []
        if to_s1b:
            s1b_dir = work_dir / "stage1b"
            if (s1b_dir / "results.jsonl").exists():
                s1b_res = [json.loads(l) for l in (s1b_dir / "results.jsonl").open() if l.strip()]
                s1b_meta = [{"reused": True}]
            else:
                s1b_sched = build_schedule(to_s1b, args.s1a_reps, args.s1a_reps + args.s1b_reps, args.seed_tag)
                s1b_res, s1b_meta = execute_stage(
                    s1b_dir, "s1b", s1b_sched, args.binary, pools,
                    args.local_workers, active_colab, args.colab_workers,
                    args.local_weight, args.colab_weight, args.preview_iters, args.battle_iters,
                )

        # Immediately release Colab VMs once computation ends
        if args.colab_sessions:
            stop_colab_sessions(args.colab_sessions)
            active_colab.clear()

        # Apply exact 3-way conservative cut rule
        agg_all = aggregate_results([s1a_res, s1b_res])
        rng = random.Random(20261120)
        early_pass_set = set(early_pass)

        prelim_status: dict[str, dict[str, Any]] = {}
        surviving_shells: set[str] = set()

        for cid in cand_ids:
            amap = agg_all[cid]
            all_xs = [x for a, _ in ANCHORS for x in amap[a]]
            overall_mean = sum(all_xs) / len(all_xs)
            overall_u95 = conservative_upper_95(all_xs, len(all_xs) * 2, rng)
            per_anchor = {}
            max_anchor_u95 = 0.0
            max_anchor_mean = 0.0
            for a, _ in ANCHORS:
                xs = amap[a]
                am = sum(xs) / len(xs)
                au = conservative_upper_95(xs, len(xs) * 2, rng)
                per_anchor[a] = {"pairs": len(xs), "mean": round(am, 4), "u95": round(au, 4)}
                max_anchor_u95 = max(max_anchor_u95, au)
                max_anchor_mean = max(max_anchor_mean, am)

            if cid in early_pass_set:
                individual_pass = True
                reason = "early_pass_s1a"
            else:
                # Cut ONLY if overall u95 < 0.35 AND every single anchor u95 < 0.55
                individual_pass = not (overall_u95 < args.cut_overall_u95 and max_anchor_u95 < args.cut_anchor_u95)
                reason = (
                    "cut_provably_weak"
                    if not individual_pass
                    else ("survive_rps_anchor" if overall_u95 < args.cut_overall_u95 else "survive_overall")
                )

            if individual_pass:
                surviving_shells.add(shell_of[cid])

            prelim_status[cid] = {
                "id": cid,
                "shell_hash": shell_of[cid],
                "species": meta_by_id[cid]["species"],
                "source": meta_by_id[cid]["source"],
                "pairs": len(all_xs),
                "overall_mean": round(overall_mean, 4),
                "overall_u95": round(overall_u95, 4),
                "max_anchor_mean": round(max_anchor_mean, 4),
                "max_anchor_u95": round(max_anchor_u95, 4),
                "per_anchor": per_anchor,
                "individual_pass": individual_pass,
                "reason": reason,
            }

        # Apply Condition (c): Shell-level protection
        survivors = []
        shell_rescued = []
        cut_list = []
        for cid in cand_ids:
            rec = prelim_status[cid]
            if rec["individual_pass"]:
                rec["final_status"] = "survive"
                survivors.append(rec)
            elif rec["shell_hash"] in surviving_shells:
                rec["final_status"] = "survive_shell_protected"
                rec["reason"] = "shell_protected"
                survivors.append(rec)
                shell_rescued.append(rec)
            else:
                rec["final_status"] = "cut"
                cut_list.append(rec)

        survivors.sort(key=lambda r: (-r["overall_mean"], -r["max_anchor_mean"], r["id"]))
        cut_list.sort(key=lambda r: (-r["overall_mean"], r["id"]))

        report = {
            "wall_s": round(time.monotonic() - t0, 1),
            "config": {
                "anchors": [a[0] for a in ANCHORS],
                "s1a_reps_per_anchor": args.s1a_reps,
                "s1b_reps_per_anchor": args.s1b_reps,
                "early_pass_mean": args.early_pass_mean,
                "early_pass_anchor_wins": args.early_pass_anchor_wins,
                "cut_overall_u95": args.cut_overall_u95,
                "cut_anchor_u95": args.cut_anchor_u95,
                "preview_iters": args.preview_iters,
                "battle_iters": args.battle_iters,
            },
            "stage1a_games": len(s1a_res),
            "stage1b_games": len(s1b_res),
            "stage1a_workers": s1a_meta,
            "stage1b_workers": s1b_meta,
            "counts": {
                "total_tested": len(cand_ids),
                "excluded_supp_cov": len(cand_doc["excluded_supp_cov"]),
                "early_pass_s1a": len(early_pass),
                "reviewed_s1b": len(to_s1b),
                "final_survivors": len(survivors),
                "shell_protected_rescues": len(shell_rescued),
                "final_cut": len(cut_list),
                "surviving_shells": len(surviving_shells),
                "cut_shells": len({r["shell_hash"] for r in cut_list}),
            },
            "survivors": survivors,
            "cut": cut_list,
        }
        out_path = work_dir / "phase1-report.json"
        out_path.write_text(json.dumps(report, indent=2))
        print(json.dumps({
            "ok": True,
            "report": str(out_path),
            "wall_s": report["wall_s"],
            "total_games": len(s1a_res) + len(s1b_res),
            "counts": report["counts"],
            "top10": [{"id": r["id"], "mean": r["overall_mean"], "max_a": r["max_anchor_mean"]} for r in survivors[:10]],
        }))
    finally:
        if args.colab_sessions:
            stop_colab_sessions(args.colab_sessions)


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_prep = sub.add_parser("prepare")
    p_prep.add_argument("--swiss-dir", type=Path, default=ROOT / "target/nash-work/swiss-v1")
    p_prep.add_argument("--out-dir", type=Path, default=ROOT / "target/nash-work/phase1-cut")
    p_prep.set_defaults(func=cmd_prepare)

    p_run = sub.add_parser("run")
    p_run.add_argument("--dir", type=Path, default=ROOT / "target/nash-work/phase1-cut")
    p_run.add_argument("--binary", type=Path, default=ROOT / "target/native-pgo/release/examples/nash_refresh")
    p_run.add_argument("--seed-tag", type=str, default="v1")
    p_run.add_argument("--local-workers", type=int, default=10)
    p_run.add_argument("--colab-sessions", nargs="*", default=[])
    p_run.add_argument("--colab-workers", type=int, default=8)
    p_run.add_argument("--local-weight", type=float, default=0.72)
    p_run.add_argument("--colab-weight", type=float, default=0.14)
    p_run.add_argument("--preview-iters", type=int, default=81000)
    p_run.add_argument("--battle-iters", type=int, default=9000)
    p_run.add_argument("--s1a-reps", type=int, default=2)
    p_run.add_argument("--s1b-reps", type=int, default=4)
    p_run.add_argument("--early-pass-mean", type=float, default=0.40)
    p_run.add_argument("--early-pass-anchor-wins", type=float, default=3.0)
    p_run.add_argument("--cut-overall-u95", type=float, default=0.35)
    p_run.add_argument("--cut-anchor-u95", type=float, default=0.55)
    p_run.set_defaults(func=cmd_run)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
