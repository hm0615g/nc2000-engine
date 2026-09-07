# Fixed-policy evaluation experiment

**The candidate is not adopted.** It removes adaptive sampling history from the new root estimates, but does not establish a T11 improvement. Across 13 seeds its fixed-policy best response chooses Switch 2 ten times, Swords Dance twice, and Rock Slide once. The additional root-matrix control favors Earthquake in only one of five seeds. No production policy or profile is changed, and no blind-strength benefit is claimed.

Hypothesis: the root comparison is contaminated by returns from exploratory continuation actions. Keep one trained tree, freeze it, and evaluate root alternatives under its execution policy. This changes how root values are measured, rather than dividing search into independent trees or merging distinct battle states.

The initial candidate trains for 22,500 iterations and reserves 7,500 additional simulations for equally allocated evaluation of all legal root actions. At each known continuation node, both sides use the most visited action (the existing execution rule), without changing counts, weights, the tree, or the training RNG. An unknown or unsampled continuation falls back to the existing rollout and leaf evaluation. Final candidate selection maximizes the new evaluation mean among root-eligible actions. The production search is unchanged while this candidate is tested.

All root alternatives share determinization and RNG seeds within each evaluation round. The opponent's root action uses the fixed most-visited policy and cannot depend on our forced action. Evaluation therefore measures a response to that learned fixed opponent policy; it does not solve a new equilibrium or prove globally optimal play. Rollout fallback retains its original exploration and cutoff.

Before inspecting results, the discovery seed is 61001; confirmation uses 62001–62004 from the causal investigation and new seeds 63001–63008. Each candidate's nominal simulation budget is 30,000, as is its unchanged-search comparator. Wall time and terminal fractions must also be reported because simulations have different lengths. Extra control arms are experiment cost, not a hidden addition to the candidate's budget.

The causal control replaces execution below the root with sampling the frozen visit distribution, separately for our side, their side, and both. Root opponent policy, tree, random seeds, rollout and budget stay fixed. This is a factorial intervention on an extracted fixed policy, not a replay of adaptive UCB. The prediction is that sampling opponent exploratory actions increases root rewards, particularly below Hidden Power. If the candidate merely changes the chosen action without changing this mechanism, action agreement alone is insufficient evidence.

Checks include a known-value continuation where opponent exploration reverses the root ranking, a visit-scale confound, immutable evaluation and subsequent training-RNG equivalence, and root simultaneity. T26 and blind-mode checks follow only if discovery supports the candidate. These checks detect specific defects; general playing-strength improvement requires independent terminal-game evaluation and is not inferred from search means or action agreement.

## Discovery and next control

Seed 61001 selects Switch 2 under fixed execution. Earthquake averages 0.5192, Hidden Power 0.5543, and Switch 2 0.6281. Sampling only the opponent's continuation visit distribution raises Earthquake to 0.6426 and Hidden Power to 0.6826. Sampling only our continuation lowers Hidden Power to 0.4715 while Earthquake stays at 0.5190. Thus both sides' exploratory actions materially affect this comparison; removing only opponent exploration cannot be treated as a complete fix.

Across the four previously traced confirmation seeds, fixed execution chooses Switch 2 three times and Rock Slide once, never Earthquake. Hidden Power minus Earthquake changes sign across seeds. The frozen root opponent concentrates on Encore or awake Sleep Talk, so the fixed-policy best response can exploit that root choice. This is a limitation of the candidate's objective, not evidence of stronger general play.

Before inspecting the next result, an additional OpenSheet control will evaluate every root joint-action cell under the same frozen execution continuation and solve that estimated zero-sum matrix with the existing RM+ routine (2,000 sweeps). Its total remains 22,500 training plus 7,500 evaluation simulations. Each side chooses independently via its matrix policy; no opponent action is chosen after observing our current action. This control tests whether re-solving the root rather than freezing its opponent repairs the fixed-root-policy weakness. It does not remove errors already learned inside the frozen continuation. No blind matrix is defined here because an opponent move need not exist in every determinization.

## Completed results

| Policy below the root | Choices on 8 new seeds | Mean Hidden Power minus Earthquake reward |
|---|---|---:|
| Unchanged 30,000-iteration search | Hidden Power 8 | +0.02144 |
| Execute both frozen policies | Switch 2: 6; Swords Dance: 2 | -0.01319 |
| Sample only our frozen visit distribution | Switch 2: 7; Rock Slide: 1 | +0.00531 |
| Sample only their frozen visit distribution | Hidden Power: 5; Rock Slide: 2; Earthquake: 1 | +0.01274 |
| Sample both frozen visit distributions | Hidden Power: 5; Rock Slide: 2; Switch 2: 1 | +0.01619 |

On these new seeds, restoring opponent sampling increases Earthquake's mean reward by 0.14233 and Hidden Power's by 0.16826. The paired *difference in these increases* is +0.02593, with a seed-level 95% Student-t interval **[-0.02364, +0.07550]**. The absolute reward inflation is clear; this sample does not establish a reliably larger inflation for Hidden Power across seeds. The earlier exact local intervention remains evidence of a contributing mechanism, not proof that it explains the entire root ranking. Removing only our sampling also changes rewards substantially. `summary.json` retains all 13-seed and new-8-seed summaries, including uncertainty rather than treating individual adaptive tree iterations as independent trials.

For seed 61001, the first observed full-HP Starmie encounter below Earthquake uses Surf in all 1,500 frozen evaluations. Below Hidden Power it uses Surf in 1,081 of 1,083 observed encounters; two choose Recover. Sampling only their frozen visit distribution changes these shares to 1,156/1,450 and 434/729 respectively. These counts cover the evaluated tree prefix, not fallback rollout decisions; path-dependent encounter counts must not be interpreted as a matched-state comparison.

The matrix control allocates 250 evaluations to each of 30 root joint cells. Its modal actions on seeds 61001, 62001–62004 are Switch 2, Rock Slide, Hidden Power, Earthquake, Switch 2. The artifact reports the complete mixed policy; `modal_action` is descriptive, not a claim that the solver plays a pure action. In every seed, Earthquake's four staying-opponent cells have **exactly identical means** with paired RNGs, unlike the adaptive historical means in the causal report. The integration test checks the stronger per-sample equality over 32 RNG seeds.

This resolves one estimator defect: equivalent continuations no longer receive different values because they were sampled at different stages of training. However, freezing a learned policy does not establish that its continuation choices are good. The policy remains based on contaminated training values, visit ties and sparsely sampled states; unknown continuations still use the old rollout. We have not isolated which remaining limitation explains each alternative ranking. In particular, a later Hidden Power choice by Marowak must not be labeled wrong merely from its name: different positions can give Earthquake and Hidden Power the same immediate KO outcome.

Cutoffs remain material. Across all 13 seeds, terminal fractions per action under frozen execution range from 69.8% to 99.87%; the remaining returns use leaf estimates. Measured training plus evaluation takes roughly 1.9–2.2 seconds per candidate search on this machine while other jobs run. These are timing observations, not an isolated throughput comparison. No general win-rate gate, T26 candidate gate or blind candidate gate was run after this failure to establish a useful T11 correction. The blind integration checks establish information handling and noninterference only.

## Validation and reproduction

The two estimator unit tests and 14 focused integration tests (`report_4296`, `blind`, `learning_search`, `mask_sleep_talk`) pass, as does `cargo check --workspace --all-targets`. Existing unrelated warnings remain. The integration checks cover immutable evaluation, legal actions, repeated-evaluation determinism, equal root opponent action across our forced alternatives, equal Earthquake staying continuations, and exactly unchanged subsequent search statistics/RNG in both pinned and blind modes.

All final result rows reproduce the initial measured policy statistics exactly after ignoring wall time and newly added diagnostic fields. The unchanged baseline after evaluation also reproduces the archived causal investigation's five 30,000-iteration root results, rather than merely agreeing on the chosen action. `build.json` binds the executable, sources and inputs; `summary.json` binds the result files.

```sh
cargo build --release -p nc2000-bot --example frozen_decision

target/release/examples/frozen_decision \
  --position data/report-4296/turn-11.json \
  --opponent-team data/report-4296/opponent-team.json \
  --seed 61001 --ablate
```

The same command with `--seed 62001 --seeds 4` produces `confirmation-known.jsonl`, and with `--seed 63001 --seeds 8` produces `confirmation-new.jsonl`. Replace `--ablate` with `--matrix` for the discovery and four known-seed matrix files. `--matrix` also emits an independent fixed-root evaluation and the baseline; those are controls and are not charged to the matrix candidate's stated budget. To aggregate saved files:

```sh
python3 data/report-4296/t11-frozen/analyze.py data/report-4296/t11-frozen
```
