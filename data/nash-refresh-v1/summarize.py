"""Verify the seat-paired records and summarize the restricted Nash refresh."""
import hashlib
import importlib.util
import itertools
import json
from collections import Counter
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def read_json(path):
    return json.loads(path.read_text())


def games(phase):
    records = [json.loads(line) for line in (HERE / f"{phase}.jsonl").read_text().splitlines()]
    assert [r["index"] for r in records] == list(range(len(records)))
    return records


def score(record):
    outcome = record["outcome"]
    if outcome in ("tie", "cap"):
        return 0.5
    assert outcome in ("p1", "p2")
    return float(outcome == f"p{record['rowSeat'] + 1}")


def paired(rows):
    a, b = sorted(rows, key=lambda r: r["rowSeat"])
    assert [a["rowSeat"], b["rowSeat"]] == [0, 1]
    assert a["ids"] == b["ids"][::-1] and a["seeds"] == b["seeds"][::-1]
    assert a["battleSeed"] == b["battleSeed"]
    return (score(a) + score(b)) / 2


def payoff_matrix(scores):
    matrix = np.full((4, 4), 0.5)
    for cell, (i, j) in enumerate(itertools.combinations(range(4), 2)):
        matrix[i, j] = scores[:, cell].mean()
        matrix[j, i] = 1 - matrix[i, j]
    return matrix


def fitted_mix(matrix):
    spec = importlib.util.spec_from_file_location("nash_solve", ROOT / "tools/nash-solve.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.rm_plus(matrix)


def preview_result(records, weights):
    # Reuse a control when the compiled picker already chose the intervention.
    actions = []
    deltas = {}
    for pair in read_json(HERE / "preview-pairs.json"):
        control, test = [records[p][i] for p, i in (pair["control"], pair["test"])]
        side = pair["rowSeat"]
        assert all(control[k] == test[k] for k in ("rowSeat", "ids", "seeds", "battleSeed", "block"))
        assert pair["block"] == control["block"] and side == control["rowSeat"]
        assert control["preview"][1-side] == test["preview"][1-side]
        assert test["preview"][side]["input"] == "team 2, 1, 4"
        actions.append((pair["opponent"], score(control), score(test)))
        block = pair["block"]
        deltas[block] = deltas.get(block, 0.0) + (score(test) - score(control)) / 2
    strata = [np.array([[c, t] for j, c, t in actions if j == i]) for i in range(4)]
    assert all(s.shape == (4, 2) for s in strata)
    estimates = weights @ np.array([s.mean(axis=0) for s in strata])
    return {"control": estimates[0], "forced": estimates[1],
            "scores_by_opponent": [s.sum(axis=0).tolist() for s in strata],
            "seat_pair_deltas": [deltas[k] for k in sorted(deltas)]}


def summarize():
    records = {p: games(p) for p in ("pilot", "preview", "holdout", "response", "expansion")}
    manifest = read_json(HERE / "manifest.json")
    for phase, rows in records.items():
        digest = hashlib.sha256((HERE / f"{phase}.jsonl").read_bytes()).hexdigest()
        assert manifest["phases"][phase] == {"games": len(rows), "sha256": digest}
    assert sum(map(len, records.values())) == 178
    # Cells 0, 1 and 2 test the first incumbent against each of its three peers.
    first_pairs = [next(r["ids"] for r in records["pilot"] if r["cell"] == c and r["rowSeat"] == 0)
                   for c in range(3)]
    ids = [first_pairs[0][0], *(pair[1] for pair in first_pairs)]
    old = np.array([0.40, 0.30, 0.15, 0.15])
    scores = np.empty((8, 7))
    for block, cell in itertools.product(range(8), range(7)):
        group = [r for r in records["pilot"] if (r["block"], r["cell"]) == (block, cell)]
        scores[block, cell] = paired(group)
    for cell, (i, j) in enumerate(itertools.combinations(range(4), 2)):
        assert all(r["ids"][r["rowSeat"]] == ids[i] and r["ids"][1-r["rowSeat"]] == ids[j]
                   for r in records["pilot"] if r["cell"] == cell)
    # Select a response and fit weights on the first four complete seed blocks.
    # The last four blocks test both at the same matchup cost.
    first = payoff_matrix(scores[:4])
    last = payoff_matrix(scores[4:])
    response = int(np.argmax(first @ old))
    provisional = fitted_mix(first)
    split = {"response": ids[response], "first_four_vs_old": (first @ old)[response],
             "fit_first_four": provisional.tolist(), "last_four_vs_old": (last @ old)[response],
             "last_four_vs_provisional": (last @ provisional)[response]}
    matrix = payoff_matrix(scores)
    fit = fitted_mix(matrix)
    # Every incumbent faces two different Sample07 variants on independent seed blocks.
    external = [[] for _ in ids]
    assert {r["battleSeed"] for r in records["pilot"]}.isdisjoint(r["battleSeed"] for r in records["holdout"])
    for block in range(8):
        group = [r for r in records["holdout"] if r["block"] == block]
        i = ids.index(group[0]["ids"][1-group[0]["rowSeat"]])
        external[i].append(paired(group))
    assert all(len(x) == 2 for x in external)
    challenger = np.mean(external, axis=1)
    # Fresh seat pairs against the first-half best response. The mirror term is exact.
    by_type = [[] for _ in ids]
    for block in range(8):
        group = [r for r in records["response"] if r["block"] == block]
        assert all(r["ids"][r["rowSeat"]] == ids[response] for r in group)
        opponent = group[0]["ids"][1-group[0]["rowSeat"]]
        by_type[ids.index(opponent)].append(paired(group))
    assert [len(v) for v in by_type] == [3, 3, 0, 2]
    response_scores = np.array([np.mean(v) if v else 0.5 for v in by_type])
    fresh = {"response": ids[response], "paired_scores_by_type": by_type,
             "vs_old": response_scores @ old, "vs_provisional": response_scores @ provisional,
             "vs_full_fit": response_scores @ fit}
    # Fit the expanded game after the holdout checks. Preserve the fresh balanced
    # four-variant estimate against the incumbent that won the pilot.
    expansion = {}
    for cell in (11, 12):
        expansion[cell] = [paired([r for r in records["expansion"] if r["block"] == b and r["cell"] == cell])
                           for b in range(4)]
    adjusted = matrix.copy()
    for i, values in enumerate(by_type):
        if values:
            estimate = (8 * matrix[response, i] + sum(values)) / (8 + len(values))
            adjusted[response, i] = estimate
            adjusted[i, response] = 1 - estimate
    five = np.full((5, 5), 0.5)
    five[:4, :4] = adjusted
    five[4, :4] = challenger
    five[4, response] = np.mean(expansion[11])
    five[:4, 4] = 1 - five[4, :4]
    x = fitted_mix(five)
    expanded = {"ids": [*ids, "Sample07-beq-4var-shell"], "payoffs": five.tolist(),
                "weights": x.tolist(), "empirical_response_margin": max(five @ x) - 0.5,
                "fresh_sample07_vs_incumbent_leader": expansion[11],
                "fresh_sample07_vs_deme": expansion[12]}
    counts = {p: {"games": len(rows), "outcomes": dict(Counter(r["outcome"] for r in rows)),
                  "previews": dict(Counter(a["source"] for r in rows for a in r["preview"]))}
              for p, rows in records.items()}
    return {"ids": ids, "old_mix": old.tolist(), "matrix": matrix.tolist(),
            "restricted_fit": fit.tolist(), "responses_to_old": (matrix @ old).tolist(), "split_check": split,
            "sample07_pilot": scores[:, 6].mean(), "sample07_holdout_by_type": challenger.tolist(),
            "sample07_holdout_vs_old": challenger @ old, "sample07_holdout_vs_fit": challenger @ fit,
            "expanded_fit": expanded, "fresh_response": fresh, "preview": preview_result(records, old), "counts": counts}


if __name__ == "__main__":
    print(json.dumps(summarize(), separators=(",", ":")))
