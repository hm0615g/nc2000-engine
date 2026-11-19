"""Evaluate a reallocation from s0155 to Sample17 using exact disclosed pools."""
from collections import defaultdict
from hashlib import sha256
from math import isclose, log, sqrt
from pathlib import Path
from statistics import mean
import json
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
from analyze import fit, interval, load, paired, pools, records
from responses import draw
from screen import distribution, pair, save_games

S17 = "sample-17-orig"
DONOR = "red-v10-s1105-r2-s0155"
BASE, ENTRY = "base-seven", "enter-sample17"
GUARDS = {"sample07":4, "incumbent":2, DONOR:2, "D5":2}


def write(name, obj):
    (HERE / name).write_text(json.dumps(obj, indent=2) + "\n")


def source():
    candidate = load("conclusion-candidate.json")
    families = candidate["shells"]
    base = [{k:t[k] for k in ("id","weight")} for t in candidate["teams"]]
    cfg = load("screen-pools.json")
    donor = next(t["weight"] for t in base if t["id"] == DONOR)
    q = donor/2
    shared = [dict(t,weight=t["weight"]-(q if t["id"]==DONOR else 0)) for t in base]
    cfg["mixtures"] = {
        BASE:base, ENTRY:shared+distribution([S17],[q]), S17:distribution([S17]),
        DONOR:distribution([DONOR]), "sample07":families[0]["conditional"], "D5":cfg["mixtures"]["D5"],
    }
    return cfg, [dict(t,weight=t["weight"]/(1-q)) for t in shared], q


def historical_blocks():
    by_type = defaultdict(list)
    for fname, config, reverse in [("stratified","confirmation-pools",False),("s0155/holdout","s0155/pools",True)]:
        rs = paired(records(fname+"-games.jsonl"),load(config+".json"))
        for (a,b,_),(v,rows) in rs.items():
            expected = ("original-pool",S17) if reverse else (S17,"proposed")
            if (a,b)==expected:
                by_type[rows[0]["ids"][0 if reverse else 1]].append(1-v if reverse else v)
    return by_type


def estimate(by_type, members):
    values, weights = [], []
    for t in members:
        observations = by_type[t["id"]]
        values.extend(observations)
        weights.extend([t["weight"]/len(observations)]*len(observations))
    return interval(values,weights)


def historical():
    candidate = load("conclusion-candidate.json")
    by_type = historical_blocks()
    shells = candidate["shells"]
    shell_scores = [estimate(by_type,s["conditional"]) for s in shells]
    shell_game = load("fit-summary.json")["matrix"]
    table = [row[1:]+[1-s["score"]] for row,s in zip(shell_game[1:],shell_scores,strict=True)]
    table.append([s["score"] for s in shell_scores]+[.5])
    write("historical.json", {"by_shell":dict(zip([s["family"] for s in shells],shell_scores)),
         "type_blocks":{k:len(v) for k,v in by_type.items()},"overall":estimate(by_type,candidate["teams"]),
         "four_shell_matrix":table,"fitted":fit(table)})


def add(rows,b,guard,oid,old,new,weight,labels,kind):
    for policy,tid in [(BASE,old),(ENTRY,new)]:
        rows.extend(pair("sample17-entry",b,b,[oid,tid],[guard,policy]))
    labels.append({"block":b, "opponent":guard, "stratum":kind,"weight":weight})


def design():
    cfg, shared, q = source()
    historical()
    write("pools.json",cfg)
    rows, labels = [], []
    base = cfg["mixtures"][BASE]
    # The same preallocated draws test the baseline and alternative. The six donor
    # blocks split evenly between staying with the donor and moving to Sample17.
    counts = [8,3,3,2,1,1,6]
    assert base[-1]["id"]==DONOR and len(base)==len(counts)
    for t,n in zip(base,counts,strict=True):
        for i in range(n):
            old = t["id"]
            new = S17 if old==DONOR and i%2 else old
            add(rows,len(labels),S17,S17,old,new,t["weight"]/n,labels,"transfer" if new!=old else "shared")
    for guard,n in GUARDS.items():
        for kind, mass in [("shared",1-q),("transfer",q)]:
            for i in range(n):
                b=len(labels)
                oid=draw(pools(cfg)[guard],f"nc2000/sample17-entry/opponent/{b}")
                old=draw(shared,f"nc2000/sample17-entry/shared/{b}") if kind=="shared" else DONOR
                new=old if kind=="shared" else S17
                add(rows,b,guard,oid,old,new,mass/n,labels,kind)
    write("blocks.json",labels)
    save_games(HERE/"schedule.jsonl",rows)


def read(name):
    return json.loads((HERE / name).read_text())


def summaries(matches, labels):
    result = {}
    for guard in dict.fromkeys(row["opponent"] for row in labels):
        rows = [row for row in labels if row["opponent"]==guard]
        values = {BASE:[],ENTRY:[]}
        shared_games = same_actions = 0
        for label in rows:
            b = label["block"]
            old, revised = [matches[guard,p,b][1] for p in (BASE,ENTRY)]
            for a,r in zip(old,revised,strict=True):
                assert all(a[k]==r[k] for k in ("rowSeat","battleSeed","seeds"))
                side=a["rowSeat"]
                assert a["ids"][side]==r["ids"][side]
                if label["stratum"]=="shared":
                    assert a["ids"]==r["ids"]
                    shared_games += 1
                    same_actions += a["actionSha256"]==r["actionSha256"]
                else:
                    assert (a["ids"][1-side],r["ids"][1-side])==(DONOR,S17)
            for p in values:
                values[p].append(matches[guard,p,b][0])
        weights = [r["weight"] for r in rows]
        delta = [e-b for e,b in zip(values[ENTRY],values[BASE],strict=True)]
        result[guard] = {p:interval(v,weights) for p,v in values.items()}
        result[guard].update(delta=interval(delta,weights,span=2), shared_games=shared_games,
                             identical_shared_action_hashes=same_actions)
    return result


def followup(guards):
    cfg, shared, q = source()
    rows, labels = [], []
    # Fresh common draws, balanced across the four combinations of policy/stratum.
    for guard in guards:
        for kind, mass in [("shared",1-q),("transfer",q)]:
            for _ in range(6):
                b = len(labels)
                stem = f"nc2000/sample17-entry/holdout/{b}"
                oid = draw(pools(cfg)[guard],stem+"/opponent")
                old = draw(shared,stem+"/shared") if kind=="shared" else DONOR
                new = old if kind=="shared" else S17
                for p,tid in [(BASE,old),(ENTRY,new)]:
                    rows.extend(pair("sample17-entry-holdout",b,b,[oid,tid],[guard,p]))
                labels.append(dict(block=b,opponent=guard,stratum=kind,weight=mass/6))
    write("holdout-blocks.json",labels)
    save_games(HERE/"holdout-schedule.jsonl",rows)


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def collect(directory, holdout=False):
    prefix = "holdout-" if holdout else ""
    cfg = read("pools.json")
    manifest = json.loads((directory/"manifest.json").read_text())
    previous = load("s0155/manifest.json")
    schedule = HERE/(prefix+"schedule.jsonl")
    assert manifest["tasks"]==digest(schedule) and manifest["poolsSha256"]==digest(HERE/"pools.json")
    assert manifest["wasm"]==previous["wasm"] and manifest["data"]==previous["inputs"]
    assert digest(HERE.parents[2]/"web/tests/nash-refresh.mjs")==previous["runner"]
    games=[]
    for i,line in enumerate(schedule.read_text().splitlines()):
        task=json.loads(line)
        r=json.loads((directory/f"{i}.json").read_text())
        assert r.pop("index")==i and all(r[k]==v for k,v in task.items())
        games.append(r)
    seeds={r["battleSeed"] for r in games}
    for path in [*HERE.parent.glob("*-games.jsonl"),*(HERE.parent/"s0155").glob("*-games.jsonl")]:
        assert not seeds.intersection(json.loads(l)["battleSeed"] for l in path.read_text().splitlines())
    if holdout:
        assert not seeds.intersection(r["battleSeed"] for r in records("sample17/games.jsonl"))
    matches = paired(games,cfg)
    results = summaries(matches,read(prefix+"blocks.json"))
    out=HERE/(prefix+"games.jsonl")
    save_games(out,games)
    write(prefix+"manifest.json",dict(manifest,runner=previous["runner"],records=digest(out),games=len(games),
          distinct_seeds=len(seeds),ties=sum(r["outcome"]=="tie" for r in games),caps=sum(r["outcome"]=="cap" for r in games)))
    return matches,results


def evaluate(directory):
    matches,results = collect(directory)
    worst = {p:max(results,key=lambda g:results[g][p]["score"]) for p in (BASE,ENTRY)}
    peak = {p:results[g][p]["score"] for p,g in worst.items()}
    targeted = results[S17]["delta"]["score"]
    advance = targeted < -1e-12 and peak[ENTRY] < peak[BASE]-1e-12
    combined = historical_blocks()
    for (a,p,_),(score,rs) in matches.items():
        if a==S17 and p==BASE:
            combined[rs[0]["ids"][1]].append(score)
    candidate = load("conclusion-candidate.json")
    labels = read("blocks.json")
    weights = [l["weight"] for l in labels if l["opponent"]==S17]
    upper = min(1,results[S17][BASE]["score"]+sqrt(sum(w*w for w in weights)*log(20)/2))
    write("result.json", {"challengers":results,"worst":worst,"peak":peak,"advance":advance,
          "fresh_sample17_vs_original_one_sided_95_upper":upper,
          "archive_plus_new_descriptive":estimate(combined,candidate["teams"])})
    if advance:
        followup(list(dict.fromkeys([S17,worst[BASE],worst[ENTRY]])))


def confirm(directory):
    _,results = collect(directory,holdout=True)
    peak = {p:max(g[p]["score"] for g in results.values()) for p in (BASE,ENTRY)}
    promote = results[S17]["delta"]["score"]<0 and peak[ENTRY]<peak[BASE]
    write("holdout-result.json",{"challengers":results,"peak":peak,"promote":promote})
    if promote:
        candidate = load("conclusion-candidate.json")
        cfg,_,q = source()
        from analyze import catalog
        teams = catalog(cfg)
        candidate["teams"] = [m | {"sets":teams[m["id"]]} for m in cfg["mixtures"][ENTRY]]
        next(s for s in candidate["shells"] if s["family"]==DONOR)["weight"] -= q
        candidate["shells"].append(dict(family=S17,weight=q,conditional=distribution([S17])))
        candidate["profile"] = ENTRY
        assert isclose(sum(s["weight"] for s in candidate["shells"]),1)
        write("candidate.json",candidate)


if __name__=="__main__":
    if sys.argv[1:] == ["design"]:
        design()
    else:
        {"evaluate":evaluate,"confirm":confirm}[sys.argv[1]](Path(sys.argv[2]))
