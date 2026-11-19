#!/usr/bin/env python3
"""Canonical shell-profile matchup ledger & Local/Colab shard manager.

Exploits the Shell-Level Bilinear Decomposition theorem:
  Once Turn-0 Team Preview reveals the opponent's public shell S, `Belief` filters
  `alive` exclusively to candidates matching S and normalizes weights within S.
  Therefore, any production game between realized variants (v_A, v_B) under
  disclosed pools (P_A, P_B) depends ONLY on the conditional shell-profiles
  sigma_A = (S_A, q_A) and sigma_B = (S_B, q_B), and is 100% reusable across all
  outer-weight combinations over {sigma_k}.

Subcommands:
  import-archive   Ingest existing `data/nash-camouflage-v1` production records.
  shard            Split a schedule JSONL into N shards for Local + Colab workers.
  merge            Merge completed `nash_refresh` output directories into a ledger JSONL.
  summary          Compute pooled shell-profile matrix & challenger upper bounds.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import defaultdict
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent

BASE_TEAM_FILES = [
    "kodoku-beq-v1/novel_evolved_teams",
    "team-pool-v2/team-pool",
    "kodoku-r3/conclusion-equilibrium-deme",
]

ARCHIVE_DATASETS = [
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


def sha256_json(obj: Any) -> str:
    raw = json.dumps(obj, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(raw).hexdigest()[:16]


def shell_signature(sets: list[dict[str, Any]]) -> tuple[tuple[Any, ...], ...]:
    """Turn-0 public appearance signature (species, level, gender, has_item, order)."""
    return tuple(
        (m["species"], int(m.get("level", 50)), str(m.get("gender", "")), bool(m.get("item")))
        for m in sets
    )


def load_catalog_and_mixtures(pools_path: Path | None) -> tuple[dict[str, list[dict[str, Any]]], dict[str, list[dict[str, Any]]]]:
    teams: dict[str, list[dict[str, Any]]] = {}
    for name in BASE_TEAM_FILES:
        data = json.loads((ROOT / "data" / f"{name}.json").read_text())
        for t in data["teams"]:
            teams[t["id"]] = t["sets"]

    mixtures: dict[str, list[dict[str, Any]]] = {}
    cand = json.loads((ROOT / "data/nash-refresh-v1/candidate.json").read_text())
    mixtures["candidate"] = [{"id": t["id"], "weight": float(t["weight"])} for t in cand["teams"]]
    inc = json.loads((ROOT / "data/kodoku-r3/conclusion-equilibrium-deme.json").read_text())
    mixtures["incumbent"] = [{"id": t["id"], "weight": float(t["weight"])} for t in inc["teams"]]
    mixtures["deme"] = [{"id": "deme-r1-00-s0030", "weight": 1.0}]

    if pools_path is not None:
        extra = json.loads(pools_path.read_text())
        for t in extra.get("teams", []):
            teams[t["id"]] = t["sets"]
        for name, members in extra.get("mixtures", {}).items():
            mixtures[name] = [{"id": m["id"], "weight": float(m["weight"])} for m in members]
    return teams, mixtures


def conditional_shell_profile(
    realized_id: str,
    mixture_members: list[dict[str, Any]],
    teams: dict[str, list[dict[str, Any]]],
) -> dict[str, Any]:
    """Extract canonical conditional shell-profile (S, q) for the realized team's shell."""
    sig = shell_signature(teams[realized_id])
    # Collect all members in mixture sharing this exact public shell
    matching = []
    for m in mixture_members:
        tid = m["id"]
        if shell_signature(teams[tid]) == sig:
            matching.append((tid, float(m["weight"])))
    total = sum(w for _, w in matching)
    assert total > 0.0, f"realized {realized_id} missing from shell"
    # Merge identical sets if any, normalize conditional weights to 1.0
    by_set: dict[str, tuple[str, float]] = {}
    for tid, w in matching:
        sh = sha256_json(teams[tid])
        cond_w = w / total
        if sh in by_set:
            prev_id, prev_w = by_set[sh]
            by_set[sh] = (min(prev_id, tid), prev_w + cond_w)
        else:
            by_set[sh] = (tid, cond_w)
    canon = sorted((sh, round(w, 8)) for sh, (_, w) in by_set.items())
    profile_hash = sha256_json({"shell": sig, "cond": canon})
    label_parts = [f"{tid}:{round(w, 3)}" for _, (tid, w) in sorted(by_set.items(), key=lambda kv: -kv[1][1])]
    return {
        "shell_hash": sha256_json(sig),
        "profile_hash": profile_hash,
        "species": [s[0] for s in sig],
        "label": "+".join(label_parts),
        "realized_set_hash": sha256_json(teams[realized_id]),
    }


def row_score(outcome: str, seat: int) -> float:
    if outcome in ("tie", "cap"):
        return 0.5
    win_seat = 0 if outcome == "p1" else 1
    return 1.0 if win_seat == seat else 0.0


def cmd_import_archive(args: argparse.Namespace) -> None:
    base = ROOT / "data/nash-camouflage-v1"
    seen: set[str] = set()
    records: list[dict[str, Any]] = []
    for games_rel, pools_rel in ARCHIVE_DATASETS:
        teams, mixtures = load_catalog_and_mixtures(base / pools_rel)
        for line in (base / games_rel).open():
            if not line.strip():
                continue
            r = json.loads(line)
            mix = r.get("mixtures")
            if not mix:
                continue
            p0 = conditional_shell_profile(r["ids"][0], mixtures[mix[0]], teams)
            p1 = conditional_shell_profile(r["ids"][1], mixtures[mix[1]], teams)
            dedup_key = sha256_json([
                p0["profile_hash"], p0["realized_set_hash"],
                p1["profile_hash"], p1["realized_set_hash"],
                r["seeds"], r["battleSeed"],
            ])
            if dedup_key in seen:
                continue
            seen.add(dedup_key)
            records.append({
                "source": games_rel,
                "key": dedup_key,
                "rowSeat": int(r["rowSeat"]),
                "battleSeed": int(r["battleSeed"]),
                "seeds": r["seeds"],
                "ids": r["ids"],
                "profiles": [p0["profile_hash"], p1["profile_hash"]],
                "profile_labels": [p0["label"], p1["label"]],
                "realized_hashes": [p0["realized_set_hash"], p1["realized_set_hash"]],
                "outcome": r["outcome"],
                "turns": int(r["turns"]),
                "actionSha256": r["actionSha256"],
            })

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w") as fh:
        for rec in records:
            fh.write(json.dumps(rec) + "\n")

    distinct_profiles = {p for rec in records for p in rec["profiles"]}
    print(json.dumps({
        "imported_unique_games": len(records),
        "distinct_shell_profiles": len(distinct_profiles),
        "out": str(args.out),
    }))


def cmd_shard(args: argparse.Namespace) -> None:
    rows = [l.strip() for l in args.schedule.open() if l.strip()]
    args.out_dir.mkdir(parents=True, exist_ok=True)
    shards: list[list[str]] = [[] for _ in range(args.shards)]
    # Round-robin keeps seat pairs / strata balanced across workers
    for i, row in enumerate(rows):
        shards[i % args.shards].append(row)
    written = []
    for k, chunk in enumerate(shards):
        if not chunk:
            continue
        p = args.out_dir / f"shard-{k}.jsonl"
        p.write_text("\n".join(chunk) + "\n")
        written.append({"shard": k, "games": len(chunk), "path": str(p)})
    print(json.dumps({"total_games": len(rows), "shards": len(written), "files": written}))


def cmd_merge(args: argparse.Namespace) -> None:
    teams, mixtures = load_catalog_and_mixtures(args.pools)
    existing: dict[str, dict[str, Any]] = {}
    if args.ledger.exists():
        for line in args.ledger.open():
            if line.strip():
                rec = json.loads(line)
                existing[rec["key"]] = rec

    added = 0
    for out_dir in args.dirs:
        for path in sorted(out_dir.glob("*.json")):
            if path.name == "manifest.json":
                continue
            r = json.loads(path.read_text())
            mix = r.get("mixtures")
            if not mix:
                continue
            p0 = conditional_shell_profile(r["ids"][0], mixtures[mix[0]], teams)
            p1 = conditional_shell_profile(r["ids"][1], mixtures[mix[1]], teams)
            key = sha256_json([
                p0["profile_hash"], p0["realized_set_hash"],
                p1["profile_hash"], p1["realized_set_hash"],
                r["seeds"], r["battleSeed"],
            ])
            if key in existing:
                continue
            existing[key] = {
                "source": str(out_dir),
                "key": key,
                "rowSeat": int(r["rowSeat"]),
                "battleSeed": int(r["battleSeed"]),
                "seeds": r["seeds"],
                "ids": r["ids"],
                "profiles": [p0["profile_hash"], p1["profile_hash"]],
                "profile_labels": [p0["label"], p1["label"]],
                "realized_hashes": [p0["realized_set_hash"], p1["realized_set_hash"]],
                "outcome": r["outcome"],
                "turns": int(r["turns"]),
                "actionSha256": r["actionSha256"],
            }
            added += 1

    args.ledger.parent.mkdir(parents=True, exist_ok=True)
    with args.ledger.open("w") as fh:
        for rec in existing.values():
            fh.write(json.dumps(rec) + "\n")
    print(json.dumps({"ledger": str(args.ledger), "added": added, "total_games": len(existing)}))


def cmd_summary(args: argparse.Namespace) -> None:
    pair_games: dict[tuple[str, str], list[float]] = defaultdict(list)
    labels: dict[str, str] = {}
    total = 0
    for line in args.ledger.open():
        if not line.strip():
            continue
        r = json.loads(line)
        total += 1
        p0, p1 = r["profiles"]
        labels[p0] = r["profile_labels"][0]
        labels[p1] = r["profile_labels"][1]
        s0 = row_score(r["outcome"], 0)
        pair_games[(p0, p1)].append(s0)
        pair_games[(p1, p0)].append(1.0 - s0)

    top_pairs = sorted(
        ((a, b, len(v), sum(v) / len(v)) for (a, b), v in pair_games.items() if a < b),
        key=lambda t: -t[2],
    )[: args.top]
    print(json.dumps({
        "total_games": total,
        "distinct_profiles": len(labels),
        "top_shell_matchups": [
            {
                "a": labels[a],
                "b": labels[b],
                "games": n,
                "score_a": round(sc, 4),
            }
            for a, b, n, sc in top_pairs
        ],
    }))


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_imp = sub.add_parser("import-archive")
    p_imp.add_argument("--out", type=Path, default=ROOT / "target/nash-ledger.jsonl")
    p_imp.set_defaults(func=cmd_import_archive)

    p_sh = sub.add_parser("shard")
    p_sh.add_argument("--schedule", type=Path, required=True)
    p_sh.add_argument("--shards", type=int, required=True)
    p_sh.add_argument("--out-dir", type=Path, required=True)
    p_sh.set_defaults(func=cmd_shard)

    p_mg = sub.add_parser("merge")
    p_mg.add_argument("--ledger", type=Path, required=True)
    p_mg.add_argument("--pools", type=Path, default=None)
    p_mg.add_argument("dirs", nargs="+", type=Path)
    p_mg.set_defaults(func=cmd_merge)

    p_sum = sub.add_parser("summary")
    p_sum.add_argument("--ledger", type=Path, required=True)
    p_sum.add_argument("--top", type=int, default=15)
    p_sum.set_defaults(func=cmd_summary)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
