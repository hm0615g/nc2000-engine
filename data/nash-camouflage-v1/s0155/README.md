# One-move variants of the s0155 Blissey–Miltank roster

The archived confirmation and stratified follow-up contained five seed pairs with s0155 against Sample17. The incumbent lost all ten games and chose Snorlax, Blissey and Suicune in that order in every seat. In the ten-turn game (stratified block 12, row seat 0), Exeggutor opened with Explosion against Snorlax. Porygon2 then used Curse three times, Recover once and Return to KO all three opponents. In the 19-turn game (block 13, row seat 0), opposing Snorlax traded with Snorlax via Self-Destruct before Porygon2 used Curse three times. Both diagnostic replays matched the archived action hashes and outcomes.

Three variants change one move on those selected Pokémon. Blissey exchanges Present for Counter, Suicune exchanges Sleep Talk for Roar, and Snorlax exchanges Sleep Talk for Self-Destruct. The production validator checks every team and the two player channels show identical public preview frames across variants. Both players search preview and battle with the same production WASM as the previous run. They know the complete opposing distribution and see only their own player channel. The other six party types and the published 27.27% s0155 weight are fixed throughout.

## Result

Retain the original seven-type research candidate. The 50/50 parent–Counter Blissey mixture advanced from the screen, where it won 4/8 against Sample17. Each pure policy won 1/8 across the same four seed pairs. For the exact realized types selected by the mixture, type-revealing controls won 0/8. The Roar and Self-Destruct mixtures won 2/8 and 1/8.

| Fresh comparison, scored from the s0155 player's side | Original policy | 50/50 Counter policy |
| --- | ---: | ---: |
| Conditional on s0155, against Sample17 | 4/12 | 2/12 |
| Entire pool, against Sample17, weighted by party frequencies | 46.14% | 41.59% |
| Conditional on s0155, against Sample07, weighted by opposing type | 60.00% | 17.50% |
| Conditional on s0155, against APEX4, weighted by opposing type | 47.50% | 13.75% |

On the six fresh s0155 seat pairs against Sample17, the mixed policy won 2/12 with its draw hidden and 3/12 with the same realized type identifiable. Five opponent picks and five outcomes changed. All 12 games drawn from the other six unchanged types matched their paired controls on the entire action hash. The full-pool change of −4.55 percentage points comes from the s0155 slot. The intervals are wide at this budget. None of the three matchups met the promotion rule.

[56-game screen](screen-summary.json) · [102-game holdout](result.json) · [WASM, input and record hashes](manifest.json). The two diagnostic replays are excluded from the 158 new games. All 158 ran locally, with zero ties and zero turn caps.

## Selection and checks

The 56-game screen conditions on seeing the s0155 roster. Baseline, each pure replacement, and each even parent–mutant mixture play four common seed pairs against Sample17. Each mixed candidate draws parent and mutant twice. The full seven- or eight-type distribution is disclosed in every game. Compare a mixed draw to the matched pure disclosure for that actual type to measure camouflage. Nominate the highest score strictly above baseline, preferring a pure replacement on ties and then the printed edit order (Counter, Roar, Self-Destruct).

The holdout uses new seeds. Each of the six unchanged types gets one seat pair, while s0155 gets six, with equal parent and mutant representation for a mixed nominee. Aggregate the paired scores at their published weights. Two additional seed pairs per opposing type test the old and revised s0155 roster against the supported Sample07 and APEX4 distributions. Each of those checks is weighted by the opponent's conditional probabilities. A mixed nominee also gets the same-draw disclosure control on the six target-matchup pairs. Retain a revision as a research candidate only if its full-pool and conditional s0155 scores both improve against Sample17 and neither weighted guard score decreases. This is a resource-bounded selection rule. Interval estimates count seat pairs as the independent blocks.

## Reproduction

Pass `pools.json` via `NASH_POOLS` and either archived games JSONL to `web/tests/nash-refresh.mjs` with a fresh output directory. Run `python3 data/nash-camouflage-v1/s0155/experiment.py nominate` and then `python3 data/nash-camouflage-v1/s0155/experiment.py summarize` to reproduce the selection and holdout analysis from the records.
