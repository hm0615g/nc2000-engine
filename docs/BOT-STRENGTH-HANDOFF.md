# Bot Strength, Search & Belief Reference

Authoritative snapshot of `crates/bot` playing-strength mechanisms, measured-null dead ends, and open structural frontiers on `HEAD`. Consult live code under [`crates/bot/src/`](../crates/bot/src/) rather than gitignored scratch archives (`findings/`, `tmp/`, `.git/sota/`).

---

## 1. Shipped Search, Root, Endgame & Belief Stack

### 1.1 Budgets & Profiles
- **Live Web budgets ([`data/live-search.json`](../data/live-search.json))**: Fixed `battle.strong = 81,000` iterations per battle decision and `unknownPreview = 729,000` iterations when Team Preview misses [`data/compiled-preview-v1.json`](../data/compiled-preview-v1.json).
- **Profile baseline ([`data/search-profiles.json`](../data/search-profiles.json))**: `blind` UCB `c = 0.4` (`27,000` profile baseline used by `?evaluate`, `?fork`, and offline probes; `open` is retired except historical kifu/fork replay).

### 1.2 Imperfect-Information Reconstruction & Belief ([`belief.rs`](../crates/bot/src/belief.rs), [`observe.rs`](../crates/bot/src/observe.rs), [`hp_evidence.rs`](../crates/bot/src/hp_evidence.rs))
- **Player observation boundary**: Web workers, `ProtocolAgent`, `BlindAgent`, and `BlindSearcher` reconstruct roots strictly through [`ObservedPosition`](../crates/bot/src/position.rs) / [`LocalPlayer`](../crates/bot/src/player.rs) with symmetric hidden runtime timers ([`player_state.rs`](../crates/bot/src/player_state.rs)), `|teamsize|` opponent pick-count tracking ([`import.rs`](../crates/bot/src/import.rs)), live Hidden Power effectiveness narrowing, and deductive passive-item exclusions.
- **Strict pool trust until contradiction**: While `Belief::alive` is non-empty over [`data/belief-pool-v3/belief-pool.json`](../data/belief-pool-v3/belief-pool.json) (`471` entries), candidate filtering and Turn 0 Team Preview remain exact.
- **Post-contradiction off-pool synthesis**: Once the known pool is contradicted, `Belief` activates near-pool soft shell repair, quantile-coupled `FallbackCandPool` synthesis, bundled zero-prior Gen 2 species coverage ([`belief_supplement_v1.json`](../crates/bot/src/belief_supplement_v1.json)), and low-Speed / special low-Attack EV/IV spread synthesis validated against all revealed moves, items, and public HP buckets (preventing impossible-HP leaks), using an independent party-sampling RNG stream.
- **Opt-in priors ([`prior.rs`](../crates/bot/src/prior.rs), [`pick_prior.rs`](../crates/bot/src/pick_prior.rs))**: `BeliefPrior` (carry marginals for moves/items/leads, applied once in `FallbackCandPool` without `LoadoutDraw` double-counting) and `PickPrior` (3-of-6 team preview frequencies) are opt-in and inert by default; see [`docs/community-belief-prior-design.md`](community-belief-prior-design.md).
- **World subsampling**: `BlindSearch` defaults to `MAX_SUBSAMPLED_WORLDS = 24` with bounded warmup ([`blind.rs`](../crates/bot/src/blind.rs)).

### 1.3 Public Root Masks & Root Decision Policy ([`mask.rs`](../crates/bot/src/mask.rs), [`root_policy.rs`](../crates/bot/src/root_policy.rs), [`endgame_root.rs`](../crates/bot/src/endgame_root.rs))
- **Turn-order-independent public root pruning**: `BlindSearch::use_public_exclusions` masks only actions proven useless or self-forfeiting from public and own-certain state (never from guessed opponent Speed, Quick Claw, bench sets, or HP rolls): public Self-KO and operational Sleep-Clause forfeits, awake `Sleep Talk`/`Snore` (vs sleep-less foe learnset or statused/Safeguard/Sub/curing-berry self), frozen `Sleep Talk`/`Snore`, safe full-HP `Rest`/heals, weak/already-up `Substitute`, weak/capped `Belly Drum`, capped self-boosts, already-up `Spikes`, and `MaskRules::default().immunity_all_switchins = true`. Intentional stall/wait turns (`destiny_bond`, `perishsong == 1`, residual/recovery waits, `encore`) are preserved via `wait_exempt`.
- **Root selection & Bayesian equilibrium override**:
  - Forced switches (`Request::Switch`) use UCB1 visit allocation and mean tie-breaking; flat `(visits, mean)` ties break deterministically toward `Move` before `Switch` across all entrypoints.
  - Exact column-generation Bayesian root LP ([`root_policy.rs`](../crates/bot/src/root_policy.rs)) runs over eligible actions (`min_own_v = (max_v / 8).max(4).min(max_v)`) with conservative cell shrinkage, `override_value` LCB gating (`MIN_MIX_MARGIN = 0.02`), and `SafeWin` / `FastWin` / `ComebackLoss` + Spikes/status-aware `tactical_desperation_bonus` ([`endgame_root.rs`](../crates/bot/src/endgame_root.rs)).

### 1.4 Rollout, Endgame Linear Valuation & Certified Solver ([`mcts.rs`](../crates/bot/src/mcts.rs), [`linear.rs`](../crates/bot/src/linear.rs), [`bounds.rs`](../crates/bot/src/bounds.rs))
- **Continuation (`mcts.rs`, `eval.rs`)**: 8-turn `CompactBattle` heavy playout with `RmConfig::rollout_combo = true` (Perish trap/escape/phaze rules) and `LeafSettlementMode::BlindSpotLeaf`.
- **Low-piece stage-game valuation (`linear.rs`)**: When `n0 + n1 <= 3`, `compact_linear_diff` / `compact_rollout_linear_pick` apply Schur-Bellman / Toeplitz stage-game valuation (eliminating full-HP recovery passivity in 1v1–2v1 endgames).
- **Certified exact/interval solver (`exact.rs`, `bounds.rs`, `stall.rs`)**: Integer-damage-quotiented lazy interval solver (`BoundSolver`) with pointwise matrix dominance pruning and `TwoSidedHeal` resource-DAG scheduling (`?solver` door and `endgame_exactness_corpus` / `anchor_gate`).

---

## 2. Measured-Null & Rejected Directions (Do Not Repeat)

| Candidate / Lever | Measured Result | Why It Fails / Stays Off |
| :--- | :--- | :--- |
| **`RmConfig::rollout_status` / `rollout_switch` (`m16c`)** | `0.493 ± 0.023` (status, `1,600` CRN) / `0.466 ± 0.032` (switch, `800` CRN); combined `0.505 ± 0.019` (`2,400` CRN, [`data/m16c-rollout-v2/README.md`](../data/m16c-rollout-v2/README.md)) | Leaf `eval.rs` already prices boosts/status so tree lookahead recovers setup value; heuristic rollout switches surrender tempo. |
| **`LeafSettlementMode::BlindSpotBoth`** (`R1/R2a/R2b/R3b` rollout overrides) | **`-10.0%`** (`0.450` vs `0.550` on `setup_gauntlet`, `160` paired games) vs `BlindSpotLeaf` | Hard deterministic `if-then` overrides inside fixed 8-turn rollouts over-fire and corrupt rollout expectations. |
| **`O(0.1)` static `eval.rs` balance-sheet tweaks** (`exchange_scheme`, `spikes_no_arbitrage`, `phaze_boost_writedown`) | `1.00x` single-seed flip floor; `spikes_no_arbitrage` `0.5071 ± 0.030` (`840` CRN @ `3k`) & `0.4875 ± 0.0537` (`240` CRN @ `9k`); `phaze_boost_writedown` `0.5000` (`240W–240L`, `480` CRN @ `3k`+`9k`) | At `3k`–`81k` (`c = 0.4`), tree + 8-turn rollout + `linear.rs` + root Bayesian LP already resolve decisive branches; small static leaf shifts only perturb near-tied roots at seed noise. |
| **Blanket switch-loop penalties** | Breaks `0.71–0.89` Q-value zero-Spikes double-counter pivots | Prediction-bleed switch loops at `10k` were already resolved by `81k` depth + Bayesian root LP (`root_policy.rs`) catching worst-case prediction vulnerability. |
| **Raising `MAX_SUBSAMPLED_WORLDS > 24`** (`world_budget_probe`) | `K = 4..192` flip rates `<=` seed noise (`0.23–0.33`) at `27k`/`81k`; `K = 64` vs `24` off-pool duel `0.5208 ± 0.0712` (`96` games) | `24` quantile-coupled worlds already saturate root action resolution under current `FallbackCandPool` diversity. |
| **Retired search/value architectures** | Residual NN / TPU (`tpu/`, `bot::value`, `fvi_*`), 30-tree independent ensemble, frozen-tree root re-eval | Dual-engine drift / per-iteration latency overhead, or loss of deep single-tree continuation resolution. |
| **Rejected micro-optimizations** | WASM `simd128` / manual `u64x2` `WordHasher`, `BellmanCache` inner SIMD, tail-batched `hash_state_impl`, per-field `Battle` dirty bitmasks | Failed the 5% median/p95 gate in [`web/tests/portable-perf.mjs`](../web/tests/portable-perf.mjs) (`BellmanCache` already hits on most leaves) or risked `ENGINE_FINGERPRINT` desync. |

---

## 3. Open Structural Frontiers (at `81,000` Iterations)

1. **Multi-turn `Baton Pass` + evasion/boost chain horizon**:
   - When an opponent chains `Substitute` + `Double Team` / `Swords Dance` + `Baton Pass` (e.g., `Jolteon` passing to `Marowak`), `rollout_status = false` halts opponent setup compounding at the tree leaf, so early counter-switches or phazers can look no better than staying in until the pass completes.
2. **Long-horizon recovery-PP exhaustion vs pooled `w.pp`**:
   - [`eval::compact_side_score`](../crates/bot/src/eval.rs) pools all 12 move slots across 3 Pokémon into `w.pp * (sum_pp / sum_max_pp)` (`w.pp = 0.2`, `~0.001` per recovery PP), and `solve_schur_bellman_2d` in [`linear.rs`](../crates/bot/src/linear.rs) assumes infinite PP (`GAMMA = 0.985`). Relevant only in deep stall/evasion games where tree lookahead cannot reach recovery depletion.
3. **Off-pool cold-start belief calibration**:
   - Improving early-turn move/item/spread joint priors after known-pool contradiction before all 4 moves are revealed. Always evaluate against `OpenAgent` oracle controls and held-out human rosters via [`crates/bot/examples/belief_probe.rs`](../crates/bot/examples/belief_probe.rs) — never rely on `BlindAgent`-vs-`BlindAgent` self-play alone (`offpool_fallback_gauntlet`) or raw spectator-reveal counts (which under-count passive/reactive moves) or hard Gen 2 damage-roll / speed-tie exclusions.
4. **`n0 + n1 >= 4` leaf-frontier switch lock**:
   - `linear.rs` activates only when `n0 + n1 <= 3`, while `rollout_switch = false` locks both actives for 8 rollout turns after the tree leaf (and `eval.rs` prices `w.spikes = 1.5` on grounded bench slots only). Any candidate targeting `4+`-alive horizon valuation must clear the concrete-position seed-noise screen before CRN duels; naive static leaf adjustments already measured `0.500`.

---

## 4. Screening, Profiling & Lock Discipline

- **Strength screening order**:
  1. Inspect concrete boards and measure top-1 flip rate vs the single-seed noise floor at `81k` (`node tools/diagnose-position.js`, `cargo run --release -p nc2000-bot --example setup_gauntlet`, `combo_gauntlet`, `belief_probe`).
  2. Run paired CRN duels (`cargo run --release -p nc2000-bot --example arena -- ... --crn-seeds`) on `team-pool-v2` / `belief-pool-v3` (`>= 3,000–9,000` iters) before touching compiled preview tables.
- **Performance profiling (always measure current `HEAD`)**:
  - Native throughput: `cargo run --release -p nc2000-bot --example perf_probe`
  - Multi-turn live hotspot flamegraph: `cargo run --release -p nc2000-bot --example profile_blind -- <RECORDED_GAMES> tmp/blind.svg` (avoid single-turn `profile_mcts`, which measures cold allocator setup)
  - Portable WASM/browser gate: `node web/tests/portable-perf.mjs`
- **Artifact & hash locks**: Editing `crates/{engine,conformance,bot}/src/**/*.rs` updates `ENGINE_FINGERPRINT` / `M17E_SOLVER_BUILD_FINGERPRINT` and invalidates `data/compiled-preview-v1.json` and `engine-manifest.json`; follow [`AGENTS.md`](../AGENTS.md) §2.
