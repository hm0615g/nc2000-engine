"""Verify the independent seeds, equal-budget contrasts and preview counterfactuals."""
from collections import Counter, defaultdict
import hashlib
import json
import math
from statistics import variance

from summarize import HERE, WEIGHTS, arm, key, lines, score, weighted


def pairs(rows):
    groups = defaultdict(list)
    for row in rows:
        groups[row["block"]].append(row)
    result = {}
    for block, games in groups.items():
        a, b = sorted(games, key=lambda r: r["rowSeat"])
        assert [a["rowSeat"], b["rowSeat"]] == [0, 1]
        assert a["battleSeed"] == b["battleSeed"] and a["ids"] == b["ids"][::-1]
        assert a["seeds"] == b["seeds"][::-1] and a["policies"] == b["policies"][::-1]
        result[block] = (score(a) + score(b)) / 2
    return result


def verify_base(row, base):
    for field in ("block", "cell", "rowSeat", "ids", "mixtures", "previewBeliefs"):
        assert row[field] == base[field]
    assert len(bytes.fromhex(row["actionSha256"])) == 32


def counts(rows):
    return {"wins": sum(score(r) == 1 for r in rows), "losses": sum(score(r) == 0 for r in rows),
            "ties": sum(r["outcome"] == "tie" for r in rows), "caps": sum(r["outcome"] == "cap" for r in rows)}


def main():
    games = lines(HERE / "games.jsonl")
    primary = [r for r in games if arm(r) == "current/current"]
    baseline = {key(r): r for r in games}
    repeat = lines(HERE / "repeat-games.jsonl")
    assert len(repeat) == 80
    for r in repeat:
        base = baseline[key(r)]
        verify_base(r, base)
        assert r["block"] < 40 and arm(r) == "current/current"
        assert all(p["source"] == "live" for p in r["preview"])
        digest = hashlib.sha256(f"nc2000/disclosed-apex/20261119/repeat/{r['block']}".encode()).digest()
        numbers = [int.from_bytes(digest[i:i+4], "big") for i in (0, 4, 8)]
        assert r["seeds"][::(-1 if r["rowSeat"] else 1)] == numbers[:2]
        assert r["battleSeed"] == numbers[2]
    assert not {r["battleSeed"] for r in primary} & {r["battleSeed"] for r in repeat}
    joint = pairs(primary) | {80+b: v for b, v in pairs(repeat).items()}
    assert len(joint) == 120
    pooled = weighted(joint)
    radius = math.sqrt(math.log(80) / 2 * sum(w*w/6 for w in WEIGHTS.values()))
    pooled["two_look_simultaneous_95"] = [max(0, pooled["score"]-radius), min(1, pooled["score"]+radius)]
    cells = defaultdict(list)
    for b, v in joint.items():
        cells[b % 20].append(v)
    pooled["observed_stratified_se"] = math.sqrt(sum(
        WEIGHTS[c]**2 * variance(values) / len(values) for c, values in cells.items()
    ))
    budget = lines(HERE / "budget-games.jsonl")
    budget_rows = defaultdict(list)
    refs = {"current/current81": ("current/old", "current/current"), "current81/old": ("old/old", "current/old")}
    for r in budget:
        name = arm(r)
        assert name in refs and 0 <= r["block"] < 20
        base = baseline["current/current", r["block"], r["rowSeat"]]
        verify_base(r, base)
        assert r["battleSeed"] == base["battleSeed"] and r["seeds"] == base["seeds"]
        assert all(x["source"] == "live" for x in r["preview"])
        budget_rows[name].append(r)
    budget_summary = {}
    for name, ref_arms in refs.items():
        rows = budget_rows[name]
        assert len(rows) == 40
        values = pairs(rows)
        comparisons = {}
        for ref in ref_arms:
            ref_values = pairs([r for r in games if arm(r) == ref and r["block"] < 20])
            comparisons[ref] = {"reference": weighted(ref_values),
                                "difference": weighted({b: v-ref_values[b] for b, v in values.items()}, 2)}
        budget_summary[name] = {**weighted(values), **counts(rows), "relative_to": comparisons}

    forced = lines(HERE / "pick-games.jsonl")
    by_contrast = defaultdict(Counter)
    for r in forced:
        base = baseline[key(r)]
        verify_base(r, base)
        assert r["battleSeed"] == base["battleSeed"] and r["seeds"] == base["seeds"]
        annotation = r["selection_diagnostic"]
        if annotation.get("control"):
            assert r["actionSha256"] == base["actionSha256"]
            assert r["outcome"] == base["outcome"] and r["turns"] == base["turns"]
            continue
        focal = baseline[annotation["focal"], r["block"], r["rowSeat"]]
        assert annotation["ref"] == arm(r)
        assert r["picks"] == [p["input"] for p in focal["preview"]]
        assert all(x["source"] == "forced" for x in r["preview"])
        assert [p["input"] for p in r["preview"]] == r["picks"]
        stats = by_contrast[arm(r) + " with " + annotation["focal"] + " picks"]
        stats["games"] += 1
        stats["changed_picks"] += r["picks"] != [p["input"] for p in base["preview"]]
        stats["outcome_changed_by_picks"] += score(r) != score(base)
        stats["initial_disagreements"] += score(base) != score(focal)
        stats["disagreements_after_override"] += score(r) != score(focal)
        stats["base_candidate_points"] += score(base)
        stats["override_candidate_points"] += score(r)
        stats["focal_candidate_points"] += score(focal)
    assert sum(bool(r["selection_diagnostic"].get("control")) for r in forced) == 1
    result = {"independent_repeat": {**weighted(pairs(repeat)), **counts(repeat)},
              "pooled": pooled, "equal_budget": budget_summary, "forced_previews": by_contrast}
    print(json.dumps(result, separators=(",", ":")))


if __name__ == "__main__":
    main()
