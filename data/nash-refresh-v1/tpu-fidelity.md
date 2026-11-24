# Fidelity of the archived TPU evaluator

The November 14 JAX transition kernel reproduced the current Rust compact simulator on every sampled transition. The November 4 version used to generate the historical Kodoku population diverged on 45 of 50 fresh trajectories. Even the November 14 evaluator changes the information available to the players and assigns materially different values to some endgames. Use the later kernel to propose mutations. Measure their payoffs with the current player-channel bot before fitting the disclosed-mixture game.

## Revisions and samples

All comparisons ran locally on CPU using the archived JAX implementation and the November 14 lockfile. The Rust reference was `423b2327`. The November 4 source was the revision that committed the evolution results, `b0a3846f`. The last revision containing the JAX source was `25ed6f33`. These tests concern implementation behavior. They do not measure accelerator throughput or estimate how often these cases occur in ladder play. [Evolution data](../kodoku-beq-v1/kodoku_evolution_summary.json) · [Archived JAX source](https://github.com/puniu3/nc2000-engine/tree/25ed6f33/tpu/src/gen2_tpu)

- The archival fixture contains five matchup scenarios with four games each, 20 initial states and 466 actions. Its original parity test passed again on the November 14 code. We also independently generated all 486 Rust states with the current engine and the stored action sequences. The old test checks a material/HP leaf score and race margin. It does not compare the production search leaf or the bot's choices. [Archived test](https://github.com/puniu3/nc2000-engine/blob/25ed6f33/tpu/tests/test_tpu_gen2.py#L547)
- The new sample uses both complete rosters and the two actual preview choices from each of the 50 disclosed-pool games, in file order. Each trajectory starts with a fresh battle seed of `41,918,2026,index+1` (zero-based file index). Both simulators receive the same legal actions selected from the Rust reference. The seeded sampler favors moves over switches, and uses both forced and voluntary switches. All 50 trajectories reached a terminal state, for 1,296 transitions. The live bot chose the trios in this sample. Its historical battle actions and outcomes are separate from these synthetic trajectories. [Source games](disclosed/games.jsonl)

## Transitions and legal actions

| Comparison against current Rust | November 4 JAX | November 14 JAX |
| --- | ---: | ---: |
| Archived trajectories agreeing through the last recorded state | 6/20 | 20/20 |
| Fresh trajectories agreeing through the last recorded state | 5/50 | 50/50 |

The trajectory test starts each implementation from its own initial state. It checks main and Substitute HP, PP, major status and counter, items, boosts, active party indices, alive masks, requests, outcome, turn, weather, screens, Safeguard, PRNG and both players' legal actions before accepting each prefix. For the November 4 version we stopped each trajectory at its first disagreement, so an illegal continuation cannot inflate the count. Of the 45 differing fresh trajectories, 37 already differed in HP at that first disagreement and seven differed in legal actions. These counts overlap.

For the November 14 kernel we additionally applied every action independently from the corresponding current Rust state and compared all 27 shared JAX state fields, including volatile flags, timers, slot references and the complete RNG. All 466 archival and 1,296 fresh transitions matched exactly. Both players' legal lists matched on all 3,524 pre-action checks. Engine-internal caches and static fingerprints were outside this comparison.

The full Rust battle engine uses additional mechanics. In a one-turn control, a level-55 Jolteon used Pin Missile into a level-55 Snorlax using Curse. Over 64 seeds JAX and Rust compact agreed on Snorlax's remaining HP 64 times. Full Rust agreed 18 times. For seed `76,11,19,2`, full Rust left 264 HP and both compact implementations left 271. Full Rust samples the hit count and processes the hits. Compact scales one damage roll by the mean count. [Full battle damage](../../crates/engine/src/battle/moveexec.rs#L2044) · [Compact battle damage](../../crates/engine/src/compact.rs#L2994)

## Leaf values, picks and private information

On the 1,296 fresh pre-action states, the November 14 `eval_compact_leaf` differed from current Rust `compact_eval_leaf_blindspot` by more than `1e-5` in 442 cases. The mean absolute score difference was 0.04255 on the 0–1 scale. The scores fell on opposite sides of 0.5 in 39 cases, excluding numerical differences within the tolerance. On turn 17 of fresh trajectory 36, both sides had only Snorlax left. APEX C04 had 120 HP and Sample07 g1-c4 had 119 HP. JAX gave APEX 0.05218 and Rust gave it 0.93481. These are leaf evaluations, not empirical win probabilities. Current Rust includes a low-piece endgame solver. [Current evaluation](../../crates/bot/src/eval.rs#L795) · [Endgame calculation](../../crates/bot/src/linear.rs#L2892)

The historical evolution pipeline used two distinct selection methods. Its first stage screens legal trios and refines a small payoff submatrix. For the second-stage policy games, `evaluate_population_tpu` takes the first legal trio in roster enumeration for each team. Compared with the 100 decisions actually made by the current live preview in the disclosed sample, that rule matched the lead and trio 0 times and the unordered trio once. Deme's representative trio was Tyranitar / Skarmory / Marowak. Live Deme never selected those three together in its 30 games. These counts refer specifically to the second-stage evolution games. [First and second stage in the archived evolution code](https://github.com/puniu3/nc2000-engine/blob/25ed6f33/tpu/src/gen2_tpu/kodoku.py#L5431) · [Actual representative picks](https://github.com/puniu3/nc2000-engine/blob/25ed6f33/tpu/src/gen2_tpu/kodoku.py#L5534) · [Live preview records](disclosed/games.jsonl)

In those games the archived driver inserts the opponent's actual selected trio into every hypothesized variant table. It weights the distinct variant indices equally. For a singleton public shell in a larger population it can also add up to three other teams by species overlap. This changes both the hidden bench and the published prior, including Deme's singleton case. Its battle policy evaluates a one-ply joint-action grid across those tables and uses short continuations. The current bot reconstructs hidden worlds from its player channel and uses the published weights, a 729k live preview and an adaptive 243k battle cap in the disclosed experiment. [Archived variant construction and duel inputs](https://github.com/puniu3/nc2000-engine/blob/25ed6f33/tpu/src/gen2_tpu/kodoku.py#L3746) · [Current disclosed protocol](disclosed/README.md)

A first-turn counterexample isolates the effect of a private selection. Deme always picked Tyranitar / Snorlax / Suicune, with Tyranitar leading against Sample07's Cloyster. Sample07's bench was Electrode plus either Tauros or Snorlax. With all four Sample07 set variants weighted equally and the same random seeds in both runs, the JAX policy chose **Rock Slide** against the Tauros bench and **Crunch** against the Snorlax bench. The current WASM player channel produced byte-identical Deme frames in preview and on turn one. Those two worlds therefore require the same action distribution given the same information. The archived selector changed the action even with its private random draw held fixed. [Runnable counterexample](tpu-hidden-bench.py)

To reproduce the counterexample from the repository root, build the Node WASM package if needed and run the following. The archived program finds its dex relative to the extracted source.

```sh
audit_dir=$(mktemp -d)
git archive 25ed6f33 tpu data/gen2stadium2.json | tar -x -C "$audit_dir"
PYTHONPATH="$audit_dir/tpu/src" uv run --frozen --project "$audit_dir/tpu" python data/nash-refresh-v1/tpu-hidden-bench.py
```
