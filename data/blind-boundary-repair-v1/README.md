# Blind boundary repair — 2026-11-03

This repairs the product paths identified by the
[historical boundary/PS audit](../blind-boundary-audit-v1/README.md). The work is
on `fix/blind-boundary`, based on `97a133f6`; no deployment is part of this work.

## Product information boundary

The ordinary Web search worker receives only `ownTeam`, search configuration,
and incremental `PlayerChannel` frames. The trusted host retains the game and
sends filtered log lines, the observing player's request and legal inputs.
Opponent true sets, game RNG and unannounced choices are absent from the live
worker messages. Public preview level-cap rules also travel in the request;
external PS requests without that local extension retain the standard cap. Input aliases are resolved against the legal actions captured
when that search started, so a late result cannot read a newer private state.

Native `BlindAgent` and the compatible WASM `BlindSearcher` use `LocalPlayer`:
an explicit whitelist of the observing player's canonical submitted set plus
the same filtered protocol. Evaluation and PS play use `ProtocolAgent`.
`ProtocolTracker` reconstructs hidden runtime state from public history and
independent model randomness instead of copying real counters, Substitute HP,
selection, pending moves or Quick Claw chance results. Reconstruction supports
different roster sizes and keeps repeated own request members in distinct
slots, preserving the evaluator's relaxed-party mode.

`BlindSearch::from_observation` requires `ObservedPosition`; its real battle
field and constructor are private. A compile-fail doctest checks unauthorized
construction. This is a product API boundary, not a proof of the entire crate:
the adapter/importer remain trusted code, `model()` exposes a *hypothetical*
world for diagnostics, and `BlindSearch::new(&Battle, ...)` remains explicitly
a full-world research API. Historical fork/research workers still own their
recorded reconstruction; ordinary live workers do not receive that payload.

## HP and exclusions

The importer retains the announced numerator and denominator independently of
the guessed maximum. Every opponent candidate reconstructs HP using that
original bucket, including /100, legacy /48, full and fainted observations.
Changing a sampled maximum therefore cannot turn a visible 50% into 57%.
Raw research determinization also reconstructs opponent HP from a /100 bucket;
product paths use the original protocol denominator.

Product permanent heuristic root exclusions are empty. One guessed item,
speed or hidden counter cannot prove that a move is useless in every supported
world. Even selecting a last-mon self-KO move can win: sleep prevents execution
and the opponent then faints to poison. A regression first proves that win with
the engine, then checks the product mask. Request legality, format level caps
and exact preview aliases remain separate. Selection, execution distributions
and early-stop decisions consume the same eligibility. Simulation/rollout
heuristics still operate inside sampled hypothetical worlds.

The old zero-iteration priority-heal test required Recover to be excluded
against a guessed slow opponent. It now verifies that both actions remain in
the execution support and separately checks the actual priority/healing
mechanics. Residual-damage and own-Leftovers waiting scenarios still pass.

## Selective preview regeneration

Only the four primary `nash:true` catalog parties and their public-roster
interactions were regenerated. They share the APEX public roster.

| Quantity | Count |
| --- | ---: |
| Recomputed type-pair matrices | 182 |
| Reused other matrices | 946 |
| Rebuilt affected policy keys | 149 |
| Byte-identical unrelated policy keys | 841 |
| Complete active keys | 990 |

`--nash-only-reuse` checks the previous artifact/manifest/basis, computation
settings, unchanged dependencies and kernel functions, matrix job identities,
public candidate membership and exact own/public keys before reusing data.
[Preview verification](preview-verification.json) contains the baseline hash
and proof. The recomputed choices also matched the baseline: this finite
rollout baker does not call the repaired live search. The active artifact has
the current source identity and records which results were reused; it was not
made current by changing hashes alone. Catalog sets, prior contents and party
draw weights are unchanged.

[Production verification](production-verification.json) covers 1,089 ordered
catalog pairs on both sides (2,178 cases), all using the table with zero search
iterations. It also checks hidden-loadout invariance and rejects changed own
sets, prior bytes, public gender and public item presence.
[WASM coverage](compiled-coverage.json) checks all 69,696 catalog/quantization-slot
cases against engine legality.

## Verification

The regression compares complete independently filtered player frames before
comparing reconstructed worlds, root masks, visits, values and decisions.
Both sides are tested with different exact opponent HP in one public bucket,
hidden move/item/speed/bench selection, own sleep duration, pending opponent
input, real RNG and Quick Claw result. The production-budget run uses 81,000
iterations. Candidate-order/item/speed tests retain Recover under both
supported hypotheses; every admitted maximum reannounces the original HP.

The machine-readable check inventory is in `verification.json`. Existing
fallback unit-test failures are reported separately, with the same eight
failures reproduced at the unmodified baseline; they were not weakened or
fixed as part of this task.

## Adversarial Nash recheck (known-pool rules)

Information rules (owner, 2026-11-03): each bot does not see the opponent's
sets, but both sides know the opponent's selection pool and mixture weights,
as a Nash equilibrium assumes. Same-shell variant mixing is a fair tactic;
off-pool surprise parties are not allowed.

Harness: `omniscient_audit --production-challengers`. Per side, the prior is
`belief-pool-v3` with every team sharing the opponent's public preview
signature replaced by the opponent's mixture. At team preview every game
asserts `!is_fallback()` and `candidate_count == opponent variant count`
(zero violations). Both sides use 729k live preview search (the compiled
table was baked under the unmodified prior). Otherwise production settings:
`ProtocolAgent` + `PlayerChannel`, 81k iterations, `c = 0.4`, UCB, `best()`.

Candidates: shells ranked by variant-weight-optimized Bayesian preview value
vs `APEX4` from `data/kodoku-r3/apex-exp-cache.json`; all four are below 0.5
at that layer.

| Challenger | Variants (weights) | Preview-layer value | Games | W–L | Win rate | 95% CI |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| `Police3-shell` | sample-08 .221 / hc75-3rd-bean .535 / rental-cban-6 .244 | 0.4751 | 192 | 102–90 | 53.12% | [44.94%, 61.31%] |
| `mjj-2022-nrsk` | 1 | 0.4614 | 64 | 26–38 | 40.62% | [27.84%, 53.41%] |
| `Garap2-shell` | garap-a .68 / sib-r2-n3-garap-a-s06 .32 | 0.4754 | 64 | 25–39 | 39.06% | [24.65%, 53.48%] |
| `Sample12-5var-shell` | s0093 .30 / s0083 .25 / r2-s0101 .20 / s0089 .15 / sample-12 .10 | 0.4673 | 64 | 23–41 | 35.94% | [23.31%, 48.56%] |

Verdict: no challenger significantly beats `APEX4` under known-pool rules.
Raw records: `known-pool-challenger.json`, `known-pool-police3-p32-96.json`.

The earlier `mjj-2022-nrsk` 41–23 (64.06%) result (`challenger.json`) is
withdrawn: `APEX4`'s prior did not contain that party, i.e. an off-pool
surprise. Disclosed, it scores 40.62%.

### Police3 vs expectation

Preview-layer values (same cache, mixtures known, row side's score):

| | APEX4 | old APEX2 | Garap2 | nrsk | Sample12 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Police3 | 0.4751 | 0.5030 | 0.5669 | 0.5574 | 0.5026 |
| APEX4 | — | n/a | 0.5247 | 0.5386 | 0.5506 |
| old APEX2 | n/a | — | 0.4785 | 0.5271 | 0.5252 |

Police3's historical role was hunting the predators of the old APEX2
(Garap2 beat APEX2 0.5215). Police3 itself was roughly even with APEX2
(0.5030). APEX4 was selected to maximize its minimum over Police3 and Garap2,
so it covers Garap2 directly and beats Police3 by 2.5 points at this layer;
Police3 then left the support.

Bot play does not preserve the preview-layer ordering. Against APEX4 the
predicted shells all lost more than predicted (Garap2 −8.5, nrsk −5.5,
Sample12 −10.8 points) while Police3 gained (+5.6). The Police3 gap is
1.3 SE, so "Police3 slightly loses" is not contradicted, but the direction
is consistent with a layer effect: preview matrices come from heavy-playout
rollouts, not 81k search. Police3 games are much longer (mean 48 turns vs
16–33), and within Police3 the `hc75-3rd-bean` draws scored 67/106 while
`sample-08` scored 14/36 and `rental-cban-6` 21/50. Both are hypotheses
until tested with a pre-registered run.

## Reproduction

```sh
cargo test --release -p nc2000-bot --test player_boundary --test player --test blind --test import --test position --test hidden_pick_legality --test fork --test compiled_preview
NC2000_BOUNDARY_ITERS=81000 cargo test --release -p nc2000-bot --test player_boundary
cargo test --doc -p nc2000-bot
cargo test -p nc2000-engine --lib
cargo test -p conformance --test kifu --test replay
```

For a selective bake, retain the previous artifact, matrix, basis and manifest
in a scratch directory alongside its basis-named matrix cache, then run:

```sh
cargo run --release -p nc2000-bot --example meta_eco -- bake-catalog-preview --threads 8 --nash-only-reuse tmp/preview-baseline --reuse-revision 97a133f6 --out-dir tmp/preview-nash-repair
python3 tools/activate-preview-bake.py tmp/preview-nash-repair
crates/wasm/build.sh
crates/wasm/build.sh nodejs
node tools/verify-selective-preview.cjs tmp/preview-baseline/compiled-preview-v1.json
```

Build/test the Web client using the canonical `web/package.json` scripts.
The exact fixed challenger command is in `challenger-protocol.json`.
