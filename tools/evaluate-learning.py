#!/usr/bin/env python3
import argparse
import json
import math
import statistics
from pathlib import Path


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
        "complete_pairs": len(pairs), "score": mean, "normal95": interval, "hoeffding95": bounded,
        "empirical_bernstein95": bernstein,
        "wins": sum(row["score"] == 1 for row in games.values()),
        "losses": sum(row["score"] == 0 for row in games.values()),
        "ties": sum(row["score"] == .5 for row in games.values()),
        "timing": times,
        "positive_strength_evidence": bool(complete and bernstein and bernstein[0] > .5),
        "pair_scores": pairs,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("path")
    parser.add_argument("--partial", action="store_true")
    args = parser.parse_args()
    result = summarize(args.path, not args.partial)
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == "__main__":
    main()
