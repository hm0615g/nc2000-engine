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

## Limited adversarial Nash recheck

The primary four-party weights are historical measurements, not a new
certificate for the repaired policy. The strongest lawful full-game challenger
in the existing 81k record was tied between T1c and T2; T2's stronger 27k record
broke the tie. Only **T2 Bayesian policy-oracle response** is rechecked. This
selects an opponent policy from recorded evidence, rather than searching new
parties or rerunning all challenger tiers.

The [fixed protocol](challenger-protocol.json) was written before the run:
Independent weighted party draws in each of 32 mirrored blocks, 64 games,
81,000 iterations per ordinary
search, fresh seed 2026110307, 250-turn cap, K=2. The `same_budget` switch shares
81k between the attacker's own warmup/response; its two additional opponent
policy queries cost another 2×81k. Total compute is therefore greater for the
attacker. The 1v1 bound solver retains its fixed work and 80 ms time limits.

Blue uses the actual `ProtocolAgent`/`PlayerChannel` path, frozen production
prior and regenerated compiled choice `u32(seed) % 64`. Red knows the public
mixture, samples hypothetical worlds from public history, models the compiled
preview distribution and computes an approximate response. Its queries never
receive Blue's realized hidden loadout, private request or next action.
The harness can also run historical full-information tiers, but this corrected
run selects only the lawful T2 tier and no separate endgame corpus.

`challenger-provenance.json` hashes the executable, source/data inputs and
protocol. `challenger.json` retains every game's teams, seeds, side, score,
turn count and solver interventions, plus paired uncertainty estimates.
A failure to detect an advantage against this one opponent cannot establish
a global Nash equilibrium or rule out untested parties/search policies.
Two incomplete runs were discarded when integration/adversarial tests
required source changes (relaxed parties, then the self-KO counterexample).
The [restart record](challenger-restart.json) documents both technical restarts;
the opponent, seed, budget and fixed count were unchanged. The final fixed run completed in 1,921 seconds. T2 won **42 games to 22**,
with no ties or turn caps (maximum 102 turns). Its win rate was **65.625%**;
the paired standard error was 6.123 percentage points and the approximate
normal 95% interval was **53.6–77.6%**. There were five solver interventions.
[Independent aggregate checks](challenger-summary.json) validate all 32 mirrored
pairs and confirm that every recorded source/data/executable hash stayed
unchanged throughout the final run.

This is evidence of a T2 advantage under the stated greater-compute setting.
The normal interval is approximate; the conservative one-sided 95% Hoeffding
upper bound on T2's mean is 87.3%, not a useful low-exploitability certificate.
The broad full-game Nash claim is not supported. This experiment changes the
opponent policy, so it does not by itself disprove an equilibrium of a game
that fixes both players to the same search policy. That narrower claim would
require fresh party-payoff measurement. We retain the historical weights and
label them accordingly rather than replacing them with unmeasured weights.

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
