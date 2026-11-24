"""Check five distinct archived rosters against the new and former distributions."""
from collections import Counter, defaultdict
from hashlib import sha256
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from analyze import catalog, interval, load, logical, paired, pools
from screen import DATA, distribution, pair, save_games

CURRENT = "current-seven"
OLD = "incumbent"
SOURCES = {
    "hc75-1st-show": ("team-inventory-v1/inventory","Misdreavus","Perish Song"),
    "pb-137": ("partybox-v0/teams","Machamp","Cross Chop"),
    "pb-122": ("partybox-v0/teams","Jolteon","Baton Pass"),
    "sample-24": ("team-inventory-v1/inventory","Charizard","Belly Drum"),
    "pb-95": ("partybox-v0/teams","Starmie","Rapid Spin"),
}


def write(name,obj):
    (HERE/name).write_text(json.dumps(obj,indent=2)+"\n")


def read(name):
    return json.loads((HERE/name).read_text())


def design():
    ref=load("conclusion-candidate.json")
    current=[dict(id=t["id"],weight=t["weight"]) for t in ref["teams"]]
    prev=load("confirmation-pools.json")
    teams=catalog(prev)
    screened=load("confirmation-design.json")["screen_scores"]
    seen={frozenset(p["species"] for p in teams[t["id"]]) for name in screened for t in pools(prev)[name]}
    cfg={"teams":[],"mixtures":{CURRENT:current}}
    for name,(file,mon,move) in SOURCES.items():
        t=next(x for x in json.loads((DATA/(file+".json")).read_text())["teams"] if x["id"]==name)
        roster=frozenset(p["species"] for p in t["sets"])
        assert roster not in seen and move in next(p for p in t["sets"] if p["species"]==mon)["moves"]
        seen.add(roster)
        cfg["teams"].append({"id":name,"sets":t["sets"]})
        cfg["mixtures"][name]=distribution([name])
    write("pools.json",cfg)
    schedule=[]
    for name in SOURCES:
        for policy in (CURRENT,OLD):
            for t in pools(cfg)[policy]:
                b=next(i for i,m in enumerate(current) if m["id"]==t["id"])
                schedule.extend(pair("coverage-screen",b,b,[name,t["id"]],[name,policy]))
    save_games(HERE/"screen-schedule.jsonl",schedule)


def holdout_schedule(name,cfg,stage="holdout",blocks=20):
    # Couple the common APEX4 draw exactly. Split the remaining probability at
    # both distributions' cumulative type boundaries to preserve both marginals.
    cur,old=pools(cfg)[CURRENT],pools(cfg)[OLD]
    common={t["id"]:t["weight"] for t in cur if t["id"] in {v["id"] for v in old}}
    strata=[(i,i,w) for i,w in common.items()]
    left=[(t["id"],t["weight"]) for t in cur if t["id"] not in common]
    right=[(t["id"],t["weight"]-common[t["id"]]) for t in old]
    i=j=0
    a,b=left[0][1],right[0][1]
    while i<len(left) and j<len(right):
        w=min(a,b);strata.append((left[i][0],right[j][0],w));a-=w;b-=w
        if a<1e-10:
            i+=1
            if i<len(left):a=left[i][1]
        if b<1e-10:
            j+=1
            if j<len(right):b=right[j][1]
    for col,policy in enumerate((cur,old)):
        for t in policy:
            assert abs(sum(s[2] for s in strata if s[col]==t["id"])-t["weight"])<1e-10
    counts=[1]*len(strata)
    for _ in range(blocks-len(strata)):
        k=max(range(len(strata)),key=lambda k:strata[k][2]**2/(counts[k]*(counts[k]+1)))
        counts[k]+=1
    rows=[];weights=[]
    for (new,former,w),n in zip(strata,counts,strict=True):
        for _ in range(n):
            k=len(weights);weights.append(w/n)
            for p,tid in [(CURRENT,new),(OLD,former)]:
                rows.extend(pair("coverage-"+stage,k,k,[name,tid],[name,p]))
    write(stage+"-weights.json",weights)
    save_games(HERE/(stage+"-schedule.jsonl"),rows)



def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def collect(stage,directory):
    schedule=HERE/(stage+"-schedule.jsonl")
    tasks=[json.loads(l) for l in schedule.read_text().splitlines()]
    manifest=json.loads((directory/"manifest.json").read_text())
    previous=load("sample17/manifest.json")
    assert manifest["wasm"]==previous["wasm"] and manifest["data"]==previous["data"]
    assert manifest["tasks"]==digest(schedule) and manifest["poolsSha256"]==digest(HERE/"pools.json")
    rows=[]
    for i,task in enumerate(tasks):
        r=json.loads((directory/f"{i}.json").read_text())
        assert r.pop("index")==i and all(r[k]==v for k,v in task.items())
        log=json.loads((directory/"logs"/f"{i}.json").read_text())
        side=f"p{r['rowSeat']+1}a: "
        moves=Counter()
        for line in log:
            fields=line.split("|")
            if len(fields)>=4 and fields[1]=="move" and fields[2].startswith(side):
                moves[fields[2][len(side):]+" / "+fields[3]]+=1
        assert moves
        r["challengerMoveCounts"]=dict(moves)
        rows.append(r)
    out=HERE/(stage+"-games.jsonl")
    seeds={r["battleSeed"] for r in rows}
    for f in HERE.parent.rglob("*-games.jsonl"):
        if f != out:
            assert all(json.loads(l)["battleSeed"] not in seeds for l in f.read_text().splitlines())
    matches=paired(rows,read("pools.json"))
    save_games(out,rows)
    write(stage+"-manifest.json",dict(manifest,runner=digest(DATA.parent/"web/tests/nash-refresh.mjs"),
        records=digest(out),games=len(rows),battle_seeds=len(seeds),ties=sum(r["outcome"]=="tie" for r in rows),
        turn_caps=sum(r["outcome"]=="cap" for r in rows)))
    return matches


def move_summary(rows,name):
    cfg=read("pools.json")
    team=next(t for t in cfg["teams"] if t["id"]==name)["sets"]
    mon,move=SOURCES[name][1:]
    selected=Counter()
    use=Counter()
    for r in rows:
        pick=logical(r,"preview")[0]["input"].removeprefix("team ")
        selected.update(team[int(slot)-1]["species"] for slot in pick.split(","))
        use.update(r["challengerMoveCounts"].keys())
    return {"games":len(rows),"featured_user_selected":selected[mon],"featured_move_attempted":use[mon+" / "+move],
            "selection_counts":dict(selected.most_common()),"move_games":dict(use.most_common())}


def screen(directory):
    cfg=read("pools.json")
    matches=collect("screen",directory)
    results={}
    for name in SOURCES:
        scores={}
        for policy in (CURRENT,OLD):
            members={t["id"]:t["weight"] for t in pools(cfg)[policy]}
            blocks=[v for (a,b,_),v in matches.items() if (a,b)==(name,policy)]
            weights=[members[rows[0]["ids"][1]] for _,rows in blocks]
            scores[policy]=interval([v for v,_ in blocks],weights)["score"]
        scores["change"]=scores[CURRENT]-scores[OLD]
        scores["nomination_score"]=min(scores[CURRENT]-.5,scores["change"])
        results[name]=scores
    nominee=max(results,key=lambda n:(results[n]["nomination_score"],results[n][CURRENT]))
    write("screen-summary.json",{"challengers":results,"nominee":nominee})
    holdout_schedule(nominee,cfg)


def deterioration():
    results=read("screen-summary.json")["challengers"]
    name=max(results,key=lambda n:results[n]["change"])
    assert name!=read("screen-summary.json")["nominee"]
    write("secondary-nominee.json",{"nominee":name})
    holdout_schedule(name,read("pools.json"),stage="secondary",blocks=10)


def holdout(directory,stage="holdout"):
    summary="screen-summary.json" if stage=="holdout" else "secondary-nominee.json"
    name=read(summary)["nominee"]
    matches=collect(stage,directory)
    weights=read(stage+"-weights.json")
    result={}
    values={}
    for policy in (CURRENT,OLD):
        blocks=[matches[name,policy,b] for b in range(len(weights))]
        values[policy]=[v for v,_ in blocks]
        result[policy]=dict(**interval(values[policy],weights),
            usage=move_summary([r for _,pair_rows in blocks for r in pair_rows],name))
    strata=defaultdict(list)
    for b,w in enumerate(weights):
        v,rows=matches[name,CURRENT,b]
        _,old_rows=matches[name,OLD,b]
        for x,y in zip(rows,old_rows,strict=True):
            assert all(x[f]==y[f] for f in ("battleSeed","seeds","rowSeat"))
        family=next(s["family"] for s in load("conclusion-candidate.json")["shells"]
                    if any(m["id"]==rows[0]["ids"][1] for m in s["conditional"]))
        strata[family].append((v,w))
    result["by_current_shell"]={k:sum(v*w for v,w in vs)/sum(w for _,w in vs) for k,vs in strata.items()}
    result["change"]=interval([a-b for a,b in zip(values[CURRENT],values[OLD])],weights,span=2)
    result["nominee"]=name
    write(stage+"-summary.json",result)


if __name__=="__main__":
    if sys.argv[1:]==["design"]:
        design()
    elif sys.argv[1:]==["deterioration"]:
        deterioration()
    elif sys.argv[1]=="secondary":
        holdout(Path(sys.argv[2]),"secondary")
    else:
        {"screen":screen,"holdout":holdout}[sys.argv[1]](Path(sys.argv[2]))
