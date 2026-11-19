"""Export the final public distribution and independently audit the recorded games."""
from collections import Counter, defaultdict
from hashlib import sha256
import json
import math
from pathlib import Path
import subprocess
import tempfile

from analyze import catalog, interval, load, paired, pools, public_shell, records
from screen import HERE, write

CONFIGS = {
    "screen": "screen", "response": "response", "refine": "response", "fit": "response",
    "deviation": "final", "confirmation": "confirmation", "repair": "repair", "recheck": "repair",
}
TASK_FIELDS = ("ids", "seeds", "battleSeed", "rowSeat", "cell", "mixtures", "block")


def task_key(row):
    return json.dumps([row[k] for k in TASK_FIELDS], separators=(",", ":"))


def audit(phases):
    counts = {}
    seen_seeds = set()
    for stage in phases:
        games = records(stage + "-games.jsonl")
        jobs = records(stage + "-schedule.jsonl")
        if stage == "screen":
            jobs += records("apex-controls.jsonl")
        expected = Counter(map(task_key, jobs))
        assert all(count == 1 for count in expected.values()), stage
        assert Counter(map(task_key, games)) == expected, stage
        paired(games, load(CONFIGS[stage] + "-pools.json"))
        seeds = {r["battleSeed"] for r in games}
        assert not (seeds & seen_seeds), stage
        seen_seeds.update(seeds)
        counts[stage] = {"games": len(games), "battle_seeds": len(seeds),
                         "ties": sum(r["outcome"] == "tie" for r in games),
                         "turn_caps": sum(r["outcome"] == "cap" for r in games),
                         "sha256": sha256((HERE / (stage + "-games.jsonl")).read_bytes()).hexdigest()}
    return counts


def export(cfg, name):
    definitions = pools(cfg)
    teams = catalog(cfg)
    members = definitions[name]
    assert math.isclose(sum(t["weight"] for t in members), 1)
    families = {public_shell(teams[definitions[group][0]["id"]]): label
                for group, label in [("D5", "Deme"), ("S4", "Sample07"), ("incumbent", "APEX4")]}
    shells = defaultdict(list)
    for t in members:
        shells[public_shell(teams[t["id"]])].append(t)
    details = []
    check = {}
    for roster, by_type in shells.items():
        family = families.get(roster, by_type[0]["id"])
        weight = sum(t["weight"] for t in by_type)
        conditional = [dict(id=t["id"], weight=t["weight"]/weight) for t in by_type]
        check[family] = conditional
        details.append({"family": family, "weight": weight, "conditional": conditional})
    # Validate every released set and compare complete opponent frames on both sides.
    verifier_input = dict(teams=cfg["teams"], mixtures=check)
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "public.json"
        path.write_text(json.dumps(verifier_input))
        subprocess.run(["node", str(HERE / "verify-frames.cjs"), str(path), *check],
                       stdout=subprocess.PIPE, check=True)
    return {"name": "NC2000 disclosed pool search", "profile": name,
            "status": "restricted empirical candidate, research only",
            "shells": details, "teams": [t | {"sets": teams[t["id"]]} for t in members]}


def main():
    confirmation = load("confirmation-summary.json")
    repaired = confirmation["try_redistribution"]
    phases = list(CONFIGS) if repaired else list(CONFIGS)[:-2]
    counts = audit(phases)
    name = load("repair-summary.json")["selected"] if repaired else "proposed"
    cfg = load(("repair" if repaired else "confirmation") + "-pools.json")
    candidate = export(cfg, name)
    if repaired:
        comparisons = paired(records("recheck-games.jsonl"), cfg)
        groups = defaultdict(list)
        for (q, p, _), (score, _) in comparisons.items():
            assert p == name
            groups[q].append(score)
        assert all(len(v) == 12 for v in groups.values())
        evaluations = {q: interval(v, alpha=.05/len(groups)) for q, v in groups.items()}
    else:
        evaluations = confirmation["heldout"]
    opponents = {q: result for q, result in evaluations.items() if q != name}
    largest = max(opponents, key=lambda k: opponents[k]["score"])
    write("conclusion-candidate.json", candidate)
    write("results.json", {
        "candidate": name, "redistribution_tested": repaired, "worst_measured_opponent": largest,
        "largest_observed_gain": opponents[largest]["score"] - .5,
        "upper_gain_on_tested_shortlist": max(r["interval"][1] for r in opponents.values()) - .5,
        "heldout_against_final_pool": evaluations, "phases": counts,
        "games": sum(n["games"] for n in counts.values()),
        "outside_parties_screened": len(load("response-design.json")["outside"]),
        "direct_deviations_screened": len(load("confirmation-design.json")["screen_scores"]),
        "bounds": "Two-sided simultaneous 95% Hoeffding intervals on independent seat-pair scores, Bonferroni across the final held-out opponents.",
    })


if __name__ == "__main__":
    main()
