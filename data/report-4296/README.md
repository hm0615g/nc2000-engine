# Battle 4296: OpenSheet decision audit

The final reference comparison supports Earthquake at T11, Earthquake or switching at T24, and Thunderbolt at T26. It does not support the blanket T18 claim. The separate OpenSheet continuation sample points in the same direction for the supported comparisons, but T11 and T24 are not independently established at its smaller sample size. T24's Earthquake result also changes sign under the 6-HP correction; switching is the more consistent candidate. No tested configuration establishes an improvement without regressions, so the production policy is unchanged.

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
