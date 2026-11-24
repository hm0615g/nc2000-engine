#!/usr/bin/env python3
"""Export deterministic Castle PBS validation dataset (`data/castle-validation.json`)
from PBSTools CSVs (`levelLowerLimit.csv`, `moveWhiteList.csv`, `moveBlackList.csv`)
and the authoritative Gym Leader's Castle `ev_rules.txt`.

Emits a single structured JSON telemetry line to stdout.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys


REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_PBSTOOLS_DATA = pathlib.Path(
    "/tmp/pbstools-ref/PBSTools/sub_tools/004_Validation_Check/Data"
)
DEFAULT_EV_RULES = pathlib.Path("/tmp/pbs-wiki/ev_rules.txt")
DEFAULT_OUT = REPO_ROOT / "data" / "castle-validation.json"

BANNED_SPECIES = ["celebi", "hooh", "lugia", "mew", "mewtwo"]
OHKO_MOVES = ["fissure", "guillotine", "horndrill"]
INERT_ITEMS = ["bitterberry", "dragonfang"]

# Upstream typo normalization in ev_rules.txt:210 / EffortLevelCheck.java:496
EV_RULES_MOVE_TYPO_FIXES = {
    "ふくろただき": "ふくろだたき",
}


def read_sjis_lines(path: pathlib.Path) -> list[str]:
    return [
        line.strip()
        for line in path.read_bytes().decode("cp932").splitlines()
        if line.strip()
    ]


def parse_ev_rules(
    ev_rules_path: pathlib.Path,
    ja_sp_to_id: dict[str, str],
    ja_mv_to_id: dict[str, str],
) -> tuple[list[str], dict[str, list[dict[str, object]]]]:
    lines = ev_rules_path.read_text(encoding="utf-8").splitlines()
    gen2_only_species: set[str] = set()
    gen2_req_moves: dict[str, dict[str, int | None]] = {}

    mode = None
    for raw in lines:
        line = raw.strip()
        if line == "無条件で第一世代で育成できない種族の一覧":
            mode = "species"
            continue
        if line == "特定の技を覚えさせたい場合に第一世代で育成できないポケモンの一覧":
            mode = "moves"
            continue
        if mode == "species":
            if not line.startswith("|"):
                mode = None
                continue
            name = line.lstrip("|").strip()
            if name == "種族名":
                continue
            if name not in ja_sp_to_id:
                raise ValueError(f"Unknown JP species in ev_rules.txt: {name}")
            gen2_only_species.add(ja_sp_to_id[name])
        elif mode == "moves":
            if not line.startswith("|"):
                mode = None
                continue
            parts = [p.strip() for p in line.lstrip("|").split("|")]
            if len(parts) < 2 or parts[0] == "種族名":
                continue
            sp_ja, mv_ja = parts[0], EV_RULES_MOVE_TYPO_FIXES.get(parts[1], parts[1])
            note = parts[2] if len(parts) >= 3 else "-"
            if sp_ja not in ja_sp_to_id:
                raise ValueError(f"Unknown JP species in ev_rules.txt moves: {sp_ja}")
            if mv_ja not in ja_mv_to_id:
                raise ValueError(f"Unknown JP move in ev_rules.txt moves: {mv_ja}")
            sp_id = ja_sp_to_id[sp_ja]
            mv_id = ja_mv_to_id[mv_ja]
            below_level: int | None = None
            m = re.search(r"レベル(\d+)以上は可", note)
            if m:
                below_level = int(m.group(1))
            gen2_req_moves.setdefault(sp_id, {})[mv_id] = below_level

    serialized_req: dict[str, list[dict[str, object]]] = {}
    for sp_id in sorted(gen2_req_moves):
        entries = []
        for mv_id in sorted(gen2_req_moves[sp_id]):
            entry: dict[str, object] = {"move": mv_id}
            below = gen2_req_moves[sp_id][mv_id]
            if below is not None:
                entry["belowLevel"] = below
            entries.append(entry)
        serialized_req[sp_id] = entries

    return sorted(gen2_only_species), serialized_req


def box(
    hp: list[int],
    atk: list[int],
    defense: list[int],
    spe: list[int],
    spc: list[int],
    *,
    gen: int | None = None,
    reason: str | None = None,
    min_effort: int | None = None,
) -> dict[str, object]:
    out: dict[str, object] = {
        "bounds": [hp, atk, defense, spe, spc],
    }
    if gen is not None:
        out["gen"] = gen
    if reason is not None:
        out["reason"] = reason
    if min_effort is not None:
        out["minEffort"] = min_effort
    return out


def build_ev_rule_boxes() -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    """Authoritative effort-level bounds from `/tmp/pbs-wiki/ev_rules.txt`.
    Bounds order: [hp, atk, def, spe, spc], inclusive [lo, hi] in 0..=63.
    Explicitly fixes the upstream copy-paste bug in `EffortLevelCheck.java:164-171`
    and includes all spreads proved realizable in `ev_rules.txt`.
    """
    allowed = [
        # Single stat lowered
        box([0, 63], [63, 63], [63, 63], [63, 63], [63, 63]),  # X-63-63-63-63
        box([63, 63], [0, 63], [63, 63], [63, 63], [63, 63]),  # 63-X-63-63-63
        box([63, 63], [63, 63], [21, 63], [63, 63], [63, 63]),  # 63-63-X-63-63 (X>=21)
        box([63, 63], [63, 63], [63, 63], [0, 63], [63, 63]),  # 63-63-63-X-63
        box([63, 63], [63, 63], [63, 63], [63, 63], [22, 63]),  # 63-63-63-63-X (X>=22)
        # HP + Atk
        box([48, 63], [0, 0], [63, 63], [63, 63], [63, 63]),  # X-0-63-63-63 (X>=48)
        box([25, 63], [22, 63], [63, 63], [63, 63], [63, 63]),  # X-Y-63-63-63 (X>=25,Y>=22)
        # HP + Def (ev_rules.txt:636; fixes EffortLevelCheck.java:164-171 copy-paste bug!)
        box([30, 63], [63, 63], [21, 63], [63, 63], [63, 63]),  # X-63-Y-63-63 (X>=30,Y>=21)
        # HP + Spe (ev_rules.txt:649 & 653)
        box([42, 63], [63, 63], [63, 63], [24, 63], [63, 63]),  # X-63-63-Y-63 (X>=42,Y>=24)
        box([32, 63], [63, 63], [63, 63], [32, 63], [63, 63], gen=1),  # Gen1 X>=32,Y>=32
        box([33, 63], [63, 63], [63, 63], [33, 63], [63, 63], gen=2),  # Gen2 X>=33,Y>=33
        # HP + Spc (ev_rules.txt:670)
        box([41, 63], [63, 63], [63, 63], [63, 63], [22, 63]),
        box([32, 63], [63, 63], [63, 63], [63, 63], [23, 63]),
        box([30, 63], [63, 63], [63, 63], [63, 63], [24, 63]),
        # Atk + Spe (ev_rules.txt:688,691,696)
        box([63, 63], [0, 63], [63, 63], [33, 63], [63, 63], gen=1),
        box([63, 63], [0, 63], [63, 63], [39, 63], [63, 63], gen=2),
        # Atk=0 + Spc (ev_rules.txt:701)
        box([63, 63], [0, 0], [63, 63], [63, 63], [16, 63]),
        # Def + Spe (ev_rules.txt:713-714)
        box([63, 63], [63, 63], [28, 63], [28, 63], [63, 63]),
        # Def + Spc (ev_rules.txt:723)
        box([63, 63], [63, 63], [22, 63], [63, 63], [45, 63]),
        # Atk=0 + Def + Spe (ev_rules.txt:674,741; covers 63-0-X-63-63 as well)
        box([63, 63], [0, 0], [0, 63], [33, 63], [63, 63], gen=1),
        box([63, 63], [0, 0], [0, 63], [39, 63], [63, 63], gen=2),
        # Wobbuffet multi-stat spreads (ev_rules.txt:755)
        box([62, 62], [0, 0], [15, 63], [20, 20], [17, 17]),
        box([63, 63], [0, 0], [46, 63], [20, 20], [17, 17]),
    ]

    forbidden = [
        # Proven impossible lower bounds from ev_rules.txt
        box(
            [63, 63], [63, 63], [0, 20], [63, 63], [63, 63],
            reason="def-min-21", min_effort=21,
        ),
        box(
            [63, 63], [63, 63], [63, 63], [63, 63], [0, 21],
            reason="spc-min-22", min_effort=22,
        ),
        box(
            [0, 47], [0, 0], [63, 63], [63, 63], [63, 63],
            reason="hp-with-zero-atk-min-48", min_effort=48,
        ),
        box(
            [63, 63], [0, 0], [63, 63], [63, 63], [0, 15],
            reason="spc-with-zero-atk-min-16", min_effort=16,
        ),
        box(
            [63, 63], [0, 0], [0, 63], [0, 32], [63, 63],
            gen=1, reason="spe-with-zero-atk-min-33", min_effort=33,
        ),
        box(
            [63, 63], [0, 0], [0, 63], [0, 38], [63, 63],
            gen=2, reason="spe-with-zero-atk-min-39", min_effort=39,
        ),
    ]
    return allowed, forbidden


def main() -> int:
    parser = argparse.ArgumentParser(description="Export Castle PBS validation JSON.")
    parser.add_argument("--pbstools-data", type=pathlib.Path, default=DEFAULT_PBSTOOLS_DATA)
    parser.add_argument("--ev-rules", type=pathlib.Path, default=DEFAULT_EV_RULES)
    parser.add_argument("--out", type=pathlib.Path, default=DEFAULT_OUT)
    args = parser.parse_args()

    dex = json.loads((REPO_ROOT / "data" / "gen2stadium2.json").read_text(encoding="utf-8"))
    ja = json.loads((REPO_ROOT / "data" / "i18n-ja.json").read_text(encoding="utf-8"))
    ja_sp_to_id = {v: k for k, v in ja["species"].items()}
    ja_mv_to_id = {v: k for k, v in ja["moves"].items()}

    # 1. levelLowerLimit.csv
    level_lower: dict[str, int] = {}
    for line in read_sjis_lines(args.pbstools_data / "levelLowerLimit.csv"):
        sp_ja, lv_s = line.split(",")
        sp_id = ja_sp_to_id[sp_ja]
        level_lower[sp_id] = int(lv_s)

    # 2. moveWhiteList.csv
    whitelist_moves: dict[str, dict[str, int]] = {}
    for line in read_sjis_lines(args.pbstools_data / "moveWhiteList.csv"):
        sp_ja, mv_ja, lv_s = line.split(",")
        sp_id = ja_sp_to_id[sp_ja]
        mv_id = ja_mv_to_id[mv_ja]
        lv = int(lv_s)
        prev = whitelist_moves.setdefault(sp_id, {}).get(mv_id)
        if prev is None or lv < prev:
            whitelist_moves[sp_id][mv_id] = lv

    # 3. moveBlackList.csv (5th column = min legal level; 256 = permanently banned)
    blacklist_map: dict[str, dict[tuple[str, ...], int]] = {}
    bl_total_rows = 0
    bl_perm = 0
    bl_leveled = 0
    for line in read_sjis_lines(args.pbstools_data / "moveBlackList.csv"):
        sp_ja, m1, m2, m3, lv_s = line.split(",")
        sp_id = ja_sp_to_id[sp_ja]
        combo = tuple(sorted(ja_mv_to_id[m] for m in (m1, m2, m3) if m))
        lv = int(lv_s)
        if len(combo) < 2 or len(combo) > 3:
            raise ValueError(f"Unexpected blacklist arity {len(combo)} in {line}")
        bl_total_rows += 1
        if lv == 256:
            bl_perm += 1
        else:
            bl_leveled += 1
        prev = blacklist_map.setdefault(sp_id, {}).get(combo)
        if prev is None or lv > prev:
            blacklist_map[sp_id][combo] = lv

    # 4. ev_rules.txt
    gen2_only_species, gen2_req_moves = parse_ev_rules(
        args.ev_rules, ja_sp_to_id, ja_mv_to_id
    )
    allowed_boxes, forbidden_boxes = build_ev_rule_boxes()

    species_out: dict[str, dict[str, object]] = {}
    for sp_id in sorted(dex["species"].keys()):
        wl = whitelist_moves.get(sp_id, {})
        moves_sorted = sorted(wl.keys())
        move_min_level = {m: wl[m] for m in moves_sorted if wl[m] > 0}
        bl_entries = []
        for combo in sorted(blacklist_map.get(sp_id, {}).keys()):
            bl_entries.append([list(combo), blacklist_map[sp_id][combo]])
        entry: dict[str, object] = {
            "minLevel": level_lower[sp_id],
            "moves": moves_sorted,
        }
        if move_min_level:
            entry["moveMinLevel"] = move_min_level
        if bl_entries:
            entry["blacklist"] = bl_entries
        species_out[sp_id] = entry

    payload = {
        "schema": "nc2000-castle-validation-v1",
        "bannedSpecies": BANNED_SPECIES,
        "ohkoMoves": OHKO_MOVES,
        "inertItems": INERT_ITEMS,
        "species": species_out,
        "evRules": {
            "gen2OnlySpecies": gen2_only_species,
            "gen2RequiredMoves": gen2_req_moves,
            "tyrogueLine": ["hitmonlee", "hitmonchan", "hitmontop"],
            "allowed": allowed_boxes,
            "forbidden": forbidden_boxes,
        },
    }

    encoded = json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n"
    args.out.write_text(encoded, encoding="utf-8")

    summary = {
        "schema": payload["schema"],
        "species": len(species_out),
        "whitelist_entries": sum(len(v["moves"]) for v in species_out.values()),
        "blacklist_rows": bl_total_rows,
        "blacklist_permanent": bl_perm,
        "blacklist_min_level": bl_leveled,
        "gen2_only_species": len(gen2_only_species),
        "gen2_req_move_species": len(gen2_req_moves),
        "bytes": len(encoded.encode("utf-8")),
    }
    print(json.dumps(summary, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
