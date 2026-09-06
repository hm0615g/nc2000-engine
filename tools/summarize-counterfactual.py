import argparse
import itertools
import json
import math
import statistics
from collections import defaultdict
from pathlib import Path


def summarize(paths):
    groups = defaultdict(lambda: defaultdict(dict))
    for path in paths:
        for line in Path(path).read_text().splitlines():
            row = json.loads(line)
            group = (
                row["turn"], row.get("policy", "skuct"), row["iters"],
                row["foe_iters"], row["reply"], row["tail"],
            )
            trial = (row["seed"], row["trial"])
            trials = groups[group][row["action"]]
            if trial in trials:
                raise ValueError(f"duplicate trial: {group}, {row['action']}, {trial}")
            score = row["score"]
            expected = {"win": 1, "loss": 0, "tie": 0.5, "cap": None}[row["outcome"]]
            if score != expected:
                raise ValueError(f"inconsistent outcome: {row}")
            trials[trial] = score
    result = []
    for group, actions in sorted(groups.items()):
        turn, policy, iters, foe_iters, reply, tail = group
        item = dict(turn=turn, policy=policy, iters=iters, foe_iters=foe_iters,
                    reply=reply, tail=tail, actions={}, pairs=[])
        for action, trials in sorted(actions.items()):
            values = list(trials.values())
            caps = values.count(None)
            score = sum(v for v in values if v is not None)
            item["actions"][action] = dict(
                trials=len(values), wins=values.count(1), losses=values.count(0),
                ties=values.count(0.5), caps=caps,
                score=score / len(values) if not caps else None,
                score_bounds=[score / len(values), (score + caps) / len(values)],
            )
        for a, b in itertools.combinations(sorted(actions), 2):
            keys = sorted(actions[a].keys() & actions[b].keys())
            pair = dict(a=a, b=b, paired_trials=len(keys))
            if not keys or any(actions[a][k] is None or actions[b][k] is None for k in keys):
                pair.update(difference=None, normal95_interval=None, exact_mcnemar_p=None)
            else:
                differences = [actions[a][k] - actions[b][k] for k in keys]
                mean = statistics.mean(differences)
                se = statistics.stdev(differences) / math.sqrt(len(keys)) if len(keys) > 1 else None
                plus = differences.count(1)
                minus = differences.count(-1)
                discordant = plus + minus
                binary = all(actions[a][k] in (0, 1) and actions[b][k] in (0, 1) for k in keys)
                p = min(1, 2 * sum(math.comb(discordant, k) for k in range(min(plus, minus) + 1))
                        / 2 ** discordant) if binary else None
                pair.update(
                    difference=mean,
                    normal95_interval=[mean - 1.96 * se, mean + 1.96 * se] if se is not None else None,
                    a_only_wins=plus, b_only_wins=minus, exact_mcnemar_p=p,
                )
            item["pairs"].append(pair)
        result.append(item)
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("inputs", nargs="+")
    args = parser.parse_args()
    print(json.dumps(summarize(args.inputs), indent=2))
