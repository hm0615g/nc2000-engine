"""Verify paired production games, fit shell weights and measure disclosure effects."""
from collections import Counter, defaultdict
import importlib.util
import json
import math
from statistics import mean

import numpy as np
from screen import A, DATA, HERE, distribution


def load(name):
    return json.loads((HERE / name).read_text())


def records(name):
    return [json.loads(line) for line in (HERE / name).read_text().splitlines()]


def catalog(cfg):
    teams = {}
    for src in ["kodoku-beq-v1/novel_evolved_teams", "team-pool-v2/team-pool", "kodoku-r3/conclusion-equilibrium-deme"]:
        for t in json.loads((DATA / (src + ".json")).read_text())["teams"]:
            teams[t["id"]] = t["sets"]
    for t in cfg["teams"]:
        assert t["id"] not in teams
        teams[t["id"]] = t["sets"]
    return teams


def pools(cfg):
    return cfg["mixtures"] | {"incumbent": [{"id": t["id"], "weight": t["weight"]} for t in A]}


def public_shell(sets):
    return tuple((p["species"], p["level"], p["gender"], bool(p["item"])) for p in sets)


def logical(row, field):
    return row[field][::(-1 if row["rowSeat"] else 1)]


def game_score(row):
    assert row["outcome"] in ("tie", "cap", "p1", "p2")
    return 0.5 if row["outcome"] in ("tie", "cap") else float(row["outcome"] == f"p{row['rowSeat']+1}")


def paired(games, cfg):
    grouped = defaultdict(list)
    all_pools, teams = pools(cfg), catalog(cfg)
    for r in games:
        assert r["rowSeat"] in (0, 1)
        assert all(x["source"] == "live" for x in r["preview"])
        assert len(bytes.fromhex(r["actionSha256"])) == 32
        names = r["mixtures"]
        for s, tid in enumerate(r["ids"]):
            members = all_pools[names[s]]
            assert any(t["id"] == tid for t in members)
            expected = {t["id"] for t in members if public_shell(teams[t["id"]]) == public_shell(teams[tid])}
            assert sorted(r["previewBeliefs"][1-s]) == sorted(expected)
        grouped[(*logical(r,"mixtures"),r["block"])].append(r)
    result = {}
    for k, pair in grouped.items():
        a,b = sorted(pair,key=lambda r:r["rowSeat"])
        assert a["rowSeat"] == 0 and b["rowSeat"] == 1
        assert a["battleSeed"] == b["battleSeed"] and a["cell"] == b["cell"]
        assert a["seeds"] == b["seeds"][::-1] and a["ids"] == b["ids"][::-1]
        result[k] = ((game_score(a)+game_score(b))/2,[a,b])
    return result


def interval(values, weights=None, span=1, alpha=.05):
    if weights is None:
        weights = [1/len(values)] * len(values)
    assert math.isclose(sum(weights), 1) and len(values) == len(weights)
    center = sum(v*w for v,w in zip(values,weights))
    radius = span * math.sqrt(math.log(2/alpha)*sum(w*w for w in weights)/2)
    return {"score":center,"blocks":len(values),"interval":[max(0 if span==1 else -1,center-radius),min(1,center+radius)]}


def fit(matrix):
    script = DATA.parent / "tools/nash-solve.py"
    spec = importlib.util.spec_from_file_location("nash_solve",script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    p = module.rm_plus(np.asarray(matrix))
    p = np.where(p >= 0.001, p, 0)
    return (p/p.sum()).tolist()


def compose(names, outer, mixture_defs):
    weights = defaultdict(float)
    for name, probability in zip(names, outer, strict=True):
        for t in mixture_defs[name]:
            weights[t["id"]] += probability * t["weight"]
    return [{"id":tid,"weight":w} for tid,w in weights.items() if w>0]


def stage_one():
    cfg = load("screen-pools.json")
    games = records("screen-games.jsonl")
    pairs = paired(games,cfg)
    defs = pools(cfg)
    names = ["D5","S4","incumbent"]
    matrix = np.full((3,3), 0.5)
    evidence = {}
    rankings = {k:defaultdict(list) for k in names}
    camouflage = {}
    for x,y in [(0,1),(1,2),(0,2)]:
        left,right = names[x],names[y]
        lw,rw = [{t["id"]:t["weight"] for t in defs[k]} for k in (left,right)]
        values,weights=[],[]
        diffs=[[],[]]
        changes=[Counter(),Counter()]
        for (a,b,k),(v,rows) in pairs.items():
            if (a,b)!=(left,right):continue
            ta,tb=rows[0]["ids"]
            values.append(v);weights.append(lw[ta]*rw[tb])
            rankings[left][ta].append((right,v*rw[tb]))
            rankings[right][tb].append((left,(1-v)*lw[ta]))
            for side,disclosure in enumerate([(ta,b,k),(a,tb,k)]):
                if disclosure not in pairs:continue
                other,controls=pairs[disclosure]
                diffs[side].append((v-other) * (1 if side==0 else -1))
                for row,control in zip(rows,controls):
                    for field in ("battleSeed","seeds","ids","rowSeat"):
                        assert row[field]==control[field]
                    own=logical(row,"preview")[side]["input"]
                    assert own==logical(control,"preview")[side]["input"]
                    changes[side]["opponent_pick_changes"]+=logical(row,"preview")[1-side]!=logical(control,"preview")[1-side]
                    changes[side]["outcome_changes"]+=row["outcome"]!=control["outcome"]
        assert len(values)==len(lw)*len(rw)
        scored=interval(values,weights)
        evidence[left+" vs "+right]=scored
        matrix[x,y]=scored["score"]; matrix[y,x]=1-scored["score"]
        for side,ds in enumerate(diffs):
            if ds:
                names2=[left,right][::(-1 if side else 1)]
                delta=interval(ds,weights,span=2)
                mixed=matrix[[x,y][side],[y,x][side]]
                camouflage[" vs ".join(names2)] = {**delta,"mixed":float(mixed),"singleton_weighted":float(mixed-delta["score"]),**changes[side]}
    inner_scores={}
    for name,types in rankings.items():
        inner_scores[name]={}
        for tid,obs in types.items():
            per_shell=defaultdict(float)
            for opponent,contribution in obs:per_shell[opponent]+=contribution
            inner_scores[name][tid]=dict(per_shell)
    return {"shells":names, "matrix":matrix.tolist(),"outer":fit(matrix),"matchups":evidence,"camouflage":camouflage,"by_type":inner_scores}


if __name__ == "__main__":
    print(json.dumps(stage_one(),separators=(",",":")))
