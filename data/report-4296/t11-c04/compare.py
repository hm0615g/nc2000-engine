import collections
import hashlib
import json
import statistics
from pathlib import Path


ROOT = Path(__file__).resolve().parent


def roots(names, seeds, coefficient):
    rows = {}
    for name in names:
        for line in (ROOT / name).read_text().splitlines():
            row = json.loads(line)
            assert row["c"] == coefficient and row["iterations"] == 30000
            assert row["seed"] not in rows
            assert sum(a["visits"] for a in row["actions"]) == 30000
            rows[row["seed"]] = row
    assert set(rows) == set(seeds)
    return rows


def root_summary(rows):
    gap = []
    for row in rows.values():
        actions = {a["action"]: a for a in row["actions"]}
        if "move earthquake" in actions and "move hiddenpowerbug" in actions:
            gap.append(actions["move hiddenpowerbug"]["mean"] - actions["move earthquake"]["mean"])
    return {
        "seeds": len(rows),
        "choices": dict(collections.Counter(r["best"] for r in rows.values())),
        "mean_hp_minus_eq": statistics.mean(gap) if gap else None,
    }


def mechanism(path):
    source = json.loads(path.read_text())
    assert sorted(map(int, source["iterations_by_seed"])) == [61001, 62001, 62002, 62003, 62004]
    output = {}
    for reply in ["all", "move encore"]:
        output[reply] = {}
        for action in ["move earthquake", "move hiddenpowerbug"]:
            groups = [r for r in source["groups"] if r["cohort"] == "all"
                      and r["reply_filter"] == reply and r["action"] == action]
            n = sum(r["starmie_count"] for r in groups)
            count = lambda move: sum(r["starmie_actions"].get(move, {}).get("n", 0) for r in groups)
            exploration = sum(r["starmie_exploration"] for r in groups)
            unvisited = sum(r["starmie_untried"] for r in groups)
            output[reply][action] = {
                "encounters": n,
                "surf": count("move surf"), "surf_fraction": count("move surf") / n,
                "recover": count("move recover"), "recover_fraction": count("move recover") / n,
                "visited_below_max_mean": exploration, "visited_below_max_mean_fraction": exploration / n,
                "unvisited": unvisited, "unvisited_fraction": unvisited / n,
                "either_exploration_fraction": (exploration + unvisited) / n,
                "mean_prior_node_samples": sum(r["starmie_mean_visits_before"] * r["starmie_count"] for r in groups) / n,
            }
    return output


def main():
    heldout = {str(c): roots([f"heldout-{name}-a.jsonl", f"heldout-{name}-b.jsonl"], range(66001, 66033), c)
               for c, name in [(1.0, "c1"), (0.4, "c04")]}
    t26 = {str(c): roots([f"t26-{name}.jsonl"], range(66001, 66017), c)
           for c, name in [(1.0, "c1"), (0.4, "c04")]}
    probe = json.loads((ROOT.parent / "t11-cause/probe-24418.json").read_text())
    held_fixed = [{"action": a["action"], "ucb_c1": a["ucb"],
                   "ucb_c04": a["mean"] + 0.4 * a["exploration_bonus"]}
                  for a in probe["selection"]["actions"]]
    paths = list(ROOT.glob("*.jsonl")) + [ROOT / "trace-summary/summary.json",
            ROOT.parent / "t11-cause/summary.json", ROOT.parent / "t11-cause/probe-24418.json"]
    result = {
        "estimand": "Same-budget OpenSheet action selection and adaptive-search behavior, not independent terminal win rates.",
        "heldout_t11": {c: root_summary(rows) for c, rows in heldout.items()},
        "paired_choices": dict(collections.Counter(
            heldout["1.0"][seed]["best"] + " -> " + heldout["0.4"][seed]["best"] for seed in heldout["1.0"])),
        "t26": {c: root_summary(rows) for c, rows in t26.items()},
        "mechanism": {
            "1.0": mechanism(ROOT.parent / "t11-cause/summary.json"),
            "0.4": mechanism(ROOT / "trace-summary/summary.json"),
        },
        "same_node_coefficient_intervention": held_fixed,
        "sha256": {str(p.relative_to(ROOT.parent)): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths},
    }
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
