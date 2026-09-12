# How T11 selects Hidden Power

The unchanged single-tree search credits wins caused by the opponent's exploratory mistakes to the root action that led there. Those mistakes occur more often below Hidden Power than below Earthquake: Hidden Power splits the continuation into more states, leaving fewer samples at each opponent decision. This is a concrete mechanism inflating Hidden Power's search reward, verified by tracing selection and backpropagation and by an exact one-step intervention. Its contribution to the entire root ranking has not yet been isolated from our own exploratory mistakes and adaptive sampling history.

This investigation changes observation only. It adds no corrective policy, action override in the live search, new allocation rule, or production setting.

The subsequent [fixed-policy correction experiment](../t11-frozen/README.md) tests this mechanism by separating training from evaluation. It is not adopted as a production fix; the report retains the failed candidate and its causal controls.

The later [c=0.4 OpenSheet comparison](../t11-c04/README.md) reduces measured exploratory selections but does not establish a T11 improvement and regresses the targeted T26 decision check. The production coefficient remains unchanged.

## The actual selection rule

At the information-set root, `blind::select_global` selects an unvisited action first, then maximizes

```
mean_reward + c * sqrt(ln(total_visits) / action_visits)
```

Each simulation increments that root action's visit count. Its final reward is accumulated into that action's reward sum. The opponent and both sides below the root use the same UCB rule over their respective state-node statistics. Backpropagation uses the simulation's realized reward, including outcomes following exploratory choices. `BlindSearch::best` finally chooses the eligible action with the most visits.

OpenSheet T11, seed 61001, 30,000 simulations, c=1:

| Root action | Visits | Mean search reward |
|---|---:|---:|
| Earthquake | 3,935 | 0.6298012090 |
| Hidden Power Bug | 13,927 | 0.6537693001 |

Hidden Power has the most visits across all five legal choices. These values exactly match the original archived reproduction. They are adaptive-search rewards, not independently measured win probabilities under a fixed continuation policy.

## One complete credit-assignment example

Seed 61001, iteration 24,418, fourth tree decision:

1. T11: Marowak uses Hidden Power into Shuckle's Encore. Shuckle survives.
2. T12: Marowak, locked into Hidden Power, knocks out Shuckle. Marowak has 156/166 HP and +2 Attack.
3. Starmie enters at 182/182 HP.
4. At T13 the opponent's node chooses Psychic into Marowak's Hidden Power.

The opponent's statistics immediately before that choice are:

| Opponent action | Prior visits | Opponent mean reward | Exploration bonus | UCB |
|---|---:|---:|---:|---:|
| Surf | 17 | 0.294118 | 0.496438 | 0.790555 |
| Psychic | 14 | 0.249377 | 0.547048 | 0.796425 |

Surf already has the higher empirical mean. Psychic is selected because its exploration bonus is larger. The sampled Psychic leaves Marowak at 73 HP; Hidden Power knocks out Starmie. The simulation ultimately wins for our side. The global root then gains exactly one Hidden Power visit and one unit of Hidden Power reward. The other root actions receive neither. `probe-24418.json` records the pre/post root statistics and the node's full UCB inputs; `examples.jsonl` contains the entire traced tree path and its terminal result.

An intervention clones that exact T13 state and changes only Starmie's chosen move. It exhaustively enumerates the engine's chance outcomes, including damage and critical hits:

| Opponent action, against the same Hidden Power | Starmie knocked out | Marowak knocked out |
|---|---:|---:|
| Psychic | 97.7564% | 2.2436% |
| Surf | 24.0385% | 75.9615% |

Both enumerations have probability mass 1. They establish the consequence of this local exploratory choice, not final counterfactual win rates. The intervention runs on a clone and does not alter the traced search.

Other recorded examples include a full-HP Starmie choosing Recover and being knocked out. Interior UCB selection does not consult the root's no-op exclusions; it can try such actions even when the existing `dominated_actions` helper identifies them as failing.

## Why the two root branches face different opponent behavior

Against Encore, the first child has exactly two state keys after Earthquake in every traced seed: Shuckle is knocked out before acting, with the two Quick Claw flag values distinguished by the existing state key.

Hidden Power produces 18 first-child keys: two critical-KO states, plus 16 noncritical states formed by two remaining-HP buckets, four remaining Encore durations, and two Quick Claw flag values. The engine initializes Encore duration in `[3, 7)` and the observed noncritical child has 2–5 turns remaining. `summary.json` records these dimensions. This is an observation of the current key and mechanics, not a proposal to merge different battle states.

The same total root budget consequently supplies very different numbers of samples to subsequent opponent nodes. Across seeds 61001 and 62001–62004, conditioned on root Encore, the first **observed tree decision** with full-HP Starmie against living Marowak gives:

| Quantity | Earthquake branch | Hidden Power branch |
|---|---:|---:|
| Observed encounters | 12,671 | 17,078 |
| Mean prior samples at the encountered opponent node | 1,483.2 | 154.0 |
| Surf chosen | 87.44% | 63.29% |
| Full-HP Recover chosen | 214 | 1,064 |
| A previously sampled action below the node's highest empirical mean chosen | 11.62% | 30.08% |
| An unvisited action chosen | 0.33% | 4.72% |

These are correlated observations inside adaptive searches. They are neither independent trials nor counts of rollout decisions. States differ in HP, Encore and history; the exact-state intervention above isolates one local opponent-action effect.

Encore is the most frequent root reply in four of the five seeds. Seed 62001 instead concentrates on awake Sleep Talk. The same sampling-density pattern appears there: two Earthquake child states versus six Hidden Power child states, and subsequent observed Surf shares of 89.60% versus 59.88%. Thus the observed effect is not confined to Encore restricting our own action list.

All five unchanged searches select Hidden Power. Its root mean exceeds Earthquake by 0.0155–0.0240, despite receiving more lenient opponent play on the traced continuations.

## Adaptive history also changes the meaning of the averages

In all five seeds, Earthquake against each of Shuckle's four staying moves reaches the same pair of child keys: Shuckle cannot execute the selected move before being knocked out. Yet the root cell means differ. For seed 61001, Earthquake/Rest averages 0.6963 and Earthquake/Encore 0.6079. Their samples occur at mean iteration 4,115 and 16,751 respectively, while the continuation policies are changing. These cell averages do not hold continuation strength or sampling time fixed.

The accumulated root score also retains earlier optimistic samples. In seed 61001's final quarter, Earthquake's reward is 0.5750 and Hidden Power's 0.5698, while the full-history means still favor Hidden Power. The final-quarter order reverses in three of five seeds, not all five. Replacing the average with a recent window is not tested or recommended by this observation.

The concrete defect under investigation is therefore the comparison of root actions using returns from opponent policies with different amounts of exploratory error and different sampling histories. A remaining causal test is to evaluate the branches with the same fixed continuation policy and separate opponent exploration from our own exploration. The present evidence identifies and verifies a contributing credit-assignment mechanism; it does not prove that this mechanism alone accounts for every part of the ranking or that Earthquake is globally optimal.

## Instrumentation and reproduction

`SearchTrace` supplies immutable battle/state-statistic views at tree choice, leaf entry and result. `trace_decision` snapshots the actual trajectory; it does not generate a fresh principal variation and present that as the original reasoning. At a choice event with multiple legal actions, visits already include the current selection and rewards exclude its backpropagation. Forced single-action choices skip these statistics. The probe subtracts the current selection before reconstructing UCB.

For all five seeds, traced and untraced visits, means, root matrices and node counts match exactly, including 100 subsequent untraced iterations. Summing all 150,000 traced rewards independently reproduces the final root means and counts. The added test also checks observation under both ordinary heavy and uniform rollouts, including cloned-state moves and cloned RNG reads. The original mechanic tests remain.

`cargo check --workspace --all-targets` and all 13 focused tests pass (`blind`, `learning_search`, `mask_sleep_talk`, `report_4296`). Existing unrelated compiler warnings remain.

```sh
cargo build --release -p nc2000-bot --example trace_decision
mkdir -p tmp/t11-cause

target/release/examples/trace_decision \
  --position data/report-4296/turn-11.json \
  --opponent-team data/report-4296/opponent-team.json \
  --seed 61001 --trace-out tmp/t11-cause/trace-61001-final.jsonl \
  --probe-iteration 24418 --probe-step 4 --probe-action 'move surf' \
  --probe-out tmp/t11-cause/probe-24418.json --verify \
  > tmp/t11-cause/root-61001-final.jsonl

target/release/examples/trace_decision \
  --position data/report-4296/turn-11.json \
  --opponent-team data/report-4296/opponent-team.json \
  --seed 62001 --seeds 4 --trace-out tmp/t11-cause/trace-confirm.jsonl --verify \
  > tmp/t11-cause/root-confirm.jsonl

python3 data/report-4296/t11-cause/analyze.py \
  tmp/t11-cause/trace-61001-final.jsonl tmp/t11-cause/trace-confirm.jsonl \
  --out tmp/t11-cause/summary \
  --checkpoints tmp/t11-cause/root-61001-final.jsonl tmp/t11-cause/root-confirm.jsonl
```

`root-checkpoints.jsonl` retains every 1,000-iteration root snapshot. `summary.json` contains per-seed and per-quarter aggregates, trace hashes, and the equivalent Earthquake child-key checks. `examples.jsonl` retains four explicitly identified illustrative trajectories. Full traces are reproducible scratch artifacts; `build.json` binds the measured executable, sources, position, actual opponent sheet and dex.

The replay did not provide the original bot RNG or private requests. This explains the reproducible current OpenSheet search on the established T11 reconstruction, not the historical bot's exact internal trajectory. No production policy has been changed.
