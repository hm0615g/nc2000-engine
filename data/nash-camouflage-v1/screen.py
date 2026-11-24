"""Build the legal legacy-derived rosters and the paired disclosure experiment."""
from hashlib import sha256
from itertools import product
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
DATA = HERE.parent
D = "deme-r1-00-s0030"
S = [t["id"] for t in json.loads((DATA / "nash-refresh-v1/candidate.json").read_text())["teams"] if t["id"].startswith("sample-07")]
A = json.loads((DATA / "kodoku-r3/conclusion-equilibrium-deme.json").read_text())["teams"]


def write(name, obj):
    (HERE / name).write_text(json.dumps(obj, indent=2) + "\n")


def distribution(ids, weights=None):
    if weights is None:
        weights = [1/len(ids)] * len(ids)
    return [{"id": tid, "weight": weight} for tid, weight in zip(ids, weights, strict=True)]


def pair(stage, block, cell, ids, mixtures):
    digest = sha256(f"nc2000/camouflage/v1/{stage}/{block}".encode()).digest()
    seeds = [int.from_bytes(digest[i:i+4], "big") for i in (0, 4, 8)]
    return [dict(block=block, cell=cell, rowSeat=side, ids=ids[::step], mixtures=mixtures[::step],
                 seeds=seeds[:2][::step], battleSeed=seeds[2]) for side, step in enumerate((1, -1))]


def save_games(path, rows):
    path.write_text("".join(json.dumps(r, separators=(",", ":")) + "\n" for r in rows))


def main():
    legacy = json.loads((DATA / "kodoku-beq-v1/kodoku_evolution_summary.json").read_text())["final_population"]
    extra = []
    for t in legacy:
        if not t["id"].startswith(D + "-"):
            continue
        sets = t["sets"]
        raikou = next(p for p in sets if p["species"] == "Raikou")
        assert raikou["moves"][3] == "Crunch"
        raikou["moves"][3] = "Thunder" if "Sleep Talk" in raikou["moves"] else "Sleep Talk"
        extra.append({"id": t["id"]+"-legal", "source": t["id"], "sets": sets})
    deme = [D] + [t["id"] for t in extra]
    assert len(deme) == 5 and len(S) == 4
    mixes = {"D5": distribution(deme), "S4": distribution(S)}
    for t in deme + S + [a["id"] for a in A]:
        mixes[t] = distribution([t])
    write("screen-pools.json", {"teams": extra, "mixtures": mixes})
    rows = []
    for cell, (a, b) in enumerate(product(deme, S)):
        for pools in [["D5","S4"], [a,"S4"], ["D5",b]]:
            rows.extend(pair("screen", cell, cell, [a,b], pools))
    offset = 20
    for name, ids in [("S4", S), ("D5", deme)]:
        for idx, (a,b) in enumerate(product(ids, [t["id"] for t in A])):
            rows.extend(pair("screen", offset+idx, offset+idx, [a,b], [name,"incumbent"]))
        offset += len(ids) * len(A)
    save_games(HERE / "screen-schedule.jsonl", rows)
    controls = []
    for idx, (s, a) in enumerate(product(S, [t["id"] for t in A])):
        controls.extend(pair("screen", 20+idx, 20+idx, [s,a], ["S4",a]))
    save_games(HERE / "apex-controls.jsonl", controls)


if __name__ == "__main__":
    main()
