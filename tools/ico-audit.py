#!/usr/bin/env python3
"""Reproducible ICO vs shipped 81k first-decision census (no production edits)."""
import argparse
import concurrent.futures
import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INPUTS = ["data/search-profiles.json", "data/live-search.json",
          "data/team-pool-v2/team-pool.json", "data/belief-pool-v3/belief-pool.json",
          "data/kodoku-r3/conclusion-equilibrium-deme.json", "data/compiled-preview-v1.json",
          "data/gen2stadium2.json", "crates/bot/examples/ico_export.rs",
          "tpu/src/gen2_tpu/ico.py"]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--opponents", nargs="+")
    p.add_argument("--horizon", type=int, default=1)
    p.add_argument("--samples", type=int, default=32)
    p.add_argument("--picks", type=int, default=2)
    p.add_argument("--seed", type=int, default=20261103)
    p.add_argument("--threads", type=int, default=4)
    p.add_argument("--max-nodes", type=int, default=5000000)
    p.add_argument("--solve", action="store_true")
    p.add_argument("--iterations", type=int, default=4096)
    args = p.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    profile = json.loads((ROOT/"data/live-search.json").read_text())
    assert profile["battle"]["strong"] == 81000
    catalog = json.loads((ROOT/"data/team-pool-v2/team-pool.json").read_text())["teams"]
    if args.opponents:
        ids = args.opponents
    else:
        ids, seen = [], set()
        for t in catalog:
            key = tuple(sorted((s["species"], s["level"], s.get("gender", ""), bool(s.get("item"))) for s in t["sets"]))
            if key not in seen:
                ids.append(t["id"])
                seen.add(key)
    hashes = {f: hashlib.sha256((ROOT/f).read_bytes()).hexdigest() for f in INPUTS}
    manifest = dict(format="nc2000-ico-census-v1", arguments={k:str(v) if isinstance(v,Path) else v for k,v in vars(args).items()},
                    opponents=ids, input_sha256=hashes,
                    git_head=subprocess.check_output(["git","rev-parse","HEAD"], cwd=ROOT,text=True).strip())
    mp=args.out/"manifest.json"
    if mp.exists() and json.loads(mp.read_text()) != manifest:
        raise SystemExit("Refusing to mix conditions: existing manifest differs")
    mp.write_text(json.dumps(manifest,indent=2)+"\n")

    def one(foe):
        target=args.out/f"{foe}.game.json"
        started=time.monotonic()
        if not target.exists():
            temporary=target.with_suffix(".tmp")
            command=[str(ROOT/"target/release/examples/ico_export"),"--opponent",foe,
                     "--out",str(temporary),"--horizon",str(args.horizon),"--samples",str(args.samples),
                     "--picks",str(args.picks),"--seed",str(args.seed),"--bot-iters","81000",
                     "--max-nodes",str(args.max_nodes)]
            with (args.out/f"{foe}.log").open("w") as log:
                subprocess.run(command,cwd=ROOT,stdout=log,stderr=log,check=True)
            g=json.loads(temporary.read_text())
            assert g["metadata"]["baseline_iterations"]==81000
            temporary.rename(target)
        else:
            g=json.loads(target.read_text())
        m=g["metadata"]
        summary=dict(opponent=foe,nodes=m["nodes"],types=len(m["opponent_types"]),
                     worlds=len(m["worlds"]),baseline_calls=m["baseline_calls"],
                     native_hidden_disagreements=m["baseline_hidden_world_disagreements"],
                     seconds=time.monotonic()-started,game_sha256=hashlib.sha256(target.read_bytes()).hexdigest())
        if args.solve:
            subprocess.run(["uv","run","--project",str(ROOT/"tpu"),"python","-m","gen2_tpu.ico",
                            str(target),"--out",str(args.out/f"{foe}.result.json"),"--iterations",str(args.iterations)],
                           cwd=ROOT,check=True,stdout=subprocess.DEVNULL)
        print(json.dumps(summary),flush=True)
        return summary
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.threads) as pool:
        results=list(pool.map(one,ids))
    (args.out/"export-summary.json").write_text(json.dumps(results,indent=2)+"\n")


if __name__=="__main__":
    main()
