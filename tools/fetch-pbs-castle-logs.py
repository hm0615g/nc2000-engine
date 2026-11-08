#!/usr/bin/env python3
"""
fetch-pbs-castle-logs.py

Collects and archives all Gym Leader's Castle (ジムリーダーの城) PBS battle logs
from the PBS Archive web service (https://wp736907.wpx.jp/pbs/index.html).

Output layout:
  data/pbs-castle-logs/
    logs/*.html          Raw EUC-JP HTML battle logs with original names
    site/*.html          Snapshots of index, pokemon_usage, player_battle_count
    manifest.json        Full structured metadata array (rosters, picks, leads, turns, etc.)
    manifest.jsonl       Streaming JSON lines metadata
    summary.json         Dataset summary & statistics
    README.md            Dataset documentation
  data/pbs-castle-logs.zip Clean zip bundle containing logs + manifest

Follows the M2M Output Principle:
  - Silent during execution (no per-item prints).
  - Emits a single compact JSON telemetry line on stdout upon completion.
"""

import hashlib
import json
import os
import re
import sys
import time
import urllib.request
import zipfile
from collections import Counter
from concurrent.futures import ThreadPoolExecutor, as_completed

BASE_URL = "https://wp736907.wpx.jp/pbs"
INDEX_URL = f"{BASE_URL}/index.html?v=20261108083026"
REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
DEST_DIR = os.path.join(REPO_ROOT, "data", "pbs-castle-logs")
LOGS_DIR = os.path.join(DEST_DIR, "logs")
SITE_DIR = os.path.join(DEST_DIR, "site")
ZIP_PATH = os.path.join(REPO_ROOT, "data", "pbs-castle-logs.zip")

ROOM_TO_CASTLE = {
    "C": "第1",
    "D": "第2",
    "E": "第3",
    "F": "第4",
    "G": "第5",
    "H": "第6",
}

DASHES_PATTERN = re.compile(
    r"[\u002D\u058A\u05BE\u1400\u1806\u2010\u2011\u2012\u2013\u2014\u2015\u2212\u2E17\u2E1A\u2E3A\u2E3B\u2E40\u301C\u3030\u30A0\uFE58\uFE63\uFF0D\uFF70]"
)

USER_AGENT = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko)"


def normalize_pokemon_name(s: str) -> str:
    cleaned = DASHES_PATTERN.sub("ー", s)
    return cleaned.strip()


def fetch_url(url: str, retries: int = 3, delay: float = 0.5) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    for attempt in range(retries):
        try:
            with urllib.request.urlopen(req, timeout=20) as resp:
                return resp.read()
        except Exception:
            if attempt == retries - 1:
                raise
            time.sleep(delay * (attempt + 1))
    return b""


def parse_index_items(html_text: str):
    items = []
    blocks = re.findall(r'<div class="log-item">(.*?)</div>\s*</a>\s*</div>', html_text, re.DOTALL)
    if not blocks:
        raw_parts = html_text.split('<div class="log-item">')
        blocks = raw_parts[1:]

    for block in blocks:
        m_href = re.search(r'href="([^"]+)"', block)
        if not m_href:
            continue
        href = m_href.group(1).strip()
        filename = os.path.basename(href)

        m_room = re.search(r'\[([C-H])\]', block)
        room = m_room.group(1) if m_room else ""

        # Player names & winner from index
        m_players = re.search(
            r'class="players">\[[A-H]\]\s*(.+?)\s+vs\s+(.+?)(?:<span class="winner">\(Winner:\s*(.+?)\)</span>)?</div>',
            block,
            re.DOTALL,
        )
        p1 = ""
        p2 = ""
        winner_from_index = ""
        if m_players:
            p1 = m_players.group(1).strip()
            p2 = m_players.group(2).strip()
            if m_players.group(3):
                winner_from_index = m_players.group(3).strip()

        m_time = re.search(r'class="timestamp">([^<]+)</div>', block)
        timestamp = m_time.group(1).strip() if m_time else ""

        items.append({
            "href": href,
            "filename": filename,
            "room": room,
            "castle_room": ROOM_TO_CASTLE.get(room, ""),
            "player1": p1,
            "player2": p2,
            "winner_index": winner_from_index,
            "timestamp": timestamp,
        })
    return items


def extract_player_picks(text: str, player: str):
    if not player:
        return [], None
    pattern = rf'<B>{re.escape(player)}</B>は(?:<B>[^<]+</B>を戻して)?<B>([^<]+)</B>を繰り出した'
    picks = []
    lead = None
    for mon in re.findall(pattern, text):
        nm = normalize_pokemon_name(mon)
        if not picks:
            lead = nm
        if nm not in picks:
            picks.append(nm)
    return picks, lead


def parse_roster_column(chunk: str, picks: list) -> list:
    res = []
    for tt in re.findall(r'<TT>(.*?)</TT>', chunk, re.I | re.S):
        m_lv = re.search(r'LV\s*(\d+)', tt, re.I)
        level = int(m_lv.group(1)) if m_lv else None
        cleaned = re.sub(r'<[^>]+>', ' ', tt).replace('○', '').strip()
        gender = 'M' if '♂' in cleaned else ('F' if '♀' in cleaned else None)
        species_raw = cleaned.replace('♂', '').replace('♀', '')
        species_raw = re.sub(r'LV\s*\d+', '', species_raw).strip()
        species = normalize_pokemon_name(species_raw)
        picked = species in picks
        res.append({
            'species': species,
            'level': level,
            'gender': gender,
            'picked': picked,
        })
    return res


def parse_log_details(raw_bytes: bytes, meta: dict) -> dict:
    try:
        text = raw_bytes.decode("euc-jp", errors="replace")
    except Exception:
        text = raw_bytes.decode("utf-8", errors="replace")

    p1 = meta["player1"]
    p2 = meta["player2"]

    # Winner from inside the log body
    winner = meta.get("winner_index") or ""
    m_win = re.search(r'Color=["\']?GoldenRod["\']?>\s*(.*?)(?:&nbsp;|\s)*ＷＩＮ！', text, re.I)
    if m_win:
        w_raw = re.sub(r'<[^>]+>', '', m_win.group(1)).strip()
        if w_raw:
            winner = w_raw

    # Picks & Leads
    p1_picks, p1_lead = extract_player_picks(text, p1)
    p2_picks, p2_lead = extract_player_picks(text, p2)

    # Teams from preview table
    p1_team = []
    p2_team = []
    m_table = re.search(r'<TABLE[^>]*>(.*?)</TABLE>', text, re.I | re.S)
    if m_table:
        table_html = m_table.group(1)
        tr_matches = re.findall(r'<TR>(.*?)</TR>', table_html, re.I | re.S)
        if len(tr_matches) >= 2:
            roster_tr = tr_matches[1]
            td_chunks = [c for c in re.split(r'<TD[^>]*>', roster_tr, flags=re.I) if c.strip()]
            if len(td_chunks) >= 2:
                p1_team = parse_roster_column(td_chunks[0], p1_picks)
                p2_team = parse_roster_column(td_chunks[1], p2_picks)

    # Approximate turns by HR count
    hr_count = len(re.findall(r'<hr\b', text, re.I))

    # SHA256 of the raw file
    sha256 = hashlib.sha256(raw_bytes).hexdigest()

    return {
        "filename": meta["filename"],
        "href": meta["href"],
        "room": meta["room"],
        "castle_room": meta["castle_room"],
        "timestamp": meta["timestamp"],
        "player1": p1,
        "player2": p2,
        "winner": winner,
        "player1_lead": p1_lead,
        "player2_lead": p2_lead,
        "player1_picks": p1_picks,
        "player2_picks": p2_picks,
        "player1_team": p1_team,
        "player2_team": p2_team,
        "turns_approx": hr_count,
        "size_bytes": len(raw_bytes),
        "sha256": sha256,
    }


def main():
    t_start = time.time()

    os.makedirs(LOGS_DIR, exist_ok=True)
    os.makedirs(SITE_DIR, exist_ok=True)

    # 1. Fetch index page
    index_bytes = fetch_url(INDEX_URL)
    with open(os.path.join(SITE_DIR, "index.html"), "wb") as f:
        f.write(index_bytes)
    index_text = index_bytes.decode("utf-8", errors="replace")

    # 2. Fetch ancillary ranking pages
    for page in ["pokemon_usage.html", "player_battle_count.html"]:
        try:
            page_bytes = fetch_url(f"{BASE_URL}/{page}")
            with open(os.path.join(SITE_DIR, page), "wb") as f:
                f.write(page_bytes)
        except Exception:
            pass

    # 3. Parse index for all log references
    items = parse_index_items(index_text)
    total_logs = len(items)

    # 4. Concurrently download all logs (or reuse already downloaded files if byte size matches)
    downloaded_records = []
    total_bytes = 0

    def process_item(item):
        local_path = os.path.join(LOGS_DIR, item["filename"])
        if os.path.exists(local_path) and os.path.getsize(local_path) > 0:
            with open(local_path, "rb") as f:
                raw = f.read()
        else:
            url = f"{BASE_URL}/{item['href']}"
            raw = fetch_url(url)
            with open(local_path, "wb") as f:
                f.write(raw)
        parsed = parse_log_details(raw, item)
        return parsed, len(raw)

    with ThreadPoolExecutor(max_workers=16) as executor:
        futures = {executor.submit(process_item, it): it for it in items}
        for fut in as_completed(futures):
            parsed, byte_len = fut.result()
            downloaded_records.append(parsed)
            total_bytes += byte_len

    # Sort records chronologically (newest first like index)
    downloaded_records.sort(key=lambda r: r["timestamp"], reverse=True)

    # 5. Write manifest.json & manifest.jsonl
    manifest_path = os.path.join(DEST_DIR, "manifest.json")
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(downloaded_records, f, ensure_ascii=False, indent=2)

    manifest_jsonl_path = os.path.join(DEST_DIR, "manifest.jsonl")
    with open(manifest_jsonl_path, "w", encoding="utf-8") as f:
        for rec in downloaded_records:
            f.write(json.dumps(rec, ensure_ascii=False) + "\n")

    # 6. Generate summary statistics
    rooms_counter = {}
    player_games = {}
    player_wins = {}
    pokemon_roster_count = Counter()
    pokemon_pick_count = Counter()
    pokemon_lead_count = Counter()

    for r in downloaded_records:
        rm = r["room"]
        rooms_counter[rm] = rooms_counter.get(rm, 0) + 1
        p1 = r["player1"]
        p2 = r["player2"]
        w = r["winner"]
        if p1:
            player_games[p1] = player_games.get(p1, 0) + 1
        if p2:
            player_games[p2] = player_games.get(p2, 0) + 1
        if w:
            player_wins[w] = player_wins.get(w, 0) + 1

        for m in r.get("player1_team", []):
            sp = m.get("species")
            if sp:
                pokemon_roster_count[sp] += 1
        for m in r.get("player2_team", []):
            sp = m.get("species")
            if sp:
                pokemon_roster_count[sp] += 1

        for pk in r.get("player1_picks", []):
            pokemon_pick_count[pk] += 1
        for pk in r.get("player2_picks", []):
            pokemon_pick_count[pk] += 1

        if r.get("player1_lead"):
            pokemon_lead_count[r["player1_lead"]] += 1
        if r.get("player2_lead"):
            pokemon_lead_count[r["player2_lead"]] += 1

    summary = {
        "dataset": "Gym Leader's Castle (ジムリーダーの城) PBS Battle Logs",
        "source_url": INDEX_URL,
        "collected_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "total_games": len(downloaded_records),
        "total_raw_bytes": total_bytes,
        "date_range": {
            "newest": downloaded_records[0]["timestamp"] if downloaded_records else "",
            "oldest": downloaded_records[-1]["timestamp"] if downloaded_records else "",
        },
        "rooms": {
            rm: {
                "castle_alias": ROOM_TO_CASTLE.get(rm, ""),
                "games": count,
            }
            for rm, count in sorted(rooms_counter.items())
        },
        "top_active_players": sorted(
            [{"name": k, "games": v, "wins": player_wins.get(k, 0)} for k, v in player_games.items()],
            key=lambda x: x["games"],
            reverse=True,
        )[:20],
        "top_roster_pokemon": [
            {"species": sp, "count": cnt}
            for sp, cnt in pokemon_roster_count.most_common(20)
        ],
        "top_picked_pokemon": [
            {"species": sp, "count": cnt}
            for sp, cnt in pokemon_pick_count.most_common(20)
        ],
        "top_lead_pokemon": [
            {"species": sp, "count": cnt}
            for sp, cnt in pokemon_lead_count.most_common(20)
        ],
    }

    summary_path = os.path.join(DEST_DIR, "summary.json")
    with open(summary_path, "w", encoding="utf-8") as f:
        json.dump(summary, f, ensure_ascii=False, indent=2)

    # 7. Write README.md
    readme_path = os.path.join(DEST_DIR, "README.md")
    with open(readme_path, "w", encoding="utf-8") as f:
        f.write(f"""# Gym Leader's Castle (`ジムリーダーの城`) PBS Battle Logs

Archive of human and bot battle logs collected from the PBS archive at `{BASE_URL}/index.html`.

## Dataset Overview
- **Total Battles**: {len(downloaded_records)}
- **Date Range**: {summary['date_range']['oldest']} ~ {summary['date_range']['newest']}
- **Rooms Covered**:
  - `第1` (Room C): {rooms_counter.get('C', 0)} matches
  - `第2` (Room D): {rooms_counter.get('D', 0)} matches
  - `第3` (Room E): {rooms_counter.get('E', 0)} matches
  - `第4` (Room F): {rooms_counter.get('F', 0)} matches
  - `第5` (Room G): {rooms_counter.get('G', 0)} matches
  - `第6` (Room H): {rooms_counter.get('H', 0)} matches
- **Encoding**: Original EUC-JP HTML files preserved in `logs/`.

## Directory Structure
- `logs/`: {len(downloaded_records)} raw HTML files (e.g. `C_20261108_083024_8654daa31915.html`)
- `site/`: HTML snapshots of the archive UI (`index.html`, `pokemon_usage.html`, `player_battle_count.html`)
- `manifest.json`: Array of structured match metadata (teams, levels, genders, picks, leads, turns)
- `manifest.jsonl`: Line-delimited JSON of match metadata
- `summary.json`: High-level aggregate statistics

## Re-running
```bash
python3 tools/fetch-pbs-castle-logs.py
```
""")

    # 8. Create compressed bundle data/pbs-castle-logs.zip
    with zipfile.ZipFile(ZIP_PATH, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        zf.write(manifest_path, arcname="manifest.json")
        zf.write(summary_path, arcname="summary.json")
        zf.write(readme_path, arcname="README.md")
        for rec in downloaded_records:
            fname = rec["filename"]
            local_p = os.path.join(LOGS_DIR, fname)
            zf.write(local_p, arcname=f"logs/{fname}")

    zip_size = os.path.getsize(ZIP_PATH)
    elapsed = time.time() - t_start

    # M2M single-line structured telemetry
    telemetry = {
        "status": "success",
        "total_logs": total_logs,
        "downloaded": len(downloaded_records),
        "total_raw_bytes": total_bytes,
        "zip_bytes": zip_size,
        "dest_dir": os.path.relpath(DEST_DIR, REPO_ROOT),
        "zip_path": os.path.relpath(ZIP_PATH, REPO_ROOT),
        "duration_sec": round(elapsed, 2),
    }
    print(json.dumps(telemetry, ensure_ascii=False))


if __name__ == "__main__":
    main()
