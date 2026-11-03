# Bayesian Equilibrium Search & Kodoku Evolution Handoff

## 1. Completed in This Session

1. **Rust Search Repair (`849a3545`)**
   - [`crates/bot/src/smmcts.rs`](../crates/bot/src/smmcts.rs): Replaced raw mean backup at Depth-1 (`node_idx != 0`) with minimax contraction (`backed_up_value`, mixing 75% empirical mean with 25% smooth minimax value after 16 visits) so opponent exploratory errors do not propagate as optimistic cell payoffs.
   - [`crates/bot/src/root_policy.rs`](../crates/bot/src/root_policy.rs): Added `WorldPayoffInput` and `solve_bayesian_equilibrium` to solve $\max_{p \in \Delta} \sum_k w_k \min_{j \in \text{legal}_k} \sum_i p_i \hat{V}_k(i, j)$ with per-world opponent legal-action masking.
   - [`crates/bot/src/blind.rs`](../crates/bot/src/blind.rs): Added deterministic Low-Discrepancy Systematic Resampling (`active_world_indices`, $K = 48$ worlds) and wired `BlindSearch::best()` / `policy_distribution()` to `solve_bayesian_equilibrium`.

2. **JAX/TPU Port & Unified Verification Pipeline (`bc711b11`, `43c45e8b`, `d28f96b4`)**
   - [`tpu/src/gen2_tpu/kodoku.py`](../tpu/src/gen2_tpu/kodoku.py):
     - Implemented `solve_bayesian_equilibrium_jax`, updated `select_shell_belief_minimax_action_jax` with per-world opponent legal masking and Depth-1 contraction (`0.75 * ro.leaf_values + 0.25 * v_leaf`), and cached compiled batch execution in `_get_jit_shell_belief_minimax_batch_fn`.
     - Implemented `run_unified_nash_and_bake_pipeline_tpu` (`gen2-tpu nash-pipeline`) and ran it on Colab TPU `v6e1` (`data/tpu-nash-bake-v1/unified_summary.json`).
   - **Key Findings on Existing `team-pool-v2`**:
     - Against the pure "勝手読み" baseline (`opponent_uniform_row_mean_pure_argmax_katteyomi`), the coupled Bayesian Equilibrium policy won `72.22%` (`36` games); against the previous world-averaged RM+ / 81k bot (`data/dominance-audit-v13-verified/omniscient-audit-prod81k.json`), the true strength gain is `+4.2% ~ +5.6%` (`WR 54.2% ~ 55.6%`).
     - Testing `vc-kodoku-nash-a` as a 100% single-shell incumbent re-confirmed the 3-shell equilibrium (`vc-kodoku-nash-a` `50.4%`, `mjj-2010-garap-a` `31.5%`, `sample-08` `18.1%`, game value `0.5000`), while reducing `mjj-2010-garap-a`'s win rate vs `vc-kodoku-nash-a` from `62.08%` (old policy) down to `57.19%`.

---

## 2. Completed Follow-Up Tasks (Tasks A & B)

### Task A: Evolve Parties (Kodoku) Under the New Bayesian Equilibrium Policy on TPU (`DONE`)
1. **Two-Stage Bayesian Equilibrium Kodoku Evaluator & Evolutionary Loop ([`tpu/src/gen2_tpu/kodoku.py`](../tpu/src/gen2_tpu/kodoku.py), [`tpu/src/gen2_tpu/__init__.py`](../tpu/src/gen2_tpu/__init__.py))**:
   - Implemented `normalize_training_set` / `normalize_training_team` matching [`tools/normalize-training.js`](../tools/normalize-training.js) (`training-v3` DV/EV/happiness/gender/shiny/Hidden Power rules) and legal Gen 2 learnset mutation (`mutate_team_for_kodoku`, `load_learnsets`) with Item Clause, Species Clause, `Sleep Talk`+`Rest` co-requirement, and `preserve_shell` support.
   - Implemented `evaluate_population_tpu` (`KodokuPopulationEvaluation`) and `run_kodoku_tpu` (`KodokuEvolutionResult`, CLI: `gen2-tpu kodoku-evolve`):
     - **Stage 1**: Fast `evaluate_kodoku_gauntlet_tpu` + Bayesian election-call screen (`evaluate_shell_election_call_matrix_tpu`) to prune clearly weak mutations.
     - **Stage 2**: Coupled Bayesian Equilibrium blind search evaluation (`_run_policy_duel_batch_tpu` calling `simulate_shell_belief_minimax_games_tpu` with `bayesian_eq_a=True, bayesian_eq_b=True`) for top contenders, blended into the population payoff matrix and solved via `solve_coupled_shell_equilibrium_tpu` with `detect_hitchhiker_slots`.
2. **Full-Horizon Multi-Deme Evolution & 3-Gate Nash Pipeline ([`data/kodoku-beq-v1/`](../data/kodoku-beq-v1/))**:
   - Ran 3-generation Multi-Deme / Challenger evolution (`screen_turns=40, refine_turns=60, policy_turns=2, policy_followup_turns=36`) seeded with the 4 `vc-kodoku-nash-a` variants, `mjj-2010-garap-a`, `sample-08`, and top challengers (`data/kodoku-beq-v1/kodoku_evolution_summary.json`, `data/kodoku-beq-v1/unified_summary.json`, plus 48 baked `PairTable` JSONs).
   - Gate 1 (`NON_INFERIOR_CONFIRMED`): `mean = 0.513889` (`72` CRN games across 2 waves).
   - Gate 2: Re-confirmed the 3 equilibrium shells at full 40/60-turn horizon (`sample-08` shell via `sample-08-beq-g1-c2`, `vc-kodoku-nash-a` shell via `apex-c04-bug-3atkstar-boomgen`, and `mjj-2010-garap-a` shell).
   - Kept the deployed 33-party catalog (`data/team-pool-v2/team-pool.json`) and 411-party prior (`data/belief-pool-v3/belief-pool.json`) frozen so all `training-v3` exhaustive adversarial certificates (`data/training-v3/verification.json`) remain intact.

### Task B: Full Production Preview Rebake (`data/compiled-preview-v1.json`) (`DONE`)
1. **Full Scratch Rebake Against the 411-Party Deployed Prior**:
   - Rebuilt [`crates/wasm/pkg-web`](../crates/wasm/pkg-web) and [`crates/wasm/pkg-node`](../crates/wasm/pkg-node) for the updated Rust search (`849a3545`).
   - Ran `cargo run --release -p nc2000-bot --example meta_eco -- bake-prior-preview` (`1,081` unordered type-pair matrices across `48` prior entries / `46` distinct types in `30` public rosters, solving all `990` policies) and activated via [`tools/activate-preview-bake.py`](../tools/activate-preview-bake.py) and [`tools/build-training-products.js`](../tools/build-training-products.js).
   - Refreshed [`data/compiled-preview-v1.json`](../data/compiled-preview-v1.json), [`data/team-pool-v2/preview-nash-matrix.json`](../data/team-pool-v2/preview-nash-matrix.json), [`data/preview-training-v3/basis.json`](../data/preview-training-v3/basis.json), [`data/preview-training-v3/bake-manifest.json`](../data/preview-training-v3/bake-manifest.json), and [`data/belief-pool-v3/known-expansion.json`](../data/belief-pool-v3/known-expansion.json).
2. **Production & Coverage Verification**:
   - All `990 / 990` policies (`33` own parties $\times$ `30` public rosters) are now prior-compatible (`invalidated: 0`) with the deployed 411-party `data/belief-pool-v3/belief-pool.json` (`71f34e05...`), and all source `inputs` SHA256 hashes (`blind.rs`, `root_policy.rs`, `smmcts.rs`) and `build` fingerprint (`fnv1a64:3912b518cf820a3a:m17e-solver-build-v3`) match current HEAD.
   - `npm run test:preview-production` ([`data/preview-training-v3/production-verification.json`](../data/preview-training-v3/production-verification.json)): `2,178 / 2,178` side cases served from `table` (`0` live fallbacks), median lookup `8.0 ms` (`p95 = 9.0 ms`), round-trip `p95 = 30.8 ms`.
   - `node tests/compiled-preview-coverage.mjs` ([`data/preview-training-v3/compiled-coverage.json`](../data/preview-training-v3/compiled-coverage.json)): `1,089` matchups, `990` unique keys (`990` valid, `0` invalid), `69,696` slot checks all legal.
