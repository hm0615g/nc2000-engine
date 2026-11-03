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

## 2. Next Tasks (In Order)

### Task A: Evolve Parties (Kodoku) Under the New Bayesian Equilibrium Policy on TPU
Currently, the evolutionary loop in [`tpu/src/gen2_tpu/kodoku.py`](../tpu/src/gen2_tpu/kodoku.py) (`run_kodoku_tpu` / `evaluate_population_tpu`) evaluates candidate movesets/spreads using fast heuristic rollouts (`simulate_packed_batch`) or legacy world-averaged evaluators. Evolving parties before rebaking is required so the final preview bake does not become immediately obsolete.

1. **Wire New Policy into Kodoku Fitness Evaluation**:
   - In [`tpu/src/gen2_tpu/kodoku.py`](../tpu/src/gen2_tpu/kodoku.py), integrate `simulate_shell_belief_minimax_games_tpu` (with `bayesian_eq_a=True, bayesian_eq_b=True`) into the top-candidate / survivor evaluation stage of `run_kodoku_tpu` (two-stage screening: fast rollout / election-call pruning for obvious weak mutations, followed by `simulate_shell_belief_minimax_games_tpu` for surviving contenders and Nash support updates).
2. **Run Multi-Deme / Challenger Evolution on Colab TPU (`v6e1`)**:
   - Seed with `data/team-pool-v2/team-pool.json` (especially the 3 equilibrium shells: `vc-kodoku-nash-a`, `mjj-2010-garap-a`, `sample-08`, plus top challenger archetypes).
   - Evolve movesets/items/spreads under the new policy where opponent worlds best-respond without "勝手読み" exploitation bias.
3. **Update Catalog & Belief Pool**:
   - Package the newly evolved/verified Nash and challenger parties into `data/team-pool-v2/team-pool.json` (and `data/belief-pool-v3/belief-pool.json` if prior expansion changes).

### Task B: Full Production Preview Rebake (`data/compiled-preview-v1.json`)
Once the evolved party pool and Nash support are finalized in Task A:
1. Rebake the full catalog preview tables (`data/compiled-preview-v1.json` and `data/team-pool-v2/preview-nash-matrix.json`) covering all `own_parties x public_rosters` (`990` policies) and activate via [`tools/activate-preview-bake.py`](../tools/activate-preview-bake.py).
2. Note that `data/compiled-preview-v1.json` currently has stale `inputs` SHA256 hashes for `crates/bot/src/blind.rs`, `crates/bot/src/root_policy.rs`, and `crates/bot/src/smmcts.rs` (checked by [`web/vite.config.ts`](../web/vite.config.ts)); the final rebake in Task B will refresh `inputs` and `build` fingerprint simultaneously.
