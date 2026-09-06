#!/usr/bin/env python3
import argparse
from collections import defaultdict
import hashlib
import importlib.util
import itertools
import json
import math
from pathlib import Path
import statistics


def load_module(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("games", type=Path)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--agent", type=int, choices=[0, 1], required=True)
    args = parser.parse_args()
    evaluation = load_module("evaluate_learning", "evaluate-learning.py")
    fitting = load_module("fit_pick_prior", "fit-pick-prior.py")
    evaluation.summarize(args.games)
    model = json.loads(args.model.read_text())
    if model["source"]["sha256"] == hashlib.sha256(args.games.read_bytes()).hexdigest():
        raise ValueError("calibration games must differ from training games")
    lookup = {}
    for row in model["rows"]:
        signature = tuple(sorted((m["species"], m["level"], m["gender"], m["item"]) for m in row["enemy_preview"]))
        lookup[(row["team"], signature, row["side"])] = row["choices"]
    pairs = defaultdict(list)
    predictions = []
    coverage = defaultdict(int)
    with args.games.open() as file:
        manifest = json.loads(next(file))
        pool = json.loads(Path(manifest["config"]["pool"]).read_text())["teams"]
        for line in file:
            game = json.loads(line)
            frames = [entry for entry in game["frames"] if entry["agent"] == args.agent
                      and entry["frame"]["request"].get("teamPreview")]
            if len(frames) != 1:
                raise ValueError("missing initial selection for target agent")
            entry = frames[0]
            side = entry["side"]
            team = pool[game["team_ids"][side]]
            roster = [fitting.toid(p["details"].split(",")[0]) for p in entry["frame"]["request"]["side"]["pokemon"]]
            key = (team["id"], fitting.preview(entry["frame"]["lines"], 1-side), side)
            specific = lookup.get(key)
            general = lookup.get((team["id"], (), side))
            coverage["specific" if specific else "general" if general else "uniform"] += 1
            if specific and general:
                count = sum(c["count"] for c in specific)
                weight = count / (count + model["backoff_strength"])
                sources = [(specific, weight), (general, 1-weight)]
            elif specific or general:
                sources = [(specific or general, 1.0)]
            else:
                sources = []
            combinations = list(itertools.combinations(sorted(roster), 3))
            smoothing = model["smoothing"] if sources else 1.0
            probabilities = {pick: smoothing/len(combinations) for pick in combinations}
            for choices, weight in sources:
                total = sum(c["count"] for c in choices)
                for choice in choices:
                    probabilities[tuple(sorted(choice["species"]))] += (1-smoothing)*weight*choice["count"]/total
            action = entry["response"]["action"]
            if action not in entry["frame"]["legal_actions"]:
                raise ValueError("illegal recorded selection")
            slots = [int(s.strip())-1 for s in action[5:].split(",")]
            lead = roster[slots[0]]
            truth = tuple(sorted(roster[slot] for slot in slots))
            support = {pick: p for pick, p in probabilities.items() if lead in pick}
            probability = support[truth] / sum(support.values())
            gain = math.log(probability*len(support))
            pairs[game["pair"]].append(gain)
            predictions.append({"game": game["game"], "p_true_bench": probability, "log_gain": gain})
    if any(len(values) != 2 for values in pairs.values()):
        raise ValueError("calibration requires paired games")
    gains = [statistics.mean(values) for values in pairs.values()]
    print(json.dumps({"games": len(predictions), "pairs": len(gains), "coverage": dict(coverage),
        "mean_log_gain_over_uniform_given_lead": statistics.mean(gains),
        "mean_true_bench_probability": statistics.mean(p["p_true_bench"] for p in predictions),
        "model_sha256": hashlib.sha256(args.model.read_bytes()).hexdigest(),
        "data_sha256": hashlib.sha256(args.games.read_bytes()).hexdigest(),
        "teacher": manifest["hashes"]["agents"][args.agent], "predictions": predictions}, indent=2))


if __name__ == "__main__":
    main()
