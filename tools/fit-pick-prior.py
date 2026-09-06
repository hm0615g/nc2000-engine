#!/usr/bin/env python3
import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path


def toid(value):
    return "".join(c.lower() for c in value if c.isascii() and c.isalnum())


def preview(lines, side):
    mons = []
    for line in lines:
        fields = line.split("|")
        if len(fields) < 5 or fields[1:3] != ["poke", f"p{side+1}"]:
            continue
        details = [part.strip() for part in fields[3].split(",")]
        level = next((int(p[1:]) for p in details[1:] if p.startswith("L")), 100)
        gender = next((p for p in details[1:] if p in ("M", "F")), "")
        mons.append((toid(details[0]), level, gender, fields[4] == "item"))
    if len(mons) != 6 or len({m[0] for m in mons}) != 6:
        raise ValueError("preview must contain six distinct species")
    return tuple(sorted(mons))


def fit(path, smoothing):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    manifest, games = rows[0], rows[1:]
    config = manifest["config"]
    if manifest["type"] != "manifest" or config["schema"] not in ("nc2000-preview-collection-v1", "nc2000-learning-arena-v1"):
        raise ValueError("expected a preview collection or recorded games")
    preview_only = config["schema"] == "nc2000-preview-collection-v1"
    if config.get("crn_agent_seeds"):
        raise ValueError("collection needs independent agent seeds")
    ids = [row["game"] for row in games]
    if len(ids) != len(set(ids)) or set(ids) != set(range(config["games"])):
        raise ValueError("collection is incomplete or has duplicate games")
    hashes = manifest["hashes"]
    if hashes["agents"][0] != hashes["agents"][1] or config["agents"][0] != config["agents"][1]:
        raise ValueError("both seats must use the same frozen teacher")
    pool_path = Path(config["pool"])
    if "sha256:" + hashlib.sha256(pool_path.read_bytes()).hexdigest() != hashes["pool"]:
        raise ValueError("pool hash changed")
    pool = json.loads(pool_path.read_text())["teams"]
    counts = defaultdict(Counter)
    seeds = set()
    iterations = set()
    for row in games:
        if preview_only:
            if row["type"] != "preview" or len(row["frames"]) != 2 or "score" in row:
                raise ValueError("invalid preview record")
        elif row["type"] != "game" or row["capped"] or row["outcome"] not in ("p1", "p2", "tie"):
            raise ValueError("invalid or capped game")
        frames = [entry for entry in row["frames"] if entry["frame"]["request"].get("teamPreview")]
        if row["pair"] != row["game"] // 2 or row["swap"] != row["game"] % 2:
            raise ValueError("invalid preview pairing")
        if len(frames) != 2 or {frame["side"] for frame in frames} != {0, 1}:
            raise ValueError("missing preview seat")
        for entry in frames:
            side = entry["side"]
            if entry["agent"] != side ^ row["swap"]:
                raise ValueError("invalid teacher seat")
            seed = row["agent_seeds"][entry["agent"]]
            frame, response = entry["frame"], entry["response"]
            if entry["turn"] != 0 or not frame["request"].get("teamPreview"):
                raise ValueError("not an initial preview")
            if response["legality_drift"] or response["projections"]:
                raise ValueError("teacher legality error")
            iterations.add(response["iterations"])
            action = response["action"]
            if action not in frame["legal_actions"] or not action.startswith("team "):
                raise ValueError("illegal teacher selection")
            slots = [int(p.strip()) - 1 for p in action[5:].split(",")]
            if len(slots) != 3 or len(set(slots)) != 3:
                raise ValueError("teacher must select three distinct slots")
            own = pool[row["team_ids"][side]]
            roster = frame["request"]["side"]["pokemon"]
            picked = tuple(toid(roster[i]["details"].split(",")[0]) for i in slots)
            own_species = {toid(mon["species"]) for mon in own["sets"]}
            if own_species != {m[0] for m in preview(frame["lines"], side)}:
                raise ValueError("pool identity disagrees with public roster")
            key = (own["id"], preview(frame["lines"], 1-side), side)
            if (key, seed) in seeds:
                raise ValueError("duplicate query seed")
            seeds.add((key, seed))
            counts[key][picked] += 1
            counts[(own["id"], (), side)][picked] += 1
    if len(iterations) != 1 or next(iter(iterations)) <= 0:
        raise ValueError("teacher budget changed")
    data = []
    for (team, enemy, side), choices in sorted(counts.items()):
        data.append({
            "team": team, "enemy_preview": [dict(zip(("species", "level", "gender", "item"), m)) for m in enemy],
            "side": side, "choices": [{"species": list(pick), "count": count} for pick, count in sorted(choices.items())],
        })
    return {
        "schema": "nc2000-pick-prior-v1", "smoothing": smoothing, "backoff_strength": 8.0,
        "source": {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "hashes": hashes,
                   "games": len(games), "iterations": next(iter(iterations))},
        "rows": data,
    }


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--smoothing", type=float, default=.2)
    args = parser.parse_args()
    if not 0 < args.smoothing <= 1:
        parser.error("smoothing must be in (0, 1]")
    model = fit(args.input, args.smoothing)
    with args.out.open("x") as file:
        json.dump(model, file, separators=(",", ":"))
        file.write("\n")
    print(json.dumps({"rows": len(model["rows"]), "games": model["source"]["games"], "out": str(args.out)}))


if __name__ == "__main__":
    main()
