"""Construct a single disclosed pool and measure deviations against that exact pool."""
from itertools import combinations
from statistics import mean
import sys

import numpy as np

from analyze import catalog, compose, fit, interval, load, paired, pools, records
from responses import ranking_key, scores, trials
from screen import HERE, save_games, write


def finalists():
    cfg = load("response-pools.json")
    nom = load("nominations.json")
    stats = scores(records("refine-games.jsonl"), cfg)
    chosen = []
    comparisons = {}
    for base, alt in nom["within_shell"].items():
        base_scores, alt_scores = stats[base, "initial"], stats[alt, "initial"]
        selected = alt if mean(alt_scores) > mean(base_scores) else base
        chosen.append(selected)
        comparisons[base] = {"nominee":alt,"base":mean(base_scores),"nominated":mean(alt_scores),
                             "paired_delta":interval([a-b for a,b in zip(alt_scores,base_scores,strict=True)],span=2)}
    outsiders = sorted(nom["outsiders"],key=lambda n:ranking_key(stats[n,"initial"],n))
    if mean(stats[outsiders[0],"initial"]) > .5:
        chosen.append(outsiders[0])
    write("finalists.json",{"profiles":chosen,"comparisons":comparisons,"outsiders":{n:mean(stats[n,"initial"]) for n in outsiders}})
    known=load("screen-summary.json")["shells"]
    jobs=[(a,b,8) for a,b in combinations(chosen,2) if a not in known or b not in known]
    save_games(HERE/"fit-schedule.jsonl",trials("fit",jobs,cfg))


def export_candidate(name, profiles, outer, cfg):
    defs = pools(cfg)
    teams = catalog(cfg)
    parties = compose(profiles,outer,defs)
    return {"name":name, "protocol":"mutually disclosed full six-party distributions; realized sets and chosen trio hidden",
            "status":"restricted empirical candidate",
            "shells":[{"profile":k,"weight":w,"conditional":defs[k]} for k,w in zip(profiles,outer) if w>0],
            "teams":[t | {"sets":teams[t["id"]]} for t in parties]}


def assemble():
    cfg=load("response-pools.json")
    ref=load("screen-summary.json")
    selected=load("finalists.json")["profiles"]
    sample=scores(records("fit-games.jsonl"),cfg)
    matrix=np.full((len(selected),len(selected)),.5)
    for i,j in combinations(range(len(selected)),2):
        a,b=selected[i],selected[j]
        if (a,b) in sample:value=mean(sample[a,b])
        else:value=ref["matrix"][ref["shells"].index(a)][ref["shells"].index(b)]
        matrix[i,j]=value;matrix[j,i]=1-value
    outer=fit(matrix)
    cfg["mixtures"]["proposed"]=compose(selected,outer,pools(cfg))
    write("candidate.json", export_candidate("Camouflage search candidate",selected,outer,cfg))
    write("fit-summary.json",{"profiles":selected,"matrix":matrix.tolist(),"outer":outer})
    design=load("response-design.json")
    # Real, published changes from a jointly assembled multi-shell distribution.
    alternatives=["initial","previous"] + selected[:3]
    for base,options in design["candidates"].items():
        alternatives.extend([base,*options[1:]])
    alternatives.extend(design["novel"].values())
    if len(selected)>1:
        outer_tilt=[.5*w+.5*(i==int(np.argmax(outer))) for i,w in enumerate(outer)]
        cfg["mixtures"]["outer-tilt"]=compose(selected,outer_tilt,pools(cfg))
        groups_equal=[1/len(selected)]*len(selected)
        cfg["mixtures"]["outer-even"]=compose(selected,groups_equal,pools(cfg))
        alternatives.extend(["outer-tilt","outer-even"])
    # Exact duplicates are redundant, including a deviation that equals the proposed pool.
    def signature(n):
        return tuple(sorted((t["id"],round(t["weight"],10)) for t in pools(cfg)[n]))
    seen={signature("proposed")}
    jobs=[]
    probes = []
    proposed = cfg["mixtures"]["proposed"]
    support = {t["id"] for t in proposed}
    for base, mutant in design["novel"].items():
        probes.extend(t["id"] for t in pools(cfg)[base])
        if mutant not in support:
            name = "add-" + mutant
            cfg["mixtures"][name] = compose(["proposed",mutant],[.8,.2],pools(cfg))
            probes.append(name)
    if len(proposed) > 1:
        for member in proposed:
            name = "drop-" + member["id"]
            kept = [t for t in proposed if t["id"] != member["id"]]
            total = sum(t["weight"] for t in kept)
            cfg["mixtures"][name] = [dict(t, weight=t["weight"]/total) for t in kept]
            probes.append(name)
    for n in alternatives+design["outside"]+probes:
        sig=signature(n)
        if sig in seen:continue
        seen.add(sig)
        jobs.append((n,"proposed",3 if n in alternatives else 1))
    write("final-pools.json",cfg)
    save_games(HERE/"deviation-schedule.jsonl",trials("deviation",jobs,cfg))


def validate():
    cfg=load("final-pools.json")
    samples=scores(records("deviation-games.jsonl"),cfg)
    outside=set(load("response-design.json")["outside"])
    # One independently tested leader from each search track plus the overall runner-up.
    tracks=[[n for n,b in samples if n not in outside],[n for n,b in samples if n in outside]]
    ordered=[sorted(t,key=lambda n:ranking_key(samples[n,"proposed"],n)) for t in tracks]
    leads=[t[0] for t in ordered if t]
    others=sorted((n for t in ordered for n in t if n not in leads),key=lambda n:ranking_key(samples[n,"proposed"],n))
    if others:leads.append(others[0])
    jobs=[(n,"proposed",16) for n in leads]
    rows=trials("confirmation",jobs,cfg,common=True)
    # Confirm camouflage in the actual assembled pool against the leading within-shell challenger.
    control=[]
    for r in rows:
        side=r["rowSeat"]
        if r["mixtures"][side]!=leads[0]:continue
        tid=r["ids"][1-side]
        if tid not in cfg["mixtures"]:
            cfg["mixtures"][tid]=[{"id":tid,"weight":1.0}]
        names=list(r["mixtures"]);names[1-side]=tid
        if len(pools(cfg)["proposed"])>1:
            control.append({**r,"mixtures":names})
    write("confirmation-pools.json",cfg)
    write("confirmation-design.json",{"leaders":leads,"screen_scores":{n:mean(v) for (n,_),v in samples.items()},
                                      "camouflage_opponent":leads[0]})
    save_games(HERE/"confirmation-schedule.jsonl",rows+control)


if __name__=="__main__":
    {"nominate":finalists,"assemble":assemble,"validate":validate}[sys.argv[1]]()
