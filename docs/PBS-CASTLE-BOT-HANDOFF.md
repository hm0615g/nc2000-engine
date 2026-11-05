# Gym Leader's Castle (`PS.cgi`) Resident Discord Bot — Operational Handoff

This document is the complete operational and technical handoff for running and maintaining the Gym Leader's Castle (`ジムリーダーの城`, `http://psense.lib.net/PBS/_/C..O/PS.cgi`) resident Discord bot on an external host (Windows, macOS, or Linux). It is structured so both human operators and AI coding assistants can immediately understand, configure, debug, and extend the system.

---

## 1. System Overview & File Map

The portable bundle (`pbs-castle-bot/`) is self-contained and requires only **Node.js >= 18** (no Rust toolchain, `wasm-pack`, or external `iconv` binary needed):

| Path | Purpose |
|---|---|
| `tools/pbs-discord-bot.js` | Resident Discord Gateway v10 (`ws`) + REST v10 (`fetch`) daemon. Watches `#2000-一撃なし` (or configured channels) for `@mentions`, coordinates room selection (`第1`/`第2`/`第3`), and spawns `pbs-client.js`. |
| `tools/pbs-client.js` | Direct HTTP/EUC-JP client for `psense.lib.net/PBS/_/C..O/PS.cgi` and `PDINPUT.cgi`. Parses EUC-JP battle HTML (`ENTORY`, `CHECK`, `SELECT`, `INPUT`, `CHANGE`, `PDAT`, `LLOG`), exploits Castle-specific information leaks, adapts online to opponent sets, and drives the WASM MCTS engine. |
| `tools/export-pdinput.js` | Encodes 6-Pokémon sets into PBS's `PD0` cookie format (exact Gen 2 stats, DVs, StatExp, moves, items, and the PBS `Bitter Berry -> Ice Berry` bug workaround). |
| `crates/wasm/pkg-node/` | Precompiled Node.js WebAssembly binary (`nc2000_wasm.js` + `nc2000_wasm_bg.wasm`) exposing `Dex` and `ProtocolSearcher`. |
| `data/kodoku-r3/conclusion-equilibrium-deme.json` | Default tier (`--tier default`): APEX4 40:30:15:15 Nash equilibrium mixture over the 6-mon shell `Snorlax55/Cloyster50/Gengar50/Heracross50/Marowak50/Starmie50`. |
| `data/team-pool-v2/team-pool.json` | Mercy / handicap tier (`--tier mercy`): 8 iconic human-origin Tier A teams (`TIER_A_BOX_IDS` in `tools/pbs-client.js`). |
| `data/belief-pool-v3/belief-pool.json` | Base opponent belief pool (77 teams), dynamically reweighted and augmented per match using Castle leaks and opponent history. |
| `data/compiled-preview-v1.json` | Precomputed Team Preview opening book (`compiledPreviewKey()`). |
| `data/pbs-opponent-db.json` | *(Created at runtime)* Persistent opponent memory database (shells, picked frequencies, revealed moves, items, `noLeftovers`, and exact `maxHp`). |
| `data/pbs-matches.jsonl` | *(Created at runtime)* Append-only JSONL log of completed matches for offline prior expansion. |
| `data/discord-events.jsonl` | *(Created at runtime)* Append-only JSONL log of Discord channel events (`MESSAGE_CREATE`, `MESSAGE_UPDATE`, `THREAD_CREATE`) for debugging room-bot interactions. |

---

## 2. Host Setup (Windows / macOS / Linux)

### Step 1: Install Node.js
Install **Node.js v18 or newer** (v20 / v22 LTS recommended) from https://nodejs.org/.

### Step 2: Create & Invite the Discord Bot
1. Open the Discord Developer Portal (https://discord.com/developers/applications) and create an application (or select your existing Bot application).
2. Open the **Bot** tab:
   - Under **Privileged Gateway Intents**, enable **MESSAGE CONTENT INTENT** (**required** so the bot can read `"手加減"`, `"第1"`, `"第2"`, `"第3"`, and `アナウンス [APP]` room links).
   - Click **Reset Token** and copy the bot token.
3. Open **OAuth2 -> URL Generator**:
   - Scopes: `bot`
   - Bot Permissions: `View Channels`, `Send Messages`, `Read Message History`
   - Open the generated URL to invite the bot to the Castle Discord server.

### Step 3: Create `pbs-bot.config.json`
Copy `pbs-bot.config.example.json` to `pbs-bot.config.json` in the root folder and fill in your credentials:

```json
{
  "discordToken": "PASTE_YOUR_DISCORD_BOT_TOKEN_HERE",
  "pbsName": "YOUR_CASTLE_TRAINER_NAME",
  "pbsPass": "your_pass",
  "pbsMsg": "よろしくお願いします",
  "channelIds": [],
  "roomBotIds": [],
  "replyTriggerText": "応戦します",
  "replyMode": "reply",
  "roomEnterGreetText": "よろしくお願いします",
  "postMatchThanksText": "ありがとうございました",
  "iters": 81000,
  "hostTimeoutSec": 240,
  "roomWaitTimeoutSec": 90,
  "autoPickRoomAfterSec": 15,
  "pbsBaseUrl": "http://psense.lib.net/PBS/_/{ROOM}/PS.cgi",
  "observeOnly": false,
  "dryRunPbs": false,
  "logAllChannelEvents": true
}
```

- `pbsPass`: Must be 1–8 ASCII characters (`PS.cgi` `PASS` input has `MaxLength="8"`).
- `channelIds`: Leave as `[]` to respond to `@mentions` in any channel the bot can see, or restrict to specific channel IDs (e.g. `["123456789012345678"]` for `#2000-一撃なし`).

### Step 4: Launch the Bot
- **Windows**: Double-click `start-bot.bat` (automatically runs `npm install --omit=dev` on first launch).
- **macOS / Linux**: Run `./start-bot.sh` (or `npm install --omit=dev && node tools/pbs-discord-bot.js`).

---

## 3. Verified `#2000-一撃なし` Discord Protocol & Room Mapping

### Official Room Mapping (`http://psense.lib.net/PBS/chat/`)
In the Castle Discord server (`#2000-一撃なし`), the three battle rooms are called `第1`, `第2`, and `第3`:

| Castle Discord Name | Room Letter | `PS.cgi` URL |
|---|---|---|
| `第1` / `第一` | `C` | `http://psense.lib.net/PBS/_/C/PS.cgi` |
| `第2` / `第二` | `D` | `http://psense.lib.net/PBS/_/D/PS.cgi` |
| `第3` / `第三` | `E` | `http://psense.lib.net/PBS/_/E/PS.cgi` |

*(Direct room letters `C`–`O` and full `PS.cgi` URLs are also accepted.)*

### End-to-End Match Sequence
1. **Idle State**: The bot maintains a WebSocket connection to Discord Gateway v10 and sends **zero HTTP traffic** to `psense.lib.net`.
2. **Challenge `@mention`**: A player posts e.g. `@Bot 対戦希望します` (or `@Bot 手加減で対戦希望します`).
   - The bot immediately sends a Discord Reply: `応戦します`.
3. **Room Selection (3 supported paths)**:
   - **Path A (Challenger or `アナウンス [APP]` specifies the room)**: The challenger replies `第1へ` / `第2にて` / `では第三でお願いします` (which also causes the server's `アナウンス [APP]` bot to post a `[第1]` / `[第2]` / `[第3]` hyperlink). As soon as our bot sees either message, it posts `よろしくお願いします` and launches `pbs-client.js` in that room (`C` / `D` / `E`).
   - **Path B (Room specified in initial `@mention`)**: If the challenger wrote `@Bot 第2で対戦希望します`, the bot replies `応戦します`, posts `よろしくお願いします`, and immediately enters `第2 (D)`.
   - **Path C (Auto-pick fallback after `autoPickRoomAfterSec` = 15s)**: If the challenger waits for the bot to pick a room, after 15 seconds `probeCastleDiscordRooms()` checks `第1 (C)`, `第2 (D)`, and `第3 (E)` on `psense.lib.net`, selects the room where a player is waiting or the first empty room, posts `第1にて` + `よろしくお願いします` in Discord (triggering `アナウンス [APP]`), and enters that room.
4. **Match Completion**: When `PS.cgi` reaches `WIN` / `LOSE`, the bot cleans up the room via `PS.cgi?ACTION=END` and posts `ありがとうございました` in the Discord channel.

---

## 4. Two-Tier Party Selection & Castle Leak Exploitation

### Two-Tier Strength / Party Configuration
- **Default (`--tier default`)**:
  - Triggered by normal `@mentions` (`@Bot 対戦希望します`).
  - Search budget: `81,000` iterations (`c = 0.4`) + `compiled-preview-v1.json` opening book.
  - Party pool: `pool:nash` (`APEX4` 40:30:15:15 equilibrium mixture over `Snorlax55/Cloyster50/Gengar50/Heracross50/Marowak50/Starmie50`).
- **Mercy / Handicap (`--tier mercy`)**:
  - Triggered when the `@mention` contains any of: `手加減`, `てかげん`, `ハンデ`, `別パ`, `サブ`, `サブパ`, `tier a`, `tiera`, `tier-a`, `tier1`, `弱め`, `やさしく`, `優しく`, `mercy`, `handicap`.
  - Search budget: `81,000` iterations (`c = 0.4`).
  - Party pool: `pool:tier-a` (8 iconic human-origin Tier A teams defined in `TIER_A_BOX_IDS` in `tools/pbs-client.js`: `sample-07`, `sample-08`, `sample-12`, `mjj-2020-rraikou`, `mjj-2014-thdrcomb`, `sample-17-orig`, `sample-11`, `rental-cban-8`).

### Castle-Specific Information Leak Exploitation & Online Adaptation
1. **Persistent Bench Exact HP & Status (`seenFoeMons`)**: `PS.cgi` displays exact numerical `hp / maxHp` for active Pokémon. When an opponent switches out, `pbs-client.js` retains its exact `hp_num / hp_den`, `status`, and revealed item on the bench across turns.
2. **Exact `maxHp` DV & Hidden Power Type Filtering**: `calcSetMaxHp()` and `inferHpTypeMaskFromMaxHp()` use the opponent's exact `maxHp` to downweight incompatible candidate sets in `belief-pool-v3` (e.g. `Marowak@L50` `166` vs `158`, `Heracross@L50` `186` vs `174`) and set `hp_type_mask` (narrowing Hidden Power to <= 4 types) once `hiddenpower` is revealed.
3. **Leftovers Absence Inference (`noLeftovers`)**: If a damaged opponent active Pokémon (`0 < hp < maxHp`) ends a turn without Leftovers recovery and without end-of-turn residual damage, `noLeftovers = true` is inferred and Leftovers-holding candidate sets are downweighted (`×0.02`).
4. **Online Rematch Adaptation (`data/pbs-opponent-db.json`)**: After every game, `recordOpponentObservation()` saves the opponent's 6-mon shell, picked mons, revealed moves, items, and `maxHp`. On rematches, `buildDynamicBeliefPoolJson()` synthesizes an adapted team candidate (`OPPONENT_ADAPTED_*`) and injects it into `ProtocolSearcher`.

---

## 5. Diagnostic & Testing Commands (For Operator or AI Assistant)

```bash
# 1. Run built-in offline state-machine & parser self-test (no Discord token needed)
node tools/pbs-discord-bot.js --selftest

# 2. Passive observation mode: logs all Discord channel messages/embeds to console
#    and data/discord-events.jsonl without replying or entering PS.cgi
node tools/pbs-discord-bot.js --observe

# 3. Discord dry-run mode: replies "応戦します" on @mention and detects 第1/第2/第3,
#    but stops before launching pbs-client.js
node tools/pbs-discord-bot.js --dry-run-pbs

# 4. Play 1 standalone game directly in a specific room (without Discord):
node tools/pbs-client.js --room 第1 --name NC2000 --pass nc2pass --tier default --games 1
```

### Periodic Data Sync Back to Main Research Repo
To expand the main repository's prior with games played on the friend's PC, copy back:
- `data/pbs-opponent-db.json` (aggregated per-player and per-shell observations)
- `data/pbs-matches.jsonl` (per-game match logs)
