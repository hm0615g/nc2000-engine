# Bot Strength & Battle-Log Audit Handoff

> **Status**: Partially implemented through [`40673df2`](../crates/bot/src/blind.rs) (code changes landed in `b164009a`, `ab2c1e50`, and `6e2c2ac5`; compiled preview tables re-baked and SDK manifest synced in `40673df2`).
>
> - **Landed**: **P0-1** (forced-switch visit allocation & mean tie-break), **P0-2** (public/own-certain root no-op pruning in `ProtocolAgent` / `BlindSearch`), **P1-1** (`MIN_MIX_MARGIN` equilibrium override gate & `SafeWin` protection, keeping conservative cell shrinkage), and **P1-2 first half** (`MaskRules::default().immunity_all_switchins = true` & `tactical_desperation_bonus` guards).
> - **Remaining**: **P1-1** optional `min_own_v` / LCB refinement, **P1-2 second half** (root-cause diagnosis of progressively losing switch loops), **P2-1** (midgame search/value-sign diagnosis for phazing/setup/traps — noting that parked `RmConfig` rollout flags and `EvalWeights::exchange_scheme` were previously measured NULL/below seed-noise floor), and **P2-2** (Team Preview `T0` predictability / off-pool 60-permutation dilution).
>
> **Scope**: Tracks historical log issues and live bottlenecks from the `42cf9f69` baseline (`omniscient_audit`, `BoundSolver` `1v1` certificates, and `184` `puniu3` ladder games in `logs_20260804.zip` + `51` games in `logs/decision-log.jsonl`) through the post-fix verification runs on `6e2c2ac5` / `40673df2`.

---

## 1. Quantitative Baseline & Post-Fix Verification

### 1.1 `omniscient_audit --corrected-product` (`9,000` iters)

| Tier | Pre-Fix Baseline (`42cf9f69`, `24` games/tier) | Post-Fix (`6e2c2ac5` / `40673df2`, `96` games/tier) |
| :--- | :--- | :--- |
| **`T2_Rules_Bayesian_Oracle_BR`** (public info + determinized oracle + coupled BR) | Bot **15W–9L** (`Red_WR = 37.5%`) | Bot **47W–49L** (`Red_WR = 51.04% ± 4.34%`, `Blue_WR = 48.96%`, `Gain = +1.04%`, `Turns = 22.7`) |
| **`T5_Omniscient_God_BR`** (full state + live-policy oracle + `BoundSolver`) | Bot **7W–17L** (`Red_WR = 70.83% ± 7.43%`, `Gain = +20.83%`, `Turns = 33.4`) | Bot **54W–42L** (`Red_WR = 43.75% ± 5.29%`, `Blue_WR = 56.25%`, `Gain = -6.25%`, `Turns = 30.8`) |

- **Additional post-fix production duel check** (`6e2c2ac5`, `9,000` base / `729,000` preview iters, `320` games vs `Raikou3-shell`): Bot (`Blue`) scored **`64.69%`** (`Red_WR = 35.31%`, 95% CI `[29.64%, 40.98%]`).
- **`1v1` Last-Mon Endgame Exactness (`BoundSolver`, `24` states)**:
  - On `42cf9f69` (`AvgWidth = 0.6814`, `6/24` tightly closed with `value_width <= 0.10`), `Blind400_TightMid = 4.90%` and `Skuct3200_TightMid = 4.83%` (`0.00%` exact loss on the `4` states closed to `0.0`). On the `6e2c2ac5` `96`-game audit run (`AvgWidth = 0.7892`, `1/24` tight state `<= 0.10`), `Blind400_TightMid = 4.49%` matched `Skuct3200_TightMid = 4.49%`. Terminal `1v1` play remains strong; remaining headroom sits in **midgame rollouts/evaluation, switching loops, and Team Preview (`T0`)**.

### 1.2 Human-vs-Bot Ladder Corpus (`puniu3`, `logs_20260804.zip` + `logs/decision-log.jsonl`)

Across **184 decided `puniu3` games** (`88W–96L`, `47.8%` WR):
- **Bot self-errors correlate with losses**:
  - Moves hitting opponent type immunity (`|-immune|`): **`35.4%` (`34/96`) of losses** vs `23.9%` (`21/88`) of wins.
  - Self move failures (`|-fail|`): **`27.1%` (`26/96`) of losses** vs `12.5%` (`11/88`) of wins.
- **Lowest bot win rates by opponent move used** (still unaddressed by rollout/eval defaults in **P2-1**):
  - `Baton Pass`: **25.0%** (`2W–6L`, `n=8`)
  - `Whirlwind`: **32.3%** (`10W–21L`, `n=31`)
  - `Sleep Talk`: **41.4%** (`12W–17L`, `n=29`)
  - `Curse`: **41.5%** (`27W–38L`, `n=65`)
  - `Swords Dance`: **41.7%** (`5W–7L`, `n=12`)
  - `Mean Look`: **42.9%** (`9W–12L`, `n=21`)
  - `Perish Song`: **43.8%** (`14W–18L`, `n=32`)

---

## 2. Status of Audit Items

### Priority 0 — Critical Implementation Bugs (Landed)

#### P0-1. Forced-Switch (`Request::Switch`) Uniform Visit Split & `switch 3` Tie-Break Bug — **LANDED (`b164009a`)**
- **Original Defect**: On `42cf9f69`, `18/38` (`47.4%`) forced switches picked `switch 3` over a strictly higher-mean `switch 2` (`0/38` selections for `switch 2`, up to `-36.21%` EV loss) because `min_floor_pick` overrode UCB1 whenever `replies == 0` (`w_n == 0 < world_floor`) and `max_by_key` broke equal-visit ties toward the last element (`switch 3`).
- **Resolution**:
  - [`crates/bot/src/blind.rs#L1494-L1498`](../crates/bot/src/blind.rs#L1494-L1498) gates `min_floor_pick` on `self.root_types[type_index].replies > 0` so forced switches fall through to `select_global_with_prior` (UCB1).
  - [`crates/bot/src/blind.rs#L2496`](../crates/bot/src/blind.rs#L2496) and [`crates/bot/src/smmcts.rs#L227`](../crates/bot/src/smmcts.rs#L227) (`cmp_visits_then_mean`) break visit ties by mean reward (`my_w / my_n`).

#### P0-2. Live `ProtocolAgent` / `BlindSearch::from_observation` Root No-Op Pruning — **LANDED (`b164009a`, `ab2c1e50`)**
- **Original Defect**: `BlindAgent::new` disabled `root_pruning`, `BlindSearch::from_observation_with_rng` cleared `my_dominated`, and `ProtocolAgent::on_request` never called `prune_dominated()`, allowing last-mon `Perish Song` / `Destiny Bond` and repeated dead moves in production.
- **Resolution**:
  - [`crates/bot/src/smmcts.rs#L2250-L2343`](../crates/bot/src/smmcts.rs#L2250-L2343) introduces `public_noop_reason` / `raw_public_noop_reason`, restricting live exclusions strictly to turn-order-independent public/own-certain state: own screens/Safeguard/Mist already up, phazing with an empty foe bench, `perishsong`/`destinybond` from the last mon, `batonpass` with an empty bench (`rules.extended_noops`), foe Safeguard blocking status/non-Swagger confusion, and `immunity_all_switchins` (respecting `wait_exempt` and `foe_may_change_type`).
  - [`crates/bot/src/blind.rs#L971-L995`](../crates/bot/src/blind.rs#L971-L995) populates `my_dominated` via `public_noop_reason` in `use_public_exclusions`, and [`crates/bot/src/import.rs#L2832`](../crates/bot/src/import.rs#L2832) calls `search.prune_dominated()`.
- **Intentional Scope Boundary**: Turn-order-dependent rules (`sleeptalk`/`snore` while awake, `rest`/`recover` at full HP) remain in [`noop_reason`](../crates/bot/src/smmcts.rs#L2345-L2470) gated on `faster_than_foe`, **not** in `public_noop_reason`, because a faster foe can inflict sleep or deal damage before our move resolves.

---

### Priority 1 — Root Policy & Type-Immunity Exploitation (Partially Landed)

#### P1-1. `bayesian_equilibrium()` Override Bias & Shrinkage — **PARTIALLY LANDED (`ab2c1e50`, adjusted in `6e2c2ac5`)**
- **Landed**:
  - [`crates/bot/src/blind.rs#L2472-L2512`](../crates/bot/src/blind.rs#L2472-L2512) protects `EndgameRegime::SafeWin` alongside `FastWin` / `ComebackLoss`, computes `base_idx` from `endgame.select_best` or `cmp_visits_then_mean`, and only overrides with `bayesian_equilibrium()` when `eq.equilibrium_value > eq.pure_guarantees[base_idx] + MIN_MIX_MARGIN` (`0.02`, [`root_policy.rs#L3`](../crates/bot/src/root_policy.rs#L3)).
  - `6e2c2ac5` intentionally restored conservative Bayesian cell shrinkage in [`crates/bot/src/root_policy.rs#L423-L448`](../crates/bot/src/root_policy.rs#L423-L448) (`KAPPA_ROW = 16.0`, `KAPPA_CELL = 64.0`, non-negative `col_blunder_bonus`) after `ab2c1e50`'s symmetric `col_delta` / `KAPPA_CELL = 16.0` proved overly punitive on sparse exploratory cells, and fixed `ImperfectInfoCore` telemetry indexing in [`blind.rs#L247-L250`](../crates/bot/src/blind.rs#L247-L250).
- **Remaining (Optional Tuning)**:
  - [`crates/bot/src/root_policy.rs#L349`](../crates/bot/src/root_policy.rs#L349) still admits candidate actions down to `min_own_v = (max_v / 32).clamp(4, 64)`. Tightening `min_own_v` or adding an LCB sample-count penalty has not been added and is only worth pursuing if post-`6e2c2ac5` traces still show low-visit pure overrides losing EV.

#### P1-2. Immunity Wastes (`Ghost` / `Ground`) & Endless Switching Loops — **HALF LANDED (`ab2c1e50`) / HALF REMAINING**
- **Landed (Type-Immunity Mask & Desperation Tie-Breaker)**:
  - [`crates/bot/src/smmcts.rs#L1913-L1924`](../crates/bot/src/smmcts.rs#L1913-L1924) promotes `MaskRules::default().immunity_all_switchins` to `true`, so `public_noop_reason` prunes moves whose type cannot hit the active foe or any live/possible switch-in candidate (with `foe_may_change_type` guards for `Conversion`/`Conversion 2`/`Sketch`/transformed `Hidden Power` and `wait_exempt` guards).
  - [`crates/bot/src/smmcts.rs#L895-L1023`](../crates/bot/src/smmcts.rs#L895-L1023) updates `tactical_desperation_bonus` (`foe_status_or_volatile_blocked` and Spikes-aware switch scoring) so losing roots stop rewarding blocked status moves or lethal switch-ins.
- **Remaining (Root-Cause Diagnosis of Progressively Losing Switch Loops)**:
  - Concrete traces still exhibit repeated switching loops under passive damage (`Spikes`/status) or setup (`Snorlax`↔`Cloyster`, `Zapdos`↔`Steelix`↔`Raichu`). Do **not** add an ad-hoc `if-then` loop-penalty patch, which risks breaking legitimate cycle/pivot play; instead, inspect post-`6e2c2ac5` traces to diagnose *why* search evaluates repeated switches as higher or flat Q-value relative to staying in. Note also that `MaskRules::immunity_ignores_switch_read` remains `false` by design so single-immunity read targets are not blindly masked.

---

### Priority 2 — Midgame Search, Opponent Modeling & Team Preview (Remaining)

#### P2-1. Midgame Phazing / Setup / Trap Weaknesses (Parked Rollout/Eval Flags Already Measured Null) — **REMAINING (Diagnosis Needed)**
- **Current Code State & Historical Null Findings**:
  - `RmConfig::default()` ([`crates/bot/src/smmcts.rs#L1039-L1066`](../crates/bot/src/smmcts.rs#L1039-L1066)) enables `rollout_combo: true` (which already implies `perish_escape` in [`crates/bot/src/mcts.rs#L470`](../crates/bot/src/mcts.rs#L470)) and `rollout_baton_pass: true`, while keeping `rollout_status: false`, `rollout_switch: false`, and `rollout_perish: false`. Those M16c rollout rules (`rollout_status`, `rollout_switch`) are **intentionally off** because full-pool CRN re-runs measured them as strength **NULL** (`0.493 ± 0.023` for `rollout_status`, `0.466 ± 0.032` for `rollout_switch`, `0.505 ± 0.019` combined): leaf eval already prices boosts and status, so rollout heuristics only add variance unless the rollout gets the value **sign** wrong (as `perish_combo` fixed).
  - Likewise, `EvalWeights::default()` ([`crates/bot/src/eval.rs#L166-L190`](../crates/bot/src/eval.rs#L166-L190)) keeps `exchange: 0.0`, `confusion: 0.0`, `exchange_v2: false` **intentionally off** because candidate screening (`eval-candidate-screen`) showed even `EvalWeights::exchange_scheme` ([`eval.rs#L252`](../crates/bot/src/eval.rs#L252)) moved top-1 choices at `1.00x` of the single-seed flip floor (`0.1542` vs `0.1547`), meaning additive/matrix leaf-eval tweaks alone stay buried under search seed noise.
- **What Remains**:
  - Ladder WR drops against `Baton Pass` (`25.0%`), `Whirlwind` (`32.3%`), `Sleep Talk` (`41.4%`), `Curse` (`41.5%`), `Swords Dance` (`41.7%`), `Mean Look` (`42.9%`), and `Perish Song` (`43.8%`), plus staying in vs revealed threats (`room 3755`), reflect midgame **search / opponent-response / horizon or value-sign** gaps rather than unflipped `rollout_status`/`exchange_v2` switches. Any fix should come from concrete trace diagnosis of where search fails to separate or price those continuations, not re-enabling the parked M16c/M17 flags.

#### P2-2. Team Preview (`T0`) Off-Pool Variance & Compiled-Table Predictability — **REMAINING**
- **Current Code State**:
  - Compiled preview tables (`data/compiled-preview-v1.json`) remain deterministic per seed/matchup key (`choices[u32(seed)%64]`), which an adaptive opponent (`T5`) or informed human can counter-lead.
  - For off-pool rosters falling back to live `T0` search, evaluating all `60` `(lead, bench1, bench2)` permutations at ~10k iterations still spreads visits thinly (~150–1,000 visits per trio).

---

## 3. Remaining Execution Sequence

1. **Trace Audit on Post-`6e2c2ac5` `omniscient_audit` (`T2`/`T5`) & Ladder Cases**:
   - Inspect concrete turns where search still picks progressively losing switch loops (**P1-2**), fails to answer setup/phazing/trap continuations (**P2-1**), or lets low-visit actions override UCB via `bayesian_equilibrium` (**P1-1** `min_own_v`). Focus on structural search/value-sign causes rather than ad-hoc `if-then` patches or re-enabling the parked `rollout_status`/`rollout_switch`/`exchange_v2` flags.
2. **Address P2-2 (`T0` Off-Pool Pruning / Mixing)** if `T0` off-pool logs or challenger counter-leads show measurable loss.
3. **Build & Lock Discipline**: Any edit to `crates/bot/src/` or `crates/wasm/` updates `M17E_SOLVER_BUILD_FINGERPRINT` and requires `cargo t`, WASM/web tests, and re-baking `data/compiled-preview-v1.json` + `engine-manifest.json` per [`AGENTS.md`](../AGENTS.md).
