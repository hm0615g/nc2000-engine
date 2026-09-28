import argparse
import collections
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--scratch", type=Path, default=Path("tmp/perish-audit"))
parser.add_argument("--out", type=Path, default=Path(__file__).with_name("results.json"))
args = parser.parse_args()

cohorts = {
    "blind_offense_screen": ("t27-s0", "t27-s0-after", range(81001, 81005)),
    "blind_offense_holdout": ("t27-holdout-before", "t27-holdout-after", range(82001, 82009)),
    "open_reconstructed_offense_holdout": ("t27-open-before", "t27-open-after", range(82001, 82009)),
    "blind_expiry_screen": ("t30-s0", "t30-s0-after", range(81001, 81005)),
    "blind_defense_control": ("t28-s1", "t28-s1-after", range(81001, 81005)),
    "open_t11_control": ("control-t11-before", "control-t11-after", range(81001, 81005)),
}
result = {}
for cohort, (before, after, seeds) in cohorts.items():
    rows = []
    choices = {arm: collections.Counter() for arm in ("before", "after")}
    for seed in seeds:
        pair = []
        row = {"seed": seed}
        for arm, directory in (("before", before), ("after", after)):
            path = args.scratch / directory / f"seed-{seed}.json"
            raw = path.read_bytes()
            data = json.loads(raw)
            assert data["ordinary_control_equal"]
            assert sum(a["visits"] for a in data["actions"]) == data["iterations"]
            assert sum(s["n"] for s in data["root_states"]) == data["iterations"]
            row[arm] = {key: data[key] for key in (
                "best", "iterations", "profile", "c", "actions", "root_states", "leaf_replays"
            )}
            row[arm]["artifact_sha256"] = hashlib.sha256(raw).hexdigest()
            choices[arm][data["best"]] += 1
            pair.append(data)
        row["actions_identical"] = pair[0]["actions"] == pair[1]["actions"]
        row["events_identical"] = pair[0]["events"] == pair[1]["events"]
        if cohort.endswith("control"):
            assert row["actions_identical"] and row["events_identical"]
        rows.append(row)
    result[cohort] = {"choices": choices, "rows": rows}
args.out.write_text(json.dumps(result, indent=2) + "\n")
for cohort, data in result.items():
    print(cohort, {arm: dict(counts) for arm, counts in data["choices"].items()})
