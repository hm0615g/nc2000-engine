import argparse
import itertools
import json
import math
import statistics
from collections import defaultdict
from pathlib import Path


def fisher_exact_p(wins_a, losses_a, wins_b, losses_b):
    n_a, n_b, wins = wins_a + losses_a, wins_b + losses_b, wins_a + wins_b
    total = math.comb(n_a + n_b, wins)
    prob = lambda k: math.comb(n_a, k) * math.comb(n_b, wins - k) / total
    observed = prob(wins_a)
    support = range(max(0, wins - n_b), min(n_a, wins) + 1)
    return min(1, sum(prob(k) for k in support if prob(k) <= observed * (1 + 1e-9)))


def unpaired(values_a, values_b):
    """Independent samples, e.g. games against a human: no trial is shared."""
    if None in values_a or None in values_b or not values_a or not values_b:
        return None
    mean_a, mean_b = statistics.mean(values_a), statistics.mean(values_b)
    var = lambda v, m: sum((x - m) ** 2 for x in v) / (len(v) - 1) if len(v) > 1 else None
    var_a, var_b = var(values_a, mean_a), var(values_b, mean_b)
    difference = mean_a - mean_b
    se = math.sqrt(var_a / len(values_a) + var_b / len(values_b)) if var_a is not None and var_b is not None else None
    binary = all(v in (0, 1) for v in values_a + values_b)
    return dict(
        trials=[len(values_a), len(values_b)], difference=difference,
        normal95_interval=[difference - 1.96 * se, difference + 1.96 * se] if se else None,
        exact_fisher_p=fisher_exact_p(values_a.count(1), values_a.count(0),
                                      values_b.count(1), values_b.count(0)) if binary else None,
    )


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
            if not keys:
                pair.update(difference=None, normal95_interval=None, exact_mcnemar_p=None,
                            unpaired=unpaired(list(actions[a].values()), list(actions[b].values())))
            elif any(actions[a][k] is None or actions[b][k] is None for k in keys):
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
