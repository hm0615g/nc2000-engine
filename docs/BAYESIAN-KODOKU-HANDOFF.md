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

---

## 2. Methodological Audit & Production Pipeline Alignment (`1. nash計算 -> 2. パーティプール適正化 -> 3. 焼き込み`)

Following the 5 methodological questions, the entire post-Kodoku pipeline was re-executed in strict dependency order under production rules:

### Step 1: Production Nash Calculation (`nash計算`) — Multi-Variant Camouflaged Shells at `81k / 729k`
- **Evaluator**: [`crates/bot/examples/omniscient_audit.rs`](../crates/bot/examples/omniscient_audit.rs) running full production settings (`ProtocolAgent` + `PlayerChannel`, `c = 0.4`, `iters = 81,000` per turn, `preview_iters = 729,000` at team preview, `max_turns = 250`, Known-Pool disclosed priors, Bayesian equilibrium root solver + $K = 48$ systematic world resampling + Depth-1 minimax contraction).
- **Incumbent**: `APEX4` (`vc-kodoku-nash-a` 4-variant mixture: `apex-c08-surgical-142` `.30`, `red-v10-s1105-r2-s0142` `.25`, `apex-c04-bug-3atkstar-boomgen` `.25`, `red-v10-s1105-r1-s0148` `.20`).
- **Multi-Variant Camouflaged Challenger Shell Results** ([`data/kodoku-beq-v1/production-nash-verification.json`](../data/kodoku-beq-v1/production-nash-verification.json)):
  - `Sample07-beq-4var-shell` (`vc02` 4v: `sample-07`, `sample-07-beq-g1-c3`, `sample-07-beq-g1-c4`, `sample-07-beq-g2-c4`): **53.12%** ($\pm 4.29\%$ SE, 95% CI `[44.71%, 61.54%]`, $z = +0.73$, 68W-60L-0T across `64` paired blocks = **128 games**). Does not statistically exceed 50%.
  - `Police3-shell` (`vc06` 3v: `sample-08`, `hc75-3rd-bean`, `rental-cban-6`): **45.83%** ($\pm 7.92\%$ SE, $z = -0.53$, 22W-26L-0T across `24` pairs = **48 games**).
  - `Raikou3-shell` (`vc16` 3v: `mjj-2020-rraikou`, `deme-r1-03-s0054`, `deme-r3-b1-s0018`): **43.75%** ($\pm 5.48\%$ SE, $z = -1.14$, 21W-27L-0T across `24` pairs = **48 games**).
  - `APEX5-shell` (`vc-kodoku-nash-a` 5v adding `red-v10-s1105-r2-s0142-beq-g1-c5-beq-g3-c2`): **41.67%** ($\pm 7.77\%$ SE, $z = -1.07$, 20W-28L-0T across `24` pairs = **48 games**; the TPU-mutated 5th variant dropped Swords Dance on Marowak for double Fire moves, weakening the shell).
  - `mjj-2022-nrsk` (1v): **50.00%** ($\pm 13.36\%$ SE, $z = 0.00$, 8W-8L-0T across `8` pairs = **16 games**; historical 64g: `40.62%`).
  - `Sample12-5var-shell` (`vc11` 5v): **43.75%** ($\pm 14.75\%$ SE, $z = -0.42$, 7W-9L-0T across `8` pairs = **16 games**; historical 64g: `35.94%`).
  - `Garap2-shell` (`vc15` 2v: `mjj-2010-garap-a`, `sib-r2-n3-mjj-2010-garap-a-s06`): **37.50%** ($\pm 12.50\%$ SE, $z = -1.00$, 6W-10L-0T across `8` pairs = **16 games**; early-stopped `< 40%`, historical 64g: `39.06%`).
  - `Deme30-beq-5var-shell` (`vc-kodoku-evo-ttar-raikou` 5v): **31.25%** ($\pm 13.15\%$ SE, $z = -1.43$, 5W-11L-0T across `8` pairs = **16 games**; early-stopped `< 40%`).
- **Conclusion**: Under full production search settings (`81k / 729k`) and multi-variant camouflage, the 3-shell convergence observed under the TPU short-horizon proxy (`policy_turns=2, followup=36`) was an artifact of the proxy horizon. No multi-variant camouflaged challenger shell statistically beats `APEX4`, confirming the 1-shell 4-variant `APEX4` Nash equilibrium in [`data/kodoku-r3/conclusion-equilibrium-deme.json`](../data/kodoku-r3/conclusion-equilibrium-deme.json).

### Step 2: Party Pool Optimization (`パーティプール適正化`)
- Restored [`data/team-pool-v2/team-pool.json`](../data/team-pool-v2/team-pool.json) and [`data/belief-pool-v3/belief-pool.json`](../data/belief-pool-v3/belief-pool.json) to the strict **"Nash shell (4 camouflaged variants) + Tier 2 with meaningful diversity (`size: 1` per non-Nash cluster)"** policy: **33 catalog parties across 30 distinct public rosters** (`411` frozen prior entries, `sets=3174 classes=575 exact_catalog_support=33 issues=0`).

### Step 3: Tiered Production / Approximation Preview Bake (`焼き込み`)
- Updated [`crates/bot/examples/meta_eco/catalog_preview.rs`](../crates/bot/examples/meta_eco/catalog_preview.rs) (`bake-catalog-preview`) to enforce the tiered policy:
  - **Nash-involving keys (`c.apex || s.apex`, `149` unique keys)**: Evaluated with full production preview search (`LocalPlayer::new` + `ObservedPosition` + `BlindSearch::from_observation`, `729,000` iterations, `c = 0.4`, `prune_preview_aliases`, `Belief` over `belief-pool-v3`).
  - **Non-Nash keys (`!c.apex && !s.apex`, `841` unique keys)**: Evaluated via the `903` non-Nash rollout matrices (`ns_other = 2`, `250` turns) and debiased Bayesian RM+ (`600` sweeps) with ordinary-party near-optimal diversity (`casual`).
- Activated all `990` policies (`149` production + `841` rollout approximation) into [`data/compiled-preview-v1.json`](../data/compiled-preview-v1.json) and [`data/team-pool-v2/preview-nash-matrix.json`](../data/team-pool-v2/preview-nash-matrix.json).
- **Production & Coverage Verification**:
  - `npm run test:preview-production` ([`data/preview-training-v3/production-verification.json`](../data/preview-training-v3/production-verification.json)): `2,178 / 2,178` side cases served from `table` (`0` live fallbacks), median lookup `7.2 ms` (`p95 = 7.8 ms`), round-trip `p95 = 28.1 ms`.
  - `node tests/compiled-preview-coverage.mjs` ([`data/preview-training-v3/compiled-coverage.json`](../data/preview-training-v3/compiled-coverage.json)): `1,089` matchups, `990` unique keys (`990` valid, `0` invalid), `69,696` slot checks all legal.
  - `npm run test:e2e`: `70 / 70` Playwright E2E tests passed.
