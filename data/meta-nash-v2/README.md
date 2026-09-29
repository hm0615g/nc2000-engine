# meta-nash-v2 — the Nash door's mixture, rebuilt

`pool-artifact.json` is generated (`node tools/build-team-products.js`) from
`solution.json`. It is what `?nash` draws the bot's team from, once per
battle. Every support team is labelled strong in
`data/team-inventory-v1/classification.json`; the weights are stored here,
never as labels.

## Conditions

The product's Nash-door information structure on both sides: blind search
with the frozen opponent prior (`data/belief-pool-v2`), equal iterations,
no pondering, live preview search, 500-turn cap. Harness:
`crates/bot/examples/team_eval.rs`; solver: `tools/nash-solve.py` (RM+,
seed-index bootstrap); challenges: `tools/nash-challenge.py`.
META-NASH v1's harness (`meta_nash.rs`) was not reused because it plays
true-state `skuct` on both sides and never consults a belief prior.

## Files

| File | What |
|---|---|
| `solution-blind300.json` | the 79 strong teams, all pairs, 32 games each (98,592 games) |
| `solution-blind1000.json` | 18 support-relevant teams, 64 games per pair |
| `solution-blind3000.json` | the same 18 teams at 3000 iterations |
| `solution-blind3000-do1.json` | 21 teams: the 18 plus the three closest challengers (one double-oracle round) — the shipped game |
| `challenge-blind3000.json` | every other measured team, the held-out page teams and the off-prior chimeras against the mixture |
| `solution.json` | the shipped weights (support ≥ 0.01, renormalized) |

## What the budget does

The equilibrium moves with search budget, as META-NASH v1 found: at 300
iterations the anchor is sample-07 with a wide support; at 1000 sample-07,
rental-cban-2, sample-08, rental-cban-3 and 魔人島 サンダー昆布; at 3000
サンダー昆布 (Spikes + Whirlwind) becomes the heaviest team and beats
sample-07 0.78 head to head. The product plays at ~30k iterations, so the
highest measured budget is shipped. Exact weights are not claimed to be
budget-invariant; the claim is the one `docs/TEAM-POOL-REBUILD-PLAN.md`
§Results states, with its challenge evidence and limits.
