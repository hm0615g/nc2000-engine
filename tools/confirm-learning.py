#!/usr/bin/env python3
import argparse
import datetime
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess


def digest(path):
    return "sha256:" + hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def write_new(path, value):
    with Path(path).open("x") as file:
        file.write(json.dumps(value, indent=2, allow_nan=False) + "\n")


def evaluator(path):
    spec = importlib.util.spec_from_file_location("confirmation_evaluator", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def manifest(directory):
    directory = Path(directory).resolve()
    config = read(directory / "config.json")
    arena = read(directory / "launcher.json")["arena"]
    hashes = {key: digest(config[key]) for key in ["pool", "dex"]}
    hashes["arena"] = digest(arena)
    hashes["agents"] = [
        {"program": digest(agent["program"]),
         "artifacts": [{"path": path, "hash": digest(path)} for path in agent["artifacts"]]}
        for agent in config["agents"]
    ]
    return {"type": "manifest", "config": config, "hashes": hashes}


def freeze(path, directories):
    path = Path(path).resolve()
    if path.exists():
        raise ValueError("confirmation already registered")
    source = Path(__file__).with_name("evaluate-learning.py")
    module = evaluator(source)
    blocks, seeds = [], set()
    identity = None
    total = 0
    for directory in directories:
        directory = Path(directory).resolve()
        if (directory / "games.jsonl").exists():
            raise ValueError("confirmation must be registered before any games start")
        row = manifest(directory)
        config = row["config"]
        if config["schema"] != "nc2000-learning-arena-v1" or config.get("crn_agent_seeds", False):
            raise ValueError("confirmation requires paired games with independent agent seeds")
        if config["games"] <= 0 or config["games"] % 2 or config["seed"] in seeds:
            raise ValueError("invalid block size or repeated seed")
        seeds.add(config["seed"])
        current = module.run_identity(row)
        if identity is not None and current != identity:
            raise ValueError("confirmation candidates, data, and arena must remain fixed")
        identity = current
        total += config["games"]
        blocks.append({"directory": str(directory), "checkpoint_games": total,
                       "config_hash": digest(directory / "config.json"),
                       "launcher_hash": digest(directory / "launcher.json"), "manifest": row})
    if not blocks:
        raise ValueError("no confirmation blocks")
    frozen_evaluator = path.with_name(path.stem + "-evaluator.py")
    with frozen_evaluator.open("xb") as file:
        file.write(source.read_bytes())
    frozen_evaluator.chmod(0o444)
    write_new(path, {"schema": "nc2000-learning-confirmation-v1",
                     "registered_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                     "evaluator": str(frozen_evaluator), "evaluator_hash": digest(frozen_evaluator),
                     "controller_hash": digest(__file__), "identity": identity,
                     "sample_cap_games": total, "blocks": blocks})
    path.chmod(0o444)


def verify(path):
    gate = read(path)
    if gate["schema"] != "nc2000-learning-confirmation-v1":
        raise ValueError("invalid confirmation schema")
    if digest(gate["evaluator"]) != gate["evaluator_hash"] or digest(__file__) != gate["controller_hash"]:
        raise ValueError("confirmation evaluator or controller changed")
    for block in gate["blocks"]:
        directory = Path(block["directory"])
        if (digest(directory / "config.json") != block["config_hash"]
                or digest(directory / "launcher.json") != block["launcher_hash"]
                or manifest(directory) != block["manifest"]):
            raise ValueError("registered confirmation configuration or artifact changed")
    return gate


def run(path):
    gate = verify(path)
    module = evaluator(gate["evaluator"])
    completed = []
    for block in gate["blocks"]:
        verify(path)
        directory = Path(block["directory"])
        games = directory / "games.jsonl"
        arena = read(directory / "launcher.json")["arena"]
        command = [arena, str(directory / "config.json"), str(games)]
        if games.exists():
            command.append("--resume")
        subprocess.run(command, check=True)
        with games.open() as file:
            actual = json.loads(next(file))
        if actual["config"] != block["manifest"]["config"] or actual["hashes"] != block["manifest"]["hashes"]:
            raise ValueError("completed run does not match registration")
        completed.append(games)
        result = module.combine(completed)
        if result["games"] != block["checkpoint_games"] or result["identity"] != gate["identity"]:
            raise ValueError("checkpoint does not match registration")
        result["registration_hash"] = digest(path)
        result["strength_test_passed"] = result["positive_strength_evidence"]
        checkpoint = Path(path).with_name(Path(path).stem + f"-{result['games']}.json")
        if checkpoint.exists():
            if read(checkpoint) != result:
                raise ValueError("completed checkpoint changed")
        else:
            write_new(checkpoint, result)
        print(json.dumps({key: result[key] for key in ["games", "wins", "losses", "ties", "score", "betting95", "strength_test_passed"]}), flush=True)
        if result["strength_test_passed"]:
            return


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("operation", choices=["freeze", "run", "verify"])
    parser.add_argument("registration", type=Path)
    parser.add_argument("blocks", nargs="*", type=Path)
    args = parser.parse_args()
    if args.operation == "freeze":
        freeze(args.registration, args.blocks)
    elif args.blocks:
        parser.error("block directories are only accepted by freeze")
    elif args.operation == "run":
        run(args.registration)
    else:
        verify(args.registration)


if __name__ == "__main__":
    main()
