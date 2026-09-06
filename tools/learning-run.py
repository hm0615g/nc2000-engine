#!/usr/bin/env python3
import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path


def main():
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--games", type=int, default=128)
    parser.add_argument("--seed", type=int, default=41017)
    parser.add_argument("--threads", type=int, default=8)
    parser.add_argument("--arena", type=Path, default=root/"target/release/examples/learning_arena")
    parser.add_argument("--worker", type=Path, default=root/"target/release/examples/blind_worker")
    parser.add_argument("--b-worker", type=Path)
    parser.add_argument("--pool", type=Path, default=root/"data/meta-pool-v0/meta-pool.json")
    parser.add_argument("--belief-pool", type=Path, default=root/"data/meta-pool-v0/meta-pool.json")
    parser.add_argument("--a-model", type=Path)
    parser.add_argument("--b-model", type=Path)
    parser.add_argument("--a-leaf-model", type=Path)
    parser.add_argument("--b-leaf-model", type=Path)
    parser.add_argument("--a-prune-root", action="store_true")
    parser.add_argument("--b-prune-root", action="store_true")
    parser.add_argument("--a-leaf-preview-iters", type=int, default=0)
    parser.add_argument("--b-leaf-preview-iters", type=int, default=0)
    parser.add_argument("--a-shared-iters", type=int)
    parser.add_argument("--b-shared-iters", type=int)
    parser.add_argument("--a-iters", type=int, default=30000)
    parser.add_argument("--b-iters", type=int, default=30000)
    parser.add_argument("--a-temperature", type=float, default=0.0)
    parser.add_argument("--b-temperature", type=float, default=0.0)
    parser.add_argument("--record", action="store_true")
    parser.add_argument("--crn-agent-seeds", action="store_true")
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--prepare-only", action="store_true")
    args = parser.parse_args()
    out = args.out.absolute()
    if args.resume:
        saved = json.loads((out/"launcher.json").read_text())
        command = [saved["arena"], str(out/"config.json"), str(out/"games.jsonl"), "--resume"]
    else:
        if args.games <= 0 or args.games % 2 or args.threads <= 0:
            parser.error("games must be positive/even and threads positive")
        if (args.a_model and args.a_leaf_model) or (args.b_model and args.b_leaf_model):
            parser.error("a worker can use either a direct policy or a leaf model")
        inputs = [args.arena, args.worker, args.b_worker or args.worker, args.pool, args.belief_pool,
                  root/"data/gen2stadium2.json"] + [p for p in [args.a_model, args.b_model, args.a_leaf_model, args.b_leaf_model] if p]
        for path in inputs:
            if not path.is_file():
                parser.error(f"missing input {path}; build the Rust examples first")
        out.mkdir(parents=True, exist_ok=False)
        artifacts = out/"artifacts"
        artifacts.mkdir()
        def freeze(source):
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            target = artifacts/f"{digest[:16]}-{source.name}"
            if not target.exists():
                shutil.copy2(source, target)
                target.chmod(0o555 if source.stat().st_mode & 0o111 else 0o444)
            return str(target)
        arena = freeze(args.arena)
        pool = freeze(args.pool)
        belief = freeze(args.belief_pool)
        dex = freeze(root/"data/gen2stadium2.json")
        agents = []
        for worker, model, leaf_model, preview_iters, shared_iters, prune, iters, temperature in [
            (args.worker, args.a_model, args.a_leaf_model, args.a_leaf_preview_iters, args.a_shared_iters, args.a_prune_root, args.a_iters, args.a_temperature),
            (args.b_worker or args.worker, args.b_model, args.b_leaf_model, args.b_leaf_preview_iters, args.b_shared_iters, args.b_prune_root, args.b_iters, args.b_temperature),
        ]:
            spec = {"program": freeze(worker), "args": ["--iters", str(iters), "--pool", belief, "--dex", dex], "artifacts": [belief, dex]}
            if model:
                frozen_model = freeze(model)
                spec["args"] += ["--model", frozen_model, "--temperature", str(temperature)]
                spec["artifacts"].append(frozen_model)
            if leaf_model:
                frozen_model = freeze(leaf_model)
                spec["args"] += ["--leaf-model", frozen_model]
                spec["artifacts"].append(frozen_model)
                if preview_iters:
                    spec["args"] += ["--leaf-preview-iters", str(preview_iters)]
            if prune:
                spec["args"].append("--prune-root")
            if shared_iters is not None:
                spec["args"] += ["--shared-search", "--shared-iters", str(shared_iters)]
            if args.record:
                spec["args"].append("--features")
            agents.append(spec)
        config = {
            "schema": "nc2000-learning-arena-v1", "seed": args.seed, "games": args.games,
            "threads": args.threads, "pool": pool, "dex": dex, "agents": agents,
            "record": args.record, "crn_agent_seeds": args.crn_agent_seeds,
        }
        (out/"config.json").write_text(json.dumps(config, indent=2)+"\n")
        (out/"launcher.json").write_text(json.dumps({"arena": arena}, indent=2)+"\n")
        command = [arena, str(out/"config.json"), str(out/"games.jsonl")]
    print(json.dumps({"command": command}), flush=True)
    if not args.prepare_only:
        subprocess.run(command, check=True)


if __name__ == "__main__":
    main()
