#!/usr/bin/env python3
import argparse
import json
import hashlib
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("games", type=Path)
    parser.add_argument("--a-worker", required=True)
    parser.add_argument("--b-worker", required=True)
    parser.add_argument("--a-args", type=json.loads, default=[])
    parser.add_argument("--b-args", type=json.loads, default=[])
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--preview-only", action="store_true")
    parser.add_argument("--allow-differences", action="store_true")
    args = parser.parse_args()
    groups = {key: {"decisions": 0, "different": 0, "a_ns": 0, "b_ns": 0} for key in ["preview", "battle", "forced"]}
    with args.games.open() as file:
        manifest = json.loads(next(file))
        config = manifest["config"]
        pool = json.loads(Path(config["pool"]).read_text())["teams"]
        dex = Path(config.get("dex", Path(__file__).resolve().parent.parent / "data/gen2stadium2.json"))
        if "sha256:" + hashlib.sha256(dex.read_bytes()).hexdigest() != manifest["hashes"]["dex"]:
            raise ValueError("dex hash changed")
        common = ["--pool", config["pool"], "--dex", str(dex), "--features"]
        count = 0
        policy_comparisons = 0
        for line in file:
            game = json.loads(line)
            workers = [[subprocess.Popen([worker] + common + flags, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
                        for _ in range(2)] for worker, flags in [(args.a_worker, args.a_args), (args.b_worker, args.b_args)]]
            def call(worker, message):
                worker.stdin.write(json.dumps(message) + "\n")
                worker.stdin.flush()
                line = worker.stdout.readline()
                if not line:
                    raise RuntimeError(f"worker exited: {worker.poll()}")
                return json.loads(line)
            try:
                for arm in workers:
                    for side, worker in enumerate(arm):
                        call(worker, {"op": "new", "side": side, "team": pool[game["team_ids"][side]]["sets"],
                                      "seed": game["agent_seeds"][side ^ game["swap"]]})
                for entry in game["frames"]:
                    preview = entry["frame"]["request"].get("teamPreview", False)
                    if args.preview_only and not preview:
                        break
                    responses = [None, None]
                    for arm in [count % 2, 1 - count % 2]:
                        responses[arm] = call(workers[arm][entry["side"]], {"op": "choose", "frame": entry["frame"]})
                    keys = ["action", "iterations", "observation", "legality_drift", "projections"]
                    if all("root_policy" in response for response in responses):
                        keys.append("root_policy")
                        policy_comparisons += 1
                    same = all(responses[0][key] == responses[1][key] for key in keys)
                    if not same and not args.allow_differences:
                        raise AssertionError(f"worker parity failed at game {game['game']} frame {count}")
                    key = "preview" if preview else "forced" if max(r["iterations"] for r in responses) == 0 else "battle"
                    group = groups[key]
                    group["decisions"] += 1
                    group["different"] += int(not same)
                    group["a_ns"] += responses[0]["elapsed_ns"]
                    group["b_ns"] += responses[1]["elapsed_ns"]
                    count += 1
                    if count >= args.limit:
                        break
            finally:
                for arm in workers:
                    for worker in arm:
                        worker.terminate()
                        worker.wait(timeout=10)
            if count >= args.limit:
                break
    if count == 0:
        raise ValueError("no decisions to compare")
    for group in groups.values():
        group["a_over_b"] = group["a_ns"] / group["b_ns"] if group["b_ns"] else None
    print(json.dumps({"decisions": count, "root_policy_comparisons": policy_comparisons, "groups": groups}, indent=2))


if __name__ == "__main__":
    main()
