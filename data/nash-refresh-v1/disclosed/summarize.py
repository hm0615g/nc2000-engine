"""Summarize fresh seed pairs after conditioning on the published team distributions."""
import hashlib
import json
import math
from pathlib import Path
from statistics import mean

HERE = Path(__file__).resolve().parent


def read(path):
    return json.loads(path.read_text())


def shell(sets):
    return sorted((p["species"], p["level"], p["gender"], bool(p["item"])) for p in sets)


def score(record):
    if record["outcome"] in ("tie", "cap"):
        return 0.5
    assert record["outcome"] in ("p1", "p2")
    return int(record["outcome"] == f"p{1 + record['rowSeat']}")


def main():
    data = HERE.parent.parent
    candidate = read(HERE.parent / "candidate.json")["teams"]
    incumbent = read(data / "kodoku-r3/conclusion-equilibrium-deme.json")["teams"]
    deme = next(t for t in read(data / "team-pool-v2/team-pool.json")["teams"] if t["id"] == "deme-r1-00-s0030")
    mixtures = {"candidate": candidate, "incumbent": incumbent, "deme": [deme]}
    games_path = HERE / "games.jsonl"
    records = [json.loads(s) for s in games_path.read_text().splitlines()]
    assert hashlib.sha256(games_path.read_bytes()).hexdigest() == read(HERE / "manifest.json")["gamesSha256"]
    assert [r["index"] for r in records] == list(range(50))
    used_seeds = set()
    summary = {}
    for opponent, n in [("deme", 3), ("incumbent", 2)]:
        rows = [r for r in records if r["mixtures"][1-r["rowSeat"]] == opponent]
        paired = {}
        for r in rows:
            side = r["rowSeat"]
            assert r["mixtures"][side] == "candidate"
            assert all(p["source"] == "live" for p in r["preview"])
            for i, (team_id, pool_name) in enumerate(zip(r["ids"], r["mixtures"])):
                pool = mixtures[pool_name]
                actual = next(t for t in pool if t["id"] == team_id)
                allowed = sorted(t["id"] for t in pool if shell(t["sets"]) == shell(actual["sets"]))
                assert sorted(r["previewBeliefs"][1-i]) == allowed
            paired.setdefault(r["block"], []).append(r)
        strata = {t["id"]: [] for t in candidate}
        for block in paired.values():
            a, b = sorted(block, key=lambda r: r["rowSeat"])
            assert (a["rowSeat"], b["rowSeat"]) == (0, 1)
            assert a["battleSeed"] == b["battleSeed"] and a["seeds"] == b["seeds"][::-1]
            assert a["ids"] == b["ids"][::-1] and a["mixtures"] == b["mixtures"][::-1]
            assert a["battleSeed"] not in used_seeds
            used_seeds.add(a["battleSeed"])
            strata[a["ids"][0]].append((score(a) + score(b)) / 2)
        assert all(len(v) == n for v in strata.values())
        estimate = sum(t["weight"] * mean(strata[t["id"]]) for t in candidate)
        # Each independent seed block contributes a [0,1] pair score. Reweight
        # within each candidate type; the two seats are never independent draws.
        half_width = math.sqrt(math.log(40) / 2 * sum(t["weight"]**2 / n for t in candidate))
        summary[opponent] = {
            "wins": sum(score(r) == 1 for r in rows),
            "losses": sum(score(r) == 0 for r in rows),
            "draws": sum(score(r) == 0.5 for r in rows),
            "candidate_score": estimate,
            "pair_hoeffding_95": [max(0, estimate-half_width), min(1, estimate+half_width)],
            "pair_scores_by_type": strata,
        }
    print(json.dumps(summary, separators=(",", ":")))


if __name__ == "__main__":
    main()
