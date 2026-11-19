"""Repeat the leading held-out deviation with all seven candidate types represented."""
import math
import sys
from collections import defaultdict
from statistics import mean

from analyze import interval, load, logical, paired, pools, records
from screen import HERE, pair, save_games, write


def schedule():
    cfg = load("confirmation-pools.json")
    summary = load("confirmation-summary.json")
    assert not summary["try_redistribution"]
    opponent = summary["worst"]
    members = pools(cfg)["proposed"]
    assert len(pools(cfg)[opponent]) == 1
    opponent_id = pools(cfg)[opponent][0]["id"]
    rows = []
    for i, t in enumerate(members):
        for rep in range(2):
            block = 2*i + rep
            rows.extend(pair("stratified", block, i, [opponent_id, t["id"]], [opponent, "proposed"]))
    save_games(HERE / "stratified-schedule.jsonl", rows)


def summarize():
    cfg = load("confirmation-pools.json")
    members = pools(cfg)["proposed"]
    opponent = load("confirmation-summary.json")["worst"]
    scores = defaultdict(list)
    for (a,b,_), (v,rows) in paired(records("stratified-games.jsonl"), cfg).items():
        assert (a,b) == (opponent,"proposed")
        scores[logical(rows[0], "ids")[1]].append(v)
    values, weights = [], []
    for t in members:
        blocks = scores[t["id"]]
        assert len(blocks) == 2
        values.extend(blocks)
        weights.extend([t["weight"]/len(blocks)]*len(blocks))
    assert math.isclose(sum(weights), 1)
    write("stratified-summary.json", {
        "opponent": opponent, "heldout_preceding": load("confirmation-summary.json")["heldout"][opponent],
        "challenger": interval(values, weights), "conditional_scores": {k: mean(v) for k,v in scores.items()},
        "sampling": "Two fresh seat-pairs per candidate type; challenger receives the full proposed pool throughout.",
    })


if __name__ == "__main__":
    {"schedule": schedule, "summarize": summarize}[sys.argv[1]]()
