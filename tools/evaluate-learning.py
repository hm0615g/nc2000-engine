#!/usr/bin/env python3
import argparse
from collections import Counter
import hashlib
import json
import math
import statistics
from pathlib import Path


BETTING_FRACTIONS = (.025, .05, .1, .2, .3, .4, .5, .7, .9)


def log_betting_evalue(counts, null_mean):
    if not 0 < null_mean <= 1:
        raise ValueError("null mean must be in (0, 1]")
    if any(not 0 <= value <= 1 or count < 0 for value, count in counts.items()):
        raise ValueError("scores must be bounded by zero and one")
    logs = [sum(count*math.log1p(fraction*(value/null_mean-1)) for value, count in counts.items())
            for fraction in BETTING_FRACTIONS]
    maximum = max(logs)
    return maximum + math.log(sum(math.exp(value-maximum) for value in logs)/len(logs))


def betting_interval(samples, alpha=.05):
    if not 0 < alpha < 1:
        raise ValueError("alpha must be in (0, 1)")
    if not samples:
        return None
    threshold = math.log(2/alpha)
    def lower(counts):
        lo, hi = 0.0, 1.0
        for _ in range(60):
            mean = (lo+hi)/2
            if log_betting_evalue(counts, mean) >= threshold:
                lo = mean
            else:
                hi = mean
        return lo
    return [lower(Counter(samples)), 1-lower(Counter(1-value for value in samples))]


def score_intervals(pairs):
    mean = statistics.mean(pairs) if pairs else None
    margin = 1.959963984540054 * statistics.stdev(pairs) / math.sqrt(len(pairs)) if len(pairs) > 1 else None
    interval = [mean - margin, mean + margin] if margin is not None else None
    bounded_margin = math.sqrt(math.log(40) / (2 * len(pairs))) if pairs else None
    bounded = [max(0, mean - bounded_margin), min(1, mean + bounded_margin)] if pairs else None
    bernstein = None
    if len(pairs) > 1:
        log = math.log(4 / .05)
        width = math.sqrt(2 * statistics.variance(pairs) * log / len(pairs)) + 7 * log / (3 * (len(pairs) - 1))
        bernstein = [max(0, mean - width), min(1, mean + width)]
    return {
        "complete_pairs": len(pairs), "score": mean, "normal95": interval, "hoeffding95": bounded,
        "empirical_bernstein95": bernstein, "betting95": betting_interval(pairs),
        "strength_test": "fixed-fraction-mixture-v1",
        "log_evalue_at_half": log_betting_evalue(Counter(pairs), .5) if pairs else None,
    }


def summarize(path, require_complete=True):
    games = {}
    with Path(path).open() as file:
        manifest = json.loads(next(file))
        if manifest["type"] != "manifest" or manifest["config"]["schema"] != "nc2000-learning-arena-v1":
            raise ValueError("invalid manifest")
        count = manifest["config"]["games"]
        if count <= 0 or count % 2:
            raise ValueError("game count must be positive/even")
        for line in file:
            row = json.loads(line)
            game = row["game"]
            if row["type"] != "game" or not isinstance(game, int) or not 0 <= game < count or game in games:
                raise ValueError("invalid/duplicate game")
            if row["pair"] != game // 2 or row["swap"] != game % 2 or row["capped"] is not False:
                raise ValueError("invalid pairing or capped game")
            if row["outcome"] not in ["p1", "p2", "tie"]:
                raise ValueError("invalid outcome")
            p1 = {"p1": 1.0, "p2": 0.0, "tie": 0.5}[row["outcome"]]
            expected = p1 if row["swap"] == 0 else 1 - p1
            if row["score"] != expected:
                raise ValueError("score does not match outcome and orientation")
            if not 0 <= row["turns"] <= 1001:
                raise ValueError("turn count exceeds engine limit")
            for key in ["decision_ns", "worker_ns"]:
                if len(row[key]) != 2 or any(not xs or any(type(x) is not int or x < 0 for x in xs) for xs in row[key]):
                    raise ValueError("invalid decision timings")
            games[game] = row
    complete = len(games) == count
    if require_complete and not complete:
        raise ValueError(f"incomplete run: {len(games)}/{count}")
    pairs = []
    for game in range(0, count, 2):
        if game not in games or game + 1 not in games:
            continue
        a, b = games[game], games[game + 1]
        if a["team_ids"] != b["team_ids"] or a["battle_seed"] != b["battle_seed"]:
            raise ValueError("side-swap pair changed teams or battle seed")
        pairs.append((a["score"] + b["score"]) / 2)
    intervals = score_intervals(pairs)
    times = []
    for side in range(2):
        xs = sorted(x / 1e6 for row in games.values() for x in row["decision_ns"][side])
        times.append({
            "decisions": len(xs), "mean_ms": statistics.mean(xs) if xs else None,
            "p95_ms": xs[math.ceil(len(xs)*.95)-1] if xs else None,
            "p99_ms": xs[math.ceil(len(xs)*.99)-1] if xs else None,
        })
    return {
        "complete": complete, "games": len(games), "planned_games": count,
        **intervals,
        "wins": sum(row["score"] == 1 for row in games.values()),
        "losses": sum(row["score"] == 0 for row in games.values()),
        "ties": sum(row["score"] == .5 for row in games.values()),
        "timing": times,
        "positive_strength_evidence": bool(complete and intervals["betting95"] and intervals["betting95"][0] > .5),
        "pair_scores": pairs,
    }


def run_identity(manifest):
    config, hashes = manifest["config"], manifest["hashes"]
    agents = []
    if len(config["agents"]) != 2 or len(hashes["agents"]) != 2:
        raise ValueError("a run must have two agents")
    for spec, record in zip(config["agents"], hashes["agents"]):
        artifacts = {item["path"]: item["hash"] for item in record["artifacts"]}
        agents.append({"program": record["program"], "args": [artifacts.get(arg, arg) for arg in spec["args"]],
                       "artifacts": sorted(artifacts.values())})
    return {"schema": config["schema"], "arena": hashes["arena"], "pool": hashes["pool"], "dex": hashes["dex"],
            "agents": agents, "crn_agent_seeds": config.get("crn_agent_seeds", False)}


def combine(paths, require_complete=True):
    summaries, sources = [], []
    seeds, schedules = set(), set()
    identity = None
    for path in paths:
        summaries.append(summarize(path, require_complete))
        with Path(path).open() as file:
            manifest = json.loads(next(file))
            current = run_identity(manifest)
            if identity is not None and current != identity:
                raise ValueError("run identities differ: agents, arguments, data, or arena changed")
            identity = current
            seed = manifest["config"]["seed"]
            if seed in seeds:
                raise ValueError("duplicate run seed")
            seeds.add(seed)
            for line in file:
                row = json.loads(line)
                if row["swap"] == 0:
                    key = (row["battle_seed"], tuple(row["team_ids"]))
                    if key in schedules:
                        raise ValueError("duplicate battle seed and teams across blocks")
                    schedules.add(key)
        sources.append({"path": str(path), "sha256": hashlib.sha256(Path(path).read_bytes()).hexdigest(), "seed": seed})
    if not summaries:
        raise ValueError("no runs to combine")
    pairs = [score for summary in summaries for score in summary["pair_scores"]]
    intervals = score_intervals(pairs)
    complete = all(summary["complete"] for summary in summaries)
    return {
        "complete": complete, **intervals,
        **{key: sum(summary[key] for summary in summaries) for key in ["games", "planned_games", "wins", "losses", "ties"]},
        "positive_strength_evidence": bool(complete and intervals["betting95"] and intervals["betting95"][0] > .5),
        "pair_scores": pairs, "sources": sources, "identity": identity,
        "timing_by_run": [summary["timing"] for summary in summaries],
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("path", nargs="+")
    parser.add_argument("--partial", action="store_true")
    args = parser.parse_args()
    result = summarize(args.path[0], not args.partial) if len(args.path) == 1 else combine(args.path, not args.partial)
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
