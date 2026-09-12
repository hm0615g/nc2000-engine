#!/usr/bin/env python3
import argparse
import hashlib
import json
import math
from pathlib import Path

import torch
from torch import nn
from torch.nn import functional as F


class PolicyValue(nn.Module):
    def __init__(self, vocabulary):
        super().__init__()
        self.vocabulary = vocabulary
        self.species = nn.Embedding(len(vocabulary["species"]), 16, padding_idx=0)
        self.items = nn.Embedding(len(vocabulary["items"]), 8, padding_idx=0)
        self.moves = nn.Embedding(len(vocabulary["moves"]), 8, padding_idx=0)
        self.mon = nn.Linear(104, 32)
        self.context = nn.Linear(408, 128)
        self.action = nn.Linear(153, 64)
        self.policy = nn.Linear(64, 1)
        self.value = nn.Linear(128, 1)
        self.value_baseline = None

    def forward(self, batch):
        mons = torch.cat([
            batch["mons"], self.species(batch["species"]),
            self.items(batch["items"]), self.moves(batch["moves"]).mean(dim=2),
        ], dim=-1)
        state = torch.cat([F.relu(self.mon(mons)).flatten(1), batch["global"]], dim=-1)
        context = F.relu(self.context(state))
        actions = torch.cat([
            context[:, None].expand(-1, batch["actions"].shape[1], -1),
            batch["actions"], self.moves(batch["action_moves"]),
        ], dim=-1)
        logits = self.policy(F.relu(self.action(actions))).squeeze(-1)
        logits = logits.masked_fill(~batch["mask"], -1e9)
        value = self.value(context).squeeze(-1)
        if self.value_baseline == "material":
            value = material_logit(batch) + 0.5 * value.tanh()
        return logits, value

    def export(self, training):
        def matrix(tensor):
            return {"shape": list(tensor.shape), "data": tensor.detach().flatten().tolist()}

        result = {
            "schema": "nc2000-policy-value-v2" if self.value_baseline else "nc2000-policy-value-v1",
            "observation_schema": "nc2000-observation-v1",
            "vocabulary": self.vocabulary,
            "training": training,
        }
        if self.value_baseline:
            result["value_baseline"] = self.value_baseline
        for name in ["species", "items", "moves"]:
            result[name] = matrix(getattr(self, name).weight)
        for name in ["mon", "context", "action", "policy", "value"]:
            layer = getattr(self, name)
            result[name] = {"weight": matrix(layer.weight), "bias": layer.bias.detach().tolist()}
        return result

    def load_export(self, data):
        baseline = data.get("value_baseline")
        expected = "nc2000-policy-value-v2" if baseline else "nc2000-policy-value-v1"
        if baseline not in [None, "material"] or data["schema"] != expected or data["vocabulary"] != self.vocabulary:
            raise ValueError("model schema/vocabulary mismatch")
        self.value_baseline = baseline
        state = {}
        for name in ["species", "items", "moves"]:
            m = data[name]
            state[name + ".weight"] = torch.tensor(m["data"]).reshape(m["shape"])
        for name in ["mon", "context", "action", "policy", "value"]:
            m = data[name]["weight"]
            state[name + ".weight"] = torch.tensor(m["data"]).reshape(m["shape"])
            state[name + ".bias"] = torch.tensor(data[name]["bias"])
        self.load_state_dict(state)


def material_logit(batch):
    f = batch["mons"]
    deficit = ((1 - f[..., 1]) * f[..., 4] * (1 - f[..., 7])).reshape(-1, 2, 6).sum(-1)
    health = batch["global"][:, 4:6] * 6 - deficit
    return 2 * (health[:, 0] - health[:, 1])


def collate(rows):
    observations = [row["observation"] for row in rows]
    n = len(rows)
    width = max(len(o["actions"]) for o in observations)
    result = {
        "global": torch.tensor([o["global"] for o in observations], dtype=torch.float32),
        "mons": torch.tensor([[m["features"] for m in o["mons"]] for o in observations], dtype=torch.float32),
        "species": torch.tensor([[m["species"] for m in o["mons"]] for o in observations]),
        "items": torch.tensor([[m["item"] for m in o["mons"]] for o in observations]),
        "moves": torch.tensor([[m["moves"] for m in o["mons"]] for o in observations]),
        "actions": torch.zeros(n, width, 17),
        "action_moves": torch.zeros(n, width, dtype=torch.long),
        "mask": torch.zeros(n, width, dtype=torch.bool),
        "target": torch.tensor([r["target"] for r in rows]),
        "reward": torch.tensor([r["reward"] for r in rows], dtype=torch.float32),
    }
    for i, o in enumerate(observations):
        k = len(o["actions"])
        result["actions"][i, :k] = torch.tensor([a["features"] for a in o["actions"]])
        result["action_moves"][i, :k] = torch.tensor([a["move_id"] for a in o["actions"]])
        result["mask"][i, :k] = torch.tensor([a["eligible"] for a in o["actions"]])
    if not result["mask"].gather(1, result["target"][:, None]).all():
        raise ValueError("teacher selected a masked action")
    return result


def read_games(paths, validation_fraction, split_seed):
    train, validation, sources = [], [], []
    seen = set()
    for path in paths:
        raw = Path(path).read_bytes()
        digest = hashlib.sha256(raw).hexdigest()
        sources.append({"path": str(path), "sha256": digest})
        lines = raw.splitlines()
        manifest = json.loads(lines[0])
        if manifest["type"] != "manifest" or not manifest["config"]["record"]:
            raise ValueError("training requires recorded learning_arena games")
        stream = hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest()
        count = 0
        for line in lines[1:]:
            game = json.loads(line)
            key = (stream, game["game"])
            if key in seen:
                raise ValueError("duplicate training game")
            seen.add(key)
            count += 1
            if game["type"] != "game" or game["capped"] or game["outcome"] not in ["p1", "p2", "tie"]:
                raise ValueError("training requires completed, uncapped games")
            pairing = f'{manifest["hashes"]["pool"]}:{game["battle_seed"]}:{game["team_ids"]}'
            split = int.from_bytes(hashlib.sha256(f'{split_seed}:{pairing}'.encode()).digest()[:8], "big") / 2**64
            destination = validation if split < validation_fraction else train
            for frame in game["frames"]:
                observation = frame["response"].get("observation")
                if observation is None or observation["schema"] != "nc2000-observation-v1":
                    raise ValueError("worker must emit --features")
                actions = [a["input"] for a in observation["actions"]]
                target = actions.index(frame["response"]["action"])
                reward = 0.5 if game["outcome"] == "tie" else float(game["outcome"] == f'p{frame["side"] + 1}')
                destination.append({"observation": observation, "target": target, "reward": reward})
        if count != manifest["config"]["games"]:
            raise ValueError("training requires complete runs; early finishers bias the position distribution")
    return train, validation, sources


def evaluate(model, rows, batch_size):
    if not rows:
        return None
    total_loss = total_brier = material_brier = correct = count = choice_count = choice_correct = 0
    model.eval()
    with torch.no_grad():
        for start in range(0, len(rows), batch_size):
            batch = collate(rows[start:start + batch_size])
            logits, value = model(batch)
            count += len(batch["target"])
            total_loss += F.cross_entropy(logits, batch["target"], reduction="sum").item()
            total_brier += ((value.sigmoid() - batch["reward"])**2).sum().item()
            material_brier += ((material_logit(batch).sigmoid() - batch["reward"])**2).sum().item()
            correct += (logits.argmax(-1) == batch["target"]).sum().item()
            choosing = batch["mask"].sum(-1) > 1
            choice_count += choosing.sum().item()
            choice_correct += ((logits.argmax(-1) == batch["target"]) & choosing).sum().item()
    return {"rows": count, "policy_loss": total_loss/count, "accuracy": correct/count, "brier": total_brier/count,
            "choice_rows": choice_count, "choice_accuracy": choice_correct/choice_count if choice_count else None,
            "material_brier": material_brier/count}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("games", nargs="*")
    parser.add_argument("--vocabulary", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--init")
    parser.add_argument("--initial-only", action="store_true")
    parser.add_argument("--epochs", type=int, default=20)
    parser.add_argument("--batch-size", type=int, default=128)
    parser.add_argument("--lr", type=float, default=0.001)
    parser.add_argument("--seed", type=int, default=19)
    parser.add_argument("--validation-fraction", type=float, default=0.2)
    parser.add_argument("--threads", type=int, default=2)
    parser.add_argument("--material-value", action="store_true")
    parser.add_argument("--select", choices=["policy_loss", "brier"], default="policy_loss")
    parser.add_argument("--resume", action="store_true")
    args = parser.parse_args()
    out = Path(args.out)
    if out.exists():
        parser.error(f"output already exists: {out}")
    if args.epochs <= 0 or args.batch_size <= 0 or args.threads <= 0:
        parser.error("epochs, batch size and threads must be positive")
    torch.set_num_threads(args.threads)
    torch.manual_seed(args.seed)
    vocabulary = json.loads(Path(args.vocabulary).read_text())
    model = PolicyValue(vocabulary)
    if args.init:
        model.load_export(json.loads(Path(args.init).read_text()))
    if args.material_value and model.value_baseline is None:
        model.value_baseline = "material"
        nn.init.zeros_(model.value.weight)
        nn.init.zeros_(model.value.bias)
    training = {"seed": args.seed, "objective": "teacher_action_and_terminal_outcome", "torch_version": str(torch.__version__)}
    if not args.initial_only:
        if not args.games or not 0 < args.validation_fraction < 1:
            parser.error("training needs games and a validation fraction in (0, 1)")
        train, validation, sources = read_games(args.games, args.validation_fraction, args.seed)
        if not train or not validation:
            parser.error("both training and validation must contain complete game pairs")
        optimizer = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.0001)
        training.update({"sources": sources, "training_rows": len(train), "validation_rows": len(validation)})
        checkpoint = out.with_suffix(".checkpoints")
        identity = {
            "sources": sources, "vocabulary_sha256": hashlib.sha256(Path(args.vocabulary).read_bytes()).hexdigest(),
            "trainer_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "initial_sha256": hashlib.sha256(Path(args.init).read_bytes()).hexdigest() if args.init else None,
            "seed": args.seed, "epochs": args.epochs, "batch_size": args.batch_size,
            "lr": args.lr, "threads": args.threads, "select": args.select,
            "validation_fraction": args.validation_fraction, "material_value": args.material_value,
            "torch_version": str(torch.__version__),
        }
        best_loss = math.inf
        best = None
        first_epoch = 0
        if args.resume:
            state = torch.load(checkpoint/"latest.pt", weights_only=True)
            if state["identity"] != identity:
                raise ValueError("resume training inputs/configuration differ")
            model.load_state_dict(state["model"])
            optimizer.load_state_dict(state["optimizer"])
            torch.set_rng_state(state["rng"])
            first_epoch = state["epoch"]
            best_loss, best = state["best_loss"], state["best"]
        else:
            checkpoint.mkdir(parents=True, exist_ok=False)
        for epoch in range(first_epoch, args.epochs):
            model.train()
            permutation = torch.randperm(len(train)).tolist()
            for start in range(0, len(train), args.batch_size):
                batch = collate([train[i] for i in permutation[start:start + args.batch_size]])
                logits, value = model(batch)
                loss = F.cross_entropy(logits, batch["target"]) + 0.5 * F.binary_cross_entropy_with_logits(value, batch["reward"])
                optimizer.zero_grad()
                loss.backward()
                nn.utils.clip_grad_norm_(model.parameters(), 1.0)
                optimizer.step()
            metrics = evaluate(model, validation, args.batch_size)
            print(json.dumps({"epoch": epoch + 1, "validation": metrics}), flush=True)
            if metrics[args.select] < best_loss:
                best_loss = metrics[args.select]
                best = model.export({**training, "epoch": epoch + 1, "validation": metrics})
                (checkpoint/"best.tmp").write_text(json.dumps(best, separators=(",", ":"), allow_nan=False)+"\n")
                (checkpoint/"best.tmp").replace(checkpoint/"best.json")
            torch.save({
                "identity": identity, "epoch": epoch + 1, "model": model.state_dict(),
                "optimizer": optimizer.state_dict(), "rng": torch.get_rng_state(),
                "best_loss": best_loss, "best": best,
            }, checkpoint/"latest.tmp")
            (checkpoint/"latest.tmp").replace(checkpoint/"latest.pt")
        payload = best
    else:
        payload = model.export({**training, "untrained": True})
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("x") as file:
        json.dump(payload, file, separators=(",", ":"), allow_nan=False)
        file.write("\n")


if __name__ == "__main__":
    main()
