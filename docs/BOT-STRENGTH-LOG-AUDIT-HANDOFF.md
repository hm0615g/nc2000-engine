# Bot Strength & Battle-Log Audit Handoff

> **Status**: Analysis-only handoff on HEAD [`42cf9f69`](../crates/bot/src/blind.rs). No code edits or locked-artifact changes have been made yet.
>
> **Scope**: Separates already-fixed historical log issues from live suboptimalities and bugs still active on current HEAD, combining `omniscient_audit` HEAD traces (`48` games), `BoundSolver` `1v1` endgame certificates (`24` states), and historical human-vs-bot ladder logs (`184` decided `puniu3` games in `logs_20260804.zip`, `51` matched games in `logs/decision-log.jsonl`).

---

## 1. Quantitative Baseline & Bottleneck Map

### 1.1 Current HEAD (`omniscient_audit --corrected-product`, `9,000` iters, `24` games/tier)

- **vs `T2_Rules_Bayesian_Oracle_BR` (public info + determinized policy oracle + coupled BR)**:
  - Bot (`ProtocolAgent`, compiled preview): **15W–9L** (`Red_WR = 37.5%`).
  - Still shows clear self-inflicted blunders: `18/38` suboptimal forced switches across T2+T5 (`8` in T2 and `10` in T5, losing `> 0.1%` EV by picking `switch 3` over `switch 2`), `65/483` (`13.5%`) normal turns deviating from max-visit action in T2 (`121/844` = `14.3%` in T5), and `6` moves wasted into opponent type immunities in T2.
- **vs `T5_Omniscient_God_BR` (full state + live-policy oracle + `BoundSolver`)**:
  - Bot: **7W–17L** (`Red_WR = 70.83% ± 7.43%`, `Gain = +20.83%`, `Turns = 33.4`, `Mix = 0/0/0`).
  - Consistent with existing production-profile snapshots in [`data/dominance-audit-v13-verified/omniscient-audit-prod27k.json`](../data/dominance-audit-v13-verified/omniscient-audit-prod27k.json) (`T2 = 55.56%`, `T5 = 72.22%`, `T6 = 88.89%`) and [`data/dominance-audit-v13-verified/omniscient-audit-prod81k.json`](../data/dominance-audit-v13-verified/omniscient-audit-prod81k.json) (`T2 = 54.17%`, `T2a_3x_Search = 41.67%`).
- **`1v1` Last-Mon Endgame Exactness (`BoundSolver`, `400` iters base, `24` states)**:
  - `AvgWidth = 0.6814`; `6/24` states tightly closed (`value_width <= 0.10`): `Blind400_TightMid = 4.90%`, `Skuct3200_TightMid = 4.83%`, `Blind400+Mix_TightMid = 10.54%`.
  - On the `4` states closed to `value_width = 0.0` (`idx 2, 7, 11, 23`), `Blind400` achieved **`0.00%` exact loss**. Thus `1v1` terminal play is mostly solid; primary Elo headroom lies in **root action selection bugs, midgame switch/setup handling, and opponent-response modeling** before reaching `1v1`.

### 1.2 Human-vs-Bot Ladder Corpus (`puniu3`, `logs_20260804.zip` + `logs/decision-log.jsonl`)

Across **184 decided `puniu3` games** (`88W–96L`, `47.8%` WR):
- **Bot self-errors correlate with losses**:
  - Moves hitting opponent type immunity (`|-immune|`): **`35.4%` (`34/96`) of losses** vs `23.9%` (`21/88`) of wins.
  - Self move failures (`|-fail|`): **`27.1%` (`26/96`) of losses** vs `12.5%` (`11/88`) of wins.
- **Lowest bot win rates by opponent move used**:
  - `Baton Pass`: **25.0%** (`2W–6L`, `n=8`)
  - `Whirlwind`: **32.3%** (`10W–21L`, `n=31`)
  - `Sleep Talk`: **41.4%** (`12W–17L`, `n=29`)
  - `Curse`: **41.5%** (`27W–38L`, `n=65`)
  - `Swords Dance`: **41.7%** (`5W–7L`, `n=12`)
  - `Mean Look`: **42.9%** (`9W–12L`, `n=21`)
  - `Perish Song`: **43.8%** (`14W–18L`, `n=32`)

*(Historical defects such as missing `pokemon_left`, HP rounding mismatches, Sleep Clause attribution, and `MaskRules` rules for last-mon `perishsong`/`destinybond` or awake `sleeptalk` in `smmcts.rs` were implemented previously — see item #2 below for why several remain unexecuted in the live product path.)*

---

## 2. Prioritized Actionable Opportunities on Current HEAD

### Priority 0 — Critical Implementation Bugs

#### P0-1. Forced-Switch (`Request::Switch`) Uniform Visit Split & `switch 3` Tie-Break Bug
- **Evidence (HEAD traces)**: Out of **38** forced-switch decisions with `>= 2` legal switches and `iterations > 0`, **`18/38` (`47.4%`) picked `switch 3` when `switch 2` had a strictly higher search mean** (up to **-36.21% EV** loss), and `switch 2` was selected **`0/38` times**.
  - `T5` game-007 T5: chose `switch 3` (`4500v`, mean `0.3422`) over `switch 2` (`4500v`, mean `0.7043`, **-36.21%**)
  - `T2` game-022 T4: chose `switch 3` (`4500v`, mean `0.4530`) over `switch 2` (`4500v`, mean `0.6597`, **-20.67%**)
  - `T2` game-011 T5: chose `switch 3` (`4500v`, mean `0.4311`) over `switch 2` (`4500v`, mean `0.6239`, **-19.28%**)
  - `T2` game-002 T5: chose `switch 3` (`4500v`, mean `0.1863`) over `switch 2` (`4500v`, mean `0.3088`, **-12.24%**, lost game)
  - `T5` game-014 T4: chose `switch 3` (`4500v`, mean `0.1885`) over `switch 2` (`4500v`, mean `0.3074`, **-11.90%**, lost game)
- **Root Cause**:
  1. In [`crates/bot/src/blind.rs#L1479-L1503`](../crates/bot/src/blind.rs#L1479-L1503) (`BlindSearch::step_one_impl`), `min_floor_pick` iterates over `elig` and sums `stats.cells.get(row_start..row_start + replies)`. On a forced switch (`Request::Switch`), the opponent has `0` legal replies (`replies == 0`), so the slice is `0..0` and `w_n` is always `0`. Once `stats.samples` makes `world_floor > 0` (after `done >= 512`), `min_floor_pick` overrides `select_global_with_prior` on **every** iteration and allocates exact uniform round-robin visits (`4500 / 4500` at 9k).
  2. When selecting the final move, `bayesian_equilibrium()` skips types with `replies == 0` (`None`), and [`crates/bot/src/blind.rs#L2510`](../crates/bot/src/blind.rs#L2510) / [`crates/bot/src/smmcts.rs#L428`](../crates/bot/src/smmcts.rs#L428) / [`L777`](../crates/bot/src/smmcts.rs#L777) call `max_by_key` on equal visit counts (`4500 == 4500`), which in Rust returns the **last** maximum (`switch 3` > `switch 2`) without comparing `my_w[i] / my_n[i]`.
- **Fix Direction**:
  1. Gate `min_floor_pick` on `replies > 0` so forced switches use `select_global_with_prior` (UCB1).
  2. Break visit ties in `best()` and `EndgameRootStats` using mean reward (`my_w[i] / my_n[i]`) as secondary key (just like preview at [`blind.rs#L2504`](../crates/bot/src/blind.rs#L2504) and `import.rs#L2933`).

#### P0-2. Live `ProtocolAgent` / `BlindSearch::from_observation` Never Executes Root No-Op Pruning
- **Evidence (HEAD traces & ladder logs)**:
  - Across the `48` HEAD traces (where Blue runs [`ProtocolAgent`](../crates/bot/src/import.rs#L2518), matching WASM in [`crates/wasm/src/lib.rs#L891-L954`](../crates/wasm/src/lib.rs#L891-L954)):
    - **6 turns** failed `Perish Song` on Blue's last Pokemon (`T5` game-002 T84 spent `4,567` visits on `move perishsong`, triggering `|-fail|` with the Stadium 2 last-Pokemon hint).
    - **1 turn** failed `Rest` at full HP (`292/292`, `T5` game-017 T16).
    - In `T5` game-009 T13, an **awake** `Snorlax` (`163/292 None`) spent **`4,023 / 9,000` (`44.7%`)** root iterations exploring `move sleeptalk`.
  - Matches ladder room `3791` T10–T12 (`Gengar` last mon clicking `Destiny Bond` 3 turns straight).
- **Root Cause**:
  - [`crates/bot/src/smmcts.rs#L2205`](../crates/bot/src/smmcts.rs#L2205) already implements `perishsong | destinybond` last-mon pruning and `noop_reason`, **but**:
    - `BlindAgent::new` sets `root_pruning: false` ([`crates/bot/src/blind.rs#L173`](../crates/bot/src/blind.rs#L173));
    - `BlindSearch::from_observation_with_rng` passes `compute_dominated = false` and clears `my_dominated` in `use_public_exclusions` ([`crates/bot/src/blind.rs#L962-L985`](../crates/bot/src/blind.rs#L962-L985));
    - [`ProtocolAgent::on_request`](../crates/bot/src/import.rs#L2762-L2836) never calls `search.prune_dominated()`.
- **Important Caveat & Fix Direction**:
  - Per [`docs/SEARCH-AND-BELIEF-HANDOFF.md`](SEARCH-AND-BELIEF-HANDOFF.md), guessed-world exclusions (such as assuming opponent sleep duration or unseen switch locks) must **not** run blindly on `ObservedPosition`.
  - However, rules that depend strictly on **own-side or publicly certain state** — `b.sides[side].pokemon_left == 1` (`perishsong`, `destinybond`, `batonpass`), own `status != Slp` (`sleeptalk`), own `hp == maxhp && status == None` (`rest`/`recover`/etc. unless `MaskRules` says otherwise), and publicly empty foe bench (`roar`/`whirlwind`) — cannot misread hidden state and should be extracted into a `public_noop_reason` pass enabled in `ProtocolAgent` / `BlindSearch::from_observation` (then A/B tested via non-mirror `T2`/self-play gates).

---

### Priority 1 — Root Policy & Type-Immunity Exploitation

#### P1-1. `bayesian_equilibrium()` Sample-Poverty Bias Picks Lower-Mean / Low-Visit Actions
- **Evidence (HEAD traces)**:
  - In [`crates/bot/src/blind.rs#L2465-L2472`](../crates/bot/src/blind.rs#L2465-L2472), `best()` overrides UCB max-visits / `EndgameRootStats` with `bayesian_equilibrium()` on **every** non-preview turn with `done >= 64` whenever opponent has `>= 1` reply — even when no mixing passes the `+0.005` threshold and `policy[best_pure_action] = 1.0` ([`crates/bot/src/root_policy.rs#L575-L585`](../crates/bot/src/root_policy.rs#L575-L585)).
  - In HEAD traces, `blue_choice != max_visits` occurred on **`65/483` (`13.5%`)** T2 turns and **`121/844` (`14.3%`)** T5 turns. Excluding forced switches, `chosen != max_visits` occurred `168` times across T2+T5 combined; while `21` turns improved mean by `> 2%` (avoiding traps), **`21` turns lost `> 2%` mean EV** relative to the UCB favorite:
    - `T5` game-018 T18: vs `Cloyster(30/156)`, `switch 3` (`5665v`, mean `0.7924`) was overridden by `move flamethrower` (`2914v`, mean `0.6620`, **-13.05% EV**, lost game).
    - `T2` game-006 T10: `move doubleedge` (`7613v`, `0.8651`) overridden by `move curse` (`446v`, `0.7998`, **-6.52% EV**).
    - `T2` game-001 T2: `move thunderwave` (`4550v`, `0.6114`) overridden by `switch 3` (`1162v`, `0.5475`, **-6.39% EV**).
    - `T5` game-023 T2: `move sleeptalk` (`2883v`, `0.3053`) overridden by `switch 2` (`489v`, `0.2559`, **-4.94% EV**).
- **Root Cause**:
  - [`crates/bot/src/root_policy.rs#L349`](../crates/bot/src/root_policy.rs#L349) admits low-visit actions down to `min_own_v = (max_v / 32).clamp(4, 64)` (`0.7%` of a 9k budget), and [`L423-L448`](../crates/bot/src/root_policy.rs#L423-L448) shrinks each `(world, own, reply)` cell with `KAPPA_CELL = 64.0` toward `mu_row + col_blunder_bonus`.
  - Low-visit actions with only `1–5` samples per `(world, reply)` cell shrink upward toward the row prior and disguise tail losses, whereas high-visit UCB favorites accumulate enough samples to reveal bad replies — causing `pure_guarantees` to rank under-sampled actions ahead of the main line.
- **Fix Direction**:
  - Tighten `min_own_v` (e.g., proportional to `max_v`, such as `max_v / 8`) and/or add sample-count confidence penalties (LCB-style adjustment per cell or world) before replacing the UCB/Endgame favorite in pure-strategy regimes.

#### P1-2. Immunity Wastes (`Ghost` / `Ground`) & Endless Switching Loops
- **Evidence**:
  - **HEAD**: `T2` game-013 (`Gengar` threw `Shadow Ball` into sleeping `Snorlax` **5 times**: T6, T39, T52, T61, T64); `T2` game-001 T4 (`Starmie` poured `7,117` visits into `Thunder Wave` while `T2` BR predicted `Thunder Wave` and brought `Marowak` in free).
  - **Ladder (`decision-log.jsonl`)**:
    - Room `3746`: `Snorlax`↔`Cloyster` loop (`9` switches, `6` immunities vs `Misdreavus`); `T2` `Marowak` stayed in to `Rock Slide` (`74.3%`) into L55 `Zapdos` after `Hidden Power` was revealed on T1.
    - Room `3730`: `Zapdos`↔`Steelix`↔`Raichu` loop (`9` switches, `4` Electric hits into `Steelix`).
    - Room `3800` T83–T84: `Heracross` gave `Hidden Power [Fighting]` **`34.3%` visit share** against `Gengar` even on the turn immediately after it proved `|-immune|`.
    - [`4069.log`](../tmp/release-strength-v1/combined/2026-08-19%2013%3A30%3A52%20FreeDrink%20VS%20puniu3%20(Winner%3A%20FreeDrink)%20battle-gen2nintendocup2000noohkostadium2strict-4069.log) T1–T5: `Miltank` Cursed into `Mean Look + Perish Song Misdreavus` and repeatedly `Return`-ed into immunity until fainting.
- **Code Hooks**:
  - [`crates/bot/src/smmcts.rs#L1796-L1808`](../crates/bot/src/smmcts.rs#L1796-L1808) keeps `immunity_all_switchins: false` and `immunity_ignores_switch_read: false`, even though `immunity_all_switchins` is already implemented at [`L2347-L2354`](../crates/bot/src/smmcts.rs#L2347-L2354).
  - Testing `immunity_all_switchins` (plus public-only variant when the entire live/known opponent roster is immune, e.g., sleeping `Snorlax` when foe won't switch) and penalisations for low-entropy repeated switching under passive damage are direct candidates.

---

### Priority 2 — Midgame Rollouts, Opponent Modeling & Evaluation

#### P2-1. Greedy Max-Damage Rollout & Zeroed Leaf Weights Blind to Phazing / Setup / Traps
- **Code State**:
  - `RmConfig::default()` ([`crates/bot/src/smmcts.rs#L935-L943`](../crates/bot/src/smmcts.rs#L935-L943)) runs `Playout::heavy()` with **`rollout_status: false`, `rollout_switch: false`, `rollout_perish: false`**.
  - `EvalWeights::default()` ([`crates/bot/src/eval.rs#L186-L188`](../crates/bot/src/eval.rs#L186-L188)) keeps **`exchange: 0.0`, `confusion: 0.0`, `exchange_v2: false`**.
- **Symptom Match**:
  - Matches the steep ladder WR drops against **`Whirlwind` (32.3%)**, **`Baton Pass` (25.0%)**, **`Sleep Talk` (41.4%)**, **`Curse` (41.5%)**, **`Swords Dance` (41.7%)**, **`Mean Look` (42.9%)**, **`Perish Song` (43.8%)**, and concrete losses like `room 3781` T11–T13 (allowing `+3 Curse Heracross` while shuffling `Cloyster`↔`Snorlax`) and `room 3755` T7–T8 (`L55 Snorlax` staying in at `149/292` with `99.3%` visits on `Return`, `Q = 0.634`, immediately KO'd by revealed `Heracross` `Hidden Power`).
  - Also explains much of the `+20.8%..+22.2%` gap to `T5_Omniscient_God_BR`: `T5` uses coupled policy-oracle anticipation (`predicts` when Blue stays/clicks `Double-Edge`/`Thunder Wave` so it switches `Gengar`/`Marowak` in or triggers `Explosion` / counter-leads), whereas Blue's tree + greedy rollout underestimates opponent defensive switches, phazing loops, and setup races.

#### P2-2. Team Preview (`T0`) off-pool variance & compiled-table predictability
- In `omniscient_audit`, `T5` consistently counter-picked Blue's fixed compiled-preview lead/trio (e.g., bringing `Starmie` lead into `Marowak` lead in Pair 11 games `g022` T9 / `g023` T8).
- In off-pool ladder games (`logs/decision-log.jsonl` `T0`), searching `60` `(lead, bench1, bench2)` permutations at 10k iters spreads visits thinly (~150–1000 per trio, e.g., `room 3730` top-1 at `6.9%` / `0.284` vs `4.3%` / `0.275`).

---

## 3. Suggested Next Execution Sequence

1. **Patch & Gate P0-1 (`blind.rs` forced switch + `mean` tie-break) and P0-2 (public-certain `prune_dominated` in `ProtocolAgent`)**:
   - Note: [`crates/bot/src/`](../crates/bot/src) and [`crates/wasm/`](../crates/wasm) changes affect SHA-256 build fingerprints referenced in [`AGENTS.md`](../AGENTS.md); verify tests with `cargo t` first and rebake `data/compiled-preview-v1.json` only after confirming strength wins.
2. **Patch & Gate P1-1 (`root_policy.rs` `min_own_v` / shrinkage asymmetry vs UCB max-visits)** using non-mirror `omniscient_audit` (`T2_Rules_Bayesian_Oracle_BR`, `T5_Omniscient_God_BR`) rather than symmetric self-play alone.
3. **Investigate `room 3755` T7–T8 (`Hidden Power` Fighting vs `Snorlax`)** and `immunity_all_switchins` / lightweight switch & setup awareness in rollouts (`P1-2` / `P2-1`).
