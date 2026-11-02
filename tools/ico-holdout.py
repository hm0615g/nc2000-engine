#!/usr/bin/env python3
"""Evaluate learned ICO root policies on independent chance samples."""
import argparse
import copy
import hashlib
import json
from pathlib import Path


def assess(training, game, baseline_result, iterations=4096):
    from gen2_tpu.ico import solve
    trained={r["key"]:r for r in training["root_policies"]}
    g=copy.deepcopy(game)
    keys=set()
    for info in g["players"][0]["infosets"]:
        if "baseline" not in info:
            continue
        keys.add(info["key"])
        row=trained[info["key"]]  # fail closed on an unseen root observation
        if row["actions"] != info["actions"]:
            raise ValueError("root action mismatch")
        info["baseline"]=row["ico"]
    if keys != set(trained):
        raise ValueError("root information-set support differs")
    frozen=solve(g,iterations,freeze_baseline=True,checkpoints=[256,1024,iterations])
    base=baseline_result["baseline_root"]
    return dict(format="nc2000-ico-root-transfer-v1",
                training_game_sha256=training["game_sha256"],
                holdout_game_sha256=baseline_result["game_sha256"],
                training_seed=training["metadata"]["seed"],
                holdout_seed=game["metadata"]["seed"],
                frozen_training_root={k:v for k,v in frozen.items() if k!="policies"},
                baseline_root=base,
                heldout_root_gain_interval=[frozen["lower"]-base["upper"],frozen["upper"]-base["lower"]],
                full_game_certified=False,
                scope="training root policy frozen; continuation optimized on independent finite chance game; not a match win rate")


if __name__=="__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("training",type=Path)
    p.add_argument("game",type=Path)
    p.add_argument("baseline",type=Path)
    p.add_argument("--out",type=Path,required=True)
    p.add_argument("--iterations",type=int,default=4096)
    a=p.parse_args()
    training=json.loads(a.training.read_text())
    raw=a.game.read_bytes()
    baseline=json.loads(a.baseline.read_text())
    assert hashlib.sha256(raw).hexdigest()==baseline["game_sha256"]
    result=assess(training,json.loads(raw),baseline,a.iterations)
    a.out.write_text(json.dumps(result,indent=2)+"\n")
    print(json.dumps(result))
