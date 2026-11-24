"""Check the full crossed schedule and summarize weighted, independent seat blocks."""
import hashlib
import json
import math
from collections import Counter, defaultdict
from pathlib import Path
from statistics import mean

HERE = Path(__file__).resolve().parent
DATA = HERE.parents[1]
CANDIDATE = json.loads((HERE.parent / "candidate.json").read_text())["teams"]
INCUMBENT = json.loads((DATA / "kodoku-r3/conclusion-equilibrium-deme.json").read_text())["teams"]
WEIGHTS = {4*i+j: a["weight"]*b["weight"] for i, a in enumerate(CANDIDATE) for j, b in enumerate(INCUMBENT)}
ARMS = ("current/current", "old/current", "current/old", "old/old")


def lines(path):
    return [json.loads(s) for s in path.read_text().splitlines()]


def arm(row):
    versions = row["policies"][::1 if row["rowSeat"] == 0 else -1]
    return "/".join(versions)


def key(row):
    return arm(row), row["block"], row["rowSeat"]


def score(row):
    if row["outcome"] in ("tie", "cap"):
        return 0.5
    assert row["outcome"] in ("p1", "p2")
    return float(row["outcome"] == f"p{1 + row['rowSeat']}")


def shell(team):
    return sorted((p["species"], p["level"], p["gender"], bool(p["item"])) for p in team["sets"])


def weighted(values, width=1):
    cells = defaultdict(list)
    for block, v in values.items():
        cells[block % 20].append(v)
    assert cells.keys() == WEIGHTS.keys()
    est = sum(w*mean(cells[c]) for c,w in WEIGHTS.items())
    radius = width*math.sqrt(math.log(40)/2 * sum(w*w/len(cells[c]) for c,w in WEIGHTS.items()))
    lo = 0 if width == 1 else -1
    return {"score": est, "pair_hoeffding_95": [max(lo, est-radius), min(1, est+radius)],
            "blocks": len(values)}


def main():
    expected = lines(HERE / "schedule.jsonl")
    games = lines(HERE / "games.jsonl")
    by_key = {key(r): r for r in games}
    assert len(games) == len(expected) == len(by_key) == 400
    pools = {"candidate": CANDIDATE, "incumbent": INCUMBENT}
    for e in expected:
        r = by_key[key(e)]
        assert all(r[k] == v for k,v in e.items())
        assert len(bytes.fromhex(r["actionSha256"])) == 32
        for side in range(2):
            pool = pools[r["mixtures"][side]]
            team = next(t for t in pool if t["id"] == r["ids"][side])
            assert sorted(r["previewBeliefs"][1-side]) == sorted(t["id"] for t in pool if shell(t) == shell(team))
            assert r["preview"][side]["source"] == "live"
    blocks = {}
    summaries = {}
    for name in ARMS:
        pairs = {}
        rows = [r for r in games if arm(r) == name]
        for r in rows:
            if r["rowSeat"] == 1:
                continue
            other = by_key[name, r["block"], 1]
            assert r["ids"] == other["ids"][::-1] and r["seeds"] == other["seeds"][::-1]
            assert r["battleSeed"] == other["battleSeed"]
            pairs[r["block"]] = (score(r)+score(other))/2
        blocks[name] = pairs
        summaries[name] = {**weighted(pairs), "outcomes": dict(Counter(score(r) for r in rows)),
                           "caps": sum(r["outcome"]=="cap" for r in rows)}
    current_blocks = blocks[ARMS[0]]
    matched_current = weighted({b: x for b, x in current_blocks.items() if b < 40})
    cell_scores = [[mean(current_blocks[20*r + 4*i + j] for r in range(4))
                    for j in range(len(INCUMBENT))] for i in range(len(CANDIDATE))]
    contrasts = {}
    for a,b in [(ARMS[0],ARMS[1]),(ARMS[0],ARMS[2]),(ARMS[0],ARMS[3]),
                (ARMS[2],ARMS[3]),(ARMS[1],ARMS[3])]:
        shared = sorted(blocks[a].keys() & blocks[b].keys())
        diffs = {n:blocks[a][n]-blocks[b][n] for n in shared}
        same_pick = [0,0]
        flips = Counter()
        for n in shared:
            for seat in range(2):
                x,y = by_key[a,n,seat],by_key[b,n,seat]
                flips[score(x)-score(y)] += 1
                for i in range(2):
                    side = seat if i==0 else 1-seat
                    same_pick[i]+=x["preview"][side]["input"]==y["preview"][side]["input"]
        contrasts[a+" minus "+b]={**weighted(diffs,2),"outcome_changes":dict(flips),
                                  "same_candidate_and_incumbent_picks":same_pick}
    result={"games_sha256":hashlib.sha256((HERE/"games.jsonl").read_bytes()).hexdigest(),
            "arms":summaries, "matched_current":matched_current, "paired_differences":contrasts,
            "current_cell_scores":{"rows":[t["id"] for t in CANDIDATE],
                                   "columns":[t["id"] for t in INCUMBENT],"values":cell_scores}}
    print(json.dumps(result,separators=(",",":")))


if __name__ == "__main__":
    main()
