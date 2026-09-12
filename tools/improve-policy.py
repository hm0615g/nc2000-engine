#!/usr/bin/env python3
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path

import torch
from torch import nn
from torch.nn import functional as F


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("games", nargs="+")
    parser.add_argument("--model", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--agent", type=int, choices=[0, 1], default=0)
    parser.add_argument("--epochs", type=int, default=4)
    parser.add_argument("--batch-size", type=int, default=128)
    parser.add_argument("--lr", type=float, default=0.0001)
    parser.add_argument("--seed", type=int, default=47)
    parser.add_argument("--threads", type=int, default=2)
    parser.add_argument("--entropy", type=float, default=0.01)
    args = parser.parse_args()
    torch.set_num_threads(args.threads)
    torch.manual_seed(args.seed)
    spec = importlib.util.spec_from_file_location("train_policy", Path(__file__).with_name("train-policy.py"))
    training = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(training)
    raw_model = Path(args.model).read_bytes()
    parent_hash = hashlib.sha256(raw_model).hexdigest()
    payload = json.loads(raw_model)
    model = training.PolicyValue(payload["vocabulary"])
    model.load_export(payload)
    rows, sources = [], []
    seen = set()
    for path in args.games:
        raw = Path(path).read_bytes()
        sources.append({"path": path, "sha256": hashlib.sha256(raw).hexdigest()})
        lines = raw.splitlines()
        manifest = json.loads(lines[0])
        if manifest["type"] != "manifest" or not manifest["config"]["record"]:
            raise ValueError("PPO requires recorded learning_arena games")
        artifacts = manifest["hashes"]["agents"][args.agent]["artifacts"]
        if not any(a["hash"] == "sha256:" + parent_hash for a in artifacts):
            raise ValueError("trajectory policy is not the supplied parent model")
        stream = hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
        count = 0
        for line in lines[1:]:
            game = json.loads(line)
            key = (stream, game["game"])
            if key in seen or game["type"] != "game" or game["capped"]:
                raise ValueError("duplicate/invalid/capped PPO trajectory")
            seen.add(key)
            count += 1
            if game["outcome"] not in ["p1", "p2", "tie"]:
                raise ValueError("invalid terminal outcome")
            trajectory = [f for f in game["frames"] if f["agent"] == args.agent]
            for frame in trajectory:
                response = frame["response"]
                if response.get("temperature") != 1.0 or response.get("log_prob") is None:
                    raise ValueError("PPO requires sampling at temperature 1")
                observation = response["observation"]
                target = next(i for i, a in enumerate(observation["actions"]) if a["input"] == response["action"])
                reward = 0.5 if game["outcome"] == "tie" else float(game["outcome"] == f'p{frame["side"] + 1}')
                rows.append({
                    "observation": observation, "target": target, "reward": reward,
                    "old_log_prob": response["log_prob"], "advantage": reward - response["value"],
                })
        if count != manifest["config"]["games"]:
            raise ValueError("PPO requires a complete rollout batch")
    if len(rows) < 2:
        raise ValueError("insufficient PPO decisions")
    model.eval()
    with torch.no_grad():
        for start in range(0, len(rows), args.batch_size):
            selected = rows[start:start + args.batch_size]
            batch = training.collate(selected)
            logits, _ = model(batch)
            actual = F.log_softmax(logits, dim=-1).gather(1, batch["target"][:, None]).squeeze(-1)
            old = torch.tensor([r["old_log_prob"] for r in selected])
            if (actual - old).abs().max().item() > 0.0001:
                raise ValueError("behavior log probability differs from parent model")
    advantages = torch.tensor([r["advantage"] for r in rows])
    advantages = (advantages - advantages.mean()) / advantages.std().clamp_min(1e-8)
    for row, advantage in zip(rows, advantages.tolist()):
        row["advantage"] = advantage
    optimizer = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.0001)
    metrics = []
    model.train()
    for epoch in range(args.epochs):
        permutation = torch.randperm(len(rows)).tolist()
        total = 0.0
        for start in range(0, len(rows), args.batch_size):
            selected = [rows[i] for i in permutation[start:start + args.batch_size]]
            batch = training.collate(selected)
            logits, value = model(batch)
            dist = torch.distributions.Categorical(logits=logits)
            old = torch.tensor([r["old_log_prob"] for r in selected])
            advantage = torch.tensor([r["advantage"] for r in selected])
            ratio = (dist.log_prob(batch["target"]) - old).exp()
            policy_loss = -torch.minimum(ratio * advantage, ratio.clamp(0.8, 1.2) * advantage).mean()
            value_loss = F.mse_loss(value.sigmoid(), batch["reward"])
            loss = policy_loss + 0.5 * value_loss - args.entropy * dist.entropy().mean()
            optimizer.zero_grad()
            loss.backward()
            nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            optimizer.step()
            total += loss.item() * len(selected)
        if not math.isfinite(total):
            raise ValueError("non-finite PPO loss")
        metric = {"epoch": epoch + 1, "loss": total / len(rows)}
        metrics.append(metric)
        print(json.dumps(metric), flush=True)
    exported = model.export({
        "algorithm": "ppo", "reward": "terminal_win_1_tie_half_loss_0", "gamma": 1.0,
        "parent_sha256": parent_hash, "sources": sources, "decisions": len(rows),
        "seed": args.seed, "metrics": metrics,
    })
    with Path(args.out).open("x") as file:
        json.dump(exported, file, separators=(",", ":"), allow_nan=False)
        file.write("\n")


if __name__ == "__main__":
    main()
