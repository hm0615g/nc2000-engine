#!/usr/bin/env python3
import argparse
import importlib.util
import json
import subprocess
from pathlib import Path

import torch


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--games", required=True)
    parser.add_argument("--limit", type=int, default=100)
    args = parser.parse_args()
    spec = importlib.util.spec_from_file_location("train_policy", Path(__file__).with_name("train-policy.py"))
    training = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(training)
    payload = json.loads(Path(args.model).read_text())
    model = training.PolicyValue(payload["vocabulary"])
    model.load_export(payload)
    model.eval()
    torch.set_num_threads(1)
    count = 0
    worst_logit = worst_value = 0.0
    with Path(args.games).open() as data:
        manifest = json.loads(next(data))
        pool = json.loads(Path(manifest["config"]["pool"]).read_text())["teams"]
        for line in data:
            game = json.loads(line)
            workers = [subprocess.Popen([args.worker, "--model", args.model, "--features"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) for _ in range(2)]
            def call(worker, message):
                worker.stdin.write(json.dumps(message) + "\n")
                worker.stdin.flush()
                reply = worker.stdout.readline()
                if not reply:
                    raise RuntimeError(f"worker exited: {worker.poll()}")
                return json.loads(reply)
            try:
                for side, worker in enumerate(workers):
                    agent = side ^ game["swap"]
                    call(worker, {"op": "new", "side": side, "team": pool[game["team_ids"][side]]["sets"], "seed": game["agent_seeds"][agent]})
                for frame in game["frames"]:
                    response = call(workers[frame["side"]], {"op": "choose", "frame": frame["frame"]})
                    observation = frame["response"]["observation"]
                    if observation != response["observation"]:
                        raise AssertionError("teacher and model encoded different observations")
                    target = next(i for i, a in enumerate(observation["actions"]) if a["eligible"])
                    batch = training.collate([{"observation": observation, "target": target, "reward": 0.5}])
                    with torch.no_grad():
                        logits, value = model(batch)
                    for i, action in enumerate(observation["actions"]):
                        if action["eligible"]:
                            worst_logit = max(worst_logit, abs(logits[0, i].item() - response["logits"][i]))
                    worst_value = max(worst_value, abs(value.sigmoid()[0].item() - response["value"]))
                    count += 1
                    if count >= args.limit:
                        break
            finally:
                for worker in workers:
                    worker.terminate()
                    worker.wait(timeout=10)
            if count >= args.limit:
                break
    if count == 0 or worst_logit > 0.00002 or worst_value > 0.00002:
        raise AssertionError((count, worst_logit, worst_value))
    print(json.dumps({"decisions": count, "max_logit_error": worst_logit, "max_value_error": worst_value}))


if __name__ == "__main__":
    main()
