"""Evaluate held-out best responses and reprice a disclosed redistribution if needed."""
from collections import Counter
from statistics import mean
import sys

from analyze import compose, interval, load, paired, pools, records
from responses import trials
from screen import HERE, write, save_games


def heldout():
    cfg=load("confirmation-pools.json")
    design=load("confirmation-design.json")
    matches=paired(records("confirmation-games.jsonl"),cfg)
    leaders=design["leaders"]
    results={}
    for n in leaders:
        values=[matches[n,"proposed",b][0] for b in range(16)]
        results[n]={**interval(values,alpha=.05/len(leaders)),
                    "wins":sum(r["outcome"]==f"p{1+r['rowSeat']}" for b in range(16) for r in matches[n,"proposed",b][1])}
    worst=max(leaders,key=lambda n:results[n]["score"])
    n=design["camouflage_opponent"]
    diff=[]
    retained=Counter()
    if len(pools(cfg)["proposed"])>1:
        for b in range(16):
            hidden,rs=matches[n,"proposed",b]
            known,ks=matches[n,rs[0]["ids"][1],b]
            diff.append(known-hidden)  # Candidate is the column player.
            for r,k in zip(rs,ks):
                assert all(r[f]==k[f] for f in ("ids","seeds","battleSeed","rowSeat"))
                side=r["rowSeat"]
                assert r["preview"][1-side]==k["preview"][1-side]
                retained["opponent_pick_changes"]+=r["preview"][side]!=k["preview"][side]
                retained["outcome_changes"]+=r["outcome"]!=k["outcome"]
    decision=results[worst]["score"]>=.65 and design["screen_scores"][worst]>.5
    return {"heldout":results,"worst":worst,"max_observed_gain":results[worst]["score"]-.5,
            "tested_upper_gain":max(r["interval"][1] for r in results.values())-.5,
            "candidate_camouflage":{**interval(diff,span=2),**retained} if diff else {"score":0,"reason":"singleton"},
            "try_redistribution":decision}


def repair():
    report=heldout();write("confirmation-summary.json",report)
    if not report["try_redistribution"]:return
    cfg=load("confirmation-pools.json")
    worst=report["worst"]
    second=next(n for n in sorted(report["heldout"],key=lambda n:-report["heldout"][n]["score"]) if n!=worst)
    paths=["proposed"]
    for percent in [25,50,75]:
        name="repair-"+str(percent)
        cfg["mixtures"][name]=compose(["proposed",worst],[1-percent/100,percent/100],pools(cfg))
        paths.append(name)
    opponents=list(dict.fromkeys([worst,second,"proposed"]))
    write("repair-design.json",{"paths":paths,"opponents":opponents})
    write("repair-pools.json",cfg)
    jobs=[(q,p,4) for p in paths for q in opponents]
    save_games(HERE/"repair-schedule.jsonl",trials("repair",jobs,cfg,common=True))


def recheck():
    from responses import scores
    cfg=load("repair-pools.json")
    design=load("repair-design.json")
    values=scores(records("repair-games.jsonl"),cfg)
    worst={p:max(mean(values[q,p]) for q in design["opponents"]) for p in design["paths"]}
    # Retain the original profile on ties.
    best=min(design["paths"],key=lambda p:(worst[p],sum(mean(values[q,p]) for q in design["opponents"])))
    write("repair-summary.json",{"max_challenger_scores":worst,"selected":best,
                                 "scores":{p:{q:mean(values[q,p]) for q in design["opponents"]} for p in worst}})
    q=load("confirmation-design.json")["leaders"]+["proposed"]
    save_games(HERE/"recheck-schedule.jsonl",trials("recheck",[(n,best,12) for n in q],cfg,common=True))


if __name__=="__main__":
    {"repair":repair,"recheck":recheck}[sys.argv[1]]()
