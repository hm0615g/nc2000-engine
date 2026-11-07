# Gym Leader's Castle (`PS.cgi`) Resident Discord Bot — Operational Handoff

This document is the operational handoff for running the Gym Leader's Castle (`ジムリーダーの城`, `http://psense.lib.net/PBS/_/C..O/PS.cgi`) resident Discord bot from an existing `nc2000-engine` clone (alongside or instead of the Pokémon Showdown bot `tools/ps-client.js`).

---

## 1. Quick Start After `git pull`

If `tools/ps-client.js` is already working on this machine, `crates/wasm/pkg-node/` and `tools/node_modules/ws` are already present — no extra dependencies or Rust builds are required.

```bash
# 1. Pull latest master
git pull

# 2. Verify parser & state machine (runs offline, no token needed)
node tools/pbs-discord-bot.js --selftest

# 3. Create local config from template (gitignored)
cp pbs-bot.config.example.json pbs-bot.config.json
# Edit pbs-bot.config.json to set:
#   "discordToken": "<Discord Bot Token>",
#   "pbsName": "<Trainer Name in Castle>",
#   "pbsPass": "<1-8 char ASCII password>"

# 4. Start the resident Discord bot
node tools/pbs-discord-bot.js
```

*(Alternatively, you can pass flags directly without a config file, just like `tools/ps-client.js`:)*
```bash
node tools/pbs-discord-bot.js --token "$DISCORD_TOKEN" --name "NC2000" --pass "nc2pass"
```

---

## 2. Discord Bot Token & Portal Setup

1. Open Discord Developer Portal (https://discord.com/developers/applications) and select/create your Bot application.
2. In the **Bot** tab:
   - Under **Privileged Gateway Intents**, turn **ON** **`MESSAGE CONTENT INTENT`** (**required** so the bot can read `"手加減"`, `"第1"`, `"第2"`, `"第3"`, and `アナウンス [APP]` room links).
   - Copy the Bot Token into `pbs-bot.config.json` (`"discordToken"`).
3. In **OAuth2 -> URL Generator**:
   - Scopes: `bot`
   - Bot Permissions: `View Channels`, `Send Messages`, `Read Message History`
   - Invite the bot to the Castle Discord server.

---

## 3. Architecture: How the 3 Layers Fit Together

```text
[Layer 3: Discord Bot]  tools/pbs-discord-bot.js
  │  Connects to Discord Gateway v10 (ws) + REST v10 (fetch).
  │  Idles with zero HTTP traffic to psense.lib.net until @mentioned.
  │  Replies "応戦します", resolves room 第1(C) through 第6(H),
  │  and spawns 1 game of tools/pbs-client.js.
  ▼
[Layer 2: Castle Adapter]  tools/pbs-client.js + tools/export-pdinput.js
  │  Encodes 6-mon party into PD0 cookie & registers via PDINPUT.cgi.
  │  Drives PS.cgi (EUC-JP, pure-JS codec — no external iconv binary needed).
  │  Parses ENTORY / CHECK / SELECT / INPUT / CHANGE / PDAT / LLOG.
  │  Tracks Castle-specific leaks (bench exact HP/status, exact maxHp DVs,
  │  noLeftovers inference) and updates data/pbs-opponent-db.json.
  ▼
[Layer 1: Bot Core (WASM MCTS + Data)]
  ・crates/wasm/pkg-node/nc2000_wasm.js (Dex + ProtocolSearcher)
  ・data/kodoku-r3/conclusion-equilibrium-deme.json (Default: APEX4 Nash mixture)
  ・data/team-pool-v2/team-pool.json (Mercy: 8 human-origin Tier A teams)
  ・data/belief-pool-v3/belief-pool.json (Base opponent belief pool)
  ・data/compiled-preview-v1.json (Compiled Team Preview opening book)
```

---

## 4. Verified `#2000-一撃なし` Discord Protocol & Room Mapping

### Official Castle Discord Room Table (`http://psense.lib.net/PBS/chat/`)

| Castle Discord Name | Room Letter | `PS.cgi` URL |
|---|---|---|
| `第1` / `第一` | `C` | `http://psense.lib.net/PBS/_/C/PS.cgi` |
| `第2` / `第二` | `D` | `http://psense.lib.net/PBS/_/D/PS.cgi` |
| `第3` / `第三` | `E` | `http://psense.lib.net/PBS/_/E/PS.cgi` |
| `第4` / `第四` | `F` | `http://psense.lib.net/PBS/_/F/PS.cgi` |
| `第5` / `第五` | `G` | `http://psense.lib.net/PBS/_/G/PS.cgi` |
| `第6` / `第六` | `H` | `http://psense.lib.net/PBS/_/H/PS.cgi` |

*(Direct room letters `C`–`O` and full `PS.cgi` URLs are also accepted.)*

### Match Flow in `#2000-一撃なし`
1. **Challenge `@mention`**: A user posts e.g. `@Bot 対戦希望します` (or `@Bot 手加減で対戦希望します`).
   - Bot immediately sends a Discord Reply: `応戦します`.
2. **Room Resolution (3 paths)**:
   - **Path A (Standard `#2000-一撃なし` flow)**: Challenger replies `第1へ` / `第2にて` / `では第三でお願いします` (which also triggers the server's `アナウンス [APP]` bot to post a `[第1]` through `[第6]` link). As soon as our bot sees either message, it posts `よろしくお願いします` and launches `tools/pbs-client.js` in that room (`C` through `H`).
   - **Path B (Room included in initial `@mention`)**: If the challenger wrote `@Bot 第2で対戦希望します`, the bot replies `応戦します`, posts `よろしくお願いします`, and enters `第2 (D)` immediately.
   - **Path C (Auto-pick fallback after `autoPickRoomAfterSec` = 15s)**: If 15 seconds pass after `応戦します` without the challenger naming a room, `probeCastleDiscordRooms()` checks `第1 (C)` through `第6 (H)` on `psense.lib.net`, picks the room where the challenger is waiting or the first empty room, posts `第1にて` + `よろしくお願いします` in Discord (triggering `アナウンス [APP]`), and enters that room.
3. **Post-Match**: When the battle finishes (`WIN` / `LOSE`), the bot cleans up the room via `PS.cgi?ACTION=END` and posts `ありがとうございました` in the Discord channel.

### Result semantics

- Completed Castle LLOG names the actual winner as `TRAINER_NAME WIN!` after Unicode normalization.
- Result handling uses that winner name and does not infer WIN/LOSE from P1/P2 position.
- Final `ACTION=LLOG` is parsed through `parseLlogHtml()` and merged into `seenFoeMons` / `lastLlogFoeMons` before recording match outcomes and opponent observations so final-turn moves (e.g., `Selfdestruct` / `Explosion`), faints, PP uses, and item reveals are persisted.

---

## 5. Two-Stage Party Tiers & Castle Leak Exploitation

### Two-Stage Party Selection
- **Default (`--tier default`)**:
  - Used on normal `@mentions`.
  - `81,000` iterations (`c = 0.4`) + `pool:nash` (`data/kodoku-r3/conclusion-equilibrium-deme.json`, APEX4 40:30:15:15 mixture over `Snorlax55/Cloyster50/Gengar50/Heracross50/Marowak50/Starmie50`) + `data/compiled-preview-v1.json`.
- **Mercy / Handicap (`--tier mercy`)**:
  - Triggered when the `@mention` includes any of: `手加減`, `てかげん`, `ハンデ`, `別パ`, `サブ`, `サブパ`, `tier a`, `tiera`, `tier-a`, `tier1`, `弱め`, `やさしく`, `優しく`, `mercy`, `handicap`.
  - `81,000` iterations (`c = 0.4`) + `pool:tier-a` (`TIER_A_BOX_IDS` in `tools/pbs-client.js`: 8 iconic human-origin Tier A teams from `data/team-pool-v2/team-pool.json`).

### Castle-Specific Information Leak Exploitation & Online Adaptation
1. **Persistent Bench Exact HP & Status (`seenFoeMons`)**: `PS.cgi` shows exact `hp / maxHp` for active Pokémon. When an opponent mon switches out, its exact `hp_num / hp_den`, `status`, and revealed `ItemKnowledge` are preserved on the bench across turns.
2. **Exact `maxHp` DV & Hidden Power Type Filtering**: `calcSetMaxHp()` and `inferHpTypeMaskFromMaxHp()` downweight `belief-pool-v3` sets whose `maxHp` disagrees with the observed `maxHp` (`×0.03`) and set `hp_type_mask` (narrowing Hidden Power to <= 4 types) once `hiddenpower` is revealed.
3. **Leftovers Absence Inference (`noLeftovers`)**: When a damaged opponent active mon (`0 < hp < maxHp`) ends a normal turn without Leftovers recovery and without residual damage, `noLeftovers = true` is inferred and Leftovers-holding candidate sets are downweighted (`×0.02`).
4. **Online Rematch Adaptation & Prior Logging**:
   - `data/pbs-opponent-db.json`: Persists each opponent's 6-mon shell, picked frequencies, revealed moves, items, `noLeftovers`, and `maxHp`, synthesizing an adapted candidate team (`OPPONENT_ADAPTED_*`) on rematches.
   - `data/pbs-matches.jsonl`: Appends a JSON summary after every game for offline prior expansion.
5. **Raw Trace Archiving & Automatic Post-Match Parity Audit (`tools/pbs-parity-audit.js`)**:
   - `data/pbs-traces/<matchId>.jsonl` (git-ignored): Automatically saves a complete per-match JSONL bundle containing raw `PS.cgi` HTML (`TEAM_PREVIEW_CHECK`, per-turn `BATTLE_INPUT`/`BATTLE_CHANGE`, `PDAT`, `LLOG`, `FINISHED`), reconstructed `ProtocolTracker` / `PositionSpec` states, sanitizer clamp diffs, and `<select Name="MOVE">` projection paths.
   - At the end of every match, `tools/pbs-client.js` automatically runs `auditMatchTrace()` across independent information channels (Top Status Table vs `PDAT` vs `LLOG` vs `PositionSpec` vs Rust WASM `ProtocolSearcher.setPosition()`).
   - If any parser desync, unrecognized Japanese log line, sanitizer forced clamp, or action fallback occurs, `[PARITY_WARN]` is printed and the structured anomaly report is appended to `data/pbs-anomalies.jsonl`.

---

## 6. Diagnostic Commands

```bash
# Run offline state-machine, room-parser & parity-audit self-test
node tools/pbs-discord-bot.js --selftest

# Re-parse and audit all archived raw match traces against current parser + WASM engine
node tools/pbs-parity-audit.js data/pbs-traces

# Passive observation mode (logs Discord events to console & data/discord-events.jsonl without replying)
node tools/pbs-discord-bot.js --observe

# Discord dry-run mode (replies "応戦します" and detects 第1 through 第6 without entering PS.cgi)
node tools/pbs-discord-bot.js --dry-run-pbs

# Direct standalone Castle battle without Discord (e.g. in 第1 = Room C)
node tools/pbs-client.js --room 第1 --name NC2000 --pass nc2pass --tier default --games 1
```
