# Bot Strength, Midgame Continuation & Battle-Log Audit Handoff

> **Status**: Synced to `d1ea4f9a` (`crates/bot` root masks, exact Bayesian column-generation LP with LCB override gate, flat tie-break unification, and `81,000` / `729,000` live search budgets).
>
> - **Fully Landed**:
>   - **P0-1**: Forced-switch (`Request::Switch`) UCB1 visit allocation & mean tie-break ([`crates/bot/src/blind.rs`](../crates/bot/src/blind.rs), [`crates/bot/src/endgame_root.rs`](../crates/bot/src/endgame_root.rs)).
>   - **P0-2**: Unified public/own-certain root dominance & no-op pruning across all observation entrypoints ([`crates/bot/src/mask.rs`](../crates/bot/src/mask.rs), [`crates/bot/tests/suite/mask_oracle.rs`](../crates/bot/tests/suite/mask_oracle.rs)), including public Self-KO and operational Sleep-Clause forfeit masks (`b9a53e4b`).
>   - **P1-1**: Exact column-generation Bayesian root LP with `min_own_v = (max_v / 8).max(4).min(max_v)`, conservative cell shrinkage, `override_value` LCB gating (`MIN_MIX_MARGIN = 0.02`), and `SafeWin` / `FastWin` / `ComebackLoss` protection ([`crates/bot/src/root_policy.rs`](../crates/bot/src/root_policy.rs)).
>   - **P1-2 (Mask & Tie-Break Half)**: `MaskRules::default().immunity_all_switchins = true` ([`crates/bot/src/mask.rs`](../crates/bot/src/mask.rs)) and Spikes/status-aware `tactical_desperation_bonus` ([`crates/bot/src/endgame_root.rs`](../crates/bot/src/endgame_root.rs)).
>   - **Production Budgets (`data/live-search.json`)**: Live Web play runs fixed `battle.strong = 81,000` iterations in battle and `unknownPreview = 729,000` iterations on uncached Team Preview (`web/tests/strength.spec.ts`).
> - **Diagnosed Remaining Structural Bottlenecks (at `81,000` iterations)**:
>   1. **Phazing valuation blindness in `n0 + n1 >= 4` rollouts & leaves** (`Curse`/`Swords Dance` stacking into known `Whirlwind`/`Roar`, and defenders undervaluing phazing when dragged-in counters punish slugfest rollouts).
>   2. **Awake `Sleep Talk` / full-HP `Rest` passivity vs evasion/stall** (learnset-blind awake `Sleep Talk` admissibility + recoil-vs-heal-PP horizon distortion when `w.pp = 0.2` pools all 12 move slots equally).
>   3. **Multi-turn `Baton Pass` + `Double Team` / `Substitute` chain horizon** (non-boosting 8-turn rollouts underprice compounding setup before a pass).
>   4. **Flying↔Grounded Spikes static accounting inversion & leaf-frontier counter-switch bias** (`w.spikes = 1.5 > w.hp = 1.0` on grounded bench slots only + `rollout_switch = false` slugfest lock).

---

## 1. Quantitative Baselines & What Already Resolved on `HEAD`

### 1.1 `omniscient_audit --corrected-product` (`9,000` iters) & `BoundSolver` `1v1` Audit

| Tier | Pre-Fix Baseline (`42cf9f69`, `24` games/tier) | Post-Fix (`6e2c2ac5`+, `96` games/tier) |
| :--- | :--- | :--- |
| **`T2_Rules_Bayesian_Oracle_BR`** (public info + determinized oracle + coupled BR) | Bot **15W–9L** (`Red_WR = 37.5%`) | Bot **47W–49L** (`Red_WR = 51.04% ± 4.34%`, `Blue_WR = 48.96%`, `Gain = +1.04%`) |
| **`T5_Omniscient_God_BR`** (full state + live-policy oracle + `BoundSolver`) | Bot **7W–17L** (`Red_WR = 70.83% ± 7.43%`) | Bot **54W–42L** (`Red_WR = 43.75% ± 5.29%`, `Blue_WR = 56.25%`, `Gain = -6.25%`) |

- **Production duel check** (`9,000` base / `729,000` preview iters, `320` games vs `Raikou3-shell`): Bot (`Blue`) scored **`64.69%`** (`Red_WR = 35.31%`, 95% CI `[29.64%, 40.98%]`).
- **Terminal `1v1` exactness (`BoundSolver`, `24` states)**: `Blind400_TightMid = 4.49%` matches `Skuct3200_TightMid = 4.49%` (`0.00%` loss on closed states).

### 1.2 Historical Issues Eliminated on Current `HEAD` at `81,000` Iterations

Replaying the historical endurance (`findings/deployed-endurance-v1/games/g4.json`) and August 2026 ladder traces (`logs/decision-log.jsonl`, originally recorded at `10,000` iterations before `linear.rs`, `BlindSpotLeaf`, `mask.rs`, and `root_policy.rs`) through current `ProtocolSearcher` at **`81,000` iterations** shows two previously reported failure classes are **already resolved**:

1. **Endurance mirror full-HP `Soft-Boiled` (`g4` Turn 53)**:
   - On `2026-10-17`, full-HP `Blissey` (`361/361`) chose `move softboiled` even at `729,000` iterations.
   - On current `HEAD` (`d1ea4f9a`), `g4` Turn 53 selects **`move thunder` (`67.7%–68.1%` visit share, `mean = 0.581`)** over `move softboiled` (`8.7%–11.0%`, `mean = 0.570`) across seeds at both `81k` and `243k`, driven by `linear::compact_rollout_linear_pick` (Bellman stage-game policy never heals at `me_hp == maxhp`) and `n0 + n1 <= 3` Schur-Bellman/Toeplitz valuation.
2. **Prediction-driven Spikes & Perish switch loops (`room 3793`, `room 3795`)**:
   - In `room 3793` (`Starmie` ↔ `Marowak` bleeding out to `Cloyster` `Spikes` + `Raikou`), **4 of 6 voluntary switches (`d8, d12, d16, d17`) flip to staying in (`move thunderwave` / `move earthquake`)** at `81k`. Notably, on `d16` raw UCB visits still favor `switch 2` (`48%` visits, optimistic mean `0.374`) because `switch 2` scores `0.521` vs `Thunderbolt` (`n = 25,617`) but `0.087` vs `Hidden Power` (`n = 13,252`); `solve_bayesian_equilibrium` (`root_policy.rs`) catches the `0.087` best-reply vulnerability and overrides to `move thunderwave`.
   - In `room 3795` (`Snorlax` ↔ `Misdreavus`), the loop turns `d14` and `d16` flip from `switch 3` to `move perishsong` and `move curse`.
   - Conversely, high-switch wins such as `room 3728` (`11` switches, Q-value `0.71–0.89`, `Starmie` ↔ `Articuno` mirroring `Blissey` ↔ `Machamp` with no `Spikes`) are legitimate double-counter pivots, confirming why blanket loop penalties must never be added.

---

## 2. Remaining Midgame Continuation Bottlenecks on `HEAD` (`81,000` Iterations)

> **Why previously parked levers stay off**:
> - `RmConfig::rollout_status` (`0.493 ± 0.023`), `rollout_switch` (`0.466 ± 0.032`), and `EvalWeights::exchange_scheme` (`1.00x` single-seed flip floor) all measured **NULL** or weakly negative in CRN duels.
> - Enabling `LeafSettlementMode::BlindSpotBoth` (`compact_blindspot_tactical_override` deterministic `R1/R2a/R2b/R3b` rollout overrides in [`crates/bot/src/mcts.rs`](../crates/bot/src/mcts.rs)) **loses `-10.0%` (`0.450` vs `0.550` across `160` paired games: `-0.100 ± 0.129` in both `defense` and `offense` on `setup_gauntlet`)** against shipped `BlindSpotLeaf`, because hard deterministic `if-then` overrides inside fixed 8-turn rollouts over-fire and corrupt rollout expectations.

Any future midgame improvement must address the four concrete structural mechanisms below without introducing rigid deterministic rollout overrides:

### 2.1 Phazing Valuation Blindness When `n0 + n1 >= 4` (`Curse` / `SD` vs `Whirlwind` / `Roar`)
- **81k Witness (`room 3809`, `d24` Turn 21 & `d26` Turn 23 — Zero Belief Error)**:
  - `Belief` has converged to the exact singleton pool team `mjj-2024-raista` (`fallback: false`), so the bot knows active `Skarmory` carries `["drillpeck", "curse", "whirlwind", "rest"]` and bench `Raikou` carries `Roar`.
  - Despite this, mono-Normal `Snorlax` (`Double-Edge / Curse / Rest / Sleep Talk`) allocates **`100%` of `81,000` visits (`74,777` in cell `(Curse, Curse)`, `mean = 0.847`)** to `move curse` (`move doubleedge`: `38` visits, `mean = 0.377`), and the searched 4-ply principal variation is both sides clicking `Curse` 4 turns in a row.
- **Exact Structural Mechanism**:
  1. [`linear::compact_linear_diff`](../crates/bot/src/linear.rs#L2872-L2955) only runs when `n0 + n1 <= 3` (`1v1`, `2v1`, `1v2`). On `3v3`, `3v2`, `2v3`, and `2v2` (`n0 + n1 >= 4`), leaf evaluation falls back strictly to additive [`eval::compact_side_score`](../crates/bot/src/eval.rs#L1051-L1143).
  2. In [`mcts::compact_greedy_pick`](../crates/bot/src/mcts.rs#L914-L1017), `Whirlwind`/`Roar` deals `0` direct damage (`raw_score = 0.0`) and `rollout_switch` is `false`.
  3. When the tree explores `Skarmory` clicking `Whirlwind` (`cell [0][2]`, `n = 1,972`), `Whirlwind` drags in `Tentacruel` (`Surf`), and at the tree leaf the 8-turn slugfest rollout **locks `Skarmory` on the field (`rollout_switch = false`) against `Tentacruel` until `Skarmory` faints**, scoring `Whirlwind` at `0.880` for `Snorlax` (`0.120` for `Skarmory` — worse than letting `Snorlax` boost!).
  4. Along `(Curse, Curse)` (`cell [0][0]`), neither mon ever phazes during the 8-turn rollout, leaving `Snorlax` with inflated `+Atk/+Def` stage bonuses at the horizon cutoff (`0.844`). This simultaneously causes offensive `Curse` users to over-stack into known phazers and defensive phazers to under-use `Whirlwind`/`Roar` whenever an unfavorable drag-in exists on the opponent's bench.

### 2.2 Awake `Sleep Talk` Mask Gap & Recoil-vs-PP Passivity (`room 3800`, `d44` & `d46`)
- **81k Witness (`room 3800`, `d44` Turn 43 & `d46` Turn 45)**:
  - Awake, full-HP `Snorlax` (`292/292`) facing Baton-Passed `+6 Evasion` `Starmie` (`166/166`, revealed `Thunder Wave / Psychic / Recover / Hydro Pump`) selects **`move sleeptalk` (`71%` visits, `mean = 0.502`)** on `d44` and **`move rest` (`88%` visits, `mean = 0.556`)** on `d46` over `move doubleedge` (`0.484` / `0.512`), burning PP on guaranteed `|-fail|` turns.
- **Exact Structural Mechanism (Two Parts)**:
  1. **Public mask gap ([`crates/bot/src/mask.rs`](../crates/bot/src/mask.rs))**: `public_noop_reason` withholds awake `sleeptalk`/`snore` pruning whenever the active foe might move first, even when (a) all 4 moves of the active foe are publicly revealed with no sleep move, (b) the active foe species cannot legally learn any sleep move in `data/learnsets-gen2.json` (and cannot `Metronome`/`Sleep Talk`/`Mirror Move` into one) while bench switches cannot inflict sleep on the turn of switching, or (c) we already have another living sleeping bench Pokémon (so any foe-inflicted sleep immediately forfeits the game in our favor under Operational Sleep Clause).
  2. **Pooled PP weight vs recoil/chip over an 8-turn horizon ([`crates/bot/src/eval.rs#L1100-L1107`](../crates/bot/src/eval.rs#L1100-L1107))**: `compact_side_score` pools all 12 move slots across 3 Pokémon into `w.pp * (sum_pp / sum_max_pp)` (`w.pp = 0.2`, `sum_max_pp ≈ 200`), valuing `1` PP of `Recover`/`Soft-Boiled` at `~0.001` score (`< 0.04%` win prob). When `Snorlax` uses `Double-Edge` into `+6 Evasion` `Starmie`, hits inflict `1/4` recoil on `Snorlax` while `Starmie` immediately heals back to `100%` via Bellman rollout (`compact_rollout_linear_pick`), so at the 8-turn cutoff **deliberately failing (`Sleep Talk` while awake / `Rest` at full HP) preserves `100%` own HP and outscores forcing recovery PP burn** (`solve_schur_bellman_2d` in `linear.rs` likewise assumes infinite PP with `GAMMA = 0.985`).

### 2.3 Multi-Turn `Baton Pass` + Evasion/Boost Chain Horizon (`room 3786`, `d3`–`d11`)
- **81k Witness (`room 3786`, `d3`–`d11`)**:
  - `Blissey` (`Present / Ice Beam / Soft-Boiled / Heal Bell`) stays in clicking `Present` (`51%–72%` visits) against `Jolteon` (`Substitute + Double Team + Baton Pass`) until `Jolteon` passes `+6 Evasion + Substitute` to `Swords Dance` `Marowak`.
- **Exact Structural Mechanism**:
  - In the searched principal variation at `d4` (`Turn 3`), the opponent tree node prefers immediate `move batonpass` (`0.390` for `Blissey`) or single-step `Double Team` (`0.192`), after which the 8-turn rollout never clicks `Double Team` or `Swords Dance` (`rollout_status = false`). Because multi-turn evasion/setup compounding stops at the tree leaf, switching early to `Snorlax`/`Cloyster` (`0.154–0.211`) shows no horizon advantage over staying in with `Blissey` until `Marowak` has already received the pass and boosted.

### 2.4 Flying↔Grounded Spikes Static Inversion & Leaf-Frontier Switch Bias
- **Exact Structural Mechanism ([`crates/bot/src/eval.rs#L1075-L1115`](../crates/bot/src/eval.rs#L1075-L1115))**:
  1. `compact_side_score` penalizes grounded **bench** Pokémon by `-w.spikes * (layers_frac) = -1.5 * 0.125 = -0.1875` each (`spikes_exposed` excludes `active_slot`), while `w.hp = 1.0`. Switching an active **Flying** Pokémon (`Zapdos`/`Skarmory`) out for a **Grounded** bench Pokémon (`Snorlax`/`Steelix`/`Marowak`) under 1 layer of `Spikes` costs `-0.125` HP (`+0.0625` Leftovers) but reduces `spikes_exposed` by `1` (`+0.1875`), yielding an artificial **`+0.0625` (`+0.125` with Leftovers) static leaf-eval profit for stepping onto Spikes** (and `-0.1875` extra static penalty when switching Grounded → Flying).
  2. Because `rollout_switch = false` locks both actives into an 8-turn slugfest starting at the tree leaf, whichever player lands the **last counter-switch right before the tree leaf** traps the countered foe in place for 8 turns, inflating switch Q-values near the tree horizon when `n0 + n1 >= 4`.

---

## 3. Build, Verification & Lock Discipline

- **Source & artifact locks**: Any edit to `crates/{engine,conformance,bot}/src/**/*.rs` changes `M17E_SOLVER_BUILD_FINGERPRINT` (`crates/bot/build.rs`) and invalidates `data/compiled-preview-v1.json` (`web/vite.config.ts`) and `engine-manifest.json` (`tools/bundle-core-sdk.js`).
- **Screening discipline**: Screen any candidate midgame fix first on concrete position flip rates vs the `81k` seed-noise floor (`tools/diagnose-position.js`, `setup_gauntlet`, `combo_gauntlet`) and paired CRN duels (`arena`) before re-baking `data/compiled-preview-v1.json`.
