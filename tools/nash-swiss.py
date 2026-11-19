#!/usr/bin/env python3
"""Progressive-cut Swiss tournament engine for large disclosed-singleton catalogs.

Subcommands:
  pair     Generate round-R Swiss schedule (optionally cutting active field to top K).
  ingest   Merge local/Colab `nash_refresh` result directories into round-R results & update standings.
  report   Fit Bradley-Terry Elo across all completed rounds and emit Top-K + RPS rescue list.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import defaultdict
from pathlib import Path
from typing import Any


def u32_hash(tag: str) -> int:
    return int.from_bytes(hashlib.sha256(tag.encode("utf-8")).digest()[:4], "big")


def seat_score(outcome: str, seat: int) -> float:
    if outcome in ("tie", "cap"):
        return 0.5
    win_seat = 0 if outcome == "p1" else 1
    return 1.0 if win_seat == seat else 0.0


def fit_bradley_terry(
    team_ids: list[str],
    pair_results: list[tuple[str, str, float]],
    prior_games: float = 0.5,
    iters: int = 80,
) -> dict[str, float]:
    """Fit Bradley-Terry Elo ratings via MM updates with weak virtual-draw prior."""
    idx = {tid: i for i, tid in enumerate(team_ids)}
    n = len(team_ids)
    wins = [0.5 * prior_games] * n
    opps: list[dict[int, float]] = [defaultdict(float) for _ in range(n)]

    for a, b, sc_a in pair_results:
        ia, ib = idx[a], idx[b]
        wins[ia] += sc_a
        wins[ib] += 1.0 - sc_a
        opps[ia][ib] += 1.0
        opps[ib][ia] += 1.0

    # Anchor virtual average player with strength 1.0
    p = [1.0] * n
    for _ in range(iters):
        p_new = [1.0] * n
        for i in range(n):
            denom = prior_games / (p[i] + 1.0)
            for j, n_ij in opps[i].items():
                denom += n_ij / (p[i] + p[j])
            p_new[i] = max(1e-6, wins[i] / denom)
        # Geometric mean normalization to 1.0
        log_mean = sum(math.log(v) for v in p_new) / n
        scale = math.exp(log_mean)
        p = [v / scale for v in p_new]

    # Convert to Elo scale (400 / ln(10) * ln(p)) centered at 1500
    c = 400.0 / math.log(10.0)
    return {team_ids[i]: 1500.0 + c * math.log(p[i]) for i in range(n)}


def load_all_rounds(work_dir: Path) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    cat = json.loads((work_dir / "catalog.json").read_text())["teams"]
    state_path = work_dir / "state.json"
    if state_path.exists():
        state = json.loads(state_path.read_text())
    else:
        state = {
            "active": [t["id"] for t in cat],
            "completed_rounds": [],
            "byes": {},
            "cuts": {},
        }
    return cat, state


def compute_standings(work_dir: Path, cat: list[dict[str, Any]], state: dict[str, Any]) -> dict[str, Any]:
    all_ids = [t["id"] for t in cat]
    points: dict[str, float] = {tid: 0.0 for tid in all_ids}
    games: dict[str, int] = {tid: 0 for tid in all_ids}
    played: dict[str, list[tuple[str, float, int]]] = defaultdict(list)  # tid -> [(opp, pair_score, round)]
    pair_history: set[tuple[str, str]] = set()
    bt_records: list[tuple[str, str, float]] = []

    for r in state["completed_rounds"]:
        rpath = work_dir / f"round-{r}" / "results.jsonl"
        if not rpath.exists():
            continue
        by_pair: dict[int, list[dict[str, Any]]] = defaultdict(list)
        for line in rpath.open():
            if line.strip():
                rec = json.loads(line)
                by_pair[int(rec["block"])].append(rec)
        for blk, recs in by_pair.items():
            assert len(recs) == 2, f"round {r} block {blk} has {len(recs)} games"
            # Determine logical pair (a, b) from rowSeat == 0
            r0 = next(x for x in recs if int(x["rowSeat"]) == 0)
            a, b = r0["ids"][0], r0["ids"][1]
            sc_a = 0.5 * sum(seat_score(x["outcome"], int(x["rowSeat"])) for x in recs)
            points[a] += sc_a
            points[b] += 1.0 - sc_a
            games[a] += 2
            games[b] += 2
            played[a].append((b, sc_a, r))
            played[b].append((a, 1.0 - sc_a, r))
            pair_history.add((min(a, b), max(a, b)))
            bt_records.append((a, b, sc_a))

    # Neutral 0.5 point per bye so a bottom team doesn't jump ahead artificially
    for tid, bye_rounds in state.get("byes", {}).items():
        points[tid] += 0.5 * len(bye_rounds)

    # Buchholz = sum of opponents' total points
    buchholz: dict[str, float] = {}
    for tid in all_ids:
        buchholz[tid] = sum(points[opp] for opp, _, _ in played[tid])

    bt_elo = fit_bradley_terry(all_ids, bt_records) if bt_records else {tid: 1500.0 for tid in all_ids}
    return {
        "points": points,
        "games": games,
        "buchholz": buchholz,
        "bt_elo": bt_elo,
        "played": played,
        "pair_history": pair_history,
    }


def rank_teams(
    ids: list[str],
    st: dict[str, Any],
    seed_salt: str,
) -> list[str]:
    return sorted(
        ids,
        key=lambda tid: (
            -round(st["points"][tid], 6),
            -round(st["buchholz"][tid], 6),
            -round(st["bt_elo"][tid], 4),
            u32_hash(f"{seed_salt}/{tid}"),
        ),
    )


def swiss_pair(
    ordered: list[str],
    pair_history: set[tuple[str, str]],
) -> list[tuple[str, str]]:
    """Greedy Swiss pairing avoiding repeat matchups where possible."""
    unmatched = list(ordered)
    pairs: list[tuple[str, str]] = []
    while len(unmatched) >= 2:
        a = unmatched.pop(0)
        chosen_idx = None
        for idx, b in enumerate(unmatched):
            if (min(a, b), max(a, b)) not in pair_history:
                chosen_idx = idx
                break
        if chosen_idx is None:
            chosen_idx = 0  # forced repeat only when unavoidable
        b = unmatched.pop(chosen_idx)
        pairs.append((a, b))
    return pairs


def cmd_pair(args: argparse.Namespace) -> None:
    cat, state = load_all_rounds(args.dir)
    st = compute_standings(args.dir, cat, state)
    active = list(state["active"])

    # Apply cut if requested before this round
    if args.cut and len(active) > args.cut:
        ranked = rank_teams(active, st, f"cut-r{args.round}")
        active = ranked[: args.cut]
        state["active"] = active
        state["cuts"][str(args.round)] = {"kept": len(active), "cutoff_points": st["points"][active[-1]], "cutoff_elo": round(st["bt_elo"][active[-1]], 1)}

    ordered = rank_teams(active, st, f"pair-r{args.round}")

    bye_team = None
    if len(ordered) % 2 == 1:
        # Give bye to lowest-ranked active team with fewest prior byes
        byes_map = state.setdefault("byes", {})
        bye_team = min(
            reversed(ordered),
            key=lambda tid: len(byes_map.get(tid, [])),
        )
        ordered.remove(bye_team)
        byes_map.setdefault(bye_team, []).append(args.round)

    pairs = swiss_pair(ordered, st["pair_history"])
    repeats = sum(1 for a, b in pairs if (min(a, b), max(a, b)) in st["pair_history"])

    rdir = args.dir / f"round-{args.round}"
    rdir.mkdir(parents=True, exist_ok=True)
    sched_rows: list[dict[str, Any]] = []
    for blk, (a, b) in enumerate(pairs):
        tag = f"nc2000/swiss-v1/r{args.round}/b{blk}/{a}/{b}"
        seed_a = u32_hash(f"{tag}/a")
        seed_b = u32_hash(f"{tag}/b")
        bseed = u32_hash(f"{tag}/battle")
        for row_seat in (0, 1):
            if row_seat == 0:
                ids = [a, b]
                seeds = [seed_a, seed_b]
                mix = [f"s:{a}", f"s:{b}"]
            else:
                ids = [b, a]
                seeds = [seed_b, seed_a]
                mix = [f"s:{b}", f"s:{a}"]
            sched_rows.append({
                "block": blk,
                "cell": {"round": args.round, "a": a, "b": b},
                "rowSeat": row_seat,
                "ids": ids,
                "seeds": seeds,
                "battleSeed": bseed,
                "mixtures": mix,
            })

    with (rdir / "schedule.jsonl").open("w") as fh:
        for row in sched_rows:
            fh.write(json.dumps(row) + "\n")

    # Save updated state (active list / bye)
    (args.dir / "state.json").write_text(json.dumps(state, indent=2))
    print(json.dumps({
        "round": args.round,
        "active": len(active),
        "pairs": len(pairs),
        "games": len(sched_rows),
        "bye": bye_team,
        "repeats": repeats,
        "schedule": str(rdir / "schedule.jsonl"),
    }))


def cmd_ingest(args: argparse.Namespace) -> None:
    cat, state = load_all_rounds(args.dir)
    rdir = args.dir / f"round-{args.round}"
    expected_rows = [json.loads(l) for l in (rdir / "schedule.jsonl").open() if l.strip()]
    expected_keys = {(int(r["block"]), int(r["rowSeat"])) for r in expected_rows}

    collected: dict[tuple[int, int], dict[str, Any]] = {}
    for d in args.dirs:
        for p in sorted(d.glob("*.json")):
            if p.name == "manifest.json":
                continue
            r = json.loads(p.read_text())
            key = (int(r["block"]), int(r["rowSeat"]))
            if key in expected_keys:
                collected[key] = r

    missing = expected_keys - set(collected.keys())
    assert not missing, f"round {args.round} missing {len(missing)} games (e.g. {list(missing)[:5]})"

    ordered_results = [collected[(int(r["block"]), int(r["rowSeat"]))] for r in expected_rows]
    with (rdir / "results.jsonl").open("w") as fh:
        for rec in ordered_results:
            fh.write(json.dumps(rec) + "\n")

    if args.round not in state["completed_rounds"]:
        state["completed_rounds"].append(args.round)
        state["completed_rounds"].sort()
    (args.dir / "state.json").write_text(json.dumps(state, indent=2))

    st = compute_standings(args.dir, cat, state)
    ranked = rank_teams(state["active"], st, f"rank-r{args.round}")
    top5 = [
        {
            "id": tid,
            "pts": round(st["points"][tid], 2),
            "games": st["games"][tid],
            "elo": round(st["bt_elo"][tid], 1),
        }
        for tid in ranked[:5]
    ]
    print(json.dumps({
        "round": args.round,
        "ingested_games": len(ordered_results),
        "active": len(state["active"]),
        "top5": top5,
    }))


def cmd_report(args: argparse.Namespace) -> None:
    cat, state = load_all_rounds(args.dir)
    cat_by_id = {t["id"]: t for t in cat}
    st = compute_standings(args.dir, cat, state)
    all_ids = [t["id"] for t in cat]
    # Rank by (completed_games desc? No: active stage then points then BT Elo, and also pure BT Elo!)
    ranked_swiss = rank_teams(all_ids, st, "final-swiss")
    ranked_elo = sorted(all_ids, key=lambda tid: (-st["bt_elo"][tid], -st["points"][tid]))

    top64_set = set(ranked_swiss[:64])

    # RPS specialist rescue: any team outside Top 64 that beat a Top-64 opponent (>= 1.5/2 on pair => sc >= 0.75)
    # OR is the highest-Elo representative of a distinct 6-species unordered archetype not in Top 64.
    giant_killers = []
    for tid in ranked_swiss[64:]:
        beaten_top64 = [opp for opp, sc, _r in st["played"][tid] if opp in top64_set and sc >= 0.75]
        if beaten_top64:
            giant_killers.append({
                "id": tid,
                "elo": round(st["bt_elo"][tid], 1),
                "pts": round(st["points"][tid], 2),
                "games": st["games"][tid],
                "beaten_top64": beaten_top64,
                "species": cat_by_id[tid]["species"],
            })
    giant_killers.sort(key=lambda r: (-len(r["beaten_top64"]), -r["elo"]))

    top_table = []
    for rank_i, tid in enumerate(ranked_swiss[: args.top], 1):
        g = st["games"][tid]
        pts = st["points"][tid]
        top_table.append({
            "rank": rank_i,
            "id": tid,
            "pts": round(pts, 2),
            "games": g,
            "win_rate": round((pts - 0.5 * len(state.get("byes", {}).get(tid, []))) / (g / 2.0), 4) if g > 0 else 0.0,
            "buchholz": round(st["buchholz"][tid], 2),
            "bt_elo": round(st["bt_elo"][tid], 1),
            "shell_hash": cat_by_id[tid]["shell_hash"],
            "species": cat_by_id[tid]["species"],
            "source": cat_by_id[tid]["source"],
        })

    out_obj = {
        "completed_rounds": state["completed_rounds"],
        "cuts": state["cuts"],
        "total_teams": len(all_ids),
        "top_swiss": top_table,
        "top32_by_bt_elo": [
            {"id": tid, "bt_elo": round(st["bt_elo"][tid], 1), "pts": round(st["points"][tid], 2), "games": st["games"][tid]}
            for tid in ranked_elo[:32]
        ],
        "rps_rescue_giant_killers": giant_killers,
    }
    out_path = args.out or (args.dir / "swiss-report.json")
    out_path.write_text(json.dumps(out_obj, indent=2))
    print(json.dumps({
        "report": str(out_path),
        "rounds": len(state["completed_rounds"]),
        "top10": [{"id": r["id"], "pts": r["pts"], "g": r["games"], "elo": r["bt_elo"]} for r in top_table[:10]],
        "rps_rescue_count": len(giant_killers),
    }))


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_pair = sub.add_parser("pair")
    p_pair.add_argument("--dir", type=Path, required=True)
    p_pair.add_argument("--round", type=int, required=True)
    p_pair.add_argument("--cut", type=int, default=0)
    p_pair.set_defaults(func=cmd_pair)

    p_ing = sub.add_parser("ingest")
    p_ing.add_argument("--dir", type=Path, required=True)
    p_ing.add_argument("--round", type=int, required=True)
    p_ing.add_argument("dirs", nargs="+", type=Path)
    p_ing.set_defaults(func=cmd_ingest)

    p_rep = sub.add_parser("report")
    p_rep.add_argument("--dir", type=Path, required=True)
    p_rep.add_argument("--top", type=int, default=64)
    p_rep.add_argument("--out", type=Path, default=None)
    p_rep.set_defaults(func=cmd_report)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
