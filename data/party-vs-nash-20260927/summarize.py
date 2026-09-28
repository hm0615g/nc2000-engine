import collections
import json
import math
import pathlib
import statistics
import sys

def summarize(rows):
    rows = sorted(rows, key=lambda r: r["g"])
    assert len({r["g"] for r in rows}) == len(rows)
    pairs = collections.defaultdict(list)
    counts = collections.Counter()
    for row in rows:
        if row["result"] == "p1_win":
            assert row["score"] == float(row["x_is_p1"])
        elif row["result"] == "p2_win":
            assert row["score"] == float(not row["x_is_p1"])
        else:
            assert row["result"] in ("tie", "turn_capped") and row["score"] == 0.5
        score = 1 - row["score"]
        pairs[row["g"] // 2].append(row)
        counts["wins" if score == 1 else "losses" if score == 0 else "draws"] += 1
        counts["turn_capped"] += row["result"] == "turn_capped"
    means = []
    for pair in pairs.values():
        assert len(pair) == 2
        assert pair[0]["x_team"] == pair[1]["x_team"]
        assert {r["x_is_p1"] for r in pair} == {True, False}
        means.append(statistics.mean(1 - r["score"] for r in pair))
    mean = statistics.mean(means)
    half = 1.96 * statistics.stdev(means) / math.sqrt(len(means)) if len(means) > 1 else None
    return {
        "games": len(rows), "paired_blocks": len(means),
        **{k: counts[k] for k in ["wins", "losses", "draws", "turn_capped"]},
        "score": mean, "ci95": [max(0, mean-half), min(1, mean+half)] if half is not None else None,
        "mean_turns": statistics.mean(r["turns"] for r in rows),
        "nash_belief_fallback_games": sum(r["belief_fallback"] for r in rows),
    }


result = {}
for filename in sys.argv[1:]:
    path = pathlib.Path(filename)
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    metadata = json.loads(path.with_suffix(".summary.json").read_text())
    assert metadata["y_blind"]
    assert len(rows) == metadata["games_requested"] == metadata["games_done"]
    assert {r["g"] for r in rows} == set(range(len(rows)))
    by_team = collections.defaultdict(list)
    for row in rows:
        by_team[row["x_team"]].append(row)
    result[path.stem] = {
        "party_perspective": True,
        "iterations": metadata["iters"], "seed": metadata["seed"],
        "overall": summarize(rows),
        "by_opponent": {team: summarize(group) for team, group in sorted(by_team.items())},
    }
print(json.dumps(result, indent=2))
