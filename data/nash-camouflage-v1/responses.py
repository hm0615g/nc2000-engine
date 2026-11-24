"""Propose legal same-shell response distributions and screen the remaining catalog."""
from copy import deepcopy
from hashlib import sha256
import json
from statistics import mean

from analyze import catalog, compose, load, paired, pools, records
from screen import DATA, HERE, distribution, pair, save_games, write


def u01(label):
    return int.from_bytes(sha256(label.encode()).digest()[:8], "big") / 2**64


def draw(members, label):
    roll = u01(label)
    for t in members:
        roll -= t["weight"]
        if roll < 0:
            return t["id"]
    return members[-1]["id"]


def trials(stage, jobs, cfg, common=False):
    lookup = pools(cfg)
    rows = []
    for cell,(left,right,n) in enumerate(jobs):
        for rep in range(n):
            block=rep if common else len(rows)//2
            stem=f"nc2000/camouflage/{stage}/{block}"
            ids=[draw(lookup[name],stem+"/"+str(side)) for side,name in enumerate((left,right))]
            rows.extend(pair(stage,block,cell,ids,[left,right]))
    return rows


def scores(games, cfg):
    results = {}
    for (a,b,_),(value,_) in paired(games,cfg).items():
        results.setdefault((a,b),[]).append(value)
    return results


def ranking_key(values, name):
    # Hash tie-breaks do not depend on catalog file order.
    return -mean(values), sha256(("camo-response/"+name).encode()).digest()


def phase_two():
    screen=load("screen-summary.json")
    cfg=load("screen-pools.json")
    groups=cfg["mixtures"]
    base_names=screen["shells"]
    defs=pools(cfg)
    teams=catalog(cfg)
    groups["initial"]=compose(base_names,screen["outer"],defs)
    candidates={}
    novel_ids={}
    for k,base in enumerate(base_names):
        members=defs[base]
        scores_by_type=screen["by_type"][base]
        opponent_probs={s:w for s,w in zip(base_names,screen["outer"]) if s!=base}
        if sum(opponent_probs.values())<1e-9:
            opponent_probs=dict.fromkeys(opponent_probs,1.0)
        def strength(t):
            return sum(w*scores_by_type[t["id"]][s] for s,w in opponent_probs.items())
        ranked=sorted(members,key=strength,reverse=True)
        best,second=[t["id"] for t in ranked[:2]]
        cfg["mixtures"][base+"-tilt"]=distribution([best,second],[.70,.30])
        # Change a coverage or recovery move without changing the public roster.
        sets=deepcopy(teams[best])
        if base=="D5":
            mon=next(p for p in sets if p["species"]=="Tyranitar")
            assert mon["moves"][2] in ("Dynamic Punch", "Fire Blast")
            mon["moves"][2]="Thunderbolt"
            label="tbolt-ttar"
        elif base=="S4":
            mon=next(p for p in sets if p["species"]=="Cloyster")
            mon["moves"][mon["moves"].index("Curse")]="Explosion" if "Surf" in mon["moves"] else "Surf"
            label="surf-boom-cloyster"
        else:
            mon=next(p for p in sets if p["species"]=="Snorlax")
            mon["moves"][mon["moves"].index("Sleep Talk")]="Self-Destruct"
            label="boom-lax"
        new_id=best+"-cam1-"+label
        assert sets not in teams.values()
        cfg["teams"].append({"id":new_id,"source":best,"sets":sets})
        groups[new_id]=distribution([new_id])
        groups[base+"-mut"]=distribution([best,new_id])
        candidates[base]=[base,best,base+"-tilt",base+"-mut"]
        novel_ids[base]=new_id
    groups["apex-equal"]=distribution([t["id"] for t in defs["incumbent"]])
    candidates["incumbent"].append("apex-equal")
    members=json.loads((DATA/"nash-refresh-v1/candidate.json").read_text())["teams"]
    groups["previous"]=distribution([t["id"] for t in members],[t["weight"] for t in members])
    all_catalog=json.loads((DATA/"team-pool-v2/team-pool.json").read_text())["teams"]
    used={t["id"] for name in base_names for t in defs[name]}
    outside=[t["id"] for t in all_catalog if t["id"] not in used]
    for n in outside:groups[n]=distribution([n])
    jobs=[(name,"initial",4) for options in candidates.values() for name in options]
    jobs.append(("previous","initial",4))
    jobs.extend((name,"initial",1) for name in outside)
    write("response-pools.json",cfg)
    write("response-design.json",{"candidates":candidates,"outside":outside,"novel":novel_ids})
    save_games(HERE/"response-schedule.jsonl",trials("response",jobs,cfg))


def promotions():
    cfg=load("response-pools.json")
    design=load("response-design.json")
    samples=scores(records("response-games.jsonl"),cfg)
    nominations={}
    jobs=[]
    for base, options in design["candidates"].items():
        ordered=sorted(options[1:],key=lambda n:ranking_key(samples[n,"initial"],n))
        nominations[base]=ordered[0]
        jobs.extend([(base,"initial",6),(ordered[0],"initial",6)])
    outsiders=sorted(design["outside"],key=lambda n:ranking_key(samples[n,"initial"],n))[:3]
    jobs.extend((n,"initial",6) for n in outsiders)
    write("nominations.json",{"within_shell":nominations,"outsiders":outsiders})
    save_games(HERE/"refine-schedule.jsonl",trials("refine",jobs,cfg,common=True))


if __name__=="__main__":
    import sys
    {"screen":phase_two,"refine":promotions}[sys.argv[1]]()
