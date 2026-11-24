# Stratified disclosed-pool revalidation

The five-type candidate scored **146–94** against the deployed APEX4 mixture across 240 fresh live games. Its mixture-weighted score was **61.0%**, with a conservative simultaneous 95% interval of **46.3–75.8%**. Both independent samples favored the candidate. The prespecified lower-bound criterion for rejecting APEX4 was not met. This experiment estimates one deviation and cannot establish a Nash equilibrium over other parties and mixtures. [Primary records](games.jsonl) · [Independent repeat](repeat-games.jsonl)

| Current vs current | Candidate W–L | Weighted score | Seat-pair interval |
| --- | ---: | ---: | ---: |
| Preregistered 160 games | 94–66 | 57.48% | 40.91–74.06% |
| Independent 80-game repeat | 52–28 | 68.08% | 44.63–91.52% |
| Combined | 146–94 | 61.01% | 46.26–75.77%, simultaneous across looks |

The two individual intervals are pointwise 95% Hoeffding intervals. The observed stratified standard error on the combined sample was 3.30 percentage points. Scores conditional on drawing each candidate type, weighted against the full APEX4 distribution, were 45.4% for red, 65.4% for base Sample07, 54.2% for g1-c4, 64.2% for g2-c4 and 80.0% for g1-c3. These are conditional scores inside the tested five-type disclosure. Changing the published distribution requires a new decision by each player.

## Historical attribution

All four rows below use the same 40 seed blocks, both seats, the current battle engine, current sets and deployed draw probabilities. The historical policy is the November 4 build associated with the adoption of 40/30/15/15. Historical candidates and inferences during the original optimization could have differed from this retrospective matchup.

| Candidate policy | APEX4 policy | Raw candidate W–L | Candidate weighted score |
| --- | --- | ---: | ---: |
| Current | Current | 44–36 | 55.83% |
| November 4 | Current | 39–41 | 51.45% |
| Current | November 4 | 62–18 | 74.18% |
| November 4 | November 4 | 48–32 | 59.31% |

Upgrading only the challenger added 4.38 points against the current defender and 14.86 points against the historical defender. Upgrading only the defender reduced the current challenger's score by 18.35 points and the historical challenger's score by 7.87 points. The bounded paired-difference intervals include zero. The historical challenger itself led in the old/old sample. The claim that its inability to play the candidate caused APEX4's adoption is unsupported. The [original 128-game evaluation](../../kodoku-beq-v1/production-nash-verification.json) used a different challenger and earlier incumbent weights.

### Budget and preview diagnostics

On the first seat-pair per cell (40 games per arm), changing the challenger from November 4 to current at the same fixed 81k battle budget raised its score against the 81k historical defender from 46.4% to 68.8%. Against the adaptive current challenger, the historical 81k defender allowed 67.0%, the current 81k defender allowed 70.3%, and the current adaptive defender allowed 56.3%. These are seed-matched diagnostic estimates from a smaller subset. Changing the search budget can change the apparent effect of a revision. All policies used 729k live preview. [Budget records](budget-games.jsonl) · [Paired diagnostics](followup-summary.json)

Across the 80 matched games, the updated challenger chose a different preview in 29 and the updated APEX defender did so in 62. For each contrast we selected the first two distinct blocks in each direction with a flipped winner and a changed preview, and replayed both seats. We retained normal preview search and random-number consumption, then gave the older side the later policy's realized picks. For challenger upgrades, this changed 4 of 8 results and reduced disagreements with the later policy from 6 to 4. For defender upgrades it changed 5 of 8 and reduced disagreements from 5 to 4. A separate self-override control reproduced every action. Selections explain some individual flips, with differences also persisting during battle. The selected blocks do not estimate the population fraction attributable to preview. [Override records](pick-games.jsonl)

The audit contains 577 scheduled or diagnostic games including the self-override control, plus one duplicate cross-platform gate execution. The Colab CPU runtime was released after collection. The original mixture remains deployed; the Deme follow-on was conditional on rejecting APEX4 and was not triggered.

## Design fixed before primary evaluation


Schedule fixed before observing new outcomes. The primary matchup is the existing five-type candidate against the deployed APEX4 weights. Both players know the opponent's exact distribution. Each of 20 type pairs has four independent seed blocks played in both seats. Seeds are the first three big-endian uint32 words of SHA-256 of `nc2000/disclosed-apex/20261119/v1/{block}`, in candidate-search, incumbent-search and battle order. Each policy searches its own preview. A public 500-turn cap counts as half a point and is reported separately.

The primary endpoint is the sum of seat-pair means weighted by the product of published probabilities. Its pointwise two-sided 95% Hoeffding radius is sqrt(log(40) / 2 × sum of squared cell weights divided by four). There is one primary look after 160 games. A lower bound above 0.5 rejects APEX4 against this candidate.

The first two blocks per cell are shared with the three historical-policy arms. These 80-game contrasts use the same current battle engine as the primary arm. The historical agent comes from a8ba973c, the adoption of the current mixture. It uses 729k live preview and fixed 81k battle search. The current agent uses 729k live preview and adaptive 243k battle search. Report paired differences over the matching blocks separately from independent evidence. If the primary interval crosses 0.5, use one additional independent schedule with identical stratification and its own interval. Subsequent analysis of a selected contrast is exploratory.

For the resource-limited independent repeat, use two fresh seat-pairs in each of the 20 cells (80 games), with the same SHA-256 seed rule and prefix `nc2000/disclosed-apex/20261119/repeat/`. Report the repeat separately. If reporting a decision across the 160- and 240-game looks, use simultaneous Bonferroni-Hoeffding bounds (97.5% per look).

## Reproduction

Run `python3 data/nash-refresh-v1/stratified/summarize.py` for the four scheduled arms and `python3 data/nash-refresh-v1/stratified/check-followups.py` for the independent repeat, paired budget contrasts and forced previews. Each prints a single JSON record. The [manifest](manifest.json) contains the input and WASM hashes and a cross-platform check of the entire action sequence.

To replay the schedule, build the current Node WASM package at the source revision in the manifest. Build the November 4 package in a separate checkout at `a8ba973c` with the Rust WASM target installed. Set `NASH_OLD_WASM` to that checkout's absolute `crates/wasm/pkg-node` directory, then run `node web/tests/nash-refresh.mjs data/nash-refresh-v1/stratified/schedule.jsonl NEW_OUTPUT_DIR 8` from the current project. The same runner accepts the repeat, budget and override game files as input schedules. Output timing and local file indices depend on the execution shard.
