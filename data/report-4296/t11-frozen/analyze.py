import argparse
import collections
import hashlib
import json
import statistics
from pathlib import Path


def paired(values):
    n = len(values)
    critical = {5: 2.776, 8: 2.365, 12: 2.201, 13: 2.179}.get(n)
    mean = statistics.mean(values)
    radius = critical * statistics.stdev(values) / n**0.5 if critical else None
    return {"n": n, "mean": mean, "ci95": None if radius is None else [mean - radius, mean + radius]}


def summarize(rows):
    groups = collections.defaultdict(dict)
    for row in rows:
        assert row["seed"] not in groups[row["arm"]]
        groups[row["arm"]][row["seed"]] = row
    result = {}
    for arm, group in groups.items():
        deltas = []
        terminals = []
        for row in group.values():
            actions = {a["action"]: a for a in row["actions"]}
            deltas.append(actions["move hiddenpowerbug"]["mean"] - actions["move earthquake"]["mean"])
            if arm != "baseline":
                assert sum(a["n"] for a in row["actions"]) == row["evaluation"]
                assert row["training"] + row["evaluation"] == 30000
                assert all(a["root_replies"] == row["actions"][0]["root_replies"] for a in row["actions"])
                terminals.extend(a["terminal"] / a["n"] for a in row["actions"])
        result[arm] = {
            "choices": dict(collections.Counter(r["best"] for r in group.values())),
            "hp_minus_eq": paired(deltas),
            "terminal_fraction_range": [min(terminals), max(terminals)] if terminals else None,
        }
    effects = collections.defaultdict(list)
    for seed, execution in groups["execute_both"].items():
        arms = {arm: {a["action"]: a["mean"] for a in group[seed]["actions"]} for arm, group in groups.items()}
        for name, other in [("opponent_sampling", "sample_foe"), ("own_sampling", "sample_ours")]:
            delta = {a: arms[other][a] - arms["execute_both"][a] for a in arms[other]}
            for action in ["move earthquake", "move hiddenpowerbug"]:
                effects[f"{name}:{action}"].append(delta[action])
            effects[f"{name}:hp_minus_eq"].append(delta["move hiddenpowerbug"] - delta["move earthquake"])
    result["paired_effects"] = {k: paired(v) for k, v in effects.items()}
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    rows = []
    hashes = {}
    for name in ["discovery.jsonl", "confirmation-known.jsonl", "confirmation-new.jsonl"]:
        path = args.directory / name
        hashes[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        rows.extend(json.loads(line) for line in path.read_text().splitlines())
    matrix = []
    for name in ["matrix-discovery.jsonl", "matrix-confirmation.jsonl"]:
        path = args.directory / name
        hashes[name] = hashlib.sha256(path.read_bytes()).hexdigest()
        for row in map(json.loads, path.read_text().splitlines()):
            if row["arm"] != "matrix":
                continue
            assert sum(row["n"]) == row["evaluation"]
            width = len(row["replies"])
            eq = row["actions"].index("move earthquake")
            staying = [i for i, reply in enumerate(row["replies"]) if reply.startswith("move ")]
            means = [row["means"][eq * width + i] for i in staying]
            assert len(staying) == 4 and all(v == means[0] for v in means)
            matrix.append({"seed": row["seed"], "modal_action": row["modal_action"],
                           "policy": dict(zip([row["actions"][a] for a in row["eligible"]], row["policy"])),
                           "equal_earthquake_staying_cells": True})
    output = {
        "hashes": hashes,
        "all_13": summarize(rows),
        "new_8": summarize([r for r in rows if 63001 <= r["seed"] <= 63008]),
        "matrix": matrix,
    }
    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
