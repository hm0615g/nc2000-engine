"""Test one-slot s0155 variants under the complete published party distribution."""
from copy import deepcopy
import json
from pathlib import Path
from statistics import mean
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from analyze import catalog, interval, load, paired, pools
from screen import distribution, pair, save_games

PARENT = "red-v10-s1105-r2-s0155"
FOE = "sample-17-orig"
EDITS = {
    "counter": ("Blissey", "Present", "Counter"),
    "roar": ("Suicune", "Sleep Talk", "Roar"),
    "boom": ("Snorlax", "Sleep Talk", "Self-Destruct"),
}
ORIGINAL = "original-pool"


def read(name):
    return json.loads((HERE / name).read_text())


def write(name, value):
    (HERE / name).write_text(json.dumps(value, indent=2) + "\n")


def lines(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def rosters():
    candidate = load("conclusion-candidate.json")
    weights = [{k:t[k] for k in ("id","weight")} for t in candidate["teams"]]
    cfg = {"teams": [], "mixtures": {ORIGINAL: weights, FOE: distribution([FOE])}}
    mixes = cfg["mixtures"]
    mixes["sample07-guard"] = next(s["conditional"] for s in candidate["shells"] if s["family"]=="Sample07")
    base = catalog(cfg)[PARENT]
    for key, (mon, old, new) in EDITS.items():
        team = deepcopy(base)
        moves = next(p["moves"] for p in team if p["species"] == mon)
        moves[moves.index(old)] = new
        tid = PARENT + "-" + key
        cfg["teams"].append({"id": tid, "source": PARENT, "sets": team})
        mixes["shell-" + key] = distribution([PARENT, tid])
        for prefix, fraction in [("pure-", 1.), ("mixed-", .5)]:
            pool = []
            for t in weights:
                if t["id"] != PARENT:
                    pool.append(t)
                else:
                    if fraction < 1:
                        pool.append(dict(t, weight=t["weight"]*(1-fraction)))
                    pool.append(dict(id=tid, weight=t["weight"]*fraction))
            mixes[prefix + key] = pool
    return cfg


def realized(profile, block):
    if profile == ORIGINAL:
        return PARENT
    mode, key = profile.split("-", 1)
    return PARENT if mode == "mixed" and block % 2 == 0 else PARENT + "-" + key


def add(rows, stage, block, profile, tid, opponent=FOE, opposing_id=FOE):
    rows.extend(pair("s0155-"+stage, block, block, [tid, opposing_id], [profile, opponent]))


def screen():
    write("pools.json", rosters())
    rows = []
    for profile in [ORIGINAL] + [p+k for k in EDITS for p in ("pure-", "mixed-")]:
        for b in range(4):
            add(rows,"screen",b,profile,realized(profile,b))
    save_games(HERE/"screen-schedule.jsonl", rows)


def scores(stage):
    schedule = lines(HERE/(stage+"-schedule.jsonl"))
    games = lines(HERE/(stage+"-games.jsonl"))
    cfg = read("pools.json")
    assert len(games) == len(schedule)
    for task, r in zip(schedule,games,strict=True):
        assert all(r[k]==v for k,v in task.items())
    return paired(games,cfg)


def control(profile,b):
    return ORIGINAL if realized(profile,b) == PARENT else "pure-" + profile.split("-",1)[1]


def disclosure_contrast(matches, profile, blocks):
    deltas = []
    switches = 0
    changed_results = 0
    for b in blocks:
        hidden, hs = matches[profile,FOE,b]
        known, ks = matches[control(profile,b),FOE,b]
        for a,k in zip(hs,ks,strict=True):
            assert all(a[f] == k[f] for f in ("ids","battleSeed","rowSeat","seeds"))
            own = a["rowSeat"]
            assert a["preview"][own] == k["preview"][own]
            switches += a["preview"][1-own] != k["preview"][1-own]
            changed_results += a["outcome"] != k["outcome"]
        deltas.append(hidden-known)
    return {"delta":interval(deltas,span=2), "changed_opponent_selections":switches,
            "changed_results":changed_results}


def nominate():
    data = scores("screen")
    profiles = [p+k for k in EDITS for p in ("pure-", "mixed-")]
    result = {p: mean(data[p,FOE,b][0] for b in range(4)) for p in [ORIGINAL,*profiles]}
    camouflage = {p: disclosure_contrast(data,p,range(4)) for p in profiles if p.startswith("mixed-")}
    best = max(profiles, key=lambda p: (result[p], p.startswith("pure-")))
    nominee = best if result[best] > result[ORIGINAL] else None
    write("screen-summary.json", {"scores":result,"matched_camouflage":camouflage,"nominee":nominee})
    if not nominee:
        return
    cfg = read("pools.json")
    base = cfg["mixtures"][ORIGINAL]
    rows = []
    for b, t in enumerate(t for t in base if t["id"] != PARENT):
        for p in (ORIGINAL,nominee):
            add(rows,"holdout",b,p,t["id"])
    for b in range(6,12):
        for p in (ORIGINAL,nominee):
            add(rows,"holdout",b,p,realized(p,b))
        if nominee.startswith("mixed-") and control(nominee,b)!=ORIGINAL:
            add(rows,"holdout",b,control(nominee,b),realized(nominee,b))
    block = 12
    for opponent in ("sample07-guard","incumbent"):
        for member in pools(cfg)[opponent]:
            for _ in range(2):
                for p in (ORIGINAL,nominee):
                    add(rows,"holdout",block,p,realized(p,block),opponent,member["id"])
                block += 1
    save_games(HERE/"holdout-schedule.jsonl",rows)


def comparison(matches, nominee, opponent, blocks, weights):
    old, new = [], []
    for b in blocks:
        base_score, base_rows = matches[ORIGINAL,opponent,b]
        revised_score, revised_rows = matches[nominee,opponent,b]
        for base, revised in zip(base_rows,revised_rows,strict=True):
            for key in ("rowSeat","seeds","battleSeed"):
                assert base[key] == revised[key]
            opposing_seat = 1-base["rowSeat"]
            assert base["ids"][opposing_seat] == revised["ids"][opposing_seat]
        old.append(base_score)
        new.append(revised_score)
    return {"original":interval(old,weights),"revised":interval(new,weights),
            "delta":interval([n-o for n,o in zip(new,old)],weights,span=2)}


def summarize():
    cfg = read("pools.json")
    matches = scores("holdout")
    nominee = read("screen-summary.json")["nominee"]
    members = cfg["mixtures"][ORIGINAL]
    siblings = [t for t in members if t["id"] != PARENT]
    mass = next(t["weight"] for t in members if t["id"] == PARENT)
    weights = [t["weight"] for t in siblings] + [mass/6]*6
    overall = comparison(matches,nominee,FOE,range(12),weights)
    shell = comparison(matches,nominee,FOE,range(6,12),[1/6]*6)
    guards = {}
    start = 12
    for opponent in ("sample07-guard","incumbent"):
        weights = [t["weight"]/2 for t in pools(cfg)[opponent] for _ in range(2)]
        guards[opponent] = comparison(matches,nominee,opponent,range(start,start+len(weights)),weights)
        start += len(weights)
    camo = None
    if nominee.startswith("mixed-"):
        camo = disclosure_contrast(matches,nominee,range(6,12))
    promote = (overall["delta"]["score"] > 1e-12 and shell["delta"]["score"] > 0
               and all(g["delta"]["score"] >= -1e-12 for g in guards.values()))
    write("result.json",{"nominee":nominee,"pool_vs_sample17":overall,"s0155_vs_sample17":shell,
                         "s0155_vs_guards":guards,"s0155_camouflage":camo,"promote":promote})
    if promote:
        candidate = load("conclusion-candidate.json")
        by_id = catalog(cfg)
        candidate["profile"] = nominee
        members = cfg["mixtures"][nominee]
        candidate["teams"] = [t | {"sets": by_id[t["id"]]} for t in members]
        group = next(s for s in candidate["shells"] if s["family"] == PARENT)
        group["conditional"] = [dict(t,weight=t["weight"]/mass) for t in members
                                if t["id"].startswith(PARENT)]
        write("candidate.json",candidate)


if __name__ == "__main__":
    {"screen":screen, "nominate":nominate, "summarize":summarize}[sys.argv[1]]()
