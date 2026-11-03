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

The primary four-party weights (`APEX4` in `data/kodoku-r3/conclusion-equilibrium-deme.json`)
are historical measurements from before the blind-boundary repair. To test
whether any existing party achieves > 50% against `APEX4` when **both sides use
the repaired bot under identical production settings** (`data/live-search.json`:
`ProtocolAgent` + `PlayerChannel`, `81,000` battle iterations, `729,000`
unknown-preview iterations or compiled preview lookup `u32(seed) % 64` when
covered by `data/compiled-preview-v1.json`, `c = 0.4`, `SelRule::Ucb`, legacy
argmax action policy, and the unmodified `data/belief-pool-v3/belief-pool.json`
prior), we evaluated the highest-scoring historical challenger parties against
`APEX4`:

1. **`mjj-2022-nrsk`** (Starmie50 / Cloyster50 / Machamp50 / Umbreon50 / Marowak50 / Snorlax55):
   the #1 highest-scoring opponent across all 307 evaluated challengers in
   `data/kodoku-r3/apex-exp-cache.json` (`0.474688` = `1 - 0.525312148962149`,
   the exact opponent that set `min_matrix_payoff` in `conclusion-equilibrium-deme.json`).
2. **Top catalog challengers** from `data/team-pool-v2/preview-nash-matrix.json`
   and `data/partybox-v0/eval-report.json`: `sample-07` (#1 in the `training-v3`
   preview matrix, `0.4977`), `mjj-2010-garap-a` (#1 in the pre-`training-v3`
   catalog matrix, `0.4783`), `pb-100` (#1 in `partybox-v0`, `0.4513`), and
   `sample-08` (#2 non-APEX4 by `1 - APEX4` value in `training-v3`, `0.4620`).

| Challenger party | Historical score vs `APEX4` | Games (Mirrored Pairs) | W–L–T–Cap | Win Rate vs `APEX4` | Paired SE | Approx. 95% CI | Preview Source (Blue / Red) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| **`mjj-2022-nrsk`** | `0.4747` (#1 overall in `apex-exp-cache.json`) | 64 (32) | **41–23–0–0** | **64.06%** | 6.44% | **[51.44%, 76.69%]** | `search` / `search` (729k) |
| `sample-07` | `0.4977` (#1 catalog in `training-v3` matrix) | 64 (32) | 32–32–0–0 | 50.00% | 5.94% | [38.36%, 61.64%] | `table` / `table` |
| `mjj-2010-garap-a` | `0.4783` (#1 catalog in `572d77de` matrix) | 64 (32) | 20–44–0–0 | 31.25% | 6.64% | [18.23%, 44.27%] | `table` / `table` |
| `pb-100` | `0.4513` (#1 in `partybox-v0/eval-report.json`) | 32 (16) | 17–15–0–0 | 53.12% | 8.50% | [36.46%, 69.79%] | `table` / `table` |
| `sample-08` | `0.4620` (#2 catalog by `1 - APEX4` in `training-v3`) | 32 (16) | 16–16–0–0 | 50.00% | 10.21% | [30.00%, 70.00%] | `table` / `table` |

In the primary 32-pair (64-game) match under identical production bot settings,
`mjj-2022-nrsk` won **41 games to 23** (`64.0625%`, paired SE `6.4424%`,
approximate 95% CI **`[51.44%, 76.69%]`**), with positive or even results across
all four `APEX4` variants (`19–5` vs `apex-c08-surgical-142`, `10–6` vs
`red-v10-s1105-r1-s0148`, `8–8` vs `red-v10-s1105-r2-s0142`, `4–4` vs
`apex-c04-bug-3atkstar-boomgen`; exact mixture-weighted score `61.25%`).
Within the 33-team catalog (`src=table`), none of the top challengers beat
`APEX4` (`sample-07` 50.00%, `sample-08` 50.00%, `pb-100` 53.12% ± 8.50%,
`mjj-2010-garap-a` 31.25%). Because an off-catalog party (`mjj-2022-nrsk`)
achieves > 50% against `APEX4` under identical production settings of the
repaired bot, we retain the historical mixture weights and label them as
historical rather than claiming an equilibrium certificate for the repaired bot.

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
