#!/usr/bin/env python3
"""
Gate B: 711-Unique-Trajectory Gym Leader's Castle (PBS / PS.cgi) Shadow Replay Harness.

Verifies 3 separated layers across all 711 unique trajectories extracted from
data/pbs-castle-logs/logs/*.html (737 raw HTML files minus 26 prefix duplicates):
  Layer 1: Observation prefix consistency across all 711 unique logs
           (exact HP continuity, Gen 2 legal max-HP bounds, Castle exact integer
           formulas for Leftovers/Poison/Toxic/Burn/Recover/Berries/Spikes/
           Sandstorm/Leech/Sub/Curse/BellyDrum/Recoil/Present, EOT phase & fixed
           1P->2P seat order, Turn-Cut, Sleep/Rest/Confusion duration bounds,
           Sleep Clause, item-presence & Thief consistency, and all 7 §0-C traps).
  Layer 2: Raw PS.cgi outcome consistency across all 701 decided logs (700 WIN + 1 DRAW),
           truncating the 6 post-WIN double-POST logs at the first GoldenRod WIN.
  Layer 3: Tournament overlay classification explicitly separating T81+ (9 games),
           direct OHKO games, 1v1 recoil DRAW override (H_20260407), and non-standard
           pick counts (1v1 probes).

M2M Output Principle: emits ONE compact JSON telemetry line to stdout and writes
full diagnostic details to /tmp/pbs-gate-b-report.json (or --report).
"""

import argparse
import json
import math
import re
import subprocess
import sys
import unicodedata
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple


OHKO_MOVES_JA = {"じわれ", "つのドリル", "ハサミギロチン"}
RECOIL_MOVES_JA = {"すてみタックル", "とっしん", "じごくぐるま", "わるあがき"}
HJK_MOVES_JA = {"とびげり", "とびひざげり"}
SELFKO_MOVES_JA = {"じばく", "だいばくはつ"}
HALF_HEAL_MOVES_JA = {"じこさいせい", "タマゴうみ", "ミルクのみ"}
CALLER_MOVES_JA = {"ゆびをふる", "オウム返し", "ねごと"}
BIND_MOVES_JA = {"まきつく", "しめつける", "ほのおのうず", "うずしお", "からではさむ"}
TWO_TURN_CHARGE_MOVES_JA = {"そらをとぶ", "あなをほる", "ソーラービーム", "ゴッドバード", "かまいたち", "ロケットずつき"}
LOCKED_MOVES_JA = {"あばれる", "げきりん", "はなびらのまい", "ころがる", "がまん"}

# Exact Castle EOT phase rank (smaller runs earlier).
# Screens (1) -> Weather/Sandstorm (2) -> Perish (3) -> [1P Items(4) -> 1P Thaw(5) -> 2P Items(6) -> 2P Thaw(7)] -> Encore(8)
EOT_SCREENS = 1
EOT_WEATHER = 2
EOT_PERISH = 3
EOT_ITEM_P1 = 4
EOT_THAW_P1 = 5
EOT_ITEM_P2 = 6
EOT_THAW_P2 = 7
EOT_ENCORE = 8


def norm_ja(s: str) -> str:
    s = s.replace("\u2212", "ー").replace("\u30fc", "ー").replace("-", "ー")
    s = unicodedata.normalize("NFKC", s)
    return s.strip()


def resolve_data_root(repo_root: Path) -> Path:
    if (repo_root / "data" / "pbs-castle-logs" / "logs").is_dir():
        return repo_root / "data"
    try:
        common = subprocess.check_output(
            ["git", "rev-parse", "--git-common-dir"],
            cwd=str(repo_root),
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
        common_path = Path(common)
        if not common_path.is_absolute():
            common_path = (repo_root / common_path).resolve()
        main_root = common_path.parent
        if (main_root / "data" / "pbs-castle-logs" / "logs").is_dir():
            return main_root / "data"
    except Exception:
        pass
    return repo_root / "data"


def decode_html(path: Path) -> str:
    raw = path.read_bytes()
    try:
        txt = raw.decode("utf-8")
    except UnicodeDecodeError:
        txt = raw.decode("euc_jp", errors="replace")
    return txt.replace("\u2212", "ー")


def strip_tail_link(txt: str) -> str:
    txt = re.sub(
        r'<p\s+Align="right">\s*<a\s+Href="PS\.html">戻る</a>\s*</p>.*',
        "",
        txt,
        flags=re.I | re.S,
    )
    txt = re.sub(r"</body>\s*</html>\s*$", "", txt, flags=re.I | re.S)
    return txt.strip()


def load_vocab(i18n_path: Path, dex_path: Path) -> Dict[str, Any]:
    i18n = json.loads(i18n_path.read_text(encoding="utf-8"))
    dex = json.loads(dex_path.read_text(encoding="utf-8"))
    sp_ja_to_id = {norm_ja(ja): sid for sid, ja in i18n["species"].items()}
    mv_ja_to_id = {norm_ja(ja): mid for mid, ja in i18n["moves"].items()}
    it_ja_to_id = {norm_ja(ja): iid for iid, ja in i18n["items"].items()}
    # Add PBS spelling aliases (e.g. くろいめがね vs くろいメガネ)
    it_ja_to_id[norm_ja("くろいめがね")] = "blackglasses"
    it_ja_to_id[norm_ja("きのみ")] = "berry"
    it_ja_to_id[norm_ja("おうごんのみ")] = "goldberry"
    it_ja_to_id[norm_ja("きのみジュース")] = "berryjuice"
    it_ja_to_id[norm_ja("きせきのみ")] = "miracleberry"
    it_ja_to_id[norm_ja("はっかのみ")] = "mintberry"
    it_ja_to_id[norm_ja("まひなおしのみ")] = "przcureberry"
    it_ja_to_id[norm_ja("どくけしのみ")] = "psncureberry"
    it_ja_to_id[norm_ja("にがいきのみ")] = "bitterberry"
    it_ja_to_id[norm_ja("こおったきのみ")] = "iceberry"
    it_ja_to_id[norm_ja("やけたきのみ")] = "burntberry"
    it_ja_to_id[norm_ja("ふしぎなきのみ")] = "mysteryberry"
    it_ja_to_id[norm_ja("はかいのいでんし")] = "berserkgene"
    it_ja_to_id[norm_ja("たべのこし")] = "leftovers"
    it_ja_to_id[norm_ja("ひかりのこな")] = "brightpowder"
    it_ja_to_id[norm_ja("せんせいのツメ")] = "quickclaw"
    it_ja_to_id[norm_ja("みずたまリボン")] = "polkadotbow"
    it_ja_to_id[norm_ja("ピンクのリボン")] = "pinkbow"
    base_hp = {
        sid: int(data["baseStats"]["hp"])
        for sid, data in dex["species"].items()
    }
    sp_display = {sid: data["name"] for sid, data in dex["species"].items()}
    return {
        "sp_ja_to_id": sp_ja_to_id,
        "mv_ja_to_id": mv_ja_to_id,
        "it_ja_to_id": it_ja_to_id,
        "base_hp": base_hp,
        "sp_display": sp_display,
    }


def legal_hp_bounds(base_hp: int, level: int) -> Tuple[int, int]:
    # Gen 2 HP formula: floor((2*base + 2*dv + floor(ceil(sqrt(ev))/4)) * lv / 100) + lv + 10
    # dv in [0, 15], effort term in [0, 64] (PBS effort 0..63 gives 0..63; Gen 2 cart max 65535 -> 64).
    lo = ((2 * base_hp) * level) // 100 + level + 10
    hi = ((2 * base_hp + 30 + 64) * level) // 100 + level + 10
    return lo, hi


def deduplicate_logs(files: List[Path]) -> Tuple[List[Path], List[Tuple[str, str]]]:
    stripped = {f.name: strip_tail_link(decode_html(f)) for f in files}
    by_hdr: Dict[str, List[str]] = defaultdict(list)
    for name, txt in stripped.items():
        m = re.search(r"<hr[^>]*>", txt, flags=re.I)
        by_hdr[txt[: m.start()] if m else txt].append(name)

    dropped: Dict[str, str] = {}
    for _, names in by_hdr.items():
        if len(names) <= 1:
            continue
        names_sorted = sorted(names, key=lambda n: (-len(stripped[n]), n))
        kept: List[str] = []
        for n in names_sorted:
            t = stripped[n]
            parent = next((k for k in kept if stripped[k].startswith(t)), None)
            if parent is not None:
                dropped[n] = parent
            else:
                kept.append(n)
    unique_files = [f for f in files if f.name not in dropped]
    return unique_files, sorted(dropped.items())


def parse_header(hdr_html: str, vocab: Dict[str, Any]) -> Tuple[List[str], List[List[Dict[str, Any]]]]:
    m_pl = re.search(
        r"<TR><TD>(.*?)</TD><TD[^>]*>.*?ＶＳ.*?</TD><TD>(.*?)</TD></TR>",
        hdr_html,
        flags=re.I | re.S,
    )
    if not m_pl:
        raise ValueError("missing player header row")
    p1 = re.sub(r"<[^>]+>", "", m_pl.group(1)).strip()
    p2 = re.sub(r"<[^>]+>", "", m_pl.group(2)).strip()

    tds = re.findall(
        r'<TD\s+VALIGN="middle">(.*?)(?=<TD\s+VALIGN="middle">|</TD></TR>)',
        hdr_html,
        flags=re.I | re.S,
    )
    if len(tds) != 2:
        raise ValueError(f"expected 2 roster TDs, got {len(tds)}")

    rosters: List[List[Dict[str, Any]]] = [[], []]
    for s_idx, td in enumerate(tds):
        for tt in re.findall(r"<TT>(.*?)</TT>", td, flags=re.I | re.S):
            clean = re.sub(r"<[^>]+>", "", tt).replace("\u3000", " ")
            m_mon = re.match(
                r"^\s*([^\s♂♀]+?)\s*([♂♀]?)\s*LV\s*(\d+)\s*(○?)\s*$",
                clean,
            )
            if not m_mon:
                raise ValueError(f"unparseable roster row: {clean!r}")
            sp_ja = m_mon.group(1)
            sp_key = norm_ja(sp_ja)
            if sp_key not in vocab["sp_ja_to_id"]:
                raise ValueError(f"unknown species: {sp_ja!r}")
            sid = vocab["sp_ja_to_id"][sp_key]
            g_ch = m_mon.group(2)
            gender = "M" if g_ch == "♂" else ("F" if g_ch == "♀" else "")
            level = int(m_mon.group(3))
            has_item = m_mon.group(4) == "○"
            rosters[s_idx].append(
                {
                    "species_ja": sp_key,
                    "species_id": sid,
                    "species_name": vocab["sp_display"][sid],
                    "level": level,
                    "gender": gender,
                    "has_item": has_item,
                    "hp_bounds": legal_hp_bounds(vocab["base_hp"][sid], level),
                }
            )
    return [p1, p2], rosters


def parse_turn_blocks(seg_html: str, players: List[str]) -> List[Dict[str, Any]]:
    """Parse all <P ALIGN=...>...</P> blocks inside a single turn segment."""
    blocks: List[Dict[str, Any]] = []
    # Match <P ALIGN="left|right|center">...</P> (plus optional adjacent <!-- ... //-->)
    pat = re.compile(
        r'<P\s+ALIGN="(left|right|center)"[^>]*>(.*?)</P>(?:\s*<!--\s*(.*?)\s*//-->)?',
        flags=re.I | re.S,
    )
    for m in pat.finditer(seg_html):
        align_str = m.group(1).lower()
        align_side = 0 if align_str == "left" else (1 if align_str == "right" else None)
        inner = m.group(2)
        comment = m.group(3)
        alt_m = re.search(r'ALT="([^"]*)"', inner, flags=re.I)
        alt_raw = alt_m.group(1) if alt_m else None
        alt_side: Optional[int] = None
        if alt_raw is not None:
            # Match exact player name + "の" (Trap 7 safe even when player name has は/の)
            if alt_raw == f"{players[0]}の":
                alt_side = 0
            elif alt_raw == f"{players[1]}の":
                alt_side = 1
        # Split on BOTH <BR> and raw \n (Trap 5: Curse / Nightmare raw newline HP lines)
        raw_lines = re.split(r"<BR\s*/?>|\r?\n", inner, flags=re.I)
        lines: List[Dict[str, Any]] = []
        for rl in raw_lines:
            # Remove <IMG ...> first
            rl_noimg = re.sub(r"<IMG[^>]*>", "", rl, flags=re.I)
            bolds = [norm_ja(b) for b in re.findall(r"<B>(.*?)</B>", rl_noimg, flags=re.I | re.S)]
            text = re.sub(r"<[^>]+>", "", rl_noimg).strip()
            if text:
                lines.append({"text": text, "bolds": bolds, "raw": rl_noimg.strip()})
        if lines:
            blocks.append(
                {
                    "align": align_side,
                    "alt_side": alt_side,
                    "alt_raw": alt_raw,
                    "lines": lines,
                    "comment": comment,
                }
            )
    return blocks


class MonState:
    __slots__ = (
        "species_ja",
        "species_id",
        "species_name",
        "level",
        "gender",
        "has_item",
        "hp_bounds",
        "picked",
        "fainted",
        "cur_hp",
        "max_hp",
        "exact_max_hp_confirmed",
        "status",
        "is_rest_sleep",
        "sleep_turns",
        "confused",
        "conf_turns",
        "tox_counter",
        "has_sub",
        "transformed",
        "mimic_move",
        "orig_item",
        "gained_item",
        "lost_item",
        "no_leftovers",
        "moves_own",
        "moves_excluded_ohko",
        "moves_excluded_foreign",
        "silent_focus_band_procs",
    )

    def __init__(self, info: Dict[str, Any]):
        self.species_ja: str = info["species_ja"]
        self.species_id: str = info["species_id"]
        self.species_name: str = info["species_name"]
        self.level: int = info["level"]
        self.gender: str = info["gender"]
        self.has_item: bool = info["has_item"]
        self.hp_bounds: Tuple[int, int] = info["hp_bounds"]
        self.picked: bool = False
        self.fainted: bool = False
        self.cur_hp: Optional[int] = None
        self.max_hp: Optional[int] = None
        self.exact_max_hp_confirmed: bool = False
        self.status: str = "none"
        self.is_rest_sleep: bool = False
        self.sleep_turns: int = 0
        self.confused: bool = False
        self.conf_turns: int = 0
        self.tox_counter: int = 0
        self.has_sub: bool = False
        self.transformed: bool = False
        self.mimic_move: Optional[str] = None
        self.orig_item: Optional[str] = None
        self.gained_item: bool = False
        self.lost_item: bool = False
        self.no_leftovers: bool = False
        self.moves_own: set = set()
        self.moves_excluded_ohko: set = set()
        self.moves_excluded_foreign: set = set()
        self.silent_focus_band_procs: int = 0

    def on_switch_out(self) -> None:
        self.confused = False
        self.conf_turns = 0
        self.has_sub = False
        self.transformed = False
        self.mimic_move = None
        if self.status == "tox":
            self.status = "psn"
            self.tox_counter = 0


def parse_and_replay_log(
    path: Path,
    vocab: Dict[str, Any],
) -> Dict[str, Any]:
    """
    Full 3-layer Gate B shadow replay for one unique Castle log.
    Returns structured parsed summary + contradictions + hidden assignments + overlay flags.
    """
    txt = decode_html(path)
    # Find FIRST GoldenRod WIN or Black DRAW (truncate the 6 post-WIN double-POST logs)
    m_end = re.search(
        r'<font[^>]*Color="(GoldenRod|Black)"[^>]*>(.*?)</font>',
        txt,
        flags=re.I | re.S,
    )
    post_win_truncated = False
    outcome_kind = "undecided"
    winner_name: Optional[str] = None
    if m_end:
        tail_after = re.sub(
            r'<p\s+Align="right">\s*<a\s+Href="PS\.html">戻る</a>\s*</p>',
            "",
            txt[m_end.end() :],
            flags=re.I | re.S,
        )
        tail_after = re.sub(r"</center>|</body>|</html>", "", tail_after, flags=re.I | re.S).strip()
        if tail_after:
            post_win_truncated = True
        banner = m_end.group(2).replace("&nbsp;", " ").strip()
        if "ＷＩＮ" in banner or "WIN" in banner:
            outcome_kind = "win"
            winner_name = re.sub(r"\s*ＷＩＮ！?$", "", banner).strip()
        elif "ＤＲＡＷ" in banner or "DRAW" in banner:
            outcome_kind = "draw"
        txt = txt[: m_end.end()]

    parts = re.split(r"<hr[^>]*>", txt, flags=re.I)
    players, raw_rosters = parse_header(parts[0], vocab)
    sides: List[List[MonState]] = [
        [MonState(m) for m in raw_rosters[0]],
        [MonState(m) for m in raw_rosters[1]],
    ]
    sp_lookup: List[Dict[str, MonState]] = [
        {m.species_ja: m for m in sides[0]},
        {m.species_ja: m for m in sides[1]},
    ]
    active: List[Optional[MonState]] = [None, None]

    prefix_contradictions: List[Dict[str, Any]] = []
    outcome_contradictions: List[Dict[str, Any]] = []
    checks = Counter()
    traps_seen = Counter()
    direct_ohko_uses: List[Tuple[int, int, str]] = []
    metronome_calls_excluded = 0
    last_move_of_side: List[Optional[str]] = [None, None]
    last_selfko_side: Optional[int] = None
    last_recoil_turn_side: Optional[int] = None

    def add_contradiction(turn_no: int, code: str, detail: str) -> None:
        prefix_contradictions.append({"turn": turn_no, "code": code, "detail": detail})

    def record_orig_item(mon: MonState, item_id: str, turn_no: int, allow_no_flag: bool = False) -> None:
        if mon.gained_item:
            return
        if not mon.has_item and not allow_no_flag:
            add_contradiction(turn_no, "item_without_header_flag", f"{mon.species_id} revealed {item_id} without ○")
            return
        if mon.orig_item is not None and mon.orig_item != item_id:
            add_contradiction(turn_no, "conflicting_item", f"{mon.species_id}: {mon.orig_item} vs {item_id}")
            return
        mon.orig_item = item_id
        checks["item_reveals"] += 1

    def apply_hp_change(
        mon: MonState,
        old_hp: int,
        new_hp: int,
        turn_no: int,
        reason: str,
        expect_faint_in_turn: bool = True,
    ) -> int:
        checks["hp_updates"] += 1
        # Establish / verify max_hp
        if mon.cur_hp is None:
            # First time this mon's HP is observed!
            # If this is damage (new < old) or cost (sub/drum/curse) or full-HP heal, old_hp is exact max_hp
            # (unless mon was somehow healed first from unknown HP, which can't happen before taking damage).
            mon.cur_hp = old_hp
            if mon.max_hp is None or old_hp > mon.max_hp:
                mon.max_hp = old_hp
                mon.exact_max_hp_confirmed = True
            if new_hp > mon.max_hp:
                mon.max_hp = new_hp
        else:
            if old_hp != mon.cur_hp:
                add_contradiction(
                    turn_no,
                    "hp_continuity",
                    f"{mon.species_id} ({reason}): expected cur_hp={mon.cur_hp}, saw {old_hp}->{new_hp}",
                )
            if old_hp > (mon.max_hp or 0):
                mon.max_hp = old_hp
            if new_hp > (mon.max_hp or 0):
                mon.max_hp = new_hp

        # Check Gen 2 legal HP upper bound (and lower bound once confirmed full)
        if mon.max_hp is not None:
            lo, hi = mon.hp_bounds
            if mon.max_hp > hi or (mon.exact_max_hp_confirmed and mon.max_hp < lo):
                add_contradiction(
                    turn_no,
                    "illegal_max_hp",
                    f"{mon.species_id} L{mon.level}: observed max_hp={mon.max_hp} outside [{lo},{hi}]",
                )

        # Trap 1: HP X -> 0 without 倒れた。。 means internal HP = 1 (Focus Band / False Swipe)
        if new_hp == 0 and not expect_faint_in_turn:
            traps_seen["trap1_hp_zero_survive"] += 1
            mon.cur_hp = 1
            mon.silent_focus_band_procs += 1
            return old_hp - 1
        mon.cur_hp = new_hp
        return old_hp - new_hp

    def switch_in(side_idx: int, sp_ja_norm: str, turn_no: int, is_lead: bool = False) -> Optional[MonState]:
        mon = sp_lookup[side_idx].get(sp_ja_norm)
        if mon is None:
            add_contradiction(turn_no, "unknown_roster_mon", f"side {side_idx} sent {sp_ja_norm}")
            return None
        if mon.fainted:
            add_contradiction(turn_no, "switched_in_fainted", f"side {side_idx} sent fainted {mon.species_id}")
        old = active[side_idx]
        if old is not None and old is not mon and not old.fainted:
            old.on_switch_out()
        mon.picked = True
        active[side_idx] = mon
        checks["switches"] += 1
        return mon

    # Match switch patterns using exact player names (Trap 7 safe!)
    p_esc = [re.escape(players[0]), re.escape(players[1])]

    def parse_switch_line(text: str, align_side: Optional[int]) -> Optional[Tuple[int, Optional[str], str]]:
        # Returns (side, out_sp_ja_or_None, in_sp_ja)
        # Try exact player names first
        order = [align_side, 1 - align_side] if align_side in (0, 1) else [0, 1]
        for s in order:
            m_ret = re.match(rf"^{p_esc[s]}は(.+?)を戻して(.+?)を繰り出した！$", text)
            if m_ret:
                if "は" in players[s]:
                    traps_seen["trap7_player_has_ha"] += 1
                return (s, norm_ja(m_ret.group(1)), norm_ja(m_ret.group(2)))
            m_send = re.match(rf"^{p_esc[s]}は(.+?)を繰り出した！$", text)
            if m_send:
                if "は" in players[s]:
                    traps_seen["trap7_player_has_ha"] += 1
                return (s, None, norm_ja(m_send.group(1)))
        return None

    # Pre-check Turn 0 (leads)
    if len(parts) >= 2:
        t0_blocks = parse_turn_blocks(parts[1], players)
        for blk in t0_blocks:
            for ln in blk["lines"]:
                sw = parse_switch_line(ln["text"], blk["align"])
                if sw:
                    s_idx, _, in_sp = sw
                    switch_in(s_idx, in_sp, 0, is_lead=True)
                elif "はかいのいでんしがさどうした！" in ln["text"]:
                    s_idx = blk["alt_side"] if blk["alt_side"] is not None else blk["align"]
                    if s_idx in (0, 1) and active[s_idx] is not None:
                        record_orig_item(active[s_idx], "berserkgene", 0)
                        active[s_idx].lost_item = True
                        active[s_idx].confused = True
                        active[s_idx].conf_turns = 0

    n_turns = max(0, len(parts) - 2)
    last_turn_was_recoil_ko = False
    last_turn_was_selfko = False

    # Replay Turns 1..n_turns
    for t_idx in range(1, n_turns + 1):
        seg = parts[t_idx + 1]
        blocks = parse_turn_blocks(seg, players)
        fainted_this_turn: set = set()
        # Pre-scan which (side, species_ja) actually have a 倒れた。。 line in this turn (for Trap 1!)
        faint_markers: Counter = Counter()
        for blk in blocks:
            s_hint = blk["alt_side"] if blk["alt_side"] is not None else blk["align"]
            for ln in blk["lines"]:
                m_f = re.match(r"^(.+?)は倒れた。。$", ln["text"])
                if m_f and s_hint in (0, 1):
                    faint_markers[(s_hint, norm_ja(m_f.group(1)))] += 1

        # Per-turn trackers
        cur_actor: Optional[int] = None
        cur_move_ja: Optional[str] = None
        in_metronome_call = False
        in_sleeptalk_call = False
        in_mirrormove_call = False
        last_hit_dealt = 0
        last_hit_was_sub = False
        leftovers_healed_this_turn = [False, False]
        had_switch_faint_turn_cut = False
        seen_any_move_this_turn = False
        eot_phase_max = 0
        eot_phase_side = {EOT_SCREENS: -1, EOT_WEATHER: -1, EOT_PERISH: -1, EOT_ENCORE: -1}
        last_turn_was_recoil_ko = False
        last_turn_was_selfko = False

        # Detect if this turn ends in a switch-in Turn-Cut (no move before faint)
        for bi, blk in enumerate(blocks):
            align = blk["align"]
            alt_side = blk["alt_side"]
            eff_side = alt_side if alt_side is not None else align

            # Trap 4: Weather lines have ALIGN="center" and no ALT
            if align is None and alt_side is None:
                traps_seen["trap4_center_weather"] += 1

            # Check Trap 5: raw newline inside Curse / Nightmare block
            if len(blk["lines"]) >= 2 and any(
                w in blk["lines"][0]["text"]
                for w in ("はのろわれている！", "はあくむにうなされている！")
            ):
                traps_seen["trap5_curse_nightmare_newline"] += 1

            for li, ln in enumerate(blk["lines"]):
                text = ln["text"]

                # 1. Switch / send-out line
                sw = parse_switch_line(text, align)
                if sw:
                    s_idx, out_sp, in_sp = sw
                    cur_actor = s_idx
                    cur_move_ja = None
                    in_metronome_call = False
                    in_sleeptalk_call = False
                    in_mirrormove_call = False
                    switch_in(s_idx, in_sp, t_idx)
                    continue

                # 2. Roar / Whirlwind drag-in (Trap 6: uses actor's ALT, NOT target's!)
                m_drag = re.match(r"^(.+?)が戦闘にひきずりだされた！$", text)
                if m_drag:
                    traps_seen["trap6_roar_encore_alt"] += 1
                    drag_side = (1 - cur_actor) if cur_actor in (0, 1) else (1 - eff_side if eff_side in (0, 1) else None)
                    if drag_side in (0, 1):
                        switch_in(drag_side, norm_ja(m_drag.group(1)), t_idx)
                    continue

                # 3. Berserk Gene on switch-in
                if "はかいのいでんしがさどうした！" in text and eff_side in (0, 1):
                    mon = active[eff_side]
                    if mon is not None:
                        record_orig_item(mon, "berserkgene", t_idx)
                        mon.lost_item = True
                        mon.confused = True
                        mon.conf_turns = 0
                    continue

                # 4. Move declaration: <B>Sp</B>の<B>Move</B>！
                m_mv = re.match(r"^(.+?)の(.+?)！$", text)
                if m_mv and len(ln["bolds"]) >= 2 and eff_side in (0, 1):
                    sp_ja = norm_ja(m_mv.group(1))
                    mv_ja = norm_ja(m_mv.group(2))
                    if mv_ja in vocab["mv_ja_to_id"] and sp_ja in sp_lookup[eff_side]:
                        seen_any_move_this_turn = True
                        mon = active[eff_side]
                        if mon is None or mon.species_ja != sp_ja:
                            # Fallback if active drifted (should not happen)
                            add_contradiction(
                                t_idx,
                                "actor_mismatch",
                                f"side {eff_side} move by {sp_ja}, active is {mon.species_ja if mon else None}",
                            )
                            mon = sp_lookup[eff_side][sp_ja]
                            active[eff_side] = mon

                        # Trap 2: Sketch success prints メロメロ！ followed by をスケッチした！
                        lookahead_texts = [x["text"] for x in blk["lines"][li + 1 :]]
                        if bi + 1 < len(blocks):
                            lookahead_texts.extend(x["text"] for x in blocks[bi + 1]["lines"][:2])
                        if mv_ja == "メロメロ" and any("をスケッチした！" in lt for lt in lookahead_texts):
                            traps_seen["trap2_sketch_attract_misprint"] += 1
                            mv_ja = "スケッチ"

                        # Determine whether this is a called follow-up move in the same actor's turn
                        is_called_move = (cur_actor == eff_side) and (
                            in_metronome_call or in_sleeptalk_call or in_mirrormove_call
                        )
                        cur_actor = eff_side
                        cur_move_ja = mv_ja
                        last_move_of_side[eff_side] = mv_ja
                        last_hit_dealt = 0
                        last_hit_was_sub = False

                        mv_id = vocab["mv_ja_to_id"][mv_ja]
                        if mv_ja == "ゆびをふる":
                            in_metronome_call = True
                            if not mon.transformed and mon.mimic_move != mv_id:
                                mon.moves_own.add(mv_id)
                        elif mv_ja == "ねごと":
                            in_sleeptalk_call = True
                            if not mon.transformed and mon.mimic_move != mv_id:
                                mon.moves_own.add(mv_id)
                        elif mv_ja == "オウム返し":
                            in_mirrormove_call = True
                            if not mon.transformed and mon.mimic_move != mv_id:
                                mon.moves_own.add(mv_id)
                        else:
                            if is_called_move:
                                if in_metronome_call or in_mirrormove_call:
                                    metronome_calls_excluded += 1
                                    mon.moves_excluded_foreign.add(mv_id)
                                elif in_sleeptalk_call:
                                    # In Gen 2 Sleep Talk only calls user's own moves (unless transformed/mimic)
                                    if not mon.transformed and mon.mimic_move != mv_id and mv_ja != "わるあがき":
                                        if mv_ja in OHKO_MOVES_JA:
                                            mon.moves_excluded_ohko.add(mv_id)
                                            direct_ohko_uses.append((t_idx, eff_side, mv_id))
                                        else:
                                            mon.moves_own.add(mv_id)
                                in_metronome_call = False
                                in_sleeptalk_call = False
                                in_mirrormove_call = False
                            else:
                                in_metronome_call = False
                                in_sleeptalk_call = False
                                in_mirrormove_call = False
                                if mv_ja == "わるあがき":
                                    pass
                                elif mon.transformed or mon.mimic_move == mv_id:
                                    mon.moves_excluded_foreign.add(mv_id)
                                elif mv_ja in OHKO_MOVES_JA:
                                    mon.moves_excluded_ohko.add(mv_id)
                                    direct_ohko_uses.append((t_idx, eff_side, mv_id))
                                else:
                                    mon.moves_own.add(mv_id)

                        if mv_ja in SELFKO_MOVES_JA:
                            last_selfko_side = eff_side
                            last_turn_was_selfko = True
                        if mv_ja == "へんしん":
                            mon.transformed = True
                        continue

                # 5. Mimic / Sketch copy tracking
                m_mim = re.search(r"は(.+?)をコピーした！|は(.+?)を覚えた！|は(.+?)をスケッチした！", text)
                if m_mim and eff_side in (0, 1) and active[eff_side] is not None:
                    copied_ja = norm_ja(next(g for g in m_mim.groups() if g))
                    if copied_ja in vocab["mv_ja_to_id"]:
                        active[eff_side].mimic_move = vocab["mv_ja_to_id"][copied_ja]
                    continue

                # 6. Thief item steal: AはBからITEMを奪った！
                m_thief = re.match(r"^(.+?)は(.+?)から(.+?)を奪った！$", text)
                if m_thief and eff_side in (0, 1):
                    thief_mon = active[eff_side]
                    victim_mon = active[1 - eff_side]
                    item_ja = norm_ja(m_thief.group(3))
                    item_id = vocab["it_ja_to_id"].get(item_ja)
                    if victim_mon is not None and item_id is not None:
                        record_orig_item(victim_mon, item_id, t_idx)
                        victim_mon.lost_item = True
                    if thief_mon is not None:
                        thief_mon.gained_item = True
                    continue

                # 7. Trap 7: Leech Seed miss ("しかし、うまくきまらなかった。")
                if cur_move_ja == "やどりぎのタネ" and "うまくきまらなかった" in text:
                    traps_seen["trap7_leech_seed_miss"] += 1
                    continue

                # 8. Trap 6: Encore target has attacker's ALT
                if "はアンコールをうけた！" in text:
                    traps_seen["trap6_roar_encore_alt"] += 1
                    continue

                # 9. Substitute tracking
                if "分身をつくりだした！" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    active[eff_side].has_sub = True
                    continue
                if "にかわって分身が攻撃を受けた！" in text or "分身はきえてしまった" in text:
                    last_hit_was_sub = True
                    opp_s = (1 - cur_actor) if cur_actor in (0, 1) else (eff_side if eff_side in (0, 1) else None)
                    if "分身はきえてしまった" in text and opp_s in (0, 1) and active[opp_s] is not None:
                        active[opp_s].has_sub = False
                    continue

                # 10. Sleep / Rest / Confusion / Status / Freeze tracking
                if "眠りはじめた！" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    m_st = active[eff_side]
                    m_st.status = "slp"
                    m_st.is_rest_sleep = True
                    m_st.sleep_turns = 0
                    m_st.tox_counter = 0
                    continue
                if "は眠ってしまった！" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    m_st = active[eff_side]
                    # Sleep Clause check (ignore Rest): no other teammate on this side may be asleep
                    other_slp = any(
                        (o is not m_st and not o.fainted and o.status == "slp")
                        for o in sides[eff_side]
                    )
                    if other_slp:
                        add_contradiction(t_idx, "sleep_clause_violation", f"side {eff_side} second sleep on {m_st.species_id}")
                    checks["sleep_clause_checks"] += 1
                    m_st.status = "slp"
                    m_st.is_rest_sleep = False
                    m_st.sleep_turns = 0
                    continue
                if "はぐうぐう眠っている" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    m_st = active[eff_side]
                    m_st.status = "slp"
                    m_st.sleep_turns += 1
                    max_slp = 2 if m_st.is_rest_sleep else 4
                    if m_st.sleep_turns > max_slp:
                        add_contradiction(
                            t_idx,
                            "sleep_duration_exceeded",
                            f"{m_st.species_id} slept {m_st.sleep_turns} > {max_slp} (rest={m_st.is_rest_sleep})",
                        )
                    checks["sleep_duration_checks"] += 1
                    continue
                if "は目を覚ました！" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    m_st = active[eff_side]
                    if m_st.is_rest_sleep and m_st.sleep_turns != 2:
                        add_contradiction(
                            t_idx,
                            "rest_sleep_not_2",
                            f"{m_st.species_id} Rest woke after {m_st.sleep_turns} turns",
                        )
                    m_st.status = "none"
                    m_st.is_rest_sleep = False
                    m_st.sleep_turns = 0
                    continue
                if "は混乱した！" in text and eff_side in (0, 1):
                    # In Castle, self-confusion (Outrage/Thrash/Swagger? Wait: Swagger uses target's ALT!)
                    target_mon = active[eff_side]
                    if target_mon is not None:
                        target_mon.confused = True
                        target_mon.conf_turns = 0
                    continue
                if "は混乱している" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    m_st = active[eff_side]
                    m_st.confused = True
                    m_st.conf_turns += 1
                    if m_st.conf_turns > 4:
                        add_contradiction(t_idx, "confusion_duration_exceeded", f"{m_st.species_id} conf_turns={m_st.conf_turns}")
                    checks["confusion_duration_checks"] += 1
                    continue
                if "は正気に戻った！" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    m_st = active[eff_side]
                    m_st.confused = False
                    m_st.conf_turns = 0
                    continue
                if ("は猛毒におかされた！" in text or "は猛毒をあびた！" in text) and eff_side in (0, 1) and active[eff_side] is not None:
                    active[eff_side].status = "tox"
                    active[eff_side].tox_counter = 0
                    continue
                if ("は毒におかされた！" in text or "は毒をあびた！" in text or "はどくをあびた！" in text or "は毒をうけた！" in text) and eff_side in (0, 1) and active[eff_side] is not None:
                    active[eff_side].status = "psn"
                    active[eff_side].tox_counter = 0
                    continue
                if "は火傷をおった！" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    active[eff_side].status = "brn"
                    continue
                if "は麻痺した！" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    active[eff_side].status = "par"
                    continue
                if "はこおりづけになった！" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    active[eff_side].status = "frz"
                    continue
                if "の氷がとけた！" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    active[eff_side].status = "none"
                    # Check if this is EOT natural thaw (not mid-turn fire move thaw)
                    prev_text = blk["lines"][li - 1]["text"] if li > 0 else ""
                    if cur_move_ja not in ("かえんほうしゃ", "だいもんじ", "ほのおのパンチ", "ひのこ", "かえんぐるま", "せいなるほのお") and "HP" not in prev_text:
                        ph = EOT_THAW_P1 if eff_side == 0 else EOT_THAW_P2
                        if ph < eot_phase_max:
                            add_contradiction(t_idx, "eot_order_violation", f"thaw phase {ph} after {eot_phase_max}")
                        eot_phase_max = max(eot_phase_max, ph)
                        checks["eot_order_checks"] += 1
                    continue
                if "状態異常が治った！" in text or "のすずの音色が響き渡った！" in text:
                    # Heal Bell resets ALL team status AND resets Toxic counter (Probe 12B!)
                    bell_side = cur_actor if cur_actor in (0, 1) else eff_side
                    if bell_side in (0, 1):
                        for o in sides[bell_side]:
                            if not o.fainted:
                                o.status = "none"
                                o.is_rest_sleep = False
                                o.sleep_turns = 0
                                o.tox_counter = 0
                    continue

                # 11. Berry & Item reveals + EOT phase order checks
                if "たべのこしでＨＰを回復" in text and eff_side in (0, 1) and active[eff_side] is not None:
                    record_orig_item(active[eff_side], "leftovers", t_idx)
                    leftovers_healed_this_turn[eff_side] = True
                    ph = EOT_ITEM_P1 if eff_side == 0 else EOT_ITEM_P2
                    if ph < eot_phase_max:
                        add_contradiction(t_idx, "eot_order_violation", f"leftovers side {eff_side} (ph={ph}) after {eot_phase_max}")
                    eot_phase_max = max(eot_phase_max, ph)
                    checks["eot_order_checks"] += 1
                    continue
                if any(b in text for b in ("おうごんのみでＨＰを回復", "きのみジュースでＨＰを回復", "きのみでＨＰを回復")) and eff_side in (0, 1) and active[eff_side] is not None:
                    mon = active[eff_side]
                    item_id = (
                        "goldberry" if "おうごんのみ" in text else
                        "berryjuice" if "きのみジュース" in text else "berry"
                    )
                    record_orig_item(mon, item_id, t_idx)
                    mon.lost_item = True
                    ph = EOT_ITEM_P1 if eff_side == 0 else EOT_ITEM_P2
                    if ph < eot_phase_max:
                        add_contradiction(t_idx, "eot_order_violation", f"hp-berry side {eff_side} (ph={ph}) after {eot_phase_max}")
                    eot_phase_max = max(eot_phase_max, ph)
                    checks["eot_order_checks"] += 1
                    continue
                if any(
                    b in text
                    for b in (
                        "きせきのみで",
                        "はっかのみで",
                        "まひなおしのみで",
                        "どくけしのみで",
                        "にがいきのみで",
                        "こおったきのみで",
                        "やけたきのみで",
                        "ふしぎなきのみで",
                    )
                ) and eff_side in (0, 1) and active[eff_side] is not None:
                    mon = active[eff_side]
                    if "きせきのみ" in text:
                        bid = "miracleberry"
                    elif "はっかのみ" in text:
                        bid = "mintberry"
                    elif "まひなおしのみ" in text:
                        bid = "przcureberry"
                    elif "どくけしのみ" in text:
                        bid = "psncureberry"
                    elif "にがいきのみ" in text:
                        # Castle ID 81 disguise bug (Probe 6): ID 81 (iceberry) cures confusion while printing にがいきのみ!
                        bid = "iceberry"
                    elif "こおったきのみ" in text:
                        bid = "iceberry"
                    elif "やけたきのみ" in text:
                        bid = "burntberry"
                    else:
                        bid = "mysteryberry"
                    record_orig_item(mon, bid, t_idx)
                    mon.lost_item = True
                    if "こんらんを回復" in text:
                        mon.confused = False
                        mon.conf_turns = 0
                    elif any(w in text for w in ("めをさました", "まひをなおした", "どくをなおした", "やけどをなおした", "こおりをとかした")):
                        mon.status = "none"
                        mon.is_rest_sleep = False
                        mon.sleep_turns = 0
                        mon.tox_counter = 0
                    continue

                # EOT phase order markers: screens / weather / perish / encore
                if any(w in text for w in ("リフレクターの効果がなくなった", "ひかりのかべの効果がなくなった")) and eff_side in (0, 1):
                    if EOT_SCREENS < eot_phase_max or eff_side < eot_phase_side[EOT_SCREENS]:
                        add_contradiction(t_idx, "eot_order_violation", f"screen end side {eff_side}")
                    eot_phase_max = max(eot_phase_max, EOT_SCREENS)
                    eot_phase_side[EOT_SCREENS] = eff_side
                    checks["eot_order_checks"] += 1
                    continue
                if ("すなあらしのダメージを受けている" in text or "砂嵐が襲う" in text) and eff_side in (0, 1):
                    if EOT_WEATHER < eot_phase_max or eff_side < eot_phase_side[EOT_WEATHER]:
                        add_contradiction(t_idx, "eot_order_violation", f"sandstorm side {eff_side}")
                    eot_phase_max = max(eot_phase_max, EOT_WEATHER)
                    eot_phase_side[EOT_WEATHER] = eff_side
                    checks["eot_order_checks"] += 1
                    continue
                if "ほろびのカウントが" in text and eff_side in (0, 1):
                    if EOT_PERISH < eot_phase_max or eff_side < eot_phase_side[EOT_PERISH]:
                        add_contradiction(t_idx, "eot_order_violation", f"perish side {eff_side}")
                    eot_phase_max = max(eot_phase_max, EOT_PERISH)
                    eot_phase_side[EOT_PERISH] = eff_side
                    checks["eot_order_checks"] += 1
                    continue
                if "アンコール状態がとけた" in text and eff_side in (0, 1):
                    if EOT_ENCORE < eot_phase_max or eff_side < eot_phase_side[EOT_ENCORE]:
                        add_contradiction(t_idx, "eot_order_violation", f"encore end side {eff_side}")
                    eot_phase_max = max(eot_phase_max, EOT_ENCORE)
                    eot_phase_side[EOT_ENCORE] = eff_side
                    checks["eot_order_checks"] += 1
                    continue

                # 12. Destiny Bond KO sets opponent's HP to 0 without an HP line
                if "をみちづれにした！" in text and eff_side in (0, 1):
                    tgt_mon = active[1 - eff_side]
                    if tgt_mon is not None:
                        tgt_mon.cur_hp = 0
                    continue

                # 13. Faint line
                m_fnt = re.match(r"^(.+?)は倒れた。。$", text)
                if m_fnt and eff_side in (0, 1):
                    f_sp = norm_ja(m_fnt.group(1))
                    f_mon = sp_lookup[eff_side].get(f_sp)
                    if f_mon is None:
                        add_contradiction(t_idx, "unknown_faint_mon", f"side {eff_side} faint {f_sp}")
                    else:
                        f_mon.fainted = True
                        f_mon.cur_hp = 0
                        f_mon.on_switch_out()
                        fainted_this_turn.add((eff_side, f_sp))
                        if not seen_any_move_this_turn:
                            had_switch_faint_turn_cut = True
                        if cur_move_ja in RECOIL_MOVES_JA:
                            last_recoil_turn_side = cur_actor
                            last_turn_was_recoil_ko = True
                    continue

                # 13. HP Change line: <Sp>の HP A→ B
                m_hp = re.match(r"^(.+?)の\s*HP\s*(\d+)→\s*(\d+)$", text)
                if m_hp:
                    hp_sp = norm_ja(m_hp.group(1))
                    old_hp = int(m_hp.group(2))
                    new_hp = int(m_hp.group(3))
                    prev_text = blk["lines"][li - 1]["text"] if li > 0 else ""

                    # Trap 3: Present heal line has reversed arrow (new -> old) AND attacker's ALT!
                    if (
                        cur_move_ja == "プレゼント"
                        and cur_actor in (0, 1)
                        and li > 0
                        and "プレゼント" in blk["lines"][0]["text"]
                    ):
                        traps_seen["trap3_present_reversed_heal"] += 1
                        tgt_side = 1 - cur_actor
                        tgt_mon = active[tgt_side]
                        if tgt_mon is None or tgt_mon.species_ja != hp_sp:
                            tgt_mon = sp_lookup[tgt_side].get(hp_sp)
                        if tgt_mon is not None:
                            # Reversed arrow: new_hp is BEFORE heal, old_hp is AFTER heal!
                            real_old, real_new = new_hp, old_hp
                            if tgt_mon.max_hp is not None:
                                exp_new = min(tgt_mon.max_hp, real_old + 80)
                                if real_new != exp_new:
                                    add_contradiction(
                                        t_idx,
                                        "present_heal_formula",
                                        f"{tgt_mon.species_id} {real_old}->{real_new} expected {exp_new} (M={tgt_mon.max_hp})",
                                    )
                                checks["present_heal_checks"] += 1
                            apply_hp_change(tgt_mon, real_old, real_new, t_idx, "present_heal")
                        continue

                    # Resolve which side owns this HP line
                    hp_side = eff_side
                    block_first = blk["lines"][0]["text"]
                    hp_idx_in_block = sum(
                        1
                        for prev_l in blk["lines"][:li]
                        if re.match(r"^.+?の\s*HP\s*\d+→\s*\d+$", prev_l["text"])
                    )
                    # Pain Split shares one <P> block (attacker's ALT) for BOTH sides:
                    # 1st HP line = attacker (cur_actor), 2nd HP line = target (1 - cur_actor).
                    is_pain_split = any("おたがいの体力をわかちあった" in prev_l["text"] for prev_l in blk["lines"][:li])
                    # Leech Seed shares one <P> block (victim's ALT) for BOTH sides:
                    # 1st HP line = victim (eff_side), 2nd HP line = opponent healer (1 - eff_side).
                    is_leech_block = "たいりょくをうばう" in block_first or "やどりぎ" in block_first
                    if is_pain_split and cur_actor in (0, 1):
                        hp_side = cur_actor if hp_idx_in_block == 0 else (1 - cur_actor)
                    elif is_leech_block and eff_side in (0, 1):
                        hp_side = eff_side if hp_idx_in_block == 0 else (1 - eff_side)
                    elif hp_side not in (0, 1) or active[hp_side] is None or active[hp_side].species_ja != hp_sp:
                        if hp_side in (0, 1) and active[1 - hp_side] is not None and active[1 - hp_side].species_ja == hp_sp:
                            hp_side = 1 - hp_side
                    if hp_side not in (0, 1):
                        continue
                    mon = active[hp_side]
                    if mon is None or mon.species_ja != hp_sp:
                        mon = sp_lookup[hp_side].get(hp_sp)
                    if mon is None:
                        add_contradiction(t_idx, "unknown_hp_mon", f"side {hp_side} {hp_sp}")
                        continue

                    # Determine whether a 0 result actually has a 倒れた。。 in this turn (Trap 1!)
                    will_faint = faint_markers[(hp_side, hp_sp)] > 0

                    # Exact Castle integer formula checks!
                    reason = "attack"
                    if is_pain_split:
                        reason = "pain_split"
                    elif is_leech_block and hp_idx_in_block == 1:
                        reason = "leech_heal"
                    elif "たべのこし" in prev_text:
                        reason = "leftovers"
                        if mon.max_hp is not None:
                            exp = min(mon.max_hp, old_hp + max(1, mon.max_hp // 16))
                            if new_hp != exp:
                                add_contradiction(t_idx, "leftovers_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                            checks["leftovers_formula_checks"] += 1
                    elif any(b in prev_text for b in ("おうごんのみ", "きのみジュース", "きのみでＨＰ")):
                        reason = "hp_berry"
                        heal_amt = 30 if "おうごんのみ" in prev_text else (20 if "きのみジュース" in prev_text else 10)
                        if mon.max_hp is not None:
                            if old_hp >= mon.max_hp // 2:
                                add_contradiction(t_idx, "hp_berry_threshold", f"{mon.species_id} triggered at {old_hp}/{mon.max_hp}")
                            exp = min(mon.max_hp, old_hp + heal_amt)
                            if new_hp != exp:
                                add_contradiction(t_idx, "hp_berry_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp}")
                            checks["hp_berry_checks"] += 1
                    elif "毒のダメージ" in prev_text:
                        reason = "poison"
                        if mon.status == "tox":
                            mon.tox_counter += 1
                            if mon.max_hp is not None:
                                exp = max(0, old_hp - max(1, mon.max_hp // 16) * mon.tox_counter)
                                if new_hp != exp:
                                    add_contradiction(t_idx, "toxic_formula", f"{mon.species_id} n={mon.tox_counter} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                                checks["toxic_formula_checks"] += 1
                        else:
                            if mon.max_hp is not None:
                                exp = max(0, old_hp - max(1, mon.max_hp // 8))
                                if new_hp != exp:
                                    add_contradiction(t_idx, "poison_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                                checks["poison_formula_checks"] += 1
                    elif "火傷のダメージ" in prev_text:
                        reason = "burn"
                        if mon.max_hp is not None:
                            exp = max(0, old_hp - max(1, mon.max_hp // 8))
                            if new_hp != exp:
                                add_contradiction(t_idx, "burn_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                            checks["burn_formula_checks"] += 1
                    elif "まきびしのダメージ" in prev_text:
                        reason = "spikes"
                        if mon.max_hp is not None:
                            exp = max(0, old_hp - max(1, mon.max_hp // 8))
                            if new_hp != exp:
                                add_contradiction(t_idx, "spikes_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                            checks["spikes_formula_checks"] += 1
                    elif "すなあらしのダメージを受けている" in prev_text or "砂嵐が襲う" in prev_text:
                        reason = "sandstorm"
                        if mon.max_hp is not None:
                            exp = max(0, old_hp - max(1, mon.max_hp // 8))
                            if new_hp != exp:
                                add_contradiction(t_idx, "sandstorm_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                            checks["sandstorm_formula_checks"] += 1
                    elif "やどりぎ" in prev_text and new_hp < old_hp:
                        reason = "leech_drain"
                        if mon.max_hp is not None:
                            exp = max(0, old_hp - max(1, mon.max_hp // 8))
                            if new_hp != exp:
                                add_contradiction(t_idx, "leech_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                            checks["leech_formula_checks"] += 1
                    elif "分身をつくりだした" in prev_text:
                        reason = "substitute"
                        if mon.max_hp is not None:
                            exp = old_hp - (mon.max_hp // 4)
                            if new_hp != exp:
                                add_contradiction(t_idx, "sub_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                            checks["sub_formula_checks"] += 1
                    elif "はのろわれている！" in prev_text or "はあくむにうなされている！" in prev_text:
                        reason = "curse_or_nightmare"
                        if mon.max_hp is not None:
                            exp = max(0, old_hp - max(1, mon.max_hp // 4))
                            if new_hp != exp:
                                add_contradiction(t_idx, "curse_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                            checks["curse_formula_checks"] += 1
                    elif "攻撃の反動をうけた" in prev_text:
                        reason = "recoil"
                        if not last_hit_was_sub and last_hit_dealt > 0:
                            exp = max(0, old_hp - (last_hit_dealt // 4))
                            # Account for possible silent Focus Band on recoil if new_hp == 1 and exp == 0
                            if new_hp != exp and not (exp == 0 and new_hp == 1):
                                add_contradiction(t_idx, "recoil_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (dealt={last_hit_dealt})")
                            checks["recoil_formula_checks"] += 1
                    elif cur_move_ja in HALF_HEAL_MOVES_JA and cur_actor == hp_side and new_hp > old_hp:
                        reason = "half_heal"
                        if mon.max_hp is not None:
                            # Castle strictly uses floor(max_hp / 2), NEVER ceil!
                            exp = min(mon.max_hp, old_hp + (mon.max_hp // 2))
                            if new_hp != exp:
                                add_contradiction(t_idx, "half_heal_formula", f"{mon.species_id} {old_hp}->{new_hp} != {exp} (M={mon.max_hp})")
                            checks["half_heal_checks"] += 1
                    elif "眠りはじめた" in prev_text or (cur_move_ja == "ねむる" and cur_actor == hp_side and new_hp > old_hp):
                        reason = "rest_heal"
                        if mon.max_hp is None or new_hp > mon.max_hp:
                            mon.max_hp = new_hp
                            mon.exact_max_hp_confirmed = True
                        elif new_hp != mon.max_hp:
                            add_contradiction(t_idx, "rest_full_heal", f"{mon.species_id} Rest healed to {new_hp} != {mon.max_hp}")
                        checks["rest_heal_checks"] += 1

                    dealt = apply_hp_change(mon, old_hp, new_hp, t_idx, reason, expect_faint_in_turn=will_faint)
                    if reason == "attack" and dealt > 0 and cur_actor == 1 - hp_side:
                        last_hit_dealt = dealt
                        # Check for Focus Band silent survival at 1 HP? (valid hidden item)
                    continue

        # End of turn: check no-leftovers deduction on non-Turn-Cut, non-final turns
        if not had_switch_faint_turn_cut and t_idx < n_turns:
            for s_idx in (0, 1):
                mon = active[s_idx]
                if (
                    mon is not None
                    and not mon.fainted
                    and mon.cur_hp is not None
                    and mon.max_hp is not None
                    and 0 < mon.cur_hp < mon.max_hp
                    and not leftovers_healed_this_turn[s_idx]
                    and not mon.lost_item
                    and not mon.gained_item
                ):
                    mon.no_leftovers = True

    # Layer 1 final roster checks: picked count <= 3 (standard) or <= 6 (raw CGI)
    picked_counts = [sum(1 for m in sides[0] if m.picked), sum(1 for m in sides[1] if m.picked)]
    fainted_counts = [sum(1 for m in sides[0] if m.fainted), sum(1 for m in sides[1] if m.fainted)]
    for s_idx in (0, 1):
        if picked_counts[s_idx] > 6:
            add_contradiction(n_turns, "picked_gt_6", f"side {s_idx} picked {picked_counts[s_idx]}")

    # Layer 2: Raw PS.cgi Outcome Consistency (701 decided logs)
    winner_side: Optional[int] = None
    outcome_reason = "undecided"
    if outcome_kind == "win":
        if winner_name == players[0]:
            winner_side = 0
        elif winner_name == players[1]:
            winner_side = 1
        else:
            outcome_contradictions.append({"code": "unknown_winner_name", "winner": winner_name, "players": players})

        if winner_side is not None:
            loser_side = 1 - winner_side
            # In raw PS.cgi, a win happens when:
            # (a) loser lost all picked mons (3 faints in standard 3v3, or 1 faint in 1v1 probe when no replacement came in),
            # (b) 1v1 selfdestruct double-KO (selfdestruct user loses -> opponent wins),
            # (c) L_TURN judge (alive count -> sum(hp/max_hp) ratio).
            if fainted_counts[loser_side] >= 3 and fainted_counts[winner_side] < 3:
                outcome_reason = "ko_3v3"
            elif fainted_counts[loser_side] == 3 and fainted_counts[winner_side] == 3:
                if last_turn_was_selfko and last_selfko_side == loser_side:
                    outcome_reason = "selfko_1v1_loser"
                else:
                    # Perish / other 1v1 double faint where PS.cgi awarded win to winner_side
                    outcome_reason = "double_ko_win"
            elif fainted_counts[loser_side] == picked_counts[loser_side] and picked_counts[loser_side] < 3 and last_turn_was_selfko:
                outcome_reason = "probe_1v1_selfko"
            elif fainted_counts[loser_side] == picked_counts[loser_side] and picked_counts[loser_side] > 0:
                outcome_reason = "ko_custom_pick_size"
            else:
                # L_TURN judge check
                alive_w = 3 - fainted_counts[winner_side]
                alive_l = 3 - fainted_counts[loser_side]
                if alive_w >= alive_l:
                    outcome_reason = "turn_limit_judge"
                else:
                    outcome_contradictions.append(
                        {
                            "code": "outcome_mismatch",
                            "winner_side": winner_side,
                            "fainted": fainted_counts,
                            "picked": picked_counts,
                        }
                    )
    elif outcome_kind == "draw":
        # Raw PS.cgi DRAW on 1v1 recoil double-KO (H_20260407) or sandstorm/judge tie
        if fainted_counts[0] == fainted_counts[1] and last_turn_was_recoil_ko:
            outcome_reason = "recoil_1v1_raw_draw"
        elif fainted_counts[0] == fainted_counts[1]:
            outcome_reason = "raw_draw_equal_faints"
        else:
            outcome_contradictions.append(
                {"code": "draw_mismatch", "fainted": fainted_counts, "picked": picked_counts}
            )

    # Layer 3: Tournament Overlay Classification
    overlay_flags = {
        "t81_plus": n_turns >= 81,
        "direct_ohko": len(direct_ohko_uses) > 0,
        "recoil_1v1_draw_override": outcome_reason == "recoil_1v1_raw_draw",
        "non_standard_pick_count": (outcome_kind != "undecided" and any(p < 3 and f == p for p, f in zip(picked_counts, fainted_counts))),
    }

    # Hidden input assignment summary (every unobserved field explicitly tagged)
    hidden_assignment = []
    for s_idx in (0, 1):
        side_assign = []
        for m in sides[s_idx]:
            side_assign.append(
                {
                    "speciesId": m.species_id,
                    "level": m.level,
                    "picked": m.picked,
                    "maxHp": m.max_hp if m.max_hp is not None else f"unobserved[{m.hp_bounds[0]}..{m.hp_bounds[1]}]",
                    "item": m.orig_item if m.orig_item is not None else ("none" if not m.has_item else "unobserved"),
                    "silentFocusBandProcs": m.silent_focus_band_procs,
                }
            )
        hidden_assignment.append(side_assign)

    return {
        "file": path.name,
        "players": players,
        "turns": n_turns,
        "post_win_truncated": post_win_truncated,
        "outcome_kind": outcome_kind,
        "winner_side": winner_side,
        "outcome_reason": outcome_reason,
        "picked_counts": picked_counts,
        "fainted_counts": fainted_counts,
        "prefix_contradictions": prefix_contradictions,
        "outcome_contradictions": outcome_contradictions,
        "overlay": overlay_flags,
        "direct_ohko_uses": direct_ohko_uses,
        "metronome_calls_excluded": metronome_calls_excluded,
        "checks": dict(checks),
        "traps_seen": dict(traps_seen),
        "sides": sides,
        "hidden_assignment": hidden_assignment,
    }


def main() -> int:
    repo_root = Path(__file__).resolve().parent.parent
    data_root = resolve_data_root(repo_root)

    parser = argparse.ArgumentParser(description="Gate B: 711-unique Castle log shadow replay verifier")
    parser.add_argument("--logs-dir", type=Path, default=data_root / "pbs-castle-logs" / "logs")
    parser.add_argument("--i18n", type=Path, default=repo_root / "data" / "i18n-ja.json")
    parser.add_argument("--dex", type=Path, default=repo_root / "data" / "gen2stadium2.json")
    parser.add_argument("--report", type=Path, default=Path("/tmp/pbs-gate-b-report.json"))
    args = parser.parse_args()

    vocab = load_vocab(args.i18n, args.dex)
    all_files = sorted(args.logs_dir.glob("*.html"))
    unique_files, dropped = deduplicate_logs(all_files)

    total_checks = Counter()
    total_traps = Counter()
    outcome_reasons = Counter()
    prefix_contradictions_all: List[Dict[str, Any]] = []
    outcome_contradictions_all: List[Dict[str, Any]] = []
    post_win_files: List[str] = []
    t81_files: List[Tuple[str, int]] = []
    ohko_files: List[Tuple[str, List[Tuple[int, int, str]]]] = []
    recoil_draw_files: List[str] = []
    non_std_pick_files: List[str] = []
    per_log_traces: List[Dict[str, Any]] = []
    wins = 0
    draws = 0
    undecided = 0

    for f in unique_files:
        res = parse_and_replay_log(f, vocab)
        total_checks.update(res["checks"])
        total_traps.update(res["traps_seen"])
        outcome_reasons[res["outcome_reason"]] += 1
        if res["outcome_kind"] == "win":
            wins += 1
        elif res["outcome_kind"] == "draw":
            draws += 1
        else:
            undecided += 1
        if res["post_win_truncated"]:
            post_win_files.append(res["file"])
        if res["overlay"]["t81_plus"]:
            t81_files.append((res["file"], res["turns"]))
        if res["overlay"]["direct_ohko"]:
            ohko_files.append((res["file"], res["direct_ohko_uses"]))
        if res["overlay"]["recoil_1v1_draw_override"]:
            recoil_draw_files.append(res["file"])
        if res["overlay"]["non_standard_pick_count"]:
            non_std_pick_files.append(res["file"])
        for c in res["prefix_contradictions"]:
            prefix_contradictions_all.append({"file": res["file"], **c})
        for c in res["outcome_contradictions"]:
            outcome_contradictions_all.append({"file": res["file"], **c})
        per_log_traces.append(
            {
                "file": res["file"],
                "players": res["players"],
                "turns": res["turns"],
                "outcome_kind": res["outcome_kind"],
                "winner_side": res["winner_side"],
                "outcome_reason": res["outcome_reason"],
                "post_win_truncated": res["post_win_truncated"],
                "overlay": res["overlay"],
                "prefix_contradictions": res["prefix_contradictions"],
                "outcome_contradictions": res["outcome_contradictions"],
                "hidden_assignment": res["hidden_assignment"],
            }
        )

    status = (
        "PASS"
        if len(prefix_contradictions_all) == 0 and len(outcome_contradictions_all) == 0
        else "FAIL"
    )
    summary = {
        "total_html": len(all_files),
        "prefix_duplicates_excluded": len(dropped),
        "unique_logs": len(unique_files),
        "post_win_truncated": len(post_win_files),
        "decided_logs": wins + draws,
        "wins": wins,
        "draws": draws,
        "undecided_logs": undecided,
        "prefix_contradictions": len(prefix_contradictions_all),
        "raw_outcome_contradictions": len(outcome_contradictions_all),
        "overlay_t81_plus": len(t81_files),
        "overlay_ohko": len(ohko_files),
        "overlay_recoil_1v1_draw": len(recoil_draw_files),
        "overlay_non_standard_pick": len(non_std_pick_files),
        "checks": dict(total_checks),
        "traps_seen": dict(total_traps),
        "status": status,
    }
    report = {
        "summary": summary,
        "dropped_duplicates": dropped,
        "post_win_files": post_win_files,
        "t81_files": t81_files,
        "ohko_files": ohko_files,
        "recoil_draw_files": recoil_draw_files,
        "non_std_pick_files": non_std_pick_files,
        "outcome_reasons": dict(outcome_reasons),
        "prefix_contradictions": prefix_contradictions_all,
        "outcome_contradictions": outcome_contradictions_all,
        "logs": per_log_traces,
    }
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False))
    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())
