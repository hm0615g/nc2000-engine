#!/usr/bin/env python3
"""Run uploaded finite ICO games on a Colab TPU; checkpoint each result."""
import argparse
import hashlib
import json
import platform
import traceback
from pathlib import Path


def run_batch(game_dir, out_dir, iterations=4096):
    import jax
    import numpy as np
    from gen2_tpu import ico
    game_dir, out_dir = Path(game_dir), Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    runtime = dict(backend=jax.default_backend(), devices=[str(d) for d in jax.devices()],
                   python=platform.python_version(), jax=jax.__version__, numpy=np.__version__,
                   solver_sha256=hashlib.sha256(Path(ico.__file__).read_bytes()).hexdigest())
    assert runtime["backend"] == "tpu", "this worker requires the requested TPU; no silent CPU fallback"
    (out_dir/"runtime.json").write_text(json.dumps(runtime,indent=2)+"\n")
    try:
        with (out_dir/"progress.jsonl").open("a", buffering=1) as log:
            for src in sorted(game_dir.glob("*.game.json")):
                raw=src.read_bytes()
                digest=hashlib.sha256(raw).hexdigest()
                target=out_dir/src.name.replace(".game.json",".result.json")
                if target.exists():
                    existing=json.loads(target.read_text())
                    if existing["game_sha256"]!=digest or existing["runtime"]!=runtime or existing["unrestricted"]["iterations"]!=iterations:
                        raise ValueError(f"refuse condition mismatch at {target}")
                    continue
                log.write(json.dumps({"start":src.name})+"\n")
                result=ico.compare(json.loads(raw),iterations)
                result.update(game_sha256=digest,runtime=runtime)
                temporary=target.with_suffix(".tmp")
                temporary.write_text(json.dumps(result,indent=2)+"\n")
                temporary.rename(target)
                log.write(json.dumps({"done":src.name,"root_loss":result["estimated_root_loss_interval"],
                    "solver_gap":result["unrestricted"]["saddle_gap"],
                    "baseline_gap":result["baseline_root"]["saddle_gap"]})+"\n")
                jax.clear_caches()
        (out_dir/"complete.json").write_text(json.dumps({"complete":True,"games":len(list(out_dir.glob("*.result.json")))})+"\n")
    except BaseException:
        (out_dir/"error.txt").write_text(traceback.format_exc())
        raise


if __name__=="__main__":
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("game_dir",type=Path)
    p.add_argument("out_dir",type=Path)
    p.add_argument("--iterations",type=int,default=4096)
    a=p.parse_args()
    run_batch(a.game_dir,a.out_dir,a.iterations)
