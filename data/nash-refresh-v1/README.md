# Adaptive bot Nash audit

The 178 scored games found two successful responses to the deployed mixture. The fixed-prior five-strategy fit allocates **76.01% to Sample07** (19.00% per variant) and **23.99% to `red-v10-s1105-r1-s0148`**. The candidate carries the measured sets. The Web default and its compiled preview policy retain their deployed settings. [Candidate](candidate.json) · [Numerical results](summary.json) · [Build and input hashes](manifest.json)

| Question | Result |
| --- | --- |
| Can an existing type exploit the old draw? | `apex-c08-surgical-142` scored an estimated 61.9% on the four training blocks, 60.6% on the other four, and 71.7% on the independent focused test after weighting opponents. |
| Does Sample07 challenge the draw? | 11/16 pilot wins, then 12/16 fresh wins (85.0% when weighted by the deployed type probabilities). |
| How does Sample07 fare on expansion? | 6/8 against `apex-c08-surgical-142`; 3/8 against outside challenger `deme-r1-00-s0030`. Deme's result covers only the Sample07 component of the proposed mixture. |
| Is a changed preview ready to deploy? | Cloyster leading Snorlax and Heracross increased the raw score from 4/16 to 7/16 and the opponent-weighted score from 15.0% to 33.75%. Two seed pairs improved; the other six had zero net change. |

**Decision.** Keep this as a candidate. Three of the four Sample07 variants have no equivalent full set in the shipped opponent prior, so the measured advantage includes play against opponents who cannot infer those sets from that prior. This first experiment held the prior fixed. The [disclosed-pool follow-up](disclosed/README.md) tests the complete candidate, including its red component, against Deme and the incumbent. Both players know the exact opposing mixture and search their own three-mon selection.

The empirical solution also has a range of equally good weights. In the recorded matrix, Sample07 and the red component drew their direct matchup. Giving red anywhere from 0% to two-thirds and allocating the rest to Sample07 yields the same game value. RM+ returned one point in this interval. The four Sample07 variants were always proposed as a uniform draw; their relative weights were fixed throughout.

The pilot, preview counterfactual, internal response, and expansion ran locally. Colab CPU ran six of the sixteen external holdout games. Its WASM, picker and data hashes matched the local run. All games ended before the turn cap.

## Conditions

The unit of replication is a pair of games with the same battle seed, teams, and team-specific search seeds. The teams exchange seats for the second game. Both players receive their own team and player-channel observations. Each uses the shipped belief pool, with equal conditional weight on the four incumbent types. The incumbent team draw uses weights of 40%, 30%, 15% and 15%.

The harness runs the Node WASM build of the game and protocol searcher at `0325ea9a`. It imports the Web compiled-preview picker and compatibility check. Each side independently takes its compiled draw or searches a missing preview at 729k. Battle decisions use the adaptive 243k cap and commit the legacy execution policy. A tie or 500-turn cutoff scores half a point. Each JSONL line records the seeds, both team IDs, actual preview sources and choices, outcome, turns and elapsed milliseconds.

## Measurements

- **Pilot.** All six distinct pairs of incumbent types play the same eight seed blocks. The extra cell draws an incumbent by shipped weight and pairs it with the historical Sample07 four-variant challenger. Each challenger variant appears in two blocks. The different randomization schemes mean that the mix of incumbent types in this cell can deviate from the shipped weights.
- **Selection.** The most common incumbent, `apex-c04-bug-3atkstar-boomgen`, tries its second most frequent baked selection against the incumbent public shell. Cloyster leads Snorlax and Heracross (`team 2, 1, 4`, 13 of 64 baked entries). The intervention is identical against every hidden type. Each opponent type gets two seed pairs, weighted at its shipped frequency in the estimate. Twelve control games come from the pilot. Four mirror controls are new. In two of the sixteen comparisons the control already selected the intervention, leaving fourteen intervention games to play. [Matched game indices](preview-pairs.json)
- **External holdout.** Eight fresh blocks give each incumbent two paired matches against different Sample07 variants. All four challenger variants occur twice. The challenger sees the same incumbent public shell. The same per-incumbent scores can therefore be reweighted for both the deployed and fitted mixtures.
- **Internal holdout.** The first four pilot blocks select a provisional mixture and its test opponent (the best incumbent response to the deployed mix). The remaining four blocks assess this response against each mixture. The full eight blocks fit the final four-type empirical matrix with the existing RM+ solver. Its diagonal is 0.5 by symmetry.
- **Focused response.** The first-half leader scored 61.875% against the deployed weights. Eight additional independent seat pairs test it against the other incumbents (three blocks each against types 1 and 2, two against type 4). The mirror contributes exactly 0.5 to its expected score. Every mixture uses the same three estimated matchup scores.
- **Expansion.** Sample07 advanced after winning both its pilot and holdout challenges. Four fresh seat pairs per opponent test its four variants against the four-type pilot winner and against the catalog team with the largest approximate preview game value against the Sample07 shell (`deme-r1-00-s0030`, 0.8085). The five-strategy fit uses the balanced fresh result against the pilot winner, the holdout for the other three columns, and pools the focused incumbent-response samples with their corresponding pilot cells. The Deme check stays outside this five-strategy fit.

## Reproduction

Build the Node WASM target and install the Web dependencies. Each phase uses a new output directory. Replaying an archived JSONL file uses only its task fields. The verifier reads the archived outcomes and uses the same RM+ code as the historical team-game solver.

```sh
crates/wasm/build.sh nodejs
(cd web && npm ci)
node web/tests/nash-refresh.mjs data/nash-refresh-v1/pilot.jsonl target/pilot-replay 8
node web/tests/nash-refresh.mjs data/nash-refresh-v1/preview.jsonl target/preview-replay 8
node web/tests/nash-refresh.mjs data/nash-refresh-v1/holdout.jsonl target/holdout-replay 8
node web/tests/nash-refresh.mjs data/nash-refresh-v1/response.jsonl target/response-replay 8
node web/tests/nash-refresh.mjs data/nash-refresh-v1/expansion.jsonl target/expansion-replay 8
python3 data/nash-refresh-v1/summarize.py
```
