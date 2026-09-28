import collections
import json
from pathlib import Path

ROOT = Path(__file__).parent / "results"


def rows(name):
    return [json.loads(line) for line in (ROOT / (name + ".jsonl")).read_text().splitlines()]


control = rows("no-perish-control")
candidate = rows("no-perish-escape")
assert len(control) == len(candidate) == 4
for a, b in zip(control, candidate):
    for key in ("stage", "side", "seed", "state", "actions", "best"):
        assert a[key] == b[key], key
    assert b["escape_overrides"] == 0
print("No-song controls: 4/4 exact root-statistics matches")

for file in sorted(ROOT.glob("*.jsonl")):
    data = rows(file.stem)
    if not data or "continuation" in data[0]:
        continue
    groups = collections.defaultdict(collections.Counter)
    for row in data:
        groups[(row["stage"], row["side"])][row["best"]] += 1
    print(file.stem, dict(groups))

for name, expected in [
    ("confuse-unboosted-follow-screen", 24),
    ("rest-phaze-follow-screen", 16),
    ("rest-phaze-follow-10k", 16),
]:
    data = rows(name)
    assert len(data) == expected
    groups = collections.defaultdict(dict)
    for row in data:
        c = row["continuation"]
        groups[c["forced"]][c["seed"]] = c["score"]
    print(name, {a: (sum(s.values()), len(s)) for a, s in groups.items()})
    actions = list(groups)
    for action in actions[1:]:
        a, b = groups[actions[0]], groups[action]
        assert a.keys() == b.keys()
        differences = [b[seed] - a[seed] for seed in a]
        print("  paired vs", actions[0], action, dict(collections.Counter(differences)))

for row in rows("rest-phaze-trace"):
    eligible = [r for r in row["events"] if r["event"]["side"] == 1
                and r["event"]["counter"] == 1 and r["event"]["switch_available"]
                and r["event"]["phase"] == "rollout"]
    total = sum(r["n"] for r in eligible)
    switched = sum(r["n"] for r in eligible if r["event"]["choice"].startswith("switch "))
    print("Trace: rollout escape opportunities", total, "switches", switched)

rerun = rows("rest-phaze-first-step-10k")
assert len(rerun) == 2
saved = {r["continuation"]["forced"]: r["continuation"]
         for r in rows("rest-phaze-follow-10k") if r["continuation"]["seed"] == 97001}
for row in rerun:
    c = row["continuation"]
    for key in ("seed", "score", "turns", "decisions"):
        assert c[key] == saved[c["forced"]][key]
    assert c["first_step"]["choices"][0] == "move protect"
    assert "|-activate|p1a: Misdreavus|Protect" in c["first_step"]["log"]
print("Instrumented terminal rerun: both initial actions blocked by Protect")
