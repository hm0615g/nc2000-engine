# New matchups after moving from APEX4 to the seven-type pool

Compare the former 40/30/15/15 APEX4 pool and the retained seven-type candidate with the same production policy, full opposing distributions, and hidden realized types and trios. The 13.64% Sample17 entry proposal was rejected and is excluded. A worse matchup after the pool change and a winning challenger against the current pool are separate outcomes.

## Result

The five-team screen and two fresh checks used 230 local production games. Neither follow-up reproduced a winning matchup against the seven-type pool or an increase over the former APEX4 pool. Each percentage below is the challenger's score at the published defender weights.

| Challenger | Screen, former → current | Independent, former → current |
| --- | ---: | ---: |
| pb-122, Jolteon–Marowak | 57.5% → 58.9% | 56.0% → 39.6% (20 seat pairs per pool) |
| hc75-1st-show, Misdreavus–Alakazam | 7.5% → 38.4% | 37.7% → 36.8% (10 seat pairs per pool) |

Against the current pool, Jolteon appeared in 28/40 games and attempted Baton Pass in 9/40. Misdreavus appeared in 14/20 and attempted Perish Song in 11/20. The latter team scored an estimated 80.0% conditional on s0155, a small-stratum observation. The primary 95% interval for the full current pool was 8.8–70.4%; the secondary interval was 0–89.9%. The three other challengers received screen data only. The seven-type research candidate is retained.

[Screen](screen-summary.json) · [Primary check](holdout-summary.json) · [Secondary check](secondary-summary.json). All 230 games ended before the cap, with no ties. Optional raw logs are captured from the completed battle using `NASH_SAVE_LOGS=1`. One archived confirmation replay matched the original selections, action hash, outcome and turn count with this switch enabled; it is excluded from the totals.

Each new challenger is an unmodified legal singleton. Its unordered six species differ from every team in the prior 67-deviation screen and from the other four entrants. The tactics below motivated the choices. A bot can select a different plan from the same six.

| Team | Reason for testing |
| --- | --- |
| hc75-1st-show | Perish trapping by Misdreavus plus Encore Alakazam. The new s0155 sets have no phazing move. |
| pb-137 | Cross Chop and Encore on Machamp with explosive support. The new s0155 roster includes three Normal types. |
| pb-122 | Agility Baton Pass Jolteon and Thick Club Marowak with screens. Both the APEX4 and s0155 sets lack a phazing move. |
| sample-24 | Sun, sleep and explosions with Belly Drum Charizard or Moltres. The designated fire attacker can vary by opponent. |
| pb-95 | Spikes, phazing and Rapid Spin plus rain Starmie. The Sample07 and APEX4 rosters both carry Spikes and lack Rapid Spin. |

The screen uses one seat pair per defender type for each challenger (seven current, four former). All challengers share seeds and the four overlapping APEX4 types reuse the same seeds across disclosures. Weight scores by the actual published type probabilities. Rank candidates by the smaller of their current win rate minus 50% and their improvement over the former matchup. Ties prefer the higher current score and then the table order. Recheck one candidate on 20 fresh paired blocks against both full distributions, including when all screening scores fall below the nomination criteria.

The independent check pairs the common APEX4 portion with the identical realized type and splits the remaining probability at both distributions' cumulative type boundaries. Each joint stratum receives at least one seed pair. Additional pairs reduce the sum of squared block weights. Report separate 95% Hoeffding intervals for the current score and the paired change. A changed outcome alone does not establish the cause. Selection frequencies and recorded move attempts describe what the challenger actually did.

The screen nominated pb-122 for the primary check. The largest current-minus-former change belonged to a different team, hc75-1st-show, whose current screening score was below half. To address the separate question of matchup deterioration, freeze this second team before its follow-up and use one new seat pair for each of the ten joint strata (40 games across the two disclosures). This is a secondary exploratory comparison chosen after screening.
