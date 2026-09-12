#!/usr/bin/env python3
import argparse
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path


def active(state, side):
    roster = state["sides"][side]
    return next((p for p in roster["party"] if p["slot"] == roster["active"]), None)


def accumulator():
    return {"n": 0, "reward": 0.0, "terminal": 0, "tree_steps": 0,
            "child_keys": set(), "child_dimensions": set(), "starmie_nodes": set(),
            "starmie_visits_sum": 0, "starmie_count": 0,
            "starmie_exploration": 0, "starmie_untried": 0,
            "starmie_actions": defaultdict(lambda: [0, 0.0]),
            "replacement": defaultdict(lambda: [0, 0.0])}


def add(group, row):
    tree = row["tree"]
    group["n"] += 1
    group["reward"] += row["reward"]
    group["terminal"] += row["terminal"]
    group["tree_steps"] += len(tree)
    child = tree[1]["state"] if len(tree) > 1 else row["leaf"]["state"]
    group["child_keys"].add(child["key"])
    foe, mine = active(child, 0), active(child, 1)
    duration = dict(mine["durations"]).get("encore")
    hp_bucket = foe["hp"] * 16 // foe["maxhp"]
    group["child_dimensions"].add((child["quick_claw"], foe["species"], hp_bucket, duration))
    for event in tree[1:]:
        foe = active(event["state"], 0)
        chosen = event["chosen"][0]
        if foe["species"] == "shuckle" and foe["hp"] == 0 and chosen and chosen.startswith("switch "):
            target = event["state"]["sides"][0]["party"][int(chosen.split()[1]) - 1]["species"]
            cell = group["replacement"][target]
            cell[0] += 1
            cell[1] += row["reward"]
            break
    else:
        cell = group["replacement"]["not_observed_in_tree"]
        cell[0] += 1
        cell[1] += row["reward"]
    for event in tree[1:]:
        foe, mine = active(event["state"], 0), active(event["state"], 1)
        if (foe["species"] != "starmie" or foe["hp"] != foe["maxhp"]
                or mine["species"] != "marowak" or mine["hp"] <= 0):
            continue
        chosen = event["chosen"][0]
        if chosen is None:
            continue
        index = event["actions"][0].index(chosen)
        counts = event["visits"][0].copy()
        if len(counts) > 1:
            counts[index] -= 1
        means = [w / n if n else None for n, w in zip(counts, event["rewards"][0])]
        group["starmie_count"] += 1
        group["starmie_nodes"].add(event["node"])
        group["starmie_visits_sum"] += sum(counts)
        group["starmie_untried"] += counts[index] == 0
        group["starmie_exploration"] += (counts[index] > 0 and
            means[index] < max(m for m in means if m is not None))
        cell = group["starmie_actions"][chosen]
        cell[0] += 1
        cell[1] += row["reward"]
        break


parser = argparse.ArgumentParser()
parser.add_argument("traces", nargs="+")
parser.add_argument("--out", required=True)
parser.add_argument("--checkpoints", nargs="*", default=[])
args = parser.parse_args()
out = Path(args.out)
out.mkdir(parents=True, exist_ok=True)
groups = defaultdict(accumulator)
root = defaultdict(lambda: [0, 0.0])
counts = Counter()
hashes = {}
examples = []
eq_replies = defaultdict(lambda: {"keys": set(), "n": 0, "iteration_sum": 0})
for filename in args.traces:
    digest = hashlib.sha256()
    with open(filename, "rb") as source:
        for line in source:
            digest.update(line)
            row = json.loads(line)
            seed, iteration = row["seed"], row["iteration"]
            counts[seed] += 1
            assert counts[seed] == iteration
            assert 1 <= iteration <= 30000
            action = row["tree"][0]["chosen"][1]
            reply = row["tree"][0]["chosen"][0]
            root[seed, action][0] += 1
            root[seed, action][1] += row["reward"]
            if action == "move earthquake" and reply.startswith("move "):
                child = row["tree"][1]["state"] if len(row["tree"]) > 1 else row["leaf"]["state"]
                cell = eq_replies[seed, reply]
                cell["keys"].add(child["key"])
                cell["n"] += 1
                cell["iteration_sum"] += iteration
            if seed == 61001 and iteration in (2576, 7885, 22501, 24418):
                examples.append(row)
            if action not in ("move earthquake", "move hiddenpowerbug"):
                continue
            filters = ["all"] + ([reply] if reply in ("move encore", "move sleeptalk") else [])
            for reply_filter in filters:
                for cohort in ["all", f"quarter_{(iteration-1)//7500+1}"]:
                    add(groups[seed, action, reply_filter, cohort], row)
    hashes[Path(filename).name] = digest.hexdigest()
assert all(n == 30000 for n in counts.values()), counts
checked = set()
for filename in args.checkpoints:
    for line in Path(filename).read_text().splitlines():
        row = json.loads(line)
        if row["iterations"] != 30000:
            continue
        assert row["seed"] not in checked
        checked.add(row["seed"])
        for action in row["actions"]:
            n, reward = root[row["seed"], action["action"]]
            assert n == action["visits"]
            assert abs(reward/n-action["mean"]) < 1e-12
if args.checkpoints:
    assert checked == set(counts)
rows = []
for (seed, action, reply_filter, cohort), group in sorted(groups.items()):
    n = group.pop("n")
    group["mean"] = group.pop("reward") / n
    group["mean_tree_steps"] = group.pop("tree_steps") / n
    group["child_key_count"] = len(group.pop("child_keys"))
    group["child_dimensions"] = sorted(group["child_dimensions"], key=str)
    group["starmie_node_count"] = len(group.pop("starmie_nodes"))
    group["starmie_mean_visits_before"] = group.pop("starmie_visits_sum") / max(1, group["starmie_count"])
    for key in ("replacement", "starmie_actions"):
        group[key] = {k: {"n": v[0], "mean": v[1] / v[0]} for k, v in sorted(group[key].items())}
    rows.append({"seed": seed, "action": action, "reply_filter": reply_filter, "cohort": cohort, "n": n, **group})
equivalent = []
for seed in sorted(counts):
    cells = {reply: cell for (s, reply), cell in eq_replies.items() if s == seed}
    shared = all(cell["keys"] == next(iter(cells.values()))["keys"] for cell in cells.values())
    equivalent.append({"seed": seed, "all_stay_replies_share_child_keys": shared,
        "replies": {reply: {"child_keys": sorted(cell["keys"]), "n": cell["n"],
                            "mean_iteration": cell["iteration_sum"]/cell["n"]}
                    for reply, cell in sorted(cells.items())}})
summary = {"schema": "nc2000-t11-search-cause-v1", "trace_sha256": hashes,
           "iterations_by_seed": counts, "reply_filters": ["all", "move encore", "move sleeptalk"],
           "starmie_filter": "First observed tree decision with full-HP Starmie versus living Marowak; not a count of rollout decisions.",
           "estimand": "Rewards and choices of the original adaptive search, not independent terminal win rates.",
           "checkpoint_seeds_verified": sorted(checked), "earthquake_equivalent_replies": equivalent,
           "root": [{"seed": seed, "action": action, "visits": n, "mean": reward/n}
                    for (seed, action), (n, reward) in sorted(root.items())], "groups": rows}
(out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
(out / "examples.jsonl").write_text("".join(json.dumps(r, separators=(",", ":")) + "\n" for r in examples))
print(json.dumps({"iterations_by_seed": counts, "groups": len(rows), "examples": len(examples)}))
