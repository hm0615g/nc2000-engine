#!/usr/bin/env python3
"""
Clean, deterministic builder for data/pbs-opponent-db.json from the 711 unique
Gym Leader's Castle trajectories in data/pbs-castle-logs/logs/*.html.

Fixes all documented corruptions in the legacy DB (pbs-vs-ps-spec.md §7-B.5):
  1. Populates `maxHps` accurately across ALL pool (`shells`) and per-player
     (`players[name].shells`) mon records from observed exact HP denominators.
  2. Strictly excludes Metronome-called moves (including 2-turn & locked moves
     such as D_20260213 Snorlax `かまいたち`/`ソーラービーム` and H_20260423
     Snorlax `ふくろだたき`), Mirror Move / Transform / Mimic foreign moves,
     and Struggle from a mon's `moves` map.
  3. Excludes direct OHKO moves (`つのドリル`, `じわれ`, `ハサミギロチン`) and
     any `data/castle-validation.json`-illegal species/level moves (e.g.
     raw-CGI illegal `venusaur/sludgebomb`, `raticate/zapcannon`,
     `poliwhirl/dynamicpunch`) from pool move priors while recording audit counts.
  4. Remaps Castle ID 81 confusion-cure display disguise (`にがいきのみ`) -> `iceberry`,
     handles Thief item transfers cleanly, and populates `noLeftoversCount`.
  5. Deduplicates the 26 prefix-duplicate log pairs (737 -> 711 unique trajectories)
     and truncates the 6 post-WIN double-POST logs at the first GoldenRod WIN.
  6. Records unambiguous `playerWins` / `playerLosses` / `draws` AND properly
     oriented opponent-perspective `winsAgainst` (= `playerLosses`, games won
     against this player) and `lossesAgainst` (= `playerWins`, games lost
     against this player) for backwards-compatible schema consumers.

M2M Output Principle: emits ONE compact JSON summary line to stdout.
"""

import argparse
import importlib.util
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict


def load_replay_module(script_dir: Path):
    mod_path = script_dir / "pbs-shadow-replay.py"
    spec = importlib.util.spec_from_file_location("pbs_shadow_replay", mod_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {mod_path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def filename_to_iso(filename: str) -> str:
    m = re.match(r"^[A-Z]_(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})_", filename)
    if not m:
        return "1970-01-01T00:00:00.000Z"
    y, mo, d, hh, mm, ss = m.groups()
    return f"{y}-{mo}-{d}T{hh}:{mm}:{ss}.000Z"


def shell_key_for_side(mons) -> str:
    return "/".join(sorted(f"{m.species_id}:{m.level}" for m in mons))


def ensure_shell(container: Dict[str, Any], s_key: str, mons, iso_ts: str) -> Dict[str, Any]:
    entry = container.get(s_key)
    if entry is None:
        entry = {
            "games": 0,
            "lastSeen": iso_ts,
            "mons": {},
        }
        container[s_key] = entry
    entry["games"] += 1
    if iso_ts > entry["lastSeen"]:
        entry["lastSeen"] = iso_ts
    for m in mons:
        mk = f"{m.species_id}:{m.level}"
        if mk not in entry["mons"]:
            entry["mons"][mk] = {
                "species": m.species_name,
                "speciesId": m.species_id,
                "level": m.level,
                "gender": m.gender,
                "itemFlag": m.has_item,
                "pickedCount": 0,
                "maxHps": {},
                "items": {},
                "noLeftoversCount": 0,
                "moves": {},
            }
    return entry


def load_castle_validation(path: Path) -> Dict[str, Any]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    species_map: Dict[str, Dict[str, Any]] = {}
    for sid, entry in raw["species"].items():
        species_map[sid] = {
            "minLevel": int(entry.get("minLevel", 1)),
            "moves": set(entry.get("moves", [])),
            "moveMinLevel": {k: int(v) for k, v in entry.get("moveMinLevel", {}).items()},
        }
    return {
        "ohkoMoves": set(raw.get("ohkoMoves", [])),
        "species": species_map,
    }


def is_castle_move_legal(
    validation: Dict[str, Any], species_id: str, level: int, mv_id: str
) -> bool:
    base_mv = "hiddenpower" if mv_id.startswith("hiddenpower") else mv_id
    if base_mv in validation["ohkoMoves"]:
        return False
    sp = validation["species"].get(species_id)
    if sp is None:
        return False
    if base_mv not in sp["moves"]:
        return False
    if level < sp["moveMinLevel"].get(base_mv, 1):
        return False
    return True


def accumulate_mon(
    dst: Dict[str, Any],
    m,
    validation: Dict[str, Any],
) -> int:
    if m.picked:
        dst["pickedCount"] += 1
    if m.max_hp is not None and m.max_hp > 0:
        hp_k = str(m.max_hp)
        dst["maxHps"][hp_k] = dst["maxHps"].get(hp_k, 0) + 1
    if m.orig_item is not None:
        dst["items"][m.orig_item] = dst["items"].get(m.orig_item, 0) + 1
    if m.no_leftovers:
        dst["noLeftoversCount"] += 1
    # Only record own Castle-legal non-OHKO moves (Metronome/2-turn/locked/Mimic/
    # Transform/Mirror Move/OHKO/castle-validation-illegal moves excluded!).
    illegal_excluded = 0
    for mv_id in sorted(m.moves_own):
        if not is_castle_move_legal(validation, m.species_id, m.level, mv_id):
            illegal_excluded += 1
            continue
        dst["moves"][mv_id] = dst["moves"].get(mv_id, 0) + 1
    return illegal_excluded


def main() -> int:
    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parent
    replay = load_replay_module(script_dir)
    data_root = replay.resolve_data_root(repo_root)

    parser = argparse.ArgumentParser(description="Rebuild data/pbs-opponent-db.json cleanly from 711 unique Castle logs")
    parser.add_argument("--logs-dir", type=Path, default=data_root / "pbs-castle-logs" / "logs")
    parser.add_argument("--i18n", type=Path, default=repo_root / "data" / "i18n-ja.json")
    parser.add_argument("--dex", type=Path, default=repo_root / "data" / "gen2stadium2.json")
    parser.add_argument("--validation", type=Path, default=repo_root / "data" / "castle-validation.json")
    parser.add_argument("--out", type=Path, default=data_root / "pbs-opponent-db.json")
    args = parser.parse_args()

    vocab = replay.load_vocab(args.i18n, args.dex)
    validation = load_castle_validation(args.validation)
    all_files = sorted(args.logs_dir.glob("*.html"))
    unique_files, dropped = replay.deduplicate_logs(all_files)

    players_db: Dict[str, Any] = {}
    shells_db: Dict[str, Any] = {}
    metronome_excluded_total = 0
    ohko_moves_excluded_total = 0
    illegal_moves_excluded_total = 0
    over_4_moves_in_game = 0
    ohko_games = 0
    latest_ts = "1970-01-01T00:00:00.000Z"

    for f in unique_files:
        res = replay.parse_and_replay_log(f, vocab)
        iso_ts = filename_to_iso(f.name)
        if iso_ts > latest_ts:
            latest_ts = iso_ts
        metronome_excluded_total += res["metronome_calls_excluded"]
        if res["overlay"]["direct_ohko"]:
            ohko_games += 1
        for s_idx in (0, 1):
            p_name = res["players"][s_idx]
            mons = res["sides"][s_idx]
            for m in mons:
                ohko_moves_excluded_total += len(m.moves_excluded_ohko)
                if len(m.moves_own) > 4:
                    over_4_moves_in_game += 1
            s_key = shell_key_for_side(mons)

            # Player record:
            # - playerWins / playerLosses / draws: this player's own record.
            # - winsAgainst / lossesAgainst: opponent-perspective counts
            #   (winsAgainst == playerLosses; lossesAgainst == playerWins).
            p_rec = players_db.get(p_name)
            if p_rec is None:
                p_rec = {
                    "name": p_name,
                    "games": 0,
                    "playerWins": 0,
                    "playerLosses": 0,
                    "draws": 0,
                    "winsAgainst": 0,
                    "lossesAgainst": 0,
                    "shells": {},
                }
                players_db[p_name] = p_rec
            p_rec["games"] += 1
            if res["winner_side"] is not None:
                if res["winner_side"] == s_idx:
                    p_rec["playerWins"] += 1
                    p_rec["lossesAgainst"] += 1
                else:
                    p_rec["playerLosses"] += 1
                    p_rec["winsAgainst"] += 1
            elif res["outcome_kind"] == "draw":
                p_rec["draws"] += 1

            p_shell = ensure_shell(p_rec["shells"], s_key, mons, iso_ts)
            g_shell = ensure_shell(shells_db, s_key, mons, iso_ts)
            for m in mons:
                mk = f"{m.species_id}:{m.level}"
                accumulate_mon(p_shell["mons"][mk], m, validation)
                illegal_moves_excluded_total += accumulate_mon(g_shell["mons"][mk], m, validation)

    # Sort keys deterministically for clean diffs / reproducible builds
    sorted_players = {k: players_db[k] for k in sorted(players_db)}
    for p_rec in sorted_players.values():
        p_rec["shells"] = {sk: p_rec["shells"][sk] for sk in sorted(p_rec["shells"])}
    sorted_shells = {sk: shells_db[sk] for sk in sorted(shells_db)}

    out_obj = {
        "schema": "pbs-opponent-db-v1",
        "updatedAt": latest_ts,
        "players": sorted_players,
        "shells": sorted_shells,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(out_obj, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    # Compute verification telemetry
    s_mons_total = 0
    s_mons_picked = 0
    s_mons_nonempty_maxhp = 0
    p_mons_total = 0
    p_mons_picked = 0
    p_mons_nonempty_maxhp = 0
    snorlax_beatup_count = 0
    ohko_leak_count = 0
    illegal_move_leaks = 0
    ohko_ids = {"fissure", "horndrill", "guillotine"}

    for sv in sorted_shells.values():
        for mk, mv in sv["mons"].items():
            s_mons_total += 1
            if mv["pickedCount"] > 0:
                s_mons_picked += 1
            if mv["maxHps"]:
                s_mons_nonempty_maxhp += 1
            if mv["speciesId"] == "snorlax" and "beatup" in mv["moves"]:
                snorlax_beatup_count += 1
            if any(k in mv["moves"] for k in ohko_ids):
                ohko_leak_count += 1
            for mv_id in mv["moves"]:
                if not is_castle_move_legal(validation, mv["speciesId"], mv["level"], mv_id):
                    illegal_move_leaks += 1

    for pv in sorted_players.values():
        for sv in pv["shells"].values():
            for mv in sv["mons"].values():
                p_mons_total += 1
                if mv["pickedCount"] > 0:
                    p_mons_picked += 1
                if mv["maxHps"]:
                    p_mons_nonempty_maxhp += 1
                for mv_id in mv["moves"]:
                    if not is_castle_move_legal(validation, mv["speciesId"], mv["level"], mv_id):
                        illegal_move_leaks += 1

    summary = {
        "schema": "pbs-opponent-db-v1",
        "output": str(args.out),
        "unique_logs": len(unique_files),
        "prefix_duplicates_excluded": len(dropped),
        "players": len(sorted_players),
        "shells": len(sorted_shells),
        "shell_mons_total": s_mons_total,
        "shell_mons_picked": s_mons_picked,
        "shell_mons_with_maxHps": s_mons_nonempty_maxhp,
        "player_mons_total": p_mons_total,
        "player_mons_picked": p_mons_picked,
        "player_mons_with_maxHps": p_mons_nonempty_maxhp,
        "metronome_calls_excluded": metronome_excluded_total,
        "ohko_games_filtered": ohko_games,
        "ohko_moves_excluded": ohko_moves_excluded_total,
        "illegal_moves_excluded": illegal_moves_excluded_total,
        "over_4_moves_in_game": over_4_moves_in_game,
        "snorlax_beatup_leaks": snorlax_beatup_count,
        "ohko_move_leaks": ohko_leak_count,
        "illegal_move_leaks": illegal_move_leaks,
    }
    print(json.dumps(summary, ensure_ascii=False))
    return 0 if (
        snorlax_beatup_count == 0
        and ohko_leak_count == 0
        and illegal_move_leaks == 0
        and over_4_moves_in_game == 0
        and s_mons_nonempty_maxhp > 0
    ) else 1


if __name__ == "__main__":
    sys.exit(main())
