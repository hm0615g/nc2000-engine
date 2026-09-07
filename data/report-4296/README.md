# Battle 4296: OpenSheet decision audit

The final reference comparison supports Earthquake at T11, Earthquake or switching at T24, and Thunderbolt at T26. It does not support the blanket T18 claim. The separate OpenSheet continuation sample points in the same direction for the supported comparisons, but T11 and T24 are not independently established at its smaller sample size. T24's Earthquake result also changes sign under the 6-HP correction; switching is the more consistent candidate. No tested configuration establishes an improvement without regressions, so the production policy is unchanged.

The [T11 follow-up](T11.md) records a withdrawn 30-tree candidate and its interrupted regression experiment. A later [trace of the unchanged search](t11-cause/README.md) follows a concrete source of Hidden Power's optimistic score: more opponent exploration in its fragmented continuation, with exploratory mistakes backed up as root reward. The [fixed-policy experiment](t11-frozen/README.md) and [OpenSheet c=0.4 experiment](t11-c04/README.md) did not establish an adoptable correction. The full causal contribution remains unresolved; no corrective policy has been applied. [Deferred algorithm research](#deferred-algorithm-research) below preserves the subsequent literature review, implementation constraints and effort estimate for a later session.

The reporter supplied the opponent's complete six sets and confirmed OpenSheet mode. `opponent-submitted.json` preserves the submitted stats, moves, items and abilities; genders are filled from the replay and Gen2 DVs. `opponent-team.json` is the local Gen2 validator's canonical result. Only the ability and nature fields change. The EVs and IVs remain as submitted.

The own sheet is inferred from meta-pool index 13 (`sample-23`). Its preview, revealed moves and items agree with the replay. The original private request and original executable/seed are unavailable. Consequently this is a reconstruction of the reported positions, not a byte-for-byte reproduction of the original client session.

The replay contains spectator HP percentages. In particular, the importer writes 7/166 HP for Marowak at T24, whereas the poison sequence implies 6/166 under the inferred own set. The one-step regression check covers both values; a separate 64-seed reproduction with 6 HP chooses Hidden Power 55 times, Swords Dance 4, Rock Slide 2, switch 2 and Earthquake 1. The reported Swords Dance therefore recurs with either reconstruction. Starmie's 13% at T26 allows 22 or 23 HP; the midpoint importer uses 23. The one-step tests verify Thunderbolt's higher KO probability at both values. All three selected Pokémon on both sides have appeared by the investigated turns; the unselected three are not introduced into the continuations. The player names in `battle.raw.log` are anonymized.

Blind-mode runs and runs using guessed opponent sets are excluded from this report.

## Current policy reproduction

OpenSheet, the shared open profile (`c=1`, 30,000 iterations), seeds 61001–61064. Each decision starts a fresh `ProtocolAgent` with the actual opponent sheet pinned. The own sheet override avoids the corpus importer's otherwise incorrect Marowak completion.

| Turn | Reported action | Current choices, 64 seeds |
|---|---|---|
| 11 | Ampharos switch | Hidden Power 64 |
| 18 | Hidden Power | Earthquake 49, Rock Slide 12, Hidden Power 3 |
| 24 | Swords Dance | Hidden Power 56, Ampharos switch 3, Swords Dance 3, Earthquake 2 |
| 26 | Fire Punch | Thunderbolt 64 |

`reproduction.jsonl` contains every seed's chosen action, root visits and search rewards. These rewards are not terminal win rates. A zero count over 64 seeds does not prove that an action is impossible.

## Counterfactual method

Only our first action is forced. The opponent chooses its simultaneous first response by an independent search on the common initial state; its response is not selected after seeing our forced action. Each action uses the same trial's battle, own-agent and opponent-agent seeds. Both sides then choose freely until the game ends.

Scores are terminal win = 1, loss = 0, tie = 0.5. A turn/step cap produces an unknown score, never a fabricated draw. `tools/summarize-counterfactual.py` rejects duplicated trials and keeps capped outcomes unmeasurable. Paired differences use matching `(seed, trial)` values. The reported interval is a normal approximation; the exact two-sided McNemar test is also retained for binary outcomes.

The final reference comparison fixes 1,024 trials per action before completion: four seeds 9973401–9973404, 256 trials each. Both continuation agents use full-information `RmAgent` with UCB selection and 1,000 iterations per decision. The separate OpenSheet continuation check uses `OpenAgent`, 256 trials per action, seed 9985201 and 1,000 iterations per decision. Neither continuation budget is the product's 30,000-iteration decision budget. These are conditional policy comparisons, not estimates of optimal play or ladder win rates against humans.

The earlier actual-sheet screen (128 trials, 300 iterations, seed 9712401) and confirmation (256 trials, 1,000 iterations, seed 9853219) remain separate from the final sample. They are not pooled into it. T18's Swords Dance and switch alternatives were selected from the screen for further comparison.

The T24 HP sensitivity uses 128 trials with seed 9853219 and the same 1,000-iteration reference continuation. With 6 HP, Earthquake wins 24/128, Swords Dance 26/128, switching 40/128 and Hidden Power 26/128. The matching 7-HP runs win 35/128, 29/128, 34/128 and 25/128 respectively. Small state changes can change later search choices and therefore whole trajectories; the percentage reconstruction is a material limitation, not an exact reconstruction claim.

| Turn | Action | Reference wins / 1,024 | OpenSheet wins / 256 |
|---|---|---:|---:|
| 11 | Earthquake | 332 (32.4%) | 81 (31.6%) |
| 11 | Reported switch | 236 (23.0%) | 66 (25.8%) |
| 11 | Current Hidden Power | 253 (24.7%) | 73 (28.5%) |
| 18 | Earthquake | 464 (45.3%) | — |
| 18 | Reported Hidden Power | 454 (44.3%) | 117 (45.7%) |
| 18 | Swords Dance | 440 (43.0%) | 113 (44.1%) |
| 18 | Switch | 450 (43.9%) | — |
| 24 | Earthquake | 287 (28.0%) | 63 (24.6%) |
| 24 | Switch | 313 (30.6%) | 74 (28.9%) |
| 24 | Reported Swords Dance | 230 (22.5%) | 55 (21.5%) |
| 24 | Current Hidden Power | 189 (18.5%) | 48 (18.8%) |
| 26 | Thunderbolt | 334 (32.6%) | 83 (32.4%) |
| 26 | Reported Fire Punch | 257 (25.1%) | 64 (25.0%) |

These completed runs contain no ties or caps. The table's T24 uses the ordinary 7-HP reconstruction; the 6-HP sensitivity is reported above. `final-skuct.jsonl` and `open-tail.jsonl` retain every final trial. Their `.summary.json` files contain all paired comparisons and confidence intervals.

The seven declared reference comparisons use exact two-sided McNemar tests with Holm adjustment. All except T18's Swords Dance versus Hidden Power remain significant at 0.05: T11 Earthquake versus switch +9.4 percentage points (adjusted p=0.000018), versus Hidden Power +7.7 (p=0.000200); T24 Earthquake versus Swords Dance +5.6 (p=0.00682), switch versus Swords Dance +8.1 (p=0.000062), Earthquake versus Hidden Power +9.6 (p<0.000001); T26 Thunderbolt versus Fire Punch +7.5 (p=0.000012). T18 Swords Dance versus Hidden Power is −1.4 points (p=0.551).

The 256-trial OpenSheet check is a separate sensitivity analysis. Its nominal exact p-values are 0.163 for T11 Earthquake versus switch, 0.475 versus Hidden Power, 0.769 for T18 Swords Dance versus Hidden Power, 0.466 for T24 Earthquake versus Swords Dance, 0.064 for switch versus Swords Dance, and 0.034 for T26 Thunderbolt versus Fire Punch. These smaller-sample results should not be described as independent proof of every reference result. The T26 result supports Thunderbolt specifically, not every move other than Fire Punch.


## Existing configuration candidates

Each candidate uses 30,000 iterations, the actual sheet and seeds 61001–61008. This is a diagnostic screen, not a strength gate.

| Candidate | T11 | T24 | T26 |
|---|---|---|---|
| Current profile | Hidden Power 8 | Hidden Power 8 | Thunderbolt 8 |
| `c=0.4` | Switch 5, Hidden Power 3 | Swords Dance 4, Hidden Power 4 | Fire Punch 7, Thunderbolt 1 |
| 24-turn rollout | Hidden Power 8 | Hidden Power 8 | Thunderbolt 8 |
| Existing M16c rollout heuristics | Switch 1, Hidden Power 7 | Switch 5, Hidden Power 3 | Thunderbolt 7, Dynamic Punch 1 |
| M16c + 24-turn rollout | Switch 4, Hidden Power 4 | Switch 6, Hidden Power 2 | Thunderbolt 6, Fire Punch 2 |

None chooses Earthquake at T11. Lowering exploration or combining the rollout changes reintroduces Fire Punch at T26. Increasing rollout length alone leaves T11 and T24 unchanged. No production policy or shared search profile is changed by this audit. The measurements do not establish a regression-free improvement.

## Mechanic checks

Exact enumeration against the recorded opponent response gives the following one-turn results. They describe this response only and must not be substituted for final win rates.

- T11 against Rest: Earthquake KOs Shuckle with probability 1; Hidden Power only with its 1/16 critical-hit branch.
- T24 against the Starmie switch: Earthquake and Hidden Power let Marowak survive in the 1/16 critical-KO branch. Swords Dance leaves Marowak fainted with probability 1. A KO can suppress the acting Pokémon's poison tick in this engine's frozen Gen2 rules.
- T26 against Confuse Ray at the reconstructed 23 HP: Thunderbolt KOs Starmie with probability 1/2; Fire Punch with probability 1/32. At 22 HP a maximum noncritical Fire Punch can also KO, but its total KO probability remains below Thunderbolt's. Starmie's Miracle Berry is still held, while Ampharos's Miracle Berry and Shuckle's Gold Berry have already been consumed.

The four tests in `crates/bot/tests/report_4296.rs` verify the actual sheet, consumed-item state and these branches. The aggregation checks cover opposite outcomes, duplicate trials and capped results. The new optional OpenSheet continuation path leaves the default full-information path's checked outcomes and battle seeds unchanged.

Control at T11, forced Earthquake, 128 trials, 300-iteration opponent, identical trial seeds: random continuation 9 wins, conformant greedy 48, search 52. This detects the intentionally empty policy; it does not establish that increasing search iterations always improves play. The exact one-step tests provide independently known mechanic anchors.

## Reproduce

Run commands from the repository root. Use a new output directory for new measurements.

```sh
cargo build --release -p nc2000-bot --example reported_decisions --example position_counterfactual
cargo test --release -p nc2000-bot --test report_4296

target/release/examples/reported_decisions \
  --log data/report-4296/battle.raw.log \
  --own-team data/report-4296/own-team.json \
  --opponent-team data/report-4296/opponent-team.json \
  --profile open --seed 61001 --seeds 64 \
  --out tmp/report-4296-rerun

target/release/examples/position_counterfactual \
  --position data/report-4296/turn-11.json \
  --opponent-team data/report-4296/opponent-team.json \
  --action 'move earthquake' --reply search \
  --seed 9973401 --trials 256 --iters 1000
```

Repeat the last command for each listed action and each of the four final seeds. For the separate OpenSheet continuation check, add `--policy open --seed 9985201` and keep 256 trials. For exact enumeration, use `--one-step --reply 'move rest'` at T11; the other recorded replies are `move sleeptalk` at T18, `switch 2` at T24 and `move confuseray` at T26.

For the HP correction, use `turn-24-hp6.json` with the continuation command. To regenerate its current-policy reproduction, copy the raw log to scratch and change the sole T23 `|-damage|p2a: Marowak|4/100 psn|[from] psn` line to `|-damage|p2a: Marowak|6/166 psn|[from] psn`, then run `reported_decisions --turns 24` against that copy. This supplies the inferred exact own HP: 166 minus eight normal-poison ticks of `floor(166/8)`, giving 6. The original replay file is retained intact apart from anonymization.

The reproduction harness's `--balanced` flag, if used for another investigation, allocates the requested iteration count **per action**. Its output explicitly labels a shared opponent-root search reward, not a terminal win rate. All reproduction and candidate counts above use ordinary UCB allocation.

`position_counterfactual --start-trial N --trials K` resumes exactly trials N through N+K−1 of the same seeded run. It advances the trial RNG through the skipped prefix without changing the synthesized initial state. Sixteen skipped-prefix trials across the four positions matched the frozen binary's outcomes, battle seeds and step counts. This was used to finish an interrupted execution without dropping or duplicating completed trials.

OpenSheet continuation agents are initialized before the forced first action. The discarded initial OpenSheet continuation implementation delayed our agent's initialization until its next free choice; if the forced move consumed the opponent's Miracle Berry, the pin incorrectly recorded an originally empty item slot. The corrected four-trial case with seed 9985201 completes, including the formerly failing Fire Punch branch. Only corrected OpenSheet continuations are included in the final results.

## Deferred algorithm research

Status as of 2026-09-07, code baseline `507f331`: the user explicitly deferred implementation to a later session and requested this record. No SM-MCTS-A or MCCFR replacement has been implemented or accepted. Production profiles remain in [search-profiles.json](../search-profiles.json); publishing this investigation does not adopt a candidate. The scope below is online decision search, not a commitment to solve the whole battle game or replace team preview.

### Evidence to retain before restarting

Use the linked reports for measurements and reproduction commands; their `build.json`, manifests and result files bind the relevant code, inputs and seeds. Do not rerun the entire investigation just to recover its conclusions.

| Investigation | Recorded revision | What it establishes and what it does not |
|---|---|---|
| [Reconstruction and conditional continuation comparisons](#counterfactual-method) | `d89f5ef` production baseline; report manifest binds measured builds | Earthquake is supported at reconstructed T11 under the tested continuation policies. This is neither a solution of the position nor a prediction of a replacement algorithm's overall win rate. The historical private requests and bot RNG remain unavailable. |
| [30-tree candidate and interrupted regression](T11.md), [archived run](../ensemble-regression-v1/README.md) | integration `1f9ea54`, withdrawal `19c434a` | Action agreement did not explain or repair the cause. The interrupted experiment is not a completed strength gate; do not reinstate the ensemble on this evidence. |
| [Unchanged-search trace](t11-cause/README.md) | `e60efd2` | A recorded opponent Psychic instead of Surf causes a large local survival reversal and credits the resulting reward to root Hidden Power. Branch fragmentation and adaptive sampling histories differ. This does not establish that the identified mechanism explains the entire root ranking. |
| [Frozen continuation and root-matrix control](t11-frozen/README.md) | `92e933e` | Fresh evaluation removes one adaptive-history confound, but the learned continuation is still inadequate. Opponent sampling inflates rewards; the paired difference in inflation between Hidden Power and Earthquake is not established across the new seeds. Neither candidate supports adoption. |
| [OpenSheet c=0.4](t11-c04/README.md) | `507f331` | Reduces the measured exploration asymmetry, mostly changes T11 to switching, and adversely changes the T26 diagnostic. It is not evidence for an Open profile change or a full-game strength gain. |

### Diagnosis and intended objective

Incrementing a visit count is not, by itself, a logic error. In the current UCB path, a rollout return increases `W`, the estimate `W/N` drives later allocations, and the execution rule chooses the most visited eligible root action. Returns include both players' exploratory continuation choices. Actions with differently explored descendants are therefore compared under different, changing continuation policies. Merely excluding some visits, selecting the largest mean, lowering `c`, or keeping only recent samples does not establish a valid replacement estimator.

The local causal finding is stronger than an action-frequency observation but weaker than proof that Earthquake is optimal. The frozen-policy result is an explicit reason not to promise that removing opponent exploration alone will repair T11. Distinct HP, Encore-duration and Quick Claw states must not be merged merely to make the preferred action win.

For a finite, fully observed, two-player zero-sum simultaneous game, the reference objective is matrix-game backward induction:

```text
Q(s, a, b) = sum over s' of P(s' | s, a, b) V(s')
V(s) = max over x in Delta(A) min over y in Delta(B) x^T Q(s) y
```

Chance is averaged by its actual probabilities. Both current actions are chosen without observing the other's choice. Mixed strategies are necessary in general. Solving a matrix of old UCB cell means only solves that supplied matrix; it does not correct its continuation values.

### Candidate algorithms and limits

1. **Exact matrix backward induction** is the correctness reference on small finite games and tractable battle fragments. Reuse `ExactSolver` and `solve_matrix_full` in [exact.rs](../../crates/bot/src/exact.rs), including the primal/dual best-response gap. Engine chance enumeration already exists. Its bounds, incomplete enumeration and leaf approximations must stay visible; this is not a proposal for exhaustive full-battle search.

2. **SM-MCTS-A with an appropriate regret-matching or Exp3 selector** is a candidate for fully observed simultaneous search. The averaging variant matters: in Algorithm 2 of Kovařík and Lisý, selection at the parent receives the child's accumulated mean, while the node accumulates the raw sampled return. Repeatedly averaging already averaged returns is not the same algorithm. Its guarantee also requires the stated no-regret selection and guaranteed exploration assumptions. Replacing UCB with an arbitrary regret matcher and continuing raw-return backup is insufficient. Use the paper's average/empirical strategy output, not an assumed pure argmax. The finite-tree convergence result does not automatically cover this engine's state abstraction, shared transpositions or hidden-state determinizations. The paper also reports slower empirical convergence for the averaging variant; correctness is not a finite-budget strength guarantee. [Analysis of Hannan Consistent Selection for Monte Carlo Tree Search in Simultaneous Move Games](https://arxiv.org/html/1804.09045), Algorithms 1–2, Theorem 5.4 and Sections 6–7.

3. **Information-set external-sampling MCCFR** is the architectural candidate discussed for supporting blind play. Maintain one policy across histories indistinguishable to the acting player, update counterfactual regrets with the correct reach/sampling weights, and return the appropriately weighted average strategy. External sampling enumerates the updating player's alternatives while sampling opponent and chance actions. Do not transfer an outcome-sampling importance-weight formula without deriving the estimator for the chosen sampling scheme. The convergence results concern the defined finite, perfect-recall, two-player zero-sum game; they do not establish that arbitrary root resets, cutoffs or guessed-state policies solve the original imperfect-information game. [Monte Carlo Sampling for Regret Minimization in Extensive Games](https://proceedings.neurips.cc/paper_files/paper/2009/hash/00411460f7c92d2124a67ea0f4cb5f85-Abstract.html).

DUCT has simultaneous-game counterexamples, yet outperformed the alternatives overall in one published nine-game comparison. Thus the theoretical reason to consider replacement does not provide an expected Elo gain. [Monte Carlo Tree Search Variants for Simultaneous Move Games](https://mlanctot.info/files/papers/cig14-smmctsggp.pdf).

MCCFR was called the leading architectural candidate in the discussion, not a demonstrated best investment. Online imperfect-information search/re-solving remains an unresolved design item: define the public/private root state, reach or boundary information, and reuse/reset semantics before claiming CFR guarantees. A potentially relevant follow-up is [Online Monte Carlo Counterfactual Regret Minimization for Search in Imperfect Information Games](https://www.mlanctot.info/files/papers/aamas15-iioos.pdf); it was identified, but not fully reviewed in this investigation.

### Implementation map

| Existing component | Reuse or required change |
|---|---|
| [smmcts.rs](../../crates/bot/src/smmcts.rs): `Node`, `select_ucb`, `run_iteration_with_leaf` | Current per-action counts/reward sums and raw-return backup; replace value/update/output semantics together. `solve_rm_plus` is a matrix solver, not an information-set CFR implementation. |
| [blind.rs](../../crates/bot/src/blind.rs): `BlindSearch::step_one_impl`, `best` | Root belief sampling and global UCB allocation; current best action uses visits. Preserve information access and legality while defining the new strategy representation. |
| [observe.rs](../../crates/bot/src/observe.rs), [import.rs](../../crates/bot/src/import.rs) | Reuse real-game parsing and reconstruction. `Observer` operates at real decision points, not inside simulated search trajectories; it is not a ready perfect-recall information-set adapter. |
| [shared_search.rs](../../crates/bot/src/shared_search.rs) | Contains UCB and masked state keys, not CFR. Do not mistake its shared own-state key for a complete information-set model. |
| Engine, `Belief`, position snapshots, `exact.rs` | Reuse legal choices, clone/reseed, chance enumeration where tractable, root data and exact small-game references. A belief sample is not permission to expose guessed hidden state to a policy. |
| `SearchTrace`, [frozen.rs](../../crates/bot/src/frozen.rs), [report_4296.rs](../../crates/bot/tests/report_4296.rs) | Existing observation, causal controls and regression fixtures; frozen execution remains an experimental evaluator, not the adopted search policy. |
| [WASM bridge](../../crates/wasm/src/lib.rs): `step`, `iterations`, `best`; arena/duel harness | Preserve responsive stepping and legal output. Define a compute budget appropriate to the new algorithm; one external-sampling traversal is not one current sampled trajectory. |

The substantial missing work is a per-player action/observation history within simulation, with the player's own private information and perfect-recall-compatible information-set keys. A policy must not condition on guessed opponent sets or other hidden fields. A sequential encoding of simultaneous choices must hide the first choice from the second player. OpenSheet reveals sets, but does not generally reveal picks or every hidden dynamic field; it must not be treated as full information by its mode name alone.

For the first prototype, use exact keys and an explicitly defined finite test game. Then separately evaluate any HP bucketing, history compression, transposition sharing, root belief restriction, action masking, heavy rollout or horizon cutoff. These alter the modeled game or estimator; no theorem about the unabstracted game can simply be attached to them. Existing module documentation about failed online outcome-sampling RM experiments is a practical caution, not evidence that correctly implemented external-sampling MCCFR has already been tried.

### Effort and expected return

The following are provisional human engineering estimates for one experienced Rust/search engineer, including investigation and validation, assuming reuse of the engine and harnesses and no team-preview rewrite. They are cumulative, not additive, and are not elapsed-time promises for an automated session.

| Milestone | Cumulative effort | Evidence bought |
|---|---|---|
| Correctness prototype on small games with known solutions | 3–5 person-days | Whether the implementation approaches the right value and strategy despite exploratory play. |
| Connection to Open battle positions and equal-time comparison | 1–2 person-weeks | Whether better estimation survives the practical compute budget. |
| Blind information sets, online search semantics, WASM integration and regression evaluation | 4–8 person-weeks | A potentially shippable candidate; unresolved information modeling can extend this range. |

There is no supported forecast in win-rate points or Elo. Possible benefits are reduced dependence on opponent exploration mistakes, mixed-strategy handling and more consistent hidden-information reasoning. At equal thinking time the candidate may instead be weaker because it covers less of the game. The T11 conditional continuation advantage in this report must not be converted into a whole-game gain for an unimplemented algorithm.

### Resume with a bounded feasibility study

The suggested first investment is 3–5 person-days, not approval already granted to implement it. A later session should establish scope before proceeding beyond the user's deferred request.

1. Define the finite test game, information structure, utility, output strategy and sampling estimator before coding. Validate against a solved matrix game requiring a mixture, a multi-level game where opponent exploration can reverse the apparent root ranking, and a hidden-information game that detects conditioning on inaccessible information. Inspect regret, value error and exact best-response exploitability; action agreement alone is insufficient.
2. Compare at fixed wall time and record engine transitions, traversal count, memory and uncertainty across independent seeds. External sampling can do substantially more work per traversal. Separate correctness/convergence results from finite-budget strength results; do not use nominally equal 30,000-iteration runs as equal compute.
3. Only after the small-game estimator checks pass, define the Open-position prototype and an independent position panel. Include the recorded T11 mechanism and T26 regression, but do not tune only to these known examples. Preserve replay/HP uncertainty. Report terminal versus cutoff evaluations explicitly.
4. Advance to blind modeling and full-game validation only when the earlier evidence justifies it. Freeze candidate settings and evaluate independent terminal games with paired seeds/sides and uncertainty, including the current bot and other relevant opponents. Correctness on small games is necessary evidence, not proof of whole-game improvement.
5. Stop and reassess if the implementation cannot reach the known solution, the mechanism persists, or equal-time comparisons show no useful improvement. Do not substitute an ensemble, an exploration-coefficient sweep or an unplanned large arena for identifying the cause.
