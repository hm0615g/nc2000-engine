# Sample17 entry test

Compare the seven-type baseline with a fixed transfer of half the s0155 probability to the publicly distinguishable Sample17 roster. Sample07 and APEX4 keep their exact weights and within-roster probabilities. Each bot receives the complete opposing distribution, sees its usual player channel and chooses its own three. The transferred mass is 13.635% of all games. The proposal tests whether removing some favorable prey from Sample17 helps the pool and what that costs against the other rosters.

## Archived estimates

The original three challenger scores came from 16 random draws, two seat pairs per baseline type, and a follow-up with one pair for each other type and six for s0155. For the exploratory four-roster fit, pool the latter two prestratified trials within each type before applying the published weights. The 16 random draws that identified Sample17 as the leading challenger do not enter this fit. The different allocation of replication among types means this estimator can fall outside the range of the two trial totals. The four-roster matrix keeps the historical shell matchups among the three baseline rosters and uses these pooled estimates to price Sample17. The matrix is a proposal tool. The games below test the actual disclosures.

## Fresh evaluation and stopping rule

Against Sample17, preallocate 24 new seat pairs to each policy, with counts 8/3/3/2/1/1/6 following the seven published baseline IDs. The policies face the same opponent, starting seed and search seeds. For six s0155 pairs, the entry policy fields s0155 in three and Sample17 in three. Reweight each type to its published mass. This produces a fresh estimate and a one-sided 95% Hoeffding upper bound specifically for the Sample17 challenge to the original pool. An upper bound above 55% leaves a five-point tolerance unresolved. The archive-plus-new estimate is descriptive because the archive also motivated the follow-up.

Guard opponents are Sample07, APEX4, s0155 and the five-type Deme. Sample07 receives four seed pairs per stratum; each other guard receives two. The common stratum (86.365%) draws an identical type for both policies according to the normalized common mass. In the transferred stratum (13.635%), s0155 plays for the baseline and Sample17 for the entry policy. Opponent types are drawn at their published probabilities and matched across policies. All scores use stratum weights and count the two seats as one block. This design takes 176 new games locally.

Advance the entry proposal only if both Sample17's score and the largest score across all five challengers fall. If it advances, recheck Sample17 and the worst opponent for each policy on new seeds (six pairs per stratum and disclosure per distinct opponent). Require both improvements again before recording a new research candidate. Estimates for this finite shortlist do not certify a Nash equilibrium over arbitrary legal pools.

Run `entry.py design` to regenerate the inputs. Run the production `web/tests/nash-refresh.mjs` with `NASH_POOLS` pointing to this directory's `pools.json`, passing `schedule.jsonl` and a fresh output directory. Run `entry.py evaluate OUTPUT_DIR` to verify the matched rows, seeds and production hashes and calculate the results. If a holdout schedule is emitted, run the same harness on it and use `entry.py confirm HOLDOUT_OUTPUT_DIR`.
